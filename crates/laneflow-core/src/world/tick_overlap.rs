use super::*;

impl CoreWorld {

    pub(super) fn validate_candidate_overlap(
        &mut self,
        route: RouteHandle,
        candidate_id: &str,
        candidate: &NormalizedVehicleInput,
    ) -> Result<(), CoreError> {
        let Some(overlap) = self.find_candidate_overlap(None, route, candidate) else {
            return Ok(());
        };
        Err(self.candidate_overlap_error(candidate_id, overlap))
    }

    pub(super) fn validate_candidate_overlap_excluding(
        &mut self,
        excluded: VehicleHandle,
        route: RouteHandle,
        candidate: &NormalizedVehicleInput,
    ) -> Result<(), CoreError> {
        let Some(overlap) = self.find_candidate_overlap(Some(excluded), route, candidate) else {
            return Ok(());
        };
        let candidate_id = self
            .vehicle_external_id(excluded)
            .expect("excluded vehicle must be live");
        Err(self.candidate_overlap_error(candidate_id, overlap))
    }

    pub(super) fn find_candidate_overlap_excluding(
        &mut self,
        excluded: VehicleHandle,
        route: RouteHandle,
        candidate: &NormalizedVehicleInput,
    ) -> Option<CandidateVehicleOverlap> {
        self.find_candidate_overlap(Some(excluded), route, candidate)
    }

    pub(super) fn find_candidate_overlap(
        &mut self,
        excluded: Option<VehicleHandle>,
        route: RouteHandle,
        candidate: &NormalizedVehicleInput,
    ) -> Option<CandidateVehicleOverlap> {
        if matches!(
            candidate.status,
            VehicleStatus::Completed | VehicleStatus::Parked
        ) {
            return None;
        }
        let candidate_length = self
            .vehicle_profile(candidate.profile)
            .expect("candidate profile must exist")
            .iidm()
            .length;
        let mut spatial = std::mem::take(&mut self.command_spatial_index);
        let route_edges = &self
            .route_slot(route)
            .expect("candidate route must exist")
            .edge_handles;
        let mut resolve_progress = |handle| {
            self.vehicle(handle)
                .expect("command spatial occupant must be live")
                .edge_progress
                .value()
        };
        spatial.gather_overlap_candidates(
            route_edges,
            candidate.route_edge_index,
            candidate.edge_progress.value(),
            candidate_length,
            self.vehicles.len(),
            &mut resolve_progress,
        );
        spatial.sort_candidates_by_key(|handle| {
            self.vehicles[handle.index()]
                .update_order_position
                .expect("command candidate must be live")
        });
        let result = self.find_candidate_overlap_for_handles(
            route,
            candidate,
            spatial
                .candidates()
                .iter()
                .copied()
                .filter(|handle| Some(*handle) != excluded),
        );
        self.command_spatial_index = spatial;
        result
    }

    pub(super) fn validate_parking_leave_followers(
        &mut self,
        leaving_vehicle: VehicleHandle,
        space: crate::ParkingSpaceHandle,
        route: RouteHandle,
        route_edge_index: usize,
        candidate: &NormalizedVehicleInput,
    ) -> Result<(), CoreError> {
        let candidate_edge = self.routes[route.index()].edge_handles[route_edge_index];
        let candidate_length = self
            .vehicle_profile(candidate.profile)
            .expect("candidate profile must exist")
            .iidm()
            .length;
        let emergency_horizon = parking_emergency_travel(
            "leave_global_emergency_horizon",
            leaving_vehicle,
            space,
            self.command_spatial_index.max_vehicle_speed(),
            self.command_spatial_index.min_emergency_deceleration(),
            self.fixed_delta_time_ms as f64 / 1_000.0,
        )?;
        let reverse_horizon = if candidate_length > f64::MAX - emergency_horizon {
            f64::MAX
        } else {
            candidate_length + emergency_horizon
        };
        let mut spatial = std::mem::take(&mut self.command_spatial_index);
        let mut resolve_progress = |handle| {
            self.vehicle(handle)
                .expect("command spatial occupant must be live")
                .edge_progress
                .value()
        };
        spatial.gather_direct_follower_candidates(
            candidate_edge,
            candidate.edge_progress.value(),
            reverse_horizon,
            self.vehicles.len(),
            &mut resolve_progress,
        );
        spatial.sort_candidates_by_key(|handle| {
            self.vehicles[handle.index()]
                .update_order_position
                .expect("command candidate must be live")
        });
        let result = self.validate_parking_leave_followers_for_handles(
            leaving_vehicle,
            space,
            candidate_edge,
            candidate.edge_progress.value(),
            candidate_length,
            spatial.candidates(),
        );
        self.command_spatial_index = spatial;
        result
    }

