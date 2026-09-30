use crate::{BASE, EXPERIMENT, Result, cache_research::prepare, io, need};
use serde_json::json;
use std::{fs, path::Path};
const TICK: &str = "crates/laneflow-runtime/src/kernel/tick.rs";
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
pub(crate) fn export(root: &Path, arm: &str, mode: &str) -> Result<()> {
    need(
        ["base", "candidate"].contains(&arm) && ["plain", "detail"].contains(&mode),
        "arm/mode",
    )?;
    let repo = std::env::current_dir()?;
    io::git(&repo, &["merge-base", "--is-ancestor", BASE, "HEAD"])?;
    fs::create_dir_all(root)?;
    let source = root.join(format!("{arm}-{mode}-source"));
    let mut index = prepare::export_at(
        &repo,
        &source,
        if mode == "detail" { "stages" } else { mode },
        BASE,
    )?;
    let factor = if arm == "base" { 2 } else { 4 };
    if arm == "candidate" {
        patch(
            &source,
            TICK,
            "    // 块数 = 线程数 × 2 与活动数取较小者；语义中立（与 P2 同默认值）。\n    let chunk_count = execution\n        .dispatch_threads()\n        .saturating_mul(2)",
            "    // #801 研究候选：P5 每线程目标四块，规范消费与领域原语保持。\n    let chunk_count = execution\n        .dispatch_threads()\n        .saturating_mul(4)",
        )?;
        // 候选只有分块预期变化；真实参与/重叠及完整 join 断言保持原样。
        patch(
            &source,
            "crates/laneflow-runtime/src/kernel/waiting.rs",
            "            assert_eq!(starts, (0..64).step_by(8).collect::<Vec<_>>());\n            let stats = last_motion_dispatch_stats().expect(\"分发统计\");\n            assert_eq!(stats.dispatched_chunks, 8);\n            assert_eq!(stats.completed_chunks, 8);\n            assert_eq!(stats.ticket_grabs, 8);",
            "            assert_eq!(starts, (0..64).step_by(4).collect::<Vec<_>>());\n            let stats = last_motion_dispatch_stats().expect(\"分发统计\");\n            assert_eq!(stats.dispatched_chunks, 16);\n            assert_eq!(stats.completed_chunks, 16);\n            assert_eq!(stats.ticket_grabs, 16);",
        )?;
    }
    if mode == "detail" {
        instrument(&source)?;
    }
    index["arm"] = json!(arm);
    index["mode"] = json!(mode);
    index["protocol"] = json!(EXPERIMENT.protocol);
    index["chunks_per_worker"] = json!(factor);
    index["source_files"] = io::source_index(&source)?;
    io::write_new(&root.join(format!("{arm}-{mode}-source.json")), &index)
}
fn instrument(root: &Path) -> Result<()> {
    let mut profile = fs::read_to_string(root.join(PROFILE))?;
    profile.push_str(include_str!("../chunk_profile.rs"));
    fs::write(root.join(PROFILE), profile)?;
    patch(
        root,
        PROFILE,
        "    NANOS.set([0; 12]);",
        "    NANOS.set([0; 12]);\n    CHUNK_REPORT.with(|cell| cell.borrow_mut().take());",
    )?;
    patch(
        root,
        PROFILE,
        "    (NANOS.get(), CALLS.get())",
        "    emit_chunks();\n    (NANOS.get(), CALLS.get())",
    )?;
    patch(
        root,
        TICK,
        "    let compute = |_chunk_view: crate::kernel::phase::StepReadView<'_>,",
        "    let timing_records = (0..chunk_count).map(|_| super::performance_profile::ChunkTiming::default()).collect::<Vec<_>>();\n    let timing_caller = execution.dispatch_threads() - 1;\n    let timing_origin = std::time::Instant::now();\n    let compute = |_chunk_view: crate::kernel::phase::StepReadView<'_>,",
    )?;
    patch(
        root,
        TICK,
        "    >]| {\n        #[cfg(test)]\n        if let Some(probe) = &participation {",
        "    >]| {\n        let timing_start = timing_origin.elapsed().as_nanos() as u64;\n        #[cfg(test)]\n        if let Some(probe) = &participation {",
    )?;
    patch(
        root,
        TICK,
        "        #[cfg(test)]\n        if let (Some(records), Some(baseline)) = (&chunk_records, chunk_baseline) {\n            records[start / chunk_size].store_deltas(baseline);\n        }",
        "        timing_records[start / chunk_size].store(start, chunk.len(), timing_start, timing_origin.elapsed().as_nanos() as u64, rayon_core::current_thread_index().unwrap_or(timing_caller));\n        #[cfg(test)]\n        if let (Some(records), Some(baseline)) = (&chunk_records, chunk_baseline) {\n            records[start / chunk_size].store_deltas(baseline);\n        }",
    )?;
    patch(
        root,
        TICK,
        "    let dispatch_stats =\n        execution.try_for_each_chunk(view.read, slots, &first_error, chunk_size, compute);",
        "    let dispatch_stats =\n        execution.try_for_each_chunk(view.read, slots, &first_error, chunk_size, compute);\n    let dispatch_ns = timing_origin.elapsed().as_nanos() as u64;\n    super::performance_profile::save_chunks(view.read.committed.tick_index + 1, workload, chunk_size, dispatch_ns, &timing_records);",
    )?;
    Ok(())
}
