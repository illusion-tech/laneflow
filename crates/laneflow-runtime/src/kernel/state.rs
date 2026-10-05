//! 固定步进与管理操作共享的五类私有状态所有者。
//!
//! 状态归属见 `traffic-runtime-phase-protocol.md`；这些类型不改变公开 facade。

use crate::kernel::conflict::{ApproachEstimate, ApproachFrontierCell};
use crate::kernel::occupancy::OccupancyIndex;
use crate::kernel::parking::ParkingRuntimeState;
use crate::kernel::tables::RouteSlot;
use crate::kernel::waiting::{
    WaitingAdmissionClaim, WaitingQueueEnds, WaitingQueueLink, WaitingVehiclePlan, WaitingZoneState,
};
use crate::{
    CommittedNetworkSource, ObservationStateSequence, VehicleHandle, VehicleState, WorldConfig,
    WorldGeneration,
};
use laneflow_static_contract::SignalAspect;
use laneflow_static_network::SharedNetworkRevision;
use std::sync::Arc;

/// 已提交 Conflict 资格表。每次可变借用都换一个进程内唯一的版本号：步进交换
/// 暂存表时据此判断上拍记下的 Some 位置清单是否仍覆盖这张表（宿主命令、恢复
/// 与切换都可能改写它），覆盖时只清这些位置，否则整表清空。
#[derive(Clone, Debug, Default)]
pub(crate) struct EligibilityTable {
    slots: Vec<Option<crate::ConflictEligibilityState>>,
    version: u64,
}

static ELIGIBILITY_TABLE_VERSION: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

fn next_eligibility_table_version() -> u64 {
    ELIGIBILITY_TABLE_VERSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl EligibilityTable {
    /// 当前内容的版本；内容只在版本号更换后才可能改变。
    pub(crate) const fn version(&self) -> u64 {
        self.version
    }

    pub(crate) fn into_vec(self) -> Vec<Option<crate::ConflictEligibilityState>> {
        self.slots
    }
}

impl From<Vec<Option<crate::ConflictEligibilityState>>> for EligibilityTable {
    fn from(slots: Vec<Option<crate::ConflictEligibilityState>>) -> Self {
        Self {
            slots,
            version: next_eligibility_table_version(),
        }
    }
}

impl std::ops::Deref for EligibilityTable {
    type Target = Vec<Option<crate::ConflictEligibilityState>>;
    fn deref(&self) -> &Self::Target {
        &self.slots
    }
}

impl std::ops::DerefMut for EligibilityTable {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.version = next_eligibility_table_version();
        &mut self.slots
    }
}

impl PartialEq for EligibilityTable {
    fn eq(&self, other: &Self) -> bool {
        self.slots == other.slots
    }
}

/// 已提交状态校验账本：完整不变量校验移到入口。记下最近一次已知有效的
/// (世界世代, 观测序号)；其后已提交状态只经由本 runtime 的步进提交与单车命令
/// 改变时，下一拍只核对 `slots` 记下的车位（上拍写过资格的车位与命令改过的
/// 车位）。安装、恢复、切换等其他路径改变世代或序号即令账本对不上，下一拍
/// 照常完整校验。
#[derive(Debug, Default)]
pub(crate) struct CommittedCheck {
    stamp: Option<(WorldGeneration, ObservationStateSequence)>,
    slots: Vec<u32>,
    /// 本拍预检走了账本；P0 的 Waiting 行完整校验随之跳过。
    trusted: bool,
}

/// 两拍之间单车命令可追加的待核对车位余量；用尽时账本失效，下一拍完整校验。
const COMMITTED_CHECK_COMMAND_SLOTS: usize = 4_096;

impl CommittedCheck {
    /// 步进预检：账本对得上时返回 `true`，只需核对 [`Self::slots`]；否则完整校验。
    pub(crate) fn begin_step(
        &mut self,
        generation: WorldGeneration,
        sequence: ObservationStateSequence,
    ) -> bool {
        self.trusted = self.stamp == Some((generation, sequence));
        self.trusted
    }

    /// 账本记下的待核对车位。
    pub(crate) fn slots(&self) -> &[u32] {
        &self.slots
    }

    /// 本拍预检是否走了账本。
    pub(crate) const fn trusted(&self) -> bool {
        self.trusted
    }

