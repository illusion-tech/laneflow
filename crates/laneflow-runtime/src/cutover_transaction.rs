//! 切换事务对象（#302 切换合同 §4/§5；#513 切片 C-3）。
//!
//! Prepare →（宿主泵式 Delta Catch-up）→ Quiescent Commit / 放弃。候选由
//! Prepare 时的结构克隆直移构造（切片 C-2），此后只应用迁移增量（切片 C-1
//! 日志）：不模拟未来、不重执行输入命令。静默提交在同一原子边界排空日志
//! 尾 → 占用重建 + 全量重验证 → 确定性摘要复核（期望值 = 旧世界静默点
//! 捕获的头部替换形式：`origin` 换为 target，记录内容全部按稳定引用键控、
//! 直移不改写）→ 最终游标原子取样 → 不可失败原地晋升（世代 checked+1、
//! 观测序号同界重置）。任一失败整体放弃：旧修订、旧动态状态、旧来源
//! 原样生效、零事件。

use std::sync::Arc;

use laneflow_static_contract::{
    LaneEdgeOrdinal, ParkingSpaceOrdinal, ParticipantClassOrdinal, VehicleProfileOrdinal,
};
use laneflow_static_network::SharedNetworkRevision;

use crate::cutover::{
    CutoverError, CutoverEventBatch, CutoverPreflightLimits, MigrationPolicyKind,
    NetworkRevisionCutoverDescriptor,
};
use crate::cutover_migration::{
    CrossRevisionRebinding, migrate_structural_clone, revalidate_migrated_vehicles,
    revalidate_vehicle_on,
};
use crate::migration_journal::{
    DEFAULT_MIGRATION_DELTA_JOURNAL_BYTES, JournalRecord, VEHICLE_DELTA_BYTES, VehicleDelta,
    raw_u32_stream,
};
use crate::snapshot_digest::deterministic_state_digest;
use crate::{
    CommittedNetworkSource, ObservationStateSequence, RouteHandle, TrafficWorld, VehicleHandle,
    VehicleState, WorldGeneration,
};

/// 最大追赶滞后的文档化默认值（tick 距离；切换合同 §9）。
///
/// v1 取 600 tick：4 ms 固定步进下约 2.4 s 的可追窗口；更紧的宿主预算
/// 可显式配置更小值。初值随切片 C 证据登记。
pub const DEFAULT_MAX_CATCH_UP_LAG_TICKS: u64 = 600;

/// 每泵记录数预算的文档化默认值（后台工作上限；切换合同 §9）。
pub const DEFAULT_MAX_RECORDS_PER_PUMP: u64 = 4_096;

/// 切换事务资源上限（切换合同 §9 追赶滞后与后台资源行）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CutoverTransactionLimits {
    /// 迁移增量日志字节上界（溢出失败关闭）。
    pub max_journal_bytes: u64,
    /// 最大追赶滞后（tick 距离，超限放弃候选）。
    pub max_catch_up_lag_ticks: u64,
    /// 每泵最多应用的日志记录数（后台工作预算）。
    pub max_records_per_pump: u64,
}

impl Default for CutoverTransactionLimits {
    fn default() -> Self {
        Self {
            max_journal_bytes: DEFAULT_MIGRATION_DELTA_JOURNAL_BYTES,
            max_catch_up_lag_ticks: DEFAULT_MAX_CATCH_UP_LAG_TICKS,
            max_records_per_pump: DEFAULT_MAX_RECORDS_PER_PUMP,
        }
    }
}

/// 单次泵的结果。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PumpOutcome {
    /// 本次泵应用的记录数。
    pub applied_records: u64,
    /// 是否已追平当前日志尾。
    pub caught_up: bool,
}

/// 静默提交成功记录。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CutoverCommit {
    /// 晋升后的世界世代。
    pub world_generation: WorldGeneration,
    /// 静默点原子取样的最终命令游标（半开覆盖区间的上界）。
    pub final_command_cursor: u64,
    /// 恰一次交付的切换事件批次（#302 切换合同 §6）。
    pub events: CutoverEventBatch,
}

/// 跨修订直移切换事务（切换合同 §4 状态机的进程内形态）。
///
/// 生命周期：`prepare_cross_revision_cutover` 构造 → 宿主在步进间隙反复
/// [`Self::pump`] → [`Self::commit`] 在固定步进安全边界（旧世界已停表）
/// 静默提交；任一失败（含泵入途中）都整体放弃——调用方直接丢弃事务对象，
/// 旧世界从暂停点恢复步进。事务不实现 `Drop` 副作用：解除日志武装始终
/// 显式发生在失败/提交路径内。
pub struct CutoverTransaction {
    candidate: TrafficWorld,
    rebinding: CrossRevisionRebinding,
    target_revision: Arc<SharedNetworkRevision>,
    limits: CutoverTransactionLimits,
    next_world_generation: WorldGeneration,
    applied_records: u64,
    settled: bool,
}

