use std::{alloc::System, hint::black_box};

use laneflow_core::{
    CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge, LaneGraph,
    Route, Speed, SpeedLimit, VehicleProfile, VehicleProfileHandle, VehicleProfileRegistry,
    VehicleReplaceExternalId, VehicleReplaceInput, VehicleReplaceOutcome, VehicleSpawnInput,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn measure<T>(operation: impl FnOnce() -> T) -> (T, Stats) {
    let region = Region::new(GLOBAL);
    let output = operation();
    black_box(&output);
    let change = black_box(region.change());
    (output, change)
}

fn assert_zero_allocation(label: &str, stats: Stats) {
    assert_eq!(stats.allocations, 0, "{label}: allocation count");
    assert_eq!(stats.reallocations, 0, "{label}: reallocation count");
    assert_eq!(stats.bytes_allocated, 0, "{label}: allocated bytes");
    assert_eq!(stats.bytes_reallocated, 0, "{label}: reallocated bytes");
}

fn allocation_world(with_blocker: bool) -> (CoreWorld, VehicleProfileHandle) {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new(
            "old-edge",
            EdgeLength::try_new(100.0).expect("old edge length"),
            SpeedLimit::try_new(20.0).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
        LaneEdge::new(
            "target-edge",
            EdgeLength::try_new(100.0).expect("target edge length"),
            SpeedLimit::try_new(20.0).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("valid graph");
    let profiles = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        "profile",
        IidmProfileSpec {
            length: 4.5,
            desired_speed: 13.9,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 1.4,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 4.0,
        },
    )
    .expect("valid profile")])
    .expect("valid profile registry");
    let profile = profiles.profile_handle("profile").expect("profile handle");
    let traffic = InitialTrafficData::try_new(
        lane_graph,
        [
            Route::try_new("old-route", ["old-edge"]).expect("old route"),
            Route::try_new("target-route", ["target-edge"]).expect("target route"),
        ],
        profiles,
    )
    .expect("valid traffic");
    let mut vehicles = vec![VehicleSpawnInput::completed(
        "old",
        profile,
        "old-route",
        0,
        EdgeProgress::try_new(100.0).expect("route end"),
    )];
    if with_blocker {
        vehicles.push(VehicleSpawnInput::active(
            "blocker",
            profile,
            "target-route",
            0,
            EdgeProgress::try_new(1.0).expect("blocker progress"),
            Speed::ZERO,
        ));
    }
    (
        CoreWorld::with_traffic_data(20, traffic, vehicles).expect("allocation world"),
        profile,
    )
}

fn input(
    world: &CoreWorld,
    profile: VehicleProfileHandle,
    external_id: VehicleReplaceExternalId,
) -> VehicleReplaceInput {
    VehicleReplaceInput::new(
        external_id,
        profile,
        world.route_handle("target-route").expect("target route"),
        0,
        EdgeProgress::ZERO,
        Speed::ZERO,
    )
}

fn warm_success_capacities(world: &mut CoreWorld, profile: VehicleProfileHandle) {
    let temporary = world
        .spawn_vehicle(VehicleSpawnInput::active(
            "temporary",
            profile,
            "target-route",
            0,
            EdgeProgress::try_new(50.0).expect("temporary progress"),
            Speed::ZERO,
        ))
        .expect("temporary spawn");
    world.despawn_vehicle(temporary).expect("temporary despawn");
}

#[test]
fn warm_preserve_success_and_blocked_retry_are_allocation_free() {
    let (mut success_world, profile) = allocation_world(false);
    warm_success_capacities(&mut success_world, profile);
    let old = success_world.vehicle_handle("old").expect("old handle");
    let preserve = input(&success_world, profile, VehicleReplaceExternalId::Preserve);
    let (outcome, success_stats) = measure(|| {
        success_world
            .replace_completed_vehicle(old, &preserve)
            .expect("preserve replacement")
    });
    assert!(matches!(outcome, VehicleReplaceOutcome::Replaced(_)));
    assert_zero_allocation("warm Preserve success", success_stats);

    let (mut blocked_world, profile) = allocation_world(true);
    let old = blocked_world.vehicle_handle("old").expect("old handle");
    let preserve = input(&blocked_world, profile, VehicleReplaceExternalId::Preserve);
    let warm = blocked_world
        .replace_completed_vehicle(old, &preserve)
        .expect("warm blocked retry");
    assert!(matches!(warm, VehicleReplaceOutcome::Blocked(_)));
    let (outcome, blocked_stats) = measure(|| {
        blocked_world
            .replace_completed_vehicle(old, &preserve)
            .expect("measured blocked retry")
    });
    assert!(matches!(outcome, VehicleReplaceOutcome::Blocked(_)));
    assert_zero_allocation("warm blocked retry", blocked_stats);

    assert_replace_with_allocation_bound();
}

fn assert_replace_with_allocation_bound() {
    let (mut world, profile) = allocation_world(false);
    warm_success_capacities(&mut world, profile);
    let old = world.vehicle_handle("old").expect("old handle");
    let replacement_id = "new-id";
    let replace_with = input(
        &world,
        profile,
        VehicleReplaceExternalId::ReplaceWith(replacement_id.to_owned()),
    );

    let (outcome, stats) = measure(|| {
        world
            .replace_completed_vehicle(old, &replace_with)
            .expect("new ID replacement")
    });
    assert!(matches!(outcome, VehicleReplaceOutcome::Replaced(_)));
    assert_eq!(stats.allocations, 2, "slot and resolver each own one ID");
    assert_eq!(stats.reallocations, 0);
    assert_eq!(stats.bytes_allocated, replacement_id.len() * 2);
    assert_eq!(stats.bytes_reallocated, 0);
}
