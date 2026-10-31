use std::{collections::HashMap, path::Path};

use laneflow_runtime::{
    ParkingTarget, StepOutcome, TrafficTransitionKind, TrafficWorld, VehicleHandle,
};

use crate::{Error, failure, require};

/// #1166：精确有序事件的内部证据，句柄转成逻辑 rank；不扫描全部车辆状态。
pub(crate) struct Recorder {
    bytes: Vec<u8>,
    next_tick: u64,
    pub transitions: u64,
    pub arrivals: u64,
}

impl Recorder {
    pub(crate) fn new(ticks: u64, scale: u32) -> Self {
        Self::from_tick(ticks, scale, 0)
    }

    pub(crate) fn from_tick(ticks: u64, scale: u32, next_tick: u64) -> Self {
        let mut bytes = Vec::with_capacity(ticks as usize * 32 + scale as usize * 100);
        bytes.extend_from_slice(b"lf-synth-ordered-events-1\0");
        Self {
            bytes,
            next_tick,
            transitions: 0,
            arrivals: 0,
        }
    }

    pub(crate) fn byte_length(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn record(
        &mut self,
        world: &TrafficWorld,
        ranks: &HashMap<VehicleHandle, usize>,
        outcome: Option<&StepOutcome>,
    ) -> Result<(), Error> {
        let tick = world.tick_index();
        require(tick == self.next_tick, || {
            format!("事件重复消费或跳拍：{tick}/{}", self.next_tick)
        })?;
        require(
            outcome.map_or(tick == 0, |o| {
                o.tick_index() == tick && o.time_ms() == world.time_ms()
            }),
            || "事件结果时刻不符".into(),
        )?;
        self.next_tick += 1;
        let events = world.latest_transition_events();
        let arrivals = outcome.map_or(&[][..], StepOutcome::parking_arrivals);
        for value in [tick, world.time_ms(), world.command_cursor()] {
            self.bytes.extend_from_slice(&value.to_le_bytes());
        }
        for count in [events.len(), arrivals.len()] {
            self.bytes.extend_from_slice(&(count as u32).to_le_bytes());
        }
        for event in events {
            let rank = *ranks
                .get(&event.vehicle())
                .ok_or_else(|| failure("事件句柄", tick))?;
            let anchor = event.anchor();
            require(
                event.tick() == tick
                    && world
                        .vehicle(event.vehicle())
                        .is_some_and(|s| s.route() == anchor.route()),
                || "事件 tick/路线权威不符".into(),
            )?;
            let (kind, payload) = match event.kind() {
                TrafficTransitionKind::GateCrossed { gate } => (1, gate.raw()),
                TrafficTransitionKind::ManeuverTraversalCompleted {
                    maneuver_occurrence_index,
                } => (8, maneuver_occurrence_index),
                other => return Err(failure("LF-SYNTH 未声明的事件类型", other)),
            };
            for value in [
                rank as u32,
                event.vehicle_update_sequence(),
                rank as u32 / 25,
                anchor.maneuver_occurrence_index(),
                anchor.hop(),
                anchor.position().route_edge_index(),
                anchor.position().progress_mm(),
                u32::from(anchor.position().carry_um()),
                kind,
                payload,
            ] {
                self.bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        for arrival in arrivals {
            let rank = *ranks
                .get(&arrival.vehicle)
                .ok_or_else(|| failure("到达句柄", tick))?;
            let ParkingTarget::ExplicitSpace(space) = arrival.target else {
                return Err(failure("未声明的到达类型", arrival.target));
            };
            self.bytes.extend_from_slice(&(rank as u32).to_le_bytes());
            self.bytes.extend_from_slice(&space.raw().to_le_bytes());
        }
        self.transitions += events.len() as u64;
        self.arrivals += arrivals.len() as u64;
        Ok(())
    }

    pub(crate) fn digest(&self) -> String {
        blake3::hash(&self.bytes).to_hex().to_string()
    }

    pub(crate) fn write(&self, path: &Path) -> Result<(), Error> {
        std::fs::write(path, &self.bytes)?;
        Ok(())
    }
}
