//! 所有路径在同一轮消费同一赋值；只缓存候选索引完全相同的路径闭包。

use super::*;

pub(super) fn evaluate(
    traffic: &TrafficPackage,
    paths: &[&ManeuverPath],
    originals: &BTreeMap<String, re::RoadEditingCurveProgram>,
    pools: &BTreeMap<String, Vec<Candidate>>,
) -> Value {
    let keys = pools.keys().cloned().collect::<Vec<_>>();
    assert!(keys.iter().all(|key| !originals.contains_key(key)));
    let dependencies = paths
        .iter()
        .map(|path| {
            let part = clip(traffic, path);
            let complete = part
                .lane_graph
                .edges
                .iter()
                .all(|e| originals.contains_key(&e.id) || pools.contains_key(&e.id));
            keys.iter()
                .enumerate()
                .filter_map(|(i, key)| {
                    (complete && part.lane_graph.edges.iter().any(|e| &e.id == key)).then_some(i)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut cache = vec![BTreeMap::<Vec<usize>, Value>::new(); paths.len()];
    let mut choices = vec![0; keys.len()];
    let mut programs = originals.clone();
    let mut trials = Vec::new();
    let mut best = Value::Null;
    let mut best_accepted = 0;
    let mut exhausted = false;
    let mut evaluations = 0;
    let mut hits = 0;
    for trial in 0..ASSIGNMENT_LIMIT {
        for (i, key) in keys.iter().enumerate() {
            programs.insert(key.clone(), pools[key][choices[i]].repaired.program.clone());
        }
        let assignment = keys.iter().zip(&choices).collect::<BTreeMap<_, _>>();
        let mut results = Vec::new();
        let mut counts = BTreeMap::<String, usize>::new();
        for (i, path) in paths.iter().enumerate() {
            let signature = dependencies[i]
                .iter()
                .map(|j| choices[*j])
                .collect::<Vec<_>>();
            let result = if let Some(result) = cache[i].get(&signature) {
                hits += 1;
                result.clone()
            } else {
                evaluations += 1;
                let result = compile_path(traffic, path, &programs);
                cache[i].insert(signature, result.clone());
                result
            };
            *counts
                .entry(result["status"].as_str().unwrap().to_owned())
                .or_default() += 1;
            results.push(json!({"path":path.id,"edges":path_edges(path),"result":result}));
        }
        let accepted = counts.get("accepted").copied().unwrap_or(0);
        trials.push(json!({"trial":trial,"assignment":assignment,"counts":counts}));
        if trial == 0 || accepted > best_accepted {
            best_accepted = accepted;
            best = json!({"assignment":assignment,"paths":results,"counts":counts});
        }
        let mut advanced = false;
        for i in (0..keys.len()).rev() {
            choices[i] += 1;
            if choices[i] < pools[&keys[i]].len() {
                advanced = true;
                break;
            }
            choices[i] = 0;
        }
        if !advanced {
            exhausted = true;
            break;
        }
    }
    for (key, program) in originals {
        assert_eq!(program_bits(program), program_bits(&programs[key]));
    }
    json!({"trials":trials,"best":best,"all_targets_accepted":best_accepted==paths.len(),
        "retained_pool_assignments_exhausted":exhausted,"unique_path_evaluations":evaluations,
        "exact_dependency_cache_hits":hits,"assignment_limit":ASSIGNMENT_LIMIT,
        "candidate_pool_sizes":pools.iter().map(|(key, pool)|(key,pool.len())).collect::<BTreeMap<_,_>>()})
}

#[test]
fn shared_assignment_cache_preserves_complete_path_failure() {
    let observation = small_observation();
    let traffic = observation.traffic.as_ref().unwrap();
    let mut programs = small_programs(&observation);
    let path = &traffic.maneuver_paths[0];
    programs.remove(&path.entry_edge_id);
    let result = evaluate(traffic, &[path], &programs, &BTreeMap::new());
    assert_eq!(result["trials"].as_array().unwrap().len(), 1);
    assert_eq!(result["unique_path_evaluations"], 1);
    assert_eq!(result["retained_pool_assignments_exhausted"], true);
    assert_eq!(result["all_targets_accepted"], false);
    assert_eq!(
        result["best"]["paths"][0]["result"]["stage"],
        "emission_prerequisite"
    );
}

#[test]
fn candidate_assignment_cache_uses_every_dependency_index() {
    let observation = small_observation();
    let traffic = observation.traffic.as_ref().unwrap();
    let mut programs = small_programs(&observation);
    let paths = traffic.maneuver_paths.iter().collect::<Vec<_>>();
    let key = paths[0].internal_edge_ids[0].clone();
    let original = programs.remove(&key).unwrap();
    let points = program_points(&original);
    let shifted = re::RoadEditingCurveProgram::try_new(
        fixture_point(fixture_translate(points[0], [0.0, 0.2, 0.0])),
        points[1..]
            .iter()
            .map(|p| {
                re::RoadEditingCurveSegment::line(fixture_point(fixture_translate(
                    *p,
                    [0.0, 0.2, 0.0],
                )))
            })
            .collect(),
    )
    .unwrap();
    let candidate = |program| Candidate {
        repaired: geom::RepairedCurve {
            program,
            quantized_arc_meters: 0.0,
        },
        spans: Vec::new(),
    };
    let pools = BTreeMap::from([
        (
            key.clone(),
            vec![candidate(shifted), candidate(original.clone())],
        ),
        (
            "zz-unused".to_owned(),
            vec![candidate(original.clone()), candidate(original)],
        ),
    ]);
    let result = evaluate(traffic, &paths, &programs, &pools);
    assert_eq!(result["trials"].as_array().unwrap().len(), 4);
    assert_eq!(result["unique_path_evaluations"], paths.len() * 2);
    assert_eq!(result["exact_dependency_cache_hits"], paths.len() * 2);
    assert_eq!(result["best"]["assignment"][&key], 1);
    assert_eq!(result["best"]["assignment"]["zz-unused"], 0);
    assert_eq!(result["all_targets_accepted"], true);
    assert_eq!(result["retained_pool_assignments_exhausted"], true);
    assert!(result["trials"][0]["counts"].get("accepted").is_none());
}
