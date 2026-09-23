use laneflow_static_contract::{LaneEdgeOrdinal, MAX_VEHICLE_LENGTH_MM, VehicleProfileOrdinal};
use laneflow_static_network::{BoundedDistance, VehicleProfileView};

use crate::admin::migration_journal::VehicleDelta;
use crate::kernel::occupancy::LeaderQueryHorizon;
#[cfg(test)]
use crate::kernel::tables::occupancy_front_gap;
use crate::kernel::tables::{
    CompiledRoute, distance_to_occurrence_progress, distance_to_occurrence_start,
    remaining_to_route_end,
};
use crate::kernel::units::{ceil_mm, round_mm, round_um};
use crate::{
    ParkingArrivalObservation, ParkingBinding, ParkingReservation, StepError, StepOutcome,
    TickInput, VehicleState, VehicleStatus,
};

/// 整数毫米合同下覆盖 `s0` 边界舍入的专用容差。跟车前视公式里的
/// `minimum_gap_tolerance`。
const MINIMUM_GAP_TOLERANCE_MM: u32 = 1;

/// 按本拍拍初 Active 顺序保存的私有输入与预览；完整句柄防止错配槽位。
#[derive(Clone, Copy, Debug)]
pub(crate) struct MotionCacheEntry {
    pub(crate) vehicle: crate::VehicleHandle,
    pub(crate) update_sequence: usize,
    pub(crate) horizon: Option<LeaderQueryHorizon>,
    pub(crate) preview: Option<MotionPreview>,
}

/// 同一拍初状态上的完整运动预览；在新增停止约束后消费前复核。
#[derive(Clone, Copy, Debug)]
pub(crate) struct MotionPreview {
    pub(crate) next: VehicleState,
    waiting_stop: Option<crate::kernel::waiting::WaitingStopConstraint>,
    bounds: MotionBounds,
}

/// 新鲜摆放用来对照求解器本拍硬截断的预测。不是步进结果。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PlacementMotion {
    pub(crate) next_speed_mm_s: u32,
    pub(crate) committed_travel_mm: u32,
    pub(crate) hard_clamped: bool,
}

/// 新鲜摆放看到的已占用冲突区。空区不是停车点。
#[derive(Clone, Copy, Debug)]
enum OccupiedConflictPreview {
    Clear,
    Stop(crate::kernel::waiting::WaitingStopConstraint),
    Unprovable,
}

/// 由共同运动内核产生的复用证明；缺失证明时只能复用完全相同的约束。
#[derive(Clone, Copy, Debug)]
enum MotionBounds {
    Unknown,
    HardStopped,
    Travel {
        meters: f32,
        proposed_mm: u64,
        committed_mm: u32,
        exhausted: bool,
    },
}

/// P2 第一遍逐车独立预览的暂存输出。协调器按 Active 顺序规范消费；
/// 需要 `next` 时从 `preview` 提取（`preview.next`），不复制存储。
#[derive(Clone, Copy, Debug)]
pub(crate) struct WaitingPreviewEntry {
    pub(crate) horizon: Option<LeaderQueryHorizon>,
    pub(crate) preview: Option<MotionPreview>,
}

impl MotionPreview {
    pub(crate) fn with_waiting_stop(
        mut self,
        waiting_stop: Option<crate::kernel::waiting::WaitingStopConstraint>,
    ) -> Option<Self> {
        if self.waiting_stop == waiting_stop {
            return Some(self);
        }
        // 只能证明新增约束不改变结果；移除或替换已经参与计算的约束必须重算。
        if self.waiting_stop.is_some() {
            return None;
        }
        let stop = waiting_stop?;
        if !self.unaffected_by(stop) {
            return None;
        }
        self.waiting_stop = waiting_stop;
        Some(self)
    }

    fn reuse(
        self,
        waiting_stop: Option<crate::kernel::waiting::WaitingStopConstraint>,
        conflict_stop: Option<crate::kernel::waiting::WaitingStopConstraint>,
    ) -> Option<VehicleState> {
        let preview = self.with_waiting_stop(waiting_stop)?;
        let Some(stop) = conflict_stop else {
            return Some(preview.next);
        };
        if Some(stop) == waiting_stop {
            return Some(preview.next);
        }
        preview.unaffected_by(stop).then_some(preview.next)
    }

