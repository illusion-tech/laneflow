//! 新鲜摆放的运动安全准入。只在 `spawn_vehicle` 与 `replace_completed_vehicle`
//! 提交前使用；快照恢复和修订切换不调用。

use std::cell::Cell;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::conflict::{ApproachEstimate, PreparedApproachEta};
use super::entry_frontier::{delay_approach_for_signal, finite_entry_distance};
use super::occupancy::LeaderQueryHorizon;
use super::state::{
    CELL_APPROACH_NONE, CELL_APPROACH_UNPROVABLE, ContenderBuilt, ContenderRank, WaitingEntrant,
    ZoneContender,
};
use super::tables::{
    distance_to_occurrence_start, for_each_admission_interval, occupancy_front_gap,
};
use super::tick::{PlacementMotion, PlacementMotionError, leader_query_horizon};
use crate::kernel::units::ceil_mm;
use crate::{
    GateCandidateKind, GatePolicyDecision, SpawnError, VehicleHandle, VehicleSpawnInput,
    VehicleState, VehicleStatus,
};
use laneflow_static_contract::{
    ManeuverGateOrdinal, ParticipantStreamOrdinal, VehicleProfileOrdinal,
};
use laneflow_static_network::BoundedDistance;

struct ReachedGate {
    hop: u32,
    gate: Option<ManeuverGateOrdinal>,
    zones: Vec<usize>,
    streams: Vec<ParticipantStreamOrdinal>,
    has_waiting: bool,
    waiting_member: bool,
    crossed_waiting: bool,
}

struct ContenderNotes {
    cells: Vec<(crate::ConflictPassageAddress, ApproachEstimate)>,
    ranks: Vec<(usize, ContenderRank, u32)>,
    waiting: Option<(usize, u32, WaitingEntrant)>,
}

#[cfg(test)]
thread_local! {
    static FOLLOWER_CANDIDATES: Cell<u64> = const { Cell::new(0) };
}

#[cfg(any(test, feature = "placement-fixtures"))]
thread_local! {
    static FAIL_CONTENDER_RESERVE: Cell<bool> = const { Cell::new(false) };
    static FAIL_NOTE_RESERVE: Cell<bool> = const { Cell::new(false) };
}

thread_local! {
    static NOTE_ALLOC_FAILED: Cell<bool> = const { Cell::new(false) };
}

/// 下一次「记下这一辆的到达和名次」时，小块预留按失败处理。
#[cfg(any(test, feature = "placement-fixtures"))]
#[doc(hidden)]
pub fn set_contender_note_reserve_failure(fail: bool) {
    FAIL_NOTE_RESERVE.with(|cell| cell.set(fail));
}

/// 下一次争用名单预留按失败处理。只给测试注入分配失败。
#[cfg(any(test, feature = "placement-fixtures"))]
#[doc(hidden)]
pub fn set_contender_reserve_failure(fail: bool) {
    FAIL_CONTENDER_RESERVE.with(|cell| cell.set(fail));
}

fn contender_reserve<T>(
    items: &mut Vec<T>,
    additional: usize,
) -> Result<(), FreshAdmissionFailure> {
    #[cfg(any(test, feature = "placement-fixtures"))]
    if FAIL_CONTENDER_RESERVE.with(Cell::get) {
        return Err(FreshAdmissionFailure::OccupancyAlloc);
    }
    items
        .try_reserve(additional)
        .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)
}

fn note_reserve<T>(items: &mut Vec<T>, additional: usize) -> Option<()> {
    #[cfg(any(test, feature = "placement-fixtures"))]
    if FAIL_NOTE_RESERVE.with(Cell::get) {
        NOTE_ALLOC_FAILED.with(|cell| cell.set(true));
        return None;
    }
    match contender_reserve(items, additional) {
        Ok(()) => Some(()),
        Err(_) => {
            NOTE_ALLOC_FAILED.with(|cell| cell.set(true));
            None
        }
    }
}

fn take_note_alloc() -> bool {
    NOTE_ALLOC_FAILED.with(|cell| cell.replace(false))
}

#[cfg(test)]
pub(crate) fn reset_follower_candidates() {
    FOLLOWER_CANDIDATES.set(0);
}

#[cfg(test)]
pub(crate) fn follower_candidates() -> u64 {
    FOLLOWER_CANDIDATES.with(Cell::get)
}

/// 新鲜摆放在重叠和权威检查之后仍可能失败的原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FreshAdmissionFailure {
    /// 当前必须停车的门按紧急制动停不住。
    StopConstraint,
    /// 前方更低限速按紧急制动降不到。
    DownstreamSpeed,
    /// 已有移动后车会把候选当成直接前车，且无法安全制动。
    UnsafeFollower(VehicleHandle),
    /// 候选相对最近前车无法安全制动。
    UnsafeLeader(VehicleHandle),
    /// 派生占用索引重建时分配失败。
    OccupancyAlloc,
}

impl FreshAdmissionFailure {
    pub(crate) fn into_spawn(self) -> SpawnError {
        match self {
            Self::StopConstraint => SpawnError::StopConstraintUnsatisfiable,
            Self::DownstreamSpeed => SpawnError::DownstreamSpeedUnsatisfiable,
            Self::UnsafeFollower(follower) => SpawnError::UnsafeFollower { follower },
            Self::UnsafeLeader(leader) => SpawnError::UnsafeLeader { leader },
            Self::OccupancyAlloc => SpawnError::OccupancyAllocFailed,
        }
    }

    pub(crate) fn into_replace(self) -> crate::ReplaceError {
        match self {
            Self::StopConstraint => crate::ReplaceError::StopConstraintUnsatisfiable,
            Self::DownstreamSpeed => crate::ReplaceError::DownstreamSpeedUnsatisfiable,
            Self::UnsafeFollower(follower) => crate::ReplaceError::UnsafeFollower { follower },
            Self::UnsafeLeader(leader) => crate::ReplaceError::UnsafeLeader { leader },
            Self::OccupancyAlloc => crate::ReplaceError::OccupancyAllocFailed,
        }
    }
}

/// 速度为 0 视为已经停住。紧急减速度不是有限正数时，无法证明能在给定距离内停住。
/// 红灯到达下界与新鲜摆放共用这个判断。
pub(crate) fn can_stop_before(
    speed_mm_s: u32,
    emergency_decel_m_s2: f32,
    distance_mm: u32,
) -> bool {
    can_slow_to_before(speed_mm_s, 0, emergency_decel_m_s2, distance_mm)
}

