use laneflow_core::{
    AccessRegistry, CoreError, CoreWorld, CrossSectionRegistry, EdgeLength, EdgeProgress,
    IidmProfileSpec, InitialTrafficData, Junction, JunctionRegistry, LaneEdge, LaneGraph,
    ManeuverGate, ManeuverPath, Movement, ParkingRegistry, ParticipantClass,
    ParticipantClassRegistry, Route, SignalControlInput, SignalRegistry, Speed, SpeedLimit,
    StopLine, StopLineLocation, VehicleProfile, VehicleProfileRegistry, WaitingRegistry,
    WaitingZone, WaitingZoneError,
};

fn edge(id: &str, length: f64, next: &[&str]) -> LaneEdge {
    LaneEdge::new(
        id,
        EdgeLength::try_new(length).expect("edge length"),
        SpeedLimit::try_new(10.0).expect("speed limit"),
        next.iter().copied(),
    )
}

fn graph() -> LaneGraph {
    LaneGraph::try_new([
        edge("entry", 10.0, &["internal-a"]),
        edge("internal-a", 6.0, &["internal-b"]),
        edge("internal-b", 7.0, &["exit"]),
        edge("exit", 10.0, &["entry"]),
    ])
    .expect("graph")
}

fn junctions(graph: &LaneGraph) -> JunctionRegistry {
    JunctionRegistry::try_new(
        graph,
        [Junction::new("junction")],
        [Movement::new("movement", "junction")],
        [ManeuverPath::new(
            "path",
            "movement",
            "entry",
            ["internal-a", "internal-b"],
            "exit",
        )],
    )
    .expect("junctions")
}

fn signals(graph: &LaneGraph, junctions: &JunctionRegistry) -> SignalRegistry {
    SignalRegistry::try_new(
        graph,
        junctions,
        [
            StopLine::new("stop-entry", "entry", StopLineLocation::EdgeEnd),
            StopLine::new("stop-middle", "internal-a", StopLineLocation::EdgeEnd),
            StopLine::new("stop-release", "internal-b", StopLineLocation::EdgeEnd),
        ],
        std::iter::empty(),
        std::iter::empty(),
        [
            // Deliberately reverse input order: per-path queries must use transition order.
            ManeuverGate::new(
                "gate-release",
                "path",
                2,
                "stop-release",
                SignalControlInput::None,
            ),
            ManeuverGate::new(
                "gate-middle",
                "path",
                1,
                "stop-middle",
                SignalControlInput::None,
            ),
            ManeuverGate::new(
                "gate-entry",
                "path",
                0,
                "stop-entry",
                SignalControlInput::None,
            ),
        ],
    )
    .expect("signals")
}

fn waiting(junctions: &JunctionRegistry, signals: &SignalRegistry) -> WaitingRegistry {
    WaitingRegistry::try_new(
        junctions,
        signals,
        [
            WaitingZone::new("zone-b", "path", "gate-middle", "gate-release", 1),
            WaitingZone::new("zone-a", "path", "gate-entry", "gate-middle", 2),
        ],
    )
    .expect("waiting zones")
}

fn classes_and_profiles() -> (ParticipantClassRegistry, VehicleProfileRegistry) {
    let classes = ParticipantClassRegistry::try_new(vec![ParticipantClass::new("car", None)])
        .expect("classes");
    let class = classes.class_handle("car").expect("car class");
    let profile = |id, length| {
        VehicleProfile::try_new_iidm(
            id,
            class,
            IidmProfileSpec {
                length,
                desired_speed: 10.0,
                min_gap: 2.0,
                time_headway: 1.5,
                max_acceleration: 1.5,
                comfortable_deceleration: 2.0,
                emergency_deceleration: 6.0,
            },
        )
        .expect("profile")
    };
    let profiles =
        VehicleProfileRegistry::try_new(&classes, [profile("short", 5.0), profile("long", 8.0)])
            .expect("profiles");
    (classes, profiles)
}

fn world_from_parts(
    graph: LaneGraph,
    junctions: JunctionRegistry,
    signals: SignalRegistry,
    waiting: WaitingRegistry,
) -> CoreWorld {
    let (classes, profiles) = classes_and_profiles();
    let traffic = InitialTrafficData::try_new_with_waiting(
        graph,
        [Route::try_new("route", ["entry", "internal-a", "internal-b", "exit"]).expect("route")],
        profiles,
        junctions,
        signals,
        ParkingRegistry::empty(),
        classes,
        CrossSectionRegistry::empty(),
        AccessRegistry::empty(),
        waiting,
    )
    .expect("traffic");
    CoreWorld::with_traffic_data(20, traffic, Vec::new()).expect("world")
}

fn world() -> CoreWorld {
    let graph = graph();
    let junctions = junctions(&graph);
    let signals = signals(&graph, &junctions);
    let waiting = waiting(&junctions, &signals);
    world_from_parts(graph, junctions, signals, waiting)
}

