//! #814 独立诊断导出。普通测量树只保留公共 step 外的时钟。
use crate::{Result, io, need};
use std::{fs, path::Path};

const K: &str = "crates/laneflow-runtime/src/kernel/";

pub(super) fn patch(root: &Path, file: &str, old: &str, new: &str) -> Result<()> {
    let path = root.join(file);
    let text = fs::read_to_string(&path)?.replace("\r\n", "\n");
    need(
        text.matches(old).count() == 1,
        &format!("814 diagnostic anchor {file}: {old}"),
    )?;
    fs::write(path, text.replacen(old, new, 1))?;
    Ok(())
}

fn function_counter(root: &Path, file: &str, signature: &str, index: usize) -> Result<()> {
    let path = root.join(file);
    let text = fs::read_to_string(&path)?.replace("\r\n", "\n");
    need(text.matches(signature).count() == 1, "814 counter function")?;
    let start = text.find(signature).ok_or("counter signature")?;
    let body = start + text[start..].find('{').ok_or("counter body")? + 1;
    fs::write(
        path,
        format!(
            "{}\ncrate::kernel::tick::note_columnar_work({index}, 1);{}",
            &text[..body],
            &text[body..]
        ),
    )?;
    Ok(())
}

fn promote_guards(text: &str) -> String {
    let lines: Vec<_> = text.lines().collect();
    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            if line.trim() == "#[cfg(test)]" {
                let next = lines[index + 1..]
                    .iter()
                    .find(|line| {
                        !line.trim().starts_with("///") && !line.trim().starts_with("#[derive")
                    })
                    .map_or("", |line| line.trim());
                if next.starts_with("note_columnar_work(")
                    || next.starts_with("static COLUMNAR_WORK")
                    || next.starts_with("fn note_columnar_work")
                    || next.starts_with("pub(crate) fn take_columnar_work")
                {
                    return None;
                }
            }
            Some(format!("{line}\n"))
        })
        .collect()
}

