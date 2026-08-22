use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
use laneflow_runtime::{PoseSource, TickInput, TrafficWorld, VehicleSpawnInput, WorldConfig};
use laneflow_static_contract::{StaticRouteOrdinal, VehicleProfileOrdinal};
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

fn route_distance(world: &TrafficWorld, source: PoseSource) -> f64 {
    let PoseSource::Lane { edge, progress } = source else {
        panic!("expected lane pose");
    };
    let edges = world
        .traffic()
        .relations()
        .static_route_edges(StaticRouteOrdinal::from_raw(0))
        .expect("static edges");
    let lengths = world.traffic().lane_lengths_meters();
    let mut distance = 0.0;
    for current in edges {
        if *current == edge {
            return distance + progress;
        }
        distance += lengths[current.index()];
    }
    panic!("edge not on static route");
}

#[test]
fn follower_cannot_penetrate_leader_occupancy() {
    let mut world = world();
    let route = world
        .static_route(StaticRouteOrdinal::from_raw(0))
        .expect("static route");
    let profile = world
        .traffic()
        .relations()
        .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
        .expect("profile");
    let leader = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            profile.length() + 1.0,
            0.0,
        ))
        .expect("leader");
    let follower = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0.0,
            0.0,
        ))
        .expect("follower");

    for _ in 0..40 {
        world.step(TickInput::new(100)).expect("step");
    }

    let poses = world.committed_pose_sources();
    let leader_progress = poses
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == leader)
        .map(|(_, source)| route_distance(&world, *source))
        .expect("leader pose");
    let follower_progress = poses
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == follower)
        .map(|(_, source)| route_distance(&world, *source))
        .expect("follower pose");
    assert!(
        follower_progress + profile.length() <= leader_progress + 1e-6,
        "follower {follower_progress} penetrated leader {leader_progress} length {}",
        profile.length()
    );
}

#[test]
fn both_vehicles_can_advance_on_fixture_route() {
    let mut world = world();
    let route = world
        .static_route(StaticRouteOrdinal::from_raw(0))
        .expect("static route");
    let profile = world
        .traffic()
        .relations()
        .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
        .expect("profile");
    let follower_start = 1.0;
    let leader_start = follower_start + profile.length() + profile.min_gap() + 2.0;
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            leader_start,
            0.0,
        ))
        .expect("leader");
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            follower_start,
            0.0,
        ))
        .expect("follower");
    let before: Vec<f64> = world
        .committed_pose_sources()
        .as_slice()
        .iter()
        .map(|(_, source)| route_distance(&world, *source))
        .collect();
    for _ in 0..20 {
        world.step(TickInput::new(100)).expect("step");
    }
    let after: Vec<f64> = world
        .committed_pose_sources()
        .as_slice()
        .iter()
        .map(|(_, source)| route_distance(&world, *source))
        .collect();
    assert_eq!(before.len(), 2);
    assert!(
        after
            .iter()
            .zip(&before)
            .all(|(next, prev)| *next + 1e-9 >= *prev),
        "progress must not reverse: {before:?} -> {after:?}"
    );
    assert!(
        after
            .iter()
            .zip(&before)
            .all(|(next, prev)| *next > *prev + 1e-4),
        "both vehicles must advance: {before:?} -> {after:?}"
    );
}

#[test]
fn parked_vehicle_does_not_move() {
    let mut world = world();
    let route = world
        .static_route(StaticRouteOrdinal::from_raw(0))
        .expect("static route");
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0.0,
            0.0,
        ))
        .expect("spawn");
    let space = laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0);
    world.occupy_parking(vehicle, space).expect("occupy");
    world.step(TickInput::new(100)).expect("step");
    assert!(matches!(
        world.committed_pose_sources().as_slice()[0].1,
        laneflow_runtime::PoseSource::Parking { space: occupied } if occupied == space
    ));
}

#[test]
fn successful_step_publishes_signal_snapshot_for_t_plus_d() {
    let mut world = world();
    let at_zero = world.committed_signal_groups();
    world.step(TickInput::new(100)).expect("step");
    let after = world.committed_signal_groups();
    assert_eq!(at_zero.as_slice().len(), after.as_slice().len());
    assert_eq!(world.time_ms(), 100);
}

#[test]
fn identical_step_sequences_are_deterministic() {
    let run = || {
        let mut world = world();
        let route = world
            .static_route(StaticRouteOrdinal::from_raw(0))
            .expect("static route");
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                0.0,
                0.0,
            ))
            .expect("spawn");
        for _ in 0..8 {
            world.step(TickInput::new(100)).expect("step");
        }
        world
            .committed_pose_sources()
            .as_slice()
            .iter()
            .map(|(_, source)| *source)
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}
