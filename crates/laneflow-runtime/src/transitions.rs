//! 已验证的 Waiting/Conflict 转移到唯一公开事件批次的投影。

use laneflow_static_contract::{ManeuverGateOrdinal, WaitingZoneOrdinal};

use crate::{
    ConflictPassageOccurrenceLocator, ConflictPassageRange, DownstreamRoutePoint,
    ManeuverTraversalPhase, RouteHandle, StepError, TrafficWorld, VehicleHandle, VehicleState,
    WaitingProjectionReason, WaitingRouteAnchor,
};

/// 事件的语义 Gate/机动出现项和实际触发位置。车尾事件不锚回已驶过的入口。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrafficTransitionAnchor {
    route: RouteHandle,
    maneuver_occurrence_index: u32,
    hop: u32,
    position: DownstreamRoutePoint,
}

impl TrafficTransitionAnchor {
    pub(crate) fn at_gate(anchor: WaitingRouteAnchor) -> Self {
        Self {
            route: anchor.route,
            maneuver_occurrence_index: anchor.maneuver_occurrence_index,
            hop: anchor.hop,
            position: DownstreamRoutePoint::new(
                anchor.hop.checked_add(1).expect("checked Gate hop"),
                0,
                0,
            )
            .expect("canonical Gate position"),
        }
    }

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
    #[must_use]
    pub const fn position(self) -> DownstreamRoutePoint {
        self.position
    }
}

/// 一次成功固定步进实际提交的领域转移；Grant 仅暂存而未过门时没有资源事件。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrafficTransitionKind {
    ProjectionApplied {
        zone: WaitingZoneOrdinal,
        reason: WaitingProjectionReason,
    },
    GateCrossed {
        gate: ManeuverGateOrdinal,
    },
    WaitingLeft {
        zone: WaitingZoneOrdinal,
        admission_sequence: u64,
    },
    WaitingEntered {
        zone: WaitingZoneOrdinal,
        admission_sequence: u64,
    },
    ReservationAcquired {
        passage_range: ConflictPassageRange,
    },
    ConflictEntered {
        passage: ConflictPassageOccurrenceLocator,
    },
    ConflictCleared {
        passage: ConflictPassageOccurrenceLocator,
    },
    ReservationReleased {
        passage_range: ConflictPassageRange,
    },
    ManeuverTraversalCompleted {
        maneuver_occurrence_index: u32,
    },
}

impl TrafficTransitionKind {
    fn order(self) -> (u8, u32, u32, u32) {
        match self {
            Self::ProjectionApplied { zone, .. } => (0, zone.raw(), 0, 0),
            Self::GateCrossed { gate } => (1, gate.raw(), 0, 0),
            Self::WaitingLeft { zone, .. } => (2, zone.raw(), 0, 0),
            Self::WaitingEntered { zone, .. } => (3, zone.raw(), 0, 0),
            Self::ReservationAcquired { .. } => (4, 0, 0, 0),
            Self::ConflictEntered { passage } | Self::ConflictCleared { passage } => {
                let address = passage.address();
                (
                    if matches!(self, Self::ConflictEntered { .. }) {
                        5
                    } else {
                        6
                    },
                    address.zone().raw(),
                    address.stream().raw(),
                    address.passage_local_index(),
                )
            }
            Self::ReservationReleased { .. } => (7, 0, 0, 0),
            Self::ManeuverTraversalCompleted { .. } => (8, 0, 0, 0),
        }
    }
}

/// 刚完成成功固定步进的统一交通转移事件。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrafficTransitionEvent {
    pub(crate) tick: u64,
    pub(crate) vehicle: VehicleHandle,
    pub(crate) vehicle_update_sequence: u32,
    pub(crate) anchor: TrafficTransitionAnchor,
    pub(crate) kind: TrafficTransitionKind,
}

impl TrafficTransitionEvent {
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
    pub const fn anchor(self) -> TrafficTransitionAnchor {
        self.anchor
    }
    #[must_use]
    pub const fn kind(self) -> TrafficTransitionKind {
        self.kind
    }
}

