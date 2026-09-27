use super::{BASE, Result, io, need, prepare};
use serde_json::json;
use std::{fs, path::Path};

fn edit(root: &Path, relative: &str, old: &str, new: &str) -> Result<()> {
    let path = root.join(relative);
    let text = fs::read_to_string(&path)?.replace("\r\n", "\n");
    let (start, end) = if relative.ends_with("/tick.rs") {
        (
            text.find("    pub(crate) fn waiting_preview_entry(")
                .ok_or("preview start")?,
            text.find("    pub(crate) fn placement_motion(")
                .ok_or("preview end")?,
        )
    } else {
        (0, text.len())
    };
    need(start < end, "edit range")?;
    let body = &text[start..end];
    need(
        body.matches(old).count() == 1,
        &format!("anchor {relative}: {old}"),
    )?;
    fs::write(
        path,
        format!(
            "{}{}{}",
            &text[..start],
            body.replacen(old, new, 1),
            &text[end..]
        ),
    )?;
    Ok(())
}
fn timer(root: &Path, file: &str, old: &str, stage: &str, end: bool) -> Result<()> {
    let name = stage.to_ascii_lowercase();
    let line = if end {
        format!("        drop({name}_timer);")
    } else {
        format!(
            "        let {name}_timer = super::performance_profile::begin(super::performance_profile::Stage::{stage});"
        )
    };
    edit(
        root,
        file,
        old,
        &if end {
            format!("{old}\n{line}")
        } else {
            format!("{line}\n{old}")
        },
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
        let profile = "crates/laneflow-runtime/src/kernel/performance_profile.rs";
        for file in [profile, "crates/laneflow-runtime/src/lib.rs"] {
            let path = source.join(file);
            fs::write(&path, fs::read_to_string(&path)?.replace("; 18]", "; 32]"))?;
        }
        edit(
            &source,
            profile,
            "    P3Tail,",
            "    P3Tail,\n    P3Workload,\n    P2All,\n    P2Discover,\n    P2Slots,\n    P2Dispatch,\n    P2Consume,\n    P2Fused,\n    WaitingAssembly,",
        )?;
        edit(
            &source,
            profile,
            "    NANOS.set([0; 32]);",
            "    NANOS.set([0; 32]);\n    LOCAL_COUNTS.set([0; 6]);\n    for counter in &COUNTERS { counter.store(0, std::sync::atomic::Ordering::Relaxed); }",
        )?;
        edit(
            &source,
            profile,
            "    (NANOS.get(), CALLS.get())",
            "    flush_counts();\n    let mut calls = CALLS.get();\n    for (index, counter) in COUNTERS.iter().enumerate() { calls[25 + index] = counter.load(std::sync::atomic::Ordering::Relaxed); }\n    (NANOS.get(), calls)",
        )?;
        let mut text = fs::read_to_string(source.join(profile))?;
        text.push_str("\nstatic COUNTERS: [std::sync::atomic::AtomicU64; 6] = [const { std::sync::atomic::AtomicU64::new(0) }; 6];\nthread_local! { static LOCAL_COUNTS: Cell<[u64; 6]> = const { Cell::new([0; 6]) }; }\npub(crate) fn count(index: usize) { LOCAL_COUNTS.with(|cell| { let mut counts = cell.get(); counts[index] += 1; cell.set(counts); }); }\npub(crate) fn flush_counts() { let local = LOCAL_COUNTS.replace([0; 6]); for (counter, value) in COUNTERS.iter().zip(local) { if value != 0 { counter.fetch_add(value, std::sync::atomic::Ordering::Relaxed); } } }\n");
        fs::write(source.join(profile), text)?;
        let mut text = fs::read_to_string(source.join(profile))?;
        text.push_str("\npub(crate) fn p2_workload(count: usize) { CALLS.with(|cell| { let mut calls = cell.get(); calls[31] = count as u64; cell.set(calls); }); }\n");
        fs::write(source.join(profile), text)?;
        edit(
            &source,
            "crates/laneflow-runtime/src/kernel/execution.rs",
            "    compute(view, start, chunk);",
            "    compute(view, start, chunk);\n    super::performance_profile::flush_counts();",
        )?;
        let waiting = "crates/laneflow-runtime/src/kernel/waiting.rs";
        timer(
            &source,
            waiting,
            "        match execution {\n            Some(resources @ crate::kernel::execution::ExecutionResources::Pool(_))",
            "P2All",
            false,
        )?;
        edit(
            &source,
            waiting,
            "        #[cfg(test)]\n        let _assembly =",
            "        drop(p2all_timer);\n        let _waiting_assembly = super::performance_profile::begin(super::performance_profile::Stage::WaitingAssembly);\n        #[cfg(test)]\n        let _assembly =",
        )?;
        timer(
            &source,
            waiting,
            "    let inputs = &mut workspace.waiting_preview_inputs;",
            "P2Discover",
            false,
        )?;
        timer(
            &source,
            waiting,
            "    let workload = inputs.len();",
            "P2Discover",
            true,
        )?;
        edit(
            &source,
            waiting,
            "    let workload = inputs.len();",
            "    let workload = inputs.len();\n    super::performance_profile::p2_workload(workload);",
        )?;
        timer(
            &source,
            waiting,
            "    let slots = &mut workspace.waiting_preview_slots;",
            "P2Slots",
            false,
        )?;
        timer(
            &source,
            waiting,
            "    slots.resize(workload, crate::kernel::execution::DispatchSlot::Pending);",
            "P2Slots",
            true,
        )?;
        timer(
            &source,
            waiting,
            "    let dispatch_stats =\n        execution.try_for_each_chunk(view, slots, &first_error, chunk_size, compute);",
            "P2Dispatch",
            false,
        )?;
        timer(
            &source,
            waiting,
            "        execution.try_for_each_chunk(view, slots, &first_error, chunk_size, compute);",
            "P2Dispatch",
            true,
        )?;
        // cfg(test) 属于原语句；新增正式导出时钟必须放在该属性之前。
        edit(
            &source,
            waiting,
            "    #[cfg(test)]\n    let _consume = preview_stage::begin(preview_stage::CONSUME);",
            "    let _p2_consume_timer = super::performance_profile::begin(super::performance_profile::Stage::P2Consume);\n    #[cfg(test)]\n    let _consume = preview_stage::begin(preview_stage::CONSUME);",
        )?;
        edit(
            &source,
            waiting,
            "    let _fused_loop = preview_stage::begin(preview_stage::FUSED_LOOP);",
            "    let _fused_loop = preview_stage::begin(preview_stage::FUSED_LOOP);\n    let _p2_fused = super::performance_profile::begin(super::performance_profile::Stage::P2Fused);",
        )?;
        let tick = "crates/laneflow-runtime/src/kernel/tick.rs";
        edit(
            &source,
            tick,
            "        let gate_reachable = cache_reachability",
            "        super::performance_profile::count(0);\n        if cache_reachability { super::performance_profile::count(5); }\n        let gate_reachable = cache_reachability",
        )?;
        edit(
            &source,
            tick,
            "        let Some(gate_hop) = compiled.gate_hops.get(gate_index).copied() else {",
            "        let Some(gate_hop) = compiled.gate_hops.get(gate_index).copied() else {\n            super::performance_profile::count(1);",
        )?;
        edit(
            &source,
            tick,
            "        let gate_distance = distance_to_occurrence_start(",
            "        super::performance_profile::count(4);\n        let gate_distance = distance_to_occurrence_start(",
        )?;
        edit(
            &source,
            tick,
            "        let Some(BoundedDistance::Finite(gate_distance_mm)) = gate_distance else {",
            "        let Some(BoundedDistance::Finite(gate_distance_mm)) = gate_distance else {\n            super::performance_profile::count(2);",
        )?;
        edit(
            &source,
            tick,
            "        if gate_distance_mm > horizon.front_query_mm {",
            "        if gate_distance_mm > horizon.front_query_mm {\n            super::performance_profile::count(2);",
        )?;
        edit(
            &source,
            tick,
            "        let preview = self\n            .preview_active_vehicle_with_waiting_stop(state, delta_s, None, Some(horizon))",
            "        super::performance_profile::count(3);\n        let preview = self\n            .preview_active_vehicle_with_waiting_stop(state, delta_s, None, Some(horizon))",
        )?;
        index["source_files"] = io::source_index(&source)?;
    }
    index["protocol"] = json!("p2-cost-v1");
    io::write_new(&root.join(format!("{mode}-source.json")), &index)
}