    pub(super) fn validate_parking_leave_followers_for_handles(
        &self,
        leaving_vehicle: VehicleHandle,
        space: crate::ParkingSpaceHandle,
        candidate_edge: EdgeHandle,
        candidate_progress: f64,
        candidate_length: f64,
        handles: &[VehicleHandle],
    ) -> Result<(), CoreError> {
        for follower_handle in handles.iter().copied() {
            if follower_handle == leaving_vehicle {
                continue;
            }
            let follower = self
                .vehicle(follower_handle)
                .expect("leave follower candidate must be live");
            if follower.status != VehicleStatus::Active {
                continue;
            }
            let follower_profile = self
                .vehicle_profile(follower.profile)
                .expect("live follower profile must exist")
                .iidm();
            let emergency_travel = parking_emergency_travel(
                "leave_follower_emergency_travel",
                follower_handle,
                space,
                follower.current_speed.value(),
                follower_profile.emergency_deceleration,
                self.fixed_delta_time_ms as f64 / 1_000.0,
            )?;
            let follower_horizon = if candidate_length > f64::MAX - emergency_travel {
                f64::MAX
            } else {
                candidate_length + emergency_travel
            };
            let Some(candidate_front_distance) = self.route_front_distance_within(
                follower.route,
                follower.route_edge_index,
                follower.edge_progress.value(),
                candidate_edge,
                candidate_progress,
                follower_horizon,
            ) else {
                continue;
            };

            let has_intervening_leader = handles.iter().copied().any(|other_handle| {
                if other_handle == follower_handle || other_handle == leaving_vehicle {
                    return false;
                }
                let Some(other) = self.vehicle(other_handle) else {
                    return false;
                };
                if !matches!(other.status, VehicleStatus::Active | VehicleStatus::Stopped) {
                    return false;
                }
                let other_edge = self.vehicle_edge(other);
                self.route_front_distance_within(
                    follower.route,
                    follower.route_edge_index,
                    follower.edge_progress.value(),
                    other_edge,
                    other.edge_progress.value(),
                    candidate_front_distance,
                )
                .is_some_and(|distance| {
                    distance + LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS < candidate_front_distance
                })
            });
            if has_intervening_leader {
                continue;
            }

            let bumper_gap = candidate_front_distance - candidate_length;
            if bumper_gap + PHYSICAL_GAP_TOLERANCE_METERS < emergency_travel {
                return Err(CoreError::ParkingLeaveUnsafeFollower {
                    vehicle: leaving_vehicle,
                    space,
                    follower: follower_handle,
                });
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn validate_parking_leave_followers_full_scan(
        &self,
        leaving_vehicle: VehicleHandle,
        space: crate::ParkingSpaceHandle,
        route: RouteHandle,
        route_edge_index: usize,
        candidate: &NormalizedVehicleInput,
    ) -> Result<(), CoreError> {
        let candidate_edge = self.routes[route.index()].edge_handles[route_edge_index];
        let candidate_length = self
            .vehicle_profile(candidate.profile)
            .expect("candidate profile must exist")
            .iidm()
            .length;
        let handles = self.vehicle_update_order.iter().collect::<Vec<_>>();
        self.validate_parking_leave_followers_for_handles(
            leaving_vehicle,
            space,
            candidate_edge,
            candidate.edge_progress.value(),
            candidate_length,
            &handles,
        )
    }

    #[cfg(test)]
    pub(super) fn validate_candidate_overlap_full_scan(
        &self,
        route: RouteHandle,
        candidate_id: &str,
        candidate: &NormalizedVehicleInput,
    ) -> Result<(), CoreError> {
        self.validate_candidate_overlap_for_handles(
            route,
            candidate_id,
            candidate,
            self.vehicle_update_order.iter(),
        )
    }

    #[cfg(test)]
    pub(super) fn validate_candidate_overlap_for_handles<I>(
        &self,
        route: RouteHandle,
        candidate_id: &str,
        candidate: &NormalizedVehicleInput,
        existing_handles: I,
    ) -> Result<(), CoreError>
    where
        I: IntoIterator<Item = VehicleHandle>,
    {
        let Some(overlap) =
            self.find_candidate_overlap_for_handles(route, candidate, existing_handles)
        else {
            return Ok(());
        };
        Err(self.candidate_overlap_error(candidate_id, overlap))
    }

    pub(super) fn find_candidate_overlap_for_handles<I>(
        &self,
        route: RouteHandle,
        candidate: &NormalizedVehicleInput,
        existing_handles: I,
    ) -> Option<CandidateVehicleOverlap>
    where
        I: IntoIterator<Item = VehicleHandle>,
    {
        if matches!(
            candidate.status,
            VehicleStatus::Completed | VehicleStatus::Parked
        ) {
            return None;
        }

        let candidate_edge = self
            .route_slot(route)
            .expect("candidate route must exist")
            .edge_handles[candidate.route_edge_index];
        let candidate_length = self
            .vehicle_profile(candidate.profile)
            .expect("candidate profile must exist")
            .iidm()
            .length;

        for handle in existing_handles {
            let existing = self
                .vehicle(handle)
                .expect("command spatial candidate must be live");
            if matches!(
                existing.status,
                VehicleStatus::Completed | VehicleStatus::Parked
            ) {
                continue;
            }
            let existing_edge = self.vehicle_edge(existing);
            let existing_length = self
                .vehicle_profile(existing.profile)
                .expect("existing profile must exist")
                .iidm()
                .length;
            if let Some(front_distance) = self.route_front_distance_within(
                route,
                candidate.route_edge_index,
                candidate.edge_progress.value(),
                existing_edge,
                existing.edge_progress.value(),
                existing_length,
            ) {
                let bumper_gap = front_distance - existing_length;
                if physical_gap_is_overlap(bumper_gap) {
                    return Some(CandidateVehicleOverlap {
                        blocker: handle,
                        blocker_position: VehicleReplaceBlockerPosition::Ahead,
                        bumper_gap,
                    });
                }
            }

            if let Some(front_distance) = self.route_front_distance_within(
                existing.route,
                existing.route_edge_index,
                existing.edge_progress.value(),
                candidate_edge,
                candidate.edge_progress.value(),
                candidate_length,
            ) {
                let bumper_gap = front_distance - candidate_length;
                if physical_gap_is_overlap(bumper_gap) {
                    return Some(CandidateVehicleOverlap {
                        blocker: handle,
                        blocker_position: VehicleReplaceBlockerPosition::Behind,
                        bumper_gap,
                    });
                }
            }
        }

        None
    }

    pub(super) fn candidate_overlap_error(
        &self,
        candidate_id: &str,
        overlap: CandidateVehicleOverlap,
    ) -> CoreError {
        let blocker_id = self
            .vehicle_external_id(overlap.blocker)
            .expect("overlap blocker external ID must exist");
        let (follower_id, leader_id) = match overlap.blocker_position {
            VehicleReplaceBlockerPosition::Ahead => {
                (candidate_id.to_owned(), blocker_id.to_owned())
            }
            VehicleReplaceBlockerPosition::Behind => {
                (blocker_id.to_owned(), candidate_id.to_owned())
            }
        };
        CoreError::VehiclePhysicalOverlap {
            follower_id,
            leader_id,
            bumper_gap: overlap.bumper_gap,
        }
    }

    pub(super) fn route_front_distance_within(
        &self,
        route: RouteHandle,
        route_edge_index: usize,
        front_progress: f64,
        target_edge: EdgeHandle,
        target_front_progress: f64,
        max_front_distance: f64,
    ) -> Option<f64> {
        let route_handle = route;
        let route = self.route_slot(route_handle).expect("route must exist");
        let current_edge = route.edge_handles[route_edge_index];
        let target_occurrence =
            if current_edge == target_edge && target_front_progress >= front_progress {
                route_edge_index
            } else {
                route
                    .edge_handles
                    .iter()
                    .copied()
                    .enumerate()
                    .skip(route_edge_index + 1)
                    .find_map(|(index, edge)| (edge == target_edge).then_some(index))?
            };

        match self.route_distance_indices[route_handle.index()].distance_within(
            route_edge_index,
            front_progress,
            target_occurrence,
            target_front_progress,
            max_front_distance,
        ) {
            RouteDistanceQuery::Within(distance) => Some(distance),
            RouteDistanceQuery::Passed | RouteDistanceQuery::BeyondHorizon => None,
        }
    }

    pub(super) fn validate_initial_vehicle_overlaps(&mut self) -> Result<(), CoreError> {
        let mut scratch = std::mem::take(&mut self.occupancy_scratch);
        self.build_occupancy(&mut scratch);
        let result = self.validate_occupancy_overlaps(&scratch);
        self.occupancy_scratch = scratch;
        result
    }

    pub(super) fn validate_occupancy_overlaps(&self, scratch: &OccupancyScratch) -> Result<(), CoreError> {
        for edge_index in 0..self.lane_graph.edges().len() {
            for pair in scratch.edge(EdgeHandle::new(edge_index)).windows(2) {
                let follower = pair[0];
                let leader = pair[1];
                let bumper_gap =
                    leader.front_progress - follower.front_progress - leader.vehicle_length;
                if physical_gap_is_overlap(bumper_gap) {
                    return Err(self.vehicle_overlap_error(
                        follower.vehicle,
                        leader.vehicle,
                        bumper_gap,
                    ));
                }
            }
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

            if let Some(observation) = self.find_leader(scratch, vehicle, 0.0)?
                && physical_gap_is_overlap(observation.bumper_gap)
            {
                return Err(self.vehicle_overlap_error(
                    handle,
                    observation.leader,
                    observation.bumper_gap,
                ));
            }
        }

        Ok(())
    }

    pub(super) fn vehicle_overlap_error(
        &self,
        follower: VehicleHandle,
        leader: VehicleHandle,
        bumper_gap: f64,
    ) -> CoreError {
        CoreError::VehiclePhysicalOverlap {
            follower_id: self
                .vehicle_external_id(follower)
                .expect("occupant vehicle ID must exist")
                .to_owned(),
            leader_id: self
                .vehicle_external_id(leader)
                .expect("occupant vehicle ID must exist")
                .to_owned(),
            bumper_gap,
        }
    }
}
