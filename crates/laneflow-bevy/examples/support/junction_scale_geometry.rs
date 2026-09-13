//! Static route geometry for the private frozen-workload observer.

use super::{Check, ensure};
use laneflow_runtime::{
    ConflictPassageOccurrenceLocator, ManeuverTraversalPhase, TrafficTransitionEvent,
    TrafficTransitionKind, TrafficWorld, VehicleState,
};
use laneflow_static_contract::{ManeuverPathOrdinal, WaitingZoneOrdinal};
use laneflow_static_network::ConflictPathAnchor;

type Point = (u32, u32, u16);

pub(super) fn point(state: VehicleState) -> Point {
    (
        state.route_edge_index(),
        state.progress_mm(),
        state.carry_um(),
    )
}

pub(super) fn crossed<T: Ord>(before: T, after: T, target: T) -> bool {
    before < target && after >= target
}

#[derive(Clone)]
struct Maneuver {
    start: u32,
    exit: u32,
    path: ManeuverPathOrdinal,
    completion: Point,
    waiting: bool,
    gates: Vec<u32>,
}

#[derive(Clone)]
pub(super) struct RouteGeometry {
    lengths: Vec<u32>,
    maneuvers: Vec<Maneuver>,
}

impl RouteGeometry {
    pub(super) fn new(world: &TrafficWorld, state: VehicleState) -> Check<Self> {
        let route = world.route_edges(state.route()).unwrap();
        let network = world.traffic().maneuvers();
        let mut result = Self {
            lengths: route
                .iter()
                .map(|edge| world.traffic().lane_lengths_millimetres()[edge.index()])
                .collect(),
            maneuvers: Vec::new(),
        };
        // Match complete static paths, including paths without gates. Occurrence
        // numbering comes from route order, not from an event or traversal row.
        for start in 0..route.len() - 1 {
            for candidate in network.transition_candidates(route[start]).unwrap_or(&[]) {
                let path = network.maneuver_path(candidate.maneuver_path()).unwrap();
                if candidate.transition_index() != 0 || !route[start..].starts_with(path.edges()) {
                    continue;
                }
                ensure(
                    result
                        .maneuvers
                        .last()
                        .is_none_or(|previous| previous.exit <= start as u32),
                    "event_causality",
                    || "overlapping static path occurrences".into(),
                )?;
                let exit = (start + path.edges().len() - 1) as u32;
                let mut completion = (exit, 0, 0);
                let revision = world.revision();
                for &stream in revision
                    .conflict()
                    .maneuver_path_participant_streams(candidate.maneuver_path())
                    .unwrap()
                {
                    for passage in revision
                        .conflict()
                        .participant_stream(stream)
                        .unwrap()
                        .passages()
                    {
                        let clearance = result.anchor(world, start as u32, passage.exit())?;
                        completion = completion.max(result.advance(clearance, state.length_mm())?);
                    }
                }
                result.maneuvers.push(Maneuver {
                    start: start as u32,
                    exit,
                    path: candidate.maneuver_path(),
                    completion,
                    waiting: !path.waiting_zones().is_empty(),
                    gates: path
                        .maneuver_gates()
                        .iter()
                        .map(|&gate| {
                            start as u32
                                + world
                                    .traffic()
                                    .relations()
                                    .maneuver_gate(gate)
                                    .unwrap()
                                    .transition_index()
                        })
                        .collect(),
                });
            }
        }
        ensure(
            result
                .maneuvers
                .windows(2)
                .all(|pair| pair[0].completion < pair[1].completion),
            "event_causality",
            || "frozen route completion positions are not ordered".into(),
        )?;
        Ok(result)
    }

