use std::collections::{BTreeMap, BTreeSet, HashMap};

use laneflow_runtime::*;
use laneflow_static_contract::{
    ParkingFacilityId, ParkingSpaceId, VehicleProfileId, VehicleProfileOrdinal,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Artifacts, ResolvedPlan, Result, checked, invalid, observe, sha256};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IndividualId {
    pub tile: u32,
    pub slot: u32,
    pub incarnation: u32,
}

pub(crate) struct Individual {
    pub id: IndividualId,
    pub handle: Option<VehicleHandle>,
    pub crossed_tile: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TickRecord {
    pub tick: u64,
    pub time_ms: u64,
    pub domain: String,
    #[serde(rename = "N_individual")]
    pub live: usize,
    #[serde(rename = "N_active")]
    pub active: usize,
    pub parked: usize,
    pub completed: usize,
    #[serde(rename = "N_intent")]
    pub intent: usize,
    pub intent_basis: String,
    #[serde(rename = "N_presented")]
    pub presented: usize,
    #[serde(rename = "N_aggregate_records")]
    pub aggregate_records: usize,
    #[serde(rename = "N_aggregate_equivalent")]
    pub aggregate_equivalent: usize,
    pub future_departures: usize,
    pub pending_departures: usize,
    pub exhausted_departures: usize,
    pub command_cursor: u64,
    pub event_cursor: u64,
    pub state_digest: String,
    pub event_digest: String,
    pub commands_digest: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TileEvidence {
    pub crossed_tile_completed: u64,
    pub red_wait_then_crossed: u64,
    pub leaves: u64,
    pub explicit_parks: u64,
    pub virtual_parks: u64,
    pub planned_east: u64,
    pub planned_west: u64,
    pub admitted_east: u64,
    pub admitted_west: u64,
    pub garage_exit_0: u64,
    pub garage_exit_1: u64,
    pub safe_leave_rejections: u64,
    pub retried_leave_successes: u64,
    pub exclusive_rejections: u64,
    pub full_rejections: u64,
    pub waiting_entries: u64,
    pub waiting_capacity_rejections: u64,
    pub waiting_storage_rejections: u64,
    pub waiting_releases: u64,
    pub waiting_entry_order: Vec<u32>,
    pub waiting_release_order: Vec<u32>,
    pub permissive_no_grants: u64,
    pub permissive_grants: u64,
    pub permissive_passes: u64,
    pub mainline_passes: u64,
    pub yield_waits: u64,
    pub yield_passes: u64,
    pub role_no_grant_reasons: BTreeMap<String, u64>,
    pub phase_changes: u64,
    pub lifecycle_successes: u64,
    pub before_boundary_commands: u64,
    pub after_boundary_commands: u64,
}

#[derive(Clone)]
enum Command {
    Replace {
        route: String,
        occurrence: u32,
        progress_mm: u32,
        east: Option<bool>,
        role: bool,
    },
    Leave {
        target: String,
        exit: usize,
        east: bool,
    },
    Reserve {
        target: String,
        expected_rejection: Option<String>,
    },
    Park {
        target: String,
    },
    Despawn,
    Spawn {
        profile: String,
        route: String,
        occurrence: u32,
        progress_mm: u32,
    },
}

impl Command {
    fn rank(&self) -> u8 {
        match self {
            Self::Park { .. } => 0,
            Self::Leave { .. } => 1,
            Self::Replace { .. } | Self::Despawn | Self::Spawn { .. } => 2,
            Self::Reserve { .. } => 3,
        }
    }
    fn name(&self) -> &'static str {
        match self {
            Self::Park { .. } => "park",
            Self::Leave { .. } => "leave",
            Self::Replace { .. } => "replace",
            Self::Reserve { .. } => "reserve",
            Self::Despawn => "despawn",
            Self::Spawn { .. } => "spawn",
        }
    }
    fn departure(&self) -> Option<bool> {
        match self {
            Self::Leave { east, .. } => Some(*east),
            Self::Replace { east, .. } => *east,
            _ => None,
        }
    }
    fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Replace { .. }
                | Self::Leave { .. }
                | Self::Reserve {
                    expected_rejection: Some(_),
                    ..
                }
        )
    }
}

#[derive(Clone)]
struct Request {
    due: u64,
    original_due: u64,
    slot: usize,
    sequence: u32,
    attempt: u32,
    command: Command,
}

