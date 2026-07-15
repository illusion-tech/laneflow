//! Vehicle Following 的私有纵向计算与 no-overlap 投影。

use crate::{
    CoreError, IidmProfileSpec, VehicleHandle, occupancy::LeaderObservation,
    profile::GEOMETRY_GAP_EPSILON, signal::SignalStopConstraint,
};

const UNVISITED: u8 = 0;
const VISITING: u8 = 1;
const RESOLVED: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LeaderKinematics {
    pub(crate) observation: LeaderObservation,
    pub(crate) current_speed: f64,
    pub(crate) emergency_deceleration: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LongitudinalMotion {
    pub(crate) vehicle: VehicleHandle,
    update_sequence: u64,
    current_speed: f64,
    leader: Option<LeaderObservation>,
    candidate_speed: f64,
    candidate_travel: f64,
    emergency_min_travel: f64,
    final_speed: f64,
    final_travel: f64,
    route_end_distance: Option<f64>,
    signal_stop: Option<SignalStopConstraint>,
    signal_projection_applied: bool,
    safety_projection_applied: bool,
}

impl LongitudinalMotion {
    pub(crate) fn stationary(vehicle: VehicleHandle, update_sequence: u64) -> Self {
        Self {
            vehicle,
            update_sequence,
            current_speed: 0.0,
            leader: None,
            candidate_speed: 0.0,
            candidate_travel: 0.0,
            emergency_min_travel: 0.0,
            final_speed: 0.0,
            final_travel: 0.0,
            route_end_distance: None,
            signal_stop: None,
            signal_projection_applied: false,
            safety_projection_applied: false,
        }
    }

    pub(crate) fn cap_to_route_end(
        &mut self,
        distance: f64,
        delta_time: f64,
    ) -> Result<(), CoreError> {
        self.route_end_distance = Some(distance);
        if distance >= self.candidate_travel {
            return Ok(());
        }

        self.candidate_speed = speed_after_travel_cap(
            self.vehicle,
            self.candidate_speed,
            self.candidate_travel,
            distance,
            delta_time,
        )?;
        self.candidate_travel = distance;
        self.emergency_min_travel = self.emergency_min_travel.min(distance);
        self.final_speed = self.candidate_speed;
        self.final_travel = distance;
        Ok(())
    }

    pub(crate) fn apply_signal_stop(
        &mut self,
        constraint: SignalStopConstraint,
        profile: IidmProfileSpec,
        delta_time: f64,
    ) -> Result<(), CoreError> {
        self.signal_stop = Some(constraint);
        let result = (|| {
            let speed_ceiling = safe_speed(
                self.vehicle,
                self.current_speed,
                profile.comfortable_deceleration,
                0.0,
                profile.comfortable_deceleration,
                constraint.route_distance,
                delta_time,
            )?;
            let emergency_speed_step = finite(
                self.vehicle,
                "signal_emergency_speed_step",
                profile.emergency_deceleration * delta_time,
            )?;
            let emergency_floor = (self.current_speed - emergency_speed_step).max(0.0);
            let constrained_speed = self.candidate_speed.min(speed_ceiling.max(emergency_floor));
            if constrained_speed < self.candidate_speed {
                let acceleration = finite(
                    self.vehicle,
                    "signal_candidate_acceleration",
                    (constrained_speed - self.current_speed) / delta_time,
                )?;
                let candidate =
                    ballistic_motion(self.vehicle, self.current_speed, acceleration, delta_time)?;
                self.candidate_speed = candidate.speed;
                self.candidate_travel = candidate.travel;
            }

            let travel_before_hard_projection = self.candidate_travel;
            if constraint.route_distance < self.candidate_travel {
                self.candidate_speed = speed_after_travel_cap(
                    self.vehicle,
                    self.candidate_speed,
                    self.candidate_travel,
                    constraint.route_distance,
                    delta_time,
                )?;
                self.candidate_travel = constraint.route_distance;
            }
            if self.candidate_travel + GEOMETRY_GAP_EPSILON >= constraint.route_distance {
                self.candidate_speed = 0.0;
            }

            self.final_speed = self.candidate_speed;
            self.final_travel = self.candidate_travel;
            self.signal_projection_applied = constraint.route_distance + GEOMETRY_GAP_EPSILON
                < self.emergency_min_travel
                && constraint.route_distance < travel_before_hard_projection;
            Ok(())
        })();

        result.map_err(signal_stop_error)
    }

    pub(crate) const fn final_speed(self) -> f64 {
        self.final_speed
    }

    pub(crate) const fn final_travel(self) -> f64 {
        self.final_travel
    }

    pub(crate) fn applied_acceleration(self, delta_time: f64) -> Result<f64, CoreError> {
        finite(
            self.vehicle,
            "applied_acceleration",
            (self.final_speed - self.current_speed) / delta_time,
        )
    }

    pub(crate) fn safety_projection_leader(self) -> Option<VehicleHandle> {
        self.safety_projection_applied.then(|| {
            self.leader
                .expect("projection motion must have a leader")
                .leader
        })
    }

    pub(crate) fn signal_stop_projection(self) -> Option<SignalStopConstraint> {
        self.signal_projection_applied.then(|| {
            self.signal_stop
                .expect("signal projection must have attribution")
        })
    }

    pub(crate) fn reaches_route_end(self) -> bool {
        self.route_end_distance
            .is_some_and(|distance| self.final_travel + GEOMETRY_GAP_EPSILON >= distance)
    }

    fn apply_geometry_cap(
        &mut self,
        leader_final_travel: f64,
        delta_time: f64,
    ) -> Result<(), CoreError> {
        let Some(leader) = self.leader else {
            return Ok(());
        };
        let geometry_cap = finite(
            self.vehicle,
            "geometry_cap",
            leader.bumper_gap + leader_final_travel,
        )?
        .max(0.0);
        let travel_before_geometry_projection = self.candidate_travel;
        let final_travel = travel_before_geometry_projection.min(geometry_cap);

        if final_travel < self.candidate_travel {
            self.final_speed = speed_after_travel_cap(
                self.vehicle,
                self.candidate_speed,
                self.candidate_travel,
                final_travel,
                delta_time,
            )?;
        } else {
            self.final_speed = self.candidate_speed;
        }
        self.final_travel = final_travel;
        self.safety_projection_applied = final_travel + GEOMETRY_GAP_EPSILON
            < travel_before_geometry_projection
            && final_travel + GEOMETRY_GAP_EPSILON < self.emergency_min_travel;
        Ok(())
    }
}

/// 可跨 tick 复用、但不属于 Core authority state 的纵向派生 scratch。
#[derive(Clone, Debug, Default)]
pub(crate) struct LongitudinalScratch {
    motions: Vec<Option<LongitudinalMotion>>,
    visit_state: Vec<u8>,
    path: Vec<usize>,
}

impl PartialEq for LongitudinalScratch {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl LongitudinalScratch {
    pub(crate) fn begin(&mut self, vehicle_slot_count: usize) {
        self.motions.clear();
        self.motions.resize(vehicle_slot_count, None);
        self.visit_state.clear();
        self.visit_state.resize(vehicle_slot_count, UNVISITED);
        self.path.clear();
    }

    pub(crate) fn set(&mut self, motion: LongitudinalMotion) {
        let index = motion.vehicle.index();
        if motion.leader.is_none() {
            self.visit_state[index] = RESOLVED;
        }
        self.motions[index] = Some(motion);
    }

    pub(crate) fn motion(&self, vehicle: VehicleHandle) -> Option<LongitudinalMotion> {
        self.motions
            .get(vehicle.index())
            .copied()
            .flatten()
            .filter(|motion| motion.vehicle == vehicle)
    }

    pub(crate) fn project(
        &mut self,
        update_order: &[VehicleHandle],
        delta_time: f64,
    ) -> Result<(), CoreError> {
        let mut path = std::mem::take(&mut self.path);
        let result = (|| {
            for start in update_order.iter().copied() {
                let start_index = start.index();
                if self.motion(start).is_none() || self.visit_state[start_index] == RESOLVED {
                    continue;
                }

                path.clear();
                let mut current = Some(start_index);
                let mut cycle_start = None;

                while let Some(index) = current {
                    match self.visit_state[index] {
                        UNVISITED => {
                            self.visit_state[index] = VISITING;
                            path.push(index);
                            current = self.leader_index(index);
                        }
                        VISITING => {
                            cycle_start = path.iter().position(|candidate| *candidate == index);
                            break;
                        }
                        RESOLVED => break,
                        _ => unreachable!("visit state must be valid"),
                    }
                }

                if let Some(cycle_start) = cycle_start {
                    self.resolve_cycle(&path[cycle_start..], delta_time)?;
                    for index in path[..cycle_start].iter().rev().copied() {
                        self.resolve_node(index, delta_time)?;
                    }
                } else {
                    for index in path.iter().rev().copied() {
                        self.resolve_node(index, delta_time)?;
                    }
                }
            }

            Ok(())
        })();
        self.path = path;
        result
    }

    fn leader_index(&self, index: usize) -> Option<usize> {
        let leader = self.motions[index]?.leader?.leader;
        self.motion(leader).map(|_| leader.index())
    }

    fn resolve_node(&mut self, index: usize, delta_time: f64) -> Result<(), CoreError> {
        if self.visit_state[index] == RESOLVED {
            return Ok(());
        }

        let leader_final_travel = self.leader_index(index).map(|leader_index| {
            self.motions[leader_index]
                .expect("resolved leader motion must exist")
                .final_travel
        });
        if let Some(leader_final_travel) = leader_final_travel {
            self.motions[index]
                .as_mut()
                .expect("motion path node must exist")
                .apply_geometry_cap(leader_final_travel, delta_time)?;
        }
        self.visit_state[index] = RESOLVED;
        Ok(())
    }

    fn resolve_cycle(&mut self, cycle: &[usize], delta_time: f64) -> Result<(), CoreError> {
        debug_assert!(
            cycle.len() >= 2,
            "self leader is excluded by occupancy query"
        );
        let anchor_offset = cycle
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| {
                let left = self.motions[**left].expect("cycle motion must exist");
                let right = self.motions[**right].expect("cycle motion must exist");
                left.candidate_travel
                    .total_cmp(&right.candidate_travel)
                    .then_with(|| left.update_sequence.cmp(&right.update_sequence))
            })
            .map(|(offset, _)| offset)
            .expect("cycle must not be empty");

        let anchor = cycle[anchor_offset];
        self.visit_state[anchor] = RESOLVED;
        for step in 1..cycle.len() {
            let follower_offset = (anchor_offset + cycle.len() - step) % cycle.len();
            self.resolve_node(cycle[follower_offset], delta_time)?;
        }

        let anchor_leader_final = self
            .leader_index(anchor)
            .map(|leader_index| {
                self.motions[leader_index]
                    .expect("cycle leader motion must exist")
                    .final_travel
            })
            .expect("cycle anchor must have a leader");
        let previous_anchor_travel = self.motions[anchor]
            .expect("cycle anchor motion must exist")
            .final_travel;
        self.motions[anchor]
            .as_mut()
            .expect("cycle anchor motion must exist")
            .apply_geometry_cap(anchor_leader_final, delta_time)?;

        // 非负 gap 下 anchor 不会被 closing constraint 收紧。保留一次线性安全回退，
        // 防止极端浮点舍入破坏 closing constraint。
        if self.motions[anchor]
            .expect("cycle anchor motion must exist")
            .final_travel
            < previous_anchor_travel
        {
            for step in 1..cycle.len() {
                let follower_offset = (anchor_offset + cycle.len() - step) % cycle.len();
                let follower = cycle[follower_offset];
                self.visit_state[follower] = VISITING;
                self.resolve_node(follower, delta_time)?;
            }
        }
        self.visit_state[anchor] = RESOLVED;
        Ok(())
    }
}

pub(crate) fn compute_motion(
    vehicle: VehicleHandle,
    update_sequence: u64,
    current_speed: f64,
    profile: IidmProfileSpec,
    leader: Option<LeaderKinematics>,
    delta_time: f64,
) -> Result<LongitudinalMotion, CoreError> {
    let comfort_acceleration = iidm_acceleration(vehicle, current_speed, profile, leader)?;
    let comfort = ballistic_motion(vehicle, current_speed, comfort_acceleration, delta_time)?;
    let emergency_min_travel = emergency_min_travel(
        vehicle,
        current_speed,
        profile.emergency_deceleration,
        delta_time,
    )?;

    let candidate = if let Some(leader) = leader {
        let safe_speed = safe_speed(
            vehicle,
            current_speed,
            profile.emergency_deceleration,
            leader.current_speed,
            leader.emergency_deceleration,
            leader.observation.bumper_gap,
            delta_time,
        )?;
        let emergency_step = finite(
            vehicle,
            "emergency_speed_step",
            profile.emergency_deceleration * delta_time,
        )?;
        let emergency_floor = (current_speed - emergency_step).max(0.0);
        let candidate_speed = comfort.speed.min(safe_speed).max(emergency_floor);

        if candidate_speed == comfort.speed {
            comfort
        } else {
            let acceleration = finite(
                vehicle,
                "candidate_acceleration",
                (candidate_speed - current_speed) / delta_time,
            )?;
            ballistic_motion(vehicle, current_speed, acceleration, delta_time)?
        }
    } else {
        comfort
    };

    Ok(LongitudinalMotion {
        vehicle,
        update_sequence,
        current_speed,
        leader: leader.map(|value| value.observation),
        candidate_speed: candidate.speed,
        candidate_travel: candidate.travel,
        emergency_min_travel,
        final_speed: candidate.speed,
        final_travel: candidate.travel,
        route_end_distance: None,
        signal_stop: None,
        signal_projection_applied: false,
        safety_projection_applied: false,
    })
}

fn signal_stop_error(error: CoreError) -> CoreError {
    match error {
        CoreError::NonFiniteLongitudinalComputation {
            vehicle,
            stage,
            value,
        } => CoreError::NonFiniteSignalStopComputation {
            vehicle,
            stage,
            value,
        },
        error => error,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct BallisticMotion {
    speed: f64,
    travel: f64,
}

fn iidm_acceleration(
    vehicle: VehicleHandle,
    current_speed: f64,
    profile: IidmProfileSpec,
    leader: Option<LeaderKinematics>,
) -> Result<f64, CoreError> {
    let free_acceleration = if current_speed <= profile.desired_speed {
        let ratio = current_speed / profile.desired_speed;
        let speed_term = finite(vehicle, "iidm_free_speed_term", ratio.powi(4))?;
        finite(
            vehicle,
            "iidm_free_acceleration",
            profile.max_acceleration * (1.0 - speed_term),
        )?
    } else {
        let exponent = finite(
            vehicle,
            "iidm_free_exponent",
            profile.max_acceleration * 4.0 / profile.comfortable_deceleration,
        )?;
        let speed_term = finite(
            vehicle,
            "iidm_free_speed_term",
            (profile.desired_speed / current_speed).powf(exponent),
        )?;
        finite(
            vehicle,
            "iidm_free_acceleration",
            -profile.comfortable_deceleration * (1.0 - speed_term),
        )?
    };

    let Some(leader) = leader else {
        return Ok(
            free_acceleration.clamp(-profile.comfortable_deceleration, profile.max_acceleration)
        );
    };
    if leader.observation.bumper_gap <= GEOMETRY_GAP_EPSILON {
        return Ok(-profile.comfortable_deceleration);
    }

    let closing_speed = current_speed - leader.current_speed;
    let sqrt_acceleration_product = finite(
        vehicle,
        "iidm_sqrt_acceleration_product",
        profile.max_acceleration.sqrt() * profile.comfortable_deceleration.sqrt(),
    )?;
    let dynamic_gap = finite(
        vehicle,
        "iidm_dynamic_gap",
        half_product(current_speed, closing_speed) / sqrt_acceleration_product,
    )?;
    let speed_headway = finite(
        vehicle,
        "iidm_speed_headway",
        current_speed * profile.time_headway,
    )?;
    let desired_gap = finite(
        vehicle,
        "iidm_desired_gap",
        profile.min_gap + (speed_headway + dynamic_gap).max(0.0),
    )?;
    let gap_ratio = finite(
        vehicle,
        "iidm_gap_ratio",
        desired_gap / leader.observation.bumper_gap,
    )?;

    let acceleration = if gap_ratio >= 1.0 {
        let gap_term = finite(vehicle, "iidm_gap_term", gap_ratio * gap_ratio)?;
        finite(
            vehicle,
            "iidm_interaction_acceleration",
            profile.max_acceleration * (1.0 - gap_term),
        )?
    } else if free_acceleration > 0.0 {
        let exponent = finite(
            vehicle,
            "iidm_interaction_exponent",
            2.0 * profile.max_acceleration / free_acceleration,
        )?;
        let gap_term = finite(vehicle, "iidm_gap_term", gap_ratio.powf(exponent))?;
        finite(
            vehicle,
            "iidm_interaction_acceleration",
            free_acceleration * (1.0 - gap_term),
        )?
    } else {
        free_acceleration
    };

    Ok(acceleration.clamp(-profile.comfortable_deceleration, profile.max_acceleration))
}

fn safe_speed(
    vehicle: VehicleHandle,
    follower_speed: f64,
    follower_deceleration: f64,
    leader_speed: f64,
    leader_deceleration: f64,
    bumper_gap: f64,
    delta_time: f64,
) -> Result<f64, CoreError> {
    let gap_term = finite(
        vehicle,
        "safe_speed_gap_term",
        multiply_factors([2.0, follower_deceleration, bumper_gap.max(0.0)]),
    )?;
    let leader_term = finite(
        vehicle,
        "safe_speed_leader_term",
        follower_deceleration / leader_deceleration * leader_speed * leader_speed,
    )?;
    let rhs = finite(vehicle, "safe_speed_rhs", gap_term + leader_term)?;
    let follower_step_term = finite(
        vehicle,
        "safe_speed_follower_step_term",
        multiply_factors([follower_deceleration, follower_speed, delta_time]),
    )?;
    let c = finite(vehicle, "safe_speed_c", follower_step_term - rhs)?;
    if c > 0.0 {
        return Ok(0.0);
    }
    if c == 0.0 {
        return Ok(0.0);
    }

    let b = finite(vehicle, "safe_speed_b", follower_deceleration * delta_time)?;
    let sqrt_discriminant = finite(
        vehicle,
        "safe_speed_sqrt_discriminant",
        b.hypot(2.0 * (-c).sqrt()),
    )?;
    let denominator = finite(
        vehicle,
        "safe_speed_denominator",
        0.5 * b + 0.5 * sqrt_discriminant,
    )?;
    let root = finite(vehicle, "safe_speed_root", (-c) / denominator)?.max(0.0);
    Ok(if root > 0.0 { root.next_down() } else { 0.0 })
}

fn ballistic_motion(
    vehicle: VehicleHandle,
    current_speed: f64,
    acceleration: f64,
    delta_time: f64,
) -> Result<BallisticMotion, CoreError> {
    if acceleration < 0.0 {
        let stop_time = finite(
            vehicle,
            "ballistic_stop_time",
            current_speed / -acceleration,
        )?;
        if stop_time < delta_time {
            return Ok(BallisticMotion {
                speed: 0.0,
                travel: braking_distance(vehicle, current_speed, -acceleration)?,
            });
        }
    }

    let speed = finite(
        vehicle,
        "ballistic_speed",
        (current_speed + acceleration * delta_time).max(0.0),
    )?;
    let travel = finite(
        vehicle,
        "ballistic_travel",
        half_product(current_speed, delta_time) + half_product(speed, delta_time),
    )?;
    Ok(BallisticMotion { speed, travel })
}

fn emergency_min_travel(
    vehicle: VehicleHandle,
    current_speed: f64,
    emergency_deceleration: f64,
    delta_time: f64,
) -> Result<f64, CoreError> {
    let speed_step = finite(
        vehicle,
        "emergency_speed_step",
        emergency_deceleration * delta_time,
    )?;
    if current_speed <= speed_step {
        braking_distance(vehicle, current_speed, emergency_deceleration)
    } else {
        finite(
            vehicle,
            "emergency_min_travel",
            (current_speed - 0.5 * speed_step) * delta_time,
        )
    }
}

fn braking_distance(
    vehicle: VehicleHandle,
    speed: f64,
    deceleration: f64,
) -> Result<f64, CoreError> {
    if speed == 0.0 {
        return Ok(0.0);
    }
    let value = if deceleration > f64::MAX / 2.0 {
        speed / deceleration * (0.5 * speed)
    } else {
        let denominator = 2.0 * deceleration;
        if speed < 1.0 {
            speed / (denominator / speed)
        } else {
            speed / denominator * speed
        }
    };
    finite(vehicle, "braking_distance", value)
}

fn speed_after_travel_cap(
    vehicle: VehicleHandle,
    candidate_speed: f64,
    candidate_travel: f64,
    final_travel: f64,
    delta_time: f64,
) -> Result<f64, CoreError> {
    if candidate_speed == 0.0 {
        return Ok(0.0);
    }
    let removed_travel = candidate_travel - final_travel;
    let speed_reduction = finite(
        vehicle,
        "projection_speed_reduction",
        removed_travel / delta_time * 2.0,
    )?;
    Ok((candidate_speed - speed_reduction)
        .max(0.0)
        .min(candidate_speed))
}

fn finite(vehicle: VehicleHandle, stage: &'static str, value: f64) -> Result<f64, CoreError> {
    if !value.is_finite() {
        return Err(CoreError::NonFiniteLongitudinalComputation {
            vehicle,
            stage,
            value,
        });
    }
    Ok(if value == 0.0 { 0.0 } else { value })
}

fn half_product(left: f64, right: f64) -> f64 {
    if left.abs() >= right.abs() {
        (0.5 * left) * right
    } else {
        left * (0.5 * right)
    }
}

fn multiply_factors<const N: usize>(mut factors: [f64; N]) -> f64 {
    factors.sort_unstable_by(|left, right| left.abs().total_cmp(&right.abs()));
    factors.into_iter().product()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vehicle(index: usize) -> VehicleHandle {
        VehicleHandle::new(index, 0)
    }

    fn profile() -> IidmProfileSpec {
        IidmProfileSpec {
            length: 4.5,
            desired_speed: 20.0,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 2.0,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 8.0,
        }
    }

    #[test]
    fn free_road_acceleration_changes_sign_at_desired_speed() {
        assert!(iidm_acceleration(vehicle(0), 10.0, profile(), None).unwrap() > 0.0);
        assert_eq!(
            iidm_acceleration(vehicle(0), 20.0, profile(), None).unwrap(),
            0.0
        );
        assert!(iidm_acceleration(vehicle(0), 30.0, profile(), None).unwrap() < 0.0);
    }

    #[test]
    fn zero_gap_uses_comfortable_deceleration_without_division() {
        let leader = LeaderKinematics {
            observation: LeaderObservation {
                leader: vehicle(1),
                bumper_gap: 0.0,
            },
            current_speed: 0.0,
            emergency_deceleration: 8.0,
        };
        assert_eq!(
            iidm_acceleration(vehicle(0), 10.0, profile(), Some(leader)).unwrap(),
            -2.0
        );
    }

    #[test]
    fn ballistic_motion_stops_inside_tick() {
        let motion = ballistic_motion(vehicle(0), 1.0, -2.0, 1.0).unwrap();
        assert_eq!(motion.speed, 0.0);
        assert_eq!(motion.travel, 0.25);
    }

    #[test]
    fn safe_speed_root_satisfies_stopping_inequality() {
        let speed = safe_speed(vehicle(0), 10.0, 8.0, 0.0, 8.0, 10.0, 1.0).unwrap();
        let lhs = 0.5 * (10.0 + speed) + speed * speed / 16.0;
        assert!(lhs <= 10.0);
        assert!(speed > 0.0);
    }

    #[test]
    fn cycle_projection_uses_deterministic_minimum_anchor() {
        let mut scratch = LongitudinalScratch::default();
        scratch.begin(2);
        scratch.set(LongitudinalMotion {
            vehicle: vehicle(0),
            update_sequence: 0,
            current_speed: 2.0,
            leader: Some(LeaderObservation {
                leader: vehicle(1),
                bumper_gap: 0.0,
            }),
            candidate_speed: 2.0,
            candidate_travel: 2.0,
            emergency_min_travel: 1.0,
            final_speed: 2.0,
            final_travel: 2.0,
            route_end_distance: None,
            signal_stop: None,
            signal_projection_applied: false,
            safety_projection_applied: false,
        });
        scratch.set(LongitudinalMotion {
            vehicle: vehicle(1),
            update_sequence: 1,
            current_speed: 3.0,
            leader: Some(LeaderObservation {
                leader: vehicle(0),
                bumper_gap: 0.0,
            }),
            candidate_speed: 3.0,
            candidate_travel: 3.0,
            emergency_min_travel: 1.0,
            final_speed: 3.0,
            final_travel: 3.0,
            route_end_distance: None,
            signal_stop: None,
            signal_projection_applied: false,
            safety_projection_applied: false,
        });

        scratch.project(&[vehicle(0), vehicle(1)], 1.0).unwrap();

        assert_eq!(scratch.motion(vehicle(0)).unwrap().final_travel(), 2.0);
        assert_eq!(scratch.motion(vehicle(1)).unwrap().final_travel(), 2.0);
    }

    #[test]
    fn route_end_cap_keeps_speed_mapping_until_completion_is_committed() {
        let mut motion = LongitudinalMotion {
            vehicle: vehicle(0),
            update_sequence: 0,
            current_speed: 10.0,
            leader: None,
            candidate_speed: 10.0,
            candidate_travel: 10.0,
            emergency_min_travel: 6.0,
            final_speed: 10.0,
            final_travel: 10.0,
            route_end_distance: None,
            signal_stop: None,
            signal_projection_applied: false,
            safety_projection_applied: false,
        };

        motion.cap_to_route_end(8.0, 1.0).unwrap();

        assert_eq!(motion.final_travel(), 8.0);
        assert_eq!(motion.final_speed(), 6.0);
        assert!(motion.reaches_route_end());
    }

    #[test]
    fn route_end_cap_does_not_expand_epsilon_short_motion() {
        let mut motion = LongitudinalMotion {
            vehicle: vehicle(0),
            update_sequence: 0,
            current_speed: 6.0,
            leader: None,
            candidate_speed: 6.0,
            candidate_travel: 8.0,
            emergency_min_travel: 5.0,
            final_speed: 6.0,
            final_travel: 8.0,
            route_end_distance: None,
            signal_stop: None,
            signal_projection_applied: false,
            safety_projection_applied: false,
        };

        motion
            .cap_to_route_end(8.0 + GEOMETRY_GAP_EPSILON / 2.0, 1.0)
            .unwrap();

        assert_eq!(motion.candidate_travel, 8.0);
        assert_eq!(motion.final_travel(), 8.0);
        assert_eq!(motion.final_speed(), 6.0);
        assert!(motion.reaches_route_end());
    }
}
