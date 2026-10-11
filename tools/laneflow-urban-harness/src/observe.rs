use std::collections::BTreeMap;

use laneflow_runtime::*;
use laneflow_static_contract::{
    EntityKind, ManeuverGateOrdinal, ParkingFacilityOrdinal, SignalAspect, WaitingZoneOrdinal,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::fastmap::FastMap;
use crate::{Artifacts, Harness, IndividualId, Result, invalid};

// This encoding belongs to the harness v1 observation, not a Runtime wire format.
struct Hash {
    digest: Sha256,
    pending: [u8; 256],
    used: usize,
}

impl Default for Hash {
    fn default() -> Self {
        Self {
            digest: Sha256::new(),
            pending: [0; 256],
            used: 0,
        }
    }
}

impl Hash {
    // 只合批送入相同字节，不改变观测编码、摘要域或检查频率。
    fn bytes(&mut self, bytes: &[u8]) {
        if bytes.len() >= self.pending.len() {
            self.flush();
            self.digest.update(bytes);
            return;
        }
        if self.used + bytes.len() > self.pending.len() {
            self.flush();
        }
        self.pending[self.used..self.used + bytes.len()].copy_from_slice(bytes);
        self.used += bytes.len();
    }

    fn flush(&mut self) {
        if self.used != 0 {
            self.digest.update(&self.pending[..self.used]);
            self.used = 0;
        }
    }

    fn n(&mut self, value: impl Into<u64>) {
        self.bytes(&value.into().to_le_bytes());
    }
    fn text(&mut self, text: &str) {
        self.n(text.len() as u64);
        self.bytes(text.as_bytes());
    }
    fn id(&mut self, id: IndividualId) {
        self.n(id.tile);
        self.n(id.slot);
        self.n(id.incarnation);
    }
    fn finish(mut self) -> String {
        self.flush();
        crate::hex(&self.digest.finalize())
    }
}

/// 复用缓冲编码 JSON 后按原样送入哈希。`to_writer` 与 `to_string`/`to_vec` 同一实现，
/// 因此与 `serde_json::to_string` 逐字节一致（有单元测试比对）。
fn text_json<T: serde::Serialize + ?Sized>(
    hash: &mut Hash,
    scratch: &mut Vec<u8>,
    value: &T,
) -> Result<()> {
    scratch.clear();
    serde_json::to_writer(&mut *scratch, value)?;
    hash.n(scratch.len() as u64);
    hash.bytes(scratch);
    Ok(())
}

pub(crate) fn transition_gates(a: &Artifacts) -> FastMap<(u32, u32), (u32, u32)> {
    let mut result = FastMap::default();
    for raw in 0..a.revision.identity().entity_count(EntityKind::ManeuverGate) {
        let gate = a
            .revision
            .traffic()
            .relations()
            .maneuver_gate(ManeuverGateOrdinal::from_raw(raw))
            .expect("checked gate");
        if let Some(group) = gate.signal_group() {
            let path = a
                .revision
                .traffic()
                .maneuvers()
                .maneuver_path(gate.path())
                .expect("checked path");
            let hop = gate.transition_index() as usize;
            result.insert(
                (path.edges()[hop].raw(), path.edges()[hop + 1].raw()),
                (raw, group.raw()),
            );
        }
    }
    result
}

// counts() 与 state() 共用同一生命周期/绑定匹配矩阵，避免冷热路径语义漂移。
fn binding_matches_status(status: VehicleStatus, binding: Option<ParkingBinding>) -> bool {
    matches!(
        (status, binding),
        (VehicleStatus::Parked, Some(ParkingBinding::Occupied(_)))
            | (VehicleStatus::Active, Some(ParkingBinding::Reserved(_)))
            | (VehicleStatus::Active | VehicleStatus::Completed, None)
    )
}

pub(crate) fn counts(h: &Harness<'_>) -> Result<(usize, usize, usize)> {
    let live = h
        .individuals
        .iter()
        .filter(|individual| individual.handle.is_some())
        .count();
    if h.world.live_vehicles().len() != live || h.slots.len() != live {
        return Err(invalid("live identity conservation failed"));
    }
    let expected_live = i64::from(h.plan.individuals) + h.births as i64 - h.removals as i64;
    if expected_live < 0 || live != expected_live as usize {
        return Err(invalid("birth/removal identity conservation failed"));
    }
    let mut counts = (0, 0, 0);
    for (index, individual) in h.individuals.iter().enumerate() {
        let Some(handle) = individual.handle else {
            continue;
        };
        if h.slots.get(&handle) != Some(&index) {
            return Err(invalid("handle identity map differs"));
        }
        let state = h
            .world
            .vehicle(handle)
            .ok_or_else(|| invalid("missing live state"))?;
        match state.status() {
            VehicleStatus::Active => counts.0 += 1,
            VehicleStatus::Parked => counts.1 += 1,
            VehicleStatus::Completed => counts.2 += 1,
        }
        if !binding_matches_status(state.status(), h.world.parking_binding(handle)) {
            return Err(invalid("parking binding and lifecycle disagree"));
        }
    }
    Ok(counts)
}

// parking_invariants() 与 state() 共用同一逐车绑定汇总与车位归属检查。
fn accumulate_parking_binding(
    world: &crate::host::Host,
    bindings: &mut FastMap<ParkingTarget, (u64, u64)>,
    handle: VehicleHandle,
    binding: Option<ParkingBinding>,
) -> Result<()> {
    let Some(binding) = binding else {
        return Ok(());
    };
    let (target, reserved) = match binding {
        ParkingBinding::Reserved(r) => (r.target(), true),
        ParkingBinding::Occupied(t) => (t, false),
    };
    let count = bindings.entry(target).or_default();
    if reserved {
        count.0 += 1;
    } else {
        count.1 += 1;
    }
    if let ParkingTarget::ExplicitSpace(space) = target {
        let expected = if reserved {
            ParkingSpaceState::Reserved(handle)
        } else {
            ParkingSpaceState::Occupied(handle)
        };
        if world.parking_space_state(space) != Some(expected) {
            return Err(invalid("space owner differs from binding"));
        }
    }
    Ok(())
}

pub(crate) fn parking_invariants(h: &Harness<'_>) -> Result<()> {
    let mut bindings: FastMap<ParkingTarget, (u64, u64)> = FastMap::default();
    for individual in &h.individuals {
        let Some(handle) = individual.handle else {
            continue;
        };
        accumulate_parking_binding(&h.world, &mut bindings, handle, h.world.parking_binding(handle))?;
    }
    for (target, key) in &h.target_keys {
        let (reserved, occupied) = bindings.get(target).copied().unwrap_or_default();
        if let ParkingTarget::ExplicitSpace(space) = target
            && (reserved + occupied > 1
                || (reserved + occupied == 0
                    && h.world.parking_space_state(*space) != Some(ParkingSpaceState::Vacant)))
        {
            return Err(invalid(format!(
                "explicit capacity or ownership differs: {key}"
            )));
        }
    }
    for raw in 0..h
        .artifacts
        .revision
        .identity()
        .entity_count(EntityKind::ParkingFacility)
    {
        let facility = ParkingFacilityOrdinal::from_raw(raw);
        let definition = h
            .artifacts
            .revision
            .traffic()
            .relations()
            .parking_facility(facility)
            .ok_or_else(|| invalid("missing facility definition"))?;
        let explicit = definition
            .spaces()
            .iter()
            .fold((0, 0), |(reserved, occupied), space| {
                let count = bindings
                    .get(&ParkingTarget::ExplicitSpace(*space))
                    .copied()
                    .unwrap_or_default();
                (reserved + count.0, occupied + count.1)
            });
        let virtual_pool = bindings
            .get(&ParkingTarget::VirtualPool(facility))
            .copied()
            .unwrap_or_default();
        let counts = h
            .world
            .parking_facility_counts(facility)
            .ok_or_else(|| invalid("missing facility counts"))?;
        validate_facility_counts(counts, explicit, virtual_pool)?;
    }
    Ok(())
}

fn validate_facility_counts(
    counts: ParkingFacilityCounts,
    explicit: (u64, u64),
    virtual_pool: (u64, u64),
) -> Result<()> {
    if (counts.explicit.reserved, counts.explicit.occupied) != explicit {
        return Err(invalid("explicit facility binding count differs"));
    }
    if (counts.virtual_pool.reserved, counts.virtual_pool.occupied) != virtual_pool
        || (counts.total.reserved, counts.total.occupied)
            != (explicit.0 + virtual_pool.0, explicit.1 + virtual_pool.1)
    {
        return Err(invalid("facility binding count differs"));
    }
    for pool in [counts.explicit, counts.virtual_pool, counts.total] {
        if pool.reserved + pool.occupied + pool.vacant != pool.capacity {
            return Err(invalid("parking capacity conservation failed"));
        }
    }
    Ok(())
}

/// step 前一遍完成：step_before 快照、intent 计数与 red_waiters 采集。
/// 状态值、判断条件与插入集合与原先三段独立遍历完全相同。
pub(crate) fn pre_step(h: &mut Harness<'_>) -> usize {
    let warm = h.world.tick_index() >= h.plan.window.warm_up_ticks;
    h.scratch_red.clear();
    if warm {
        let signals = h.world.committed_signal_groups();
        h.scratch_red.extend(
            signals
                .as_slice()
                .iter()
                .map(|(g, a)| (g.raw(), *a == SignalAspect::Red)),
        );
    }
    h.step_before.clear();
    h.step_before.reserve(h.individuals.len());
    let mut intent = 0;
    for i in 0..h.individuals.len() {
        let state = h.individuals[i]
            .handle
            .and_then(|handle| h.world.vehicle(handle));
        if let Some(state) = state
            && state.status() == VehicleStatus::Active
        {
            intent += 1;
            if warm && state.speed_mm_s() == 0 {
                let route = h.route_table.index(state.route());
                let current = state.route_edge_index() as usize;
                if current + 1 < h.route_table.edge_count(route)
                    && h.route_table
                        .edge_len_mm(route, current)
                        .saturating_sub(state.progress_mm())
                        <= 1
                    && let Some(&(gate, group)) = h.transition_gates.get(&(
                        h.route_table.edge_raw(route, current),
                        h.route_table.edge_raw(route, current + 1),
                    ))
                    && h.scratch_red.get(&group) == Some(&true)
                {
                    let id = h.individuals[i].id;
                    h.red_waiters.insert((id, gate));
                }
            }
        }
        h.step_before.push(state);
    }
    intent
}

fn conflict_reason(reason: ConflictNoGrantReason) -> u32 {
    match reason {
        ConflictNoGrantReason::WaitingCapacity => 0,
        ConflictNoGrantReason::WaitingPhysicalStorage => 1,
        ConflictNoGrantReason::WaitingCycle => 2,
        ConflictNoGrantReason::ConflictOccupied => 3,
        ConflictNoGrantReason::LagGap => 4,
        ConflictNoGrantReason::ApproachUnprovable => 5,
        ConflictNoGrantReason::LeadGap => 6,
        ConflictNoGrantReason::DownstreamStorageBoundary => 7,
        ConflictNoGrantReason::DownstreamClaimConflict => 8,
    }
}

fn conflict_reason_name(reason: ConflictNoGrantReason) -> &'static str {
    match reason {
        ConflictNoGrantReason::WaitingCapacity => "waiting-capacity",
        ConflictNoGrantReason::WaitingPhysicalStorage => "waiting-storage",
        ConflictNoGrantReason::WaitingCycle => "waiting-cycle",
        ConflictNoGrantReason::ConflictOccupied => "conflict-occupied",
        ConflictNoGrantReason::LagGap => "lag-gap",
        ConflictNoGrantReason::ApproachUnprovable => "approach-unprovable",
        ConflictNoGrantReason::LeadGap => "lead-gap",
        ConflictNoGrantReason::DownstreamStorageBoundary => "downstream-storage",
        ConflictNoGrantReason::DownstreamClaimConflict => "downstream-claim",
    }
}

