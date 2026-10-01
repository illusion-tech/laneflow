//! #814：诊断树中的阶段成本与工作密度；耗时之和不是阶段墙钟或线程 CPU。
use crate::{Result, columnar_export::patch};
use std::{fs, path::Path};

const MOTION: &str = "crates/laneflow-runtime/src/kernel/tick/columnar_motion.rs";
const TICK: &str = "crates/laneflow-runtime/src/kernel/tick.rs";
const WAITING: &str = "crates/laneflow-runtime/src/kernel/waiting.rs";

pub(crate) fn instrument(root: &Path, candidate: bool) -> Result<()> {
    patch(
        root,
        "crates/laneflow-runtime/src/kernel/execution.rs",
        "    compute(view, start, chunk);",
        "    let _pipeline_flush = super::tick::PipelineFlush;\n    compute(view, start, chunk);",
    )?;
    if candidate {
        instrument_motion(root)?;
        instrument_preview(root)?;
        instrument_finalize(root)?;
        instrument_commit(root)?;
    }
    let lib = root.join("crates/laneflow-runtime/src/lib.rs");
    let mut text = fs::read_to_string(&lib)?;
    text.push_str("\n#[doc(hidden)] pub fn research_pipeline() -> ([u64; 28], [u64; 28], [u64; 40]) { kernel::tick::take_pipeline() }\n");
    fs::write(lib, text)?;
    patch(
        root,
        "tools/laneflow-urban-harness/src/host.rs",
        "                laneflow_runtime::research_work();",
        "                laneflow_runtime::research_work();\n                laneflow_runtime::research_pipeline();",
    )?;
    patch(
        root,
        "tools/laneflow-urban-harness/src/host.rs",
        "                let (stages, calls) = laneflow_runtime::research_take();",
        "                let (stages, calls) = laneflow_runtime::research_take();\n                let (pipeline_elapsed, measured_calls, counts) = laneflow_runtime::research_pipeline();\n                eprintln!(\"LF814_PIPELINE {{\\\"tick\\\":{},\\\"elapsed_sum_ns\\\":{:?},\\\"measured_calls\\\":{:?},\\\"counts\\\":{:?}}}\", world.tick_index(), pipeline_elapsed, measured_calls, counts);\n                eprintln!(\"LF814_BLOCK {{\\\"tick\\\":{},\\\"elapsed_sum_ns\\\":{:?}}}\", world.tick_index(), [pipeline_elapsed[6], pipeline_elapsed[11..15].iter().sum::<u64>(), pipeline_elapsed[15]+pipeline_elapsed[16], pipeline_elapsed[17], pipeline_elapsed[18]]);",
    )
}

