//! #779：逐项扫描预言机覆盖投影区间；完整合法资源生命周期另由 phase_equivalence 验证。
use super::*;
use crate::kernel::waiting::tests::multi_gate_world;
use crate::{ConflictDecisionOutcome, ManeuverTraversalState};

#[test]
fn empty_ranges_match_linear_projection_at_every_edge_boundary() {
    let mut world = multi_gate_world(1);
    let vehicle = world.live_vehicles()[0];
    let initial = world.vehicle(vehicle).unwrap();
    let slot = vehicle.index() as usize;
    let edges = world.route_edges(initial.route).unwrap().to_vec();
    world.state.workspace.next_state_by_vehicle[slot] = 1;
    for old_hop in 0..edges.len() as u32 {
        for next_hop in old_hop..edges.len() as u32 {
            let length =
                world.traffic().lane_lengths_millimetres()[edges[next_hop as usize].index()];
            for (old_progress, old_carry) in [(0, 0), (0, 1), (1, 0)] {
                for next_progress in [0, 1, length - 1, length] {
                    for clearing in [false, true] {
                        let mut old = initial;
                        old.route_edge_index = old_hop;
                        old.progress_mm = old_progress;
                        old.carry_um = old_carry;
                        old.maneuver_traversal = clearing.then_some(ManeuverTraversalState {
                            route: old.route,
                            maneuver_occurrence_index: 0,
                            phase: ManeuverTraversalPhase::Clearing {
                                admission_gate_hop: 0,
                            },
                        });
                        world.state.committed.vehicles[slot].state = Some(old);
                        let mut next = old;
                        next.route_edge_index = next_hop;
                        next.progress_mm = next_progress;
                        next.maneuver_traversal = None;
                        let compiled = world.state.compiled_route(old.route).unwrap();
                        let expected_gates: Vec<_> = compiled
                            .gate_hops
                            .iter()
                            .copied()
                            .filter(|hop| *hop >= old_hop && *hop < next_hop)
                            .collect();
                        let expected_completed: Vec<_> = compiled
                            .maneuvers
                            .iter()
                            .enumerate()
                            .filter(|(index, item)| {
                                (item.exit_route_edge_index > old_hop
                                    && item.exit_route_edge_index <= next_hop)
                                    || (clearing
                                        && *index == 0
                                        && item.exit_route_edge_index <= old_hop)
                            })
                            .map(|(index, _)| index as u32)
                            .collect();
                        let first_hop = if old_progress == 0 && old_carry == 0 {
                            old_hop.saturating_sub(1)
                        } else {
                            old_hop
                        };
                        let expected_decisions: Vec<_> = compiled
                            .gate_hops
                            .iter()
                            .copied()
                            .filter(|hop| *hop >= first_hop && *hop <= next_hop)
                            .filter(|hop| *hop < next_hop || next_progress >= length)
                            .filter(|hop| {
                                compiled.conflict_gate_ranges[*hop as usize].len == 0
                                    && !compiled.waiting.iter().any(|entry| entry.entry_hop == *hop)
                            })
                            .collect();
                        let mut actual = Vec::new();
                        world
                            .state
                            .step_workspace()
                            .visit_transition_events(&[(slot, next)], 1, |event| actual.push(event))
                            .unwrap();
                        assert_eq!(
                            actual
                                .iter()
                                .filter_map(|event| matches!(
                                    event.kind,
                                    TrafficTransitionKind::GateCrossed { .. }
                                )
                                .then_some(event.anchor.hop))
                                .collect::<Vec<_>>(),
                            expected_gates
                        );
                        assert_eq!(
                            actual
                                .iter()
                                .filter_map(|event| match event.kind {
                                    TrafficTransitionKind::ManeuverTraversalCompleted {
                                        maneuver_occurrence_index,
                                    } => Some(maneuver_occurrence_index),
                                    _ => None,
                                })
                                .collect::<Vec<_>>(),
                            expected_completed
                        );
                        world.state.workspace.conflict_staged_decisions.clear();
                        world
                            .state
                            .step_workspace()
                            .stage_resource_free_gate_decisions(&next, 0)
                            .unwrap();
                        let decisions = &world.state.workspace.conflict_staged_decisions;
                        assert_eq!(
                            decisions
                                .iter()
                                .map(|decision| decision.anchor().hop())
                                .collect::<Vec<_>>(),
                            expected_decisions
                        );
                        assert!(
                            decisions.iter().all(|decision| decision.outcome()
                                == ConflictDecisionOutcome::NotRequired)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn same_cursor_prepared_grant_still_emits_crossed_gate() {
    let mut world = multi_gate_world(1);
    let vehicle = world.live_vehicles()[0];
    let slot = vehicle.index() as usize;
    world.state.prepare_waiting_step(0.1).unwrap();
    world.state.prepare_conflict_step(0.1, 1, None).unwrap();
    let gate_hop = world.state.workspace.conflict_grants[0].gate_hop;
    let mut old = world.vehicle(vehicle).unwrap();
    old.route_edge_index = gate_hop + 1;
    old.progress_mm = 0;
    world.state.committed.vehicles[slot].state = Some(old);
    world.state.workspace.next_state_by_vehicle[slot] = 1;
    let mut events = Vec::new();
    world
        .state
        .step_workspace()
        .visit_transition_events(&[(slot, old)], 1, |event| events.push(event))
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter_map(
                |event| matches!(event.kind, TrafficTransitionKind::GateCrossed { .. })
                    .then_some(event.anchor.hop)
            )
            .collect::<Vec<_>>(),
        [gate_hop]
    );
}

#[test]
fn empty_resource_free_range_still_rejects_missing_route() {
    let mut world = multi_gate_world(1);
    let next = world.vehicle(world.live_vehicles()[0]).unwrap();
    world.state.committed.routes[next.route.index() as usize].compiled = None;
    assert_eq!(
        world
            .state
            .step_workspace()
            .stage_resource_free_gate_decisions(&next, 0),
        Err(StepError::ConflictInvariantViolation)
    );
}
