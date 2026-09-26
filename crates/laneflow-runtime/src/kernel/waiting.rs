use crate::{TrafficTransitionEvent, TrafficTransitionKind};
use laneflow_static_contract::WaitingZoneOrdinal;

use crate::kernel::tables::{CompiledRoute, distance_to_occurrence_start};
use crate::{RouteHandle, VehicleHandle};
use laneflow_static_network::BoundedDistance;

#[cfg(test)]
thread_local! {
    static WAITING_RESERVATIONS_BEFORE_FAILURE: core::cell::Cell<Option<usize>> =
        const { core::cell::Cell::new(None) };
    static NON_ENTRY_GENERATION_VISITS: core::cell::Cell<usize> =
        const { core::cell::Cell::new(0) };
    static NON_ENTRY_DISCOVERY_VISITS: core::cell::Cell<usize> =
        const { core::cell::Cell::new(0) };
    static NON_ENTRY_SEQUENCE_VISITS: core::cell::Cell<usize> =
        const { core::cell::Cell::new(0) };
    static WAITING_LOOKUP_VISITS: core::cell::Cell<[usize; 4]> =
        const { core::cell::Cell::new([0; 4]) };
    static WAITING_WORK_COUNTS: core::cell::Cell<WaitingWorkCounts> =
        const { core::cell::Cell::new(WaitingWorkCounts {
            checked_zones: 0, staged_zones: 0, journal_zones: 0,
            committed_zones: 0, member_vehicles: 0,
        }) };
    /// 可选并行暂存预留失败的注入开关；协调器线程本地，武装时强制走融合回退。
    static PREVIEW_SLOT_RESERVE_FAILURE: core::cell::Cell<bool> =
        const { core::cell::Cell::new(false) };
    /// 缺失槽位注入：join 完成后把该 Active 位置的槽位改写为 `Pending`。
    static PREVIEW_SLOT_GAP: core::cell::Cell<Option<usize>> =
        const { core::cell::Cell::new(None) };
    /// 输入表预留注入：下一次发现循环的 checked 预留强制失败。
    static PREVIEW_INPUT_RESERVE_FAILURE: core::cell::Cell<bool> =
        const { core::cell::Cell::new(false) };
    static PREVIEW_PATH_COUNTS: core::cell::Cell<WaitingPreviewPathCounts> =
        const { core::cell::Cell::new(WaitingPreviewPathCounts {
            dispatched: 0, fused: 0, slot_fallback: 0,
        }) };
}

/// 测试专用：车型检查、机动定位、Waiting 成员查询、后续出现项访问。
#[cfg(test)]
fn count_waiting_lookup(index: usize) {
    WAITING_LOOKUP_VISITS.with(|counts| {
        let mut value = counts.get();
        value[index] += 1;
        counts.set(value);
    });
}

/// 测试专用访问计数，不进入生产世界布局或持久状态。
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WaitingWorkCounts {
    pub(crate) checked_zones: usize,
    pub(crate) staged_zones: usize,
    pub(crate) journal_zones: usize,
    pub(crate) committed_zones: usize,
    pub(crate) member_vehicles: usize,
}

/// 本拍正式 motion 中已发现的非入口 Gate；只引用 updates 和编译路线。
#[derive(Clone, Copy, Debug)]
pub(crate) struct NonEntryGateAnchor {
    update_index: usize,
    maneuver_occurrence_index: u32,
    hop: u32,
}

#[cfg(test)]
pub(crate) fn count_waiting_work(update: impl FnOnce(&mut WaitingWorkCounts)) {
    WAITING_WORK_COUNTS.with(|counts| {
        let mut value = counts.get();
        update(&mut value);
        counts.set(value);
    });
}

#[cfg(test)]
struct WaitingReservationFailpointReset(Option<usize>);

#[cfg(test)]
impl Drop for WaitingReservationFailpointReset {
    fn drop(&mut self) {
        WAITING_RESERVATIONS_BEFORE_FAILURE.with(|remaining| remaining.set(self.0));
    }
}

#[cfg(test)]
fn fail_waiting_reservation_after(successes: usize) -> WaitingReservationFailpointReset {
    let previous =
        WAITING_RESERVATIONS_BEFORE_FAILURE.with(|remaining| remaining.replace(Some(successes)));
    WaitingReservationFailpointReset(previous)
}

#[cfg(test)]
fn waiting_reservation_injected_failure() -> bool {
    WAITING_RESERVATIONS_BEFORE_FAILURE.with(|remaining| match remaining.get() {
        Some(0) => true,
        Some(value) => {
            remaining.set(Some(value - 1));
            false
        }
        None => false,
    })
}

/// P2 逐车预览的故障注入面（#705 验收）。错误/panic 注入在任务线程内被读取，
/// 必须用进程级静态量并按下标武装；协调器侧钩子（暂存回退、缺失槽位、路径
/// 计数）保持线程本地。注入按「世界身份 + live 序」定位，各测试使用互不相同的
/// 世界身份，避免并行测试互相观测到对方的武装状态。
#[cfg(test)]
mod preview_injection {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    pub(super) const DISABLED_WORLD: u64 = u64::MAX;

    pub(super) static NONFINITE_WORLD: AtomicU64 = AtomicU64::new(DISABLED_WORLD);
    pub(super) static NONFINITE_POSITIONS: AtomicU64 = AtomicU64::new(0);
    pub(super) static INVARIANT_WORLD: AtomicU64 = AtomicU64::new(DISABLED_WORLD);
    pub(super) static INVARIANT_POSITIONS: AtomicU64 = AtomicU64::new(0);
    pub(super) static PANIC_WORLD: AtomicU64 = AtomicU64::new(DISABLED_WORLD);
    pub(super) static PANIC_POSITION: AtomicUsize = AtomicUsize::new(usize::MAX);

    pub(super) fn position_mask(positions: &[usize]) -> u64 {
        positions.iter().fold(0_u64, |mask, position| {
            assert!(
                *position < u64::BITS as usize,
                "preview injection position fits mask"
            );
            mask | (1_u64 << position)
        })
    }

    pub(super) fn snapshot() -> (u64, u64, u64, u64, u64, usize) {
        (
            NONFINITE_WORLD.load(Ordering::SeqCst),
            NONFINITE_POSITIONS.load(Ordering::SeqCst),
            INVARIANT_WORLD.load(Ordering::SeqCst),
            INVARIANT_POSITIONS.load(Ordering::SeqCst),
            PANIC_WORLD.load(Ordering::SeqCst),
            PANIC_POSITION.load(Ordering::SeqCst),
        )
    }

    pub(super) fn restore(state: (u64, u64, u64, u64, u64, usize)) {
        NONFINITE_WORLD.store(state.0, Ordering::SeqCst);
        NONFINITE_POSITIONS.store(state.1, Ordering::SeqCst);
        INVARIANT_WORLD.store(state.2, Ordering::SeqCst);
        INVARIANT_POSITIONS.store(state.3, Ordering::SeqCst);
        PANIC_WORLD.store(state.4, Ordering::SeqCst);
        PANIC_POSITION.store(state.5, Ordering::SeqCst);
    }
}

/// 恢复全部 P2 注入先前值；测试 panic 路径时也保证复位。
#[cfg(test)]
pub(crate) struct PreviewInjectionGuard {
    state: (u64, u64, u64, u64, u64, usize),
}

#[cfg(test)]
impl Drop for PreviewInjectionGuard {
    fn drop(&mut self) {
        preview_injection::restore(self.state);
    }
}

/// 按逻辑位置（live 序）武装预览错误注入；`nonfinite`/`invariant` 分别在该
/// 位置强制 `NonFiniteMotion`/`WaitingInvariantViolation`。
#[cfg(test)]
pub(crate) fn inject_preview_errors(
    world_id: u64,
    nonfinite: &[usize],
    invariant: &[usize],
) -> PreviewInjectionGuard {
    use std::sync::atomic::Ordering;
    let guard = PreviewInjectionGuard {
        state: preview_injection::snapshot(),
    };
    preview_injection::NONFINITE_WORLD.store(world_id, Ordering::SeqCst);
    preview_injection::NONFINITE_POSITIONS.store(
        preview_injection::position_mask(nonfinite),
        Ordering::SeqCst,
    );
    preview_injection::INVARIANT_WORLD.store(world_id, Ordering::SeqCst);
    preview_injection::INVARIANT_POSITIONS.store(
        preview_injection::position_mask(invariant),
        Ordering::SeqCst,
    );
    guard
}

/// 在指定逻辑位置（live 序）武装 P2 计算 panic 注入。
#[cfg(test)]
fn inject_preview_panic(world_id: u64, position: usize) -> PreviewInjectionGuard {
    use std::sync::atomic::Ordering;
    let guard = PreviewInjectionGuard {
        state: preview_injection::snapshot(),
    };
    preview_injection::PANIC_WORLD.store(world_id, Ordering::SeqCst);
    preview_injection::PANIC_POSITION.store(position, Ordering::SeqCst);
    guard
}

/// P2 逐车原语内的错误注入检查：命中返回须在原逻辑位置公开的完整领域错误。
#[cfg(test)]
pub(crate) fn injected_preview_error(
    world_id: u64,
    update_sequence: usize,
) -> Option<crate::StepError> {
    use std::sync::atomic::Ordering;
    let position = u64::try_from(update_sequence).ok()?;
    if position >= u64::BITS as u64 {
        return None;
    }
    let bit = 1_u64 << position;
    if preview_injection::NONFINITE_WORLD.load(Ordering::SeqCst) == world_id
        && preview_injection::NONFINITE_POSITIONS.load(Ordering::SeqCst) & bit != 0
    {
        return Some(crate::StepError::NonFiniteMotion);
    }
    if preview_injection::INVARIANT_WORLD.load(Ordering::SeqCst) == world_id
        && preview_injection::INVARIANT_POSITIONS.load(Ordering::SeqCst) & bit != 0
    {
        return Some(crate::StepError::WaitingInvariantViolation);
    }
    None
}

/// P2 逐车原语内的 panic 注入检查；panic 不按 `StepError` 映射，由执行器
/// 完整 join 后向世界宿主传播。
#[cfg(test)]
pub(crate) fn injected_preview_panics(world_id: u64, update_sequence: usize) -> bool {
    use std::sync::atomic::Ordering;
    preview_injection::PANIC_WORLD.load(Ordering::SeqCst) == world_id
        && preview_injection::PANIC_POSITION.load(Ordering::SeqCst) == update_sequence
}

/// 测试专用：P2 本阶段谁执行的计数证据；只读，不改变语义。
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WaitingPreviewPathCounts {
    pub(crate) dispatched: usize,
    pub(crate) fused: usize,
    pub(crate) slot_fallback: usize,
}

#[cfg(test)]
pub(crate) fn preview_path_counts() -> WaitingPreviewPathCounts {
    PREVIEW_PATH_COUNTS.with(|counts| counts.get())
}

#[cfg(test)]
fn count_preview_path(update: impl FnOnce(&mut WaitingPreviewPathCounts)) {
    PREVIEW_PATH_COUNTS.with(|counts| {
        let mut value = counts.get();
        update(&mut value);
        counts.set(value);
    });
}

/// P2 第一遍诊断子段（cfg(test)，#705 机制测量）。线程本地槽只承载协调器
/// 侧墙钟：前置检查与输入准备 / 融合循环墙钟 / 分发作用域墙钟（含并行
/// 计算，不与任务时间合计相加） / 规范消费 / 后续 Waiting 组装。分发的
/// 块计算耗时由任务写进块级诊断记录槽（waiting.rs 分发函数内，按块对齐、
/// 独占写入），join 后由协调器汇总进 CHUNK_* 累计，辅助线程不被漏记。
/// 只在探针启用时打开，不影响生产路径与 #583 阶段协议测试。
#[cfg(test)]
pub(crate) mod preview_stage {
    use std::cell::Cell;
    use std::time::Instant;

    /// 前置检查与输入准备（协调器：成员校验、scratch 预留、输入发现）。
    pub(crate) const PREAMBLE: usize = 0;
    /// 融合循环墙钟（仅融合臂；包住逐车 staging，与分发块计算边界不同）。
    pub(crate) const FUSED_LOOP: usize = 1;
    /// 分发作用域墙钟（分发到 join 返回，含并行计算；不是纯调度开销）。
    pub(crate) const DISPATCH_SCOPE: usize = 2;
    /// 规范消费（协调器按 Active 顺序写共享暂存）。
    pub(crate) const CONSUME: usize = 3;
    /// 后续 Waiting 组装（第二遍决策、排序与暂存）。
    pub(crate) const ASSEMBLY: usize = 4;
    pub(crate) const STAGE_COUNT: usize = 5;

    thread_local! {
        static ENABLED: Cell<bool> = const { Cell::new(false) };
        static NANOS: Cell<[u128; STAGE_COUNT]> = const { Cell::new([0; STAGE_COUNT]) };
        static CHUNK_TOTAL_NANOS: Cell<u128> = const { Cell::new(0) };
        static CHUNK_MAX_NANOS: Cell<u128> = const { Cell::new(0) };
    }

    pub(crate) struct Span(Option<(usize, Instant)>);

    pub(crate) fn begin(index: usize) -> Span {
        Span(ENABLED.with(|enabled| enabled.get().then(|| (index, Instant::now()))))
    }

    impl Drop for Span {
        fn drop(&mut self) {
            if let Some((index, started)) = self.0 {
                let elapsed = started.elapsed().as_nanos();
                NANOS.with(|nanos| {
                    let mut next = nanos.get();
                    next[index] += elapsed;
                    nanos.set(next);
                });
            }
        }
    }

    pub(crate) fn enabled() -> bool {
        ENABLED.with(Cell::get)
    }

    /// 协调器在 join 后汇总本拍块级记录：全部块计算时间合计与单拍最长块。
    pub(crate) fn note_chunk_batch(total_nanos: u128, max_nanos: u128) {
        if !enabled() {
            return;
        }
        CHUNK_TOTAL_NANOS.with(|total| total.set(total.get() + total_nanos));
        CHUNK_MAX_NANOS.with(|max| {
            if max_nanos > max.get() {
                max.set(max_nanos);
            }
        });
    }

    pub(crate) fn set_enabled(enabled: bool) {
        ENABLED.with(|cell| cell.set(enabled));
    }

    pub(crate) fn take() -> ([u128; STAGE_COUNT], u128, u128) {
        (
            NANOS.with(|nanos| nanos.take()),
            CHUNK_TOTAL_NANOS.with(|total| total.take()),
            CHUNK_MAX_NANOS.with(|max| max.take()),
        )
    }
}

/// 分块粒度倍数（cfg(test) 探针旋钮，默认 2）：块数 =
/// `dispatch_threads × 倍数`，与 `workload` 取较小者；语义中立。
#[cfg(test)]
fn preview_chunk_multiplier() -> usize {
    PREVIEW_CHUNK_MULTIPLIER.with(core::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn set_preview_chunk_multiplier(multiplier: usize) {
    PREVIEW_CHUNK_MULTIPLIER.with(|cell| cell.set(multiplier.max(1)));
}

#[cfg(test)]
thread_local! {
    static PREVIEW_CHUNK_MULTIPLIER: core::cell::Cell<usize> = const { core::cell::Cell::new(2) };
}

/// 测试专用：生产分发阈值为保守的 1_024；小场景测试经该守卫强制走真实
/// 分发（不改变语义，等价验收已有全套对拍）。
#[cfg(test)]
fn preview_dispatch_forced() -> bool {
    PREVIEW_FORCE_DISPATCH.with(core::cell::Cell::get)
}

#[cfg(test)]
pub(crate) struct ForcePreviewDispatchGuard(bool);

#[cfg(test)]
impl Drop for ForcePreviewDispatchGuard {
    fn drop(&mut self) {
        PREVIEW_FORCE_DISPATCH.with(|forced| forced.set(self.0));
    }
}

/// 测试专用：本拍起强制 P2 真实分发（工作集非空时），返回复位守卫。
#[cfg(test)]
pub(crate) fn force_preview_dispatch() -> ForcePreviewDispatchGuard {
    ForcePreviewDispatchGuard(PREVIEW_FORCE_DISPATCH.with(|forced| forced.replace(true)))
}

#[cfg(test)]
thread_local! {
    static PREVIEW_FORCE_DISPATCH: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
    static PREVIEW_FORCE_FUSE: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

#[cfg(test)]
fn preview_dispatch_fuse_forced() -> bool {
    PREVIEW_FORCE_FUSE.with(core::cell::Cell::get)
}

#[cfg(not(test))]
fn preview_dispatch_fuse_forced() -> bool {
    false
}

#[cfg(test)]
pub(crate) struct ForcePreviewFuseGuard(bool);

#[cfg(test)]
impl Drop for ForcePreviewFuseGuard {
    fn drop(&mut self) {
        PREVIEW_FORCE_FUSE.with(|forced| forced.set(self.0));
    }
}

/// 测试专用：本拍起强制 P2 保持融合（即使 Pool 执行器与工作集在阈值
/// 之上；#706 增量 E 组合矩阵的融合侧入口，fuse 优先于 force）。
#[cfg(test)]
pub(crate) fn force_preview_fuse() -> ForcePreviewFuseGuard {
    ForcePreviewFuseGuard(PREVIEW_FORCE_FUSE.with(|forced| forced.replace(true)))
}

#[cfg(test)]
fn preview_slot_reserve_injected_failure() -> bool {
    PREVIEW_SLOT_RESERVE_FAILURE.with(|failure| failure.get())
}

#[cfg(test)]
fn preview_slot_gap_position() -> Option<usize> {
    PREVIEW_SLOT_GAP.with(|gap| gap.get())
}

#[cfg(test)]
struct PreviewSlotReserveFailureGuard(bool);

#[cfg(test)]
impl Drop for PreviewSlotReserveFailureGuard {
    fn drop(&mut self) {
        PREVIEW_SLOT_RESERVE_FAILURE.with(|failure| failure.set(self.0));
    }
}

/// 测试专用：下一次分发暂存预留强制失败，验证可选工作区回退。
#[cfg(test)]
fn fail_preview_slot_reserve() -> PreviewSlotReserveFailureGuard {
    PreviewSlotReserveFailureGuard(
        PREVIEW_SLOT_RESERVE_FAILURE.with(|failure| failure.replace(true)),
    )
}

#[cfg(test)]
fn preview_input_reserve_injected_failure() -> bool {
    PREVIEW_INPUT_RESERVE_FAILURE.with(|failure| failure.get())
}

#[cfg(test)]
struct PreviewInputReserveFailureGuard(bool);

#[cfg(test)]
impl Drop for PreviewInputReserveFailureGuard {
    fn drop(&mut self) {
        PREVIEW_INPUT_RESERVE_FAILURE.with(|failure| failure.set(self.0));
    }
}

/// 测试专用：下一次输入发现循环的 checked 预留强制失败，验证输入表
/// 冷态/增长预留失败回退融合。
#[cfg(test)]
fn fail_preview_input_reserve() -> PreviewInputReserveFailureGuard {
    PreviewInputReserveFailureGuard(
        PREVIEW_INPUT_RESERVE_FAILURE.with(|failure| failure.replace(true)),
    )
}

#[cfg(test)]
struct PreviewSlotGapGuard(Option<usize>);

#[cfg(test)]
impl Drop for PreviewSlotGapGuard {
    fn drop(&mut self) {
        PREVIEW_SLOT_GAP.with(|gap| gap.set(self.0));
    }
}

/// 测试专用：join 完成后把指定 Active 位置的槽位改写为 `Pending`，
/// 验证完成前沿不变量在首错之前的检出。
#[cfg(test)]
fn drop_preview_slot_at(position: usize) -> PreviewSlotGapGuard {
    PreviewSlotGapGuard(PREVIEW_SLOT_GAP.with(|gap| gap.replace(Some(position))))
}

/// 车辆在一个 stateful maneuver occurrence 中的已提交阶段。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManeuverTraversalPhase {
    /// 已进入 occurrence，但尚未跨过下一道 Gate。
    PreGate {
        /// 下一道待跨 Gate 的 hop 下标。
        next_gate_hop: u32,
    },
    /// 已跨过至少一道 Gate，当前未因 release Gate 等待。
    Committed {
        /// 最近跨过的 Gate 的 hop 下标。
        last_crossed_gate_hop: u32,
    },
    /// 已到达所持 membership 的 release Gate，且该 Gate 是最终硬约束归因。
    Waiting {
        /// 作为最终硬约束的 release Gate hop 下标。
        release_gate_hop: u32,
    },
    /// 已 crossing，继续持有 Conflict/downstream authority，直到车尾清空全部 coverage。
    /// reservation 内容只由 `ConflictArbiter` 按 vehicle owner 保存；这里仅保留
    /// 与 Waiting traversal 共用的 admission Gate 路线锚点。
    Clearing {
        /// 与 Waiting traversal 共用的准入 Gate hop 下标。
        admission_gate_hop: u32,
    },
}

/// 车辆当前 stateful maneuver occurrence 的语义状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManeuverTraversalState {
    pub(crate) route: RouteHandle,
    pub(crate) maneuver_occurrence_index: u32,
    pub(crate) phase: ManeuverTraversalPhase,
}

impl ManeuverTraversalState {
    /// 返回遍历所属的路线句柄。
    #[must_use]
    pub const fn route(self) -> RouteHandle {
        self.route
    }

    /// 返回机动出现项下标。
    #[must_use]
    pub const fn maneuver_occurrence_index(self) -> u32 {
        self.maneuver_occurrence_index
    }

    /// 返回当前遍历阶段。
    #[must_use]
    pub const fn phase(self) -> ManeuverTraversalPhase {
        self.phase
    }
}

/// 车辆持有的 WaitingZone 语义 membership。队列 link 不属于该持久语义。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitingMembership {
    pub(crate) waiting_zone: WaitingZoneOrdinal,
    pub(crate) admission_sequence: u64,
    pub(crate) release_hop: u32,
}

impl WaitingMembership {
    /// 返回 membership 所属的等待区序号。
    #[must_use]
    pub const fn waiting_zone(self) -> WaitingZoneOrdinal {
        self.waiting_zone
    }

    /// 返回准入时分配的准入序号。
    #[must_use]
    pub const fn admission_sequence(self) -> u64 {
        self.admission_sequence
    }

    /// 返回释放 membership 的 release hop 下标。
    #[must_use]
    pub const fn release_hop(self) -> u32 {
        self.release_hop
    }
}

/// WaitingZone 的只读已提交计数。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitingZoneSnapshot {
    pub(crate) zone: WaitingZoneOrdinal,
    pub(crate) occupancy: u32,
    pub(crate) max_occupancy: u32,
    pub(crate) next_admission_sequence: u64,
}

impl WaitingZoneSnapshot {
    /// 返回等待区序号。
    #[must_use]
    pub const fn zone(self) -> WaitingZoneOrdinal {
        self.zone
    }

    /// 返回当前占用数（辆）。
    #[must_use]
    pub const fn occupancy(self) -> u32 {
        self.occupancy
    }

    /// 返回容量上限（辆）。
    #[must_use]
    pub const fn max_occupancy(self) -> u32 {
        self.max_occupancy
    }

    /// 返回下一次准入将分配的准入序号。
    #[must_use]
    pub const fn next_admission_sequence(self) -> u64 {
        self.next_admission_sequence
    }
}

/// 按 zone、admission sequence 排列的只读 member 行。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitingZoneMember {
    pub(crate) zone: WaitingZoneOrdinal,
    pub(crate) vehicle: VehicleHandle,
    pub(crate) admission_sequence: u64,
    pub(crate) release_hop: u32,
}

impl WaitingZoneMember {
    /// 返回成员所属的等待区序号。
    #[must_use]
    pub const fn zone(self) -> WaitingZoneOrdinal {
        self.zone
    }

    /// 返回成员车辆句柄。
    #[must_use]
    pub const fn vehicle(self) -> VehicleHandle {
        self.vehicle
    }

    /// 返回准入时分配的准入序号。
    #[must_use]
    pub const fn admission_sequence(self) -> u64 {
        self.admission_sequence
    }

    /// 返回释放 membership 的 release hop 下标。
    #[must_use]
    pub const fn release_hop(self) -> u32 {
        self.release_hop
    }
}

/// Waiting admission 没有取得 claim 的原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitingNoGrantReason {
    /// 等待区占用达到 `max_occupancy`。
    Capacity,
    /// 已占用存储、最小间隙与车长之和超过该入口的本地存储长度。
    PhysicalStorage,
    /// 本地可准入，但组合资源未取得。
    CombinedResource(crate::ConflictNoGrantReason),
}

/// 刚完成 successful tick 的 Waiting admission 决定。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitingDecisionOutcome {
    /// 本地预选可行，但该 entry 未进入本 tick 的组合资源求值。
    Deferred,
    /// 车辆停在限制性 Gate 边界，本拍未做准入求值。
    NotEvaluated,
    /// Gate 非限制（如释放门）或未触发接触/越界，无需准入求值；车辆不必停在
    /// 边界。
    NotRequired,
    /// 仲裁授予入场；为阶段性授予——运动投影回落到门内时不分配准入序号、
    /// 不提交成员关系。
    Granted,
    /// 未取得准入，附拒绝原因。
    NoGrant(WaitingNoGrantReason),
}

/// Waiting decision 的稳定 route anchor。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitingRouteAnchor {
    pub(crate) route: RouteHandle,
    pub(crate) maneuver_occurrence_index: u32,
    pub(crate) hop: u32,
}

impl WaitingRouteAnchor {
    /// 返回锚点所属的路线句柄。
    #[must_use]
    pub const fn route(self) -> RouteHandle {
        self.route
    }

    /// 返回机动出现项下标。
    #[must_use]
    pub const fn maneuver_occurrence_index(self) -> u32 {
        self.maneuver_occurrence_index
    }

    /// 返回锚点所在的路线 hop 下标。
    #[must_use]
    pub const fn hop(self) -> u32 {
        self.hop
    }
}

/// 一条 Waiting admission 决定。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitingDecision {
    pub(crate) vehicle: VehicleHandle,
    pub(crate) vehicle_update_sequence: u32,
    pub(crate) zone: Option<WaitingZoneOrdinal>,
    pub(crate) anchor: WaitingRouteAnchor,
    pub(crate) outcome: WaitingDecisionOutcome,
}

impl WaitingDecision {
    /// 返回决定针对的车辆句柄。
    #[must_use]
    pub const fn vehicle(self) -> VehicleHandle {
        self.vehicle
    }

    /// 返回车辆在稳定更新顺序中的下标。
    #[must_use]
    pub const fn vehicle_update_sequence(self) -> u32 {
        self.vehicle_update_sequence
    }

    /// 返回决定涉及的等待区；决定未绑定具体 zone 时为 `None`。
    #[must_use]
    pub const fn zone(self) -> Option<WaitingZoneOrdinal> {
        self.zone
    }

    /// 返回决定的稳定 route 锚点。
    #[must_use]
    pub const fn anchor(self) -> WaitingRouteAnchor {
        self.anchor
    }

    /// 返回本拍决定结果。
    #[must_use]
    pub const fn outcome(self) -> WaitingDecisionOutcome {
        self.outcome
    }
}

/// 前保险杠被投影到 Waiting entry boundary 的原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitingProjectionReason {
    /// 下一等待区入口已进入本拍求值时窗，前沿先投影到其入口边界。
    EvaluationHorizon,
    /// 等待区容量不足，前沿投影到入口边界停车。
    Capacity,
    /// 等待区物理存储不足，前沿投影到入口边界停车。
    PhysicalStorage,
}

/// `despawn_vehicle` 同步回显的 Waiting membership 释放。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitingMembershipReleaseRecord {
    pub(crate) waiting_zone: WaitingZoneOrdinal,
    pub(crate) route_anchor: WaitingRouteAnchor,
    pub(crate) admission_sequence: u64,
}

impl WaitingMembershipReleaseRecord {
    /// 返回被释放的等待区序号。
    #[must_use]
    pub const fn waiting_zone(self) -> WaitingZoneOrdinal {
        self.waiting_zone
    }

    /// 返回 membership 的稳定 route 锚点。
    #[must_use]
    pub const fn route_anchor(self) -> WaitingRouteAnchor {
        self.route_anchor
    }

