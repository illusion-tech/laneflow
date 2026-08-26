//! #441 分配账本。独立 integration test，避免污染 uninstrumented 墙钟。
//!
//! 本文件只有一个默认测试，避免 `stats_alloc::Region` 在并行测试之间串账。
//! 每条账本采样先丢一次 warmup；`retained` 必须稳定，`stats_alloc` 计数只作描述性输出。
//! `allocated_bytes` 采用 `stats_alloc` 0.1.10 净值：普通 alloc 记全量，变大 realloc
//! 只把 `new_size - old_size` 计入 `bytes_allocated`。`live_bytes` 已含该净增长；
//! `reallocated_delta_bytes` 是有符号 realloc 净值，不得再加进 `live_bytes`。

use std::alloc::System;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use laneflow_corridor_generator::{CorridorConfig, generate};
use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{TrafficWorld, WorldConfig};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const MIN_HEADLESS: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-variants/min-headless.lfca"
);
const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
);
const CORRIDOR: &[u8] = include_bytes!("../../../examples/data/v0.2-signalized-corridor.lfca");
const CORRIDOR_CONFIG: &str =
    include_str!("../../../examples/config/v0.10-signalized-corridor.toml");

const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AllocSample {
    allocations: usize,
    reallocations: usize,
    allocated_bytes: usize,
    deallocated_bytes: usize,
    reallocated_delta_bytes: isize,
    live_bytes: usize,
    retained: u64,
}

fn build(bytes: &[u8], spatial: SpatialBuildOption) -> Arc<SharedNetworkRevision> {
    let input = check_canonical_network_input(bytes, FormatLimits::HARD).expect("checked");
    build_shared_network_revision(input, SharedNetworkBuildOptions::new(spatial, BUILD_LIMITS))
        .expect("build")
}

fn sample_from_stats(stats: stats_alloc::Stats, retained: u64) -> AllocSample {
    AllocSample {
        allocations: stats.allocations,
        reallocations: stats.reallocations,
        allocated_bytes: stats.bytes_allocated,
        deallocated_bytes: stats.bytes_deallocated,
        reallocated_delta_bytes: stats.bytes_reallocated,
        live_bytes: stats
            .bytes_allocated
            .saturating_sub(stats.bytes_deallocated),
        retained,
    }
}

fn sample_build(bytes: &[u8], spatial: SpatialBuildOption) -> AllocSample {
    let region = Region::new(GLOBAL);
    let revision = build(bytes, spatial);
    black_box(&revision);
    sample_from_stats(region.change(), revision.retained_logical_bytes())
}

fn warmup_build(bytes: &[u8], spatial: SpatialBuildOption) {
    black_box(sample_build(bytes, spatial));
}

fn assert_stable_build(
    scene: &str,
    bytes: &[u8],
    spatial: SpatialBuildOption,
    max_reallocations: usize,
) -> AllocSample {
    warmup_build(bytes, spatial);
    let first = sample_build(bytes, spatial);
    let second = sample_build(bytes, spatial);
    assert_eq!(
        first.retained, second.retained,
        "{scene} retained logical bytes must be deterministic"
    );
    assert!(
        second.reallocations <= max_reallocations,
        "{scene} reallocations {} exceed bound {max_reallocations}",
        second.reallocations
    );
    println!(
        "shared-static-network-evidence allocation scene={scene} spatial={spatial:?} allocations={} reallocations={} allocated_bytes={} deallocated_bytes={} reallocated_delta_bytes={} live_bytes={} retained={} lfca_exact={}",
        second.allocations,
        second.reallocations,
        second.allocated_bytes,
        second.deallocated_bytes,
        second.reallocated_delta_bytes,
        second.live_bytes,
        second.retained,
        bytes.len(),
    );
    second
}

fn sample_worlds(revision: &Arc<SharedNetworkRevision>, count: usize) -> AllocSample {
    let region = Region::new(GLOBAL);
    let mut worlds = Vec::with_capacity(count);
    for _ in 0..count {
        worlds.push(
            TrafficWorld::install(Arc::clone(revision), WorldConfig::new(8, 8, 1, 16))
                .expect("install"),
        );
    }
    black_box(&worlds);
    let stats = region.change();
    assert!(
        worlds
            .iter()
            .all(|world| Arc::ptr_eq(&world.revision(), revision))
    );
    sample_from_stats(stats, revision.retained_logical_bytes())
}

fn sample_held_candidate(
    current: &Arc<SharedNetworkRevision>,
    base: &[u8],
    target_lfca: &[u8],
    target_lfsm: &[u8],
    target_lfsd: &[u8],
    spatial: SpatialBuildOption,
) -> AllocSample {
    let region = Region::new(GLOBAL);
    let candidate = build(target_lfca, spatial);
    black_box((
        current,
        base,
        target_lfca,
        target_lfsm,
        target_lfsd,
        &candidate,
    ));
    sample_from_stats(region.change(), candidate.retained_logical_bytes())
}

