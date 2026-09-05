//! W7 fixed-step Conflict orchestration.
//!
//! 本模块只编排已经由 `conflict`、`tables` 与 `waiting` 拥有的语义原语：静态
//! passage cell 不复制到动态路线，候选先完整求值，再由单写者 arbiter 按稳定键取得
//! 组合资源。tick-local grant 不进入公开状态，也不跨 tick 保留。

use laneflow_static_contract::{VehicleProfileOrdinal, WaitingZoneOrdinal};
use laneflow_static_network::BoundedDistance;

use crate::conflict::{
    ConflictAcquireError, ConflictCandidateOrderKey, ConflictGrant, GrantResourceBundle,
    WaitingAdmissionEntitlement,
};
use crate::migration_journal::{ConflictOccurrenceJournalLocator, MigrationDeltaJournal};
use crate::occupancy::LeaderQueryHorizon;
use crate::tables::distance_to_occurrence_progress;
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
    #[must_use]
    pub const fn route(self) -> RouteHandle {
        self.route
    }
    #[must_use]
    pub const fn maneuver_occurrence_index(self) -> u32 {
        self.maneuver_occurrence_index
    }
    #[must_use]
    pub const fn hop(self) -> u32 {
        self.hop
    }
}

/// successful tick 内一个候选没有取得完整组合资源的稳定归因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictNoGrantReason {
    WaitingCapacity,
    WaitingPhysicalStorage,
    WaitingCycle,
    ConflictOccupied,
    LagGap,
    ApproachUnprovable,
    LeadGap,
    DownstreamStorageBoundary,
    DownstreamClaimConflict,
}

