use super::*;
use laneflow_runtime::{
    HandoffGeometryReadError, RouteHandoffRequest, SpawnError, TickInput, TrafficWorld,
    VehicleSpawnInput, deterministic_state_digest,
};

fn scene() -> (TrafficWorld, RouteHandoffRequest) {
    let revision = compile_road_editing_revision(module(0.));
    let config = WorldConfig::try_new(WorldConfigInput {
        vehicle_capacity: 4,
        route_capacity: 4,
        route_edge_occurrence_capacity: 64,
        route_conflict_occurrence_capacity: 32,
        fixed_delta_time_ms: 100,
    })
    .unwrap()
    .with_handoff_driving(registration::driving());
    let mut world = install_fixture(Arc::clone(&revision), config).unwrap();
    let (section, s, t) = stop_line::road_lanes(&revision);
    let source_index = 2;
    let route = world
        .register_route(
            RouteRegisterInput::new(vec![s[0], s[1], s[2], t[1], t[2]]).with_handoffs(vec![
                RouteHandoffOccurrence {
                    revision: revision.network_revision(),
                    source_index,
                    owner: BoundaryOwner::Section(section),
                    left_index: 0,
                    stations_mm: [24_000, 26_000],
                },
            ]),
        )
        .unwrap();
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            conflict_profile(&revision, "car"),
            route,
            1,
            3_400,
            2_000,
        ))
        .unwrap();
    (
        world,
        RouteHandoffRequest {
            vehicle,
            route,
            revision: revision.network_revision(),
            source_index,
            station_window_mm: [0, 50_000],
            rear_stations_mm: [10_000, 40_000],
            speed_m_s: 3.,
            limits: GeometryWorkLimits::STANDARD,
        },
    )
}

#[test]
fn parallel_traffic_on_a_source_edge_absent_from_the_registered_route_prevents_entry() {
    let (mut world, request) = scene();
    let revision = world.revision();
    let (_, src, _) = stop_line::road_lanes(&revision);
    assert!(!world.route_edges(request.route).unwrap().contains(&src[3]));
    let parallel = world
        .register_route(RouteRegisterInput::new(vec![src[3]]))
        .unwrap();
    let other = world
        .spawn_vehicle(VehicleSpawnInput::new(
            conflict_profile(&revision, "car"),
            parallel,
            0,
            4_200,
            1_000,
        ))
        .unwrap();
    world
        .step_with_route_handoffs(TickInput::new(100), &[request])
        .unwrap();
    assert!(
        world
            .vehicle_route_handoffs(request.vehicle)
            .next()
            .is_none()
    );
    assert!(
        world
            .committed_handoff_geometry(request.vehicle)
            .unwrap()
            .is_none()
    );
    assert!(world.vehicle(other).unwrap().progress_mm() > 4_200);
}

#[test]
fn a_committed_handoff_blocks_new_placement_and_despawn_releases_its_entire_reservation() {
    let (mut world, request) = scene();
    let revision = world.revision();
    let (_, _, target) = stop_line::road_lanes(&revision);
    let straight = world
        .register_route(RouteRegisterInput::new(target.to_vec()))
        .unwrap();
    let placement =
        VehicleSpawnInput::new(conflict_profile(&revision, "car"), straight, 1, 10_000, 0);
    world
        .step_with_route_handoffs(TickInput::new(100), &[request])
        .unwrap();
    let digest =
        |w: &TrafficWorld| deterministic_state_digest(&w.capture_snapshot().unwrap()).unwrap();
    let before = digest(&world);
    let pose = world
        .committed_handoff_geometry(request.vehicle)
        .unwrap()
        .unwrap();
    assert_eq!(
        (pose.tick, pose.revision, pose.route),
        (
            world.tick_index(),
            revision.network_revision(),
            request.route
        )
    );
    assert_eq!(pose.source_index, request.source_index);
    assert_eq!(
        pose.front_occurrence,
        world.vehicle(request.vehicle).unwrap().route_edge_index()
    );
    assert!(pose.pose.up[1][0] > 0.99);
    assert_eq!(
        world.spawn_vehicle(placement),
        Err(SpawnError::StopConstraintUnsatisfiable)
    );
    assert_eq!(digest(&world), before);
    world.despawn_vehicle(request.vehicle).unwrap();
    assert_eq!(
        world.committed_handoff_geometry(request.vehicle),
        Err(HandoffGeometryReadError::UnknownVehicle)
    );
    let replacement = world.spawn_vehicle(placement).unwrap();
    assert!(world.vehicle_route_handoffs(replacement).next().is_none());
}
