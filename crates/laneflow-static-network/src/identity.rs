use core::marker::PhantomData;
use core::mem::size_of;

use laneflow_static_contract::{
    EntityKind, EntityKindMarker, Ordinal, OrdinalKind, StableId, StableId128,
};

use crate::{BuildError, BuildStructure, EntityCounts};

const ENTITY_KIND_COUNT: usize = EntityKind::ALL.len();

/// 构建期单一种类的身份排序暂存条目；排序后投影为该种类的 ordinal 排列，不进入保留内存。
#[derive(Clone, Copy)]
pub(crate) struct IdentitySortEntry {
    stable_id: StableId128,
    ordinal: u32,
}

impl IdentitySortEntry {
    const DUMMY: Self = Self {
        stable_id: StableId128::ZERO,
        ordinal: 0,
    };
}

/// 所有稳定实体的 typed ordinal ↔ StableId128 冷双向索引。
///
/// 正向表按种类保存 typed ordinal → `StableId128`；反向查找按种类各保存一份 ordinal 排列，
/// 排列中的 ordinal 按其正向 `StableId128` 严格升序，每个稳定实体保留 4 B。
pub struct SharedIdentityIndex {
    pub(crate) forward: [Box<[StableId128]>; ENTITY_KIND_COUNT],
    pub(crate) by_stable_id: [Box<[u32]>; ENTITY_KIND_COUNT],
}

impl SharedIdentityIndex {
    /// 由已就序的正向表与各种类排列组装索引；仅供 builder 调用。
    pub(crate) fn from_parts(
        forward: [Box<[StableId128]>; ENTITY_KIND_COUNT],
        by_stable_id: [Box<[u32]>; ENTITY_KIND_COUNT],
    ) -> Self {
        Self {
            forward,
            by_stable_id,
        }
    }

    /// 按 typed ordinal 查询稳定标识；越界时返回 None。
    #[must_use]
    pub fn stable_id<K>(&self, ordinal: Ordinal<K>) -> Option<StableId<K>>
    where
        K: EntityKindMarker + OrdinalKind,
    {
        let raw = *self.forward[kind_index(K::KIND)].get(ordinal.index())?;
        Some(StableId::from_untyped(raw))
    }

    /// 按稳定标识查询 typed ordinal；未收录时返回 None。
    ///
    /// 在该种类的排列上二分，比较时经正向表取 `StableId128`。
    #[must_use]
    pub fn ordinal<K>(&self, stable_id: StableId<K>) -> Option<Ordinal<K>>
    where
        K: EntityKindMarker + OrdinalKind,
    {
        let raw = stable_id.into_untyped();
        let kind = kind_index(K::KIND);
        let forward = &self.forward[kind];
        let order = &self.by_stable_id[kind];
        let index = order
            .binary_search_by(|ordinal| forward[*ordinal as usize].cmp(&raw))
            .ok()?;
        Some(Ordinal::from_raw(order[index]))
    }

    /// 按 `StableId128` 升序遍历某一种类的 typed ordinal；借用连续排列，不分配。
    #[must_use]
    pub fn ordinals_by_stable_id<K>(&self) -> StableIdOrder<'_, K>
    where
        K: EntityKindMarker + OrdinalKind,
    {
        StableIdOrder {
            ordinals: &self.by_stable_id[kind_index(K::KIND)],
            marker: PhantomData,
        }
    }

    /// 返回指定实体种类的稳定实体数量。
    #[must_use]
    pub fn entity_count(&self, entity_kind: EntityKind) -> u32 {
        u32::try_from(self.forward[kind_index(entity_kind)].len())
            .expect("format-bounded identity count fits u32")
    }

    /// 返回本索引全部保留内存的逻辑字节数。
    #[must_use]
    pub fn retained_logical_bytes(&self) -> u64 {
        let forward_count = self.forward.iter().map(|items| items.len()).sum::<usize>();
        let order_count = self
            .by_stable_id
            .iter()
            .map(|items| items.len())
            .sum::<usize>();
        logical_bytes::<StableId128>(forward_count) + logical_bytes::<u32>(order_count)
    }
}

