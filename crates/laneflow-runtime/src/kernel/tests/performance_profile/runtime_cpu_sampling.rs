//! #216 外部 CPU 采样入口：重复固定观测窗口，链接没有测试探针的生产库。

use std::time::Instant;

use super::support::{Fixtures, Harness, Scene};

const ROAD_CASES: [(u32, u32, &str); 3] = [
    (
        1_000,
        256,
        "31e4896e0811cd527a52b8d7820526ccac2e9cdb70b426da1f1b1814c7a46e1b",
    ),
    (
        10_000,
        256,
        "81ebf8b06a078a3eae8859958353fcd8f57c7e93cdcb224785358902fe2cdfcb",
    ),
    (
        10_000,
        16,
        "dc65c7b614b506ad5c2ae7ac22391127b055c9b8f1e74426f65db59325ddc5aa",
    ),
];

// 只固定最外层研究窗口的栈锚点，不限制任何生产函数的优化或内联。
// CPU 报告筛选这棵调用子树，排除建世界、暖机、快照和校验。
#[inline(never)]
fn observed_cpu_window(harness: &mut Harness) {
    for _ in 0..harness.steps {
        harness.step();
    }
}

fn run_windows(case_index: usize, rounds: usize) {
    let (count, edges, expected_digest) = ROAD_CASES[case_index];
    let scene = Scene::Lane {
        count,
        edges,
        journal: false,
    };
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
fn cpu_sampling_windows_match_frozen_digests() {
    for case_index in 0..ROAD_CASES.len() {
        run_windows(case_index, 1);
    }
}

#[test]
#[ignore = "manual external CPU sampling; repeat the fixed 1k/256-edge window"]
fn runtime_cpu_sampling_1k_256() {
    run_windows(0, 1_024);
}

#[test]
#[ignore = "manual external CPU sampling; repeat the fixed 10k/256-edge window"]
fn runtime_cpu_sampling_10k_256() {
    run_windows(1, 128);
}

#[test]
#[ignore = "manual external CPU sampling; repeat the fixed 10k/16-edge window"]
fn runtime_cpu_sampling_10k_16() {
    run_windows(2, 128);
}
