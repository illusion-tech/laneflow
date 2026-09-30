use crate::{BASE, EXPERIMENT, Result, cache_research::prepare, chunk_build, io, need};
use serde_json::json;
use std::{fs, path::Path};

const TICK: &str = "crates/laneflow-runtime/src/kernel/tick.rs";

fn replace(text: &mut String, anchor: &str, value: &str) -> Result<()> {
    need(
        text.matches(anchor).count() == 1,
        &format!("SIMD anchor: {anchor}"),
    )?;
    *text = text.replacen(anchor, value, 1);
    Ok(())
}

pub(crate) fn patch_tick(text: &mut String, arm: &str) -> Result<()> {
    need(["layout", "candidate"].contains(&arm), "SIMD patch arm")?;
    let declaration =
        "    #[allow(clippy::too_many_arguments)]\n    fn calculate_active_vehicle_motion(";
    let begin = text.find(declaration).ok_or("motion declaration")?;
    let end = text[begin..]
        .find("\n    /// 按同一拍初 binding")
        .ok_or("motion end")?
        + begin;
    let original = text[begin..end].to_owned();
    let body = original.find("        #[cfg(test)]").ok_or("motion body")?;
    let solve = original
        .find("        let (mut travel_m, next_speed_m) = si_comfort_travel(")
        .ok_or("motion solve")?;
    let suffix = original[solve..]
        .find("        if travel_m < 0.0")
        .ok_or("motion suffix")?
        + solve;
    let prefix = &original[body..solve];
    let tail = &original[suffix..];
    let wrapper = original[..body].replace("mut state: VehicleState", "state: VehicleState");
    let rewritten = format!(
        r#"{wrapper}
        let prepared = self.prepare_active_motion(state, delta_s, waiting_stop, conflict_stop,
            parking_binding, horizon, leader_gap_override)?;
        self.finish_active_motion(prepared, motion_bounds, leader_constraint_only,
            prepared.iidm.scalar())
    }}

    #[allow(clippy::too_many_arguments)]
    fn prepare_active_motion(
        self, state: VehicleState, delta_s: f32,
        waiting_stop: Option<crate::kernel::waiting::WaitingStopConstraint>,
        conflict_stop: Option<crate::kernel::waiting::WaitingStopConstraint>,
        parking_binding: Option<ParkingBinding>, horizon: Option<LeaderQueryHorizon>,
        leader_gap_override: Option<Option<i64>>,
    ) -> Option<PreparedActiveMotion<'a>> {{
{prefix}
        let iidm = IidmInput {{ speed: si_speed(state.speed_mm_s), desired: si_speed(desired_mm_s),
            gap: leader_gap_m(leader_gap), min_gap: si_meters(profile.min_gap_mm()),
            headway: profile.time_headway(), accel: profile.max_accel(),
            comfort: profile.comfort_decel(), emergency: profile.emergency_decel(), delta: delta_s }};
        Some(PreparedActiveMotion {{ state, compiled, lengths, speed_limits, cursor, edge, profile,
            leader_gap, route_end, reach, parking, movement_stop, waiting_stop, conflict_stop,
            delta_s, desired_mm_s, iidm }})
    }}

    fn finish_active_motion(
        self, prepared: PreparedActiveMotion<'a>, motion_bounds: Option<&mut MotionBounds>,
        leader_constraint_only: bool, iidm: Option<(f32, f32)>,
    ) -> Option<VehicleState> {{
        let PreparedActiveMotion {{ mut state, compiled, lengths, speed_limits, cursor, edge, profile,
            leader_gap, route_end, reach, parking, movement_stop, waiting_stop, conflict_stop,
            delta_s, desired_mm_s, .. }} = prepared;
        let (mut travel_m, next_speed_m) = si_comfort_travel_precomputed(
            state.speed_mm_s, desired_mm_s, leader_gap, profile, route_end, movement_stop,
            compiled, lengths, speed_limits, cursor, state.progress_mm, delta_s, iidm)?;
{tail}
"#
    );
    replace(text, &original, &rewritten)?;

    // 仅 P5 批次传入预计算算术；路线限速、安全投影与量化继续由原后半段处理。
    replace(
        text,
        "fn si_comfort_travel(",
        "fn si_comfort_travel_precomputed(",
    )?;
    let begin = text
        .find("fn si_comfort_travel_precomputed(")
        .ok_or("SI declaration")?;
    let end = text[begin..]
        .find("\nfn clamp_si_travel(")
        .ok_or("SI end")?
        + begin;
    let mut si = text[begin..end].to_owned();
    si = si.replace("    desired_mm_s: u32,", "    _desired_mm_s: u32,");
    si = si.replace(
        "    delta_s: f32,\n) ->",
        "    delta_s: f32,\n    iidm: Option<(f32, f32)>,\n) ->",
    );
    replace(&mut si, "    let desired = si_speed(desired_mm_s);\n", "")?;
    replace(
        &mut si,
        "    let (mut travel, mut next_speed) = iidm_travel(speed, desired, leader_m, profile, delta_s)?;",
        "    let (mut travel, mut next_speed) = iidm?;",
    )?;
    text.replace_range(begin..end, &si);
    let begin = text.find("fn iidm_travel(").ok_or("unused IIDM wrapper")?;
    let end = text[begin..]
        .find("#[allow(clippy::too_many_arguments)]\nfn iidm_step(")
        .ok_or("IIDM scalar boundary")?
        + begin;
    text.replace_range(begin..end, "");
    replace(
        text,
        "    use std::sync::atomic::{AtomicUsize, Ordering};",
        "    use std::sync::atomic::AtomicUsize;",
    )?;

    let begin = text
        .find("    fn vehicle_motion_outcome(")
        .ok_or("outcome")?;
    let body = text[begin..]
        .find("        #[cfg(test)]")
        .ok_or("outcome body")?
        + begin;
    let end = text[body..]
        .find("        let next = reused")
        .ok_or("outcome split")?
        + body;
    let mut prefix = text[body..end].to_owned();
    replace(
        &mut prefix,
        "        #[cfg(test)]\n        let cache_served = reused.is_some();\n",
        "",
    )?;
    let method = format!(
        r#"
impl<'a> MotionTaskView<'a> {{
    fn prepare_vehicle_motion(self, state: &VehicleState, active_index: usize, delta_s: f32)
        -> Result<PreparedVehicleMotion<'a>, StepError> {{
{prefix}
        let motion = if let Some(next) = reused {{ PreparedMotionState::Reused(next) }} else {{
            PreparedMotionState::Compute(self.read.prepare_active_motion(*state, delta_s,
                waiting_stop, conflict_stop, parking_binding,
                cached.and_then(|entry| entry.horizon), None).ok_or(StepError::NonFiniteMotion)?)
        }};
        Ok(PreparedVehicleMotion {{ motion, reservation, arrived_before, handle }})
    }}
}}
"#
    );
    let begin = text
        .find("        for (offset, slot) in chunk.iter_mut().enumerate() {")
        .ok_or("chunk loop")?;
    let end = text[begin..]
        .find("        #[cfg(test)]\n        if let (Some(records), Some(baseline))")
        .ok_or("chunk loop end")?
        + begin;
    text.replace_range(
        begin..end,
        "        view.compute_motion_batch(start, chunk, delta_s, &first_error);\n",
    );
    text.push_str(include_str!("../simd_kernel.rs"));
    text.push_str(&include_str!("../simd_motion.rs").replace(
        "BATCH_SOLVE",
        if arm == "layout" { "scalar" } else { "simd" },
    ));
    text.push_str(&method);
    Ok(())
}

