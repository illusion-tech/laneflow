//! W7 fixed-step Conflict orchestration.
//!
//! 本模块只编排已经由 `conflict`、`tables` 与 `waiting` 拥有的语义原语：静态
//! passage cell 不复制到动态路线，候选先完整求值，再由单写者 arbiter 按稳定键取得
//! 组合资源。tick-local grant 不进入公开状态，也不跨 tick 保留。

use laneflow_static_contract::{VehicleProfileOrdinal, WaitingZoneOrdinal};
use laneflow_static_network::BoundedDistance;

use crate::admin::migration_journal::{ConflictOccurrenceJournalLocator, MigrationDeltaJournal};
use crate::kernel::conflict::{
    ConflictAcquireError, ConflictCandidateOrderKey, ConflictGrant, GrantResourceBundle,
    WaitingAdmissionEntitlement,
};
use crate::kernel::occupancy::LeaderQueryHorizon;
use crate::kernel::tables::distance_to_occurrence_progress;
use crate::{
    ApproachEstimate, ConflictEligibilityState, ConflictPassageAddress,
    ConflictPassageOccurrenceLocator, ConflictPassageRange, ConflictResourceNoGrant,
    ConflictYieldOutcome, GateCandidateKind, GatePolicyDecision, ManeuverTraversalPhase,
    ManeuverTraversalState, ParkingBinding, RouteHandle, StepError, TrafficWorld, VehicleHandle,
    VehicleState, VehicleStatus,
};

/// Conflict 决定的稳定动态路线锚点。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConflictRouteAnchor {
    pub(crate) route: RouteHandle,
    pub(crate) maneuver_occurrence_index: u32,
    pub(crate) hop: u32,
}

impl ConflictRouteAnchor {
    /// 锚点所属的动态路线句柄。
    #[must_use]
    pub const fn route(self) -> RouteHandle {
        self.route
    }
    /// 路线内的机动路径出现项下标。
    #[must_use]
    pub const fn maneuver_occurrence_index(self) -> u32 {
        self.maneuver_occurrence_index
    }
    /// 决定发生的路线 hop（准入 Gate 所在边下标）。
    #[must_use]
    pub const fn hop(self) -> u32 {
        self.hop
    }
}

/// successful tick 内一个候选没有取得完整组合资源的稳定归因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictNoGrantReason {
    /// Waiting 准入因容量拒绝，连带组合资源拒绝。
    WaitingCapacity,
    /// Waiting 准入因物理存储拒绝，连带组合资源拒绝。
    WaitingPhysicalStorage,
    /// 组合资源依赖在 Waiting 依赖图中成环。
    WaitingCycle,
    /// 冲突 zone/cell 已被其它 owner 占用或提交。
    ConflictOccupied,
    /// 后随间隙不足：滞后基准（实际清空或切换保守起点 `CutoverFloor`）起算
    /// 的已逝时间小于 required lag。
    LagGap,
    /// 接近估计不可证明，保守拒绝 crossing。
    ApproachUnprovable,
    /// 前导间隙不足：对方保守最早到达早于或等于 required lead（仅严格更晚
    /// 被接受）。
    LeadGap,
    /// 无法派生下游 claim：清空目标越过路线存储上界、前方车辆间隙不足，或
    /// 目标位于下一 Gate/等待停车/预留停车之后。
    DownstreamStorageBoundary,
    /// 下游 claim 区间与既有 claim（含 follower 最小间隙）冲突。
    DownstreamClaimConflict,
}

/// 刚完成 successful tick 的 Conflict 决定。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictDecisionOutcome {
    /// 门规则拒绝，未进入组合资源求值。
    NotEvaluated,
    /// 该 Gate 无 passage 资源要求，无需求值；仅当 finalized 运动确实过门时
    /// 发布该结果。
    NotRequired,
    /// 组合资源全部取得；为阶段性授予——运动投影回落到门上时不提交授予。
    Granted,
    /// 未取得组合资源，附稳定归因。
    NoGrant(ConflictNoGrantReason),
}

/// 一条 Conflict/Gate 组合资源决定。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConflictDecision {
    pub(crate) vehicle: VehicleHandle,
    pub(crate) vehicle_update_sequence: u32,
    pub(crate) anchor: ConflictRouteAnchor,
    pub(crate) passage: Option<ConflictPassageOccurrenceLocator>,
    pub(crate) outcome: ConflictDecisionOutcome,
}

impl ConflictDecision {
    /// 决定针对的车辆句柄。
    #[must_use]
    pub const fn vehicle(self) -> VehicleHandle {
        self.vehicle
    }
    /// 车辆在稳定更新顺序中的下标。
    #[must_use]
    pub const fn vehicle_update_sequence(self) -> u32 {
        self.vehicle_update_sequence
    }
    /// 决定的稳定动态路线锚点。
    #[must_use]
    pub const fn anchor(self) -> ConflictRouteAnchor {
        self.anchor
    }
    /// 涉及的冲突通行段出现项 locator；无 passage 资源时为 `None`。
    #[must_use]
    pub const fn passage(self) -> Option<ConflictPassageOccurrenceLocator> {
        self.passage
    }
    /// 本拍决定结果（授予、未要求或拒绝归因）。
    #[must_use]
    pub const fn outcome(self) -> ConflictDecisionOutcome {
        self.outcome
    }
}

/// 等待组合仲裁的单车 Conflict 候选及其工作区切片。
#[derive(Clone, Copy, Debug)]
pub(crate) struct ConflictCandidate {
    pub(crate) vehicle: VehicleHandle,
    pub(crate) vehicle_update_sequence: u32,
    pub(crate) key: ConflictCandidateOrderKey,
    pub(crate) anchor: ConflictRouteAnchor,
    pub(crate) passage: Option<ConflictPassageOccurrenceLocator>,
    pub(crate) passage_range: Option<ConflictPassageRange>,
    pub(crate) cells_start: usize,
    pub(crate) cells_end: usize,
    pub(crate) downstream_start: usize,
    pub(crate) downstream_end: usize,
    pub(crate) follower_min_gap_mm: u32,
    pub(crate) waiting_zone: Option<WaitingZoneOrdinal>,
    pub(crate) preflight_no_grant: Option<ConflictNoGrantReason>,
}

/// 已通过 Gate 法规与本地准入检查的资源请求入口。
struct EvaluatedGate {
    anchor: ConflictRouteAnchor,
    passage: Option<ConflictPassageOccurrenceLocator>,
    range: crate::kernel::tables::ConflictGateRange,
    kind: GateCandidateKind,
    waiting_zone: Option<WaitingZoneOrdinal>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ReadyCandidate {
    key: ConflictCandidateOrderKey,
    index: usize,
}

/// 全局只比较局部就绪队首。容器位置不改变 Waiting 冻结的前后顺序。
#[derive(Default)]
pub(crate) struct ConflictSchedule {
    ready: std::collections::BinaryHeap<std::cmp::Reverse<ReadyCandidate>>,
    successors: Vec<Option<usize>>,
    local: Vec<(WaitingZoneOrdinal, u32, usize)>,
}

impl ConflictSchedule {
    fn prepare(
        &mut self,
        candidates: &[ConflictCandidate],
        plans: &[Option<std::num::NonZeroU32>],
    ) -> Result<(), StepError> {
        self.ready.clear();
        self.successors.clear();
        self.local.clear();
        #[cfg(test)]
        if candidates.len() > self.ready.capacity() {
            crate::kernel::conflict::check_allocation_failpoint()
                .map_err(|_| StepError::ConflictScratchAllocFailed)?;
        }
        self.ready
            .try_reserve(candidates.len())
            .map_err(|_| StepError::ConflictScratchAllocFailed)?;
        reserve(&mut self.successors, candidates.len())?;
        reserve(&mut self.local, candidates.len())?;
        self.successors.resize(candidates.len(), None);
        for (index, candidate) in candidates.iter().enumerate() {
            if let Some(zone) = candidate.waiting_zone {
                let order = plans[candidate.vehicle.index() as usize]
                    .ok_or(StepError::WaitingInvariantViolation)?
                    .get();
                self.local.push((zone, order, index));
            } else {
                self.ready.push(std::cmp::Reverse(ReadyCandidate {
                    key: candidate.key,
                    index,
                }));
            }
        }
        self.local.sort_unstable();
        for group in self.local.chunk_by(|left, right| left.0 == right.0) {
            let index = group[0].2;
            self.ready.push(std::cmp::Reverse(ReadyCandidate {
                key: candidates[index].key,
                index,
            }));
            for pair in group.windows(2) {
                self.successors[pair[0].2] = Some(pair[1].2);
            }
        }
        Ok(())
    }

    fn next(&mut self, candidates: &[ConflictCandidate]) -> Option<usize> {
        let std::cmp::Reverse(item) = self.ready.pop()?;
        if let Some(index) = self.successors[item.index] {
            self.ready.push(std::cmp::Reverse(ReadyCandidate {
                key: candidates[index].key,
                index,
            }));
        }
        Some(item.index)
    }

    /// 测试专用：调度暂存按容量计的存续逻辑字节数。
    #[cfg(test)]
    pub(crate) fn retained_logical_bytes(&self) -> usize {
        let Self {
            ready,
            successors,
            local,
        } = self;
        ready.capacity() * std::mem::size_of::<std::cmp::Reverse<ReadyCandidate>>()
            + successors.capacity() * std::mem::size_of::<Option<usize>>()
            + local.capacity() * std::mem::size_of::<(WaitingZoneOrdinal, u32, usize)>()
    }
}

/// 已通过组合仲裁、等待车辆位置验证后提交的 Conflict grant。
pub(crate) struct PreparedConflictGrant {
    pub(crate) vehicle: VehicleHandle,
    pub(crate) gate_hop: u32,
    pub(crate) passage_range: Option<ConflictPassageRange>,
    pub(crate) grant: ConflictGrant,
}

/// 单车本拍的 Conflict 运动决定：Gate hop、结果与 grant 下标。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ConflictMotionPlan {
    pub(crate) gate_hop: u32,
    pub(crate) outcome: ConflictDecisionOutcome,
    pub(crate) grant_index: Option<std::num::NonZeroU32>,
}

/// 单个冲突通行段出现项的进入/清空转移记录。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ConflictPassageTransition {
    pub(crate) vehicle: VehicleHandle,
    pub(crate) occurrence_index: u32,
    pub(crate) address: ConflictPassageAddress,
    pub(crate) enter: bool,
    pub(crate) clear: bool,
}

/// 以 `try_reserve` 扩容暂存向量；失败映射为 `StepError::ConflictScratchAllocFailed`。
pub(crate) fn reserve<T>(values: &mut Vec<T>, additional: usize) -> Result<(), StepError> {
    #[cfg(test)]
    if additional > values.capacity() - values.len() {
        crate::kernel::conflict::check_allocation_failpoint()
            .map_err(|_| StepError::ConflictScratchAllocFailed)?;
    }
    values
        .try_reserve(additional)
        .map_err(|_| StepError::ConflictScratchAllocFailed)
}

fn gate_boundary(hop: u32) -> Result<crate::DownstreamRoutePoint, ConflictAcquireError> {
    crate::DownstreamRoutePoint::new(
        hop.checked_add(1)
            .ok_or(ConflictAcquireError::InvalidBundle)?,
        0,
        0,
    )
    .ok_or(ConflictAcquireError::InvalidBundle)
}

fn map_acquire_error(error: ConflictAcquireError) -> Result<ConflictNoGrantReason, StepError> {
    match error {
        ConflictAcquireError::NoGrant(reason) => Ok(match reason {
            ConflictResourceNoGrant::WaitingCycle => ConflictNoGrantReason::WaitingCycle,
            ConflictResourceNoGrant::ConflictOccupied => ConflictNoGrantReason::ConflictOccupied,
            ConflictResourceNoGrant::DownstreamStorageBoundary => {
                ConflictNoGrantReason::DownstreamStorageBoundary
            }
            ConflictResourceNoGrant::DownstreamClaimConflict => {
                ConflictNoGrantReason::DownstreamClaimConflict
            }
        }),
        ConflictAcquireError::InvalidBundle | ConflictAcquireError::Capacity => {
            Err(StepError::ConflictInvariantViolation)
        }
        ConflictAcquireError::ScratchAllocFailed => Err(StepError::ConflictScratchAllocFailed),
    }
}

fn map_yield(outcome: ConflictYieldOutcome) -> Option<ConflictNoGrantReason> {
    match outcome {
        ConflictYieldOutcome::Accepted => None,
        ConflictYieldOutcome::Occupied => Some(ConflictNoGrantReason::ConflictOccupied),
        ConflictYieldOutcome::LagGap => Some(ConflictNoGrantReason::LagGap),
        ConflictYieldOutcome::LeadGap => Some(ConflictNoGrantReason::LeadGap),
        ConflictYieldOutcome::ApproachUnprovable => Some(ConflictNoGrantReason::ApproachUnprovable),
    }
}

const fn no_grant_rank(reason: ConflictNoGrantReason) -> u8 {
    match reason {
        ConflictNoGrantReason::WaitingCapacity | ConflictNoGrantReason::WaitingPhysicalStorage => 0,
        ConflictNoGrantReason::WaitingCycle => 0,
        ConflictNoGrantReason::ConflictOccupied => 1,
        ConflictNoGrantReason::LagGap => 2,
        ConflictNoGrantReason::ApproachUnprovable => 3,
        ConflictNoGrantReason::LeadGap => 4,
        ConflictNoGrantReason::DownstreamStorageBoundary => 5,
        ConflictNoGrantReason::DownstreamClaimConflict => 6,
    }
}

impl crate::kernel::state::WorldState {
    /// 刚完成 successful tick 的 Conflict 决定批次。
    #[must_use]
    pub fn latest_conflict_decisions(&self) -> &[ConflictDecision] {
        &self.committed.latest_conflict_decisions
    }

    /// 测试专用：执行本拍 Conflict 准备（候选求值与组合仲裁）。
    #[cfg(test)]
    pub(crate) fn prepare_conflict_step(
        &mut self,
        delta_s: f32,
        tick: u64,
        execution: Option<&crate::kernel::execution::ExecutionResources>,
    ) -> Result<(), StepError> {
        self.step_workspace()
            .prepare_conflict_step(delta_s, tick, execution)
    }

    #[cfg(test)]
    fn prepare_conflict_candidates(
        &mut self,
        delta_s: f32,
        tick: u64,
        execution: Option<&crate::kernel::execution::ExecutionResources>,
    ) -> Result<(), StepError> {
        self.step_workspace()
            .prepare_conflict_candidates(delta_s, tick, execution)
    }

    #[cfg(test)]
    fn acquire_conflict_candidates(&mut self, tick: u64) -> Result<(), StepError> {
        self.step_workspace().acquire_conflict_candidates(tick)
    }

    /// 测试专用：验证并定稿本拍 Conflict 提交计划。
    #[cfg(test)]
    pub(crate) fn finalize_conflict_step(
        &mut self,
        updates: &mut [(usize, VehicleState)],
    ) -> Result<(), StepError> {
        self.step_workspace().finalize_conflict_step(updates)
    }

    /// 测试专用：Conflict 相关 committed 与暂存容量的存续逻辑字节总数。
    #[cfg(test)]
    pub(crate) fn conflict_retained_logical_bytes(&self) -> u64 {
        fn vec_bytes<T>(values: &Vec<T>) -> usize {
            values.capacity().saturating_mul(core::mem::size_of::<T>())
        }
        let bytes = self.conflict_read().retained_logical_bytes() as usize
            + self.workspace.conflict_schedule.retained_logical_bytes()
            + vec_bytes(&self.committed.conflict_eligibility)
            + vec_bytes(&self.workspace.conflict_candidates)
            + vec_bytes(&self.workspace.conflict_candidate_cells)
            + vec_bytes(&self.workspace.conflict_candidate_downstream)
            + vec_bytes(&self.workspace.conflict_cell_work)
            + vec_bytes(&self.workspace.conflict_downstream_work)
            + vec_bytes(&self.workspace.conflict_grants)
            + self.workspace.conflict_motion_by_vehicle.len()
                * core::mem::size_of::<Option<ConflictMotionPlan>>()
            + self.workspace.conflict_next_eligibility.len()
                * core::mem::size_of::<Option<ConflictEligibilityState>>()
            + vec_bytes(&self.workspace.conflict_passage_transitions)
            + vec_bytes(&self.workspace.conflict_changed_owners)
            + self.workspace.waiting_dependencies.retained_logical_bytes() as usize
            + vec_bytes(&self.workspace.conflict_staged_decisions)
            + vec_bytes(&self.workspace.motion_cache)
            + vec_bytes(&self.committed.latest_conflict_decisions);
        u64::try_from(bytes).expect("Conflict retained bytes fit u64")
    }
}

impl TrafficWorld {
    /// 刚完成 successful tick 的 Conflict 决定批次。
    ///
    /// # Panics
    ///
    /// 世界因执行 panic 失效后调用会 panic；宿主必须销毁并重新构建世界。
    #[must_use]
    pub fn latest_conflict_decisions(&self) -> &[ConflictDecision] {
        self.execution.assert_usable();
        self.state.latest_conflict_decisions()
    }
}

impl<'a> crate::kernel::phase::StepReadView<'a> {
    /// 把路线内的冲突出现项编码为迁移日志的稳定 locator。
    pub(crate) fn conflict_journal_locator(
        self,
        route: RouteHandle,
        conflict_occurrence_index: u32,
    ) -> Result<ConflictOccurrenceJournalLocator, ()> {
        let occurrence = self
            .compiled_route(route)
            .and_then(|compiled| compiled.conflicts.get(conflict_occurrence_index as usize))
            .ok_or(())?;
        Ok(ConflictOccurrenceJournalLocator {
            route,
            stream: occurrence.stream.raw(),
            zone: occurrence.zone.raw(),
            passage_local_index: occurrence.passage_local_index,
            entry_route_edge_index: occurrence.entry.route_edge_index,
            entry_progress_mm: occurrence.entry.progress_mm,
            clearance_route_edge_index: occurrence.clearance.route_edge_index,
            clearance_progress_mm: occurrence.clearance.progress_mm,
        })
    }
}