    pub(super) fn expected_traversal(
        &self,
        world: &TrafficWorld,
        state: VehicleState,
        release_restrictive: bool,
    ) -> Option<(u32, ManeuverTraversalPhase)> {
        if let Some(reservation) = world.conflict_reservation(state.handle()) {
            let range = reservation.passage_range();
            return Some((
                range.maneuver_occurrence_index(),
                ManeuverTraversalPhase::Clearing {
                    admission_gate_hop: range.admission_gate_hop(),
                },
            ));
        }
        let cursor = state.route_edge_index();
        let occurrence = self.maneuvers.partition_point(|item| item.exit <= cursor);
        let item = self.maneuvers.get(occurrence)?;
        if !item.waiting || cursor < item.start {
            return None;
        }
        // Only Waiting-bearing paths have persistent pre-gate/committed state;
        // Conflict-only paths retain a traversal while a reservation owns it.
        let next = item.gates.partition_point(|&hop| hop < cursor);
        let phase = if let Some(member) = state.waiting_membership()
            && cursor == member.release_hop()
            && state.progress_mm() == self.lengths[cursor as usize]
            && release_restrictive
        {
            ManeuverTraversalPhase::Waiting {
                release_gate_hop: member.release_hop(),
            }
        } else if next > 0 {
            ManeuverTraversalPhase::Committed {
                last_crossed_gate_hop: item.gates[next - 1],
            }
        } else {
            ManeuverTraversalPhase::PreGate {
                next_gate_hop: *item.gates.first()?,
            }
        };
        Some((occurrence as u32, phase))
    }

    pub(super) fn crossed_completions(
        &self,
        before: Point,
        after: Point,
        owner: usize,
        output: &mut Vec<(usize, u32)>,
    ) {
        let first = self
            .maneuvers
            .partition_point(|item| item.completion <= before);
        for (index, item) in self.maneuvers.iter().enumerate().skip(first) {
            if item.completion > after {
                break;
            }
            output.push((owner, index as u32));
        }
    }

    pub(super) fn gate_occurrence(&self, hop: u32) -> Check<u32> {
        let index = self.maneuvers.partition_point(|item| item.exit <= hop);
        ensure(
            self.maneuvers
                .get(index)
                .is_some_and(|item| item.start <= hop && hop < item.exit),
            "event_causality",
            || "semantic hop has no static maneuver occurrence".into(),
        )?;
        Ok(index as u32)
    }

    fn anchor(&self, world: &TrafficWorld, start: u32, anchor: ConflictPathAnchor) -> Check<Point> {
        let (hop, progress) = match anchor {
            ConflictPathAnchor::Gate(gate) => (
                start
                    + world
                        .traffic()
                        .relations()
                        .maneuver_gate(gate)
                        .unwrap()
                        .transition_index()
                    + 1,
                0,
            ),
            ConflictPathAnchor::EdgeBoundary(index) => (start + index, 0),
            ConflictPathAnchor::Interior {
                path_edge_index,
                progress_millimetres,
            } => (start + path_edge_index, progress_millimetres),
        };
        if hop as usize == self.lengths.len() && progress == 0 {
            return Ok((hop - 1, *self.lengths.last().unwrap(), 0));
        }
        self.advance((hop, progress, 0), 0)
    }

    fn advance(&self, from: Point, distance: u32) -> Check<Point> {
        let mut remaining = u64::from(from.1) + u64::from(distance);
        for hop in from.0 as usize..self.lengths.len() {
            let length = u64::from(self.lengths[hop]);
            if remaining < length || (remaining == length && hop + 1 == self.lengths.len()) {
                return Ok((hop as u32, remaining as u32, 0));
            }
            remaining -= length;
        }
        Err((
            "event_causality",
            "frozen route has insufficient tail-clearance geometry".into(),
        ))
    }

    pub(super) fn passage_points(
        &self,
        world: &TrafficWorld,
        state: VehicleState,
        locator: ConflictPassageOccurrenceLocator,
    ) -> Check<(Point, Point)> {
        let item = self
            .maneuvers
            .get(locator.maneuver_occurrence_index() as usize)
            .ok_or_else(|| ("event_causality", "passage occurrence missing".into()))?;
        let revision = world.revision();
        let stream = revision
            .conflict()
            .participant_stream(locator.address().stream())
            .ok_or_else(|| ("event_causality", "passage stream missing".into()))?;
        let passage = stream
            .passages()
            .get(locator.address().passage_local_index() as usize)
            .ok_or_else(|| ("event_causality", "static passage missing".into()))?;
        let gate = world
            .traffic()
            .relations()
            .maneuver_gate(passage.admission_gate())
            .unwrap();
        ensure(
            locator.route() == state.route()
                && stream.maneuver_path() == item.path
                && locator.admission_gate_hop() == item.start + gate.transition_index()
                && locator.address().zone() == passage.conflict_zone(),
            "event_causality",
            || "passage locator differs from static route occurrence".into(),
        )?;
        Ok((
            self.anchor(world, item.start, passage.entry())?,
            self.advance(
                self.anchor(world, item.start, passage.exit())?,
                state.length_mm(),
            )?,
        ))
    }