    /// 提交前为本拍账本预留容量（上界为 Active 数加命令余量），提交窗口内不再
    /// 分配；预留失败时提交后账本失效。
    pub(crate) fn reserve(&mut self, active: usize) {
        let needed = active.saturating_add(COMMITTED_CHECK_COMMAND_SLOTS);
        let _ = self
            .slots
            .try_reserve(needed.saturating_sub(self.slots.len()));
    }

    /// 本拍准备结束（无论成败）后调用；失败零提交，账本保持不变。
    pub(crate) fn end_step(&mut self) {
        self.trusted = false;
    }

    /// 步进成功提交后以新序号重建账本；`slots` 为本拍写过资格的车位，未知时
    /// 账本失效。
    pub(crate) fn after_step(
        &mut self,
        generation: WorldGeneration,
        sequence: ObservationStateSequence,
        slots: Option<&[u32]>,
    ) {
        self.stamp = None;
        self.trusted = false;
        self.slots.clear();
        let Some(slots) = slots else {
            return;
        };
        // 容量已在提交前预留；不足时不在提交窗口内分配，令账本失效。
        if slots.len().saturating_add(COMMITTED_CHECK_COMMAND_SLOTS) > self.slots.capacity() {
            return;
        }
        self.slots.extend_from_slice(slots);
        self.stamp = Some((generation, sequence));
    }

    /// 单车命令提交：账本仍对得上命令前的序号时追加该车位并跟到新序号。
    pub(crate) fn note_command(
        &mut self,
        generation: WorldGeneration,
        previous: ObservationStateSequence,
        next: ObservationStateSequence,
        slot: usize,
    ) {
        if self.stamp != Some((generation, previous)) {
            return;
        }
        match u32::try_from(slot) {
            Ok(slot) if self.slots.len() < self.slots.capacity() => {
                self.slots.push(slot);
                self.stamp = Some((generation, next));
            }
            _ => self.stamp = None,
        }
    }

    /// 测试用：绕过入口直接改写已提交状态后，令下一拍完整校验。
    #[cfg(test)]
    pub(crate) fn invalidate(&mut self) {
        self.stamp = None;
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> u64 {
        crate::kernel::state::vec_bytes(&self.slots)
    }
}

/// 交通准入与候选迁移共同拥有的数据；不包含执行配置或线程资源。
pub(crate) struct WorldState {
    pub(crate) binding: WorldBindingState,
    pub(crate) committed: CommittedWorldState,
    pub(crate) derived: DerivedIndexes,
    pub(crate) workspace: TickWorkspace,
    pub(crate) admin: crate::admin::state::AdministrativeState,
}

/// 同一活动根、来源、世界身份与配置；步进只读。
pub(crate) struct WorldBindingState {
    pub(crate) revision: Arc<SharedNetworkRevision>,
    pub(crate) source: CommittedNetworkSource,
    /// 宿主指定的世界身份；切换描述符 `worldBinding` 在事务启动时比对。
    pub(crate) world_id: u64,
    /// 活动聚合世代；成功切换/恢复的唯一失效轴。
    pub(crate) world_generation: WorldGeneration,
    pub(crate) config: WorldConfig,
    pub(crate) policy_binding: crate::kernel::policy::WorldPolicyBinding,
}

/// 登记、资源权威、时钟游标及上次成功发布的批次。
pub(crate) struct CommittedWorldState {
    /// 已提交的冲突/下游资源权威。
    pub(crate) conflict: crate::kernel::conflict::ConflictCommittedState,
    /// 车辆槽位对应的 exact Gate occurrence 首次资格时钟。
    pub(crate) conflict_eligibility: EligibilityTable,
    pub(crate) latest_conflict_decisions: Vec<crate::ConflictDecision>,
    pub(crate) tick_index: u64,
    pub(crate) time_ms: u64,
    /// 已应用输入命令计数（快照合同 §3 双游标之一；切换 `worldBinding`
    /// 基线在事务启动时与之逐项比对）。
    pub(crate) command_cursor: u64,
    /// 已提交切换事件游标（#513 切片 C-4）：每次成功切换原子递增一个
    /// 事件批次；事件批次只随晋升恰一次交付。
    pub(crate) event_cursor: u64,
    /// 当前世界世代/观测 stream 内严格单调的已提交状态序号。
    pub(crate) observation_state_sequence: ObservationStateSequence,
    pub(crate) signal_aspects: Box<[SignalAspect]>,
    pub(crate) routes: Vec<RouteSlot>,
    pub(crate) free_routes: Vec<usize>,
    pub(crate) live_route_count: u32,
    pub(crate) live_route_edge_occurrence_count: u64,
    pub(crate) live_route_conflict_occurrence_count: u64,
    pub(crate) vehicles: crate::kernel::vehicle_store::VehicleStore,
    pub(crate) free_vehicles: Vec<usize>,
    pub(crate) live_order: Vec<VehicleHandle>,
    pub(crate) parking: ParkingRuntimeState,
    /// 每个静态 WaitingZone 的稠密本地动态状态。
    pub(crate) waiting_zones: Box<[WaitingZoneState]>,
    /// 刚完成 successful tick 的 latest decision batch。
    pub(crate) latest_waiting_decisions: Vec<crate::WaitingDecision>,
    /// 刚完成 successful tick 的 committed transition event batch。
    pub(crate) latest_transition_events: Vec<crate::TrafficTransitionEvent>,
}

/// 从已提交基线构建的查询索引；不拥有新的交通权威。
pub(crate) struct DerivedIndexes {
    pub(crate) conflict: crate::kernel::conflict::ConflictDerivedIndexes,
    /// 仅含 `Active` 的固定步进执行顺序；按 `live_order` 投影维护，Parked / Completed
    /// 不进入 tick 或 lane occupancy 重建扫描。
    pub(crate) active_order: Vec<VehicleHandle>,
    pub(crate) live_order_index: crate::kernel::active_order::LiveOrderIndex,
    /// 车辆槽位下标对应的 intrusive queue link；长度固定为 `vehicle_capacity`。
    pub(crate) waiting_queue_ends: Box<[WaitingQueueEnds]>,
    pub(crate) waiting_links: Box<[WaitingQueueLink]>,
    /// 只读 member batch，按 `(zone, admission_sequence)` 排列。
    pub(crate) waiting_member_rows: Vec<crate::WaitingZoneMember>,
    pub(crate) occupancy: OccupancyIndex,
    pub(crate) spawn_overlap: crate::kernel::spawn_overlap::SpawnOverlapIndex,
    /// 这一拍也会申请同一冲突区的已有车。生成之间增量维护，步进后按序号作废。
    pub(crate) spawn_contenders: SpawnConflictContenders,
}

/// 已在路上、这一拍会申请该冲突区的一辆车。比较顺序与正式候选键一致，不含车辆身份。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ContenderRank {
    not_protected: u8,
    missing_priority: u8,
    /// 越大越优先，比较时反过来。
    priority: i32,
    first_eligible_tick: u64,
    missing_waiting: u8,
    waiting_sequence: u64,
    update_sequence: u32,
}

