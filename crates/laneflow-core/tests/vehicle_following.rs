use laneflow_core::{
    Acceleration, CoreEvent, CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec,
    InitialTrafficData, LaneEdge, LaneGraph, Route, Speed, TickInput, VehicleProfile,
    VehicleProfileHandle, VehicleProfileRegistry, VehicleSpawnInput, VehicleState, VehicleStatus,
};

const EPSILON: f64 = 1.0e-9;

fn edge_length(value: f64) -> EdgeLength {
    EdgeLength::try_new(value).expect("valid edge length")
}

fn progress(value: f64) -> EdgeProgress {
    EdgeProgress::try_new(value).expect("valid progress")
}

fn speed(value: f64) -> Speed {
    Speed::try_new(value).expect("valid speed")
}

fn profile(
    desired_speed: f64,
    comfortable_deceleration: f64,
    emergency_deceleration: f64,
) -> (VehicleProfileRegistry, VehicleProfileHandle) {
    let registry = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        "following-profile",
        IidmProfileSpec {
            length: 4.0,
            desired_speed,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 2.0,
            comfortable_deceleration,
            emergency_deceleration,
        },
    )
    .expect("valid following profile")])
    .expect("valid profile registry");
    let handle = registry
        .profile_handle("following-profile")
        .expect("profile handle exists");
    (registry, handle)
}

fn single_edge_world(
    desired_speed: f64,
    vehicles: impl FnOnce(VehicleProfileHandle) -> Vec<VehicleSpawnInput>,
) -> CoreWorld {
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "E",
        edge_length(200.0),
        std::iter::empty::<&str>(),
    )])
    .expect("valid graph");
    let route = Route::try_new("R", ["E"]).expect("valid route");
    let (profiles, handle) = profile(desired_speed, 2.0, 8.0);
    let traffic_data =
        InitialTrafficData::try_new(lane_graph, [route], profiles).expect("valid traffic data");
    CoreWorld::with_traffic_data(1_000, traffic_data, vehicles(handle)).expect("valid world")
}

fn vehicle<'a>(world: &'a CoreWorld, id: &str) -> &'a VehicleState {
    let handle = world.vehicle_handle(id).expect("vehicle handle exists");
    world.vehicle(handle).expect("vehicle state exists")
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= EPSILON,
        "actual={actual}, expected={expected}"
    );
}

#[test]
fn free_road_iidm_accelerates_toward_desired_speed() {
    let mut world = single_edge_world(20.0, |profile| {
        vec![VehicleSpawnInput::active(
            "V1",
            profile,
            "R",
            0,
            progress(10.0),
            speed(10.0),
        )]
    });

    let result = world.step(TickInput::new(1_000)).expect("step succeeds");
    let vehicle = vehicle(&world, "V1");

    assert!(result.events.is_empty());
    assert!(vehicle.current_speed.value() > 10.0);
    assert!(vehicle.applied_acceleration.value() > 0.0);
    assert!(vehicle.edge_progress.value() > 20.0);
}

#[test]
fn ballistic_integration_stops_inside_tick_without_projection() {
    let mut world = single_edge_world(0.1, |profile| {
        vec![VehicleSpawnInput::active(
            "V1",
            profile,
            "R",
            0,
            EdgeProgress::ZERO,
            speed(1.0),
        )]
    });

    let result = world.step(TickInput::new(1_000)).expect("step succeeds");
    let vehicle = vehicle(&world, "V1");

    assert!(result.events.is_empty());
    assert_eq!(vehicle.status, VehicleStatus::Active);
    assert_eq!(vehicle.current_speed, Speed::ZERO);
    assert!(vehicle.edge_progress.value() > 0.24);
    assert!(vehicle.edge_progress.value() < 0.26);
    assert_eq!(
        vehicle.applied_acceleration,
        Acceleration::try_new(-1.0).unwrap()
    );
}

