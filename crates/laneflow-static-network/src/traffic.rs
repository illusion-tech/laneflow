use core::mem::size_of;

use laneflow_static_contract::{
    EntityKind, LaneEdgeOrdinal, ManeuverGateOrdinal, ManeuverPathOrdinal, MovementOrdinal,
    WaitingZoneOrdinal,
};

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

/// 一条 Runtime 可执行 transition 对应的机动路径上下文。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManeuverTransitionCandidate {
    successor: LaneEdgeOrdinal,
    maneuver_path: ManeuverPathOrdinal,
    transition_index: u32,
    maneuver_gate: Option<ManeuverGateOrdinal>,
}

impl ManeuverTransitionCandidate {
    pub(crate) const fn new(
        successor: LaneEdgeOrdinal,
        maneuver_path: ManeuverPathOrdinal,
        transition_index: u32,
        maneuver_gate: Option<ManeuverGateOrdinal>,
    ) -> Self {
        Self {
            successor,
            maneuver_path,
            transition_index,
            maneuver_gate,
        }
    }

    #[must_use]
    pub const fn successor(self) -> LaneEdgeOrdinal {
        self.successor
    }

    #[must_use]
    pub const fn maneuver_path(self) -> ManeuverPathOrdinal {
        self.maneuver_path
    }

    #[must_use]
    pub const fn transition_index(self) -> u32 {
        self.transition_index
    }

    #[must_use]
    pub const fn maneuver_gate(self) -> Option<ManeuverGateOrdinal> {
        self.maneuver_gate
    }
}

/// 一条规范机动路径的连续 Runtime 借用。
#[derive(Clone, Copy, Debug)]
pub struct ManeuverPathView<'a> {
    movement: MovementOrdinal,
    edges: &'a [LaneEdgeOrdinal],
    maneuver_gates: &'a [ManeuverGateOrdinal],
    waiting_zones: &'a [WaitingZoneOrdinal],
}

impl<'a> ManeuverPathView<'a> {
    #[must_use]
    pub const fn movement(self) -> MovementOrdinal {
        self.movement
    }

    #[must_use]
    pub const fn edges(self) -> &'a [LaneEdgeOrdinal] {
        self.edges
    }

    #[must_use]
    pub const fn maneuver_gates(self) -> &'a [ManeuverGateOrdinal] {
        self.maneuver_gates
    }

    #[must_use]
    pub const fn waiting_zones(self) -> &'a [WaitingZoneOrdinal] {
        self.waiting_zones
    }
}

/// 路口机动路径、transition candidate 与 gate/waiting 引用的共享静态数据。
pub struct SharedManeuverNetwork {
    movements: Box<[MovementOrdinal]>,
    edge_ranges: Box<[RangeU32]>,
    edges: Box<[LaneEdgeOrdinal]>,
    maneuver_gate_ranges: Box<[RangeU32]>,
    maneuver_gates: Box<[ManeuverGateOrdinal]>,
    waiting_zone_ranges: Box<[RangeU32]>,
    waiting_zones: Box<[WaitingZoneOrdinal]>,
    candidate_ranges: Box<[RangeU32]>,
    candidates: Box<[ManeuverTransitionCandidate]>,
}

