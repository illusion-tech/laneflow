use laneflow_static_contract::{
    AccessEffect, AccessRuleOrdinal, AuthoringLaneOrdinal, EntityKind, FacilityBandOrdinal,
    JunctionOrdinal, LaneEdgeOrdinal, LaneGroupOrdinal, ManeuverGateOrdinal, ManeuverPathOrdinal,
    MovementOrdinal, ParkingFacilityOrdinal, ParkingSpaceOrdinal, ParticipantClassOrdinal,
    RoadCorridorOrdinal, RoadSectionOrdinal, SignalAspect, SignalControllerOrdinal,
    SignalGroupOrdinal, SignalPhaseOrdinal, StopLineOrdinal, VehicleProfileOrdinal,
    WaitingZoneOrdinal,
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

/// 有界距离：Finite 侧是 `u32` 毫米。溢出是 `BeyondFinite`，禁止饱和成 `u32::MAX`。
///
/// 不上 `u64`：城市一趟行程（Spatial 单 frame 约 32 km，通勤/过境几十公里）落在
/// `u32` 满量程约 4295 km 之内。为「理论最长边序列 × 10 km」加宽会把查询面变成
/// 另一套整数合同（ADR 0028）。占用间隙的 `i64` 只服务有符号空隙，不是前缀先例。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundedDistance {
    Finite(u32),
    BeyondFinite,
}

impl BoundedDistance {
    #[must_use]
    pub fn add_u32(self, value: u32) -> Self {
        match self {
            Self::Finite(current) => current
                .checked_add(value)
                .map(Self::Finite)
                .unwrap_or(Self::BeyondFinite),
            Self::BeyondFinite => Self::BeyondFinite,
        }
    }

    /// 两段有界距离相加。任一端越界或 `u32` 溢出则为 `BeyondFinite`，不上 `u64`。
    #[must_use]
    pub fn add_bounded(self, other: Self) -> Self {
        match (self, other) {
            (Self::Finite(left), Self::Finite(right)) => left
                .checked_add(right)
                .map(Self::Finite)
                .unwrap_or(Self::BeyondFinite),
            _ => Self::BeyondFinite,
        }
    }

    /// 从 Finite 后缀扣边内进度。`BeyondFinite` 保持越界，不上 `u64`。
    #[must_use]
    pub fn saturating_sub(self, value: u32) -> Self {
        match self {
            Self::Finite(current) => Self::Finite(current.saturating_sub(value)),
            Self::BeyondFinite => Self::BeyondFinite,
        }
    }

    /// 两个后缀相减得到窗口距离。两端都越界时差仍越界，不上 `u64`。
    #[must_use]
    pub fn saturating_sub_bounded(self, other: Self) -> Self {
        match (self, other) {
            (Self::Finite(left), Self::Finite(right)) => Self::Finite(left.saturating_sub(right)),
            (Self::BeyondFinite, Self::Finite(_)) => Self::BeyondFinite,
            (Self::Finite(_), Self::BeyondFinite) => Self::Finite(0),
            (Self::BeyondFinite, Self::BeyondFinite) => Self::BeyondFinite,
        }
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
    area: Option<ParkingFacilityOrdinal>,
    entry_edge: LaneEdgeOrdinal,
    entry_progress_mm: u32,
    exit_edge: LaneEdgeOrdinal,
    exit_progress_mm: u32,
    lateral_mm: i32,
    heading: f32,
    length_mm: u32,
    width_mm: u32,
}

impl ParkingSpaceView {
    #[must_use]
    pub const fn area(self) -> Option<ParkingFacilityOrdinal> {
        self.area
    }

    #[must_use]
    pub const fn entry(self) -> (LaneEdgeOrdinal, u32) {
        (self.entry_edge, self.entry_progress_mm)
    }

    #[must_use]
    pub const fn exit(self) -> (LaneEdgeOrdinal, u32) {
        (self.exit_edge, self.exit_progress_mm)
    }

