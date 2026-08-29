//! 切换事务对象（#302 切换合同 §4/§5；#513 切片 C-3）。
//!
//! Prepare →（宿主泵式 Delta Catch-up）→ Quiescent Commit / 放弃。候选由
//! Prepare 时的结构克隆直移构造（切片 C-2），此后只应用迁移增量（切片 C-1
//! 日志）：不模拟未来、不重执行输入命令。静默提交在同一原子边界排空日志
//! 尾 → 最终命令游标原子取样并写入候选 → 占用重建 + 全量重验证 →
//! 确定性摘要复核（期望值 = 旧世界静默点捕获的头部替换形式：`origin`
//! 换为 target，记录内容全部按稳定引用键控、直移不改写；取样必须先于
//! 摘要——双游标是摘要输入）→ 不可失败原地晋升（世代 checked+1、观测
//! 序号同界重置）。任一失败整体放弃：旧修订、旧动态状态、旧来源原样
//! 生效、零事件。

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
/// v1 取 600 tick：4 ms 固定步进下约 2.4 s 的可追窗口上限；实际可追
/// 窗口由日志字节上界与滞后上限共同约束——千车量级先撞字节上界
/// （8 MiB / 约 48 KB 每 tick ≈ 174 tick），更紧的宿主预算可显式配置
/// 更小值。初值随切片 C 证据登记。
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
///
/// `#[must_use]`：内含 [`CutoverEventBatch`]（恰一次交付物），语句位置丢弃
/// 属交付丢失。
#[must_use = "提交记录内含事件批次，丢弃即丢失交付"]
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
/// 静默提交；任一失败（含泵入途中）都整体放弃。事务绑定构造它的世界
/// 身份与世代：`pump`/`commit`/`abandon` 传入其它世界按
/// [`CutoverError::TransactionWorldMismatch`] 失败关闭，防止误解除他世界
/// 的在途日志。事务必须以 `commit` 或 `abandon` 结算——静默丢弃会使其
/// 来源世界保持在途锁定（`InFlightTransaction`）；不实现 `Drop` 副作用，
/// 解除日志武装始终显式发生在结算路径内。事务丢失后的世界级恢复入口为
/// [`TrafficWorld::abandon_in_flight_cutover`]。
#[must_use = "未以 commit 或 abandon 结算的切换事务会使其来源世界保持在途锁定"]
pub struct CutoverTransaction {
    candidate: TrafficWorld,
    rebinding: CrossRevisionRebinding,
    target_revision: Arc<SharedNetworkRevision>,
    limits: CutoverTransactionLimits,
    next_world_generation: WorldGeneration,
    world_id: u64,
    prepare_world_generation: WorldGeneration,
    /// prepare 时的日志武装轮次：世界级恢复后重新武装的新日志按配对
    /// 失配失败关闭，旧事务不得认领后继日志。
    armed_epoch: u64,
    applied_records: u64,
    /// 日志消费的字节偏移（记录边界）：泵送从此处续读，避免每次泵从头
    /// 重扫已消费记录（总量二次方）。
    consumed_offset: usize,
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
        // 目标信号程序按本世界步长复验（install 路径的对等校验）：候选不经
        // `TrafficWorld::install` 构造，相位与步长的合同约束必须在此显式把关。
        crate::world::validate_signal_programs(
            target_revision.as_ref(),
            self.config.fixed_delta_time_ms(),
        )
        .map_err(|_| CutoverError::TargetSignalProgramInvalid)?;
        descriptor.verify_semantic_diff(lfsd_bytes, base_origin, target_origin)?;
        // 世代耗尽必须在任何候选暂存/分配之前失败关闭。
        let next_world_generation = self
            .world_generation
            .checked_next()
            .ok_or(CutoverError::WorldGenerationExhausted)?;
        // 武装日志：以当前命令游标为半开覆盖区间下界，字节上界一次预留。
        self.arm_migration_journal(transaction_limits.max_journal_bytes)
            .map_err(|_| CutoverError::StagingAllocFailed)?;
        let armed_epoch = self.migration_epoch;
        // 基准捕获（结构克隆）+ 直移构造候选；失败即解除武装。
        let rebinding = match CrossRevisionRebinding::build(
            self.revision.identity(),
            target_revision.identity(),
        ) {
            Ok(rebinding) => rebinding,
            Err(error) => {
                self.disarm_migration_journal();
                return Err(error);
            }
        };
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
            world_id: self.world_id,
            prepare_world_generation: self.world_generation,
            armed_epoch,
            applied_records: 0,
            consumed_offset: 0,
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

