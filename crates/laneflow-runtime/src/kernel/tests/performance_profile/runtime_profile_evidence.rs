//! #583 非插桩 release 整步墙钟。固定研究窗口，不是产品认证。

use laneflow_runtime as runtime_types;
#[path = "runtime_profile.rs"]
mod support;

#[path = "runtime_cpu_sampling.rs"]
mod cpu_sampling;

#[path = "runtime_journal_ab.rs"]
mod journal_ab;

use std::time::Instant;
use support::{CASES, Fixtures, Scene};

#[derive(Clone, Copy)]
struct RoadWindowBaseline {
    count: u32,
    edges: u32,
    expected_digest: &'static str,
}

impl RoadWindowBaseline {
    fn scene(self, journal: bool) -> Scene {
        Scene::Lane {
            count: self.count,
            edges: self.edges,
            journal,
        }
    }
}

// 摘要 9 绑定 LFCA 8。四个普通窗口用 a94acf02 的运动内核独立推进，
// 按 8/6/9 编入空路线窗口、车辆横移记录和空驾驶配置后核对。
// Journal 开关共用同一状态摘要基线（#1557）。
const ROAD_1K_256: RoadWindowBaseline = RoadWindowBaseline {
    count: 1_000,
    edges: 256,
    expected_digest: "c2a464eb3a2d6524f25c2471c7af40a92ea61257dfda7fae6061d9a0bb39c39d",
};
const ROAD_10K_256: RoadWindowBaseline = RoadWindowBaseline {
    count: 10_000,
    edges: 256,
    expected_digest: "e6f2e62bf17520d5f4d14286ffe4248c126d30f1e0d9b9c35a820d5af9d25d57",
};
const ROAD_10K_16: RoadWindowBaseline = RoadWindowBaseline {
    count: 10_000,
    edges: 16,
    expected_digest: "ed2b6bad629c949363eefd954950306a84e7dbbf7fc405e3f6d201e740b43730",
};

#[test]
fn road_1k_256_window_matches_frozen_digest() {
    let scene = ROAD_1K_256.scene(false);
    let fixtures = Fixtures::new();
    let mut harness = fixtures.world(scene);
    harness.validate();
    for _ in 0..harness.steps {
        harness.step();
    }
    harness.validate();
    assert_eq!(
        harness.digest(),
        ROAD_1K_256.expected_digest,
        "frozen state digest mismatch for {}",
        scene.name(),
    );
}

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
        "d705252b005aba2101b037151e282fad5b2234c8fec968c42beea8304dd75ed0"
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
