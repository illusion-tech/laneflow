//! 冲突候选、间隙证明与组合资源的单写者核心。
//!
//! W7 已把仲裁、crossing、tail-clear、快照与在线切换接入生产固定步进。
#![cfg_attr(not(test), allow(dead_code))]

use core::cmp::Ordering;

use laneflow_static_contract::{
    ConflictZoneId, ConflictZoneOrdinal, GateInterpretation, GateProhibition, LaneEdgeOrdinal,
    ParticipantStreamId, ParticipantStreamOrdinal, SignalAspect, WaitingZoneOrdinal,
};
use laneflow_static_network::ResolvedGatePolicy;

use crate::{RouteHandle, VehicleHandle};

#[cfg(test)]
thread_local! {
    static CONFLICT_WORK_COUNTS: core::cell::Cell<ConflictWorkCounts> =
        const { core::cell::Cell::new(ConflictWorkCounts::ZERO) };
    static CONFLICT_ALLOCATION_FAILPOINT: core::cell::Cell<Option<usize>> = const { core::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_allocation_failpoint(remaining: Option<usize>) {
    CONFLICT_ALLOCATION_FAILPOINT.with(|slot| slot.set(remaining));
}

#[cfg(test)]
pub(crate) fn check_allocation_failpoint() -> Result<(), ConflictAcquireError> {
    CONFLICT_ALLOCATION_FAILPOINT.with(|slot| match slot.get() {
        Some(0) => {
            slot.set(None);
            Err(ConflictAcquireError::ScratchAllocFailed)
        }
        Some(remaining) => {
            slot.set(Some(remaining - 1));
            Ok(())
        }
        None => Ok(()),
    })
}

/// 测试专用访问计数；不进入生产世界布局、公开 API 或持久状态。
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ConflictWorkCounts {
    pub(crate) vehicle_grant_lookups: usize,
    pub(crate) grant_update_lookups: usize,
    pub(crate) visited_passages: usize,
    pub(crate) frontier_updates: usize,
    pub(crate) candidates: usize,
    pub(crate) yield_queries: usize,
    pub(crate) cell_claim_queries: usize,
    pub(crate) downstream_claim_queries: usize,
    pub(crate) downstream_interval_visits: usize,
    pub(crate) owner_record_moves: usize,
    pub(crate) commit_resource_visits: usize,
    pub(crate) collision_rejections: usize,
    pub(crate) wait_for_nodes: usize,
    pub(crate) wait_for_edges: usize,
    pub(crate) wait_for_visits: usize,
    pub(crate) wait_for_reorders: usize,
    pub(crate) wait_for_rollbacks: usize,
    pub(crate) wait_for_thresholds: usize,
}

#[cfg(test)]
impl ConflictWorkCounts {
    const ZERO: Self = Self {
        vehicle_grant_lookups: 0,
        grant_update_lookups: 0,
        visited_passages: 0,
        frontier_updates: 0,
        candidates: 0,
        yield_queries: 0,
        cell_claim_queries: 0,
        downstream_claim_queries: 0,
        downstream_interval_visits: 0,
        owner_record_moves: 0,
        commit_resource_visits: 0,
        collision_rejections: 0,
        wait_for_nodes: 0,
        wait_for_edges: 0,
        wait_for_visits: 0,
        wait_for_reorders: 0,
        wait_for_rollbacks: 0,
        wait_for_thresholds: 0,
    };
}

#[cfg(test)]
pub(crate) fn count_conflict_work(update: impl FnOnce(&mut ConflictWorkCounts)) {
    CONFLICT_WORK_COUNTS.with(|counts| {
        let mut value = counts.get();
        update(&mut value);
        counts.set(value);
    });
}

#[cfg(test)]
pub(crate) fn reset_conflict_work_counts() {
    CONFLICT_WORK_COUNTS.with(|counts| counts.set(ConflictWorkCounts::ZERO));
}

#[cfg(test)]
pub(crate) fn conflict_work_counts() -> ConflictWorkCounts {
    CONFLICT_WORK_COUNTS.with(core::cell::Cell::get)
}

/// 静态 passage cell 的规范地址；同一流中的 owner-local 下标不会跨流解释。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ConflictPassageAddress {
    zone: ConflictZoneOrdinal,
    stream: ParticipantStreamOrdinal,
    passage_local_index: u32,
}

impl ConflictPassageAddress {
    pub(crate) const fn new(
        zone: ConflictZoneOrdinal,
        stream: ParticipantStreamOrdinal,
        passage_local_index: u32,
    ) -> Self {
        Self {
            zone,
            stream,
            passage_local_index,
        }
    }

    #[must_use]
    pub const fn zone(self) -> ConflictZoneOrdinal {
        self.zone
    }

    #[must_use]
    pub const fn stream(self) -> ParticipantStreamOrdinal {
        self.stream
    }

    #[must_use]
    pub const fn passage_local_index(self) -> u32 {
        self.passage_local_index
    }
}

/// 可持久化、可跨修订重绑定的冲突通行段定位值。
///
/// 同一 `(ParticipantStream, ConflictZone)` 在受检 LFCA 中至多对应一条 passage；
/// Runtime ordinal/local index 不进入该值。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConflictPassageLocator {
    participant_stream_stable_id: ParticipantStreamId,
    conflict_zone_stable_id: ConflictZoneId,
}

impl ConflictPassageLocator {
    pub(crate) const fn new(
        participant_stream_stable_id: ParticipantStreamId,
        conflict_zone_stable_id: ConflictZoneId,
    ) -> Self {
        Self {
            participant_stream_stable_id,
            conflict_zone_stable_id,
        }
    }

    #[must_use]
    pub const fn participant_stream_stable_id(self) -> ParticipantStreamId {
        self.participant_stream_stable_id
    }

    #[must_use]
    pub const fn conflict_zone_stable_id(self) -> ConflictZoneId {
        self.conflict_zone_stable_id
    }
}

/// 动态 Route 中一次 passage occurrence 的精确运行时定位信息。
///
/// `conflict_occurrence_index` 区分循环路线中稳定 locator 相同的重复出现项。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConflictPassageOccurrenceLocator {
    route: RouteHandle,
    maneuver_occurrence_index: u32,
    admission_gate_hop: u32,
    conflict_occurrence_index: u32,
    address: ConflictPassageAddress,
    stable_locator: ConflictPassageLocator,
}

impl ConflictPassageOccurrenceLocator {
    pub(crate) const fn new(
        route: RouteHandle,
        maneuver_occurrence_index: u32,
        admission_gate_hop: u32,
        conflict_occurrence_index: u32,
        address: ConflictPassageAddress,
        stable_locator: ConflictPassageLocator,
    ) -> Self {
        Self {
            route,
            maneuver_occurrence_index,
            admission_gate_hop,
            conflict_occurrence_index,
            address,
            stable_locator,
        }
    }

    #[must_use]
    pub const fn route(self) -> RouteHandle {
        self.route
    }

    #[must_use]
    pub const fn maneuver_occurrence_index(self) -> u32 {
        self.maneuver_occurrence_index
    }

    #[must_use]
    pub const fn admission_gate_hop(self) -> u32 {
        self.admission_gate_hop
    }

    #[must_use]
    pub const fn conflict_occurrence_index(self) -> u32 {
        self.conflict_occurrence_index
    }

    #[must_use]
    pub const fn address(self) -> ConflictPassageAddress {
        self.address
    }

    #[must_use]
    pub const fn stable_locator(self) -> ConflictPassageLocator {
        self.stable_locator
    }
}

/// 策略解释后、进入资源仲裁前的候选类型。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GateCandidateKind {
    Protected,
    Permissive,
    Uncontrolled,
}

/// 门规则只生成 deny 或候选，不直接授予通行权。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GatePolicyDecision {
    DenyAndStop,
    Candidate(GateCandidateKind),
}

/// 按受检 gate rule 解释当前灯态。错误 binding 失败关闭。
pub(crate) fn interpret_gate_policy(
    rule: ResolvedGatePolicy,
    signal_bound: bool,
    aspect: Option<SignalAspect>,
) -> Option<GatePolicyDecision> {
    interpret_gate_declaration(
        rule.interpretation(),
        rule.prohibition(),
        signal_bound,
        aspect,
    )
}

fn interpret_gate_declaration(
    interpretation: GateInterpretation,
    prohibition: GateProhibition,
    signal_bound: bool,
    aspect: Option<SignalAspect>,
) -> Option<GatePolicyDecision> {
    if prohibition == GateProhibition::Always {
        return Some(GatePolicyDecision::DenyAndStop);
    }
    if (interpretation == GateInterpretation::Uncontrolled) == signal_bound {
        return None;
    }
    if !signal_bound {
        if prohibition == GateProhibition::OnRed || aspect.is_some() {
            return None;
        }
        return Some(GatePolicyDecision::Candidate(
            GateCandidateKind::Uncontrolled,
        ));
    }
    let aspect = aspect?;
    if prohibition == GateProhibition::OnRed && aspect == SignalAspect::Red {
        return Some(GatePolicyDecision::DenyAndStop);
    }
    let candidate = match (interpretation, aspect) {
        (GateInterpretation::ProtectedGroup, SignalAspect::Green)
        | (GateInterpretation::DirectionalRightProtected, SignalAspect::Green) => {
            Some(GateCandidateKind::Protected)
        }
        (GateInterpretation::PermissiveGroup, SignalAspect::Green)
        | (GateInterpretation::DirectionalRightPermissive, SignalAspect::Green)
        | (GateInterpretation::CnCircularRightTurn, SignalAspect::Red | SignalAspect::Green) => {
            Some(GateCandidateKind::Permissive)
        }
        (GateInterpretation::Uncontrolled, _) => return None,
        _ => None,
    };
    Some(candidate.map_or(
        GatePolicyDecision::DenyAndStop,
        GatePolicyDecision::Candidate,
    ))
}

/// §6.3 的完整稳定候选键。presence 单独编码，合法整数不充当 absent sentinel。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ConflictCandidateOrderKey {
    kind: GateCandidateKind,
    priority: Option<i32>,
    first_eligible_tick: u64,
    waiting_admission_sequence: Option<u64>,
    vehicle_update_sequence: u32,
}

impl ConflictCandidateOrderKey {
    pub(crate) const fn new(
        kind: GateCandidateKind,
        priority: Option<i32>,
        first_eligible_tick: u64,
        waiting_admission_sequence: Option<u64>,
        vehicle_update_sequence: u32,
    ) -> Self {
        Self {
            kind,
            priority,
            first_eligible_tick,
            waiting_admission_sequence,
            vehicle_update_sequence,
        }
    }

    fn tuple(self) -> (u8, u8, core::cmp::Reverse<i32>, u64, u8, u64, u32) {
        (
            u8::from(self.kind != GateCandidateKind::Protected),
            u8::from(self.priority.is_none()),
            core::cmp::Reverse(self.priority.unwrap_or_default()),
            self.first_eligible_tick,
            u8::from(self.waiting_admission_sequence.is_none()),
            self.waiting_admission_sequence.unwrap_or_default(),
            self.vehicle_update_sequence,
        )
    }
}

impl Ord for ConflictCandidateOrderKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.tuple().cmp(&other.tuple())
    }
}

impl PartialOrd for ConflictCandidateOrderKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// 同一 Gate coverage 的实际规则优先级；声明/流遍历顺序不影响最小值。
pub(crate) fn coverage_min_priority(priorities: impl IntoIterator<Item = i32>) -> Option<i32> {
    priorities.into_iter().min()
}

/// 一辆车对一个 exact Gate occurrence 的首次资格时钟。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConflictEligibilityState {
    locator: ConflictPassageOccurrenceLocator,
    first_eligible_tick: u64,
}

impl ConflictEligibilityState {
    #[must_use]
    pub const fn locator(self) -> ConflictPassageOccurrenceLocator {
        self.locator
    }

    #[must_use]
    pub const fn first_eligible_tick(self) -> u64 {
        self.first_eligible_tick
    }

    pub(crate) fn update(
        current: Option<Self>,
        locator: ConflictPassageOccurrenceLocator,
        eligible: bool,
        tick: u64,
    ) -> Option<Self> {
        if !eligible {
            return None;
        }
        Some(match current {
            Some(current) if current.locator == locator => current,
            Some(_) | None => Self {
                locator,
                first_eligible_tick: tick,
            },
        })
    }
}

/// 对一个 passage cell 的保守最早到达证明。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApproachEstimate {
    Unprovable,
    Finite(u64),
    OutsideHorizon,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ApproachOwner {
    vehicle: VehicleHandle,
    vehicle_update_sequence: u32,
    estimate: ApproachEstimate,
}

impl ApproachOwner {
    fn rank(self) -> (u8, u64, u32) {
        match self.estimate {
            ApproachEstimate::Unprovable => (0, 0, self.vehicle_update_sequence),
            ApproachEstimate::Finite(ms) => (1, ms, self.vehicle_update_sequence),
            ApproachEstimate::OutsideHorizon => (2, 0, self.vehicle_update_sequence),
        }
    }
}

/// 每个静态 cell 只保留两个不同 owner，查询时可排除 subject 自身。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ApproachFrontierCell {
    first: Option<ApproachOwner>,
    second: Option<ApproachOwner>,
}

impl ApproachFrontierCell {
    /// 调用方必须先完成 owner-local current/upcoming/repeated 归约。
    pub(crate) fn insert_owner_reduced(
        &mut self,
        vehicle: VehicleHandle,
        vehicle_update_sequence: u32,
        estimate: ApproachEstimate,
    ) {
        if estimate == ApproachEstimate::OutsideHorizon {
            return;
        }
        let incoming = ApproachOwner {
            vehicle,
            vehicle_update_sequence,
            estimate,
        };
        let mut owners = [self.first, self.second, Some(incoming)];
        let mut reduced = [None, None, None];
        let mut len = 0;
        for owner in owners.iter_mut().filter_map(Option::take) {
            if let Some(index) =
                reduced[..len]
                    .iter()
                    .position(|current: &Option<ApproachOwner>| {
                        current.is_some_and(|current| current.vehicle == owner.vehicle)
                    })
            {
                let current = reduced[index].expect("matched owner exists");
                reduced[index] = Some(if owner.rank() < current.rank() {
                    owner
                } else {
                    current
                });
            } else {
                reduced[len] = Some(owner);
                len += 1;
            }
        }
        reduced[..len].sort_unstable_by_key(|owner| owner.expect("dense owners").rank());
        self.first = reduced.first().copied().flatten();
        self.second = reduced.get(1).copied().flatten();
    }

    pub(crate) fn value_excluding(self, subject: VehicleHandle) -> ApproachEstimate {
        self.first
            .filter(|owner| owner.vehicle != subject)
            .or_else(|| self.second.filter(|owner| owner.vehicle != subject))
            .map_or(ApproachEstimate::OutsideHorizon, |owner| owner.estimate)
    }
}

/// ETA 输入只使用已提交整数纵向状态与受检 profile 加速度。
#[derive(Clone, Copy, Debug)]
pub(crate) struct ApproachEtaInput {
    pub(crate) exact_distance_mm: u64,
    pub(crate) carry_um: u16,
    pub(crate) speed_mm_s: u32,
    pub(crate) max_acceleration_m_s2: f32,
    pub(crate) proof_horizon_ms: u64,
}

/// 计算 directed lower-bound ETA；任何无法证明的浮点状态都保守拒绝。
pub(crate) fn approach_eta_lower_bound(input: ApproachEtaInput) -> ApproachEstimate {
    if input.carry_um >= 1_000
        || !input.max_acceleration_m_s2.is_finite()
        || input.max_acceleration_m_s2 < 0.0
    {
        return ApproachEstimate::Unprovable;
    }
    let distance = input.exact_distance_mm as f64;
    if distance >= u64::MAX as f64 || distance as u64 != input.exact_distance_mm {
        return ApproachEstimate::Unprovable;
    }
    let calculated = (|| -> Option<ApproachEstimate> {
        let carry = upper_div(f64::from(input.carry_um), 1_000.0)?;
        let d_lower = lower_sub(distance, carry)?.max(0.0);
        if d_lower == 0.0 {
            return Some(ApproachEstimate::Finite(0));
        }
        let speed = f64::from(input.speed_mm_s);
        let acceleration = upper_mul(f64::from(input.max_acceleration_m_s2), 1_000.0)?;
        let horizon_s = upper_div(input.proof_horizon_ms as f64, 1_000.0)?;
        let reachable = upper_add(
            upper_mul(speed, horizon_s)?,
            upper_mul(
                0.5,
                upper_mul(acceleration, upper_mul(horizon_s, horizon_s)?)?,
            )?,
        )?;
        if reachable < d_lower {
            return Some(ApproachEstimate::OutsideHorizon);
        }
        let eta_s = if acceleration > 0.0 {
            let radicand = upper_add(
                upper_mul(speed, speed)?,
                upper_mul(upper_mul(2.0, acceleration)?, d_lower)?,
            )?;
            let denominator = upper_add(upper_sqrt(radicand)?, speed)?;
            lower_div(lower_mul(2.0, d_lower)?, denominator)?
        } else if speed > 0.0 {
            lower_div(d_lower, speed)?
        } else {
            return Some(ApproachEstimate::OutsideHorizon);
        };
        let millis = lower_mul(eta_s.max(0.0), 1_000.0)?;
        finite_approach_estimate(millis)
    })();
    calculated.unwrap_or(ApproachEstimate::Unprovable)
}

