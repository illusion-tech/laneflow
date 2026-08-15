use super::*;

impl CoreWorld {
    /// 为 live Active vehicle 预订 caller-selected ParkingSpace。
    pub fn reserve_parking_space(
        &mut self,
        vehicle: VehicleHandle,
        space: crate::ParkingSpaceHandle,
    ) -> Result<ParkingReservationRecord, CoreError> {
        let vehicle_state = self
            .vehicle(vehicle)
            .ok_or(CoreError::UnknownVehicleHandle { vehicle })?;
        let status = vehicle_state.status;
        if self.parking.space(space).is_none() {
            return Err(CoreError::UnknownParkingSpaceHandle { space });
        }

        let binding = self.parking_runtime.vehicle_binding(vehicle);
        let space_state = self
            .parking_runtime
            .space_state(space)
            .expect("resolved ParkingSpace must have runtime state");
        if matches!(
            binding,
            Some(RuntimeVehicleParkingBinding::Reserved {
                space: current,
                ..
            }) if current == space
        ) && space_state == (ParkingSpaceState::Reserved { vehicle })
        {
            return Ok(ParkingReservationRecord {
                vehicle,
                space,
                effect: ParkingCommandEffect::AlreadySatisfied,
            });
        }
        if status != VehicleStatus::Active {
            return Err(CoreError::ParkingVehicleStatusMismatch {
                command: ParkingCommandKind::Reserve,
                vehicle,
                expected: VehicleStatus::Active,
                actual: status,
            });
        }
        if let Some(binding) = binding {
            return Err(CoreError::ParkingVehicleAlreadyBound {
                command: ParkingCommandKind::Reserve,
                vehicle,
                requested_space: space,
                current_space: binding.space(),
                binding: binding.kind(),
            });
        }
        match space_state {
            ParkingSpaceState::Vacant => {}
            ParkingSpaceState::Reserved {
                vehicle: current_vehicle,
            } => {
                return Err(CoreError::ParkingSpaceUnavailable {
                    command: ParkingCommandKind::Reserve,
                    space,
                    requested_vehicle: vehicle,
                    current_vehicle,
                    binding: ParkingBindingKind::Reserved,
                });
            }
            ParkingSpaceState::Occupied {
                vehicle: current_vehicle,
            } => {
                return Err(CoreError::ParkingSpaceUnavailable {
                    command: ParkingCommandKind::Reserve,
                    space,
                    requested_vehicle: vehicle,
                    current_vehicle,
                    binding: ParkingBindingKind::Occupied,
                });
            }
        }

        let target = self.first_reachable_parking_entry(
            vehicle_state.route,
            vehicle_state.route_edge_index,
            vehicle_state.edge_progress.value(),
            space,
        );
        self.parking_runtime
            .reserve(&self.parking, vehicle, space, target);
        Ok(ParkingReservationRecord {
            vehicle,
            space,
            effect: ParkingCommandEffect::Applied,
        })
    }

    /// 只取消 exact Reserved pair；不会强制释放其他 owner。
    pub fn cancel_parking_reservation(
        &mut self,
        vehicle: VehicleHandle,
        space: crate::ParkingSpaceHandle,
    ) -> Result<ParkingReservationCancellationRecord, CoreError> {
        self.vehicle(vehicle)
            .ok_or(CoreError::UnknownVehicleHandle { vehicle })?;
        if self.parking.space(space).is_none() {
            return Err(CoreError::UnknownParkingSpaceHandle { space });
        }
        let binding = self.parking_runtime.vehicle_binding(vehicle);
        let space_state = self
            .parking_runtime
            .space_state(space)
            .expect("resolved ParkingSpace must have runtime state");
        if matches!(
            binding,
            Some(RuntimeVehicleParkingBinding::Reserved {
                space: current,
                ..
            }) if current == space
        ) && space_state == (ParkingSpaceState::Reserved { vehicle })
        {
            self.parking_runtime.cancel(&self.parking, vehicle, space);
            return Ok(ParkingReservationCancellationRecord {
                vehicle,
                space,
                effect: ParkingCommandEffect::Applied,
            });
        }
        if binding.is_none() && space_state == ParkingSpaceState::Vacant {
            return Ok(ParkingReservationCancellationRecord {
                vehicle,
                space,
                effect: ParkingCommandEffect::AlreadySatisfied,
            });
        }
        Err(CoreError::ParkingReservationMismatch {
            command: ParkingCommandKind::CancelReservation,
            vehicle,
            space,
        })
    }

