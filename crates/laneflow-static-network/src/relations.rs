use core::mem::size_of;

use laneflow_static_contract::{
    AccessEffect, AccessRuleOrdinal, AuthoringLaneOrdinal, FacilityBandOrdinal, JunctionOrdinal,
    LaneEdgeOrdinal, LaneGroupOrdinal, ManeuverGateOrdinal, ManeuverPathOrdinal, MovementOrdinal,
    ParkingAreaOrdinal, ParkingSpaceOrdinal, ParticipantClassOrdinal, RoadCorridorOrdinal,
    RoadSectionOrdinal, SignalAspect, SignalControllerOrdinal, SignalGroupOrdinal,
    SignalPhaseOrdinal, StaticRouteOrdinal, StopLineOrdinal, WaitingZoneOrdinal,
};

use crate::RangeU32;
use crate::traffic::logical_bytes;

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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessCell {
    Unconstrained,
    Decided {
        rule: AccessRuleOrdinal,
        effect: AccessEffect,
    },
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
    route_gate_occ_ranges: Box<[RangeU32]>,
    route_gate_occ_gates: Box<[ManeuverGateOrdinal]>,
    route_waiting_occ_ranges: Box<[RangeU32]>,
    route_waiting_occ_zones: Box<[WaitingZoneOrdinal]>,
    route_distance_to_end: Box<[f64]>,
    route_distance_ranges: Box<[RangeU32]>,
    next_controlled_gate: Box<[Option<ManeuverGateOrdinal>]>,
    next_controlled_from: Box<[u32]>,
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
    pub fn parking_space_entry(
        &self,
        space: ParkingSpaceOrdinal,
    ) -> Option<(LaneEdgeOrdinal, f64)> {
        Some((
            *self.space_entry_edge.get(space.index())?,
            *self.space_entry_progress.get(space.index())?,
        ))
    }

    #[must_use]
    pub fn parking_space_geometry(
        &self,
        space: ParkingSpaceOrdinal,
    ) -> Option<(f64, f64, f64, f64)> {
        Some((
            *self.space_lateral.get(space.index())?,
            *self.space_heading.get(space.index())?,
            *self.space_length.get(space.index())?,
            *self.space_width.get(space.index())?,
        ))
    }

    #[must_use]
    pub fn edge_access(&self, edge: LaneEdgeOrdinal, class: ParticipantClassOrdinal) -> AccessCell {
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
    ) -> AccessCell {
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
    pub fn route_distance_to_end(&self, route: StaticRouteOrdinal) -> Option<&[f64]> {
        Some(
            self.route_distance_ranges
                .get(route.index())?
                .slice(&self.route_distance_to_end),
        )
    }

    #[must_use]
    pub fn next_controlled_transition(
        &self,
        route: StaticRouteOrdinal,
        edge_index: usize,
    ) -> Option<ManeuverGateOrdinal> {
        let range = *self.route_edge_ranges.get(route.index())?;
        if edge_index >= usize::try_from(range.len()).ok()? {
            return None;
        }
        let index = usize::try_from(range.start()).ok()? + edge_index;
        self.next_controlled_gate.get(index).copied().flatten()
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
        self.gate_signal_group.get(gate.index())
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
            + logical_bytes::<RangeU32>(self.route_gate_occ_ranges.len())
            + logical_bytes::<ManeuverGateOrdinal>(self.route_gate_occ_gates.len())
            + logical_bytes::<RangeU32>(self.route_waiting_occ_ranges.len())
            + logical_bytes::<WaitingZoneOrdinal>(self.route_waiting_occ_zones.len())
            + logical_bytes::<f64>(self.route_distance_to_end.len())
            + logical_bytes::<RangeU32>(self.route_distance_ranges.len())
            + logical_bytes::<Option<ManeuverGateOrdinal>>(self.next_controlled_gate.len())
            + logical_bytes::<u32>(self.next_controlled_from.len())
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

fn plane_cell(
    row_starts: &[u32],
    cells: &[AccessCell],
    class_count: u32,
    unit: usize,
    class: usize,
) -> AccessCell {
    let Some(&start) = row_starts.get(unit) else {
        return AccessCell::Unconstrained;
    };
    if start == UNCONSTRAINED_ROW {
        return AccessCell::Unconstrained;
    }
    if class >= usize::try_from(class_count).unwrap_or(0) {
        return AccessCell::Unconstrained;
    }
    cells
        .get(usize::try_from(start).unwrap_or(0) + class)
        .copied()
        .unwrap_or(AccessCell::Unconstrained)
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
        route_gate_occ_ranges: Box<[RangeU32]>,
        route_gate_occ_gates: Box<[ManeuverGateOrdinal]>,
        route_waiting_occ_ranges: Box<[RangeU32]>,
        route_waiting_occ_zones: Box<[WaitingZoneOrdinal]>,
        route_distance_to_end: Box<[f64]>,
        route_distance_ranges: Box<[RangeU32]>,
        next_controlled_gate: Box<[Option<ManeuverGateOrdinal>]>,
        next_controlled_from: Box<[u32]>,
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
            route_gate_occ_ranges,
            route_gate_occ_gates,
            route_waiting_occ_ranges,
            route_waiting_occ_zones,
            route_distance_to_end,
            route_distance_ranges,
            next_controlled_gate,
            next_controlled_from,
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
            0,
            vec![UNCONSTRAINED_ROW; lane].into_boxed_slice(),
            Box::new([]),
            Box::new([]),
            Box::new([]),
        )
    }
}
