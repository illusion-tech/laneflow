//! 本 tick 实际 Waiting 依赖的增量无环图；候选拒绝恢复边和拓扑序。

use crate::StepError;

fn reserve<T>(values: &mut Vec<T>, count: usize) -> Result<(), StepError> {
    crate::conflict_tick::reserve(values, count)
}

#[derive(Clone, Copy)]
struct Edge {
    from: usize,
    to: usize,
    next: Option<usize>,
    active: bool,
}

/// 邻接表在拍初按实际候选物化，候选仅切换边。拓扑序不参与业务胜者排序。
#[derive(Default)]
pub(crate) struct WaitingGraph {
    heads: Vec<Option<usize>>,
    edges: Vec<Edge>,
    order: Vec<usize>,
    rank: Vec<usize>,
    indegree: Vec<usize>,
    seen: Vec<u32>,
    epoch: u32,
    stack: Vec<usize>,
    reorder: Vec<usize>,
    edge_undo: Vec<(usize, bool)>,
    order_undo: Vec<(usize, usize)>,
}

impl WaitingGraph {
    pub(crate) fn clear(&mut self) {
        self.heads.clear();
        self.edges.clear();
        self.order.clear();
        self.rank.clear();
        self.indegree.clear();
        self.seen.clear();
        self.stack.clear();
        self.reorder.clear();
        self.edge_undo.clear();
        self.order_undo.clear();
        self.epoch = 0;
    }

    pub(crate) fn node(&mut self) -> Result<usize, StepError> {
        reserve(&mut self.heads, 1)?;
        let index = self.heads.len();
        self.heads.push(None);
        Ok(index)
    }

    pub(crate) fn edge(
        &mut self,
        from: usize,
        to: usize,
        active: bool,
    ) -> Result<usize, StepError> {
        if from >= self.heads.len() || to >= self.heads.len() {
            return Err(StepError::ConflictInvariantViolation);
        }
        reserve(&mut self.edges, 1)?;
        let index = self.edges.len();
        self.edges.push(Edge {
            from,
            to,
            next: self.heads[from],
            active,
        });
        self.heads[from] = Some(index);
        Ok(index)
    }

    /// 拍初完整校验，之后只检查候选实际影响的图区域。
    pub(crate) fn initialize(&mut self, index: usize, active: bool) {
        debug_assert!(self.order.is_empty());
        self.edges[index].active = active;
    }

    pub(crate) fn seal(&mut self) -> Result<bool, StepError> {
        let count = self.heads.len();
        reserve(&mut self.indegree, count)?;
        reserve(&mut self.order, count)?;
        reserve(&mut self.rank, count)?;
        reserve(&mut self.seen, count)?;
        reserve(&mut self.stack, count)?;
        reserve(&mut self.reorder, count)?;
        self.indegree.resize(count, 0);
        self.rank.resize(count, 0);
        self.seen.resize(count, 0);
        for edge in &self.edges {
            if edge.active {
                self.indegree[edge.to] += 1;
            }
        }
        for node in 0..count {
            if self.indegree[node] == 0 {
                self.order.push(node);
            }
        }
        let mut cursor = 0;
        while let Some(node) = self.order.get(cursor).copied() {
            self.rank[node] = cursor;
            cursor += 1;
            let mut edge = self.heads[node];
            while let Some(index) = edge {
                let value = self.edges[index];
                edge = value.next;
                count_visit();
                if value.active {
                    self.indegree[value.to] -= 1;
                    if self.indegree[value.to] == 0 {
                        self.order.push(value.to);
                    }
                }
            }
        }
        #[cfg(test)]
        crate::conflict::count_conflict_work(|work| {
            work.wait_for_nodes += count;
            work.wait_for_edges += self.edges.len();
        });
        Ok(self.order.len() == count)
    }

    pub(crate) fn begin(&mut self) {
        debug_assert!(self.edge_undo.is_empty() && self.order_undo.is_empty());
    }

