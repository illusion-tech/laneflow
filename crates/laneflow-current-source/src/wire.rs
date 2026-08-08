//! 当前 v0.10 JSON 格式的 wire DTO。
//!
//! record 类型字段私有，跨包消费只经逐项借用 accessor；serde 行为
//! （`deny_unknown_fields`、explicit-null 拒绝、priority 原始词法、untagged 枚举）
//! 与 `laneflow-data` 迁移前逐字节一致。

use serde::Deserialize;

/// 可选字段拒绝显式 `null`：缺省字段返回 `None`（配合 `#[serde(default)]`），
/// 显式 `null` 返回反序列化错误。loader 路径不执行 JSON Schema，而 schema 不接受
/// `null`；对 `timeWindows` 这类 null 会改变语义的字段（被当作未声明而绕过
/// capability guard），缺省与显式 null 必须可区分。
fn non_null_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    match Option::<T>::deserialize(deserializer)? {
        Some(value) => Ok(Some(value)),
        None => Err(serde::de::Error::custom(
            "可选字段不接受显式 null；请省略该字段",
        )),
    }
}

// priority 保留原始数值字面量：serde_json::Number 会按 f64 归一化
// （1.00000000000000001 变 1.0、1e400 溢出为 JsonShape），字面量必须不经
// 浮点转换进入 Core phase 9.5 精确校验（capability guard 之后）。wire 层只
// 把关 JSON number type/语法，整数性与 i32 范围语义归 Core。
fn access_priority_lexeme<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = match Option::<Box<serde_json::value::RawValue>>::deserialize(deserializer)? {
        Some(raw) => raw,
        None => {
            return Err(serde::de::Error::custom(
                "可选字段不接受显式 null；请省略该字段",
            ));
        }
    };
    let lexeme = raw.get().trim();
    if is_json_number_lexeme(lexeme) {
        Ok(Some(lexeme.to_owned()))
    } else {
        Err(serde::de::Error::custom(format!(
            "priority 必须是 JSON number，实际为 `{lexeme}`"
        )))
    }
}

/// JSON number 语法：`-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`。
fn is_json_number_lexeme(lexeme: &str) -> bool {
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    let lexeme = lexeme.strip_prefix('-').unwrap_or(lexeme);
    let (mantissa, exponent) = match lexeme.find(['e', 'E']) {
        Some(index) => (&lexeme[..index], Some(&lexeme[index + 1..])),
        None => (lexeme, None),
    };
    if let Some(exponent) = exponent {
        let exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        if !digits(exponent) {
            return false;
        }
    }
    let (integer, fraction) = match mantissa.find('.') {
        Some(index) => (&mantissa[..index], Some(&mantissa[index + 1..])),
        None => (mantissa, None),
    };
    let integer_ok = integer == "0"
        || (integer.starts_with(|c: char| c.is_ascii_digit() && c != '0') && digits(integer));
    integer_ok && fraction.is_none_or(digits)
}