pub struct Harness<'a> {
    pub(crate) artifacts: &'a Artifacts,
    pub(crate) plan: &'a ResolvedPlan,
    pub(crate) world: TrafficWorld,
    pub(crate) individuals: Vec<Individual>,
    pub(crate) slots: HashMap<VehicleHandle, usize>,
    pub(crate) routes: BTreeMap<String, RouteHandle>,
    profiles: BTreeMap<String, VehicleProfileOrdinal>,
    pub(crate) route_keys: HashMap<RouteHandle, String>,
    pub(crate) route_edges: HashMap<RouteHandle, Vec<String>>,
    targets: BTreeMap<String, ParkingTarget>,
    pub(crate) target_keys: HashMap<ParkingTarget, String>,
    schedule: BTreeMap<u64, Vec<Request>>,
    pending: BTreeSet<u32>,
    exhausted: BTreeSet<u32>,
    pub(crate) evidence: Vec<TileEvidence>,
    pub(crate) red_waiters: BTreeSet<(IndividualId, u32)>,
    pub(crate) claims: BTreeMap<(IndividualId, String, u32), u32>,
    pub(crate) transition_gates: HashMap<(u32, u32), (u32, u32)>,
    pub(crate) step_before: Vec<Option<VehicleState>>,
    pub(crate) commands: Vec<Value>,
    pub(crate) events: Vec<Value>,
    pub(crate) error_counts: BTreeMap<String, u64>,
    pub(crate) atomic_rejections: BTreeMap<String, u32>,
    watched: BTreeMap<String, u32>,
    pub(crate) replacements: u64,
    pub(crate) births: u64,
    pub(crate) removals: u64,
    last_signals: BTreeMap<u32, u8>,
    pub(crate) last_step_ns: u64,
    pub(crate) last_command_ns: u64,
    pub(crate) last_observation_ns: u64,
}

