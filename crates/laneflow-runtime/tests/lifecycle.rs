use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
use laneflow_runtime::{
    ParkingError, PoseSource, RouteError, RouteRegisterInput, SpawnError, TrafficWorld,
    VehicleSpawnInput, WorldConfig,
};
use laneflow_static_contract::{
    EntityKind, LaneEdgeOrdinal, ParkingSpaceOrdinal, StaticRouteOrdinal, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-full-spatial/expected.lfca"
);

fn revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
    let input = check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD)
        .expect("checked canonical network input");
    build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("shared network revision")
}

fn world() -> TrafficWorld {
    TrafficWorld::install(revision(), WorldConfig::new(8, 4, 1, 100)).expect("install")
}

fn edge_for_length(world: &TrafficWorld, length: f64) -> LaneEdgeOrdinal {
    let index = world
        .traffic()
        .lane_lengths_meters()
        .iter()
        .position(|actual| *actual == length)
        .expect("fixture lane length");
    LaneEdgeOrdinal::try_from_usize(index).expect("fixture lane ordinal")
}

fn spawn_on_static(
    world: &mut TrafficWorld,
    progress: f64,
    speed: f64,
) -> laneflow_runtime::VehicleHandle {
    let route = world
        .static_route(StaticRouteOrdinal::from_raw(0))
        .expect("static route");
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            progress,
            speed,
        ))
        .expect("spawn")
}

#[test]
fn register_route_requires_connected_shared_edges() {
    let mut world = world();
    let first = edge_for_length(&world, 10.0);
    let middle = edge_for_length(&world, 8.0);
    let last = edge_for_length(&world, 12.0);

    assert_eq!(
        world
            .register_route(RouteRegisterInput::new(Vec::new()))
            .unwrap_err(),
        RouteError::EmptySequence
    );
    assert_eq!(
        world
            .register_route(RouteRegisterInput::new(vec![first, last]))
            .unwrap_err(),
        RouteError::Disconnected
    );
    assert_eq!(
        world
            .register_route(RouteRegisterInput::new(vec![LaneEdgeOrdinal::from_raw(
                u32::MAX
            )]))
            .unwrap_err(),
        RouteError::UnknownEdge
    );

    let route = world
        .register_route(RouteRegisterInput::new(vec![first, middle, last]))
        .expect("connected dynamic route");
    world.remove_route(route).expect("unused dynamic route");
    assert_eq!(
        world.remove_route(route).unwrap_err(),
        RouteError::StaleHandle
    );
}

#[test]
fn remove_route_rejects_static_handle() {
    let mut world = world();
    let route = world
        .static_route(StaticRouteOrdinal::from_raw(0))
        .expect("static route");
    assert_eq!(
        world.remove_route(route).unwrap_err(),
        RouteError::StaticHandle
    );
}

#[test]
fn spawn_respects_speed_limit_equality_and_overlap() {
    let mut world = world();
    let route = world
        .static_route(StaticRouteOrdinal::from_raw(0))
        .expect("static route");
    let edge = world
        .traffic()
        .relations()
        .static_route_edges(StaticRouteOrdinal::from_raw(0))
        .expect("static edges")[0];
    let limit = world.traffic().lane_speed_limits_meters_per_second()[edge.index()];

    let first = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0.0,
            limit,
        ))
        .expect("equal speed spawn");
    assert_eq!(
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                0.0,
                limit + 0.1,
            ))
            .unwrap_err(),
        SpawnError::SpeedExceedsLimit
    );
    assert_eq!(
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                0.0,
                0.0,
            ))
            .unwrap_err(),
        SpawnError::Overlap
    );

    let poses = world.committed_pose_sources();
    assert_eq!(poses.as_slice().len(), 1);
    assert_eq!(poses.as_slice()[0].0, first);
    assert!(matches!(
        poses.as_slice()[0].1,
        PoseSource::Lane { progress, .. } if progress == 0.0
    ));
}

#[test]
fn occupy_parking_enforces_one_to_one_and_same_space_idempotent() {
    let mut world = world();
    let spaces = world
        .traffic()
        .entity_counts()
        .count(EntityKind::ParkingSpace);
    assert!(spaces >= 1);
    let space = ParkingSpaceOrdinal::from_raw(0);
    let vehicle = spawn_on_static(&mut world, 0.0, 0.0);

    world.occupy_parking(vehicle, space).expect("occupy");
    world.occupy_parking(vehicle, space).expect("idempotent");
    assert_eq!(world.committed_parking_occupant(space), Some(vehicle));
    assert!(matches!(
        world.committed_pose_sources().as_slice()[0].1,
        PoseSource::Parking { space: occupied } if occupied == space
    ));

    if spaces >= 2 {
        let other = ParkingSpaceOrdinal::from_raw(1);
        assert_eq!(
            world.occupy_parking(vehicle, other).unwrap_err(),
            ParkingError::VehicleBoundToOtherSpace
        );
    }

    let other_vehicle = {
        let route = world
            .static_route(StaticRouteOrdinal::from_raw(0))
            .expect("static route");
        let profile = world
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .expect("profile");
        let edge = world
            .traffic()
            .relations()
            .static_route_edges(StaticRouteOrdinal::from_raw(0))
            .expect("edges")[0];
        let length = profile.length();
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                length + 0.5,
                0.0,
            ))
            .unwrap_or_else(|_| {
                panic!(
                    "second spawn at progress {} on edge length {}",
                    length + 0.5,
                    world.traffic().lane_lengths_meters()[edge.index()]
                );
            })
    };
    assert_eq!(
        world.occupy_parking(other_vehicle, space).unwrap_err(),
        ParkingError::SpaceOccupiedByOther
    );
}

#[test]
fn remove_dynamic_route_rejects_live_vehicle() {
    let mut world = world();
    let first = edge_for_length(&world, 10.0);
    let middle = edge_for_length(&world, 8.0);
    let last = edge_for_length(&world, 12.0);
    let route = world
        .register_route(RouteRegisterInput::new(vec![first, middle, last]))
        .expect("dynamic route");
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0.0,
            0.0,
        ))
        .expect("spawn on dynamic");
    assert_eq!(
        world.remove_route(route).unwrap_err(),
        RouteError::InUse { vehicle, route }
    );
}

#[test]
fn parking_releases_dynamic_route_so_remove_succeeds() {
    let mut world = world();
    let first = edge_for_length(&world, 10.0);
    let middle = edge_for_length(&world, 8.0);
    let last = edge_for_length(&world, 12.0);
    let route = world
        .register_route(RouteRegisterInput::new(vec![first, middle, last]))
        .expect("dynamic route");
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0.0,
            0.0,
        ))
        .expect("spawn on dynamic");
    world
        .occupy_parking(vehicle, ParkingSpaceOrdinal::from_raw(0))
        .expect("park");
    world
        .remove_route(route)
        .expect("parked vehicle does not pin route");
}

#[test]
fn spawn_rejects_out_of_range_index_and_progress() {
    let mut world = world();
    let route = world
        .static_route(StaticRouteOrdinal::from_raw(0))
        .expect("static route");
    assert_eq!(
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                99,
                0.0,
                0.0,
            ))
            .unwrap_err(),
        SpawnError::RouteIndexOutOfRange
    );
    let edge = world
        .traffic()
        .relations()
        .static_route_edges(StaticRouteOrdinal::from_raw(0))
        .expect("edges")[0];
    let length = world.traffic().lane_lengths_meters()[edge.index()];
    assert_eq!(
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                length + 1.0,
                0.0,
            ))
            .unwrap_err(),
        SpawnError::InvalidProgress
    );
}
