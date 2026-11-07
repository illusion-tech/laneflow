//! #583 非插桩 release 整步墙钟。固定研究窗口，不是产品认证。

use laneflow_runtime as runtime_types;
#[path = "runtime_profile.rs"]
mod support;

#[path = "runtime_cpu_sampling.rs"]
mod cpu_sampling;

#[path = "runtime_journal_ab.rs"]
mod journal_ab;

use std::time::Instant;
#[cfg(feature = "placement-fixtures")]
use support::Scene;
use support::{CASES, Fixtures};

#[cfg(feature = "placement-fixtures")]
#[test]
fn profile_resource_windows_preserve_active_resource_states() {
    let fixtures = Fixtures::new();
    for scene in [Scene::Waiting, Scene::Conflict] {
        let mut harness = fixtures.world(scene);
        harness.validate();
        for _ in 0..harness.steps {
            harness.step();
        }
        harness.validate();
        assert!(!harness.digest().is_empty());
    }
}

/// 不依赖负载的摘要正确性回归：小规模固定窗口的摘要必须与冻结基线一致。
/// 摘要算法或版本迁移时此断言失败，提醒同步更新所有冻结基线（#1301）。
#[cfg(feature = "placement-fixtures")]
#[test]
fn state_digest_matches_frozen_baseline() {
    let fixtures = Fixtures::new();
    let scene = Scene::Lane {
        count: 1,
        edges: 1,
        journal: false,
    };
    let mut harness = fixtures.world(scene);
    harness.validate();
    for _ in 0..harness.steps {
        harness.step();
    }
    harness.validate();
    assert_eq!(
        harness.digest(),
        "9ac1adbc1a1743cde5f8034a026adf77f8589f09dd9ccb75ceb2958760baf7de"
    );
}

#[test]
#[ignore = "manual release current Runtime profile, without diagnostic instrumentation"]
fn runtime_profile_wall_clock() {
    let fixtures = Fixtures::new();
    for scene in CASES {
        for round in 0..3 {
            let mut harness = fixtures.world(scene);
            harness.validate();
            let mut samples = vec![0; harness.steps];
            for sample in &mut samples {
                let started = Instant::now();
                harness.step();
                *sample = started.elapsed().as_nanos();
            }
            harness.validate();
            samples.sort_unstable();
            let percentile = |percent: usize| samples[(samples.len() * percent).div_ceil(100) - 1];
            println!(
                "profile-tick scene={} round={round} steps={} delta_ms={} individual={} active={} intent={} presented=0 aggregate=0 p50_ns={} p95_ns={} p99_ns={} max_ns={} sum_ns={} journal_bytes={} digest={}",
                scene.name(),
                harness.steps,
                harness.delta_ms,
                scene.count(),
                scene.count(),
                scene.count(),
                percentile(50),
                percentile(95),
                percentile(99),
                samples.last().unwrap(),
                samples.iter().sum::<u128>(),
                harness.journal_bytes(),
                harness.digest(),
            );
        }
    }
}
