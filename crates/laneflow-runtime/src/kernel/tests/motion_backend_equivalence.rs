//! #814 验收：同一布局下 scalar/AVX2/AVX512 三个数值后端与多个 worker 数整世界逐拍
//! 一致。逐拍比较 `StepOutcome`、已提交状态摘要、Conflict/Waiting 决策与转移事件；
//! 首错注入后同拍重试、快照恢复后续跑都必须与 scalar 单 worker 的 fresh 参考一致。
//! 本机不支持的后端跳过并在输出中注明，scalar 永远参与。

use std::num::NonZeroU32;
use std::sync::Arc;

use laneflow_motion_kernel::{Backend, Kernel};

use crate::admin::cutover_migration::tests::{
    conflict_scale_revision, conflict_scale_world_with_identity,
};
use crate::kernel::execution::{RESOURCE_TEST_LOCK, WorldExecution};
use crate::kernel::waiting::tests::multi_gate_world_with_id;
use crate::{
    ExecutionConfig, SnapshotRestoreLimits, StepError, TickInput, TrafficWorld,
    deterministic_state_digest, encode_lfrs, restore_lfrs,
};

const BACKENDS: [Backend; 3] = [Backend::Scalar, Backend::Avx2, Backend::Avx512];
const WORKERS: [u32; 3] = [1, 4, 16];
const FAIL_TICK: usize = 5;
const RESTORE_TICK: usize = 17;
/// 故障注入按世界身份隔离（#792），取 `conflict_tick` 常量表之外的独占身份。
const WAITING_WORLD: u64 = 708_020;
const CONFLICT_WORLD: u64 = 708_021;

struct Scenario<'a> {
    name: &'a str,
    build: &'a dyn Fn() -> TrafficWorld,
    delta_ms: u64,
    ticks: usize,
    /// 注入非有限运动的规范 Active 序号。
    fail_row: usize,
}

fn kernels() -> Vec<(Backend, Kernel)> {
    let kernels: Vec<_> = BACKENDS
        .into_iter()
        .filter_map(|backend| Kernel::for_backend(backend).map(|kernel| (backend, kernel)))
        .collect();
    let skipped: Vec<_> = BACKENDS
        .into_iter()
        .filter(|backend| !kernels.iter().any(|(available, _)| available == backend))
        .collect();
    if !skipped.is_empty() {
        eprintln!("motion backends unsupported on this host, skipped: {skipped:?}");
    }
    kernels
}

fn configure(world: &mut TrafficWorld, kernel: Kernel, workers: u32) {
    world.state.workspace.motion_kernel = kernel;
    world.execution = WorldExecution::start_private(
        ExecutionConfig::new(NonZeroU32::new(workers).unwrap()),
        &world.state,
    );
}

fn record(world: &TrafficWorld, outcome: &Result<crate::StepOutcome, StepError>) -> String {
    let snapshot = world.capture_snapshot().unwrap();
    format!(
        "outcome={outcome:?}\ndigest={:?}\nconflict={:?}\nwaiting={:?}\nevents={:?}",
        deterministic_state_digest(&snapshot).unwrap(),
        world.latest_conflict_decisions(),
        world.latest_waiting_decisions(),
        world.latest_transition_events(),
    )
}

fn restore(world: &TrafficWorld, kernel: Kernel, workers: u32) -> TrafficWorld {
    let snapshot = world.capture_snapshot().unwrap();
    let mut restored = restore_lfrs(
        &encode_lfrs(&snapshot),
        world.revision(),
        world.committed_source().clone(),
        world.config(),
        ExecutionConfig::new(NonZeroU32::MIN),
        SnapshotRestoreLimits::new(64 * 1_024 * 1_024, 4 * 1_024),
    )
    .unwrap()
    .into_world();
    configure(&mut restored, kernel, workers);
    restored
}

/// 参考：scalar、单 worker、无注入、不恢复。
fn reference(scenario: &Scenario<'_>) -> Vec<String> {
    let mut world = (scenario.build)();
    configure(&mut world, Kernel::for_backend(Backend::Scalar).unwrap(), 1);
    (0..scenario.ticks)
        .map(|_| {
            let outcome = world.step(TickInput::new(scenario.delta_ms));
            record(&world, &outcome)
        })
        .collect()
}

/// 被测：指定后端与 worker 数；`FAIL_TICK` 先注入非有限运动首错并同拍重试，
/// `RESTORE_TICK` 前经快照恢复换成新世界续跑。
fn candidate(scenario: &Scenario<'_>, kernel: Kernel, workers: u32) -> Vec<String> {
    let tick_input = TickInput::new(scenario.delta_ms);
    let mut world = (scenario.build)();
    configure(&mut world, kernel, workers);
    let mut records = Vec::with_capacity(scenario.ticks);
    for tick in 0..scenario.ticks {
        if tick == FAIL_TICK {
            let before = world.capture_snapshot().unwrap();
            {
                let _failure = crate::kernel::tick::inject_motion_nonfinite(
                    world.state.binding.world_id,
                    &[scenario.fail_row],
                );
                assert_eq!(
                    world.step(tick_input),
                    Err(StepError::NonFiniteMotion),
                    "injected first error must surface (workers={workers})"
                );
            }
            assert_eq!(
                world.capture_snapshot().unwrap(),
                before,
                "failed tick must not publish (workers={workers})"
            );
        }
        if tick == RESTORE_TICK {
            world = restore(&world, kernel, workers);
        }
        let outcome = world.step(tick_input);
        records.push(record(&world, &outcome));
    }
    records
}

fn assert_matrix(scenario: &Scenario<'_>, exercised: &str) {
    let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
    let _motion = crate::kernel::tick::force_motion_dispatch();
    let _preview = crate::kernel::waiting::force_preview_dispatch();
    let name = scenario.name;
    let expected = reference(scenario);
    if let Some(failed) = expected
        .iter()
        .find(|record| !record.starts_with("outcome=Ok"))
    {
        panic!("{name}: reference run failed: {failed}");
    }
    assert!(
        expected.iter().any(|record| !record.contains(exercised)),
        "{name}: reference never produced a non-empty `{exercised}`"
    );
    for (backend, kernel) in kernels() {
        for workers in WORKERS {
            let actual = candidate(scenario, kernel, workers);
            for (tick, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
                assert_eq!(
                    actual, expected,
                    "{name} diverged: backend={backend:?} workers={workers} tick={tick}"
                );
            }
        }
    }
}

#[test]
fn waiting_world_matches_across_backends_workers_retry_and_restore() {
    // 300 行：两个满块加 44 行尾块；含 Waiting 决策与路线末端 Completed。
    let build = || multi_gate_world_with_id(300, WAITING_WORLD);
    assert!(build().live_vehicles().len() > 2 * crate::kernel::vehicle_store::BLOCK_ROWS);
    let scenario = Scenario {
        name: "waiting-multi-gate",
        build: &build,
        delta_ms: 100,
        ticks: 40,
        fail_row: 37,
    };
    assert_matrix(&scenario, "\nwaiting=[]\n");
}

#[test]
fn conflict_world_matches_across_backends_workers_retry_and_restore() {
    let revision = conflict_scale_revision();
    let build = || conflict_scale_world_with_identity(Arc::clone(&revision), 64, 1, CONFLICT_WORLD);
    let scenario = Scenario {
        name: "conflict-scale",
        build: &build,
        delta_ms: 4,
        ticks: 200,
        fail_row: 21,
    };
    assert_matrix(&scenario, "\nconflict=[]\n");
}