    /// 事务与世界配对校验：身份、世代或日志武装轮次不符即失败关闭，
    /// 不触及任何一方（轮次比对覆盖世界级恢复后旧事务认领后继日志）。
    fn ensure_origin_world(&self, world: &TrafficWorld) -> Result<(), CutoverError> {
        if world.world_id != self.world_id
            || world.world_generation != self.prepare_world_generation
            || world.migration_epoch != self.armed_epoch
        {
            return Err(CutoverError::TransactionWorldMismatch {
                expected_world: self.world_id,
            });
        }
        Ok(())
    }

    /// 失败路径的统一收尾：解除世界日志武装、标记结算并释放候选重状态
    /// （动态表与重绑表即时归还，宿主保留事务变量不再滞留整份候选）。
    /// 旧世界原样继续。
    fn settle_failure(&mut self, world: &mut TrafficWorld) {
        self.settled = true;
        self.candidate.release_dynamic_state();
        self.rebinding.release();
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
        self.ensure_origin_world(world)?;
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
        let mut records = journal.records_from(self.consumed_offset);
        // 偏移只前进到已应用记录的边界：预算断点处预取的下一条不计入。
        let mut offset = self.consumed_offset;
        let mut applied: u64 = 0;
        let mut caught_up = true;
        while let Some(record) = records.next() {
            if applied >= max_records {
                caught_up = false;
                break;
            }
            apply_record(&mut self.candidate, &self.rebinding, &record)?;
            applied += 1;
            offset = records.offset();
        }
        self.consumed_offset = offset;
        self.applied_records += applied;
        Ok(PumpOutcome {
            applied_records: applied,
            caught_up,
        })
    }

    /// Quiescent Commit（切换合同 §4/§5）：在固定步进安全边界调用——
    /// 旧世界已停表、含输入。排空日志尾后依次完成最终游标原子取样（写入
    /// 候选）、占用重建、全量重验证与确定性摘要复核；全部通过后只剩
    /// 不可失败的原地晋升。任一失败整体放弃，旧世界从暂停点恢复步进。
    ///
    /// 消耗事务：换出的旧世界（候选槽内）随事务析构同步释放，Retire
    /// 不依赖宿主后续丢弃；结算态在类型层不可重入。
    pub fn commit(mut self, world: &mut TrafficWorld) -> Result<CutoverCommit, CutoverError> {
        self.ensure_live()?;
        self.ensure_origin_world(world)?;
        let outcome = self.commit_internal(world);
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
        // 事件批次在可失败区构建（分配可失败），晋升区只做游标递增；
        // 游标推进量在此预检，耗尽先于任何换绑失败关闭。
        let events = CutoverEventBatch::revision_cutover_committed(
            self.next_world_generation,
            self.target_revision.canonical_origin().network_revision(),
        );
        let event_advance = events.len();
        world
            .event_cursor
            .checked_add(event_advance)
            .ok_or(CutoverError::EventCursorExhausted)?;
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
        world.event_cursor += event_advance;
        Ok(CutoverCommit {
            world_generation: self.next_world_generation,
            final_command_cursor,
            events,
        })
    }

    /// 显式放弃：丢弃候选并解除日志武装；旧世界从暂停点恢复步进。
    /// 已结算事务或错误世界按失败关闭返回，不解除任何日志。
    pub fn abandon(self, world: &mut TrafficWorld) -> Result<(), CutoverError> {
        if self.settled {
            return Err(CutoverError::TransactionSettled);
        }
        self.ensure_origin_world(world)?;
        world.disarm_migration_journal();
        Ok(())
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
            // 计数加法在容量约束（u64 容量、注册路径先比容量）下结构性
            // 不可达溢出；按内部不变量处理，不用数据错误变体承载哨兵值。
            let next_count = candidate
                .live_route_count
                .checked_add(1)
                .expect("route count preflight guarantees room");
            let next_occurrence = candidate
                .live_route_edge_occurrence_count
                .checked_add(u64::try_from(target_edges.len()).expect("edge count fits u64"))
                .expect("occurrence preflight guarantees room");
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
            } else if slot_index < candidate.routes.len() {
                // 复用槽只能来自空闲表栈顶（镜像 world 注册路径的 LIFO
                // 弹出）；不消费空闲表会让晋升把含占用槽的表换回世界，
                // 后续注册复用同一槽并覆盖存活路线。
                if candidate.free_routes.last().copied() != Some(slot_index) {
                    return Err(CutoverError::ReplayInconsistent);
                }
                candidate.free_routes.pop();
                let existing = &mut candidate.routes[slot_index];
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
        ExactByteLength, ParkingSpaceOrdinal, SEMANTIC_DIFF_FORMAT_VERSION, Sha256Digest,
        VehicleProfileOrdinal,
    };
    use laneflow_static_network::{
        CanonicalNetworkOrigin, SharedNetworkBuildLimits, SharedNetworkBuildOptions,
        SpatialBuildOption, build_shared_network_revision,
    };
    use sha2::Digest as _;