    fn unaffected_by(self, stop: crate::kernel::waiting::WaitingStopConstraint) -> bool {
        let beyond_motion = match self.bounds {
            MotionBounds::Unknown => false,
            // 原 hard_room 已为零；额外停止约束仍走相同的零位移提前返回。
            MotionBounds::HardStopped => return true,
            MotionBounds::Travel {
                meters,
                proposed_mm,
                ..
            } => match stop.distance {
                BoundedDistance::Finite(mm) => {
                    // SI clamp 不变，且新整数硬边界严格在含 carry/舍入的提案外，
                    // 因而不会改变 exhausted、余量或速度。不能只比较整数位置。
                    u64::from(mm) > proposed_mm && si_meters(mm) >= meters
                }
                BoundedDistance::BeyondFinite => true,
            },
        };
        beyond_motion && self.next.route_edge_index <= stop.hop
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum StepFailpoint {
    AfterGrants,
    AfterTransitions,
    AllocationAfterGrants,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BarrierQueryCounts {
    pub(crate) conflict_scans: u32,
    pub(crate) waiting_entry_scans: u32,
    pub(crate) signal_gates: u32,
    pub(crate) hard_room_permissions: u32,
}

#[cfg(test)]
std::thread_local! {
    pub(super) static STEP_FAILPOINT: std::cell::Cell<Option<StepFailpoint>> = const { std::cell::Cell::new(None) };
    static MOTION_CACHE_LIMIT: std::cell::Cell<usize> = const { std::cell::Cell::new(usize::MAX) };
    static HORIZON_CALCULATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static MOTION_CALCULATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static MOTION_CACHE_HITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static MOTION_CACHE_MISSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static BARRIER_QUERIES: std::cell::Cell<BarrierQueryCounts> = const {
        std::cell::Cell::new(BarrierQueryCounts {
            conflict_scans: 0,
            waiting_entry_scans: 0,
            signal_gates: 0,
            hard_room_permissions: 0,
        })
    };
}

#[cfg(test)]
pub(crate) fn reset_barrier_query_counts() {
    BARRIER_QUERIES.with(|counts| counts.set(BarrierQueryCounts::default()));
}

#[cfg(test)]
pub(crate) fn barrier_query_counts() -> BarrierQueryCounts {
    BARRIER_QUERIES.with(std::cell::Cell::get)
}

#[cfg(test)]
fn note_barrier_query(update: impl FnOnce(&mut BarrierQueryCounts)) {
    BARRIER_QUERIES.with(|counts| {
        let mut current = counts.get();
        update(&mut current);
        counts.set(current);
    });
}

#[cfg(test)]
pub(crate) fn motion_cache_limit() -> usize {
    MOTION_CACHE_LIMIT.get()
}

/// 测试专用：MotionPreview 缓存复用/重算计数；机制测量探针读取，不改变语义。
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct MotionCacheUse {
    pub(crate) hits: usize,
    pub(crate) misses: usize,
}

#[cfg(test)]
pub(crate) fn motion_cache_use() -> MotionCacheUse {
    MotionCacheUse {
        hits: MOTION_CACHE_HITS.with(std::cell::Cell::get),
        misses: MOTION_CACHE_MISSES.with(std::cell::Cell::get),
    }
}

#[cfg(test)]
fn note_motion_cache_use(hit: bool) {
    if hit {
        MOTION_CACHE_HITS.with(|value| value.set(value.get() + 1));
    } else {
        MOTION_CACHE_MISSES.with(|value| value.set(value.get() + 1));
    }
}

/// P5 分发阈值：Active 投影低于该值时融合执行逐车运动；初版保守选择
///（与 P2 同值、独立常量），待增量 E 机制证据登记后校准。只决定本阶段
/// 谁执行，不改变语义、首错、容量和输出。
const MOTION_DISPATCH_MIN_ACTIVE: usize = 1_024;

// W5-B：P7（commit）分配窗口的 cfg(test) 诊断挂点。生产调用链与签名
// 不变；发布构建零开销零语义变化。Drop 守卫保证单出口正确计量；
// 计数器的读取/存储无堆分配（TLS Cell + 全局计数器读），测试代码的
// 断言与输出全部在窗口外执行。
#[cfg(test)]
thread_local! {
    static COMMIT_ALLOC_WINDOW: core::cell::Cell<Option<(u64, u64)>> =
        const { core::cell::Cell::new(None) };
}

#[cfg(test)]
struct CommitAllocWindowGuard {
    allocations: u64,
    reallocations: u64,
}

#[cfg(test)]
impl Drop for CommitAllocWindowGuard {
    fn drop(&mut self) {
        let stats = stats_alloc::INSTRUMENTED_SYSTEM.stats();
        COMMIT_ALLOC_WINDOW.with(|window| {
            window.set(Some((
                stats
                    .allocations
                    .try_into()
                    .unwrap_or(u64::MAX)
                    .wrapping_sub(self.allocations),
                stats
                    .reallocations
                    .try_into()
                    .unwrap_or(u64::MAX)
                    .wrapping_sub(self.reallocations),
            )));
        });
    }
}

/// 测试专用：读取最近一次 commit 的（allocations, reallocations）窗口。
#[cfg(test)]
pub(crate) fn last_commit_alloc_window() -> Option<(u64, u64)> {
    COMMIT_ALLOC_WINDOW.with(core::cell::Cell::get)
}

#[cfg(test)]
fn begin_commit_alloc_window() -> CommitAllocWindowGuard {
    let stats = stats_alloc::INSTRUMENTED_SYSTEM.stats();
    CommitAllocWindowGuard {
        allocations: stats.allocations.try_into().unwrap_or(u64::MAX),
        reallocations: stats.reallocations.try_into().unwrap_or(u64::MAX),
    }
}

/// 测试专用：P5 本阶段谁执行的计数证据（与 P2 的 WaitingPreviewPathCounts
/// 相互独立，按阶段自己的口径断言；融合/分发/回退互斥）。
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct MotionPathCounts {
    pub(crate) dispatched: usize,
    pub(crate) fused: usize,
    pub(crate) slot_fallback: usize,
}

#[cfg(test)]
pub(crate) fn motion_path_counts() -> MotionPathCounts {
    MOTION_PATH_COUNTS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn count_motion_path(update: impl FnOnce(&mut MotionPathCounts)) {
    MOTION_PATH_COUNTS.with(|counts| {
        let mut value = counts.get();
        update(&mut value);
        counts.set(value);
    });
}

#[cfg(test)]
thread_local! {
    static MOTION_PATH_COUNTS: std::cell::Cell<MotionPathCounts> = const { std::cell::Cell::new(MotionPathCounts { dispatched: 0, fused: 0, slot_fallback: 0 }) };
    /// 生产分发阈值为保守 1_024；小场景测试经该守卫强制 P5 真实分发。
    static MOTION_FORCE_DISPATCH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// 组合矩阵融合侧入口：强制 P5 保持融合（fuse 优先于 force）。
    static MOTION_FORCE_FUSE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// join 完成后把槽位改写为缺失的注入位置（完成前沿检出测试）。
    static MOTION_SLOT_GAP: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// 块级计数诊断开关：开启时分发路径按块记录并汇总四计数；
    /// 关闭时热态分发无 LaneFlow 自有分配（分配证据预算口径）。
    static MOTION_DIAGNOSTICS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static LAST_MOTION_DISPATCH_STATS: std::cell::Cell<Option<crate::kernel::execution::DispatchStats>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn motion_dispatch_forced() -> bool {
    MOTION_FORCE_DISPATCH.with(std::cell::Cell::get)
}

#[cfg(test)]
fn motion_dispatch_fuse_forced() -> bool {
    MOTION_FORCE_FUSE.with(std::cell::Cell::get)
}

#[cfg(not(test))]
fn motion_dispatch_fuse_forced() -> bool {
    false
}

#[cfg(test)]
pub(crate) struct ForceMotionFuseGuard(bool);

#[cfg(test)]
impl Drop for ForceMotionFuseGuard {
    fn drop(&mut self) {
        MOTION_FORCE_FUSE.with(|forced| forced.set(self.0));
    }
}

/// 测试专用：本拍起强制 P5 保持融合（#706 增量 E 组合矩阵融合侧入口，
/// fuse 优先于 force），返回复位守卫。
#[cfg(test)]
pub(crate) fn force_motion_fuse() -> ForceMotionFuseGuard {
    ForceMotionFuseGuard(MOTION_FORCE_FUSE.with(|forced| forced.replace(true)))
}

#[cfg(test)]
pub(crate) struct ForceMotionDispatchGuard(bool);

#[cfg(test)]
impl Drop for ForceMotionDispatchGuard {
    fn drop(&mut self) {
        MOTION_FORCE_DISPATCH.with(|forced| forced.set(self.0));
    }
}

/// 测试专用：本拍起强制 P5 真实分发（工作集非空时），返回复位守卫。
#[cfg(test)]
pub(crate) fn force_motion_dispatch() -> ForceMotionDispatchGuard {
    ForceMotionDispatchGuard(MOTION_FORCE_DISPATCH.with(|forced| forced.replace(true)))
}

/// 测试专用：最近一次 P5 分发的调度统计（与 P2 的统计槽位按阶段分离）。
#[cfg(test)]
pub(crate) fn last_motion_dispatch_stats() -> Option<crate::kernel::execution::DispatchStats> {
    LAST_MOTION_DISPATCH_STATS.with(std::cell::Cell::get)
}

/// 块级计数诊断开关的复位守卫。
#[cfg(test)]
pub(crate) struct MotionDiagnosticsGuard(bool);

#[cfg(test)]
impl Drop for MotionDiagnosticsGuard {
    fn drop(&mut self) {
        MOTION_DIAGNOSTICS.with(|flag| flag.set(self.0));
    }
}

/// 测试专用：开启 P5 分发路径的块级计数诊断（计数对拍类测试使用；
/// 分配证据测试保持关闭以守住零 LaneFlow 自有分配预算）。
#[cfg(test)]
pub(crate) fn enable_motion_diagnostics() -> MotionDiagnosticsGuard {
    MotionDiagnosticsGuard(MOTION_DIAGNOSTICS.with(|flag| flag.replace(true)))
}

/// 测试专用：horizon/运动内核重算计数（分发路径经块级记录汇总后的总值）。
#[cfg(test)]
pub(crate) fn motion_diagnostic_counts() -> (usize, usize) {
    (
        HORIZON_CALCULATIONS.with(std::cell::Cell::get),
        MOTION_CALCULATIONS.with(std::cell::Cell::get),
    )
}

/// 线程本地四计数快照；分发 join 后按块级记录汇总回协调器线程。
#[cfg(test)]
#[derive(Clone, Copy, Default)]
struct MotionTlsSnapshot {
    horizon: usize,
    motion: usize,
    hits: usize,
    misses: usize,
}

#[cfg(test)]
fn motion_tls_snapshot() -> MotionTlsSnapshot {
    MotionTlsSnapshot {
        horizon: HORIZON_CALCULATIONS.with(std::cell::Cell::get),
        motion: MOTION_CALCULATIONS.with(std::cell::Cell::get),
        hits: MOTION_CACHE_HITS.with(std::cell::Cell::get),
        misses: MOTION_CACHE_MISSES.with(std::cell::Cell::get),
    }
}

/// 块级诊断记录：一个执行块的四计数增量（任务写本块独占槽，join 后
/// 由协调器汇总进线程本地计数器，使分发路径读到的总值与融合一致）。
#[cfg(test)]
#[derive(Default)]
struct MotionWorkChunkRecord {
    horizon: std::sync::atomic::AtomicU64,
    motion: std::sync::atomic::AtomicU64,
    hits: std::sync::atomic::AtomicU64,
    misses: std::sync::atomic::AtomicU64,
}

#[cfg(test)]
impl MotionWorkChunkRecord {
    fn store_deltas(&self, before: MotionTlsSnapshot) {
        use std::sync::atomic::Ordering;
        let after = motion_tls_snapshot();
        self.horizon.store(
            after.horizon.wrapping_sub(before.horizon) as u64,
            Ordering::Relaxed,
        );
        self.motion.store(
            after.motion.wrapping_sub(before.motion) as u64,
            Ordering::Relaxed,
        );
        self.hits.store(
            after.hits.wrapping_sub(before.hits) as u64,
            Ordering::Relaxed,
        );
        self.misses.store(
            after.misses.wrapping_sub(before.misses) as u64,
            Ordering::Relaxed,
        );
    }
}

/// 汇总：协调器线程计数器 = 分发前基线 + 全部块增量（调用线程自己的块
/// 增量经记录回灌，不重复计）。
#[cfg(test)]
fn aggregate_motion_tls(baseline: MotionTlsSnapshot, records: &[MotionWorkChunkRecord]) {
    use std::sync::atomic::Ordering;
    let mut sums = MotionTlsSnapshot::default();
    for record in records {
        sums.horizon += record.horizon.load(Ordering::Relaxed) as usize;
        sums.motion += record.motion.load(Ordering::Relaxed) as usize;
        sums.hits += record.hits.load(Ordering::Relaxed) as usize;
        sums.misses += record.misses.load(Ordering::Relaxed) as usize;
    }
    HORIZON_CALCULATIONS.with(|v| v.set(baseline.horizon + sums.horizon));
    MOTION_CALCULATIONS.with(|v| v.set(baseline.motion + sums.motion));
    MOTION_CACHE_HITS.with(|v| v.set(baseline.hits + sums.hits));
    MOTION_CACHE_MISSES.with(|v| v.set(baseline.misses + sums.misses));
}

/// P5 任务侧错误/失败注入（进程级原子量，按世界身份 + Active 紧凑位置
/// 武装；与 P2 的 preview_injection 相互独立，P2 融合首遍不会消费）。
#[cfg(test)]
mod motion_injection {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    pub(super) const DISABLED_WORLD: u64 = u64::MAX;

    pub(super) static NONFINITE_WORLD: AtomicU64 = AtomicU64::new(DISABLED_WORLD);
    pub(super) static NONFINITE_POSITIONS: AtomicU64 = AtomicU64::new(0);
    pub(super) static ARRIVAL_RESERVE_FAILURE: AtomicBool = AtomicBool::new(false);
    pub(super) static INPUT_RESERVE_FAILURE: AtomicBool = AtomicBool::new(false);
    pub(super) static SLOT_RESERVE_FAILURE: AtomicBool = AtomicBool::new(false);

    pub(super) fn position_mask(positions: &[usize]) -> u64 {
        positions.iter().fold(0_u64, |mask, position| {
            assert!(
                *position < u64::BITS as usize,
                "motion injection position fits mask"
            );
            mask | (1_u64 << position)
        })
    }

    pub(super) fn nonfinite_injected(world_id: u64, active_position: usize) -> bool {
        NONFINITE_WORLD.load(Ordering::SeqCst) == world_id
            && active_position < u64::BITS as usize
            && NONFINITE_POSITIONS.load(Ordering::SeqCst) & (1_u64 << active_position) != 0
    }

    pub(super) fn arrival_reserve_injected() -> bool {
        ARRIVAL_RESERVE_FAILURE.load(Ordering::SeqCst)
    }

    pub(super) fn input_reserve_injected() -> bool {
        INPUT_RESERVE_FAILURE.load(Ordering::SeqCst)
    }

    pub(super) fn slot_reserve_injected() -> bool {
        SLOT_RESERVE_FAILURE.load(Ordering::SeqCst)
    }
}

/// 只恢复 NonFinite 两个字段的复位守卫：与到达/预留类注入可任意组合
///（全量快照恢复会被后创建的兄弟注入覆盖）。
#[cfg(test)]
pub(crate) struct MotionNonfiniteGuard(u64, u64);

#[cfg(test)]
impl Drop for MotionNonfiniteGuard {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        motion_injection::NONFINITE_WORLD.store(self.0, Ordering::SeqCst);
        motion_injection::NONFINITE_POSITIONS.store(self.1, Ordering::SeqCst);
    }
}

/// 测试专用：按 Active 紧凑位置武装 P5 逐车 NonFiniteMotion 注入。
#[cfg(test)]
pub(crate) fn inject_motion_nonfinite(world_id: u64, positions: &[usize]) -> MotionNonfiniteGuard {
    use std::sync::atomic::Ordering;
    MotionNonfiniteGuard(
        motion_injection::NONFINITE_WORLD.swap(world_id, Ordering::SeqCst),
        motion_injection::NONFINITE_POSITIONS
            .swap(motion_injection::position_mask(positions), Ordering::SeqCst),
    )
}

/// 原子布尔注入的复位守卫。
#[cfg(test)]
pub(crate) struct MotionBoolGuard(&'static std::sync::atomic::AtomicBool, bool);

#[cfg(test)]
impl Drop for MotionBoolGuard {
    fn drop(&mut self) {
        self.0.store(self.1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
fn swap_motion_flag(flag: &'static std::sync::atomic::AtomicBool) -> MotionBoolGuard {
    MotionBoolGuard(flag, flag.swap(true, std::sync::atomic::Ordering::SeqCst))
}

/// 测试专用：下一次到达观察真实预留强制失败。
#[cfg(test)]
pub(crate) fn fail_motion_arrival_reserve() -> MotionBoolGuard {
    swap_motion_flag(&motion_injection::ARRIVAL_RESERVE_FAILURE)
}

/// 测试专用：下一次输入发现预留强制失败（冷态/增长回退测试）。
#[cfg(test)]
pub(crate) fn fail_motion_input_reserve() -> MotionBoolGuard {
    swap_motion_flag(&motion_injection::INPUT_RESERVE_FAILURE)
}

/// 测试专用：下一次结果槽位预留强制失败（冷态/增长回退测试）。
#[cfg(test)]
pub(crate) fn fail_motion_slot_reserve() -> MotionBoolGuard {
    swap_motion_flag(&motion_injection::SLOT_RESERVE_FAILURE)
}

#[cfg(test)]
pub(crate) struct MotionSlotGapGuard(Option<usize>);

#[cfg(test)]
impl Drop for MotionSlotGapGuard {
    fn drop(&mut self) {
        MOTION_SLOT_GAP.with(|gap| gap.set(self.0));
    }
}

/// 测试专用：join 完成后把指定 Active 位置的槽位改写为 `Pending`，
/// 验证完成前沿不变量在首错之前的检出。
#[cfg(test)]
pub(crate) fn drop_motion_slot_at(position: usize) -> MotionSlotGapGuard {
    MotionSlotGapGuard(MOTION_SLOT_GAP.with(|gap| gap.replace(Some(position))))
}

/// 到达观察的规范消费：真实 `try_reserve(1)` + 追加（含注入失败面）。
fn push_parking_arrival(
    parking_arrivals: &mut Vec<ParkingArrivalObservation>,
    arrival: ParkingArrivalObservation,
) -> Result<(), StepError> {
    #[cfg(test)]
    if motion_injection::arrival_reserve_injected()
        && parking_arrivals.len() == parking_arrivals.capacity()
    {
        // R4：注入仅在真实必要增长时触发（余量足够不得伪造预留失败）。
        return Err(StepError::ParkingObservationAllocFailed);
    }
    parking_arrivals
        .try_reserve(1)
        .map_err(|_| StepError::ParkingObservationAllocFailed)?;
    parking_arrivals.push(arrival);
    Ok(())
}

#[cfg(test)]
fn injected_step_failure(point: StepFailpoint) -> Result<(), StepError> {
    STEP_FAILPOINT.with(|failpoint| {
        if point == StepFailpoint::AfterGrants
            && failpoint.get() == Some(StepFailpoint::AllocationAfterGrants)
        {
            failpoint.set(None);
            crate::kernel::conflict::set_allocation_failpoint(Some(0));
            return Ok(());
        }
        if failpoint.get() == Some(point) {
            failpoint.set(None);
            Err(StepError::ParkingObservationAllocFailed)
        } else {
            Ok(())
        }
    })
}

#[cfg(test)]
mod transaction_tests {
    use super::*;
    use crate::admin::cutover_migration::tests::{conflict_scale_revision, conflict_scale_world};

    struct CacheLimitGuard(usize);

    impl CacheLimitGuard {
        fn set(limit: usize) -> Self {
            Self(MOTION_CACHE_LIMIT.replace(limit))
        }
    }

    impl Drop for CacheLimitGuard {
        fn drop(&mut self) {
            MOTION_CACHE_LIMIT.set(self.0);
        }
    }

    #[test]
    fn same_tick_cache_matches_full_calculation_with_zero_partial_and_full_capacity() {
        let mut cached = crate::kernel::waiting::tests::multi_gate_world(8);
        let mut partial = crate::kernel::waiting::tests::multi_gate_world(8);
        let mut uncached = crate::kernel::waiting::tests::multi_gate_world(8);
        for world in [&mut cached, &mut partial, &mut uncached] {
            // 槽位复用打乱 handle 次序；中间车辆完成后 live 与 Active 序号不同。
            let middle = world.live_vehicles()[3];
            let old = *world.state.vehicle_state(middle).unwrap();
            world.despawn_vehicle(middle).unwrap();
            world
                .spawn_vehicle(crate::VehicleSpawnInput::new(
                    old.profile,
                    old.route,
                    2,
                    20_000,
                    0,
                ))
                .unwrap();
            let first = world.live_vehicles()[0];
            let old = *world.state.vehicle_state(first).unwrap();
            world.despawn_vehicle(first).unwrap();
            world
                .spawn_vehicle(crate::VehicleSpawnInput::new(
                    old.profile,
                    old.route,
                    old.route_edge_index,
                    old.progress_mm,
                    old.speed_mm_s,
                ))
                .unwrap();
        }
        let mut work = [(0, 0); 3];
        for _ in 0..64 {
            for (index, (world, limit)) in [
                (&mut cached, usize::MAX),
                (&mut partial, 3),
                (&mut uncached, 0),
            ]
            .into_iter()
            .enumerate()
            {
                let _guard = CacheLimitGuard::set(limit);
                HORIZON_CALCULATIONS.set(0);
                MOTION_CALCULATIONS.set(0);
                world.step(TickInput::new(100)).unwrap();
                work[index].0 += HORIZON_CALCULATIONS.get();
                work[index].1 += MOTION_CALCULATIONS.get();
                assert!(world.state.workspace.motion_cache.is_empty());
            }
            for world in [&cached, &partial] {
                assert_eq!(
                    world.capture_snapshot().unwrap(),
                    uncached.capture_snapshot().unwrap()
                );
                assert_eq!(
                    world.latest_transition_events(),
                    uncached.latest_transition_events()
                );
                assert_eq!(
                    world.latest_waiting_decisions(),
                    uncached.latest_waiting_decisions()
                );
                assert_eq!(
                    world.latest_conflict_decisions(),
                    uncached.latest_conflict_decisions()
                );
            }
        }
        assert!(
            work[0].0 < work[1].0 && work[1].0 < work[2].0,
            "horizon work: {work:?}"
        );
        assert!(
            work[0].1 < work[1].1 && work[1].1 < work[2].1,
            "motion work: {work:?}"
        );
        eprintln!(
            "same-tick work (horizon, motion), full/partial/none: {work:?}; entry_bytes={} preview_bytes={}",
            std::mem::size_of::<MotionCacheEntry>(),
            std::mem::size_of::<MotionPreview>()
        );
    }

    #[test]
    fn wrong_generation_cache_entries_cannot_supply_horizon_or_motion() {
        let mut world = crate::kernel::waiting::tests::multi_gate_world(2);
        let mut reference = crate::kernel::waiting::tests::multi_gate_world(2);
        for world in [&mut world, &mut reference] {
            world.state.rebuild_occupancy_index().unwrap();
            world.state.prepare_waiting_step(0.1).unwrap();
        }
        reference.state.workspace.motion_cache.clear();
        for entry in &mut world.state.workspace.motion_cache {
            entry.vehicle =
                crate::VehicleHandle::new(entry.vehicle.index(), entry.vehicle.generation() + 1);
            entry.horizon = Some(LeaderQueryHorizon::new(0, 0));
            let preview = entry.preview.as_mut().expect("near Gate preview");
            preview.next.progress_mm = u32::MAX;
            preview.bounds = MotionBounds::HardStopped;
        }
        let mut actual = Vec::new();
        let mut expected = Vec::new();
        assert_eq!(
            world
                .state
                .step_workspace()
                .stage_vehicle_transitions(0.1, 1, 100, &mut actual, None),
            reference.state.step_workspace().stage_vehicle_transitions(
                0.1,
                1,
                100,
                &mut expected,
                None
            )
        );
        assert_eq!(actual, expected);
        assert!(world.state.workspace.motion_cache.is_empty());
    }

    #[test]
    fn motion_preview_storage_is_counted_consumed_and_discarded_on_failure() {
        let mut world = crate::kernel::waiting::tests::multi_gate_world(2);
        let mut fresh = crate::kernel::waiting::tests::multi_gate_world(2);
        assert_eq!(world.state.workspace.motion_cache.capacity(), 0);
        world.state.rebuild_occupancy_index().unwrap();
        world.state.prepare_waiting_step(0.1).unwrap();
        world.state.prepare_conflict_step(0.1, 1, None).unwrap();
        assert!(!world.state.workspace.motion_cache.is_empty());
        let with_previews = world.state.workspace.retained_logical_bytes();
        let previews = std::mem::take(&mut world.state.workspace.motion_cache);
        assert_eq!(
            with_previews - world.state.workspace.retained_logical_bytes(),
            (previews.capacity() * std::mem::size_of::<MotionCacheEntry>()) as u64
        );
        world.state.workspace.motion_cache = previews;
        let empty_workspace = {
            world.state.workspace.motion_cache = Vec::new();
            world.state.workspace.retained_logical_bytes()
        };
        for capacity in [10_000, 100_000] {
            world.state.workspace.motion_cache = Vec::with_capacity(capacity);
            let bytes = world.state.workspace.retained_logical_bytes() - empty_workspace;
            assert_eq!(
                bytes,
                (capacity * std::mem::size_of::<MotionCacheEntry>()) as u64
            );
            eprintln!("motion-cache retained capacity={capacity} bytes={bytes}");
        }
        // 上面停在内部准备阶段，不能把尚未丢弃的 grant 当成公开 step 的输入。
        world = crate::kernel::waiting::tests::multi_gate_world(2);
        let before = world.capture_snapshot().unwrap();
        STEP_FAILPOINT.set(Some(StepFailpoint::AfterGrants));
        assert_eq!(
            world.step(TickInput::new(100)),
            Err(StepError::ParkingObservationAllocFailed)
        );
        assert!(world.state.workspace.motion_cache.is_empty());
        assert_eq!(world.capture_snapshot().unwrap(), before);
        for _ in 0..24 {
            world.step(TickInput::new(100)).unwrap();
            fresh.step(TickInput::new(100)).unwrap();
            assert!(world.state.workspace.motion_cache.is_empty());
            assert_eq!(
                world.capture_snapshot().unwrap(),
                fresh.capture_snapshot().unwrap()
            );
            assert_eq!(
                world.latest_transition_events(),
                fresh.latest_transition_events()
            );
            assert_eq!(
                world.latest_conflict_decisions(),
                fresh.latest_conflict_decisions()
            );
            assert_eq!(
                world.latest_waiting_decisions(),
                fresh.latest_waiting_decisions()
            );
        }
    }

    #[test]
    fn every_new_conflict_scratch_allocation_failure_is_atomic_and_retryable() {
        let revision = conflict_scale_revision();
        for waiting in [false, true] {
            let mut failures = 0;
            for allocation in 0..200 {
                let mut world = if waiting {
                    crate::kernel::waiting::tests::multi_gate_world(2)
                } else {
                    conflict_scale_world(std::sync::Arc::clone(&revision), 2)
                };
                let before = world.capture_snapshot().unwrap();
                let events = world.latest_transition_events().to_vec();
                let delta = world.config().fixed_delta_time_ms();
                crate::kernel::conflict::set_allocation_failpoint(Some(allocation));
                let result = world.step(TickInput::new(delta));
                crate::kernel::conflict::set_allocation_failpoint(None);
                if result.is_ok() {
                    break;
                }
                assert_eq!(
                    result,
                    Err(StepError::ConflictScratchAllocFailed),
                    "allocation {allocation}"
                );
                failures += 1;
                assert_eq!(world.capture_snapshot().unwrap(), before);
                assert_eq!(world.latest_transition_events(), events);
                assert!(world.state.conflict_state_valid());
                world
                    .step(TickInput::new(delta))
                    .expect("retry after allocation failure");
            }
            assert!(
                failures >= 12,
                "all actual index, graph and output allocation sites are visited"
            );
            assert!(
                failures < 200,
                "the successful end of the allocation sequence must be reached"
            );
        }
    }

    #[test]
    fn failed_grant_and_transition_staging_preserves_world_and_can_retry() {
        let revision = conflict_scale_revision();
        for point in [StepFailpoint::AfterGrants, StepFailpoint::AfterTransitions] {
            let mut world = conflict_scale_world(std::sync::Arc::clone(&revision), 2);
            let before =
                crate::deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap();
            let tick = world.tick_index();
            let time = world.time_ms();
            let decisions = world.latest_conflict_decisions().to_vec();
            STEP_FAILPOINT.with(|failpoint| failpoint.set(Some(point)));
            assert_eq!(
                world.step(TickInput::new(4)),
                Err(StepError::ParkingObservationAllocFailed)
            );
            assert!(world.state.conflict_state_valid());
            assert_eq!(world.tick_index(), tick);
            assert_eq!(world.time_ms(), time);
            assert_eq!(world.latest_conflict_decisions(), decisions);
            assert_eq!(
                crate::deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap(),
                before
            );
            world
                .step(TickInput::new(4))
                .expect("failure must not poison the next tick");
            assert!(world.state.conflict_state_valid());
            assert!(
                world
                    .latest_conflict_decisions()
                    .iter()
                    .any(|decision| decision.outcome() == crate::ConflictDecisionOutcome::Granted)
            );
        }
    }

    fn journal_trace(world: &TrafficWorld) -> String {
        format!(
            "{:?}",
            world
                .state
                .migration_journal()
                .unwrap()
                .records_from(0)
                .collect::<Vec<_>>()
        )
    }

    fn waiting_retry_world() -> TrafficWorld {
        let mut world = crate::kernel::waiting::tests::multi_gate_world(1);
        let initial = world.vehicle(world.state.committed.live_order[0]).unwrap();
        world.state.arm_migration_journal(128 * 1_024).unwrap();
        world.step(TickInput::new(100)).unwrap();
        assert!(!world.latest_conflict_decisions().is_empty());
        assert!(!world.latest_waiting_decisions().is_empty());
        assert!(!world.latest_transition_events().is_empty());
        // 生命周期命令保留上次发布批次；在相同路线重新制造一次有效申请。
        world.despawn_vehicle(initial.handle).unwrap();
        world
            .spawn_vehicle(crate::VehicleSpawnInput::new(
                initial.profile,
                initial.route,
                initial.route_edge_index,
                initial.progress_mm,
                initial.speed_mm_s,
            ))
            .unwrap();
        world
            .state
            .workspace
            .conflict
            .set_serial_for_test(u64::MAX - 1);
        world
    }

    #[test]
    fn conflict_serial_retry_preserves_nonempty_batches_and_journal() {
        for point in [StepFailpoint::AfterGrants, StepFailpoint::AfterTransitions] {
            let mut world = waiting_retry_world();
            let mut fresh = waiting_retry_world();
            let before = world.capture_snapshot().unwrap();
            let conflicts = world.latest_conflict_decisions().to_vec();
            let waiting = world.latest_waiting_decisions().to_vec();
            let events = world.latest_transition_events().to_vec();
            let journal = journal_trace(&world);
            let stats = world.migration_journal_stats();
            STEP_FAILPOINT.set(Some(point));
            assert_eq!(
                world.step(TickInput::new(100)),
                Err(StepError::ParkingObservationAllocFailed)
            );
            assert_eq!(
                world.state.workspace.conflict.serial_for_test(),
                u64::MAX - 1
            );
            assert!(world.state.workspace.conflict_grants.is_empty());
            assert!(world.state.conflict_state_valid());
            assert_eq!(world.capture_snapshot().unwrap(), before);
            assert_eq!(world.latest_conflict_decisions(), conflicts);
            assert_eq!(world.latest_waiting_decisions(), waiting);
            assert_eq!(world.latest_transition_events(), events);
            assert_eq!(world.migration_journal_stats(), stats);
            assert_eq!(journal_trace(&world), journal);

            world.step(TickInput::new(100)).unwrap();
            fresh.step(TickInput::new(100)).unwrap();
            assert_eq!(world.state.workspace.conflict.serial_for_test(), u64::MAX);
            assert_eq!(
                world.capture_snapshot().unwrap(),
                fresh.capture_snapshot().unwrap()
            );
            assert_eq!(
                world.latest_conflict_decisions(),
                fresh.latest_conflict_decisions()
            );
            assert_eq!(
                world.latest_waiting_decisions(),
                fresh.latest_waiting_decisions()
            );
            assert_eq!(
                world.latest_transition_events(),
                fresh.latest_transition_events()
            );
            assert_eq!(journal_trace(&world), journal_trace(&fresh));
        }
    }

    #[test]
    fn conflict_serial_retry_after_real_allocation_failure() {
        let revision = conflict_scale_revision();
        let mut world = conflict_scale_world(std::sync::Arc::clone(&revision), 2);
        let mut fresh = conflict_scale_world(revision, 2);
        for target in [&mut world, &mut fresh] {
            target
                .state
                .workspace
                .conflict
                .set_serial_for_test(u64::MAX - 1);
        }
        let before = world.capture_snapshot().unwrap();
        STEP_FAILPOINT.set(Some(StepFailpoint::AllocationAfterGrants));
        let failed = world.step(TickInput::new(4));
        crate::kernel::conflict::set_allocation_failpoint(None);
        assert_eq!(failed, Err(StepError::ConflictScratchAllocFailed));
        assert_eq!(
            world.state.workspace.conflict.serial_for_test(),
            u64::MAX - 1
        );
        assert!(world.state.workspace.conflict_grants.is_empty());
        assert_eq!(world.capture_snapshot().unwrap(), before);
        world.step(TickInput::new(4)).unwrap();
        fresh.step(TickInput::new(4)).unwrap();
        assert_eq!(world.state.workspace.conflict.serial_for_test(), u64::MAX);
        assert_eq!(
            world.capture_snapshot().unwrap(),
            fresh.capture_snapshot().unwrap()
        );
        assert_eq!(
            world.latest_conflict_decisions(),
            fresh.latest_conflict_decisions()
        );
        assert_eq!(
            world.latest_transition_events(),
            fresh.latest_transition_events()
        );
    }

    #[test]
    fn conflict_serial_exhaustion_preserves_earlier_step_errors() {
        let mut world = crate::kernel::waiting::tests::multi_gate_world(1);
        world.state.workspace.conflict.set_serial_for_test(u64::MAX);
        let before = world.capture_snapshot().unwrap();
        assert!(matches!(
            world.step(TickInput::new(1)),
            Err(StepError::DeltaMismatch { .. })
        ));
        for _ in 0..2 {
            assert_eq!(
                world.step(TickInput::new(100)),
                Err(StepError::ConflictInvariantViolation)
            );
            assert_eq!(world.state.workspace.conflict.serial_for_test(), u64::MAX);
            assert_eq!(world.capture_snapshot().unwrap(), before);
            assert!(world.state.workspace.conflict_grants.is_empty());
        }
    }

    #[test]
    fn failed_empty_event_tick_preserves_previous_batch_and_retries_like_fresh() {
        let mut world = crate::kernel::waiting::tests::multi_gate_world(2);
        let mut fresh = crate::kernel::waiting::tests::multi_gate_world(2);
        for _ in 0..64 {
            fresh.step(TickInput::new(100)).unwrap();
            if !world.latest_transition_events().is_empty()
                && fresh.latest_transition_events().is_empty()
            {
                let before = world.capture_snapshot().unwrap();
                let events = world.latest_transition_events().to_vec();
                STEP_FAILPOINT.set(Some(StepFailpoint::AfterTransitions));
                assert_eq!(
                    world.step(TickInput::new(100)),
                    Err(StepError::ParkingObservationAllocFailed)
                );
                assert_eq!(world.capture_snapshot().unwrap(), before);
                assert_eq!(world.latest_transition_events(), events);
                world.step(TickInput::new(100)).unwrap();
                assert_eq!(
                    world.capture_snapshot().unwrap(),
                    fresh.capture_snapshot().unwrap()
                );
                assert_eq!(
                    world.latest_transition_events(),
                    fresh.latest_transition_events()
                );
                assert_eq!(
                    world.latest_waiting_decisions(),
                    fresh.latest_waiting_decisions()
                );
                assert_eq!(
                    world.latest_conflict_decisions(),
                    fresh.latest_conflict_decisions()
                );
                return;
            }
            world.step(TickInput::new(100)).unwrap();
        }
        panic!("fixture must transition from a nonempty event batch to an empty one");
    }

    #[test]
    fn failed_zero_non_entry_tick_preserves_outputs_and_retries_like_fresh() {
        let mut world = crate::kernel::waiting::tests::multi_gate_world(2);
        let mut fresh = crate::kernel::waiting::tests::multi_gate_world(2);
        for _ in 0..64 {
            fresh.step(TickInput::new(100)).unwrap();
            let previous_non_entry = world
                .latest_waiting_decisions()
                .iter()
                .any(|decision| decision.zone().is_none());
            let next_has_no_non_entry = fresh
                .latest_waiting_decisions()
                .iter()
                .all(|decision| decision.zone().is_some());
            if previous_non_entry && next_has_no_non_entry {
                let before = world.capture_snapshot().unwrap();
                let waiting = world.latest_waiting_decisions().to_vec();
                let conflict = world.latest_conflict_decisions().to_vec();
                let events = world.latest_transition_events().to_vec();
                STEP_FAILPOINT.set(Some(StepFailpoint::AfterTransitions));
                assert_eq!(
                    world.step(TickInput::new(100)),
                    Err(StepError::ParkingObservationAllocFailed)
                );
                assert_eq!(world.capture_snapshot().unwrap(), before);
                assert_eq!(world.latest_waiting_decisions(), waiting);
                assert_eq!(world.latest_conflict_decisions(), conflict);
                assert_eq!(world.latest_transition_events(), events);
                world.step(TickInput::new(100)).unwrap();
                assert_eq!(
                    world.capture_snapshot().unwrap(),
                    fresh.capture_snapshot().unwrap()
                );
                assert_eq!(
                    world.latest_waiting_decisions(),
                    fresh.latest_waiting_decisions()
                );
                assert_eq!(
                    world.latest_conflict_decisions(),
                    fresh.latest_conflict_decisions()
                );
                assert_eq!(
                    world.latest_transition_events(),
                    fresh.latest_transition_events()
                );
                return;
            }
            world.step(TickInput::new(100)).unwrap();
        }
        panic!("fixture must transition from non-entry Gate decisions to none");
    }
}

/// §10.1 跟车查询窗：静止前车最坏情况，SI 有限后 `ceil` 到毫米。
///
/// `bumper_gap_mm` 是后杠间隙接纳窗；`front_query_mm` 是出现项行走窗
///（`ceil(bumper) + MAX_VEHICLE_LENGTH_MM`）。溢出饱和，禁止缩短行走窗。
/// 非有限输入失败关闭，不得当成「本拍无前车」。
pub(crate) fn leader_query_horizon(
    speed_mm_s: u32,
    profile: VehicleProfileView,
    delta_s: f32,
) -> Option<LeaderQueryHorizon> {
    #[cfg(test)]
    HORIZON_CALCULATIONS.set(HORIZON_CALCULATIONS.get() + 1);
    if !delta_s.is_finite() || delta_s <= 0.0 {
        return None;
    }
    let speed = si_speed(speed_mm_s);
    let accel = profile.max_accel();
    let emergency = profile.emergency_decel();
    let min_gap = si_meters(profile.min_gap_mm());
    let headway = profile.time_headway();
    if ![speed, accel, emergency, min_gap, headway]
        .into_iter()
        .all(f32::is_finite)
    {
        return None;
    }
    if accel < 0.0 || emergency <= 0.0 || min_gap < 0.0 || headway < 0.0 {
        return None;
    }
    let v_upper = speed + accel * delta_s;
    let travel_upper = 0.5 * (speed + v_upper) * delta_s;
    let hard_horizon = travel_upper + v_upper * v_upper / (2.0 * emergency);
    let comfort_horizon = min_gap + speed * headway;
    let minimum_gap_horizon = min_gap + travel_upper + si_meters(MINIMUM_GAP_TOLERANCE_MM);
    let bumper = hard_horizon.max(comfort_horizon).max(minimum_gap_horizon);
    if !bumper.is_finite() || bumper < 0.0 {
        return None;
    }
    let bumper_gap_mm = ceil_mm(f64::from(bumper))?;
    Some(LeaderQueryHorizon::new(
        bumper_gap_mm,
        bumper_gap_mm.saturating_add(MAX_VEHICLE_LENGTH_MM),
    ))
}

impl crate::kernel::state::WorldState {
    /// 固定步进唯一入口：预检、重建占用索引、准备并原子提交一拍。
    pub(crate) fn step_vehicles(
        &mut self,
        input: TickInput,
        execution: Option<&crate::kernel::execution::ExecutionResources>,
    ) -> Result<StepOutcome, StepError> {
        self.workspace.motion_cache.clear();
        #[cfg(test)]
        let preflight_timer =
            super::performance_profile::begin(super::performance_profile::Stage::Preflight);
        let expected = self.binding.config.fixed_delta_time_ms();
        if input.delta_time_ms != expected {
            return Err(StepError::DeltaMismatch {
                expected_delta_time_ms: expected,
                actual_delta_time_ms: input.delta_time_ms,
            });
        }
        if !self.conflict_state_valid() {
            return Err(StepError::ConflictInvariantViolation);
        }
        let tick_index = self
            .committed
            .tick_index
            .checked_add(1)
            .ok_or(StepError::Overflow)?;
        let time_ms = self
            .committed
            .time_ms
            .checked_add(expected)
            .ok_or(StepError::Overflow)?;
        let observation_state_sequence = self
            .committed
            .observation_state_sequence
            .checked_next()
            .ok_or(StepError::ObservationStateSequenceExhausted)?;
        let delta_s = expected as f32 / 1_000.0;
        #[cfg(test)]
        drop(preflight_timer);
        #[cfg(test)]
        let occupancy_timer =
            super::performance_profile::begin(super::performance_profile::Stage::Occupancy);
        self.rebuild_occupancy_index()?;
        #[cfg(test)]
        drop(occupancy_timer);
        let plan = self.step_workspace().prepare_commit(
            delta_s,
            tick_index,
            time_ms,
            observation_state_sequence,
            execution,
        );
        // prepare 的任一首错（包括 Waiting 预选失败）都丢弃本拍输入与证明。
        self.workspace.motion_cache.clear();
        Ok(self.committed_mut().commit(plan?))
    }

    /// 测试专用：无 Waiting/Conflict 停车约束地推进一步活动车辆。
    #[cfg(test)]
    pub(crate) fn advance_active_vehicle(
        &self,
        state: VehicleState,
        delta_s: f32,
    ) -> Option<VehicleState> {
        self.read_view().advance_active_vehicle(state, delta_s)
    }

    /// 测试专用：读本拍占用索引上的前保险杠间隙。不是生产热路径。
    ///
    /// 使用与 `advance_active_vehicle` 相同的公式窗。公式非有限时 panic，
    /// 不得把失败关闭写成「本拍无前车」。调用前必须已 `rebuild_occupancy_index`。
    #[cfg(test)]
    pub(crate) fn leader_bumper_gap(
        &self,
        follower: &VehicleState,
        edges: &[LaneEdgeOrdinal],
        lengths: &[u32],
    ) -> Option<i64> {
        self.read_view().leader_bumper_gap(follower, edges, lengths)
    }

    /// 测试专用：计算该跟车车辆本拍的跟车前视查询窗。
    #[cfg(test)]
    pub(crate) fn leader_query_horizon_for(&self, follower: &VehicleState) -> LeaderQueryHorizon {
        self.read_view().leader_query_horizon_for(follower)
    }

    /// `cfg(test)` 全扫描再按 `bumper_gap_horizon` 过滤，不是生产热路径。
    #[cfg(test)]
    pub(crate) fn leader_bumper_gap_scan(
        &self,
        follower: &VehicleState,
        edges: &[LaneEdgeOrdinal],
        lengths: &[u32],
    ) -> Option<i64> {
        self.read_view()
            .leader_bumper_gap_scan(follower, edges, lengths)
    }

    /// 测试专用：判断该机动门对指定车辆配置是否拒绝并停车。
    #[cfg(test)]
    pub(crate) fn gate_is_restrictive(
        &self,
        gate: laneflow_static_contract::ManeuverGateOrdinal,
        profile: VehicleProfileOrdinal,
    ) -> bool {
        self.read_view().gate_is_restrictive(gate, profile)
    }

    /// 测试专用：按当前已提交信号求值机动门的策略决定。
    #[cfg(test)]
    pub(crate) fn gate_policy_decision(
        &self,
        gate: laneflow_static_contract::ManeuverGateOrdinal,
        profile: VehicleProfileOrdinal,
    ) -> crate::GatePolicyDecision {
        self.read_view().gate_policy_decision(gate, profile)
    }
}

/// 完整性证明仅在本模块创建；借用尚未释放便消费，不能跨 step 保存或重用。
/// 其余已校验的转移、批次和信号留在同一世界的 Workspace，避免复制工作集。
struct CommitPlan {
    updates: Vec<(usize, VehicleState)>,
    parking_arrivals: Vec<ParkingArrivalObservation>,
    tick_index: u64,
    time_ms: u64,
    observation_state_sequence: crate::ObservationStateSequence,
}

impl crate::kernel::phase::StepWorkspace<'_> {
    fn prepare_commit(
        &mut self,
        delta_s: f32,
        tick_index: u64,
        time_ms: u64,
        observation_state_sequence: crate::ObservationStateSequence,
        execution: Option<&crate::kernel::execution::ExecutionResources>,
    ) -> Result<CommitPlan, StepError> {
        #[cfg(test)]
        let waiting_timer =
            super::performance_profile::begin(super::performance_profile::Stage::WaitingPrepare);
        self.prepare_waiting_step(delta_s, execution)?;
        #[cfg(test)]
        drop(waiting_timer);
        let serial_checkpoint = self.workspace.conflict.serial_checkpoint();
        let mut updates = std::mem::take(&mut self.workspace.next_states);
        updates.clear();
        let parking_arrivals = match self.stage_vehicle_transitions(
            delta_s,
            tick_index,
            time_ms,
            &mut updates,
            execution,
        ) {
            Ok(arrivals) => arrivals,
            Err(error) => {
                self.rollback_waiting_step();
                self.committed
                    .prepare_conflict(&mut self.derived, &mut self.workspace.conflict)
                    .discard_staged();
                self.workspace.conflict_grants.clear();
                // grant 不离开本次准备；先丢弃所有凭证，再恢复失败 attempt 的额度。
                self.workspace.conflict.restore_serial(serial_checkpoint);
                self.workspace.conflict_staged_decisions.clear();
                self.workspace.conflict_passage_transitions.clear();
                self.workspace.motion_cache.clear();
                updates.clear();
                self.workspace.next_states = updates;
                return Err(error);
            }
        };
        Ok(CommitPlan {
            updates,
            parking_arrivals,
            tick_index,
            time_ms,
            observation_state_sequence,
        })
    }
}

impl crate::kernel::phase::CommittedStateMut<'_> {
    /// P7 唯一入口：只消费已经完整校验的计划，不再返回 StepError。
    fn commit(mut self, plan: CommitPlan) -> StepOutcome {
        #[cfg(test)]
        let _commit_timer =
            super::performance_profile::begin(super::performance_profile::Stage::Commit);
        // W5-B：P7 分配窗口计量（cfg(test)，生产构建无此代码）。
        #[cfg(test)]
        let _p7_alloc_window = begin_commit_alloc_window();
        let CommitPlan {
            mut updates,
            parking_arrivals,
            tick_index,
            time_ms,
            observation_state_sequence,
        } = plan;
        // 以下提交仅消费已预留、已验证的转移；没有可恢复错误出口。
        self.commit_conflict_transitions(&updates, time_ms);
        self.commit_waiting_removals(&updates);
        // 武装期 TICK 记录在状态写回前开帧、写回后闭帧；无变化条目被过滤，
        // 零变化步进仍保留空记录（tick/时间是候选时钟与摘要头部的收敛依据）。
        let mut migration_journal = self.journal.take();
        if let Some(journal) = migration_journal.as_mut() {
            journal.begin_tick(tick_index, time_ms);
        }
        for (slot, next) in &updates {
            let previous = self.committed.vehicles[*slot].state.replace(*next);
            if let Some(journal) = migration_journal.as_mut()
                && !previous.as_ref().is_some_and(|old| *old == *next)
            {
                let delta =
                    VehicleDelta::from_state(next, self.read_view().compiled_route(next.route));
                journal.tick_entry(&delta);
            }
        }
        if let Some(journal) = migration_journal.as_mut() {
            for claims in self
                .workspace
                .waiting_claims
                .chunk_by(|left, right| left.zone == right.zone)
            {
                #[cfg(test)]
                crate::kernel::waiting::count_waiting_work(|counts| counts.journal_zones += 1);
                let zone = claims[0].zone;
                journal.tick_waiting_zone(zone, self.workspace.waiting_next_counters[zone.index()]);
            }
            self.write_conflict_tick_journal(journal, &updates);
            journal.finish_tick();
        }
        *self.journal = migration_journal;
        self.commit_waiting_additions(&updates);
        self.commit_conflict_step();
        updates.clear();
        self.workspace.next_states = updates;
        let vehicles = &self.committed.vehicles;
        self.derived.active_order.retain(|handle| {
            let index = usize::try_from(handle.index()).expect("vehicle index fits usize");
            vehicles.get(index).is_some_and(|slot| {
                slot.generation == handle.generation()
                    && slot
                        .state
                        .as_ref()
                        .is_some_and(|state| state.status == VehicleStatus::Active)
            })
        });
        self.derived.spawn_overlap.mark_stale();
        self.committed.tick_index = tick_index;
        self.committed.time_ms = time_ms;
        self.committed.observation_state_sequence = observation_state_sequence;
        core::mem::swap(
            &mut self.committed.signal_aspects,
            &mut self.workspace.next_signal_aspects,
        );
        self.workspace.frontier_maintenance.publish();
        StepOutcome::new(tick_index, time_ms, parking_arrivals)
    }
}

impl<'a> crate::kernel::phase::StepReadView<'a> {
    /// 无 Waiting/Conflict 停车约束地推进一步活动车辆。
    #[cfg(test)]
    pub(crate) fn advance_active_vehicle(
        self,
        state: VehicleState,
        delta_s: f32,
    ) -> Option<VehicleState> {
        self.advance_active_vehicle_with_waiting_stop(state, delta_s, None, None)
    }

    /// 在跟车、信号、停车与 Waiting/Conflict 停车约束下推进一步活动车辆。
    #[cfg(test)]
    pub(crate) fn advance_active_vehicle_with_waiting_stop(
        self,
        state: VehicleState,
        delta_s: f32,
        waiting_stop: Option<crate::kernel::waiting::WaitingStopConstraint>,
        conflict_stop: Option<crate::kernel::waiting::WaitingStopConstraint>,
    ) -> Option<VehicleState> {
        let parking_binding = self.committed.parking.binding(state.handle);
        self.advance_active_vehicle_with_parking_binding(
            state,
            delta_s,
            waiting_stop,
            conflict_stop,
            parking_binding,
            None,
        )
    }

    /// 正式推进复用同一拍初状态的 binding；独立预览仍从自身读取入口取得当前值。
    pub(crate) fn advance_active_vehicle_with_parking_binding(
        self,
        state: VehicleState,
        delta_s: f32,
        waiting_stop: Option<crate::kernel::waiting::WaitingStopConstraint>,
        conflict_stop: Option<crate::kernel::waiting::WaitingStopConstraint>,
        parking_binding: Option<ParkingBinding>,
        horizon: Option<LeaderQueryHorizon>,
    ) -> Option<VehicleState> {
        self.calculate_active_vehicle_motion(
            state,
            delta_s,
            waiting_stop,
            conflict_stop,
            parking_binding,
            horizon,
            None,
            None,
            false,
        )
    }

    pub(crate) fn preview_active_vehicle_with_waiting_stop(
        self,
        state: VehicleState,
        delta_s: f32,
        waiting_stop: Option<crate::kernel::waiting::WaitingStopConstraint>,
        horizon: Option<LeaderQueryHorizon>,
    ) -> Option<MotionPreview> {
        let mut bounds = MotionBounds::Unknown;
        let next = self.calculate_active_vehicle_motion(
            state,
            delta_s,
            waiting_stop,
            None,
            self.committed.parking.binding(state.handle),
            horizon,
            Some(&mut bounds),
            None,
            false,
        )?;
        Some(MotionPreview {
            next,
            waiting_stop,
            bounds,
        })
    }

    /// P2 逐车独立预览原语：受检读取 state/route/profile，求值 Waiting 前视窗
    /// 与运动预览，不触碰共享工作区。与串行第一遍同一领域原语、同一检查次序：
    /// state/route/profile 缺失为 `WaitingInvariantViolation`；horizon 或预览
    /// 非有限为 `NonFiniteMotion`。无后续 Gate、Gate 距离非有限或超出前视窗时
    /// 按原语义返回 `None` 字段。调用方负责 Active 过滤（`vehicle` 必须来自
    /// `update_sequence` 处的 live 配对）与 `update_sequence` 暂存。
    pub(crate) fn waiting_preview_entry(
        self,
        vehicle: crate::VehicleHandle,
        update_sequence: usize,
        delta_s: f32,
    ) -> Result<WaitingPreviewEntry, StepError> {
        debug_assert_eq!(
            self.committed.live_order.get(update_sequence),
            Some(&vehicle),
            "preview entry provenance"
        );
        #[cfg(test)]
        {
            if crate::kernel::waiting::injected_preview_panics(
                self.binding.world_id,
                update_sequence,
            ) {
                panic!("injected P2 preview panic at logical position {update_sequence}");
            }
            if let Some(error) = crate::kernel::waiting::injected_preview_error(
                self.binding.world_id,
                update_sequence,
            ) {
                return Err(error);
            }
        }
        let state = *self
            .vehicle_state(vehicle)
            .ok_or(StepError::WaitingInvariantViolation)?;
        let compiled = self
            .compiled_route(state.route)
            .ok_or(StepError::WaitingInvariantViolation)?;
        let cursor = state.route_edge_index as usize;
        let gate_index = compiled
            .gate_hops
            .partition_point(|hop| (*hop as usize) < cursor);
        let Some(gate_hop) = compiled.gate_hops.get(gate_index).copied() else {
            return Ok(WaitingPreviewEntry {
                horizon: None,
                preview: None,
            });
        };
        let profile = self
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
            .ok_or(StepError::WaitingInvariantViolation)?;
        let horizon = leader_query_horizon(state.speed_mm_s, profile, delta_s)
            .ok_or(StepError::NonFiniteMotion)?;
        let gate_distance = distance_to_occurrence_start(
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            cursor,
            state.progress_mm,
            (gate_hop as usize)
                .checked_add(1)
                .ok_or(StepError::WaitingInvariantViolation)?,
        );
        let Some(BoundedDistance::Finite(gate_distance_mm)) = gate_distance else {
            return Ok(WaitingPreviewEntry {
                horizon: Some(horizon),
                preview: None,
            });
        };
        if gate_distance_mm > horizon.front_query_mm {
            return Ok(WaitingPreviewEntry {
                horizon: Some(horizon),
                preview: None,
            });
        }
        let preview = self
            .preview_active_vehicle_with_waiting_stop(state, delta_s, None, Some(horizon))
            .ok_or(StepError::NonFiniteMotion)?;
        Ok(WaitingPreviewEntry {
            horizon: Some(horizon),
            preview: Some(preview),
        })
    }

    /// 用指定前车空隙预测本拍硬截断。不写入世界。
    ///
    /// `leader_constraint_only` 时硬房间只含这名前车的快照空隙，避免把后车自己的
    /// 信号或路终清零算成这名前车造成的硬投影。否则，已经被其他车占用，或空着但
    /// 已有别的车这一拍够得到的冲突区，也算进这次预览。空着且没人够得到的冲突区
    /// 仍可能在本拍获准，不预写通行权。
    pub(crate) fn placement_motion(
        self,
        state: VehicleState,
        leader_gap: Option<i64>,
        leader_constraint_only: bool,
    ) -> Option<PlacementMotion> {
        let delta_s = self.binding.config.fixed_delta_time_ms() as f32 / 1_000.0;
        let conflict_stop = if leader_constraint_only {
            None
        } else {
            match self.occupied_conflict_stop(&state) {
                OccupiedConflictPreview::Clear => None,
                OccupiedConflictPreview::Stop(stop) => Some(stop),
                OccupiedConflictPreview::Unprovable => return None,
            }
        };
        let mut bounds = MotionBounds::Unknown;
        let next = self.calculate_active_vehicle_motion(
            state,
            delta_s,
            None,
            conflict_stop,
            self.committed.parking.binding(state.handle),
            None,
            Some(&mut bounds),
            Some(leader_gap),
            leader_constraint_only,
        )?;
        let (committed_travel_mm, hard_clamped) = match bounds {
            MotionBounds::HardStopped => (0, true),
            MotionBounds::Travel {
                committed_mm,
                exhausted,
                ..
            } => (committed_mm, exhausted),
            MotionBounds::Unknown => return None,
        };
        Some(PlacementMotion {
            next_speed_mm_s: next.speed_mm_s,
            committed_travel_mm,
            hard_clamped,
        })
    }

    /// 最近一个这一拍通行权没有保证的冲突准入。
    ///
    /// 已经有主，或空着但已有别的车这一拍够得到，都算停车点。空着且没人够得到的区跳过。
    /// 只读已提交占用和生成前记下的够得到名单，不看本拍暂存，也不预写通行权。
    /// 距离算到该 hop 的后继出现项起点，与正式步进相同。
    fn occupied_conflict_stop(self, state: &VehicleState) -> OccupiedConflictPreview {
        let Some(compiled) = self.compiled_route(state.route) else {
            return OccupiedConflictPreview::Clear;
        };
        let first_hop = if state.progress_mm == 0 && state.carry_um == 0 {
            state.route_edge_index.saturating_sub(1)
        } else {
            state.route_edge_index
        };
        let read = self.conflict_read();
        let outsider = crate::VehicleHandle::new(u32::MAX, u32::MAX);
        let mut index = compiled
            .conflicts
            .partition_point(|entry| entry.admission_hop < first_hop);
        while index < compiled.conflicts.len() {
            let hop = compiled.conflicts[index].admission_hop;
            let mut owned = false;
            while index < compiled.conflicts.len() && compiled.conflicts[index].admission_hop == hop
            {
                let entry = compiled.conflicts[index];
                let address = entry.address();
                if read.cells_unavailable(outsider, std::slice::from_ref(&address))
                    || self.conflict_zone_contended(entry.zone)
                {
                    owned = true;
                }
                index = index.saturating_add(1);
            }
            if !owned {
                continue;
            }
            let Some(stop_index) = usize::try_from(hop).ok().and_then(|hop| hop.checked_add(1))
            else {
                return OccupiedConflictPreview::Unprovable;
            };
            let Some(distance) = distance_to_occurrence_start(
                &compiled.occurrence_segments,
                &compiled.occurrence_offsets,
                &compiled.segment_totals,
                state.route_edge_index as usize,
                state.progress_mm,
                stop_index,
            ) else {
                return OccupiedConflictPreview::Unprovable;
            };
            return match distance {
                BoundedDistance::Finite(_) => {
                    OccupiedConflictPreview::Stop(crate::kernel::waiting::WaitingStopConstraint {
                        distance,
                        hop,
                    })
                }
                BoundedDistance::BeyondFinite => OccupiedConflictPreview::Clear,
            };
        }
        OccupiedConflictPreview::Clear
    }

    /// 已有车这一拍也会申请这个冲突区，新车的通行权没有保证。
    /// 名单还没按当前序号建好，或这个区不在名单里时，按会有人来抢处理。
    fn conflict_zone_contended(self, zone: laneflow_static_contract::ConflictZoneOrdinal) -> bool {
        let contenders = &self.derived.spawn_contenders;
        if contenders.built_sequence != Some(self.committed.observation_state_sequence) {
            return true;
        }
        contenders.counts.get(zone.index()).copied().unwrap_or(1) > 0
    }

    #[allow(clippy::too_many_arguments)]
    fn calculate_active_vehicle_motion(
        self,
        mut state: VehicleState,
        delta_s: f32,
        waiting_stop: Option<crate::kernel::waiting::WaitingStopConstraint>,
        conflict_stop: Option<crate::kernel::waiting::WaitingStopConstraint>,
        parking_binding: Option<ParkingBinding>,
        horizon: Option<LeaderQueryHorizon>,
        motion_bounds: Option<&mut MotionBounds>,
        leader_gap_override: Option<Option<i64>>,
        leader_constraint_only: bool,
    ) -> Option<VehicleState> {
        #[cfg(test)]
        MOTION_CALCULATIONS.set(MOTION_CALCULATIONS.get() + 1);
        #[cfg(test)]
        let inputs_timer = super::exact_path_research::begin(
            super::exact_path_research::Stage::RouteProfileInputs,
        );
        let compiled = self.compiled_route(state.route)?;
        let edges = compiled.edges.as_slice();
        let cursor = usize::try_from(state.route_edge_index).ok()?;
        let edge = *edges.get(cursor)?;
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
        let speed_limits = self
            .binding
            .revision
            .traffic()
            .lane_speed_limits_millimetres_per_second();
        let current_limit = *speed_limits.get(edge.index())?;
        let profile = self
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)?;
        let desired_mm_s = profile.desired_speed_mm_s().min(current_limit);
        #[cfg(test)]
        drop(inputs_timer);
        #[cfg(test)]
        let horizon_timer =
            super::exact_path_research::begin(super::exact_path_research::Stage::LeaderHorizon);
        let horizon = match horizon {
            Some(horizon) => horizon,
            None => leader_query_horizon(state.speed_mm_s, profile, delta_s)?,
        };
        #[cfg(test)]
        drop(horizon_timer);
        #[cfg(test)]
        let gap_timer =
            super::exact_path_research::begin(super::exact_path_research::Stage::LeaderGap);
        let leader_gap = match leader_gap_override {
            Some(gap) => gap,
            None => self.derived.occupancy.leader_gap(
                state.handle,
                edges,
                cursor,
                state.progress_mm,
                lengths,
                horizon,
            ),
        };
        #[cfg(test)]
        drop(gap_timer);
        #[cfg(test)]
        let stop_timer =
            super::exact_path_research::begin(super::exact_path_research::Stage::RouteStopQueries);
        let route_end =
            remaining_to_route_end(*compiled.remaining_to_end.get(cursor)?, state.progress_mm);
        let reach = MotionReach::from_tick(state.speed_mm_s, profile.max_accel(), delta_s);
        let signal_stop = self.signal_stop_distance(compiled, &state, cursor, reach);
        let parking = self.parking_stop_distance(compiled, &state, cursor, parking_binding)?;
        #[cfg(test)]
        drop(stop_timer);
        let parking_stop = parking.map(|(_, distance)| distance);
        let selected_stop = select_movement_stop(signal_stop, parking_stop, route_end);
        let mut movement_stop = (!matches!(selected_stop.attribution, StopAttribution::RouteEnd))
            .then_some(selected_stop.distance);
        if let Some(waiting) = waiting_stop {
            movement_stop = match movement_stop {
                Some(current) if !stop_is_nearer_or_equal(waiting.distance, current) => {
                    Some(current)
                }
                Some(_) | None => Some(waiting.distance),
            };
        }
        if let Some(conflict) = conflict_stop {
            movement_stop = match movement_stop {
                Some(current) if !stop_is_nearer_or_equal(conflict.distance, current) => {
                    Some(current)
                }
                Some(_) | None => Some(conflict.distance),
            };
        }
        // 信号链看不到没有信号组的拒绝门。这一拍够得到时，硬房间算到那道门，
        // 不能等 apply_travel 截断后还留着原来的速度。
        if let Some(gate_stop) =
            self.restrictive_gate_stop(compiled, edges, lengths, &state, cursor, reach)
        {
            movement_stop = match movement_stop {
                Some(current) if !stop_is_nearer_or_equal(gate_stop, current) => Some(current),
                Some(_) | None => Some(gate_stop),
            };
        }
        let (mut travel_m, next_speed_m) = si_comfort_travel(
            state.speed_mm_s,
            desired_mm_s,
            leader_gap,
            profile,
            route_end,
            movement_stop,
            compiled,
            lengths,
            speed_limits,
            cursor,
            state.progress_mm,
            delta_s,
        )?;
        if travel_m < 0.0 {
            travel_m = 0.0;
        }
        if !travel_m.is_finite() || !next_speed_m.is_finite() {
            return None;
        }

        let edge_length_mm = lengths.get(edge.index()).copied()?;
        let edge_remaining_mm = edge_length_mm.saturating_sub(state.progress_mm);
        // 到不了本边尽头时，许可读数不会收紧这一次 hard_room。真正跨边仍在 apply_travel_mm 里检查。
        let permitted_for_hard_room = reach.is_some_and(|reach| reach.excludes(edge_remaining_mm))
            || {
                #[cfg(test)]
                note_barrier_query(|counts| counts.hard_room_permissions += 1);
                self.hop_permitted(state.route, edges, cursor, state.profile)
            };
        let hard_room = if leader_constraint_only {
            match leader_gap {
                Some(gap) => snapshot_leader_room_mm(gap, profile.min_gap_mm()),
                None => u32::MAX,
            }
        } else {
            hard_room_mm(
                leader_gap,
                profile.min_gap_mm(),
                movement_stop,
                route_end,
                edge_length_mm,
                state.progress_mm,
                permitted_for_hard_room,
            )
        };
        if hard_room == 0 {
            if let Some(bounds) = motion_bounds {
                *bounds = MotionBounds::HardStopped;
            }
            state.speed_mm_s = 0;
            state.carry_um = 0;
            let arrived = parking
                .is_some_and(|(reservation, _)| self.parking_arrived_for(state, reservation));
            if matches!(route_end, BoundedDistance::Finite(0)) && !arrived {
                state.status = VehicleStatus::Completed;
            }
            return Some(state);
        }

        let um = u64::from(state.carry_um).saturating_add(round_um(f64::from(travel_m))?);
        let travel_mm_for_bounds = u32::try_from((um / 1_000).min(u64::from(hard_room))).ok()?;
        let exhausted_for_bounds = travel_mm_for_bounds == hard_room;
        if let Some(bounds) = motion_bounds {
            *bounds = MotionBounds::Travel {
                meters: travel_m,
                proposed_mm: um / 1_000,
                committed_mm: travel_mm_for_bounds,
                exhausted: exhausted_for_bounds,
            };
        }
        let travel_mm = travel_mm_for_bounds;
        let exhausted = exhausted_for_bounds;
        if exhausted {
            state.carry_um = 0;
        } else {
            state.carry_um = u16::try_from(um % 1_000).ok()?;
        }
        let route = state.route;
        let vehicle_profile = state.profile;
        apply_travel_mm(&mut state, edges, lengths, travel_mm, |index| {
            self.hop_permitted(route, edges, index, vehicle_profile)
                && waiting_stop.is_none_or(|waiting| waiting.hop as usize != index)
                && conflict_stop.is_none_or(|conflict| conflict.hop as usize != index)
        })?;
        let committed_index = usize::try_from(state.route_edge_index).ok()?;
        let committed_edge = *edges.get(committed_index)?;
        let committed_limit = *speed_limits.get(committed_edge.index())?;
        let remaining = remaining_to_route_end(
            *compiled.remaining_to_end.get(committed_index)?,
            state.progress_mm,
        );
        if exhausted || matches!(remaining, BoundedDistance::Finite(0)) {
            state.speed_mm_s = 0;
            state.carry_um = 0;
            let arrived = parking
                .is_some_and(|(reservation, _)| self.parking_arrived_for(state, reservation));
            if matches!(remaining, BoundedDistance::Finite(0)) && !arrived {
                state.status = VehicleStatus::Completed;
            }
            return Some(state);
        }
        let speed_mm_s = round_mm(f64::from(next_speed_m))?.min(committed_limit);
        state.speed_mm_s = speed_mm_s;
        Some(state)
    }

    /// 按同一拍初 binding 计算停车入口距离；外层 `None` 表示绑定不一致的失败关闭。
    pub(crate) fn parking_stop_distance(
        self,
        compiled: &CompiledRoute,
        state: &VehicleState,
        cursor: usize,
        parking_binding: Option<ParkingBinding>,
    ) -> Option<Option<(ParkingReservation, BoundedDistance)>> {
        let Some(ParkingBinding::Reserved(reservation)) = parking_binding else {
            return Some(None);
        };
        if reservation.route() != state.route {
            return None;
        }
        let (edge, progress_mm) = self.reservation_anchor(reservation)?;
        let entry_index = usize::try_from(reservation.entry_route_occurrence()).ok()?;
        if compiled.edges.get(entry_index).copied()? != edge {
            return None;
        }
        let distance = distance_to_occurrence_progress(
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            cursor,
            state.progress_mm,
            entry_index,
            progress_mm,
        )?;
        Some(Some((reservation, distance)))
    }

    /// 测试专用：读本拍占用索引上的前保险杠间隙。不是生产热路径。
    ///
    /// 使用与 `advance_active_vehicle` 相同的公式窗。公式非有限时 panic，
    /// 不得把失败关闭写成「本拍无前车」。调用前必须已 `rebuild_occupancy_index`。
    #[cfg(test)]
    pub(crate) fn leader_bumper_gap(
        self,
        follower: &VehicleState,
        edges: &[LaneEdgeOrdinal],
        lengths: &[u32],
    ) -> Option<i64> {
        let cursor = usize::try_from(follower.route_edge_index).ok()?;
        let horizon = self.leader_query_horizon_for(follower);
        self.derived.occupancy.leader_gap(
            follower.handle,
            edges,
            cursor,
            follower.progress_mm,
            lengths,
            horizon,
        )
    }

    /// 测试专用：计算该跟车车辆本拍的跟车前视查询窗。
    #[cfg(test)]
    pub(crate) fn leader_query_horizon_for(self, follower: &VehicleState) -> LeaderQueryHorizon {
        let profile = self
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(follower.profile)
            .expect("test follower profile");
        let delta_s = self.binding.config.fixed_delta_time_ms() as f32 / 1_000.0;
        leader_query_horizon(follower.speed_mm_s, profile, delta_s)
            .expect("finite leader query horizon")
    }

    /// `cfg(test)` 全扫描再按 `bumper_gap_horizon` 过滤，不是生产热路径。
    #[cfg(test)]
    pub(crate) fn leader_bumper_gap_scan(
        self,
        follower: &VehicleState,
        edges: &[LaneEdgeOrdinal],
        lengths: &[u32],
    ) -> Option<i64> {
        let cursor = usize::try_from(follower.route_edge_index).ok()?;
        let accept = i64::from(self.leader_query_horizon_for(follower).bumper_gap_mm);
        let mut best: Option<i64> = None;
        for handle in self.committed.live_order.iter().copied() {
            if handle == follower.handle {
                continue;
            }
            let Some(leader) = self.vehicle_state(handle) else {
                continue;
            };
            if leader.status != VehicleStatus::Active {
                continue;
            }
            let Some(leader_edges) = self.route_edges(leader.route) else {
                continue;
            };
            let Ok(leader_index) = usize::try_from(leader.route_edge_index) else {
                continue;
            };
            let Some(gap) = occupancy_front_gap(
                lengths,
                edges,
                cursor,
                follower.progress_mm,
                leader_edges,
                leader_index,
                leader.progress_mm,
                leader.length_mm,
            ) else {
                continue;
            };
            if gap > accept {
                continue;
            }
            best = Some(best.map_or(gap, |current| current.min(gap)));
        }
        best
    }

    /// 下一受控门是拓扑链。绿灯则沿链继续，直到当前限制的门；不要在注册时冻红灯列。
    ///
    /// 停车距离读 hop 上已物化的 `distance_from_hop_start`，不靠两条「到路终」后缀相减。
    /// 路终越界时近处有界门距仍是 `Finite`。
    ///
    /// `reach` 有值且下一扇门已被证明严格远于本拍上界时，不再解释该门和更远的门。
    /// `None` 保持逐门解释。
    pub(crate) fn signal_stop_distance(
        self,
        compiled: &CompiledRoute,
        state: &VehicleState,
        cursor: usize,
        reach: Option<MotionReach>,
    ) -> Option<BoundedDistance> {
        let mut hop = cursor;
        let mut from_cursor_start = BoundedDistance::Finite(0);
        let mut accumulated = false;
        while hop < compiled.next_controlled.len() {
            let next = compiled.next_controlled[hop]?;
            from_cursor_start = if accumulated {
                from_cursor_start.add_bounded(next.distance_from_hop_start)
            } else {
                next.distance_from_hop_start
            };
            accumulated = true;
            if reach.is_some_and(|reach| {
                reach.class_is_unreachable(Some(from_cursor_start), state.progress_mm)
            }) {
                return None;
            }
            #[cfg(test)]
            note_barrier_query(|counts| counts.signal_gates += 1);
            if self.gate_is_restrictive(next.gate, state.profile) {
                return Some(from_cursor_start.saturating_sub(state.progress_mm));
            }
            let next_hop = usize::try_from(next.hop).ok()?.checked_add(1)?;
            if next_hop <= hop {
                return None;
            }
            hop = next_hop;
        }
        None
    }

    /// 沿 `hop_gate` 找这一拍够得到的最近拒绝门，含没有信号组的门。
    ///
    /// 距离算到该 hop 的边末，与 `apply_travel_mm` 停住的位置相同。超出本拍上界就停，
    /// 更远的门留到后面的拍。
    fn restrictive_gate_stop(
        self,
        compiled: &CompiledRoute,
        edges: &[LaneEdgeOrdinal],
        lengths: &[u32],
        state: &VehicleState,
        cursor: usize,
        reach: Option<MotionReach>,
    ) -> Option<BoundedDistance> {
        let mut distance = 0u32;
        for hop in cursor..edges.len().saturating_sub(1) {
            let edge = *edges.get(hop)?;
            let length = *lengths.get(edge.index())?;
            let step = if hop == cursor {
                length.saturating_sub(state.progress_mm)
            } else {
                length
            };
            distance = distance.saturating_add(step);
            if reach.is_some_and(|reach| reach.excludes(distance)) {
                return None;
            }
            if compiled
                .hop_gate
                .get(hop)
                .copied()
                .flatten()
                .is_some_and(|gate| self.gate_is_restrictive(gate, state.profile))
            {
                return Some(BoundedDistance::Finite(distance));
            }
        }
        None
    }

    /// 判断该 hop 允许越过：不是路线末边，且其 Gate 对该车辆配置不拒绝。
    pub(crate) fn hop_permitted(
        self,
        route: crate::RouteHandle,
        edges: &[LaneEdgeOrdinal],
        hop_index: usize,
        profile: VehicleProfileOrdinal,
    ) -> bool {
        if hop_index + 1 >= edges.len() {
            return false;
        }
        let Some(compiled) = self.compiled_route(route) else {
            return false;
        };
        match compiled.hop_gate.get(hop_index).copied().flatten() {
            Some(gate) => !self.gate_is_restrictive(gate, profile),
            None => true,
        }
    }

    /// 判断该机动门对指定车辆配置是否拒绝并停车。
    pub(crate) fn gate_is_restrictive(
        self,
        gate: laneflow_static_contract::ManeuverGateOrdinal,
        profile: VehicleProfileOrdinal,
    ) -> bool {
        matches!(
            self.gate_policy_decision(gate, profile),
            crate::GatePolicyDecision::DenyAndStop
        )
    }

    /// 按当前已提交信号求值机动门的策略决定。
    pub(crate) fn gate_policy_decision(
        self,
        gate: laneflow_static_contract::ManeuverGateOrdinal,
        profile: VehicleProfileOrdinal,
    ) -> crate::GatePolicyDecision {
        self.gate_policy_decision_with_signals(gate, profile, &self.committed.signal_aspects)
    }

    /// 按调用方给定的信号时刻求值机动门的策略决定；未知门、未知配置或无适用规则失败关闭为 `DenyAndStop`。
    pub(crate) fn gate_policy_decision_with_signals(
        self,
        gate: laneflow_static_contract::ManeuverGateOrdinal,
        profile: VehicleProfileOrdinal,
        signal_aspects: &[laneflow_static_contract::SignalAspect],
    ) -> crate::GatePolicyDecision {
        let gate_view = self
            .binding
            .revision
            .traffic()
            .relations()
            .maneuver_gate(gate);
        let Some(gate_view) = gate_view else {
            return crate::GatePolicyDecision::DenyAndStop;
        };
        let Some(class) = self
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(profile)
            .map(|p| p.class())
        else {
            return crate::GatePolicyDecision::DenyAndStop;
        };
        let Some(rule) = self.policy().and_then(|policy| policy.gate(gate, class)) else {
            return crate::GatePolicyDecision::DenyAndStop;
        };
        let signal_group = gate_view.signal_group();
        let aspect = signal_group.and_then(|group| signal_aspects.get(group.index()).copied());
        crate::kernel::conflict::interpret_gate_policy(*rule, signal_group.is_some(), aspect)
            .unwrap_or(crate::GatePolicyDecision::DenyAndStop)
    }
}

/// P5 逐车独立计算的固定大小结果（#706 增量 B）：最终下一状态 + 停车到达
/// 观察候选。`arrival` 只表示按原语判定应生成到达观察，不代表已完成输出
/// 预留或发布；真实 `try_reserve` 与追加由协调器在该车原逻辑位置执行
///（首错交错顺序：该车全部可失败计算先于其到达 reserve）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct VehicleMotionOutcome {
    pub(crate) next: VehicleState,
    pub(crate) arrival: Option<ParkingArrivalObservation>,
}

/// P5 任务视图：拍初基线只读投影 + 本拍已冻结的协调器暂存（P4 裁决的
/// waiting/conflict 运动计划与 P2 motion_cache）。逐车原语只读这些输入、
/// 写独占结果槽位；grant 生命周期不出协调器（视图不含可写引用）。
#[derive(Clone, Copy)]
struct MotionTaskView<'a> {
    read: crate::kernel::phase::StepReadView<'a>,
    waiting_plans: &'a [crate::kernel::waiting::WaitingVehiclePlan],
    waiting_plan_by_vehicle: &'a [Option<std::num::NonZeroU32>],
    conflict_motion_by_vehicle: &'a [Option<crate::kernel::conflict_tick::ConflictMotionPlan>],
    conflict_staged: &'a crate::kernel::conflict::ConflictWorkspace,
    motion_cache: &'a [MotionCacheEntry],
}

impl MotionTaskView<'_> {
    /// waiting_stop_for 的冻结暂存视图版：读取本拍 Waiting 裁决暂存与编译路线；
    /// 检查与错误变体与 StepWorkspace::waiting_stop_for 逐行一致。
    fn waiting_stop_for(
        self,
        state: &VehicleState,
    ) -> Result<Option<crate::kernel::waiting::WaitingStopConstraint>, StepError> {
        let Some(plan) = self
            .waiting_plan_by_vehicle
            .get(state.handle.index() as usize)
            .copied()
            .flatten()
            .and_then(|index| self.waiting_plans.get(index.get() as usize - 1).copied())
            .filter(|plan| plan.vehicle == state.handle)
        else {
            return Ok(None);
        };
        let Some(stop_hop) = plan.stop_hop else {
            return Ok(None);
        };
        let compiled = self
            .read
            .compiled_route(state.route)
            .ok_or(StepError::WaitingInvariantViolation)?;
        let stop_index = usize::try_from(stop_hop)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(StepError::WaitingInvariantViolation)?;
        let distance = crate::kernel::tables::distance_to_occurrence_start(
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            state.route_edge_index as usize,
            state.progress_mm,
            stop_index,
        )
        .ok_or(StepError::WaitingInvariantViolation)?;
        Ok(Some(crate::kernel::waiting::WaitingStopConstraint {
            distance,
            hop: stop_hop,
        }))
    }

    /// conflict_stop_for 的冻结暂存视图版：P4 裁决 motion plan + 拍初
    /// reservation（committed 合并层）+ 静态编译路线；错误变体逐行一致。
    ///
    /// 几何上界只决定能否省掉某一类搜索。给不出证明、进度和余量都为 0，或索引缺行时，
    /// 仍走下面的完整查询。两类都够不着时不读取授权。
    fn conflict_stop_for(
        self,
        state: &VehicleState,
        delta_s: f32,
    ) -> Result<Option<crate::kernel::waiting::WaitingStopConstraint>, StepError> {
        let compiled = self
            .read
            .compiled_route(state.route)
            .ok_or(StepError::ConflictInvariantViolation)?;
        let reach = self
            .read
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
            .and_then(|profile| {
                MotionReach::from_tick(state.speed_mm_s, profile.max_accel(), delta_s)
            });
        let (skip_conflict, skip_waiting) = unreachable_barrier_classes(compiled, state, reach);
        if skip_conflict && skip_waiting {
            return Ok(None);
        }
        let grant_hop = self
            .conflict_motion_by_vehicle
            .get(state.handle.index() as usize)
            .copied()
            .flatten()
            .filter(|plan| plan.outcome == crate::ConflictDecisionOutcome::Granted)
            .map(|plan| plan.gate_hop);
        let owned_hop = crate::kernel::conflict::ConflictRead::new(
            &self.read.committed.conflict,
            &self.read.derived.conflict,
            self.conflict_staged,
        )
        .reservation(state.handle)
        .map(|reservation| reservation.passage_range().admission_gate_hop());
        let first_hop = if state.progress_mm == 0 && state.carry_um == 0 {
            state.route_edge_index.saturating_sub(1)
        } else {
            state.route_edge_index
        };
        let held_waiting_hop = state.waiting_membership.and_then(|member| {
            let index = compiled
                .waiting
                .partition_point(|entry| entry.release_hop < member.release_hop);
            compiled
                .waiting
                .get(index)
                .filter(|entry| {
                    entry.release_hop == member.release_hop && entry.zone == member.waiting_zone
                })
                .map(|entry| entry.entry_hop)
        });
        let authorized = |hop| [grant_hop, owned_hop, held_waiting_hop].contains(&Some(hop));
        // 直接查询有序资源出现项，不扫描不需要资源的普通 Gate。
        // 同一 admission Gate 的多个 passage 用 partition_point 整段跳过。
        let mut minimum = first_hop;
        let conflict = if skip_conflict {
            None
        } else {
            #[cfg(test)]
            note_barrier_query(|counts| counts.conflict_scans += 1);
            loop {
                let index = compiled
                    .conflicts
                    .partition_point(|entry| entry.admission_hop < minimum);
                let Some(entry) = compiled.conflicts.get(index) else {
                    break None;
                };
                if !authorized(entry.admission_hop) {
                    break Some(entry.admission_hop);
                }
                minimum = entry
                    .admission_hop
                    .checked_add(1)
                    .ok_or(StepError::ConflictInvariantViolation)?;
            }
        };
        let waiting = if skip_waiting {
            None
        } else {
            #[cfg(test)]
            note_barrier_query(|counts| counts.waiting_entry_scans += 1);
            let waiting = compiled
                .waiting
                .partition_point(|entry| entry.entry_hop < first_hop);
            compiled.waiting[waiting..]
                .iter()
                .find(|entry| !authorized(entry.entry_hop))
                .map(|entry| entry.entry_hop)
        };
        // 申请资格不能决定运动屏障。既有权威和本拍 grant 只授权各自的 Gate。
        let Some(hop) = conflict.into_iter().chain(waiting).min() else {
            return Ok(None);
        };
        let distance = crate::kernel::tables::distance_to_occurrence_start(
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            state.route_edge_index as usize,
            state.progress_mm,
            hop as usize + 1,
        )
        .ok_or(StepError::ConflictInvariantViolation)?;
        Ok(Some(crate::kernel::waiting::WaitingStopConstraint {
            distance,
            hop,
        }))
    }

    /// P5 逐车运动原语：Parking binding 校验 → reservation/arrived_before 判定
    /// → waiting/conflict stop → MotionPreview 复用证明或重算 → next state 与
    /// 停车到达检查。逐车检查次序与错误变体保持串行原义（checkpoint-map
    /// §4 #5-14）；读取拍初 C(T)、P2 motion_cache 与已冻结的 P4 裁决暂存，
    /// 不写入任何共享状态。融合与分发路径调用同一原语；到达观察仅为候选，
    /// 真实预留由协调器在该车原逻辑位置执行。
    fn vehicle_motion_outcome(
        self,
        state: &VehicleState,
        active_index: usize,
        delta_s: f32,
    ) -> Result<VehicleMotionOutcome, StepError> {
        #[cfg(test)]
        if motion_injection::nonfinite_injected(self.read.binding.world_id, active_index) {
            return Err(StepError::NonFiniteMotion);
        }
        let handle = state.handle;
        let parking_binding = self.read.committed.parking.binding(handle);
        if !self
            .read
            .parking_state_valid_with_binding(handle, *state, parking_binding)
        {
            return Err(StepError::ParkingInvariantViolation);
        }
        let reservation = match parking_binding {
            Some(ParkingBinding::Reserved(reservation)) => Some(reservation),
            Some(ParkingBinding::Occupied(_)) => {
                return Err(StepError::ParkingInvariantViolation);
            }
            None => None,
        };
        let arrived_before = reservation
            .is_some_and(|reservation| self.read.parking_arrived_for(*state, reservation));
        let waiting_stop = self.waiting_stop_for(state)?;
        let conflict_stop = self.conflict_stop_for(state, delta_s)?;
        let cached = self
            .motion_cache
            .get(active_index)
            .filter(|entry| entry.vehicle == handle);
        let reused = cached
            .and_then(|entry| entry.preview)
            .and_then(|preview| preview.reuse(waiting_stop, conflict_stop));
        #[cfg(test)]
        let cache_served = reused.is_some();
        let next = reused
            .or_else(|| {
                self.read.advance_active_vehicle_with_parking_binding(
                    *state,
                    delta_s,
                    waiting_stop,
                    conflict_stop,
                    parking_binding,
                    cached.and_then(|entry| entry.horizon),
                )
            })
            .ok_or(StepError::NonFiniteMotion)?;
        // 复用/重算分类口径不变：成功求值后、到达检查前记账（与提取前同位）。
        #[cfg(test)]
        note_motion_cache_use(cache_served);
        let arrival = if let Some(reservation) = reservation {
            if next.status != VehicleStatus::Active {
                return Err(StepError::ParkingInvariantViolation);
            }
            (!arrived_before && self.read.parking_arrived_for(next, reservation)).then(|| {
                ParkingArrivalObservation {
                    vehicle: handle,
                    target: reservation.target(),
                }
            })
        } else {
            None
        };
        Ok(VehicleMotionOutcome { next, arrival })
    }
}

#[cfg(test)]
impl crate::kernel::phase::StepWorkspace<'_> {
    /// 生产 `MotionTaskView::conflict_stop_for`。测试用它对照未裁剪的精确查询。
    pub(crate) fn motion_conflict_stop_for(
        &self,
        state: &VehicleState,
        delta_s: f32,
    ) -> Result<Option<crate::kernel::waiting::WaitingStopConstraint>, StepError> {
        MotionTaskView {
            read: self.read_view(),
            waiting_plans: &self.workspace.waiting_plans,
            waiting_plan_by_vehicle: &self.workspace.waiting_plan_by_vehicle,
            conflict_motion_by_vehicle: &self.workspace.conflict_motion_by_vehicle,
            conflict_staged: &self.workspace.conflict,
            motion_cache: &self.workspace.motion_cache,
        }
        .conflict_stop_for(state, delta_s)
    }
}

/// P5 融合路径：Active 序上逐车直调原语并就地规范消费，不物化输入表
///（可选暂存回退时的同领域原语执行形态，checkpoint-map §6.2-4）。
fn prepare_motion_fused(
    workspace: &crate::kernel::state::TickWorkspace,
    read: crate::kernel::phase::StepReadView<'_>,
    delta_s: f32,
    parking_arrivals: &mut Vec<ParkingArrivalObservation>,
    updates: &mut Vec<(usize, VehicleState)>,
) -> Result<(), StepError> {
    let view = MotionTaskView {
        read,
        waiting_plans: &workspace.waiting_plans,
        waiting_plan_by_vehicle: &workspace.waiting_plan_by_vehicle,
        conflict_motion_by_vehicle: &workspace.conflict_motion_by_vehicle,
        conflict_staged: &workspace.conflict,
        motion_cache: &workspace.motion_cache,
    };
    for (active_index, handle) in view.read.derived.active_order.iter().copied().enumerate() {
        let Some(state) = view.read.vehicle_state(handle) else {
            continue;
        };
        debug_assert_eq!(state.status, VehicleStatus::Active);
        let outcome = view.vehicle_motion_outcome(state, active_index, delta_s)?;
        if let Some(arrival) = outcome.arrival {
            push_parking_arrival(parking_arrivals, arrival)?;
        }
        let slot = usize::try_from(handle.index()).expect("vehicle index fits usize");
        updates.push((slot, outcome.next));
    }
    Ok(())
}

/// P5 分发路径：输入发现（checked 预留，失败/注入回退融合）→ 任务独占
/// 结果槽位、只读冻结视图计算 → 完整 join → 协调器按 Active 序规范消费
///（该车失败在此处返回；到达观察在此处真实预留；然后 updates 接纳）。
/// 首错来自规范消费，任务侧 first_error 原子仅作更晚块跳过的调度提示。
#[allow(clippy::too_many_arguments)]
fn prepare_motion_dispatched(
    workspace: &mut crate::kernel::state::TickWorkspace,
    read: crate::kernel::phase::StepReadView<'_>,
    execution: &crate::kernel::execution::ExecutionResources,
    delta_s: f32,
    parking_arrivals: &mut Vec<ParkingArrivalObservation>,
    updates: &mut Vec<(usize, VehicleState)>,
) -> Result<(), StepError> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let view = MotionTaskView {
        read,
        waiting_plans: &workspace.waiting_plans,
        waiting_plan_by_vehicle: &workspace.waiting_plan_by_vehicle,
        conflict_motion_by_vehicle: &workspace.conflict_motion_by_vehicle,
        conflict_staged: &workspace.conflict,
        motion_cache: &workspace.motion_cache,
    };
    let inputs = &mut workspace.motion_inputs;
    inputs.clear();
    #[cfg(test)]
    let input_injected = motion_injection::input_reserve_injected();
    #[cfg(not(test))]
    let input_injected = false;
    // 上界用 Active 投影：输入只收 Active 三元组（§4 #5 同谓词）。
    if inputs
        .try_reserve(view.read.derived.active_order.len())
        .is_err()
        || input_injected
    {
        // 回退拍计入 slot_fallback，与 fused/dispatched 互斥。
        #[cfg(test)]
        count_motion_path(|counts| counts.slot_fallback += 1);
        return prepare_motion_fused(workspace, view.read, delta_s, parking_arrivals, updates);
    }
    for (active_index, handle) in view.read.derived.active_order.iter().copied().enumerate() {
        let Some(state) = view.read.vehicle_state(handle) else {
            continue;
        };
        inputs.push((handle, active_index, *state));
    }
    let workload = inputs.len();
    #[cfg(test)]
    let forced = motion_dispatch_forced();
    #[cfg(not(test))]
    let forced = false;
    if workload < MOTION_DISPATCH_MIN_ACTIVE && !(forced && workload > 0) {
        #[cfg(test)]
        count_motion_path(|counts| counts.fused += 1);
        return prepare_motion_fused(workspace, view.read, delta_s, parking_arrivals, updates);
    }
    let slots = &mut workspace.motion_slots;
    slots.clear();
    #[cfg(test)]
    let slot_injected = motion_injection::slot_reserve_injected();
    #[cfg(not(test))]
    let slot_injected = false;
    if slots.try_reserve(workload).is_err() || slot_injected {
        // 可选并行暂存预留失败：退回同一领域原语的融合求值，不新增领域错误。
        #[cfg(test)]
        count_motion_path(|counts| counts.slot_fallback += 1);
        return prepare_motion_fused(workspace, view.read, delta_s, parking_arrivals, updates);
    }
    // 调度统计在可选槽位预留成功后才登记：回退拍只计 slot_fallback，
    // 与 dispatched/fused 互斥。
    #[cfg(test)]
    count_motion_path(|counts| counts.dispatched += 1);
    slots.resize(workload, crate::kernel::execution::DispatchSlot::Pending);
    // 块数 = 线程数 × 2 与活动数取较小者；语义中立（与 P2 同默认值）。
    let chunk_count = execution
        .dispatch_threads()
        .saturating_mul(2)
        .clamp(1, workload);
    let chunk_size = workload.div_ceil(chunk_count).max(1);
    let first_error = AtomicUsize::new(usize::MAX);
    // 块级计数诊断按协调器开关分配/记录；任务内以捕获的布尔为准（辅助
    // 线程读不到协调器线程本地开关，避免漏记）。
    #[cfg(test)]
    let diagnostics = MOTION_DIAGNOSTICS.with(std::cell::Cell::get);
    #[cfg(not(test))]
    #[allow(unused_variables)]
    let diagnostics = false;
    #[cfg(test)]
    let chunk_records = diagnostics.then(|| {
        (0..chunk_count)
            .map(|_| MotionWorkChunkRecord::default())
            .collect::<Vec<_>>()
    });
    #[cfg(test)]
    let tls_baseline = diagnostics.then(motion_tls_snapshot);
    let compute =
        |_chunk_view: crate::kernel::phase::StepReadView<'_>,
         start: usize,
         chunk: &mut [crate::kernel::execution::DispatchSlot<VehicleMotionOutcome>]| {
            #[cfg(test)]
            let chunk_baseline = diagnostics.then(motion_tls_snapshot);
            for (offset, slot) in chunk.iter_mut().enumerate() {
                let index = start + offset;
                let (_handle, active_index, state) = workspace.motion_inputs[index];
                match view.vehicle_motion_outcome(&state, active_index, delta_s) {
                    Ok(outcome) => {
                        *slot = crate::kernel::execution::DispatchSlot::Done(Ok(outcome));
                    }
                    Err(error) => {
                        first_error.fetch_min(index, Ordering::Relaxed);
                        *slot = crate::kernel::execution::DispatchSlot::Done(Err(error));
                        break;
                    }
                }
            }
            #[cfg(test)]
            if let (Some(records), Some(baseline)) = (&chunk_records, chunk_baseline) {
                records[start / chunk_size].store_deltas(baseline);
            }
        };
    let dispatch_stats =
        execution.try_for_each_chunk(view.read, slots, &first_error, chunk_size, compute);
    #[cfg(test)]
    {
        if let (Some(baseline), Some(records)) = (tls_baseline, &chunk_records) {
            aggregate_motion_tls(baseline, records);
        }
        crate::kernel::execution::note_last_dispatch_stats(dispatch_stats);
        LAST_MOTION_DISPATCH_STATS.with(|cell| cell.set(Some(dispatch_stats)));
        if let Some(position) = MOTION_SLOT_GAP.with(std::cell::Cell::get)
            && let Some(slot) = slots.get_mut(position)
        {
            // 完成前沿不变量注入：首错之前出现未计算槽位，协调器须检出而非成功。
            *slot = crate::kernel::execution::DispatchSlot::Pending;
        }
    }
    #[cfg(not(test))]
    let _ = dispatch_stats;
    for ((vehicle, _active_index, _state), slot) in workspace.motion_inputs.iter().zip(slots.iter())
    {
        match slot {
            crate::kernel::execution::DispatchSlot::Done(Ok(outcome)) => {
                if let Some(arrival) = outcome.arrival {
                    push_parking_arrival(parking_arrivals, arrival)?;
                }
                let slot = usize::try_from(vehicle.index()).expect("vehicle index fits usize");
                updates.push((slot, outcome.next));
            }
            crate::kernel::execution::DispatchSlot::Done(Err(error)) => {
                // 完整 join 后按 Active 序规范消费首错（不做最小下标预扫描）。
                return Err(*error);
            }
            crate::kernel::execution::DispatchSlot::Pending
            | crate::kernel::execution::DispatchSlot::Skipped => {
                // 完成前沿不变量违例：首错之前的槽位缺失/跳过/旧 attempt
                // 回报不得视为成功或无结果。
                return Err(StepError::ConflictInvariantViolation);
            }
        }
    }
    Ok(())
}