fn instrument_motion(root: &Path) -> Result<()> {
    for (old, new) in [
        (
            "    let mut batch = Batch::new();\n    let n = chunk.cursor.len();",
            "    let _pipeline_flush = super::PipelineFlush;\n    let _sample = super::PipelineSample::begin(start % 4_096 == 0);\n    let _prepare_timer = super::PipelineTimer::begin(6);\n    let mut batch = Batch::new();\n    let n = chunk.cursor.len();",
        ),
        (
            "        let Some(state) = view.read.committed.vehicles.active_at(start + row) else {",
            "        super::note_pipeline(8, 1);\n        let _row_timer = super::PipelineTimer::sampled(7);\n        let Some(state) = view.read.committed.vehicles.active_at(start + row) else {",
        ),
        (
            "        let prepared = (|| {",
            "        drop(_row_timer);\n        super::note_pipeline(9, 1);\n        let prepared = (|| {\n            let _constraint_timer = super::PipelineTimer::sampled(8);",
        ),
        (
            "            chunk.reports[row].checkpoint = MotionCheckpoint::Calculation;",
            "            drop(_constraint_timer);\n            chunk.reports[row].checkpoint = MotionCheckpoint::Calculation;",
        ),
        (
            "                let next = next.apply(state);",
            "                super::note_pipeline(10, 1);\n                let next = next.apply(state);",
        ),
        (
            "            let reused_basis = cached",
            "            let _basis_timer = super::PipelineTimer::sampled(9);\n            let reused_basis = cached",
        ),
        (
            "            let basis = reused_basis",
            "            super::note_pipeline(if reused_basis.is_some() { 11 } else { 12 }, 1);\n            let basis = reused_basis",
        ),
        (
            "            *route = Some(compiled);",
            "            drop(_basis_timer);\n            let _batch_timer = super::PipelineTimer::sampled(10);\n            *route = Some(compiled);",
        ),
        (
            "    if !batch.enabled[..n].iter().any(|&enabled| enabled) {",
            "    drop(_prepare_timer);\n    if !batch.enabled[..n].iter().any(|&enabled| enabled) {",
        ),
        (
            "    let n = range.len();\n    let offset = offset + range.start;",
            "    let _numeric_timer = super::PipelineTimer::begin(match phase { NumericPhase::Fused => 11, NumericPhase::Proposal => 12, NumericPhase::Project => 13, NumericPhase::Quantize => 14 });\n    let n = range.len();\n    let offset = offset + range.start;",
        ),
        (
            "    for row in range.clone() {\n        batch.drop_index[row]",
            "    let _limit_timer = super::PipelineTimer::begin(if boundary { 16 } else { 15 });\n    for row in range.clone() {\n        batch.drop_index[row]",
        ),
        (
            "        if complex {\n            numerical(",
            "        super::note_pipeline(if complex { 14 } else { 13 }, 1);\n        super::note_pipeline(if complex { 16 } else { 15 }, range.len());\n        super::note_pipeline(if complex { 19 } else { 18 }, batch.enabled[range.clone()].iter().filter(|&&v| v).count());\n        super::note_pipeline(17, range.clone().filter(|&row| batch.enabled[row] && batch.complex[row]).count());\n        if complex {\n            numerical(",
        ),
        (
            "    // 工作掩码驱动真实多跳行走；",
            "    let _walk_timer = super::PipelineTimer::begin(17);\n    // 工作掩码驱动真实多跳行走；",
        ),
        (
            "    while batch.walk_active[..n].iter().any(|&active| active) {",
            "    while batch.walk_active[..n].iter().any(|&active| active) {\n        super::note_pipeline(20, 1);\n        super::note_pipeline(22, batch.walk_active[..n].iter().filter(|&&v| v).count());",
        ),
        (
            "            let state = view\n                .read\n                .committed\n                .vehicles\n                .active_at(start + row)",
            "            super::note_pipeline(21, 1);\n            let state = view\n                .read\n                .committed\n                .vehicles\n                .active_at(start + row)",
        ),
        (
            "    for (row, route) in routes.iter().enumerate().take(n) {\n        if !batch.enabled[row] || chunk.reports[row].error.is_some() {",
            "    drop(_walk_timer);\n    let _finish_timer = super::PipelineTimer::begin(18);\n    for (row, route) in routes.iter().enumerate().take(n) {\n        if !batch.enabled[row] || chunk.reports[row].error.is_some() {",
        ),
    ] {
        patch(root, MOTION, old, new)?;
    }
    Ok(())
}

