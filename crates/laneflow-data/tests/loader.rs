use laneflow_core::{
    CoreError, MAX_PORTABLE_SIGNAL_TIME_MS, MovementGateKey, SignalAspect, SignalControl,
};
use laneflow_data::{CURRENT_FORMAT_VERSION, DataError, LoadedPackage, from_json_str};
use serde_json::{Value, json};

const SIGNALS_FIXTURE: &str =
    include_str!("../../../examples/data/v0.4-signals-baseline.laneflow.json");
const EMPTY_SIGNALS_FIXTURE: &str =
    include_str!("../../../examples/data/v0.4-empty-signals.laneflow.json");

#[test]
fn current_loader_normalizes_static_signals_and_resolvers() {
    assert_eq!(CURRENT_FORMAT_VERSION, "0.4");
    let loaded = from_json_str(SIGNALS_FIXTURE).expect("v0.4 Signals fixture must load");
    let traffic = loaded.initial_traffic_data();
    let signals = traffic.signals();

    let stop_line = signals
        .stop_line_handle("stop-entry")
        .expect("StopLine handle must resolve");
    let group = signals
        .group_handle("main")
        .expect("SignalGroup handle must resolve");
    let controller = signals
        .controller_handle("controller-main")
        .expect("SignalController handle must resolve");
    assert_eq!(signals.stop_line_external_id(stop_line), Some("stop-entry"));
    assert_eq!(signals.group_external_id(group), Some("main"));
    assert_eq!(signals.group_controller(group), Some(controller));
    assert_eq!(
        signals.controller_cycle_duration_ms(controller),
        Some(53_000)
    );
    let yellow = signals
        .phase_ref(controller, "yellow")
        .expect("yellow phase must resolve");
    assert_eq!(
        signals.phase_aspects(yellow),
        Some([SignalAspect::Yellow].as_slice())
    );
    assert_eq!(signals.phase_end_offset_ms(yellow), Some(33_000));

    let entry = traffic
        .lane_graph()
        .edge_handle("entry")
        .expect("entry edge");
    let through = traffic
        .lane_graph()
        .edge_handle("through")
        .expect("through edge");
    let bypass = traffic
        .lane_graph()
        .edge_handle("bypass")
        .expect("bypass edge");
    assert_eq!(
        signals.movement_gate_control(MovementGateKey::new(entry, through)),
        Some(SignalControl::Group(group))
    );
    assert_eq!(
        signals.movement_gate_control(MovementGateKey::new(entry, bypass)),
        Some(SignalControl::None)
    );
}

#[test]
fn explicit_empty_signals_is_valid_current_v0_4() {
    let loaded = from_json_str(EMPTY_SIGNALS_FIXTURE).expect("empty Signals fixture must load");
    assert!(loaded.initial_traffic_data().signals().is_empty());
    assert_eq!(loaded.initial_traffic_data().vehicle_profiles().len(), 1);
    assert_eq!(loaded.initial_traffic_data().routes().len(), 2);
}

#[test]
fn unsupported_versions_are_rejected_before_current_shape_and_units() {
    for version in ["0.3", "0.5"] {
        let mut value = empty_value();
        value["formatVersion"] = json!(version);
        value["units"]["distance"] = json!("kilometer");
        value
            .as_object_mut()
            .expect("root object")
            .remove("signals");
        value["futureTopLevelField"] = json!({ "newShape": true });

        let error = load_value(value).expect_err("unsupported version must fail first");
        std::assert_matches!(
            error,
            DataError::UnsupportedFormatVersion { expected: "0.4", actual }
                if actual == version
        );
    }
}

#[test]
fn malformed_or_trailing_json_fails_before_version_dispatch() {
    for source in [
        r#"{"formatVersion":"0.3","#.to_owned(),
        format!("{EMPTY_SIGNALS_FIXTURE} true"),
    ] {
        std::assert_matches!(
            from_json_str(&source).expect_err("invalid JSON syntax must fail first"),
            DataError::JsonSyntax { line, column, .. } if line > 0 && column > 0
        );
    }
}

