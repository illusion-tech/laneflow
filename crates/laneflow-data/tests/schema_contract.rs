use jsonschema::draft202012;
use laneflow_core::{EDGE_BOUNDARY_EPSILON, GEOMETRY_GAP_EPSILON, MAX_PORTABLE_SIGNAL_TIME_MS};
use serde_json::Value;

const CURRENT_SCHEMA: &str = include_str!("../../../schemas/laneflow-data-v0.4.schema.json");
const SIGNALS_FIXTURE: &str =
    include_str!("../../../examples/data/v0.4-signals-baseline.laneflow.json");
const EMPTY_SIGNALS_FIXTURE: &str =
    include_str!("../../../examples/data/v0.4-empty-signals.laneflow.json");

fn schema(source: &str) -> Value {
    serde_json::from_str(source).expect("data format schema must be valid JSON")
}

#[test]
fn schema_satisfies_draft_2020_12_meta_schema() {
    draft202012::meta::validate(&schema(CURRENT_SCHEMA))
        .expect("repository schema must satisfy Draft 2020-12");
}

#[test]
fn schema_locks_current_version_units_and_required_signals_shape() {
    let schema = schema(CURRENT_SCHEMA);

    let mut required = string_array(&schema["required"]);
    required.sort_unstable();
    assert_eq!(
        required,
        [
            "formatVersion",
            "laneGraph",
            "routes",
            "signals",
            "units",
            "vehicleProfiles"
        ]
    );
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["formatVersion"]["const"], "0.4");
    assert_eq!(
        schema["$defs"]["unitSpec"]["properties"]["time"]["const"],
        "second"
    );
    assert_eq!(
        schema["$defs"]["signals"]["required"],
        serde_json::json!(["stopLines", "movementGates", "groups", "controllers"])
    );
    assert_eq!(schema["$defs"]["signals"]["additionalProperties"], false);
    assert_external_id_and_numeric_bounds(&schema);
}

#[test]
fn schema_locks_v0_4_id_names_tagged_union_and_timing_bounds() {
    let schema = schema(CURRENT_SCHEMA);
    assert_eq!(
        schema["$defs"]["laneConnection"]["required"],
        serde_json::json!(["toEdgeId"])
    );
    assert_eq!(
        schema["$defs"]["route"]["required"],
        serde_json::json!(["id", "edgeIds"])
    );
    assert_eq!(
        schema["$defs"]["groupSignalControl"]["properties"]["kind"]["const"],
        "group"
    );
    assert_eq!(
        schema["$defs"]["noneSignalControl"]["properties"]["kind"]["const"],
        "none"
    );
    assert_eq!(
        schema["$defs"]["portableMilliseconds"]["maximum"],
        MAX_PORTABLE_SIGNAL_TIME_MS
    );
    assert_eq!(
        schema["$defs"]["positivePortableMilliseconds"]["maximum"],
        MAX_PORTABLE_SIGNAL_TIME_MS
    );
}

#[test]
fn schema_keeps_cross_record_domain_validation_in_core() {
    let current = schema(CURRENT_SCHEMA);
    assert!(
        current["$defs"]["laneEdge"]["properties"]["connections"]
            .get("uniqueItems")
            .and_then(Value::as_bool)
            != Some(true),
        "connection uniqueness is a Core domain rule"
    );
    assert!(
        current["$defs"]["signalController"]["properties"]["groupIds"]
            .get("uniqueItems")
            .and_then(Value::as_bool)
            != Some(true),
        "controller ownership and duplicate membership are Core domain rules"
    );
    assert!(
        current["$defs"]["signalPhase"]["properties"]["states"]
            .get("uniqueItems")
            .and_then(Value::as_bool)
            != Some(true),
        "complete-state membership is a Core domain rule"
    );
    assert!(
        current["$defs"]["vehicleProfile"].get("allOf").is_none(),
        "deceleration cross-field ordering stays in Core domain validation"
    );
}

