//! Private observer for the frozen #285 workload. It never changes TrafficWorld.
//! Body segments follow the public-state check used by the #544 urban harness;
//! no demand plan, route normalization, or traffic solver is reproduced here.

use std::collections::BTreeMap;

use bevy_ecs::resource::Resource;
use laneflow_runtime::{
    ConflictPassageRange, TrafficTransitionEvent, TrafficTransitionKind, TrafficWorld,
    VehicleState, VehicleStatus, WaitingDecisionOutcome, WaitingNoGrantReason,
    WaitingProjectionReason, WorldGeneration,
};
use laneflow_static_contract::{
    EntityKind, GateInterpretation, GateProhibition, ManeuverGateOrdinal, SignalAspect,
    SignalControllerOrdinal, WaitingZoneOrdinal,
};
use serde_json::{Value, json};

#[cfg(test)]
#[allow(dead_code)]
#[path = "junction_debug_scene.rs"]
mod scene;

type Check<T = ()> = Result<T, (&'static str, String)>;
type Body = (u32, u64, u64, usize);

fn ensure(ok: bool, kind: &'static str, message: impl FnOnce() -> String) -> Check {
    if ok { Ok(()) } else { Err((kind, message())) }
}

#[derive(Resource)]
pub struct Validation {
    previous: Vec<VehicleState>,
    current: Vec<VehicleState>,
    bodies: Vec<Body>,
    previous_bodies: Vec<Body>,
    expected_gates: Vec<(u32, u32, u32)>,
    actual_gates: Vec<(u32, u32, u32)>,
    signals: Vec<SignalAspect>,
    waiting: BTreeMap<usize, (u32, u64)>,
    reservations: BTreeMap<usize, ConflictPassageRange>,
    claims: BTreeMap<(usize, u32), (u32, bool)>,
    tick: u64,
    time: u64,
    generation: WorldGeneration,
    command_cursor: u64,
    checked_ticks: u64,
    checked_vehicle_rows: u64,
    checked_gate_crossings: u64,
    checked_events: u64,
    failure: Option<(&'static str, String)>,
}

impl Validation {
    pub fn new(world: &TrafficWorld) -> Check<Self> {
        let previous = world
            .live_vehicles()
            .iter()
            .map(|&id| {
                world.vehicle(id).ok_or_else(|| {
                    (
                        "identity_route_lifecycle",
                        "missing initial identity".into(),
                    )
                })
            })
            .collect::<Check<Vec<_>>>()?;
        let mut result = Self {
            current: Vec::with_capacity(previous.len()),
            bodies: Vec::with_capacity(previous.len() * 2),
            previous_bodies: Vec::with_capacity(previous.len() * 2),
            expected_gates: Vec::new(),
            actual_gates: Vec::new(),
            signals: vec![
                SignalAspect::Red;
                world
                    .traffic()
                    .entity_counts()
                    .count(EntityKind::SignalGroup) as usize
            ],
            waiting: BTreeMap::new(),
            reservations: BTreeMap::new(),
            claims: BTreeMap::new(),
            tick: world.tick_index(),
            time: world.time_ms(),
            generation: world.world_generation(),
            command_cursor: world.command_cursor(),
            previous,
            checked_ticks: 0,
            checked_vehicle_rows: 0,
            checked_gate_crossings: 0,
            checked_events: 0,
            failure: None,
        };
        result.check_signals(world)?;
        result.check_states(world, false)?;
        result.previous_bodies.clone_from(&result.bodies);
        Ok(result)
    }

    pub fn check(&mut self, world: &TrafficWorld) -> Check {
        let result = self.check_step(world);
        if let Err(error) = &result {
            self.failure = Some(error.clone());
        }
        result
    }

    fn check_step(&mut self, world: &TrafficWorld) -> Check {
        let delta = world.config().fixed_delta_time_ms();
        ensure(
            world.tick_index() == self.tick + 1 && world.time_ms() == self.time + delta,
            "tick_time",
            || "successful step did not advance exactly one fixed quantum".into(),
        )?;
        self.check_states(world, true)?;
        self.check_events(world)?;
        self.check_signals(world)?;
        self.previous.clone_from(&self.current);
        self.previous_bodies.clone_from(&self.bodies);
        self.tick = world.tick_index();
        self.time = world.time_ms();
        self.checked_ticks += 1;
        self.checked_vehicle_rows += self.current.len() as u64;
        Ok(())
    }

    fn check_states(&mut self, world: &TrafficWorld, stepped: bool) -> Check {
        ensure(
            world.world_generation() == self.generation
                && world.command_cursor() == self.command_cursor
                && world.live_vehicles().len() == self.previous.len(),
            "identity_route_lifecycle",
            || "frozen workload identity, generation, or command stream changed".into(),
        )?;
        self.current.clear();
        self.bodies.clear();
        self.expected_gates.clear();
        let traffic = world.traffic();
        let lengths = traffic.lane_lengths_millimetres();
        for (index, (&handle, &before)) in
            world.live_vehicles().iter().zip(&self.previous).enumerate()
        {
            let state = world
                .vehicle(handle)
                .ok_or_else(|| ("identity_route_lifecycle", "missing current vehicle".into()))?;
            ensure(
                handle == before.handle()
                    && state.route() == before.route()
                    && state.profile() == before.profile()
                    && state.class() == before.class()
                    && state.length_mm() == before.length_mm()
                    && state.status() == VehicleStatus::Active,
                "identity_route_lifecycle",
                || format!("identity/route/lifecycle changed for {handle:?}"),
            )?;
            // The frozen command plan has no parking, replacement, or despawn operations.
            ensure(
                world.parking_binding(handle).is_none(),
                "parking_binding",
                || format!("unexpected parking binding for {handle:?}"),
            )?;
            let route = world
                .route_edges(state.route())
                .ok_or_else(|| ("identity_route_lifecycle", "route missing".into()))?;
            let cursor = state.route_edge_index() as usize;
            let edge = route.get(cursor).ok_or_else(|| {
                (
                    "identity_route_lifecycle",
                    "route occurrence out of range".into(),
                )
            })?;
            // Committed geometry is integer millimetres. carry_um is an
            // uncommitted remainder and is legitimately cleared by a hard stop.
            let front = u64::from(state.progress_mm()) * 1000;
            ensure(
                state.carry_um() < 1000
                    && front <= u64::from(lengths[edge.index()]) * 1000
                    && state.speed_mm_s()
                        <= traffic.lane_speed_limits_millimetres_per_second()[edge.index()],
                "numeric_geometry",
                || format!("speed/progress outside current edge for {handle:?}"),
            )?;
            if stepped {
                ensure(
                    (state.route_edge_index(), front)
                        >= (
                            before.route_edge_index(),
                            u64::from(before.progress_mm()) * 1000,
                        ),
                    "identity_route_lifecycle",
                    || {
                        format!(
                            "route progress went backwards for {handle:?}: before={before:?}, after={state:?}"
                        )
                    },
                )?;
                let acceleration = (f64::from(state.speed_mm_s()) - f64::from(before.speed_mm_s()))
                    / delta_seconds(world);
                ensure(acceleration.is_finite(), "numeric_geometry", || {
                    "nonfinite derived acceleration".into()
                })?;
                for hop in before.route_edge_index()..state.route_edge_index() {
                    if let Some(gate) = world.route_gate(state.route(), hop) {
                        self.expected_gates
                            .push((index as u32, hop, gate.gate().raw()));
                        self.check_gate(world, gate.gate(), state)?;
                    }
                }
            }
            append_body(
                &mut self.bodies,
                route,
                lengths,
                cursor,
                front,
                u64::from(state.length_mm()) * 1000,
                index,
            );
            self.current.push(state);
        }
        if let Some((left, right)) = overlapping_bodies(&mut self.bodies) {
            return Err((
                "overlap",
                format!(
                    "physical segments {left:?}, {right:?}; left before={:?} after={:?}; right before={:?} after={:?}",
                    self.previous[left.3],
                    self.current[left.3],
                    self.previous[right.3],
                    self.current[right.3]
                ),
            ));
        }
        if stepped {
            for (index, (&before, &after)) in self.previous.iter().zip(&self.current).enumerate() {
                let minimum = u64::from(
                    traffic
                        .relations()
                        .vehicle_profile(after.profile())
                        .ok_or_else(|| ("minimum_gap", "profile missing".into()))?
                        .min_gap_mm(),
                ) * 1000;
                let route = world
                    .route_edges(after.route())
                    .ok_or_else(|| ("minimum_gap", "route missing".into()))?;
                let before_gap = body_gap(
                    &self.previous_bodies,
                    route,
                    lengths,
                    before.route_edge_index() as usize,
                    u64::from(before.progress_mm()) * 1000,
                    index,
                    minimum,
                );
                let after_gap = body_gap(
                    &self.bodies,
                    route,
                    lengths,
                    after.route_edge_index() as usize,
                    u64::from(after.progress_mm()) * 1000,
                    index,
                    minimum,
                );
                ensure(
                    minimum_gap_preserved(before_gap, after_gap, minimum),
                    "minimum_gap",
                    || {
                        format!(
                            "vehicle {index}: previous gap={before_gap}, current gap={after_gap}, profile minimum={minimum} (um); before={before:?}, after={after:?}"
                        )
                    },
                )?;
            }
        }
        Ok(())
    }

    fn check_gate(
        &self,
        world: &TrafficWorld,
        gate: ManeuverGateOrdinal,
        state: VehicleState,
    ) -> Check {
        let declaration = world
            .traffic()
            .relations()
            .maneuver_gate(gate)
            .ok_or_else(|| ("signal_stop_line", "missing crossed gate".into()))?;
        let rule = world
            .policy()
            .and_then(|policy| {
                policy
                    .gate_classes(gate)
                    .iter()
                    .find(|rule| rule.class() == state.class())
                    .copied()
            })
            .ok_or_else(|| {
                (
                    "signal_stop_line",
                    "crossed gate has no bound policy".into(),
                )
            })?;
        let aspect = declaration
            .signal_group()
            .map(|group| self.signals[group.index()]);
        ensure(
            gate_allows_crossing(rule.interpretation(), rule.prohibition(), aspect),
            "signal_stop_line",
            || {
                format!(
                    "vehicle {:?} crossed gate {} against snapshot(T) indication {aspect:?}",
                    state.handle(),
                    gate.raw()
                )
            },
        )
    }

    fn check_signals(&mut self, world: &TrafficWorld) -> Check {
        self.signals.fill(SignalAspect::Red);
        let relations = world.traffic().relations();
        for raw in 0..world
            .traffic()
            .entity_counts()
            .count(EntityKind::SignalController)
        {
            let controller = relations
                .signal_controller(SignalControllerOrdinal::from_raw(raw))
                .ok_or_else(|| ("signal_authority", "missing controller".into()))?;
            let position = ((u128::from(world.time_ms()) + u128::from(controller.offset_ms()))
                % u128::from(controller.cycle_ms())) as u64;
            let mut elapsed = 0;
            for &phase in controller.phases() {
                let phase = relations
                    .signal_phase(phase)
                    .ok_or_else(|| ("signal_authority", "missing phase".into()))?;
                elapsed += phase.duration_ms();
                if position < elapsed {
                    for (group, aspect) in phase.states() {
                        self.signals[group.index()] = aspect;
                    }
                    break;
                }
            }
        }
        let actual = world.committed_signal_groups();
        ensure(
            actual.as_slice().len() == self.signals.len()
                && actual
                    .as_slice()
                    .iter()
                    .enumerate()
                    .all(|(index, (group, aspect))| {
                        group.index() == index && *aspect == self.signals[index]
                    }),
            "signal_authority",
            || "committed signal groups differ from the frozen phase program".into(),
        )
    }

    fn check_events(&mut self, world: &TrafficWorld) -> Check {
        self.actual_gates.clear();
        let mut previous_key = None;
        // Keep tick-start and newly acquired owners even after release events:
        // committed resources cannot be reused by another owner in this tick.
        let mut held_this_tick = BTreeMap::new();
        for (&(owner, _), &(zone, _)) in &self.claims {
            hold_zone(&mut held_this_tick, zone, owner)?;
        }
        for &event in world.latest_transition_events() {
            let index = event.vehicle_update_sequence() as usize;
            let state = self
                .current
                .get(index)
                .ok_or_else(|| ("event_causality", "event sequence has no vehicle".into()))?;
            let anchor = event.anchor();
            let route = world
                .route_edges(state.route())
                .ok_or_else(|| ("event_causality", "event route missing".into()))?;
            ensure(
                event.tick() == world.tick_index()
                    && event.vehicle() == state.handle()
                    && anchor.route() == state.route()
                    && (anchor.hop() as usize) < route.len() - 1
                    && (anchor.position().route_edge_index() as usize) < route.len(),
                "event_causality",
                || format!("event identity/time/route mismatch: {event:?}"),
            )?;
            let key = event_key(event);
            ensure(
                previous_key.as_ref().is_none_or(|previous| previous < &key),
                "event_order",
                || format!("duplicate or out-of-order event: {event:?}"),
            )?;
            previous_key = Some(key);
            let handle = index;
            match event.kind() {
                TrafficTransitionKind::GateCrossed { gate } => {
                    self.actual_gates
                        .push((index as u32, anchor.hop(), gate.raw()))
                }
                TrafficTransitionKind::WaitingEntered {
                    zone,
                    admission_sequence,
                } => {
                    ensure(
                        self.waiting
                            .insert(handle, (zone.raw(), admission_sequence))
                            .is_none(),
                        "event_causality",
                        || "waiting entered twice without leaving".into(),
                    )?;
                }
                TrafficTransitionKind::WaitingLeft {
                    zone,
                    admission_sequence,
                } => {
                    ensure(
                        self.waiting.remove(&handle) == Some((zone.raw(), admission_sequence)),
                        "event_causality",
                        || "waiting left without matching membership".into(),
                    )?;
                }
                TrafficTransitionKind::ReservationAcquired { passage_range } => {
                    ensure(
                        self.reservations.insert(handle, passage_range).is_none(),
                        "event_causality",
                        || "reservation acquired twice without release".into(),
                    )?;
                    for occurrence in passage_range.first_conflict_occurrence_index()
                        ..passage_range.first_conflict_occurrence_index()
                            + passage_range.passage_count()
                    {
                        let passage = world
                            .conflict_passage_occurrence_locator(passage_range.route(), occurrence)
                            .ok_or_else(|| {
                                ("event_causality", "reservation passage missing".into())
                            })?;
                        hold_zone(&mut held_this_tick, passage.address().zone().raw(), handle)?;
                        ensure(
                            self.claims
                                .insert(
                                    (handle, occurrence),
                                    (passage.address().zone().raw(), false),
                                )
                                .is_none(),
                            "event_causality",
                            || "duplicate passage claim".into(),
                        )?;
                    }
                }
                TrafficTransitionKind::ConflictEntered { passage } => {
                    let claim = self
                        .claims
                        .get_mut(&(handle, passage.conflict_occurrence_index()))
                        .ok_or_else(|| {
                            ("event_causality", "conflict entered without claim".into())
                        })?;
                    ensure(
                        claim.0 == passage.address().zone().raw() && !claim.1,
                        "event_causality",
                        || "conflict entry mismatches claim".into(),
                    )?;
                    claim.1 = true;
                }
                TrafficTransitionKind::ConflictCleared { passage } => {
                    ensure(
                        self.claims
                            .remove(&(handle, passage.conflict_occurrence_index()))
                            == Some((passage.address().zone().raw(), true)),
                        "event_causality",
                        || "conflict cleared without prior entry".into(),
                    )?;
                }
                TrafficTransitionKind::ReservationReleased { passage_range } => {
                    ensure(
                        self.reservations.remove(&handle) == Some(passage_range)
                            && !self.claims.keys().any(|(owner, _)| *owner == handle),
                        "event_causality",
                        || "reservation released before claims cleared".into(),
                    )?;
                }
                TrafficTransitionKind::ProjectionApplied { zone, reason } => {
                    self.check_projection(world, event, zone, reason)?;
                }
                TrafficTransitionKind::ManeuverTraversalCompleted {
                    maneuver_occurrence_index,
                } => {
                    ensure(
                        maneuver_occurrence_index == anchor.maneuver_occurrence_index(),
                        "event_causality",
                        || "completed occurrence differs from anchor".into(),
                    )?;
                }
            }
        }
        ensure(
            self.actual_gates == self.expected_gates,
            "event_causality",
            || "gate event batch differs from actual route crossings".into(),
        )?;
        for &(owner, _) in self.claims.keys() {
            ensure(
                self.reservations.contains_key(&owner),
                "event_causality",
                || "claim has no reservation owner".into(),
            )?;
        }
        for (index, state) in self.current.iter().enumerate() {
            let waiting = state.waiting_membership().map(|membership| {
                (
                    membership.waiting_zone().raw(),
                    membership.admission_sequence(),
                )
            });
            ensure(
                self.waiting.get(&index).copied() == waiting
                    && self.reservations.get(&index).copied()
                        == world
                            .conflict_reservation(state.handle())
                            .map(|reservation| reservation.passage_range()),
                "event_causality",
                || {
                    format!(
                        "events do not reconstruct current resources for {:?}",
                        state.handle()
                    )
                },
            )?;
        }
        self.checked_gate_crossings += self.actual_gates.len() as u64;
        self.checked_events += world.latest_transition_events().len() as u64;
        Ok(())
    }

    fn check_projection(
        &self,
        world: &TrafficWorld,
        event: TrafficTransitionEvent,
        zone: WaitingZoneOrdinal,
        reason: WaitingProjectionReason,
    ) -> Check {
        let index = event.vehicle_update_sequence() as usize;
        let before = self.previous[index];
        let after = self.current[index];
        let anchor = event.anchor();
        let boundary = world
            .route_gate(anchor.route(), anchor.hop())
            .ok_or_else(|| ("event_causality", "projection has no gate".into()))?;
        let waiting = world
            .traffic()
            .relations()
            .waiting_zone(zone)
            .ok_or_else(|| ("event_causality", "projection has no waiting zone".into()))?;
        ensure(
            waiting.entry_gate() == boundary.gate()
                && (before.route_edge_index(), before.progress_mm())
                    < (anchor.hop(), boundary.progress_mm())
                && (after.route_edge_index(), after.progress_mm())
                    == (anchor.hop(), boundary.progress_mm())
                && after.speed_mm_s() == 0
                && after.carry_um() == 0,
            "event_causality",
            || "projection is not the first hard contact with its waiting entry".into(),
        )?;
        let expected = world
            .latest_waiting_decisions()
            .iter()
            .find_map(|decision| {
                if decision.vehicle() != event.vehicle()
                    || decision.anchor().route() != anchor.route()
                    || (decision.anchor().hop() == anchor.hop() && decision.zone() != Some(zone))
                {
                    return None;
                }
                projection_reason(decision.outcome(), decision.anchor().hop(), anchor.hop())
            });
        ensure(expected == Some(reason), "event_causality", || {
            format!(
                "projection reason {reason:?} differs from committed admission/route boundary {expected:?}"
            )
        })
    }

    pub fn report(&self) -> Value {
        let mut violations = json!({"overlap":0,"minimum_gap":0,"signal_stop_line":0,"numeric_geometry":0,
            "identity_route_lifecycle":0,"parking_binding":0,"signal_authority":0,"tick_time":0,
            "event_order":0,"event_causality":0,"conflict_exclusivity":0});
        if let Some((kind, _)) = &self.failure {
            violations[*kind] = json!(1);
        }
        json!({"schema":"junction-scale-validation-v1", "checked_ticks":self.checked_ticks,
            "checked_vehicle_rows":self.checked_vehicle_rows,"checked_gate_crossings":self.checked_gate_crossings,
            "checked_events":self.checked_events,"violations":violations,"failure":self.failure,
            "scope":"every warmup and observation tick of the frozen Active-only, no-parking-command plan",
            "failure_retry_and_lifecycle_commands":"not exercised by this plan; separate finite acceptance matrix"})
    }
}

fn delta_seconds(world: &TrafficWorld) -> f64 {
    world.config().fixed_delta_time_ms() as f64 / 1000.0
}

fn projection_reason(
    outcome: WaitingDecisionOutcome,
    decision_hop: u32,
    contact_hop: u32,
) -> Option<WaitingProjectionReason> {
    match outcome {
        WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::Capacity)
            if decision_hop == contact_hop =>
        {
            Some(WaitingProjectionReason::Capacity)
        }
        WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::PhysicalStorage)
            if decision_hop == contact_hop =>
        {
            Some(WaitingProjectionReason::PhysicalStorage)
        }
        WaitingDecisionOutcome::Granted if decision_hop < contact_hop => {
            Some(WaitingProjectionReason::EvaluationHorizon)
        }
        _ => None,
    }
}