    /// 把 exact Arrived reservation 原子提交为 Occupied + Parked。
    pub fn commit_parking(
        &mut self,
        vehicle: VehicleHandle,
        space: crate::ParkingSpaceHandle,
    ) -> Result<ParkingCommitRecord, CoreError> {
        let state = self
            .vehicle(vehicle)
            .ok_or(CoreError::UnknownVehicleHandle { vehicle })?;
        let status = state.status;
        if self.parking.space(space).is_none() {
            return Err(CoreError::UnknownParkingSpaceHandle { space });
        }
        let binding = self.parking_runtime.vehicle_binding(vehicle);
        let space_state = self
            .parking_runtime
            .space_state(space)
            .expect("resolved ParkingSpace must have runtime state");
        if status == VehicleStatus::Parked
            && matches!(
                binding,
                Some(RuntimeVehicleParkingBinding::Occupied {
                    space: current,
                    ..
                }) if current == space
            )
            && space_state == (ParkingSpaceState::Occupied { vehicle })
        {
            return Ok(ParkingCommitRecord {
                vehicle,
                space,
                effect: ParkingCommandEffect::AlreadySatisfied,
            });
        }
        if status != VehicleStatus::Active {
            return Err(CoreError::ParkingVehicleStatusMismatch {
                command: ParkingCommandKind::Commit,
                vehicle,
                expected: VehicleStatus::Active,
                actual: status,
            });
        }
        let target = match binding {
            Some(RuntimeVehicleParkingBinding::Reserved {
                space: current,
                target,
                ..
            }) if current == space && space_state == (ParkingSpaceState::Reserved { vehicle }) => {
                target
            }
            _ => {
                return Err(CoreError::ParkingReservationMismatch {
                    command: ParkingCommandKind::Commit,
                    vehicle,
                    space,
                });
            }
        };
        if !self.parking_arrived(vehicle, space, target) {
            return Err(CoreError::ParkingVehicleNotArrived { vehicle, space });
        }

        let edge = self.vehicle_edge(state);
        let removed_speed = state.current_speed.value();
        let occupant = CommandOccupant {
            vehicle,
            front_progress: state.edge_progress.value(),
        };
        let mut spatial = std::mem::take(&mut self.command_spatial_index);
        let vehicles = &self.vehicles;
        spatial.prepare_speed_removal(
            removed_speed,
            vehicles.iter().filter_map(|slot| {
                let state = slot.state.as_ref()?;
                (state.status == VehicleStatus::Active)
                    .then_some((state.handle, state.current_speed.value()))
            }),
        );
        let mut resolve_progress = |candidate: VehicleHandle| {
            vehicles[candidate.index()]
                .state
                .as_ref()
                .expect("command spatial occupant must be live")
                .edge_progress
                .value()
        };
        spatial.remove(edge, occupant, &mut resolve_progress);
        self.command_spatial_index = spatial;

        let entry = self
            .parking
            .space_entry(space)
            .expect("resolved ParkingSpace must have entry");
        let state = self.vehicles[vehicle.index()]
            .state
            .as_mut()
            .expect("resolved vehicle must remain live");
        state.edge_progress = EdgeProgress::try_new(entry.progress()).expect("entry is canonical");
        state.current_speed = Speed::ZERO;
        state.applied_acceleration = Acceleration::ZERO;
        state.status = VehicleStatus::Parked;
        self.parking_runtime.commit(&self.parking, vehicle, space);
        Ok(ParkingCommitRecord {
            vehicle,
            space,
            effect: ParkingCommandEffect::Applied,
        })
    }

