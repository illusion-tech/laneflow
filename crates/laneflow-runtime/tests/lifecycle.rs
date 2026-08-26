use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    ParkingError, PoseSource, ReplaceError, RouteError, RouteHandle, RouteRegisterInput,
    SpawnError, TickInput, TrafficWorld, VehicleSpawnInput, VehicleStatus, WorldConfig,
};
use laneflow_static_contract::{
    EntityKind, LaneEdgeOrdinal, ParkingSpaceOrdinal, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
);

fn revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
    let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
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

fn edge_for_length(world: &TrafficWorld, length: u32) -> LaneEdgeOrdinal {
    let index = world
        .traffic()
        .lane_lengths_millimetres()
        .iter()
        .position(|actual| *actual == length)
        .expect("fixture lane length");
    LaneEdgeOrdinal::try_from_usize(index).expect("fixture lane ordinal")
}

fn fixture_edges(world: &TrafficWorld) -> Vec<LaneEdgeOrdinal> {
    vec![
        edge_for_length(world, 10_000),
        edge_for_length(world, 8_000),
        edge_for_length(world, 12_000),
    ]
}

fn fixture_route(world: &mut TrafficWorld) -> RouteHandle {
    world
        .register_route(RouteRegisterInput::new(fixture_edges(world)))
        .expect("register")
}

fn spawn_on_route(
    world: &mut TrafficWorld,
    route: RouteHandle,
    progress: u32,
    speed: u32,
) -> laneflow_runtime::VehicleHandle {
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
    let first = edge_for_length(&world, 10_000);
    let middle = edge_for_length(&world, 8_000);
    let last = edge_for_length(&world, 12_000);

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
fn remove_route_unused_succeeds_and_in_use_when_occupied() {
    let mut world = world();
    let unused = fixture_route(&mut world);
    world.remove_route(unused).expect("unused registered route");
    assert_eq!(
        world.remove_route(unused).unwrap_err(),
        RouteError::StaleHandle
    );

    let route = fixture_route(&mut world);
    let vehicle = spawn_on_route(&mut world, route, 0, 0);
    assert_eq!(
        world.remove_route(route).unwrap_err(),
        RouteError::InUse { vehicle, route }
    );
}

#[test]
fn spawn_respects_speed_limit_equality_and_overlap() {
    let mut world = world();
    let route = fixture_route(&mut world);
    let edge = world.route_edges(route).expect("edges")[0];
    let limit = world.traffic().lane_speed_limits_millimetres_per_second()[edge.index()];

    let first = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            limit,
        ))
        .expect("equal speed spawn");
    assert_eq!(
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                0,
                limit + 1,
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
                0,
                0,
            ))
            .unwrap_err(),
        SpawnError::Overlap
    );

    let poses = world.committed_pose_sources();
    assert_eq!(poses.as_slice().len(), 1);
    assert_eq!(poses.as_slice()[0].0, first);
    assert!(matches!(
        poses.as_slice()[0].1,
        PoseSource::Lane { progress_mm, .. } if progress_mm == 0
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
    let route = fixture_route(&mut world);
    let vehicle = spawn_on_route(&mut world, route, 0, 0);

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
        let profile = world
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .expect("profile");
        let edge = world.route_edges(route).expect("edges")[0];
        let length = profile.length_mm();
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                length + 500,
                0,
            ))
            .unwrap_or_else(|_| {
                panic!(
                    "second spawn at progress {} on edge length {}",
                    length + 500,
                    world.traffic().lane_lengths_millimetres()[edge.index()]
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
    let first = edge_for_length(&world, 10_000);
    let middle = edge_for_length(&world, 8_000);
    let last = edge_for_length(&world, 12_000);
    let route = world
        .register_route(RouteRegisterInput::new(vec![first, middle, last]))
        .expect("dynamic route");
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            0,
        ))
        .expect("spawn on dynamic");
    assert_eq!(
        world.remove_route(route).unwrap_err(),
        RouteError::InUse { vehicle, route }
    );
}

#[test]
fn parking_keeps_dynamic_route_so_remove_fails() {
    let mut world = world();
    let first = edge_for_length(&world, 10_000);
    let middle = edge_for_length(&world, 8_000);
    let last = edge_for_length(&world, 12_000);
    let route = world
        .register_route(RouteRegisterInput::new(vec![first, middle, last]))
        .expect("dynamic route");
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            0,
        ))
        .expect("spawn on dynamic");
    world
        .occupy_parking(vehicle, ParkingSpaceOrdinal::from_raw(0))
        .expect("park");
    assert_eq!(
        world.remove_route(route).unwrap_err(),
        RouteError::InUse { vehicle, route }
    );
}