    /// 返回被释放 membership 的准入序号。
    #[must_use]
    pub const fn admission_sequence(self) -> u64 {
        self.admission_sequence
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct WaitingZoneState {
    pub occupancy: u32,
    pub next_admission_sequence: u64,
}

/// 从 membership 和 admission sequence 派生的队列两端。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct WaitingQueueEnds {
    pub head: Option<VehicleHandle>,
    pub tail: Option<VehicleHandle>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct WaitingQueueLink {
    pub previous: Option<VehicleHandle>,
    pub next: Option<VehicleHandle>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WaitingAdmissionClaim {
    pub vehicle: VehicleHandle,
    pub vehicle_update_sequence: u32,
    pub occurrence_index: u32,
    pub zone: WaitingZoneOrdinal,
    pub entry_hop: u32,
    pub release_hop: u32,
    pub approach_distance_mm: u32,
    pub plan_index: u32,
    pub post_step_group: u8,
    pub post_step_rank: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WaitingVehiclePlan {
    pub vehicle: VehicleHandle,
    pub vehicle_update_sequence: u32,
    pub occurrence_index: u32,
    pub zone: WaitingZoneOrdinal,
    pub maneuver_index: u32,
    pub entry_hop: u32,
    pub release_hop: u32,
    pub approach_distance_mm: u32,
    pub preview_route_edge_index: u32,
    pub decision: WaitingDecisionOutcome,
    pub stop_hop: Option<u32>,
    pub stop_zone: Option<WaitingZoneOrdinal>,
    pub stop_maneuver_index: Option<u32>,
    pub projection: Option<WaitingProjectionReason>,
    pub admission_sequence: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WaitingStopConstraint {
    pub distance: laneflow_static_network::BoundedDistance,
    pub hop: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WaitingBindingError {
    VehicleTooLong,
    StatefulManeuverInterior,
    InvalidRoute,
    AuthorityMismatch,
    ParkingConflict,
}

impl crate::kernel::state::WorldState {
    pub(crate) fn validate_waiting_parking_anchor(
        &self,
        route: RouteHandle,
        route_occurrence: u32,
    ) -> Result<(), WaitingBindingError> {
        self.read_view()
            .validate_waiting_parking_anchor(route, route_occurrence)
    }

    pub(crate) fn rebind_waiting_authority(
        &self,
        state: crate::VehicleState,
        new_route: RouteHandle,
        new_cursor: usize,
    ) -> Result<(Option<ManeuverTraversalState>, Option<WaitingMembership>), WaitingBindingError>
    {
        let Some(old_traversal) = state.maneuver_traversal else {
            if state.waiting_membership.is_some() {
                return Err(WaitingBindingError::AuthorityMismatch);
            }
            return self
                .validate_waiting_bootstrap(new_route, new_cursor, state.length_mm)
                .map(|traversal| (traversal, None));
        };
        let old_compiled = self
            .compiled_route(state.route)
            .ok_or(WaitingBindingError::InvalidRoute)?;
        let old_maneuver = old_compiled
            .maneuvers
            .get(old_traversal.maneuver_occurrence_index as usize)
            .ok_or(WaitingBindingError::AuthorityMismatch)?;
        let new_compiled = self
            .compiled_route(new_route)
            .ok_or(WaitingBindingError::InvalidRoute)?;
        let new_cursor_u32 =
            u32::try_from(new_cursor).map_err(|_| WaitingBindingError::InvalidRoute)?;
        let (new_maneuver_index, new_maneuver) = new_compiled
            .maneuvers
            .iter()
            .enumerate()
            .find(|(_, maneuver)| {
                maneuver.path == old_maneuver.path
                    && new_cursor_u32 >= maneuver.entry_route_edge_index
                    && new_cursor_u32 < maneuver.exit_route_edge_index
            })
            .ok_or(WaitingBindingError::AuthorityMismatch)?;

        let map_hop = |old_hop: u32| -> Result<u32, WaitingBindingError> {
            let gate = old_compiled
                .hop_gate
                .get(old_hop as usize)
                .copied()
                .flatten()
                .ok_or(WaitingBindingError::AuthorityMismatch)?;
            (new_maneuver.entry_route_edge_index..new_maneuver.exit_route_edge_index)
                .find(|hop| {
                    new_compiled.hop_gate.get(*hop as usize).copied().flatten() == Some(gate)
                })
                .ok_or(WaitingBindingError::AuthorityMismatch)
        };
        let phase = match old_traversal.phase {
            ManeuverTraversalPhase::PreGate { next_gate_hop } => {
                let mapped = map_hop(next_gate_hop)?;
                if new_cursor_u32 > mapped {
                    return Err(WaitingBindingError::AuthorityMismatch);
                }
                ManeuverTraversalPhase::PreGate {
                    next_gate_hop: mapped,
                }
            }
            ManeuverTraversalPhase::Committed {
                last_crossed_gate_hop,
            } => {
                let mapped = map_hop(last_crossed_gate_hop)?;
                if new_cursor_u32 <= mapped {
                    return Err(WaitingBindingError::AuthorityMismatch);
                }
                ManeuverTraversalPhase::Committed {
                    last_crossed_gate_hop: mapped,
                }
            }
            ManeuverTraversalPhase::Waiting { release_gate_hop } => {
                let mapped = map_hop(release_gate_hop)?;
                if new_cursor_u32 != mapped {
                    return Err(WaitingBindingError::AuthorityMismatch);
                }
                ManeuverTraversalPhase::Waiting {
                    release_gate_hop: mapped,
                }
            }
            ManeuverTraversalPhase::Clearing { .. } => {
                return Err(WaitingBindingError::AuthorityMismatch);
            }
        };
        let membership = match state.waiting_membership {
            None => None,
            Some(old_membership) => {
                let old_waiting = old_compiled
                    .waiting
                    .iter()
                    .find(|waiting| {
                        waiting.maneuver_index == old_traversal.maneuver_occurrence_index
                            && waiting.zone == old_membership.waiting_zone
                            && waiting.release_hop == old_membership.release_hop
                    })
                    .ok_or(WaitingBindingError::AuthorityMismatch)?;
                let new_waiting = new_compiled
                    .waiting
                    .iter()
                    .find(|waiting| {
                        waiting.maneuver_index as usize == new_maneuver_index
                            && waiting.zone == old_waiting.zone
                            && new_compiled
                                .hop_gate
                                .get(waiting.entry_hop as usize)
                                .copied()
                                .flatten()
                                == old_compiled
                                    .hop_gate
                                    .get(old_waiting.entry_hop as usize)
                                    .copied()
                                    .flatten()
                            && new_compiled
                                .hop_gate
                                .get(waiting.release_hop as usize)
                                .copied()
                                .flatten()
                                == old_compiled
                                    .hop_gate
                                    .get(old_waiting.release_hop as usize)
                                    .copied()
                                    .flatten()
                    })
                    .ok_or(WaitingBindingError::AuthorityMismatch)?;
                if !waiting_membership_cursor_valid(new_cursor_u32, new_waiting) {
                    return Err(WaitingBindingError::AuthorityMismatch);
                }
                Some(WaitingMembership {
                    waiting_zone: old_membership.waiting_zone,
                    admission_sequence: old_membership.admission_sequence,
                    release_hop: new_waiting.release_hop,
                })
            }
        };
        for waiting in &new_compiled.waiting {
            let is_held = membership.is_some_and(|member| {
                member.waiting_zone == waiting.zone
                    && member.release_hop == waiting.release_hop
                    && waiting.maneuver_index as usize == new_maneuver_index
            });
            if !is_held
                && new_cursor_u32 <= waiting.release_hop
                && state.length_mm > waiting.storage_length_mm
            {
                return Err(WaitingBindingError::VehicleTooLong);
            }
        }
        Ok((
            Some(ManeuverTraversalState {
                route: new_route,
                maneuver_occurrence_index: u32::try_from(new_maneuver_index)
                    .map_err(|_| WaitingBindingError::InvalidRoute)?,
                phase,
            }),
            membership,
        ))
    }

    #[cfg(test)]
    pub(crate) fn prepare_waiting_step(&mut self, delta_s: f32) -> Result<(), crate::StepError> {
        self.step_workspace().prepare_waiting_step(delta_s, None)
    }

    #[cfg(test)]
    pub(crate) fn finalize_waiting_step(
        &mut self,
        updates: &mut [(usize, crate::VehicleState)],
    ) -> Result<(), crate::StepError> {
        self.step_workspace().finalize_waiting_step(updates)
    }

    #[cfg(test)]
    pub(crate) fn finalize_waiting_outputs(
        &mut self,
        updates: &[(usize, crate::VehicleState)],
        tick: u64,
    ) -> Result<(), crate::StepError> {
        self.step_workspace()
            .finalize_waiting_outputs(updates, tick)
    }

    pub(crate) fn derive_waiting_traversal_with_signals(
        &self,
        state: crate::VehicleState,
        apply_current_signals: bool,
    ) -> Result<Option<ManeuverTraversalState>, crate::StepError> {
        self.read_view()
            .derive_waiting_traversal_with_signals(state, apply_current_signals)
    }

    /// 为没有既有 Waiting authority 的 Active 候选建立唯一可推导的初始状态。
    pub(crate) fn validate_waiting_bootstrap(
        &self,
        route: RouteHandle,
        cursor: usize,
        vehicle_length_mm: u32,
    ) -> Result<Option<ManeuverTraversalState>, WaitingBindingError> {
        let compiled = self
            .compiled_route(route)
            .ok_or(WaitingBindingError::InvalidRoute)?;
        let cursor_u32 = u32::try_from(cursor).map_err(|_| WaitingBindingError::InvalidRoute)?;

        for occurrence in &compiled.waiting {
            #[cfg(test)]
            count_waiting_lookup(0);
            if cursor_u32 <= occurrence.release_hop
                && vehicle_length_mm > occurrence.storage_length_mm
            {
                return Err(WaitingBindingError::VehicleTooLong);
            }
        }

        // 注册期已拒绝相交的机动半开区间；当前位置最多对应一个出现项。
        let Some(maneuver_index) = maneuver_index_at_hop(compiled, cursor_u32) else {
            return Ok(None);
        };
        if compiled
            .waiting
            .binary_search_by_key(&maneuver_index, |waiting| {
                #[cfg(test)]
                count_waiting_lookup(2);
                waiting.maneuver_index as usize
            })
            .is_err()
        {
            return Ok(None);
        }
        let maneuver = &compiled.maneuvers[maneuver_index];
        let first_gate_hop =
            first_gate_hop(compiled, maneuver).ok_or(WaitingBindingError::InvalidRoute)?;
        if cursor_u32 > first_gate_hop {
            return Err(WaitingBindingError::StatefulManeuverInterior);
        }
        Ok(Some(ManeuverTraversalState {
            route,
            maneuver_occurrence_index: u32::try_from(maneuver_index)
                .map_err(|_| WaitingBindingError::InvalidRoute)?,
            phase: ManeuverTraversalPhase::PreGate {
                next_gate_hop: first_gate_hop,
            },
        }))
    }

    pub(crate) fn waiting_state_valid(&self) -> bool {
        if self.committed.live_order.iter().any(|vehicle| {
            self.vehicle_state(*vehicle).is_some_and(|state| {
                self.conflict_reservation(*vehicle).is_some() && state.waiting_membership.is_some()
            })
        }) {
            return false;
        }
        let mut total = 0_usize;
        for zone_index in 0..self.committed.waiting_zones.len() {
            let zone = WaitingZoneOrdinal::from_raw(
                u32::try_from(zone_index).expect("waiting zone index fits u32"),
            );
            let Some(count) = self.waiting_zone_member_count(zone) else {
                return false;
            };
            total = match total.checked_add(count) {
                Some(value) if value <= self.committed.live_order.len() => value,
                _ => return false,
            };
        }
        self.waiting_semantic_member_count() == total
    }

    /// 稳态从已有 member batch 定位非空 zone，而非遍历静态表。队列和语义仍交叉
    /// 验证；未涉及的空 zone 保留历史 counter，由 restore/cutover 做全量验证。
    #[cfg(test)]
    fn waiting_member_rows_valid(&self) -> bool {
        self.read_view().waiting_member_rows_valid()
    }

    /// 验证一个实际涉及的 zone；全量冷路径和稀疏热路径共享同一队列合同。
    fn waiting_zone_member_count(&self, zone: WaitingZoneOrdinal) -> Option<usize> {
        self.read_view().waiting_zone_member_count(zone)
    }

    fn waiting_semantic_member_count(&self) -> usize {
        self.read_view().waiting_semantic_member_count()
    }

    pub(crate) fn restored_waiting_authority_valid(&self, state: crate::VehicleState) -> bool {
        // phase 是产生该状态的 tick-start Gate 归因，不以恢复/新修订的当前信号改判历史。
        let clearing = matches!(
            state.maneuver_traversal,
            Some(ManeuverTraversalState {
                phase: ManeuverTraversalPhase::Clearing { .. },
                ..
            })
        );
        if clearing {
            // Clearing 的完整 owner/passages/downstream 由 Conflict 聚合校验；
            // Waiting 侧只证明它没有 membership。跨修订新增的 Waiting 覆盖不得
            // 把已经 crossing 的 Conflict owner 重新解释为 Waiting member；其物理
            // 占用由既有 downstream reservation 继续保护。
            return state.status == crate::VehicleStatus::Active
                && state.waiting_membership.is_none();
        } else {
            let Ok(mut expected) = self.derive_waiting_traversal_with_signals(state, false) else {
                return false;
            };
            if let Some(ManeuverTraversalState {
                phase: ManeuverTraversalPhase::Waiting { release_gate_hop },
                ..
            }) = state.maneuver_traversal
            {
                let Some(membership) = state.waiting_membership else {
                    return false;
                };
                let Some(compiled) = self.compiled_route(state.route) else {
                    return false;
                };
                if release_gate_hop != membership.release_hop
                    || state.speed_mm_s != 0
                    || state.carry_um != 0
                    || !front_at_hop_boundary(
                        compiled,
                        &state,
                        release_gate_hop,
                        self.binding.revision.traffic().lane_lengths_millimetres(),
                    )
                {
                    return false;
                }
                let Some(traversal) = expected.as_mut() else {
                    return false;
                };
                if !matches!(traversal.phase, ManeuverTraversalPhase::Committed { .. }) {
                    return false;
                }
                traversal.phase = ManeuverTraversalPhase::Waiting { release_gate_hop };
            }
            if expected != state.maneuver_traversal {
                return false;
            }
        }
        let Some(compiled) = self.compiled_route(state.route) else {
            return false;
        };
        if state.status == crate::VehicleStatus::Active
            && compiled.waiting.iter().any(|occurrence| {
                state.route_edge_index <= occurrence.release_hop
                    && state.length_mm > occurrence.storage_length_mm
            })
        {
            return false;
        }
        let Some(membership) = state.waiting_membership else {
            // 目标修订可以新增区间，但不能让 Active cursor 在区间内凭空获得 storage。
            // Parked / Completed 的保留 cursor 不代表一次 Waiting 进入。
            let next = compiled
                .waiting
                .partition_point(|occurrence| occurrence.release_hop < state.route_edge_index);
            return state.status != crate::VehicleStatus::Active
                || compiled.waiting.get(next).is_none_or(|occurrence| {
                    !waiting_membership_cursor_valid(state.route_edge_index, occurrence)
                });
        };
        let Some(traversal) = state.maneuver_traversal else {
            return false;
        };
        compiled.waiting.iter().any(|occurrence| {
            occurrence.maneuver_index == traversal.maneuver_occurrence_index
                && occurrence.zone == membership.waiting_zone
                && occurrence.release_hop == membership.release_hop
                && waiting_membership_cursor_valid(state.route_edge_index, occurrence)
                && state.length_mm <= occurrence.storage_length_mm
        })
    }

    pub(crate) fn waiting_snapshot_storage_valid(&self) -> bool {
        for queue in self.derived.waiting_queue_ends.iter().copied() {
            let mut used = 0_u64;
            let mut current = queue.head;
            let mut has_front = false;
            let mut previous_rank = None;
            let mut previous_length_mm = 0_u32;
            while let Some(vehicle) = current {
                let Some(vehicle_state) = self.vehicle_state(vehicle) else {
                    return false;
                };
                let Some(profile) = self
                    .binding
                    .revision
                    .traffic()
                    .relations()
                    .vehicle_profile(vehicle_state.profile)
                else {
                    return false;
                };
                if has_front {
                    let Some(next) = used.checked_add(u64::from(profile.min_gap_mm())) else {
                        return false;
                    };
                    used = next;
                }
                let Some(next) = used.checked_add(u64::from(vehicle_state.length_mm)) else {
                    return false;
                };
                used = next;
                let Some(membership) = vehicle_state.waiting_membership else {
                    return false;
                };
                let Some(traversal) = vehicle_state.maneuver_traversal else {
                    return false;
                };
                let Some((compiled, occurrence)) = self
                    .compiled_route(vehicle_state.route)
                    .and_then(|compiled| {
                        compiled
                            .waiting
                            .iter()
                            .find(|occurrence| {
                                occurrence.maneuver_index == traversal.maneuver_occurrence_index
                                    && occurrence.zone == membership.waiting_zone
                                    && occurrence.release_hop == membership.release_hop
                            })
                            .map(|occurrence| (compiled, occurrence))
                    })
                else {
                    return false;
                };
                let Some(rank) =
                    post_step_physical_rank(compiled, vehicle_state, occurrence.release_hop)
                else {
                    return false;
                };
                if previous_rank.is_some_and(|previous| previous >= rank) {
                    return false;
                }
                if let Some(front_rank) = previous_rank {
                    let Some(front_distance_mm) = waiting_front_distance_mm(front_rank, rank)
                    else {
                        return false;
                    };
                    let Some(required_distance_mm) =
                        u64::from(previous_length_mm).checked_add(u64::from(profile.min_gap_mm()))
                    else {
                        return false;
                    };
                    if front_distance_mm < required_distance_mm {
                        return false;
                    }
                }
                if used > u64::from(occurrence.storage_length_mm) {
                    return false;
                }
                previous_rank = Some(rank);
                previous_length_mm = vehicle_state.length_mm;
                has_front = true;
                current = self.derived.waiting_links[vehicle.index() as usize].next;
            }
        }
        true
    }

    pub(crate) fn rebuild_waiting_member_rows(&mut self) {
        self.committed_mut().rebuild_waiting_member_rows()
    }

    pub(crate) fn rebuild_waiting_aggregate_from_semantics(&mut self) -> bool {
        self.derived.waiting_member_rows.clear();
        for vehicle in self.committed.live_order.iter().copied() {
            let Some(state) = self.vehicle_state(vehicle) else {
                return false;
            };
            if let Some(membership) = state.waiting_membership {
                if self.derived.waiting_member_rows.len()
                    == self.derived.waiting_member_rows.capacity()
                {
                    return false;
                }
                self.derived.waiting_member_rows.push(WaitingZoneMember {
                    zone: membership.waiting_zone,
                    vehicle,
                    admission_sequence: membership.admission_sequence,
                    release_hop: membership.release_hop,
                });
            }
        }
        self.derived
            .waiting_member_rows
            .sort_unstable_by_key(|member| {
                (
                    member.zone.raw(),
                    member.admission_sequence,
                    member.vehicle.index(),
                    member.vehicle.generation(),
                )
            });
        if self.derived.waiting_member_rows.windows(2).any(|pair| {
            pair[0].zone == pair[1].zone && pair[0].admission_sequence == pair[1].admission_sequence
        }) {
            return false;
        }
        for state in &mut self.committed.waiting_zones {
            state.occupancy = 0;
        }
        self.derived
            .waiting_queue_ends
            .fill(WaitingQueueEnds::default());
        self.derived.waiting_links.fill(WaitingQueueLink::default());
        for index in 0..self.derived.waiting_member_rows.len() {
            let member = self.derived.waiting_member_rows[index];
            let Some(zone) = self.committed.waiting_zones.get(member.zone.index()) else {
                return false;
            };
            if member.admission_sequence >= zone.next_admission_sequence {
                return false;
            }
            self.append_waiting_member(
                member.vehicle,
                WaitingMembership {
                    waiting_zone: member.zone,
                    admission_sequence: member.admission_sequence,
                    release_hop: member.release_hop,
                },
            );
        }
        self.waiting_state_valid() && self.waiting_snapshot_storage_valid()
    }

    pub(crate) fn unlink_waiting_member(
        &mut self,
        vehicle: VehicleHandle,
        membership: WaitingMembership,
    ) {
        self.committed_mut()
            .unlink_waiting_member(vehicle, membership)
    }

    pub(crate) fn append_waiting_member(
        &mut self,
        vehicle: VehicleHandle,
        membership: WaitingMembership,
    ) {
        self.committed_mut()
            .append_waiting_member(vehicle, membership)
    }
}

impl<'a> crate::kernel::phase::StepReadView<'a> {
    pub(crate) fn validate_waiting_parking_anchor(
        self,
        route: RouteHandle,
        route_occurrence: u32,
    ) -> Result<(), WaitingBindingError> {
        let compiled = self
            .compiled_route(route)
            .ok_or(WaitingBindingError::InvalidRoute)?;
        let cursor = route_occurrence;
        if compiled
            .maneuvers
            .iter()
            .enumerate()
            .any(|(index, maneuver)| {
                cursor >= maneuver.entry_route_edge_index
                    && cursor < maneuver.exit_route_edge_index
                    && compiled
                        .waiting
                        .iter()
                        .any(|waiting| waiting.maneuver_index as usize == index)
            })
        {
            return Err(WaitingBindingError::ParkingConflict);
        }
        Ok(())
    }

    pub(crate) fn derive_waiting_traversal(
        self,
        state: crate::VehicleState,
    ) -> Result<Option<ManeuverTraversalState>, crate::StepError> {
        self.derive_waiting_traversal_with_signals(state, true)
    }

    pub(crate) fn derive_waiting_traversal_with_signals(
        self,
        state: crate::VehicleState,
        apply_current_signals: bool,
    ) -> Result<Option<ManeuverTraversalState>, crate::StepError> {
        if state.status != crate::VehicleStatus::Active {
            return Ok(None);
        }
        let compiled = self
            .compiled_route(state.route)
            .ok_or(crate::StepError::WaitingInvariantViolation)?;
        if compiled.waiting.is_empty() {
            return Ok(None);
        }
        let cursor = state.route_edge_index;
        let Some(maneuver_index) = maneuver_index_at_hop(compiled, cursor) else {
            return Ok(None);
        };
        if compiled
            .waiting
            .binary_search_by_key(&maneuver_index, |waiting| waiting.maneuver_index as usize)
            .is_err()
        {
            return Ok(None);
        }
        let maneuver = &compiled.maneuvers[maneuver_index];
        let first_gate = first_gate_hop(compiled, maneuver)
            .ok_or(crate::StepError::WaitingInvariantViolation)?;
        let gate_index = compiled.gate_hops.partition_point(|hop| *hop < cursor);
        let last_crossed = gate_index
            .checked_sub(1)
            .and_then(|index| compiled.gate_hops.get(index).copied())
            .filter(|hop| *hop >= maneuver.entry_route_edge_index);
        let next_gate = compiled
            .gate_hops
            .get(gate_index)
            .copied()
            .filter(|hop| *hop < maneuver.exit_route_edge_index);
        let phase = if let Some(membership) = state.waiting_membership {
            let release_gate = compiled
                .hop_gate
                .get(membership.release_hop as usize)
                .copied()
                .flatten()
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            if front_at_hop_boundary(
                compiled,
                &state,
                membership.release_hop,
                self.binding.revision.traffic().lane_lengths_millimetres(),
            ) && apply_current_signals
                && self.gate_is_restrictive(release_gate, state.profile)
            {
                ManeuverTraversalPhase::Waiting {
                    release_gate_hop: membership.release_hop,
                }
            } else if let Some(last_crossed) = last_crossed {
                ManeuverTraversalPhase::Committed {
                    last_crossed_gate_hop: last_crossed,
                }
            } else {
                ManeuverTraversalPhase::PreGate {
                    next_gate_hop: next_gate.unwrap_or(first_gate),
                }
            }
        } else if let Some(last_crossed) = last_crossed {
            ManeuverTraversalPhase::Committed {
                last_crossed_gate_hop: last_crossed,
            }
        } else {
            ManeuverTraversalPhase::PreGate {
                next_gate_hop: next_gate.unwrap_or(first_gate),
            }
        };
        Ok(Some(ManeuverTraversalState {
            route: state.route,
            maneuver_occurrence_index: u32::try_from(maneuver_index)
                .map_err(|_| crate::StepError::WaitingInvariantViolation)?,
            phase,
        }))
    }

    /// 稳态从已有 member batch 定位非空 zone，而非遍历静态表。队列和语义仍交叉
    /// 验证；未涉及的空 zone 保留历史 counter，由 restore/cutover 做全量验证。
    pub(crate) fn waiting_member_rows_valid(self) -> bool {
        if self.derived.waiting_member_rows.windows(2).any(|pair| {
            (pair[0].zone.raw(), pair[0].admission_sequence)
                >= (pair[1].zone.raw(), pair[1].admission_sequence)
        }) {
            return false;
        }
        for members in self
            .derived
            .waiting_member_rows
            .chunk_by(|left, right| left.zone == right.zone)
        {
            if self.waiting_zone_member_count(members[0].zone) != Some(members.len()) {
                return false;
            }
            let mut current = self.derived.waiting_queue_ends[members[0].zone.index()].head;
            for member in members {
                if current != Some(member.vehicle)
                    || self
                        .vehicle_state(member.vehicle)
                        .and_then(|state| state.waiting_membership)
                        != Some(WaitingMembership {
                            waiting_zone: member.zone,
                            admission_sequence: member.admission_sequence,
                            release_hop: member.release_hop,
                        })
                {
                    return false;
                }
                current = self.derived.waiting_links[member.vehicle.index() as usize].next;
            }
        }
        self.waiting_semantic_member_count() == self.derived.waiting_member_rows.len()
    }

    /// 验证一个实际涉及的 zone；全量冷路径和稀疏热路径共享同一队列合同。
    pub(crate) fn waiting_zone_member_count(self, zone: WaitingZoneOrdinal) -> Option<usize> {
        #[cfg(test)]
        count_waiting_work(|counts| counts.checked_zones += 1);
        let state = self.committed.waiting_zones.get(zone.index())?;
        let queue = self.derived.waiting_queue_ends.get(zone.index())?;
        let view = self
            .binding
            .revision
            .traffic()
            .relations()
            .waiting_zone(zone)?;
        if state.occupancy > view.max_occupancy()
            || (state.occupancy == 0) != (queue.head.is_none() && queue.tail.is_none())
        {
            return None;
        }
        let mut previous = None;
        let mut current = queue.head;
        let mut count = 0_u32;
        let mut previous_sequence = None;
        while let Some(vehicle) = current {
            let membership = self.vehicle_state(vehicle)?.waiting_membership?;
            if membership.waiting_zone != zone
                || membership.admission_sequence >= state.next_admission_sequence
                || previous_sequence.is_some_and(|value| value >= membership.admission_sequence)
            {
                return None;
            }
            let link = self.derived.waiting_links.get(vehicle.index() as usize)?;
            if link.previous != previous {
                return None;
            }
            previous = Some(vehicle);
            previous_sequence = Some(membership.admission_sequence);
            current = link.next;
            count = count.checked_add(1)?;
            if count as usize > self.committed.live_order.len() {
                return None;
            }
        }
        (count == state.occupancy && previous == queue.tail).then_some(count as usize)
    }

    pub(crate) fn waiting_semantic_member_count(self) -> usize {
        self.committed
            .live_order
            .iter()
            .filter(|vehicle| {
                self.vehicle_state(**vehicle)
                    .is_some_and(|state| state.waiting_membership.is_some())
            })
            .count()
    }
}

/// P2 活动数低于该阈值时不走向量分发，由协调器内联融合求值同一原语；只决定
/// 本阶段谁执行，不改变语义、首错位置或输出。初版保守值：机制测量（合成
/// multi-gate 场景、每车预览 ~185ns）显示分发净收益的交叉区下沿约一千
/// 活动，取 1_024 的保守一侧；待 #707 城市证据收敛后再校准。测试小场景
/// 用 `force_preview_dispatch` 覆盖（cfg(test)）。
const WAITING_PREVIEW_DISPATCH_MIN_ACTIVE: usize = 1_024;

/// 规范消费一个已完成预览：按现行 staging 规则写 `motion_cache`（受
/// `cache_limit` 容量降级约束）与 `next_states`（仅预览存在时写入）。
fn stage_waiting_preview(
    motion_cache: &mut Vec<crate::kernel::tick::MotionCacheEntry>,
    next_states: &mut Vec<(usize, crate::VehicleState)>,
    cache_index: usize,
    cache_limit: usize,
    vehicle: crate::VehicleHandle,
    update_sequence: usize,
    entry: &crate::kernel::tick::WaitingPreviewEntry,
) {
    if cache_index < cache_limit {
        motion_cache.push(crate::kernel::tick::MotionCacheEntry {
            vehicle,
            update_sequence,
            horizon: entry.horizon,
            preview: entry.preview,
        });
    }
    if let Some(next) = entry.preview.map(|preview| preview.next) {
        next_states.push((update_sequence, next));
    }
}

/// 融合路径：在 live 序上逐车交错求值同一原语并就地规范消费（身份检查 →
/// Active 过滤 → 预览 → staging），不物化输入表；与改动前串行第一遍逐字节
/// 同序，首错位置一致。worker=1、小工作集与一切可选暂存预留失败都走这里。
fn prepare_waiting_previews_fused(
    workspace: &mut crate::kernel::state::TickWorkspace,
    view: crate::kernel::phase::StepReadView<'_>,
    delta_s: f32,
    cache_limit: usize,
) -> Result<(), crate::StepError> {
    #[cfg(test)]
    let _fused_loop = preview_stage::begin(preview_stage::FUSED_LOOP);
    let mut cache_index = 0;
    for (update_sequence, vehicle) in view.committed.live_order.iter().copied().enumerate() {
        let state = *view
            .vehicle_state(vehicle)
            .ok_or(crate::StepError::WaitingInvariantViolation)?;
        if state.status != crate::VehicleStatus::Active {
            continue;
        }
        let entry = view.waiting_preview_entry(vehicle, update_sequence, delta_s)?;
        stage_waiting_preview(
            &mut workspace.motion_cache,
            &mut workspace.next_states,
            cache_index,
            cache_limit,
            vehicle,
            update_sequence,
            &entry,
        );
        cache_index += 1;
    }
    Ok(())
}

/// 分发路径：输入发现循环先对输入表做 checked 预留（上限 live 长度），失败
/// 退回流式融合（不新增领域错误）；发现中遇身份错误停止收集更晚输入，把该
/// 错误记为待规范消费的终止位置——已收集前缀照常分发，协调器按序消费
/// （更早预览错误先返回），前缀全部成功后才公开该身份错误。如此首错与
/// 串行逐车交错一致：位置早于终止位置的义务先于该身份错误。小工作集同样
/// 退回流式融合。任务独占连续输出槽位、只读共享视图计算；首错之前出现
/// `Pending`/`Skipped` 属完成前沿不变量违例。
fn prepare_waiting_previews_dispatched(
    workspace: &mut crate::kernel::state::TickWorkspace,
    view: crate::kernel::phase::StepReadView<'_>,
    execution: &crate::kernel::execution::ExecutionResources,
    delta_s: f32,
    cache_limit: usize,
) -> Result<(), crate::StepError> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let inputs = &mut workspace.waiting_preview_inputs;
    inputs.clear();
    #[cfg(test)]
    let input_injected = preview_input_reserve_injected_failure();
    #[cfg(not(test))]
    let input_injected = false;
    // 上界用 Active 投影而非 live 长度：Completed 车辆会累积在 live_order，
    // 输入表只收 Active 配对，按 live 预留会在车辆完成时反复再分配。
    if inputs.try_reserve(view.derived.active_order.len()).is_err() || input_injected {
        // 输入表预留失败（冷态或增长）：退回同一领域原语的流式融合。
        #[cfg(test)]
        count_preview_path(|counts| {
            counts.slot_fallback += 1;
            counts.fused += 1;
        });
        return prepare_waiting_previews_fused(workspace, view, delta_s, cache_limit);
    }
    #[cfg(test)]
    let _discover = preview_stage::begin(preview_stage::PREAMBLE);
    let mut pending_identity_error = None;
    for (sequence, vehicle) in view.committed.live_order.iter().copied().enumerate() {
        let Some(state) = view.vehicle_state(vehicle) else {
            // 身份失败：更晚输入不再收集；先兑现已收集前缀的更早义务。
            pending_identity_error = Some(crate::StepError::WaitingInvariantViolation);
            break;
        };
        if state.status != crate::VehicleStatus::Active {
            continue;
        }
        inputs.push((vehicle, sequence));
    }
    #[cfg(test)]
    drop(_discover);
    let workload = inputs.len();
    #[cfg(test)]
    let forced = preview_dispatch_forced();
    #[cfg(not(test))]
    let forced = false;
    if workload < WAITING_PREVIEW_DISPATCH_MIN_ACTIVE && !(forced && workload > 0) {
        #[cfg(test)]
        count_preview_path(|counts| counts.fused += 1);
        return prepare_waiting_previews_fused(workspace, view, delta_s, cache_limit);
    }
    #[cfg(test)]
    count_preview_path(|counts| counts.dispatched += 1);
    let slots = &mut workspace.waiting_preview_slots;
    slots.clear();
    #[cfg(test)]
    let reserve_injected = preview_slot_reserve_injected_failure();
    #[cfg(not(test))]
    let reserve_injected = false;
    if slots.try_reserve(workload).is_err() || reserve_injected {
        // 可选并行暂存预留失败：退回同一领域原语的融合求值，不新增领域错误。
        #[cfg(test)]
        count_preview_path(|counts| counts.slot_fallback += 1);
        return prepare_waiting_previews_fused(workspace, view, delta_s, cache_limit);
    }
    slots.resize(workload, crate::kernel::execution::DispatchSlot::Pending);
    // 块数取线程数倍数与活动数的较小者，块数可多于线程数以便均衡；语义中立。
    // 倍数默认 2，cfg(test) 探针可扫描 1/2/4 以粒度证据调整。
    #[cfg(test)]
    let multiplier = preview_chunk_multiplier();
    #[cfg(not(test))]
    let multiplier = 2;
    let chunk_count = execution
        .dispatch_threads()
        .saturating_mul(multiplier)
        .clamp(1, workload);
    let chunk_size = workload.div_ceil(chunk_count).max(1);
    let first_error = AtomicUsize::new(usize::MAX);
    // 块级诊断记录槽：与输出块对齐，每块一个 u64 nanos，任务独占写入
    // （无竞争、无共享锁）；仅在诊断启用时分配，热态非诊断构建零成本。
    #[cfg(test)]
    let chunk_records = preview_stage::enabled().then(|| {
        (0..chunk_count)
            .map(|_| std::sync::atomic::AtomicU64::new(0))
            .collect::<Vec<_>>()
    });
    let compute = |chunk_view: crate::kernel::phase::StepReadView<'_>,
                   start: usize,
                   chunk: &mut [crate::kernel::execution::DispatchSlot<
        crate::kernel::tick::WaitingPreviewEntry,
    >]| {
        // 计时无条件开启；是否记录由协调器创建的 chunk_records 决定（普通
        // Option 捕获，跨线程一致）。不能用线程本地 ENABLE 作门——辅助
        // 线程读不到协调器的开关，会漏记（审阅阻断二的同类陷阱）。
        #[cfg(test)]
        let chunk_started = std::time::Instant::now();
        for (offset, slot) in chunk.iter_mut().enumerate() {
            let index = start + offset;
            let (vehicle, update_sequence) = workspace.waiting_preview_inputs[index];
            match chunk_view.waiting_preview_entry(vehicle, update_sequence, delta_s) {
                Ok(entry) => *slot = crate::kernel::execution::DispatchSlot::Done(Ok(entry)),
                Err(error) => {
                    first_error.fetch_min(index, Ordering::Relaxed);
                    *slot = crate::kernel::execution::DispatchSlot::Done(Err(error));
                    break;
                }
            }
        }
        #[cfg(test)]
        if let (started, Some(records)) = (chunk_started, &chunk_records) {
            // 每执行块恰好写一条记录；块计算远超 0ns，0 视作未记录。
            records[start / chunk_size].store(
                started.elapsed().as_nanos().max(1) as u64,
                Ordering::Relaxed,
            );
        }
    };
    #[cfg(test)]
    let _dispatch_scope = preview_stage::begin(preview_stage::DISPATCH_SCOPE);
    let dispatch_stats =
        execution.try_for_each_chunk(view, slots, &first_error, chunk_size, compute);
    #[cfg(test)]
    drop(_dispatch_scope);
    #[cfg(test)]
    if let Some(records) = &chunk_records {
        let mut recorded = 0_u128;
        let mut total = 0_u128;
        let mut longest = 0_u128;
        for record in records {
            let nanos = u128::from(record.load(Ordering::Relaxed));
            if nanos > 0 {
                recorded += 1;
                total += nanos;
                longest = longest.max(nanos);
            }
        }
        assert_eq!(
            recorded, dispatch_stats.dispatched_chunks as u128,
            "每执行块恰好一条块级诊断记录"
        );
        preview_stage::note_chunk_batch(total, longest);
    }
    #[cfg(test)]
    crate::kernel::execution::note_last_dispatch_stats(dispatch_stats);
    #[cfg(not(test))]
    let _ = dispatch_stats;
    #[cfg(test)]
    if let Some(position) = preview_slot_gap_position()
        && let Some(slot) = slots.get_mut(position)
    {
        // 完成前沿不变量注入：首错之前出现未计算槽位，协调器须检出而非成功。
        *slot = crate::kernel::execution::DispatchSlot::Pending;
    }
    #[cfg(test)]
    let _consume = preview_stage::begin(preview_stage::CONSUME);
    for (cache_index, ((vehicle, update_sequence), slot)) in workspace
        .waiting_preview_inputs
        .iter()
        .zip(slots.iter())
        .enumerate()
    {
        match slot {
            crate::kernel::execution::DispatchSlot::Done(Ok(entry)) => {
                stage_waiting_preview(
                    &mut workspace.motion_cache,
                    &mut workspace.next_states,
                    cache_index,
                    cache_limit,
                    *vehicle,
                    *update_sequence,
                    entry,
                );
            }
            crate::kernel::execution::DispatchSlot::Done(Err(error)) => return Err(*error),
            crate::kernel::execution::DispatchSlot::Pending
            | crate::kernel::execution::DispatchSlot::Skipped => {
                return Err(crate::StepError::WaitingInvariantViolation);
            }
        }
    }
    // 前缀全部成功才公开发现阶段记录的身份终止错误；更早预览错误已在上文返回。
    if let Some(error) = pending_identity_error {
        return Err(error);
    }
    Ok(())
}

impl crate::kernel::phase::StepWorkspace<'_> {
    pub(crate) fn prepare_waiting_step(
        &mut self,
        delta_s: f32,
        execution: Option<&crate::kernel::execution::ExecutionResources>,
    ) -> Result<(), crate::StepError> {
        #[cfg(test)]
        let _preamble = preview_stage::begin(preview_stage::PREAMBLE);
        if !self.waiting_member_rows_valid() {
            return Err(crate::StepError::WaitingInvariantViolation);
        }
        self.workspace.waiting_plans.clear();
        self.workspace.waiting_claims.clear();
        self.workspace.waiting_staged_decisions.clear();
        self.workspace.waiting_non_entry_anchors.clear();
        self.workspace.staged_transition_events.clear();
        self.workspace.waiting_plan_by_vehicle.fill(None);
        reserve_waiting_exact(
            &mut self.workspace.waiting_plans,
            self.derived.active_order.len(),
        )?;

        // 同拍缓存按活动车辆顺序保留；非入口 Gate 决策仍使用正式 staged motion。
        // 扩容失败只缩短可复用的前缀，不新增错误，也不改变领域检查的首错。
        self.workspace.motion_cache.clear();
        let _ = self
            .workspace
            .motion_cache
            .try_reserve(self.derived.active_order.len());
        let cache_limit = self.workspace.motion_cache.capacity();
        #[cfg(test)]
        let cache_limit = cache_limit.min(crate::kernel::tick::motion_cache_limit());
        self.workspace.next_states.clear();
        reserve_waiting_exact(
            &mut self.workspace.next_states,
            self.derived.active_order.len(),
        )?;
        // P2 第一遍：独立计算只读共享输入、写独占槽位；motion_cache/next_states
        // 规范写入保持协调器单写者（#705 提取边界）。融合路径在 live 序上流式
        // 逐车交错（身份检查 → Active 过滤 → 预览 → staging），不物化输入表；
        // 分发路径在内部完成输入发现（checked 预留）与路径选择。
        // 与 read_view 相同的字段级只读投影：持有 committed/derived 借用期间
        // 仍可独占 workspace 字段完成规范消费。
        let view = crate::kernel::phase::StepReadView {
            binding: self.binding,
            committed: &self.committed,
            derived: &self.derived,
        };
        #[cfg(test)]
        drop(_preamble);
        match execution {
            Some(resources @ crate::kernel::execution::ExecutionResources::Pool(_))
                if !preview_dispatch_fuse_forced() =>
            {
                prepare_waiting_previews_dispatched(
                    self.workspace,
                    view,
                    resources,
                    delta_s,
                    cache_limit,
                )?;
            }
            _ => {
                #[cfg(test)]
                count_preview_path(|counts| counts.fused += 1);
                prepare_waiting_previews_fused(self.workspace, view, delta_s, cache_limit)?
            }
        }
        #[cfg(test)]
        let _assembly = preview_stage::begin(preview_stage::ASSEMBLY);

        for preview_index in 0..self.workspace.next_states.len() {
            let (update_sequence, preview) = self.workspace.next_states[preview_index];
            let vehicle = self.committed.live_order[update_sequence];
            let state = *self
                .vehicle_state(vehicle)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let compiled = self
                .compiled_route(state.route)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            // Waiting 区间按路线顺序且不重叠；既有 membership 的 entry 已在 cursor 后方。
            let first_pending = compiled
                .waiting
                .partition_point(|occurrence| occurrence.entry_hop < state.route_edge_index);
            let Some((occurrence_index, occurrence)) = compiled
                .waiting
                .iter()
                .copied()
                .enumerate()
                .skip(first_pending)
                .find(|(_, occurrence)| {
                    let held = state.waiting_membership.is_some_and(|membership| {
                        membership.waiting_zone == occurrence.zone
                            && membership.release_hop == occurrence.release_hop
                    }) && state.maneuver_traversal.is_some_and(|traversal| {
                        traversal.maneuver_occurrence_index == occurrence.maneuver_index
                    });
                    !held && state.route_edge_index <= occurrence.entry_hop
                })
            else {
                continue;
            };
            let entry_index = usize::try_from(occurrence.entry_hop)
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let approach_distance_mm = match distance_to_occurrence_start(
                &compiled.occurrence_segments,
                &compiled.occurrence_offsets,
                &compiled.segment_totals,
                state.route_edge_index as usize,
                state.progress_mm,
                entry_index,
            ) {
                Some(BoundedDistance::Finite(value)) => value,
                Some(BoundedDistance::BeyondFinite) | None => continue,
            };
            let entry_gate = compiled
                .hop_gate
                .get(occurrence.entry_hop as usize)
                .copied()
                .flatten()
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let preview_crossed = preview.route_edge_index > occurrence.entry_hop;
            let preview_at_boundary = front_at_hop_boundary(
                compiled,
                &preview,
                occurrence.entry_hop,
                self.binding.revision.traffic().lane_lengths_millimetres(),
            );
            let decision = if preview_crossed {
                WaitingDecisionOutcome::Granted
            } else if preview_at_boundary && self.gate_is_restrictive(entry_gate, state.profile) {
                WaitingDecisionOutcome::NotEvaluated
            } else {
                continue;
            };
            self.workspace.waiting_plans.push(WaitingVehiclePlan {
                vehicle,
                vehicle_update_sequence: u32::try_from(update_sequence)
                    .map_err(|_| crate::StepError::WaitingInvariantViolation)?,
                occurrence_index: u32::try_from(occurrence_index)
                    .map_err(|_| crate::StepError::WaitingInvariantViolation)?,
                zone: occurrence.zone,
                maneuver_index: occurrence.maneuver_index,
                entry_hop: occurrence.entry_hop,
                release_hop: occurrence.release_hop,
                approach_distance_mm,
                preview_route_edge_index: preview.route_edge_index,
                decision,
                stop_hop: None,
                stop_zone: None,
                stop_maneuver_index: None,
                projection: None,
                admission_sequence: None,
            });
        }

        self.workspace.waiting_plans.sort_unstable_by_key(|plan| {
            (
                plan.zone.raw(),
                plan.approach_distance_mm,
                plan.vehicle_update_sequence,
                plan.entry_hop,
            )
        });
        let mut staged_zone = None;
        for index in 0..self.workspace.waiting_plans.len() {
            if self.workspace.waiting_plans[index].decision == WaitingDecisionOutcome::NotEvaluated
            {
                continue;
            }
            let plan = self.workspace.waiting_plans[index];
            if staged_zone != Some(plan.zone) {
                self.stage_waiting_zone(plan.zone)?;
                staged_zone = Some(plan.zone);
            }
            let zone_index = plan.zone.index();
            let max_occupancy = self
                .binding
                .revision
                .traffic()
                .relations()
                .waiting_zone(plan.zone)
                .ok_or(crate::StepError::WaitingInvariantViolation)?
                .max_occupancy();
            if self.workspace.waiting_staged_occupancy[zone_index] >= max_occupancy {
                self.workspace.waiting_plans[index].decision =
                    WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::Capacity);
                continue;
            }
            let state = self
                .vehicle_state(plan.vehicle)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let profile = self
                .binding
                .revision
                .traffic()
                .relations()
                .vehicle_profile(state.profile)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let gap = if self.workspace.waiting_staged_occupancy[zone_index] == 0 {
                0
            } else {
                u64::from(profile.min_gap_mm())
            };
            let required = self.workspace.waiting_staged_storage_mm[zone_index]
                .checked_add(gap)
                .and_then(|value| value.checked_add(u64::from(state.length_mm)))
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let storage_length_mm = self
                .compiled_route(state.route)
                .and_then(|compiled| compiled.waiting.get(plan.occurrence_index as usize))
                .map(|occurrence| occurrence.storage_length_mm)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            if required > u64::from(storage_length_mm) {
                self.workspace.waiting_plans[index].decision =
                    WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::PhysicalStorage);
                continue;
            }
            self.workspace.waiting_staged_occupancy[zone_index] =
                self.workspace.waiting_staged_occupancy[zone_index]
                    .checked_add(1)
                    .ok_or(crate::StepError::WaitingInvariantViolation)?;
            self.workspace.waiting_staged_storage_mm[zone_index] = required;
        }

        let grant_count = self
            .workspace
            .waiting_plans
            .iter()
            .filter(|plan| plan.decision == WaitingDecisionOutcome::Granted)
            .count();
        reserve_waiting_exact(&mut self.workspace.waiting_claims, grant_count)?;
        reserve_waiting_exact(
            &mut self.workspace.waiting_staged_decisions,
            self.workspace.waiting_plans.len(),
        )?;
        for index in 0..self.workspace.waiting_plans.len() {
            let mut plan = self.workspace.waiting_plans[index];
            match plan.decision {
                WaitingDecisionOutcome::Granted => {
                    let state = self
                        .vehicle_state(plan.vehicle)
                        .ok_or(crate::StepError::WaitingInvariantViolation)?;
                    let compiled = self
                        .compiled_route(state.route)
                        .ok_or(crate::StepError::WaitingInvariantViolation)?;
                    if let Some(next) = next_crossed_waiting(
                        compiled,
                        plan.occurrence_index,
                        plan.preview_route_edge_index,
                    ) {
                        plan.stop_hop = Some(next.entry_hop);
                        plan.stop_zone = Some(next.zone);
                        plan.stop_maneuver_index = Some(next.maneuver_index);
                        plan.projection = Some(WaitingProjectionReason::EvaluationHorizon);
                    }
                }
                WaitingDecisionOutcome::NoGrant(reason) => {
                    plan.stop_hop = Some(plan.entry_hop);
                    plan.stop_zone = Some(plan.zone);
                    plan.stop_maneuver_index = Some(plan.maneuver_index);
                    plan.projection = Some(match reason {
                        WaitingNoGrantReason::Capacity => WaitingProjectionReason::Capacity,
                        WaitingNoGrantReason::PhysicalStorage => {
                            WaitingProjectionReason::PhysicalStorage
                        }
                        WaitingNoGrantReason::CombinedResource(_) => {
                            unreachable!("local reducer only rejects local resources")
                        }
                    });
                }
                WaitingDecisionOutcome::NotEvaluated | WaitingDecisionOutcome::NotRequired => {}
                WaitingDecisionOutcome::Deferred => {
                    unreachable!("local reducer has no combined outcome")
                }
            }
            self.workspace.waiting_plans[index] = plan;
            self.workspace.waiting_plan_by_vehicle[plan.vehicle.index() as usize] =
                std::num::NonZeroU32::new(
                    u32::try_from(index)
                        .map_err(|_| crate::StepError::WaitingInvariantViolation)?
                        + 1,
                );
        }
        Ok(())
    }

    /// Conflict 单写者取得包含该 Waiting entitlement 的完整 bundle 后，才把本地
    /// zone reducer 的 provisional grant 激活为可随 crossing 提交的 claim。
    pub(crate) fn activate_waiting_claim(
        &mut self,
        vehicle: crate::VehicleHandle,
        entry_hop: u32,
    ) -> Result<(), crate::StepError> {
        let plan = self
            .workspace
            .waiting_plan_by_vehicle
            .get(vehicle.index() as usize)
            .copied()
            .flatten()
            .and_then(|index| {
                self.workspace
                    .waiting_plans
                    .get(index.get() as usize - 1)
                    .copied()
            })
            .filter(|plan| {
                plan.vehicle == vehicle
                    && plan.entry_hop == entry_hop
                    && plan.decision == WaitingDecisionOutcome::Granted
            });
        let Some(plan) = plan else {
            return Ok(());
        };
        let plan_index = self.workspace.waiting_plan_by_vehicle[vehicle.index() as usize]
            .ok_or(crate::StepError::WaitingInvariantViolation)?
            .get()
            - 1;
        reserve_waiting_exact(&mut self.workspace.waiting_claims, 1)?;
        self.workspace.waiting_claims.push(WaitingAdmissionClaim {
            vehicle: plan.vehicle,
            vehicle_update_sequence: plan.vehicle_update_sequence,
            occurrence_index: plan.occurrence_index,
            zone: plan.zone,
            entry_hop: plan.entry_hop,
            release_hop: plan.release_hop,
            approach_distance_mm: plan.approach_distance_mm,
            plan_index,
            post_step_group: 0,
            post_step_rank: 0,
        });
        Ok(())
    }

    /// 只初始化本 tick 实际请求的 zone；失败重试也总是从 committed state 开始。
    pub(crate) fn stage_waiting_zone(
        &mut self,
        zone: WaitingZoneOrdinal,
    ) -> Result<(), crate::StepError> {
        if self.waiting_zone_member_count(zone).is_none() {
            return Err(crate::StepError::WaitingInvariantViolation);
        }
        #[cfg(test)]
        count_waiting_work(|counts| counts.staged_zones += 1);
        let state = self.committed.waiting_zones[zone.index()];
        let queue = self.derived.waiting_queue_ends[zone.index()];
        self.workspace.waiting_next_counters[zone.index()] = state.next_admission_sequence;
        self.workspace.waiting_staged_occupancy[zone.index()] = state.occupancy;
        let mut used = 0_u64;
        let mut current = queue.head;
        let mut has_front = false;
        while let Some(vehicle) = current {
            let member = self
                .vehicle_state(vehicle)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let profile = self
                .binding
                .revision
                .traffic()
                .relations()
                .vehicle_profile(member.profile)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            if has_front {
                used = used
                    .checked_add(u64::from(profile.min_gap_mm()))
                    .ok_or(crate::StepError::WaitingInvariantViolation)?;
            }
            used = used
                .checked_add(u64::from(member.length_mm))
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            has_front = true;
            current = self.derived.waiting_links[vehicle.index() as usize].next;
        }
        self.workspace.waiting_staged_storage_mm[zone.index()] = used;
        Ok(())
    }

    pub(crate) fn waiting_stop_for(
        &self,
        state: &crate::VehicleState,
    ) -> Result<Option<WaitingStopConstraint>, crate::StepError> {
        let Some(plan) = self
            .workspace
            .waiting_plan_by_vehicle
            .get(state.handle.index() as usize)
            .copied()
            .flatten()
            .and_then(|index| {
                self.workspace
                    .waiting_plans
                    .get(index.get() as usize - 1)
                    .copied()
            })
            .filter(|plan| plan.vehicle == state.handle)
        else {
            return Ok(None);
        };
        let Some(stop_hop) = plan.stop_hop else {
            return Ok(None);
        };
        let compiled = self
            .compiled_route(state.route)
            .ok_or(crate::StepError::WaitingInvariantViolation)?;
        let stop_index = usize::try_from(stop_hop)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(crate::StepError::WaitingInvariantViolation)?;
        let distance = distance_to_occurrence_start(
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            state.route_edge_index as usize,
            state.progress_mm,
            stop_index,
        )
        .ok_or(crate::StepError::WaitingInvariantViolation)?;
        Ok(Some(WaitingStopConstraint {
            distance,
            hop: stop_hop,
        }))
    }

    pub(crate) fn finalize_waiting_step(
        &mut self,
        updates: &mut [(usize, crate::VehicleState)],
    ) -> Result<(), crate::StepError> {
        self.workspace.next_state_by_vehicle.fill(0);
        for (update_index, (slot, _)) in updates.iter().enumerate() {
            let encoded = u32::try_from(update_index)
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            self.workspace.next_state_by_vehicle[*slot] = encoded;
        }

        // 先 stage tick-start membership 的 successful release。
        for (slot, next) in updates.iter_mut() {
            let old = self.committed.vehicles[*slot]
                .state
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            if let Some(membership) = old.waiting_membership
                && next.route_edge_index > membership.release_hop
            {
                next.waiting_membership = None;
            }
        }

        // successful entry 才消耗 admission sequence；同拍 entry+release 仍消耗 counter。
        // 先从 staged motion 计算物理 rank，再按 zone 内 post-step front-to-back 分配。
        for claim_index in 0..self.workspace.waiting_claims.len() {
            let mut claim = self.workspace.waiting_claims[claim_index];
            let encoded = self.workspace.next_state_by_vehicle[claim.vehicle.index() as usize];
            let update_index = encoded
                .checked_sub(1)
                .map(|value| value as usize)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let next = updates[update_index].1;
            if next.route_edge_index <= claim.entry_hop {
                claim.post_step_group = u8::MAX;
                self.workspace.waiting_claims[claim_index] = claim;
                continue;
            }
            let (group, rank) = post_step_physical_rank(
                self.compiled_route(next.route)
                    .ok_or(crate::StepError::WaitingInvariantViolation)?,
                &next,
                claim.release_hop,
            )
            .ok_or(crate::StepError::WaitingInvariantViolation)?;
            claim.post_step_group = group;
            claim.post_step_rank = rank;
            self.workspace.waiting_claims[claim_index] = claim;
        }
        self.workspace
            .waiting_claims
            .retain(|claim| claim.post_step_group != u8::MAX);
        self.workspace.waiting_claims.sort_unstable_by_key(|claim| {
            (
                claim.zone.raw(),
                claim.post_step_group,
                claim.post_step_rank,
                claim.vehicle_update_sequence,
                claim.entry_hop,
            )
        });
        for claim in self.workspace.waiting_claims.iter().copied() {
            let plan_index = claim.plan_index as usize;
            let mut plan = self.workspace.waiting_plans[plan_index];
            let encoded = self.workspace.next_state_by_vehicle[claim.vehicle.index() as usize];
            let update_index = encoded
                .checked_sub(1)
                .map(|value| value as usize)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let next = &mut updates[update_index].1;
            let zone_index = claim.zone.index();
            let sequence = self.workspace.waiting_next_counters[zone_index];
            self.workspace.waiting_next_counters[zone_index] = sequence
                .checked_add(1)
                .ok_or(crate::StepError::WaitingAdmissionSequenceExhausted)?;
            plan.admission_sequence = Some(sequence);
            if next.route_edge_index <= claim.release_hop {
                next.waiting_membership = Some(WaitingMembership {
                    waiting_zone: claim.zone,
                    admission_sequence: sequence,
                    release_hop: claim.release_hop,
                });
            } else {
                next.waiting_membership = None;
            }
            self.workspace.waiting_plans[plan_index] = plan;
        }

        Ok(())
    }

    pub(crate) fn finalize_waiting_outputs(
        &mut self,
        updates: &[(usize, crate::VehicleState)],
        tick: u64,
    ) -> Result<(), crate::StepError> {
        for plan in self.workspace.waiting_plans.iter().copied() {
            self.workspace
                .waiting_staged_decisions
                .push(WaitingDecision {
                    vehicle: plan.vehicle,
                    vehicle_update_sequence: plan.vehicle_update_sequence,
                    zone: Some(plan.zone),
                    anchor: WaitingRouteAnchor {
                        route: self
                            .vehicle_state(plan.vehicle)
                            .ok_or(crate::StepError::WaitingInvariantViolation)?
                            .route,
                        maneuver_occurrence_index: plan.maneuver_index,
                        hop: plan.entry_hop,
                    },
                    outcome: self.final_waiting_decision(plan),
                });
        }
        self.workspace.waiting_non_entry_anchors.clear();
        for (update_index, (slot, next)) in updates.iter().enumerate() {
            let old = self.committed.vehicles[*slot]
                .state
                .expect("staged live vehicle");
            let compiled = self.committed.routes[old.route.index() as usize]
                .compiled
                .as_ref()
                .expect("live route");
            for (maneuver_occurrence_index, hop) in non_entry_gate_anchors(
                compiled,
                old,
                *next,
                self.binding.revision.traffic().lane_lengths_millimetres(),
            ) {
                let anchors = &mut self.workspace.waiting_non_entry_anchors;
                if anchors.len() == anchors.capacity() {
                    #[cfg(test)]
                    if waiting_reservation_injected_failure() {
                        return Err(crate::StepError::WaitingScratchAllocFailed);
                    }
                    // 按已发现的行摊还增长，不按全路线或车辆容量预留。
                    anchors
                        .try_reserve(1)
                        .map_err(|_| crate::StepError::WaitingScratchAllocFailed)?;
                }
                anchors.push(NonEntryGateAnchor {
                    update_index,
                    maneuver_occurrence_index,
                    hop,
                });
            }
        }
        reserve_waiting_exact(
            &mut self.workspace.waiting_staged_decisions,
            self.workspace.waiting_non_entry_anchors.len(),
        )?;
        let mut live_cursor = 0;
        let mut previous_update = None;
        let mut vehicle_update_sequence = 0;
        for index in 0..self.workspace.waiting_non_entry_anchors.len() {
            #[cfg(test)]
            NON_ENTRY_GENERATION_VISITS.set(NON_ENTRY_GENERATION_VISITS.get() + 1);
            let anchor = self.workspace.waiting_non_entry_anchors[index];
            let old = self.committed.vehicles[updates[anchor.update_index].0]
                .state
                .expect("staged live vehicle");
            if previous_update != Some(anchor.update_index) {
                let sequence = self
                    .workspace
                    .motion_cache
                    .get(anchor.update_index)
                    .filter(|entry| entry.vehicle == old.handle)
                    .map(|entry| entry.update_sequence);
                let sequence = if let Some(sequence) = sequence {
                    live_cursor = sequence;
                    sequence
                } else {
                    // 可选运动缓存可能仅保留前缀；缺失时仍从正式 live 顺序线性合并。
                    loop {
                        #[cfg(test)]
                        NON_ENTRY_SEQUENCE_VISITS.set(NON_ENTRY_SEQUENCE_VISITS.get() + 1);
                        match self.committed.live_order.get(live_cursor) {
                            Some(vehicle) if *vehicle == old.handle => break live_cursor,
                            Some(_) => live_cursor += 1,
                            None => return Err(crate::StepError::WaitingInvariantViolation),
                        }
                    }
                };
                vehicle_update_sequence = u32::try_from(sequence)
                    .map_err(|_| crate::StepError::WaitingInvariantViolation)?;
                previous_update = Some(anchor.update_index);
            }
            let compiled = self.committed.routes[old.route.index() as usize]
                .compiled
                .as_ref()
                .expect("live route");
            let gate = compiled.hop_gate[anchor.hop as usize].expect("indexed Gate");
            // finalize 使用本 tick 起始灯色；发布后的信号刷新不改写历史决定。
            let outcome = if self.gate_is_restrictive(gate, old.profile) {
                WaitingDecisionOutcome::NotEvaluated
            } else {
                WaitingDecisionOutcome::NotRequired
            };
            self.workspace
                .waiting_staged_decisions
                .push(WaitingDecision {
                    vehicle: old.handle,
                    vehicle_update_sequence,
                    zone: None,
                    anchor: WaitingRouteAnchor {
                        route: old.route,
                        maneuver_occurrence_index: anchor.maneuver_occurrence_index,
                        hop: anchor.hop,
                    },
                    outcome,
                });
        }
        self.workspace.waiting_non_entry_anchors.clear();
        self.workspace
            .waiting_staged_decisions
            .sort_unstable_by_key(|decision| {
                (decision.vehicle_update_sequence, decision.anchor.hop)
            });
        self.stage_transition_events(updates, tick)?;
        Ok(())
    }

    pub(crate) fn final_waiting_decision(
        &self,
        plan: WaitingVehiclePlan,
    ) -> WaitingDecisionOutcome {
        if plan.decision != WaitingDecisionOutcome::Granted {
            return plan.decision;
        }
        let Some(motion) = self.workspace.conflict_motion_by_vehicle[plan.vehicle.index() as usize]
            .filter(|motion| motion.gate_hop == plan.entry_hop)
        else {
            return WaitingDecisionOutcome::Deferred;
        };
        match motion.outcome {
            crate::ConflictDecisionOutcome::Granted => WaitingDecisionOutcome::Granted,
            crate::ConflictDecisionOutcome::NoGrant(reason) => {
                WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::CombinedResource(reason))
            }
            crate::ConflictDecisionOutcome::NotEvaluated => WaitingDecisionOutcome::NotEvaluated,
            crate::ConflictDecisionOutcome::NotRequired => WaitingDecisionOutcome::NotRequired,
        }
    }

    pub(crate) fn rollback_waiting_step(&mut self) {
        self.workspace.waiting_dependencies.abort();
        for claim in &self.workspace.waiting_claims {
            let zone_index = claim.zone.index();
            self.workspace.waiting_next_counters[zone_index] =
                self.committed.waiting_zones[zone_index].next_admission_sequence;
        }
        self.workspace.waiting_claims.clear();
        self.workspace.waiting_plans.clear();
        self.workspace.waiting_non_entry_anchors.clear();
        self.workspace.waiting_staged_decisions.clear();
        self.workspace.staged_transition_events.clear();
        self.workspace.waiting_plan_by_vehicle.fill(None);
    }

    pub(crate) fn visit_waiting_events(
        &self,
        old: crate::VehicleState,
        next: crate::VehicleState,
        tick: u64,
        update_sequence: u32,
        mut push_event: impl FnMut(TrafficTransitionEvent),
    ) {
        let plan = self
            .workspace
            .waiting_plan_by_vehicle
            .get(old.handle.index() as usize)
            .copied()
            .flatten()
            .and_then(|index| {
                self.workspace
                    .waiting_plans
                    .get(index.get() as usize - 1)
                    .copied()
            })
            .filter(|plan| plan.vehicle == old.handle);
        if let Some(plan) = plan
            && let (Some(reason), Some(stop_hop), Some(zone), Some(maneuver_index)) = (
                plan.projection,
                plan.stop_hop,
                plan.stop_zone,
                plan.stop_maneuver_index,
            )
            && front_strictly_upstream(
                self.compiled_route(old.route).expect("live route"),
                &old,
                stop_hop,
                self.binding.revision.traffic().lane_lengths_millimetres(),
            )
            && front_at_hop_boundary(
                self.compiled_route(next.route).expect("live route"),
                &next,
                stop_hop,
                self.binding.revision.traffic().lane_lengths_millimetres(),
            )
        {
            push_event(TrafficTransitionEvent {
                tick,
                vehicle: old.handle,
                vehicle_update_sequence: update_sequence,
                anchor: crate::TrafficTransitionAnchor::at_gate(WaitingRouteAnchor {
                    route: old.route,
                    maneuver_occurrence_index: maneuver_index,
                    hop: stop_hop,
                }),
                kind: TrafficTransitionKind::ProjectionApplied { zone, reason },
            });
        }
        if let Some(membership) = old.waiting_membership
            && next.route_edge_index > membership.release_hop
        {
            let maneuver_index = old
                .maneuver_traversal
                .map_or(0, |traversal| traversal.maneuver_occurrence_index);
            push_event(TrafficTransitionEvent {
                tick,
                vehicle: old.handle,
                vehicle_update_sequence: update_sequence,
                anchor: crate::TrafficTransitionAnchor::at_gate(WaitingRouteAnchor {
                    route: old.route,
                    maneuver_occurrence_index: maneuver_index,
                    hop: membership.release_hop,
                }),
                kind: TrafficTransitionKind::WaitingLeft {
                    zone: membership.waiting_zone,
                    admission_sequence: membership.admission_sequence,
                },
            });
        }
        if let Some(plan) = plan
            && let Some(sequence) = plan.admission_sequence
        {
            push_event(TrafficTransitionEvent {
                tick,
                vehicle: old.handle,
                vehicle_update_sequence: update_sequence,
                anchor: crate::TrafficTransitionAnchor::at_gate(WaitingRouteAnchor {
                    route: old.route,
                    maneuver_occurrence_index: plan.maneuver_index,
                    hop: plan.entry_hop,
                }),
                kind: TrafficTransitionKind::WaitingEntered {
                    zone: plan.zone,
                    admission_sequence: sequence,
                },
            });
            if next.route_edge_index > plan.release_hop {
                push_event(TrafficTransitionEvent {
                    tick,
                    vehicle: old.handle,
                    vehicle_update_sequence: update_sequence,
                    anchor: crate::TrafficTransitionAnchor::at_gate(WaitingRouteAnchor {
                        route: old.route,
                        maneuver_occurrence_index: plan.maneuver_index,
                        hop: plan.release_hop,
                    }),
                    kind: TrafficTransitionKind::WaitingLeft {
                        zone: plan.zone,
                        admission_sequence: sequence,
                    },
                });
            }
        }
    }

    pub(crate) fn derive_waiting_traversal(
        &self,
        state: crate::VehicleState,
    ) -> Result<Option<ManeuverTraversalState>, crate::StepError> {
        self.read_view().derive_waiting_traversal(state)
    }

    /// 稳态从已有 member batch 定位非空 zone，而非遍历静态表。队列和语义仍交叉
    /// 验证；未涉及的空 zone 保留历史 counter，由 restore/cutover 做全量验证。
    pub(crate) fn waiting_member_rows_valid(&self) -> bool {
        self.read_view().waiting_member_rows_valid()
    }

    /// 验证一个实际涉及的 zone；全量冷路径和稀疏热路径共享同一队列合同。
    pub(crate) fn waiting_zone_member_count(&self, zone: WaitingZoneOrdinal) -> Option<usize> {
        self.read_view().waiting_zone_member_count(zone)
    }
}

impl crate::kernel::phase::CommittedStateMut<'_> {
    pub(crate) fn commit_waiting_removals(&mut self, updates: &[(usize, crate::VehicleState)]) {
        for (slot, next) in updates {
            let old = self.committed.vehicles[*slot]
                .state
                .expect("staged next state has a live predecessor");
            if let Some(membership) = old.waiting_membership
                && next.waiting_membership != Some(membership)
            {
                self.unlink_waiting_member(old.handle, membership);
            }
        }
    }