/// 某一冲突区里当前最优先的申请者，以及它申请的入口 hop。
#[derive(Clone, Copy, Debug)]
pub(crate) struct ZoneContender {
    pub(crate) rank: ContenderRank,
    pub(crate) vehicle: VehicleHandle,
    pub(crate) hop: u32,
}

/// 争用名单建好时对应的世界世代和观测序号。两个都对上才可以复用。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ContenderBuilt {
    pub(crate) generation: WorldGeneration,
    pub(crate) sequence: ObservationStateSequence,
}

/// 这一拍舒适预览会进入该排队区的已有车。已在区里的成员不记。
#[derive(Clone, Copy, Debug)]
pub(crate) struct WaitingEntrant {
    pub(crate) vehicle: crate::VehicleHandle,
    pub(crate) approach_mm: u32,
    pub(crate) update_sequence: u32,
    pub(crate) length_mm: u32,
    pub(crate) min_gap_mm: u32,
}

/// 一辆车写进争用名单的贡献。撤销时按这里把名次、格点到达和排队记录撤掉再重算。
#[derive(Clone, Debug)]
pub(crate) struct OwnerContribution {
    pub(crate) update_sequence: u32,
    pub(crate) zones: Vec<usize>,
    pub(crate) cells: Vec<(usize, ApproachEstimate)>,
    pub(crate) waiting_zone: Option<usize>,
}

impl ContenderRank {
    pub(crate) const fn new(
        protected: bool,
        priority: Option<i32>,
        first_eligible_tick: u64,
        waiting_sequence: Option<u64>,
        update_sequence: u32,
    ) -> Self {
        Self {
            not_protected: if protected { 0 } else { 1 },
            missing_priority: if priority.is_none() { 1 } else { 0 },
            priority: match priority {
                Some(priority) => priority,
                None => 0,
            },
            first_eligible_tick,
            missing_waiting: if waiting_sequence.is_none() { 1 } else { 0 },
            waiting_sequence: match waiting_sequence {
                Some(sequence) => sequence,
                None => 0,
            },
            update_sequence,
        }
    }

