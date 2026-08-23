use laneflow_static_contract::{LaneEdgeOrdinal, SignalAspect, StaticRouteOrdinal};
use laneflow_static_network::VehicleProfileView;

use crate::tables::{
    VehicleState, VehicleStatus, remaining_along_route, remaining_to_route_end,
    static_route_ordinal,
};
use crate::{StepError, StepOutcome, TickInput, TrafficWorld};

impl TrafficWorld {
    pub(crate) fn step_vehicles(&mut self, input: TickInput) -> Result<StepOutcome, StepError> {
        let expected = self.config.fixed_delta_time_ms();
        if input.delta_time_ms != expected {
            return Err(StepError::DeltaMismatch {
                expected_delta_time_ms: expected,
                actual_delta_time_ms: input.delta_time_ms,
            });
        }
        let tick_index = self.tick_index.checked_add(1).ok_or(StepError::Overflow)?;
        let time_ms = self
            .time_ms
            .checked_add(expected)
            .ok_or(StepError::Overflow)?;
        let delta_s = expected as f64 / 1_000.0;
        self.next_states.clear();
        self.next_states.reserve(self.live_order.len());
        for handle in self.live_order.iter().copied() {
            let Some(state) = self.vehicle_state(handle).copied() else {
                continue;
            };
            if state.status != VehicleStatus::Active {
                continue;
            }
            let next = self
                .advance_active_vehicle(state, delta_s)
                .ok_or(StepError::NonFiniteMotion)?;
            let slot = usize::try_from(handle.index()).expect("vehicle index fits usize");
            self.next_states.push((slot, next));
        }
        let mut updates = std::mem::take(&mut self.next_states);
        for (slot, next) in updates.drain(..) {
            if next.status == VehicleStatus::Completed {
                if let Some(previous) = self.vehicles[slot].state
                    && previous.status == VehicleStatus::Active
                {
                    self.release_route_ref(previous.route);
                    self.retire_completed_vehicle(slot, previous.handle);
                }
                continue;
            }
            self.vehicles[slot].state = Some(next);
        }
        self.next_states = updates;
        self.tick_index = tick_index;
        self.time_ms = time_ms;
        self.refresh_signals();
        Ok(StepOutcome::new(tick_index, time_ms))
    }

    pub(crate) fn advance_active_vehicle(
        &self,
        mut state: VehicleState,
        delta_s: f64,
    ) -> Option<VehicleState> {
        let edges = self.route_edges(state.route)?;
        let cursor = usize::try_from(state.route_edge_index).ok()?;
        let edge = *edges.get(cursor)?;
        let lengths = self.revision.traffic().lane_lengths_meters();
        let speed_limits = self
            .revision
            .traffic()
            .lane_speed_limits_meters_per_second();
        let speed_limit = *speed_limits.get(edge.index())?;
        let profile = self
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)?;
        let desired = profile.desired_speed().min(speed_limit).max(0.0);
        let leader_gap = self.leader_bumper_gap(&state, edges, lengths);
        let route_end = remaining_to_route_end(lengths, edges, cursor, state.progress)?;
        let signal_stop = self.signal_stop_distance(&state, edges, lengths, cursor);

        let mut travel = iidm_travel(state.speed, desired, leader_gap, profile, delta_s)?;
        if let Some(gap) = leader_gap {
            travel = travel.min((gap - profile.min_gap()).max(0.0));
        }
        if let Some(stop) = signal_stop {
            travel = travel.min(stop.max(0.0));
        }
        travel = travel.min(route_end.max(0.0));
        if !travel.is_finite() || travel < 0.0 {
            return None;
        }