impl crate::kernel::phase::StepWorkspace<'_> {
    pub(crate) fn stage_vehicle_transitions(
        &mut self,
        delta_s: f32,
        tick_index: u64,
        time_ms: u64,
        updates: &mut Vec<(usize, VehicleState)>,
        execution: Option<&crate::kernel::execution::ExecutionResources>,
    ) -> Result<Vec<ParkingArrivalObservation>, StepError> {
        #[cfg(test)]
        let conflict_timer =
            super::performance_profile::begin(super::performance_profile::Stage::ConflictPrepare);
        self.prepare_conflict_step(delta_s, tick_index, execution)?;
        #[cfg(test)]
        drop(conflict_timer);
        #[cfg(test)]
        injected_step_failure(StepFailpoint::AfterGrants)?;
        let mut parking_arrivals = Vec::new();
        #[cfg(test)]
        let motion_timer =
            super::performance_profile::begin(super::performance_profile::Stage::MotionLoop);
        // 与 read_view 相同的字段级只读投影：持有 committed/derived 借用期间
        // 仍可独占 workspace 字段完成发现、槽位写入与规范消费。
        let read = crate::kernel::phase::StepReadView {
            binding: self.binding,
            committed: &self.committed,
            derived: &self.derived,
        };
        match execution {
            Some(resources @ crate::kernel::execution::ExecutionResources::Pool(_))
                if !motion_dispatch_fuse_forced() =>
            {
                prepare_motion_dispatched(
                    self.workspace,
                    read,
                    resources,
                    delta_s,
                    &mut parking_arrivals,
                    updates,
                )?;
            }
            _ => {
                #[cfg(test)]
                count_motion_path(|counts| counts.fused += 1);
                prepare_motion_fused(
                    self.workspace,
                    read,
                    delta_s,
                    &mut parking_arrivals,
                    updates,
                )?;
            }
        }
        #[cfg(test)]
        drop(motion_timer);
        #[cfg(test)]
        let waiting_timer =
            super::performance_profile::begin(super::performance_profile::Stage::WaitingFinalize);
        self.finalize_waiting_step(updates)?;
        #[cfg(test)]
        drop(waiting_timer);
        // 决策和运动使用拍初信号；资格与日志必须描述下一提交时刻。
        #[cfg(test)]
        let signal_timer =
            super::performance_profile::begin(super::performance_profile::Stage::Signals);
        crate::kernel::world::fill_signal_aspects(
            &self.binding.revision,
            time_ms,
            &mut self.workspace.next_signal_aspects,
        );
        #[cfg(test)]
        drop(signal_timer);
        #[cfg(test)]
        let conflict_timer =
            super::performance_profile::begin(super::performance_profile::Stage::ConflictFinalize);
        self.finalize_conflict_step(updates)?;
        #[cfg(test)]
        drop(conflict_timer);
        #[cfg(test)]
        let output_timer =
            super::performance_profile::begin(super::performance_profile::Stage::WaitingOutputs);
        self.finalize_waiting_outputs(updates, tick_index)?;
        self.workspace.motion_cache.clear();
        crate::kernel::entry_frontier::classify_pending(self, delta_s, updates)?;
        #[cfg(test)]
        drop(output_timer);
        #[cfg(test)]
        injected_step_failure(StepFailpoint::AfterTransitions)?;
        Ok(parking_arrivals)
    }

    /// 判断该机动门对指定车辆配置是否拒绝并停车。
    pub(crate) fn gate_is_restrictive(
        &self,
        gate: laneflow_static_contract::ManeuverGateOrdinal,
        profile: VehicleProfileOrdinal,
    ) -> bool {
        self.read_view().gate_is_restrictive(gate, profile)
    }

    /// 按当前已提交信号求值机动门的策略决定。
    pub(crate) fn gate_policy_decision(
        &self,
        gate: laneflow_static_contract::ManeuverGateOrdinal,
        profile: VehicleProfileOrdinal,
    ) -> crate::GatePolicyDecision {
        self.read_view().gate_policy_decision(gate, profile)
    }
}

