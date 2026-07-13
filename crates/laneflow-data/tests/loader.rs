use laneflow_core::CoreError;
use laneflow_data::{CURRENT_FORMAT_VERSION, DataError, LoadedPackage, from_json_str};
use serde_json::{Value, json};

const V0_3_FIXTURE: &str =
    include_str!("../../../examples/data/v0.3-profile-baseline.laneflow.json");

#[test]
fn v0_3_loader_normalizes_profile_registry_and_resolvers() {
    assert_eq!(CURRENT_FORMAT_VERSION, "0.3");
    let loaded = from_json_str(V0_3_FIXTURE).expect("v0.3 fixture must load");
    let profiles = loaded.initial_traffic_data().vehicle_profiles();
    let handle = profiles
        .profile_handle("passenger-car")
        .expect("profile handle must resolve");
    assert_eq!(profiles.profile_external_id(handle), Some("passenger-car"));
    assert_eq!(
        profiles
            .profile(handle)
            .expect("profile must resolve")
            .iidm()
            .desired_speed,
        13.9
    );
}

#[test]
fn v0_3_requires_profile_field_but_allows_empty_profile_array() {
    let mut value = v0_3_value();
    value["vehicleProfiles"] = json!([]);

    let loaded = load_value(value).expect("empty explicit v0.3 profile list is valid");
    assert!(loaded.initial_traffic_data().vehicle_profiles().is_empty());
}

#[test]
fn unsupported_version_is_rejected_before_units() {
    let mut value = v0_3_value();
    value["formatVersion"] = json!("0.4");
    value["units"]["distance"] = json!("kilometer");
    value["futureTopLevelField"] = json!({ "newShape": true });

    let error = load_value(value).expect_err("unknown version must fail first");
    std::assert_matches!(
        error,
        DataError::UnsupportedFormatVersion { expected: "0.3", actual }
            if actual == "0.4"
    );
}

#[test]
fn stale_version_is_rejected_before_current_shape_validation() {
    let mut value = v0_3_value();
    value["formatVersion"] = json!("0.2");
    value["units"] = json!({ "distance": "meter" });
    value
        .as_object_mut()
        .expect("fixture root must be object")
        .remove("vehicleProfiles");

    let error = load_value(value).expect_err("stale format must not enter v0.3 validation");
    std::assert_matches!(
        error,
        DataError::UnsupportedFormatVersion { expected: "0.3", actual }
            if actual == "0.2"
    );
}

#[test]
fn current_version_requires_current_shape() {
    let mut value = v0_3_value();
    value
        .as_object_mut()
        .expect("fixture root must be object")
        .remove("vehicleProfiles");
    let error = load_value(value).expect_err("current format requires vehicleProfiles");
    std::assert_matches!(error, DataError::JsonShape { .. });

    let mut value = v0_3_value();
    value["units"]
        .as_object_mut()
        .expect("units must be object")
        .remove("time");
    let error = load_value(value).expect_err("current format requires units.time");
    std::assert_matches!(error, DataError::JsonShape { .. });
}

#[test]
fn explicit_null_version_fields_are_shape_errors_not_missing_fields() {
    for (path, mut value) in [
        ("vehicleProfiles", v0_3_value()),
        ("units.time", v0_3_value()),
    ] {
        match path {
            "vehicleProfiles" => value["vehicleProfiles"] = Value::Null,
            "units.time" => value["units"]["time"] = Value::Null,
            _ => unreachable!("known paths only"),
        }
        let error = load_value(value).expect_err("explicit null must fail shape validation");
        std::assert_matches!(
            error,
            DataError::JsonShape { path: actual, .. } if actual.contains(path)
        );
    }
}

