use laneflow_static_contract::{LaneEdgeOrdinal, SignalAspect, StaticRouteOrdinal};
use laneflow_static_network::VehicleProfileView;

use crate::tables::{
    compiled_hop_gate, occupancy_front_gap, remaining_along_route, remaining_to_route_end,
    static_route_ordinal,
};
use crate::{StepError, StepOutcome, TickInput, TrafficWorld, VehicleState, VehicleStatus};

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
        let current_limit = *speed_limits.get(edge.index())?;
        let profile = self
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)?;
        let desired = profile.desired_speed().min(current_limit).max(0.0);
        let leader_gap = self.leader_bumper_gap(&state, edges, lengths);
        let route_end = remaining_to_route_end(lengths, edges, cursor, state.progress)?;
        let signal_stop = self.signal_stop_distance(&state, edges, lengths, cursor);
        let envelope = speed_limit_path_envelope(
            edges,
            lengths,
            speed_limits,
            cursor,
            state.progress,
            delta_s,
        )?;

        let mut travel = iidm_travel(state.speed, desired, leader_gap, profile, delta_s)?;
        if let Some(gap) = leader_gap {
            travel = travel.min((gap - profile.min_gap()).max(0.0));
        }
        if let Some(stop) = signal_stop {
            travel = travel.min(stop.max(0.0));
        }
        travel = travel.min(route_end.max(0.0)).min(envelope);
        if !travel.is_finite() || travel < 0.0 {
            return None;
        }

        let mut speed = if travel <= 0.0 {
            0.0
        } else {
            (2.0 * travel / delta_s - state.speed)
                .max(0.0)
                .min(current_limit)
        };
        speed = constrain_upcoming_speed_limits(
            state.speed,
            speed,
            delta_s,
            edges,
            lengths,
            speed_limits,
            cursor,
            state.progress,
            profile.comfort_decel(),
            profile.emergency_decel(),
        )?;
        travel = ((state.speed + speed) * 0.5 * delta_s).max(0.0);
        if let Some(gap) = leader_gap {
            travel = travel.min((gap - profile.min_gap()).max(0.0));
        }
        if let Some(stop) = signal_stop {
            travel = travel.min(stop.max(0.0));
        }
        travel = travel.min(route_end.max(0.0)).min(envelope);
        travel = clamp_travel_to_speed_down_boundary(
            travel,
            state.speed,
            speed,
            delta_s,
            edges,
            lengths,
            speed_limits,
            cursor,
            state.progress,
        )?;
        if travel <= 0.0 {
            speed = 0.0;
        } else {
            speed = (2.0 * travel / delta_s - state.speed)
                .max(0.0)
                .min(current_limit);
        }
        apply_travel(&mut state, edges, lengths, travel)?;
        let committed_index = usize::try_from(state.route_edge_index).ok()?;
        let committed_edge = *edges.get(committed_index)?;
        let committed_limit = *speed_limits.get(committed_edge.index())?;
        speed = speed.min(committed_limit);
        let remaining = remaining_to_route_end(lengths, edges, committed_index, state.progress)?;
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
            let Ok(leader_index) = usize::try_from(leader.route_edge_index) else {
                continue;
            };
            let Some(gap) = occupancy_front_gap(
                lengths,
                edges,
                cursor,
                follower.progress,
                leader_edges,
                leader_index,
                leader.progress,
                leader.length,
            ) else {
                continue;
            };
            best = Some(best.map_or(gap, |current| current.min(gap)));
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
        self.dynamic_signal_stop_distance(state, edges, lengths, cursor)
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
        state: &VehicleState,
        edges: &[LaneEdgeOrdinal],
        lengths: &[f64],
        cursor: usize,
    ) -> Option<f64> {
        let compiled = self.compiled_dynamic_route(state.route)?;
        let network = self.revision.traffic().maneuvers();
        for index in cursor..edges.len().saturating_sub(1) {
            let from = edges[index];
            let to = edges[index + 1];
            let Some(gate) = compiled_hop_gate(network, compiled, index, from, to) else {
                continue;
            };
            if !self.gate_is_restrictive(gate) {
                continue;
            }
            let edge_length = *lengths.get(from.index())?;
            return remaining_along_route(
                lengths,
                edges,
                cursor,
                state.progress,
                index,
                edge_length,
            );
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
            Some(SignalAspect::Green) => false,
            Some(SignalAspect::Red | SignalAspect::Yellow) => true,
            Some(_) | None => true,
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
    let next_speed = (speed + accel * delta_s).max(0.0).min(desired.max(0.0));
    let travel = ((speed + next_speed) * 0.5 * delta_s).max(0.0);
    travel.is_finite().then_some(travel)
}

fn speed_limit_path_envelope(
    edges: &[LaneEdgeOrdinal],
    lengths: &[f64],
    speed_limits: &[f64],
    mut index: usize,
    mut progress: f64,
    delta_s: f64,
) -> Option<f64> {
    if delta_s <= 0.0 {
        return None;
    }
    let mut remaining_t = delta_s;
    let mut total = 0.0;
    loop {
        let edge = *edges.get(index)?;
        let length = *lengths.get(edge.index())?;
        let limit = *speed_limits.get(edge.index())?;
        if !length.is_finite() || !limit.is_finite() || length < 0.0 || limit < 0.0 {
            return None;
        }
        let leftover = (length - progress).max(0.0);
        if leftover < 0.0 {
            return None;
        }
        if limit <= 0.0 {
            break;
        }
        let cap = limit * remaining_t;
        if cap <= leftover {
            total += cap;
            break;
        }
        total += leftover;
        remaining_t -= leftover / limit;
        if remaining_t <= 0.0 || index + 1 >= edges.len() {
            break;
        }
        index += 1;
        progress = 0.0;
    }
    total.is_finite().then_some(total.max(0.0))
}

#[allow(clippy::too_many_arguments)]
fn constrain_upcoming_speed_limits(
    current_speed: f64,
    mut next_speed: f64,
    delta_s: f64,
    edges: &[LaneEdgeOrdinal],
    lengths: &[f64],
    speed_limits: &[f64],
    cursor: usize,
    progress: f64,
    comfort: f64,
    emergency: f64,
) -> Option<f64> {
    for (index, edge) in edges.iter().enumerate().skip(cursor + 1) {
        let limit = *speed_limits.get(edge.index())?;
        if !limit.is_finite() || limit + 1e-12 >= next_speed {
            continue;
        }
        let distance = remaining_along_route(lengths, edges, cursor, progress, index, 0.0)?;
        if !distance.is_finite() || distance < 0.0 {
            return None;
        }
        if distance <= 1e-12 {
            next_speed = next_speed.min(limit.max(0.0));
            continue;
        }
        next_speed = cap_next_speed_for_limit(
            current_speed,
            next_speed,
            delta_s,
            distance,
            limit,
            comfort,
            emergency,
        )?;
    }
    Some(next_speed.max(0.0))
}

fn cap_next_speed_for_limit(
    current_speed: f64,
    next_speed: f64,
    delta_s: f64,
    distance: f64,
    limit: f64,
    comfort: f64,
    emergency: f64,
) -> Option<f64> {
    if let Some(capped) =
        max_next_speed_for_decel(current_speed, next_speed, delta_s, distance, limit, comfort)
    {
        return Some(capped);
    }
    max_next_speed_for_decel(
        current_speed,
        next_speed,
        delta_s,
        distance,
        limit,
        emergency,
    )
    .or(Some(0.0))
}

fn max_next_speed_for_decel(
    current_speed: f64,
    next_speed: f64,
    delta_s: f64,
    distance: f64,
    limit: f64,
    decel: f64,
) -> Option<f64> {
    if decel <= 0.0 || delta_s <= 0.0 {
        return None;
    }
    let limit = limit.max(0.0);
    if 0.5 * current_speed * delta_s > distance + 1e-12 {
        return None;
    }
    let linear = ((2.0 * distance / delta_s) - current_speed)
        .min(limit)
        .min(next_speed)
        .max(0.0);
    let b_dt = decel * delta_s;
    let constant = decel * current_speed * delta_s - limit * limit - 2.0 * decel * distance;
    let discriminant = b_dt * b_dt - 4.0 * constant;
    let quadratic = if discriminant >= 0.0 {
        ((-b_dt + discriminant.sqrt()) / 2.0).min(next_speed)
    } else {
        f64::NEG_INFINITY
    };
    let mut best = linear;
    if quadratic > limit
        && speed_down_constraint_holds(current_speed, quadratic, delta_s, distance, limit, decel)
    {
        best = best.max(quadratic);
    }
    speed_down_constraint_holds(current_speed, best, delta_s, distance, limit, decel)
        .then_some(best.min(next_speed).max(0.0))
}

fn speed_down_constraint_holds(
    current_speed: f64,
    next_speed: f64,
    delta_s: f64,
    distance: f64,
    limit: f64,
    decel: f64,
) -> bool {
    let travel = 0.5 * (current_speed + next_speed) * delta_s;
    let braking = (next_speed * next_speed - limit * limit).max(0.0) / (2.0 * decel);
    travel + braking <= distance + 1e-9
}

#[allow(clippy::too_many_arguments)]
fn clamp_travel_to_speed_down_boundary(
    mut travel: f64,
    current_speed: f64,
    next_speed: f64,
    delta_s: f64,
    edges: &[LaneEdgeOrdinal],
    lengths: &[f64],
    speed_limits: &[f64],
    cursor: usize,
    progress: f64,
) -> Option<f64> {
    let min_travel = 0.5 * current_speed * delta_s;
    for (index, edge) in edges.iter().enumerate().skip(cursor + 1) {
        let limit = *speed_limits.get(edge.index())?;
        if !limit.is_finite() || limit + 1e-12 >= current_speed || limit + 1e-12 >= next_speed {
            continue;
        }
        let distance = remaining_along_route(lengths, edges, cursor, progress, index, 0.0)?;
        if distance <= 1e-12 {
            continue;
        }
        // 能停在更低限速边之前时才钳在边界；min_travel 已超过距离则本拍必须进入，
        // 否则会把行程压到低于 0.5 v Δt，再被夹成在入口边尽头静止。
        if min_travel <= distance + 1e-12 && travel > distance {
            travel = travel.min(distance);
        }
    }
    Some(travel.max(0.0))
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

    fn install_preview_world() -> TrafficWorld {
        let input = check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD).unwrap();
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .unwrap();
        TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 100)).unwrap()
    }

    #[test]
    fn successful_ticks_reuse_preallocated_scratch() {
        let mut world = install_preview_world();
        let route = world.static_route(StaticRouteOrdinal::from_raw(0)).unwrap();
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1.0,
                0.0,
            ))
            .unwrap();
        world.step(TickInput::new(100)).unwrap();
        let next_cap = world.next_states.capacity();
        let live_cap = world.live_order.capacity();
        let vehicle_cap = world.vehicles.capacity();
        for _ in 0..16 {
            world.step(TickInput::new(100)).unwrap();
            assert_eq!(world.next_states.capacity(), next_cap);
            assert_eq!(world.live_order.capacity(), live_cap);
            assert_eq!(world.vehicles.capacity(), vehicle_cap);
        }
    }

    #[test]
    fn step_error_is_copy_without_diagnostic_allocation() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<StepError>();
        assert!(
            std::mem::size_of::<StepError>() <= 32,
            "StepError must stay a small Copy code, size={}",
            std::mem::size_of::<StepError>()
        );
    }

    #[test]
    fn overflow_step_leaves_committed_time_unchanged() {
        let mut world = install_preview_world();
        world.tick_index = u64::MAX;
        let time = world.time_ms;
        assert_eq!(world.step(TickInput::new(100)), Err(StepError::Overflow));
        assert_eq!(world.tick_index, u64::MAX);
        assert_eq!(world.time_ms, time);
    }

    #[test]
    fn non_finite_motion_after_staging_does_not_commit_earlier_vehicles() {
        let mut world = install_preview_world();
        let route = world.static_route(StaticRouteOrdinal::from_raw(0)).unwrap();
        let profile = world
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .unwrap();
        let first = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1.0 + profile.length() + profile.min_gap() + 2.0,
                0.0,
            ))
            .unwrap();
        let second = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1.0,
                0.0,
            ))
            .unwrap();
        let before_progress = world.vehicle_state(first).unwrap().progress;
        let before_tick = world.tick_index;
        let slot = usize::try_from(second.index()).unwrap();
        world.vehicles[slot].state.as_mut().unwrap().progress = f64::NAN;
        assert_eq!(
            world.step(TickInput::new(100)),
            Err(StepError::NonFiniteMotion)
        );
        assert_eq!(world.tick_index, before_tick);
        assert_eq!(
            world.vehicle_state(first).unwrap().progress,
            before_progress
        );
        assert_eq!(world.time_ms, 0);
    }
}
