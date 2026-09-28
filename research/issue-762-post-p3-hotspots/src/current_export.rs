use crate::{BASE, PROTOCOL, Result, STAGES, io, need, p2_cost::prepare};
use serde_json::json;
use std::{fs, path::Path};

const TICK: &str = "crates/laneflow-runtime/src/kernel/tick.rs";
const WAITING: &str = "crates/laneflow-runtime/src/kernel/waiting.rs";
const CONFLICT: &str = "crates/laneflow-runtime/src/kernel/conflict_tick.rs";
const PROFILE: &str = "crates/laneflow-runtime/src/kernel/performance_profile.rs";

fn patch(root: &Path, file: &str, old: &str, new: &str) -> Result<()> {
    let path = root.join(file);
    let text = fs::read_to_string(&path)?.replace("\r\n", "\n");
    need(
        text.matches(old).count() == 1,
        &format!("anchor {file}: {old}"),
    )?;
    fs::write(path, text.replacen(old, new, 1))?;
    Ok(())
}
fn before(root: &Path, file: &str, anchor: &str, code: &str) -> Result<()> {
    patch(root, file, anchor, &format!("{code}\n{anchor}"))
}
fn after(root: &Path, file: &str, anchor: &str, code: &str) -> Result<()> {
    patch(root, file, anchor, &format!("{anchor}\n{code}"))
}
fn begin(stage: &str) -> String {
    let variable = stage.to_ascii_lowercase();
    format!(
        "let _timer_{variable} = super::performance_profile::begin(super::performance_profile::Stage::{stage});"
    )
}
fn span(root: &Path, file: &str, anchor: &str, stage: &str) -> Result<()> {
    before(root, file, anchor, &begin(stage))?;
    after(
        root,
        file,
        anchor,
        &format!("drop(_timer_{});", stage.to_ascii_lowercase()),
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
    index["protocol"] = json!(PROTOCOL.name);
    io::write_new(&root.join(format!("{mode}-source.json")), &index)
}
fn instrument(root: &Path) -> Result<()> {
    for file in [PROFILE, "crates/laneflow-runtime/src/lib.rs"] {
        let path = root.join(file);
        fs::write(&path, fs::read_to_string(&path)?.replace("; 18]", "; 34]"))?;
    }
    let variants = STAGES[17..31]
        .iter()
        .map(|s| format!("    {s},"))
        .collect::<Vec<_>>()
        .join("\n");
    after(root, PROFILE, "    P3Tail,", &variants)?;
    let mut profile = fs::read_to_string(root.join(PROFILE))?;
    profile.push_str("\npub(crate) fn count(index: usize, value: usize) { CALLS.with(|cell| { let mut calls = cell.get(); calls[index] = value as u64; cell.set(calls); }); }\n");
    fs::write(root.join(PROFILE), profile)?;

    let setup = "    // Active 投影是任务的唯一位置表；任务在对应位置重新读取完整句柄并核对";
    before(root, TICK, setup, &begin("P5Setup"))?;
    after(
        root,
        TICK,
        "    let workload = view.read.derived.active_order.len();",
        "drop(_timer_p5setup);\nsuper::performance_profile::count(31, workload);",
    )?;
    before(
        root,
        TICK,
        "    let slots = &mut workspace.motion_slots;",
        &begin("P5Slots"),
    )?;
    // 该 resize 在 P5 中唯一；P2 位于 waiting.rs。
    after(
        root,
        TICK,
        "    slots.resize(workload, crate::kernel::execution::DispatchSlot::Pending);",
        "drop(_timer_p5slots);",
    )?;
    span(
        root,
        TICK,
        "    let dispatch_stats =\n        execution.try_for_each_chunk(view.read, slots, &first_error, chunk_size, compute);",
        "P5Dispatch",
    )?;
    before(
        root,
        TICK,
        "    for (vehicle, slot) in view\n        .read\n        .derived\n        .active_order",
        &begin("P5Consume"),
    )?;
    // 融合入口通过含完整签名尾的唯一锚点定位，计时到函数返回。
    patch(
        root,
        TICK,
        "    updates: &mut Vec<(usize, VehicleState)>,\n) -> Result<(), StepError> {\n    let view = MotionTaskView {\n",
        &format!(
            "    updates: &mut Vec<(usize, VehicleState)>,\n) -> Result<(), StepError> {{\n{}\n    let view = MotionTaskView {{\n",
            begin("P5Fused")
        ),
    )?;

    before(
        root,
        WAITING,
        "        match execution {\n            Some(resources @ crate::kernel::execution::ExecutionResources::Pool(_))",
        &begin("WaitingPreview"),
    )?;
    before(
        root,
        WAITING,
        "        #[cfg(test)]\n        let _assembly = preview_stage::begin(preview_stage::ASSEMBLY);",
        &format!("drop(_timer_waitingpreview);\n{}", begin("WaitingAssembly")),
    )?;
    let clear_anchor = "        self.workspace.waiting_plan_by_vehicle.fill(None);\n        #[cfg(test)]\n        drop(sparse_clear);\n        reserve_waiting_exact(";
    patch(
        root,
        WAITING,
        clear_anchor,
        &format!(
            "{}\n{}",
            begin("WaitingClear"),
            clear_anchor.replace("fill(None);", "fill(None);\ndrop(_timer_waitingclear);")
        ),
    )?;
    span(
        root,
        CONFLICT,
        "            self.workspace.conflict_motion_by_vehicle.fill(None);",
        "ConflictMotionClear",
    )?;
    span(
        root,
        CONFLICT,
        "            self.workspace.conflict_next_eligibility.fill(None);",
        "EligibilityClear",
    )?;
    after(
        root,
        CONFLICT,
        "    pub(crate) fn commit_conflict_step(&mut self) {",
        &begin("P7Eligibility"),
    )?;
    after(
        root,
        CONFLICT,
        "        let all_empty = {",
        &begin("P7Scan"),
    )?;
    before(
        root,
        CONFLICT,
        "        if !all_empty {",
        "super::performance_profile::count(32, self.workspace.conflict_next_eligibility.len());\nsuper::performance_profile::count(33, usize::from(all_empty));",
    )?;
    after(root, CONFLICT, "        if !all_empty {", &begin("P7Copy"))?;
    Ok(())
}
