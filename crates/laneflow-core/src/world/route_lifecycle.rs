use super::*;

impl CoreWorld {
    /// 注册新的 route definition。
    pub fn register_route(&mut self, route: Route) -> Result<RouteHandle, CoreError> {
        if self.route_handles.contains_key(route.id()) {
            return Err(CoreError::DuplicateRouteId {
                route_id: route.id().to_owned(),
            });
        }

        let route = compile_route(
            &self.lane_graph,
            &self.junctions,
            &self.signals,
            &self.waiting,
            route,
        )?;
        self.register_compiled_route(route)
    }

    pub(super) fn register_compiled_route(&mut self, route: CompiledRoute) -> Result<RouteHandle, CoreError> {
        if self.route_handles.contains_key(route.definition().id()) {
            return Err(CoreError::DuplicateRouteId {
                route_id: route.definition().id().to_owned(),
            });
        }

        let CompiledRoute {
            definition,
            edge_handles,
            transition_gates,
            maneuver_occurrences,
            gate_occurrences,
            waiting_zone_occurrences,
        } = route;
        let edge_lengths = edge_handles
            .iter()
            .map(|edge| {
                self.lane_graph
                    .edge_length(*edge)
                    .expect("normalized route edge must exist")
                    .value()
            })
            .collect::<Vec<_>>();
        let (transitions, next_controlled_transition, speed_limit_transitions) =
            self.build_route_metadata(&edge_handles, &edge_lengths, &transition_gates);
        let distance_index = RouteDistanceIndex::build(&edge_lengths);
        let external_id = definition.id().to_owned();

        let handle = if let Some(index) = self.free_route_indices.pop() {
            let generation = self.routes[index].generation;
            self.routes[index] = RouteSlot {
                generation,
                external_id: external_id.clone(),
                edge_handles,
                transitions,
                maneuver_occurrences,
                gate_occurrences,
                waiting_zone_occurrences,
                next_controlled_transition,
                speed_limit_transitions,
                active: true,
            };
            self.route_distance_indices[index] = distance_index;
            self.route_reference_indices[index] = RouteReferenceIndex::default();
            RouteHandle::new(index, generation)
        } else {
            let handle = RouteHandle::new(self.routes.len(), 0);
            self.routes.push(RouteSlot {
                generation: 0,
                external_id: external_id.clone(),
                edge_handles,
                transitions,
                maneuver_occurrences,
                gate_occurrences,
                waiting_zone_occurrences,
                next_controlled_transition,
                speed_limit_transitions,
                active: true,
            });
            self.route_distance_indices.push(distance_index);
            self.route_reference_indices
                .push(RouteReferenceIndex::default());
            handle
        };

        self.route_handles.insert(external_id, handle);
        Ok(handle)
    }

    pub(super) fn build_route_metadata(
        &self,
        edge_handles: &[EdgeHandle],
        edge_lengths: &[f64],
        transition_gates: &[Option<ManeuverGateHandle>],
    ) -> (
        Vec<RouteTransition>,
        Vec<Option<NextControlledRouteTransition>>,
        Vec<SpeedLimitRouteTransition>,
    ) {
        let transitions = edge_handles
            .windows(2)
            .zip(transition_gates.iter().copied())
            .map(|(pair, gate)| {
                let to_edge = pair[1];
                RouteTransition { to_edge, gate }
            })
            .collect::<Vec<_>>();
        let mut next_controlled_transition = vec![None; edge_handles.len()];
        let mut next = None;
        for route_edge_index in (0..edge_handles.len()).rev() {
            let edge_length = edge_lengths[route_edge_index];
            if let Some(gate) = transitions
                .get(route_edge_index)
                .and_then(|transition| transition.gate)
                .filter(|gate| self.signals.maneuver_gate_is_signal_controlled(*gate))
            {
                next = Some(NextControlledRouteTransition {
                    from_route_edge_index: route_edge_index,
                    gate,
                    distance_from_edge_start: BoundedDistance::Finite(edge_length),
                });
            } else if let Some(candidate) = next.as_mut() {
                candidate.distance_from_edge_start =
                    candidate.distance_from_edge_start.add(edge_length);
            }
            next_controlled_transition[route_edge_index] = next;
        }
        let speed_limit_transitions = edge_handles
            .windows(2)
            .enumerate()
            .filter_map(|(from_route_edge_index, pair)| {
                let from_speed = self
                    .lane_graph
                    .edge_speed_limit(pair[0])
                    .expect("normalized route edge must exist")
                    .value();
                let target_speed = self
                    .lane_graph
                    .edge_speed_limit(pair[1])
                    .expect("normalized route edge must exist")
                    .value();
                (target_speed < from_speed).then_some(SpeedLimitRouteTransition {
                    from_route_edge_index,
                    to_edge: pair[1],
                    target_speed,
                })
            })
            .collect();
        (
            transitions,
            next_controlled_transition,
            speed_limit_transitions,
        )
    }

