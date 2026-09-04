use std::sync::Arc;

#[cfg(test)]
use std::cell::Cell;

use laneflow_static_contract::{
    EntityKind, ParkingSpaceOrdinal, ParticipantClassOrdinal, SignalAspect,
    SignalControllerOrdinal, SignalGroupOrdinal,
};
use laneflow_static_network::SharedNetworkRevision;

use crate::conflict::ConflictAcquireError;
use crate::migration_journal::{MigrationDeltaJournal, MigrationJournalError, VehicleDelta};
use crate::occupancy::OccupancyIndex;
use crate::parking::ParkingRuntimeState;
use crate::tables::{
    CompiledRoute, ConflictCapabilityError, RouteSlot, VehicleSlot, bodies_overlap,
    check_conflict_capability, compile_route, occupancy_front_gap, route_access_denied,
};
use crate::waiting::{
    WaitingAdmissionClaim, WaitingQueueLink, WaitingVehiclePlan, WaitingZoneState,
};
use crate::{
    CommittedNetworkSource, CommittedPoseSourceBatch, CommittedSignalGroupBatch, CutoverError,
    InstallError, ObservationStateSequence, ParkingBinding, ParkingFacilityCounts,
    ParkingPoolCounts, ParkingSpaceState, ParkingTarget, PoseSource, ReplaceError, RouteError,
    RouteHandle, RouteRegisterInput, SpawnError, StepError, StepOutcome, TickInput, VehicleHandle,
    VehicleReplaceBlock, VehicleReplaceRecord, VehicleSpawnInput, VehicleState, VehicleStatus,
    WorldConfig,
};

