//! #972 证据构建：调度统计与资源账。只在 `execution-evidence` feature 下编译。
//!
//! 账本归世界所有；执行作用域把它挂到协调线程上，拍内阶段与分发只在协调线程登记，
//! 块计时经分发自带的原子量汇总，不读写交通状态，也不参与分发决策。

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// 拍内阶段；`Unstaged` 收纳执行作用域内阶段之外的时间与分发（生命周期命令、
/// 阶段间隙）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Stage {
    Unstaged,
    Preflight,
    Occupancy,
    WaitingPrepare,
    ConflictPrepare,
    MotionLoop,
    FrontierClassify,
    WaitingFinalize,
    Signals,
    ConflictFinalize,
    WaitingOutputs,
    Validate,
    Commit,
}

const STAGE_COUNT: usize = 13;

const STAGE_NAMES: [&str; STAGE_COUNT] = [
    "unstaged",
    "preflight",
    "occupancy",
    "waiting_prepare",
    "conflict_prepare",
    "motion_loop",
    "frontier_classify",
    "waiting_finalize",
    "signals",
    "conflict_finalize",
    "waiting_outputs",
    "validate",
    "commit",
];

/// 有融合 / 分发 / 回退三选一决策的独立计算段。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phase {
    WaitingPreview,
    ConflictCandidates,
    Motion,
}

const PHASE_COUNT: usize = 3;

const PHASE_NAMES: [&str; PHASE_COUNT] = ["waiting_preview", "conflict_candidates", "motion"];

/// 本拍该段由谁执行。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Route {
    Dispatched,
    /// 单线程执行器或强制融合：没有可分发的池。
    FusedSerialExecutor,
    /// 工作量低于分发门槛。
    FusedBelowThreshold,
    /// 本拍工作量为零。
    FusedEmpty,
    /// 可选并行暂存预留失败，退回同一领域原语的融合求值。
    ScratchFallback,
}

