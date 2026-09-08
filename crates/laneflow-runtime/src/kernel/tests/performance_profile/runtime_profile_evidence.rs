//! #583 非插桩 release 整步墙钟。固定研究窗口，不是产品认证。

use laneflow_runtime as runtime_types;
#[path = "runtime_profile.rs"]
mod support;

#[path = "runtime_cpu_sampling.rs"]
mod cpu_sampling;

use std::time::Instant;
use support::{CASES, Fixtures, Scene};

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
