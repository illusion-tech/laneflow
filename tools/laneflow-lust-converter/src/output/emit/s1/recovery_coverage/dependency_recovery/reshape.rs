//! #1115：有限几何候选的完整依赖分量验证，保留原成功几何及控制锚点。

use super::*;
use experiment::{merge, pool};

const EDGES: [&str; 2] = ["sumo::-16502_0_0", "sumo::-28860_3_0"];

fn normalized(value: &Value) -> Value {
    serde_json::from_str(&value.to_string()).unwrap()
}

fn frozen_inputs() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/inputs.json"
    ))
    .unwrap()
}

fn run_searches(inputs: &Value) -> (Value, Value) {
    let mut searches = BTreeMap::new();
    let mut timing = BTreeMap::new();
    for id in EDGES {
        let input = measurement::input(&inputs[id]);
        let started = Instant::now();
        let search = sampler::reshape::search(&input);
        timing.insert(id, started.elapsed().as_secs_f64());
        searches.insert(
            id,
            json!({"search":search.report,"candidates":recovery::candidates(&search)}),
        );
    }
    (json!(searches), json!(timing))
}

#[test]
#[ignore = "requires fresh CURVE_DESIGN_OUTPUT_DIR"]
fn fixed_failed_curve_design() {
    let out = PathBuf::from(std::env::var("CURVE_DESIGN_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh curve design output");
    fs::create_dir_all(&out).unwrap();
    let (searches, timing) = run_searches(&frozen_inputs());
    recovery::write_json(&out, "searches.json", &searches);
    recovery::write_json(&out, "timing.json", &timing);
}

#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, DIRECT_SUCCESSOR_BASELINE_DIR, CURVE_DESIGN_FROZEN_RESULT, CURVE_DESIGN_ORDER and fresh CURVE_DESIGN_OUTPUT_DIR"]
fn pinned_lust_failed_curve_experiment() {
    let out = PathBuf::from(std::env::var("CURVE_DESIGN_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh full curve design output");
    fs::create_dir_all(&out).unwrap();
    let order = std::env::var("CURVE_DESIGN_ORDER").unwrap();
    assert!(matches!(order.as_str(), "baseline-new" | "new-baseline"));
    let frozen: Value = serde_json::from_slice(
        &fs::read(std::env::var("CURVE_DESIGN_FROZEN_RESULT").unwrap()).unwrap(),
    )
    .unwrap();
    let prior: Value = serde_json::from_str(include_str!("../../../../../../../../research/issue-1113-closure-feasibility/full-regression-results.json")).unwrap();
    let inventory: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/results.json"
    ))
    .unwrap();
    let (observation, source, mut timing) = capture_lust();
    assert_eq!(source, prior["source"]);
    let originals = original_programs(&observation);
    snapshot(&out, &observation, &originals);
    let traffic = observation.traffic.as_ref().unwrap();
    let components = components::inventory(traffic, &originals, &target_ids(&inventory));
    assert_eq!(json!(components), inventory["inventory"]["components"]);
    measurement::write_inputs(
        &out,
        &observation,
        &json!({"samples":EDGES.map(|id|json!({"edge":id}))}),
    );
    let inputs: Value =
        serde_json::from_slice(&fs::read(out.join("inputs.json")).unwrap()).unwrap();
    let old_inputs = frozen_inputs();
    for id in EDGES {
        assert_eq!(
            inputs[id], old_inputs[id],
            "unchanged source and emission failure"
        );
        assert!(!originals.contains_key(id), "only failed edges redesigned");
    }
    let (searches, search_times) = run_searches(&inputs);
    assert_eq!(normalized(&searches), frozen, "frozen candidate replay");
    recovery::write_json(&out, "searches.json", &searches);
    let legacy: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1103-s1-recovery-budget/results.json"
    ))
    .unwrap();
    let old: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/old-searches.json"
    ))
    .unwrap();
    let grid: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1111-grid-calibration/grid-searches.json"
    ))
    .unwrap();
    let mut baseline = pool(&legacy["tiers"]["32768"]["searches"]);
    merge(&mut baseline, pool(&old));
    merge(&mut baseline, pool(&grid));
    let mut combined = baseline.clone();
    merge(&mut combined, pool(&searches));
    let mut results = BTreeMap::new();
    let mut validation_times = BTreeMap::new();
    for family in order.split('-') {
        let pools = if family == "baseline" {
            &baseline
        } else {
            &combined
        };
        let started = Instant::now();
        let mut programs = originals.clone();
        for (id, choice) in reference()["tiers"]["32768"]["best"]["assignment"]
            .as_object()
            .unwrap()
        {
            programs.insert(
                id.clone(),
                pools[id][choice.as_u64().unwrap() as usize].clone(),
            );
        }
        let mut rows = Vec::new();
        for (index, component) in components.iter().enumerate() {
            let part = combinations::part(traffic, component);
            if component
                .edges
                .iter()
                .any(|id| EDGES.contains(&id.as_str()))
            {
                assert!(
                    part.signals
                        .stop_lines
                        .iter()
                        .all(|s| originals.contains_key(&s.edge_id)),
                    "affected stop line geometry is original"
                );
            }
            let file = format!("{family}-component-{index:02}.lfca");
            let result = combinations::evaluate(
                traffic,
                component,
                &originals,
                pools,
                Some(&out.join(&file)),
            );
            if family == "baseline" {
                assert_eq!(
                    normalized(&result),
                    prior["variants"]["combined"]["components"][index]["result"],
                    "complete baseline component"
                );
            }
            if result["status"] == "accepted" {
                for (id, choice) in result["selected_assignment"].as_object().unwrap() {
                    programs.insert(
                        id.clone(),
                        pools[id][choice.as_u64().unwrap() as usize].clone(),
                    );
                }
            }
            rows.push(json!({"component":component.id,"result":result,
                "artifact":(result["status"]=="accepted").then_some(file)}));
        }
        let paths=inventory["baseline_paths"].as_array().unwrap().iter().map(|row| {
            let path=traffic.maneuver_paths.iter().find(|p|p.id==row["path"].as_str().unwrap()).unwrap();
            json!({"path":path.id,"edges":path_edges(path),"result":compile_path(traffic,path,&programs)})
        }).collect::<Vec<_>>();
        if family == "baseline" {
            assert_eq!(
                normalized(&json!(paths)),
                prior["variants"]["combined"]["paths"],
                "complete baseline paths"
            );
        }
        for (current, previous) in paths
            .iter()
            .zip(prior["variants"]["combined"]["paths"].as_array().unwrap())
        {
            assert_eq!(current["path"], previous["path"]);
            if previous["result"]["status"] == "accepted" {
                assert_eq!(
                    current["result"]["status"], "accepted",
                    "accepted target retained"
                );
            }
        }
        let mut counts = BTreeMap::<String, usize>::new();
        for row in &paths {
            *counts
                .entry(row["result"]["status"].as_str().unwrap().to_owned())
                .or_default() += 1;
        }
        results.insert(
            family,
            json!({"components":rows,"paths":paths,"path_counts":counts}),
        );
        validation_times.insert(family, started.elapsed().as_secs_f64());
    }
    timing["search_seconds"] = search_times;
    timing["validation_seconds"] = json!(validation_times);
    timing["order"] = json!(order);
    recovery::write_json(
        &out,
        "results.json",
        &json!({"schema":"laneflow-1115-failed-curve-design",
        "source":source,"policy":sampler::reshape::policy(),"variants":results,
        "original_geometry_controls_and_failures_equal":true,"production_g1_accepted":false}),
    );
    recovery::write_json(&out, "timing.json", &timing);
}