impl TrafficWorld {
    pub(crate) fn stage_transition_events(
        &mut self,
        updates: &[(usize, VehicleState)],
        tick: u64,
    ) -> Result<(), StepError> {
        let mut count = 0_usize;
        self.visit_transition_events(updates, tick, |_| {
            count = count.saturating_add(1);
        })?;
        if count == usize::MAX {
            return Err(StepError::ConflictInvariantViolation);
        }
        crate::conflict_tick::reserve(&mut self.staged_transition_events, count)?;
        let mut events = std::mem::take(&mut self.staged_transition_events);
        let result = self.visit_transition_events(updates, tick, |event| events.push(event));
        if result.is_err() {
            events.clear();
        }
        events.sort_unstable_by_key(|event| {
            (
                event.vehicle_update_sequence,
                event.anchor.position,
                event.kind.order(),
                event.anchor.maneuver_occurrence_index,
                event.anchor.hop,
            )
        });
        self.staged_transition_events = events;
        result
    }

    pub(crate) fn visit_transition_events(
        &self,
        updates: &[(usize, VehicleState)],
        tick: u64,
        mut emit: impl FnMut(TrafficTransitionEvent),
    ) -> Result<(), StepError> {
        let mut passage_cursor = 0;
        for (sequence, vehicle) in self.live_order.iter().copied().enumerate() {
            let Some(update) = self.next_state_by_vehicle[vehicle.index() as usize].checked_sub(1)
            else {
                continue;
            };
            let old = *self
                .vehicle_state(vehicle)
                .ok_or(StepError::ConflictInvariantViolation)?;
            let next = updates[update as usize].1;
            let sequence =
                u32::try_from(sequence).map_err(|_| StepError::ConflictInvariantViolation)?;
            let compiled = self
                .compiled_route(old.route)
                .ok_or(StepError::ConflictInvariantViolation)?;
            self.visit_waiting_events(old, next, tick, sequence, &mut emit);
            let prepared = self.conflict_motion_by_vehicle[vehicle.index() as usize]
                .and_then(|plan| plan.grant_index)
                .and_then(|index| self.conflict_grants.get(index.get() as usize - 1))
                .filter(|grant| next.route_edge_index > grant.gate_hop);
            let mut push = |anchor, kind| {
                emit(TrafficTransitionEvent {
                    tick,
                    vehicle,
                    vehicle_update_sequence: sequence,
                    anchor,
                    kind,
                })
            };
            let first_hop = prepared.map_or(old.route_edge_index, |grant| {
                old.route_edge_index.min(grant.gate_hop)
            });
            let first = compiled.gate_hops.partition_point(|hop| *hop < first_hop);
            for hop in compiled.gate_hops[first..]
                .iter()
                .copied()
                .take_while(|hop| *hop < next.route_edge_index)
            {
                let index = compiled
                    .maneuvers
                    .partition_point(|item| item.exit_route_edge_index <= hop);
                let gate =
                    compiled.hop_gate[hop as usize].ok_or(StepError::ConflictInvariantViolation)?;
                push(
                    TrafficTransitionAnchor::at_gate(WaitingRouteAnchor {
                        route: old.route,
                        maneuver_occurrence_index: index as u32,
                        hop,
                    }),
                    TrafficTransitionKind::GateCrossed { gate },
                );
            }
            let range = prepared.and_then(|grant| grant.passage_range).or_else(|| {
                self.conflict_arbiter
                    .reservation(vehicle)
                    .map(|value| value.passage_range())
            });
            let mut release_anchor = None;
            if let Some(range) = range {
                let anchor = TrafficTransitionAnchor::at_gate(WaitingRouteAnchor {
                    route: old.route,
                    maneuver_occurrence_index: range.maneuver_occurrence_index(),
                    hop: range.admission_gate_hop(),
                });
                if prepared.and_then(|grant| grant.passage_range).is_some() {
                    push(
                        anchor,
                        TrafficTransitionKind::ReservationAcquired {
                            passage_range: range,
                        },
                    );
                }
                if !next.maneuver_traversal.is_some_and(|traversal| {
                    matches!(traversal.phase, ManeuverTraversalPhase::Clearing { .. })
                }) {
                    let position = self
                        .reservation_downstream_claim_plan(range, next.length_mm)
                        .map_err(|_| StepError::ConflictInvariantViolation)?
                        .target();
                    let anchor = TrafficTransitionAnchor { position, ..anchor };
                    push(
                        anchor,
                        TrafficTransitionKind::ReservationReleased {
                            passage_range: range,
                        },
                    );
                    release_anchor = Some(anchor);
                }
            }
            // passage staging 和 updates 均按 active_order（live_order 的投影）产生，线性合并。
            while let Some(transition) = self
                .conflict_passage_transitions
                .get(passage_cursor)
                .filter(|transition| transition.vehicle == vehicle)
            {
                passage_cursor += 1;
                let occurrence = compiled.conflicts[transition.occurrence_index as usize];
                let passage = self
                    .conflict_passage_occurrence_locator(old.route, transition.occurrence_index)
                    .ok_or(StepError::ConflictInvariantViolation)?;
                let anchor = TrafficTransitionAnchor {
                    route: old.route,
                    maneuver_occurrence_index: occurrence.maneuver_index,
                    hop: occurrence.admission_hop,
                    position: DownstreamRoutePoint::new(
                        occurrence.entry.route_edge_index,
                        occurrence.entry.progress_mm,
                        0,
                    )
                    .ok_or(StepError::ConflictInvariantViolation)?,
                };
                if transition.enter {
                    push(anchor, TrafficTransitionKind::ConflictEntered { passage });
                }
                if transition.clear {
                    let position = crate::conflict::downstream_claim_target(
                        &compiled.edges,
                        self.revision.traffic().lane_lengths_millimetres(),
                        DownstreamRoutePoint::new(
                            occurrence.clearance.route_edge_index,
                            occurrence.clearance.progress_mm,
                            0,
                        )
                        .ok_or(StepError::ConflictInvariantViolation)?,
                        next.length_mm,
                    )
                    .map_err(|_| StepError::ConflictInvariantViolation)?;
                    push(
                        TrafficTransitionAnchor { position, ..anchor },
                        TrafficTransitionKind::ConflictCleared { passage },
                    );
                }
            }
            let first_exit = compiled
                .maneuvers
                .partition_point(|item| item.exit_route_edge_index <= old.route_edge_index);
            let last_exit = compiled
                .maneuvers
                .partition_point(|item| item.exit_route_edge_index <= next.route_edge_index);
            let delayed = old
                .maneuver_traversal
                .filter(|traversal| {
                    matches!(traversal.phase, ManeuverTraversalPhase::Clearing { .. })
                        && (traversal.maneuver_occurrence_index as usize) < first_exit
                })
                .map(|traversal| traversal.maneuver_occurrence_index as usize);
            for index in delayed.into_iter().chain(first_exit..last_exit) {
                let maneuver = compiled
                    .maneuvers
                    .get(index)
                    .ok_or(StepError::ConflictInvariantViolation)?;
                let pending_clearance = next.maneuver_traversal.is_some_and(|traversal| {
                    traversal.maneuver_occurrence_index as usize == index
                        && matches!(traversal.phase, ManeuverTraversalPhase::Clearing { .. })
                });
                if !pending_clearance {
                    let mut anchor = TrafficTransitionAnchor::at_gate(WaitingRouteAnchor {
                        route: old.route,
                        maneuver_occurrence_index: index as u32,
                        hop: maneuver
                            .exit_route_edge_index
                            .checked_sub(1)
                            .ok_or(StepError::ConflictInvariantViolation)?,
                    });
                    if let Some(release) = release_anchor
                        .filter(|release| release.maneuver_occurrence_index as usize == index)
                    {
                        anchor.position = anchor.position.max(release.position);
                    }
                    push(
                        anchor,
                        TrafficTransitionKind::ManeuverTraversalCompleted {
                            maneuver_occurrence_index: index as u32,
                        },
                    );
                }
            }
        }
        if passage_cursor != self.conflict_passage_transitions.len() {
            return Err(StepError::ConflictInvariantViolation);
        }
        Ok(())
    }
}