    pub(crate) const fn is_protected(self) -> bool {
        self.not_protected == 0
    }

    pub(crate) const fn update_sequence(self) -> u32 {
        self.update_sequence
    }

    /// 正式调度里排在 `other` 前面。资格更早、已有排队序号的更优先，然后才比更新序号。
    pub(crate) fn sorts_before(self, other: Self) -> bool {
        (
            self.not_protected,
            self.missing_priority,
            core::cmp::Reverse(self.priority),
            self.first_eligible_tick,
            self.missing_waiting,
            self.waiting_sequence,
            self.update_sequence,
        ) < (
            other.not_protected,
            other.missing_priority,
            core::cmp::Reverse(other.priority),
            other.first_eligible_tick,
            other.missing_waiting,
            other.waiting_sequence,
            other.update_sequence,
        )
    }
}

/// 生成停车判断用的已提交接近表。步进不重建。世代或序号对不上就整份作废，下次生成再重建。
#[derive(Debug, Default)]
pub(crate) struct SpawnConflictContenders {
    /// 每个冲突区里这一拍会申请的车，按正式名次排好。不只留最优先的一辆。
    pub(crate) best: Vec<Vec<ZoneContender>>,
    /// 按冲突格点，只留两名最紧急的不同车。排除自己时读另一名。
    pub(crate) cell_approach: Vec<ApproachFrontierCell>,
    /// 按排队区。只含这一拍预览会新进入的车，按接近距离和更新序号排好。
    pub(crate) waiting_entrants: Vec<Vec<WaitingEntrant>>,
    /// 按车辆槽位记下这份名单里的贡献。没有贡献的槽是 `None`。
    pub(crate) owners: Vec<Option<OwnerContribution>>,
    /// `None` 表示名单不能当当前世界使用。建失败或更新不完整都留在这里，不假装已经建好。
    pub(crate) built_for: Option<ContenderBuilt>,
}

impl SpawnConflictContenders {
    pub(crate) fn invalidate(&mut self) {
        self.built_for = None;
    }

    #[cfg(test)]
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        vec_bytes(&self.best)
            + self.best.iter().map(vec_bytes).sum::<u64>()
            + vec_bytes(&self.cell_approach)
            + vec_bytes(&self.waiting_entrants)
            + self.waiting_entrants.iter().map(vec_bytes).sum::<u64>()
            + vec_bytes(&self.owners)
            + self
                .owners
                .iter()
                .filter_map(Option::as_ref)
                .map(|owner| vec_bytes(&owner.zones) + vec_bytes(&owner.cells))
                .sum::<u64>()
    }
}

