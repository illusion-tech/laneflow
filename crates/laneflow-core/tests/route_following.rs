mod common;

use common::world_with_test_profile;
use laneflow_core::{
    Acceleration, CoreError, CoreEvent, CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec,
    InitialTrafficData, LaneEdge, LaneGraph, Route, Speed, TickInput, VehicleProfile,
    VehicleProfileHandle, VehicleProfileRegistry, VehicleSpawnInput, VehicleState, VehicleStatus,
};

const CURRENT_EDGE_BOUNDARY_TOLERANCE_METERS: f64 = 1.0e-9;
const PROGRESS_ASSERTION_TOLERANCE_METERS: f64 = 1.0e-9;

#[derive(Debug, PartialEq, Eq)]
enum EventView {
    Changed {
        tick_index: u64,
        vehicle: String,
        route: String,
        from_edge: String,
        to_edge: String,
        from_route_edge_index: usize,
        to_route_edge_index: usize,
    },
    Completed {
        tick_index: u64,
        vehicle: String,
        route: String,
        edge: String,
        route_edge_index: usize,
    },
}

fn edge_length(value: f64) -> EdgeLength {
    EdgeLength::try_new(value).expect("valid edge length")
}

fn progress(value: f64) -> EdgeProgress {
    EdgeProgress::try_new(value).expect("valid progress")
}

fn speed(value: f64) -> Speed {
    Speed::try_new(value).expect("valid speed")
}

fn world_with_cruise_profile<I, F>(
    fixed_delta_time_ms: u64,
    desired_speed: f64,
    lane_graph: LaneGraph,
    routes: I,
    vehicle_inputs: F,
) -> Result<CoreWorld, CoreError>
where
    I: IntoIterator<Item = Route>,
    F: FnOnce(VehicleProfileHandle) -> Vec<VehicleSpawnInput>,
{
    let profiles = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        "route-test-profile",
        IidmProfileSpec {
            length: 4.5,
            desired_speed,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 1.4,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 4.0,
        },
    )?])?;
    let profile = profiles
        .profile_handle("route-test-profile")
        .expect("profile handle exists");
    let traffic_data = InitialTrafficData::try_new(lane_graph, routes, profiles)?;
    CoreWorld::with_traffic_data(fixed_delta_time_ms, traffic_data, vehicle_inputs(profile))
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= PROGRESS_ASSERTION_TOLERANCE_METERS,
        "actual={actual}, expected={expected}"
    );
}

fn canonical_world(
    desired_speed: f64,
    vehicle: impl FnOnce(VehicleProfileHandle) -> VehicleSpawnInput,
) -> CoreWorld {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new(
            "A",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["B"],
        ),
        LaneEdge::new(
            "B",
            edge_length(5.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A", "B"]).expect("valid route");

    world_with_cruise_profile(1_000, desired_speed, lane_graph, [route], |profile| {
        vec![vehicle(profile)]
    })
    .expect("valid world")
}

fn vehicle_by_id<'a>(world: &'a CoreWorld, id: &str) -> &'a VehicleState {
    let handle = world.vehicle_handle(id).expect("vehicle handle exists");
    world.vehicle(handle).expect("vehicle state exists")
}

fn event_views(world: &CoreWorld, events: &[CoreEvent]) -> Vec<EventView> {
    events
        .iter()
        .map(|event| match event {
            CoreEvent::VehicleChangedEdge(event) => EventView::Changed {
                tick_index: event.tick_index,
                vehicle: world
                    .vehicle_external_id(event.vehicle)
                    .expect("vehicle id exists")
                    .to_owned(),
                route: world
                    .route_external_id(event.route)
                    .expect("route id exists")
                    .to_owned(),
                from_edge: world
                    .edge_external_id(event.from_edge)
                    .expect("from edge id exists")
                    .to_owned(),
                to_edge: world
                    .edge_external_id(event.to_edge)
                    .expect("to edge id exists")
                    .to_owned(),
                from_route_edge_index: event.from_route_edge_index,
                to_route_edge_index: event.to_route_edge_index,
            },
            CoreEvent::VehicleCompletedRoute(event) => EventView::Completed {
                tick_index: event.tick_index,
                vehicle: world
                    .vehicle_external_id(event.vehicle)
                    .expect("vehicle id exists")
                    .to_owned(),
                route: world
                    .route_external_id(event.route)
                    .expect("route id exists")
                    .to_owned(),
                edge: world
                    .edge_external_id(event.edge)
                    .expect("edge id exists")
                    .to_owned(),
                route_edge_index: event.route_edge_index,
            },
            _ => unreachable!("route following tests only create route events"),
        })
        .collect()
}

