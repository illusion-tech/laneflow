//! 单测试进程，避免其他 libtest 用例的分配进入全局计数窗口。

#[path = "support/policy.rs"]
mod test_policy;

use std::{alloc::System, sync::Arc};

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, PublishedLfcaReference, RouteRegisterInput, SpawnError, TickInput,
    TrafficWorld, VehicleSpawnInput, WorldConfig,
};
use laneflow_static_contract::{LaneEdgeOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

#[test]
fn warm_overlap_queries_do_not_allocate_routes_or_intervals() {
    let checked = check_canonical_network_input(
        include_bytes!(
            "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
        ),
        FormatLimits::HARD,
    )
    .unwrap();
    let revision = build_shared_network_revision(
        checked,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .unwrap();
    let origin = *revision.canonical_origin();
    let source = CommittedNetworkSource::Published {
        reference: PublishedLfcaReference::new(
            "fixture://overlap-allocation",
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .unwrap(),
    };
    let mut world = TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
        source,
        528,
        test_policy::selection(&revision),
    )
    .unwrap();
    let first = LaneEdgeOrdinal::from_raw(0);
    let mut edges = vec![first];
    if let Some(second) = world.traffic().successors(first).unwrap_or(&[]).first() {
        edges.push(*second);
    }
    let route = world
        .register_route(RouteRegisterInput::new(edges))
        .unwrap();
    let profile = VehicleProfileOrdinal::from_raw(0);
    let blocker = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, route, 0, 1_000, 0))
        .unwrap();
    world.step(TickInput::new(100)).unwrap();
    let state = world.vehicle(blocker).unwrap();
    let query = VehicleSpawnInput::new(
        profile,
        route,
        state.route_edge_index(),
        state.progress_mm(),
        0,
    );
    assert_eq!(world.spawn_vehicle(query), Err(SpawnError::Overlap));

    let region = Region::new(GLOBAL);
    for _ in 0..256 {
        assert_eq!(world.spawn_vehicle(query), Err(SpawnError::Overlap));
    }
    let stats = region.change();
    assert_eq!(stats.allocations, 0, "warm overlap queries allocated");
    assert_eq!(stats.reallocations, 0, "warm overlap queries reallocated");
}
