//! #441 手动插桩墙钟校准；不与分配硬断言共用 libtest 二进制。
//!
//! 即使此测试 ignored，libtest 报告其结果也会分配，污染并行运行的进程级分配账本。
//! 保留同一 stats_alloc 包装、构建夹具与计时范围，只将手动入口隔离到独立进程。
//! 手动运行：cargo test --release --locked -p laneflow-runtime
//! --test shared_network_instrumented_wall_clock_evidence -- --ignored --nocapture

use std::alloc::System;
use std::hint::black_box;
use std::time::Instant;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const CORRIDOR: &[u8] = include_bytes!("../../../examples/data/v0.2-signalized-corridor.lfca");
const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024);

#[test]
#[ignore = "manual instrumented vs uninstrumented wall-clock delta"]
fn instrumented_build_wall_clock_for_calibration() {
    let started = Instant::now();
    let input = check_canonical_network_input(CORRIDOR, FormatLimits::HARD).expect("checked");
    let held = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
    )
    .expect("build");
    let elapsed_ns = started.elapsed().as_nanos();
    black_box(&held);
    println!(
        "shared-static-network-evidence calibrate instrumented_corridor_build_ns={elapsed_ns}"
    );
}
