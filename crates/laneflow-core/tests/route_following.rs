use laneflow_core::{
    CoreError, CoreEvent, CoreWorld, EDGE_BOUNDARY_EPSILON, EdgeLength, EdgeProgress, LaneEdge,
    LaneGraph, Route, Speed, TickInput, VehicleSpawnInput, VehicleState, VehicleStatus,
};

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

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= EDGE_BOUNDARY_EPSILON,
        "actual={actual}, expected={expected}"
    );
}

fn canonical_world(vehicle: VehicleSpawnInput) -> CoreWorld {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new("A", edge_length(10.0), ["B"]),
        LaneEdge::new("B", edge_length(5.0), std::iter::empty::<&str>()),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A", "B"]).expect("valid route");

    CoreWorld::with_traffic_data(1000, lane_graph, [route], vec![vehicle]).expect("valid world")
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
    let vehicle = VehicleSpawnInput::active("V1", "R", 0, progress(0.0), speed(6.0));
    let mut world = canonical_world(vehicle);

    let tick1 = world.step(TickInput::new(1000)).expect("tick 1 succeeds");
    assert_eq!(tick1.tick_index, 1);
    assert_eq!(tick1.time_ms, 1000);
    assert!(tick1.events.is_empty());
    let vehicle = vehicle_by_id(&world, "V1");
    assert_eq!(vehicle.status, VehicleStatus::Active);
    assert_eq!(vehicle.route_edge_index, 0);
    assert_close(vehicle.edge_progress.value(), 6.0);

    let tick2 = world.step(TickInput::new(1000)).expect("tick 2 succeeds");
    assert_eq!(tick2.tick_index, 2);
    assert_eq!(tick2.time_ms, 2000);
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

    let tick3 = world.step(TickInput::new(1000)).expect("tick 3 succeeds");
    assert_eq!(tick3.tick_index, 3);
    assert_eq!(tick3.time_ms, 3000);
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

    let tick4 = world.step(TickInput::new(1000)).expect("tick 4 succeeds");
    assert_eq!(tick4.tick_index, 4);
    assert_eq!(tick4.time_ms, 4000);
    assert!(tick4.events.is_empty());
    let vehicle = vehicle_by_id(&world, "V1");
    assert_eq!(vehicle.status, VehicleStatus::Completed);
    assert_eq!(vehicle.route_edge_index, 1);
    assert_close(vehicle.edge_progress.value(), 5.0);
}

#[test]
fn single_tick_can_cross_multiple_edges_in_route_order() {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new("A", edge_length(10.0), ["B"]),
        LaneEdge::new("B", edge_length(5.0), ["C"]),
        LaneEdge::new("C", edge_length(2.0), std::iter::empty::<&str>()),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A", "B", "C"]).expect("valid route");
    let vehicle = VehicleSpawnInput::active("V1", "R", 0, progress(0.0), speed(30.0));
    let mut world = CoreWorld::with_traffic_data(1000, lane_graph, [route], vec![vehicle])
        .expect("valid world");

    let result = world.step(TickInput::new(1000)).expect("step succeeds");

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
        LaneEdge::new("A", edge_length(10.0), ["B"]),
        LaneEdge::new("B", edge_length(5.0), ["A"]),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A", "B", "A"]).expect("valid repeated-edge route");
    let vehicle = VehicleSpawnInput::active("V1", "R", 0, progress(0.0), speed(30.0));
    let mut world = CoreWorld::with_traffic_data(1000, lane_graph, [route], vec![vehicle])
        .expect("valid world");

    let result = world.step(TickInput::new(1000)).expect("step succeeds");

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
    fn world_with_vehicle_order(vehicles: Vec<VehicleSpawnInput>) -> CoreWorld {
        let lane_graph = LaneGraph::try_new([
            LaneEdge::new("A", edge_length(1.0), ["B"]),
            LaneEdge::new("B", edge_length(1.0), std::iter::empty::<&str>()),
        ])
        .expect("valid lane graph");
        let route = Route::try_new("R", ["A", "B"]).expect("valid route");

        CoreWorld::with_traffic_data(1000, lane_graph, [route], vehicles).expect("valid world")
    }

    let v1 = VehicleSpawnInput::active("V1", "R", 0, progress(0.0), speed(2.0));
    let v2 = VehicleSpawnInput::active("V2", "R", 0, progress(0.0), speed(2.0));
    let mut first = world_with_vehicle_order(vec![v2.clone(), v1.clone()]);
    let mut second = world_with_vehicle_order(vec![v1, v2]);

    let first_events = first
        .step(TickInput::new(1000))
        .expect("step succeeds")
        .events;
    let second_events = second
        .step(TickInput::new(1000))
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
    assert_eq!(event_vehicle_ids, ["V1", "V1", "V2", "V2"]);
}

