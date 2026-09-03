//! #282 WaitingZone 稳态 heap allocation 证据。
//!
//! 单一默认测试承载全局计数分配器。车辆已成功进入 WaitingZone、但尚未触达
//! release Gate 后，再测量连续固定步进；该窗口覆盖 membership、traversal phase、
//! occupancy 和本地存储约束的 steady path，硬断言 allocation / reallocation 均为零。

#[path = "support/policy.rs"]
mod test_policy;

use std::alloc::System;
use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, PublishedLfcaReference, RouteRegisterInput, TickInput, TrafficWorld,
    VehicleSpawnInput, WorldConfig,
};
use laneflow_static_contract::{ManeuverPathOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
);
const DELTA_MS: u64 = 4;
const STEADY_TICKS: u32 = 16;

#[test]
fn waiting_steady_tick_has_zero_heap_allocation_after_warmup() {
    let input =
        check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).expect("checked fixture");
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("revision");
    let origin = *revision.canonical_origin();
    let mut world = TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(8, 4, 1_024, 1_024, 1, DELTA_MS),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://waiting-budget",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("source"),
        },
        282,
        test_policy::selection(&revision),
    )
    .expect("world");
    let edges = world
        .traffic()
        .maneuvers()
        .maneuver_path(ManeuverPathOrdinal::from_raw(0))
        .expect("main path")
        .edges()
        .to_vec();
    let entry_length_mm = world.traffic().lane_lengths_millimetres()[edges[0].index()];
    let route = world
        .register_route(RouteRegisterInput::new(edges))
        .expect("route");
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            entry_length_mm - 1,
            8_000,
        ))
        .expect("vehicle");

    world.step(TickInput::new(DELTA_MS)).expect("admission");
    assert!(
        world
            .vehicle(vehicle)
            .and_then(|state| state.waiting_membership())
            .is_some(),
        "fixture vehicle must hold a Waiting membership before measurement"
    );

    let stats = {
        let region = Region::new(GLOBAL);
        for _ in 0..STEADY_TICKS {
            world.step(TickInput::new(DELTA_MS)).expect("steady step");
        }
        region.change()
    };
    assert_eq!(stats.allocations, 0, "steady Waiting ticks allocated");
    assert_eq!(stats.reallocations, 0, "steady Waiting ticks reallocated");
    assert_eq!(
        stats.bytes_allocated, 0,
        "steady Waiting ticks allocated bytes"
    );
    assert!(
        world
            .vehicle(vehicle)
            .and_then(|state| state.waiting_membership())
            .is_some(),
        "measurement window must remain in the Waiting steady path"
    );
    println!(
        "waiting-g2-allocation-evidence steady_ticks={STEADY_TICKS} allocations={} \
         reallocations={} allocated_bytes={}",
        stats.allocations, stats.reallocations, stats.bytes_allocated
    );
}
