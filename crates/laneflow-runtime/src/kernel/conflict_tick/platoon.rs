use laneflow_static_network::BoundedDistance;

use super::{
    ConflictCandidate, ConflictNoGrantReason, ConflictTaskView, DownstreamEvalError, map_yield,
    no_grant_rank, reserve,
};
use crate::kernel::conflict::ConflictAcquireError;
use crate::kernel::occupancy::{LeaderContact, LeaderQueryHorizon};
use crate::kernel::phase::{StepReadView, StepWorkspace};
use crate::kernel::tables::distance_to_occurrence_progress;
use crate::{
    ConflictPassageAddress, ConflictPassageRange, DownstreamRoutePoint, GateCandidateKind,
    GatePolicyDecision, StepError, VehicleHandle, VehicleState,
};

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct PlatoonScratch {
    /// 候选在首位，随后按物理顺序向链首推进。
    members: Vec<(VehicleHandle, ConflictPassageRange)>,
    pub(super) owners: Vec<VehicleHandle>,
    pub(super) claims: Vec<crate::DownstreamInterval>,
}

impl PlatoonScratch {
    #[cfg(any(test, feature = "execution-evidence"))]
    pub(super) fn retained_logical_bytes(&self) -> usize {
        self.members.capacity() * size_of::<(VehicleHandle, ConflictPassageRange)>()
            + self.owners.capacity() * size_of::<VehicleHandle>()
            + self.claims.capacity() * size_of::<crate::DownstreamInterval>()
    }
}

fn direct_leader(read: StepReadView<'_>, state: VehicleState) -> Option<LeaderContact> {
    let route = read.compiled_route(state.route)?;
    read.derived.occupancy.nearest_leader(
        state.handle,
        &route.edges,
        state.route_edge_index as usize,
        state.progress_mm,
        read.binding.revision.traffic().lane_lengths_millimetres(),
        LeaderQueryHorizon::new(u32::MAX, u32::MAX),
        false,
    )
}

