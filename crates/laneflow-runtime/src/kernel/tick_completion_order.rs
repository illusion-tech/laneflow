//! #1278 整拍级完成顺序强制：固定 worker 4 真实池、强制 P2/P3/P5 分发，
//! 用领取序窗口置换把「调度相关的确定性缺陷只会偶发出现」变成必然出现。
//! 成功路径的逐拍公开状态与摘要、失败路径的首错、失败后公开状态与清除
//! 注入后的重试续跑，都必须与完成顺序无关（与单 worker 融合参考一致）。
//!
//! 与 `phase_equivalence` 的分工：那边按 worker 矩阵对拍自然调度下的
//! digest 轨迹（仅强制 P2 分发）；这边三阶段全部强制分发并主动换序，
//! 覆盖同一领域原语在融合与分发路径下的交错义务差异。

use std::num::NonZeroU32;
use std::sync::Arc;

use super::phase_equivalence::checkpoint;
use super::{force_motion_dispatch, inject_motion_nonfinite};
use crate::kernel::conflict_tick::force_conflict_dispatch;
use crate::kernel::execution::{
    DispatchEntry, DispatchOrderCallLog, DispatchOrderMode, RESOURCE_TEST_LOCK, WorldExecution,
    dispatch_order_schedule, force_dispatch_completion_order, take_dispatch_order_calls,
};
use crate::kernel::waiting::{force_preview_dispatch, inject_preview_errors};
use crate::{StepError, TickInput, TrafficWorld};

/// 池线程数：门的死锁安全前置要求窗口小于它。
const WORKERS: u32 = 4;
/// 领取序窗口（2..WORKERS）：窗口内单元按模式置换放行，窗口外自然执行。
const WINDOW: usize = 3;
/// 成功路径对拍拍数：等待环覆盖门序轮转，冲突场景覆盖授权与事件活动；
/// 两场景在各自拍数内活动车辆数不变（Completed 均晚于此处拍数）。
const RING_TICKS: usize = 24;
const CONFLICT_TICKS: usize = 32;

struct Scenario {
    name: &'static str,
    build: Box<dyn Fn() -> TrafficWorld>,
    ticks: usize,
    /// 断言必须出现换序证据的分发入口。
    entries: &'static [DispatchEntry],
}

fn scenarios() -> Vec<Scenario> {
    let revision = crate::admin::cutover_migration::tests::conflict_scale_revision();
    vec![
        Scenario {
            name: "waiting-ring",
            build: Box::new(|| crate::kernel::waiting::tests::multi_gate_world(16)),
            ticks: RING_TICKS,
            entries: &[
                DispatchEntry::Preview,
                DispatchEntry::Candidate,
                DispatchEntry::Motion,
            ],
        },
        Scenario {
            // 冲突场景的 P3 候选每拍约 1 块（门可达车辆少），无从换序；
            // P3 换序证据由 waiting-ring 承担，这里覆盖 P2/P5 与冲突授权。
            name: "conflict-scale",
            build: Box::new(move || {
                crate::admin::cutover_migration::tests::conflict_scale_world(
                    Arc::clone(&revision),
                    16,
                )
            }),
            ticks: CONFLICT_TICKS,
            entries: &[DispatchEntry::Preview, DispatchEntry::Motion],
        },
    ]
}

/// 与安装合同同一计划/资源准备路径，把世界重建到 worker 4 真实池。
fn reinstall_execution(world: &mut TrafficWorld) {
    world.execution = WorldExecution::start_private(
        crate::ExecutionConfig::new(NonZeroU32::new(WORKERS).unwrap()),
        &world.state,
    );
}

/// 单 worker 融合参考：每拍（含拍初）的公开状态与摘要。
fn fused_trace(scenario: &Scenario) -> Vec<String> {
    let mut world = (scenario.build)();
    let input = TickInput::new(world.config().fixed_delta_time_ms());
    let mut trace = vec![checkpoint(&world)];
    for _ in 0..scenario.ticks {
        world.step(input).unwrap();
        trace.push(checkpoint(&world));
    }
    trace
}

/// worker 4 真实池 + 三阶段强制分发；`order` 为 `None` 时自然调度（对照）。
/// 返回逐拍公开状态与完成顺序门的调用证据。
fn dispatched_trace(
    scenario: &Scenario,
    order: Option<DispatchOrderMode>,
) -> (Vec<String>, Vec<DispatchOrderCallLog>) {
    let mut world = (scenario.build)();
    reinstall_execution(&mut world);
    let _p2 = force_preview_dispatch();
    let _p3 = force_conflict_dispatch();
    let _p5 = force_motion_dispatch();
    let _gate = order.map(|mode| force_dispatch_completion_order(mode, WINDOW, WORKERS as usize));
    let input = TickInput::new(world.config().fixed_delta_time_ms());
    let mut trace = vec![checkpoint(&world)];
    for _ in 0..scenario.ticks {
        world.step(input).unwrap();
        trace.push(checkpoint(&world));
    }
    (trace, take_dispatch_order_calls())
}

