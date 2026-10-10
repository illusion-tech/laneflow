//! 新鲜摆放的运动安全准入。只在 `spawn_vehicle` 与 `replace_completed_vehicle`
//! 提交前使用；快照恢复和修订切换不调用。

use std::cell::Cell;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::conflict::intervals_conflict;
use super::conflict::{ApproachEstimate, ApproachFrontierCell, PreparedApproachEta};
use super::entry_frontier::{
    CachedOccurrence, FrontierMaintenance, PreparedSignalApproach, finite_entry_distance,
};
use super::occupancy::LeaderQueryHorizon;
use super::state::{
    ContenderBuilt, ContenderRank, NO_LINK, OwnerContribution, RouteContenderLink, WaitingEntrant,
    ZoneContender,
};
use super::tables::{
    body_interval_slots, distance_to_occurrence_start, for_each_admission_interval,
    for_each_occupancy_interval, occupancy_front_gap,
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
    static EDGE_ROUTE_LIMIT: Cell<u64> = const { Cell::new(u32::MAX as u64) };
}
#[cfg(test)]
crate::kernel::execution::carry_hooks!(carry_test_hooks: FOLLOWER_CANDIDATES, EDGE_ROUTE_LIMIT);

/// 「边 → 路线」压缩稀疏行能容纳的路线经过边总次数：下标是 `u32`。
#[cfg(not(test))]
fn edge_route_limit() -> u64 {
    u64::from(u32::MAX)
}

#[cfg(test)]
fn edge_route_limit() -> u64 {
    EDGE_ROUTE_LIMIT.with(Cell::get)
}

#[cfg(test)]
pub(crate) fn set_edge_route_limit(limit: u64) {
    EDGE_ROUTE_LIMIT.with(|cell| cell.set(limit));
}

#[cfg(any(test, feature = "placement-fixtures"))]
thread_local! {
    static FAIL_CONTENDER_RESERVE: Cell<u8> = const { Cell::new(0) };
    static FAIL_NOTE_RESERVE: Cell<bool> = const { Cell::new(false) };
    static INCREMENTAL_VISITS: Cell<u64> = const { Cell::new(0) };
    static REBUILD_SCANS: Cell<u64> = const { Cell::new(0) };
    static RECHECK_BODY_CHECKS: Cell<u64> = const { Cell::new(0) };
    static REBUILD_PREVIEWS: Cell<u64> = const { Cell::new(0) };
    static LAZY_SOURCES: Cell<u64> = const { Cell::new(0) };
    static FULL_CONTENDER_REBUILD: Cell<bool> = const { Cell::new(false) };
    static EDGE_ROUTE_REBUILDS: Cell<u64> = const { Cell::new(0) };
}
#[cfg(any(test, feature = "placement-fixtures"))]
crate::kernel::execution::carry_hooks!(carry_fixture_hooks: FAIL_CONTENDER_RESERVE, FAIL_NOTE_RESERVE, INCREMENTAL_VISITS, REBUILD_SCANS, RECHECK_BODY_CHECKS, REBUILD_PREVIEWS, LAZY_SOURCES, FULL_CONTENDER_REBUILD, EDGE_ROUTE_REBUILDS);

#[cfg(any(test, feature = "placement-fixtures"))]
const FAIL_BEST: u8 = 1;
#[cfg(any(test, feature = "placement-fixtures"))]
const FAIL_CELL: u8 = 2;
#[cfg(any(test, feature = "placement-fixtures"))]
const FAIL_WAITING: u8 = 4;
#[cfg(any(test, feature = "placement-fixtures"))]
const FAIL_DOWNSTREAM: u8 = 8;
#[cfg(any(test, feature = "placement-fixtures"))]
const FAIL_ORDER: u8 = 16;
#[cfg(any(test, feature = "placement-fixtures"))]
const FAIL_REFRESH: u8 = 32;
#[cfg(any(test, feature = "placement-fixtures"))]
const FAIL_RECHECK_SCRATCH: u8 = 64;
#[cfg(any(test, feature = "placement-fixtures"))]
const FAIL_REACH_MASK: u8 = 128;
#[cfg(any(test, feature = "placement-fixtures"))]
const FAIL_TABLES: u8 = FAIL_BEST | FAIL_CELL | FAIL_WAITING;

/// 预检里哪一块名单内存要按失败处理。只给测试注入。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionReserve {
    /// 每个路口的申请者名单。
    Best,
    /// 冲突格点到达。
    Cell,
    /// 排队进入者名单。
    Waiting,
    /// 下游声明的临时区间。
    Downstream,
    /// 排序前留出的工作区。
    Order,
    /// 车已经放进世界之后，刷新旧名单用的临时表。
    Refresh,
    /// 复核车身时按车辆去重的可选位图。失败时退回逐条判定，不报错。
    RecheckScratch,
    /// 重建名单时标记这一拍可能够到门的车辆的可选位图。失败时退回逐辆预览，不报错。
    ReachMask,
}

#[cfg(any(test, feature = "placement-fixtures"))]
fn reserve_bit(kind: AdmissionReserve) -> u8 {
    match kind {
        AdmissionReserve::Best => FAIL_BEST,
        AdmissionReserve::Cell => FAIL_CELL,
        AdmissionReserve::Waiting => FAIL_WAITING,
        AdmissionReserve::Downstream => FAIL_DOWNSTREAM,
        AdmissionReserve::Order => FAIL_ORDER,
        AdmissionReserve::Refresh => FAIL_REFRESH,
        AdmissionReserve::RecheckScratch => FAIL_RECHECK_SCRATCH,
        AdmissionReserve::ReachMask => FAIL_REACH_MASK,
    }
}