        let mut speed = if travel <= 0.0 {
            0.0
        } else {
            let next = (2.0 * travel / delta_s - state.speed).max(0.0);
            next.min(speed_limit)
        };
        apply_travel(&mut state, edges, lengths, travel)?;
        let remaining = remaining_to_route_end(
            lengths,
            edges,
            usize::try_from(state.route_edge_index).ok()?,
            state.progress,
        )?;
        if remaining <= 1e-9 {
            speed = 0.0;
            state.status = VehicleStatus::Completed;
        }
        if signal_stop.is_some_and(|stop| travel + 1e-9 >= stop) {
            speed = 0.0;
        }
        state.speed = speed;
        Some(state)
    }

    pub(crate) fn leader_bumper_gap(
        &self,
        follower: &VehicleState,
        edges: &[LaneEdgeOrdinal],
        lengths: &[f64],
    ) -> Option<f64> {
        let cursor = usize::try_from(follower.route_edge_index).ok()?;
        let mut best: Option<f64> = None;
        for handle in self.live_order.iter().copied() {
            if handle == follower.handle {
                continue;
            }
            let Some(leader) = self.vehicle_state(handle) else {
                continue;
            };
            if leader.status != VehicleStatus::Active {
                continue;
            }
            let Some(leader_edges) = self.route_edges(leader.route) else {
                continue;
            };
            let leader_index = usize::try_from(leader.route_edge_index).ok()?;
            let Some(leader_edge) = leader_edges.get(leader_index).copied() else {
                continue;
            };
            let Some(found) = edges
                .iter()
                .enumerate()
                .skip(cursor)
                .find_map(|(index, edge)| (*edge == leader_edge).then_some(index))
            else {
                continue;
            };
            if found == cursor && leader.progress <= follower.progress {
                continue;
            }
            let Some(front_to_front) = remaining_along_route(
                lengths,
                edges,
                cursor,
                follower.progress,
                found,
                leader.progress,
            ) else {
                continue;
            };
            let bumper = front_to_front - leader.length;
            best = Some(best.map_or(bumper, |current| current.min(bumper)));
        }
        best
    }

    pub(crate) fn signal_stop_distance(
        &self,
        state: &VehicleState,
        edges: &[LaneEdgeOrdinal],
        lengths: &[f64],
        cursor: usize,
    ) -> Option<f64> {
        if let Some(ordinal) = static_route_ordinal(state.route) {
            return self.static_signal_stop_distance(ordinal, state, edges, lengths, cursor);
        }
        self.dynamic_signal_stop_distance(edges, lengths, cursor, state.progress)
    }

    fn static_signal_stop_distance(
        &self,
        ordinal: StaticRouteOrdinal,
        state: &VehicleState,
        edges: &[LaneEdgeOrdinal],
        lengths: &[f64],
        cursor: usize,
    ) -> Option<f64> {
        let relations = self.revision.traffic().relations();
        let mut search = cursor;
        loop {
            let nxt = relations.next_controlled_transition(ordinal, search)?;
            let from = usize::try_from(nxt.from_route_edge_index()).ok()?;
            if self.gate_is_restrictive(nxt.gate()) {
                let stop_edge = *edges.get(from)?;
                let stop_at = *lengths.get(stop_edge.index())?;
                return remaining_along_route(
                    lengths,
                    edges,
                    cursor,
                    state.progress,
                    from,
                    stop_at,
                );
            }
            search = from.checked_add(1)?;
            if search >= edges.len() {
                return None;
            }
        }
    }

    fn dynamic_signal_stop_distance(
        &self,
        edges: &[LaneEdgeOrdinal],
        lengths: &[f64],
        cursor: usize,
        progress: f64,
    ) -> Option<f64> {
        let traffic = self.revision.traffic();
        for index in cursor..edges.len().saturating_sub(1) {
            let from = edges[index];
            let to = edges[index + 1];
            let Some(candidates) = traffic.maneuvers().transition_candidates(from) else {
                continue;
            };
            let Some(candidate) = candidates
                .iter()
                .find(|candidate| candidate.successor() == to)
            else {
                continue;
            };
            let Some(gate) = candidate.maneuver_gate() else {
                continue;
            };
            if !self.gate_is_restrictive(gate) {
                continue;
            }
            let edge_length = *lengths.get(from.index())?;
            return remaining_along_route(lengths, edges, cursor, progress, index, edge_length);
        }
        None
    }

    fn gate_is_restrictive(&self, gate: laneflow_static_contract::ManeuverGateOrdinal) -> bool {
        self.revision
            .traffic()
            .relations()
            .maneuver_gate(gate)
            .and_then(|view| view.signal_group())
            .is_some_and(|group| self.group_is_restrictive(group))
    }

    fn group_is_restrictive(&self, group: laneflow_static_contract::SignalGroupOrdinal) -> bool {
        match self.signal_aspects.get(group.index()).copied() {
            Some(SignalAspect::Red | SignalAspect::Yellow) => true,
            Some(SignalAspect::Green) | None => false,
            _ => false,
        }
    }
}