fn si_meters(mm: u32) -> f32 {
    mm as f32 / 1_000.0
}

fn si_speed(mm_s: u32) -> f32 {
    mm_s as f32 / 1_000.0
}

/// 本拍运动位移的保守上界，单位毫米。只用来决定能否跳过停止查询。
#[derive(Clone, Copy, Debug)]
pub(crate) struct MotionReach {
    millimeters: f64,
}

impl MotionReach {
    /// 拍初速度、最大加速度和步长能证明的位移上界。
    ///
    /// 超出已证明范围时返回 `None`，调用方改做完整查询。50 mm 覆盖舍入和亚毫米余量。
    pub(crate) fn from_tick(speed_mm_s: u32, max_accel_m_s2: f32, delta_s: f32) -> Option<Self> {
        let dt = f64::from(delta_s);
        let accel = f64::from(max_accel_m_s2);
        if !(dt.is_finite() && (0.004..=1.0).contains(&dt)) {
            return None;
        }
        if !(accel.is_finite() && (0.5..=50.0).contains(&accel)) {
            return None;
        }
        if speed_mm_s > 100_000 {
            return None;
        }
        // 500 = 0.5 × 1_000，把 m/s² 的半加速度项换成毫米。
        let millimeters = f64::from(speed_mm_s) * dt + 500.0 * accel * dt * dt + 50.0;
        millimeters.is_finite().then_some(Self { millimeters })
    }