    use super::*;
    use crate::{
        CutoverEvent, LfcaOriginBinding, ObservationError, ObservationExportMode,
        ObservationSelection, RouteRegisterInput, SemanticDiffOriginBinding, TickInput,
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
        crate::cutover_migration::assert_committed_logical_state_equal(cut, plain);
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
        spawn_on(&mut cut, route, 20_000, 5_000);
        spawn_on(&mut plain, route, 20_000, 5_000);
        spawn_on(&mut cut, route, 2_000, 5_000);
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
        let _ = tx.commit(&mut cut).expect("retry commit");
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
        let _ = tx.commit(&mut cut).expect("commit");
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
        )
        .unwrap();
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
        let _ = tx.commit(&mut cut).expect("commit after clearing");
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
        let tx = prepare(
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
    fn maintenance_pause_retry_succeeds_after_online_overflow() {
        // 切换合同 §4/§5：溢出放弃后宿主可显式改用维护暂停模式重试——
        // 整个准备期停表（Prepare 与静默提交之间零步进），日志零增长，
        // 同一预算下在线失败的场景成功；暂停停顿单独计量（§9）。
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
        // 在线尝试：追赶窗口内一步即溢出，整体放弃、零事件。
        let mut tx = prepare(&mut cut, ORACLE_TARGET, ORACLE_LFSD, &limits);
        cut.step(TickInput::new(100))
            .expect("online step overflows");
        assert_eq!(
            tx.pump(&mut cut).unwrap_err(),
            CutoverError::JournalOverflow
        );
        assert_eq!(cut.event_cursor(), 0);
        assert_eq!(cut.world_generation(), WorldGeneration::INITIAL);
        // 维护暂停重试：停表，零记录泵入 + 静默提交。
        let mut tx = prepare(&mut cut, ORACLE_TARGET, ORACLE_LFSD, &limits);
        let outcome = tx.pump(&mut cut).expect("paused pump drains empty journal");
        assert_eq!(outcome.applied_records, 0);
        assert!(outcome.caught_up);
        let commit = tx.commit(&mut cut).expect("paused commit");
        assert_eq!(commit.events.as_slice().len(), 1);
        assert_eq!(cut.event_cursor(), 1);
        assert_eq!(cut.world_generation(), commit.world_generation);
        assert_eq!(
            cut.revision().canonical_origin().network_revision(),
            revision(ORACLE_TARGET)
                .canonical_origin()
                .network_revision()
        );
        // 恢复运行：句柄保持、继续步进。
        assert_eq!(cut.vehicle_state(vehicle).expect("vehicle").route(), route);
        cut.step(TickInput::new(100)).expect("resumes on target");
    }

    #[test]
    fn slot_reuse_replay_consumes_free_list_and_prevents_post_commit_collision() {
        // P1 回归：切换前删除释放槽 → 窗口内注册复用该槽 → 提交。重放
        // 必须按 LIFO 消费候选空闲表，否则晋升把含占用槽的空闲表换回
        // 世界，提交后首次注册会复用同一槽并覆盖存活路线（句柄碰撞）。
        let mut cut = installed_world(ORACLE_BASE, "fixture://cut");
        let (entry, exit) = entry_exit(&cut);
        let survivor = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("survivor route");
        let doomed = cut
            .register_route(RouteRegisterInput::new(vec![entry]))
            .expect("doomed route");
        let vehicle = spawn_on(&mut cut, survivor, 10_000, 5_000);
        cut.remove_route(doomed).expect("free the doomed slot");
        let doomed_slot = doomed.index();

        let mut tx = prepare(
            &mut cut,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits::default(),
        );
        // 窗口内注册复用被释放的槽（世界侧经 free_routes.pop()）。
        let reused = cut
            .register_route(RouteRegisterInput::new(vec![exit]))
            .expect("reused route");
        assert_eq!(reused.index(), doomed_slot, "world reuses the freed slot");
        let reused_edges = cut.route_edges(reused).expect("reused route").to_vec();
        tx.pump(&mut cut).expect("pump replays slot reuse");
        let _ = tx.commit(&mut cut).expect("commit");

        // 提交后注册不得与存活路线碰撞：复用路线仍可解析、句柄互异、
        // 存活路线边序列未被覆盖。
        let after = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("post-commit route");
        assert_ne!(after.index(), reused.index());
        assert_ne!(after, reused);
        assert_eq!(cut.route_edges(reused), Some(reused_edges.as_slice()));
        let survivor_edges = cut.route_edges(survivor).expect("survivor route");
        assert_eq!(survivor_edges.len(), 2);
        assert_eq!(
            cut.vehicle_state(vehicle).expect("vehicle").route(),
            survivor
        );
        cut.step(TickInput::new(100)).expect("steps on target");
    }

    #[test]
    fn commit_resets_observation_seam_and_invalidates_old_sessions() {
        // #303 接缝（跨修订侧）：世界世代/观测 stream/状态序号与 root
        // 同界原子变化；旧 session 报 StreamBindingMismatch，新 stream
        // 从初始序号起步并绑定 target 修订。
        let mut cut = installed_world(ORACLE_BASE, "fixture://cut");
        let (entry, exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        spawn_on(&mut cut, route, 10_000, 5_000);
        cut.step(TickInput::new(100)).expect("step");
        let mut session = cut
            .open_observation_export(ObservationSelection::AllLaneEdges)
            .expect("open");
        cut.export_observation(&mut session, ObservationExportMode::Full)
            .expect("full");

        let mut tx = prepare(
            &mut cut,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits::default(),
        );
        cut.step(TickInput::new(100)).expect("window step");
        tx.pump(&mut cut).expect("pump");
        let _ = tx.commit(&mut cut).expect("commit");
        assert_eq!(
            cut.observation_state_sequence(),
            crate::ObservationStateSequence::INITIAL
        );
        assert_eq!(
            cut.export_observation(&mut session, ObservationExportMode::Delta)
                .unwrap_err(),
            ObservationError::StreamBindingMismatch
        );
        let mut replacement = cut
            .open_observation_export(ObservationSelection::AllLaneEdges)
            .expect("replacement session");
        let full = cut
            .export_observation(&mut replacement, ObservationExportMode::Full)
            .expect("replacement full");
        assert_eq!(full.sequence(), 0);
        assert_eq!(
            full.observation_state_sequence(),
            crate::ObservationStateSequence::INITIAL
        );
    }

    #[test]
    fn failures_and_abandon_keep_observation_seam_untouched() {
        // #303 接缝（abort 三者完全不变）：摘要复核失败与显式 abandon
        // 都保持世代/root/观测序号，旧 session 继续 Delta。
        let setup = |key: &str| {
            let mut world = installed_world(ORACLE_BASE, key);
            let (entry, exit) = entry_exit(&world);
            let route = world
                .register_route(RouteRegisterInput::new(vec![entry, exit]))
                .expect("route");
            let vehicle = spawn_on(&mut world, route, 10_000, 5_000);
            world.step(TickInput::new(100)).expect("step");
            let mut session = world
                .open_observation_export(ObservationSelection::AllLaneEdges)
                .expect("open");
            world
                .export_observation(&mut session, ObservationExportMode::Full)
                .expect("full");
            (world, vehicle, session)
        };

        // 摘要复核失败路径。
        let (mut world, vehicle, mut session) = setup("fixture://digest-fail");
        let mut tx = prepare(
            &mut world,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits::default(),
        );
        world.step(TickInput::new(100)).expect("step");
        tx.pump(&mut world).expect("pump");
        let index = usize::try_from(vehicle.index()).expect("index");
        tx.candidate.vehicles[index]
            .state
            .as_mut()
            .expect("candidate vehicle")
            .progress_mm += 1;
        let generation_before = world.world_generation();
        let sequence_before = world.observation_state_sequence();
        assert_eq!(
            tx.commit(&mut world).unwrap_err(),
            CutoverError::DigestMismatch
        );
        assert_eq!(world.world_generation(), generation_before);
        assert_eq!(world.observation_state_sequence(), sequence_before);
        assert_eq!(world.event_cursor(), 0);
        let delta = world
            .export_observation(&mut session, ObservationExportMode::Delta)
            .expect("old session remains live");
        assert_eq!(delta.sequence(), 1);

        // 显式 abandon 路径。
        let (mut world, _, mut session) = setup("fixture://abandon");
        let tx = prepare(
            &mut world,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits::default(),
        );
        let generation_before = world.world_generation();
        let sequence_before = world.observation_state_sequence();
        tx.abandon(&mut world).expect("abandon");
        assert_eq!(world.world_generation(), generation_before);
        assert_eq!(world.observation_state_sequence(), sequence_before);
        assert!(world.migration_journal_stats().is_none());
        let delta = world
            .export_observation(&mut session, ObservationExportMode::Delta)
            .expect("old session remains live after abandon");
        assert_eq!(delta.sequence(), 1);
    }

    #[test]
    fn transaction_world_mismatch_fails_closed() {
        let mut cut = installed_world(ORACLE_BASE, "fixture://cut");
        let (entry, exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        spawn_on(&mut cut, route, 10_000, 5_000);
        let mut tx = prepare(
            &mut cut,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits::default(),
        );
        // 另一个世界（不同 world_id）误用本事务：失败关闭且不解除任何
        // 一方的日志；原世界保持在途。
        let mut other = {
            let revision = revision(ORACLE_BASE);
            let origin = *revision.canonical_origin();
            TrafficWorld::install(
                revision,
                WorldConfig::new(8, 4, 1_024, 1, 100),
                source_for(origin, "fixture://other-world"),
                9,
            )
            .expect("install other world")
        };
        assert_eq!(
            tx.pump(&mut other).unwrap_err(),
            CutoverError::TransactionWorldMismatch { expected_world: 0 }
        );
        assert!(other.migration_journal_stats().is_none());
        assert!(cut.migration_journal_stats().is_some());
        // 正确世界仍可结算。
        let mut tx = tx;
        tx.pump(&mut cut).expect("pump on origin world");
        let _ = tx.commit(&mut cut).expect("commit");
    }

    #[test]
    fn rebinding_reservation_failure_disarms_and_world_continues() {
        let mut cut = installed_world(ORACLE_BASE, "fixture://rebind-fail");
        let (entry, exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        spawn_on(&mut cut, route, 10_000, 5_000);
        cut.step(TickInput::new(100)).expect("step");
        let target_revision = revision(ORACLE_TARGET);
        let target_origin = *target_revision.canonical_origin();
        let descriptor = descriptor_for(&cut, target_origin, ORACLE_LFSD);
        let result = crate::cutover_migration::with_staging_allocation_failure_after(0, || {
            cut.prepare_cross_revision_cutover(
                target_revision,
                source_for(target_origin, "fixture://rebind-target"),
                &descriptor,
                ORACLE_LFSD,
                &preflight_limits(),
                &CutoverTransactionLimits::default(),
            )
        });
        assert_eq!(
            match result {
                Err(error) => error,
                Ok(_) => panic!("reservation failure must fail closed"),
            },
            CutoverError::StagingAllocFailed
        );
        assert!(
            cut.migration_journal_stats().is_none(),
            "失败解除武装，不留下在途日志"
        );
        cut.step(TickInput::new(100)).expect("world unaffected");
    }

    #[test]
    fn world_mismatch_consuming_settle_recovers_via_world_entry() {
        let mut cut = installed_world(ORACLE_BASE, "fixture://cut");
        let (entry, exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        spawn_on(&mut cut, route, 10_000, 5_000);
        cut.step(TickInput::new(100)).expect("step");
        let mut other = {
            let revision = revision(ORACLE_BASE);
            let origin = *revision.canonical_origin();
            TrafficWorld::install(
                revision,
                WorldConfig::new(8, 4, 1_024, 1, 100),
                source_for(origin, "fixture://other-world"),
                9,
            )
            .expect("install other world")
        };
        let tx = prepare(
            &mut cut,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits::default(),
        );
        // 错世界结算：消耗形 API 丢弃事务对象，来源世界保持在途锁定。
        assert_eq!(
            tx.commit(&mut other).unwrap_err(),
            CutoverError::TransactionWorldMismatch { expected_world: 0 }
        );
        drop(other);
        assert!(cut.migration_journal_stats().is_some(), "来源世界仍武装");
        // 世界级恢复入口：显式放弃在途候选，旧世界从当前状态继续。
        cut.abandon_in_flight_cutover().expect("recover in-flight");
        assert!(cut.migration_journal_stats().is_none());
        cut.step(TickInput::new(100)).expect("world continues");
        // 恢复后可重新发起并正常结算。
        let retry = prepare(
            &mut cut,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits::default(),
        );
        retry
            .abandon(&mut cut)
            .expect("clean abandon after recovery");
        assert!(cut.migration_journal_stats().is_none());
    }

    #[test]
    fn stale_transaction_cannot_adopt_recovered_journal() {
        let mut cut = installed_world(ORACLE_BASE, "fixture://stale");
        let (entry, exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        spawn_on(&mut cut, route, 10_000, 5_000);
        cut.step(TickInput::new(100)).expect("step");
        let stale = prepare(
            &mut cut,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits::default(),
        );
        // 宿主误用：恢复在途锁定的同时仍持有旧事务，随后重新发起。
        cut.abandon_in_flight_cutover().expect("recover");
        let mut fresh = prepare(
            &mut cut,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits::default(),
        );
        cut.step(TickInput::new(100)).expect("silence step");
        // 旧事务对新轮次日志失败关闭：pump 与 abandon 都不触碰新日志。
        let mut stale = stale;
        assert_eq!(
            stale.pump(&mut cut).unwrap_err(),
            CutoverError::TransactionWorldMismatch { expected_world: 0 }
        );
        assert!(
            cut.migration_journal_stats().is_some(),
            "新事务日志原样武装"
        );
        assert_eq!(
            stale.abandon(&mut cut).unwrap_err(),
            CutoverError::TransactionWorldMismatch { expected_world: 0 }
        );
        assert!(cut.migration_journal_stats().is_some());
        // 新事务不受影响，正常结算。
        fresh.pump(&mut cut).expect("fresh pump");
        let _ = fresh.commit(&mut cut).expect("fresh commit");
    }

    #[test]
    fn pump_failure_settlement_releases_candidate_state() {
        let mut cut = installed_world(ORACLE_BASE, "fixture://release");
        let (entry, exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        spawn_on(&mut cut, route, 10_000, 5_000);
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
        // 宿主保留事务变量：候选动态表与重绑表已即时归还，不滞留整份候选。
        assert!(tx.candidate.routes.is_empty());
        assert!(tx.candidate.vehicles.is_empty());
        assert!(tx.candidate.live_order.is_empty());
        assert!(tx.candidate.signal_aspects.is_empty());
        assert!(cut.migration_journal().is_none());
    }

    #[test]
    fn abandon_in_flight_cutover_fails_without_in_flight() {
        let mut cut = installed_world(ORACLE_BASE, "fixture://clean");
        assert_eq!(
            cut.abandon_in_flight_cutover().unwrap_err(),
            CutoverError::NoInFlightTransaction
        );
    }

    #[test]
    fn occurrence_capacity_bounds_window_registrations() {
        // max+1 注入：世界侧 occurrence 计数被测试置零（驱动防御闭合——
        // 合法输入下世界预检与重放算术同构，本分支结构性不可达）。
        let mut cut = {
            let revision = revision(ORACLE_BASE);
            let origin = *revision.canonical_origin();
            TrafficWorld::install(
                revision,
                WorldConfig::new(8, 4, 2, 1, 100),
                source_for(origin, "fixture://capacity-cut"),
                0,
            )
            .expect("install")
        };
        let (entry, exit) = entry_exit(&cut);
        cut.register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        cut.live_route_edge_occurrence_count = 0;
        let mut tx = prepare(
            &mut cut,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits::default(),
        );
        cut.register_route(RouteRegisterInput::new(vec![entry]))
            .expect("window route passes tampered world preflight");
        assert_eq!(
            tx.pump(&mut cut).unwrap_err(),
            CutoverError::EdgeOccurrenceCapacityExceeded {
                total: 3,
                capacity: 2,
            }
        );
        assert!(cut.migration_journal_stats().is_none());
        cut.step(TickInput::new(100)).expect("old world continues");

        // max 形态：恰好等于容量时通过。
        let mut tight = {
            let revision = revision(ORACLE_BASE);
            let origin = *revision.canonical_origin();
            TrafficWorld::install(
                revision,
                WorldConfig::new(8, 4, 3, 1, 100),
                source_for(origin, "fixture://capacity-tight"),
                0,
            )
            .expect("install")
        };
        let (entry, exit) = entry_exit(&tight);
        tight
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        let mut tx = prepare(
            &mut tight,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits {
                max_journal_bytes: 64 * 1_024,
                ..CutoverTransactionLimits::default()
            },
        );
        tight
            .register_route(RouteRegisterInput::new(vec![exit]))
            .expect("third edge stays within capacity");
        tx.pump(&mut tight)
            .expect("pump applies to the capacity bound");
        let _ = tx.commit(&mut tight).expect("commit at the capacity bound");
    }

    #[test]
    fn vehicle_replaced_during_window_replays_correctly() {
        let mut cut = installed_world(ORACLE_BASE, "fixture://cut");
        let mut plain = installed_world(ORACLE_BASE, "fixture://plain");
        let (entry, exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        plain
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        let old = spawn_on(&mut cut, route, 20_000, 5_000);
        spawn_on(&mut plain, route, 20_000, 5_000);
        for _ in 0..2 {
            cut.step(TickInput::new(100)).expect("cut step");
            plain.step(TickInput::new(100)).expect("plain step");
        }
        let mut tx = prepare(
            &mut cut,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits::default(),
        );
        // 窗口内两世界同序列：强制完成 + 原子替换。
        let force_complete = |world: &mut TrafficWorld, handle: crate::VehicleHandle| {
            let index = usize::try_from(handle.index()).expect("index");
            world.vehicles[index]
                .state
                .as_mut()
                .expect("vehicle")
                .status = crate::VehicleStatus::Completed;
        };
        force_complete(&mut cut, old);
        force_complete(&mut plain, old);
        let replacement =
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 2_000, 0);
        cut.replace_completed_vehicle(old, replacement)
            .expect("cut replace");
        plain
            .replace_completed_vehicle(old, replacement)
            .expect("plain replace");
        for _ in 0..2 {
            cut.step(TickInput::new(100)).expect("cut step");
            plain.step(TickInput::new(100)).expect("plain step");
        }
        tx.pump(&mut cut).expect("pump replays replacement");
        let commit = tx.commit(&mut cut).expect("commit");
        assert_eq!(commit.events.as_slice().len(), 1);
        assert!(!commit.events.is_empty());
        assert!(matches!(
            commit.events.as_slice()[0],
            CutoverEvent::RevisionCutoverCommitted { .. }
        ));
        assert_same_committed(&cut, &plain);
        for _ in 0..3 {
            cut.step(TickInput::new(100)).expect("cut step");
            plain.step(TickInput::new(100)).expect("plain step");
            assert_same_committed(&cut, &plain);
        }
    }

    #[test]
    fn pump_budget_splits_application_across_pumps() {
        let mut cut = installed_world(ORACLE_BASE, "fixture://cut");
        let (entry, exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        spawn_on(&mut cut, route, 10_000, 5_000);
        let mut tx = prepare(
            &mut cut,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits {
                max_records_per_pump: 1,
                ..CutoverTransactionLimits::default()
            },
        );
        cut.step(TickInput::new(100)).expect("step");
        spawn_on(&mut cut, route, 2_000, 0);
        // 两条记录（TICK + SPAWNED）按预算分段应用。
        let first = tx.pump(&mut cut).expect("first pump");
        assert_eq!(first.applied_records, 1);
        assert!(!first.caught_up);
        assert_eq!(tx.applied_records(), 1);
        let second = tx.pump(&mut cut).expect("second pump");
        assert_eq!(second.applied_records, 1);
        assert!(second.caught_up);
        assert_eq!(tx.applied_records(), 2);
        let _ = tx.commit(&mut cut).expect("commit after segmented pumps");
    }

    #[test]
    fn newly_denied_suffix_increment_fails_and_clearing_retries() {
        // 窗口内「重绑后非法」：base 合法生成于 [entry, exit] 的车辆在
        // target 的 exit 拒绝下按增量重验证失败关闭；宿主清场（完成并
        // 替换到允许路线、移除受限路线）后显式重试成功。
        let mut cut = installed_world(BASE, "fixture://migration-base");
        let (entry, exit) = entry_exit(&cut);
        let allowed = cut
            .register_route(RouteRegisterInput::new(vec![entry]))
            .expect("allowed route");
        spawn_on(&mut cut, allowed, 20_000, 5_000);
        let mut tx = prepare(
            &mut cut,
            TARGET,
            LFSD_BYTES,
            &CutoverTransactionLimits::default(),
        );
        let denied_route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route legal on base");
        let vehicle = spawn_on(&mut cut, denied_route, 1_000, 5_000);
        assert_eq!(
            tx.pump(&mut cut).unwrap_err(),
            CutoverError::VehicleRevalidationFailed {
                vehicle: vehicle.index()
            }
        );
        assert_eq!(cut.event_cursor(), 0);
        // 清场：完成并替换到允许路线，移除受限路线。
        let index = usize::try_from(vehicle.index()).expect("index");
        cut.vehicles[index].state.as_mut().expect("vehicle").status =
            crate::VehicleStatus::Completed;
        cut.replace_completed_vehicle(
            vehicle,
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), allowed, 0, 1_000, 0),
        )
        .expect("clear onto allowed route");
        cut.remove_route(denied_route)
            .expect("clear restricted route");
        let mut tx = prepare(
            &mut cut,
            TARGET,
            LFSD_BYTES,
            &CutoverTransactionLimits::default(),
        );
        tx.pump(&mut cut).expect("pump after clearing");
        let _ = tx.commit(&mut cut).expect("commit after clearing");
        cut.step(TickInput::new(100)).expect("steps on target");
    }

    #[test]
    fn cross_revision_target_source_mismatch_fails_before_armed() {
        let mut cut = installed_world(ORACLE_BASE, "fixture://cut");
        let (entry, exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        spawn_on(&mut cut, route, 10_000, 5_000);
        let target_revision = revision(ORACLE_TARGET);
        let target_origin = *target_revision.canonical_origin();
        let descriptor = descriptor_for(&cut, target_origin, ORACLE_LFSD);
        let wrong_source = source_for(
            *revision(ORACLE_BASE).canonical_origin(),
            "fixture://wrong-target-source",
        );
        assert_eq!(
            match cut.prepare_cross_revision_cutover(
                target_revision,
                wrong_source,
                &descriptor,
                ORACLE_LFSD,
                &preflight_limits(),
                &CutoverTransactionLimits::default(),
            ) {
                Err(error) => error,
                Ok(_) => panic!("wrong target source must fail closed"),
            },
            CutoverError::TargetSourceRevisionMismatch
        );
        assert!(
            cut.migration_journal_stats().is_none(),
            "失败先于武装，不留下在途日志"
        );
        cut.step(TickInput::new(100)).expect("world unaffected");
    }

    const FULL_SPATIAL_LFCA: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
    );

    fn installed_world_with_dt(bytes: &[u8], key: &str, dt: u64) -> TrafficWorld {
        let revision = revision(bytes);
        let origin = *revision.canonical_origin();
        TrafficWorld::install(
            revision,
            WorldConfig::new(8, 4, 1_024, 1, dt),
            source_for(origin, key),
            0,
        )
        .expect("install")
    }

    #[test]
    fn prepare_rejects_target_signal_program_misaligned_with_tick() {
        // 源修订无信号控制器（任意步长可安装）；目标 full-spatial 相位
        // 30_000/5_000 ms，dt=600 下 5_000 非步长整数倍。
        let mut cut = installed_world_with_dt(BASE, "fixture://signal-cut", 600);
        let target_revision = revision(FULL_SPATIAL_LFCA);
        let target_origin = *target_revision.canonical_origin();
        let descriptor = descriptor_for(&cut, target_origin, &[]);
        assert_eq!(
            match cut.prepare_cross_revision_cutover(
                target_revision,
                source_for(target_origin, "fixture://signal-target"),
                &descriptor,
                &[],
                &preflight_limits(),
                &CutoverTransactionLimits::default(),
            ) {
                Err(error) => error,
                Ok(_) => panic!("misaligned signal program must fail closed"),
            },
            CutoverError::TargetSignalProgramInvalid
        );
        assert!(
            cut.migration_journal_stats().is_none(),
            "失败先于武装，不留下在途日志"
        );
        cut.step(TickInput::new(600)).expect("world unaffected");
    }

    #[test]
    fn prepare_signal_gate_passes_aligned_program_to_diff_auth() {
        // 对照：dt=500 与两相位均整除，信号关卡放行，失败移交 LFSD 认证。
        let mut cut = installed_world_with_dt(BASE, "fixture://signal-aligned", 500);
        let target_revision = revision(FULL_SPATIAL_LFCA);
        let target_origin = *target_revision.canonical_origin();
        let descriptor = descriptor_for(&cut, target_origin, &[]);
        let error = match cut.prepare_cross_revision_cutover(
            target_revision,
            source_for(target_origin, "fixture://signal-target"),
            &descriptor,
            &[],
            &preflight_limits(),
            &CutoverTransactionLimits::default(),
        ) {
            Err(error) => error,
            Ok(_) => panic!("empty diff bytes must fail LFSD auth"),
        };
        assert!(
            matches!(error, CutoverError::Descriptor(_)),
            "aligned program must pass the signal gate: {error}"
        );
        assert!(
            cut.migration_journal_stats().is_none(),
            "失败先于武装，不留下在途日志"
        );
    }

    #[test]
    fn commit_fails_closed_on_event_cursor_exhaustion() {
        let mut cut = installed_world(ORACLE_BASE, "fixture://cursor-cut");
        let (entry, exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        spawn_on(&mut cut, route, 10_000, 5_000);
        cut.step(TickInput::new(100)).expect("step");
        cut.event_cursor = u64::MAX;
        let before_generation = cut.world_generation();
        let mut tx = prepare(
            &mut cut,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits::default(),
        );
        cut.step(TickInput::new(100)).expect("silence step");
        tx.pump(&mut cut).expect("pump");
        // 耗尽在任何换绑前失败关闭：旧世界原样、武装解除、游标不动。
        assert_eq!(
            tx.commit(&mut cut).unwrap_err(),
            CutoverError::EventCursorExhausted
        );
        assert_eq!(cut.event_cursor, u64::MAX);
        assert_eq!(cut.world_generation(), before_generation);
        assert!(cut.migration_journal_stats().is_none(), "结算解除武装");
        cut.step(TickInput::new(100)).expect("world unaffected");
    }

    #[test]
    fn commit_consumes_transaction_and_releases_retired_world() {
        let mut cut = installed_world(ORACLE_BASE, "fixture://retire");
        let (entry, exit) = entry_exit(&cut);
        let route = cut
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        spawn_on(&mut cut, route, 10_000, 5_000);
        cut.step(TickInput::new(100)).expect("step");
        let retired = Arc::downgrade(&cut.revision);
        let mut tx = prepare(
            &mut cut,
            ORACLE_TARGET,
            ORACLE_LFSD,
            &CutoverTransactionLimits::default(),
        );
        cut.step(TickInput::new(100)).expect("silence step");
        tx.pump(&mut cut).expect("pump");
        let commit = tx.commit(&mut cut).expect("commit");
        assert_eq!(commit.world_generation, cut.world_generation());
        // 事务已消耗：换出的旧世界随事务析构同步释放（§4 Retire）。
        assert!(
            retired.upgrade().is_none(),
            "retired revision must drop with the consumed transaction"
        );
        cut.step(TickInput::new(100)).expect("promoted world steps");
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
        let _ = tx.commit(&mut cut).expect("commit unaffected by capture");
    }
}
