mod observe;

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use laneflow_runtime::*;
use laneflow_static_contract::{ParkingSpaceId, ParkingSpaceOrdinal};
use laneflow_synth_generator::{Bundle, Case};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Error, Plan, failure, install::Sample, require};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Operation {
    ReserveParking,
    ParkVehicle,
    LeaveParking,
    Despawn,
    Spawn,
    AtomicReplace,
}

impl Operation {
    fn code(self) -> u32 {
        match self {
            Self::ReserveParking => 0,
            Self::ParkVehicle => 1,
            Self::LeaveParking => 2,
            Self::Despawn => 3,
            Self::Spawn => 4,
            Self::AtomicReplace => 5,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Command {
    operation: Operation,
    logical_rank: u32,
    route_rank: u32,
    edge_occurrence: u32,
    progress_mm: u32,
    speed_mm_per_s: u32,
}

impl Command {
    fn words(&self) -> [u32; 6] {
        [
            self.operation.code(),
            self.logical_rank,
            self.route_rank,
            self.edge_occurrence,
            self.progress_mm,
            self.speed_mm_per_s,
        ]
    }

    fn encode(&self, target: &mut Vec<u8>) {
        for word in self.words() {
            target.extend_from_slice(&word.to_le_bytes());
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scheduled {
    boundary: u64,
    commands: Vec<Command>,
}

#[derive(Debug, Deserialize)]
struct ReplacementRule {
    ranks: Vec<u32>,
    edge_occurrence: u32,
    progress_mm: u32,
    speed_mm_per_s: u32,
}

struct Journal {
    output: BufWriter<File>,
    digest: blake3::Hasher,
    count: u64,
    blocked: u64,
}

impl Journal {
    fn new(path: &Path) -> Result<Self, Error> {
        let mut output = BufWriter::new(File::create(path)?);
        output.write_all(b"lf-synth-command-receipts-1\0")?;
        let mut digest = blake3::Hasher::new();
        digest.update(b"lf-synth-command-receipts-1\0");
        Ok(Self {
            output,
            digest,
            count: 0,
            blocked: 0,
        })
    }

    fn append(
        &mut self,
        command: &Command,
        boundary: u64,
        cursors: [u64; 2],
        incarnations: [u32; 2],
        committed: bool,
    ) -> Result<(), Error> {
        let mut bytes = Vec::with_capacity(68);
        for word in [boundary, self.count, cursors[0], cursors[1]] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        command.encode(&mut bytes);
        for word in [incarnations[0], incarnations[1], u32::from(!committed)] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        self.output.write_all(&bytes)?;
        self.digest.update(&bytes);
        self.count += 1;
        self.blocked += u64::from(!committed);
        Ok(())
    }
}

struct Burst {
    sample: Sample,
    spaces: Vec<ParkingSpaceOrdinal>,
    live_ranks: Vec<usize>,
    live_sequences: Vec<u32>,
    handle_ranks: HashMap<VehicleHandle, usize>,
    incarnations: Vec<u32>,
    expected_status: Vec<VehicleStatus>,
    terminal: Vec<bool>,
    lane_order: Vec<usize>,
    arrivals: Vec<Option<u64>>,
    completions: Vec<(u64, u32)>,
    arrival_records: Vec<(u64, u32)>,
    replacements: Option<ReplacementRule>,
    pending: Vec<Command>,
    parked_boundary: Option<u64>,
    left: bool,
    counts: [u32; 3],
    changed: Vec<bool>,
    total_crossings: Vec<u32>,
}

impl Burst {
    fn new(bundle: &Bundle, plan: &Plan) -> Result<Self, Error> {
        let sample = Sample::new(bundle)?;
        let spaces = sample
            .state
            .vehicles
            .iter()
            .map(|vehicle| {
                let id = vehicle
                    .parking_space_id
                    .parse::<ParkingSpaceId>()
                    .map_err(|e| failure("parking id", e))?;
                sample
                    .world
                    .revision()
                    .identity()
                    .ordinal(id)
                    .ok_or_else(|| failure("space", "missing"))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let size = sample.handles.len();
        let replacements = plan
            .rules
            .iter()
            .find(|rule| rule["kind"] == "completed_replacement")
            .map(|rule| serde_json::from_value(rule.clone()))
            .transpose()?;
        let mut result = Self {
            live_ranks: (0..size).collect(),
            live_sequences: (0..size as u32).collect(),
            handle_ranks: sample
                .handles
                .iter()
                .copied()
                .enumerate()
                .map(|(rank, h)| (h, rank))
                .collect(),
            expected_status: sample.initial.iter().map(|s| s.status()).collect(),
            incarnations: vec![0; size],
            terminal: vec![false; size],
            lane_order: Vec::new(),
            arrivals: vec![None; size / 100],
            completions: Vec::new(),
            arrival_records: Vec::new(),
            replacements,
            pending: Vec::new(),
            parked_boundary: None,
            left: false,
            counts: [0; 3],
            changed: vec![false; size],
            total_crossings: vec![0; size],
            sample,
            spaces,
        };
        result.refresh_order();
        Ok(result)
    }

    fn refresh_order(&mut self) {
        for (sequence, &rank) in self.live_ranks.iter().enumerate() {
            self.live_sequences[rank] = sequence as u32;
        }
        self.lane_order.clear();
        self.lane_order.extend(
            (0..self.sample.handles.len())
                .filter(|&rank| self.expected_status[rank] == VehicleStatus::Active),
        );
        self.lane_order.sort_unstable_by_key(|&rank| {
            (
                rank / 25,
                crate::observe::position(self.sample.previous[rank]),
            )
        });
    }

    fn checkpoint(&self, stream: &blake3::Hasher) -> Result<Value, Error> {
        Ok(
            json!({"tick":self.sample.world.tick_index(), "state_digest":state_digest(&self.sample.world)?,
            "public_trace_digest":stream.clone().finalize().to_hex().to_string(),
            "population":{"active":self.counts[0],"parked":self.counts[1],"completed":self.counts[2]},
            "command_cursor":self.sample.cursor,"gate_crossings":self.sample.transitions}),
        )
    }

    fn input(&self, command: &Command) -> VehicleSpawnInput {
        VehicleSpawnInput::new(
            self.sample.initial[command.logical_rank as usize].profile(),
            self.sample.routes[command.route_rank as usize],
            command.edge_occurrence,
            command.progress_mm,
            command.speed_mm_per_s,
        )
    }

    fn command(&mut self, command: &Command) -> Result<bool, Error> {
        let rank = command.logical_rank as usize;
        require(
            rank < self.sample.handles.len() && command.route_rank as usize == rank / 25,
            || "命令逻辑身份不符".into(),
        )?;
        let old = self.sample.handles[rank];
        let route = self.sample.routes[rank / 25];
        let space = self.spaces[rank];
        let target = ParkingTarget::ExplicitSpace(space);
        let cursor = self.sample.cursor;
        let mut state_changed = true;
        match command.operation {
            Operation::ReserveParking => {
                let record = self
                    .sample
                    .world
                    .reserve_parking(
                        old,
                        ReserveParkingTarget::ExplicitSpace {
                            space,
                            entry_route_occurrence: command.edge_occurrence,
                        },
                    )
                    .map_err(|e| failure("reserve", e))?;
                let ParkingCommandOutcome::Committed(record) = record else {
                    return Err(failure("reserve", "NoChange"));
                };
                require(
                    record.vehicle == old
                        && record.target == target
                        && record.route == route
                        && record.entry_route_occurrence == 1
                        && record.virtual_entry_selector.is_none()
                        && !record.arrived,
                    || format!("reservation record rank={rank}"),
                )?;
                let Some(ParkingBinding::Reserved(binding)) =
                    self.sample.world.parking_binding(old)
                else {
                    return Err(failure("reserve binding", rank));
                };
                require(
                    binding.target() == target
                        && binding.route() == route
                        && binding.entry_route_occurrence() == 1
                        && binding.virtual_entry_selector().is_none(),
                    || format!("reservation payload rank={rank}"),
                )?;
                self.sample.bindings[rank] = Some(ParkingBinding::Reserved(binding));
            }
            Operation::ParkVehicle => {
                let record = self
                    .sample
                    .world
                    .park_vehicle(old, target)
                    .map_err(|e| failure("park", e))?;
                let ParkingCommandOutcome::Committed(record) = record else {
                    return Err(failure("park", "NoChange"));
                };
                require(record.vehicle == old && record.target == target, || {
                    format!("park record rank={rank}")
                })?;
                self.expected_status[rank] = VehicleStatus::Parked;
                self.sample.bindings[rank] = Some(ParkingBinding::Occupied(target));
            }
            Operation::LeaveParking => {
                let record = self
                    .sample
                    .world
                    .leave_parking(
                        old,
                        LeaveParkingTarget::ExplicitSpace {
                            space,
                            route,
                            exit_route_occurrence: command.edge_occurrence,
                        },
                    )
                    .map_err(|e| failure("leave", e))?;
                require(
                    record.vehicle == old
                        && record.target == target
                        && record.route == route
                        && record.exit_route_occurrence == 1
                        && record.virtual_exit_selector.is_none(),
                    || format!("leave record rank={rank}"),
                )?;
                self.expected_status[rank] = VehicleStatus::Active;
                self.sample.bindings[rank] = None;
            }
            Operation::Despawn => {
                let record = self
                    .sample
                    .world
                    .despawn_vehicle(old)
                    .map_err(|e| failure("despawn", e))?;
                require(
                    record.vehicle == old
                        && record.status == VehicleStatus::Active
                        && record.parking_binding.is_none()
                        && record.waiting_release.is_none()
                        && record.conflict_release.is_none()
                        && self.sample.world.vehicle(old).is_none(),
                    || format!("despawn record rank={rank}"),
                )?;
                let index = self
                    .live_ranks
                    .iter()
                    .position(|&r| r == rank)
                    .ok_or_else(|| failure("live rank", rank))?;
                self.live_ranks.remove(index);
                self.handle_ranks.remove(&old);
                state_changed = false;
            }
            Operation::Spawn => {
                require(
                    !self.handle_ranks.contains_key(&old)
                        && self.sample.world.vehicle(old).is_none(),
                    || format!("spawn needs prior despawn rank={rank}"),
                )?;
                let new = self
                    .sample
                    .world
                    .spawn_vehicle(self.input(command))
                    .map_err(|e| failure("spawn", e))?;
                self.accept_handle(rank, old, new)?;
                self.live_ranks.push(rank);
                self.expected_status[rank] = VehicleStatus::Active;
                self.terminal[rank] =
                    command.edge_occurrence == 1 && command.progress_mm == 5_130_000;
            }
            Operation::AtomicReplace => {
                require(
                    self.expected_status[rank] == VehicleStatus::Completed,
                    || format!("replace needs completion rank={rank}"),
                )?;
                match self
                    .sample
                    .world
                    .replace_completed_vehicle(old, self.input(command))
                {
                    Ok(record) => {
                        require(record.old == old, || format!("replace record rank={rank}"))?;
                        self.accept_handle(rank, old, record.new)?;
                        self.expected_status[rank] = VehicleStatus::Active;
                    }
                    Err(ReplaceError::Blocked(_))
                        if self.sample.state.identity.case == Case::W3 =>
                    {
                        require(
                            self.sample.world.command_cursor() == cursor
                                && self.sample.world.vehicle(old)
                                    == Some(self.sample.previous[rank]),
                            || format!("blocked changed state rank={rank}"),
                        )?;
                        return Ok(false);
                    }
                    Err(error) => return Err(failure("replace", error)),
                }
            }
        }
        self.sample.cursor += 1;
        require(
            self.sample.world.command_cursor() == self.sample.cursor,
            || format!("command cursor rank={rank}"),
        )?;
        if state_changed {
            let handle = self.sample.handles[rank];
            let state = self
                .sample
                .world
                .vehicle(handle)
                .ok_or_else(|| failure("post command vehicle", rank))?;
            require(
                state.status() == self.expected_status[rank]
                    && state.route() == route
                    && self.sample.world.parking_binding(handle) == self.sample.bindings[rank],
                || format!("post command authority rank={rank}"),
            )?;
            if matches!(
                command.operation,
                Operation::Spawn
                    | Operation::AtomicReplace
                    | Operation::LeaveParking
                    | Operation::ParkVehicle
            ) {
                require(
                    state.route_edge_index() == command.edge_occurrence
                        && state.progress_mm() == command.progress_mm
                        && state.speed_mm_s() == command.speed_mm_per_s
                        && state.carry_um() == 0,
                    || format!("post command position rank={rank}"),
                )?;
            } else {
                require(state == self.sample.previous[rank], || {
                    format!("reservation moved vehicle rank={rank}")
                })?;
            }
            self.sample.previous[rank] = state;
        }
        Ok(true)
    }

    fn accept_handle(
        &mut self,
        rank: usize,
        old: VehicleHandle,
        new: VehicleHandle,
    ) -> Result<(), Error> {
        require(
            new != old
                && self.sample.world.vehicle(old).is_none()
                && !self.handle_ranks.contains_key(&new),
            || format!("handle incarnation rank={rank}"),
        )?;
        self.handle_ranks.remove(&old);
        self.handle_ranks.insert(new, rank);
        self.sample.handles[rank] = new;
        self.incarnations[rank] += 1;
        Ok(())
    }

    fn boundary_commands(
        &mut self,
        boundary: u64,
        total: u64,
        scheduled: &[Scheduled],
    ) -> Result<Vec<Command>, Error> {
        require(boundary < total, || {
            "不能在观察窗口后执行命令或额外 step".into()
        })?;
        let mut commands = Vec::new();
        if self.sample.state.identity.case == Case::W3 {
            if self.arrivals.iter().all(Option::is_some) && self.parked_boundary.is_none() {
                require(total - boundary >= 2, || {
                    "arrival barrier 后不足两拍".into()
                })?;
                self.parked_boundary = Some(boundary);
                commands.extend(
                    (0..self.arrivals.len())
                        .map(|cell| parking_command(Operation::ParkVehicle, cell)),
                );
            } else if self.parked_boundary.is_some()
                && self.parked_boundary == boundary.checked_sub(1)
                && !self.left
            {
                self.left = true;
                commands.extend(
                    (0..self.arrivals.len())
                        .map(|cell| parking_command(Operation::LeaveParking, cell)),
                );
            }
            commands.append(&mut self.pending);
        }
        if let Some(batch) = scheduled.iter().find(|batch| batch.boundary == boundary) {
            commands.extend(batch.commands.clone());
        }
        Ok(commands)
    }
}

fn parking_command(operation: Operation, cell: usize) -> Command {
    let rank = cell as u32 * 100;
    Command {
        operation,
        logical_rank: rank,
        route_rank: rank / 25,
        edge_occurrence: 1,
        progress_mm: 5_000,
        speed_mm_per_s: 0,
    }
}

fn state_digest(world: &TrafficWorld) -> Result<String, Error> {
    let snapshot = world
        .capture_snapshot()
        .map_err(|e| failure("snapshot", e))?;
    let digest = deterministic_state_digest(&snapshot).map_err(|e| failure("state digest", e))?;
    Ok(digest
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn input_digest(commands: &[Command]) -> String {
    let mut bytes = b"lf-synth-command-batch-1\0".to_vec();
    bytes.extend_from_slice(&(commands.len() as u32).to_le_bytes());
    for command in commands {
        command.encode(&mut bytes);
    }
    blake3::hash(&bytes).to_hex().to_string()
}

pub(super) fn run(
    bundle: &Bundle,
    plan: &Plan,
    implementation: &str,
    output: &Path,
) -> Result<Value, Error> {
    let scheduled: Vec<Scheduled> = plan
        .scheduled
        .iter()
        .cloned()
        .map(serde_json::from_value)
        .collect::<Result<_, _>>()?;
    let mut burst = Burst::new(bundle, plan)?;
    let initial_cursor = burst.sample.cursor;
    let mut journal = Journal::new(&output.join("commands.bin"))?;
    let mut trace = BufWriter::new(File::create(output.join("ticks.bin"))?);
    trace.write_all(b"lf-synth-burst-ticks-1\0")?;
    let mut stream = blake3::Hasher::new();
    stream.update(b"lf-synth-burst-stream-1\0");
    let total = plan.warm_up_ticks + plan.observation_ticks;
    let checkpoints_at = [
        0,
        plan.warm_up_ticks,
        plan.warm_up_ticks + 1,
        plan.warm_up_ticks + 2,
        plan.warm_up_ticks + 1_024,
        plan.warm_up_ticks + 2_048,
        plan.warm_up_ticks + 4_096,
        total,
    ];
    let mut checkpoints = Vec::new();
    let mut batches = Vec::new();
    let mut outcome = None;
    for tick in 0..=total {
        let boundary_digest = burst.observe(tick, plan.warm_up_ticks, outcome.as_ref())?;
        trace.write_all(&tick.to_le_bytes())?;
        trace.write_all(boundary_digest.as_bytes())?;
        stream.update(&tick.to_le_bytes());
        stream.update(boundary_digest.as_bytes());
        if checkpoints_at.contains(&tick)
            || burst
                .parked_boundary
                .is_some_and(|b| tick == b + 1 || tick == b + 2)
        {
            checkpoints.push(burst.checkpoint(&stream)?);
        }
        if tick % 1_024 == 0 || tick == total {
            trace.flush()?;
            journal.output.flush()?;
            eprintln!(
                "{:?} scale={} dt={} tick={tick}/{total}",
                plan.identity.case, plan.identity.scale, plan.identity.fixed_delta_ms
            );
        }
        if tick == total {
            break;
        }
        let commands = burst.boundary_commands(tick, total, &scheduled)?;
        if !commands.is_empty() {
            let before = state_digest(&burst.sample.world)?;
            let cursor = burst.sample.cursor;
            let first_receipt = journal.count;
            for command in &commands {
                let rank = command.logical_rank as usize;
                let old_incarnation = burst
                    .incarnations
                    .get(rank)
                    .copied()
                    .ok_or_else(|| failure("rank", rank))?;
                let before_cursor = burst.sample.cursor;
                let committed = burst.command(command)?;
                journal.append(
                    command,
                    tick,
                    [before_cursor, burst.sample.cursor],
                    [old_incarnation, burst.incarnations[rank]],
                    committed,
                )?;
                if !committed {
                    burst.pending.push(command.clone());
                }
            }
            burst.refresh_order();
            let batch = json!({"boundary":tick,"command_count":commands.len(),"first_receipt":first_receipt,
                "input_digest":input_digest(&commands),"pre_command_state_digest":before,
                "post_command_state_digest":state_digest(&burst.sample.world)?,"cursor_before":cursor,"cursor_after":burst.sample.cursor});
            batches.push(batch);
            crate::write_json(
                &output.join("batches.json"),
                &json!({"complete":false,"batches":batches}),
            )?;
            journal.output.flush()?;
        }
        outcome = Some(
            burst
                .sample
                .world
                .step(TickInput::new(u64::from(plan.identity.fixed_delta_ms)))
                .map_err(|e| failure(&format!("step {}", tick + 1), e))?,
        );
    }
    burst.finish(plan)?;
    require(
        burst.sample.cursor - initial_cursor == u64::from(plan.identity.scale / 100) * 6
            && journal.count == burst.sample.cursor - initial_cursor + journal.blocked,
        || "固定命令总数不符".into(),
    )?;
    trace.flush()?;
    journal.output.flush()?;
    crate::write_json(
        &output.join("batches.json"),
        &json!({"complete":true,"batches":batches}),
    )?;
    let mut step_bytes = b"lf-synth-step-input-1\0".to_vec();
    step_bytes.extend_from_slice(&u64::from(plan.identity.fixed_delta_ms).to_le_bytes());
    Ok(json!({"format":"lf-synth-burst-run-1","complete":true,
        "scope":"W3_W4_semantics_only_not_retry_or_performance_or_product_certification",
        "implementation_commit":implementation,"input_manifest":bundle.manifest(),
        "warm_up_ticks":plan.warm_up_ticks,"observation_ticks":plan.observation_ticks,"validated_boundaries":total+1,
        "caller_commands_after_initialization":burst.sample.cursor-initial_cursor,
        "command_attempts":journal.count,"blocked_attempts":journal.blocked,
        "command_receipts_digest":journal.digest.finalize().to_hex().to_string(),"command_batches":batches,
        "step_input_digest":blake3::hash(&step_bytes).to_hex().to_string(),
        "host_completion_observations":burst.completions,"parking_arrivals":burst.arrival_records,
        "parking_commit_boundary":burst.parked_boundary,"parking_left":burst.left,
        "active":burst.counts[0],"parked":burst.counts[1],"completed":burst.counts[2],
        "gate_crossings":burst.sample.transitions,
        "max_observed_deceleration_mm_s2":burst.sample.max_deceleration_mm_s2,
        "deceleration_above_profile_count":burst.sample.deceleration_above_profile_count,
        "public_trace_digest":stream.finalize().to_hex().to_string(),"checkpoints":checkpoints}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use laneflow_synth_generator::{Identity, Provenance, generate};

    #[test]
    fn reservation_and_barrier_reject_duplicate_and_extra_boundaries() {
        let bundle = generate(
            Identity::new(10_000, Case::W3, 33).unwrap(),
            Provenance {
                source_commit: "a".repeat(40),
                generator_commit: "b".repeat(40),
            },
        )
        .unwrap();
        let plan = Plan::read(&bundle).unwrap();
        let mut burst = Burst::new(&bundle, &plan).unwrap();
        let scheduled = plan
            .scheduled
            .iter()
            .cloned()
            .map(serde_json::from_value)
            .collect::<Result<Vec<Scheduled>, _>>()
            .unwrap();
        burst.observe(0, plan.warm_up_ticks, None).unwrap();
        let total = plan.warm_up_ticks + plan.observation_ticks;
        let commands = burst.boundary_commands(0, total, &scheduled).unwrap();
        assert_eq!(commands.len(), 100);
        assert!(!burst.left);
        assert!(
            commands
                .iter()
                .all(|c| c.operation == Operation::ReserveParking)
        );
        for command in &commands {
            assert!(burst.command(command).unwrap());
        }
        let outcome = burst.sample.world.step(TickInput::new(33)).unwrap();
        burst
            .observe(1, plan.warm_up_ticks, Some(&outcome))
            .unwrap();
        assert!(
            burst
                .boundary_commands(1, total, &scheduled)
                .unwrap()
                .is_empty()
        );
        assert!(burst.boundary_commands(total, total, &scheduled).is_err());
        burst.arrivals.fill(Some(total - 2));
        assert!(
            burst
                .boundary_commands(total - 1, total, &scheduled)
                .is_err()
        );
        let park = burst
            .boundary_commands(total - 2, total, &scheduled)
            .unwrap();
        assert_eq!(park.len(), 100);
        assert!(park.iter().all(|c| c.operation == Operation::ParkVehicle));
        let leave = burst
            .boundary_commands(total - 1, total, &scheduled)
            .unwrap();
        assert_eq!(leave.len(), 100);
        assert!(leave.iter().all(|c| c.operation == Operation::LeaveParking));
        assert!(burst.command(&commands[0]).is_err());
        assert!(
            burst
                .observe(1, plan.warm_up_ticks, Some(&outcome))
                .is_err()
        );
    }

    #[test]
    fn command_input_digest_commits_to_order_and_payload() {
        let first = parking_command(Operation::ParkVehicle, 0);
        let second = parking_command(Operation::LeaveParking, 1);
        let expected = input_digest(&[first.clone(), second.clone()]);
        assert_ne!(expected, input_digest(&[second.clone(), first.clone()]));
        let mut changed = first;
        changed.progress_mm += 1;
        assert_ne!(expected, input_digest(&[changed, second]));
    }
}