    /// 距离严格大于上界时，该停止位置本拍不可能收紧位移。
    pub(crate) fn excludes(self, distance_mm: u32) -> bool {
        f64::from(distance_mm) > self.millimeters
    }

    /// `None` 表示没有这类屏障。`BeyondFinite` 用 `u32::MAX - progress` 做保守下界。
    fn class_is_unreachable(
        self,
        distance_from_start: Option<BoundedDistance>,
        progress_mm: u32,
    ) -> bool {
        match distance_from_start {
            None => true,
            Some(BoundedDistance::Finite(distance_mm)) => {
                self.excludes(distance_mm.saturating_sub(progress_mm))
            }
            Some(BoundedDistance::BeyondFinite) => {
                self.excludes(u32::MAX.saturating_sub(progress_mm))
            }
        }
    }
}

/// 进度和余量都为 0、上界无效或索引缺行时，两类都不跳过。
fn unreachable_barrier_classes(
    compiled: &CompiledRoute,
    state: &VehicleState,
    reach: Option<MotionReach>,
) -> (bool, bool) {
    if state.progress_mm == 0 && state.carry_um == 0 {
        return (false, false);
    }
    let Some(reach) = reach else {
        return (false, false);
    };
    let Some(row) = compiled
        .nearest_motion_barriers
        .get(state.route_edge_index as usize)
    else {
        return (false, false);
    };
    (
        reach.class_is_unreachable(row.conflict_from_occurrence_start, state.progress_mm),
        reach.class_is_unreachable(row.waiting_from_occurrence_start, state.progress_mm),
    )
}

