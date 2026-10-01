use crate::{Result, need, simd_export};
use std::path::Path;

fn replace(text: &mut String, anchor: &str, value: &str) -> Result<()> {
    need(
        text.matches(anchor).count() == 1,
        &format!("hot input anchor: {anchor}"),
    )?;
    *text = text.replacen(anchor, value, 1);
    Ok(())
}

pub(crate) fn patch_tick(text: &mut String, arm: &str) -> Result<()> {
    simd_export::patch_tick(text, arm)?;
    replace(text, "    iidm: IidmInput,\n", "")?;
    replace(
        text,
        "        let prepared = self.prepare_active_motion(state, delta_s, waiting_stop, conflict_stop,\n            parking_binding, horizon, leader_gap_override)?;\n        self.finish_active_motion(prepared, motion_bounds, leader_constraint_only,\n            prepared.iidm.scalar())",
        "        let mut iidm = IidmInput::EMPTY;\n        let prepared = self.prepare_active_motion(state, delta_s, waiting_stop, conflict_stop,\n            parking_binding, horizon, leader_gap_override, &mut iidm)?;\n        self.finish_active_motion(prepared, motion_bounds, leader_constraint_only, iidm.scalar())",
    )?;
    replace(
        text,
        "        leader_gap_override: Option<Option<i64>>,\n    ) -> Option<PreparedActiveMotion<'a>>",
        "        leader_gap_override: Option<Option<i64>>, iidm_out: &mut IidmInput,\n    ) -> Option<PreparedActiveMotion<'a>>",
    )?;
    replace(
        text,
        "        let iidm = IidmInput { speed:",
        "        *iidm_out = IidmInput { speed:",
    )?;
    replace(
        text,
        "            delta_s, desired_mm_s, iidm })",
        "            delta_s, desired_mm_s })",
    )?;
    replace(
        text,
        "    fn prepare_vehicle_motion(self, state: &VehicleState, active_index: usize, delta_s: f32)\n        -> Result<PreparedVehicleMotion<'a>, StepError>",
        "    fn prepare_vehicle_motion(self, state: &VehicleState, active_index: usize, delta_s: f32,\n        batch: &mut Option<IidmBatch>, lane: usize)\n        -> Result<PreparedVehicleMotion<'a>, StepError>",
    )?;
    replace(
        text,
        "            PreparedMotionState::Compute(self.read.prepare_active_motion(*state, delta_s,\n                waiting_stop, conflict_stop, parking_binding,\n                cached.and_then(|entry| entry.horizon), None).ok_or(StepError::NonFiniteMotion)?)",
        "            let mut input = IidmInput::EMPTY;\n            let prepared = self.read.prepare_active_motion(*state, delta_s,\n                waiting_stop, conflict_stop, parking_binding,\n                cached.and_then(|entry| entry.horizon), None, &mut input)\n                .ok_or(StepError::NonFiniteMotion)?;\n            batch.get_or_insert_with(|| IidmBatch::EMPTY).write(lane, input);\n            PreparedMotionState::Compute(prepared)",
    )?;
    let begin = text
        .find("#[derive(Clone, Copy)]\nstruct IidmBatch")
        .ok_or("hot kernel start")?;
    let end = text[begin..]
        .find("#[cfg(test)]\nmod iidm_simd_tests")
        .ok_or("hot kernel end")?
        + begin;
    text.replace_range(begin..end, include_str!("../hot_kernel.rs"));
    replace(
        text,
        "std::hint::black_box(packed[i]).scalar()",
        "std::hint::black_box(&packed[i]).scalar()",
    )?;
    replace(
        text,
        "std::hint::black_box(packed[i]).simd()",
        "std::hint::black_box(&packed[i]).simd()",
    )?;
    let begin = text
        .find("    fn compute_motion_batch(")
        .ok_or("hot motion start")?;
    let end = text[begin..]
        .find("\n}\n\nimpl<'a> MotionTaskView<'a>")
        .ok_or("hot motion end")?
        + begin;
    let method = include_str!("../hot_motion.rs").replace(
        "BATCH_SOLVE",
        if arm == "layout" { "scalar" } else { "simd" },
    );
    text.replace_range(begin..end, &method);
    Ok(())
}

pub(crate) fn export(root: &Path, arm: &str) -> Result<()> {
    simd_export::export_patched(root, arm, patch_tick)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unknown_arm_and_missing_anchors() {
        assert!(patch_tick(&mut String::new(), "unknown").is_err());
        assert!(patch_tick(&mut String::new(), "layout").is_err());
    }

    #[test]
    fn patches_current_runtime_and_keeps_canonical_consumer() {
        let original = include_str!("../../../crates/laneflow-runtime/src/kernel/tick.rs");
        for arm in ["layout", "candidate"] {
            let mut text = original.replace("\r\n", "\n");
            patch_tick(&mut text, arm).unwrap();
            assert!(!text.contains("BATCH_SOLVE"));
            assert!(!text.contains("prepared.iidm"));
            assert!(text.contains(
                "execution.try_for_each_chunk(view.read, slots, &first_error, chunk_size, compute)"
            ));
            assert!(text.contains(
                "push_parking_arrival(parking_arrivals, arrival, view.read.binding.world_id)?"
            ));
        }
    }
}
