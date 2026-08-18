use super::*;

impl CoreWorld {
    pub(super) fn rebuild_occupancy_and_leaders(&mut self) -> Result<(), TickInvariantError> {
        let mut scratch = std::mem::take(&mut self.occupancy_scratch);
        let result = (|| {
            self.build_occupancy(&mut scratch);

            if scratch.occupant_count() <= 1 {
                return Ok(());
            }

            for handle in self.vehicle_update_order.iter() {
                let Some(vehicle) = self.vehicle(handle) else {
                    continue;
                };
                if !matches!(
                    vehicle.status,
                    VehicleStatus::Active | VehicleStatus::Stopped
                ) {
                    continue;
                }

                let horizon = self.leader_horizon(vehicle)?;
                let leader = self.find_leader(&scratch, vehicle, horizon)?;
                scratch.set_leader(handle, leader);
            }

            Ok(())
        })();
        self.occupancy_scratch = scratch;
        result
    }

    pub(super) fn rebuild_longitudinal_motions<P: StepProbe>(
        &mut self,
        probe: &mut P,
    ) -> Result<(), TickInvariantError> {
        let longitudinal_started = P::ENABLED.then(std::time::Instant::now);
        let result = if self.parking_runtime.reserved_count() == 0 {
            self.rebuild_longitudinal_motions_for_parking::<false, P>(probe)
        } else {
            self.rebuild_longitudinal_motions_for_parking::<true, P>(probe)
        };
        if let Some(started) = longitudinal_started {
            probe.note_longitudinal_duration(started.elapsed());
        }
        result
    }

    pub(super) fn rebuild_longitudinal_motions_for_parking<
        const PARKING_ACTIVE: bool,
        P: StepProbe,
    >(
        &mut self,
        probe: &mut P,
    ) -> Result<(), TickInvariantError> {
        let mut scratch = std::mem::take(&mut self.longitudinal_scratch);
        let result = (|| {
            let proposal_started = P::ENABLED.then(std::time::Instant::now);
            scratch.begin(self.vehicles.len());
            let delta_time = self.fixed_delta_time_ms as f64 / 1_000.0;

            for (update_sequence, handle) in self.vehicle_update_order.iter().enumerate() {
                let Some(vehicle) = self.vehicle(handle) else {
                    continue;
                };
                let update_sequence = u64::try_from(update_sequence)
                    .expect("vehicle update sequence must fit in u64");

                match vehicle.status {
                    VehicleStatus::Completed | VehicleStatus::Parked => {
                        continue;
                    }
                    VehicleStatus::Stopped => {
                        scratch.set(LongitudinalMotion::stationary(handle, update_sequence));
                    }
                    VehicleStatus::Active => {
                        let profile = self
                            .vehicle_profile(vehicle.profile)
                            .expect("live vehicle profile must exist")
                            .iidm();
                        let signal_stop = if !self.signal_state.has_restrictive_group() {
                            None
                        } else {
                            let horizon = self.signal_stop_horizon(vehicle, profile)?;
                            self.nearest_denied_signal_stop(vehicle, horizon)?
                        };
                        let leader = self.occupancy_scratch.leader(handle).map(|observation| {
                            let leader = self
                                .vehicle(observation.leader)
                                .expect("occupancy leader must be live");
                            let leader_profile = self
                                .vehicle_profile(leader.profile)
                                .expect("leader profile must exist")
                                .iidm();
                            LeaderKinematics {
                                observation,
                                current_speed: leader.current_speed.value(),
                                emergency_deceleration: leader_profile.emergency_deceleration,
                            }
                        });
                        let mut motion = compute_motion(
                            handle,
                            update_sequence,
                            vehicle.current_speed.value(),
                            profile,
                            self.lane_graph
                                .edge_speed_limit(self.vehicle_edge(vehicle))
                                .expect("live vehicle edge must exist")
                                .value(),
                            leader,
                            delta_time,
                        )?;
                        let speed_limit_constrained = self.apply_speed_limit_constraints(
                            vehicle,
                            profile,
                            &mut motion,
                            delta_time,
                        )?;
                        let route_end_distance =
                            self.route_end_distance_within(vehicle, motion.final_travel());
                        let parking_stop = if PARKING_ACTIVE {
                            self.parking_stop_within(vehicle, profile)?
                        } else {
                            None
                        };
                        if route_end_distance.is_none()
                            && signal_stop.is_none()
                            && parking_stop.is_none()
                            && !speed_limit_constrained
                        {
                            scratch.set(motion);
                            continue;
                        }
                        if let Some(constraint) = parking_stop {
                            scratch.push_parking_stop(handle, constraint);
                        }
                        motion.apply_spatial_stops(
                            route_end_distance,
                            signal_stop,
                            parking_stop,
                            (signal_stop.is_some() || parking_stop.is_some()).then_some(profile),
                            delta_time,
                        )?;
                        scratch.set(motion);
                    }
                }
            }

            let proposal_duration = proposal_started.map(|started| started.elapsed());
            let projection_started = P::ENABLED.then(std::time::Instant::now);
            let projection_result = scratch.project(self.vehicle_update_order.iter(), delta_time);
            if let (Some(proposal), Some(projection_started)) =
                (proposal_duration, projection_started)
            {
                probe.note_longitudinal_breakdown(proposal, projection_started.elapsed());
            }
            projection_result
        })();
        self.longitudinal_scratch = scratch;
        result
    }

