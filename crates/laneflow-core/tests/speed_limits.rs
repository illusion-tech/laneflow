use laneflow_core::{
    CoreError, CoreEvent, CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData,
    LaneEdge, LaneGraph, Route, Speed, SpeedLimit, TickInput, VehicleProfile, VehicleProfileHandle,
    VehicleProfileRegistry, VehicleSpawnInput,
};

const MAIN_LIMIT: f64 = 60.0 / 3.6;
const SIDE_LIMIT: f64 = 40.0 / 3.6;

fn edge(id: &str, length: f64, limit: f64, next: &[&str]) -> LaneEdge {
    LaneEdge::new(
        id,
        EdgeLength::try_new(length).expect("edge length"),
        SpeedLimit::try_new(limit).expect("speed limit"),
        next.iter().copied(),
    )
}

fn profile() -> (VehicleProfileRegistry, VehicleProfileHandle) {
    let profiles = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        "speed-limit-profile",
        IidmProfileSpec {
            length: 4.0,
            desired_speed: 30.0,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 2.0,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 4.0,
        },
    )
    .expect("profile")])
    .expect("profile registry");
    let handle = profiles
        .profile_handle("speed-limit-profile")
        .expect("profile handle");
    (profiles, handle)
}

fn world(
    fixed_delta_time_ms: u64,
    edges: impl IntoIterator<Item = LaneEdge>,
    route_edges: &[&str],
    vehicle: impl FnOnce(VehicleProfileHandle) -> Vec<VehicleSpawnInput>,
) -> CoreWorld {
    let graph = LaneGraph::try_new(edges).expect("graph");
    let route = Route::try_new("R", route_edges.iter().copied()).expect("route");
    let (profiles, profile) = profile();
    let traffic = InitialTrafficData::try_new(graph, [route], profiles).expect("traffic");
    CoreWorld::with_traffic_data(fixed_delta_time_ms, traffic, vehicle(profile)).expect("world")
}

fn vehicle_speed(world: &CoreWorld) -> f64 {
    let handle = world.vehicle_handle("V").expect("vehicle handle");
    world
        .vehicle(handle)
        .expect("vehicle")
        .current_speed
        .value()
}

#[test]
fn current_edge_ceiling_prevents_discrete_tick_overshoot() {
    let mut world = world(1_000, [edge("A", 1_000.0, 10.0, &[])], &["A"], |profile| {
        vec![VehicleSpawnInput::active(
            "V",
            profile,
            "R",
            0,
            EdgeProgress::ZERO,
            Speed::try_new(9.9).expect("speed"),
        )]
    });

    world.step(TickInput::new(1_000)).expect("step");

    assert!(vehicle_speed(&world) > 9.9);
    assert!(vehicle_speed(&world) <= 10.0);
}

#[test]
fn spawn_above_current_edge_limit_is_atomic() {
    let mut world = world(1_000, [edge("A", 100.0, 10.0, &[])], &["A"], |_| Vec::new());
    let profile = world
        .vehicle_profile_handle("speed-limit-profile")
        .expect("profile handle");
    let before = world.clone();

    let error = world
        .spawn_vehicle(VehicleSpawnInput::active(
            "V",
            profile,
            "R",
            0,
            EdgeProgress::ZERO,
            Speed::try_new(10.0_f64.next_up()).expect("speed"),
        ))
        .expect_err("over-limit spawn must fail");

    std::assert_matches!(
        error,
        CoreError::VehicleInitialSpeedExceedsLimit {
            vehicle_id,
            edge_id,
            initial_speed,
            speed_limit,
        } if vehicle_id == "V"
            && edge_id == "A"
            && initial_speed == 10.0_f64.next_up()
            && speed_limit == 10.0
    );
    assert_eq!(world, before);
}

#[test]
fn sixty_to_forty_brakes_before_boundary_without_projection() {
    let mut world = world(
        500,
        [
            edge("A", 100.0, MAIN_LIMIT, &["B"]),
            edge("B", 500.0, SIDE_LIMIT, &[]),
        ],
        &["A", "B"],
        |profile| {
            vec![VehicleSpawnInput::active(
                "V",
                profile,
                "R",
                0,
                EdgeProgress::try_new(50.0).expect("progress"),
                Speed::try_new(16.0).expect("speed"),
            )]
        },
    );
    let vehicle = world.vehicle_handle("V").expect("vehicle");
    let mut crossed = false;

    for _ in 0..20 {
        let result = world.step(TickInput::new(500)).expect("step");
        let state = world.vehicle(vehicle).expect("vehicle");
        let edge = world.route_edges(state.route).expect("route")[state.route_edge_index];
        let limit = world
            .lane_graph()
            .edge_speed_limit(edge)
            .expect("limit")
            .value();
        assert!(state.current_speed.value() <= limit);
        assert!(
            result
                .events
                .iter()
                .all(|event| !matches!(event, CoreEvent::VehicleSpeedLimitProjectionApplied(_)))
        );
        if state.route_edge_index == 1 {
            crossed = true;
            assert!(state.current_speed.value() <= SIDE_LIMIT);
            break;
        }
    }

    assert!(crossed, "vehicle must cross the lower-limit boundary");
}

