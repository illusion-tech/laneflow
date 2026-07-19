mod common;

use common::{test_profile, world_with_test_profile};
use laneflow_core::{
    CoreError, CoreEvent, CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData,
    LaneEdge, LaneGraph, Route, Speed, TickInput, VehicleProfile, VehicleProfileHandle,
    VehicleProfileRegistry, VehicleSpawnInput,
};

fn edge_length(value: f64) -> EdgeLength {
    EdgeLength::try_from(value).expect("valid edge length")
}

fn progress(value: f64) -> EdgeProgress {
    EdgeProgress::try_new(value).expect("valid progress")
}

fn speed(value: f64) -> Speed {
    Speed::try_from(value).expect("valid speed")
}

fn route_world(vehicles: impl FnOnce(VehicleProfileHandle) -> Vec<VehicleSpawnInput>) -> CoreWorld {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new("A", edge_length(1.000_001), ["B"]),
        LaneEdge::new("B", edge_length(1.000_001), std::iter::empty::<&str>()),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A", "B"]).expect("valid route");

    world_with_test_profile(1_000, lane_graph, [route], vehicles).expect("valid world")
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
    let mut world = route_world(|profile| {
        vec![VehicleSpawnInput::active(
            "V1",
            profile,
            "R",
            0,
            progress(0.0),
            speed(0.0),
        )]
    });
    let profile = world
        .vehicle_profile_handle("test-profile")
        .expect("profile handle exists");
    let old_handle = world.vehicle_handle("V1").expect("vehicle handle exists");

    let record = world.despawn_vehicle(old_handle).expect("despawn succeeds");

    assert_eq!(record.external_id, "V1");
    assert_eq!(record.profile, profile);
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
            profile,
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
fn world_resolves_profile_and_rejects_unknown_profile_handle() {
    let mut world = route_world(|_| Vec::new());
    let profile = world
        .vehicle_profile_handle("test-profile")
        .expect("profile handle exists");

    assert_eq!(
        world
            .vehicle_profile(profile)
            .expect("profile resolves")
            .external_id(),
        "test-profile"
    );
    assert_eq!(
        world.vehicle_profile_external_id(profile),
        Some("test-profile")
    );

    let foreign_registry =
        VehicleProfileRegistry::try_new([test_profile("foreign-a"), test_profile("foreign-b")])
            .expect("foreign registry is valid");
    let unknown_profile = foreign_registry
        .profile_handle("foreign-b")
        .expect("foreign profile handle exists");
    let before = world.clone();

    let error = world
        .spawn_vehicle(VehicleSpawnInput::active(
            "V1",
            unknown_profile,
            "R",
            0,
            progress(0.0),
            speed(1.0),
        ))
        .expect_err("out-of-range profile handle must fail");

    std::assert_matches!(
        error,
        CoreError::UnknownVehicleProfileHandle {
            vehicle_id,
            profile: actual_profile
        } if vehicle_id == "V1" && actual_profile == unknown_profile
    );
    assert_eq!(world, before);
}

#[test]
fn route_remove_rejects_live_vehicle_and_stales_old_handle() {
    let mut world = route_world(|profile| {
        vec![VehicleSpawnInput::active(
            "V1",
            profile,
            "R",
            0,
            progress(0.0),
            speed(0.0),
        )]
    });
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
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new("A", edge_length(1.000_001), ["B"]),
        LaneEdge::new("B", edge_length(1.000_001), std::iter::empty::<&str>()),
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
    let mut world = CoreWorld::with_traffic_data(
        1_000,
        traffic_data,
        vec![VehicleSpawnInput::active(
            "V2",
            profile,
            "R",
            0,
            progress(0.501),
            speed(2.0),
        )],
    )
    .expect("valid world");
    world
        .spawn_vehicle(VehicleSpawnInput::active(
            "V1",
            profile,
            "R",
            0,
            progress(0.001),
            speed(2.0),
        ))
        .expect("spawn succeeds");

    let result = world.step(TickInput::new(1_000)).expect("step succeeds");

    assert_eq!(event_vehicle_ids(&world, &result.events), ["V2", "V1"]);
}
