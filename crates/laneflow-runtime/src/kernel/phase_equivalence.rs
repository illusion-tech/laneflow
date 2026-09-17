//! 相同文件在 e011745e94986a17049c66e730d54ac9fccc59f9 与当前实现运行。
//! 只比较公开状态/批次及既有日志记录；不读取分区、容量或私有 grant serial。
//!
//! #705 整步等价：`trace_with_workers` 把同一 digest 轨迹在真实多 worker 世界
//! 下重跑（`WorldExecution::start_private` 重建世界独占执行资源，等同安装合同
//! 的计划/资源准备），逐场景断言 worker 2/4/8/16 与 worker 1 完全一致，并与
//! 固定 fixture `phase-parallel-matrix-c009d2dc.txt` 一致；fixture 值取自
//! worker 1 融合路径，其 digest 管线已由 pre-refactor fixture
//! `phase-protocol-e011745e.txt` 锚定。

use super::{STEP_FAILPOINT, StepFailpoint};
use crate::{StepError, TickInput, TrafficWorld};
use sha2::{Digest, Sha256};
use std::num::NonZeroU32;
use std::sync::Arc;

fn checkpoint(world: &TrafficWorld) -> String {
    let snapshot = world.capture_snapshot().unwrap();
    let vehicles: Vec<_> = world
        .live_vehicles()
        .iter()
        .map(|id| world.vehicle(*id))
        .collect();
    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        crate::deterministic_state_digest(&snapshot).unwrap(),
        world.live_vehicles(),
        vehicles,
        world.latest_waiting_decisions(),
        world.latest_conflict_decisions(),
        world.latest_transition_events(),
        world.committed_signal_groups(),
        world.waiting_zone_members(),
        world.migration_journal_stats(),
        (
            world.observation_state_sequence(),
            world.world_generation(),
            world.command_cursor(),
            world.event_cursor()
        )
    )
}

fn journal(world: &TrafficWorld) -> String {
    format!(
        "{:?}",
        world
            .state
            .migration_journal()
            .filter(|log| !log.overflowed())
            .map(|log| log.records_from(0).collect::<Vec<_>>())
    )
}

/// 场景车辆数不低于 P2 分发门槛（`WAITING_PREVIEW_FUSION_MIN_ACTIVE`），
/// 保证多 worker 运行真正走分发路径而非融合回退。
const DISPATCH_MIN_ACTIVE: usize = 8;

/// 用目标 worker 数重建世界独占执行资源；与 `TrafficWorld::install` 同一
/// `ExecutionPlan::prepare` + `ExecutionResources::start` 路径，供既有
/// 单 worker 场景构造 helper 在多 worker 下复用。
fn reinstall_execution(world: &mut TrafficWorld, workers: NonZeroU32) {
    world.execution = crate::kernel::execution::WorldExecution::start_private(
        crate::ExecutionConfig::new(workers),
        &world.state,
    );
}

fn trace(world: TrafficWorld, ticks: usize, retry: bool, journal_bound: Option<u64>) -> String {
    trace_with_workers(world, ticks, retry, journal_bound, NonZeroU32::MIN)
}

fn trace_with_workers(
    mut world: TrafficWorld,
    ticks: usize,
    retry: bool,
    journal_bound: Option<u64>,
    workers: NonZeroU32,
) -> String {
    if let Some(bound) = journal_bound {
        world.state.arm_migration_journal(bound).unwrap();
    }
    let mut digest = Sha256::new();
    digest.update(checkpoint(&world).as_bytes());
    let input = TickInput::new(world.config().fixed_delta_time_ms());
    let mut event_count = 0;
    for tick in 0..ticks {
        if retry && tick < 2 {
            let before = checkpoint(&world);
            let journal_before = journal(&world);
            let point = if tick == 0 {
                StepFailpoint::AfterGrants
            } else {
                StepFailpoint::AfterTransitions
            };
            STEP_FAILPOINT.with(|slot| slot.set(Some(point)));
            assert_eq!(
                world.step(input),
                Err(StepError::ParkingObservationAllocFailed)
            );
            assert_eq!(
                checkpoint(&world),
                before,
                "failed staging preserves all published state"
            );
            assert_eq!(
                journal(&world),
                journal_before,
                "failed staging never appends a journal frame"
            );
        }
        let outcome = world.step(input).unwrap();
        event_count += world.latest_transition_events().len();
        digest.update(
            format!(
                "{outcome:?}|{}
",
                checkpoint(&world)
            )
            .as_bytes(),
        );
    }
    digest.update(journal(&world).as_bytes());
    // 恢复不重建历史 latest batches；按快照局部身份比较后续语义。
    let captured = world.capture_snapshot().unwrap();
    let mut restored = crate::restore_lfrs(
        &crate::encode_lfrs(&captured),
        world.revision(),
        world.committed_source().clone(),
        world.config(),
        crate::ExecutionConfig::new(workers),
        crate::SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4_096),
    )
    .unwrap()
    .into_world();
    assert_eq!(
        crate::deterministic_state_digest(&restored.capture_snapshot().unwrap()).unwrap(),
        crate::deterministic_state_digest(&captured).unwrap()
    );
    world.step(input).unwrap();
    restored.step(input).unwrap();
    let continued = crate::deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap();
    assert_eq!(
        crate::deterministic_state_digest(&restored.capture_snapshot().unwrap()).unwrap(),
        continued
    );
    digest.update(format!("{continued:?}").as_bytes());
    eprintln!(
        "phase-trace-cost vehicles={} ticks={ticks} retry={retry} journal_bound={journal_bound:?} conflict_retained_bytes={}",
        world.live_vehicles().len(),
        world.state.conflict_retained_logical_bytes()
    );
    let hex: String = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{hex}:{event_count}")
}