/// 当前速度已经不高于目标速度时通过。否则要求紧急制动距离不超过剩余房间。
pub(crate) fn can_slow_to_before(
    speed_mm_s: u32,
    target_mm_s: u32,
    emergency_decel_m_s2: f32,
    distance_mm: u32,
) -> bool {
    if speed_mm_s <= target_mm_s {
        return true;
    }
    if !emergency_decel_m_s2.is_finite() || emergency_decel_m_s2 <= 0.0 {
        return false;
    }
    let decel_mm_s2 = f64::from(emergency_decel_m_s2) * 1_000.0;
    let speed = f64::from(speed_mm_s);
    let target = f64::from(target_mm_s);
    let needed_mm = (speed * speed - target * target) / (2.0 * decel_mm_s2);
    needed_mm.is_finite() && needed_mm <= f64::from(distance_mm)
}

/// 驶离停车使用的静止前车算式。前车速度不为 0 时仍按调用方传入的速度计算，
/// 新鲜摆放不再走这里。
pub(crate) fn moving_follower_can_admit(
    follower_speed_mm_s: u32,
    follower_emergency_m_s2: f32,
    follower_min_gap_mm: u32,
    gap_mm: u32,
    leader_speed_mm_s: u32,
    leader_emergency_m_s2: f32,
    delta_s: f32,
) -> bool {
    let v = follower_speed_mm_s as f32 / 1_000.0;
    let emergency = follower_emergency_m_s2;
    let gap_m = gap_mm as f32 / 1_000.0;
    let preserved_gap_mm = gap_mm.min(follower_min_gap_mm);
    let raw_available_gap_mm = gap_mm.saturating_sub(preserved_gap_mm);
    let available_gap_mm = if raw_available_gap_mm <= 1 {
        0
    } else {
        raw_available_gap_mm
    };
    if ![v, emergency, delta_s, gap_m]
        .into_iter()
        .all(f32::is_finite)
        || emergency <= 0.0
        || delta_s <= 0.0
    {
        return false;
    }
    let u_min = (v - emergency * delta_s).max(0.0);
    let safe_envelope = 0.5 * (v + u_min) * delta_s + u_min * u_min / (2.0 * emergency);
    let emergency_min_travel = if v <= emergency * delta_s {
        v * v / (2.0 * emergency)
    } else {
        v * delta_s - 0.5 * emergency * delta_s * delta_s
    };
    let (leader_stop_m, leader_min_travel_m) = if leader_speed_mm_s == 0 {
        (0.0, 0.0)
    } else {
        let leader_v = leader_speed_mm_s as f32 / 1_000.0;
        if !leader_v.is_finite()
            || !leader_emergency_m_s2.is_finite()
            || leader_emergency_m_s2 <= 0.0
        {
            return false;
        }
        let leader_stop = leader_v * leader_v / (2.0 * leader_emergency_m_s2);
        let leader_min_travel = if leader_v <= leader_emergency_m_s2 * delta_s {
            leader_v * leader_v / (2.0 * leader_emergency_m_s2)
        } else {
            leader_v * delta_s - 0.5 * leader_emergency_m_s2 * delta_s * delta_s
        };
        if !leader_stop.is_finite() || !leader_min_travel.is_finite() {
            return false;
        }
        (leader_stop, leader_min_travel)
    };
    let available_m = available_gap_mm as f32 / 1_000.0;
    safe_envelope.is_finite()
        && emergency_min_travel.is_finite()
        && safe_envelope <= gap_m + leader_stop_m
        && emergency_min_travel <= available_m + leader_min_travel_m
}

fn room_to_edge_end(
    compiled: &super::tables::CompiledRoute,
    lengths: &[u32],
    cursor: usize,
    progress_mm: u32,
    end_hop: usize,
) -> Option<u32> {
    let current = *compiled.edges.get(cursor)?;
    let mut room = lengths.get(current.index())?.saturating_sub(progress_mm);
    for index in (cursor + 1)..=end_hop {
        let edge = *compiled.edges.get(index)?;
        room = room.saturating_add(*lengths.get(edge.index())?);
    }
    Some(room)
}

impl crate::kernel::state::WorldState {
    /// 重叠与权威已经通过之后，检查当前约束和前后车。失败不提交车辆。
    pub(crate) fn fresh_motion_admission(
        &mut self,
        input: VehicleSpawnInput,
        vehicle_length_mm: u32,
        update_sequence: u32,
    ) -> Result<(), FreshAdmissionFailure> {
        let profile = self
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(input.profile())
            .expect("未停车校验已经解析过车辆 profile");
        let emergency = profile.emergency_decel();
        let cursor = usize::try_from(input.route_edge_index()).expect("route index fits usize");
        if let Some(distance_mm) = self.restrictive_stop_mm(
            input.route(),
            input.route_edge_index(),
            input.progress_mm(),
            input.profile(),
        ) && !can_stop_before(input.initial_speed_mm_s(), emergency, distance_mm)
        {
            return Err(FreshAdmissionFailure::StopConstraint);
        }
        if self.downstream_speed_infeasible(
            input.route(),
            cursor,
            input.progress_mm(),
            input.initial_speed_mm_s(),
            emergency,
        ) {
            return Err(FreshAdmissionFailure::DownstreamSpeed);
        }
        self.ensure_current_occupancy()
            .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
        self.ensure_spawn_downstream_index()
            .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
        self.ensure_spawn_contenders()?;
        let delta_s = self.binding.config.fixed_delta_time_ms() as f32 / 1_000.0;
        self.admit_own_motion(input, profile, vehicle_length_mm, delta_s, update_sequence)?;
        self.admit_nearest_leader(input, profile, vehicle_length_mm, delta_s, update_sequence)?;
        self.admit_direct_followers(input, vehicle_length_mm, delta_s)
    }

    /// 索引脏时只在这一批的第一次生成补齐，后面的车走已经建好的树。
    fn ensure_spawn_downstream_index(
        &mut self,
    ) -> Result<(), crate::kernel::conflict::ConflictAcquireError> {
        crate::kernel::conflict::ConflictWrite::new(
            &mut self.committed.conflict,
            &mut self.derived.conflict,
            &mut self.workspace.conflict,
        )
        .ensure_downstream_index()
    }

    /// 已有车这一拍够得到的冲突区。世代或序号对不上就整份重数。
    /// 生成成功后不把新车补进旧名单再标成有效；下次生成重新数。
    fn ensure_spawn_contenders(&mut self) -> Result<(), FreshAdmissionFailure> {
        let source = ContenderBuilt {
            generation: self.binding.world_generation,
            sequence: self.committed.observation_state_sequence,
        };
        if self.derived.spawn_contenders.built_for == Some(source) {
            return Ok(());
        }
        self.rebuild_spawn_contenders()?;
        self.derived.spawn_contenders.built_for = Some(source);
        Ok(())
    }

    pub(crate) fn invalidate_spawn_contenders(&mut self) {
        self.derived.spawn_contenders.invalidate();
    }

