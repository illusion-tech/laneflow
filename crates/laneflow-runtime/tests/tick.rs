use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    PoseSource, RouteHandle, RouteRegisterInput, SpawnError, TickInput, TrafficWorld,
    VehicleSpawnInput, VehicleStatus, WorldConfig,
};
use laneflow_static_contract::{
    EntityKind, LaneEdgeOrdinal, SignalAspect, SignalControllerOrdinal, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

fn install_fixture(
    revision: std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
    config: laneflow_runtime::WorldConfig,
) -> Result<laneflow_runtime::TrafficWorld, laneflow_runtime::InstallError> {
    let origin = *revision.canonical_origin();
    laneflow_runtime::TrafficWorld::install(
        revision,
        config,
        laneflow_runtime::CommittedNetworkSource::Published {
            reference: laneflow_runtime::PublishedLfcaReference::new(
                "fixture://in-process",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("non-empty fixture key"),
        },
        0,
    )
}

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
    world_with_delta(100)
}

fn world_with_delta(delta_ms: u64) -> TrafficWorld {
    install_fixture(revision(), WorldConfig::new(8, 4, 1_024, 1, delta_ms)).expect("install")
}

fn aspects_at(world: &TrafficWorld, time_ms: u64) -> Vec<SignalAspect> {
    let group_count = usize::try_from(
        world
            .traffic()
            .entity_counts()
            .count(EntityKind::SignalGroup),
    )
    .expect("signal group count fits usize");
    let mut aspects = vec![SignalAspect::Red; group_count];
    let relations = world.traffic().relations();
    let controller_count = world
        .traffic()
        .entity_counts()
        .count(EntityKind::SignalController);
    for raw in 0..controller_count {
        let controller = SignalControllerOrdinal::from_raw(raw);
        let Some(view) = relations.signal_controller(controller) else {
            continue;
        };
        let cycle_ms = view.cycle_ms();
        if cycle_ms == 0 || view.phases().is_empty() {
            continue;
        }
        let position = u64::try_from(
            (u128::from(time_ms) + u128::from(view.offset_ms())) % u128::from(cycle_ms),
        )
        .expect("cycle position fits u64");
        let phases = view.phases();
        let phase_index = phases.partition_point(|phase| {
            relations.phase_end_offset_ms(*phase).unwrap_or(0) <= position
        });
        let Some(phase) = phases.get(phase_index).copied() else {
            continue;
        };
        let Some((groups, values)) = relations.phase_states(phase) else {
            continue;
        };
        for (group, aspect) in groups.iter().copied().zip(values.iter().copied()) {
            if let Some(slot) = aspects.get_mut(group.index()) {
                *slot = aspect;
            }
        }
    }
    aspects
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

fn bumper_gap(
    world: &TrafficWorld,
    edges: &[LaneEdgeOrdinal],
    leader: laneflow_runtime::VehicleHandle,
    follower: laneflow_runtime::VehicleHandle,
    length: u32,
) -> i64 {
    let poses = world.committed_pose_sources();
    let leader_progress = poses
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == leader)
        .map(|(_, source)| route_distance(world, edges, *source))
        .expect("leader pose");
    let follower_progress = poses
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == follower)
        .map(|(_, source)| route_distance(world, edges, *source))
        .expect("follower pose");
    i64::from(leader_progress) - i64::from(follower_progress) - i64::from(length)
}

fn route_distance(world: &TrafficWorld, edges: &[LaneEdgeOrdinal], source: PoseSource) -> u32 {
    let PoseSource::Lane { edge, progress_mm } = source else {
        panic!("expected lane pose");
    };
    let lengths = world.traffic().lane_lengths_millimetres();
    let mut distance: u32 = 0;
    for current in edges {
        if *current == edge {
            return distance.saturating_add(progress_mm);
        }
        distance = distance.saturating_add(lengths[current.index()]);
    }
    panic!("edge not on registered route");
}