/// 本拍候选与输出暂存；失败撤销逻辑结果并复用容量。
pub(crate) struct TickWorkspace {
    pub(crate) conflict: crate::kernel::conflict::ConflictWorkspace,
    /// 固定步进 scratch；所有增长走 checked reserve，warm-up 后不再分配。
    pub(crate) conflict_candidates: Vec<crate::kernel::conflict_tick::ConflictCandidate>,
    /// 候选规范排序的紧凑键与原下标，容量与候选相同。
    pub(crate) conflict_candidate_order: Vec<crate::kernel::conflict_tick::CandidateOrderKey>,
    pub(crate) conflict_schedule: crate::kernel::conflict_tick::ConflictSchedule,
    pub(crate) conflict_candidate_cells: Vec<crate::ConflictPassageAddress>,
    pub(crate) conflict_candidate_downstream: Vec<crate::DownstreamInterval>,
    pub(crate) conflict_cell_work: Vec<crate::ConflictPassageAddress>,
    pub(crate) conflict_downstream_work: Vec<crate::DownstreamInterval>,
    pub(crate) conflict_grants: Vec<crate::kernel::conflict_tick::PreparedConflictGrant>,
    pub(crate) conflict_motion_by_vehicle:
        Box<[Option<crate::kernel::conflict_tick::ConflictMotionPlan>]>,
    pub(crate) conflict_next_eligibility: Box<[Option<crate::ConflictEligibilityState>]>,
    /// 两张逐车位暂存表与已提交资格表中可能为 Some 的位置，供下一拍稀疏清空。
    pub(crate) conflict_table_writes: crate::kernel::conflict_tick::ConflictTableWrites,
    /// 已提交状态校验账本（见 [`CommittedCheck`]）。
    pub(crate) committed_check: CommittedCheck,
    pub(crate) conflict_passage_transitions:
        Vec<crate::kernel::conflict_tick::ConflictPassageTransition>,
    /// 本 tick reservation/stage/release 发生变化的稀疏 owner 集；迁移日志据此
    /// 写 authority replacement，避免为在线切换额外扫描车辆容量。
    pub(crate) conflict_changed_owners: Vec<VehicleHandle>,
    pub(crate) waiting_dependencies: crate::kernel::waiting_dependencies::WaitingDependencies,
    pub(crate) conflict_staged_decisions: Vec<crate::ConflictDecision>,
    /// 下一提交时刻的信号暂存；tick 失败时不影响已发布信号。
    pub(crate) next_signal_aspects: Box<[SignalAspect]>,
    /// tick scratch：每车至多一个新 Waiting admission claim。
    pub(crate) waiting_claims: Vec<WaitingAdmissionClaim>,
    pub(crate) waiting_plans: Vec<WaitingVehiclePlan>,
    pub(crate) waiting_plan_by_vehicle: Box<[Option<std::num::NonZeroU32>]>,
    pub(crate) next_state_by_vehicle: Box<[u32]>,
    pub(crate) waiting_staged_decisions: Vec<crate::WaitingDecision>,
    pub(crate) waiting_non_entry_anchors: Vec<crate::kernel::waiting::NonEntryGateAnchor>,
    pub(crate) staged_transition_events: Vec<crate::TrafficTransitionEvent>,
    pub(crate) waiting_next_counters: Box<[u64]>,
    pub(crate) waiting_staged_occupancy: Box<[u32]>,
    pub(crate) waiting_staged_storage_mm: Box<[u64]>,
    pub(crate) occupancy_scratch: crate::kernel::occupancy::OccupancyScratch,
    pub(crate) motion_cache: Vec<crate::kernel::tick::MotionCacheEntry>,
    /// 上一份运动缓存缓冲，保留旧行（内容无效，只供换入后免去整表补占位行）；
    /// 任何读取者都不看它。
    pub(crate) motion_cache_spare: Vec<crate::kernel::tick::MotionCacheEntry>,
    pub(crate) motion_bases: Vec<crate::kernel::tick::MotionBasis>,
    /// `motion_cache` 行引用的完整预览（稀疏，只有近门车辆）。
    pub(crate) motion_previews: Vec<crate::kernel::tick::MotionPreview>,
    pub(crate) waiting_preview_bases: Vec<Vec<crate::kernel::tick::MotionBasis>>,
    /// P2 分发块内暂存的完整预览；槽位只存块内下标，规范消费时移入
    /// `motion_previews`。每块按块长预留，预留失败退回融合求值。
    pub(crate) waiting_preview_payloads: Vec<Vec<crate::kernel::tick::MotionPreview>>,
    pub(crate) next_states: Vec<(usize, super::vehicle_store::MotionValue)>,
    pub(crate) motion_next: super::motion_updates::MotionUpdates,
    pub(crate) motion_kernel: laneflow_motion_kernel::Kernel,
    /// P2 逐车独立计算的输入配对（Active 紧凑位置 -> 完整句柄 + live 序）；
    /// 协调器构建，任务只读。
    pub(crate) waiting_preview_inputs: Vec<(crate::VehicleHandle, usize)>,
    /// P2 独立预览输出槽位，按 Active 紧凑位置索引；任务独占连续切片写入，
    /// 协调器按序消费。预留失败只退回融合求值，不新增领域错误。
    pub(crate) waiting_preview_slots:
        Vec<crate::kernel::execution::DispatchSlot<crate::kernel::tick::WaitingPreviewSlot>>,
    /// #740 近门名单、冲突距离缓存与生命周期增量。已发布名单只在成功提交时替换。
    pub(crate) frontier_maintenance: crate::kernel::entry_frontier::FrontierMaintenance,
    /// P3 候选求值输入四元组（live 序 -> 句柄 + live 序 + Active 紧凑位 +
    /// 拍初状态）；发现镜像串行循环跳过语义，协调器构建，任务只读。
    pub(crate) conflict_inputs: Vec<(crate::VehicleHandle, u32, usize, VehicleState)>,
    /// P3 候选多段报告槽位，按发现序索引；任务独占连续切片写入完整报告，
    /// 协调器按 live×gate 原序规范消费。预留失败只退回融合求值，不新增
    /// 领域错误。
    pub(crate) conflict_slots:
        Vec<crate::kernel::execution::DispatchSlot<crate::kernel::conflict_tick::CandidateReport>>,
}