#[test]
fn canonical_fixture_ticks_match_design() {
    let mut world = canonical_world(6.0, |profile| {
        VehicleSpawnInput::active("V1", profile, "R", 0, progress(0.0), speed(6.0))
    });

    let tick1 = world.step(TickInput::new(1_000)).expect("tick 1 succeeds");
    assert_eq!(tick1.tick_index, 1);
    assert_eq!(tick1.time_ms, 1_000);
    assert!(tick1.events.is_empty());
    let vehicle = vehicle_by_id(&world, "V1");
    assert_eq!(vehicle.status, VehicleStatus::Active);
    assert_eq!(vehicle.route_edge_index, 0);
    assert_close(vehicle.edge_progress.value(), 6.0);

    let tick2 = world.step(TickInput::new(1_000)).expect("tick 2 succeeds");
    assert_eq!(tick2.tick_index, 2);
    assert_eq!(tick2.time_ms, 2_000);
    assert_eq!(
        event_views(&world, &tick2.events),
        vec![EventView::Changed {
            tick_index: 2,
            vehicle: "V1".to_owned(),
            route: "R".to_owned(),
            from_edge: "A".to_owned(),
            to_edge: "B".to_owned(),
            from_route_edge_index: 0,
            to_route_edge_index: 1,
        }]
    );
    let vehicle = vehicle_by_id(&world, "V1");
    assert_eq!(vehicle.status, VehicleStatus::Active);
    assert_eq!(vehicle.route_edge_index, 1);
    assert_close(vehicle.edge_progress.value(), 2.0);

    let tick3 = world.step(TickInput::new(1_000)).expect("tick 3 succeeds");
    assert_eq!(tick3.tick_index, 3);
    assert_eq!(tick3.time_ms, 3_000);
    assert_eq!(
        event_views(&world, &tick3.events),
        vec![EventView::Completed {
            tick_index: 3,
            vehicle: "V1".to_owned(),
            route: "R".to_owned(),
            edge: "B".to_owned(),
            route_edge_index: 1,
        }]
    );
    let vehicle = vehicle_by_id(&world, "V1");
    assert_eq!(vehicle.status, VehicleStatus::Completed);
    assert_eq!(vehicle.route_edge_index, 1);
    assert_close(vehicle.edge_progress.value(), 5.0);
    assert_eq!(vehicle.current_speed, Speed::ZERO);
    assert_eq!(vehicle.applied_acceleration, Acceleration::ZERO);

    let tick4 = world.step(TickInput::new(1_000)).expect("tick 4 succeeds");
    assert_eq!(tick4.tick_index, 4);
    assert_eq!(tick4.time_ms, 4_000);
    assert!(tick4.events.is_empty());
    let vehicle = vehicle_by_id(&world, "V1");
    assert_eq!(vehicle.status, VehicleStatus::Completed);
    assert_eq!(vehicle.route_edge_index, 1);
    assert_close(vehicle.edge_progress.value(), 5.0);
}