    /// 移除未被 live vehicle 引用的 route definition。
    pub fn remove_route(&mut self, handle: RouteHandle) -> Result<RouteRemoveRecord, CoreError> {
        self.route_slot(handle)
            .ok_or(CoreError::UnknownRouteHandle { route: handle })?;

        if self.route_reference_indices[handle.index()].live_count() > 0 {
            let vehicle = self.first_route_reference(handle).or_else(|| {
                self.rebuild_route_reference_index(handle);
                self.first_route_reference(handle)
            });
            return Err(CoreError::RouteInUse {
                route: handle,
                vehicle: vehicle.expect("positive route reference count must have a live vehicle"),
            });
        }

        let reusable = self.routes[handle.index()].generation.checked_add(1);
        if reusable.is_some() {
            self.free_route_indices.reserve(1);
        }
        let route = &mut self.routes[handle.index()];
        let external_id = std::mem::take(&mut route.external_id);
        route.active = false;
        route.edge_handles.clear();
        route.transitions.clear();
        route.maneuver_occurrences.clear();
        route.gate_occurrences.clear();
        route.waiting_zone_occurrences.clear();
        route.next_controlled_transition.clear();
        route.speed_limit_transitions.clear();
        self.route_distance_indices[handle.index()].clear();
        self.route_reference_indices[handle.index()].clear();
        let removed = self.route_handles.swap_remove(&external_id);
        assert_eq!(
            removed,
            Some(handle),
            "route resolver must identify removed route"
        );
        // generation 耗尽时不复用 slot，避免 stale handle 在回绕后复活。
        if let Some(next_generation) = reusable {
            route.generation = next_generation;
            self.free_route_indices.push(handle.index());
        }

        Ok(RouteRemoveRecord {
            handle,
            external_id,
        })
    }

    /// (ParticipantClass, Route) 绑定期静态准入校验（SSOT §6.5）。
    ///
    /// 只覆盖 `cursor` 起的可达后缀：edge 平面查 `[cursor..]` 的 edge，path 平面查
    /// `exit_route_edge_index > cursor` 的 pending occurrence（`cursor < entry` 的未来
    /// occurrence 与 `entry <= cursor < exit` 的进行中 occurrence 都作为原子整体校验；
    /// `cursor == exit` 时 traversal 已完成，exit edge 准入由 edge 平面覆盖）。两平面
    /// 合取：任一平面命中 deny 即原子拒绝，allow 不跨平面解除 deny。成功路径只做
    /// O(1) 查表，不做字符串匹配、层级匹配、组合裁决或 per-vehicle allocation。
    pub(super) fn validate_route_access(
        &self,
        profile: VehicleProfileHandle,
        route: RouteHandle,
        cursor: usize,
    ) -> Result<(), CoreError> {
        let class = self
            .vehicle_profile(profile)
            .expect("validated profile must exist")
            .participant_class();
        let route_slot = self
            .route_slot(route)
            .expect("validated route handle must remain active");
        let denied = |plane: &'static str,
                      route_edge_index: usize,
                      target_id: String,
                      rule: AccessRuleHandle| {
            CoreError::RouteAccessDenied {
                profile_id: self
                    .vehicle_profiles
                    .profile_external_id(profile)
                    .expect("validated profile must have external ID")
                    .to_owned(),
                route_id: route_slot.external_id.clone(),
                plane,
                route_edge_index,
                target_id,
                rule_id: self
                    .access
                    .rule_external_id(rule)
                    .expect("resolved AccessRule must have external ID")
                    .to_owned(),
            }
        };
        for (route_edge_index, edge) in route_slot
            .edge_handles
            .iter()
            .copied()
            .enumerate()
            .skip(cursor)
        {
            if let AccessCell::Decided {
                rule,
                effect: AccessEffect::Deny,
            } = self.access.edge_access(edge, class)
            {
                return Err(denied(
                    "edge",
                    route_edge_index,
                    self.lane_graph
                        .edge_external_id(edge)
                        .expect("normalized route edge must exist")
                        .to_owned(),
                    rule,
                ));
            }
        }
        for occurrence in &route_slot.maneuver_occurrences {
            if occurrence.exit_route_edge_index() <= cursor {
                continue;
            }
            if let AccessCell::Decided {
                rule,
                effect: AccessEffect::Deny,
            } = self.access.path_access(occurrence.maneuver_path(), class)
            {
                return Err(denied(
                    "path",
                    occurrence.entry_route_edge_index(),
                    self.junctions
                        .maneuver_path_external_id(occurrence.maneuver_path())
                        .expect("normalized occurrence ManeuverPath must exist")
                        .to_owned(),
                    rule,
                ));
            }
        }
        Ok(())
    }

    pub(super) fn validate_route_assignment(
        &self,
        profile: VehicleProfileHandle,
        route: RouteHandle,
        cursor: usize,
    ) -> Result<(), CoreError> {
        // Binding validation order is contractual: Access first, profile-specific
        // static feasibility second, then stateful bootstrap/runtime capability.
        self.validate_route_access(profile, route, cursor)?;
        self.validate_waiting_zone_static_feasibility(profile, route, cursor)?;
        self.validate_stateful_maneuver_bootstrap(route, cursor)?;
        self.validate_waiting_zone_runtime_capability(route, cursor)
    }