    /// 原子创建 off-lane Parked vehicle 与 Occupied binding。
    pub fn spawn_parked_vehicle(
        &mut self,
        input: ParkedVehicleSpawnInput,
    ) -> Result<ParkedVehicleSpawnRecord, CoreError> {
        validate_external_id("vehicles[].id", &input.id)?;
        validate_external_id("vehicles[].routeId", &input.route_id)?;
        if self.vehicle_handles.contains_key(&input.id) {
            return Err(CoreError::DuplicateVehicleId {
                vehicle_id: input.id,
            });
        }
        if self.vehicle_profile(input.profile).is_none() {
            return Err(CoreError::UnknownVehicleProfileHandle {
                vehicle_id: input.id,
                profile: input.profile,
            });
        }
        let route =
            self.route_handle(&input.route_id)
                .ok_or_else(|| CoreError::UnknownVehicleRoute {
                    vehicle_id: input.id.clone(),
                    route_id: input.route_id.clone(),
                })?;
        if self.parking.space(input.space).is_none() {
            return Err(CoreError::UnknownParkingSpaceHandle { space: input.space });
        }

        let planned_slot_index = self
            .free_vehicle_indices
            .last()
            .copied()
            .unwrap_or(self.vehicles.len());
        let planned_generation = self
            .vehicles
            .get(planned_slot_index)
            .map_or(0, |slot| slot.generation);
        let handle = VehicleHandle::new(planned_slot_index, planned_generation);
        match self
            .parking_runtime
            .space_state(input.space)
            .expect("resolved ParkingSpace must have runtime state")
        {
            ParkingSpaceState::Vacant => {}
            ParkingSpaceState::Reserved { vehicle } => {
                return Err(CoreError::ParkingSpaceUnavailable {
                    command: ParkingCommandKind::SpawnParkedVehicle,
                    space: input.space,
                    requested_vehicle: handle,
                    current_vehicle: vehicle,
                    binding: ParkingBindingKind::Reserved,
                });
            }
            ParkingSpaceState::Occupied { vehicle } => {
                return Err(CoreError::ParkingSpaceUnavailable {
                    command: ParkingCommandKind::SpawnParkedVehicle,
                    space: input.space,
                    requested_vehicle: handle,
                    current_vehicle: vehicle,
                    binding: ParkingBindingKind::Occupied,
                });
            }
        }
        let route_slot = self
            .route_slot(route)
            .expect("resolved route handle must remain active");
        let Some(actual_edge) = route_slot.edge_handles.get(input.route_edge_index).copied() else {
            return Err(CoreError::InvalidParkingRouteOccurrence {
                command: ParkingCommandKind::SpawnParkedVehicle,
                vehicle: handle,
                route,
                route_edge_index: input.route_edge_index,
                route_edge_count: route_slot.edge_handles.len(),
            });
        };
        let entry = self
            .parking
            .space_entry(input.space)
            .expect("resolved ParkingSpace must have entry");
        if actual_edge != entry.edge() {
            return Err(CoreError::ParkingRouteOccurrenceEdgeMismatch {
                command: ParkingCommandKind::SpawnParkedVehicle,
                space: input.space,
                anchor: crate::ParkingAnchorKind::Entry,
                route,
                route_edge_index: input.route_edge_index,
                expected_edge: entry.edge(),
                actual_edge,
            });
        }
        self.validate_route_assignment(input.profile, route, input.route_edge_index)?;

        self.vehicle_handles.reserve(1);
        self.vehicle_update_order.reserve_for_append();
        if planned_slot_index == self.vehicles.len() {
            self.vehicles.reserve(1);
        }
        self.route_reference_indices[route.index()].reserve_for_attach();
        self.parking_runtime
            .prepare_vehicle_slot(planned_slot_index);
        self.candidate_state_scratch.reserve_for_slots(
            self.vehicles.len() + usize::from(planned_slot_index == self.vehicles.len()),
        );

        let external_id = input.id;
        let resolver_id = external_id.clone();
        let update_order_position = self.vehicle_update_order.append(handle);
        let state = VehicleState::new(
            handle,
            input.profile,
            route,
            input.route_edge_index,
            EdgeProgress::try_new(entry.progress()).expect("entry progress is canonical"),
            Speed::ZERO,
            VehicleStatus::Parked,
        );
        let slot = VehicleSlot {
            generation: planned_generation,
            external_id,
            state: Some(state),
            update_order_position: Some(update_order_position),
        };
        if planned_slot_index < self.vehicles.len() {
            let popped = self
                .free_vehicle_indices
                .pop()
                .expect("planned reusable vehicle slot must remain available");
            assert_eq!(popped, planned_slot_index);
            self.vehicles[planned_slot_index] = slot;
        } else {
            self.vehicles.push(slot);
        }
        self.vehicle_handles.insert(resolver_id, handle);
        self.route_reference_indices[route.index()].attach(handle, update_order_position);
        self.parking_runtime
            .occupy_new(&self.parking, handle, input.space);
        self.compact_update_order_if_needed();
        Ok(ParkedVehicleSpawnRecord {
            vehicle: handle,
            space: input.space,
            route,
            route_edge_index: input.route_edge_index,
        })
    }