fn finite_meters(distance: BoundedDistance) -> Option<f32> {
    match distance {
        BoundedDistance::Finite(mm) => Some(si_meters(mm)),
        BoundedDistance::BeyondFinite => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StopAttribution {
    SignalStop,
    ParkingStop,
    RouteEnd,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SelectedStop {
    distance: BoundedDistance,
    attribution: StopAttribution,
}

/// 数值更近者优先；完全同值时调用顺序编码
/// `SignalStop -> ParkingStop -> RouteEnd` 的归因权威。
fn select_movement_stop(
    signal: Option<BoundedDistance>,
    parking: Option<BoundedDistance>,
    route_end: BoundedDistance,
) -> SelectedStop {
    let mut selected = SelectedStop {
        distance: route_end,
        attribution: StopAttribution::RouteEnd,
    };
    if let Some(distance) = parking {
        let candidate = SelectedStop {
            distance,
            attribution: StopAttribution::ParkingStop,
        };
        if stop_is_nearer_or_equal(candidate.distance, selected.distance) {
            selected = candidate;
        }
    }
    if let Some(distance) = signal {
        let candidate = SelectedStop {
            distance,
            attribution: StopAttribution::SignalStop,
        };
        if stop_is_nearer_or_equal(candidate.distance, selected.distance) {
            selected = candidate;
        }
    }
    selected
}

fn stop_is_nearer_or_equal(candidate: BoundedDistance, current: BoundedDistance) -> bool {
    match (candidate, current) {
        (BoundedDistance::Finite(candidate), BoundedDistance::Finite(current)) => {
            candidate <= current
        }
        (BoundedDistance::Finite(_), BoundedDistance::BeyondFinite)
        | (BoundedDistance::BeyondFinite, BoundedDistance::BeyondFinite) => true,
        (BoundedDistance::BeyondFinite, BoundedDistance::Finite(_)) => false,
    }
}

/// 求解器本拍从前车快照空隙承认的硬房间。`hard_room_mm` 与新鲜摆放共用这条整数规则。
pub(crate) fn snapshot_leader_room_mm(gap_mm: i64, min_gap_mm: u32) -> u32 {
    let leftover = gap_mm.saturating_sub(i64::from(min_gap_mm));
    if leftover <= 0 {
        0
    } else {
        u32::try_from(leftover).unwrap_or(u32::MAX)
    }
}

/// 本拍硬约束。`BeyondFinite` 路终/停车距离不参与包络；Finite 侧保持 `u32`，不上 `u64`。
fn hard_room_mm(
    leader_gap: Option<i64>,
    min_gap_mm: u32,
    signal_stop: Option<BoundedDistance>,
    route_end: BoundedDistance,
    edge_length_mm: u32,
    progress_mm: u32,
    hop_permitted: bool,
) -> u32 {
    let mut room = u32::MAX;
    if let Some(gap) = leader_gap {
        room = room.min(snapshot_leader_room_mm(gap, min_gap_mm));
    }
    if let Some(BoundedDistance::Finite(stop)) = signal_stop {
        room = room.min(stop);
    }
    if let BoundedDistance::Finite(remaining) = route_end {
        room = room.min(remaining);
    }
    if !hop_permitted {
        room = room.min(edge_length_mm.saturating_sub(progress_mm));
    }
    room
}

fn leader_gap_m(gap: Option<i64>) -> Option<f32> {
    gap.map(|gap| if gap <= 0 { 0.0 } else { gap as f32 / 1_000.0 })
}

#[allow(clippy::too_many_arguments)]
fn si_comfort_travel(
    speed_mm_s: u32,
    desired_mm_s: u32,
    leader_gap: Option<i64>,
    profile: VehicleProfileView,
    route_end: BoundedDistance,
    signal_stop: Option<BoundedDistance>,
    compiled: &CompiledRoute,
    lengths: &[u32],
    speed_limits: &[u32],
    cursor: usize,
    progress_mm: u32,
    delta_s: f32,
) -> Option<(f32, f32)> {
    let speed = si_speed(speed_mm_s);
    let desired = si_speed(desired_mm_s);
    let leader_m = leader_gap_m(leader_gap);
    let min_gap_m = si_meters(profile.min_gap_mm());
    let envelope = speed_limit_path_envelope(
        compiled.edges.as_slice(),
        lengths,
        speed_limits,
        cursor,
        progress_mm,
        delta_s,
    )?;
    let (mut travel, mut next_speed) = iidm_travel(speed, desired, leader_m, profile, delta_s)?;
    travel = clamp_si_travel(
        travel,
        leader_m,
        min_gap_m,
        signal_stop,
        route_end,
        envelope,
    );
    if !travel.is_finite() || !next_speed.is_finite() {
        return None;
    }
    next_speed = constrain_upcoming_speed_limits(
        speed,
        next_speed,
        delta_s,
        compiled,
        cursor,
        progress_mm,
        profile.comfort_decel(),
        profile.emergency_decel(),
    )?;
    travel = ((speed + next_speed) * 0.5 * delta_s).max(0.0);
    travel = clamp_si_travel(
        travel,
        leader_m,
        min_gap_m,
        signal_stop,
        route_end,
        envelope,
    );
    travel = clamp_travel_to_speed_down_boundary(
        travel,
        speed,
        next_speed,
        delta_s,
        compiled,
        cursor,
        progress_mm,
    )?;
    Some((travel.max(0.0), next_speed.max(0.0)))
}

fn clamp_si_travel(
    mut travel: f32,
    leader_m: Option<f32>,
    min_gap_m: f32,
    signal_stop: Option<BoundedDistance>,
    route_end: BoundedDistance,
    envelope: f32,
) -> f32 {
    if let Some(gap) = leader_m {
        travel = travel.min((gap - min_gap_m).max(0.0));
    }
    if let Some(stop) = signal_stop.and_then(finite_meters) {
        travel = travel.min(stop.max(0.0));
    }
    if let Some(end) = finite_meters(route_end) {
        travel = travel.min(end.max(0.0));
    }
    travel.min(envelope).max(0.0)
}

fn iidm_travel(
    speed: f32,
    desired: f32,
    leader_gap: Option<f32>,
    profile: VehicleProfileView,
    delta_s: f32,
) -> Option<(f32, f32)> {
    iidm_step(
        speed,
        desired,
        leader_gap,
        si_meters(profile.min_gap_mm()),
        profile.time_headway(),
        profile.max_accel(),
        profile.comfort_decel(),
        profile.emergency_decel(),
        delta_s,
    )
}

#[allow(clippy::too_many_arguments)]
fn iidm_step(
    speed: f32,
    desired: f32,
    leader_gap: Option<f32>,
    min_gap_m: f32,
    time_headway: f32,
    accel_max: f32,
    comfort: f32,
    emergency: f32,
    delta_s: f32,
) -> Option<(f32, f32)> {
    if !speed.is_finite() || !desired.is_finite() || delta_s <= 0.0 {
        return None;
    }
    if accel_max <= 0.0 || comfort <= 0.0 || emergency <= 0.0 {
        return None;
    }
    if leader_gap.is_some_and(|gap| gap <= 0.0) {
        return Some((0.0, 0.0));
    }
    let speed_term = if desired <= 0.0 {
        1.0
    } else {
        (speed / desired).max(0.0).powi(4)
    };
    let gap_term = if let Some(gap) = leader_gap {
        let s_star = min_gap_m + speed * time_headway;
        (s_star / gap).max(0.0).powi(2)
    } else {
        0.0
    };
    let accel = accel_max * (1.0 - speed_term - gap_term);
    let next_speed = (speed + accel * delta_s).max(0.0).min(desired.max(0.0));
    let travel = ((speed + next_speed) * 0.5 * delta_s).max(0.0);
    (travel.is_finite() && next_speed.is_finite()).then_some((travel, next_speed))
}

fn speed_limit_path_envelope(
    edges: &[LaneEdgeOrdinal],
    lengths: &[u32],
    speed_limits: &[u32],
    mut index: usize,
    mut progress_mm: u32,
    delta_s: f32,
) -> Option<f32> {
    if delta_s <= 0.0 {
        return None;
    }
    let mut remaining_t = delta_s;
    let mut total = 0.0;
    loop {
        let edge = *edges.get(index)?;
        let length = si_meters(*lengths.get(edge.index())?);
        let limit = si_speed(*speed_limits.get(edge.index())?);
        let leftover = (length - si_meters(progress_mm)).max(0.0);
        if limit <= 0.0 {
            break;
        }
        let cap = limit * remaining_t;
        if cap <= leftover {
            total += cap;
            break;
        }
        total += leftover;
        remaining_t -= leftover / limit;
        if remaining_t <= 0.0 || index + 1 >= edges.len() {
            break;
        }
        index += 1;
        progress_mm = 0;
    }
    total.is_finite().then_some(total.max(0.0))
}

#[allow(clippy::too_many_arguments)]
/// 本世界限速下降转换。不扫剩余边；限速值写在 drop 列，与共享根边热列同形。
fn constrain_upcoming_speed_limits(
    current_speed: f32,
    mut next_speed: f32,
    delta_s: f32,
    compiled: &CompiledRoute,
    cursor: usize,
    progress_mm: u32,
    comfort: f32,
    emergency: f32,
) -> Option<f32> {
    // Twice the candidate's full-stop distance at comfortable deceleration is a
    // conservative window even for a zero-speed target. Keep the original
    // solver inside it: merely testing feasibility at the candidate can change
    // its f32 result by one ULP near a binding limit.
    let constraint_window =
        delta_s * (current_speed + next_speed) + next_speed * next_speed / comfort;
    let cursor_hop = u32::try_from(cursor).ok()?;
    let first = compiled
        .speed_limit_drop
        .partition_point(|drop| drop.from_route_edge_index < cursor_hop);
    for drop in &compiled.speed_limit_drop[first..] {
        let from = usize::try_from(drop.from_route_edge_index).ok()?;
        let limit = si_speed(drop.target_mm_s);
        let to_index = from.checked_add(1)?;
        match distance_to_occurrence_start(
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            cursor,
            progress_mm,
            to_index,
        )? {
            BoundedDistance::BeyondFinite => break,
            BoundedDistance::Finite(0) => {
                next_speed = next_speed.min(limit.max(0.0));
            }
            BoundedDistance::Finite(mm) => {
                let distance = si_meters(mm);
                // Drops are in route order, so every later target is farther.
                // next_speed only decreases; this initial window remains an
                // upper bound after a nearer target has constrained it.
                if distance > constraint_window {
                    break;
                }
                if limit >= next_speed {
                    continue;
                }
                next_speed = cap_next_speed_for_limit(
                    current_speed,
                    next_speed,
                    delta_s,
                    distance,
                    limit,
                    comfort,
                    emergency,
                )?;
            }
        }
    }
    Some(next_speed.max(0.0))
}

fn cap_next_speed_for_limit(
    current_speed: f32,
    next_speed: f32,
    delta_s: f32,
    distance: f32,
    limit: f32,
    comfort: f32,
    emergency: f32,
) -> Option<f32> {
    if let Some(capped) =
        max_next_speed_for_decel(current_speed, next_speed, delta_s, distance, limit, comfort)
    {
        return Some(capped);
    }
    max_next_speed_for_decel(
        current_speed,
        next_speed,
        delta_s,
        distance,
        limit,
        emergency,
    )
    .or(Some(0.0))
}

fn max_next_speed_for_decel(
    current_speed: f32,
    next_speed: f32,
    delta_s: f32,
    distance: f32,
    limit: f32,
    decel: f32,
) -> Option<f32> {
    if decel <= 0.0 || delta_s <= 0.0 {
        return None;
    }
    let limit = limit.max(0.0);
    if 0.5 * current_speed * delta_s > distance {
        return None;
    }
    let linear = ((2.0 * distance / delta_s) - current_speed)
        .min(limit)
        .min(next_speed)
        .max(0.0);
    let b_dt = decel * delta_s;
    let constant = decel * current_speed * delta_s - limit * limit - 2.0 * decel * distance;
    let discriminant = b_dt * b_dt - 4.0 * constant;
    let quadratic = if discriminant >= 0.0 {
        ((-b_dt + discriminant.sqrt()) / 2.0).min(next_speed)
    } else {
        f32::NEG_INFINITY
    };
    let mut best = linear;
    if quadratic > limit
        && speed_down_constraint_holds(current_speed, quadratic, delta_s, distance, limit, decel)
    {
        best = best.max(quadratic);
    }
    speed_down_constraint_holds(current_speed, best, delta_s, distance, limit, decel)
        .then_some(best.min(next_speed).max(0.0))
}

fn speed_down_constraint_holds(
    current_speed: f32,
    next_speed: f32,
    delta_s: f32,
    distance: f32,
    limit: f32,
    decel: f32,
) -> bool {
    let travel = 0.5 * (current_speed + next_speed) * delta_s;
    let braking = (next_speed * next_speed - limit * limit).max(0.0) / (2.0 * decel);
    travel + braking <= distance
}

#[allow(clippy::too_many_arguments)]
fn clamp_travel_to_speed_down_boundary(
    mut travel: f32,
    current_speed: f32,
    next_speed: f32,
    delta_s: f32,
    compiled: &CompiledRoute,
    cursor: usize,
    progress_mm: u32,
) -> Option<f32> {
    let min_travel = 0.5 * current_speed * delta_s;
    let cursor_hop = u32::try_from(cursor).ok()?;
    let first = compiled
        .speed_limit_drop
        .partition_point(|drop| drop.from_route_edge_index < cursor_hop);
    for drop in &compiled.speed_limit_drop[first..] {
        let from = usize::try_from(drop.from_route_edge_index).ok()?;
        let limit = si_speed(drop.target_mm_s);
        let to_index = from.checked_add(1)?;
        let BoundedDistance::Finite(mm) = distance_to_occurrence_start(
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            cursor,
            progress_mm,
            to_index,
        )?
        else {
            break;
        };
        if mm == 0 {
            continue;
        }
        let distance = si_meters(mm);
        // A later route occurrence cannot clamp a travel that already ends
        // before this boundary. Preserve the existing boundary calculation.
        if distance >= travel {
            break;
        }
        if limit >= current_speed || limit >= next_speed {
            continue;
        }
        if min_travel <= distance && travel > distance {
            travel = travel.min(distance);
        }
    }
    Some(travel.max(0.0))
}

fn apply_travel_mm(
    state: &mut VehicleState,
    edges: &[LaneEdgeOrdinal],
    lengths: &[u32],
    mut remaining: u32,
    hop_permitted: impl Fn(usize) -> bool,
) -> Option<()> {
    let mut index = usize::try_from(state.route_edge_index).ok()?;
    // 亚毫米余量也表示前进：已在边界时必须过门，余量才能归属下一条边。
    while remaining > 0 || state.carry_um > 0 {
        let edge = *edges.get(index)?;
        let edge_length = *lengths.get(edge.index())?;
        let leftover = edge_length.saturating_sub(state.progress_mm);
        if remaining < leftover {
            state.progress_mm = state.progress_mm.saturating_add(remaining);
            break;
        }
        remaining -= leftover;
        if !hop_permitted(index) || index + 1 >= edges.len() {
            state.progress_mm = edge_length;
            break;
        }
        index += 1;
        state.progress_mm = 0;
    }
    state.route_edge_index = u32::try_from(index).ok()?;
    Some(())
}

#[cfg(test)]
mod motion_reuse_tests {
    use super::*;
    use crate::kernel::waiting::WaitingStopConstraint;

    #[test]
    fn near_stop_reuses_hard_stops_and_submillimetre_motion_but_not_crossings() {
        let (mut world, follower, leader) =
            crate::kernel::occupancy::tests::zero_progress_merge_fixture();
        let state = VehicleState {
            speed_mm_s: 0,
            carry_um: 0,
            ..world.vehicle(follower).unwrap()
        };
        // 合流前车把 follower 挡在距真实 hop 边界 2 mm 处，远小于跟车窗。
        let stop = WaitingStopConstraint {
            hop: 0,
            distance: BoundedDistance::Finite(2),
        };
        let profile = world
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
            .unwrap();
        let horizon = leader_query_horizon(0, profile, 0.016).unwrap();
        let view = world.state.read_view();
        let preview = view
            .preview_active_vehicle_with_waiting_stop(state, 0.016, None, None)
            .unwrap();
        assert_eq!(preview.next, state);
        assert!(matches!(preview.bounds, MotionBounds::HardStopped));
        assert!(horizon.bumper_gap_mm > 2);
        assert_eq!(
            preview.reuse(None, Some(stop)),
            view.advance_active_vehicle_with_waiting_stop(state, 0.016, None, Some(stop))
        );
        assert_eq!(
            preview.reuse(Some(stop), None),
            view.advance_active_vehicle_with_waiting_stop(state, 0.016, Some(stop), None)
        );

        world.despawn_vehicle(leader).unwrap();
        world.state.ensure_current_occupancy().unwrap();
        let state = VehicleState {
            carry_um: 1,
            ..state
        };
        let view = world.state.read_view();
        let preview = view
            .preview_active_vehicle_with_waiting_stop(state, 0.001, None, None)
            .unwrap();
        let next = preview.next;
        assert_eq!(next.route_edge_index, state.route_edge_index);
        assert_eq!(next.progress_mm, state.progress_mm);
        assert!(next.carry_um > 0);
        assert_ne!(next, state);
        // 整数位置相同仍有亚毫米运动；实际位移证明保留余量和非零速度。
        assert!(matches!(
            preview.bounds,
            MotionBounds::Travel { proposed_mm: 0, .. }
        ));
        assert_eq!(preview.reuse(None, Some(stop)), Some(next));
        assert_eq!(
            Some(next),
            view.advance_active_vehicle_with_waiting_stop(state, 0.001, None, Some(stop))
        );

        let crossing = view
            .preview_active_vehicle_with_waiting_stop(state, 0.1, None, None)
            .unwrap();
        let stopped = view
            .advance_active_vehicle_with_waiting_stop(state, 0.1, None, Some(stop))
            .unwrap();
        assert_ne!(crossing.next, stopped);
        assert!(crossing.reuse(None, Some(stop)).is_none());
    }

    #[test]
    fn reused_motion_matches_full_calculation_across_integer_and_carry_boundaries() {
        let mut world = crate::kernel::waiting::tests::multi_gate_world(1);
        world.state.rebuild_occupancy_index().unwrap();
        let base = world.vehicle(world.state.derived.active_order[0]).unwrap();
        let view = world.state.read_view();
        let compiled = view.compiled_route(base.route).unwrap();
        let lengths = world.traffic().lane_lengths_millimetres();
        let limits = world.traffic().lane_speed_limits_millimetres_per_second();
        let mut reused = 0;
        let mut hard_stopped_reused = 0;
        let mut constrained_fallbacks = 0;
        for cursor in 0..compiled.edges.len() {
            let edge = compiled.edges[cursor];
            let length = lengths[edge.index()];
            let limit = limits[edge.index()];
            for progress in [0, 1, length / 2, length - 1, length] {
                for speed in [0, 1, limit / 2, limit] {
                    for carry in [0, 1, 999] {
                        let state = VehicleState {
                            route_edge_index: cursor as u32,
                            progress_mm: progress,
                            speed_mm_s: speed,
                            carry_um: carry,
                            ..base
                        };
                        let stops: Vec<_> = (cursor..compiled.edges.len() - 1)
                            .map(|hop| WaitingStopConstraint {
                                hop: hop as u32,
                                distance: distance_to_occurrence_start(
                                    &compiled.occurrence_segments,
                                    &compiled.occurrence_offsets,
                                    &compiled.segment_totals,
                                    cursor,
                                    progress,
                                    hop + 1,
                                )
                                .unwrap(),
                            })
                            .collect();
                        for (delta_s, waiting_stop) in
                            [0.001, 0.016, 0.033, 0.1].into_iter().flat_map(|delta_s| {
                                [None, stops.first().copied()].map(|stop| (delta_s, stop))
                            })
                        {
                            let preview = view
                                .preview_active_vehicle_with_waiting_stop(
                                    state,
                                    delta_s,
                                    waiting_stop,
                                    None,
                                )
                                .unwrap();
                            let next = preview.next;
                            for conflict_stop in
                                std::iter::once(None).chain(stops.iter().copied().map(Some))
                            {
                                let expected = view
                                    .advance_active_vehicle_with_waiting_stop(
                                        state,
                                        delta_s,
                                        waiting_stop,
                                        conflict_stop,
                                    )
                                    .unwrap();
                                if let Some(actual) = preview.reuse(waiting_stop, conflict_stop) {
                                    assert_eq!(
                                        actual, expected,
                                        "state={state:?}, delta_s={delta_s}, stop={conflict_stop:?}"
                                    );
                                    reused += 1;
                                    hard_stopped_reused += usize::from(matches!(
                                        preview.bounds,
                                        MotionBounds::HardStopped
                                    ));
                                } else if expected != next {
                                    constrained_fallbacks += 1;
                                }
                            }
                            for changed in
                                std::iter::once(None).chain(stops.iter().copied().map(Some))
                            {
                                if changed == waiting_stop {
                                    continue;
                                }
                                for conflict in
                                    std::iter::once(None).chain(stops.iter().copied().map(Some))
                                {
                                    let actual = preview.reuse(changed, conflict);
                                    if waiting_stop.is_some() {
                                        assert!(actual.is_none());
                                    } else if let Some(actual) = actual {
                                        assert_eq!(
                                            Some(actual),
                                            view.advance_active_vehicle_with_waiting_stop(
                                                state, delta_s, changed, conflict
                                            )
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(reused > 100);
        assert!(hard_stopped_reused > 0);
        assert!(
            constrained_fallbacks > 0,
            "near barriers must exercise an actual change in motion"
        );
    }

    #[test]
    fn motion_bounds_preserve_integer_exhaustion_float_clamps_and_hop_barriers() {
        let world = crate::kernel::waiting::tests::multi_gate_world(1);
        let next = world.vehicle(world.state.derived.active_order[0]).unwrap();
        for (meters, proposed_mm, distance) in [
            (0.000_999_6, 1, 1), // round_um 已把亚毫米结果进位；等于新边界会清余量。
            (0.001_1, 0, 1),     // 整数提案不能代替 SI clamp 的证明。
            (si_meters(u32::MAX), u64::from(u32::MAX) + 1, u32::MAX),
        ] {
            let preview = MotionPreview {
                next,
                waiting_stop: None,
                bounds: MotionBounds::Travel {
                    meters,
                    proposed_mm,
                    committed_mm: 0,
                    exhausted: false,
                },
            };
            let stop = WaitingStopConstraint {
                hop: next.route_edge_index + 1,
                distance: BoundedDistance::Finite(distance),
            };
            assert!(preview.reuse(None, Some(stop)).is_none());
            assert!(preview.reuse(Some(stop), None).is_none());
        }
        let preview = MotionPreview {
            next,
            waiting_stop: None,
            bounds: MotionBounds::Travel {
                meters: 0.000_1,
                proposed_mm: 0,
                committed_mm: 0,
                exhausted: false,
            },
        };
        let stop = WaitingStopConstraint {
            hop: next.route_edge_index,
            distance: BoundedDistance::Finite(1),
        };
        assert_eq!(preview.reuse(None, Some(stop)), Some(next));
        assert!(
            MotionPreview {
                next: VehicleState {
                    route_edge_index: stop.hop + 1,
                    ..next
                },
                ..preview
            }
            .reuse(None, Some(stop))
            .is_none()
        );
        assert!(
            MotionPreview {
                bounds: MotionBounds::Unknown,
                ..preview
            }
            .reuse(None, Some(stop))
            .is_none()
        );
    }
}

#[cfg(test)]
mod preview {
    use super::*;

    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_static_contract::{
        LaneEdgeOrdinal, ParticipantClassOrdinal, VehicleProfileOrdinal,
    };
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    use crate::{RouteHandle, RouteRegisterInput, VehicleHandle, VehicleSpawnInput, WorldConfig};
    fn install_fixture(
        revision: std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
        config: crate::WorldConfig,
    ) -> Result<crate::TrafficWorld, crate::InstallError> {
        let origin = *revision.canonical_origin();
        crate::TrafficWorld::install(
            std::sync::Arc::clone(&revision),
            config,
            crate::ExecutionConfig::new(std::num::NonZeroU32::MIN),
            crate::CommittedNetworkSource::Published {
                reference: crate::PublishedLfcaReference::new(
                    "fixture://in-process",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .expect("non-empty fixture key"),
            },
            0,
            crate::test_policy::selection(&revision),
        )
    }

    fn preview_route(world: &mut TrafficWorld) -> RouteHandle {
        let traffic = world.traffic();
        let mut edges = Vec::new();
        let count = traffic.lane_edge_count();
        for raw in 0..count {
            let edge = LaneEdgeOrdinal::from_raw(raw);
            if traffic.relations().lane_edge_junction(edge).is_some() {
                continue;
            }
            if traffic.relations().stop_line_for_edge(edge).is_some() {
                continue;
            }
            edges.push(edge);
            if let Some(succ) = traffic
                .successors(edge)
                .and_then(|items| items.first().copied())
                && traffic.relations().stop_line_for_edge(succ).is_none()
            {
                edges.push(succ);
            }
            break;
        }
        world
            .register_route(RouteRegisterInput::new(edges))
            .expect("preview route")
    }

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
    );

    #[test]
    fn distant_speed_drop_preserves_candidate_but_near_drop_keeps_solver_rounding() {
        let mut route = CompiledRoute {
            edges: vec![LaneEdgeOrdinal::from_raw(0), LaneEdgeOrdinal::from_raw(1)],
            maneuvers: Vec::new(),
            hop_gate: vec![None, None],
            gate_hops: Vec::new(),
            remaining_to_end: vec![
                BoundedDistance::Finite(75_266),
                BoundedDistance::Finite(1_000),
            ],
            occurrence_segments: vec![0, 0],
            occurrence_offsets: vec![0, 74_266],
            segment_totals: vec![75_266],
            next_controlled: vec![None, None],
            speed_limit_drop: vec![crate::kernel::tables::SpeedLimitDrop {
                from_route_edge_index: 0,
                to_edge: LaneEdgeOrdinal::from_raw(1),
                target_mm_s: 42_074,
            }],
            waiting: Vec::new(),
            conflicts: Vec::new(),
            conflict_gate_ranges: Vec::new(),
            final_conflict_clearance: None,
            nearest_motion_barriers: Vec::new(),
        };
        // A direct feasibility shortcut changes this real f32 boundary by one ULP.
        let candidate = 66.89_f32;
        let near =
            constrain_upcoming_speed_limits(65.761, candidate, 0.033, &route, 0, 0, 18.758, 20.0)
                .unwrap();
        assert_eq!(near.to_bits(), candidate.to_bits() - 1);

        // The same drop beyond twice the full-stop distance cannot bind.
        route.occurrence_offsets[1] = 300_000;
        route.segment_totals[0] = 301_000;
        route.remaining_to_end[0] = BoundedDistance::Finite(301_000);
        let far =
            constrain_upcoming_speed_limits(65.761, candidate, 0.033, &route, 0, 0, 18.758, 20.0)
                .unwrap();
        assert_eq!(far.to_bits(), candidate.to_bits());
    }

    #[test]
    fn preview_follower_constraints() {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).unwrap();
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .unwrap();
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100)).unwrap();
        let route = preview_route(&mut world);
        let profile = world
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .unwrap();
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000 + profile.length_mm() + profile.min_gap_mm() + 2_000,
                0,
            ))
            .unwrap();
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .unwrap();
        world
            .state
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
        let state = world.state.vehicle_state(follower).copied().unwrap();
        let next = world.state.advance_active_vehicle(state, 0.1_f32).unwrap();
        assert!(
            next.progress_mm > state.progress_mm || next.carry_um > state.carry_um,
            "follower should start moving, {} -> {}",
            state.progress_mm,
            next.progress_mm
        );
    }

    fn install_preview_world() -> TrafficWorld {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).unwrap();
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .unwrap();
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100)).unwrap()
    }

    #[test]
    fn failed_signal_boundary_publication_preserves_committed_world_and_retries() {
        let mut world = install_preview_world();
        assert!(!world.state.committed.signal_aspects.is_empty());
        assert_eq!(
            world.state.workspace.next_signal_aspects.len(),
            world.state.committed.signal_aspects.len()
        );
        let boundary = (1..6_000)
            .find(|tick| {
                world.state.committed.time_ms = (tick - 1) * 100;
                world.state.refresh_signals();
                crate::kernel::world::fill_signal_aspects(
                    &world.state.binding.revision,
                    tick * 100,
                    &mut world.state.workspace.next_signal_aspects,
                );
                world.state.committed.signal_aspects != world.state.workspace.next_signal_aspects
            })
            .expect("fixture crosses an ordinary signal phase");
        world.state.committed.tick_index = boundary - 1;
        let before = world.capture_snapshot().unwrap();
        let signals = world.state.committed.signal_aspects.clone();
        let next_signals = world.state.workspace.next_signal_aspects.clone();
        STEP_FAILPOINT.with(|failpoint| failpoint.set(Some(StepFailpoint::AfterTransitions)));
        assert_eq!(
            world.step(TickInput::new(100)),
            Err(StepError::ParkingObservationAllocFailed)
        );
        assert_eq!(world.capture_snapshot().unwrap(), before);
        assert_eq!(world.state.committed.signal_aspects, signals);
        assert_eq!(world.state.workspace.next_signal_aspects, next_signals);
        world.step(TickInput::new(100)).unwrap();
        assert_eq!(world.state.committed.signal_aspects, next_signals);
        assert_eq!(world.time_ms(), boundary * 100);
        assert!(world.state.conflict_state_valid());
    }

    #[test]
    fn successful_ticks_reuse_preallocated_scratch() {
        let mut world = install_preview_world();
        let route = preview_route(&mut world);
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .unwrap();
        world.step(TickInput::new(100)).unwrap();
        let next_cap = world.state.workspace.next_states.capacity();
        let live_cap = world.state.committed.live_order.capacity();
        let vehicle_cap = world.state.committed.vehicles.capacity();
        let occupancy_records = world.state.derived.occupancy.records_capacity();
        let occupancy_scratch = world.state.workspace.occupancy_scratch.capacity();
        let occupancy_offsets = world.state.derived.occupancy.offsets_capacity();
        let occupancy_suffix = world.state.derived.occupancy.suffix_min_lo_capacity();
        let occupancy_second = world.state.derived.occupancy.suffix_second_lo_capacity();
        for _ in 0..16 {
            world.step(TickInput::new(100)).unwrap();
            assert_eq!(world.state.workspace.next_states.capacity(), next_cap);
            assert_eq!(world.state.committed.live_order.capacity(), live_cap);
            assert_eq!(world.state.committed.vehicles.capacity(), vehicle_cap);
            assert_eq!(
                world.state.derived.occupancy.records_capacity(),
                occupancy_records
            );
            assert_eq!(
                world.state.workspace.occupancy_scratch.capacity(),
                occupancy_scratch
            );
            assert_eq!(
                world.state.derived.occupancy.offsets_capacity(),
                occupancy_offsets
            );
            assert_eq!(
                world.state.derived.occupancy.suffix_min_lo_capacity(),
                occupancy_suffix
            );
            assert_eq!(
                world.state.derived.occupancy.suffix_second_lo_capacity(),
                occupancy_second
            );
        }
    }

    #[test]
    fn step_error_is_copy_without_diagnostic_allocation() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<StepError>();
        assert!(
            std::mem::size_of::<StepError>() <= 32,
            "StepError must stay a small Copy code, size={}",
            std::mem::size_of::<StepError>()
        );
    }

    #[test]
    fn overflow_step_leaves_committed_time_unchanged() {
        let mut world = install_preview_world();
        world.state.committed.tick_index = u64::MAX;
        let time = world.state.committed.time_ms;
        assert_eq!(world.step(TickInput::new(100)), Err(StepError::Overflow));
        assert_eq!(world.state.committed.tick_index, u64::MAX);
        assert_eq!(world.state.committed.time_ms, time);
    }

    #[test]
    fn non_finite_motion_after_staging_does_not_commit_earlier_vehicles() {
        let mut world = install_preview_world();
        let route = preview_route(&mut world);
        let profile = world
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .unwrap();
        let first = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000 + profile.length_mm() + profile.min_gap_mm() + 2_000,
                0,
            ))
            .unwrap();
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .unwrap();
        let before_progress = world.state.vehicle_state(first).unwrap().progress_mm;
        let before_tick = world.state.committed.tick_index;
        assert_eq!(
            world.step(TickInput::new(50)),
            Err(StepError::DeltaMismatch {
                expected_delta_time_ms: 100,
                actual_delta_time_ms: 50,
            })
        );
        assert_eq!(world.state.committed.tick_index, before_tick);
        assert_eq!(
            world.state.vehicle_state(first).unwrap().progress_mm,
            before_progress
        );
        assert_eq!(world.state.committed.time_ms, 0);
    }

    fn travel_state(route_edge_index: u32, progress_mm: u32) -> VehicleState {
        VehicleState {
            handle: VehicleHandle::new(0, 0),
            profile: VehicleProfileOrdinal::from_raw(0),
            class: ParticipantClassOrdinal::from_raw(0),
            route: RouteHandle::new(0, 0),
            route_edge_index,
            progress_mm,
            carry_um: 0,
            speed_mm_s: 0,
            length_mm: 4_500,
            status: VehicleStatus::Active,
            maneuver_traversal: None,
            waiting_membership: None,
        }
    }

    #[test]
    fn apply_travel_hops_when_remaining_equals_leftover_and_hop_is_permitted() {
        let edges = [LaneEdgeOrdinal::from_raw(0), LaneEdgeOrdinal::from_raw(1)];
        let lengths = [1_000, 2_000];
        let mut state = travel_state(0, 500);
        apply_travel_mm(&mut state, &edges, &lengths, 500, |index| {
            index + 1 < edges.len()
        })
        .unwrap();
        assert_eq!(state.route_edge_index, 1);
        assert_eq!(state.progress_mm, 0);
    }

    #[test]
    fn apply_travel_stays_at_length_when_remaining_equals_leftover_and_hop_is_denied() {
        let edges = [LaneEdgeOrdinal::from_raw(0), LaneEdgeOrdinal::from_raw(1)];
        let lengths = [1_000, 2_000];
        let mut state = travel_state(0, 500);
        apply_travel_mm(&mut state, &edges, &lengths, 500, |_| false).unwrap();
        assert_eq!(state.route_edge_index, 0);
        assert_eq!(state.progress_mm, 1_000);
    }

    #[test]
    fn hard_stop_clears_carry_um() {
        let mut world = install_preview_world();
        let route = preview_route(&mut world);
        let profile = world
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .unwrap();
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000 + profile.length_mm() + profile.min_gap_mm(),
                0,
            ))
            .unwrap();
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .unwrap();
        world
            .state
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
        let mut state = world.state.vehicle_state(follower).copied().unwrap();
        state.carry_um = 777;
        let next = world.state.advance_active_vehicle(state, 0.1_f32).unwrap();
        assert_eq!(next.carry_um, 0);
        assert_eq!(next.speed_mm_s, 0);
        assert_eq!(next.progress_mm, 1_000);
        assert_eq!(next.status, VehicleStatus::Active);
    }

    #[test]
    fn crawl_retains_sub_millimetre_carry() {
        let mut world = install_preview_world();
        let route = preview_route(&mut world);
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .unwrap();
        world
            .state
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
        let state = world.state.vehicle_state(follower).copied().unwrap();
        let next = world
            .state
            .advance_active_vehicle(state, 0.004_f32)
            .unwrap();
        assert_eq!(next.progress_mm, state.progress_mm);
        assert!(next.carry_um > state.carry_um);
        assert!(next.speed_mm_s > 0);
        assert_eq!(next.status, VehicleStatus::Active);
    }

    #[test]
    fn movement_stop_retains_exact_tie_attribution() {
        let at = BoundedDistance::Finite(4_000);
        assert_eq!(
            select_movement_stop(Some(at), Some(at), BoundedDistance::Finite(5_000)),
            SelectedStop {
                distance: at,
                attribution: StopAttribution::SignalStop,
            }
        );
        assert_eq!(
            select_movement_stop(None, Some(at), at),
            SelectedStop {
                distance: at,
                attribution: StopAttribution::ParkingStop,
            }
        );
        assert_eq!(
            select_movement_stop(Some(at), Some(at), at),
            SelectedStop {
                distance: at,
                attribution: StopAttribution::SignalStop,
            }
        );
        assert_eq!(
            select_movement_stop(
                Some(BoundedDistance::Finite(6_000)),
                Some(BoundedDistance::Finite(3_000)),
                BoundedDistance::Finite(4_000),
            ),
            SelectedStop {
                distance: BoundedDistance::Finite(3_000),
                attribution: StopAttribution::ParkingStop,
            }
        );
        assert_eq!(
            select_movement_stop(
                Some(BoundedDistance::Finite(6_000)),
                Some(BoundedDistance::Finite(5_000)),
                BoundedDistance::Finite(4_000),
            ),
            SelectedStop {
                distance: BoundedDistance::Finite(4_000),
                attribution: StopAttribution::RouteEnd,
            }
        );
    }

    #[test]
    fn iidm_committed_speed_rounds_f32_si() {
        let (_, next) = iidm_step(
            si_speed(9_639),
            si_speed(12_516),
            Some(5_560.0_f32 / 1_000.0),
            2.0,
            1.4,
            1.8,
            2.0,
            4.5,
            8.0_f32 / 1_000.0,
        )
        .unwrap();
        assert_eq!(round_mm(f64::from(next)), Some(9_536));
    }

    #[test]
    fn signal_chain_skips_gates_beyond_the_reach() {
        use laneflow_static_contract::{ManeuverPathOrdinal, SignalAspect};

        use crate::kernel::tables::NextControlled;

        let mut world = install_preview_world();
        let edges = world
            .traffic()
            .maneuvers()
            .maneuver_path(ManeuverPathOrdinal::from_raw(0))
            .expect("fixture path")
            .edges()
            .to_vec();
        let route = world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route");
        let vehicle = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("spawn");
        let mut state = world.state.vehicle_state(vehicle).copied().expect("state");
        state.progress_mm = 0;
        state.speed_mm_s = 0;
        state.carry_um = 0;
        let profile = world
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
            .expect("profile");
        let gates = world
            .state
            .read_view()
            .compiled_route(route)
            .expect("compiled")
            .hop_gate
            .clone();
        let gate = gates
            .into_iter()
            .flatten()
            .find(|gate| {
                world
                    .traffic()
                    .relations()
                    .maneuver_gate(*gate)
                    .and_then(|view| view.signal_group())
                    .is_some()
            })
            .expect("signal gate");
        let slot = usize::try_from(route.index()).expect("slot");
        let write_chain = |world: &mut crate::TrafficWorld, first_mm: u32| {
            let compiled = world.state.committed.routes[slot]
                .compiled
                .as_mut()
                .expect("compiled");
            compiled.next_controlled = vec![
                Some(NextControlled {
                    hop: 0,
                    gate,
                    distance_from_hop_start: BoundedDistance::Finite(first_mm),
                }),
                Some(NextControlled {
                    hop: 1,
                    gate,
                    distance_from_hop_start: BoundedDistance::Finite(20_000),
                }),
            ];
        };
        write_chain(&mut world, 20);
        let reach = MotionReach::from_tick(0, profile.max_accel(), 0.033).expect("reach");
        assert!(!reach.excludes(20));
        assert!(reach.excludes(20_020));

        let query = |world: &crate::TrafficWorld, reach: Option<MotionReach>| {
            reset_barrier_query_counts();
            let view = world.state.read_view();
            let compiled = view.compiled_route(route).expect("compiled");
            let stop = view.signal_stop_distance(compiled, &state, 0, reach);
            (stop, barrier_query_counts().signal_gates)
        };

        world
            .state
            .committed
            .signal_aspects
            .fill(SignalAspect::Green);
        let (green, green_reads) = query(&world, Some(reach));
        assert_eq!(
            green, None,
            "near green does not stop, far gate is not read"
        );
        assert_eq!(green_reads, 1);

        world.state.committed.signal_aspects.fill(SignalAspect::Red);
        let (red, red_reads) = query(&world, Some(reach));
        assert_eq!(red, Some(BoundedDistance::Finite(20)));
        assert_eq!(red_reads, 1);

        write_chain(&mut world, 20_000);
        let (far_red, far_reads) = query(&world, Some(reach));
        assert_eq!(far_red, None);
        assert_eq!(far_reads, 0);
        let (exact, exact_reads) = query(&world, None);
        assert_eq!(exact, Some(BoundedDistance::Finite(20_000)));
        assert_eq!(exact_reads, 1);
    }
}

