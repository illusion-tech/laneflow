//! #441 分配账本。独立 integration test，避免污染 uninstrumented 墙钟。

use std::alloc::System;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
use laneflow_runtime::{TrafficWorld, WorldConfig};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const MIN_HEADLESS: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-variants/min-headless.lfca"
);
const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-full-spatial/expected.lfca"
);
const CORRIDOR: &[u8] = include_bytes!("../../../examples/data/v0.2-signalized-corridor.lfca");

const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024);

fn build(
    bytes: &[u8],
    spatial: SpatialBuildOption,
) -> Arc<laneflow_static_network::SharedNetworkRevision> {
    let input = check_canonical_network_input_v1(bytes, FormatLimits::V1_HARD).expect("checked");
    build_shared_network_revision(input, SharedNetworkBuildOptions::new(spatial, BUILD_LIMITS))
        .expect("build")
}

fn measure_build(scene: &str, bytes: &[u8], spatial: SpatialBuildOption) {
    let region = Region::new(GLOBAL);
    let started = Instant::now();
    let revision = build(bytes, spatial);
    black_box(&revision);
    let elapsed_ns = started.elapsed().as_nanos();
    let stats = region.change();
    println!(
        "shared-static-network-evidence allocation scene={scene} spatial={spatial:?} elapsed_ns={elapsed_ns} allocations={} reallocations={} allocated_bytes={} deallocated_bytes={} retained={} lfca_exact={}",
        stats.allocations,
        stats.reallocations,
        stats.bytes_allocated,
        stats.bytes_deallocated,
        revision.retained_logical_bytes(),
        bytes.len(),
    );
}

fn measure_worlds(bytes: &[u8]) {
    let revision = build(bytes, SpatialBuildOption::RetainAvailable);
    for count in [2_usize, 8, 32] {
        let region = Region::new(GLOBAL);
        let worlds: Vec<_> = (0..count)
            .map(|_| {
                TrafficWorld::install(Arc::clone(&revision), WorldConfig::new(8, 4, 1, 16))
                    .expect("install")
            })
            .collect();
        black_box(&worlds);
        let stats = region.change();
        let per_world = stats.bytes_allocated / count;
        println!(
            "shared-static-network-evidence allocation worlds count={count} allocations={} reallocations={} allocated_bytes={} approx_per_world_allocated={per_world} static_retained={}",
            stats.allocations,
            stats.reallocations,
            stats.bytes_allocated,
            revision.retained_logical_bytes(),
        );
        assert!(
            worlds
                .iter()
                .all(|world| Arc::ptr_eq(&world.revision(), &revision))
        );
    }
}

#[test]
fn build_allocation_ledgers_for_frozen_scenes() {
    measure_build(
        "min-headless",
        MIN_HEADLESS,
        SpatialBuildOption::RetainAvailable,
    );
    measure_build("full-spatial-omit", FULL_SPATIAL, SpatialBuildOption::Omit);
    measure_build(
        "full-lane-spatial",
        FULL_SPATIAL,
        SpatialBuildOption::RetainAvailable,
    );
    measure_build("corridor", CORRIDOR, SpatialBuildOption::RetainAvailable);
}

#[test]
fn multi_world_install_does_not_copy_static_payload() {
    measure_worlds(CORRIDOR);
}

#[test]
#[ignore = "manual instrumented vs uninstrumented wall-clock delta"]
fn instrumented_build_wall_clock_for_calibration() {
    let region = Region::new(GLOBAL);
    let started = Instant::now();
    black_box(build(CORRIDOR, SpatialBuildOption::RetainAvailable));
    let _ = region.change();
    println!(
        "shared-static-network-evidence calibrate instrumented_corridor_build_ns={}",
        started.elapsed().as_nanos()
    );
}