    pub(crate) fn set(&mut self, index: usize, active: bool) -> Result<bool, StepError> {
        let edge = *self
            .edges
            .get(index)
            .ok_or(StepError::ConflictInvariantViolation)?;
        if edge.active == active {
            return Ok(true);
        }
        reserve(&mut self.edge_undo, 1)?;
        self.edge_undo.push((index, edge.active));
        self.edges[index].active = active;
        if !active || self.rank[edge.from] < self.rank[edge.to] {
            return Ok(true);
        }
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.seen.fill(0);
            self.epoch = 1;
        }
        let first = self.rank[edge.to];
        let last = self.rank[edge.from];
        self.stack.clear();
        self.stack.push(edge.to);
        self.seen[edge.to] = self.epoch;
        while let Some(node) = self.stack.pop() {
            count_visit();
            if node == edge.from {
                return Ok(false);
            }
            let mut outgoing = self.heads[node];
            while let Some(index) = outgoing {
                let value = self.edges[index];
                outgoing = value.next;
                count_visit();
                if value.active && self.rank[value.to] <= last && self.seen[value.to] != self.epoch
                {
                    self.seen[value.to] = self.epoch;
                    self.stack.push(value.to);
                }
            }
        }
        // 将可达集合稳定移到 from 之后。只改受影响区间，不重排全世界节点。
        reserve(&mut self.order_undo, last - first + 1)?;
        self.reorder.clear();
        self.reorder.extend(
            self.order[first..=last]
                .iter()
                .copied()
                .filter(|node| self.seen[*node] != self.epoch),
        );
        self.reorder.extend(
            self.order[first..=last]
                .iter()
                .copied()
                .filter(|node| self.seen[*node] == self.epoch),
        );
        for (offset, node) in self.reorder.iter().copied().enumerate() {
            let position = first + offset;
            self.order_undo.push((position, self.order[position]));
            self.order[position] = node;
            self.rank[node] = position;
            #[cfg(test)]
            crate::conflict::count_conflict_work(|work| work.wait_for_reorders += 1);
        }
        Ok(true)
    }

    pub(crate) fn accept(&mut self) {
        self.edge_undo.clear();
        self.order_undo.clear();
    }

    pub(crate) fn rollback(&mut self) {
        for (position, node) in self.order_undo.drain(..).rev() {
            self.order[position] = node;
            self.rank[node] = position;
            #[cfg(test)]
            crate::conflict::count_conflict_work(|work| work.wait_for_rollbacks += 1);
        }
        for (index, active) in self.edge_undo.drain(..).rev() {
            self.edges[index].active = active;
        }
    }

    #[cfg(test)]
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        let Self {
            heads,
            edges,
            order,
            rank,
            indegree,
            seen,
            epoch: _,
            stack,
            reorder,
            edge_undo,
            order_undo,
        } = self;
        fn bytes<T>(v: &Vec<T>) -> u64 {
            (v.capacity() * std::mem::size_of::<T>()) as u64
        }
        bytes(heads)
            + bytes(edges)
            + bytes(order)
            + bytes(rank)
            + bytes(indegree)
            + bytes(seen)
            + bytes(stack)
            + bytes(reorder)
            + bytes(edge_undo)
            + bytes(order_undo)
    }
}

fn count_visit() {
    #[cfg(test)]
    crate::conflict::count_conflict_work(|work| work.wait_for_visits += 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acyclic(nodes: usize, edges: &[Edge]) -> bool {
        let mut degree = vec![0; nodes];
        for edge in edges {
            if edge.active {
                degree[edge.to] += 1;
            }
        }
        let mut ready = (0..nodes)
            .filter(|node| degree[*node] == 0)
            .collect::<Vec<_>>();
        let mut visited = 0;
        while let Some(node) = ready.pop() {
            visited += 1;
            for edge in edges.iter().filter(|edge| edge.active && edge.from == node) {
                degree[edge.to] -= 1;
                if degree[edge.to] == 0 {
                    ready.push(edge.to);
                }
            }
        }
        visited == nodes
    }

    #[test]
    fn candidate_batches_match_complete_graph_and_rollback_topology() {
        let mut graph = WaitingGraph::default();
        for _ in 0..12 {
            graph.node().unwrap();
        }
        for from in 0..12 {
            for to in 0..12 {
                if from != to {
                    graph.edge(from, to, false).unwrap();
                }
            }
        }
        assert!(graph.seal().unwrap());
        let mut seed = 19_u64;
        for attempt in 0..2_000 {
            let previous = graph
                .edges
                .iter()
                .map(|edge| edge.active)
                .collect::<Vec<_>>();
            let previous_order = graph.order.clone();
            graph.begin();
            let mut changes = Vec::new();
            for _ in 0..5 {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                changes.push(((seed as usize) % graph.edges.len(), seed & 256 != 0));
            }
            changes.sort_unstable_by_key(|change| (change.1, change.0));
            changes.dedup();
            let mut expected_edges = graph.edges.clone();
            for (edge, active) in &changes {
                expected_edges[*edge].active = *active;
            }
            let expected = acyclic(12, &expected_edges);
            let mut accepted = true;
            for (edge, active) in changes {
                if !graph.set(edge, active).unwrap() {
                    accepted = false;
                    break;
                }
            }
            assert_eq!(accepted, expected, "candidate {attempt}");
            if accepted && attempt % 3 != 0 {
                graph.accept();
                assert!(
                    graph
                        .edges
                        .iter()
                        .filter(|edge| edge.active)
                        .all(|edge| graph.rank[edge.from] < graph.rank[edge.to])
                );
            } else {
                graph.rollback();
                assert_eq!(graph.order, previous_order);
                assert_eq!(
                    graph
                        .edges
                        .iter()
                        .map(|edge| edge.active)
                        .collect::<Vec<_>>(),
                    previous
                );
                for (rank, node) in graph.order.iter().enumerate() {
                    assert_eq!(graph.rank[*node], rank);
                }
            }
        }
        assert!(graph.retained_logical_bytes() > 0);
        graph.clear();
        assert!(graph.seal().unwrap());
    }
}