pub(crate) fn export(root: &Path, arm: &str) -> Result<()> {
    need(["base", "layout", "candidate"].contains(&arm), "SIMD arm")?;
    let repo = std::env::current_dir()?;
    io::git(&repo, &["merge-base", "--is-ancestor", BASE, "HEAD"])?;
    fs::create_dir_all(root)?;
    chunk_build::ensure_outputs(root, arm, "plain")?;
    let source = root.join(format!("{arm}-plain-source"));
    let mut index = prepare::export_at(&repo, &source, "plain", BASE)?;
    if arm != "base" {
        let mut text = fs::read_to_string(source.join(TICK))?.replace("\r\n", "\n");
        patch_tick(&mut text, arm)?;
        fs::write(source.join(TICK), text)?;
        let manifest = source.join("crates/laneflow-runtime/Cargo.toml");
        let mut text = fs::read_to_string(&manifest)?.replace("\r\n", "\n");
        replace(
            &mut text,
            "[dependencies]\n",
            "[dependencies]\nwide = \"=1.5.0\"\n",
        )?;
        fs::write(manifest, text)?;
        let research_manifest = source.join("research/issue-762-post-p3-hotspots/Cargo.toml");
        let mut text = fs::read_to_string(&research_manifest)?.replace("\r\n", "\n");
        replace(
            &mut text,
            "[dependencies]\n",
            "[dependencies]\nwide = \"=1.5.0\"\n",
        )?;
        fs::write(research_manifest, text)?;
        // 两臂共用同一锁文件，导出树独立添加研究 SIMD 依赖。
        fs::copy(repo.join("Cargo.lock"), source.join("Cargo.lock"))?;
        let mut lock = fs::read_to_string(source.join("Cargo.lock"))?.replace("\r\n", "\n");
        let start = lock
            .find("name = \"laneflow-runtime\"")
            .ok_or("runtime lock")?;
        let end = lock[start..]
            .find("\n[[package]]")
            .ok_or("runtime lock end")?
            + start;
        let mut entry = lock[start..end].to_owned();
        replace(&mut entry, " \"toml\",\n", " \"toml\",\n \"wide\",\n")?;
        lock.replace_range(start..end, &entry);
        fs::write(source.join("Cargo.lock"), lock)?;
    }
    index["arm"] = json!(arm);
    index["mode"] = json!("plain");
    index["protocol"] = json!(EXPERIMENT.protocol);
    index["batch_lanes"] = json!(if arm == "base" { 1 } else { 4 });
    index["source_files"] = io::source_index(&source)?;
    index["build"] = chunk_build::build(root, &source, &index)?;
    io::write_new(&root.join(format!("{arm}-plain-source.json")), &index)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unknown_arm_and_missing_patch_anchors() {
        assert!(patch_tick(&mut String::new(), "layout").is_err());
        assert!(patch_tick(&mut String::new(), "unknown").is_err());
    }

    #[test]
    fn both_variants_patch_current_tick_and_preserve_commit() {
        let original = include_str!("../../../crates/laneflow-runtime/src/kernel/tick.rs");
        for arm in ["layout", "candidate"] {
            let mut text = original.replace("\r\n", "\n");
            patch_tick(&mut text, arm).unwrap();
            assert!(!text.contains("BATCH_SOLVE"));
            assert!(text.contains(
                "execution.try_for_each_chunk(view.read, slots, &first_error, chunk_size, compute)"
            ));
            assert!(text.contains(
                "push_parking_arrival(parking_arrivals, arrival, view.read.binding.world_id)?"
            ));
        }
    }
}