impl StepWorkspace<'_> {
    pub(super) fn platoon_admission_reason(
        &self,
        candidate: ConflictCandidate,
        cells: &[ConflictPassageAddress],
    ) -> Result<Option<ConflictNoGrantReason>, StepError> {
        let read = self.read_view();
        let state = self
            .vehicle_state(candidate.vehicle)
            .ok_or(StepError::ConflictInvariantViolation)?;
        if candidate.key.is_protected() {
            return Ok(None);
        }
        let policy = read
            .binding
            .policy_binding
            .policy(&read.binding.revision)
            .ok_or(StepError::ConflictInvariantViolation)?;
        let mut reason = None;
        for &cell in cells {
            let stream = policy
                .stream(cell.stream(), state.class)
                .ok_or(StepError::ConflictInvariantViolation)?;
            let (_, targets) = policy
                .yield_targets(cell.stream(), state.class, cell.passage_local_index())
                .ok_or(StepError::ConflictInvariantViolation)?;
            let Some(index) = stream.gap_profile_index() else {
                if !targets.is_empty() {
                    return Err(StepError::ConflictInvariantViolation);
                }
                continue;
            };
            let gap = read
                .binding
                .policy_binding
                .gaps()
                .get(index as usize)
                .ok_or(StepError::ConflictInvariantViolation)?;
            for target in targets {
                let address = ConflictPassageAddress::from_resource(
                    cell.resource(),
                    target.stream(),
                    target.passage_local_index(),
                );
                let outcome = self
                    .conflict_read()
                    .platoon_yield_target(
                        state.handle,
                        address,
                        read.committed.time_ms,
                        gap.required_lead_ms(),
                    )
                    .ok_or(StepError::ConflictInvariantViolation)?;
                if let Some(next) = map_yield(outcome)
                    && reason.is_none_or(|prev| no_grant_rank(next) < no_grant_rank(prev))
                {
                    reason = Some(next);
                }
            }
        }
        Ok(reason)
    }
    fn granted_passages(&self, vehicle: VehicleHandle) -> Option<ConflictPassageRange> {
        if let Some(committed) = self.conflict_read().reservation(vehicle) {
            return Some(committed.passage_range());
        }
        let grant = self
            .workspace
            .conflict_motion_by_vehicle
            .get(vehicle.index() as usize)?
            .as_ref()?
            .grant_index?;
        let staged = self
            .workspace
            .conflict_grants
            .get(grant.get() as usize - 1)?;
        (staged.vehicle == vehicle)
            .then(|| staged.passage_range())
            .flatten()
    }

    /// 只在一份普通 coverage 已被持有时进入此路径。每一跳从占用索引取得物理
    /// 直接前车，遇到链外车辆即结束。成员身份在取得资源前再与账本逐项核验。
    pub(super) fn platoon_storage(
        &self,
        candidate: ConflictCandidate,
        cells: &[ConflictPassageAddress],
        scratch: &mut PlatoonScratch,
    ) -> Result<Option<DownstreamRoutePoint>, StepError> {
        scratch.members.clear();
        scratch.owners.clear();
        scratch.claims.clear();
        let Some(range) = candidate.passage_range else {
            return Ok(None);
        };
        let read = self.read_view();
        let conflict = self.conflict_read();
        let mut state = self
            .vehicle_state(candidate.vehicle)
            .ok_or(StepError::ConflictInvariantViolation)?;
        let mut member_range = range;
        let Some(mut front) = storage_plan(read, range, state.length_mm)? else {
            return Ok(None);
        };
        let source = read
            .compiled_route(range.route())
            .ok_or(StepError::ConflictInvariantViolation)?;
        let lengths = read.binding.revision.traffic().lane_lengths_millimetres();
        let view = ConflictTaskView {
            read,
            conflict,
            waiting_plans: &self.workspace.waiting_plans,
            waiting_plan_by_vehicle: &self.workspace.waiting_plan_by_vehicle,
            motion_cache: &self.workspace.motion_cache,
            motion_previews: &self.workspace.motion_previews,
        };
        let mut target = front.target();
        loop {
            let profile = read
                .binding
                .revision
                .traffic()
                .relations()
                .vehicle_profile(state.profile)
                .ok_or(StepError::ConflictInvariantViolation)?;
            let member_gate = member_range.admission_gate_hop();
            let local = target
                .rebase(range.admission_gate_hop(), member_gate)
                .ok_or(StepError::ConflictInvariantViolation)?;
            match view.downstream_hard_bounds_allow(state, member_gate, local) {
                Ok(()) => {}
                Err(DownstreamEvalError::NoGrant) => return Ok(None),
                Err(DownstreamEvalError::Invariant) => {
                    return Err(StepError::ConflictInvariantViolation);
                }
            }
            reserve(&mut scratch.members, 1)?;
            scratch.members.push((state.handle, member_range));
            let Some(contact) = direct_leader(read, state) else {
                break;
            };
            if !conflict.holds_ordinary_coverage(contact.vehicle, cells) {
                if !leader_has_storage(read, state, local, contact, profile.min_gap_mm()) {
                    return Ok(None);
                }
                break;
            }
            if scratch
                .members
                .iter()
                .any(|(owner, _)| *owner == contact.vehicle)
            {
                return Ok(None);
            }
            state = self
                .vehicle_state(contact.vehicle)
                .ok_or(StepError::ConflictInvariantViolation)?;
            member_range = self
                .granted_passages(contact.vehicle)
                .ok_or(StepError::ConflictInvariantViolation)?;
            let Some(delta) = state.length_mm.checked_add(profile.min_gap_mm()) else {
                return Ok(None);
            };
            match crate::kernel::conflict::downstream_claim_target(
                &source.edges,
                lengths,
                target,
                delta,
            ) {
                Ok(next) => target = next,
                Err(ConflictAcquireError::NoGrant(_)) => return Ok(None),
                Err(_) => return Err(StepError::ConflictInvariantViolation),
            }
        }
        if scratch.members.len() < 2 {
            return Ok(None);
        }
        reserve(&mut scratch.owners, scratch.members.len() - 1)?;
        for &(member, member_range) in &scratch.members[1..] {
            let compiled = read
                .compiled_route(member_range.route())
                .ok_or(StepError::ConflictInvariantViolation)?;
            let local = target
                .rebase(
                    range.admission_gate_hop(),
                    member_range.admission_gate_hop(),
                )
                .ok_or(StepError::ConflictInvariantViolation)?;
            let prefix = |gate: u32, end: DownstreamRoutePoint| {
                gate as usize..end.route_edge_index() as usize + usize::from(end.progress_mm() != 0)
            };
            if source
                .edges
                .get(prefix(range.admission_gate_hop(), target))
                .is_none_or(|edges| {
                    compiled
                        .edges
                        .get(prefix(member_range.admission_gate_hop(), local))
                        != Some(edges)
                })
            {
                return Ok(None);
            }
            scratch.owners.push(member);
        }
        scratch.owners.sort_unstable_by_key(|owner| owner.index());
        if !conflict.matches_platoon(&scratch.owners, cells) {
            return Ok(None);
        }
        let next = source
            .gate_hops
            .partition_point(|hop| *hop <= range.admission_gate_hop());
        let Ok(plan) = front.plan.with_authorized_target(
            &source.edges,
            lengths,
            source.gate_hops.get(next).copied(),
            target,
        ) else {
            return Ok(None);
        };
        front.plan = plan;
        reserve(&mut scratch.claims, front.raw_interval_capacity())?;
        crate::kernel::conflict::derive_downstream_claims_from_plan(
            &source.edges,
            lengths,
            plan,
            &mut scratch.claims,
        )
        .map_err(|_| StepError::ConflictInvariantViolation)?;
        Ok(Some(target))
    }
}