impl SharedManeuverNetwork {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        movements: Box<[MovementOrdinal]>,
        edge_ranges: Box<[RangeU32]>,
        edges: Box<[LaneEdgeOrdinal]>,
        maneuver_gate_ranges: Box<[RangeU32]>,
        maneuver_gates: Box<[ManeuverGateOrdinal]>,
        waiting_zone_ranges: Box<[RangeU32]>,
        waiting_zones: Box<[WaitingZoneOrdinal]>,
        candidate_ranges: Box<[RangeU32]>,
        candidates: Box<[ManeuverTransitionCandidate]>,
    ) -> Self {
        Self {
            movements,
            edge_ranges,
            edges,
            maneuver_gate_ranges,
            maneuver_gates,
            waiting_zone_ranges,
            waiting_zones,
            candidate_ranges,
            candidates,
        }
    }

    #[must_use]
    pub fn maneuver_path_count(&self) -> u32 {
        u32::try_from(self.movements.len()).expect("format-bounded maneuver path count fits u32")
    }

    #[must_use]
    pub fn maneuver_path(&self, path: ManeuverPathOrdinal) -> Option<ManeuverPathView<'_>> {
        let index = path.index();
        Some(ManeuverPathView {
            movement: *self.movements.get(index)?,
            edges: self.edge_ranges.get(index)?.slice(&self.edges),
            maneuver_gates: self
                .maneuver_gate_ranges
                .get(index)?
                .slice(&self.maneuver_gates),
            waiting_zones: self
                .waiting_zone_ranges
                .get(index)?
                .slice(&self.waiting_zones),
        })
    }

    pub(crate) fn path_gate_ranges(&self) -> &[RangeU32] {
        &self.maneuver_gate_ranges
    }

    pub(crate) fn path_gates(&self) -> &[ManeuverGateOrdinal] {
        &self.maneuver_gates
    }

    pub(crate) fn path_waiting_ranges(&self) -> &[RangeU32] {
        &self.waiting_zone_ranges
    }

    pub(crate) fn path_waiting_zones(&self) -> &[WaitingZoneOrdinal] {
        &self.waiting_zones
    }

    #[must_use]
    pub fn transition_candidates(
        &self,
        predecessor: LaneEdgeOrdinal,
    ) -> Option<&[ManeuverTransitionCandidate]> {
        let range = *self.candidate_ranges.get(predecessor.index())?;
        Some(range.slice(&self.candidates))
    }

    #[must_use]
    pub fn retained_logical_bytes(&self) -> u64 {
        logical_bytes::<MovementOrdinal>(self.movements.len())
            + logical_bytes::<RangeU32>(self.edge_ranges.len())
            + logical_bytes::<LaneEdgeOrdinal>(self.edges.len())
            + logical_bytes::<RangeU32>(self.maneuver_gate_ranges.len())
            + logical_bytes::<ManeuverGateOrdinal>(self.maneuver_gates.len())
            + logical_bytes::<RangeU32>(self.waiting_zone_ranges.len())
            + logical_bytes::<WaitingZoneOrdinal>(self.waiting_zones.len())
            + logical_bytes::<RangeU32>(self.candidate_ranges.len())
            + logical_bytes::<ManeuverTransitionCandidate>(self.candidates.len())
    }
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
/// 当前基线冻结所有实体基数、LaneEdge 热列/完整可执行 CSR，以及携带路径、transition、
/// gate/waiting 引用的路口机动候选；后继 #440 实现切片继续加入其余静态关系，但不会改变
/// 根唯一共享所有权。
pub struct SharedTrafficNetwork {
    entity_counts: EntityCounts,
    lane_lengths_millimetres: Box<[u32]>,
    lane_speed_limits_millimetres_per_second: Box<[u32]>,
    successor_ranges: Box<[RangeU32]>,
    successors: Box<[LaneEdgeOrdinal]>,
    predecessor_ranges: Box<[RangeU32]>,
    predecessors: Box<[LaneEdgeOrdinal]>,
    maneuvers: SharedManeuverNetwork,
    relations: crate::SharedRelationClosure,
}

