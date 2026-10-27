use super::*;
use components::Component;

const LIMIT: usize = 256;
pub(super) type Pools = BTreeMap<String, Vec<re::RoadEditingCurveProgram>>;

pub(super) fn part(traffic: &TrafficPackage, component: &Component) -> TrafficPackage {
    let part = clip_paths(
        traffic,
        component.paths.iter().map(String::as_str).collect(),
    );
    assert_eq!(
        part.maneuver_paths
            .iter()
            .map(|p| p.id.clone())
            .collect::<BTreeSet<_>>(),
        component.paths.iter().cloned().collect()
    );
    assert_eq!(
        part.lane_graph
            .edges
            .iter()
            .map(|p| p.id.clone())
            .collect::<BTreeSet<_>>(),
        component.edges.iter().cloned().collect()
    );
    assert_eq!(
        part.signals
            .controllers
            .iter()
            .map(|p| p.id.clone())
            .collect::<BTreeSet<_>>(),
        component.controllers.iter().cloned().collect()
    );
    let expected = traffic
        .signals
        .controllers
        .iter()
        .filter(|c| component.controllers.contains(&c.id))
        .collect::<Vec<_>>();
    assert_eq!(
        json!(expected),
        json!(part.signals.controllers),
        "complete unchanged controller programs"
    );
    part
}

pub(super) fn evaluate(
    traffic: &TrafficPackage,
    component: &Component,
    originals: &BTreeMap<String, re::RoadEditingCurveProgram>,
    pools: &Pools,
    save: Option<&Path>,
) -> Value {
    let part = part(traffic, component);
    let path = &part.maneuver_paths[0];
    let mut programs = component
        .edges
        .iter()
        .filter_map(|id| originals.get(id).map(|p| (id.clone(), p.clone())))
        .collect::<BTreeMap<_, _>>();
    let missing = component
        .missing_edges
        .iter()
        .filter(|id| pools.get(*id).is_none_or(Vec::is_empty))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return json!({"status":"missing_real_programs","missing_edges":missing,"trials":[],"retained_pool_assignments_exhausted":false});
    }
    let keys = &component.missing_edges;
    assert!(keys.iter().all(|id| !originals.contains_key(id)));
    let total = keys
        .iter()
        .try_fold(1usize, |n, id| n.checked_mul(pools[id].len()));
    let mut choices = vec![0; keys.len()];
    let mut trials = Vec::new();
    let mut selected = None;
    let mut exhausted = false;
    for trial in 0..LIMIT {
        let assignment = keys
            .iter()
            .cloned()
            .zip(choices.iter().copied())
            .collect::<BTreeMap<_, _>>();
        for (id, &choice) in &assignment {
            programs.insert(id.clone(), pools[id][choice].clone());
        }
        let result = compile_part(traffic, path, &part, &programs, None);
        if result["status"] == "accepted" && selected.is_none() {
            selected = Some(assignment.clone());
        }
        trials.push(json!({"trial":trial,"assignment":assignment,"result":
            if result["status"] == "accepted" {json!({"status":"accepted"})} else {result}}));
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
    let accepted = selected.is_some();
    let selected_result = selected.as_ref().map(|assignment| {
        for (id, &choice) in assignment {
            programs.insert(id.clone(), pools[id][choice].clone());
        }
        let result = compile_part(traffic, path, &part, &programs, save);
        assert_eq!(result["status"], "accepted");
        result
    });
    for id in &component.edges {
        if let Some(original) = originals.get(id) {
            assert_eq!(
                program_bits(original),
                program_bits(&programs[id]),
                "fixed successful edge"
            );
        }
    }
    json!({"status":if accepted {"accepted"} else if exhausted {"retained_pool_rejected"} else {"assignment_budget_stopped"},
        "selected_assignment":selected,"selected_result":selected_result,"trials":trials,
        "retained_pool_assignments_exhausted":exhausted,"assignment_budget_stopped":!exhausted,
        "total_assignments":total,"assignment_limit":LIMIT,"complete_component_paths":component.paths,
        "signals":part.signals})
}

#[test]
fn shared_two_edge_assignments_require_the_whole_signal_component() {
    let observation = small_observation();
    let traffic = observation.traffic.as_ref().unwrap();
    let mut originals = small_programs(&observation);
    let targets = BTreeSet::from([traffic.maneuver_paths[0].id.clone()]);
    let first = traffic.maneuver_paths[0].entry_edge_id.clone();
    let second = traffic.maneuver_paths[0].internal_edge_ids[0].clone();
    let shifted = |program: &re::RoadEditingCurveProgram| {
        let p = program_points(program);
        re::RoadEditingCurveProgram::try_new(
            fixture_point(fixture_translate(p[0], [0.0, 0.2, 0.0])),
            p[1..]
                .iter()
                .map(|&v| {
                    re::RoadEditingCurveSegment::line(fixture_point(fixture_translate(
                        v,
                        [0.0, 0.2, 0.0],
                    )))
                })
                .collect(),
        )
        .unwrap()
    };
    let a = originals.remove(&first).unwrap();
    let b = originals.remove(&second).unwrap();
    let pools = BTreeMap::from([
        (first.clone(), vec![shifted(&a), a]),
        (second.clone(), vec![shifted(&b), b]),
    ]);
    let component = components::inventory(traffic, &originals, &targets).remove(0);
    assert!(component.paths.len() > 1);
    assert!(!component.controllers.is_empty());
    let result = evaluate(traffic, &component, &originals, &pools, None);
    assert_eq!(result["trials"].as_array().unwrap().len(), 4);
    assert_eq!(result["retained_pool_assignments_exhausted"], true);
    assert_eq!(result["selected_assignment"][&first], 1);
    assert_eq!(result["selected_assignment"][&second], 1);
    assert_eq!(result["status"], "accepted");
    let mut missing = pools.clone();
    missing.remove(&second);
    assert_eq!(
        evaluate(traffic, &component, &originals, &missing, None)["status"],
        "missing_real_programs"
    );
    let bad = pools
        .into_iter()
        .map(|(id, p)| (id, vec![p[0].clone()]))
        .collect();
    let rejected = evaluate(traffic, &component, &originals, &bad, None);
    assert_eq!(rejected["status"], "retained_pool_rejected");
    assert_eq!(rejected["retained_pool_assignments_exhausted"], true);
}

#[test]
fn assignment_limit_does_not_claim_exhaustion_or_accept_an_unvisited_candidate() {
    let observation = small_observation();
    let traffic = observation.traffic.as_ref().unwrap();
    let mut originals = small_programs(&observation);
    let path = &traffic.maneuver_paths[0];
    let id = path.internal_edge_ids[0].clone();
    let good = originals.remove(&id).unwrap();
    let points = program_points(&good);
    let bad = re::RoadEditingCurveProgram::try_new(
        fixture_point(fixture_translate(points[0], [0.0, 1.0, 0.0])),
        points[1..]
            .iter()
            .map(|&p| {
                re::RoadEditingCurveSegment::line(fixture_point(fixture_translate(
                    p,
                    [0.0, 1.0, 0.0],
                )))
            })
            .collect(),
    )
    .unwrap();
    let mut candidates = vec![bad; LIMIT];
    candidates.push(good);
    let pools = BTreeMap::from([(id, candidates)]);
    let component =
        components::inventory(traffic, &originals, &BTreeSet::from([path.id.clone()])).remove(0);
    let result = evaluate(traffic, &component, &originals, &pools, None);
    assert_eq!(result["status"], "assignment_budget_stopped");
    assert_eq!(result["retained_pool_assignments_exhausted"], false);
    assert!(result["selected_assignment"].is_null());
}