impl<'a> Harness<'a> {
    pub fn install(artifacts: &'a Artifacts, plan: &'a ResolvedPlan) -> Result<Self> {
        plan.validate(artifacts)?;
        let mut world = artifacts.install()?;
        let mut routes = BTreeMap::new();
        let mut route_keys = HashMap::new();
        let mut route_edges = HashMap::new();
        let mut catalog_routes: Vec<_> = artifacts.catalog.routes.iter().collect();
        catalog_routes.sort_by_key(|r| &r.key);
        for r in catalog_routes {
            let edges: Vec<_> = r.edge_keys.iter().map(|key| artifacts.edges[key]).collect();
            let handle = checked(
                "register route",
                world.register_route(RouteRegisterInput::new(edges)),
            )?;
            routes.insert(r.key.clone(), handle);
            route_keys.insert(handle, r.key.clone());
            route_edges.insert(handle, r.edge_keys.clone());
        }
        let mut targets = BTreeMap::new();
        let mut target_keys = HashMap::new();
        for p in &artifacts.catalog.parking {
            let target = if p.kind == "explicit" {
                let id: ParkingSpaceId = crate::catalog_id(&p.stable_id)?;
                ParkingTarget::ExplicitSpace(
                    artifacts
                        .revision
                        .identity()
                        .ordinal(id)
                        .ok_or_else(|| invalid("unknown space"))?,
                )
            } else {
                let id: ParkingFacilityId = crate::catalog_id(&p.stable_id)?;
                ParkingTarget::VirtualPool(
                    artifacts
                        .revision
                        .identity()
                        .ordinal(id)
                        .ok_or_else(|| invalid("unknown facility"))?,
                )
            };
            targets.insert(p.key.clone(), target);
            target_keys.insert(target, p.key.clone());
        }
        let mut profiles = BTreeMap::new();
        for (key, id) in &artifacts.catalog.profile_ids {
            let id: VehicleProfileId = crate::catalog_id(id)?;
            profiles.insert(
                key.clone(),
                artifacts
                    .revision
                    .identity()
                    .ordinal(id)
                    .ok_or_else(|| invalid("unknown profile"))?,
            );
        }
        let mut individuals = Vec::new();
        let mut slots = HashMap::new();
        for initial in &plan.initial {
            let profile = profiles[&initial.profile];
            let route = routes[&initial.route];
            let handle = if let Some(key) = &initial.parking {
                let target = targets[key];
                let record = checked(
                    "initial Parked binding",
                    world.spawn_parked_vehicle(
                        ParkedVehicleSpawnInput::new(
                            profile,
                            route,
                            initial.occurrence,
                            initial.progress_mm,
                        ),
                        target,
                    ),
                )?;
                if world.parking_binding(record.vehicle) != Some(ParkingBinding::Occupied(target)) {
                    return Err(invalid("initial binding differs"));
                }
                record.vehicle
            } else {
                checked(
                    "initial Active",
                    world.spawn_vehicle(VehicleSpawnInput::new(
                        profile,
                        route,
                        initial.occurrence,
                        initial.progress_mm,
                        0,
                    )),
                )?
            };
            let id = IndividualId {
                tile: initial.tile,
                slot: initial.slot,
                incarnation: 0,
            };
            slots.insert(handle, individuals.len());
            individuals.push(Individual {
                id,
                handle: Some(handle),
                crossed_tile: false,
            });
        }
        let last_signals = signal_signature(&world)?;
        let mut harness = Self {
            artifacts,
            plan,
            world,
            individuals,
            slots,
            routes,
            profiles,
            route_keys,
            route_edges,
            targets,
            target_keys,
            schedule: BTreeMap::new(),
            pending: BTreeSet::new(),
            exhausted: BTreeSet::new(),
            evidence: vec![TileEvidence::default(); plan.tiles as usize],
            red_waiters: BTreeSet::new(),
            claims: BTreeMap::new(),
            transition_gates: observe::transition_gates(artifacts),
            step_before: Vec::new(),
            commands: Vec::new(),
            events: Vec::new(),
            error_counts: BTreeMap::new(),
            atomic_rejections: BTreeMap::new(),
            watched: BTreeMap::new(),
            replacements: 0,
            births: 0,
            removals: 0,
            last_signals,
            last_step_ns: 0,
            last_command_ns: 0,
            last_observation_ns: 0,
        };
        for b in &plan.departures {
            for i in 0..10 {
                harness.enqueue(Request {
                    due: b.due_tick,
                    original_due: b.due_tick,
                    slot: b.first_slot as usize + i,
                    sequence: b.sequence + i as u32,
                    attempt: 1,
                    command: Command::Replace {
                        route: b.routes[i].clone(),
                        occurrence: 0,
                        progress_mm: 7_000,
                        east: Some(i < 7),
                        role: false,
                    },
                });
            }
        }
        for l in &plan.leaves {
            harness.enqueue(Request {
                due: l.due_tick,
                original_due: l.due_tick,
                slot: l.slot as usize,
                sequence: l.sequence,
                attempt: 1,
                command: Command::Leave {
                    target: l.target.clone(),
                    exit: l.exit,
                    east: l.direction == "east",
                },
            });
        }
        for arrival in &plan.arrivals {
            harness.enqueue(Request {
                due: arrival.reserve_tick,
                original_due: arrival.reserve_tick,
                slot: arrival.slot as usize,
                sequence: arrival.sequence,
                attempt: 1,
                command: Command::Reserve {
                    target: arrival.target.clone(),
                    expected_rejection: None,
                },
            });
        }
        for rejection in &plan.reservation_rejections {
            harness.enqueue(Request {
                due: rejection.due_tick,
                original_due: rejection.due_tick,
                slot: rejection.slot as usize,
                sequence: rejection.sequence,
                attempt: 1,
                command: Command::Reserve {
                    target: rejection.target.clone(),
                    expected_rejection: Some(rejection.expected.clone()),
                },
            });
        }
        for role in &plan.role_departures {
            harness.enqueue(Request {
                due: role.due_tick,
                original_due: role.due_tick,
                slot: role.slot as usize,
                sequence: role.sequence,
                attempt: 1,
                command: Command::Replace {
                    route: role.route.clone(),
                    occurrence: role.occurrence,
                    progress_mm: role.progress_mm,
                    east: None,
                    role: true,
                },
            });
        }
        for burst in &plan.lifecycle_bursts {
            harness.enqueue(Request {
                due: burst.despawn_tick,
                original_due: burst.despawn_tick,
                slot: burst.slot as usize,
                sequence: burst.sequence,
                attempt: 1,
                command: Command::Despawn,
            });
            harness.enqueue(Request {
                due: burst.spawn_tick,
                original_due: burst.spawn_tick,
                slot: burst.slot as usize,
                sequence: burst.sequence + 1,
                attempt: 1,
                command: Command::Spawn {
                    profile: burst.profile.clone(),
                    route: burst.route.clone(),
                    occurrence: burst.occurrence,
                    progress_mm: burst.progress_mm,
                },
            });
        }
        let counts = observe::counts(&harness)?;
        if counts
            != (
                plan.initial_counts.active as usize,
                plan.initial_counts.parked as usize,
                plan.initial_counts.completed as usize,
            )
        {
            return Err(invalid("initial lifecycle counts differ"));
        }
        observe::parking_invariants(&harness)?;
        Ok(harness)
    }

    pub fn world(&self) -> &TrafficWorld {
        &self.world
    }
    pub fn stable_individual(&self, handle: VehicleHandle) -> Option<IndividualId> {
        self.slots.get(&handle).map(|i| self.individuals[*i].id)
    }
    pub fn checkpoint(&self) -> Result<String> {
        let snapshot = checked("capture checkpoint", self.world.capture_snapshot())?;
        Ok(format!(
            "{:x}",
            checked("checkpoint digest", deterministic_state_digest(&snapshot))?
        ))
    }
    fn enqueue(&mut self, request: Request) {
        self.schedule.entry(request.due).or_default().push(request);
    }
    pub(crate) fn in_observation(&self, tick: u64) -> bool {
        tick >= self.plan.window.warm_up_ticks && tick < self.plan.window.end()
    }