fn passage(h: &Harness<'_>, p: ConflictPassageOccurrenceLocator) -> Value {
    json!([
        h.route_key(p.route()),
        p.maneuver_occurrence_index(),
        p.admission_gate_hop(),
        p.conflict_occurrence_index(),
        p.address().zone().raw(),
        p.address().stream().raw(),
        p.address().passage_local_index()
    ])
}
fn range(h: &Harness<'_>, r: ConflictPassageRange) -> Value {
    json!([
        h.route_key(r.route()),
        r.maneuver_occurrence_index(),
        r.admission_gate_hop(),
        r.first_conflict_occurrence_index(),
        r.passage_count()
    ])
}

// Match an actual tick-start claim to this subject's declared yield target. Generic
// resource rejection, an active pulse elsewhere, or a downstream blockage is not proof.
fn occupied_yield_pulse(
    h: &Harness<'_>,
    decision: ConflictDecision,
    subject: IndividualId,
    pulse_prefix: &str,
) -> Option<crate::runner::RightOfWayWitness> {
    if decision.outcome()
        != ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::ConflictOccupied)
    {
        return None;
    }
    let first = decision.passage()?;
    let class = h.world.vehicle(decision.vehicle())?.class();
    let policy = h.world.policy()?;
    // The decision anchors the first passage of the whole admission bundle, not the
    // particular passage that rejected it. Inspect only that same maneuver/gate bundle.
    let passages = (first.conflict_occurrence_index()..)
        .map_while(|index| {
            h.world
                .conflict_passage_occurrence_locator(first.route(), index)
        })
        .take_while(|passage| {
            passage.maneuver_occurrence_index() == first.maneuver_occurrence_index()
                && passage.admission_gate_hop() == first.admission_gate_hop()
        });
    for passage in passages {
        let address = passage.address();
        let (zone, targets) =
            policy.yield_targets(address.stream(), class, address.passage_local_index())?;
        if zone != address.zone() {
            return None;
        }
        let pulse = h
            .claims
            .iter()
            .find_map(|((owner, route, index), claimed)| {
                if owner.tile != subject.tile || *claimed != zone.raw() {
                    return None;
                }
                let slot = owner.tile as usize * 1_000 + owner.slot as usize;
                if h.individuals[slot].id != *owner
                    || !h.plan.initial[slot]
                        .role
                        .as_deref()
                        .is_some_and(|role| role.starts_with(pulse_prefix))
                {
                    return None;
                }
                let target = h
                    .world
                    .conflict_passage_occurrence_locator(h.route_table.handle(*route), *index)?
                    .address();
                targets
                    .iter()
                    .any(|cell| {
                        cell.stream() == target.stream()
                            && cell.passage_local_index() == target.passage_local_index()
                    })
                    .then_some(*owner)
            });
        let Some(pulse) = pulse else {
            continue;
        };
        return Some(crate::runner::RightOfWayWitness {
            subject,
            pulse,
            zone: zone.raw(),
            maneuver_occurrence: decision.anchor().maneuver_occurrence_index(),
            occupied_wait: (h.world.tick_index(), decision.vehicle_update_sequence()),
            pulse_cleared: None,
            granted: None,
            passed: None,
        });
    }
    None
}