/// 融合时按执行器与工作量给出原因。
pub(crate) const fn fused(pool: bool, workload: usize) -> Route {
    if !pool {
        Route::FusedSerialExecutor
    } else if workload == 0 {
        Route::FusedEmpty
    } else {
        Route::FusedBelowThreshold
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct DispatchTotals {
    dispatches: u64,
    inline_dispatches: u64,
    wall_ns: u64,
    inline_wall_ns: u64,
    blocks: u64,
    skipped_blocks: u64,
    extra_work_blocks: u64,
    thread_slots: u64,
    busy_ns: u64,
    critical_path_ns: u64,
    max_block_ns: u64,
    output_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct StageTotals {
    calls: u64,
    wall_ns: u64,
    dispatch: DispatchTotals,
}

/// 世界持有的证据账本；首次进入执行作用域时创建。`take` 取走自上次以来的累计。
#[derive(Debug, Default)]
pub(crate) struct Ledger {
    steps: u64,
    step_wall_ns: u64,
    stages: [StageTotals; STAGE_COUNT],
    routes: [[u64; 5]; PHASE_COUNT],
    current: Option<Stage>,
    /// 当前执行作用域内各阶段墙钟之和；作用域结束时余下部分记入 `unstaged`。
    staged_in_scope_ns: u64,
}

impl Ledger {
    pub(crate) fn take(&mut self) -> crate::ExecutionEvidence {
        let ledger = std::mem::take(self);
        crate::ExecutionEvidence {
            steps: ledger.steps,
            step_wall_ns: ledger.step_wall_ns,
            stages: ledger
                .stages
                .iter()
                .zip(STAGE_NAMES)
                .map(|(totals, stage)| crate::StageEvidence {
                    stage,
                    calls: totals.calls,
                    wall_ns: totals.wall_ns,
                    dispatch: crate::DispatchEvidence {
                        dispatches: totals.dispatch.dispatches,
                        inline_dispatches: totals.dispatch.inline_dispatches,
                        wall_ns: totals.dispatch.wall_ns,
                        inline_wall_ns: totals.dispatch.inline_wall_ns,
                        blocks: totals.dispatch.blocks,
                        skipped_blocks: totals.dispatch.skipped_blocks,
                        extra_work_blocks: totals.dispatch.extra_work_blocks,
                        thread_slots: totals.dispatch.thread_slots,
                        busy_ns: totals.dispatch.busy_ns,
                        critical_path_ns: totals.dispatch.critical_path_ns,
                        max_block_ns: totals.dispatch.max_block_ns,
                        output_bytes: totals.dispatch.output_bytes,
                    },
                })
                .collect(),
            routes: ledger
                .routes
                .iter()
                .zip(PHASE_NAMES)
                .map(|(counts, phase)| crate::RouteEvidence {
                    phase,
                    dispatched: counts[0],
                    fused_serial_executor: counts[1],
                    fused_below_threshold: counts[2],
                    fused_empty: counts[3],
                    scratch_fallback: counts[4],
                })
                .collect(),
        }
    }
}

thread_local! {
    static ACTIVE: RefCell<Option<Box<Ledger>>> = const { RefCell::new(None) };
}

fn with_active(update: impl FnOnce(&mut Ledger)) {
    ACTIVE.with(|active| {
        if let Some(ledger) = active.borrow_mut().as_mut() {
            update(ledger);
        }
    });
}

fn active() -> bool {
    ACTIVE.with(|active| active.borrow().is_some())
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// 在当前线程（执行作用域的协调线程）挂上世界账本运行；返回或展开时取回。
pub(crate) fn scoped<R>(slot: &mut Option<Box<Ledger>>, run: impl FnOnce() -> R) -> R {
    struct Restore<'a>(&'a mut Option<Box<Ledger>>);
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            *self.0 = ACTIVE.with(RefCell::take);
        }
    }
    /// 作用域墙钟扣除阶段覆盖的部分记入 `unstaged`：阶段之外的分发（生命周期
    /// 命令、阶段间隙）都落在这段时间内，串行时间 = 墙钟 − 分发墙钟不为负。
    struct Unstaged(Instant);
    impl Drop for Unstaged {
        fn drop(&mut self) {
            let ns = elapsed_ns(self.0);
            with_active(|ledger| {
                let totals = &mut ledger.stages[Stage::Unstaged as usize];
                totals.calls += 1;
                totals.wall_ns = totals
                    .wall_ns
                    .saturating_add(ns.saturating_sub(ledger.staged_in_scope_ns));
                ledger.staged_in_scope_ns = 0;
            });
        }
    }
    let ledger = slot.take().unwrap_or_default();
    let previous = ACTIVE.with(|active| active.replace(Some(ledger)));
    assert!(previous.is_none(), "execution evidence scope nested");
    let _restore = Restore(slot);
    let _unstaged = Unstaged(Instant::now());
    run()
}

/// 一次固定步进的墙钟。
pub(crate) struct StepSpan(Option<Instant>);

pub(crate) fn step() -> StepSpan {
    StepSpan(active().then(Instant::now))
}

impl Drop for StepSpan {
    fn drop(&mut self) {
        if let Some(started) = self.0 {
            let ns = elapsed_ns(started);
            with_active(|ledger| {
                ledger.steps += 1;
                ledger.step_wall_ns = ledger.step_wall_ns.saturating_add(ns);
            });
        }
    }
}

/// 阶段墙钟；期间开始的分发记入该阶段。阶段不嵌套。
pub(crate) struct StageSpan(Option<(Stage, Instant)>);

pub(crate) fn stage(stage: Stage) -> StageSpan {
    let mut started = None;
    with_active(|ledger| {
        ledger.current = Some(stage);
        started = Some((stage, Instant::now()));
    });
    StageSpan(started)
}

impl Drop for StageSpan {
    fn drop(&mut self) {
        if let Some((stage, started)) = self.0 {
            let ns = elapsed_ns(started);
            with_active(|ledger| {
                let totals = &mut ledger.stages[stage as usize];
                totals.calls += 1;
                totals.wall_ns = totals.wall_ns.saturating_add(ns);
                ledger.staged_in_scope_ns = ledger.staged_in_scope_ns.saturating_add(ns);
                ledger.current = None;
            });
        }
    }
}

/// 登记一段独立计算本拍的执行路径。
pub(crate) fn route(phase: Phase, route: Route) {
    with_active(|ledger| ledger.routes[phase as usize][route as usize] += 1);
}

/// 一次分发（提交到完整 join）的统计；块在任意池线程上经原子量汇总，
/// 协调线程在 Drop 时并入当前阶段。账本未挂上时不计时。
pub(crate) struct DispatchProbe {
    started: Option<Instant>,
    inline: bool,
    output_bytes: AtomicU64,
    blocks: AtomicU64,
    skipped: AtomicU64,
    extra: AtomicU64,
    threads: AtomicU64,
    /// 按执行线程（池序号，池外调用线程记末位）累计的块计算时间。
    thread_busy_ns: [AtomicU64; 64],
    max_block_ns: AtomicU64,
}

pub(crate) fn dispatch(inline: bool) -> DispatchProbe {
    DispatchProbe {
        started: active().then(Instant::now),
        inline,
        output_bytes: AtomicU64::new(0),
        blocks: AtomicU64::new(0),
        skipped: AtomicU64::new(0),
        extra: AtomicU64::new(0),
        threads: AtomicU64::new(0),
        thread_busy_ns: [const { AtomicU64::new(0) }; 64],
        max_block_ns: AtomicU64::new(0),
    }
}

impl DispatchProbe {
    /// 计时一块计算并登记执行线程（池线程按序号，池外调用线程记最高位）。
    pub(crate) fn block<R>(&self, run: impl FnOnce() -> R) -> R {
        if self.started.is_none() {
            return run();
        }
        let started = Instant::now();
        let result = run();
        let ns = elapsed_ns(started);
        let bit = rayon_core::current_thread_index().map_or(63, |index| index.min(63));
        const {
            assert!(
                crate::kernel::config::MAX_SUPPORTED_WORKERS < 64,
                "池线程序号与池外调用线程须各占一个线程槽"
            );
        }
        self.threads.fetch_or(1 << bit, Ordering::Relaxed);
        self.blocks.fetch_add(1, Ordering::Relaxed);
        self.thread_busy_ns[bit].fetch_add(ns, Ordering::Relaxed);
        self.max_block_ns.fetch_max(ns, Ordering::Relaxed);
        result
    }

    /// 整块晚于已错位置、未执行。
    pub(crate) fn note_skipped(&self) {
        if self.started.is_some() {
            self.skipped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// 已有错误位置时仍执行的块（完整 join 的错误后多做工作）。
    pub(crate) fn note_extra_work(&self) {
        if self.started.is_some() {
            self.extra.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// 分发期间按块追加的输出缓冲字节。
    pub(crate) fn note_output(&self, bytes: usize) {
        if self.started.is_some() {
            self.output_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        }
    }
}

impl Drop for DispatchProbe {
    fn drop(&mut self) {
        let Some(started) = self.started else {
            return;
        };
        let wall = elapsed_ns(started);
        // 关键路径取单线程块时间之和的最大者：同一线程连续执行的块不算等待。
        let (busy, critical) = self
            .thread_busy_ns
            .iter_mut()
            .map(|busy| *busy.get_mut())
            .fold((0_u64, 0_u64), |(sum, max), busy| {
                (sum.saturating_add(busy), max.max(busy))
            });
        let totals = DispatchTotals {
            dispatches: 1,
            inline_dispatches: u64::from(self.inline),
            wall_ns: wall,
            inline_wall_ns: if self.inline { wall } else { 0 },
            blocks: *self.blocks.get_mut(),
            skipped_blocks: *self.skipped.get_mut(),
            extra_work_blocks: *self.extra.get_mut(),
            thread_slots: u64::from(self.threads.get_mut().count_ones()),
            busy_ns: busy,
            critical_path_ns: critical,
            max_block_ns: *self.max_block_ns.get_mut(),
            output_bytes: *self.output_bytes.get_mut(),
        };
        with_active(|ledger| {
            let stage = ledger.current.unwrap_or(Stage::Unstaged);
            let sum = &mut ledger.stages[stage as usize].dispatch;
            sum.dispatches += totals.dispatches;
            sum.inline_dispatches += totals.inline_dispatches;
            sum.wall_ns = sum.wall_ns.saturating_add(totals.wall_ns);
            sum.inline_wall_ns = sum.inline_wall_ns.saturating_add(totals.inline_wall_ns);
            sum.blocks += totals.blocks;
            sum.skipped_blocks += totals.skipped_blocks;
            sum.extra_work_blocks += totals.extra_work_blocks;
            sum.thread_slots += totals.thread_slots;
            sum.busy_ns = sum.busy_ns.saturating_add(totals.busy_ns);
            sum.critical_path_ns = sum.critical_path_ns.saturating_add(totals.critical_path_ns);
            sum.max_block_ns = sum.max_block_ns.max(totals.max_block_ns);
            sum.output_bytes = sum.output_bytes.saturating_add(totals.output_bytes);
        });
    }
}

/// 一个观察区间内的调度证据：自上次 [`crate::TrafficWorld::take_execution_evidence`]
/// 以来在本世界执行作用域内累计的值。只在证据构建（`execution-evidence`）中存在；
/// 计时含插桩开销，不作为性能认证。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExecutionEvidence {
    /// 完成的固定步进次数（含失败返回的步进）。
    pub steps: u64,
    /// 这些步进在执行作用域内的墙钟合计。
    pub step_wall_ns: u64,
    /// 按拍内阶段固定顺序列出；`unstaged` 收纳执行作用域内阶段之外的时间与分发。
    pub stages: Vec<StageEvidence>,
    /// 有融合 / 分发 / 回退决策的独立计算段，各自的路径次数。
    pub routes: Vec<RouteEvidence>,
}

/// 单个拍内阶段的墙钟与其中开始的分发。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StageEvidence {
    /// 阶段名。
    pub stage: &'static str,
    /// 进入次数。
    pub calls: u64,
    /// 墙钟合计；减去 `dispatch.wall_ns` 为协调线程串行时间。
    pub wall_ns: u64,
    /// 本阶段内开始的分发。
    pub dispatch: DispatchEvidence,
}

/// 分发统计合计。关键路径是每次分发中单个线程块计算时间之和的最大者；
/// 屏障与调度等待按每次分发的 `墙钟 − 关键路径` 求和，即
/// `wall_ns − critical_path_ns`；实际并发为 `busy_ns / wall_ns`。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DispatchEvidence {
    /// 分发次数（含单线程执行器上的内联执行）。
    pub dispatches: u64,
    /// 其中在调用线程上内联执行的次数。
    pub inline_dispatches: u64,
    /// 提交到完整 join 的墙钟合计。
    pub wall_ns: u64,
    /// 其中内联执行的墙钟。
    pub inline_wall_ns: u64,
    /// 实际执行的块数。
    pub blocks: u64,
    /// 晚于已错位置而整块跳过的块数。
    pub skipped_blocks: u64,
    /// 已有错误位置时仍执行的块数（错误后多做工作）。
    pub extra_work_blocks: u64,
    /// 每次分发实际执行过块的不同线程数之和（整块跳过不计）。
    pub thread_slots: u64,
    /// 各块计算时间合计。
    pub busy_ns: u64,
    /// 每次分发关键路径（单线程块时间之和的最大者）之和。
    pub critical_path_ns: u64,
    /// 单块最长时间。
    pub max_block_ns: u64,
    /// 保序分发写入的结果槽位字节（物化）；分段缓冲与原位写入存储的分发不计，
    /// 分段缓冲的容量见资源账的协调器分段缓冲。
    pub output_bytes: u64,
}

/// 一段独立计算本区间内各执行路径的次数，路径互斥。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RouteEvidence {
    /// 独立计算段名。
    pub phase: &'static str,
    /// 实际分发。
    pub dispatched: u64,
    /// 单线程执行器（或无池）而融合。
    pub fused_serial_executor: u64,
    /// 工作量低于分发门槛而融合。
    pub fused_below_threshold: u64,
    /// 工作量为零。
    pub fused_empty: u64,
    /// 可选并行暂存预留失败，退回融合。
    pub scratch_fallback: u64,
}

/// 调用时刻的资源账。容量按 `capacity × 元素大小` 计，不含分配器开销；
/// 线程栈只报告请求的栈大小，平台实际保留与提交字节不在此列。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResourceEvidence {
    /// 已安装执行配置的 worker 数。
    pub workers: u32,
    /// 辅助池线程数（单 worker 不创建线程，为 0）。
    pub pool_threads: u32,
    /// 池线程请求的栈大小合计；平台可能向上取整，实际保留不小于此值。
    pub worker_stack_requested_bytes: u64,
    /// 共享静态路网（跨世界共享，相加时去重）。
    pub shared_network_bytes: u64,
    /// 世界绑定（来源、策略、编译表）。
    pub binding_bytes: u64,
    /// 已提交状态。
    pub committed_bytes: u64,
    /// 派生索引。
    pub derived_bytes: u64,
    /// 拍内 scratch 与队列（容量跨拍保留，近似峰值）。
    pub workspace_bytes: u64,
    /// 管理数据（迁移日志等）。
    pub administrative_bytes: u64,
    /// 活动执行计划。候选与退休计划只在切换事务中存在，不在此列。
    pub active_plan_bytes: u64,
    /// 执行池持有的协调器分段缓冲。
    pub coordinator_scratch_bytes: u64,
}