    pub(super) fn apply_speed_limit_constraints(
        &self,
        vehicle: &VehicleState,
        profile: crate::IidmProfileSpec,
        motion: &mut LongitudinalMotion,
        delta_time: f64,
    ) -> Result<bool, TickInvariantError> {
        let route = self
            .route_slot(vehicle.route)
            .expect("live vehicle route must exist");
        let first = route.speed_limit_transitions.partition_point(|transition| {
            transition.from_route_edge_index < vehicle.route_edge_index
        });
        if first == route.speed_limit_transitions.len() {
            return Ok(false);
        }

        let horizon = self.speed_limit_horizon(vehicle, profile)?;
        let mut constrained = false;
        for transition in &route.speed_limit_transitions[first..] {
            let from_edge = route.edge_handles[transition.from_route_edge_index];
            let boundary_progress = self
                .lane_graph
                .edge_length(from_edge)
                .expect("normalized route edge must exist")
                .value();
            let route_distance = match self.route_distance_indices[vehicle.route.index()]
                .distance_within(
                    vehicle.route_edge_index,
                    vehicle.edge_progress.value(),
                    transition.from_route_edge_index,
                    boundary_progress,
                    horizon,
                ) {
                RouteDistanceQuery::Within(distance) => distance,
                RouteDistanceQuery::BeyondHorizon => break,
                RouteDistanceQuery::Passed => continue,
            };
            constrained |= motion.apply_speed_limit_constraint(
                SpeedLimitConstraint {
                    route: vehicle.route,
                    from_route_edge_index: transition.from_route_edge_index,
                    to_route_edge_index: transition.from_route_edge_index + 1,
                    from_edge,
                    to_edge: transition.to_edge,
                    route_distance,
                    target_speed: transition.target_speed,
                },
                profile,
                delta_time,
            )?;
        }
        Ok(constrained)
    }

    pub(super) fn speed_limit_horizon(
        &self,
        vehicle: &VehicleState,
        profile: crate::IidmProfileSpec,
    ) -> Result<f64, TickInvariantError> {
        let speed = vehicle.current_speed.value();
        let delta_time = self.fixed_delta_time_ms as f64 / 1_000.0;
        let upper_speed = Self::finite_speed_limit_value(
            vehicle.handle,
            "speed_limit_upper_speed",
            speed + profile.max_acceleration * delta_time,
        )?;
        let travel_upper = Self::finite_speed_limit_value(
            vehicle.handle,
            "speed_limit_travel_upper",
            Self::half_product(speed, delta_time) + Self::half_product(upper_speed, delta_time),
        )?;
        let braking_distance = Self::finite_speed_limit_value(
            vehicle.handle,
            "speed_limit_comfortable_braking_distance",
            Self::braking_distance(upper_speed, profile.comfortable_deceleration),
        )?;
        Self::finite_speed_limit_value(
            vehicle.handle,
            "speed_limit_comfortable_horizon",
            travel_upper + braking_distance,
        )
    }