#[test]
fn single_tick_can_cross_multiple_edges_in_route_order() {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new(
            "A",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["B"],
        ),
        LaneEdge::new(
            "B",
            edge_length(5.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["C"],
        ),
        LaneEdge::new(
            "C",
            edge_length(2.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A", "B", "C"]).expect("valid route");
    let mut world = world_with_cruise_profile(1_000, 30.0, lane_graph, [route], |profile| {
        vec![VehicleSpawnInput::active(
            "V1",
            profile,
            "R",
            0,
            progress(0.0),
            speed(30.0),
        )]
    })
    .expect("valid world");

    let result = world.step(TickInput::new(1_000)).expect("step succeeds");

    assert_eq!(
        event_views(&world, &result.events),
        vec![
            EventView::Changed {
                tick_index: 1,
                vehicle: "V1".to_owned(),
                route: "R".to_owned(),
                from_edge: "A".to_owned(),
                to_edge: "B".to_owned(),
                from_route_edge_index: 0,
                to_route_edge_index: 1,
            },
            EventView::Changed {
                tick_index: 1,
                vehicle: "V1".to_owned(),
                route: "R".to_owned(),
                from_edge: "B".to_owned(),
                to_edge: "C".to_owned(),
                from_route_edge_index: 1,
                to_route_edge_index: 2,
            },
            EventView::Completed {
                tick_index: 1,
                vehicle: "V1".to_owned(),
                route: "R".to_owned(),
                edge: "C".to_owned(),
                route_edge_index: 2,
            },
        ]
    );
    let vehicle = vehicle_by_id(&world, "V1");
    assert_eq!(vehicle.status, VehicleStatus::Completed);
    assert_eq!(vehicle.route_edge_index, 2);
    assert_close(vehicle.edge_progress.value(), 2.0);
}

#[test]
fn repeated_edge_route_uses_route_edge_index_to_disambiguate_position() {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new(
            "A",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["B"],
        ),
        LaneEdge::new(
            "B",
            edge_length(5.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["A"],
        ),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A", "B", "A"]).expect("valid repeated-edge route");
    let mut world = world_with_cruise_profile(1_000, 30.0, lane_graph, [route], |profile| {
        vec![VehicleSpawnInput::active(
            "V1",
            profile,
            "R",
            0,
            progress(0.0),
            speed(30.0),
        )]
    })
    .expect("valid world");

    let result = world.step(TickInput::new(1_000)).expect("step succeeds");

    assert_eq!(
        event_views(&world, &result.events),
        vec![
            EventView::Changed {
                tick_index: 1,
                vehicle: "V1".to_owned(),
                route: "R".to_owned(),
                from_edge: "A".to_owned(),
                to_edge: "B".to_owned(),
                from_route_edge_index: 0,
                to_route_edge_index: 1,
            },
            EventView::Changed {
                tick_index: 1,
                vehicle: "V1".to_owned(),
                route: "R".to_owned(),
                from_edge: "B".to_owned(),
                to_edge: "A".to_owned(),
                from_route_edge_index: 1,
                to_route_edge_index: 2,
            },
            EventView::Completed {
                tick_index: 1,
                vehicle: "V1".to_owned(),
                route: "R".to_owned(),
                edge: "A".to_owned(),
                route_edge_index: 2,
            },
        ]
    );
    let vehicle = vehicle_by_id(&world, "V1");
    assert_eq!(vehicle.status, VehicleStatus::Completed);
    assert_eq!(vehicle.route_edge_index, 2);
    assert_close(vehicle.edge_progress.value(), 10.0);
}

#[test]
fn event_order_uses_initial_stable_update_order_not_input_order() {
    fn world_with_vehicle_order(reverse_input: bool) -> CoreWorld {
        let lane_graph = LaneGraph::try_new([
            LaneEdge::new(
                "A",
                edge_length(1.0),
                laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                ["B"],
            ),
            LaneEdge::new(
                "B",
                edge_length(1.0),
                laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                std::iter::empty::<&str>(),
            ),
        ])
        .expect("valid lane graph");
        let route = Route::try_new("R", ["A", "B"]).expect("valid route");

        let profiles = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
            "short-profile",
            IidmProfileSpec {
                length: 0.25,
                desired_speed: 13.9,
                min_gap: 0.25,
                time_headway: 1.5,
                max_acceleration: 1.4,
                comfortable_deceleration: 2.0,
                emergency_deceleration: 4.0,
            },
        )
        .expect("valid short profile")])
        .expect("valid profile registry");
        let profile = profiles
            .profile_handle("short-profile")
            .expect("profile handle exists");
        let traffic_data =
            InitialTrafficData::try_new(lane_graph, [route], profiles).expect("valid traffic data");
        let vehicles = {
            let v1 = VehicleSpawnInput::active("V1", profile, "R", 0, progress(0.0), speed(2.0));
            let v2 = VehicleSpawnInput::active("V2", profile, "R", 0, progress(0.5), speed(2.0));
            if reverse_input {
                vec![v2, v1]
            } else {
                vec![v1, v2]
            }
        };
        CoreWorld::with_traffic_data(1_000, traffic_data, vehicles).expect("valid world")
    }

    let mut first = world_with_vehicle_order(true);
    let mut second = world_with_vehicle_order(false);

    let first_events = first
        .step(TickInput::new(1_000))
        .expect("step succeeds")
        .events;
    let second_events = second
        .step(TickInput::new(1_000))
        .expect("step succeeds")
        .events;
    let first_views = event_views(&first, &first_events);
    let second_views = event_views(&second, &second_events);

    assert_eq!(first_views, second_views);
    let event_vehicle_ids: Vec<_> = first_views
        .iter()
        .map(|event| match event {
            EventView::Changed { vehicle, .. } | EventView::Completed { vehicle, .. } => {
                vehicle.as_str()
            }
        })
        .collect();
    assert_eq!(event_vehicle_ids, ["V1", "V2", "V2"]);
}

#[test]
fn epsilon_snap_crosses_boundary_when_tick_has_travel() {
    let tick_milliseconds = 1_000;
    let tick_seconds = tick_milliseconds as f64 / 1_000.0;
    let boundary_crossing_speed_meters_per_second =
        1.25 * CURRENT_EDGE_BOUNDARY_TOLERANCE_METERS / tick_seconds;
    let mut world = canonical_world(boundary_crossing_speed_meters_per_second, |profile| {
        VehicleSpawnInput::active(
            "V1",
            profile,
            "R",
            0,
            progress(10.0 - CURRENT_EDGE_BOUNDARY_TOLERANCE_METERS / 2.0),
            speed(boundary_crossing_speed_meters_per_second),
        )
    });

    let result = world
        .step(TickInput::new(tick_milliseconds))
        .expect("step succeeds");

    assert_eq!(
        event_views(&world, &result.events),
        vec![EventView::Changed {
            tick_index: 1,
            vehicle: "V1".to_owned(),
            route: "R".to_owned(),
            from_edge: "A".to_owned(),
            to_edge: "B".to_owned(),
            from_route_edge_index: 0,
            to_route_edge_index: 1,
        }]
    );
    let vehicle = vehicle_by_id(&world, "V1");
    assert_eq!(vehicle.route_edge_index, 1);
    assert_close(vehicle.edge_progress.value(), 0.0);
}

#[test]
fn stopped_vehicle_at_boundary_does_not_transition() {
    let mut world = canonical_world(13.9, |profile| {
        VehicleSpawnInput::stopped("V1", profile, "R", 0, progress(10.0))
    });

    let result = world.step(TickInput::new(1_000)).expect("step succeeds");

    assert!(result.events.is_empty());
    let vehicle = vehicle_by_id(&world, "V1");
    assert_eq!(vehicle.status, VehicleStatus::Stopped);
    assert_eq!(vehicle.route_edge_index, 0);
    assert_close(vehicle.edge_progress.value(), 10.0);
}

#[test]
fn completed_vehicle_initial_state_must_be_at_route_end_and_is_snapped() {
    let world = canonical_world(13.9, |profile| {
        VehicleSpawnInput::completed(
            "V1",
            profile,
            "R",
            1,
            progress(5.0 - CURRENT_EDGE_BOUNDARY_TOLERANCE_METERS / 2.0),
        )
    });

    let vehicle = vehicle_by_id(&world, "V1");
    assert_eq!(vehicle.status, VehicleStatus::Completed);
    assert_eq!(vehicle.route_edge_index, 1);
    assert_close(vehicle.edge_progress.value(), 5.0);
    assert_eq!(vehicle.current_speed, Speed::ZERO);

    let error = canonical_world_result(|profile| {
        vec![VehicleSpawnInput::completed(
            "V2",
            profile,
            "R",
            0,
            progress(10.0),
        )]
    })
    .expect_err("completed vehicle in middle of route must fail");
    std::assert_matches!(
        error,
        CoreError::InvalidCompletedVehicleState {
            vehicle_id,
            route_id,
            route_edge_index: 0,
            expected_route_edge_index: 1,
            ..
        } if vehicle_id == "V2" && route_id == "R"
    );

    let error = canonical_world_result(|profile| {
        vec![VehicleSpawnInput::completed(
            "V3",
            profile,
            "R",
            1,
            progress(1.0),
        )]
    })
    .expect_err("completed vehicle before route end must fail");
    std::assert_matches!(
        error,
        CoreError::InvalidCompletedVehicleState {
            vehicle_id,
            route_id,
            route_edge_index: 1,
            expected_route_edge_index: 1,
            ..
        } if vehicle_id == "V3" && route_id == "R"
    );
}

fn canonical_world_result(
    vehicles: impl FnOnce(VehicleProfileHandle) -> Vec<VehicleSpawnInput>,
) -> Result<CoreWorld, CoreError> {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new(
            "A",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["B"],
        ),
        LaneEdge::new(
            "B",
            edge_length(5.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A", "B"]).expect("valid route");

    world_with_test_profile(1_000, lane_graph, [route], vehicles)
}

#[test]
fn non_finite_longitudinal_travel_returns_error_and_keeps_world_unchanged() {
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "A",
        edge_length(f64::MAX),
        laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
        std::iter::empty::<&str>(),
    )])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A"]).expect("valid route");
    let mut world = world_with_test_profile(2_000, lane_graph, [route], |profile| {
        vec![VehicleSpawnInput::active(
            "V1",
            profile,
            "R",
            0,
            progress(0.0),
            speed(f64::MAX),
        )]
    })
    .expect("valid world");
    let before = world.clone();
    let vehicle = world.vehicle_handle("V1").expect("vehicle handle exists");

    let error = world
        .step(TickInput::new(2_000))
        .expect_err("non-finite route travel must fail");

    std::assert_matches!(
        error,
        CoreError::NonFiniteLongitudinalComputation {
            vehicle: actual_vehicle,
            stage: "ballistic_travel",
            value
        } if actual_vehicle == vehicle && value.is_infinite()
    );
    assert_eq!(world, before);
}