    fn execute(&mut self, request: Request, tick: u64) -> Result<()> {
        if matches!(&request.command, Command::Despawn | Command::Spawn { .. }) {
            return self.execute_lifecycle_boundary(request, tick);
        }
        let id = self.individuals[request.slot].id;
        let role_hold = matches!(&request.command, Command::Replace { role: false, .. })
            && self.plan.initial[request.slot].role.is_some();
        if role_hold {
            self.defer_role_owned_replace(request, tick, id);
            return Ok(());
        }
        let handle = self.individuals[request.slot]
            .handle
            .ok_or_else(|| invalid("command targets an absent individual"))?;
        let before = self
            .world
            .vehicle(handle)
            .ok_or_else(|| invalid("lost live individual"))?;
        let binding_before = self.world.parking_binding(handle);
        let cursor_before = self.world.command_cursor();
        let event_before = self.world.event_cursor();
        let name = request.command.name();
        if request.command.retryable() && request.attempt == 1 {
            self.pending.insert(request.sequence);
        }
        if let Some(east) = request.command.departure()
            && request.attempt == 1
            && self.in_observation(request.original_due)
        {
            if east {
                self.evidence[id.tile as usize].planned_east += 1;
            } else {
                self.evidence[id.tile as usize].planned_west += 1;
            }
        }
        let caller_deferred = matches!(&request.command, Command::Replace { .. })
            && before.status() != VehicleStatus::Completed;
        // Bound whole-world snapshots to eight candidate calls per command kind until one rejects.
        let watch_count = self.watched.get(name).copied().unwrap_or(0);
        let watch = !caller_deferred
            && !self.atomic_rejections.contains_key(name)
            && watch_count < 8
            && matches!(
                request.command,
                Command::Replace { .. }
                    | Command::Leave { .. }
                    | Command::Reserve {
                        expected_rejection: Some(_),
                        ..
                    }
            );
        let digest_before = if watch {
            self.watched.insert(name.into(), watch_count + 1);
            Some(self.checkpoint()?)
        } else {
            None
        };
        let mut rejection: Option<(&str, Option<VehicleHandle>)> = None;
        let mut extra = Value::Null;
        if caller_deferred {
            rejection = Some(("not-completed", None));
        } else {
            match &request.command {
                Command::Replace {
                    route,
                    occurrence,
                    progress_mm,
                    ..
                } => {
                    let input = VehicleSpawnInput::new(
                        before.profile(),
                        self.routes[route],
                        *occurrence,
                        *progress_mm,
                        0,
                    );
                    match self.world.replace_completed_vehicle(handle, input) {
                        Ok(record) => {
                            self.slots.remove(&handle);
                            self.slots.insert(record.new, request.slot);
                            self.individuals[request.slot] = Individual {
                                id: IndividualId {
                                    incarnation: request.sequence + 1,
                                    ..id
                                },
                                handle: Some(record.new),
                                crossed_tile: false,
                            };
                            self.replacements += 1;
                            extra = json!({"new_individual": self.individuals[request.slot].id, "route": route});
                        }
                        Err(ReplaceError::Blocked(block)) => {
                            rejection = Some(("entry-blocked", Some(block.blocker)));
                        }
                        Err(error) => {
                            return Err(invalid(format!(
                                "unexpected replacement error at tick {tick}: {error}"
                            )));
                        }
                    }
                }
                Command::Leave { target, exit, .. } => {
                    let spec = self
                        .artifacts
                        .catalog
                        .parking
                        .iter()
                        .find(|p| p.key == *target)
                        .expect("validated parking target");
                    let anchor = &spec.exits[*exit];
                    let input = match self.targets[target] {
                        ParkingTarget::ExplicitSpace(space) => LeaveParkingTarget::ExplicitSpace {
                            space,
                            route: self.routes[&anchor.route],
                            exit_route_occurrence: anchor.route_edge_index,
                        },
                        ParkingTarget::VirtualPool(facility) => LeaveParkingTarget::VirtualPool {
                            facility,
                            route: self.routes[&anchor.route],
                            exit_anchor: VirtualExitAnchorSelector::from_raw(
                                anchor.virtual_anchor_index.expect("virtual selector"),
                            ),
                            exit_route_occurrence: anchor.route_edge_index,
                        },
                    };
                    match self.world.leave_parking(handle, input) {
                        Ok(_) => {
                            if self.in_observation(tick) {
                                self.evidence[id.tile as usize].leaves += 1;
                                if *exit == 0 {
                                    self.evidence[id.tile as usize].garage_exit_0 += 1;
                                } else if *exit == 1 {
                                    self.evidence[id.tile as usize].garage_exit_1 += 1;
                                }
                                if request.attempt > 1 {
                                    self.evidence[id.tile as usize].retried_leave_successes += 1;
                                }
                            }
                            extra = json!({"target":target,"exit":exit});
                        }
                        Err(ParkingError::LeavePhysicalOverlap { blocker }) => {
                            if self.in_observation(tick) {
                                self.evidence[id.tile as usize].safe_leave_rejections += 1;
                            }
                            rejection = Some(("leave-overlap", Some(blocker)));
                        }
                        Err(ParkingError::LeaveUnsafeFollower { follower }) => {
                            if self.in_observation(tick) {
                                self.evidence[id.tile as usize].safe_leave_rejections += 1;
                            }
                            rejection = Some(("leave-unsafe-follower", Some(follower)));
                        }
                        Err(error) => {
                            return Err(invalid(format!(
                                "unexpected leave error at tick {tick}: {error}"
                            )));
                        }
                    }
                }
                Command::Reserve {
                    target,
                    expected_rejection,
                } => {
                    let spec = self
                        .artifacts
                        .catalog
                        .parking
                        .iter()
                        .find(|p| p.key == *target)
                        .expect("validated target");
                    let anchor = &spec.entries[0];
                    let input = match self.targets[target] {
                        ParkingTarget::ExplicitSpace(space) => {
                            ReserveParkingTarget::ExplicitSpace {
                                space,
                                entry_route_occurrence: anchor.route_edge_index,
                            }
                        }
                        ParkingTarget::VirtualPool(facility) => ReserveParkingTarget::VirtualPool {
                            facility,
                            entry_anchor: VirtualEntryAnchorSelector::from_raw(
                                anchor.virtual_anchor_index.expect("virtual selector"),
                            ),
                            entry_route_occurrence: anchor.route_edge_index,
                        },
                    };
                    match self.world.reserve_parking(handle, input) {
                        Ok(_) if expected_rejection.is_none() => {
                            extra = json!({"target":target});
                        }
                        Ok(_) => {
                            return Err(invalid(format!(
                                "expected reservation rejection committed at tick {tick}"
                            )));
                        }
                        Err(ParkingError::TargetBoundByOther)
                            if expected_rejection.as_deref() == Some("exclusive-occupied") =>
                        {
                            if self.in_observation(tick) {
                                self.evidence[id.tile as usize].exclusive_rejections += 1;
                            }
                            rejection = Some(("exclusive-occupied", None));
                        }
                        Err(ParkingError::VirtualCapacityExhausted)
                            if expected_rejection.as_deref() == Some("virtual-full") =>
                        {
                            if self.in_observation(tick) {
                                self.evidence[id.tile as usize].full_rejections += 1;
                            }
                            rejection = Some(("virtual-full", None));
                        }
                        Err(ParkingError::InvalidVehicleStatus) if expected_rejection.is_some() => {
                            rejection = Some(("role-not-active", None));
                        }
                        Err(error) => {
                            return Err(invalid(format!(
                                "unexpected reserve error at tick {tick}: {error}"
                            )));
                        }
                    }
                }
                Command::Park { target } => {
                    checked(
                        "park at committed arrival",
                        self.world.park_vehicle(handle, self.targets[target]),
                    )?;
                    if self.in_observation(tick) {
                        match self.targets[target] {
                            ParkingTarget::ExplicitSpace(_) => {
                                self.evidence[id.tile as usize].explicit_parks += 1
                            }
                            ParkingTarget::VirtualPool(_) => {
                                self.evidence[id.tile as usize].virtual_parks += 1
                            }
                        }
                    }
                    extra = json!({"target":target});
                }
                Command::Despawn | Command::Spawn { .. } => unreachable!(),
            }
        }
        if let Some((reason, blocker)) = rejection {
            if self.world.vehicle(handle) != Some(before)
                || self.world.parking_binding(handle) != binding_before
                || self.world.command_cursor() != cursor_before
                || self.world.event_cursor() != event_before
            {
                return Err(invalid(format!("rejected {name} changed committed state")));
            }
            if let Some(before_digest) = digest_before {
                if before_digest != self.checkpoint()? {
                    return Err(invalid("rejected command changed complete state"));
                }
                *self.atomic_rejections.entry(name.into()).or_default() += 1;
            }
            *self.error_counts.entry(reason.into()).or_default() += 1;
            extra = json!({"reason": reason, "blocker": blocker.and_then(|v| self.stable_individual(v))});
            if request.command.retryable() {
                let expected_reservation = matches!(
                    request.command,
                    Command::Reserve {
                        expected_rejection: Some(_),
                        ..
                    }
                );
                if expected_reservation && reason != "role-not-active" {
                    self.pending.remove(&request.sequence);
                } else {
                    let next_tick = tick + self.plan.retry_ticks;
                    if request.attempt < self.plan.max_attempts
                        && next_tick < self.plan.window.end()
                    {
                        self.enqueue(Request {
                            due: next_tick,
                            attempt: request.attempt + 1,
                            ..request.clone()
                        });
                    } else {
                        self.exhausted.insert(request.sequence);
                    }
                }
            }
        } else if request.command.retryable() {
            self.pending.remove(&request.sequence);
            if let Some(east) = request.command.departure()
                && self.in_observation(tick)
            {
                if east {
                    self.evidence[id.tile as usize].admitted_east += 1;
                } else {
                    self.evidence[id.tile as usize].admitted_west += 1;
                }
            }
        }
        if rejection.is_none() {
            let individual = &self.individuals[request.slot];
            let after = self
                .world
                .vehicle(individual.handle.expect("committed command remains live"))
                .expect("live individual");
            if id != individual.id || before.status() != after.status() {
                if self.in_observation(tick) {
                    self.evidence[id.tile as usize].lifecycle_successes += 1;
                }
                self.events.push(json!({"kind":"lifecycle", "phase":"command", "tick":tick,
                    "sequence":request.sequence, "attempt":request.attempt, "command":name,
                    "individual":id, "after_individual":individual.id,
                    "before":observe::status(before.status()), "after":observe::status(after.status())}));
            }
        }
        self.commands.push(json!({"boundary":tick,"due":request.original_due,"sequence":request.sequence,
            "attempt":request.attempt,"individual":id,"command":name,"committed":rejection.is_none(),
            "cursor_before":cursor_before,"cursor_after":self.world.command_cursor(),"details":extra}));
        if self.in_observation(tick) {
            let evidence = &mut self.evidence[id.tile as usize];
            if self
                .plan
                .boundary_windows
                .iter()
                .any(|window| window.tile == id.tile && window.before_tick == tick)
            {
                evidence.before_boundary_commands += 1;
            }
            if self
                .plan
                .boundary_windows
                .iter()
                .any(|window| window.tile == id.tile && window.after_tick == tick)
            {
                evidence.after_boundary_commands += 1;
            }
        }
        Ok(())
    }