#[test]
fn both_canonical_current_fixtures_satisfy_schema() {
    let schema = schema(CURRENT_SCHEMA);
    for source in [SIGNALS_FIXTURE, EMPTY_SIGNALS_FIXTURE] {
        let instance: Value =
            serde_json::from_str(source).expect("current fixture must be valid JSON");
        draft202012::validate(&schema, &instance)
            .expect("current fixture must satisfy repository schema");
    }
}

#[test]
fn schema_rejects_legacy_fields_json_ld_and_open_signal_control() {
    let schema = schema(CURRENT_SCHEMA);
    let mut instance: Value = serde_json::from_str(EMPTY_SIGNALS_FIXTURE).expect("fixture JSON");
    instance["laneGraph"]["edges"][0]["connections"][0] = serde_json::json!({ "to": "exit" });
    assert!(draft202012::validate(&schema, &instance).is_err());

    let mut instance: Value = serde_json::from_str(EMPTY_SIGNALS_FIXTURE).expect("fixture JSON");
    instance["routes"][0] = serde_json::json!({ "id": "main-route", "edges": ["entry", "exit"] });
    assert!(draft202012::validate(&schema, &instance).is_err());

    let mut instance: Value = serde_json::from_str(EMPTY_SIGNALS_FIXTURE).expect("fixture JSON");
    instance["@context"] = serde_json::json!({ "@vocab": "https://example.invalid/" });
    assert!(draft202012::validate(&schema, &instance).is_err());

    let mut instance: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
    instance["signals"]["movementGates"][1]["signalControl"]["groupId"] = serde_json::json!("main");
    assert!(draft202012::validate(&schema, &instance).is_err());
}

#[test]
fn schema_enforces_signal_enums_and_portable_integer_field_bounds() {
    let schema = schema(CURRENT_SCHEMA);

    for (duration_ms, offset_ms) in [
        (1, 0),
        (MAX_PORTABLE_SIGNAL_TIME_MS, MAX_PORTABLE_SIGNAL_TIME_MS),
    ] {
        let mut instance: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
        instance["signals"]["controllers"][0]["phases"][0]["durationMs"] =
            serde_json::json!(duration_ms);
        instance["signals"]["controllers"][0]["offsetMs"] = serde_json::json!(offset_ms);
        draft202012::validate(&schema, &instance)
            .expect("schema must accept portable field boundaries");
    }

    for invalid in [
        serde_json::json!(0),
        serde_json::json!(MAX_PORTABLE_SIGNAL_TIME_MS + 1),
        serde_json::json!(-1),
        serde_json::json!(1.5),
    ] {
        let mut instance: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
        instance["signals"]["controllers"][0]["phases"][0]["durationMs"] = invalid;
        assert!(draft202012::validate(&schema, &instance).is_err());
    }

    for invalid in [
        serde_json::json!(-1),
        serde_json::json!(MAX_PORTABLE_SIGNAL_TIME_MS + 1),
        serde_json::json!(1.5),
    ] {
        let mut instance: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
        instance["signals"]["controllers"][0]["offsetMs"] = invalid;
        assert!(draft202012::validate(&schema, &instance).is_err());
    }

    for mutate in ["location", "kind", "aspect"] {
        let mut instance: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
        match mutate {
            "location" => {
                instance["signals"]["stopLines"][0]["location"] = serde_json::json!("midEdge")
            }
            "kind" => instance["signals"]["controllers"][0]["kind"] = serde_json::json!("actuated"),
            "aspect" => {
                instance["signals"]["controllers"][0]["phases"][0]["states"][0]["aspect"] =
                    serde_json::json!("blue")
            }
            _ => unreachable!(),
        }
        assert!(draft202012::validate(&schema, &instance).is_err());
    }
}

fn assert_external_id_and_numeric_bounds(schema: &Value) {
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
    assert_eq!(
        schema["$defs"]["vehicleProfile"]["properties"]["length"]["exclusiveMinimum"]
            .as_f64()
            .expect("profile length minimum must be numeric"),
        GEOMETRY_GAP_EPSILON
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