impl crate::kernel::phase::StepWorkspace<'_> {
    /// 计算车辆因未授权 Conflict/Waiting 资源而必须停车的最近约束。
    /// 生产路径经 MotionTaskView::conflict_stop_for（冻结暂存视图）读取；
    /// StepWorkspace 版仅服务既有测试调用。
    #[cfg(test)]
    pub(crate) fn conflict_stop_for(
        &self,
        state: &VehicleState,
    ) -> Result<Option<crate::kernel::waiting::WaitingStopConstraint>, StepError> {
        let grant_hop = self
            .workspace
            .conflict_motion_by_vehicle
            .get(state.handle.index() as usize)
            .copied()
            .flatten()
            .filter(|plan| plan.outcome == ConflictDecisionOutcome::Granted)
            .map(|plan| plan.gate_hop);
        let compiled = self
            .compiled_route(state.route)
            .ok_or(StepError::ConflictInvariantViolation)?;
        let owned_hop = self
            .conflict_read()
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
        let conflict = loop {
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
        };
        let waiting = compiled
            .waiting
            .partition_point(|entry| entry.entry_hop < first_hop);
        let waiting = compiled.waiting[waiting..]
            .iter()
            .find(|entry| !authorized(entry.entry_hop))
            .map(|entry| entry.entry_hop);
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

    /// 执行本拍 Conflict 准备：候选求值后接组合仲裁。
    pub(crate) fn prepare_conflict_step(
        &mut self,
        delta_s: f32,
        tick: u64,
        execution: Option<&crate::kernel::execution::ExecutionResources>,
    ) -> Result<(), StepError> {
        self.prepare_conflict_candidates(delta_s, tick, execution)?;
        self.acquire_conflict_candidates(tick)
    }

    /// 清空暂存、重建求值前沿，并求值全部活动车辆的 Gate 候选。
    pub(crate) fn prepare_conflict_candidates(
        &mut self,
        delta_s: f32,
        tick: u64,
        execution: Option<&crate::kernel::execution::ExecutionResources>,
    ) -> Result<(), StepError> {
        self.committed
            .prepare_conflict(&mut self.derived, &mut self.workspace.conflict)
            .discard_staged();
        self.committed
            .prepare_conflict(&mut self.derived, &mut self.workspace.conflict)
            .clear_approach_frontier();
        self.workspace.conflict_candidates.clear();
        self.workspace.conflict_candidate_cells.clear();
        self.workspace.conflict_candidate_downstream.clear();
        self.workspace.conflict_grants.clear();
        self.workspace.conflict_passage_transitions.clear();
        self.workspace.conflict_staged_decisions.clear();
        self.workspace.conflict_motion_by_vehicle.fill(None);
        self.workspace.conflict_next_eligibility.fill(None);

        self.rebuild_conflict_frontier()?;
        reserve(
            &mut self.workspace.conflict_candidates,
            self.derived.active_order.len(),
        )?;
        reserve(
            &mut self.workspace.conflict_staged_decisions,
            self.derived.active_order.len(),
        )?;

        // #706 增量 D：Pool 执行器下走 P3 真实分发（阈值/强制 + 发现、槽位
        // 预留失败回退）；Caller/无执行器保持融合循环。分发或回退后共享
        // 尾部 reserve + sort；融合循环体一行不动。增量 E：fuse 旋钮
        // （组合矩阵融合侧）优先于 force 与 Pool 执行器。
        let mut dispatched = false;
        if !conflict_dispatch_fuse_forced()
            && let Some(resources @ crate::kernel::execution::ExecutionResources::Pool(_)) =
                execution
        {
            dispatched = self.prepare_conflict_candidates_dispatched(resources, delta_s, tick)?;
        } else {
            #[cfg(test)]
            count_conflict_path(|counts| counts.fused += 1);
        }
        if !dispatched {
            // 与分发发现序同序的非有限注入计数（cfg(test)，融合参考臂同样
            // 按 live×gate 发现位点火；生产构建零开销）。
            #[cfg(test)]
            let mut workload_index = 0_usize;
            let mut active_index = 0;
            for sequence in 0..self.committed.live_order.len() {
                let vehicle = self.committed.live_order[sequence];
                let Some(state) = self.vehicle_state(vehicle).copied() else {
                    continue;
                };
                if state.status != VehicleStatus::Active {
                    continue;
                }
                let cache_index = active_index;
                active_index += 1;
                if self.conflict_read().reservation(vehicle).is_some() {
                    continue;
                }
                #[cfg(test)]
                {
                    if conflict_injection::nonfinite_injected(self.binding.world_id, workload_index)
                    {
                        return Err(StepError::NonFiniteMotion);
                    }
                    workload_index += 1;
                }
                self.evaluate_vehicle_gates(
                    state,
                    u32::try_from(sequence).map_err(|_| StepError::ConflictInvariantViolation)?,
                    cache_index,
                    delta_s,
                    tick,
                )?;
            }
        }

        reserve(
            &mut self.workspace.conflict_staged_decisions,
            self.workspace.conflict_candidates.len(),
        )?;
        self.workspace
            .conflict_candidates
            .sort_unstable_by_key(|candidate| (candidate.key, candidate.vehicle_update_sequence));
        Ok(())
    }

    /// P3 分发路径（D3/D5）：输入发现（镜像串行循环的跳过语义与
    /// cache_index 递增次序）→ 阈值/强制 → 槽位 → 块分发 → 完整 join →
    /// 协调器按 live×gate 发现序规范消费（共享写与真实预留原位施加）。
    /// 返回 Ok(false) 表示回退融合（调用方执行原串行循环）；发现/槽位
    /// 预留失败与任务局部暂存不足都不新增领域错误。
    fn prepare_conflict_candidates_dispatched(
        &mut self,
        execution: &crate::kernel::execution::ExecutionResources,
        delta_s: f32,
        tick: u64,
    ) -> Result<bool, StepError> {
        let view = ConflictTaskView {
            read: crate::kernel::phase::StepReadView {
                binding: self.binding,
                committed: &self.committed,
                derived: &self.derived,
            },
            conflict: crate::kernel::conflict::ConflictRead::new(
                &self.committed.conflict,
                &self.derived.conflict,
                &self.workspace.conflict,
            ),
            waiting_plans: &self.workspace.waiting_plans,
            waiting_plan_by_vehicle: &self.workspace.waiting_plan_by_vehicle,
            motion_cache: &self.workspace.motion_cache,
        };
        let inputs = &mut self.workspace.conflict_inputs;
        inputs.clear();
        #[cfg(test)]
        let input_injected = conflict_injection::input_reserve_injected();
        #[cfg(not(test))]
        let input_injected = false;
        if inputs
            .try_reserve(view.read.committed.live_order.len())
            .is_err()
            || input_injected
        {
            #[cfg(test)]
            count_conflict_path(|counts| counts.slot_fallback += 1);
            return Ok(false);
        }
        let mut active_index = 0_usize;
        for (sequence, vehicle) in view.read.committed.live_order.iter().copied().enumerate() {
            let Some(state) = view.read.vehicle_state(vehicle) else {
                continue;
            };
            if state.status != VehicleStatus::Active {
                continue;
            }
            // cache_index 在 reservation 跳过之前递增：Active 紧凑位与融合
            // 循环保持一致，跳过车辆不占候选但占缓存位。
            let cache_index = active_index;
            active_index += 1;
            if view.conflict.reservation(vehicle).is_some() {
                continue;
            }
            let sequence =
                u32::try_from(sequence).map_err(|_| StepError::ConflictInvariantViolation)?;
            inputs.push((vehicle, sequence, cache_index, *state));
        }
        let workload = inputs.len();
        #[cfg(test)]
        let forced = conflict_dispatch_forced();
        #[cfg(not(test))]
        let forced = false;
        if workload < CONFLICT_DISPATCH_MIN_ACTIVE && !(forced && workload > 0) {
            #[cfg(test)]
            count_conflict_path(|counts| counts.fused += 1);
            return Ok(false);
        }
        let slots = &mut self.workspace.conflict_slots;
        slots.clear();
        #[cfg(test)]
        let slot_injected = conflict_injection::slot_reserve_injected();
        #[cfg(not(test))]
        let slot_injected = false;
        if slots.try_reserve(workload).is_err() || slot_injected {
            #[cfg(test)]
            count_conflict_path(|counts| counts.slot_fallback += 1);
            return Ok(false);
        }
        // 调度统计在可选槽位预留成功后才登记：回退拍只计 slot_fallback，
        // 与 dispatched/fused 互斥。
        #[cfg(test)]
        count_conflict_path(|counts| counts.dispatched += 1);
        slots.resize(workload, crate::kernel::execution::DispatchSlot::Pending);
        // 块数 = 线程数 × 2 与候选工作集取较小者；语义中立（与 P5 同默认值）。
        let chunk_count = execution
            .dispatch_threads()
            .saturating_mul(2)
            .clamp(1, workload);
        let chunk_size = workload.div_ceil(chunk_count).max(1);
        let first_error = std::sync::atomic::AtomicUsize::new(usize::MAX);
        // 块级计数诊断按协调器开关分配/记录；任务内以捕获的布尔为准。
        #[cfg(test)]
        let diagnostics = CONFLICT_WORK_DIAGNOSTICS.with(std::cell::Cell::get);
        #[cfg(not(test))]
        #[allow(unused_variables)]
        let diagnostics = false;
        #[cfg(test)]
        let chunk_records = diagnostics.then(|| {
            (0..chunk_count)
                .map(|_| ConflictWorkChunkRecord::default())
                .collect::<Vec<_>>()
        });
        #[cfg(test)]
        let tls_baseline = diagnostics.then(conflict_tls_snapshot);
        let compute =
            |_chunk_view: crate::kernel::phase::StepReadView<'_>,
             start: usize,
             chunk: &mut [crate::kernel::execution::DispatchSlot<CandidateReport>]| {
                #[cfg(test)]
                let chunk_baseline = diagnostics.then(conflict_tls_snapshot);
                for (offset, slot) in chunk.iter_mut().enumerate() {
                    let index = start + offset;
                    let (_vehicle, sequence, cache_index, state) =
                        self.workspace.conflict_inputs[index];
                    *slot = crate::kernel::execution::DispatchSlot::Done(Ok(
                        view.evaluate_candidate(state, sequence, index, cache_index, delta_s, tick)
                    ));
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
                aggregate_conflict_tls(baseline, records);
            }
            crate::kernel::execution::note_last_dispatch_stats(dispatch_stats);
            LAST_CONFLICT_DISPATCH_STATS.with(|cell| cell.set(Some(dispatch_stats)));
            if let Some(position) = CONFLICT_SLOT_GAP.with(std::cell::Cell::get)
                && let Some(slot) = slots.get_mut(position)
            {
                // 完成前沿不变量注入：首错之前出现未计算槽位，协调器须检出。
                *slot = crate::kernel::execution::DispatchSlot::Pending;
            }
        }
        #[cfg(not(test))]
        let _ = dispatch_stats;
        for index in 0..self.workspace.conflict_inputs.len() {
            let (vehicle, sequence, cache_index, state) = self.workspace.conflict_inputs[index];
            let slot = core::mem::replace(
                &mut self.workspace.conflict_slots[index],
                crate::kernel::execution::DispatchSlot::Skipped,
            );
            let report = match slot {
                crate::kernel::execution::DispatchSlot::Done(Ok(report)) => report,
                crate::kernel::execution::DispatchSlot::Done(Err(error)) => return Err(error),
                crate::kernel::execution::DispatchSlot::Pending
                | crate::kernel::execution::DispatchSlot::Skipped => {
                    // 完成前沿不变量违例：缺失/跳过/旧 attempt 回报不得视为
                    // 成功或无结果。
                    return Err(StepError::ConflictInvariantViolation);
                }
            };
            self.consume_conflict_candidate(vehicle, sequence, cache_index, state, report, tick)?;
        }
        Ok(true)
    }

    /// P3 规范消费（D3/D5）：按发现序先落缓存更新，再按报告施加共享写与
    /// 真实预留（F1/F2/F3b/F4/staged 原位，次序与融合逐行一致）；任务局部
    /// 暂存不足的段由协调器以同领域原语补算，不冒充领域分配失败。
    fn consume_conflict_candidate(
        &mut self,
        vehicle: VehicleHandle,
        update_sequence: u32,
        cache_index: usize,
        state: VehicleState,
        report: CandidateReport,
        tick: u64,
    ) -> Result<(), StepError> {
        match &report {
            CandidateReport::None { cache }
            | CandidateReport::Staged { cache, .. }
            | CandidateReport::Failed { cache, .. } => {
                self.apply_conflict_cache_updates(vehicle, cache_index, cache)
            }
            CandidateReport::Resource(resource) => {
                self.apply_conflict_cache_updates(vehicle, cache_index, &resource.cache)
            }
        }
        match report {
            CandidateReport::None { .. } => Ok(()),
            CandidateReport::Staged { decision, .. } => {
                reserve(&mut self.workspace.conflict_staged_decisions, 1)?;
                self.workspace.conflict_staged_decisions.push(decision);
                Ok(())
            }
            CandidateReport::Failed { error, .. } => Err(error),
            CandidateReport::Resource(resource) => {
                self.workspace.conflict_motion_by_vehicle[vehicle.index() as usize] =
                    Some(resource.motion_plan);
                if let Some(eligibility) = resource.next_eligibility {
                    self.workspace.conflict_next_eligibility[vehicle.index() as usize] =
                        Some(eligibility);
                }
                match resource.stage {
                    ResourceStage::PureWaitingEmpty {
                        key,
                        anchor,
                        waiting_zone,
                    } => {
                        let cells_len = self.workspace.conflict_candidate_cells.len();
                        let downstream_len = self.workspace.conflict_candidate_downstream.len();
                        self.workspace.conflict_candidates.push(ConflictCandidate {
                            vehicle,
                            vehicle_update_sequence: update_sequence,
                            key,
                            anchor,
                            passage: None,
                            passage_range: None,
                            cells_start: cells_len,
                            cells_end: cells_len,
                            downstream_start: downstream_len,
                            downstream_end: downstream_len,
                            follower_min_gap_mm: 0,
                            waiting_zone,
                            preflight_no_grant: None,
                        });
                        Ok(())
                    }
                    ResourceStage::CheckFailed(error) => Err(error),
                    ResourceStage::Computed {
                        stable_passage,
                        passage_range,
                        gate_kind,
                        waiting_zone,
                        priority,
                        preflight_no_grant,
                        cells,
                        downstream,
                    } => self.consume_computed_candidate(
                        vehicle,
                        update_sequence,
                        state,
                        tick,
                        resource.next_eligibility,
                        stable_passage,
                        passage_range,
                        gate_kind,
                        waiting_zone,
                        priority,
                        preflight_no_grant,
                        cells,
                        downstream,
                    ),
                }
            }
        }
    }

    /// Computed 段消费：cells/downstream 各段按序施加真实预留与共享写；
    /// 段内失败与融合同位同序（F1 先于同车领域错误，F2 先于更晚的
    /// downstream 检查错误）。
    #[allow(clippy::too_many_arguments)]
    fn consume_computed_candidate(
        &mut self,
        vehicle: VehicleHandle,
        update_sequence: u32,
        state: VehicleState,
        tick: u64,
        next_eligibility: Option<crate::ConflictEligibilityState>,
        stable_passage: crate::ConflictPassageOccurrenceLocator,
        passage_range: ConflictPassageRange,
        gate_kind: GateCandidateKind,
        waiting_zone: Option<laneflow_static_contract::WaitingZoneOrdinal>,
        priority: Option<i32>,
        preflight_no_grant: Option<ConflictNoGrantReason>,
        cells: CellsSegment,
        downstream: DownstreamSegment,
    ) -> Result<(), StepError> {
        let eligibility = next_eligibility.ok_or(StepError::ConflictInvariantViolation)?;
        let gate_hop = passage_range.admission_gate_hop();
        let mut preflight_no_grant = preflight_no_grant;
        let (cells_start, cells_end) = match cells {
            CellsSegment::Values(values) => {
                // R1：cell 工作区生命周期对齐融合原语——进入本候选的 F1
                // 之前清空（不得只在一拍开始时清一次）。
                self.workspace.conflict_cell_work.clear();
                self.reserve_conflict_cell_work(passage_range.passage_count() as usize)?;
                // 内容对拍：cell 工作区终态与融合一致（当前候选的 cells），
                // 缓冲长度只属当前候选、不跨候选/跨拍累积。
                self.workspace.conflict_cell_work.extend_from_slice(&values);
                let cells_start = self.workspace.conflict_candidate_cells.len();
                self.reserve_conflict_candidate_cells(values.len())?;
                self.workspace
                    .conflict_candidate_cells
                    .extend_from_slice(&values);
                (cells_start, self.workspace.conflict_candidate_cells.len())
            }
            CellsSegment::Failed(error) => {
                self.workspace.conflict_cell_work.clear();
                self.reserve_conflict_cell_work(passage_range.passage_count() as usize)?;
                return Err(error);
            }
            CellsSegment::Unmaterialized => {
                // 任务局部暂存不足：协调器以同领域原语整段补算（F1/F2/
                // downstream/候选 push 一体，含 motion plan/eligibility 重写，
                // 值与已写报告一致）。
                let gate = EvaluatedGate {
                    anchor: ConflictRouteAnchor {
                        route: passage_range.route(),
                        maneuver_occurrence_index: passage_range.maneuver_occurrence_index(),
                        hop: gate_hop,
                    },
                    passage: Some(stable_passage),
                    range: crate::kernel::tables::ConflictGateRange {
                        start: passage_range.first_conflict_occurrence_index(),
                        len: passage_range.passage_count(),
                    },
                    kind: gate_kind,
                    waiting_zone,
                };
                return self.prepare_resource_candidate(state, update_sequence, tick, gate);
            }
        };
        // R1：downstream 工作区在进入 downstream 分支之前清空（含 preflight
        // 跳过路径），与融合 prepare_resource_candidate 在进入 downstream
        // 前 clear 对齐——缓冲内容只属当前候选。
        self.workspace.conflict_downstream_work.clear();
        let (downstream_start, downstream_end) = match downstream {
            DownstreamSegment::SkippedPreflight => {
                let len = self.workspace.conflict_candidate_downstream.len();
                (len, len)
            }
            DownstreamSegment::PreFailed(DownstreamEvalError::NoGrant) => {
                // A4/A5：折入 preflight，不公开错误；空 downstream 区间。
                // F4 前结束，无 F4 义务（不得补预留）。
                preflight_no_grant = Some(map_acquire_error(ConflictAcquireError::NoGrant(
                    ConflictResourceNoGrant::DownstreamStorageBoundary,
                ))?);
                let len = self.workspace.conflict_candidate_downstream.len();
                (len, len)
            }
            DownstreamSegment::PreFailed(DownstreamEvalError::Invariant) => {
                return Err(StepError::ConflictInvariantViolation);
            }
            DownstreamSegment::Obligated { raw_capacity, fill } => {
                // R2：F4 义务先兑现（与融合同位同序）——真实增长失败/注入
                // 先于 F4 后填充检查错误公开；随后消费填充结果。
                self.reserve_conflict_downstream_work(raw_capacity)?;
                match fill {
                    Ok(claims) => {
                        self.workspace
                            .conflict_downstream_work
                            .extend_from_slice(&claims);
                        let downstream_start = self.workspace.conflict_candidate_downstream.len();
                        self.reserve_conflict_downstream_pool(claims.len())?;
                        self.workspace
                            .conflict_candidate_downstream
                            .extend_from_slice(&claims);
                        (
                            downstream_start,
                            self.workspace.conflict_candidate_downstream.len(),
                        )
                    }
                    Err(DownstreamFillError::Invariant) => {
                        return Err(StepError::ConflictInvariantViolation);
                    }
                }
            }
            DownstreamSegment::Unmaterialized => {
                // 任务局部 claims 暂存不足：协调器同原语补算（F4 在内部；
                // 工作区已在分支前清空），随后原位 F3b 预留与接纳。
                match self.prepare_candidate_downstream(state, passage_range, gate_hop) {
                    Ok(()) => {
                        let downstream_start = self.workspace.conflict_candidate_downstream.len();
                        reserve(
                            &mut self.workspace.conflict_candidate_downstream,
                            self.workspace.conflict_downstream_work.len(),
                        )?;
                        self.workspace
                            .conflict_candidate_downstream
                            .extend_from_slice(&self.workspace.conflict_downstream_work);
                        (
                            downstream_start,
                            self.workspace.conflict_candidate_downstream.len(),
                        )
                    }
                    Err(ConflictAcquireError::NoGrant(reason)) => {
                        preflight_no_grant =
                            Some(map_acquire_error(ConflictAcquireError::NoGrant(reason))?);
                        let len = self.workspace.conflict_candidate_downstream.len();
                        (len, len)
                    }
                    Err(ConflictAcquireError::InvalidBundle | ConflictAcquireError::Capacity) => {
                        return Err(StepError::ConflictInvariantViolation);
                    }
                    Err(ConflictAcquireError::ScratchAllocFailed) => {
                        return Err(StepError::ConflictScratchAllocFailed);
                    }
                }
            }
        };
        let key = ConflictCandidateOrderKey::new(
            gate_kind,
            priority,
            eligibility.first_eligible_tick(),
            state
                .waiting_membership
                .map(|member| member.admission_sequence),
            update_sequence,
        );
        let anchor = ConflictRouteAnchor {
            route: passage_range.route(),
            maneuver_occurrence_index: passage_range.maneuver_occurrence_index(),
            hop: gate_hop,
        };
        let follower_min_gap_mm = self
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
            .ok_or(StepError::ConflictInvariantViolation)?
            .min_gap_mm();
        self.workspace.conflict_candidates.push(ConflictCandidate {
            vehicle,
            vehicle_update_sequence: update_sequence,
            key,
            anchor,
            passage: Some(stable_passage),
            passage_range: Some(passage_range),
            cells_start,
            cells_end,
            downstream_start,
            downstream_end,
            follower_min_gap_mm,
            waiting_zone,
            preflight_no_grant,
        });
        Ok(())
    }

    /// §2 #7/#11：horizon/preview 在门距早退之前已算出，无论报告形态都须
    /// 按 Active 紧凑位原位落缓存（与融合写点一致）。
    fn apply_conflict_cache_updates(
        &mut self,
        vehicle: VehicleHandle,
        cache_index: usize,
        cache: &CacheUpdates,
    ) {
        if cache.horizon.is_none() && cache.preview.is_none() {
            return;
        }
        if let Some(entry) = self
            .workspace
            .motion_cache
            .get_mut(cache_index)
            .filter(|entry| entry.vehicle == vehicle)
        {
            if let Some(horizon) = cache.horizon {
                entry.horizon = Some(horizon);
            }
            if let Some(preview) = cache.preview {
                entry.preview = Some(preview);
            }
        }
    }

    /// F1 位：cell 工作区真实预留（与融合同位同序）。注入仅在真实必要
    /// 增长（additional > capacity - len）时触发（R4），失败统一映射
    /// ConflictScratchAllocFailed。
    fn reserve_conflict_cell_work(&mut self, additional: usize) -> Result<(), StepError> {
        #[cfg(test)]
        if conflict_reserve_probe(
            ConflictReserveSite::CellWork,
            additional,
            self.workspace.conflict_cell_work.len(),
            self.workspace.conflict_cell_work.capacity(),
            conflict_injection::cell_work_reserve_injected(),
        ) {
            return Err(StepError::ConflictScratchAllocFailed);
        }
        reserve(&mut self.workspace.conflict_cell_work, additional)
    }

    /// F2 位：candidate_cells 真实预留（注入同 F1 的增长门控语义）。
    fn reserve_conflict_candidate_cells(&mut self, additional: usize) -> Result<(), StepError> {
        #[cfg(test)]
        if conflict_reserve_probe(
            ConflictReserveSite::CandidateCells,
            additional,
            self.workspace.conflict_candidate_cells.len(),
            self.workspace.conflict_candidate_cells.capacity(),
            conflict_injection::cells_reserve_injected(),
        ) {
            return Err(StepError::ConflictScratchAllocFailed);
        }
        reserve(&mut self.workspace.conflict_candidate_cells, additional)
    }

    /// F4 位：downstream 工作区真实预留（注入同 F1 的增长门控语义）。
    fn reserve_conflict_downstream_work(&mut self, additional: usize) -> Result<(), StepError> {
        #[cfg(test)]
        if conflict_reserve_probe(
            ConflictReserveSite::DownstreamWork,
            additional,
            self.workspace.conflict_downstream_work.len(),
            self.workspace.conflict_downstream_work.capacity(),
            conflict_injection::downstream_work_reserve_injected(),
        ) {
            return Err(StepError::ConflictScratchAllocFailed);
        }
        reserve(&mut self.workspace.conflict_downstream_work, additional)
    }

    /// F3b 位：candidate_downstream 真实预留（注入同 F1 的增长门控语义）。
    fn reserve_conflict_downstream_pool(&mut self, additional: usize) -> Result<(), StepError> {
        #[cfg(test)]
        if conflict_reserve_probe(
            ConflictReserveSite::DownstreamPool,
            additional,
            self.workspace.conflict_candidate_downstream.len(),
            self.workspace.conflict_candidate_downstream.capacity(),
            conflict_injection::downstream_pool_reserve_injected(),
        ) {
            return Err(StepError::ConflictScratchAllocFailed);
        }
        reserve(
            &mut self.workspace.conflict_candidate_downstream,
            additional,
        )
    }

    /// 按证明时长为活动车辆重建 approach frontier 所有者表。
    pub(crate) fn rebuild_conflict_frontier(&mut self) -> Result<(), StepError> {
        let Some(horizon_ms) = self.frontier_proof_horizon_ms() else {
            // 没有任何 gap profile 时不存在 lead frontier 查询；静态 Conflict cell
            // 仍可能被 protected/uncontrolled 或空 yield coverage 使用。
            return Ok(());
        };
        for sequence in 0..self.committed.live_order.len() {
            let vehicle = self.committed.live_order[sequence];
            let Some(state) = self.vehicle_state(vehicle).copied() else {
                continue;
            };
            if state.status != VehicleStatus::Active {
                continue;
            }
            let profile = self
                .binding
                .revision
                .traffic()
                .relations()
                .vehicle_profile(state.profile)
                .ok_or(StepError::ConflictInvariantViolation)?;
            let (compiled, mut conflict) = self
                .committed
                .prepare_conflict_for_route(
                    &mut self.derived,
                    &mut self.workspace.conflict,
                    state.route,
                )
                .ok_or(StepError::ConflictInvariantViolation)?;
            let first_conflict = compiled.conflicts.partition_point(|occurrence| {
                (
                    occurrence.entry.route_edge_index,
                    occurrence.entry.progress_mm,
                ) < (state.route_edge_index, state.progress_mm)
            });
            let conflict_count = compiled.conflicts.len();
            if first_conflict == conflict_count {
                continue;
            }
            let prepared_eta = crate::kernel::conflict::PreparedApproachEta::new(
                state.carry_um,
                state.speed_mm_s,
                profile.max_accel(),
                horizon_ms,
            );
            for occurrence_index in first_conflict..conflict_count {
                #[cfg(test)]
                crate::kernel::conflict::count_conflict_work(|counts| counts.visited_passages += 1);
                let Some((occurrence, exact_distance_mm)) = (|| {
                    let occurrence = *compiled.conflicts.get(occurrence_index)?;
                    let BoundedDistance::Finite(exact_distance_mm) =
                        distance_to_occurrence_progress(
                            &compiled.occurrence_segments,
                            &compiled.occurrence_offsets,
                            &compiled.segment_totals,
                            state.route_edge_index as usize,
                            state.progress_mm,
                            occurrence.entry.route_edge_index as usize,
                            occurrence.entry.progress_mm,
                        )?
                    else {
                        return None;
                    };
                    Some((occurrence, exact_distance_mm))
                })() else {
                    continue;
                };
                let estimate = prepared_eta.map_or(ApproachEstimate::Unprovable, |prepared| {
                    prepared.lower_bound(u64::from(exact_distance_mm))
                });
                if estimate == ApproachEstimate::OutsideHorizon {
                    // `conflicts` 按 route position 排列；更远 occurrence 的 directed
                    // lower-bound ETA 也在 proof horizon 外，无需扫完整路线后缀。
                    break;
                }
                conflict
                    .insert_approach_owner_reduced(
                        occurrence.address(),
                        vehicle,
                        u32::try_from(sequence)
                            .map_err(|_| StepError::ConflictInvariantViolation)?,
                        estimate,
                    )
                    .map_err(|error| match error {
                        ConflictAcquireError::ScratchAllocFailed => {
                            StepError::ConflictScratchAllocFailed
                        }
                        _ => StepError::ConflictInvariantViolation,
                    })?;
                #[cfg(test)]
                crate::kernel::conflict::count_conflict_work(|counts| counts.frontier_updates += 1);
            }
        }
        Ok(())
    }

    /// 求值单车在前视窗内到达的各 Gate，生成候选或记录无资源决定。
    pub(crate) fn evaluate_vehicle_gates(
        &mut self,
        state: VehicleState,
        update_sequence: u32,
        active_index: usize,
        delta_s: f32,
        tick: u64,
    ) -> Result<(), StepError> {
        let compiled =
            crate::kernel::tables::compiled_route_for_handle(&self.committed.routes, state.route)
                .ok_or(StepError::ConflictInvariantViolation)?;
        // Route cursor 将上一条边终点规范化为下一条边零点；该位置仍然位于
        // admission Gate boundary，不能因此跳过上一 hop 的正式仲裁。
        let first_possible_hop = if state.progress_mm == 0 && state.carry_um == 0 {
            state.route_edge_index.saturating_sub(1)
        } else {
            state.route_edge_index
        };
        let first_gate = compiled
            .gate_hops
            .partition_point(|hop| *hop < first_possible_hop);
        let Some(first_hop) = compiled.gate_hops.get(first_gate).copied() else {
            return Ok(());
        };
        let profile = self
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
            .ok_or(StepError::ConflictInvariantViolation)?;
        let cached = self
            .workspace
            .motion_cache
            .get(active_index)
            .copied()
            .filter(|entry| entry.vehicle == state.handle);
        let horizon = match cached.and_then(|entry| entry.horizon) {
            Some(horizon) => horizon,
            None => crate::kernel::tick::leader_query_horizon(state.speed_mm_s, profile, delta_s)
                .ok_or(StepError::NonFiniteMotion)?,
        };
        let distance = crate::kernel::tables::distance_to_occurrence_start(
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            state.route_edge_index as usize,
            state.progress_mm,
            first_hop as usize + 1,
        )
        .ok_or(StepError::ConflictInvariantViolation)?;
        let gate_count = compiled.gate_hops.len();
        if let Some(entry) = self
            .workspace
            .motion_cache
            .get_mut(active_index)
            .filter(|entry| entry.vehicle == state.handle)
        {
            entry.horizon = Some(horizon);
        }
        if !matches!(distance, BoundedDistance::Finite(mm) if mm <= horizon.front_query_mm) {
            return Ok(());
        }
        let waiting_plan = self
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
            });
        let waiting_stop = self.waiting_stop_for(&state)?;
        let motion = cached
            .and_then(|entry| entry.preview)
            .and_then(|preview| preview.with_waiting_stop(waiting_stop))
            .or_else(|| {
                self.read_view().preview_active_vehicle_with_waiting_stop(
                    state,
                    delta_s,
                    waiting_stop,
                    Some(horizon),
                )
            })
            .ok_or(StepError::NonFiniteMotion)?;
        let preview = motion.next;
        if let Some(entry) = self
            .workspace
            .motion_cache
            .get_mut(active_index)
            .filter(|entry| entry.vehicle == state.handle)
        {
            entry.preview = Some(motion);
        }
        for gate_index in first_gate..gate_count {
            let gate_hop = compiled.gate_hops[gate_index];
            let gate_edge = compiled.edges[gate_hop as usize];
            let gate_progress =
                self.binding.revision.traffic().lane_lengths_millimetres()[gate_edge.index()];
            let reaches_gate = preview.route_edge_index > gate_hop
                || (preview.route_edge_index == gate_hop && preview.progress_mm == gate_progress)
                || (state.route_edge_index == gate_hop && state.progress_mm == gate_progress);
            if !reaches_gate {
                break;
            }
            let waiting_index = compiled
                .waiting
                .partition_point(|entry| entry.entry_hop < gate_hop);
            let waiting = compiled
                .waiting
                .get(waiting_index)
                .copied()
                .filter(|entry| entry.entry_hop == gate_hop);
            if waiting.is_some_and(|entry| {
                state.waiting_membership.is_some_and(|member| {
                    member.waiting_zone == entry.zone && member.release_hop == entry.release_hop
                })
            }) {
                continue;
            }
            let maneuver_index = compiled
                .maneuvers
                .partition_point(|entry| entry.exit_route_edge_index <= gate_hop);
            compiled
                .maneuvers
                .get(maneuver_index)
                .filter(|entry| entry.entry_route_edge_index <= gate_hop)
                .ok_or(StepError::ConflictInvariantViolation)?;
            let anchor = ConflictRouteAnchor {
                route: state.route,
                maneuver_occurrence_index: u32::try_from(maneuver_index)
                    .map_err(|_| StepError::ConflictInvariantViolation)?,
                hop: gate_hop,
            };
            let range = compiled.conflict_gate_ranges[gate_hop as usize];
            let passage = if range.len != 0 {
                Some(
                    self.conflict_passage_occurrence_locator(state.route, range.start)
                        .ok_or(StepError::ConflictInvariantViolation)?,
                )
            } else {
                None
            };
            let gate = compiled.hop_gate[gate_hop as usize]
                .ok_or(StepError::ConflictInvariantViolation)?;
            let decision = self.gate_policy_decision(gate, state.profile);
            let outcome = match decision {
                GatePolicyDecision::DenyAndStop => Some(ConflictDecisionOutcome::NotEvaluated),
                GatePolicyDecision::Candidate(_) => {
                    match waiting
                        .and_then(|_| waiting_plan.filter(|plan| plan.entry_hop == gate_hop))
                    {
                        Some(plan) => match plan.decision {
                            crate::WaitingDecisionOutcome::Granted => None,
                            crate::WaitingDecisionOutcome::NoGrant(
                                crate::WaitingNoGrantReason::Capacity,
                            ) => Some(ConflictDecisionOutcome::NoGrant(
                                ConflictNoGrantReason::WaitingCapacity,
                            )),
                            crate::WaitingDecisionOutcome::NoGrant(
                                crate::WaitingNoGrantReason::PhysicalStorage,
                            ) => Some(ConflictDecisionOutcome::NoGrant(
                                ConflictNoGrantReason::WaitingPhysicalStorage,
                            )),
                            _ => return Err(StepError::WaitingInvariantViolation),
                        },
                        None if waiting.is_some() => Some(ConflictDecisionOutcome::NoGrant(
                            ConflictNoGrantReason::WaitingPhysicalStorage,
                        )),
                        None if range.len == 0 => Some(ConflictDecisionOutcome::NotRequired),
                        None => None,
                    }
                }
            };
            if let Some(outcome) = outcome {
                if waiting.is_none() && range.len == 0 {
                    // 无资源决定按最终运动范围输出，避免前方资源拒绝后仍报告未到达的 Gate。
                    if outcome == ConflictDecisionOutcome::NotRequired {
                        continue;
                    }
                    return Ok(());
                }
                reserve(&mut self.workspace.conflict_staged_decisions, 1)?;
                self.workspace
                    .conflict_staged_decisions
                    .push(ConflictDecision {
                        vehicle: state.handle,
                        vehicle_update_sequence: update_sequence,
                        anchor,
                        passage,
                        outcome,
                    });
                return Ok(());
            }
            let GatePolicyDecision::Candidate(kind) = decision else {
                unreachable!("denied Gate already produced a decision");
            };
            return self.prepare_resource_candidate(
                state,
                update_sequence,
                tick,
                EvaluatedGate {
                    anchor,
                    passage,
                    range,
                    kind,
                    waiting_zone: waiting.map(|entry| entry.zone),
                },
            );
        }
        Ok(())
    }

    fn prepare_resource_candidate(
        &mut self,
        state: VehicleState,
        update_sequence: u32,
        tick: u64,
        gate: EvaluatedGate,
    ) -> Result<(), StepError> {
        let EvaluatedGate {
            anchor,
            passage,
            range,
            kind,
            waiting_zone,
        } = gate;
        let gate_hop = anchor.hop;
        let maneuver_index = anchor.maneuver_occurrence_index;
        self.workspace.conflict_motion_by_vehicle[state.handle.index() as usize] =
            Some(ConflictMotionPlan {
                gate_hop,
                outcome: ConflictDecisionOutcome::NotEvaluated,
                grant_index: None,
            });
        let stable_passage = if range.len != 0 {
            passage.ok_or(StepError::ConflictInvariantViolation)?
        } else {
            // pure Waiting 没有 passage；eligibility 不持久化空 Conflict identity。
            let priority = None;
            let key = ConflictCandidateOrderKey::new(
                kind,
                priority,
                tick,
                state
                    .waiting_membership
                    .map(|member| member.admission_sequence),
                update_sequence,
            );
            self.workspace.conflict_candidates.push(ConflictCandidate {
                vehicle: state.handle,
                vehicle_update_sequence: update_sequence,
                key,
                anchor,
                passage: None,
                passage_range: None,
                cells_start: self.workspace.conflict_candidate_cells.len(),
                cells_end: self.workspace.conflict_candidate_cells.len(),
                downstream_start: self.workspace.conflict_candidate_downstream.len(),
                downstream_end: self.workspace.conflict_candidate_downstream.len(),
                follower_min_gap_mm: 0,
                waiting_zone,
                preflight_no_grant: None,
            });
            return Ok(());
        };

        let eligibility = ConflictEligibilityState::update(
            self.committed
                .conflict_eligibility
                .get(state.handle.index() as usize)
                .copied()
                .flatten(),
            stable_passage,
            true,
            tick,
        )
        .ok_or(StepError::ConflictInvariantViolation)?;
        self.workspace.conflict_next_eligibility[state.handle.index() as usize] = Some(eligibility);

        let passage_end = range
            .start
            .checked_add(range.len)
            .ok_or(StepError::ConflictInvariantViolation)?;
        if self
            .compiled_route(state.route)
            .and_then(|compiled| {
                compiled
                    .conflicts
                    .get(range.start as usize..passage_end as usize)
            })
            .is_none()
        {
            return Err(StepError::ConflictInvariantViolation);
        }
        let passage_range = ConflictPassageRange::new(
            state.route,
            maneuver_index,
            gate_hop,
            range.start,
            range.len,
        )
        .ok_or(StepError::ConflictInvariantViolation)?;
        let mut priority = None;
        let mut preflight_no_grant = None;
        let class = self
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
            .ok_or(StepError::ConflictInvariantViolation)?
            .class();
        self.workspace.conflict_cell_work.clear();
        #[cfg(test)]
        if conflict_reserve_probe(
            ConflictReserveSite::CellWork,
            range.len as usize,
            self.workspace.conflict_cell_work.len(),
            self.workspace.conflict_cell_work.capacity(),
            conflict_injection::cell_work_reserve_injected(),
        ) {
            return Err(StepError::ConflictScratchAllocFailed);
        }
        reserve(&mut self.workspace.conflict_cell_work, range.len as usize)?;
        for occurrence_index in range.start..passage_end {
            #[cfg(test)]
            crate::kernel::conflict::count_conflict_work(|counts| counts.visited_passages += 1);
            let occurrence = *self
                .compiled_route(state.route)
                .and_then(|compiled| compiled.conflicts.get(occurrence_index as usize))
                .ok_or(StepError::ConflictInvariantViolation)?;
            self.workspace.conflict_cell_work.push(occurrence.address());
            let policy = self
                .binding
                .policy_binding
                .policy(&self.binding.revision)
                .ok_or(StepError::ConflictInvariantViolation)?;
            let stream = policy
                .stream(occurrence.stream, class)
                .ok_or(StepError::ConflictInvariantViolation)?;
            priority = Some(priority.map_or(stream.priority(), |current: i32| {
                current.min(stream.priority())
            }));
            // 保护候选仍解析规则并收集全部冲突资源，只跳过让行间隙求值。
            // 占用、预留、下游净空和运动安全继续走共同的仲裁路径。
            if kind == GateCandidateKind::Protected {
                continue;
            }
            let (zone, targets) = policy
                .yield_targets(occurrence.stream, class, occurrence.passage_local_index)
                .ok_or(StepError::ConflictInvariantViolation)?;
            if zone != occurrence.zone {
                return Err(StepError::ConflictInvariantViolation);
            }
            let Some(gap_index) = stream.gap_profile_index() else {
                if !targets.is_empty() {
                    return Err(StepError::ConflictInvariantViolation);
                }
                continue;
            };
            let gap = *self
                .binding
                .policy_binding
                .gaps()
                .get(gap_index as usize)
                .ok_or(StepError::ConflictInvariantViolation)?;
            for target in targets {
                let address = ConflictPassageAddress::new(
                    occurrence.zone,
                    target.stream(),
                    target.passage_local_index(),
                );
                let outcome = self
                    .conflict_read()
                    .evaluate_yield_target(
                        state.handle,
                        address,
                        self.committed.time_ms,
                        gap.required_lag_ms(),
                        gap.required_lead_ms(),
                    )
                    .ok_or(StepError::ConflictInvariantViolation)?;
                if let Some(reason) = map_yield(outcome) {
                    preflight_no_grant = Some(preflight_no_grant.map_or(reason, |current| {
                        if no_grant_rank(reason) < no_grant_rank(current) {
                            reason
                        } else {
                            current
                        }
                    }));
                }
            }
        }
        self.workspace.conflict_cell_work.sort_unstable();
        self.workspace.conflict_cell_work.dedup();
        let cells_start = self.workspace.conflict_candidate_cells.len();
        #[cfg(test)]
        if conflict_reserve_probe(
            ConflictReserveSite::CandidateCells,
            self.workspace.conflict_cell_work.len(),
            self.workspace.conflict_candidate_cells.len(),
            self.workspace.conflict_candidate_cells.capacity(),
            conflict_injection::cells_reserve_injected(),
        ) {
            return Err(StepError::ConflictScratchAllocFailed);
        }
        reserve(
            &mut self.workspace.conflict_candidate_cells,
            self.workspace.conflict_cell_work.len(),
        )?;
        self.workspace
            .conflict_candidate_cells
            .extend_from_slice(&self.workspace.conflict_cell_work);

        self.workspace.conflict_downstream_work.clear();
        if preflight_no_grant.is_none() {
            match self.prepare_candidate_downstream(state, passage_range, gate_hop) {
                Ok(()) => {}
                Err(ConflictAcquireError::NoGrant(reason)) => {
                    preflight_no_grant =
                        Some(map_acquire_error(ConflictAcquireError::NoGrant(reason))?);
                }
                Err(ConflictAcquireError::InvalidBundle | ConflictAcquireError::Capacity) => {
                    return Err(StepError::ConflictInvariantViolation);
                }
                Err(ConflictAcquireError::ScratchAllocFailed) => {
                    return Err(StepError::ConflictScratchAllocFailed);
                }
            }
        }
        let downstream_start = self.workspace.conflict_candidate_downstream.len();
        #[cfg(test)]
        if conflict_reserve_probe(
            ConflictReserveSite::DownstreamPool,
            self.workspace.conflict_downstream_work.len(),
            self.workspace.conflict_candidate_downstream.len(),
            self.workspace.conflict_candidate_downstream.capacity(),
            conflict_injection::downstream_pool_reserve_injected(),
        ) {
            return Err(StepError::ConflictScratchAllocFailed);
        }
        reserve(
            &mut self.workspace.conflict_candidate_downstream,
            self.workspace.conflict_downstream_work.len(),
        )?;
        self.workspace
            .conflict_candidate_downstream
            .extend_from_slice(&self.workspace.conflict_downstream_work);

        let key = ConflictCandidateOrderKey::new(
            kind,
            priority,
            eligibility.first_eligible_tick(),
            state
                .waiting_membership
                .map(|member| member.admission_sequence),
            update_sequence,
        );
        self.workspace.conflict_candidates.push(ConflictCandidate {
            vehicle: state.handle,
            vehicle_update_sequence: update_sequence,
            key,
            anchor,
            passage: Some(stable_passage),
            passage_range: Some(passage_range),
            cells_start,
            cells_end: self.workspace.conflict_candidate_cells.len(),
            downstream_start,
            downstream_end: self.workspace.conflict_candidate_downstream.len(),
            follower_min_gap_mm: self
                .binding
                .revision
                .traffic()
                .relations()
                .vehicle_profile(state.profile)
                .ok_or(StepError::ConflictInvariantViolation)?
                .min_gap_mm(),
            waiting_zone,
            preflight_no_grant,
        });
        Ok(())
    }

    /// 为候选派生下游物理资源声明，并检查存储边界、前车间隙与停车锚点。
    pub(crate) fn prepare_candidate_downstream(
        &mut self,
        state: VehicleState,
        range: ConflictPassageRange,
        gate_hop: u32,
    ) -> Result<(), ConflictAcquireError> {
        let plan = self.reservation_downstream_claim_plan(range, state.length_mm)?;
        let compiled = self
            .compiled_route(state.route)
            .ok_or(ConflictAcquireError::InvalidBundle)?;
        let target = plan.target();
        let required = match distance_to_occurrence_progress(
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            state.route_edge_index as usize,
            state.progress_mm,
            target.route_edge_index() as usize,
            target.progress_mm(),
        ) {
            Some(BoundedDistance::Finite(value)) => value,
            Some(BoundedDistance::BeyondFinite) | None => {
                return Err(ConflictAcquireError::NoGrant(
                    ConflictResourceNoGrant::DownstreamStorageBoundary,
                ));
            }
        };
        let profile = self
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
            .ok_or(ConflictAcquireError::InvalidBundle)?;
        let leader_gap = self.derived.occupancy.leader_gap(
            state.handle,
            &compiled.edges,
            state.route_edge_index as usize,
            state.progress_mm,
            self.binding.revision.traffic().lane_lengths_millimetres(),
            LeaderQueryHorizon::new(u32::MAX, u32::MAX),
        );
        if leader_gap.is_some_and(|gap| {
            gap < i64::from(required).saturating_add(i64::from(profile.min_gap_mm()))
        }) {
            return Err(ConflictAcquireError::NoGrant(
                ConflictResourceNoGrant::DownstreamStorageBoundary,
            ));
        }
        if let Some(next_gate) = compiled
            .gate_hops
            .iter()
            .copied()
            .find(|hop| *hop > gate_hop)
            && target > gate_boundary(next_gate)?
        {
            return Err(ConflictAcquireError::NoGrant(
                ConflictResourceNoGrant::DownstreamStorageBoundary,
            ));
        }
        if let Some(waiting) = self
            .waiting_stop_for(&state)
            .map_err(|_| ConflictAcquireError::InvalidBundle)?
            && waiting.hop > gate_hop
            && target > gate_boundary(waiting.hop)?
        {
            return Err(ConflictAcquireError::NoGrant(
                ConflictResourceNoGrant::DownstreamStorageBoundary,
            ));
        }
        if let Some(ParkingBinding::Reserved(reservation)) =
            self.committed.parking.binding(state.handle)
        {
            if reservation.route() != state.route {
                return Err(ConflictAcquireError::InvalidBundle);
            }
            let (_, progress_mm) = self
                .reservation_anchor(reservation)
                .ok_or(ConflictAcquireError::InvalidBundle)?;
            let parking = crate::DownstreamRoutePoint::new(
                reservation.entry_route_occurrence(),
                progress_mm,
                0,
            )
            .ok_or(ConflictAcquireError::InvalidBundle)?;
            if target > parking {
                return Err(ConflictAcquireError::NoGrant(
                    ConflictResourceNoGrant::DownstreamStorageBoundary,
                ));
            }
        }
        #[cfg(test)]
        if conflict_reserve_probe(
            ConflictReserveSite::DownstreamWork,
            plan.raw_interval_capacity(),
            self.workspace.conflict_downstream_work.len(),
            self.workspace.conflict_downstream_work.capacity(),
            conflict_injection::downstream_work_reserve_injected(),
        ) {
            return Err(ConflictAcquireError::ScratchAllocFailed);
        }
        reserve(
            &mut self.workspace.conflict_downstream_work,
            plan.raw_interval_capacity(),
        )
        .map_err(|_| ConflictAcquireError::ScratchAllocFailed)?;
        let route = plan.route();
        let compiled = self
            .committed
            .routes
            .get(route.index() as usize)
            .filter(|slot| slot.generation == route.generation())
            .and_then(|slot| slot.compiled.as_ref())
            .ok_or(ConflictAcquireError::InvalidBundle)?;
        crate::kernel::conflict::derive_downstream_claims_from_plan(
            &compiled.edges,
            self.binding.revision.traffic().lane_lengths_millimetres(),
            plan.plan,
            &mut self.workspace.conflict_downstream_work,
        )
    }

    /// 按稳定顺序仲裁候选：授予组合资源并暂存本拍决定。
    pub(crate) fn acquire_conflict_candidates(&mut self, tick: u64) -> Result<(), StepError> {
        self.workspace.conflict_schedule.prepare(
            &self.workspace.conflict_candidates,
            &self.workspace.waiting_plan_by_vehicle,
        )?;
        self.prepare_waiting_dependencies(true)?;
        #[cfg(test)]
        crate::kernel::conflict::count_conflict_work(|counts| {
            counts.candidates += self.workspace.conflict_candidates.len();
        });
        while let Some(index) = self
            .workspace
            .conflict_schedule
            .next(&self.workspace.conflict_candidates)
        {
            let candidate = self.workspace.conflict_candidates[index];
            let entitlement = candidate
                .waiting_zone
                .map(|zone| WaitingAdmissionEntitlement::new(candidate.vehicle, zone, tick));
            let cycle_clear = if entitlement.is_some() {
                let plan = self.workspace.waiting_plan_by_vehicle
                    [candidate.vehicle.index() as usize]
                    .ok_or(StepError::WaitingInvariantViolation)?
                    .get() as usize
                    - 1;
                self.workspace.waiting_dependencies.stage(plan)?
            } else {
                true
            };
            // 本拍此前接受的候选也参与最高优先级 Occupied 判定。
            // yield target 与申请 passage 共用 zone，zone 摘要覆盖两者。
            let reason = if !cycle_clear {
                Some(ConflictNoGrantReason::WaitingCycle)
            } else if self.conflict_read().cells_unavailable(
                candidate.vehicle,
                &self.workspace.conflict_candidate_cells
                    [candidate.cells_start..candidate.cells_end],
            ) {
                Some(ConflictNoGrantReason::ConflictOccupied)
            } else {
                candidate.preflight_no_grant
            };
            let outcome = if let Some(reason) = reason {
                self.workspace.waiting_dependencies.rollback();
                ConflictDecisionOutcome::NoGrant(reason)
            } else {
                let result = self
                    .committed
                    .prepare_conflict(&mut self.derived, &mut self.workspace.conflict)
                    .try_acquire(
                        tick,
                        GrantResourceBundle {
                            owner: candidate.vehicle,
                            follower_min_gap_mm: candidate.follower_min_gap_mm,
                            cells: &self.workspace.conflict_candidate_cells
                                [candidate.cells_start..candidate.cells_end],
                            downstream: &self.workspace.conflict_candidate_downstream
                                [candidate.downstream_start..candidate.downstream_end],
                            waiting_entitlement: entitlement,
                        },
                    );
                match result {
                    Ok(grant) => {
                        self.workspace.conflict_motion_by_vehicle
                            [candidate.vehicle.index() as usize] = Some(ConflictMotionPlan {
                            gate_hop: candidate.anchor.hop,
                            outcome: ConflictDecisionOutcome::Granted,
                            grant_index: std::num::NonZeroU32::new(
                                u32::try_from(self.workspace.conflict_grants.len())
                                    .map_err(|_| StepError::ConflictInvariantViolation)?
                                    + 1,
                            ),
                        });
                        self.activate_waiting_claim(candidate.vehicle, candidate.anchor.hop)?;
                        if entitlement.is_some() {
                            self.workspace.waiting_dependencies.accept();
                        }
                        self.workspace.conflict_grants.push(PreparedConflictGrant {
                            vehicle: candidate.vehicle,
                            gate_hop: candidate.anchor.hop,
                            passage_range: candidate.passage_range,
                            grant,
                        });
                        ConflictDecisionOutcome::Granted
                    }
                    Err(error) => {
                        self.workspace.waiting_dependencies.rollback();
                        ConflictDecisionOutcome::NoGrant(map_acquire_error(error)?)
                    }
                }
            };
            self.workspace.conflict_motion_by_vehicle[candidate.vehicle.index() as usize]
                .as_mut()
                .expect("candidate has motion slot")
                .outcome = outcome;
            self.workspace
                .conflict_staged_decisions
                .push(ConflictDecision {
                    vehicle: candidate.vehicle,
                    vehicle_update_sequence: candidate.vehicle_update_sequence,
                    anchor: candidate.anchor,
                    passage: candidate.passage,
                    outcome,
                });
        }
        Ok(())
    }

    /// 资源仲裁完成后，以最终可达范围输出无资源 Gate 决定；同样覆盖本拍已有 reservation 的车辆。
    pub(crate) fn stage_resource_free_gate_decisions(
        &mut self,
        next: &VehicleState,
        update_sequence: u32,
    ) -> Result<(), StepError> {
        let previous = self
            .vehicle_state(next.handle)
            .ok_or(StepError::ConflictInvariantViolation)?;
        let first_hop = if previous.progress_mm == 0 && previous.carry_um == 0 {
            previous.route_edge_index.saturating_sub(1)
        } else {
            previous.route_edge_index
        };
        let compiled =
            crate::kernel::tables::compiled_route_for_handle(&self.committed.routes, next.route)
                .ok_or(StepError::ConflictInvariantViolation)?;
        let first = compiled.gate_hops.partition_point(|hop| *hop < first_hop);
        let last = compiled
            .gate_hops
            .partition_point(|hop| *hop <= next.route_edge_index);
        for index in first..last {
            let hop = compiled.gate_hops[index];
            let edge = compiled.edges[hop as usize];
            if next.route_edge_index == hop
                && next.progress_mm
                    < self.binding.revision.traffic().lane_lengths_millimetres()[edge.index()]
            {
                break;
            }
            let waiting = compiled
                .waiting
                .partition_point(|entry| entry.entry_hop < hop);
            if compiled.conflict_gate_ranges[hop as usize].len != 0
                || compiled
                    .waiting
                    .get(waiting)
                    .is_some_and(|entry| entry.entry_hop == hop)
            {
                continue;
            }
            let maneuver = compiled
                .maneuvers
                .partition_point(|entry| entry.exit_route_edge_index <= hop);
            compiled
                .maneuvers
                .get(maneuver)
                .filter(|entry| entry.entry_route_edge_index <= hop)
                .ok_or(StepError::ConflictInvariantViolation)?;
            let gate =
                compiled.hop_gate[hop as usize].ok_or(StepError::ConflictInvariantViolation)?;
            let outcome = match self.gate_policy_decision(gate, next.profile) {
                GatePolicyDecision::DenyAndStop => ConflictDecisionOutcome::NotEvaluated,
                GatePolicyDecision::Candidate(_) => ConflictDecisionOutcome::NotRequired,
            };
            reserve(&mut self.workspace.conflict_staged_decisions, 1)?;
            self.workspace
                .conflict_staged_decisions
                .push(ConflictDecision {
                    vehicle: next.handle,
                    vehicle_update_sequence: update_sequence,
                    anchor: ConflictRouteAnchor {
                        route: next.route,
                        maneuver_occurrence_index: u32::try_from(maneuver)
                            .map_err(|_| StepError::ConflictInvariantViolation)?,
                        hop,
                    },
                    passage: None,
                    outcome,
                });
        }
        Ok(())
    }

    /// 在 mutation boundary 前验证并定稿本拍 Conflict 提交计划。
    pub(crate) fn finalize_conflict_step(
        &mut self,
        updates: &mut [(usize, VehicleState)],
    ) -> Result<(), StepError> {
        self.workspace.conflict_passage_transitions.clear();
        self.workspace.conflict_changed_owners.clear();
        let journal_armed = self.journal_armed;
        let transition_capacity = self
            .workspace
            .conflict_grants
            .iter()
            .filter_map(|grant| grant.passage_range)
            .map(|range| range.passage_count() as usize)
            .chain(
                updates
                    .iter()
                    .filter_map(|(_, next)| self.conflict_reservation(next.handle))
                    .map(|reservation| reservation.passage_range().passage_count() as usize),
            )
            .sum();
        reserve(
            &mut self.workspace.conflict_passage_transitions,
            transition_capacity,
        )?;
        if journal_armed {
            reserve(
                &mut self.workspace.conflict_changed_owners,
                transition_capacity
                    .checked_add(self.workspace.conflict_grants.len())
                    .ok_or(StepError::ConflictInvariantViolation)?,
            )?;
        }

        // live_order 同时包含 parked/completed；双游标合并两份有序列表，保留正式更新序号。
        let mut update_sequence = 0;
        for (_, next) in updates.iter_mut() {
            while self.committed.live_order.get(update_sequence) != Some(&next.handle) {
                update_sequence += 1;
                if update_sequence >= self.committed.live_order.len() {
                    return Err(StepError::ConflictInvariantViolation);
                }
            }
            self.stage_resource_free_gate_decisions(
                next,
                u32::try_from(update_sequence)
                    .map_err(|_| StepError::ConflictInvariantViolation)?,
            )?;
            #[cfg(test)]
            crate::kernel::conflict::count_conflict_work(|counts| {
                counts.vehicle_grant_lookups += 1
            });
            let grant_index = self.workspace.conflict_motion_by_vehicle
                [next.handle.index() as usize]
                .and_then(|plan| plan.grant_index)
                .map(|index| index.get() as usize - 1);
            let crossed = grant_index.is_some_and(|index| {
                next.route_edge_index > self.workspace.conflict_grants[index].gate_hop
            });
            if crossed {
                self.workspace.conflict_next_eligibility[next.handle.index() as usize] = None;
            }
            let range = grant_index
                .filter(|_| crossed)
                .and_then(|index| self.workspace.conflict_grants[index].passage_range)
                .or_else(|| {
                    self.conflict_reservation(next.handle)
                        .map(|reservation| reservation.passage_range())
                });
            let all_clear = match range {
                Some(range) => self.stage_passage_transitions(*next, range)?,
                None => true,
            };
            if all_clear {
                next.maneuver_traversal = self.derive_waiting_traversal(*next)?;
            } else {
                next.maneuver_traversal = Some(ManeuverTraversalState {
                    route: next.route,
                    maneuver_occurrence_index: range
                        .expect("uncleared coverage")
                        .maneuver_occurrence_index(),
                    phase: ManeuverTraversalPhase::Clearing {
                        admission_gate_hop: range.expect("uncleared coverage").admission_gate_hop(),
                    },
                });
            }
            if (!all_clear && next.waiting_membership.is_some())
                || (next.status != VehicleStatus::Active
                    && (next.maneuver_traversal.is_some() || next.waiting_membership.is_some()))
            {
                return Err(StepError::ConflictInvariantViolation);
            }
            let slot = next.handle.index() as usize;
            if let Some(eligibility) = self.workspace.conflict_next_eligibility[slot]
                && !self.conflict_eligibility_valid_with_signals(
                    next,
                    eligibility,
                    &self.workspace.next_signal_aspects,
                )
            {
                self.workspace.conflict_next_eligibility[slot] = None;
            }
        }

        // 在 mutation boundary 前验证完整提交计划；失败仍只丢弃 tick-local staging。
        for prepared in &self.workspace.conflict_grants {
            #[cfg(test)]
            crate::kernel::conflict::count_conflict_work(|counts| counts.grant_update_lookups += 1);
            let index = self.workspace.next_state_by_vehicle[prepared.vehicle.index() as usize]
                .checked_sub(1)
                .ok_or(StepError::ConflictInvariantViolation)? as usize;
            let next = updates
                .get(index)
                .map(|(_, next)| *next)
                .filter(|next| next.handle == prepared.vehicle)
                .ok_or(StepError::ConflictInvariantViolation)?;
            if next.route_edge_index <= prepared.gate_hop {
                continue;
            }
            match prepared.passage_range {
                Some(range) => self
                    .conflict_read()
                    .validate_gate_crossing(&prepared.grant, range)
                    .map_err(|_| StepError::ConflictInvariantViolation)?,
                None => self
                    .conflict_read()
                    .validate_pure_waiting_grant(&prepared.grant)
                    .map_err(|_| StepError::ConflictInvariantViolation)?,
            }
        }
        for transition in self.workspace.conflict_passage_transitions.iter().copied() {
            if !self
                .conflict_read()
                .passage_transition_valid_after_staged_commits(
                    transition.vehicle,
                    transition.address,
                    transition.enter,
                    transition.clear,
                )
            {
                return Err(StepError::ConflictInvariantViolation);
            }
        }

        self.workspace
            .conflict_staged_decisions
            .sort_unstable_by_key(|decision| {
                (
                    decision.vehicle_update_sequence,
                    decision.anchor.hop,
                    decision.passage.map(|passage| passage.address()),
                )
            });

        Ok(())
    }

    /// 按车辆新位置暂存冲突通行段的进入/清空转移；返回是否全部清空。
    pub(crate) fn stage_passage_transitions(
        &mut self,
        next: VehicleState,
        range: ConflictPassageRange,
    ) -> Result<bool, StepError> {
        let end = range
            .first_conflict_occurrence_index()
            .checked_add(range.passage_count())
            .ok_or(StepError::ConflictInvariantViolation)?;
        let mut all_clear = true;
        for index in range.first_conflict_occurrence_index()..end {
            let occurrence = *self
                .compiled_route(range.route())
                .and_then(|compiled| compiled.conflicts.get(index as usize))
                .ok_or(StepError::ConflictInvariantViolation)?;
            let stage = self
                .conflict_read()
                .passage_stage(next.handle, occurrence.address());
            let front_reached = route_front_at_or_beyond(
                next,
                occurrence.entry.route_edge_index,
                occurrence.entry.progress_mm,
            );
            let rear_cleared = crate::kernel::tables::vehicle_rear_at_or_beyond(
                self.binding.revision.traffic().lane_lengths_millimetres(),
                &self
                    .compiled_route(range.route())
                    .ok_or(StepError::ConflictInvariantViolation)?
                    .edges,
                next.route_edge_index as usize,
                next.progress_mm,
                next.carry_um,
                next.length_mm,
                occurrence.clearance,
            )
            .ok_or(StepError::ConflictInvariantViolation)?;
            let already_cleared = matches!(
                stage,
                Some(crate::kernel::conflict::ConflictPassageStage::Cleared)
            );
            let occupied = matches!(
                stage,
                Some(crate::kernel::conflict::ConflictPassageStage::Occupied)
            );
            let enter = front_reached && !already_cleared && !occupied;
            let clear = rear_cleared && !already_cleared;
            if enter || clear {
                self.workspace
                    .conflict_passage_transitions
                    .push(ConflictPassageTransition {
                        vehicle: next.handle,
                        occurrence_index: index,
                        address: occurrence.address(),
                        enter,
                        clear,
                    });
            }
            all_clear &= already_cleared || rear_cleared;
        }
        Ok(all_clear)
    }
}

