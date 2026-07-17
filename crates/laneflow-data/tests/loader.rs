use laneflow_core::{
    CoreError, MAX_PORTABLE_SIGNAL_TIME_MS, MovementGateKey, SignalAspect, SignalControl,
};
use laneflow_data::{CURRENT_FORMAT_VERSION, DataError, LoadedPackage, from_json_str};
use serde_json::{Value, json};

const SIGNALS_FIXTURE: &str =
    include_str!("../../../examples/data/v0.5-parking-signals-baseline.laneflow.json");
const EMPTY_SIGNALS_FIXTURE: &str =
    include_str!("../../../examples/data/v0.5-empty-signals-and-parking.laneflow.json");

#[test]
fn current_loader_normalizes_static_signals_parking_and_resolvers() {
    assert_eq!(CURRENT_FORMAT_VERSION, "0.5");
    let loaded = from_json_str(SIGNALS_FIXTURE).expect("v0.5 fixture must load");
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

    let parking = traffic.parking();
    assert_eq!(
        parking.areas().map(|area| area.id()).collect::<Vec<_>>(),
        ["lot-main"]
    );
    assert_eq!(
        parking.spaces().map(|space| space.id()).collect::<Vec<_>>(),
        ["lot-main-01", "lot-main-02", "curbside-01"]
    );
    let lot = parking.area_handle("lot-main").expect("ParkingArea handle");
    assert_eq!(
        parking
            .area_spaces(lot)
            .expect("known area")
            .iter()
            .map(|space| parking.space_external_id(*space).expect("known space"))
            .collect::<Vec<_>>(),
        ["lot-main-01", "lot-main-02"]
    );
    let curbside = parking
        .space_handle("curbside-01")
        .expect("standalone curbside space");
    assert_eq!(parking.space_area(curbside), Some(None));
    let curbside_entry = parking.space_entry(curbside).expect("entry anchor");
    assert_eq!(
        traffic.lane_graph().edge_external_id(curbside_entry.edge()),
        Some("bypass")
    );
    assert_eq!(curbside_entry.progress(), 8.0);
}

#[test]
fn explicit_empty_signals_and_parking_is_valid_current_v0_5() {
    let loaded = from_json_str(EMPTY_SIGNALS_FIXTURE).expect("empty Signals fixture must load");
    assert!(loaded.initial_traffic_data().signals().is_empty());
    assert!(loaded.initial_traffic_data().parking().is_empty());
    assert_eq!(loaded.initial_traffic_data().vehicle_profiles().len(), 1);
    assert_eq!(loaded.initial_traffic_data().routes().len(), 2);
}

