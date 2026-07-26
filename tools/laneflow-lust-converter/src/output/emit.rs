//! JSON emit, schema validation, and production loader round-trip.

use laneflow_core::CoreWorld;
use laneflow_data::{
    NamedArtifact, SPATIAL_PACKAGE_MEDIA_TYPE, TRAFFIC_PACKAGE_MEDIA_TYPE, from_scenario_json_slice,
};
use laneflow_spatial::{SpatialEdgeInput, SpatialRegistry};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    Error, Result,
    output::model::{ArtifactDescriptor, ScenarioManifest},
};

const TRAFFIC_SCHEMA: &str = include_str!("../../../../schemas/laneflow-data-v0.8.schema.json");
const SPATIAL_SCHEMA: &str = include_str!("../../../../schemas/laneflow-spatial-v0.1.schema.json");
const MANIFEST_SCHEMA: &str =
    include_str!("../../../../schemas/laneflow-scenario-manifest-v0.1.schema.json");

/// Validated Traffic / Spatial / ScenarioManifest byte packages.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopologyArtifacts {
    pub traffic: Vec<u8>,
    pub spatial: Vec<u8>,
    pub manifest: Vec<u8>,
    pub traffic_artifact_ref: String,
    pub spatial_artifact_ref: String,
    pub edge_count: usize,
}

/// Serialize `value` as pretty JSON with a trailing newline.
pub fn json_bytes<T: Serialize>(document: &'static str, value: &T) -> Result<Vec<u8>> {
    let mut bytes =
        serde_json::to_vec_pretty(value).map_err(|source| Error::Json { document, source })?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Build a ScenarioManifest descriptor for raw artifact bytes.
pub fn descriptor(artifact_ref: String, media_type: &'static str, bytes: &[u8]) -> ArtifactDescriptor {
    ArtifactDescriptor {
        artifact_ref,
        media_type,
        digest: format!("sha256:{}", hex_digest(Sha256::digest(bytes).as_slice())),
        size: u64::try_from(bytes.len()).expect("artifact size fits in u64"),
    }
}

/// Validate JSON Schema + production loader + SpatialRegistry + CoreWorld.
pub fn validate_runtime(
    fixed_delta_ms: u64,
    traffic_artifact_ref: &str,
    spatial_artifact_ref: &str,
    traffic: &[u8],
    spatial: &[u8],
    manifest: &[u8],
) -> Result<()> {
    validate_schema("TrafficPackage", TRAFFIC_SCHEMA, traffic)?;
    validate_schema("SpatialPackage", SPATIAL_SCHEMA, spatial)?;
    validate_schema("ScenarioManifest", MANIFEST_SCHEMA, manifest)?;

    let loaded = from_scenario_json_slice(
        manifest,
        &[
            NamedArtifact::new(traffic_artifact_ref, traffic),
            NamedArtifact::new(spatial_artifact_ref, spatial),
        ],
    )
    .map_err(|error| Error::Validation {
        stage: "production scenario loader",
        message: error.to_string(),
    })?;
    let (traffic_pkg, spatial_pkg) = loaded.into_parts();
    let traffic_data = traffic_pkg.into_initial_traffic_data();
    SpatialRegistry::try_new(
        traffic_data.lane_graph(),
        spatial_pkg.frame_id().clone(),
        spatial_pkg
            .edges()
            .iter()
            .map(|edge| SpatialEdgeInput::new(edge.edge(), edge.points())),
    )
    .map_err(|error| Error::Validation {
        stage: "SpatialRegistry",
        message: error.to_string(),
    })?;
    CoreWorld::with_traffic_data(fixed_delta_ms, traffic_data, Vec::new()).map_err(|error| {
        Error::Validation {
            stage: "CoreWorld",
            message: error.to_string(),
        }
    })?;
    Ok(())
}

/// Pair Traffic/Spatial into a ScenarioManifest and validate the trio.
pub fn finish_topology_artifacts(
    fixed_delta_ms: u64,
    traffic_artifact_ref: String,
    spatial_artifact_ref: String,
    traffic: Vec<u8>,
    spatial: Vec<u8>,
    edge_count: usize,
) -> Result<TopologyArtifacts> {
    let manifest = ScenarioManifest {
        format_version: "0.1",
        traffic: descriptor(
            traffic_artifact_ref.clone(),
            TRAFFIC_PACKAGE_MEDIA_TYPE,
            &traffic,
        ),
        spatial: descriptor(
            spatial_artifact_ref.clone(),
            SPATIAL_PACKAGE_MEDIA_TYPE,
            &spatial,
        ),
    };
    let manifest_bytes = json_bytes("ScenarioManifest", &manifest)?;
    validate_runtime(
        fixed_delta_ms,
        &traffic_artifact_ref,
        &spatial_artifact_ref,
        &traffic,
        &spatial,
        &manifest_bytes,
    )?;
    Ok(TopologyArtifacts {
        traffic,
        spatial,
        manifest: manifest_bytes,
        traffic_artifact_ref,
        spatial_artifact_ref,
        edge_count,
    })
}

fn validate_schema(document: &'static str, schema_source: &str, input: &[u8]) -> Result<()> {
    let schema = serde_json::from_str(schema_source).map_err(|source| Error::Json {
        document: "repository schema",
        source,
    })?;
    let instance =
        serde_json::from_slice(input).map_err(|source| Error::Json { document, source })?;
    jsonschema::draft202012::validate(&schema, &instance).map_err(|error| Error::Schema {
        document,
        message: error.to_string(),
    })
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}