#[test]
fn finite_route_travel_does_not_overflow_in_millisecond_conversion() {
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "A",
        edge_length(f64::MAX),
        laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
        std::iter::empty::<&str>(),
    )])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A"]).expect("valid route");
    let mut world = world_with_test_profile(1_000, lane_graph, [route], |profile| {
        vec![VehicleSpawnInput::active(
            "V1",
            profile,
            "R",
            0,
            progress(0.0),
            speed(f64::MAX),
        )]
    })
    .expect("valid world");
    let vehicle = world.vehicle_handle("V1").expect("vehicle handle exists");

    world
        .step(TickInput::new(1_000))
        .expect("finite one-second travel must succeed");

    let vehicle = world.vehicle(vehicle).expect("vehicle remains live");
    assert_eq!(vehicle.status, VehicleStatus::Completed);
    assert_eq!(vehicle.edge_progress.value(), f64::MAX);
}

#[test]
fn step_failure_after_prior_vehicle_progress_keeps_world_unchanged() {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new(
            "A",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["B"],
        ),
        LaneEdge::new(
            "B",
            edge_length(f64::MAX),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A", "B"]).expect("valid route");
    let mut world = world_with_test_profile(2_000, lane_graph, [route], |profile| {
        vec![
            VehicleSpawnInput::active("V1", profile, "R", 0, progress(0.0), speed(2.0)),
            VehicleSpawnInput::active("V2", profile, "R", 1, progress(0.0), speed(f64::MAX)),
        ]
    })
    .expect("valid world");
    let before = world.clone();
    let vehicle = world.vehicle_handle("V2").expect("vehicle handle exists");

    let error = world
        .step(TickInput::new(2_000))
        .expect_err("later vehicle failure must fail the whole step");

    std::assert_matches!(
        error,
        CoreError::NonFiniteLeaderComputation {
            vehicle: actual_vehicle,
            stage: "travel_upper",
            value
        } if actual_vehicle == vehicle && value.is_infinite()
    );
    assert_eq!(world, before);
}