#[cfg(test)]
mod barrier_query_tests {
    use super::*;

    use crate::kernel::conflict_tick::ConflictMotionPlan;
    use crate::kernel::tables::{
        ConflictPassageOccurrence, RoutePosition, WaitingOccurrence,
        compile_nearest_motion_barriers,
    };
    use crate::kernel::waiting::WaitingMembership;
    use crate::{ConflictDecisionOutcome, TickInput, VehicleSpawnInput, VehicleStatus};
    use laneflow_static_contract::{
        ConflictZoneOrdinal, ParticipantStreamOrdinal, SignalAspect, VehicleProfileOrdinal,
        WaitingZoneOrdinal,
    };

    #[test]
    fn motion_reach_is_strict_and_abandons_unproven_ranges() {
        let exact = MotionReach::from_tick(0, 0.5, 1.0).expect("proven");
        assert!(!exact.excludes(300));
        assert!(exact.excludes(301));
        assert!(exact.class_is_unreachable(None, 0));
        assert!(exact.class_is_unreachable(Some(BoundedDistance::BeyondFinite), 0));
        assert!(!exact.class_is_unreachable(Some(BoundedDistance::Finite(300)), 0));

        let sample = MotionReach::from_tick(10_000, 3.0, 0.033).expect("sample");
        assert!(
            sample.millimeters > 381.0 && sample.millimeters < 382.0,
            "10 m/s, 3 m/s², 33 ms reaches about 381.6 mm, got {}",
            sample.millimeters
        );
        assert!(!sample.excludes(381));
        assert!(sample.excludes(382));

        assert!(MotionReach::from_tick(100_001, 1.0, 0.033).is_none());
        assert!(MotionReach::from_tick(0, 0.4, 0.033).is_none());
        assert!(MotionReach::from_tick(0, 50.1, 0.033).is_none());
        assert!(MotionReach::from_tick(0, 0.5, 0.003).is_none());
        assert!(MotionReach::from_tick(0, 0.5, 1.001).is_none());
        assert!(MotionReach::from_tick(0, f32::NAN, 0.033).is_none());
        assert!(MotionReach::from_tick(100_000, 50.0, 1.0).is_some());
        assert!(MotionReach::from_tick(0, 0.5, 0.004).is_some());
    }