/// 版本闸口专用的头部 DTO；恰好一个合法字符串 `formatVersion` occurrence 才
/// 通过本 DTO 参与版本裁决（缺失、显式 `null`、非字符串或重复 occurrence 都
/// 在本步以 shape 错误立即失败）。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireVersionHeader {
    pub(crate) format_version: String,
}

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WirePackage {
    format_version: String,
    units: WireUnits,
    lane_graph: WireLaneGraph,
    junctions: Vec<WireJunction>,
    movements: Vec<WireMovement>,
    maneuver_paths: Vec<WireManeuverPath>,
    routes: Vec<WireRoute>,
    vehicle_profiles: Vec<WireVehicleProfile>,
    participant_classes: Vec<WireParticipantClass>,
    facility_bands: Vec<WireFacilityBand>,
    road_sections: Vec<WireRoadSection>,
    lane_groups: Vec<WireLaneGroup>,
    road_corridors: Vec<WireRoadCorridor>,
    access_rules: Vec<WireAccessRule>,
    waiting_zones: Vec<WireWaitingZone>,
    signals: WireSignals,
    parking: WireParking,
    #[serde(default, rename = "extensions")]
    _extensions: serde_json::Map<String, serde_json::Value>,
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
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireUnits {
    distance: String,
    time: String,
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireLaneGraph {
    edges: Vec<WireLaneEdge>,
}

impl WireLaneGraph {
    pub fn edges(&self) -> &[WireLaneEdge] {
        &self.edges
    }
}

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireLaneEdge {
    id: String,
    length: f64,
    #[serde(rename = "speedLimit")]
    speed_limit: f64,
    #[serde(rename = "connections")]
    connections: Vec<WireLaneConnection>,
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireLaneConnection {
    to_edge_id: String,
}

impl WireLaneConnection {
    pub fn to_edge_id(&self) -> &str {
        &self.to_edge_id
    }
}

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireJunction {
    id: String,
}

impl WireJunction {
    pub fn id(&self) -> &str {
        &self.id
    }
}

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireMovement {
    id: String,
    junction_id: String,
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireManeuverPath {
    id: String,
    movement_id: String,
    entry_edge_id: String,
    internal_edge_ids: Vec<String>,
    exit_edge_id: String,
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireRoute {
    id: String,
    edge_ids: Vec<String>,
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireVehicleProfile {
    id: String,
    length: f64,
    model: String,
    desired_speed: f64,
    min_gap: f64,
    time_headway: f64,
    max_acceleration: f64,
    comfortable_deceleration: f64,
    emergency_deceleration: f64,
    participant_class_id: String,
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireParticipantClass {
    id: String,
    #[serde(default, deserialize_with = "non_null_option")]
    extends_id: Option<String>,
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireFacilityBand {
    id: String,
    kind_id: String,
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireRoadSection {
    id: String,
    kind_id: String,
    lanes: Vec<WireSectionLane>,
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireSectionLane {
    edge_ids: Vec<String>,
    #[serde(default, deserialize_with = "non_null_option")]
    lane_group_id: Option<String>,
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireLaneGroup {
    id: String,
    road_section_id: String,
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireRoadCorridor {
    id: String,
    reference_section_id: String,
    elements: Vec<WireCorridorElement>,
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
#[derive(Deserialize)]
#[serde(untagged)]
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireCorridorSectionElement {
    section_id: String,
}

impl WireCorridorSectionElement {
    pub fn section_id(&self) -> &str {
        &self.section_id
    }
}

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireCorridorBandElement {
    band_id: String,
}

impl WireCorridorBandElement {
    pub fn band_id(&self) -> &str {
        &self.band_id
    }
}

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireAccessRule {
    id: String,
    target: WireAccessTarget,
    effect: WireAccessEffect,
    participant_class_ids: Vec<String>,
    #[serde(default, deserialize_with = "non_null_option")]
    time_windows: Option<Vec<Box<serde_json::value::RawValue>>>,
    #[serde(default, deserialize_with = "non_null_option")]
    regulation: Option<WireRegulation>,
    #[serde(default, deserialize_with = "access_priority_lexeme")]
    priority: Option<String>,
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

    /// timeWindows 是 v1 不可用 capability：wire 层只以 RawValue 不透明捕获，
    /// 调用方只需要是否声明。
    pub fn has_time_windows(&self) -> bool {
        self.time_windows.is_some()
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireAccessTarget {
    kind: WireAccessTargetKind,
    id: String,
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireWaitingZone {
    id: String,
    maneuver_path_id: String,
    entry_gate_id: String,
    release_gate_id: String,
    max_occupancy: u32,
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
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WireAccessTargetKind {
    LaneEdge,
    LaneGroup,
    RoadSection,
    ManeuverPath,
    FacilityBand,
}

#[doc(hidden)]
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WireAccessEffect {
    Allow,
    Deny,
}

// timeWindows 是 v1 不可用 capability：wire 层只校验字段是数组（JSON type
// 检查）并以 RawValue 不透明捕获元素，窗口内容（字段结构、分钟数值）一律不
// 解码。capability guard 先于 shape（cross-section-access.md §10 phase 9：
// 能力整体拒绝后其内部细节校验无意义），因此极端数值（1e400 这类
// serde_json::Number 无法表示的字面量）、缺字段、未知字段、错误类型都必须先
// 抵达 guard 得到 AccessCapabilityUnavailable，而不是在 DTO 解码期以
// DataError::JsonShape 抢先。capability 未来可用时，窗口子树在 guard 之后的
// phase 再做完整 shape/range 校验（空数组、days 空集、分钟越界等 §10 已规约）。

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireRegulation {
    jurisdiction: String,
    version: String,
    #[serde(default, deserialize_with = "non_null_option")]
    source: Option<String>,
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireParking {
    areas: Vec<WireParkingArea>,
    spaces: Vec<WireParkingSpace>,
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
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireParkingArea {
    id: String,
}

impl WireParkingArea {
    pub fn id(&self) -> &str {
        &self.id
    }
}

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireParkingSpace {
    id: String,
    #[serde(default)]
    area_id: OmittedAreaId,
    entry: WireParkingAnchor,
    exit: WireParkingAnchor,
    geometry: WireParkingGeometry,
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

#[derive(Default)]
struct OmittedAreaId(Option<String>);

impl OmittedAreaId {
    fn as_deref(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

impl<'de> Deserialize<'de> for OmittedAreaId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer).map(|value| Self(Some(value)))
    }
}

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireParkingAnchor {
    edge_id: String,
    progress: f64,
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireParkingGeometry {
    lateral_offset: f64,
    heading_offset_radians: f64,
    length: f64,
    width: f64,
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireSignals {
    stop_lines: Vec<WireStopLine>,
    maneuver_gates: Vec<WireManeuverGate>,
    groups: Vec<WireSignalGroup>,
    controllers: Vec<WireSignalController>,
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireStopLine {
    id: String,
    edge_id: String,
    location: WireStopLineLocation,
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
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WireStopLineLocation {
    EdgeEnd,
}

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireManeuverGate {
    id: String,
    maneuver_path_id: String,
    transition_index: u32,
    stop_line_id: String,
    signal_control: WireSignalControl,
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
#[derive(Deserialize)]
#[serde(untagged)]
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireGroupSignalControl {
    kind: WireGroupSignalControlKind,
    group_id: String,
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
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WireGroupSignalControlKind {
    Group,
}

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireNoneSignalControl {
    kind: WireNoneSignalControlKind,
}

impl WireNoneSignalControl {
    pub fn kind(&self) -> WireNoneSignalControlKind {
        self.kind
    }
}

#[doc(hidden)]
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WireNoneSignalControlKind {
    None,
}

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireSignalGroup {
    id: String,
}

impl WireSignalGroup {
    pub fn id(&self) -> &str {
        &self.id
    }
}

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireSignalController {
    id: String,
    kind: WireSignalControllerKind,
    offset_ms: u64,
    group_ids: Vec<String>,
    phases: Vec<WireSignalPhase>,
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
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WireSignalControllerKind {
    FixedTime,
}

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireSignalPhase {
    id: String,
    duration_ms: u64,
    states: Vec<WireSignalGroupState>,
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireSignalGroupState {
    group_id: String,
    aspect: WireSignalAspect,
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
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WireSignalAspect {
    Red,
    Yellow,
    Green,
}