#[test]
fn unsupported_versions_are_rejected_before_current_shape_and_units() {
    for version in ["0.4", "0.6"] {
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
            DataError::UnsupportedFormatVersion { expected: "0.5", actual }
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
fn current_v0_5_requires_signals_parking_and_all_nested_arrays() {
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

    let mut missing_parking = empty_value();
    missing_parking
        .as_object_mut()
        .expect("root object")
        .remove("parking");
    std::assert_matches!(
        load_value(missing_parking).expect_err("parking is required"),
        DataError::JsonShape { .. }
    );

    for field in ["areas", "spaces"] {
        let mut value = empty_value();
        value["parking"]
            .as_object_mut()
            .expect("parking object")
            .remove(field);
        let error = load_value(value).expect_err("every Parking array is required");
        std::assert_matches!(error, DataError::JsonShape { path, .. } if path.contains("parking"));
    }
}

#[test]
fn parking_area_id_is_omitted_only_and_all_shapes_are_closed() {
    let baseline = signals_value();
    assert!(
        baseline["parking"]["spaces"][2].get("areaId").is_none(),
        "canonical standalone space must omit areaId"
    );
    load_value(baseline).expect("omitted areaId must load");

    let mut explicit_null = signals_value();
    explicit_null["parking"]["spaces"][2]["areaId"] = Value::Null;
    std::assert_matches!(
        load_value(explicit_null).expect_err("explicit null areaId must fail"),
        DataError::JsonShape { path, .. } if path.contains("parking.spaces[2].areaId")
    );

    for target in ["parking", "area", "space", "entry", "geometry"] {
        let mut value = signals_value();
        match target {
            "parking" => value["parking"]["typo"] = json!(true),
            "area" => value["parking"]["areas"][0]["typo"] = json!(true),
            "space" => value["parking"]["spaces"][0]["typo"] = json!(true),
            "entry" => value["parking"]["spaces"][0]["entry"]["typo"] = json!(true),
            "geometry" => value["parking"]["spaces"][0]["geometry"]["typo"] = json!(true),
            _ => unreachable!(),
        }
        std::assert_matches!(
            load_value(value).expect_err("Parking shapes must reject unknown fields"),
            DataError::JsonShape { path, .. } if path.contains("parking")
        );
    }
}

#[test]
fn parking_domain_errors_use_narrowest_paths() {
    for expected_path in [
        "parking.areas[0].id",
        "parking.spaces[0].id",
        "parking.spaces[0].areaId",
        "parking.spaces[0].entry.edgeId",
        "parking.spaces[0].exit.edgeId",
    ] {
        let mut value = signals_value();
        match expected_path {
            "parking.areas[0].id" => value["parking"]["areas"][0]["id"] = json!("bad id"),
            "parking.spaces[0].id" => value["parking"]["spaces"][0]["id"] = json!("bad id"),
            "parking.spaces[0].areaId" => value["parking"]["spaces"][0]["areaId"] = json!("bad id"),
            "parking.spaces[0].entry.edgeId" => {
                value["parking"]["spaces"][0]["entry"]["edgeId"] = json!("bad id")
            }
            "parking.spaces[0].exit.edgeId" => {
                value["parking"]["spaces"][0]["exit"]["edgeId"] = json!("bad id")
            }
            _ => unreachable!(),
        }
        std::assert_matches!(
            load_value(value).expect_err("invalid Parking external ID"),
            DataError::CoreDomain {
                path,
                source: CoreError::InvalidExternalId { .. },
            } if path == expected_path
        );
    }

    let mut duplicate_area = signals_value();
    let duplicate = duplicate_area["parking"]["areas"][0].clone();
    duplicate_area["parking"]["areas"]
        .as_array_mut()
        .expect("areas")
        .push(duplicate);
    std::assert_matches!(
        load_value(duplicate_area).expect_err("duplicate area"),
        DataError::CoreDomain {
            path,
            source: CoreError::DuplicateParkingAreaId { area_id },
        } if path == "parking.areas[1].id" && area_id == "lot-main"
    );

    let mut duplicate_space = signals_value();
    let duplicate = duplicate_space["parking"]["spaces"][0].clone();
    duplicate_space["parking"]["spaces"]
        .as_array_mut()
        .expect("spaces")
        .push(duplicate);
    std::assert_matches!(
        load_value(duplicate_space).expect_err("duplicate space"),
        DataError::CoreDomain {
            path,
            source: CoreError::DuplicateParkingSpaceId { space_id },
        } if path == "parking.spaces[3].id" && space_id == "lot-main-01"
    );

    let mut unknown_area = signals_value();
    unknown_area["parking"]["spaces"][0]["areaId"] = json!("missing");
    std::assert_matches!(
        load_value(unknown_area).expect_err("unknown area"),
        DataError::CoreDomain {
            path,
            source: CoreError::UnknownParkingSpaceArea { area_id, .. },
        } if path == "parking.spaces[0].areaId" && area_id == "missing"
    );

    let mut unknown_entry = signals_value();
    unknown_entry["parking"]["spaces"][0]["entry"]["edgeId"] = json!("missing");
    std::assert_matches!(
        load_value(unknown_entry).expect_err("unknown entry edge"),
        DataError::CoreDomain {
            path,
            source: CoreError::UnknownParkingAnchorEdge { .. },
        } if path == "parking.spaces[0].entry.edgeId"
    );

    let mut invalid_exit_progress = signals_value();
    invalid_exit_progress["parking"]["spaces"][0]["exit"]["progress"] = json!(40.0);
    std::assert_matches!(
        load_value(invalid_exit_progress).expect_err("exit endpoint is invalid"),
        DataError::CoreDomain {
            path,
            source: CoreError::ParkingAnchorProgressOutOfRange { .. },
        } if path == "parking.spaces[0].exit.progress"
    );

    let mut invalid_geometry = signals_value();
    invalid_geometry["parking"]["spaces"][0]["geometry"]["headingOffsetRadians"] =
        json!(std::f64::consts::PI);
    std::assert_matches!(
        load_value(invalid_geometry).expect_err("non-canonical heading"),
        DataError::CoreDomain {
            path,
            source: CoreError::InvalidParkingGeometryValue { field, .. },
        } if path == "parking.spaces[0].geometry.headingOffsetRadians"
            && field == "headingOffsetRadians"
    );

    let mut orphan = signals_value();
    orphan["parking"]["spaces"][0]
        .as_object_mut()
        .expect("space")
        .remove("areaId");
    orphan["parking"]["spaces"][1]
        .as_object_mut()
        .expect("space")
        .remove("areaId");
    std::assert_matches!(
        load_value(orphan).expect_err("orphan area"),
        DataError::CoreDomain {
            path,
            source: CoreError::OrphanParkingArea { area_id },
        } if path == "parking.areas[0]" && area_id == "lot-main"
    );
}

#[test]
fn normalization_priority_is_signals_then_parking_then_routes() {
    let mut value = signals_value();
    value["signals"]["controllers"][0]["groupIds"][0] = json!("missing-group");
    value["parking"]["spaces"][0]["areaId"] = json!("missing-area");
    value["routes"][0]["edgeIds"][1] = json!("missing-edge");
    std::assert_matches!(
        load_value(value.clone()).expect_err("Signals must fail first"),
        DataError::CoreDomain {
            source: CoreError::UnknownSignalControllerGroup { .. },
            ..
        }
    );

    value["signals"]["controllers"][0]["groupIds"][0] = json!("main");
    std::assert_matches!(
        load_value(value.clone()).expect_err("Parking must fail before routes"),
        DataError::CoreDomain {
            source: CoreError::UnknownParkingSpaceArea { .. },
            ..
        }
    );

    value["parking"]["spaces"][0]["areaId"] = json!("lot-main");
    std::assert_matches!(
        load_value(value).expect_err("route must fail after static registries"),
        DataError::CoreDomain {
            source: CoreError::UnknownRouteEdge { .. },
            ..
        }
    );
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
