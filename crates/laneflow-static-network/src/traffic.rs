use core::mem::size_of;

use laneflow_static_contract::{EntityKind, LaneEdgeOrdinal};

use crate::{BuildError, BuildStructure};

const ENTITY_KIND_COUNT: usize = EntityKind::ALL.len();

/// 连续 flat payload 中的受检 `u32` 区间。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RangeU32 {
    start: u32,
    len: u32,
}

impl RangeU32 {
    pub(crate) const fn new(start: u32, len: u32) -> Self {
        Self { start, len }
    }

    #[must_use]
    pub const fn start(self) -> u32 {
        self.start
    }

    #[must_use]
    pub const fn len(self) -> u32 {
        self.len
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    pub(crate) fn slice<T>(self, values: &[T]) -> &[T] {
        let start = usize::try_from(self.start).expect("checked u32 range start");
        let end = usize::try_from(
            self.start
                .checked_add(self.len)
                .expect("checked u32 range end"),
        )
        .expect("checked u32 range end fits usize");
        &values[start..end]
    }
}

/// 22 种稳定实体的致密表基数。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EntityCounts {
    counts: [u32; ENTITY_KIND_COUNT],
}

impl EntityCounts {
    pub(crate) const fn new(counts: [u32; ENTITY_KIND_COUNT]) -> Self {
        Self { counts }
    }

    #[must_use]
    pub const fn count(self, entity_kind: EntityKind) -> u32 {
        self.counts[(entity_kind.code() - 1) as usize]
    }

    #[must_use]
    pub fn typed_count<K: laneflow_static_contract::EntityKindMarker>(&self) -> u32 {
        self.count(K::KIND)
    }

    #[must_use]
    pub const fn as_array(&self) -> &[u32; ENTITY_KIND_COUNT] {
        &self.counts
    }
}

/// 面向 Traffic Runtime 的必选共享静态数据。
///
/// 当前基线先冻结所有实体基数和 LaneEdge 热列/CSR；后继 #300 实现切片继续加入其余
/// 已冻结静态关系，但不会改变根唯一共享所有权。
pub struct SharedTrafficNetwork {
    entity_counts: EntityCounts,
    lane_lengths_meters: Box<[f64]>,
    lane_speed_limits_meters_per_second: Box<[f64]>,
    successor_ranges: Box<[RangeU32]>,
    successors: Box<[LaneEdgeOrdinal]>,
    predecessor_ranges: Box<[RangeU32]>,
    predecessors: Box<[LaneEdgeOrdinal]>,
}

impl SharedTrafficNetwork {
    pub(crate) fn new(
        entity_counts: EntityCounts,
        lane_lengths_meters: Box<[f64]>,
        lane_speed_limits_meters_per_second: Box<[f64]>,
        successor_ranges: Box<[RangeU32]>,
        successors: Box<[LaneEdgeOrdinal]>,
        predecessor_ranges: Box<[RangeU32]>,
        predecessors: Box<[LaneEdgeOrdinal]>,
    ) -> Self {
        Self {
            entity_counts,
            lane_lengths_meters,
            lane_speed_limits_meters_per_second,
            successor_ranges,
            successors,
            predecessor_ranges,
            predecessors,
        }
    }

    #[must_use]
    pub const fn entity_counts(&self) -> &EntityCounts {
        &self.entity_counts
    }

    #[must_use]
    pub fn lane_edge_count(&self) -> u32 {
        self.entity_counts.count(EntityKind::LaneEdge)
    }

    #[must_use]
    pub fn lane_lengths_meters(&self) -> &[f64] {
        &self.lane_lengths_meters
    }

    #[must_use]
    pub fn lane_speed_limits_meters_per_second(&self) -> &[f64] {
        &self.lane_speed_limits_meters_per_second
    }

    #[must_use]
    pub fn successors(&self, lane_edge: LaneEdgeOrdinal) -> Option<&[LaneEdgeOrdinal]> {
        let range = *self.successor_ranges.get(lane_edge.index())?;
        Some(range.slice(&self.successors))
    }

    #[must_use]
    pub fn predecessors(&self, lane_edge: LaneEdgeOrdinal) -> Option<&[LaneEdgeOrdinal]> {
        let range = *self.predecessor_ranges.get(lane_edge.index())?;
        Some(range.slice(&self.predecessors))
    }

    #[must_use]
    pub fn retained_logical_bytes(&self) -> u64 {
        logical_bytes::<f64>(self.lane_lengths_meters.len())
            + logical_bytes::<f64>(self.lane_speed_limits_meters_per_second.len())
            + logical_bytes::<RangeU32>(self.successor_ranges.len())
            + logical_bytes::<LaneEdgeOrdinal>(self.successors.len())
            + logical_bytes::<RangeU32>(self.predecessor_ranges.len())
            + logical_bytes::<LaneEdgeOrdinal>(self.predecessors.len())
    }
}

/// 从规范 LaneEdge 关系确定性派生、与 worker 数无关的非语义规划提示。
pub struct PartitionPlanningHints {
    edge_boundary_weights: Box<[u32]>,
}

impl PartitionPlanningHints {
    pub(crate) fn from_traffic(traffic: &SharedTrafficNetwork) -> Result<Self, BuildError> {
        let capacity =
            usize::try_from(traffic.lane_edge_count()).expect("u32 lane count fits usize");
        let mut edge_boundary_weights = Vec::new();
        edge_boundary_weights
            .try_reserve_exact(capacity)
            .map_err(|_| BuildError::AllocationFailure {
                structure: BuildStructure::PlanningHints,
            })?;
        for raw in 0..traffic.lane_edge_count() {
            let edge = LaneEdgeOrdinal::from_raw(raw);
            let successors = traffic.successors(edge).map_or(0, <[LaneEdgeOrdinal]>::len);
            let predecessors = traffic
                .predecessors(edge)
                .map_or(0, <[LaneEdgeOrdinal]>::len);
            edge_boundary_weights.push(
                u32::try_from(successors + predecessors)
                    .expect("format-bounded adjacency fits u32"),
            );
        }
        Ok(Self {
            edge_boundary_weights: edge_boundary_weights.into_boxed_slice(),
        })
    }

    #[must_use]
    pub fn edge_boundary_weights(&self) -> &[u32] {
        &self.edge_boundary_weights
    }

    #[must_use]
    pub fn retained_logical_bytes(&self) -> u64 {
        logical_bytes::<u32>(self.edge_boundary_weights.len())
    }
}

pub(crate) fn logical_bytes<T>(len: usize) -> u64 {
    let bytes = len
        .checked_mul(size_of::<T>())
        .expect("format-bounded retained length must fit usize");
    u64::try_from(bytes).expect("retained length must fit u64")
}
