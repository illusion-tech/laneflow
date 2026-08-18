use super::*;

impl CoreWorld {
    /// 推进一个 fixed-step tick。
    ///
    /// 成功时，`StepResult` 使用 post-step tick/time。失败时权威 tick/time、vehicle state
    /// 与 events 保持不变；私有派生 scratch 可以重建，且不参与 `CoreWorld` 语义相等。
    pub fn step(&mut self, input: TickInput) -> Result<StepResult, CoreError> {
        self.step_with_probe(input, &mut NoOpProbe)
    }

    /// 推进一个 fixed-step tick，并经由仪器探针边界上报六段计时。
    ///
    /// 生产路径的默认探针为 [`NoOpProbe`]（`StepProbe::ENABLED = false`），空操作实现
    /// 经 monomorphization 与常量折叠整体编译消除；`step` 的行为与签名不变。研究态
    /// 探针实现（如 `instrumentation` feature 或 crate 内测试的记录探针）经此入口注入。
    /// `inline(always)` 保证 `step` 的 `NoOpProbe` 委托在发布构建不引入调用间接。
    #[doc(hidden)]
    #[inline(always)]
    pub fn step_with_probe<P: StepProbe>(
        &mut self,
        input: TickInput,
        probe: &mut P,
    ) -> Result<StepResult, CoreError> {
        if input.delta_time_ms != self.fixed_delta_time_ms {
            return Err(CoreError::TickDeltaMismatch {
                expected_delta_time_ms: self.fixed_delta_time_ms,
                actual_delta_time_ms: input.delta_time_ms,
            });
        }

        let next_tick_index = self
            .tick_index
            .checked_add(1)
            .ok_or(CoreError::TimeOverflow)?;
        let next_time_ms = self
            .time_ms
            .checked_add(self.fixed_delta_time_ms)
            .ok_or(CoreError::TimeOverflow)?;

        self.step_inner(next_tick_index, next_time_ms, probe)
            .map_err(|error| self.expand_tick_invariant_error(error))
    }

    fn step_inner<P: StepProbe>(
        &mut self,
        next_tick_index: u64,
        next_time_ms: u64,
        probe: &mut P,
    ) -> Result<StepResult, TickInvariantError> {
        self.parking_runtime.validate_step_sentinel(&self.parking)?;
        // `ParkingVehicleCapabilityUnavailable` 是 #108 过渡期保留的 public variant。
        // #109 的完整 ParkingStop/arrival/release pipeline 激活后，合法 world 不再返回它。

        let mut signal_candidate_scratch = std::mem::take(&mut self.signal_candidate_scratch);
        self.signals
            .populate_runtime_state(next_time_ms, signal_candidate_scratch.state_mut());

        let occupancy_started = P::ENABLED.then(std::time::Instant::now);
        if let Err(error) = self.rebuild_occupancy_and_leaders() {
            self.signal_candidate_scratch = signal_candidate_scratch;
            return Err(error);
        }
        if let Some(started) = occupancy_started {
            probe.note_occupancy_duration(started.elapsed());
        }
        if let Err(error) = self.rebuild_longitudinal_motions(probe) {
            self.signal_candidate_scratch = signal_candidate_scratch;
            return Err(error);
        }
        let post_longitudinal_started = P::ENABLED.then(std::time::Instant::now);

        let mut candidate_states = std::mem::take(&mut self.candidate_state_scratch);
        candidate_states.begin(&self.vehicles);
        let mut events = Vec::new();
        let mut candidate_max_vehicle_speed = 0.0_f64;
        let advance_result = if self.parking_runtime.reserved_count() > 0 {
            self.advance_all_vehicles::<true>(
                &mut candidate_states,
                &mut events,
                next_tick_index,
                &mut candidate_max_vehicle_speed,
            )
        } else {
            self.advance_all_vehicles::<false>(
                &mut candidate_states,
                &mut events,
                next_tick_index,
                &mut candidate_max_vehicle_speed,
            )
        };

        if let Err(error) = advance_result {
            candidate_states.clear();
            self.candidate_state_scratch = candidate_states;
            self.signal_candidate_scratch = signal_candidate_scratch;
            return Err(error);
        }

        let invalid_release = candidate_states
            .parking_releases
            .iter()
            .copied()
            .find(|release| {
                !self
                    .parking_runtime
                    .validate_reserved_pair(release.vehicle, release.space)
            });
        if let Some(release) = invalid_release {
            candidate_states.clear();
            self.candidate_state_scratch = candidate_states;
            self.signal_candidate_scratch = signal_candidate_scratch;
            return Err(TickInvariantError::ParkingBindingInvariantViolation {
                stage: "step_completion_release_validate",
                vehicle: Some(release.vehicle),
                space: Some(release.space),
            });
        }

        self.append_signal_events(
            next_tick_index,
            signal_candidate_scratch.state(),
            &mut events,
        );
        self.sync_changed_command_spatial_memberships(&candidate_states);
        self.command_spatial_index
            .set_max_vehicle_speed(candidate_max_vehicle_speed);
        for release in &candidate_states.parking_releases {
            let applied = self.parking_runtime.release(&self.parking, release.vehicle);
            assert_eq!(
                applied,
                Some((release.space, ParkingBindingKind::Reserved)),
                "validated completion release must commit exact Reserved pair"
            );
        }
        candidate_states.commit_into(&mut self.vehicles);
        std::mem::swap(&mut self.signal_state, signal_candidate_scratch.state_mut());
        self.tick_index = next_tick_index;
        self.time_ms = next_time_ms;
        self.candidate_state_scratch = candidate_states;
        self.signal_candidate_scratch = signal_candidate_scratch;
        if let Some(started) = post_longitudinal_started {
            probe.note_post_longitudinal_duration(started.elapsed());
        }

        Ok(StepResult {
            tick_index: next_tick_index,
            time_ms: next_time_ms,
            events,
        })
    }

