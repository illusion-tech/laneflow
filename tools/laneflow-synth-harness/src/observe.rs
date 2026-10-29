use laneflow_runtime::{
    ParkingBinding, ParkingTarget, TrafficTransitionKind, VehicleState, VehicleStatus,
};
use laneflow_static_contract::SignalAspect;

use crate::install::Sample;
use crate::{Error, cycle_ms, durations, require};

const EDGE_MM: u64 = 5_130_000;

pub(crate) fn aspect(cell: u32, route: u32, tick: u64, dt: u32) -> SignalAspect {
    let cycle = cycle_ms(dt);
    let mut position = (tick * u64::from(dt) + u64::from(cell) * 997 % cycle) % cycle;
    let phase = durations(dt)
        .into_iter()
        .position(|duration| {
            if position < duration {
                true
            } else {
                position -= duration;
                false
            }
        })
        .expect("positive cycle");
    match (phase, route % 2) {
        (0, 0) | (3, 1) => SignalAspect::Green,
        (1, 0) | (4, 1) => SignalAspect::Yellow,
        _ => SignalAspect::Red,
    }
}

pub(crate) fn position(state: VehicleState) -> u64 {
    (u64::from(state.route_edge_index()) * EDGE_MM + u64::from(state.progress_mm())) * 1_000
        + u64::from(state.carry_um())
}

pub(crate) fn motion_valid(
    old_position: u64,
    new_position: u64,
    old_speed: u32,
    new_speed: u32,
    dt: u32,
) -> bool {
    new_position >= old_position
        && new_position - old_position <= 13_900 * u64::from(dt) + 1
        && new_speed <= 13_900
        && new_speed.saturating_sub(old_speed) <= (1_400 * dt).div_ceil(1_000) + 1
}

