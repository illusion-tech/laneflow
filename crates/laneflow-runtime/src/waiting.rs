use laneflow_static_contract::WaitingZoneOrdinal;

use crate::tables::{CompiledRoute, distance_to_occurrence_start};
use crate::{RouteHandle, VehicleHandle};
use laneflow_static_network::BoundedDistance;

#[cfg(test)]
thread_local! {
    static WAITING_RESERVATIONS_BEFORE_FAILURE: core::cell::Cell<Option<usize>> =
        const { core::cell::Cell::new(None) };
}

#[cfg(test)]
struct WaitingReservationFailpointReset(Option<usize>);

#[cfg(test)]
impl Drop for WaitingReservationFailpointReset {
    fn drop(&mut self) {
        WAITING_RESERVATIONS_BEFORE_FAILURE.with(|remaining| remaining.set(self.0));
    }
}

#[cfg(test)]
fn fail_waiting_reservation_after(successes: usize) -> WaitingReservationFailpointReset {
    let previous =
        WAITING_RESERVATIONS_BEFORE_FAILURE.with(|remaining| remaining.replace(Some(successes)));
    WaitingReservationFailpointReset(previous)
}

#[cfg(test)]
fn waiting_reservation_injected_failure() -> bool {
    WAITING_RESERVATIONS_BEFORE_FAILURE.with(|remaining| match remaining.get() {
        Some(0) => true,
        Some(value) => {
            remaining.set(Some(value - 1));
            false
        }
        None => false,
    })
}

/// 车辆在一个 stateful maneuver occurrence 中的已提交阶段。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManeuverTraversalPhase {
    /// 已进入 occurrence，但尚未跨过下一道 Gate。
    PreGate { next_gate_hop: u32 },
    /// 已跨过至少一道 Gate，当前未因 release Gate 等待。
    Committed { last_crossed_gate_hop: u32 },
    /// 已到达所持 membership 的 release Gate，且该 Gate 是最终硬约束归因。
    Waiting { release_gate_hop: u32 },
}

/// 车辆当前 stateful maneuver occurrence 的语义状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManeuverTraversalState {
    pub(crate) route: RouteHandle,
    pub(crate) maneuver_occurrence_index: u32,
    pub(crate) phase: ManeuverTraversalPhase,
}

impl ManeuverTraversalState {
    #[must_use]
    pub const fn route(self) -> RouteHandle {
        self.route
    }

    #[must_use]
    pub const fn maneuver_occurrence_index(self) -> u32 {
        self.maneuver_occurrence_index
    }

    #[must_use]
    pub const fn phase(self) -> ManeuverTraversalPhase {
        self.phase
    }
}

/// 车辆持有的 WaitingZone 语义 membership。队列 link 不属于该持久语义。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitingMembership {
    pub(crate) waiting_zone: WaitingZoneOrdinal,
    pub(crate) admission_sequence: u64,
    pub(crate) release_hop: u32,
}

impl WaitingMembership {
    #[must_use]
    pub const fn waiting_zone(self) -> WaitingZoneOrdinal {
        self.waiting_zone
    }

    #[must_use]
    pub const fn admission_sequence(self) -> u64 {
        self.admission_sequence
    }

    #[must_use]
    pub const fn release_hop(self) -> u32 {
        self.release_hop
    }
}

/// WaitingZone 的只读已提交计数。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitingZoneSnapshot {
    pub(crate) zone: WaitingZoneOrdinal,
    pub(crate) occupancy: u32,
    pub(crate) max_occupancy: u32,
    pub(crate) next_admission_sequence: u64,
}

impl WaitingZoneSnapshot {
    #[must_use]
    pub const fn zone(self) -> WaitingZoneOrdinal {
        self.zone
    }

    #[must_use]
    pub const fn occupancy(self) -> u32 {
        self.occupancy
    }

    #[must_use]
    pub const fn max_occupancy(self) -> u32 {
        self.max_occupancy
    }

    #[must_use]
    pub const fn next_admission_sequence(self) -> u64 {
        self.next_admission_sequence
    }
}

/// 按 zone、admission sequence 排列的只读 member 行。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitingZoneMember {
    pub(crate) zone: WaitingZoneOrdinal,
    pub(crate) vehicle: VehicleHandle,
    pub(crate) admission_sequence: u64,
    pub(crate) release_hop: u32,
}

impl WaitingZoneMember {
    #[must_use]
    pub const fn zone(self) -> WaitingZoneOrdinal {
        self.zone
    }

    #[must_use]
    pub const fn vehicle(self) -> VehicleHandle {
        self.vehicle
    }

    #[must_use]
    pub const fn admission_sequence(self) -> u64 {
        self.admission_sequence
    }

    #[must_use]
    pub const fn release_hop(self) -> u32 {
        self.release_hop
    }
}

/// Waiting admission 没有取得 claim 的本地原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitingNoGrantReason {
    Capacity,
    PhysicalStorage,
}

/// 刚完成 successful tick 的 Waiting admission 决定。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitingDecisionOutcome {
    NotEvaluated,
    NotRequired,
    Granted,
    NoGrant(WaitingNoGrantReason),
}

/// Waiting decision 的稳定 route anchor。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitingRouteAnchor {
    pub(crate) route: RouteHandle,
    pub(crate) maneuver_occurrence_index: u32,
    pub(crate) hop: u32,
}

impl WaitingRouteAnchor {
    #[must_use]
    pub const fn route(self) -> RouteHandle {
        self.route
    }

    #[must_use]
    pub const fn maneuver_occurrence_index(self) -> u32 {
        self.maneuver_occurrence_index
    }

    #[must_use]
    pub const fn hop(self) -> u32 {
        self.hop
    }
}

/// 一条 Waiting admission 决定。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitingDecision {
    pub(crate) vehicle: VehicleHandle,
    pub(crate) vehicle_update_sequence: u32,
    pub(crate) zone: Option<WaitingZoneOrdinal>,
    pub(crate) anchor: WaitingRouteAnchor,
    pub(crate) outcome: WaitingDecisionOutcome,
}

impl WaitingDecision {
    #[must_use]
    pub const fn vehicle(self) -> VehicleHandle {
        self.vehicle
    }

    #[must_use]
    pub const fn vehicle_update_sequence(self) -> u32 {
        self.vehicle_update_sequence
    }

    #[must_use]
    pub const fn zone(self) -> Option<WaitingZoneOrdinal> {
        self.zone
    }

    #[must_use]
    pub const fn anchor(self) -> WaitingRouteAnchor {
        self.anchor
    }

    #[must_use]
    pub const fn outcome(self) -> WaitingDecisionOutcome {
        self.outcome
    }
}

/// 前保险杠被投影到 Waiting entry boundary 的原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitingProjectionReason {
    EvaluationHorizon,
    Capacity,
    PhysicalStorage,
}

/// successful fixed step 中提交的一条 Waiting transition。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitingTransitionKind {
    ProjectionApplied {
        zone: WaitingZoneOrdinal,
        reason: WaitingProjectionReason,
    },
    Left {
        zone: WaitingZoneOrdinal,
        admission_sequence: u64,
    },
    Entered {
        zone: WaitingZoneOrdinal,
        admission_sequence: u64,
    },
    ManeuverTraversalCompleted {
        maneuver_occurrence_index: u32,
    },
}

/// 刚完成 successful tick 的 Waiting transition event。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitingTransitionEvent {
    pub(crate) tick: u64,
    pub(crate) vehicle: VehicleHandle,
    pub(crate) vehicle_update_sequence: u32,
    pub(crate) anchor: WaitingRouteAnchor,
    pub(crate) kind: WaitingTransitionKind,
}

impl WaitingTransitionEvent {
    #[must_use]
    pub const fn tick(self) -> u64 {
        self.tick
    }

    #[must_use]
    pub const fn vehicle(self) -> VehicleHandle {
        self.vehicle
    }

    #[must_use]
    pub const fn vehicle_update_sequence(self) -> u32 {
        self.vehicle_update_sequence
    }

    #[must_use]
    pub const fn anchor(self) -> WaitingRouteAnchor {
        self.anchor
    }

    #[must_use]
    pub const fn kind(self) -> WaitingTransitionKind {
        self.kind
    }
}

/// `despawn_vehicle` 同步回显的 Waiting membership 释放。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitingMembershipReleaseRecord {
    pub(crate) waiting_zone: WaitingZoneOrdinal,
    pub(crate) route_anchor: WaitingRouteAnchor,
    pub(crate) admission_sequence: u64,
}

impl WaitingMembershipReleaseRecord {
    #[must_use]
    pub const fn waiting_zone(self) -> WaitingZoneOrdinal {
        self.waiting_zone
    }

    #[must_use]
    pub const fn route_anchor(self) -> WaitingRouteAnchor {
        self.route_anchor
    }

