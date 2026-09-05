//! 固定步进与管理操作共享的五类私有状态所有者。
//!
//! 状态归属见 `traffic-runtime-phase-protocol.md`；这些类型不改变公开 facade。

use crate::kernel::occupancy::OccupancyIndex;
use crate::kernel::parking::ParkingRuntimeState;
use crate::kernel::tables::{RouteSlot, VehicleSlot};
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
    pub(crate) conflict_eligibility: Vec<Option<crate::ConflictEligibilityState>>,
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
    pub(crate) vehicles: Vec<VehicleSlot>,
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
    /// 车辆槽位下标对应的 intrusive queue link；长度固定为 `vehicle_capacity`。
    pub(crate) waiting_queue_ends: Box<[WaitingQueueEnds]>,
    pub(crate) waiting_links: Box<[WaitingQueueLink]>,
    /// 只读 member batch，按 `(zone, admission_sequence)` 排列。
    pub(crate) waiting_member_rows: Vec<crate::WaitingZoneMember>,
    pub(crate) occupancy: OccupancyIndex,
}

/// 本拍候选与输出暂存；失败撤销逻辑结果并复用容量。
pub(crate) struct TickWorkspace {
    pub(crate) conflict: crate::kernel::conflict::ConflictWorkspace,
    /// 固定步进 scratch；所有增长走 checked reserve，warm-up 后不再分配。
    pub(crate) conflict_candidates: Vec<crate::kernel::conflict_tick::ConflictCandidate>,
    pub(crate) conflict_schedule: crate::kernel::conflict_tick::ConflictSchedule,
    pub(crate) conflict_candidate_cells: Vec<crate::ConflictPassageAddress>,
    pub(crate) conflict_candidate_downstream: Vec<crate::DownstreamInterval>,
    pub(crate) conflict_cell_work: Vec<crate::ConflictPassageAddress>,
    pub(crate) conflict_downstream_work: Vec<crate::DownstreamInterval>,
    pub(crate) conflict_grants: Vec<crate::kernel::conflict_tick::PreparedConflictGrant>,
    pub(crate) conflict_motion_by_vehicle:
        Box<[Option<crate::kernel::conflict_tick::ConflictMotionPlan>]>,
    pub(crate) conflict_next_eligibility: Box<[Option<crate::ConflictEligibilityState>]>,
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
    pub(crate) staged_transition_events: Vec<crate::TrafficTransitionEvent>,
    pub(crate) waiting_next_counters: Box<[u64]>,
    pub(crate) waiting_staged_occupancy: Box<[u32]>,
    pub(crate) waiting_staged_storage_mm: Box<[u64]>,
    pub(crate) occupancy_scratch: crate::kernel::occupancy::OccupancyScratch,
    pub(crate) next_states: Vec<(usize, VehicleState)>,
}

#[cfg(test)]
impl WorldBindingState {
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
            + crate::kernel::state::vec_bytes(vehicles)
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
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        let Self {
            conflict,
            active_order,
            waiting_queue_ends,
            waiting_links,
            waiting_member_rows,
            occupancy,
        } = self;
        crate::kernel::state::vec_bytes(active_order)
            + crate::kernel::state::vec_bytes(waiting_member_rows)
            + crate::kernel::state::slice_bytes(waiting_queue_ends)
            + crate::kernel::state::slice_bytes(waiting_links)
            + conflict.retained_logical_bytes()
            + occupancy.retained_logical_bytes()
    }
}

#[cfg(test)]
impl TickWorkspace {
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        let Self {
            conflict,
            conflict_candidates,
            conflict_schedule,
            conflict_candidate_cells,
            conflict_candidate_downstream,
            conflict_cell_work,
            conflict_downstream_work,
            conflict_grants,
            conflict_motion_by_vehicle,
            conflict_next_eligibility,
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
            staged_transition_events,
            waiting_next_counters,
            waiting_staged_occupancy,
            waiting_staged_storage_mm,
            occupancy_scratch,
            next_states,
        } = self;
        crate::kernel::state::vec_bytes(conflict_candidates)
            + crate::kernel::state::vec_bytes(conflict_candidate_cells)
            + crate::kernel::state::vec_bytes(conflict_candidate_downstream)
            + crate::kernel::state::vec_bytes(conflict_cell_work)
            + crate::kernel::state::vec_bytes(conflict_downstream_work)
            + crate::kernel::state::vec_bytes(conflict_grants)
            + crate::kernel::state::vec_bytes(conflict_passage_transitions)
            + crate::kernel::state::vec_bytes(conflict_changed_owners)
            + crate::kernel::state::vec_bytes(conflict_staged_decisions)
            + crate::kernel::state::vec_bytes(waiting_claims)
            + crate::kernel::state::vec_bytes(waiting_plans)
            + crate::kernel::state::vec_bytes(waiting_staged_decisions)
            + crate::kernel::state::vec_bytes(staged_transition_events)
            + crate::kernel::state::vec_bytes(next_states)
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

#[cfg(test)]
pub(crate) fn vec_bytes<T>(values: &Vec<T>) -> u64 {
    (values.capacity() * core::mem::size_of::<T>()) as u64
}

#[cfg(test)]
pub(crate) fn slice_bytes<T>(values: &[T]) -> u64 {
    core::mem::size_of_val(values) as u64
}

/// 实例自有 backing 的唯一总账；共享根另列，跨世界相加时按 Arc 去重。
/// HashMap 计 payload capacity，不把 allocator 桶元数据或分配器开销冒充逻辑存储。
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct WorldMemoryLedger {
    pub(crate) shared_network: u64,
    pub(crate) partitions: [u64; 5],
}

#[cfg(test)]
impl WorldMemoryLedger {
    pub(crate) fn world_owned_bytes(&self) -> u64 {
        self.partitions.iter().sum()
    }
}

#[cfg(test)]
impl crate::TrafficWorld {
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
    fn complete_retained_memory_covers_warm_partitions_and_armed_journal() {
        let mut world = crate::kernel::waiting::tests::multi_gate_world(2);
        let initial = world.retained_memory();
        assert!(initial.shared_network > 0);
        assert!(initial.partitions[..4].iter().all(|bytes| *bytes > 0));
        assert_eq!(initial.partitions[4], 0);
        for _ in 0..3 {
            world
                .step(crate::TickInput::new(world.config().fixed_delta_time_ms()))
                .unwrap();
        }
        let warm = world.retained_memory();
        assert!(warm.world_owned_bytes() >= initial.world_owned_bytes());
        assert!(world.workspace.occupancy_scratch.retained_logical_bytes() > 0);
        world.admin.migration_journal =
            Some(MigrationDeltaJournal::arm(4_096, world.committed.command_cursor).unwrap());
        let armed = world.retained_memory();
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