    /// 在不 teleport 的前提下替换 exact Reserved Active vehicle 的 route occurrence。
    pub fn rebind_reserved_vehicle_route(
        &mut self,
        input: RebindReservedVehicleRouteInput,
    ) -> Result<ReservedVehicleRouteRebindRecord, CoreError> {
        let state = self
            .vehicle(input.vehicle)
            .ok_or(CoreError::UnknownVehicleHandle {
                vehicle: input.vehicle,
            })?;
        let status = state.status;
        let from_route = state.route;
        let from_route_edge_index = state.route_edge_index;
        let current_progress = state.edge_progress;
        let profile = state.profile;
        if self.parking.space(input.space).is_none() {
            return Err(CoreError::UnknownParkingSpaceHandle { space: input.space });
        }
        let route_slot = self
            .route_slot(input.route)
            .ok_or(CoreError::UnknownRouteHandle { route: input.route })?;
        let Some(target_edge) = route_slot.edge_handles.get(input.route_edge_index).copied() else {
            return Err(CoreError::InvalidParkingRouteOccurrence {
                command: ParkingCommandKind::RebindReservedVehicleRoute,
                vehicle: input.vehicle,
                route: input.route,
                route_edge_index: input.route_edge_index,
                route_edge_count: route_slot.edge_handles.len(),
            });
        };
        if status != VehicleStatus::Active {
            return Err(CoreError::ParkingVehicleStatusMismatch {
                command: ParkingCommandKind::RebindReservedVehicleRoute,
                vehicle: input.vehicle,
                expected: VehicleStatus::Active,
                actual: status,
            });
        }
        let exact_reservation = matches!(
            self.parking_runtime.vehicle_binding(input.vehicle),
            Some(RuntimeVehicleParkingBinding::Reserved { space, .. })
                if space == input.space
        ) && self.parking_runtime.space_state(input.space)
            == Some(ParkingSpaceState::Reserved {
                vehicle: input.vehicle,
            });
        if !exact_reservation {
            return Err(CoreError::ParkingReservationMismatch {
                command: ParkingCommandKind::RebindReservedVehicleRoute,
                vehicle: input.vehicle,
                space: input.space,
            });
        }
        if from_route == input.route && from_route_edge_index == input.route_edge_index {
            return Ok(ReservedVehicleRouteRebindRecord {
                vehicle: input.vehicle,
                space: input.space,
                from_route,
                from_route_edge_index,
                to_route: input.route,
                to_route_edge_index: input.route_edge_index,
                effect: ParkingCommandEffect::AlreadySatisfied,
            });
        }

        let current_edge = self
            .routes
            .get(from_route.index())
            .and_then(|route| route.edge_handles.get(from_route_edge_index))
            .copied()
            .expect("live vehicle occurrence must remain valid");
        if target_edge != current_edge {
            return Err(CoreError::ParkingRouteRebindEdgeMismatch {
                vehicle: input.vehicle,
                space: input.space,
                route: input.route,
                route_edge_index: input.route_edge_index,
                current_edge,
                target_edge,
            });
        }
        let target = self
            .first_reachable_parking_entry(
                input.route,
                input.route_edge_index,
                current_progress.value(),
                input.space,
            )
            .ok_or(CoreError::ParkingEntryUnreachable {
                vehicle: input.vehicle,
                space: input.space,
                route: input.route,
                from_route_edge_index: input.route_edge_index,
            })?;
        let candidate = NormalizedVehicleInput {
            profile,
            route_edge_index: input.route_edge_index,
            edge_progress: current_progress,
            current_speed: state.current_speed,
            status: VehicleStatus::Active,
        };
        self.validate_route_assignment(profile, input.route, input.route_edge_index)?;
        self.validate_candidate_overlap_excluding(input.vehicle, input.route, &candidate)?;

        let update_order_position = self.vehicles[input.vehicle.index()]
            .update_order_position
            .expect("live vehicle must have update order");
        if from_route != input.route {
            self.route_reference_indices[input.route.index()].reserve_for_attach();
        }
        let state = self.vehicles[input.vehicle.index()]
            .state
            .as_mut()
            .expect("resolved vehicle must remain live");
        state.route = input.route;
        state.route_edge_index = input.route_edge_index;
        if from_route != input.route {
            self.route_reference_indices[from_route.index()]
                .detach(input.vehicle, update_order_position);
            self.route_reference_indices[input.route.index()]
                .attach(input.vehicle, update_order_position);
        }
        self.parking_runtime.rebind_target(input.vehicle, target);
        Ok(ReservedVehicleRouteRebindRecord {
            vehicle: input.vehicle,
            space: input.space,
            from_route,
            from_route_edge_index,
            to_route: input.route,
            to_route_edge_index: input.route_edge_index,
            effect: ParkingCommandEffect::Applied,
        })
    }

