//! P6 资源收尾工作集：只排除可证明没有控制、边界或 owner 义务的同边运动。

use super::phase::StepWorkspace;
use super::tables::CompiledRoute;
use super::vehicle_store::MotionPosition;
#[cfg(test)]
use crate::VehicleStatus;

/// P5 只标记必须保留的工作，不拥有资源、不提前返回领域错误。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FinalizeHints(u8);

impl FinalizeHints {
    pub(crate) fn frozen(obligation: bool) -> Self {
        Self(u8::from(obligation))
    }

    pub(crate) fn retain(self) -> bool {
        self.0 != 0
    }

    pub(crate) fn with_motion(
        mut self,
        compiled: Option<&CompiledRoute>,
        lengths: &[u32],
        previous: MotionPosition,
        next: MotionPosition,
        completed: bool,
    ) -> Self {
        if completed || previous.route_edge_index != next.route_edge_index {
            self.0 |= 2;
            return self;
        }
        let Some(compiled) = compiled else {
            self.0 |= 16;
            return self;
        };
        let Some(length) = compiled
            .edges
            .get(next.route_edge_index as usize)
            .and_then(|edge| lengths.get(edge.index()))
        else {
            self.0 |= 16;
            return self;
        };
        if next.progress_mm >= *length
            || (previous.progress_mm == 0
                && previous.carry_um == 0
                && previous.route_edge_index > 0
                && compiled
                    .hop_gate
                    .get(previous.route_edge_index as usize - 1)
                    .is_none_or(Option::is_some))
        {
            self.0 |= 4;
        }
        match compiled.waiting_maneuver_at_hop(next.route_edge_index) {
            Some(true) => self.0 |= 8,
            None => self.0 |= 16,
            Some(false) => {}
        }
        self
    }
}

#[cfg(test)]
thread_local! {
    static FULL_SCAN_ORACLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    pub(crate) static LAST_RESOURCE_ROW_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn with_full_scan_oracle<T>(run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            FULL_SCAN_ORACLE.set(self.0);
        }
    }
    let _reset = Reset(FULL_SCAN_ORACLE.replace(true));
    run()
}

impl StepWorkspace<'_> {
    pub(crate) fn complete_resource_rows(
        &self,
        updates: &mut super::motion_updates::MotionUpdates,
    ) {
        updates.supplement_resource_rows();
        #[cfg(test)]
        {
            // Oracle 必须在 Waiting 暂存完成之后取值；独立旧谓词不读 P5 hints 或静态位。
            assert!(
                updates
                    .resource_rows()
                    .windows(2)
                    .all(|pair| pair[0] < pair[1])
            );
            assert!(
                updates
                    .resource_rows()
                    .last()
                    .is_none_or(|&index| index < updates.len())
            );
            for index in 0..updates.len() {
                assert_eq!(
                    updates.resource_rows().binary_search(&index).is_ok(),
                    self.reference_resource_row(updates.row(index, &self.committed.vehicles)),
                    "post-Waiting resource selection row={index}"
                );
            }
            if FULL_SCAN_ORACLE.get() {
                updates.select_resource_rows(&self.committed.vehicles, |_| true);
            }
            LAST_RESOURCE_ROW_COUNT.set(updates.resource_rows().len());
        }
    }

    #[cfg(test)]
    pub(crate) fn select_resource_rows(&self, updates: &mut super::motion_updates::MotionUpdates) {
        #[cfg(test)]
        if FULL_SCAN_ORACLE.get() {
            updates.select_resource_rows(&self.committed.vehicles, |_| true);
            LAST_RESOURCE_ROW_COUNT.set(updates.resource_rows().len());
            return;
        }
        updates.select_resource_rows(&self.committed.vehicles, |row| {
            self.reference_resource_row(row)
        });
        LAST_RESOURCE_ROW_COUNT.set(updates.resource_rows().len());
    }

    #[cfg(test)]
    fn reference_resource_row(&self, row: super::motion_updates::UpdateView<'_>) -> bool {
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
        let previous = row.source.position();
        let next = row.position();
        let old_control = row.source.control();
        let next_control = row.control();
        let handle = row.source.handle();
        let slot = handle.index() as usize;
        if previous.route_edge_index != next.route_edge_index
            || row.status() != VehicleStatus::Active
            || old_control.waiting.is_some()
            || old_control.maneuver.is_some()
            || next_control.waiting.is_some()
            || next_control.maneuver.is_some()
            || self.workspace.waiting_plan_by_vehicle[slot].is_some()
            || self.workspace.conflict_motion_by_vehicle[slot].is_some()
            || self.workspace.conflict_next_eligibility[slot].is_some()
            || self.conflict_reservation(handle).is_some()
        {
            return true;
        }
        let Some(compiled) = self.compiled_route(row.source.route()) else {
            // 缺失路线仍交给原阶段的完整验证，筛选不产生新首错。
            return true;
        };
        let Some(length) = compiled
            .edges
            .get(next.route_edge_index as usize)
            .and_then(|edge| lengths.get(edge.index()))
        else {
            return true;
        };
        // 同 cursor 的边尾决定、零进度的旧 Gate 回看都不能漏掉。
        if next.progress_mm >= *length
            || (previous.progress_mm == 0
                && previous.carry_um == 0
                && previous.route_edge_index > 0
                && compiled.hop_gate[previous.route_edge_index as usize - 1].is_some())
        {
            return true;
        }
        // 不依赖旧 traversal 已规范化：夹具或受控恢复仍可能在此拍首次推导它。
        waiting_maneuver_at_hop(compiled, next.route_edge_index)
    }
}