    #[cold]
    #[inline(never)]
    pub(super) fn expand_tick_invariant_error(&self, error: TickInvariantError) -> CoreError {
        match error {
            TickInvariantError::NonFiniteParkingComputation {
                stage,
                vehicle,
                space,
                value,
            } => CoreError::NonFiniteParkingComputation {
                stage,
                vehicle,
                space,
                value,
            },
            TickInvariantError::NonFiniteLeaderComputation {
                vehicle,
                stage,
                value,
            } => CoreError::NonFiniteLeaderComputation {
                vehicle,
                stage,
                value,
            },
            TickInvariantError::NonFiniteLongitudinalComputation {
                vehicle,
                stage,
                value,
            } => CoreError::NonFiniteLongitudinalComputation {
                vehicle,
                stage,
                value,
            },
            TickInvariantError::NonFiniteSpeedLimitComputation {
                vehicle,
                stage,
                value,
            } => CoreError::NonFiniteSpeedLimitComputation {
                vehicle,
                stage,
                value,
            },
            TickInvariantError::SpeedLimitTraversalInvariant {
                vehicle,
                route,
                from_route_edge_index,
                to_route_edge_index,
                final_speed,
                target_limit,
            } => {
                let edges = self
                    .route_edges(route)
                    .expect("tick route must remain live");
                CoreError::SpeedLimitTraversalInvariant {
                    vehicle,
                    route,
                    from_route_edge_index,
                    to_route_edge_index,
                    from_edge: edges[from_route_edge_index],
                    to_edge: edges[to_route_edge_index],
                    final_speed,
                    target_limit,
                }
            }
            TickInvariantError::NonFiniteSignalStopComputation {
                vehicle,
                stage,
                value,
            } => CoreError::NonFiniteSignalStopComputation {
                vehicle,
                stage,
                value,
            },
            TickInvariantError::NonFiniteRouteTravel {
                vehicle,
                speed,
                delta_time_ms,
            } => CoreError::NonFiniteRouteTravel {
                vehicle,
                speed,
                delta_time_ms,
            },
            TickInvariantError::SignalTraversalDeniedInvariant {
                vehicle,
                route,
                from_route_edge_index,
                to_route_edge_index,
                gate,
                remaining_travel,
                final_speed,
            } => CoreError::SignalTraversalDeniedInvariant {
                vehicle,
                route,
                from_route_edge_index,
                to_route_edge_index,
                gate,
                remaining_travel,
                final_speed,
            },
            TickInvariantError::ParkingLeaveUnsafeFollower {
                vehicle,
                space,
                follower,
            } => CoreError::ParkingLeaveUnsafeFollower {
                vehicle,
                space,
                follower,
            },
            TickInvariantError::ParkingBindingInvariantViolation {
                stage,
                vehicle,
                space,
            } => CoreError::ParkingBindingInvariantViolation {
                stage,
                vehicle,
                space,
            },
            TickInvariantError::ParkingTraversalBoundaryInvariant {
                vehicle,
                space,
                route,
                route_edge_index,
                remaining_travel,
                final_speed,
            } => CoreError::ParkingTraversalBoundaryInvariant {
                vehicle,
                space,
                route,
                route_edge_index,
                remaining_travel,
                final_speed,
            },
            TickInvariantError::VehiclePhysicalOverlap {
                follower,
                leader,
                bumper_gap,
            } => CoreError::VehiclePhysicalOverlap {
                follower_id: self
                    .vehicle_external_id(follower)
                    .expect("overlap follower must remain live")
                    .to_owned(),
                leader_id: self
                    .vehicle_external_id(leader)
                    .expect("overlap leader must remain live")
                    .to_owned(),
                bumper_gap,
            },
        }
    }