#[test]
fn current_v0_4_requires_signals_and_all_four_arrays() {
    let mut missing_signals = empty_value();
    missing_signals
        .as_object_mut()
        .expect("root object")
        .remove("signals");
    std::assert_matches!(
        load_value(missing_signals).expect_err("signals is required"),
        DataError::JsonShape { .. }
    );

    for field in ["stopLines", "movementGates", "groups", "controllers"] {
        let mut value = empty_value();
        value["signals"]
            .as_object_mut()
            .expect("signals object")
            .remove(field);
        let error = load_value(value).expect_err("every Signals array is required");
        std::assert_matches!(error, DataError::JsonShape { path, .. } if path.contains("signals"));
    }
}

#[test]
fn legacy_reference_fields_and_json_ld_are_rejected() {
    let mut value = empty_value();
    value["laneGraph"]["edges"][0]["connections"][0] = json!({ "to": "exit" });
    let error = load_value(value).expect_err("legacy connection.to must fail");
    std::assert_matches!(error, DataError::JsonShape { path, .. } if path.contains("connections[0]"));

    let mut value = empty_value();
    value["routes"][0] = json!({ "id": "main-route", "edges": ["entry", "exit"] });
    let error = load_value(value).expect_err("legacy route.edges must fail");
    std::assert_matches!(error, DataError::JsonShape { path, .. } if path.contains("routes[0]"));

    let mut value = empty_value();
    value["@context"] = json!({ "@vocab": "https://example.invalid/" });
    std::assert_matches!(
        load_value(value).expect_err("JSON-LD is not current canonical JSON"),
        DataError::JsonShape { .. }
    );

    let mut value = empty_value();
    value["extensions"] = Value::Null;
    std::assert_matches!(
        load_value(value).expect_err("extensions must remain an object when present"),
        DataError::JsonShape { path, .. } if path.contains("extensions")
    );
}

#[test]
fn signal_control_is_a_closed_tagged_union() {
    let mut value = signals_value();
    value["signals"]["movementGates"][1]["signalControl"]["groupId"] = json!("main");
    std::assert_matches!(
        load_value(value).expect_err("none control cannot carry groupId"),
        DataError::JsonShape { path, .. } if path.contains("signalControl")
    );

    let mut value = signals_value();
    value["signals"]["movementGates"][0]["signalControl"] = json!({ "kind": "group" });
    std::assert_matches!(
        load_value(value).expect_err("group control requires groupId"),
        DataError::JsonShape { path, .. } if path.contains("signalControl")
    );

    let mut value = signals_value();
    value["signals"]["movementGates"][0]["signalControl"] = json!({ "kind": "free" });
    std::assert_matches!(
        load_value(value).expect_err("unknown control kind must fail"),
        DataError::JsonShape { path, .. } if path.contains("signalControl")
    );
}

#[test]
fn signal_location_controller_kind_and_aspect_are_closed_enums() {
    for (path, value) in [
        ("signals.stopLines[0].location", json!("midEdge")),
        ("signals.controllers[0].kind", json!("actuated")),
        (
            "signals.controllers[0].phases[0].states[0].aspect",
            json!("blue"),
        ),
    ] {
        let mut package = signals_value();
        match path {
            "signals.stopLines[0].location" => {
                package["signals"]["stopLines"][0]["location"] = value;
            }
            "signals.controllers[0].kind" => {
                package["signals"]["controllers"][0]["kind"] = value;
            }
            _ => {
                package["signals"]["controllers"][0]["phases"][0]["states"][0]["aspect"] = value;
            }
        }
        std::assert_matches!(
            load_value(package).expect_err("closed signal enum must reject unknown value"),
            DataError::JsonShape { path: actual, .. } if actual.contains(path)
        );
    }
}

