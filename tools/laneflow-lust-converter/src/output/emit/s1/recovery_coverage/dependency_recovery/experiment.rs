use super::*;
use combinations::Pools;

#[test]
fn shared_baseline_facade_reachable_from_dependency_recovery_subtree() {
    // 契约（lead E0603 修复授权）：门面须自 dependency_recovery 兄弟子树按 README
    // 路径可达（A c12/B c17 调用点）；`mod shared_baseline` 可见性回归即编译失败。
    let _: fn() -> Pools = super::calibration::shared_baseline::shared_pools;
    let _: fn() -> Value = super::calibration::shared_baseline::results_anchor_pinned;
}

#[test]
fn quality_fit_entries_reachable_from_emit_subtree() {
    // 契约（lead E0603 同族防回归；可见性类缺陷根治）：reshape::quality 的
    // fit_with_handle/Fit 须按文档路径自 emit 兄弟子树可达（D #1154
    // calibration::witness_guided_search 调用点）；`mod quality` 可见性回归即编译
    // 失败。`fit` 保持 pub(super)（签名含 reshape 内部 Budget，不对外泄露——D 只经
    // fit_with_handle 调用）。
    use crate::output::geom::s1::reshape::quality;
    type FitWithHandle = fn(
        &[[[f64; 3]; 6]],
        &[[f64; 3]],
        &mut crate::output::geom::s1::grid::WorkBudget,
    ) -> Option<quality::Fit>;
    let _: FitWithHandle = quality::fit_with_handle;
}

#[test]
fn reference_capture_reachable_from_emit_subtree() {
    // 契约（可见性类防回归）：reshape::reference_with_curve_cap（全边参考入口，
    // D #1154 P2-1 修法）须自 emit 兄弟子树可达；可见性回归即编译失败。
    type ReferenceCapture = fn(&observe::EdgeInput, usize) -> Vec<[[f64; 3]; 6]>;
    let _: ReferenceCapture = crate::output::geom::s1::reshape::reference_with_curve_cap;
}

fn normalized(value: &Value) -> Value {
    serde_json::from_str(&value.to_string()).unwrap()
}

pub(super) fn pool(rows: &Value) -> Pools {
    rows.as_object()
        .unwrap()
        .iter()
        .filter_map(|(id, row)| {
            let candidates = row["candidates"].as_array().unwrap();
            (!candidates.is_empty()).then(|| {
                (
                    id.clone(),
                    candidates
                        .iter()
                        .map(|c| successors::frozen_candidate(c).repaired.program)
                        .collect(),
                )
            })
        })
        .collect()
}

pub(super) fn merge(pools: &mut Pools, new: Pools) {
    let bits = |p: &re::RoadEditingCurveProgram| {
        program_points(p)
            .into_iter()
            .map(|v| v.map(|x| (x as f32).to_bits()))
            .collect::<Vec<_>>()
    };
    for (id, candidates) in new {
        let selected = pools.entry(id).or_default();
        for candidate in candidates {
            let key = bits(&candidate);
            if !selected.iter().any(|p| bits(p) == key) {
                assert!(selected.len() < 16, "bounded merged pool");
                selected.push(candidate);
            }
        }
    }
}

fn run_family(
    observation: &observe::EmissionObservation,
    ids: &BTreeSet<String>,
    grid: bool,
) -> (Value, Value) {
    let mut rows = BTreeMap::new();
    let mut times = BTreeMap::new();
    let old: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1103-s1-recovery-budget/results.json"
    ))
    .unwrap();
    for ((id, original), input) in observation.edges.iter().zip(&observation.inputs) {
        if !ids.contains(id) {
            continue;
        }
        let observe::EdgeEmission::Failed(error) = original else {
            panic!("only failed edges searched");
        };
        let started = Instant::now();
        let search = if grid {
            sampler::grid::search(input)
        } else {
            sampler::search(input, sampler::SearchBudget::default())
        };
        times.insert(id.clone(), started.elapsed().as_secs_f64());
        let candidates = recovery::candidates(&search);
        if !grid && let Some(reference) = old["tiers"]["32768"]["searches"].get(id) {
            assert_eq!(
                normalized(&search.report),
                reference["search"],
                "old search unchanged"
            );
            assert_eq!(
                normalized(&candidates),
                reference["candidates"],
                "old candidate pool unchanged"
            );
        }
        rows.insert(
            id.clone(),
            json!({"original_error":error,"search":search.report,"candidates":candidates}),
        );
    }
    assert_eq!(rows.len(), ids.len());
    (json!(rows), json!(times))
}