    /// 没有冲突或排队的车不改变已有接近记录，只把名单序号推到这次提交。
    /// 有冲突，或名单里已经有申请者时，旧记录可能过期，整份作废。
    pub(crate) fn note_inserted_vehicle(
        &mut self,
        handle: VehicleHandle,
        previous: crate::ObservationStateSequence,
    ) {
        let previous_source = ContenderBuilt {
            generation: self.binding.world_generation,
            sequence: previous,
        };
        let route_needs_admission = self
            .vehicle_state(handle)
            .is_some_and(|state| self.route_needs_contender(state.route));
        if self.derived.spawn_contenders.built_for != Some(previous_source)
            || route_needs_admission
            || self.cache_has_contender()
        {
            self.invalidate_spawn_contenders();
            return;
        }
        self.derived.spawn_contenders.built_for = Some(ContenderBuilt {
            generation: self.binding.world_generation,
            sequence: self.committed.observation_state_sequence,
        });
    }

    /// 把接近名单留成空的，并标成推进世代之前的当前序号。
    ///
    /// 测试用它把「只对过观测序号」从真实切换里拆出来。名单容量和空单元格都像
    /// 刚建好的一样，所以复用时会当成这一拍没有申请者。世代耗尽时返回 `false`，
    /// 已提交世界不变。
    #[cfg(any(test, feature = "placement-fixtures"))]
    pub(crate) fn detach_contender_cache_generation_for_test(&mut self) -> bool {
        let Some(next_generation) = self.binding.world_generation.checked_next() else {
            return false;
        };
        let zone_count = usize::try_from(
            self.binding
                .revision
                .traffic()
                .entity_counts()
                .count(laneflow_static_contract::EntityKind::ConflictZone),
        )
        .unwrap_or(0);
        let cell_count = self.read_view().conflict_read().cell_len();
        let waiting_count = usize::try_from(
            self.binding
                .revision
                .traffic()
                .entity_counts()
                .count(laneflow_static_contract::EntityKind::WaitingZone),
        )
        .unwrap_or(0);
        self.derived.spawn_contenders.best.clear();
        self.derived.spawn_contenders.cell_approach_ms.clear();
        self.derived.spawn_contenders.waiting_entrants.clear();
        self.derived
            .spawn_contenders
            .best
            .resize_with(zone_count, Vec::new);
        self.derived
            .spawn_contenders
            .cell_approach_ms
            .resize(cell_count, CELL_APPROACH_NONE);
        self.derived
            .spawn_contenders
            .waiting_entrants
            .resize_with(waiting_count, Vec::new);
        self.derived.spawn_contenders.built_for = Some(ContenderBuilt {
            generation: self.binding.world_generation,
            sequence: self.committed.observation_state_sequence,
        });
        self.binding.world_generation = next_generation;
        true
    }

    fn admission_clocks(
        &self,
        state: &VehicleState,
        hop: u32,
        occurrence_index: u32,
    ) -> (u64, Option<u64>) {
        let stored = self
            .committed
            .conflict_eligibility
            .get(state.handle.index() as usize)
            .copied()
            .flatten();
        let first = stored
            .and_then(|item| item.tick_if_same_passage(state.route, hop, occurrence_index))
            .unwrap_or(self.committed.tick_index);
        let waiting = state
            .waiting_membership
            .map(|member| member.admission_sequence);
        (first, waiting)
    }

    fn route_needs_contender(&self, route: crate::RouteHandle) -> bool {
        self.compiled_route(route)
            .is_some_and(|compiled| !compiled.conflicts.is_empty() || !compiled.waiting.is_empty())
    }

    fn cache_has_contender(&self) -> bool {
        self.derived
            .spawn_contenders
            .best
            .iter()
            .any(|list| !list.is_empty())
            || self
                .derived
                .spawn_contenders
                .cell_approach_ms
                .iter()
                .any(|ms| *ms != CELL_APPROACH_NONE)
            || self
                .derived
                .spawn_contenders
                .waiting_entrants
                .iter()
                .any(|entrants| !entrants.is_empty())
    }

    fn rebuild_spawn_contenders(&mut self) -> Result<(), FreshAdmissionFailure> {
        let zone_count = usize::try_from(
            self.binding
                .revision
                .traffic()
                .entity_counts()
                .count(laneflow_static_contract::EntityKind::ConflictZone),
        )
        .unwrap_or(0);
        let cell_count = self.read_view().conflict_read().cell_len();
        let waiting_count = usize::try_from(
            self.binding
                .revision
                .traffic()
                .entity_counts()
                .count(laneflow_static_contract::EntityKind::WaitingZone),
        )
        .unwrap_or(0);
        self.derived.spawn_contenders.invalidate();
        self.derived.spawn_contenders.best.clear();
        self.derived.spawn_contenders.cell_approach_ms.clear();
        if self.derived.spawn_contenders.waiting_entrants.len() > waiting_count {
            self.derived
                .spawn_contenders
                .waiting_entrants
                .truncate(waiting_count);
        }
        contender_reserve(&mut self.derived.spawn_contenders.best, zone_count)?;
        contender_reserve(
            &mut self.derived.spawn_contenders.cell_approach_ms,
            cell_count,
        )?;
        let waiting_missing =
            waiting_count.saturating_sub(self.derived.spawn_contenders.waiting_entrants.len());
        contender_reserve(
            &mut self.derived.spawn_contenders.waiting_entrants,
            waiting_missing,
        )?;
        self.derived
            .spawn_contenders
            .best
            .resize_with(zone_count, Vec::new);
        for list in &mut self.derived.spawn_contenders.best {
            list.clear();
        }
        self.derived
            .spawn_contenders
            .cell_approach_ms
            .resize(cell_count, CELL_APPROACH_NONE);
        self.derived
            .spawn_contenders
            .waiting_entrants
            .resize(waiting_count, Vec::new());
        for entrants in &mut self.derived.spawn_contenders.waiting_entrants {
            entrants.clear();
        }
        let live_count = self.committed.live_order.len();
        for sequence in 0..live_count {
            let Ok(update_sequence) = u32::try_from(sequence) else {
                return Err(FreshAdmissionFailure::OccupancyAlloc);
            };
            let Some(handle) = self.committed.live_order.get(sequence).copied() else {
                return Err(FreshAdmissionFailure::StopConstraint);
            };
            self.add_spawn_contender(handle, update_sequence)?;
        }
        for entrants in &mut self.derived.spawn_contenders.waiting_entrants {
            entrants.sort_unstable_by_key(|entrant| (entrant.approach_mm, entrant.update_sequence));
        }
        Ok(())
    }