    #[must_use]
    pub const fn geometry(self) -> (i32, f32, u32, u32) {
        (self.lateral_mm, self.heading, self.length_mm, self.width_mm)
    }
}

/// 停车设施虚拟容量使用的 LaneEdge 内部锚点。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParkingLaneAnchor {
    pub(crate) lane_edge: LaneEdgeOrdinal,
    pub(crate) progress_mm: u32,
}

impl ParkingLaneAnchor {
    #[must_use]
    pub const fn lane_edge(self) -> LaneEdgeOrdinal {
        self.lane_edge
    }

    #[must_use]
    pub const fn progress_mm(self) -> u32 {
        self.progress_mm
    }
}

/// 停车设施的显式泊位与虚拟容量闭包。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParkingFacilityView<'a> {
    spaces: &'a [ParkingSpaceOrdinal],
    virtual_capacity: u32,
    virtual_entries: &'a [ParkingLaneAnchor],
    virtual_exits: &'a [ParkingLaneAnchor],
}

impl<'a> ParkingFacilityView<'a> {
    #[must_use]
    pub const fn spaces(self) -> &'a [ParkingSpaceOrdinal] {
        self.spaces
    }

    #[must_use]
    pub const fn virtual_capacity(self) -> u32 {
        self.virtual_capacity
    }

    #[must_use]
    pub fn total_capacity(self) -> u64 {
        u64::try_from(self.spaces.len()).expect("validated LFCA count fits u64")
            + u64::from(self.virtual_capacity)
    }

    #[must_use]
    pub const fn virtual_entries(self) -> &'a [ParkingLaneAnchor] {
        self.virtual_entries
    }

    #[must_use]
    pub const fn virtual_exits(self) -> &'a [ParkingLaneAnchor] {
        self.virtual_exits
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
    length_mm: u32,
    desired_speed_mm_s: u32,
    min_gap_mm: u32,
    time_headway: f32,
    max_accel: f32,
    comfort_decel: f32,
    emergency_decel: f32,
}

impl VehicleProfileView {
    #[must_use]
    pub const fn class(self) -> ParticipantClassOrdinal {
        self.class
    }

    #[must_use]
    pub const fn length_mm(self) -> u32 {
        self.length_mm
    }

    #[must_use]
    pub const fn desired_speed_mm_s(self) -> u32 {
        self.desired_speed_mm_s
    }

    #[must_use]
    pub const fn min_gap_mm(self) -> u32 {
        self.min_gap_mm
    }

    #[must_use]
    pub const fn time_headway(self) -> f32 {
        self.time_headway
    }

    #[must_use]
    pub const fn max_accel(self) -> f32 {
        self.max_accel
    }

    #[must_use]
    pub const fn comfort_decel(self) -> f32 {
        self.comfort_decel
    }

