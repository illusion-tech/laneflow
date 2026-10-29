use std::collections::HashMap;

use bevy_ecs::{entity::Entity, resource::Resource, world::World};
use laneflow_bevy::{LaneFlowSession, LaneFlowVehicleReplaceOutcome};
use laneflow_runtime::*;
use laneflow_static_contract::{ParkingSpaceId, ParkingSpaceOrdinal};
use laneflow_synth_generator::Case;
use serde_json::{Value, json};

use crate::burst::{Command, Operation, ReplacementRule, Scheduled, parking_command};
use crate::{Error, Plan, failure, install::Sample, require};

use super::events::Recorder;

/// 宿主只观察计划指名的 completion 候选和实际 arrival；不预测到达拍数。
#[derive(Resource)]
pub(super) struct Driver {
    pub handles: Vec<VehicleHandle>,
    pub ranks: HashMap<VehicleHandle, usize>,
    pub entities: Vec<Option<Entity>>,
    routes: Vec<RouteHandle>,
    profile: laneflow_static_contract::VehicleProfileOrdinal,
    spaces: Vec<ParkingSpaceOrdinal>,
    scheduled: Vec<Scheduled>,
    replacement: Option<ReplacementRule>,
    pending: Vec<Command>,
    completion_candidates: Vec<usize>,
    incarnations: Vec<u32>,
    arrivals: Vec<bool>,
    arrival_count: usize,
    parked_at: Option<u64>,
    case: Case,
    pub events: Recorder,
    pub receipts: Vec<u8>,
    pub command_count: u64,
    pub blocked_count: u64,
    pub counts: [u32; 3],
    pub lifecycle_commands: usize,
    pub completion_records: Vec<(u64, usize)>,
    pub arrival_records: Vec<(u64, usize)>,
    pub batches: Vec<Value>,
}

impl Driver {
    pub fn new(sample: &Sample, plan: &Plan) -> Result<Self, Error> {
        let size = sample.handles.len();
        let spaces = sample
            .state
            .vehicles
            .iter()
            .map(|v| {
                let id = v
                    .parking_space_id
                    .parse::<ParkingSpaceId>()
                    .map_err(|e| failure("parking id", e))?;
                sample
                    .world
                    .revision()
                    .identity()
                    .ordinal(id)
                    .ok_or_else(|| failure("parking ordinal", id))
            })
            .collect::<Result<_, _>>()?;
        let replacement = plan
            .rules
            .iter()
            .find(|r| r["kind"] == "completed_replacement")
            .map(|r| serde_json::from_value(r.clone()))
            .transpose()?;
        let mut counts = [0; 3];
        for s in &sample.initial {
            counts[status_index(s.status())] += 1;
        }
        Ok(Self {
            handles: sample.handles.clone(),
            ranks: sample
                .handles
                .iter()
                .copied()
                .enumerate()
                .map(|(r, h)| (h, r))
                .collect(),
            entities: vec![None; size],
            routes: sample.routes.clone(),
            profile: sample.initial[0].profile(),
            spaces,
            scheduled: plan
                .scheduled
                .iter()
                .cloned()
                .map(serde_json::from_value)
                .collect::<Result<_, _>>()?,
            replacement,
            pending: Vec::new(),
            completion_candidates: Vec::new(),
            incarnations: vec![0; size],
            arrivals: vec![false; size / 100],
            arrival_count: 0,
            parked_at: None,
            case: plan.identity.case,
            events: Recorder::new(
                plan.warm_up_ticks + plan.observation_ticks + 1,
                plan.identity.scale,
            ),
            receipts: b"lf-synth-command-receipts-1\0".to_vec(),
            command_count: 0,
            blocked_count: 0,
            counts,
            lifecycle_commands: 0,
            completion_records: Vec::new(),
            arrival_records: Vec::new(),
            batches: Vec::new(),
        })
    }

