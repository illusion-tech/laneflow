use laneflow_core::{CoreError, MAX_PORTABLE_SIGNAL_TIME_MS, SignalAspect, SignalControl};
use laneflow_data::{CURRENT_FORMAT_VERSION, DataError, LoadedPackage, from_json_str};
use serde_json::{Value, json};

const SIGNALS_FIXTURE: &str =
    include_str!("../../../examples/data/v0.9-parking-signals-baseline.laneflow.json");
const EMPTY_SIGNALS_FIXTURE: &str =
    include_str!("../../../examples/data/v0.9-empty-signals-and-parking.laneflow.json");

fn into_core_domain(error: DataError) -> (String, CoreError) {
    match error {
        DataError::CoreDomain { path, source } => (path, *source),
        other => panic!("expected CoreDomain error, got {other:?}"),
    }
}

#[test]
fn current_loader_normalizes_static_signals_parking_and_resolvers() {
    assert_eq!(CURRENT_FORMAT_VERSION, "0.9");
    let loaded = from_json_str(SIGNALS_FIXTURE).expect("v0.9 fixture must load");
    let traffic = loaded.initial_traffic_data();
    let signals = traffic.signals();
    let topology = traffic.junctions();

    let motor_vehicle = traffic
        .participant_classes()
        .class_handle("motorVehicle")
        .expect("ParticipantClass handle must resolve");
    assert_eq!(
        traffic
            .participant_classes()
            .class_external_id(motor_vehicle),
        Some("motorVehicle")
    );
    let cross_section = traffic.cross_section();
    assert!(
        cross_section
            .section_handle("section-main")
            .is_some_and(
                |section| cross_section.section_external_id(section) == Some("section-main")
            )
    );
    assert!(cross_section.corridor_handle("corridor-main").is_some());
    assert!(traffic.access().is_empty());

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
    assert_eq!(
        traffic
            .lane_graph()
            .edge_speed_limit(entry)
            .expect("entry speed limit")
            .value(),
        30.0
    );
    assert_eq!(
        signals.maneuver_gate_control(
            signals
                .maneuver_gate_handle("gate-controlled")
                .expect("controlled Gate"),
        ),
        Some(SignalControl::Group(group))
    );
    assert_eq!(
        signals.maneuver_gate_control(
            signals
                .maneuver_gate_handle("gate-uncontrolled")
                .expect("uncontrolled Gate"),
        ),
        Some(SignalControl::None)
    );
    let junction = topology
        .junction_handle("junction-main")
        .expect("Junction handle");
    let controlled = topology
        .movement_handle("movement-controlled")
        .expect("Movement handle");
    let controlled_path = topology
        .maneuver_path_handle("path-controlled")
        .expect("ManeuverPath handle");
    assert_eq!(topology.movement_junction(controlled), Some(junction));
    assert_eq!(
        topology.maneuver_path_movement(controlled_path),
        Some(controlled)
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
fn explicit_empty_static_domains_are_valid_current_v0_9() {
    let loaded = from_json_str(EMPTY_SIGNALS_FIXTURE).expect("empty Signals fixture must load");
    assert!(loaded.initial_traffic_data().junctions().is_empty());
    assert!(loaded.initial_traffic_data().signals().is_empty());
    assert!(loaded.initial_traffic_data().parking().is_empty());
    assert_eq!(loaded.initial_traffic_data().vehicle_profiles().len(), 1);
    assert_eq!(loaded.initial_traffic_data().routes().len(), 2);
}

#[test]
fn unsupported_versions_are_rejected_before_current_shape_and_units() {
    for version in ["0.4", "0.5", "0.6", "0.7", "0.8", "1.0"] {
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
            DataError::UnsupportedFormatVersion { expected: "0.9", actual }
                if actual == version
        );
    }
}

#[test]
fn speed_limit_is_required_closed_and_uses_the_narrowest_domain_path() {
    let mut missing = empty_value();
    missing["laneGraph"]["edges"][0]
        .as_object_mut()
        .expect("edge")
        .remove("speedLimit");
    std::assert_matches!(
        load_value(missing).expect_err("speedLimit is required"),
        DataError::JsonShape { path, .. } if path.contains("laneGraph.edges[0]")
    );

    for invalid in [0.0, -1.0] {
        let mut value = empty_value();
        value["laneGraph"]["edges"][0]["speedLimit"] = json!(invalid);
        std::assert_matches!(
            into_core_domain(load_value(value).expect_err("invalid speedLimit")),
            (path, CoreError::InvalidSpeedLimit { speed_limit })
                if path == "laneGraph.edges[0].speedLimit" && speed_limit == invalid
        );
    }

    let mut unknown = empty_value();
    unknown["laneGraph"]["edges"][0]["speedLimitMph"] = json!(60);
    std::assert_matches!(
        load_value(unknown).expect_err("unknown edge field"),
        DataError::JsonShape { path, .. } if path.contains("laneGraph.edges[0]")
    );
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
fn current_v0_9_requires_all_static_domains_and_nested_arrays() {
    for field in [
        "junctions",
        "movements",
        "maneuverPaths",
        "participantClasses",
        "facilityBands",
        "roadSections",
        "laneGroups",
        "roadCorridors",
        "accessRules",
    ] {
        let mut value = empty_value();
        value.as_object_mut().expect("root object").remove(field);
        let error = load_value(value).expect_err("every topology array is required");
        std::assert_matches!(error, DataError::JsonShape { path, .. } if path == "$");
    }

    let mut missing_signals = empty_value();
    missing_signals
        .as_object_mut()
        .expect("root object")
        .remove("signals");
    std::assert_matches!(
        load_value(missing_signals).expect_err("signals is required"),
        DataError::JsonShape { .. }
    );

    for field in ["stopLines", "maneuverGates", "groups", "controllers"] {
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
            into_core_domain(load_value(value).expect_err("invalid Parking external ID")),
            (path, CoreError::InvalidExternalId { .. }) if path == expected_path
        );
    }

    let mut duplicate_area = signals_value();
    let duplicate = duplicate_area["parking"]["areas"][0].clone();
    duplicate_area["parking"]["areas"]
        .as_array_mut()
        .expect("areas")
        .push(duplicate);
    std::assert_matches!(
        into_core_domain(load_value(duplicate_area).expect_err("duplicate area")),
        (path, CoreError::DuplicateParkingAreaId { area_id })
            if path == "parking.areas[1].id" && area_id == "lot-main"
    );

    let mut duplicate_space = signals_value();
    let duplicate = duplicate_space["parking"]["spaces"][0].clone();
    duplicate_space["parking"]["spaces"]
        .as_array_mut()
        .expect("spaces")
        .push(duplicate);
    std::assert_matches!(
        into_core_domain(load_value(duplicate_space).expect_err("duplicate space")),
        (path, CoreError::DuplicateParkingSpaceId { space_id })
            if path == "parking.spaces[3].id" && space_id == "lot-main-01"
    );

    let mut unknown_area = signals_value();
    unknown_area["parking"]["spaces"][0]["areaId"] = json!("missing");
    std::assert_matches!(
        into_core_domain(load_value(unknown_area).expect_err("unknown area")),
        (path, CoreError::UnknownParkingSpaceArea { area_id, .. })
            if path == "parking.spaces[0].areaId" && area_id == "missing"
    );

    let mut unknown_entry = signals_value();
    unknown_entry["parking"]["spaces"][0]["entry"]["edgeId"] = json!("missing");
    std::assert_matches!(
        into_core_domain(load_value(unknown_entry).expect_err("unknown entry edge")),
        (path, CoreError::UnknownParkingAnchorEdge { .. })
            if path == "parking.spaces[0].entry.edgeId"
    );

    let mut invalid_exit_progress = signals_value();
    invalid_exit_progress["parking"]["spaces"][0]["exit"]["progress"] = json!(40.0);
    std::assert_matches!(
        into_core_domain(
            load_value(invalid_exit_progress).expect_err("exit endpoint is invalid")
        ),
        (path, CoreError::ParkingAnchorProgressOutOfRange { .. })
            if path == "parking.spaces[0].exit.progress"
    );

    let mut invalid_geometry = signals_value();
    invalid_geometry["parking"]["spaces"][0]["geometry"]["headingOffsetRadians"] =
        json!(std::f64::consts::PI);
    std::assert_matches!(
        into_core_domain(load_value(invalid_geometry).expect_err("non-canonical heading")),
        (path, CoreError::InvalidParkingGeometryValue { field, .. })
            if path == "parking.spaces[0].geometry.headingOffsetRadians"
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
        into_core_domain(load_value(orphan).expect_err("orphan area")),
        (path, CoreError::OrphanParkingArea { area_id })
            if path == "parking.areas[0]" && area_id == "lot-main"
    );
}

#[test]
fn normalization_priority_is_signals_then_parking_then_routes() {
    let mut value = signals_value();
    value["signals"]["controllers"][0]["groupIds"][0] = json!("missing-group");
    value["parking"]["spaces"][0]["areaId"] = json!("missing-area");
    value["routes"][0]["edgeIds"][1] = json!("missing-edge");
    std::assert_matches!(
        into_core_domain(load_value(value.clone()).expect_err("Signals must fail first")),
        (_, CoreError::UnknownSignalControllerGroup { .. })
    );

    value["signals"]["controllers"][0]["groupIds"][0] = json!("main");
    std::assert_matches!(
        into_core_domain(load_value(value.clone()).expect_err("Parking must fail before routes")),
        (_, CoreError::UnknownParkingSpaceArea { .. })
    );

    value["parking"]["spaces"][0]["areaId"] = json!("lot-main");
    std::assert_matches!(
        into_core_domain(load_value(value).expect_err("route must fail after static registries")),
        (_, CoreError::UnknownRouteEdge { .. })
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
    value["signals"]["maneuverGates"][1]["signalControl"]["groupId"] = json!("main");
    std::assert_matches!(
        load_value(value).expect_err("none control cannot carry groupId"),
        DataError::JsonShape { path, .. } if path.contains("signalControl")
    );

    let mut value = signals_value();
    value["signals"]["maneuverGates"][0]["signalControl"] = json!({ "kind": "group" });
    std::assert_matches!(
        load_value(value).expect_err("group control requires groupId"),
        DataError::JsonShape { path, .. } if path.contains("signalControl")
    );

    let mut value = signals_value();
    value["signals"]["maneuverGates"][0]["signalControl"] = json!({ "kind": "free" });
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
        into_core_domain(error),
        (path, CoreError::InvalidSignalControllerOffset { .. })
            if path == "signals.controllers[0].offsetMs"
    );

    for duration_ms in [0, MAX_PORTABLE_SIGNAL_TIME_MS + 1] {
        let mut value = signals_value();
        value["signals"]["controllers"][0]["phases"][0]["durationMs"] = json!(duration_ms);
        let error = load_value(value).expect_err("duration outside portable range");
        std::assert_matches!(
            into_core_domain(error),
            (path, CoreError::InvalidSignalPhaseDuration { duration_ms: actual, .. })
                if path == "signals.controllers[0].phases[0].durationMs"
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
        into_core_domain(error),
        (path, CoreError::DuplicateSignalPhaseGroup { group_id, .. })
            if path == "signals.controllers[0].phases[0].states[1].groupId"
            && group_id == "main"
    );

    let mut value = signals_value();
    value["signals"]["controllers"][0]["phases"][0]["states"] = json!([]);
    let error = load_value(value).expect_err("missing phase group must fail");
    std::assert_matches!(
        into_core_domain(error),
        (path, CoreError::MissingSignalPhaseGroup { group_id, .. })
            if path == "signals.controllers[0].phases[0].states" && group_id == "main"
    );
}

#[test]
fn domain_errors_use_the_narrowest_available_id_path() {
    for (expected_path, core_field) in [
        (
            "maneuverPaths[0].entryEdgeId",
            "maneuverPaths[].entryEdgeId",
        ),
        (
            "maneuverPaths[0].internalEdgeIds[0]",
            "maneuverPaths[].internalEdgeIds[]",
        ),
        ("maneuverPaths[0].exitEdgeId", "maneuverPaths[].exitEdgeId"),
    ] {
        let mut value = signals_value();
        match core_field {
            "maneuverPaths[].entryEdgeId" => {
                value["maneuverPaths"][0]["entryEdgeId"] = json!("bad id");
            }
            "maneuverPaths[].internalEdgeIds[]" => {
                value["maneuverPaths"][0]["internalEdgeIds"] = json!(["bad id"]);
            }
            "maneuverPaths[].exitEdgeId" => {
                value["maneuverPaths"][0]["exitEdgeId"] = json!("bad id");
            }
            _ => unreachable!(),
        }
        std::assert_matches!(
            into_core_domain(load_value(value).expect_err("invalid ManeuverPath edge ID")),
            (
                path,
                CoreError::InvalidExternalId {
                    field,
                    external_id,
                    ..
                }
            ) if path == expected_path && field == core_field && external_id == "bad id"
        );
    }

    let mut value = signals_value();
    value["signals"]["controllers"][0]["groupIds"][0] = json!("unknown");
    std::assert_matches!(
        into_core_domain(load_value(value).expect_err("unknown controller group must fail")),
        (path, CoreError::UnknownSignalControllerGroup { group_id, .. })
            if path == "signals.controllers[0].groupIds[0]" && group_id == "unknown"
    );

    let mut value = signals_value();
    let duplicate = value["signals"]["controllers"][0]["phases"][0].clone();
    value["signals"]["controllers"][0]["phases"]
        .as_array_mut()
        .expect("phase array")
        .push(duplicate);
    std::assert_matches!(
        into_core_domain(load_value(value).expect_err("duplicate phase ID must fail")),
        (path, CoreError::DuplicateSignalPhaseId { phase_id, .. })
            if path == "signals.controllers[0].phases[3].id" && phase_id == "green"
    );

    let mut value = signals_value();
    value["signals"]["controllers"][0]["phases"][0]["id"] = json!("bad id");
    std::assert_matches!(
        into_core_domain(load_value(value).expect_err("invalid phase ID must fail")),
        (path, CoreError::InvalidExternalId { external_id, .. })
            if path == "signals.controllers[0].phases[0].id" && external_id == "bad id"
    );

    let mut value = signals_value();
    value["signals"]["maneuverGates"][0]["signalControl"]["groupId"] = json!("unknown");
    std::assert_matches!(
        into_core_domain(load_value(value).expect_err("unknown Gate group must fail")),
        (path, CoreError::UnknownManeuverGateSignalGroup { group_id, .. })
            if path == "signals.maneuverGates[0].signalControl.groupId" && group_id == "unknown"
    );

    let mut value = empty_value();
    value["routes"][0]["edgeIds"][1] = json!("missing");
    std::assert_matches!(
        into_core_domain(load_value(value).expect_err("unknown route edge must fail")),
        (path, CoreError::UnknownRouteEdge { edge_id, .. })
            if path == "routes[0].edgeIds[1]" && edge_id == "missing"
    );
}

#[test]
fn maneuver_path_role_conflicts_point_to_the_later_definition() {
    let earlier_internal_later_entry = role_conflict_value(
        json!({
            "id": "path-controlled",
            "movementId": "movement-controlled",
            "entryEdgeId": "entry",
            "internalEdgeIds": ["through"],
            "exitEdgeId": "bypass"
        }),
        json!({
            "id": "path-uncontrolled",
            "movementId": "movement-uncontrolled",
            "entryEdgeId": "through",
            "internalEdgeIds": [],
            "exitEdgeId": "bypass"
        }),
    );
    std::assert_matches!(
        into_core_domain(
            load_value(earlier_internal_later_entry)
                .expect_err("later entry boundary must conflict with earlier internal edge")
        ),
        (
            path,
            CoreError::ManeuverPathEdgeRoleConflict {
                internal_maneuver_path_id,
                boundary_maneuver_path_id,
                edge_id,
            }
        ) if path == "maneuverPaths[1].entryEdgeId"
            && internal_maneuver_path_id == "path-controlled"
            && boundary_maneuver_path_id == "path-uncontrolled"
            && edge_id == "through"
    );

    let earlier_internal_later_exit = role_conflict_value(
        json!({
            "id": "path-controlled",
            "movementId": "movement-controlled",
            "entryEdgeId": "entry",
            "internalEdgeIds": ["through"],
            "exitEdgeId": "bypass"
        }),
        json!({
            "id": "path-uncontrolled",
            "movementId": "movement-uncontrolled",
            "entryEdgeId": "entry",
            "internalEdgeIds": [],
            "exitEdgeId": "through"
        }),
    );
    std::assert_matches!(
        into_core_domain(
            load_value(earlier_internal_later_exit)
                .expect_err("later exit boundary must conflict with earlier internal edge")
        ),
        (
            path,
            CoreError::ManeuverPathEdgeRoleConflict {
                internal_maneuver_path_id,
                boundary_maneuver_path_id,
                edge_id,
            }
        ) if path == "maneuverPaths[1].exitEdgeId"
            && internal_maneuver_path_id == "path-controlled"
            && boundary_maneuver_path_id == "path-uncontrolled"
            && edge_id == "through"
    );

    let earlier_boundary_later_internal = role_conflict_value(
        json!({
            "id": "path-controlled",
            "movementId": "movement-controlled",
            "entryEdgeId": "entry",
            "internalEdgeIds": [],
            "exitEdgeId": "through"
        }),
        json!({
            "id": "path-uncontrolled",
            "movementId": "movement-uncontrolled",
            "entryEdgeId": "entry",
            "internalEdgeIds": ["through"],
            "exitEdgeId": "bypass"
        }),
    );
    std::assert_matches!(
        into_core_domain(
            load_value(earlier_boundary_later_internal)
                .expect_err("later internal edge must conflict with earlier boundary")
        ),
        (
            path,
            CoreError::ManeuverPathEdgeRoleConflict {
                internal_maneuver_path_id,
                boundary_maneuver_path_id,
                edge_id,
            }
        ) if path == "maneuverPaths[1].internalEdgeIds[0]"
            && internal_maneuver_path_id == "path-uncontrolled"
            && boundary_maneuver_path_id == "path-controlled"
            && edge_id == "through"
    );
}

#[test]
fn global_coverage_and_route_final_stop_line_errors_are_structured() {
    let mut value = signals_value();
    value["signals"]["maneuverGates"]
        .as_array_mut()
        .expect("Gate array")
        .pop();
    let error = load_value(value).expect_err("missing Gate coverage must fail");
    std::assert_matches!(
        into_core_domain(error),
        (path, CoreError::MissingManeuverGateCoverage { maneuver_path_id, .. })
            if path == "signals.stopLines[0]" && maneuver_path_id == "path-uncontrolled"
    );

    let mut value = signals_value();
    value["routes"][0]["edgeIds"] = json!(["entry"]);
    let error = load_value(value).expect_err("route cannot terminate at StopLine");
    std::assert_matches!(
        into_core_domain(error),
        (path, CoreError::RouteTerminatesAtStopLine { route_id, .. })
            if path == "routes[0].edgeIds[0]" && route_id == "controlled-route"
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
        into_core_domain(load_value(value).expect_err("invalid profile length")),
        (path, CoreError::InvalidVehicleProfileValue { field, .. })
            if path == "vehicleProfiles[0]" && field == "length"
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

#[test]
fn participant_class_and_profile_phase_precedes_lane_graph() {
    let mut duplicate_class = empty_value();
    let duplicate = duplicate_class["participantClasses"][0].clone();
    duplicate_class["participantClasses"]
        .as_array_mut()
        .expect("participantClasses")
        .push(duplicate);
    duplicate_class["laneGraph"]["edges"][0]["length"] = json!(0.0);
    std::assert_matches!(
        into_core_domain(
            load_value(duplicate_class).expect_err("participant class phase must fail first")
        ),
        (path, CoreError::DuplicateParticipantClassId { class_id })
            if path == "participantClasses[1].id" && class_id == "motorVehicle"
    );

    let mut unknown_class = empty_value();
    unknown_class["vehicleProfiles"][0]["participantClassId"] = json!("missing");
    unknown_class["laneGraph"]["edges"][0]["length"] = json!(0.0);
    std::assert_matches!(
        load_value(unknown_class).expect_err("profile class phase must fail before lane graph"),
        DataError::UnknownVehicleProfileParticipantClass { path, profile_id, class_id }
            if path == "vehicleProfiles[0].participantClassId"
                && profile_id == "passenger-car"
                && class_id == "missing"
    );
}

#[test]
fn participant_class_errors_use_narrowest_paths() {
    let mut unknown_extends = empty_value();
    unknown_extends["participantClasses"]
        .as_array_mut()
        .expect("participantClasses")
        .push(json!({ "id": "bus", "extendsId": "missing" }));
    std::assert_matches!(
        into_core_domain(load_value(unknown_extends).expect_err("unknown extendsId")),
        (path, CoreError::UnknownParticipantClassExtends { class_id, .. })
            if path == "participantClasses[1].extendsId" && class_id == "bus"
    );

    let mut invalid_id = empty_value();
    invalid_id["participantClasses"][0]["id"] = json!("bad id");
    std::assert_matches!(
        into_core_domain(load_value(invalid_id).expect_err("invalid class id")),
        (path, CoreError::InvalidExternalId { .. }) if path == "participantClasses[0].id"
    );
}

#[test]
fn cross_section_phase_precedes_access() {
    let mut value = signals_value();
    value["roadSections"][0]["lanes"][0]["edgeIds"][1] = json!("missing");
    value["accessRules"] = json!([{
        "id": "rule-1",
        "target": { "kind": "laneEdge", "id": "also-missing" },
        "effect": "deny",
        "participantClassIds": ["motorVehicle"]
    }]);
    std::assert_matches!(
        into_core_domain(load_value(value).expect_err("cross-section must fail before access")),
        (path, CoreError::UnknownSectionLaneEdge { .. })
            if path == "roadSections[0].lanes[0].edgeIds[1]"
    );
}

#[test]
fn cross_section_errors_use_narrowest_paths() {
    let mut disconnected = signals_value();
    disconnected["roadSections"][0]["lanes"][0]["edgeIds"] = json!(["through", "entry"]);
    std::assert_matches!(
        into_core_domain(load_value(disconnected).expect_err("disconnected lane chain")),
        (path, CoreError::DisconnectedSectionLane { lane_index, transition_index, .. })
            if path == "roadSections[0].lanes[0].edgeIds[1]"
                && lane_index == 0
                && transition_index == 0
    );

    let mut unknown_element = signals_value();
    unknown_element["roadCorridors"][0]["elements"] = json!([{ "sectionId": "missing" }]);
    std::assert_matches!(
        into_core_domain(load_value(unknown_element).expect_err("unknown corridor element")),
        (path, CoreError::UnknownCorridorElement { element_id, .. })
            if path == "roadCorridors[0].elements[0]" && element_id == "missing"
    );

    let mut unowned = signals_value();
    unowned["roadSections"]
        .as_array_mut()
        .expect("roadSections")
        .push(json!({
            "id": "section-side",
            "kindId": "motorLane",
            "lanes": [{ "edgeIds": ["bypass"] }]
        }));
    std::assert_matches!(
        into_core_domain(load_value(unowned).expect_err("section without corridor owner")),
        (path, CoreError::UnownedCorridorElement { element_id, .. })
            if path == "roadSections[1]" && element_id == "section-side"
    );

    let mut unknown_group = signals_value();
    unknown_group["roadSections"][0]["lanes"][0]["laneGroupId"] = json!("missing");
    std::assert_matches!(
        into_core_domain(load_value(unknown_group).expect_err("unknown lane group")),
        (path, CoreError::UnknownSectionLaneGroup { .. })
            if path == "roadSections[0].lanes[0].laneGroupId"
    );

    let mut not_lane_bearing = signals_value();
    not_lane_bearing["roadSections"][0]["kindId"] = json!("median");
    std::assert_matches!(
        into_core_domain(load_value(not_lane_bearing).expect_err("non lane-bearing section kind")),
        (path, CoreError::RoadSectionKindNotLaneBearing { .. })
            if path == "roadSections[0].kindId"
    );

    // referenceSectionId 语法非法必须归因到自身字段，而不是回落到 roadSections。
    let mut bad_reference = signals_value();
    bad_reference["roadCorridors"][0]["referenceSectionId"] = json!("bad id");
    std::assert_matches!(
        into_core_domain(load_value(bad_reference).expect_err("invalid referenceSectionId syntax")),
        (path, CoreError::InvalidExternalId { field, .. })
            if path == "roadCorridors[0].referenceSectionId"
                && field == "roadCorridors[].referenceSectionId"
    );
}

#[test]
fn duplicate_section_lane_edge_attributes_second_occurrence() {
    // 构造 through→entry 回边，使重复 edge 落在连通链的第二次出现处；
    // 归因必须指向第二次出现，而不是第一次合法出现。
    let mut value = signals_value();
    value["laneGraph"]["edges"][1]["connections"] = json!([{ "toEdgeId": "entry" }]);
    value["roadSections"][0]["lanes"][0]["edgeIds"] = json!(["entry", "through", "entry"]);
    std::assert_matches!(
        into_core_domain(load_value(value).expect_err("duplicate edge in lane chain")),
        (path, CoreError::DuplicateSectionLaneEdge { edge_id, .. })
            if path == "roadSections[0].lanes[0].edgeIds[2]" && edge_id == "entry"
    );
}

#[test]
fn corridor_element_attribution_distinguishes_section_and_band_kinds() {
    // section 与 band 合法共享 external ID "section-main"；重复 band 元素的错误
    // 必须归因到 band 类别的第二次出现，而不是同名 section 元素。
    let mut value = signals_value();
    value["facilityBands"] = json!([{ "id": "section-main", "kindId": "median" }]);
    value["roadCorridors"][0]["elements"] = json!([
        { "sectionId": "section-main" },
        { "bandId": "section-main" },
        { "bandId": "section-main" }
    ]);
    std::assert_matches!(
        into_core_domain(load_value(value).expect_err("duplicate band corridor element")),
        (path, CoreError::DuplicateCorridorElement { element_id, .. })
            if path == "roadCorridors[0].elements[2]" && element_id == "section-main"
    );

    // 同样地，unknown band 元素不能归因到同名 section 元素的位置。
    let mut unknown_band = signals_value();
    unknown_band["roadCorridors"][0]["elements"] = json!([
        { "sectionId": "section-main" },
        { "bandId": "missing-band" }
    ]);
    std::assert_matches!(
        into_core_domain(load_value(unknown_band).expect_err("unknown band corridor element")),
        (path, CoreError::UnknownCorridorElement { element_id, .. })
            if path == "roadCorridors[0].elements[1]" && element_id == "missing-band"
    );
}

#[test]
fn access_capability_guards_are_structured_and_attributed() {
    let mut time_windows = signals_value();
    time_windows["accessRules"] = json!([{
        "id": "rule-peak",
        "target": { "kind": "laneEdge", "id": "entry" },
        "effect": "deny",
        "participantClassIds": ["motorVehicle"],
        "timeWindows": [{
            "days": ["mon", "tue"],
            "startMinuteOfDay": 420,
            "endMinuteOfDay": 540
        }]
    }]);
    std::assert_matches!(
        into_core_domain(load_value(time_windows).expect_err("timeWindows rule must be guarded")),
        (path, CoreError::AccessCapabilityUnavailable { rule_id, capability })
            if path == "accessRules[0].timeWindows"
                && rule_id == "rule-peak"
                && capability == "timeWindows"
    );

    let mut band_target = signals_value();
    band_target["facilityBands"] = json!([{ "id": "band-median", "kindId": "median" }]);
    band_target["roadCorridors"][0]["elements"]
        .as_array_mut()
        .expect("corridor elements")
        .push(json!({ "bandId": "band-median" }));
    band_target["accessRules"] = json!([{
        "id": "rule-band",
        "target": { "kind": "facilityBand", "id": "band-median" },
        "effect": "deny",
        "participantClassIds": ["motorVehicle"]
    }]);
    std::assert_matches!(
        into_core_domain(load_value(band_target).expect_err("facilityBand target must be guarded")),
        (path, CoreError::AccessCapabilityUnavailable { rule_id, capability })
            if path == "accessRules[0].target.id"
                && rule_id == "rule-band"
                && capability == "facilityBandTarget"
    );
}

#[test]
fn time_window_minute_range_decoding_defers_to_capability_guard() {
    // 分钟字段在 wire 层保留原始数值：负数、超 u32 范围、小数等 JSON 数值都必须
    // 先抵达 capability guard（AccessCapabilityUnavailable），而不是在解码期被
    // u32 范围检查以 JsonShape 抢先拒绝——guard 先于 phase 9 shape 检查。
    for (start, end) in [
        (serde_json::json!(-1), serde_json::json!(60)),
        (
            serde_json::json!(u64::from(u32::MAX) + 1),
            serde_json::json!(60),
        ),
        (serde_json::json!(420.5), serde_json::json!(60)),
    ] {
        let mut value = signals_value();
        value["accessRules"] = json!([{
            "id": "rule-peak",
            "target": { "kind": "laneEdge", "id": "entry" },
            "effect": "deny",
            "participantClassIds": ["motorVehicle"],
            "timeWindows": [{
                "days": ["mon"],
                "startMinuteOfDay": start,
                "endMinuteOfDay": end
            }]
        }]);
        std::assert_matches!(
            into_core_domain(
                load_value(value).expect_err("numeric timeWindows must reach the guard")
            ),
            (path, CoreError::AccessCapabilityUnavailable { rule_id, capability })
                if path == "accessRules[0].timeWindows"
                    && rule_id == "rule-peak"
                    && capability == "timeWindows"
        );
    }
}

#[test]
fn regulation_shape_defers_to_capability_guard() {
    // 同一条规则同时带 timeWindows 与非法 regulation 字符串时，capability guard
    // 必须先于 regulation shape 报错（SSOT §10 phase 9：guard 先于 shape 检查）。
    let mut value = signals_value();
    value["accessRules"] = json!([{
        "id": "rule-peak",
        "target": { "kind": "laneEdge", "id": "entry" },
        "effect": "deny",
        "participantClassIds": ["motorVehicle"],
        "timeWindows": [{
            "days": ["mon"],
            "startMinuteOfDay": 420,
            "endMinuteOfDay": 540
        }],
        "regulation": { "jurisdiction": "", "version": "2024" }
    }]);
    std::assert_matches!(
        into_core_domain(load_value(value).expect_err("guard must precede regulation shape")),
        (path, CoreError::AccessCapabilityUnavailable { rule_id, capability })
            if path == "accessRules[0].timeWindows"
                && rule_id == "rule-peak"
                && capability == "timeWindows"
    );

    // 无更早 phase 错误时，shape 违规按 (field, len) 归因到对应规则字段。
    let mut invalid = signals_value();
    invalid["accessRules"] = json!([
        {
            "id": "rule-ok",
            "target": { "kind": "laneEdge", "id": "entry" },
            "effect": "deny",
            "participantClassIds": ["motorVehicle"],
            "regulation": { "jurisdiction": "cn-sh", "version": "2024" }
        },
        {
            "id": "rule-bad",
            "target": { "kind": "laneEdge", "id": "bypass" },
            "effect": "allow",
            "participantClassIds": ["motorVehicle"],
            "regulation": { "jurisdiction": "cn-sh", "version": "" }
        }
    ]);
    std::assert_matches!(
        into_core_domain(load_value(invalid).expect_err("invalid regulation shape")),
        (path, CoreError::InvalidAccessRegulationString { field, len })
            if path == "accessRules[1].regulation.version" && field == "version" && len == 0
    );
}

#[test]
fn explicit_null_on_optional_fields_is_rejected() {
    // 显式 null 不等于字段缺省（loader 路径不执行 JSON Schema）：timeWindows 为 null
    // 不得被当作未声明而绕过 capability guard；其余可选字段同口径拒绝。
    for field in ["timeWindows", "regulation", "priority"] {
        let mut doc = signals_value();
        let mut rule = json!({
            "id": "rule-1",
            "target": { "kind": "laneEdge", "id": "entry" },
            "effect": "deny",
            "participantClassIds": ["motorVehicle"]
        });
        rule[field] = json!(null);
        doc["accessRules"] = json!([rule]);
        assert!(
            load_value(doc).is_err(),
            "explicit null accessRules[].{field} must be rejected"
        );
    }

    let mut null_extends = signals_value();
    null_extends["participantClasses"][0]["extendsId"] = json!(null);
    assert!(
        load_value(null_extends).is_err(),
        "explicit null extendsId must be rejected"
    );

    let mut null_lane_group = signals_value();
    null_lane_group["roadSections"][0]["lanes"][0]["laneGroupId"] = json!(null);
    assert!(
        load_value(null_lane_group).is_err(),
        "explicit null laneGroupId must be rejected"
    );

    let mut null_source = signals_value();
    null_source["accessRules"] = json!([{
        "id": "rule-1",
        "target": { "kind": "laneEdge", "id": "entry" },
        "effect": "deny",
        "participantClassIds": ["motorVehicle"],
        "regulation": { "jurisdiction": "cn-sh", "version": "2024", "source": null }
    }]);
    assert!(
        load_value(null_source).is_err(),
        "explicit null regulation.source must be rejected"
    );
}

#[test]
fn access_rule_errors_use_narrowest_paths() {
    let mut unknown_target = signals_value();
    unknown_target["accessRules"] = json!([{
        "id": "rule-1",
        "target": { "kind": "laneEdge", "id": "missing" },
        "effect": "deny",
        "participantClassIds": ["motorVehicle"]
    }]);
    std::assert_matches!(
        into_core_domain(load_value(unknown_target).expect_err("unknown access target")),
        (path, CoreError::UnknownAccessRuleTarget { target_id, .. })
            if path == "accessRules[0].target.id" && target_id == "missing"
    );

    let mut unknown_class = signals_value();
    unknown_class["accessRules"] = json!([{
        "id": "rule-1",
        "target": { "kind": "laneEdge", "id": "entry" },
        "effect": "deny",
        "participantClassIds": ["missing"]
    }]);
    std::assert_matches!(
        into_core_domain(load_value(unknown_class).expect_err("unknown access class")),
        (path, CoreError::UnknownAccessRuleParticipantClass { class_id, .. })
            if path == "accessRules[0].participantClassIds[0]" && class_id == "missing"
    );

    let mut regulation_mismatch = signals_value();
    regulation_mismatch["accessRules"] = json!([
        {
            "id": "rule-1",
            "target": { "kind": "laneEdge", "id": "entry" },
            "effect": "deny",
            "participantClassIds": ["motorVehicle"],
            "regulation": { "jurisdiction": "cn-sh", "version": "2024" }
        },
        {
            "id": "rule-2",
            "target": { "kind": "laneEdge", "id": "bypass" },
            "effect": "allow",
            "participantClassIds": ["motorVehicle"],
            "regulation": { "jurisdiction": "cn-sh", "version": "2025", "source": "gov" }
        }
    ]);
    std::assert_matches!(
        into_core_domain(
            load_value(regulation_mismatch).expect_err("regulation provenance must be uniform")
        ),
        (path, CoreError::AccessRegulationMismatch { duplicate_rule_id, .. })
            if path == "accessRules[1].regulation" && duplicate_rule_id == "rule-2"
    );

    let mut duplicate_rule = signals_value();
    duplicate_rule["accessRules"] = json!([
        {
            "id": "rule-1",
            "target": { "kind": "laneEdge", "id": "entry" },
            "effect": "deny",
            "participantClassIds": ["motorVehicle"]
        },
        {
            "id": "rule-1",
            "target": { "kind": "laneEdge", "id": "bypass" },
            "effect": "allow",
            "participantClassIds": ["motorVehicle"]
        }
    ]);
    std::assert_matches!(
        into_core_domain(load_value(duplicate_rule).expect_err("duplicate rule id")),
        (path, CoreError::DuplicateAccessRuleId { rule_id })
            if path == "accessRules[1].id" && rule_id == "rule-1"
    );

    // regulation 字段长度越界（loader 路径不执行 JSON Schema，由 Core 构造器拒绝）。
    let mut empty_jurisdiction = signals_value();
    empty_jurisdiction["accessRules"] = json!([{
        "id": "rule-1",
        "target": { "kind": "laneEdge", "id": "entry" },
        "effect": "deny",
        "participantClassIds": ["motorVehicle"],
        "regulation": { "jurisdiction": "", "version": "2024" }
    }]);
    std::assert_matches!(
        into_core_domain(
            load_value(empty_jurisdiction).expect_err("empty regulation jurisdiction")
        ),
        (path, CoreError::InvalidAccessRegulationString { field, len })
            if path == "accessRules[0].regulation.jurisdiction" && field == "jurisdiction"
                && len == 0
    );
}

#[test]
fn access_combination_ambiguity_is_rejected_with_narrowest_path() {
    let mut value = signals_value();
    value["accessRules"] = json!([
        {
            "id": "rule-allow",
            "target": { "kind": "laneEdge", "id": "entry" },
            "effect": "allow",
            "participantClassIds": ["motorVehicle"]
        },
        {
            "id": "rule-deny",
            "target": { "kind": "laneEdge", "id": "entry" },
            "effect": "deny",
            "participantClassIds": ["motorVehicle"]
        }
    ]);
    std::assert_matches!(
        into_core_domain(load_value(value).expect_err("allow/deny tie must be ambiguous")),
        (path, CoreError::AccessRuleAmbiguity { first_rule_id, second_rule_id, .. })
            if path == "accessRules[1]"
                && first_rule_id == "rule-allow"
                && second_rule_id == "rule-deny"
    );

    let mut exempted = signals_value();
    exempted["participantClasses"]
        .as_array_mut()
        .expect("participantClasses")
        .push(json!({ "id": "bus", "extendsId": "motorVehicle" }));
    exempted["accessRules"] = json!([
        {
            "id": "rule-bus-lane",
            "target": { "kind": "laneGroup", "id": "group-main" },
            "effect": "deny",
            "participantClassIds": ["motorVehicle"]
        },
        {
            "id": "rule-bus-lane-allow-bus",
            "target": { "kind": "laneGroup", "id": "group-main" },
            "effect": "allow",
            "participantClassIds": ["bus"]
        }
    ]);
    exempted["laneGroups"] = json!([{ "id": "group-main", "roadSectionId": "section-main" }]);
    exempted["roadSections"][0]["lanes"][0]["laneGroupId"] = json!("group-main");
    let loaded = load_value(exempted).expect("deeper class exemption must resolve");
    assert!(!loaded.initial_traffic_data().access().is_empty());
}

#[test]
fn maneuver_path_target_rule_resolves_on_path_plane_end_to_end() {
    // path 平面规则经真实 fixture 端到端通过：只落在目标 ManeuverPath 上，
    // 不展平到共享 entry edge 的 edge 平面（SSOT §6.2）。
    let mut value = signals_value();
    value["accessRules"] = json!([{
        "id": "rule-path-deny",
        "target": { "kind": "maneuverPath", "id": "path-controlled" },
        "effect": "deny",
        "participantClassIds": ["motorVehicle"]
    }]);
    let loaded = load_value(value).expect("maneuverPath target rule must load");
    let traffic = loaded.initial_traffic_data();
    let motor_vehicle = traffic
        .participant_classes()
        .class_handle("motorVehicle")
        .expect("ParticipantClass handle must resolve");
    let controlled = traffic
        .junctions()
        .maneuver_path_handle("path-controlled")
        .expect("ManeuverPath handle must resolve");
    let uncontrolled = traffic
        .junctions()
        .maneuver_path_handle("path-uncontrolled")
        .expect("ManeuverPath handle must resolve");
    assert!(matches!(
        traffic.access().path_access(controlled, motor_vehicle),
        laneflow_core::AccessCell::Decided {
            effect: laneflow_core::AccessEffect::Deny,
            ..
        }
    ));
    assert!(matches!(
        traffic.access().path_access(uncontrolled, motor_vehicle),
        laneflow_core::AccessCell::Unconstrained
    ));
    let entry = traffic
        .lane_graph()
        .edge_handle("entry")
        .expect("LaneEdge handle must resolve");
    assert!(matches!(
        traffic.access().edge_access(entry, motor_vehicle),
        laneflow_core::AccessCell::Unconstrained
    ));
}

#[test]
fn current_v0_9_profile_and_new_domain_shapes_are_closed() {
    let mut missing_class_id = empty_value();
    missing_class_id["vehicleProfiles"][0]
        .as_object_mut()
        .expect("profile")
        .remove("participantClassId");
    std::assert_matches!(
        load_value(missing_class_id).expect_err("participantClassId is required"),
        DataError::JsonShape { path, .. } if path.contains("vehicleProfiles[0]")
    );

    let mut typo_class = empty_value();
    typo_class["participantClasses"][0]["typo"] = json!(true);
    std::assert_matches!(
        load_value(typo_class).expect_err("participantClass shape is closed"),
        DataError::JsonShape { path, .. } if path.contains("participantClasses[0]")
    );

    let mut typo_rule = empty_value();
    typo_rule["accessRules"] = json!([{
        "id": "rule-1",
        "target": { "kind": "laneEdge", "id": "entry" },
        "effect": "deny",
        "participantClassIds": ["motorVehicle"],
        "typo": true
    }]);
    std::assert_matches!(
        load_value(typo_rule).expect_err("accessRule shape is closed"),
        DataError::JsonShape { path, .. } if path.contains("accessRules[0]")
    );

    for element in [
        json!({ "sectionId": "section-main", "bandId": "band-1" }),
        json!({ "typo": "section-main" }),
    ] {
        let mut value = empty_value();
        value["roadCorridors"][0]["elements"] = json!([element]);
        std::assert_matches!(
            load_value(value).expect_err("corridor element must be exactly one reference"),
            DataError::JsonShape { path, .. } if path.contains("roadCorridors[0].elements[0]")
        );
    }
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

fn role_conflict_value(first_path: Value, second_path: Value) -> Value {
    let mut value = signals_value();
    value["laneGraph"]["edges"][1]["connections"] = json!([{ "toEdgeId": "bypass" }]);
    value["maneuverPaths"] = json!([first_path, second_path]);
    value
}