#[test]
fn spawn_rejects_out_of_range_index_and_progress() {
    let mut world = world();
    let route = fixture_route(&mut world);
    assert_eq!(
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                99,
                0,
                0,
            ))
            .unwrap_err(),
        SpawnError::RouteIndexOutOfRange
    );
    let edge = world.route_edges(route).expect("edges")[0];
    let length = world.traffic().lane_lengths_millimetres()[edge.index()];
    assert_eq!(
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                length + 1_000,
                0,
            ))
            .unwrap_err(),
        SpawnError::InvalidProgress
    );
}

fn drive_to_completed(
    world: &mut TrafficWorld,
    route: RouteHandle,
) -> laneflow_runtime::VehicleHandle {
    let edges = world.route_edges(route).expect("edges").to_vec();
    let last = *edges.last().expect("route has edges");
    let last_length = world.traffic().lane_lengths_millimetres()[last.index()];
    let speed_limit = world.traffic().lane_speed_limits_millimetres_per_second()[last.index()];
    let last_index = u32::try_from(edges.len() - 1).expect("index fits u32");
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            last_index,
            last_length.saturating_sub(500),
            speed_limit,
        ))
        .expect("spawn near end");
    for _ in 0..8 {
        world.step(TickInput::new(100)).expect("step");
        if world
            .vehicle(vehicle)
            .is_some_and(|state| state.status() == VehicleStatus::Completed)
        {
            break;
        }
    }
    assert_eq!(
        world.vehicle(vehicle).expect("retained").status(),
        VehicleStatus::Completed
    );
    vehicle
}

#[test]
fn completed_vehicle_is_retained_without_pose_or_occupancy() {
    let mut world = world();
    let route = fixture_route(&mut world);
    let old = drive_to_completed(&mut world, route);
    assert!(
        world
            .committed_pose_sources()
            .as_slice()
            .iter()
            .all(|(handle, _)| *handle != old),
        "Completed must leave committed_pose_sources"
    );
    assert_eq!(world.live_vehicles(), &[old]);
    let edges = world.route_edges(route).expect("edges").to_vec();
    let last = *edges.last().expect("route has edges");
    let last_length = world.traffic().lane_lengths_millimetres()[last.index()];
    let last_index = u32::try_from(edges.len() - 1).expect("index fits u32");
    let occupancy = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            last_index,
            last_length.saturating_sub(500),
            0,
        ))
        .expect("Completed must release lane occupancy");
    assert_ne!(occupancy, old);
    assert!(world.vehicle(old).is_some(), "old handle must stay live");
}

#[test]
fn completed_vehicle_occupies_capacity_until_replace() {
    let mut world =
        TrafficWorld::install(revision(), WorldConfig::new(1, 4, 1, 100)).expect("install");
    let route = fixture_route(&mut world);
    let old = drive_to_completed(&mut world, route);
    assert_eq!(
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                0,
                0,
            ))
            .unwrap_err(),
        SpawnError::CapacityExceeded
    );
    let record = world
        .replace_completed_vehicle(
            old,
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0, 0),
        )
        .expect("atomic replace");
    assert_eq!(record.old, old);
    assert_ne!(record.new, old);
    assert!(world.vehicle(old).is_none(), "old handle must be stale");
    assert_eq!(
        world.vehicle(record.new).expect("new").status(),
        VehicleStatus::Active
    );
    assert_eq!(world.live_vehicles(), &[record.new]);
}

