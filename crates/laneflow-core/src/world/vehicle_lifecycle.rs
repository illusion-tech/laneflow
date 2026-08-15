use super::*;

impl CoreWorld {
    /// 创建新的 vehicle runtime entity。
    pub fn spawn_vehicle(&mut self, input: VehicleSpawnInput) -> Result<VehicleHandle, CoreError> {
        self.spawn_vehicle_with_overlap_validation(input, true)
    }

    /// 把一个 live、未绑定 Parking 的 Completed vehicle 原子替换为新的 Active vehicle。
    ///
    /// 物理重叠返回可重试的 [`VehicleReplaceOutcome::Blocked`]；其他 validation failure
    /// 返回 [`CoreError`]。任一失败结果都不会修改 committed world state。
    pub fn replace_completed_vehicle(
        &mut self,
        old: VehicleHandle,
        input: &VehicleReplaceInput,
    ) -> Result<VehicleReplaceOutcome, CoreError> {
        let old_slot = self
            .vehicle_slot(old)
            .ok_or(CoreError::UnknownVehicleHandle { vehicle: old })?;
        let old_state = old_slot
            .state
            .as_ref()
            .expect("validated vehicle slot must contain state");
        if old_state.status != VehicleStatus::Completed {
            return Err(CoreError::VehicleReplaceStatusMismatch {
                vehicle: old,
                actual: old_state.status,
            });
        }
        if let Some(binding) = self.parking_runtime.vehicle_binding(old) {
            return Err(CoreError::ParkingBindingInvariantViolation {
                stage: "replace_completed_vehicle",
                vehicle: Some(old),
                space: Some(binding.space()),
            });
        }

        let old_external_id = old_slot.external_id.as_str();
        let (preserve_external_id, replacement_external_id) = match &input.external_id {
            VehicleReplaceExternalId::Preserve => (true, old_external_id),
            VehicleReplaceExternalId::ReplaceWith(external_id)
                if external_id == old_external_id =>
            {
                (true, old_external_id)
            }
            VehicleReplaceExternalId::ReplaceWith(external_id) => {
                validate_external_id("vehicleReplace.externalId", external_id)?;
                if self.vehicle_handles.contains_key(external_id) {
                    return Err(CoreError::DuplicateVehicleId {
                        vehicle_id: external_id.clone(),
                    });
                }
                (false, external_id.as_str())
            }
        };
        if self.vehicle_profile(input.profile).is_none() {
            return Err(CoreError::UnknownVehicleProfileHandle {
                vehicle_id: replacement_external_id.to_owned(),
                profile: input.profile,
            });
        }
        self.route_slot(input.route)
            .ok_or(CoreError::UnknownRouteHandle { route: input.route })?;
        let normalized = self.normalize_vehicle_replace_input(old, input)?;
        self.validate_route_assignment(input.profile, input.route, normalized.route_edge_index)?;
        let old_generation = old_slot.generation;
        let old_route = old_state.route;
        let update_order_position = old_slot
            .update_order_position
            .expect("live vehicle must have reverse update-order position");
        if let Some(overlap) = self.find_candidate_overlap_excluding(old, input.route, &normalized)
        {
            return Ok(VehicleReplaceOutcome::Blocked(VehicleReplaceBlock {
                old,
                blocker: overlap.blocker,
                blocker_position: overlap.blocker_position,
                bumper_gap: overlap.bumper_gap,
            }));
        }

        let reusable_old_generation = old_generation.checked_add(1);
        let planned_slot_index = reusable_old_generation.map_or_else(
            || {
                self.free_vehicle_indices
                    .last()
                    .copied()
                    .unwrap_or(self.vehicles.len())
            },
            |_| old.index(),
        );
        let planned_generation = reusable_old_generation.unwrap_or_else(|| {
            self.vehicles
                .get(planned_slot_index)
                .map_or(0, |slot| slot.generation)
        });
        let new = VehicleHandle::new(planned_slot_index, planned_generation);
        assert_ne!(old, new, "replacement must create a distinct handle");

        let prepared_ids = if preserve_external_id {
            PreparedVehicleReplaceIds::Preserve
        } else {
            let VehicleReplaceExternalId::ReplaceWith(external_id) = &input.external_id else {
                unreachable!("non-preserve replacement must provide an external ID")
            };
            PreparedVehicleReplaceIds::Replace {
                slot: external_id.clone(),
                resolver: external_id.clone(),
            }
        };
        self.parking_runtime
            .prepare_vehicle_slot(planned_slot_index);
        if planned_slot_index == self.vehicles.len() {
            self.vehicles.reserve(1);
        }
        if old_route != input.route {
            self.route_reference_indices[input.route.index()].reserve_for_attach();
        }
        let replacement_edge =
            self.routes[input.route.index()].edge_handles[normalized.route_edge_index];
        let replacement_occupant = CommandOccupant {
            vehicle: new,
            front_progress: normalized.edge_progress.value(),
        };
        {
            let vehicles = &self.vehicles;
            let mut resolve_progress = |handle: VehicleHandle| {
                vehicles[handle.index()]
                    .state
                    .as_ref()
                    .expect("command spatial occupant must be live")
                    .edge_progress
                    .value()
            };
            self.command_spatial_index.prepare_insert(
                replacement_edge,
                replacement_occupant,
                &mut resolve_progress,
            );
        }
        self.command_spatial_index
            .prepare_note_vehicle_speed(normalized.current_speed.value());
        self.candidate_state_scratch.reserve_for_slots(
            self.vehicles.len() + usize::from(planned_slot_index == self.vehicles.len()),
        );

        #[cfg(any(test, feature = "test-support"))]
        if self.replace_failure_after_prepare {
            return Err(CoreError::ParkingBindingInvariantViolation {
                stage: "test_after_vehicle_replace_prepare",
                vehicle: Some(old),
                space: None,
            });
        }

        let old_slot = &mut self.vehicles[old.index()];
        old_slot
            .state
            .take()
            .expect("validated old vehicle must remain live");
        let old_slot_external_id = std::mem::take(&mut old_slot.external_id);
        old_slot.update_order_position = None;
        let (old_resolver_external_id, removed) = self
            .vehicle_handles
            .swap_remove_entry(old_slot_external_id.as_str())
            .expect("vehicle resolver must contain old external ID");
        assert_eq!(removed, old, "vehicle resolver must identify old vehicle");
        let (slot_external_id, resolver_external_id) = match prepared_ids {
            PreparedVehicleReplaceIds::Preserve => (old_slot_external_id, old_resolver_external_id),
            PreparedVehicleReplaceIds::Replace { slot, resolver } => (slot, resolver),
        };
        let replacement_slot = VehicleSlot {
            generation: planned_generation,
            external_id: slot_external_id,
            state: Some(VehicleState::new(
                new,
                normalized.profile,
                input.route,
                normalized.route_edge_index,
                normalized.edge_progress,
                normalized.current_speed,
                VehicleStatus::Active,
            )),
            update_order_position: Some(update_order_position),
        };
        if planned_slot_index == old.index() {
            self.vehicles[planned_slot_index] = replacement_slot;
        } else if planned_slot_index < self.vehicles.len() {
            let popped = self
                .free_vehicle_indices
                .pop()
                .expect("planned reusable vehicle slot must remain available");
            assert_eq!(popped, planned_slot_index);
            self.vehicles[planned_slot_index] = replacement_slot;
        } else {
            self.vehicles.push(replacement_slot);
        }

        self.vehicle_update_order
            .replace(update_order_position, old, new);
        if old_route == input.route {
            self.route_reference_indices[input.route.index()].replace(
                old,
                new,
                update_order_position,
            );
        } else {
            self.route_reference_indices[old_route.index()].detach(old, update_order_position);
            self.route_reference_indices[input.route.index()].attach(new, update_order_position);
        }
        let replaced = self.vehicle_handles.insert(resolver_external_id, new);
        assert!(
            replaced.is_none(),
            "validated replacement external ID must remain unoccupied"
        );
        self.parking_runtime.register_unbound_vehicle(new);
        {
            let vehicles = &self.vehicles;
            let mut resolve_progress = |handle: VehicleHandle| {
                vehicles[handle.index()]
                    .state
                    .as_ref()
                    .expect("command spatial occupant must be live")
                    .edge_progress
                    .value()
            };
            self.command_spatial_index.insert(
                replacement_edge,
                replacement_occupant,
                &mut resolve_progress,
            );
        }
        self.command_spatial_index
            .note_vehicle_speed(new, normalized.current_speed.value());

        Ok(VehicleReplaceOutcome::Replaced(VehicleReplaceRecord {
            old,
            new,
        }))
    }