/// 刚完成 successful tick 的 Conflict 决定。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictDecisionOutcome {
    NotEvaluated,
    NotRequired,
    Granted,
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
    #[must_use]
    pub const fn vehicle(self) -> VehicleHandle {
        self.vehicle
    }
    #[must_use]
    pub const fn vehicle_update_sequence(self) -> u32 {
        self.vehicle_update_sequence
    }
    #[must_use]
    pub const fn anchor(self) -> ConflictRouteAnchor {
        self.anchor
    }
    #[must_use]
    pub const fn passage(self) -> Option<ConflictPassageOccurrenceLocator> {
        self.passage
    }
    #[must_use]
    pub const fn outcome(self) -> ConflictDecisionOutcome {
        self.outcome
    }
}

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
    range: crate::tables::ConflictGateRange,
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
            crate::conflict::check_allocation_failpoint()
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

    #[cfg(test)]
    fn retained_logical_bytes(&self) -> usize {
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

pub(crate) struct PreparedConflictGrant {
    pub(crate) vehicle: VehicleHandle,
    pub(crate) gate_hop: u32,
    pub(crate) passage_range: Option<ConflictPassageRange>,
    pub(crate) grant: ConflictGrant,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ConflictMotionPlan {
    pub(crate) gate_hop: u32,
    pub(crate) outcome: ConflictDecisionOutcome,
    pub(crate) grant_index: Option<std::num::NonZeroU32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ConflictPassageTransition {
    pub(crate) vehicle: VehicleHandle,
    pub(crate) occurrence_index: u32,
    pub(crate) address: ConflictPassageAddress,
    pub(crate) enter: bool,
    pub(crate) clear: bool,
}

pub(crate) fn reserve<T>(values: &mut Vec<T>, additional: usize) -> Result<(), StepError> {
    #[cfg(test)]
    if additional > values.capacity() - values.len() {
        crate::conflict::check_allocation_failpoint()
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

impl TrafficWorld {
    /// 刚完成 successful tick 的 Conflict 决定批次。
    #[must_use]
    pub fn latest_conflict_decisions(&self) -> &[ConflictDecision] {
        &self.latest_conflict_decisions
    }

    pub(crate) fn conflict_stop_for(
        &self,
        state: VehicleState,
    ) -> Result<Option<crate::waiting::WaitingStopConstraint>, StepError> {
        let grant_hop = self
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
            .conflict_arbiter
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
        let distance = crate::tables::distance_to_occurrence_start(
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            state.route_edge_index as usize,
            state.progress_mm,
            hop as usize + 1,
        )
        .ok_or(StepError::ConflictInvariantViolation)?;
        Ok(Some(crate::waiting::WaitingStopConstraint {
            distance,
            hop,
        }))
    }

    pub(crate) fn prepare_conflict_step(
        &mut self,
        delta_s: f32,
        tick: u64,
    ) -> Result<(), StepError> {
        self.prepare_conflict_candidates(delta_s, tick)?;
        self.acquire_conflict_candidates(tick)
    }

    fn prepare_conflict_candidates(&mut self, delta_s: f32, tick: u64) -> Result<(), StepError> {
        self.conflict_arbiter.expire_unconsumed_grants();
        self.conflict_arbiter.clear_approach_frontier();
        self.conflict_candidates.clear();
        self.conflict_candidate_cells.clear();
        self.conflict_candidate_downstream.clear();
        self.conflict_grants.clear();
        self.conflict_passage_transitions.clear();
        self.conflict_staged_decisions.clear();
        self.conflict_motion_by_vehicle.fill(None);
        self.conflict_next_eligibility.fill(None);

        self.rebuild_conflict_frontier()?;
        reserve(&mut self.conflict_candidates, self.active_order.len())?;
        reserve(&mut self.conflict_staged_decisions, self.active_order.len())?;

        for sequence in 0..self.live_order.len() {
            let vehicle = self.live_order[sequence];
            let Some(state) = self.vehicle_state(vehicle).copied() else {
                continue;
            };
            if state.status != VehicleStatus::Active
                || self.conflict_arbiter.reservation(vehicle).is_some()
            {
                continue;
            }
            self.evaluate_vehicle_gates(
                state,
                u32::try_from(sequence).map_err(|_| StepError::ConflictInvariantViolation)?,
                delta_s,
                tick,
            )?;
        }

        reserve(
            &mut self.conflict_staged_decisions,
            self.conflict_candidates.len(),
        )?;
        self.conflict_candidates
            .sort_unstable_by_key(|candidate| (candidate.key, candidate.vehicle_update_sequence));
        Ok(())
    }

    fn rebuild_conflict_frontier(&mut self) -> Result<(), StepError> {
        let Some(horizon_ms) = self.frontier_proof_horizon_ms() else {
            // 没有任何 gap profile 时不存在 lead frontier 查询；静态 Conflict cell
            // 仍可能被 protected/uncontrolled 或空 yield coverage 使用。
            return Ok(());
        };
        for sequence in 0..self.live_order.len() {
            let vehicle = self.live_order[sequence];
            let Some(state) = self.vehicle_state(vehicle).copied() else {
                continue;
            };
            if state.status != VehicleStatus::Active {
                continue;
            }
            let profile = self
                .revision
                .traffic()
                .relations()
                .vehicle_profile(state.profile)
                .ok_or(StepError::ConflictInvariantViolation)?;
            let first_conflict = self
                .compiled_route(state.route)
                .ok_or(StepError::ConflictInvariantViolation)?
                .conflicts
                .partition_point(|occurrence| {
                    (
                        occurrence.entry.route_edge_index,
                        occurrence.entry.progress_mm,
                    ) < (state.route_edge_index, state.progress_mm)
                });
            let conflict_count = self
                .compiled_route(state.route)
                .ok_or(StepError::ConflictInvariantViolation)?
                .conflicts
                .len();
            for occurrence_index in first_conflict..conflict_count {
                #[cfg(test)]
                crate::conflict::count_conflict_work(|counts| counts.visited_passages += 1);
                let Some((occurrence, exact_distance_mm)) = (|| {
                    let compiled = self.compiled_route(state.route)?;
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
                let estimate =
                    crate::conflict::approach_eta_lower_bound(crate::conflict::ApproachEtaInput {
                        exact_distance_mm: u64::from(exact_distance_mm),
                        carry_um: state.carry_um,
                        speed_mm_s: state.speed_mm_s,
                        max_acceleration_m_s2: profile.max_accel(),
                        proof_horizon_ms: horizon_ms,
                    });
                if estimate == ApproachEstimate::OutsideHorizon {
                    // `conflicts` 按 route position 排列；更远 occurrence 的 directed
                    // lower-bound ETA 也在 proof horizon 外，无需扫完整路线后缀。
                    break;
                }
                self.conflict_arbiter
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
                crate::conflict::count_conflict_work(|counts| counts.frontier_updates += 1);
            }
        }
        Ok(())
    }

    fn evaluate_vehicle_gates(
        &mut self,
        state: VehicleState,
        update_sequence: u32,
        delta_s: f32,
        tick: u64,
    ) -> Result<(), StepError> {
        let compiled = self
            .compiled_route(state.route)
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
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
            .ok_or(StepError::ConflictInvariantViolation)?;
        let horizon = crate::tick::leader_query_horizon(state.speed_mm_s, profile, delta_s)
            .ok_or(StepError::NonFiniteMotion)?;
        let distance = crate::tables::distance_to_occurrence_start(
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            state.route_edge_index as usize,
            state.progress_mm,
            first_hop as usize + 1,
        )
        .ok_or(StepError::ConflictInvariantViolation)?;
        if !matches!(distance, BoundedDistance::Finite(mm) if mm <= horizon.front_query_mm) {
            return Ok(());
        }
        let waiting_plan = self
            .waiting_plan_by_vehicle
            .get(state.handle.index() as usize)
            .copied()
            .flatten()
            .and_then(|index| self.waiting_plans.get(index.get() as usize - 1).copied());
        let waiting_stop = self.waiting_stop_for(state)?;
        let preview = self
            .advance_active_vehicle_with_waiting_stop(state, delta_s, waiting_stop, None)
            .ok_or(StepError::NonFiniteMotion)?;
        let gate_count = compiled.gate_hops.len();
        for gate_index in first_gate..gate_count {
            let compiled = self
                .compiled_route(state.route)
                .ok_or(StepError::ConflictInvariantViolation)?;
            let gate_hop = compiled.gate_hops[gate_index];
            let gate_edge = compiled.edges[gate_hop as usize];
            let gate_progress =
                self.revision.traffic().lane_lengths_millimetres()[gate_edge.index()];
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
                reserve(&mut self.conflict_staged_decisions, 1)?;
                self.conflict_staged_decisions.push(ConflictDecision {
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
        self.conflict_motion_by_vehicle[state.handle.index() as usize] = Some(ConflictMotionPlan {
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
            self.conflict_candidates.push(ConflictCandidate {
                vehicle: state.handle,
                vehicle_update_sequence: update_sequence,
                key,
                anchor,
                passage: None,
                passage_range: None,
                cells_start: self.conflict_candidate_cells.len(),
                cells_end: self.conflict_candidate_cells.len(),
                downstream_start: self.conflict_candidate_downstream.len(),
                downstream_end: self.conflict_candidate_downstream.len(),
                follower_min_gap_mm: 0,
                waiting_zone,
                preflight_no_grant: None,
            });
            return Ok(());
        };

        let eligibility = ConflictEligibilityState::update(
            self.conflict_eligibility
                .get(state.handle.index() as usize)
                .copied()
                .flatten(),
            stable_passage,
            true,
            tick,
        )
        .ok_or(StepError::ConflictInvariantViolation)?;
        self.conflict_next_eligibility[state.handle.index() as usize] = Some(eligibility);

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
        self.conflict_cell_work.clear();
        reserve(&mut self.conflict_cell_work, range.len as usize)?;
        for occurrence_index in range.start..passage_end {
            #[cfg(test)]
            crate::conflict::count_conflict_work(|counts| counts.visited_passages += 1);
            let occurrence = *self
                .compiled_route(state.route)
                .and_then(|compiled| compiled.conflicts.get(occurrence_index as usize))
                .ok_or(StepError::ConflictInvariantViolation)?;
            self.conflict_cell_work.push(occurrence.address());
            let policy = self
                .policy_binding
                .policy(&self.revision)
                .ok_or(StepError::ConflictInvariantViolation)?;
            let stream = policy
                .stream(occurrence.stream, state.profile)
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
                .yield_targets(
                    occurrence.stream,
                    state.profile,
                    occurrence.passage_local_index,
                )
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
                    .conflict_arbiter
                    .evaluate_yield_target(
                        state.handle,
                        address,
                        self.time_ms,
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
        self.conflict_cell_work.sort_unstable();
        self.conflict_cell_work.dedup();
        let cells_start = self.conflict_candidate_cells.len();
        reserve(
            &mut self.conflict_candidate_cells,
            self.conflict_cell_work.len(),
        )?;
        self.conflict_candidate_cells
            .extend_from_slice(&self.conflict_cell_work);

        self.conflict_downstream_work.clear();
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
        let downstream_start = self.conflict_candidate_downstream.len();
        reserve(
            &mut self.conflict_candidate_downstream,
            self.conflict_downstream_work.len(),
        )?;
        self.conflict_candidate_downstream
            .extend_from_slice(&self.conflict_downstream_work);

        let key = ConflictCandidateOrderKey::new(
            kind,
            priority,
            eligibility.first_eligible_tick(),
            state
                .waiting_membership
                .map(|member| member.admission_sequence),
            update_sequence,
        );
        self.conflict_candidates.push(ConflictCandidate {
            vehicle: state.handle,
            vehicle_update_sequence: update_sequence,
            key,
            anchor,
            passage: Some(stable_passage),
            passage_range: Some(passage_range),
            cells_start,
            cells_end: self.conflict_candidate_cells.len(),
            downstream_start,
            downstream_end: self.conflict_candidate_downstream.len(),
            follower_min_gap_mm: self
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

    fn prepare_candidate_downstream(
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
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
            .ok_or(ConflictAcquireError::InvalidBundle)?;
        let leader_gap = self.occupancy.leader_gap(
            state.handle,
            &compiled.edges,
            state.route_edge_index as usize,
            state.progress_mm,
            self.revision.traffic().lane_lengths_millimetres(),
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
            .waiting_stop_for(state)
            .map_err(|_| ConflictAcquireError::InvalidBundle)?
            && waiting.hop > gate_hop
            && target > gate_boundary(waiting.hop)?
        {
            return Err(ConflictAcquireError::NoGrant(
                ConflictResourceNoGrant::DownstreamStorageBoundary,
            ));
        }
        if let Some(ParkingBinding::Reserved(reservation)) = self.parking.binding(state.handle) {
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
        reserve(
            &mut self.conflict_downstream_work,
            plan.raw_interval_capacity(),
        )
        .map_err(|_| ConflictAcquireError::ScratchAllocFailed)?;
        let route = plan.route();
        let compiled = self
            .routes
            .get(route.index() as usize)
            .filter(|slot| slot.generation == route.generation())
            .and_then(|slot| slot.compiled.as_ref())
            .ok_or(ConflictAcquireError::InvalidBundle)?;
        crate::conflict::derive_downstream_claims_from_plan(
            &compiled.edges,
            self.revision.traffic().lane_lengths_millimetres(),
            plan.plan,
            &mut self.conflict_downstream_work,
        )
    }

    fn acquire_conflict_candidates(&mut self, tick: u64) -> Result<(), StepError> {
        self.conflict_schedule
            .prepare(&self.conflict_candidates, &self.waiting_plan_by_vehicle)?;
        self.prepare_waiting_dependencies(true)?;
        #[cfg(test)]
        crate::conflict::count_conflict_work(|counts| {
            counts.candidates += self.conflict_candidates.len();
        });
        while let Some(index) = self.conflict_schedule.next(&self.conflict_candidates) {
            let candidate = self.conflict_candidates[index];
            let entitlement = candidate
                .waiting_zone
                .map(|zone| WaitingAdmissionEntitlement::new(candidate.vehicle, zone, tick));
            let cycle_clear = if entitlement.is_some() {
                let plan = self.waiting_plan_by_vehicle[candidate.vehicle.index() as usize]
                    .ok_or(StepError::WaitingInvariantViolation)?
                    .get() as usize
                    - 1;
                self.waiting_dependencies.stage(plan)?
            } else {
                true
            };
            // 本拍此前接受的候选也参与最高优先级 Occupied 判定。
            // yield target 与申请 passage 共用 zone，zone 摘要覆盖两者。
            let reason = if !cycle_clear {
                Some(ConflictNoGrantReason::WaitingCycle)
            } else if self.conflict_arbiter.cells_unavailable(
                candidate.vehicle,
                &self.conflict_candidate_cells[candidate.cells_start..candidate.cells_end],
            ) {
                Some(ConflictNoGrantReason::ConflictOccupied)
            } else {
                candidate.preflight_no_grant
            };
            let outcome = if let Some(reason) = reason {
                self.waiting_dependencies.rollback();
                ConflictDecisionOutcome::NoGrant(reason)
            } else {
                let result = self.conflict_arbiter.try_acquire(
                    tick,
                    GrantResourceBundle {
                        owner: candidate.vehicle,
                        follower_min_gap_mm: candidate.follower_min_gap_mm,
                        cells: &self.conflict_candidate_cells
                            [candidate.cells_start..candidate.cells_end],
                        downstream: &self.conflict_candidate_downstream
                            [candidate.downstream_start..candidate.downstream_end],
                        waiting_entitlement: entitlement,
                    },
                );
                match result {
                    Ok(grant) => {
                        self.conflict_motion_by_vehicle[candidate.vehicle.index() as usize] =
                            Some(ConflictMotionPlan {
                                gate_hop: candidate.anchor.hop,
                                outcome: ConflictDecisionOutcome::Granted,
                                grant_index: std::num::NonZeroU32::new(
                                    u32::try_from(self.conflict_grants.len())
                                        .map_err(|_| StepError::ConflictInvariantViolation)?
                                        + 1,
                                ),
                            });
                        self.activate_waiting_claim(candidate.vehicle, candidate.anchor.hop)?;
                        if entitlement.is_some() {
                            self.waiting_dependencies.accept();
                        }
                        self.conflict_grants.push(PreparedConflictGrant {
                            vehicle: candidate.vehicle,
                            gate_hop: candidate.anchor.hop,
                            passage_range: candidate.passage_range,
                            grant,
                        });
                        ConflictDecisionOutcome::Granted
                    }
                    Err(error) => {
                        self.waiting_dependencies.rollback();
                        ConflictDecisionOutcome::NoGrant(map_acquire_error(error)?)
                    }
                }
            };
            self.conflict_motion_by_vehicle[candidate.vehicle.index() as usize]
                .as_mut()
                .expect("candidate has motion slot")
                .outcome = outcome;
            self.conflict_staged_decisions.push(ConflictDecision {
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
    fn stage_resource_free_gate_decisions(
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
        let compiled = self
            .compiled_route(next.route)
            .ok_or(StepError::ConflictInvariantViolation)?;
        let first = compiled.gate_hops.partition_point(|hop| *hop < first_hop);
        let last = compiled
            .gate_hops
            .partition_point(|hop| *hop <= next.route_edge_index);
        for index in first..last {
            let compiled = self
                .compiled_route(next.route)
                .ok_or(StepError::ConflictInvariantViolation)?;
            let hop = compiled.gate_hops[index];
            let edge = compiled.edges[hop as usize];
            if next.route_edge_index == hop
                && next.progress_mm
                    < self.revision.traffic().lane_lengths_millimetres()[edge.index()]
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
            reserve(&mut self.conflict_staged_decisions, 1)?;
            self.conflict_staged_decisions.push(ConflictDecision {
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

    pub(crate) fn finalize_conflict_step(
        &mut self,
        updates: &mut [(usize, VehicleState)],
    ) -> Result<(), StepError> {
        self.conflict_passage_transitions.clear();
        self.conflict_changed_owners.clear();
        let journal_armed = self.migration_journal.is_some();
        let transition_capacity = self
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
        reserve(&mut self.conflict_passage_transitions, transition_capacity)?;
        if journal_armed {
            reserve(
                &mut self.conflict_changed_owners,
                transition_capacity
                    .checked_add(self.conflict_grants.len())
                    .ok_or(StepError::ConflictInvariantViolation)?,
            )?;
        }

        // live_order 同时包含 parked/completed；双游标合并两份有序列表，保留正式更新序号。
        let mut update_sequence = 0;
        for (_, next) in updates.iter_mut() {
            while self.live_order.get(update_sequence) != Some(&next.handle) {
                update_sequence += 1;
                if update_sequence >= self.live_order.len() {
                    return Err(StepError::ConflictInvariantViolation);
                }
            }
            self.stage_resource_free_gate_decisions(
                next,
                u32::try_from(update_sequence)
                    .map_err(|_| StepError::ConflictInvariantViolation)?,
            )?;
            #[cfg(test)]
            crate::conflict::count_conflict_work(|counts| counts.vehicle_grant_lookups += 1);
            let grant_index = self.conflict_motion_by_vehicle[next.handle.index() as usize]
                .and_then(|plan| plan.grant_index)
                .map(|index| index.get() as usize - 1);
            let crossed = grant_index
                .is_some_and(|index| next.route_edge_index > self.conflict_grants[index].gate_hop);
            if crossed {
                self.conflict_next_eligibility[next.handle.index() as usize] = None;
            }
            let range = grant_index
                .filter(|_| crossed)
                .and_then(|index| self.conflict_grants[index].passage_range)
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
            if let Some(eligibility) = self.conflict_next_eligibility[slot]
                && !self.conflict_eligibility_valid_with_signals(
                    next,
                    eligibility,
                    &self.next_signal_aspects,
                )
            {
                self.conflict_next_eligibility[slot] = None;
            }
        }

        // 在 mutation boundary 前验证完整提交计划；失败仍只丢弃 tick-local staging。
        for prepared in &self.conflict_grants {
            #[cfg(test)]
            crate::conflict::count_conflict_work(|counts| counts.grant_update_lookups += 1);
            let index = self.next_state_by_vehicle[prepared.vehicle.index() as usize]
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
                    .conflict_arbiter
                    .validate_gate_crossing(&prepared.grant, range)
                    .map_err(|_| StepError::ConflictInvariantViolation)?,
                None => self
                    .conflict_arbiter
                    .validate_pure_waiting_grant(&prepared.grant)
                    .map_err(|_| StepError::ConflictInvariantViolation)?,
            }
        }
        for transition in self.conflict_passage_transitions.iter().copied() {
            if !self
                .conflict_arbiter
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

        self.conflict_staged_decisions
            .sort_unstable_by_key(|decision| {
                (
                    decision.vehicle_update_sequence,
                    decision.anchor.hop,
                    decision.passage.map(|passage| passage.address()),
                )
            });

        Ok(())
    }

    pub(crate) fn commit_conflict_transitions(
        &mut self,
        updates: &[(usize, VehicleState)],
        post_step_time_ms: u64,
    ) {
        let journal_armed = self.migration_journal.is_some();
        // mutation boundary：下面只执行上方已经完整验证且已预留容量的操作。
        for prepared in self.conflict_grants.drain(..) {
            #[cfg(test)]
            crate::conflict::count_conflict_work(|counts| counts.grant_update_lookups += 1);
            let index = self.next_state_by_vehicle[prepared.vehicle.index() as usize] as usize - 1;
            let next = updates[index].1;
            debug_assert_eq!(next.handle, prepared.vehicle);
            if next.route_edge_index <= prepared.gate_hop {
                continue;
            }
            if journal_armed && prepared.passage_range.is_some() {
                self.conflict_changed_owners.push(prepared.vehicle);
            }
            match prepared.passage_range {
                Some(range) => {
                    self.conflict_arbiter
                        .commit_gate_crossing_deferred(prepared.grant, range)
                        .expect("prevalidated Conflict crossing commit");
                }
                None => {
                    self.conflict_arbiter
                        .consume_pure_waiting_grant(prepared.grant)
                        .expect("prevalidated pure Waiting grant commit");
                }
            }
        }
        self.conflict_arbiter.expire_unconsumed_grants();
        let mut released = false;
        for transition in self.conflict_passage_transitions.iter().copied() {
            if journal_armed && (transition.enter || transition.clear) {
                self.conflict_changed_owners.push(transition.vehicle);
            }
            if transition.enter {
                assert!(
                    self.conflict_arbiter
                        .enter_passage(transition.vehicle, transition.address),
                    "prevalidated Conflict passage entry"
                );
            }
            if transition.clear {
                released |= self
                    .conflict_arbiter
                    .clear_passage_deferred(
                        transition.vehicle,
                        transition.address,
                        post_step_time_ms,
                    )
                    .expect("prevalidated Conflict passage clearance")
                    == crate::conflict::ConflictClearOutcome::ReservationReleased;
            }
        }
        if released {
            self.conflict_arbiter.finish_releases();
        }
        if journal_armed {
            self.conflict_changed_owners
                .sort_unstable_by_key(|owner| (owner.index(), owner.generation()));
            self.conflict_changed_owners.dedup();
        }
    }

    fn stage_passage_transitions(
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
                .conflict_arbiter
                .passage_stage(next.handle, occurrence.address());
            let front_reached = route_front_at_or_beyond(
                next,
                occurrence.entry.route_edge_index,
                occurrence.entry.progress_mm,
            );
            let rear_cleared = crate::tables::vehicle_rear_at_or_beyond(
                self.revision.traffic().lane_lengths_millimetres(),
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
            let already_cleared =
                matches!(stage, Some(crate::conflict::ConflictPassageStage::Cleared));
            let occupied = matches!(stage, Some(crate::conflict::ConflictPassageStage::Occupied));
            let enter = front_reached && !already_cleared && !occupied;
            let clear = rear_cleared && !already_cleared;
            if enter || clear {
                self.conflict_passage_transitions
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

    pub(crate) fn commit_conflict_step(&mut self) {
        self.conflict_eligibility.clear();
        self.conflict_eligibility
            .extend_from_slice(&self.conflict_next_eligibility);
        self.normalize_conflict_eligibility();
        core::mem::swap(
            &mut self.latest_conflict_decisions,
            &mut self.conflict_staged_decisions,
        );
    }

    pub(crate) fn write_conflict_tick_journal(
        &self,
        journal: &mut MigrationDeltaJournal,
        updates: &[(usize, VehicleState)],
    ) {
        for (slot, _) in updates {
            let previous = self.conflict_eligibility.get(*slot).copied().flatten();
            let next = self.conflict_next_eligibility.get(*slot).copied().flatten();
            if previous == next {
                continue;
            }
            let owner = self
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

        for owner in self.conflict_changed_owners.iter().copied() {
            let Some(reservation) = self.conflict_arbiter.reservation(owner) else {
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
                    .conflict_arbiter
                    .passage_stage(owner, compiled.conflicts[index as usize].address())
                    .expect("reservation range owns every encoded Conflict cell")
                    .journal_tag();
                (locator, stage)
            });
            journal.tick_conflict_authority(owner, reservation.acquired_tick(), cells);
        }

        for transition in self
            .conflict_passage_transitions
            .iter()
            .filter(|transition| transition.clear)
        {
            let reference = self
                .conflict_arbiter
                .lag_reference(transition.address)
                .expect("validated clear transition retains its Conflict cell");
            journal.tick_conflict_lag(transition.address, reference);
        }
    }

    fn conflict_journal_locator(
        &self,
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

    #[cfg(test)]
    pub(crate) fn conflict_retained_logical_bytes(&self) -> u64 {
        fn vec_bytes<T>(values: &Vec<T>) -> usize {
            values.capacity().saturating_mul(core::mem::size_of::<T>())
        }
        let bytes = self.conflict_arbiter.retained_logical_bytes() as usize
            + self.conflict_schedule.retained_logical_bytes()
            + vec_bytes(&self.conflict_eligibility)
            + vec_bytes(&self.conflict_candidates)
            + vec_bytes(&self.conflict_candidate_cells)
            + vec_bytes(&self.conflict_candidate_downstream)
            + vec_bytes(&self.conflict_cell_work)
            + vec_bytes(&self.conflict_downstream_work)
            + vec_bytes(&self.conflict_grants)
            + self.conflict_motion_by_vehicle.len()
                * core::mem::size_of::<Option<ConflictMotionPlan>>()
            + self.conflict_next_eligibility.len()
                * core::mem::size_of::<Option<ConflictEligibilityState>>()
            + vec_bytes(&self.conflict_passage_transitions)
            + vec_bytes(&self.conflict_changed_owners)
            + self.waiting_dependencies.retained_logical_bytes() as usize
            + vec_bytes(&self.conflict_staged_decisions)
            + vec_bytes(&self.latest_conflict_decisions);
        u64::try_from(bytes).expect("Conflict retained bytes fit u64")
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
    use crate::conflict::{ApproachFrontierCell, conflict_work_counts, reset_conflict_work_counts};
    use crate::cutover_migration::tests::{conflict_scale_revision, conflict_scale_world};

    #[test]
    fn rejected_waiting_bundle_has_no_claim_counter_or_granted_output() {
        let mut world = crate::waiting::tests::multi_gate_world(1);
        assert_eq!(world.conflict_schedule.retained_logical_bytes(), 0);
        world.rebuild_occupancy_index().unwrap();
        world.prepare_waiting_step(0.1).unwrap();
        world.prepare_conflict_candidates(0.1, 1).unwrap();
        assert_eq!(world.conflict_candidates.len(), 1);
        // 单独验证组合仲裁拒绝与 Waiting 发布的接缝；SCC 本身由 arbiter 测试覆盖。
        world.conflict_candidates[0].preflight_no_grant = Some(ConflictNoGrantReason::WaitingCycle);
        world.acquire_conflict_candidates(1).unwrap();
        assert!(world.conflict_schedule.retained_logical_bytes() > 0);
        assert!(world.waiting_claims.is_empty());
        let candidate = world.conflict_candidates[0];
        let old = world.vehicle(candidate.vehicle).unwrap();
        let next = world
            .advance_active_vehicle_with_waiting_stop(
                old,
                0.1,
                world.waiting_stop_for(old).unwrap(),
                world.conflict_stop_for(old).unwrap(),
            )
            .unwrap();
        let mut updates = [(candidate.vehicle.index() as usize, next)];
        world.finalize_waiting_step(&mut updates).unwrap();
        world.finalize_conflict_step(&mut updates).unwrap();
        world.finalize_waiting_outputs(&updates, 1).unwrap();
        assert_eq!(
            world.waiting_staged_decisions[0].outcome(),
            crate::WaitingDecisionOutcome::NoGrant(crate::WaitingNoGrantReason::CombinedResource(
                ConflictNoGrantReason::WaitingCycle
            ))
        );
        assert!(updates[0].1.waiting_membership.is_none());
        assert!(world.staged_transition_events.is_empty());
        assert!(
            world
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
                .step(TickInput::new(world.config.fixed_delta_time_ms()))
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
                .prepare_conflict_step(0.004, world.tick_index() + 1)
                .expect("Conflict arbitration sample");
            if sample >= 3 {
                samples.push(started.elapsed().as_nanos());
            }
        }
        samples.sort_unstable();
        (samples[10], samples[19])
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
        assert_eq!(world.conflict_arbiter.cell_count(), 2);
        assert!(world.conflict_state_valid());
    }

    #[test]
    #[ignore = "manual release-mode Conflict 10k/100k scale evidence"]
    fn conflict_10k_100k_scale_evidence() {
        let revision = conflict_scale_revision();

        let mut product = conflict_scale_world(Arc::clone(&revision), 10_000);
        product.step(TickInput::new(4)).expect("10k warm tick");
        let retained_10k = product.conflict_retained_logical_bytes();
        let (arbitration_p50_ns, arbitration_p95_ns) = arbitration_samples(&mut product);
        let (tick_p50_ns, tick_p95_ns) = percentile_samples(&mut product);
        assert!(product.conflict_state_valid());

        let mut scale = conflict_scale_world(revision, 100_000);
        reset_conflict_work_counts();
        scale.step(TickInput::new(4)).expect("100k evidence tick");
        let work = conflict_work_counts();
        let retained_100k = scale.conflict_retained_logical_bytes();
        let top_two_cells = scale.conflict_arbiter.cell_count();
        let top_two_bytes = top_two_cells * core::mem::size_of::<ApproachFrontierCell>();
        assert!(scale.conflict_state_valid());
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
