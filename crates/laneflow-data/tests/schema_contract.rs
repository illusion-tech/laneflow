use jsonschema::draft202012;
use laneflow_core::{EDGE_BOUNDARY_EPSILON, GEOMETRY_GAP_EPSILON};
use serde_json::Value;

const CURRENT_SCHEMA: &str = include_str!("../../../schemas/laneflow-data-v0.3.schema.json");
const CURRENT_FIXTURE: &str =
    include_str!("../../../examples/data/v0.3-profile-baseline.laneflow.json");

fn schema(source: &str) -> Value {
    serde_json::from_str(source).expect("data format schema must be valid JSON")
}

#[test]
fn schema_satisfies_draft_2020_12_meta_schema() {
    draft202012::meta::validate(&schema(CURRENT_SCHEMA))
        .expect("repository schema must satisfy Draft 2020-12");
}

#[test]
fn schema_locks_current_version_units_and_vehicle_profile_shape() {
    let schema = schema(CURRENT_SCHEMA);

    let mut required = string_array(&schema["required"]);
    required.sort_unstable();
    assert_eq!(
        required,
        [
            "formatVersion",
            "laneGraph",
            "routes",
            "units",
            "vehicleProfiles"
        ]
    );
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["formatVersion"]["const"], "0.3");
    assert_eq!(
        schema["$defs"]["unitSpec"]["required"],
        serde_json::json!(["distance", "time"])
    );
    assert_eq!(
        schema["$defs"]["unitSpec"]["properties"]["time"]["const"],
        "second"
    );
    assert_eq!(
        schema["$defs"]["vehicleProfile"]["properties"]["model"]["const"],
        "iidm"
    );
    assert_eq!(
        schema["$defs"]["vehicleProfile"]["properties"]["length"]["exclusiveMinimum"]
            .as_f64()
            .expect("profile length minimum must be numeric"),
        GEOMETRY_GAP_EPSILON
    );
    assert_external_id_and_lane_length(&schema);
}

#[test]
fn schema_keeps_domain_validation_out_of_json_shape_layer() {
    let current = schema(CURRENT_SCHEMA);
    assert_eq!(
        current["$defs"]["laneConnection"]["required"],
        serde_json::json!(["to"])
    );
    assert!(
        current["$defs"]["laneEdge"]["properties"]["connections"]
            .get("uniqueItems")
            .and_then(Value::as_bool)
            != Some(true),
        "connection uniqueness is a Core domain rule"
    );
    assert_eq!(
        current["$defs"]["route"]["properties"]["edges"]["minItems"],
        1
    );
    assert!(
        current["$defs"]["vehicleProfile"].get("allOf").is_none(),
        "deceleration cross-field ordering stays in Core domain validation"
    );
}

#[test]
fn current_fixture_satisfies_schema() {
    let instance: Value =
        serde_json::from_str(CURRENT_FIXTURE).expect("current fixture must be valid JSON");
    draft202012::validate(&schema(CURRENT_SCHEMA), &instance)
        .expect("current fixture must satisfy repository schema");
}

fn assert_external_id_and_lane_length(schema: &Value) {
    assert_eq!(schema["$defs"]["externalId"]["minLength"], 1);
    assert_eq!(schema["$defs"]["externalId"]["maxLength"], 128);
    assert_eq!(
        schema["$defs"]["externalId"]["pattern"],
        "^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$"
    );
    assert_eq!(
        schema["$defs"]["laneEdge"]["properties"]["length"]["exclusiveMinimum"]
            .as_f64()
            .expect("edge length minimum must be numeric"),
        EDGE_BOUNDARY_EPSILON
    );
}

fn string_array(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .expect("value must be an array")
        .iter()
        .map(|item| item.as_str().expect("array item must be a string"))
        .collect()
}