fn finite_approach_estimate(millis: f64) -> Option<ApproachEstimate> {
    if !millis.is_finite() || millis < 0.0 || millis >= u64::MAX as f64 {
        return None;
    }
    Some(ApproachEstimate::Finite(millis.floor() as u64))
}

fn lower_sub(left: f64, right: f64) -> Option<f64> {
    directed(left - right, false)
}
fn lower_mul(left: f64, right: f64) -> Option<f64> {
    directed(left * right, false)
}
fn lower_div(left: f64, right: f64) -> Option<f64> {
    if right <= 0.0 {
        return None;
    }
    directed(left / right, false)
}
fn upper_add(left: f64, right: f64) -> Option<f64> {
    directed(left + right, true)
}
fn upper_mul(left: f64, right: f64) -> Option<f64> {
    directed(left * right, true)
}
fn upper_div(left: f64, right: f64) -> Option<f64> {
    if right <= 0.0 {
        return None;
    }
    directed(left / right, true)
}
fn upper_sqrt(value: f64) -> Option<f64> {
    if value < 0.0 {
        return None;
    }
    directed(value.sqrt(), true)
}
fn directed(value: f64, upper: bool) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    Some(if value == 0.0 {
        value
    } else if upper {
        value.next_up()
    } else {
        value.next_down()
    })
}

/// 已清空 cell 的滞后基准；切换保守基准与真实 clear 不混淆。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictLagReference {
    NoHistory,
    ActualClear(u64),
    CutoverFloor(u64),
}

/// 间隙 normal outcome。lag 相等通过，lead 相等拒绝。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictGapOutcome {
    Accepted,
    LagGap,
    LeadGap,
    ApproachUnprovable,
}

/// exact yield-target cell 的完整检查结果。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictYieldOutcome {
    Accepted,
    Occupied,
    LagGap,
    LeadGap,
    ApproachUnprovable,
}

pub(crate) fn check_gap(
    now_ms: u64,
    reference: ConflictLagReference,
    required_lag_ms: u64,
    approach: ApproachEstimate,
    required_lead_ms: u64,
) -> Option<ConflictGapOutcome> {
    let reference = match reference {
        ConflictLagReference::NoHistory => None,
        ConflictLagReference::ActualClear(at) | ConflictLagReference::CutoverFloor(at) => Some(at),
    };
    if let Some(reference) = reference {
        let elapsed = now_ms.checked_sub(reference)?;
        if elapsed < required_lag_ms {
            return Some(ConflictGapOutcome::LagGap);
        }
    }
    Some(match approach {
        ApproachEstimate::OutsideHorizon => ConflictGapOutcome::Accepted,
        ApproachEstimate::Finite(ms) if ms > required_lead_ms => ConflictGapOutcome::Accepted,
        ApproachEstimate::Finite(_) => ConflictGapOutcome::LeadGap,
        ApproachEstimate::Unprovable => ConflictGapOutcome::ApproachUnprovable,
    })
}

/// 下游物理 claim 的规范半开区间。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DownstreamInterval {
    edge: LaneEdgeOrdinal,
    start_mm: u32,
    end_mm: u32,
}

/// Route occurrence 上带微米余数的规范位置，用于 mandatory downstream proof。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DownstreamRoutePoint {
    route_edge_index: u32,
    progress_mm: u32,
    carry_um: u16,
}

/// downstream claim 的已验证派生计划。
///
/// `raw_interval_capacity` 是合并重复物理边之前的 route occurrence 数。持久化与
/// cutover 调用方先通过各自的资源预算入口预留该容量，再调用无分配填充入口，
/// 从而不让共享语义 helper 隐式改变错误轴。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DownstreamClaimPlan {
    gate_crossed_side: DownstreamRoutePoint,
    target: DownstreamRoutePoint,
    raw_interval_capacity: usize,
}

impl DownstreamClaimPlan {
    #[must_use]
    pub(crate) const fn raw_interval_capacity(self) -> usize {
        self.raw_interval_capacity
    }

    pub(crate) const fn target(self) -> DownstreamRoutePoint {
        self.target
    }
}

impl DownstreamRoutePoint {
    pub(crate) const fn new(
        route_edge_index: u32,
        progress_mm: u32,
        carry_um: u16,
    ) -> Option<Self> {
        if carry_um >= 1_000 {
            return None;
        }
        Some(Self {
            route_edge_index,
            progress_mm,
            carry_um,
        })
    }

    #[must_use]
    pub const fn route_edge_index(self) -> u32 {
        self.route_edge_index
    }
    #[must_use]
    pub const fn progress_mm(self) -> u32 {
        self.progress_mm
    }
    #[must_use]
    pub const fn carry_um(self) -> u16 {
        self.carry_um
    }
}

/// 从 Gate crossed side 到“最远 passage clearance + 实际车长”的物理 claim。
///
/// `storage_upper_bound` 已由 leader、next Gate、RouteEnd、ParkingStop 与 Waiting hard
/// boundary 取最小值；相等通过，提前一微米失败。
pub(crate) fn prove_downstream_clearance(
    route_edges: &[LaneEdgeOrdinal],
    edge_lengths_mm: &[u32],
    gate_crossed_side: DownstreamRoutePoint,
    farthest_clearance: DownstreamRoutePoint,
    vehicle_length_mm: u32,
    storage_upper_bound: DownstreamRoutePoint,
    output: &mut Vec<DownstreamInterval>,
) -> Result<(), ConflictAcquireError> {
    let target = downstream_claim_target(
        route_edges,
        edge_lengths_mm,
        farthest_clearance,
        vehicle_length_mm,
    )?;
    if storage_upper_bound < target {
        return Err(ConflictAcquireError::NoGrant(
            ConflictResourceNoGrant::DownstreamStorageBoundary,
        ));
    }
    derive_downstream_claims(
        route_edges,
        edge_lengths_mm,
        gate_crossed_side,
        target,
        output,
    )
}

/// 从 reservation 级路线证明重建 mandatory downstream 物理资源并集。
///
/// 输出只包含 `(physical edge, start, end)`；循环路线中同一物理边的重叠
/// occurrence 会合并。持久化与切换必须重新调用本函数并精确比较结果，不能
/// 为合并后的物理区间虚构单一来源 occurrence。
pub(crate) fn derive_downstream_claims(
    route_edges: &[LaneEdgeOrdinal],
    edge_lengths_mm: &[u32],
    gate_crossed_side: DownstreamRoutePoint,
    target: DownstreamRoutePoint,
    output: &mut Vec<DownstreamInterval>,
) -> Result<(), ConflictAcquireError> {
    let plan = downstream_claim_plan(gate_crossed_side, target)?;
    reserve_for_len(output, plan.raw_interval_capacity)?;
    derive_downstream_claims_from_plan(route_edges, edge_lengths_mm, plan, output)
}

pub(crate) fn downstream_claim_plan(
    gate_crossed_side: DownstreamRoutePoint,
    target: DownstreamRoutePoint,
) -> Result<DownstreamClaimPlan, ConflictAcquireError> {
    if gate_crossed_side.carry_um != 0 || target.carry_um != 0 {
        return Err(ConflictAcquireError::InvalidBundle);
    }
    if gate_crossed_side > target {
        return Err(ConflictAcquireError::InvalidBundle);
    }
    let start_index = usize::try_from(gate_crossed_side.route_edge_index)
        .map_err(|_| ConflictAcquireError::InvalidBundle)?;
    let target_index = usize::try_from(target.route_edge_index)
        .map_err(|_| ConflictAcquireError::InvalidBundle)?;
    let required = target_index
        .checked_sub(start_index)
        .and_then(|value| value.checked_add(1))
        .ok_or(ConflictAcquireError::InvalidBundle)?;
    Ok(DownstreamClaimPlan {
        gate_crossed_side,
        target,
        raw_interval_capacity: required,
    })
}

/// 使用调用方已经显式预留的 scratch 填充物理区间并集。
pub(crate) fn derive_downstream_claims_from_plan(
    route_edges: &[LaneEdgeOrdinal],
    edge_lengths_mm: &[u32],
    plan: DownstreamClaimPlan,
    output: &mut Vec<DownstreamInterval>,
) -> Result<(), ConflictAcquireError> {
    if output.capacity() < plan.raw_interval_capacity {
        return Err(ConflictAcquireError::Capacity);
    }
    output.clear();
    let gate_crossed_side = plan.gate_crossed_side;
    let target = plan.target;
    let start_index = usize::try_from(gate_crossed_side.route_edge_index)
        .map_err(|_| ConflictAcquireError::InvalidBundle)?;
    let target_index = usize::try_from(target.route_edge_index)
        .map_err(|_| ConflictAcquireError::InvalidBundle)?;
    for index in start_index..=target_index {
        let edge = *route_edges
            .get(index)
            .ok_or(ConflictAcquireError::InvalidBundle)?;
        let length = *edge_lengths_mm
            .get(edge.index())
            .ok_or(ConflictAcquireError::InvalidBundle)?;
        let start = if index == start_index {
            gate_crossed_side.progress_mm
        } else {
            0
        };
        let end = if index == target_index {
            target.progress_mm
        } else {
            length
        };
        if start > length || end > length || start > end {
            return Err(ConflictAcquireError::InvalidBundle);
        }
        if let Some(interval) = DownstreamInterval::new(edge, start, end) {
            output.push(interval);
        }
    }
    output.sort_unstable();
    let mut write_index = 0;
    for read_index in 0..output.len() {
        let interval = output[read_index];
        if write_index != 0
            && output[write_index - 1].edge == interval.edge
            && interval.start_mm <= output[write_index - 1].end_mm
        {
            output[write_index - 1].end_mm = output[write_index - 1].end_mm.max(interval.end_mm);
        } else {
            output[write_index] = interval;
            write_index += 1;
        }
    }
    output.truncate(write_index);
    Ok(())
}

pub(crate) fn downstream_claim_target(
    route_edges: &[LaneEdgeOrdinal],
    edge_lengths_mm: &[u32],
    farthest_clearance: DownstreamRoutePoint,
    vehicle_length_mm: u32,
) -> Result<DownstreamRoutePoint, ConflictAcquireError> {
    if farthest_clearance.carry_um != 0 {
        return Err(ConflictAcquireError::InvalidBundle);
    }
    advance_route_point(
        route_edges,
        edge_lengths_mm,
        farthest_clearance,
        vehicle_length_mm,
    )
    .ok_or(ConflictAcquireError::NoGrant(
        ConflictResourceNoGrant::DownstreamStorageBoundary,
    ))
}

fn advance_route_point(
    route_edges: &[LaneEdgeOrdinal],
    edge_lengths_mm: &[u32],
    mut point: DownstreamRoutePoint,
    mut distance_mm: u32,
) -> Option<DownstreamRoutePoint> {
    if point.carry_um != 0 {
        return None;
    }
    let mut index = usize::try_from(point.route_edge_index).ok()?;
    loop {
        let edge = *route_edges.get(index)?;
        let length = *edge_lengths_mm.get(edge.index())?;
        if point.progress_mm > length {
            return None;
        }
        let available = length - point.progress_mm;
        if distance_mm < available || (distance_mm == available && index + 1 == route_edges.len()) {
            point.progress_mm = point.progress_mm.checked_add(distance_mm)?;
            return Some(point);
        }
        distance_mm -= available;
        index = index.checked_add(1)?;
        route_edges.get(index)?;
        point.route_edge_index = u32::try_from(index).ok()?;
        point.progress_mm = 0;
        if distance_mm == 0 {
            return Some(point);
        }
    }
}

impl DownstreamInterval {
    pub(crate) const fn new(edge: LaneEdgeOrdinal, start_mm: u32, end_mm: u32) -> Option<Self> {
        if start_mm >= end_mm {
            return None;
        }
        Some(Self {
            edge,
            start_mm,
            end_mm,
        })
    }

    #[must_use]
    pub const fn edge(self) -> LaneEdgeOrdinal {
        self.edge
    }
    #[must_use]
    pub const fn start_mm(self) -> u32 {
        self.start_mm
    }
    #[must_use]
    pub const fn end_mm(self) -> u32 {
        self.end_mm
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OwnedDownstreamClaim {
    owner: VehicleHandle,
    follower_min_gap_mm: u32,
    interval: DownstreamInterval,
    serial: u64,
}

/// snapshot/cutover 边界只读取的已提交 downstream claim。
///
/// `serial` 是仲裁器内部连接值，不通过此类型离开进程。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PersistedDownstreamClaim {
    pub(crate) follower_min_gap_mm: u32,
    pub(crate) interval: DownstreamInterval,
}

/// 冷路径为每个 live vehicle 建立的 committed Conflict 直接索引。
///
/// 调用方负责按 `vehicle_capacity` 预留这张临时表，使 snapshot/cutover 的
/// 分配失败仍归入各自错误轴；arbiter 只在线性扫描中填充并验证它。
#[derive(Clone, Copy, Debug)]
pub(crate) struct CommittedConflictIndexEntry {
    reservation: ConflictReservation,
    downstream_start: usize,
    downstream_count: usize,
}

/// 一份 committed reservation 及其连续 downstream claims 的只读视图。
pub(crate) struct PersistedConflictAuthority<'a> {
    pub(crate) reservation: ConflictReservation,
    downstream: &'a [OwnedDownstreamClaim],
}

impl PersistedConflictAuthority<'_> {
    pub(crate) fn downstream_claims(
        &self,
    ) -> impl ExactSizeIterator<Item = PersistedDownstreamClaim> + '_ {
        self.downstream
            .iter()
            .map(|claim| PersistedDownstreamClaim {
                follower_min_gap_mm: claim.follower_min_gap_mm,
                interval: claim.interval,
            })
    }
}

/// snapshot/cutover 共用的 committed Conflict 批量视图。
///
/// 构建成本为 `O(vehicle_capacity + reservations + downstream_claims)`；随后按
/// `VehicleHandle` 读取 reservation 与 claims 都是 `O(1 + owner_claims)`。
pub(crate) struct ConflictPersistenceView<'a> {
    downstream: &'a [OwnedDownstreamClaim],
    index: &'a [Option<CommittedConflictIndexEntry>],
}

impl ConflictPersistenceView<'_> {
    pub(crate) fn authority(&self, owner: VehicleHandle) -> Option<PersistedConflictAuthority<'_>> {
        let entry = self.index.get(owner.index() as usize).copied().flatten()?;
        if entry.reservation.owner != owner {
            return None;
        }
        let end = entry.downstream_start.checked_add(entry.downstream_count)?;
        let downstream = self.downstream.get(entry.downstream_start..end)?;
        Some(PersistedConflictAuthority {
            reservation: entry.reservation,
            downstream,
        })
    }
}

/// restore/cutover 已由车辆位姿与 passage 锚点重建的 cell 阶段。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RestoredConflictCell {
    pub(crate) address: ConflictPassageAddress,
    pub(crate) occupant: bool,
    pub(crate) cleared: bool,
}

/// restore/cutover 在未发布 staging world 中重建的一份完整 reservation 输入。
pub(crate) struct RestoredConflictReservation<'a> {
    pub(crate) follower_min_gap_mm: u32,
    pub(crate) acquired_tick: u64,
    pub(crate) passage_range: ConflictPassageRange,
    pub(crate) cells: &'a [RestoredConflictCell],
    pub(crate) downstream: &'a [DownstreamInterval],
}

/// 组合资源 preflight 的 normal no-grant 原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictResourceNoGrant {
    WaitingCycle,
    ConflictOccupied,
    DownstreamStorageBoundary,
    DownstreamClaimConflict,
}

/// 非 normal 的 bundle 拒绝；调用方必须把 invariant/capacity 映射成零提交 step error。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConflictAcquireError {
    NoGrant(ConflictResourceNoGrant),
    InvalidBundle,
    Capacity,
    ScratchAllocFailed,
}

/// 冲突仲裁器冷安装失败；与运行期 bundle 拒绝分开，避免丢失宿主可采取的补救语义。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConflictInstallError {
    InvalidNetwork,
    CapacityOverflow,
    AllocationFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ConflictCellAuthority {
    frontier: ApproachFrontierCell,
    zone_committed_owner: Option<VehicleHandle>,
    zone_staged_owner: Option<VehicleHandle>,
    reservation: Option<VehicleHandle>,
    reservation_serial: Option<u64>,
    occupant: Option<VehicleHandle>,
    cleared: bool,
    lag: ConflictLagReference,
}

