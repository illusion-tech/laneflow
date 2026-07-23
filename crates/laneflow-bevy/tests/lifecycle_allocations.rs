use std::{alloc::System, hint::black_box, num::NonZeroU32};

use bevy_ecs::world::World;
use laneflow_bevy::{
    LaneFlowSession, LaneFlowSessionConfig, LaneFlowVehicleReplaceOutcome,
    replace_completed_vehicle,
};
use laneflow_core::{
    CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge, LaneGraph,
    Route, Speed, SpeedLimit, VehicleProfile, VehicleProfileHandle, VehicleProfileRegistry,
    VehicleReplaceExternalId, VehicleReplaceInput, VehicleSpawnInput,
};
use laneflow_spatial::{CanonicalFrameId, SpatialRegistry};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn progress(value: f64) -> EdgeProgress {
    EdgeProgress::try_new(value).expect("valid progress")
}

fn allocation_world(with_blocker: bool) -> (CoreWorld, VehicleProfileHandle) {
    let graph = LaneGraph::try_new([
        LaneEdge::new(
            "old-edge",
            EdgeLength::try_new(100.0).expect("old length"),
            SpeedLimit::try_new(20.0).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
        LaneEdge::new(
            "target-edge",
            EdgeLength::try_new(100.0).expect("target length"),
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
    .expect("valid profiles");
    let profile = profiles.profile_handle("profile").expect("profile handle");
    let traffic = InitialTrafficData::try_new(
        graph,
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
        progress(100.0),
    )];
    if with_blocker {
        vehicles.push(VehicleSpawnInput::active(
            "blocker",
            profile,
            "target-route",
            0,
            progress(12.0),
            Speed::ZERO,
        ));
    }
    let mut core = CoreWorld::with_traffic_data(20, traffic, vehicles).expect("valid world");
    if !with_blocker {
        let temporary = core
            .spawn_vehicle(VehicleSpawnInput::active(
                "temporary",
                profile,
                "target-route",
                0,
                progress(50.0),
                Speed::ZERO,
            ))
            .expect("warm spawn capacity");
        core.despawn_vehicle(temporary).expect("warm despawn");
    }
    (core, profile)
}

fn prepared_world(with_blocker: bool, bound: bool) -> (World, VehicleReplaceInput) {
    let (core, profile) = allocation_world(with_blocker);
    let old = core.vehicle_handle("old").expect("old handle");
    let input = VehicleReplaceInput::new(
        VehicleReplaceExternalId::Preserve,
        profile,
        core.route_handle("target-route").expect("target route"),
        0,
        progress(10.0),
        Speed::ZERO,
    );
    let spatial = SpatialRegistry::try_new(
        &LaneGraph::empty(),
        CanonicalFrameId::try_new("test:lifecycle-allocation").expect("frame"),
        [],
    )
    .expect("empty spatial");
    let config = LaneFlowSessionConfig::new(NonZeroU32::new(1).expect("non-zero"));
    let mut session = LaneFlowSession::with_pose_capacity(core, spatial, config, 1);
    let mut world = World::new();
    if bound {
        let entity = world.spawn_empty().id();
        session
            .bind_vehicle_entity(old, entity)
            .expect("bind old vehicle");
    }
    world.insert_resource(session);
    world.resource_scope(|_world, _session: bevy_ecs::world::Mut<'_, LaneFlowSession>| {});
    (world, input)
}

fn measure<T>(operation: impl FnOnce() -> T) -> (T, Stats) {
    let region = Region::new(GLOBAL);
    let output = operation();
    black_box(&output);
    let stats = black_box(region.change());
    (output, stats)
}

fn assert_zero_allocation(label: &str, stats: Stats) {
    assert_eq!(stats.allocations, 0, "{label}: allocations");
    assert_eq!(stats.reallocations, 0, "{label}: reallocations");
    assert_eq!(stats.bytes_allocated, 0, "{label}: allocated bytes");
    assert_eq!(stats.bytes_reallocated, 0, "{label}: reallocated bytes");
}

#[test]
fn stable_capacity_bound_success_and_blocked_retry_are_zero_allocation() {
    let (mut success_world, input) = prepared_world(false, true);
    let old = success_world
        .resource::<LaneFlowSession>()
        .core()
        .vehicle_handle("old")
        .expect("old handle");
    let (outcome, stats) = measure(|| {
        replace_completed_vehicle(&mut success_world, old, &input).expect("replacement")
    });
    assert!(matches!(
        outcome,
        LaneFlowVehicleReplaceOutcome::Replaced(_)
    ));
    assert_zero_allocation("bound Preserve success", stats);

    let (mut unbound_world, input) = prepared_world(false, false);
    let old = unbound_world
        .resource::<LaneFlowSession>()
        .core()
        .vehicle_handle("old")
        .expect("old handle");
    let (outcome, stats) = measure(|| {
        replace_completed_vehicle(&mut unbound_world, old, &input).expect("replacement")
    });
    assert!(matches!(
        outcome,
        LaneFlowVehicleReplaceOutcome::Replaced(record) if record.entity.is_none()
    ));
    assert_zero_allocation("unbound Preserve success", stats);

    let (mut blocked_world, input) = prepared_world(true, true);
    let old = blocked_world
        .resource::<LaneFlowSession>()
        .core()
        .vehicle_handle("old")
        .expect("old handle");
    let warm = replace_completed_vehicle(&mut blocked_world, old, &input).expect("warm block");
    assert!(matches!(warm, LaneFlowVehicleReplaceOutcome::Blocked(_)));
    let (outcome, stats) = measure(|| {
        replace_completed_vehicle(&mut blocked_world, old, &input).expect("measured block")
    });
    assert!(matches!(outcome, LaneFlowVehicleReplaceOutcome::Blocked(_)));
    assert_zero_allocation("bound Blocked retry", stats);
}
