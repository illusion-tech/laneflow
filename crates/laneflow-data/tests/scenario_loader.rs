use laneflow_core::EdgeProgress;
use laneflow_data::{
    ArtifactRole, CURRENT_SCENARIO_MANIFEST_FORMAT_VERSION, CURRENT_SPATIAL_FORMAT_VERSION,
    NamedArtifact, SPATIAL_PACKAGE_MEDIA_TYPE, ScenarioDocument, ScenarioError,
    TRAFFIC_PACKAGE_MEDIA_TYPE, from_scenario_json_slice, from_scenario_json_str,
};
use laneflow_spatial::{SpatialEdgeInput, SpatialRegistry};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const TRAFFIC_REF: &str = "v0.5-empty-signals-and-parking.laneflow.json";
const SPATIAL_REF: &str = "v0.1-campus.spatial.json";
const TRAFFIC: &[u8] =
    include_bytes!("../../../examples/data/v0.5-empty-signals-and-parking.laneflow.json");
const SPATIAL: &[u8] = include_bytes!("../../../examples/data/v0.1-campus.spatial.json");
const MANIFEST: &str = include_str!("../../../examples/data/v0.1-campus.scenario.json");

#[test]
fn scenario_error_is_a_thread_safe_standard_error() {
    fn assert_error<T: std::error::Error + Send + Sync>() {}

    assert_error::<ScenarioError>();
}

#[test]
fn canonical_scenario_loads_atomically_in_lane_graph_order() {
    assert_eq!(CURRENT_SCENARIO_MANIFEST_FORMAT_VERSION, "0.1");
    assert_eq!(CURRENT_SPATIAL_FORMAT_VERSION, "0.1");
    let loaded = load(MANIFEST, TRAFFIC, SPATIAL).expect("canonical scenario must load");
    assert_eq!(loaded.spatial().frame_id().as_str(), "campus-local");

    let graph = loaded.traffic().initial_traffic_data().lane_graph();
    let external_ids = loaded
        .spatial()
        .edges()
        .iter()
        .map(|edge| graph.edge_external_id(edge.edge()).expect("known edge"))
        .collect::<Vec<_>>();
    assert_eq!(external_ids, ["entry", "exit", "loop", "isolated"]);

    let entry = &loaded.spatial().edges()[0];
    assert_eq!(entry.points().len(), 2);
    assert_eq!(entry.points()[0].x(), 0.0);
    assert_eq!(entry.points()[1].x(), 12.0);
    assert_eq!(entry.points()[1].y(), 0.0);
    assert_eq!(entry.points()[1].z(), 0.0);
}

#[test]
fn loaded_spatial_package_maps_directly_into_spatial_registry_inputs() {
    let loaded = load(MANIFEST, TRAFFIC, SPATIAL).expect("canonical scenario loads");
    let graph = loaded.traffic().initial_traffic_data().lane_graph();

    let registry = SpatialRegistry::try_new(
        graph,
        loaded.spatial().frame_id().clone(),
        loaded
            .spatial()
            .edges()
            .iter()
            .map(|edge| SpatialEdgeInput::new(edge.edge(), edge.points())),
    )
    .expect("loaded #134 output binds without a conversion copy");

    assert_eq!(registry.len(), graph.edges().len());
    let loop_handle = graph.edge_handle("loop").expect("loop edge");
    let pose = registry
        .sample(
            loop_handle,
            EdgeProgress::try_new(2.5).expect("valid progress"),
        )
        .expect("loop turnaround sample");
    assert_eq!([pose.position().x(), pose.position().z()], [2.5, 10.0]);
    assert_eq!([pose.tangent().x(), pose.tangent().z()], [-1.0, 0.0]);
}

#[test]
fn canonical_manifest_pins_the_exact_raw_fixture_bytes() {
    let value: Value = serde_json::from_str(MANIFEST).expect("manifest JSON");
    assert_eq!(value["traffic"]["size"], TRAFFIC.len());
    assert_eq!(value["spatial"]["size"], SPATIAL.len());
    assert_eq!(value["traffic"]["digest"], digest(TRAFFIC));
    assert_eq!(value["spatial"]["digest"], digest(SPATIAL));
}

#[test]
fn manifest_version_is_rejected_before_current_shape() {
    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["formatVersion"] = json!("0.2");
    manifest["future"] = json!(true);
    std::assert_matches!(
        load_value(&manifest, TRAFFIC, SPATIAL).expect_err("unsupported manifest"),
        ScenarioError::UnsupportedFormatVersion {
            document: ScenarioDocument::Manifest,
            expected: "0.1",
            actual,
        } if actual == "0.2"
    );
}