impl Default for ConflictCellAuthority {
    fn default() -> Self {
        Self {
            frontier: ApproachFrontierCell::default(),
            zone_committed_owner: None,
            zone_staged_owner: None,
            reservation: None,
            reservation_serial: None,
            occupant: None,
            cleared: false,
            lag: ConflictLagReference::NoHistory,
        }
    }
}

/// crossing 后保留到全部 passage 车尾清空的明确状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConflictPassageRange {
    route: RouteHandle,
    maneuver_occurrence_index: u32,
    admission_gate_hop: u32,
    first_conflict_occurrence_index: u32,
    passage_count: u32,
}

impl ConflictPassageRange {
    pub(crate) const fn new(
        route: RouteHandle,
        maneuver_occurrence_index: u32,
        admission_gate_hop: u32,
        first_conflict_occurrence_index: u32,
        passage_count: u32,
    ) -> Option<Self> {
        if passage_count == 0
            || first_conflict_occurrence_index
                .checked_add(passage_count)
                .is_none()
        {
            return None;
        }
        Some(Self {
            route,
            maneuver_occurrence_index,
            admission_gate_hop,
            first_conflict_occurrence_index,
            passage_count,
        })
    }

    #[must_use]
    pub const fn route(self) -> RouteHandle {
        self.route
    }
    #[must_use]
    pub const fn maneuver_occurrence_index(self) -> u32 {
        self.maneuver_occurrence_index
    }
    #[must_use]
    pub const fn admission_gate_hop(self) -> u32 {
        self.admission_gate_hop
    }
    #[must_use]
    pub const fn first_conflict_occurrence_index(self) -> u32 {
        self.first_conflict_occurrence_index
    }
    #[must_use]
    pub const fn passage_count(self) -> u32 {
        self.passage_count
    }
}

/// crossing 后保留到全部 passage 车尾清空的明确状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConflictReservation {
    owner: VehicleHandle,
    passage_range: ConflictPassageRange,
    downstream_owner: VehicleHandle,
    downstream_claim_count: u32,
    acquired_tick: u64,
    claim_serial: u64,
}

impl ConflictReservation {
    #[must_use]
    pub const fn owner(self) -> VehicleHandle {
        self.owner
    }
    #[must_use]
    pub const fn route(self) -> RouteHandle {
        self.passage_range.route()
    }
    #[must_use]
    pub const fn maneuver_occurrence_index(self) -> u32 {
        self.passage_range.maneuver_occurrence_index()
    }
    #[must_use]
    pub const fn admission_gate_hop(self) -> u32 {
        self.passage_range.admission_gate_hop()
    }
    #[must_use]
    pub const fn passage_range(self) -> ConflictPassageRange {
        self.passage_range
    }
    #[must_use]
    pub const fn downstream_owner(self) -> VehicleHandle {
        self.downstream_owner
    }
    #[must_use]
    pub const fn downstream_claim_count(self) -> u32 {
        self.downstream_claim_count
    }
    #[must_use]
    pub const fn acquired_tick(self) -> u64 {
        self.acquired_tick
    }
}

pub(crate) struct ConflictGrant {
    owner: VehicleHandle,
    serial: u64,
    tick: u64,
    waiting_zone: Option<WaitingZoneOrdinal>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StagedGrant {
    owner: VehicleHandle,
    serial: u64,
    consumed: bool,
}

/// 只为实际持有 staged/committed authority 的车辆保留的稀疏索引。
///
/// vehicle slot 索引定位紧凑 owner 行，世代核对阻止槽位复用继承旧授权。
/// 完整 row/cell 对应关系仍由线性 aggregate 校验复核。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ConflictOwnerAuthority {
    owner: VehicleHandle,
    staged_serial: Option<u64>,
    staged_cell_count: usize,
    staged_downstream_claim_count: usize,
    reservation: Option<ConflictReservation>,
    committed_cell_count: usize,
    committed_downstream_claim_count: usize,
    staged_cell_start: usize,
    staged_downstream_start: usize,
    staged_grant_index: usize,
    committed_cell_start: usize,
    committed_downstream_start: usize,
    uncleared_cell_count: usize,
    pending_commit: bool,
}

impl ConflictOwnerAuthority {
    const fn staged(
        owner: VehicleHandle,
        serial: u64,
        cell_count: usize,
        downstream_claim_count: usize,
    ) -> Self {
        Self {
            owner,
            staged_serial: Some(serial),
            staged_cell_count: cell_count,
            staged_downstream_claim_count: downstream_claim_count,
            reservation: None,
            committed_cell_count: 0,
            committed_downstream_claim_count: 0,
            staged_cell_start: 0,
            staged_downstream_start: 0,
            staged_grant_index: 0,
            committed_cell_start: 0,
            committed_downstream_start: 0,
            uncleared_cell_count: 0,
            pending_commit: false,
        }
    }

    const fn committed(
        reservation: ConflictReservation,
        cell_count: usize,
        downstream_claim_count: usize,
    ) -> Self {
        Self {
            owner: reservation.owner,
            staged_serial: None,
            staged_cell_count: 0,
            staged_downstream_claim_count: 0,
            reservation: Some(reservation),
            committed_cell_count: cell_count,
            committed_downstream_claim_count: downstream_claim_count,
            staged_cell_start: 0,
            staged_downstream_start: 0,
            staged_grant_index: 0,
            committed_cell_start: 0,
            committed_downstream_start: 0,
            uncleared_cell_count: cell_count,
            pending_commit: true,
        }
    }

    const fn consumed(owner: VehicleHandle) -> Self {
        Self {
            owner,
            staged_serial: None,
            staged_cell_count: 0,
            staged_downstream_claim_count: 0,
            reservation: None,
            committed_cell_count: 0,
            committed_downstream_claim_count: 0,
            staged_cell_start: 0,
            staged_downstream_start: 0,
            staged_grant_index: 0,
            committed_cell_start: 0,
            committed_downstream_start: 0,
            uncleared_cell_count: 0,
            pending_commit: false,
        }
    }

    const fn has_authority(self) -> bool {
        self.staged_serial.is_some() || self.reservation.is_some()
    }
}

pub(crate) struct ConflictCrossingCommit {
    pub(crate) reservation: ConflictReservation,
    pub(crate) waiting_admission: Option<WaitingZoneOrdinal>,
}

struct ConflictCrossingPreflight {
    cells: usize,
    downstream_claims: usize,
    downstream_claim_count: u32,
    authority_index: usize,
    entered_index: Option<usize>,
}

struct PureWaitingGrantPreflight {
    waiting_zone: WaitingZoneOrdinal,
    staged_index: usize,
    authority_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConflictPassageStage {
    Reserved,
    Occupied,
    Cleared,
}

impl ConflictPassageStage {
    pub(crate) const fn journal_tag(self) -> u8 {
        match self {
            Self::Reserved => 0,
            Self::Occupied => 1,
            Self::Cleared => 2,
        }
    }

    pub(crate) const fn from_journal_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::Reserved),
            1 => Some(Self::Occupied),
            2 => Some(Self::Cleared),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConflictClearOutcome {
    Retained,
    ReservationReleased,
}

/// Waiting reducer 预演后签发的 tick-local 资格；字段私有，宿主无法伪造。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WaitingAdmissionEntitlement {
    owner: VehicleHandle,
    zone: WaitingZoneOrdinal,
    tick: u64,
}

impl WaitingAdmissionEntitlement {
    pub(crate) const fn new(owner: VehicleHandle, zone: WaitingZoneOrdinal, tick: u64) -> Self {
        Self { owner, zone, tick }
    }
}

/// 候选所需的完整 Conflict/downstream 资源；声明顺序不承载 winner 语义。
pub(crate) struct GrantResourceBundle<'a> {
    pub(crate) owner: VehicleHandle,
    pub(crate) follower_min_gap_mm: u32,
    pub(crate) cells: &'a [ConflictPassageAddress],
    pub(crate) downstream: &'a [DownstreamInterval],
    pub(crate) waiting_entitlement: Option<WaitingAdmissionEntitlement>,
}

/// Conflict 与 downstream 的唯一 mutation owner。
pub(crate) struct ConflictArbiter {
    addresses: Box<[ConflictPassageAddress]>,
    cells: Vec<ConflictCellAuthority>,
    staged_cells: Vec<(usize, VehicleHandle, u64)>,
    committed_cells: Vec<(usize, VehicleHandle, u64)>,
    scratch_cell_indices: Vec<usize>,
    staged_downstream: Vec<OwnedDownstreamClaim>,
    committed_downstream: Vec<OwnedDownstreamClaim>,
    staged_grants: Vec<StagedGrant>,
    owner_authorities: Vec<ConflictOwnerAuthority>,
    owner_lookup: Vec<Option<std::num::NonZeroU32>>,
    downstream_index: crate::downstream_index::DownstreamIndex,
    downstream_index_dirty: bool,
    next_serial: u64,
    conflict_capacity: usize,
    vehicle_capacity: usize,
}

