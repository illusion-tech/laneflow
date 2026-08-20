use core::mem::size_of;

use laneflow_static_contract::{
    AccessEffect, AccessRuleOrdinal, AuthoringLaneOrdinal, EntityKind, FacilityBandOrdinal,
    JunctionOrdinal, LaneEdgeOrdinal, LaneGroupOrdinal, ManeuverGateOrdinal, ManeuverPathOrdinal,
    MovementOrdinal, ParkingAreaOrdinal, ParkingSpaceOrdinal, ParticipantClassOrdinal,
    RoadCorridorOrdinal, RoadSectionOrdinal, SignalAspect, SignalControllerOrdinal,
    SignalGroupOrdinal, SignalPhaseOrdinal, StaticRouteOrdinal, StopLineOrdinal,
    VehicleProfileOrdinal, WaitingZoneOrdinal,
};

use crate::RangeU32;
use crate::traffic::logical_bytes;
use crate::{BuildError, BuildStructure, EntityCounts};

const UNCONSTRAINED_ROW: u32 = u32::MAX;

/// 道路走廊横断面中的有类型成员。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CorridorElement {
    RoadSection(RoadSectionOrdinal),
    FacilityBand(FacilityBandOrdinal),
}

/// 与当前 Core `FacilityKind` 等价的紧凑类型；自定义 token 只出现在冷 intern 表。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FacilityKind {
    MotorLane,
    NonMotorLane,
    Sidewalk,
    Median,
    PlantingStrip,
    FacilityStrip,
    Shoulder,
    Custom { intern: u32, lane_bearing: bool },
}

impl FacilityKind {
    #[must_use]
    pub const fn is_lane_bearing(self) -> bool {
        matches!(
            self,
            Self::MotorLane
                | Self::NonMotorLane
                | Self::Custom {
                    lane_bearing: true,
                    ..
                }
        )
    }
}

/// LFCA AccessRule 四种 typed target。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessTarget {
    LaneEdge(LaneEdgeOrdinal),
    LaneGroup(LaneGroupOrdinal),
    RoadSection(RoadSectionOrdinal),
    ManeuverPath(ManeuverPathOrdinal),
}

/// 共享准入平面单元。
///
/// 只表示**本修订内**的裁决。边或 class ordinal 越界由查询函数返回 `None`，
/// 不得把无效 handle 编码成 [`AccessCell::Unconstrained`]。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessCell {
    Unconstrained,
    Decided {
        rule: AccessRuleOrdinal,
        effect: AccessEffect,
    },
}

/// 与当前 Core `BoundedDistance` 同构的有界距离；禁止把溢出写成非有限 `f64`。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoundedDistance {
    Finite(f64),
    BeyondFinite,
}

impl BoundedDistance {
    pub(crate) const fn finite(value: f64) -> Self {
        Self::Finite(value)
    }

    pub(crate) fn add(self, value: f64) -> Self {
        match self {
            Self::Finite(current) if value.is_finite() && value <= f64::MAX - current => {
                Self::Finite(current + value)
            }
            Self::Finite(_) | Self::BeyondFinite => Self::BeyondFinite,
        }
    }
}

/// 与当前 Core `RouteDistanceQuery` 同构的有界距离查询结果。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RouteDistanceQuery {
    Passed,
    BeyondHorizon,
    Within(f64),
}

/// 共享静态路线距离索引：分段坐标 + 后缀有界距离。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RouteDistanceIndexView<'a> {
    occurrence_segments: &'a [u32],
    occurrence_offsets: &'a [f64],
    segment_totals: &'a [f64],
    distance_to_end: &'a [BoundedDistance],
}

impl<'a> RouteDistanceIndexView<'a> {
    #[cfg(test)]
    pub(crate) const fn from_parts(
        occurrence_segments: &'a [u32],
        occurrence_offsets: &'a [f64],
        segment_totals: &'a [f64],
        distance_to_end: &'a [BoundedDistance],
    ) -> Self {
        Self {
            occurrence_segments,
            occurrence_offsets,
            segment_totals,
            distance_to_end,
        }
    }

    #[must_use]
    pub const fn occurrence_segments(self) -> &'a [u32] {
        self.occurrence_segments
    }

    #[must_use]
    pub const fn occurrence_offsets(self) -> &'a [f64] {
        self.occurrence_offsets
    }

    #[must_use]
    pub const fn segment_totals(self) -> &'a [f64] {
        self.segment_totals
    }

    #[must_use]
    pub const fn distance_to_end(self) -> &'a [BoundedDistance] {
        self.distance_to_end
    }

    #[must_use]
    pub fn distance_within(
        self,
        from_occurrence: usize,
        from_progress: f64,
        target_occurrence: usize,
        target_progress: f64,
        horizon: f64,
    ) -> RouteDistanceQuery {
        if target_occurrence < from_occurrence
            || (target_occurrence == from_occurrence && target_progress < from_progress)
        {
            return RouteDistanceQuery::Passed;
        }
        let Some(from) = self.coordinate(from_occurrence) else {
            return RouteDistanceQuery::Passed;
        };
        let Some(target) = self.coordinate(target_occurrence) else {
            return RouteDistanceQuery::Passed;
        };

        if from.0 == target.0 {
            let distance = (target.1 + target_progress) - (from.1 + from_progress);
            return if distance <= horizon {
                RouteDistanceQuery::Within(distance.max(0.0))
            } else {
                RouteDistanceQuery::BeyondHorizon
            };
        }

        let Some(&from_segment_total) = self.segment_totals.get(from.0) else {
            return RouteDistanceQuery::Passed;
        };
        let mut distance = from_segment_total - (from.1 + from_progress);
        if distance > horizon {
            return RouteDistanceQuery::BeyondHorizon;
        }
        for segment in (from.0 + 1)..target.0 {
            let Some(&segment_total) = self.segment_totals.get(segment) else {
                return RouteDistanceQuery::Passed;
            };
            if segment_total > horizon - distance || (distance > 0.0 && segment_total >= horizon) {
                return RouteDistanceQuery::BeyondHorizon;
            }
            distance += segment_total;
        }
        let target_distance = target.1 + target_progress;
        if target_distance > horizon - distance || (distance > 0.0 && target_distance >= horizon) {
            return RouteDistanceQuery::BeyondHorizon;
        }
        RouteDistanceQuery::Within(distance + target_distance)
    }

    fn coordinate(self, occurrence: usize) -> Option<(usize, f64)> {
        let segment = usize::try_from(*self.occurrence_segments.get(occurrence)?).ok()?;
        let offset = *self.occurrence_offsets.get(occurrence)?;
        Some((segment, offset))
    }
}

/// 编制车道上的停止线与机动门绑定。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StopLineView<'a> {
    edge: LaneEdgeOrdinal,
    gates: &'a [ManeuverGateOrdinal],
}

impl<'a> StopLineView<'a> {
    #[must_use]
    pub const fn edge(self) -> LaneEdgeOrdinal {
        self.edge
    }

    #[must_use]
    pub const fn gates(self) -> &'a [ManeuverGateOrdinal] {
        self.gates
    }
}

/// 机动门拓扑：路径、transition、停止线与可选信号组。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManeuverGateView {
    path: ManeuverPathOrdinal,
    transition_index: u32,
    stop_line: StopLineOrdinal,
    signal_group: Option<SignalGroupOrdinal>,
}

impl ManeuverGateView {
    #[must_use]
    pub const fn path(self) -> ManeuverPathOrdinal {
        self.path
    }

    #[must_use]
    pub const fn transition_index(self) -> u32 {
        self.transition_index
    }

    #[must_use]
    pub const fn stop_line(self) -> StopLineOrdinal {
        self.stop_line
    }

    #[must_use]
    pub const fn signal_group(self) -> Option<SignalGroupOrdinal> {
        self.signal_group
    }
}

/// 等待区拓扑。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitingZoneView {
    path: ManeuverPathOrdinal,
    entry_gate: ManeuverGateOrdinal,
    release_gate: ManeuverGateOrdinal,
    max_occupancy: u32,
}

impl WaitingZoneView {
    #[must_use]
    pub const fn path(self) -> ManeuverPathOrdinal {
        self.path
    }

    #[must_use]
    pub const fn entry_gate(self) -> ManeuverGateOrdinal {
        self.entry_gate
    }

    #[must_use]
    pub const fn release_gate(self) -> ManeuverGateOrdinal {
        self.release_gate
    }

    #[must_use]
    pub const fn max_occupancy(self) -> u32 {
        self.max_occupancy
    }
}

/// 固定时制控制器程序。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SignalControllerView<'a> {
    offset_ms: u64,
    cycle_ms: u64,
    groups: &'a [SignalGroupOrdinal],
    phases: &'a [SignalPhaseOrdinal],
}

impl<'a> SignalControllerView<'a> {
    #[must_use]
    pub const fn offset_ms(self) -> u64 {
        self.offset_ms
    }

    #[must_use]
    pub const fn cycle_ms(self) -> u64 {
        self.cycle_ms
    }

    #[must_use]
    pub const fn groups(self) -> &'a [SignalGroupOrdinal] {
        self.groups
    }

    #[must_use]
    pub const fn phases(self) -> &'a [SignalPhaseOrdinal] {
        self.phases
    }
}