    #[must_use]
    pub const fn emergency_decel(self) -> f32 {
        self.emergency_decel
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
    parking_virtual_capacity: Box<[u32]>,
    parking_virtual_entry_ranges: Box<[RangeU32]>,
    parking_virtual_entries: Box<[ParkingLaneAnchor]>,
    parking_virtual_exit_ranges: Box<[RangeU32]>,
    parking_virtual_exits: Box<[ParkingLaneAnchor]>,
    space_area: OptionalColumn<ParkingFacilityOrdinal>,
    space_entry_edge: Box<[LaneEdgeOrdinal]>,
    space_entry_progress: Box<[u32]>,
    space_exit_edge: Box<[LaneEdgeOrdinal]>,
    space_exit_progress: Box<[u32]>,
    space_lateral: Box<[i32]>,
    space_heading: Box<[f32]>,
    space_length: Box<[u32]>,
    space_width: Box<[u32]>,
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
    profile_length: Box<[u32]>,
    profile_desired_speed: Box<[u32]>,
    profile_min_gap: Box<[u32]>,
    profile_time_headway: Box<[f32]>,
    profile_max_accel: Box<[f32]>,
    profile_comfort_decel: Box<[f32]>,
    profile_emergency_decel: Box<[f32]>,
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
    pub fn section_corridor(&self, section: RoadSectionOrdinal) -> Option<RoadCorridorOrdinal> {
        self.section_corridor.get(section.index()).copied()
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
    pub fn authoring_section(&self, lane: AuthoringLaneOrdinal) -> Option<RoadSectionOrdinal> {
        self.authoring_section.get(lane.index()).copied()
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
    pub fn movement_junction(&self, movement: MovementOrdinal) -> Option<JunctionOrdinal> {
        self.movement_junction.get(movement.index()).copied()
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
    pub fn parking_facility(
        &self,
        facility: ParkingFacilityOrdinal,
    ) -> Option<ParkingFacilityView<'_>> {
        Some(ParkingFacilityView {
            spaces: self
                .parking_space_ranges
                .get(facility.index())?
                .slice(&self.parking_spaces),
            virtual_capacity: *self.parking_virtual_capacity.get(facility.index())?,
            virtual_entries: self
                .parking_virtual_entry_ranges
                .get(facility.index())?
                .slice(&self.parking_virtual_entries),
            virtual_exits: self
                .parking_virtual_exit_ranges
                .get(facility.index())?
                .slice(&self.parking_virtual_exits),
        })
    }

    #[must_use]
    pub fn parking_facility_spaces(
        &self,
        facility: ParkingFacilityOrdinal,
    ) -> Option<&[ParkingSpaceOrdinal]> {
        self.parking_facility(facility)
            .map(ParkingFacilityView::spaces)
    }

    #[must_use]
    pub fn parking_space(&self, space: ParkingSpaceOrdinal) -> Option<ParkingSpaceView> {
        Some(ParkingSpaceView {
            area: self.space_area.get(space.index()),
            entry_edge: *self.space_entry_edge.get(space.index())?,
            entry_progress_mm: *self.space_entry_progress.get(space.index())?,
            exit_edge: *self.space_exit_edge.get(space.index())?,
            exit_progress_mm: *self.space_exit_progress.get(space.index())?,
            lateral_mm: *self.space_lateral.get(space.index())?,
            heading: *self.space_heading.get(space.index())?,
            length_mm: *self.space_length.get(space.index())?,
            width_mm: *self.space_width.get(space.index())?,
        })
    }

    #[must_use]
    pub fn parking_space_entry(
        &self,
        space: ParkingSpaceOrdinal,
    ) -> Option<(LaneEdgeOrdinal, u32)> {
        self.parking_space(space).map(|view| view.entry())
    }

    #[must_use]
    pub fn parking_space_geometry(
        &self,
        space: ParkingSpaceOrdinal,
    ) -> Option<(i32, f32, u32, u32)> {
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
            length_mm: *self.profile_length.get(profile.index())?,
            desired_speed_mm_s: *self.profile_desired_speed.get(profile.index())?,
            min_gap_mm: *self.profile_min_gap.get(profile.index())?,
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
    pub fn lane_group_members(&self, group: LaneGroupOrdinal) -> Option<&[AuthoringLaneOrdinal]> {
        Some(
            self.lane_group_member_ranges
                .get(group.index())?
                .slice(&self.lane_group_members),
        )
    }

    #[must_use]
    pub fn lane_group_section(&self, group: LaneGroupOrdinal) -> Option<RoadSectionOrdinal> {
        self.lane_group_section.get(group.index()).copied()
    }

    #[must_use]
    pub fn band_kind(&self, band: FacilityBandOrdinal) -> Option<FacilityKind> {
        self.band_kind.get(band.index()).copied()
    }

    #[must_use]
    pub fn band_corridor(&self, band: FacilityBandOrdinal) -> Option<RoadCorridorOrdinal> {
        self.band_corridor.get(band.index()).copied()
    }

    #[must_use]
    pub fn gate_signal_group(&self, gate: ManeuverGateOrdinal) -> Option<SignalGroupOrdinal> {
        self.maneuver_gate(gate)?.signal_group()
    }

    #[must_use]
    pub fn retained_logical_bytes(&self) -> u64 {
        logical_bytes::<Box<str>>(self.intern.len())
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
            + logical_bytes::<u32>(self.parking_virtual_capacity.len())
            + logical_bytes::<RangeU32>(self.parking_virtual_entry_ranges.len())
            + logical_bytes::<ParkingLaneAnchor>(self.parking_virtual_entries.len())
            + logical_bytes::<RangeU32>(self.parking_virtual_exit_ranges.len())
            + logical_bytes::<ParkingLaneAnchor>(self.parking_virtual_exits.len())
            + self.space_area.retained_bytes()
            + logical_bytes::<LaneEdgeOrdinal>(self.space_entry_edge.len())
            + logical_bytes::<u32>(self.space_entry_progress.len())
            + logical_bytes::<LaneEdgeOrdinal>(self.space_exit_edge.len())
            + logical_bytes::<u32>(self.space_exit_progress.len())
            + logical_bytes::<i32>(self.space_lateral.len())
            + logical_bytes::<f32>(self.space_heading.len())
            + logical_bytes::<u32>(self.space_length.len())
            + logical_bytes::<u32>(self.space_width.len())
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
            + logical_bytes::<u32>(self.profile_length.len())
            + logical_bytes::<u32>(self.profile_desired_speed.len())
            + logical_bytes::<u32>(self.profile_min_gap.len())
            + logical_bytes::<f32>(self.profile_time_headway.len())
            + logical_bytes::<f32>(self.profile_max_accel.len())
            + logical_bytes::<f32>(self.profile_comfort_decel.len())
            + logical_bytes::<f32>(self.profile_emergency_decel.len())
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

fn floor_add_bytes(total: u64, bytes: u32) -> Result<u64, BuildError> {
    total
        .checked_add(u64::from(bytes))
        .ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::RetainedOutput,
        })
}

#[derive(Clone, Copy, Default)]
pub(crate) struct RelationPayloads {
    pub corridor_elements: u32,
    pub section_lanes: u32,
    pub authoring_edges: u32,
    pub junction_movements: u32,
    pub movement_paths: u32,
    pub stop_line_gates: u32,
    pub group_gates: u32,
    pub controller_groups: u32,
    pub controller_phases: u32,
    pub phase_states: u32,
    pub parking_spaces: u32,
    pub parking_virtual_entries: u32,
    pub parking_virtual_exits: u32,
    pub lane_group_members: u32,
    pub rule_classes: u32,
    pub intern_keys: u32,
    pub intern_utf8: u32,
    pub edge_cells: u32,
    pub path_cells: u32,
    pub pass_a_scratch: u64,
}

pub(crate) fn relation_retained_floor(
    counts: &EntityCounts,
    payloads: RelationPayloads,
) -> Result<u64, BuildError> {
    // 闭合对象内联在 SharedNetworkRevision 中，对象头已计入根 size_of。
    let mut total = 0_u64;
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
    let area = counts.count(EntityKind::ParkingFacility);
    let space = counts.count(EntityKind::ParkingSpace);
    let lane_group = counts.count(EntityKind::LaneGroup);
    let band = counts.count(EntityKind::FacilityBand);
    let class = counts.count(EntityKind::ParticipantClass);
    let rule = counts.count(EntityKind::AccessRule);
    let profile = counts.count(EntityKind::VehicleProfile);
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
    total = floor_add::<u32>(total, area)?;
    total = floor_add::<RangeU32>(total, area)?;
    total = floor_add::<RangeU32>(total, area)?;
    total = floor_add::<Option<ParkingFacilityOrdinal>>(total, space)?;
    total = floor_add::<LaneEdgeOrdinal>(total, space)?;
    total = floor_add::<u32>(total, space)?;
    total = floor_add::<LaneEdgeOrdinal>(total, space)?;
    total = floor_add::<u32>(total, space)?;
    total = floor_add::<i32>(total, space)?;
    total = floor_add::<f32>(total, space)?;
    total = floor_add::<u32>(total, space)?;
    total = floor_add::<u32>(total, space)?;
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
    total = floor_add::<u32>(total, profile)?;
    total = floor_add::<u32>(total, profile)?;
    total = floor_add::<u32>(total, profile)?;
    total = floor_add::<f32>(total, profile)?;
    total = floor_add::<f32>(total, profile)?;
    total = floor_add::<f32>(total, profile)?;
    total = floor_add::<f32>(total, profile)?;
    total = floor_add::<u32>(total, lane)?;
    total = floor_add::<u32>(total, path)?;
    total = floor_add::<CorridorElement>(total, payloads.corridor_elements)?;
    total = floor_add::<AuthoringLaneOrdinal>(total, payloads.section_lanes)?;
    total = floor_add::<LaneEdgeOrdinal>(total, payloads.authoring_edges)?;
    total = floor_add::<MovementOrdinal>(total, payloads.junction_movements)?;
    total = floor_add::<ManeuverPathOrdinal>(total, payloads.movement_paths)?;
    total = floor_add::<ManeuverGateOrdinal>(total, payloads.stop_line_gates)?;
    total = floor_add::<ManeuverGateOrdinal>(total, payloads.group_gates)?;
    total = floor_add::<SignalGroupOrdinal>(total, payloads.controller_groups)?;
    total = floor_add::<SignalPhaseOrdinal>(total, payloads.controller_phases)?;
    total = floor_add::<SignalGroupOrdinal>(total, payloads.phase_states)?;
    total = floor_add::<SignalAspect>(total, payloads.phase_states)?;
    total = floor_add::<ParkingSpaceOrdinal>(total, payloads.parking_spaces)?;
    total = floor_add::<ParkingLaneAnchor>(total, payloads.parking_virtual_entries)?;
    total = floor_add::<ParkingLaneAnchor>(total, payloads.parking_virtual_exits)?;
    total = floor_add::<AuthoringLaneOrdinal>(total, payloads.lane_group_members)?;
    total = floor_add::<ParticipantClassOrdinal>(total, payloads.rule_classes)?;
    total = floor_add::<Box<str>>(total, payloads.intern_keys)?;
    total = floor_add_bytes(total, payloads.intern_utf8)?;
    total = floor_add::<AccessCell>(total, payloads.edge_cells)?;
    total = floor_add::<AccessCell>(total, payloads.path_cells)?;
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
        parking_virtual_capacity: Box<[u32]>,
        parking_virtual_entry_ranges: Box<[RangeU32]>,
        parking_virtual_entries: Box<[ParkingLaneAnchor]>,
        parking_virtual_exit_ranges: Box<[RangeU32]>,
        parking_virtual_exits: Box<[ParkingLaneAnchor]>,
        space_area: OptionalColumn<ParkingFacilityOrdinal>,
        space_entry_edge: Box<[LaneEdgeOrdinal]>,
        space_entry_progress: Box<[u32]>,
        space_exit_edge: Box<[LaneEdgeOrdinal]>,
        space_exit_progress: Box<[u32]>,
        space_lateral: Box<[i32]>,
        space_heading: Box<[f32]>,
        space_length: Box<[u32]>,
        space_width: Box<[u32]>,
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
        profile_length: Box<[u32]>,
        profile_desired_speed: Box<[u32]>,
        profile_min_gap: Box<[u32]>,
        profile_time_headway: Box<[f32]>,
        profile_max_accel: Box<[f32]>,
        profile_comfort_decel: Box<[f32]>,
        profile_emergency_decel: Box<[f32]>,
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
            parking_virtual_capacity,
            parking_virtual_entry_ranges,
            parking_virtual_entries,
            parking_virtual_exit_ranges,
            parking_virtual_exits,
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
            // corridor / section / authoring
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
            // junction / movement / gates
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
            // waiting / signals / parking facility payloads
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
            empty_optional(0).expect("empty"),
            // parking spaces / lane groups / facility bands
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
            // participant classes / rules / profiles
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