    pub(super) fn check_anchor(
        &self,
        world: &TrafficWorld,
        state: VehicleState,
        event: TrafficTransitionEvent,
    ) -> Check {
        let anchor = event.anchor();
        let occurrence = self.gate_occurrence(anchor.hop())?;
        match event.kind() {
            TrafficTransitionKind::ConflictEntered { passage }
            | TrafficTransitionKind::ConflictCleared { passage } => {
                ensure(
                    anchor.hop() == passage.admission_gate_hop()
                        && occurrence == passage.maneuver_occurrence_index(),
                    "event_causality",
                    || "passage event semantic anchor differs from its admission occurrence".into(),
                )?;
            }
            TrafficTransitionKind::ReservationAcquired { passage_range }
            | TrafficTransitionKind::ReservationReleased { passage_range } => {
                ensure(
                    anchor.hop() == passage_range.admission_gate_hop()
                        && occurrence == passage_range.maneuver_occurrence_index(),
                    "event_causality",
                    || {
                        "reservation event semantic anchor differs from its admission occurrence"
                            .into()
                    },
                )?;
            }
            _ => {}
        }
        let expected = match event.kind() {
            TrafficTransitionKind::ConflictEntered { passage } => {
                self.passage_points(world, state, passage)?.0
            }
            TrafficTransitionKind::ConflictCleared { passage } => {
                self.passage_points(world, state, passage)?.1
            }
            TrafficTransitionKind::ReservationReleased { passage_range } => {
                let mut target = (0, 0, 0);
                for index in passage_range.first_conflict_occurrence_index()
                    ..passage_range.first_conflict_occurrence_index()
                        + passage_range.passage_count()
                {
                    let locator = world
                        .conflict_passage_occurrence_locator(state.route(), index)
                        .ok_or_else(|| ("event_causality", "released passage missing".into()))?;
                    target = target.max(self.passage_points(world, state, locator)?.1);
                }
                target
            }
            TrafficTransitionKind::ManeuverTraversalCompleted { .. } => {
                let item = &self.maneuvers[occurrence as usize];
                ensure(anchor.hop() == item.exit - 1, "event_causality", || {
                    "completion semantic hop is not its path exit".into()
                })?;
                item.completion
            }
            _ => {
                ensure(
                    world.route_gate(state.route(), anchor.hop()).is_some(),
                    "event_causality",
                    || "gate event has no actual route gate".into(),
                )?;
                (anchor.hop() + 1, 0, 0)
            }
        };
        let actual = (
            anchor.position().route_edge_index(),
            anchor.position().progress_mm(),
            anchor.position().carry_um(),
        );
        ensure(
            anchor.maneuver_occurrence_index() == occurrence && actual == expected,
            "event_causality",
            || {
                format!(
                    "event anchor differs from canonical static geometry: {event:?}; expected occurrence={occurrence}, position={expected:?}"
                )
            },
        )
    }
}

pub(super) fn waiting_release_hop(
    world: &TrafficWorld,
    zone: WaitingZoneOrdinal,
    entry_hop: u32,
) -> Check<u32> {
    let relations = world.traffic().relations();
    let waiting = relations
        .waiting_zone(zone)
        .ok_or_else(|| ("event_causality", "waiting zone missing".into()))?;
    let entry = relations.maneuver_gate(waiting.entry_gate()).unwrap();
    let release = relations.maneuver_gate(waiting.release_gate()).unwrap();
    entry_hop
        .checked_sub(entry.transition_index())
        .and_then(|start| start.checked_add(release.transition_index()))
        .ok_or_else(|| {
            (
                "event_causality",
                "waiting release occurrence invalid".into(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_crossings_distinguish_the_upstream_edge_end_from_the_crossed_side() {
        assert!(crossed((2, 10_000, 0), (3, 0, 0), (3, 0, 0)));
        assert!(!crossed((2, 9_999, 999), (2, 10_000, 0), (3, 0, 0)));
        assert!(!crossed((3, 0, 0), (3, 1, 0), (3, 0, 0)));
        let geometry = RouteGeometry {
            lengths: vec![10_000, 3_000, 10_000],
            maneuvers: Vec::new(),
        };
        assert_eq!(geometry.advance((0, 9_000, 0), 4_000).unwrap(), (2, 0, 0));
        assert_eq!(
            geometry.advance((2, 5_500, 0), 4_500).unwrap(),
            (2, 10_000, 0)
        );
    }
}
