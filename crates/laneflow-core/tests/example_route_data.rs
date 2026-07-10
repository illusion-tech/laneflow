use jsonschema::draft202012;
use laneflow_core::{
    CoreEvent, CoreWorld, EDGE_BOUNDARY_EPSILON, EdgeLength, EdgeProgress, LaneEdge, LaneGraph,
    Route, RouteHandle, Speed, TickInput, VehicleChangedEdgeEvent, VehicleCompletedRouteEvent,
    VehicleSpawnInput, VehicleStatus,
};
use serde::Deserialize;
use serde_json::Value;

const EXAMPLE_ROUTE_DATA: &str =
    include_str!("../../../examples/data/v0.2-route-baseline.laneflow.json");
const DATA_FORMAT_SCHEMA: &str = include_str!("../../../schemas/laneflow-data-v0.2.schema.json");
const MILLISECONDS_PER_SECOND: f64 = 1_000.0;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExamplePackage {
    format_version: String,
    units: UnitSpec,
    lane_graph: LaneGraphData,
    routes: Vec<RouteData>,
    #[serde(default)]
    extensions: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnitSpec {
    distance: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LaneGraphData {
    edges: Vec<LaneEdgeData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LaneEdgeData {
    id: String,
    length: f64,
    connections: Vec<LaneConnectionData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LaneConnectionData {
    to: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RouteData {
    id: String,
    edges: Vec<String>,
}

fn schema() -> Value {
    serde_json::from_str(DATA_FORMAT_SCHEMA).expect("data format schema must be valid JSON")
}

fn example_value() -> Value {
    serde_json::from_str(EXAMPLE_ROUTE_DATA).expect("example route data must be valid JSON")
}

fn example_package() -> ExamplePackage {
    serde_json::from_value(example_value())
        .expect("example route data must match the Core input view")
}

fn load_example_world() -> CoreWorld {
    let package = example_package();
    assert_eq!(package.format_version, "0.2");
    assert_eq!(package.units.distance, "meter");
    let _extensions = package.extensions;

    let lane_graph = LaneGraph::try_new(package.lane_graph.edges.into_iter().map(|edge| {
        LaneEdge::new(
            edge.id,
            EdgeLength::try_new(edge.length).expect("example edge length must be valid"),
            edge.connections.into_iter().map(|connection| connection.to),
        )
    }))
    .expect("example lane graph must satisfy topology validation");
    let routes = package.routes.into_iter().map(|route| {
        Route::try_new(route.id, route.edges).expect("example route shape must be valid")
    });

    CoreWorld::with_traffic_data(1_000, lane_graph, routes, Vec::new())
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
fn example_route_data_validates_against_draft_2020_12_schema() {
    let schema = schema();
    draft202012::meta::validate(&schema).expect("repository schema must satisfy Draft 2020-12");
    draft202012::validate(&schema, &example_value())
        .expect("example route data must satisfy the repository schema");
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
fn example_route_data_drives_main_and_repeated_routes_to_completion() {
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

    let main_vehicle = world
        .spawn_vehicle(VehicleSpawnInput::active(
            "main-vehicle",
            "main-route",
            0,
            EdgeProgress::ZERO,
            main_speed,
        ))
        .expect("main vehicle spawns");
    let loop_vehicle = world
        .spawn_vehicle(VehicleSpawnInput::active(
            "loop-vehicle",
            "loop-once",
            0,
            EdgeProgress::ZERO,
            loop_speed,
        ))
        .expect("loop vehicle spawns");

    let result = world
        .step(TickInput::new(world.fixed_delta_time_ms()))
        .expect("example routes complete in one tick");

    assert_eq!(
        result.events,
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
            CoreEvent::VehicleCompletedRoute(VehicleCompletedRouteEvent {
                tick_index: 1,
                vehicle: main_vehicle,
                route: main_route,
                edge: exit,
                route_edge_index: 1,
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
