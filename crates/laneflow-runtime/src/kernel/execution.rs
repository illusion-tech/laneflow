//! 世界独占的持久资源；作用域之外没有交通状态借用。

use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::JoinHandle;

use laneflow_static_network::SharedNetworkRevision;

use super::state::WorldState;
use crate::{ExecutionConfig, ExecutionInitError, ExecutionPlanError, WorldGeneration};

const WORKER_STACK_BYTES: usize = 2 * 1_024 * 1_024;

#[cfg(test)]
#[path = "tests/execution_memory.rs"]
mod memory_tests;

/// 持有根的实际内存身份，不能只凭相同 NetworkRevisionId 复用。
pub(crate) struct ExecutionPlan {
    root: Arc<SharedNetworkRevision>,
    generation: WorldGeneration,
    ranges: Vec<Range<usize>>,
}

impl ExecutionPlan {
    pub(crate) fn prepare(
        state: &WorldState,
        config: ExecutionConfig,
        generation: WorldGeneration,
    ) -> Result<Self, ExecutionPlanError> {
        Self::prepare_for_root(
            &state.binding.revision,
            state.derived.active_order.len(),
            config,
            generation,
        )
    }

    pub(crate) fn prepare_for_root(
        root: &Arc<SharedNetworkRevision>,
        work_count: usize,
        config: ExecutionConfig,
        generation: WorldGeneration,
    ) -> Result<Self, ExecutionPlanError> {
        let count = usize::try_from(config.worker_count().get())
            .map_err(|_| ExecutionPlanError::SizeOverflow)?;
        count
            .checked_mul(size_of::<Range<usize>>())
            .filter(|bytes| *bytes <= isize::MAX as usize)
            .ok_or(ExecutionPlanError::SizeOverflow)?;
        #[cfg(test)]
        if PLAN_FAILURE.with(|fail| fail.get()) {
            return Err(ExecutionPlanError::ReservationFailed);
        }
        let mut ranges = Vec::new();
        ranges
            .try_reserve_exact(count)
            .map_err(|_| ExecutionPlanError::ReservationFailed)?;
        ranges.resize(count, 0..0);
        let mut plan = Self {
            root: Arc::clone(root),
            generation,
            ranges,
        };
        plan.refresh_workset(work_count);
        Ok(plan)
    }

    /// 每次操作重新划分当前顺序；计划不保存车辆引用或安装时的顺序。
    pub(crate) fn refresh_workset(&mut self, count: usize) {
        let workers = self.ranges.len();
        let quotient = count / workers;
        let remainder = count % workers;
        let mut start = 0;
        for (index, range) in self.ranges.iter_mut().enumerate() {
            let end = start + quotient + usize::from(index < remainder);
            *range = start..end;
            start = end;
        }
    }

    pub(crate) fn assert_binding(&self, state: &WorldState) {
        assert!(
            Arc::ptr_eq(&self.root, &state.binding.revision),
            "execution root binding"
        );
        assert_eq!(
            self.generation, state.binding.world_generation,
            "execution generation binding"
        );
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        size_of::<Self>() + self.ranges.capacity() * size_of::<Range<usize>>()
    }

    #[cfg(test)]
    pub(crate) fn work_len(&self) -> usize {
        self.ranges.last().map_or(0, |range| range.end)
    }
}

/// 候选是私有数据，不能独立获得执行资源或从公开 API 步进。
pub(crate) struct PreparedWorldState {
    pub(crate) state: WorldState,
    pub(crate) plan: ExecutionPlan,
}