#[cfg(test)]
fn waiting_maneuver_at_hop(compiled: &CompiledRoute, hop: u32) -> bool {
    if compiled.waiting.is_empty() {
        return false;
    }
    let index = compiled
        .maneuvers
        .partition_point(|maneuver| maneuver.exit_route_edge_index <= hop);
    compiled.maneuvers.get(index).is_some_and(|maneuver| {
        maneuver.entry_route_edge_index <= hop
            && compiled
                .waiting
                .binary_search_by_key(&(index as u32), |occurrence| occurrence.maneuver_index)
                .is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::motion_updates::MotionUpdates;
    use crate::kernel::waiting::tests::multi_gate_world;

    #[test]
    fn motion_hints_match_full_selector_at_every_occurrence_and_boundary() {
        let mut world = multi_gate_world(1);
        let handle = world.live_vehicles()[0];
        let initial = world.vehicle(handle).unwrap();
        let compiled = world.state.compiled_route(initial.route).unwrap().clone();
        let lengths = world.traffic().lane_lengths_millimetres().to_vec();
        for cursor in 0..compiled.edges.len() as u32 {
            let length = lengths[compiled.edges[cursor as usize].index()];
            for old_progress in [0, 1] {
                for carry in [0, 999] {
                    for progress in [1, length.saturating_sub(1), length] {
                        for completed in [false, true] {
                            let mut old = initial;
                            old.route_edge_index = cursor;
                            old.progress_mm = old_progress;
                            old.carry_um = carry;
                            old.maneuver_traversal = None;
                            old.waiting_membership = None;
                            world
                                .state
                                .committed
                                .vehicles
                                .slot_mut(handle.index() as usize)
                                .state = Some(old);
                            let mut next = old;
                            next.progress_mm = progress;
                            if completed {
                                next.status = crate::VehicleStatus::Completed;
                            }
                            let updates = MotionUpdates::from_states(
                                &[(handle.index() as usize, next)],
                                &world.state.committed.vehicles,
                            );
                            let workspace = world.state.step_workspace();
                            let expected = workspace.reference_resource_row(
                                updates.row(0, &workspace.committed.vehicles),
                            );
                            let actual = FinalizeHints::default().with_motion(
                                Some(&compiled),
                                &lengths,
                                (&old).into(),
                                (&next).into(),
                                completed,
                            );
                            assert_eq!(
                                actual.retain(),
                                expected,
                                "cursor={cursor} old={old_progress} carry={carry} progress={progress} completed={completed}"
                            );
                        }
                    }
                }
            }
        }
        let mut next = initial;
        next.route_edge_index += 1;
        assert!(
            FinalizeHints::default()
                .with_motion(
                    Some(&compiled),
                    &lengths,
                    (&initial).into(),
                    (&next).into(),
                    false
                )
                .retain()
        );
    }

    #[test]
    fn sparse_post_waiting_control_supplement_is_unique_ordered_and_reuses_capacity() {
        let mut world = multi_gate_world(3);
        let handles = world.live_vehicles().to_vec();
        let states: Vec<_> = handles
            .iter()
            .map(|&handle| {
                let mut state = world.vehicle(handle).unwrap();
                state.route_edge_index =
                    world.state.compiled_route(state.route).unwrap().edges.len() as u32 - 1;
                state.progress_mm = 0;
                state.carry_um = 1;
                state.maneuver_traversal = None;
                state.waiting_membership = None;
                world
                    .state
                    .committed
                    .vehicles
                    .slot_mut(handle.index() as usize)
                    .state = Some(state);
                (handle.index() as usize, state)
            })
            .collect();
        let mut updates = MotionUpdates::from_states(&states, &world.state.committed.vehicles);
        updates.select_resource_rows(&world.state.committed.vehicles, |_| false);
        let reserved = updates.retained_logical_bytes();
        let member = crate::WaitingMembership {
            waiting_zone: laneflow_static_contract::WaitingZoneOrdinal::from_raw(0),
            admission_sequence: 0,
            release_hop: 1,
        };
        for index in [2, 0, 2] {
            updates
                .set_control(
                    index,
                    crate::VehicleStatus::Active,
                    None,
                    Some(member),
                    &world.state.committed.vehicles,
                )
                .unwrap();
        }
        updates.supplement_resource_rows();
        assert_eq!(updates.resource_rows(), [0, 2]);
        let reserved_with_controls = updates.retained_logical_bytes();
        assert!(reserved_with_controls >= reserved);
        world
            .state
            .step_workspace()
            .complete_resource_rows(&mut updates);
        assert_eq!(updates.resource_rows(), [0, 2]);
        assert_eq!(updates.retained_logical_bytes(), reserved_with_controls);
    }

    #[test]
    fn missing_geometry_is_conservatively_retained_without_returning_a_domain_error() {
        let world = multi_gate_world(1);
        let state = world.vehicle(world.live_vehicles()[0]).unwrap();
        let compiled = world.state.compiled_route(state.route).unwrap();
        let position = (&state).into();
        assert!(
            FinalizeHints::default()
                .with_motion(None, &[], position, position, false)
                .retain()
        );
        assert!(
            FinalizeHints::default()
                .with_motion(Some(compiled), &[], position, position, false)
                .retain()
        );
        let mut incomplete = compiled.clone();
        incomplete.waiting_maneuver_bits.clear();
        assert!(!incomplete.waiting.is_empty());
        assert!(
            FinalizeHints::default()
                .with_motion(
                    Some(&incomplete),
                    world.traffic().lane_lengths_millimetres(),
                    position,
                    position,
                    false
                )
                .retain()
        );
    }

    #[test]
    fn same_cursor_grant_lookback_matches_full_events_and_rejects_cursor_only_selection() {
        let mut world = multi_gate_world(1);
        let vehicle = world.live_vehicles()[0];
        world.state.prepare_waiting_step(0.1).unwrap();
        world.state.prepare_conflict_step(0.1, 1, None).unwrap();
        let gate_hop = world.state.workspace.conflict_grants[0].gate_hop;
        let slot = vehicle.index() as usize;
        let mut old = world.vehicle(vehicle).unwrap();
        old.route_edge_index = gate_hop + 1;
        old.progress_mm = 0;
        world.state.committed.vehicles.slot_mut(slot).state = Some(old);
        world.state.workspace.next_state_by_vehicle[slot] = 1;
        let mut updates =
            MotionUpdates::from_states(&[(slot, old)], &world.state.committed.vehicles);
        let visit = |updates: &MotionUpdates| {
            let mut events = Vec::new();
            world
                .state
                .step_workspace()
                .visit_transition_events(updates, 1, |event| events.push(event))
                .unwrap();
            events
        };
        let mut visit = visit;
        let complete = visit(&updates);
        assert!(complete.iter().any(|event| {
            matches!(event.kind, crate::TrafficTransitionKind::GateCrossed { .. })
                && event.anchor.hop() == gate_hop
        }));
        world
            .state
            .step_workspace()
            .select_resource_rows(&mut updates);
        assert_eq!(updates.resource_rows(), [0]);
        let mut selected = Vec::new();
        world
            .state
            .step_workspace()
            .visit_transition_events(&updates, 1, |event| selected.push(event))
            .unwrap();
        assert_eq!(selected, complete);
        updates.select_resource_rows(&world.state.committed.vehicles, |row| {
            row.source.position().route_edge_index != row.position().route_edge_index
        });
        let mut incomplete = Vec::new();
        world
            .state
            .step_workspace()
            .visit_transition_events(&updates, 1, |event| incomplete.push(event))
            .unwrap();
        assert!(incomplete.is_empty());
        assert_ne!(
            incomplete, complete,
            "cursor-only selection must fail the event oracle"
        );
    }
}
