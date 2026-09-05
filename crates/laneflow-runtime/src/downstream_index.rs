//! 按物理 edge 和实际后车间距索引的 AVL 区间树；节点来自可复用连续缓冲。

use crate::conflict::ConflictAcquireError;
use crate::{DownstreamInterval, VehicleHandle};
use std::num::NonZeroU32;

#[derive(Clone, Copy)]
struct Node {
    interval: DownstreamInterval,
    owner: VehicleHandle,
    gap: u32,
    left: Option<NonZeroU32>,
    right: Option<NonZeroU32>,
    height: u8,
    max_end: u64,
}

impl Node {
    fn end(self) -> u64 {
        u64::from(self.interval.end_mm()) + u64::from(self.gap)
    }
    fn key(self) -> (u32, u32, u64, u32, u32) {
        (
            self.interval.edge().raw(),
            self.interval.start_mm(),
            self.end(),
            self.owner.index(),
            self.owner.generation(),
        )
    }
}

#[derive(Default)]
pub(crate) struct DownstreamIndex {
    root: Option<NonZeroU32>,
    nodes: Vec<Node>,
}

fn index(id: NonZeroU32) -> usize {
    id.get() as usize - 1
}

impl DownstreamIndex {
    pub(crate) fn clear(&mut self) {
        self.root = None;
        self.nodes.clear();
    }

    pub(crate) fn reserve(&mut self, additional: usize) -> Result<(), ConflictAcquireError> {
        let count = self
            .nodes
            .len()
            .checked_add(additional)
            .ok_or(ConflictAcquireError::Capacity)?;
        u32::try_from(count).map_err(|_| ConflictAcquireError::Capacity)?;
        #[cfg(test)]
        if additional > self.nodes.capacity() - self.nodes.len() {
            crate::conflict::check_allocation_failpoint()?;
        }
        self.nodes
            .try_reserve(additional)
            .map_err(|_| ConflictAcquireError::ScratchAllocFailed)
    }

    pub(crate) fn insert(&mut self, interval: DownstreamInterval, owner: VehicleHandle, gap: u32) {
        let id =
            NonZeroU32::new(u32::try_from(self.nodes.len() + 1).expect("preflight node count"))
                .expect("nonzero node");
        self.nodes.push(Node {
            interval,
            owner,
            gap,
            left: None,
            right: None,
            height: 1,
            max_end: u64::from(interval.end_mm()) + u64::from(gap),
        });
        self.root = Some(self.insert_at(self.root, id));
    }

    fn height(&self, id: Option<NonZeroU32>) -> u8 {
        id.map_or(0, |id| self.nodes[index(id)].height)
    }
    fn refresh(&mut self, id: NonZeroU32) {
        let node = self.nodes[index(id)];
        let height = 1 + self.height(node.left).max(self.height(node.right));
        let max_end = [node.left, node.right]
            .into_iter()
            .flatten()
            .fold(node.end(), |end, child| {
                end.max(self.nodes[index(child)].max_end)
            });
        self.nodes[index(id)].height = height;
        self.nodes[index(id)].max_end = max_end;
    }
    fn rotate_left(&mut self, id: NonZeroU32) -> NonZeroU32 {
        let root = self.nodes[index(id)].right.expect("right-heavy tree");
        self.nodes[index(id)].right = self.nodes[index(root)].left;
        self.nodes[index(root)].left = Some(id);
        self.refresh(id);
        self.refresh(root);
        root
    }
    fn rotate_right(&mut self, id: NonZeroU32) -> NonZeroU32 {
        let root = self.nodes[index(id)].left.expect("left-heavy tree");
        self.nodes[index(id)].left = self.nodes[index(root)].right;
        self.nodes[index(root)].right = Some(id);
        self.refresh(id);
        self.refresh(root);
        root
    }
    fn insert_at(&mut self, root: Option<NonZeroU32>, id: NonZeroU32) -> NonZeroU32 {
        let Some(root) = root else {
            return id;
        };
        if self.nodes[index(id)].key() < self.nodes[index(root)].key() {
            let child = self.insert_at(self.nodes[index(root)].left, id);
            self.nodes[index(root)].left = Some(child);
        } else {
            let child = self.insert_at(self.nodes[index(root)].right, id);
            self.nodes[index(root)].right = Some(child);
        }
        self.refresh(root);
        let node = self.nodes[index(root)];
        let balance = i16::from(self.height(node.left)) - i16::from(self.height(node.right));
        if balance > 1 {
            let left = node.left.expect("left-heavy tree");
            let child = self.nodes[index(left)];
            if self.height(child.left) < self.height(child.right) {
                self.nodes[index(root)].left = Some(self.rotate_left(left));
            }
            self.rotate_right(root)
        } else if balance < -1 {
            let right = node.right.expect("right-heavy tree");
            let child = self.nodes[index(right)];
            if self.height(child.right) < self.height(child.left) {
                self.nodes[index(root)].right = Some(self.rotate_right(right));
            }
            self.rotate_left(root)
        } else {
            root
        }
    }