    /// 沿这辆车自己的路线记下证明时窗内的格点到达，以及这一拍预览会申请的第一处冲突。
    /// 分配失败返回 [`FreshAdmissionFailure::OccupancyAlloc`]。材料不齐不能当成停不住。
    fn add_spawn_contender(
        &mut self,
        handle: VehicleHandle,
        update_sequence: u32,
    ) -> Result<(), FreshAdmissionFailure> {
        let Some(state) = self.vehicle_state(handle).copied() else {
            return Ok(());
        };
        if state.status != VehicleStatus::Active || !self.route_needs_contender(state.route) {
            return Ok(());
        }
        let Some(profile) = self
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
        else {
            return Err(FreshAdmissionFailure::StopConstraint);
        };
        let delta_s = self.binding.config.fixed_delta_time_ms() as f32 / 1_000.0;
        let Some(preview) = self
            .read_view()
            .preview_active_vehicle_with_waiting_stop(state, delta_s, None, None)
        else {
            return Err(FreshAdmissionFailure::StopConstraint);
        };
        let notes = self.contender_notes(
            &state,
            update_sequence,
            preview.next,
            profile.max_accel(),
            profile.emergency_decel(),
            profile.min_gap_mm(),
        );
        if take_note_alloc() {
            return Err(FreshAdmissionFailure::OccupancyAlloc);
        }
        let Some(notes) = notes else {
            return Err(FreshAdmissionFailure::StopConstraint);
        };
        self.apply_contender_notes(handle, notes)
    }

    fn contender_notes(
        &self,
        state: &VehicleState,
        update_sequence: u32,
        preview_next: VehicleState,
        max_accel: f32,
        emergency_decel: f32,
        min_gap_mm: u32,
    ) -> Option<ContenderNotes> {
        let _ = take_note_alloc();
        let horizon = self.binding.policy_binding.horizon();
        let prepared = match horizon {
            Some(horizon_ms) => Some(PreparedApproachEta::new(
                state.carry_um,
                state.speed_mm_s,
                max_accel,
                horizon_ms,
            )?),
            None => None,
        };
        let first_possible = if state.progress_mm == 0 && state.carry_um == 0 {
            state.route_edge_index.saturating_sub(1)
        } else {
            state.route_edge_index
        };
        let (approaches, reached, waiting) = (|| {
            let read = self.read_view();
            let compiled = read.compiled_route(state.route)?;
            let lengths = read.binding.revision.traffic().lane_lengths_millimetres();
            let mut approaches = Vec::new();
            if let (Some(prepared), Some(horizon_ms)) = (prepared, horizon) {
                let first = compiled.conflicts.partition_point(|occurrence| {
                    (
                        occurrence.entry.route_edge_index,
                        occurrence.entry.progress_mm,
                    ) < (state.route_edge_index, state.progress_mm)
                });
                for occurrence in &compiled.conflicts[first..] {
                    let Some(distance_mm) = finite_entry_distance(compiled, state, occurrence)
                    else {
                        continue;
                    };
                    let kinematic = prepared.lower_bound(u64::from(distance_mm));
                    if kinematic == ApproachEstimate::OutsideHorizon {
                        break;
                    }
                    note_reserve(&mut approaches, 1)?;
                    approaches.push((occurrence.address(), kinematic, distance_mm, horizon_ms));
                }
            }
            let mut reached = Vec::new();
            let first_gate = compiled
                .gate_hops
                .partition_point(|hop| *hop < first_possible);
            for gate_index in first_gate..compiled.gate_hops.len() {
                let hop = compiled.gate_hops[gate_index];
                let hop_index = usize::try_from(hop).ok()?;
                let edge = *compiled.edges.get(hop_index)?;
                let gate_progress = *lengths.get(edge.index())?;
                let at_gate = |cursor, progress| cursor == hop && progress == gate_progress;
                let reaches_gate = preview_next.route_edge_index > hop
                    || at_gate(preview_next.route_edge_index, preview_next.progress_mm)
                    || at_gate(state.route_edge_index, state.progress_mm);
                if !reaches_gate {
                    break;
                }
                let gate = compiled.hop_gate.get(hop_index).copied().flatten();
                let mut zones = Vec::new();
                let mut streams = Vec::new();
                for entry in compiled
                    .conflicts
                    .iter()
                    .filter(|entry| entry.admission_hop == hop)
                {
                    note_reserve(&mut zones, 1)?;
                    note_reserve(&mut streams, 1)?;
                    zones.push(entry.zone.index());
                    streams.push(entry.stream);
                }
                let waiting_here = compiled.waiting.iter().find(|entry| entry.entry_hop == hop);
                let waiting_member = waiting_here.is_some_and(|entry| {
                    state.waiting_membership.is_some_and(|member| {
                        member.waiting_zone == entry.zone && member.release_hop == entry.release_hop
                    })
                });
                note_reserve(&mut reached, 1)?;
                reached.push(ReachedGate {
                    hop,
                    gate,
                    zones,
                    streams,
                    has_waiting: waiting_here.is_some(),
                    waiting_member,
                    crossed_waiting: preview_next.route_edge_index > hop,
                });
            }
            let mut waiting = None;
            let first_wait = compiled
                .waiting
                .partition_point(|entry| entry.entry_hop < first_possible);
            for occurrence in &compiled.waiting[first_wait..] {
                if state.waiting_membership.is_some_and(|member| {
                    member.waiting_zone == occurrence.zone
                        && member.release_hop == occurrence.release_hop
                }) {
                    continue;
                }
                let hop = occurrence.entry_hop;
                let hop_index = usize::try_from(hop).ok()?;
                if preview_next.route_edge_index <= hop {
                    break;
                }
                let stop_index = hop_index.checked_add(1)?;
                let distance = distance_to_occurrence_start(
                    &compiled.occurrence_segments,
                    &compiled.occurrence_offsets,
                    &compiled.segment_totals,
                    usize::try_from(state.route_edge_index).ok()?,
                    state.progress_mm,
                    stop_index,
                )?;
                let BoundedDistance::Finite(approach_mm) = distance else {
                    return None;
                };
                waiting = Some((occurrence.zone.index(), occurrence.entry_hop, approach_mm));
                break;
            }
            Some((approaches, reached, waiting))
        })()?;
        let mut cells = Vec::new();
        note_reserve(&mut cells, approaches.len())?;
        for (address, kinematic, distance_mm, horizon_ms) in approaches {
            let estimate = delay_approach_for_signal(
                self.read_view(),
                state.handle,
                state,
                kinematic,
                distance_mm,
                horizon_ms,
                emergency_decel,
            );
            if estimate == ApproachEstimate::OutsideHorizon {
                continue;
            }
            cells.push((address, estimate));
        }
        let mut ranks = Vec::new();
        {
            let read = self.read_view();
            for gate in &reached {
                let Some(ordinal) = gate.gate else {
                    continue;
                };
                if gate.waiting_member {
                    continue;
                }
                if gate.has_waiting && !gate.crossed_waiting {
                    break;
                }
                match read.gate_policy_decision(ordinal, state.profile) {
                    GatePolicyDecision::DenyAndStop => break,
                    GatePolicyDecision::Candidate(kind) => {
                        if gate.zones.is_empty() {
                            if gate.has_waiting {
                                break;
                            }
                            continue;
                        }
                        if gate.streams.is_empty() {
                            return None;
                        }
                        let protected = kind == GateCandidateKind::Protected;
                        let policy = read.policy();
                        let mut priority = None;
                        for stream in &gate.streams {
                            let rule = policy?.stream(*stream, state.class)?;
                            priority = Some(priority.map_or(rule.priority(), |current: i32| {
                                current.min(rule.priority())
                            }));
                        }
                        let compiled = self.compiled_route(state.route)?;
                        let occurrence_index = compiled
                            .conflicts
                            .partition_point(|entry| entry.admission_hop < gate.hop)
                            as u32;
                        let (first_eligible, waiting_sequence) =
                            self.admission_clocks(state, gate.hop, occurrence_index);
                        let rank = ContenderRank::new(
                            protected,
                            priority,
                            first_eligible,
                            waiting_sequence,
                            update_sequence,
                        );
                        note_reserve(&mut ranks, gate.zones.len())?;
                        for zone in &gate.zones {
                            ranks.push((*zone, rank, gate.hop));
                        }
                        break;
                    }
                }
            }
        }
        let waiting = waiting.map(|(zone, hop, approach_mm)| {
            (
                zone,
                hop,
                WaitingEntrant {
                    approach_mm,
                    update_sequence,
                    length_mm: state.length_mm,
                    min_gap_mm,
                },
            )
        });
        Some(ContenderNotes {
            cells,
            ranks,
            waiting,
        })
    }

