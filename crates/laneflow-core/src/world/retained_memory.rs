use super::*;

impl RouteReferenceIndex {
    fn retained_bytes(&self) -> usize {
        let Self { by_update_position } = self;
        by_update_position.capacity() * std::mem::size_of::<(usize, VehicleHandle)>()
    }
}

#[derive(Clone, Copy, Debug)]
struct RouteSlotRetainedStats {
    total_bytes: usize,
    maneuver_occurrence_bytes: usize,
    gate_occurrence_bytes: usize,
    waiting_zone_occurrence_bytes: usize,
}

impl RouteSlot {
    fn retained_stats(&self) -> RouteSlotRetainedStats {
        let Self {
            generation: _,
            external_id,
            edge_handles,
            transitions,
            maneuver_occurrences,
            gate_occurrences,
            waiting_zone_occurrences,
            next_controlled_transition,
            speed_limit_transitions,
            active: _,
        } = self;
        let maneuver_occurrence_bytes =
            maneuver_occurrences.capacity() * std::mem::size_of::<ManeuverOccurrence>();
        let gate_occurrence_bytes =
            gate_occurrences.capacity() * std::mem::size_of::<GateOccurrence>();
        let waiting_zone_occurrence_bytes =
            waiting_zone_occurrences.capacity() * std::mem::size_of::<WaitingZoneOccurrence>();
        let total_bytes = external_id.capacity()
            + edge_handles.capacity() * std::mem::size_of::<EdgeHandle>()
            + transitions.capacity() * std::mem::size_of::<RouteTransition>()
            + maneuver_occurrence_bytes
            + gate_occurrence_bytes
            + waiting_zone_occurrence_bytes
            + next_controlled_transition.capacity()
                * std::mem::size_of::<Option<NextControlledRouteTransition>>()
            + speed_limit_transitions.capacity() * std::mem::size_of::<SpeedLimitRouteTransition>();
        RouteSlotRetainedStats {
            total_bytes,
            maneuver_occurrence_bytes,
            gate_occurrence_bytes,
            waiting_zone_occurrence_bytes,
        }
    }
}

impl VehicleSlot {
    fn retained_bytes(&self) -> usize {
        let Self {
            generation: _,
            external_id,
            state: _,
            update_order_position: _,
        } = self;
        external_id.capacity()
    }
}