#[test]
fn follower_cannot_penetrate_leader_occupancy() {
    let mut world = world();
    let route = fixture_route(&mut world);
    let edges = world.route_edges(route).expect("edges").to_vec();
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
            profile.length_mm() + 1_000,
            0,
        ))
        .expect("leader");
    let follower = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            0,
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
        .map(|(_, source)| route_distance(&world, &edges, *source))
        .expect("leader pose");
    let follower_progress = poses
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == follower)
        .map(|(_, source)| route_distance(&world, &edges, *source))
        .expect("follower pose");
    assert!(
        follower_progress + profile.length_mm() <= leader_progress,
        "follower {follower_progress} penetrated leader {leader_progress} length {}",
        profile.length_mm()
    );
}

#[test]
fn both_vehicles_can_advance_on_fixture_route() {
    let mut world = world();
    let route = fixture_route(&mut world);
    let edges = world.route_edges(route).expect("edges").to_vec();
    let profile = world
        .traffic()
        .relations()
        .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
        .expect("profile");
    let follower_start = 1_000;
    let leader_start = follower_start + profile.length_mm() + profile.min_gap_mm() + 2_000;
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            leader_start,
            0,
        ))
        .expect("leader");
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            follower_start,
            0,
        ))
        .expect("follower");
    let before: Vec<u32> = world
        .committed_pose_sources()
        .as_slice()
        .iter()
        .map(|(_, source)| route_distance(&world, &edges, *source))
        .collect();
    for _ in 0..20 {
        world.step(TickInput::new(100)).expect("step");
    }
    let after: Vec<u32> = world
        .committed_pose_sources()
        .as_slice()
        .iter()
        .map(|(_, source)| route_distance(&world, &edges, *source))
        .collect();
    assert_eq!(before.len(), 2);
    assert!(
        after.iter().zip(&before).all(|(next, prev)| *next >= *prev),
        "progress must not reverse: {before:?} -> {after:?}"
    );
    assert!(
        after.iter().zip(&before).all(|(next, prev)| *next > *prev),
        "both vehicles must advance: {before:?} -> {after:?}"
    );
}

#[test]
fn parked_vehicle_does_not_move() {
    let mut world = world();
    let route = fixture_route(&mut world);
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            0,
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
        let route = fixture_route(&mut world);
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                0,
                0,
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

#[test]
fn min_gap_is_preserved_when_spawn_gap_is_feasible() {
    let mut world = world();
    let route = fixture_route(&mut world);
    let edges = world.route_edges(route).expect("edges").to_vec();
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
            profile.length_mm() + profile.min_gap_mm() + 500,
            0,
        ))
        .expect("leader");
    let follower = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            0,
        ))
        .expect("follower");
    let initial_gap = bumper_gap(&world, &edges, leader, follower, profile.length_mm());
    assert!(initial_gap >= i64::from(profile.min_gap_mm()));
    for _ in 0..40 {
        world.step(TickInput::new(100)).expect("step");
    }
    let gap = bumper_gap(&world, &edges, leader, follower, profile.length_mm());
    assert!(
        gap >= i64::from(profile.min_gap_mm()),
        "min_gap invaded: {gap} < {}",
        profile.min_gap_mm()
    );
}

#[test]
fn follower_is_observably_constrained_versus_solo() {
    let profile_of = |world: &TrafficWorld| {
        world
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .expect("profile")
    };
    let mut paired = world();
    let profile = profile_of(&paired);
    let route = fixture_route(&mut paired);
    let paired_edges = paired.route_edges(route).expect("edges").to_vec();
    paired
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            profile.length_mm() + profile.min_gap_mm() + 500,
            0,
        ))
        .expect("leader");
    let follower = paired
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            0,
        ))
        .expect("follower");
    let mut solo = world();
    let solo_route = fixture_route(&mut solo);
    let solo_edges = solo.route_edges(solo_route).expect("edges").to_vec();
    let solo_vehicle = solo
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            solo_route,
            0,
            0,
            0,
        ))
        .expect("solo");
    for _ in 0..20 {
        paired.step(TickInput::new(100)).expect("step");
        solo.step(TickInput::new(100)).expect("step");
    }
    let follower_distance = route_distance(
        &paired,
        &paired_edges,
        paired
            .committed_pose_sources()
            .as_slice()
            .iter()
            .find(|(handle, _)| *handle == follower)
            .expect("follower pose")
            .1,
    );
    let solo_distance = route_distance(
        &solo,
        &solo_edges,
        solo.committed_pose_sources()
            .as_slice()
            .iter()
            .find(|(handle, _)| *handle == solo_vehicle)
            .expect("solo pose")
            .1,
    );
    assert!(
        follower_distance + 50 < solo_distance,
        "follower {follower_distance} should lag solo {solo_distance}"
    );
}