    pub(crate) fn commit_waiting_additions(&mut self, updates: &[(usize, crate::VehicleState)]) {
        for plan_index in 0..self.workspace.waiting_plans.len() {
            let plan = self.workspace.waiting_plans[plan_index];
            let Some(sequence) = plan.admission_sequence else {
                continue;
            };
            if plan.vehicle.index() as usize >= self.workspace.next_state_by_vehicle.len() {
                continue;
            }
            let Some(update_index) = self.workspace.next_state_by_vehicle
                [plan.vehicle.index() as usize]
                .checked_sub(1)
                .map(|value| value as usize)
            else {
                continue;
            };
            let next = updates[update_index].1;
            let membership = WaitingMembership {
                waiting_zone: plan.zone,
                admission_sequence: sequence,
                release_hop: plan.release_hop,
            };
            if next.waiting_membership == Some(membership) {
                self.append_waiting_member(plan.vehicle, membership);
            }
        }
        // finalize 后 claims 只含 successful entry，按 zone 排序；同拍 enter+leave
        // 虽无终态 member，仍必须提交该 zone 的 counter。
        for claims in self
            .workspace
            .waiting_claims
            .chunk_by(|left, right| left.zone == right.zone)
        {
            #[cfg(test)]
            count_waiting_work(|counts| counts.committed_zones += 1);
            let zone_index = claims[0].zone.index();
            self.committed.waiting_zones[zone_index].next_admission_sequence =
                self.workspace.waiting_next_counters[zone_index];
        }
        self.rebuild_waiting_member_rows();
        std::mem::swap(
            &mut self.committed.latest_waiting_decisions,
            &mut self.workspace.waiting_staged_decisions,
        );
        std::mem::swap(
            &mut self.committed.latest_transition_events,
            &mut self.workspace.staged_transition_events,
        );
    }

    pub(crate) fn rebuild_waiting_member_rows(&mut self) {
        self.derived.waiting_member_rows.clear();
        for vehicle in self.committed.live_order.iter().copied() {
            #[cfg(test)]
            count_waiting_work(|counts| counts.member_vehicles += 1);
            if let Some(membership) = self
                .vehicle_state(vehicle)
                .and_then(|state| state.waiting_membership)
            {
                self.derived.waiting_member_rows.push(WaitingZoneMember {
                    zone: membership.waiting_zone,
                    vehicle,
                    admission_sequence: membership.admission_sequence,
                    release_hop: membership.release_hop,
                });
            }
        }
        self.derived
            .waiting_member_rows
            .sort_unstable_by_key(|member| (member.zone.raw(), member.admission_sequence));
    }

    pub(crate) fn unlink_waiting_member(
        &mut self,
        vehicle: VehicleHandle,
        membership: WaitingMembership,
    ) {
        let index = vehicle.index() as usize;
        let link = self.derived.waiting_links[index];
        let zone = &mut self.committed.waiting_zones[membership.waiting_zone.index()];
        let queue = &mut self.derived.waiting_queue_ends[membership.waiting_zone.index()];
        match link.previous {
            Some(previous) => {
                self.derived.waiting_links[previous.index() as usize].next = link.next
            }
            None => queue.head = link.next,
        }
        match link.next {
            Some(next) => {
                self.derived.waiting_links[next.index() as usize].previous = link.previous
            }
            None => queue.tail = link.previous,
        }
        zone.occupancy = zone
            .occupancy
            .checked_sub(1)
            .expect("validated occupancy covers member");
        self.derived.waiting_links[index] = WaitingQueueLink::default();
    }

    pub(crate) fn append_waiting_member(
        &mut self,
        vehicle: VehicleHandle,
        membership: WaitingMembership,
    ) {
        let index = vehicle.index() as usize;
        let zone = &mut self.committed.waiting_zones[membership.waiting_zone.index()];
        let queue = &mut self.derived.waiting_queue_ends[membership.waiting_zone.index()];
        let previous = queue.tail;
        self.derived.waiting_links[index] = WaitingQueueLink {
            previous,
            next: None,
        };
        if let Some(previous) = previous {
            self.derived.waiting_links[previous.index() as usize].next = Some(vehicle);
        } else {
            queue.head = Some(vehicle);
        }
        queue.tail = Some(vehicle);
        zone.occupancy = zone
            .occupancy
            .checked_add(1)
            .expect("admission preflight guarantees occupancy room");
    }
}

const fn waiting_membership_cursor_valid(
    route_edge_index: u32,
    occurrence: &crate::kernel::tables::WaitingOccurrence,
) -> bool {
    route_edge_index > occurrence.entry_hop && route_edge_index <= occurrence.release_hop
}

fn waiting_front_distance_mm(front: (u8, u64), follower: (u8, u64)) -> Option<u64> {
    match (front.0, follower.0) {
        (0, 0) | (1, 1) => follower.1.checked_sub(front.1),
        (0, 1) => u64::MAX.checked_sub(front.1)?.checked_add(follower.1),
        _ => None,
    }
}

fn post_step_physical_rank(
    compiled: &CompiledRoute,
    state: &crate::VehicleState,
    release_hop: u32,
) -> Option<(u8, u64)> {
    let release_index = usize::try_from(release_hop).ok()?.checked_add(1)?;
    let cursor = usize::try_from(state.route_edge_index).ok()?;
    if cursor >= release_index {
        let distance = distance_to_occurrence_start(
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            release_index,
            0,
            cursor,
        )?;
        let BoundedDistance::Finite(to_edge_start) = distance else {
            return None;
        };
        let beyond = u64::from(to_edge_start).checked_add(u64::from(state.progress_mm))?;
        Some((0, u64::MAX - beyond))
    } else {
        let distance = distance_to_occurrence_start(
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            cursor,
            state.progress_mm,
            release_index,
        )?;
        let BoundedDistance::Finite(remaining) = distance else {
            return None;
        };
        Some((1, u64::from(remaining)))
    }
}

fn non_entry_gate_anchors<'a>(
    compiled: &'a CompiledRoute,
    old: crate::VehicleState,
    next: crate::VehicleState,
    lengths: &'a [u32],
) -> impl Iterator<Item = (u32, u32)> + 'a {
    #[cfg(test)]
    NON_ENTRY_DISCOVERY_VISITS.set(NON_ENTRY_DISCOVERY_VISITS.get() + 1);
    let start = compiled
        .gate_hops
        .partition_point(|hop| *hop < old.route_edge_index);
    compiled.gate_hops[start..]
        .iter()
        .copied()
        .take_while(move |hop| *hop <= next.route_edge_index)
        .filter_map(move |hop| non_entry_gate_anchor(compiled, &next, hop as usize, lengths))
}

fn non_entry_gate_anchor(
    compiled: &CompiledRoute,
    preview: &crate::VehicleState,
    hop: usize,
    lengths: &[u32],
) -> Option<(u32, u32)> {
    let hop_u32 = u32::try_from(hop).ok()?;
    compiled.hop_gate.get(hop).copied().flatten()?;
    if compiled
        .waiting
        .binary_search_by_key(&hop_u32, |occurrence| occurrence.entry_hop)
        .is_ok()
    {
        return None;
    }
    if preview.route_edge_index <= hop_u32
        && !front_at_hop_boundary(compiled, preview, hop_u32, lengths)
    {
        return None;
    }
    maneuver_index_at_hop(compiled, hop_u32)
        .and_then(|index| u32::try_from(index).ok())
        .map(|maneuver_index| (maneuver_index, hop_u32))
}

fn next_crossed_waiting(
    compiled: &CompiledRoute,
    occurrence_index: u32,
    preview_route_edge_index: u32,
) -> Option<&crate::kernel::tables::WaitingOccurrence> {
    // Waiting 按路线 entry hop 排序；未越过紧邻入口就不可能越过后缀入口。
    compiled
        .waiting
        .get(occurrence_index as usize + 1)
        .filter(|occurrence| {
            #[cfg(test)]
            count_waiting_lookup(3);
            preview_route_edge_index > occurrence.entry_hop
        })
}

fn maneuver_index_at_hop(compiled: &CompiledRoute, hop: u32) -> Option<usize> {
    let index = compiled.maneuvers.partition_point(|maneuver| {
        #[cfg(test)]
        count_waiting_lookup(1);
        maneuver.exit_route_edge_index <= hop
    });
    compiled
        .maneuvers
        .get(index)
        .and_then(|maneuver| (maneuver.entry_route_edge_index <= hop).then_some(index))
}

fn reserve_waiting_exact<T>(
    values: &mut Vec<T>,
    additional: usize,
) -> Result<(), crate::StepError> {
    let missing = additional.saturating_sub(values.capacity().saturating_sub(values.len()));
    if missing == 0 {
        return Ok(());
    }
    #[cfg(test)]
    if waiting_reservation_injected_failure() {
        return Err(crate::StepError::WaitingScratchAllocFailed);
    }
    values
        .try_reserve_exact(additional)
        .map_err(|_| crate::StepError::WaitingScratchAllocFailed)
}

fn front_at_hop_boundary(
    compiled: &CompiledRoute,
    state: &crate::VehicleState,
    hop: u32,
    lengths: &[u32],
) -> bool {
    if state.route_edge_index != hop {
        return false;
    }
    compiled
        .edges
        .get(hop as usize)
        .and_then(|edge| lengths.get(edge.index()))
        .is_some_and(|length| state.progress_mm == *length)
}

fn front_strictly_upstream(
    compiled: &CompiledRoute,
    state: &crate::VehicleState,
    hop: u32,
    lengths: &[u32],
) -> bool {
    if state.route_edge_index < hop {
        return true;
    }
    if state.route_edge_index > hop {
        return false;
    }
    compiled
        .edges
        .get(hop as usize)
        .and_then(|edge| lengths.get(edge.index()))
        .is_some_and(|length| state.progress_mm < *length)
}

fn first_gate_hop(
    compiled: &CompiledRoute,
    maneuver: &crate::kernel::tables::ManeuverOccurrence,
) -> Option<u32> {
    let start = compiled
        .gate_hops
        .partition_point(|hop| *hop < maneuver.entry_route_edge_index);
    compiled
        .gate_hops
        .get(start)
        .copied()
        .filter(|hop| *hop < maneuver.exit_route_edge_index)
}

#[cfg(test)]
pub(crate) mod tests {
    #[test]
    fn non_entry_outputs_discover_once_and_generate_only_present_anchors() {
        let mut world = multi_gate_world(2);
        let mut saw_non_entry = false;
        let mut saw_waiting_without_non_entry = false;
        let mut saw_events_without_non_entry = false;
        let mut previous_non_entry = false;
        let mut saw_non_entry_to_empty = false;
        for _ in 0..64 {
            NON_ENTRY_GENERATION_VISITS.set(0);
            NON_ENTRY_DISCOVERY_VISITS.set(0);
            NON_ENTRY_SEQUENCE_VISITS.set(0);
            let active = world.state.derived.active_order.len();
            world.step(TickInput::new(100)).unwrap();
            let decisions = world.latest_waiting_decisions();
            let has_non_entry = decisions.iter().any(|decision| decision.zone().is_none());
            assert_eq!(
                NON_ENTRY_GENERATION_VISITS.get(),
                decisions
                    .iter()
                    .filter(|decision| decision.zone().is_none())
                    .count()
            );
            assert_eq!(NON_ENTRY_DISCOVERY_VISITS.get(), active);
            assert_eq!(NON_ENTRY_SEQUENCE_VISITS.get(), 0);
            assert!(world.state.workspace.waiting_non_entry_anchors.is_empty());
            assert!(decisions.windows(2).all(|pair| {
                (pair[0].vehicle_update_sequence(), pair[0].anchor().hop())
                    <= (pair[1].vehicle_update_sequence(), pair[1].anchor().hop())
            }));
            saw_non_entry |= has_non_entry;
            saw_waiting_without_non_entry |= !has_non_entry && !decisions.is_empty();
            saw_events_without_non_entry |=
                !has_non_entry && !world.latest_transition_events().is_empty();
            saw_non_entry_to_empty |= previous_non_entry && decisions.is_empty();
            previous_non_entry = has_non_entry;
        }
        assert!(saw_non_entry);
        assert!(saw_waiting_without_non_entry);
        assert!(saw_events_without_non_entry);
        assert!(saw_non_entry_to_empty);
    }

    #[test]
    fn zero_non_entry_count_keeps_transition_validation() {
        use crate::kernel::conflict_tick::ConflictPassageTransition;
        use laneflow_static_contract::{ConflictZoneOrdinal, ParticipantStreamOrdinal};

        let mut world = multi_gate_world(1);
        world
            .state
            .workspace
            .conflict_passage_transitions
            .push(ConflictPassageTransition {
                vehicle: world.live_vehicles()[0],
                occurrence_index: 0,
                address: crate::ConflictPassageAddress::new(
                    ConflictZoneOrdinal::from_raw(0),
                    ParticipantStreamOrdinal::from_raw(0),
                    0,
                ),
                enter: true,
                clear: false,
            });
        // 没有车辆更新，却残留通行段转移：必须到达后续事件访问器并报告不变量错误。
        assert_eq!(
            world.state.finalize_waiting_outputs(&[], 1),
            Err(crate::StepError::ConflictInvariantViolation)
        );
        assert!(world.latest_waiting_decisions().is_empty());
        assert!(world.latest_transition_events().is_empty());
    }

    #[test]
    fn non_entry_scratch_failure_preserves_published_batch_and_same_tick_retry() {
        let mut reference = multi_gate_world(8);
        let mut first_output_tick = 0;
        for tick in 1..=64 {
            reference.step(TickInput::new(100)).unwrap();
            if reference
                .latest_waiting_decisions()
                .iter()
                .any(|d| d.zone().is_none())
            {
                first_output_tick = tick;
                break;
            }
        }
        assert!(first_output_tick > 1);
        assert!(
            reference
                .state
                .workspace
                .waiting_non_entry_anchors
                .capacity()
                >= 8
        );
        let retained = reference.state.retained_memory().world_owned_bytes();
        let anchors = std::mem::take(&mut reference.state.workspace.waiting_non_entry_anchors);
        assert_eq!(
            retained - reference.state.retained_memory().world_owned_bytes(),
            crate::kernel::state::vec_bytes(&anchors)
        );
        reference.state.workspace.waiting_non_entry_anchors = anchors;
        let mut failures = 0;
        for fail_after in 0..16 {
            let mut world = multi_gate_world(8);
            for _ in 1..first_output_tick {
                world.step(TickInput::new(100)).unwrap();
            }
            let before = world.capture_snapshot().unwrap();
            let decisions = world.latest_waiting_decisions().to_vec();
            let events = world.latest_transition_events().to_vec();
            let guard = fail_waiting_reservation_after(fail_after);
            let result = world.step(TickInput::new(100));
            drop(guard);
            if result.is_ok() {
                break;
            }
            assert_eq!(result, Err(crate::StepError::WaitingScratchAllocFailed));
            failures += 1;
            assert_eq!(world.capture_snapshot().unwrap(), before);
            assert_eq!(world.latest_waiting_decisions(), decisions);
            assert_eq!(world.latest_transition_events(), events);
            assert!(world.state.workspace.waiting_non_entry_anchors.is_empty());
            world.step(TickInput::new(100)).unwrap();
            assert_eq!(
                world.capture_snapshot().unwrap(),
                reference.capture_snapshot().unwrap()
            );
            assert_eq!(
                world.latest_waiting_decisions(),
                reference.latest_waiting_decisions()
            );
            assert_eq!(
                world.latest_transition_events(),
                reference.latest_transition_events()
            );
        }
        assert!(failures >= 2, "anchor growth and decision reservation");
        eprintln!(
            "non-entry scratch payload={} capacity={} retained={} motion_entry={}",
            size_of::<NonEntryGateAnchor>(),
            reference
                .state
                .workspace
                .waiting_non_entry_anchors
                .capacity(),
            crate::kernel::state::vec_bytes(&reference.state.workspace.waiting_non_entry_anchors),
            size_of::<crate::kernel::tick::MotionCacheEntry>()
        );
    }

    #[test]
    fn waiting_plan_invariant_precedes_non_entry_scratch_failure() {
        let mut world = multi_gate_world(1);
        world.state.prepare_waiting_step(0.1).unwrap();
        world.state.workspace.waiting_plans[0].vehicle = VehicleHandle::new(u32::MAX, 0);
        let _guard = fail_waiting_reservation_after(0);
        assert_eq!(
            world.state.finalize_waiting_outputs(&[], 1),
            Err(crate::StepError::WaitingInvariantViolation)
        );
    }

