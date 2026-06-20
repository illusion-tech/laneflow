use laneflow_core::{
    CoreError, CoreEvent, CoreWorld, EDGE_BOUNDARY_EPSILON, EdgeLength, EdgeProgress, LaneEdge,
    LaneGraph, Route, Speed, TickInput, VehicleChangedEdgeEvent, VehicleCompletedRouteEvent,
    VehicleState, VehicleStatus,
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

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= EDGE_BOUNDARY_EPSILON,
        "actual={actual}, expected={expected}"
    );
}

fn canonical_world(vehicle: VehicleState) -> CoreWorld {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new("A", edge_length(10.0), ["B"]),
        LaneEdge::new("B", edge_length(5.0), std::iter::empty::<&str>()),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A", "B"]).expect("valid route");

    CoreWorld::with_traffic_data(1000, lane_graph, [route], vec![vehicle]).expect("valid world")
}

#[test]
fn canonical_fixture_ticks_match_design() {
    let vehicle = VehicleState::active("V1", "R", 0, progress(0.0), speed(6.0));
    let mut world = canonical_world(vehicle);

    let tick1 = world.step(TickInput::new(1000)).expect("tick 1 succeeds");
    assert_eq!(tick1.tick_index, 1);
    assert_eq!(tick1.time_ms, 1000);
    assert!(tick1.events.is_empty());
    let vehicle = &world.vehicles()[0];
    assert_eq!(vehicle.status, VehicleStatus::Active);
    assert_eq!(vehicle.route_edge_index, 0);
    assert_close(vehicle.edge_progress.value(), 6.0);

    let tick2 = world.step(TickInput::new(1000)).expect("tick 2 succeeds");
    assert_eq!(tick2.tick_index, 2);
    assert_eq!(tick2.time_ms, 2000);
    assert_eq!(
        tick2.events,
        vec![CoreEvent::VehicleChangedEdge(VehicleChangedEdgeEvent {
            tick_index: 2,
            vehicle_id: "V1".to_owned(),
            route_id: "R".to_owned(),
            from_edge_id: "A".to_owned(),
            to_edge_id: "B".to_owned(),
            from_route_edge_index: 0,
            to_route_edge_index: 1,
        })]
    );
    let vehicle = &world.vehicles()[0];
    assert_eq!(vehicle.status, VehicleStatus::Active);
    assert_eq!(vehicle.route_edge_index, 1);
    assert_close(vehicle.edge_progress.value(), 2.0);

    let tick3 = world.step(TickInput::new(1000)).expect("tick 3 succeeds");
    assert_eq!(tick3.tick_index, 3);
    assert_eq!(tick3.time_ms, 3000);
    assert_eq!(
        tick3.events,
        vec![CoreEvent::VehicleCompletedRoute(
            VehicleCompletedRouteEvent {
                tick_index: 3,
                vehicle_id: "V1".to_owned(),
                route_id: "R".to_owned(),
                edge_id: "B".to_owned(),
                route_edge_index: 1,
            }
        )]
    );
    let vehicle = &world.vehicles()[0];
    assert_eq!(vehicle.status, VehicleStatus::Completed);
    assert_eq!(vehicle.route_edge_index, 1);
    assert_close(vehicle.edge_progress.value(), 5.0);

    let tick4 = world.step(TickInput::new(1000)).expect("tick 4 succeeds");
    assert_eq!(tick4.tick_index, 4);
    assert_eq!(tick4.time_ms, 4000);
    assert!(tick4.events.is_empty());
    let vehicle = &world.vehicles()[0];
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
    let vehicle = VehicleState::active("V1", "R", 0, progress(0.0), speed(30.0));
    let mut world = CoreWorld::with_traffic_data(1000, lane_graph, [route], vec![vehicle])
        .expect("valid world");

    let result = world.step(TickInput::new(1000)).expect("step succeeds");

    assert_eq!(
        result.events,
        vec![
            CoreEvent::VehicleChangedEdge(VehicleChangedEdgeEvent {
                tick_index: 1,
                vehicle_id: "V1".to_owned(),
                route_id: "R".to_owned(),
                from_edge_id: "A".to_owned(),
                to_edge_id: "B".to_owned(),
                from_route_edge_index: 0,
                to_route_edge_index: 1,
            }),
            CoreEvent::VehicleChangedEdge(VehicleChangedEdgeEvent {
                tick_index: 1,
                vehicle_id: "V1".to_owned(),
                route_id: "R".to_owned(),
                from_edge_id: "B".to_owned(),
                to_edge_id: "C".to_owned(),
                from_route_edge_index: 1,
                to_route_edge_index: 2,
            }),
            CoreEvent::VehicleCompletedRoute(VehicleCompletedRouteEvent {
                tick_index: 1,
                vehicle_id: "V1".to_owned(),
                route_id: "R".to_owned(),
                edge_id: "C".to_owned(),
                route_edge_index: 2,
            }),
        ]
    );
    let vehicle = &world.vehicles()[0];
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
    let vehicle = VehicleState::active("V1", "R", 0, progress(0.0), speed(30.0));
    let mut world = CoreWorld::with_traffic_data(1000, lane_graph, [route], vec![vehicle])
        .expect("valid world");

    let result = world.step(TickInput::new(1000)).expect("step succeeds");

    assert_eq!(
        result.events,
        vec![
            CoreEvent::VehicleChangedEdge(VehicleChangedEdgeEvent {
                tick_index: 1,
                vehicle_id: "V1".to_owned(),
                route_id: "R".to_owned(),
                from_edge_id: "A".to_owned(),
                to_edge_id: "B".to_owned(),
                from_route_edge_index: 0,
                to_route_edge_index: 1,
            }),
            CoreEvent::VehicleChangedEdge(VehicleChangedEdgeEvent {
                tick_index: 1,
                vehicle_id: "V1".to_owned(),
                route_id: "R".to_owned(),
                from_edge_id: "B".to_owned(),
                to_edge_id: "A".to_owned(),
                from_route_edge_index: 1,
                to_route_edge_index: 2,
            }),
            CoreEvent::VehicleCompletedRoute(VehicleCompletedRouteEvent {
                tick_index: 1,
                vehicle_id: "V1".to_owned(),
                route_id: "R".to_owned(),
                edge_id: "A".to_owned(),
                route_edge_index: 2,
            }),
        ]
    );
    let vehicle = &world.vehicles()[0];
    assert_eq!(vehicle.status, VehicleStatus::Completed);
    assert_eq!(vehicle.route_edge_index, 2);
    assert_close(vehicle.edge_progress.value(), 10.0);
}