fn instrument_preview(root: &Path) -> Result<()> {
    for (file, old, new) in [
        (
            TICK,
            "        debug_assert_eq!(\n            self.committed.live_order.get(update_sequence),",
            "        let _sample = PipelineSample::begin(update_sequence % 32 == 0);\n        let _entry_timer = PipelineTimer::sampled(4);\n        note_pipeline(3, 1);\n        debug_assert_eq!(\n            self.committed.live_order.get(update_sequence),",
        ),
        (
            TICK,
            "        let preview = self\n            .preview_active_vehicle_with_waiting_stop(row.state(), delta_s, None, Some(horizon))",
            "        drop(_entry_timer);\n        let _preview_timer = PipelineTimer::sampled(5);\n        note_pipeline(4, 1);\n        let preview = self\n            .preview_active_vehicle_with_waiting_stop(row.state(), delta_s, None, Some(horizon))",
        ),
        (
            WAITING,
            "    if cache_index < cache_limit {\n        motion_cache.push(",
            "    let _sample = super::tick::PipelineSample::begin(cache_index % 32 == 0);\n    let _cache_timer = super::tick::PipelineTimer::sampled(3);\n    if cache_index < cache_limit {\n        super::tick::note_pipeline(5, 1);\n        super::tick::note_pipeline(6, usize::from(entry.horizon.is_some()));\n        super::tick::note_pipeline(7, usize::from(entry.preview.is_some()));\n        motion_cache.push(",
        ),
        (
            WAITING,
            "    let inputs = &mut workspace.waiting_preview_inputs;",
            "    let _discover_timer = super::tick::PipelineTimer::begin(0);\n    let inputs = &mut workspace.waiting_preview_inputs;",
        ),
        (
            WAITING,
            "    let workload = inputs.len();",
            "    drop(_discover_timer);\n    let workload = inputs.len();\n    super::tick::note_pipeline(0, view.committed.live_order.len());\n    super::tick::note_pipeline(1, workload);",
        ),
        (
            WAITING,
            "    let slots = &mut workspace.waiting_preview_slots;",
            "    let _slot_timer = super::tick::PipelineTimer::begin(1);\n    let slots = &mut workspace.waiting_preview_slots;",
        ),
        (
            WAITING,
            "    slots.resize(workload, crate::kernel::execution::DispatchSlot::Pending);",
            "    slots.resize(workload, crate::kernel::execution::DispatchSlot::Pending);\n    drop(_slot_timer);\n    super::tick::note_pipeline(2, workload);",
        ),
        (
            WAITING,
            "    for (cache_index, ((vehicle, update_sequence), slot)) in workspace",
            "    let _consume_timer = super::tick::PipelineTimer::begin(2);\n    for (cache_index, ((vehicle, update_sequence), slot)) in workspace",
        ),
    ] {
        patch(root, file, old, new)?;
    }
    Ok(())
}

fn instrument_finalize(root: &Path) -> Result<()> {
    for (file, old, new) in [
        (
            "crates/laneflow-runtime/src/kernel/vehicle_store.rs",
            "    ) -> Option<ActiveRow<'a>> {\n        if !matches!(entry.location, Location::Active(row) if row == physical) {",
            "    ) -> Option<ActiveRow<'a>> {\n        super::tick::note_pipeline(24, 1);\n        if !matches!(entry.location, Location::Active(row) if row == physical) {",
        ),
        (
            "crates/laneflow-runtime/src/kernel/vehicle_store.rs",
            "    pub(crate) fn active_at(&self, physical: usize) -> Option<VehicleState> {",
            "    pub(crate) fn active_at(&self, physical: usize) -> Option<VehicleState> {\n        super::tick::note_pipeline(25, 1);",
        ),
        (
            "crates/laneflow-runtime/src/kernel/motion_updates.rs",
            "        assert!(!self.published, \"next motion has not been published\");\n        let row = self.order[index];\n        let next = &self.motion[row.physical / BLOCK_ROWS];",
            "        super::tick::note_pipeline(23, 1);\n        assert!(!self.published, \"next motion has not been published\");\n        let row = self.order[index];\n        let next = &self.motion[row.physical / BLOCK_ROWS];",
        ),
        (
            "crates/laneflow-runtime/src/kernel/motion_updates.rs",
            "        let old = current\n            .active_control(row.slot)",
            "        super::tick::note_pipeline(28, 1);\n        let old = current\n            .active_control(row.slot)",
        ),
        (
            "crates/laneflow-runtime/src/kernel/motion_updates.rs",
            "            None if changed => {",
            "            None if changed => {\n                super::tick::note_pipeline(29, 1);",
        ),
        (
            "crates/laneflow-runtime/src/kernel/conflict_tick.rs",
            "        for index in 0..updates.len() {\n            let row = updates.row(index, &self.committed.vehicles);",
            "        for index in 0..updates.len() {\n            super::tick::note_pipeline(26, 1);\n            let row = updates.row(index, &self.committed.vehicles);\n            let control_obligation = row.control().waiting.is_some() || row.control().maneuver.is_some();",
        ),
        (
            "crates/laneflow-runtime/src/kernel/conflict_tick.rs",
            "            let all_clear = match range {",
            "            super::tick::note_pipeline(27, usize::from(control_obligation || range.is_some() || grant_index.is_some() || self.workspace.conflict_next_eligibility[handle.index() as usize].is_some()));\n            super::tick::note_pipeline(32, usize::from(fields.previous.route_edge_index != position.route_edge_index));\n            let all_clear = match range {",
        ),
        (
            WAITING,
            "        for update_index in 0..updates.len() {\n            let row = updates.row(update_index, &self.committed.vehicles);",
            "        for update_index in 0..updates.len() {\n            super::tick::note_pipeline(31, 1);\n            let row = updates.row(update_index, &self.committed.vehicles);",
        ),
        (
            WAITING,
            "                anchors.push(NonEntryGateAnchor {",
            "                super::tick::note_pipeline(34, 1);\n                anchors.push(NonEntryGateAnchor {",
        ),
    ] {
        patch(root, file, old, new)?;
    }
    Ok(())
}

