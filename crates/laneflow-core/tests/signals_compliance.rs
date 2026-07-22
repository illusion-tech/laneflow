use laneflow_core::{
    CoreEvent, CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge,
    LaneGraph, MovementGate, MovementGateKey, Route, SignalAspect, SignalControlInput,
    SignalController, SignalGroup, SignalGroupState, SignalPhase, SignalRegistry, Speed, StopLine,
    StopLineLocation, TickInput, VehicleProfile, VehicleProfileHandle, VehicleProfileRegistry,
    VehicleSpawnInput, VehicleState, VehicleStatus,
};

fn edge_length(value: f64) -> EdgeLength {
    EdgeLength::try_new(value).expect("valid edge length")
}

fn progress(value: f64) -> EdgeProgress {
    EdgeProgress::try_new(value).expect("valid progress")
}

fn speed(value: f64) -> Speed {
    Speed::try_new(value).expect("valid speed")
}

fn phase(id: &str, duration_ms: u64, states: &[(&str, SignalAspect)]) -> SignalPhase {
    SignalPhase::new(
        id,
        duration_ms,
        states
            .iter()
            .map(|(group, aspect)| SignalGroupState::new(*group, *aspect)),
    )
}

fn profiles() -> (VehicleProfileRegistry, VehicleProfileHandle) {
    let registry = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        "car",
        IidmProfileSpec {
            length: 4.0,
            desired_speed: 30.0,
            min_gap: 2.0,
            time_headway: 1.0,
            max_acceleration: 2.0,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 8.0,
        },
    )
    .expect("valid profile")])
    .expect("valid profile registry");
    let handle = registry.profile_handle("car").expect("profile handle");
    (registry, handle)
}

fn single_gate_world(
    phases: Vec<SignalPhase>,
    vehicles: impl FnOnce(VehicleProfileHandle) -> Vec<VehicleSpawnInput>,
) -> CoreWorld {
    let graph = LaneGraph::try_new([
        LaneEdge::new(
            "entry",
            edge_length(100.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["exit"],
        ),
        LaneEdge::new(
            "exit",
            edge_length(100.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            Vec::<String>::new(),
        ),
    ])
    .expect("valid graph");
    let signals = SignalRegistry::try_new(
        &graph,
        [StopLine::new(
            "stop-entry",
            "entry",
            StopLineLocation::EdgeEnd,
        )],
        [SignalGroup::new("main")],
        [SignalController::new_fixed_time(
            "controller",
            0,
            ["main"],
            phases,
        )],
        [MovementGate::new(
            "entry",
            "exit",
            "stop-entry",
            SignalControlInput::Group("main".to_owned()),
        )],
    )
    .expect("valid signals");
    let (profiles, profile) = profiles();
    let traffic = InitialTrafficData::try_new_with_signals(
        graph,
        [Route::try_new("route", ["entry", "exit"]).expect("valid route")],
        profiles,
        signals,
    )
    .expect("valid traffic");
    CoreWorld::with_traffic_data(1_000, traffic, vehicles(profile)).expect("valid world")
}

fn vehicle<'a>(world: &'a CoreWorld, id: &str) -> &'a VehicleState {
    world
        .vehicle(world.vehicle_handle(id).expect("vehicle handle"))
        .expect("vehicle state")
}

#[test]
fn red_and_restrictive_yellow_stop_at_exact_boundary_without_projection_event() {
    for (id, aspect) in [("red", SignalAspect::Red), ("yellow", SignalAspect::Yellow)] {
        let mut world =
            single_gate_world(vec![phase(id, 60_000, &[("main", aspect)])], |profile| {
                vec![VehicleSpawnInput::active(
                    "vehicle",
                    profile,
                    "route",
                    0,
                    progress(99.0),
                    Speed::ZERO,
                )]
            });

        let mut events = Vec::new();
        for _ in 0..5 {
            events.extend(
                world
                    .step(TickInput::new(1_000))
                    .expect("step succeeds")
                    .events,
            );
            assert_eq!(vehicle(&world, "vehicle").route_edge_index, 0);
            assert!(vehicle(&world, "vehicle").edge_progress.value() <= 100.0);
        }
        let vehicle = vehicle(&world, "vehicle");

        assert!(events.is_empty(), "normal {id} stop is not an event");
        assert_eq!(vehicle.route_edge_index, 0);
        assert_eq!(vehicle.edge_progress, progress(100.0));
        assert_eq!(vehicle.current_speed, Speed::ZERO);
        assert_eq!(vehicle.status, VehicleStatus::Active);
    }
}