#[cfg(test)]
thread_local! {
    static OVERLAP_BLOCKER_INSPECTIONS: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
fn reset_overlap_blocker_inspections() {
    OVERLAP_BLOCKER_INSPECTIONS.set(0);
}

#[cfg(test)]
fn overlap_blocker_inspections() -> usize {
    OVERLAP_BLOCKER_INSPECTIONS.get()
}

/// 活动世界世代。安装时从 [`Self::INITIAL`] 开始，每次成功换绑活动聚合时递增。
///
/// 字段保持私有：调用方应从 [`TrafficWorld::world_generation`] 取得当前值，
/// 再把它绑定到切换、观测或 Routing 会话，而不是自行猜测世代。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct WorldGeneration(u64);

impl WorldGeneration {
    /// 新安装世界的初始世代。
    pub const INITIAL: Self = Self(0);

    /// 用于日志、诊断与同进程绑定的精确 `u64` 值。
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    #[cfg(test)]
    pub(crate) const fn from_raw_for_test(value: u64) -> Self {
        Self(value)
    }
}

/// 1-worker 交通世界。只克隆根 `Arc`，不复制静态 component。
/// 生命周期命令（路线、车辆、parking lifecycle 与原子 replace/despawn）只在两次
/// `step` 之间调用。
pub struct TrafficWorld {
    pub(crate) revision: Arc<SharedNetworkRevision>,
    pub(crate) source: CommittedNetworkSource,
    /// 宿主指定的世界身份；切换描述符 `worldBinding` 在事务启动时比对。
    pub(crate) world_id: u64,
    /// 活动聚合世代；成功切换/恢复的唯一失效轴。
    pub(crate) world_generation: WorldGeneration,
    pub(crate) config: WorldConfig,
    pub(crate) policy_binding: crate::policy::WorldPolicyBinding,
    /// W4 的单写者冲突/下游资源权威；生产 passage 入口仍由 3A 保护到完整接通。
    pub(crate) conflict_arbiter: crate::conflict::ConflictArbiter,
    /// 车辆槽位对应的 exact Gate occurrence 首次资格时钟；W5 负责持久化。
    pub(crate) conflict_eligibility: Vec<Option<crate::ConflictEligibilityState>>,
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
    pub(crate) vehicles: Vec<VehicleSlot>,
    pub(crate) free_vehicles: Vec<usize>,
    pub(crate) live_order: Vec<VehicleHandle>,
    /// 仅含 `Active` 的固定步进执行顺序；按 `live_order` 投影维护，Parked / Completed
    /// 不进入 tick 或 lane occupancy 重建扫描。
    pub(crate) active_order: Vec<VehicleHandle>,
    pub(crate) parking: ParkingRuntimeState,
    /// 每个静态 WaitingZone 的稠密本地动态状态。
    pub(crate) waiting_zones: Box<[WaitingZoneState]>,
    /// 车辆槽位下标对应的 intrusive queue link；长度固定为 `vehicle_capacity`。
    pub(crate) waiting_links: Box<[WaitingQueueLink]>,
    /// 只读 member batch，按 `(zone, admission_sequence)` 排列。
    pub(crate) waiting_member_rows: Vec<crate::WaitingZoneMember>,
    /// tick scratch：每车至多一个新 Waiting admission claim。
    pub(crate) waiting_claims: Vec<WaitingAdmissionClaim>,
    pub(crate) waiting_plans: Vec<WaitingVehiclePlan>,
    pub(crate) waiting_plan_by_vehicle: Box<[Option<WaitingVehiclePlan>]>,
    pub(crate) waiting_next_state_index: Box<[u32]>,
    pub(crate) waiting_staged_decisions: Vec<crate::WaitingDecision>,
    pub(crate) waiting_staged_events: Vec<crate::WaitingTransitionEvent>,
    pub(crate) waiting_next_counters: Box<[u64]>,
    pub(crate) waiting_staged_occupancy: Box<[u32]>,
    pub(crate) waiting_staged_storage_mm: Box<[u64]>,
    /// 刚完成 successful tick 的 latest decision batch。
    pub(crate) latest_waiting_decisions: Vec<crate::WaitingDecision>,
    /// 刚完成 successful tick 的 committed transition event batch。
    pub(crate) latest_waiting_events: Vec<crate::WaitingTransitionEvent>,
    pub(crate) next_states: Vec<(usize, VehicleState)>,
    pub(crate) occupancy: OccupancyIndex,
    /// 武装中的迁移增量日志（#513 切片 C）：`Some` ⟺ 本世界存在在途切换事务。
    /// 武装与解除都只发生在切换事务的原子边界；溢出粘性置位，从不影响本世界
    /// 自身的提交路径。
    pub(crate) migration_journal: Option<MigrationDeltaJournal>,
    /// 日志武装轮次：每次成功武装递增（进程内守卫，不落盘）。事务绑定
    /// 武装时的轮次，配对校验一并比对——世界级恢复后重新武装的新日志
    /// 对旧事务按配对失配失败关闭，防止旧事务认领后继日志。
    pub(crate) migration_epoch: u64,
}

struct UnparkedVehicleAuthority {
    class: ParticipantClassOrdinal,
    length_mm: u32,
    maneuver_traversal: Option<crate::ManeuverTraversalState>,
    waiting_membership: Option<crate::WaitingMembership>,
}

impl TrafficWorld {
    /// 安装完整共享根并指名已提交来源（#302 活动聚合）。失败不留下
    /// 可观察的半个 world。
    ///
    /// 来源的 `NetworkRevisionId` 必须与共享根 origin 精确相等；digest /
    /// length 差异（同修订重发布）按合同只承担来源审计，不构成拒绝条件。
    pub fn install(
        revision: Arc<SharedNetworkRevision>,
        config: WorldConfig,
        source: CommittedNetworkSource,
        world_id: u64,
        policy_selection: crate::WorldPolicySelection,
    ) -> Result<Self, InstallError> {
        let installed = revision.canonical_origin().network_revision();
        if source.network_revision() != installed {
            return Err(InstallError::SourceRevisionMismatch {
                source_revision: source.network_revision(),
                installed_revision: installed,
            });
        }
        let dt = config.fixed_delta_time_ms();
        if !(4..=1_000).contains(&dt) {
            return Err(InstallError::DeltaOutOfRange {
                actual: dt,
                min: 4,
                max: 1_000,
            });
        }
        if config.worker_count() != 1 {
            return Err(InstallError::WorkerCountNotOne);
        }
        validate_signal_programs(revision.as_ref(), config.fixed_delta_time_ms())?;
        let policy_binding =
            crate::policy::WorldPolicyBinding::install(&revision, policy_selection, dt)?;
        let group_count = usize::try_from(
            revision
                .traffic()
                .entity_counts()
                .count(EntityKind::SignalGroup),
        )
        .expect("signal group count fits usize");
        let space_count = usize::try_from(
            revision
                .traffic()
                .entity_counts()
                .count(EntityKind::ParkingSpace),
        )
        .expect("parking space count fits usize");
        let facility_count = usize::try_from(
            revision
                .traffic()
                .entity_counts()
                .count(EntityKind::ParkingFacility),
        )
        .expect("parking facility count fits usize");
        let waiting_zone_count = usize::try_from(
            revision
                .traffic()
                .entity_counts()
                .count(EntityKind::WaitingZone),
        )
        .expect("waiting zone count fits usize");
        let vehicle_capacity = usize::try_from(config.vehicle_capacity()).unwrap_or(0);
        let route_capacity = usize::try_from(config.route_capacity()).unwrap_or(0);
        let conflict_arbiter =
            crate::conflict::ConflictArbiter::install(&revision, vehicle_capacity)
                .map_err(map_conflict_install_error)?;
        let mut world = Self {
            revision,
            source,
            world_id,
            world_generation: WorldGeneration::INITIAL,
            config,
            policy_binding,
            conflict_arbiter,
            conflict_eligibility: Vec::new(),
            tick_index: 0,
            time_ms: 0,
            command_cursor: 0,
            event_cursor: 0,
            observation_state_sequence: ObservationStateSequence::INITIAL,
            signal_aspects: vec![SignalAspect::Red; group_count].into_boxed_slice(),
            routes: Vec::with_capacity(route_capacity),
            free_routes: Vec::with_capacity(route_capacity),
            live_route_count: 0,
            live_route_edge_occurrence_count: 0,
            live_route_conflict_occurrence_count: 0,
            vehicles: Vec::with_capacity(vehicle_capacity),
            free_vehicles: Vec::with_capacity(vehicle_capacity),
            live_order: Vec::with_capacity(vehicle_capacity),
            active_order: Vec::with_capacity(vehicle_capacity),
            parking: ParkingRuntimeState::new(space_count, facility_count),
            waiting_zones: vec![WaitingZoneState::default(); waiting_zone_count].into_boxed_slice(),
            waiting_links: vec![WaitingQueueLink::default(); vehicle_capacity].into_boxed_slice(),
            waiting_member_rows: Vec::with_capacity(vehicle_capacity),
            waiting_claims: Vec::with_capacity(vehicle_capacity),
            waiting_plans: Vec::with_capacity(vehicle_capacity),
            waiting_plan_by_vehicle: vec![None; vehicle_capacity].into_boxed_slice(),
            waiting_next_state_index: vec![0; vehicle_capacity].into_boxed_slice(),
            waiting_staged_decisions: Vec::new(),
            waiting_staged_events: Vec::new(),
            waiting_next_counters: vec![0; waiting_zone_count].into_boxed_slice(),
            waiting_staged_occupancy: vec![0; waiting_zone_count].into_boxed_slice(),
            waiting_staged_storage_mm: vec![0; waiting_zone_count].into_boxed_slice(),
            latest_waiting_decisions: Vec::new(),
            latest_waiting_events: Vec::new(),
            next_states: Vec::with_capacity(vehicle_capacity),
            occupancy: OccupancyIndex::with_capacity(0, 0),
            migration_journal: None,
            migration_epoch: 0,
        };
        world.refresh_signals();
        Ok(world)
    }

    /// 已提交路网来源（#302 活动聚合的来源指名）。
    #[must_use]
    pub const fn committed_source(&self) -> &CommittedNetworkSource {
        &self.source
    }

    /// 宿主指定的世界身份（切换描述符 `worldBinding` 的比对对象）。
    #[must_use]
    pub const fn world_id(&self) -> u64 {
        self.world_id
    }

    #[must_use]
    pub const fn policy_selection(&self) -> crate::WorldPolicySelection {
        self.policy_binding.selection()
    }

    /// 当前世界唯一所选策略，借用同一个共享根。
    #[must_use]
    pub fn policy(&self) -> Option<laneflow_static_network::PolicyView<'_>> {
        self.policy_binding.policy(&self.revision)
    }

    #[must_use]
    pub fn policy_gap_profiles(&self) -> &[crate::DerivedPolicyGap] {
        self.policy_binding.gaps()
    }

    /// 把当前根内的 passage 地址派生为可持久化的稳定 locator。
    #[must_use]
    pub fn conflict_passage_locator(
        &self,
        address: crate::ConflictPassageAddress,
    ) -> Option<crate::ConflictPassageLocator> {
        if !self.conflict_arbiter.contains_address(address) {
            return None;
        }
        Some(crate::ConflictPassageLocator::new(
            self.revision.identity().stable_id(address.stream())?,
            self.revision.identity().stable_id(address.zone())?,
        ))
    }

    /// 返回已注册路线中的 exact conflict occurrence locator。
    ///
    /// 该只读派生不授予通行权；循环路线中的重复 passage 由 occurrence 下标区分，
    /// 其 `stable_locator` 仍只包含跨修订所需的两个稳定 ID。
    #[must_use]
    pub fn conflict_passage_occurrence_locator(
        &self,
        route: RouteHandle,
        conflict_occurrence_index: u32,
    ) -> Option<crate::ConflictPassageOccurrenceLocator> {
        let compiled = self.compiled_route(route)?;
        let occurrence = *compiled
            .conflicts
            .get(usize::try_from(conflict_occurrence_index).ok()?)?;
        let stable_locator = self.conflict_passage_locator(occurrence.address())?;
        compiled.conflict_occurrence_locator(route, conflict_occurrence_index, stable_locator)
    }

    /// 当前共享根中的静态 conflict passage cell 数；动态路线及其重复 occurrence 不复制 cell。
    #[must_use]
    pub fn conflict_passage_cell_count(&self) -> usize {
        self.conflict_arbiter.cell_count()
    }

    /// 返回该车辆当前由 Conflict arbiter 单独持有的 committed reservation。
    #[must_use]
    pub fn conflict_reservation(
        &self,
        vehicle: VehicleHandle,
    ) -> Option<crate::ConflictReservation> {
        self.conflict_arbiter.reservation(vehicle)
    }

    pub(crate) fn conflict_state_valid(&self) -> bool {
        if !self.conflict_eligibility.is_empty()
            && self.conflict_eligibility.len()
                != usize::try_from(self.config.vehicle_capacity()).unwrap_or(usize::MAX)
        {
            return false;
        }
        for (index, slot) in self.vehicles.iter().enumerate() {
            let state = slot.state.as_ref();
            let eligibility = self.conflict_eligibility.get(index).copied().flatten();
            match (state, eligibility) {
                (None, None) => {}
                (None, Some(_)) => return false,
                (Some(state), eligibility) => {
                    if !self.conflict_arbiter.state_valid(state) {
                        return false;
                    }
                    if let Some(eligibility) = eligibility
                        && !self.conflict_eligibility_authority_valid(state, eligibility)
                    {
                        return false;
                    }
                    if let Some(reservation) = self.conflict_reservation(state.handle) {
                        let range = reservation.passage_range();
                        let Some(compiled) = self.compiled_route(state.route) else {
                            return false;
                        };
                        let Some(gate_range) = compiled
                            .conflict_gate_ranges
                            .get(range.admission_gate_hop() as usize)
                        else {
                            return false;
                        };
                        if gate_range.start != range.first_conflict_occurrence_index()
                            || gate_range.len != range.passage_count()
                            || reservation.acquired_tick() > self.tick_index
                        {
                            return false;
                        }
                        let Some(gate_edge) = compiled
                            .edges
                            .get(range.admission_gate_hop() as usize)
                            .copied()
                        else {
                            return false;
                        };
                        let Some(gate_progress_mm) = self
                            .revision
                            .traffic()
                            .lane_lengths_millimetres()
                            .get(gate_edge.index())
                            .copied()
                        else {
                            return false;
                        };
                        let Some(gate_crossed_side) = crate::DownstreamRoutePoint::new(
                            range.admission_gate_hop(),
                            gate_progress_mm,
                            0,
                        ) else {
                            return false;
                        };
                        let Some(front) = crate::DownstreamRoutePoint::new(
                            state.route_edge_index,
                            state.progress_mm,
                            state.carry_um,
                        ) else {
                            return false;
                        };
                        if front < gate_crossed_side {
                            return false;
                        }
                        let Some(end) = range
                            .first_conflict_occurrence_index()
                            .checked_add(range.passage_count())
                        else {
                            return false;
                        };
                        for occurrence_index in range.first_conflict_occurrence_index()..end {
                            let Some(locator) = self
                                .conflict_passage_occurrence_locator(state.route, occurrence_index)
                            else {
                                return false;
                            };
                            if locator.maneuver_occurrence_index()
                                != range.maneuver_occurrence_index()
                                || locator.admission_gate_hop() != range.admission_gate_hop()
                                || !self
                                    .conflict_arbiter
                                    .reservation_has_cell(state.handle, locator.address())
                            {
                                return false;
                            }
                        }
                    }
                }
            }
        }
        if self
            .conflict_eligibility
            .get(self.vehicles.len()..)
            .is_some_and(|tail| tail.iter().any(Option::is_some))
        {
            return false;
        }
        self.conflict_arbiter.authority_owners_valid(
            |owner| self.vehicle_state(owner).is_some(),
            self.config.fixed_delta_time_ms(),
        )
    }

    /// W5 已恢复/迁移但尚未由 W7 tick 推进的 Conflict authority。
    ///
    /// eligibility 表在无条目时规范化为空，因此稳态无权威路径为 O(1)；
    /// arbiter 只扫描实际 owner authority 行，不按车辆容量扫描。
    pub(crate) fn conflict_authority_pending(&self) -> bool {
        !self.conflict_eligibility.is_empty() || self.conflict_arbiter.has_live_authority()
    }

    pub(crate) fn normalize_conflict_eligibility(&mut self) {
        if self.conflict_eligibility.iter().all(Option::is_none) {
            self.conflict_eligibility.clear();
        }
    }

    pub(crate) fn clear_conflict_eligibility(&mut self, vehicle: VehicleHandle) {
        if let Some(eligibility) = self.conflict_eligibility.get_mut(vehicle.index() as usize) {
            *eligibility = None;
        }
        self.normalize_conflict_eligibility();
    }

    pub(crate) fn vehicle_has_conflict_authority(&self, vehicle: VehicleHandle) -> bool {
        self.conflict_reservation(vehicle).is_some()
            || self
                .conflict_eligibility
                .get(vehicle.index() as usize)
                .is_some_and(Option::is_some)
            || self.conflict_arbiter.has_authority(vehicle)
    }

    pub(crate) fn pending_conflict_runtime_unavailable(
        &self,
    ) -> Option<crate::ConflictRuntimeUnavailable> {
        self.live_order.iter().copied().find_map(|handle| {
            let state = self.vehicle_state(handle)?;
            let eligibility = self
                .conflict_eligibility
                .get(handle.index() as usize)
                .copied()
                .flatten();
            if self.conflict_reservation(handle).is_none() && eligibility.is_none() {
                return None;
            }
            match self.check_active_conflict_capability(
                state.route,
                usize::try_from(state.route_edge_index).ok()?,
                state.progress_mm,
                state.carry_um,
                state.length_mm,
            ) {
                Err(crate::tables::ConflictCapabilityError::RuntimeUnavailable(error)) => {
                    Some(error)
                }
                Ok(()) | Err(crate::tables::ConflictCapabilityError::InvalidCursor) => None,
            }
        })
    }

    pub(crate) fn conflict_eligibility_position_valid(
        &self,
        state: &VehicleState,
        eligibility: crate::ConflictEligibilityState,
    ) -> bool {
        let hop = eligibility.locator().admission_gate_hop();
        let Some(compiled) = self.compiled_route(state.route) else {
            return false;
        };
        let Some(edge) = compiled.edges.get(hop as usize) else {
            return false;
        };
        state.route_edge_index == hop
            && state.progress_mm == self.revision.traffic().lane_lengths_millimetres()[edge.index()]
            && state.carry_um == 0
    }

    /// eligibility 的唯一 committed authority predicate。
    ///
    /// snapshot restore、cross-revision migration/expected projection 与 aggregate
    /// 校验都必须复用本入口，避免“身份/位置成立但当前 Gate policy 已拒绝”的
    /// 半份资格进入已发布世界。
    pub(crate) fn conflict_eligibility_authority_valid(
        &self,
        state: &VehicleState,
        eligibility: crate::ConflictEligibilityState,
    ) -> bool {
        let locator = eligibility.locator();
        if state.status != VehicleStatus::Active
            || self.conflict_reservation(state.handle).is_some()
            || locator.route() != state.route
            || self.conflict_passage_occurrence_locator(
                state.route,
                locator.conflict_occurrence_index(),
            ) != Some(locator)
            || !self.conflict_eligibility_position_valid(state, eligibility)
        {
            return false;
        }
        let Some(gate) = self
            .compiled_route(state.route)
            .and_then(|compiled| compiled.hop_gate.get(locator.admission_gate_hop() as usize))
            .copied()
            .flatten()
        else {
            return false;
        };
        matches!(
            self.gate_policy_decision(gate, state.profile),
            crate::GatePolicyDecision::Candidate(_)
        )
    }

    /// 从 reservation 级 exact route/Gate/passage 证明重建 downstream 物理资源并集。
    ///
    /// committed claim 只保留热路径需要的物理区间；snapshot/restore/cutover
    /// 必须调用本入口证明这些区间仍是当前根与整车长度的唯一推导结果。
    pub(crate) fn derive_reservation_downstream_claims(
        &self,
        range: crate::ConflictPassageRange,
        vehicle_length_mm: u32,
        output: &mut Vec<crate::DownstreamInterval>,
    ) -> Result<(), ConflictAcquireError> {
        let compiled = self
            .compiled_route(range.route())
            .ok_or(ConflictAcquireError::InvalidBundle)?;
        let first = usize::try_from(range.first_conflict_occurrence_index())
            .map_err(|_| ConflictAcquireError::InvalidBundle)?;
        let end = range
            .first_conflict_occurrence_index()
            .checked_add(range.passage_count())
            .and_then(|end| usize::try_from(end).ok())
            .ok_or(ConflictAcquireError::InvalidBundle)?;
        let occurrences = compiled
            .conflicts
            .get(first..end)
            .filter(|occurrences| {
                !occurrences.is_empty()
                    && occurrences.iter().all(|occurrence| {
                        occurrence.admission_hop == range.admission_gate_hop()
                            && occurrence.maneuver_index == range.maneuver_occurrence_index()
                    })
            })
            .ok_or(ConflictAcquireError::InvalidBundle)?;
        let farthest = occurrences
            .iter()
            .map(|occurrence| occurrence.clearance)
            .max()
            .ok_or(ConflictAcquireError::InvalidBundle)?;
        let gate_hop = usize::try_from(range.admission_gate_hop())
            .map_err(|_| ConflictAcquireError::InvalidBundle)?;
        let gate_edge = *compiled
            .edges
            .get(gate_hop)
            .ok_or(ConflictAcquireError::InvalidBundle)?;
        let gate_progress_mm = *self
            .revision
            .traffic()
            .lane_lengths_millimetres()
            .get(gate_edge.index())
            .ok_or(ConflictAcquireError::InvalidBundle)?;
        let gate =
            crate::DownstreamRoutePoint::new(range.admission_gate_hop(), gate_progress_mm, 0)
                .ok_or(ConflictAcquireError::InvalidBundle)?;
        let farthest =
            crate::DownstreamRoutePoint::new(farthest.route_edge_index, farthest.progress_mm, 0)
                .ok_or(ConflictAcquireError::InvalidBundle)?;
        let target = crate::conflict::downstream_claim_target(
            &compiled.edges,
            self.revision.traffic().lane_lengths_millimetres(),
            farthest,
            vehicle_length_mm,
        )?;
        crate::conflict::derive_downstream_claims(
            &compiled.edges,
            self.revision.traffic().lane_lengths_millimetres(),
            gate,
            target,
            output,
        )
    }

    #[must_use]
    pub const fn frontier_proof_horizon_ms(&self) -> Option<u64> {
        self.policy_binding.horizon()
    }

    /// 当前活动世界世代。成功切换后递增；失败或放弃保持不变。
    #[must_use]
    pub const fn world_generation(&self) -> WorldGeneration {
        self.world_generation
    }

    /// 共享根。
    #[must_use]
    pub fn revision(&self) -> Arc<SharedNetworkRevision> {
        Arc::clone(&self.revision)
    }

    /// 共享 Traffic component。
    #[must_use]
    pub fn traffic(&self) -> &laneflow_static_network::SharedTrafficNetwork {
        self.revision.traffic()
    }

    /// 已提交 `tick_index`。`install` 后为 0；成功 `step` 与 `StepOutcome` 一致；失败不变。
    #[must_use]
    pub const fn tick_index(&self) -> u64 {
        self.tick_index
    }

    /// 已提交 `time_ms`。`install` 后为 0；成功 `step` 与 `StepOutcome` 一致；失败不变。
    #[must_use]
    pub const fn time_ms(&self) -> u64 {
        self.time_ms
    }

    /// 已应用输入命令计数（快照合同 §3 双游标之一）。
    ///
    /// 生命周期命令（路线、车辆、parking lifecycle 与原子 replace/despawn）成功返回即
    /// 计数；合法 parking `NoChange` 同样计数，失败命令不计数。`step`
    /// 与切换事务不是输入命令，不推进本游标。安装后为零。
    #[must_use]
    pub const fn command_cursor(&self) -> u64 {
        self.command_cursor
    }

    /// 已提交切换事件游标（快照合同 §3 双游标之一；#513 切片 C 起
    /// 随事件批次通道成为真实轴）。安装后为零；每次成功切换（含放弃后
    /// 重试成功）恰递增一个事件批次。
    #[must_use]
    pub const fn event_cursor(&self) -> u64 {
        self.event_cursor
    }

    /// 安装时冻结的 world 配置。
    #[must_use]
    pub const fn config(&self) -> WorldConfig {
        self.config
    }

    /// 武装迁移增量日志（#513 切片 C）。只在切换事务 Prepare 边界调用：以
    /// 当前命令游标为覆盖区间下界，按字节上界一次预留 arena（此后武装期
    /// 稳态 tick 写入预留空间、不新增分配）。已有在途日志时武装失败。
    pub(crate) fn arm_migration_journal(
        &mut self,
        byte_bound: u64,
    ) -> Result<(), MigrationJournalError> {
        if self.migration_journal.is_some() {
            return Err(MigrationJournalError::AlreadyArmed);
        }
        let journal = MigrationDeltaJournal::arm(byte_bound, self.command_cursor)?;
        self.migration_epoch = self
            .migration_epoch
            .checked_add(1)
            .expect("migration epoch fits u64");
        self.migration_journal = Some(journal);
        Ok(())
    }

    /// 解除并取回迁移增量日志（切换事务放弃或提交边界的收尾步骤）。
    pub(crate) fn disarm_migration_journal(&mut self) -> Option<MigrationDeltaJournal> {
        self.migration_journal.take()
    }

    /// 世界级在途切换恢复入口：显式放弃武装中的迁移增量日志。
    ///
    /// 事务被静默丢弃、或以错世界结算（消耗形 `commit`/`abandon` 在
    /// [`CutoverError::TransactionWorldMismatch`] 后丢弃事务对象）时，来源
    /// 世界会保持在途锁定（`InFlightTransaction`）且不再存在可结算的事务
    /// 对象——本入口即该状态下的唯一恢复手段：旧世界从当前状态继续步进，
    /// 零事件、无候选晋升。无在途事务时按 [`CutoverError::NoInFlightTransaction`]
    /// 失败关闭。
    pub fn abandon_in_flight_cutover(&mut self) -> Result<(), CutoverError> {
        if self.disarm_migration_journal().is_some() {
            Ok(())
        } else {
            Err(CutoverError::NoInFlightTransaction)
        }
    }

    /// 武装中的日志只读视图（滞后、溢出与覆盖区间观测）。
    pub(crate) fn migration_journal(&self) -> Option<&MigrationDeltaJournal> {
        self.migration_journal.as_ref()
    }

    /// 武装中迁移增量日志的统计快照；`None` = 无在途切换事务。宿主据此
    /// 观测追赶滞后（tick 距离）、字节占用与溢出，编排泵入节奏或在超限
    /// 前显式改用维护暂停模式重试。
    #[must_use]
    pub fn migration_journal_stats(
        &self,
    ) -> Option<crate::migration_journal::MigrationJournalStats> {
        self.migration_journal
            .as_ref()
            .map(|journal| journal.stats())
    }

    /// 注册本世界路线。失败不留下半条路线。
    ///
    /// 在 compiled 槽位物化分段 `u32` 前缀、后缀距离、受控 hop 链和限速下降转换；
    /// 不上 `u64`，不存当前红灯。句柄不含 world 身份，只在本 `TrafficWorld` 内有效。
    pub fn register_route(&mut self, input: RouteRegisterInput) -> Result<RouteHandle, RouteError> {
        self.register_route_edges(input.edges())
    }

    /// 所有路线入口共用的权威注册路径。容量预检发生在 compiled O(n) 分配前。
    pub(crate) fn register_route_edges(
        &mut self,
        edges: &[laneflow_static_contract::LaneEdgeOrdinal],
    ) -> Result<RouteHandle, RouteError> {
        let next_occurrence_count = self.preflight_route_registration(edges.len())?;
        let next_command_cursor = self
            .command_cursor
            .checked_add(1)
            .expect("route preflight guarantees command cursor room");
        let compiled = compile_route(
            self.revision.as_ref(),
            edges,
            self.live_route_conflict_occurrence_count,
            self.config.route_conflict_occurrence_capacity(),
        )?;
        let added_conflict_occurrences =
            u64::try_from(compiled.conflicts.len()).expect("conflict occurrence count fits u64");
        let next_conflict_occurrence_count = self
            .live_route_conflict_occurrence_count
            .checked_add(added_conflict_occurrences)
            .expect("route conflict capacity preflight guarantees room");
        let slot_index = self.free_routes.pop().unwrap_or(self.routes.len());
        let generation = self
            .routes
            .get(slot_index)
            .map_or(0, |slot| slot.generation);
        let handle = RouteHandle::new(
            u32::try_from(slot_index).expect("route index fits u32"),
            generation,
        );
        let slot = RouteSlot {
            generation,
            compiled: Some(compiled),
            live_vehicles: 0,
        };
        if slot_index == self.routes.len() {
            self.routes.push(slot);
        } else {
            self.routes[slot_index] = slot;
        }
        self.live_route_count = self
            .live_route_count
            .checked_add(1)
            .expect("route count preflight guarantees room");
        self.live_route_edge_occurrence_count = next_occurrence_count;
        self.live_route_conflict_occurrence_count = next_conflict_occurrence_count;
        self.command_cursor = next_command_cursor;
        if let Some(journal) = self.migration_journal.as_mut() {
            journal.record_route_registered(next_command_cursor, handle, edges);
        }
        Ok(handle)
    }

    /// 所有路线入口共享的 O(1) 容量预检。调用方可在稳定标识解析前提早失败，
    /// 但提交入口仍须再次调用本函数，避免形成旁路权威。
    pub(crate) fn preflight_route_registration(
        &self,
        edge_count: usize,
    ) -> Result<u64, RouteError> {
        if edge_count == 0 {
            return Err(RouteError::EmptySequence);
        }
        if self.live_route_count >= self.config.route_capacity() {
            return Err(RouteError::CapacityExceeded);
        }
        let added_occurrences =
            u64::try_from(edge_count).map_err(|_| RouteError::EdgeOccurrenceCapacityExceeded)?;
        let next_occurrence_count = self
            .live_route_edge_occurrence_count
            .checked_add(added_occurrences)
            .ok_or(RouteError::EdgeOccurrenceCapacityExceeded)?;
        if next_occurrence_count > self.config.route_edge_occurrence_capacity() {
            return Err(RouteError::EdgeOccurrenceCapacityExceeded);
        }
        self.command_cursor
            .checked_add(1)
            .ok_or(RouteError::CommandCursorExhausted)?;
        Ok(next_occurrence_count)
    }

    /// 只移除本世界已注册路线。
    pub fn remove_route(&mut self, route: RouteHandle) -> Result<(), RouteError> {
        let index = usize::try_from(route.index()).expect("route index fits usize");
        let Some(slot) = self.routes.get(index) else {
            return Err(RouteError::StaleHandle);
        };
        if slot.generation != route.generation() || slot.compiled.is_none() {
            return Err(RouteError::StaleHandle);
        }
        if slot.live_vehicles > 0 {
            let vehicle = self
                .live_order
                .iter()
                .copied()
                .find(|vehicle| {
                    self.vehicle_state(*vehicle)
                        .is_some_and(|state| state.route == route)
                })
                .expect("live_vehicles > 0 必须能找到引用车辆");
            return Err(RouteError::InUse { vehicle, route });
        }
        let removed_occurrences = u64::try_from(
            slot.compiled
                .as_ref()
                .expect("live route has compiled state")
                .edges
                .len(),
        )
        .expect("route edge count fits u64");
        let removed_conflict_occurrences = u64::try_from(
            slot.compiled
                .as_ref()
                .expect("live route has compiled state")
                .conflicts
                .len(),
        )
        .expect("route conflict occurrence count fits u64");
        let next_route_count = self
            .live_route_count
            .checked_sub(1)
            .expect("live route count covers every compiled route");
        let next_occurrence_count = self
            .live_route_edge_occurrence_count
            .checked_sub(removed_occurrences)
            .expect("route occurrence count covers every compiled route");
        let next_conflict_occurrence_count = self
            .live_route_conflict_occurrence_count
            .checked_sub(removed_conflict_occurrences)
            .expect("route conflict occurrence count covers every compiled route");
        let next_command_cursor = self
            .command_cursor
            .checked_add(1)
            .ok_or(RouteError::CommandCursorExhausted)?;

        // 所有可失败预检完成后才取可变槽位并一次提交。
        let slot = self
            .routes
            .get_mut(index)
            .expect("immutable preflight proved route slot exists");
        slot.compiled = None;
        self.live_route_count = next_route_count;
        self.live_route_edge_occurrence_count = next_occurrence_count;
        self.live_route_conflict_occurrence_count = next_conflict_occurrence_count;
        let mut recyclable = false;
        if let Some(next_generation) = slot.generation.checked_add(1) {
            slot.generation = next_generation;
            self.free_routes.push(index);
            recyclable = true;
        }
        self.command_cursor = next_command_cursor;
        if let Some(journal) = self.migration_journal.as_mut() {
            journal.record_route_removed(
                next_command_cursor,
                route.index(),
                recyclable,
                self.routes[index].generation,
            );
        }
        Ok(())
    }

    /// 生成一辆车。失败不留半辆车。
    pub fn spawn_vehicle(&mut self, input: VehicleSpawnInput) -> Result<VehicleHandle, SpawnError> {
        let (class, length_mm, traversal) =
            self.validate_unparked_vehicle(input, 0, VehicleStatus::Active, None, false)?;
        let next_observation_state_sequence = self
            .observation_state_sequence
            .checked_next()
            .ok_or(SpawnError::ObservationStateSequenceExhausted)?;
        let next_command_cursor = self
            .command_cursor
            .checked_add(1)
            .ok_or(SpawnError::CommandCursorExhausted)?;

        let authority = UnparkedVehicleAuthority {
            class,
            length_mm,
            maneuver_traversal: traversal,
            waiting_membership: None,
        };
        let (handle, state) =
            self.commit_unparked_vehicle(input, 0, VehicleStatus::Active, authority);
        self.observation_state_sequence = next_observation_state_sequence;
        self.command_cursor = next_command_cursor;
        let delta = VehicleDelta::from_state(&state, self.compiled_route(state.route));
        if let Some(journal) = self.migration_journal.as_mut() {
            journal.record_vehicle_spawned(next_command_cursor, delta);
        }
        Ok(handle)
    }

    /// 快照恢复专用：以最终 `Active` / `Completed` 状态一次提交，
    /// 不经过临时 `Active` 状态，也不生成已被快照游标覆盖的恢复命令。
    pub(crate) fn restore_unparked_vehicle(
        &mut self,
        input: VehicleSpawnInput,
        carry_um: u16,
        status: VehicleStatus,
        maneuver_traversal: Option<crate::ManeuverTraversalState>,
        waiting_membership: Option<crate::WaitingMembership>,
        conflict_authority_pending: bool,
    ) -> Result<VehicleHandle, SpawnError> {
        debug_assert!(matches!(
            status,
            VehicleStatus::Active | VehicleStatus::Completed
        ));
        let (class, length_mm, traversal) = self.validate_unparked_vehicle(
            input,
            carry_um,
            status,
            Some(maneuver_traversal),
            conflict_authority_pending,
        )?;
        let authority = UnparkedVehicleAuthority {
            class,
            length_mm,
            maneuver_traversal: traversal,
            waiting_membership,
        };
        let (handle, _) = self.commit_unparked_vehicle(input, carry_um, status, authority);
        Ok(handle)
    }

    fn validate_unparked_vehicle(
        &self,
        input: VehicleSpawnInput,
        carry_um: u16,
        status: VehicleStatus,
        restored_traversal: Option<Option<crate::ManeuverTraversalState>>,
        conflict_authority_pending: bool,
    ) -> Result<
        (
            ParticipantClassOrdinal,
            u32,
            Option<crate::ManeuverTraversalState>,
        ),
        SpawnError,
    > {
        let live = u32::try_from(self.live_order.len()).expect("live vehicle count fits u32");
        if live >= self.config.vehicle_capacity() {
            return Err(SpawnError::CapacityExceeded);
        }
        let profile = self
            .revision
            .traffic()
            .relations()
            .vehicle_profile(input.profile())
            .ok_or(SpawnError::UnknownProfile)?;
        let cursor = usize::try_from(input.route_edge_index()).expect("route index fits usize");
        let edge = {
            let edges = self
                .route_edges(input.route())
                .ok_or(SpawnError::UnknownRoute)?;
            *edges.get(cursor).ok_or(SpawnError::RouteIndexOutOfRange)?
        };
        let length_mm = self.revision.traffic().lane_lengths_millimetres()[edge.index()];
        if input.progress_mm() > length_mm {
            return Err(SpawnError::InvalidProgress);
        }
        let speed_limit = self
            .revision
            .traffic()
            .lane_speed_limits_millimetres_per_second()[edge.index()];
        if input.initial_speed_mm_s() > speed_limit {
            return Err(SpawnError::SpeedExceedsLimit);
        }
        if self.route_suffix_denied(input.route(), profile.class(), cursor) {
            return Err(SpawnError::AccessDenied);
        }
        let traversal = if status == VehicleStatus::Active {
            if let Some(traversal) = restored_traversal {
                traversal
            } else {
                self.validate_waiting_bootstrap(input.route(), cursor, profile.length_mm())
                    .map_err(|error| match error {
                        crate::waiting::WaitingBindingError::VehicleTooLong => {
                            SpawnError::WaitingVehicleTooLong
                        }
                        crate::waiting::WaitingBindingError::StatefulManeuverInterior => {
                            SpawnError::WaitingStatefulManeuverInterior
                        }
                        crate::waiting::WaitingBindingError::InvalidRoute
                        | crate::waiting::WaitingBindingError::AuthorityMismatch
                        | crate::waiting::WaitingBindingError::ParkingConflict => {
                            SpawnError::InvalidProgress
                        }
                    })?
            }
        } else {
            None
        };
        if status == VehicleStatus::Active {
            if self
                .overlap_blocker(
                    input.route(),
                    cursor,
                    input.progress_mm(),
                    profile.length_mm(),
                )
                .is_some()
            {
                return Err(SpawnError::Overlap);
            }
            match self.check_active_conflict_capability(
                input.route(),
                cursor,
                input.progress_mm(),
                carry_um,
                profile.length_mm(),
            ) {
                Ok(()) => {}
                Err(ConflictCapabilityError::InvalidCursor) => {
                    return Err(SpawnError::InvalidProgress);
                }
                Err(ConflictCapabilityError::RuntimeUnavailable(_))
                    if conflict_authority_pending => {}
                Err(ConflictCapabilityError::RuntimeUnavailable(error)) => {
                    return Err(SpawnError::ConflictRuntimeUnavailable(error));
                }
            }
        }
        Ok((profile.class(), profile.length_mm(), traversal))
    }

    fn commit_unparked_vehicle(
        &mut self,
        input: VehicleSpawnInput,
        carry_um: u16,
        status: VehicleStatus,
        authority: UnparkedVehicleAuthority,
    ) -> (VehicleHandle, VehicleState) {
        let slot_index = self.free_vehicles.pop().unwrap_or(self.vehicles.len());
        let generation = self
            .vehicles
            .get(slot_index)
            .map_or(0, |slot| slot.generation);
        let handle = VehicleHandle::new(
            u32::try_from(slot_index).expect("vehicle index fits u32"),
            generation,
        );
        let state = VehicleState {
            handle,
            profile: input.profile(),
            class: authority.class,
            route: input.route(),
            route_edge_index: input.route_edge_index(),
            progress_mm: input.progress_mm(),
            carry_um,
            speed_mm_s: input.initial_speed_mm_s(),
            length_mm: authority.length_mm,
            status,
            maneuver_traversal: authority.maneuver_traversal,
            waiting_membership: authority.waiting_membership,
        };
        if let Some(eligibility) = self.conflict_eligibility.get_mut(slot_index) {
            *eligibility = None;
        }
        let slot = VehicleSlot {
            generation,
            state: Some(state),
        };
        if slot_index == self.vehicles.len() {
            self.vehicles.push(slot);
        } else {
            self.vehicles[slot_index] = slot;
        }
        let route_index = usize::try_from(input.route().index()).expect("route index fits usize");
        self.routes[route_index].live_vehicles += 1;
        self.live_order.push(handle);
        if status == VehicleStatus::Active {
            self.active_order.push(handle);
        }
        (handle, state)
    }

    /// 把 live 的 Completed 车辆原子替换为新的 Active 车辆。
    ///
    /// 入口占用返回可重试的 [`ReplaceError::Blocked`]；其他失败为致命错误。
    /// 任一失败都保持已提交世界不变。成功后旧句柄立即 stale；公开契约不保证同一 slot index。
    pub fn replace_completed_vehicle(
        &mut self,
        old: VehicleHandle,
        input: VehicleSpawnInput,
    ) -> Result<VehicleReplaceRecord, ReplaceError> {
        let old_state = self.vehicle_state(old).ok_or(ReplaceError::StaleHandle)?;
        if old_state.status != VehicleStatus::Completed {
            return Err(ReplaceError::NotCompleted);
        }
        if self.parking.binding(old).is_some() {
            return Err(ReplaceError::ParkingOccupied);
        }
        if self.conflict_reservation(old).is_some() || self.conflict_arbiter.has_authority(old) {
            return Err(ReplaceError::ConflictInvariantViolation);
        }
        if old_state.maneuver_traversal.is_some() || old_state.waiting_membership.is_some() {
            return Err(ReplaceError::WaitingInvariantViolation);
        }
        let Some(order_index) = self.live_order.iter().position(|handle| *handle == old) else {
            return Err(ReplaceError::StaleHandle);
        };

        let profile = self
            .revision
            .traffic()
            .relations()
            .vehicle_profile(input.profile())
            .ok_or(ReplaceError::UnknownProfile)?;
        let cursor = usize::try_from(input.route_edge_index()).expect("route index fits usize");
        let edge = {
            let edges = self
                .route_edges(input.route())
                .ok_or(ReplaceError::UnknownRoute)?;
            *edges
                .get(cursor)
                .ok_or(ReplaceError::RouteIndexOutOfRange)?
        };
        let length_mm = self.revision.traffic().lane_lengths_millimetres()[edge.index()];
        if input.progress_mm() > length_mm {
            return Err(ReplaceError::InvalidProgress);
        }
        let speed_limit = self
            .revision
            .traffic()
            .lane_speed_limits_millimetres_per_second()[edge.index()];
        if input.initial_speed_mm_s() > speed_limit {
            return Err(ReplaceError::SpeedExceedsLimit);
        }
        if self.route_suffix_denied(input.route(), profile.class(), cursor) {
            return Err(ReplaceError::AccessDenied);
        }
        let traversal = self
            .validate_waiting_bootstrap(input.route(), cursor, profile.length_mm())
            .map_err(|error| match error {
                crate::waiting::WaitingBindingError::VehicleTooLong => {
                    ReplaceError::WaitingVehicleTooLong
                }
                crate::waiting::WaitingBindingError::StatefulManeuverInterior => {
                    ReplaceError::WaitingStatefulManeuverInterior
                }
                crate::waiting::WaitingBindingError::InvalidRoute
                | crate::waiting::WaitingBindingError::AuthorityMismatch
                | crate::waiting::WaitingBindingError::ParkingConflict => {
                    ReplaceError::InvalidProgress
                }
            })?;
        if let Some(blocker) = self.overlap_blocker(
            input.route(),
            cursor,
            input.progress_mm(),
            profile.length_mm(),
        ) {
            let (blocker_ahead, bumper_gap) = self.overlap_relation(
                input.route(),
                cursor,
                input.progress_mm(),
                profile.length_mm(),
                blocker,
            );
            return Err(ReplaceError::Blocked(VehicleReplaceBlock {
                old,
                blocker,
                blocker_ahead,
                bumper_gap,
            }));
        }
        match self.check_active_conflict_capability(
            input.route(),
            cursor,
            input.progress_mm(),
            0,
            profile.length_mm(),
        ) {
            Ok(()) => {}
            Err(ConflictCapabilityError::InvalidCursor) => {
                return Err(ReplaceError::InvalidProgress);
            }
            Err(ConflictCapabilityError::RuntimeUnavailable(error)) => {
                return Err(ReplaceError::ConflictRuntimeUnavailable(error));
            }
        }
        let next_observation_state_sequence = self
            .observation_state_sequence
            .checked_next()
            .ok_or(ReplaceError::ObservationStateSequenceExhausted)?;
        let next_command_cursor = self
            .command_cursor
            .checked_add(1)
            .ok_or(ReplaceError::CommandCursorExhausted)?;

        let old_route = old_state.route;
        let old_index = usize::try_from(old.index()).expect("vehicle index fits usize");
        let reusable_generation = self.vehicles[old_index].generation.checked_add(1);
        let slot_index = reusable_generation.map_or_else(
            || self.free_vehicles.pop().unwrap_or(self.vehicles.len()),
            |_| old_index,
        );
        let generation = reusable_generation.unwrap_or_else(|| {
            self.vehicles
                .get(slot_index)
                .map_or(0, |slot| slot.generation)
        });
        let new = VehicleHandle::new(
            u32::try_from(slot_index).expect("vehicle index fits u32"),
            generation,
        );
        let state = VehicleState {
            handle: new,
            profile: input.profile(),
            class: profile.class(),
            route: input.route(),
            route_edge_index: input.route_edge_index(),
            progress_mm: input.progress_mm(),
            carry_um: 0,
            speed_mm_s: input.initial_speed_mm_s(),
            length_mm: profile.length_mm(),
            status: VehicleStatus::Active,
            maneuver_traversal: traversal,
            waiting_membership: None,
        };
        if let Some(eligibility) = self.conflict_eligibility.get_mut(slot_index) {
            *eligibility = None;
        }

        if reusable_generation.is_some() {
            self.vehicles[old_index] = VehicleSlot {
                generation,
                state: Some(state),
            };
        } else {
            self.vehicles[old_index].state = None;
            let slot = VehicleSlot {
                generation,
                state: Some(state),
            };
            if slot_index == self.vehicles.len() {
                self.vehicles.push(slot);
            } else {
                self.vehicles[slot_index] = slot;
            }
        }
        self.release_route_ref(old_route);
        let route_index = usize::try_from(input.route().index()).expect("route index fits usize");
        self.routes[route_index].live_vehicles += 1;
        self.live_order[order_index] = new;
        self.rebuild_active_order();
        self.observation_state_sequence = next_observation_state_sequence;
        self.command_cursor = next_command_cursor;
        let new_state = self
            .vehicle_state(new)
            .copied()
            .expect("freshly committed replacement vehicle");
        let new_delta = VehicleDelta::from_state(&new_state, self.compiled_route(new_state.route));
        if let Some(journal) = self.migration_journal.as_mut() {
            journal.record_vehicle_replaced(
                next_command_cursor,
                old,
                u32::try_from(order_index).expect("live order index fits u32"),
                new_delta,
            );
        }
        Ok(VehicleReplaceRecord { old, new })
    }

    /// 固定步进。`delta_time_ms` 必须等于 `WorldConfig.fixed_delta_time_ms`；
    /// `tick_index`/`time_ms` 用 checked 加法。运动、跟车与信号遵守只读 snapshot(T)；
    /// 成功后再提交 T+D 的 pose、时间与 `committed_signal_groups`。相位边界落在
    /// `[T, T+D)` 时该拍仍用 snapshot(T) 灯色。失败不推进时间，已提交查询与失败前一致。
    /// 生命周期命令只在两次 `step` 之间调用。
    pub fn step(&mut self, input: TickInput) -> Result<StepOutcome, StepError> {
        self.step_vehicles(input)
    }

    /// 稳定顺序的已提交 pose 源。
    #[must_use]
    pub fn committed_pose_sources(&self) -> CommittedPoseSourceBatch {
        let items = self
            .live_order
            .iter()
            .copied()
            .filter_map(|handle| {
                let state = self.vehicle_state(handle)?;
                let source = match state.status {
                    VehicleStatus::Completed => return None,
                    VehicleStatus::Parked => match self.parking.binding(handle) {
                        Some(ParkingBinding::Occupied(ParkingTarget::ExplicitSpace(space))) => {
                            PoseSource::Parking { space }
                        }
                        Some(ParkingBinding::Occupied(ParkingTarget::VirtualPool(_))) => {
                            return None;
                        }
                        _ => return None,
                    },
                    VehicleStatus::Active => {
                        let edges = self.route_edges(state.route)?;
                        let edge = *edges.get(usize::try_from(state.route_edge_index).ok()?)?;
                        PoseSource::Lane {
                            edge,
                            progress_mm: state.progress_mm,
                        }
                    }
                };
                Some((handle, source))
            })
            .collect();
        CommittedPoseSourceBatch { items }
    }

    /// 按停车位序号读占用者。
    #[must_use]
    pub fn committed_parking_occupant(&self, space: ParkingSpaceOrdinal) -> Option<VehicleHandle> {
        match self.parking.explicit_state(space)? {
            ParkingSpaceState::Occupied(vehicle) => Some(vehicle),
            ParkingSpaceState::Vacant | ParkingSpaceState::Reserved(_) => None,
        }
    }

    /// 车辆的只读 tagged parking binding。
    #[must_use]
    pub fn parking_binding(&self, vehicle: VehicleHandle) -> Option<ParkingBinding> {
        self.vehicle_state(vehicle)?;
        self.parking.binding(vehicle)
    }

    /// 显式泊位的排他资源状态。
    #[must_use]
    pub fn parking_space_state(&self, space: ParkingSpaceOrdinal) -> Option<ParkingSpaceState> {
        self.revision.traffic().relations().parking_space(space)?;
        self.parking.explicit_state(space)
    }

    /// 设施显式池、虚拟池和总量的守恒查询。
    #[must_use]
    pub fn parking_facility_counts(
        &self,
        facility: laneflow_static_contract::ParkingFacilityOrdinal,
    ) -> Option<ParkingFacilityCounts> {
        let view = self
            .revision
            .traffic()
            .relations()
            .parking_facility(facility)?;
        let mut explicit_reserved = 0_u64;
        let mut explicit_occupied = 0_u64;
        for &space in view.spaces() {
            match self.parking.explicit_state(space)? {
                ParkingSpaceState::Vacant => {}
                ParkingSpaceState::Reserved(_) => explicit_reserved += 1,
                ParkingSpaceState::Occupied(_) => explicit_occupied += 1,
            }
        }
        let virtual_state = self.parking.virtual_state(facility)?;
        let explicit = ParkingPoolCounts::checked(
            u64::try_from(view.spaces().len()).ok()?,
            explicit_reserved,
            explicit_occupied,
        )?;
        let virtual_pool = ParkingPoolCounts::checked(
            u64::from(view.virtual_capacity()),
            u64::from(virtual_state.reserved_count),
            u64::from(virtual_state.occupied_count),
        )?;
        let total = ParkingPoolCounts::checked(
            explicit.capacity.checked_add(virtual_pool.capacity)?,
            explicit.reserved.checked_add(virtual_pool.reserved)?,
            explicit.occupied.checked_add(virtual_pool.occupied)?,
        )?;
        Some(ParkingFacilityCounts {
            explicit,
            virtual_pool,
            total,
        })
    }

    /// 稳定按组序号的当前 aspect。
    #[must_use]
    pub fn committed_signal_groups(&self) -> CommittedSignalGroupBatch {
        let items = self
            .signal_aspects
            .iter()
            .enumerate()
            .map(|(index, aspect)| {
                (
                    SignalGroupOrdinal::from_raw(
                        u32::try_from(index).expect("signal group index fits u32"),
                    ),
                    *aspect,
                )
            })
            .collect();
        CommittedSignalGroupBatch { items }
    }

    /// 已提交车辆快照。`Completed` 仍可读；stale 句柄返回 `None`。
    #[must_use]
    pub fn vehicle(&self, handle: VehicleHandle) -> Option<VehicleState> {
        self.vehicle_state(handle).copied()
    }

    /// WaitingZone 的已提交计数；未知 zone 返回 `None`。
    #[must_use]
    pub fn waiting_zone(
        &self,
        zone: laneflow_static_contract::WaitingZoneOrdinal,
    ) -> Option<crate::WaitingZoneSnapshot> {
        let state = *self.waiting_zones.get(zone.index())?;
        let max_occupancy = self
            .revision
            .traffic()
            .relations()
            .waiting_zone(zone)?
            .max_occupancy();
        Some(crate::WaitingZoneSnapshot {
            zone,
            occupancy: state.occupancy,
            max_occupancy,
            next_admission_sequence: state.next_admission_sequence,
        })
    }

    /// 按 `(zone ordinal, admission sequence)` 排列的全部 Waiting member。
    #[must_use]
    pub fn waiting_zone_members(&self) -> &[crate::WaitingZoneMember] {
        &self.waiting_member_rows
    }

    /// 刚完成 successful tick 的 Waiting admission decision batch。
    /// 跨修订成功切换后置空；需要历史记录的调用方必须在切换前消费。
    /// 同修订切换、生命周期命令和失败操作保留原批次。
    #[must_use]
    pub fn latest_waiting_decisions(&self) -> &[crate::WaitingDecision] {
        &self.latest_waiting_decisions
    }

    /// 刚完成 successful tick 的 Waiting transition event batch。
    /// 跨修订成功切换后置空；需要历史记录的调用方必须在切换前消费。
    /// 同修订切换、生命周期命令和失败操作保留原批次。
    #[must_use]
    pub fn latest_waiting_events(&self) -> &[crate::WaitingTransitionEvent] {
        &self.latest_waiting_events
    }

    /// 稳定更新顺序，含 Active / Parked / Completed。
    #[must_use]
    pub fn live_vehicles(&self) -> &[VehicleHandle] {
        &self.live_order
    }

    pub(crate) fn vehicle_state(&self, handle: VehicleHandle) -> Option<&VehicleState> {
        let slot = self.vehicles.get(usize::try_from(handle.index()).ok()?)?;
        if slot.generation != handle.generation() {
            return None;
        }
        slot.state.as_ref()
    }

    /// 本世界已注册路线的边序列。句柄无效时返回 `None`。
    #[must_use]
    pub fn route_edges(
        &self,
        route: RouteHandle,
    ) -> Option<&[laneflow_static_contract::LaneEdgeOrdinal]> {
        Some(self.compiled_route(route)?.edges.as_slice())
    }

    /// 本世界当前有效路线句柄，按槽位下标。
    pub fn live_routes(&self) -> impl Iterator<Item = RouteHandle> + '_ {
        self.routes.iter().enumerate().filter_map(|(index, slot)| {
            slot.compiled.as_ref().map(|_| {
                RouteHandle::new(
                    u32::try_from(index).expect("route index fits u32"),
                    slot.generation,
                )
            })
        })
    }

    pub(crate) fn compiled_route(&self, route: RouteHandle) -> Option<&CompiledRoute> {
        let slot = self.routes.get(usize::try_from(route.index()).ok()?)?;
        if slot.generation != route.generation() {
            return None;
        }
        slot.compiled.as_ref()
    }

    pub(crate) fn check_active_conflict_capability(
        &self,
        route: RouteHandle,
        cursor: usize,
        progress_mm: u32,
        carry_um: u16,
        vehicle_length_mm: u32,
    ) -> Result<(), ConflictCapabilityError> {
        let compiled = self
            .compiled_route(route)
            .ok_or(ConflictCapabilityError::InvalidCursor)?;
        check_conflict_capability(
            route,
            compiled,
            self.revision.traffic().lane_lengths_millimetres(),
            cursor,
            progress_mm,
            carry_um,
            vehicle_length_mm,
        )
    }

    pub(crate) fn route_suffix_denied(
        &self,
        route: RouteHandle,
        class: laneflow_static_contract::ParticipantClassOrdinal,
        cursor: usize,
    ) -> bool {
        let Some(edges) = self.route_edges(route) else {
            return true;
        };
        let Some(compiled) = self.compiled_route(route) else {
            return true;
        };
        route_access_denied(
            self.revision.traffic(),
            class,
            edges,
            cursor,
            compiled
                .maneuvers
                .iter()
                .map(|occurrence| (occurrence.path, occurrence.exit_route_edge_index)),
        )
    }

    pub(crate) fn overlap_blocker(
        &self,
        route: RouteHandle,
        cursor: usize,
        progress: u32,
        length: u32,
    ) -> Option<VehicleHandle> {
        let spawn_edges = self.route_edges(route)?;
        let lengths = self.revision.traffic().lane_lengths_millimetres();
        self.active_order.iter().copied().find(|&handle| {
            #[cfg(test)]
            OVERLAP_BLOCKER_INSPECTIONS.set(OVERLAP_BLOCKER_INSPECTIONS.get() + 1);
            let Some(state) = self.vehicle_state(handle) else {
                return false;
            };
            if state.status != VehicleStatus::Active {
                return false;
            }
            let Some(edges) = self.route_edges(state.route) else {
                return false;
            };
            let Ok(index) = usize::try_from(state.route_edge_index) else {
                return false;
            };
            bodies_overlap(
                lengths,
                spawn_edges,
                cursor,
                progress,
                length,
                edges,
                index,
                state.progress_mm,
                state.length_mm,
            )
        })
    }

    fn overlap_relation(
        &self,
        route: RouteHandle,
        cursor: usize,
        progress: u32,
        length: u32,
        blocker: VehicleHandle,
    ) -> (bool, i64) {
        let Some(spawn_edges) = self.route_edges(route) else {
            return (true, 0);
        };
        let Some(leader) = self.vehicle_state(blocker) else {
            return (true, 0);
        };
        let Some(blocker_edges) = self.route_edges(leader.route) else {
            return (true, 0);
        };
        let Ok(blocker_index) = usize::try_from(leader.route_edge_index) else {
            return (true, 0);
        };
        let lengths = self.revision.traffic().lane_lengths_millimetres();
        if let Some(gap) = occupancy_front_gap(
            lengths,
            spawn_edges,
            cursor,
            progress,
            blocker_edges,
            blocker_index,
            leader.progress_mm,
            leader.length_mm,
        ) {
            return (true, gap);
        }
        if let Some(gap) = occupancy_front_gap(
            lengths,
            blocker_edges,
            blocker_index,
            leader.progress_mm,
            spawn_edges,
            cursor,
            progress,
            length,
        ) {
            return (false, gap);
        }
        (true, 0)
    }

    pub(crate) fn release_route_ref(&mut self, route: RouteHandle) {
        let Ok(index) = usize::try_from(route.index()) else {
            return;
        };
        let Some(slot) = self.routes.get_mut(index) else {
            return;
        };
        if slot.generation != route.generation() || slot.compiled.is_none() {
            return;
        }
        slot.live_vehicles = slot.live_vehicles.saturating_sub(1);
    }

    pub(crate) fn rebuild_active_order(&mut self) {
        let vehicles = &self.vehicles;
        self.active_order.clear();
        for handle in self.live_order.iter().copied() {
            let index = usize::try_from(handle.index()).expect("vehicle index fits usize");
            if vehicles.get(index).is_some_and(|slot| {
                slot.generation == handle.generation()
                    && slot
                        .state
                        .as_ref()
                        .is_some_and(|state| state.status == VehicleStatus::Active)
            }) {
                self.active_order.push(handle);
            }
        }
    }

    pub(crate) fn refresh_signals(&mut self) {
        fill_signal_aspects(
            self.revision.as_ref(),
            self.time_ms,
            &mut self.signal_aspects,
        );
    }
}