fn covered(
    route: &crate::kernel::tables::CompiledRoute,
    hop: u32,
) -> Option<&[crate::kernel::tables::ConflictPassageOccurrence]> {
    let range = route.conflict_gate_ranges.get(hop as usize)?;
    route
        .conflicts
        .get(range.start as usize..range.start.checked_add(range.len)? as usize)
}

impl StepReadView<'_> {
    /// 在制动窗中只把真实的跨流截断作为持续阻塞；当前车能否获授仍交给 P4。
    pub(crate) fn ordinary_platoon_truncation(
        self,
        state: VehicleState,
        hop: u32,
        conflict: crate::kernel::conflict::ConflictRead<'_>,
    ) -> Result<Option<bool>, StepError> {
        let route = self
            .compiled_route(state.route)
            .ok_or(StepError::ConflictInvariantViolation)?;
        let occurrences = covered(route, hop).ok_or(StepError::ConflictInvariantViolation)?;
        if occurrences
            .first()
            .is_none_or(|entry| entry.static_zone().is_none())
        {
            return Ok(None);
        }
        let Some(leader) = direct_leader(self, state) else {
            return Ok(None);
        };
        if !conflict.holds_ordinary_occurrences(leader.vehicle, occurrences) {
            return Ok(None);
        }
        self.platoon_truncated(state, hop, occurrences).map(Some)
    }

    fn platoon_truncated(
        self,
        state: VehicleState,
        hop: u32,
        occurrences: &[crate::kernel::tables::ConflictPassageOccurrence],
    ) -> Result<bool, StepError> {
        let invariant = StepError::ConflictInvariantViolation;
        let route = self.compiled_route(state.route).ok_or(invariant)?;
        let distance = crate::kernel::tables::distance_to_occurrence_start(
            &route.occurrence_segments,
            &route.occurrence_offsets,
            &route.segment_totals,
            state.route_edge_index as usize,
            state.progress_mm,
            hop as usize + 1,
        );
        let Some(BoundedDistance::Finite(mm)) = distance else {
            return Ok(false);
        };
        let profile = self
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
            .ok_or(invariant)?;
        // 整数几何 §5.1 的连续可停判据与黄灯豁免相同；离散位移余数由运动提交处理。
        if laneflow_motion_kernel::stop_distance_um(state.speed_mm_s, profile.emergency_decel())
            > u64::from(mm) * 1_000
        {
            return Ok(false);
        }
        let first = occurrences.first().ok_or(invariant)?;
        let clock = self
            .committed
            .conflict_eligibility
            .get(state.handle.index() as usize)
            .copied()
            .flatten()
            .filter(|e| {
                e.locator().route() == state.route && e.locator().admission_gate_hop() == hop
            })
            .map(|e| e.first_eligible_tick());
        let Some(subject_rank) = self.platoon_priority(state, hop, occurrences, clock)? else {
            return Ok(false);
        };
        if self.committed.conflict_eligibility.is_empty() {
            return Ok(false);
        }
        // 已建立资格的车辆只能在其准入门的上游边尾或跨过侧零点，后一种的
        // 正长度车身仍覆盖上游边。反查竞争流的门，仅访问门所在物理边的占用桶。
        let topology = self.binding.revision.conflict();
        let relations = self.binding.revision.traffic().relations();
        for occurrence in occurrences {
            let zone = occurrence.static_zone().ok_or(invariant)?;
            for &stream in topology
                .conflict_zone(zone)
                .ok_or(invariant)?
                .participant_streams()
            {
                if stream == first.stream {
                    continue;
                }
                let competing = topology.participant_stream(stream).ok_or(invariant)?;
                for passage in competing
                    .passages()
                    .iter()
                    .filter(|p| p.conflict_zone() == zone)
                {
                    let gate = relations
                        .maneuver_gate(passage.admission_gate())
                        .ok_or(invariant)?;
                    let edge = relations
                        .stop_line(gate.stop_line())
                        .ok_or(invariant)?
                        .edge();
                    for owner in self.derived.occupancy.edge_vehicles(edge.index()) {
                        let Some(eligibility) = self
                            .committed
                            .conflict_eligibility
                            .get(owner.index() as usize)
                            .copied()
                            .flatten()
                        else {
                            continue;
                        };
                        let locator = eligibility.locator();
                        if locator.address().stream() == first.stream {
                            continue;
                        }
                        let vehicle = self.vehicle_state(owner).ok_or(invariant)?;
                        let other_route = self.compiled_route(vehicle.route).ok_or(invariant)?;
                        let hop = locator.admission_gate_hop();
                        if vehicle.route != locator.route()
                            || other_route.hop_gate.get(hop as usize).copied().flatten()
                                != Some(passage.admission_gate())
                        {
                            continue;
                        }
                        let other = covered(other_route, hop).ok_or(invariant)?;
                        if self
                            .platoon_priority(
                                vehicle,
                                hop,
                                other,
                                Some(eligibility.first_eligible_tick()),
                            )?
                            .is_some_and(|other_rank| other_rank < subject_rank)
                        {
                            return Ok(true);
                        }
                    }
                }
            }
        }
        Ok(false)
    }

    fn platoon_priority(
        self,
        state: VehicleState,
        hop: u32,
        occurrences: &[crate::kernel::tables::ConflictPassageOccurrence],
        clock: Option<u64>,
    ) -> Result<Option<TruncationPriority>, StepError> {
        let invariant = StepError::ConflictInvariantViolation;
        let gate = self
            .compiled_route(state.route)
            .and_then(|route| route.hop_gate.get(hop as usize))
            .copied()
            .flatten()
            .ok_or(invariant)?;
        let GatePolicyDecision::Candidate(kind) = self.gate_policy_decision(gate, state.profile)
        else {
            return Ok(None);
        };
        let policy = self
            .binding
            .policy_binding
            .policy(&self.binding.revision)
            .ok_or(invariant)?;
        let mut priority = None;
        for entry in occurrences {
            let value = policy
                .stream(entry.stream, state.class)
                .ok_or(invariant)?
                .priority();
            priority = Some(priority.map_or(value, |old: i32| old.min(value)));
        }
        Ok(Some((
            kind != GateCandidateKind::Protected,
            core::cmp::Reverse(priority.ok_or(invariant)?),
            clock.is_none(),
            clock.unwrap_or_default(),
        )))
    }
}

