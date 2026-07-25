use laneflow_core::{
    CoreError, CoreWorld, EdgeLength, InitialTrafficData, Junction, JunctionRegistry, LaneEdge,
    LaneGraph, ManeuverGate, ManeuverGateSignalState, ManeuverPath, Movement, ParkingRegistry,
    Route, SignalAspect, SignalControlInput, SignalController, SignalGroup, SignalGroupState,
    SignalLayerPermission, SignalPhase, SignalRegistry, SpeedLimit, StopLine, StopLineLocation,
    VehicleProfileRegistry,
};

fn edge(id: &str, next: &[&str]) -> LaneEdge {
    LaneEdge::new(
        id,
        EdgeLength::try_new(10.0).expect("test edge length"),
        SpeedLimit::try_new(10.0).expect("test speed limit"),
        next.iter().copied(),
    )
}

#[test]
fn route_compiler_uses_full_path_identity_for_shared_entry_pair() {
    let graph = LaneGraph::try_new([
        edge("entry", &["internal"]),
        edge("internal", &["exit-a", "exit-b", "bypass"]),
        edge("exit-a", &[]),
        edge("exit-b", &[]),
        edge("bypass", &[]),
    ])
    .expect("test graph");
    let junctions = JunctionRegistry::try_new(
        &graph,
        [Junction::new("junction")],
        [
            Movement::new("movement-a", "junction"),
            Movement::new("movement-b", "junction"),
        ],
        [
            ManeuverPath::new("path-a", "movement-a", "entry", ["internal"], "exit-a"),
            ManeuverPath::new("path-b", "movement-b", "entry", ["internal"], "exit-b"),
        ],
    )
    .expect("junction registry");

    let signals = SignalRegistry::try_new(
        &graph,
        &junctions,
        [StopLine::new("stop", "entry", StopLineLocation::EdgeEnd)],
        [SignalGroup::new("group")],
        [SignalController::new_fixed_time(
            "controller",
            0,
            ["group"],
            [SignalPhase::new(
                "red",
                1_000,
                [SignalGroupState::new("group", SignalAspect::Red)],
            )],
        )],
        [
            ManeuverGate::new(
                "gate-a",
                "path-a",
                0,
                "stop",
                SignalControlInput::Group("group".to_owned()),
            ),
            ManeuverGate::new(
                "gate-b",
                "path-b",
                0,
                "stop",
                SignalControlInput::Group("group".to_owned()),
            ),
        ],
    )
    .expect("signal registry");

    let route_a =
        Route::try_new("route-a", ["entry", "internal", "exit-a"]).expect("route-a definition");
    let traffic = InitialTrafficData::try_new(
        graph,
        [route_a],
        VehicleProfileRegistry::empty(),
        junctions,
        signals,
        ParkingRegistry::empty(),
    )
    .expect("initial traffic");
    let mut world = CoreWorld::with_traffic_data(20, traffic, Vec::new()).expect("world");

    let route_a = world.route_handle("route-a").expect("route-a handle");
    let path_a = world
        .junctions()
        .maneuver_path_handle("path-a")
        .expect("path-a handle");
    let gate_a = world
        .signals()
        .maneuver_gate_handle("gate-a")
        .expect("gate-a handle");
    let occurrences = world
        .route_maneuver_occurrences(route_a)
        .expect("route-a occurrences");
    assert_eq!(occurrences.len(), 1);
    assert_eq!(occurrences[0].maneuver_path(), path_a);
    assert_eq!(occurrences[0].entry_route_edge_index(), 0);
    assert_eq!(occurrences[0].exit_route_edge_index(), 2);
    assert_eq!(world.route_transition_gate(route_a, 0), Some(Some(gate_a)));

    let gate_state = world
        .maneuver_gate_state(gate_a)
        .expect("gate-a runtime state");
    assert!(matches!(
        gate_state.signal(),
        ManeuverGateSignalState::Controlled {
            aspect: SignalAspect::Red,
            permission: SignalLayerPermission::DenyAndStop,
            ..
        }
    ));

    let route_b = world
        .register_route(
            Route::try_new("route-b", ["entry", "internal", "exit-b"]).expect("route-b definition"),
        )
        .expect("dynamic route-b");
    let path_b = world
        .junctions()
        .maneuver_path_handle("path-b")
        .expect("path-b handle");
    let gate_b = world
        .signals()
        .maneuver_gate_handle("gate-b")
        .expect("gate-b handle");
    let occurrences = world
        .route_maneuver_occurrences(route_b)
        .expect("route-b occurrences");
    assert_eq!(occurrences.len(), 1);
    assert_eq!(occurrences[0].maneuver_path(), path_b);
    assert_eq!(occurrences[0].entry_route_edge_index(), 0);
    assert_eq!(occurrences[0].exit_route_edge_index(), 2);
    assert_eq!(world.route_transition_gate(route_b, 0), Some(Some(gate_b)));
    assert_ne!(gate_a, gate_b);

    let route_count = world.routes().count();
    let error = world
        .register_route(
            Route::try_new("route-bypass", ["entry", "internal", "bypass"])
                .expect("route-bypass definition"),
        )
        .expect_err("incomplete candidate match must fail");
    assert!(matches!(
        error,
        CoreError::RouteManeuverNoFullMatch {
            route_id,
            entry_route_edge_index: 0,
            candidate_count: 2,
            ..
        } if route_id == "route-bypass"
    ));
    assert_eq!(world.routes().count(), route_count);
    assert!(world.route_handle("route-bypass").is_none());
}
