use crate::{BASE, EXPERIMENT, Result, cache_research::prepare, chunk_build, hot_export, io, need};
use serde_json::json;
use std::{fs, ops::Range, path::Path};

const TICK: &str = "crates/laneflow-runtime/src/kernel/tick.rs";
const MOTION: &str =
    "    #[allow(clippy::too_many_arguments)]\n    fn calculate_active_vehicle_motion(";
const MOTION_END: &str = "\n    /// 按同一拍初 binding";
const SI: &str = "fn si_comfort_travel(";
const SI_SPLIT: &str = "fn si_comfort_travel_precomputed(";
const SI_END: &str = "\nfn clamp_si_travel(";
const IIDM: &str = "fn iidm_travel(";
const IIDM_END: &str = "#[allow(clippy::too_many_arguments)]\nfn iidm_step(";

fn region(text: &str, start: &str, end: &str) -> Result<Range<usize>> {
    need(text.matches(start).count() == 1, "split start anchor")?;
    let begin = text.find(start).ok_or("split start")?;
    let finish = text[begin..].find(end).ok_or("split end")? + begin;
    Ok(begin..finish)
}

pub(crate) fn patch_tick(text: &mut String, arm: &str) -> Result<()> {
    need(arm == "candidate", "split patch arm")?;
    let mut hot = text.clone();
    hot_export::patch_tick(&mut hot, "layout")?;
    // #810：只移植公共原语，原 P5 循环、复用、规范消费和 join 逐字保持。
    for (old, new, end) in [(MOTION, MOTION, MOTION_END), (SI, SI_SPLIT, SI_END)] {
        let source = region(&hot, new, end)?;
        let target = region(text, old, end)?;
        text.replace_range(target, &hot[source]);
    }
    let old_iidm = region(text, IIDM, IIDM_END)?;
    text.replace_range(old_iidm, "");
    text.push_str(include_str!("../split_state.rs"));
    Ok(())
}

pub(crate) fn export(root: &Path, arm: &str) -> Result<()> {
    need(["base", "candidate"].contains(&arm), "split arm")?;
    let repo = std::env::current_dir()?;
    io::git(&repo, &["merge-base", "--is-ancestor", BASE, "HEAD"])?;
    fs::create_dir_all(root)?;
    chunk_build::ensure_outputs(root, arm, "plain")?;
    let source = root.join(format!("{arm}-plain-source"));
    let mut index = prepare::export_at(&repo, &source, "plain", BASE)?;
    if arm == "candidate" {
        let mut text = fs::read_to_string(source.join(TICK))?.replace("\r\n", "\n");
        patch_tick(&mut text, arm)?;
        fs::write(source.join(TICK), text)?;
    }
    index["arm"] = json!(arm);
    index["mode"] = json!("plain");
    index["protocol"] = json!(EXPERIMENT.protocol);
    index["batch_lanes"] = json!(1);
    index["motion_split"] = json!(arm == "candidate");
    index["source_files"] = io::source_index(&source)?;
    index["build"] = chunk_build::build(root, &source, &index)?;
    io::write_new(&root.join(format!("{arm}-plain-source.json")), &index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_arm_and_missing_anchors() {
        assert!(patch_tick(&mut String::new(), "candidate").is_err());
        assert!(patch_tick(&mut String::new(), "layout").is_err());
    }

    #[test]
    fn changes_only_common_primitive_and_matches_hot_split() {
        let original = include_str!("../../../crates/laneflow-runtime/src/kernel/tick.rs")
            .replace("\r\n", "\n");
        let mut split = original.clone();
        patch_tick(&mut split, "candidate").unwrap();
        let mut hot = original.clone();
        hot_export::patch_tick(&mut hot, "layout").unwrap();
        for (start, end) in [(MOTION, MOTION_END), (SI_SPLIT, SI_END)] {
            assert_eq!(
                split[region(&split, start, end).unwrap()],
                hot[region(&hot, start, end).unwrap()]
            );
        }
        assert!(!split.contains("IidmBatch"));
        assert!(!split.contains("wide::"));
        assert!(!split.contains("prepare_vehicle_motion"));
        assert!(!split.contains("compute_motion_batch"));
        split.truncate(split.len() - include_str!("../split_state.rs").len());
        for (old, new, end) in [(MOTION, MOTION, MOTION_END), (SI, SI_SPLIT, SI_END)] {
            let range = region(&split, new, end).unwrap();
            split.replace_range(range, &original[region(&original, old, end).unwrap()]);
        }
        let pos = split.find(IIDM_END).unwrap();
        split.insert_str(pos, &original[region(&original, IIDM, IIDM_END).unwrap()]);
        assert_eq!(split, original);
    }
}
