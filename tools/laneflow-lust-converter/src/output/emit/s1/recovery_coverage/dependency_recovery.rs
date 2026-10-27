//! #1107：固定成功几何上的完整依赖分量、候选恢复及共享组合研究。

use super::*;
use std::collections::BTreeSet;
use std::path::Path;

mod combinations;
mod components;
mod experiment;

fn reference() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../../../research/issue-1105-direct-successors/results.json"
    ))
    .unwrap()
}

fn fresh_output() -> PathBuf {
    let out = PathBuf::from(std::env::var("DEPENDENCY_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh evidence output");
    fs::create_dir_all(&out).unwrap();
    out
}

fn target_ids(reference: &Value) -> BTreeSet<String> {
    let ids = reference["baseline_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["path"].as_str().unwrap().to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), 84);
    ids
}

fn snapshot(
    out: &Path,
    observation: &observe::EmissionObservation,
    originals: &BTreeMap<String, re::RoadEditingCurveProgram>,
) {
    assert_eq!(originals.len(), 16_265);
    successors::write_geometry(out, originals);
    recovery::write_json(out, "topology.json", &json!(observation.traffic));
    let failures = observation
        .edges
        .iter()
        .filter_map(|(id, result)| match result {
            observe::EdgeEmission::Failed(error) => Some((id, error)),
            observe::EdgeEmission::Emitted(_) => None,
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(failures.len(), 8_230);
    recovery::write_json(out, "failed-edges.json", &json!(failures));
    let baseline = PathBuf::from(std::env::var("DIRECT_SUCCESSOR_BASELINE_DIR").unwrap());
    for name in [
        "successful-programs.jsonl",
        "topology.json",
        "failed-edges.json",
    ] {
        successors::assert_same_file(&baseline.join(name), &out.join(name));
    }
}

#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, frozen DIRECT_SUCCESSOR_BASELINE_DIR and fresh DEPENDENCY_OUTPUT_DIR"]
fn pinned_lust_dependency_inventory() {
    let out = fresh_output();
    let reference = reference();
    let (observation, source, timing) = capture_lust();
    assert_eq!(source, reference["source"]);
    let originals = original_programs(&observation);
    snapshot(&out, &observation, &originals);
    let traffic = observation.traffic.as_ref().unwrap();
    let components = components::inventory(traffic, &originals, &target_ids(&reference));
    let selection = components::selection(&components);
    recovery::write_json(
        &out,
        "inventory.json",
        &json!({"schema":"laneflow-1107-dependency-inventory","source":source,
            "target_paths":84,"components":components,"selection":selection,
            "successful_programs_and_control_snapshot_equal":true,
            "production_g1_accepted":false}),
    );
    recovery::write_json(&out, "timing.json", &timing);
    println!("dependency inventory saved={}", out.display());
}