impl crate::kernel::phase::CommittedStateMut<'_> {
    /// mutation boundary：提交已验证的 grant 与冲突通行段转移。
    pub(crate) fn commit_conflict_transitions(
        &mut self,
        updates: &[(usize, VehicleState)],
        post_step_time_ms: u64,
    ) {
        let journal_armed = self.journal.is_some();
        // mutation boundary：下面只执行上方已经完整验证且已预留容量的操作。
        for prepared in self.workspace.conflict_grants.drain(..) {
            #[cfg(test)]
            crate::kernel::conflict::count_conflict_work(|counts| counts.grant_update_lookups += 1);
            let index = self.workspace.next_state_by_vehicle[prepared.vehicle.index() as usize]
                as usize
                - 1;
            let next = updates[index].1;
            debug_assert_eq!(next.handle, prepared.vehicle);
            if next.route_edge_index <= prepared.gate_hop {
                continue;
            }
            if journal_armed && prepared.passage_range.is_some() {
                self.workspace
                    .conflict_changed_owners
                    .push(prepared.vehicle);
            }
            match prepared.passage_range {
                Some(range) => {
                    crate::kernel::conflict::ConflictWrite::new(
                        &mut self.committed.conflict,
                        &mut self.derived.conflict,
                        &mut self.workspace.conflict,
                    )
                    .commit_gate_crossing_deferred(prepared.grant, range)
                    .expect("prevalidated Conflict crossing commit");
                }
                None => {
                    crate::kernel::conflict::ConflictWrite::new(
                        &mut self.committed.conflict,
                        &mut self.derived.conflict,
                        &mut self.workspace.conflict,
                    )
                    .consume_pure_waiting_grant(prepared.grant)
                    .expect("prevalidated pure Waiting grant commit");
                }
            }
        }
        crate::kernel::conflict::ConflictWrite::new(
            &mut self.committed.conflict,
            &mut self.derived.conflict,
            &mut self.workspace.conflict,
        )
        .expire_unconsumed_grants();
        let mut released = false;
        for transition in self.workspace.conflict_passage_transitions.iter().copied() {
            if journal_armed && (transition.enter || transition.clear) {
                self.workspace
                    .conflict_changed_owners
                    .push(transition.vehicle);
            }
            if transition.enter {
                assert!(
                    crate::kernel::conflict::ConflictWrite::new(
                        &mut self.committed.conflict,
                        &mut self.derived.conflict,
                        &mut self.workspace.conflict
                    )
                    .enter_passage(transition.vehicle, transition.address),
                    "prevalidated Conflict passage entry"
                );
            }
            if transition.clear {
                released |= crate::kernel::conflict::ConflictWrite::new(
                    &mut self.committed.conflict,
                    &mut self.derived.conflict,
                    &mut self.workspace.conflict,
                )
                .clear_passage_deferred(transition.vehicle, transition.address, post_step_time_ms)
                .expect("prevalidated Conflict passage clearance")
                    == crate::kernel::conflict::ConflictClearOutcome::ReservationReleased;
            }
        }
        if released {
            crate::kernel::conflict::ConflictWrite::new(
                &mut self.committed.conflict,
                &mut self.derived.conflict,
                &mut self.workspace.conflict,
            )
            .finish_releases();
        }
        if journal_armed {
            self.workspace
                .conflict_changed_owners
                .sort_unstable_by_key(|owner| (owner.index(), owner.generation()));
            self.workspace.conflict_changed_owners.dedup();
        }
    }

    /// 提交本拍 Conflict 资格表与决定批次到已发布状态。
    pub(crate) fn commit_conflict_step(&mut self) {
        self.committed.conflict_eligibility.clear();
        self.committed
            .conflict_eligibility
            .extend_from_slice(&self.workspace.conflict_next_eligibility);
        self.normalize_conflict_eligibility();
        core::mem::swap(
            &mut self.committed.latest_conflict_decisions,
            &mut self.workspace.conflict_staged_decisions,
        );
    }

    /// 把本拍 Conflict 资格、权威与清空记录写入迁移日志。
    pub(crate) fn write_conflict_tick_journal(
        &self,
        journal: &mut MigrationDeltaJournal,
        updates: &[(usize, VehicleState)],
    ) {
        for (slot, _) in updates {
            let previous = self
                .committed
                .conflict_eligibility
                .get(*slot)
                .copied()
                .flatten();
            let next = self
                .workspace
                .conflict_next_eligibility
                .get(*slot)
                .copied()
                .flatten();
            if previous == next {
                continue;
            }
            let owner = self
                .committed
                .vehicles
                .get(*slot)
                .and_then(|slot| slot.state.as_ref())
                .map(|state| state.handle)
                .expect("successful tick update retains its vehicle slot");
            let encoded = next
                .map(|value| {
                    self.conflict_journal_locator(
                        value.locator().route(),
                        value.locator().conflict_occurrence_index(),
                    )
                    .map(|locator| (locator, value.first_eligible_tick()))
                })
                .transpose()
                .expect("committed Conflict eligibility resolves its compiled occurrence");
            journal.tick_conflict_eligibility(owner, encoded);
        }

        for owner in self.workspace.conflict_changed_owners.iter().copied() {
            let Some(reservation) = self.conflict_read().reservation(owner) else {
                journal.tick_conflict_authority_absent(owner);
                continue;
            };
            let range = reservation.passage_range();
            let start = range.first_conflict_occurrence_index() as usize;
            let end = start
                .checked_add(range.passage_count() as usize)
                .expect("validated Conflict reservation range does not overflow");
            let compiled = self
                .compiled_route(range.route())
                .expect("validated Conflict reservation retains its route");
            let occurrences = compiled
                .conflicts
                .get(start..end)
                .expect("validated Conflict reservation range resolves");
            let cells = occurrences.iter().enumerate().map(|(offset, occurrence)| {
                let index = u32::try_from(start + offset)
                    .expect("compiled Conflict occurrence index fits u32");
                let locator = ConflictOccurrenceJournalLocator {
                    route: range.route(),
                    stream: occurrence.stream.raw(),
                    zone: occurrence.zone.raw(),
                    passage_local_index: occurrence.passage_local_index,
                    entry_route_edge_index: occurrence.entry.route_edge_index,
                    entry_progress_mm: occurrence.entry.progress_mm,
                    clearance_route_edge_index: occurrence.clearance.route_edge_index,
                    clearance_progress_mm: occurrence.clearance.progress_mm,
                };
                let stage = self
                    .conflict_read()
                    .passage_stage(owner, compiled.conflicts[index as usize].address())
                    .expect("reservation range owns every encoded Conflict cell")
                    .journal_tag();
                (locator, stage)
            });
            journal.tick_conflict_authority(owner, reservation.acquired_tick(), cells);
        }

        for transition in self
            .workspace
            .conflict_passage_transitions
            .iter()
            .filter(|transition| transition.clear)
        {
            let reference = self
                .conflict_read()
                .lag_reference(transition.address)
                .expect("validated clear transition retains its Conflict cell");
            journal.tick_conflict_lag(transition.address, reference);
        }
    }

    /// 把路线内的冲突出现项编码为迁移日志的稳定 locator。
    pub(crate) fn conflict_journal_locator(
        &self,
        route: RouteHandle,
        conflict_occurrence_index: u32,
    ) -> Result<ConflictOccurrenceJournalLocator, ()> {
        self.read_view()
            .conflict_journal_locator(route, conflict_occurrence_index)
    }
}