/// 信号相位及其累计互斥边界。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SignalPhaseView<'a> {
    controller: SignalControllerOrdinal,
    duration_ms: u64,
    end_offset_ms: u64,
    groups: &'a [SignalGroupOrdinal],
    aspects: &'a [SignalAspect],
}

impl<'a> SignalPhaseView<'a> {
    #[must_use]
    pub const fn controller(self) -> SignalControllerOrdinal {
        self.controller
    }

    #[must_use]
    pub const fn duration_ms(self) -> u64 {
        self.duration_ms
    }

    #[must_use]
    pub const fn end_offset_ms(self) -> u64 {
        self.end_offset_ms
    }

    pub fn states(self) -> impl Iterator<Item = (SignalGroupOrdinal, SignalAspect)> + 'a {
        self.groups
            .iter()
            .copied()
            .zip(self.aspects.iter().copied())
    }
}

/// 信号组属主与反向门。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SignalGroupView<'a> {
    controller: SignalControllerOrdinal,
    gates: &'a [ManeuverGateOrdinal],
}

impl<'a> SignalGroupView<'a> {
    #[must_use]
    pub const fn controller(self) -> SignalControllerOrdinal {
        self.controller
    }

    #[must_use]
    pub const fn gates(self) -> &'a [ManeuverGateOrdinal] {
        self.gates
    }
}

/// 停车位入口/出口锚点与几何。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParkingSpaceView {
    area: Option<ParkingAreaOrdinal>,
    entry_edge: LaneEdgeOrdinal,
    entry_progress: f64,
    exit_edge: LaneEdgeOrdinal,
    exit_progress: f64,
    lateral: f64,
    heading: f64,
    length: f64,
    width: f64,
}

impl ParkingSpaceView {
    #[must_use]
    pub const fn area(self) -> Option<ParkingAreaOrdinal> {
        self.area
    }

    #[must_use]
    pub const fn entry(self) -> (LaneEdgeOrdinal, f64) {
        (self.entry_edge, self.entry_progress)
    }

    #[must_use]
    pub const fn exit(self) -> (LaneEdgeOrdinal, f64) {
        (self.exit_edge, self.exit_progress)
    }

    #[must_use]
    pub const fn geometry(self) -> (f64, f64, f64, f64) {
        (self.lateral, self.heading, self.length, self.width)
    }
}

/// 参与者类别区间编码。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParticipantClassView {
    parent: Option<ParticipantClassOrdinal>,
    depth: u32,
    subtree_enter: u32,
    subtree_exit: u32,
}

impl ParticipantClassView {
    #[must_use]
    pub const fn parent(self) -> Option<ParticipantClassOrdinal> {
        self.parent
    }

    #[must_use]
    pub const fn depth(self) -> u32 {
        self.depth
    }

    #[must_use]
    pub const fn subtree_range(self) -> (u32, u32) {
        (self.subtree_enter, self.subtree_exit)
    }
}

/// 准入规则审计列（查询权威是 resolved 平面）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessRuleView<'a> {
    target: AccessTarget,
    effect: AccessEffect,
    classes: &'a [ParticipantClassOrdinal],
    priority: i32,
}

impl<'a> AccessRuleView<'a> {
    #[must_use]
    pub const fn target(self) -> AccessTarget {
        self.target
    }

    #[must_use]
    pub const fn effect(self) -> AccessEffect {
        self.effect
    }

    #[must_use]
    pub const fn classes(self) -> &'a [ParticipantClassOrdinal] {
        self.classes
    }

    #[must_use]
    pub const fn priority(self) -> i32 {
        self.priority
    }
}

/// 车型跟驰参数。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VehicleProfileView {
    class: ParticipantClassOrdinal,
    length: f64,
    desired_speed: f64,
    min_gap: f64,
    time_headway: f64,
    max_accel: f64,
    comfort_decel: f64,
    emergency_decel: f64,
}

impl VehicleProfileView {
    #[must_use]
    pub const fn class(self) -> ParticipantClassOrdinal {
        self.class
    }

    #[must_use]
    pub const fn length(self) -> f64 {
        self.length
    }

    #[must_use]
    pub const fn desired_speed(self) -> f64 {
        self.desired_speed
    }

    #[must_use]
    pub const fn min_gap(self) -> f64 {
        self.min_gap
    }

    #[must_use]
    pub const fn time_headway(self) -> f64 {
        self.time_headway
    }

    #[must_use]
    pub const fn max_accel(self) -> f64 {
        self.max_accel
    }

    #[must_use]
    pub const fn comfort_decel(self) -> f64 {
        self.comfort_decel
    }

    #[must_use]
    pub const fn emergency_decel(self) -> f64 {
        self.emergency_decel
    }
}

/// 路线上从某条边起的下一受控转换。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NextControlledTransition {
    gate: ManeuverGateOrdinal,
    from_route_edge_index: u32,
    distance_from_edge_start: BoundedDistance,
}

impl NextControlledTransition {
    #[must_use]
    pub const fn gate(self) -> ManeuverGateOrdinal {
        self.gate
    }

    #[must_use]
    pub const fn from_route_edge_index(self) -> u32 {
        self.from_route_edge_index
    }

    #[must_use]
    pub const fn distance_from_edge_start(self) -> BoundedDistance {
        self.distance_from_edge_start
    }
}

/// 一条 StaticRoute 上的机动 occurrence，含 owner-local 门/等待区 range。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteManeuverOccurrence {
    path: ManeuverPathOrdinal,
    entry_route_edge_index: u32,
    exit_route_edge_index: u32,
    gate_occurrence_range: RangeU32,
    waiting_occurrence_range: RangeU32,
}

impl RouteManeuverOccurrence {
    #[must_use]
    pub const fn path(self) -> ManeuverPathOrdinal {
        self.path
    }

    #[must_use]
    pub const fn entry_route_edge_index(self) -> u32 {
        self.entry_route_edge_index
    }

    #[must_use]
    pub const fn exit_route_edge_index(self) -> u32 {
        self.exit_route_edge_index
    }

    #[must_use]
    pub const fn gate_occurrence_range(self) -> RangeU32 {
        self.gate_occurrence_range
    }

    #[must_use]
    pub const fn waiting_occurrence_range(self) -> RangeU32 {
        self.waiting_occurrence_range
    }
}

/// 一条 StaticRoute 上的门 occurrence。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteGateOccurrence {
    gate: ManeuverGateOrdinal,
    maneuver_occurrence_index: u32,
    from_route_edge_index: u32,
    next_gate_occurrence_index: Option<u32>,
    next_boundary_route_edge_index: u32,
    waiting_zone_occurrence_index: Option<u32>,
}

impl RouteGateOccurrence {
    #[must_use]
    pub const fn gate(self) -> ManeuverGateOrdinal {
        self.gate
    }

    #[must_use]
    pub const fn maneuver_occurrence_index(self) -> u32 {
        self.maneuver_occurrence_index
    }

    #[must_use]
    pub const fn from_route_edge_index(self) -> u32 {
        self.from_route_edge_index
    }

    #[must_use]
    pub const fn next_gate_occurrence_index(self) -> Option<u32> {
        self.next_gate_occurrence_index
    }

    #[must_use]
    pub const fn next_boundary_route_edge_index(self) -> u32 {
        self.next_boundary_route_edge_index
    }

    #[must_use]
    pub const fn waiting_zone_occurrence_index(self) -> Option<u32> {
        self.waiting_zone_occurrence_index
    }
}

/// 一条 StaticRoute 上的等待区 occurrence。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteWaitingOccurrence {
    zone: WaitingZoneOrdinal,
    maneuver_occurrence_index: u32,
    entry_gate_occurrence_index: u32,
    release_gate_occurrence_index: u32,
    entry_route_edge_index: u32,
    release_route_edge_index: u32,
}

impl RouteWaitingOccurrence {
    #[must_use]
    pub const fn zone(self) -> WaitingZoneOrdinal {
        self.zone
    }

    #[must_use]
    pub const fn maneuver_occurrence_index(self) -> u32 {
        self.maneuver_occurrence_index
    }

    #[must_use]
    pub const fn entry_gate_occurrence_index(self) -> u32 {
        self.entry_gate_occurrence_index
    }

    #[must_use]
    pub const fn release_gate_occurrence_index(self) -> u32 {
        self.release_gate_occurrence_index
    }

    #[must_use]
    pub const fn entry_route_edge_index(self) -> u32 {
        self.entry_route_edge_index
    }

    #[must_use]
    pub const fn release_route_edge_index(self) -> u32 {
        self.release_route_edge_index
    }
}

/// 可选一对一反向：`None` 表示缺失，禁止用 `0` 冒充有效 ordinal。
#[derive(Clone, Debug)]
pub(crate) struct OptionalColumn<T> {
    values: Box<[Option<T>]>,
}

impl<T: Copy> OptionalColumn<T> {
    fn empty(len: usize) -> Result<Self, crate::BuildError> {
        let mut values = Vec::new();
        values
            .try_reserve_exact(len)
            .map_err(|_| crate::BuildError::AllocationFailure {
                structure: crate::BuildStructure::RelationClosure,
            })?;
        values.resize(len, None);
        Ok(Self {
            values: values.into_boxed_slice(),
        })
    }

