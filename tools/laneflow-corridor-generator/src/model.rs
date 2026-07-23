use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TrafficPackage {
    pub format_version: &'static str,
    pub units: Units,
    pub lane_graph: LaneGraph,
    pub routes: Vec<Route>,
    pub vehicle_profiles: Vec<VehicleProfile>,
    pub signals: Signals,
    pub parking: Parking,
}

#[derive(Debug, Serialize)]
pub(crate) struct Units {
    pub distance: &'static str,
    pub time: &'static str,
}

#[derive(Debug, Serialize)]
pub(crate) struct LaneGraph {
    pub edges: Vec<LaneEdge>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LaneEdge {
    pub id: String,
    pub length: f64,
    pub speed_limit: f64,
    pub connections: Vec<LaneConnection>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LaneConnection {
    pub to_edge_id: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Route {
    pub id: String,
    pub edge_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VehicleProfile {
    pub id: &'static str,
    pub length: f64,
    pub model: &'static str,
    pub desired_speed: f64,
    pub min_gap: f64,
    pub time_headway: f64,
    pub max_acceleration: f64,
    pub comfortable_deceleration: f64,
    pub emergency_deceleration: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Signals {
    pub stop_lines: Vec<StopLine>,
    pub movement_gates: Vec<MovementGate>,
    pub groups: Vec<SignalGroup>,
    pub controllers: Vec<SignalController>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StopLine {
    pub id: String,
    pub edge_id: String,
    pub location: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MovementGate {
    pub from_edge_id: String,
    pub to_edge_id: String,
    pub stop_line_id: String,
    pub signal_control: SignalControl,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignalControl {
    pub kind: &'static str,
    pub group_id: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SignalGroup {
    pub id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignalController {
    pub id: String,
    pub kind: &'static str,
    pub offset_ms: u64,
    pub group_ids: Vec<String>,
    pub phases: Vec<SignalPhase>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignalPhase {
    pub id: String,
    pub duration_ms: u64,
    pub states: Vec<SignalGroupState>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignalGroupState {
    pub group_id: String,
    pub aspect: &'static str,
}

#[derive(Debug, Serialize)]
pub(crate) struct Parking {
    pub areas: Vec<Never>,
    pub spaces: Vec<Never>,
}

#[derive(Debug, Serialize)]
pub(crate) enum Never {}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpatialPackage {
    pub format_version: &'static str,
    pub frame_id: String,
    pub edges: Vec<SpatialEdge>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpatialEdge {
    pub traffic_edge_id: String,
    pub centerline: Centerline,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Centerline {
    pub points: Vec<[f64; 3]>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScenarioManifest {
    pub format_version: &'static str,
    pub traffic: ArtifactDescriptor,
    pub spatial: ArtifactDescriptor,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactDescriptor {
    pub artifact_ref: String,
    pub media_type: &'static str,
    pub digest: String,
    pub size: u64,
}
