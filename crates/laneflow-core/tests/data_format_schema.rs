use laneflow_core::EDGE_BOUNDARY_EPSILON;
use serde_json::Value;

fn schema() -> Value {
    serde_json::from_str(include_str!(
        "../../../schemas/laneflow-data-v0.2.schema.json"
    ))
    .expect("data format schema must be valid JSON")
}

#[test]
fn schema_locks_v0_2_format_version_units_and_external_id_shape() {
    let schema = schema();

    let mut required = string_array(&schema["required"]);
    required.sort_unstable();
    assert_eq!(required, ["formatVersion", "laneGraph", "routes", "units"]);
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["formatVersion"]["const"], "0.2");
    assert_eq!(
        schema["$defs"]["unitSpec"]["properties"]["distance"]["const"],
        "meter"
    );
    assert_eq!(schema["$defs"]["externalId"]["minLength"], 1);
    assert_eq!(schema["$defs"]["externalId"]["maxLength"], 128);
    assert_eq!(
        schema["$defs"]["externalId"]["pattern"],
        "^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$"
    );
    assert_eq!(
        schema["$defs"]["laneEdge"]["properties"]["length"]["exclusiveMinimum"]
            .as_f64()
            .expect("exclusiveMinimum must be numeric"),
        EDGE_BOUNDARY_EPSILON
    );
}

#[test]
fn schema_keeps_topology_validation_out_of_json_shape_layer() {
    let schema = schema();

    assert_eq!(
        schema["$defs"]["laneConnection"]["required"],
        serde_json::json!(["to"])
    );
    assert!(
        schema["$defs"]["laneEdge"]["properties"]["connections"]
            .get("uniqueItems")
            .and_then(Value::as_bool)
            != Some(true),
        "duplicate connection target must stay in domain validation because uniqueness is by `to`"
    );
    assert_eq!(
        schema["$defs"]["route"]["properties"]["edges"]["minItems"],
        1
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
