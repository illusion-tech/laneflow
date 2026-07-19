//! SpatialPackage 0.1 与 ScenarioManifest 0.1 的私有 wire DTO。

use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireScenarioManifest {
    pub(crate) format_version: String,
    pub(crate) traffic: WireArtifactDescriptor,
    pub(crate) spatial: WireArtifactDescriptor,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireArtifactDescriptor {
    pub(crate) artifact_ref: String,
    pub(crate) media_type: String,
    pub(crate) digest: String,
    pub(crate) size: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireSpatialPackage {
    pub(crate) format_version: String,
    pub(crate) frame_id: String,
    pub(crate) edges: Vec<WireSpatialEdge>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireSpatialEdge {
    pub(crate) traffic_edge_id: String,
    pub(crate) centerline: WireCenterline,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireCenterline {
    pub(crate) points: Vec<[f64; 3]>,
}