fn hold_zone(held: &mut BTreeMap<u32, usize>, zone: u32, owner: usize) -> Check {
    ensure(
        held.get(&zone).is_none_or(|previous| *previous == owner),
        "conflict_exclusivity",
        || format!("conflict zone {zone} reused by owner {owner} within one tick"),
    )?;
    held.insert(zone, owner);
    Ok(())
}

fn minimum_gap_preserved(before: u64, after: u64, minimum: u64) -> bool {
    after >= before.min(minimum)
}

// Observe only the short route interval in which the profile minimum can be
// violated. Body buckets are sorted physical intervals; no motion is predicted.
#[allow(clippy::too_many_arguments)]
fn body_gap(
    bodies: &[Body],
    route: &[laneflow_static_contract::LaneEdgeOrdinal],
    lengths: &[u32],
    cursor: usize,
    front: u64,
    owner: usize,
    maximum: u64,
) -> u64 {
    let mut best = maximum;
    let mut base = 0;
    for (index, edge) in route.iter().enumerate().skip(cursor) {
        if base > best {
            break;
        }
        let lower = if index == cursor {
            front.saturating_sub(u64::from(laneflow_static_contract::MAX_VEHICLE_LENGTH_MM) * 1000)
        } else {
            0
        };
        let first = bodies.partition_point(|body| (body.0, body.1) < (edge.raw(), lower));
        for body in &bodies[first..] {
            if body.0 != edge.raw() {
                break;
            }
            let gap = if index == cursor {
                body.1.saturating_sub(front)
            } else {
                base + body.1
            };
            if gap > best {
                break;
            }
            if body.3 == owner || (index == cursor && body.2 <= front) {
                continue;
            }
            best = best.min(gap);
        }
        let length = u64::from(lengths[edge.index()]) * 1000;
        base += if index == cursor {
            length - front
        } else {
            length
        };
    }
    best
}