/// 某一种类按 `StableId128` 升序排列的 typed ordinal；借用索引内的连续排列。
pub struct StableIdOrder<'a, K: OrdinalKind> {
    ordinals: &'a [u32],
    marker: PhantomData<fn() -> K>,
}

impl<K: OrdinalKind> Clone for StableIdOrder<'_, K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: OrdinalKind> Copy for StableIdOrder<'_, K> {}

impl<'a, K: OrdinalKind> StableIdOrder<'a, K> {
    /// 该种类的稳定实体数量。
    #[must_use]
    pub const fn len(&self) -> usize {
        self.ordinals.len()
    }

    /// 该种类是否没有稳定实体。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.ordinals.is_empty()
    }

    /// 按 `StableId128` 升序的第 `index` 个 typed ordinal；越界时返回 None。
    #[must_use]
    pub fn get(&self, index: usize) -> Option<Ordinal<K>> {
        self.ordinals.get(index).copied().map(Ordinal::from_raw)
    }

    /// 按 `StableId128` 升序遍历 typed ordinal。
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = Ordinal<K>> + ExactSizeIterator + 'a {
        self.ordinals.iter().copied().map(Ordinal::from_raw)
    }
}

/// 按实体计数为各类正向身份表精确预分配容量。
pub(crate) fn allocate_forward_identity(
    counts: &EntityCounts,
) -> Result<[Vec<StableId128>; ENTITY_KIND_COUNT], BuildError> {
    allocate_per_kind(counts)
}

fn allocate_per_kind<T>(counts: &EntityCounts) -> Result<[Vec<T>; ENTITY_KIND_COUNT], BuildError> {
    let mut failed = false;
    let result = core::array::from_fn(|index| {
        let entity_kind = EntityKind::ALL[index];
        let capacity = usize::try_from(counts.count(entity_kind)).expect("u32 count fits usize");
        let mut values = Vec::new();
        if values.try_reserve_exact(capacity).is_err() {
            failed = true;
        }
        values
    });
    if failed {
        return Err(BuildError::AllocationFailure {
            structure: BuildStructure::CanonicalIdentity,
        });
    }
    Ok(result)
}

/// 由各种类正向表建立按 `StableId128` 升序的 ordinal 排列，并检出跨种类重复的稳定标识。
///
/// 各种类依次在两份按最大种类规模预留的暂存上做 16 趟 LSD 基数排序，投影为排列后复用暂存；
/// 暂存在返回前释放。随后对各种类排列做 K 路归并，按全局 `StableId128` 升序检出重复，报告
/// 最小的重复稳定标识。每趟之间与每个有界批次之间检查取消。
pub(crate) fn build_identity_order(
    forward: &[Vec<StableId128>; ENTITY_KIND_COUNT],
    mut check_cancelled: impl FnMut() -> Result<(), BuildError>,
) -> Result<[Box<[u32]>; ENTITY_KIND_COUNT], BuildError> {
    check_cancelled()?;
    let largest = forward.iter().map(Vec::len).max().unwrap_or(0);
    let mut source = scratch_entries(largest)?;
    let mut target = scratch_entries(largest)?;
    let mut orders: [Box<[u32]>; ENTITY_KIND_COUNT] = core::array::from_fn(|_| Box::default());
    for (kind, ids) in forward.iter().enumerate() {
        source.clear();
        for (ordinal, stable_id) in ids.iter().enumerate() {
            source.push(IdentitySortEntry {
                stable_id: *stable_id,
                ordinal: u32::try_from(ordinal).map_err(|_| BuildError::ArithmeticOverflow {
                    structure: BuildStructure::CanonicalIdentity,
                })?,
            });
        }
        target.clear();
        target.resize(source.len(), IdentitySortEntry::DUMMY);
        for pass in 0..16_usize {
            check_cancelled()?;
            radix_pass(&source, &mut target, 15 - pass);
            core::mem::swap(&mut source, &mut target);
        }
        let mut order = Vec::new();
        order
            .try_reserve_exact(source.len())
            .map_err(|_| BuildError::AllocationFailure {
                structure: BuildStructure::CanonicalIdentity,
            })?;
        order.extend(source.iter().map(|entry| entry.ordinal));
        orders[kind] = order.into_boxed_slice();
    }
    drop(source);
    drop(target);
    reject_duplicate_stable_ids(forward, &orders, check_cancelled)?;
    Ok(orders)
}