    fn apply_contender_notes(
        &mut self,
        handle: VehicleHandle,
        notes: ContenderNotes,
    ) -> Result<(), FreshAdmissionFailure> {
        for (address, estimate) in notes.cells {
            let index = self.read_view().conflict_read().cell_index_of(address);
            let Some(index) = index else {
                return Err(FreshAdmissionFailure::StopConstraint);
            };
            let Some(slot) = self
                .derived
                .spawn_contenders
                .cell_approach_ms
                .get_mut(index)
            else {
                return Err(FreshAdmissionFailure::StopConstraint);
            };
            match estimate {
                ApproachEstimate::OutsideHorizon => {}
                ApproachEstimate::Unprovable => *slot = CELL_APPROACH_UNPROVABLE,
                ApproachEstimate::Finite(ms) => {
                    if *slot != CELL_APPROACH_UNPROVABLE {
                        *slot = (*slot).min(ms);
                    }
                }
            }
        }
        for (zone, rank, hop) in notes.ranks {
            let Some(list) = self.derived.spawn_contenders.best.get_mut(zone) else {
                return Err(FreshAdmissionFailure::StopConstraint);
            };
            contender_reserve(list, 1)?;
            let position = list.partition_point(|item| item.rank.sorts_before(rank));
            list.insert(
                position,
                ZoneContender {
                    rank,
                    vehicle: handle,
                    hop,
                },
            );
        }
        if let Some((zone, _hop, entrant)) = notes.waiting {
            let Some(list) = self.derived.spawn_contenders.waiting_entrants.get_mut(zone) else {
                return Err(FreshAdmissionFailure::StopConstraint);
            };
            contender_reserve(list, 1)?;
            let position = list.partition_point(|item| {
                (item.approach_mm, item.update_sequence)
                    < (entrant.approach_mm, entrant.update_sequence)
            });
            list.insert(position, entrant);
        }
        Ok(())
    }

    fn restrictive_stop_mm(
        &self,
        route: crate::RouteHandle,
        route_edge_index: u32,
        progress_mm: u32,
        profile: VehicleProfileOrdinal,
    ) -> Option<u32> {
        let compiled = self.compiled_route(route)?;
        let read = self.read_view();
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
        let rolled = progress_mm == 0 && route_edge_index > 0;
        let mut hop = usize::try_from(if rolled {
            route_edge_index - 1
        } else {
            route_edge_index
        })
        .ok()?;
        let progress_base = if rolled {
            let edge = *compiled.edges.get(hop)?;
            *lengths.get(edge.index())?
        } else {
            progress_mm
        };
        let edge = *compiled.edges.get(hop)?;
        let mut room = lengths.get(edge.index())?.saturating_sub(progress_base);
        loop {
            if let Some(gate) = compiled.hop_gate.get(hop).copied().flatten()
                && read.gate_is_restrictive(gate, profile)
            {
                return Some(room);
            }
            hop = hop.checked_add(1)?;
            let edge = compiled.edges.get(hop).copied()?;
            room = room.saturating_add(*lengths.get(edge.index())?);
        }
    }

    fn downstream_speed_infeasible(
        &self,
        route: crate::RouteHandle,
        cursor: usize,
        progress_mm: u32,
        speed_mm_s: u32,
        emergency_decel_m_s2: f32,
    ) -> bool {
        let Some(compiled) = self.compiled_route(route) else {
            return false;
        };
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
        let Some(current) = compiled.edges.get(cursor).copied() else {
            return false;
        };
        let Some(current_length) = lengths.get(current.index()).copied() else {
            return false;
        };
        // 降速点按路线顺序写出。从当前位置向前累加一次，不再为每个点从头重走。
        let mut room = current_length.saturating_sub(progress_mm);
        let mut walked = cursor;
        for drop in &compiled.speed_limit_drop {
            let Ok(from) = usize::try_from(drop.from_route_edge_index) else {
                continue;
            };
            if from < cursor || speed_mm_s <= drop.target_mm_s {
                continue;
            }
            let room_mm = if from < walked {
                let Some(restarted) =
                    room_to_edge_end(compiled, lengths, cursor, progress_mm, from)
                else {
                    continue;
                };
                restarted
            } else {
                while walked < from {
                    walked = walked.saturating_add(1);
                    let Some(edge) = compiled.edges.get(walked).copied() else {
                        return false;
                    };
                    let Some(length) = lengths.get(edge.index()).copied() else {
                        return false;
                    };
                    room = room.saturating_add(length);
                }
                room
            };
            if !can_slow_to_before(speed_mm_s, drop.target_mm_s, emergency_decel_m_s2, room_mm) {
                return true;
            }
        }
        false
    }