#[test]
fn allocation_ledgers_and_per_world_live_bytes() {
    assert_stable_build(
        "min-headless",
        MIN_HEADLESS,
        SpatialBuildOption::RetainAvailable,
        0,
    );
    assert_stable_build(
        "full-spatial-omit",
        FULL_SPATIAL,
        SpatialBuildOption::Omit,
        64,
    );
    assert_stable_build(
        "full-lane-spatial",
        FULL_SPATIAL,
        SpatialBuildOption::RetainAvailable,
        64,
    );
    let corridor_build = assert_stable_build(
        "corridor",
        CORRIDOR,
        SpatialBuildOption::RetainAvailable,
        256,
    );

    let config = CorridorConfig::parse(CORRIDOR_CONFIG).expect("corridor config");
    let generated = generate(&config).expect("generate corridor");
    let (target_lfsm, target_lfsd) = generated.emit_portable_sidecars().expect("sidecars");
    let base = CORRIDOR.to_vec();
    let target_lfca = generated.lfca_bytes().to_vec();
    assert!(!target_lfsm.is_empty());
    assert!(!target_lfsd.is_empty());
    let current = build(&base, SpatialBuildOption::RetainAvailable);
    black_box(sample_held_candidate(
        &current,
        &base,
        &target_lfca,
        &target_lfsm,
        &target_lfsd,
        SpatialBuildOption::RetainAvailable,
    ));
    let first = sample_held_candidate(
        &current,
        &base,
        &target_lfca,
        &target_lfsm,
        &target_lfsd,
        SpatialBuildOption::RetainAvailable,
    );
    let second = sample_held_candidate(
        &current,
        &base,
        &target_lfca,
        &target_lfsm,
        &target_lfsd,
        SpatialBuildOption::RetainAvailable,
    );
    assert_eq!(
        first.retained, second.retained,
        "held coexistence candidate retained must be deterministic"
    );
    assert!(
        second.reallocations <= 256,
        "held candidate reallocations {} exceed bound 256",
        second.reallocations
    );
    assert!(second.live_bytes > 0);
    println!(
        "shared-static-network-evidence allocation coexistence-held current_retained={} candidate_live={} candidate_retained={} reallocations={} reallocated_delta_bytes={} target_lfca={} target_lfsm={} target_lfsd={}",
        current.retained_logical_bytes(),
        second.live_bytes,
        second.retained,
        second.reallocations,
        second.reallocated_delta_bytes,
        target_lfca.len(),
        target_lfsm.len(),
        target_lfsd.len(),
    );

    let revision = build(CORRIDOR, SpatialBuildOption::RetainAvailable);
    let static_retained = usize::try_from(revision.retained_logical_bytes()).expect("retained");
    let mut per_world = Vec::new();
    for count in [2_usize, 8, 32] {
        black_box(sample_worlds(&revision, count));
        let first = sample_worlds(&revision, count);
        let second = sample_worlds(&revision, count);
        assert_eq!(
            first.retained, second.retained,
            "{count} worlds retained must be deterministic"
        );
        assert!(
            second.reallocations <= 32,
            "{count} worlds reallocations {} exceed bound 32",
            second.reallocations
        );
        let live_per = second.live_bytes / count;
        assert!(
            live_per > 0,
            "{count} worlds must allocate per-world tables"
        );
        // 共享根退役 StaticRoute 后 retained 变小；占用索引与 compiled 路线表在每世界上。
        // 2/8 个空世界仍应低于一条走廊静态根；32 个允许不超过两倍。
        let live_budget = if count >= 32 {
            static_retained.saturating_mul(2)
        } else {
            static_retained
        };
        assert!(
            live_per * count < live_budget,
            "{count} worlds live {} must stay below budget {live_budget} (static retained {static_retained})",
            second.live_bytes
        );
        assert!(
            u64::try_from(live_per).expect("per-world") * 16 < corridor_build.retained,
            "per-world live {live_per} must not masquerade as static retained"
        );
        per_world.push(live_per);
        println!(
            "shared-static-network-evidence allocation worlds count={count} live_bytes={} live_per_world={live_per} reallocations={} reallocated_delta_bytes={} static_retained={}",
            second.live_bytes,
            second.reallocations,
            second.reallocated_delta_bytes,
            second.retained
        );
    }
    let min = *per_world.iter().min().expect("per-world");
    let max = *per_world.iter().max().expect("per-world");
    assert!(
        max <= min.saturating_mul(2) + 64,
        "per-world live bytes must stay in a tight band, got {per_world:?}"
    );
}

#[test]
#[ignore = "manual instrumented vs uninstrumented wall-clock delta"]
fn instrumented_build_wall_clock_for_calibration() {
    let started = Instant::now();
    let held = build(CORRIDOR, SpatialBuildOption::RetainAvailable);
    let elapsed_ns = started.elapsed().as_nanos();
    black_box(&held);
    println!(
        "shared-static-network-evidence calibrate instrumented_corridor_build_ns={elapsed_ns}"
    );
}
