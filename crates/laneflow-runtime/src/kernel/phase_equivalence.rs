//! 相同文件在 e011745e94986a17049c66e730d54ac9fccc59f9 与当前实现运行。
//! 只比较公开状态/批次及既有日志记录；不读取分区、容量或私有 grant serial。
//!
//! #705 整步等价：`trace_with_workers` 把同一 digest 轨迹在真实多 worker 世界
//! 下重跑（`WorldExecution::start_private` 重建世界独占执行资源，等同安装合同
//! 的计划/资源准备），逐场景断言 worker 4/16 与 worker 1 完全一致，并与
//! 固定 fixture `phase-parallel-matrix-c009d2dc.txt` 一致；fixture 值取自
//! worker 1 融合路径，其 digest 管线已由 pre-refactor fixture
//! `phase-protocol-e011745e.txt` 锚定。worker 档位与拍数经 #915 outcome 级
//! 测量论证降档（车辆场景档位 [1,2,4,16]、signals [1,4,16]，拍数
//! 64/12/320/360），恢复+续跑比较抽样于分区形状唯一的运行配置 workers∈{1,2,16}
//! （续跑 digest 跨抽样配置对照）。

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
/// 车辆-bearing 场景的活动数下限：证明场景确实 populated（与生产分发
/// 阈值无关；多 worker 真实分发经 force_preview_dispatch 强制）。
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
    trace_with_workers(
        world,
        ticks,
        retry,
        journal_bound,
        NonZeroU32::MIN,
        RestoreCheck::IntoDigest,
    )
    .trace
}

/// 恢复+续跑比较的执行方式。
#[derive(Clone, Copy, PartialEq, Eq)]
enum RestoreCheck {
    /// 续跑 digest 并入轨迹 digest（e011745e pre-refactor 锚定路径，行为冻结）。
    IntoDigest,
    /// 恢复+续跑在各配置内独立断言，不进轨迹 digest（worker 矩阵中抽样的配置），
    /// 保持跨配置 digest 可比较；续跑 digest 随 [`TraceDigest`] 返回，
    /// 由调用方跨抽样配置对照。
    SideAssert,
    /// 不执行（矩阵中未抽样的配置；恢复机制与 worker 数无关，抽样覆盖）。
    Skip,
}

/// `trace_with_workers` 的产物。
struct TraceDigest {
    /// 轨迹 digest：fixture 比较与跨配置相等的对象。
    trace: String,
    /// 恢复+续跑后的状态 digest；仅 采用 [`RestoreCheck::SideAssert`] 的运行配置产生。
    continued: Option<String>,
}

/// 恢复快照并续跑一拍：恢复不重建历史 latest batches，按快照局部身份比较
/// 后续语义。返回续跑后状态 digest 的 `{continued:?}` 形式。
fn restore_and_continue(world: &mut TrafficWorld, workers: NonZeroU32, input: TickInput) -> String {
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
    format!("{continued:?}")
}