fn map_conflict_install_error(error: crate::conflict::ConflictInstallError) -> InstallError {
    match error {
        crate::conflict::ConflictInstallError::InvalidNetwork => {
            InstallError::ConflictArbiterInvalidNetwork
        }
        crate::conflict::ConflictInstallError::CapacityOverflow => {
            InstallError::ConflictArbiterCapacityOverflow
        }
        crate::conflict::ConflictInstallError::AllocationFailed => {
            InstallError::ConflictArbiterAllocationFailed
        }
    }
}

#[cfg(test)]
mod conflict_install_error_tests {
    use super::*;

    #[test]
    fn conflict_install_errors_keep_distinct_host_remediation_semantics() {
        assert_eq!(
            map_conflict_install_error(crate::conflict::ConflictInstallError::InvalidNetwork),
            InstallError::ConflictArbiterInvalidNetwork
        );
        assert_eq!(
            map_conflict_install_error(crate::conflict::ConflictInstallError::CapacityOverflow),
            InstallError::ConflictArbiterCapacityOverflow
        );
        assert_eq!(
            map_conflict_install_error(crate::conflict::ConflictInstallError::AllocationFailed),
            InstallError::ConflictArbiterAllocationFailed
        );
    }
}

pub(crate) fn fill_signal_aspects(
    revision: &SharedNetworkRevision,
    time_ms: u64,
    aspects: &mut [SignalAspect],
) {
    aspects.fill(SignalAspect::Red);
    let relations = revision.traffic().relations();
    let controller_count = revision
        .traffic()
        .entity_counts()
        .count(EntityKind::SignalController);
    for raw in 0..controller_count {
        let controller = SignalControllerOrdinal::from_raw(raw);
        let Some(view) = relations.signal_controller(controller) else {
            continue;
        };
        let cycle_ms = view.cycle_ms();
        if cycle_ms == 0 || view.phases().is_empty() {
            continue;
        }
        let position = u64::try_from(
            (u128::from(time_ms) + u128::from(view.offset_ms())) % u128::from(cycle_ms),
        )
        .expect("cycle position fits u64");
        let phases = view.phases();
        let phase_index = phases.partition_point(|phase| {
            relations.phase_end_offset_ms(*phase).unwrap_or(0) <= position
        });
        let Some(phase) = phases.get(phase_index).copied() else {
            continue;
        };
        let Some((groups, values)) = relations.phase_states(phase) else {
            continue;
        };
        for (group, aspect) in groups.iter().copied().zip(values.iter().copied()) {
            if let Some(slot) = aspects.get_mut(group.index()) {
                *slot = aspect;
            }
        }
    }
}