    /// 单一车辆推进循环：无保留停车与保留停车两分支的收敛实现。
    ///
    /// `PARKING_ACTIVE` 为编译期常量泛型：`false` 实例（无保留停车热路径）经
    /// monomorphization 折叠全部停车分支，不引入运行时开销；`true` 实例承载
    /// parking_stops 解析、Reserved binding 上下文与 release/arrival 事件。
    /// first-error 语义、事件顺序、失败原子性与故障注入语义与收敛前一致。
    fn advance_all_vehicles<const PARKING_ACTIVE: bool>(
        &mut self,
        candidate_states: &mut CandidateStateScratch,
        events: &mut Vec<CoreEvent>,
        next_tick_index: u64,
        candidate_max_vehicle_speed: &mut f64,
    ) -> Result<(), TickInvariantError> {
        let advance_context = VehicleAdvanceContext {
            lane_graph: &self.lane_graph,
            signals: &self.signals,
            signal_state: &self.signal_state,
            routes: &self.routes,
            fixed_delta_time_ms: self.fixed_delta_time_ms,
            tick_index: next_tick_index,
        };
        let CandidateStateScratch {
            states,
            spatial_changes,
            parking_releases,
        } = candidate_states;
        let mut parking_stops = PARKING_ACTIVE.then(|| {
            self.longitudinal_scratch
                .parking_stops()
                .iter()
                .copied()
                .peekable()
        });
        for vehicle_handle in self.vehicle_update_order.iter() {
            let Some(current_slot) = self
                .vehicles
                .get(vehicle_handle.index())
                .filter(|slot| slot.generation == vehicle_handle.generation())
            else {
                continue;
            };
            if current_slot.state.is_none() {
                continue;
            }
            let Some(motion) = self.longitudinal_scratch.motion(vehicle_handle) else {
                debug_assert!(matches!(
                    current_slot.state.as_ref().map(|state| state.status),
                    Some(VehicleStatus::Completed | VehicleStatus::Parked)
                ));
                continue;
            };
            let parking_stop = if PARKING_ACTIVE {
                parking_stops
                    .as_mut()
                    .expect("PARKING_ACTIVE keeps parking_stops")
                    .peek()
                    .filter(|stop| stop.vehicle == vehicle_handle)
                    .map(|stop| stop.constraint)
            } else {
                None
            };
            if PARKING_ACTIVE && parking_stop.is_some() {
                parking_stops
                    .as_mut()
                    .expect("PARKING_ACTIVE keeps parking_stops")
                    .next();
            }
            let parking_binding = PARKING_ACTIVE
                .then(|| self.parking_runtime.vehicle_binding(vehicle_handle))
                .flatten();
            let reaches_parking_stop =
                parking_stop.is_some_and(|constraint| motion.reaches_parking_stop(constraint));
            let (reserved_space, reserved_target, entry_progress, was_arrived) = if PARKING_ACTIVE {
                match parking_binding {
                    Some(RuntimeVehicleParkingBinding::Reserved { space, target, .. }) => {
                        let entry_progress = reaches_parking_stop.then(|| {
                            self.parking
                                .space_entry(space)
                                .expect("reserved ParkingSpace must have entry")
                                .progress()
                        });
                        let was_arrived = current_slot.state.as_ref().is_some_and(|state| {
                            parking_arrived_state(state, target, entry_progress)
                        });
                        (Some(space), target, entry_progress, was_arrived)
                    }
                    Some(RuntimeVehicleParkingBinding::Occupied { .. }) | None => {
                        (None, None, None, false)
                    }
                }
            } else {
                (None, None, None, false)
            };

            let Some(vehicle) = states
                .get_mut(vehicle_handle.index())
                .and_then(Option::as_mut)
            else {
                continue;
            };

            let completed_event = if PARKING_ACTIVE {
                Self::advance_vehicle::<true>(
                    advance_context,
                    vehicle,
                    motion,
                    parking_stop,
                    events,
                    spatial_changes,
                )?
            } else {
                Self::advance_vehicle::<false>(
                    advance_context,
                    vehicle,
                    motion,
                    None,
                    events,
                    spatial_changes,
                )?
            };
            if PARKING_ACTIVE {
                if let Some(space) = reserved_space {
                    if let Some(completed_event) = completed_event {
                        if reserved_target.is_some() {
                            return Err(TickInvariantError::ParkingBindingInvariantViolation {
                                stage: "step_reachable_target_completed",
                                vehicle: Some(vehicle_handle),
                                space: Some(space),
                            });
                        }
                        parking_releases.push(ParkingStepRelease {
                            vehicle: vehicle_handle,
                            space,
                        });
                        events.push(CoreEvent::ParkingReservationReleased(
                            ParkingReservationReleasedEvent {
                                tick_index: next_tick_index,
                                vehicle: vehicle_handle,
                                space,
                                reason: ParkingReleaseReason::RouteCompleted,
                            },
                        ));
                        events.push(CoreEvent::VehicleCompletedRoute(completed_event));
                    } else if reaches_parking_stop
                        && !was_arrived
                        && parking_arrived_state(vehicle, reserved_target, entry_progress)
                    {
                        let target = reserved_target
                            .expect("arrived reservation must have an approach target");
                        events.push(CoreEvent::VehicleParkingArrivalReached(
                            VehicleParkingArrivalReachedEvent {
                                tick_index: next_tick_index,
                                vehicle: vehicle_handle,
                                space,
                                route: target.route,
                                route_edge_index: target.route_edge_index,
                            },
                        ));
                    }
                } else if let Some(completed_event) = completed_event {
                    events.push(CoreEvent::VehicleCompletedRoute(completed_event));
                }
            } else if let Some(completed_event) = completed_event {
                events.push(CoreEvent::VehicleCompletedRoute(completed_event));
            }
            #[cfg(any(test, feature = "test-support"))]
            if self.step_failure_after_vehicle == Some(vehicle_handle) {
                return Err(TickInvariantError::ParkingBindingInvariantViolation {
                    stage: "test_after_vehicle_advance",
                    vehicle: Some(vehicle_handle),
                    space: if PARKING_ACTIVE { reserved_space } else { None },
                });
            }
            if vehicle.status == VehicleStatus::Active {
                *candidate_max_vehicle_speed =
                    candidate_max_vehicle_speed.max(vehicle.current_speed.value());
            }
        }
        if PARKING_ACTIVE {
            debug_assert!(
                parking_stops
                    .as_mut()
                    .expect("PARKING_ACTIVE keeps parking_stops")
                    .next()
                    .is_none()
            );
        }
        Ok(())
    }
}