#[test]
fn spatial_version_is_rejected_before_current_spatial_shape() {
    let mut spatial = spatial_value();
    spatial["formatVersion"] = json!("0.2");
    spatial["future"] = json!(true);
    let spatial = serde_json::to_vec(&spatial).expect("spatial JSON");
    let manifest = manifest_value(TRAFFIC, &spatial);
    std::assert_matches!(
        load_value(&manifest, TRAFFIC, &spatial).expect_err("unsupported spatial"),
        ScenarioError::UnsupportedFormatVersion {
            document: ScenarioDocument::Spatial,
            expected: "0.1",
            actual,
        } if actual == "0.2"
    );
}

#[test]
fn strict_manifest_and_spatial_shapes_preserve_paths() {
    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["traffic"]["future"] = json!(true);
    std::assert_matches!(
        load_value(&manifest, TRAFFIC, SPATIAL).expect_err("unknown manifest field"),
        ScenarioError::JsonShape { document: ScenarioDocument::Manifest, path, .. }
            if path.contains("traffic")
    );

    let mut spatial = spatial_value();
    spatial["edges"][0]["centerline"]["points"][0] = json!({ "x": 0, "y": 0, "z": 0 });
    let spatial = serde_json::to_vec(&spatial).expect("spatial JSON");
    let manifest = manifest_value(TRAFFIC, &spatial);
    std::assert_matches!(
        load_value(&manifest, TRAFFIC, &spatial).expect_err("object point"),
        ScenarioError::JsonShape { document: ScenarioDocument::Spatial, path, .. }
            if path.contains("points[0]")
    );
}

#[test]
fn descriptors_and_provided_artifacts_are_exact_and_unambiguous() {
    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["traffic"]["mediaType"] = json!("application/json");
    std::assert_matches!(
        load_value(&manifest, TRAFFIC, SPATIAL).expect_err("wrong media type"),
        ScenarioError::InvalidMediaType {
            path: "traffic.mediaType",
            ..
        }
    );

    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["spatial"]["digest"] = json!("sha256:ABCDEF");
    std::assert_matches!(
        load_value(&manifest, TRAFFIC, SPATIAL).expect_err("wrong digest syntax"),
        ScenarioError::InvalidDigest {
            path: "spatial.digest",
            ..
        }
    );

    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["spatial"]["size"] = json!(9_007_199_254_740_992_u64);
    std::assert_matches!(
        load_value(&manifest, TRAFFIC, SPATIAL).expect_err("non-portable size"),
        ScenarioError::ArtifactSizeOutOfRange {
            path: "spatial.size",
            ..
        }
    );

    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["spatial"]["artifactRef"] = json!(TRAFFIC_REF);
    std::assert_matches!(
        load_value(&manifest, TRAFFIC, SPATIAL).expect_err("conflicting refs"),
        ScenarioError::ConflictingManifestArtifactRef { .. }
    );

    let source = serde_json::to_vec(&manifest_value(TRAFFIC, SPATIAL)).expect("manifest JSON");
    let duplicate = [
        NamedArtifact::new(TRAFFIC_REF, TRAFFIC),
        NamedArtifact::new(SPATIAL_REF, SPATIAL),
        NamedArtifact::new(SPATIAL_REF, SPATIAL),
    ];
    std::assert_matches!(
        from_scenario_json_slice(&source, &duplicate).expect_err("duplicate provided ref"),
        ScenarioError::DuplicateProvidedArtifactRef { artifact_ref, .. }
            if artifact_ref == SPATIAL_REF
    );

    let missing = [NamedArtifact::new(TRAFFIC_REF, TRAFFIC)];
    std::assert_matches!(
        from_scenario_json_slice(&source, &missing).expect_err("missing spatial artifact"),
        ScenarioError::MissingArtifact {
            role: ArtifactRole::Spatial,
            ..
        }
    );
}

#[test]
fn raw_size_is_checked_before_raw_digest() {
    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["traffic"]["size"] = json!(TRAFFIC.len() + 1);
    manifest["traffic"]["digest"] = json!(format!("sha256:{}", "0".repeat(64)));
    std::assert_matches!(
        load_value(&manifest, TRAFFIC, SPATIAL).expect_err("size must win"),
        ScenarioError::ArtifactSizeMismatch {
            role: ArtifactRole::Traffic,
            ..
        }
    );

    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["traffic"]["digest"] = json!(format!("sha256:{}", "0".repeat(64)));
    std::assert_matches!(
        load_value(&manifest, TRAFFIC, SPATIAL).expect_err("digest mismatch"),
        ScenarioError::ArtifactDigestMismatch {
            role: ArtifactRole::Traffic,
            ..
        }
    );
}

