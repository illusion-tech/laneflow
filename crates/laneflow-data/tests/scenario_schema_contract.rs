use jsonschema::draft202012;
use serde_json::{Value, json};

const SPATIAL_SCHEMA: &str = include_str!("../../../schemas/laneflow-spatial-v0.1.schema.json");
const MANIFEST_SCHEMA: &str =
    include_str!("../../../schemas/laneflow-scenario-manifest-v0.1.schema.json");
const SPATIAL_FIXTURE: &str = include_str!("../../../examples/data/v0.1-campus.spatial.json");
const MANIFEST_FIXTURE: &str = include_str!("../../../examples/data/v0.1-campus.scenario.json");

fn parse(source: &str) -> Value {
    serde_json::from_str(source).expect("repository JSON must parse")
}

#[test]
fn scenario_schemas_satisfy_draft_2020_12_meta_schema() {
    for source in [SPATIAL_SCHEMA, MANIFEST_SCHEMA] {
        draft202012::meta::validate(&parse(source))
            .expect("scenario schema must satisfy Draft 2020-12");
    }
}

#[test]
fn scenario_schemas_lock_identifiers_versions_and_closed_shapes() {
    let spatial = parse(SPATIAL_SCHEMA);
    assert_eq!(
        spatial["$id"],
        "https://illusion-tech.github.io/laneflow/schema/laneflow-spatial-v0.1.schema.json"
    );
    assert_eq!(spatial["properties"]["formatVersion"]["const"], "0.1");
    assert_eq!(spatial["additionalProperties"], false);
    assert_eq!(
        spatial["$defs"]["spatialEdge"]["additionalProperties"],
        false
    );
    assert_eq!(
        spatial["$defs"]["centerline"]["additionalProperties"],
        false
    );

    let point = &spatial["$defs"]["point3"];
    assert_eq!(point["minItems"], 3);
    assert_eq!(point["maxItems"], 3);
    assert_eq!(point["items"], false);
    assert_eq!(point["prefixItems"].as_array().map(Vec::len), Some(3));
    assert_eq!(spatial["$defs"]["coordinate"]["minimum"], -16_384);
    assert_eq!(spatial["$defs"]["coordinate"]["maximum"], 16_384);

    let manifest = parse(MANIFEST_SCHEMA);
    assert_eq!(
        manifest["$id"],
        "https://illusion-tech.github.io/laneflow/schema/laneflow-scenario-manifest-v0.1.schema.json"
    );
    assert_eq!(manifest["properties"]["formatVersion"]["const"], "0.1");
    assert_eq!(manifest["additionalProperties"], false);
    assert_eq!(
        manifest["$defs"]["trafficArtifact"]["properties"]["mediaType"]["const"],
        "application/vnd.laneflow.traffic+json"
    );
    assert_eq!(
        manifest["$defs"]["spatialArtifact"]["properties"]["mediaType"]["const"],
        "application/vnd.laneflow.spatial+json"
    );
    assert_eq!(
        manifest["$defs"]["digest"]["pattern"],
        "^sha256:[0-9a-f]{64}$"
    );
    assert_eq!(
        manifest["$defs"]["size"]["maximum"],
        9_007_199_254_740_991_u64
    );
}

#[test]
fn canonical_scenario_fixtures_satisfy_their_schemas() {
    draft202012::validate(&parse(SPATIAL_SCHEMA), &parse(SPATIAL_FIXTURE))
        .expect("canonical spatial fixture must satisfy its schema");
    draft202012::validate(&parse(MANIFEST_SCHEMA), &parse(MANIFEST_FIXTURE))
        .expect("canonical manifest fixture must satisfy its schema");
}

#[test]
fn spatial_schema_requires_exact_xyz_triples_and_two_point_centerlines() {
    let schema = parse(SPATIAL_SCHEMA);
    let fixture = parse(SPATIAL_FIXTURE);

    for invalid_point in [
        json!([0.0, 0.0]),
        json!([0.0, 0.0, 0.0, 1.0]),
        json!({ "x": 0.0, "y": 0.0, "z": 0.0 }),
    ] {
        let mut instance = fixture.clone();
        instance["edges"][0]["centerline"]["points"][0] = invalid_point;
        assert!(draft202012::validate(&schema, &instance).is_err());
    }

    let mut one_point = fixture.clone();
    one_point["edges"][0]["centerline"]["points"] = json!([[0.0, 0.0, 0.0]]);
    assert!(draft202012::validate(&schema, &one_point).is_err());

    for invalid_coordinate in [json!(-16_384.000_1), json!(16_384.000_1)] {
        let mut instance = fixture.clone();
        instance["edges"][0]["centerline"]["points"][0][0] = invalid_coordinate;
        assert!(draft202012::validate(&schema, &instance).is_err());
    }
}

#[test]
fn scenario_schemas_reject_unknown_fields_and_invalid_descriptors() {
    let spatial_schema = parse(SPATIAL_SCHEMA);
    let mut spatial = parse(SPATIAL_FIXTURE);
    spatial["edges"][0]["centerline"]["future"] = json!(true);
    assert!(draft202012::validate(&spatial_schema, &spatial).is_err());

    let manifest_schema = parse(MANIFEST_SCHEMA);
    let fixture = parse(MANIFEST_FIXTURE);
    for (field, invalid) in [
        ("artifactRef", json!("")),
        ("mediaType", json!("application/json")),
        ("digest", json!("sha256:ABCDEF")),
        ("size", json!(-1)),
        ("size", json!(9_007_199_254_740_992_u64)),
    ] {
        let mut instance = fixture.clone();
        instance["traffic"][field] = invalid;
        assert!(draft202012::validate(&manifest_schema, &instance).is_err());
    }

    let mut unknown = fixture;
    unknown["spatial"]["future"] = json!(true);
    assert!(draft202012::validate(&manifest_schema, &unknown).is_err());
}
