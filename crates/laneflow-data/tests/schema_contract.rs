use jsonschema::draft202012;
use laneflow_core::MAX_PORTABLE_SIGNAL_TIME_MS;
use serde_json::Value;

const CURRENT_SCHEMA: &str = include_str!("../../../schemas/laneflow-data-v0.9.schema.json");
const SIGNALS_FIXTURE: &str =
    include_str!("../../../examples/data/v0.9-parking-signals-baseline.laneflow.json");
const EMPTY_SIGNALS_FIXTURE: &str =
    include_str!("../../../examples/data/v0.9-empty-signals-and-parking.laneflow.json");
const CURRENT_V0_9_MIN_EDGE_LENGTH_EXCLUSIVE_METERS: f64 = 1.0e-9;
const CURRENT_V0_9_MIN_VEHICLE_LENGTH_EXCLUSIVE_METERS: f64 = 1.0e-9;
const CURRENT_V0_9_PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS: f64 = 1.0e-9;
const CURRENT_V0_9_MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS: f64 = 1.0e-9;
const CURRENT_V0_9_MIN_PARKING_EXTENT_EXCLUSIVE_METERS: f64 = 1.0e-9;

fn schema(source: &str) -> Value {
    serde_json::from_str(source).expect("data format schema must be valid JSON")
}

#[test]
fn schema_satisfies_draft_2020_12_meta_schema() {
    draft202012::meta::validate(&schema(CURRENT_SCHEMA))
        .expect("repository schema must satisfy Draft 2020-12");
}

#[test]
fn schema_locks_current_version_units_and_required_static_shape() {
    let schema = schema(CURRENT_SCHEMA);

    assert_eq!(
        schema["$id"],
        "https://illusion-tech.github.io/laneflow/schema/laneflow-data-v0.9.schema.json"
    );
    assert_eq!(schema["title"], "LaneFlow Data Package v0.9");

    let mut required = string_array(&schema["required"]);
    required.sort_unstable();
    assert_eq!(
        required,
        [
            "accessRules",
            "facilityBands",
            "formatVersion",
            "junctions",
            "laneGraph",
            "laneGroups",
            "maneuverPaths",
            "movements",
            "parking",
            "participantClasses",
            "roadCorridors",
            "roadSections",
            "routes",
            "signals",
            "units",
            "vehicleProfiles"
        ]
    );
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["formatVersion"]["const"], "0.9");
    assert_eq!(
        schema["$defs"]["laneEdge"]["required"],
        serde_json::json!(["id", "length", "speedLimit", "connections"])
    );
    assert_eq!(
        schema["$defs"]["laneEdge"]["properties"]["speedLimit"]["exclusiveMinimum"],
        0
    );
    assert_eq!(
        schema["$defs"]["unitSpec"]["properties"]["time"]["const"],
        "second"
    );
    assert_eq!(
        schema["$defs"]["signals"]["required"],
        serde_json::json!(["stopLines", "maneuverGates", "groups", "controllers"])
    );
    assert_eq!(schema["$defs"]["signals"]["additionalProperties"], false);
    assert_eq!(
        schema["$defs"]["parking"]["required"],
        serde_json::json!(["areas", "spaces"])
    );
    assert_eq!(schema["$defs"]["parking"]["additionalProperties"], false);
    assert_eq!(
        schema["$defs"]["parkingSpace"]["required"],
        serde_json::json!(["id", "entry", "exit", "geometry"])
    );
    assert_eq!(
        schema["$defs"]["parkingSpace"]["additionalProperties"],
        false
    );
    assert_eq!(
        schema["$defs"]["vehicleProfile"]["required"],
        serde_json::json!([
            "id",
            "length",
            "model",
            "desiredSpeed",
            "minGap",
            "timeHeadway",
            "maxAcceleration",
            "comfortableDeceleration",
            "emergencyDeceleration",
            "participantClassId"
        ])
    );
    assert_external_id_and_numeric_bounds(&schema);
}

