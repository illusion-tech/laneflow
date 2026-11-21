mod observe;
pub mod restore;

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use laneflow_runtime::*;
use laneflow_static_contract::{ParkingSpaceId, ParkingSpaceOrdinal};
use laneflow_synth_generator::{Bundle, Case};
use laneflow_traffic_verification::session::CommandResult;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::verification::{RULE_COMMANDS, WITNESS_LEAVE, WITNESS_PARK};
use crate::verification_cost::{Buffer, Phase, Probe, measured};
use crate::{Error, Plan, failure, install::Sample, require};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Command {
    pub operation: Operation,
    pub logical_rank: u32,
    pub route_rank: u32,
    pub edge_occurrence: u32,
    pub progress_mm: u32,
    pub speed_mm_per_s: u32,
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

    pub(crate) fn encode(&self, target: &mut Vec<u8>) {
        for word in self.words() {
            target.extend_from_slice(&word.to_le_bytes());
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scheduled {
    pub boundary: u64,
    pub commands: Vec<Command>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReplacementRule {
    pub ranks: Vec<u32>,
    pub edge_occurrence: u32,
    pub progress_mm: u32,
    pub speed_mm_per_s: u32,
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
    fn cost_buffers(&self, probe: &mut impl Probe) {
        crate::verification_cost::sample_buffers(probe, &self.sample);
        for (buffer, length, capacity, item_bytes) in [
            (
                Buffer::LaneOrder,
                self.lane_order.len(),
                self.lane_order.capacity(),
                size_of::<usize>(),
            ),
            (
                Buffer::Changed,
                self.changed.len(),
                self.changed.capacity(),
                size_of::<bool>(),
            ),
            (
                Buffer::Pending,
                self.pending.len(),
                self.pending.capacity(),
                size_of::<Command>(),
            ),
        ] {
            probe.buffer(buffer, length, capacity, item_bytes);
        }
    }

    fn new(bundle: &Bundle, plan: &Plan, execution: ExecutionConfig) -> Result<Self, Error> {
        let sample = Sample::new(bundle, execution)?;
        let spaces = sample
            .state
            .vehicles
            .iter()
            .map(|vehicle| {
                let id = vehicle
                    .parking_space_id
                    .as_deref()
                    .ok_or_else(|| failure("parking id", "missing"))?
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
            "public_state_digest":crate::public_state_digest(&self.sample.world)?,
            "population":{"active":self.counts[0],"parked":self.counts[1],"completed":self.counts[2]},
            "command_cursor":self.sample.cursor,"gate_crossings":self.sample.transitions,
            "verification":crate::verification_material(&self.sample)?}),
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

    /// 会话协议包裹的单条边界命令执行（设计 §6 命令结果阶段）：命令权限
    /// 规则在本条命令的开放义务内登记实际运行，宿主核验失败把违反记到
    /// 该规则后如实上抛；随后按实际结果消费——成功推进成功命令游标，
    /// Blocked 只计尝试与拒绝统计，不推进游标、代次或位置。W3 场景见证
    /// 在提交点记录屏障停车/离位。
    fn execute_command(&mut self, command: &Command) -> Result<bool, Error> {
        let committed = match self.command(command) {
            Ok(committed) => committed,
            Err(error) => {
                crate::observe::session_violation(&mut self.sample.session, RULE_COMMANDS.id);
                return Err(error);
            }
        };
        crate::observe::session_note(&mut self.sample.session, RULE_COMMANDS)?;
        crate::observe::session_result(
            &mut self.sample.session,
            if committed {
                CommandResult::Succeeded
            } else {
                CommandResult::Blocked
            },
        )?;
        if committed && self.sample.state.identity.case == Case::W3 {
            match command.operation {
                Operation::ParkVehicle => {
                    crate::observe::session_witness(&mut self.sample.session, WITNESS_PARK)?;
                }
                Operation::LeaveParking => {
                    crate::observe::session_witness(&mut self.sample.session, WITNESS_LEAVE)?;
                }
                _ => {}
            }
        }
        Ok(committed)
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

pub(crate) fn parking_command(operation: Operation, cell: usize) -> Command {
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

pub(crate) fn state_digest(world: &TrafficWorld) -> Result<String, Error> {
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

pub(crate) fn input_digest(commands: &[Command]) -> String {
    let mut bytes = b"lf-synth-command-batch-1\0".to_vec();
    bytes.extend_from_slice(&(commands.len() as u32).to_le_bytes());
    for command in commands {
        command.encode(&mut bytes);
    }
    blake3::hash(&bytes).to_hex().to_string()
}

#[cfg(test)]
pub(super) fn run(
    bundle: &Bundle,
    plan: &Plan,
    implementation: &str,
    execution: ExecutionConfig,
    output: &Path,
) -> Result<Value, Error> {
    run_with_probe(
        bundle,
        plan,
        implementation,
        execution,
        output,
        &mut crate::verification_cost::Noop,
    )
}

pub(super) fn run_with_probe<P: Probe>(
    bundle: &Bundle,
    plan: &Plan,
    implementation: &str,
    execution: ExecutionConfig,
    output: &Path,
    probe: &mut P,
) -> Result<Value, Error> {
    let scheduled: Vec<Scheduled> = plan
        .scheduled
        .iter()
        .cloned()
        .map(serde_json::from_value)
        .collect::<Result<_, _>>()?;
    let total = plan.warm_up_ticks + plan.observation_ticks;
    let mut burst = measured(probe, Phase::Setup, |_| {
        // 诊断对照 D（experiment/1457-cD-synth-session）：摘除 W3/W4 burst
        // 会话接线——不绑定共享验证会话；execute_command 的记检/见证、
        // 步进/命令协议与收据产出点全部对 None 容忍，final 报告的
        // verification_receipt 与检查点验证材料为 null。
        Burst::new(bundle, plan, execution)
    })?;
    burst.cost_buffers(probe);
    let setup = probe.start();
    let initial_cursor = burst.sample.cursor;
    let mut journal = Journal::new(&output.join("commands.bin"))?;
    let mut trace = BufWriter::new(File::create(output.join("ticks.bin"))?);
    trace.write_all(b"lf-synth-burst-ticks-1\0")?;
    let mut stream = blake3::Hasher::new();
    stream.update(b"lf-synth-burst-stream-1\0");
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
    let mut events = crate::measurement::events::Recorder::new(total + 1, plan.identity.scale);
    probe.end(Phase::Setup, setup);
    for tick in 0..=total {
        probe.boundary(tick);
        let boundary_digest = measured(probe, Phase::Observation, |probe| {
            burst.observe_with_probe(tick, plan.warm_up_ticks, outcome.as_ref(), probe)
        })?;
        measured(probe, Phase::Session, |_| {
            if tick > 0 {
                // 步进结果先于检查点与命令恰核销一次（设计 §6）。
                if let Some(session) = burst.sample.session.as_mut() {
                    session
                        .note_successful_step(tick)
                        .map_err(|e| failure("验证会话步进协议", e))?;
                }
            }
            Ok(())
        })?;
        burst.cost_buffers(probe);
        measured(probe, Phase::Evidence, |_| {
            events.record(
                &burst.sample.world,
                &burst.handle_ranks,
                &burst.sample.routes,
                outcome.as_ref(),
            )?;
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
            Ok(())
        })?;
        if tick == total {
            probe.boundary_complete();
            break;
        }
        probe.command_boundary(tick);
        let commands = measured(probe, Phase::Commands, |_| {
            burst.boundary_commands(tick, total, &scheduled)
        })?;
        // 会话命令阶段：声明本边界命令总数（可为零）后逐条执行；每条
        // 命令的义务在结果消费时核销（设计 §6 命令结果阶段）。
        measured(probe, Phase::Session, |_| {
            if let Some(session) = burst.sample.session.as_mut() {
                session
                    .declare_commands(commands.len() as u32)
                    .map_err(|e| failure("验证会话命令阶段", e))?;
            }
            Ok(())
        })?;
        measured(probe, Phase::Commands, |_| {
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
                    let committed = burst.execute_command(command)?;
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
            Ok(())
        })?;
        burst.cost_buffers(probe);
        // 命令结束：全部消费后形成拍前事实并立即发起步进；零命令边界
        // 同样经过完整的阶段协议。
        measured(probe, Phase::Session, |_| {
            if let Some(session) = burst.sample.session.as_mut() {
                session
                    .advance_to_pre_tick()
                    .map_err(|e| failure("验证会话拍前阶段", e))?;
                session
                    .begin_step()
                    .map_err(|e| failure("验证会话步进发起", e))?;
            }
            Ok(())
        })?;
        probe.runtime_step_tick(tick + 1);
        outcome = Some(measured(probe, Phase::RuntimeStep, |_| {
            burst
                .sample
                .world
                .step(TickInput::new(u64::from(plan.identity.fixed_delta_ms)))
                .map_err(|e| failure(&format!("step {}", tick + 1), e))
        })?);
        probe.boundary_complete();
    }
    let finish = probe.start();
    burst.finish(plan)?;
    require(
        burst.sample.cursor - initial_cursor == u64::from(plan.identity.scale / 100) * 6
            && journal.count == burst.sample.cursor - initial_cursor + journal.blocked,
        || "固定命令总数不符".into(),
    )?;
    // 会话终结：完整窗口、逐轮覆盖、必需场景见证（到达/停车/离位/Blocked
    // 重试/Green 探针）与无违反同时成立才产生回执（设计 §3.3/§10）。
    let verification_receipt = match burst.sample.session.as_mut() {
        Some(session) => {
            let receipt = session
                .finish()
                .map_err(|e| Error::Invalid(format!("验证会话终结被拒：{e:?}")))?;
            Some(crate::receipt_json(&receipt))
        }
        None => None,
    };
    probe.end(Phase::Finish, finish);
    trace.flush()?;
    journal.output.flush()?;
    events.write(&output.join("events.bin"))?;
    crate::write_json(
        &output.join("batches.json"),
        &json!({"complete":true,"batches":batches}),
    )?;
    let mut step_bytes = b"lf-synth-step-input-1\0".to_vec();
    step_bytes.extend_from_slice(&u64::from(plan.identity.fixed_delta_ms).to_le_bytes());
    Ok(json!({"format":"lf-synth-burst-run-1","complete":true,
        "semantic_contract":crate::SEMANTIC_CONTRACT,
        "scope":"W3_W4_semantics_only_not_retry_or_performance_or_product_certification",
        "implementation_commit":implementation,"input_manifest":bundle.manifest(),
        "workers":burst.sample.world.execution_config().worker_count().get(),
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
        "public_trace_digest":stream.finalize().to_hex().to_string(),"checkpoints":checkpoints,
        "verification_receipt":verification_receipt,
        "ordered_events_digest":events.digest()}))
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
        let mut burst = Burst::new(
            &bundle,
            &plan,
            ExecutionConfig::new(std::num::NonZeroU32::MIN),
        )
        .unwrap();
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

    /// 压缩时序的 W3 会话接线（与恢复 E2E 同法：只压缩宿主时间安排、
    /// 平移计划边界，不改冻结输入字节）：真实 Runtime、真实命令。冻结
    /// W3 输入的完成替换在替换位恒为空走廊、不被拒（走廊 3 的 75–80
    /// 已前行、81 刚被替换掉、82–99 停驶）；钉住：每条命令恰一次义务
    /// 核销、提交数即成功命令游标增量、压缩窗口缺必需见证不得冒充完整
    /// 通过（§3.3/§10）。Blocked 容错语义见注入探针测试。
    #[test]
    fn w3_burst_session_consumes_command_results() {
        use laneflow_traffic_verification::session::FinishRejected;
        use laneflow_traffic_verification::spec::RuleId;

        let bundle = generate(
            Identity::new(10_000, Case::W3, 16).unwrap(),
            Provenance {
                source_commit: "a".repeat(40),
                generator_commit: "b".repeat(40),
            },
        )
        .unwrap();
        let mut plan = Plan::read(&bundle).unwrap();
        let old_warm = plan.warm_up_ticks;
        plan.warm_up_ticks = 1;
        plan.observation_ticks = 5;
        let mut scheduled: Vec<Scheduled> = plan
            .scheduled
            .iter()
            .cloned()
            .map(serde_json::from_value)
            .collect::<Result<_, _>>()
            .unwrap();
        for batch in &mut scheduled {
            if batch.boundary > 0 {
                batch.boundary = batch.boundary + 1 - old_warm;
            }
        }
        let end = plan.warm_up_ticks + plan.observation_ticks;
        let mut burst = Burst::new(
            &bundle,
            &plan,
            ExecutionConfig::new(std::num::NonZeroU32::MIN),
        )
        .unwrap();
        let case = plan.identity.case.clone();
        crate::verification::bind_burst_session(&mut burst.sample, &bundle, &case, end).unwrap();
        let initial_cursor = burst.sample.cursor;
        let mut consumed = 0_u64;
        let mut outcome = None;
        for tick in 0..=end {
            burst
                .observe(tick, plan.warm_up_ticks, outcome.as_ref())
                .unwrap();
            if tick > 0 {
                burst
                    .sample
                    .session
                    .as_mut()
                    .unwrap()
                    .note_successful_step(tick)
                    .unwrap();
            }
            if tick == end {
                break;
            }
            let commands = burst.boundary_commands(tick, end, &scheduled).unwrap();
            burst
                .sample
                .session
                .as_mut()
                .unwrap()
                .declare_commands(commands.len() as u32)
                .unwrap();
            for command in &commands {
                assert!(burst.execute_command(command).unwrap());
                consumed += 1;
            }
            burst.refresh_order();
            let session = burst.sample.session.as_mut().unwrap();
            session.advance_to_pre_tick().unwrap();
            session.begin_step().unwrap();
            outcome = Some(burst.sample.world.step(TickInput::new(16)).unwrap());
        }
        // 冻结输入全程无 Blocked：提交数恰为成功命令游标增量。
        assert_eq!(
            burst.sample.cursor - initial_cursor,
            consumed,
            "无 Blocked 时提交数即游标增量"
        );
        // 每条命令恰一次义务核销：命令权限规则触发数 == 实际回执数。
        let session = burst.sample.session.as_ref().unwrap();
        assert_eq!(
            session.trigger_count(RuleId("w-command-authority")),
            consumed
        );
        let material =
            serde_json::to_value(session.checkpoint(&burst.sample.input_identity).unwrap())
                .unwrap();
        assert_eq!(material["blocked_attempts"].as_u64().unwrap(), 0);
        assert_eq!(
            material["successful_commands"].as_u64().unwrap(),
            burst.sample.cursor - initial_cursor
        );
        assert_eq!(
            material["trigger_counts"]["w-motion-identity"]
                .as_u64()
                .unwrap(),
            end + 1,
            "初态轮 + 每个成功步进各一次"
        );
        // 压缩窗口内到达/停车/离位见证未发生：不得凭缺失见证通过（§3.3）。
        let session = burst.sample.session.as_mut().unwrap();
        assert!(matches!(
            session.finish(),
            Err(FinishRejected::WitnessMissing(_))
        ));
        assert_eq!(session.finish(), Err(FinishRejected::DoubleFinish));
    }

    /// Blocked 容错语义注入探针（§10 命令行“Blocked 不推进成功游标及
    /// 位置”）：压缩 W3 上，在替换位（走廊 3 的 183.5–188 m）先经
    /// despawn/spawn 放置一辆 Active 阻挡车，完成替换被 Runtime 真实
    /// 拒绝（`ReplaceError::Blocked`）——只计尝试、不推进成功命令游标
    /// 与代次、宿主世界保持不变；同输入在后一边界（阻挡清除后）重试
    /// 提交。冻结输入自身不触发 Blocked，语义由此探针钉住。
    #[test]
    fn w3_burst_session_blocked_command_neither_advances_cursor_nor_incarnation() {
        let bundle = generate(
            Identity::new(10_000, Case::W3, 16).unwrap(),
            Provenance {
                source_commit: "a".repeat(40),
                generator_commit: "b".repeat(40),
            },
        )
        .unwrap();
        let mut plan = Plan::read(&bundle).unwrap();
        let old_warm = plan.warm_up_ticks;
        plan.warm_up_ticks = 1;
        plan.observation_ticks = 3;
        let mut scheduled: Vec<Scheduled> = plan
            .scheduled
            .iter()
            .cloned()
            .map(serde_json::from_value)
            .collect::<Result<_, _>>()
            .unwrap();
        for batch in &mut scheduled {
            if batch.boundary > 0 {
                batch.boundary = batch.boundary + 1 - old_warm;
            }
        }
        let end = plan.warm_up_ticks + plan.observation_ticks;
        let mut burst = Burst::new(
            &bundle,
            &plan,
            ExecutionConfig::new(std::num::NonZeroU32::MIN),
        )
        .unwrap();
        let case = plan.identity.case.clone();
        crate::verification::bind_burst_session(&mut burst.sample, &bundle, &case, end).unwrap();
        let initial_cursor = burst.sample.cursor;
        let mut outcome = None;
        // 边界 0/1 按冻结计划执行（预约批次、rank 81 终态重建）。
        for tick in 0..=1 {
            burst
                .observe(tick, plan.warm_up_ticks, outcome.as_ref())
                .unwrap();
            if tick > 0 {
                burst
                    .sample
                    .session
                    .as_mut()
                    .unwrap()
                    .note_successful_step(tick)
                    .unwrap();
            }
            let commands = burst.boundary_commands(tick, end, &scheduled).unwrap();
            burst
                .sample
                .session
                .as_mut()
                .unwrap()
                .declare_commands(commands.len() as u32)
                .unwrap();
            for command in &commands {
                assert!(burst.execute_command(command).unwrap());
            }
            burst.refresh_order();
            let session = burst.sample.session.as_mut().unwrap();
            session.advance_to_pre_tick().unwrap();
            session.begin_step().unwrap();
            outcome = Some(burst.sample.world.step(TickInput::new(16)).unwrap());
        }
        // 边界 2：observe 看到全部 rank 81 完成，完成替换入队。执行前把
        // 阻挡车命令插到队首（不经 pending——observe 以 pending.len() 对
        // 账完成数，注入队列会破坏该不变量）：despawn(75) 后 spawn 到
        // 186 000 mm，车身 [181.5, 186] m 与 cell 0 替换目标 [183.5, 188] m
        // 相交；其余 99 个 cell 的替换在各自走廊，不受影响。
        burst
            .observe(2, plan.warm_up_ticks, outcome.as_ref())
            .unwrap();
        burst
            .sample
            .session
            .as_mut()
            .unwrap()
            .note_successful_step(2)
            .unwrap();
        let cursor_before = burst.sample.cursor;
        let incarnation_before = burst.incarnations[81];
        let mut commands = burst.boundary_commands(2, end, &scheduled).unwrap();
        assert_eq!(commands.len(), 100, "全部 cell 的完成替换");
        commands.splice(
            0..0,
            [
                Command {
                    operation: Operation::Despawn,
                    logical_rank: 75,
                    route_rank: 3,
                    edge_occurrence: 0,
                    progress_mm: 0,
                    speed_mm_per_s: 0,
                },
                Command {
                    operation: Operation::Spawn,
                    logical_rank: 75,
                    route_rank: 3,
                    edge_occurrence: 0,
                    progress_mm: 186_000,
                    speed_mm_per_s: 0,
                },
            ],
        );
        burst
            .sample
            .session
            .as_mut()
            .unwrap()
            .declare_commands(commands.len() as u32)
            .unwrap();
        let mut blocked_at_two = 0_u64;
        for command in &commands {
            let committed = burst.execute_command(command).unwrap();
            if command.logical_rank == 81 && command.route_rank == 3 {
                assert!(!committed, "cell 0 替换目标位被阻挡车占据必须真实 Blocked");
                blocked_at_two += 1;
                burst.pending.push(command.clone());
            } else {
                assert!(committed);
            }
        }
        assert_eq!(blocked_at_two, 1);
        assert_eq!(burst.pending.len(), 1);
        // Blocked 不推进成功命令游标、代次或位置：本边界游标增量只来自
        // 提交的命令（despawn+spawn+99 个替换），rank 81 代次保持不变。
        assert_eq!(burst.sample.cursor - cursor_before, 101);
        assert_eq!(burst.incarnations[81], incarnation_before);
        burst.refresh_order();
        let session = burst.sample.session.as_mut().unwrap();
        session.advance_to_pre_tick().unwrap();
        // 命令结束边界（AfterCommands）的会话材料：尝试计数如实分离。
        let material = serde_json::to_value(
            burst
                .sample
                .session
                .as_ref()
                .unwrap()
                .checkpoint(&burst.sample.input_identity)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(material["blocked_attempts"].as_u64().unwrap(), 1);
        assert_eq!(
            material["successful_commands"].as_u64().unwrap(),
            burst.sample.cursor - initial_cursor
        );
        let session = burst.sample.session.as_mut().unwrap();
        session.begin_step().unwrap();
        outcome = Some(burst.sample.world.step(TickInput::new(16)).unwrap());
        // 边界 3：observe 以 pending（仅剩被拒替换）对账完成数；执行前
        // 插入清阻挡命令，同输入重试完成替换并提交，代次随提交前进一次。
        burst
            .observe(3, plan.warm_up_ticks, outcome.as_ref())
            .unwrap();
        burst
            .sample
            .session
            .as_mut()
            .unwrap()
            .note_successful_step(3)
            .unwrap();
        let cursor_before = burst.sample.cursor;
        let mut commands = burst.boundary_commands(3, end, &scheduled).unwrap();
        assert_eq!(commands.len(), 1, "仅剩被拒替换在队");
        commands.insert(
            0,
            Command {
                operation: Operation::Despawn,
                logical_rank: 75,
                route_rank: 3,
                edge_occurrence: 0,
                progress_mm: 0,
                speed_mm_per_s: 0,
            },
        );
        assert_eq!(commands.len(), 2, "清阻挡 + 重试替换");
        burst
            .sample
            .session
            .as_mut()
            .unwrap()
            .declare_commands(commands.len() as u32)
            .unwrap();
        let mut replaced = false;
        for command in &commands {
            let committed = burst.execute_command(command).unwrap();
            if command.operation == Operation::AtomicReplace {
                assert!(committed, "阻挡清除后同输入重试必须提交");
                replaced = true;
            } else {
                assert!(committed);
            }
        }
        assert!(replaced);
        assert_eq!(burst.sample.cursor - cursor_before, 2);
        assert_eq!(burst.incarnations[81], incarnation_before + 1);
        assert!(burst.pending.is_empty());
        burst
            .sample
            .session
            .as_mut()
            .unwrap()
            .advance_to_pre_tick()
            .unwrap();
        let material = serde_json::to_value(
            burst
                .sample
                .session
                .as_ref()
                .unwrap()
                .checkpoint(&burst.sample.input_identity)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(material["blocked_attempts"].as_u64().unwrap(), 1);
        assert_eq!(
            material["successful_commands"].as_u64().unwrap(),
            burst.sample.cursor - initial_cursor
        );
    }

    /// 压缩时序的 W4 会话以真实见证正向终结：B1 指定过门（rank 61）的
    /// 许可原因恰为 Green，过门与 Green 探针见证全部来自真实执行，不经
    /// 注入；Blocked 为零（W4 计划 blocked 即失败）。窗口等于压缩终点，
    /// 回执只证明该范围（§8 探针消费口径）。
    #[test]
    fn w4_burst_session_finishes_with_real_green_probe() {
        let bundle = generate(
            Identity::new(10_000, Case::W4, 16).unwrap(),
            Provenance {
                source_commit: "a".repeat(40),
                generator_commit: "b".repeat(40),
            },
        )
        .unwrap();
        let mut plan = Plan::read(&bundle).unwrap();
        let old_warm = plan.warm_up_ticks;
        plan.warm_up_ticks = 1;
        plan.observation_ticks = 2;
        let mut scheduled: Vec<Scheduled> = plan
            .scheduled
            .iter()
            .cloned()
            .map(serde_json::from_value)
            .collect::<Result<_, _>>()
            .unwrap();
        for batch in &mut scheduled {
            if batch.boundary > 0 {
                batch.boundary = batch.boundary + 1 - old_warm;
            }
        }
        let end = plan.warm_up_ticks + plan.observation_ticks;
        let mut burst = Burst::new(
            &bundle,
            &plan,
            ExecutionConfig::new(std::num::NonZeroU32::MIN),
        )
        .unwrap();
        let case = plan.identity.case.clone();
        crate::verification::bind_burst_session(&mut burst.sample, &bundle, &case, end).unwrap();
        let initial_cursor = burst.sample.cursor;
        let mut consumed = 0_u64;
        let mut outcome = None;
        for tick in 0..=end {
            burst
                .observe(tick, plan.warm_up_ticks, outcome.as_ref())
                .unwrap();
            if tick > 0 {
                burst
                    .sample
                    .session
                    .as_mut()
                    .unwrap()
                    .note_successful_step(tick)
                    .unwrap();
            }
            if tick == end {
                break;
            }
            let commands = burst.boundary_commands(tick, end, &scheduled).unwrap();
            burst
                .sample
                .session
                .as_mut()
                .unwrap()
                .declare_commands(commands.len() as u32)
                .unwrap();
            for command in &commands {
                assert!(burst.execute_command(command).unwrap());
                consumed += 1;
            }
            burst.refresh_order();
            let session = burst.sample.session.as_mut().unwrap();
            session.advance_to_pre_tick().unwrap();
            session.begin_step().unwrap();
            outcome = Some(burst.sample.world.step(TickInput::new(16)).unwrap());
        }
        // 宿主侧完成断言（W4 必须在 B1 后结束）通过后，会话以真实见证终结。
        burst.finish(&plan).unwrap();
        assert_eq!(
            burst.sample.cursor - initial_cursor,
            consumed,
            "W4 无 Blocked，提交数即游标增量"
        );
        let receipt = burst
            .sample
            .session
            .as_mut()
            .unwrap()
            .finish()
            .expect("真实过门 + Green 探针见证在档的完整窗口必须成功终结");
        assert_eq!(receipt.spec_name(), "lf-synth-w4-burst-semantic-run");
        assert_eq!(receipt.successful_steps(), end);
        assert_eq!(receipt.blocked_attempts(), 0);
        let witnesses = receipt.witnesses();
        let cells = u64::from(plan.identity.scale / 100);
        assert_eq!(
            witnesses[&laneflow_traffic_verification::spec::WitnessId("per-active-crossing")],
            cells
        );
        assert_eq!(
            witnesses[&laneflow_traffic_verification::spec::WitnessId("w4-green-probe")],
            cells
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