impl TrafficWorld {
    /// 跨修订直移切换事务的 Prepare（切换合同 §4）。
    ///
    /// 在固定步进安全边界原子完成基准捕获与迁移增量日志武装（同一调用、
    /// 无窗口），随后从该基准构造候选并返回事务对象；旧世界恢复步进。
    /// 构造失败即丢弃候选、解除武装，旧世界无感知。在途唯一：本世界已
    /// 有武装日志（在途事务）时失败关闭。
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_cross_revision_cutover(
        &mut self,
        target_revision: Arc<SharedNetworkRevision>,
        target_source: CommittedNetworkSource,
        descriptor: &NetworkRevisionCutoverDescriptor,
        lfsd_bytes: &[u8],
        limits: &CutoverPreflightLimits,
        transaction_limits: &CutoverTransactionLimits,
    ) -> Result<CutoverTransaction, CutoverError> {
        if self.migration_journal.is_some() {
            return Err(CutoverError::InFlightTransaction);
        }
        // 认证先于策略：描述符一致性（含 O(1) 预检）与 LFSD 字节级认证。
        let base_origin = *self.revision.canonical_origin();
        let target_origin = *target_revision.canonical_origin();
        descriptor.validate(base_origin, target_origin, limits)?;
        if descriptor.world_binding().world_id() != self.world_id
            || descriptor.world_binding().world_generation() != self.world_generation
        {
            return Err(CutoverError::WorldBindingMismatch);
        }
        if descriptor.world_binding().baseline_command_cursor() != self.command_cursor {
            return Err(CutoverError::BaselineCommandCursorMismatch {
                descriptor: descriptor.world_binding().baseline_command_cursor(),
                world: self.command_cursor,
            });
        }
        if descriptor.world_binding().baseline_event_cursor() != self.event_cursor {
            return Err(CutoverError::BaselineEventCursorMismatch {
                descriptor: descriptor.world_binding().baseline_event_cursor(),
                world: self.event_cursor,
            });
        }
        if descriptor.policy_kind() != MigrationPolicyKind::CrossRevisionDirect {
            return Err(CutoverError::PolicyMismatch);
        }
        if target_source.network_revision() != target_origin.network_revision() {
            return Err(CutoverError::TargetSourceRevisionMismatch);
        }
        descriptor.verify_semantic_diff(lfsd_bytes, base_origin, target_origin)?;
        // 世代耗尽必须在任何候选暂存/分配之前失败关闭。
        let next_world_generation = self
            .world_generation
            .checked_next()
            .ok_or(CutoverError::WorldGenerationExhausted)?;
        // 武装日志：以当前命令游标为半开覆盖区间下界，字节上界一次预留。
        self.arm_migration_journal(transaction_limits.max_journal_bytes)
            .map_err(|_| CutoverError::StagingAllocFailed)?;
        // 基准捕获（结构克隆）+ 直移构造候选；失败即解除武装。
        let rebinding =
            CrossRevisionRebinding::build(self.revision.identity(), target_revision.identity());
        let candidate = match migrate_structural_clone(
            self,
            Arc::clone(&target_revision),
            target_source,
            &rebinding,
        ) {
            Ok(candidate) => candidate,
            Err(error) => {
                self.disarm_migration_journal();
                return Err(error);
            }
        };
        Ok(CutoverTransaction {
            candidate,
            rebinding,
            target_revision,
            limits: *transaction_limits,
            next_world_generation,
            applied_records: 0,
            settled: false,
        })
    }
}

impl CutoverTransaction {
    /// 候选当前已追到的 tick（滞后观测）。
    #[must_use]
    pub const fn candidate_tick(&self) -> u64 {
        self.candidate.tick_index()
    }

    /// 已应用的迁移增量记录数。
    #[must_use]
    pub const fn applied_records(&self) -> u64 {
        self.applied_records
    }

    fn ensure_live(&self) -> Result<(), CutoverError> {
        if self.settled {
            Err(CutoverError::TransactionSettled)
        } else {
            Ok(())
        }
    }

    /// 失败路径的统一收尾：解除世界日志武装、标记结算。旧世界原样继续。
    fn settle_failure(&mut self, world: &mut TrafficWorld) {
        self.settled = true;
        world.disarm_migration_journal();
    }

    /// 检查粘性溢出与追赶滞后；超限即整体放弃。
    fn check_health(&mut self, world: &mut TrafficWorld) -> Result<(), CutoverError> {
        let Some(journal) = world.migration_journal() else {
            self.settle_failure(world);
            return Err(CutoverError::JournalMissing);
        };
        if journal.overflowed() {
            self.settle_failure(world);
            return Err(CutoverError::JournalOverflow);
        }
        let lag = world
            .tick_index()
            .saturating_sub(self.candidate.tick_index());
        if lag > self.limits.max_catch_up_lag_ticks {
            self.settle_failure(world);
            return Err(CutoverError::CatchUpLagExceeded {
                lag,
                limit: self.limits.max_catch_up_lag_ticks,
            });
        }
        Ok(())
    }