/// 每次被门作用的调用都真实换序：单元数不少于窗口、多线程参与、放行序
/// 恰为武装模式的放行表；且每个入口在整个运行中至少有一次单元数超过
/// 窗口（窗口内外单元并存，换序才有自然序可对照）。
fn assert_forced_completion_orders(
    name: &str,
    entries: &[DispatchEntry],
    mode: DispatchOrderMode,
    calls: &[DispatchOrderCallLog],
) {
    let expected = dispatch_order_schedule(mode, WINDOW);
    for entry in entries {
        let entry_calls: Vec<&DispatchOrderCallLog> =
            calls.iter().filter(|call| call.entry == *entry).collect();
        assert!(
            !entry_calls.is_empty(),
            "{name}: no dispatched {entry:?} call recorded"
        );
        assert!(
            entry_calls.iter().any(|call| call.claimed > WINDOW),
            "{name}: {entry:?} never dispatched more units than the window"
        );
        for call in entry_calls {
            assert!(
                call.claimed >= WINDOW,
                "{name}: {entry:?} needs at least window-sized units"
            );
            assert!(
                call.participating_threads >= 2,
                "{name}: {entry:?} must run on multiple pool threads"
            );
            assert_eq!(
                call.released, expected,
                "{name}: {entry:?} release order under {mode:?}"
            );
        }
    }
}

#[test]
fn whole_tick_state_matches_fused_reference_across_completion_orders() {
    let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
    for scenario in scenarios() {
        let reference = fused_trace(&scenario);
        // 自然调度（强制三阶段分发、不换序）对照：分发路径本身与融合一致。
        let (natural, natural_calls) = dispatched_trace(&scenario, None);
        assert_eq!(
            natural, reference,
            "{}: dispatched natural order must match the fused reference",
            scenario.name
        );
        assert!(
            natural_calls.is_empty(),
            "{}: no gate armed for the natural control",
            scenario.name
        );
        for mode in [DispatchOrderMode::Reverse, DispatchOrderMode::Rotate(1)] {
            let (trace, calls) = dispatched_trace(&scenario, Some(mode));
            assert_eq!(
                trace, reference,
                "{}: state diverged under {mode:?}",
                scenario.name
            );
            assert_forced_completion_orders(scenario.name, scenario.entries, mode, &calls);
        }
    }
}

/// 失败注入的两档：P2 多位置（P5 同时武装但不被触及）与 P5 多位置。
/// 两档的首错都取最早逻辑位置，与各错误到达协调器的先后无关。
fn inject_failure_case(case: &str, world: &mut TrafficWorld) -> StepError {
    let world_id = world.state.binding.world_id;
    match case {
        // live 序 2 的 WaitingInvariantViolation 早于 13 的 NonFiniteMotion；
        // P5 注入同拍武装，覆盖「多个注入错误同时在场」。
        "preview-multi" => {
            let _p2 = inject_preview_errors(world_id, &[13], &[2]);
            let _p5 = inject_motion_nonfinite(world_id, &[5, 11]);
            world
                .step(TickInput::new(world.config().fixed_delta_time_ms()))
                .unwrap_err()
        }
        // Active 紧凑位置 5 的 NonFiniteMotion 早于 11。
        _ => {
            let _p5 = inject_motion_nonfinite(world_id, &[5, 11]);
            world
                .step(TickInput::new(world.config().fixed_delta_time_ms()))
                .unwrap_err()
        }
    }
}

#[test]
fn whole_tick_first_error_and_retry_independent_of_completion_order() {
    let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
    for scenario in scenarios() {
        // 无故障自然分发参考：失败点前轨迹、失败点后的重试续跑都以它对拍。
        let (clean_trace, _) = dispatched_trace(&scenario, None);
        let fail_at = scenario.ticks / 2;
        for case in ["preview-multi", "motion-multi"] {
            for mode in [DispatchOrderMode::Reverse, DispatchOrderMode::Rotate(1)] {
                let mut world = (scenario.build)();
                reinstall_execution(&mut world);
                let _p2 = force_preview_dispatch();
                let _p3 = force_conflict_dispatch();
                let _p5 = force_motion_dispatch();
                let _gate = force_dispatch_completion_order(mode, WINDOW, WORKERS as usize);
                let input = TickInput::new(world.config().fixed_delta_time_ms());
                let mut trace = vec![checkpoint(&world)];
                for _ in 0..fail_at {
                    world.step(input).unwrap();
                    trace.push(checkpoint(&world));
                }
                assert_eq!(
                    trace,
                    clean_trace[..=fail_at],
                    "{}: pre-failure trace under {mode:?}",
                    scenario.name
                );
                let before = checkpoint(&world);
                let error = inject_failure_case(case, &mut world);
                let expected = match case {
                    "preview-multi" => StepError::WaitingInvariantViolation,
                    _ => StepError::NonFiniteMotion,
                };
                assert_eq!(
                    error, expected,
                    "{}: first error must be order-independent ({case})",
                    scenario.name
                );
                assert_eq!(
                    checkpoint(&world),
                    before,
                    "{}: failed step keeps published state ({case})",
                    scenario.name
                );
                assert_forced_completion_orders(
                    scenario.name,
                    scenario.entries,
                    mode,
                    &take_dispatch_order_calls(),
                );
                // 清除注入后重试成功，并回到无故障参考的续跑轨迹。
                world.step(input).unwrap();
                let mut continuation = vec![checkpoint(&world)];
                for _ in fail_at + 2..=scenario.ticks {
                    world.step(input).unwrap();
                    continuation.push(checkpoint(&world));
                }
                assert_eq!(
                    continuation,
                    clean_trace[fail_at + 1..],
                    "{}: retry continuation under {mode:?} ({case})",
                    scenario.name
                );
            }
        }
    }
}