#[test]
fn event_order_is_stable_by_vehicle_id_not_input_order() {
    fn world_with_vehicle_order(vehicles: Vec<VehicleState>) -> CoreWorld {
        let lane_graph = LaneGraph::try_new([
            LaneEdge::new("A", edge_length(1.0), ["B"]),
            LaneEdge::new("B", edge_length(1.0), std::iter::empty::<&str>()),
        ])
        .expect("valid lane graph");
        let route = Route::try_new("R", ["A", "B"]).expect("valid route");

        CoreWorld::with_traffic_data(1000, lane_graph, [route], vehicles).expect("valid world")
    }

    let v1 = VehicleState::active("V1", "R", 0, progress(0.0), speed(2.0));
    let v2 = VehicleState::active("V2", "R", 0, progress(0.0), speed(2.0));
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

    assert_eq!(first_events, second_events);
    let event_vehicle_ids: Vec<_> = first_events
        .iter()
        .map(|event| match event {
            CoreEvent::VehicleChangedEdge(event) => event.vehicle_id.as_str(),
            CoreEvent::VehicleCompletedRoute(event) => event.vehicle_id.as_str(),
            _ => unreachable!("route following tests only create v0.1 route events"),
        })
        .collect();
    assert_eq!(event_vehicle_ids, ["V1", "V1", "V2", "V2"]);
}

#[test]
fn epsilon_snap_crosses_boundary_when_tick_has_travel() {
    let vehicle = VehicleState::active(
        "V1",
        "R",
        0,
        progress(10.0 - EDGE_BOUNDARY_EPSILON / 2.0),
        speed(EDGE_BOUNDARY_EPSILON * 1.25),
    );
    let mut world = canonical_world(vehicle);

    let result = world.step(TickInput::new(1000)).expect("step succeeds");

    assert_eq!(
        result.events,
        vec![CoreEvent::VehicleChangedEdge(VehicleChangedEdgeEvent {
            tick_index: 1,
            vehicle_id: "V1".to_owned(),
            route_id: "R".to_owned(),
            from_edge_id: "A".to_owned(),
            to_edge_id: "B".to_owned(),
            from_route_edge_index: 0,
            to_route_edge_index: 1,
        })]
    );
    let vehicle = &world.vehicles()[0];
    assert_eq!(vehicle.route_edge_index, 1);
    assert_close(vehicle.edge_progress.value(), 0.0);
}

#[test]
fn zero_speed_at_boundary_does_not_transition() {
    let vehicle = VehicleState::active("V1", "R", 0, progress(10.0), Speed::ZERO);
    let mut world = canonical_world(vehicle);

    let result = world.step(TickInput::new(1000)).expect("step succeeds");

    assert!(result.events.is_empty());
    let vehicle = &world.vehicles()[0];
    assert_eq!(vehicle.status, VehicleStatus::Active);
    assert_eq!(vehicle.route_edge_index, 0);
    assert_close(vehicle.edge_progress.value(), 10.0);
}

#[test]
fn completed_vehicle_initial_state_must_be_at_route_end_and_is_snapped() {
    let vehicle = VehicleState::completed(
        "V1",
        "R",
        1,
        progress(5.0 - EDGE_BOUNDARY_EPSILON / 2.0),
        speed(8.0),
    );

    let world = canonical_world(vehicle);

    let vehicle = &world.vehicles()[0];
    assert_eq!(vehicle.status, VehicleStatus::Completed);
    assert_eq!(vehicle.route_edge_index, 1);
    assert_close(vehicle.edge_progress.value(), 5.0);
    assert_eq!(vehicle.speed.value(), 8.0);

    let invalid_middle = VehicleState::completed("V2", "R", 0, progress(10.0), speed(1.0));
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

    let invalid_progress = VehicleState::completed("V3", "R", 1, progress(1.0), speed(1.0));
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

fn canonical_world_result(vehicles: Vec<VehicleState>) -> Result<CoreWorld, CoreError> {
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
    let vehicle = VehicleState::active("V1", "R", 0, progress(0.0), speed(f64::MAX));
    let mut world = CoreWorld::with_traffic_data(1000, lane_graph, [route], vec![vehicle])
        .expect("valid world");
    let before = world.clone();

    let error = world
        .step(TickInput::new(1000))
        .expect_err("non-finite route travel must fail");

    std::assert_matches!(
        error,
        CoreError::NonFiniteRouteTravel {
            vehicle_id,
            speed,
            delta_time_ms: 1000
        } if vehicle_id == "V1" && speed == f64::MAX
    );
    assert_eq!(world, before);
}