#[test]
fn waiting_registry_orders_shared_boundaries_and_rejects_overlap() {
    let graph = graph();
    let junctions = junctions(&graph);
    let signals = signals(&graph, &junctions);
    let waiting = waiting(&junctions, &signals);
    let path = junctions.maneuver_path_handle("path").expect("path");
    let ids = waiting
        .maneuver_path_waiting_zones(path)
        .expect("path waiting zones")
        .map(|handle| {
            waiting
                .waiting_zone_external_id(handle)
                .expect("zone id")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(ids, ["zone-a", "zone-b"]);

    let error = WaitingRegistry::try_new(
        &junctions,
        &signals,
        [
            WaitingZone::new("outer", "path", "gate-entry", "gate-release", 1),
            WaitingZone::new("nested", "path", "gate-middle", "gate-release", 1),
        ],
    )
    .expect_err("overlap and nesting must fail");
    std::assert_matches!(
        error,
        CoreError::WaitingZone(WaitingZoneError::Overlap {
            maneuver_path_id,
            first_waiting_zone_id,
            second_waiting_zone_id,
        }) if maneuver_path_id == "path"
            && first_waiting_zone_id == "outer"
            && second_waiting_zone_id == "nested"
    );
}

#[test]
fn route_compiles_all_gate_and_waiting_zone_occurrences() {
    let world = world();
    let route = world.route_handle("route").expect("route");
    let gates = world
        .route_gate_occurrences(route)
        .expect("gate occurrences");
    assert_eq!(gates.len(), 3);
    assert_eq!(
        gates
            .iter()
            .map(|occurrence| occurrence.from_route_edge_index())
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(gates[0].next_gate_occurrence_index(), Some(1));
    assert_eq!(gates[0].next_boundary_route_edge_index(), 1);
    assert_eq!(gates[1].next_gate_occurrence_index(), Some(2));
    assert_eq!(gates[2].next_gate_occurrence_index(), None);
    assert_eq!(gates[2].next_boundary_route_edge_index(), 3);

    let zones = world
        .route_waiting_zone_occurrences(route)
        .expect("waiting occurrences");
    assert_eq!(zones.len(), 2);
    assert_eq!(zones[0].entry_gate_occurrence_index(), 0);
    assert_eq!(zones[0].release_gate_occurrence_index(), 1);
    assert_eq!(zones[0].entry_route_edge_index(), 0);
    assert_eq!(zones[0].release_route_edge_index(), 1);
    assert_eq!(zones[1].entry_gate_occurrence_index(), 1);
    assert_eq!(zones[1].release_gate_occurrence_index(), 2);
    assert_eq!(zones[1].entry_route_edge_index(), 1);
    assert_eq!(zones[1].release_route_edge_index(), 2);

    let maneuver = world
        .route_maneuver_occurrences(route)
        .expect("maneuver occurrences")[0];
    assert_eq!(maneuver.gate_occurrence_range(), 0..3);
    assert_eq!(maneuver.waiting_zone_occurrence_range(), 0..2);
}

#[test]
fn repeated_path_and_dynamic_route_lifecycle_keep_occurrence_identity() {
    let mut world = world();
    let repeated_edges = (0..2)
        .flat_map(|_| ["entry", "internal-a", "internal-b", "exit"])
        .collect::<Vec<_>>();
    let first = world
        .register_route(Route::try_new("dynamic", repeated_edges.clone()).expect("dynamic route"))
        .expect("dynamic route registration");

    let maneuvers = world
        .route_maneuver_occurrences(first)
        .expect("maneuver occurrences");
    assert_eq!(maneuvers.len(), 2);
    assert_eq!(maneuvers[0].gate_occurrence_range(), 0..3);
    assert_eq!(maneuvers[0].waiting_zone_occurrence_range(), 0..2);
    assert_eq!(maneuvers[1].gate_occurrence_range(), 3..6);
    assert_eq!(maneuvers[1].waiting_zone_occurrence_range(), 2..4);
    assert_eq!(
        world
            .route_gate_occurrences(first)
            .expect("gate occurrences")
            .iter()
            .map(|occurrence| occurrence.maneuver_occurrence_index())
            .collect::<Vec<_>>(),
        [0, 0, 0, 1, 1, 1]
    );
    assert_eq!(
        world
            .route_waiting_zone_occurrences(first)
            .expect("waiting occurrences")
            .iter()
            .map(|occurrence| occurrence.maneuver_occurrence_index())
            .collect::<Vec<_>>(),
        [0, 0, 1, 1]
    );

    world.remove_route(first).expect("remove dynamic route");
    assert!(world.route_gate_occurrences(first).is_none());
    assert!(world.route_waiting_zone_occurrences(first).is_none());

    let second = world
        .register_route(Route::try_new("dynamic", repeated_edges).expect("replacement route"))
        .expect("replacement route registration");
    assert_ne!(second, first);
    assert_eq!(
        world
            .route_waiting_zone_occurrences(second)
            .expect("replacement waiting occurrences")
            .len(),
        4
    );
}

#[test]
fn binding_checks_static_capacity_before_runtime_capability_guards() {
    let mut world = world();
    let short = world
        .vehicle_profiles()
        .profile_handle("short")
        .expect("short profile");
    let long = world
        .vehicle_profiles()
        .profile_handle("long")
        .expect("long profile");
    let spawn = |id, profile, cursor| {
        laneflow_core::VehicleSpawnInput::active(
            id,
            profile,
            "route",
            cursor,
            EdgeProgress::try_new(0.0).expect("progress"),
            Speed::ZERO,
        )
    };

    let short_error = world
        .spawn_vehicle(spawn("short-before", short, 0))
        .expect_err("runtime capability must prevent silent traversal");
    std::assert_matches!(
        short_error,
        CoreError::WaitingZone(WaitingZoneError::RuntimeUnavailable {
            route_id,
            waiting_zone_id,
        }) if route_id == "route" && waiting_zone_id == "zone-a"
    );

    let long_error = world
        .spawn_vehicle(spawn("long-before", long, 0))
        .expect_err("static capacity must fail before runtime capability");
    std::assert_matches!(
        long_error,
        CoreError::WaitingZone(WaitingZoneError::InsufficientStorage {
            profile_id,
            route_id,
            waiting_zone_id,
            available_meters: 6.0,
            required_meters: 8.0,
        }) if profile_id == "long" && route_id == "route" && waiting_zone_id == "zone-a"
    );

    let release_boundary_error = world
        .spawn_vehicle(spawn("long-at-release", long, 1))
        .expect_err("release Gate edge remains pending for static feasibility");
    std::assert_matches!(
        release_boundary_error,
        CoreError::WaitingZone(WaitingZoneError::InsufficientStorage {
            profile_id,
            route_id,
            waiting_zone_id,
            available_meters: 6.0,
            required_meters: 8.0,
        }) if profile_id == "long" && route_id == "route" && waiting_zone_id == "zone-a"
    );

    let bootstrap_error = world
        .spawn_vehicle(spawn("short-inside", short, 1))
        .expect_err("stateful interior cursor needs explicit bootstrap");
    std::assert_matches!(
        bootstrap_error,
        CoreError::StatefulManeuverBootstrapUnavailable {
            route_id,
            maneuver_path_id,
            first_gate_route_edge_index: 0,
            exit_route_edge_index: 3,
            cursor: 1,
        } if route_id == "route" && maneuver_path_id == "path"
    );

    world
        .spawn_vehicle(spawn("short-after", short, 3))
        .expect("completed maneuver suffix has no pending WaitingZone");
}

#[test]
fn binding_rejects_unprovable_waiting_storage_distance() {
    let graph = LaneGraph::try_new([
        edge("entry", 10.0, &["internal-a"]),
        edge("internal-a", f64::MAX, &["internal-b"]),
        edge("internal-b", f64::MAX, &["exit"]),
        edge("exit", 10.0, &["entry"]),
    ])
    .expect("graph");
    let junctions = junctions(&graph);
    let signals = signals(&graph, &junctions);
    let waiting = WaitingRegistry::try_new(
        &junctions,
        &signals,
        [WaitingZone::new(
            "zone-overflow",
            "path",
            "gate-entry",
            "gate-release",
            1,
        )],
    )
    .expect("waiting zone");
    let mut world = world_from_parts(graph, junctions, signals, waiting);
    let short = world
        .vehicle_profiles()
        .profile_handle("short")
        .expect("short profile");

    let error = world
        .spawn_vehicle(laneflow_core::VehicleSpawnInput::active(
            "unprovable-storage",
            short,
            "route",
            0,
            EdgeProgress::try_new(0.0).expect("progress"),
            Speed::ZERO,
        ))
        .expect_err("unrepresentable storage distance must fail closed");

    std::assert_matches!(
        error,
        CoreError::WaitingZone(WaitingZoneError::StorageDistanceUnprovable {
            profile_id,
            route_id,
            waiting_zone_id,
            entry_route_edge_index: 0,
            release_route_edge_index: 2,
        }) if profile_id == "short"
            && route_id == "route"
            && waiting_zone_id == "zone-overflow"
    );
    assert!(world.vehicle_handle("unprovable-storage").is_none());
}

#[test]
fn pure_multi_gate_maneuver_requires_bootstrap_but_single_gate_does_not() {
    let multi_graph = graph();
    let multi_junctions = junctions(&multi_graph);
    let multi_signals = signals(&multi_graph, &multi_junctions);
    let mut multi_gate_world = world_from_parts(
        multi_graph,
        multi_junctions,
        multi_signals,
        WaitingRegistry::empty(),
    );
    let short = multi_gate_world
        .vehicle_profiles()
        .profile_handle("short")
        .expect("short profile");
    let spawn = |id| {
        laneflow_core::VehicleSpawnInput::active(
            id,
            short,
            "route",
            1,
            EdgeProgress::try_new(0.0).expect("progress"),
            Speed::ZERO,
        )
    };

    let error = multi_gate_world
        .spawn_vehicle(spawn("multi-gate-inside"))
        .expect_err("pure multi-Gate occurrence is stateful");
    std::assert_matches!(
        error,
        CoreError::StatefulManeuverBootstrapUnavailable {
            route_id,
            maneuver_path_id,
            first_gate_route_edge_index: 0,
            exit_route_edge_index: 3,
            cursor: 1,
        } if route_id == "route" && maneuver_path_id == "path"
    );

    let single_graph = graph();
    let single_junctions = junctions(&single_graph);
    let single_gate_signals = SignalRegistry::try_new(
        &single_graph,
        &single_junctions,
        [StopLine::new(
            "stop-entry",
            "entry",
            StopLineLocation::EdgeEnd,
        )],
        std::iter::empty(),
        std::iter::empty(),
        [ManeuverGate::new(
            "gate-entry",
            "path",
            0,
            "stop-entry",
            SignalControlInput::None,
        )],
    )
    .expect("single gate signals");
    let mut single_gate_world = world_from_parts(
        single_graph,
        single_junctions,
        single_gate_signals,
        WaitingRegistry::empty(),
    );
    let short = single_gate_world
        .vehicle_profiles()
        .profile_handle("short")
        .expect("short profile");

    single_gate_world
        .spawn_vehicle(laneflow_core::VehicleSpawnInput::active(
            "single-gate-inside",
            short,
            "route",
            1,
            EdgeProgress::try_new(0.0).expect("progress"),
            Speed::ZERO,
        ))
        .expect("single-Gate occurrence needs no state bootstrap");
}

#[test]
fn pending_waiting_uses_release_boundary_before_bootstrap_and_runtime_guards() {
    let later_graph = graph();
    let later_junctions = junctions(&later_graph);
    let later_signals = signals(&later_graph, &later_junctions);
    let later_waiting = WaitingRegistry::try_new(
        &later_junctions,
        &later_signals,
        [WaitingZone::new(
            "zone-b",
            "path",
            "gate-middle",
            "gate-release",
            1,
        )],
    )
    .expect("later waiting zone");
    let mut later_world =
        world_from_parts(later_graph, later_junctions, later_signals, later_waiting);
    let short = later_world
        .vehicle_profiles()
        .profile_handle("short")
        .expect("short profile");

    let error = later_world
        .spawn_vehicle(laneflow_core::VehicleSpawnInput::active(
            "before-later-zone",
            short,
            "route",
            1,
            EdgeProgress::try_new(0.0).expect("progress"),
            Speed::ZERO,
        ))
        .expect_err("maneuver bootstrap precedes pending Waiting runtime guard");
    std::assert_matches!(
        error,
        CoreError::StatefulManeuverBootstrapUnavailable {
            route_id,
            maneuver_path_id,
            first_gate_route_edge_index: 0,
            exit_route_edge_index: 3,
            cursor: 1,
        } if route_id == "route" && maneuver_path_id == "path"
    );

    let completed_graph = graph();
    let completed_junctions = junctions(&completed_graph);
    let completed_signals = signals(&completed_graph, &completed_junctions);
    let completed_waiting = WaitingRegistry::try_new(
        &completed_junctions,
        &completed_signals,
        [WaitingZone::new(
            "zone-a",
            "path",
            "gate-entry",
            "gate-middle",
            2,
        )],
    )
    .expect("completed waiting zone");
    let mut completed_world = world_from_parts(
        completed_graph,
        completed_junctions,
        completed_signals,
        completed_waiting,
    );
    let long = completed_world
        .vehicle_profiles()
        .profile_handle("long")
        .expect("long profile");

    let error = completed_world
        .spawn_vehicle(laneflow_core::VehicleSpawnInput::active(
            "after-zone-release",
            long,
            "route",
            2,
            EdgeProgress::try_new(0.0).expect("progress"),
            Speed::ZERO,
        ))
        .expect_err("completed WaitingZone is ignored before maneuver bootstrap");
    std::assert_matches!(
        error,
        CoreError::StatefulManeuverBootstrapUnavailable {
            route_id,
            maneuver_path_id,
            first_gate_route_edge_index: 0,
            exit_route_edge_index: 3,
            cursor: 2,
        } if route_id == "route" && maneuver_path_id == "path"
    );
}