impl std::ops::Deref for PreparedWorldState {
    type Target = WorldState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl std::ops::DerefMut for PreparedWorldState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

/// 字段析构顺序必须先关闭 Rayon registry，再等待实际 OS 线程退出。
pub(crate) struct PoolResources {
    pool: rayon_core::ThreadPool,
    _workers: WorkerJoins,
}

#[derive(Default)]
struct WorkerJoins(Vec<JoinHandle<()>>);

impl Drop for WorkerJoins {
    fn drop(&mut self) {
        for worker in self.0.drain(..) {
            let _ = worker.join();
        }
    }
}

pub(crate) enum ExecutionResources {
    Caller,
    Pool(PoolResources),
}

/// 可失败保序分发的输出槽位：`Pending` 尚未计算；`Done` 已完成（含完整领域
/// 错误）；`Skipped` 整块晚于已错位置、按完整 join 语义未执行。协调器按逻辑
/// 顺序消费，首错位置之前出现 `Pending`/`Skipped` 属完成前沿不变量违例。
#[derive(Clone)]
pub(crate) enum DispatchSlot<T> {
    Pending,
    Done(Result<T, crate::StepError>),
    Skipped,
}

/// `try_for_each_chunk` 的调度统计；只度量本阶段谁执行，不改变交通语义。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DispatchStats {
    /// 实际启动执行的块数（不含整块跳过）。
    pub(crate) dispatched_chunks: usize,
    /// 全部槽位均完成（无中途首错返回）的块数。
    pub(crate) completed_chunks: usize,
    /// 因块起点晚于已错位置而整块跳过、未执行的块数。
    pub(crate) skipped_chunks: usize,
    /// 启动时已存在已错位置仍执行的块数（完整 join 的错误后多做工作）。
    pub(crate) extra_work_chunks: usize,
    /// 实际参与本调用的线程数。
    pub(crate) threads: usize,
}

/// 单次分发调用内的廉价计数器；join 完成后汇总为 [`DispatchStats`]。
#[derive(Default)]
struct DispatchCounters {
    dispatched: AtomicUsize,
    completed: AtomicUsize,
    skipped: AtomicUsize,
    extra_work: AtomicUsize,
    threads: Mutex<Vec<std::thread::ThreadId>>,
}

impl DispatchCounters {
    fn note_thread(&self) {
        let id = std::thread::current().id();
        let mut threads = self.threads.lock().expect("dispatch stats threads");
        if !threads.contains(&id) {
            threads.push(id);
        }
    }

    fn stats(&self) -> DispatchStats {
        DispatchStats {
            dispatched_chunks: self.dispatched.load(Ordering::Relaxed),
            completed_chunks: self.completed.load(Ordering::Relaxed),
            skipped_chunks: self.skipped.load(Ordering::Relaxed),
            extra_work_chunks: self.extra_work.load(Ordering::Relaxed),
            threads: self.threads.lock().expect("dispatch stats threads").len(),
        }
    }
}