#[test]
fn replace_is_atomic_and_blocked_overlap_is_retryable() {
    let mut world = world();
    let route = fixture_route(&mut world);
    let old = drive_to_completed(&mut world, route);
    let blocker = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            0,
        ))
        .expect("blocker at entry");
    let before = world.vehicle(old);
    let error = world
        .replace_completed_vehicle(
            old,
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0, 0),
        )
        .unwrap_err();
    let ReplaceError::Blocked(block) = error else {
        panic!("expected blocked replace, got {error:?}");
    };
    assert_eq!(block.old, old);
    assert_eq!(block.blocker, blocker);
    assert!(block.blocker_ahead, "entry spawn is behind the blocker");
    assert_eq!(world.vehicle(old), before, "Blocked must not mutate world");
    assert_eq!(world.live_vehicles(), &[old, blocker]);

    assert_eq!(
        world
            .replace_completed_vehicle(
                blocker,
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 1_000, 0),
            )
            .unwrap_err(),
        ReplaceError::NotCompleted
    );
    assert_eq!(
        world
            .replace_completed_vehicle(
                old,
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, u32::MAX, 0, 0,),
            )
            .unwrap_err(),
        ReplaceError::RouteIndexOutOfRange
    );
    assert_eq!(world.vehicle(old), before);

    let record = world
        .replace_completed_vehicle(
            old,
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 8_000, 0),
        )
        .expect("entry behind blocker");
    assert_ne!(record.new, old);
    assert!(world.vehicle(old).is_none());
    assert_eq!(world.live_vehicles(), &[record.new, blocker]);
}

#[test]
fn replace_does_not_use_despawn_then_spawn() {
    let mut world =
        TrafficWorld::install(revision(), WorldConfig::new(1, 4, 1, 100)).expect("install");
    let route = fixture_route(&mut world);
    let old = drive_to_completed(&mut world, route);
    assert!(
        world.vehicle(old).is_some(),
        "禁止先消失再生成：Completed 必须仍可读"
    );
    assert_eq!(
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                0,
                0,
            ))
            .unwrap_err(),
        SpawnError::CapacityExceeded,
        "不得把跑完即退役再 spawn 写成回流"
    );
}

#[test]
fn completed_dynamic_route_stays_referenced_until_replace() {
    let mut world = world();
    let first = edge_for_length(&world, 10_000);
    let middle = edge_for_length(&world, 8_000);
    let last = edge_for_length(&world, 12_000);
    let dynamic = world
        .register_route(RouteRegisterInput::new(vec![first, middle, last]))
        .expect("dynamic");
    let last_length = world.traffic().lane_lengths_millimetres()[last.index()];
    let speed_limit = world.traffic().lane_speed_limits_millimetres_per_second()[last.index()];
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            dynamic,
            2,
            last_length.saturating_sub(500),
            speed_limit,
        ))
        .expect("spawn near end");
    for _ in 0..8 {
        world.step(TickInput::new(100)).expect("step");
        if world
            .vehicle(vehicle)
            .is_some_and(|state| state.status() == VehicleStatus::Completed)
        {
            break;
        }
    }
    assert_eq!(
        world.vehicle(vehicle).expect("retained").status(),
        VehicleStatus::Completed
    );
    assert_eq!(
        world.remove_route(dynamic).unwrap_err(),
        RouteError::InUse {
            vehicle,
            route: dynamic
        }
    );
    world.step(TickInput::new(100)).expect("extra step");
    assert_eq!(
        world.vehicle(vehicle).expect("no timeout retire").status(),
        VehicleStatus::Completed
    );

    let replacement = fixture_route(&mut world);
    let record = world
        .replace_completed_vehicle(
            vehicle,
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), replacement, 0, 0, 0),
        )
        .expect("replace onto another registered route");
    world.remove_route(dynamic).expect("old dynamic unused");
    assert!(world.vehicle(record.new).is_some());
}

#[test]
fn parked_and_stale_replace_leave_world_unchanged() {
    let mut world = world();
    let route = fixture_route(&mut world);
    let parked = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            0,
        ))
        .expect("spawn");
    world
        .occupy_parking(parked, ParkingSpaceOrdinal::from_raw(0))
        .expect("park");
    assert_eq!(
        world
            .replace_completed_vehicle(
                parked,
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 1_000, 0),
            )
            .unwrap_err(),
        ReplaceError::NotCompleted
    );
    assert_eq!(
        world.vehicle(parked).expect("unchanged").status(),
        VehicleStatus::Parked
    );

    let stale = parked;
    let old = drive_to_completed(&mut world, route);
    world
        .replace_completed_vehicle(
            old,
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 8_000, 0),
        )
        .expect("free the completed slot");
    assert_eq!(
        world
            .replace_completed_vehicle(
                old,
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 8_000, 0),
            )
            .unwrap_err(),
        ReplaceError::StaleHandle
    );
    let _ = stale;
}