    /// Delta Catch-up 泵：按规范顺序应用至多 `max_records_per_pump` 条
    /// 迁移增量。宿主在旧世界步进间隙调用；不模拟未来、不重执行输入。
    pub fn pump(&mut self, world: &mut TrafficWorld) -> Result<PumpOutcome, CutoverError> {
        self.ensure_live()?;
        self.check_health(world)?;
        let outcome = self.apply_up_to(world, self.limits.max_records_per_pump);
        match outcome {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.settle_failure(world);
                Err(error)
            }
        }
    }

    /// 从未消费处应用至多 `max_records` 条记录；`u64::MAX` 即无预算排空
    /// （静默提交的日志尾排空形态，排空量受日志字节上界约束）。
    fn apply_up_to(
        &mut self,
        world: &mut TrafficWorld,
        max_records: u64,
    ) -> Result<PumpOutcome, CutoverError> {
        let Some(journal) = world.migration_journal() else {
            return Err(CutoverError::JournalMissing);
        };
        let mut records = journal.records();
        for _ in 0..self.applied_records {
            records.next();
        }
        let mut applied: u64 = 0;
        let mut caught_up = true;
        for record in records {
            if applied >= max_records {
                caught_up = false;
                break;
            }
            apply_record(&mut self.candidate, &self.rebinding, &record)?;
            applied += 1;
        }
        self.applied_records += applied;
        Ok(PumpOutcome {
            applied_records: applied,
            caught_up,
        })
    }

    /// Quiescent Commit（切换合同 §4/§5）：在固定步进安全边界调用——
    /// 旧世界已停表、含输入。排空日志尾后依次完成占用重建、全量重验证、
    /// 确定性摘要复核与最终游标原子取样；全部通过后只剩不可失败的原地
    /// 晋升。任一失败整体放弃，旧世界从暂停点恢复步进。
    pub fn commit(&mut self, world: &mut TrafficWorld) -> Result<CutoverCommit, CutoverError> {
        self.ensure_live()?;
        let outcome = self.commit_internal(world);
        self.settled = true;
        world.disarm_migration_journal();
        outcome
    }

    fn commit_internal(&mut self, world: &mut TrafficWorld) -> Result<CutoverCommit, CutoverError> {
        self.check_health(world)?;
        // 排空日志尾（静默期不再有新提交；排空量受日志字节上界约束，
        // 不受每泵预算限制——预算只约束后台追赶）。
        self.apply_up_to(world, u64::MAX)?;
        if self.candidate.tick_index() != world.tick_index() {
            // 日志记录了全部已提交 step；tick 不一致即重放路径损坏。
            return Err(CutoverError::ReplayInconsistent);
        }
        // 最终游标在同一原子边界取样（半开覆盖区间上界；幂等重占等无记录
        // 提交的归属由取样而非重放决定），先写入候选供摘要复核与晋升共用。
        let final_command_cursor = world.command_cursor;
        self.candidate.command_cursor = final_command_cursor;
        // 可失败步骤全部前置：占用索引重建 + 终态全量重验证。
        self.candidate
            .rebuild_occupancy_index()
            .map_err(CutoverError::OccupancyRebuild)?;
        revalidate_migrated_vehicles(&self.candidate)?;
        // 确定性摘要复核：期望值 = 旧世界静默点捕获 + target origin 头部
        // 替换（记录内容按稳定引用键控，直移不改写；候选侧走自身捕获）。
        let mut expected = world.capture_snapshot();
        expected.origin = *self.target_revision.canonical_origin();
        let expected_digest = deterministic_state_digest(&expected);
        let candidate_digest = deterministic_state_digest(&self.candidate.capture_snapshot());
        if expected_digest != candidate_digest {
            return Err(CutoverError::DigestMismatch);
        }
        // 事件批次在可失败区构建（分配可失败），晋升区只做游标递增。
        let events = CutoverEventBatch::revision_cutover_committed(
            self.next_world_generation,
            self.target_revision.canonical_origin().network_revision(),
        );
        let candidate = &mut self.candidate;
        // 不可失败原地晋升：逐字段交换（零分配），世代与观测序号同界写入。
        std::mem::swap(&mut world.revision, &mut candidate.revision);
        std::mem::swap(&mut world.source, &mut candidate.source);
        std::mem::swap(&mut world.routes, &mut candidate.routes);
        std::mem::swap(&mut world.free_routes, &mut candidate.free_routes);
        std::mem::swap(&mut world.vehicles, &mut candidate.vehicles);
        std::mem::swap(&mut world.free_vehicles, &mut candidate.free_vehicles);
        std::mem::swap(&mut world.live_order, &mut candidate.live_order);
        std::mem::swap(
            &mut world.parking_occupants,
            &mut candidate.parking_occupants,
        );
        std::mem::swap(&mut world.signal_aspects, &mut candidate.signal_aspects);
        std::mem::swap(&mut world.next_states, &mut candidate.next_states);
        std::mem::swap(&mut world.occupancy, &mut candidate.occupancy);
        world.live_route_count = candidate.live_route_count;
        world.live_route_edge_occurrence_count = candidate.live_route_edge_occurrence_count;
        world.tick_index = candidate.tick_index;
        world.time_ms = candidate.time_ms;
        world.command_cursor = final_command_cursor;
        world.world_generation = self.next_world_generation;
        world.observation_state_sequence = ObservationStateSequence::INITIAL;
        world.event_cursor += events.len();
        Ok(CutoverCommit {
            world_generation: self.next_world_generation,
            final_command_cursor,
            events,
        })
    }

    /// 显式放弃：丢弃候选并解除日志武装；旧世界从暂停点恢复步进。
    pub fn abandon(self, world: &mut TrafficWorld) {
        world.disarm_migration_journal();
    }
}

fn rebind_profile(
    rebinding: &CrossRevisionRebinding,
    delta: &VehicleDelta,
) -> Result<VehicleProfileOrdinal, CutoverError> {
    rebinding
        .vehicle_profile(VehicleProfileOrdinal::from_raw(delta.profile))
        .ok_or(CutoverError::UnmappableVehicleProfile {
            base_profile: delta.profile,
        })
}

fn rebind_class(
    rebinding: &CrossRevisionRebinding,
    delta: &VehicleDelta,
) -> Result<ParticipantClassOrdinal, CutoverError> {
    rebinding
        .participant_class(ParticipantClassOrdinal::from_raw(delta.class))
        .ok_or(CutoverError::UnmappableParticipantClass {
            base_class: delta.class,
        })
}

fn rebind_parking(
    rebinding: &CrossRevisionRebinding,
    delta: &VehicleDelta,
) -> Result<Option<ParkingSpaceOrdinal>, CutoverError> {
    match delta.parking {
        None => Ok(None),
        Some(raw) => rebinding
            .parking_space(ParkingSpaceOrdinal::from_raw(raw))
            .map(Some)
            .ok_or(CutoverError::UnmappableParkingSpace { base_space: raw }),
    }
}

fn vehicle_state_from_delta(
    rebinding: &CrossRevisionRebinding,
    delta: &VehicleDelta,
) -> Result<VehicleState, CutoverError> {
    Ok(VehicleState {
        handle: VehicleHandle::new(delta.slot, delta.generation),
        profile: rebind_profile(rebinding, delta)?,
        class: rebind_class(rebinding, delta)?,
        route: RouteHandle::new(delta.route_index, delta.route_generation),
        route_edge_index: delta.route_edge_index,
        progress_mm: delta.progress_mm,
        carry_um: delta.carry_um,
        speed_mm_s: delta.speed_mm_s,
        length_mm: delta.length_mm,
        status: delta.status,
        parking: rebind_parking(rebinding, delta)?,
    })
}

