//! Bounded spawn, replace, and steady-step account. Not a city run.

#[path = "support/policy.rs"]
#[cfg(feature = "placement-fixtures")]
mod test_policy;

#[cfg(feature = "placement-fixtures")]
use std::alloc::System;
#[cfg(feature = "placement-fixtures")]
use std::sync::Arc;
#[cfg(feature = "placement-fixtures")]
use std::time::Instant;

#[cfg(feature = "placement-fixtures")]
use laneflow_format::{FormatLimits, check_canonical_network_input};
#[cfg(feature = "placement-fixtures")]
use laneflow_runtime::{
    CommittedNetworkSource, ExecutionConfig, PublishedLfcaReference, RouteRegisterInput, TickInput,
    TrafficWorld, VehicleSpawnInput, VehicleStatus, WorldConfig,
};
#[cfg(feature = "placement-fixtures")]
use laneflow_static_contract::{ParticipantStreamOrdinal, VehicleProfileOrdinal};
#[cfg(feature = "placement-fixtures")]
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};
#[cfg(feature = "placement-fixtures")]
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
#[cfg(feature = "placement-fixtures")]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

#[cfg(feature = "placement-fixtures")]
const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
);
#[cfg(feature = "placement-fixtures")]
fn placement_bounded_account() {
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
        WorldConfig::new(32, 8, 256, 64, 100),
        ExecutionConfig::new(std::num::NonZeroU32::MIN),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://placement-account",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("source"),
        },
        742,
        test_policy::selection(&revision),
    )
    .expect("world");
    let stream = revision
        .conflict()
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("stream");
    let edges = revision
        .traffic()
        .maneuvers()
        .maneuver_path(stream.maneuver_path())
        .expect("path")
        .edges()
        .to_vec();
    let route = world
        .register_route(RouteRegisterInput::new(edges.clone()))
        .expect("route");
    let started = Instant::now();
    let region = Region::new(GLOBAL);
    let approach_length = world.traffic().lane_lengths_millimetres()[edges[0].index()];
    let mut progress = 0u32;
    while progress + 4_500 < approach_length && progress / 5_000 < 8 {
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, progress, 0)
                    .with_open_entrance(),
            )
            .expect("spawn");
        progress = progress.saturating_add(5_000);
    }
    let last = u32::try_from(edges.len() - 1).expect("index");
    let last_length = world.traffic().lane_lengths_millimetres()[edges[last as usize].index()];
    let completed = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                last,
                last_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("end spawn");
    world.step(TickInput::new(100)).expect("complete step");
    if world.vehicle(completed).map(|state| state.status()) == Some(VehicleStatus::Completed) {
        world
            .replace_completed_vehicle(
                completed,
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    last,
                    last_length,
                    0,
                )
                .with_open_entrance(),
            )
            .expect("replace");
    }
    for _ in 0..4 {
        world.step(TickInput::new(100)).expect("steady");
    }
    let stats = region.change();
    let elapsed_us = started.elapsed().as_micros();
    eprintln!(
        "ACCOUNT elapsed_us={elapsed_us} allocs={} retained_bytes={} vehicles={}",
        stats.allocations,
        stats
            .bytes_allocated
            .saturating_sub(stats.bytes_deallocated),
        world.live_vehicles().len()
    );
}

fn main() {
    let mut args = libtest_mimic::Arguments::from_args();
    // 全进程计数不能与框架调度并发；命令行指定更多线程也不能改变测量边界。
    args.test_threads = Some(1);
    #[cfg_attr(not(feature = "placement-fixtures"), allow(unused_variables))]
    let main_thread = std::thread::current().id();
    #[cfg(feature = "placement-fixtures")]
    let tests = vec![libtest_mimic::Trial::test(
        "placement_bounded_account",
        move || {
            assert_eq!(std::thread::current().id(), main_thread);
            placement_bounded_account();
            Ok(())
        },
    )];
    #[cfg(not(feature = "placement-fixtures"))]
    let tests: Vec<libtest_mimic::Trial> = Vec::new();
    libtest_mimic::run(&args, tests).exit();
}