fn instrument_commit(root: &Path) -> Result<()> {
    for (file, old, new) in [
        (
            TICK,
            "        self.commit_conflict_transitions(&updates, time_ms);",
            "        let _resource_timer = PipelineTimer::begin(19);\n        self.commit_conflict_transitions(&updates, time_ms);",
        ),
        (
            TICK,
            "        let mut migration_journal = self.journal.take();",
            "        drop(_resource_timer);\n        let _journal_timer = PipelineTimer::begin(20);\n        let mut migration_journal = self.journal.take();",
        ),
        (
            TICK,
            "        *self.journal = migration_journal;\n        updates.publish(&mut self.committed.vehicles);",
            "        *self.journal = migration_journal;\n        drop(_journal_timer);\n        let _publish_timer = PipelineTimer::begin(25);\n        updates.publish(&mut self.committed.vehicles);\n        drop(_publish_timer);\n        let _resources_after = PipelineTimer::begin(23);",
        ),
        (
            TICK,
            "        self.commit_conflict_step();\n        updates.clear();",
            "        self.commit_conflict_step();\n        drop(_resources_after);\n        let _rest_timer = PipelineTimer::begin(24);\n        updates.clear();",
        ),
        (
            "crates/laneflow-runtime/src/kernel/motion_updates.rs",
            "        for control_index in 0..self.control.len() {",
            "        let _control_timer = super::tick::PipelineTimer::begin(21);\n        for control_index in 0..self.control.len() {",
        ),
        (
            "crates/laneflow-runtime/src/kernel/motion_updates.rs",
            "            if state.status == VehicleStatus::Active {\n                current.apply_control(state);",
            "            if state.status == VehicleStatus::Active {\n                super::tick::note_pipeline(35, 1);\n                current.apply_control(state);",
        ),
        (
            "crates/laneflow-runtime/src/kernel/motion_updates.rs",
            "                let generation = current.slot(row.slot).generation;",
            "                super::tick::note_pipeline(30, 1);\n                let generation = current.slot(row.slot).generation;",
        ),
        (
            "crates/laneflow-runtime/src/kernel/motion_updates.rs",
            "        std::mem::swap(&mut current.motion, &mut self.motion);",
            "        drop(_control_timer);\n        let _swap_timer = super::tick::PipelineTimer::begin(22);\n        std::mem::swap(&mut current.motion, &mut self.motion);",
        ),
    ] {
        patch(root, file, old, new)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "columnar_pipeline_probe.rs"]
mod probe;
#[cfg(test)]
mod tests {
    use super::probe;
    #[test]
    fn worker_records_reduce_once_and_do_not_leak_between_ticks() {
        probe::take_columnar_work();
        probe::take_pipeline();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    let _flush = probe::PipelineFlush;
                    let _sample = probe::PipelineSample::begin(true);
                    let _timer = probe::PipelineTimer::sampled(7);
                    for _ in 0..128 {
                        probe::note_columnar_work(17, 1);
                        probe::note_pipeline(24, 1);
                    }
                    let _whole = probe::PipelineTimer::begin(6);
                });
            }
        });
        assert_eq!(probe::take_columnar_work()[17], 512);
        let (nanos, calls, counts) = probe::take_pipeline();
        assert_eq!(counts[24], 512);
        assert_eq!(calls[6], 4);
        assert_eq!(calls[7], 4);
        assert!(nanos[7] > 0);
        assert_eq!(probe::take_columnar_work(), [0; 20]);
        assert_eq!(probe::take_pipeline(), ([0; 28], [0; 28], [0; 40]));
    }
}