    pub(super) fn waiting_zone_occurrence_is_pending(
        occurrence: WaitingZoneOccurrence,
        cursor: usize,
    ) -> bool {
        cursor <= occurrence.release_route_edge_index()
    }

    pub(super) fn validate_waiting_zone_static_feasibility(
        &self,
        profile: VehicleProfileHandle,
        route: RouteHandle,
        cursor: usize,
    ) -> Result<(), CoreError> {
        let route_slot = self
            .route_slot(route)
            .expect("validated route handle must remain active");
        let vehicle_profile = self
            .vehicle_profile(profile)
            .expect("validated profile must exist");
        let required_meters = vehicle_profile.iidm().length;
        let distance_index = &self.route_distance_indices[route.index()];
        for occurrence in &route_slot.waiting_zone_occurrences {
            if !Self::waiting_zone_occurrence_is_pending(*occurrence, cursor) {
                continue;
            }
            let storage_start_route_edge_index = occurrence.entry_route_edge_index() + 1;
            let release_route_edge_index = occurrence.release_route_edge_index();
            let release_edge = route_slot.edge_handles[release_route_edge_index];
            let release_edge_length = self
                .lane_graph
                .edge_length(release_edge)
                .expect("compiled route edge must exist")
                .value();
            let waiting_zone_id = self
                .waiting
                .waiting_zone_external_id(occurrence.waiting_zone())
                .expect("compiled WaitingZone occurrence must exist");
            let available_meters = match distance_index
                .finite_distance(
                    storage_start_route_edge_index,
                    0.0,
                    release_route_edge_index,
                    release_edge_length,
                )
                .expect("WaitingZone entry must precede its release boundary")
            {
                BoundedDistance::Finite(available_meters) => available_meters,
                BoundedDistance::BeyondFinite => {
                    return Err(WaitingZoneError::StorageDistanceUnprovable {
                        profile_id: vehicle_profile.external_id().to_owned(),
                        route_id: route_slot.external_id.clone(),
                        waiting_zone_id: waiting_zone_id.to_owned(),
                        entry_route_edge_index: occurrence.entry_route_edge_index(),
                        release_route_edge_index,
                    }
                    .into());
                }
            };
            if available_meters < required_meters
                && required_meters - available_meters > WAITING_ZONE_STORAGE_TOLERANCE_METERS
            {
                return Err(WaitingZoneError::InsufficientStorage {
                    profile_id: vehicle_profile.external_id().to_owned(),
                    route_id: route_slot.external_id.clone(),
                    waiting_zone_id: waiting_zone_id.to_owned(),
                    available_meters,
                    required_meters,
                }
                .into());
            }
        }
        Ok(())
    }

    pub(super) fn validate_stateful_maneuver_bootstrap(
        &self,
        route: RouteHandle,
        cursor: usize,
    ) -> Result<(), CoreError> {
        let route_slot = self
            .route_slot(route)
            .expect("validated route handle must remain active");
        for maneuver in &route_slot.maneuver_occurrences {
            let gate_range = maneuver.gate_occurrence_range();
            let waiting_zone_range = maneuver.waiting_zone_occurrence_range();
            let is_stateful = gate_range.len() > 1 || !waiting_zone_range.is_empty();
            if !is_stateful {
                continue;
            }
            let first_gate = route_slot.gate_occurrences[gate_range.start];
            if cursor > first_gate.from_route_edge_index()
                && cursor < maneuver.exit_route_edge_index()
            {
                return Err(CoreError::StatefulManeuverBootstrapUnavailable {
                    route_id: route_slot.external_id.clone(),
                    maneuver_path_id: self
                        .junctions
                        .maneuver_path_external_id(maneuver.maneuver_path())
                        .expect("compiled ManeuverPath occurrence must exist")
                        .to_owned(),
                    first_gate_route_edge_index: first_gate.from_route_edge_index(),
                    exit_route_edge_index: maneuver.exit_route_edge_index(),
                    cursor,
                });
            }
        }
        Ok(())
    }

    pub(super) fn validate_waiting_zone_runtime_capability(
        &self,
        route: RouteHandle,
        cursor: usize,
    ) -> Result<(), CoreError> {
        let route_slot = self
            .route_slot(route)
            .expect("validated route handle must remain active");
        if let Some(occurrence) = route_slot
            .waiting_zone_occurrences
            .iter()
            .find(|occurrence| Self::waiting_zone_occurrence_is_pending(**occurrence, cursor))
        {
            return Err(WaitingZoneError::RuntimeUnavailable {
                route_id: route_slot.external_id.clone(),
                waiting_zone_id: self
                    .waiting
                    .waiting_zone_external_id(occurrence.waiting_zone())
                    .expect("compiled WaitingZone occurrence must exist")
                    .to_owned(),
            }
            .into());
        }
        Ok(())
    }

}
