use laneflow_static_contract::{LaneEdgeOrdinal, SignalAspect};
use laneflow_static_network::{BoundedDistance, VehicleProfileView};

#[cfg(test)]
use crate::tables::occupancy_front_gap;
use crate::tables::{CompiledRoute, remaining_to_occurrence_start, remaining_to_route_end};
use crate::units::{round_mm, round_um};
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
        let delta_s = expected as f32 / 1_000.0;
        self.rebuild_occupancy_index()?;
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
        delta_s: f32,
    ) -> Option<VehicleState> {
        let compiled = self.compiled_route(state.route)?;
        let edges = compiled.edges.as_ref();
        let cursor = usize::try_from(state.route_edge_index).ok()?;
        let edge = *edges.get(cursor)?;
        let lengths = self.revision.traffic().lane_lengths_millimetres();
        let speed_limits = self
            .revision
            .traffic()
            .lane_speed_limits_millimetres_per_second();
        let current_limit = *speed_limits.get(edge.index())?;
        let profile = self
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)?;
        let desired_mm_s = profile.desired_speed_mm_s().min(current_limit);
        let leader_gap = self.leader_bumper_gap(&state, edges, lengths);
        let route_end =
            remaining_to_route_end(*compiled.remaining_to_end.get(cursor)?, state.progress_mm);
        let signal_stop = self.signal_stop_distance(compiled, &state, cursor);
        let (mut travel_m, next_speed_m) = si_comfort_travel(
            state.speed_mm_s,
            desired_mm_s,
            leader_gap,
            profile,
            route_end,
            signal_stop,
            compiled,
            lengths,
            speed_limits,
            cursor,
            state.progress_mm,
            delta_s,
        )?;
        if travel_m < 0.0 {
            travel_m = 0.0;
        }
        if !travel_m.is_finite() || !next_speed_m.is_finite() {
            return None;
        }

        let hard_room = hard_room_mm(
            leader_gap,
            profile.min_gap_mm(),
            signal_stop,
            route_end,
            lengths.get(edge.index()).copied()?,
            state.progress_mm,
            self.hop_permitted(state.route, edges, cursor),
        );
        if hard_room == 0 {
            state.speed_mm_s = 0;
            state.carry_um = 0;
            if matches!(route_end, BoundedDistance::Finite(0)) {
                state.status = VehicleStatus::Completed;
            }
            return Some(state);
        }

        let um = u64::from(state.carry_um).saturating_add(round_um(f64::from(travel_m))?);
        let travel_mm = u32::try_from((um / 1_000).min(u64::from(hard_room))).ok()?;
        let exhausted = travel_mm == hard_room;
        if exhausted {
            state.carry_um = 0;
        } else {
            state.carry_um = u16::try_from(um % 1_000).ok()?;
        }
        let route = state.route;
        apply_travel_mm(&mut state, edges, lengths, travel_mm, |index| {
            self.hop_permitted(route, edges, index)
        })?;
        let committed_index = usize::try_from(state.route_edge_index).ok()?;
        let committed_edge = *edges.get(committed_index)?;
        let committed_limit = *speed_limits.get(committed_edge.index())?;
        let remaining = remaining_to_route_end(
            *compiled.remaining_to_end.get(committed_index)?,
            state.progress_mm,
        );
        if exhausted || matches!(remaining, BoundedDistance::Finite(0)) {
            state.speed_mm_s = 0;
            state.carry_um = 0;
            if matches!(remaining, BoundedDistance::Finite(0)) {
                state.status = VehicleStatus::Completed;
            }
            return Some(state);
        }
        let speed_mm_s = round_mm(f64::from(next_speed_m))?.min(committed_limit);
        state.speed_mm_s = speed_mm_s;
        Some(state)
    }

    /// 读本拍占用索引上的前保险杠间隙。调用前必须已 `rebuild_occupancy_index`。
    pub(crate) fn leader_bumper_gap(
        &self,
        follower: &VehicleState,
        edges: &[LaneEdgeOrdinal],
        lengths: &[u32],
    ) -> Option<i64> {
        let cursor = usize::try_from(follower.route_edge_index).ok()?;
        self.occupancy.leader_gap(
            follower.handle,
            edges,
            cursor,
            follower.progress_mm,
            lengths,
        )
    }

    /// `cfg(test)` 全扫描预言机，不是生产热路径。
    #[cfg(test)]
    pub(crate) fn leader_bumper_gap_scan(
        &self,
        follower: &VehicleState,
        edges: &[LaneEdgeOrdinal],
        lengths: &[u32],
    ) -> Option<i64> {
        let cursor = usize::try_from(follower.route_edge_index).ok()?;
        let mut best: Option<i64> = None;
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
                follower.progress_mm,
                leader_edges,
                leader_index,
                leader.progress_mm,
                leader.length_mm,
            ) else {
                continue;
            };
            best = Some(best.map_or(gap, |current| current.min(gap)));
        }
        best
    }

    /// 下一受控门是拓扑链。绿灯则沿链继续，直到当前限制的门；不要在注册时冻红灯列。
    pub(crate) fn signal_stop_distance(
        &self,
        compiled: &CompiledRoute,
        state: &VehicleState,
        cursor: usize,
    ) -> Option<BoundedDistance> {
        let mut hop = cursor;
        while hop < compiled.next_controlled.len() {
            let Some(next) = compiled.next_controlled[hop] else {
                return None;
            };
            if self.gate_is_restrictive(next.gate) {
                let hop_index = usize::try_from(next.hop).ok()?;
                return remaining_to_occurrence_start(
                    &compiled.remaining_to_end,
                    cursor,
                    state.progress_mm,
                    hop_index + 1,
                );
            }
            let next_hop = usize::try_from(next.hop).ok()?.checked_add(1)?;
            if next_hop <= hop {
                return None;
            }
            hop = next_hop;
        }
        None
    }

    fn hop_permitted(
        &self,
        route: crate::RouteHandle,
        edges: &[LaneEdgeOrdinal],
        hop_index: usize,
    ) -> bool {
        if hop_index + 1 >= edges.len() {
            return false;
        }
        let Some(compiled) = self.compiled_route(route) else {
            return false;
        };
        match compiled.hop_gate.get(hop_index).copied().flatten() {
            Some(gate) => !self.gate_is_restrictive(gate),
            None => true,
        }
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

fn si_meters(mm: u32) -> f32 {
    mm as f32 / 1_000.0
}

fn si_speed(mm_s: u32) -> f32 {
    mm_s as f32 / 1_000.0
}

fn finite_meters(distance: BoundedDistance) -> Option<f32> {
    match distance {
        BoundedDistance::Finite(mm) => Some(si_meters(mm)),
        BoundedDistance::BeyondFinite => None,
    }
}

/// 本拍硬约束。`BeyondFinite` 路终/停车距离不参与包络；Finite 侧保持 `u32`，不上 `u64`。
fn hard_room_mm(
    leader_gap: Option<i64>,
    min_gap_mm: u32,
    signal_stop: Option<BoundedDistance>,
    route_end: BoundedDistance,
    edge_length_mm: u32,
    progress_mm: u32,
    hop_permitted: bool,
) -> u32 {
    let mut room = u32::MAX;
    if let Some(gap) = leader_gap {
        let leftover = gap.saturating_sub(i64::from(min_gap_mm));
        let leader_room = if leftover <= 0 {
            0
        } else {
            u32::try_from(leftover).unwrap_or(u32::MAX)
        };
        room = room.min(leader_room);
    }
    if let Some(BoundedDistance::Finite(stop)) = signal_stop {
        room = room.min(stop);
    }
    if let BoundedDistance::Finite(remaining) = route_end {
        room = room.min(remaining);
    }
    if !hop_permitted {
        room = room.min(edge_length_mm.saturating_sub(progress_mm));
    }
    room
}

fn leader_gap_m(gap: Option<i64>) -> Option<f32> {
    gap.map(|gap| if gap <= 0 { 0.0 } else { gap as f32 / 1_000.0 })
}

#[allow(clippy::too_many_arguments)]
fn si_comfort_travel(
    speed_mm_s: u32,
    desired_mm_s: u32,
    leader_gap: Option<i64>,
    profile: VehicleProfileView,
    route_end: BoundedDistance,
    signal_stop: Option<BoundedDistance>,
    compiled: &CompiledRoute,
    lengths: &[u32],
    speed_limits: &[u32],
    cursor: usize,
    progress_mm: u32,
    delta_s: f32,
) -> Option<(f32, f32)> {
    let speed = si_speed(speed_mm_s);
    let desired = si_speed(desired_mm_s);
    let leader_m = leader_gap_m(leader_gap);
    let min_gap_m = si_meters(profile.min_gap_mm());
    let envelope = speed_limit_path_envelope(
        compiled.edges.as_ref(),
        lengths,
        speed_limits,
        cursor,
        progress_mm,
        delta_s,
    )?;
    let (mut travel, mut next_speed) = iidm_travel(speed, desired, leader_m, profile, delta_s)?;
    travel = clamp_si_travel(
        travel,
        leader_m,
        min_gap_m,
        signal_stop,
        route_end,
        envelope,
    );
    if !travel.is_finite() || !next_speed.is_finite() {
        return None;
    }
    next_speed = constrain_upcoming_speed_limits(
        speed,
        next_speed,
        delta_s,
        compiled,
        cursor,
        progress_mm,
        profile.comfort_decel(),
        profile.emergency_decel(),
    )?;
    travel = ((speed + next_speed) * 0.5 * delta_s).max(0.0);
    travel = clamp_si_travel(
        travel,
        leader_m,
        min_gap_m,
        signal_stop,
        route_end,
        envelope,
    );
    travel = clamp_travel_to_speed_down_boundary(
        travel,
        speed,
        next_speed,
        delta_s,
        compiled,
        cursor,
        progress_mm,
    )?;
    Some((travel.max(0.0), next_speed.max(0.0)))
}

fn clamp_si_travel(
    mut travel: f32,
    leader_m: Option<f32>,
    min_gap_m: f32,
    signal_stop: Option<BoundedDistance>,
    route_end: BoundedDistance,
    envelope: f32,
) -> f32 {
    if let Some(gap) = leader_m {
        travel = travel.min((gap - min_gap_m).max(0.0));
    }
    if let Some(stop) = signal_stop.and_then(finite_meters) {
        travel = travel.min(stop.max(0.0));
    }
    if let Some(end) = finite_meters(route_end) {
        travel = travel.min(end.max(0.0));
    }
    travel.min(envelope).max(0.0)
}

fn iidm_travel(
    speed: f32,
    desired: f32,
    leader_gap: Option<f32>,
    profile: VehicleProfileView,
    delta_s: f32,
) -> Option<(f32, f32)> {
    iidm_step(
        speed,
        desired,
        leader_gap,
        si_meters(profile.min_gap_mm()),
        profile.time_headway(),
        profile.max_accel(),
        profile.comfort_decel(),
        profile.emergency_decel(),
        delta_s,
    )
}

#[allow(clippy::too_many_arguments)]
fn iidm_step(
    speed: f32,
    desired: f32,
    leader_gap: Option<f32>,
    min_gap_m: f32,
    time_headway: f32,
    accel_max: f32,
    comfort: f32,
    emergency: f32,
    delta_s: f32,
) -> Option<(f32, f32)> {
    if !speed.is_finite() || !desired.is_finite() || delta_s <= 0.0 {
        return None;
    }
    if accel_max <= 0.0 || comfort <= 0.0 || emergency <= 0.0 {
        return None;
    }
    if leader_gap.is_some_and(|gap| gap <= 0.0) {
        return Some((0.0, 0.0));
    }
    let speed_term = if desired <= 0.0 {
        1.0
    } else {
        (speed / desired).max(0.0).powi(4)
    };
    let gap_term = if let Some(gap) = leader_gap {
        let s_star = min_gap_m + speed * time_headway;
        (s_star / gap).max(0.0).powi(2)
    } else {
        0.0
    };
    let accel = accel_max * (1.0 - speed_term - gap_term);
    let next_speed = (speed + accel * delta_s).max(0.0).min(desired.max(0.0));
    let travel = ((speed + next_speed) * 0.5 * delta_s).max(0.0);
    (travel.is_finite() && next_speed.is_finite()).then_some((travel, next_speed))
}

fn speed_limit_path_envelope(
    edges: &[LaneEdgeOrdinal],
    lengths: &[u32],
    speed_limits: &[u32],
    mut index: usize,
    mut progress_mm: u32,
    delta_s: f32,
) -> Option<f32> {
    if delta_s <= 0.0 {
        return None;
    }
    let mut remaining_t = delta_s;
    let mut total = 0.0;
    loop {
        let edge = *edges.get(index)?;
        let length = si_meters(*lengths.get(edge.index())?);
        let limit = si_speed(*speed_limits.get(edge.index())?);
        let leftover = (length - si_meters(progress_mm)).max(0.0);
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
        progress_mm = 0;
    }
    total.is_finite().then_some(total.max(0.0))
}

#[allow(clippy::too_many_arguments)]
/// 本世界限速下降转换。不扫剩余边；限速值写在 drop 列，与共享根边热列同形。
fn constrain_upcoming_speed_limits(
    current_speed: f32,
    mut next_speed: f32,
    delta_s: f32,
    compiled: &CompiledRoute,
    cursor: usize,
    progress_mm: u32,
    comfort: f32,
    emergency: f32,
) -> Option<f32> {
    for drop in compiled.speed_limit_drop.iter() {
        let from = usize::try_from(drop.from_route_edge_index).ok()?;
        if from < cursor {
            continue;
        }
        let limit = si_speed(drop.target_mm_s);
        if limit >= next_speed {
            continue;
        }
        let to_index = from.checked_add(1)?;
        match remaining_to_occurrence_start(
            &compiled.remaining_to_end,
            cursor,
            progress_mm,
            to_index,
        )? {
            BoundedDistance::BeyondFinite => continue,
            BoundedDistance::Finite(0) => {
                next_speed = next_speed.min(limit.max(0.0));
            }
            BoundedDistance::Finite(mm) => {
                let distance = si_meters(mm);
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
        }
    }
    Some(next_speed.max(0.0))
}

fn cap_next_speed_for_limit(
    current_speed: f32,
    next_speed: f32,
    delta_s: f32,
    distance: f32,
    limit: f32,
    comfort: f32,
    emergency: f32,
) -> Option<f32> {
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
    current_speed: f32,
    next_speed: f32,
    delta_s: f32,
    distance: f32,
    limit: f32,
    decel: f32,
) -> Option<f32> {
    if decel <= 0.0 || delta_s <= 0.0 {
        return None;
    }
    let limit = limit.max(0.0);
    if 0.5 * current_speed * delta_s > distance {
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
        f32::NEG_INFINITY
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
    current_speed: f32,
    next_speed: f32,
    delta_s: f32,
    distance: f32,
    limit: f32,
    decel: f32,
) -> bool {
    let travel = 0.5 * (current_speed + next_speed) * delta_s;
    let braking = (next_speed * next_speed - limit * limit).max(0.0) / (2.0 * decel);
    travel + braking <= distance
}

#[allow(clippy::too_many_arguments)]
fn clamp_travel_to_speed_down_boundary(
    mut travel: f32,
    current_speed: f32,
    next_speed: f32,
    delta_s: f32,
    compiled: &CompiledRoute,
    cursor: usize,
    progress_mm: u32,
) -> Option<f32> {
    let min_travel = 0.5 * current_speed * delta_s;
    for drop in compiled.speed_limit_drop.iter() {
        let from = usize::try_from(drop.from_route_edge_index).ok()?;
        if from < cursor {
            continue;
        }
        let limit = si_speed(drop.target_mm_s);
        if limit >= current_speed || limit >= next_speed {
            continue;
        }
        let to_index = from.checked_add(1)?;
        let BoundedDistance::Finite(mm) = remaining_to_occurrence_start(
            &compiled.remaining_to_end,
            cursor,
            progress_mm,
            to_index,
        )?
        else {
            continue;
        };
        if mm == 0 {
            continue;
        }
        let distance = si_meters(mm);
        if min_travel <= distance && travel > distance {
            travel = travel.min(distance);
        }
    }
    Some(travel.max(0.0))
}

fn apply_travel_mm(
    state: &mut VehicleState,
    edges: &[LaneEdgeOrdinal],
    lengths: &[u32],
    mut remaining: u32,
    hop_permitted: impl Fn(usize) -> bool,
) -> Option<()> {
    let mut index = usize::try_from(state.route_edge_index).ok()?;
    while remaining > 0 {
        let edge = *edges.get(index)?;
        let edge_length = *lengths.get(edge.index())?;
        let leftover = edge_length.saturating_sub(state.progress_mm);
        if remaining < leftover {
            state.progress_mm = state.progress_mm.saturating_add(remaining);
            break;
        }
        remaining -= leftover;
        if !hop_permitted(index) || index + 1 >= edges.len() {
            state.progress_mm = edge_length;
            break;
        }
        index += 1;
        state.progress_mm = 0;
    }
    state.route_edge_index = u32::try_from(index).ok()?;
    Some(())
}

#[cfg(test)]
mod preview {
    use super::*;

    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_static_contract::{
        LaneEdgeOrdinal, ParticipantClassOrdinal, VehicleProfileOrdinal,
    };
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    use crate::{RouteHandle, RouteRegisterInput, VehicleHandle, VehicleSpawnInput, WorldConfig};

    fn preview_route(world: &mut TrafficWorld) -> RouteHandle {
        let traffic = world.traffic();
        let mut edges = Vec::new();
        let count = traffic.lane_edge_count();
        for raw in 0..count {
            let edge = LaneEdgeOrdinal::from_raw(raw);
            if traffic.relations().lane_edge_junction(edge).is_some() {
                continue;
            }
            if traffic.relations().stop_line_for_edge(edge).is_some() {
                continue;
            }
            edges.push(edge);
            if let Some(succ) = traffic
                .successors(edge)
                .and_then(|items| items.first().copied())
            {
                if traffic.relations().stop_line_for_edge(succ).is_none() {
                    edges.push(succ);
                }
            }
            break;
        }
        world
            .register_route(RouteRegisterInput::new(edges))
            .expect("preview route")
    }

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
    );

    #[test]
    fn preview_follower_constraints() {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).unwrap();
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .unwrap();
        let mut world = TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 100)).unwrap();
        let route = preview_route(&mut world);
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
                1_000 + profile.length_mm() + profile.min_gap_mm() + 2_000,
                0,
            ))
            .unwrap();
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .unwrap();
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        let state = world.vehicle_state(follower).copied().unwrap();
        let next = world.advance_active_vehicle(state, 0.1_f32).unwrap();
        assert!(
            next.progress_mm > state.progress_mm || next.carry_um > state.carry_um,
            "follower should start moving, {} -> {}",
            state.progress_mm,
            next.progress_mm
        );
    }

    fn install_preview_world() -> TrafficWorld {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).unwrap();
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
        let route = preview_route(&mut world);
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .unwrap();
        world.step(TickInput::new(100)).unwrap();
        let next_cap = world.next_states.capacity();
        let live_cap = world.live_order.capacity();
        let vehicle_cap = world.vehicles.capacity();
        let occupancy_records = world.occupancy.records_capacity();
        let occupancy_scratch = world.occupancy.scratch_capacity();
        let occupancy_offsets = world.occupancy.offsets_capacity();
        let occupancy_suffix = world.occupancy.suffix_min_lo_capacity();
        let occupancy_second = world.occupancy.suffix_second_lo_capacity();
        for _ in 0..16 {
            world.step(TickInput::new(100)).unwrap();
            assert_eq!(world.next_states.capacity(), next_cap);
            assert_eq!(world.live_order.capacity(), live_cap);
            assert_eq!(world.vehicles.capacity(), vehicle_cap);
            assert_eq!(world.occupancy.records_capacity(), occupancy_records);
            assert_eq!(world.occupancy.scratch_capacity(), occupancy_scratch);
            assert_eq!(world.occupancy.offsets_capacity(), occupancy_offsets);
            assert_eq!(world.occupancy.suffix_min_lo_capacity(), occupancy_suffix);
            assert_eq!(
                world.occupancy.suffix_second_lo_capacity(),
                occupancy_second
            );
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
        let route = preview_route(&mut world);
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
                1_000 + profile.length_mm() + profile.min_gap_mm() + 2_000,
                0,
            ))
            .unwrap();
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .unwrap();
        let before_progress = world.vehicle_state(first).unwrap().progress_mm;
        let before_tick = world.tick_index;
        assert_eq!(
            world.step(TickInput::new(50)),
            Err(StepError::DeltaMismatch {
                expected_delta_time_ms: 100,
                actual_delta_time_ms: 50,
            })
        );
        assert_eq!(world.tick_index, before_tick);
        assert_eq!(
            world.vehicle_state(first).unwrap().progress_mm,
            before_progress
        );
        assert_eq!(world.time_ms, 0);
    }

    fn travel_state(route_edge_index: u32, progress_mm: u32) -> VehicleState {
        VehicleState {
            handle: VehicleHandle::new(0, 0),
            profile: VehicleProfileOrdinal::from_raw(0),
            class: ParticipantClassOrdinal::from_raw(0),
            route: RouteHandle::new(0, 0),
            route_edge_index,
            progress_mm,
            carry_um: 0,
            speed_mm_s: 0,
            length_mm: 4_500,
            status: VehicleStatus::Active,
            parking: None,
        }
    }

    #[test]
    fn apply_travel_hops_when_remaining_equals_leftover_and_hop_is_permitted() {
        let edges = [LaneEdgeOrdinal::from_raw(0), LaneEdgeOrdinal::from_raw(1)];
        let lengths = [1_000, 2_000];
        let mut state = travel_state(0, 500);
        apply_travel_mm(&mut state, &edges, &lengths, 500, |index| {
            index + 1 < edges.len()
        })
        .unwrap();
        assert_eq!(state.route_edge_index, 1);
        assert_eq!(state.progress_mm, 0);
    }

    #[test]
    fn apply_travel_stays_at_length_when_remaining_equals_leftover_and_hop_is_denied() {
        let edges = [LaneEdgeOrdinal::from_raw(0), LaneEdgeOrdinal::from_raw(1)];
        let lengths = [1_000, 2_000];
        let mut state = travel_state(0, 500);
        apply_travel_mm(&mut state, &edges, &lengths, 500, |_| false).unwrap();
        assert_eq!(state.route_edge_index, 0);
        assert_eq!(state.progress_mm, 1_000);
    }

    #[test]
    fn hard_stop_clears_carry_um() {
        let mut world = install_preview_world();
        let route = preview_route(&mut world);
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
                1_000 + profile.length_mm() + profile.min_gap_mm(),
                0,
            ))
            .unwrap();
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .unwrap();
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        let mut state = world.vehicle_state(follower).copied().unwrap();
        state.carry_um = 777;
        let next = world.advance_active_vehicle(state, 0.1_f32).unwrap();
        assert_eq!(next.carry_um, 0);
        assert_eq!(next.speed_mm_s, 0);
        assert_eq!(next.progress_mm, 1_000);
        assert_eq!(next.status, VehicleStatus::Active);
    }

    #[test]
    fn crawl_retains_sub_millimetre_carry() {
        let mut world = install_preview_world();
        let route = preview_route(&mut world);
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .unwrap();
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        let state = world.vehicle_state(follower).copied().unwrap();
        let next = world.advance_active_vehicle(state, 0.004_f32).unwrap();
        assert_eq!(next.progress_mm, state.progress_mm);
        assert!(next.carry_um > state.carry_um);
        assert!(next.speed_mm_s > 0);
        assert_eq!(next.status, VehicleStatus::Active);
    }

    #[test]
    fn iidm_committed_speed_rounds_f32_si() {
        let (_, next) = iidm_step(
            si_speed(9_639),
            si_speed(12_516),
            Some(5_560.0_f32 / 1_000.0),
            2.0,
            1.4,
            1.8,
            2.0,
            4.5,
            8.0_f32 / 1_000.0,
        )
        .unwrap();
        assert_eq!(round_mm(f64::from(next)), Some(9_536));
    }
}