    fn defer_role_owned_replace(&mut self, request: Request, tick: u64, id: IndividualId) {
        debug_assert!(matches!(
            &request.command,
            Command::Replace { role: false, .. }
        ));
        if request.attempt == 1 {
            self.pending.insert(request.sequence);
            if let Some(east) = request.command.departure()
                && self.in_observation(request.original_due)
            {
                if east {
                    self.evidence[id.tile as usize].planned_east += 1;
                } else {
                    self.evidence[id.tile as usize].planned_west += 1;
                }
            }
        }
        *self.error_counts.entry("role-held".into()).or_default() += 1;
        let next_tick = tick + self.plan.retry_ticks;
        if request.attempt < self.plan.max_attempts && next_tick < self.plan.window.end() {
            self.enqueue(Request {
                due: next_tick,
                attempt: request.attempt + 1,
                ..request.clone()
            });
        } else {
            self.exhausted.insert(request.sequence);
        }
        let cursor = self.world.command_cursor();
        self.commands
            .push(json!({"boundary":tick,"due":request.original_due,
            "sequence":request.sequence,"attempt":request.attempt,"individual":id,
            "command":"replace","committed":false,"cursor_before":cursor,
            "cursor_after":cursor,"details":{"reason":"role-held","blocker":null}}));
        if self.in_observation(tick) {
            let evidence = &mut self.evidence[id.tile as usize];
            if self
                .plan
                .boundary_windows
                .iter()
                .any(|window| window.tile == id.tile && window.before_tick == tick)
            {
                evidence.before_boundary_commands += 1;
            }
            if self
                .plan
                .boundary_windows
                .iter()
                .any(|window| window.tile == id.tile && window.after_tick == tick)
            {
                evidence.after_boundary_commands += 1;
            }
        }
    }