impl StableVehicleOrder {
    fn retained_bytes(&self) -> usize {
        let Self {
            entries,
            tombstones: _,
        } = self;
        entries.capacity() * std::mem::size_of::<Option<VehicleHandle>>()
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CompleteRetainedComponents {
    lane_graph_bytes: usize,
    vehicle_profile_registry_bytes: usize,
    junction_registry_bytes: usize,
    signal_registry_bytes: usize,
    signal_runtime_state_bytes: usize,
    signal_runtime_scratch_bytes: usize,
    participant_class_registry_bytes: usize,
    cross_section_registry_bytes: usize,
    access_registry_bytes: usize,
    waiting_registry_bytes: usize,
    route_bytes: usize,
    vehicle_bytes: usize,
    resolver_bytes: usize,
    free_list_bytes: usize,
    vehicle_order_bytes: usize,
    candidate_state_bytes: usize,
    parking_registry_runtime_bytes: usize,
    occupancy_scratch_bytes: usize,
    longitudinal_scratch_bytes: usize,
    command_spatial_bytes: usize,
}

impl CompleteRetainedComponents {
    pub(super) fn owned_heap_bytes(self) -> usize {
        let Self {
            lane_graph_bytes,
            vehicle_profile_registry_bytes,
            junction_registry_bytes,
            signal_registry_bytes,
            signal_runtime_state_bytes,
            signal_runtime_scratch_bytes,
            participant_class_registry_bytes,
            cross_section_registry_bytes,
            access_registry_bytes,
            waiting_registry_bytes,
            route_bytes,
            vehicle_bytes,
            resolver_bytes,
            free_list_bytes,
            vehicle_order_bytes,
            candidate_state_bytes,
            parking_registry_runtime_bytes,
            occupancy_scratch_bytes,
            longitudinal_scratch_bytes,
            command_spatial_bytes,
        } = self;
        lane_graph_bytes
            + vehicle_profile_registry_bytes
            + junction_registry_bytes
            + signal_registry_bytes
            + signal_runtime_state_bytes
            + signal_runtime_scratch_bytes
            + participant_class_registry_bytes
            + cross_section_registry_bytes
            + access_registry_bytes
            + waiting_registry_bytes
            + route_bytes
            + vehicle_bytes
            + resolver_bytes
            + free_list_bytes
            + vehicle_order_bytes
            + candidate_state_bytes
            + parking_registry_runtime_bytes
            + occupancy_scratch_bytes
            + longitudinal_scratch_bytes
            + command_spatial_bytes
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct LifecycleRetainedStats {
    pub(super) accounted_bytes: usize,
    pub(super) expanded_accounted_bytes: usize,
    pub(super) complete_accounted_bytes: usize,
    pub(super) owned_heap_bytes: usize,
    pub(super) world_inline_bytes: usize,
    pub(super) lane_graph_bytes: usize,
    pub(super) vehicle_profile_registry_bytes: usize,
    pub(super) junction_registry_bytes: usize,
    pub(super) signal_registry_bytes: usize,
    pub(super) signal_runtime_state_bytes: usize,
    pub(super) signal_runtime_scratch_bytes: usize,
    pub(super) participant_class_registry_bytes: usize,
    pub(super) cross_section_registry_bytes: usize,
    pub(super) access_registry_bytes: usize,
    pub(super) waiting_registry_bytes: usize,
    pub(super) route_bytes: usize,
    pub(super) route_maneuver_occurrence_bytes: usize,
    pub(super) route_gate_occurrence_bytes: usize,
    pub(super) route_waiting_zone_occurrence_bytes: usize,
    pub(super) route_distance_bytes: usize,
    pub(super) route_reference_bytes: usize,
    pub(super) vehicle_bytes: usize,
    pub(super) resolver_bytes: usize,
    pub(super) free_list_bytes: usize,
    pub(super) vehicle_order_bytes: usize,
    pub(super) candidate_state_bytes: usize,
    pub(super) parking_bytes: usize,
    pub(super) parking_registry_runtime_bytes: usize,
    pub(super) occupancy_scratch_bytes: usize,
    pub(super) longitudinal_scratch_bytes: usize,
    pub(super) command_spatial_bytes: usize,
    pub(super) lane_graph_inline_size: usize,
    pub(super) vehicle_profile_registry_inline_size: usize,
    pub(super) junction_registry_inline_size: usize,
    pub(super) signal_registry_inline_size: usize,
    pub(super) signal_runtime_state_inline_size: usize,
    pub(super) signal_runtime_scratch_inline_size: usize,
    pub(super) vehicle_state_size: usize,
    pub(super) vehicle_slot_size: usize,
    pub(super) live_vehicles: usize,
    pub(super) route_occurrences: usize,
    pub(super) tombstones: usize,
    pub(super) route_candidate_nodes: usize,
    pub(super) stale_route_candidate_nodes: usize,
    pub(super) spatial_occupants: usize,
}

impl LifecycleRetainedStats {
    pub(super) fn complete_components(self) -> CompleteRetainedComponents {
        CompleteRetainedComponents {
            lane_graph_bytes: self.lane_graph_bytes,
            vehicle_profile_registry_bytes: self.vehicle_profile_registry_bytes,
            junction_registry_bytes: self.junction_registry_bytes,
            signal_registry_bytes: self.signal_registry_bytes,
            signal_runtime_state_bytes: self.signal_runtime_state_bytes,
            signal_runtime_scratch_bytes: self.signal_runtime_scratch_bytes,
            participant_class_registry_bytes: self.participant_class_registry_bytes,
            cross_section_registry_bytes: self.cross_section_registry_bytes,
            access_registry_bytes: self.access_registry_bytes,
            waiting_registry_bytes: self.waiting_registry_bytes,
            route_bytes: self.route_bytes,
            vehicle_bytes: self.vehicle_bytes,
            resolver_bytes: self.resolver_bytes,
            free_list_bytes: self.free_list_bytes,
            vehicle_order_bytes: self.vehicle_order_bytes,
            candidate_state_bytes: self.candidate_state_bytes,
            parking_registry_runtime_bytes: self.parking_registry_runtime_bytes,
            occupancy_scratch_bytes: self.occupancy_scratch_bytes,
            longitudinal_scratch_bytes: self.longitudinal_scratch_bytes,
            command_spatial_bytes: self.command_spatial_bytes,
        }
    }
}

impl CandidateStateScratch {
    fn retained_bytes(&self) -> usize {
        let Self {
            states,
            spatial_changes,
            parking_releases,
        } = self;
        states.capacity() * std::mem::size_of::<Option<VehicleState>>()
            + spatial_changes.capacity() * std::mem::size_of::<VehicleHandle>()
            + parking_releases.capacity() * std::mem::size_of::<ParkingStepRelease>()
    }
}

impl CoreWorld {
    pub(super) fn lifecycle_retained_stats(&self) -> LifecycleRetainedStats {
        let Self {
            fixed_delta_time_ms: _,
            tick_index: _,
            time_ms: _,
            lane_graph,
            vehicle_profiles,
            junctions,
            signals,
            parking,
            participant_classes,
            cross_section,
            access,
            waiting,
            parking_runtime,
            signal_state,
            signal_candidate_scratch,
            routes,
            route_distance_indices,
            route_reference_indices,
            route_handles,
            free_route_indices,
            vehicles,
            vehicle_handles,
            free_vehicle_indices,
            vehicle_update_order,
            candidate_state_scratch,
            occupancy_scratch,
            longitudinal_scratch,
            command_spatial_index,
            step_failure_after_vehicle: _,
            replace_failure_after_prepare: _,
        } = self;

        let route_occurrences = routes
            .iter()
            .filter(|route| route.active)
            .map(|route| route.edge_handles.len())
            .sum();
        let route_candidate_nodes = route_reference_indices
            .iter()
            .map(RouteReferenceIndex::live_count)
            .sum();
        let stale_route_candidate_nodes = 0;
        let route_distance_bytes = route_distance_indices.capacity()
            * std::mem::size_of::<RouteDistanceIndex>()
            + route_distance_indices
                .iter()
                .map(RouteDistanceIndex::retained_bytes)
                .sum::<usize>();
        let route_reference_bytes = route_reference_indices.capacity()
            * std::mem::size_of::<RouteReferenceIndex>()
            + route_reference_indices
                .iter()
                .map(RouteReferenceIndex::retained_bytes)
                .sum::<usize>();
        let (
            route_slot_bytes,
            route_maneuver_occurrence_bytes,
            route_gate_occurrence_bytes,
            route_waiting_zone_occurrence_bytes,
        ) = routes.iter().map(RouteSlot::retained_stats).fold(
            (0_usize, 0_usize, 0_usize, 0_usize),
            |(total, maneuver, gate, waiting), stats| {
                (
                    total + stats.total_bytes,
                    maneuver + stats.maneuver_occurrence_bytes,
                    gate + stats.gate_occurrence_bytes,
                    waiting + stats.waiting_zone_occurrence_bytes,
                )
            },
        );
        let route_bytes = routes.capacity() * std::mem::size_of::<RouteSlot>()
            + route_slot_bytes
            + route_distance_bytes
            + route_reference_bytes;
        let vehicle_bytes = vehicles.capacity() * std::mem::size_of::<VehicleSlot>()
            + vehicles
                .iter()
                .map(VehicleSlot::retained_bytes)
                .sum::<usize>();
        let resolver_bytes = route_handles.capacity()
            * std::mem::size_of::<(String, RouteHandle)>()
            + route_handles.keys().map(String::capacity).sum::<usize>()
            + vehicle_handles.capacity() * std::mem::size_of::<(String, VehicleHandle)>()
            + vehicle_handles.keys().map(String::capacity).sum::<usize>();
        let free_list_bytes = free_route_indices.capacity() * std::mem::size_of::<usize>()
            + free_vehicle_indices.capacity() * std::mem::size_of::<usize>();
        let vehicle_order_bytes = vehicle_update_order.retained_bytes();
        let candidate_state_bytes = candidate_state_scratch.retained_bytes();
        let lifecycle_scratch_bytes = free_list_bytes + vehicle_order_bytes + candidate_state_bytes;
        let parking_registry_runtime_bytes =
            parking.retained_bytes() + parking_runtime.retained_bytes();
        let parking_bytes =
            parking_registry_runtime_bytes + longitudinal_scratch.parking_retained_bytes();
        let world_inline_bytes = std::mem::size_of::<Self>();
        let complete_components = CompleteRetainedComponents {
            lane_graph_bytes: lane_graph.retained_bytes(),
            vehicle_profile_registry_bytes: vehicle_profiles.retained_bytes(),
            junction_registry_bytes: junctions.retained_bytes(),
            signal_registry_bytes: signals.retained_bytes(),
            signal_runtime_state_bytes: signal_state.retained_bytes(),
            signal_runtime_scratch_bytes: signal_candidate_scratch.retained_bytes(),
            participant_class_registry_bytes: participant_classes.retained_bytes(),
            cross_section_registry_bytes: cross_section.retained_bytes(),
            access_registry_bytes: access.retained_bytes(),
            waiting_registry_bytes: waiting.retained_bytes(),
            route_bytes,
            vehicle_bytes,
            resolver_bytes,
            free_list_bytes,
            vehicle_order_bytes,
            candidate_state_bytes,
            parking_registry_runtime_bytes,
            occupancy_scratch_bytes: occupancy_scratch.retained_bytes(),
            longitudinal_scratch_bytes: longitudinal_scratch.retained_bytes(),
            command_spatial_bytes: command_spatial_index.retained_bytes(),
        };
        let accounted_bytes = world_inline_bytes
            + route_bytes
            + vehicle_bytes
            + resolver_bytes
            + lifecycle_scratch_bytes
            + parking_bytes
            + complete_components.command_spatial_bytes;
        let expanded_accounted_bytes = world_inline_bytes
            + route_bytes
            + vehicle_bytes
            + resolver_bytes
            + lifecycle_scratch_bytes
            + parking_registry_runtime_bytes
            + complete_components.occupancy_scratch_bytes
            + complete_components.longitudinal_scratch_bytes
            + complete_components.command_spatial_bytes;
        let owned_heap_bytes = complete_components.owned_heap_bytes();
        let complete_accounted_bytes = world_inline_bytes + owned_heap_bytes;
        LifecycleRetainedStats {
            accounted_bytes,
            expanded_accounted_bytes,
            complete_accounted_bytes,
            owned_heap_bytes,
            world_inline_bytes,
            lane_graph_bytes: complete_components.lane_graph_bytes,
            vehicle_profile_registry_bytes: complete_components.vehicle_profile_registry_bytes,
            junction_registry_bytes: complete_components.junction_registry_bytes,
            signal_registry_bytes: complete_components.signal_registry_bytes,
            signal_runtime_state_bytes: complete_components.signal_runtime_state_bytes,
            signal_runtime_scratch_bytes: complete_components.signal_runtime_scratch_bytes,
            participant_class_registry_bytes: complete_components.participant_class_registry_bytes,
            cross_section_registry_bytes: complete_components.cross_section_registry_bytes,
            access_registry_bytes: complete_components.access_registry_bytes,
            waiting_registry_bytes: complete_components.waiting_registry_bytes,
            route_bytes,
            route_maneuver_occurrence_bytes,
            route_gate_occurrence_bytes,
            route_waiting_zone_occurrence_bytes,
            route_distance_bytes,
            route_reference_bytes,
            vehicle_bytes,
            resolver_bytes,
            free_list_bytes,
            vehicle_order_bytes,
            candidate_state_bytes,
            parking_bytes,
            parking_registry_runtime_bytes,
            occupancy_scratch_bytes: complete_components.occupancy_scratch_bytes,
            longitudinal_scratch_bytes: complete_components.longitudinal_scratch_bytes,
            command_spatial_bytes: complete_components.command_spatial_bytes,
            lane_graph_inline_size: std::mem::size_of::<LaneGraph>(),
            vehicle_profile_registry_inline_size: std::mem::size_of::<VehicleProfileRegistry>(),
            junction_registry_inline_size: std::mem::size_of::<JunctionRegistry>(),
            signal_registry_inline_size: std::mem::size_of::<SignalRegistry>(),
            signal_runtime_state_inline_size: std::mem::size_of::<SignalRuntimeState>(),
            signal_runtime_scratch_inline_size: std::mem::size_of::<SignalRuntimeScratch>(),
            vehicle_state_size: std::mem::size_of::<VehicleState>(),
            vehicle_slot_size: std::mem::size_of::<VehicleSlot>(),
            live_vehicles: vehicle_update_order.iter().count(),
            route_occurrences,
            tombstones: vehicle_update_order.tombstones,
            route_candidate_nodes,
            stale_route_candidate_nodes,
            spatial_occupants: command_spatial_index.occupant_count(),
        }
    }
}