pub(crate) fn events(h: &mut Harness<'_>) -> Result<()> {
    // Every decision, including NotEvaluated/NotRequired, contributes in public batch order.
    // Store its digest once per tick instead of millions of repetitive diagnostic rows.
    let mut decisions = Hash::default();
    decisions.text("waiting");
    for d in h.world.latest_waiting_decisions() {
        let id = h
            .stable_individual(d.vehicle())
            .ok_or_else(|| invalid("unknown decision owner"))?;
        decisions.id(id);
        decisions.n(d.vehicle_update_sequence());
        decisions.text(h.route_key(d.anchor().route()));
        decisions.n(d.anchor().maneuver_occurrence_index());
        decisions.n(d.anchor().hop());
        decisions.n(d.zone().map_or(u32::MAX, |z| z.raw()));
        let (tag, reason) = match d.outcome() {
            WaitingDecisionOutcome::Deferred => (0_u32, 0),
            WaitingDecisionOutcome::NotEvaluated => (1, 0),
            WaitingDecisionOutcome::NotRequired => (2, 0),
            WaitingDecisionOutcome::Granted => (3, 0),
            WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::Capacity) => (4, 0),
            WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::PhysicalStorage) => (4, 1),
            WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::CombinedResource(r)) => {
                (4, 2 + conflict_reason(r))
            }
        };
        if h.in_step_observation(h.world.tick_index())
            && h.plan.case == "WAITING-RELEASE"
            && h.plan.initial[id.tile as usize * 1_000 + id.slot as usize]
                .role
                .as_deref()
                .is_some_and(|role| role.starts_with("waiting-fifo-"))
        {
            match d.outcome() {
                WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::Capacity) => {
                    h.evidence[id.tile as usize].waiting_capacity_rejections += 1;
                }
                WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::PhysicalStorage) => {
                    h.evidence[id.tile as usize].waiting_storage_rejections += 1;
                }
                WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::CombinedResource(
                    ConflictNoGrantReason::WaitingCapacity,
                )) => {
                    h.evidence[id.tile as usize].waiting_capacity_rejections += 1;
                }
                WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::CombinedResource(
                    ConflictNoGrantReason::WaitingPhysicalStorage
                    | ConflictNoGrantReason::DownstreamStorageBoundary,
                )) => {
                    h.evidence[id.tile as usize].waiting_storage_rejections += 1;
                }
                _ => {}
            }
        }
        decisions.n(tag);
        decisions.n(reason);
    }
    decisions.text("conflict");
    for d in h.world.latest_conflict_decisions() {
        let id = h
            .stable_individual(d.vehicle())
            .ok_or_else(|| invalid("unknown decision owner"))?;
        decisions.id(id);
        decisions.n(d.vehicle_update_sequence());
        decisions.text(h.route_key(d.anchor().route()));
        decisions.n(d.anchor().maneuver_occurrence_index());
        decisions.n(d.anchor().hop());
        let encoded_passage = d.passage().map(|p| passage(h, p));
        text_json(&mut decisions, &mut h.json_scratch, &encoded_passage)?;
        let (tag, reason) = match d.outcome() {
            ConflictDecisionOutcome::NotEvaluated => (0_u32, 0),
            ConflictDecisionOutcome::NotRequired => (1, 0),
            ConflictDecisionOutcome::Granted => (2, 0),
            ConflictDecisionOutcome::NoGrant(r) => (3, conflict_reason(r)),
        };
        let role = h.plan.initial[id.tile as usize * 1_000 + id.slot as usize]
            .role
            .as_deref();
        let pulse_prefix = match (h.plan.case.as_str(), role) {
            ("PERMISSIVE-LEFT", Some("permissive-left")) => Some("opposing-pulse-"),
            ("UNCONTROLLED-YIELD", Some("yield-role")) => Some("mainline-pulse-"),
            _ => None,
        };
        if h.in_step_observation(h.world.tick_index())
            && let Some(prefix) = pulse_prefix
        {
            let witness = h.evidence[id.tile as usize]
                .right_of_way
                .is_none()
                .then(|| occupied_yield_pulse(h, *d, id, prefix))
                .flatten();
            let evidence = &mut h.evidence[id.tile as usize];
            if let Some(witness) = witness {
                if h.plan.case == "PERMISSIVE-LEFT" {
                    evidence.permissive_no_grants += 1;
                } else {
                    evidence.yield_waits += 1;
                }
                evidence.right_of_way = Some(witness);
            }
            match d.outcome() {
                ConflictDecisionOutcome::Granted => {
                    if h.plan.case == "PERMISSIVE-LEFT" {
                        evidence.permissive_grants += 1;
                    }
                    if let Some(witness) = &mut evidence.right_of_way
                        && witness.subject == id
                        && witness.maneuver_occurrence == d.anchor().maneuver_occurrence_index()
                    {
                        witness
                            .granted
                            .get_or_insert((h.world.tick_index(), d.vehicle_update_sequence()));
                    }
                }
                ConflictDecisionOutcome::NoGrant(reason) => {
                    *evidence
                        .role_no_grant_reasons
                        .entry(conflict_reason_name(reason).into())
                        .or_default() += 1;
                }
                _ => {}
            }
        }
        if h.in_step_observation(h.world.tick_index())
            && h.plan.case == "WAITING-RELEASE"
            && role.is_some_and(|role| role.starts_with("waiting-fifo-"))
            && matches!(
                d.outcome(),
                ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::DownstreamStorageBoundary)
            )
        {
            h.evidence[id.tile as usize].waiting_storage_rejections += 1;
        }
        decisions.n(tag);
        decisions.n(reason);
    }
    h.events.push(json!({"kind":"decision-batch", "tick":h.world.tick_index(),
        "waiting_count":h.world.latest_waiting_decisions().len(), "conflict_count":h.world.latest_conflict_decisions().len(), "digest":decisions.finish()}));
    for e in h.world.latest_transition_events() {
        let id = h
            .stable_individual(e.vehicle())
            .ok_or_else(|| invalid("unknown transition owner"))?;
        match e.kind() {
            TrafficTransitionKind::ReservationAcquired { passage_range: r } => {
                for index in r.first_conflict_occurrence_index()
                    ..r.first_conflict_occurrence_index() + r.passage_count()
                {
                    let p = h
                        .world
                        .conflict_passage_occurrence_locator(r.route(), index)
                        .ok_or_else(|| invalid("unknown reservation passage"))?;
                    if h.claims
                        .insert(
                            (id, h.route_table.index(r.route()), index),
                            p.address().zone().raw(),
                        )
                        .is_some()
                    {
                        return Err(invalid("duplicate committed claim"));
                    }
                }
            }
            TrafficTransitionKind::ConflictCleared { passage: p } => {
                if h.in_step_observation(e.tick())
                    && let Some(witness) = &mut h.evidence[id.tile as usize].right_of_way
                    && witness.pulse == id
                    && witness.zone == p.address().zone().raw()
                {
                    witness
                        .pulse_cleared
                        .get_or_insert((e.tick(), e.vehicle_update_sequence()));
                }
                if h.claims
                    .remove(&(
                        id,
                        h.route_table.index(p.route()),
                        p.conflict_occurrence_index(),
                    ))
                    .is_none()
                {
                    return Err(invalid("cleared passage has no committed claim"));
                }
            }
            TrafficTransitionKind::ReservationReleased { .. }
                if h.claims.keys().any(|(owner, _, _)| *owner == id) =>
            {
                return Err(invalid("reservation released before all claims cleared"));
            }
            _ => {}
        }
        let detail = match e.kind() {
            TrafficTransitionKind::ProjectionApplied { zone, reason } => json!([
                "projection",
                zone.raw(),
                match reason {
                    WaitingProjectionReason::EvaluationHorizon => "horizon",
                    WaitingProjectionReason::Capacity => "capacity",
                    WaitingProjectionReason::PhysicalStorage => "storage",
                }
            ]),
            TrafficTransitionKind::GateCrossed { gate } => {
                if h.red_waiters.remove(&(id, gate.raw())) {
                    h.evidence[id.tile as usize].red_wait_then_crossed += 1;
                }
                json!(["gate-crossed", gate.raw()])
            }
            TrafficTransitionKind::WaitingLeft {
                zone,
                admission_sequence,
            } => {
                if h.in_step_observation(e.tick())
                    && h.plan.case == "WAITING-RELEASE"
                    && h.plan.initial[id.tile as usize * 1_000 + id.slot as usize]
                        .role
                        .as_deref()
                        .is_some_and(|role| role.starts_with("waiting-fifo-"))
                {
                    let evidence = &mut h.evidence[id.tile as usize];
                    evidence.waiting_releases += 1;
                    evidence.waiting_release_order.push(id.slot);
                }
                json!(["waiting-left", zone.raw(), admission_sequence])
            }
            TrafficTransitionKind::WaitingEntered {
                zone,
                admission_sequence,
            } => {
                if h.in_step_observation(e.tick())
                    && h.plan.case == "WAITING-RELEASE"
                    && h.plan.initial[id.tile as usize * 1_000 + id.slot as usize]
                        .role
                        .as_deref()
                        .is_some_and(|role| role.starts_with("waiting-fifo-"))
                {
                    let evidence = &mut h.evidence[id.tile as usize];
                    evidence.waiting_entries += 1;
                    evidence.waiting_entry_order.push(id.slot);
                }
                json!(["waiting-entered", zone.raw(), admission_sequence])
            }
            TrafficTransitionKind::ReservationAcquired { passage_range } => {
                json!(["reservation-acquired", range(h, passage_range)])
            }
            TrafficTransitionKind::ReservationReleased { passage_range } => {
                json!(["reservation-released", range(h, passage_range)])
            }
            TrafficTransitionKind::ConflictEntered { passage: p } => {
                json!(["conflict-entered", passage(h, p)])
            }
            TrafficTransitionKind::ConflictCleared { passage: p } => {
                json!(["conflict-cleared", passage(h, p)])
            }
            TrafficTransitionKind::ManeuverTraversalCompleted {
                maneuver_occurrence_index,
            } => {
                let role = h.plan.initial[id.tile as usize * 1_000 + id.slot as usize]
                    .role
                    .as_deref();
                let in_observation = h.in_step_observation(e.tick());
                let case = h.plan.case.as_str();
                let evidence = &mut h.evidence[id.tile as usize];
                if in_observation
                    && let Some(witness) = &mut evidence.right_of_way
                    && witness.subject == id
                    && witness.maneuver_occurrence == maneuver_occurrence_index
                {
                    witness
                        .passed
                        .get_or_insert((e.tick(), e.vehicle_update_sequence()));
                }
                if in_observation && case == "PERMISSIVE-LEFT" && role == Some("permissive-left") {
                    evidence.permissive_passes += 1;
                }
                if in_observation && case == "UNCONTROLLED-YIELD" {
                    if role.is_some_and(|role| role.starts_with("mainline-pulse-")) {
                        evidence.mainline_passes += 1;
                    } else if role == Some("yield-role") {
                        evidence.yield_passes += 1;
                    }
                }
                json!(["maneuver-completed", maneuver_occurrence_index])
            }
        };
        let a = e.anchor();
        let p = a.position();
        h.events.push(json!({"kind":"transition", "tick":e.tick(), "individual":id, "sequence":e.vehicle_update_sequence(),
            "route":h.route_key(a.route()), "occurrence":a.maneuver_occurrence_index(), "hop":a.hop(),
            "position":[p.route_edge_index(),p.progress_mm(),p.carry_um()], "detail":detail}));
    }
    Ok(())
}