#[test]
fn red_snapshot_prevents_controlled_transition() {
    let mut world = world();
    let route = fixture_route(&mut world);
    let edges = world.route_edges(route).expect("edges").to_vec();
    let from = edges[1];
    let speed_limit = world.traffic().lane_speed_limits_millimetres_per_second()[from.index()];
    let t_aspects = aspects_at(&world, world.time_ms());
    assert_eq!(t_aspects.get(1).copied(), Some(SignalAspect::Red));
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            1,
            7_200,
            speed_limit,
        ))
        .expect("spawn");
    world.step(TickInput::new(100)).expect("step");
    let PoseSource::Lane { edge, progress_mm } = world.committed_pose_sources().as_slice()[0].1
    else {
        panic!("expected lane pose");
    };
    assert_eq!(edge, from, "must not complete fromEdge → toEdge on Red");
    assert!(
        progress_mm <= 8_000,
        "front bumper must not pass StopLine {progress_mm}"
    );
}

#[test]
fn phase_boundary_inside_tick_keeps_snapshot_t_and_publishes_t_plus_d() {
    const DELTA: u64 = 200;
    let mut world = world_with_delta(DELTA);
    while world.time_ms() < 28_800 {
        world.step(TickInput::new(DELTA)).expect("advance");
    }
    assert_eq!(world.time_ms(), 28_800);
    let t = world.time_ms();
    let snapshot_t = aspects_at(&world, t);
    let snapshot_td = aspects_at(&world, t + DELTA);
    assert_eq!(
        world
            .committed_signal_groups()
            .as_slice()
            .iter()
            .map(|(_, aspect)| *aspect)
            .collect::<Vec<_>>(),
        snapshot_t
    );
    assert_ne!(
        snapshot_t, snapshot_td,
        "phase boundary must fall in [T, T+D)"
    );
    assert_eq!(snapshot_t.first().copied(), Some(SignalAspect::Green));
    assert_eq!(snapshot_td.first().copied(), Some(SignalAspect::Yellow));

    let route = fixture_route(&mut world);
    let edges = world.route_edges(route).expect("edges").to_vec();
    let to = edges[1];
    let speed_limit = world.traffic().lane_speed_limits_millimetres_per_second()[edges[0].index()];
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            9_000,
            speed_limit,
        ))
        .expect("spawn");
    world.step(TickInput::new(DELTA)).expect("step");
    let PoseSource::Lane { edge, progress_mm } = world.committed_pose_sources().as_slice()[0].1
    else {
        panic!("expected lane pose");
    };
    assert_eq!(
        edge, to,
        "snapshot(T) is Green so the vehicle must complete entry StopLine; got edge={edge:?} progress={progress_mm}"
    );
    let committed: Vec<SignalAspect> = world
        .committed_signal_groups()
        .as_slice()
        .iter()
        .map(|(_, aspect)| *aspect)
        .collect();
    assert_eq!(committed, snapshot_td);
    assert_eq!(world.time_ms(), t + DELTA);
}

