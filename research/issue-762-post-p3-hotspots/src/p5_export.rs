use crate::{BASE, Result, STAGES, io, need, p2_cost::prepare};
use serde_json::json;
use std::{fs, path::Path};

fn patch(
    root: &Path,
    file: &str,
    range: Option<(&str, &str)>,
    edits: &[(&str, String)],
) -> Result<()> {
    let path = root.join(file);
    let text = fs::read_to_string(&path)?.replace("\r\n", "\n");
    let (start, end) = if let Some((a, b)) = range {
        let start = text.find(a).ok_or("range start")?;
        (start, start + text[start..].find(b).ok_or("range end")?)
    } else {
        (0, text.len())
    };
    need(start < end, "patch range")?;
    let mut body = text[start..end].to_owned();
    for (old, new) in edits {
        need(
            body.matches(old).count() == 1,
            &format!("anchor {file}: {old}"),
        )?;
        body = body.replacen(old, new, 1);
    }
    fs::write(path, format!("{}{}{}", &text[..start], body, &text[end..]))?;
    Ok(())
}
fn before<'a>(old: &'a str, code: &str) -> (&'a str, String) {
    (old, format!("{code}\n{old}"))
}
fn after<'a>(old: &'a str, code: &str) -> (&'a str, String) {
    (old, format!("{old}\n{code}"))
}
fn begin(name: &str, stage: &str, sample: bool) -> String {
    format!(
        "        let {name} = super::performance_profile::{}(super::performance_profile::Stage::{stage});",
        if sample { "sample" } else { "begin" }
    )
}
pub(crate) fn export(root: &Path, mode: &str) -> Result<()> {
    need(["plain", "detail"].contains(&mode), "mode")?;
    let repo = std::env::current_dir()?;
    io::git(&repo, &["merge-base", "--is-ancestor", BASE, "HEAD"])?;
    fs::create_dir_all(root)?;
    let source = root.join(format!("{mode}-source"));
    let mut index = prepare::export_at(&repo, &source, mode, BASE)?;
    if mode == "detail" {
        instrument(&source)?;
        index["source_files"] = io::source_index(&source)?;
    }
    index["protocol"] = json!("p5-cost-v1");
    io::write_new(&root.join(format!("{mode}-source.json")), &index)
}
fn instrument(root: &Path) -> Result<()> {
    let profile = "crates/laneflow-runtime/src/kernel/performance_profile.rs";
    for file in [profile, "crates/laneflow-runtime/src/lib.rs"] {
        let path = root.join(file);
        fs::write(&path, fs::read_to_string(&path)?.replace("; 18]", "; 55]"))?;
    }
    let variants = STAGES[18..38]
        .iter()
        .map(|s| format!("    {s},"))
        .collect::<Vec<_>>()
        .join("\n");
    patch(
        root,
        profile,
        None,
        &[
            (
                "    P3Tail,",
                format!("    P3Tail,\n    P3Workload,\n{variants}"),
            ),
            after("    NANOS.set([0; 55]);", "    reset_p5();"),
            ("    (NANOS.get(), CALLS.get())", "    take_p5()".into()),
        ],
    )?;
    let path = root.join(profile);
    let mut text = fs::read_to_string(&path)?;
    text.push_str(include_str!("p5_profile.rs"));
    fs::write(path, text)?;
    patch(
        root,
        "crates/laneflow-runtime/src/kernel/execution.rs",
        None,
        &[after(
            "    compute(view, start, chunk);",
            "    super::performance_profile::flush_p5();",
        )],
    )?;
    let tick = "crates/laneflow-runtime/src/kernel/tick.rs";
    patch(
        root,
        tick,
        Some((
            "fn prepare_motion_dispatched(",
            "impl crate::kernel::phase::StepWorkspace<'_> {",
        )),
        &[
            before(
                "    let inputs = &mut workspace.motion_inputs;",
                &begin("discover_timer", "P5Discover", false),
            ),
            after(
                "    let workload = inputs.len();",
                "    super::performance_profile::count_by(6, workload as u64);\n    drop(discover_timer);",
            ),
            before(
                "    let slots = &mut workspace.motion_slots;",
                &begin("slots_timer", "P5Slots", false),
            ),
            after(
                "    slots.resize(workload, crate::kernel::execution::DispatchSlot::Pending);",
                "    drop(slots_timer);",
            ),
            before(
                "    let dispatch_stats =",
                &begin("dispatch_timer", "P5Dispatch", false),
            ),
            after(
                "        execution.try_for_each_chunk(view.read, slots, &first_error, chunk_size, compute);",
                "    drop(dispatch_timer);",
            ),
            before(
                "    for ((vehicle, _active_index, _state), slot) in workspace.motion_inputs.iter().zip(slots.iter())",
                &begin("_consume_timer", "P5Consume", false),
            ),
        ],
    )?;
    patch(
        root,
        tick,
        Some(("fn prepare_motion_fused(", "fn prepare_motion_dispatched(")),
        &[before(
            "    let view = MotionTaskView {",
            &begin("_fused_timer", "P5Fused", false),
        )],
    )?;
    patch(
        root,
        tick,
        Some((
            "    fn vehicle_motion_outcome(",
            "    pub(crate) fn motion_conflict_stop_for(",
        )),
        &[
            before(
                "        let handle = state.handle;",
                "        let _scope = super::performance_profile::sample_scope(active_index, self.read.committed.tick_index);\n        let _sample_total = super::performance_profile::sample(super::performance_profile::Stage::SampleTotal);\n        drop(super::performance_profile::sample(super::performance_profile::Stage::ClockFloor));\n        let inputs_timer = super::performance_profile::sample(super::performance_profile::Stage::SampleInputs);",
            ),
            before(
                "        let waiting_stop = self.waiting_stop_for(state)?;",
                "        drop(inputs_timer);\n        let stops_timer = super::performance_profile::sample(super::performance_profile::Stage::SampleStops);",
            ),
            before(
                "        let cached = self",
                &format!(
                    "        drop(stops_timer);\n{}",
                    begin("reuse_timer", "SampleReuse", true)
                ),
            ),
            before(
                "        #[cfg(test)]\n        let cache_served = reused.is_some();",
                "        drop(reuse_timer);\n        super::performance_profile::classify(reused.is_some());",
            ),
            after(
                "            .or_else(|| {",
                &begin("_recompute_timer", "SampleRecompute", true),
            ),
            before(
                "        let arrival = if let Some(reservation) = reservation {",
                &begin("_arrival_timer", "SampleArrival", true),
            ),
            before(
                "        Ok(VehicleMotionOutcome { next, arrival })",
                "        if arrival.is_some() {super::performance_profile::count_by(16,1);}",
            ),
        ],
    )?;
    patch(root,tick,Some(("    fn calculate_active_vehicle_motion(","    pub(crate) fn parking_stop_distance(")),&[
        before("        let compiled = self.compiled_route(state.route)?;",&format!("        super::performance_profile::count_motion(7);\n{}",begin("motion_inputs","MotionInputs",true))),
        after("        let desired_mm_s = profile.desired_speed_mm_s().min(current_limit);","        drop(motion_inputs);"),
        before("        let horizon = match horizon {",&begin("motion_horizon","MotionHorizon",true)),
        ("            Some(horizon) => horizon,","            Some(horizon) => {super::performance_profile::count_motion(8); horizon},".into()),
        ("            None => leader_query_horizon(state.speed_mm_s, profile, delta_s)?,","            None => {super::performance_profile::count_motion(9); leader_query_horizon(state.speed_mm_s, profile, delta_s)?},".into()),
        before("        #[cfg(test)]\n        drop(horizon_timer);","        drop(motion_horizon);"),
        before("        let leader_gap = match leader_gap_override {",&begin("leader_timer","LeaderGap",true)),
        before("        #[cfg(test)]\n        drop(gap_timer);","        drop(leader_timer);"),
        before("        let route_end =",&begin("route_stops","RouteStops",true)),
        before("        let parking_stop = parking.map(|(_, distance)| distance);","        drop(route_stops);"),
        before("        if let Some(gate_stop) = self.restrictive_gate_stop(compiled, &state, cursor, reach) {",&begin("gate_stop_timer","GateStop",true)),
        before("        let (mut travel_m, next_speed_m) = si_comfort_travel(",&format!("        drop(gate_stop_timer);\n{}",begin("solve_timer","Solve",true))),
        before("        if travel_m < 0.0 {",&format!("        drop(solve_timer);\n{}",begin("hard_room_timer","HardRoom",true))),
        before("        if hard_room == 0 {","        drop(hard_room_timer);"),
        after("        if hard_room == 0 {","            super::performance_profile::count_motion(12);"),
        before("        let um = u64::from(state.carry_um).saturating_add(round_um(f64::from(travel_m))?);",&begin("_next_state","NextState",true)),
        before("        let committed_index = usize::try_from(state.route_edge_index).ok()?;","        super::performance_profile::count_motion(if travel_mm == 0 {13} else if state.route_edge_index as usize == cursor {14} else {15});"),
    ])?;
    patch(
        root,
        tick,
        Some((
            "    pub(crate) fn signal_stop_distance(",
            "    fn restrictive_gate_stop(",
        )),
        &[before(
            "            if self.gate_is_restrictive(next.gate, state.profile) {",
            "            super::performance_profile::count_motion(10);",
        )],
    )?;
    patch(
        root,
        tick,
        Some(("    fn restrictive_gate_stop(", "/// P5")),
        &[before(
            "            if compiled\n                .hop_gate",
            "            super::performance_profile::count_motion(11);",
        )],
    )?;
    Ok(())
}