#[cfg(test)]
mod tests {
    use crate::TickInput;
    use crate::admin::cutover::tests::transaction_tests::world_with_vehicle;
    use crate::kernel::execution::{DispatchSlot, RESOURCE_TEST_LOCK, WorldExecution};
    use std::num::NonZeroU32;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn config(workers: u32) -> crate::ExecutionConfig {
        crate::ExecutionConfig::new(NonZeroU32::new(workers).unwrap())
    }

    fn stage<'a>(evidence: &'a crate::ExecutionEvidence, name: &str) -> &'a super::StageEvidence {
        evidence
            .stages
            .iter()
            .find(|stage| stage.stage == name)
            .unwrap()
    }

    #[test]
    fn dispatch_evidence_matches_dispatch_statistics_and_resets_on_take() {
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        for workers in [1, 4] {
            let (mut world, _, _) = world_with_vehicle(true);
            world.execution = WorldExecution::start_private(config(workers), &world.state);
            let first_error = AtomicUsize::new(usize::MAX);
            let mut output: Vec<DispatchSlot<u64>> = vec![DispatchSlot::Pending; 16];
            let stats = world.execution.run(&mut world.state, |state, resources| {
                resources.try_for_each_chunk(
                    state.read_view(),
                    &mut output,
                    &first_error,
                    4,
                    |_, start, chunk: &mut [DispatchSlot<u64>]| {
                        for (offset, slot) in chunk.iter_mut().enumerate() {
                            let index = start + offset;
                            if index == 6 {
                                first_error.fetch_min(index, Ordering::SeqCst);
                                *slot = DispatchSlot::Done(Err(crate::StepError::NonFiniteMotion));
                                return;
                            }
                            *slot = DispatchSlot::Done(Ok(index as u64));
                        }
                    },
                )
            });
            let evidence = world.take_execution_evidence();
            assert_eq!(evidence.steps, 0);
            let unstaged = stage(&evidence, "unstaged").dispatch;
            assert_eq!(unstaged.dispatches, 1);
            assert_eq!(unstaged.inline_dispatches, u64::from(workers == 1));
            assert_eq!(unstaged.blocks, stats.dispatched_chunks as u64);
            assert_eq!(unstaged.skipped_blocks, stats.skipped_chunks as u64);
            assert_eq!(unstaged.extra_work_blocks, stats.extra_work_chunks as u64);
            // 跳过的块不计执行线程：证据只数真正算过块的线程。
            assert!((1..=stats.participating_threads as u64).contains(&unstaged.thread_slots));
            assert_eq!(unstaged.output_bytes, size_of_val(output.as_slice()) as u64);
            // 作用域墙钟覆盖其中的分发：串行时间不为负。
            assert!(stage(&evidence, "unstaged").wall_ns >= unstaged.wall_ns);
            assert!(unstaged.critical_path_ns <= unstaged.wall_ns);
            assert!(unstaged.max_block_ns <= unstaged.critical_path_ns);
            assert!(unstaged.busy_ns >= unstaged.critical_path_ns);
            if workers == 1 {
                // 单线程内联：关键路径就是全部块时间。
                assert_eq!(unstaged.critical_path_ns, unstaged.busy_ns);
            }
            let again = world.take_execution_evidence();
            assert_eq!(
                again
                    .stages
                    .iter()
                    .map(|stage| stage.dispatch.dispatches)
                    .sum::<u64>(),
                0
            );
        }
    }

    #[test]
    fn steps_record_stages_routes_and_resources() {
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        for workers in [1, 4] {
            let (mut world, _, _) = world_with_vehicle(true);
            world.execution = WorldExecution::start_private(config(workers), &world.state);
            let delta = world.state.binding.config.fixed_delta_time_ms();
            for _ in 0..3 {
                world.step(TickInput::new(delta)).unwrap();
            }
            let evidence = world.take_execution_evidence();
            assert_eq!(evidence.steps, 3);
            let staged: u64 = evidence.stages.iter().map(|stage| stage.wall_ns).sum();
            assert!(staged >= evidence.step_wall_ns);
            for stage in &evidence.stages {
                assert!(stage.wall_ns >= stage.dispatch.wall_ns, "{}", stage.stage);
            }
            for name in [
                "preflight",
                "occupancy",
                "motion_loop",
                "validate",
                "commit",
            ] {
                assert_eq!(stage(&evidence, name).calls, 3, "{name}");
            }
            let motion = evidence
                .routes
                .iter()
                .find(|route| route.phase == "motion")
                .unwrap();
            if workers == 1 {
                assert_eq!(motion.fused_serial_executor, 3);
                assert!(evidence.stages.iter().all(|stage| {
                    stage.dispatch.inline_dispatches == stage.dispatch.dispatches
                }));
            } else {
                assert_eq!(motion.fused_below_threshold, 3);
            }
            let resources = world.resource_evidence();
            assert_eq!(resources.workers, workers);
            let pool_threads = if workers == 1 { 0 } else { workers };
            assert_eq!(resources.pool_threads, pool_threads);
            assert_eq!(
                resources.worker_stack_requested_bytes,
                u64::from(pool_threads) * 2 * 1_024 * 1_024
            );
            assert!(resources.committed_bytes > 0);
            assert!(resources.workspace_bytes > 0);
            assert!(resources.active_plan_bytes > 0);
            assert_eq!(resources.coordinator_scratch_bytes == 0, workers == 1);
        }
    }
}