fn append_body(
    bodies: &mut Vec<Body>,
    route: &[laneflow_static_contract::LaneEdgeOrdinal],
    lengths: &[u32],
    mut cursor: usize,
    mut front: u64,
    mut remaining: u64,
    identity: usize,
) {
    // Two positive-length vehicles may not share the same zero-progress
    // front bumper, even when their bodies remain on different incoming edges.
    if front == 0 && remaining > 0 {
        bodies.push((route[cursor].raw(), 0, 0, identity));
    }
    loop {
        let back = front.saturating_sub(remaining);
        if front > back {
            bodies.push((route[cursor].raw(), back, front, identity));
        }
        remaining = remaining.saturating_sub(front);
        if remaining == 0 || cursor == 0 {
            break;
        }
        cursor -= 1;
        front = u64::from(lengths[route[cursor].index()]) * 1000;
    }
}

fn overlapping_bodies(bodies: &mut [Body]) -> Option<(Body, Body)> {
    bodies.sort_unstable();
    let mut farthest = bodies.first().copied()?;
    for &body in bodies.iter().skip(1) {
        let same_entry_point = farthest.1 == farthest.2 && body.1 == body.2 && farthest.1 == body.1;
        if farthest.0 == body.0 && (farthest.2 > body.1 || same_entry_point) && farthest.3 != body.3
        {
            return Some((farthest, body));
        }
        if farthest.0 != body.0 || body.2 > farthest.2 {
            farthest = body;
        }
    }
    None
}