    pub(super) fn parking_stop_within(
        &self,
        vehicle: &VehicleState,
        profile: crate::IidmProfileSpec,
    ) -> Result<Option<ParkingStopConstraint>, TickInvariantError> {
        let Some(RuntimeVehicleParkingBinding::Reserved {
            vehicle: bound_vehicle,
            space,
            target,
        }) = self.parking_runtime.vehicle_binding(vehicle.handle)
        else {
            return Ok(None);
        };
        if bound_vehicle != vehicle.handle
            || self.parking_runtime.space_state(space)
                != Some(ParkingSpaceState::Reserved {
                    vehicle: vehicle.handle,
                })
        {
            return Err(TickInvariantError::ParkingBindingInvariantViolation {
                stage: "step_parking_target_pair",
                vehicle: Some(vehicle.handle),
                space: Some(space),
            });
        }
        let Some(target) = target else {
            return Ok(None);
        };
        if target.route != vehicle.route {
            return Err(TickInvariantError::ParkingBindingInvariantViolation {
                stage: "step_parking_target_route",
                vehicle: Some(vehicle.handle),
                space: Some(space),
            });
        }
        let entry = self
            .parking
            .space_entry(space)
            .expect("normalized ParkingSpace must have entry");

        let horizon = self.parking_stop_horizon(vehicle, profile, space)?;
        match self.route_distance_indices[vehicle.route.index()].distance_within(
            vehicle.route_edge_index,
            vehicle.edge_progress.value(),
            target.route_edge_index,
            entry.progress(),
            horizon,
        ) {
            RouteDistanceQuery::Within(route_distance) => Ok(Some(ParkingStopConstraint {
                space,
                route: vehicle.route,
                route_edge_index: target.route_edge_index,
                entry_progress: entry.progress(),
                route_distance,
            })),
            RouteDistanceQuery::BeyondHorizon => Ok(None),
            RouteDistanceQuery::Passed => {
                Err(TickInvariantError::ParkingBindingInvariantViolation {
                    stage: "step_parking_target_passed",
                    vehicle: Some(vehicle.handle),
                    space: Some(space),
                })
            }
        }
    }

    pub(super) fn route_end_distance_within(
        &self,
        vehicle: &VehicleState,
        max_travel: f64,
    ) -> Option<f64> {
        let horizon = if max_travel <= f64::MAX - LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS {
            max_travel + LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS
        } else {
            f64::MAX
        };
        let route = self
            .route_slot(vehicle.route)
            .expect("live vehicle route must exist");
        let current_edge_length = self
            .lane_graph
            .edge_length(route.edge_handles[vehicle.route_edge_index])
            .expect("route edge must exist")
            .value();
        let remaining_on_edge = current_edge_length - vehicle.edge_progress.value();
        if remaining_on_edge > horizon {
            return None;
        }
        let Some(next_edge) = route.edge_handles.get(vehicle.route_edge_index + 1) else {
            return Some(remaining_on_edge.max(0.0));
        };
        let next_edge_length = self
            .lane_graph
            .edge_length(*next_edge)
            .expect("route edge must exist")
            .value();
        if next_edge_length > horizon - remaining_on_edge {
            return None;
        }
        match self.route_distance_indices[vehicle.route.index()].distance_to_end_within(
            vehicle.route_edge_index,
            vehicle.edge_progress.value(),
            horizon,
        ) {
            RouteDistanceQuery::Within(distance) => Some(distance),
            RouteDistanceQuery::Passed | RouteDistanceQuery::BeyondHorizon => None,
        }
    }