pub(crate) fn instrument(root: &Path, candidate: bool) -> Result<()> {
    let paths = io::source_index(root)?;
    for file in paths
        .as_object()
        .ok_or("diagnostic source index")?
        .keys()
        .filter(|file| file.starts_with("crates/laneflow-runtime/src/") && file.ends_with(".rs"))
    {
        let path = root.join(file);
        let original = fs::read_to_string(&path)?.replace("\r\n", "\n");
        let mut text = promote_guards(&original);
        // 成功 profile 查询在所有 Runtime 调用点统计，失败查询不计入该口径。
        let mut cursor = 0;
        while let Some(offset) = text[cursor..].find(".vehicle_profile(") {
            let begin = cursor + offset + ".vehicle_profile(".len();
            let mut depth = 1;
            let mut end = begin;
            for (offset, byte) in text[begin..].bytes().enumerate() {
                if byte == b'(' {
                    depth += 1;
                }
                if byte == b')' {
                    depth -= 1;
                }
                if depth == 0 {
                    end = begin + offset + 1;
                    break;
                }
            }
            need(end > begin, "profile call closing delimiter")?;
            let counter = ".inspect(|_| crate::kernel::tick::note_columnar_work(18, 1))";
            text.insert_str(end, counter);
            cursor = end + counter.len();
        }
        if text != original {
            fs::write(path, text)?;
        }
    }
    let tick = format!("{K}tick.rs");
    if candidate {
        let path = root.join(&tick);
        let mut text = fs::read_to_string(&path)?;
        let start = text
            .find("static COLUMNAR_WORK:")
            .ok_or("work counter start")?;
        let end = text
            .find("use laneflow_static_contract::")
            .ok_or("work counter end")?;
        need(start < end, "work counter range")?;
        text.replace_range(start..end, include_str!("columnar_pipeline_probe.rs"));
        fs::write(path, text)?;
        let motion = format!("{K}tick/columnar_motion.rs");
        patch(
            root,
            &motion,
            "motion_tls_snapshot, note_columnar_work, note_motion_cache_use,",
            "motion_tls_snapshot, note_motion_cache_use,",
        )?;
        let path = root.join(&motion);
        let mut text = fs::read_to_string(&path)?;
        if text.contains(".run_direct_with_stats(") {
            // 正常 release 没有逐行统计；只有冻结诊断导出启用同公式的统计实例。
            let guarded = "        #[cfg(test)]\n        let stats = kernel\n            .run_direct_with_stats";
            need(
                text.matches(guarded).count() == 1,
                "direct diagnostic stats anchor",
            )?;
            text = text.replace(
                guarded,
                "        let stats = kernel\n            .run_direct_with_stats",
            );
            let plain = "        #[cfg(not(test))]\n        let stats = {\n            kernel\n                .run_direct(&input, &mut output, delta_s)\n                .expect(\"physical motion columns have identical ranges\");\n            laneflow_motion_kernel::Stats::default()\n        };\n";
            need(
                text.matches(plain).count() == 1,
                "direct ordinary stats anchor",
            )?;
            text = text.replace(plain, "");
        }
        text = text.replace("    #[cfg(test)]\n    {\n        if matches!(phase, NumericPhase::Fused | NumericPhase::Proposal)",
            "    {\n        if matches!(phase, NumericPhase::Fused | NumericPhase::Proposal)");
        text.push_str("\nuse super::note_columnar_work;\npub(super) fn diagnostic_batch_bytes() -> usize { std::mem::size_of::<Batch>() }\n");
        fs::write(path, text)?;
        patch(
            root,
            &motion,
            "    let work = chunks(updates, &read.committed.vehicles, extent, rows);",
            "    let _dispatch_timer = super::super::performance_profile::begin(super::super::performance_profile::Stage::P5Dispatch);\n    let work = chunks(updates, &read.committed.vehicles, extent, rows);",
        )?;
        patch(
            root,
            &motion,
            "    // 规范首错及每辆车的真实到达 reserve 交错保留，物理块和 ISA 都不改变消费顺序。",
            "    drop(_dispatch_timer);\n    let _consume_timer = super::super::performance_profile::begin(super::super::performance_profile::Stage::P5Consume);\n    // 规范首错及每辆车的真实到达 reserve 交错保留，物理块和 ISA 都不改变消费顺序。",
        )?;
    } else {
        let path = root.join(&tick);
        let mut text = fs::read_to_string(&path)?;
        text.push('\n');
        text.push_str(include_str!("columnar_pipeline_probe.rs"));
        fs::write(path, text)?;
        function_counter(root, &tick, "fn iidm_step(", 1)?;
        patch(
            root,
            &tick,
            "    let dispatch_stats =\n        execution.try_for_each_chunk(view.read, slots, &first_error, chunk_size, compute);",
            "    let _dispatch_timer = super::performance_profile::begin(super::performance_profile::Stage::P5Dispatch);\n    let dispatch_stats =\n        execution.try_for_each_chunk(view.read, slots, &first_error, chunk_size, compute);\n    drop(_dispatch_timer);\n    let _consume_timer = super::performance_profile::begin(super::performance_profile::Stage::P5Consume);",
        )?;
    }
    function_counter(root, &tick, "fn si_comfort_travel(", 16)?;
    function_counter(
        root,
        &format!("{K}tables.rs"),
        "fn compiled_route_for_handle(",
        17,
    )?;
    function_counter(root, &format!("{K}occupancy.rs"), "fn leader_gap(", 19)?;
    let waiting = format!("{K}waiting.rs");
    patch(
        root,
        &waiting,
        "        match execution {\n            Some(resources @ crate::kernel::execution::ExecutionResources::Pool(_))",
        "        let _preview_timer = super::performance_profile::begin(super::performance_profile::Stage::WaitingPreview);\n        match execution {\n            Some(resources @ crate::kernel::execution::ExecutionResources::Pool(_))",
    )?;
    patch(
        root,
        &waiting,
        "        #[cfg(test)]\n        let _assembly = preview_stage::begin(preview_stage::ASSEMBLY);",
        "        drop(_preview_timer);\n        #[cfg(test)]\n        let _assembly = preview_stage::begin(preview_stage::ASSEMBLY);",
    )?;
    for file in [
        format!("{K}performance_profile.rs"),
        "crates/laneflow-runtime/src/lib.rs".to_owned(),
    ] {
        let path = root.join(file);
        let text = fs::read_to_string(&path)?.replace("; 12]", "; 16]");
        fs::write(path, text)?;
    }
    patch(
        root,
        &format!("{K}performance_profile.rs"),
        "    P4,",
        "    P4,\n    P5Dispatch,\n    P5Consume,\n    WaitingPreview,\n    Reserved,",
    )?;
    let path = root.join(&tick);
    let mut text = fs::read_to_string(&path)?;
    text.push_str(if candidate {
        "\npub(crate) fn diagnostic_layout_bytes() -> [usize; 3] { [columnar_motion::diagnostic_batch_bytes(), std::mem::size_of::<MotionBasis>(), std::mem::size_of::<MotionCacheEntry>()] }\n"
    } else { "\npub(crate) fn diagnostic_layout_bytes() -> [usize; 3] { [0, 0, std::mem::size_of::<MotionCacheEntry>()] }\n" });
    fs::write(path, text)?;
    let lib = root.join("crates/laneflow-runtime/src/lib.rs");
    let mut text = fs::read_to_string(&lib)?;
    text.push_str("\n#[doc(hidden)] pub fn research_work() -> [u64; 20] { kernel::tick::take_columnar_work() }\n#[doc(hidden)] pub fn research_layout() -> [usize; 3] { kernel::tick::diagnostic_layout_bytes() }\n");
    fs::write(lib, text)?;
    patch(
        root,
        "tools/laneflow-urban-harness/src/host.rs",
        "                laneflow_runtime::research_reset();",
        "                laneflow_runtime::research_reset();\n                laneflow_runtime::research_work();",
    )?;
    patch(
        root,
        "tools/laneflow-urban-harness/src/host.rs",
        "                let (stages, calls) = laneflow_runtime::research_take();",
        "                let (stages, calls) = laneflow_runtime::research_take();\n                eprintln!(\"LF814 {{\\\"tick\\\":{},\\\"work\\\":{:?},\\\"layout\\\":{:?}}}\", world.tick_index(), laneflow_runtime::research_work(), laneflow_runtime::research_layout());",
    )?;
    crate::columnar_cpu_export::instrument(root, candidate)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_candidate_export_checks_every_pipeline_anchor() {
        let repo = std::path::PathBuf::from(
            io::git(
                &std::env::current_dir().unwrap(),
                &["rev-parse", "--show-toplevel"],
            )
            .unwrap(),
        );
        let head = io::git(&repo, &["rev-parse", "HEAD"]).unwrap();
        let root = repo
            .join("target")
            .join(format!("814-anchor-test-{}", uuid::Uuid::new_v4()));
        crate::prepare::export_at(&repo, &root, "stages", &head).unwrap();
        instrument(&root, true).unwrap();
        let tick = fs::read_to_string(root.join(format!("{K}tick.rs"))).unwrap();
        assert!(tick.contains("PIPELINE_LOCAL"));
        assert!(!tick.contains("COLUMNAR_WORK_ENABLED"));
        let host =
            fs::read_to_string(root.join("tools/laneflow-urban-harness/src/host.rs")).unwrap();
        assert_eq!(host.matches("LF814_PIPELINE").count(), 1);
        assert!(!host.contains("let (elapsed,"));
    }
    #[test]
    fn promotion_excludes_test_modules_and_unrelated_probes() {
        let text = "#[cfg(test)]\nmod tests {\n fn retained_memory() {}\n}\n#[cfg(test)]\nfn unrelated_probe() {}\n#[cfg(test)]\npub(crate) fn retained_columns_bytes() {}\n#[cfg(test)]\nnote_columnar_work(1, 1);\n";
        let output = promote_guards(text);
        assert!(output.contains("#[cfg(test)]\nmod tests"));
        assert!(output.contains("#[cfg(test)]\nfn unrelated_probe"));
        assert!(output.contains("#[cfg(test)]\npub(crate) fn retained_columns"));
        assert!(!output.contains("#[cfg(test)]\nnote_columnar_work"));
    }
}