    /// 把 exact Occupied/Parked pair 安全插入 caller-selected exit occurrence。
    pub fn leave_parking(
        &mut self,
        input: LeaveParkingInput,
    ) -> Result<ParkingLeaveRecord, CoreError> {
        let state = self
            .vehicle(input.vehicle)
            .ok_or(CoreError::UnknownVehicleHandle {
                vehicle: input.vehicle,
            })?;
        let status = state.status;
        let from_route = state.route;
        let profile = state.profile;
        if self.parking.space(input.space).is_none() {
            return Err(CoreError::UnknownParkingSpaceHandle { space: input.space });
        }
        let route_slot = self
            .route_slot(input.route)
            .ok_or(CoreError::UnknownRouteHandle { route: input.route })?;
        let Some(actual_edge) = route_slot.edge_handles.get(input.route_edge_index).copied() else {
            return Err(CoreError::InvalidParkingRouteOccurrence {
                command: ParkingCommandKind::Leave,
                vehicle: input.vehicle,
                route: input.route,
                route_edge_index: input.route_edge_index,
                route_edge_count: route_slot.edge_handles.len(),
            });
        };
        let exit = self
            .parking
            .space_exit(input.space)
            .expect("resolved ParkingSpace must have exit");
        if actual_edge != exit.edge() {
            return Err(CoreError::ParkingRouteOccurrenceEdgeMismatch {
                command: ParkingCommandKind::Leave,
                space: input.space,
                anchor: crate::ParkingAnchorKind::Exit,
                route: input.route,
                route_edge_index: input.route_edge_index,
                expected_edge: exit.edge(),
                actual_edge,
            });
        }

        let binding = self.parking_runtime.vehicle_binding(input.vehicle);
        let space_state = self
            .parking_runtime
            .space_state(input.space)
            .expect("resolved ParkingSpace must have runtime state");
        let exact_noop = status == VehicleStatus::Active
            && binding.is_none()
            && space_state == ParkingSpaceState::Vacant
            && state.route == input.route
            && state.route_edge_index == input.route_edge_index
            && state.edge_progress.value() == exit.progress()
            && state.current_speed == Speed::ZERO
            && state.applied_acceleration == Acceleration::ZERO;
        if exact_noop {
            return Ok(ParkingLeaveRecord {
                vehicle: input.vehicle,
                space: input.space,
                route: input.route,
                route_edge_index: input.route_edge_index,
                effect: ParkingCommandEffect::AlreadySatisfied,
            });
        }
        if status != VehicleStatus::Parked {
            return Err(CoreError::ParkingVehicleStatusMismatch {
                command: ParkingCommandKind::Leave,
                vehicle: input.vehicle,
                expected: VehicleStatus::Parked,
                actual: status,
            });
        }
        let exact_occupancy = matches!(
            binding,
            Some(RuntimeVehicleParkingBinding::Occupied { space, .. })
                if space == input.space
        ) && space_state
            == (ParkingSpaceState::Occupied {
                vehicle: input.vehicle,
            });
        if !exact_occupancy {
            return Err(CoreError::ParkingOccupancyMismatch {
                command: ParkingCommandKind::Leave,
                vehicle: input.vehicle,
                space: input.space,
            });
        }

        let candidate = NormalizedVehicleInput {
            profile,
            route_edge_index: input.route_edge_index,
            edge_progress: EdgeProgress::try_new(exit.progress()).expect("exit is canonical"),
            current_speed: Speed::ZERO,
            status: VehicleStatus::Active,
        };
        self.validate_route_assignment(profile, input.route, input.route_edge_index)?;
        self.validate_candidate_overlap_excluding(input.vehicle, input.route, &candidate)?;
        self.validate_parking_leave_followers(
            input.vehicle,
            input.space,
            input.route,
            input.route_edge_index,
            &candidate,
        )?;

        let update_order_position = self.vehicles[input.vehicle.index()]
            .update_order_position
            .expect("live vehicle must have update order");
        if from_route != input.route {
            self.route_reference_indices[input.route.index()].reserve_for_attach();
        }
        let occupant = CommandOccupant {
            vehicle: input.vehicle,
            front_progress: exit.progress(),
        };
        let mut spatial = std::mem::take(&mut self.command_spatial_index);
        let vehicles = &self.vehicles;
        let mut resolve_progress = |handle: VehicleHandle| {
            vehicles[handle.index()]
                .state
                .as_ref()
                .expect("command spatial occupant must be live")
                .edge_progress
                .value()
        };
        spatial.prepare_insert(exit.edge(), occupant, &mut resolve_progress);

        let state = self.vehicles[input.vehicle.index()]
            .state
            .as_mut()
            .expect("resolved vehicle must remain live");
        state.route = input.route;
        state.route_edge_index = input.route_edge_index;
        state.edge_progress = candidate.edge_progress;
        state.current_speed = Speed::ZERO;
        state.applied_acceleration = Acceleration::ZERO;
        state.status = VehicleStatus::Active;
        if from_route != input.route {
            self.route_reference_indices[from_route.index()]
                .detach(input.vehicle, update_order_position);
            self.route_reference_indices[input.route.index()]
                .attach(input.vehicle, update_order_position);
        }
        let released = self.parking_runtime.release(&self.parking, input.vehicle);
        assert_eq!(released, Some((input.space, ParkingBindingKind::Occupied)));
        let vehicles = &self.vehicles;
        let mut resolve_progress = |handle: VehicleHandle| {
            vehicles[handle.index()]
                .state
                .as_ref()
                .expect("command spatial occupant must be live")
                .edge_progress
                .value()
        };
        spatial.insert(exit.edge(), occupant, &mut resolve_progress);
        self.command_spatial_index = spatial;
        Ok(ParkingLeaveRecord {
            vehicle: input.vehicle,
            space: input.space,
            route: input.route,
            route_edge_index: input.route_edge_index,
            effect: ParkingCommandEffect::Applied,
        })
    }

