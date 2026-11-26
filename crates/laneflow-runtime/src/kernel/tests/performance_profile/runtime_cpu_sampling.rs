//! #216 外部 CPU 采样入口：重复固定观测窗口，链接没有测试探针的生产库。

use std::time::Instant;

use super::support::{Fixtures, Harness};
use super::{ROAD_1K_256, ROAD_10K_16, ROAD_10K_256, RoadWindowBaseline};

// 只固定最外层研究窗口的栈锚点，不限制任何生产函数的优化或内联。
// CPU 报告筛选这棵调用子树，排除建世界、暖机、快照和校验。
#[inline(never)]
fn observed_cpu_window(harness: &mut Harness) {
    for _ in 0..harness.steps {
        harness.step();
    }
}

fn run_windows(baseline: RoadWindowBaseline, rounds: usize) {
    let RoadWindowBaseline {
        count,
        expected_digest,
        ..
    } = baseline;
    let scene = baseline.scene(false);
    let fixtures = Fixtures::new();
    let mut elapsed_ns = 0_u128;
    let mut observed_steps = 0_usize;
    for _ in 0..rounds {
        let mut harness = fixtures.world(scene);
        harness.validate();
        let started = Instant::now();
        observed_cpu_window(&mut harness);
        elapsed_ns += started.elapsed().as_nanos();
        observed_steps += harness.steps;
        harness.validate();
        assert_eq!(harness.digest(), expected_digest);
    }
    println!(
        "cpu-sampling-window scene={} rounds={rounds} steps={observed_steps} delta_ms=100 individual={count} active={count} intent={count} presented=0 aggregate=0 observed_wall_ns={elapsed_ns} digest={expected_digest}",
        scene.name(),
    );
}

#[test]
#[ignore = "证据测量窗口，夜间 evidence.yml 运行（#748）"]
fn cpu_sampling_windows_match_frozen_digests() {
    for baseline in [ROAD_1K_256, ROAD_10K_256, ROAD_10K_16] {
        run_windows(baseline, 1);
    }
}

#[test]
#[ignore = "manual external CPU sampling; repeat the fixed 1k/256-edge window"]
fn runtime_cpu_sampling_1k_256() {
    run_windows(ROAD_1K_256, 1_024);
}

#[test]
#[ignore = "manual external CPU sampling; repeat the fixed 10k/256-edge window"]
fn runtime_cpu_sampling_10k_256() {
    run_windows(ROAD_10K_256, 128);
}

#[test]
#[ignore = "manual external CPU sampling; repeat the fixed 10k/16-edge window"]
fn runtime_cpu_sampling_10k_16() {
    run_windows(ROAD_10K_16, 128);
}