    fn execute_lifecycle_boundary(&mut self, request: Request, tick: u64) -> Result<()> {
        let before_id = self.individuals[request.slot].id;
        let cursor_before = self.world.command_cursor();
        let name = request.command.name();
        let (before, after, after_id, details) = match &request.command {
            Command::Despawn => {
                let handle = self.individuals[request.slot]
                    .handle
                    .ok_or_else(|| invalid("despawn targets an absent individual"))?;
                let before = self
                    .world
                    .vehicle(handle)
                    .ok_or_else(|| invalid("despawn lost its live individual"))?;
                checked("boundary despawn", self.world.despawn_vehicle(handle))?;
                self.slots.remove(&handle);
                self.individuals[request.slot].handle = None;
                self.removals += 1;
                (
                    Some(observe::status(before.status())),
                    None,
                    before_id,
                    Value::Null,
                )
            }
            Command::Spawn {
                profile,
                route,
                occurrence,
                progress_mm,
            } => {
                if self.individuals[request.slot].handle.is_some() {
                    return Err(invalid("spawn targets a live individual"));
                }
                let handle = checked(
                    "boundary spawn",
                    self.world.spawn_vehicle(VehicleSpawnInput::new(
                        self.profiles[profile],
                        self.routes[route],
                        *occurrence,
                        *progress_mm,
                        0,
                    )),
                )?;
                let after_id = IndividualId {
                    incarnation: request.sequence + 1,
                    ..before_id
                };
                self.individuals[request.slot].id = after_id;
                self.individuals[request.slot].handle = Some(handle);
                self.individuals[request.slot].crossed_tile = false;
                self.slots.insert(handle, request.slot);
                self.births += 1;
                (
                    None,
                    Some(observe::status(VehicleStatus::Active)),
                    after_id,
                    json!({"route":route,"occurrence":occurrence,"progress_mm":progress_mm}),
                )
            }
            _ => unreachable!(),
        };
        if self.in_observation(tick) {
            let evidence = &mut self.evidence[before_id.tile as usize];
            evidence.lifecycle_successes += 1;
            if self
                .plan
                .boundary_windows
                .iter()
                .any(|window| window.tile == before_id.tile && window.before_tick == tick)
            {
                evidence.before_boundary_commands += 1;
            }
            if self
                .plan
                .boundary_windows
                .iter()
                .any(|window| window.tile == before_id.tile && window.after_tick == tick)
            {
                evidence.after_boundary_commands += 1;
            }
        }
        self.events
            .push(json!({"kind":"lifecycle", "phase":"command", "tick":tick,
            "sequence":request.sequence,"attempt":1,"command":name,"individual":before_id,
            "after_individual":after_id,"before":before,"after":after}));
        self.commands.push(json!({"boundary":tick,"due":request.original_due,
            "sequence":request.sequence,"attempt":1,"individual":before_id,"command":name,
            "committed":true,"cursor_before":cursor_before,"cursor_after":self.world.command_cursor(),
            "details":details}));
        Ok(())
    }