/// 应用一条迁移增量到候选。生命周期类增量（生成/替换/停车）在写入后
/// 立即重验证（重绑即重验证覆盖窗口内新建绑定）；tick 增量只搬整值状态，
/// 终态由静默提交的全量重验证与摘要复核闭合。
fn apply_record(
    candidate: &mut TrafficWorld,
    rebinding: &CrossRevisionRebinding,
    record: &JournalRecord<'_>,
) -> Result<(), CutoverError> {
    match record {
        JournalRecord::Tick {
            tick_index,
            time_ms,
            entries,
        } => {
            candidate.tick_index = *tick_index;
            candidate.time_ms = *time_ms;
            for chunk in entries.chunks_exact(VEHICLE_DELTA_BYTES) {
                let delta = VehicleDelta::decode(chunk);
                let slot_index =
                    usize::try_from(delta.slot).map_err(|_| CutoverError::ReplayInconsistent)?;
                let slot = candidate
                    .vehicles
                    .get_mut(slot_index)
                    .ok_or(CutoverError::ReplayInconsistent)?;
                let state = slot
                    .state
                    .as_mut()
                    .ok_or(CutoverError::ReplayInconsistent)?;
                if state.handle.index() != delta.slot
                    || state.handle.generation() != delta.generation
                {
                    return Err(CutoverError::ReplayInconsistent);
                }
                state.profile = rebind_profile(rebinding, &delta)?;
                state.class = rebind_class(rebinding, &delta)?;
                state.route_edge_index = delta.route_edge_index;
                state.progress_mm = delta.progress_mm;
                state.carry_um = delta.carry_um;
                state.speed_mm_s = delta.speed_mm_s;
                state.status = delta.status;
                state.parking = rebind_parking(rebinding, &delta)?;
            }
            candidate.refresh_signals();
        }
        JournalRecord::RouteRegistered {
            slot,
            generation,
            edges,
            ..
        } => {
            let mut target_edges = Vec::new();
            target_edges
                .try_reserve_exact(edges.len() / 4)
                .map_err(|_| CutoverError::StagingAllocFailed)?;
            for raw in raw_u32_stream(edges) {
                let base_edge = LaneEdgeOrdinal::from_raw(raw);
                target_edges.push(
                    rebinding
                        .lane_edge(base_edge)
                        .ok_or(CutoverError::UnmappableLaneEdge { base_edge: raw })?,
                );
            }
            let compiled = compile_candidate_route(candidate, target_edges.as_slice())?;
            let slot_index =
                usize::try_from(*slot).map_err(|_| CutoverError::ReplayInconsistent)?;
            let next_count = candidate.live_route_count.checked_add(1).ok_or(
                CutoverError::EdgeOccurrenceCapacityExceeded {
                    total: u64::MAX,
                    capacity: candidate.config.route_edge_occurrence_capacity(),
                },
            )?;
            let next_occurrence = candidate
                .live_route_edge_occurrence_count
                .checked_add(u64::try_from(target_edges.len()).expect("edge count fits u64"))
                .ok_or(CutoverError::EdgeOccurrenceCapacityExceeded {
                    total: u64::MAX,
                    capacity: candidate.config.route_edge_occurrence_capacity(),
                })?;
            if next_occurrence > candidate.config.route_edge_occurrence_capacity() {
                return Err(CutoverError::EdgeOccurrenceCapacityExceeded {
                    total: next_occurrence,
                    capacity: candidate.config.route_edge_occurrence_capacity(),
                });
            }
            let staged = crate::tables::RouteSlot {
                generation: *generation,
                compiled: Some(compiled),
                live_vehicles: 0,
            };
            if slot_index == candidate.routes.len() {
                candidate
                    .routes
                    .try_reserve_exact(1)
                    .map_err(|_| CutoverError::StagingAllocFailed)?;
                candidate.routes.push(staged);
            } else if let Some(existing) = candidate.routes.get_mut(slot_index) {
                if existing.compiled.is_some() {
                    return Err(CutoverError::ReplayInconsistent);
                }
                *existing = staged;
            } else {
                return Err(CutoverError::ReplayInconsistent);
            }
            candidate.live_route_count = next_count;
            candidate.live_route_edge_occurrence_count = next_occurrence;
        }
        JournalRecord::RouteRemoved {
            slot,
            recyclable,
            generation_after,
            ..
        } => {
            let slot_index =
                usize::try_from(*slot).map_err(|_| CutoverError::ReplayInconsistent)?;
            let existing = candidate
                .routes
                .get_mut(slot_index)
                .ok_or(CutoverError::ReplayInconsistent)?;
            let Some(compiled) = existing.compiled.as_ref() else {
                return Err(CutoverError::ReplayInconsistent);
            };
            let removed = u64::try_from(compiled.edges.len()).expect("edge count fits u64");
            existing.compiled = None;
            candidate.live_route_count = candidate
                .live_route_count
                .checked_sub(1)
                .ok_or(CutoverError::ReplayInconsistent)?;
            candidate.live_route_edge_occurrence_count = candidate
                .live_route_edge_occurrence_count
                .checked_sub(removed)
                .ok_or(CutoverError::ReplayInconsistent)?;
            if *recyclable {
                existing.generation = *generation_after;
                candidate.free_routes.push(slot_index);
            }
        }
        JournalRecord::VehicleSpawned { vehicle, .. } => {
            let state = vehicle_state_from_delta(rebinding, vehicle)?;
            let handle = state.handle;
            let route = state.route;
            let slot_index =
                usize::try_from(vehicle.slot).map_err(|_| CutoverError::ReplayInconsistent)?;
            let staged = crate::tables::VehicleSlot {
                generation: vehicle.generation,
                state: Some(state),
            };
            if slot_index == candidate.vehicles.len() {
                candidate
                    .vehicles
                    .try_reserve_exact(1)
                    .map_err(|_| CutoverError::StagingAllocFailed)?;
                candidate.vehicles.push(staged);
            } else if let Some(existing) = candidate.vehicles.get_mut(slot_index) {
                if existing.state.is_some() {
                    return Err(CutoverError::ReplayInconsistent);
                }
                *existing = staged;
            } else {
                return Err(CutoverError::ReplayInconsistent);
            }
            candidate
                .live_order
                .try_reserve_exact(1)
                .map_err(|_| CutoverError::StagingAllocFailed)?;
            candidate.live_order.push(handle);
            let route_index =
                usize::try_from(route.index()).map_err(|_| CutoverError::ReplayInconsistent)?;
            if let Some(slot) = candidate.routes.get_mut(route_index) {
                slot.live_vehicles += 1;
            }
            revalidate_vehicle_on(candidate, handle)?;
        }
        JournalRecord::VehicleReplaced {
            old_slot,
            old_generation,
            order_index,
            vehicle,
            ..
        } => {
            let state = vehicle_state_from_delta(rebinding, vehicle)?;
            let old_handle = VehicleHandle::new(*old_slot, *old_generation);
            let new_handle = state.handle;
            let new_route = state.route;
            let old_index =
                usize::try_from(*old_slot).map_err(|_| CutoverError::ReplayInconsistent)?;
            let old_slot_ref = candidate
                .vehicles
                .get_mut(old_index)
                .ok_or(CutoverError::ReplayInconsistent)?;
            let Some(old_state) = old_slot_ref.state.as_ref() else {
                return Err(CutoverError::ReplayInconsistent);
            };
            if old_state.handle != old_handle {
                return Err(CutoverError::ReplayInconsistent);
            }
            let released_route = old_state.route;
            let slot_index =
                usize::try_from(vehicle.slot).map_err(|_| CutoverError::ReplayInconsistent)?;
            let staged = crate::tables::VehicleSlot {
                generation: vehicle.generation,
                state: Some(state),
            };
            if slot_index == old_index {
                *old_slot_ref = staged;
            } else {
                old_slot_ref.state = None;
                if slot_index == candidate.vehicles.len() {
                    candidate
                        .vehicles
                        .try_reserve_exact(1)
                        .map_err(|_| CutoverError::StagingAllocFailed)?;
                    candidate.vehicles.push(staged);
                } else if let Some(existing) = candidate.vehicles.get_mut(slot_index) {
                    if existing.state.is_some() {
                        return Err(CutoverError::ReplayInconsistent);
                    }
                    *existing = staged;
                } else {
                    return Err(CutoverError::ReplayInconsistent);
                }
            }
            candidate.release_route_ref(released_route);
            let order =
                usize::try_from(*order_index).map_err(|_| CutoverError::ReplayInconsistent)?;
            let order_slot = candidate
                .live_order
                .get_mut(order)
                .ok_or(CutoverError::ReplayInconsistent)?;
            if *order_slot != old_handle {
                return Err(CutoverError::ReplayInconsistent);
            }
            *order_slot = new_handle;
            let route_index =
                usize::try_from(new_route.index()).map_err(|_| CutoverError::ReplayInconsistent)?;
            if let Some(slot) = candidate.routes.get_mut(route_index) {
                slot.live_vehicles += 1;
            }
            revalidate_vehicle_on(candidate, new_handle)?;
        }
        JournalRecord::ParkingOccupied {
            slot,
            generation,
            space,
            ..
        } => {
            let handle = VehicleHandle::new(*slot, *generation);
            let target_space = rebinding
                .parking_space(ParkingSpaceOrdinal::from_raw(*space))
                .ok_or(CutoverError::UnmappableParkingSpace { base_space: *space })?;
            let slot_index =
                usize::try_from(*slot).map_err(|_| CutoverError::ReplayInconsistent)?;
            let state = candidate
                .vehicles
                .get_mut(slot_index)
                .and_then(|slot| slot.state.as_mut())
                .ok_or(CutoverError::ReplayInconsistent)?;
            if state.handle != handle {
                return Err(CutoverError::ReplayInconsistent);
            }
            state.status = crate::VehicleStatus::Parked;
            state.speed_mm_s = 0;
            state.carry_um = 0;
            state.parking = Some(target_space);
            if let Some(occupant) = candidate.parking_occupants.get_mut(target_space.index()) {
                *occupant = Some(handle);
            }
        }
    }
    Ok(())
}