#[test]
fn spatial_coordinates_are_checked_in_f64_before_f32_normalization() {
    for invalid in [-16_384.000_1, 16_384.000_1] {
        let mut spatial = spatial_value();
        spatial["edges"][0]["centerline"]["points"][0][0] = json!(invalid);
        std::assert_matches!(
            load_spatial_value(&spatial).expect_err("out-of-range coordinate"),
            ScenarioError::CoordinateOutOfRange { value, .. } if value == invalid
        );
    }

    let mut spatial = spatial_value();
    spatial["edges"][0]["centerline"]["points"][0][0] = json!(16_384.0);
    load_spatial_value(&spatial).expect("inclusive boundary must load");

    let mut spatial = spatial_value();
    spatial["edges"][0]["centerline"]["points"] = json!([[0.0, 0.0, 0.0]]);
    std::assert_matches!(
        load_spatial_value(&spatial).expect_err("one point is insufficient"),
        ScenarioError::InsufficientCenterlinePoints {
            actual: 1,
            min: 2,
            ..
        }
    );
}

#[test]
fn spatial_edge_coverage_is_total_unique_and_bound_to_traffic_ids() {
    let mut unknown = spatial_value();
    unknown["edges"][0]["trafficEdgeId"] = json!("unknown");
    std::assert_matches!(
        load_spatial_value(&unknown).expect_err("unknown edge"),
        ScenarioError::UnknownTrafficEdge { traffic_edge_id, .. } if traffic_edge_id == "unknown"
    );

    let mut duplicate = spatial_value();
    duplicate["edges"][1]["trafficEdgeId"] = duplicate["edges"][0]["trafficEdgeId"].clone();
    std::assert_matches!(
        load_spatial_value(&duplicate).expect_err("duplicate edge"),
        ScenarioError::DuplicateTrafficEdge { .. }
    );

    let mut missing = spatial_value();
    missing["edges"].as_array_mut().expect("edges array").pop();
    std::assert_matches!(
        load_spatial_value(&missing).expect_err("missing edge"),
        ScenarioError::MissingTrafficEdge { traffic_edge_id, .. } if traffic_edge_id == "exit"
    );
}

#[test]
fn traffic_failure_does_not_expose_a_partial_scenario() {
    let invalid_traffic = br#"{"formatVersion":"0.5"}"#;
    let manifest = manifest_value(invalid_traffic, SPATIAL);
    std::assert_matches!(
        load_value(&manifest, invalid_traffic, SPATIAL).expect_err("traffic must fail"),
        ScenarioError::TrafficPackage { artifact_ref, .. } if artifact_ref == TRAFFIC_REF
    );
}

fn load(
    manifest: &str,
    traffic: &[u8],
    spatial: &[u8],
) -> Result<laneflow_data::LoadedScenario, ScenarioError> {
    let artifacts = [
        NamedArtifact::new(TRAFFIC_REF, traffic),
        NamedArtifact::new(SPATIAL_REF, spatial),
    ];
    from_scenario_json_str(manifest, &artifacts)
}

fn load_value(
    manifest: &Value,
    traffic: &[u8],
    spatial: &[u8],
) -> Result<laneflow_data::LoadedScenario, ScenarioError> {
    let source = serde_json::to_string(manifest).expect("manifest JSON");
    load(&source, traffic, spatial)
}

fn load_spatial_value(spatial: &Value) -> Result<laneflow_data::LoadedScenario, ScenarioError> {
    let spatial = serde_json::to_vec(spatial).expect("spatial JSON");
    let manifest = manifest_value(TRAFFIC, &spatial);
    load_value(&manifest, TRAFFIC, &spatial)
}

fn manifest_value(traffic: &[u8], spatial: &[u8]) -> Value {
    json!({
        "formatVersion": CURRENT_SCENARIO_MANIFEST_FORMAT_VERSION,
        "traffic": {
            "artifactRef": TRAFFIC_REF,
            "mediaType": TRAFFIC_PACKAGE_MEDIA_TYPE,
            "digest": digest(traffic),
            "size": traffic.len(),
        },
        "spatial": {
            "artifactRef": SPATIAL_REF,
            "mediaType": SPATIAL_PACKAGE_MEDIA_TYPE,
            "digest": digest(spatial),
            "size": spatial.len(),
        }
    })
}

fn spatial_value() -> Value {
    serde_json::from_slice(SPATIAL).expect("spatial fixture JSON")
}

fn digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::from("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(encoded, "{byte:02x}").expect("write to String");
    }
    encoded
}