#[test]
fn protected_green_and_uncontrolled_gate_allow_crossing() {
    let mut green = single_gate_world(
        vec![phase("green", 60_000, &[("main", SignalAspect::Green)])],
        |profile| {
            vec![VehicleSpawnInput::active(
                "vehicle",
                profile,
                "route",
                0,
                progress(99.0),
                Speed::ZERO,
            )]
        },
    );
    let result = green.step(TickInput::new(1_000)).expect("green step");
    assert_eq!(vehicle(&green, "vehicle").route_edge_index, 1);
    std::assert_matches!(result.events.as_slice(), [CoreEvent::VehicleChangedEdge(_)]);

    let graph = LaneGraph::try_new([
        LaneEdge::new(
            "entry",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["exit"],
        ),
        LaneEdge::new(
            "exit",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            Vec::<String>::new(),
        ),
    ])
    .expect("graph");
    let signals = SignalRegistry::try_new(
        &graph,
        [StopLine::new("stop", "entry", StopLineLocation::EdgeEnd)],
        std::iter::empty::<SignalGroup>(),
        std::iter::empty::<SignalController>(),
        [MovementGate::new(
            "entry",
            "exit",
            "stop",
            SignalControlInput::None,
        )],
    )
    .expect("uncontrolled signals");
    let (profiles, profile) = profiles();
    let traffic = InitialTrafficData::try_new_with_signals(
        graph,
        [Route::try_new("route", ["entry", "exit"]).expect("route")],
        profiles,
        signals,
    )
    .expect("traffic");
    let mut uncontrolled = CoreWorld::with_traffic_data(
        1_000,
        traffic,
        vec![VehicleSpawnInput::active(
            "vehicle",
            profile,
            "route",
            0,
            progress(9.0),
            Speed::ZERO,
        )],
    )
    .expect("world");
    uncontrolled
        .step(TickInput::new(1_000))
        .expect("uncontrolled step");
    assert_eq!(vehicle(&uncontrolled, "vehicle").route_edge_index, 1);
}

#[test]
fn hard_signal_projection_emits_once_before_route_events() {
    let mut world = single_gate_world(
        vec![phase("red", 60_000, &[("main", SignalAspect::Red)])],
        |profile| {
            vec![VehicleSpawnInput::active(
                "vehicle",
                profile,
                "route",
                0,
                progress(99.0),
                speed(20.0),
            )]
        },
    );

    let result = world.step(TickInput::new(1_000)).expect("step succeeds");
    let vehicle = vehicle(&world, "vehicle");

    std::assert_matches!(
        result.events.as_slice(),
        [CoreEvent::VehicleSignalStopProjectionApplied(event)]
            if event.vehicle == vehicle.handle
                && event.from_route_edge_index == 0
                && event.to_route_edge_index == 1
                && event.gate
                    == MovementGateKey::new(
                        world.edge_handle("entry").expect("entry handle"),
                        world.edge_handle("exit").expect("exit handle"),
                    )
                && event.stop_line
                    == world
                        .signals()
                        .stop_line_handle("stop-entry")
                        .expect("stop-line handle")
                && event.group == world.signals().group_handle("main").expect("group handle")
                && event.aspect == SignalAspect::Red
    );
    assert_eq!(vehicle.route_edge_index, 0);
    assert_eq!(vehicle.edge_progress, progress(100.0));
    assert_eq!(vehicle.current_speed, Speed::ZERO);
}

#[test]
fn exact_boundary_crosses_only_on_tick_after_red_turns_green() {
    let mut world = single_gate_world(
        vec![
            phase("red", 1_000, &[("main", SignalAspect::Red)]),
            phase("green", 60_000, &[("main", SignalAspect::Green)]),
        ],
        |profile| {
            vec![VehicleSpawnInput::active(
                "vehicle",
                profile,
                "route",
                0,
                progress(100.0),
                Speed::ZERO,
            )]
        },
    );

    let red_tick = world.step(TickInput::new(1_000)).expect("red tick");
    assert_eq!(vehicle(&world, "vehicle").route_edge_index, 0);
    assert!(
        red_tick
            .events
            .iter()
            .all(|event| !matches!(event, CoreEvent::VehicleChangedEdge(_)))
    );

    let green_tick = world.step(TickInput::new(1_000)).expect("green tick");
    assert_eq!(vehicle(&world, "vehicle").route_edge_index, 1);
    std::assert_matches!(
        green_tick.events.as_slice(),
        [CoreEvent::VehicleChangedEdge(_)]
    );
}