    pub(super) fn build_occupancy(&self, scratch: &mut OccupancyScratch) {
        scratch.begin(self.lane_graph.edges().len(), self.vehicles.len());

        for handle in self.vehicle_update_order.iter() {
            let Some(vehicle) = self.vehicle(handle) else {
                continue;
            };
            if !matches!(
                vehicle.status,
                VehicleStatus::Active | VehicleStatus::Stopped
            ) {
                continue;
            }

            let edge = self.vehicle_edge(vehicle);
            let vehicle_length = self
                .vehicle_profile(vehicle.profile)
                .expect("live vehicle profile must exist")
                .iidm()
                .length;
            scratch.count(edge, vehicle_length);
        }

        scratch.allocate_occupants();
        for (update_sequence, handle) in self.vehicle_update_order.iter().enumerate() {
            let Some(vehicle) = self.vehicle(handle) else {
                continue;
            };
            if !matches!(
                vehicle.status,
                VehicleStatus::Active | VehicleStatus::Stopped
            ) {
                continue;
            }

            let edge = self.vehicle_edge(vehicle);
            let vehicle_length = self
                .vehicle_profile(vehicle.profile)
                .expect("live vehicle profile must exist")
                .iidm()
                .length;
            scratch.insert(
                edge,
                Occupant {
                    vehicle: handle,
                    front_progress: vehicle.edge_progress.value(),
                    vehicle_length,
                    update_sequence: u64::try_from(update_sequence)
                        .expect("vehicle update sequence must fit in u64"),
                },
            );
        }
        scratch.sort_edges();
    }

    pub(super) fn vehicle_edge(&self, vehicle: &VehicleState) -> EdgeHandle {
        self.route_slot(vehicle.route)
            .expect("live vehicle route must exist")
            .edge_handles[vehicle.route_edge_index]
    }

    pub(super) fn leader_horizon(&self, vehicle: &VehicleState) -> Result<f64, TickInvariantError> {
        let profile = self
            .vehicle_profile(vehicle.profile)
            .expect("live vehicle profile must exist")
            .iidm();
        let speed = vehicle.current_speed.value();
        let delta_time = self.fixed_delta_time_ms as f64 / 1_000.0;
        let upper_speed = speed + profile.max_acceleration * delta_time;
        Self::finite_leader_value(vehicle.handle, "upper_speed", upper_speed)?;
        let travel_upper =
            Self::half_product(speed, delta_time) + Self::half_product(upper_speed, delta_time);
        Self::finite_leader_value(vehicle.handle, "travel_upper", travel_upper)?;
        let braking_distance = Self::braking_distance(upper_speed, profile.emergency_deceleration);
        Self::finite_leader_value(vehicle.handle, "braking_distance", braking_distance)?;
        let hard_horizon = travel_upper + braking_distance;
        Self::finite_leader_value(vehicle.handle, "hard_horizon", hard_horizon)?;
        let comfort_horizon = profile.min_gap + speed * profile.time_headway;
        Self::finite_leader_value(vehicle.handle, "comfort_horizon", comfort_horizon)?;
        let minimum_gap_with_tick_travel = profile.min_gap + travel_upper;
        Self::finite_leader_value(
            vehicle.handle,
            "minimum_gap_with_tick_travel",
            minimum_gap_with_tick_travel,
        )?;
        let minimum_gap_horizon = minimum_gap_with_tick_travel + MINIMUM_GAP_TOLERANCE_METERS;
        Self::finite_leader_value(vehicle.handle, "minimum_gap_horizon", minimum_gap_horizon)?;

        Ok(hard_horizon.max(comfort_horizon).max(minimum_gap_horizon))
    }

    pub(super) fn signal_stop_horizon(
        &self,
        vehicle: &VehicleState,
        profile: crate::IidmProfileSpec,
    ) -> Result<f64, TickInvariantError> {
        let speed = vehicle.current_speed.value();
        let delta_time = self.fixed_delta_time_ms as f64 / 1_000.0;
        let upper_speed = Self::finite_signal_stop_value(
            vehicle.handle,
            "signal_upper_speed",
            speed + profile.max_acceleration * delta_time,
        )?;
        let travel_upper = Self::finite_signal_stop_value(
            vehicle.handle,
            "signal_travel_upper",
            Self::half_product(speed, delta_time) + Self::half_product(upper_speed, delta_time),
        )?;
        let comfortable_braking_distance = Self::finite_signal_stop_value(
            vehicle.handle,
            "signal_comfortable_braking_distance",
            Self::braking_distance(upper_speed, profile.comfortable_deceleration),
        )?;
        let comfortable_horizon = Self::finite_signal_stop_value(
            vehicle.handle,
            "signal_comfortable_horizon",
            travel_upper + comfortable_braking_distance,
        )?;
        Ok(comfortable_horizon.max(self.leader_horizon(vehicle)?))
    }

