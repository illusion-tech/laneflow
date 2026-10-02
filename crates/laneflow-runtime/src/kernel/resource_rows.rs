//! P6 资源收尾工作集：只排除可证明没有控制、边界或 owner 义务的同边运动。

use super::phase::StepWorkspace;
use super::tables::CompiledRoute;
use crate::VehicleStatus;

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
    pub(crate) fn select_resource_rows(&self, updates: &mut super::motion_updates::MotionUpdates) {
        #[cfg(test)]
        if FULL_SCAN_ORACLE.get() {
            updates.select_resource_rows(&self.committed.vehicles, |_| true);
            LAST_RESOURCE_ROW_COUNT.set(updates.resource_rows().len());
            return;
        }
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
        updates.select_resource_rows(&self.committed.vehicles, |row| {
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
        });
        #[cfg(test)]
        LAST_RESOURCE_ROW_COUNT.set(updates.resource_rows().len());
    }
}

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
    use crate::kernel::motion_updates::MotionUpdates;
    use crate::kernel::waiting::tests::multi_gate_world;

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