impl SharedTrafficNetwork {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        entity_counts: EntityCounts,
        lane_lengths_millimetres: Box<[u32]>,
        lane_speed_limits_millimetres_per_second: Box<[u32]>,
        successor_ranges: Box<[RangeU32]>,
        successors: Box<[LaneEdgeOrdinal]>,
        predecessor_ranges: Box<[RangeU32]>,
        predecessors: Box<[LaneEdgeOrdinal]>,
        maneuvers: SharedManeuverNetwork,
        relations: crate::SharedRelationClosure,
    ) -> Self {
        Self {
            entity_counts,
            lane_lengths_millimetres,
            lane_speed_limits_millimetres_per_second,
            successor_ranges,
            successors,
            predecessor_ranges,
            predecessors,
            maneuvers,
            relations,
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
    pub fn lane_lengths_millimetres(&self) -> &[u32] {
        &self.lane_lengths_millimetres
    }

    #[must_use]
    pub fn lane_speed_limits_millimetres_per_second(&self) -> &[u32] {
        &self.lane_speed_limits_millimetres_per_second
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
    pub const fn maneuvers(&self) -> &SharedManeuverNetwork {
        &self.maneuvers
    }

    #[must_use]
    pub const fn relations(&self) -> &crate::SharedRelationClosure {
        &self.relations
    }

    #[must_use]
    pub fn retained_logical_bytes(&self) -> u64 {
        logical_bytes::<u32>(self.lane_lengths_millimetres.len())
            + logical_bytes::<u32>(self.lane_speed_limits_millimetres_per_second.len())
            + logical_bytes::<RangeU32>(self.successor_ranges.len())
            + logical_bytes::<LaneEdgeOrdinal>(self.successors.len())
            + logical_bytes::<RangeU32>(self.predecessor_ranges.len())
            + logical_bytes::<LaneEdgeOrdinal>(self.predecessors.len())
            + self.maneuvers.retained_logical_bytes()
            + self.relations.retained_logical_bytes()
    }
}

/// 从规范 LaneEdge 关系确定性派生、与 worker 数无关的非语义规划提示。
pub struct PartitionPlanningHints {
    edge_boundary_weights: Box<[u32]>,
}

impl PartitionPlanningHints {
    pub(crate) fn from_traffic(
        traffic: &SharedTrafficNetwork,
        mut poll_cancelled: impl FnMut(u32) -> Result<(), BuildError>,
    ) -> Result<Self, BuildError> {
        poll_cancelled(0)?;
        let capacity =
            usize::try_from(traffic.lane_edge_count()).expect("u32 lane count fits usize");
        let mut edge_boundary_weights = Vec::new();
        edge_boundary_weights
            .try_reserve_exact(capacity)
            .map_err(|_| BuildError::AllocationFailure {
                structure: BuildStructure::PlanningHints,
            })?;
        for raw in 0..traffic.lane_edge_count() {
            if raw != 0 {
                poll_cancelled(raw)?;
            }
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

#[cfg(test)]
mod tests {
    use core::cell::Cell;

    use super::*;

    #[test]
    fn planning_hints_propagate_cancellation_from_lane_scan() {
        let lane_count = 2_u32;
        let mut entity_counts = [0; ENTITY_KIND_COUNT];
        entity_counts[(EntityKind::LaneEdge.code() - 1) as usize] = lane_count;
        let empty_ranges = || vec![RangeU32::new(0, 0); lane_count as usize].into_boxed_slice();
        let maneuvers = SharedManeuverNetwork::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            empty_ranges(),
            Box::new([]),
        );
        let traffic = SharedTrafficNetwork::new(
            EntityCounts::new(entity_counts),
            vec![1_000; lane_count as usize].into_boxed_slice(),
            vec![1_000; lane_count as usize].into_boxed_slice(),
            empty_ranges(),
            Box::new([]),
            empty_ranges(),
            Box::new([]),
            maneuvers,
            crate::relations::empty_for_tests(lane_count),
        );
        let last_polled = Cell::new(None);

        let result = PartitionPlanningHints::from_traffic(&traffic, |raw| {
            last_polled.set(Some(raw));
            if raw == 1 {
                Err(BuildError::Cancelled)
            } else {
                Ok(())
            }
        });

        assert!(matches!(result, Err(BuildError::Cancelled)));
        assert_eq!(last_polled.get(), Some(1));
    }
}
