//! #1111：固定四条真实边的诊断与校准；完整闭包由独立实验入口验证。

use super::*;

const EDGES: [&str; 4] = [
    "sumo::-10004_5_1",
    "sumo::-7348_3_0",
    "sumo::-16502_0_0",
    "sumo::-28860_3_0",
];

#[test]
#[ignore = "requires fresh GRID_CALIBRATION_OUTPUT_DIR"]
fn fixed_grid_zero_candidate_diagnosis() {
    let out = PathBuf::from(std::env::var("GRID_CALIBRATION_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh diagnostic output");
    fs::create_dir_all(&out).unwrap();
    let inputs: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/inputs.json"
    ))
    .unwrap();
    let reference: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/grid-searches.json"
    ))
    .unwrap();
    let mut rows = BTreeMap::new();
    let mut times = BTreeMap::new();
    for id in EDGES {
        let input = measurement::input(&inputs[id]);
        let started = Instant::now();
        let mut search = sampler::grid::search_audited(&input);
        times.insert(id, started.elapsed().as_secs_f64());
        let audit = search
            .report
            .as_object_mut()
            .unwrap()
            .remove("end_audit")
            .unwrap();
        let normalized: Value = serde_json::from_str(&search.report.to_string()).unwrap();
        assert_eq!(
            normalized, reference[id]["search"],
            "unchanged grid decisions"
        );
        assert!(search.candidates.is_empty());
        rows.insert(id, json!({"search":search.report,"end_audit":audit}));
    }
    recovery::write_json(
        &out,
        "diagnosis.json",
        &json!({"edges":rows,"old_grid_results_equal":true}),
    );
    recovery::write_json(&out, "timing.json", &json!(times));
    println!("grid diagnosis saved={}", out.display());
}