impl CoreWorld {
    pub(super) fn append_signal_events(
        &self,
        tick_index: u64,
        candidate: &SignalRuntimeState,
        events: &mut Vec<CoreEvent>,
    ) {
        for (controller, next_controller_state) in candidate.controller_states() {
            let current_controller_state = self
                .signal_state
                .controller_state(controller)
                .expect("committed controller state must exist");
            let from_phase = current_controller_state.current_phase();
            let to_phase = next_controller_state.current_phase();
            if from_phase != to_phase {
                events.push(CoreEvent::SignalPhaseChanged(SignalPhaseChangedEvent {
                    tick_index,
                    controller,
                    from_phase,
                    to_phase,
                }));
            }

            for group in self
                .signals
                .controller_groups(controller)
                .expect("normalized controller groups must exist")
            {
                let from_aspect = self
                    .signal_state
                    .group_state(*group)
                    .expect("committed group state must exist")
                    .aspect();
                let to_aspect = candidate
                    .group_state(*group)
                    .expect("candidate group state must exist")
                    .aspect();
                if from_aspect != to_aspect {
                    events.push(CoreEvent::SignalGroupAspectChanged(
                        SignalGroupAspectChangedEvent {
                            tick_index,
                            group: *group,
                            from_aspect,
                            to_aspect,
                        },
                    ));
                }
            }
        }
    }
}

/// 测试支持接缝（C 类故障注入）：编译期门控，不进入默认发布构建。
/// 见 `docs/design/core-research-instrumentation-externalization.md` §3。
#[cfg(feature = "test-support")]
impl CoreWorld {
    /// 武装/解除 step 故障注入检查点：下一个 step 处理到指定车辆后返回受控失败。
    #[doc(hidden)]
    pub fn set_step_failure_after_vehicle(&mut self, vehicle: Option<VehicleHandle>) {
        self.step_failure_after_vehicle = vehicle;
    }

    /// 武装/解除 replace 故障注入检查点：下一次原子 replace 在 prepare 后返回受控失败。
    #[doc(hidden)]
    pub fn set_replace_failure_after_prepare(&mut self, fail: bool) {
        self.replace_failure_after_prepare = fail;
    }
}