    /// 没有前车时也预览新车自己的第一拍。红灯、未放行的边末、路的尽头，
    /// 以及已经被其他车占用的冲突区，都在这次预览里。
    fn admit_own_motion(
        &self,
        input: VehicleSpawnInput,
        profile: laneflow_static_network::VehicleProfileView,
        vehicle_length_mm: u32,
        delta_s: f32,
        update_sequence: u32,
    ) -> Result<(), FreshAdmissionFailure> {
        let state = preview_vehicle(input, profile, vehicle_length_mm);
        let motion = motion_or_stop(self.read_view().placement_motion(
            state,
            None,
            false,
            update_sequence,
            None,
        ))?;
        if projection_exceeds_emergency(
            input.initial_speed_mm_s(),
            motion,
            profile.emergency_decel(),
            delta_s,
        ) {
            return Err(FreshAdmissionFailure::StopConstraint);
        }
        reject_existing_hard_stop(self, state, update_sequence, profile, delta_s)
    }

    fn admit_nearest_leader(
        &self,
        input: VehicleSpawnInput,
        profile: laneflow_static_network::VehicleProfileView,
        vehicle_length_mm: u32,
        delta_s: f32,
        update_sequence: u32,
    ) -> Result<(), FreshAdmissionFailure> {
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
        let follower_edges = self.route_edges(input.route()).expect("已校验的路线仍在");
        let follower_index =
            usize::try_from(input.route_edge_index()).expect("route index fits usize");
        let Some(horizon) = leader_query_horizon(input.initial_speed_mm_s(), profile, delta_s)
        else {
            return Err(FreshAdmissionFailure::UnsafeLeader(VehicleHandle::new(
                0, 0,
            )));
        };
        let Some(contact) = self.derived.occupancy.nearest_leader(
            VehicleHandle::new(u32::MAX, 0),
            follower_edges,
            follower_index,
            input.progress_mm(),
            lengths,
            horizon,
            true,
        ) else {
            return Ok(());
        };
        if self.vehicle_state(contact.vehicle).is_none() {
            return Err(FreshAdmissionFailure::UnsafeLeader(contact.vehicle));
        }
        let state = preview_vehicle(input, profile, vehicle_length_mm);
        let motion = match self.read_view().placement_motion(
            state,
            Some(contact.gap_mm),
            false,
            update_sequence,
            None,
        ) {
            Ok(motion) => motion,
            Err(PlacementMotionError::Alloc) => return Err(FreshAdmissionFailure::OccupancyAlloc),
            Err(PlacementMotionError::Unprovable) => {
                return Err(FreshAdmissionFailure::UnsafeLeader(contact.vehicle));
            }
        };
        if projection_exceeds_emergency(
            input.initial_speed_mm_s(),
            motion,
            profile.emergency_decel(),
            delta_s,
        ) {
            return Err(FreshAdmissionFailure::UnsafeLeader(contact.vehicle));
        }
        Ok(())
    }

    fn admit_direct_followers(
        &mut self,
        input: VehicleSpawnInput,
        vehicle_length_mm: u32,
        delta_s: f32,
    ) -> Result<(), FreshAdmissionFailure> {
        let candidates = self.upstream_follower_candidates(input, vehicle_length_mm)?;
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
        let candidate_edges = self.route_edges(input.route()).expect("已校验的路线仍在");
        let candidate_index =
            usize::try_from(input.route_edge_index()).expect("route index fits usize");
        #[cfg(test)]
        FOLLOWER_CANDIDATES.with(|count| {
            count.set(count.get().saturating_add(candidates.len() as u64));
        });
        for handle in candidates {
            let Some(follower) = self.vehicle_state(handle).copied() else {
                continue;
            };
            if follower.status != VehicleStatus::Active || follower.speed_mm_s == 0 {
                continue;
            }
            let Some(follower_edges) = self.route_edges(follower.route) else {
                return Err(FreshAdmissionFailure::UnsafeFollower(handle));
            };
            let Ok(follower_index) = usize::try_from(follower.route_edge_index) else {
                return Err(FreshAdmissionFailure::UnsafeFollower(handle));
            };
            let Some(candidate_gap) = occupancy_front_gap(
                lengths,
                follower_edges,
                follower_index,
                follower.progress_mm,
                candidate_edges,
                candidate_index,
                input.progress_mm(),
                vehicle_length_mm,
            ) else {
                continue;
            };
            if candidate_gap < 0 {
                continue;
            }
            let candidate_gap_limit = u32::try_from(candidate_gap).unwrap_or(u32::MAX);
            if self
                .derived
                .occupancy
                .leader_gap(
                    handle,
                    follower_edges,
                    follower_index,
                    follower.progress_mm,
                    lengths,
                    LeaderQueryHorizon::new(candidate_gap_limit, u32::MAX),
                )
                .is_some()
            {
                continue;
            }
            let profile = self
                .binding
                .revision
                .traffic()
                .relations()
                .vehicle_profile(follower.profile)
                .ok_or(FreshAdmissionFailure::UnsafeFollower(handle))?;
            let motion = match self.read_view().placement_motion(
                follower,
                Some(candidate_gap),
                true,
                0,
                None,
            ) {
                Ok(motion) => motion,
                Err(PlacementMotionError::Alloc) => {
                    return Err(FreshAdmissionFailure::OccupancyAlloc);
                }
                Err(PlacementMotionError::Unprovable) => {
                    return Err(FreshAdmissionFailure::UnsafeFollower(handle));
                }
            };
            if projection_exceeds_emergency(
                follower.speed_mm_s,
                motion,
                profile.emergency_decel(),
                delta_s,
            ) {
                return Err(FreshAdmissionFailure::UnsafeFollower(handle));
            }
        }
        Ok(())
    }