    pub(super) fn parking_stop_horizon(
        &self,
        vehicle: &VehicleState,
        profile: crate::IidmProfileSpec,
        space: crate::ParkingSpaceHandle,
    ) -> Result<f64, TickInvariantError> {
        let finite = |stage, value: f64| {
            if value.is_finite() {
                Ok(value)
            } else {
                Err(TickInvariantError::NonFiniteParkingComputation {
                    stage,
                    vehicle: vehicle.handle,
                    space,
                    value,
                })
            }
        };
        let speed = vehicle.current_speed.value();
        let delta_time = self.fixed_delta_time_ms as f64 / 1_000.0;
        let upper_speed = finite(
            "parking_upper_speed",
            speed + profile.max_acceleration * delta_time,
        )?;
        let travel_upper = finite(
            "parking_travel_upper",
            Self::half_product(speed, delta_time) + Self::half_product(upper_speed, delta_time),
        )?;
        let braking_distance = finite(
            "parking_comfortable_braking_distance",
            Self::braking_distance(upper_speed, profile.comfortable_deceleration),
        )?;
        finite(
            "parking_comfortable_horizon",
            travel_upper + braking_distance,
        )
    }

    pub(super) fn nearest_denied_signal_stop(
        &self,
        vehicle: &VehicleState,
        horizon: f64,
    ) -> Result<Option<SignalStopConstraint>, TickInvariantError> {
        let route = self
            .route_slot(vehicle.route)
            .expect("live vehicle route must exist");
        let mut search_edge_index = vehicle.route_edge_index;
        let mut distance = 0.0;
        let mut first = true;

        while let Some(next) = route
            .next_controlled_transition
            .get(search_edge_index)
            .copied()
            .flatten()
        {
            let progress = if first {
                vehicle.edge_progress.value()
            } else {
                0.0
            };
            let BoundedDistance::Finite(distance_from_edge_start) = next.distance_from_edge_start
            else {
                break;
            };
            let segment_distance = (distance_from_edge_start - progress).max(0.0);
            if segment_distance > horizon - distance {
                break;
            }
            distance += segment_distance;
            if distance > horizon + LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS {
                break;
            }

            let gate = self
                .signals
                .maneuver_gate_state_by_handle(&self.signal_state, next.gate)
                .expect("normalized route Gate must have committed state");
            if let ManeuverGateSignalState::Controlled {
                group,
                aspect,
                permission: SignalLayerPermission::DenyAndStop,
            } = gate.signal()
            {
                return Ok(Some(SignalStopConstraint {
                    route_distance: distance,
                    gate: gate.gate(),
                    stop_line: gate.stop_line(),
                    group,
                    aspect,
                    from_route_edge_index: next.from_route_edge_index,
                    to_route_edge_index: next.from_route_edge_index + 1,
                }));
            }

            search_edge_index = next.from_route_edge_index + 1;
            if search_edge_index >= route.edge_handles.len() {
                break;
            }
            first = false;
        }

        Ok(None)
    }