    pub(super) fn spawn_vehicle_without_overlap_validation(
        &mut self,
        input: VehicleSpawnInput,
    ) -> Result<VehicleHandle, CoreError> {
        self.spawn_vehicle_with_overlap_validation(input, false)
    }

    pub(super) fn spawn_vehicle_with_overlap_validation(
        &mut self,
        input: VehicleSpawnInput,
        validate_overlap: bool,
    ) -> Result<VehicleHandle, CoreError> {
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
        if input.status == VehicleStatus::Parked {
            return Err(CoreError::ParkedVehicleRequiresParkingCommand {
                vehicle_id: input.id,
            });
        }
        let normalized = self.normalize_vehicle_input(route, &input)?;
        self.validate_route_assignment(input.profile, route, normalized.route_edge_index)?;
        if validate_overlap {
            self.validate_candidate_overlap(route, &input.id, &normalized)?;
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
        self.parking_runtime
            .prepare_vehicle_slot(planned_slot_index);
        let spatial_occupant = matches!(
            normalized.status,
            VehicleStatus::Active | VehicleStatus::Stopped
        )
        .then(|| {
            (
                self.routes[route.index()].edge_handles[normalized.route_edge_index],
                CommandOccupant {
                    vehicle: handle,
                    front_progress: normalized.edge_progress.value(),
                },
            )
        });
        self.vehicle_handles.reserve(1);
        self.vehicle_update_order.reserve_for_append();
        if planned_slot_index == self.vehicles.len() {
            self.vehicles.reserve(1);
        }
        self.route_reference_indices[route.index()].reserve_for_attach();
        if let Some((edge, occupant)) = spatial_occupant {
            let vehicles = &self.vehicles;
            let mut resolve_progress = |handle: VehicleHandle| {
                vehicles[handle.index()]
                    .state
                    .as_ref()
                    .expect("command spatial occupant must be live")
                    .edge_progress
                    .value()
            };
            self.command_spatial_index
                .prepare_insert(edge, occupant, &mut resolve_progress);
        }
        self.candidate_state_scratch.reserve_for_slots(
            self.vehicles.len() + usize::from(planned_slot_index == self.vehicles.len()),
        );

        let external_id = input.id;
        let resolver_id = external_id.clone();
        let update_order_position = self.vehicle_update_order.append(handle);
        let slot = VehicleSlot {
            generation: planned_generation,
            external_id,
            state: Some(VehicleState::new(
                handle,
                normalized.profile,
                route,
                normalized.route_edge_index,
                normalized.edge_progress,
                normalized.current_speed,
                normalized.status,
            )),
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
        self.parking_runtime.register_unbound_vehicle(handle);
        self.route_reference_indices[route.index()].attach(handle, update_order_position);
        if let Some((edge, occupant)) = spatial_occupant {
            let vehicles = &self.vehicles;
            let mut resolve_progress = |handle: VehicleHandle| {
                vehicles[handle.index()]
                    .state
                    .as_ref()
                    .expect("command spatial occupant must be live")
                    .edge_progress
                    .value()
            };
            self.command_spatial_index
                .insert(edge, occupant, &mut resolve_progress);
        }
        if normalized.status == VehicleStatus::Active {
            self.command_spatial_index
                .note_vehicle_speed(handle, normalized.current_speed.value());
        }
        self.compact_update_order_if_needed();
        Ok(handle)
    }

    /// 移除 live vehicle runtime entity。
    pub fn despawn_vehicle(
        &mut self,
        handle: VehicleHandle,
    ) -> Result<VehicleDespawnRecord, CoreError> {
        self.vehicle_slot(handle)
            .ok_or(CoreError::UnknownVehicleHandle { vehicle: handle })?;

        let slot = &self.vehicles[handle.index()];
        let update_order_position = slot
            .update_order_position
            .expect("live vehicle must have reverse update-order position");
        let state = slot
            .state
            .as_ref()
            .expect("validated vehicle slot must contain state");
        let profile = state.profile;
        let route = state.route;
        let status = state.status;
        let removed_speed = state.current_speed.value();
        let parking_binding = self.parking_runtime.vehicle_binding(handle);
        let parking_binding_valid = match (status, parking_binding) {
            (VehicleStatus::Parked, Some(RuntimeVehicleParkingBinding::Occupied { space, .. })) => {
                self.parking_runtime.space_state(space)
                    == Some(ParkingSpaceState::Occupied { vehicle: handle })
            }
            (VehicleStatus::Parked, _) => false,
            (VehicleStatus::Active, Some(RuntimeVehicleParkingBinding::Reserved { space, .. })) => {
                self.parking_runtime.space_state(space)
                    == Some(ParkingSpaceState::Reserved { vehicle: handle })
            }
            (_, Some(_)) => false,
            (_, None) => true,
        };
        if !parking_binding_valid {
            return Err(CoreError::ParkingBindingInvariantViolation {
                stage: "despawn",
                vehicle: Some(handle),
                space: parking_binding.map(RuntimeVehicleParkingBinding::space),
            });
        }
        let spatial_occupant = matches!(status, VehicleStatus::Active | VehicleStatus::Stopped)
            .then(|| {
                (
                    self.routes[route.index()].edge_handles[state.route_edge_index],
                    CommandOccupant {
                        vehicle: handle,
                        front_progress: state.edge_progress.value(),
                    },
                )
            });
        let reusable = slot.generation.checked_add(1);
        if reusable.is_some() {
            self.free_vehicle_indices.reserve(1);
        }
        if let Some((edge, occupant)) = spatial_occupant {
            let mut spatial = std::mem::take(&mut self.command_spatial_index);
            let vehicles = &self.vehicles;
            if status == VehicleStatus::Active {
                spatial.prepare_speed_removal(
                    removed_speed,
                    vehicles.iter().filter_map(|slot| {
                        let state = slot.state.as_ref()?;
                        (state.status == VehicleStatus::Active)
                            .then_some((state.handle, state.current_speed.value()))
                    }),
                );
            }
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
        }

        let parking_release =
            self.parking_runtime
                .release(&self.parking, handle)
                .map(|(space, previous_binding)| ParkingReleaseRecord {
                    vehicle: handle,
                    space,
                    previous_binding,
                    reason: ParkingReleaseReason::VehicleDespawn,
                });
        let slot = &mut self.vehicles[handle.index()];
        slot.state
            .take()
            .expect("validated vehicle slot must contain state");
        let external_id = std::mem::take(&mut slot.external_id);
        slot.update_order_position = None;
        let removed = self.vehicle_handles.swap_remove(&external_id);
        assert_eq!(
            removed,
            Some(handle),
            "vehicle resolver must identify removed vehicle"
        );
        // generation 耗尽时不复用 slot，避免 stale handle 在回绕后复活。
        if let Some(next_generation) = reusable {
            slot.generation = next_generation;
            self.free_vehicle_indices.push(handle.index());
        }
        self.vehicle_update_order
            .tombstone(update_order_position, handle);
        self.route_reference_indices[route.index()].detach(handle, update_order_position);
        self.compact_update_order_if_needed();

        Ok(VehicleDespawnRecord {
            handle,
            external_id,
            profile,
            route,
            status,
            parking_release,
        })
    }

    pub(super) fn first_route_reference(&mut self, route: RouteHandle) -> Option<VehicleHandle> {
        self.route_reference_indices[route.index()].first()
    }

    pub(super) fn rebuild_route_reference_index(&mut self, route: RouteHandle) {
        let order = &self.vehicle_update_order;
        let vehicles = &self.vehicles;
        let index = &mut self.route_reference_indices[route.index()];
        index.clear();
        for (position, vehicle) in order
            .entries
            .iter()
            .enumerate()
            .filter_map(|(position, entry)| entry.map(|vehicle| (position, vehicle)))
        {
            let Some(state) = vehicles
                .get(vehicle.index())
                .filter(|slot| slot.generation == vehicle.generation())
                .and_then(|slot| slot.state.as_ref())
            else {
                continue;
            };
            if state.route == route {
                index.attach(vehicle, position);
            }
        }
    }

    pub(super) fn rebuild_all_route_reference_indices(&mut self) {
        for index in &mut self.route_reference_indices {
            index.clear();
        }
        for (position, vehicle) in self
            .vehicle_update_order
            .entries
            .iter()
            .enumerate()
            .filter_map(|(position, entry)| entry.map(|vehicle| (position, vehicle)))
        {
            let state = self.vehicles[vehicle.index()]
                .state
                .as_ref()
                .expect("stable update order must identify live vehicle");
            self.route_reference_indices[state.route.index()].attach(vehicle, position);
        }
    }

    pub(super) fn compact_update_order_if_needed(&mut self) {
        if self.vehicle_update_order.compact(&mut self.vehicles) {
            self.rebuild_all_route_reference_indices();
        }
    }

    #[cfg(test)]
    pub(super) fn assert_lifecycle_indices_consistent(&mut self) {
        let mut seen = vec![false; self.vehicles.len()];
        let mut expected_route_counts = vec![0_usize; self.routes.len()];
        let mut expected_route_first = vec![None; self.routes.len()];

        for (position, vehicle) in self
            .vehicle_update_order
            .entries
            .iter()
            .enumerate()
            .filter_map(|(position, entry)| entry.map(|vehicle| (position, vehicle)))
        {
            let slot = self
                .vehicles
                .get(vehicle.index())
                .filter(|slot| slot.generation == vehicle.generation())
                .expect("stable update order entry must resolve");
            assert!(
                !seen[vehicle.index()],
                "vehicle must occur once in update order"
            );
            seen[vehicle.index()] = true;
            assert_eq!(slot.update_order_position, Some(position));
            let state = slot.state.as_ref().expect("ordered vehicle must be live");
            expected_route_counts[state.route.index()] += 1;
            expected_route_first[state.route.index()].get_or_insert(vehicle);
        }

        for (index, slot) in self.vehicles.iter().enumerate() {
            assert_eq!(slot.state.is_some(), seen[index]);
            assert_eq!(slot.update_order_position.is_some(), seen[index]);
        }

        for route_index in 0..self.routes.len() {
            if !self.routes[route_index].active {
                continue;
            }
            assert_eq!(
                self.route_reference_indices[route_index].live_count(),
                expected_route_counts[route_index]
            );
            let actual_first = self.route_reference_indices[route_index].first();
            assert_eq!(actual_first, expected_route_first[route_index]);
        }

        let mut expected_spatial = self
            .vehicles()
            .filter(|vehicle| {
                matches!(
                    vehicle.status,
                    VehicleStatus::Active | VehicleStatus::Stopped
                )
            })
            .map(|vehicle| {
                (
                    self.vehicle_edge(vehicle),
                    CommandOccupant {
                        vehicle: vehicle.handle,
                        front_progress: vehicle.edge_progress.value(),
                    },
                )
            })
            .collect::<Vec<_>>();
        let mut actual_spatial = self.command_spatial_index.occupants().collect::<Vec<_>>();
        let compare = |left: &(EdgeHandle, CommandOccupant),
                       right: &(EdgeHandle, CommandOccupant)| {
            left.0
                .index()
                .cmp(&right.0.index())
                .then_with(|| left.1.front_progress.total_cmp(&right.1.front_progress))
                .then_with(|| left.1.vehicle.index().cmp(&right.1.vehicle.index()))
                .then_with(|| {
                    left.1
                        .vehicle
                        .generation()
                        .cmp(&right.1.vehicle.generation())
                })
        };
        expected_spatial.sort_unstable_by(compare);
        actual_spatial.sort_unstable_by(compare);
        assert_eq!(actual_spatial, expected_spatial);
        self.parking_runtime
            .assert_consistent(&self.parking, |vehicle| {
                self.vehicle(vehicle).map(|state| state.status)
            });
    }
}

impl CoreWorld {
    pub(super) fn normalize_vehicle_replace_input(
        &self,
        old: VehicleHandle,
        input: &VehicleReplaceInput,
    ) -> Result<NormalizedVehicleInput, CoreError> {
        let route = self
            .route_slot(input.route)
            .expect("replacement route handle must be active");
        let edge = route
            .edge_handles
            .get(input.route_edge_index)
            .copied()
            .ok_or(CoreError::InvalidVehicleReplaceRouteEdgeIndex {
                vehicle: old,
                route: input.route,
                route_edge_index: input.route_edge_index,
                route_edge_count: route.edge_handles.len(),
            })?;
        let edge_length = self
            .lane_graph
            .edge_length(edge)
            .expect("validated replacement route edge must exist");
        if input.edge_progress.value() > edge_length.value() {
            return Err(CoreError::VehicleReplaceEdgeProgressOutOfRange {
                vehicle: old,
                edge,
                edge_progress: input.edge_progress.value(),
                edge_length: edge_length.value(),
            });
        }
        let speed_limit = self
            .lane_graph
            .edge_speed_limit(edge)
            .expect("validated replacement route edge must exist");
        if input.initial_speed.value() > speed_limit.value() {
            return Err(CoreError::VehicleReplaceInitialSpeedExceedsLimit {
                vehicle: old,
                edge,
                initial_speed: input.initial_speed.value(),
                speed_limit: speed_limit.value(),
            });
        }

        Ok(NormalizedVehicleInput {
            profile: input.profile,
            route_edge_index: input.route_edge_index,
            edge_progress: input.edge_progress,
            current_speed: input.initial_speed,
            status: VehicleStatus::Active,
        })
    }

    pub(super) fn normalize_vehicle_input(
        &self,
        route: RouteHandle,
        input: &VehicleSpawnInput,
    ) -> Result<NormalizedVehicleInput, CoreError> {
        let route_slot = self
            .route_slot(route)
            .expect("route handle was resolved from active route map");
        let edge = route_slot
            .edge_handles
            .get(input.route_edge_index)
            .copied()
            .ok_or_else(|| CoreError::InvalidVehicleRouteEdgeIndex {
                vehicle_id: input.id.clone(),
                route_id: input.route_id.clone(),
                route_edge_index: input.route_edge_index,
                route_edge_count: route_slot.edge_handles.len(),
            })?;

        let edge_length = self
            .lane_graph
            .edge_length(edge)
            .expect("validated route edge must exist");
        if input.edge_progress.value() > edge_length.value() {
            return Err(CoreError::VehicleEdgeProgressOutOfRange {
                vehicle_id: input.id.clone(),
                edge_id: self
                    .lane_graph
                    .edge_external_id(edge)
                    .expect("validated route edge must exist")
                    .to_owned(),
                edge_progress: input.edge_progress.value(),
                edge_length: edge_length.value(),
            });
        }

        if input.status != VehicleStatus::Active && input.initial_speed != Speed::ZERO {
            return Err(CoreError::InvalidInactiveVehicleMotion {
                vehicle_id: input.id.clone(),
                status: input.status,
                initial_speed: input.initial_speed.value(),
            });
        }

        let speed_limit = self
            .lane_graph
            .edge_speed_limit(edge)
            .expect("validated route edge must exist");
        if input.initial_speed.value() > speed_limit.value() {
            return Err(CoreError::VehicleInitialSpeedExceedsLimit {
                vehicle_id: input.id.clone(),
                edge_id: self
                    .lane_graph
                    .edge_external_id(edge)
                    .expect("validated route edge must exist")
                    .to_owned(),
                initial_speed: input.initial_speed.value(),
                speed_limit: speed_limit.value(),
            });
        }

        let mut edge_progress = input.edge_progress;
        if input.status == VehicleStatus::Completed {
            let expected_route_edge_index = route_slot.edge_handles.len() - 1;
            if input.route_edge_index != expected_route_edge_index
                || input.edge_progress.value() + EDGE_BOUNDARY_TOLERANCE_METERS
                    < edge_length.value()
            {
                return Err(CoreError::InvalidCompletedVehicleState {
                    vehicle_id: input.id.clone(),
                    route_id: input.route_id.clone(),
                    route_edge_index: input.route_edge_index,
                    expected_route_edge_index,
                    edge_progress: input.edge_progress.value(),
                    edge_length: edge_length.value(),
                });
            }

            edge_progress =
                EdgeProgress::try_new(edge_length.value()).expect("edge length is valid");
        }

        Ok(NormalizedVehicleInput {
            profile: input.profile,
            route_edge_index: input.route_edge_index,
            edge_progress,
            current_speed: input.initial_speed,
            status: input.status,
        })
    }
}