    /// 车身所在边，以及制动窗内能开到候选车的上游边。精确路线过滤仍在调用方。
    fn upstream_follower_candidates(
        &mut self,
        input: VehicleSpawnInput,
        vehicle_length_mm: u32,
    ) -> Result<Vec<VehicleHandle>, FreshAdmissionFailure> {
        let reach = self.max_follower_bumper_mm();
        let traffic = self.binding.revision.traffic();
        self.workspace
            .occupancy_scratch
            .ensure_maneuver_upstream(traffic)
            .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
        let lengths = traffic.lane_lengths_millimetres();
        let Some(edges) = self.route_edges(input.route()) else {
            return Ok(Vec::new());
        };
        let Ok(cursor) = usize::try_from(input.route_edge_index()) else {
            return Ok(Vec::new());
        };
        let mut body: Vec<(laneflow_static_contract::LaneEdgeOrdinal, u32)> = Vec::new();
        let mut alloc_failed = false;
        let _ = for_each_admission_interval(
            lengths,
            edges,
            cursor,
            input.progress_mm(),
            vehicle_length_mm,
            |edge, lo, _| {
                if alloc_failed {
                    return;
                }
                // 环线上同一条物理边可以有两段车身。两段后杠都要留，不能只留更小的那个。
                if body.try_reserve(1).is_err() {
                    alloc_failed = true;
                } else {
                    body.push((edge, lo));
                }
            },
        );
        if alloc_failed {
            return Err(FreshAdmissionFailure::OccupancyAlloc);
        }
        if body.is_empty()
            && let Some(edge) = edges.get(cursor).copied()
        {
            push_fallible(&mut body, (edge, input.progress_mm()))?;
        }
        let mut found = Vec::new();
        let mut search = UpstreamSearch {
            reach,
            body_edges: Vec::new(),
            best_behind: Vec::new(),
            pending: BinaryHeap::new(),
        };
        // 车身跨边时先按后杠开窗。距离为 0 的再次进入仍是这次车身。
        // 绕环走过一段正距离后再回到这条物理边，按上游窗口再查一次。
        for (edge, rear_lo) in body {
            push_fallible(&mut search.body_edges, edge.raw())?;
            collect_follower_window(
                &self.derived.occupancy,
                edge,
                rear_lo.saturating_sub(reach),
                rear_lo,
                &mut found,
            )?;
            if rear_lo <= reach {
                relax_follower_upstream(
                    traffic,
                    &self.workspace.occupancy_scratch,
                    edge,
                    rear_lo,
                    &mut search,
                )?;
            }
        }
        while let Some(Reverse((behind_end, raw))) = search.pending.pop() {
            if search
                .best_behind
                .iter()
                .find(|(edge, _)| *edge == raw)
                .is_none_or(|(_, best)| *best != behind_end)
                || behind_end > search.reach
            {
                continue;
            }
            let edge = laneflow_static_contract::LaneEdgeOrdinal::from_raw(raw);
            let length = lengths.get(edge.index()).copied().unwrap_or(0);
            let budget = reach - behind_end;
            collect_follower_window(
                &self.derived.occupancy,
                edge,
                length.saturating_sub(budget),
                length,
                &mut found,
            )?;
            let next = behind_end.saturating_add(length);
            if next <= reach {
                relax_follower_upstream(
                    traffic,
                    &self.workspace.occupancy_scratch,
                    edge,
                    next,
                    &mut search,
                )?;
            }
        }
        found.sort_unstable_by_key(|(sequence, handle)| (handle.index(), *sequence));
        found.dedup_by_key(|(_, handle)| handle.index());
        found.sort_unstable_by_key(|(sequence, handle)| (*sequence, handle.index()));
        let mut handles = Vec::new();
        handles
            .try_reserve(found.len())
            .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
        handles.extend(found.into_iter().map(|(_, handle)| handle));
        Ok(handles)
    }

    fn max_follower_bumper_mm(&mut self) -> u32 {
        if let Some(reach) = self.workspace.occupancy_scratch.follower_bumper_mm() {
            return reach;
        }
        let traffic = self.binding.revision.traffic();
        let delta_s = self.binding.config.fixed_delta_time_ms() as f32 / 1_000.0;
        let speed = traffic
            .lane_speed_limits_millimetres_per_second()
            .iter()
            .copied()
            .max()
            .unwrap_or(0);
        let profiles = traffic
            .entity_counts()
            .count(laneflow_static_contract::EntityKind::VehicleProfile);
        let mut reach = 0u32;
        for raw in 0..profiles {
            let Some(profile) = traffic
                .relations()
                .vehicle_profile(VehicleProfileOrdinal::from_raw(raw))
            else {
                continue;
            };
            if let Some(horizon) = leader_query_horizon(speed, profile, delta_s) {
                reach = reach.max(horizon.bumper_gap_mm);
            }
        }
        self.workspace
            .occupancy_scratch
            .remember_follower_bumper_mm(reach);
        reach
    }
}

fn collect_follower_window(
    occupancy: &super::occupancy::OccupancyIndex,
    edge: laneflow_static_contract::LaneEdgeOrdinal,
    min_hi: u32,
    max_hi: u32,
    found: &mut Vec<(u32, VehicleHandle)>,
) -> Result<(), FreshAdmissionFailure> {
    let mut alloc_failed = false;
    occupancy.for_each_record_in_hi_window(edge, min_hi, max_hi, |vehicle, sequence| {
        if alloc_failed {
            return;
        }
        if found.try_reserve(1).is_err() {
            alloc_failed = true;
        } else {
            found.push((sequence, vehicle));
        }
    });
    if alloc_failed {
        Err(FreshAdmissionFailure::OccupancyAlloc)
    } else {
        Ok(())
    }
}

struct UpstreamSearch {
    reach: u32,
    body_edges: Vec<u32>,
    best_behind: Vec<(u32, u32)>,
    pending: BinaryHeap<Reverse<(u32, u32)>>,
}

fn relax_follower_upstream(
    traffic: &laneflow_static_network::SharedTrafficNetwork,
    scratch: &super::occupancy::OccupancyScratch,
    edge: laneflow_static_contract::LaneEdgeOrdinal,
    behind_end: u32,
    search: &mut UpstreamSearch,
) -> Result<(), FreshAdmissionFailure> {
    if behind_end > search.reach {
        return Ok(());
    }
    if let Some(predecessors) = traffic.predecessors(edge) {
        for predecessor in predecessors {
            note_shorter_upstream(*predecessor, behind_end, search)?;
        }
    }
    for raw in scratch.maneuver_upstream(edge) {
        note_shorter_upstream(
            laneflow_static_contract::LaneEdgeOrdinal::from_raw(*raw),
            behind_end,
            search,
        )?;
    }
    Ok(())
}

fn preview_vehicle(
    input: VehicleSpawnInput,
    profile: laneflow_static_network::VehicleProfileView,
    vehicle_length_mm: u32,
) -> VehicleState {
    VehicleState {
        handle: VehicleHandle::new(u32::MAX, 0),
        profile: input.profile(),
        class: profile.class(),
        route: input.route(),
        route_edge_index: input.route_edge_index(),
        progress_mm: input.progress_mm(),
        carry_um: 0,
        speed_mm_s: input.initial_speed_mm_s(),
        length_mm: vehicle_length_mm,
        status: VehicleStatus::Active,
        maneuver_traversal: None,
        waiting_membership: None,
    }
}

fn note_shorter_upstream(
    edge: laneflow_static_contract::LaneEdgeOrdinal,
    behind_end: u32,
    search: &mut UpstreamSearch,
) -> Result<(), FreshAdmissionFailure> {
    let raw = edge.raw();
    // 后杠那一次出现已经开过窗。绕环后再遇到同一条物理边时，behind_end 是走过的正距离。
    if behind_end == 0 && search.body_edges.contains(&raw) {
        return Ok(());
    }
    if let Some((_, best)) = search.best_behind.iter_mut().find(|(item, _)| *item == raw) {
        if *best <= behind_end {
            return Ok(());
        }
        *best = behind_end;
    } else {
        push_fallible(&mut search.best_behind, (raw, behind_end))?;
    }
    search
        .pending
        .try_reserve(1)
        .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
    search.pending.push(Reverse((behind_end, raw)));
    Ok(())
}