    #[test]
    fn unreachable_barrier_queries_stay_out_of_the_tick() {
        let mut world = crate::kernel::waiting::tests::multi_gate_world(1);
        world.state.rebuild_occupancy_index().expect("occupancy");
        let handle = world.state.committed.live_order[0];
        let base = world.state.vehicle_state(handle).copied().expect("vehicle");
        let lengths = world.traffic().lane_lengths_millimetres().to_vec();
        let slot = usize::try_from(base.route.index()).expect("slot");
        let (edge_length, first_zone, first_release, conflict_absent) = {
            let compiled = world.state.committed.routes[slot]
                .compiled
                .as_mut()
                .expect("compiled");
            let first = compiled.waiting[0];
            compiled.waiting.push(WaitingOccurrence {
                zone: WaitingZoneOrdinal::from_raw(first.zone.raw().saturating_add(1)),
                maneuver_index: first.maneuver_index,
                entry_hop: 1,
                release_hop: first.release_hop.saturating_add(1),
                storage_length_mm: 1,
                dependency_end: 0,
            });
            let edge_lengths: Vec<u32> = compiled
                .edges
                .iter()
                .map(|edge| lengths[edge.index()])
                .collect();
            compiled.nearest_motion_barriers = compile_nearest_motion_barriers(
                &edge_lengths,
                &compiled.conflicts,
                &compiled.waiting,
            )
            .expect("rebuild barriers");
            let absent = compiled.nearest_motion_barriers[0]
                .conflict_from_occurrence_start
                .is_none();
            (edge_lengths[0], first.zone, first.release_hop, absent)
        };
        assert!(
            edge_length > 1_000,
            "idle edge is long enough to stand far from its gate"
        );
        assert!(
            conflict_absent,
            "this fixture has no conflict admission ahead"
        );

        let mut far = base;
        far.route_edge_index = 0;
        far.progress_mm = 1;
        far.carry_um = 0;
        far.speed_mm_s = 0;
        let mut near = base;
        near.route_edge_index = 0;
        near.progress_mm = edge_length - 1;
        near.carry_um = 0;
        near.speed_mm_s = 0;

        let production = |world: &mut crate::TrafficWorld, state: &VehicleState, delta_s: f32| {
            reset_barrier_query_counts();
            let phase = world.state.step_workspace();
            let pruned = phase
                .motion_conflict_stop_for(state, delta_s)
                .expect("production");
            let exact = phase.conflict_stop_for(state).expect("exact");
            let counts = barrier_query_counts();
            (pruned, exact, counts)
        };

        let (pruned, exact, counts) = production(&mut world, &far, 0.033);
        assert_eq!(pruned, None);
        assert!(
            exact.is_some(),
            "exact query still sees the far waiting entry"
        );
        assert_eq!(counts.waiting_entry_scans, 0);
        assert_eq!(counts.conflict_scans, 0);
        for carry in [0_u16, 1, 999] {
            let state = VehicleState {
                carry_um: carry,
                ..far
            };
            let (_, exact, _) = production(&mut world, &state, 0.033);
            let view = world.state.read_view();
            assert_eq!(
                view.advance_active_vehicle_with_waiting_stop(state, 0.033, None, exact),
                view.advance_active_vehicle_with_waiting_stop(state, 0.033, None, None),
                "carry {carry} is unchanged when the only stop is beyond reach"
            );
        }

        let (pruned, exact, counts) = production(&mut world, &near, 0.033);
        assert_eq!(pruned, exact);
        assert_eq!(pruned.expect("near gate").hop, 0);
        assert_eq!(counts.waiting_entry_scans, 1);
        assert_eq!(counts.conflict_scans, 0);

        let mut boundary = far;
        boundary.progress_mm = 0;
        boundary.carry_um = 0;
        boundary.speed_mm_s = 5_000;
        let (pruned, exact, counts) = production(&mut world, &boundary, 0.033);
        assert_eq!(pruned, exact);
        assert!(
            counts.waiting_entry_scans >= 1,
            "zero progress and carry keep the query"
        );

        let (pruned, exact, counts) = production(&mut world, &far, 0.001);
        assert_eq!(pruned, exact);
        assert!(
            counts.waiting_entry_scans >= 1,
            "a step shorter than 4 ms is not proven"
        );

        let mut authorized = near;
        authorized.speed_mm_s = 100_000;
        authorized.waiting_membership = Some(WaitingMembership {
            waiting_zone: first_zone,
            admission_sequence: 1,
            release_hop: first_release,
        });
        let (pruned, exact, counts) = production(&mut world, &authorized, 1.0);
        assert_eq!(pruned, exact);
        let later = pruned.expect("later gate");
        assert_eq!(later.hop, 1);
        assert_eq!(counts.waiting_entry_scans, 1);
        let BoundedDistance::Finite(later_mm) = later.distance else {
            panic!("later unauthorized gate still has a finite stop distance");
        };
        let accel = world
            .traffic()
            .relations()
            .vehicle_profile(authorized.profile)
            .expect("profile")
            .max_accel();
        let reach = MotionReach::from_tick(authorized.speed_mm_s, accel, 1.0).expect("reach");
        assert!(
            !reach.excludes(later_mm),
            "an authorized near gate must not hide the next gate inside the reach"
        );

        world.state.committed.routes[slot]
            .compiled
            .as_mut()
            .expect("compiled")
            .nearest_motion_barriers
            .clear();
        let (pruned, exact, counts) = production(&mut world, &far, 0.033);
        assert_eq!(pruned, exact);
        assert!(exact.is_some());
        assert!(
            counts.waiting_entry_scans >= 1,
            "a missing index is not an empty barrier list"
        );

        let mut at_start = far;
        at_start.progress_mm = 0;
        at_start.speed_mm_s = 0;
        reset_barrier_query_counts();
        world
            .state
            .advance_active_vehicle(at_start, 0.004)
            .expect("short step");
        assert_eq!(barrier_query_counts().hard_room_permissions, 0);
        let mut at_end = near;
        at_end.speed_mm_s = 0;
        reset_barrier_query_counts();
        world
            .state
            .advance_active_vehicle(at_end, 0.004)
            .expect("near edge end");
        assert_eq!(barrier_query_counts().hard_room_permissions, 1);
    }

    fn conflict_passage(admission_hop: u32) -> ConflictPassageOccurrence {
        ConflictPassageOccurrence {
            stream: ParticipantStreamOrdinal::from_raw(0),
            passage_local_index: admission_hop,
            zone: ConflictZoneOrdinal::from_raw(0),
            maneuver_index: 0,
            admission_hop,
            entry: RoutePosition {
                route_edge_index: admission_hop.saturating_add(1),
                progress_mm: 4_000,
            },
            clearance: RoutePosition {
                route_edge_index: admission_hop.saturating_add(1),
                progress_mm: 8_000,
            },
        }
    }

    fn waiting_on(template: WaitingOccurrence, entry_hop: u32) -> WaitingOccurrence {
        WaitingOccurrence {
            entry_hop,
            release_hop: entry_hop,
            ..template
        }
    }

    fn install_barriers(
        world: &mut crate::TrafficWorld,
        route_slot: usize,
        conflicts: Vec<ConflictPassageOccurrence>,
        waiting: Vec<WaitingOccurrence>,
    ) -> u32 {
        let lengths = world.traffic().lane_lengths_millimetres().to_vec();
        let compiled = world.state.committed.routes[route_slot]
            .compiled
            .as_mut()
            .expect("compiled route");
        let edge_lengths: Vec<u32> = compiled
            .edges
            .iter()
            .map(|edge| lengths[edge.index()])
            .collect();
        compiled.conflicts = conflicts;
        compiled.waiting = waiting;
        compiled.nearest_motion_barriers =
            compile_nearest_motion_barriers(&edge_lengths, &compiled.conflicts, &compiled.waiting)
                .expect("barrier table");
        assert!(
            !compiled.conflicts.is_empty(),
            "the production search must see a real admission list"
        );
        edge_lengths[0]
    }

    fn query_conflict(
        world: &mut crate::TrafficWorld,
        state: &VehicleState,
        delta_s: f32,
    ) -> (
        Option<crate::kernel::waiting::WaitingStopConstraint>,
        Option<crate::kernel::waiting::WaitingStopConstraint>,
        BarrierQueryCounts,
    ) {
        reset_barrier_query_counts();
        let phase = world.state.step_workspace();
        let pruned = phase
            .motion_conflict_stop_for(state, delta_s)
            .expect("production conflict stop");
        let exact = phase.conflict_stop_for(state).expect("exact conflict stop");
        (pruned, exact, barrier_query_counts())
    }

    #[test]
    fn nonempty_conflict_admissions_skip_and_return_independently() {
        let mut world = crate::kernel::waiting::tests::multi_gate_world(1);
        let handle = world.state.committed.live_order[0];
        let base = world.state.vehicle_state(handle).copied().expect("vehicle");
        let slot = usize::try_from(base.route.index()).expect("slot");
        let template = world.state.committed.routes[slot]
            .compiled
            .as_ref()
            .expect("compiled")
            .waiting[0];
        let edge_length = install_barriers(
            &mut world,
            slot,
            vec![conflict_passage(0)],
            vec![waiting_on(template, 1)],
        );
        let mut near = base;
        near.route_edge_index = 0;
        near.progress_mm = edge_length - 1;
        near.carry_um = 0;
        near.speed_mm_s = 0;
        world.state.workspace.conflict_motion_by_vehicle.fill(None);

        let (pruned, exact, counts) = query_conflict(&mut world, &near, 0.033);
        assert_eq!(pruned, exact);
        assert_eq!(pruned.expect("near conflict").hop, 0);
        assert_eq!(counts.conflict_scans, 1);
        assert_eq!(counts.waiting_entry_scans, 0);

        install_barriers(
            &mut world,
            slot,
            vec![conflict_passage(1)],
            vec![waiting_on(template, 0)],
        );
        let (pruned, exact, counts) = query_conflict(&mut world, &near, 0.033);
        assert_eq!(pruned, exact);
        assert_eq!(pruned.expect("near waiting entry").hop, 0);
        assert_eq!(counts.conflict_scans, 0);
        assert_eq!(counts.waiting_entry_scans, 1);

        install_barriers(
            &mut world,
            slot,
            vec![conflict_passage(0), conflict_passage(1)],
            Vec::new(),
        );
        let mut far = near;
        far.progress_mm = 1;
        world.state.workspace.conflict_motion_by_vehicle[handle.index() as usize] =
            Some(ConflictMotionPlan {
                gate_hop: 0,
                outcome: ConflictDecisionOutcome::Granted,
                grant_index: None,
            });
        let (pruned, exact, counts) = query_conflict(&mut world, &far, 0.033);
        assert_eq!(pruned, None);
        assert_eq!(exact.expect("exact still walks the later admission").hop, 1);
        assert_eq!(counts.conflict_scans, 0);
        assert_eq!(counts.waiting_entry_scans, 0);

        let mut authorized_near = near;
        authorized_near.speed_mm_s = 100_000;
        let (pruned, exact, counts) = query_conflict(&mut world, &authorized_near, 1.0);
        assert_eq!(pruned, exact);
        let later = pruned.expect("later unauthorized admission");
        assert_eq!(later.hop, 1);
        assert_eq!(counts.conflict_scans, 1);
        let BoundedDistance::Finite(later_mm) = later.distance else {
            panic!("later admission has a finite stop");
        };
        let accel = world
            .traffic()
            .relations()
            .vehicle_profile(authorized_near.profile)
            .expect("profile")
            .max_accel();
        let reach = MotionReach::from_tick(100_000, accel, 1.0).expect("reach");
        assert!(!reach.excludes(later_mm));

        install_barriers(
            &mut world,
            slot,
            vec![conflict_passage(0)],
            vec![waiting_on(template, 1)],
        );
        world.state.workspace.conflict_motion_by_vehicle.fill(None);
        let mut lookback = base;
        lookback.route_edge_index = 1;
        lookback.progress_mm = 0;
        lookback.carry_um = 0;
        lookback.speed_mm_s = 5_000;
        let (pruned, exact, counts) = query_conflict(&mut world, &lookback, 0.033);
        assert_eq!(pruned, exact);
        assert_eq!(
            pruned.expect("previous admission").hop,
            0,
            "standing on the next occurrence still checks the admission behind it"
        );
        assert!(counts.conflict_scans >= 1);
    }

    #[test]
    fn committed_motion_restores_the_conflict_query_on_the_next_tick() {
        let (mut world, route) =
            crate::admin::cutover_migration::tests::signal_frontier_world(10_000, None, false);
        let compiled = world
            .state
            .read_view()
            .compiled_route(route)
            .expect("compiled")
            .clone();
        assert!(!compiled.conflicts.is_empty());
        let admission = compiled.conflicts[0].admission_hop;
        assert_eq!(admission, 0);
        let edge = compiled.edges[admission as usize];
        let edge_length = world.traffic().lane_lengths_millimetres()[edge.index()];
        let profile = world
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .expect("profile");
        let gate = compiled.hop_gate[admission as usize].expect("admission gate");
        assert!(
            world
                .state
                .gate_is_restrictive(gate, VehicleProfileOrdinal::from_raw(0)),
            "a red admission is restrictive once it is actually interpreted"
        );
        let speed = 5_000;
        let delta_s = 0.1;
        let reach = MotionReach::from_tick(speed, profile.max_accel(), delta_s).expect("reach");
        let beyond = u32::try_from(reach.millimeters.floor() as u64)
            .expect("reach fits")
            .saturating_add(20);
        assert!(reach.excludes(beyond));
        assert!(edge_length > beyond + 1);
        let handle = world
            .state
            .place_existing_active_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                admission,
                edge_length - beyond,
                speed,
            ))
            .expect("spawn");
        {
            let state = world.state.committed.vehicles[handle.index() as usize]
                .state
                .as_mut()
                .expect("state");
            state.carry_um = 999;
        }
        world.state.rebuild_occupancy_index().expect("occupancy");
        let before = world.state.vehicle_state(handle).copied().expect("before");
        assert_eq!(before.carry_um, 999);
        assert!(reach.excludes(edge_length - before.progress_mm));

        reset_barrier_query_counts();
        world.step(TickInput::new(100)).expect("first tick");
        assert_eq!(
            barrier_query_counts().conflict_scans,
            0,
            "the first tick is still beyond the reach"
        );
        let mid = world.state.vehicle_state(handle).copied().expect("mid");
        assert_eq!(mid.route_edge_index, admission);
        assert!(mid.progress_mm > before.progress_mm);
        assert!(mid.progress_mm < edge_length);
        assert_eq!(mid.status, VehicleStatus::Active);
        let remaining = edge_length - mid.progress_mm;
        let mid_reach = MotionReach::from_tick(mid.speed_mm_s, profile.max_accel(), delta_s)
            .expect("mid reach");
        assert!(
            !mid_reach.excludes(remaining),
            "committed progress {remaining} mm must be inside the next reach {}",
            mid_reach.millimeters
        );

        // 灯保持红时，信号和冲突停在同一扇门。这里临时改成绿灯，只比较运动内核：
        // 恢复出来的冲突停止本身就能拦住车，不依赖红灯。
        let red_aspects = world.state.committed.signal_aspects.clone();
        world
            .state
            .committed
            .signal_aspects
            .fill(SignalAspect::Green);
        world.state.workspace.conflict_motion_by_vehicle.fill(None);
        let (stop, exact, _) = query_conflict(&mut world, &mid, delta_s);
        assert_eq!(stop, exact);
        let stop = stop.expect("the committed state restores the admission stop");
        let view = world.state.read_view();
        let held = view
            .advance_active_vehicle_with_waiting_stop(mid, delta_s, None, Some(stop))
            .expect("held motion");
        let free = view
            .advance_active_vehicle_with_waiting_stop(mid, delta_s, None, None)
            .expect("free motion");
        assert_eq!(held.route_edge_index, admission);
        assert!(
            free.route_edge_index > held.route_edge_index || free.progress_mm > held.progress_mm,
            "without the restored admission stop the vehicle travels farther"
        );
        world.state.committed.signal_aspects = red_aspects;

        reset_barrier_query_counts();
        world.step(TickInput::new(100)).expect("second tick");
        assert!(
            barrier_query_counts().conflict_scans >= 1,
            "the next tick queries the admission again"
        );
        let after = world.state.vehicle_state(handle).copied().expect("after");
        assert_eq!(after.status, VehicleStatus::Active);
        assert_eq!(
            after.route_edge_index, admission,
            "an unauthorized admission does not let the vehicle cross"
        );
        assert!(after.progress_mm <= edge_length);
        assert!(
            after.progress_mm < edge_length || after.speed_mm_s == 0,
            "reaching the admission stops the vehicle instead of passing it"
        );
    }
}

#[cfg(test)]
#[path = "phase_equivalence.rs"]
mod phase_equivalence;

#[cfg(test)]
use crate::TrafficWorld;