#[test]
fn successful_step_matches_program_aspect_at_t_plus_d() {
    let mut world = world();
    let before = aspects_at(&world, world.time_ms());
    assert_eq!(
        world
            .committed_signal_groups()
            .as_slice()
            .iter()
            .map(|(_, aspect)| *aspect)
            .collect::<Vec<_>>(),
        before
    );
    world.step(TickInput::new(100)).expect("step");
    let expected = aspects_at(&world, world.time_ms());
    assert_eq!(
        world
            .committed_signal_groups()
            .as_slice()
            .iter()
            .map(|(_, aspect)| *aspect)
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn failed_step_leaves_pose_occupancy_signals_and_time_unchanged() {
    let mut world = world();
    let route = fixture_route(&mut world);
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            0,
        ))
        .expect("spawn");
    let poses = world.committed_pose_sources();
    let signals = world.committed_signal_groups();
    let time = world.time_ms();
    let tick = world.tick_index();
    let occupant = world
        .committed_parking_occupant(laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0));
    assert!(world.step(TickInput::new(50)).is_err());
    assert_eq!(world.committed_pose_sources(), poses);
    assert_eq!(world.committed_signal_groups(), signals);
    assert_eq!(world.time_ms(), time);
    assert_eq!(world.tick_index(), tick);
    assert_eq!(
        world
            .committed_parking_occupant(laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0)),
        occupant
    );
    assert_eq!(world.committed_pose_sources().as_slice()[0].0, vehicle);
}

#[test]
fn spawn_at_vacated_progress_succeeds_after_leader_advances() {
    let mut world = world();
    let route = fixture_route(&mut world);
    let edges = world.route_edges(route).expect("edges").to_vec();
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
            0,
            0,
        ))
        .expect("leader");
    let mut leader_progress = 0;
    for _ in 0..80 {
        world.step(TickInput::new(100)).expect("step");
        leader_progress = route_distance(
            &world,
            &edges,
            world
                .committed_pose_sources()
                .as_slice()
                .iter()
                .find(|(handle, _)| *handle == leader)
                .expect("leader pose")
                .1,
        );
        if leader_progress > profile.length_mm() + profile.min_gap_mm() {
            break;
        }
    }
    assert!(
        leader_progress > profile.length_mm() + profile.min_gap_mm(),
        "leader should vacate the origin, got {leader_progress}"
    );
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            0,
        ))
        .expect("vacated origin must accept a new vehicle");
    let PoseSource::Lane {
        edge: leader_edge,
        progress_mm: leader_edge_progress,
    } = world
        .committed_pose_sources()
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == leader)
        .expect("leader pose")
        .1
    else {
        panic!("leader must remain on a lane");
    };
    let leader_index = edges
        .iter()
        .position(|edge| *edge == leader_edge)
        .expect("leader edge on route");
    assert_eq!(
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                u32::try_from(leader_index).expect("index fits u32"),
                leader_edge_progress,
                0,
            ))
            .unwrap_err(),
        SpawnError::Overlap
    );
}

#[test]
fn route_end_leaves_committed_poses_and_lane_occupancy() {
    let mut world = world();
    let route = fixture_route(&mut world);
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
            .committed_pose_sources()
            .as_slice()
            .iter()
            .all(|(handle, _)| *handle != vehicle)
        {
            break;
        }
    }
    assert!(
        world
            .committed_pose_sources()
            .as_slice()
            .iter()
            .all(|(handle, _)| *handle != vehicle),
        "completed vehicle must leave committed_pose_sources"
    );
    assert_eq!(
        world.vehicle(vehicle).expect("retained").status(),
        VehicleStatus::Completed
    );
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            last_index,
            last_length.saturating_sub(500),
            0,
        ))
        .expect("route-end occupancy must be released");
    assert!(
        world.vehicle(vehicle).is_some(),
        "Completed handle must stay live after occupancy-releasing spawn"
    );
}