fn scratch_entries(capacity: usize) -> Result<Vec<IdentitySortEntry>, BuildError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| BuildError::AllocationFailure {
            structure: BuildStructure::BuilderScratch,
        })?;
    Ok(values)
}

fn radix_pass(source: &[IdentitySortEntry], target: &mut [IdentitySortEntry], byte: usize) {
    let mut counts = [0_usize; 256];
    for entry in source {
        counts[usize::from(entry.stable_id.as_bytes()[byte])] += 1;
    }
    let mut offsets = [0_usize; 256];
    let mut next = 0_usize;
    for (offset, count) in offsets.iter_mut().zip(counts) {
        *offset = next;
        next += count;
    }
    for entry in source {
        let bucket = usize::from(entry.stable_id.as_bytes()[byte]);
        target[offsets[bucket]] = *entry;
        offsets[bucket] += 1;
    }
}

/// 按全局 `StableId128` 升序归并各种类排列；相邻相等即重复。只用固定大小的游标数组。
fn reject_duplicate_stable_ids(
    forward: &[Vec<StableId128>; ENTITY_KIND_COUNT],
    orders: &[Box<[u32]>; ENTITY_KIND_COUNT],
    mut check_cancelled: impl FnMut() -> Result<(), BuildError>,
) -> Result<(), BuildError> {
    let mut cursors = [0_usize; ENTITY_KIND_COUNT];
    let head = |kind: usize, cursor: usize| {
        orders[kind]
            .get(cursor)
            .map(|ordinal| forward[kind][*ordinal as usize])
    };
    let mut previous: Option<StableId128> = None;
    let mut emitted = 0_usize;
    loop {
        let mut next: Option<(usize, StableId128)> = None;
        for (kind, cursor) in cursors.iter().enumerate() {
            if let Some(stable_id) = head(kind, *cursor)
                && next.is_none_or(|(_, best)| stable_id < best)
            {
                next = Some((kind, stable_id));
            }
        }
        let Some((kind, stable_id)) = next else {
            return Ok(());
        };
        if previous == Some(stable_id) {
            return Err(BuildError::DuplicateStableId { stable_id });
        }
        previous = Some(stable_id);
        cursors[kind] += 1;
        emitted += 1;
        if emitted & 1_023 == 0 {
            check_cancelled()?;
        }
    }
}

/// 身份排列构建的暂存峰值：两份按最大种类规模预留的排序条目。
pub(crate) const fn identity_sort_entry_bytes() -> usize {
    size_of::<IdentitySortEntry>()
}

/// 把各类正向身份 Vec 封存为 Box 切片。
pub(crate) fn seal_forward_identity(
    forward: [Vec<StableId128>; ENTITY_KIND_COUNT],
) -> [Box<[StableId128]>; ENTITY_KIND_COUNT] {
    forward.map(Vec::into_boxed_slice)
}

/// 返回实体种类在 ALL 数组中的下标。
pub(crate) const fn kind_index(entity_kind: EntityKind) -> usize {
    (entity_kind.code() - 1) as usize
}

