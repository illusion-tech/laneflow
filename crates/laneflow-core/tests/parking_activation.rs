use laneflow_core::{
    CoreEvent, CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge,
    LaneGraph, MovementGate, ParkingApproachState, ParkingRegistry, ParkingSpace,
    ParkingSpaceGeometry, Route, SignalAspect, SignalControlInput, SignalController, SignalGroup,
    SignalGroupState, SignalPhase, SignalRegistry, Speed, StopLine, StopLineLocation, TickInput,
    VehicleParkingState, VehicleProfile, VehicleProfileHandle, VehicleProfileRegistry,
    VehicleSpawnInput,
};

const CURRENT_LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS: f64 = 1.0e-9;

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
    .expect("profile")])
    .expect("profiles");
    let profile = registry.profile_handle("car").expect("profile handle");
    (registry, profile)
}

fn progress(value: f64) -> EdgeProgress {
    EdgeProgress::try_new(value).expect("progress")
}

fn speed(value: f64) -> Speed {
    Speed::try_new(value).expect("speed")
}

fn signal_parking_world(
    parking_entry_edge: &str,
    parking_entry_progress: f64,
    vehicle_progress: f64,
) -> CoreWorld {
    let graph = LaneGraph::try_new([
        LaneEdge::new(
            "entry",
            EdgeLength::try_new(100.0).expect("entry length"),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["exit"],
        ),
        LaneEdge::new(
            "exit",
            EdgeLength::try_new(100.0).expect("exit length"),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("graph");
    let signals = SignalRegistry::try_new(
        &graph,
        [StopLine::new("stop", "entry", StopLineLocation::EdgeEnd)],
        [SignalGroup::new("main")],
        [SignalController::new_fixed_time(
            "controller",
            0,
            ["main"],
            [SignalPhase::new(
                "red",
                60_000,
                [SignalGroupState::new("main", SignalAspect::Red)],
            )],
        )],
        [MovementGate::new(
            "entry",
            "exit",
            "stop",
            SignalControlInput::Group("main".to_owned()),
        )],
    )
    .expect("signals");
    let parking = ParkingRegistry::try_new(
        &graph,
        [],
        [ParkingSpace::new(
            "space",
            None,
            parking_entry_edge,
            parking_entry_progress,
            "exit",
            20.0,
            ParkingSpaceGeometry::new(-3.0, 0.0, 4.0, 2.4),
        )],
    )
    .expect("parking");
    let (profiles, profile) = profiles();
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        graph,
        [Route::try_new("route", ["entry", "exit"]).expect("route")],
        profiles,
        signals,
        parking,
    )
    .expect("traffic");
    CoreWorld::with_traffic_data(
        1_000,
        traffic,
        vec![VehicleSpawnInput::active(
            "vehicle",
            profile,
            "route",
            0,
            progress(vehicle_progress),
            speed(20.0),
        )],
    )
    .expect("world")
}

fn reserve(world: &mut CoreWorld) {
    let vehicle = world.vehicle_handle("vehicle").expect("vehicle");
    let space = world.parking().space_handle("space").expect("space");
    world
        .reserve_parking_space(vehicle, space)
        .expect("reservation");
}

#[test]
fn parking_boundary_outside_signal_epsilon_remains_stricter() {
    let mut world = signal_parking_world(
        "entry",
        100.0 - 2.0 * CURRENT_LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS,
        95.0,
    );
    reserve(&mut world);

    let result = world.step(TickInput::new(1_000)).expect("epsilon step");

    assert!(matches!(
        result.events.as_slice(),
        [
            CoreEvent::VehicleParkingStopProjectionApplied(_),
            CoreEvent::VehicleParkingArrivalReached(_)
        ]
    ));
    assert!(
        result
            .events
            .iter()
            .all(|event| !matches!(event, CoreEvent::VehicleSignalStopProjectionApplied(_)))
    );
}

#[test]
fn parking_before_signal_attributes_parking_and_arrives() {
    let mut world = signal_parking_world("entry", 90.0, 85.0);
    reserve(&mut world);

    let result = world
        .step(TickInput::new(1_000))
        .expect("parking-first step");

    assert!(matches!(
        result.events.as_slice(),
        [
            CoreEvent::VehicleParkingStopProjectionApplied(_),
            CoreEvent::VehicleParkingArrivalReached(_)
        ]
    ));
}

#[test]
fn signal_before_parking_stops_at_gate_without_arrival() {
    let mut world = signal_parking_world("exit", 10.0, 95.0);
    reserve(&mut world);
    let vehicle = world.vehicle_handle("vehicle").expect("vehicle");
    let space = world.parking().space_handle("space").expect("space");

    let result = world
        .step(TickInput::new(1_000))
        .expect("signal-first step");

    assert!(matches!(
        result.events.as_slice(),
        [CoreEvent::VehicleSignalStopProjectionApplied(_)]
    ));
    assert_eq!(
        world.parking_snapshot().vehicle_state(vehicle),
        Some(VehicleParkingState::Reserved {
            space,
            approach: ParkingApproachState::Approaching {
                route: world.route_handle("route").expect("route"),
                route_edge_index: 1,
            },
        })
    );
}

#[test]
fn parking_projection_precedes_stricter_following_projection() {
    let graph = LaneGraph::try_new([LaneEdge::new(
        "edge",
        EdgeLength::try_new(100.0).expect("edge length"),
        laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
        std::iter::empty::<&str>(),
    )])
    .expect("graph");
    let parking = ParkingRegistry::try_new(
        &graph,
        [],
        [ParkingSpace::new(
            "space",
            None,
            "edge",
            10.0,
            "edge",
            20.0,
            ParkingSpaceGeometry::new(-3.0, 0.0, 4.0, 2.4),
        )],
    )
    .expect("parking");
    let (profiles, profile) = profiles();
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        graph,
        [Route::try_new("route", ["edge"]).expect("route")],
        profiles,
        SignalRegistry::empty(),
        parking,
    )
    .expect("traffic");
    let mut world = CoreWorld::with_traffic_data(
        1_000,
        traffic,
        vec![
            VehicleSpawnInput::active(
                "follower",
                profile,
                "route",
                0,
                EdgeProgress::ZERO,
                speed(20.0),
            ),
            VehicleSpawnInput::stopped("leader", profile, "route", 0, progress(8.0)),
        ],
    )
    .expect("world");
    let follower = world.vehicle_handle("follower").expect("follower");
    let space = world.parking().space_handle("space").expect("space");
    world
        .reserve_parking_space(follower, space)
        .expect("reservation");

    let result = world.step(TickInput::new(1_000)).expect("dual projection");

    assert!(matches!(
        result.events.as_slice(),
        [
            CoreEvent::VehicleParkingStopProjectionApplied(_),
            CoreEvent::VehicleFollowingSafetyProjectionApplied(_)
        ]
    ));
    assert!(
        world
            .vehicle(follower)
            .expect("follower state")
            .edge_progress
            .value()
            < 10.0
    );
    assert!(matches!(
        world.parking_snapshot().vehicle_state(follower),
        Some(VehicleParkingState::Reserved {
            approach: ParkingApproachState::Approaching { .. },
            ..
        })
    ));
}

#[test]
fn repeated_edge_uses_selected_occurrence_and_orders_edge_before_arrival() {
    let graph = LaneGraph::try_new([
        LaneEdge::new(
            "a",
            EdgeLength::try_new(100.0).expect("a length"),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["b"],
        ),
        LaneEdge::new(
            "b",
            EdgeLength::try_new(100.0).expect("b length"),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["a"],
        ),
    ])
    .expect("graph");
    let parking = ParkingRegistry::try_new(
        &graph,
        [],
        [ParkingSpace::new(
            "space",
            None,
            "a",
            10.0,
            "a",
            20.0,
            ParkingSpaceGeometry::new(-3.0, 0.0, 4.0, 2.4),
        )],
    )
    .expect("parking");
    let (profiles, profile) = profiles();
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        graph,
        [Route::try_new("route", ["a", "b", "a"]).expect("route")],
        profiles,
        SignalRegistry::empty(),
        parking,
    )
    .expect("traffic");
    let mut world = CoreWorld::with_traffic_data(
        1_000,
        traffic,
        vec![VehicleSpawnInput::active(
            "vehicle",
            profile,
            "route",
            1,
            progress(95.0),
            speed(20.0),
        )],
    )
    .expect("world");
    reserve(&mut world);

    let result = world
        .step(TickInput::new(1_000))
        .expect("repeated edge step");

    assert!(matches!(
        result.events.as_slice(),
        [
            CoreEvent::VehicleParkingStopProjectionApplied(_),
            CoreEvent::VehicleChangedEdge(_),
            CoreEvent::VehicleParkingArrivalReached(_)
        ]
    ));
    let state = world
        .vehicle(world.vehicle_handle("vehicle").expect("vehicle"))
        .expect("state");
    assert_eq!(state.route_edge_index, 2);
    assert_eq!(state.edge_progress, progress(10.0));
    assert_eq!(state.current_speed, Speed::ZERO);
}