    /// Processes caller commands, one real fixed step, and public committed observations.
    pub fn advance(&mut self) -> Result<TickRecord> {
        let boundary = self.world.tick_index();
        if boundary >= self.plan.window.end() {
            return Err(invalid("finite plan finished"));
        }
        self.commands.clear();
        self.events.clear();
        let command_started = std::time::Instant::now();
        while let Some(mut commands) = self.schedule.remove(&boundary) {
            commands.sort_by_key(|r| (r.command.rank(), r.due, r.slot, r.sequence, r.attempt));
            for request in commands {
                self.execute(request, boundary)?;
            }
        }
        self.last_command_ns = command_started
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        self.step_before = self
            .individuals
            .iter()
            .map(|i| i.handle.and_then(|handle| self.world.vehicle(handle)))
            .collect();
        let intent = self
            .step_before
            .iter()
            .flatten()
            .filter(|s| s.status() == VehicleStatus::Active)
            .count();
        observe::red_waiters(self);
        let started = std::time::Instant::now();
        let outcome = checked(
            "TrafficWorld step",
            self.world.step(TickInput::new(self.plan.dt)),
        )?;
        let signals = signal_signature(&self.world)?;
        if signals != self.last_signals {
            for window in &self.plan.boundary_windows {
                if window.after_tick == outcome.tick_index()
                    && signal_groups_changed(&self.last_signals, &signals, &window.groups)?
                {
                    self.evidence[window.tile as usize].phase_changes += 1;
                }
            }
            self.last_signals = signals;
        }
        self.last_step_ns = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        let observation_started = std::time::Instant::now();
        for arrival in outcome.parking_arrivals() {
            let slot = *self
                .slots
                .get(&arrival.vehicle)
                .ok_or_else(|| invalid("unknown arrival individual"))?;
            let due = self
                .plan
                .arrivals
                .iter()
                .find(|a| a.slot as usize == slot)
                .map_or(boundary + 1, |a| (boundary + 1).max(a.park_not_before_tick));
            self.enqueue(Request {
                due,
                original_due: due,
                slot,
                sequence: 20_000_000 + slot as u32,
                attempt: 1,
                command: Command::Park {
                    target: self.target_keys[&arrival.target].clone(),
                },
            });
            self.events.push(json!({"kind":"parking-arrival", "tick":outcome.tick_index(),
                "individual":self.individuals[slot].id, "target":self.target_keys[&arrival.target]}));
        }
        observe::events(self)?;
        let state_digest = observe::state(self)?;
        let (active, parked, completed) = observe::counts(self)?;
        observe::parking_invariants(self)?;
        self.last_observation_ns = observation_started
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        Ok(TickRecord {
            tick: outcome.tick_index(),
            time_ms: outcome.time_ms(),
            domain: "road_motor_vehicle".into(),
            live: self
                .individuals
                .iter()
                .filter(|i| i.handle.is_some())
                .count(),
            active,
            parked,
            completed,
            intent,
            intent_basis: "exact_active_before_step".into(),
            presented: 0,
            aggregate_records: 0,
            aggregate_equivalent: 0,
            future_departures: (self.plan.departures.len()
                - self
                    .plan
                    .departures
                    .partition_point(|b| b.due_tick <= boundary))
                * 10
                + self.plan.leaves.len()
                - self.plan.leaves.partition_point(|l| l.due_tick <= boundary),
            pending_departures: self.pending.len() - self.exhausted.len(),
            exhausted_departures: self.exhausted.len(),
            command_cursor: self.world.command_cursor(),
            event_cursor: self.world.event_cursor(),
            state_digest,
            event_digest: sha256(&serde_json::to_vec(&self.events)?),
            commands_digest: sha256(&serde_json::to_vec(&self.commands)?),
        })
    }
}