#[test]
fn portable_integer_timing_is_enforced_by_shape_and_core() {
    let mut value = signals_value();
    value["signals"]["controllers"][0]["offsetMs"] = json!(-1);
    std::assert_matches!(
        load_value(value).expect_err("negative offset is shape-invalid"),
        DataError::JsonShape { path, .. } if path.contains("offsetMs")
    );

    let mut value = signals_value();
    value["signals"]["controllers"][0]["phases"][0]["durationMs"] = json!(1.5);
    std::assert_matches!(
        load_value(value).expect_err("fractional duration is shape-invalid"),
        DataError::JsonShape { path, .. } if path.contains("durationMs")
    );

    let mut value = signals_value();
    value["signals"]["controllers"][0]["offsetMs"] = json!(MAX_PORTABLE_SIGNAL_TIME_MS + 1);
    let error = load_value(value).expect_err("Core owns portable scheduling invariant");
    std::assert_matches!(
        error,
        DataError::CoreDomain {
            path,
            source: CoreError::InvalidSignalControllerOffset { .. },
        } if path == "signals.controllers[0].offsetMs"
    );

    for duration_ms in [0, MAX_PORTABLE_SIGNAL_TIME_MS + 1] {
        let mut value = signals_value();
        value["signals"]["controllers"][0]["phases"][0]["durationMs"] = json!(duration_ms);
        let error = load_value(value).expect_err("duration outside portable range");
        std::assert_matches!(
            error,
            DataError::CoreDomain {
                path,
                source: CoreError::InvalidSignalPhaseDuration { duration_ms: actual, .. },
            } if path == "signals.controllers[0].phases[0].durationMs"
                && actual == duration_ms
        );
    }

    for (duration_ms, offset_ms) in [
        (1, 0),
        (MAX_PORTABLE_SIGNAL_TIME_MS, MAX_PORTABLE_SIGNAL_TIME_MS - 1),
    ] {
        let mut value = signals_value();
        value["signals"]["controllers"][0]["offsetMs"] = json!(offset_ms);
        value["signals"]["controllers"][0]["phases"] = json!([{
            "id": "only",
            "durationMs": duration_ms,
            "states": [{ "groupId": "main", "aspect": "green" }]
        }]);
        load_value(value).expect("portable min/max timing boundary must load");
    }
}

#[test]
fn phase_state_errors_preserve_exact_path_and_core_source() {
    let mut value = signals_value();
    let duplicate = value["signals"]["controllers"][0]["phases"][0]["states"][0].clone();
    value["signals"]["controllers"][0]["phases"][0]["states"]
        .as_array_mut()
        .expect("states array")
        .push(duplicate);
    let error = load_value(value).expect_err("duplicate phase group must fail");
    std::assert_matches!(
        error,
        DataError::CoreDomain {
            path,
            source: CoreError::DuplicateSignalPhaseGroup { group_id, .. },
        } if path == "signals.controllers[0].phases[0].states[1].groupId"
            && group_id == "main"
    );

    let mut value = signals_value();
    value["signals"]["controllers"][0]["phases"][0]["states"] = json!([]);
    let error = load_value(value).expect_err("missing phase group must fail");
    std::assert_matches!(
        error,
        DataError::CoreDomain {
            path,
            source: CoreError::MissingSignalPhaseGroup { group_id, .. },
        } if path == "signals.controllers[0].phases[0].states" && group_id == "main"
    );
}

