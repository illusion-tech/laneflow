use laneflow_core::{
    CoreError, CoreEvent, CoreWorld, EdgeLength, EdgeProgress, LaneEdge, LaneGraph, Route, Speed,
    TickInput, VehicleSpawnInput,
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

fn route_world(vehicles: Vec<VehicleSpawnInput>) -> CoreWorld {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new("A", edge_length(1.0), ["B"]),
        LaneEdge::new("B", edge_length(1.0), std::iter::empty::<&str>()),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A", "B"]).expect("valid route");

    CoreWorld::with_traffic_data(1000, lane_graph, [route], vehicles).expect("valid world")
}

fn event_vehicle_ids(world: &CoreWorld, events: &[CoreEvent]) -> Vec<String> {
    events
        .iter()
        .map(|event| match event {
            CoreEvent::VehicleChangedEdge(event) => world
                .vehicle_external_id(event.vehicle)
                .expect("vehicle id exists")
                .to_owned(),
            CoreEvent::VehicleCompletedRoute(event) => world
                .vehicle_external_id(event.vehicle)
                .expect("vehicle id exists")
                .to_owned(),
            _ => unreachable!("handle tests only create route events"),
        })
        .collect()
}

#[test]
fn vehicle_handle_generation_rejects_stale_handle_after_despawn() {
    let mut world = route_world(vec![VehicleSpawnInput::active(
        "V1",
        "R",
        0,
        progress(0.0),
        speed(0.0),
    )]);
    let old_handle = world.vehicle_handle("V1").expect("vehicle handle exists");

    let record = world.despawn_vehicle(old_handle).expect("despawn succeeds");

    assert_eq!(record.external_id, "V1");
    assert_eq!(world.vehicle_external_id(old_handle), None);
    std::assert_matches!(
        world
            .despawn_vehicle(old_handle)
            .expect_err("stale handle must fail"),
        CoreError::UnknownVehicleHandle { vehicle } if vehicle == old_handle
    );

    let new_handle = world
        .spawn_vehicle(VehicleSpawnInput::active(
            "V2",
            "R",
            0,
            progress(0.0),
            speed(0.0),
        ))
        .expect("spawn succeeds");

    assert_ne!(new_handle, old_handle);
    assert_eq!(world.vehicle_external_id(new_handle), Some("V2"));
    assert_eq!(world.vehicle_external_id(old_handle), None);
}

#[test]
fn route_remove_rejects_live_vehicle_and_stales_old_handle() {
    let mut world = route_world(vec![VehicleSpawnInput::active(
        "V1",
        "R",
        0,
        progress(0.0),
        speed(0.0),
    )]);
    let route = world.route_handle("R").expect("route handle exists");
    let vehicle = world.vehicle_handle("V1").expect("vehicle handle exists");

    std::assert_matches!(
        world
            .remove_route(route)
            .expect_err("route in use must fail"),
        CoreError::RouteInUse {
            route: actual_route,
            vehicle: actual_vehicle
        } if actual_route == route && actual_vehicle == vehicle
    );

    world.despawn_vehicle(vehicle).expect("despawn succeeds");
    let record = world.remove_route(route).expect("remove route succeeds");
    assert_eq!(record.external_id, "R");
    assert_eq!(world.route_external_id(route), None);
    std::assert_matches!(
        world
            .remove_route(route)
            .expect_err("stale route handle must fail"),
        CoreError::UnknownRouteHandle { route: actual_route } if actual_route == route
    );

    let new_route = world
        .register_route(Route::try_new("R", ["A", "B"]).expect("valid route"))
        .expect("register route succeeds");
    assert_ne!(new_route, route);
    assert_eq!(world.route_external_id(new_route), Some("R"));
}

#[test]
fn spawned_vehicle_keeps_command_order_after_initial_update_order() {
    let mut world = route_world(vec![VehicleSpawnInput::active(
        "V2",
        "R",
        0,
        progress(0.0),
        speed(2.0),
    )]);
    world
        .spawn_vehicle(VehicleSpawnInput::active(
            "V1",
            "R",
            0,
            progress(0.0),
            speed(2.0),
        ))
        .expect("spawn succeeds");

    let result = world.step(TickInput::new(1000)).expect("step succeeds");

    assert_eq!(
        event_vehicle_ids(&world, &result.events),
        ["V2", "V2", "V1", "V1"]
    );
}
