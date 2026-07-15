use laneflow_core::{
    CoreEvent, CoreWorld, EDGE_BOUNDARY_EPSILON, EdgeProgress, RouteHandle, Speed, TickInput,
    VehicleChangedEdgeEvent, VehicleCompletedRouteEvent, VehicleSpawnInput, VehicleStatus,
};
use laneflow_data::from_json_str;

const EXAMPLE_ROUTE_DATA: &str =
    include_str!("../../../examples/data/v0.4-empty-signals.laneflow.json");
const MILLISECONDS_PER_SECOND: f64 = 1_000.0;

fn load_example_world() -> CoreWorld {
    let loaded = from_json_str(EXAMPLE_ROUTE_DATA).expect("current example package must load");
    let traffic_data = loaded.into_initial_traffic_data();
    assert_eq!(
        traffic_data.vehicle_profiles().len(),
        1,
        "current fixture declares one profile"
    );
    assert!(
        traffic_data.signals().is_empty(),
        "legacy route behavior fixture must use explicit empty v0.4 Signals"
    );

    CoreWorld::with_traffic_data(1_000, traffic_data, Vec::new())
        .expect("example route data must initialize CoreWorld")
}

fn route_distance(world: &CoreWorld, route: RouteHandle) -> f64 {
    world
        .route_edges(route)
        .expect("example route exists")
        .iter()
        .map(|edge| {
            world
                .lane_graph()
                .edge_length(*edge)
                .expect("route edge exists in the lane graph")
                .value()
        })
        .sum()
}

fn speed_to_complete_route_in_one_tick(world: &CoreWorld, route: RouteHandle) -> Speed {
    let delta_time_seconds = world.fixed_delta_time_ms() as f64 / MILLISECONDS_PER_SECOND;
    Speed::try_new(route_distance(world, route) / delta_time_seconds)
        .expect("example route travel speed must be valid")
}

#[test]
fn example_route_data_loads_into_core_with_declared_topology_boundaries() {
    let world = load_example_world();

    let entry = world.edge_handle("entry").expect("entry edge exists");
    let exit = world.edge_handle("exit").expect("terminal edge exists");
    let loop_edge = world.edge_handle("loop").expect("loop edge exists");
    let isolated = world
        .edge_handle("isolated")
        .expect("disconnected edge exists");

    assert!(world.lane_graph().can_traverse(entry, exit));
    assert!(world.lane_graph().can_traverse(loop_edge, loop_edge));
    assert!(!world.lane_graph().can_traverse(exit, isolated));

    let main_route = world
        .route_handle("main-route")
        .expect("normal two-edge route exists");
    let loop_route = world
        .route_handle("loop-once")
        .expect("repeated edge route exists");

    assert_eq!(world.route_external_id(main_route), Some("main-route"));
    assert_eq!(world.route_external_id(loop_route), Some("loop-once"));
    assert_eq!(
        world
            .route_edges(main_route)
            .expect("main route is active")
            .iter()
            .map(|handle| world
                .edge_external_id(*handle)
                .expect("edge resolver exists"))
            .collect::<Vec<_>>(),
        ["entry", "exit"]
    );
    assert_eq!(
        world
            .route_edges(loop_route)
            .expect("loop route is active")
            .iter()
            .map(|handle| world
                .edge_external_id(*handle)
                .expect("edge resolver exists"))
            .collect::<Vec<_>>(),
        ["loop", "loop"]
    );
}

#[test]
fn example_route_data_drives_main_and_repeated_routes_to_completion_under_iidm() {
    let mut world = load_example_world();
    let main_route = world.route_handle("main-route").expect("main route exists");
    let loop_route = world.route_handle("loop-once").expect("loop route exists");
    let entry = world.edge_handle("entry").expect("entry edge exists");
    let exit = world.edge_handle("exit").expect("exit edge exists");
    let loop_edge = world.edge_handle("loop").expect("loop edge exists");
    let main_speed = speed_to_complete_route_in_one_tick(&world, main_route);
    let loop_speed = speed_to_complete_route_in_one_tick(&world, loop_route);
    let exit_length = world
        .lane_graph()
        .edge_length(exit)
        .expect("exit edge length exists")
        .value();
    let loop_length = world
        .lane_graph()
        .edge_length(loop_edge)
        .expect("loop edge length exists")
        .value();

    let profile = world
        .vehicle_profile_handle("passenger-car")
        .expect("fixture profile exists");
    let main_vehicle = world
        .spawn_vehicle(VehicleSpawnInput::active(
            "main-vehicle",
            profile,
            "main-route",
            0,
            EdgeProgress::ZERO,
            main_speed,
        ))
        .expect("main vehicle spawns");
    let loop_vehicle = world
        .spawn_vehicle(VehicleSpawnInput::active(
            "loop-vehicle",
            profile,
            "loop-once",
            0,
            EdgeProgress::ZERO,
            loop_speed,
        ))
        .expect("loop vehicle spawns");

    let mut events = Vec::new();
    for _ in 0..4 {
        let result = world
            .step(TickInput::new(world.fixed_delta_time_ms()))
            .expect("example route step succeeds");
        events.extend(result.events);
        if world.vehicle(main_vehicle).unwrap().status == VehicleStatus::Completed
            && world.vehicle(loop_vehicle).unwrap().status == VehicleStatus::Completed
        {
            break;
        }
    }

    assert_eq!(
        events,
        vec![
            CoreEvent::VehicleChangedEdge(VehicleChangedEdgeEvent {
                tick_index: 1,
                vehicle: main_vehicle,
                route: main_route,
                from_edge: entry,
                to_edge: exit,
                from_route_edge_index: 0,
                to_route_edge_index: 1,
            }),
            CoreEvent::VehicleChangedEdge(VehicleChangedEdgeEvent {
                tick_index: 1,
                vehicle: loop_vehicle,
                route: loop_route,
                from_edge: loop_edge,
                to_edge: loop_edge,
                from_route_edge_index: 0,
                to_route_edge_index: 1,
            }),
            CoreEvent::VehicleCompletedRoute(VehicleCompletedRouteEvent {
                tick_index: 1,
                vehicle: loop_vehicle,
                route: loop_route,
                edge: loop_edge,
                route_edge_index: 1,
            }),
            CoreEvent::VehicleCompletedRoute(VehicleCompletedRouteEvent {
                tick_index: 2,
                vehicle: main_vehicle,
                route: main_route,
                edge: exit,
                route_edge_index: 1,
            }),
        ]
    );

    let main_vehicle = world
        .vehicle(main_vehicle)
        .expect("main vehicle remains live");
    assert_eq!(main_vehicle.status, VehicleStatus::Completed);
    assert_eq!(main_vehicle.route_edge_index, 1);
    assert!(
        (main_vehicle.edge_progress.value() - exit_length).abs() <= EDGE_BOUNDARY_EPSILON,
        "main vehicle must finish at the terminal edge boundary"
    );

    let loop_vehicle = world
        .vehicle(loop_vehicle)
        .expect("loop vehicle remains live");
    assert_eq!(loop_vehicle.status, VehicleStatus::Completed);
    assert_eq!(loop_vehicle.route_edge_index, 1);
    assert!(
        (loop_vehicle.edge_progress.value() - loop_length).abs() <= EDGE_BOUNDARY_EPSILON,
        "loop vehicle must finish at the second route occurrence"
    );
}