#[test]
fn domain_errors_use_the_narrowest_available_id_path() {
    let mut value = signals_value();
    value["signals"]["controllers"][0]["groupIds"][0] = json!("unknown");
    std::assert_matches!(
        load_value(value).expect_err("unknown controller group must fail"),
        DataError::CoreDomain {
            path,
            source: CoreError::UnknownSignalControllerGroup { group_id, .. },
        } if path == "signals.controllers[0].groupIds[0]" && group_id == "unknown"
    );

    let mut value = signals_value();
    let duplicate = value["signals"]["controllers"][0]["phases"][0].clone();
    value["signals"]["controllers"][0]["phases"]
        .as_array_mut()
        .expect("phase array")
        .push(duplicate);
    std::assert_matches!(
        load_value(value).expect_err("duplicate phase ID must fail"),
        DataError::CoreDomain {
            path,
            source: CoreError::DuplicateSignalPhaseId { phase_id, .. },
        } if path == "signals.controllers[0].phases[3].id" && phase_id == "green"
    );

    let mut value = signals_value();
    value["signals"]["controllers"][0]["phases"][0]["id"] = json!("bad id");
    std::assert_matches!(
        load_value(value).expect_err("invalid phase ID must fail"),
        DataError::CoreDomain {
            path,
            source: CoreError::InvalidExternalId { external_id, .. },
        } if path == "signals.controllers[0].phases[0].id" && external_id == "bad id"
    );

    let mut value = signals_value();
    value["signals"]["movementGates"][0]["signalControl"]["groupId"] = json!("unknown");
    std::assert_matches!(
        load_value(value).expect_err("unknown Gate group must fail"),
        DataError::CoreDomain {
            path,
            source: CoreError::UnknownMovementGateSignalGroup { group_id },
        } if path == "signals.movementGates[0].signalControl.groupId" && group_id == "unknown"
    );

    let mut value = empty_value();
    value["routes"][0]["edgeIds"][1] = json!("missing");
    std::assert_matches!(
        load_value(value).expect_err("unknown route edge must fail"),
        DataError::CoreDomain {
            path,
            source: CoreError::UnknownRouteEdge { edge_id, .. },
        } if path == "routes[0].edgeIds[1]" && edge_id == "missing"
    );
}

#[test]
fn global_coverage_and_route_final_stop_line_errors_are_structured() {
    let mut value = signals_value();
    value["signals"]["movementGates"]
        .as_array_mut()
        .expect("Gate array")
        .pop();
    let error = load_value(value).expect_err("missing Gate coverage must fail");
    std::assert_matches!(
        error,
        DataError::CoreDomain {
            path,
            source: CoreError::MissingMovementGateCoverage { to_edge_id, .. },
        } if path == "signals.stopLines[0]" && to_edge_id == "bypass"
    );

    let mut value = signals_value();
    value["routes"][0]["edgeIds"] = json!(["entry"]);
    let error = load_value(value).expect_err("route cannot terminate at StopLine");
    std::assert_matches!(
        error,
        DataError::CoreDomain {
            path,
            source: CoreError::RouteTerminatesAtStopLine { route_id, .. },
        } if path == "routes[0].edgeIds[0]" && route_id == "controlled-route"
    );
}

#[test]
fn invalid_units_profile_and_shape_errors_remain_structured() {
    let mut value = empty_value();
    value["units"]["distance"] = json!("kilometer");
    std::assert_matches!(
        load_value(value).expect_err("distance unit must be meter"),
        DataError::InvalidUnit {
            path: "units.distance",
            expected: "meter",
            actual,
        } if actual == "kilometer"
    );

    let mut value = empty_value();
    value["vehicleProfiles"][0]["length"] = json!(0.0);
    std::assert_matches!(
        load_value(value).expect_err("invalid profile length"),
        DataError::CoreDomain {
            path,
            source: CoreError::InvalidVehicleProfileValue { field, .. },
        } if path == "vehicleProfiles[0]" && field == "length"
    );

    let mut value = empty_value();
    value["vehicleProfiles"][0]["typo"] = json!(true);
    std::assert_matches!(
        load_value(value).expect_err("unknown profile field"),
        DataError::JsonShape { path, line, column, .. }
            if path.contains("vehicleProfiles[0]") && line > 0 && column > 0
    );
}

#[test]
fn missing_or_null_format_version_is_a_shape_error() {
    let mut missing = empty_value();
    missing
        .as_object_mut()
        .expect("root object")
        .remove("formatVersion");
    std::assert_matches!(
        load_value(missing).expect_err("formatVersion required"),
        DataError::JsonShape { path, .. } if path == "$"
    );

    let mut null = empty_value();
    null["formatVersion"] = Value::Null;
    std::assert_matches!(
        load_value(null).expect_err("null formatVersion invalid"),
        DataError::JsonShape { .. }
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

fn signals_value() -> Value {
    serde_json::from_str(SIGNALS_FIXTURE).expect("Signals fixture JSON")
}

fn empty_value() -> Value {
    serde_json::from_str(EMPTY_SIGNALS_FIXTURE).expect("empty Signals fixture JSON")
}