    #[test]
    fn warmed_non_entry_outputs_need_no_waiting_scratch_growth() {
        let mut world = multi_gate_world(8);
        let inputs = world
            .live_vehicles()
            .iter()
            .map(|vehicle| {
                let state = world.state.vehicle_state(*vehicle).unwrap();
                VehicleSpawnInput::new(
                    state.profile,
                    state.route,
                    state.route_edge_index,
                    state.progress_mm,
                    state.speed_mm_s,
                )
                .with_open_entrance()
            })
            .collect::<Vec<_>>();
        for _ in 0..64 {
            world.step(TickInput::new(100)).unwrap();
        }
        let capacity = world.state.workspace.waiting_non_entry_anchors.capacity();
        assert!(capacity >= 8);
        for vehicle in world.live_vehicles().to_vec() {
            world.despawn_vehicle(vehicle).unwrap();
        }
        for input in inputs {
            world.spawn_vehicle(input).unwrap();
        }
        let _guard = fail_waiting_reservation_after(0);
        let mut outputs = 0;
        for _ in 0..64 {
            world.step(TickInput::new(100)).unwrap();
            outputs += world
                .latest_waiting_decisions()
                .iter()
                .filter(|d| d.zone().is_none())
                .count();
        }
        assert!(outputs >= 8);
        assert_eq!(
            world.state.workspace.waiting_non_entry_anchors.capacity(),
            capacity
        );
    }

    #[test]
    fn combined_scheduler_preserves_same_zone_physical_order() {
        let revision = waiting_scale_revision_with_layout(8.0, 1, ScaleLayout::MergingApproaches);
        let origin = *revision.canonical_origin();
        let mut world = TrafficWorld::install(
            Arc::clone(&revision),
            WorldConfig::new(2, 2, 64, 64, 100),
            crate::ExecutionConfig::new(std::num::NonZeroU32::MIN),
            CommittedNetworkSource::Published {
                reference: PublishedLfcaReference::new(
                    "fixture://round2-order",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .unwrap(),
            },
            82,
            crate::test_policy::selection(&revision),
        )
        .unwrap();
        let limits = CompileLimits::p100_initial_v1();
        let mut vehicles = Vec::new();
        for (prefix, progress) in [("idle-0-right", 19_300), ("idle-0-left", 19_500)] {
            let edges = [prefix, "idle-0-entry", "idle-0-storage", "idle-0-exit"].map(|key| {
                let stable = derive_canonical_stable_id_v1(
                    EntityKind::LaneEdge,
                    "city/waiting-scale",
                    key,
                    &limits,
                )
                .unwrap();
                revision
                    .identity()
                    .ordinal(LaneEdgeId::from_untyped(stable))
                    .unwrap()
            });
            let route = world
                .register_route(RouteRegisterInput::new(edges.to_vec()))
                .unwrap();
            vehicles.push(
                world
                    .spawn_vehicle(
                        VehicleSpawnInput::new(
                            VehicleProfileOrdinal::from_raw(0),
                            route,
                            0,
                            progress,
                            10_000,
                        )
                        .with_open_entrance(),
                    )
                    .unwrap(),
            );
        }
        world.state.rebuild_occupancy_index().unwrap();
        world.state.prepare_waiting_step(0.1).unwrap();
        assert_eq!(world.state.workspace.waiting_plans.len(), 2);
        assert!(
            world
                .state
                .workspace
                .waiting_plans
                .iter()
                .all(|plan| plan.decision == WaitingDecisionOutcome::Granted)
        );
        assert_eq!(world.state.workspace.waiting_plans[0].vehicle, vehicles[1]);
        world.state.prepare_conflict_step(0.1, 1, None).unwrap();
        assert_eq!(world.state.workspace.conflict_candidates.len(), 2);
        assert_eq!(
            world.state.workspace.conflict_grants[0].vehicle, vehicles[1],
            "combined reducer reordered the physical front behind its follower"
        );
    }

    #[test]
    fn held_waiting_entry_at_post_gate_zero_is_not_a_new_resource_barrier() {
        let mut world = multi_gate_world(1);
        world.step(TickInput::new(100)).unwrap();
        let vehicle = world.state.committed.live_order[0];
        let state = world.state.committed.vehicles[vehicle.index() as usize]
            .state
            .as_mut()
            .unwrap();
        assert!(state.waiting_membership.is_some());
        state.progress_mm = 0;
        state.carry_um = 0;
        // 已持 membership 的合法边界等价游标；下一拍不得重复申请已越过的 entry。
        world.step(TickInput::new(100)).unwrap();
        assert!(world.vehicle(vehicle).unwrap().progress_mm() > 0);
        assert!(
            world
                .vehicle(vehicle)
                .unwrap()
                .waiting_membership()
                .is_some()
        );
    }

    use std::mem::size_of;
    use std::sync::Arc;
    use std::time::Instant;

    use laneflow_compiler::{
        CompilationUnitBuilder, CompileLimits, Compiler, IidmVehicleProfileInput, JunctionInput,
        JunctionReference, LaneEdgeInput, LaneEdgeReference, ManeuverGateInput,
        ManeuverGateReference, ManeuverPathInput, ManeuverPathReference, MovementInput,
        MovementReference, ParticipantClassInput, ParticipantClassReference, PortableDiffBase,
        PortableEmissionProvenance, SignalControlInput, SourceModuleHeader,
        SourceModuleHeaderInput, StopLineInput, StopLineReference, SyntheticModuleBuilder,
        VehicleProfileInput, WaitingZoneInput, derive_canonical_stable_id_v1,
        emit_portable_candidate,
    };
    use laneflow_format::{
        FormatLimits, check_canonical_network_input, check_post_emission_bundle,
    };
    use laneflow_static_contract::{
        EntityKind, LaneEdgeId, ManeuverPathOrdinal, VehicleProfileOrdinal,
    };
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    use super::*;
    use crate::admin::migration_journal::{
        JournalRecord, VEHICLE_DELTA_BYTES, VehicleDelta, waiting_zone_delta_stream,
    };
    use crate::{
        CommittedNetworkSource, CutoverPreflightLimits, LfcaOriginBinding, MigrationPolicyKind,
        NetworkRevisionCutoverDescriptor, PublishedLfcaReference, RouteRegisterInput,
        SnapshotRestoreError, SnapshotRestoreLimits, TickInput, TrafficWorld, VehicleSpawnInput,
        WorldConfig, deterministic_state_digest, encode_lfrs, restore_lfrs,
    };

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
    );

    fn waiting_world() -> (
        TrafficWorld,
        RouteHandle,
        crate::kernel::tables::WaitingOccurrence,
    ) {
        waiting_world_at_delta(100)
    }

    fn waiting_world_at_delta(
        delta_time_ms: u64,
    ) -> (
        TrafficWorld,
        RouteHandle,
        crate::kernel::tables::WaitingOccurrence,
    ) {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
            .expect("checked fixture");
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::Omit,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("revision");
        let origin = *revision.canonical_origin();
        let mut world = TrafficWorld::install(
            Arc::clone(&revision),
            WorldConfig::new(16, 8, 1_024, 1_024, delta_time_ms),
            crate::ExecutionConfig::new(std::num::NonZeroU32::MIN),
            CommittedNetworkSource::Published {
                reference: PublishedLfcaReference::new(
                    "fixture://waiting-runtime",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .expect("source"),
            },
            82,
            crate::test_policy::selection(&revision),
        )
        .expect("install");
        let edges = world
            .traffic()
            .maneuvers()
            .maneuver_path(ManeuverPathOrdinal::from_raw(0))
            .expect("main path")
            .edges()
            .to_vec();
        let route = world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route");
        let occurrence = world
            .state
            .compiled_route(route)
            .and_then(|compiled| compiled.waiting.first().copied())
            .expect("Waiting occurrence");
        (world, route, occurrence)
    }

    #[test]
    #[ignore = "manual release comparison of simultaneous independent Gate requests"]
    fn multi_gate_comparison_evidence() {
        for count in [64, 256, 1_024] {
            let mut world = multi_gate_world(count);
            let mut samples = Vec::with_capacity(21);
            for sample in 0..24 {
                if sample != 0 {
                    for handle in world.state.committed.live_order.clone() {
                        let route = world.vehicle(handle).unwrap().route();
                        let edge = world.route_edges(route).unwrap()[0];
                        let boundary = world.traffic().lane_lengths_millimetres()[edge.index()];
                        world.despawn_vehicle(handle).unwrap();
                        world
                            .spawn_vehicle(
                                VehicleSpawnInput::new(
                                    VehicleProfileOrdinal::from_raw(0),
                                    route,
                                    0,
                                    boundary - 1,
                                    10_000,
                                )
                                .with_open_entrance(),
                            )
                            .unwrap();
                    }
                }
                let started = Instant::now();
                world.step(TickInput::new(100)).unwrap();
                let elapsed = started.elapsed().as_nanos();
                if sample >= 3 {
                    samples.push(elapsed);
                }
                assert_eq!(world.state.derived.waiting_member_rows.len(), count);
                assert_eq!(world.latest_waiting_decisions().len(), count);
                assert!(
                    world
                        .latest_waiting_decisions()
                        .iter()
                        .all(|decision| decision.outcome() == WaitingDecisionOutcome::Granted)
                );
            }
            samples.sort_unstable();
            eprintln!(
                "multi-gate-comparison candidates={count} tick_p50_ns={} tick_p95_ns={} waiting_retained_bytes={} conflict_retained_bytes={}",
                samples[10],
                samples[19],
                waiting_retained_bytes(&world),
                world.state.conflict_retained_logical_bytes()
            );
        }
    }

    fn waiting_scale_revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
        waiting_scale_revision_with(8.0, 1)
    }

    pub(crate) fn multi_gate_world(count: usize) -> TrafficWorld {
        multi_gate_world_with_id(count, 82)
    }

    /// 与 [`multi_gate_world`] 相同构造，但使用指定世界身份；故障注入按世界
    /// 身份隔离的测试因此可以并行运行而互不观测到对方的武装状态。
    pub(crate) fn multi_gate_world_with_id(count: usize, world_id: u64) -> TrafficWorld {
        multi_gate_world_partial(count, count, world_id).0
    }

    /// 注册全部 `count` 条路径的路线，但只在其中 `spawned` 条上放车辆；
    /// 供增长/收缩工作集测试在两次 step 之间继续在同一路线上生成车辆。
    /// 机制测量探针复用同一构造并自行补员，保持稳态活动车队。
    pub(crate) fn multi_gate_world_partial(
        count: usize,
        spawned: usize,
        world_id: u64,
    ) -> (TrafficWorld, Vec<RouteHandle>) {
        assert!(spawned <= count, "spawned vehicles fit path count");
        let revision = waiting_scale_revision_with_layout(8.0, 1, ScaleLayout::IdleZones(count));
        install_multi_gate(&revision, count, spawned, world_id)
    }