    fn get(&self, index: usize) -> Option<T> {
        self.values.get(index).copied().flatten()
    }

    fn set(&mut self, index: usize, value: T) -> Result<(), crate::BuildError> {
        let slot = self
            .values
            .get_mut(index)
            .ok_or(crate::BuildError::InputInvariant {
                structure: crate::BuildStructure::RelationClosure,
            })?;
        if slot.is_some() {
            return Err(crate::BuildError::InputInvariant {
                structure: crate::BuildStructure::RelationClosure,
            });
        }
        *slot = Some(value);
        Ok(())
    }

    fn retained_bytes(&self) -> u64 {
        logical_bytes::<Option<T>>(self.values.len())
    }
}

/// #440 闭合的剩余 Traffic 静态关系。
#[allow(dead_code)]
pub struct SharedRelationClosure {
    intern: Box<[Box<str>]>,
    corridor_reference_section: Box<[RoadSectionOrdinal]>,
    corridor_element_ranges: Box<[RangeU32]>,
    corridor_elements: Box<[CorridorElement]>,
    section_corridor: Box<[RoadCorridorOrdinal]>,
    section_kind: Box<[FacilityKind]>,
    section_lane_ranges: Box<[RangeU32]>,
    section_lanes: Box<[AuthoringLaneOrdinal]>,
    authoring_section: Box<[RoadSectionOrdinal]>,
    authoring_edge_ranges: Box<[RangeU32]>,
    authoring_edges: Box<[LaneEdgeOrdinal]>,
    authoring_group: OptionalColumn<LaneGroupOrdinal>,
    edge_authoring_lane: OptionalColumn<AuthoringLaneOrdinal>,
    edge_junction: OptionalColumn<JunctionOrdinal>,
    edge_stop_line: OptionalColumn<StopLineOrdinal>,
    junction_movement_ranges: Box<[RangeU32]>,
    junction_movements: Box<[MovementOrdinal]>,
    movement_junction: Box<[JunctionOrdinal]>,
    movement_path_ranges: Box<[RangeU32]>,
    movement_paths: Box<[ManeuverPathOrdinal]>,
    stop_line_edge: Box<[LaneEdgeOrdinal]>,
    stop_line_gate_ranges: Box<[RangeU32]>,
    stop_line_gates: Box<[ManeuverGateOrdinal]>,
    gate_path: Box<[ManeuverPathOrdinal]>,
    gate_transition_index: Box<[u32]>,
    gate_stop_line: Box<[StopLineOrdinal]>,
    gate_signal_group: OptionalColumn<SignalGroupOrdinal>,
    waiting_path: Box<[ManeuverPathOrdinal]>,
    waiting_entry_gate: Box<[ManeuverGateOrdinal]>,
    waiting_release_gate: Box<[ManeuverGateOrdinal]>,
    waiting_max_occupancy: Box<[u32]>,
    group_controller: Box<[SignalControllerOrdinal]>,
    group_gate_ranges: Box<[RangeU32]>,
    group_gates: Box<[ManeuverGateOrdinal]>,
    controller_offset_ms: Box<[u64]>,
    controller_cycle_ms: Box<[u64]>,
    controller_group_ranges: Box<[RangeU32]>,
    controller_groups: Box<[SignalGroupOrdinal]>,
    controller_phase_ranges: Box<[RangeU32]>,
    controller_phases: Box<[SignalPhaseOrdinal]>,
    phase_controller: Box<[SignalControllerOrdinal]>,
    phase_duration_ms: Box<[u64]>,
    phase_end_offset_ms: Box<[u64]>,
    phase_state_ranges: Box<[RangeU32]>,
    phase_state_groups: Box<[SignalGroupOrdinal]>,
    phase_state_aspects: Box<[SignalAspect]>,
    parking_space_ranges: Box<[RangeU32]>,
    parking_spaces: Box<[ParkingSpaceOrdinal]>,
    space_area: OptionalColumn<ParkingAreaOrdinal>,
    space_entry_edge: Box<[LaneEdgeOrdinal]>,
    space_entry_progress: Box<[f64]>,
    space_exit_edge: Box<[LaneEdgeOrdinal]>,
    space_exit_progress: Box<[f64]>,
    space_lateral: Box<[f64]>,
    space_heading: Box<[f64]>,
    space_length: Box<[f64]>,
    space_width: Box<[f64]>,
    lane_group_section: Box<[RoadSectionOrdinal]>,
    lane_group_member_ranges: Box<[RangeU32]>,
    lane_group_members: Box<[AuthoringLaneOrdinal]>,
    band_corridor: Box<[RoadCorridorOrdinal]>,
    band_kind: Box<[FacilityKind]>,
    class_parent: OptionalColumn<ParticipantClassOrdinal>,
    class_depth: Box<[u32]>,
    class_subtree_enter: Box<[u32]>,
    class_subtree_exit: Box<[u32]>,
    rule_target: Box<[AccessTarget]>,
    rule_effect: Box<[AccessEffect]>,
    rule_class_ranges: Box<[RangeU32]>,
    rule_classes: Box<[ParticipantClassOrdinal]>,
    rule_priority: Box<[i32]>,
    profile_class: Box<[ParticipantClassOrdinal]>,
    profile_length: Box<[f64]>,
    profile_desired_speed: Box<[f64]>,
    profile_min_gap: Box<[f64]>,
    profile_time_headway: Box<[f64]>,
    profile_max_accel: Box<[f64]>,
    profile_comfort_decel: Box<[f64]>,
    profile_emergency_decel: Box<[f64]>,
    route_edge_ranges: Box<[RangeU32]>,
    route_edges: Box<[LaneEdgeOrdinal]>,
    route_gate_ranges: Box<[RangeU32]>,
    route_transition_gates: Box<[Option<ManeuverGateOrdinal>]>,
    route_maneuver_ranges: Box<[RangeU32]>,
    route_maneuver_paths: Box<[ManeuverPathOrdinal]>,
    route_maneuver_entry: Box<[u32]>,
    route_maneuver_exit: Box<[u32]>,
    route_maneuver_gate_occ_start: Box<[u32]>,
    route_maneuver_gate_occ_count: Box<[u32]>,
    route_maneuver_waiting_occ_start: Box<[u32]>,
    route_maneuver_waiting_occ_count: Box<[u32]>,
    route_gate_occ_ranges: Box<[RangeU32]>,
    route_gate_occ_gates: Box<[ManeuverGateOrdinal]>,
    route_gate_occ_maneuver: Box<[u32]>,
    route_gate_occ_from: Box<[u32]>,
    route_gate_occ_next: Box<[Option<u32>]>,
    route_gate_occ_next_boundary: Box<[u32]>,
    route_gate_occ_waiting: Box<[Option<u32>]>,
    route_waiting_occ_ranges: Box<[RangeU32]>,
    route_waiting_occ_zones: Box<[WaitingZoneOrdinal]>,
    route_waiting_occ_maneuver: Box<[u32]>,
    route_waiting_occ_entry_gate: Box<[u32]>,
    route_waiting_occ_release_gate: Box<[u32]>,
    route_waiting_occ_entry_edge: Box<[u32]>,
    route_waiting_occ_release_edge: Box<[u32]>,
    route_reverse_kind: Box<[u16]>,
    route_reverse_ordinal: Box<[u32]>,
    route_reverse_route: Box<[StaticRouteOrdinal]>,
    route_reverse_occurrence: Box<[u32]>,
    route_distance_to_end: Box<[BoundedDistance]>,
    route_distance_ranges: Box<[RangeU32]>,
    route_distance_segments: Box<[u32]>,
    route_distance_offsets: Box<[f64]>,
    route_segment_totals: Box<[f64]>,
    route_segment_ranges: Box<[RangeU32]>,
    next_controlled_gate: Box<[Option<ManeuverGateOrdinal>]>,
    next_controlled_from: Box<[u32]>,
    next_controlled_distance: Box<[BoundedDistance]>,
    speed_limit_from: Box<[u32]>,
    speed_limit_to_edge: Box<[LaneEdgeOrdinal]>,
    speed_limit_target: Box<[f64]>,
    speed_limit_ranges: Box<[RangeU32]>,
    access_class_count: u32,
    edge_row_starts: Box<[u32]>,
    edge_cells: Box<[AccessCell]>,
    path_row_starts: Box<[u32]>,
    path_cells: Box<[AccessCell]>,
}

impl SharedRelationClosure {
    pub(crate) fn intern_token(&self, intern: u32) -> Option<&str> {
        self.intern
            .get(usize::try_from(intern).ok()?)
            .map(|token| token.as_ref())
    }

    #[must_use]
    pub fn facility_kind_token(&self, kind: FacilityKind) -> Option<&str> {
        match kind {
            FacilityKind::MotorLane => Some("motorLane"),
            FacilityKind::NonMotorLane => Some("nonMotorLane"),
            FacilityKind::Sidewalk => Some("sidewalk"),
            FacilityKind::Median => Some("median"),
            FacilityKind::PlantingStrip => Some("plantingStrip"),
            FacilityKind::FacilityStrip => Some("facilityStrip"),
            FacilityKind::Shoulder => Some("shoulder"),
            FacilityKind::Custom { intern, .. } => self.intern_token(intern),
        }
    }

