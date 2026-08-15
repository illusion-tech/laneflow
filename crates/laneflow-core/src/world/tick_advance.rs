use super::*;

impl CoreWorld {
    pub(super) fn advance_vehicle<const PARKING_ACTIVE: bool>(
        context: VehicleAdvanceContext<'_>,
        vehicle: &mut VehicleState,
        motion: LongitudinalMotion,
        parking_stop: Option<ParkingStopConstraint>,
        events: &mut Vec<CoreEvent>,
        spatial_changes: &mut Vec<VehicleHandle>,
    ) -> Result<Option<VehicleCompletedRouteEvent>, CoreError> {
        if vehicle.status != VehicleStatus::Active {
            return Ok(None);
        }

        let delta_time_seconds = context.fixed_delta_time_ms as f64 / 1_000.0;
        vehicle.current_speed =
            Speed::try_new(motion.final_speed()).expect("longitudinal motion speed must be valid");
        vehicle.applied_acceleration =
            Acceleration::try_new(motion.applied_acceleration(delta_time_seconds)?)
                .expect("longitudinal applied acceleration must be valid");
        if let Some(speed_limit) = motion.speed_limit_projection() {
            events.push(CoreEvent::VehicleSpeedLimitProjectionApplied(
                VehicleSpeedLimitProjectionAppliedEvent {
                    tick_index: context.tick_index,
                    vehicle: vehicle.handle,
                    route: speed_limit.route,
                    from_route_edge_index: speed_limit.from_route_edge_index,
                    to_route_edge_index: speed_limit.to_route_edge_index,
                    from_edge: speed_limit.from_edge,
                    to_edge: speed_limit.to_edge,
                },
            ));
        } else if let Some(signal_stop) = motion.signal_stop_projection() {
            events.push(CoreEvent::VehicleSignalStopProjectionApplied(
                VehicleSignalStopProjectionAppliedEvent {
                    tick_index: context.tick_index,
                    vehicle: vehicle.handle,
                    route: vehicle.route,
                    from_route_edge_index: signal_stop.from_route_edge_index,
                    to_route_edge_index: signal_stop.to_route_edge_index,
                    gate: signal_stop.gate,
                    stop_line: signal_stop.stop_line,
                    group: signal_stop.group,
                    aspect: signal_stop.aspect,
                },
            ));
        } else if PARKING_ACTIVE && motion.parking_stop_projection() {
            let parking_stop = parking_stop.expect("Parking projection must resolve sparse target");
            events.push(CoreEvent::VehicleParkingStopProjectionApplied(
                VehicleParkingStopProjectionAppliedEvent {
                    tick_index: context.tick_index,
                    vehicle: vehicle.handle,
                    space: parking_stop.space,
                    route: parking_stop.route,
                    route_edge_index: parking_stop.route_edge_index,
                },
            ));
        }
        if let Some(leader) = motion.safety_projection_leader() {
            events.push(CoreEvent::VehicleFollowingSafetyProjectionApplied(
                VehicleFollowingSafetyProjectionAppliedEvent {
                    tick_index: context.tick_index,
                    vehicle: vehicle.handle,
                    leader,
                },
            ));
        }

        let travel_distance = motion.final_travel();
        if travel_distance <= EDGE_BOUNDARY_TOLERANCE_METERS
            && !motion.reaches_route_end()
            && !(PARKING_ACTIVE
                && parking_stop.is_some_and(|constraint| motion.reaches_parking_stop(constraint)))
        {
            return Ok(None);
        }

        let route = context
            .routes
            .get(vehicle.route.index())
            .filter(|route| route.active && route.generation == vehicle.route.generation())
            .expect("validated vehicle route must exist");
        let max_iterations = route.edge_handles.len() - vehicle.route_edge_index;
        let mut remaining = travel_distance;
        let mut completed_event = None;

        for _ in 0..max_iterations {
            if is_edge_boundary_remainder_zero(remaining) {
                if motion.reaches_route_end()
                    && vehicle.route_edge_index + 1 == route.edge_handles.len()
                {
                    let current_edge = route.edge_handles[vehicle.route_edge_index];
                    let edge_length = context
                        .lane_graph
                        .edge_length(current_edge)
                        .expect("validated route edge must exist")
                        .value();
                    vehicle.edge_progress =
                        EdgeProgress::try_new(edge_length).expect("edge length is valid progress");
                    vehicle.current_speed = Speed::ZERO;
                    vehicle.applied_acceleration = Acceleration::ZERO;
                    vehicle.status = VehicleStatus::Completed;
                    if spatial_changes.last().copied() != Some(vehicle.handle) {
                        spatial_changes.push(vehicle.handle);
                    }
                    completed_event = Some(VehicleCompletedRouteEvent {
                        tick_index: context.tick_index,
                        vehicle: vehicle.handle,
                        route: vehicle.route,
                        edge: current_edge,
                        route_edge_index: vehicle.route_edge_index,
                    });
                }
                break;
            }

            let current_edge = route
                .edge_handles
                .get(vehicle.route_edge_index)
                .copied()
                .expect("validated route edge index must exist");
            let edge_length = context
                .lane_graph
                .edge_length(current_edge)
                .expect("validated route edge must exist")
                .value();
            let next_progress = vehicle.edge_progress.value() + remaining;
            if !next_progress.is_finite() {
                return Err(CoreError::NonFiniteRouteTravel {
                    vehicle: vehicle.handle,
                    speed: motion.final_speed(),
                    delta_time_ms: context.fixed_delta_time_ms,
                });
            }

            if PARKING_ACTIVE
                && let Some(stop) = parking_stop
                && stop.route == vehicle.route
                && stop.route_edge_index == vehicle.route_edge_index
            {
                let crosses_boundary =
                    next_progress > stop.entry_progress + LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS;
                let reaches_boundary =
                    longitudinal_constraint_reached(next_progress, stop.entry_progress);
                if crosses_boundary
                    || (reaches_boundary
                        && computed_speed_is_above_near_zero(vehicle.current_speed.value()))
                {
                    return Err(CoreError::ParkingTraversalBoundaryInvariant {
                        vehicle: vehicle.handle,
                        space: stop.space,
                        route: stop.route,
                        route_edge_index: stop.route_edge_index,
                        remaining_travel: (next_progress - stop.entry_progress).max(0.0),
                        final_speed: vehicle.current_speed.value(),
                    });
                }
                if reaches_boundary {
                    vehicle.edge_progress = EdgeProgress::try_new(stop.entry_progress)
                        .expect("normalized Parking entry progress must be valid");
                    vehicle.current_speed = Speed::ZERO;
                    vehicle.applied_acceleration = Acceleration::ZERO;
                    break;
                }
            }

            if next_progress + EDGE_BOUNDARY_TOLERANCE_METERS < edge_length {
                vehicle.edge_progress =
                    EdgeProgress::try_new(next_progress).expect("progress remains valid");
                break;
            }

            let remainder = next_progress - edge_length;
            remaining = if is_edge_boundary_remainder_zero(remainder) {
                0.0
            } else {
                remainder
            };

            if vehicle.route_edge_index + 1 < route.edge_handles.len() {
                let from_route_edge_index = vehicle.route_edge_index;
                let to_route_edge_index = from_route_edge_index + 1;
                let transition = route
                    .transitions
                    .get(from_route_edge_index)
                    .copied()
                    .expect("next route transition must exist");
                let to_edge = transition.to_edge;

                let target_limit = context
                    .lane_graph
                    .edge_speed_limit(to_edge)
                    .expect("validated route edge must exist")
                    .value();
                if vehicle.current_speed.value() > target_limit {
                    return Err(CoreError::SpeedLimitTraversalInvariant {
                        vehicle: vehicle.handle,
                        route: vehicle.route,
                        from_route_edge_index,
                        to_route_edge_index,
                        from_edge: current_edge,
                        to_edge,
                        final_speed: vehicle.current_speed.value(),
                        target_limit,
                    });
                }

                let denied_gate = transition.gate.and_then(|gate_handle| {
                    let gate = context
                        .signals
                        .maneuver_gate_state_by_handle(context.signal_state, gate_handle)
                        .expect("normalized route Gate must have committed state");
                    matches!(
                        gate.signal(),
                        ManeuverGateSignalState::Controlled {
                            permission: SignalLayerPermission::DenyAndStop,
                            ..
                        }
                    )
                    .then_some(gate)
                });
                if let Some(gate) = denied_gate {
                    if remaining > EDGE_BOUNDARY_TOLERANCE_METERS
                        || computed_speed_is_above_near_zero(vehicle.current_speed.value())
                    {
                        return Err(CoreError::SignalTraversalDeniedInvariant {
                            vehicle: vehicle.handle,
                            route: vehicle.route,
                            from_route_edge_index,
                            to_route_edge_index,
                            gate: gate.gate(),
                            remaining_travel: remaining,
                            final_speed: vehicle.current_speed.value(),
                        });
                    }
                    vehicle.edge_progress =
                        EdgeProgress::try_new(edge_length).expect("edge length is valid progress");
                    break;
                }

                if current_edge != to_edge
                    && spatial_changes.last().copied() != Some(vehicle.handle)
                {
                    spatial_changes.push(vehicle.handle);
                }

                events.push(CoreEvent::VehicleChangedEdge(VehicleChangedEdgeEvent {
                    tick_index: context.tick_index,
                    vehicle: vehicle.handle,
                    route: vehicle.route,
                    from_edge: current_edge,
                    to_edge,
                    from_route_edge_index,
                    to_route_edge_index,
                }));

                vehicle.route_edge_index = to_route_edge_index;
                vehicle.edge_progress = EdgeProgress::ZERO;
            } else {
                vehicle.edge_progress =
                    EdgeProgress::try_new(edge_length).expect("edge length is valid progress");
                vehicle.current_speed = Speed::ZERO;
                vehicle.applied_acceleration = Acceleration::ZERO;
                vehicle.status = VehicleStatus::Completed;
                if spatial_changes.last().copied() != Some(vehicle.handle) {
                    spatial_changes.push(vehicle.handle);
                }
                completed_event = Some(VehicleCompletedRouteEvent {
                    tick_index: context.tick_index,
                    vehicle: vehicle.handle,
                    route: vehicle.route,
                    edge: current_edge,
                    route_edge_index: vehicle.route_edge_index,
                });
                break;
            }
        }

        Ok(completed_event)
    }
}