fn iidm_travel(
    speed: f64,
    desired: f64,
    leader_gap: Option<f64>,
    profile: VehicleProfileView,
    delta_s: f64,
) -> Option<f64> {
    if !speed.is_finite() || !desired.is_finite() || delta_s <= 0.0 {
        return None;
    }
    let accel_max = profile.max_accel();
    let comfort = profile.comfort_decel();
    let emergency = profile.emergency_decel();
    if accel_max <= 0.0 || comfort <= 0.0 || emergency <= 0.0 {
        return None;
    }
    if leader_gap.is_some_and(|gap| gap <= 0.0) {
        return Some(0.0);
    }
    let speed_term = if desired <= 0.0 {
        1.0
    } else {
        (speed / desired).max(0.0).powi(4)
    };
    let gap_term = if let Some(gap) = leader_gap {
        let s_star = profile.min_gap() + speed * profile.time_headway();
        (s_star / gap).max(0.0).powi(2)
    } else {
        0.0
    };
    let accel = accel_max * (1.0 - speed_term - gap_term);
    let next_speed = (speed + accel * delta_s).max(0.0);
    let travel = ((speed + next_speed) * 0.5 * delta_s).max(0.0);
    travel.is_finite().then_some(travel)
}

fn apply_travel(
    state: &mut VehicleState,
    edges: &[LaneEdgeOrdinal],
    lengths: &[f64],
    mut remaining: f64,
) -> Option<()> {
    let mut index = usize::try_from(state.route_edge_index).ok()?;
    while remaining > 0.0 {
        let edge = *edges.get(index)?;
        let edge_length = *lengths.get(edge.index())?;
        let leftover = edge_length - state.progress;
        if leftover < 0.0 {
            return None;
        }
        if remaining <= leftover {
            state.progress += remaining;
            break;
        }
        remaining -= leftover;
        if index + 1 >= edges.len() {
            state.progress = edge_length;
            break;
        }
        index += 1;
        state.progress = 0.0;
    }
    state.route_edge_index = u32::try_from(index).ok()?;
    state.progress.is_finite().then_some(())
}

#[cfg(test)]
mod preview {
    use super::*;

    use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
    use laneflow_static_contract::{StaticRouteOrdinal, VehicleProfileOrdinal};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    use crate::{VehicleSpawnInput, WorldConfig};

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-full-spatial/expected.lfca"
    );

    #[test]
    fn preview_follower_constraints() {
        let input = check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD).unwrap();
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .unwrap();
        let mut world = TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 100)).unwrap();
        let route = world.static_route(StaticRouteOrdinal::from_raw(0)).unwrap();
        let profile = world
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .unwrap();
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1.0 + profile.length() + profile.min_gap() + 2.0,
                0.0,
            ))
            .unwrap();
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1.0,
                0.0,
            ))
            .unwrap();
        let state = world.vehicle_state(follower).copied().unwrap();
        let next = world.advance_active_vehicle(state, 0.1).unwrap();
        assert!(
            next.progress > state.progress,
            "follower should start moving, {} -> {}",
            state.progress,
            next.progress
        );
    }
}