fn signals_world(workers: NonZeroU32) -> TrafficWorld {
    let input = laneflow_format::check_canonical_network_input(
        include_bytes!(
            "../../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
        ),
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap();
    let revision = laneflow_static_network::build_shared_network_revision(
        input,
        laneflow_static_network::SharedNetworkBuildOptions::new(
            laneflow_static_network::SpatialBuildOption::Omit,
            laneflow_static_network::SharedNetworkBuildLimits::new(
                64 * 1_024 * 1_024,
                16 * 1_024 * 1_024,
            ),
        ),
    )
    .unwrap();
    let origin = *revision.canonical_origin();
    TrafficWorld::install(
        Arc::clone(&revision),
        crate::WorldConfig::new(4, 4, 1_024, 1_024, 100),
        crate::ExecutionConfig::new(workers),
        crate::CommittedNetworkSource::Published {
            reference: crate::PublishedLfcaReference::new(
                "fixture://phase-signals",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .unwrap(),
        },
        581,
        crate::test_policy::selection(&revision),
    )
    .unwrap()
}

#[test]
fn exact_baseline_trace_and_retry_match() {
    let revision = crate::admin::cutover_migration::tests::conflict_scale_revision();
    let mut actual = Vec::new();
    for (name, count, ticks, bound) in [
        ("waiting", 2, 160, None),
        ("waiting-journal", 2, 160, Some(1_024 * 1_024)),
        ("waiting-overflow", 2, 12, Some(32)),
    ] {
        let baseline = trace(
            crate::kernel::waiting::tests::multi_gate_world(count),
            ticks,
            false,
            bound,
        );
        assert_eq!(
            trace(
                crate::kernel::waiting::tests::multi_gate_world(count),
                ticks,
                true,
                bound
            ),
            baseline
        );
        actual.push(format!("{name}={baseline}"));
    }
    for (name, bound) in [
        ("conflict", None),
        ("conflict-journal", Some(1_024 * 1_024)),
    ] {
        let baseline = trace(
            crate::admin::cutover_migration::tests::conflict_scale_world(Arc::clone(&revision), 2),
            640,
            false,
            bound,
        );
        assert_eq!(
            trace(
                crate::admin::cutover_migration::tests::conflict_scale_world(
                    Arc::clone(&revision),
                    2
                ),
                640,
                true,
                bound
            ),
            baseline
        );
        actual.push(format!("{name}={baseline}"));
    }
    let signals = trace(
        signals_world(NonZeroU32::MIN),
        600,
        false,
        Some(1_024 * 1_024),
    );
    assert_eq!(
        trace(
            signals_world(NonZeroU32::MIN),
            600,
            true,
            Some(1_024 * 1_024)
        ),
        signals
    );
    actual.push(format!("signals-clock={signals}"));
    let actual = actual.join("\n");
    eprintln!("PHASE_BASELINE_BEGIN\n{actual}\nPHASE_BASELINE_END");
    // Filled from the fixed pre-refactor commit, never regenerated from the implementation under test.
    const EXPECTED: &str = include_str!("../../tests/fixtures/phase-protocol-e011745e.txt");
    assert_eq!(actual, EXPECTED.trim_end());
}

#[test]
fn parallel_worker_matrix_trace_matches_fixed_fixture() {
    let revision = crate::admin::cutover_migration::tests::conflict_scale_revision();
    // 车辆-bearing 场景的活动数必须达到分发门槛，多 worker 运行才真正分发；
    // 信号时钟场景无车辆，覆盖空工作集下的池世界整步。
    type Scenario = (
        &'static str,
        Box<dyn Fn(NonZeroU32) -> TrafficWorld>,
        usize,
        Option<u64>,
        bool,
    );
    let scenarios: Vec<Scenario> = vec![
        (
            "waiting-parallel",
            Box::new(|_| crate::kernel::waiting::tests::multi_gate_world(16)),
            160,
            None,
            true,
        ),
        (
            "waiting-parallel-overflow",
            Box::new(|_| crate::kernel::waiting::tests::multi_gate_world(16)),
            12,
            Some(32),
            true,
        ),
        (
            "conflict-parallel",
            Box::new(move |_| {
                crate::admin::cutover_migration::tests::conflict_scale_world(
                    Arc::clone(&revision),
                    16,
                )
            }),
            640,
            None,
            true,
        ),
        (
            "signals-clock-parallel",
            Box::new(signals_world),
            600,
            Some(1_024 * 1_024),
            false,
        ),
    ];
    let mut actual = Vec::new();
    for (name, build, ticks, bound, dispatch_required) in &scenarios {
        let mut baseline: Option<String> = None;
        for raw_workers in [1_u32, 2, 4, 8, 16] {
            let workers = NonZeroU32::new(raw_workers).expect("nonzero workers");
            let mut world = build(workers);
            // 多 worker 场景经世界独占执行资源的计划/资源准备重跑，与安装合同同一路径。
            reinstall_execution(&mut world, workers);
            if raw_workers > 1 {
                assert_eq!(
                    world.execution.thread_ids().len() + 1,
                    raw_workers as usize,
                    "{name} workers={raw_workers} must run on a real worker pool"
                );
            }
            if *dispatch_required {
                let active = world
                    .state
                    .committed
                    .live_order
                    .iter()
                    .filter(|handle| {
                        world
                            .state
                            .vehicle_state(**handle)
                            .is_some_and(|state| state.status == crate::VehicleStatus::Active)
                    })
                    .count();
                assert!(
                    active >= DISPATCH_MIN_ACTIVE,
                    "{name} must keep at least {DISPATCH_MIN_ACTIVE} active vehicles, got {active}"
                );
            }
            let rerun = trace_with_workers(world, *ticks, true, *bound, workers);
            match &baseline {
                None => baseline = Some(rerun),
                Some(expected) => assert_eq!(
                    &rerun, expected,
                    "{name} diverged between worker 1 and workers={raw_workers}"
                ),
            }
        }
        actual.push(format!("{name}={}", baseline.expect("baseline trace")));
    }
    let actual = actual.join("\n");
    eprintln!("PHASE_PARALLEL_BASELINE_BEGIN\n{actual}\nPHASE_PARALLEL_BASELINE_END");
    // 值冻结自 worker 1 融合路径（与安装合同同一计划/资源准备），digest 管线由
    // pre-refactor fixture phase-protocol-e011745e.txt 锚定；不得用被测实现重生成。
    const EXPECTED: &str = include_str!("../../tests/fixtures/phase-parallel-matrix-c009d2dc.txt");
    assert_eq!(actual, EXPECTED.trim_end());
}

#[test]
fn input_and_preparation_errors_precede_staged_failure() {
    for waiting in [true, false] {
        let mut world = if waiting {
            crate::kernel::waiting::tests::multi_gate_world(2)
        } else {
            crate::admin::cutover_migration::tests::conflict_scale_world(
                crate::admin::cutover_migration::tests::conflict_scale_revision(),
                2,
            )
        };
        let before = checkpoint(&world);
        let delta = world.config().fixed_delta_time_ms();
        STEP_FAILPOINT.with(|slot| slot.set(Some(StepFailpoint::AfterGrants)));
        assert!(matches!(
            world.step(TickInput::new(delta + 1)),
            Err(StepError::DeltaMismatch { .. })
        ));
        crate::kernel::conflict::set_allocation_failpoint(Some(0));
        let result = world.step(TickInput::new(delta));
        crate::kernel::conflict::set_allocation_failpoint(None);
        STEP_FAILPOINT.with(|slot| slot.set(None));
        assert_eq!(result, Err(StepError::ConflictScratchAllocFailed));
        assert_eq!(checkpoint(&world), before);
        world.step(TickInput::new(delta)).unwrap();
    }
}

#[test]
fn occupancy_candidate_preserves_exact_trace_and_first_error_priority() {
    crate::kernel::exact_path_research::with_candidate(true, || {
        exact_baseline_trace_and_retry_match();
        input_and_preparation_errors_precede_staged_failure();
    });
}