impl Sample {
    pub fn observe(&mut self, tick: u64, arrivals_empty: bool) -> Result<blake3::Hash, Error> {
        let dt = self.state.identity.fixed_delta_ms;
        require(
            self.world.tick_index() == tick
                && self.world.time_ms() == tick * u64::from(dt)
                && self.world.command_cursor() == self.cursor
                && self.world.live_vehicles() == self.handles
                && arrivals_empty,
            || format!("人口/命令游标/时间/停车事件不符 tick={tick}"),
        )?;
        let signals = self.world.committed_signal_groups();
        let signals = signals.as_slice();
        require(signals.len() == self.groups.len(), || {
            format!("信号数量 tick={tick}")
        })?;
        for (rank, &group) in self.groups.iter().enumerate() {
            let index = signals
                .binary_search_by_key(&group.raw(), |(g, _)| g.raw())
                .map_err(|_| Error::Invalid(format!("缺少信号组 {rank}")))?;
            require(
                signals[index].1 == aspect(rank as u32 / 4, rank as u32 % 4, tick, dt),
                || format!("信号灯态 tick={tick} route={rank}"),
            )?;
        }
        let bytes = &mut self.trace_bytes;
        bytes.clear();
        bytes.extend_from_slice(b"lf-synth-public-boundary-1\0");
        bytes.extend_from_slice(&tick.to_le_bytes());
        bytes.extend_from_slice(&self.world.time_ms().to_le_bytes());
        bytes.extend_from_slice(&self.cursor.to_le_bytes());
        bytes.extend_from_slice(&(self.handles.len() as u32).to_le_bytes());
        let mut event_index = 0;
        let events = self.world.latest_transition_events();
        let mut follower_front = None;
        for (rank, &handle) in self.handles.iter().enumerate() {
            let state = self
                .world
                .vehicle(handle)
                .ok_or_else(|| Error::Invalid(format!("句柄失效 rank={rank} tick={tick}")))?;
            let initial = self.initial[rank];
            let old = self.previous[rank];
            let route_rank = self.state.vehicles[rank].route_rank as usize;
            if rank % 25 == 0 {
                follower_front = None;
            }
            require(
                state.handle() == initial.handle()
                    && state.profile() == initial.profile()
                    && state.class() == initial.class()
                    && state.route() == initial.route()
                    && state.length_mm() == 4_500
                    && state.status() == initial.status()
                    && state.route_edge_index() <= 1
                    && u64::from(state.progress_mm()) <= EDGE_MM
                    && state.carry_um() < 1_000
                    && state.waiting_membership().is_none()
                    && state.maneuver_traversal().is_none()
                    && self.world.parking_binding(handle) == self.bindings[rank],
                || format!("身份/路线/状态/停车绑定 tick={tick} rank={rank}"),
            )?;
            if state.status() == VehicleStatus::Parked {
                require(state == initial, || {
                    format!("停车状态漂移 tick={tick} rank={rank}")
                })?;
                let Some(ParkingBinding::Occupied(ParkingTarget::ExplicitSpace(space))) =
                    self.bindings[rank]
                else {
                    return Err(Error::Invalid(format!("停车绑定类型 rank={rank}")));
                };
                require(
                    self.world.committed_parking_occupant(space) == Some(handle),
                    || format!("停车占用者 tick={tick} rank={rank}"),
                )?;
            } else {
                let next_position = position(state);
                let drop = old.speed_mm_s().saturating_sub(state.speed_mm_s());
                self.max_deceleration_mm_s2 = self
                    .max_deceleration_mm_s2
                    .max((u64::from(drop) * 1_000).div_ceil(u64::from(dt)));
                self.deceleration_above_profile_count += u64::from(drop > 6 * dt + 1);
                require(
                    motion_valid(
                        position(old),
                        next_position,
                        old.speed_mm_s(),
                        state.speed_mm_s(),
                        dt,
                    ),
                    || format!("进度/速度/加减速 tick={tick} rank={rank}: {old:?} -> {state:?}"),
                )?;
                if let Some(front) = follower_front {
                    require(
                        next_position >= front + u64::from(state.length_mm()) * 1_000,
                        || format!("车身重叠或超车 tick={tick} rank={rank}"),
                    )?;
                }
                follower_front = Some(next_position);
            }
            let changed = state.route_edge_index() != old.route_edge_index();
            if changed {
                require(
                    old.route_edge_index() == 0
                        && state.route_edge_index() == 1
                        && !self.crossed[rank]
                        && aspect(route_rank as u32 / 4, route_rank as u32 % 4, tick - 1, dt)
                            == SignalAspect::Green,
                    || format!("路线转换或闯灯 tick={tick} rank={rank}"),
                )?;
                self.crossed[rank] = true;
                self.transitions += 1;
                for kind_index in 0..2 {
                    let event = events.get(event_index).ok_or_else(|| {
                        Error::Invalid(format!("缺少转换事件 tick={tick} rank={rank}"))
                    })?;
                    event_index += 1;
                    let anchor = event.anchor();
                    let expected_kind = if kind_index == 0 {
                        TrafficTransitionKind::GateCrossed {
                            gate: self.gates[route_rank],
                        }
                    } else {
                        TrafficTransitionKind::ManeuverTraversalCompleted {
                            maneuver_occurrence_index: 0,
                        }
                    };
                    require(
                        event.tick() == tick
                            && event.vehicle() == handle
                            && event.vehicle_update_sequence() == rank as u32
                            && event.kind() == expected_kind
                            && anchor.route() == self.routes[route_rank]
                            && anchor.maneuver_occurrence_index() == 0
                            && anchor.hop() == 0
                            && anchor.position().route_edge_index() == 1
                            && anchor.position().progress_mm() == 0
                            && anchor.position().carry_um() == 0,
                        || format!("事件顺序/载荷/因果 tick={tick} rank={rank}: {event:?}"),
                    )?;
                }
            }
            for field in [
                rank as u32,
                route_rank as u32,
                state.route_edge_index(),
                state.progress_mm(),
                u32::from(state.carry_um()),
                state.speed_mm_s(),
                if state.status() == VehicleStatus::Active {
                    0
                } else {
                    1
                },
                u32::from(changed),
            ] {
                bytes.extend_from_slice(&field.to_le_bytes());
            }
            self.previous[rank] = state;
        }
        require(event_index == events.len(), || {
            format!(
                "多余事件 tick={tick}: expected={event_index} actual={}",
                events.len()
            )
        })?;
        // 每个 changed 行对应已逐字段验证的固定 GateCrossed/Completed 事件对，顺序为 rank 升序。
        bytes.extend_from_slice(&(events.len() as u32).to_le_bytes());
        Ok(blake3::hash(bytes))
    }
}