fn push_fallible<T>(items: &mut Vec<T>, value: T) -> Result<(), FreshAdmissionFailure> {
    items
        .try_reserve(1)
        .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
    items.push(value);
    Ok(())
}

fn motion_or_stop(
    motion: Result<PlacementMotion, PlacementMotionError>,
) -> Result<PlacementMotion, FreshAdmissionFailure> {
    match motion {
        Ok(motion) => Ok(motion),
        Err(PlacementMotionError::Alloc) => Err(FreshAdmissionFailure::OccupancyAlloc),
        Err(PlacementMotionError::Unprovable) => Err(FreshAdmissionFailure::StopConstraint),
    }
}

/// 材料不齐时不算这次新造成的急停。分配失败仍往外传。
fn proved_motion(
    motion: Result<PlacementMotion, PlacementMotionError>,
) -> Result<Option<PlacementMotion>, FreshAdmissionFailure> {
    match motion {
        Ok(motion) => Ok(Some(motion)),
        Err(PlacementMotionError::Alloc) => Err(FreshAdmissionFailure::OccupancyAlloc),
        Err(PlacementMotionError::Unprovable) => Ok(None),
    }
}

/// 新车若会让已经在路上的车这一拍超出紧急制动，拒绝这次生成。
/// 旧车本来就会急停的，不算这次造成的。
fn reject_existing_hard_stop(
    world: &crate::kernel::state::WorldState,
    candidate: VehicleState,
    update_sequence: u32,
    profile: laneflow_static_network::VehicleProfileView,
    delta_s: f32,
) -> Result<(), FreshAdmissionFailure> {
    if !world.route_needs_contender(candidate.route) {
        return Ok(());
    }
    let Some(preview) = world
        .read_view()
        .preview_active_vehicle_with_waiting_stop(candidate, delta_s, None, None)
    else {
        return Err(FreshAdmissionFailure::StopConstraint);
    };
    let notes = world.contender_notes(
        &candidate,
        update_sequence,
        preview.next,
        profile.max_accel(),
        profile.emergency_decel(),
        profile.min_gap_mm(),
    );
    if take_note_alloc() {
        return Err(FreshAdmissionFailure::OccupancyAlloc);
    }
    let Some(notes) = notes else {
        return Err(FreshAdmissionFailure::StopConstraint);
    };
    let mut handles = Vec::new();
    contender_reserve(&mut handles, world.derived.active_order.len())?;
    handles.extend(world.derived.active_order.iter().copied());
    for handle in handles {
        let Some(existing) = world.vehicle_state(handle).copied() else {
            continue;
        };
        if existing.status != VehicleStatus::Active {
            continue;
        }
        let Some(existing_sequence) = world
            .committed
            .live_order
            .iter()
            .position(|item| *item == handle)
            .and_then(|index| u32::try_from(index).ok())
        else {
            continue;
        };
        let induced = match world.read_view().incoming_hard_stop(
            &existing,
            super::tick::IncomingPressure {
                approaches: &notes.cells,
                ranks: &notes.ranks,
                waiting: notes.waiting,
                candidate: &candidate,
                candidate_sequence: update_sequence,
            },
        ) {
            Ok(induced) => induced,
            Err(PlacementMotionError::Alloc) => {
                return Err(FreshAdmissionFailure::OccupancyAlloc);
            }
            Err(PlacementMotionError::Unprovable) => continue,
        };
        let Some(induced) = induced else {
            continue;
        };
        let Some(before) = proved_motion(world.read_view().placement_motion(
            existing,
            None,
            false,
            existing_sequence,
            None,
        ))?
        else {
            continue;
        };
        let Some(after) = proved_motion(world.read_view().placement_motion(
            existing,
            None,
            false,
            existing_sequence,
            Some(induced),
        ))?
        else {
            continue;
        };
        let Some(existing_profile) = world
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(existing.profile)
        else {
            return Err(FreshAdmissionFailure::StopConstraint);
        };
        let before_bad = projection_exceeds_emergency(
            existing.speed_mm_s,
            before,
            existing_profile.emergency_decel(),
            delta_s,
        );
        let after_bad = projection_exceeds_emergency(
            existing.speed_mm_s,
            after,
            existing_profile.emergency_decel(),
            delta_s,
        );
        if !before_bad && after_bad {
            return Err(FreshAdmissionFailure::StopConstraint);
        }
    }
    Ok(())
}

fn projection_exceeds_emergency(
    speed_mm_s: u32,
    motion: PlacementMotion,
    emergency_m_s2: f32,
    delta_s: f32,
) -> bool {
    if !motion.hard_clamped {
        return false;
    }
    let Some(min_travel_mm) = emergency_min_travel_mm(speed_mm_s, emergency_m_s2, delta_s) else {
        return true;
    };
    if motion.committed_travel_mm < min_travel_mm {
        return true;
    }
    let Some(floor_mm_s) = emergency_floor_mm_s(speed_mm_s, emergency_m_s2, delta_s) else {
        return true;
    };
    motion.next_speed_mm_s < floor_mm_s
}

fn emergency_min_travel_mm(speed_mm_s: u32, emergency_m_s2: f32, delta_s: f32) -> Option<u32> {
    let speed_m_s = speed_mm_s as f32 / 1_000.0;
    if ![speed_m_s, emergency_m_s2, delta_s]
        .into_iter()
        .all(f32::is_finite)
        || emergency_m_s2 <= 0.0
        || delta_s <= 0.0
    {
        return None;
    }
    let travel_m = if speed_m_s <= emergency_m_s2 * delta_s {
        speed_m_s * speed_m_s / (2.0 * emergency_m_s2)
    } else {
        speed_m_s * delta_s - 0.5 * emergency_m_s2 * delta_s * delta_s
    };
    if !travel_m.is_finite() || travel_m < 0.0 {
        return None;
    }
    ceil_mm(f64::from(travel_m))
}

fn emergency_floor_mm_s(speed_mm_s: u32, emergency_m_s2: f32, delta_s: f32) -> Option<u32> {
    let drop_mm_s = f64::from(emergency_m_s2) * 1_000.0 * f64::from(delta_s);
    if !drop_mm_s.is_finite() || drop_mm_s < 0.0 {
        return None;
    }
    let speed = f64::from(speed_mm_s);
    if drop_mm_s >= speed {
        return Some(0);
    }
    let floor = (speed - drop_mm_s).ceil();
    if floor > f64::from(u32::MAX) {
        return None;
    }
    Some(floor as u32)
}