    pub fn lifecycle(&mut self, ecs: &mut World) -> Result<(), Error> {
        let boundary = ecs.resource::<LaneFlowSession>().world().tick_index();
        let mut commands = Vec::new();
        if self.case == Case::W3 {
            if self.arrival_count == self.arrivals.len() && self.parked_at.is_none() {
                self.parked_at = Some(boundary);
                commands.extend(
                    (0..self.arrivals.len())
                        .map(|cell| parking_command(Operation::ParkVehicle, cell)),
                );
            } else if self.parked_at.is_some_and(|t| boundary == t + 1) {
                commands.extend(
                    (0..self.arrivals.len())
                        .map(|cell| parking_command(Operation::LeaveParking, cell)),
                );
            }
            commands.append(&mut self.pending);
        }
        if let Some(batch) = self.scheduled.iter().find(|b| b.boundary == boundary) {
            commands.extend(batch.commands.clone());
        }
        if commands.is_empty() {
            return Ok(());
        }
        let cursor_before = ecs.resource::<LaneFlowSession>().world().command_cursor();
        let first_receipt = self.command_count;
        for command in &commands {
            let rank = command.logical_rank as usize;
            let before = ecs.resource::<LaneFlowSession>().world().command_cursor();
            let old_incarnation = self.incarnations[rank];
            let committed = self.command(ecs, command)?;
            let after = ecs.resource::<LaneFlowSession>().world().command_cursor();
            require(after == before + u64::from(committed), || {
                "宿主命令游标不符".into()
            })?;
            for value in [boundary, self.command_count, before, after] {
                self.receipts.extend_from_slice(&value.to_le_bytes());
            }
            command.encode(&mut self.receipts);
            for value in [
                old_incarnation,
                self.incarnations[rank],
                u32::from(!committed),
            ] {
                self.receipts.extend_from_slice(&value.to_le_bytes());
            }
            self.command_count += 1;
            self.blocked_count += u64::from(!committed);
            if !committed {
                self.pending.push(command.clone());
            }
        }
        self.lifecycle_commands += commands.len();
        self.batches.push(json!({"boundary":boundary,"command_count":commands.len(),
            "first_receipt":first_receipt,"input_digest":crate::burst::input_digest(&commands),
            "cursor_before":cursor_before,"cursor_after":ecs.resource::<LaneFlowSession>().world().command_cursor()}));
        Ok(())
    }

    fn command(&mut self, ecs: &mut World, command: &Command) -> Result<bool, Error> {
        let rank = command.logical_rank as usize;
        require(
            rank < self.handles.len() && command.route_rank as usize == rank / 25,
            || "命令 rank 越界".into(),
        )?;
        let old = self.handles[rank];
        let route = self.routes[rank / 25];
        let space = self.spaces[rank];
        let before = ecs
            .resource::<LaneFlowSession>()
            .world()
            .vehicle(old)
            .map(|s| s.status());
        let input = VehicleSpawnInput::new(
            self.profile,
            route,
            command.edge_occurrence,
            command.progress_mm,
            command.speed_mm_per_s,
        );
        let mut new = None;
        match command.operation {
            Operation::ReserveParking => {
                let result = ecs
                    .resource_mut::<LaneFlowSession>()
                    .world_mut()
                    .reserve_parking(
                        old,
                        ReserveParkingTarget::ExplicitSpace {
                            space,
                            entry_route_occurrence: command.edge_occurrence,
                        },
                    )
                    .map_err(|e| failure("reserve", e))?;
                require(
                    matches!(result, ParkingCommandOutcome::Committed(_)),
                    || "reserve NoChange".into(),
                )?;
            }
            Operation::ParkVehicle => {
                let result = ecs
                    .resource_mut::<LaneFlowSession>()
                    .world_mut()
                    .park_vehicle(old, ParkingTarget::ExplicitSpace(space))
                    .map_err(|e| failure("park", e))?;
                require(
                    matches!(result, ParkingCommandOutcome::Committed(_)),
                    || "park NoChange".into(),
                )?;
            }
            Operation::LeaveParking => {
                ecs.resource_mut::<LaneFlowSession>()
                    .world_mut()
                    .leave_parking(
                        old,
                        LeaveParkingTarget::ExplicitSpace {
                            space,
                            route,
                            exit_route_occurrence: command.edge_occurrence,
                        },
                    )
                    .map_err(|e| failure("leave", e))?;
            }
            Operation::Despawn => {
                let result = laneflow_bevy::despawn_vehicle(ecs, old)
                    .map_err(|e| failure("typed despawn", e))?;
                require(
                    result.runtime.vehicle == old && result.entity == self.entities[rank],
                    || "typed despawn 映射不符".into(),
                )?;
                self.ranks.remove(&old);
            }
            Operation::Spawn => {
                let handle = ecs
                    .resource_mut::<LaneFlowSession>()
                    .world_mut()
                    .spawn_vehicle(input)
                    .map_err(|e| failure("spawn", e))?;
                if let Some(entity) = self.entities[rank] {
                    ecs.resource_mut::<LaneFlowSession>()
                        .bind_vehicle_entity(handle, entity)
                        .map_err(|e| failure("spawn bind", e))?;
                }
                new = Some(handle);
            }
            Operation::AtomicReplace => {
                match laneflow_bevy::replace_completed_vehicle(ecs, old, input)
                    .map_err(|e| failure("typed replace", e))?
                {
                    LaneFlowVehicleReplaceOutcome::Replaced(record) => {
                        require(
                            record.old == old && record.entity == self.entities[rank],
                            || "replace Entity 不连续".into(),
                        )?;
                        new = Some(record.new);
                    }
                    LaneFlowVehicleReplaceOutcome::Blocked(_) if self.case == Case::W3 => {
                        return Ok(false);
                    }
                    other => return Err(failure("未声明的 replacement 结果", other)),
                }
            }
        }
        if let Some(handle) = new {
            require(
                handle != old
                    && ecs
                        .resource::<LaneFlowSession>()
                        .world()
                        .vehicle(old)
                        .is_none(),
                || "旧句柄未失效".into(),
            )?;
            self.ranks.remove(&old);
            require(self.ranks.insert(handle, rank).is_none(), || {
                "句柄重复".into()
            })?;
            self.handles[rank] = handle;
            self.incarnations[rank] += 1;
            if command.operation == Operation::Spawn {
                self.completion_candidates.push(rank);
            }
        }
        let after = ecs
            .resource::<LaneFlowSession>()
            .world()
            .vehicle(self.handles[rank])
            .map(|s| s.status());
        if let Some(status) = before {
            self.counts[status_index(status)] -= 1;
        }
        if let Some(status) = after {
            self.counts[status_index(status)] += 1;
        }
        Ok(true)
    }

