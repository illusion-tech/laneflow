//! #1131 流 A：C29+C17 失败边的曲线/分段设计变量有限探索与完整组合验收。
//! 执行口径冻结于 `research/issue-1131-curve-design-c29-c17/plan.md`，方法事实源为
//! `research/issue-1115-failed-curve-design/design.md`；本模块不改生产采样、compiler
//! 配置档、原成功几何、拓扑与控制锚点。计划与冻结实现不一致时停止该处执行并重新授权，
//! 不得单向迁就（plan §7）。三条未决边的定位是有限设计变量探索：零恢复、族耗尽、
//! 预算停止分别保存，均不写可行或不可行结论（plan §6）。

use super::*;
use experiment::{merge, pool};

/// plan §1/§6 冻结样本与段容量表：顺序即搜索顺序，`s` 为原修复参考链实际 cubic 数。
const EDGES: [(&str, usize); 4] = [
    ("sumo:--30452_0", 3),
    ("sumo::-20112_0_0", 4),
    ("sumo::-10564_3_0", 2),
    ("sumo::-10564_6_0", 3),
];
/// design.md：每段最多 96 片；临时程序点上限 96s+1（s=6 的特例即 577 点）。
const PIECES_PER_CUBIC_MAX: usize = 96;

const C29_COMPONENT: &str = "sumo:-20112:--30452_0-to---31818#6_1::-20112_1_0";
const C29_TARGET_PATHS: [&str; 2] = [
    "sumo:-20112:--30452_0-to---31818#6_1::-20112_1_0",
    "sumo:-20112:--30452_0-to--31818#7_0::-20112_0_0",
];
const C29_MISSING_EDGES: [&str; 2] = ["sumo:--30452_0", "sumo::-20112_0_0"];
const C17_COMPONENT: &str = "sumo:-10564:--32172#4_0-to--31194_0::-10564_3_0.:-10564_6_0";
const C17_TARGET_PATHS: [&str; 1] = [C17_COMPONENT];
const C17_MISSING_EDGES: [&str; 2] = ["sumo::-10564_3_0", "sumo::-10564_6_0"];

fn normalized(value: &Value) -> Value {
    serde_json::from_str(&value.to_string()).unwrap()
}

fn frozen_inputs() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/inputs.json"
    ))
    .unwrap()
}

pub(super) fn pool_counts(pools: &combinations::Pools) -> (usize, usize) {
    (pools.len(), pools.values().map(Vec::len).sum())
}

pub(super) fn per_edge_counts(pools: &combinations::Pools) -> BTreeMap<String, usize> {
    pools
        .iter()
        .map(|(id, candidates)| (id.clone(), candidates.len()))
        .collect()
}

/// plan §5 冻结基线候选池：四来源固定顺序①→④合并，去重键为规范 f32 点位；
/// 断言 9 边 75 份（边集与每边份数）。任一来源缺失或逐值不符即停止执行并重新授权。
fn baseline_pool() -> combinations::Pools {
    let legacy: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1103-s1-recovery-budget/results.json"
    ))
    .unwrap();
    let old: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/old-searches.json"
    ))
    .unwrap();
    let grid: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1111-grid-calibration/positive-calibration.json"
    ))
    .unwrap();
    let curve: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1115-failed-curve-design/searches.json"
    ))
    .unwrap();
    let mut pools = pool(&legacy["tiers"]["32768"]["searches"]);
    assert_eq!(
        pool_counts(&pools),
        (6, 48),
        "source 1: 6 edges 48 candidates"
    );
    merge(&mut pools, pool(&old));
    assert_eq!(
        pool_counts(&pools),
        (7, 56),
        "source 2: 7 edges 56 candidates"
    );
    merge(&mut pools, pool(&grid["edges"]));
    assert_eq!(
        pool_counts(&pools),
        (7, 72),
        "source 3: 7 edges 72 candidates"
    );
    merge(&mut pools, pool(&curve));
    assert_eq!(
        pool_counts(&pools),
        (9, 75),
        "source 4: 9 edges 75 candidates"
    );
    assert_eq!(
        per_edge_counts(&pools),
        BTreeMap::from([
            ("sumo::-10004_5_1".to_owned(), 16),
            ("sumo::-10034_0_0".to_owned(), 8),
            ("sumo::-10130_8_0".to_owned(), 8),
            ("sumo::-10130_16_0".to_owned(), 8),
            ("sumo::-10130_18_0".to_owned(), 8),
            ("sumo::-16502_0_0".to_owned(), 1),
            ("sumo::-28860_3_0".to_owned(), 2),
            ("sumo::-29388_3_0".to_owned(), 8),
            ("sumo::-7348_3_0".to_owned(), 16),
        ]),
        "frozen baseline per-edge candidate counts"
    );
    pools
}

