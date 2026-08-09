//! 当前 v0.10 JSON 格式的 wire DTO。
//!
//! record 类型字段 `pub(crate)`，跨包消费只经逐项借用 accessor；反序列化由
//! [`crate::parse`] 的手写单遍解析器实现，serde 行为（`deny_unknown_fields`、
//! explicit-null 拒绝、priority 原始词法、untagged 枚举）与 `laneflow-data`
//! 迁移前逐字节一致。

#[doc(hidden)]
#[derive(Debug)]
pub struct WirePackage {
    pub(crate) format_version: String,
    pub(crate) units: WireUnits,
    pub(crate) lane_graph: WireLaneGraph,
    pub(crate) junctions: Vec<WireJunction>,
    pub(crate) movements: Vec<WireMovement>,
    pub(crate) maneuver_paths: Vec<WireManeuverPath>,
    pub(crate) routes: Vec<WireRoute>,
    pub(crate) vehicle_profiles: Vec<WireVehicleProfile>,
    pub(crate) participant_classes: Vec<WireParticipantClass>,
    pub(crate) facility_bands: Vec<WireFacilityBand>,
    pub(crate) road_sections: Vec<WireRoadSection>,
    pub(crate) lane_groups: Vec<WireLaneGroup>,
    pub(crate) road_corridors: Vec<WireRoadCorridor>,
    pub(crate) access_rules: Vec<WireAccessRule>,
    pub(crate) waiting_zones: Vec<WireWaitingZone>,
    pub(crate) signals: WireSignals,
    pub(crate) parking: WireParking,
}

impl WirePackage {
    pub fn units(&self) -> &WireUnits {
        &self.units
    }

    pub fn lane_graph(&self) -> &WireLaneGraph {
        &self.lane_graph
    }

    pub fn junctions(&self) -> &[WireJunction] {
        &self.junctions
    }

    pub fn movements(&self) -> &[WireMovement] {
        &self.movements
    }

    pub fn maneuver_paths(&self) -> &[WireManeuverPath] {
        &self.maneuver_paths
    }

    pub fn routes(&self) -> &[WireRoute] {
        &self.routes
    }

    pub fn vehicle_profiles(&self) -> &[WireVehicleProfile] {
        &self.vehicle_profiles
    }

    pub fn participant_classes(&self) -> &[WireParticipantClass] {
        &self.participant_classes
    }

    pub fn facility_bands(&self) -> &[WireFacilityBand] {
        &self.facility_bands
    }

    pub fn road_sections(&self) -> &[WireRoadSection] {
        &self.road_sections
    }

    pub fn lane_groups(&self) -> &[WireLaneGroup] {
        &self.lane_groups
    }

    pub fn road_corridors(&self) -> &[WireRoadCorridor] {
        &self.road_corridors
    }

    pub fn access_rules(&self) -> &[WireAccessRule] {
        &self.access_rules
    }

    pub fn waiting_zones(&self) -> &[WireWaitingZone] {
        &self.waiting_zones
    }

    pub fn signals(&self) -> &WireSignals {
        &self.signals
    }

    pub fn parking(&self) -> &WireParking {
        &self.parking
    }

