//! #1111：冻结输入上的整边搜索、完整依赖分量与全部控制锚点回归。

use super::super::experiment::{merge, pool};
use super::*;

fn normalized(value: &Value) -> Value {
    serde_json::from_str(&value.to_string()).unwrap()
}

#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, DIRECT_SUCCESSOR_BASELINE_DIR, CALIBRATION_ORDER and fresh GRID_CALIBRATION_OUTPUT_DIR"]
fn pinned_lust_calibrated_grid_experiment() {
    let out = PathBuf::from(std::env::var("GRID_CALIBRATION_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh calibrated output");
    fs::create_dir_all(&out).unwrap();
    let order = std::env::var("CALIBRATION_ORDER").unwrap();
    assert!(matches!(order.as_str(), "old-grid" | "grid-old"));
    let prior: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/results.json"
    ))
    .unwrap();
    let old_rows: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/old-searches.json"
    ))
    .unwrap();
    let legacy: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1103-s1-recovery-budget/results.json"
    ))
    .unwrap();
    let (observation, source, mut timing) = capture_lust();
    assert_eq!(source, prior["source"]);
    let originals = original_programs(&observation);
    snapshot(&out, &observation, &originals);
    let traffic = observation.traffic.as_ref().unwrap();
    let components = components::inventory(traffic, &originals, &target_ids(&prior));
    assert_eq!(json!(components), prior["inventory"]["components"]);
    let samples = json!({"samples":EDGES.iter().map(|id|json!({"edge":id})).collect::<Vec<_>>()});
    measurement::write_inputs(&out, &observation, &samples);
    let mut searches = BTreeMap::new();
    let mut search_times = BTreeMap::new();
    for family in order.split('-') {
        let mut rows = BTreeMap::new();
        let mut times = BTreeMap::new();
        for id in EDGES {
            let index = observation
                .edges
                .iter()
                .position(|(edge, _)| edge == id)
                .unwrap();
            let input = &observation.inputs[index];
            let started = Instant::now();
            let search = if family == "old" {
                sampler::search(input, sampler::SearchBudget::default())
            } else {
                sampler::grid::search_audited(input)
            };
            times.insert(id, started.elapsed().as_secs_f64());
            let candidates = recovery::candidates(&search);
            if family == "old" {
                assert_eq!(
                    normalized(&search.report),
                    old_rows[id]["search"],
                    "unchanged original candidate family"
                );
                assert_eq!(
                    normalized(&candidates),
                    old_rows[id]["candidates"],
                    "unchanged original candidates"
                );
            }
            rows.insert(id, json!({"search":search.report,"candidates":candidates}));
        }
        let rows = json!(rows);
        recovery::write_json(&out, &format!("{family}-searches.json"), &rows);
        searches.insert(family, rows);
        search_times.insert(family, times);
    }
    let mut baseline = pool(&legacy["tiers"]["32768"]["searches"]);
    merge(&mut baseline, pool(&old_rows));
    assert_eq!(baseline.len(), 7);
    let grid = pool(&searches["grid"]);
    let mut replacement = baseline.clone();
    for (id, programs) in &grid {
        replacement.insert(id.clone(), programs.clone());
    }
    let mut combined = baseline.clone();
    merge(&mut combined, grid);
    let mut results = BTreeMap::new();
    let mut validation_times = BTreeMap::new();
    for (family, pools) in [
        ("baseline", baseline),
        ("grid_replacement", replacement),
        ("combined", combined),
    ] {
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
            let file = format!("{family}-component-{index:02}.lfca");
            let result = combinations::evaluate(
                traffic,
                component,
                &originals,
                &pools,
                Some(&out.join(&file)),
            );
            if result["status"] == "accepted" {
                for (id, choice) in result["selected_assignment"].as_object().unwrap() {
                    programs.insert(
                        id.clone(),
                        pools[id][choice.as_u64().unwrap() as usize].clone(),
                    );
                }
            }
            if family == "baseline" {
                assert_eq!(
                    normalized(&result),
                    prior["variants"]["combined"]["components"][index]["result"],
                    "unchanged complete component baseline"
                );
            }
            rows.push(json!({"component":component.id,"result":result,"artifact":(result["status"]=="accepted").then_some(file)}));
        }
        let paths=prior["baseline_paths"].as_array().unwrap().iter().map(|row| {
            let path=traffic.maneuver_paths.iter().find(|p|p.id==row["path"].as_str().unwrap()).unwrap();
            json!({"path":path.id,"edges":path_edges(path),"result":compile_path(traffic,path,&programs)})
        }).collect::<Vec<_>>();
        if family == "baseline" {
            assert_eq!(
                normalized(&json!(paths)),
                prior["variants"]["combined"]["paths"]
            );
        }
        if family == "combined" {
            for (current, previous) in paths
                .iter()
                .zip(prior["variants"]["combined"]["paths"].as_array().unwrap())
            {
                assert_eq!(current["path"], previous["path"]);
                if previous["result"]["status"] == "accepted" {
                    assert_eq!(
                        current["result"]["status"], "accepted",
                        "no accepted target regression"
                    );
                }
            }
        }
        let mut counts = BTreeMap::<String, usize>::new();
        for row in &paths {
            *counts
                .entry(row["result"]["status"].as_str().unwrap().to_owned())
                .or_default() += 1;
        }
        results.insert(family,json!({"components":rows,"paths":paths,"path_counts":counts,"pool_sizes":pools.iter().map(|(id,p)|(id,p.len())).collect::<BTreeMap<_,_>>()}));
        validation_times.insert(family, started.elapsed().as_secs_f64());
    }
    let started = Instant::now();
    recovery::write_json(&out, "local-probes.json", &signal_probes());
    timing["local_probe_seconds"] = json!(started.elapsed().as_secs_f64());
    timing["search_order"] = json!(order);
    timing["edge_search_seconds"] = json!(search_times);
    timing["validation_seconds"] = json!(validation_times);
    recovery::write_json(
        &out,
        "results.json",
        &json!({"schema":"laneflow-1111-grid-calibration","source":source,
        "edges":EDGES,"grid_policy":sampler::grid::policy(),"variants":results,
        "all_original_geometry_and_controls_equal":true,"production_g1_accepted":false}),
    );
    recovery::write_json(&out, "timing.json", &timing);
    println!("calibrated grid evidence saved={}", out.display());
}