fn compile_candidate_route(
    candidate: &TrafficWorld,
    edges: &[LaneEdgeOrdinal],
) -> Result<crate::tables::CompiledRoute, CutoverError> {
    crate::tables::compile_route(candidate.revision.traffic(), edges).map_err(|error| {
        if error == crate::RouteError::AllocationFailed {
            CutoverError::StagingAllocFailed
        } else {
            CutoverError::RouteRevalidationFailed
        }
    })
}

#[cfg(test)]
mod tests {
    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_static_contract::{
        EntityKind, ExactByteLength, ParkingSpaceOrdinal, SEMANTIC_DIFF_FORMAT_VERSION,
        Sha256Digest, VehicleProfileOrdinal,
    };
    use laneflow_static_network::{
        CanonicalNetworkOrigin, SharedNetworkBuildLimits, SharedNetworkBuildOptions,
        SpatialBuildOption, build_shared_network_revision,
    };
    use sha2::Digest as _;

    use super::*;
    use crate::{
        LfcaOriginBinding, RouteRegisterInput, SemanticDiffOriginBinding, TickInput,
        VehicleSpawnInput, WorldConfig,
    };

    const ORACLE_BASE: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/oracle-base.lfca"
    );
    const ORACLE_TARGET: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/oracle-target.lfca"
    );
    const ORACLE_LFSD: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/oracle-expected.lfsd"
    );
    const BASE: &[u8] =
        include_bytes!("../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/base.lfca");
    const TARGET: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/target.lfca"
    );
    const LFSD_BYTES: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/expected.lfsd"
    );

    fn revision(bytes: &[u8]) -> Arc<SharedNetworkRevision> {
        let input = check_canonical_network_input(bytes, FormatLimits::HARD)
            .expect("checked canonical network input");
        build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::Omit,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("shared network revision")
    }

    fn source_for(origin: CanonicalNetworkOrigin, key: &str) -> CommittedNetworkSource {
        CommittedNetworkSource::Published {
            reference: crate::PublishedLfcaReference::new(
                key,
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("non-empty key"),
        }
    }

    fn preflight_limits() -> CutoverPreflightLimits {
        CutoverPreflightLimits::new(1_048_576)
    }

    fn descriptor_for(
        world: &TrafficWorld,
        target_origin: CanonicalNetworkOrigin,
        lfsd: &[u8],
    ) -> NetworkRevisionCutoverDescriptor {
        let digest: [u8; 32] = sha2::Sha256::digest(lfsd).into();
        NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(*world.revision.canonical_origin()),
            LfcaOriginBinding::from_canonical_origin(target_origin),
            Some(SemanticDiffOriginBinding::new(
                SEMANTIC_DIFF_FORMAT_VERSION,
                Sha256Digest::from_bytes(digest),
                ExactByteLength::new(u64::try_from(lfsd.len()).expect("fits u64")),
            )),
            MigrationPolicyKind::CrossRevisionDirect,
            world.world_binding(),
        )
    }

    fn installed_world(bytes: &[u8], key: &str) -> TrafficWorld {
        let revision = revision(bytes);
        let origin = *revision.canonical_origin();
        TrafficWorld::install(
            revision,
            WorldConfig::new(8, 4, 1_024, 1, 100),
            source_for(origin, key),
            0,
        )
        .expect("install")
    }

    fn entry_exit(world: &TrafficWorld) -> (LaneEdgeOrdinal, LaneEdgeOrdinal) {
        let traffic = world.traffic();
        for raw in 0..traffic.lane_edge_count() {
            let edge = LaneEdgeOrdinal::from_raw(raw);
            if let Some(successor) = traffic
                .successors(edge)
                .and_then(|items| items.first().copied())
            {
                return (edge, successor);
            }
        }
        panic!("fixture exposes a connected edge pair");
    }

    fn spawn_on(
        world: &mut TrafficWorld,
        route: RouteHandle,
        progress: u32,
        speed: u32,
    ) -> VehicleHandle {
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                progress,
                speed,
            ))
            .expect("spawn")
    }

    fn prepare(
        world: &mut TrafficWorld,
        target_bytes: &[u8],
        lfsd: &[u8],
        limits: &CutoverTransactionLimits,
    ) -> CutoverTransaction {
        let target_revision = revision(target_bytes);
        let target_origin = *target_revision.canonical_origin();
        let descriptor = descriptor_for(world, target_origin, lfsd);
        world
            .prepare_cross_revision_cutover(
                target_revision,
                source_for(target_origin, "fixture://cutover-target"),
                &descriptor,
                lfsd,
                &preflight_limits(),
                limits,
            )
            .expect("prepare")
    }

    fn assert_same_committed(cut: &TrafficWorld, plain: &TrafficWorld) {
        assert_eq!(cut.tick_index(), plain.tick_index());
        assert_eq!(cut.time_ms(), plain.time_ms());
        assert_eq!(cut.live_vehicles(), plain.live_vehicles());
        for handle in plain.live_vehicles() {
            assert_eq!(
                cut.vehicle_state(*handle).expect("vehicle"),
                plain.vehicle_state(*handle).expect("vehicle"),
            );
        }
        assert_eq!(
            cut.committed_pose_sources().as_slice(),
            plain.committed_pose_sources().as_slice(),
        );
        let spaces = usize::try_from(
            plain
                .traffic()
                .entity_counts()
                .count(EntityKind::ParkingSpace),
        )
        .expect("space count");
        for raw in 0..spaces {
            let space = ParkingSpaceOrdinal::from_raw(u32::try_from(raw).expect("fits u32"));
            assert_eq!(
                cut.committed_parking_occupant(space),
                plain.committed_parking_occupant(space),
            );
        }
    }

    #[test]
    fn online_cutover_is_logically_identity_against_unswitched_world() {
        let mut cut = installed_world(ORACLE_BASE, "fixture://cut");
        let mut plain = installed_world(ORACLE_BASE, "fixture://plain");
        let (entry, exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        plain
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        let leader = spawn_on(&mut cut, route, 20_000, 5_000);
        spawn_on(&mut plain, route, 20_000, 5_000);
        let follower = spawn_on(&mut cut, route, 2_000, 5_000);
        spawn_on(&mut plain, route, 2_000, 5_000);
        for _ in 0..2 {
            cut.step(TickInput::new(100)).expect("cut step");
            plain.step(TickInput::new(100)).expect("plain step");
        }
        assert_same_committed(&cut, &plain);

        // Prepare：窗口内继续步进与生命周期命令（两世界同序列执行）。
        let mut tx = prepare(
            &mut cut,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits::default(),
        );
        let third = spawn_on(&mut cut, route, 10_000, 5_000);
        spawn_on(&mut plain, route, 10_000, 5_000);
        let space = ParkingSpaceOrdinal::from_raw(0);
        cut.occupy_parking(third, space).expect("parking");
        plain.occupy_parking(third, space).expect("parking");
        let temp = cut
            .register_route(RouteRegisterInput::new(vec![entry]))
            .expect("temp route");
        plain
            .register_route(RouteRegisterInput::new(vec![entry]))
            .expect("temp route");
        cut.remove_route(temp).expect("remove");
        plain.remove_route(temp).expect("remove");
        for _ in 0..2 {
            cut.step(TickInput::new(100)).expect("cut step");
            plain.step(TickInput::new(100)).expect("plain step");
        }
        let outcome = tx.pump(&mut cut).expect("pump drains window");
        assert!(outcome.caught_up);
        for _ in 0..2 {
            cut.step(TickInput::new(100)).expect("cut step");
            plain.step(TickInput::new(100)).expect("plain step");
        }
        tx.pump(&mut cut).expect("second pump");

        let commit = tx.commit(&mut cut).expect("commit");
        assert_eq!(commit.world_generation.get(), 1);
        assert_eq!(commit.final_command_cursor, cut.command_cursor());
        assert_eq!(
            cut.revision().canonical_origin().network_revision(),
            revision(ORACLE_TARGET)
                .canonical_origin()
                .network_revision()
        );
        assert_eq!(cut.world_generation().get(), 1);
        assert_eq!(
            cut.committed_source(),
            &source_for(
                *revision(ORACLE_TARGET).canonical_origin(),
                "fixture://cutover-target"
            )
        );

        // 提交边界与后续每一步逐点一致（验收标准第二条的端到端形态）。
        assert_same_committed(&cut, &plain);
        for _ in 0..4 {
            cut.step(TickInput::new(100)).expect("cut step");
            plain.step(TickInput::new(100)).expect("plain step");
            assert_same_committed(&cut, &plain);
        }
        let _ = (leader, follower);
    }

    #[test]
    fn journal_overflow_abandons_and_world_continues() {
        let mut cut = installed_world(ORACLE_BASE, "fixture://cut");
        let (entry, exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        let vehicle = spawn_on(&mut cut, route, 10_000, 5_000);
        let limits = CutoverTransactionLimits {
            max_journal_bytes: 64,
            ..CutoverTransactionLimits::default()
        };
        let mut tx = prepare(&mut cut, ORACLE_TARGET, ORACLE_LFSD, &limits);
        cut.step(TickInput::new(100))
            .expect("step overflows journal");
        assert_eq!(
            tx.pump(&mut cut).unwrap_err(),
            CutoverError::JournalOverflow
        );
        assert!(cut.migration_journal().is_none(), "abandon disarms journal");
        for _ in 0..3 {
            cut.step(TickInput::new(100))
                .expect("old world keeps stepping");
        }
        assert_eq!(
            tx.pump(&mut cut).unwrap_err(),
            CutoverError::TransactionSettled
        );
        assert!(cut.vehicle_state(vehicle).expect("vehicle").progress_mm() > 10_000);
        // 放弃后可再次发起（在途唯一解除）。
        let limits = CutoverTransactionLimits::default();
        let mut tx = prepare(&mut cut, ORACLE_TARGET, ORACLE_LFSD, &limits);
        tx.pump(&mut cut).expect("pump after retry prepare");
        tx.commit(&mut cut).expect("retry commit");
    }

    #[test]
    fn lag_exceeded_abandons() {
        let mut cut = installed_world(ORACLE_BASE, "fixture://cut");
        let (entry, exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        spawn_on(&mut cut, route, 10_000, 5_000);
        for _ in 0..2 {
            cut.step(TickInput::new(100)).expect("step");
        }
        let limits = CutoverTransactionLimits {
            max_catch_up_lag_ticks: 2,
            ..CutoverTransactionLimits::default()
        };
        let mut tx = prepare(&mut cut, ORACLE_TARGET, ORACLE_LFSD, &limits);
        for _ in 0..3 {
            cut.step(TickInput::new(100)).expect("host stops pumping");
        }
        assert_eq!(
            tx.pump(&mut cut).unwrap_err(),
            CutoverError::CatchUpLagExceeded { lag: 3, limit: 2 }
        );
        cut.step(TickInput::new(100))
            .expect("old world keeps stepping");
    }

    #[test]
    fn in_flight_uniqueness_blocks_second_transaction_and_sync_cutover() {
        let mut cut = installed_world(ORACLE_BASE, "fixture://cut");
        let (entry, exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        spawn_on(&mut cut, route, 10_000, 5_000);
        let target_revision = revision(ORACLE_TARGET);
        let target_origin = *target_revision.canonical_origin();
        let mut tx = prepare(
            &mut cut,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits::default(),
        );

        let again = cut.prepare_cross_revision_cutover(
            revision(ORACLE_TARGET),
            source_for(target_origin, "fixture://second"),
            &descriptor_for(&cut, target_origin, ORACLE_LFSD),
            ORACLE_LFSD,
            &preflight_limits(),
            &CutoverTransactionLimits::default(),
        );
        assert_eq!(
            match again {
                Err(error) => error,
                Ok(_) => panic!("second prepare must fail closed"),
            },
            CutoverError::InFlightTransaction
        );

        // 同步同修订入口同样被在途唯一拒绝。
        let same = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(*cut.revision.canonical_origin()),
            LfcaOriginBinding::from_canonical_origin(*cut.revision.canonical_origin()),
            None,
            crate::MigrationPolicyKind::SameRevisionRestore,
            cut.world_binding(),
        );
        assert_eq!(
            cut.cutover_same_revision(
                Arc::clone(&target_revision),
                source_for(target_origin, "fixture://sync"),
                &same,
                &preflight_limits(),
            )
            .unwrap_err(),
            CutoverError::InFlightTransaction
        );

        tx.pump(&mut cut).expect("pump");
        tx.commit(&mut cut).expect("commit");
    }

    #[test]
    fn digest_comparison_catches_candidate_side_corruption() {
        let mut cut = installed_world(ORACLE_BASE, "fixture://cut");
        let (entry, exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        let vehicle = spawn_on(&mut cut, route, 10_000, 5_000);
        let mut tx = prepare(
            &mut cut,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits::default(),
        );
        cut.step(TickInput::new(100)).expect("step");
        tx.pump(&mut cut).expect("pump");
        let before_revision = *cut.revision.canonical_origin();
        let before_generation = cut.world_generation();
        let before_state = cut.vehicle_state(vehicle).copied().expect("vehicle");

        // 注入候选侧损坏：进度偏移 1 mm，重验证通过但摘要必不相等。
        let index = usize::try_from(vehicle.index()).expect("index");
        let state = tx.candidate.vehicles[index]
            .state
            .as_mut()
            .expect("candidate vehicle");
        state.progress_mm += 1;

        assert_eq!(
            tx.commit(&mut cut).unwrap_err(),
            CutoverError::DigestMismatch
        );
        assert_eq!(*cut.revision.canonical_origin(), before_revision);
        assert_eq!(cut.world_generation(), before_generation);
        assert_eq!(cut.vehicle_state(vehicle).copied(), Some(before_state));
        cut.step(TickInput::new(100))
            .expect("old world keeps stepping");
    }

    #[test]
    fn unmappable_increment_abandons_and_clearing_retries() {
        let mut cut = installed_world(BASE, "fixture://migration-base");
        let (entry, _exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry]))
            .expect("route");
        spawn_on(&mut cut, route, 1_000, 5_000);
        let mut tx = prepare(
            &mut cut,
            TARGET,
            LFSD_BYTES,
            &CutoverTransactionLimits::default(),
        );
        // 窗口内注册引用 doomed 边的路线（base 合法、target 无对应）。
        let target_probe = revision(TARGET);
        let rebinding_probe = crate::cutover_migration::CrossRevisionRebinding::build(
            cut.revision.identity(),
            target_probe.identity(),
        );
        let mut doomed = None;
        for raw in 0..cut.traffic().lane_edge_count() {
            let edge = LaneEdgeOrdinal::from_raw(raw);
            if rebinding_probe.lane_edge(edge).is_none() {
                doomed = Some(edge);
            }
        }
        let doomed = doomed.expect("fixture has an unmappable edge");
        let doomed_route = cut
            .register_route(RouteRegisterInput::new(vec![doomed]))
            .expect("doomed route on base");
        assert_eq!(
            tx.pump(&mut cut).unwrap_err(),
            CutoverError::UnmappableLaneEdge {
                base_edge: doomed.raw()
            }
        );
        assert!(cut.migration_journal().is_none());
        // 宿主清场：移除不可映射路线后显式重试，成功直移。
        cut.remove_route(doomed_route).expect("clear doomed route");
        let mut tx = prepare(
            &mut cut,
            TARGET,
            LFSD_BYTES,
            &CutoverTransactionLimits::default(),
        );
        tx.pump(&mut cut).expect("pump");
        tx.commit(&mut cut).expect("commit after clearing");
        assert_eq!(
            cut.revision().canonical_origin().network_revision(),
            target_probe.canonical_origin().network_revision()
        );
        cut.step(TickInput::new(100)).expect("steps on target");
    }

    #[test]
    fn commit_drains_tail_and_samples_final_cursor_atomically() {
        let mut cut = installed_world(ORACLE_BASE, "fixture://cut");
        let (entry, exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        let vehicle = spawn_on(&mut cut, route, 10_000, 5_000);
        let space = ParkingSpaceOrdinal::from_raw(0);
        cut.occupy_parking(vehicle, space).expect("parking");
        let mut tx = prepare(
            &mut cut,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits::default(),
        );
        cut.step(TickInput::new(100)).expect("step");
        // 幂等重占推进命令游标但不产生日志记录：最终游标必须由静默边界
        // 原子取样，而不是重放计数。
        cut.occupy_parking(vehicle, space)
            .expect("idempotent re-occupy");
        cut.occupy_parking(vehicle, space)
            .expect("idempotent re-occupy");
        let cursor_before_commit = cut.command_cursor();
        let commit = tx.commit(&mut cut).expect("commit drains without pump");
        assert_eq!(commit.final_command_cursor, cursor_before_commit);
        assert_eq!(cut.command_cursor(), cursor_before_commit);
        assert_eq!(cut.committed_parking_occupant(space), Some(vehicle));
        cut.step(TickInput::new(100)).expect("steps on target");
    }

    #[test]
    fn snapshot_during_armed_window_captures_old_state_only() {
        let mut cut = installed_world(ORACLE_BASE, "fixture://cut");
        let mut plain = installed_world(ORACLE_BASE, "fixture://plain");
        let (entry, exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        plain
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        spawn_on(&mut cut, route, 10_000, 5_000);
        spawn_on(&mut plain, route, 10_000, 5_000);
        let mut tx = prepare(
            &mut cut,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits::default(),
        );
        cut.step(TickInput::new(100)).expect("step");
        plain.step(TickInput::new(100)).expect("plain step");
        // 准备期保存：只捕获旧修订与旧动态状态，事务不受影响（§8 行）。
        let captured = cut.capture_snapshot();
        assert_eq!(
            captured.origin.network_revision(),
            cut.revision().canonical_origin().network_revision()
        );
        assert_eq!(
            crate::deterministic_state_digest(&captured),
            crate::deterministic_state_digest(&plain.capture_snapshot())
        );
        tx.pump(&mut cut).expect("pump unaffected by capture");
        tx.commit(&mut cut).expect("commit unaffected by capture");
    }
}
