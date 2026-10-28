//! #1113：局部诊断与原完整基线的组合回归，不把缺少程序的控制器记作接受。

use super::*;

fn normalized(value: &Value) -> Value {
    serde_json::from_str(&value.to_string()).unwrap()
}

fn without_certificate(mut value: Value) -> Value {
    value
        .as_object_mut()
        .unwrap()
        .remove("exclusion_certificate");
    value
        .as_object_mut()
        .unwrap()
        .remove("certificate_verified");
    value["policy"]
        .as_object_mut()
        .unwrap()
        .remove("certificate_leaf_limit");
    value
}

fn diagnostics() -> (Value, Value, Value, Value) {
    let endpoints:Value=serde_json::from_str(include_str!("../../../../../../../../../research/issue-1113-closure-feasibility/endpoint-feasibility.json")).unwrap();
    let seam: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1113-closure-feasibility/seam-input.json"
    ))
    .unwrap();
    let old_seam: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1113-closure-feasibility/seam-feasibility.json"
    ))
    .unwrap();
    let inputs: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/inputs.json"
    ))
    .unwrap();
    let old_guided: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1113-closure-feasibility/guided-search.json"
    ))
    .unwrap();
    let mut probes = BTreeMap::new();
    let mut times = BTreeMap::new();
    for id in [EDGES[3], EDGES[2]] {
        let c: [[u64; 3]; 6] =
            serde_json::from_value(endpoints[id]["control_f64_bits"].clone()).unwrap();
        let clamp: Option<[u64; 3]> =
            serde_json::from_value(endpoints[id]["clamp_f64_bits"].clone()).unwrap();
        let start = Instant::now();
        let result = sampler::feasibility::diagnose(
            c.map(|p| p.map(f64::from_bits)),
            clamp.map(|d| d.map(f64::from_bits)),
        );
        times.insert(id, start.elapsed().as_secs_f64());
        assert_eq!(
            without_certificate(normalized(&result)),
            endpoints[id],
            "same frozen endpoint diagnosis"
        );
        probes.insert(id, result);
    }
    let left: [[u64; 3]; 6] =
        serde_json::from_value(endpoints[EDGES[2]]["control_f64_bits"].clone()).unwrap();
    let right: [[u64; 3]; 6] = serde_json::from_value(seam["control_f64_bits"].clone()).unwrap();
    let start = Instant::now();
    let seam = sampler::feasibility::diagnose_seam(
        std::array::from_fn(|i| left[i].map(f64::from_bits)),
        std::array::from_fn(|i| right[i].map(f64::from_bits)),
    );
    times.insert("fixed_seam", start.elapsed().as_secs_f64());
    assert_eq!(
        without_certificate(normalized(&seam)),
        old_seam,
        "same frozen seam diagnosis"
    );
    let tail: [u64; 3] =
        serde_json::from_value(endpoints[EDGES[2]]["witness"]["parameters_f64_bits"].clone())
            .unwrap();
    let start = Instant::now();
    let guided = sampler::grid::search_guided(
        &measurement::input(&inputs[EDGES[2]]),
        sampler::grid::Guide {
            control: left,
            tail: tail.map(f64::from_bits),
        },
    );
    times.insert("guided_search", start.elapsed().as_secs_f64());
    assert!(
        guided.candidates.is_empty(),
        "no complete program in frozen family"
    );
    let guided =
        json!({EDGES[2]:{"search":guided.report,"candidates":recovery::candidates(&guided)}});
    let mut compare = normalized(&guided);
    let prefix = compare[EDGES[2]]["search"]["failures"]["2"]
        .as_object_mut()
        .unwrap()
        .remove("accepted_prefix_spans")
        .unwrap();
    assert_eq!(
        prefix.as_array().unwrap().len(),
        2,
        "first two cubics emitted"
    );
    assert_eq!(
        prefix[1]["parameters_f64_bits"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .take(3)
            .cloned()
            .collect::<Vec<_>>(),
        tail.iter().rev().map(|&t| json!(t)).collect::<Vec<_>>()
    );
    assert_eq!(compare, old_guided, "same frozen guided search");
    (json!(probes), seam, guided, json!(times))
}

#[test]
fn frozen_curve_diagnosis_separates_grid_gap_from_fixed_constraints() {
    let (endpoints, seam, _, _) = diagnostics();
    assert_eq!(
        endpoints[EDGES[3]]["status"],
        "necessary_condition_excluded"
    );
    assert_eq!(endpoints[EDGES[3]]["certificate_verified"], true);
    assert_eq!(endpoints[EDGES[2]]["status"], "local_witness");
    assert_eq!(seam["status"], "necessary_condition_excluded");
    assert_eq!(seam["certificate_verified"], true);
}

#[test]
#[ignore = "requires pinned LuST, full baseline, CALIBRATION_ORDER, CLOSURE_ORDER and fresh GRID_CALIBRATION_OUTPUT_DIR"]
fn pinned_lust_closure_feasibility_experiment() {
    let out = PathBuf::from(std::env::var("GRID_CALIBRATION_OUTPUT_DIR").unwrap());
    let order = std::env::var("CLOSURE_ORDER").unwrap();
    assert!(matches!(
        order.as_str(),
        "diagnosis-first" | "baseline-first"
    ));
    let mut diagnostic = (order == "diagnosis-first").then(diagnostics);
    experiment::pinned_lust_calibrated_grid_experiment();
    if diagnostic.is_none() {
        diagnostic = Some(diagnostics());
    }
    let (endpoints, seam, guided, timing) = diagnostic.unwrap();
    let current: Value =
        serde_json::from_slice(&fs::read(out.join("results.json")).unwrap()).unwrap();
    let prior: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1111-grid-calibration/results.json"
    ))
    .unwrap();
    assert_eq!(
        current, prior,
        "complete prior path and component results unchanged"
    );
    let inputs: Value =
        serde_json::from_slice(&fs::read(out.join("inputs.json")).unwrap()).unwrap();
    let frozen: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1111-grid-calibration/inputs.json"
    ))
    .unwrap();
    assert_eq!(inputs, frozen, "fresh complete edge inputs unchanged");
    let closures = prior["variants"]["combined"]["components"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| {
            [EDGES[2], EDGES[3]]
                .iter()
                .any(|&id| r["result"]["missing_edges"] == json!([id]))
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(closures.len(), 2);
    for c in &closures {
        assert_eq!(c["result"]["status"], "missing_real_programs");
        assert!(c["result"]["trials"].as_array().unwrap().is_empty());
    }
    recovery::write_json(&out, "endpoint-certificates.json", &endpoints);
    recovery::write_json(&out, "seam-certificate.json", &seam);
    recovery::write_json(&out, "guided-search.json", &guided);
    recovery::write_json(
        &out,
        "closure-timing.json",
        &json!({"order":order,"seconds":timing}),
    );
    recovery::write_json(
        &out,
        "closure-summary.json",
        &json!({"schema":"laneflow-1113-closure-feasibility","source":prior["source"],
        "closures":closures,"path_counts":prior["variants"]["combined"]["path_counts"],"original_successful_edges":16_265,
        "original_geometry_controls_and_failures_equal":true,"prior_complete_results_equal":true,"production_g1_accepted":false,
        "fixed_curve_disposition":{"sumo::-28860_3_0":"first_chord_constraint_conflict","sumo::-16502_0_0":"local_grid_coverage_gap_and_fixed_cubic_seam_conflict"}}),
    );
}