/// 每边按冻结段容量 s 执行 design.md 口径的有限几何设计变量候选搜索。
fn run_searches(inputs: &Value) -> (Value, Value) {
    let mut searches = BTreeMap::new();
    let mut timing = BTreeMap::new();
    for (id, max_curves) in EDGES {
        let input = measurement::input(&inputs[id]);
        let started = Instant::now();
        let search = sampler::reshape::search_with_curve_cap(&input, max_curves);
        timing.insert(id, started.elapsed().as_secs_f64());
        // plan §6：段容量按 s 冻结；参考链实际 cubic 数与 96s+1 点上限即断言口径。
        assert_eq!(
            search.report["reference_controls_f64_bits"]
                .as_array()
                .unwrap()
                .len(),
            max_curves,
            "frozen reference cubic count"
        );
        assert_eq!(
            search.report["policy"]["reference_cubics_max"],
            json!(max_curves)
        );
        searches.insert(
            id,
            json!({"search":search.report,"candidates":recovery::candidates(&search)}),
        );
    }
    (json!(searches), json!(timing))
}

/// plan §2/§5：C29 两条完整路径同一赋值同时验收，C17 单条完整路径两份程序齐备后验收；
/// 只有整个分量接受才选用其赋值，任一路径不接受即分量保持缺程序。
fn joint_acceptance(
    rows: &[Value],
    paths: &[Value],
    component: &str,
    target_paths: &[&str],
    missing_edges: &[&str],
) -> Value {
    let result = &rows
        .iter()
        .find(|row| row["component"] == component)
        .expect("cohort component evaluated")["result"];
    let accepted = result["status"] == "accepted";
    let statuses = target_paths
        .iter()
        .map(|id| {
            let row = paths
                .iter()
                .find(|row| row["path"] == *id)
                .expect("target path row");
            json!({"path":id,"accepted":row["result"]["status"] == "accepted"})
        })
        .collect::<Vec<_>>();
    assert_eq!(
        accepted,
        statuses.iter().all(|row| row["accepted"] == true),
        "component acceptance equals simultaneous complete-path acceptance"
    );
    if accepted {
        let assignment = result["selected_assignment"].as_object().unwrap();
        let keys = assignment.keys().map(String::as_str).collect::<Vec<_>>();
        assert_eq!(
            keys,
            missing_edges.to_vec(),
            "one assignment covers every missing edge"
        );
    }
    json!({"component":component,"accepted":accepted,"target_paths":statuses,
        "selected_assignment":result["selected_assignment"],
        "trials":result["trials"].as_array().unwrap().len(),
        "assignment_budget_stopped":result["assignment_budget_stopped"],
        "unvisited_assignments":result["unvisited_assignments"]})
}

