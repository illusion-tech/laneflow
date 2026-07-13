//! 当前 v0.3 JSON 格式的私有 wire DTO。

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
    #[serde(default, rename = "extensions")]
    pub(crate) _extensions: Option<serde_json::Map<String, serde_json::Value>>,
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
    pub(crate) connections: Vec<WireLaneConnection>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireLaneConnection {
    pub(crate) to: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireRoute {
    pub(crate) id: String,
    pub(crate) edges: Vec<String>,
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