fn signal_signature(world: &TrafficWorld) -> Result<BTreeMap<u32, u8>> {
    world
        .committed_signal_groups()
        .as_slice()
        .iter()
        .map(|(group, aspect)| {
            Ok((
                group.raw(),
                match aspect {
                    laneflow_static_contract::SignalAspect::Red => 0,
                    laneflow_static_contract::SignalAspect::Yellow => 1,
                    laneflow_static_contract::SignalAspect::Green => 2,
                    _ => return Err(invalid("unsupported signal aspect")),
                },
            ))
        })
        .collect()
}

fn signal_groups_changed(
    before: &BTreeMap<u32, u8>,
    after: &BTreeMap<u32, u8>,
    groups: &[u32],
) -> Result<bool> {
    let mut changed = false;
    for group in groups {
        let before = before
            .get(group)
            .ok_or_else(|| invalid("boundary signal group missing before step"))?;
        let after = after
            .get(group)
            .ok_or_else(|| invalid("boundary signal group missing after step"))?;
        changed |= before != after;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use laneflow_urban_generator::{Scale, UrbanConfig, generate};

    #[test]
    fn role_owned_background_replace_defers_across_an_absent_lifecycle_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let config =
            UrbanConfig::parse(include_str!("../../../examples/config/cn-urban.toml")).unwrap();
        generate(&config, Scale::Fixture, &source, None).unwrap();
        let artifacts = Artifacts::load(&source).unwrap();
        let plan = ResolvedPlan::for_case(
            &artifacts,
            crate::UrbanCase::MixedPeak,
            crate::Window::probe(256).unwrap(),
        )
        .unwrap();
        let mut harness = Harness::install(&artifacts, &plan).unwrap();
        let slot = 741;
        let handle = harness.individuals[slot].handle.unwrap();
        let id = harness.individuals[slot].id;
        harness.world.despawn_vehicle(handle).unwrap();
        harness.slots.remove(&handle);
        harness.individuals[slot].handle = None;
        let cursor = harness.world.command_cursor();

        harness
            .execute(
                Request {
                    due: 0,
                    original_due: 0,
                    slot,
                    sequence: 544,
                    attempt: 1,
                    command: Command::Replace {
                        route: "must-not-resolve".into(),
                        occurrence: 0,
                        progress_mm: 7_000,
                        east: Some(true),
                        role: false,
                    },
                },
                0,
            )
            .unwrap();

        assert_eq!(harness.world.command_cursor(), cursor);
        assert!(harness.individuals[slot].handle.is_none());
        assert!(harness.pending.contains(&544));
        assert!(harness.schedule.contains_key(&plan.retry_ticks));
        assert_eq!(harness.commands.len(), 1);
        assert_eq!(harness.commands[0]["individual"], serde_json::json!(id));
        assert_eq!(harness.commands[0]["committed"], false);
        assert_eq!(harness.commands[0]["details"]["reason"], "role-held");
    }
}