impl ConflictArbiter {
    pub(crate) fn install(
        revision: &laneflow_static_network::SharedNetworkRevision,
        vehicle_capacity: usize,
    ) -> Result<Self, ConflictInstallError> {
        let stream_count = revision
            .traffic()
            .entity_counts()
            .count(laneflow_static_contract::EntityKind::ParticipantStream);
        let mut address_count = 0_usize;
        for raw in 0..stream_count {
            let stream = ParticipantStreamOrdinal::from_raw(raw);
            let passages = revision
                .conflict()
                .participant_stream(stream)
                .ok_or(ConflictInstallError::InvalidNetwork)?
                .passages();
            address_count = address_count
                .checked_add(passages.len())
                .ok_or(ConflictInstallError::CapacityOverflow)?;
        }
        let mut addresses = Vec::new();
        addresses
            .try_reserve_exact(address_count)
            .map_err(|_| ConflictInstallError::AllocationFailed)?;
        for raw in 0..stream_count {
            let stream = ParticipantStreamOrdinal::from_raw(raw);
            for (passage_local_index, passage) in revision
                .conflict()
                .participant_stream(stream)
                .ok_or(ConflictInstallError::InvalidNetwork)?
                .passages()
                .iter()
                .enumerate()
            {
                addresses.push(ConflictPassageAddress::new(
                    passage.conflict_zone(),
                    stream,
                    u32::try_from(passage_local_index)
                        .map_err(|_| ConflictInstallError::CapacityOverflow)?,
                ));
            }
        }
        addresses.sort_unstable();
        if addresses.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ConflictInstallError::InvalidNetwork);
        }
        Ok(Self::from_sorted_unique_addresses(
            addresses,
            vehicle_capacity,
        ))
    }

    pub(crate) fn new(
        mut addresses: Vec<ConflictPassageAddress>,
        vehicle_capacity: usize,
    ) -> Result<Self, ConflictAcquireError> {
        addresses.sort_unstable();
        if addresses.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ConflictAcquireError::InvalidBundle);
        }
        Ok(Self::from_sorted_unique_addresses(
            addresses,
            vehicle_capacity,
        ))
    }

    fn from_sorted_unique_addresses(
        addresses: Vec<ConflictPassageAddress>,
        vehicle_capacity: usize,
    ) -> Self {
        let conflict_capacity = addresses.len();
        let addresses = addresses.into_boxed_slice();
        let cells = Vec::new();
        let staged_cells = Vec::new();
        let scratch_cell_indices = Vec::new();
        let committed_cells = Vec::new();
        let staged_downstream = Vec::new();
        let committed_downstream = Vec::new();
        let staged_grants = Vec::new();
        let owner_authorities = Vec::new();
        Self {
            addresses,
            cells,
            staged_cells,
            committed_cells,
            scratch_cell_indices,
            staged_downstream,
            committed_downstream,
            staged_grants,
            owner_authorities,
            owner_lookup: Vec::new(),
            downstream_index: crate::downstream_index::DownstreamIndex::default(),
            downstream_index_dirty: true,
            next_serial: 0,
            conflict_capacity,
            vehicle_capacity,
        }
    }

    pub(crate) fn lag_reference(
        &self,
        address: ConflictPassageAddress,
    ) -> Option<ConflictLagReference> {
        let index = self.cell_index(address).ok()?;
        Some(
            self.cells
                .get(index)
                .map_or(ConflictLagReference::NoHistory, |cell| cell.lag),
        )
    }

    /// 按当前根的规范 address 序返回非 `NoHistory` 行。调用方在
    /// 持久化前将 address 解析为稳定 locator 并按 locator 字节重排。
    pub(crate) fn persisted_lag_rows(
        &self,
    ) -> impl Iterator<Item = (ConflictPassageAddress, ConflictLagReference)> + '_ {
        self.addresses
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, address)| {
                let reference = self
                    .cells
                    .get(index)
                    .map_or(ConflictLagReference::NoHistory, |cell| cell.lag);
                (reference != ConflictLagReference::NoHistory).then_some((address, reference))
            })
    }

    /// 按规范 address 序返回迁移所需的 cell 权威。reservation 标记直接来自
    /// arbiter 单一事实源，调用方不再为每个 cell 扫描车辆表。
    pub(crate) fn migration_rows(
        &self,
    ) -> impl Iterator<Item = (ConflictPassageAddress, ConflictLagReference, bool)> + '_ {
        self.addresses
            .iter()
            .copied()
            .enumerate()
            .map(|(index, address)| {
                let cell = self.cells.get(index).copied().unwrap_or_default();
                (address, cell.lag, cell.reservation.is_some())
            })
    }

    /// 以一次线性扫描建立 snapshot/cutover 使用的 committed authority 视图。
    ///
    /// `committed_downstream` 的同一 reservation claims 在 commit 时连续追加，
    /// release 只稳定 retain，因此这里同时验证连续性、计数和 serial 归属。
    pub(crate) fn persistence_view<'a>(
        &'a self,
        index: &'a mut [Option<CommittedConflictIndexEntry>],
    ) -> Option<ConflictPersistenceView<'a>> {
        if index.len() != self.vehicle_capacity {
            return None;
        }
        index.fill(None);
        let mut downstream_start = 0_usize;
        let mut owners = 0;
        while let Some(first_claim) = self.committed_downstream.get(downstream_start) {
            let reservation = self
                .owner_authority(first_claim.owner)?
                .reservation
                .as_ref()?;
            owners += 1;
            let owner_index = reservation.owner.index() as usize;
            let downstream_count = usize::try_from(reservation.downstream_claim_count).ok()?;
            let downstream_end = downstream_start.checked_add(downstream_count)?;
            let claims = self
                .committed_downstream
                .get(downstream_start..downstream_end)?;
            if claims.iter().any(|claim| {
                claim.owner != reservation.owner || claim.serial != reservation.claim_serial
            }) {
                return None;
            }
            let slot = index.get_mut(owner_index)?;
            if slot.is_some() {
                return None;
            }
            *slot = Some(CommittedConflictIndexEntry {
                reservation: *reservation,
                downstream_start,
                downstream_count,
            });
            downstream_start = downstream_end;
        }
        if downstream_start != self.committed_downstream.len()
            || owners
                != self
                    .owner_authorities
                    .iter()
                    .filter(|owner| owner.reservation.is_some())
                    .count()
        {
            return None;
        }
        Some(ConflictPersistenceView {
            downstream: &self.committed_downstream,
            index,
        })
    }

    /// 在未发布候选世界中安装一个已验证 reservation。全部预留与
    /// bundle 检查复用正常单写者路径；只有 occupancy/cleared 是根据已验证
    /// 车身 footprint 在 commit 后设置的恢复态。
    pub(crate) fn restore_reservation(
        &mut self,
        owner: VehicleHandle,
        restored: RestoredConflictReservation<'_>,
    ) -> Result<ConflictReservation, ConflictAcquireError> {
        let RestoredConflictReservation {
            follower_min_gap_mm,
            acquired_tick,
            passage_range,
            cells,
            downstream,
        } = restored;
        if cells.is_empty()
            || downstream.is_empty()
            || cells.iter().all(|cell| cell.cleared)
            || cells
                .windows(2)
                .any(|pair| pair[0].address >= pair[1].address)
            || cells.iter().any(|cell| cell.occupant && cell.cleared)
        {
            return Err(ConflictAcquireError::InvalidBundle);
        }
        let mut addresses = Vec::new();
        reserve_for_len(&mut addresses, cells.len())?;
        addresses.extend(cells.iter().map(|cell| cell.address));
        let grant = self.try_acquire(
            acquired_tick,
            GrantResourceBundle {
                owner,
                follower_min_gap_mm,
                cells: &addresses,
                downstream,
                waiting_entitlement: None,
            },
        )?;
        let entered = cells
            .iter()
            .find(|cell| cell.occupant)
            .or_else(|| cells.iter().find(|cell| !cell.cleared))
            .expect("validated reservation retains one uncleared cell")
            .address;
        let reservation = self
            .commit_crossing(grant, passage_range, entered)?
            .reservation;
        for restored in cells {
            let index = self.cell_index(restored.address)?;
            let cell = &mut self.cells[index];
            cell.occupant = restored.occupant.then_some(owner);
            cell.cleared = restored.cleared;
        }
        let index = self.owner_authority_index(owner).expect("restored owner");
        self.owner_authorities[index].uncleared_cell_count =
            cells.iter().filter(|cell| !cell.cleared).count();
        self.expire_unconsumed_grants();
        Ok(reservation)
    }

    pub(crate) fn restore_lag_reference(
        &mut self,
        address: ConflictPassageAddress,
        reference: ConflictLagReference,
    ) -> Result<(), ConflictAcquireError> {
        if reference == ConflictLagReference::NoHistory {
            return Err(ConflictAcquireError::InvalidBundle);
        }
        let index = self.cell_index(address)?;
        self.ensure_cells()?;
        self.cells[index].lag = reference;
        Ok(())
    }

    pub(crate) fn cell_count(&self) -> usize {
        self.addresses.len()
    }

    pub(crate) fn addresses(&self) -> impl Iterator<Item = ConflictPassageAddress> + '_ {
        self.addresses.iter().copied()
    }

    pub(crate) fn contains_address(&self, address: ConflictPassageAddress) -> bool {
        self.cell_index(address).is_ok()
    }

    /// 在已排序的规范地址表中解析唯一 `(zone, stream)` locator。
    ///
    /// 同一 stream 对同一 zone 存在多个 passage 时保持失败关闭；无需为 restore/
    /// cutover 另建常驻或临时索引。
    pub(crate) fn unique_address(
        &self,
        zone: ConflictZoneOrdinal,
        stream: ParticipantStreamOrdinal,
    ) -> Option<ConflictPassageAddress> {
        let key = (zone, stream);
        let start = self
            .addresses
            .partition_point(|address| (address.zone, address.stream) < key);
        let address = *self.addresses.get(start)?;
        if (address.zone, address.stream) != key
            || self
                .addresses
                .get(start + 1)
                .is_some_and(|next| (next.zone, next.stream) == key)
        {
            return None;
        }
        Some(address)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.staged_cells.is_empty()
            && self.committed_cells.is_empty()
            && self.staged_downstream.is_empty()
            && self.committed_downstream.is_empty()
            && self.staged_grants.is_empty()
            && self.owner_authorities.is_empty()
            && self.cells.iter().all(|cell| {
                cell.reservation.is_none()
                    && cell.zone_committed_owner.is_none()
                    && cell.zone_staged_owner.is_none()
                    && cell.reservation_serial.is_none()
                    && cell.occupant.is_none()
                    && !cell.cleared
                    && cell.lag == ConflictLagReference::NoHistory
            })
    }

    pub(crate) fn has_authority(&self, owner: VehicleHandle) -> bool {
        self.owner_authority(owner)
            .is_some_and(|authority| authority.has_authority())
    }

    pub(crate) fn clear_approach_frontier(&mut self) {
        for cell in &mut self.cells {
            cell.frontier = ApproachFrontierCell::default();
        }
    }

    pub(crate) fn insert_approach_owner_reduced(
        &mut self,
        address: ConflictPassageAddress,
        vehicle: VehicleHandle,
        vehicle_update_sequence: u32,
        estimate: ApproachEstimate,
    ) -> Result<(), ConflictAcquireError> {
        let index = self.cell_index(address)?;
        self.ensure_cells()?;
        self.cells[index]
            .frontier
            .insert_owner_reduced(vehicle, vehicle_update_sequence, estimate);
        Ok(())
    }

    pub(crate) fn reservation_has_cell(
        &self,
        owner: VehicleHandle,
        address: ConflictPassageAddress,
    ) -> bool {
        self.cell_index(address)
            .ok()
            .and_then(|index| self.cells.get(index))
            .is_some_and(|cell| cell.reservation == Some(owner))
    }

    pub(crate) fn passage_stage(
        &self,
        owner: VehicleHandle,
        address: ConflictPassageAddress,
    ) -> Option<ConflictPassageStage> {
        let cell = self.cells.get(self.cell_index(address).ok()?)?;
        if cell.reservation != Some(owner) {
            return None;
        }
        Some(if cell.cleared {
            ConflictPassageStage::Cleared
        } else if cell.occupant == Some(owner) {
            ConflictPassageStage::Occupied
        } else {
            ConflictPassageStage::Reserved
        })
    }

    pub(crate) fn reservation(&self, owner: VehicleHandle) -> Option<ConflictReservation> {
        self.owner_authority(owner)?.reservation
    }

    pub(crate) fn state_valid(&self, state: &crate::VehicleState) -> bool {
        let Some(traversal) = state.maneuver_traversal else {
            return !self.has_authority(state.handle);
        };
        let crate::ManeuverTraversalPhase::Clearing { admission_gate_hop } = traversal.phase else {
            return !self.has_authority(state.handle);
        };
        let Some(authority) = self.owner_authority(state.handle) else {
            return false;
        };
        let Some(reservation) = authority.reservation else {
            return false;
        };
        if state.status != crate::VehicleStatus::Active
            || state.waiting_membership.is_some()
            || traversal.route != state.route
            || traversal.maneuver_occurrence_index != reservation.maneuver_occurrence_index()
            || admission_gate_hop != reservation.admission_gate_hop()
            || reservation.owner != state.handle
            || reservation.route() != state.route
            || reservation.downstream_owner != state.handle
            || authority.staged_serial.is_some()
            || authority.staged_cell_count != 0
            || authority.staged_downstream_claim_count != 0
        {
            return false;
        }
        authority.committed_cell_count != 0
            && u32::try_from(authority.committed_cell_count).ok()
                == Some(reservation.passage_range.passage_count)
            && u32::try_from(authority.committed_downstream_claim_count).ok()
                == Some(reservation.downstream_claim_count)
    }

    pub(crate) fn authority_owners_valid(
        &self,
        mut owner_valid: impl FnMut(VehicleHandle) -> bool,
        fixed_delta_time_ms: u64,
    ) -> bool {
        let staged_cell_count = self
            .owner_authorities
            .iter()
            .map(|authority| authority.staged_cell_count)
            .sum::<usize>();
        let committed_cell_count = self
            .owner_authorities
            .iter()
            .map(|authority| authority.committed_cell_count)
            .sum::<usize>();
        let staged_downstream_claim_count = self
            .owner_authorities
            .iter()
            .map(|authority| authority.staged_downstream_claim_count)
            .sum::<usize>();
        let committed_downstream_claim_count = self
            .owner_authorities
            .iter()
            .map(|authority| authority.committed_downstream_claim_count)
            .sum::<usize>();
        if staged_cell_count != self.staged_cells.len()
            || committed_cell_count != self.committed_cells.len()
            || staged_downstream_claim_count != self.staged_downstream.len()
            || committed_downstream_claim_count != self.committed_downstream.len()
            || self
                .owner_authorities
                .iter()
                .filter(|authority| authority.staged_serial.is_some())
                .count()
                != self
                    .staged_grants
                    .iter()
                    .filter(|grant| !grant.consumed)
                    .count()
        {
            return false;
        }
        self.owner_authorities
            .iter()
            .enumerate()
            .all(|(index, authority)| {
                owner_valid(authority.owner)
                    && self.owner_authority_index(authority.owner) == Ok(index)
                    && !authority.pending_commit
                    && self.owner_ranges_valid(*authority)
                    && (!(authority.staged_serial.is_some() && authority.reservation.is_some()))
                    && (authority.has_authority()
                        || (authority.staged_cell_count == 0
                            && authority.staged_downstream_claim_count == 0
                            && authority.committed_cell_count == 0
                            && authority.committed_downstream_claim_count == 0))
                    && authority.reservation.is_none_or(|reservation| {
                        reservation.owner == authority.owner
                            && reservation.downstream_owner == authority.owner
                            && authority.committed_cell_count != 0
                            && authority.committed_downstream_claim_count != 0
                    })
            })
            && self.staged_cells.iter().all(|(_, owner, serial)| {
                self.owner_authority(*owner)
                    .is_some_and(|authority| authority.staged_serial == Some(*serial))
            })
            && self.committed_cells.iter().all(|(index, owner, serial)| {
                self.owner_authority(*owner).is_some_and(|authority| {
                    authority.reservation.is_some_and(|reservation| {
                        reservation.claim_serial == *serial
                            && reservation
                                .acquired_tick
                                .checked_mul(fixed_delta_time_ms)
                                .is_some_and(|acquired_time_ms| {
                                    self.cells.get(*index).is_some_and(|cell| {
                                        cell.reservation == Some(*owner)
                                            && cell.reservation_serial == Some(*serial)
                                            && (!cell.cleared || cell.occupant.is_none())
                                            && (!cell.cleared
                                                || matches!(
                                                    cell.lag,
                                                    ConflictLagReference::ActualClear(time)
                                                        if time >= acquired_time_ms
                                                ))
                                    })
                                })
                    })
                })
            })
            && self.staged_downstream.iter().all(|claim| {
                self.owner_authority(claim.owner)
                    .is_some_and(|authority| authority.staged_serial == Some(claim.serial))
            })
            && self.committed_downstream.iter().all(|claim| {
                self.owner_authority(claim.owner).is_some_and(|authority| {
                    authority
                        .reservation
                        .is_some_and(|reservation| reservation.claim_serial == claim.serial)
                })
            })
            && self.staged_grants.iter().all(|grant| {
                grant.consumed
                    || self
                        .owner_authority(grant.owner)
                        .is_some_and(|authority| authority.staged_serial == Some(grant.serial))
            })
            && self
                .cells
                .iter()
                .filter(|cell| cell.reservation.is_some())
                .count()
                == self.committed_cells.len()
            && self.cells.iter().all(|cell| {
                cell.reservation.is_none_or(&mut owner_valid)
                    && cell.occupant.is_none_or(&mut owner_valid)
                    && (cell.reservation.is_some() == cell.reservation_serial.is_some())
                    && (!cell.cleared || cell.reservation.is_some())
                    && (!cell.cleared || matches!(cell.lag, ConflictLagReference::ActualClear(_)))
                    && cell
                        .occupant
                        .is_none_or(|owner| cell.reservation == Some(owner) && !cell.cleared)
            })
    }

    fn owner_ranges_valid(&self, authority: ConflictOwnerAuthority) -> bool {
        let cells = self.committed_cells.get(
            authority.committed_cell_start
                ..authority.committed_cell_start + authority.committed_cell_count,
        );
        let claims = self.committed_downstream.get(
            authority.committed_downstream_start
                ..authority.committed_downstream_start + authority.committed_downstream_claim_count,
        );
        if authority.reservation.is_some() {
            let Some(cells) = cells else {
                return false;
            };
            let Some(claims) = claims else {
                return false;
            };
            if cells.iter().any(|row| row.1 != authority.owner)
                || claims.iter().any(|row| row.owner != authority.owner)
                || cells
                    .iter()
                    .filter(|row| !self.cells[row.0].cleared)
                    .count()
                    != authority.uncleared_cell_count
                || authority.uncleared_cell_count == 0
            {
                return false;
            }
            if cells.iter().any(|row| {
                self.cells[self.zone_index(self.addresses[row.0].zone)].zone_committed_owner
                    != Some(authority.owner)
            }) {
                return false;
            }
        }
        if authority.staged_serial.is_some() {
            let cells = self.staged_cells.get(
                authority.staged_cell_start
                    ..authority.staged_cell_start + authority.staged_cell_count,
            );
            let claims = self.staged_downstream.get(
                authority.staged_downstream_start
                    ..authority.staged_downstream_start + authority.staged_downstream_claim_count,
            );
            if cells.is_none_or(|cells| cells.iter().any(|row| row.1 != authority.owner))
                || claims.is_none_or(|claims| claims.iter().any(|row| row.owner != authority.owner))
                || self
                    .staged_grants
                    .get(authority.staged_grant_index)
                    .is_none_or(|grant| grant.owner != authority.owner || grant.consumed)
            {
                return false;
            }
        }
        true
    }

    pub(crate) fn evaluate_yield_target(
        &self,
        subject: VehicleHandle,
        target: ConflictPassageAddress,
        now_ms: u64,
        required_lag_ms: u64,
        required_lead_ms: u64,
    ) -> Option<ConflictYieldOutcome> {
        #[cfg(test)]
        count_conflict_work(|counts| counts.yield_queries += 1);
        let index = self.cell_index(target).ok()?;
        let cell = self.cells.get(index).copied().unwrap_or_default();
        if self.zone_owned_by_other(target.zone, subject) {
            return Some(ConflictYieldOutcome::Occupied);
        }
        Some(
            match check_gap(
                now_ms,
                cell.lag,
                required_lag_ms,
                cell.frontier.value_excluding(subject),
                required_lead_ms,
            )? {
                ConflictGapOutcome::Accepted => ConflictYieldOutcome::Accepted,
                ConflictGapOutcome::LagGap => ConflictYieldOutcome::LagGap,
                ConflictGapOutcome::LeadGap => ConflictYieldOutcome::LeadGap,
                ConflictGapOutcome::ApproachUnprovable => ConflictYieldOutcome::ApproachUnprovable,
            },
        )
    }

    pub(crate) fn try_acquire(
        &mut self,
        tick: u64,
        bundle: GrantResourceBundle<'_>,
    ) -> Result<ConflictGrant, ConflictAcquireError> {
        if (bundle.cells.is_empty()
            && (bundle.waiting_entitlement.is_none() || !bundle.downstream.is_empty()))
            || (!bundle.cells.is_empty() && bundle.downstream.is_empty())
            || self.owner_authority(bundle.owner).is_some()
        {
            return Err(ConflictAcquireError::InvalidBundle);
        }
        if bundle.waiting_entitlement.is_some_and(|entitlement| {
            entitlement.owner != bundle.owner || entitlement.tick != tick
        }) {
            return Err(ConflictAcquireError::InvalidBundle);
        }
        if !bundle.cells.is_empty() {
            self.ensure_cells()?;
        }
        reserve_for_len(&mut self.scratch_cell_indices, bundle.cells.len())?;
        self.scratch_cell_indices.clear();
        for address in bundle.cells {
            #[cfg(test)]
            count_conflict_work(|counts| counts.cell_claim_queries += 1);
            let index = self.cell_index(*address)?;
            if self
                .scratch_cell_indices
                .last()
                .is_some_and(|last| *last >= index)
            {
                return Err(ConflictAcquireError::InvalidBundle);
            }
            if self.zone_owned_by_other(address.zone, bundle.owner) {
                #[cfg(test)]
                count_conflict_work(|counts| counts.collision_rejections += 1);
                return Err(ConflictAcquireError::NoGrant(
                    ConflictResourceNoGrant::ConflictOccupied,
                ));
            }
            self.scratch_cell_indices.push(index);
        }
        self.ensure_downstream_index()?;
        if bundle.downstream.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(ConflictAcquireError::InvalidBundle);
        }
        for interval in bundle.downstream {
            #[cfg(test)]
            count_conflict_work(|counts| counts.downstream_claim_queries += 1);
            if self
                .downstream_index
                .conflicts(*interval, bundle.owner, bundle.follower_min_gap_mm)
            {
                #[cfg(test)]
                count_conflict_work(|counts| counts.collision_rejections += 1);
                return Err(ConflictAcquireError::NoGrant(
                    ConflictResourceNoGrant::DownstreamClaimConflict,
                ));
            }
        }
        if self
            .staged_cells
            .len()
            .saturating_add(self.scratch_cell_indices.len())
            > self.conflict_capacity
            || self.owner_authorities.len() >= self.vehicle_capacity
        {
            return Err(ConflictAcquireError::Capacity);
        }
        let staged_required = self
            .staged_downstream
            .len()
            .checked_add(bundle.downstream.len())
            .ok_or(ConflictAcquireError::Capacity)?;
        let committed_required = self
            .committed_downstream
            .len()
            .checked_add(staged_required)
            .ok_or(ConflictAcquireError::Capacity)?;
        let staged_cell_required = self
            .staged_cells
            .len()
            .checked_add(self.scratch_cell_indices.len())
            .ok_or(ConflictAcquireError::Capacity)?;
        let committed_cell_required = self
            .committed_cells
            .len()
            .checked_add(staged_cell_required)
            .ok_or(ConflictAcquireError::Capacity)?;
        let staged_grant_required = self
            .staged_grants
            .len()
            .checked_add(1)
            .ok_or(ConflictAcquireError::Capacity)?;
        let owner_authority_required = self
            .owner_authorities
            .len()
            .checked_add(1)
            .ok_or(ConflictAcquireError::Capacity)?;
        reserve_for_len(&mut self.staged_cells, staged_cell_required)?;
        reserve_for_len(&mut self.committed_cells, committed_cell_required)?;
        reserve_for_len(&mut self.staged_grants, staged_grant_required)?;
        reserve_for_len(&mut self.owner_authorities, owner_authority_required)?;
        reserve_for_len(&mut self.staged_downstream, staged_required)?;
        reserve_for_len(&mut self.committed_downstream, committed_required)?;
        reserve_for_len(&mut self.owner_lookup, self.vehicle_capacity)?;
        self.owner_lookup.resize(self.vehicle_capacity, None);
        if self
            .owner_lookup
            .get(bundle.owner.index() as usize)
            .is_none_or(Option::is_some)
        {
            return Err(ConflictAcquireError::InvalidBundle);
        }
        self.downstream_index.reserve(bundle.downstream.len())?;
        let owner_slot = std::num::NonZeroU32::new(
            u32::try_from(self.owner_authorities.len())
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or(ConflictAcquireError::Capacity)?,
        )
        .ok_or(ConflictAcquireError::Capacity)?;
        let serial = self
            .next_serial
            .checked_add(1)
            .ok_or(ConflictAcquireError::Capacity)?;
        for index in self.scratch_cell_indices.iter().copied() {
            let zone = self.zone_index(self.addresses[index].zone);
            self.cells[zone].zone_staged_owner = Some(bundle.owner);
            self.staged_cells.push((index, bundle.owner, serial));
        }
        for interval in bundle.downstream {
            self.downstream_index
                .insert(*interval, bundle.owner, bundle.follower_min_gap_mm);
            self.staged_downstream.push(OwnedDownstreamClaim {
                owner: bundle.owner,
                follower_min_gap_mm: bundle.follower_min_gap_mm,
                interval: *interval,
                serial,
            });
        }
        self.staged_grants.push(StagedGrant {
            owner: bundle.owner,
            serial,
            consumed: false,
        });
        let mut authority = ConflictOwnerAuthority::staged(
            bundle.owner,
            serial,
            self.scratch_cell_indices.len(),
            bundle.downstream.len(),
        );
        authority.staged_cell_start = self.staged_cells.len() - self.scratch_cell_indices.len();
        authority.staged_downstream_start = self.staged_downstream.len() - bundle.downstream.len();
        authority.staged_grant_index = self.staged_grants.len() - 1;
        let authority_index = self
            .owner_authority_index(bundle.owner)
            .expect_err("validated owner has no authority");
        debug_assert_eq!(authority_index, self.owner_authorities.len());
        self.owner_authorities.push(authority);
        self.owner_lookup[bundle.owner.index() as usize] = Some(owner_slot);
        self.next_serial = serial;
        Ok(ConflictGrant {
            owner: bundle.owner,
            serial,
            tick,
            waiting_zone: bundle
                .waiting_entitlement
                .map(|entitlement| entitlement.zone),
        })
    }

    pub(crate) fn commit_crossing(
        &mut self,
        grant: ConflictGrant,
        passage_range: ConflictPassageRange,
        entered_passage: ConflictPassageAddress,
    ) -> Result<ConflictCrossingCommit, ConflictAcquireError> {
        let result = self.commit_crossing_inner(grant, passage_range, Some(entered_passage))?;
        self.flush_crossings();
        Ok(result)
    }

    /// Gate crossing 建立 reservation，occupancy 由后续已验证 passage 转移建立。
    /// 所有 crossing 结束后统一移动资源行。
    pub(crate) fn commit_gate_crossing_deferred(
        &mut self,
        grant: ConflictGrant,
        range: ConflictPassageRange,
    ) -> Result<ConflictCrossingCommit, ConflictAcquireError> {
        self.commit_crossing_inner(grant, range, None)
    }

    pub(crate) fn validate_gate_crossing(
        &self,
        grant: &ConflictGrant,
        passage_range: ConflictPassageRange,
    ) -> Result<(), ConflictAcquireError> {
        self.crossing_commit_preflight(grant, passage_range, None)
            .map(|_| ())
    }

    fn commit_crossing_inner(
        &mut self,
        grant: ConflictGrant,
        passage_range: ConflictPassageRange,
        entered_passage: Option<ConflictPassageAddress>,
    ) -> Result<ConflictCrossingCommit, ConflictAcquireError> {
        let preflight = self.crossing_commit_preflight(&grant, passage_range, entered_passage)?;
        let previous = self.owner_authorities[preflight.authority_index];
        for offset in 0..previous.staged_cell_count {
            let (index, _, _) = self.staged_cells[previous.staged_cell_start + offset];
            self.cells[index].reservation = Some(grant.owner);
            self.cells[index].reservation_serial = Some(grant.serial);
            self.cells[index].cleared = false;
            if preflight.entered_index == Some(index) {
                self.cells[index].occupant = Some(grant.owner);
            }
            let zone = self.zone_index(self.addresses[index].zone);
            self.cells[zone].zone_committed_owner = Some(grant.owner);
        }
        let reservation = ConflictReservation {
            owner: grant.owner,
            passage_range,
            downstream_owner: grant.owner,
            downstream_claim_count: preflight.downstream_claim_count,
            acquired_tick: grant.tick,
            claim_serial: grant.serial,
        };
        let mut authority = ConflictOwnerAuthority::committed(
            reservation,
            preflight.cells,
            preflight.downstream_claims,
        );
        authority.staged_cell_start = previous.staged_cell_start;
        authority.staged_downstream_start = previous.staged_downstream_start;
        authority.staged_grant_index = previous.staged_grant_index;
        self.owner_authorities[preflight.authority_index] = authority;
        self.staged_grants[previous.staged_grant_index].consumed = true;
        Ok(ConflictCrossingCommit {
            reservation,
            waiting_admission: grant.waiting_zone,
        })
    }

    fn crossing_commit_preflight(
        &self,
        grant: &ConflictGrant,
        passage_range: ConflictPassageRange,
        entered_passage: Option<ConflictPassageAddress>,
    ) -> Result<ConflictCrossingPreflight, ConflictAcquireError> {
        let authority_index = self
            .owner_authority_index(grant.owner)
            .map_err(|_| ConflictAcquireError::InvalidBundle)?;
        let authority = self.owner_authorities[authority_index];
        let cells = authority.staged_cell_count;
        let downstream_claims = authority.staged_downstream_claim_count;
        let cell_count = u32::try_from(cells).map_err(|_| ConflictAcquireError::Capacity)?;
        let downstream_claim_count =
            u32::try_from(downstream_claims).map_err(|_| ConflictAcquireError::Capacity)?;
        let staged = self
            .staged_grants
            .get(authority.staged_grant_index)
            .ok_or(ConflictAcquireError::InvalidBundle)?;
        let cell_rows = self
            .staged_cells
            .get(authority.staged_cell_start..authority.staged_cell_start + cells)
            .ok_or(ConflictAcquireError::InvalidBundle)?;
        let claim_rows = self
            .staged_downstream
            .get(
                authority.staged_downstream_start
                    ..authority.staged_downstream_start + downstream_claims,
            )
            .ok_or(ConflictAcquireError::InvalidBundle)?;
        let entered_index = entered_passage
            .map(|address| self.cell_index(address))
            .transpose()?;
        if cells == 0
            || downstream_claims == 0
            || passage_range.passage_count != cell_count
            || staged.owner != grant.owner
            || staged.serial != grant.serial
            || staged.consumed
            || authority.staged_serial != Some(grant.serial)
            || authority.reservation.is_some()
            || cell_rows.iter().any(|(index, owner, serial)| {
                *owner != grant.owner
                    || *serial != grant.serial
                    || self.cells[*index].reservation.is_some()
            })
            || claim_rows
                .iter()
                .any(|claim| claim.owner != grant.owner || claim.serial != grant.serial)
            || entered_index
                .is_some_and(|index| cell_rows.binary_search_by_key(&index, |row| row.0).is_err())
        {
            return Err(ConflictAcquireError::InvalidBundle);
        }
        Ok(ConflictCrossingPreflight {
            cells,
            downstream_claims,
            downstream_claim_count,
            authority_index,
            entered_index,
        })
    }

    pub(crate) fn validate_pure_waiting_grant(
        &self,
        grant: &ConflictGrant,
    ) -> Result<(), ConflictAcquireError> {
        self.pure_waiting_grant_preflight(grant).map(|_| ())
    }

    pub(crate) fn consume_pure_waiting_grant(
        &mut self,
        grant: ConflictGrant,
    ) -> Result<WaitingZoneOrdinal, ConflictAcquireError> {
        let preflight = self.pure_waiting_grant_preflight(&grant)?;
        self.staged_grants[preflight.staged_index].consumed = true;
        self.owner_authorities[preflight.authority_index] =
            ConflictOwnerAuthority::consumed(grant.owner);
        Ok(preflight.waiting_zone)
    }

    fn pure_waiting_grant_preflight(
        &self,
        grant: &ConflictGrant,
    ) -> Result<PureWaitingGrantPreflight, ConflictAcquireError> {
        let waiting_zone = grant
            .waiting_zone
            .ok_or(ConflictAcquireError::InvalidBundle)?;
        let authority_index = self
            .owner_authority_index(grant.owner)
            .map_err(|_| ConflictAcquireError::InvalidBundle)?;
        let authority = self.owner_authorities[authority_index];
        let staged_index = authority.staged_grant_index;
        let staged = self
            .staged_grants
            .get(staged_index)
            .ok_or(ConflictAcquireError::InvalidBundle)?;
        if staged.owner != grant.owner
            || staged.serial != grant.serial
            || staged.consumed
            || authority.staged_serial != Some(grant.serial)
            || authority.staged_cell_count != 0
            || authority.staged_downstream_claim_count != 0
            || authority.reservation.is_some()
        {
            return Err(ConflictAcquireError::InvalidBundle);
        }
        Ok(PureWaitingGrantPreflight {
            waiting_zone,
            staged_index,
            authority_index,
        })
    }

    pub(crate) fn flush_crossings(&mut self) {
        let mut write = 0;
        for read in 0..self.staged_cells.len() {
            let row = self.staged_cells[read];
            let owner = self.owner_authority_index(row.1).expect("staged owner");
            if self.owner_authorities[owner].pending_commit {
                if self
                    .committed_cells
                    .last()
                    .is_none_or(|last| last.1 != row.1)
                {
                    self.owner_authorities[owner].committed_cell_start = self.committed_cells.len();
                }
                self.committed_cells.push(row);
                let zone = self.zone_index(self.addresses[row.0].zone);
                self.cells[zone].zone_staged_owner = None;
            } else {
                if write == 0 || self.staged_cells[write - 1].1 != row.1 {
                    self.owner_authorities[owner].staged_cell_start = write;
                }
                self.staged_cells[write] = row;
                write += 1;
            }
            #[cfg(test)]
            count_conflict_work(|work| work.commit_resource_visits += 1);
        }
        self.staged_cells.truncate(write);
        let mut write = 0;
        for read in 0..self.staged_downstream.len() {
            let row = self.staged_downstream[read];
            let owner = self.owner_authority_index(row.owner).expect("staged owner");
            if self.owner_authorities[owner].pending_commit {
                if self
                    .committed_downstream
                    .last()
                    .is_none_or(|last| last.owner != row.owner)
                {
                    self.owner_authorities[owner].committed_downstream_start =
                        self.committed_downstream.len();
                }
                self.committed_downstream.push(row);
            } else {
                if write == 0 || self.staged_downstream[write - 1].owner != row.owner {
                    self.owner_authorities[owner].staged_downstream_start = write;
                }
                self.staged_downstream[write] = row;
                write += 1;
            }
            #[cfg(test)]
            count_conflict_work(|work| work.commit_resource_visits += 1);
        }
        self.staged_downstream.truncate(write);
        for authority in &mut self.owner_authorities {
            authority.pending_commit = false;
        }
    }

    pub(crate) fn expire_unconsumed_grants(&mut self) {
        self.flush_crossings();
        for (index, _, _) in &self.staged_cells {
            let zone = self.zone_index(self.addresses[*index].zone);
            self.cells[zone].zone_staged_owner = None;
        }
        self.staged_cells.clear();
        self.staged_downstream.clear();
        self.staged_grants.clear();
        for index in (0..self.owner_authorities.len()).rev() {
            if self.owner_authorities[index].reservation.is_none() {
                self.remove_owner_authority(index);
            }
        }
        self.downstream_index_dirty = true;
    }

    pub(crate) fn enter_passage(
        &mut self,
        owner: VehicleHandle,
        address: ConflictPassageAddress,
    ) -> bool {
        let Ok(index) = self.cell_index(address) else {
            return false;
        };
        let cell = &mut self.cells[index];
        if cell.reservation != Some(owner)
            || cell.cleared
            || cell.occupant.is_some_and(|occupant| occupant != owner)
        {
            return false;
        }
        cell.occupant = Some(owner);
        true
    }

    pub(crate) fn passage_transition_valid_after_staged_commits(
        &self,
        owner: VehicleHandle,
        address: ConflictPassageAddress,
        enter: bool,
        clear: bool,
    ) -> bool {
        let Ok(index) = self.cell_index(address) else {
            return false;
        };
        let cell = &self.cells[index];
        let committed = cell.reservation == Some(owner);
        let staged = cell.reservation.is_none()
            && self.owner_authority(owner).is_some_and(|authority| {
                authority.staged_serial.is_some()
                    && self
                        .staged_cells
                        .get(
                            authority.staged_cell_start
                                ..authority.staged_cell_start + authority.staged_cell_count,
                        )
                        .is_some_and(|rows| rows.binary_search_by_key(&index, |row| row.0).is_ok())
            });
        if (!committed && !staged)
            || (committed && cell.cleared)
            || (enter && cell.occupant.is_some_and(|occupant| occupant != owner))
        {
            return false;
        }
        !clear || enter || cell.occupant == Some(owner)
    }

    #[cfg(test)]
    pub(crate) fn clear_passage(
        &mut self,
        owner: VehicleHandle,
        address: ConflictPassageAddress,
        post_step_time_ms: u64,
    ) -> Option<ConflictClearOutcome> {
        let outcome = self.clear_passage_deferred(owner, address, post_step_time_ms)?;
        if outcome == ConflictClearOutcome::ReservationReleased {
            self.finish_releases();
        }
        Some(outcome)
    }

    pub(crate) fn clear_passage_deferred(
        &mut self,
        owner: VehicleHandle,
        address: ConflictPassageAddress,
        post_step_time_ms: u64,
    ) -> Option<ConflictClearOutcome> {
        let index = self.cell_index(address).ok()?;
        let authority = self.owner_authority_index(owner).ok()?;
        let cell = self.cells.get_mut(index)?;
        if cell.reservation != Some(owner) || cell.occupant != Some(owner) {
            return None;
        }
        cell.occupant = None;
        cell.cleared = true;
        cell.lag = ConflictLagReference::ActualClear(post_step_time_ms);
        self.owner_authorities[authority].uncleared_cell_count -= 1;
        if self.owner_authorities[authority].uncleared_cell_count != 0 {
            return Some(ConflictClearOutcome::Retained);
        }
        self.clear_owner_cells(authority, None);
        self.remove_owner_authority(authority);
        self.downstream_index_dirty = true;
        Some(ConflictClearOutcome::ReservationReleased)
    }

    /// 只遍历该 owner 的连续 cell 段；lag 仅在显式 despawn 时补记。
    fn clear_owner_cells(&mut self, authority: usize, release_time: Option<u64>) {
        let authority = self.owner_authorities[authority];
        for offset in 0..authority.committed_cell_count {
            let index = self.committed_cells[authority.committed_cell_start + offset].0;
            let cell = &mut self.cells[index];
            debug_assert_eq!(cell.reservation, Some(authority.owner));
            if let Some(time) = release_time.filter(|_| !cell.cleared) {
                cell.lag = ConflictLagReference::ActualClear(time);
            }
            cell.reservation = None;
            cell.reservation_serial = None;
            cell.occupant = None;
            cell.cleared = false;
            let zone = self.zone_index(self.addresses[index].zone);
            self.cells[zone].zone_committed_owner = None;
            #[cfg(test)]
            count_conflict_work(|work| work.commit_resource_visits += 1);
        }
        for offset in 0..authority.staged_cell_count {
            let index = self.staged_cells[authority.staged_cell_start + offset].0;
            let zone = self.zone_index(self.addresses[index].zone);
            self.cells[zone].zone_staged_owner = None;
        }
    }

    /// 同拍所有最后净空处理结束后只压缩一次，并同步保留 owner 的范围。
    pub(crate) fn finish_releases(&mut self) {
        let mut write = 0;
        for read in 0..self.committed_cells.len() {
            let row = self.committed_cells[read];
            if let Ok(owner) = self.owner_authority_index(row.1) {
                if write == 0 || self.committed_cells[write - 1].1 != row.1 {
                    self.owner_authorities[owner].committed_cell_start = write;
                }
                self.committed_cells[write] = row;
                write += 1;
            }
            #[cfg(test)]
            count_conflict_work(|work| work.commit_resource_visits += 1);
        }
        self.committed_cells.truncate(write);
        let mut write = 0;
        for read in 0..self.committed_downstream.len() {
            let row = self.committed_downstream[read];
            if let Ok(owner) = self.owner_authority_index(row.owner) {
                if write == 0 || self.committed_downstream[write - 1].owner != row.owner {
                    self.owner_authorities[owner].committed_downstream_start = write;
                }
                self.committed_downstream[write] = row;
                write += 1;
            }
            #[cfg(test)]
            count_conflict_work(|work| work.commit_resource_visits += 1);
        }
        self.committed_downstream.truncate(write);
    }

    pub(crate) fn release_vehicle(&mut self, owner: VehicleHandle, post_step_time_ms: u64) {
        self.flush_crossings();
        if let Ok(index) = self.owner_authority_index(owner) {
            self.clear_owner_cells(index, Some(post_step_time_ms));
            self.remove_owner_authority(index);
        }
        self.staged_cells
            .retain(|(_, current, _)| *current != owner);
        self.staged_downstream.retain(|claim| claim.owner != owner);
        self.staged_grants.retain(|grant| grant.owner != owner);
        for (index, row) in self.staged_cells.iter().enumerate() {
            if index == 0 || self.staged_cells[index - 1].1 != row.1 {
                let owner = self
                    .owner_authority_index(row.1)
                    .expect("retained staged owner");
                self.owner_authorities[owner].staged_cell_start = index;
            }
        }
        for (index, row) in self.staged_downstream.iter().enumerate() {
            if index == 0 || self.staged_downstream[index - 1].owner != row.owner {
                let owner = self
                    .owner_authority_index(row.owner)
                    .expect("retained staged owner");
                self.owner_authorities[owner].staged_downstream_start = index;
            }
        }
        for (index, grant) in self.staged_grants.iter().enumerate() {
            if let Ok(owner) = self.owner_authority_index(grant.owner) {
                self.owner_authorities[owner].staged_grant_index = index;
            }
        }
        self.finish_releases();
        self.downstream_index_dirty = true;
    }

    /// journal replacement 保留独立 lag delta 的权威，不伪造 clear。
    pub(crate) fn remove_authority_for_replay(
        &mut self,
        owner: VehicleHandle,
    ) -> Result<(), ConflictAcquireError> {
        if self.staged_grants.iter().any(|grant| grant.owner == owner) {
            return Err(ConflictAcquireError::InvalidBundle);
        }
        if let Ok(index) = self.owner_authority_index(owner) {
            let authority = self.owner_authorities[index];
            if authority.pending_commit || authority.staged_serial.is_some() {
                return Err(ConflictAcquireError::InvalidBundle);
            }
            self.clear_owner_cells(index, None);
            self.remove_owner_authority(index);
            self.finish_releases();
            self.downstream_index_dirty = true;
        }
        Ok(())
    }

    fn owner_authority_index(&self, owner: VehicleHandle) -> Result<usize, usize> {
        self.owner_lookup
            .get(owner.index() as usize)
            .copied()
            .flatten()
            .map(|index| index.get() as usize - 1)
            .filter(|index| {
                self.owner_authorities
                    .get(*index)
                    .is_some_and(|value| value.owner == owner)
            })
            .ok_or(self.owner_authorities.len())
    }

    fn remove_owner_authority(&mut self, index: usize) {
        let owner = self.owner_authorities[index].owner;
        self.owner_lookup[owner.index() as usize] = None;
        self.owner_authorities.swap_remove(index);
        if let Some(moved) = self.owner_authorities.get(index) {
            self.owner_lookup[moved.owner.index() as usize] =
                std::num::NonZeroU32::new(index as u32 + 1);
        }
        #[cfg(test)]
        count_conflict_work(|work| work.owner_record_moves += 1);
    }

    fn ensure_downstream_index(&mut self) -> Result<(), ConflictAcquireError> {
        if !self.downstream_index_dirty {
            return Ok(());
        }
        self.downstream_index.clear();
        self.downstream_index.reserve(
            self.committed_downstream
                .len()
                .checked_add(self.staged_downstream.len())
                .ok_or(ConflictAcquireError::Capacity)?,
        )?;
        for claim in self
            .committed_downstream
            .iter()
            .chain(&self.staged_downstream)
        {
            self.downstream_index
                .insert(claim.interval, claim.owner, claim.follower_min_gap_mm);
        }
        self.downstream_index_dirty = false;
        Ok(())
    }

    fn owner_authority(&self, owner: VehicleHandle) -> Option<&ConflictOwnerAuthority> {
        self.owner_authority_index(owner)
            .ok()
            .and_then(|index| self.owner_authorities.get(index))
    }

    fn cell_index(&self, address: ConflictPassageAddress) -> Result<usize, ConflictAcquireError> {
        self.addresses
            .binary_search(&address)
            .map_err(|_| ConflictAcquireError::InvalidBundle)
    }

    fn ensure_cells(&mut self) -> Result<(), ConflictAcquireError> {
        if self.cells.len() == self.conflict_capacity {
            return Ok(());
        }
        if !self.cells.is_empty() {
            return Err(ConflictAcquireError::InvalidBundle);
        }
        reserve_for_len(&mut self.cells, self.conflict_capacity)?;
        self.cells
            .resize(self.conflict_capacity, ConflictCellAuthority::default());
        Ok(())
    }

    fn zone_index(&self, zone: ConflictZoneOrdinal) -> usize {
        self.addresses
            .partition_point(|address| address.zone < zone)
    }

    fn zone_owned_by_other(&self, zone: ConflictZoneOrdinal, owner: VehicleHandle) -> bool {
        self.cells.get(self.zone_index(zone)).is_some_and(|cell| {
            cell.zone_committed_owner
                .is_some_and(|other| other != owner)
                || cell.zone_staged_owner.is_some_and(|other| other != owner)
        })
    }

    pub(crate) fn cells_unavailable(
        &self,
        owner: VehicleHandle,
        cells: &[ConflictPassageAddress],
    ) -> bool {
        cells
            .iter()
            .any(|cell| self.zone_owned_by_other(cell.zone, owner))
    }

    #[cfg(test)]
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        let Self {
            addresses,
            cells,
            staged_cells,
            committed_cells,
            scratch_cell_indices,
            staged_downstream,
            committed_downstream,
            staged_grants,
            owner_authorities,
            owner_lookup,
            downstream_index,
            downstream_index_dirty: _,
            next_serial: _,
            conflict_capacity: _,
            vehicle_capacity: _,
        } = self;
        retained_slice_bytes(addresses)
            + retained_vec_bytes(cells)
            + retained_vec_bytes(staged_cells)
            + retained_vec_bytes(committed_cells)
            + retained_vec_bytes(scratch_cell_indices)
            + retained_vec_bytes(staged_downstream)
            + retained_vec_bytes(committed_downstream)
            + retained_vec_bytes(staged_grants)
            + retained_vec_bytes(owner_authorities)
            + retained_vec_bytes(owner_lookup)
            + downstream_index.retained_logical_bytes()
    }
}

