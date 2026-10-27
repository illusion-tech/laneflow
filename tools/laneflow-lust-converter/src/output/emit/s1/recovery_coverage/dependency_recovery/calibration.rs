//! #1111：固定四条真实边的诊断与校准；完整闭包由独立实验入口验证。

use super::*;

const EDGES: [&str; 4] = [
    "sumo::-10004_5_1",
    "sumo::-7348_3_0",
    "sumo::-16502_0_0",
    "sumo::-28860_3_0",
];

#[test]
fn reached_chord_reuse_preserves_calibrated_positive_candidates() {
    let inputs: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/inputs.json"
    ))
    .unwrap();
    let reference: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1111-grid-calibration/positive-calibration.json"
    ))
    .unwrap();
    for &id in &EDGES[..2] {
        let search = sampler::grid::search(&measurement::input(&inputs[id]));
        let candidates: Value =
            serde_json::from_str(&recovery::candidates(&search).to_string()).unwrap();
        assert_eq!(
            candidates, reference["edges"][id]["candidates"],
            "same BFS representatives"
        );
        assert_eq!(search.candidates.len(), 8);
        assert!(
            search.report["counts"]["previously_reached_chords_skipped"]
                .as_u64()
                .unwrap()
                > 0
        );
    }
}

#[test]
#[ignore = "requires fresh GRID_CALIBRATION_OUTPUT_DIR"]
fn fixed_grid_calibration() {
    let out = PathBuf::from(std::env::var("GRID_CALIBRATION_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh diagnostic output");
    fs::create_dir_all(&out).unwrap();
    let inputs: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/inputs.json"
    ))
    .unwrap();
    let mut rows = BTreeMap::new();
    let mut times = BTreeMap::new();
    for id in EDGES {
        let input = measurement::input(&inputs[id]);
        let started = Instant::now();
        let search = sampler::grid::search_audited(&input);
        times.insert(id, started.elapsed().as_secs_f64());
        rows.insert(
            id,
            json!({"search":search.report,"candidates":recovery::candidates(&search)}),
        );
    }
    recovery::write_json(
        &out,
        "diagnosis.json",
        &json!({"edges":rows,"scope":"first chord constraint and reached-chord reuse"}),
    );
    recovery::write_json(&out, "timing.json", &json!(times));
    println!("grid diagnosis saved={}", out.display());
}