    #[must_use]
    pub const fn admission_sequence(self) -> u64 {
        self.admission_sequence
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct WaitingZoneState {
    pub occupancy: u32,
    pub next_admission_sequence: u64,
    pub head: Option<VehicleHandle>,
    pub tail: Option<VehicleHandle>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct WaitingQueueLink {
    pub previous: Option<VehicleHandle>,
    pub next: Option<VehicleHandle>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WaitingAdmissionClaim {
    pub vehicle: VehicleHandle,
    pub vehicle_update_sequence: u32,
    pub occurrence_index: u32,
    pub zone: WaitingZoneOrdinal,
    pub entry_hop: u32,
    pub release_hop: u32,
    pub approach_distance_mm: u32,
    pub plan_index: u32,
    pub post_step_group: u8,
    pub post_step_rank: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WaitingVehiclePlan {
    pub vehicle: VehicleHandle,
    pub vehicle_update_sequence: u32,
    pub occurrence_index: u32,
    pub zone: WaitingZoneOrdinal,
    pub maneuver_index: u32,
    pub entry_hop: u32,
    pub release_hop: u32,
    pub approach_distance_mm: u32,
    pub preview_route_edge_index: u32,
    pub decision: WaitingDecisionOutcome,
    pub stop_hop: Option<u32>,
    pub stop_zone: Option<WaitingZoneOrdinal>,
    pub stop_maneuver_index: Option<u32>,
    pub projection: Option<WaitingProjectionReason>,
    pub admission_sequence: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WaitingStopConstraint {
    pub distance: laneflow_static_network::BoundedDistance,
    pub hop: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WaitingBindingError {
    VehicleTooLong,
    StatefulManeuverInterior,
    InvalidRoute,
    AuthorityMismatch,
    ParkingConflict,
}

impl crate::TrafficWorld {
    pub(crate) fn validate_waiting_parking_anchor(
        &self,
        route: RouteHandle,
        route_occurrence: u32,
    ) -> Result<(), WaitingBindingError> {
        let compiled = self
            .compiled_route(route)
            .ok_or(WaitingBindingError::InvalidRoute)?;
        let cursor = route_occurrence;
        if compiled
            .maneuvers
            .iter()
            .enumerate()
            .any(|(index, maneuver)| {
                cursor >= maneuver.entry_route_edge_index
                    && cursor < maneuver.exit_route_edge_index
                    && compiled
                        .waiting
                        .iter()
                        .any(|waiting| waiting.maneuver_index as usize == index)
            })
        {
            return Err(WaitingBindingError::ParkingConflict);
        }
        Ok(())
    }

    pub(crate) fn rebind_waiting_authority(
        &self,
        state: crate::VehicleState,
        new_route: RouteHandle,
        new_cursor: usize,
    ) -> Result<(Option<ManeuverTraversalState>, Option<WaitingMembership>), WaitingBindingError>
    {
        let Some(old_traversal) = state.maneuver_traversal else {
            if state.waiting_membership.is_some() {
                return Err(WaitingBindingError::AuthorityMismatch);
            }
            return self
                .validate_waiting_bootstrap(new_route, new_cursor, state.length_mm)
                .map(|traversal| (traversal, None));
        };
        let old_compiled = self
            .compiled_route(state.route)
            .ok_or(WaitingBindingError::InvalidRoute)?;
        let old_maneuver = old_compiled
            .maneuvers
            .get(old_traversal.maneuver_occurrence_index as usize)
            .ok_or(WaitingBindingError::AuthorityMismatch)?;
        let new_compiled = self
            .compiled_route(new_route)
            .ok_or(WaitingBindingError::InvalidRoute)?;
        let new_cursor_u32 =
            u32::try_from(new_cursor).map_err(|_| WaitingBindingError::InvalidRoute)?;
        let (new_maneuver_index, new_maneuver) = new_compiled
            .maneuvers
            .iter()
            .enumerate()
            .find(|(_, maneuver)| {
                maneuver.path == old_maneuver.path
                    && new_cursor_u32 >= maneuver.entry_route_edge_index
                    && new_cursor_u32 < maneuver.exit_route_edge_index
            })
            .ok_or(WaitingBindingError::AuthorityMismatch)?;

        let map_hop = |old_hop: u32| -> Result<u32, WaitingBindingError> {
            let gate = old_compiled
                .hop_gate
                .get(old_hop as usize)
                .copied()
                .flatten()
                .ok_or(WaitingBindingError::AuthorityMismatch)?;
            (new_maneuver.entry_route_edge_index..new_maneuver.exit_route_edge_index)
                .find(|hop| {
                    new_compiled.hop_gate.get(*hop as usize).copied().flatten() == Some(gate)
                })
                .ok_or(WaitingBindingError::AuthorityMismatch)
        };
        let phase = match old_traversal.phase {
            ManeuverTraversalPhase::PreGate { next_gate_hop } => {
                let mapped = map_hop(next_gate_hop)?;
                if new_cursor_u32 > mapped {
                    return Err(WaitingBindingError::AuthorityMismatch);
                }
                ManeuverTraversalPhase::PreGate {
                    next_gate_hop: mapped,
                }
            }
            ManeuverTraversalPhase::Committed {
                last_crossed_gate_hop,
            } => {
                let mapped = map_hop(last_crossed_gate_hop)?;
                if new_cursor_u32 <= mapped {
                    return Err(WaitingBindingError::AuthorityMismatch);
                }
                ManeuverTraversalPhase::Committed {
                    last_crossed_gate_hop: mapped,
                }
            }
            ManeuverTraversalPhase::Waiting { release_gate_hop } => {
                let mapped = map_hop(release_gate_hop)?;
                if new_cursor_u32 != mapped {
                    return Err(WaitingBindingError::AuthorityMismatch);
                }
                ManeuverTraversalPhase::Waiting {
                    release_gate_hop: mapped,
                }
            }
        };
        let membership = match state.waiting_membership {
            None => None,
            Some(old_membership) => {
                let old_waiting = old_compiled
                    .waiting
                    .iter()
                    .find(|waiting| {
                        waiting.maneuver_index == old_traversal.maneuver_occurrence_index
                            && waiting.zone == old_membership.waiting_zone
                            && waiting.release_hop == old_membership.release_hop
                    })
                    .ok_or(WaitingBindingError::AuthorityMismatch)?;
                let new_waiting = new_compiled
                    .waiting
                    .iter()
                    .find(|waiting| {
                        waiting.maneuver_index as usize == new_maneuver_index
                            && waiting.zone == old_waiting.zone
                            && new_compiled
                                .hop_gate
                                .get(waiting.entry_hop as usize)
                                .copied()
                                .flatten()
                                == old_compiled
                                    .hop_gate
                                    .get(old_waiting.entry_hop as usize)
                                    .copied()
                                    .flatten()
                            && new_compiled
                                .hop_gate
                                .get(waiting.release_hop as usize)
                                .copied()
                                .flatten()
                                == old_compiled
                                    .hop_gate
                                    .get(old_waiting.release_hop as usize)
                                    .copied()
                                    .flatten()
                    })
                    .ok_or(WaitingBindingError::AuthorityMismatch)?;
                if !waiting_membership_cursor_valid(new_cursor_u32, new_waiting) {
                    return Err(WaitingBindingError::AuthorityMismatch);
                }
                Some(WaitingMembership {
                    waiting_zone: old_membership.waiting_zone,
                    admission_sequence: old_membership.admission_sequence,
                    release_hop: new_waiting.release_hop,
                })
            }
        };
        for waiting in &new_compiled.waiting {
            let is_held = membership.is_some_and(|member| {
                member.waiting_zone == waiting.zone
                    && member.release_hop == waiting.release_hop
                    && waiting.maneuver_index as usize == new_maneuver_index
            });
            if !is_held
                && new_cursor_u32 <= waiting.release_hop
                && state.length_mm > waiting.storage_length_mm
            {
                return Err(WaitingBindingError::VehicleTooLong);
            }
        }
        Ok((
            Some(ManeuverTraversalState {
                route: new_route,
                maneuver_occurrence_index: u32::try_from(new_maneuver_index)
                    .map_err(|_| WaitingBindingError::InvalidRoute)?,
                phase,
            }),
            membership,
        ))
    }

    pub(crate) fn prepare_waiting_step(&mut self, delta_s: f32) -> Result<(), crate::StepError> {
        if !self.waiting_state_valid() {
            return Err(crate::StepError::WaitingInvariantViolation);
        }
        self.waiting_plans.clear();
        self.waiting_claims.clear();
        self.waiting_staged_decisions.clear();
        self.waiting_staged_events.clear();
        self.waiting_plan_by_vehicle.fill(None);
        reserve_waiting_exact(&mut self.waiting_plans, self.active_order.len())?;

        // 无 Waiting stop 的运动预览同时供候选收集和 NotRequired Gate 批次使用。
        // `next_states` 已按 vehicle_capacity 预分配；正式 motion staging 会在本阶段后
        // 清空并复用它，避免对每辆车重复三次相同的 leader / signal / route 计算。
        self.next_states.clear();
        reserve_waiting_exact(&mut self.next_states, self.active_order.len())?;
        for sequence in 0..self.active_order.len() {
            let vehicle = self.active_order[sequence];
            let state = *self
                .vehicle_state(vehicle)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let compiled = self
                .compiled_route(state.route)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let cursor = state.route_edge_index as usize;
            let Some(gate_hop) = compiled
                .hop_gate
                .iter()
                .enumerate()
                .skip(cursor)
                .find_map(|(hop, gate)| gate.map(|_| hop))
            else {
                continue;
            };
            let profile = self
                .revision
                .traffic()
                .relations()
                .vehicle_profile(state.profile)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let horizon = crate::tick::leader_query_horizon(state.speed_mm_s, profile, delta_s)
                .ok_or(crate::StepError::NonFiniteMotion)?;
            let Some(BoundedDistance::Finite(gate_distance_mm)) = distance_to_occurrence_start(
                &compiled.occurrence_segments,
                &compiled.occurrence_offsets,
                &compiled.segment_totals,
                cursor,
                state.progress_mm,
                gate_hop
                    .checked_add(1)
                    .ok_or(crate::StepError::WaitingInvariantViolation)?,
            ) else {
                continue;
            };
            if gate_distance_mm > horizon.front_query_mm {
                continue;
            }
            let preview = self
                .advance_active_vehicle(state, delta_s)
                .ok_or(crate::StepError::NonFiniteMotion)?;
            self.next_states.push((sequence, preview));
        }

        for preview_index in 0..self.next_states.len() {
            let (update_sequence, preview) = self.next_states[preview_index];
            let vehicle = self.active_order[update_sequence];
            let state = *self
                .vehicle_state(vehicle)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let compiled = self
                .compiled_route(state.route)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let Some((occurrence_index, occurrence)) = compiled
                .waiting
                .iter()
                .copied()
                .enumerate()
                .find(|(_, occurrence)| {
                    let held = state.waiting_membership.is_some_and(|membership| {
                        membership.waiting_zone == occurrence.zone
                            && membership.release_hop == occurrence.release_hop
                    }) && state.maneuver_traversal.is_some_and(|traversal| {
                        traversal.maneuver_occurrence_index == occurrence.maneuver_index
                    });
                    !held && state.route_edge_index <= occurrence.entry_hop
                })
            else {
                continue;
            };
            let entry_index = usize::try_from(occurrence.entry_hop)
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let approach_distance_mm = match distance_to_occurrence_start(
                &compiled.occurrence_segments,
                &compiled.occurrence_offsets,
                &compiled.segment_totals,
                state.route_edge_index as usize,
                state.progress_mm,
                entry_index,
            ) {
                Some(BoundedDistance::Finite(value)) => value,
                Some(BoundedDistance::BeyondFinite) | None => continue,
            };
            let entry_gate = compiled
                .hop_gate
                .get(occurrence.entry_hop as usize)
                .copied()
                .flatten()
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let preview_crossed = preview.route_edge_index > occurrence.entry_hop;
            let preview_at_boundary = front_at_hop_boundary(
                compiled,
                &preview,
                occurrence.entry_hop,
                self.revision.traffic().lane_lengths_millimetres(),
            );
            let decision = if preview_crossed {
                WaitingDecisionOutcome::Granted
            } else if preview_at_boundary && self.gate_is_restrictive(entry_gate) {
                WaitingDecisionOutcome::NotEvaluated
            } else {
                continue;
            };
            self.waiting_plans.push(WaitingVehiclePlan {
                vehicle,
                vehicle_update_sequence: u32::try_from(update_sequence)
                    .map_err(|_| crate::StepError::WaitingInvariantViolation)?,
                occurrence_index: u32::try_from(occurrence_index)
                    .map_err(|_| crate::StepError::WaitingInvariantViolation)?,
                zone: occurrence.zone,
                maneuver_index: occurrence.maneuver_index,
                entry_hop: occurrence.entry_hop,
                release_hop: occurrence.release_hop,
                approach_distance_mm,
                preview_route_edge_index: preview.route_edge_index,
                decision,
                stop_hop: None,
                stop_zone: None,
                stop_maneuver_index: None,
                projection: None,
                admission_sequence: None,
            });
        }

        for (zone_index, state) in self.waiting_zones.iter().copied().enumerate() {
            self.waiting_next_counters[zone_index] = state.next_admission_sequence;
            self.waiting_staged_occupancy[zone_index] = state.occupancy;
            let mut used = 0_u64;
            let mut current = state.head;
            let mut has_front = false;
            while let Some(vehicle) = current {
                let member = self
                    .vehicle_state(vehicle)
                    .ok_or(crate::StepError::WaitingInvariantViolation)?;
                let profile = self
                    .revision
                    .traffic()
                    .relations()
                    .vehicle_profile(member.profile)
                    .ok_or(crate::StepError::WaitingInvariantViolation)?;
                if has_front {
                    used = used
                        .checked_add(u64::from(profile.min_gap_mm()))
                        .ok_or(crate::StepError::WaitingInvariantViolation)?;
                }
                used = used
                    .checked_add(u64::from(member.length_mm))
                    .ok_or(crate::StepError::WaitingInvariantViolation)?;
                has_front = true;
                current = self.waiting_links[vehicle.index() as usize].next;
            }
            self.waiting_staged_storage_mm[zone_index] = used;
        }

        self.waiting_plans.sort_unstable_by_key(|plan| {
            (
                plan.zone.raw(),
                plan.approach_distance_mm,
                plan.vehicle_update_sequence,
                plan.entry_hop,
            )
        });
        for index in 0..self.waiting_plans.len() {
            if self.waiting_plans[index].decision == WaitingDecisionOutcome::NotEvaluated {
                continue;
            }
            let plan = self.waiting_plans[index];
            let zone_index = plan.zone.index();
            let max_occupancy = self
                .revision
                .traffic()
                .relations()
                .waiting_zone(plan.zone)
                .ok_or(crate::StepError::WaitingInvariantViolation)?
                .max_occupancy();
            if self.waiting_staged_occupancy[zone_index] >= max_occupancy {
                self.waiting_plans[index].decision =
                    WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::Capacity);
                continue;
            }
            let state = self
                .vehicle_state(plan.vehicle)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let profile = self
                .revision
                .traffic()
                .relations()
                .vehicle_profile(state.profile)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let gap = if self.waiting_staged_occupancy[zone_index] == 0 {
                0
            } else {
                u64::from(profile.min_gap_mm())
            };
            let required = self.waiting_staged_storage_mm[zone_index]
                .checked_add(gap)
                .and_then(|value| value.checked_add(u64::from(state.length_mm)))
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let storage_length_mm = self
                .compiled_route(state.route)
                .and_then(|compiled| compiled.waiting.get(plan.occurrence_index as usize))
                .map(|occurrence| occurrence.storage_length_mm)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            if required > u64::from(storage_length_mm) {
                self.waiting_plans[index].decision =
                    WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::PhysicalStorage);
                continue;
            }
            self.waiting_staged_occupancy[zone_index] = self.waiting_staged_occupancy[zone_index]
                .checked_add(1)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            self.waiting_staged_storage_mm[zone_index] = required;
        }

        let grant_count = self
            .waiting_plans
            .iter()
            .filter(|plan| plan.decision == WaitingDecisionOutcome::Granted)
            .count();
        let lengths = self.revision.traffic().lane_lengths_millimetres();
        let mut not_required_count = 0_usize;
        for preview_index in 0..self.next_states.len() {
            let (update_sequence, preview) = self.next_states[preview_index];
            let vehicle = self.active_order[update_sequence];
            let state = *self
                .vehicle_state(vehicle)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let compiled = self
                .compiled_route(state.route)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let gate_end = compiled
                .hop_gate
                .len()
                .min((preview.route_edge_index as usize).saturating_add(1));
            for hop in state.route_edge_index as usize..gate_end {
                if not_required_gate_anchor(compiled, &preview, hop, lengths).is_some() {
                    not_required_count = not_required_count
                        .checked_add(1)
                        .ok_or(crate::StepError::WaitingInvariantViolation)?;
                }
            }
        }
        let decision_count = self
            .waiting_plans
            .len()
            .checked_add(not_required_count)
            .ok_or(crate::StepError::WaitingInvariantViolation)?;
        reserve_waiting_exact(&mut self.waiting_claims, grant_count)?;
        reserve_waiting_exact(&mut self.waiting_staged_decisions, decision_count)?;
        for index in 0..self.waiting_plans.len() {
            let mut plan = self.waiting_plans[index];
            match plan.decision {
                WaitingDecisionOutcome::Granted => {
                    self.waiting_claims.push(WaitingAdmissionClaim {
                        vehicle: plan.vehicle,
                        vehicle_update_sequence: plan.vehicle_update_sequence,
                        occurrence_index: plan.occurrence_index,
                        zone: plan.zone,
                        entry_hop: plan.entry_hop,
                        release_hop: plan.release_hop,
                        approach_distance_mm: plan.approach_distance_mm,
                        plan_index: u32::try_from(index)
                            .map_err(|_| crate::StepError::WaitingInvariantViolation)?,
                        post_step_group: 0,
                        post_step_rank: 0,
                    });
                    let state = self
                        .vehicle_state(plan.vehicle)
                        .ok_or(crate::StepError::WaitingInvariantViolation)?;
                    let compiled = self
                        .compiled_route(state.route)
                        .ok_or(crate::StepError::WaitingInvariantViolation)?;
                    if let Some(next) = compiled
                        .waiting
                        .iter()
                        .skip(plan.occurrence_index as usize + 1)
                        .find(|occurrence| plan.preview_route_edge_index > occurrence.entry_hop)
                    {
                        plan.stop_hop = Some(next.entry_hop);
                        plan.stop_zone = Some(next.zone);
                        plan.stop_maneuver_index = Some(next.maneuver_index);
                        plan.projection = Some(WaitingProjectionReason::EvaluationHorizon);
                    }
                }
                WaitingDecisionOutcome::NoGrant(reason) => {
                    plan.stop_hop = Some(plan.entry_hop);
                    plan.stop_zone = Some(plan.zone);
                    plan.stop_maneuver_index = Some(plan.maneuver_index);
                    plan.projection = Some(match reason {
                        WaitingNoGrantReason::Capacity => WaitingProjectionReason::Capacity,
                        WaitingNoGrantReason::PhysicalStorage => {
                            WaitingProjectionReason::PhysicalStorage
                        }
                    });
                }
                WaitingDecisionOutcome::NotEvaluated | WaitingDecisionOutcome::NotRequired => {}
            }
            self.waiting_plans[index] = plan;
            self.waiting_plan_by_vehicle[plan.vehicle.index() as usize] = Some(plan);
            self.waiting_staged_decisions.push(WaitingDecision {
                vehicle: plan.vehicle,
                vehicle_update_sequence: plan.vehicle_update_sequence,
                zone: Some(plan.zone),
                anchor: WaitingRouteAnchor {
                    route: self
                        .vehicle_state(plan.vehicle)
                        .ok_or(crate::StepError::WaitingInvariantViolation)?
                        .route,
                    maneuver_occurrence_index: plan.maneuver_index,
                    hop: plan.entry_hop,
                },
                outcome: plan.decision,
            });
        }
        for preview_index in 0..self.next_states.len() {
            let (update_sequence, preview) = self.next_states[preview_index];
            let vehicle = self.active_order[update_sequence];
            let state = *self
                .vehicle_state(vehicle)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let gate_count = self
                .compiled_route(state.route)
                .ok_or(crate::StepError::WaitingInvariantViolation)?
                .hop_gate
                .len()
                .min((preview.route_edge_index as usize).saturating_add(1));
            for hop in state.route_edge_index as usize..gate_count {
                let anchor = {
                    let compiled = self
                        .compiled_route(state.route)
                        .ok_or(crate::StepError::WaitingInvariantViolation)?;
                    not_required_gate_anchor(
                        compiled,
                        &preview,
                        hop,
                        self.revision.traffic().lane_lengths_millimetres(),
                    )
                };
                let Some(anchor) = anchor else {
                    continue;
                };
                self.waiting_staged_decisions.push(WaitingDecision {
                    vehicle,
                    vehicle_update_sequence: u32::try_from(update_sequence)
                        .map_err(|_| crate::StepError::WaitingInvariantViolation)?,
                    zone: None,
                    anchor: WaitingRouteAnchor {
                        route: state.route,
                        maneuver_occurrence_index: anchor.0,
                        hop: anchor.1,
                    },
                    outcome: WaitingDecisionOutcome::NotRequired,
                });
            }
        }
        debug_assert_eq!(self.waiting_staged_decisions.len(), decision_count);
        self.waiting_staged_decisions
            .sort_unstable_by_key(|decision| {
                (
                    decision.vehicle_update_sequence,
                    decision.anchor.maneuver_occurrence_index,
                    decision.anchor.hop,
                )
            });
        Ok(())
    }

    pub(crate) fn waiting_stop_for(
        &self,
        state: crate::VehicleState,
    ) -> Result<Option<WaitingStopConstraint>, crate::StepError> {
        let Some(plan) = self
            .waiting_plan_by_vehicle
            .get(state.handle.index() as usize)
            .copied()
            .flatten()
            .filter(|plan| plan.vehicle == state.handle)
        else {
            return Ok(None);
        };
        let Some(stop_hop) = plan.stop_hop else {
            return Ok(None);
        };
        let compiled = self
            .compiled_route(state.route)
            .ok_or(crate::StepError::WaitingInvariantViolation)?;
        let stop_index = usize::try_from(stop_hop)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(crate::StepError::WaitingInvariantViolation)?;
        let distance = distance_to_occurrence_start(
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            state.route_edge_index as usize,
            state.progress_mm,
            stop_index,
        )
        .ok_or(crate::StepError::WaitingInvariantViolation)?;
        Ok(Some(WaitingStopConstraint {
            distance,
            hop: stop_hop,
        }))
    }

    pub(crate) fn finalize_waiting_step(
        &mut self,
        updates: &mut [(usize, crate::VehicleState)],
        tick: u64,
    ) -> Result<(), crate::StepError> {
        self.waiting_next_state_index.fill(0);
        for (update_index, (slot, _)) in updates.iter().enumerate() {
            let encoded = u32::try_from(update_index)
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            self.waiting_next_state_index[*slot] = encoded;
        }

        // 先 stage tick-start membership 的 successful release。
        for (slot, next) in updates.iter_mut() {
            let old = self.vehicles[*slot]
                .state
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            if let Some(membership) = old.waiting_membership
                && next.route_edge_index > membership.release_hop
            {
                next.waiting_membership = None;
            }
        }

        // successful entry 才消耗 admission sequence；同拍 entry+release 仍消耗 counter。
        // 先从 staged motion 计算物理 rank，再按 zone 内 post-step front-to-back 分配。
        for claim_index in 0..self.waiting_claims.len() {
            let mut claim = self.waiting_claims[claim_index];
            let encoded = self.waiting_next_state_index[claim.vehicle.index() as usize];
            let update_index = encoded
                .checked_sub(1)
                .map(|value| value as usize)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let next = updates[update_index].1;
            if next.route_edge_index <= claim.entry_hop {
                claim.post_step_group = u8::MAX;
                self.waiting_claims[claim_index] = claim;
                continue;
            }
            let (group, rank) = post_step_physical_rank(
                self.compiled_route(next.route)
                    .ok_or(crate::StepError::WaitingInvariantViolation)?,
                &next,
                claim.release_hop,
            )
            .ok_or(crate::StepError::WaitingInvariantViolation)?;
            claim.post_step_group = group;
            claim.post_step_rank = rank;
            self.waiting_claims[claim_index] = claim;
        }
        self.waiting_claims
            .retain(|claim| claim.post_step_group != u8::MAX);
        self.waiting_claims.sort_unstable_by_key(|claim| {
            (
                claim.zone.raw(),
                claim.post_step_group,
                claim.post_step_rank,
                claim.vehicle_update_sequence,
                claim.entry_hop,
            )
        });
        for claim in self.waiting_claims.iter().copied() {
            let plan_index = claim.plan_index as usize;
            let mut plan = self.waiting_plans[plan_index];
            let encoded = self.waiting_next_state_index[claim.vehicle.index() as usize];
            let update_index = encoded
                .checked_sub(1)
                .map(|value| value as usize)
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            let next = &mut updates[update_index].1;
            let zone_index = claim.zone.index();
            let sequence = self.waiting_next_counters[zone_index];
            self.waiting_next_counters[zone_index] = sequence
                .checked_add(1)
                .ok_or(crate::StepError::WaitingAdmissionSequenceExhausted)?;
            plan.admission_sequence = Some(sequence);
            if next.route_edge_index <= claim.release_hop {
                next.waiting_membership = Some(WaitingMembership {
                    waiting_zone: claim.zone,
                    admission_sequence: sequence,
                    release_hop: claim.release_hop,
                });
            } else {
                next.waiting_membership = None;
            }
            self.waiting_plans[plan_index] = plan;
            self.waiting_plan_by_vehicle[plan.vehicle.index() as usize] = Some(plan);
        }

        for (slot, next) in updates.iter_mut() {
            let old = self.vehicles[*slot]
                .state
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            next.maneuver_traversal = self.derive_waiting_traversal(*next)?;
            if next.status != crate::VehicleStatus::Active
                && (next.maneuver_traversal.is_some() || next.waiting_membership.is_some())
            {
                return Err(crate::StepError::WaitingInvariantViolation);
            }
            if old.waiting_membership.is_some() && old.maneuver_traversal.is_none() {
                return Err(crate::StepError::WaitingInvariantViolation);
            }
        }

        let event_count = updates.iter().try_fold(0_usize, |total, (slot, next)| {
            let old = self.vehicles[*slot].state.expect("next-state slot is live");
            let count = self
                .waiting_events_for(old, *next, tick)
                .into_iter()
                .flatten()
                .count();
            total
                .checked_add(count)
                .ok_or(crate::StepError::WaitingInvariantViolation)
        })?;
        reserve_waiting_exact(&mut self.waiting_staged_events, event_count)?;
        for (slot, next) in updates.iter() {
            let old = self.vehicles[*slot]
                .state
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            self.waiting_staged_events.extend(
                self.waiting_events_for(old, *next, tick)
                    .into_iter()
                    .flatten(),
            );
        }
        self.waiting_staged_events.sort_unstable_by_key(|event| {
            (
                event.vehicle_update_sequence,
                event.anchor.maneuver_occurrence_index,
                event.anchor.hop,
                transition_kind_rank(event.kind),
            )
        });
        Ok(())
    }

    pub(crate) fn commit_waiting_removals(&mut self, updates: &[(usize, crate::VehicleState)]) {
        for (slot, next) in updates {
            let old = self.vehicles[*slot]
                .state
                .expect("staged next state has a live predecessor");
            if let Some(membership) = old.waiting_membership
                && next.waiting_membership != Some(membership)
            {
                self.unlink_waiting_member(old.handle, membership);
            }
        }
    }

    pub(crate) fn commit_waiting_additions(&mut self, updates: &[(usize, crate::VehicleState)]) {
        for plan_index in 0..self.waiting_plans.len() {
            let plan = self.waiting_plans[plan_index];
            let Some(sequence) = plan.admission_sequence else {
                continue;
            };
            if plan.vehicle.index() as usize >= self.waiting_next_state_index.len() {
                continue;
            }
            let Some(update_index) = self.waiting_next_state_index[plan.vehicle.index() as usize]
                .checked_sub(1)
                .map(|value| value as usize)
            else {
                continue;
            };
            let next = updates[update_index].1;
            let membership = WaitingMembership {
                waiting_zone: plan.zone,
                admission_sequence: sequence,
                release_hop: plan.release_hop,
            };
            if next.waiting_membership == Some(membership) {
                self.append_waiting_member(plan.vehicle, membership);
            }
        }
        for (index, state) in self.waiting_zones.iter_mut().enumerate() {
            state.next_admission_sequence = self.waiting_next_counters[index];
        }
        self.rebuild_waiting_member_rows();
        std::mem::swap(
            &mut self.latest_waiting_decisions,
            &mut self.waiting_staged_decisions,
        );
        std::mem::swap(
            &mut self.latest_waiting_events,
            &mut self.waiting_staged_events,
        );
    }

    fn derive_waiting_traversal(
        &self,
        state: crate::VehicleState,
    ) -> Result<Option<ManeuverTraversalState>, crate::StepError> {
        if state.status != crate::VehicleStatus::Active {
            return Ok(None);
        }
        let compiled = self
            .compiled_route(state.route)
            .ok_or(crate::StepError::WaitingInvariantViolation)?;
        let cursor = state.route_edge_index;
        let Some((maneuver_index, maneuver)) =
            compiled
                .maneuvers
                .iter()
                .enumerate()
                .find(|(index, maneuver)| {
                    cursor >= maneuver.entry_route_edge_index
                        && cursor < maneuver.exit_route_edge_index
                        && compiled
                            .waiting
                            .iter()
                            .any(|waiting| waiting.maneuver_index as usize == *index)
                })
        else {
            return Ok(None);
        };
        let first_gate = first_gate_hop(compiled, maneuver)
            .ok_or(crate::StepError::WaitingInvariantViolation)?;
        let mut last_crossed = None;
        let mut next_gate = None;
        for hop in maneuver.entry_route_edge_index..maneuver.exit_route_edge_index {
            if compiled
                .hop_gate
                .get(hop as usize)
                .copied()
                .flatten()
                .is_none()
            {
                continue;
            }
            if hop < cursor {
                last_crossed = Some(hop);
            } else if next_gate.is_none() {
                next_gate = Some(hop);
            }
        }
        let phase = if let Some(membership) = state.waiting_membership {
            let release_gate = compiled
                .hop_gate
                .get(membership.release_hop as usize)
                .copied()
                .flatten()
                .ok_or(crate::StepError::WaitingInvariantViolation)?;
            if front_at_hop_boundary(
                compiled,
                &state,
                membership.release_hop,
                self.revision.traffic().lane_lengths_millimetres(),
            ) && self.gate_is_restrictive(release_gate)
            {
                ManeuverTraversalPhase::Waiting {
                    release_gate_hop: membership.release_hop,
                }
            } else if let Some(last_crossed) = last_crossed {
                ManeuverTraversalPhase::Committed {
                    last_crossed_gate_hop: last_crossed,
                }
            } else {
                ManeuverTraversalPhase::PreGate {
                    next_gate_hop: next_gate.unwrap_or(first_gate),
                }
            }
        } else if let Some(last_crossed) = last_crossed {
            ManeuverTraversalPhase::Committed {
                last_crossed_gate_hop: last_crossed,
            }
        } else {
            ManeuverTraversalPhase::PreGate {
                next_gate_hop: next_gate.unwrap_or(first_gate),
            }
        };
        Ok(Some(ManeuverTraversalState {
            route: state.route,
            maneuver_occurrence_index: u32::try_from(maneuver_index)
                .map_err(|_| crate::StepError::WaitingInvariantViolation)?,
            phase,
        }))
    }

    fn waiting_events_for(
        &self,
        old: crate::VehicleState,
        next: crate::VehicleState,
        tick: u64,
    ) -> [Option<WaitingTransitionEvent>; 5] {
        let update_sequence = self
            .waiting_plan_by_vehicle
            .get(old.handle.index() as usize)
            .copied()
            .flatten()
            .filter(|plan| plan.vehicle == old.handle)
            .map_or_else(
                || {
                    self.waiting_next_state_index
                        .get(old.handle.index() as usize)
                        .copied()
                        .and_then(|encoded| encoded.checked_sub(1))
                        .unwrap_or(u32::MAX)
                },
                |plan| plan.vehicle_update_sequence,
            );
        let plan = self
            .waiting_plan_by_vehicle
            .get(old.handle.index() as usize)
            .copied()
            .flatten()
            .filter(|plan| plan.vehicle == old.handle);
        let mut events = [None; 5];
        let mut event_count = 0_usize;
        {
            let mut push_event = |event| {
                events[event_count] = Some(event);
                event_count += 1;
            };
            if let Some(plan) = plan
                && let (Some(reason), Some(stop_hop), Some(zone), Some(maneuver_index)) = (
                    plan.projection,
                    plan.stop_hop,
                    plan.stop_zone,
                    plan.stop_maneuver_index,
                )
                && front_strictly_upstream(
                    self.compiled_route(old.route).expect("live route"),
                    &old,
                    stop_hop,
                    self.revision.traffic().lane_lengths_millimetres(),
                )
                && front_at_hop_boundary(
                    self.compiled_route(next.route).expect("live route"),
                    &next,
                    stop_hop,
                    self.revision.traffic().lane_lengths_millimetres(),
                )
            {
                push_event(WaitingTransitionEvent {
                    tick,
                    vehicle: old.handle,
                    vehicle_update_sequence: update_sequence,
                    anchor: WaitingRouteAnchor {
                        route: old.route,
                        maneuver_occurrence_index: maneuver_index,
                        hop: stop_hop,
                    },
                    kind: WaitingTransitionKind::ProjectionApplied { zone, reason },
                });
            }
            if let Some(membership) = old.waiting_membership
                && next.route_edge_index > membership.release_hop
            {
                let maneuver_index = old
                    .maneuver_traversal
                    .map_or(0, |traversal| traversal.maneuver_occurrence_index);
                push_event(WaitingTransitionEvent {
                    tick,
                    vehicle: old.handle,
                    vehicle_update_sequence: update_sequence,
                    anchor: WaitingRouteAnchor {
                        route: old.route,
                        maneuver_occurrence_index: maneuver_index,
                        hop: membership.release_hop,
                    },
                    kind: WaitingTransitionKind::Left {
                        zone: membership.waiting_zone,
                        admission_sequence: membership.admission_sequence,
                    },
                });
            }
            if let Some(plan) = plan
                && let Some(sequence) = plan.admission_sequence
            {
                push_event(WaitingTransitionEvent {
                    tick,
                    vehicle: old.handle,
                    vehicle_update_sequence: update_sequence,
                    anchor: WaitingRouteAnchor {
                        route: old.route,
                        maneuver_occurrence_index: plan.maneuver_index,
                        hop: plan.entry_hop,
                    },
                    kind: WaitingTransitionKind::Entered {
                        zone: plan.zone,
                        admission_sequence: sequence,
                    },
                });
                if next.route_edge_index > plan.release_hop {
                    push_event(WaitingTransitionEvent {
                        tick,
                        vehicle: old.handle,
                        vehicle_update_sequence: update_sequence,
                        anchor: WaitingRouteAnchor {
                            route: old.route,
                            maneuver_occurrence_index: plan.maneuver_index,
                            hop: plan.release_hop,
                        },
                        kind: WaitingTransitionKind::Left {
                            zone: plan.zone,
                            admission_sequence: sequence,
                        },
                    });
                }
            }
            let completed = old.maneuver_traversal.and_then(|traversal| {
                next.maneuver_traversal
                    .is_none()
                    .then_some(traversal.maneuver_occurrence_index)
            });
            let completed = completed.or_else(|| {
                let plan = plan?;
                let compiled = self.compiled_route(old.route)?;
                let maneuver = compiled.maneuvers.get(plan.maneuver_index as usize)?;
                (plan.admission_sequence.is_some()
                    && next.route_edge_index >= maneuver.exit_route_edge_index)
                    .then_some(plan.maneuver_index)
            });
            if let Some(maneuver_index) = completed
                && let Some(maneuver) = self
                    .compiled_route(old.route)
                    .and_then(|compiled| compiled.maneuvers.get(maneuver_index as usize))
            {
                push_event(WaitingTransitionEvent {
                    tick,
                    vehicle: old.handle,
                    vehicle_update_sequence: update_sequence,
                    anchor: WaitingRouteAnchor {
                        route: old.route,
                        maneuver_occurrence_index: maneuver_index,
                        hop: maneuver.exit_route_edge_index.saturating_sub(1),
                    },
                    kind: WaitingTransitionKind::ManeuverTraversalCompleted {
                        maneuver_occurrence_index: maneuver_index,
                    },
                });
            }
        }
        events
    }

    /// 为没有既有 Waiting authority 的 Active 候选建立唯一可推导的初始状态。
    pub(crate) fn validate_waiting_bootstrap(
        &self,
        route: RouteHandle,
        cursor: usize,
        vehicle_length_mm: u32,
    ) -> Result<Option<ManeuverTraversalState>, WaitingBindingError> {
        let compiled = self
            .compiled_route(route)
            .ok_or(WaitingBindingError::InvalidRoute)?;
        let cursor_u32 = u32::try_from(cursor).map_err(|_| WaitingBindingError::InvalidRoute)?;

        for occurrence in &compiled.waiting {
            if cursor_u32 <= occurrence.release_hop
                && vehicle_length_mm > occurrence.storage_length_mm
            {
                return Err(WaitingBindingError::VehicleTooLong);
            }
        }

        let mut initial = None;
        for (maneuver_index, maneuver) in compiled.maneuvers.iter().enumerate() {
            if !compiled
                .waiting
                .iter()
                .any(|waiting| waiting.maneuver_index as usize == maneuver_index)
            {
                continue;
            }
            if cursor_u32 < maneuver.entry_route_edge_index
                || cursor_u32 >= maneuver.exit_route_edge_index
            {
                continue;
            }
            let first_gate_hop =
                first_gate_hop(compiled, maneuver).ok_or(WaitingBindingError::InvalidRoute)?;
            if cursor_u32 > first_gate_hop {
                return Err(WaitingBindingError::StatefulManeuverInterior);
            }
            let candidate = ManeuverTraversalState {
                route,
                maneuver_occurrence_index: u32::try_from(maneuver_index)
                    .map_err(|_| WaitingBindingError::InvalidRoute)?,
                phase: ManeuverTraversalPhase::PreGate {
                    next_gate_hop: first_gate_hop,
                },
            };
            if initial.replace(candidate).is_some() {
                return Err(WaitingBindingError::InvalidRoute);
            }
        }
        Ok(initial)
    }

    pub(crate) fn waiting_state_valid(&self) -> bool {
        let mut total = 0_usize;
        for (zone_index, state) in self.waiting_zones.iter().copied().enumerate() {
            let zone = WaitingZoneOrdinal::from_raw(
                u32::try_from(zone_index).expect("waiting zone index fits u32"),
            );
            let Some(view) = self.revision.traffic().relations().waiting_zone(zone) else {
                return false;
            };
            if state.occupancy > view.max_occupancy()
                || (state.occupancy == 0) != (state.head.is_none() && state.tail.is_none())
            {
                return false;
            }
            let mut previous = None;
            let mut current = state.head;
            let mut count = 0_u32;
            let mut previous_sequence = None;
            while let Some(vehicle) = current {
                let Some(vehicle_state) = self.vehicle_state(vehicle) else {
                    return false;
                };
                let Some(membership) = vehicle_state.waiting_membership else {
                    return false;
                };
                if membership.waiting_zone != zone
                    || membership.admission_sequence >= state.next_admission_sequence
                {
                    return false;
                }
                if previous_sequence.is_some_and(|value| value >= membership.admission_sequence) {
                    return false;
                }
                let Some(link) = self.waiting_links.get(vehicle.index() as usize).copied() else {
                    return false;
                };
                if link.previous != previous {
                    return false;
                }
                previous = Some(vehicle);
                previous_sequence = Some(membership.admission_sequence);
                current = link.next;
                count = match count.checked_add(1) {
                    Some(value) => value,
                    None => return false,
                };
                total = match total.checked_add(1) {
                    Some(value) if value <= self.live_order.len() => value,
                    _ => return false,
                };
            }
            if count != state.occupancy || previous != state.tail {
                return false;
            }
        }
        let semantic_count = self
            .live_order
            .iter()
            .filter(|vehicle| {
                self.vehicle_state(**vehicle)
                    .is_some_and(|state| state.waiting_membership.is_some())
            })
            .count();
        semantic_count == total
    }

    pub(crate) fn restored_waiting_authority_valid(&self, state: crate::VehicleState) -> bool {
        if self.derive_waiting_traversal(state).ok() != Some(state.maneuver_traversal) {
            return false;
        }
        let Some(compiled) = self.compiled_route(state.route) else {
            return false;
        };
        if state.status == crate::VehicleStatus::Active
            && compiled.waiting.iter().any(|occurrence| {
                state.route_edge_index <= occurrence.release_hop
                    && state.length_mm > occurrence.storage_length_mm
            })
        {
            return false;
        }
        let Some(membership) = state.waiting_membership else {
            return true;
        };
        let Some(traversal) = state.maneuver_traversal else {
            return false;
        };
        compiled.waiting.iter().any(|occurrence| {
            occurrence.maneuver_index == traversal.maneuver_occurrence_index
                && occurrence.zone == membership.waiting_zone
                && occurrence.release_hop == membership.release_hop
                && waiting_membership_cursor_valid(state.route_edge_index, occurrence)
                && state.length_mm <= occurrence.storage_length_mm
        })
    }

    pub(crate) fn waiting_snapshot_storage_valid(&self) -> bool {
        for state in self.waiting_zones.iter().copied() {
            let mut used = 0_u64;
            let mut current = state.head;
            let mut has_front = false;
            let mut previous_rank = None;
            let mut previous_length_mm = 0_u32;
            while let Some(vehicle) = current {
                let Some(vehicle_state) = self.vehicle_state(vehicle) else {
                    return false;
                };
                let Some(profile) = self
                    .revision
                    .traffic()
                    .relations()
                    .vehicle_profile(vehicle_state.profile)
                else {
                    return false;
                };
                if has_front {
                    let Some(next) = used.checked_add(u64::from(profile.min_gap_mm())) else {
                        return false;
                    };
                    used = next;
                }
                let Some(next) = used.checked_add(u64::from(vehicle_state.length_mm)) else {
                    return false;
                };
                used = next;
                let Some(membership) = vehicle_state.waiting_membership else {
                    return false;
                };
                let Some(traversal) = vehicle_state.maneuver_traversal else {
                    return false;
                };
                let Some((compiled, occurrence)) = self
                    .compiled_route(vehicle_state.route)
                    .and_then(|compiled| {
                        compiled
                            .waiting
                            .iter()
                            .find(|occurrence| {
                                occurrence.maneuver_index == traversal.maneuver_occurrence_index
                                    && occurrence.zone == membership.waiting_zone
                                    && occurrence.release_hop == membership.release_hop
                            })
                            .map(|occurrence| (compiled, occurrence))
                    })
                else {
                    return false;
                };
                let Some(rank) =
                    post_step_physical_rank(compiled, vehicle_state, occurrence.release_hop)
                else {
                    return false;
                };
                if previous_rank.is_some_and(|previous| previous >= rank) {
                    return false;
                }
                if let Some(front_rank) = previous_rank {
                    let Some(front_distance_mm) = waiting_front_distance_mm(front_rank, rank)
                    else {
                        return false;
                    };
                    let Some(required_distance_mm) =
                        u64::from(previous_length_mm).checked_add(u64::from(profile.min_gap_mm()))
                    else {
                        return false;
                    };
                    if front_distance_mm < required_distance_mm {
                        return false;
                    }
                }
                if used > u64::from(occurrence.storage_length_mm) {
                    return false;
                }
                previous_rank = Some(rank);
                previous_length_mm = vehicle_state.length_mm;
                has_front = true;
                current = self.waiting_links[vehicle.index() as usize].next;
            }
        }
        true
    }

    pub(crate) fn rebuild_waiting_member_rows(&mut self) {
        self.waiting_member_rows.clear();
        for (zone_index, state) in self.waiting_zones.iter().copied().enumerate() {
            let zone = WaitingZoneOrdinal::from_raw(
                u32::try_from(zone_index).expect("waiting zone index fits u32"),
            );
            let mut current = state.head;
            while let Some(vehicle) = current {
                let membership = self
                    .vehicle_state(vehicle)
                    .and_then(|state| state.waiting_membership)
                    .expect("validated Waiting queue member");
                self.waiting_member_rows.push(WaitingZoneMember {
                    zone,
                    vehicle,
                    admission_sequence: membership.admission_sequence,
                    release_hop: membership.release_hop,
                });
                current = self.waiting_links[vehicle.index() as usize].next;
            }
        }
    }

    pub(crate) fn rebuild_waiting_aggregate_from_semantics(&mut self) -> bool {
        self.waiting_member_rows.clear();
        for vehicle in self.live_order.iter().copied() {
            let Some(state) = self.vehicle_state(vehicle) else {
                return false;
            };
            if let Some(membership) = state.waiting_membership {
                if self.waiting_member_rows.len() == self.waiting_member_rows.capacity() {
                    return false;
                }
                self.waiting_member_rows.push(WaitingZoneMember {
                    zone: membership.waiting_zone,
                    vehicle,
                    admission_sequence: membership.admission_sequence,
                    release_hop: membership.release_hop,
                });
            }
        }
        self.waiting_member_rows.sort_by_key(|member| {
            (
                member.zone.raw(),
                member.admission_sequence,
                member.vehicle.index(),
                member.vehicle.generation(),
            )
        });
        if self.waiting_member_rows.windows(2).any(|pair| {
            pair[0].zone == pair[1].zone && pair[0].admission_sequence == pair[1].admission_sequence
        }) {
            return false;
        }
        for state in &mut self.waiting_zones {
            state.occupancy = 0;
            state.head = None;
            state.tail = None;
        }
        self.waiting_links.fill(WaitingQueueLink::default());
        for index in 0..self.waiting_member_rows.len() {
            let member = self.waiting_member_rows[index];
            let Some(zone) = self.waiting_zones.get(member.zone.index()) else {
                return false;
            };
            if member.admission_sequence >= zone.next_admission_sequence {
                return false;
            }
            self.append_waiting_member(
                member.vehicle,
                WaitingMembership {
                    waiting_zone: member.zone,
                    admission_sequence: member.admission_sequence,
                    release_hop: member.release_hop,
                },
            );
        }
        self.waiting_state_valid() && self.waiting_snapshot_storage_valid()
    }

    pub(crate) fn unlink_waiting_member(
        &mut self,
        vehicle: VehicleHandle,
        membership: WaitingMembership,
    ) {
        let index = vehicle.index() as usize;
        let link = self.waiting_links[index];
        let zone = &mut self.waiting_zones[membership.waiting_zone.index()];
        match link.previous {
            Some(previous) => self.waiting_links[previous.index() as usize].next = link.next,
            None => zone.head = link.next,
        }
        match link.next {
            Some(next) => self.waiting_links[next.index() as usize].previous = link.previous,
            None => zone.tail = link.previous,
        }
        zone.occupancy = zone
            .occupancy
            .checked_sub(1)
            .expect("validated occupancy covers member");
        self.waiting_links[index] = WaitingQueueLink::default();
    }

    pub(crate) fn append_waiting_member(
        &mut self,
        vehicle: VehicleHandle,
        membership: WaitingMembership,
    ) {
        let index = vehicle.index() as usize;
        let zone = &mut self.waiting_zones[membership.waiting_zone.index()];
        let previous = zone.tail;
        self.waiting_links[index] = WaitingQueueLink {
            previous,
            next: None,
        };
        if let Some(previous) = previous {
            self.waiting_links[previous.index() as usize].next = Some(vehicle);
        } else {
            zone.head = Some(vehicle);
        }
        zone.tail = Some(vehicle);
        zone.occupancy = zone
            .occupancy
            .checked_add(1)
            .expect("admission preflight guarantees occupancy room");
    }
}

const fn waiting_membership_cursor_valid(
    route_edge_index: u32,
    occurrence: &crate::tables::WaitingOccurrence,
) -> bool {
    route_edge_index > occurrence.entry_hop && route_edge_index <= occurrence.release_hop
}

fn waiting_front_distance_mm(front: (u8, u64), follower: (u8, u64)) -> Option<u64> {
    match (front.0, follower.0) {
        (0, 0) | (1, 1) => follower.1.checked_sub(front.1),
        (0, 1) => u64::MAX.checked_sub(front.1)?.checked_add(follower.1),
        _ => None,
    }
}

fn post_step_physical_rank(
    compiled: &CompiledRoute,
    state: &crate::VehicleState,
    release_hop: u32,
) -> Option<(u8, u64)> {
    let release_index = usize::try_from(release_hop).ok()?.checked_add(1)?;
    let cursor = usize::try_from(state.route_edge_index).ok()?;
    if cursor >= release_index {
        let distance = distance_to_occurrence_start(
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            release_index,
            0,
            cursor,
        )?;
        let BoundedDistance::Finite(to_edge_start) = distance else {
            return None;
        };
        let beyond = u64::from(to_edge_start).checked_add(u64::from(state.progress_mm))?;
        Some((0, u64::MAX - beyond))
    } else {
        let distance = distance_to_occurrence_start(
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            cursor,
            state.progress_mm,
            release_index,
        )?;
        let BoundedDistance::Finite(remaining) = distance else {
            return None;
        };
        Some((1, u64::from(remaining)))
    }
}

fn not_required_gate_anchor(
    compiled: &CompiledRoute,
    preview: &crate::VehicleState,
    hop: usize,
    lengths: &[u32],
) -> Option<(u32, u32)> {
    let hop_u32 = u32::try_from(hop).ok()?;
    compiled.hop_gate.get(hop).copied().flatten()?;
    if compiled
        .waiting
        .iter()
        .any(|occurrence| occurrence.entry_hop == hop_u32)
    {
        return None;
    }
    if preview.route_edge_index <= hop_u32
        && !front_at_hop_boundary(compiled, preview, hop_u32, lengths)
    {
        return None;
    }
    compiled
        .maneuvers
        .iter()
        .enumerate()
        .find(|(_, maneuver)| {
            hop_u32 >= maneuver.entry_route_edge_index && hop_u32 < maneuver.exit_route_edge_index
        })
        .and_then(|(index, _)| u32::try_from(index).ok())
        .map(|maneuver_index| (maneuver_index, hop_u32))
}

fn reserve_waiting_exact<T>(
    values: &mut Vec<T>,
    additional: usize,
) -> Result<(), crate::StepError> {
    let missing = additional.saturating_sub(values.capacity().saturating_sub(values.len()));
    if missing == 0 {
        return Ok(());
    }
    #[cfg(test)]
    if waiting_reservation_injected_failure() {
        return Err(crate::StepError::WaitingScratchAllocFailed);
    }
    values
        .try_reserve_exact(missing)
        .map_err(|_| crate::StepError::WaitingScratchAllocFailed)
}

fn front_at_hop_boundary(
    compiled: &CompiledRoute,
    state: &crate::VehicleState,
    hop: u32,
    lengths: &[u32],
) -> bool {
    if state.route_edge_index != hop {
        return false;
    }
    compiled
        .edges
        .get(hop as usize)
        .and_then(|edge| lengths.get(edge.index()))
        .is_some_and(|length| state.progress_mm == *length)
}

fn front_strictly_upstream(
    compiled: &CompiledRoute,
    state: &crate::VehicleState,
    hop: u32,
    lengths: &[u32],
) -> bool {
    if state.route_edge_index < hop {
        return true;
    }
    if state.route_edge_index > hop {
        return false;
    }
    compiled
        .edges
        .get(hop as usize)
        .and_then(|edge| lengths.get(edge.index()))
        .is_some_and(|length| state.progress_mm < *length)
}

const fn transition_kind_rank(kind: WaitingTransitionKind) -> u8 {
    match kind {
        WaitingTransitionKind::ProjectionApplied { .. } => 0,
        WaitingTransitionKind::Left { .. } => 1,
        WaitingTransitionKind::Entered { .. } => 2,
        WaitingTransitionKind::ManeuverTraversalCompleted { .. } => 3,
    }
}

fn first_gate_hop(
    compiled: &CompiledRoute,
    maneuver: &crate::tables::ManeuverOccurrence,
) -> Option<u32> {
    let start = usize::try_from(maneuver.entry_route_edge_index).ok()?;
    let end = usize::try_from(maneuver.exit_route_edge_index).ok()?;
    compiled
        .hop_gate
        .get(start..end)?
        .iter()
        .position(Option::is_some)
        .and_then(|offset| u32::try_from(start.checked_add(offset)?).ok())
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;
    use std::sync::Arc;
    use std::time::Instant;

    use laneflow_compiler::{
        CompilationUnitBuilder, CompileLimits, Compiler, IidmVehicleProfileInput, JunctionInput,
        JunctionReference, LaneEdgeInput, LaneEdgeReference, ManeuverGateInput,
        ManeuverGateReference, ManeuverPathInput, ManeuverPathReference, MovementInput,
        MovementReference, ParticipantClassInput, ParticipantClassReference, PortableDiffBase,
        PortableEmissionProvenance, SignalControlInput, SourceModuleHeader,
        SourceModuleHeaderInput, StopLineInput, StopLineReference, SyntheticModuleBuilder,
        VehicleProfileInput, WaitingZoneInput, derive_canonical_stable_id_v1,
        emit_portable_candidate,
    };
    use laneflow_format::{
        FormatLimits, check_canonical_network_input, check_post_emission_bundle,
    };
    use laneflow_static_contract::{
        EntityKind, LaneEdgeId, ManeuverPathOrdinal, VehicleProfileOrdinal,
    };
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    use super::*;
    use crate::migration_journal::{
        JournalRecord, VEHICLE_DELTA_BYTES, VehicleDelta, waiting_zone_delta_stream,
    };
    use crate::{
        CommittedNetworkSource, CutoverPreflightLimits, LfcaOriginBinding, MigrationPolicyKind,
        NetworkRevisionCutoverDescriptor, PublishedLfcaReference, RouteRegisterInput,
        SnapshotRestoreError, SnapshotRestoreLimits, TickInput, TrafficWorld, VehicleSpawnInput,
        WorldConfig, deterministic_state_digest, encode_lfrs, restore_lfrs,
    };

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
    );

    fn waiting_world() -> (TrafficWorld, RouteHandle, crate::tables::WaitingOccurrence) {
        waiting_world_at_delta(100)
    }

    fn waiting_world_at_delta(
        delta_time_ms: u64,
    ) -> (TrafficWorld, RouteHandle, crate::tables::WaitingOccurrence) {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
            .expect("checked fixture");
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::Omit,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("revision");
        let origin = *revision.canonical_origin();
        let mut world = TrafficWorld::install(
            Arc::clone(&revision),
            WorldConfig::new(16, 8, 1_024, 1_024, 1, delta_time_ms),
            CommittedNetworkSource::Published {
                reference: PublishedLfcaReference::new(
                    "fixture://waiting-runtime",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .expect("source"),
            },
            82,
        )
        .expect("install");
        let edges = world
            .traffic()
            .maneuvers()
            .maneuver_path(ManeuverPathOrdinal::from_raw(0))
            .expect("main path")
            .edges()
            .to_vec();
        let route = world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route");
        let occurrence = world
            .compiled_route(route)
            .and_then(|compiled| compiled.waiting.first().copied())
            .expect("Waiting occurrence");
        (world, route, occurrence)
    }

    fn waiting_scale_revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
        waiting_scale_revision_with(8.0, 1)
    }

    fn waiting_scale_revision_with(
        storage_length_meters: f64,
        max_occupancy: u32,
    ) -> Arc<laneflow_static_network::SharedNetworkRevision> {
        const NS: &str = "city/waiting-scale";
        const STEM_COUNT: usize = 64;
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: NS,
                source_document_key: "waiting-scale.document",
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x31; 32],
                frontend_options_digest: [0x42; 32],
                random_seed: Some(282),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .expect("source header");
        let mut module = SyntheticModuleBuilder::new(header, &limits).expect("module");
        module
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "road-user",
                extends: None,
            })
            .expect("class")
            .add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: "car",
                participant_class: ParticipantClassReference::local("road-user"),
                iidm: IidmVehicleProfileInput {
                    length_meters: 4.5,
                    desired_speed_meters_per_second: 13.75,
                    min_gap_meters: 2.0,
                    time_headway_seconds: 1.4,
                    max_acceleration_meters_per_second_squared: 1.8,
                    comfortable_deceleration_meters_per_second_squared: 2.0,
                    emergency_deceleration_meters_per_second_squared: 4.5,
                },
            })
            .expect("profile");
        let stems = (0..STEM_COUNT)
            .map(|index| format!("stem-{index:02}"))
            .collect::<Vec<_>>();
        for (index, key) in stems.iter().enumerate() {
            let successor = stems.get(index + 1).map_or("entry", String::as_str);
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: 10_000.0,
                    speed_limit_meters_per_second: 13.75,
                    successors: &[LaneEdgeReference::local(successor)],
                })
                .expect("stem");
        }
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "entry",
                length_meters: 10_000.0,
                speed_limit_meters_per_second: 13.75,
                successors: &[LaneEdgeReference::local("storage")],
            })
            .expect("entry")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "storage",
                length_meters: storage_length_meters,
                speed_limit_meters_per_second: 13.75,
                successors: &[LaneEdgeReference::local("after-release")],
            })
            .expect("storage")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "after-release",
                length_meters: 12.0,
                speed_limit_meters_per_second: 13.75,
                successors: &[LaneEdgeReference::local("exit")],
            })
            .expect("after release")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "exit",
                length_meters: 12.0,
                speed_limit_meters_per_second: 13.75,
                successors: &[],
            })
            .expect("exit")
            .add_junction(JunctionInput {
                junction_key: "junction",
            })
            .expect("junction")
            .add_movement(MovementInput {
                movement_key: "movement",
                junction: JunctionReference::local("junction"),
                directed_entry_approach_key: "approach-in",
                directed_exit_approach_key: "approach-out",
            })
            .expect("movement")
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: "path",
                movement: MovementReference::local("movement"),
                entry_edge: LaneEdgeReference::local("entry"),
                internal_edges: &[
                    LaneEdgeReference::local("storage"),
                    LaneEdgeReference::local("after-release"),
                ],
                exit_edge: LaneEdgeReference::local("exit"),
            })
            .expect("path")
            .add_stop_line(StopLineInput {
                stop_line_key: "stop-entry",
                lane_edge: LaneEdgeReference::local("entry"),
            })
            .expect("entry stop")
            .add_stop_line(StopLineInput {
                stop_line_key: "stop-release",
                lane_edge: LaneEdgeReference::local("storage"),
            })
            .expect("release stop")
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: "gate-entry",
                maneuver_path: ManeuverPathReference::local("path"),
                transition_index: 0,
                stop_line: StopLineReference::local("stop-entry"),
                signal_control: SignalControlInput::None,
            })
            .expect("entry gate")
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: "gate-release",
                maneuver_path: ManeuverPathReference::local("path"),
                transition_index: 1,
                stop_line: StopLineReference::local("stop-release"),
                signal_control: SignalControlInput::None,
            })
            .expect("release gate")
            .add_waiting_zone(WaitingZoneInput {
                waiting_zone_key: "waiting",
                maneuver_path: ManeuverPathReference::local("path"),
                entry_gate: ManeuverGateReference::local("gate-entry"),
                release_gate: ManeuverGateReference::local("gate-release"),
                max_occupancy,
            })
            .expect("WaitingZone");
        let mut unit = CompilationUnitBuilder::new(limits);
        unit.add_synthetic_module(module.finish().expect("finished module"))
            .expect("unit module");
        let output = Compiler::new()
            .compile(unit.build().expect("unit"))
            .expect("compiled");
        let provenance =
            PortableEmissionProvenance::try_new("laneflow-waiting-scale-v1").expect("provenance");
        let candidate = emit_portable_candidate(
            &output,
            &provenance,
            FormatLimits::HARD,
            PortableDiffBase::Genesis,
        )
        .expect("portable candidate");
        let checked = check_post_emission_bundle(
            candidate.canonical_artifact().bytes(),
            candidate.source_map().bytes(),
            candidate.semantic_diff().bytes(),
            candidate.expected_semantic_diff_base(),
            FormatLimits::HARD,
        )
        .expect("checked bundle");
        build_shared_network_revision(
            checked.canonical_network_input(),
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::Omit,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("revision")
    }

    fn waiting_scale_world(
        revision: Arc<laneflow_static_network::SharedNetworkRevision>,
        vehicle_count: u32,
    ) -> (TrafficWorld, WaitingZoneOrdinal) {
        const NS: &str = "city/waiting-scale";
        const STEM_COUNT: usize = 64;
        const EDGE_LENGTH_MM: u64 = 10_000_000;
        const SPACING_MM: u64 = 6_500;
        let origin = *revision.canonical_origin();
        let mut world = TrafficWorld::install(
            Arc::clone(&revision),
            WorldConfig::new(vehicle_count, 1, 1_024, 1_024, 1, 4),
            CommittedNetworkSource::Published {
                reference: PublishedLfcaReference::new(
                    "fixture://waiting-scale",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .expect("source"),
            },
            u64::from(vehicle_count),
        )
        .expect("install");
        let limits = CompileLimits::p100_initial_v1();
        let mut keys = (0..STEM_COUNT)
            .map(|index| format!("stem-{index:02}"))
            .collect::<Vec<_>>();
        keys.extend([
            "entry".into(),
            "storage".into(),
            "after-release".into(),
            "exit".into(),
        ]);
        let edges = keys
            .iter()
            .map(|key| {
                let stable = derive_canonical_stable_id_v1(EntityKind::LaneEdge, NS, key, &limits)
                    .expect("stable edge");
                revision
                    .identity()
                    .ordinal(LaneEdgeId::from_untyped(stable))
                    .expect("edge ordinal")
            })
            .collect::<Vec<_>>();
        let route = world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route");
        let profile = VehicleProfileOrdinal::from_raw(0);
        let profile_view = world
            .traffic()
            .relations()
            .vehicle_profile(profile)
            .expect("profile");
        let profile_class = profile_view.class();
        let profile_length_mm = profile_view.length_mm();
        let entry_boundary_mm = u64::try_from(STEM_COUNT + 1).expect("count") * EDGE_LENGTH_MM;
        for update_sequence in 0..vehicle_count {
            let distance = 1_u64 + u64::from(update_sequence) * SPACING_MM;
            let absolute = entry_boundary_mm
                .checked_sub(distance)
                .expect("corridor room");
            let route_edge_index =
                u32::try_from(absolute / EDGE_LENGTH_MM).expect("route occurrence");
            let progress_mm = u32::try_from(absolute % EDGE_LENGTH_MM).expect("progress");
            let handle = VehicleHandle::new(update_sequence, 0);
            let traversal = world
                .validate_waiting_bootstrap(route, route_edge_index as usize, profile_length_mm)
                .expect("bootstrap");
            world.vehicles.push(crate::tables::VehicleSlot {
                generation: 0,
                state: Some(crate::VehicleState {
                    handle,
                    profile,
                    class: profile_class,
                    route,
                    route_edge_index,
                    progress_mm,
                    carry_um: 0,
                    speed_mm_s: if update_sequence == 0 { 10_000 } else { 0 },
                    length_mm: profile_length_mm,
                    status: crate::VehicleStatus::Active,
                    maneuver_traversal: traversal,
                    waiting_membership: None,
                }),
            });
            world.live_order.push(handle);
            world.active_order.push(handle);
        }
        world.routes[route.index() as usize].live_vehicles = vehicle_count;
        world.rebuild_occupancy_index().expect("occupancy");
        let zone = world
            .compiled_route(route)
            .and_then(|compiled| compiled.waiting.first())
            .map(|occurrence| occurrence.zone)
            .expect("Waiting occurrence");
        (world, zone)
    }

    fn waiting_retained_bytes(world: &TrafficWorld) -> u64 {
        let bytes = world.waiting_zones.len() * size_of::<WaitingZoneState>()
            + world.waiting_links.len() * size_of::<WaitingQueueLink>()
            + world.waiting_member_rows.capacity() * size_of::<WaitingZoneMember>()
            + world.waiting_claims.capacity() * size_of::<WaitingAdmissionClaim>()
            + world.waiting_plans.capacity() * size_of::<WaitingVehiclePlan>()
            + world.waiting_plan_by_vehicle.len() * size_of::<Option<WaitingVehiclePlan>>()
            + world.waiting_next_state_index.len() * size_of::<u32>()
            + world.waiting_staged_decisions.capacity() * size_of::<WaitingDecision>()
            + world.waiting_staged_events.capacity() * size_of::<WaitingTransitionEvent>()
            + world.waiting_next_counters.len() * size_of::<u64>()
            + world.waiting_staged_occupancy.len() * size_of::<u32>()
            + world.waiting_staged_storage_mm.len() * size_of::<u64>()
            + world.latest_waiting_decisions.capacity() * size_of::<WaitingDecision>()
            + world.latest_waiting_events.capacity() * size_of::<WaitingTransitionEvent>();
        u64::try_from(bytes).expect("retained bytes")
    }

    fn waiting_prepare_samples(world: &mut TrafficWorld) -> (u128, u128) {
        let mut samples = Vec::with_capacity(21);
        for sample in 0..24 {
            let started = Instant::now();
            world.prepare_waiting_step(0.004).expect("Waiting prepare");
            let elapsed = started.elapsed().as_nanos();
            if sample >= 3 {
                samples.push(elapsed);
            }
        }
        samples.sort_unstable();
        (samples[10], samples[19])
    }

    #[test]
    #[ignore = "manual release-mode Waiting 10k/100k scale evidence"]
    fn waiting_10k_100k_scale_evidence() {
        let revision = waiting_scale_revision();
        let (mut product_world, product_zone) = waiting_scale_world(Arc::clone(&revision), 10_000);
        let product_retained_bytes = waiting_retained_bytes(&product_world);
        let (product_p50_ns, product_p95_ns) = waiting_prepare_samples(&mut product_world);
        assert!(
            product_p95_ns <= 4_000_000,
            "10k Waiting p95 hard gate exceeded: {product_p95_ns} ns"
        );
        product_world
            .step(TickInput::new(4))
            .expect("10k correctness step");
        assert_eq!(
            product_world
                .waiting_zone(product_zone)
                .expect("10k zone")
                .occupancy(),
            1
        );
        assert!(product_world.waiting_state_valid());
        drop(product_world);

        let (mut scale_world, scale_zone) = waiting_scale_world(revision, 100_000);
        let scale_retained_bytes = waiting_retained_bytes(&scale_world);
        let (scale_p50_ns, scale_p95_ns) = waiting_prepare_samples(&mut scale_world);
        scale_world
            .step(TickInput::new(4))
            .expect("100k correctness step");
        assert_eq!(
            scale_world
                .waiting_zone(scale_zone)
                .expect("100k zone")
                .occupancy(),
            1
        );
        assert!(scale_world.waiting_state_valid());
        assert!(
            scale_retained_bytes <= product_retained_bytes.saturating_mul(11),
            "Waiting retained memory grows faster than the 10x population plus 10% margin"
        );
        assert!(
            scale_p95_ns <= product_p95_ns.saturating_mul(20).max(1),
            "100k Waiting p95 grows faster than the 10x population plus 2x margin"
        );

        eprintln!(
            "waiting-g2-scale-evidence 10k_p50_ns={product_p50_ns} \
             10k_p95_ns={product_p95_ns} 10k_retained_bytes={product_retained_bytes} \
             100k_p50_ns={scale_p50_ns} 100k_p95_ns={scale_p95_ns} \
             100k_retained_bytes={scale_retained_bytes}"
        );
    }

    #[test]
    fn successful_entry_persists_counter_membership_journal_and_despawn_release() {
        let (mut world, route, occurrence) = waiting_world();
        let entry_edge = world.route_edges(route).expect("route")[occurrence.entry_hop as usize];
        let entry_length = world.traffic().lane_lengths_millimetres()[entry_edge.index()];
        let vehicle = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                occurrence.entry_hop,
                entry_length - 100,
                8_000,
            ))
            .expect("spawn upstream of entry");
        world.arm_migration_journal(16 * 1_024).expect("arm");
        world.step(TickInput::new(100)).expect("entry step");

        let membership = world
            .vehicle(vehicle)
            .and_then(|state| state.waiting_membership())
            .expect("entry commits membership");
        assert_eq!(membership.waiting_zone(), occurrence.zone);
        assert_eq!(membership.admission_sequence(), 0);
        assert_eq!(membership.release_hop(), occurrence.release_hop);
        let zone = world.waiting_zone(occurrence.zone).expect("zone");
        assert_eq!(zone.occupancy(), 1);
        assert_eq!(zone.next_admission_sequence(), 1);
        assert_eq!(world.waiting_zone_members().len(), 1);
        assert!(matches!(
            world.latest_waiting_decisions(),
            [WaitingDecision {
                outcome: WaitingDecisionOutcome::Granted,
                ..
            }]
        ));
        assert!(world.latest_waiting_events().iter().any(|event| matches!(
            event.kind(),
            WaitingTransitionKind::Entered {
                admission_sequence: 0,
                ..
            }
        )));

        let records = world
            .migration_journal()
            .expect("journal")
            .records_from(0)
            .collect::<Vec<_>>();
        let JournalRecord::Tick {
            entries,
            waiting_zones,
            ..
        } = records[0]
        else {
            panic!("entry produces tick record");
        };
        assert_eq!(entries.len(), VEHICLE_DELTA_BYTES);
        let delta = VehicleDelta::decode(entries);
        assert!(delta.traversal_present);
        assert!(delta.membership_present);
        assert_eq!(delta.admission_sequence, 0);
        assert_eq!(
            waiting_zone_delta_stream(waiting_zones).collect::<Vec<_>>(),
            [(occurrence.zone, 1)]
        );
        world.disarm_migration_journal();

        let captured = world.capture_snapshot().expect("capture");
        let digest = deterministic_state_digest(&captured).expect("digest");
        let restored = restore_lfrs(
            &encode_lfrs(&captured),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
        )
        .expect("restore Waiting state");
        let restored_world = restored.world();
        assert_eq!(
            restored_world
                .waiting_zone(occurrence.zone)
                .expect("restored zone")
                .next_admission_sequence(),
            1
        );
        assert_eq!(restored_world.waiting_zone_members().len(), 1);
        assert_eq!(
            deterministic_state_digest(&restored_world.capture_snapshot().expect("capture"))
                .expect("digest"),
            digest
        );

        let cutover_target = world.revision();
        let origin = *cutover_target.canonical_origin();
        let descriptor = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(origin),
            LfcaOriginBinding::from_canonical_origin(origin),
            None,
            MigrationPolicyKind::SameRevisionRestore,
            world.world_binding(),
        );
        let cutover_source = world.committed_source().clone();
        let _cutover_events = world
            .cutover_same_revision(
                cutover_target,
                cutover_source,
                &descriptor,
                &CutoverPreflightLimits::new(1_048_576),
            )
            .expect("same-revision Waiting cutover");
        assert_eq!(
            world
                .vehicle(vehicle)
                .and_then(|state| state.waiting_membership()),
            Some(membership)
        );
        assert_eq!(
            world
                .waiting_zone(occurrence.zone)
                .expect("cutover zone")
                .next_admission_sequence(),
            1
        );

        let release = world
            .despawn_vehicle(vehicle)
            .expect("despawn member")
            .waiting_release
            .expect("typed release payload");
        assert_eq!(release.waiting_zone(), occurrence.zone);
        assert_eq!(release.admission_sequence(), 0);
        let zone = world.waiting_zone(occurrence.zone).expect("zone");
        assert_eq!(zone.occupancy(), 0);
        assert_eq!(zone.next_admission_sequence(), 1);
        assert!(world.waiting_zone_members().is_empty());

        let empty_with_history = world.capture_snapshot().expect("empty history capture");
        assert_eq!(empty_with_history.waiting_zones.len(), 1);
        assert_eq!(empty_with_history.waiting_zones[0].occupancy, 0);
        assert_eq!(
            empty_with_history.waiting_zones[0].next_admission_sequence,
            1
        );
        let empty_digest = deterministic_state_digest(&empty_with_history).expect("empty digest");
        let mut changed_counter = empty_with_history.clone();
        changed_counter.waiting_zones[0].next_admission_sequence = 2;
        assert_ne!(
            deterministic_state_digest(&changed_counter).expect("changed digest"),
            empty_digest
        );
        let restored_empty = restore_lfrs(
            &encode_lfrs(&empty_with_history),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
        )
        .expect("restore empty zone history");
        assert_eq!(
            restored_empty
                .world()
                .waiting_zone(occurrence.zone)
                .expect("restored empty zone")
                .next_admission_sequence(),
            1
        );
    }

    #[test]
    fn malformed_waiting_snapshot_aggregate_fails_closed() {
        let (mut world, route, occurrence) = waiting_world();
        let entry_edge = world.route_edges(route).expect("route")[occurrence.entry_hop as usize];
        let entry_length = world.traffic().lane_lengths_millimetres()[entry_edge.index()];
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                occurrence.entry_hop,
                entry_length - 100,
                8_000,
            ))
            .expect("vehicle");
        world.step(TickInput::new(100)).expect("entry");
        let snapshot = world.capture_snapshot().expect("capture");
        let restore = |captured: &crate::CapturedSnapshot| {
            restore_lfrs(
                &encode_lfrs(captured),
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
            )
            .map(|_| ())
        };

        let mut occupancy_mismatch = snapshot.clone();
        occupancy_mismatch.waiting_zones[0].occupancy = 0;
        assert_eq!(
            restore(&occupancy_mismatch),
            Err(SnapshotRestoreError::WaitingInvariantViolation)
        );

        let mut exhausted_counter = snapshot.clone();
        exhausted_counter.waiting_zones[0].next_admission_sequence = 0;
        assert_eq!(
            restore(&exhausted_counter),
            Err(SnapshotRestoreError::WaitingInvariantViolation)
        );

        let mut duplicate_zone = snapshot.clone();
        duplicate_zone
            .waiting_zones
            .push(duplicate_zone.waiting_zones[0]);
        assert_eq!(
            restore(&duplicate_zone),
            Err(SnapshotRestoreError::InvalidWaitingZoneState)
        );

        let mut membership_without_traversal = snapshot;
        membership_without_traversal.vehicles[0].maneuver_traversal = None;
        assert!(matches!(
            restore(&membership_without_traversal),
            Err(SnapshotRestoreError::InvalidWaitingAuthority { .. })
        ));
    }

    #[test]
    fn restore_rejects_actual_waiting_gap_below_profile_minimum() {
        let revision = waiting_scale_revision_with(20.0, 2);
        let (mut world, zone) = waiting_scale_world(revision, 2);
        world.step(TickInput::new(4)).expect("admit front member");

        let front = VehicleHandle::new(0, 0);
        let front_state = world.vehicle_state(front).copied().expect("front state");
        let membership = front_state.waiting_membership.expect("front membership");
        let profile = world
            .traffic()
            .relations()
            .vehicle_profile(front_state.profile)
            .expect("profile");
        let front_progress_mm = 15_000_u32;
        let follower_progress_at_minimum = front_progress_mm
            .checked_sub(front_state.length_mm)
            .and_then(|value| value.checked_sub(profile.min_gap_mm()))
            .expect("20 metre storage fits both vehicles");

        let mut exact_gap = world.capture_snapshot().expect("capture");
        let front_index = exact_gap
            .vehicles
            .iter()
            .position(|vehicle| vehicle.waiting_membership.is_some())
            .expect("front row");
        let follower_index = exact_gap
            .vehicles
            .iter()
            .position(|vehicle| vehicle.waiting_membership.is_none())
            .expect("follower row");
        let traversal = exact_gap.vehicles[front_index]
            .maneuver_traversal
            .expect("front traversal");
        let mut follower_membership = exact_gap.vehicles[front_index]
            .waiting_membership
            .expect("front membership row");
        follower_membership.admission_sequence = 1;
        exact_gap.vehicles[front_index].route_edge_index = membership.release_hop;
        exact_gap.vehicles[front_index].progress_mm = front_progress_mm;
        exact_gap.vehicles[front_index].speed_mm_s = 0;
        exact_gap.vehicles[follower_index].route_edge_index = membership.release_hop;
        exact_gap.vehicles[follower_index].progress_mm = follower_progress_at_minimum;
        exact_gap.vehicles[follower_index].speed_mm_s = 0;
        exact_gap.vehicles[follower_index].maneuver_traversal = Some(traversal);
        exact_gap.vehicles[follower_index].waiting_membership = Some(follower_membership);
        let zone_identity = exact_gap.waiting_zones[0].waiting_zone;
        let zone_state = exact_gap
            .waiting_zones
            .iter_mut()
            .find(|state| state.waiting_zone == zone_identity)
            .expect("zone state");
        zone_state.occupancy = 2;
        zone_state.next_admission_sequence = 2;

        let restore = |captured: &crate::CapturedSnapshot| {
            restore_lfrs(
                &encode_lfrs(captured),
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
            )
            .map(|_| ())
        };
        restore(&exact_gap).expect("exact profile minimum gap is valid");

        let mut one_millimetre_short = exact_gap;
        one_millimetre_short.vehicles[follower_index].progress_mm =
            follower_progress_at_minimum + 1;
        assert_eq!(
            restore(&one_millimetre_short),
            Err(SnapshotRestoreError::WaitingInvariantViolation)
        );
        assert_eq!(
            world.waiting_zone(zone).expect("source zone").occupancy(),
            1,
            "malformed restore must not mutate the source world"
        );
    }

    #[test]
    fn waiting_rebind_rejects_target_cursor_past_release_gate() {
        let revision = waiting_scale_revision();
        let (mut world, _) = waiting_scale_world(revision, 1);
        world.step(TickInput::new(4)).expect("admit member");
        let vehicle = VehicleHandle::new(0, 0);
        let state = world.vehicle_state(vehicle).copied().expect("member state");
        let membership = state.waiting_membership.expect("membership");
        let target_cursor = membership
            .release_hop
            .checked_add(1)
            .expect("fixture release has a following internal edge");
        let compiled = world.compiled_route(state.route).expect("compiled route");
        let traversal = state.maneuver_traversal.expect("traversal");
        let maneuver = compiled
            .maneuvers
            .get(traversal.maneuver_occurrence_index as usize)
            .expect("maneuver");
        assert!(target_cursor < maneuver.exit_route_edge_index);
        assert_eq!(
            world.rebind_waiting_authority(state, state.route, target_cursor as usize),
            Err(WaitingBindingError::AuthorityMismatch)
        );
    }

    #[test]
    fn same_tick_release_does_not_return_physical_storage_to_later_candidate() {
        let (mut world, route, occurrence) = waiting_world();
        let entry_edge = world.route_edges(route).expect("route")[occurrence.entry_hop as usize];
        let entry_length = world.traffic().lane_lengths_millimetres()[entry_edge.index()];
        let member = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                occurrence.entry_hop,
                entry_length - 100,
                8_000,
            ))
            .expect("member spawn");
        world.step(TickInput::new(100)).expect("member entry");
        assert!(
            world
                .vehicle(member)
                .and_then(|state| state.waiting_membership())
                .is_some()
        );

        let release_edge =
            world.route_edges(route).expect("route")[occurrence.release_hop as usize];
        let release_length = world.traffic().lane_lengths_millimetres()[release_edge.index()];
        let member_index = member.index() as usize;
        let member_state = world.vehicles[member_index].state.as_mut().expect("member");
        member_state.route_edge_index = occurrence.release_hop;
        member_state.progress_mm = release_length;
        member_state.speed_mm_s = 0;
        member_state.carry_um = 0;
        member_state.maneuver_traversal = Some(ManeuverTraversalState {
            route,
            maneuver_occurrence_index: occurrence.maneuver_index,
            phase: ManeuverTraversalPhase::Committed {
                last_crossed_gate_hop: occurrence.entry_hop,
            },
        });

        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                occurrence.entry_hop,
                entry_length - 100,
                8_000,
            ))
            .expect("follower spawn");
        world
            .signal_aspects
            .fill(laneflow_static_contract::SignalAspect::Green);
        world
            .step(TickInput::new(100))
            .expect("release and no-grant step");

        assert!(
            world
                .vehicle(member)
                .and_then(|state| state.waiting_membership())
                .is_none()
        );
        assert!(
            world
                .vehicle(follower)
                .and_then(|state| state.waiting_membership())
                .is_none()
        );
        let zone = world.waiting_zone(occurrence.zone).expect("zone");
        assert_eq!(zone.occupancy(), 0);
        assert_eq!(zone.next_admission_sequence(), 1);
        assert!(world.latest_waiting_decisions().iter().any(|decision| {
            decision.vehicle() == follower
                && decision.outcome()
                    == WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::PhysicalStorage)
        }));
        assert!(world.latest_waiting_events().iter().any(|event| {
            event.vehicle() == member && matches!(event.kind(), WaitingTransitionKind::Left { .. })
        }));
        assert!(world.latest_waiting_events().iter().any(|event| {
            event.vehicle() == follower
                && matches!(
                    event.kind(),
                    WaitingTransitionKind::ProjectionApplied {
                        reason: WaitingProjectionReason::PhysicalStorage,
                        ..
                    }
                )
        }));
    }

    #[test]
    fn release_gate_wins_zero_travel_tie_then_green_leader_stop_is_committed() {
        let (mut world, route, occurrence) = waiting_world();
        let entry_edge = world.route_edges(route).expect("route")[occurrence.entry_hop as usize];
        let entry_length = world.traffic().lane_lengths_millimetres()[entry_edge.index()];
        let member = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                occurrence.entry_hop,
                entry_length - 100,
                8_000,
            ))
            .expect("member spawn");
        world.step(TickInput::new(100)).expect("member entry");

        let release_edge =
            world.route_edges(route).expect("route")[occurrence.release_hop as usize];
        let release_length = world.traffic().lane_lengths_millimetres()[release_edge.index()];
        let member_state = world.vehicles[member.index() as usize]
            .state
            .as_mut()
            .expect("member");
        member_state.route_edge_index = occurrence.release_hop;
        member_state.progress_mm = release_length;
        member_state.speed_mm_s = 0;
        member_state.carry_um = 0;
        member_state.maneuver_traversal = Some(ManeuverTraversalState {
            route,
            maneuver_occurrence_index: occurrence.maneuver_index,
            phase: ManeuverTraversalPhase::Committed {
                last_crossed_gate_hop: occurrence.entry_hop,
            },
        });
        let profile = world
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .expect("profile");
        let _leader = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                occurrence.release_hop + 1,
                profile.length_mm(),
                0,
            ))
            .expect("leader touching release boundary");

        world.step(TickInput::new(100)).expect("red release tie");
        assert!(matches!(
            world
                .vehicle(member)
                .and_then(|state| state.maneuver_traversal())
                .expect("traversal")
                .phase(),
            ManeuverTraversalPhase::Waiting { release_gate_hop }
                if release_gate_hop == occurrence.release_hop
        ));
        assert!(world.latest_waiting_decisions().iter().any(|decision| {
            decision.vehicle() == member
                && decision.anchor().hop() == occurrence.release_hop
                && decision.outcome() == WaitingDecisionOutcome::NotRequired
        }));

        world
            .signal_aspects
            .fill(laneflow_static_contract::SignalAspect::Green);
        world.step(TickInput::new(100)).expect("green leader stop");
        let state = world.vehicle(member).expect("member");
        assert_eq!(state.route_edge_index(), occurrence.release_hop);
        assert_eq!(state.progress_mm(), release_length);
        assert!(matches!(
            state.maneuver_traversal().expect("traversal").phase(),
            ManeuverTraversalPhase::Committed {
                last_crossed_gate_hop
            } if last_crossed_gate_hop == occurrence.entry_hop
        ));
        assert!(state.waiting_membership().is_some());
    }

    #[test]
    fn counter_exhaustion_and_each_checked_scratch_reservation_leave_step_atomic() {
        let atomic_failure = |fail_after: Option<usize>| {
            let (mut world, route, occurrence) = waiting_world();
            let entry_edge =
                world.route_edges(route).expect("route")[occurrence.entry_hop as usize];
            let entry_length = world.traffic().lane_lengths_millimetres()[entry_edge.index()];
            let _vehicle = world
                .spawn_vehicle(VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    occurrence.entry_hop,
                    entry_length - 100,
                    8_000,
                ))
                .expect("vehicle");
            let before = world.capture_snapshot().expect("before");
            let guard = fail_after.map(fail_waiting_reservation_after);
            let error = world
                .step(TickInput::new(100))
                .expect_err("injected failure");
            assert_eq!(error, crate::StepError::WaitingScratchAllocFailed);
            assert_eq!(world.capture_snapshot().expect("after"), before);
            assert!(world.latest_waiting_decisions().is_empty());
            assert!(world.latest_waiting_events().is_empty());
            drop(guard);
            world.step(TickInput::new(100)).expect("retry");
        };
        atomic_failure(Some(0));
        atomic_failure(Some(1));

        let (mut world, route, occurrence) = waiting_world();
        let entry_edge = world.route_edges(route).expect("route")[occurrence.entry_hop as usize];
        let entry_length = world.traffic().lane_lengths_millimetres()[entry_edge.index()];
        let _vehicle = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                occurrence.entry_hop,
                entry_length - 100,
                8_000,
            ))
            .expect("vehicle");
        world.waiting_zones[occurrence.zone.index()].next_admission_sequence = u64::MAX;
        let before = world.capture_snapshot().expect("before exhaustion");
        let next_state_capacity = world.next_states.capacity();
        assert_eq!(
            world.step(TickInput::new(100)),
            Err(crate::StepError::WaitingAdmissionSequenceExhausted)
        );
        assert_eq!(world.capture_snapshot().expect("after exhaustion"), before);
        assert_eq!(world.next_states.capacity(), next_state_capacity);
    }

    #[test]
    fn leader_constraint_prevents_rear_request_while_physical_front_enters() {
        let (mut world, route, occurrence) = waiting_world();
        let entry_edge = world.route_edges(route).expect("route")[occurrence.entry_hop as usize];
        let entry_length = world.traffic().lane_lengths_millimetres()[entry_edge.index()];
        let rear = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                occurrence.entry_hop,
                entry_length - 7_100,
                8_000,
            ))
            .expect("rear first in live order");
        let front = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                occurrence.entry_hop,
                entry_length - 100,
                8_000,
            ))
            .expect("physical front second in live order");
        world.vehicles[rear.index() as usize]
            .state
            .as_mut()
            .expect("rear")
            .speed_mm_s = 100_000;
        world.vehicles[front.index() as usize]
            .state
            .as_mut()
            .expect("front")
            .speed_mm_s = 100_000;

        world.step(TickInput::new(100)).expect("ordered admission");
        assert!(
            world
                .vehicle(front)
                .and_then(|state| state.waiting_membership())
                .is_some()
        );
        assert!(
            world
                .vehicle(rear)
                .and_then(|state| state.waiting_membership())
                .is_none()
        );
        assert!(world.latest_waiting_decisions().iter().any(|decision| {
            decision.vehicle() == front && decision.outcome() == WaitingDecisionOutcome::Granted
        }));
        assert!(
            world
                .latest_waiting_decisions()
                .iter()
                .all(|decision| decision.vehicle() != rear)
        );
    }

    #[test]
    fn same_tick_enter_leave_orders_events_and_despawn_without_member_is_absent() {
        let (mut world, route, mut occurrence) = waiting_world();
        occurrence.release_hop = occurrence.entry_hop;
        world.routes[route.index() as usize]
            .compiled
            .as_mut()
            .expect("route")
            .waiting[0] = occurrence;
        let entry_edge = world.route_edges(route).expect("route")[occurrence.entry_hop as usize];
        let entry_length = world.traffic().lane_lengths_millimetres()[entry_edge.index()];
        let vehicle = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                occurrence.entry_hop,
                entry_length - 100,
                8_000,
            ))
            .expect("vehicle");
        world.vehicles[vehicle.index() as usize]
            .state
            .as_mut()
            .expect("vehicle")
            .speed_mm_s = 1_000_000;
        world
            .signal_aspects
            .fill(laneflow_static_contract::SignalAspect::Green);
        world.step(TickInput::new(100)).expect("enter and leave");

        assert!(
            world
                .vehicle(vehicle)
                .and_then(|state| state.waiting_membership())
                .is_none()
        );
        assert_eq!(
            world
                .waiting_zone(occurrence.zone)
                .expect("zone")
                .next_admission_sequence(),
            1
        );
        let transition_kinds = world
            .latest_waiting_events()
            .iter()
            .filter(|event| event.vehicle() == vehicle)
            .map(|event| event.kind())
            .collect::<Vec<_>>();
        assert!(
            matches!(
                transition_kinds.as_slice(),
                [
                    WaitingTransitionKind::Left { .. },
                    WaitingTransitionKind::Entered { .. }
                ]
            ),
            "{transition_kinds:?}"
        );
        let latest = world.latest_waiting_events().to_vec();
        let event_cursor = world.event_cursor();
        let record = world.despawn_vehicle(vehicle).expect("despawn");
        assert!(record.waiting_release.is_none());
        assert_eq!(world.latest_waiting_events(), latest);
        assert_eq!(world.event_cursor(), event_cursor);
    }
}