    #[must_use]
    pub fn corridor_elements(&self, corridor: RoadCorridorOrdinal) -> Option<&[CorridorElement]> {
        Some(
            self.corridor_element_ranges
                .get(corridor.index())?
                .slice(&self.corridor_elements),
        )
    }

    #[must_use]
    pub fn corridor_reference_section(
        &self,
        corridor: RoadCorridorOrdinal,
    ) -> Option<RoadSectionOrdinal> {
        self.corridor_reference_section
            .get(corridor.index())
            .copied()
    }

    #[must_use]
    pub fn section_kind(&self, section: RoadSectionOrdinal) -> Option<FacilityKind> {
        self.section_kind.get(section.index()).copied()
    }

    #[must_use]
    pub fn section_lanes(&self, section: RoadSectionOrdinal) -> Option<&[AuthoringLaneOrdinal]> {
        Some(
            self.section_lane_ranges
                .get(section.index())?
                .slice(&self.section_lanes),
        )
    }

    #[must_use]
    pub fn authoring_edge_chain(&self, lane: AuthoringLaneOrdinal) -> Option<&[LaneEdgeOrdinal]> {
        Some(
            self.authoring_edge_ranges
                .get(lane.index())?
                .slice(&self.authoring_edges),
        )
    }

    #[must_use]
    pub fn authoring_lane_group(&self, lane: AuthoringLaneOrdinal) -> Option<LaneGroupOrdinal> {
        self.authoring_group.get(lane.index())
    }

    #[must_use]
    pub fn lane_edge_authoring_lane(&self, edge: LaneEdgeOrdinal) -> Option<AuthoringLaneOrdinal> {
        self.edge_authoring_lane.get(edge.index())
    }

    #[must_use]
    pub fn lane_edge_junction(&self, edge: LaneEdgeOrdinal) -> Option<JunctionOrdinal> {
        self.edge_junction.get(edge.index())
    }

    #[must_use]
    pub fn stop_line_for_edge(&self, edge: LaneEdgeOrdinal) -> Option<StopLineOrdinal> {
        self.edge_stop_line.get(edge.index())
    }

    #[must_use]
    pub fn junction_movements(&self, junction: JunctionOrdinal) -> Option<&[MovementOrdinal]> {
        Some(
            self.junction_movement_ranges
                .get(junction.index())?
                .slice(&self.junction_movements),
        )
    }

    #[must_use]
    pub fn movement_paths(&self, movement: MovementOrdinal) -> Option<&[ManeuverPathOrdinal]> {
        Some(
            self.movement_path_ranges
                .get(movement.index())?
                .slice(&self.movement_paths),
        )
    }