fn logical_bytes<T>(len: usize) -> u64 {
    u64::try_from(
        len.checked_mul(size_of::<T>())
            .expect("retained size fits usize"),
    )
    .expect("retained size fits u64")
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;

    use super::*;

    fn id(high: u8, low: u8) -> StableId128 {
        let mut bytes = [0_u8; 16];
        bytes[0] = high;
        bytes[15] = low;
        StableId128::from_bytes(bytes)
    }

    fn forward(
        lane_edges: &[StableId128],
        junctions: &[StableId128],
    ) -> [Vec<StableId128>; ENTITY_KIND_COUNT] {
        let mut values: [Vec<StableId128>; ENTITY_KIND_COUNT] =
            core::array::from_fn(|_| Vec::new());
        values[kind_index(EntityKind::LaneEdge)] = lane_edges.to_vec();
        values[kind_index(EntityKind::Junction)] = junctions.to_vec();
        values
    }

    #[test]
    fn identity_order_build_propagates_cancellation_between_passes() {
        let checks = Cell::new(0_u32);
        let result = build_identity_order(&forward(&[id(1, 0)], &[]), || {
            let next = checks.get() + 1;
            checks.set(next);
            if next == 2 {
                Err(BuildError::Cancelled)
            } else {
                Ok(())
            }
        });

        assert!(matches!(result, Err(BuildError::Cancelled)));
        assert_eq!(checks.get(), 2);
    }

    #[test]
    fn identity_order_is_per_kind_and_ascends_by_forward_stable_id() {
        // 两种实体交错；高位字节与低位字节都参与排序，正向表顺序与 StableId 顺序不同。
        let lane_forward = [id(2, 0), id(0, 9), id(1, 5)];
        let junction_forward = [id(1, 0), id(0, 1)];
        let forward_tables = forward(&lane_forward, &junction_forward);
        let orders = build_identity_order(&forward_tables, || Ok(())).unwrap();
        assert_eq!(&*orders[kind_index(EntityKind::LaneEdge)], &[1, 2, 0]);
        assert_eq!(&*orders[kind_index(EntityKind::Junction)], &[1, 0]);
        for (index, order) in orders.iter().enumerate() {
            if index != kind_index(EntityKind::LaneEdge)
                && index != kind_index(EntityKind::Junction)
            {
                assert!(order.is_empty());
            }
        }

        let index = SharedIdentityIndex::from_parts(seal_forward_identity(forward_tables), orders);
        assert_eq!(index.retained_logical_bytes(), 5 * 16 + 5 * 4);
        let order = index.ordinals_by_stable_id::<laneflow_static_contract::LaneEdgeKind>();
        assert_eq!(order.len(), 3);
        assert_eq!(
            order
                .iter()
                .map(|ordinal| ordinal.raw())
                .collect::<Vec<_>>(),
            [1, 2, 0]
        );
        assert_eq!(order.get(3), None);
        for (ordinal, stable_id) in lane_forward.iter().enumerate() {
            let typed =
                StableId::<laneflow_static_contract::LaneEdgeKind>::from_untyped(*stable_id);
            assert_eq!(index.ordinal(typed).map(Ordinal::raw), Some(ordinal as u32));
        }
        // 同一 StableId 只在所属种类可查到。
        let other_kind =
            StableId::<laneflow_static_contract::JunctionKind>::from_untyped(lane_forward[0]);
        assert_eq!(index.ordinal(other_kind), None);
    }

    #[test]
    fn duplicate_stable_id_is_rejected_within_and_across_kinds_reporting_the_smallest() {
        let across = forward(&[id(0, 7), id(1, 0)], &[id(0, 7)]);
        assert!(matches!(
            build_identity_order(&across, || Ok(())),
            Err(BuildError::DuplicateStableId { stable_id }) if stable_id == id(0, 7)
        ));
        // 种类内重复与更小的跨种类重复同时存在时，报告全局最小者。
        let both = forward(&[id(3, 0), id(3, 0), id(0, 2)], &[id(0, 2)]);
        assert!(matches!(
            build_identity_order(&both, || Ok(())),
            Err(BuildError::DuplicateStableId { stable_id }) if stable_id == id(0, 2)
        ));
    }
}