    pub(crate) fn conflicts(
        &self,
        interval: DownstreamInterval,
        owner: VehicleHandle,
        gap: u32,
    ) -> bool {
        // 物理坐标较小的区间属于后车；只向前扩展该车自己的最小间距。
        let end = u64::from(interval.end_mm()) + u64::from(gap);
        self.query(self.root, interval, owner, end)
    }

    fn query(
        &self,
        root: Option<NonZeroU32>,
        subject: DownstreamInterval,
        owner: VehicleHandle,
        end: u64,
    ) -> bool {
        let Some(root) = root else {
            return false;
        };
        #[cfg(test)]
        crate::conflict::count_conflict_work(|work| work.downstream_interval_visits += 1);
        let node = self.nodes[index(root)];
        if node.max_end <= u64::from(subject.start_mm()) {
            return false;
        }
        if node.interval.edge() < subject.edge() {
            return self.query(node.right, subject, owner, end);
        }
        if node.interval.edge() > subject.edge() {
            return self.query(node.left, subject, owner, end);
        }
        if node.owner != owner
            && u64::from(node.interval.start_mm()) < end
            && node.end() > u64::from(subject.start_mm())
        {
            return true;
        }
        self.query(node.left, subject, owner, end)
            || (u64::from(node.interval.start_mm()) < end
                && self.query(node.right, subject, owner, end))
    }

    #[cfg(test)]
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        let Self { root: _, nodes } = self;
        (nodes.capacity() * std::mem::size_of::<Node>()) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use laneflow_static_contract::LaneEdgeOrdinal;

    #[test]
    fn directional_gap_queries_match_pairwise_oracle_and_reuse_storage() {
        let mut tree = DownstreamIndex::default();
        assert_eq!(tree.retained_logical_bytes(), 0);
        tree.reserve(512).unwrap();
        let mut claims = Vec::new();
        let mut seed = 7_u64;
        for number in 0..512 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let edge = LaneEdgeOrdinal::from_raw((seed % 13) as u32);
            let start = ((seed >> 16) % 10_000) as u32;
            let interval =
                DownstreamInterval::new(edge, start, start + 1 + (seed % 80) as u32).unwrap();
            let owner = VehicleHandle::new(number % 29, number / 29);
            let gap = (seed % 130) as u32;
            let expected = claims.iter().any(|(other, other_owner, other_gap)| {
                owner != *other_owner
                    && crate::conflict::intervals_conflict(interval, gap, *other, *other_gap)
            });
            assert_eq!(tree.conflicts(interval, owner, gap), expected);
            tree.insert(interval, owner, gap);
            claims.push((interval, owner, gap));
        }
        for (interval, owner, gap) in &claims {
            let expected = claims.iter().any(|(other, other_owner, other_gap)| {
                owner != other_owner
                    && crate::conflict::intervals_conflict(*interval, *gap, *other, *other_gap)
            });
            assert_eq!(tree.conflicts(*interval, *owner, *gap), expected);
        }
        let retained = tree.retained_logical_bytes();
        tree.clear();
        assert_eq!(tree.retained_logical_bytes(), retained);
    }
}
