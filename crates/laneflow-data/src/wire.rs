//! 当前 v0.9 JSON 格式的私有 wire DTO。

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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireVersionHeader {
    pub(crate) format_version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WirePackage {
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
    pub(crate) signals: WireSignals,
    pub(crate) parking: WireParking,
    #[serde(default, rename = "extensions")]
    pub(crate) _extensions: serde_json::Map<String, serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireUnits {
    pub(crate) distance: String,
    pub(crate) time: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireLaneGraph {
    pub(crate) edges: Vec<WireLaneEdge>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireLaneEdge {
    pub(crate) id: String,
    pub(crate) length: f64,
    #[serde(rename = "speedLimit")]
    pub(crate) speed_limit: f64,
    #[serde(rename = "connections")]
    pub(crate) connections: Vec<WireLaneConnection>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireLaneConnection {
    pub(crate) to_edge_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireJunction {
    pub(crate) id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireMovement {
    pub(crate) id: String,
    pub(crate) junction_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireManeuverPath {
    pub(crate) id: String,
    pub(crate) movement_id: String,
    pub(crate) entry_edge_id: String,
    pub(crate) internal_edge_ids: Vec<String>,
    pub(crate) exit_edge_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireRoute {
    pub(crate) id: String,
    pub(crate) edge_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireVehicleProfile {
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireParticipantClass {
    pub(crate) id: String,
    #[serde(default, deserialize_with = "non_null_option")]
    pub(crate) extends_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireFacilityBand {
    pub(crate) id: String,
    pub(crate) kind_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireRoadSection {
    pub(crate) id: String,
    pub(crate) kind_id: String,
    pub(crate) lanes: Vec<WireSectionLane>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireSectionLane {
    pub(crate) edge_ids: Vec<String>,
    #[serde(default, deserialize_with = "non_null_option")]
    pub(crate) lane_group_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireLaneGroup {
    pub(crate) id: String,
    pub(crate) road_section_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireRoadCorridor {
    pub(crate) id: String,
    pub(crate) reference_section_id: String,
    pub(crate) elements: Vec<WireCorridorElement>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum WireCorridorElement {
    Section(WireCorridorSectionElement),
    Band(WireCorridorBandElement),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireCorridorSectionElement {
    pub(crate) section_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireCorridorBandElement {
    pub(crate) band_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireAccessRule {
    pub(crate) id: String,
    pub(crate) target: WireAccessTarget,
    pub(crate) effect: WireAccessEffect,
    pub(crate) participant_class_ids: Vec<String>,
    #[serde(default, deserialize_with = "non_null_option")]
    pub(crate) time_windows: Option<Vec<WireTimeWindow>>,
    #[serde(default, deserialize_with = "non_null_option")]
    pub(crate) regulation: Option<WireRegulation>,
    #[serde(default, deserialize_with = "non_null_option")]
    // 与 timeWindows 分钟字段同理：JSON number type 检查留在 wire（非数值仍
    // JsonShape 拒绝），整数性与 i32 语义范围保留原始数值字面量，由 Core
    // phase 9.5 在 capability guard 之后校验。
    pub(crate) priority: Option<serde_json::Number>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireAccessTarget {
    pub(crate) kind: WireAccessTargetKind,
    pub(crate) id: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WireAccessTargetKind {
    LaneEdge,
    LaneGroup,
    RoadSection,
    ManeuverPath,
    FacilityBand,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WireAccessEffect {
    Allow,
    Deny,
}

// timeWindows 字段只参与 closed-shape 校验：v1 capability guard 对声明
// timeWindows 的规则一律拒绝载入，窗口内容不进入 Core，故字段保持未读。
// 分钟字段保留原始数值（serde_json::Number）而非 u32：负数或超 u32 范围的
// 值在语义上同样越界，但范围属于 phase 9 shape 检查，必须排在 capability
// guard 之后（cross-section-access.md §10）——解码期范围检查会让
// DataError::JsonShape 抢在 AccessCapabilityUnavailable 之前，使诊断依赖
// 不被支持的窗口内容。任何 JSON 数值都能解码为 Number 并先抵达 guard。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(
    dead_code,
    reason = "timeWindows 内容只作 closed-shape 校验，capability guard 不读取"
)]
pub(crate) struct WireTimeWindow {
    pub(crate) days: Vec<String>,
    pub(crate) start_minute_of_day: serde_json::Number,
    pub(crate) end_minute_of_day: serde_json::Number,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireRegulation {
    pub(crate) jurisdiction: String,
    pub(crate) version: String,
    #[serde(default, deserialize_with = "non_null_option")]
    pub(crate) source: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireParking {
    pub(crate) areas: Vec<WireParkingArea>,
    pub(crate) spaces: Vec<WireParkingSpace>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireParkingArea {
    pub(crate) id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireParkingSpace {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) area_id: OmittedAreaId,
    pub(crate) entry: WireParkingAnchor,
    pub(crate) exit: WireParkingAnchor,
    pub(crate) geometry: WireParkingGeometry,
}

#[derive(Default)]
pub(crate) struct OmittedAreaId(Option<String>);

impl OmittedAreaId {
    pub(crate) fn as_deref(&self) -> Option<&str> {
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireParkingAnchor {
    pub(crate) edge_id: String,
    pub(crate) progress: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireParkingGeometry {
    pub(crate) lateral_offset: f64,
    pub(crate) heading_offset_radians: f64,
    pub(crate) length: f64,
    pub(crate) width: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireSignals {
    pub(crate) stop_lines: Vec<WireStopLine>,
    pub(crate) maneuver_gates: Vec<WireManeuverGate>,
    pub(crate) groups: Vec<WireSignalGroup>,
    pub(crate) controllers: Vec<WireSignalController>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireStopLine {
    pub(crate) id: String,
    pub(crate) edge_id: String,
    pub(crate) location: WireStopLineLocation,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WireStopLineLocation {
    EdgeEnd,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireManeuverGate {
    pub(crate) id: String,
    pub(crate) maneuver_path_id: String,
    pub(crate) transition_index: u32,
    pub(crate) stop_line_id: String,
    pub(crate) signal_control: WireSignalControl,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum WireSignalControl {
    Group(WireGroupSignalControl),
    None(WireNoneSignalControl),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireGroupSignalControl {
    pub(crate) kind: WireGroupSignalControlKind,
    pub(crate) group_id: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WireGroupSignalControlKind {
    Group,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireNoneSignalControl {
    pub(crate) kind: WireNoneSignalControlKind,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WireNoneSignalControlKind {
    None,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireSignalGroup {
    pub(crate) id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireSignalController {
    pub(crate) id: String,
    pub(crate) kind: WireSignalControllerKind,
    pub(crate) offset_ms: u64,
    pub(crate) group_ids: Vec<String>,
    pub(crate) phases: Vec<WireSignalPhase>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WireSignalControllerKind {
    FixedTime,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireSignalPhase {
    pub(crate) id: String,
    pub(crate) duration_ms: u64,
    pub(crate) states: Vec<WireSignalGroupState>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireSignalGroupState {
    pub(crate) group_id: String,
    pub(crate) aspect: WireSignalAspect,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WireSignalAspect {
    Red,
    Yellow,
    Green,
}