    pub fn observe(
        &mut self,
        world: &TrafficWorld,
        outcome: Option<&StepOutcome>,
    ) -> Result<(), Error> {
        self.events.record(world, &self.ranks, outcome)?;
        let tick = world.tick_index();
        if let Some(outcome) = outcome {
            for arrival in outcome.parking_arrivals() {
                let rank = *self
                    .ranks
                    .get(&arrival.vehicle)
                    .ok_or_else(|| failure("arrival handle", tick))?;
                require(
                    self.case == Case::W3
                        && rank.is_multiple_of(100)
                        && !self.arrivals[rank / 100]
                        && arrival.target == ParkingTarget::ExplicitSpace(self.spaces[rank]),
                    || "arrival 重复或目标不符".into(),
                )?;
                self.arrivals[rank / 100] = true;
                self.arrival_count += 1;
                self.arrival_records.push((tick, rank));
            }
        }
        let mut pending_completion = Vec::new();
        for &rank in &self.completion_candidates {
            let status = world
                .vehicle(self.handles[rank])
                .ok_or_else(|| failure("completion handle", rank))?
                .status();
            if status == VehicleStatus::Completed {
                self.counts[0] -= 1;
                self.counts[2] += 1;
                self.completion_records.push((tick, rank));
                if let Some(rule) = &self.replacement {
                    require(rule.ranks.binary_search(&(rank as u32)).is_ok(), || {
                        "未声明的 replacement rank".into()
                    })?;
                    self.pending.push(Command {
                        operation: Operation::AtomicReplace,
                        logical_rank: rank as u32,
                        route_rank: rank as u32 / 25,
                        edge_occurrence: rule.edge_occurrence,
                        progress_mm: rule.progress_mm,
                        speed_mm_per_s: rule.speed_mm_per_s,
                    });
                }
            } else {
                pending_completion.push(rank);
            }
        }
        self.completion_candidates = pending_completion;
        Ok(())
    }

    pub fn validate_population(&self, world: &TrafficWorld) -> Result<(), Error> {
        let mut actual = [0; 3];
        for &handle in &self.handles {
            let state = world
                .vehicle(handle)
                .ok_or_else(|| failure("population live handle", handle))?;
            actual[status_index(state.status())] += 1;
        }
        require(
            actual == self.counts && self.ranks.len() == self.handles.len(),
            || "population ledger 不符".into(),
        )
    }
}

fn status_index(status: VehicleStatus) -> usize {
    match status {
        VehicleStatus::Active => 0,
        VehicleStatus::Parked => 1,
        VehicleStatus::Completed => 2,
    }
}
