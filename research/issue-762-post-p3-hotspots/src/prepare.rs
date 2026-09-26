use crate::{BASE, Result, io, need};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

const K: &str = "crates/laneflow-runtime/src/kernel/";

fn edit(root: &Path, relative: &str, old: &str, new: &str) -> Result<()> {
    let path = root.join(relative);
    let text = fs::read_to_string(&path)?.replace("\r\n", "\n");
    need(
        text.matches(old).count() == 1,
        &format!("anchor count: {relative}: {old}"),
    )?;
    fs::write(path, text.replacen(old, new, 1))?;
    Ok(())
}

pub(crate) fn export(repo: &Path, destination: &Path, mode: &str) -> Result<Value> {
    io::ensure_new(destination)?;
    fs::create_dir_all(destination.parent().unwrap_or(Path::new(".")))?;
    fs::create_dir(destination)?;
    let archive = destination.join(".source-export.tar");
    let output = Command::new("git")
        .current_dir(repo)
        .args(["archive", "--format=tar", BASE, "--output"])
        .arg(&archive)
        .output()?;
    need(output.status.success(), "git archive failed")?;
    let status = Command::new("tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(destination)
        .status()?;
    need(status.success(), "tar extraction failed")?;
    // 只删除此函数刚创建的临时归档，不递归清理任何目录。
    fs::remove_file(archive)?;
    let stages = mode != "plain";
    let reset = if stages {
        "                laneflow_runtime::research_reset();\n"
    } else {
        ""
    };
    let arrays = if stages {
        "laneflow_runtime::research_take()"
    } else {
        "([0_u128; 12], [0_u64; 12])"
    };
    let clock = "                let started = Instant::now();\n                let result = world.step(input);\n                let elapsed = nanos(started.elapsed());";
    let log = r#"                eprintln!("LF762 {{\"tick\":{},\"step_ns\":{},\"stages_ns\":{:?},\"calls\":{:?}}}", world.tick_index(), elapsed, stages, calls);"#;
    edit(
        destination,
        "tools/laneflow-urban-harness/src/host.rs",
        clock,
        &format!("{reset}{clock}\n                let (stages, calls) = {arrays};\n{log}"),
    )?;
    if stages {
        edit(
            destination,
            &format!("{K}mod.rs"),
            "#[cfg(test)]\npub(crate) mod performance_profile;",
            "pub(crate) mod performance_profile;",
        )?;
        let path = destination.join(format!("{K}tick.rs"));
        let original = fs::read_to_string(&path)?.replace("\r\n", "\n");
        let mut text = original.clone();
        let mut count = 0;
        for line in original.lines() {
            let trimmed = line.trim();
            let timer = [
                "_commit",
                "preflight",
                "occupancy",
                "waiting",
                "conflict",
                "motion",
                "signal",
                "output",
            ]
            .iter()
            .any(|name| {
                trimmed == format!("let {name}_timer =")
                    || trimmed == format!("drop({name}_timer);")
            });
            if timer && line.starts_with("        ") {
                let anchor = format!("        #[cfg(test)]\n{line}");
                count += text.matches(&anchor).count();
                text = text.replace(&anchor, line);
            }
        }
        need(count == 19, "batch-clock anchor count")?;
        fs::write(path, text)?;
        fs::write(
            destination.join(format!("{K}performance_profile.rs")),
            include_str!("../profile.rs").replace("\r\n", "\n"),
        )?;
        let lib = destination.join("crates/laneflow-runtime/src/lib.rs");
        let mut text = fs::read_to_string(&lib)?;
        text.push_str("\n/// 研究专用；在公共 step 计时外重置协调器时钟。\n#[doc(hidden)]\npub fn research_reset() { kernel::performance_profile::reset(); }\n/// 研究专用；读取协调器批次墙钟，不累加 worker CPU。\n#[doc(hidden)]\npub fn research_take() -> ([u128; 12], [u64; 12]) { kernel::performance_profile::take() }\n");
        fs::write(lib, text)?;
        edit(
            destination,
            &format!("{K}conflict_tick.rs"),
            "        self.rebuild_conflict_frontier()?;",
            "        let frontier_timer = super::performance_profile::begin(super::performance_profile::Stage::Frontier);\n        self.rebuild_conflict_frontier()?;\n        drop(frontier_timer);",
        )?;
        edit(
            destination,
            &format!("{K}conflict_tick.rs"),
            "        self.acquire_conflict_candidates(tick)\n    }",
            "        let _p4_timer = super::performance_profile::begin(super::performance_profile::Stage::P4);\n        self.acquire_conflict_candidates(tick)\n    }",
        )?;
    }
    if mode == "detail" {
        detail(destination)?;
    }
    Ok(json!({"base":BASE,"mode":mode,"source_files":io::source_index(destination)?}))
}

fn detail(root: &Path) -> Result<()> {
    for relative in [
        format!("{K}performance_profile.rs"),
        "crates/laneflow-runtime/src/lib.rs".to_owned(),
    ] {
        let path = root.join(&relative);
        let mut text = fs::read_to_string(&path)?
            .replace("\r\n", "\n")
            .replace("; 12]", "; 18]");
        if relative.ends_with("performance_profile.rs") {
            text = text.replace("    P4,", "    P4,\n    P3Discover,\n    P3Dispatch,\n    P3Consume,\n    P3Fused,\n    P3Tail,");
            text.push_str("\npub(crate) fn workload(count: usize) { CALLS.with(|cell| { let mut values = cell.get(); values[17] = count as u64; cell.set(values); }); }\n");
        }
        fs::write(path, text)?;
    }
    let file = format!("{K}conflict_tick.rs");
    for (old, new) in [
        (
            "    ) -> Result<bool, StepError> {\n        let view = ConflictTaskView {",
            "    ) -> Result<bool, StepError> {\n        let discover_timer = super::performance_profile::begin(super::performance_profile::Stage::P3Discover);\n        let view = ConflictTaskView {",
        ),
        (
            "        let workload = inputs.len();",
            "        let workload = inputs.len();\n        super::performance_profile::workload(workload);\n        drop(discover_timer);",
        ),
        (
            "        let dispatch_stats =\n            execution.try_for_each_chunk(view.read, slots, &first_error, chunk_size, compute);",
            "        let dispatch_timer = super::performance_profile::begin(super::performance_profile::Stage::P3Dispatch);\n        let dispatch_stats =\n            execution.try_for_each_chunk(view.read, slots, &first_error, chunk_size, compute);\n        drop(dispatch_timer);",
        ),
        (
            "        for index in 0..self.workspace.conflict_inputs.len() {",
            "        let _consume_timer = super::performance_profile::begin(super::performance_profile::Stage::P3Consume);\n        for index in 0..self.workspace.conflict_inputs.len() {",
        ),
        (
            "        if !dispatched {",
            "        if !dispatched {\n            let _fused_timer = super::performance_profile::begin(super::performance_profile::Stage::P3Fused);",
        ),
        (
            "        reserve(\n            &mut self.workspace.conflict_staged_decisions,\n            self.workspace.conflict_candidates.len(),",
            "        let _tail_timer = super::performance_profile::begin(super::performance_profile::Stage::P3Tail);\n        reserve(\n            &mut self.workspace.conflict_staged_decisions,\n            self.workspace.conflict_candidates.len(),",
        ),
    ] {
        edit(root, &file, old, new)?;
    }
    Ok(())
}