    pub(crate) fn format_version(&self) -> &str {
        &self.format_version
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireUnits {
    pub(crate) distance: String,
    pub(crate) time: String,
}

impl WireUnits {
    pub fn distance(&self) -> &str {
        &self.distance
    }

    pub fn time(&self) -> &str {
        &self.time
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireLaneGraph {
    pub(crate) edges: Vec<WireLaneEdge>,
}

impl WireLaneGraph {
    pub fn edges(&self) -> &[WireLaneEdge] {
        &self.edges
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireLaneEdge {
    pub(crate) id: String,
    pub(crate) length: f64,
    pub(crate) speed_limit: f64,
    pub(crate) connections: Vec<WireLaneConnection>,
}

impl WireLaneEdge {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn length(&self) -> f64 {
        self.length
    }

    pub fn speed_limit(&self) -> f64 {
        self.speed_limit
    }

    pub fn connections(&self) -> &[WireLaneConnection] {
        &self.connections
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireLaneConnection {
    pub(crate) to_edge_id: String,
}

impl WireLaneConnection {
    pub fn to_edge_id(&self) -> &str {
        &self.to_edge_id
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireJunction {
    pub(crate) id: String,
}

impl WireJunction {
    pub fn id(&self) -> &str {
        &self.id
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireMovement {
    pub(crate) id: String,
    pub(crate) junction_id: String,
}

impl WireMovement {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn junction_id(&self) -> &str {
        &self.junction_id
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireManeuverPath {
    pub(crate) id: String,
    pub(crate) movement_id: String,
    pub(crate) entry_edge_id: String,
    pub(crate) internal_edge_ids: Vec<String>,
    pub(crate) exit_edge_id: String,
}

impl WireManeuverPath {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn movement_id(&self) -> &str {
        &self.movement_id
    }

    pub fn entry_edge_id(&self) -> &str {
        &self.entry_edge_id
    }

    pub fn internal_edge_ids(&self) -> &[String] {
        &self.internal_edge_ids
    }

    pub fn exit_edge_id(&self) -> &str {
        &self.exit_edge_id
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireRoute {
    pub(crate) id: String,
    pub(crate) edge_ids: Vec<String>,
}

impl WireRoute {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn edge_ids(&self) -> &[String] {
        &self.edge_ids
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireVehicleProfile {
    pub(crate) id: String,
    pub(crate) length: f64,
    pub(crate) model: String,
    pub(crate) desired_speed: f64,
    pub(crate) min_gap: f64,
    pub(crate) time_headway: f64,
    pub(crate) max_acceleration: f64,
    pub(crate) comfortable_deceleration: f64,
    pub(crate) emergency_deceleration: f64,
    pub(crate) participant_class_id: String,
}

impl WireVehicleProfile {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn length(&self) -> f64 {
        self.length
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn desired_speed(&self) -> f64 {
        self.desired_speed
    }

    pub fn min_gap(&self) -> f64 {
        self.min_gap
    }

    pub fn time_headway(&self) -> f64 {
        self.time_headway
    }

    pub fn max_acceleration(&self) -> f64 {
        self.max_acceleration
    }

    pub fn comfortable_deceleration(&self) -> f64 {
        self.comfortable_deceleration
    }

    pub fn emergency_deceleration(&self) -> f64 {
        self.emergency_deceleration
    }

    pub fn participant_class_id(&self) -> &str {
        &self.participant_class_id
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireParticipantClass {
    pub(crate) id: String,
    pub(crate) extends_id: Option<String>,
}

impl WireParticipantClass {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn extends_id(&self) -> Option<&str> {
        self.extends_id.as_deref()
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireFacilityBand {
    pub(crate) id: String,
    pub(crate) kind_id: String,
}

impl WireFacilityBand {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn kind_id(&self) -> &str {
        &self.kind_id
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireRoadSection {
    pub(crate) id: String,
    pub(crate) kind_id: String,
    pub(crate) lanes: Vec<WireSectionLane>,
}

impl WireRoadSection {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn kind_id(&self) -> &str {
        &self.kind_id
    }

    pub fn lanes(&self) -> &[WireSectionLane] {
        &self.lanes
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireSectionLane {
    pub(crate) edge_ids: Vec<String>,
    pub(crate) lane_group_id: Option<String>,
}

impl WireSectionLane {
    pub fn edge_ids(&self) -> &[String] {
        &self.edge_ids
    }

    pub fn lane_group_id(&self) -> Option<&str> {
        self.lane_group_id.as_deref()
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireLaneGroup {
    pub(crate) id: String,
    pub(crate) road_section_id: String,
}

impl WireLaneGroup {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn road_section_id(&self) -> &str {
        &self.road_section_id
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireRoadCorridor {
    pub(crate) id: String,
    pub(crate) reference_section_id: String,
    pub(crate) elements: Vec<WireCorridorElement>,
}

impl WireRoadCorridor {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn reference_section_id(&self) -> &str {
        &self.reference_section_id
    }

    pub fn elements(&self) -> &[WireCorridorElement] {
        &self.elements
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub enum WireCorridorElement {
    Section(WireCorridorSectionElement),
    Band(WireCorridorBandElement),
}

impl WireCorridorElement {
    /// 匹配型 accessor：section 元素返回 `Some`，其余返回 `None`。
    pub fn as_section(&self) -> Option<&WireCorridorSectionElement> {
        match self {
            Self::Section(section) => Some(section),
            _ => None,
        }
    }

    /// 匹配型 accessor：band 元素返回 `Some`，其余返回 `None`。
    pub fn as_band(&self) -> Option<&WireCorridorBandElement> {
        match self {
            Self::Band(band) => Some(band),
            _ => None,
        }
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireCorridorSectionElement {
    pub(crate) section_id: String,
}

impl WireCorridorSectionElement {
    pub fn section_id(&self) -> &str {
        &self.section_id
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireCorridorBandElement {
    pub(crate) band_id: String,
}

impl WireCorridorBandElement {
    pub fn band_id(&self) -> &str {
        &self.band_id
    }
}

// timeWindows 是 v1 不可用 capability：wire 层只校验字段是数组（JSON type
// 检查）并记录是否声明，窗口内容（字段结构、分钟数值）一律不解码。capability
// guard 先于 shape（cross-section-access.md §10 phase 9：能力整体拒绝后其内部
// 细节校验无意义），因此极端数值（1e400 这类 serde_json::Number 无法表示的字
// 面量）、缺字段、未知字段、错误类型都必须先抵达 guard 得到
// AccessCapabilityUnavailable，而不是在 DTO 解码期以 DataError::JsonShape 抢
// 先。capability 未来可用时，窗口子树在 guard 之后的 phase 再做完整
// shape/range 校验（空数组、days 空集、分钟越界等 §10 已规约）。

#[doc(hidden)]
#[derive(Debug)]
pub struct WireAccessRule {
    pub(crate) id: String,
    pub(crate) target: WireAccessTarget,
    pub(crate) effect: WireAccessEffect,
    pub(crate) participant_class_ids: Vec<String>,
    /// 是否声明了 `timeWindows`；窗口内容不透明，wire 层只记 presence。
    pub(crate) time_windows: bool,
    pub(crate) regulation: Option<WireRegulation>,
    /// 未经浮点转换的 priority 原始数值字面量（trim 后）。
    pub(crate) priority: Option<String>,
}

impl WireAccessRule {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn target(&self) -> &WireAccessTarget {
        &self.target
    }

    pub fn effect(&self) -> WireAccessEffect {
        self.effect
    }

    pub fn participant_class_ids(&self) -> &[String] {
        &self.participant_class_ids
    }

    /// timeWindows 是 v1 不可用 capability：wire 层只记录是否声明。
    pub fn has_time_windows(&self) -> bool {
        self.time_windows
    }

    pub fn regulation(&self) -> Option<&WireRegulation> {
        self.regulation.as_ref()
    }

    /// 返回未经浮点转换的 priority 原始数值字面量。
    pub fn priority(&self) -> Option<&str> {
        self.priority.as_deref()
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireAccessTarget {
    pub(crate) kind: WireAccessTargetKind,
    pub(crate) id: String,
}

impl WireAccessTarget {
    pub fn kind(&self) -> WireAccessTargetKind {
        self.kind
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireWaitingZone {
    pub(crate) id: String,
    pub(crate) maneuver_path_id: String,
    pub(crate) entry_gate_id: String,
    pub(crate) release_gate_id: String,
    pub(crate) max_occupancy: u32,
}

impl WireWaitingZone {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn maneuver_path_id(&self) -> &str {
        &self.maneuver_path_id
    }

    pub fn entry_gate_id(&self) -> &str {
        &self.entry_gate_id
    }

    pub fn release_gate_id(&self) -> &str {
        &self.release_gate_id
    }

    pub fn max_occupancy(&self) -> u32 {
        self.max_occupancy
    }
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireAccessTargetKind {
    LaneEdge,
    LaneGroup,
    RoadSection,
    ManeuverPath,
    FacilityBand,
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireAccessEffect {
    Allow,
    Deny,
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireRegulation {
    pub(crate) jurisdiction: String,
    pub(crate) version: String,
    pub(crate) source: Option<String>,
}

impl WireRegulation {
    pub fn jurisdiction(&self) -> &str {
        &self.jurisdiction
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireParking {
    pub(crate) areas: Vec<WireParkingArea>,
    pub(crate) spaces: Vec<WireParkingSpace>,
}

impl WireParking {
    pub fn areas(&self) -> &[WireParkingArea] {
        &self.areas
    }

    pub fn spaces(&self) -> &[WireParkingSpace] {
        &self.spaces
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireParkingArea {
    pub(crate) id: String,
}

impl WireParkingArea {
    pub fn id(&self) -> &str {
        &self.id
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireParkingSpace {
    pub(crate) id: String,
    pub(crate) area_id: Option<String>,
    pub(crate) entry: WireParkingAnchor,
    pub(crate) exit: WireParkingAnchor,
    pub(crate) geometry: WireParkingGeometry,
}

impl WireParkingSpace {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn area_id(&self) -> Option<&str> {
        self.area_id.as_deref()
    }

    pub fn entry(&self) -> &WireParkingAnchor {
        &self.entry
    }

    pub fn exit(&self) -> &WireParkingAnchor {
        &self.exit
    }

    pub fn geometry(&self) -> &WireParkingGeometry {
        &self.geometry
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireParkingAnchor {
    pub(crate) edge_id: String,
    pub(crate) progress: f64,
}

impl WireParkingAnchor {
    pub fn edge_id(&self) -> &str {
        &self.edge_id
    }

    pub fn progress(&self) -> f64 {
        self.progress
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireParkingGeometry {
    pub(crate) lateral_offset: f64,
    pub(crate) heading_offset_radians: f64,
    pub(crate) length: f64,
    pub(crate) width: f64,
}

impl WireParkingGeometry {
    pub fn lateral_offset(&self) -> f64 {
        self.lateral_offset
    }

    pub fn heading_offset_radians(&self) -> f64 {
        self.heading_offset_radians
    }

    pub fn length(&self) -> f64 {
        self.length
    }

    pub fn width(&self) -> f64 {
        self.width
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireSignals {
    pub(crate) stop_lines: Vec<WireStopLine>,
    pub(crate) maneuver_gates: Vec<WireManeuverGate>,
    pub(crate) groups: Vec<WireSignalGroup>,
    pub(crate) controllers: Vec<WireSignalController>,
}

impl WireSignals {
    pub fn stop_lines(&self) -> &[WireStopLine] {
        &self.stop_lines
    }

    pub fn maneuver_gates(&self) -> &[WireManeuverGate] {
        &self.maneuver_gates
    }

    pub fn groups(&self) -> &[WireSignalGroup] {
        &self.groups
    }

    pub fn controllers(&self) -> &[WireSignalController] {
        &self.controllers
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireStopLine {
    pub(crate) id: String,
    pub(crate) edge_id: String,
    pub(crate) location: WireStopLineLocation,
}

impl WireStopLine {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn edge_id(&self) -> &str {
        &self.edge_id
    }

    pub fn location(&self) -> WireStopLineLocation {
        self.location
    }
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireStopLineLocation {
    EdgeEnd,
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireManeuverGate {
    pub(crate) id: String,
    pub(crate) maneuver_path_id: String,
    pub(crate) transition_index: u32,
    pub(crate) stop_line_id: String,
    pub(crate) signal_control: WireSignalControl,
}

impl WireManeuverGate {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn maneuver_path_id(&self) -> &str {
        &self.maneuver_path_id
    }

    pub fn transition_index(&self) -> u32 {
        self.transition_index
    }

    pub fn stop_line_id(&self) -> &str {
        &self.stop_line_id
    }

    pub fn signal_control(&self) -> &WireSignalControl {
        &self.signal_control
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub enum WireSignalControl {
    Group(WireGroupSignalControl),
    None(WireNoneSignalControl),
}

impl WireSignalControl {
    /// 匹配型 accessor：group 控制返回 `Some`，其余返回 `None`。
    pub fn as_group(&self) -> Option<&WireGroupSignalControl> {
        match self {
            Self::Group(control) => Some(control),
            _ => None,
        }
    }

    /// 匹配型 accessor：无信号控制返回 `Some`，其余返回 `None`。
    pub fn as_none(&self) -> Option<&WireNoneSignalControl> {
        match self {
            Self::None(control) => Some(control),
            _ => None,
        }
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireGroupSignalControl {
    pub(crate) kind: WireGroupSignalControlKind,
    pub(crate) group_id: String,
}

impl WireGroupSignalControl {
    pub fn kind(&self) -> WireGroupSignalControlKind {
        self.kind
    }

    pub fn group_id(&self) -> &str {
        &self.group_id
    }
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireGroupSignalControlKind {
    Group,
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireNoneSignalControl {
    pub(crate) kind: WireNoneSignalControlKind,
}

impl WireNoneSignalControl {
    pub fn kind(&self) -> WireNoneSignalControlKind {
        self.kind
    }
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireNoneSignalControlKind {
    None,
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireSignalGroup {
    pub(crate) id: String,
}

impl WireSignalGroup {
    pub fn id(&self) -> &str {
        &self.id
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireSignalController {
    pub(crate) id: String,
    pub(crate) kind: WireSignalControllerKind,
    pub(crate) offset_ms: u64,
    pub(crate) group_ids: Vec<String>,
    pub(crate) phases: Vec<WireSignalPhase>,
}

impl WireSignalController {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn kind(&self) -> WireSignalControllerKind {
        self.kind
    }

    pub fn offset_ms(&self) -> u64 {
        self.offset_ms
    }

    pub fn group_ids(&self) -> &[String] {
        &self.group_ids
    }

    pub fn phases(&self) -> &[WireSignalPhase] {
        &self.phases
    }
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireSignalControllerKind {
    FixedTime,
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireSignalPhase {
    pub(crate) id: String,
    pub(crate) duration_ms: u64,
    pub(crate) states: Vec<WireSignalGroupState>,
}

impl WireSignalPhase {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    pub fn states(&self) -> &[WireSignalGroupState] {
        &self.states
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WireSignalGroupState {
    pub(crate) group_id: String,
    pub(crate) aspect: WireSignalAspect,
}

impl WireSignalGroupState {
    pub fn group_id(&self) -> &str {
        &self.group_id
    }

    pub fn aspect(&self) -> WireSignalAspect {
        self.aspect
    }
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireSignalAspect {
    Red,
    Yellow,
    Green,
}