/// 执行一个输出块：整块晚于已错位置标记 `Skipped` 不执行；否则逐个槽位计算。
/// `compute` 遇错时须把该槽逻辑下标 min-store 进共享原子并提前返回，
/// 已完成前缀槽位保持 `Done(Ok(_))`，同块后缀保持 `Pending`。
fn run_dispatch_chunk<T, F>(
    compute: &F,
    view: super::phase::StepReadView<'_>,
    first_error: &AtomicUsize,
    counters: &DispatchCounters,
    start: usize,
    chunk: &mut [DispatchSlot<T>],
) where
    F: Fn(super::phase::StepReadView<'_>, usize, &mut [DispatchSlot<T>]) + Sync,
{
    counters.note_thread();
    let failed_at = first_error.load(Ordering::Relaxed);
    if start > failed_at {
        for slot in chunk.iter_mut() {
            *slot = DispatchSlot::Skipped;
        }
        counters.skipped.fetch_add(1, Ordering::Relaxed);
        return;
    }
    if failed_at != usize::MAX {
        counters.extra_work.fetch_add(1, Ordering::Relaxed);
    }
    counters.dispatched.fetch_add(1, Ordering::Relaxed);
    compute(view, start, chunk);
    if chunk
        .iter()
        .all(|slot| matches!(slot, DispatchSlot::Done(_)))
    {
        counters.completed.fetch_add(1, Ordering::Relaxed);
    }
}

impl ExecutionResources {
    fn start(config: ExecutionConfig) -> Result<Self, ExecutionInitError> {
        let auxiliaries = usize::try_from(config.worker_count().get() - 1)
            .map_err(|_| ExecutionInitError::ResourceReservationFailed)?;
        if auxiliaries == 0 {
            return Ok(Self::Caller);
        }
        let mut workers = WorkerJoins::default();
        workers
            .0
            .try_reserve_exact(auxiliaries)
            .map_err(|_| ExecutionInitError::ResourceReservationFailed)?;
        // spawn_handler 的登记持有每个真实 JoinHandle；构建失败也由同一 guard 结算。
        let pool = rayon_core::ThreadPoolBuilder::new()
            .num_threads(auxiliaries)
            .stack_size(WORKER_STACK_BYTES)
            .spawn_handler(|thread| {
                #[cfg(test)]
                if START_FAILURE.with(|fail| fail.get() == Some(thread.index())) {
                    return Err(std::io::Error::other("injected worker start failure"));
                }
                let worker = std::thread::Builder::new()
                    .name(format!("laneflow-{}", thread.index()))
                    .stack_size(WORKER_STACK_BYTES)
                    .spawn(move || {
                        #[cfg(test)]
                        let _activity = WorkerActivity::start();
                        thread.run();
                    })?;
                workers.0.push(worker);
                Ok(())
            })
            .build()
            .map_err(|_| ExecutionInitError::WorkerStartFailed)?;
        Ok(Self::Pool(PoolResources {
            pool,
            _workers: workers,
        }))
    }

    /// 协调调用线程计算首块，至多 N−1 个私有线程计算其余互斥输出。
    /// Rayon scope 会在传播任何 panic 前等待全部已分发任务。
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "#704 验证私有原语，#705 才接入交通独立计算")
    )]
    pub(crate) fn for_each_chunk<
        T: Send,
        F: Fn(super::phase::StepReadView<'_>, usize, &mut [T]) + Sync,
    >(
        &self,
        view: super::phase::StepReadView<'_>,
        output: &mut [T],
        chunk_size: usize,
        compute: F,
    ) {
        assert!(chunk_size > 0, "nonzero chunk size");
        match self {
            Self::Caller => {
                for (index, chunk) in output.chunks_mut(chunk_size).enumerate() {
                    compute(view, index, chunk);
                }
            }
            Self::Pool(resources) => resources.pool.in_place_scope(|scope| {
                let mut chunks = output.chunks_mut(chunk_size).enumerate();
                let first = chunks.next();
                for (index, chunk) in chunks {
                    let compute = &compute;
                    scope.spawn(move |_| compute(view, index, chunk));
                }
                if let Some((index, chunk)) = first {
                    compute(view, index, chunk);
                }
            }),
        }
    }

    /// 本次分发最多参与的线程数：调用线程加计池内辅助线程；`Caller` 为 1。
    pub(crate) fn dispatch_threads(&self) -> usize {
        match self {
            Self::Caller => 1,
            Self::Pool(resources) => resources.pool.current_num_threads().saturating_add(1),
        }
    }

    /// 可失败的保序分发：与 `for_each_chunk` 相同的互斥输出划分与完整 join，
    /// 但任务逐个槽位回报值或完整领域错误。`first_error` 由调用方以
    /// `usize::MAX` 初始化；任务遇错把该槽逻辑下标 min-store 进该原子，
    /// 整块起点晚于已错位置的输出整块标记 `Skipped` 不执行。错误不取消其他
    /// 已分发任务：Rayon scope 在传播 panic 前等待全部任务结束，领域错误也
    /// 等完整 join 后由调用方按逻辑顺序消费首错。协调调用线程执行首块。
    pub(crate) fn try_for_each_chunk<T, F>(
        &self,
        view: super::phase::StepReadView<'_>,
        output: &mut [DispatchSlot<T>],
        first_error: &AtomicUsize,
        chunk_size: usize,
        compute: F,
    ) -> DispatchStats
    where
        T: Send,
        F: Fn(super::phase::StepReadView<'_>, usize, &mut [DispatchSlot<T>]) + Sync,
    {
        assert!(chunk_size > 0, "nonzero chunk size");
        let counters = DispatchCounters::default();
        match self {
            Self::Caller => {
                for (chunk_index, chunk) in output.chunks_mut(chunk_size).enumerate() {
                    run_dispatch_chunk(
                        &compute,
                        view,
                        first_error,
                        &counters,
                        chunk_index * chunk_size,
                        chunk,
                    );
                }
            }
            Self::Pool(resources) => resources.pool.in_place_scope(|scope| {
                let compute = &compute;
                let counters = &counters;
                let mut chunks = output.chunks_mut(chunk_size).enumerate();
                let first = chunks.next();
                for (chunk_index, chunk) in chunks {
                    let start = chunk_index * chunk_size;
                    scope.spawn(move |_| {
                        run_dispatch_chunk(compute, view, first_error, counters, start, chunk);
                    });
                }
                if let Some((chunk_index, chunk)) = first {
                    run_dispatch_chunk(
                        compute,
                        view,
                        first_error,
                        counters,
                        chunk_index * chunk_size,
                        chunk,
                    );
                }
            }),
        }
        counters.stats()
    }
}

pub(crate) struct WorldExecution {
    config: ExecutionConfig,
    resources: ExecutionResources,
    pub(crate) active_plan: ExecutionPlan,
    attempt_epoch: u64,
    usable: bool,
}

pub(crate) enum ExecutionPreparationError {
    Init(ExecutionInitError),
    Plan(ExecutionPlanError),
}

impl WorldExecution {
    pub(crate) fn prepare(
        config: ExecutionConfig,
        state: &WorldState,
    ) -> Result<Self, ExecutionPreparationError> {
        config
            .validate_supported()
            .map_err(ExecutionPreparationError::Init)?;
        let plan = ExecutionPlan::prepare(state, config, state.binding.world_generation)
            .map_err(ExecutionPreparationError::Plan)?;
        Self::start(config, plan).map_err(ExecutionPreparationError::Init)
    }

    fn start(
        config: ExecutionConfig,
        active_plan: ExecutionPlan,
    ) -> Result<Self, ExecutionInitError> {
        let resources = ExecutionResources::start(config)?;
        Ok(Self {
            config,
            resources,
            active_plan,
            attempt_epoch: 0,
            usable: true,
        })
    }

    pub(crate) const fn config(&self) -> ExecutionConfig {
        self.config
    }

    pub(crate) const fn assert_usable(&self) {
        assert!(self.usable, "traffic world invalidated by execution panic");
    }

    pub(crate) fn run<R>(
        &mut self,
        state: &mut WorldState,
        operation: impl FnOnce(&mut WorldState, &ExecutionResources) -> R,
    ) -> R {
        self.assert_usable();
        // 先置为失效；只有作用域完整返回才恢复，包括可重试领域错误的返回。
        self.usable = false;
        self.attempt_epoch = self
            .attempt_epoch
            .checked_add(1)
            .expect("execution attempt epoch exhausted");
        match catch_unwind(AssertUnwindSafe(|| {
            self.active_plan.assert_binding(state);
            self.active_plan
                .refresh_workset(state.derived.active_order.len());
            operation(state, &self.resources)
        })) {
            Ok(result) => {
                self.usable = true;
                result
            }
            Err(payload) => resume_unwind(payload),
        }
    }

    #[cfg(test)]
    pub(crate) fn start_private(config: ExecutionConfig, state: &WorldState) -> Self {
        let plan = ExecutionPlan::prepare(state, config, state.binding.world_generation).unwrap();
        let execution = Self::start(config, plan).unwrap();
        // 计量与线程数断言从全部线程实际进入运行循环之后开始。
        if let ExecutionResources::Pool(resources) = &execution.resources {
            resources.pool.broadcast(|_| ());
        }
        execution
    }

    #[cfg(test)]
    pub(crate) fn thread_ids(&self) -> Vec<std::thread::ThreadId> {
        match &self.resources {
            ExecutionResources::Caller => Vec::new(),
            ExecutionResources::Pool(resources) => resources
                ._workers
                .0
                .iter()
                .map(|worker| worker.thread().id())
                .collect(),
        }
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static PLAN_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static START_FAILURE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
static LIVE_WORKERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
static STARTED_WORKERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
pub(crate) static RESOURCE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(test)]
pub(crate) fn worker_starts() -> usize {
    STARTED_WORKERS.load(std::sync::atomic::Ordering::SeqCst)
}
#[cfg(test)]
struct WorkerActivity;
#[cfg(test)]
impl WorkerActivity {
    fn start() -> Self {
        LIVE_WORKERS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        STARTED_WORKERS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self
    }
}
#[cfg(test)]
impl Drop for WorkerActivity {
    fn drop(&mut self) {
        LIVE_WORKERS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
pub(crate) fn with_plan_failure<R>(operation: impl FnOnce() -> R) -> R {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            PLAN_FAILURE.with(|flag| flag.set(self.0));
        }
    }
    let _reset = Reset(PLAN_FAILURE.with(|flag| flag.replace(true)));
    operation()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::cutover::tests::transaction_tests::{
        revision, source_for, world_with_vehicle,
    };
    use crate::{
        CutoverError, CutoverPreflightLimits, InstallError, LfcaOriginBinding, MigrationPolicyKind,
        NetworkRevisionCutoverDescriptor, SnapshotRestoreError, SnapshotRestoreLimits, TickInput,
        TrafficWorld,
    };
    use std::num::NonZeroU32;
    use std::sync::{
        Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    fn config(workers: u32) -> ExecutionConfig {
        ExecutionConfig::new(NonZeroU32::new(workers).unwrap())
    }

    #[test]
    fn partial_start_and_drop_join_real_threads_and_worker_one_stays_inline() {
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let started = STARTED_WORKERS.load(Ordering::SeqCst);
        drop(ExecutionResources::start(config(1)).unwrap());
        assert_eq!(STARTED_WORKERS.load(Ordering::SeqCst), started);
        START_FAILURE.with(|fail| fail.set(Some(2)));
        let failed = ExecutionResources::start(config(5));
        START_FAILURE.with(|fail| fail.set(None));
        assert!(matches!(failed, Err(ExecutionInitError::WorkerStartFailed)));
        assert_eq!(STARTED_WORKERS.load(Ordering::SeqCst) - started, 2);
        assert_eq!(LIVE_WORKERS.load(Ordering::SeqCst), 0);
        let resources = ExecutionResources::start(config(4)).unwrap();
        drop(resources);
        assert_eq!(STARTED_WORKERS.load(Ordering::SeqCst) - started, 5);
        assert_eq!(LIVE_WORKERS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn persistent_workers_borrow_real_views_and_return_exclusive_outputs() {
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let (mut world, _, _) = world_with_vehicle(true);
        world.execution = WorldExecution::start_private(config(4), &world.state);
        let ids = world.execution.thread_ids();
        let mut output = [0_u64; 4];
        for _ in 0..2 {
            let barrier = Barrier::new(4);
            let visited = Mutex::new(Vec::new());
            world.execution.run(&mut world.state, |state, resources| {
                resources.for_each_chunk(
                    state.read_view(),
                    &mut output,
                    1,
                    |view, index, chunk| {
                        barrier.wait();
                        let vehicle = view.committed.live_order[0];
                        chunk[0] = u64::from(view.vehicle_state(vehicle).unwrap().progress_mm)
                            + index as u64;
                        visited.lock().unwrap().push(std::thread::current().id());
                    },
                );
            });
            let visited: std::collections::HashSet<_> =
                visited.into_inner().unwrap().into_iter().collect();
            assert_eq!(visited.len(), 4);
            assert!(visited.contains(&std::thread::current().id()));
            assert!(ids.iter().all(|id| visited.contains(id)));
            assert_eq!(output, [1_000, 1_001, 1_002, 1_003]);
        }
        assert_eq!(world.execution.attempt_epoch, 2);
        assert_eq!(world.execution.thread_ids(), ids);
        drop(world);
        assert_eq!(LIVE_WORKERS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn panic_joins_all_tasks_invalidates_world_and_drop_joins_workers() {
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        for panic_index in [0, 2] {
            let (mut world, route, _) = world_with_vehicle(true);
            world.execution = WorldExecution::start_private(config(4), &world.state);
            let completed = AtomicUsize::new(0);
            let barrier = Barrier::new(4);
            let result = catch_unwind(AssertUnwindSafe(|| {
                world.execution.run(&mut world.state, |state, resources| {
                    let mut output = [0; 4];
                    resources.for_each_chunk(state.read_view(), &mut output, 1, |_, index, _| {
                        barrier.wait();
                        assert_ne!(index, panic_index, "injected execution panic");
                        completed.fetch_add(1, Ordering::SeqCst);
                    });
                });
            }));
            assert!(result.is_err());
            assert_eq!(completed.load(Ordering::SeqCst), 3);
            assert!(!world.execution.usable);
            assert!(catch_unwind(AssertUnwindSafe(|| world.step(TickInput::new(100)))).is_err());
            assert!(catch_unwind(AssertUnwindSafe(|| world.remove_route(route))).is_err());
            assert!(catch_unwind(AssertUnwindSafe(|| world.capture_snapshot())).is_err());
            assert!(catch_unwind(AssertUnwindSafe(|| world.world_binding())).is_err());
            assert!(catch_unwind(AssertUnwindSafe(|| world.execution_config())).is_err());
            drop(world);
            assert_eq!(LIVE_WORKERS.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn panic_drains_submitted_tasks_waiting_in_private_queue() {
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let (mut world, _, _) = world_with_vehicle(true);
        world.execution = WorldExecution::start_private(config(2), &world.state);
        let first_pair = Barrier::new(2);
        let release_on_unwind = Mutex::new(());
        let started = AtomicUsize::new(0);
        let auxiliary_started = AtomicUsize::new(0);
        let completed = AtomicUsize::new(0);
        let queued_at_panic = AtomicUsize::new(0);
        let mut output = [usize::MAX; 16];
        let result = catch_unwind(AssertUnwindSafe(|| {
            world.execution.run(&mut world.state, |state, resources| {
                resources.for_each_chunk(state.read_view(), &mut output, 1, |_, index, chunk| {
                    started.fetch_add(1, Ordering::SeqCst);
                    if index == 0 {
                        let _until_unwind = release_on_unwind.lock().unwrap();
                        first_pair.wait();
                        queued_at_panic
                            .store(16 - started.load(Ordering::SeqCst), Ordering::SeqCst);
                        panic!("queued-task panic");
                    }
                    if auxiliary_started.fetch_add(1, Ordering::SeqCst) == 0 {
                        first_pair.wait();
                        // 仅首个辅助任务参与握手；调用线程 unwind 才释放其余队列。
                        drop(
                            release_on_unwind
                                .lock()
                                .unwrap_or_else(|error| error.into_inner()),
                        );
                    }
                    chunk[0] = index;
                    completed.fetch_add(1, Ordering::SeqCst);
                });
            });
        }));
        assert!(result.is_err());
        assert_eq!(queued_at_panic.load(Ordering::SeqCst), 14);
        assert_eq!(started.load(Ordering::SeqCst), output.len());
        assert_eq!(completed.load(Ordering::SeqCst), output.len() - 1);
        assert_eq!(output[0], usize::MAX);
        assert!(
            output
                .iter()
                .enumerate()
                .skip(1)
                .all(|(index, value)| *value == index)
        );
        assert!(catch_unwind(AssertUnwindSafe(|| world.execution_config())).is_err());
        let settled_output = output;
        drop(world);
        assert_eq!(LIVE_WORKERS.load(Ordering::SeqCst), 0);
        assert_eq!(output, settled_output);
        assert_eq!(completed.load(Ordering::SeqCst), 15);
    }

    #[test]
    fn workset_ranges_cover_empty_uneven_growing_and_shrinking_inputs() {
        let (world, _, _) = world_with_vehicle(true);
        for workers in [1, 2, 3, 4, 8] {
            let mut plan =
                ExecutionPlan::prepare(&world.state, config(workers), world.world_generation())
                    .unwrap();
            for count in [0, 1, 2, 3, 7, 8, 17, 4, 1, 0, usize::MAX, 0] {
                plan.refresh_workset(count);
                assert_eq!(plan.ranges.len(), workers as usize);
                let mut end = 0;
                let mut smallest = usize::MAX;
                let mut largest = 0;
                for range in &plan.ranges {
                    assert_eq!(range.start, end);
                    assert!(range.start <= range.end && range.end <= count);
                    let length = range.end - range.start;
                    smallest = smallest.min(length);
                    largest = largest.max(length);
                    end = range.end;
                }
                assert_eq!(end, count);
                assert!(largest - smallest <= 1);
                if count <= 17 {
                    let mut visits = vec![0; count];
                    for range in &plan.ranges {
                        for index in range.clone() {
                            *visits.get_mut(index).unwrap() += 1;
                        }
                    }
                    assert!(visits.iter().all(|count| *count == 1));
                }
            }
        }
    }

    #[test]
    fn unchanged_workset_length_reads_current_vehicle_order() {
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let (mut world, route, first) = world_with_vehicle(true);
        let edge = world.route_edges(route).unwrap()[0];
        let second_progress = world.traffic().lane_lengths_millimetres()[edge.index()];
        let second = world
            .spawn_vehicle(crate::VehicleSpawnInput::new(
                laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                second_progress,
                0,
            ))
            .unwrap();
        world.execution = WorldExecution::start_private(config(2), &world.state);
        let ranges = world.execution.active_plan.ranges.clone();
        for expected in [
            [Some((first, 1_000)), Some((second, second_progress))],
            [Some((second, second_progress)), Some((first, 1_000))],
        ] {
            let mut output = [None; 2];
            world.execution.run(&mut world.state, |state, resources| {
                resources.for_each_chunk(
                    state.read_view(),
                    &mut output,
                    1,
                    |view, index, chunk| {
                        let handle = view.derived.active_order[index];
                        chunk[0] = Some((handle, view.vehicle_state(handle).unwrap().progress_mm));
                    },
                );
            });
            assert_eq!(world.execution.active_plan.ranges, ranges);
            assert_eq!(output, expected);
            world.state.committed.live_order.reverse();
            world.state.rebuild_active_order();
        }
        drop(world);
        assert_eq!(LIVE_WORKERS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn retry_on_same_committed_tick_uses_a_new_attempt_and_keeps_world_usable() {
        let (mut world, _, _) = world_with_vehicle(true);
        let tick = world.tick_index();
        let before = crate::deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap();
        for _ in 0..2 {
            assert!(matches!(
                world.step(TickInput::new(101)),
                Err(crate::StepError::DeltaMismatch { .. })
            ));
            assert_eq!(world.tick_index(), tick);
            assert_eq!(
                before,
                crate::deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap()
            );
        }
        world.step(TickInput::new(100)).unwrap();
        assert_eq!(world.tick_index(), tick + 1);
        assert_eq!(world.execution.attempt_epoch, 3);
    }

    #[test]
    fn plan_failure_is_after_traffic_validation_and_before_resource_start() {
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let (world, _, _) = world_with_vehicle(true);
        let started = STARTED_WORKERS.load(Ordering::SeqCst);
        let install = |execution| {
            TrafficWorld::install(
                world.revision(),
                world.config(),
                execution,
                world.committed_source().clone(),
                world.world_id(),
                world.policy_selection(),
            )
        };
        assert!(matches!(
            with_plan_failure(|| install(config(1))),
            Err(InstallError::ExecutionPlan(
                ExecutionPlanError::ReservationFailed
            ))
        ));
        // 能力校验先于计划准备：超上限数量仍被拒，不触发计划注入。
        assert!(matches!(
            with_plan_failure(|| install(config(17))),
            Err(InstallError::ExecutionInit(
                ExecutionInitError::UnsupportedWorkerCount {
                    requested: 17,
                    max_supported: 16,
                }
            ))
        ));
        let bytes = crate::encode_lfrs(&world.capture_snapshot().unwrap());
        let restore = |bytes: &[u8]| {
            crate::restore_lfrs(
                bytes,
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                config(1),
                SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
            )
        };
        assert!(matches!(
            with_plan_failure(|| restore(&bytes)),
            Err(SnapshotRestoreError::ExecutionPlan(
                ExecutionPlanError::ReservationFailed
            ))
        ));
        assert!(!matches!(
            with_plan_failure(|| restore(&bytes[..8])),
            Err(SnapshotRestoreError::ExecutionPlan(_))
        ));
        let restored = restore(&bytes).unwrap();
        restored
            .world()
            .execution
            .active_plan
            .assert_binding(&restored.world().state);
        assert_eq!(restored.world().execution.active_plan.ranges.len(), 1);
        assert_eq!(restored.world().execution.active_plan.ranges[0], 0..1);
        assert_eq!(STARTED_WORKERS.load(Ordering::SeqCst), started);
    }

    #[test]
    fn same_revision_plan_failure_preserves_root_plan_resources_and_then_promotes() {
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let (mut world, _, _) = world_with_vehicle(true);
        world.execution = WorldExecution::start_private(config(3), &world.state);
        let ids = world.execution.thread_ids();
        let before_root = world.revision();
        let before_generation = world.world_generation();
        let before = crate::deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap();
        let target = revision(false);
        let descriptor = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(*before_root.canonical_origin()),
            LfcaOriginBinding::from_canonical_origin(*target.canonical_origin()),
            None,
            MigrationPolicyKind::SameRevisionRestore,
            world.world_binding(),
        );
        let source = source_for(*target.canonical_origin(), "fixture://execution-plan");
        let result = with_plan_failure(|| {
            world.cutover_same_revision(
                Arc::clone(&target),
                source.clone(),
                &descriptor,
                &CutoverPreflightLimits::new(1_048_576),
            )
        });
        assert_eq!(
            result.unwrap_err(),
            CutoverError::ExecutionPlan(ExecutionPlanError::ReservationFailed)
        );
        assert!(Arc::ptr_eq(&world.revision(), &before_root));
        assert_eq!(world.world_generation(), before_generation);
        assert_eq!(
            before,
            crate::deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap()
        );
        world.execution.active_plan.assert_binding(&world.state);
        let _events = world
            .cutover_same_revision(
                Arc::clone(&target),
                source,
                &descriptor,
                &CutoverPreflightLimits::new(1_048_576),
            )
            .unwrap();
        assert_eq!(world.execution.thread_ids(), ids);
        world.execution.active_plan.assert_binding(&world.state);
        world.step(TickInput::new(100)).unwrap();
        assert!(Arc::ptr_eq(&world.execution.active_plan.root, &target));
    }
}