    #[must_use]
    pub fn stop_line(&self, stop: StopLineOrdinal) -> Option<StopLineView<'_>> {
        Some(StopLineView {
            edge: *self.stop_line_edge.get(stop.index())?,
            gates: self
                .stop_line_gate_ranges
                .get(stop.index())?
                .slice(&self.stop_line_gates),
        })
    }

    #[must_use]
    pub fn maneuver_gate(&self, gate: ManeuverGateOrdinal) -> Option<ManeuverGateView> {
        Some(ManeuverGateView {
            path: *self.gate_path.get(gate.index())?,
            transition_index: *self.gate_transition_index.get(gate.index())?,
            stop_line: *self.gate_stop_line.get(gate.index())?,
            signal_group: self.gate_signal_group.get(gate.index()),
        })
    }

    #[must_use]
    pub fn waiting_zone(&self, zone: WaitingZoneOrdinal) -> Option<WaitingZoneView> {
        Some(WaitingZoneView {
            path: *self.waiting_path.get(zone.index())?,
            entry_gate: *self.waiting_entry_gate.get(zone.index())?,
            release_gate: *self.waiting_release_gate.get(zone.index())?,
            max_occupancy: *self.waiting_max_occupancy.get(zone.index())?,
        })
    }

    #[must_use]
    pub fn signal_group(&self, group: SignalGroupOrdinal) -> Option<SignalGroupView<'_>> {
        Some(SignalGroupView {
            controller: *self.group_controller.get(group.index())?,
            gates: self
                .group_gate_ranges
                .get(group.index())?
                .slice(&self.group_gates),
        })
    }

    #[must_use]
    pub fn signal_controller(
        &self,
        controller: SignalControllerOrdinal,
    ) -> Option<SignalControllerView<'_>> {
        Some(SignalControllerView {
            offset_ms: *self.controller_offset_ms.get(controller.index())?,
            cycle_ms: *self.controller_cycle_ms.get(controller.index())?,
            groups: self
                .controller_group_ranges
                .get(controller.index())?
                .slice(&self.controller_groups),
            phases: self
                .controller_phase_ranges
                .get(controller.index())?
                .slice(&self.controller_phases),
        })
    }

    #[must_use]
    pub fn signal_phase(&self, phase: SignalPhaseOrdinal) -> Option<SignalPhaseView<'_>> {
        let range = *self.phase_state_ranges.get(phase.index())?;
        Some(SignalPhaseView {
            controller: *self.phase_controller.get(phase.index())?,
            duration_ms: *self.phase_duration_ms.get(phase.index())?,
            end_offset_ms: *self.phase_end_offset_ms.get(phase.index())?,
            groups: range.slice(&self.phase_state_groups),
            aspects: range.slice(&self.phase_state_aspects),
        })
    }

    #[must_use]
    pub fn controller_cycle_ms(&self, controller: SignalControllerOrdinal) -> Option<u64> {
        self.signal_controller(controller)
            .map(|view| view.cycle_ms())
    }

    #[must_use]
    pub fn controller_phases(
        &self,
        controller: SignalControllerOrdinal,
    ) -> Option<&[SignalPhaseOrdinal]> {
        self.signal_controller(controller).map(|view| view.phases())
    }

    #[must_use]
    pub fn phase_duration_ms(&self, phase: SignalPhaseOrdinal) -> Option<u64> {
        self.signal_phase(phase).map(|view| view.duration_ms())
    }

    #[must_use]
    pub fn phase_end_offset_ms(&self, phase: SignalPhaseOrdinal) -> Option<u64> {
        self.phase_end_offset_ms.get(phase.index()).copied()
    }

    #[must_use]
    pub fn phase_states(
        &self,
        phase: SignalPhaseOrdinal,
    ) -> Option<(&[SignalGroupOrdinal], &[SignalAspect])> {
        let range = *self.phase_state_ranges.get(phase.index())?;
        Some((
            range.slice(&self.phase_state_groups),
            range.slice(&self.phase_state_aspects),
        ))
    }

    #[must_use]
    pub fn parking_area_spaces(&self, area: ParkingAreaOrdinal) -> Option<&[ParkingSpaceOrdinal]> {
        Some(
            self.parking_space_ranges
                .get(area.index())?
                .slice(&self.parking_spaces),
        )
    }

    #[must_use]
    pub fn parking_space(&self, space: ParkingSpaceOrdinal) -> Option<ParkingSpaceView> {
        Some(ParkingSpaceView {
            area: self.space_area.get(space.index()),
            entry_edge: *self.space_entry_edge.get(space.index())?,
            entry_progress: *self.space_entry_progress.get(space.index())?,
            exit_edge: *self.space_exit_edge.get(space.index())?,
            exit_progress: *self.space_exit_progress.get(space.index())?,
            lateral: *self.space_lateral.get(space.index())?,
            heading: *self.space_heading.get(space.index())?,
            length: *self.space_length.get(space.index())?,
            width: *self.space_width.get(space.index())?,
        })
    }

    #[must_use]
    pub fn parking_space_entry(
        &self,
        space: ParkingSpaceOrdinal,
    ) -> Option<(LaneEdgeOrdinal, f64)> {
        self.parking_space(space).map(|view| view.entry())
    }

    #[must_use]
    pub fn parking_space_geometry(
        &self,
        space: ParkingSpaceOrdinal,
    ) -> Option<(f64, f64, f64, f64)> {
        self.parking_space(space).map(|view| view.geometry())
    }

    #[must_use]
    pub fn participant_class(
        &self,
        class: ParticipantClassOrdinal,
    ) -> Option<ParticipantClassView> {
        Some(ParticipantClassView {
            parent: self.class_parent.get(class.index()),
            depth: *self.class_depth.get(class.index())?,
            subtree_enter: *self.class_subtree_enter.get(class.index())?,
            subtree_exit: *self.class_subtree_exit.get(class.index())?,
        })
    }

    #[must_use]
    pub fn access_rule(&self, rule: AccessRuleOrdinal) -> Option<AccessRuleView<'_>> {
        Some(AccessRuleView {
            target: *self.rule_target.get(rule.index())?,
            effect: *self.rule_effect.get(rule.index())?,
            classes: self
                .rule_class_ranges
                .get(rule.index())?
                .slice(&self.rule_classes),
            priority: *self.rule_priority.get(rule.index())?,
        })
    }

    #[must_use]
    pub fn vehicle_profile(&self, profile: VehicleProfileOrdinal) -> Option<VehicleProfileView> {
        Some(VehicleProfileView {
            class: *self.profile_class.get(profile.index())?,
            length: *self.profile_length.get(profile.index())?,
            desired_speed: *self.profile_desired_speed.get(profile.index())?,
            min_gap: *self.profile_min_gap.get(profile.index())?,
            time_headway: *self.profile_time_headway.get(profile.index())?,
            max_accel: *self.profile_max_accel.get(profile.index())?,
            comfort_decel: *self.profile_comfort_decel.get(profile.index())?,
            emergency_decel: *self.profile_emergency_decel.get(profile.index())?,
        })
    }

    #[must_use]
    pub fn edge_access(
        &self,
        edge: LaneEdgeOrdinal,
        class: ParticipantClassOrdinal,
    ) -> Option<AccessCell> {
        plane_cell(
            &self.edge_row_starts,
            &self.edge_cells,
            self.access_class_count,
            edge.index(),
            class.index(),
        )
    }

    #[must_use]
    pub fn path_access(
        &self,
        path: ManeuverPathOrdinal,
        class: ParticipantClassOrdinal,
    ) -> Option<AccessCell> {
        plane_cell(
            &self.path_row_starts,
            &self.path_cells,
            self.access_class_count,
            path.index(),
            class.index(),
        )
    }

    #[must_use]
    pub fn static_route_edges(&self, route: StaticRouteOrdinal) -> Option<&[LaneEdgeOrdinal]> {
        Some(
            self.route_edge_ranges
                .get(route.index())?
                .slice(&self.route_edges),
        )
    }

    #[must_use]
    pub fn static_route_transition_gates(
        &self,
        route: StaticRouteOrdinal,
    ) -> Option<&[Option<ManeuverGateOrdinal>]> {
        Some(
            self.route_gate_ranges
                .get(route.index())?
                .slice(&self.route_transition_gates),
        )
    }

    #[must_use]
    pub fn route_maneuver_count(&self, route: StaticRouteOrdinal) -> Option<usize> {
        Some(self.route_maneuver_ranges.get(route.index())?.len() as usize)
    }

    #[must_use]
    pub fn route_maneuver_occurrence(
        &self,
        route: StaticRouteOrdinal,
        index: usize,
    ) -> Option<RouteManeuverOccurrence> {
        let range = *self.route_maneuver_ranges.get(route.index())?;
        if index >= usize::try_from(range.len()).ok()? {
            return None;
        }
        let slot = usize::try_from(range.start()).ok()? + index;
        Some(RouteManeuverOccurrence {
            path: *self.route_maneuver_paths.get(slot)?,
            entry_route_edge_index: *self.route_maneuver_entry.get(slot)?,
            exit_route_edge_index: *self.route_maneuver_exit.get(slot)?,
            gate_occurrence_range: RangeU32::new(
                *self.route_maneuver_gate_occ_start.get(slot)?,
                *self.route_maneuver_gate_occ_count.get(slot)?,
            ),
            waiting_occurrence_range: RangeU32::new(
                *self.route_maneuver_waiting_occ_start.get(slot)?,
                *self.route_maneuver_waiting_occ_count.get(slot)?,
            ),
        })
    }

    #[must_use]
    pub fn route_gate_count(&self, route: StaticRouteOrdinal) -> Option<usize> {
        Some(self.route_gate_occ_ranges.get(route.index())?.len() as usize)
    }

    #[must_use]
    pub fn route_gate_occurrence(
        &self,
        route: StaticRouteOrdinal,
        index: usize,
    ) -> Option<RouteGateOccurrence> {
        let range = *self.route_gate_occ_ranges.get(route.index())?;
        if index >= usize::try_from(range.len()).ok()? {
            return None;
        }
        let slot = usize::try_from(range.start()).ok()? + index;
        Some(RouteGateOccurrence {
            gate: *self.route_gate_occ_gates.get(slot)?,
            maneuver_occurrence_index: *self.route_gate_occ_maneuver.get(slot)?,
            from_route_edge_index: *self.route_gate_occ_from.get(slot)?,
            next_gate_occurrence_index: *self.route_gate_occ_next.get(slot)?,
            next_boundary_route_edge_index: *self.route_gate_occ_next_boundary.get(slot)?,
            waiting_zone_occurrence_index: *self.route_gate_occ_waiting.get(slot)?,
        })
    }

    #[must_use]
    pub fn route_waiting_count(&self, route: StaticRouteOrdinal) -> Option<usize> {
        Some(self.route_waiting_occ_ranges.get(route.index())?.len() as usize)
    }

    #[must_use]
    pub fn route_waiting_occurrence(
        &self,
        route: StaticRouteOrdinal,
        index: usize,
    ) -> Option<RouteWaitingOccurrence> {
        let range = *self.route_waiting_occ_ranges.get(route.index())?;
        if index >= usize::try_from(range.len()).ok()? {
            return None;
        }
        let slot = usize::try_from(range.start()).ok()? + index;
        Some(RouteWaitingOccurrence {
            zone: *self.route_waiting_occ_zones.get(slot)?,
            maneuver_occurrence_index: *self.route_waiting_occ_maneuver.get(slot)?,
            entry_gate_occurrence_index: *self.route_waiting_occ_entry_gate.get(slot)?,
            release_gate_occurrence_index: *self.route_waiting_occ_release_gate.get(slot)?,
            entry_route_edge_index: *self.route_waiting_occ_entry_edge.get(slot)?,
            release_route_edge_index: *self.route_waiting_occ_release_edge.get(slot)?,
        })
    }

    #[must_use]
    pub fn route_distance_to_end(&self, route: StaticRouteOrdinal) -> Option<&[BoundedDistance]> {
        Some(
            self.route_distance_ranges
                .get(route.index())?
                .slice(&self.route_distance_to_end),
        )
    }

    #[must_use]
    pub fn route_distance_index(
        &self,
        route: StaticRouteOrdinal,
    ) -> Option<RouteDistanceIndexView<'_>> {
        let range = *self.route_distance_ranges.get(route.index())?;
        let segment_range = *self.route_segment_ranges.get(route.index())?;
        Some(RouteDistanceIndexView {
            occurrence_segments: range.slice(&self.route_distance_segments),
            occurrence_offsets: range.slice(&self.route_distance_offsets),
            segment_totals: segment_range.slice(&self.route_segment_totals),
            distance_to_end: range.slice(&self.route_distance_to_end),
        })
    }

    #[must_use]
    pub fn next_controlled_transition(
        &self,
        route: StaticRouteOrdinal,
        edge_index: usize,
    ) -> Option<NextControlledTransition> {
        let range = *self.route_edge_ranges.get(route.index())?;
        if edge_index >= usize::try_from(range.len()).ok()? {
            return None;
        }
        let index = usize::try_from(range.start()).ok()? + edge_index;
        Some(NextControlledTransition {
            gate: self.next_controlled_gate.get(index).copied().flatten()?,
            from_route_edge_index: *self.next_controlled_from.get(index)?,
            distance_from_edge_start: *self.next_controlled_distance.get(index)?,
        })
    }

    #[must_use]
    pub fn speed_limit_transitions(
        &self,
        route: StaticRouteOrdinal,
    ) -> Option<(&[u32], &[LaneEdgeOrdinal], &[f64])> {
        let range = *self.speed_limit_ranges.get(route.index())?;
        Some((
            range.slice(&self.speed_limit_from),
            range.slice(&self.speed_limit_to_edge),
            range.slice(&self.speed_limit_target),
        ))
    }

    #[must_use]
    pub fn lane_group_members(&self, group: LaneGroupOrdinal) -> Option<&[AuthoringLaneOrdinal]> {
        Some(
            self.lane_group_member_ranges
                .get(group.index())?
                .slice(&self.lane_group_members),
        )
    }

    #[must_use]
    pub fn band_kind(&self, band: FacilityBandOrdinal) -> Option<FacilityKind> {
        self.band_kind.get(band.index()).copied()
    }

    #[must_use]
    pub fn gate_signal_group(&self, gate: ManeuverGateOrdinal) -> Option<SignalGroupOrdinal> {
        self.maneuver_gate(gate)?.signal_group()
    }

    #[must_use]
    pub fn retained_logical_bytes(&self) -> u64 {
        u64::try_from(size_of::<Self>()).unwrap_or(0)
            + self
                .intern
                .iter()
                .map(|token| token.len() as u64)
                .sum::<u64>()
            + logical_bytes::<RoadSectionOrdinal>(self.corridor_reference_section.len())
            + logical_bytes::<RangeU32>(self.corridor_element_ranges.len())
            + logical_bytes::<CorridorElement>(self.corridor_elements.len())
            + logical_bytes::<RoadCorridorOrdinal>(self.section_corridor.len())
            + logical_bytes::<FacilityKind>(self.section_kind.len())
            + logical_bytes::<RangeU32>(self.section_lane_ranges.len())
            + logical_bytes::<AuthoringLaneOrdinal>(self.section_lanes.len())
            + logical_bytes::<RoadSectionOrdinal>(self.authoring_section.len())
            + logical_bytes::<RangeU32>(self.authoring_edge_ranges.len())
            + logical_bytes::<LaneEdgeOrdinal>(self.authoring_edges.len())
            + self.authoring_group.retained_bytes()
            + self.edge_authoring_lane.retained_bytes()
            + self.edge_junction.retained_bytes()
            + self.edge_stop_line.retained_bytes()
            + logical_bytes::<RangeU32>(self.junction_movement_ranges.len())
            + logical_bytes::<MovementOrdinal>(self.junction_movements.len())
            + logical_bytes::<JunctionOrdinal>(self.movement_junction.len())
            + logical_bytes::<RangeU32>(self.movement_path_ranges.len())
            + logical_bytes::<ManeuverPathOrdinal>(self.movement_paths.len())
            + logical_bytes::<LaneEdgeOrdinal>(self.stop_line_edge.len())
            + logical_bytes::<RangeU32>(self.stop_line_gate_ranges.len())
            + logical_bytes::<ManeuverGateOrdinal>(self.stop_line_gates.len())
            + logical_bytes::<ManeuverPathOrdinal>(self.gate_path.len())
            + logical_bytes::<u32>(self.gate_transition_index.len())
            + logical_bytes::<StopLineOrdinal>(self.gate_stop_line.len())
            + self.gate_signal_group.retained_bytes()
            + logical_bytes::<ManeuverPathOrdinal>(self.waiting_path.len())
            + logical_bytes::<ManeuverGateOrdinal>(self.waiting_entry_gate.len())
            + logical_bytes::<ManeuverGateOrdinal>(self.waiting_release_gate.len())
            + logical_bytes::<u32>(self.waiting_max_occupancy.len())
            + logical_bytes::<SignalControllerOrdinal>(self.group_controller.len())
            + logical_bytes::<RangeU32>(self.group_gate_ranges.len())
            + logical_bytes::<ManeuverGateOrdinal>(self.group_gates.len())
            + logical_bytes::<u64>(self.controller_offset_ms.len())
            + logical_bytes::<u64>(self.controller_cycle_ms.len())
            + logical_bytes::<RangeU32>(self.controller_group_ranges.len())
            + logical_bytes::<SignalGroupOrdinal>(self.controller_groups.len())
            + logical_bytes::<RangeU32>(self.controller_phase_ranges.len())
            + logical_bytes::<SignalPhaseOrdinal>(self.controller_phases.len())
            + logical_bytes::<SignalControllerOrdinal>(self.phase_controller.len())
            + logical_bytes::<u64>(self.phase_duration_ms.len())
            + logical_bytes::<u64>(self.phase_end_offset_ms.len())
            + logical_bytes::<RangeU32>(self.phase_state_ranges.len())
            + logical_bytes::<SignalGroupOrdinal>(self.phase_state_groups.len())
            + logical_bytes::<SignalAspect>(self.phase_state_aspects.len())
            + logical_bytes::<RangeU32>(self.parking_space_ranges.len())
            + logical_bytes::<ParkingSpaceOrdinal>(self.parking_spaces.len())
            + self.space_area.retained_bytes()
            + logical_bytes::<LaneEdgeOrdinal>(self.space_entry_edge.len())
            + logical_bytes::<f64>(self.space_entry_progress.len())
            + logical_bytes::<LaneEdgeOrdinal>(self.space_exit_edge.len())
            + logical_bytes::<f64>(self.space_exit_progress.len())
            + logical_bytes::<f64>(self.space_lateral.len())
            + logical_bytes::<f64>(self.space_heading.len())
            + logical_bytes::<f64>(self.space_length.len())
            + logical_bytes::<f64>(self.space_width.len())
            + logical_bytes::<RoadSectionOrdinal>(self.lane_group_section.len())
            + logical_bytes::<RangeU32>(self.lane_group_member_ranges.len())
            + logical_bytes::<AuthoringLaneOrdinal>(self.lane_group_members.len())
            + logical_bytes::<RoadCorridorOrdinal>(self.band_corridor.len())
            + logical_bytes::<FacilityKind>(self.band_kind.len())
            + self.class_parent.retained_bytes()
            + logical_bytes::<u32>(self.class_depth.len())
            + logical_bytes::<u32>(self.class_subtree_enter.len())
            + logical_bytes::<u32>(self.class_subtree_exit.len())
            + logical_bytes::<AccessTarget>(self.rule_target.len())
            + logical_bytes::<AccessEffect>(self.rule_effect.len())
            + logical_bytes::<RangeU32>(self.rule_class_ranges.len())
            + logical_bytes::<ParticipantClassOrdinal>(self.rule_classes.len())
            + logical_bytes::<i32>(self.rule_priority.len())
            + logical_bytes::<ParticipantClassOrdinal>(self.profile_class.len())
            + logical_bytes::<f64>(self.profile_length.len()) * 7
            + logical_bytes::<RangeU32>(self.route_edge_ranges.len())
            + logical_bytes::<LaneEdgeOrdinal>(self.route_edges.len())
            + logical_bytes::<RangeU32>(self.route_gate_ranges.len())
            + logical_bytes::<Option<ManeuverGateOrdinal>>(self.route_transition_gates.len())
            + logical_bytes::<RangeU32>(self.route_maneuver_ranges.len())
            + logical_bytes::<ManeuverPathOrdinal>(self.route_maneuver_paths.len())
            + logical_bytes::<u32>(self.route_maneuver_entry.len())
            + logical_bytes::<u32>(self.route_maneuver_exit.len())
            + logical_bytes::<u32>(self.route_maneuver_gate_occ_start.len())
            + logical_bytes::<u32>(self.route_maneuver_gate_occ_count.len())
            + logical_bytes::<u32>(self.route_maneuver_waiting_occ_start.len())
            + logical_bytes::<u32>(self.route_maneuver_waiting_occ_count.len())
            + logical_bytes::<RangeU32>(self.route_gate_occ_ranges.len())
            + logical_bytes::<ManeuverGateOrdinal>(self.route_gate_occ_gates.len())
            + logical_bytes::<u32>(self.route_gate_occ_maneuver.len())
            + logical_bytes::<u32>(self.route_gate_occ_from.len())
            + logical_bytes::<Option<u32>>(self.route_gate_occ_next.len())
            + logical_bytes::<u32>(self.route_gate_occ_next_boundary.len())
            + logical_bytes::<Option<u32>>(self.route_gate_occ_waiting.len())
            + logical_bytes::<RangeU32>(self.route_waiting_occ_ranges.len())
            + logical_bytes::<WaitingZoneOrdinal>(self.route_waiting_occ_zones.len())
            + logical_bytes::<u32>(self.route_waiting_occ_maneuver.len())
            + logical_bytes::<u32>(self.route_waiting_occ_entry_gate.len())
            + logical_bytes::<u32>(self.route_waiting_occ_release_gate.len())
            + logical_bytes::<u32>(self.route_waiting_occ_entry_edge.len())
            + logical_bytes::<u32>(self.route_waiting_occ_release_edge.len())
            + logical_bytes::<u16>(self.route_reverse_kind.len())
            + logical_bytes::<u32>(self.route_reverse_ordinal.len())
            + logical_bytes::<StaticRouteOrdinal>(self.route_reverse_route.len())
            + logical_bytes::<u32>(self.route_reverse_occurrence.len())
            + logical_bytes::<BoundedDistance>(self.route_distance_to_end.len())
            + logical_bytes::<RangeU32>(self.route_distance_ranges.len())
            + logical_bytes::<u32>(self.route_distance_segments.len())
            + logical_bytes::<f64>(self.route_distance_offsets.len())
            + logical_bytes::<f64>(self.route_segment_totals.len())
            + logical_bytes::<RangeU32>(self.route_segment_ranges.len())
            + logical_bytes::<Option<ManeuverGateOrdinal>>(self.next_controlled_gate.len())
            + logical_bytes::<u32>(self.next_controlled_from.len())
            + logical_bytes::<BoundedDistance>(self.next_controlled_distance.len())
            + logical_bytes::<u32>(self.speed_limit_from.len())
            + logical_bytes::<LaneEdgeOrdinal>(self.speed_limit_to_edge.len())
            + logical_bytes::<f64>(self.speed_limit_target.len())
            + logical_bytes::<RangeU32>(self.speed_limit_ranges.len())
            + logical_bytes::<u32>(self.edge_row_starts.len())
            + logical_bytes::<AccessCell>(self.edge_cells.len())
            + logical_bytes::<u32>(self.path_row_starts.len())
            + logical_bytes::<AccessCell>(self.path_cells.len())
    }
}