#[test]
fn missing_format_version_and_invalid_units_are_structured_errors() {
    let mut missing_version = v0_3_value();
    missing_version
        .as_object_mut()
        .expect("fixture root must be object")
        .remove("formatVersion");
    let error = load_value(missing_version).expect_err("format version is required");
    std::assert_matches!(error, DataError::JsonShape { path, .. } if path == "$" );

    let mut invalid_distance = v0_3_value();
    invalid_distance["units"]["distance"] = json!("kilometer");
    let error = load_value(invalid_distance).expect_err("distance unit must be meter");
    std::assert_matches!(
        error,
        DataError::InvalidUnit {
            path: "units.distance",
            expected: "meter",
            actual,
        } if actual == "kilometer"
    );

    let mut invalid_time = v0_3_value();
    invalid_time["units"]["time"] = json!("millisecond");
    let error = load_value(invalid_time).expect_err("time unit must be second");
    std::assert_matches!(
        error,
        DataError::InvalidUnit {
            path: "units.time",
            expected: "second",
            actual,
        } if actual == "millisecond"
    );
}

#[test]
fn unknown_profile_field_reports_serde_path() {
    let mut value = v0_3_value();
    value["vehicleProfiles"][0]["typo"] = json!(true);

    let error = load_value(value).expect_err("unknown profile field must fail");
    std::assert_matches!(
        error,
        DataError::JsonShape { path, line, column, .. }
            if path.contains("vehicleProfiles[0]") && line > 0 && column > 0
    );
}

#[test]
fn invalid_profile_value_preserves_input_path_and_core_source() {
    let mut value = v0_3_value();
    value["vehicleProfiles"][0]["length"] = json!(0.0);

    let error = load_value(value).expect_err("invalid profile length must fail");
    std::assert_matches!(
        error,
        DataError::CoreDomain {
            path,
            source: CoreError::InvalidVehicleProfileValue { field, .. },
        } if path == "vehicleProfiles[0]" && field == "length"
    );
}

#[test]
fn unsupported_profile_model_reports_exact_field_path() {
    let mut value = v0_3_value();
    value["vehicleProfiles"][0]["model"] = json!("eidm");

    let error = load_value(value).expect_err("unsupported model must fail");
    std::assert_matches!(
        error,
        DataError::UnsupportedVehicleProfileModel {
            path,
            profile_id,
            actual,
        } if path == "vehicleProfiles[0].model"
            && profile_id == "passenger-car"
            && actual == "eidm"
    );
}

#[test]
fn duplicate_profile_id_is_rejected_by_core_registry() {
    let mut value = v0_3_value();
    let duplicate = value["vehicleProfiles"][0].clone();
    value["vehicleProfiles"]
        .as_array_mut()
        .expect("profile list must be array")
        .push(duplicate);

    let error = load_value(value).expect_err("duplicate profile id must fail");
    std::assert_matches!(
        error,
        DataError::CoreDomain {
            path,
            source: CoreError::DuplicateVehicleProfileId { profile_id },
        } if path == "vehicleProfiles" && profile_id == "passenger-car"
    );
}

#[test]
fn route_domain_error_is_returned_by_initial_traffic_data() {
    let mut value = v0_3_value();
    value["routes"][0]["edges"] = json!(["entry", "missing"]);

    let error = load_value(value).expect_err("unknown route edge must fail");
    std::assert_matches!(
        error,
        DataError::CoreDomain {
            path,
            source: CoreError::UnknownRouteEdge { route_id, edge_id },
        } if path == "initialTrafficData"
            && route_id == "main-route"
            && edge_id == "missing"
    );
}

#[test]
fn data_error_is_send_and_sync() {
    fn assert_traits<T: std::error::Error + Send + Sync>() {}
    assert_traits::<DataError>();
}

fn load_value(value: Value) -> Result<LoadedPackage, DataError> {
    from_json_str(&serde_json::to_string(&value).expect("test JSON must serialize"))
}

fn v0_3_value() -> Value {
    serde_json::from_str(V0_3_FIXTURE).expect("v0.3 fixture must be valid JSON")
}