    pub(super) fn find_leader(
        &self,
        scratch: &OccupancyScratch,
        follower: &VehicleState,
        bumper_gap_horizon: f64,
    ) -> Result<Option<LeaderObservation>, TickInvariantError> {
        Self::finite_leader_value(follower.handle, "bumper_gap_horizon", bumper_gap_horizon)?;
        let front_horizon = bumper_gap_horizon + scratch.max_vehicle_length();
        Self::finite_leader_value(follower.handle, "front_horizon", front_horizon)?;

        let route = self
            .route_slot(follower.route)
            .expect("live vehicle route must exist");
        let current_edge = route.edge_handles[follower.route_edge_index];
        let current_occupants = scratch.edge(current_edge);
        // 相同 front progress 是非法物理重叠；update sequence 只形成确定排序，不能把 tie 合法化为 leader。
        let first_strictly_ahead = current_occupants
            .partition_point(|occupant| occupant.front_progress <= follower.edge_progress.value());
        for occupant in &current_occupants[first_strictly_ahead..] {
            if occupant.vehicle == follower.handle {
                continue;
            }
            let front_distance = occupant.front_progress - follower.edge_progress.value();
            let bumper_gap = normalize_physical_gap(front_distance - occupant.vehicle_length);
            if bumper_gap <= bumper_gap_horizon {
                return Ok(Some(LeaderObservation {
                    leader: occupant.vehicle,
                    bumper_gap,
                }));
            }
            break;
        }

        let current_edge_length = self
            .lane_graph
            .edge_length(current_edge)
            .expect("route edge must exist")
            .value();
        let mut distance_to_edge_start = current_edge_length - follower.edge_progress.value();

        for edge in route
            .edge_handles
            .iter()
            .copied()
            .skip(follower.route_edge_index + 1)
        {
            Self::finite_leader_value(
                follower.handle,
                "distance_to_edge_start",
                distance_to_edge_start,
            )?;
            if distance_to_edge_start > front_horizon {
                break;
            }

            for occupant in scratch.edge(edge) {
                if occupant.vehicle == follower.handle {
                    continue;
                }
                let remaining = front_horizon - distance_to_edge_start;
                if occupant.front_progress > remaining {
                    break;
                }
                let front_distance = distance_to_edge_start + occupant.front_progress;
                let bumper_gap = normalize_physical_gap(front_distance - occupant.vehicle_length);
                if bumper_gap <= bumper_gap_horizon {
                    return Ok(Some(LeaderObservation {
                        leader: occupant.vehicle,
                        bumper_gap,
                    }));
                }
            }

            let edge_length = self
                .lane_graph
                .edge_length(edge)
                .expect("route edge must exist")
                .value();
            if edge_length > front_horizon - distance_to_edge_start {
                break;
            }
            distance_to_edge_start += edge_length;
        }

        Ok(None)
    }

    pub(super) fn finite_leader_value(
        vehicle: VehicleHandle,
        stage: &'static str,
        value: f64,
    ) -> Result<f64, TickInvariantError> {
        if !value.is_finite() {
            return Err(TickInvariantError::NonFiniteLeaderComputation {
                vehicle,
                stage,
                value,
            });
        }
        Ok(value)
    }

    pub(super) fn finite_signal_stop_value(
        vehicle: VehicleHandle,
        stage: &'static str,
        value: f64,
    ) -> Result<f64, TickInvariantError> {
        if !value.is_finite() {
            return Err(TickInvariantError::NonFiniteSignalStopComputation {
                vehicle,
                stage,
                value,
            });
        }
        Ok(if value == 0.0 { 0.0 } else { value })
    }

    pub(super) fn finite_speed_limit_value(
        vehicle: VehicleHandle,
        stage: &'static str,
        value: f64,
    ) -> Result<f64, TickInvariantError> {
        if !value.is_finite() {
            return Err(TickInvariantError::NonFiniteSpeedLimitComputation {
                vehicle,
                stage,
                value,
            });
        }
        Ok(if value == 0.0 { 0.0 } else { value })
    }

    pub(super) fn braking_distance(speed: f64, deceleration: f64) -> f64 {
        if speed == 0.0 {
            return 0.0;
        }
        if deceleration > f64::MAX / 2.0 {
            return speed / deceleration * (0.5 * speed);
        }

        let denominator = 2.0 * deceleration;
        if speed < 1.0 {
            speed / (denominator / speed)
        } else {
            speed / denominator * speed
        }
    }

    pub(super) fn half_product(left: f64, right: f64) -> f64 {
        if left >= right {
            (0.5 * left) * right
        } else {
            left * (0.5 * right)
        }
    }
}
