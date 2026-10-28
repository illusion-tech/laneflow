//! #1111：固定四条真实边的诊断与校准；完整闭包由独立实验入口验证。

use super::*;

mod experiment;

#[test]
#[ignore = "requires fresh GRID_CALIBRATION_OUTPUT_DIR"]
fn fixed_signal_guided_recovery() {
    let out = PathBuf::from(std::env::var("GRID_CALIBRATION_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh guided output");
    fs::create_dir_all(&out).unwrap();
    let inputs: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/inputs.json"
    ))
    .unwrap();
    let endpoints: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1113-closure-feasibility/endpoint-feasibility.json"
    ))
    .unwrap();
    let id = "sumo::-16502_0_0";
    let control = serde_json::from_value(endpoints[id]["control_f64_bits"].clone()).unwrap();
    let tail: [u64; 3] =
        serde_json::from_value(endpoints[id]["witness"]["parameters_f64_bits"].clone()).unwrap();
    let started = Instant::now();
    let search = sampler::grid::search_guided(
        &measurement::input(&inputs[id]),
        sampler::grid::Guide {
            control,
            tail: tail.map(f64::from_bits),
        },
    );
    let seconds = started.elapsed().as_secs_f64();
    recovery::write_json(
        &out,
        "guided-search.json",
        &json!({id:{"search":search.report,"candidates":recovery::candidates(&search)}}),
    );
    recovery::write_json(&out, "timing.json", &json!({id:seconds}));
}

#[test]
#[ignore = "requires fresh GRID_CALIBRATION_OUTPUT_DIR"]
fn fixed_signal_endpoint_feasibility() {
    let out = PathBuf::from(std::env::var("GRID_CALIBRATION_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh endpoint diagnostic output");
    fs::create_dir_all(&out).unwrap();
    let old: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1111-grid-calibration/local-probes.json"
    ))
    .unwrap();
    let mut rows = BTreeMap::new();
    let mut timing = BTreeMap::new();
    for id in ["sumo::-28860_3_0", "sumo::-16502_0_0"] {
        let bits: [[u64; 3]; 6] =
            serde_json::from_value(old[id]["control_f64_bits"].clone()).unwrap();
        let clamp: Option<[u64; 3]> =
            serde_json::from_value(old[id]["start_clamp"].clone()).unwrap();
        let started = Instant::now();
        rows.insert(
            id,
            sampler::feasibility::diagnose(
                bits.map(|p| p.map(f64::from_bits)),
                clamp.map(|d| d.map(f64::from_bits)),
            ),
        );
        timing.insert(id, started.elapsed().as_secs_f64());
    }
    recovery::write_json(&out, "endpoint-feasibility.json", &json!(rows));
    recovery::write_json(&out, "timing.json", &json!(timing));
}

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

#[test]
#[ignore = "requires fresh GRID_CALIBRATION_OUTPUT_DIR"]
fn fixed_signal_span_probe() {
    let out = PathBuf::from(std::env::var("GRID_CALIBRATION_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh probe output");
    fs::create_dir_all(&out).unwrap();
    recovery::write_json(&out, "local-probes.json", &signal_probes());
    println!("signal span probes saved={}", out.display());
}

fn signal_probes() -> Value {
    let inputs: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/inputs.json"
    ))
    .unwrap();
    let old: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/grid-searches.json"
    ))
    .unwrap();
    let mut rows = BTreeMap::new();
    for (id, index) in [(EDGES[2], "1"), (EDGES[3], "0")] {
        let bits: [[u64; 3]; 6] = serde_json::from_value(
            old[id]["search"]["failures"][index]["control_f64_bits"].clone(),
        )
        .unwrap();
        let clamp = (index == "0")
            .then(|| measurement::input(&inputs[id]).clamps[0])
            .flatten();
        rows.insert(
            id,
            sampler::grid::local_probe(bits.map(|p| p.map(f64::from_bits)), clamp),
        );
    }
    json!(rows)
}