#[test]
fn later_red_stop_caps_travel_after_permitted_gate() {
    let mut world = world_with_delta(1_000);
    let route = fixture_route(&mut world);
    let edges = world.route_edges(route).expect("edges").to_vec();
    let speed_limit = world.traffic().lane_speed_limits_millimetres_per_second()[edges[0].index()];
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            9_000,
            speed_limit,
        ))
        .expect("spawn");
    world.step(TickInput::new(1_000)).expect("step");
    let PoseSource::Lane { edge, progress_mm } = world.committed_pose_sources().as_slice()[0].1
    else {
        panic!("expected lane pose");
    };
    assert_eq!(edge, edges[1], "must stop at later red, not skip middle");
    assert!(
        progress_mm <= 8_000,
        "must not enter exit on later red {progress_mm}"
    );
}

#[test]
fn later_red_uses_compiled_path_gate() {
    let mut world = world_with_delta(1_000);
    let edges = fixture_edges(&world);
    let route = world
        .register_route(RouteRegisterInput::new(edges.clone()))
        .expect("register");
    let speed_limit = world.traffic().lane_speed_limits_millimetres_per_second()[edges[0].index()];
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            9_000,
            speed_limit,
        ))
        .expect("spawn");
    world.step(TickInput::new(1_000)).expect("step");
    let PoseSource::Lane { edge, progress_mm } = world.committed_pose_sources().as_slice()[0].1
    else {
        panic!("expected lane pose");
    };
    assert_eq!(edge, edges[1], "dynamic later red must stop on middle");
    assert!(
        progress_mm <= 8_000,
        "dynamic later red must not enter exit {progress_mm}"
    );
}

#[test]
fn registered_vehicles_follow_on_shared_edges() {
    let mut world = world();
    let route = fixture_route(&mut world);
    let edges = world.route_edges(route).expect("edges").to_vec();
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
            profile.length_mm() + profile.min_gap_mm() + 500,
            0,
        ))
        .expect("leader");
    let follower = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            0,
        ))
        .expect("follower");
    for _ in 0..20 {
        world.step(TickInput::new(100)).expect("step");
    }
    let gap = bumper_gap(&world, &edges, leader, follower, profile.length_mm());
    assert!(
        gap >= i64::from(profile.min_gap_mm()),
        "shared-edge following must keep min_gap, got {gap}"
    );
}

#[test]
fn spawn_rejects_overlap_across_adjacent_edges() {
    let mut world = world();
    let route = fixture_route(&mut world);
    let profile = world
        .traffic()
        .relations()
        .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
        .expect("profile");
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            1,
            1_000,
            0,
        ))
        .expect("leader on middle");
    assert_eq!(
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                8_000,
                0,
            ))
            .unwrap_err(),
        SpawnError::Overlap
    );
    let _ = profile;
}

#[test]
fn completed_vehicle_keeps_capacity_until_replace() {
    let mut world =
        install_fixture(revision(), WorldConfig::new(1, 4, 1_024, 1, 100)).expect("install");
    let route = fixture_route(&mut world);
    let edges = world.route_edges(route).expect("edges").to_vec();
    let last = *edges.last().expect("route has edges");
    let last_length = world.traffic().lane_lengths_millimetres()[last.index()];
    let speed_limit = world.traffic().lane_speed_limits_millimetres_per_second()[last.index()];
    let last_index = u32::try_from(edges.len() - 1).expect("index fits u32");
    let old = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            last_index,
            last_length.saturating_sub(500),
            speed_limit,
        ))
        .expect("only slot");
    for _ in 0..8 {
        world.step(TickInput::new(100)).expect("step");
        if world.committed_pose_sources().as_slice().is_empty() {
            break;
        }
    }
    assert!(world.committed_pose_sources().as_slice().is_empty());
    assert_eq!(
        world.vehicle(old).expect("retained").status(),
        VehicleStatus::Completed
    );
    assert_eq!(
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                last_index,
                0,
                0,
            ))
            .unwrap_err(),
        SpawnError::CapacityExceeded
    );
    world
        .replace_completed_vehicle(
            old,
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0, 0),
        )
        .expect("capacity rotates only via replace");
    assert!(
        world.vehicle(old).is_none(),
        "replaced handle must be stale"
    );
}
