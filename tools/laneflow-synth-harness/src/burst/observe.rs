use laneflow_runtime::*;
use laneflow_static_contract::SignalAspect;
use laneflow_synth_generator::Case;

use super::{Burst, Command, Operation};
use crate::{
    Error, Plan, cycle_ms, durations, failure,
    observe::{motion_valid, position},
    require,
};

fn aspect(case: Case, cell: usize, route: usize, tick: u64, dt: u32) -> SignalAspect {
    let cycle = cycle_ms(dt);
    let offset = if case == Case::W4 {
        cycle - u64::from(dt)
    } else {
        cell as u64 * 997 % cycle
    };
    let mut position = (tick * u64::from(dt) + offset) % cycle;
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

fn status_code(status: VehicleStatus) -> u32 {
    match status {
        VehicleStatus::Active => 0,
        VehicleStatus::Parked => 1,
        VehicleStatus::Completed => 2,
    }
}

impl Burst {
    pub(super) fn observe(
        &mut self,
        tick: u64,
        warm: u64,
        outcome: Option<&StepOutcome>,
    ) -> Result<blake3::Hash, Error> {
        let dt = self.sample.state.identity.fixed_delta_ms;
        let case = self.sample.state.identity.case;
        let cells = self.arrivals.len();
        require(
            self.sample.world.tick_index() == tick
                && self.sample.world.time_ms() == tick * u64::from(dt)
                && self.sample.world.command_cursor() == self.sample.cursor
                && self.sample.world.live_vehicles().iter().copied().eq(self
                    .live_ranks
                    .iter()
                    .map(|&rank| self.sample.handles[rank])),
            || format!("population/order/cursor/time tick={tick}"),
        )?;
        require(
            match outcome {
                Some(outcome) => {
                    tick != 0
                        && outcome.tick_index() == tick
                        && outcome.time_ms() == tick * u64::from(dt)
                }
                None => tick == 0,
            },
            || format!("step outcome tick={tick}"),
        )?;
        let signals = self.sample.world.committed_signal_groups();
        require(signals.as_slice().len() == self.sample.groups.len(), || {
            format!("signal count tick={tick}")
        })?;
        for (rank, &group) in self.sample.groups.iter().enumerate() {
            let index = signals
                .as_slice()
                .binary_search_by_key(&group.raw(), |(g, _)| g.raw())
                .map_err(|_| failure("signal group", rank))?;
            require(
                signals.as_slice()[index].1 == aspect(case, rank / 4, rank % 4, tick, dt),
                || format!("signal aspect tick={tick} route={rank}"),
            )?;
        }
        self.counts = [0; 3];
        self.changed.fill(false);
        let bytes = &mut self.sample.trace_bytes;
        bytes.clear();
        bytes.extend_from_slice(b"lf-synth-burst-boundary-1\0");
        for value in [tick, self.sample.world.time_ms(), self.sample.cursor] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&(self.sample.handles.len() as u32).to_le_bytes());
        for (rank, &handle) in self.sample.handles.iter().enumerate() {
            let state = self
                .sample
                .world
                .vehicle(handle)
                .ok_or_else(|| failure("live handle", rank))?;
            let initial = self.sample.initial[rank];
            let previous = self.sample.previous[rank];
            let route_rank = rank / 25;
            if self.terminal[rank] {
                let expected_tick = if case == Case::W3 { warm + 1 } else { warm };
                let expected_rank = if case == Case::W3 { 81 } else { 87 };
                require(
                    tick == expected_tick
                        && rank % 100 == expected_rank
                        && previous.status() == VehicleStatus::Active
                        && state.status() == VehicleStatus::Completed
                        && state.route_edge_index() == 1
                        && state.progress_mm() == 5_130_000
                        && state.speed_mm_s() == 0,
                    || format!("host completion tick={tick} rank={rank}"),
                )?;
                self.terminal[rank] = false;
                self.expected_status[rank] = VehicleStatus::Completed;
                self.completions.push((tick, rank as u32));
                if let Some(rule) = &self.replacements {
                    require(rule.ranks.binary_search(&(rank as u32)).is_ok(), || {
                        format!("undeclared completion rank={rank}")
                    })?;
                    self.pending.push(Command {
                        operation: Operation::AtomicReplace,
                        logical_rank: rank as u32,
                        route_rank: route_rank as u32,
                        edge_occurrence: rule.edge_occurrence,
                        progress_mm: rule.progress_mm,
                        speed_mm_per_s: rule.speed_mm_per_s,
                    });
                }
            }
            require(
                state.handle() == handle
                    && state.profile() == initial.profile()
                    && state.class() == initial.class()
                    && state.route() == initial.route()
                    && state.length_mm() == 4_500
                    && state.status() == self.expected_status[rank]
                    && state.route_edge_index() <= 1
                    && state.progress_mm() <= 5_130_000
                    && state.carry_um() < 1_000
                    && state.waiting_membership().is_none()
                    && state.maneuver_traversal().is_none()
                    && self.sample.world.parking_binding(handle) == self.sample.bindings[rank],
                || format!("identity/authority tick={tick} rank={rank}"),
            )?;
            let expected_space = match self.sample.bindings[rank] {
                None => ParkingSpaceState::Vacant,
                Some(ParkingBinding::Reserved(reservation)) => {
                    require(
                        case == Case::W3
                            && rank % 100 == 0
                            && reservation.target()
                                == ParkingTarget::ExplicitSpace(self.spaces[rank])
                            && reservation.route() == self.sample.routes[route_rank]
                            && reservation.entry_route_occurrence() == 1,
                        || format!("reservation tick={tick} rank={rank}"),
                    )?;
                    ParkingSpaceState::Reserved(handle)
                }
                Some(ParkingBinding::Occupied(target)) => {
                    require(
                        target == ParkingTarget::ExplicitSpace(self.spaces[rank])
                            && state.status() == VehicleStatus::Parked,
                        || format!("occupied binding tick={tick} rank={rank}"),
                    )?;
                    ParkingSpaceState::Occupied(handle)
                }
            };
            require(
                self.sample.world.parking_space_state(self.spaces[rank]) == Some(expected_space),
                || format!("space authority tick={tick} rank={rank}"),
            )?;
            if state.status() == VehicleStatus::Parked {
                require(state == previous, || {
                    format!("parked drift tick={tick} rank={rank}")
                })?;
            } else {
                require(
                    motion_valid(
                        position(previous),
                        position(state),
                        previous.speed_mm_s(),
                        state.speed_mm_s(),
                        dt,
                    ),
                    || format!("motion tick={tick} rank={rank}: {previous:?} -> {state:?}"),
                )?;
                let drop = previous.speed_mm_s().saturating_sub(state.speed_mm_s());
                self.sample.max_deceleration_mm_s2 = self
                    .sample
                    .max_deceleration_mm_s2
                    .max((u64::from(drop) * 1_000).div_ceil(u64::from(dt)));
                self.sample.deceleration_above_profile_count += u64::from(drop > 6 * dt + 1);
            }
            let changed = state.route_edge_index() != previous.route_edge_index();
            if changed {
                require(
                    previous.route_edge_index() == 0
                        && state.route_edge_index() == 1
                        && tick > 0
                        && aspect(case, rank / 100, route_rank % 4, tick - 1, dt)
                            == SignalAspect::Green,
                    || format!("transition/signal causality tick={tick} rank={rank}"),
                )?;
                self.total_crossings[rank] += 1;
                self.sample.transitions += 1;
            }
            self.changed[rank] = changed;
            self.counts[status_code(state.status()) as usize] += 1;
            for word in [
                rank as u32,
                self.incarnations[rank],
                self.live_sequences[rank],
                route_rank as u32,
                state.route_edge_index(),
                state.progress_mm(),
                u32::from(state.carry_um()),
                state.speed_mm_s(),
                status_code(state.status()),
                u32::from(changed),
            ] {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
            self.sample.previous[rank] = state;
        }
        let mut front = None;
        for &rank in &self.lane_order {
            let state = self.sample.previous[rank];
            if state.status() != VehicleStatus::Active {
                continue;
            }
            let current = position(state);
            if let Some((route, previous)) = front {
                require(
                    route != rank / 25 || current >= previous + 4_500_000,
                    || format!("overlap/overtake tick={tick} rank={rank}"),
                )?;
            }
            front = Some((rank / 25, current));
        }
        let events = self.sample.world.latest_transition_events();
        let mut event_index = 0;
        for &rank in &self.live_ranks {
            if !self.changed[rank] {
                continue;
            }
            for kind in [
                TrafficTransitionKind::GateCrossed {
                    gate: self.sample.gates[rank / 25],
                },
                TrafficTransitionKind::ManeuverTraversalCompleted {
                    maneuver_occurrence_index: 0,
                },
            ] {
                let event = events
                    .get(event_index)
                    .ok_or_else(|| failure("missing transition", (tick, rank)))?;
                event_index += 1;
                let anchor = event.anchor();
                require(
                    event.tick() == tick
                        && event.vehicle() == self.sample.handles[rank]
                        && event.vehicle_update_sequence() == self.live_sequences[rank]
                        && event.kind() == kind
                        && anchor.route() == self.sample.routes[rank / 25]
                        && anchor.maneuver_occurrence_index() == 0
                        && anchor.hop() == 0
                        && anchor.position().route_edge_index() == 1
                        && anchor.position().progress_mm() == 0
                        && anchor.position().carry_um() == 0,
                    || format!("ordered transition tick={tick} rank={rank}"),
                )?;
            }
        }
        require(event_index == events.len(), || {
            format!("extra transition tick={tick}")
        })?;
        bytes.extend_from_slice(&(events.len() as u32).to_le_bytes());
        let arrivals = outcome.map_or(&[][..], StepOutcome::parking_arrivals);
        bytes.extend_from_slice(&(arrivals.len() as u32).to_le_bytes());
        let mut previous_sequence = None;
        for arrival in arrivals {
            let rank = *self
                .handle_ranks
                .get(&arrival.vehicle)
                .ok_or_else(|| failure("arrival handle", tick))?;
            require(
                case == Case::W3
                    && tick > warm
                    && rank % 100 == 0
                    && self.parked_boundary.is_none()
                    && arrival.target == ParkingTarget::ExplicitSpace(self.spaces[rank])
                    && previous_sequence.is_none_or(|s| s < self.live_sequences[rank]),
                || format!("arrival identity/order/window tick={tick} rank={rank}"),
            )?;
            let state = self.sample.previous[rank];
            require(
                state.status() == VehicleStatus::Active
                    && state.route_edge_index() == 1
                    && state.progress_mm() == 5_000
                    && state.speed_mm_s() == 0
                    && state.carry_um() == 0,
                || format!("arrival position tick={tick} rank={rank}"),
            )?;
            record_arrival(&mut self.arrivals, rank / 100, tick)?;
            self.arrival_records.push((tick, rank as u32));
            previous_sequence = Some(self.live_sequences[rank]);
            bytes.extend_from_slice(&(rank as u32).to_le_bytes());
        }
        let count = cells as u32;
        let expected = if case == Case::W3 {
            let parked = 75 * count
                + if self.parked_boundary.is_some_and(|b| tick == b + 1) {
                    count
                } else {
                    0
                };
            let completed = self.pending.len() as u32;
            [100 * count - parked - completed, parked, completed]
        } else if tick == warm {
            [74 * count, 25 * count, count]
        } else if tick > warm {
            [76 * count, 24 * count, 0]
        } else {
            [75 * count, 25 * count, 0]
        };
        require(self.counts == expected, || {
            format!("population tick={tick}: {:?} != {expected:?}", self.counts)
        })?;
        if case == Case::W4 {
            require(
                (tick == warm + 2
                    && self
                        .changed
                        .iter()
                        .enumerate()
                        .all(|(rank, &changed)| changed == (rank % 100 == 61)))
                    || (tick != warm + 2 && self.changed.iter().all(|&changed| !changed)),
                || format!("W4 B0/B1 transition tick={tick}"),
            )?;
        }
        Ok(blake3::hash(bytes))
    }

    pub(super) fn finish(&self, plan: &Plan) -> Result<(), Error> {
        let case = plan.identity.case;
        let cells = self.arrivals.len();
        require(
            self.pending.is_empty() && self.completions.len() == cells && self.counts[2] == 0,
            || "缺少完成/替换或仍有持续 Blocked".into(),
        )?;
        let local = if case == Case::W3 { 81 } else { 87 };
        let completion_tick = plan.warm_up_ticks + u64::from(case == Case::W3);
        require(
            self.completions
                .iter()
                .enumerate()
                .all(|(cell, &(tick, rank))| {
                    tick == completion_tick && rank as usize == cell * 100 + local
                }),
            || "宿主完成观测数量/顺序不符".into(),
        )?;
        for (rank, &incarnation) in self.incarnations.iter().enumerate() {
            let expected = if rank % 100 == local {
                2
            } else if case == Case::W4 && rank % 100 == 61 {
                1
            } else {
                0
            };
            require(incarnation == expected, || {
                format!("逻辑身份代次 rank={rank}")
            })?;
            let expected_crossings = if case == Case::W3 {
                u32::from(self.sample.initial[rank].status() == VehicleStatus::Active)
            } else {
                u32::from(rank % 100 == 61)
            };
            require(self.total_crossings[rank] == expected_crossings, || {
                format!("最终过门次数 rank={rank}")
            })?;
        }
        if case == Case::W3 {
            require(
                self.arrivals.iter().all(Option::is_some)
                    && self.left
                    && self.parked_boundary.is_some()
                    && self.arrival_records.len() == cells,
                || "W3 停车闭环未完成".into(),
            )?;
        } else {
            require(
                self.sample.world.tick_index() == plan.warm_up_ticks + 2
                    && self.arrival_records.is_empty(),
                || "W4 必须在 B1 后结束".into(),
            )?;
        }
        Ok(())
    }
}

pub(super) fn record_arrival(
    arrivals: &mut [Option<u64>],
    cell: usize,
    tick: u64,
) -> Result<(), Error> {
    let entry = arrivals
        .get_mut(cell)
        .ok_or_else(|| failure("arrival cell", cell))?;
    require(entry.is_none(), || format!("重复 arrival cell={cell}"))?;
    *entry = Some(tick);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synchronized_signal_uses_pre_step_snapshot() {
        for dt in [16, 33] {
            let warm = 4 * cycle_ms(dt) / u64::from(dt);
            for cell in [0, 99, 999] {
                for route in 0..4 {
                    assert_eq!(aspect(Case::W4, cell, route, warm, dt), SignalAspect::Red);
                    assert_eq!(
                        aspect(Case::W4, cell, route, warm + 1, dt),
                        if route % 2 == 0 {
                            SignalAspect::Green
                        } else {
                            SignalAspect::Red
                        }
                    );
                }
            }
        }
    }

    #[test]
    fn arrival_tracker_rejects_duplicates_and_unknown_cells() {
        let mut arrivals = [None; 2];
        record_arrival(&mut arrivals, 1, 100).unwrap();
        assert!(!arrivals.iter().all(Option::is_some));
        assert!(record_arrival(&mut arrivals, 1, 101).is_err());
        assert!(record_arrival(&mut arrivals, 2, 101).is_err());
        record_arrival(&mut arrivals, 0, 101).unwrap();
        assert!(arrivals.iter().all(Option::is_some));
    }
}