    pub(super) fn first_reachable_parking_entry(
        &self,
        route: RouteHandle,
        from_route_edge_index: usize,
        from_progress: f64,
        space: crate::ParkingSpaceHandle,
    ) -> Option<ParkingApproachTarget> {
        let entry = self
            .parking
            .space_entry(space)
            .expect("resolved ParkingSpace must have entry");
        let route_slot = self
            .route_slot(route)
            .expect("live vehicle route must remain active");
        let current_matches = route_slot.edge_handles[from_route_edge_index] == entry.edge()
            && longitudinal_constraint_reached(entry.progress(), from_progress);
        let route_edge_index = if current_matches {
            from_route_edge_index
        } else {
            route_slot
                .edge_handles
                .iter()
                .copied()
                .enumerate()
                .skip(from_route_edge_index + 1)
                .find_map(|(index, edge)| (edge == entry.edge()).then_some(index))?
        };
        Some(ParkingApproachTarget {
            route,
            route_edge_index,
        })
    }

    pub(super) fn parking_arrived(
        &self,
        vehicle: VehicleHandle,
        space: crate::ParkingSpaceHandle,
        target: Option<ParkingApproachTarget>,
    ) -> bool {
        let Some(target) = target else {
            return false;
        };
        let Some(state) = self.vehicle(vehicle) else {
            return false;
        };
        let entry = self
            .parking
            .space_entry(space)
            .expect("resolved ParkingSpace must have entry");
        state.status == VehicleStatus::Active
            && state.route == target.route
            && state.route_edge_index == target.route_edge_index
            && longitudinal_positions_match(state.edge_progress.value(), entry.progress())
            && state.current_speed == Speed::ZERO
    }

}