impl TickWorkspace {
    pub(crate) fn clear_motion_cache(&mut self) {
        // 较长的一份旧行留作备用缓冲，供并行预览消费换入。
        if self.motion_cache.len() > self.motion_cache_spare.len() {
            std::mem::swap(&mut self.motion_cache, &mut self.motion_cache_spare);
        }
        self.motion_cache.clear();
        self.motion_bases.clear();
        self.motion_previews.clear();
        for bases in &mut self.waiting_preview_bases {
            bases.clear();
        }
        for previews in &mut self.waiting_preview_payloads {
            previews.clear();
        }
    }
}

#[cfg(test)]
impl WorldBindingState {
    /// 测试用：绑定状态持有的逻辑字节数。
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        let Self {
            revision: _,
            source,
            world_id: _,
            world_generation: _,
            config: _,
            policy_binding,
        } = self;
        source.retained_logical_bytes() + policy_binding.retained_logical_bytes()
    }
}

#[cfg(test)]
impl CommittedWorldState {
    /// 测试用：已提交状态持有的逻辑字节数。
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        let Self {
            conflict,
            conflict_eligibility,
            latest_conflict_decisions,
            tick_index: _,
            time_ms: _,
            command_cursor: _,
            event_cursor: _,
            observation_state_sequence: _,
            signal_aspects,
            routes,
            free_routes,
            live_route_count: _,
            live_route_edge_occurrence_count: _,
            live_route_conflict_occurrence_count: _,
            vehicles,
            free_vehicles,
            live_order,
            parking,
            waiting_zones,
            latest_waiting_decisions,
            latest_transition_events,
        } = self;
        crate::kernel::state::vec_bytes(conflict_eligibility)
            + crate::kernel::state::vec_bytes(latest_conflict_decisions)
            + crate::kernel::state::vec_bytes(free_routes)
            + vehicles.retained_logical_bytes()
            + crate::kernel::state::vec_bytes(free_vehicles)
            + crate::kernel::state::vec_bytes(live_order)
            + crate::kernel::state::vec_bytes(latest_waiting_decisions)
            + crate::kernel::state::vec_bytes(latest_transition_events)
            + crate::kernel::state::slice_bytes(signal_aspects)
            + crate::kernel::state::slice_bytes(waiting_zones)
            + conflict.retained_logical_bytes()
            + parking.retained_logical_bytes()
            + crate::kernel::state::vec_bytes(routes)
            + routes
                .iter()
                .map(RouteSlot::retained_logical_bytes)
                .sum::<u64>()
    }
}

#[cfg(test)]
impl DerivedIndexes {
    /// 测试用：派生索引持有的逻辑字节数。
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        let Self {
            conflict,
            active_order,
            live_order_index,
            waiting_queue_ends,
            waiting_links,
            waiting_member_rows,
            occupancy,
            spawn_overlap,
            spawn_contenders,
        } = self;
        crate::kernel::state::vec_bytes(active_order)
            + live_order_index.retained_logical_bytes()
            + crate::kernel::state::vec_bytes(waiting_member_rows)
            + crate::kernel::state::slice_bytes(waiting_queue_ends)
            + crate::kernel::state::slice_bytes(waiting_links)
            + conflict.retained_logical_bytes()
            + occupancy.retained_logical_bytes()
            + spawn_overlap.retained_logical_bytes()
            + spawn_contenders.retained_logical_bytes()
    }
}

