//! 冻结队列的真实发射、完整控制器闭包与全局候选赋值验证。

use super::*;
use std::collections::BTreeSet;

fn normalized(value: &Value) -> Value {
    serde_json::from_str(&value.to_string()).unwrap()
}

pub(super) fn closure_audit(traffic: &TrafficPackage, path: &ManeuverPath) -> Value {
    let part = clip(traffic, path);
    let controller_ids = part
        .signals
        .controllers
        .iter()
        .map(|c| &c.id)
        .collect::<HashSet<_>>();
    let expected = traffic
        .signals
        .controllers
        .iter()
        .filter(|c| controller_ids.contains(&c.id))
        .collect::<Vec<_>>();
    assert_eq!(json!(expected), json!(part.signals.controllers));
    json!({"dependency_paths":part.maneuver_paths.iter().map(|p|&p.id).collect::<BTreeSet<_>>(),
        "dependency_edges":part.lane_graph.edges.iter().map(|e|&e.id).collect::<BTreeSet<_>>(),
        "complete_original_controllers_and_phases":true,"signals":part.signals})
}

#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, frozen S1_COHORT_FILE, S1_SWEEP_DIR and fresh S1_OUTPUT_DIR"]
fn pinned_lust_recovery_validation() {
    let out = recovery::fresh_output();
    let spec: Value = serde_json::from_slice(
        &fs::read(std::env::var("S1_COHORT_FILE").expect("S1_COHORT_FILE")).unwrap(),
    )
    .unwrap();
    assert_eq!(spec["schema"], "laneflow-1103-real-path-cohort");
    assert_eq!(spec["validation_tiers"], json!([32_768, 262_144]));
    assert_eq!(spec["limits"]["assignments"], ASSIGNMENT_LIMIT);
    let (observation, source, mut timing) = capture_lust();
    assert_eq!(source, spec["source"]);
    let traffic = observation.traffic.as_ref().unwrap();
    let programs = original_programs(&observation);
    let replay_start = Instant::now();
    for ((id, original), input) in observation.edges.iter().zip(&observation.inputs) {
        if let observe::EdgeEmission::Emitted(program) = original {
            let replay = geom::repair_curve(
                &input.points,
                input.clamps[0],
                input.clamps[1],
                input.clamp_sources[0].as_ref(),
                input.clamp_sources[1].as_ref(),
            )
            .unwrap();
            assert_eq!(
                program_bits(program),
                program_bits(&replay.program),
                "original f64 program: {id}"
            );
            let bits = |p| {
                program_points(p)
                    .into_iter()
                    .map(|v| v.map(|n| (n as f32).to_bits()))
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                bits(program),
                bits(&replay.program),
                "original canonical points: {id}"
            );
        }
    }
    assert_eq!(programs.len(), 16_265);
    timing["all_success_replay_compare_seconds"] = json!(replay_start.elapsed().as_secs_f64());
    let original_samples: Value = serde_json::from_str(include_str!(
        "../../../../../../../research/issue-1093-s1-efficiency/inputs.json"
    ))
    .unwrap();
    let original_ids = original_samples
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<HashSet<_>>();
    let mut target_ids: BTreeSet<String> =
        serde_json::from_value(spec["validation_paths"].clone()).unwrap();
    let old_paths = traffic
        .maneuver_paths
        .iter()
        .filter(|p| path_edges(p).iter().any(|e| original_ids.contains(*e)))
        .collect::<Vec<_>>();
    assert_eq!(old_paths.len(), 34);
    target_ids.extend(old_paths.iter().map(|p| p.id.clone()));
    let paths = target_ids
        .iter()
        .map(|id| traffic.maneuver_paths.iter().find(|p| &p.id == id).unwrap())
        .collect::<Vec<_>>();
    let audit = paths
        .iter()
        .map(|p| (&p.id, closure_audit(traffic, p)))
        .collect::<BTreeMap<_, _>>();
    for selected in spec["targets"].as_array().unwrap() {
        let row = &audit[&selected["path"].as_str().unwrap().to_owned()];
        assert_eq!(row["dependency_paths"], selected["dependency_paths"]);
        assert_eq!(row["dependency_edges"], selected["dependency_edges"]);
    }
    let baseline = paths.iter().map(|p|json!({"path":p.id,"edges":path_edges(p),"result":compile_path(traffic,p,&programs)})).collect::<Vec<_>>();
    let old_baseline: Value = serde_json::from_str(include_str!(
        "../../../../../../../research/issue-1093-s1-efficiency/results.json"
    ))
    .unwrap();
    for old in old_baseline["baseline_paths"].as_array().unwrap() {
        let current = baseline.iter().find(|p| p["path"] == old["path"]).unwrap();
        assert_eq!(
            normalized(current),
            *old,
            "original path baseline unchanged"
        );
    }
    let mut search_ids: BTreeSet<String> =
        serde_json::from_value(spec["recovery_edges"].clone()).unwrap();
    assert!(search_ids.len() <= 64);
    search_ids.extend(
        original_ids
            .iter()
            .filter(|id| !programs.contains_key(*id))
            .cloned(),
    );
    let input_ids = search_ids
        .iter()
        .chain(original_ids.iter())
        .collect::<BTreeSet<_>>();
    measurement::write_inputs(
        &out,
        &observation,
        &json!({"samples":input_ids.iter().map(|id|json!({"edge":id})).collect::<Vec<_>>()}),
    );
    let indexed = observation
        .edges
        .iter()
        .zip(&observation.inputs)
        .map(|((id, e), input)| (id, (e, input)))
        .collect::<BTreeMap<_, _>>();
    let mut tiers = BTreeMap::new();
    let mut tier_times = BTreeMap::new();
    let sweep = PathBuf::from(std::env::var("S1_SWEEP_DIR").expect("S1_SWEEP_DIR"));
    for (partitions, reference_dir) in [(32_768, "00-warmup-32768"), (262_144, "03-warmup-262144")]
    {
        let reference: Value = serde_json::from_slice(
            &fs::read(sweep.join(reference_dir).join("results.json")).unwrap(),
        )
        .unwrap();
        let mut pools = BTreeMap::new();
        let mut rows = BTreeMap::new();
        let mut times = BTreeMap::new();
        for id in &search_ids {
            let (original, input) = indexed[id];
            let observe::EdgeEmission::Failed(error) = original else {
                panic!("only original failures searched")
            };
            let original_control =
                original_ids.contains(id) && !recovery::TRUNCATED_EDGES.contains(&id.as_str());
            let budget = sampler::SearchBudget {
                partitions: if original_control {
                    sampler::PARTITION_LIMIT
                } else {
                    partitions
                },
            };
            let started = Instant::now();
            let search = sampler::search(input, budget);
            times.insert(id.clone(), started.elapsed().as_secs_f64());
            let candidates = recovery::candidates(&search);
            if original_ids.contains(id) {
                assert_eq!(json!(search.report), reference["samples"][id]["search"]);
                assert_eq!(
                    normalized(&candidates),
                    reference["samples"][id]["candidates"]
                );
            }
            rows.insert(id.clone(),json!({"original_error":error,"partition_limit":budget.partitions,"search":search.report,"candidates":candidates}));
            if !search.candidates.is_empty() {
                pools.insert(id.clone(), search.candidates);
            }
        }
        let compile_started = Instant::now();
        let assignments = assignments::evaluate(traffic, &paths, &programs, &pools);
        tier_times.insert(partitions,json!({"edge_search_seconds":times,"assignment_seconds":compile_started.elapsed().as_secs_f64()}));
        tiers.insert(
            partitions,
            json!({"policy":sampler::policy(sampler::SearchBudget{partitions}),"searches":rows,"assignments":assignments}),
        );
    }
    timing["tiers"] = json!(tier_times);
    recovery::write_json(
        &out,
        "results.json",
        &json!({"schema":"laneflow-1103-real-path-validation","source":source,
        "successful_f64_programs_and_f32_points_replayed_equal":programs.len(),"cohort":spec,
        "original_affected_paths":34,"target_paths":paths.len(),"closure_audit":audit,
        "baseline_paths":baseline,"tiers":tiers,"g1_accepted":false}),
    );
    recovery::write_json(&out, "timing.json", &timing);
    println!(
        "S1 real-path validation targets={} saved={}",
        paths.len(),
        out.display()
    );
}