fn floor_add<T>(total: u64, count: u32) -> Result<u64, BuildError> {
    total
        .checked_add(logical_bytes::<T>(
            usize::try_from(count).expect("u32 fits usize"),
        ))
        .ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::RetainedOutput,
        })
}

pub(crate) fn relation_retained_floor(counts: &EntityCounts) -> Result<u64, BuildError> {
    let mut total = u64::try_from(size_of::<SharedRelationClosure>()).map_err(|_| {
        BuildError::ArithmeticOverflow {
            structure: BuildStructure::RetainedOutput,
        }
    })?;
    let corridor = counts.count(EntityKind::RoadCorridor);
    let section = counts.count(EntityKind::RoadSection);
    let authoring = counts.count(EntityKind::AuthoringLane);
    let lane = counts.count(EntityKind::LaneEdge);
    let junction = counts.count(EntityKind::Junction);
    let movement = counts.count(EntityKind::Movement);
    let path = counts.count(EntityKind::ManeuverPath);
    let gate = counts.count(EntityKind::ManeuverGate);
    let waiting = counts.count(EntityKind::WaitingZone);
    let stop = counts.count(EntityKind::StopLine);
    let group = counts.count(EntityKind::SignalGroup);
    let controller = counts.count(EntityKind::SignalController);
    let phase = counts.count(EntityKind::SignalPhase);
    let area = counts.count(EntityKind::ParkingArea);
    let space = counts.count(EntityKind::ParkingSpace);
    let lane_group = counts.count(EntityKind::LaneGroup);
    let band = counts.count(EntityKind::FacilityBand);
    let class = counts.count(EntityKind::ParticipantClass);
    let rule = counts.count(EntityKind::AccessRule);
    let profile = counts.count(EntityKind::VehicleProfile);
    let route = counts.count(EntityKind::StaticRoute);
    total = floor_add::<RoadSectionOrdinal>(total, corridor)?;
    total = floor_add::<RangeU32>(total, corridor)?;
    total = floor_add::<RoadCorridorOrdinal>(total, section)?;
    total = floor_add::<FacilityKind>(total, section)?;
    total = floor_add::<RangeU32>(total, section)?;
    total = floor_add::<RoadSectionOrdinal>(total, authoring)?;
    total = floor_add::<RangeU32>(total, authoring)?;
    total = floor_add::<Option<LaneGroupOrdinal>>(total, authoring)?;
    total = floor_add::<Option<AuthoringLaneOrdinal>>(total, lane)?;
    total = floor_add::<Option<JunctionOrdinal>>(total, lane)?;
    total = floor_add::<Option<StopLineOrdinal>>(total, lane)?;
    total = floor_add::<RangeU32>(total, junction)?;
    total = floor_add::<JunctionOrdinal>(total, movement)?;
    total = floor_add::<RangeU32>(total, movement)?;
    total = floor_add::<LaneEdgeOrdinal>(total, stop)?;
    total = floor_add::<RangeU32>(total, stop)?;
    total = floor_add::<ManeuverPathOrdinal>(total, gate)?;
    total = floor_add::<u32>(total, gate)?;
    total = floor_add::<StopLineOrdinal>(total, gate)?;
    total = floor_add::<Option<SignalGroupOrdinal>>(total, gate)?;
    total = floor_add::<ManeuverPathOrdinal>(total, waiting)?;
    total = floor_add::<ManeuverGateOrdinal>(total, waiting)?;
    total = floor_add::<ManeuverGateOrdinal>(total, waiting)?;
    total = floor_add::<u32>(total, waiting)?;
    total = floor_add::<SignalControllerOrdinal>(total, group)?;
    total = floor_add::<RangeU32>(total, group)?;
    total = floor_add::<u64>(total, controller)?;
    total = floor_add::<u64>(total, controller)?;
    total = floor_add::<RangeU32>(total, controller)?;
    total = floor_add::<RangeU32>(total, controller)?;
    total = floor_add::<SignalControllerOrdinal>(total, phase)?;
    total = floor_add::<u64>(total, phase)?;
    total = floor_add::<u64>(total, phase)?;
    total = floor_add::<RangeU32>(total, phase)?;
    total = floor_add::<RangeU32>(total, area)?;
    total = floor_add::<Option<ParkingAreaOrdinal>>(total, space)?;
    total = floor_add::<LaneEdgeOrdinal>(total, space)?;
    total = floor_add::<f64>(total, space)?;
    total = floor_add::<LaneEdgeOrdinal>(total, space)?;
    total = floor_add::<f64>(total, space)?;
    total = floor_add::<f64>(total, space)?;
    total = floor_add::<f64>(total, space)?;
    total = floor_add::<f64>(total, space)?;
    total = floor_add::<f64>(total, space)?;
    total = floor_add::<RoadSectionOrdinal>(total, lane_group)?;
    total = floor_add::<RangeU32>(total, lane_group)?;
    total = floor_add::<RoadCorridorOrdinal>(total, band)?;
    total = floor_add::<FacilityKind>(total, band)?;
    total = floor_add::<Option<ParticipantClassOrdinal>>(total, class)?;
    total = floor_add::<u32>(total, class)?;
    total = floor_add::<u32>(total, class)?;
    total = floor_add::<u32>(total, class)?;
    total = floor_add::<AccessTarget>(total, rule)?;
    total = floor_add::<AccessEffect>(total, rule)?;
    total = floor_add::<RangeU32>(total, rule)?;
    total = floor_add::<i32>(total, rule)?;
    total = floor_add::<ParticipantClassOrdinal>(total, profile)?;
    total = floor_add::<f64>(
        total,
        profile
            .checked_mul(7)
            .ok_or(BuildError::ArithmeticOverflow {
                structure: BuildStructure::RetainedOutput,
            })?,
    )?;
    total = floor_add::<RangeU32>(total, route)?;
    total = floor_add::<RangeU32>(total, route)?;
    total = floor_add::<RangeU32>(total, route)?;
    total = floor_add::<RangeU32>(total, route)?;
    total = floor_add::<RangeU32>(total, route)?;
    total = floor_add::<RangeU32>(total, route)?;
    total = floor_add::<RangeU32>(total, route)?;
    total = floor_add::<RangeU32>(total, route)?;
    total = floor_add::<u32>(total, lane)?;
    total = floor_add::<u32>(total, path)?;
    Ok(total)
}