/// plan §4 长度报告清单：来源 `@length`、来源 shape 折线长、原/新参考弧长（4096 区间
/// 双估计）、规范 f32 弦长累计、compiler `length_mm` 与来源差值；失败边不声称规范长度
/// 保持不变，无接受编译时 compiler 列为空。
fn edge_reports(searches: &Value, traffic: &TrafficPackage, variant_paths: &[Value]) -> Value {
    let rows = EDGES.map(|(id, max_curves)| {
        let search = &searches[id];
        let edge = traffic
            .lane_graph
            .edges
            .iter()
            .find(|edge| edge.id == id)
            .expect("sample edge in topology");
        let compiler_rows = variant_paths
            .iter()
            .filter(|row| row["result"]["status"] == "accepted")
            .flat_map(|row| {
                row["result"]["edges"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|edge_row| edge_row["edge"] == id)
                    .map(|edge_row| {
                        json!({"path":row["path"],"length_mm":edge_row["length_mm"],
                            "canonical_arc_m":edge_row["canonical_arc_m"],
                            "source_length_delta_m":edge_row["source_length_delta_m"]})
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let mut status_counts = BTreeMap::<String, u64>::new();
        for row in search["search"]["geometries"].as_array().unwrap() {
            *status_counts
                .entry(row["status"].as_str().unwrap().to_owned())
                .or_default() += 1;
        }
        json!({"edge":id,"reference_cubics":max_curves,
            "temp_program_points_max":PIECES_PER_CUBIC_MAX*max_curves+1,
            "source_length_m":edge.length,
            "source_shape_polyline_length_m":search["search"]["source_shape_polyline_length_m"],
            "original_reference_length_estimates":search["search"]["reference_length_estimates"],
            "original_reference_source_fit":search["search"]["reference_fit"],
            "geometry_status_counts":status_counts,
            "partition_attempts":search["search"]["counts"]["partitions"],
            "geometry_family_visited":search["search"]["geometry_family_visited"],
            "stop_reason":search["search"]["stop_reason"],
            "retained_candidates":search["candidates"].as_array().unwrap().iter().map(|candidate| json!({
                "geometry":candidate["spans"][0]["geometry"],
                "partition":candidate["spans"][0]["partition"],
                "pieces_per_cubic":candidate["spans"][0]["pieces_per_cubic"],
                "source_fit":candidate["spans"][0]["source_fit"],
                "canonical_source_fit":candidate["spans"][0]["canonical_source_fit"],
                "reference_length_estimates":candidate["spans"][0]["reference_length_estimates"],
                "canonical_f32_chord_sum_m":candidate["arc_m"]})).collect::<Vec<_>>(),
            "compiler_rows":compiler_rows})
    });
    json!(rows)
}

#[test]
#[ignore = "requires fresh CURVE_DESIGN_OUTPUT_DIR"]
fn fixed_stream_a_curve_design() {
    let out = PathBuf::from(std::env::var("CURVE_DESIGN_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh curve design output");
    fs::create_dir_all(&out).unwrap();
    let (searches, timing) = run_searches(&frozen_inputs());
    recovery::write_json(&out, "stream-a-searches.json", &searches);
    recovery::write_json(&out, "stream-a-timing.json", &timing);
}

#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, DIRECT_SUCCESSOR_BASELINE_DIR, CURVE_DESIGN_FROZEN_RESULT, CURVE_DESIGN_ORDER and fresh CURVE_DESIGN_OUTPUT_DIR"]
fn pinned_lust_stream_a_curve_experiment() {
    let out = PathBuf::from(std::env::var("CURVE_DESIGN_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh full curve design output");
    fs::create_dir_all(&out).unwrap();
    let order = std::env::var("CURVE_DESIGN_ORDER").unwrap();
    assert!(matches!(order.as_str(), "baseline-new" | "new-baseline"));
    let frozen: Value = serde_json::from_slice(
        &fs::read(std::env::var("CURVE_DESIGN_FROZEN_RESULT").unwrap()).unwrap(),
    )
    .unwrap();
    let comparison: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1115-failed-curve-design/results.json"
    ))
    .unwrap();
    let inventory: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/inventory.json"
    ))
    .unwrap();
    let (observation, source, mut timing) = capture_lust();
    assert_eq!(source, comparison["source"]);
    let originals = original_programs(&observation);
    snapshot(&out, &observation, &originals);
    let traffic = observation.traffic.as_ref().unwrap();
    let components = components::inventory(traffic, &originals, &target_ids(&reference()));
    assert_eq!(json!(components), inventory["components"]);
    assert_eq!(components.len(), 53, "frozen component inventory");
    assert_eq!(components.iter().map(|c| c.paths.len()).sum::<usize>(), 183);
    measurement::write_inputs(
        &out,
        &observation,
        &json!({"samples":EDGES.map(|(id, _)|json!({"edge":id}))}),
    );
    let inputs: Value =
        serde_json::from_slice(&fs::read(out.join("inputs.json")).unwrap()).unwrap();
    let old_inputs = frozen_inputs();
    for (id, _) in EDGES {
        assert_eq!(
            inputs[id], old_inputs[id],
            "unchanged source and emission failure"
        );
        assert!(!originals.contains_key(id), "only failed edges redesigned");
    }
    let (searches, search_times) = run_searches(&inputs);
    assert_eq!(normalized(&searches), frozen, "frozen candidate replay");
    recovery::write_json(&out, "stream-a-searches.json", &searches);
    let baseline = baseline_pool();
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
            let file = format!("stream-a-{family}-component-{index:02}.lfca");
            if C29_COMPONENT == component.id || C17_COMPONENT == component.id {
                let part = combinations::part(traffic, component);
                assert!(
                    part.signals
                        .stop_lines
                        .iter()
                        .all(|s| originals.contains_key(&s.edge_id)),
                    "affected stop line geometry is original"
                );
            }
            let mut result = combinations::evaluate(
                traffic,
                component,
                &originals,
                pools,
                Some(&out.join(&file)),
            );
            if family == "baseline" {
                let prior = &comparison["variants"]["new"]["components"][index];
                assert_eq!(prior["component"], json!(component.id));
                assert_eq!(
                    normalized(&result),
                    prior["result"],
                    "baseline component matches 1115 variants.new"
                );
            }
            // plan §5：组合超 256 记 `assignment_budget_stopped` 与未访问赋值数。
            let unvisited = result
                .get("total_assignments")
                .and_then(Value::as_u64)
                .map(|total| total - result["trials"].as_array().unwrap().len() as u64);
            result["unvisited_assignments"] = json!(unvisited);
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
        let paths = reference()["baseline_paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let path = traffic
                    .maneuver_paths
                    .iter()
                    .find(|p| p.id == row["path"].as_str().unwrap())
                    .unwrap();
                json!({"path":path.id,"edges":path_edges(path),"result":compile_path(traffic,path,&programs)})
            })
            .collect::<Vec<_>>();
        for (current, previous) in paths
            .iter()
            .zip(comparison["variants"]["new"]["paths"].as_array().unwrap())
        {
            assert_eq!(current["path"], previous["path"]);
            if previous["result"]["status"] == "accepted" {
                assert_eq!(
                    current["result"]["status"], "accepted",
                    "accepted target retained"
                );
            }
        }
        if family == "baseline" {
            assert_eq!(
                normalized(&json!(paths)),
                comparison["variants"]["new"]["paths"],
                "baseline target evidence matches 1115 variants.new"
            );
        }
        let counts = paths
            .iter()
            .fold(BTreeMap::<String, usize>::new(), |mut counts, row| {
                *counts
                    .entry(row["result"]["status"].as_str().unwrap().to_owned())
                    .or_default() += 1;
                counts
            });
        let joint = json!({
            "c29":joint_acceptance(&rows,&paths,C29_COMPONENT,&C29_TARGET_PATHS,&C29_MISSING_EDGES),
            "c17":joint_acceptance(&rows,&paths,C17_COMPONENT,&C17_TARGET_PATHS,&C17_MISSING_EDGES)});
        results.insert(
            family,
            json!({"components":rows,"paths":paths,"path_counts":counts,
                "pool_sizes":pools.iter().map(|(id,p)|(id,p.len())).collect::<BTreeMap<_,_>>(),
                "joint_acceptance":joint}),
        );
        validation_times.insert(family, started.elapsed().as_secs_f64());
    }
    let baseline_result = results.get("baseline").unwrap().clone();
    let new_result = results.get("new").unwrap().clone();
    let newly_accepted = baseline_result["components"]
        .as_array()
        .unwrap()
        .iter()
        .zip(new_result["components"].as_array().unwrap())
        .filter(|(before, after)| {
            after["result"]["status"] == "accepted" && before["result"]["status"] != "accepted"
        })
        .map(|(before, _)| before["component"].clone())
        .collect::<Vec<_>>();
    let trials_total = |variant: &Value| -> u64 {
        variant["components"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["result"]["trials"].as_array().unwrap().len() as u64)
            .sum()
    };
    let gain = json!({
        "new_candidates_per_edge":EDGES.map(|(id,_)|json!({"edge":id,
            "candidates":combined.get(id).map(Vec::len).unwrap_or(0)})),
        "components_newly_accepted":newly_accepted,
        "component_assignment_trials":{"baseline":trials_total(&baseline_result),
            "new":trials_total(&new_result)},
        "baseline_path_counts":baseline_result["path_counts"],
        "new_path_counts":new_result["path_counts"]});
    // 结果对照：基线重新验证与 #1115 `variants.new` 逐值一致；该文件只作结果对照，
    // 不作候选点位来源（plan §5）。
    assert_eq!(baseline_result["path_counts"]["accepted"], 28);
    let result_comparison = json!({
        "target":"research/issue-1115-failed-curve-design/results.json#variants.new",
        "role":"result comparison only; not a candidate source",
        "components_result_equal":true,"paths_equal":true,"path_counts_equal":true,
        "accepted_paths":baseline_result["path_counts"]["accepted"],
        "note":"28/84 是含 #1115 曲线候选的盘点口径；#1121 不含曲线候选的回池口径 26/84 仅作对照"});
    recovery::write_json(
        &out,
        "stream-a-results.json",
        &json!({"schema":"laneflow-1131-stream-a-curve-design","source":source,
        "cohort_source":"research/issue-1131-curve-design-c29-c17/edges.json",
        "method_source":"research/issue-1115-failed-curve-design/design.md",
        "segment_capacity":EDGES.map(|(id,max_curves)|json!({"edge":id,"reference_cubics":max_curves,
            "temp_program_points_max":PIECES_PER_CUBIC_MAX*max_curves+1})),
        "baseline_pool":{"sources":["research/issue-1103-s1-recovery-budget/results.json",
            "research/issue-1107-dependency-recovery/old-searches.json",
            "research/issue-1111-grid-calibration/positive-calibration.json#edges",
            "research/issue-1115-failed-curve-design/searches.json"],
            "edge_count":pool_counts(&baseline).0,"candidate_count":pool_counts(&baseline).1,
            "per_edge":per_edge_counts(&baseline)},
        "new_pool":{"edge_count":pool_counts(&combined).0,"candidate_count":pool_counts(&combined).1,
            "per_edge":per_edge_counts(&combined)},
        "edge_reports":edge_reports(&searches, traffic, new_result["paths"].as_array().unwrap()),
        "variants":results,"result_comparison":result_comparison,"new_gain":gain,
        "original_geometry_controls_and_failures_equal":true,"production_g1_accepted":false}),
    );
    timing["search_order"] = json!(order);
    timing["search_seconds"] = json!(search_times);
    timing["validation_seconds"] = json!(validation_times);
    recovery::write_json(&out, "stream-a-timing.json", &timing);
    println!("stream A curve design evidence saved={}", out.display());
}