fn route_front_at_or_beyond(state: VehicleState, edge: u32, progress_mm: u32) -> bool {
    (state.route_edge_index, state.progress_mm, state.carry_um) >= (edge, progress_mm, 0)
}

#[allow(dead_code)]
fn _profile_type_check(_: VehicleProfileOrdinal, _: ApproachEstimate) {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Instant;

    use super::*;
    use crate::TickInput;
    use crate::admin::cutover_migration::tests::{conflict_scale_revision, conflict_scale_world};
    use crate::kernel::conflict::{
        ApproachFrontierCell, conflict_work_counts, reset_conflict_work_counts,
    };

    #[test]
    fn rejected_waiting_bundle_has_no_claim_counter_or_granted_output() {
        let mut world = crate::kernel::waiting::tests::multi_gate_world(1);
        assert_eq!(
            world
                .state
                .workspace
                .conflict_schedule
                .retained_logical_bytes(),
            0
        );
        world.state.rebuild_occupancy_index().unwrap();
        world.state.prepare_waiting_step(0.1).unwrap();
        world
            .state
            .prepare_conflict_candidates(0.1, 1, None)
            .unwrap();
        assert_eq!(world.state.workspace.conflict_candidates.len(), 1);
        // 单独验证组合仲裁拒绝与 Waiting 发布的接缝；SCC 本身由 arbiter 测试覆盖。
        world.state.workspace.conflict_candidates[0].preflight_no_grant =
            Some(ConflictNoGrantReason::WaitingCycle);
        world.state.acquire_conflict_candidates(1).unwrap();
        assert!(
            world
                .state
                .workspace
                .conflict_schedule
                .retained_logical_bytes()
                > 0
        );
        assert!(world.state.workspace.waiting_claims.is_empty());
        let candidate = world.state.workspace.conflict_candidates[0];
        let old = world.vehicle(candidate.vehicle).unwrap();
        let phase = world.state.step_workspace();
        let next = phase
            .read_view()
            .advance_active_vehicle_with_waiting_stop(
                old,
                0.1,
                phase.waiting_stop_for(&old).unwrap(),
                phase.conflict_stop_for(&old).unwrap(),
            )
            .unwrap();
        let mut updates = [(candidate.vehicle.index() as usize, next)];
        world.state.finalize_waiting_step(&mut updates).unwrap();
        world.state.finalize_conflict_step(&mut updates).unwrap();
        world.state.finalize_waiting_outputs(&updates, 1).unwrap();
        assert_eq!(
            world.state.workspace.waiting_staged_decisions[0].outcome(),
            crate::WaitingDecisionOutcome::NoGrant(crate::WaitingNoGrantReason::CombinedResource(
                ConflictNoGrantReason::WaitingCycle
            ))
        );
        assert!(updates[0].1.waiting_membership.is_none());
        assert!(world.state.workspace.staged_transition_events.is_empty());
        assert!(
            world
                .state
                .committed
                .waiting_zones
                .iter()
                .all(|zone| zone.next_admission_sequence == 0)
        );
    }

    fn percentile_samples(world: &mut TrafficWorld) -> (u128, u128) {
        let mut samples = Vec::with_capacity(21);
        for sample in 0..24 {
            let started = Instant::now();
            world
                .step(TickInput::new(
                    world.state.binding.config.fixed_delta_time_ms(),
                ))
                .expect("Conflict scale tick");
            if sample >= 3 {
                samples.push(started.elapsed().as_nanos());
            }
        }
        samples.sort_unstable();
        (samples[10], samples[19])
    }

    fn arbitration_samples(world: &mut TrafficWorld) -> (u128, u128) {
        let mut samples = Vec::with_capacity(21);
        for sample in 0..24 {
            let started = Instant::now();
            world
                .state
                .prepare_conflict_step(0.004, world.tick_index() + 1, None)
                .expect("Conflict arbitration sample");
            if sample >= 3 {
                samples.push(started.elapsed().as_nanos());
            }
        }
        samples.sort_unstable();
        (samples[10], samples[19])
    }

    #[test]
    fn eta_preparation_count_does_not_grow_with_passages() {
        use laneflow_static_contract::{
            EntityKind, ParticipantStreamOrdinal, RightOfWayPolicySetId,
        };
        for multiple_passages in [false, true] {
            let revision = crate::admin::cutover_migration::tests::conflict_frontier_revision(
                multiple_passages,
                false,
            );
            let stream = (0..3)
                .filter_map(|raw| {
                    revision
                        .conflict()
                        .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
                })
                .max_by_key(|stream| stream.passages().len())
                .unwrap();
            let edges = revision
                .traffic()
                .maneuvers()
                .maneuver_path(stream.maneuver_path())
                .unwrap()
                .edges()
                .to_vec();
            let entry_length = revision.traffic().lane_lengths_millimetres()[edges[0].index()];
            let origin = *revision.canonical_origin();
            let policy = RightOfWayPolicySetId::from_untyped(
                laneflow_compiler::derive_canonical_stable_id_v1(
                    EntityKind::RightOfWayPolicySet,
                    "city/runtime-live-conflict-cutover",
                    "policy",
                    &laneflow_compiler::CompileLimits::single_network_1m_v2(),
                )
                .unwrap(),
            );
            let mut world = TrafficWorld::install(
                revision,
                crate::WorldConfig::new(1, 1, 3, 3, 4),
                crate::ExecutionConfig::new(std::num::NonZeroU32::MIN),
                crate::CommittedNetworkSource::Published {
                    reference: crate::PublishedLfcaReference::new(
                        "fixture://eta-count",
                        origin.canonical_artifact_digest(),
                        origin.canonical_artifact_byte_length(),
                        origin.network_revision(),
                    )
                    .unwrap(),
                },
                676,
                crate::WorldPolicySelection::Pinned(crate::PolicyPin { policy }),
            )
            .unwrap();
            let route = world
                .register_route(crate::RouteRegisterInput::new(edges))
                .unwrap();
            let vehicle = world
                .spawn_vehicle(crate::VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    entry_length - 1,
                    10_000,
                ))
                .unwrap();
            reset_conflict_work_counts();
            world
                .state
                .step_workspace()
                .rebuild_conflict_frontier()
                .unwrap();
            let counts = conflict_work_counts();
            assert_eq!(counts.eta_preparations, 1);
            assert_eq!(
                counts.eta_distance_evaluations,
                if multiple_passages { 3 } else { 1 }
            );
            // 仅把 cursor 移至没有 future entry 的出口，隔离 frontier 准备的跳过条件。
            let state = world.state.committed.vehicles[vehicle.index() as usize]
                .state
                .as_mut()
                .unwrap();
            state.route_edge_index = 2;
            state.progress_mm = 10_000;
            reset_conflict_work_counts();
            world
                .state
                .step_workspace()
                .rebuild_conflict_frontier()
                .unwrap();
            assert_eq!(conflict_work_counts().eta_preparations, 0);
        }
    }

    #[test]
    fn eta_preparation_is_per_vehicle_and_skips_routes_without_future_conflicts() {
        let revision = conflict_scale_revision();
        for vehicles in [1, 4, 16, 64] {
            let mut world = conflict_scale_world(Arc::clone(&revision), vehicles);
            reset_conflict_work_counts();
            world
                .state
                .step_workspace()
                .rebuild_conflict_frontier()
                .unwrap();
            let counts = conflict_work_counts();
            assert_eq!(counts.eta_preparations, vehicles as usize, "{counts:?}");
            assert_eq!(
                counts.eta_distance_evaluations, vehicles as usize,
                "{counts:?}"
            );
            for handle in world.live_vehicles().to_vec() {
                world.despawn_vehicle(handle).unwrap();
            }
            reset_conflict_work_counts();
            world
                .state
                .step_workspace()
                .rebuild_conflict_frontier()
                .unwrap();
            assert_eq!(conflict_work_counts().eta_preparations, 0);
        }
    }
    #[test]
    fn conflict_scale_tick_keeps_route_visits_bounded_and_state_valid() {
        let revision = conflict_scale_revision();
        let mut world = conflict_scale_world(revision, 600);
        reset_conflict_work_counts();
        world.step(TickInput::new(4)).expect("Conflict scale tick");
        let work = conflict_work_counts();
        assert!(
            work.candidates > 0,
            "Conflict scale work: {work:?}, decisions: {:?}",
            world.latest_conflict_decisions()
        );
        assert!(work.visited_passages <= 1_800);
        assert_eq!(world.state.conflict_read().cell_count(), 2);
        assert!(world.state.conflict_state_valid());
    }

    #[test]
    #[ignore = "manual release-mode Conflict 10k/100k scale evidence"]
    fn conflict_10k_100k_scale_evidence() {
        let revision = conflict_scale_revision();

        let mut product = conflict_scale_world(Arc::clone(&revision), 10_000);
        product.step(TickInput::new(4)).expect("10k warm tick");
        let retained_10k = product.state.conflict_retained_logical_bytes();
        let (arbitration_p50_ns, arbitration_p95_ns) = arbitration_samples(&mut product);
        let (tick_p50_ns, tick_p95_ns) = percentile_samples(&mut product);
        assert!(product.state.conflict_state_valid());

        let mut scale = conflict_scale_world(revision, 100_000);
        reset_conflict_work_counts();
        scale.step(TickInput::new(4)).expect("100k evidence tick");
        let work = conflict_work_counts();
        let retained_100k = scale.state.conflict_retained_logical_bytes();
        let top_two_cells = scale.state.conflict_read().cell_count();
        let top_two_bytes = top_two_cells * core::mem::size_of::<ApproachFrontierCell>();
        assert!(scale.state.conflict_state_valid());
        assert!(work.visited_passages <= 300_000);
        assert!(retained_100k <= retained_10k.saturating_mul(11));

        eprintln!(
            "conflict-g2-scale-evidence 10k_tick_p50_ns={tick_p50_ns} \
             10k_tick_p95_ns={tick_p95_ns} 10k_arbitration_p50_ns={arbitration_p50_ns} \
             10k_arbitration_p95_ns={arbitration_p95_ns} \
             10k_retained_bytes={retained_10k} 100k_retained_bytes={retained_100k} \
             100k_visited_passages={} 100k_frontier_updates={} \
             100k_top_two_cells={top_two_cells} 100k_top_two_bytes={top_two_bytes} \
             100k_candidates={} 100k_yield_queries={} 100k_cell_claim_queries={} \
             100k_downstream_claim_queries={} 100k_collision_rejections={} \
             100k_wait_for_nodes={} 100k_wait_for_edges={} 100k_wait_for_visits={}",
            work.visited_passages,
            work.frontier_updates,
            work.candidates,
            work.yield_queries,
            work.cell_claim_queries,
            work.downstream_claim_queries,
            work.collision_rejections,
            work.wait_for_nodes,
            work.wait_for_edges,
            work.wait_for_visits,
        );
    }
}