fn plane_cell(
    row_starts: &[u32],
    cells: &[AccessCell],
    class_count: u32,
    unit: usize,
    class: usize,
) -> Option<AccessCell> {
    let &start = row_starts.get(unit)?;
    if class >= usize::try_from(class_count).unwrap_or(0) {
        return None;
    }
    if start == UNCONSTRAINED_ROW {
        return Some(AccessCell::Unconstrained);
    }
    cells.get(usize::try_from(start).ok()? + class).copied()
}

pub(crate) use builder_support::*;

mod builder_support {
    use super::*;
    use crate::BuildError;

    pub(crate) fn empty_optional<T: Copy>(len: u32) -> Result<OptionalColumn<T>, BuildError> {
        OptionalColumn::empty(usize::try_from(len).expect("u32 fits usize"))
    }

    pub(crate) fn set_optional<T: Copy>(
        column: &mut OptionalColumn<T>,
        index: u32,
        value: T,
    ) -> Result<(), BuildError> {
        column.set(usize::try_from(index).expect("u32 fits usize"), value)
    }

    pub(crate) fn get_optional<T: Copy>(column: &OptionalColumn<T>, index: u32) -> Option<T> {
        column.get(usize::try_from(index).expect("u32 fits usize"))
    }

    pub(crate) const ACCESS_UNCONSTRAINED_ROW: u32 = UNCONSTRAINED_ROW;

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn assemble(
        intern: Box<[Box<str>]>,
        corridor_reference_section: Box<[RoadSectionOrdinal]>,
        corridor_element_ranges: Box<[RangeU32]>,
        corridor_elements: Box<[CorridorElement]>,
        section_corridor: Box<[RoadCorridorOrdinal]>,
        section_kind: Box<[FacilityKind]>,
        section_lane_ranges: Box<[RangeU32]>,
        section_lanes: Box<[AuthoringLaneOrdinal]>,
        authoring_section: Box<[RoadSectionOrdinal]>,
        authoring_edge_ranges: Box<[RangeU32]>,
        authoring_edges: Box<[LaneEdgeOrdinal]>,
        authoring_group: OptionalColumn<LaneGroupOrdinal>,
        edge_authoring_lane: OptionalColumn<AuthoringLaneOrdinal>,
        edge_junction: OptionalColumn<JunctionOrdinal>,
        edge_stop_line: OptionalColumn<StopLineOrdinal>,
        junction_movement_ranges: Box<[RangeU32]>,
        junction_movements: Box<[MovementOrdinal]>,
        movement_junction: Box<[JunctionOrdinal]>,
        movement_path_ranges: Box<[RangeU32]>,
        movement_paths: Box<[ManeuverPathOrdinal]>,
        stop_line_edge: Box<[LaneEdgeOrdinal]>,
        stop_line_gate_ranges: Box<[RangeU32]>,
        stop_line_gates: Box<[ManeuverGateOrdinal]>,
        gate_path: Box<[ManeuverPathOrdinal]>,
        gate_transition_index: Box<[u32]>,
        gate_stop_line: Box<[StopLineOrdinal]>,
        gate_signal_group: OptionalColumn<SignalGroupOrdinal>,
        waiting_path: Box<[ManeuverPathOrdinal]>,
        waiting_entry_gate: Box<[ManeuverGateOrdinal]>,
        waiting_release_gate: Box<[ManeuverGateOrdinal]>,
        waiting_max_occupancy: Box<[u32]>,
        group_controller: Box<[SignalControllerOrdinal]>,
        group_gate_ranges: Box<[RangeU32]>,
        group_gates: Box<[ManeuverGateOrdinal]>,
        controller_offset_ms: Box<[u64]>,
        controller_cycle_ms: Box<[u64]>,
        controller_group_ranges: Box<[RangeU32]>,
        controller_groups: Box<[SignalGroupOrdinal]>,
        controller_phase_ranges: Box<[RangeU32]>,
        controller_phases: Box<[SignalPhaseOrdinal]>,
        phase_controller: Box<[SignalControllerOrdinal]>,
        phase_duration_ms: Box<[u64]>,
        phase_end_offset_ms: Box<[u64]>,
        phase_state_ranges: Box<[RangeU32]>,
        phase_state_groups: Box<[SignalGroupOrdinal]>,
        phase_state_aspects: Box<[SignalAspect]>,
        parking_space_ranges: Box<[RangeU32]>,
        parking_spaces: Box<[ParkingSpaceOrdinal]>,
        space_area: OptionalColumn<ParkingAreaOrdinal>,
        space_entry_edge: Box<[LaneEdgeOrdinal]>,
        space_entry_progress: Box<[f64]>,
        space_exit_edge: Box<[LaneEdgeOrdinal]>,
        space_exit_progress: Box<[f64]>,
        space_lateral: Box<[f64]>,
        space_heading: Box<[f64]>,
        space_length: Box<[f64]>,
        space_width: Box<[f64]>,
        lane_group_section: Box<[RoadSectionOrdinal]>,
        lane_group_member_ranges: Box<[RangeU32]>,
        lane_group_members: Box<[AuthoringLaneOrdinal]>,
        band_corridor: Box<[RoadCorridorOrdinal]>,
        band_kind: Box<[FacilityKind]>,
        class_parent: OptionalColumn<ParticipantClassOrdinal>,
        class_depth: Box<[u32]>,
        class_subtree_enter: Box<[u32]>,
        class_subtree_exit: Box<[u32]>,
        rule_target: Box<[AccessTarget]>,
        rule_effect: Box<[AccessEffect]>,
        rule_class_ranges: Box<[RangeU32]>,
        rule_classes: Box<[ParticipantClassOrdinal]>,
        rule_priority: Box<[i32]>,
        profile_class: Box<[ParticipantClassOrdinal]>,
        profile_length: Box<[f64]>,
        profile_desired_speed: Box<[f64]>,
        profile_min_gap: Box<[f64]>,
        profile_time_headway: Box<[f64]>,
        profile_max_accel: Box<[f64]>,
        profile_comfort_decel: Box<[f64]>,
        profile_emergency_decel: Box<[f64]>,
        route_edge_ranges: Box<[RangeU32]>,
        route_edges: Box<[LaneEdgeOrdinal]>,
        route_gate_ranges: Box<[RangeU32]>,
        route_transition_gates: Box<[Option<ManeuverGateOrdinal>]>,
        route_maneuver_ranges: Box<[RangeU32]>,
        route_maneuver_paths: Box<[ManeuverPathOrdinal]>,
        route_maneuver_entry: Box<[u32]>,
        route_maneuver_exit: Box<[u32]>,
        route_maneuver_gate_occ_start: Box<[u32]>,
        route_maneuver_gate_occ_count: Box<[u32]>,
        route_maneuver_waiting_occ_start: Box<[u32]>,
        route_maneuver_waiting_occ_count: Box<[u32]>,
        route_gate_occ_ranges: Box<[RangeU32]>,
        route_gate_occ_gates: Box<[ManeuverGateOrdinal]>,
        route_gate_occ_maneuver: Box<[u32]>,
        route_gate_occ_from: Box<[u32]>,
        route_gate_occ_next: Box<[Option<u32>]>,
        route_gate_occ_next_boundary: Box<[u32]>,
        route_gate_occ_waiting: Box<[Option<u32>]>,
        route_waiting_occ_ranges: Box<[RangeU32]>,
        route_waiting_occ_zones: Box<[WaitingZoneOrdinal]>,
        route_waiting_occ_maneuver: Box<[u32]>,
        route_waiting_occ_entry_gate: Box<[u32]>,
        route_waiting_occ_release_gate: Box<[u32]>,
        route_waiting_occ_entry_edge: Box<[u32]>,
        route_waiting_occ_release_edge: Box<[u32]>,
        route_reverse_kind: Box<[u16]>,
        route_reverse_ordinal: Box<[u32]>,
        route_reverse_route: Box<[StaticRouteOrdinal]>,
        route_reverse_occurrence: Box<[u32]>,
        route_distance_to_end: Box<[BoundedDistance]>,
        route_distance_ranges: Box<[RangeU32]>,
        route_distance_segments: Box<[u32]>,
        route_distance_offsets: Box<[f64]>,
        route_segment_totals: Box<[f64]>,
        route_segment_ranges: Box<[RangeU32]>,
        next_controlled_gate: Box<[Option<ManeuverGateOrdinal>]>,
        next_controlled_from: Box<[u32]>,
        next_controlled_distance: Box<[BoundedDistance]>,
        speed_limit_from: Box<[u32]>,
        speed_limit_to_edge: Box<[LaneEdgeOrdinal]>,
        speed_limit_target: Box<[f64]>,
        speed_limit_ranges: Box<[RangeU32]>,
        access_class_count: u32,
        edge_row_starts: Box<[u32]>,
        edge_cells: Box<[AccessCell]>,
        path_row_starts: Box<[u32]>,
        path_cells: Box<[AccessCell]>,
    ) -> SharedRelationClosure {
        SharedRelationClosure {
            intern,
            corridor_reference_section,
            corridor_element_ranges,
            corridor_elements,
            section_corridor,
            section_kind,
            section_lane_ranges,
            section_lanes,
            authoring_section,
            authoring_edge_ranges,
            authoring_edges,
            authoring_group,
            edge_authoring_lane,
            edge_junction,
            edge_stop_line,
            junction_movement_ranges,
            junction_movements,
            movement_junction,
            movement_path_ranges,
            movement_paths,
            stop_line_edge,
            stop_line_gate_ranges,
            stop_line_gates,
            gate_path,
            gate_transition_index,
            gate_stop_line,
            gate_signal_group,
            waiting_path,
            waiting_entry_gate,
            waiting_release_gate,
            waiting_max_occupancy,
            group_controller,
            group_gate_ranges,
            group_gates,
            controller_offset_ms,
            controller_cycle_ms,
            controller_group_ranges,
            controller_groups,
            controller_phase_ranges,
            controller_phases,
            phase_controller,
            phase_duration_ms,
            phase_end_offset_ms,
            phase_state_ranges,
            phase_state_groups,
            phase_state_aspects,
            parking_space_ranges,
            parking_spaces,
            space_area,
            space_entry_edge,
            space_entry_progress,
            space_exit_edge,
            space_exit_progress,
            space_lateral,
            space_heading,
            space_length,
            space_width,
            lane_group_section,
            lane_group_member_ranges,
            lane_group_members,
            band_corridor,
            band_kind,
            class_parent,
            class_depth,
            class_subtree_enter,
            class_subtree_exit,
            rule_target,
            rule_effect,
            rule_class_ranges,
            rule_classes,
            rule_priority,
            profile_class,
            profile_length,
            profile_desired_speed,
            profile_min_gap,
            profile_time_headway,
            profile_max_accel,
            profile_comfort_decel,
            profile_emergency_decel,
            route_edge_ranges,
            route_edges,
            route_gate_ranges,
            route_transition_gates,
            route_maneuver_ranges,
            route_maneuver_paths,
            route_maneuver_entry,
            route_maneuver_exit,
            route_maneuver_gate_occ_start,
            route_maneuver_gate_occ_count,
            route_maneuver_waiting_occ_start,
            route_maneuver_waiting_occ_count,
            route_gate_occ_ranges,
            route_gate_occ_gates,
            route_gate_occ_maneuver,
            route_gate_occ_from,
            route_gate_occ_next,
            route_gate_occ_next_boundary,
            route_gate_occ_waiting,
            route_waiting_occ_ranges,
            route_waiting_occ_zones,
            route_waiting_occ_maneuver,
            route_waiting_occ_entry_gate,
            route_waiting_occ_release_gate,
            route_waiting_occ_entry_edge,
            route_waiting_occ_release_edge,
            route_reverse_kind,
            route_reverse_ordinal,
            route_reverse_route,
            route_reverse_occurrence,
            route_distance_to_end,
            route_distance_ranges,
            route_distance_segments,
            route_distance_offsets,
            route_segment_totals,
            route_segment_ranges,
            next_controlled_gate,
            next_controlled_from,
            next_controlled_distance,
            speed_limit_from,
            speed_limit_to_edge,
            speed_limit_target,
            speed_limit_ranges,
            access_class_count,
            edge_row_starts,
            edge_cells,
            path_row_starts,
            path_cells,
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_for_tests(lane_count: u32) -> SharedRelationClosure {
        let lane = usize::try_from(lane_count).expect("u32 fits");
        assemble(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            empty_optional(0).expect("empty"),
            empty_optional(lane_count).expect("lane optional"),
            empty_optional(lane_count).expect("lane optional"),
            empty_optional(lane_count).expect("lane optional"),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            empty_optional(0).expect("empty"),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            empty_optional(0).expect("empty"),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            empty_optional(0).expect("empty"),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            0,
            vec![UNCONSTRAINED_ROW; lane].into_boxed_slice(),
            Box::new([]),
            Box::new([]),
            Box::new([]),
        )
    }
}
