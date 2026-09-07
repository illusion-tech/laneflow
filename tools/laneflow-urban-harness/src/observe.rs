use std::collections::{BTreeMap, HashMap};

use laneflow_runtime::*;
use laneflow_static_contract::{
    EntityKind, ManeuverGateOrdinal, ParkingFacilityOrdinal, SignalAspect, WaitingZoneOrdinal,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{Artifacts, Harness, IndividualId, Result, invalid};

// This encoding belongs to the harness v1 observation, not a Runtime wire format.
#[derive(Default)]
struct Hash(Sha256);
impl Hash {
    fn n(&mut self, value: impl Into<u64>) {
        self.0.update(value.into().to_le_bytes());
    }
    fn text(&mut self, text: &str) {
        self.n(text.len() as u64);
        self.0.update(text.as_bytes());
    }
    fn id(&mut self, id: IndividualId) {
        self.n(id.tile);
        self.n(id.slot);
        self.n(id.incarnation);
    }
    fn finish(self) -> String {
        crate::hex(&self.0.finalize())
    }
}

pub(crate) fn transition_gates(a: &Artifacts) -> HashMap<(u32, u32), (u32, u32)> {
    let mut result = HashMap::new();
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

pub(crate) fn counts(h: &Harness<'_>) -> Result<(usize, usize, usize)> {
    if h.world.live_vehicles().len() != h.individuals.len() || h.slots.len() != h.individuals.len()
    {
        return Err(invalid("live identity conservation failed"));
    }
    let mut counts = (0, 0, 0);
    for (index, individual) in h.individuals.iter().enumerate() {
        if h.slots.get(&individual.handle) != Some(&index) {
            return Err(invalid("handle identity map differs"));
        }
        let state = h
            .world
            .vehicle(individual.handle)
            .ok_or_else(|| invalid("missing live state"))?;
        match state.status() {
            VehicleStatus::Active => counts.0 += 1,
            VehicleStatus::Parked => counts.1 += 1,
            VehicleStatus::Completed => counts.2 += 1,
        }
        match (state.status(), h.world.parking_binding(individual.handle)) {
            (VehicleStatus::Parked, Some(ParkingBinding::Occupied(_)))
            | (VehicleStatus::Active, Some(ParkingBinding::Reserved(_)))
            | (VehicleStatus::Active | VehicleStatus::Completed, None) => {}
            _ => return Err(invalid("parking binding and lifecycle disagree")),
        }
    }
    Ok(counts)
}

pub(crate) fn parking_invariants(h: &Harness<'_>) -> Result<()> {
    let mut bindings: HashMap<ParkingTarget, (u64, u64)> = HashMap::new();
    for individual in &h.individuals {
        if let Some(binding) = h.world.parking_binding(individual.handle) {
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
                    ParkingSpaceState::Reserved(individual.handle)
                } else {
                    ParkingSpaceState::Occupied(individual.handle)
                };
                if h.world.parking_space_state(space) != Some(expected) {
                    return Err(invalid("space owner differs from binding"));
                }
            }
        }
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

pub(crate) fn red_waiters(h: &mut Harness<'_>) {
    if h.world.tick_index() < h.plan.window.warm_up_ticks {
        return;
    }
    let signals = h.world.committed_signal_groups();
    let red: HashMap<_, _> = signals
        .as_slice()
        .iter()
        .map(|(g, a)| (g.raw(), *a == SignalAspect::Red))
        .collect();
    let lengths = h.artifacts.revision.traffic().lane_lengths_millimetres();
    for (i, state) in h.step_before.iter().enumerate() {
        if state.status() != VehicleStatus::Active || state.speed_mm_s() != 0 {
            continue;
        }
        let route = h
            .world
            .route_edges(state.route())
            .expect("registered route");
        let current = state.route_edge_index() as usize;
        if current + 1 < route.len()
            && lengths[route[current].index()].saturating_sub(state.progress_mm()) <= 1
            && let Some(&(gate, group)) = h
                .transition_gates
                .get(&(route[current].raw(), route[current + 1].raw()))
            && red.get(&group) == Some(&true)
        {
            h.red_waiters.insert((h.individuals[i].id, gate));
        }
    }
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

fn passage(h: &Harness<'_>, p: ConflictPassageOccurrenceLocator) -> Value {
    json!([
        h.route_keys[&p.route()],
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
        h.route_keys[&r.route()],
        r.maneuver_occurrence_index(),
        r.admission_gate_hop(),
        r.first_conflict_occurrence_index(),
        r.passage_count()
    ])
}

pub(crate) fn events(h: &mut Harness<'_>) -> Result<()> {
    // Every decision, including NotEvaluated/NotRequired, contributes in public batch order.
    // Store its digest once per tick instead of millions of repetitive diagnostic rows.
    let mut decisions = Hash::default();
    decisions.text("waiting");
    for d in h.world.latest_waiting_decisions() {
        decisions.id(h
            .stable_individual(d.vehicle())
            .ok_or_else(|| invalid("unknown decision owner"))?);
        decisions.n(d.vehicle_update_sequence());
        decisions.text(&h.route_keys[&d.anchor().route()]);
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
        decisions.n(tag);
        decisions.n(reason);
    }
    decisions.text("conflict");
    for d in h.world.latest_conflict_decisions() {
        decisions.id(h
            .stable_individual(d.vehicle())
            .ok_or_else(|| invalid("unknown decision owner"))?);
        decisions.n(d.vehicle_update_sequence());
        decisions.text(&h.route_keys[&d.anchor().route()]);
        decisions.n(d.anchor().maneuver_occurrence_index());
        decisions.n(d.anchor().hop());
        decisions.text(&serde_json::to_string(&d.passage().map(|p| passage(h, p)))?);
        let (tag, reason) = match d.outcome() {
            ConflictDecisionOutcome::NotEvaluated => (0_u32, 0),
            ConflictDecisionOutcome::NotRequired => (1, 0),
            ConflictDecisionOutcome::Granted => (2, 0),
            ConflictDecisionOutcome::NoGrant(r) => (3, conflict_reason(r)),
        };
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
                            (id, h.route_keys[&r.route()].clone(), index),
                            p.address().zone().raw(),
                        )
                        .is_some()
                    {
                        return Err(invalid("duplicate committed claim"));
                    }
                }
            }
            TrafficTransitionKind::ConflictCleared { passage: p } => {
                if h.claims
                    .remove(&(
                        id,
                        h.route_keys[&p.route()].clone(),
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
            } => json!(["waiting-left", zone.raw(), admission_sequence]),
            TrafficTransitionKind::WaitingEntered {
                zone,
                admission_sequence,
            } => json!(["waiting-entered", zone.raw(), admission_sequence]),
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
            } => json!(["maneuver-completed", maneuver_occurrence_index]),
        };
        let a = e.anchor();
        let p = a.position();
        h.events.push(json!({"kind":"transition", "tick":e.tick(), "individual":id, "sequence":e.vehicle_update_sequence(),
            "route":h.route_keys[&a.route()], "occurrence":a.maneuver_occurrence_index(), "hop":a.hop(),
            "position":[p.route_edge_index(),p.progress_mm(),p.carry_um()], "detail":detail}));
    }
    for (i, before) in h.step_before.iter().enumerate() {
        let individual = &mut h.individuals[i];
        let after = h.world.vehicle(individual.handle).expect("live individual");
        let route = &h.route_edges[&after.route()];
        let first = before.route_edge_index() as usize;
        let last = after.route_edge_index() as usize;
        if before.status() == VehicleStatus::Active && first < last {
            for hop in first..last {
                if route[hop].split('.').next() != route[hop + 1].split('.').next() {
                    individual.crossed_tile = true;
                }
            }
        }
        if before.status() != after.status() {
            h.events.push(json!({"kind":"lifecycle", "phase":"step", "tick":h.world.tick_index(), "individual":individual.id,
                "before":status(before.status()), "after":status(after.status()), "crossed_tile":individual.crossed_tile}));
            if after.status() == VehicleStatus::Completed
                && individual.crossed_tile
                && h.world.tick_index() > h.plan.window.warm_up_ticks
            {
                h.evidence[individual.id.tile as usize].crossed_tile_completed += 1;
            }
        }
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

pub(crate) fn state(h: &Harness<'_>) -> Result<String> {
    let mut hash = Hash::default();
    hash.text("urban-observation-v1");
    hash.n(h.world.tick_index());
    hash.n(h.world.time_ms());
    hash.n(h.world.command_cursor());
    hash.n(h.world.event_cursor());
    let lengths = h.artifacts.revision.traffic().lane_lengths_millimetres();
    let mut bodies = Vec::new();
    let mut zone_owners = HashMap::new();
    for ((id, _, _), zone) in &h.claims {
        if zone_owners.insert(*zone, *id).is_some_and(|old| old != *id) {
            return Err(invalid("committed conflict claims are not exclusive"));
        }
        let individual = &h.individuals[id.tile as usize * 1_000 + id.slot as usize];
        if individual.id != *id || h.world.conflict_reservation(individual.handle).is_none() {
            return Err(invalid("claim has no current reservation owner"));
        }
    }
    let mut waiting = BTreeMap::<u32, u32>::new();
    for individual in &h.individuals {
        let v = h
            .world
            .vehicle(individual.handle)
            .ok_or_else(|| invalid("missing vehicle"))?;
        hash.id(individual.id);
        hash.n(v.profile().raw());
        hash.n(v.class().raw());
        hash.text(&h.route_keys[&v.route()]);
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
            hash.text(&h.route_keys[&t.route()]);
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
        match h.world.parking_binding(individual.handle) {
            None => hash.n(0_u32),
            Some(ParkingBinding::Occupied(target)) => {
                hash.n(1_u32);
                hash.text(&h.target_keys[&target]);
            }
            Some(ParkingBinding::Reserved(r)) => {
                hash.n(2_u32);
                hash.text(&h.target_keys[&r.target()]);
                hash.text(&h.route_keys[&r.route()]);
                hash.n(r.entry_route_occurrence());
                hash.n(r.virtual_entry_selector().map_or(u32::MAX, |s| s.raw()));
            }
        }
        if let Some(r) = h.world.conflict_reservation(individual.handle) {
            hash.n(1_u32);
            hash.text(&serde_json::to_string(&range(h, r.passage_range()))?);
            hash.id(h
                .stable_individual(r.downstream_owner())
                .ok_or_else(|| invalid("unknown claim owner"))?);
            hash.n(r.downstream_claim_count());
            hash.n(r.acquired_tick());
        } else {
            hash.n(0_u32);
        }
        if v.status() == VehicleStatus::Active {
            let route = h.world.route_edges(v.route()).expect("registered route");
            let mut cursor = v.route_edge_index() as usize;
            let mut front = u64::from(v.progress_mm()) * 1_000 + u64::from(v.carry_um());
            let mut remaining = u64::from(v.length_mm()) * 1_000;
            if front > u64::from(lengths[route[cursor].index()]) * 1_000 {
                return Err(invalid(format!(
                    "front outside current route edge: tick={} individual={:?} route={} occurrence={} edge={} progress_mm={} carry_um={} length_mm={} speed_mm_s={}",
                    h.world.tick_index(),
                    individual.id,
                    h.route_keys[&v.route()],
                    v.route_edge_index(),
                    h.route_edges[&v.route()][cursor],
                    v.progress_mm(),
                    v.carry_um(),
                    lengths[route[cursor].index()],
                    v.speed_mm_s()
                )));
            }
            loop {
                let back = front.saturating_sub(remaining);
                if front > back {
                    bodies.push((route[cursor].raw(), back, front, individual.id));
                }
                remaining = remaining.saturating_sub(front);
                if remaining == 0 || cursor == 0 {
                    break;
                }
                cursor -= 1;
                front = u64::from(lengths[route[cursor].index()]) * 1_000;
            }
        }
    }
    bodies.sort_unstable();
    for pair in bodies.windows(2) {
        if pair[0].0 == pair[1].0 && pair[0].2 > pair[1].1 && pair[0].3 != pair[1].3 {
            return Err(invalid(format!(
                "body overlap at tick {} on edge {}: {:?} and {:?}",
                h.world.tick_index(),
                pair[0].0,
                pair[0].3,
                pair[1].3
            )));
        }
    }
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
        hash.text(route);
        hash.n(*index);
        hash.n(*zone);
    }
    Ok(hash.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

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