// ---------------------------------------------------------------------------
// #706 增量 D：P3 候选字段多段原语 + 真实分发。逐车计算在冻结视图上按
// 串行同序求值（多段报告，不抹平状态）；协调器按 live×gate 原序消费并
// 施加全部共享写入与真实预留（F1/F2/F3b/F4/staged 原位）。rebuild
// frontier 与 P4 acquire 保持协调器串行，一行不动。
// ---------------------------------------------------------------------------

/// P3 分发阈值：候选eligible 工作集低于该值时融合执行；初版保守选择
///（与 P2/P5 同值、独立常量），待增量 E 证据登记后校准。
const CONFLICT_DISPATCH_MIN_ACTIVE: usize = 1_024;

/// P3 本阶段谁执行的计数证据（融合/分发/回退互斥）。
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ConflictPathCounts {
    pub(crate) dispatched: usize,
    pub(crate) fused: usize,
    pub(crate) slot_fallback: usize,
}

#[cfg(test)]
pub(crate) fn conflict_path_counts() -> ConflictPathCounts {
    CONFLICT_PATH_COUNTS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn count_conflict_path(update: impl FnOnce(&mut ConflictPathCounts)) {
    CONFLICT_PATH_COUNTS.with(|counts| {
        let mut value = counts.get();
        update(&mut value);
        counts.set(value);
    });
}

#[cfg(test)]
thread_local! {
    static CONFLICT_PATH_COUNTS: std::cell::Cell<ConflictPathCounts> = const { std::cell::Cell::new(ConflictPathCounts { dispatched: 0, fused: 0, slot_fallback: 0 }) };
    static CONFLICT_FORCE_DISPATCH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// 组合矩阵融合侧入口：强制 P3 保持融合（fuse 优先于 force）。
    static CONFLICT_FORCE_FUSE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static CONFLICT_SLOT_GAP: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    static CONFLICT_WORK_DIAGNOSTICS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static LAST_CONFLICT_DISPATCH_STATS: std::cell::Cell<Option<crate::kernel::execution::DispatchStats>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn conflict_dispatch_forced() -> bool {
    CONFLICT_FORCE_DISPATCH.with(std::cell::Cell::get)
}

#[cfg(test)]
fn conflict_dispatch_fuse_forced() -> bool {
    CONFLICT_FORCE_FUSE.with(std::cell::Cell::get)
}

#[cfg(not(test))]
fn conflict_dispatch_fuse_forced() -> bool {
    false
}

#[cfg(test)]
pub(crate) struct ForceConflictFuseGuard(bool);

#[cfg(test)]
impl Drop for ForceConflictFuseGuard {
    fn drop(&mut self) {
        CONFLICT_FORCE_FUSE.with(|forced| forced.set(self.0));
    }
}

/// 测试专用：本拍起强制 P3 保持融合（#706 增量 E 组合矩阵融合侧入口，
/// fuse 优先于 force），返回复位守卫。
#[cfg(test)]
pub(crate) fn force_conflict_fuse() -> ForceConflictFuseGuard {
    ForceConflictFuseGuard(CONFLICT_FORCE_FUSE.with(|forced| forced.replace(true)))
}

#[cfg(test)]
pub(crate) struct ForceConflictDispatchGuard(bool);

#[cfg(test)]
impl Drop for ForceConflictDispatchGuard {
    fn drop(&mut self) {
        CONFLICT_FORCE_DISPATCH.with(|forced| forced.set(self.0));
    }
}

/// 测试专用：本拍起强制 P3 真实分发（工作集非空时），返回复位守卫。
#[cfg(test)]
pub(crate) fn force_conflict_dispatch() -> ForceConflictDispatchGuard {
    ForceConflictDispatchGuard(CONFLICT_FORCE_DISPATCH.with(|forced| forced.replace(true)))
}

/// 测试专用：最近一次 P3 分发的调度统计（按阶段独立）。
#[cfg(test)]
pub(crate) fn last_conflict_dispatch_stats() -> Option<crate::kernel::execution::DispatchStats> {
    LAST_CONFLICT_DISPATCH_STATS.with(std::cell::Cell::get)
}

/// 测试专用：P3 工作计数诊断开关（计数对拍测试开启；分配证据类关闭）。
#[cfg(test)]
pub(crate) struct ConflictDiagnosticsGuard(bool);

#[cfg(test)]
impl Drop for ConflictDiagnosticsGuard {
    fn drop(&mut self) {
        CONFLICT_WORK_DIAGNOSTICS.with(|flag| flag.set(self.0));
    }
}

#[cfg(test)]
pub(crate) fn enable_conflict_diagnostics() -> ConflictDiagnosticsGuard {
    ConflictDiagnosticsGuard(CONFLICT_WORK_DIAGNOSTICS.with(|flag| flag.replace(true)))
}

/// 线程本地 ConflictWorkCounts 快照；P3 分发 join 后按块级记录汇总回
/// 协调器线程（与 P5 的 MotionTlsSnapshot 同机制）。
#[cfg(test)]
type ConflictTlsSnapshot = crate::kernel::conflict::ConflictWorkCounts;

#[cfg(test)]
fn conflict_tls_snapshot() -> ConflictTlsSnapshot {
    crate::kernel::conflict::conflict_work_counts()
}

/// 块级诊断记录：一个执行块的 ConflictWorkCounts 增量（任务写本块独占
/// 槽，join 后由协调器汇总进线程本地计数器）。
#[cfg(test)]
#[derive(Default)]
struct ConflictWorkChunkRecord {
    delta: std::sync::Mutex<crate::kernel::conflict::ConflictWorkCounts>,
}

#[cfg(test)]
impl ConflictWorkChunkRecord {
    fn store_deltas(&self, before: ConflictTlsSnapshot) {
        let after = conflict_tls_snapshot();
        *self.delta.lock().expect("conflict chunk record delta") = after.wrapping_sub(before);
    }
}

/// 汇总：协调器线程计数器 = 分发前基线 + 全部块增量（调用线程自己的块
/// 增量经记录回灌，不重复计）。
#[cfg(test)]
fn aggregate_conflict_tls(baseline: ConflictTlsSnapshot, records: &[ConflictWorkChunkRecord]) {
    let mut sum = baseline;
    for record in records {
        sum = sum.wrapping_add(*record.delta.lock().expect("conflict chunk record delta"));
    }
    crate::kernel::conflict::set_conflict_work_counts(sum);
}

/// R4 预留探针：记录 F 位逻辑检查点是否到达、真实需求与余量、注入是否
/// 因「真实必要增长」触发。后续点位因更早失败不可达时，探针停留在更早
/// 点位（last-write-wins）。
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConflictReserveSite {
    CellWork,
    CandidateCells,
    DownstreamWork,
    DownstreamPool,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct ConflictReserveProbe {
    pub(crate) site: ConflictReserveSite,
    pub(crate) required: usize,
    pub(crate) len: usize,
    pub(crate) capacity: usize,
    pub(crate) injected: bool,
}

#[cfg(test)]
thread_local! {
    static CONFLICT_RESERVE_PROBE: std::cell::Cell<Option<ConflictReserveProbe>> =
        const { std::cell::Cell::new(None) };
}

/// 记录一次 F 位检查点访问；当且仅当真实必要增长且注入已武装时返回
/// true（调用方映射 ConflictScratchAllocFailed）。
#[cfg(test)]
fn conflict_reserve_probe(
    site: ConflictReserveSite,
    additional: usize,
    len: usize,
    capacity: usize,
    injected: bool,
) -> bool {
    let growth_needed = additional > capacity.saturating_sub(len);
    CONFLICT_RESERVE_PROBE.with(|probe| {
        probe.set(Some(ConflictReserveProbe {
            site,
            required: additional,
            len,
            capacity,
            injected: injected && growth_needed,
        }));
    });
    growth_needed && injected
}

/// 测试专用：读取最近一次 F 位检查点探针。
#[cfg(test)]
pub(crate) fn last_conflict_reserve_probe() -> Option<ConflictReserveProbe> {
    CONFLICT_RESERVE_PROBE.with(std::cell::Cell::get)
}

/// P3 任务侧注入（进程级原子量，按世界身份 + 发现序位武装；与 P2/P5
/// 注入面相互独立）。
#[cfg(test)]
mod conflict_injection {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    pub(super) const DISABLED_WORLD: u64 = u64::MAX;

    pub(super) static NONFINITE_WORLD: AtomicU64 = AtomicU64::new(DISABLED_WORLD);
    pub(super) static NONFINITE_POSITIONS: AtomicU64 = AtomicU64::new(0);
    pub(super) static INVARIANT_DOWNSTREAM_WORLD: AtomicU64 = AtomicU64::new(DISABLED_WORLD);
    pub(super) static INVARIANT_DOWNSTREAM_POSITIONS: AtomicU64 = AtomicU64::new(0);
    pub(super) static CELLS_RESERVE_FAILURE: AtomicBool = AtomicBool::new(false);
    pub(super) static CELL_WORK_RESERVE_FAILURE: AtomicBool = AtomicBool::new(false);
    pub(super) static DOWNSTREAM_WORK_RESERVE_FAILURE: AtomicBool = AtomicBool::new(false);
    pub(super) static DOWNSTREAM_POOL_RESERVE_FAILURE: AtomicBool = AtomicBool::new(false);
    pub(super) static INPUT_RESERVE_FAILURE: AtomicBool = AtomicBool::new(false);
    pub(super) static SLOT_RESERVE_FAILURE: AtomicBool = AtomicBool::new(false);

    pub(super) fn position_mask(positions: &[usize]) -> u64 {
        positions.iter().fold(0_u64, |mask, position| {
            assert!(
                *position < u64::BITS as usize,
                "conflict injection position fits mask"
            );
            mask | (1_u64 << position)
        })
    }

    pub(super) fn nonfinite_injected(world_id: u64, position: usize) -> bool {
        NONFINITE_WORLD.load(Ordering::SeqCst) == world_id
            && position < u64::BITS as usize
            && NONFINITE_POSITIONS.load(Ordering::SeqCst) & (1_u64 << position) != 0
    }

    pub(super) fn invariant_downstream_injected(world_id: u64, position: usize) -> bool {
        INVARIANT_DOWNSTREAM_WORLD.load(Ordering::SeqCst) == world_id
            && position < u64::BITS as usize
            && INVARIANT_DOWNSTREAM_POSITIONS.load(Ordering::SeqCst) & (1_u64 << position) != 0
    }

    pub(super) fn input_reserve_injected() -> bool {
        INPUT_RESERVE_FAILURE.load(Ordering::SeqCst)
    }

    pub(super) fn slot_reserve_injected() -> bool {
        SLOT_RESERVE_FAILURE.load(Ordering::SeqCst)
    }

    pub(super) fn cell_work_reserve_injected() -> bool {
        CELL_WORK_RESERVE_FAILURE.load(Ordering::SeqCst)
    }

    pub(super) fn cells_reserve_injected() -> bool {
        CELLS_RESERVE_FAILURE.load(Ordering::SeqCst)
    }

    pub(super) fn downstream_work_reserve_injected() -> bool {
        DOWNSTREAM_WORK_RESERVE_FAILURE.load(Ordering::SeqCst)
    }

    pub(super) fn downstream_pool_reserve_injected() -> bool {
        DOWNSTREAM_POOL_RESERVE_FAILURE.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
fn swap_conflict_flag(flag: &'static std::sync::atomic::AtomicBool) -> ConflictBoolGuard {
    ConflictBoolGuard(flag, flag.swap(true, std::sync::atomic::Ordering::SeqCst))
}

#[cfg(test)]
pub(crate) struct ConflictBoolGuard(&'static std::sync::atomic::AtomicBool, bool);

#[cfg(test)]
impl Drop for ConflictBoolGuard {
    fn drop(&mut self) {
        self.0.store(self.1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
pub(crate) struct ConflictNonfiniteGuard(u64, u64);

#[cfg(test)]
impl Drop for ConflictNonfiniteGuard {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        conflict_injection::NONFINITE_WORLD.store(self.0, Ordering::SeqCst);
        conflict_injection::NONFINITE_POSITIONS.store(self.1, Ordering::SeqCst);
    }
}

/// 测试专用：按发现序位武装 P3 逐车 NonFiniteMotion 注入（horizon/预览
/// 重算同原语位）。
#[cfg(test)]
pub(crate) fn inject_conflict_nonfinite(
    world_id: u64,
    positions: &[usize],
) -> ConflictNonfiniteGuard {
    use std::sync::atomic::Ordering;
    ConflictNonfiniteGuard(
        conflict_injection::NONFINITE_WORLD.swap(world_id, Ordering::SeqCst),
        conflict_injection::NONFINITE_POSITIONS.swap(
            conflict_injection::position_mask(positions),
            Ordering::SeqCst,
        ),
    )
}

#[cfg(test)]
pub(crate) struct ConflictInvariantGuard(u64, u64);

#[cfg(test)]
impl Drop for ConflictInvariantGuard {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        conflict_injection::INVARIANT_DOWNSTREAM_WORLD.store(self.0, Ordering::SeqCst);
        conflict_injection::INVARIANT_DOWNSTREAM_POSITIONS.store(self.1, Ordering::SeqCst);
    }
}

/// 测试专用：按发现序位武装「同车更晚 downstream 检查 ConflictInvariantViolation」
/// 注入（D4 核心反例的下游侧）。
#[cfg(test)]
pub(crate) fn inject_conflict_invariant_downstream(
    world_id: u64,
    positions: &[usize],
) -> ConflictInvariantGuard {
    use std::sync::atomic::Ordering;
    ConflictInvariantGuard(
        conflict_injection::INVARIANT_DOWNSTREAM_WORLD.swap(world_id, Ordering::SeqCst),
        conflict_injection::INVARIANT_DOWNSTREAM_POSITIONS.swap(
            conflict_injection::position_mask(positions),
            Ordering::SeqCst,
        ),
    )
}

/// 测试专用：下一次 cell 工作区真实预留（F1 位）强制失败。
#[cfg(test)]
pub(crate) fn fail_conflict_cell_work_reserve() -> ConflictBoolGuard {
    swap_conflict_flag(&conflict_injection::CELL_WORK_RESERVE_FAILURE)
}

/// 测试专用：下一次 candidate_cells 真实预留（F2 位）强制失败。
#[cfg(test)]
pub(crate) fn fail_conflict_cells_reserve() -> ConflictBoolGuard {
    swap_conflict_flag(&conflict_injection::CELLS_RESERVE_FAILURE)
}

/// 测试专用：下一次 downstream 工作区真实预留（F4 位）强制失败。
#[cfg(test)]
pub(crate) fn fail_conflict_downstream_work_reserve() -> ConflictBoolGuard {
    swap_conflict_flag(&conflict_injection::DOWNSTREAM_WORK_RESERVE_FAILURE)
}

/// 测试专用：下一次 candidate_downstream 真实预留（F3b 位）强制失败。
#[cfg(test)]
pub(crate) fn fail_conflict_downstream_pool_reserve() -> ConflictBoolGuard {
    swap_conflict_flag(&conflict_injection::DOWNSTREAM_POOL_RESERVE_FAILURE)
}

/// 测试专用：下一次 P3 输入表预留强制失败（冷态回退）。
#[cfg(test)]
pub(crate) fn fail_conflict_input_reserve() -> ConflictBoolGuard {
    swap_conflict_flag(&conflict_injection::INPUT_RESERVE_FAILURE)
}

/// 测试专用：下一次 P3 槽位预留强制失败（热态回退）。
#[cfg(test)]
pub(crate) fn fail_conflict_slot_reserve() -> ConflictBoolGuard {
    swap_conflict_flag(&conflict_injection::SLOT_RESERVE_FAILURE)
}

#[cfg(test)]
pub(crate) struct ConflictSlotGapGuard(Option<usize>);

#[cfg(test)]
impl Drop for ConflictSlotGapGuard {
    fn drop(&mut self) {
        CONFLICT_SLOT_GAP.with(|gap| gap.set(self.0));
    }
}

/// 测试专用：join 后把指定发现序位的槽位改写为 `Pending`（完成前沿检出）。
#[cfg(test)]
pub(crate) fn drop_conflict_slot_at(position: usize) -> ConflictSlotGapGuard {
    ConflictSlotGapGuard(CONFLICT_SLOT_GAP.with(|gap| gap.replace(Some(position))))
}

/// P3 冻结任务视图（D2 显式字段表）：
/// - `read`：拍初 C(T) 车辆/路线（静态根编译表、policy、拍初信号）+
///   derived occupancy（leader_gap 只读）；
/// - `conflict`：三腿 ConflictRead（committed+derived+workspace）。discard_staged
///   后 reservation/owner 读的是拍初 committed 语义；`cell_workspace` 的
///   approach frontier 由协调器 rebuild 完成后冻结借出；
/// - `waiting_plans`/`waiting_plan_by_vehicle`：本拍 P2 规范结果；
/// - `motion_cache`：本拍 P2 结果（horizon/preview 复用证明）。
#[derive(Clone, Copy)]
struct ConflictTaskView<'a> {
    read: crate::kernel::phase::StepReadView<'a>,
    conflict: crate::kernel::conflict::ConflictRead<'a>,
    waiting_plans: &'a [crate::kernel::waiting::WaitingVehiclePlan],
    waiting_plan_by_vehicle: &'a [Option<std::num::NonZeroU32>],
    motion_cache: &'a [crate::kernel::tick::MotionCacheEntry],
}

/// P3 多段报告（D3）：不得用单 None 或整车 Err 抹平状态。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CacheUpdates {
    /// §2 #7：horizon 在门距早退之前已算出，须落缓存。
    horizon: Option<crate::kernel::occupancy::LeaderQueryHorizon>,
    /// §2 #11：preview 在 Gate 循环之前已算出，须落缓存。
    preview: Option<crate::kernel::tick::MotionPreview>,
}

#[derive(Clone)]
pub(crate) enum CandidateReport {
    /// 无候选（无 Gate/门距超 horizon/无资源 Deny 静默/扫描完）。
    /// cache = 已求出待落缓存的 horizon/preview。
    None { cache: CacheUpdates },
    /// §2 #22：无资源门产出 staged 决定；reserve+push 在消费侧原位。
    Staged {
        cache: CacheUpdates,
        decision: crate::ConflictDecision,
    },
    /// 资源候选：motion plan 写（Err 路径亦落盘）+ 后续阶段。
    Resource(CandidateResource),
    /// 早段失败（compiled_route/profile/horizon/距离/waiting/preview/
    /// Gate 循环检查），无本车成功值义务；cache 同样须落盘。
    Failed {
        cache: CacheUpdates,
        error: StepError,
    },
}

#[derive(Clone)]
pub(crate) struct CandidateResource {
    /// §2 #7/#11：horizon/preview 在资源候选之前已算出，同样须落缓存。
    cache: CacheUpdates,
    /// §3 #2：Err 路径亦落盘的 motion plan 写。
    motion_plan: ConflictMotionPlan,
    /// §3 #6：eligibility 求值成功后写（纯 Waiting 空 range 与 #6 之前的
    /// CheckFailed 不写）。
    next_eligibility: Option<crate::ConflictEligibilityState>,
    stage: ResourceStage,
}

#[derive(Clone)]
enum ResourceStage {
    /// §3 #4：纯 Waiting 空 range 候选（空 cells/downstream 区间）。
    PureWaitingEmpty {
        key: ConflictCandidateOrderKey,
        anchor: ConflictRouteAnchor,
        waiting_zone: Option<laneflow_static_contract::WaitingZoneOrdinal>,
    },
    /// 929-1008 之间的检查失败（passage/slice/range/class；eligibility
    /// 不可达错）。此时除 motion plan/eligibility 外无本车声明义务。
    CheckFailed(StepError),
    /// 1009 之后：cell/downstream 段分别求值的多段结果。
    Computed {
        stable_passage: crate::ConflictPassageOccurrenceLocator,
        passage_range: ConflictPassageRange,
        gate_kind: GateCandidateKind,
        waiting_zone: Option<laneflow_static_contract::WaitingZoneOrdinal>,
        priority: Option<i32>,
        preflight_no_grant: Option<ConflictNoGrantReason>,
        cells: CellsSegment,
        downstream: DownstreamSegment,
    },
}

#[derive(Clone)]
enum CellsSegment {
    /// 任务局部物化成功（已 sort+dedup）。
    Values(Vec<crate::ConflictPassageAddress>),
    /// 任务局部暂存不足 → 协调器同原语补算（不冒充领域分配失败）。
    Unmaterialized,
    /// 出现项循环检查失败（F1 位之后、F2 之前）。
    Failed(StepError),
}

#[derive(Clone)]
enum DownstreamSegment {
    /// preflight NoGrant：整体跳过（downstream_start==end，A4/A5）。F4 前。
    SkippedPreflight,
    /// F4 前检查结束：NoGrant 折入 preflight；Invariant 公开
    /// ConflictInvariantViolation。本候选无 F4 义务（不得补预留）。
    PreFailed(DownstreamEvalError),
    /// 前置计划检查全部成功：F4（downstream_work 真实预留）已是本候选
    /// 义务，raw_capacity 为真实需求；fill 为 F4 之后的填充结果——
    /// 消费者必须先完成 F4 再消费 fill（失败位置保真：F4 增长失败先于
    /// F4 后填充检查失败公开）。
    Obligated {
        raw_capacity: usize,
        fill: Result<Vec<crate::DownstreamInterval>, DownstreamFillError>,
    },
    /// 任务局部暂存不足 → 协调器同原语补算（不冒充领域分配失败）。
    Unmaterialized,
}

/// F4 之前的检查失败分类。
#[derive(Clone)]
enum DownstreamEvalError {
    /// NoGrant(DownstreamStorageBoundary)：折入 preflight，不公开错误。
    NoGrant,
    /// InvalidBundle/Capacity → ConflictInvariantViolation。
    Invariant,
}

/// F4 之后的填充失败（derive 的路线下标/物理边/区间起终点检查）。
#[derive(Clone)]
enum DownstreamFillError {
    Invariant,
}

impl ConflictTaskView<'_> {
    /// P3 逐车候选求值原语（任务/补算共用）：与
    /// StepWorkspace::evaluate_vehicle_gates + prepare_resource_candidate +
    /// prepare_candidate_downstream 的检查次序与错误变体逐行一致；
    /// 不写入任何共享状态，共享写与真实预留由协调器按原序施加。
    fn evaluate_candidate(
        self,
        state: VehicleState,
        update_sequence: u32,
        _workload_index: usize,
        cache_index: usize,
        delta_s: f32,
        tick: u64,
    ) -> CandidateReport {
        #[cfg(test)]
        if conflict_injection::nonfinite_injected(self.read.binding.world_id, _workload_index) {
            return CandidateReport::Failed {
                cache: CacheUpdates::default(),
                error: StepError::NonFiniteMotion,
            };
        }
        let mut cache = CacheUpdates::default();
        let failed =
            |cache: CacheUpdates, error: StepError| CandidateReport::Failed { cache, error };
        let compiled = match self.read.compiled_route(state.route) {
            Some(compiled) => compiled,
            None => return failed(cache, StepError::ConflictInvariantViolation),
        };
        let first_possible_hop = if state.progress_mm == 0 && state.carry_um == 0 {
            state.route_edge_index.saturating_sub(1)
        } else {
            state.route_edge_index
        };
        let first_gate = compiled
            .gate_hops
            .partition_point(|hop| *hop < first_possible_hop);
        let Some(first_hop) = compiled.gate_hops.get(first_gate).copied() else {
            return CandidateReport::None { cache };
        };
        let profile = match self
            .read
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
        {
            Some(profile) => profile,
            None => return failed(cache, StepError::ConflictInvariantViolation),
        };
        let cached = self
            .motion_cache
            .get(cache_index)
            .copied()
            .filter(|entry| entry.vehicle == state.handle);
        let horizon = match cached.and_then(|entry| entry.horizon) {
            Some(horizon) => horizon,
            None => {
                match crate::kernel::tick::leader_query_horizon(state.speed_mm_s, profile, delta_s)
                {
                    Some(horizon) => horizon,
                    None => return failed(cache, StepError::NonFiniteMotion),
                }
            }
        };
        let distance = match crate::kernel::tables::distance_to_occurrence_start(
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            state.route_edge_index as usize,
            state.progress_mm,
            first_hop as usize + 1,
        ) {
            Some(distance) => distance,
            None => return failed(cache, StepError::ConflictInvariantViolation),
        };
        let gate_count = compiled.gate_hops.len();
        // §2 #7：horizon 在门距早退之前已算出，回报待落缓存。
        cache.horizon = Some(horizon);
        if !matches!(distance, BoundedDistance::Finite(mm) if mm <= horizon.front_query_mm) {
            return CandidateReport::None { cache };
        }
        let waiting_plan = self
            .waiting_plan_by_vehicle
            .get(state.handle.index() as usize)
            .copied()
            .flatten()
            .and_then(|index| self.waiting_plans.get(index.get() as usize - 1).copied())
            .filter(|plan| plan.vehicle == state.handle);
        let waiting_stop = match self.waiting_stop_for(&state) {
            Ok(stop) => stop,
            Err(error) => return failed(cache, error),
        };
        let motion = match cached
            .and_then(|entry| entry.preview)
            .and_then(|preview| preview.with_waiting_stop(waiting_stop))
            .or_else(|| {
                self.read.preview_active_vehicle_with_waiting_stop(
                    state,
                    delta_s,
                    waiting_stop,
                    Some(horizon),
                )
            }) {
            Some(motion) => motion,
            None => return failed(cache, StepError::NonFiniteMotion),
        };
        // §2 #11：preview 在 Gate 循环之前已算出，回报待落缓存。
        cache.preview = Some(motion);
        let preview = motion.next;
        for gate_index in first_gate..gate_count {
            let gate_hop = compiled.gate_hops[gate_index];
            let gate_edge = compiled.edges[gate_hop as usize];
            let gate_progress = self
                .read
                .binding
                .revision
                .traffic()
                .lane_lengths_millimetres()[gate_edge.index()];
            let reaches_gate = preview.route_edge_index > gate_hop
                || (preview.route_edge_index == gate_hop && preview.progress_mm == gate_progress)
                || (state.route_edge_index == gate_hop && state.progress_mm == gate_progress);
            if !reaches_gate {
                break;
            }
            let waiting_index = compiled
                .waiting
                .partition_point(|entry| entry.entry_hop < gate_hop);
            let waiting = compiled
                .waiting
                .get(waiting_index)
                .copied()
                .filter(|entry| entry.entry_hop == gate_hop);
            if waiting.is_some_and(|entry| {
                state.waiting_membership.is_some_and(|member| {
                    member.waiting_zone == entry.zone && member.release_hop == entry.release_hop
                })
            }) {
                continue;
            }
            let maneuver_index = compiled
                .maneuvers
                .partition_point(|entry| entry.exit_route_edge_index <= gate_hop);
            if compiled
                .maneuvers
                .get(maneuver_index)
                .filter(|entry| entry.entry_route_edge_index <= gate_hop)
                .is_none()
            {
                return failed(cache, StepError::ConflictInvariantViolation);
            }
            let anchor = ConflictRouteAnchor {
                route: state.route,
                maneuver_occurrence_index: match u32::try_from(maneuver_index) {
                    Ok(index) => index,
                    Err(_) => return failed(cache, StepError::ConflictInvariantViolation),
                },
                hop: gate_hop,
            };
            let range = compiled.conflict_gate_ranges[gate_hop as usize];
            let passage = if range.len != 0 {
                match self
                    .read
                    .conflict_passage_occurrence_locator(state.route, range.start)
                {
                    Some(locator) => Some(locator),
                    None => return failed(cache, StepError::ConflictInvariantViolation),
                }
            } else {
                None
            };
            let gate = match compiled.hop_gate[gate_hop as usize] {
                Some(gate) => gate,
                None => return failed(cache, StepError::ConflictInvariantViolation),
            };
            let decision = self.read.gate_policy_decision(gate, state.profile);
            let outcome = match decision {
                GatePolicyDecision::DenyAndStop => Some(ConflictDecisionOutcome::NotEvaluated),
                GatePolicyDecision::Candidate(_) => {
                    match waiting
                        .and_then(|_| waiting_plan.filter(|plan| plan.entry_hop == gate_hop))
                    {
                        Some(plan) => match plan.decision {
                            crate::WaitingDecisionOutcome::Granted => None,
                            crate::WaitingDecisionOutcome::NoGrant(
                                crate::WaitingNoGrantReason::Capacity,
                            ) => Some(ConflictDecisionOutcome::NoGrant(
                                ConflictNoGrantReason::WaitingCapacity,
                            )),
                            crate::WaitingDecisionOutcome::NoGrant(
                                crate::WaitingNoGrantReason::PhysicalStorage,
                            ) => Some(ConflictDecisionOutcome::NoGrant(
                                ConflictNoGrantReason::WaitingPhysicalStorage,
                            )),
                            _ => return failed(cache, StepError::WaitingInvariantViolation),
                        },
                        None if waiting.is_some() => Some(ConflictDecisionOutcome::NoGrant(
                            ConflictNoGrantReason::WaitingPhysicalStorage,
                        )),
                        None if range.len == 0 => Some(ConflictDecisionOutcome::NotRequired),
                        None => None,
                    }
                }
            };
            if let Some(outcome) = outcome {
                if waiting.is_none() && range.len == 0 {
                    // §2 A7：无资源决定按最终运动范围输出；NotRequired 继续扫描。
                    if outcome == ConflictDecisionOutcome::NotRequired {
                        continue;
                    }
                    return CandidateReport::None { cache };
                }
                return CandidateReport::Staged {
                    cache,
                    decision: crate::ConflictDecision {
                        vehicle: state.handle,
                        vehicle_update_sequence: update_sequence,
                        anchor,
                        passage,
                        outcome,
                    },
                };
            }
            let GatePolicyDecision::Candidate(kind) = decision else {
                unreachable!("denied Gate already produced a decision");
            };
            return self.prepare_resource_candidate_task(
                state,
                update_sequence,
                tick,
                _workload_index,
                cache,
                EvaluatedGate {
                    anchor,
                    passage,
                    range,
                    kind,
                    waiting_zone: waiting.map(|entry| entry.zone),
                },
            );
        }
        CandidateReport::None { cache }
    }

    /// prepare_resource_candidate 的任务版：多段报告（D3）。
    fn prepare_resource_candidate_task(
        self,
        state: VehicleState,
        update_sequence: u32,
        tick: u64,
        workload_index: usize,
        cache: CacheUpdates,
        gate: EvaluatedGate,
    ) -> CandidateReport {
        let EvaluatedGate {
            anchor,
            passage,
            range,
            kind,
            waiting_zone,
        } = gate;
        let gate_hop = anchor.hop;
        let motion_plan = ConflictMotionPlan {
            gate_hop,
            outcome: ConflictDecisionOutcome::NotEvaluated,
            grant_index: None,
        };
        let check_failed = |next_eligibility, error| {
            CandidateReport::Resource(CandidateResource {
                cache,
                motion_plan,
                next_eligibility,
                stage: ResourceStage::CheckFailed(error),
            })
        };
        let stable_passage = if range.len != 0 {
            match passage {
                Some(passage) => passage,
                None => {
                    return check_failed(None, StepError::ConflictInvariantViolation);
                }
            }
        } else {
            // §3 #4：pure Waiting 空 range，空 cells/downstream 候选早退。
            let key = ConflictCandidateOrderKey::new(
                kind,
                None,
                tick,
                state
                    .waiting_membership
                    .map(|member| member.admission_sequence),
                update_sequence,
            );
            return CandidateReport::Resource(CandidateResource {
                cache,
                motion_plan,
                next_eligibility: None,
                stage: ResourceStage::PureWaitingEmpty {
                    key,
                    anchor,
                    waiting_zone,
                },
            });
        };
        let eligibility = match crate::ConflictEligibilityState::update(
            self.read
                .committed
                .conflict_eligibility
                .get(state.handle.index() as usize)
                .copied()
                .flatten(),
            stable_passage,
            true,
            tick,
        ) {
            Some(eligibility) => eligibility,
            None => return check_failed(None, StepError::ConflictInvariantViolation),
        };
        let passage_end = match range.start.checked_add(range.len) {
            Some(end) => end,
            None => return check_failed(Some(eligibility), StepError::ConflictInvariantViolation),
        };
        if compiled_conflicts_slice(
            self.read.compiled_route(state.route),
            range.start,
            passage_end,
        )
        .is_none()
        {
            return check_failed(Some(eligibility), StepError::ConflictInvariantViolation);
        }
        let passage_range = match ConflictPassageRange::new(
            state.route,
            anchor.maneuver_occurrence_index,
            gate_hop,
            range.start,
            range.len,
        ) {
            Some(range) => range,
            None => return check_failed(Some(eligibility), StepError::ConflictInvariantViolation),
        };
        let mut priority = None;
        let mut preflight_no_grant = None;
        let class = match self
            .read
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
        {
            Some(profile) => profile.class(),
            None => return check_failed(Some(eligibility), StepError::ConflictInvariantViolation),
        };
        // 出现项循环：任务局部可失败暂存（容量不足 → Unmaterialized，
        // 不冒充领域分配失败）。
        let mut cell_work: Vec<crate::ConflictPassageAddress> = Vec::new();
        // 'cells 的全部路径（含中途 break）都先赋值再离开块。
        let cells: CellsSegment;
        'cells: {
            let policy = match self
                .read
                .binding
                .policy_binding
                .policy(&self.read.binding.revision)
            {
                Some(policy) => policy,
                None => {
                    cells = CellsSegment::Failed(StepError::ConflictInvariantViolation);
                    break 'cells;
                }
            };
            for occurrence_index in range.start..passage_end {
                #[cfg(test)]
                crate::kernel::conflict::count_conflict_work(|counts| counts.visited_passages += 1);
                let occurrence = match self
                    .read
                    .compiled_route(state.route)
                    .and_then(|compiled| compiled.conflicts.get(occurrence_index as usize))
                {
                    Some(occurrence) => *occurrence,
                    None => {
                        cells = CellsSegment::Failed(StepError::ConflictInvariantViolation);
                        break 'cells;
                    }
                };
                if cell_work.try_reserve(1).is_err() {
                    cells = CellsSegment::Unmaterialized;
                    break 'cells;
                }
                cell_work.push(occurrence.address());
                let stream = match policy.stream(occurrence.stream, class) {
                    Some(stream) => stream,
                    None => {
                        cells = CellsSegment::Failed(StepError::ConflictInvariantViolation);
                        break 'cells;
                    }
                };
                priority = Some(priority.map_or(stream.priority(), |current: i32| {
                    current.min(stream.priority())
                }));
                // Protected 候选仍解析规则并收集资源，只跳过让行间隙求值。
                if kind == GateCandidateKind::Protected {
                    continue;
                }
                let (zone, targets) = match policy.yield_targets(
                    occurrence.stream,
                    class,
                    occurrence.passage_local_index,
                ) {
                    Some(targets) => targets,
                    None => {
                        cells = CellsSegment::Failed(StepError::ConflictInvariantViolation);
                        break 'cells;
                    }
                };
                if zone != occurrence.zone {
                    cells = CellsSegment::Failed(StepError::ConflictInvariantViolation);
                    break 'cells;
                }
                let Some(gap_index) = stream.gap_profile_index() else {
                    if !targets.is_empty() {
                        cells = CellsSegment::Failed(StepError::ConflictInvariantViolation);
                        break 'cells;
                    }
                    continue;
                };
                let gap = match self
                    .read
                    .binding
                    .policy_binding
                    .gaps()
                    .get(gap_index as usize)
                {
                    Some(gap) => *gap,
                    None => {
                        cells = CellsSegment::Failed(StepError::ConflictInvariantViolation);
                        break 'cells;
                    }
                };
                for target in targets {
                    let address = crate::ConflictPassageAddress::new(
                        occurrence.zone,
                        target.stream(),
                        target.passage_local_index(),
                    );
                    let outcome = self.conflict.evaluate_yield_target(
                        state.handle,
                        address,
                        self.read.committed.time_ms,
                        gap.required_lag_ms(),
                        gap.required_lead_ms(),
                    );
                    let Some(outcome) = outcome else {
                        cells = CellsSegment::Failed(StepError::ConflictInvariantViolation);
                        break 'cells;
                    };
                    if let Some(reason) = map_yield(outcome) {
                        preflight_no_grant = Some(preflight_no_grant.map_or(reason, |current| {
                            if no_grant_rank(reason) < no_grant_rank(current) {
                                reason
                            } else {
                                current
                            }
                        }));
                    }
                }
            }
            // 循环正常结束才到达这里：中途退出（Failed 检查失败 /
            // Unmaterialized 任务局部暂存不足）由 break 跳过本段，成功前缀
            // 不保留，协调器整体补算。
            cell_work.sort_unstable();
            cell_work.dedup();
            cells = CellsSegment::Values(cell_work);
        }
        // downstream 段（A4/A5：preflight 有值即整体跳过）。
        let downstream = if preflight_no_grant.is_some() {
            DownstreamSegment::SkippedPreflight
        } else {
            self.prepare_candidate_downstream_task(state, passage_range, gate_hop, workload_index)
        };
        CandidateReport::Resource(CandidateResource {
            cache,
            motion_plan,
            next_eligibility: Some(eligibility),
            stage: ResourceStage::Computed {
                stable_passage,
                passage_range,
                gate_kind: kind,
                waiting_zone,
                priority,
                preflight_no_grant,
                cells,
                downstream,
            },
        })
    }

    /// prepare_candidate_downstream 的任务版：F3/F4 预留留在协调器原位，
    /// 这里只做检查与 claims 求值（任务局部，容量不足 → Unmaterialized）。
    fn prepare_candidate_downstream_task(
        self,
        state: VehicleState,
        range: ConflictPassageRange,
        gate_hop: u32,
        _workload_index: usize,
    ) -> DownstreamSegment {
        #[cfg(test)]
        if conflict_injection::invariant_downstream_injected(
            self.read.binding.world_id,
            _workload_index,
        ) {
            return DownstreamSegment::PreFailed(DownstreamEvalError::Invariant);
        }
        let plan = match self
            .read
            .reservation_downstream_claim_plan(range, state.length_mm)
        {
            Ok(plan) => plan,
            Err(ConflictAcquireError::NoGrant(_)) => {
                return DownstreamSegment::PreFailed(DownstreamEvalError::NoGrant);
            }
            Err(_) => {
                return DownstreamSegment::PreFailed(DownstreamEvalError::Invariant);
            }
        };
        let compiled = match self.read.compiled_route(state.route) {
            Some(compiled) => compiled,
            None => {
                return DownstreamSegment::PreFailed(DownstreamEvalError::Invariant);
            }
        };
        let target = plan.target();
        let required = match distance_to_occurrence_progress(
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            state.route_edge_index as usize,
            state.progress_mm,
            target.route_edge_index() as usize,
            target.progress_mm(),
        ) {
            Some(BoundedDistance::Finite(value)) => value,
            Some(BoundedDistance::BeyondFinite) | None => {
                return DownstreamSegment::PreFailed(DownstreamEvalError::NoGrant);
            }
        };
        let profile = match self
            .read
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
        {
            Some(profile) => profile,
            None => {
                return DownstreamSegment::PreFailed(DownstreamEvalError::Invariant);
            }
        };
        let leader_gap = self.read.derived.occupancy.leader_gap(
            state.handle,
            &compiled.edges,
            state.route_edge_index as usize,
            state.progress_mm,
            self.read
                .binding
                .revision
                .traffic()
                .lane_lengths_millimetres(),
            crate::kernel::occupancy::LeaderQueryHorizon::new(u32::MAX, u32::MAX),
        );
        if leader_gap.is_some_and(|gap| {
            gap < i64::from(required).saturating_add(i64::from(profile.min_gap_mm()))
        }) {
            return DownstreamSegment::PreFailed(DownstreamEvalError::NoGrant);
        }
        if let Some(next_gate) = compiled
            .gate_hops
            .iter()
            .copied()
            .find(|hop| *hop > gate_hop)
        {
            let boundary = match gate_boundary(next_gate) {
                Ok(boundary) => boundary,
                Err(_) => {
                    return DownstreamSegment::PreFailed(DownstreamEvalError::Invariant);
                }
            };
            if target > boundary {
                return DownstreamSegment::PreFailed(DownstreamEvalError::NoGrant);
            }
        }
        let waiting = match self.waiting_stop_for(&state) {
            Ok(waiting) => waiting,
            Err(_) => {
                return DownstreamSegment::PreFailed(DownstreamEvalError::Invariant);
            }
        };
        if let Some(waiting) = waiting
            && waiting.hop > gate_hop
        {
            let boundary = match gate_boundary(waiting.hop) {
                Ok(boundary) => boundary,
                Err(_) => {
                    return DownstreamSegment::PreFailed(DownstreamEvalError::Invariant);
                }
            };
            if target > boundary {
                return DownstreamSegment::PreFailed(DownstreamEvalError::NoGrant);
            }
        }
        if let Some(ParkingBinding::Reserved(reservation)) =
            self.read.committed.parking.binding(state.handle)
        {
            if reservation.route() != state.route {
                return DownstreamSegment::PreFailed(DownstreamEvalError::Invariant);
            }
            let Some((_, progress_mm)) = self.read.reservation_anchor(reservation) else {
                return DownstreamSegment::PreFailed(DownstreamEvalError::Invariant);
            };
            let Some(parking) = crate::DownstreamRoutePoint::new(
                reservation.entry_route_occurrence(),
                progress_mm,
                0,
            ) else {
                return DownstreamSegment::PreFailed(DownstreamEvalError::Invariant);
            };
            if target > parking {
                return DownstreamSegment::PreFailed(DownstreamEvalError::NoGrant);
            }
        }
        // F4 位（downstream_work 真实预留）留在协调器消费侧原位。
        let raw_capacity = plan.raw_interval_capacity();
        let route = plan.route();
        let compiled = match self
            .read
            .committed
            .routes
            .get(route.index() as usize)
            .filter(|slot| slot.generation == route.generation())
            .and_then(|slot| slot.compiled.as_ref())
        {
            Some(compiled) => compiled,
            None => {
                return DownstreamSegment::PreFailed(DownstreamEvalError::Invariant);
            }
        };
        let mut claims: Vec<crate::DownstreamInterval> = Vec::new();
        if claims.try_reserve(raw_capacity).is_err() {
            return DownstreamSegment::Unmaterialized;
        }
        let fill = if crate::kernel::conflict::derive_downstream_claims_from_plan(
            &compiled.edges,
            self.read
                .binding
                .revision
                .traffic()
                .lane_lengths_millimetres(),
            plan.plan,
            &mut claims,
        )
        .is_err()
            || claims.capacity() < raw_capacity
        {
            Err(DownstreamFillError::Invariant)
        } else {
            Ok(claims)
        };
        DownstreamSegment::Obligated { raw_capacity, fill }
    }
}

fn compiled_conflicts_slice(
    compiled: Option<&crate::kernel::tables::CompiledRoute>,
    start: u32,
    end: u32,
) -> Option<()> {
    compiled
        .and_then(|compiled| compiled.conflicts.get(start as usize..end as usize))
        .map(|_| ())
}

impl ConflictTaskView<'_> {
    /// waiting_stop_for 的冻结暂存视图版（与 tick.rs MotionTaskView 同义）。
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
}
