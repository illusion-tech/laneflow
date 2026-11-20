use laneflow_runtime::*;
use laneflow_synth_generator::Case;

use super::{Burst, Command, Operation};
use crate::signal_reference::SignalSchedule;
use crate::signal_reference::{CrossingPermit, aspect, signal_crossing_permission};
use crate::verification_cost::{Noop, Probe, Scan};
use crate::{
    Error, Plan, failure,
    observe::{motion_valid, position},
    require,
};

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
        self.observe_with_probe(tick, warm, outcome, &mut Noop)
    }

    pub(super) fn observe_with_probe<P: Probe>(
        &mut self,
        tick: u64,
        warm: u64,
        outcome: Option<&StepOutcome>,
        probe: &mut P,
    ) -> Result<blake3::Hash, Error> {
        let dt = self.sample.state.identity.fixed_delta_ms;
        let case = self.sample.state.identity.case.clone();
        let schedule = if case == Case::W4 {
            SignalSchedule::Synchronized
        } else {
            SignalSchedule::CellOffset
        };
        let cells = self.arrivals.len();
        probe.scan(Scan::LiveOrder);
        require(
            self.sample.world.tick_index() == tick
                && self.sample.world.time_ms() == tick * u64::from(dt)
                && self.sample.world.command_cursor() == self.sample.cursor
                && self
                    .sample
                    .world
                    .live_vehicles()
                    .iter()
                    .copied()
                    .inspect(|_| probe.visit(Scan::LiveOrder, 1))
                    .eq(self
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
        probe.scan(Scan::SignalCount);
        let signalized = self
            .sample
            .groups
            .iter()
            .inspect(|_| probe.visit(Scan::SignalCount, 1))
            .filter(|group| group.is_some())
            .count();
        require(signals.as_slice().len() == signalized, || {
            format!("signal count tick={tick}")
        })?;
        probe.scan(Scan::Signals);
        for (rank, group) in self.sample.groups.iter().enumerate() {
            probe.visit(Scan::Signals, 1);
            let Some(group) = group else {
                continue;
            };
            let index = signals
                .as_slice()
                .binary_search_by_key(&group.raw(), |(g, _)| g.raw())
                .map_err(|_| failure("signal group", rank))?;
            require(
                signals.as_slice()[index].1
                    == aspect(schedule, rank as u32 / 4, rank as u32 % 4, tick, dt),
                || format!("signal aspect tick={tick} route={rank}"),
            )?;
        }
        probe.scan(Scan::ChangedReset);
        probe.visit(Scan::ChangedReset, self.changed.len());
        self.counts = [0; 3];
        self.changed.fill(false);
        let bytes = &mut self.sample.trace_bytes;
        bytes.clear();
        bytes.extend_from_slice(b"lf-synth-burst-boundary-1\0");
        for value in [tick, self.sample.world.time_ms(), self.sample.cursor] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&(self.sample.handles.len() as u32).to_le_bytes());
        probe.scan(Scan::Vehicles);
        for (rank, &handle) in self.sample.handles.iter().enumerate() {
            probe.visit(Scan::Vehicles, 1);
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
                        self.sample.max_accel_mm_s2,
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
                // 结构与信号许可分开：命令后的 `previous` 是本次运动的真实
                // 起点（Burst::command 成功后写入；Blocked 不推进）。
                require(
                    previous.route_edge_index() == 0 && state.route_edge_index() == 1 && tick > 0,
                    || format!("transition structure tick={tick} rank={rank}"),
                )?;
                let aspect_before = aspect(
                    schedule,
                    rank as u32 / 100,
                    route_rank as u32 % 4,
                    tick - 1,
                    dt,
                );
                let distance_before_mm = u32::try_from(
                    5_130_000_u64
                        .checked_sub(u64::from(previous.progress_mm()))
                        .ok_or_else(|| {
                            failure("pre-crossing progress past stop line", (tick, rank))
                        })?,
                )
                .expect("entry 长度在 u32 域内");
                let permit = signal_crossing_permission(
                    aspect_before,
                    previous.speed_mm_s(),
                    distance_before_mm,
                    self.sample.emergency_decel_mm_s2,
                )
                .map_err(|denial| {
                    Error::Invalid(format!(
                        "signal permit denied tick={tick} tick_before={} rank={rank} \
                         route={route_rank} aspect_before={aspect_before:?} aspect_after={:?} \
                         speed_before_mm_s={} progress_before_mm={} carry_before_um={} \
                         distance_mm={distance_before_mm} emergency_decel_mm_s2={} \
                         denial={denial:?}",
                        tick - 1,
                        aspect(schedule, rank as u32 / 100, route_rank as u32 % 4, tick, dt),
                        previous.speed_mm_s(),
                        previous.progress_mm(),
                        previous.carry_um(),
                        self.sample.emergency_decel_mm_s2,
                    ))
                })?;
                if case == Case::W4 {
                    // W4 同步释放专项：B1 指定过门的许可原因必须为 Green，
                    // 强于通用信号许可（构造状态即使符合 YellowDilemma 也不满足
                    // green-release 验收）。
                    require(permit == CrossingPermit::Green, || {
                        format!("W4 crossing permit tick={tick} rank={rank} permit={permit:?}")
                    })?;
                }
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
        probe.scan(Scan::BodyOrder);
        for &rank in &self.lane_order {
            probe.visit(Scan::BodyOrder, 1);
            let state = self.sample.previous[rank];
            if state.status() != VehicleStatus::Active {
                continue;
            }
            // 本变体的车身分离是内联邻接判定（迁移前正本）；片段计数与
            // 共享判据侧挂在同一逻辑行位置，比较口径一致。
            probe.body_fragment();
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
        probe.scan(Scan::EventCandidates);
        probe.scan(Scan::TransitionEvents);
        for &rank in &self.live_ranks {
            probe.visit(Scan::EventCandidates, 1);
            if !self.changed[rank] {
                continue;
            }
            for kind in [
                TrafficTransitionKind::GateCrossed {
                    gate: self.sample.gates[rank / 25].expect("W 族唯一信号 Gate"),
                },
                TrafficTransitionKind::ManeuverTraversalCompleted {
                    maneuver_occurrence_index: 0,
                },
            ] {
                let event = events
                    .get(event_index)
                    .ok_or_else(|| failure("missing transition", (tick, rank)))?;
                probe.visit(Scan::TransitionEvents, 1);
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
        probe.scan(Scan::ParkingArrivals);
        for arrival in arrivals {
            probe.visit(Scan::ParkingArrivals, 1);
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
            probe.scan(Scan::W4Changed);
            require(
                (tick == warm + 2
                    && self
                        .changed
                        .iter()
                        .enumerate()
                        .inspect(|_| probe.visit(Scan::W4Changed, 1))
                        .all(|(rank, &changed)| changed == (rank % 100 == 61)))
                    || (tick != warm + 2
                        && self
                            .changed
                            .iter()
                            .inspect(|_| probe.visit(Scan::W4Changed, 1))
                            .all(|&changed| !changed)),
                || format!("W4 B0/B1 transition tick={tick}"),
            )?;
        }
        Ok(blake3::hash(bytes))
    }

    pub(super) fn finish(&self, plan: &Plan) -> Result<(), Error> {
        let case = plan.identity.case.clone();
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
                u32::from(self.sample.state.vehicles[rank].status == "active")
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
    use crate::cycle_ms;
    use laneflow_static_contract::SignalAspect;

    #[test]
    fn synchronized_signal_uses_pre_step_snapshot() {
        for dt in [16, 33] {
            let warm = 4 * cycle_ms(dt) / u64::from(dt);
            for cell in [0, 99, 999] {
                for route in 0..4 {
                    assert_eq!(
                        aspect(
                            SignalSchedule::Synchronized,
                            cell as u32,
                            route as u32,
                            warm,
                            dt
                        ),
                        SignalAspect::Red
                    );
                    assert_eq!(
                        aspect(
                            SignalSchedule::Synchronized,
                            cell as u32,
                            route as u32,
                            warm + 1,
                            dt
                        ),
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