#[test]
fn stream_a_cohort_and_capacities_match_the_frozen_plan() {
    let cohort: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1131-curve-design-c29-c17/edges.json"
    ))
    .unwrap();
    assert_eq!(cohort["stream"], "stream-a");
    let ids = cohort["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| edge["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids, EDGES.map(|(id, _)| id).to_vec());
    // plan §6 段容量表：289/385/193/289 = 96s+1。
    assert_eq!(
        EDGES.map(|(_, s)| PIECES_PER_CUBIC_MAX * s + 1),
        [289, 385, 193, 289]
    );
    for row in cohort["components"].as_array().unwrap() {
        let (id, target_paths, missing): (&str, &[&str], &[&str]) =
            match row["code"].as_str().unwrap() {
                "C29" => (C29_COMPONENT, &C29_TARGET_PATHS, &C29_MISSING_EDGES),
                "C17" => (C17_COMPONENT, &C17_TARGET_PATHS, &C17_MISSING_EDGES),
                other => panic!("unexpected cohort component {other}"),
            };
        assert_eq!(row["id"], id);
        assert_eq!(row["failed_edges"], json!(missing));
        let frozen_paths = match &row["paths"] {
            Value::Array(paths) => paths
                .iter()
                .map(|path| path.as_str().unwrap().to_owned())
                .collect::<Vec<_>>(),
            Value::String(path) => vec![path.clone()],
            other => panic!("unexpected frozen paths {other}"),
        };
        assert_eq!(
            frozen_paths,
            target_paths
                .iter()
                .map(|path| (*path).to_owned())
                .collect::<Vec<_>>()
        );
        assert_eq!(row["path_count"], json!(target_paths.len()));
    }
}

#[test]
fn stream_a_baseline_pool_merges_four_frozen_sources_in_order() {
    let pools = baseline_pool();
    assert_eq!(pool_counts(&pools), (9, 75));
}

#[test]
fn curve_capacity_parameterization_keeps_the_frozen_six_default() {
    assert_eq!(sampler::reshape::policy()["reference_cubics_max"], 6);
    for (id, max_curves) in EDGES {
        assert_eq!(
            sampler::reshape::policy_with_curve_cap(max_curves)["reference_cubics_max"],
            json!(max_curves),
            "{id} curve capacity"
        );
    }
}