pub(crate) fn status(value: VehicleStatus) -> u32 {
    match value {
        VehicleStatus::Active => 0,
        VehicleStatus::Parked => 1,
        VehicleStatus::Completed => 2,
    }
}

/// 按边分桶的车身不重叠检查：桶键升序、桶内按 (back, front, id) 排序，
/// 与原先全量 (edge, back, front, id) 排序后 windows(2) 扫描报同一对违例。
fn check_body_overlap(
    tick: u64,
    buckets: &mut FastMap<u32, Vec<(u64, u64, IndividualId)>>,
) -> Result<()> {
    let mut edges: Vec<u32> = buckets.keys().copied().collect();
    edges.sort_unstable();
    for edge in edges {
        let bucket = buckets.get_mut(&edge).expect("bucket key collected");
        bucket.sort_unstable();
        for pair in bucket.windows(2) {
            if pair[0].1 > pair[1].0 && pair[0].2 != pair[1].2 {
                return Err(invalid(format!(
                    "body overlap at tick {} on edge {}: {:?} and {:?}",
                    tick, edge, pair[0].2, pair[1].2
                )));
            }
        }
    }
    Ok(())
}

pub(crate) fn state(h: &mut Harness<'_>) -> Result<(String, (usize, usize, usize))> {
    let mut hash = Hash::default();
    hash.text("urban-observation-v2");
    hash.n(h.world.tick_index());
    hash.n(h.world.time_ms());
    hash.n(h.world.command_cursor());
    hash.n(h.world.event_cursor());
    let mut json_scratch = Vec::new();
    for bucket in h.scratch_body_buckets.values_mut() {
        bucket.clear();
    }
    let mut zone_owners = FastMap::default();
    for ((id, _, _), zone) in &h.claims {
        if zone_owners.insert(*zone, *id).is_some_and(|old| old != *id) {
            return Err(invalid("committed conflict claims are not exclusive"));
        }
        let individual = &h.individuals[id.tile as usize * 1_000 + id.slot as usize];
        if individual.id != *id
            || individual
                .handle
                .and_then(|handle| h.world.conflict_reservation(handle))
                .is_none()
        {
            return Err(invalid("claim has no current reservation owner"));
        }
    }
    let mut waiting = BTreeMap::<u32, u32>::new();
    let mut status_counts = (0_usize, 0_usize, 0_usize);
    let mut live = 0_usize;
    let mut bindings: FastMap<ParkingTarget, (u64, u64)> = FastMap::default();
    for i in 0..h.individuals.len() {
        // 生命周期比对（原 events() 尾部循环）：保持个体槽位序与事件内容不变，
        // events() 先返回，lifecycle 条目仍在 transitions 之后入列。
        if let Some(before) = h.step_before[i] {
            let after = h
                .world
                .vehicle(
                    h.individuals[i]
                        .handle
                        .expect("step cannot create an absent slot"),
                )
                .expect("live individual");
            let route = h.route_table.index(after.route());
            let first = before.route_edge_index() as usize;
            let last = after.route_edge_index() as usize;
            if before.status() == VehicleStatus::Active && first < last {
                let mut crossed = false;
                for hop in first..last {
                    if h.route_table.crosses_tile(route, hop) {
                        crossed = true;
                    }
                }
                if crossed {
                    h.individuals[i].crossed_tile = true;
                }
            }
            if before.status() != after.status() {
                let id = h.individuals[i].id;
                let crossed_tile = h.individuals[i].crossed_tile;
                h.events.push(json!({"kind":"lifecycle", "phase":"step", "tick":h.world.tick_index(), "individual":id,
                    "before":status(before.status()), "after":status(after.status()), "crossed_tile":crossed_tile}));
                if after.status() == VehicleStatus::Completed
                    && crossed_tile
                    && h.plan.window.contains_completed_step(h.world.tick_index())
                {
                    h.evidence[id.tile as usize].crossed_tile_completed += 1;
                }
            }
        }
        let individual = &h.individuals[i];
        hash.id(individual.id);
        let Some(handle) = individual.handle else {
            hash.n(0_u32);
            continue;
        };
        live += 1;
        hash.n(1_u32);
        let v = h
            .world
            .vehicle(handle)
            .ok_or_else(|| invalid("missing vehicle"))?;
        // counts() 的逐车部分（冷路径 counts() 仍供 install 与报告使用）。
        if h.slots.get(&handle) != Some(&i) {
            return Err(invalid("handle identity map differs"));
        }
        match v.status() {
            VehicleStatus::Active => status_counts.0 += 1,
            VehicleStatus::Parked => status_counts.1 += 1,
            VehicleStatus::Completed => status_counts.2 += 1,
        }
        let binding = h.world.parking_binding(handle);
        if !binding_matches_status(v.status(), binding) {
            return Err(invalid("parking binding and lifecycle disagree"));
        }
        accumulate_parking_binding(&h.world, &mut bindings, handle, binding)?;
        hash.n(v.profile().raw());
        hash.n(v.class().raw());
        hash.text(h.route_key(v.route()));
        for n in [
            v.route_edge_index(),
            v.progress_mm(),
            u32::from(v.carry_um()),
            v.speed_mm_s(),
            v.length_mm(),
            status(v.status()),
        ] {
            hash.n(n);
        }
        if let Some(t) = v.maneuver_traversal() {
            hash.n(1_u32);
            hash.text(h.route_key(t.route()));
            hash.n(t.maneuver_occurrence_index());
            let (tag, hop) = match t.phase() {
                ManeuverTraversalPhase::PreGate { next_gate_hop } => (0_u32, next_gate_hop),
                ManeuverTraversalPhase::Committed {
                    last_crossed_gate_hop,
                } => (1, last_crossed_gate_hop),
                ManeuverTraversalPhase::Waiting { release_gate_hop } => (2, release_gate_hop),
                ManeuverTraversalPhase::Clearing { admission_gate_hop } => (3, admission_gate_hop),
            };
            hash.n(tag);
            hash.n(hop);
        } else {
            hash.n(0_u32);
        }
        if let Some(m) = v.waiting_membership() {
            hash.n(1_u32);
            hash.n(m.waiting_zone().raw());
            hash.n(m.admission_sequence());
            hash.n(m.release_hop());
            *waiting.entry(m.waiting_zone().raw()).or_default() += 1;
        } else {
            hash.n(0_u32);
        }
        match binding {
            None => hash.n(0_u32),
            Some(ParkingBinding::Occupied(target)) => {
                hash.n(1_u32);
                hash.text(&h.target_keys[&target]);
            }
            Some(ParkingBinding::Reserved(r)) => {
                hash.n(2_u32);
                hash.text(&h.target_keys[&r.target()]);
                hash.text(h.route_key(r.route()));
                hash.n(r.entry_route_occurrence());
                hash.n(r.virtual_entry_selector().map_or(u32::MAX, |s| s.raw()));
            }
        }
        if let Some(r) = h.world.conflict_reservation(handle) {
            hash.n(1_u32);
            text_json(&mut hash, &mut json_scratch, &range(h, r.passage_range()))?;
            hash.id(h
                .stable_individual(r.downstream_owner())
                .ok_or_else(|| invalid("unknown claim owner"))?);
            hash.n(r.downstream_claim_count());
            hash.n(r.acquired_tick());
        } else {
            hash.n(0_u32);
        }
        if v.status() == VehicleStatus::Active {
            let route = h.route_table.index(v.route());
            let mut cursor = v.route_edge_index() as usize;
            let mut front = u64::from(v.progress_mm()) * 1_000 + u64::from(v.carry_um());
            let mut remaining = u64::from(v.length_mm()) * 1_000;
            if front > u64::from(h.route_table.edge_len_mm(route, cursor)) * 1_000 {
                return Err(invalid(format!(
                    "front outside current route edge: tick={} individual={:?} route={} occurrence={} edge={} progress_mm={} carry_um={} length_mm={} speed_mm_s={}",
                    h.world.tick_index(),
                    individual.id,
                    h.route_key(v.route()),
                    v.route_edge_index(),
                    h.route_table.edge_key(route, cursor),
                    v.progress_mm(),
                    v.carry_um(),
                    h.route_table.edge_len_mm(route, cursor),
                    v.speed_mm_s()
                )));
            }
            loop {
                let back = front.saturating_sub(remaining);
                if front > back {
                    h.scratch_body_buckets
                        .entry(h.route_table.edge_raw(route, cursor))
                        .or_default()
                        .push((back, front, individual.id));
                }
                remaining = remaining.saturating_sub(front);
                if remaining == 0 || cursor == 0 {
                    break;
                }
                cursor -= 1;
                front = u64::from(h.route_table.edge_len_mm(route, cursor)) * 1_000;
            }
        }
    }
    // counts() 的守恒汇总（随逐车循环累计完成，仍在同一拍报错）。
    if h.world.live_vehicles().len() != live || h.slots.len() != live {
        return Err(invalid("live identity conservation failed"));
    }
    let expected_live = i64::from(h.plan.individuals) + h.births as i64 - h.removals as i64;
    if expected_live < 0 || live != expected_live as usize {
        return Err(invalid("birth/removal identity conservation failed"));
    }
    check_body_overlap(h.world.tick_index(), &mut h.scratch_body_buckets)?;
    for raw in 0..h
        .artifacts
        .revision
        .identity()
        .entity_count(EntityKind::WaitingZone)
    {
        let zone = h
            .world
            .waiting_zone(WaitingZoneOrdinal::from_raw(raw))
            .ok_or_else(|| invalid("missing waiting zone"))?;
        if zone.occupancy() != waiting.get(&raw).copied().unwrap_or(0)
            || zone.occupancy() > zone.max_occupancy()
        {
            return Err(invalid("waiting capacity or membership differs"));
        }
        hash.n(raw);
        hash.n(zone.occupancy());
        hash.n(zone.next_admission_sequence());
    }
    for (group, aspect) in h.world.committed_signal_groups().as_slice() {
        hash.n(group.raw());
        hash.n(match aspect {
            SignalAspect::Red => 0_u32,
            SignalAspect::Yellow => 1,
            SignalAspect::Green => 2,
            _ => return Err(invalid("unsupported signal aspect")),
        });
    }
    hash.text("remaining-claims");
    for ((id, route, index), zone) in &h.claims {
        hash.id(*id);
        hash.text(h.route_table.key(*route));
        hash.n(*index);
        hash.n(*zone);
    }
    // parking_invariants() 的循环后部分（target 归属与设施计数守恒）。
    for (target, key) in &h.target_keys {
        let (reserved, occupied) = bindings.get(target).copied().unwrap_or_default();
        if let ParkingTarget::ExplicitSpace(space) = target
            && (reserved + occupied > 1
                || (reserved + occupied == 0
                    && h.world.parking_space_state(*space) != Some(ParkingSpaceState::Vacant)))
        {
            return Err(invalid(format!(
                "explicit capacity or ownership differs: {key}"
            )));
        }
    }
    for raw in 0..h
        .artifacts
        .revision
        .identity()
        .entity_count(EntityKind::ParkingFacility)
    {
        let facility = ParkingFacilityOrdinal::from_raw(raw);
        let definition = h
            .artifacts
            .revision
            .traffic()
            .relations()
            .parking_facility(facility)
            .ok_or_else(|| invalid("missing facility definition"))?;
        let explicit = definition
            .spaces()
            .iter()
            .fold((0, 0), |(reserved, occupied), space| {
                let count = bindings
                    .get(&ParkingTarget::ExplicitSpace(*space))
                    .copied()
                    .unwrap_or_default();
                (reserved + count.0, occupied + count.1)
            });
        let virtual_pool = bindings
            .get(&ParkingTarget::VirtualPool(facility))
            .copied()
            .unwrap_or_default();
        let counts = h
            .world
            .parking_facility_counts(facility)
            .ok_or_else(|| invalid("missing facility counts"))?;
        validate_facility_counts(counts, explicit, virtual_pool)?;
    }
    Ok((hash.finish(), status_counts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batched_observation_hash_preserves_the_exact_encoding() {
        assert_eq!(Hash::default().finish(), crate::sha256(&[]));
        for (numbers, text) in [
            (1, String::new()),
            (31, "短文本".into()),
            (32, "x".into()),
            (33, "y".repeat(255)),
            (64, "z".repeat(256)),
            (65, "a".repeat(257)),
            (3, "b".repeat(1_000)),
        ] {
            let mut hash = Hash::default();
            let mut bytes = Vec::new();
            for number in 0..numbers {
                hash.n(number as u64);
                bytes.extend_from_slice(&(number as u64).to_le_bytes());
            }
            hash.text(&text);
            bytes.extend_from_slice(&(text.len() as u64).to_le_bytes());
            bytes.extend_from_slice(text.as_bytes());
            let id = IndividualId {
                tile: 12,
                slot: 34,
                incarnation: 56,
            };
            hash.id(id);
            for number in [id.tile, id.slot, id.incarnation] {
                bytes.extend_from_slice(&u64::from(number).to_le_bytes());
            }
            hash.n(u64::MAX);
            bytes.extend_from_slice(&u64::MAX.to_le_bytes());
            assert_eq!(hash.finish(), crate::sha256(&bytes));
        }
    }

    #[test]
    fn text_json_matches_to_string_byte_for_byte() {
        fn check(scratch: &mut Vec<u8>, value: &impl serde::Serialize) {
            let mut expected = Hash::default();
            expected.text(&serde_json::to_string(value).unwrap());
            let mut actual = Hash::default();
            text_json(&mut actual, scratch, value).unwrap();
            assert_eq!(actual.finish(), expected.finish());
        }
        let mut scratch = Vec::new();
        let none: Option<Value> = None;
        check(&mut scratch, &none);
        check(
            &mut scratch,
            &Some(json!(["tile-0001.route.urban-crossing", 3, 1, 2, 4, 5, 6])),
        );
        check(&mut scratch, &json!(null));
        check(&mut scratch, &json!([]));
        check(&mut scratch, &json!({}));
        check(&mut scratch, &json!(0));
        check(&mut scratch, &json!("短文本\"转义\\\n"));
        check(&mut scratch, &json!([1, "two", null, {"nested": [true, false]}, ""]));
        check(&mut scratch, &json!({"unicode": "é中𐍈", "control": ""}));
        // 同一 scratch 连续复用（clear 语义）时仍逐字节一致。
        let mut expected = Hash::default();
        let mut actual = Hash::default();
        for value in [json!("x"), json!({"k": [1, 2]}), json!(null)] {
            expected.text(&serde_json::to_string(&value).unwrap());
            text_json(&mut actual, &mut scratch, &value).unwrap();
        }
        assert_eq!(actual.finish(), expected.finish());
    }

    #[test]
    #[ignore = "有界 release 编码对照，不代表城市运行耗时"]
    fn observation_hash_encoding_wall_clock() {
        fn scalar() -> String {
            let mut hash = Sha256::new();
            for index in 0..100_000_u64 {
                for value in [index, 1, 2, 3, 4, 5, 6, 7] {
                    hash.update(std::hint::black_box(value).to_le_bytes());
                }
                let text = "tile-0001.route.urban-crossing";
                hash.update((text.len() as u64).to_le_bytes());
                hash.update(text.as_bytes());
            }
            crate::hex(&hash.finalize())
        }
        fn batched() -> String {
            let mut hash = Hash::default();
            for index in 0..100_000_u64 {
                for value in [index, 1, 2, 3, 4, 5, 6, 7] {
                    hash.n(std::hint::black_box(value));
                }
                hash.text("tile-0001.route.urban-crossing");
            }
            hash.finish()
        }
        let expected = scalar();
        assert_eq!(batched(), expected);
        type HashRun = fn() -> String;
        for round in 0..6 {
            let candidates: [(&str, HashRun); 2] = if round % 2 == 0 {
                [("scalar", scalar), ("batched", batched)]
            } else {
                [("batched", batched), ("scalar", scalar)]
            };
            for (name, run) in candidates {
                let start = std::time::Instant::now();
                let digest = run();
                let elapsed = start.elapsed();
                assert_eq!(digest, expected);
                eprintln!(
                    "observation-encoding {round} {name}: {} ns {digest}",
                    elapsed.as_nanos()
                );
            }
        }
    }

    #[test]
    fn binding_status_matrix_matches_counts_semantics() {
        // Reserved 变体只能由 Runtime 内部构造；其三种组合与以下矩阵同属同一
        // matches! 模式表，且被十万辆短窗正常路径覆盖。
        let space = ParkingTarget::ExplicitSpace(ParkingSpaceOrdinal::from_raw(0));
        let occupied = Some(ParkingBinding::Occupied(space));
        assert!(binding_matches_status(VehicleStatus::Parked, occupied));
        assert!(!binding_matches_status(VehicleStatus::Active, occupied));
        assert!(!binding_matches_status(VehicleStatus::Completed, occupied));
        assert!(binding_matches_status(VehicleStatus::Active, None));
        assert!(binding_matches_status(VehicleStatus::Completed, None));
        assert!(!binding_matches_status(VehicleStatus::Parked, None));
    }

    #[test]
    fn bucketed_body_overlap_matches_global_sort() {
        fn id(tile: u32, slot: u32) -> IndividualId {
            IndividualId {
                tile,
                slot,
                incarnation: 0,
            }
        }
        fn reference(mut bodies: Vec<(u32, u64, u64, IndividualId)>) -> Option<String> {
            bodies.sort_unstable();
            for pair in bodies.windows(2) {
                if pair[0].0 == pair[1].0 && pair[0].2 > pair[1].1 && pair[0].3 != pair[1].3 {
                    return Some(format!(
                        "body overlap at tick 9 on edge {}: {:?} and {:?}",
                        pair[0].0, pair[0].3, pair[1].3
                    ));
                }
            }
            None
        }
        let cases: Vec<Vec<(u32, u64, u64, IndividualId)>> = vec![
            vec![],
            // 相接不重叠（front == back 不算）。
            vec![(5, 0, 100, id(0, 0)), (5, 100, 200, id(0, 1))],
            // 同边重叠 1mm。
            vec![(5, 0, 101, id(0, 0)), (5, 100, 200, id(0, 1))],
            // 同一个体的相邻段不报错。
            vec![(5, 0, 100, id(0, 0)), (5, 50, 150, id(0, 0))],
            // 两条边各有重叠：先报更小边号桶内的对。
            vec![
                (7, 0, 60, id(0, 3)),
                (3, 0, 50, id(0, 2)),
                (7, 30, 90, id(0, 4)),
                (3, 40, 80, id(0, 5)),
            ],
            // 跨边车身互不重叠。
            vec![(3, 0, 100, id(0, 6)), (7, 0, 100, id(0, 7))],
            // 同边三段：报排序后首个重叠对。
            vec![
                (5, 0, 100, id(0, 8)),
                (5, 90, 150, id(0, 9)),
                (5, 140, 200, id(0, 10)),
            ],
        ];
        for bodies in cases {
            let expected = reference(bodies.clone());
            let mut buckets: FastMap<u32, Vec<(u64, u64, IndividualId)>> = FastMap::default();
            for (edge, back, front, id) in bodies.iter().copied() {
                buckets.entry(edge).or_default().push((back, front, id));
            }
            let actual = check_body_overlap(9, &mut buckets).err().map(|e| e.to_string());
            assert_eq!(actual, expected, "bodies: {bodies:?}");
        }
    }

    #[test]
    fn rejects_wrong_explicit_totals_even_when_capacity_is_conserved() {
        let mut counts = ParkingFacilityCounts {
            explicit: ParkingPoolCounts {
                capacity: 2,
                reserved: 1,
                occupied: 1,
                vacant: 0,
            },
            virtual_pool: ParkingPoolCounts {
                capacity: 3,
                reserved: 0,
                occupied: 1,
                vacant: 2,
            },
            total: ParkingPoolCounts {
                capacity: 5,
                reserved: 1,
                occupied: 2,
                vacant: 2,
            },
        };
        assert!(validate_facility_counts(counts, (1, 1), (0, 1)).is_ok());
        counts.explicit.reserved = 0;
        counts.explicit.occupied = 2;
        counts.total.reserved = 0;
        counts.total.occupied = 3;
        assert!(
            validate_facility_counts(counts, (1, 1), (0, 1))
                .unwrap_err()
                .to_string()
                .contains("explicit facility binding count differs")
        );
    }
}
