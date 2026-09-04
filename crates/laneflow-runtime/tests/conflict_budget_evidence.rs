//! #569 Conflict 正式固定步进稳态 heap allocation 证据。
//!
//! 两条冲突流从真实 LFCA 完成 install -> register -> spawn -> tick；首次仲裁后，
//! 在一个车辆持有冲突区预约、另一个车辆继续重试的窗口测量固定步进。

#[path = "support/policy.rs"]
mod test_policy;

use std::alloc::System;
use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, PublishedLfcaReference, RouteRegisterInput, TickInput, TrafficWorld,
    VehicleSpawnInput, WorldConfig,
};
use laneflow_static_contract::{ParticipantStreamOrdinal, VehicleProfileOrdinal};
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
const WARM_TICKS: u32 = 8;
const STEADY_TICKS: u32 = 16;

#[test]
fn conflict_steady_tick_has_zero_heap_allocation_after_warmup() {
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
        WorldConfig::new(4, 4, 64, 2, 1, DELTA_MS),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://conflict-budget",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("source"),
        },
        569,
        test_policy::selection(&revision),
    )
    .expect("world");

    let routes = [0_u32, 1].map(|raw| {
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
            .expect("participant stream");
        let edges = revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .expect("maneuver path")
            .edges()
            .to_vec();
        world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route")
    });
    let vehicles = routes.map(|route| {
        let entry = world.route_edges(route).expect("route edges")[0];
        let gate_progress = world.traffic().lane_lengths_millimetres()[entry.index()];
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                gate_progress,
                8_000,
            ))
            .expect("Gate-boundary vehicle")
    });

    world
        .step(TickInput::new(DELTA_MS))
        .expect("warm arbitration");
    assert!(
        vehicles
            .iter()
            .any(|vehicle| world.conflict_reservation(*vehicle).is_some()),
        "warm tick must exercise the formal Conflict reservation path"
    );
    for _ in 0..WARM_TICKS {
        world
            .step(TickInput::new(DELTA_MS))
            .expect("warm steady step");
    }

    let stats = {
        let region = Region::new(GLOBAL);
        for _ in 0..STEADY_TICKS {
            world.step(TickInput::new(DELTA_MS)).expect("steady step");
        }
        region.change()
    };
    assert_eq!(stats.allocations, 0, "steady Conflict ticks allocated");
    assert_eq!(stats.reallocations, 0, "steady Conflict ticks reallocated");
    assert_eq!(
        stats.bytes_allocated, 0,
        "steady Conflict ticks allocated bytes"
    );
    assert!(
        vehicles
            .iter()
            .any(|vehicle| world.conflict_reservation(*vehicle).is_some()),
        "measurement window must remain in the Conflict steady path"
    );
    println!(
        "conflict-g2-allocation-evidence steady_ticks={STEADY_TICKS} allocations={} \
         reallocations={} allocated_bytes={}",
        stats.allocations, stats.reallocations, stats.bytes_allocated
    );
}