#[test]
fn forty_to_sixty_waits_until_next_tick_to_accelerate() {
    let mut world = world(
        500,
        [
            edge("A", 10.0, SIDE_LIMIT, &["B"]),
            edge("B", 100.0, MAIN_LIMIT, &[]),
        ],
        &["A", "B"],
        |profile| {
            vec![VehicleSpawnInput::active(
                "V",
                profile,
                "R",
                0,
                EdgeProgress::ZERO,
                Speed::try_new(SIDE_LIMIT).expect("speed"),
            )]
        },
    );
    let vehicle = world.vehicle_handle("V").expect("vehicle");

    loop {
        world.step(TickInput::new(500)).expect("step");
        let state = world.vehicle(vehicle).expect("vehicle");
        if state.route_edge_index == 1 {
            assert_eq!(state.current_speed.value(), SIDE_LIMIT);
            break;
        }
        assert_eq!(state.current_speed.value(), SIDE_LIMIT);
    }

    world
        .step(TickInput::new(500))
        .expect("first downstream step");
    assert!(vehicle_speed(&world) > SIDE_LIMIT);
}

#[test]
fn farther_sharper_drop_can_dominate_nearer_transition() {
    let build = |final_limit| {
        world(
            1_000,
            [
                edge("A", 100.0, 20.0, &["B"]),
                edge("B", 10.0, 15.0, &["C"]),
                edge("C", 100.0, final_limit, &[]),
            ],
            &["A", "B", "C"],
            |profile| {
                vec![VehicleSpawnInput::active(
                    "V",
                    profile,
                    "R",
                    0,
                    EdgeProgress::try_new(70.0).expect("progress"),
                    Speed::try_new(20.0).expect("speed"),
                )]
            },
        )
    };
    let mut only_near_drop = build(15.0);
    let mut sharper_far_drop = build(5.0);

    only_near_drop
        .step(TickInput::new(1_000))
        .expect("near step");
    sharper_far_drop
        .step(TickInput::new(1_000))
        .expect("far step");

    assert!(vehicle_speed(&sharper_far_drop) < vehicle_speed(&only_near_drop));
}

#[test]
fn vehicle_following_remains_stricter_than_a_downstream_speed_drop() {
    let mut world = world(
        1_000,
        [edge("A", 100.0, 20.0, &["B"]), edge("B", 100.0, 5.0, &[])],
        &["A", "B"],
        |profile| {
            vec![
                VehicleSpawnInput::active(
                    "V",
                    profile,
                    "R",
                    0,
                    EdgeProgress::try_new(80.0).expect("progress"),
                    Speed::try_new(20.0).expect("speed"),
                ),
                VehicleSpawnInput::stopped(
                    "leader",
                    profile,
                    "R",
                    0,
                    EdgeProgress::try_new(90.0).expect("progress"),
                ),
            ]
        },
    );

    world.step(TickInput::new(1_000)).expect("step");

    let follower = world
        .vehicle(world.vehicle_handle("V").expect("follower"))
        .expect("follower state");
    let leader = world
        .vehicle(world.vehicle_handle("leader").expect("leader"))
        .expect("leader state");
    assert_eq!(follower.route_edge_index, 0);
    assert!(
        leader.edge_progress.value() - follower.edge_progress.value() >= 4.0,
        "following projection must preserve non-overlap"
    );
    assert!(follower.current_speed.value() <= 20.0);
}

#[test]
fn infeasible_crossing_projects_once_and_attributes_repeated_occurrence() {
    let mut world = world(
        1_000,
        [
            edge("A", 100.0, 20.0, &["B"]),
            edge("B", 100.0, 5.0, &["A"]),
        ],
        &["A", "B", "A", "B"],
        |profile| {
            vec![VehicleSpawnInput::active(
                "V",
                profile,
                "R",
                2,
                EdgeProgress::try_new(99.0).expect("progress"),
                Speed::try_new(20.0).expect("speed"),
            )]
        },
    );
    let vehicle = world.vehicle_handle("V").expect("vehicle");
    let route = world.route_handle("R").expect("route");
    let from_edge = world.edge_handle("A").expect("A");
    let to_edge = world.edge_handle("B").expect("B");

    let result = world.step(TickInput::new(1_000)).expect("step");
    let state = world.vehicle(vehicle).expect("vehicle");

    assert_eq!(state.route_edge_index, 3);
    assert_eq!(state.edge_progress, EdgeProgress::ZERO);
    assert!(state.current_speed.value() <= 5.0);
    assert_eq!(
        result.events,
        [
            CoreEvent::VehicleSpeedLimitProjectionApplied(
                laneflow_core::VehicleSpeedLimitProjectionAppliedEvent {
                    tick_index: 1,
                    vehicle,
                    route,
                    from_route_edge_index: 2,
                    to_route_edge_index: 3,
                    from_edge,
                    to_edge,
                },
            ),
            CoreEvent::VehicleChangedEdge(laneflow_core::VehicleChangedEdgeEvent {
                tick_index: 1,
                vehicle,
                route,
                from_edge,
                to_edge,
                from_route_edge_index: 2,
                to_route_edge_index: 3,
            }),
        ]
    );
}