fn gate_allows_crossing(
    interpretation: GateInterpretation,
    prohibition: GateProhibition,
    aspect: Option<SignalAspect>,
) -> bool {
    if prohibition == GateProhibition::Always
        || (prohibition == GateProhibition::OnRed && aspect == Some(SignalAspect::Red))
    {
        return false;
    }
    match interpretation {
        GateInterpretation::Uncontrolled => {
            aspect.is_none() && prohibition != GateProhibition::OnRed
        }
        GateInterpretation::CnCircularRightTurn => {
            matches!(aspect, Some(SignalAspect::Red | SignalAspect::Green))
        }
        _ => aspect == Some(SignalAspect::Green),
    }
}

type EventKey = (u32, (u32, u32, u16), (u8, u32, u32, u32), u32, u32);

fn event_key(event: TrafficTransitionEvent) -> EventKey {
    let detail = match event.kind() {
        TrafficTransitionKind::ProjectionApplied { zone, .. } => (0, zone.raw(), 0, 0),
        TrafficTransitionKind::GateCrossed { gate } => (1, gate.raw(), 0, 0),
        TrafficTransitionKind::WaitingLeft { zone, .. } => (2, zone.raw(), 0, 0),
        TrafficTransitionKind::WaitingEntered { zone, .. } => (3, zone.raw(), 0, 0),
        TrafficTransitionKind::ReservationAcquired { .. } => (4, 0, 0, 0),
        TrafficTransitionKind::ConflictEntered { passage }
        | TrafficTransitionKind::ConflictCleared { passage } => (
            if matches!(event.kind(), TrafficTransitionKind::ConflictEntered { .. }) {
                5
            } else {
                6
            },
            passage.address().zone().raw(),
            passage.address().stream().raw(),
            passage.address().passage_local_index(),
        ),
        TrafficTransitionKind::ReservationReleased { .. } => (7, 0, 0, 0),
        TrafficTransitionKind::ManeuverTraversalCompleted { .. } => (8, 0, 0, 0),
    };
    let anchor = event.anchor();
    let position = anchor.position();
    (
        event.vehicle_update_sequence(),
        (
            position.route_edge_index(),
            position.progress_mm(),
            position.carry_um(),
        ),
        detail,
        anchor.maneuver_occurrence_index(),
        anchor.hop(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimum_gap_rejects_contraction_without_overlap_but_preserves_initial_short_gap() {
        use laneflow_static_contract::LaneEdgeOrdinal;
        let route = [LaneEdgeOrdinal::from_raw(0), LaneEdgeOrdinal::from_raw(1)];
        let lengths = [10_000, 10_000];
        let before = body_gap(
            &[(1, 1_000_000, 5_500_000, 1)],
            &route,
            &lengths,
            0,
            9_000_000,
            0,
            2_000_000,
        );
        let after = body_gap(
            &[(1, 999_000, 5_499_000, 1)],
            &route,
            &lengths,
            0,
            9_000_000,
            0,
            2_000_000,
        );
        assert_eq!(before, 2_000_000);
        assert_eq!(after, 1_999_000);
        assert!(!minimum_gap_preserved(before, after, 2_000_000));
        assert!(minimum_gap_preserved(1_000_000, 1_000_000, 2_000_000));
        assert!(!minimum_gap_preserved(1_000_000, 999_000, 2_000_000));
    }

    #[test]
    fn projection_reason_requires_the_matching_decision_and_contact_hop() {
        let capacity = WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::Capacity);
        let storage = WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::PhysicalStorage);
        assert_eq!(
            projection_reason(capacity, 2, 2),
            Some(WaitingProjectionReason::Capacity)
        );
        assert_ne!(
            projection_reason(storage, 2, 2),
            Some(WaitingProjectionReason::Capacity)
        );
        assert_eq!(projection_reason(capacity, 2, 3), None);
        assert_eq!(
            projection_reason(WaitingDecisionOutcome::Granted, 2, 3),
            Some(WaitingProjectionReason::EvaluationHorizon)
        );
        assert_eq!(
            projection_reason(WaitingDecisionOutcome::Granted, 2, 2),
            None
        );
    }

    #[test]
    fn a_released_claim_still_blocks_another_owner_during_the_same_tick() {
        let mut held = BTreeMap::new();
        hold_zone(&mut held, 4, 10).unwrap();
        // A release removes the persistent claim, not this per-tick history.
        hold_zone(&mut held, 4, 10).unwrap();
        assert_eq!(
            hold_zone(&mut held, 4, 11).unwrap_err().0,
            "conflict_exclusivity"
        );
        held.clear();
        hold_zone(&mut held, 4, 11).unwrap();
    }

    #[test]
    fn overlap_check_rejects_intersection_and_accepts_exact_body_boundary() {
        assert!(overlapping_bodies(&mut [(0, 0, 4500, 0), (0, 4500, 9000, 1)]).is_none());
        assert!(overlapping_bodies(&mut [(0, 0, 4500, 0), (0, 4499, 9000, 1)]).is_some());
        assert!(overlapping_bodies(&mut [(0, 0, 0, 0), (0, 0, 0, 1)]).is_some());
        assert!(overlapping_bodies(&mut [(0, 0, 0, 0), (0, 0, 4500, 1)]).is_none());
    }

    #[test]
    fn signal_check_honors_red_right_turn_and_explicit_prohibition() {
        assert!(!gate_allows_crossing(
            GateInterpretation::ProtectedGroup,
            GateProhibition::None,
            Some(SignalAspect::Red)
        ));
        assert!(gate_allows_crossing(
            GateInterpretation::CnCircularRightTurn,
            GateProhibition::None,
            Some(SignalAspect::Red)
        ));
        assert!(!gate_allows_crossing(
            GateInterpretation::CnCircularRightTurn,
            GateProhibition::OnRed,
            Some(SignalAspect::Red)
        ));
        assert!(!gate_allows_crossing(
            GateInterpretation::CnCircularRightTurn,
            GateProhibition::None,
            Some(SignalAspect::Yellow)
        ));
    }

    #[test]
    fn live_checker_rejects_missing_events_bad_clock_and_changed_lifecycle() {
        use bevy_app::App;
        use bevy_time::{TimePlugin, TimeUpdateStrategy};
        use laneflow_bevy::{LaneFlowPlugin, LaneFlowSession};
        use std::time::Duration;
        let mut app = App::new();
        app.add_plugins((TimePlugin, LaneFlowPlugin))
            .insert_resource(scene::build().unwrap().session)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
        app.update();
        let mut validation =
            Validation::new(app.world().resource::<LaneFlowSession>().world()).unwrap();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            16,
        )));
        app.update();
        let session = app.world().resource::<LaneFlowSession>();
        validation.check(session.world()).unwrap();
        assert_eq!(validation.report()["checked_ticks"], 1);
        validation.expected_gates.push((0, 0, 0));
        assert_eq!(
            validation.check_events(session.world()).unwrap_err().0,
            "event_causality"
        );
        validation.expected_gates.clear();
        assert_eq!(
            validation.check(session.world()).unwrap_err().0,
            "tick_time"
        );
        assert_eq!(validation.report()["violations"]["tick_time"], 1);

        let mut validation = Validation::new(session.world()).unwrap();
        let handle = session.world().live_vehicles()[0];
        laneflow_bevy::despawn_vehicle(app.world_mut(), handle).unwrap();
        app.update();
        let session = app.world().resource::<LaneFlowSession>();
        assert_eq!(
            validation.check(session.world()).unwrap_err().0,
            "identity_route_lifecycle"
        );
    }
}