    /// 从既有修订安装 multi-gate 世界并注册全部路线、生成 `spawned` 辆车；
    /// 同一制品可构建两个等价根供同修订切换测试使用。
    fn install_multi_gate(
        revision: &Arc<laneflow_static_network::SharedNetworkRevision>,
        count: usize,
        spawned: usize,
        world_id: u64,
    ) -> (TrafficWorld, Vec<RouteHandle>) {
        assert!(spawned <= count, "spawned vehicles fit path count");
        let origin = *revision.canonical_origin();
        let count_u32 = u32::try_from(count).unwrap();
        let mut world = TrafficWorld::install(
            Arc::clone(revision),
            WorldConfig::new(count_u32, count_u32, u64::from(count_u32) * 3, 1, 100),
            crate::ExecutionConfig::new(std::num::NonZeroU32::MIN),
            CommittedNetworkSource::Published {
                reference: PublishedLfcaReference::new(
                    "fixture://multi-gate",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .unwrap(),
            },
            world_id,
            crate::test_policy::selection(revision),
        )
        .unwrap();
        let mut routes = Vec::new();
        for raw in 0..revision.traffic().maneuvers().maneuver_path_count() {
            let edges = revision
                .traffic()
                .maneuvers()
                .maneuver_path(ManeuverPathOrdinal::from_raw(raw))
                .unwrap()
                .edges()
                .to_vec();
            if edges.len() != 3 {
                continue;
            }
            routes.push(
                world
                    .register_route(RouteRegisterInput::new(edges))
                    .unwrap(),
            );
        }
        assert_eq!(routes.len(), count);
        for route in routes.iter().take(spawned) {
            spawn_idle_zone_vehicle(&mut world, *route);
        }
        assert_eq!(world.state.committed.live_order.len(), spawned);
        (world, routes)
    }

    /// 在一条尚未使用的 idle-zone 路线上按夹具标准位置（首边末端前一毫米）
    /// 生成车辆；与 `multi_gate_world` 的初始车队同形。
    fn spawn_idle_zone_vehicle(world: &mut TrafficWorld, route: RouteHandle) -> VehicleHandle {
        let edges = world.route_edges(route).unwrap();
        let boundary = world.traffic().lane_lengths_millimetres()[edges[0].index()];
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    boundary - 1,
                    10_000,
                )
                .with_open_entrance(),
            )
            .unwrap()
    }

    #[test]
    fn simultaneous_gate_requests_use_linear_transition_links() {
        use crate::kernel::conflict::{conflict_work_counts, reset_conflict_work_counts};
        for count in [64, 256, 1_024] {
            let mut world = multi_gate_world(count);
            reset_conflict_work_counts();
            let started = Instant::now();
            world.step(TickInput::new(100)).unwrap();
            let elapsed = started.elapsed();
            let work = conflict_work_counts();
            assert_eq!(work.candidates, count);
            assert_eq!(work.vehicle_grant_lookups, count);
            assert_eq!(work.grant_update_lookups, 2 * count);
            assert_eq!(work.wait_for_nodes, 2 * count);
            assert_eq!(work.wait_for_edges, count);
            assert!(work.wait_for_visits <= 2 * count);
            assert_eq!(work.owner_record_moves, count);
            assert_eq!(world.state.derived.waiting_member_rows.len(), count);
            assert!(
                world
                    .latest_waiting_decisions()
                    .iter()
                    .all(|decision| decision.outcome() == WaitingDecisionOutcome::Granted)
            );
            assert!(world.state.conflict_state_valid());
            assert!(world.state.waiting_state_valid());
            eprintln!(
                "multi-gate candidates={count} tick_ns={} vehicle_grant_lookups={} grant_update_lookups={} waiting_retained={}",
                elapsed.as_nanos(),
                work.vehicle_grant_lookups,
                work.grant_update_lookups,
                waiting_retained_bytes(&world)
            );
        }
    }

    fn waiting_scale_revision_with(
        storage_length_meters: f64,
        max_occupancy: u32,
    ) -> Arc<laneflow_static_network::SharedNetworkRevision> {
        waiting_scale_revision_with_layout(
            storage_length_meters,
            max_occupancy,
            ScaleLayout::SingleZone,
        )
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ScaleLayout {
        NoWaiting,
        NoWaitingEarlyEntry,
        NoWaitingEarlyExit,
        SingleZone,
        AdditionalGate,
        SecondZone,
        IdleZones(usize),
        MergingApproaches,
    }

    fn waiting_scale_revision_with_layout(
        storage_length_meters: f64,
        max_occupancy: u32,
        layout: ScaleLayout,
    ) -> Arc<laneflow_static_network::SharedNetworkRevision> {
        let candidate = waiting_scale_candidate_with_layout(
            storage_length_meters,
            max_occupancy,
            layout,
            PortableDiffBase::Genesis,
        );
        waiting_scale_shared(&candidate)
    }

    fn waiting_scale_candidate_with_layout(
        storage_length_meters: f64,
        max_occupancy: u32,
        layout: ScaleLayout,
        base: PortableDiffBase<'_>,
    ) -> laneflow_compiler::PortablePublicationCandidate {
        const NS: &str = "city/waiting-scale";
        const STEM_COUNT: usize = 64;
        // 多 WaitingZone 夹具的显式策略也参与后发射预算，使用已有规模档。
        let limits = CompileLimits::single_network_1m_v2();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: NS,
                source_document_key: "waiting-scale.document",
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x31; 32],
                frontend_options_digest: [0x42; 32],
                random_seed: Some(282),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .expect("source header");
        let mut module = SyntheticModuleBuilder::new(header, &limits).expect("module");
        module
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "road-user",
                extends: None,
            })
            .expect("class")
            .add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: "car",
                participant_class: ParticipantClassReference::local("road-user"),
                iidm: IidmVehicleProfileInput {
                    length_meters: 4.5,
                    desired_speed_meters_per_second: 13.75,
                    min_gap_meters: 2.0,
                    time_headway_seconds: 1.4,
                    max_acceleration_meters_per_second_squared: 1.8,
                    comfortable_deceleration_meters_per_second_squared: 2.0,
                    emergency_deceleration_meters_per_second_squared: 4.5,
                },
            })
            .expect("profile");
        let stems = (0..STEM_COUNT)
            .map(|index| format!("stem-{index:02}"))
            .collect::<Vec<_>>();
        for (index, key) in stems.iter().enumerate() {
            let successor = stems.get(index + 1).map_or("entry", String::as_str);
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: 10_000.0,
                    speed_limit_meters_per_second: 13.75,
                    successors: &[LaneEdgeReference::local(successor)],
                })
                .expect("stem");
        }
        let internal_edges = [
            LaneEdgeReference::local("entry"),
            LaneEdgeReference::local("storage"),
            LaneEdgeReference::local("after-release"),
        ];
        let internal_range = match layout {
            ScaleLayout::NoWaitingEarlyEntry => 0..3,
            ScaleLayout::NoWaitingEarlyExit => 1..2,
            _ => 1..3,
        };
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "entry",
                length_meters: 10_000.0,
                speed_limit_meters_per_second: 13.75,
                successors: &[LaneEdgeReference::local("storage")],
            })
            .expect("entry")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "storage",
                length_meters: storage_length_meters,
                speed_limit_meters_per_second: 13.75,
                successors: &[LaneEdgeReference::local("after-release")],
            })
            .expect("storage")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "after-release",
                length_meters: 12.0,
                speed_limit_meters_per_second: 13.75,
                successors: &[LaneEdgeReference::local("exit")],
            })
            .expect("after release")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "exit",
                length_meters: 12.0,
                speed_limit_meters_per_second: 13.75,
                successors: &[LaneEdgeReference::local("entry")],
            })
            .expect("exit")
            .add_junction(JunctionInput {
                junction_key: "junction",
            })
            .expect("junction")
            .add_movement(MovementInput {
                turn_direction: None,
                movement_key: "movement",
                junction: JunctionReference::local("junction"),
                directed_entry_approach_key: "approach-in",
                directed_exit_approach_key: "approach-out",
            })
            .expect("movement")
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: "path",
                movement: MovementReference::local("movement"),
                entry_edge: LaneEdgeReference::local(
                    if layout == ScaleLayout::NoWaitingEarlyEntry {
                        "stem-63"
                    } else {
                        "entry"
                    },
                ),
                internal_edges: &internal_edges[internal_range],
                exit_edge: LaneEdgeReference::local(if layout == ScaleLayout::NoWaitingEarlyExit {
                    "after-release"
                } else {
                    "exit"
                }),
            })
            .expect("path")
            .add_stop_line(StopLineInput {
                stop_line_key: "stop-entry",
                lane_edge: LaneEdgeReference::local("entry"),
            })
            .expect("entry stop")
            .add_stop_line(StopLineInput {
                stop_line_key: "stop-release",
                lane_edge: LaneEdgeReference::local("storage"),
            })
            .expect("release stop")
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: "gate-entry",
                maneuver_path: ManeuverPathReference::local("path"),
                transition_index: u32::from(layout == ScaleLayout::NoWaitingEarlyEntry),
                stop_line: StopLineReference::local("stop-entry"),
                signal_control: SignalControlInput::None,
            })
            .expect("entry gate")
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: "gate-release",
                maneuver_path: ManeuverPathReference::local("path"),
                transition_index: 1 + u32::from(layout == ScaleLayout::NoWaitingEarlyEntry),
                stop_line: StopLineReference::local("stop-release"),
                signal_control: SignalControlInput::None,
            })
            .expect("release gate");
        if !matches!(
            layout,
            ScaleLayout::NoWaiting
                | ScaleLayout::NoWaitingEarlyEntry
                | ScaleLayout::NoWaitingEarlyExit
        ) {
            module
                .add_waiting_zone(WaitingZoneInput {
                    waiting_zone_key: "waiting",
                    maneuver_path: ManeuverPathReference::local("path"),
                    entry_gate: ManeuverGateReference::local("gate-entry"),
                    release_gate: ManeuverGateReference::local("gate-release"),
                    max_occupancy,
                })
                .expect("WaitingZone");
        }
        // 两个修订均保留相同 Gate；target 仅新增共享边界后的 Waiting 区间。
        if matches!(
            layout,
            ScaleLayout::AdditionalGate | ScaleLayout::SecondZone
        ) {
            module
                .add_stop_line(StopLineInput {
                    stop_line_key: "stop-exit",
                    lane_edge: LaneEdgeReference::local("after-release"),
                })
                .expect("exit stop")
                .add_maneuver_gate(ManeuverGateInput {
                    maneuver_gate_key: "gate-exit",
                    maneuver_path: ManeuverPathReference::local("path"),
                    transition_index: 2,
                    stop_line: StopLineReference::local("stop-exit"),
                    signal_control: SignalControlInput::None,
                })
                .expect("exit gate");
        }
        if layout == ScaleLayout::SecondZone {
            module
                .add_waiting_zone(WaitingZoneInput {
                    waiting_zone_key: "waiting-added",
                    maneuver_path: ManeuverPathReference::local("path"),
                    entry_gate: ManeuverGateReference::local("gate-release"),
                    release_gate: ManeuverGateReference::local("gate-exit"),
                    max_occupancy,
                })
                .expect("target-only WaitingZone");
        }
        if let ScaleLayout::IdleZones(count) = layout {
            for index in 0..count {
                add_idle_waiting_path(&mut module, index, false);
            }
        }
        if layout == ScaleLayout::MergingApproaches {
            add_idle_waiting_path(&mut module, 0, true);
        }
        module
            .add_parking_facility(laneflow_compiler::ParkingFacilityInput {
                parking_facility_key: "parking-after-exit",
                virtual_capacity: 2,
                virtual_entries: &[laneflow_compiler::ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("exit"),
                    progress_meters: 8.0,
                }],
                virtual_exits: &[laneflow_compiler::ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("exit"),
                    progress_meters: 10.0,
                }],
            })
            .expect("parking after maneuver");
        let mut policy_gates = vec!["gate-entry".to_owned(), "gate-release".to_owned()];
        if matches!(
            layout,
            ScaleLayout::AdditionalGate | ScaleLayout::SecondZone
        ) {
            policy_gates.push("gate-exit".to_owned());
        }
        if let ScaleLayout::IdleZones(count) = layout {
            for index in 0..count {
                policy_gates.push(format!("idle-{index}-entry-gate"));
                policy_gates.push(format!("idle-{index}-release-gate"));
            }
        }
        if layout == ScaleLayout::MergingApproaches {
            policy_gates.push("idle-0-entry-gate".to_owned());
            policy_gates.push("idle-0-release-gate".to_owned());
        }
        let policy_rules: Vec<_> = policy_gates
            .iter()
            .map(|key| {
                (
                    key.as_str(),
                    laneflow_compiler::GateInterpretation::Uncontrolled,
                )
            })
            .collect();
        crate::test_policy::add_gate_policy(&mut module, "waiting-policy", &policy_rules);
        let mut unit = CompilationUnitBuilder::new(limits);
        unit.add_synthetic_module(module.finish().expect("finished module"))
            .expect("unit module");
        let output = Compiler::new()
            .compile(unit.build().expect("unit"))
            .expect("compiled");
        let provenance =
            PortableEmissionProvenance::try_new("laneflow-waiting-scale-v1").expect("provenance");
        emit_portable_candidate(&output, &provenance, FormatLimits::HARD, base)
            .expect("portable candidate")
    }

    fn waiting_scale_shared(
        candidate: &laneflow_compiler::PortablePublicationCandidate,
    ) -> Arc<laneflow_static_network::SharedNetworkRevision> {
        let checked = check_post_emission_bundle(
            candidate.canonical_artifact().bytes(),
            candidate.source_map().bytes(),
            candidate.semantic_diff().bytes(),
            candidate.expected_semantic_diff_base(),
            FormatLimits::HARD,
        )
        .expect("checked bundle");
        build_shared_network_revision(
            checked.canonical_network_input(),
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::Omit,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("revision")
    }

    fn add_idle_waiting_path(module: &mut SyntheticModuleBuilder, index: usize, merging: bool) {
        let [
            entry,
            storage,
            exit,
            junction,
            movement,
            path,
            entry_stop,
            release_stop,
            entry_gate,
            release_gate,
            zone,
        ] = [
            "entry",
            "storage",
            "exit",
            "junction",
            "movement",
            "path",
            "entry-stop",
            "release-stop",
            "entry-gate",
            "release-gate",
            "zone",
        ]
        .map(|suffix| format!("idle-{index}-{suffix}"));
        if merging {
            for prefix in [format!("idle-{index}-left"), format!("idle-{index}-right")] {
                module
                    .add_lane_edge(LaneEdgeInput {
                        lane_edge_key: &prefix,
                        length_meters: 20.0,
                        speed_limit_meters_per_second: 13.75,
                        successors: &[LaneEdgeReference::local(&entry)],
                    })
                    .unwrap();
            }
        }
        for (edge, successor) in [(&entry, &storage), (&storage, &exit), (&exit, &entry)] {
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: edge,
                    length_meters: if merging && edge == &entry { 0.1 } else { 20.0 },
                    speed_limit_meters_per_second: 13.75,
                    successors: &[LaneEdgeReference::local(successor)],
                })
                .expect("idle lane");
        }
        module
            .add_junction(JunctionInput {
                junction_key: &junction,
            })
            .expect("idle junction")
            .add_movement(MovementInput {
                turn_direction: None,
                movement_key: &movement,
                junction: JunctionReference::local(&junction),
                directed_entry_approach_key: "in",
                directed_exit_approach_key: "out",
            })
            .expect("idle movement")
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: &path,
                movement: MovementReference::local(&movement),
                entry_edge: LaneEdgeReference::local(&entry),
                internal_edges: &[LaneEdgeReference::local(&storage)],
                exit_edge: LaneEdgeReference::local(&exit),
            })
            .expect("idle path");
        for (gate, stop, edge, transition_index) in [
            (&entry_gate, &entry_stop, &entry, 0),
            (&release_gate, &release_stop, &storage, 1),
        ] {
            module
                .add_stop_line(StopLineInput {
                    stop_line_key: stop,
                    lane_edge: LaneEdgeReference::local(edge),
                })
                .expect("idle stop")
                .add_maneuver_gate(ManeuverGateInput {
                    maneuver_gate_key: gate,
                    maneuver_path: ManeuverPathReference::local(&path),
                    transition_index,
                    stop_line: StopLineReference::local(stop),
                    signal_control: SignalControlInput::None,
                })
                .expect("idle gate");
        }
        module
            .add_waiting_zone(WaitingZoneInput {
                waiting_zone_key: &zone,
                maneuver_path: ManeuverPathReference::local(&path),
                entry_gate: ManeuverGateReference::local(&entry_gate),
                release_gate: ManeuverGateReference::local(&release_gate),
                max_occupancy: 2,
            })
            .expect("idle zone");
    }

    fn waiting_scale_world(
        revision: Arc<laneflow_static_network::SharedNetworkRevision>,
        vehicle_count: u32,
    ) -> (TrafficWorld, WaitingZoneOrdinal) {
        waiting_scale_world_at_delta(revision, vehicle_count, 4)
    }

    /// 测试夹具：构造首对 Waiting cutover 修订与语义 diff。
    pub(crate) fn first_waiting_cutover_pair() -> (
        Arc<laneflow_static_network::SharedNetworkRevision>,
        Arc<laneflow_static_network::SharedNetworkRevision>,
        Vec<u8>,
    ) {
        first_waiting_cutover_pair_with_layout(ScaleLayout::NoWaiting)
    }

    /// 测试夹具：构造入口/出口平移后身份变化的 cutover 修订对。
    pub(crate) fn first_waiting_changed_identity_cutover_pair(
        shift_entry: bool,
    ) -> (
        Arc<laneflow_static_network::SharedNetworkRevision>,
        Arc<laneflow_static_network::SharedNetworkRevision>,
        Vec<u8>,
    ) {
        first_waiting_cutover_pair_with_layout(if shift_entry {
            ScaleLayout::NoWaitingEarlyEntry
        } else {
            ScaleLayout::NoWaitingEarlyExit
        })
    }

    fn first_waiting_cutover_pair_with_layout(
        layout: ScaleLayout,
    ) -> (
        Arc<laneflow_static_network::SharedNetworkRevision>,
        Arc<laneflow_static_network::SharedNetworkRevision>,
        Vec<u8>,
    ) {
        let base = waiting_scale_candidate_with_layout(8.0, 1, layout, PortableDiffBase::Genesis);
        let values = laneflow_format::preflight_object_values(
            base.canonical_artifact().bytes(),
            laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
            FormatLimits::HARD,
        )
        .expect("base values");
        let target = waiting_scale_candidate_with_layout(
            8.0,
            1,
            ScaleLayout::SingleZone,
            PortableDiffBase::Artifact(values),
        );
        (
            waiting_scale_shared(&base),
            waiting_scale_shared(&target),
            target.semantic_diff().bytes().to_vec(),
        )
    }

    fn waiting_scale_world_at_delta(
        revision: Arc<laneflow_static_network::SharedNetworkRevision>,
        vehicle_count: u32,
        delta_time_ms: u64,
    ) -> (TrafficWorld, WaitingZoneOrdinal) {
        waiting_scale_world_with_route_capacity(revision, vehicle_count, delta_time_ms, 1_024)
    }

    fn waiting_scale_world_with_route_capacity(
        revision: Arc<laneflow_static_network::SharedNetworkRevision>,
        vehicle_count: u32,
        delta_time_ms: u64,
        route_edge_capacity: u64,
    ) -> (TrafficWorld, WaitingZoneOrdinal) {
        const NS: &str = "city/waiting-scale";
        const STEM_COUNT: usize = 64;
        const EDGE_LENGTH_MM: u64 = 10_000_000;
        const SPACING_MM: u64 = 6_500;
        let origin = *revision.canonical_origin();
        let mut world = TrafficWorld::install(
            Arc::clone(&revision),
            WorldConfig::new(vehicle_count, 2, route_edge_capacity, 1_024, delta_time_ms),
            crate::ExecutionConfig::new(std::num::NonZeroU32::MIN),
            CommittedNetworkSource::Published {
                reference: PublishedLfcaReference::new(
                    "fixture://waiting-scale",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .expect("source"),
            },
            u64::from(vehicle_count),
            crate::test_policy::selection(&revision),
        )
        .expect("install");
        let limits = CompileLimits::p100_initial_v1();
        let mut keys = (0..STEM_COUNT)
            .map(|index| format!("stem-{index:02}"))
            .collect::<Vec<_>>();
        keys.extend([
            "entry".into(),
            "storage".into(),
            "after-release".into(),
            "exit".into(),
        ]);
        let edges = keys
            .iter()
            .map(|key| {
                let stable = derive_canonical_stable_id_v1(EntityKind::LaneEdge, NS, key, &limits)
                    .expect("stable edge");
                revision
                    .identity()
                    .ordinal(LaneEdgeId::from_untyped(stable))
                    .expect("edge ordinal")
            })
            .collect::<Vec<_>>();
        let route = world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route");
        let profile = VehicleProfileOrdinal::from_raw(0);
        let profile_view = world
            .traffic()
            .relations()
            .vehicle_profile(profile)
            .expect("profile");
        let profile_class = profile_view.class();
        let profile_length_mm = profile_view.length_mm();
        let entry_boundary_mm = u64::try_from(STEM_COUNT + 1).expect("count") * EDGE_LENGTH_MM;
        for update_sequence in 0..vehicle_count {
            let distance = 1_u64 + u64::from(update_sequence) * SPACING_MM;
            let absolute = entry_boundary_mm
                .checked_sub(distance)
                .expect("corridor room");
            let route_edge_index =
                u32::try_from(absolute / EDGE_LENGTH_MM).expect("route occurrence");
            let progress_mm = u32::try_from(absolute % EDGE_LENGTH_MM).expect("progress");
            let handle = VehicleHandle::new(update_sequence, 0);
            let traversal = world
                .state
                .validate_waiting_bootstrap(route, route_edge_index as usize, profile_length_mm)
                .expect("bootstrap");
            world
                .state
                .committed
                .vehicles
                .push(crate::kernel::tables::VehicleSlot {
                    generation: 0,
                    state: Some(crate::VehicleState {
                        handle,
                        profile,
                        class: profile_class,
                        route,
                        route_edge_index,
                        progress_mm,
                        carry_um: 0,
                        speed_mm_s: if update_sequence == 0 { 10_000 } else { 0 },
                        length_mm: profile_length_mm,
                        status: crate::VehicleStatus::Active,
                        maneuver_traversal: traversal,
                        waiting_membership: None,
                    }),
                });
            world.state.committed.live_order.push(handle);
            world.state.derived.active_order.push(handle);
        }
        world.state.committed.routes[route.index() as usize].live_vehicles = vehicle_count;
        world.state.rebuild_occupancy_index().expect("occupancy");
        let zone = world
            .state
            .compiled_route(route)
            .and_then(|compiled| compiled.waiting.first())
            .map(|occurrence| occurrence.zone)
            .expect("Waiting occurrence");
        (world, zone)
    }

    fn waiting_retained_bytes(world: &TrafficWorld) -> u64 {
        let bytes = world.state.committed.waiting_zones.len() * size_of::<WaitingZoneState>()
            + world.state.derived.waiting_queue_ends.len() * size_of::<WaitingQueueEnds>()
            + world.state.derived.waiting_links.len() * size_of::<WaitingQueueLink>()
            + world.state.derived.waiting_member_rows.capacity() * size_of::<WaitingZoneMember>()
            + world.state.workspace.waiting_claims.capacity() * size_of::<WaitingAdmissionClaim>()
            + world.state.workspace.waiting_plans.capacity() * size_of::<WaitingVehiclePlan>()
            + world.state.workspace.waiting_plan_by_vehicle.len()
                * size_of::<Option<std::num::NonZeroU32>>()
            + world.state.workspace.next_state_by_vehicle.len() * size_of::<u32>()
            + world.state.workspace.waiting_staged_decisions.capacity()
                * size_of::<WaitingDecision>()
            + world.state.workspace.waiting_non_entry_anchors.capacity()
                * size_of::<NonEntryGateAnchor>()
            + world.state.workspace.staged_transition_events.capacity()
                * size_of::<TrafficTransitionEvent>()
            + world.state.workspace.waiting_next_counters.len() * size_of::<u64>()
            + world.state.workspace.waiting_staged_occupancy.len() * size_of::<u32>()
            + world.state.workspace.waiting_staged_storage_mm.len() * size_of::<u64>()
            + world.state.committed.latest_waiting_decisions.capacity()
                * size_of::<WaitingDecision>()
            + world.state.committed.latest_transition_events.capacity()
                * size_of::<TrafficTransitionEvent>();
        u64::try_from(bytes).expect("retained bytes")
    }

    fn waiting_prepare_samples(world: &mut TrafficWorld) -> (u128, u128) {
        let mut samples = Vec::with_capacity(21);
        for sample in 0..24 {
            let started = Instant::now();
            world
                .state
                .prepare_waiting_step(0.004)
                .expect("Waiting prepare");
            let elapsed = started.elapsed().as_nanos();
            if sample >= 3 {
                samples.push(elapsed);
            }
        }
        samples.sort_unstable();
        (samples[10], samples[19])
    }

    fn step_waiting_counts(world: &mut TrafficWorld) -> WaitingWorkCounts {
        WAITING_WORK_COUNTS.with(|counts| counts.set(WaitingWorkCounts::default()));
        world
            .step(TickInput::new(
                world.state.binding.config.fixed_delta_time_ms(),
            ))
            .expect("step");
        WAITING_WORK_COUNTS.with(core::cell::Cell::get)
    }

    #[test]
    fn sparse_waiting_tick_work_ignores_idle_static_zones_and_preserves_history() {
        for idle_zones in [0, 256] {
            let revision =
                waiting_scale_revision_with_layout(8.0, 1, ScaleLayout::IdleZones(idle_zones));
            for armed in [false, true] {
                let (mut empty, _) = waiting_scale_world(Arc::clone(&revision), 0);
                assert_eq!(empty.state.committed.waiting_zones.len(), idle_zones + 1);
                if armed {
                    empty.state.arm_migration_journal(16 * 1_024).unwrap();
                }
                assert_eq!(
                    step_waiting_counts(&mut empty),
                    WaitingWorkCounts::default()
                );

                let (mut world, zone) = waiting_scale_world(Arc::clone(&revision), 1);
                let vehicle = VehicleHandle::new(0, 0);
                let initial = *world.state.vehicle_state(vehicle).unwrap();
                if armed {
                    world.state.arm_migration_journal(16 * 1_024).unwrap();
                }
                assert_eq!(
                    step_waiting_counts(&mut world),
                    WaitingWorkCounts {
                        checked_zones: 1,
                        staged_zones: 1,
                        journal_zones: usize::from(armed),
                        committed_zones: 1,
                        member_vehicles: 1,
                    }
                );
                assert_eq!(world.waiting_zone_members().len(), 1);
                assert_eq!(
                    step_waiting_counts(&mut world),
                    WaitingWorkCounts {
                        checked_zones: 1,
                        member_vehicles: 1,
                        ..WaitingWorkCounts::default()
                    }
                );
                world.despawn_vehicle(vehicle).unwrap();
                for _ in 0..3 {
                    assert_eq!(
                        step_waiting_counts(&mut world),
                        WaitingWorkCounts::default()
                    );
                    assert_eq!(
                        world.state.committed.waiting_zones[zone.index()].next_admission_sequence,
                        1
                    );
                }
                let restored = roundtrip(&world);
                assert_eq!(
                    restored.state.committed.waiting_zones[zone.index()].next_admission_sequence,
                    1
                );
                let next = world
                    .spawn_vehicle(
                        VehicleSpawnInput::new(
                            initial.profile,
                            initial.route,
                            initial.route_edge_index,
                            initial.progress_mm,
                            initial.speed_mm_s,
                        )
                        .with_open_entrance(),
                    )
                    .unwrap();
                assert_eq!(step_waiting_counts(&mut world).committed_zones, 1);
                assert_eq!(
                    world
                        .state
                        .vehicle_state(next)
                        .unwrap()
                        .waiting_membership
                        .unwrap()
                        .admission_sequence,
                    1
                );
                assert_eq!(
                    world.state.committed.waiting_zones[zone.index()].next_admission_sequence,
                    2
                );
            }
        }
    }

    #[test]
    fn successful_same_tick_enter_leave_journals_counter_without_member() {
        let (mut world, zone) = waiting_scale_world_at_delta(waiting_scale_revision(), 1, 1_000);
        world.state.arm_migration_journal(16 * 1_024).unwrap();
        assert_eq!(step_waiting_counts(&mut world).journal_zones, 1);
        assert!(world.waiting_zone_members().is_empty());
        assert_eq!(
            world.state.committed.waiting_zones[zone.index()].occupancy,
            0
        );
        assert_eq!(
            world.state.committed.waiting_zones[zone.index()].next_admission_sequence,
            1
        );
        let record = world
            .state
            .migration_journal()
            .unwrap()
            .records_from(0)
            .next()
            .unwrap();
        let JournalRecord::Tick { waiting_zones, .. } = record else {
            panic!("tick");
        };
        assert_eq!(
            waiting_zone_delta_stream(waiting_zones).collect::<Vec<_>>(),
            [(zone, 1)]
        );
        assert_eq!(
            world
                .latest_transition_events()
                .iter()
                .filter(|event| matches!(
                    event.kind(),
                    TrafficTransitionKind::WaitingEntered { .. }
                        | TrafficTransitionKind::WaitingLeft { .. }
                ))
                .count(),
            2
        );
        assert_eq!(step_waiting_counts(&mut world).committed_zones, 0);
        assert_eq!(
            roundtrip(&world).state.committed.waiting_zones[zone.index()].next_admission_sequence,
            1
        );
    }

    #[test]
    #[ignore = "manual release-mode Waiting 10k/100k scale evidence"]
    fn waiting_10k_100k_scale_evidence() {
        let revision = waiting_scale_revision();
        let (mut product_world, product_zone) = waiting_scale_world(Arc::clone(&revision), 10_000);
        let product_retained_bytes = waiting_retained_bytes(&product_world);
        let (product_p50_ns, product_p95_ns) = waiting_prepare_samples(&mut product_world);
        assert!(
            product_p95_ns <= 4_000_000,
            "10k Waiting p95 hard gate exceeded: {product_p95_ns} ns"
        );
        product_world
            .step(TickInput::new(4))
            .expect("10k correctness step");
        assert_eq!(
            product_world
                .waiting_zone(product_zone)
                .expect("10k zone")
                .occupancy(),
            1
        );
        assert!(product_world.state.waiting_state_valid());
        drop(product_world);

        let (mut scale_world, scale_zone) = waiting_scale_world(revision, 100_000);
        let scale_retained_bytes = waiting_retained_bytes(&scale_world);
        let (scale_p50_ns, scale_p95_ns) = waiting_prepare_samples(&mut scale_world);
        scale_world
            .step(TickInput::new(4))
            .expect("100k correctness step");
        assert_eq!(
            scale_world
                .waiting_zone(scale_zone)
                .expect("100k zone")
                .occupancy(),
            1
        );
        assert!(scale_world.state.waiting_state_valid());
        assert!(
            scale_retained_bytes <= product_retained_bytes.saturating_mul(11),
            "Waiting retained memory grows faster than the 10x population plus 10% margin"
        );
        assert!(
            scale_p95_ns <= product_p95_ns.saturating_mul(20).max(1),
            "100k Waiting p95 grows faster than the 10x population plus 2x margin"
        );

        eprintln!(
            "waiting-g2-scale-evidence 10k_p50_ns={product_p50_ns} \
             10k_p95_ns={product_p95_ns} 10k_retained_bytes={product_retained_bytes} \
             100k_p50_ns={scale_p50_ns} 100k_p95_ns={scale_p95_ns} \
             100k_retained_bytes={scale_retained_bytes}"
        );
    }

    #[test]
    fn successful_entry_persists_counter_membership_journal_and_despawn_release() {
        let (mut world, route, occurrence) = waiting_world();
        let entry_edge = world.route_edges(route).expect("route")[occurrence.entry_hop as usize];
        let entry_length = world.traffic().lane_lengths_millimetres()[entry_edge.index()];
        let vehicle = world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    occurrence.entry_hop,
                    entry_length - 100,
                    8_000,
                )
                .with_open_entrance(),
            )
            .expect("spawn upstream of entry");
        world.state.arm_migration_journal(16 * 1_024).expect("arm");
        world.step(TickInput::new(100)).expect("entry step");

        let membership = world
            .vehicle(vehicle)
            .and_then(|state| state.waiting_membership())
            .expect("entry commits membership");
        assert_eq!(membership.waiting_zone(), occurrence.zone);
        assert_eq!(membership.admission_sequence(), 0);
        assert_eq!(membership.release_hop(), occurrence.release_hop);
        let zone = world.waiting_zone(occurrence.zone).expect("zone");
        assert_eq!(zone.occupancy(), 1);
        assert_eq!(zone.next_admission_sequence(), 1);
        assert_eq!(world.waiting_zone_members().len(), 1);
        assert!(matches!(
            world.latest_waiting_decisions(),
            [WaitingDecision {
                outcome: WaitingDecisionOutcome::Granted,
                ..
            }]
        ));
        assert!(
            world
                .latest_transition_events()
                .iter()
                .any(|event| matches!(
                    event.kind(),
                    TrafficTransitionKind::WaitingEntered {
                        admission_sequence: 0,
                        ..
                    }
                ))
        );

        let records = world
            .state
            .migration_journal()
            .expect("journal")
            .records_from(0)
            .collect::<Vec<_>>();
        let JournalRecord::Tick {
            entries,
            waiting_zones,
            ..
        } = records[0]
        else {
            panic!("entry produces tick record");
        };
        assert_eq!(entries.len(), VEHICLE_DELTA_BYTES);
        let delta = VehicleDelta::decode(entries);
        assert!(delta.traversal_present);
        assert!(delta.membership_present);
        assert_eq!(delta.admission_sequence, 0);
        assert_eq!(
            waiting_zone_delta_stream(waiting_zones).collect::<Vec<_>>(),
            [(occurrence.zone, 1)]
        );
        world.state.disarm_migration_journal();

        let captured = world.capture_snapshot().expect("capture");
        let digest = deterministic_state_digest(&captured).expect("digest");
        let restored = restore_lfrs(
            &encode_lfrs(&captured),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            crate::ExecutionConfig::new(std::num::NonZeroU32::MIN),
            SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
        )
        .expect("restore Waiting state");
        let restored_world = restored.world();
        assert_eq!(
            restored_world
                .waiting_zone(occurrence.zone)
                .expect("restored zone")
                .next_admission_sequence(),
            1
        );
        assert_eq!(restored_world.waiting_zone_members().len(), 1);
        assert_eq!(
            deterministic_state_digest(&restored_world.capture_snapshot().expect("capture"))
                .expect("digest"),
            digest
        );

        let cutover_target = world.revision();
        let origin = *cutover_target.canonical_origin();
        let descriptor = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(origin),
            LfcaOriginBinding::from_canonical_origin(origin),
            None,
            MigrationPolicyKind::SameRevisionRestore,
            world.world_binding(),
        );
        let cutover_source = world.committed_source().clone();
        let _cutover_events = world
            .cutover_same_revision(
                cutover_target,
                cutover_source,
                &descriptor,
                &CutoverPreflightLimits::new(1_048_576),
            )
            .expect("same-revision Waiting cutover");
        assert_eq!(
            world
                .vehicle(vehicle)
                .and_then(|state| state.waiting_membership()),
            Some(membership)
        );
        assert_eq!(
            world
                .waiting_zone(occurrence.zone)
                .expect("cutover zone")
                .next_admission_sequence(),
            1
        );

        let release = world
            .despawn_vehicle(vehicle)
            .expect("despawn member")
            .waiting_release
            .expect("typed release payload");
        assert_eq!(release.waiting_zone(), occurrence.zone);
        assert_eq!(release.admission_sequence(), 0);
        let zone = world.waiting_zone(occurrence.zone).expect("zone");
        assert_eq!(zone.occupancy(), 0);
        assert_eq!(zone.next_admission_sequence(), 1);
        assert!(world.waiting_zone_members().is_empty());

        let empty_with_history = world.capture_snapshot().expect("empty history capture");
        assert_eq!(empty_with_history.waiting_zones.len(), 1);
        assert_eq!(empty_with_history.waiting_zones[0].occupancy, 0);
        assert_eq!(
            empty_with_history.waiting_zones[0].next_admission_sequence,
            1
        );
        let empty_digest = deterministic_state_digest(&empty_with_history).expect("empty digest");
        let mut changed_counter = empty_with_history.clone();
        changed_counter.waiting_zones[0].next_admission_sequence = 2;
        assert_ne!(
            deterministic_state_digest(&changed_counter).expect("changed digest"),
            empty_digest
        );
        let restored_empty = restore_lfrs(
            &encode_lfrs(&empty_with_history),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            crate::ExecutionConfig::new(std::num::NonZeroU32::MIN),
            SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
        )
        .expect("restore empty zone history");
        assert_eq!(
            restored_empty
                .world()
                .waiting_zone(occurrence.zone)
                .expect("restored empty zone")
                .next_admission_sequence(),
            1
        );
    }

    fn roundtrip(world: &TrafficWorld) -> TrafficWorld {
        let snapshot = world.capture_snapshot().expect("capture");
        let restored = restore_lfrs(
            &encode_lfrs(&snapshot),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            crate::ExecutionConfig::new(std::num::NonZeroU32::MIN),
            SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
        )
        .expect("restore")
        .into_world();
        assert_eq!(
            deterministic_state_digest(&snapshot).expect("digest"),
            deterministic_state_digest(&restored.capture_snapshot().expect("restored capture"))
                .expect("digest"),
        );
        restored
    }

    #[test]
    fn parked_retained_cursor_is_not_a_waiting_arrival_and_outputs_use_live_order() {
        let (mut world, route, occurrence) = waiting_world();
        let profile = VehicleProfileOrdinal::from_raw(0);
        let parked = world
            .spawn_parked_vehicle(
                crate::ParkedVehicleSpawnInput::new(profile, route, occurrence.entry_hop, 0),
                crate::ParkingTarget::ExplicitSpace(
                    laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0),
                ),
            )
            .expect("Parked retained cursor is not lane occupancy")
            .vehicle;
        assert!(
            world
                .vehicle(parked)
                .expect("parked")
                .maneuver_traversal()
                .is_none()
        );
        roundtrip(&world);
        crate::admin::cutover_migration::revalidate_migrated_vehicles(&mut world.state)
            .expect("Parked cutover validation");
        let entry = world.route_edges(route).expect("route")[occurrence.entry_hop as usize];
        let member = world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    profile,
                    route,
                    occurrence.entry_hop,
                    world.traffic().lane_lengths_millimetres()[entry.index()] - 100,
                    8_000,
                )
                .with_open_entrance(),
            )
            .expect("active after parked");
        world.step(TickInput::new(100)).expect("admit");
        assert_eq!(world.live_vehicles(), &[parked, member]);
        assert!(!world.latest_waiting_decisions().is_empty());
        assert!(
            world
                .latest_waiting_decisions()
                .iter()
                .all(|row| row.vehicle_update_sequence() == 1)
        );
        assert!(!world.latest_transition_events().is_empty());
        assert!(
            world
                .latest_transition_events()
                .iter()
                .all(|row| row.vehicle_update_sequence() == 1)
        );
        let latest = world.latest_transition_events().to_vec();
        let target = world.revision();
        let origin = *target.canonical_origin();
        let descriptor = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(origin),
            LfcaOriginBinding::from_canonical_origin(origin),
            None,
            MigrationPolicyKind::SameRevisionRestore,
            world.world_binding(),
        );
        let _ = world
            .cutover_same_revision(
                target,
                world.committed_source().clone(),
                &descriptor,
                &CutoverPreflightLimits::new(1_048_576),
            )
            .expect("same revision");
        assert_eq!(world.latest_transition_events(), latest);
    }

    #[test]
    fn rebind_refreshes_member_rows_without_reordering_authority() {
        let (mut world, zone) = waiting_scale_world(waiting_scale_revision(), 1);
        world.step(TickInput::new(4)).expect("entry");
        let vehicle = world.live_vehicles()[0];
        let old = world.vehicle(vehicle).expect("member");
        let edges = world.route_edges(old.route()).expect("route").to_vec();
        let facility = laneflow_static_contract::ParkingFacilityOrdinal::from_raw(0);
        world
            .reserve_parking(
                vehicle,
                crate::ReserveParkingTarget::VirtualPool {
                    facility,
                    entry_anchor: crate::VirtualEntryAnchorSelector::from_raw(0),
                    entry_route_occurrence: (edges.len() - 1) as u32,
                },
            )
            .expect("reserve beyond maneuver");
        let new_route = world
            .register_route(RouteRegisterInput::new(edges[1..].to_vec()))
            .expect("shorter prefix");
        let before = world.waiting_zone(zone).expect("zone");
        world
            .rebind_parking_route(
                vehicle,
                crate::RebindParkingTarget::VirtualPool {
                    facility,
                    new_route,
                    new_current_route_occurrence: old.route_edge_index() - 1,
                    new_entry_anchor: crate::VirtualEntryAnchorSelector::from_raw(0),
                    new_entry_route_occurrence: (edges.len() - 2) as u32,
                },
            )
            .expect("rebind");
        let member = world
            .vehicle(vehicle)
            .expect("member")
            .waiting_membership()
            .expect("membership");
        assert_eq!(
            member.release_hop(),
            old.waiting_membership().expect("old").release_hop() - 1
        );
        assert_eq!(
            world.waiting_zone_members()[0].release_hop(),
            member.release_hop()
        );
        assert_eq!(
            world.waiting_zone_members()[0].admission_sequence(),
            member.admission_sequence()
        );
        assert_eq!(world.waiting_zone(zone).expect("zone"), before);
        roundtrip(&world);
    }

    #[test]
    fn signal_boundary_phase_roundtrips_without_reinterpreting_history() {
        for restrictive_before in [true, false] {
            let (mut world, route, occurrence) = waiting_world();
            let edges = world.route_edges(route).expect("route").to_vec();
            let entry_length = world.traffic().lane_lengths_millimetres()
                [edges[occurrence.entry_hop as usize].index()];
            let member = world
                .spawn_vehicle(
                    VehicleSpawnInput::new(
                        VehicleProfileOrdinal::from_raw(0),
                        route,
                        occurrence.entry_hop,
                        entry_length - 100,
                        8_000,
                    )
                    .with_open_entrance(),
                )
                .expect("spawn");
            world.step(TickInput::new(100)).expect("entry");
            let release_length = world.traffic().lane_lengths_millimetres()
                [edges[occurrence.release_hop as usize].index()];
            let (length, profile) = {
                let member_state = world.state.committed.vehicles[member.index() as usize]
                    .state
                    .as_mut()
                    .expect("member");
                member_state.route_edge_index = occurrence.release_hop;
                member_state.progress_mm = release_length;
                member_state.speed_mm_s = 0;
                member_state.carry_um = 0;
                (member_state.length_mm, member_state.profile)
            };
            world
                .spawn_vehicle(
                    VehicleSpawnInput::new(
                        VehicleProfileOrdinal::from_raw(0),
                        route,
                        occurrence.release_hop + 1,
                        length,
                        0,
                    )
                    .with_open_entrance(),
                )
                .expect("leader keeps the same release boundary on green");
            let gate = world.state.compiled_route(route).expect("route").hop_gate
                [occurrence.release_hop as usize]
                .expect("release gate");
            let boundary = (1..6_000)
                .find(|&tick| {
                    world.state.committed.time_ms = (tick - 1) * 100;
                    world.state.refresh_signals();
                    let before = world.state.gate_is_restrictive(gate, profile);
                    world.state.committed.time_ms = tick * 100;
                    world.state.refresh_signals();
                    before == restrictive_before
                        && world.state.gate_is_restrictive(gate, profile) != before
                })
                .expect("ordinary signal phase boundary");
            world.state.committed.tick_index = boundary - 1;
            world.state.committed.time_ms = (boundary - 1) * 100;
            world.state.refresh_signals();
            world.step(TickInput::new(100)).expect("boundary step");
            let state = world.vehicle(member).expect("member");
            assert_eq!(
                matches!(
                    state.maneuver_traversal().expect("phase").phase(),
                    ManeuverTraversalPhase::Waiting { .. }
                ),
                restrictive_before
            );
            assert_eq!(
                world.state.gate_is_restrictive(gate, profile),
                !restrictive_before
            );
            assert!(world.latest_waiting_decisions().iter().any(|decision| {
                decision.vehicle() == member
                    && decision.anchor().hop() == occurrence.release_hop
                    && decision.outcome()
                        == if restrictive_before {
                            WaitingDecisionOutcome::NotEvaluated
                        } else {
                            WaitingDecisionOutcome::NotRequired
                        }
            }));
            let mut restored = roundtrip(&world);
            crate::admin::cutover_migration::revalidate_migrated_vehicles(&mut world.state)
                .expect("migration validates history");
            let target = world.revision();
            let origin = *target.canonical_origin();
            let descriptor = NetworkRevisionCutoverDescriptor::new(
                LfcaOriginBinding::from_canonical_origin(origin),
                LfcaOriginBinding::from_canonical_origin(origin),
                None,
                MigrationPolicyKind::SameRevisionRestore,
                world.world_binding(),
            );
            let _ = world
                .cutover_same_revision(
                    target,
                    world.committed_source().clone(),
                    &descriptor,
                    &CutoverPreflightLimits::new(1_048_576),
                )
                .expect("same-revision boundary cutover");
            world.step(TickInput::new(100)).expect("next tick");
            restored
                .step(TickInput::new(100))
                .expect("restored next tick");
            assert_eq!(world.vehicle(member), restored.vehicle(member));
        }
    }

    #[test]
    fn target_only_waiting_interval_rejects_active_cursor_without_membership() {
        use crate::admin::cutover_migration::{CrossRevisionRebinding, migrate_structural_clone};

        let base = waiting_scale_revision_with_layout(8.0, 1, ScaleLayout::AdditionalGate);
        let target = waiting_scale_revision_with_layout(8.0, 1, ScaleLayout::SecondZone);
        let (mut world, zone) = waiting_scale_world_at_delta(base, 1, 1_000);
        let rebinding = CrossRevisionRebinding::build(
            world.state.binding.revision.identity(),
            target.identity(),
        )
        .expect("rebinding");
        let origin = *target.canonical_origin();
        let source = CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://waiting-added-target",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("source"),
        };
        // entry 上游可切换；新增区间本身合法，并非一律禁止添加 WaitingZone。
        let candidate = migrate_structural_clone(
            &world.state,
            Arc::clone(&target),
            source.clone(),
            &rebinding,
        )
        .expect("upstream cursor permits target-only zone");
        assert_eq!(candidate.committed.waiting_zones.len(), 2);

        world
            .step(TickInput::new(1_000))
            .expect("cross original release");
        let state = *world
            .state
            .vehicle_state(VehicleHandle::new(0, 0))
            .expect("vehicle");
        let occurrence = world
            .state
            .compiled_route(state.route)
            .expect("route")
            .waiting[0];
        assert_eq!(state.route_edge_index, occurrence.release_hop + 1);
        assert!(state.waiting_membership.is_none());
        assert!(matches!(
            state.maneuver_traversal.expect("phase").phase,
            ManeuverTraversalPhase::Committed { .. }
        ));
        assert_eq!(world.waiting_zone(zone).expect("zone").occupancy(), 0);
        roundtrip(&world);
        let before = world.capture_snapshot().expect("before");
        let error = migrate_structural_clone(&world.state, target, source, &rebinding)
            .err()
            .expect("target interval requires an existing membership");
        assert_eq!(
            error,
            crate::CutoverError::VehicleRevalidationFailed { vehicle: 0 }
        );
        assert_eq!(world.capture_snapshot().expect("after"), before);
        world
            .step(TickInput::new(1_000))
            .expect("source remains usable");
    }

    #[test]
    fn restore_counts_sorted_members_across_occupied_and_empty_zones() {
        let revision = waiting_scale_revision_with_layout(20.0, 2, ScaleLayout::SecondZone);
        let (mut world, _) = waiting_scale_world(revision, 2);
        for _ in 0..1_000 {
            if world
                .state
                .committed
                .waiting_zones
                .iter()
                .all(|zone| zone.occupancy != 0)
            {
                break;
            }
            world.step(TickInput::new(4)).unwrap();
        }
        assert_eq!(world.state.committed.waiting_zones.len(), 2);
        assert!(
            world
                .state
                .committed
                .waiting_zones
                .iter()
                .all(|zone| zone.occupancy == 1)
        );
        let restored = roundtrip(&world);
        assert_eq!(restored.waiting_zone_members().len(), 2);
        for empty_zone in 0..2 {
            let mut copy = roundtrip(&world);
            let member = copy.state.derived.waiting_queue_ends[empty_zone]
                .head
                .unwrap();
            copy.despawn_vehicle(member).unwrap();
            let restored = roundtrip(&copy);
            assert_eq!(
                restored.state.committed.waiting_zones[empty_zone].occupancy,
                0
            );
            assert_ne!(
                restored.state.committed.waiting_zones[empty_zone].next_admission_sequence,
                0
            );
            assert_eq!(restored.waiting_zone_members().len(), 1);
        }
    }

    #[test]
    fn sparse_member_validation_and_output_keep_queue_order() {
        let revision = waiting_scale_revision_with_layout(20.0, 2, ScaleLayout::SecondZone);
        let (mut world, _) = waiting_scale_world(revision, 2);
        for _ in 0..1_000 {
            if world
                .state
                .committed
                .waiting_zones
                .iter()
                .all(|zone| zone.occupancy == 1)
            {
                break;
            }
            world.step(TickInput::new(4)).unwrap();
        }
        assert!(
            world
                .state
                .committed
                .waiting_zones
                .iter()
                .all(|zone| zone.occupancy == 1)
        );
        let expected = world.state.derived.waiting_member_rows.clone();
        world.state.committed.live_order.reverse();
        world.state.derived.active_order.reverse();
        world.state.rebuild_waiting_member_rows();
        assert_eq!(world.state.derived.waiting_member_rows, expected);
        assert!(world.state.waiting_member_rows_valid());

        let before = world.capture_snapshot().unwrap();
        world.state.derived.waiting_member_rows.reverse();
        assert_eq!(
            world.step(TickInput::new(4)),
            Err(crate::StepError::WaitingInvariantViolation)
        );
        assert_eq!(world.capture_snapshot().unwrap(), before);
        world.state.derived.waiting_member_rows.reverse();
        world.state.derived.waiting_member_rows[0].release_hop += 1;
        assert_eq!(
            world.step(TickInput::new(4)),
            Err(crate::StepError::WaitingInvariantViolation)
        );
        world.state.derived.waiting_member_rows[0].release_hop -= 1;
        let member = world.state.derived.waiting_member_rows[0].vehicle;
        world.state.derived.waiting_links[member.index() as usize].previous = Some(member);
        assert_eq!(
            world.step(TickInput::new(4)),
            Err(crate::StepError::WaitingInvariantViolation)
        );
        world.state.derived.waiting_links[member.index() as usize].previous = None;
        assert_eq!(world.capture_snapshot().unwrap(), before);
        world
            .step(TickInput::new(4))
            .expect("retry valid member batch and queue");
        assert!(world.state.waiting_state_valid());
    }

    #[test]
    fn malformed_waiting_snapshot_aggregate_fails_closed() {
        let (mut world, route, occurrence) = waiting_world();
        let entry_edge = world.route_edges(route).expect("route")[occurrence.entry_hop as usize];
        let entry_length = world.traffic().lane_lengths_millimetres()[entry_edge.index()];
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    occurrence.entry_hop,
                    entry_length - 100,
                    8_000,
                )
                .with_open_entrance(),
            )
            .expect("vehicle");
        world.step(TickInput::new(100)).expect("entry");
        let snapshot = world.capture_snapshot().expect("capture");
        let restore = |captured: &crate::CapturedSnapshot| {
            restore_lfrs(
                &encode_lfrs(captured),
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                crate::ExecutionConfig::new(std::num::NonZeroU32::MIN),
                SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
            )
            .map(|_| ())
        };

        let mut missing_membership = snapshot.clone();
        missing_membership.vehicles[0].waiting_membership = None;
        missing_membership.waiting_zones[0].occupancy = 0;
        assert_eq!(
            restore(&missing_membership),
            Err(SnapshotRestoreError::InvalidWaitingAuthority {
                snapshot_vehicle_id: 1
            })
        );

        let mut occupancy_mismatch = snapshot.clone();
        occupancy_mismatch.waiting_zones[0].occupancy = 0;
        assert_eq!(
            restore(&occupancy_mismatch),
            Err(SnapshotRestoreError::WaitingInvariantViolation)
        );

        let mut missing_zone = snapshot.clone();
        missing_zone.waiting_zones.clear();
        assert_eq!(
            restore(&missing_zone),
            Err(SnapshotRestoreError::WaitingInvariantViolation)
        );

        let mut exhausted_counter = snapshot.clone();
        exhausted_counter.waiting_zones[0].next_admission_sequence = 0;
        assert_eq!(
            restore(&exhausted_counter),
            Err(SnapshotRestoreError::WaitingInvariantViolation)
        );

        let mut duplicate_zone = snapshot.clone();
        duplicate_zone
            .waiting_zones
            .push(duplicate_zone.waiting_zones[0]);
        assert_eq!(
            restore(&duplicate_zone),
            Err(SnapshotRestoreError::InvalidWaitingZoneState)
        );

        let mut membership_without_traversal = snapshot;
        membership_without_traversal.vehicles[0].maneuver_traversal = None;
        assert!(matches!(
            restore(&membership_without_traversal),
            Err(SnapshotRestoreError::InvalidWaitingAuthority { .. })
        ));
    }

    #[test]
    fn restore_rejects_actual_waiting_gap_below_profile_minimum() {
        let revision = waiting_scale_revision_with(20.0, 2);
        let (mut world, zone) = waiting_scale_world(revision, 2);
        world.step(TickInput::new(4)).expect("admit front member");

        let front = VehicleHandle::new(0, 0);
        let front_state = world
            .state
            .vehicle_state(front)
            .copied()
            .expect("front state");
        let membership = front_state.waiting_membership.expect("front membership");
        let profile = world
            .traffic()
            .relations()
            .vehicle_profile(front_state.profile)
            .expect("profile");
        let front_progress_mm = 15_000_u32;
        let follower_progress_at_minimum = front_progress_mm
            .checked_sub(front_state.length_mm)
            .and_then(|value| value.checked_sub(profile.min_gap_mm()))
            .expect("20 metre storage fits both vehicles");

        let mut exact_gap = world.capture_snapshot().expect("capture");
        let front_index = exact_gap
            .vehicles
            .iter()
            .position(|vehicle| vehicle.waiting_membership.is_some())
            .expect("front row");
        let follower_index = exact_gap
            .vehicles
            .iter()
            .position(|vehicle| vehicle.waiting_membership.is_none())
            .expect("follower row");
        let traversal = exact_gap.vehicles[front_index]
            .maneuver_traversal
            .expect("front traversal");
        let mut follower_membership = exact_gap.vehicles[front_index]
            .waiting_membership
            .expect("front membership row");
        follower_membership.admission_sequence = 1;
        exact_gap.vehicles[front_index].route_edge_index = membership.release_hop;
        exact_gap.vehicles[front_index].progress_mm = front_progress_mm;
        exact_gap.vehicles[front_index].speed_mm_s = 0;
        exact_gap.vehicles[follower_index].route_edge_index = membership.release_hop;
        exact_gap.vehicles[follower_index].progress_mm = follower_progress_at_minimum;
        exact_gap.vehicles[follower_index].speed_mm_s = 0;
        exact_gap.vehicles[follower_index].maneuver_traversal = Some(traversal);
        exact_gap.vehicles[follower_index].waiting_membership = Some(follower_membership);
        let zone_identity = exact_gap.waiting_zones[0].waiting_zone;
        let zone_state = exact_gap
            .waiting_zones
            .iter_mut()
            .find(|state| state.waiting_zone == zone_identity)
            .expect("zone state");
        zone_state.occupancy = 2;
        zone_state.next_admission_sequence = 2;

        let restore = |captured: &crate::CapturedSnapshot| {
            restore_lfrs(
                &encode_lfrs(captured),
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                crate::ExecutionConfig::new(std::num::NonZeroU32::MIN),
                SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
            )
            .map(|_| ())
        };
        restore(&exact_gap).expect("exact profile minimum gap is valid");

        let mut one_millimetre_short = exact_gap;
        one_millimetre_short.vehicles[follower_index].progress_mm =
            follower_progress_at_minimum + 1;
        assert_eq!(
            restore(&one_millimetre_short),
            Err(SnapshotRestoreError::WaitingInvariantViolation)
        );
        assert_eq!(
            world.waiting_zone(zone).expect("source zone").occupancy(),
            1,
            "malformed restore must not mutate the source world"
        );
    }

    #[test]
    fn waiting_rebind_rejects_target_cursor_past_release_gate() {
        let revision = waiting_scale_revision();
        let (mut world, _) = waiting_scale_world(revision, 1);
        world.step(TickInput::new(4)).expect("admit member");
        let vehicle = VehicleHandle::new(0, 0);
        let state = world
            .state
            .vehicle_state(vehicle)
            .copied()
            .expect("member state");
        let membership = state.waiting_membership.expect("membership");
        let target_cursor = membership
            .release_hop
            .checked_add(1)
            .expect("fixture release has a following internal edge");
        let compiled = world
            .state
            .compiled_route(state.route)
            .expect("compiled route");
        let traversal = state.maneuver_traversal.expect("traversal");
        let maneuver = compiled
            .maneuvers
            .get(traversal.maneuver_occurrence_index as usize)
            .expect("maneuver");
        assert!(target_cursor < maneuver.exit_route_edge_index);
        assert_eq!(
            world
                .state
                .rebind_waiting_authority(state, state.route, target_cursor as usize),
            Err(WaitingBindingError::AuthorityMismatch)
        );
    }

    #[test]
    fn same_tick_release_does_not_return_physical_storage_to_later_candidate() {
        let (mut world, route, occurrence) = waiting_world();
        let entry_edge = world.route_edges(route).expect("route")[occurrence.entry_hop as usize];
        let entry_length = world.traffic().lane_lengths_millimetres()[entry_edge.index()];
        let member = world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    occurrence.entry_hop,
                    entry_length - 100,
                    8_000,
                )
                .with_open_entrance(),
            )
            .expect("member spawn");
        world.step(TickInput::new(100)).expect("member entry");
        assert!(
            world
                .vehicle(member)
                .and_then(|state| state.waiting_membership())
                .is_some()
        );

        let release_edge =
            world.route_edges(route).expect("route")[occurrence.release_hop as usize];
        let release_length = world.traffic().lane_lengths_millimetres()[release_edge.index()];
        let member_index = member.index() as usize;
        let member_state = world.state.committed.vehicles[member_index]
            .state
            .as_mut()
            .expect("member");
        member_state.route_edge_index = occurrence.release_hop;
        member_state.progress_mm = release_length;
        member_state.speed_mm_s = 0;
        member_state.carry_um = 0;
        member_state.maneuver_traversal = Some(ManeuverTraversalState {
            route,
            maneuver_occurrence_index: occurrence.maneuver_index,
            phase: ManeuverTraversalPhase::Committed {
                last_crossed_gate_hop: occurrence.entry_hop,
            },
        });

        let follower = world
            .state
            .restore_unparked_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    occurrence.entry_hop,
                    entry_length - 100,
                    8_000,
                )
                .with_open_entrance(),
                0,
                crate::VehicleStatus::Active,
                None,
                None,
                false,
            )
            .expect("follower spawn");
        world
            .state
            .workspace
            .frontier_maintenance
            .note_active_source(follower);
        world
            .state
            .committed
            .signal_aspects
            .fill(laneflow_static_contract::SignalAspect::Green);
        world
            .step(TickInput::new(100))
            .expect("release and no-grant step");

        assert!(
            world
                .vehicle(member)
                .and_then(|state| state.waiting_membership())
                .is_none()
        );
        assert!(
            world
                .vehicle(follower)
                .and_then(|state| state.waiting_membership())
                .is_none()
        );
        let zone = world.waiting_zone(occurrence.zone).expect("zone");
        assert_eq!(zone.occupancy(), 0);
        assert_eq!(zone.next_admission_sequence(), 1);
        assert!(world.latest_waiting_decisions().iter().any(|decision| {
            decision.vehicle() == follower
                && decision.outcome()
                    == WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::PhysicalStorage)
        }));
        assert!(world.latest_transition_events().iter().any(|event| {
            event.vehicle() == member
                && matches!(event.kind(), TrafficTransitionKind::WaitingLeft { .. })
        }));
        assert!(world.latest_transition_events().iter().any(|event| {
            event.vehicle() == follower
                && matches!(
                    event.kind(),
                    TrafficTransitionKind::ProjectionApplied {
                        reason: WaitingProjectionReason::PhysicalStorage,
                        ..
                    }
                )
        }));
    }

    #[test]
    fn release_gate_wins_zero_travel_tie_then_green_leader_stop_is_committed() {
        let (mut world, route, occurrence) = waiting_world();
        let entry_edge = world.route_edges(route).expect("route")[occurrence.entry_hop as usize];
        let entry_length = world.traffic().lane_lengths_millimetres()[entry_edge.index()];
        let member = world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    occurrence.entry_hop,
                    entry_length - 100,
                    8_000,
                )
                .with_open_entrance(),
            )
            .expect("member spawn");
        world.step(TickInput::new(100)).expect("member entry");

        let release_edge =
            world.route_edges(route).expect("route")[occurrence.release_hop as usize];
        let release_length = world.traffic().lane_lengths_millimetres()[release_edge.index()];
        let member_state = world.state.committed.vehicles[member.index() as usize]
            .state
            .as_mut()
            .expect("member");
        member_state.route_edge_index = occurrence.release_hop;
        member_state.progress_mm = release_length;
        member_state.speed_mm_s = 0;
        member_state.carry_um = 0;
        member_state.maneuver_traversal = Some(ManeuverTraversalState {
            route,
            maneuver_occurrence_index: occurrence.maneuver_index,
            phase: ManeuverTraversalPhase::Committed {
                last_crossed_gate_hop: occurrence.entry_hop,
            },
        });
        let profile = world
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .expect("profile");
        let _leader = world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    occurrence.release_hop + 1,
                    profile.length_mm(),
                    0,
                )
                .with_open_entrance(),
            )
            .expect("leader touching release boundary");

        world.step(TickInput::new(100)).expect("red release tie");
        assert!(matches!(
            world
                .vehicle(member)
                .and_then(|state| state.maneuver_traversal())
                .expect("traversal")
                .phase(),
            ManeuverTraversalPhase::Waiting { release_gate_hop }
                if release_gate_hop == occurrence.release_hop
        ));
        assert!(world.latest_waiting_decisions().iter().any(|decision| {
            decision.vehicle() == member
                && decision.anchor().hop() == occurrence.release_hop
                && decision.outcome() == WaitingDecisionOutcome::NotEvaluated
        }));

        world
            .state
            .committed
            .signal_aspects
            .fill(laneflow_static_contract::SignalAspect::Green);
        world.step(TickInput::new(100)).expect("green leader stop");
        let state = world.vehicle(member).expect("member");
        assert_eq!(state.route_edge_index(), occurrence.release_hop);
        assert_eq!(state.progress_mm(), release_length);
        assert!(matches!(
            state.maneuver_traversal().expect("traversal").phase(),
            ManeuverTraversalPhase::Committed {
                last_crossed_gate_hop
            } if last_crossed_gate_hop == occurrence.entry_hop
        ));
        assert!(state.waiting_membership().is_some());
        assert!(world.latest_waiting_decisions().iter().any(|decision| {
            decision.vehicle() == member
                && decision.anchor().hop() == occurrence.release_hop
                && decision.outcome() == WaitingDecisionOutcome::NotRequired
        }));
    }

    #[test]
    fn counter_exhaustion_and_each_checked_scratch_reservation_leave_step_atomic() {
        let atomic_failure = |fail_after: Option<usize>| {
            let (mut world, route, occurrence) = waiting_world();
            let entry_edge =
                world.route_edges(route).expect("route")[occurrence.entry_hop as usize];
            let entry_length = world.traffic().lane_lengths_millimetres()[entry_edge.index()];
            let _vehicle = world
                .spawn_vehicle(
                    VehicleSpawnInput::new(
                        VehicleProfileOrdinal::from_raw(0),
                        route,
                        occurrence.entry_hop,
                        entry_length - 100,
                        8_000,
                    )
                    .with_open_entrance(),
                )
                .expect("vehicle");
            world.state.arm_migration_journal(16 * 1_024).unwrap();
            let before = world.capture_snapshot().expect("before");
            let members_before = world.state.derived.waiting_member_rows.clone();
            let guard = fail_after.map(fail_waiting_reservation_after);
            let error = world
                .step(TickInput::new(100))
                .expect_err("injected failure");
            assert_eq!(error, crate::StepError::WaitingScratchAllocFailed);
            assert_eq!(world.capture_snapshot().expect("after"), before);
            assert!(world.latest_waiting_decisions().is_empty());
            assert!(world.latest_transition_events().is_empty());
            assert_eq!(world.state.derived.waiting_member_rows, members_before);
            assert_eq!(
                world
                    .state
                    .migration_journal()
                    .unwrap()
                    .records_from(0)
                    .count(),
                0
            );
            drop(guard);
            world.step(TickInput::new(100)).expect("retry");
            assert_eq!(
                world.state.committed.waiting_zones[occurrence.zone.index()]
                    .next_admission_sequence,
                1
            );
            let record = world
                .state
                .migration_journal()
                .unwrap()
                .records_from(0)
                .next()
                .unwrap();
            let JournalRecord::Tick { waiting_zones, .. } = record else {
                panic!("tick");
            };
            assert_eq!(
                waiting_zone_delta_stream(waiting_zones).collect::<Vec<_>>(),
                [(occurrence.zone, 1)]
            );
        };
        atomic_failure(Some(0));

        let (mut world, route, occurrence) = waiting_world();
        let entry_edge = world.route_edges(route).expect("route")[occurrence.entry_hop as usize];
        let entry_length = world.traffic().lane_lengths_millimetres()[entry_edge.index()];
        let _vehicle = world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    occurrence.entry_hop,
                    entry_length - 100,
                    8_000,
                )
                .with_open_entrance(),
            )
            .expect("vehicle");
        world.state.committed.waiting_zones[occurrence.zone.index()].next_admission_sequence =
            u64::MAX;
        let before = world.capture_snapshot().expect("before exhaustion");
        let next_state_capacity = world.state.workspace.next_states.capacity();
        assert_eq!(
            world.step(TickInput::new(100)),
            Err(crate::StepError::WaitingAdmissionSequenceExhausted)
        );
        assert_eq!(world.capture_snapshot().expect("after exhaustion"), before);
        assert_eq!(
            world.state.workspace.next_states.capacity(),
            next_state_capacity
        );
    }

    #[test]
    fn leader_constraint_prevents_rear_request_while_physical_front_enters() {
        let (mut world, route, occurrence) = waiting_world();
        let entry_edge = world.route_edges(route).expect("route")[occurrence.entry_hop as usize];
        let entry_length = world.traffic().lane_lengths_millimetres()[entry_edge.index()];
        let rear = world
            .state
            .place_existing_active_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    occurrence.entry_hop,
                    entry_length - 7_100,
                    8_000,
                )
                .with_open_entrance(),
            )
            .expect("rear first in live order");
        let front = world
            .state
            .place_existing_active_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    occurrence.entry_hop,
                    entry_length - 100,
                    8_000,
                )
                .with_open_entrance(),
            )
            .expect("physical front second in live order");
        world.state.committed.vehicles[rear.index() as usize]
            .state
            .as_mut()
            .expect("rear")
            .speed_mm_s = 100_000;
        world.state.committed.vehicles[front.index() as usize]
            .state
            .as_mut()
            .expect("front")
            .speed_mm_s = 100_000;

        world.step(TickInput::new(100)).expect("ordered admission");
        assert!(
            world
                .vehicle(front)
                .and_then(|state| state.waiting_membership())
                .is_some()
        );
        assert!(
            world
                .vehicle(rear)
                .and_then(|state| state.waiting_membership())
                .is_none()
        );
        assert!(world.latest_waiting_decisions().iter().any(|decision| {
            decision.vehicle() == front && decision.outcome() == WaitingDecisionOutcome::Granted
        }));
        assert!(
            world
                .latest_waiting_decisions()
                .iter()
                .all(|decision| decision.vehicle() != rear)
        );
    }

    #[test]
    fn same_tick_enter_leave_orders_events_and_despawn_without_member_is_absent() {
        let (mut world, route, mut occurrence) = waiting_world();
        occurrence.release_hop = occurrence.entry_hop;
        world.state.committed.routes[route.index() as usize]
            .compiled
            .as_mut()
            .expect("route")
            .waiting[0] = occurrence;
        let entry_edge = world.route_edges(route).expect("route")[occurrence.entry_hop as usize];
        let entry_length = world.traffic().lane_lengths_millimetres()[entry_edge.index()];
        let vehicle = world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    occurrence.entry_hop,
                    entry_length - 100,
                    8_000,
                )
                .with_open_entrance(),
            )
            .expect("vehicle");
        world.state.committed.vehicles[vehicle.index() as usize]
            .state
            .as_mut()
            .expect("vehicle")
            .speed_mm_s = 1_000_000;
        world
            .state
            .committed
            .signal_aspects
            .fill(laneflow_static_contract::SignalAspect::Green);
        world.step(TickInput::new(100)).expect("enter and leave");

        assert!(
            world
                .vehicle(vehicle)
                .and_then(|state| state.waiting_membership())
                .is_none()
        );
        assert_eq!(
            world
                .waiting_zone(occurrence.zone)
                .expect("zone")
                .next_admission_sequence(),
            1
        );
        let transition_kinds = world
            .latest_transition_events()
            .iter()
            .filter(|event| event.vehicle() == vehicle)
            .map(|event| event.kind())
            .collect::<Vec<_>>();
        assert!(
            matches!(
                transition_kinds.as_slice(),
                [
                    TrafficTransitionKind::GateCrossed { .. },
                    TrafficTransitionKind::WaitingLeft { .. },
                    TrafficTransitionKind::WaitingEntered { .. }
                ]
            ),
            "{transition_kinds:?}"
        );
        let latest = world.latest_transition_events().to_vec();
        let event_cursor = world.event_cursor();
        let record = world.despawn_vehicle(vehicle).expect("despawn");
        assert!(record.waiting_release.is_none());
        assert_eq!(world.latest_transition_events(), latest);
        assert_eq!(world.event_cursor(), event_cursor);
    }

    #[test]
    fn exact_scratch_reserve_covers_additional_rows_with_partial_spare_capacity() {
        let mut rows = Vec::<u64>::with_capacity(4);
        rows.extend([1, 2]);
        let additional = rows.capacity() - rows.len() + 1;
        reserve_waiting_exact(&mut rows, additional).expect("reserve whole additional batch");
        assert!(rows.capacity() - rows.len() >= additional);
    }

    #[test]
    fn not_required_uses_final_projected_gate_frontier() {
        let (mut world, _) = waiting_scale_world_at_delta(waiting_scale_revision(), 1, 1_000);
        let vehicle = VehicleHandle::new(0, 0);
        let mut projected = *world.state.vehicle_state(vehicle).expect("vehicle");
        let occurrence = world
            .state
            .compiled_route(projected.route)
            .expect("route")
            .waiting[0];
        world
            .state
            .prepare_waiting_step(1.0)
            .expect("unconstrained preview");
        assert!(world.state.workspace.next_states[0].1.route_edge_index > occurrence.release_hop);
        // 独立检验 output staging：实际 movement 被 entry projection 截断。
        let edge =
            world.route_edges(projected.route).expect("route")[occurrence.entry_hop as usize];
        projected.progress_mm = world.traffic().lane_lengths_millimetres()[edge.index()];
        projected.speed_mm_s = 0;
        projected.carry_um = 0;
        world
            .state
            .prepare_conflict_step(1.0, 1, None)
            .expect("formal grant");
        let mut updates = [(vehicle.index() as usize, projected)];
        world
            .state
            .finalize_waiting_step(&mut updates)
            .expect("finalize projected motion");
        world
            .state
            .finalize_conflict_step(&mut updates)
            .expect("finalize authority");
        world
            .state
            .finalize_waiting_outputs(&updates, 1)
            .expect("finalize outputs");
        let decisions = &world.state.workspace.waiting_staged_decisions;
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].vehicle(), vehicle);
        assert_eq!(decisions[0].outcome(), WaitingDecisionOutcome::Granted);
        assert_eq!(decisions[0].anchor().hop(), occurrence.entry_hop);
        assert!(
            world.state.workspace.staged_transition_events.is_empty(),
            "grant is not a successful entry"
        );
    }

    // #675：线性预言机只存在于测试，保持优化前的首错与扫描顺序。
    fn linear_waiting_bootstrap(
        world: &TrafficWorld,
        route: RouteHandle,
        cursor: usize,
        vehicle_length_mm: u32,
    ) -> Result<Option<ManeuverTraversalState>, WaitingBindingError> {
        let compiled = world
            .state
            .compiled_route(route)
            .ok_or(WaitingBindingError::InvalidRoute)?;
        let cursor = u32::try_from(cursor).map_err(|_| WaitingBindingError::InvalidRoute)?;
        for occurrence in &compiled.waiting {
            if cursor <= occurrence.release_hop && vehicle_length_mm > occurrence.storage_length_mm
            {
                return Err(WaitingBindingError::VehicleTooLong);
            }
        }
        let mut initial = None;
        for (index, maneuver) in compiled.maneuvers.iter().enumerate() {
            if !compiled
                .waiting
                .iter()
                .any(|waiting| waiting.maneuver_index as usize == index)
            {
                continue;
            }
            if cursor < maneuver.entry_route_edge_index || cursor >= maneuver.exit_route_edge_index
            {
                continue;
            }
            let hop =
                first_gate_hop(compiled, maneuver).ok_or(WaitingBindingError::InvalidRoute)?;
            if cursor > hop {
                return Err(WaitingBindingError::StatefulManeuverInterior);
            }
            let candidate = ManeuverTraversalState {
                route,
                maneuver_occurrence_index: u32::try_from(index)
                    .map_err(|_| WaitingBindingError::InvalidRoute)?,
                phase: ManeuverTraversalPhase::PreGate { next_gate_hop: hop },
            };
            if initial.replace(candidate).is_some() {
                return Err(WaitingBindingError::InvalidRoute);
            }
        }
        Ok(initial)
    }

    fn repeated_waiting_route(world: &mut TrafficWorld, count: usize) -> RouteHandle {
        let original = world
            .state
            .vehicle_state(VehicleHandle::new(0, 0))
            .expect("vehicle")
            .route;
        let path = world.route_edges(original).expect("route")[64..].to_vec();
        world
            .register_route(RouteRegisterInput::new(path.repeat(count)))
            .expect("repeated route")
    }

    #[test]
    fn waiting_lookup_bootstrap_matches_linear_oracle_at_every_route_boundary() {
        for layout in [ScaleLayout::SingleZone, ScaleLayout::SecondZone] {
            let revision = waiting_scale_revision_with_layout(8.0, 1, layout);
            let (mut world, _) = waiting_scale_world(revision, 1);
            let route = repeated_waiting_route(&mut world, 32);
            let compiled = world.state.compiled_route(route).expect("route");
            assert!(
                compiled
                    .waiting
                    .windows(2)
                    .all(|pair| pair[0].entry_hop < pair[1].entry_hop)
            );
            assert!(
                compiled
                    .maneuvers
                    .windows(2)
                    .all(|pair| pair[0].exit_route_edge_index <= pair[1].entry_route_edge_index)
            );
            for cursor in 0..=compiled.edges.len() {
                for length in [0, 4_500, 8_000, u32::MAX] {
                    assert_eq!(
                        world
                            .state
                            .validate_waiting_bootstrap(route, cursor, length),
                        linear_waiting_bootstrap(&world, route, cursor, length),
                        "cursor={cursor} length={length}"
                    );
                }
            }
            let occurrence = compiled.waiting[0];
            assert_eq!(
                world.state.validate_waiting_bootstrap(
                    route,
                    occurrence.entry_hop as usize + 1,
                    u32::MAX
                ),
                Err(WaitingBindingError::VehicleTooLong),
                "length error precedes interior error"
            );
            assert_eq!(
                world
                    .state
                    .validate_waiting_bootstrap(RouteHandle::new(u32::MAX, 0), 0, 0),
                Err(WaitingBindingError::InvalidRoute)
            );
            #[cfg(target_pointer_width = "64")]
            assert_eq!(
                world.state.validate_waiting_bootstrap(route, usize::MAX, 0),
                Err(WaitingBindingError::InvalidRoute)
            );
        }
    }

    #[test]
    fn waiting_lookup_next_occurrence_matches_linear_oracle_and_visits_at_most_one() {
        let (mut world, _) = waiting_scale_world(
            waiting_scale_revision_with_layout(8.0, 1, ScaleLayout::SecondZone),
            1,
        );
        let route = repeated_waiting_route(&mut world, 64);
        let compiled = world.state.compiled_route(route).expect("route");
        for index in 0..compiled.waiting.len() {
            let current = compiled.waiting[index];
            for preview in [
                current.entry_hop,
                current.release_hop,
                current.release_hop + 1,
                u32::MAX,
            ] {
                WAITING_LOOKUP_VISITS.with(|counts| counts.set([0; 4]));
                let actual = next_crossed_waiting(compiled, index as u32, preview);
                let expected = compiled
                    .waiting
                    .iter()
                    .skip(index + 1)
                    .find(|next| preview > next.entry_hop);
                assert_eq!(actual, expected, "index={index} preview={preview}");
                assert_eq!(
                    WAITING_LOOKUP_VISITS.with(|counts| counts.get()[3]),
                    usize::from(index + 1 < compiled.waiting.len())
                );
            }
        }
    }

    #[test]
    fn waiting_lookup_work_scales_with_length_checks_and_logarithmic_searches() {
        for count in [1_usize, 16, 256, 1_024] {
            let (mut world, _) =
                waiting_scale_world_with_route_capacity(waiting_scale_revision(), 1, 4, 8_192);
            let route = repeated_waiting_route(&mut world, count);
            let compiled = world.state.compiled_route(route).expect("route");
            let tail = *compiled.waiting.last().expect("tail");
            WAITING_LOOKUP_VISITS.with(|counts| counts.set([0; 4]));
            let state = world
                .state
                .validate_waiting_bootstrap(route, tail.entry_hop as usize, 4_500)
                .expect("bootstrap")
                .expect("traversal");
            assert_eq!(state.maneuver_occurrence_index as usize, count - 1);
            let visits = WAITING_LOOKUP_VISITS.with(|counts| counts.get());
            let limit = count.ilog2() as usize + 2;
            assert_eq!(visits[0], count);
            assert!(visits[1] <= limit && visits[2] <= limit, "{visits:?}");
            let vehicle = VehicleHandle::new(0, 0);
            world.despawn_vehicle(vehicle).expect("despawn");
            let entry_hop = world.state.compiled_route(route).unwrap().waiting[0].entry_hop;
            let edge = world.route_edges(route).unwrap()[entry_hop as usize];
            let length = world.traffic().lane_lengths_millimetres()[edge.index()];
            world
                .spawn_vehicle(
                    VehicleSpawnInput::new(
                        VehicleProfileOrdinal::from_raw(0),
                        route,
                        entry_hop,
                        length - 1,
                        10_000,
                    )
                    .with_open_entrance(),
                )
                .expect("spawn");
            WAITING_LOOKUP_VISITS.with(|counts| counts.set([0; 4]));
            world.step(TickInput::new(4)).expect("entry step");
            assert_eq!(
                WAITING_LOOKUP_VISITS.with(|counts| counts.get()[3]),
                usize::from(count > 1)
            );
            assert_eq!(
                world.latest_waiting_decisions()[0].outcome(),
                WaitingDecisionOutcome::Granted
            );
            assert_eq!(world.waiting_zone_members().len(), 1);
        }
    }

    #[test]
    fn repeated_route_tail_uses_current_occurrence_for_candidate_phase_and_outputs() {
        let (mut world, _) = waiting_scale_world(waiting_scale_revision(), 1);
        let vehicle = VehicleHandle::new(0, 0);
        let original = world.state.vehicle_state(vehicle).expect("vehicle").route;
        let path = world.route_edges(original).expect("route")[64..].to_vec();
        let route = world
            .register_route(RouteRegisterInput::new(path.repeat(128)))
            .expect("128 Waiting occurrences");
        let occurrence = world.state.compiled_route(route).expect("route").waiting[127];
        let entry_edge = world.route_edges(route).expect("route")[occurrence.entry_hop as usize];
        let entry_length = world.traffic().lane_lengths_millimetres()[entry_edge.index()];
        world
            .despawn_vehicle(vehicle)
            .expect("remove fixture vehicle");
        let vehicle = world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    occurrence.entry_hop,
                    entry_length - 1,
                    10_000,
                )
                .with_open_entrance(),
            )
            .expect("tail entry bootstrap");
        world.step(TickInput::new(4)).expect("tail admission");
        let state = *world.state.vehicle_state(vehicle).expect("vehicle");
        let traversal = state.maneuver_traversal.expect("tail phase");
        assert_eq!(traversal.maneuver_occurrence_index, 127);
        assert_eq!(
            traversal.phase,
            ManeuverTraversalPhase::Committed {
                last_crossed_gate_hop: occurrence.entry_hop,
            }
        );
        let decisions = world.latest_waiting_decisions();
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].anchor().maneuver_occurrence_index(), 127);
        assert_eq!(decisions[0].anchor().hop(), occurrence.entry_hop);
        assert_eq!(decisions[0].outcome(), WaitingDecisionOutcome::Granted);
        let mut boundary = state;
        boundary.route_edge_index = occurrence.release_hop;
        let release_edge =
            world.route_edges(route).expect("route")[occurrence.release_hop as usize];
        boundary.progress_mm = world.traffic().lane_lengths_millimetres()[release_edge.index()];
        assert_eq!(
            non_entry_gate_anchors(
                world.state.compiled_route(route).expect("route"),
                state,
                boundary,
                world.traffic().lane_lengths_millimetres()
            )
            .collect::<Vec<_>>(),
            [(127, occurrence.release_hop)]
        );
    }

    #[test]
    fn compiled_gate_index_is_sparse_and_empty_for_gate_free_route() {
        let (mut world, _) = waiting_scale_world(waiting_scale_revision(), 1);
        let route = world
            .state
            .vehicle_state(VehicleHandle::new(0, 0))
            .expect("vehicle")
            .route;
        let compiled = world.state.compiled_route(route).expect("route");
        assert_eq!(compiled.gate_hops, [64, 65]);
        assert!(compiled.next_controlled.iter().all(Option::is_none));
        let prefix = compiled.edges[..64].to_vec();
        let gate_free = world
            .register_route(RouteRegisterInput::new(prefix))
            .expect("gate-free route");
        assert!(
            world
                .state
                .compiled_route(gate_free)
                .expect("compiled prefix")
                .gate_hops
                .is_empty()
        );
        for cursor in [0, 63] {
            assert_eq!(
                world
                    .state
                    .validate_waiting_bootstrap(gate_free, cursor, u32::MAX),
                Ok(None)
            );
            assert_eq!(
                world.state.validate_waiting_bootstrap(route, cursor, 4_500),
                Ok(None)
            );
        }
    }

    #[test]
    fn completions_cover_old_and_new_occurrences_in_unified_event_batch() {
        let (mut world, _) = waiting_scale_world(waiting_scale_revision(), 1);
        world.step(TickInput::new(4)).expect("enter old membership");
        let vehicle = VehicleHandle::new(0, 0);
        let mut old = *world.state.vehicle_state(vehicle).expect("vehicle");
        let compiled = world.state.compiled_route(old.route).expect("route");
        let path = compiled.edges[64..].to_vec();
        let repeated = path.repeat(3);
        let route = world
            .register_route(RouteRegisterInput::new(repeated))
            .expect("three occurrences of one path");
        let compiled = world.state.compiled_route(route).expect("repeated route");
        let [first, second, third] = compiled.waiting.as_slice() else {
            panic!("three occurrences")
        };
        let (first, second, third) = (*first, *second, *third);
        old.route = route;
        old.route_edge_index = first.release_hop;
        old.waiting_membership
            .as_mut()
            .expect("old membership")
            .release_hop = first.release_hop;
        old.maneuver_traversal = Some(ManeuverTraversalState {
            route,
            maneuver_occurrence_index: first.maneuver_index,
            phase: ManeuverTraversalPhase::Committed {
                last_crossed_gate_hop: first.entry_hop,
            },
        });
        let plan_index = world.state.workspace.waiting_plan_by_vehicle[vehicle.index() as usize]
            .expect("entry plan")
            .get() as usize
            - 1;
        let mut plan = world.state.workspace.waiting_plans[plan_index];
        plan.maneuver_index = second.maneuver_index;
        plan.entry_hop = second.entry_hop;
        plan.release_hop = second.release_hop;
        plan.admission_sequence = Some(1);
        world.state.workspace.waiting_plans[plan_index] = plan;
        let mut next = old;
        next.route_edge_index = second.release_hop;
        next.maneuver_traversal = Some(ManeuverTraversalState {
            route,
            maneuver_occurrence_index: second.maneuver_index,
            phase: ManeuverTraversalPhase::Committed {
                last_crossed_gate_hop: second.entry_hop,
            },
        });
        let events_for =
            |world: &mut TrafficWorld, old: crate::VehicleState, next: crate::VehicleState| {
                world.state.committed.vehicles[old.handle.index() as usize].state = Some(old);
                world.state.workspace.next_state_by_vehicle[old.handle.index() as usize] = 1;
                let mut events = Vec::new();
                world
                    .state
                    .step_workspace()
                    .visit_transition_events(&[(old.handle.index() as usize, next)], 2, |event| {
                        events.push(event)
                    })
                    .unwrap();
                events
            };
        let completed = |events: Vec<TrafficTransitionEvent>| {
            events
                .into_iter()
                .filter_map(|event| match event.kind {
                    TrafficTransitionKind::ManeuverTraversalCompleted {
                        maneuver_occurrence_index,
                    } => Some(maneuver_occurrence_index),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            completed(events_for(&mut world, old, next)),
            [first.maneuver_index]
        );

        // 跨过前两次 occurrence，在第三次 entry 的 evaluation horizon 停下。
        plan.stop_hop = Some(third.entry_hop);
        plan.stop_zone = Some(third.zone);
        plan.stop_maneuver_index = Some(third.maneuver_index);
        plan.projection = Some(WaitingProjectionReason::EvaluationHorizon);
        world.state.workspace.waiting_plans[plan_index] = plan;
        next.route_edge_index = third.entry_hop;
        next.progress_mm = world.traffic().lane_lengths_millimetres()
            [world.route_edges(route).expect("route")[third.entry_hop as usize].index()];
        let events = events_for(&mut world, old, next);
        assert_eq!(
            events
                .iter()
                .filter(|event| !matches!(event.kind(), TrafficTransitionKind::GateCrossed { .. }))
                .count(),
            6
        );
        assert_eq!(
            completed(events),
            [first.maneuver_index, second.maneuver_index]
        );
        assert_eq!(
            non_entry_gate_anchors(
                world.state.compiled_route(route).expect("route"),
                old,
                next,
                world.traffic().lane_lengths_millimetres()
            )
            .map(|(_, hop)| hop)
            .collect::<Vec<_>>(),
            [first.release_hop, second.release_hop]
        );
    }

    fn exec_config(workers: u32) -> crate::ExecutionConfig {
        crate::ExecutionConfig::new(std::num::NonZeroU32::new(workers).unwrap())
    }

    fn install_execution(world: &mut TrafficWorld, workers: u32) {
        world.execution = crate::kernel::execution::WorldExecution::start_private(
            exec_config(workers),
            &world.state,
        );
    }

    /// 公开输出等价：已提交快照与其确定性摘要逐字节一致，最新决策/事件一致。
    /// 私有暂存容量与容器地址不在等价范围内（#705 §1）。
    fn assert_public_outputs_match(left: &TrafficWorld, right: &TrafficWorld) {
        let left_snapshot = left.capture_snapshot().unwrap();
        let right_snapshot = right.capture_snapshot().unwrap();
        assert_eq!(left_snapshot, right_snapshot);
        assert_eq!(
            deterministic_state_digest(&left_snapshot).unwrap(),
            deterministic_state_digest(&right_snapshot).unwrap()
        );
        assert_eq!(
            left.latest_waiting_decisions(),
            right.latest_waiting_decisions()
        );
        assert_eq!(
            left.latest_transition_events(),
            right.latest_transition_events()
        );
        assert_eq!(
            left.latest_conflict_decisions(),
            right.latest_conflict_decisions()
        );
    }

    /// 首错不变量：worker 1/2/4/8/16 × 错误位置（首/中/尾车辆与块首/块尾边界）
    /// 公开同一个 `NonFiniteMotion`，失败后世界状态与失败前完全一致、与 worker 数
    /// 无关；同 tick 重试与无故障 fresh 首拍一致；路径计数证明该拍的执行路径
    /// （#705 验收：首错与交错、retry/fresh replay）。
    #[test]
    fn preview_first_error_is_stable_across_workers_and_positions() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        // 生产分发阈值为保守 1_024；16 车世界经强制入口走真实分发。
        let _force = super::force_preview_dispatch();
        const WORLD_ID: u64 = 705_100;
        const ACTIVE: usize = 16;
        // 0/8/15 是首/中/尾车辆且对 w=2/4/8/16 都是块边界位置；14 是 w=4 的
        // 块首；5 是 w=2/4 的块内位置：覆盖错误落在块首、块尾与块内。
        for &position in &[0_usize, 5, 8, 14, 15] {
            let mut failed_reference: Option<crate::CapturedSnapshot> = None;
            for &workers in &[1_u32, 2, 4, 8, 16] {
                let mut fresh = multi_gate_world_with_id(ACTIVE, WORLD_ID);
                install_execution(&mut fresh, workers);
                fresh.step(TickInput::new(100)).unwrap();
                let fresh_snapshot = fresh.capture_snapshot().unwrap();

                let mut world = multi_gate_world_with_id(ACTIVE, WORLD_ID);
                install_execution(&mut world, workers);
                let counts_before = preview_path_counts();
                let before = world.capture_snapshot().unwrap();
                let guard = inject_preview_errors(WORLD_ID, &[position], &[]);
                let result = world.step(TickInput::new(100));
                drop(guard);
                assert_eq!(
                    result,
                    Err(crate::StepError::NonFiniteMotion),
                    "workers={workers} position={position}"
                );
                let after = world.capture_snapshot().unwrap();
                assert_eq!(after, before, "failed step must keep committed state");
                if let Some(reference) = &failed_reference {
                    assert_eq!(
                        after, *reference,
                        "failed state must not depend on worker count"
                    );
                }
                failed_reference = Some(after);

                world.step(TickInput::new(100)).unwrap();
                assert_eq!(world.capture_snapshot().unwrap(), fresh_snapshot);
                assert_eq!(
                    world.latest_waiting_decisions(),
                    fresh.latest_waiting_decisions()
                );
                assert_eq!(
                    world.latest_transition_events(),
                    fresh.latest_transition_events()
                );

                let counts = preview_path_counts();
                let delta = WaitingPreviewPathCounts {
                    dispatched: counts.dispatched - counts_before.dispatched,
                    fused: counts.fused - counts_before.fused,
                    slot_fallback: counts.slot_fallback - counts_before.slot_fallback,
                };
                if workers == 1 {
                    assert_eq!(
                        delta,
                        WaitingPreviewPathCounts {
                            dispatched: 0,
                            fused: 2,
                            slot_fallback: 0,
                        },
                        "worker=1 必须走融合路径"
                    );
                } else {
                    assert_eq!(
                        delta,
                        WaitingPreviewPathCounts {
                            dispatched: 2,
                            fused: 0,
                            slot_fallback: 0,
                        },
                        "workers={workers} 必须走真实分发"
                    );
                }
            }
        }
    }

    /// 较晚错误绝不覆盖较早义务：两个不同 kind 的注入错误共存时，公开首错
    /// 由较小逻辑位置决定，与 kind、worker 数无关（#705 验收：首错与交错）。
    #[test]
    fn earlier_preview_error_never_overridden_by_later() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let _force = super::force_preview_dispatch();
        const WORLD_ID: u64 = 705_200;
        // (NonFiniteMotion 位置, WaitingInvariantViolation 位置, 期望公开首错)。
        for (nonfinite, invariant, expected) in [
            (
                &[4_usize][..],
                &[12_usize][..],
                crate::StepError::NonFiniteMotion,
            ),
            (
                &[12][..],
                &[4][..],
                crate::StepError::WaitingInvariantViolation,
            ),
        ] {
            for &workers in &[1_u32, 4] {
                let mut world = multi_gate_world_with_id(16, WORLD_ID);
                install_execution(&mut world, workers);
                let before = world.capture_snapshot().unwrap();
                let guard = inject_preview_errors(WORLD_ID, nonfinite, invariant);
                let result = world.step(TickInput::new(100));
                drop(guard);
                assert_eq!(
                    result,
                    Err(expected),
                    "workers={workers} nonfinite={nonfinite:?} invariant={invariant:?}"
                );
                assert_eq!(world.capture_snapshot().unwrap(), before);
            }
        }
    }

    /// 可选并行暂存预留失败回退融合：冷（首拍）与热（容量已建立）暂存都被
    /// 强制回退，输出与无故障融合逐字节一致且全部成功，不新增领域错误；
    /// 计数证明分发臂与回退确实发生（#705 验收：可选 scratch 回退）。
    #[test]
    fn preview_slot_reserve_failure_falls_back_to_fused() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let _force = super::force_preview_dispatch();
        const WORLD_ID: u64 = 705_300;
        let mut reference = multi_gate_world_with_id(16, WORLD_ID);
        install_execution(&mut reference, 1);
        let mut warm = multi_gate_world_with_id(16, WORLD_ID);
        install_execution(&mut warm, 4);
        let mut cold = multi_gate_world_with_id(16, WORLD_ID);
        install_execution(&mut cold, 4);
        for tick in 1..=6 {
            reference.step(TickInput::new(100)).unwrap();
            if tick <= 2 {
                // 先无故障分发，建立并行暂存容量（热回退的前置条件）。
                warm.step(TickInput::new(100)).unwrap();
                let guard = fail_preview_slot_reserve();
                cold.step(TickInput::new(100)).unwrap();
                drop(guard);
            } else {
                let guard = fail_preview_slot_reserve();
                warm.step(TickInput::new(100)).unwrap();
                cold.step(TickInput::new(100)).unwrap();
                drop(guard);
            }
            assert_public_outputs_match(&warm, &reference);
            assert_public_outputs_match(&cold, &reference);
        }
        assert_eq!(
            preview_path_counts(),
            WaitingPreviewPathCounts {
                // warm/cold 各 6 拍全部进入分发臂。
                dispatched: 12,
                // reference（worker=1）6 拍全融合；回退拍在分发臂内另行计数。
                fused: 6,
                // cold 6 拍 + warm 后 4 拍。
                slot_fallback: 10,
            }
        );
    }

    /// 空集/单车/小工作集/增长/收缩工作集：多 worker 世界与融合参考逐步
    /// 公开输出一致；小工作集天然融合，增长相位经强制分发入口证明真实
    /// 分发与融合都被执行（#705 验收：工作集形状与生命周期变化）。
    #[test]
    fn preview_workset_shapes_match_fused_reference() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        const WORLD_ID: u64 = 705_400;
        let step_pair = |reference: &mut TrafficWorld, parallel: &mut TrafficWorld| {
            reference.step(TickInput::new(100)).unwrap();
            parallel.step(TickInput::new(100)).unwrap();
            assert_public_outputs_match(parallel, reference);
        };

        // 空集：路线全部注册但不生成任何车辆。
        let (mut reference, _) = multi_gate_world_partial(12, 0, WORLD_ID);
        let (mut parallel, _) = multi_gate_world_partial(12, 0, WORLD_ID);
        install_execution(&mut parallel, 4);
        step_pair(&mut reference, &mut parallel);
        step_pair(&mut reference, &mut parallel);

        // 单车与阈值下工作集（低于 8 Active：多 worker 也走融合）。
        for spawned in [1_usize, 7] {
            let (mut reference, _) = multi_gate_world_partial(spawned, spawned, WORLD_ID);
            let (mut parallel, _) = multi_gate_world_partial(spawned, spawned, WORLD_ID);
            install_execution(&mut parallel, 4);
            step_pair(&mut reference, &mut parallel);
            step_pair(&mut reference, &mut parallel);
        }
        assert_eq!(
            preview_path_counts(),
            WaitingPreviewPathCounts {
                dispatched: 0,
                // 每个 step_pair 是两个世界各一拍；全部低于分发阈值。
                fused: 12,
                slot_fallback: 0,
            },
            "空集/单车/阈值下不允许真实分发"
        );

        // 增长：7 → 12，仍远低于生产阈值 1_024；经强制分发入口走真实分发。
        let (mut reference, reference_routes) = multi_gate_world_partial(12, 7, WORLD_ID);
        let (mut parallel, parallel_routes) = multi_gate_world_partial(12, 7, WORLD_ID);
        install_execution(&mut parallel, 4);
        step_pair(&mut reference, &mut parallel);
        step_pair(&mut reference, &mut parallel);
        for (reference_route, parallel_route) in
            reference_routes.iter().zip(&parallel_routes).skip(7)
        {
            spawn_idle_zone_vehicle(&mut reference, *reference_route);
            spawn_idle_zone_vehicle(&mut parallel, *parallel_route);
        }
        assert_eq!(parallel.live_vehicles().len(), 12);
        let counts_before = preview_path_counts();
        {
            let _force = super::force_preview_dispatch();
            step_pair(&mut reference, &mut parallel);
            step_pair(&mut reference, &mut parallel);
        }
        let growth_counts = preview_path_counts();
        assert!(
            growth_counts.dispatched - counts_before.dispatched >= 2,
            "强制入口下多 worker 世界必须走真实分发"
        );
        // 只有参考世界（worker=1）贡献融合计数：多 worker 世界不允许回退融合。
        assert_eq!(growth_counts.fused, counts_before.fused + 2);

        // 收缩：12 → 4，回到阈值下；再清空到空集。
        let despawned = parallel.live_vehicles()[..8].to_vec();
        for vehicle in &despawned {
            reference.despawn_vehicle(*vehicle).unwrap();
            parallel.despawn_vehicle(*vehicle).unwrap();
        }
        assert_eq!(parallel.live_vehicles().len(), 4);
        let counts_before = preview_path_counts();
        step_pair(&mut reference, &mut parallel);
        let shrink_counts = preview_path_counts();
        assert_eq!(
            shrink_counts.dispatched, counts_before.dispatched,
            "<8 Active 必须回到融合路径"
        );
        // 参考世界每拍都计融合；多 worker 世界这一拍也回到融合。
        assert_eq!(shrink_counts.fused, counts_before.fused + 2);
        for vehicle in parallel.live_vehicles().to_vec() {
            reference.despawn_vehicle(vehicle).unwrap();
            parallel.despawn_vehicle(vehicle).unwrap();
        }
        step_pair(&mut reference, &mut parallel);
    }

    /// 完成前沿不变量：缺失槽位出现在首错之前时检出
    /// `WaitingInvariantViolation` 而非当成功；缺失槽位晚于首错时不改变
    /// 公开首错（#705 验收：缺失槽位、首错次序）。
    #[test]
    fn missing_preview_slot_before_first_error_is_invariant_violation() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let _force = super::force_preview_dispatch();
        const WORLD_ID: u64 = 705_500;
        // (workers, 缺失槽位位置, NonFiniteMotion 注入位置, 期望公开首错)。
        for (workers, gap, nonfinite, expected) in [
            (
                4_u32,
                5_usize,
                &[][..],
                crate::StepError::WaitingInvariantViolation,
            ),
            (4, 5, &[12][..], crate::StepError::WaitingInvariantViolation),
            (4, 12, &[5][..], crate::StepError::NonFiniteMotion),
            (2, 5, &[12][..], crate::StepError::WaitingInvariantViolation),
            (
                16,
                5,
                &[12][..],
                crate::StepError::WaitingInvariantViolation,
            ),
        ] {
            let mut world = multi_gate_world_with_id(16, WORLD_ID);
            install_execution(&mut world, workers);
            let before = world.capture_snapshot().unwrap();
            let gap_guard = drop_preview_slot_at(gap);
            let injection_guard = inject_preview_errors(WORLD_ID, nonfinite, &[]);
            let result = world.step(TickInput::new(100));
            drop(gap_guard);
            drop(injection_guard);
            assert_eq!(
                result,
                Err(expected),
                "workers={workers} gap={gap} nonfinite={nonfinite:?}"
            );
            assert_eq!(world.capture_snapshot().unwrap(), before);
        }
    }

    /// 首错交错反例（#717 审阅）：live 序 4 预览 `NonFiniteMotion` 与 live 序
    /// 12 身份失败共存时，串行逐车交错先报位置 4 的预览错误。流式融合与
    /// worker 2/4 分发都必须公开同一个首错，失败后暂存与公开状态一致；
    /// 仅身份错误时所有路径都公开 `WaitingInvariantViolation`。
    #[test]
    fn preview_error_before_identity_failure_keeps_serial_first_error() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let _force = super::force_preview_dispatch();
        const WORLD_ID: u64 = 705_750;
        const IDENTITY_POSITION: usize = 12;
        let corrupt_live_identity = |world: &mut TrafficWorld| -> crate::VehicleState {
            let handle = world.state.committed.live_order[IDENTITY_POSITION];
            world.state.committed.vehicles[handle.index() as usize]
                .state
                .take()
                .expect("vehicle state present")
        };
        let restore_live_identity = |world: &mut TrafficWorld, state: crate::VehicleState| {
            let handle = world.state.committed.live_order[IDENTITY_POSITION];
            world.state.committed.vehicles[handle.index() as usize].state = Some(state);
        };
        let staging = |world: &TrafficWorld| {
            (
                format!("{:?}", world.state.workspace.motion_cache),
                world.state.workspace.next_states.clone(),
            )
        };
        // (注入预览错误位置, 期望公开首错)。
        for (injected, expected) in [
            (&[4_usize][..], crate::StepError::NonFiniteMotion),
            (&[][..], crate::StepError::WaitingInvariantViolation),
        ] {
            let mut reference = multi_gate_world_with_id(16, WORLD_ID);
            let before = reference.capture_snapshot().unwrap();
            let evicted = corrupt_live_identity(&mut reference);
            let injection = inject_preview_errors(WORLD_ID, injected, &[]);
            let reference_result = reference
                .state
                .step_workspace()
                .prepare_waiting_step(0.1, Some(reference.execution.resources()));
            drop(injection);
            assert_eq!(reference_result, Err(expected), "融合参考首错");
            let reference_staging = staging(&reference);
            restore_live_identity(&mut reference, evicted);
            assert_eq!(reference.capture_snapshot().unwrap(), before);
            for workers in [2_u32, 4] {
                let mut world = multi_gate_world_with_id(16, WORLD_ID);
                install_execution(&mut world, workers);
                let before = world.capture_snapshot().unwrap();
                let evicted = corrupt_live_identity(&mut world);
                let injection = inject_preview_errors(WORLD_ID, injected, &[]);
                let result = world
                    .state
                    .step_workspace()
                    .prepare_waiting_step(0.1, Some(world.execution.resources()));
                drop(injection);
                assert_eq!(
                    result,
                    Err(expected),
                    "workers={workers} injected={injected:?}"
                );
                assert_eq!(
                    staging(&world),
                    reference_staging,
                    "workers={workers} 失败后暂存与融合参考一致"
                );
                restore_live_identity(&mut world, evicted);
                assert_eq!(world.capture_snapshot().unwrap(), before);
            }
        }
    }

    /// 输入表预留失败（#717 审阅）：冷态（首拍）与增长（热暂存容量不足）都
    /// 强制回退流式融合，输出与融合参考逐步一致，不新增领域错误；无注入的
    /// 增长拍恢复真实分发。
    #[test]
    fn preview_input_reserve_failure_falls_back_to_fused() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let _force = super::force_preview_dispatch();
        const WORLD_ID: u64 = 705_800;
        let (mut reference, reference_routes) = multi_gate_world_partial(16, 8, WORLD_ID);
        let (mut parallel, parallel_routes) = multi_gate_world_partial(16, 8, WORLD_ID);
        install_execution(&mut parallel, 4);
        let step_pair = |reference: &mut TrafficWorld, parallel: &mut TrafficWorld| {
            reference.step(TickInput::new(100)).unwrap();
            parallel.step(TickInput::new(100)).unwrap();
            assert_public_outputs_match(parallel, reference);
        };

        // 冷态：首拍输入表 checked 预留失败 → 融合。
        {
            let _guard = fail_preview_input_reserve();
            step_pair(&mut reference, &mut parallel);
        }
        // 热态分发，建立输入表容量（8 Active）。
        step_pair(&mut reference, &mut parallel);
        step_pair(&mut reference, &mut parallel);
        // 增长：补到 16 Active，输入预留必须真实增长；注入失败 → 融合。
        for (reference_route, parallel_route) in
            reference_routes.iter().zip(&parallel_routes).skip(8)
        {
            spawn_idle_zone_vehicle(&mut reference, *reference_route);
            spawn_idle_zone_vehicle(&mut parallel, *parallel_route);
        }
        {
            let _guard = fail_preview_input_reserve();
            step_pair(&mut reference, &mut parallel);
        }
        // 无注入增长拍：预留成功，真实分发恢复。
        step_pair(&mut reference, &mut parallel);
        assert_eq!(
            preview_path_counts(),
            WaitingPreviewPathCounts {
                // parallel 的第 2/3/5 拍。
                dispatched: 3,
                // reference 5 拍全融合 + parallel 冷态/增长两拍回退。
                fused: 7,
                slot_fallback: 2,
            }
        );
    }

    /// 逐步对拍记录：digest、最新 Waiting 决策与统一事件、tick/时间游标。
    fn dispatch_tick_record(world: &TrafficWorld) -> String {
        let snapshot = world.capture_snapshot().unwrap();
        format!(
            "{:?}|{:?}|{:?}|{:?}",
            crate::deterministic_state_digest(&snapshot).unwrap(),
            world.latest_waiting_decisions(),
            world.latest_transition_events(),
            (world.tick_index(), world.time_ms()),
        )
    }

    /// fresh restore 后首拍（#705 审阅：小场景关键组合移入 lib）：恢复前后
    /// 逐步 digest/决策/事件与 worker=1 参考一致；多 worker 臂经强制分发在
    /// 每拍真实走 P2 分发（`WaitingPreviewPathCounts` 证明），worker=1 走融合。
    #[test]
    fn dispatch_matches_after_fresh_restore_first_tick() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let _force = super::force_preview_dispatch();
        const WORLD_ID: u64 = 705_760;
        const PRE_TICKS: usize = 4;
        const POST_TICKS: usize = 6;
        let mut reference: Option<Vec<String>> = None;
        for &workers in &[1_u32, 2, 4] {
            let counts_before = preview_path_counts();
            let mut world = multi_gate_world_with_id(16, WORLD_ID);
            install_execution(&mut world, workers);
            let mut records = Vec::with_capacity(PRE_TICKS + POST_TICKS);
            for _ in 0..PRE_TICKS {
                world.step(TickInput::new(100)).unwrap();
                records.push(dispatch_tick_record(&world));
            }
            let bytes = crate::encode_lfrs(&world.capture_snapshot().unwrap());
            let mut restored = crate::restore_lfrs(
                &bytes,
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                exec_config(workers),
                crate::SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4_096),
            )
            .unwrap()
            .into_world();
            for _ in 0..POST_TICKS {
                restored.step(TickInput::new(100)).unwrap();
                records.push(dispatch_tick_record(&restored));
            }
            let counts = preview_path_counts();
            let delta = WaitingPreviewPathCounts {
                dispatched: counts.dispatched - counts_before.dispatched,
                fused: counts.fused - counts_before.fused,
                slot_fallback: counts.slot_fallback - counts_before.slot_fallback,
            };
            if workers == 1 {
                assert_eq!(
                    delta,
                    WaitingPreviewPathCounts {
                        dispatched: 0,
                        fused: PRE_TICKS + POST_TICKS,
                        slot_fallback: 0,
                    },
                    "worker=1 必须全融合"
                );
            } else {
                assert_eq!(
                    delta,
                    WaitingPreviewPathCounts {
                        dispatched: PRE_TICKS + POST_TICKS,
                        fused: 0,
                        slot_fallback: 0,
                    },
                    "workers={workers} 必须每拍真实分发"
                );
            }
            if let Some(reference) = &reference {
                assert_eq!(records, *reference, "workers={workers} restore 后对拍发散");
            } else {
                reference = Some(records);
            }
        }
    }

    /// 同修订切换后首拍（#705 审阅：小场景关键组合移入 lib）：同一制品构建
    /// 两个等价根，换根重编译全部路线后继续步进；切换后逐步 digest/决策/
    /// 事件与 worker=1 参考一致，多 worker 臂经强制分发每拍真实走 P2 分发。
    #[test]
    fn dispatch_matches_after_same_revision_cutover_first_tick() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let _force = super::force_preview_dispatch();
        const WORLD_ID: u64 = 705_770;
        const PRE_TICKS: usize = 4;
        const POST_TICKS: usize = 6;
        let base = waiting_scale_revision_with_layout(8.0, 1, ScaleLayout::IdleZones(16));
        let republished = waiting_scale_revision_with_layout(8.0, 1, ScaleLayout::IdleZones(16));
        let mut reference: Option<Vec<String>> = None;
        for &workers in &[1_u32, 2, 4] {
            let counts_before = preview_path_counts();
            let (mut world, _routes) = install_multi_gate(&base, 16, 16, WORLD_ID);
            install_execution(&mut world, workers);
            let mut records = Vec::with_capacity(PRE_TICKS + POST_TICKS);
            for _ in 0..PRE_TICKS {
                world.step(TickInput::new(100)).unwrap();
                records.push(dispatch_tick_record(&world));
            }
            let descriptor = NetworkRevisionCutoverDescriptor::new(
                LfcaOriginBinding::from_canonical_origin(*world.revision().canonical_origin()),
                LfcaOriginBinding::from_canonical_origin(*republished.canonical_origin()),
                None,
                MigrationPolicyKind::SameRevisionRestore,
                world.world_binding(),
            );
            let origin = republished.canonical_origin();
            let _events = world
                .cutover_same_revision(
                    Arc::clone(&republished),
                    CommittedNetworkSource::Published {
                        reference: PublishedLfcaReference::new(
                            "fixture://multi-gate-republished",
                            origin.canonical_artifact_digest(),
                            origin.canonical_artifact_byte_length(),
                            origin.network_revision(),
                        )
                        .unwrap(),
                    },
                    &descriptor,
                    &CutoverPreflightLimits::new(1_048_576),
                )
                .unwrap();
            for _ in 0..POST_TICKS {
                world.step(TickInput::new(100)).unwrap();
                records.push(dispatch_tick_record(&world));
            }
            let counts = preview_path_counts();
            let delta = WaitingPreviewPathCounts {
                dispatched: counts.dispatched - counts_before.dispatched,
                fused: counts.fused - counts_before.fused,
                slot_fallback: counts.slot_fallback - counts_before.slot_fallback,
            };
            if workers == 1 {
                assert_eq!(
                    delta,
                    WaitingPreviewPathCounts {
                        dispatched: 0,
                        fused: PRE_TICKS + POST_TICKS,
                        slot_fallback: 0,
                    },
                    "worker=1 必须全融合"
                );
            } else {
                assert_eq!(
                    delta,
                    WaitingPreviewPathCounts {
                        dispatched: PRE_TICKS + POST_TICKS,
                        fused: 0,
                        slot_fallback: 0,
                    },
                    "workers={workers} 必须每拍真实分发"
                );
            }
            if let Some(reference) = &reference {
                assert_eq!(
                    records, *reference,
                    "workers={workers} 同修订切换后对拍发散"
                );
            } else {
                reference = Some(records);
            }
        }
    }

    // ------------------------------------------------------------------
    // #706 增量 C：P5 最终运动真实分发（worker 投机 + 协调器保序规范消费）。
    // 逐车单元在任务内按原顺序求值；协调器按 Active 序消费：该车失败在此处
    // 返回、到达观察在此处真实预留、然后 updates 接纳（checkpoint-map §4）。
    // ------------------------------------------------------------------

    use crate::kernel::tick::{
        MotionPathCounts, drop_motion_slot_at, fail_motion_arrival_reserve,
        fail_motion_input_reserve, fail_motion_slot_reserve, force_motion_dispatch,
        inject_motion_nonfinite, last_motion_dispatch_stats, motion_cache_use,
        motion_diagnostic_counts, motion_path_counts,
    };

    /// 逐 tick 公开对拍记录：digest、Waiting/Conflict 决策、统一事件、
    /// 结局（含停车到达观察）。
    fn motion_tick_record(world: &TrafficWorld, outcome: &crate::StepOutcome) -> String {
        let snapshot = world.capture_snapshot().unwrap();
        format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            crate::deterministic_state_digest(&snapshot).unwrap(),
            world.latest_waiting_decisions(),
            world.latest_conflict_decisions(),
            world.latest_transition_events(),
            (outcome.tick_index(), outcome.time_ms()),
            outcome.parking_arrivals(),
        )
    }

    /// 两辆车位的停车到达场景：A 在车位入口前 1 m（本拍首次到达、已预约），
    /// B 在 12 m（active 序在后）。返回未步进的世界。
    fn parking_arrival_world(workers: u32, world_id: u64) -> TrafficWorld {
        let (mut world, route, space, entry_progress) = parking_route_world(workers, world_id);
        let a = world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    entry_progress - 1,
                    10_000,
                )
                .with_open_entrance(),
            )
            .unwrap();
        world
            .reserve_parking(
                a,
                crate::ReserveParkingTarget::ExplicitSpace {
                    space,
                    entry_route_occurrence: 0,
                },
            )
            .unwrap();
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    12_000,
                    10_000,
                )
                .with_open_entrance(),
            )
            .unwrap();
        world
    }

    /// 核心反例（checkpoint-map §4 A1）：逻辑较前 A 车计算成功且首次到达、
    /// 其到达 reserve 注入失败；逻辑较晚 B 车注入 NonFiniteMotion ⇒ 无论 B
    /// 在 worker 上多早完成，规范消费先兑现 A 的到达预留义务，公开
    /// `ParkingObservationAllocFailed`；失败后状态不变，清注入同 tick 重试
    /// 与 fresh 世界同拍一致。
    #[test]
    fn earlier_arrival_reserve_failure_beats_later_motion_error() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        const WORLD_ID: u64 = 706_100;
        let _force = force_motion_dispatch();
        let counts_before = motion_path_counts();
        let mut fresh = parking_arrival_world(4, WORLD_ID);
        let fresh_outcome = fresh.step(TickInput::new(100)).unwrap();
        assert_eq!(
            fresh_outcome.parking_arrivals().len(),
            1,
            "fresh 首拍 A 必须首次到达"
        );
        let fresh_snapshot = fresh.capture_snapshot().unwrap();

        let mut world = parking_arrival_world(4, WORLD_ID);
        let before = world.capture_snapshot().unwrap();
        let reserve_guard = fail_motion_arrival_reserve();
        let nonfinite_guard = inject_motion_nonfinite(WORLD_ID, &[1]);
        let result = world.step(TickInput::new(100));
        drop(reserve_guard);
        drop(nonfinite_guard);
        assert_eq!(result, Err(crate::StepError::ParkingObservationAllocFailed));
        assert_eq!(world.capture_snapshot().unwrap(), before);
        let retry = world.step(TickInput::new(100)).unwrap();
        assert_eq!(retry, fresh_outcome, "同 tick 重试必须等于 fresh 首拍");
        assert_eq!(world.capture_snapshot().unwrap(), fresh_snapshot);
        let counts = motion_path_counts();
        assert_eq!(
            counts.dispatched - counts_before.dispatched,
            3,
            "fresh + 失败 + 重试拍全部真实分发: {counts:?}"
        );
        assert_eq!(
            counts.fused - counts_before.fused,
            0,
            "本场景不允许融合计数: {counts:?}"
        );
    }

    /// 首错稳定：NonFiniteMotion 注入首/中/尾 Active 位置 × worker 1/2/4：
    /// 公开同一个错误、失败后已提交状态与失败前一致、清注入重试与 w1 参考
    /// 首拍一致；路径计数按臂互斥（worker=1 融合、>1 分发）。
    #[test]
    fn motion_first_error_is_stable_across_workers_and_positions() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        const WORLD_ID: u64 = 706_110;
        let _force = force_motion_dispatch();
        let counts_before = motion_path_counts();
        let mut reference: Option<crate::StepOutcome> = None;
        for &position in &[0_usize, 8, 15] {
            for &workers in &[1_u32, 2, 4] {
                let mut world = multi_gate_world_with_id(16, WORLD_ID);
                install_execution(&mut world, workers);
                let before = world.capture_snapshot().unwrap();
                let guard = inject_motion_nonfinite(WORLD_ID, &[position]);
                let result = world.step(TickInput::new(100));
                drop(guard);
                assert_eq!(
                    result,
                    Err(crate::StepError::NonFiniteMotion),
                    "position={position} workers={workers}"
                );
                assert_eq!(world.capture_snapshot().unwrap(), before);
                let retry = world.step(TickInput::new(100)).unwrap();
                if workers == 1 {
                    reference = Some(retry);
                } else {
                    assert_eq!(
                        retry,
                        reference.clone().unwrap(),
                        "position={position} workers={workers} 重试必须等于 w1 参考"
                    );
                }
            }
        }
        let counts = motion_path_counts();
        let delta = MotionPathCounts {
            dispatched: counts.dispatched - counts_before.dispatched,
            fused: counts.fused - counts_before.fused,
            slot_fallback: counts.slot_fallback - counts_before.slot_fallback,
        };
        // 每 (position, workers) 臂 = 失败拍 + 重试拍 = 2 拍。
        assert_eq!(
            delta,
            MotionPathCounts {
                dispatched: 2 * 3 * 2,
                fused: 2 * 3,
                slot_fallback: 0,
            },
            "融合/分发/回退互斥计数: {delta:?}"
        );
    }

    /// 跨块多错误：两个注入位置（worker=4、块大小 2）无论先写哪个位置，
    /// 公开的都是 Active 序较小位置的同一个 NonFiniteMotion 首错、失败拍
    /// 状态不变。表述修正（R5）：两位移映射同一错误枚举，本测试证明的是
    /// 「书写顺序不影响公开首错」而非完成序区分；完成序的区分由
    /// conflict_first_error 的双错误类别臂（NonFinite × downstream CIV）
    /// 覆盖。
    #[test]
    fn motion_canonical_first_error_across_chunk_boundaries() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        const WORLD_ID: u64 = 706_120;
        let _force = force_motion_dispatch();
        for positions in [&[3_usize, 11][..], &[11, 3]] {
            let mut world = multi_gate_world_with_id(16, WORLD_ID);
            install_execution(&mut world, 4);
            let before = world.capture_snapshot().unwrap();
            let guard = inject_motion_nonfinite(WORLD_ID, positions);
            let result = world.step(TickInput::new(100));
            drop(guard);
            assert_eq!(result, Err(crate::StepError::NonFiniteMotion));
            assert_eq!(world.capture_snapshot().unwrap(), before);
            world.step(TickInput::new(100)).unwrap();
        }
    }

    /// 可选暂存回退：输入表/结果槽位的冷态（首拍）与热态（容量已建立）
    /// 预留注入失败都退回融合路径，逐拍输出与 w1 融合参考一致，不新增
    /// 领域错误；回退计数与分发计数互斥。
    #[test]
    fn motion_scratch_reserve_failure_falls_back_to_fused() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        const WORLD_ID: u64 = 706_130;
        let _force = force_motion_dispatch();
        let run = |workers: u32, inject: Option<fn() -> crate::kernel::tick::MotionBoolGuard>| {
            let mut world = multi_gate_world_with_id(16, WORLD_ID);
            install_execution(&mut world, workers);
            let mut records = Vec::new();
            for tick in 0..4 {
                let _guard = inject.map(|arm| arm());
                let outcome = world.step(TickInput::new(100)).unwrap();
                records.push(motion_tick_record(&world, &outcome));
                assert_eq!(tick + 1, world.tick_index() as usize);
            }
            records
        };
        let reference = run(1, None);
        // 冷态：首拍输入表预留失败；其后热态槽位预留失败一拍。
        let counts_before = motion_path_counts();
        let mut world = multi_gate_world_with_id(16, WORLD_ID);
        install_execution(&mut world, 4);
        let mut records = Vec::new();
        {
            let _guard = fail_motion_input_reserve();
            let outcome = world.step(TickInput::new(100)).unwrap();
            records.push(motion_tick_record(&world, &outcome));
        }
        for _ in 0..2 {
            let outcome = world.step(TickInput::new(100)).unwrap();
            records.push(motion_tick_record(&world, &outcome));
        }
        {
            let _guard = fail_motion_slot_reserve();
            let outcome = world.step(TickInput::new(100)).unwrap();
            records.push(motion_tick_record(&world, &outcome));
        }
        assert_eq!(records, reference, "回退拍输出必须等于融合参考");
        let counts = motion_path_counts();
        let delta = MotionPathCounts {
            dispatched: counts.dispatched - counts_before.dispatched,
            fused: counts.fused - counts_before.fused,
            slot_fallback: counts.slot_fallback - counts_before.slot_fallback,
        };
        assert_eq!(
            delta,
            MotionPathCounts {
                dispatched: 2,
                fused: 0,
                slot_fallback: 2,
            },
            "回退与分发互斥: {delta:?}"
        );
        let _ = run;
    }

    /// 完成前沿不变量：join 后首错之前出现缺失槽位检出
    /// `ConflictInvariantViolation` 而非当成功；缺失槽位晚于首错时不改变
    /// 公开首错。
    #[test]
    fn missing_motion_slot_before_first_error_is_detected() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        const WORLD_ID: u64 = 706_140;
        let _force = force_motion_dispatch();
        for (gap, expected) in [
            (5_usize, crate::StepError::ConflictInvariantViolation),
            (15, crate::StepError::NonFiniteMotion),
        ] {
            let mut world = multi_gate_world_with_id(16, WORLD_ID);
            install_execution(&mut world, 4);
            let before = world.capture_snapshot().unwrap();
            let gap_guard = drop_motion_slot_at(gap);
            let error_guard = inject_motion_nonfinite(WORLD_ID, &[12]);
            let result = world.step(TickInput::new(100));
            drop(gap_guard);
            drop(error_guard);
            assert_eq!(result, Err(expected), "gap={gap}");
            assert_eq!(world.capture_snapshot().unwrap(), before);
            world.step(TickInput::new(100)).unwrap();
        }
    }

    /// 多线程参与（R5 表述修正：participating_threads 度量出现过的线程
    /// 数，不等于同时执行重叠；真实并发重叠由 execution.rs 屏障测试覆
    /// 盖）：64 车、worker=4、连续 16 拍自然强制分发，每拍 8 块全部
    /// 完成且票据取完，参与线程数峰值 ≥ 2（调用线程 + 池任务）。
    #[test]
    fn motion_dispatch_multi_thread_participation() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        const WORLD_ID: u64 = 706_150;
        let _force = force_motion_dispatch();
        let mut world = multi_gate_world_with_id(64, WORLD_ID);
        install_execution(&mut world, 4);
        let mut peak_threads = 1_usize;
        for _ in 0..16 {
            world.step(TickInput::new(100)).unwrap();
            let stats = last_motion_dispatch_stats().expect("分发统计");
            assert_eq!(stats.dispatched_chunks, 8);
            assert_eq!(stats.completed_chunks, 8);
            assert_eq!(stats.ticket_grabs, 8);
            peak_threads = peak_threads.max(stats.participating_threads);
        }
        assert!(
            peak_threads >= 2,
            "必须观察到真实多线程参与，peak={peak_threads}"
        );
    }

    /// 小场景强制分发等价：multi-gate（Waiting 密集）worker 2/4 逐拍
    /// digest/决策/事件/结局与 worker=1 融合参考一致。
    #[test]
    fn motion_dispatch_matches_fused_reference() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        const WORLD_ID: u64 = 706_160;
        let _force = force_motion_dispatch();
        let run = |workers: u32| {
            let mut world = multi_gate_world_with_id(16, WORLD_ID);
            install_execution(&mut world, workers);
            let mut records = Vec::new();
            for _ in 0..6 {
                let outcome = world.step(TickInput::new(100)).unwrap();
                records.push(motion_tick_record(&world, &outcome));
            }
            records
        };
        let reference = run(1);
        for workers in [2_u32, 4, 8, 16] {
            assert_eq!(
                run(workers),
                reference,
                "workers={workers} 必须与融合参考逐拍一致"
            );
        }
        let counts = motion_path_counts();
        eprintln!("DEBUG counts={counts:?}");
        assert!(counts.dispatched >= 12 && counts.fused >= 6);
    }

    /// 停车到达场景强制分发等价：worker 2/4 的逐拍记录（含
    /// `parking_arrivals`）与 worker=1 融合参考一致。
    #[test]
    fn motion_parking_arrival_matches_fused_reference() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        const WORLD_ID: u64 = 706_170;
        let _force = force_motion_dispatch();
        let run = |workers: u32| {
            let mut world = parking_arrival_world(workers, WORLD_ID);
            let mut records = Vec::new();
            for _ in 0..3 {
                let outcome = world.step(TickInput::new(100)).unwrap();
                records.push(motion_tick_record(&world, &outcome));
            }
            records
        };
        let reference = run(1);
        for workers in [2_u32, 4, 8, 16] {
            assert_eq!(
                run(workers),
                reference,
                "workers={workers} 停车到达必须与融合参考一致"
            );
        }
    }

    /// 缓存与重算计数口径：w1 融合与 w4 强制分发的 cache hit/miss 与
    /// 运动内核/horizon 重算计数总计一致（任务线程增量经块级记录汇总）。
    #[test]
    fn motion_counters_match_between_fused_and_dispatched() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        const WORLD_ID: u64 = 706_180;
        let _force = force_motion_dispatch();
        let _diagnostics = crate::kernel::tick::enable_motion_diagnostics();
        let run = |workers: u32| {
            let mut world = multi_gate_world_with_id(16, WORLD_ID);
            install_execution(&mut world, workers);
            let cache_before = motion_cache_use();
            let diag_before = motion_diagnostic_counts();
            for _ in 0..6 {
                world.step(TickInput::new(100)).unwrap();
            }
            let cache = motion_cache_use();
            let diag = motion_diagnostic_counts();
            (
                cache.hits - cache_before.hits,
                cache.misses - cache_before.misses,
                diag.0 - diag_before.0,
                diag.1 - diag_before.1,
            )
        };
        let fused = run(1);
        let dispatched = run(4);
        assert_eq!(
            dispatched, fused,
            "分发路径计数必须经块级记录汇总后与融合一致: fused={fused:?} dispatched={dispatched:?}"
        );
        assert!(fused.0 + fused.1 > 0, "场景必须覆盖复用判定");
    }

    // ------------------------------------------------------------------
    // #706 增量 D：P3 候选求值真实分发（多段报告 + 协调器保序规范消费 +
    // F1/F2/F3b/F4 真实预留原位）。逐车计算在冻结三腿视图上按串行同序
    // 求值；协调器按 live×gate 发现序消费（checkpoint-map §5）。
    // 夹具：multi_gate（16 候选位置，空区间 Waiting 门）覆盖 None/Staged/
    // 纯 Waiting 与首错矩阵；conflict_scale（单车到达真冲突区，cells/
    // downstream 非空 Computed 候选）覆盖 F1/F2/F3b/F4 与同车段序。
    // ------------------------------------------------------------------

    use crate::admin::cutover_migration::tests::{conflict_scale_revision, conflict_scale_world};
    use crate::kernel::conflict::conflict_work_counts;
    use crate::kernel::conflict_tick::{
        ConflictPathCounts, conflict_path_counts, drop_conflict_slot_at,
        enable_conflict_diagnostics, fail_conflict_cell_work_reserve, fail_conflict_cells_reserve,
        fail_conflict_downstream_pool_reserve, fail_conflict_downstream_work_reserve,
        fail_conflict_input_reserve, fail_conflict_slot_reserve, force_conflict_dispatch,
        force_conflict_fuse, inject_conflict_invariant_downstream, inject_conflict_nonfinite,
        last_conflict_dispatch_stats,
    };

    /// P3 工作区语义池字节级记录：候选（含 cells/downstream 区间偏移）、
    /// 两个全池与 eligibility/motion plan 表。私有暂存容量、输入表与槽位
    /// 不在等价范围内（#705 §1 同口径）。
    fn conflict_workspace_record(world: &mut TrafficWorld) -> String {
        let step = world.state.step_workspace();
        format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            step.workspace.conflict_candidates,
            step.workspace.conflict_candidate_cells,
            step.workspace.conflict_candidate_downstream,
            step.workspace.conflict_next_eligibility,
            step.workspace.conflict_motion_by_vehicle,
            // R1：两个工作缓冲的长度纳入对拍（只属当前候选、不累积）。
            step.workspace.conflict_cell_work.len(),
            step.workspace.conflict_downstream_work.len(),
        )
    }

    /// 核心反例（checkpoint-map §5 D4-1）：候选车 cells 段已物化成功（F2
    /// 位到达），且同车更晚 downstream 检查注入 ConflictInvariantViolation；
    /// F2 真实预留原位先于 downstream 检查 ⇒ 公开
    /// `ConflictScratchAllocFailed` 而非 downstream 的 CIV。预演臂证明
    /// downstream 注入确实命中；失败拍状态不变；清注入同 tick 重试与
    /// fresh 世界同拍一致。
    #[test]
    fn earlier_f2_failure_beats_later_downstream_invariant() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let _force = force_conflict_dispatch();
        let revision = conflict_scale_revision();
        let mut probe = conflict_scale_world(Arc::clone(&revision), 16);
        install_execution(&mut probe, 4);
        let probe_id = probe.state.binding.world_id;
        let guard = inject_conflict_invariant_downstream(probe_id, &[0]);
        let result = probe.step(TickInput::new(4));
        drop(guard);
        assert_eq!(
            result,
            Err(crate::StepError::ConflictInvariantViolation),
            "downstream CIV 注入必须命中（预演臂）"
        );

        let mut fresh = conflict_scale_world(Arc::clone(&revision), 16);
        install_execution(&mut fresh, 4);
        let fresh_outcome = fresh.step(TickInput::new(4)).unwrap();
        let fresh_snapshot = fresh.capture_snapshot().unwrap();
        let fresh_workspace = conflict_workspace_record(&mut fresh);

        let mut world = conflict_scale_world(Arc::clone(&revision), 16);
        install_execution(&mut world, 4);
        let world_id = world.state.binding.world_id;
        let before = world.capture_snapshot().unwrap();
        let cells_guard = fail_conflict_cells_reserve();
        let downstream_guard = inject_conflict_invariant_downstream(world_id, &[0]);
        let result = world.step(TickInput::new(4));
        drop(cells_guard);
        drop(downstream_guard);
        assert_eq!(
            result,
            Err(crate::StepError::ConflictScratchAllocFailed),
            "F2 真实预留失败必须先于同车更晚 downstream CIV 公开"
        );
        assert_eq!(world.capture_snapshot().unwrap(), before);
        let retry = world.step(TickInput::new(4)).unwrap();
        assert_eq!(retry, fresh_outcome, "同 tick 重试必须等于 fresh 首拍");
        assert_eq!(world.capture_snapshot().unwrap(), fresh_snapshot);
        assert_eq!(
            conflict_workspace_record(&mut world),
            fresh_workspace,
            "重试后的工作区语义池必须与 fresh 一致"
        );
    }

    /// 核心反例（checkpoint-map §5 D4-2）：F1（cell 工作区真实预留）先于
    /// 同车任何更晚领域错误——注入 F1 失败 + 同车 downstream CIV ⇒ 公开
    /// `ConflictScratchAllocFailed`。同 D4-1 的失败拍不变 + 同 tick 重试
    /// 规范。
    #[test]
    fn earlier_f1_failure_beats_later_downstream_invariant() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let _force = force_conflict_dispatch();
        let revision = conflict_scale_revision();
        let mut fresh = conflict_scale_world(Arc::clone(&revision), 16);
        install_execution(&mut fresh, 4);
        let fresh_outcome = fresh.step(TickInput::new(4)).unwrap();
        let fresh_snapshot = fresh.capture_snapshot().unwrap();

        let mut world = conflict_scale_world(Arc::clone(&revision), 16);
        install_execution(&mut world, 4);
        let world_id = world.state.binding.world_id;
        let before = world.capture_snapshot().unwrap();
        let cell_work_guard = fail_conflict_cell_work_reserve();
        let downstream_guard = inject_conflict_invariant_downstream(world_id, &[0]);
        let result = world.step(TickInput::new(4));
        drop(cell_work_guard);
        drop(downstream_guard);
        assert_eq!(
            result,
            Err(crate::StepError::ConflictScratchAllocFailed),
            "F1 真实预留失败必须先于同车更晚 downstream CIV 公开"
        );
        assert_eq!(world.capture_snapshot().unwrap(), before);
        let retry = world.step(TickInput::new(4)).unwrap();
        assert_eq!(retry, fresh_outcome, "同 tick 重试必须等于 fresh 首拍");
        assert_eq!(world.capture_snapshot().unwrap(), fresh_snapshot);
    }

    /// 首错矩阵：NonFiniteMotion 注入位置 × worker 1/2/4 公开同一首错；
    /// 失败拍状态不变；重试拍等于 w1 参考；融合/分发/回退互斥计数。
    /// 表述注记（R5）：可用夹具每拍至多一个 Computed 候选，per-position
    /// 双错误类别（如 NonFinite × downstream CIV）矩阵受夹具限制未建，
    /// 消费序先于完成序的性质由 F1/F2/F4/F3b 位序反例覆盖。
    #[test]
    fn conflict_first_error_is_stable_across_workers_and_positions() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        const WORLD_ID: u64 = 706_220;
        let _force = force_conflict_dispatch();
        let counts_before = conflict_path_counts();
        let mut reference: Option<crate::StepOutcome> = None;
        for &position in &[0_usize, 8, 15] {
            for &workers in &[1_u32, 2, 4] {
                let mut world = multi_gate_world_with_id(16, WORLD_ID);
                install_execution(&mut world, workers);
                let before = world.capture_snapshot().unwrap();
                let guard = inject_conflict_nonfinite(WORLD_ID, &[position]);
                let result = world.step(TickInput::new(100));
                drop(guard);
                assert_eq!(
                    result,
                    Err(crate::StepError::NonFiniteMotion),
                    "position={position} workers={workers}"
                );
                assert_eq!(world.capture_snapshot().unwrap(), before);
                let retry = world.step(TickInput::new(100)).unwrap();
                if workers == 1 {
                    reference = Some(retry);
                } else {
                    assert_eq!(
                        retry,
                        reference.clone().unwrap(),
                        "position={position} workers={workers} 重试必须等于 w1 参考"
                    );
                }
            }
        }
        let counts = conflict_path_counts();
        let delta = ConflictPathCounts {
            dispatched: counts.dispatched - counts_before.dispatched,
            fused: counts.fused - counts_before.fused,
            slot_fallback: counts.slot_fallback - counts_before.slot_fallback,
        };
        // 每 (position, workers) 臂 = 失败拍 + 重试拍 = 2 拍。
        assert_eq!(
            delta,
            ConflictPathCounts {
                dispatched: 2 * 3 * 2,
                fused: 2 * 3,
                slot_fallback: 0,
            },
            "融合/分发/回退互斥计数: {delta:?}"
        );
    }

    /// 可选暂存回退：输入表（冷态首拍）与结果槽位（热态）预留注入失败都
    /// 退回融合路径，逐拍输出与 w1 融合参考一致，不新增领域错误；回退
    /// 计数与分发计数互斥。
    #[test]
    fn conflict_scratch_reserve_failure_falls_back_to_fused() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        const WORLD_ID: u64 = 706_230;
        let _force = force_conflict_dispatch();
        let mut reference = multi_gate_world_with_id(16, WORLD_ID);
        let outcome = reference.step(TickInput::new(100)).unwrap();
        let expected = (
            motion_tick_record(&reference, &outcome),
            conflict_workspace_record(&mut reference),
        );
        let counts_before = conflict_path_counts();
        // 两种预留失败各用近门首拍，防止注入落在过门后的空工作集。
        for input_failure in [true, false] {
            let mut world = multi_gate_world_with_id(16, WORLD_ID);
            install_execution(&mut world, 4);
            let _input = input_failure.then(fail_conflict_input_reserve);
            let _slot = (!input_failure).then(fail_conflict_slot_reserve);
            let outcome = world.step(TickInput::new(100)).unwrap();
            assert_eq!(
                (
                    motion_tick_record(&world, &outcome),
                    conflict_workspace_record(&mut world)
                ),
                expected
            );
        }
        let counts = conflict_path_counts();
        assert_eq!(
            (
                counts.dispatched - counts_before.dispatched,
                counts.fused - counts_before.fused,
                counts.slot_fallback - counts_before.slot_fallback
            ),
            (0, 0, 2)
        );
    }

    /// 完成前沿不变量：join 后首错之前出现缺失槽位检出
    /// `ConflictInvariantViolation` 而非当成功；缺失槽位晚于首错时不改变
    /// 公开首错。
    #[test]
    fn missing_conflict_slot_before_first_error_is_detected() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        const WORLD_ID: u64 = 706_240;
        let _force = force_conflict_dispatch();
        for (gap, expected) in [
            (3_usize, crate::StepError::ConflictInvariantViolation),
            (15, crate::StepError::NonFiniteMotion),
        ] {
            let mut world = multi_gate_world_with_id(16, WORLD_ID);
            install_execution(&mut world, 4);
            let before = world.capture_snapshot().unwrap();
            let gap_guard = drop_conflict_slot_at(gap);
            let error_guard = inject_conflict_nonfinite(WORLD_ID, &[12]);
            let result = world.step(TickInput::new(100));
            drop(gap_guard);
            drop(error_guard);
            assert_eq!(result, Err(expected), "gap={gap}");
            assert_eq!(world.capture_snapshot().unwrap(), before);
            world.step(TickInput::new(100)).unwrap();
        }
    }

    /// 多线程参与（R5 表述修正：participating_threads 度量的是出现过的
    /// 线程数，不等于同时执行的重叠；真实并发重叠由 execution.rs 屏障
    /// 测试覆盖）：600 条独立路线的近门车、worker=4、4 个新世界首拍强制分发，每拍
    /// 8 块全部完成且参与线程 > 1；P3 调度统计按阶段独立登记。
    #[test]
    fn conflict_dispatch_multi_thread_participation() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let _force = force_conflict_dispatch();
        let mut peak_threads = 1_usize;
        for _ in 0..4 {
            let mut world = multi_gate_world(600);
            install_execution(&mut world, 4);
            world.step(TickInput::new(100)).unwrap();
            assert_eq!(world.state.workspace.conflict_inputs.len(), 600);
            let stats = last_conflict_dispatch_stats().expect("P3 分发统计");
            assert_eq!(stats.dispatched_chunks, 8);
            assert_eq!(stats.completed_chunks, 8);
            assert_eq!(stats.ticket_grabs, 8);
            peak_threads = peak_threads.max(stats.participating_threads);
        }
        assert!(
            peak_threads >= 2,
            "必须观察到真实多线程参与，peak={peak_threads}"
        );
    }

    /// 小场景强制分发等价：multi-gate 16 车（含 Granted 候选、staged
    /// 决定、旧 reservation 跳过的 Active 紧凑位语义）worker 2/4 的逐拍
    /// 公开记录与工作区语义池字节级记录同 worker=1 融合参考一致。
    #[test]
    fn conflict_dispatch_matches_fused_reference() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        const WORLD_ID: u64 = 706_260;
        let _force = force_conflict_dispatch();
        let before = conflict_path_counts();
        let run = |workers: u32| {
            let mut world = multi_gate_world_with_id(16, WORLD_ID);
            install_execution(&mut world, workers);
            let mut records = Vec::new();
            for _ in 0..6 {
                let outcome = world.step(TickInput::new(100)).unwrap();
                records.push((
                    motion_tick_record(&world, &outcome),
                    conflict_workspace_record(&mut world),
                ));
            }
            records
        };
        let reference = run(1);
        for workers in [2_u32, 4, 8, 16] {
            assert_eq!(
                run(workers),
                reference,
                "workers={workers} 必须与融合参考逐拍一致"
            );
        }
        let counts = conflict_path_counts();
        // 每臂仅首拍有近门候选，后五拍为空；w1 始终融合。
        assert_eq!(
            (
                counts.dispatched - before.dispatched,
                counts.fused - before.fused
            ),
            (4, 26)
        );
    }

    /// Computed 候选等价：conflict_scale 首拍单车到达真冲突区（cells/
    /// downstream 非空、Granted 组合资源），worker 2/4 的逐拍记录与工作区
    /// 语义池同 worker=1 融合参考一致。
    #[test]
    fn conflict_computed_candidate_matches_fused_reference() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let _force = force_conflict_dispatch();
        let revision = conflict_scale_revision();
        let run = |workers: u32| {
            let mut world = conflict_scale_world(Arc::clone(&revision), 16);
            install_execution(&mut world, workers);
            let mut records = Vec::new();
            for _ in 0..4 {
                let outcome = world.step(TickInput::new(4)).unwrap();
                records.push((
                    motion_tick_record(&world, &outcome),
                    conflict_workspace_record(&mut world),
                ));
            }
            records
        };
        let reference = run(1);
        for workers in [2_u32, 4, 8, 16] {
            assert_eq!(
                run(workers),
                reference,
                "workers={workers} Computed 候选必须与融合参考一致"
            );
        }
    }

    /// 停车 + 下游净空小场景强制分发等价：两车同一路线（前车紧邻车位
    /// 入口、已预约），downstream 边界 NoGrant 折入 preflight 与零候选
    /// 车辆路径都在分发臂下与融合逐拍一致。
    #[test]
    fn conflict_parking_scenario_matches_fused_reference() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        const WORLD_ID: u64 = 706_270;
        let _force = force_conflict_dispatch();
        let run = |workers: u32| {
            let mut world = parking_arrival_world(workers, WORLD_ID);
            let mut records = Vec::new();
            for _ in 0..3 {
                let outcome = world.step(TickInput::new(100)).unwrap();
                records.push((
                    motion_tick_record(&world, &outcome),
                    conflict_workspace_record(&mut world),
                ));
            }
            records
        };
        let reference = run(1);
        for workers in [2_u32, 4, 8, 16] {
            assert_eq!(
                run(workers),
                reference,
                "workers={workers} 停车场景必须与融合参考一致"
            );
        }
    }

    /// F3b/F4 真实预留失败：downstream 工作区（F4）与 candidate_downstream
    /// 池（F3b）注入失败都映射 `ConflictScratchAllocFailed`，且先于同车更晚
    /// 领域错误；失败拍状态不变。
    #[test]
    fn downstream_reserve_failures_map_to_scratch_alloc_failed() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let _force = force_conflict_dispatch();
        let revision = conflict_scale_revision();
        for arm in 0..2 {
            let mut world = conflict_scale_world(Arc::clone(&revision), 16);
            install_execution(&mut world, 4);
            let before = world.capture_snapshot().unwrap();
            let guard = if arm == 0 {
                fail_conflict_downstream_work_reserve()
            } else {
                fail_conflict_downstream_pool_reserve()
            };
            let result = world.step(TickInput::new(4));
            drop(guard);
            assert_eq!(
                result,
                Err(crate::StepError::ConflictScratchAllocFailed),
                "arm={arm}"
            );
            assert_eq!(world.capture_snapshot().unwrap(), before);
            world.step(TickInput::new(4)).unwrap();
        }
    }

    /// 工作计数口径：w1 融合与 w4 强制分发的 ConflictWorkCounts 逐字段
    /// 一致（任务线程增量经块级记录汇总；诊断开启只改记录，不改语义）。
    /// conflict_scale 的 P3 段内产出：cells 循环 visited_passages 与
    /// yield target 求值。
    #[test]
    fn conflict_counters_match_between_fused_and_dispatched() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let _force = force_conflict_dispatch();
        let _diagnostics = enable_conflict_diagnostics();
        let revision = conflict_scale_revision();
        let run = |workers: u32| {
            let mut world = conflict_scale_world(Arc::clone(&revision), 16);
            install_execution(&mut world, workers);
            let before = conflict_work_counts();
            for _ in 0..6 {
                world.step(TickInput::new(4)).unwrap();
            }
            let after = conflict_work_counts();
            after.wrapping_sub(before)
        };
        let fused = run(1);
        let dispatched = run(4);
        assert_eq!(
            dispatched, fused,
            "分发路径计数必须经块级记录汇总后与融合一致: fused={fused:?} dispatched={dispatched:?}"
        );
        assert!(
            fused.visited_passages > 0 && fused.yield_queries > 0,
            "场景必须覆盖 P3 段内计数: {fused:?}"
        );
    }

    /// E2 生命周期组合（#705 parallel_preview_equivalence 模式扩展到 P3/P5
    /// 分发）：Parked 夹入 live 序列、Completed 保留中部、同槽位新代次
    /// respawn——三个生命周期场景下 P2/P5 真实分发，P3 筛为空，
    /// 逐拍公开记录与工作区语义池同 w1 全融合参照一致。
    /// P3 非空生命周期身份由 gate_scope 测试覆盖。
    #[test]
    fn mixed_lifecycle_empty_conflict_workset_matches_fused_reference() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        use crate::kernel::tick::{MotionPathCounts, force_motion_dispatch, motion_path_counts};
        use crate::kernel::waiting::{
            WaitingPreviewPathCounts, force_preview_dispatch, preview_path_counts,
        };
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        const WORLD_ID: u64 = 706_310;
        const PROFILE: VehicleProfileOrdinal = VehicleProfileOrdinal::from_raw(0);
        let _force_preview = force_preview_dispatch();
        let _force_conflict = force_conflict_dispatch();
        let _force_motion = force_motion_dispatch();

        let step_pair = |reference: &mut TrafficWorld, parallel: &mut TrafficWorld| {
            let preview_before = preview_path_counts();
            let conflict_before = conflict_path_counts();
            let motion_before = motion_path_counts();
            let parallel_outcome = parallel.step(TickInput::new(100)).unwrap();
            let reference_outcome = reference.step(TickInput::new(100)).unwrap();
            assert_eq!(
                (
                    motion_tick_record(parallel, &parallel_outcome),
                    conflict_workspace_record(parallel)
                ),
                (
                    motion_tick_record(reference, &reference_outcome),
                    conflict_workspace_record(reference)
                ),
                "混合生命周期全分发必须等于全融合参照"
            );
            let preview_delta = WaitingPreviewPathCounts {
                dispatched: preview_path_counts().dispatched - preview_before.dispatched,
                fused: preview_path_counts().fused - preview_before.fused,
                slot_fallback: preview_path_counts().slot_fallback - preview_before.slot_fallback,
            };
            let conflict_delta = ConflictPathCounts {
                dispatched: conflict_path_counts().dispatched - conflict_before.dispatched,
                fused: conflict_path_counts().fused - conflict_before.fused,
                slot_fallback: conflict_path_counts().slot_fallback - conflict_before.slot_fallback,
            };
            let motion_delta = MotionPathCounts {
                dispatched: motion_path_counts().dispatched - motion_before.dispatched,
                fused: motion_path_counts().fused - motion_before.fused,
                slot_fallback: motion_path_counts().slot_fallback - motion_before.slot_fallback,
            };
            assert_eq!(
                (preview_delta.dispatched, preview_delta.slot_fallback),
                (1, 0),
                "P2 必须真实分发"
            );
            assert_eq!(
                (
                    conflict_delta.dispatched,
                    conflict_delta.fused,
                    conflict_delta.slot_fallback
                ),
                (0, 2, 0),
                "本夹具无可达 Gate，P3 工作集为空"
            );
            assert_eq!(
                (motion_delta.dispatched, motion_delta.slot_fallback),
                (1, 0),
                "P5 必须真实分发"
            );
        };

        // 场景 A：Active→Parked 转换拍后连续步进（Parked 夹入 live 序列，
        // Active 紧凑位 ≠ 逻辑 update_sequence）。
        let (mut reference, route, space, entry_progress) = parking_route_world(1, WORLD_ID);
        let (mut parallel, _, _, _) = parking_route_world(4, WORLD_ID);
        let target = crate::ParkingTarget::ExplicitSpace(space);
        let reserve = crate::ReserveParkingTarget::ExplicitSpace {
            space,
            entry_route_occurrence: 0,
        };
        for world in [&mut reference, &mut parallel] {
            world
                .spawn_vehicle(
                    VehicleSpawnInput::new(PROFILE, route, 0, 12_000, 0).with_open_entrance(),
                )
                .unwrap();
            world
                .spawn_vehicle(
                    VehicleSpawnInput::new(PROFILE, route, 0, entry_progress, 0)
                        .with_open_entrance(),
                )
                .unwrap();
            for index in 0..8_u32 {
                world
                    .spawn_vehicle(
                        VehicleSpawnInput::new(PROFILE, route, 0, 20_000 + index * 8_000, 0)
                            .with_open_entrance(),
                    )
                    .unwrap();
            }
            let parker = world.live_vehicles()[1];
            world.reserve_parking(parker, reserve).expect("reserve");
        }
        let parker = reference.live_vehicles()[1];
        reference.park_vehicle(parker, target).expect("park");
        parallel.park_vehicle(parker, target).expect("park");
        for _ in 0..6 {
            step_pair(&mut reference, &mut parallel);
        }

        // 场景 B：Completed 产生拍（近终点车）+ 同槽位新代次 spawn 拍。
        let (mut reference, route, _, _) = parking_route_world(1, WORLD_ID);
        let (mut parallel, _, _, _) = parking_route_world(4, WORLD_ID);
        let edges = reference.route_edges(route).unwrap().to_vec();
        let speed_limit = reference
            .traffic()
            .lane_speed_limits_millimetres_per_second()[edges[0].index()];
        let exit_length = reference.traffic().lane_lengths_millimetres()[edges[1].index()];
        for world in [&mut reference, &mut parallel] {
            for index in 0..5_u32 {
                world
                    .spawn_vehicle(
                        VehicleSpawnInput::new(PROFILE, route, 0, 12_000 + index * 8_000, 0)
                            .with_open_entrance(),
                    )
                    .unwrap();
            }
            world
                .state
                .place_existing_active_vehicle(
                    VehicleSpawnInput::new(PROFILE, route, 1, exit_length - 500, speed_limit)
                        .with_open_entrance(),
                )
                .unwrap();
            for index in 5..10_u32 {
                world
                    .spawn_vehicle(
                        VehicleSpawnInput::new(PROFILE, route, 0, 12_000 + index * 8_000, 0)
                            .with_open_entrance(),
                    )
                    .unwrap();
            }
        }
        let finisher = reference.live_vehicles()[5];
        let mut completed = false;
        for _ in 0..6 {
            step_pair(&mut reference, &mut parallel);
            completed = completed
                || reference
                    .vehicle(finisher)
                    .is_some_and(|state| state.status() == crate::VehicleStatus::Completed);
            if completed {
                break;
            }
        }
        assert!(completed, "近终点车必须在脚本内到达路线终点");
        step_pair(&mut reference, &mut parallel);
        let stale = reference.live_vehicles()[3];
        let respawn = VehicleSpawnInput::new(PROFILE, route, 0, 0, 0).with_open_entrance();
        for world in [&mut reference, &mut parallel] {
            world.despawn_vehicle(stale).expect("despawn");
        }
        let reference_new = reference.spawn_vehicle(respawn).unwrap();
        let parallel_new = parallel.spawn_vehicle(respawn).unwrap();
        assert_eq!(reference_new, parallel_new);
        assert_eq!(reference_new.index(), stale.index(), "同槽位复用");
        assert_ne!(reference_new, stale, "新代次");
        for _ in 0..3 {
            step_pair(&mut reference, &mut parallel);
        }
    }

    /// R1：消费侧工作缓冲不跨候选/跨拍累积——预热后容量有界不随拍数
    /// 增长（旧实现 reserve 把已有 len 计入需求 → 每拍额外增长）。
    #[test]
    fn conflict_scratch_buffers_do_not_accumulate_across_ticks() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let _force = force_conflict_dispatch();
        let revision = conflict_scale_revision();
        let mut world = conflict_scale_world(Arc::clone(&revision), 16);
        install_execution(&mut world, 4);
        let mut capacities = Vec::new();
        for _ in 0..6 {
            world.step(TickInput::new(4)).unwrap();
            let step = world.state.step_workspace();
            capacities.push((
                step.workspace.conflict_cell_work.capacity(),
                step.workspace.conflict_downstream_work.capacity(),
                step.workspace.conflict_cell_work.len(),
                step.workspace.conflict_downstream_work.len(),
            ));
        }
        assert!(
            capacities[2..].iter().all(|&entry| entry == capacities[2]),
            "预热后工作缓冲容量必须稳定（不随拍数增长）: {capacities:?}"
        );
        assert!(
            capacities.iter().all(|&(_, _, cells, downstream)| {
                // 每拍至多一个候选：cell 工作区长度上界为 passage 数（本夹具 1），
                // downstream 工作区长度上界为 claims 数（小常数）——绝不累积。
                cells <= 4 && downstream <= 8
            }),
            "工作缓冲长度只属当前候选: {capacities:?}"
        );
    }

    /// R2+R4：F4 预留注入仅在真实必要增长时触发，且融合与分发两条路径
    /// 在同一逻辑检查点对拍——冷态（需增长）+ 武装 →
    /// ConflictScratchAllocFailed；热态（余量足够）+ 已武装 → 不得制造
    /// 错误；清注入同 tick 重试 == fresh。
    #[test]
    fn f4_reserve_injection_gated_by_real_growth_on_both_paths() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let _force = force_conflict_dispatch();
        let revision = conflict_scale_revision();
        for workers in [1_u32, 4] {
            let mut fresh = conflict_scale_world(Arc::clone(&revision), 16);
            install_execution(&mut fresh, workers);
            let fresh_outcome = fresh.step(TickInput::new(4)).unwrap();
            let fresh_snapshot = fresh.capture_snapshot().unwrap();

            // 冷态 + 武装：F4 真实增长失败（F4 义务先于 F4 后检查公开）。
            let mut world = conflict_scale_world(Arc::clone(&revision), 16);
            install_execution(&mut world, workers);
            let before = world.capture_snapshot().unwrap();
            let guard = fail_conflict_downstream_work_reserve();
            let result = world.step(TickInput::new(4));
            drop(guard);
            assert_eq!(
                result,
                Err(crate::StepError::ConflictScratchAllocFailed),
                "workers={workers} 冷态 F4 需真实增长，武装注入必须先于 F4 后检查"
            );
            assert_eq!(world.capture_snapshot().unwrap(), before);
            let retry = world.step(TickInput::new(4)).unwrap();
            assert_eq!(retry, fresh_outcome, "workers={workers} 重试必须等于 fresh");
            assert_eq!(world.capture_snapshot().unwrap(), fresh_snapshot);

            // 热态 + 已武装：余量足够不得伪造预留失败（W4：探针先清空；
            // 候选周期内 F4 稀疏，持武装步进 12 拍全程不得公开错误，
            // 且探针 fired 计数必须为零（余量足够+已武装 ⇒ 永不伪造失败）。
            crate::kernel::conflict_tick::reset_conflict_reserve_probe_log();
            let guard = fail_conflict_downstream_work_reserve();
            for _ in 0..12 {
                world.step(TickInput::new(4)).unwrap();
            }
            drop(guard);
            let log = crate::kernel::conflict_tick::conflict_reserve_probe_log();
            assert_eq!(
                log.fired, 0,
                "余量足够+已武装：任何 F 位都不得触发注入: {log:?}"
            );
        }
    }

    /// R4+W4：预留探针记录逻辑检查点的可达性、真实需求与余量。每个
    /// case 测前清空日志；按 last + 各点位命中数双重断言：普通拍全点位
    /// 按序到达且 injected=false；武装 F1 的冷态拍失败后 CellWork 之后
    /// 的点位本次命中数为零（不可达）。
    #[test]
    fn reserve_probe_records_site_reachability_and_growth() {
        use crate::kernel::conflict_tick::{
            ConflictReserveSite, conflict_reserve_probe_log, fail_conflict_cell_work_reserve,
            reset_conflict_reserve_probe_log,
        };
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let _force = force_conflict_dispatch();
        let revision = conflict_scale_revision();

        let mut world = conflict_scale_world(Arc::clone(&revision), 16);
        install_execution(&mut world, 4);
        reset_conflict_reserve_probe_log();
        world.step(TickInput::new(4)).unwrap();
        let log = conflict_reserve_probe_log();
        let probe = log.last.expect("F 位探针");
        assert_eq!(probe.site, ConflictReserveSite::DownstreamPool);
        assert!(!probe.injected, "普通拍不得触发注入: {probe:?}");
        assert!(probe.required >= 1, "探针须记录真实需求（≥1）: {probe:?}");
        assert!(
            log.hits.iter().all(|&hits| hits >= 1),
            "普通拍四个 F 位必须全部到达: {log:?}"
        );

        let mut world = conflict_scale_world(Arc::clone(&revision), 16);
        install_execution(&mut world, 4);
        reset_conflict_reserve_probe_log();
        let guard = fail_conflict_cell_work_reserve();
        let result = world.step(TickInput::new(4));
        drop(guard);
        assert_eq!(result, Err(crate::StepError::ConflictScratchAllocFailed));
        let log = conflict_reserve_probe_log();
        let probe = log.last.expect("F 位探针");
        assert_eq!(
            probe.site,
            ConflictReserveSite::CellWork,
            "F1 失败后探针停在 CellWork"
        );
        assert!(probe.injected, "冷态 F1 真实增长 + 武装必须触发: {probe:?}");
        assert_eq!(
            log.hits[1..],
            [0, 0, 0],
            "F1 失败后 F2/F4/F3b 本次访问次数必须为零（不可达）: {log:?}"
        );
    }

    /// R4（P5）：到达预留注入仅真实必要增长时触发——到达拍（观察 Vec 空、
    /// 首次增长）+ 武装 → 先公开 ParkingObservationAllocFailed，清注入重试
    /// 与 fresh 一致；其后的无到达拍 + 已武装 → 余量足够/未达检查点，不得
    /// 制造错误。（夹具入口前空间只容一车排队，同拍多到达用每拍观察 Vec
    /// 重新开始的连续到达拍覆盖。）
    #[test]
    fn parking_arrival_injection_requires_growth_per_step() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        use crate::kernel::tick::{fail_motion_arrival_reserve, motion_path_counts};
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        const WORLD_ID: u64 = 706_320;
        let _force = force_motion_dispatch();
        let counts_before = motion_path_counts();

        let (mut fresh, route, space, entry_progress) = parking_route_world(4, WORLD_ID);
        let reserve = crate::ReserveParkingTarget::ExplicitSpace {
            space,
            entry_route_occurrence: 0,
        };
        let a = fresh
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    entry_progress - 1,
                    10_000,
                )
                .with_open_entrance(),
            )
            .unwrap();
        fresh.reserve_parking(a, reserve).unwrap();
        let fresh_outcome = fresh.step(TickInput::new(100)).unwrap();
        assert_eq!(fresh_outcome.parking_arrivals().len(), 1, "到达拍兑现");
        let fresh_snapshot = fresh.capture_snapshot().unwrap();

        let (mut world, route, _, entry_progress) = parking_route_world(4, WORLD_ID);
        let a = world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    entry_progress - 1,
                    10_000,
                )
                .with_open_entrance(),
            )
            .unwrap();
        world.reserve_parking(a, reserve).unwrap();
        let guard = fail_motion_arrival_reserve();
        let result = world.step(TickInput::new(100));
        drop(guard);
        assert_eq!(
            result,
            Err(crate::StepError::ParkingObservationAllocFailed),
            "到达拍观察 Vec 首次增长 + 武装必须先公开到达预留失败"
        );
        let retry = world.step(TickInput::new(100)).unwrap();
        assert_eq!(retry, fresh_outcome, "清注入重试必须等于 fresh 首拍");
        assert_eq!(world.capture_snapshot().unwrap(), fresh_snapshot);
        // 无到达拍 + 已武装：不得制造错误。
        let guard = fail_motion_arrival_reserve();
        world.step(TickInput::new(100)).unwrap();
        world.step(TickInput::new(100)).unwrap();
        drop(guard);
        let counts = motion_path_counts();
        assert!(
            counts.dispatched - counts_before.dispatched >= 5,
            "全程真实分发: {counts:?}"
        );
    }

    /// E1 组合矩阵（审阅者 §8.3）：P2/P3/P5 七个分发/融合组合 + 全融合
    /// 参照，multi-gate 16 车 w4、每臂 4 拍。被要求分发的相位以各自路径
    /// 计数确认真实分发（与 fused/slot_fallback 互斥），融合相位计 fused；
    /// 全部组合的公开记录 + 工作区语义池逐拍一致（分发不改变语义），且
    /// 失败注入语义在全分发组合下不回归（矩阵外由 D4/首错测试覆盖）。
    #[test]
    fn phase_combination_matrix_matches_across_all_arms() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        const WORLD_ID: u64 = 706_300;
        use crate::kernel::tick::{
            MotionPathCounts, force_motion_dispatch, force_motion_fuse, motion_path_counts,
        };
        use crate::kernel::waiting::{
            WaitingPreviewPathCounts, force_preview_dispatch, force_preview_fuse,
            preview_path_counts,
        };
        // (P2, P3, P5)：true = 强制分发，false = 强制融合（fuse 优先）。
        let arms: [(bool, bool, bool); 8] = [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (false, false, true),
            (true, true, false),
            (true, false, true),
            (false, true, true),
            (true, true, true),
        ];
        let mut reference: Option<Vec<(String, String)>> = None;
        for &(p2, p3, p5) in &arms {
            let mut world = multi_gate_world_with_id(16, WORLD_ID);
            install_execution(&mut world, 4);
            let _p2_dispatch = p2.then(force_preview_dispatch);
            let _p2_fuse = (!p2).then(force_preview_fuse);
            let _p3_dispatch = p3.then(force_conflict_dispatch);
            let _p3_fuse = (!p3).then(force_conflict_fuse);
            let _p5_dispatch = p5.then(force_motion_dispatch);
            let _p5_fuse = (!p5).then(force_motion_fuse);
            let preview_arm_before = preview_path_counts();
            let conflict_arm_before = conflict_path_counts();
            let motion_arm_before = motion_path_counts();
            let mut records = Vec::new();
            for _ in 0..4 {
                let outcome = world.step(TickInput::new(100)).unwrap();
                records.push((
                    motion_tick_record(&world, &outcome),
                    conflict_workspace_record(&mut world),
                ));
            }
            if let Some(reference) = &reference {
                assert_eq!(
                    &records, reference,
                    "arm=({p2},{p3},{p5}) 必须与全融合参照逐拍一致"
                );
            } else {
                reference = Some(records);
            }
            let preview_delta = WaitingPreviewPathCounts {
                dispatched: preview_path_counts().dispatched - preview_arm_before.dispatched,
                fused: preview_path_counts().fused - preview_arm_before.fused,
                slot_fallback: preview_path_counts().slot_fallback
                    - preview_arm_before.slot_fallback,
            };
            let conflict_delta = ConflictPathCounts {
                dispatched: conflict_path_counts().dispatched - conflict_arm_before.dispatched,
                fused: conflict_path_counts().fused - conflict_arm_before.fused,
                slot_fallback: conflict_path_counts().slot_fallback
                    - conflict_arm_before.slot_fallback,
            };
            let motion_delta = MotionPathCounts {
                dispatched: motion_path_counts().dispatched - motion_arm_before.dispatched,
                fused: motion_path_counts().fused - motion_arm_before.fused,
                slot_fallback: motion_path_counts().slot_fallback - motion_arm_before.slot_fallback,
            };
            let expect = |on: bool| {
                if on { (4, 0, 0) } else { (0, 4, 0) }
            };
            let (pd, pf, pb) = expect(p2);
            assert_eq!(
                (
                    preview_delta.dispatched,
                    preview_delta.fused,
                    preview_delta.slot_fallback
                ),
                (pd, pf, pb),
                "P2 arm=({p2},{p3},{p5}) 路径计数"
            );
            // P3 首拍近门分发，其余三拍无未来 Gate。
            let (cd, cf, cb) = if p3 { (1, 3, 0) } else { (0, 4, 0) };
            assert_eq!(
                (
                    conflict_delta.dispatched,
                    conflict_delta.fused,
                    conflict_delta.slot_fallback
                ),
                (cd, cf, cb),
                "P3 arm=({p2},{p3},{p5}) 路径计数"
            );
            let (md, mf, mb) = expect(p5);
            assert_eq!(
                (
                    motion_delta.dispatched,
                    motion_delta.fused,
                    motion_delta.slot_fallback
                ),
                (md, mf, mb),
                "P5 arm=({p2},{p3},{p5}) 路径计数"
            );
        }
    }

    /// P2 计算 panic 端到端：panic 不按 `StepError` 映射；世界永久失效，
    /// 交通步进、快照与管理/配置查询全部拒绝；注入与断言沿用 execution.rs
    /// panic 测试的模式（#705 验收：执行器异常）。
    #[test]
    fn preview_panic_invalidates_world_and_rejects_queries() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        use std::panic::{AssertUnwindSafe, catch_unwind};
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let _force = super::force_preview_dispatch();
        const WORLD_ID: u64 = 705_600;
        let mut world = multi_gate_world_with_id(16, WORLD_ID);
        install_execution(&mut world, 4);
        // 位置 15 位于调用线程首块之外的末尾块：panic 发生在池线程上，
        // 执行器须先完整 join 再向世界宿主传播。
        let guard = inject_preview_panic(WORLD_ID, 15);
        let result = catch_unwind(AssertUnwindSafe(|| world.step(TickInput::new(100))));
        drop(guard);
        assert!(result.is_err());
        assert!(catch_unwind(AssertUnwindSafe(|| world.execution.assert_usable())).is_err());
        assert!(catch_unwind(AssertUnwindSafe(|| world.step(TickInput::new(100)))).is_err());
        assert!(catch_unwind(AssertUnwindSafe(|| world.capture_snapshot())).is_err());
        assert!(catch_unwind(AssertUnwindSafe(|| world.world_binding())).is_err());
        assert!(catch_unwind(AssertUnwindSafe(|| world.execution_config())).is_err());
    }

    const PARKING_ONLY: &[u8] = include_bytes!(
        "../../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/oracle-base.lfca"
    );

    /// 停车夹具 + 一条「入口边（含泊位）→出口边」路线；返回世界、路线、泊位
    /// 序号与泊位入口在入口边上的进度。worker 数在安装时指定。
    fn parking_route_world(
        workers: u32,
        world_id: u64,
    ) -> (
        TrafficWorld,
        RouteHandle,
        laneflow_static_contract::ParkingSpaceOrdinal,
        u32,
    ) {
        let input =
            check_canonical_network_input(PARKING_ONLY, FormatLimits::HARD).expect("checked");
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::Omit,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("revision");
        let origin = *revision.canonical_origin();
        let mut world = TrafficWorld::install(
            Arc::clone(&revision),
            WorldConfig::new(12, 4, 1_024, 1_024, 100),
            exec_config(workers),
            CommittedNetworkSource::Published {
                reference: PublishedLfcaReference::new(
                    "fixture://dispatch-confirm-parking",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .unwrap(),
            },
            world_id,
            crate::test_policy::selection(&revision),
        )
        .unwrap();
        let space = laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0);
        let (entry_edge, entry_progress) = world
            .traffic()
            .relations()
            .parking_space(space)
            .expect("parking space")
            .entry();
        let exit_edge = world
            .traffic()
            .successors(entry_edge)
            .and_then(|successors| successors.first())
            .copied()
            .expect("successor");
        let route = world
            .register_route(RouteRegisterInput::new(vec![entry_edge, exit_edge]))
            .unwrap();
        (world, route, space, entry_progress)
    }

    fn active_vehicle_count(world: &TrafficWorld) -> usize {
        world
            .live_vehicles()
            .iter()
            .filter(|vehicle| {
                world
                    .vehicle(**vehicle)
                    .is_some_and(|state| state.status() == crate::VehicleStatus::Active)
            })
            .count()
    }

    /// Active 投影紧凑位置与逻辑 live 序错位的结构证据：live 序列中夹有非
    /// Active 成员时两者长度不同。
    fn assert_active_projection_is_mixed(world: &TrafficWorld) {
        assert!(
            world.state.derived.active_order.len() < world.state.committed.live_order.len(),
            "live 序列必须夹有非 Active 成员（Active 紧凑位置 ≠ 逻辑 update_sequence）"
        );
    }

    /// 步进一对世界并分别归因路径计数：两世界该拍 Active 必须 ≥ 8（分发阈值
    /// 之上），多 worker 世界必须走真实分发（dispatched +1、融合/回退 +0），
    /// worker=1 参考必须走融合；随后验证两世界公开输出逐字节一致。
    fn step_pair_dispatched(reference: &mut TrafficWorld, parallel: &mut TrafficWorld) {
        assert!(
            active_vehicle_count(reference) >= 8 && active_vehicle_count(parallel) >= 8,
            "关键拍必须保持 Active ≥ 8（分发阈值之上）"
        );
        let counts_before = preview_path_counts();
        parallel.step(TickInput::new(100)).unwrap();
        let counts_parallel = preview_path_counts();
        reference.step(TickInput::new(100)).unwrap();
        let counts_reference = preview_path_counts();
        let parallel_delta = WaitingPreviewPathCounts {
            dispatched: counts_parallel.dispatched - counts_before.dispatched,
            fused: counts_parallel.fused - counts_before.fused,
            slot_fallback: counts_parallel.slot_fallback - counts_before.slot_fallback,
        };
        assert_eq!(
            parallel_delta,
            WaitingPreviewPathCounts {
                dispatched: 1,
                fused: 0,
                slot_fallback: 0,
            },
            "多 worker 世界在该拍必须走真实分发，而非融合或回退"
        );
        let reference_delta = WaitingPreviewPathCounts {
            dispatched: counts_reference.dispatched - counts_parallel.dispatched,
            fused: counts_reference.fused - counts_parallel.fused,
            slot_fallback: counts_reference.slot_fallback - counts_parallel.slot_fallback,
        };
        assert_eq!(
            reference_delta,
            WaitingPreviewPathCounts {
                dispatched: 0,
                fused: 1,
                slot_fallback: 0,
            },
            "worker=1 参考世界必须走融合路径"
        );
        assert_public_outputs_match(parallel, reference);
    }

    /// 混合生命周期下的分发内部确认：live 序列夹有 Parked/Completed 成员
    /// （Active 紧凑位置 ≠ 逻辑 update_sequence）时，路径计数必须证明多
    /// worker 世界走真实分发而非融合/回退，且与 worker=1 参考世界逐步公开
    /// 输出一致。覆盖 Active→Parked 转换拍、Completed 产生拍、同槽位新代次
    /// spawn 拍（#705 审阅缺陷 4 的内部确认半边）。生产分发阈值为保守
    /// 1_024，小场景经强制入口分发。
    #[test]
    fn mixed_lifecycle_ticks_take_real_dispatch() {
        use crate::kernel::execution::RESOURCE_TEST_LOCK;
        let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
        let _force = super::force_preview_dispatch();
        const WORLD_ID: u64 = 705_700;
        const PROFILE: VehicleProfileOrdinal = VehicleProfileOrdinal::from_raw(0);

        // 场景 A：Active→Parked 转换拍。跟车间距 8 m，停车者被跟随者夹在
        // live 序列中间；park 命令后它保持 live 但离开 Active 投影。
        let (mut reference, route, space, entry_progress) = parking_route_world(1, WORLD_ID);
        let (mut parallel, _, _, _) = parking_route_world(4, WORLD_ID);
        let target = crate::ParkingTarget::ExplicitSpace(space);
        let reserve = crate::ReserveParkingTarget::ExplicitSpace {
            space,
            entry_route_occurrence: 0,
        };
        for world in [&mut reference, &mut parallel] {
            world
                .spawn_vehicle(
                    VehicleSpawnInput::new(PROFILE, route, 0, 12_000, 0).with_open_entrance(),
                )
                .unwrap();
            world
                .spawn_vehicle(
                    VehicleSpawnInput::new(PROFILE, route, 0, entry_progress, 0)
                        .with_open_entrance(),
                )
                .unwrap();
            for index in 0..8_u32 {
                world
                    .spawn_vehicle(
                        VehicleSpawnInput::new(PROFILE, route, 0, 20_000 + index * 8_000, 0)
                            .with_open_entrance(),
                    )
                    .unwrap();
            }
            let parker = world.live_vehicles()[1];
            world.reserve_parking(parker, reserve).expect("reserve");
            assert!(world.parking_arrived(parker, target));
        }
        let parker = reference.live_vehicles()[1];
        assert_eq!(parker, parallel.live_vehicles()[1]);
        assert!(active_vehicle_count(&reference) >= 8);
        assert!(active_vehicle_count(&parallel) >= 8);
        // 转换拍：park 命令立即生效，此后每拍的 live 序列中都夹有 Parked 成员，
        // 而 Active 数保持 ≥ 8。
        reference.park_vehicle(parker, target).expect("park");
        parallel.park_vehicle(parker, target).expect("park");
        for world in [&reference, &parallel] {
            assert_eq!(
                world.vehicle(parker).expect("parker").status(),
                crate::VehicleStatus::Parked
            );
            assert!(active_vehicle_count(world) >= 8);
            assert_active_projection_is_mixed(world);
        }
        for _ in 0..6 {
            step_pair_dispatched(&mut reference, &mut parallel);
        }

        // 场景 B/C：Completed 产生拍与同槽位新代次 spawn 拍。近终点车在序列
        // 中部先 Completed 并保留在 live 序中；随后 despawn 一辆中部跟随者并
        // 在同位置重新 spawn，新句柄同槽位、新代次、立即 Active。
        let (mut reference, route, _, _) = parking_route_world(1, WORLD_ID);
        let (mut parallel, _, _, _) = parking_route_world(4, WORLD_ID);
        let edges = reference.route_edges(route).unwrap().to_vec();
        let speed_limit = reference
            .traffic()
            .lane_speed_limits_millimetres_per_second()[edges[0].index()];
        let exit_length = reference.traffic().lane_lengths_millimetres()[edges[1].index()];
        for world in [&mut reference, &mut parallel] {
            for index in 0..5_u32 {
                world
                    .spawn_vehicle(
                        VehicleSpawnInput::new(PROFILE, route, 0, 12_000 + index * 8_000, 0)
                            .with_open_entrance(),
                    )
                    .unwrap();
            }
            world
                .state
                .place_existing_active_vehicle(
                    VehicleSpawnInput::new(PROFILE, route, 1, exit_length - 500, speed_limit)
                        .with_open_entrance(),
                )
                .unwrap();
            for index in 5..10_u32 {
                world
                    .spawn_vehicle(
                        VehicleSpawnInput::new(PROFILE, route, 0, 12_000 + index * 8_000, 0)
                            .with_open_entrance(),
                    )
                    .unwrap();
            }
        }
        let finisher = reference.live_vehicles()[5];
        assert_eq!(finisher, parallel.live_vehicles()[5]);
        let mut completed = false;
        for _ in 0..6 {
            step_pair_dispatched(&mut reference, &mut parallel);
            completed = completed
                || reference
                    .vehicle(finisher)
                    .is_some_and(|state| state.status() == crate::VehicleStatus::Completed);
            if completed {
                break;
            }
        }
        assert!(completed, "近终点车必须在脚本内到达路线终点");
        // Completed 产生拍之后：保留在 live 序列中部，Active 投影错位。
        for world in [&reference, &parallel] {
            assert_eq!(world.live_vehicles()[5], finisher);
            assert_eq!(
                world.vehicle(finisher).expect("finisher").status(),
                crate::VehicleStatus::Completed
            );
            assert!(active_vehicle_count(world) >= 8);
            assert_active_projection_is_mixed(world);
        }
        step_pair_dispatched(&mut reference, &mut parallel);
        // 同槽位新代次 spawn 拍：despawn 中部跟随者后同位置重新 spawn。
        let stale = reference.live_vehicles()[3];
        assert_eq!(stale, parallel.live_vehicles()[3]);
        let respawn = VehicleSpawnInput::new(PROFILE, route, 0, 0, 0).with_open_entrance();
        for world in [&mut reference, &mut parallel] {
            world.despawn_vehicle(stale).expect("despawn");
        }
        let reference_new = reference.spawn_vehicle(respawn).unwrap();
        let parallel_new = parallel.spawn_vehicle(respawn).unwrap();
        assert_eq!(reference_new, parallel_new);
        assert_eq!(reference_new.index(), stale.index(), "同槽位复用");
        assert_ne!(reference_new, stale, "新代次");
        for world in [&reference, &parallel] {
            assert_eq!(
                world.vehicle(reference_new).expect("respawned").status(),
                crate::VehicleStatus::Active
            );
            assert!(active_vehicle_count(world) >= 8);
            assert_active_projection_is_mixed(world);
        }
        for _ in 0..3 {
            step_pair_dispatched(&mut reference, &mut parallel);
        }
    }
}
