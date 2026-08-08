//! SpatialPackage 0.1 与 ScenarioManifest 0.1 的 wire DTO。
//!
//! record 类型字段私有，跨包消费只经逐项借用 accessor；serde 行为与
//! `laneflow-data` 迁移前逐字节一致。

use serde::Deserialize;

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireScenarioManifest {
    format_version: String,
    traffic: WireArtifactDescriptor,
    spatial: WireArtifactDescriptor,
}

impl WireScenarioManifest {
    pub fn traffic(&self) -> &WireArtifactDescriptor {
        &self.traffic
    }

    pub fn spatial(&self) -> &WireArtifactDescriptor {
        &self.spatial
    }

    pub(crate) fn format_version(&self) -> &str {
        &self.format_version
    }
}

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireArtifactDescriptor {
    artifact_ref: String,
    media_type: String,
    digest: String,
    size: u64,
}

impl WireArtifactDescriptor {
    pub fn artifact_ref(&self) -> &str {
        &self.artifact_ref
    }

    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn size(&self) -> u64 {
        self.size
    }
}

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireSpatialPackage {
    format_version: String,
    frame_id: String,
    edges: Vec<WireSpatialEdge>,
}

impl WireSpatialPackage {
    pub fn frame_id(&self) -> &str {
        &self.frame_id
    }

    pub fn edges(&self) -> &[WireSpatialEdge] {
        &self.edges
    }

    pub(crate) fn format_version(&self) -> &str {
        &self.format_version
    }
}

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireSpatialEdge {
    traffic_edge_id: String,
    centerline: WireCenterline,
}

impl WireSpatialEdge {
    pub fn traffic_edge_id(&self) -> &str {
        &self.traffic_edge_id
    }

    pub fn centerline(&self) -> &WireCenterline {
        &self.centerline
    }
}

#[doc(hidden)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireCenterline {
    points: Vec<[f64; 3]>,
}

impl WireCenterline {
    pub fn points(&self) -> &[[f64; 3]] {
        &self.points
    }
}