#[test]
fn schema_locks_v0_9_topology_gate_ids_tagged_union_and_timing_bounds() {
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
        schema["$defs"]["movement"]["required"],
        serde_json::json!(["id", "junctionId"])
    );
    assert_eq!(
        schema["$defs"]["maneuverPath"]["required"],
        serde_json::json!([
            "id",
            "movementId",
            "entryEdgeId",
            "internalEdgeIds",
            "exitEdgeId"
        ])
    );
    assert_eq!(
        schema["$defs"]["maneuverGate"]["required"],
        serde_json::json!([
            "id",
            "maneuverPathId",
            "transitionIndex",
            "stopLineId",
            "signalControl"
        ])
    );
    assert_eq!(
        schema["$defs"]["maneuverGate"]["properties"]["transitionIndex"]["maximum"],
        u32::MAX
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
fn schema_accepts_omitted_area_id_and_rejects_explicit_null() {
    let schema = schema(CURRENT_SCHEMA);
    let instance: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
    assert!(instance["parking"]["spaces"][2].get("areaId").is_none());
    draft202012::validate(&schema, &instance).expect("omitted areaId must satisfy schema");

    let mut explicit_null = instance;
    explicit_null["parking"]["spaces"][2]["areaId"] = Value::Null;
    assert!(draft202012::validate(&schema, &explicit_null).is_err());
}

#[test]
fn schema_enforces_parking_closed_shapes_and_numeric_field_bounds() {
    let schema = schema(CURRENT_SCHEMA);

    for target in ["parking", "area", "space", "anchor", "geometry"] {
        let mut instance: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
        match target {
            "parking" => instance["parking"]["typo"] = serde_json::json!(true),
            "area" => instance["parking"]["areas"][0]["typo"] = serde_json::json!(true),
            "space" => instance["parking"]["spaces"][0]["typo"] = serde_json::json!(true),
            "anchor" => instance["parking"]["spaces"][0]["entry"]["typo"] = serde_json::json!(true),
            "geometry" => {
                instance["parking"]["spaces"][0]["geometry"]["typo"] = serde_json::json!(true)
            }
            _ => unreachable!(),
        }
        assert!(draft202012::validate(&schema, &instance).is_err());
    }

    for (path, invalid) in [
        (
            "progress",
            serde_json::json!(CURRENT_V0_9_PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS),
        ),
        (
            "lateralOffset",
            serde_json::json!(CURRENT_V0_9_MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS),
        ),
        (
            "headingOffsetRadians",
            serde_json::json!(std::f64::consts::PI),
        ),
        (
            "length",
            serde_json::json!(CURRENT_V0_9_MIN_PARKING_EXTENT_EXCLUSIVE_METERS),
        ),
        ("width", serde_json::json!(0.0)),
    ] {
        let mut instance: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
        if path == "progress" {
            instance["parking"]["spaces"][0]["entry"][path] = invalid;
        } else {
            instance["parking"]["spaces"][0]["geometry"][path] = invalid;
        }
        assert!(
            draft202012::validate(&schema, &instance).is_err(),
            "{path} boundary must be rejected"
        );
    }

    let mut lower_heading: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
    lower_heading["parking"]["spaces"][0]["geometry"]["headingOffsetRadians"] =
        serde_json::json!(-std::f64::consts::PI);
    draft202012::validate(&schema, &lower_heading).expect("-PI is canonical");
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
    instance["signals"]["maneuverGates"][1]["signalControl"]["groupId"] = serde_json::json!("main");
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

#[test]
fn schema_locks_v0_9_cross_section_and_access_shapes() {
    let schema = schema(CURRENT_SCHEMA);

    assert_eq!(
        schema["$defs"]["participantClass"]["required"],
        serde_json::json!(["id"])
    );
    assert_eq!(
        schema["$defs"]["facilityBand"]["required"],
        serde_json::json!(["id", "kindId"])
    );
    assert_eq!(
        schema["$defs"]["roadSection"]["required"],
        serde_json::json!(["id", "kindId", "lanes"])
    );
    assert_eq!(
        schema["$defs"]["sectionLane"]["required"],
        serde_json::json!(["edgeIds"])
    );
    assert_eq!(
        schema["$defs"]["laneGroup"]["required"],
        serde_json::json!(["id", "roadSectionId"])
    );
    assert_eq!(
        schema["$defs"]["roadCorridor"]["required"],
        serde_json::json!(["id", "referenceSectionId", "elements"])
    );
    assert_eq!(
        schema["$defs"]["accessRule"]["required"],
        serde_json::json!(["id", "target", "effect", "participantClassIds"])
    );
    assert_eq!(
        schema["$defs"]["accessTarget"]["properties"]["kind"]["enum"],
        serde_json::json!([
            "laneEdge",
            "laneGroup",
            "roadSection",
            "maneuverPath",
            "facilityBand"
        ])
    );
    assert_eq!(
        schema["$defs"]["accessRule"]["properties"]["effect"]["enum"],
        serde_json::json!(["allow", "deny"])
    );
    assert_eq!(
        schema["$defs"]["accessRule"]["properties"]["participantClassIds"]["minItems"],
        1
    );
    // 结构数组在契约中显式非空（SSOT §3/§6.1），schema 与 loader 拒绝口径保持一致。
    assert_eq!(
        schema["$defs"]["roadSection"]["properties"]["lanes"]["minItems"],
        1
    );
    assert_eq!(
        schema["$defs"]["sectionLane"]["properties"]["edgeIds"]["minItems"],
        1
    );
    assert_eq!(
        schema["$defs"]["roadCorridor"]["properties"]["elements"]["minItems"],
        1
    );
    assert_eq!(
        schema["$defs"]["accessRule"]["properties"]["timeWindows"]["minItems"],
        1
    );
    assert_eq!(
        schema["$defs"]["accessRule"]["properties"]["priority"],
        serde_json::json!({
            "type": "integer",
            "minimum": -2147483648_i64,
            "maximum": 2147483647_i64
        })
    );
    assert_eq!(
        schema["$defs"]["timeWindow"]["properties"]["startMinuteOfDay"],
        serde_json::json!({ "type": "integer", "minimum": 0, "maximum": 1439 })
    );
    assert_eq!(
        schema["$defs"]["timeWindow"]["properties"]["endMinuteOfDay"],
        serde_json::json!({ "type": "integer", "minimum": 1, "maximum": 1440 })
    );
    assert_eq!(
        schema["$defs"]["timeWindow"]["properties"]["days"]["minItems"],
        1
    );
    assert_eq!(
        schema["$defs"]["timeWindow"]["properties"]["days"]["items"]["enum"],
        serde_json::json!(["mon", "tue", "wed", "thu", "fri", "sat", "sun"])
    );

    for def in [
        "participantClass",
        "facilityBand",
        "roadSection",
        "sectionLane",
        "laneGroup",
        "roadCorridor",
        "corridorSectionElement",
        "corridorBandElement",
        "accessRule",
        "accessTarget",
        "timeWindow",
        "regulation",
    ] {
        assert_eq!(
            schema["$defs"][def]["additionalProperties"], false,
            "{def} must stay a closed shape"
        );
    }
}

#[test]
fn schema_enforces_v0_9_participant_class_and_access_ranges() {
    let schema = schema(CURRENT_SCHEMA);

    let mut missing_class_id: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
    missing_class_id["vehicleProfiles"][0]
        .as_object_mut()
        .expect("profile")
        .remove("participantClassId");
    assert!(
        draft202012::validate(&schema, &missing_class_id).is_err(),
        "participantClassId is required"
    );

    let mut unknown_class_id: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
    unknown_class_id["vehicleProfiles"][0]["participantClassId"] = serde_json::json!("bad id");
    assert!(draft202012::validate(&schema, &unknown_class_id).is_err());

    let valid_rule = serde_json::json!({
        "id": "rule-1",
        "target": { "kind": "laneEdge", "id": "entry" },
        "effect": "deny",
        "participantClassIds": ["motorVehicle"],
        "timeWindows": [{
            "days": ["mon", "sun"],
            "startMinuteOfDay": 0,
            "endMinuteOfDay": 1440
        }],
        "regulation": { "jurisdiction": "cn-sh", "version": "2024", "source": "gov" },
        "priority": -5
    });
    let mut valid: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
    valid["accessRules"] = serde_json::json!([valid_rule.clone()]);
    draft202012::validate(&schema, &valid).expect("full legal accessRule must satisfy schema");

    for (field, invalid) in [
        ("priority", serde_json::json!(2147483648_i64)),
        ("priority", serde_json::json!(-2147483649_i64)),
        ("priority", serde_json::json!(1.5)),
    ] {
        let mut rule = valid_rule.clone();
        rule[field] = invalid;
        let mut instance: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
        instance["accessRules"] = serde_json::json!([rule]);
        assert!(
            draft202012::validate(&schema, &instance).is_err(),
            "{field} out of i32 range must be rejected"
        );
    }

    for (start, end) in [(-1, 60), (1440, 60), (0, 0), (0, 1441)] {
        let mut rule = valid_rule.clone();
        rule["timeWindows"][0]["startMinuteOfDay"] = serde_json::json!(start);
        rule["timeWindows"][0]["endMinuteOfDay"] = serde_json::json!(end);
        let mut instance: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
        instance["accessRules"] = serde_json::json!([rule]);
        assert!(
            draft202012::validate(&schema, &instance).is_err(),
            "minute range ({start}, {end}) must be rejected"
        );
    }

    for days in [serde_json::json!([]), serde_json::json!(["monday"])] {
        let mut rule = valid_rule.clone();
        rule["timeWindows"][0]["days"] = days;
        let mut instance: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
        instance["accessRules"] = serde_json::json!([rule]);
        assert!(
            draft202012::validate(&schema, &instance).is_err(),
            "days must be a non-empty mon..sun subset"
        );
    }

    for kind in [serde_json::json!("lane"), serde_json::json!("facilityband")] {
        let mut rule = valid_rule.clone();
        rule["target"]["kind"] = kind;
        let mut instance: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
        instance["accessRules"] = serde_json::json!([rule]);
        assert!(draft202012::validate(&schema, &instance).is_err());
    }

    let mut empty_class_ids = valid_rule.clone();
    empty_class_ids["participantClassIds"] = serde_json::json!([]);
    let mut instance: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
    instance["accessRules"] = serde_json::json!([empty_class_ids]);
    assert!(draft202012::validate(&schema, &instance).is_err());

    // 结构数组空数组拒绝（与 production normalization / capability guard 口径一致）。
    let mut empty_time_windows = valid_rule.clone();
    empty_time_windows["timeWindows"] = serde_json::json!([]);
    let mut instance: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
    instance["accessRules"] = serde_json::json!([empty_time_windows]);
    assert!(
        draft202012::validate(&schema, &instance).is_err(),
        "declared timeWindows must be non-empty"
    );

    let mut empty_lanes: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
    empty_lanes["roadSections"][0]["lanes"] = serde_json::json!([]);
    assert!(
        draft202012::validate(&schema, &empty_lanes).is_err(),
        "roadSection.lanes must be non-empty"
    );

    let mut empty_edge_ids: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
    empty_edge_ids["roadSections"][0]["lanes"][0]["edgeIds"] = serde_json::json!([]);
    assert!(
        draft202012::validate(&schema, &empty_edge_ids).is_err(),
        "sectionLane.edgeIds must be non-empty"
    );

    let mut empty_elements: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
    empty_elements["roadCorridors"][0]["elements"] = serde_json::json!([]);
    assert!(
        draft202012::validate(&schema, &empty_elements).is_err(),
        "roadCorridor.elements must be non-empty"
    );

    for element in [
        serde_json::json!({ "sectionId": "section-main", "bandId": "band-1" }),
        serde_json::json!({ "kindId": "section-main" }),
    ] {
        let mut instance: Value = serde_json::from_str(SIGNALS_FIXTURE).expect("fixture JSON");
        instance["roadCorridors"][0]["elements"] = serde_json::json!([element]);
        assert!(
            draft202012::validate(&schema, &instance).is_err(),
            "corridor element must be exactly one closed reference"
        );
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
        CURRENT_V0_9_MIN_EDGE_LENGTH_EXCLUSIVE_METERS
    );
    assert_eq!(
        schema["$defs"]["vehicleProfile"]["properties"]["length"]["exclusiveMinimum"]
            .as_f64()
            .expect("profile length minimum must be numeric"),
        CURRENT_V0_9_MIN_VEHICLE_LENGTH_EXCLUSIVE_METERS
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