fn trace_with_workers(
    mut world: TrafficWorld,
    ticks: usize,
    retry: bool,
    journal_bound: Option<u64>,
    workers: NonZeroU32,
    restore: RestoreCheck,
) -> TraceDigest {
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
    let continued = match restore {
        RestoreCheck::Skip => None,
        RestoreCheck::IntoDigest => {
            let continued = restore_and_continue(&mut world, workers, input);
            digest.update(continued.as_bytes());
            None
        }
        RestoreCheck::SideAssert => Some(restore_and_continue(&mut world, workers, input)),
    };
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
    TraceDigest {
        trace: format!("{hex}:{event_count}"),
        continued,
    }
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
    // 场景反复创建真实线程池：持有资源测试锁，避免与 execution.rs 测试族的
    // 真实线程池测试并发抢占 CPU（工作线程计数已按建池线程分开记）。
    let _lock = crate::kernel::execution::RESOURCE_TEST_LOCK.lock().unwrap();
    // 生产分发阈值为保守 1_024；矩阵场景为 16 车，经强制入口保持
    // 多 worker 真实分发覆盖（digest 轨迹与路径无关，冻结 fixture 不变）。
    let _force = crate::kernel::waiting::force_preview_dispatch();
    let revision = crate::admin::cutover_migration::tests::conflict_scale_revision();
    // 车辆-bearing 场景的活动数必须达到分发门槛，多 worker 运行才真正分发；
    // 信号时钟场景无车辆，覆盖空工作集下的池世界整步。
    // 拍数论证（#915，w=1 outcome 级测量：各 outcome/事件/状态种类首次出现拍与
    // 活动尾拍；详见 Issue 测量表）：
    // - waiting 64：种类穷尽于 35、活动止于 18（含 Completed@34），近 2× 余量；
    // - conflict 320：所有 outcome 种类第 0 拍穷尽，决策/事件活动止于 307；
    //   Completed@592 的机制由 waiting-parallel 与 #814 waiting 场景覆盖；
    // - signals 360：信号组切换发生在 0/289/339，保留全部切换加余量；
    // - overflow 12：journal 溢出形状所需，不变。
    type Scenario = (
        &'static str,
        Box<dyn Fn(NonZeroU32) -> TrafficWorld>,
        usize,
        Option<u64>,
        bool,
        &'static [u32],
    );
    let scenarios: Vec<Scenario> = vec![
        (
            "waiting-parallel",
            Box::new(|_| crate::kernel::waiting::tests::multi_gate_world(16)),
            64,
            None,
            true,
            &[1, 2, 4, 16],
        ),
        (
            "waiting-parallel-overflow",
            Box::new(|_| crate::kernel::waiting::tests::multi_gate_world(16)),
            12,
            Some(32),
            true,
            &[1, 2, 4, 16],
        ),
        (
            "conflict-parallel",
            Box::new(move |_| {
                crate::admin::cutover_migration::tests::conflict_scale_world(
                    Arc::clone(&revision),
                    16,
                )
            }),
            320,
            None,
            true,
            &[1, 2, 4, 16],
        ),
        (
            "signals-clock-parallel",
            Box::new(signals_world),
            360,
            Some(1_024 * 1_024),
            false,
            &[1, 4, 16],
        ),
    ];
    let mut actual = Vec::new();
    for (name, build, ticks, bound, dispatch_required, tiers) in &scenarios {
        let mut baseline: Option<String> = None;
        let mut continued_baseline: Option<String> = None;
        // #915 档位论证：8 与 16 同构（超订细分块）裁去；16 保留——4 vCPU CI 上
        // 线程超订配置调度扰动最强、最易暴露顺序依赖。车辆场景保留 2：预览发现
        // parts=线程数×4 被车辆数 16 封顶，w=2 是唯一每段 2 车（span=2）的配置，
        // 4/16 均退化为每段 1 车；signals 无车辆，2 无新分区形状。
        for &raw_workers in *tiers {
            let workers = NonZeroU32::new(raw_workers).expect("nonzero workers");
            let mut world = build(workers);
            // 多 worker 场景经世界独占执行资源的计划/资源准备重跑，与安装合同同一路径。
            reinstall_execution(&mut world, workers);
            if raw_workers > 1 {
                assert_eq!(
                    world.execution.thread_ids().len(),
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
            // 恢复+续跑比较抽样于分区形状唯一的配置：1=融合参考、2=span=2 多车段、
            // 16=span=1 超订；4 与 16 同形（span=1）跳过。比较不进轨迹 digest，
            // 跨配置 digest 保持可比较；抽样配置的续跑 digest 跨配置对照（配置内断言
            // 观测不到恢复后首拍的跨 worker 发散）。
            let restore = if matches!(raw_workers, 1 | 2 | 16) {
                RestoreCheck::SideAssert
            } else {
                RestoreCheck::Skip
            };
            let rerun = trace_with_workers(world, *ticks, true, *bound, workers, restore);
            if let Some(continued) = &rerun.continued {
                match &continued_baseline {
                    None => continued_baseline = Some(continued.clone()),
                    Some(expected) => assert_eq!(
                        continued, expected,
                        "{name} restored continuation diverged between worker 1 and workers={raw_workers}"
                    ),
                }
            }
            match &baseline {
                None => baseline = Some(rerun.trace),
                Some(expected) => assert_eq!(
                    &rerun.trace, expected,
                    "{name} diverged between worker 1 and workers={raw_workers}"
                ),
            }
        }
        actual.push(format!("{name}={}", baseline.expect("baseline trace")));
    }
    let actual = actual.join("\n");
    eprintln!("PHASE_PARALLEL_BASELINE_BEGIN\n{actual}\nPHASE_PARALLEL_BASELINE_END");
    // 值取自 worker 1 融合路径（与安装合同同一计划/资源准备）；#915 降档时随
    // 矩阵形状变化重生成一次——digest 管线与场景构造器仍由 pre-refactor fixture
    // phase-protocol-e011745e.txt 锚定（exact_baseline 拍数不变），此后不得用
    // 被测实现重生成。
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
fn resource_rows_match_complete_resource_lifecycle_and_retry_traces() {
    let workers = NonZeroU32::new(4).unwrap();
    for scenario in ["waiting", "conflict", "signals"] {
        let build = || match scenario {
            "waiting" => crate::kernel::waiting::tests::multi_gate_world(16),
            "conflict" => crate::admin::cutover_migration::tests::conflict_scale_world(
                crate::admin::cutover_migration::tests::conflict_scale_revision(),
                16,
            ),
            "signals" => signals_world(workers),
            _ => unreachable!(),
        };
        let selected = trace_with_workers(
            build(),
            640,
            true,
            Some(1_024 * 1_024),
            workers,
            RestoreCheck::SideAssert,
        );
        let full = crate::kernel::resource_rows::with_full_scan_oracle(|| {
            trace_with_workers(
                build(),
                640,
                true,
                Some(1_024 * 1_024),
                workers,
                RestoreCheck::SideAssert,
            )
        });
        assert_eq!(
            selected.trace, full.trace,
            "{scenario}: full resource and journal trace"
        );
        // 续跑拍同样跨模式对照：稀疏选择与全扫描若在恢复快照才暴露的 retained
        // 状态上发散，唯一显形机会是续跑拍；配置内断言只覆盖各自模式内部。
        assert_eq!(
            selected.continued, full.continued,
            "{scenario}: full resource and journal continued state"
        );
    }
}

#[test]
fn occupancy_candidate_preserves_exact_trace_and_first_error_priority() {
    crate::kernel::exact_path_research::with_candidate(true, || {
        exact_baseline_trace_and_retry_match();
        input_and_preparation_errors_precede_staged_failure();
    });
}