#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, DIRECT_SUCCESSOR_BASELINE_DIR, DEPENDENCY_ORDER and fresh DEPENDENCY_OUTPUT_DIR"]
fn pinned_lust_dependency_experiment() {
    let out = fresh_output();
    let order = std::env::var("DEPENDENCY_ORDER").unwrap();
    assert!(matches!(order.as_str(), "old-grid" | "grid-old"));
    let reference = reference();
    let (observation, source, mut timing) = capture_lust();
    assert_eq!(source, reference["source"]);
    let originals = original_programs(&observation);
    snapshot(&out, &observation, &originals);
    let traffic = observation.traffic.as_ref().unwrap();
    let components = components::inventory(traffic, &originals, &target_ids(&reference));
    let frozen: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/inventory.json"
    ))
    .unwrap();
    assert_eq!(json!(components), frozen["components"]);
    assert_eq!(components::selection(&components), frozen["selection"]);
    let ids: BTreeSet<String> =
        serde_json::from_value(frozen["selection"]["search_edges"].clone()).unwrap();
    measurement::write_inputs(
        &out,
        &observation,
        &json!({"samples":ids.iter().map(|id|json!({"edge":id})).collect::<Vec<_>>()}),
    );
    let baseline = reference["baseline_paths"].as_array().unwrap().iter().map(|row| {
        let path = traffic.maneuver_paths.iter().find(|p|p.id == row["path"].as_str().unwrap()).unwrap();
        let current = json!({"path":path.id,"edges":path_edges(path),"result":compile_path(traffic,path,&originals)});
        assert_eq!(normalized(&current),*row,"unchanged original path result");
        current
    }).collect::<Vec<_>>();
    let mut searches = BTreeMap::new();
    let mut search_times = BTreeMap::new();
    for family in order.split('-') {
        let (rows, times) = run_family(&observation, &ids, family == "grid");
        recovery::write_json(&out, &format!("{family}-searches.json"), &rows);
        searches.insert(family.to_owned(), rows);
        search_times.insert(family.to_owned(), times);
    }
    let old: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1103-s1-recovery-budget/results.json"
    ))
    .unwrap();
    let legacy = pool(&old["tiers"]["32768"]["searches"]);
    assert_eq!(legacy.len(), 6);
    let mut extended = legacy.clone();
    merge(&mut extended, pool(&searches["old"]));
    let mut combined = extended.clone();
    merge(&mut combined, pool(&searches["grid"]));
    let mut results = BTreeMap::new();
    let mut validation_times = BTreeMap::new();
    for (family, pools) in [
        ("legacy", legacy),
        ("extended_old", extended),
        ("combined", combined),
    ] {
        let started = Instant::now();
        let mut programs = originals.clone();
        for (id, index) in reference["tiers"]["32768"]["best"]["assignment"]
            .as_object()
            .unwrap()
        {
            programs.insert(
                id.clone(),
                pools[id][index.as_u64().unwrap() as usize].clone(),
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
                for (id, index) in result["selected_assignment"].as_object().unwrap() {
                    programs.insert(
                        id.clone(),
                        pools[id][index.as_u64().unwrap() as usize].clone(),
                    );
                }
            }
            rows.push(json!({"component":component.id,"result":result,"artifact":
                (result["status"] == "accepted").then_some(file)}));
        }
        let paths = reference["baseline_paths"].as_array().unwrap().iter().map(|row| {
            let path = traffic.maneuver_paths.iter().find(|p|p.id == row["path"].as_str().unwrap()).unwrap();
            json!({"path":path.id,"edges":path_edges(path),"result":compile_path(traffic,path,&programs)})
        }).collect::<Vec<_>>();
        for (row, prior) in paths.iter().zip(
            reference["tiers"]["32768"]["best"]["paths"]
                .as_array()
                .unwrap(),
        ) {
            assert_eq!(row["path"], prior["path"]);
            if prior["result"]["status"] == "accepted" {
                assert_eq!(row["result"]["status"], "accepted", "no regression");
            }
            if family == "legacy" {
                assert_eq!(normalized(row), *prior, "legacy target evidence");
            }
        }
        let counts = paths
            .iter()
            .fold(BTreeMap::<String, usize>::new(), |mut counts, row| {
                *counts
                    .entry(row["result"]["status"].as_str().unwrap().to_owned())
                    .or_default() += 1;
                counts
            });
        results.insert(
            family,
            json!({"components":rows,"paths":paths,"path_counts":counts,
            "pool_sizes":pools.iter().map(|(id,p)|(id,p.len())).collect::<BTreeMap<_,_>>()}),
        );
        validation_times.insert(family, started.elapsed().as_secs_f64());
    }
    timing["search_order"] = json!(order);
    timing["edge_search_seconds"] = json!(search_times);
    timing["validation_seconds"] = json!(validation_times);
    recovery::write_json(
        &out,
        "results.json",
        &json!({"schema":"laneflow-1107-dependency-recovery", "source":source,
        "grid_policy":sampler::grid::policy(),"old_policy":sampler::policy(sampler::SearchBudget::default()),
        "inventory":frozen,"baseline_paths":baseline,"variants":results,
        "all_original_geometry_and_controls_equal":true,"production_g1_accepted":false}),
    );
    recovery::write_json(&out, "timing.json", &timing);
    println!("dependency recovery saved={}", out.display());
}