#[test]
fn impossible_emergency_gap_projects_to_no_overlap_and_emits_once() {
    let mut world = single_edge_world(20.0, |profile| {
        vec![
            VehicleSpawnInput::active("follower", profile, "R", 0, progress(5.0), speed(20.0)),
            VehicleSpawnInput::stopped("leader", profile, "R", 0, progress(10.0)),
        ]
    });
    let follower_handle = world.vehicle_handle("follower").unwrap();
    let leader_handle = world.vehicle_handle("leader").unwrap();

    let result = world.step(TickInput::new(1_000)).expect("step succeeds");
    let follower = vehicle(&world, "follower");

    assert_eq!(
        result.events,
        [CoreEvent::VehicleFollowingSafetyProjectionApplied(
            laneflow_core::VehicleFollowingSafetyProjectionAppliedEvent {
                tick_index: 1,
                vehicle: follower_handle,
                leader: leader_handle,
            }
        )]
    );
    assert_eq!(follower.status, VehicleStatus::Active);
    assert_eq!(follower.current_speed, Speed::ZERO);
    assert_close(follower.applied_acceleration.value(), -20.0);
    assert_close(follower.edge_progress.value(), 6.0);
    assert_close(
        vehicle(&world, "leader").edge_progress.value() - follower.edge_progress.value() - 4.0,
        0.0,
    );
}

#[test]
fn emergency_envelope_can_stop_at_geometry_cap_without_projection_event() {
    let mut world = single_edge_world(20.0, |profile| {
        vec![
            VehicleSpawnInput::active("follower", profile, "R", 0, progress(5.0), speed(20.0)),
            VehicleSpawnInput::stopped("leader", profile, "R", 0, progress(25.0)),
        ]
    });

    let result = world.step(TickInput::new(1_000)).expect("step succeeds");
    let follower = vehicle(&world, "follower");

    assert!(result.events.is_empty());
    assert_eq!(follower.status, VehicleStatus::Active);
    assert_close(follower.current_speed.value(), 12.0);
    assert_close(follower.applied_acceleration.value(), -8.0);
    assert_close(follower.edge_progress.value(), 21.0);
    assert_close(
        vehicle(&world, "leader").edge_progress.value() - follower.edge_progress.value() - 4.0,
        0.0,
    );
}

#[test]
fn acyclic_platoon_projects_front_to_back_without_overlap() {
    let mut world = single_edge_world(20.0, |profile| {
        vec![
            VehicleSpawnInput::active("V1", profile, "R", 0, progress(5.0), speed(20.0)),
            VehicleSpawnInput::active("V2", profile, "R", 0, progress(10.0), speed(20.0)),
            VehicleSpawnInput::active("V3", profile, "R", 0, progress(15.0), speed(20.0)),
            VehicleSpawnInput::stopped("V4", profile, "R", 0, progress(20.0)),
        ]
    });

    world.step(TickInput::new(1_000)).expect("step succeeds");

    let fronts = ["V1", "V2", "V3", "V4"].map(|id| vehicle(&world, id).edge_progress.value());
    assert_eq!(fronts, [8.0, 12.0, 16.0, 20.0]);
    for pair in fronts.windows(2) {
        assert!(pair[1] - pair[0] - 4.0 >= -EPSILON);
    }
    for id in ["V1", "V2", "V3"] {
        assert_eq!(vehicle(&world, id).status, VehicleStatus::Active);
    }
}

#[test]
fn safety_projection_event_precedes_actual_edge_transition() {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new("A", edge_length(5.0), ["B"]),
        LaneEdge::new("B", edge_length(100.0), std::iter::empty::<&str>()),
    ])
    .expect("valid graph");
    let route = Route::try_new("R", ["A", "B"]).expect("valid route");
    let (profiles, profile) = profile(20.0, 2.0, 8.0);
    let traffic_data =
        InitialTrafficData::try_new(lane_graph, [route], profiles).expect("valid traffic data");
    let mut world = CoreWorld::with_traffic_data(
        1_000,
        traffic_data,
        vec![
            VehicleSpawnInput::active("follower", profile, "R", 0, progress(4.0), speed(20.0)),
            VehicleSpawnInput::stopped("leader", profile, "R", 1, progress(4.0)),
        ],
    )
    .expect("valid world");

    let result = world.step(TickInput::new(1_000)).expect("step succeeds");

    assert!(matches!(
        result.events[0],
        CoreEvent::VehicleFollowingSafetyProjectionApplied(_)
    ));
    assert!(matches!(result.events[1], CoreEvent::VehicleChangedEdge(_)));
    assert_eq!(vehicle(&world, "follower").route_edge_index, 1);
    assert_eq!(
        vehicle(&world, "follower").edge_progress,
        EdgeProgress::ZERO
    );
}

