//! 当前 v0.4 JSON 格式的私有 wire DTO。

use serde::Deserialize;

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
    pub(crate) routes: Vec<WireRoute>,
    pub(crate) vehicle_profiles: Vec<WireVehicleProfile>,
    pub(crate) signals: WireSignals,
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
    #[serde(rename = "connections")]
    pub(crate) connections: Vec<WireLaneConnection>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireLaneConnection {
    pub(crate) to_edge_id: String,
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
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireSignals {
    pub(crate) stop_lines: Vec<WireStopLine>,
    pub(crate) movement_gates: Vec<WireMovementGate>,
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
pub(crate) struct WireMovementGate {
    pub(crate) from_edge_id: String,
    pub(crate) to_edge_id: String,
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