pub(crate) fn admission_reserve_denied(kind: AdmissionReserve) -> bool {
    #[cfg(any(test, feature = "placement-fixtures"))]
    {
        FAIL_CONTENDER_RESERVE.with(|cell| cell.get() & reserve_bit(kind) != 0)
    }
    #[cfg(not(any(test, feature = "placement-fixtures")))]
    {
        let _ = kind;
        false
    }
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

/// 下一次争用名单的外层表预留按失败处理。只给测试注入分配失败。
#[cfg(any(test, feature = "placement-fixtures"))]
#[doc(hidden)]
pub fn set_contender_reserve_failure(fail: bool) {
    FAIL_CONTENDER_RESERVE.with(|cell| cell.set(if fail { FAIL_TABLES } else { 0 }));
}

/// 只让指定的那一块预留失败。`false` 清掉全部注入。
#[cfg(any(test, feature = "placement-fixtures"))]
#[doc(hidden)]
pub fn set_admission_reserve_failure(kind: AdmissionReserve, fail: bool) {
    FAIL_CONTENDER_RESERVE.with(|cell| {
        let bit = reserve_bit(kind);
        let next = if fail {
            cell.get() | bit
        } else {
            cell.get() & !bit
        };
        cell.set(next);
    });
}

#[cfg(any(test, feature = "placement-fixtures"))]
#[doc(hidden)]
pub fn reset_contender_update_counts() {
    INCREMENTAL_VISITS.with(|cell| cell.set(0));
    REBUILD_SCANS.with(|cell| cell.set(0));
}

#[cfg(any(test, feature = "placement-fixtures"))]
#[doc(hidden)]
pub fn incremental_contender_visits() -> u64 {
    INCREMENTAL_VISITS.with(Cell::get)
}

#[cfg(any(test, feature = "placement-fixtures"))]
#[doc(hidden)]
pub fn contender_rebuild_scans() -> u64 {
    REBUILD_SCANS.with(Cell::get)
}

/// 复核目标收集中，按路线判定「是否碰到新车车身」的次数。
#[cfg(any(test, feature = "placement-fixtures"))]
#[doc(hidden)]
pub fn recheck_body_checks() -> u64 {
    RECHECK_BODY_CHECKS.with(Cell::get)
}

/// 把上面的计数清零。
#[cfg(any(test, feature = "placement-fixtures"))]
#[doc(hidden)]
pub fn reset_recheck_body_checks() {
    RECHECK_BODY_CHECKS.with(|cell| cell.set(0));
}

/// 「边 → 路线」压缩稀疏行的重建次数（含失败的尝试）。
#[cfg(any(test, feature = "placement-fixtures"))]
#[doc(hidden)]
pub fn edge_route_rebuilds() -> u64 {
    EDGE_ROUTE_REBUILDS.with(Cell::get)
}

/// 把上面的计数清零。
#[cfg(any(test, feature = "placement-fixtures"))]
#[doc(hidden)]
pub fn reset_edge_route_rebuilds() {
    EDGE_ROUTE_REBUILDS.with(|cell| cell.set(0));
}

/// 最近一次名单重建里做过运动预览的车辆数。
#[cfg(any(test, feature = "placement-fixtures"))]
#[doc(hidden)]
pub fn contender_rebuild_previews() -> u64 {
    REBUILD_PREVIEWS.with(Cell::get)
}

/// 本线程累计在读取格点时从 frontier 并入的来源车次数。
#[cfg(any(test, feature = "placement-fixtures"))]
#[doc(hidden)]
pub fn contender_lazy_sources() -> u64 {
    LAZY_SOURCES.with(Cell::get)
}

/// 打开后名单重建不读 frontier，逐辆预览，作对拍参考。
#[cfg(any(test, feature = "placement-fixtures"))]
#[doc(hidden)]
pub fn set_full_contender_rebuild(enabled: bool) {
    FULL_CONTENDER_REBUILD.with(|cell| cell.set(enabled));
}

#[cfg(any(test, feature = "placement-fixtures"))]
fn full_contender_rebuild() -> bool {
    FULL_CONTENDER_REBUILD.with(Cell::get)
}

#[cfg(not(any(test, feature = "placement-fixtures")))]
const fn full_contender_rebuild() -> bool {
    false
}

fn contender_reserve<T>(
    items: &mut Vec<T>,
    additional: usize,
    kind: AdmissionReserve,
) -> Result<(), FreshAdmissionFailure> {
    if admission_reserve_denied(kind) {
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
    match items.try_reserve(additional) {
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

fn sort_waiting_entrants(entrants: &mut [WaitingEntrant]) -> Result<(), FreshAdmissionFailure> {
    let mut workspace = Vec::new();
    if admission_reserve_denied(AdmissionReserve::Order)
        || workspace.try_reserve(entrants.len()).is_err()
    {
        return Err(FreshAdmissionFailure::OccupancyAlloc);
    }
    workspace.extend_from_slice(entrants);
    mergesort_copy(entrants, &mut workspace, |left, right| {
        (left.approach_mm, left.update_sequence) < (right.approach_mm, right.update_sequence)
    });
    Ok(())
}

/// 用已经留好的两块等长缓冲区归并。比较次数随人数按对数增长，不再申请内存。
pub(super) fn mergesort_copy<T: Copy>(
    items: &mut [T],
    scratch: &mut [T],
    mut less: impl FnMut(&T, &T) -> bool,
) {
    fn pass<T: Copy>(
        source: &[T],
        dest: &mut [T],
        width: usize,
        less: &mut impl FnMut(&T, &T) -> bool,
    ) {
        let count = source.len();
        let mut start = 0usize;
        while start < count {
            let mid = start.saturating_add(width).min(count);
            let end = mid.saturating_add(width).min(count);
            let mut left = start;
            let mut right = mid;
            let mut slot = start;
            while slot < end {
                let take_left =
                    right >= end || (left < mid && !less(&source[right], &source[left]));
                dest[slot] = if take_left {
                    source[left]
                } else {
                    source[right]
                };
                if take_left {
                    left = left.saturating_add(1);
                } else {
                    right = right.saturating_add(1);
                }
                slot = slot.saturating_add(1);
            }
            start = end;
        }
    }

    let count = items.len();
    debug_assert_eq!(scratch.len(), count);
    let mut width = 1usize;
    let mut in_scratch = false;
    while width < count {
        if in_scratch {
            pass(scratch, items, width, &mut less);
        } else {
            pass(items, scratch, width, &mut less);
        }
        in_scratch = !in_scratch;
        width = width.saturating_mul(2);
    }
    if in_scratch {
        items.copy_from_slice(scratch);
    }
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
        #[cfg(any(test, feature = "placement-fixtures"))]
        REBUILD_SCANS.with(|cell| cell.set(0));
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

    /// 没有冲突或排队、名单里也没有申请者时，只把序号推到这次提交。
    /// 否则只撤掉并重算受影响的旧车。预留失败就整份作废，不把半份标成当前。
    pub(crate) fn note_inserted_vehicle(
        &mut self,
        handle: VehicleHandle,
        previous: crate::ObservationStateSequence,
        update_sequence: u32,
    ) {
        let previous_source = ContenderBuilt {
            generation: self.binding.world_generation,
            sequence: previous,
        };
        if self.derived.spawn_contenders.built_for != Some(previous_source) {
            self.invalidate_spawn_contenders();
            return;
        }
        // 按需模式读格点时按 live 序号归约；生成追加到 live 序尾，这里把序号表补齐。
        if self.derived.spawn_contenders.lazy_cells
            && !self
                .derived
                .live_order_index
                .ensure(&self.committed.live_order, self.committed.vehicles.len())
        {
            self.invalidate_spawn_contenders();
            return;
        }
        let route_needs_admission = self
            .vehicle_state(handle)
            .is_some_and(|state| self.route_needs_contender(state.route));
        if (route_needs_admission || self.cache_has_contender())
            && self
                .refresh_contender_cache(handle, update_sequence)
                .is_err()
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
    #[cfg(feature = "placement-fixtures")]
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
        self.derived.spawn_contenders.cell_approach.clear();
        self.derived.spawn_contenders.waiting_entrants.clear();
        self.derived.spawn_contenders.owners.clear();
        self.derived.spawn_contenders.lazy_cells = false;
        if let Some(index) = self.derived.spawn_contenders.recheck_index_mut() {
            index.linked = false;
        }
        self.derived
            .spawn_contenders
            .best
            .resize_with(zone_count, Vec::new);
        self.derived
            .spawn_contenders
            .cell_approach
            .resize(cell_count, ApproachFrontierCell::default());
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

    #[cfg(feature = "placement-fixtures")]
    pub(crate) fn force_rebuild_contenders_for_test(&mut self) {
        self.invalidate_spawn_contenders();
        let _ = self.ensure_spawn_contenders();
    }

    #[cfg(feature = "placement-fixtures")]
    pub(crate) fn contender_fingerprint_for_test(&self) -> Vec<(u32, u32, u32, u32)> {
        let mut rows = Vec::new();
        for (zone, list) in self.derived.spawn_contenders.best.iter().enumerate() {
            for contender in list {
                rows.push((
                    u32::try_from(zone).unwrap_or(u32::MAX),
                    contender.vehicle.index(),
                    contender.rank.update_sequence(),
                    0,
                ));
            }
        }
        for (zone, list) in self
            .derived
            .spawn_contenders
            .waiting_entrants
            .iter()
            .enumerate()
        {
            for entrant in list {
                rows.push((
                    u32::try_from(zone).unwrap_or(u32::MAX),
                    entrant.vehicle.index(),
                    entrant.update_sequence,
                    entrant.approach_mm.saturating_add(1),
                ));
            }
        }
        rows
    }

    #[cfg(feature = "placement-fixtures")]
    pub(crate) fn contender_cells_for_test(&self) -> Vec<String> {
        let read = self.read_view();
        (0..self.derived.spawn_contenders.cell_approach.len())
            .filter_map(|index| {
                let address = read.conflict_read().cell_address(index)?;
                match read.contender_cell(index, address) {
                    Ok(Some(cell)) if cell.occupied() => Some(format!("{index}:{cell:?}")),
                    Ok(_) => None,
                    Err(()) => Some(format!("{index}:unproven")),
                }
            })
            .collect()
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
        // 正式步进把新资格记在下一拍。名单预览漏记时也用下一拍，不能和刚记下的车挤在同一拍。
        let first = stored
            .and_then(|item| item.tick_if_same_passage(state.route, hop, occurrence_index))
            .unwrap_or_else(|| self.committed.tick_index.saturating_add(1));
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
        // 按需模式下未求值的格点可能有 owner，按有处理：刷新总是正确的，只是多做一次。
        self.derived.spawn_contenders.lazy_cells
            || self
                .derived
                .spawn_contenders
                .best
                .iter()
                .any(|list| !list.is_empty())
            || self
                .derived
                .spawn_contenders
                .cell_approach
                .iter()
                .any(|cell| cell.occupied())
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
        self.derived.spawn_contenders.lazy_cells = false;
        self.derived.spawn_contenders.best.clear();
        self.derived.spawn_contenders.cell_approach.clear();
        self.derived.spawn_contenders.owners.clear();
        self.reset_route_contenders();
        if self.derived.spawn_contenders.waiting_entrants.len() > waiting_count {
            self.derived
                .spawn_contenders
                .waiting_entrants
                .truncate(waiting_count);
        }
        contender_reserve(
            &mut self.derived.spawn_contenders.best,
            zone_count,
            AdmissionReserve::Best,
        )?;
        contender_reserve(
            &mut self.derived.spawn_contenders.cell_approach,
            cell_count,
            AdmissionReserve::Cell,
        )?;
        let waiting_missing =
            waiting_count.saturating_sub(self.derived.spawn_contenders.waiting_entrants.len());
        contender_reserve(
            &mut self.derived.spawn_contenders.waiting_entrants,
            waiting_missing,
            AdmissionReserve::Waiting,
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
            .cell_approach
            .resize(cell_count, ApproachFrontierCell::default());
        self.derived
            .spawn_contenders
            .waiting_entrants
            .resize(waiting_count, Vec::new());
        for entrants in &mut self.derived.spawn_contenders.waiting_entrants {
            entrants.clear();
        }
        let live_count = self.committed.live_order.len();
        #[cfg(any(test, feature = "placement-fixtures"))]
        {
            REBUILD_SCANS.with(|cell| cell.set(0));
            REBUILD_PREVIEWS.with(|cell| cell.set(0));
        }
        let mut reach_mask = std::mem::take(&mut self.derived.spawn_contenders.reach_mask);
        let masked = self.mark_reach_sources(&mut reach_mask);
        let lazy = masked && self.prepare_lazy_cells(cell_count);
        let scanned = if lazy {
            self.add_masked_contenders(&reach_mask)
        } else {
            self.add_live_contenders(live_count, masked.then_some(&reach_mask))
        };
        self.derived.spawn_contenders.reach_mask = reach_mask;
        scanned?;
        self.derived.spawn_contenders.lazy_cells = lazy;
        for entrants in &mut self.derived.spawn_contenders.waiting_entrants {
            sort_waiting_entrants(entrants)?;
        }
        Ok(())
    }

    /// 按 `live_order` 逐辆写入名单。给了 `reach_mask` 时，位图外的车只记格点，不预览。
    fn add_live_contenders(
        &mut self,
        live_count: usize,
        reach_mask: Option<&Vec<u64>>,
    ) -> Result<(), FreshAdmissionFailure> {
        for sequence in 0..live_count {
            #[cfg(any(test, feature = "placement-fixtures"))]
            REBUILD_SCANS.with(|cell| cell.set(cell.get().saturating_add(1)));
            let Ok(update_sequence) = u32::try_from(sequence) else {
                return Err(FreshAdmissionFailure::OccupancyAlloc);
            };
            let Some(handle) = self.committed.live_order.get(sequence).copied() else {
                return Err(FreshAdmissionFailure::StopConstraint);
            };
            let slot = handle.index() as usize;
            let may_reach = reach_mask.is_none_or(|mask| {
                mask.get(slot / 64)
                    .is_some_and(|word| word & (1u64 << (slot % 64)) != 0)
            });
            self.add_spawn_contender(handle, update_sequence, may_reach)?;
        }
        Ok(())
    }

    /// 按需模式只处理求值集合里的车：按槽位位图逐辆预览并写全部贡献，其余车不访问。
    /// 名单各表的归约都以 live 序号为键，与处理顺序无关。
    fn add_masked_contenders(&mut self, reach_mask: &[u64]) -> Result<(), FreshAdmissionFailure> {
        for (word_index, word) in reach_mask.iter().copied().enumerate() {
            let mut bits = word;
            while bits != 0 {
                let slot = word_index * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                #[cfg(any(test, feature = "placement-fixtures"))]
                REBUILD_SCANS.with(|cell| cell.set(cell.get().saturating_add(1)));
                let Some(handle) = self
                    .committed
                    .vehicles
                    .get(slot)
                    .and_then(|vehicle| vehicle.state)
                    .map(|state| state.handle)
                else {
                    continue;
                };
                let Some(Some(update_sequence)) = self
                    .derived
                    .live_order_index
                    .prepared_rank(&self.committed.live_order, handle)
                else {
                    return Err(FreshAdmissionFailure::StopConstraint);
                };
                self.add_spawn_contender(handle, update_sequence, true)?;
            }
        }
        Ok(())
    }

    /// 准备按需模式：live 序号表盖住当前 live 序，清空已求值格点。预留失败返回
    /// `false`，调用方逐辆写格点。
    fn prepare_lazy_cells(&mut self, cell_count: usize) -> bool {
        if admission_reserve_denied(AdmissionReserve::ReachMask)
            || !self
                .derived
                .live_order_index
                .ensure(&self.committed.live_order, self.committed.vehicles.len())
        {
            return false;
        }
        let Some(lazy) = self.derived.spawn_contenders.ensure_lazy() else {
            return false;
        };
        let words = cell_count.div_ceil(64);
        lazy.values.clear();
        lazy.evaluated.clear();
        if lazy.evaluated.try_reserve(words).is_err() {
            return false;
        }
        lazy.evaluated.resize(words, 0);
        true
    }

    /// 在 `mask` 里标出已发布近门集合、失效名单和本窗生命周期增量中仍然有效的车。
    ///
    /// 这一拍可能够到门的车都在其中（`traffic-runtime-contender-frontier.md` 第 3 节）。
    /// 没有证明时窗或路权策略、frontier 未发布或世界身份不符、位图预留失败时返回
    /// `false`，调用方逐辆预览。
    fn mark_reach_sources(&self, mask: &mut Vec<u64>) -> bool {
        if full_contender_rebuild()
            || self.binding.policy_binding.horizon().is_none()
            || self.read_view().policy().is_none()
        {
            return false;
        }
        let words = self.committed.vehicles.len().div_ceil(64);
        mask.clear();
        if admission_reserve_denied(AdmissionReserve::ReachMask) || mask.try_reserve(words).is_err()
        {
            return false;
        }
        mask.resize(words, 0);
        self.workspace
            .frontier_maintenance
            .for_each_published_source(
                self.binding.world_id,
                self.binding.world_generation,
                |vehicle| {
                    if self.vehicle_state(vehicle).is_none() {
                        return;
                    }
                    let slot = vehicle.index() as usize;
                    if let Some(word) = mask.get_mut(slot / 64) {
                        *word |= 1u64 << (slot % 64);
                    }
                },
            )
    }

    /// 沿这辆车自己的路线记下证明时窗内的格点到达，以及这一拍预览会申请的第一处冲突。
    /// `may_reach` 为假时这辆车这一拍够不到门，只记格点，不做运动预览。
    /// 分配失败返回 [`FreshAdmissionFailure::OccupancyAlloc`]。材料不齐不能当成停不住。
    fn add_spawn_contender(
        &mut self,
        handle: VehicleHandle,
        update_sequence: u32,
        may_reach: bool,
    ) -> Result<(), FreshAdmissionFailure> {
        let Some(state) = self.vehicle_state(handle) else {
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
        let preview_next = if may_reach {
            #[cfg(any(test, feature = "placement-fixtures"))]
            REBUILD_PREVIEWS.with(|cell| cell.set(cell.get().saturating_add(1)));
            let delta_s = self.binding.config.fixed_delta_time_ms() as f32 / 1_000.0;
            let Some(preview) = self
                .read_view()
                .preview_active_vehicle_with_waiting_stop(state, delta_s, None, None)
            else {
                return Err(FreshAdmissionFailure::StopConstraint);
            };
            Some(preview.next.apply(state))
        } else {
            None
        };
        let notes = self.contender_notes(
            &state,
            update_sequence,
            preview_next,
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
        self.apply_contender_notes(handle, update_sequence, notes)
    }

    fn contender_notes(
        &self,
        state: &VehicleState,
        update_sequence: u32,
        preview_next: Option<VehicleState>,
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
        let (cells, reached, waiting) = (|| {
            let read = self.read_view();
            let compiled = read.compiled_route(state.route)?;
            let lengths = read.binding.revision.traffic().lane_lengths_millimetres();
            let mut cells = Vec::new();
            let mut signal = None;
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
                    let signal = signal.get_or_insert_with(|| {
                        PreparedSignalApproach::new(read, state, emergency_decel)
                    });
                    let estimate = signal.apply(kinematic, distance_mm, horizon_ms);
                    if estimate != ApproachEstimate::OutsideHorizon {
                        note_reserve(&mut cells, 1)?;
                        cells.push((occurrence.address(), estimate));
                    }
                }
            }
            let mut reached = Vec::new();
            let mut waiting = None;
            // 没有预览说明这辆车这一拍够不到门，也越不过排队区入口。
            let Some(preview_next) = preview_next else {
                note_reserve(&mut cells, 0)?;
                return Some((cells, reached, waiting));
            };
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
            // 空结果仍经过同一可失败预留入口，保留故障注入与拒绝分类。
            note_reserve(&mut cells, 0)?;
            Some((cells, reached, waiting))
        })()?;
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
                    vehicle: state.handle,
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

    /// 格点 owner 与逐车记录都以这辆车的 live 序号为平局键，任何插入或重新归约顺序
    /// 都留下同样两名车（`traffic-runtime-contender-frontier.md` 4.3 节）。
    fn apply_contender_notes(
        &mut self,
        handle: VehicleHandle,
        update_sequence: u32,
        notes: ContenderNotes,
    ) -> Result<(), FreshAdmissionFailure> {
        let mut contributed_cells = Vec::new();
        let mut contributed_zones = Vec::new();
        for (address, estimate) in notes.cells {
            if estimate == ApproachEstimate::OutsideHorizon {
                continue;
            }
            let index = self.read_view().conflict_read().cell_index_of(address);
            let Some(index) = index else {
                return Err(FreshAdmissionFailure::StopConstraint);
            };
            let Some(slot) = self.derived.spawn_contenders.cell_approach.get_mut(index) else {
                return Err(FreshAdmissionFailure::StopConstraint);
            };
            slot.insert_owner_reduced(handle, update_sequence, estimate);
            contender_reserve(&mut contributed_cells, 1, AdmissionReserve::Cell)?;
            contributed_cells.push((index, estimate));
        }
        for (zone, rank, hop) in notes.ranks {
            contender_reserve(&mut contributed_zones, 1, AdmissionReserve::Best)?;
            contributed_zones.push(zone);
            let Some(list) = self.derived.spawn_contenders.best.get_mut(zone) else {
                return Err(FreshAdmissionFailure::StopConstraint);
            };
            contender_reserve(list, 1, AdmissionReserve::Best)?;
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
        let waiting_zone = if let Some((zone, _hop, mut entrant)) = notes.waiting {
            let Some(list) = self.derived.spawn_contenders.waiting_entrants.get_mut(zone) else {
                return Err(FreshAdmissionFailure::StopConstraint);
            };
            contender_reserve(list, 1, AdmissionReserve::Waiting)?;
            let position = list.partition_point(|item| {
                (item.approach_mm, item.update_sequence)
                    < (entrant.approach_mm, entrant.update_sequence)
            });
            entrant.vehicle = handle;
            list.insert(position, entrant);
            Some(zone)
        } else {
            None
        };
        self.store_owner(
            handle,
            OwnerContribution {
                update_sequence,
                zones: contributed_zones,
                cells: contributed_cells,
                waiting_zone,
            },
        )?;
        Ok(())
    }

    fn store_owner(
        &mut self,
        handle: VehicleHandle,
        contribution: OwnerContribution,
    ) -> Result<(), FreshAdmissionFailure> {
        let slot = handle.index() as usize;
        let owners = &mut self.derived.spawn_contenders.owners;
        if slot >= owners.len() {
            let extra = slot.saturating_add(1).saturating_sub(owners.len());
            contender_reserve(owners, extra, AdmissionReserve::Best)?;
            owners.resize_with(slot.saturating_add(1), || None);
        }
        let ranked = !contribution.zones.is_empty();
        owners[slot] = Some(contribution);
        if ranked {
            self.link_route_contender(handle);
        }
        Ok(())
    }

    /// 名单重建时清空「路线 → 申请者」链表表头。预留失败时这份名单不用索引。
    fn reset_route_contenders(&mut self) {
        let routes = self.committed.routes.len();
        let denied = admission_reserve_denied(AdmissionReserve::RecheckScratch);
        let Some(index) = self.derived.spawn_contenders.ensure_recheck_index() else {
            return;
        };
        index.linked = false;
        index.linked_count = 0;
        index.route_heads.clear();
        if denied || index.route_heads.try_reserve(routes).is_err() {
            return;
        }
        index.route_heads.resize(routes, NO_LINK);
        index.linked = true;
    }

    /// 把刚写入、冲突区列表非空的贡献记录链到它所在路线的表头。
    fn link_route_contender(&mut self, handle: VehicleHandle) {
        let route = self
            .vehicle_state(handle)
            .map(|state| state.route.index() as usize);
        let Some(index) = self.derived.spawn_contenders.recheck_index_mut() else {
            return;
        };
        if !index.linked {
            return;
        }
        let (Some(route), Ok(route_u32)) = (route, u32::try_from(route.unwrap_or(usize::MAX)))
        else {
            index.linked = false;
            return;
        };
        let slot = handle.index() as usize;
        if index.links.len() <= slot {
            let extra = slot + 1 - index.links.len();
            if index.links.try_reserve(extra).is_err() {
                index.linked = false;
                return;
            }
            index.links.resize(slot + 1, RouteContenderLink::EMPTY);
        }
        if index.route_heads.len() <= route {
            let extra = route + 1 - index.route_heads.len();
            if index.route_heads.try_reserve(extra).is_err() {
                index.linked = false;
                return;
            }
            index.route_heads.resize(route + 1, NO_LINK);
        }
        let next = index.route_heads[route];
        index.links[slot] = RouteContenderLink {
            vehicle: handle,
            route: route_u32,
            prev: NO_LINK,
            next,
        };
        if let Some(next) = index.links.get_mut(next as usize) {
            next.prev = handle.index();
        }
        index.route_heads[route] = handle.index();
        index.linked_count = index.linked_count.saturating_add(1);
    }

    /// 撤销冲突区列表非空的贡献记录时，把这辆车从路线链表里摘掉。
    fn unlink_route_contender(&mut self, handle: VehicleHandle) {
        let Some(index) = self.derived.spawn_contenders.recheck_index_mut() else {
            return;
        };
        if !index.linked {
            return;
        }
        let Some(link) = index.links.get(handle.index() as usize).copied() else {
            index.linked = false;
            return;
        };
        if link.vehicle != handle {
            index.linked = false;
            return;
        }
        if link.prev == NO_LINK {
            match index.route_heads.get_mut(link.route as usize) {
                Some(head) => *head = link.next,
                None => {
                    index.linked = false;
                    return;
                }
            }
        } else if let Some(prev) = index.links.get_mut(link.prev as usize) {
            prev.next = link.next;
        }
        if let Some(next) = index.links.get_mut(link.next as usize) {
            next.prev = link.prev;
        }
        index.linked_count = index.linked_count.saturating_sub(1);
    }

    fn edge_routes_signature(&self) -> (super::world::WorldGeneration, u32, usize) {
        (
            self.binding.world_generation,
            self.committed.live_route_count,
            self.committed.routes.len(),
        )
    }

    /// 复核能否查「边 → 路线」：已与路线注册表一致，或失效后的逐区扫描量已抵得上一次
    /// 重建（路线经过边的总次数）而重建成功。否则这次逐区扫描。
    fn edge_routes_ready(&mut self) -> bool {
        let signature = self.edge_routes_signature();
        let occurrences = self.committed.live_route_edge_occurrence_count;
        let Some(index) = self.derived.spawn_contenders.recheck_index() else {
            return false;
        };
        if index.edges_built_for == Some(signature) {
            return true;
        }
        index.scan_debt >= occurrences && self.ensure_edge_routes()
    }

    /// 「边 → 路线」压缩稀疏行与当前路线注册表一致；不一致就按计数排序重建。
    /// 预留失败或总次数超出 `u32` 下标时返回 `false`，复核退回逐区扫描。
    pub(crate) fn ensure_edge_routes(&mut self) -> bool {
        let signature = self.edge_routes_signature();
        let limit = edge_route_limit();
        let edge_count = self
            .binding
            .revision
            .traffic()
            .lane_lengths_millimetres()
            .len();
        let routes = &self.committed.routes;
        let Some(index) = self.derived.spawn_contenders.ensure_recheck_index() else {
            return false;
        };
        if index.edges_built_for == Some(signature) {
            return true;
        }
        index.edges_built_for = None;
        // 无论成败，下一次尝试都要再攒够一次重建的扫描量。
        index.scan_debt = 0;
        #[cfg(any(test, feature = "placement-fixtures"))]
        EDGE_ROUTE_REBUILDS.with(|cell| cell.set(cell.get().saturating_add(1)));
        let offsets = &mut index.edge_offsets;
        offsets.clear();
        if offsets.try_reserve(edge_count + 1).is_err() {
            return false;
        }
        offsets.resize(edge_count + 1, 0);
        // 计数：每条路线在每条经过的边上计一次（环路重复出现时重复计，查询时去重）。
        let mut total = 0usize;
        for slot in routes {
            let Some(compiled) = slot.compiled.as_ref() else {
                continue;
            };
            for edge in &compiled.edges {
                let Some(count) = offsets.get_mut(edge.index()) else {
                    return false;
                };
                if total as u64 >= limit {
                    return false;
                }
                *count += 1;
                total += 1;
            }
        }
        // 计数转为起点（独占前缀和），再按起点放置并推进，最后右移一位得到区间边界。
        let mut start = 0u32;
        for count in offsets.iter_mut().take(edge_count) {
            let here = *count;
            *count = start;
            start += here;
        }
        let edge_routes = &mut index.edge_routes;
        edge_routes.clear();
        if edge_routes.try_reserve(total).is_err() {
            return false;
        }
        edge_routes.resize(total, 0);
        for (route, slot) in routes.iter().enumerate() {
            let Some(compiled) = slot.compiled.as_ref() else {
                continue;
            };
            let Ok(route) = u32::try_from(route) else {
                return false;
            };
            for edge in &compiled.edges {
                let cursor = &mut offsets[edge.index()];
                edge_routes[*cursor as usize] = route;
                *cursor += 1;
            }
        }
        offsets.copy_within(0..edge_count, 1);
        offsets[0] = 0;
        index.edges_built_for = Some(signature);
        true
    }

    /// 某辆申请者在全冲突区扫描里第一次出现的位置：(冲突区编号, 名单内位置)。
    fn first_contender_position(&self, handle: VehicleHandle) -> Option<(usize, usize)> {
        let owner = self
            .derived
            .spawn_contenders
            .owners
            .get(handle.index() as usize)?
            .as_ref()?;
        owner
            .zones
            .iter()
            .filter_map(|zone| {
                let list = self.derived.spawn_contenders.best.get(*zone)?;
                let position = list.iter().position(|item| item.vehicle == handle)?;
                Some((*zone, position))
            })
            .min()
    }

    /// 路线经过车身所在边的申请者，按全冲突区扫描里第一次出现的顺序排列。
    fn body_contenders_by_index(
        &self,
        body: &[crate::DownstreamInterval],
    ) -> Result<Vec<VehicleHandle>, FreshAdmissionFailure> {
        let Some(index) = self.derived.spawn_contenders.recheck_index() else {
            return Err(FreshAdmissionFailure::StopConstraint);
        };
        let mut found: Vec<(usize, usize, VehicleHandle)> = Vec::new();
        for interval in body {
            let edge = interval.edge().index();
            let (Some(start), Some(end)) = (
                index.edge_offsets.get(edge).copied(),
                index.edge_offsets.get(edge + 1).copied(),
            ) else {
                continue;
            };
            let Some(routes) = index.edge_routes.get(start as usize..end as usize) else {
                return Err(FreshAdmissionFailure::StopConstraint);
            };
            for route in routes {
                let mut cursor = index
                    .route_heads
                    .get(*route as usize)
                    .copied()
                    .unwrap_or(NO_LINK);
                while cursor != NO_LINK {
                    let Some(link) = index.links.get(cursor as usize).copied() else {
                        return Err(FreshAdmissionFailure::StopConstraint);
                    };
                    cursor = link.next;
                    if found.iter().any(|(_, _, vehicle)| *vehicle == link.vehicle)
                        || self.vehicle_state(link.vehicle).is_none()
                    {
                        continue;
                    }
                    let Some((zone, position)) = self.first_contender_position(link.vehicle) else {
                        return Err(FreshAdmissionFailure::StopConstraint);
                    };
                    push_fallible(&mut found, (zone, position, link.vehicle))?;
                }
            }
        }
        found.sort_unstable_by_key(|(zone, position, _)| (*zone, *position));
        let mut vehicles = Vec::new();
        vehicles
            .try_reserve(found.len())
            .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
        vehicles.extend(found.into_iter().map(|(_, _, vehicle)| vehicle));
        Ok(vehicles)
    }

    /// 逐个冲突区扫描申请者名单，找出路线经过车身所在边的车，按第一次出现排列。
    /// 每辆车只判定一次；去重位图预留失败时逐条判定，结果相同。另返回检查过的名单
    /// 条目数。
    fn body_contenders_by_scan(
        &self,
        body: &[crate::DownstreamInterval],
        count_checks: bool,
    ) -> Result<(Vec<VehicleHandle>, u64), FreshAdmissionFailure> {
        let mut touching = Vec::new();
        let mut visited = 0u64;
        let words = self.derived.spawn_contenders.owners.len().div_ceil(64);
        let mut evaluated: Vec<u64> = Vec::new();
        if !admission_reserve_denied(AdmissionReserve::RecheckScratch)
            && evaluated.try_reserve_exact(words).is_ok()
        {
            evaluated.resize(words, 0);
        }
        for list in &self.derived.spawn_contenders.best {
            for contender in list {
                visited = visited.saturating_add(1);
                let vehicle = contender.vehicle;
                let slot = vehicle.index() as usize;
                let bit = 1u64 << (slot % 64);
                if evaluated
                    .get(slot / 64)
                    .is_some_and(|word| *word & bit != 0)
                {
                    continue;
                }
                if self.vehicle_state(vehicle).is_none() {
                    continue;
                }
                if let Some(word) = evaluated.get_mut(slot / 64) {
                    *word |= bit;
                }
                #[cfg(any(test, feature = "placement-fixtures"))]
                if count_checks {
                    RECHECK_BODY_CHECKS.with(|cell| cell.set(cell.get().saturating_add(1)));
                }
                #[cfg(not(any(test, feature = "placement-fixtures")))]
                let _ = count_checks;
                if self.route_touches_body(vehicle, body) {
                    push_recheck(&mut touching, vehicle)?;
                }
            }
        }
        Ok((touching, visited))
    }

    fn owner_zones(&self, handle: VehicleHandle) -> Result<Vec<usize>, FreshAdmissionFailure> {
        let Some(mark) = self
            .derived
            .spawn_contenders
            .owners
            .get(handle.index() as usize)
            .and_then(Option::as_ref)
        else {
            return Ok(Vec::new());
        };
        let mut zones = Vec::new();
        contender_reserve(&mut zones, mark.zones.len(), AdmissionReserve::Refresh)?;
        zones.extend_from_slice(&mark.zones);
        Ok(zones)
    }

    fn owner_sequence(&self, handle: VehicleHandle) -> Option<u32> {
        self.derived
            .spawn_contenders
            .owners
            .get(handle.index() as usize)
            .and_then(Option::as_ref)
            .map(|mark| mark.update_sequence)
    }

    fn revoke_owner(
        &mut self,
        handle: VehicleHandle,
        dirty: &mut Vec<usize>,
    ) -> Result<(), FreshAdmissionFailure> {
        let slot = handle.index() as usize;
        let Some(mark) = self
            .derived
            .spawn_contenders
            .owners
            .get_mut(slot)
            .and_then(Option::take)
        else {
            return Ok(());
        };
        if !mark.zones.is_empty() {
            self.unlink_route_contender(handle);
        }
        for zone in &mark.zones {
            if let Some(list) = self.derived.spawn_contenders.best.get_mut(*zone) {
                list.retain(|item| item.vehicle != handle);
            }
        }
        if let Some(zone) = mark.waiting_zone
            && let Some(list) = self.derived.spawn_contenders.waiting_entrants.get_mut(zone)
        {
            list.retain(|item| item.vehicle != handle);
        }
        for (index, _) in &mark.cells {
            remember_dirty(dirty, *index)?;
        }
        Ok(())
    }

    fn note_owner_cells(
        &self,
        handle: VehicleHandle,
        dirty: &mut Vec<usize>,
    ) -> Result<(), FreshAdmissionFailure> {
        let Some(mark) = self
            .derived
            .spawn_contenders
            .owners
            .get(handle.index() as usize)
            .and_then(Option::as_ref)
        else {
            return Ok(());
        };
        for (index, _) in &mark.cells {
            remember_dirty(dirty, *index)?;
        }
        Ok(())
    }

    fn rereduce_cell(&mut self, index: usize) {
        let mut reduced = ApproachFrontierCell::default();
        for (slot, owner) in self.derived.spawn_contenders.owners.iter().enumerate() {
            let Some(mark) = owner else {
                continue;
            };
            let Some(handle) = self
                .committed
                .vehicles
                .get(slot)
                .and_then(|vehicle| vehicle.state)
                .map(|state| state.handle)
            else {
                continue;
            };
            for (cell, estimate) in &mark.cells {
                if *cell != index {
                    continue;
                }
                reduced.insert_owner_reduced(handle, mark.update_sequence, *estimate);
            }
        }
        if let Some(slot) = self.derived.spawn_contenders.cell_approach.get_mut(index) {
            *slot = reduced;
        }
        if let Some(lazy) = self.derived.spawn_contenders.lazy_mut() {
            lazy.forget(index);
        }
    }

    fn remember_handle(
        affected: &mut Vec<VehicleHandle>,
        handle: VehicleHandle,
    ) -> Result<(), FreshAdmissionFailure> {
        if !affected.contains(&handle) {
            contender_reserve(affected, 1, AdmissionReserve::Refresh)?;
            affected.push(handle);
        }
        Ok(())
    }

    fn members_in_zones(
        &self,
        zones: &[usize],
        into: &mut Vec<VehicleHandle>,
        skip: VehicleHandle,
    ) -> Result<(), FreshAdmissionFailure> {
        for zone in zones {
            if let Some(list) = self.derived.spawn_contenders.best.get(*zone) {
                for contender in list {
                    if contender.vehicle != skip {
                        Self::remember_handle(into, contender.vehicle)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// 只刷新这次插入碰得到的旧车。每辆车只重算一次。预留失败时调用方整份作废。
    fn refresh_contender_cache(
        &mut self,
        handle: VehicleHandle,
        update_sequence: u32,
    ) -> Result<(), FreshAdmissionFailure> {
        #[cfg(any(test, feature = "placement-fixtures"))]
        INCREMENTAL_VISITS.with(|cell| cell.set(0));
        let notes = self.preview_notes(handle, update_sequence)?;
        let mut affected = Vec::new();
        let followers = self.follower_handles(handle)?;
        for follower in followers {
            if follower != handle {
                Self::remember_handle(&mut affected, follower)?;
            }
        }
        let mut zones = Vec::new();
        contender_reserve(&mut zones, notes.ranks.len(), AdmissionReserve::Refresh)?;
        zones.extend(notes.ranks.iter().map(|(zone, _, _)| *zone));
        self.members_in_zones(&zones, &mut affected, handle)?;
        if let Some((zone, _, _)) = notes.waiting
            && let Some(list) = self.derived.spawn_contenders.waiting_entrants.get(zone)
        {
            for entrant in list {
                if entrant.vehicle != handle {
                    Self::remember_handle(&mut affected, entrant.vehicle)?;
                }
            }
        }
        let mut dirty = Vec::new();
        let mut cursor = 0usize;
        while cursor < affected.len() {
            let current = affected[cursor];
            cursor = cursor.saturating_add(1);
            let Some(sequence) = self.owner_sequence(current) else {
                continue;
            };
            #[cfg(any(test, feature = "placement-fixtures"))]
            INCREMENTAL_VISITS.with(|cell| cell.set(cell.get().saturating_add(1)));
            let before = self.owner_zones(current)?;
            self.revoke_owner(current, &mut dirty)?;
            self.add_spawn_contender(current, sequence, true)?;
            self.note_owner_cells(current, &mut dirty)?;
            let after = self.owner_zones(current)?;
            if before != after {
                self.members_in_zones(&before, &mut affected, handle)?;
                self.members_in_zones(&after, &mut affected, handle)?;
            }
        }
        self.add_spawn_contender(handle, update_sequence, true)?;
        self.note_owner_cells(handle, &mut dirty)?;
        if self.derived.spawn_contenders.lazy_cells {
            // 新车已经写了全部贡献，按需读格点时不能再从 frontier 并入它的旧槽位。
            let mask = &mut self.derived.spawn_contenders.reach_mask;
            let slot = handle.index() as usize;
            let words = slot / 64 + 1;
            if mask.len() < words {
                contender_reserve(mask, words - mask.len(), AdmissionReserve::Refresh)?;
                mask.resize(words, 0);
            }
            mask[slot / 64] |= 1u64 << (slot % 64);
        }
        for index in dirty {
            self.rereduce_cell(index);
        }
        Ok(())
    }

    fn follower_handles(
        &mut self,
        handle: VehicleHandle,
    ) -> Result<Vec<VehicleHandle>, FreshAdmissionFailure> {
        let Some(state) = self.vehicle_state(handle) else {
            return Ok(Vec::new());
        };
        let input = VehicleSpawnInput::new(
            state.profile,
            state.route,
            state.route_edge_index,
            state.progress_mm,
            state.speed_mm_s,
        )
        .with_open_entrance();
        self.upstream_follower_candidates(input, state.length_mm)
    }

    fn preview_notes(
        &self,
        handle: VehicleHandle,
        update_sequence: u32,
    ) -> Result<ContenderNotes, FreshAdmissionFailure> {
        let Some(state) = self.vehicle_state(handle) else {
            return Ok(ContenderNotes {
                cells: Vec::new(),
                ranks: Vec::new(),
                waiting: None,
            });
        };
        if state.status != VehicleStatus::Active || !self.route_needs_contender(state.route) {
            return Ok(ContenderNotes {
                cells: Vec::new(),
                ranks: Vec::new(),
                waiting: None,
            });
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
            Some(preview.next.apply(state)),
            profile.max_accel(),
            profile.emergency_decel(),
            profile.min_gap_mm(),
        );
        if take_note_alloc() {
            return Err(FreshAdmissionFailure::OccupancyAlloc);
        }
        notes.ok_or(FreshAdmissionFailure::StopConstraint)
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
        &mut self,
        input: VehicleSpawnInput,
        profile: laneflow_static_network::VehicleProfileView,
        vehicle_length_mm: u32,
        delta_s: f32,
        update_sequence: u32,
    ) -> Result<(), FreshAdmissionFailure> {
        let state = preview_vehicle(input, profile, vehicle_length_mm);
        let Some(preview) = self
            .read_view()
            .preview_active_vehicle_with_waiting_stop(state, delta_s, None, None)
        else {
            return Err(FreshAdmissionFailure::StopConstraint);
        };
        let notes = self.contender_notes(
            &state,
            update_sequence,
            Some(preview.next.apply(state)),
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
        let motion = motion_or_stop(self.read_view().placement_motion(
            state,
            None,
            false,
            update_sequence,
            None,
            &notes.cells,
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
        let Some(preview) = self
            .read_view()
            .preview_active_vehicle_with_waiting_stop(state, delta_s, None, None)
        else {
            return Err(FreshAdmissionFailure::UnsafeLeader(contact.vehicle));
        };
        let notes = self.contender_notes(
            &state,
            update_sequence,
            Some(preview.next.apply(state)),
            profile.max_accel(),
            profile.emergency_decel(),
            profile.min_gap_mm(),
        );
        if take_note_alloc() {
            return Err(FreshAdmissionFailure::OccupancyAlloc);
        }
        let Some(notes) = notes else {
            return Err(FreshAdmissionFailure::UnsafeLeader(contact.vehicle));
        };
        let motion = match self.read_view().placement_motion(
            state,
            Some(contact.gap_mm),
            false,
            update_sequence,
            None,
            &notes.cells,
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
            let Some(follower) = self.vehicle_state(handle) else {
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
            let sequence = self
                .recorded_sequence(handle)
                .ok_or(FreshAdmissionFailure::UnsafeFollower(handle))?;
            let before =
                match self
                    .read_view()
                    .placement_motion(follower, None, false, sequence, None, &[])
                {
                    Ok(motion) => motion,
                    Err(PlacementMotionError::Alloc) => {
                        return Err(FreshAdmissionFailure::OccupancyAlloc);
                    }
                    Err(PlacementMotionError::Unprovable) => {
                        return Err(FreshAdmissionFailure::UnsafeFollower(handle));
                    }
                };
            let after = match self.read_view().placement_motion(
                follower,
                Some(candidate_gap),
                false,
                sequence,
                None,
                &[],
            ) {
                Ok(motion) => motion,
                Err(PlacementMotionError::Alloc) => {
                    return Err(FreshAdmissionFailure::OccupancyAlloc);
                }
                Err(PlacementMotionError::Unprovable) => {
                    return Err(FreshAdmissionFailure::UnsafeFollower(handle));
                }
            };
            let before_bad = projection_exceeds_emergency(
                follower.speed_mm_s,
                before,
                profile.emergency_decel(),
                delta_s,
            );
            let after_bad = projection_exceeds_emergency(
                follower.speed_mm_s,
                after,
                profile.emergency_decel(),
                delta_s,
            );
            if !before_bad && after_bad {
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
        let Some(registered) = self.route_edges(input.route()) else {
            return Ok(Vec::new());
        };
        let Ok(cursor) = usize::try_from(input.route_edge_index()) else {
            return Ok(Vec::new());
        };
        // 距离表借出期间不能再借用整份世界。路线边先复制出来。
        let mut edges = Vec::new();
        edges
            .try_reserve(registered.len())
            .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
        edges.extend_from_slice(registered);
        let traffic = self.binding.revision.traffic();
        self.workspace
            .occupancy_scratch
            .ensure_maneuver_upstream(traffic)
            .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
        let edge_count = usize::try_from(traffic.lane_edge_count()).unwrap_or(0);
        let generation = self
            .workspace
            .occupancy_scratch
            .begin_upstream_search(edge_count)
            .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
        let (upstream_seen, upstream_distance) =
            self.workspace.occupancy_scratch.take_upstream_tables();
        let mut restore_upstream = UpstreamTableRestore {
            scratch: &mut self.workspace.occupancy_scratch,
            seen: upstream_seen,
            distance: upstream_distance,
        };
        let lengths = traffic.lane_lengths_millimetres();
        let mut body: Vec<(laneflow_static_contract::LaneEdgeOrdinal, u32)> = Vec::new();
        let mut alloc_failed = false;
        let _ = for_each_admission_interval(
            lengths,
            &edges,
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
            generation,
            body_edges: Vec::new(),
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
                    restore_upstream.scratch,
                    &mut restore_upstream.seen,
                    &mut restore_upstream.distance,
                    edge,
                    rear_lo,
                    &mut search,
                )?;
            }
        }
        while let Some(Reverse((behind_end, raw))) = search.pending.pop() {
            if upstream_best(
                &restore_upstream.seen,
                &restore_upstream.distance,
                search.generation,
                raw,
            ) != Some(behind_end)
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
                    restore_upstream.scratch,
                    &mut restore_upstream.seen,
                    &mut restore_upstream.distance,
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

    /// 活动顺序里的序号。先用争用名单上记下的，没有再看占用记录。
    fn recorded_sequence(&self, handle: VehicleHandle) -> Option<u32> {
        if let Some(sequence) = self.owner_sequence(handle) {
            return Some(sequence);
        }
        let state = self.vehicle_state(handle)?;
        let edges = self.route_edges(state.route)?;
        let index = usize::try_from(state.route_edge_index).ok()?;
        let edge = *edges.get(index)?;
        let mut found = None;
        self.derived.occupancy.for_each_record_in_hi_window(
            edge,
            state.progress_mm,
            state.progress_mm,
            |vehicle, sequence| {
                if vehicle == handle {
                    found = Some(sequence);
                }
            },
        );
        found
    }

    /// 车身铺开的半开区间。贴成一点的零长度不占下游。
    fn candidate_body_intervals(
        &self,
        state: &VehicleState,
    ) -> Result<Vec<crate::DownstreamInterval>, FreshAdmissionFailure> {
        let Some(compiled) = self.compiled_route(state.route) else {
            return Err(FreshAdmissionFailure::StopConstraint);
        };
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
        let Ok(index) = usize::try_from(state.route_edge_index) else {
            return Err(FreshAdmissionFailure::StopConstraint);
        };
        let mut intervals = Vec::new();
        let slots = body_interval_slots(state.length_mm);
        super::tick::note_body_reserve(slots);
        intervals
            .try_reserve(slots)
            .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
        let walked = for_each_occupancy_interval(
            lengths,
            &compiled.edges,
            index,
            state.progress_mm,
            state.length_mm,
            |edge, start_mm, end_mm| {
                if let Some(interval) = crate::DownstreamInterval::new(edge, start_mm, end_mm) {
                    intervals.push(interval);
                }
            },
        );
        if walked.is_none() {
            return Err(FreshAdmissionFailure::StopConstraint);
        }
        Ok(intervals)
    }

    fn route_touches_body(
        &self,
        handle: VehicleHandle,
        body: &[crate::DownstreamInterval],
    ) -> bool {
        let Some(state) = self.vehicle_state(handle) else {
            return false;
        };
        let Some(edges) = self.route_edges(state.route) else {
            return false;
        };
        edges
            .iter()
            .any(|edge| body.iter().any(|interval| interval.edge() == *edge))
    }

    /// 这次新车可能碰到的已有车：后车、同区申请者、让行格点上的车、车身、已提交下游，以及这一拍会申到同一段下游的车。
    fn recheck_targets(
        &mut self,
        candidate: &VehicleState,
        notes: &ContenderNotes,
        claims: &[crate::DownstreamInterval],
        claim_gap_mm: u32,
    ) -> Result<Vec<(u32, VehicleHandle)>, FreshAdmissionFailure> {
        if super::tick::full_recheck_enabled() {
            let mut paired = Vec::new();
            paired
                .try_reserve(self.committed.live_order.len())
                .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
            for (sequence, handle) in self.committed.live_order.iter().copied().enumerate() {
                let Ok(sequence) = u32::try_from(sequence) else {
                    return Err(FreshAdmissionFailure::OccupancyAlloc);
                };
                paired.push((sequence, handle));
            }
            return Ok(paired);
        }
        let mut handles = Vec::new();
        let input = VehicleSpawnInput::new(
            candidate.profile,
            candidate.route,
            candidate.route_edge_index,
            candidate.progress_mm,
            candidate.speed_mm_s,
        )
        .with_open_entrance();
        for handle in self.upstream_follower_candidates(input, candidate.length_mm)? {
            push_recheck(&mut handles, handle)?;
        }
        let mut zones = Vec::new();
        zones
            .try_reserve(notes.ranks.len())
            .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
        for (zone, _, _) in &notes.ranks {
            if !zones.contains(zone) {
                zones.push(*zone);
            }
        }
        let mut zone_cursor = 0usize;
        while zone_cursor < zones.len() {
            let zone = zones[zone_cursor];
            zone_cursor = zone_cursor.saturating_add(1);
            self.push_zone_contenders(zone, &mut handles)?;
        }
        if let Some((zone, _, _)) = notes.waiting {
            self.push_waiting_entrants(zone, &mut handles)?;
        }
        for (address, _) in &notes.cells {
            self.push_zone_contenders(address.zone().index(), &mut handles)?;
            let Some(index) = self.read_view().conflict_read().cell_index_of(*address) else {
                continue;
            };
            let slot = match self.read_view().contender_cell(index, *address) {
                Ok(Some(slot)) => slot,
                Ok(None) => continue,
                Err(()) => return Err(FreshAdmissionFailure::StopConstraint),
            };
            for owner in slot.retained_owners().into_iter().flatten() {
                push_recheck(&mut handles, owner)?;
            }
        }
        let body = self.candidate_body_intervals(candidate)?;
        if !body.is_empty() {
            let body_gap = self
                .binding
                .revision
                .traffic()
                .relations()
                .vehicle_profile(candidate.profile)
                .map(|profile| profile.min_gap_mm())
                .ok_or(FreshAdmissionFailure::StopConstraint)?;
            // 路线经过车身所在边的申请者：先查「边 → 路线 → 申请者」索引，索引不可用时
            // 逐个冲突区扫描。各路径得到同样的车和同样的先后。
            let linked = if admission_reserve_denied(AdmissionReserve::RecheckScratch) {
                None
            } else {
                self.derived
                    .spawn_contenders
                    .recheck_index()
                    .filter(|index| index.linked)
                    .map(|index| index.linked_count)
            };
            let touching = if linked == Some(0) {
                #[cfg(any(test, feature = "placement-fixtures"))]
                assert!(
                    self.body_contenders_by_scan(&body, false)?.0.is_empty(),
                    "an empty route index must match the zone scan"
                );
                Vec::new()
            } else if linked.is_some() && self.edge_routes_ready() {
                let found = self.body_contenders_by_index(&body)?;
                #[cfg(any(test, feature = "placement-fixtures"))]
                {
                    RECHECK_BODY_CHECKS.with(|cell| {
                        cell.set(cell.get().saturating_add(found.len() as u64));
                    });
                    assert_eq!(
                        found,
                        self.body_contenders_by_scan(&body, false)?.0,
                        "route index must match the zone scan"
                    );
                }
                found
            } else {
                let (found, visited) = self.body_contenders_by_scan(&body, true)?;
                if linked.is_some()
                    && let Some(index) = self.derived.spawn_contenders.recheck_index_mut()
                {
                    index.scan_debt = index.scan_debt.saturating_add(visited);
                }
                found
            };
            for vehicle in touching {
                push_recheck(&mut handles, vehicle)?;
            }
            let mut overlapped = Vec::new();
            let claim_count = self.read_view().conflict_read().committed_downstream_len();
            overlapped
                .try_reserve(claim_count)
                .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
            self.read_view()
                .conflict_read()
                .for_each_committed_downstream(|owner, interval, gap| {
                    if body
                        .iter()
                        .any(|occupied| intervals_conflict(*occupied, body_gap, interval, gap))
                    {
                        overlapped.push(owner);
                    }
                });
            for owner in overlapped {
                push_recheck(&mut handles, owner)?;
            }
            for interval in &body {
                let mut present = Vec::new();
                let mut allocation_failed = false;
                self.derived.occupancy.for_each_record_in_hi_window(
                    interval.edge(),
                    interval.start_mm(),
                    interval.end_mm().saturating_sub(1),
                    |vehicle, _| {
                        if present.try_reserve(1).is_err() {
                            allocation_failed = true;
                            return;
                        }
                        present.push(vehicle);
                    },
                );
                if allocation_failed {
                    return Err(FreshAdmissionFailure::OccupancyAlloc);
                }
                for vehicle in present {
                    push_recheck(&mut handles, vehicle)?;
                }
            }
        }
        let sharing = self
            .read_view()
            .contenders_sharing_downstream(claims, claim_gap_mm)
            .map_err(|error| match error {
                super::tick::PlacementMotionError::Alloc => FreshAdmissionFailure::OccupancyAlloc,
                super::tick::PlacementMotionError::Unprovable => {
                    FreshAdmissionFailure::StopConstraint
                }
            })?;
        for handle in sharing {
            push_recheck(&mut handles, handle)?;
        }
        let mut paired = Vec::new();
        paired
            .try_reserve(handles.len())
            .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
        for handle in handles {
            if handle == candidate.handle {
                continue;
            }
            let Some(state) = self.vehicle_state(handle) else {
                continue;
            };
            if state.status != VehicleStatus::Active {
                continue;
            }
            let Some(sequence) = self.recorded_sequence(handle) else {
                continue;
            };
            paired.push((sequence, handle));
        }
        Ok(paired)
    }

    fn push_zone_contenders(
        &self,
        zone: usize,
        handles: &mut Vec<VehicleHandle>,
    ) -> Result<(), FreshAdmissionFailure> {
        let Some(len) = self
            .derived
            .spawn_contenders
            .best
            .get(zone)
            .map(|list| list.len())
        else {
            return Ok(());
        };
        let mut item = 0usize;
        while item < len {
            let Some(vehicle) = self
                .derived
                .spawn_contenders
                .best
                .get(zone)
                .and_then(|list| list.get(item))
                .map(|contender| contender.vehicle)
            else {
                break;
            };
            item = item.saturating_add(1);
            push_recheck(handles, vehicle)?;
        }
        Ok(())
    }

    fn push_waiting_entrants(
        &self,
        zone: usize,
        handles: &mut Vec<VehicleHandle>,
    ) -> Result<(), FreshAdmissionFailure> {
        let Some(len) = self
            .derived
            .spawn_contenders
            .waiting_entrants
            .get(zone)
            .map(|list| list.len())
        else {
            return Ok(());
        };
        let mut item = 0usize;
        while item < len {
            let Some(vehicle) = self
                .derived
                .spawn_contenders
                .waiting_entrants
                .get(zone)
                .and_then(|list| list.get(item))
                .map(|entrant| entrant.vehicle)
            else {
                break;
            };
            item = item.saturating_add(1);
            push_recheck(handles, vehicle)?;
        }
        Ok(())
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
    generation: u32,
    body_edges: Vec<u32>,
    pending: BinaryHeap<Reverse<(u32, u32)>>,
}

struct UpstreamTableRestore<'a> {
    scratch: &'a mut super::occupancy::OccupancyScratch,
    seen: Vec<u32>,
    distance: Vec<u32>,
}

impl Drop for UpstreamTableRestore<'_> {
    fn drop(&mut self) {
        self.scratch.restore_upstream_tables(
            std::mem::take(&mut self.seen),
            std::mem::take(&mut self.distance),
        );
    }
}

fn upstream_best(seen: &[u32], distance: &[u32], generation: u32, raw: u32) -> Option<u32> {
    let index = usize::try_from(raw).ok()?;
    (seen.get(index).copied() == Some(generation))
        .then_some(distance.get(index).copied())
        .flatten()
}

fn relax_follower_upstream(
    traffic: &laneflow_static_network::SharedTrafficNetwork,
    scratch: &super::occupancy::OccupancyScratch,
    seen: &mut [u32],
    distance: &mut [u32],
    edge: laneflow_static_contract::LaneEdgeOrdinal,
    behind_end: u32,
    search: &mut UpstreamSearch,
) -> Result<(), FreshAdmissionFailure> {
    if behind_end > search.reach {
        return Ok(());
    }
    if let Some(predecessors) = traffic.predecessors(edge) {
        for predecessor in predecessors {
            note_shorter_upstream(*predecessor, behind_end, search, seen, distance)?;
        }
    }
    for raw in scratch.maneuver_upstream(edge) {
        note_shorter_upstream(
            laneflow_static_contract::LaneEdgeOrdinal::from_raw(*raw),
            behind_end,
            search,
            seen,
            distance,
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
    seen: &mut [u32],
    distance: &mut [u32],
) -> Result<(), FreshAdmissionFailure> {
    let raw = edge.raw();
    // 后杠那一次出现已经开过窗。绕环后再遇到同一条物理边时，behind_end 是走过的正距离。
    if behind_end == 0 && search.body_edges.contains(&raw) {
        return Ok(());
    }
    if !note_upstream_distance(seen, distance, search.generation, raw, behind_end) {
        return Ok(());
    }
    search
        .pending
        .try_reserve(1)
        .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
    search.pending.push(Reverse((behind_end, raw)));
    Ok(())
}

fn note_upstream_distance(
    seen: &mut [u32],
    distance: &mut [u32],
    generation: u32,
    raw: u32,
    behind_end: u32,
) -> bool {
    let Ok(index) = usize::try_from(raw) else {
        return false;
    };
    let (Some(seen), Some(best)) = (seen.get_mut(index), distance.get_mut(index)) else {
        return false;
    };
    if *seen == generation {
        if *best <= behind_end {
            return false;
        }
        *best = behind_end;
        return true;
    }
    *seen = generation;
    *best = behind_end;
    true
}

fn remember_dirty(dirty: &mut Vec<usize>, index: usize) -> Result<(), FreshAdmissionFailure> {
    if dirty.contains(&index) {
        return Ok(());
    }
    dirty
        .try_reserve(1)
        .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
    dirty.push(index);
    Ok(())
}

fn push_recheck(
    handles: &mut Vec<VehicleHandle>,
    handle: VehicleHandle,
) -> Result<(), FreshAdmissionFailure> {
    if handles.contains(&handle) {
        return Ok(());
    }
    push_fallible(handles, handle)
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
/// 旧车本来就会急停的，不算这次造成的。没有路口申请的路线仍要看车身占住的下游。
fn reject_existing_hard_stop(
    world: &mut crate::kernel::state::WorldState,
    candidate: VehicleState,
    update_sequence: u32,
    profile: laneflow_static_network::VehicleProfileView,
    delta_s: f32,
) -> Result<(), FreshAdmissionFailure> {
    let (notes, candidate_blocked_at, candidate_claims) =
        if world.route_needs_contender(candidate.route) {
            let Some(preview) = world
                .read_view()
                .preview_active_vehicle_with_waiting_stop(candidate, delta_s, None, None)
            else {
                return Err(FreshAdmissionFailure::StopConstraint);
            };
            let notes = world.contender_notes(
                &candidate,
                update_sequence,
                Some(preview.next.apply(candidate)),
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
            let (candidate_blocked_at, candidate_claims) = match world.read_view().candidate_hold(
                &candidate,
                update_sequence,
                notes.ranks.first().map(|(_, _, hop)| *hop),
                &notes.cells,
            ) {
                Ok(hold) => hold,
                Err(super::tick::PlacementMotionError::Alloc) => {
                    return Err(FreshAdmissionFailure::OccupancyAlloc);
                }
                Err(super::tick::PlacementMotionError::Unprovable) => {
                    return Err(FreshAdmissionFailure::StopConstraint);
                }
            };
            (notes, candidate_blocked_at, candidate_claims)
        } else {
            (
                ContenderNotes {
                    cells: Vec::new(),
                    ranks: Vec::new(),
                    waiting: None,
                },
                None,
                Vec::new(),
            )
        };
    let targets =
        world.recheck_targets(&candidate, &notes, &candidate_claims, profile.min_gap_mm())?;
    for (existing_sequence, handle) in targets {
        let Some(existing) = world.vehicle_state(handle) else {
            continue;
        };
        if existing.status != VehicleStatus::Active {
            continue;
        }
        super::tick::note_recheck_visit();
        let induced = match world.read_view().incoming_hard_stop(
            &existing,
            existing_sequence,
            super::tick::IncomingPressure {
                approaches: &notes.cells,
                ranks: &notes.ranks,
                waiting: notes.waiting,
                candidate: &candidate,
                candidate_blocked_at,
                candidate_claims: &candidate_claims,
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
            &[],
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
            &[],
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

impl crate::kernel::phase::StepReadView<'_> {
    /// 准入候选名单在格点 `cell`（地址 `address`）上留下的两名车。
    ///
    /// 名单处于按需模式时，第一次读取把求值集合外、frontier 反向索引里的来源车并入
    /// 并缓存到名单失效。格点下标不存在返回 `Ok(None)`。按需求值的前提不成立（没有
    /// frontier、来源槽位与当前状态对不上、到达下界不可证明）返回 `Err(())`，调用方
    /// 按不可证明处理。
    pub(crate) fn contender_cell(
        self,
        cell: usize,
        address: crate::ConflictPassageAddress,
    ) -> Result<Option<ApproachFrontierCell>, ()> {
        let contenders = &self.derived.spawn_contenders;
        let Some(eager) = contenders.cell_approach.get(cell).copied() else {
            return Ok(None);
        };
        if !contenders.lazy_cells {
            return Ok(Some(eager));
        }
        let frontier = self.contender_frontier.ok_or(())?;
        let mut lazy = contenders.lazy().ok_or(())?.lock().map_err(|_| ())?;
        if let Some(value) = lazy.get(cell) {
            return Ok(Some(value));
        }
        let mut value = eager;
        self.merge_lazy_cell_sources(frontier, address, &mut value)?;
        // 缓存失败只影响下一次读取要不要重算，这次结果照样可用。
        let _ = lazy.remember(cell, value);
        Ok(Some(value))
    }

    /// 把求值集合外的来源车并入格点。每辆来源车的到达下界与 `contender_notes` 相同：
    /// 当前位置到这处出现项的距离 = 缓存距离 − 同一条边上的进度差；已越过的不计，
    /// 证明时窗外的不计。
    fn merge_lazy_cell_sources(
        self,
        frontier: &FrontierMaintenance,
        address: crate::ConflictPassageAddress,
        value: &mut ApproachFrontierCell,
    ) -> Result<(), ()> {
        let horizon_ms = self.binding.policy_binding.horizon().ok_or(())?;
        let mask = &self.derived.spawn_contenders.reach_mask;
        frontier.for_each_cached_occurrence(address, |occurrence: CachedOccurrence| {
            let slot = occurrence.slot as usize;
            if mask
                .get(slot / 64)
                .is_some_and(|word| word & (1u64 << (slot % 64)) != 0)
            {
                return Ok(());
            }
            let handle = VehicleHandle::new(occurrence.slot, occurrence.generation);
            // 句柄过期、已完成或停车的车不再是来源，全量重建同样不计。槽位换了新车时，
            // 新车经过生命周期登记，已在求值集合里或已把旧缓存摘掉。
            let Some(state) = self
                .vehicle_state(handle)
                .filter(|state| state.status == VehicleStatus::Active)
            else {
                return Ok(());
            };
            // 求值集合外的 Active 车没有经历生命周期变化，槽位缓存必须还对得上当前状态。
            if state.route.index() != occurrence.route_index
                || state.route.generation() != occurrence.route_generation
                || state.route_edge_index != occurrence.edge
                || state.progress_mm < occurrence.progress_mm
            {
                return Err(());
            }
            let compiled = self.compiled_route(state.route).ok_or(())?;
            if compiled.conflicts.is_empty() && compiled.waiting.is_empty() {
                return Ok(());
            }
            let traveled = state.progress_mm - occurrence.progress_mm;
            let Some(distance_mm) = occurrence.distance_mm.checked_sub(traveled) else {
                return Ok(());
            };
            let profile = self
                .binding
                .revision
                .traffic()
                .relations()
                .vehicle_profile(state.profile)
                .ok_or(())?;
            let prepared = PreparedApproachEta::new(
                state.carry_um,
                state.speed_mm_s,
                profile.max_accel(),
                horizon_ms,
            )
            .ok_or(())?;
            let kinematic = prepared.lower_bound(u64::from(distance_mm));
            if kinematic == ApproachEstimate::OutsideHorizon {
                return Ok(());
            }
            let estimate = PreparedSignalApproach::new(self, &state, profile.emergency_decel())
                .apply(kinematic, distance_mm, horizon_ms);
            if estimate == ApproachEstimate::OutsideHorizon {
                return Ok(());
            }
            let Some(Some(update_sequence)) = self
                .derived
                .live_order_index
                .prepared_rank(&self.committed.live_order, handle)
            else {
                return Err(());
            };
            #[cfg(any(test, feature = "placement-fixtures"))]
            LAZY_SOURCES.with(|cell| cell.set(cell.get().saturating_add(1)));
            value.insert_owner_reduced(handle, update_sequence, estimate);
            Ok(())
        })
    }
}