#[cfg(test)]
impl TickWorkspace {
    /// 测试用：步进工作区持有的逻辑字节数。
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        let Self {
            conflict,
            conflict_candidates,
            conflict_candidate_order,
            conflict_schedule,
            conflict_candidate_cells,
            conflict_candidate_downstream,
            conflict_cell_work,
            conflict_downstream_work,
            conflict_grants,
            conflict_motion_by_vehicle,
            conflict_next_eligibility,
            conflict_table_writes,
            committed_check,
            conflict_passage_transitions,
            conflict_changed_owners,
            waiting_dependencies,
            conflict_staged_decisions,
            next_signal_aspects,
            waiting_claims,
            waiting_plans,
            waiting_plan_by_vehicle,
            next_state_by_vehicle,
            waiting_staged_decisions,
            waiting_non_entry_anchors,
            staged_transition_events,
            waiting_next_counters,
            waiting_staged_occupancy,
            waiting_staged_storage_mm,
            occupancy_scratch,
            motion_cache,
            motion_cache_spare,
            motion_bases,
            motion_previews,
            waiting_preview_bases,
            waiting_preview_payloads,
            next_states,
            motion_next,
            motion_kernel: _,
            waiting_preview_inputs,
            waiting_preview_slots,
            conflict_inputs,
            conflict_slots,
            frontier_maintenance,
        } = self;
        crate::kernel::state::vec_bytes(conflict_candidates)
            + crate::kernel::state::vec_bytes(conflict_candidate_order)
            + crate::kernel::state::vec_bytes(conflict_candidate_cells)
            + crate::kernel::state::vec_bytes(conflict_candidate_downstream)
            + crate::kernel::state::vec_bytes(conflict_cell_work)
            + crate::kernel::state::vec_bytes(conflict_downstream_work)
            + crate::kernel::state::vec_bytes(conflict_grants)
            + crate::kernel::state::vec_bytes(conflict_passage_transitions)
            + crate::kernel::state::vec_bytes(conflict_changed_owners)
            + conflict_table_writes.retained_bytes()
            + committed_check.retained_bytes()
            + crate::kernel::state::vec_bytes(conflict_staged_decisions)
            + crate::kernel::state::vec_bytes(waiting_claims)
            + crate::kernel::state::vec_bytes(waiting_plans)
            + crate::kernel::state::vec_bytes(waiting_staged_decisions)
            + crate::kernel::state::vec_bytes(waiting_non_entry_anchors)
            + crate::kernel::state::vec_bytes(staged_transition_events)
            + crate::kernel::state::vec_bytes(next_states)
            + motion_next.retained_logical_bytes()
            + crate::kernel::state::vec_bytes(motion_cache)
            + crate::kernel::state::vec_bytes(motion_cache_spare)
            + crate::kernel::state::vec_bytes(motion_bases)
            + crate::kernel::state::vec_bytes(motion_previews)
            + crate::kernel::state::vec_bytes(waiting_preview_bases)
            + waiting_preview_bases.iter().map(crate::kernel::state::vec_bytes).sum::<u64>()
            + crate::kernel::state::vec_bytes(waiting_preview_payloads)
            + waiting_preview_payloads.iter().map(crate::kernel::state::vec_bytes).sum::<u64>()
            + crate::kernel::state::vec_bytes(waiting_preview_inputs)
            + crate::kernel::state::vec_bytes(waiting_preview_slots)
            + crate::kernel::state::vec_bytes(conflict_inputs)
            + crate::kernel::state::vec_bytes(conflict_slots)
            + frontier_maintenance.retained_logical_bytes()
            // R3-3b：槽位报告的段 Vec backing 峰值（CellsSegment::Values /
            // Obligated fill）；失败未消费报告在下一拍分发前回收清理，
            // 此处计的是清理前可达的峰值保有。
            + u64::try_from(
                conflict_slots
                    .iter()
                    .map(|slot| match slot {
                        crate::kernel::execution::DispatchSlot::Done(Ok(report)) => {
                            report.retained_logical_bytes()
                        }
                        crate::kernel::execution::DispatchSlot::Pending
                        | crate::kernel::execution::DispatchSlot::Done(Err(_))
                        | crate::kernel::execution::DispatchSlot::Skipped => 0,
                    })
                    .sum::<usize>(),
            )
            .unwrap_or(u64::MAX)
            + crate::kernel::state::slice_bytes(conflict_motion_by_vehicle)
            + crate::kernel::state::slice_bytes(conflict_next_eligibility)
            + crate::kernel::state::slice_bytes(next_signal_aspects)
            + crate::kernel::state::slice_bytes(waiting_plan_by_vehicle)
            + crate::kernel::state::slice_bytes(next_state_by_vehicle)
            + crate::kernel::state::slice_bytes(waiting_next_counters)
            + crate::kernel::state::slice_bytes(waiting_staged_occupancy)
            + crate::kernel::state::slice_bytes(waiting_staged_storage_mm)
            + conflict.retained_logical_bytes()
            + conflict_schedule.retained_logical_bytes() as u64
            + waiting_dependencies.retained_logical_bytes()
            + occupancy_scratch.retained_logical_bytes()
    }
}

/// 测试用：`Vec` 逻辑字节数（capacity × 元素大小）。
#[cfg(test)]
pub(crate) fn vec_bytes<T>(values: &Vec<T>) -> u64 {
    (values.capacity() * core::mem::size_of::<T>()) as u64
}

/// 测试用：切片逻辑字节数。
#[cfg(test)]
pub(crate) fn slice_bytes<T>(values: &[T]) -> u64 {
    core::mem::size_of_val(values) as u64
}