type TruncationPriority = (bool, core::cmp::Reverse<i32>, bool, u64);

fn storage_plan(
    read: StepReadView<'_>,
    range: ConflictPassageRange,
    length: u32,
) -> Result<Option<crate::kernel::world::ReservationDownstreamClaimPlan>, StepError> {
    match read.reservation_downstream_claim_plan(range, length) {
        Ok(plan) => Ok(Some(plan)),
        Err(ConflictAcquireError::NoGrant(_)) => Ok(None),
        Err(_) => Err(StepError::ConflictInvariantViolation),
    }
}

fn leader_has_storage(
    read: StepReadView<'_>,
    state: VehicleState,
    target: DownstreamRoutePoint,
    contact: LeaderContact,
    gap: u32,
) -> bool {
    let Some(compiled) = read.compiled_route(state.route) else {
        return false;
    };
    let current = (state.route_edge_index, state.progress_mm);
    let endpoint = (target.route_edge_index(), target.progress_mm());
    let (start, end) = if current <= endpoint {
        (current, endpoint)
    } else {
        (endpoint, current)
    };
    let Some(BoundedDistance::Finite(mm)) = distance_to_occurrence_progress(
        &compiled.occurrence_segments,
        &compiled.occurrence_offsets,
        &compiled.segment_totals,
        start.0 as usize,
        start.1,
        end.0 as usize,
        end.1,
    ) else {
        return false;
    };
    let required = if current <= endpoint {
        i64::from(mm)
    } else {
        -i64::from(mm)
    };
    contact.gap_mm >= required + i64::from(gap)
}