#[test]
fn leader_route_completion_uses_actual_terminal_travel_for_projection() {
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "E",
        edge_length(20.0),
        std::iter::empty::<&str>(),
    )])
    .expect("valid graph");
    let route = Route::try_new("R", ["E"]).expect("valid route");
    let (profiles, profile) = profile(20.0, 2.0, 8.0);
    let traffic_data =
        InitialTrafficData::try_new(lane_graph, [route], profiles).expect("valid traffic data");
    let mut world = CoreWorld::with_traffic_data(
        1_000,
        traffic_data,
        vec![
            VehicleSpawnInput::active("follower", profile, "R", 0, progress(14.0), speed(10.0)),
            VehicleSpawnInput::active("leader", profile, "R", 0, progress(19.0), speed(10.0)),
        ],
    )
    .expect("valid world");

    world.step(TickInput::new(1_000)).expect("step succeeds");
    let follower = vehicle(&world, "follower");
    let leader = vehicle(&world, "leader");

    assert_eq!(leader.status, VehicleStatus::Completed);
    assert_close(leader.edge_progress.value(), 20.0);
    assert_close(follower.edge_progress.value(), 16.0);
    assert_close(
        leader.edge_progress.value() - follower.edge_progress.value() - 4.0,
        0.0,
    );
}

#[test]
fn repeated_edge_cycle_is_deterministic_across_input_order() {
    fn world(reverse: bool) -> CoreWorld {
        let lane_graph = LaneGraph::try_new([LaneEdge::new("E", edge_length(100.0), ["E"])])
            .expect("valid graph");
        let route = Route::try_new("R", ["E", "E"]).expect("valid repeated route");
        let (profiles, profile) = profile(20.0, 2.0, 8.0);
        let traffic_data =
            InitialTrafficData::try_new(lane_graph, [route], profiles).expect("valid traffic data");
        let first = VehicleSpawnInput::active("A", profile, "R", 0, progress(20.0), speed(10.0));
        let second = VehicleSpawnInput::active("B", profile, "R", 0, progress(70.0), speed(10.0));
        let vehicles = if reverse {
            vec![second, first]
        } else {
            vec![first, second]
        };
        CoreWorld::with_traffic_data(1_000, traffic_data, vehicles).expect("valid world")
    }

    let mut first = world(false);
    let mut second = world(true);

    let first_result = first.step(TickInput::new(1_000)).expect("step succeeds");
    let second_result = second.step(TickInput::new(1_000)).expect("step succeeds");

    assert_eq!(first_result, second_result);
    assert_eq!(first, second);
}

#[test]
fn despawned_leader_leaves_snapshot_and_reused_slot_identifies_replacement() {
    let mut world = single_edge_world(20.0, |profile| {
        vec![
            VehicleSpawnInput::active("follower", profile, "R", 0, progress(5.0), speed(8.0)),
            VehicleSpawnInput::stopped("leader", profile, "R", 0, progress(20.0)),
        ]
    });
    let stale_leader = world.vehicle_handle("leader").expect("leader exists");

    world.step(TickInput::new(1_000)).expect("step succeeds");
    let constrained_speed = vehicle(&world, "follower").current_speed.value();

    world
        .despawn_vehicle(stale_leader)
        .expect("leader despawn succeeds");
    let free_result = world
        .step(TickInput::new(1_000))
        .expect("free step succeeds");
    assert!(free_result.events.is_empty());
    assert!(vehicle(&world, "follower").current_speed.value() > constrained_speed);
    assert!(world.vehicle(stale_leader).is_none());

    let profile = world
        .vehicle_profile_handle("following-profile")
        .expect("profile exists");
    let follower_front = vehicle(&world, "follower").edge_progress.value();
    let replacement = world
        .spawn_vehicle(VehicleSpawnInput::stopped(
            "replacement",
            profile,
            "R",
            0,
            progress(follower_front + 4.0),
        ))
        .expect("replacement leader spawn succeeds");

    assert_ne!(replacement, stale_leader);
    assert!(world.vehicle(stale_leader).is_none());

    let result = world.step(TickInput::new(1_000)).expect("step succeeds");
    assert!(result.events.iter().any(|event| {
        matches!(
            event,
            CoreEvent::VehicleFollowingSafetyProjectionApplied(event)
                if event.leader == replacement
        )
    }));
    assert_close(
        vehicle(&world, "replacement").edge_progress.value()
            - vehicle(&world, "follower").edge_progress.value()
            - 4.0,
        0.0,
    );
}