pub(crate) fn intervals_conflict(
    left: DownstreamInterval,
    left_min_gap: u32,
    right: DownstreamInterval,
    right_min_gap: u32,
) -> bool {
    if left.edge != right.edge {
        return false;
    }
    if left.start_mm < right.end_mm && right.start_mm < left.end_mm {
        return true;
    }
    if left.end_mm <= right.start_mm {
        right.start_mm - left.end_mm < left_min_gap
    } else {
        left.start_mm - right.end_mm < right_min_gap
    }
}

/// 测试基准使用的 Waiting wait-for 图规范节点。
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum WaitingDependencyNode {
    Owner(u32),
    Zone(WaitingZoneOrdinal),
}

/// 加入候选依赖后是否形成含两个以上 owner 的 SCC。
#[cfg(test)]
pub(crate) fn contains_multi_owner_waiting_cycle(
    edges: &[(WaitingDependencyNode, WaitingDependencyNode)],
) -> bool {
    WaitingCycleScratch::default()
        .contains_multi_owner_cycle(edges)
        .unwrap_or(true)
}

#[cfg(test)]
#[derive(Default)]
struct WaitingCycleScratch {
    nodes: Vec<WaitingDependencyNode>,
    forward: Vec<(usize, usize)>,
    reverse: Vec<(usize, usize)>,
    forward_offsets: Vec<usize>,
    reverse_offsets: Vec<usize>,
    visited: Vec<bool>,
    finish: Vec<usize>,
    dfs_stack: Vec<(usize, usize)>,
    component_stack: Vec<usize>,
}