pub(crate) fn validate_signal_programs(
    revision: &SharedNetworkRevision,
    fixed_delta_time_ms: u64,
) -> Result<(), InstallError> {
    let relations = revision.traffic().relations();
    let controller_count = revision
        .traffic()
        .entity_counts()
        .count(EntityKind::SignalController);
    for raw in 0..controller_count {
        let controller = SignalControllerOrdinal::from_raw(raw);
        let Some(view) = relations.signal_controller(controller) else {
            return Err(InstallError::InvalidSignalProgram);
        };
        if view.cycle_ms() == 0 || view.phases().is_empty() {
            return Err(InstallError::InvalidSignalProgram);
        }
        for phase in view.phases() {
            let Some(duration_ms) = relations.phase_duration_ms(*phase) else {
                return Err(InstallError::InvalidSignalProgram);
            };
            if duration_ms < fixed_delta_time_ms {
                return Err(InstallError::PhaseShorterThanTick);
            }
            if duration_ms % fixed_delta_time_ms != 0 {
                return Err(InstallError::PhaseNotMultipleOfTick);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod overflow_tests {
    use super::*;
    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_static_contract::{LaneEdgeOrdinal, VehicleProfileOrdinal};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
    );

    fn world() -> TrafficWorld {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
            .expect("checked canonical network input");
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("shared network revision");
        let origin = *revision.canonical_origin();
        TrafficWorld::install(
            std::sync::Arc::clone(&revision),
            WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
            CommittedNetworkSource::Published {
                reference: crate::PublishedLfcaReference::new(
                    "fixture://overflow-tests",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .expect("non-empty fixture key"),
            },
            0,
            crate::test_policy::selection(&revision),
        )
        .expect("install")
    }

    #[test]
    fn step_rejects_tick_and_time_overflow() {
        let mut world = world();
        world.tick_index = u64::MAX;
        world.time_ms = 0;
        assert_eq!(
            world.step(TickInput::new(100)).unwrap_err(),
            StepError::Overflow
        );
        assert_eq!(world.tick_index, u64::MAX);
        assert_eq!(world.time_ms, 0);

        world.tick_index = 0;
        world.time_ms = u64::MAX;
        assert_eq!(
            world.step(TickInput::new(100)).unwrap_err(),
            StepError::Overflow
        );
        assert_eq!(world.tick_index, 0);
        assert_eq!(world.time_ms, u64::MAX);
    }

    #[test]
    fn overlap_blocker_inspects_only_active_order() {
        let mut world = world();
        let edge_for_length = |world: &TrafficWorld, length: u32| {
            let index = world
                .traffic()
                .lane_lengths_millimetres()
                .iter()
                .position(|actual| *actual == length)
                .expect("fixture LaneEdge length");
            LaneEdgeOrdinal::try_from_usize(index).expect("fixture LaneEdge ordinal")
        };
        let route = world
            .register_route(RouteRegisterInput::new(vec![
                edge_for_length(&world, 10_000),
                edge_for_length(&world, 8_000),
                edge_for_length(&world, 12_000),
            ]))
            .expect("register route");
        let profile = VehicleProfileOrdinal::from_raw(0);
        let positions = [(0, 0), (0, 6_000), (1, 2_000), (2, 0)];
        let vehicles = positions.map(|(occurrence, progress)| {
            let spawn_occurrence = if occurrence == 1 { 2 } else { occurrence };
            let spawn_progress = if occurrence == 1 { 6_000 } else { progress };
            let vehicle = world
                .spawn_vehicle(VehicleSpawnInput::new(
                    profile,
                    route,
                    spawn_occurrence,
                    spawn_progress,
                    0,
                ))
                .expect("non-overlapping vehicle");
            if occurrence == 1 {
                let index = usize::try_from(vehicle.index()).expect("vehicle index");
                let state = world.vehicles[index].state.as_mut().expect("vehicle");
                state.route_edge_index = occurrence;
                state.progress_mm = progress;
            }
            vehicle
        });
        for vehicle in vehicles.iter().take(3).copied() {
            let index = usize::try_from(vehicle.index()).expect("vehicle index");
            world.vehicles[index]
                .state
                .as_mut()
                .expect("live vehicle")
                .status = VehicleStatus::Parked;
        }
        world.rebuild_active_order();
        assert_eq!(world.live_order.len(), 4);
        assert_eq!(world.active_order.len(), 1);

        let vehicle_length = world
            .traffic()
            .relations()
            .vehicle_profile(profile)
            .expect("profile")
            .length_mm();
        reset_overlap_blocker_inspections();
        assert_eq!(world.overlap_blocker(route, 0, 0, vehicle_length), None);
        assert_eq!(overlap_blocker_inspections(), 1);
    }
}

#[cfg(test)]
mod source_tests {
    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_static_contract::{ExactByteLength, NetworkRevisionId, Sha256Digest};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    use super::*;

    use crate::PublishedLfcaReference;

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
    );

    fn revision() -> Arc<SharedNetworkRevision> {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
            .expect("checked canonical network input");
        build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("shared network revision")
    }

    fn reference_for(origin_revision: NetworkRevisionId) -> PublishedLfcaReference {
        PublishedLfcaReference::new(
            "asset://city/roads",
            Sha256Digest::from_bytes([9; 32]),
            ExactByteLength::new(4_096),
            origin_revision,
        )
        .expect("non-empty key")
    }

    #[test]
    fn install_binds_matching_revision() {
        let revision = revision();
        let origin = *revision.canonical_origin();
        // digest / length 与根 origin 不同（重发布语义），同修订判据只比对修订标识。
        let reference = reference_for(origin.network_revision());
        let world = TrafficWorld::install(
            revision.clone(),
            WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
            CommittedNetworkSource::Published { reference },
            0,
            crate::test_policy::selection(&revision),
        )
        .expect("source matches installed revision");
        assert_eq!(world.world_generation(), WorldGeneration::INITIAL);
        assert_eq!(
            world.committed_source(),
            &CommittedNetworkSource::Published {
                reference: reference_for(origin.network_revision())
            }
        );
        assert_eq!(
            world.committed_source().network_revision(),
            revision.canonical_origin().network_revision()
        );
    }

    #[test]
    fn install_rejects_revision_mismatch() {
        let revision = revision();
        let mismatched = NetworkRevisionId::from_digest(Sha256Digest::from_bytes([1; 32]));
        let error = match TrafficWorld::install(
            revision.clone(),
            WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
            CommittedNetworkSource::Published {
                reference: reference_for(mismatched),
            },
            0,
            crate::test_policy::selection(&revision),
        ) {
            Err(error) => error,
            Ok(_) => panic!("mismatched revision must fail closed"),
        };
        assert_eq!(
            error,
            InstallError::SourceRevisionMismatch {
                source_revision: mismatched,
                installed_revision: revision.canonical_origin().network_revision(),
            }
        );
    }
}