/// 实例自有 backing 的唯一总账；共享根另列，跨世界相加时按 Arc 去重。
/// `Vec` 与切片按元素 `capacity` 或长度计字节，不把分配器开销冒充逻辑存储。
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct WorldMemoryLedger {
    pub(crate) shared_network: u64,
    pub(crate) partitions: [u64; 5],
}

#[cfg(test)]
impl WorldMemoryLedger {
    /// 五个分区自有字节合计（不含共享根）。
    pub(crate) fn world_owned_bytes(&self) -> u64 {
        self.partitions.iter().sum()
    }
}

#[cfg(test)]
impl crate::kernel::state::WorldState {
    /// 汇总世界五类私有状态与共享根的存续内存总账。
    pub(crate) fn retained_memory(&self) -> WorldMemoryLedger {
        let Self {
            binding,
            committed,
            derived,
            workspace,
            admin,
        } = self;
        WorldMemoryLedger {
            shared_network: binding.revision.retained_logical_bytes(),
            partitions: [
                binding.retained_logical_bytes(),
                committed.retained_logical_bytes(),
                derived.retained_logical_bytes(),
                workspace.retained_logical_bytes(),
                admin.retained_logical_bytes(),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::admin::migration_journal::MigrationDeltaJournal;

    #[test]
    fn committed_check_follows_step_and_single_vehicle_commands_only() {
        use super::CommittedCheck;
        use crate::{ObservationStateSequence, WorldGeneration};
        let generation = WorldGeneration::INITIAL;
        let s0 = ObservationStateSequence::INITIAL;
        let s1 = s0.checked_next().unwrap();
        let s2 = s1.checked_next().unwrap();
        let mut check = CommittedCheck::default();
        // 安装后的第一拍没有账本：完整校验。
        assert!(!check.begin_step(generation, s0));
        check.end_step();
        check.reserve(4);
        check.after_step(generation, s1, Some(&[3, 5]));
        assert!(check.begin_step(generation, s1));
        assert_eq!(check.slots(), &[3, 5]);
        check.end_step();
        // 单车命令沿用账本并追加车位。
        check.note_command(generation, s1, s2, 7);
        assert!(check.begin_step(generation, s2));
        assert_eq!(check.slots(), &[3, 5, 7]);
        check.end_step();
        // 序号对不上（其他路径改写过已提交状态）：命令不再续上，完整校验。
        check.note_command(generation, s1, s2, 9);
        assert!(check.begin_step(generation, s2));
        assert!(!check.begin_step(generation.checked_next().unwrap(), s2));
        check.end_step();
        // 写过资格的车位未知时账本失效。
        check.after_step(generation, s2, None);
        assert!(!check.begin_step(generation, s2));
        check.end_step();
        // 提交前未预留足够容量时不在提交窗口分配，账本失效。
        let mut fresh = CommittedCheck::default();
        fresh.after_step(generation, s1, Some(&[1]));
        assert!(!fresh.begin_step(generation, s1));
    }

    #[test]
    fn complete_retained_memory_covers_warm_partitions_and_armed_journal() {
        let mut world = crate::kernel::waiting::tests::multi_gate_world(2);
        let initial = world.state.retained_memory();
        assert!(initial.shared_network > 0);
        assert!(initial.partitions[..4].iter().all(|bytes| *bytes > 0));
        assert_eq!(initial.partitions[4], 0);
        for _ in 0..3 {
            world
                .step(crate::TickInput::new(world.config().fixed_delta_time_ms()))
                .unwrap();
        }
        let warm = world.state.retained_memory();
        assert!(warm.world_owned_bytes() >= initial.world_owned_bytes());
        assert!(
            world
                .state
                .workspace
                .occupancy_scratch
                .retained_logical_bytes()
                > 0
        );
        world.state.admin.migration_journal =
            Some(MigrationDeltaJournal::arm(4_096, world.state.committed.command_cursor).unwrap());
        let armed = world.state.retained_memory();
        assert_eq!(armed.shared_network, warm.shared_network);
        assert_eq!(&armed.partitions[..4], &warm.partitions[..4]);
        assert!(armed.partitions[4] >= 4_096);
        assert_eq!(
            armed.world_owned_bytes() - warm.world_owned_bytes(),
            armed.partitions[4]
        );
        eprintln!("runtime-state-memory initial={initial:?} warm={warm:?} armed={armed:?}");
    }
}