#[test]
fn one_tick_crosses_permitted_gate_then_stops_at_nearest_denied_gate() {
    let graph = LaneGraph::try_new([
        LaneEdge::new(
            "a",
            edge_length(5.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["b"],
        ),
        LaneEdge::new(
            "b",
            edge_length(5.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["c"],
        ),
        LaneEdge::new(
            "c",
            edge_length(100.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            Vec::<String>::new(),
        ),
    ])
    .expect("graph");
    let signals = SignalRegistry::try_new(
        &graph,
        [
            StopLine::new("sa", "a", StopLineLocation::EdgeEnd),
            StopLine::new("sb", "b", StopLineLocation::EdgeEnd),
        ],
        [SignalGroup::new("g1"), SignalGroup::new("g2")],
        [SignalController::new_fixed_time(
            "controller",
            0,
            ["g1", "g2"],
            [phase(
                "phase",
                60_000,
                &[("g1", SignalAspect::Green), ("g2", SignalAspect::Red)],
            )],
        )],
        [
            MovementGate::new("a", "b", "sa", SignalControlInput::Group("g1".to_owned())),
            MovementGate::new("b", "c", "sb", SignalControlInput::Group("g2".to_owned())),
        ],
    )
    .expect("signals");
    let (profiles, profile) = profiles();
    let traffic = InitialTrafficData::try_new_with_signals(
        graph,
        [Route::try_new("route", ["a", "b", "c"]).expect("route")],
        profiles,
        signals,
    )
    .expect("traffic");
    let mut world = CoreWorld::with_traffic_data(
        1_000,
        traffic,
        vec![VehicleSpawnInput::active(
            "vehicle",
            profile,
            "route",
            0,
            EdgeProgress::ZERO,
            speed(20.0),
        )],
    )
    .expect("world");

    let result = world.step(TickInput::new(1_000)).expect("step");
    let vehicle = vehicle(&world, "vehicle");

    assert_eq!(vehicle.route_edge_index, 1);
    assert_eq!(vehicle.edge_progress, progress(5.0));
    assert_eq!(vehicle.current_speed, Speed::ZERO);
    std::assert_matches!(
        result.events.as_slice(),
        [
            CoreEvent::VehicleSignalStopProjectionApplied(_),
            CoreEvent::VehicleChangedEdge(event)
        ] if event.from_route_edge_index == 0 && event.to_route_edge_index == 1
    );
}

#[test]
fn repeated_physical_edge_is_checked_by_route_occurrence() {
    let graph = LaneGraph::try_new([
        LaneEdge::new(
            "a",
            edge_length(5.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["a", "b"],
        ),
        LaneEdge::new(
            "b",
            edge_length(100.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            Vec::<String>::new(),
        ),
    ])
    .expect("graph");
    let signals = SignalRegistry::try_new(
        &graph,
        [StopLine::new("stop", "a", StopLineLocation::EdgeEnd)],
        [SignalGroup::new("loop"), SignalGroup::new("exit")],
        [SignalController::new_fixed_time(
            "controller",
            0,
            ["loop", "exit"],
            [phase(
                "phase",
                60_000,
                &[("loop", SignalAspect::Green), ("exit", SignalAspect::Red)],
            )],
        )],
        [
            MovementGate::new(
                "a",
                "a",
                "stop",
                SignalControlInput::Group("loop".to_owned()),
            ),
            MovementGate::new(
                "a",
                "b",
                "stop",
                SignalControlInput::Group("exit".to_owned()),
            ),
        ],
    )
    .expect("signals");
    let (profiles, profile) = profiles();
    let traffic = InitialTrafficData::try_new_with_signals(
        graph,
        [Route::try_new("route", ["a", "a", "b"]).expect("route")],
        profiles,
        signals,
    )
    .expect("traffic");
    let mut world = CoreWorld::with_traffic_data(
        1_000,
        traffic,
        vec![VehicleSpawnInput::active(
            "vehicle",
            profile,
            "route",
            0,
            EdgeProgress::ZERO,
            speed(20.0),
        )],
    )
    .expect("world");

    let result = world.step(TickInput::new(1_000)).expect("step");
    let vehicle = vehicle(&world, "vehicle");

    assert_eq!(vehicle.route_edge_index, 1);
    assert_eq!(vehicle.edge_progress, progress(5.0));
    std::assert_matches!(
        result.events.as_slice(),
        [
            CoreEvent::VehicleSignalStopProjectionApplied(projection),
            CoreEvent::VehicleChangedEdge(changed)
        ] if projection.from_route_edge_index == 1
            && projection.to_route_edge_index == 2
            && changed.from_edge == changed.to_edge
            && changed.from_route_edge_index == 0
            && changed.to_route_edge_index == 1
    );
}

#[test]
fn queue_releases_naturally_after_green_without_release_event() {
    let mut world = single_gate_world(
        vec![
            phase("red", 1_000, &[("main", SignalAspect::Red)]),
            phase("green", 60_000, &[("main", SignalAspect::Green)]),
        ],
        |profile| {
            vec![
                VehicleSpawnInput::active(
                    "follower",
                    profile,
                    "route",
                    0,
                    progress(93.0),
                    Speed::ZERO,
                ),
                VehicleSpawnInput::active(
                    "leader",
                    profile,
                    "route",
                    0,
                    progress(99.0),
                    Speed::ZERO,
                ),
            ]
        },
    );

    world.step(TickInput::new(1_000)).expect("red tick");
    assert_eq!(vehicle(&world, "leader").route_edge_index, 0);
    assert_eq!(vehicle(&world, "follower").route_edge_index, 0);

    let mut release_events = Vec::new();
    for _ in 0..5 {
        release_events.extend(
            world
                .step(TickInput::new(1_000))
                .expect("green tick")
                .events,
        );
    }
    assert_eq!(vehicle(&world, "leader").route_edge_index, 1);
    assert_eq!(vehicle(&world, "follower").route_edge_index, 1);
    assert!(
        release_events
            .iter()
            .all(|event| !matches!(event, CoreEvent::VehicleSignalStopProjectionApplied(_)))
    );
}

#[test]
fn signal_then_following_projection_order_is_per_vehicle() {
    let mut world = single_gate_world(
        vec![phase("red", 60_000, &[("main", SignalAspect::Red)])],
        |profile| {
            vec![
                VehicleSpawnInput::active(
                    "follower",
                    profile,
                    "route",
                    0,
                    progress(90.0),
                    speed(20.0),
                ),
                VehicleSpawnInput::active(
                    "leader",
                    profile,
                    "route",
                    0,
                    progress(96.0),
                    speed(20.0),
                ),
            ]
        },
    );
    let follower = world.vehicle_handle("follower").expect("follower");

    let result = world.step(TickInput::new(1_000)).expect("step");

    std::assert_matches!(
        result.events.as_slice(),
        [
            CoreEvent::VehicleSignalStopProjectionApplied(signal),
            CoreEvent::VehicleFollowingSafetyProjectionApplied(following),
            CoreEvent::VehicleSignalStopProjectionApplied(_)
        ] if signal.vehicle == follower && following.vehicle == follower
    );
}

#[test]
fn signal_beyond_finite_route_distance_horizon_is_ignored() {
    let graph = LaneGraph::try_new([
        LaneEdge::new(
            "a",
            edge_length(f64::MAX),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["b"],
        ),
        LaneEdge::new(
            "b",
            edge_length(f64::MAX),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["c"],
        ),
        LaneEdge::new(
            "c",
            edge_length(100.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            Vec::<String>::new(),
        ),
    ])
    .expect("graph");
    let signals = SignalRegistry::try_new(
        &graph,
        [StopLine::new("stop", "b", StopLineLocation::EdgeEnd)],
        [SignalGroup::new("main")],
        [SignalController::new_fixed_time(
            "controller",
            0,
            ["main"],
            [phase("red", 60_000, &[("main", SignalAspect::Red)])],
        )],
        [MovementGate::new(
            "b",
            "c",
            "stop",
            SignalControlInput::Group("main".to_owned()),
        )],
    )
    .expect("signals");
    let (profiles, profile) = profiles();
    let traffic = InitialTrafficData::try_new_with_signals(
        graph,
        [Route::try_new("route", ["a", "b", "c"]).expect("route")],
        profiles,
        signals,
    )
    .expect("traffic");
    let mut world = CoreWorld::with_traffic_data(
        1_000,
        traffic,
        vec![VehicleSpawnInput::active(
            "vehicle",
            profile,
            "route",
            0,
            EdgeProgress::ZERO,
            Speed::ZERO,
        )],
    )
    .expect("world");
    let result = world
        .step(TickInput::new(1_000))
        .expect("signal beyond every finite horizon must not fail the tick");

    assert!(result.events.is_empty());
    assert_eq!(world.tick_index(), 1);
}