#[test]
fn epsilon_snap_crosses_boundary_when_tick_has_travel() {
    let vehicle = VehicleSpawnInput::active(
        "V1",
        "R",
        0,
        progress(10.0 - EDGE_BOUNDARY_EPSILON / 2.0),
        speed(EDGE_BOUNDARY_EPSILON * 1.25),
    );
    let mut world = canonical_world(vehicle);

    let result = world.step(TickInput::new(1000)).expect("step succeeds");

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
fn zero_speed_at_boundary_does_not_transition() {
    let vehicle = VehicleSpawnInput::active("V1", "R", 0, progress(10.0), Speed::ZERO);
    let mut world = canonical_world(vehicle);

    let result = world.step(TickInput::new(1000)).expect("step succeeds");

    assert!(result.events.is_empty());
    let vehicle = vehicle_by_id(&world, "V1");
    assert_eq!(vehicle.status, VehicleStatus::Active);
    assert_eq!(vehicle.route_edge_index, 0);
    assert_close(vehicle.edge_progress.value(), 10.0);
}

#[test]
fn completed_vehicle_initial_state_must_be_at_route_end_and_is_snapped() {
    let vehicle = VehicleSpawnInput::completed(
        "V1",
        "R",
        1,
        progress(5.0 - EDGE_BOUNDARY_EPSILON / 2.0),
        speed(8.0),
    );

    let world = canonical_world(vehicle);

    let vehicle = vehicle_by_id(&world, "V1");
    assert_eq!(vehicle.status, VehicleStatus::Completed);
    assert_eq!(vehicle.route_edge_index, 1);
    assert_close(vehicle.edge_progress.value(), 5.0);
    assert_eq!(vehicle.speed.value(), 8.0);

    let invalid_middle = VehicleSpawnInput::completed("V2", "R", 0, progress(10.0), speed(1.0));
    let error = canonical_world_result(vec![invalid_middle])
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

    let invalid_progress = VehicleSpawnInput::completed("V3", "R", 1, progress(1.0), speed(1.0));
    let error = canonical_world_result(vec![invalid_progress])
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

fn canonical_world_result(vehicles: Vec<VehicleSpawnInput>) -> Result<CoreWorld, CoreError> {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new("A", edge_length(10.0), ["B"]),
        LaneEdge::new("B", edge_length(5.0), std::iter::empty::<&str>()),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A", "B"]).expect("valid route");

    CoreWorld::with_traffic_data(1000, lane_graph, [route], vehicles)
}

#[test]
fn non_finite_route_travel_returns_error_and_keeps_world_unchanged() {
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "A",
        edge_length(f64::MAX),
        std::iter::empty::<&str>(),
    )])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A"]).expect("valid route");
    let vehicle = VehicleSpawnInput::active("V1", "R", 0, progress(0.0), speed(f64::MAX));
    let mut world = CoreWorld::with_traffic_data(2000, lane_graph, [route], vec![vehicle])
        .expect("valid world");
    let before = world.clone();
    let vehicle = world.vehicle_handle("V1").expect("vehicle handle exists");

    let error = world
        .step(TickInput::new(2000))
        .expect_err("non-finite route travel must fail");

    std::assert_matches!(
        error,
        CoreError::NonFiniteRouteTravel {
            vehicle: actual_vehicle,
            speed,
            delta_time_ms: 2000
        } if actual_vehicle == vehicle && speed == f64::MAX
    );
    assert_eq!(world, before);
}

#[test]
fn finite_route_travel_does_not_overflow_in_millisecond_conversion() {
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "A",
        edge_length(f64::MAX),
        std::iter::empty::<&str>(),
    )])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A"]).expect("valid route");
    let vehicle = VehicleSpawnInput::active("V1", "R", 0, progress(0.0), speed(f64::MAX));
    let mut world = CoreWorld::with_traffic_data(1000, lane_graph, [route], vec![vehicle])
        .expect("valid world");
    let vehicle = world.vehicle_handle("V1").expect("vehicle handle exists");

    world
        .step(TickInput::new(1000))
        .expect("finite one-second travel must succeed");

    let vehicle = world.vehicle(vehicle).expect("vehicle remains live");
    assert_eq!(vehicle.status, VehicleStatus::Completed);
    assert_eq!(vehicle.edge_progress.value(), f64::MAX);
}

#[test]
fn step_failure_after_prior_vehicle_progress_keeps_world_unchanged() {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new("A", edge_length(1.0), ["B"]),
        LaneEdge::new("B", edge_length(f64::MAX), std::iter::empty::<&str>()),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A", "B"]).expect("valid route");
    let vehicles = vec![
        VehicleSpawnInput::active("V1", "R", 0, progress(0.0), speed(2.0)),
        VehicleSpawnInput::active("V2", "R", 1, progress(0.0), speed(f64::MAX)),
    ];
    let mut world =
        CoreWorld::with_traffic_data(2000, lane_graph, [route], vehicles).expect("valid world");
    let before = world.clone();
    let vehicle = world.vehicle_handle("V2").expect("vehicle handle exists");

    let error = world
        .step(TickInput::new(2000))
        .expect_err("later vehicle failure must fail the whole step");

    std::assert_matches!(
        error,
        CoreError::NonFiniteRouteTravel {
            vehicle: actual_vehicle,
            speed,
            delta_time_ms: 2000
        } if actual_vehicle == vehicle && speed == f64::MAX
    );
    assert_eq!(world, before);
}