#[cfg(test)]
impl WaitingCycleScratch {
    fn contains_multi_owner_cycle(
        &mut self,
        edges: &[(WaitingDependencyNode, WaitingDependencyNode)],
    ) -> Result<bool, ConflictAcquireError> {
        let node_limit = edges
            .len()
            .checked_mul(2)
            .ok_or(ConflictAcquireError::Capacity)?;
        reserve_for_len(&mut self.nodes, node_limit)?;
        self.nodes.clear();
        for (from, to) in edges {
            self.nodes.push(*from);
            self.nodes.push(*to);
        }
        self.nodes.sort_unstable();
        self.nodes.dedup();
        if self.nodes.is_empty() {
            return Ok(false);
        }

        reserve_for_len(&mut self.forward, edges.len())?;
        self.forward.clear();
        for (from, to) in edges {
            self.forward.push((
                self.nodes
                    .binary_search(from)
                    .expect("collected source node"),
                self.nodes.binary_search(to).expect("collected target node"),
            ));
        }
        self.forward.sort_unstable();
        self.forward.dedup();

        #[cfg(test)]
        count_conflict_work(|counts| {
            counts.wait_for_nodes += self.nodes.len();
            counts.wait_for_edges += self.forward.len();
        });

        reserve_for_len(&mut self.reverse, self.forward.len())?;
        self.reverse.clear();
        self.reverse
            .extend(self.forward.iter().map(|(from, to)| (*to, *from)));
        self.reverse.sort_unstable();

        let node_count = self.nodes.len();
        fill_dependency_offsets(&mut self.forward_offsets, node_count, &self.forward)?;
        fill_dependency_offsets(&mut self.reverse_offsets, node_count, &self.reverse)?;
        reserve_for_len(&mut self.visited, node_count)?;
        self.visited.clear();
        self.visited.resize(node_count, false);
        reserve_for_len(&mut self.finish, node_count)?;
        self.finish.clear();
        reserve_for_len(&mut self.dfs_stack, node_count)?;
        self.dfs_stack.clear();

        for root in 0..node_count {
            if self.visited[root] {
                continue;
            }
            self.visited[root] = true;
            #[cfg(test)]
            count_conflict_work(|counts| counts.wait_for_visits += 1);
            self.dfs_stack.push((root, self.forward_offsets[root]));
            while let Some((node, next)) = self.dfs_stack.last_mut() {
                let end = self.forward_offsets[*node + 1];
                if *next < end {
                    let target = self.forward[*next].1;
                    *next += 1;
                    if !self.visited[target] {
                        self.visited[target] = true;
                        #[cfg(test)]
                        count_conflict_work(|counts| counts.wait_for_visits += 1);
                        self.dfs_stack.push((target, self.forward_offsets[target]));
                    }
                } else {
                    let (node, _) = self.dfs_stack.pop().expect("non-empty DFS stack");
                    self.finish.push(node);
                }
            }
        }

        self.visited.fill(false);
        reserve_for_len(&mut self.component_stack, node_count)?;
        self.component_stack.clear();
        for finish_index in (0..self.finish.len()).rev() {
            let root = self.finish[finish_index];
            if self.visited[root] {
                continue;
            }
            self.visited[root] = true;
            #[cfg(test)]
            count_conflict_work(|counts| counts.wait_for_visits += 1);
            let mut owner_count =
                usize::from(matches!(self.nodes[root], WaitingDependencyNode::Owner(_)));
            self.component_stack.push(root);
            while let Some(node) = self.component_stack.pop() {
                for (_, target) in
                    &self.reverse[self.reverse_offsets[node]..self.reverse_offsets[node + 1]]
                {
                    if !self.visited[*target] {
                        self.visited[*target] = true;
                        #[cfg(test)]
                        count_conflict_work(|counts| counts.wait_for_visits += 1);
                        owner_count += usize::from(matches!(
                            self.nodes[*target],
                            WaitingDependencyNode::Owner(_)
                        ));
                        self.component_stack.push(*target);
                    }
                }
            }
            if owner_count >= 2 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    #[cfg(test)]
    fn retained_logical_bytes(&self) -> u64 {
        let Self {
            nodes,
            forward,
            reverse,
            forward_offsets,
            reverse_offsets,
            visited,
            finish,
            dfs_stack,
            component_stack,
        } = self;
        retained_vec_bytes(nodes)
            + retained_vec_bytes(forward)
            + retained_vec_bytes(reverse)
            + retained_vec_bytes(forward_offsets)
            + retained_vec_bytes(reverse_offsets)
            + retained_vec_bytes(visited)
            + retained_vec_bytes(finish)
            + retained_vec_bytes(dfs_stack)
            + retained_vec_bytes(component_stack)
    }
}

#[cfg(test)]
fn retained_vec_bytes<T>(values: &Vec<T>) -> u64 {
    u64::try_from(
        values
            .capacity()
            .checked_mul(core::mem::size_of::<T>())
            .expect("retained byte count fits usize"),
    )
    .expect("retained byte count fits u64")
}

#[cfg(test)]
fn retained_slice_bytes<T>(values: &[T]) -> u64 {
    u64::try_from(
        values
            .len()
            .checked_mul(core::mem::size_of::<T>())
            .expect("retained byte count fits usize"),
    )
    .expect("retained byte count fits u64")
}

fn reserve_for_len<T>(values: &mut Vec<T>, required: usize) -> Result<(), ConflictAcquireError> {
    if values.capacity() < required {
        #[cfg(test)]
        check_allocation_failpoint()?;
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| ConflictAcquireError::ScratchAllocFailed)?;
    }
    Ok(())
}

#[cfg(test)]
fn fill_dependency_offsets(
    offsets: &mut Vec<usize>,
    node_count: usize,
    edges: &[(usize, usize)],
) -> Result<(), ConflictAcquireError> {
    let required = node_count
        .checked_add(1)
        .ok_or(ConflictAcquireError::Capacity)?;
    reserve_for_len(offsets, required)?;
    offsets.clear();
    offsets.resize(required, 0);
    for (from, _) in edges {
        offsets[*from + 1] += 1;
    }
    for index in 1..offsets.len() {
        offsets[index] += offsets[index - 1];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vehicle(index: u32) -> VehicleHandle {
        VehicleHandle::new(index, 0)
    }
    fn route(index: u32) -> RouteHandle {
        RouteHandle::new(index, 0)
    }
    fn address(zone: u32, stream: u32, passage: u32) -> ConflictPassageAddress {
        ConflictPassageAddress::new(
            ConflictZoneOrdinal::from_raw(zone),
            ParticipantStreamOrdinal::from_raw(stream),
            passage,
        )
    }
    fn stable_locator(zone: u8, stream: u8) -> ConflictPassageLocator {
        ConflictPassageLocator::new(
            ParticipantStreamId::from_untyped(laneflow_static_contract::StableId128::from_bytes(
                [stream; 16],
            )),
            ConflictZoneId::from_untyped(laneflow_static_contract::StableId128::from_bytes(
                [zone; 16],
            )),
        )
    }
    fn passage_range(
        route_index: u32,
        maneuver: u32,
        gate_hop: u32,
        first_occurrence: u32,
        count: u32,
    ) -> ConflictPassageRange {
        ConflictPassageRange::new(
            route(route_index),
            maneuver,
            gate_hop,
            first_occurrence,
            count,
        )
        .unwrap()
    }
    fn clearing_state(
        owner: VehicleHandle,
        reservation: ConflictReservation,
    ) -> crate::VehicleState {
        crate::VehicleState {
            handle: owner,
            profile: laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
            class: laneflow_static_contract::ParticipantClassOrdinal::from_raw(0),
            route: reservation.route(),
            route_edge_index: 0,
            progress_mm: 0,
            carry_um: 0,
            speed_mm_s: 0,
            length_mm: 4_000,
            status: crate::VehicleStatus::Active,
            maneuver_traversal: Some(crate::ManeuverTraversalState {
                route: reservation.route(),
                maneuver_occurrence_index: reservation.maneuver_occurrence_index(),
                phase: crate::ManeuverTraversalPhase::Clearing {
                    admission_gate_hop: reservation.admission_gate_hop(),
                },
            }),
            waiting_membership: None,
        }
    }
    fn downstream_claims(
        route_edges: &[LaneEdgeOrdinal],
        edge_lengths_mm: &[u32],
        gate_crossed_side: DownstreamRoutePoint,
        farthest_clearance: DownstreamRoutePoint,
        vehicle_length_mm: u32,
        storage_upper_bound: DownstreamRoutePoint,
    ) -> Result<Vec<DownstreamInterval>, ConflictAcquireError> {
        let mut claims = Vec::new();
        prove_downstream_clearance(
            route_edges,
            edge_lengths_mm,
            gate_crossed_side,
            farthest_clearance,
            vehicle_length_mm,
            storage_upper_bound,
            &mut claims,
        )?;
        Ok(claims)
    }

    #[test]
    fn cleared_reservation_cells_require_actual_clear_history() {
        let owner = vehicle(1);
        let cleared = address(0, 0, 0);
        let occupied = address(1, 0, 1);
        let mut arbiter = ConflictArbiter::new(vec![cleared, occupied], 2).expect("arbiter");
        let downstream =
            [DownstreamInterval::new(LaneEdgeOrdinal::from_raw(0), 0, 10).expect("downstream")];
        arbiter
            .restore_reservation(
                owner,
                RestoredConflictReservation {
                    follower_min_gap_mm: 5,
                    acquired_tick: 10,
                    passage_range: passage_range(0, 0, 0, 0, 2),
                    cells: &[
                        RestoredConflictCell {
                            address: cleared,
                            occupant: false,
                            cleared: true,
                        },
                        RestoredConflictCell {
                            address: occupied,
                            occupant: true,
                            cleared: false,
                        },
                    ],
                    downstream: &downstream,
                },
            )
            .expect("restore reservation");
        assert!(!arbiter.authority_owners_valid(|candidate| candidate == owner, 100));

        arbiter
            .restore_lag_reference(cleared, ConflictLagReference::ActualClear(999))
            .expect("restore old actual clear");
        assert!(!arbiter.authority_owners_valid(|candidate| candidate == owner, 100));

        arbiter
            .restore_lag_reference(cleared, ConflictLagReference::ActualClear(1_000))
            .expect("restore acquisition-time clear");
        assert!(arbiter.authority_owners_valid(|candidate| candidate == owner, 100));

        arbiter
            .restore_lag_reference(cleared, ConflictLagReference::CutoverFloor(0))
            .expect("replace with cutover floor");
        assert!(!arbiter.authority_owners_valid(|candidate| candidate == owner, 100));
    }

    #[test]
    fn migration_rows_cover_lazy_cells_once_in_canonical_order() {
        let first = address(0, 0, 0);
        let second = address(1, 0, 1);
        let arbiter = ConflictArbiter::new(vec![second, first], 1).expect("arbiter");
        assert_eq!(
            arbiter.migration_rows().collect::<Vec<_>>(),
            vec![
                (first, ConflictLagReference::NoHistory, false),
                (second, ConflictLagReference::NoHistory, false),
            ]
        );
    }

    #[test]
    fn persistence_view_indexes_each_reservation_without_rescanning_claims() {
        let first_owner = vehicle(1);
        let second_owner = vehicle(3);
        let first_cell = address(0, 0, 0);
        let second_cell = address(1, 0, 1);
        let first_claims = [
            DownstreamInterval::new(LaneEdgeOrdinal::from_raw(0), 0, 10).expect("claim"),
            DownstreamInterval::new(LaneEdgeOrdinal::from_raw(1), 0, 10).expect("claim"),
        ];
        let second_claims =
            [DownstreamInterval::new(LaneEdgeOrdinal::from_raw(2), 0, 10).expect("claim")];
        let mut arbiter = ConflictArbiter::new(vec![first_cell, second_cell], 4).expect("arbiter");
        let first = arbiter
            .restore_reservation(
                first_owner,
                RestoredConflictReservation {
                    follower_min_gap_mm: 5,
                    acquired_tick: 1,
                    passage_range: passage_range(0, 0, 0, 0, 1),
                    cells: &[RestoredConflictCell {
                        address: first_cell,
                        occupant: true,
                        cleared: false,
                    }],
                    downstream: &first_claims,
                },
            )
            .expect("first reservation");
        let second = arbiter
            .restore_reservation(
                second_owner,
                RestoredConflictReservation {
                    follower_min_gap_mm: 7,
                    acquired_tick: 2,
                    passage_range: passage_range(1, 0, 0, 0, 1),
                    cells: &[RestoredConflictCell {
                        address: second_cell,
                        occupant: true,
                        cleared: false,
                    }],
                    downstream: &second_claims,
                },
            )
            .expect("second reservation");

        let mut index = vec![None; 4];
        let view = arbiter.persistence_view(&mut index).expect("valid view");
        assert_eq!(
            view.authority(first_owner)
                .expect("first authority")
                .downstream_claims()
                .map(|claim| claim.interval)
                .collect::<Vec<_>>(),
            first_claims
        );
        assert_eq!(
            view.authority(second_owner)
                .expect("second authority")
                .downstream_claims()
                .map(|claim| claim.interval)
                .collect::<Vec<_>>(),
            second_claims
        );
        assert_eq!(view.authority(first_owner).unwrap().reservation, first);
        assert_eq!(view.authority(second_owner).unwrap().reservation, second);
        assert!(view.authority(vehicle(0)).is_none());

        arbiter.release_vehicle(first_owner, 300);
        let mut compacted_index = vec![None; 4];
        let compacted = arbiter
            .persistence_view(&mut compacted_index)
            .expect("view after stable retain");
        assert!(compacted.authority(first_owner).is_none());
        assert_eq!(
            compacted
                .authority(second_owner)
                .expect("retained authority")
                .downstream_claims()
                .map(|claim| claim.interval)
                .collect::<Vec<_>>(),
            second_claims
        );
    }

    #[test]
    fn chinese_circular_red_is_permissive_but_directional_red_and_prohibitions_deny() {
        assert_eq!(
            interpret_gate_declaration(
                GateInterpretation::CnCircularRightTurn,
                GateProhibition::None,
                true,
                Some(SignalAspect::Red),
            ),
            Some(GatePolicyDecision::Candidate(GateCandidateKind::Permissive))
        );
        for interpretation in [
            GateInterpretation::DirectionalRightProtected,
            GateInterpretation::DirectionalRightPermissive,
        ] {
            assert_eq!(
                interpret_gate_declaration(
                    interpretation,
                    GateProhibition::None,
                    true,
                    Some(SignalAspect::Red),
                ),
                Some(GatePolicyDecision::DenyAndStop)
            );
        }
        for prohibition in [GateProhibition::Always, GateProhibition::OnRed] {
            assert_eq!(
                interpret_gate_declaration(
                    GateInterpretation::CnCircularRightTurn,
                    prohibition,
                    true,
                    Some(SignalAspect::Red),
                ),
                Some(GatePolicyDecision::DenyAndStop)
            );
        }
        assert_eq!(
            interpret_gate_declaration(
                GateInterpretation::CnCircularRightTurn,
                GateProhibition::None,
                false,
                None,
            ),
            None,
            "wrong binding fails closed instead of becoming uncontrolled"
        );
    }

    #[test]
    fn candidate_order_consumes_coverage_min_priority_and_explicit_absence() {
        assert_eq!(coverage_min_priority([7, 2, 9]), Some(2));
        assert_eq!(coverage_min_priority([9, 7, 2]), Some(2));
        let mut keys = [
            ConflictCandidateOrderKey::new(GateCandidateKind::Permissive, None, 1, Some(1), 1),
            ConflictCandidateOrderKey::new(GateCandidateKind::Permissive, Some(4), 1, None, 2),
            ConflictCandidateOrderKey::new(
                GateCandidateKind::Permissive,
                coverage_min_priority([7, 2]),
                1,
                None,
                3,
            ),
            ConflictCandidateOrderKey::new(GateCandidateKind::Protected, Some(-100), 9, None, 4),
        ];
        keys.sort_unstable();
        assert_eq!(keys[0].vehicle_update_sequence, 4);
        assert_eq!(keys[1].vehicle_update_sequence, 2);
        assert_eq!(keys[2].vehicle_update_sequence, 3);
        assert_eq!(keys[3].vehicle_update_sequence, 1);
    }

    #[test]
    fn repeated_locator_resets_first_eligible_tick() {
        let stable = stable_locator(0, 0);
        let first =
            ConflictPassageOccurrenceLocator::new(route(0), 0, 2, 0, address(0, 0, 0), stable);
        let repeated =
            ConflictPassageOccurrenceLocator::new(route(0), 2, 8, 3, address(0, 0, 0), stable);
        let state = ConflictEligibilityState::update(None, first, true, 10).unwrap();
        assert_eq!(
            ConflictEligibilityState::update(Some(state), first, true, 20)
                .unwrap()
                .first_eligible_tick(),
            10
        );
        assert_eq!(
            ConflictEligibilityState::update(Some(state), repeated, true, 20)
                .unwrap()
                .first_eligible_tick(),
            20
        );
        assert_eq!(
            ConflictEligibilityState::update(Some(state), first, false, 30),
            None
        );
    }

    #[test]
    fn top_two_excludes_looping_subject_without_losing_other_owner() {
        let mut cell = ApproachFrontierCell::default();
        cell.insert_owner_reduced(vehicle(1), 10, ApproachEstimate::Finite(20));
        cell.insert_owner_reduced(vehicle(2), 11, ApproachEstimate::Finite(30));
        cell.insert_owner_reduced(vehicle(1), 10, ApproachEstimate::Finite(5));
        assert_eq!(
            cell.value_excluding(vehicle(1)),
            ApproachEstimate::Finite(30)
        );
        assert_eq!(
            cell.value_excluding(vehicle(2)),
            ApproachEstimate::Finite(5)
        );
    }

    #[test]
    fn eta_and_gap_boundaries_are_conservative() {
        assert_eq!(
            approach_eta_lower_bound(ApproachEtaInput {
                exact_distance_mm: u64::MAX,
                carry_um: 0,
                speed_mm_s: u32::MAX,
                max_acceleration_m_s2: 0.0,
                proof_horizon_ms: u64::MAX,
            }),
            ApproachEstimate::Unprovable,
            "u64::MAX rounds to 2^64 as f64 and must fail closed"
        );
        assert_eq!(
            finite_approach_estimate(u64::MAX as f64),
            None,
            "the saturating f64-to-u64 boundary is not a finite ETA"
        );
        assert_eq!(
            approach_eta_lower_bound(ApproachEtaInput {
                exact_distance_mm: 0,
                carry_um: 0,
                speed_mm_s: 0,
                max_acceleration_m_s2: 1.0,
                proof_horizon_ms: 1_001,
            }),
            ApproachEstimate::Finite(0)
        );
        assert_eq!(
            approach_eta_lower_bound(ApproachEtaInput {
                exact_distance_mm: 10_000,
                carry_um: 0,
                speed_mm_s: 0,
                max_acceleration_m_s2: 0.0,
                proof_horizon_ms: 1_001,
            }),
            ApproachEstimate::OutsideHorizon
        );
        assert_eq!(
            check_gap(
                500,
                ConflictLagReference::ActualClear(0),
                500,
                ApproachEstimate::Finite(1_000),
                1_000,
            ),
            Some(ConflictGapOutcome::LeadGap)
        );
        assert_eq!(
            check_gap(
                500,
                ConflictLagReference::ActualClear(0),
                500,
                ApproachEstimate::Finite(1_001),
                1_000,
            ),
            Some(ConflictGapOutcome::Accepted)
        );
        assert_eq!(
            check_gap(
                499,
                ConflictLagReference::ActualClear(0),
                500,
                ApproachEstimate::OutsideHorizon,
                1_000,
            ),
            Some(ConflictGapOutcome::LagGap)
        );
        assert_eq!(
            check_gap(
                499,
                ConflictLagReference::ActualClear(500),
                0,
                ApproachEstimate::OutsideHorizon,
                0,
            ),
            None,
            "future history is an invariant error"
        );
    }

    #[test]
    fn cutover_floor_lag_uses_exact_elapsed_time() {
        for floor in [0, 104] {
            let reference = ConflictLagReference::CutoverFloor(floor);
            assert_eq!(
                check_gap(
                    floor + 499,
                    reference,
                    500,
                    ApproachEstimate::OutsideHorizon,
                    1_000
                ),
                Some(ConflictGapOutcome::LagGap),
            );
            assert_eq!(
                check_gap(
                    floor + 500,
                    reference,
                    500,
                    ApproachEstimate::OutsideHorizon,
                    1_000
                ),
                Some(ConflictGapOutcome::Accepted),
            );
            assert_eq!(
                check_gap(
                    floor + 500,
                    reference,
                    500,
                    ApproachEstimate::Finite(1_000),
                    1_000
                ),
                Some(ConflictGapOutcome::LeadGap),
                "satisfying the cutover lag does not bypass the independent lead proof",
            );
        }
    }

    #[test]
    fn exact_target_checks_occupied_lag_unprovable_and_lead_in_order() {
        let target = address(0, 0, 0);
        let mut arbiter = ConflictArbiter::new(vec![target], 3).unwrap();
        arbiter
            .insert_approach_owner_reduced(target, vehicle(2), 2, ApproachEstimate::Unprovable)
            .unwrap();
        assert_eq!(
            arbiter.evaluate_yield_target(vehicle(1), target, 10, 0, 0),
            Some(ConflictYieldOutcome::ApproachUnprovable)
        );
        let grant = arbiter
            .try_acquire(
                1,
                GrantResourceBundle {
                    owner: vehicle(2),
                    follower_min_gap_mm: 0,
                    cells: &[target],
                    downstream: &[
                        DownstreamInterval::new(LaneEdgeOrdinal::from_raw(0), 0, 1).unwrap()
                    ],
                    waiting_entitlement: None,
                },
            )
            .unwrap();
        assert_eq!(
            arbiter.evaluate_yield_target(vehicle(1), target, 10, 0, 0,),
            Some(ConflictYieldOutcome::Occupied)
        );
        let _commit = arbiter
            .commit_crossing(grant, passage_range(0, 0, 0, 0, 1), target)
            .unwrap();
        assert_eq!(
            arbiter.clear_passage(vehicle(2), target, 100),
            Some(ConflictClearOutcome::ReservationReleased)
        );
        arbiter.clear_approach_frontier();
        assert_eq!(
            arbiter.evaluate_yield_target(vehicle(1), target, 599, 500, 0,),
            Some(ConflictYieldOutcome::LagGap)
        );
        assert_eq!(
            arbiter.evaluate_yield_target(vehicle(1), target, 600, 500, 0,),
            Some(ConflictYieldOutcome::Accepted)
        );
    }

    #[test]
    fn bundle_is_all_or_nothing_and_release_waits_for_last_cell() {
        let a = address(0, 0, 0);
        let b = address(0, 0, 1);
        let c = address(1, 1, 0);
        let mut arbiter = ConflictArbiter::new(vec![c, b, a], 8).unwrap();
        let interval = DownstreamInterval::new(LaneEdgeOrdinal::from_raw(3), 10, 30).unwrap();
        let cells = [a, b];
        let downstream = [interval];
        let grant = arbiter
            .try_acquire(
                7,
                GrantResourceBundle {
                    owner: vehicle(1),
                    follower_min_gap_mm: 5,
                    cells: &cells,
                    downstream: &downstream,
                    waiting_entitlement: Some(WaitingAdmissionEntitlement::new(
                        vehicle(1),
                        WaitingZoneOrdinal::from_raw(0),
                        7,
                    )),
                },
            )
            .unwrap();
        assert_eq!(grant.waiting_zone, Some(WaitingZoneOrdinal::from_raw(0)));
        use WaitingDependencyNode::{Owner, Zone};
        let cycle_edges = [
            (Owner(1), Zone(WaitingZoneOrdinal::from_raw(0))),
            (Zone(WaitingZoneOrdinal::from_raw(0)), Owner(2)),
            (Owner(2), Zone(WaitingZoneOrdinal::from_raw(1))),
            (Zone(WaitingZoneOrdinal::from_raw(1)), Owner(1)),
        ];
        assert!(contains_multi_owner_waiting_cycle(&cycle_edges));
        let rejected = arbiter.try_acquire(
            7,
            GrantResourceBundle {
                owner: vehicle(2),
                follower_min_gap_mm: 5,
                cells: &[c],
                downstream: &[
                    DownstreamInterval::new(LaneEdgeOrdinal::from_raw(3), 31, 40).unwrap(),
                ],
                waiting_entitlement: None,
            },
        );
        assert_eq!(
            rejected.err(),
            Some(ConflictAcquireError::NoGrant(
                ConflictResourceNoGrant::DownstreamClaimConflict
            ))
        );
        assert_eq!(
            arbiter
                .staged_cells
                .iter()
                .filter(|(_, owner, _)| *owner == vehicle(2))
                .count(),
            0,
            "failed bundle leaves no partial zone claim"
        );
        let commit = arbiter
            .commit_crossing(grant, passage_range(0, 3, 4, 0, 2), a)
            .unwrap();
        let reservation = commit.reservation;
        assert_eq!(
            commit.waiting_admission,
            Some(WaitingZoneOrdinal::from_raw(0))
        );
        assert_eq!(reservation.acquired_tick(), 7);
        assert_eq!(reservation.passage_range().passage_count(), 2);
        assert_eq!(reservation.downstream_owner(), vehicle(1));
        assert_eq!(reservation.downstream_claim_count(), 1);
        let mut state = clearing_state(vehicle(1), reservation);
        assert!(arbiter.state_valid(&state));
        state
            .maneuver_traversal
            .as_mut()
            .expect("Clearing traversal")
            .maneuver_occurrence_index += 1;
        assert!(!arbiter.state_valid(&state));
        state
            .maneuver_traversal
            .as_mut()
            .expect("Clearing traversal")
            .maneuver_occurrence_index -= 1;
        state.waiting_membership = Some(crate::WaitingMembership {
            waiting_zone: WaitingZoneOrdinal::from_raw(0),
            admission_sequence: 0,
            release_hop: 4,
        });
        assert!(!arbiter.state_valid(&state));
        state.waiting_membership = None;
        assert_eq!(
            arbiter.clear_passage(vehicle(1), a, 800),
            Some(ConflictClearOutcome::Retained)
        );
        assert!(arbiter.state_valid(&state));
        assert_eq!(
            arbiter
                .owner_authorities
                .iter()
                .filter(|owner| owner.reservation.is_some())
                .count(),
            1
        );
        assert_eq!(arbiter.committed_downstream.len(), 1);
        assert!(arbiter.enter_passage(vehicle(1), b));
        assert_eq!(
            arbiter.clear_passage(vehicle(1), b, 900),
            Some(ConflictClearOutcome::ReservationReleased)
        );
        assert!(!arbiter.state_valid(&state));
        assert!(
            arbiter
                .owner_authorities
                .iter()
                .all(|owner| owner.reservation.is_none())
        );
        assert!(arbiter.committed_downstream.is_empty());
        assert!(!arbiter.has_authority(vehicle(1)));
        arbiter.expire_unconsumed_grants();
        assert!(!arbiter.is_empty(), "last-clear history is W5 state");
        assert_eq!(
            arbiter.lag_reference(b),
            Some(ConflictLagReference::ActualClear(900))
        );
    }

    #[test]
    fn downstream_clearance_uses_actual_length_and_exact_micrometre_boundary() {
        let edges = [LaneEdgeOrdinal::from_raw(0), LaneEdgeOrdinal::from_raw(1)];
        let lengths = [100, 100];
        let gate = DownstreamRoutePoint::new(0, 50, 0).unwrap();
        let clearance = DownstreamRoutePoint::new(0, 90, 0).unwrap();
        let exact = DownstreamRoutePoint::new(1, 10, 0).unwrap();
        let claims = downstream_claims(&edges, &lengths, gate, clearance, 20, exact)
            .expect("target equality passes");
        assert_eq!(
            claims,
            vec![
                DownstreamInterval::new(edges[0], 50, 100).unwrap(),
                DownstreamInterval::new(edges[1], 0, 10).unwrap(),
            ]
        );
        assert_eq!(
            downstream_claims(
                &edges,
                &lengths,
                gate,
                clearance,
                20,
                DownstreamRoutePoint::new(1, 9, 999).unwrap(),
            )
            .unwrap_err(),
            ConflictAcquireError::NoGrant(ConflictResourceNoGrant::DownstreamStorageBoundary)
        );
        assert_eq!(
            downstream_claims(
                &edges,
                &lengths,
                gate,
                clearance,
                111,
                DownstreamRoutePoint::new(1, 100, 0).unwrap(),
            )
            .unwrap_err(),
            ConflictAcquireError::NoGrant(ConflictResourceNoGrant::DownstreamStorageBoundary)
        );
    }

    #[test]
    fn downstream_claims_merge_repeated_physical_edges_and_use_follower_gap() {
        let shared = LaneEdgeOrdinal::from_raw(0);
        let middle = LaneEdgeOrdinal::from_raw(1);
        let edges = [shared, middle, shared];
        let lengths = [100, 50];
        let claims = downstream_claims(
            &edges,
            &lengths,
            DownstreamRoutePoint::new(0, 20, 0).unwrap(),
            DownstreamRoutePoint::new(2, 10, 0).unwrap(),
            30,
            DownstreamRoutePoint::new(2, 40, 0).unwrap(),
        )
        .unwrap();
        assert_eq!(
            claims,
            vec![
                DownstreamInterval::new(shared, 0, 100).unwrap(),
                DownstreamInterval::new(middle, 0, 50).unwrap(),
            ]
        );

        let follower = DownstreamInterval::new(shared, 0, 10).unwrap();
        let leader = DownstreamInterval::new(shared, 15, 20).unwrap();
        assert!(!intervals_conflict(follower, 5, leader, 99));
        assert!(intervals_conflict(follower, 6, leader, 0));
        assert!(!intervals_conflict(leader, 99, follower, 5));
        assert!(intervals_conflict(leader, 0, follower, 6));
    }

    #[test]
    fn downstream_claim_plan_exposes_raw_loop_capacity_before_union_merge() {
        let shared = LaneEdgeOrdinal::from_raw(0);
        let middle = LaneEdgeOrdinal::from_raw(1);
        let edges = [shared, middle, shared];
        let lengths = [100, 50];
        let plan = downstream_claim_plan(
            DownstreamRoutePoint::new(0, 20, 0).unwrap(),
            DownstreamRoutePoint::new(2, 40, 0).unwrap(),
        )
        .expect("valid repeated-edge plan");
        assert_eq!(plan.raw_interval_capacity(), 3);

        let mut undersized = Vec::with_capacity(2);
        assert_eq!(
            derive_downstream_claims_from_plan(&edges, &lengths, plan, &mut undersized),
            Err(ConflictAcquireError::Capacity)
        );

        let mut exact = Vec::with_capacity(plan.raw_interval_capacity());
        derive_downstream_claims_from_plan(&edges, &lengths, plan, &mut exact)
            .expect("exact raw capacity avoids hidden allocation");
        assert_eq!(
            exact,
            vec![
                DownstreamInterval::new(shared, 0, 100).unwrap(),
                DownstreamInterval::new(middle, 0, 50).unwrap(),
            ]
        );
    }

    #[test]
    fn unique_address_uses_canonical_table_and_rejects_ambiguous_locator() {
        let zone = ConflictZoneOrdinal::from_raw(0);
        let stream = ParticipantStreamOrdinal::from_raw(0);
        let unique = address(0, 0, 0);
        let arbiter = ConflictArbiter::new(vec![unique], 1).expect("unique arbiter");
        assert_eq!(arbiter.unique_address(zone, stream), Some(unique));

        let ambiguous = ConflictArbiter::new(vec![unique, address(0, 0, 1)], 1)
            .expect("distinct passage addresses are valid");
        assert_eq!(ambiguous.unique_address(zone, stream), None);
        assert_eq!(
            ambiguous.unique_address(zone, ParticipantStreamOrdinal::from_raw(1)),
            None
        );
    }

    #[test]
    fn checked_failure_never_partially_stages_a_bundle_or_crossing() {
        let cell = address(0, 0, 0);
        let interval = DownstreamInterval::new(LaneEdgeOrdinal::from_raw(0), 0, 10).unwrap();
        let mut arbiter = ConflictArbiter::new(vec![cell], 2).unwrap();
        arbiter.next_serial = u64::MAX;
        assert_eq!(
            arbiter
                .try_acquire(
                    1,
                    GrantResourceBundle {
                        owner: vehicle(1),
                        follower_min_gap_mm: 0,
                        cells: &[cell],
                        downstream: &[interval],
                        waiting_entitlement: None,
                    },
                )
                .err(),
            Some(ConflictAcquireError::Capacity)
        );
        assert!(arbiter.is_empty());
        arbiter.next_serial = 0;

        let grant = arbiter
            .try_acquire(
                1,
                GrantResourceBundle {
                    owner: vehicle(1),
                    follower_min_gap_mm: 0,
                    cells: &[cell],
                    downstream: &[
                        DownstreamInterval::new(LaneEdgeOrdinal::from_raw(0), 0, 1).unwrap()
                    ],
                    waiting_entitlement: None,
                },
            )
            .unwrap();
        assert_eq!(
            arbiter
                .commit_crossing(grant, passage_range(0, 0, 0, 0, 2), cell)
                .err(),
            Some(ConflictAcquireError::InvalidBundle)
        );
        assert_eq!(arbiter.staged_cells.len(), 1);
        assert!(
            arbiter
                .owner_authorities
                .iter()
                .all(|owner| owner.reservation.is_none())
        );
        assert!(arbiter.committed_downstream.is_empty());
    }

    #[test]
    fn crossing_requires_nonempty_downstream_claim() {
        let cell = address(0, 0, 0);
        let mut arbiter = ConflictArbiter::new(vec![cell], 1).unwrap();
        assert_eq!(
            arbiter
                .try_acquire(
                    1,
                    GrantResourceBundle {
                        owner: vehicle(1),
                        follower_min_gap_mm: 0,
                        cells: &[cell],
                        downstream: &[],
                        waiting_entitlement: None,
                    },
                )
                .err(),
            Some(ConflictAcquireError::InvalidBundle)
        );
        assert!(arbiter.is_empty(), "rejection must leave the ledger empty");
    }

    #[test]
    fn staged_crossings_pre_reserve_every_later_commit() {
        let a = address(0, 0, 0);
        let b = address(1, 1, 0);
        let downstream_a = [DownstreamInterval::new(LaneEdgeOrdinal::from_raw(0), 0, 10).unwrap()];
        let downstream_b = [DownstreamInterval::new(LaneEdgeOrdinal::from_raw(1), 0, 10).unwrap()];
        let mut arbiter = ConflictArbiter::new(vec![a, b], 3).unwrap();
        let grant_a = arbiter
            .try_acquire(
                1,
                GrantResourceBundle {
                    owner: vehicle(1),
                    follower_min_gap_mm: 0,
                    cells: &[a],
                    downstream: &downstream_a,
                    waiting_entitlement: None,
                },
            )
            .unwrap();
        let grant_b = arbiter
            .try_acquire(
                1,
                GrantResourceBundle {
                    owner: vehicle(2),
                    follower_min_gap_mm: 0,
                    cells: &[b],
                    downstream: &downstream_b,
                    waiting_entitlement: None,
                },
            )
            .unwrap();

        assert!(arbiter.committed_cells.capacity() >= 2);
        assert!(arbiter.committed_downstream.capacity() >= 2);
        assert!(arbiter.owner_authorities.capacity() >= 2);
        arbiter
            .commit_crossing(grant_a, passage_range(0, 0, 0, 0, 1), a)
            .unwrap();
        arbiter
            .commit_crossing(grant_b, passage_range(0, 1, 1, 1, 1), b)
            .unwrap();
        assert_eq!(
            arbiter
                .owner_authorities
                .iter()
                .filter(|owner| owner.reservation.is_some())
                .count(),
            2
        );
        assert_eq!(arbiter.owner_authorities.len(), 2);
    }

    #[test]
    fn batch_commit_and_last_clear_visit_resources_linearly_and_reuse_owner_slots() {
        const COUNT: u32 = 512;
        let addresses: Vec<_> = (0..COUNT).map(|index| address(index, index, 0)).collect();
        let mut arbiter = ConflictArbiter::new(addresses.clone(), COUNT as usize).unwrap();
        let mut grants = Vec::new();
        for (index, cell) in addresses.iter().copied().enumerate() {
            grants.push(
                arbiter
                    .try_acquire(
                        1,
                        GrantResourceBundle {
                            owner: vehicle(index as u32),
                            follower_min_gap_mm: 2,
                            cells: &[cell],
                            downstream: &[DownstreamInterval::new(
                                LaneEdgeOrdinal::from_raw(index as u32),
                                0,
                                10,
                            )
                            .unwrap()],
                            waiting_entitlement: None,
                        },
                    )
                    .unwrap(),
            );
        }
        reset_conflict_work_counts();
        for (index, grant) in grants.into_iter().enumerate().rev() {
            arbiter
                .commit_gate_crossing_deferred(
                    grant,
                    passage_range(0, index as u32, 0, index as u32, 1),
                )
                .unwrap();
        }
        arbiter.expire_unconsumed_grants();
        assert!(arbiter.authority_owners_valid(|owner| owner.index() < COUNT, 100));
        for (index, cell) in addresses.iter().copied().enumerate().rev() {
            assert!(arbiter.enter_passage(vehicle(index as u32), cell));
            assert_eq!(
                arbiter.clear_passage_deferred(vehicle(index as u32), cell, 200),
                Some(ConflictClearOutcome::ReservationReleased)
            );
        }
        arbiter.finish_releases();
        assert!(arbiter.authority_owners_valid(|_| true, 100));
        assert!(arbiter.committed_cells.is_empty());
        assert!(conflict_work_counts().commit_resource_visits <= 5 * COUNT as usize);
        assert_eq!(conflict_work_counts().owner_record_moves, COUNT as usize);
        let reused = VehicleHandle::new(0, 1);
        let interval = [DownstreamInterval::new(LaneEdgeOrdinal::from_raw(0), 0, 10).unwrap()];
        arbiter
            .try_acquire(
                2,
                GrantResourceBundle {
                    owner: reused,
                    follower_min_gap_mm: 2,
                    cells: &addresses[..1],
                    downstream: &interval,
                    waiting_entitlement: None,
                },
            )
            .unwrap();
        assert!(!arbiter.has_authority(vehicle(0)));
        assert!(arbiter.has_authority(reused));
    }

    #[test]
    fn unused_grant_expires_without_committed_authority() {
        let cell = address(0, 0, 0);
        let mut arbiter = ConflictArbiter::new(vec![cell], 2).unwrap();
        let installed_bytes = arbiter.retained_logical_bytes();
        assert!(installed_bytes >= core::mem::size_of::<ConflictPassageAddress>() as u64);
        let _grant = arbiter
            .try_acquire(
                1,
                GrantResourceBundle {
                    owner: vehicle(1),
                    follower_min_gap_mm: 0,
                    cells: &[cell],
                    downstream: &[
                        DownstreamInterval::new(LaneEdgeOrdinal::from_raw(0), 0, 1).unwrap()
                    ],
                    waiting_entitlement: None,
                },
            )
            .unwrap();
        assert!(
            arbiter.retained_logical_bytes() > installed_bytes,
            "first actual arbitration must expose its lazily retained owner tables"
        );
        arbiter.expire_unconsumed_grants();
        assert!(arbiter.staged_cells.is_empty());
        assert!(
            arbiter
                .owner_authorities
                .iter()
                .all(|owner| owner.reservation.is_none())
        );
        assert!(arbiter.is_empty());
    }

    #[test]
    fn pure_waiting_grant_consumes_entitlement_without_empty_reservation() {
        let mut arbiter = ConflictArbiter::new(Vec::new(), 2).unwrap();
        let zone = WaitingZoneOrdinal::from_raw(3);
        let grant = arbiter
            .try_acquire(
                9,
                GrantResourceBundle {
                    owner: vehicle(1),
                    follower_min_gap_mm: 0,
                    cells: &[],
                    downstream: &[],
                    waiting_entitlement: Some(WaitingAdmissionEntitlement::new(
                        vehicle(1),
                        zone,
                        9,
                    )),
                },
            )
            .unwrap();
        assert_eq!(arbiter.consume_pure_waiting_grant(grant), Ok(zone));
        assert!(
            arbiter
                .owner_authorities
                .iter()
                .all(|owner| owner.reservation.is_none())
        );
        assert!(arbiter.committed_downstream.is_empty());
        assert_eq!(
            arbiter
                .try_acquire(
                    9,
                    GrantResourceBundle {
                        owner: vehicle(1),
                        follower_min_gap_mm: 0,
                        cells: &[],
                        downstream: &[],
                        waiting_entitlement: Some(WaitingAdmissionEntitlement::new(
                            vehicle(1),
                            zone,
                            9,
                        )),
                    },
                )
                .err(),
            Some(ConflictAcquireError::InvalidBundle),
            "one vehicle can consume at most one new Waiting claim in a tick"
        );
        arbiter.expire_unconsumed_grants();
        assert!(arbiter.is_empty());
    }

    #[test]
    fn waiting_cycle_requires_two_distinct_owners_in_one_scc() {
        use WaitingDependencyNode::{Owner, Zone};
        let mut oracle = WaitingCycleScratch::default();
        assert_eq!(oracle.retained_logical_bytes(), 0);
        assert!(
            !oracle
                .contains_multi_owner_cycle(&[(Owner(1), Zone(WaitingZoneOrdinal::from_raw(0)))])
                .unwrap()
        );
        assert!(oracle.retained_logical_bytes() > 0);
        assert!(contains_multi_owner_waiting_cycle(&[
            (Owner(1), Zone(WaitingZoneOrdinal::from_raw(0))),
            (Zone(WaitingZoneOrdinal::from_raw(0)), Owner(2)),
            (Owner(2), Zone(WaitingZoneOrdinal::from_raw(1))),
            (Zone(WaitingZoneOrdinal::from_raw(1)), Owner(1)),
        ]));
        assert!(!contains_multi_owner_waiting_cycle(&[
            (Owner(1), Zone(WaitingZoneOrdinal::from_raw(0))),
            (Zone(WaitingZoneOrdinal::from_raw(0)), Owner(1)),
        ]));
    }
}
