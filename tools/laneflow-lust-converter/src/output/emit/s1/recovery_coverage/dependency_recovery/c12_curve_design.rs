//! #1136 流 A：C12 单缺边（`sumo::-10100_1_0`）的曲线/分段设计变量有限探索与完整分量验收。
//! 执行口径冻结于 `research/issue-1136-c12-curve-design/plan.md`，方法事实源为
//! `research/issue-1115-failed-curve-design/design.md`；本模块不改生产采样、compiler
//! 配置档、原成功几何、拓扑与控制锚点。首弦全参数域排除只对原固定 cubic 成立，span 0
//! 末弦见证仅为局部问题存在性、与排除正交并存；不预设恢复，零恢复为有效结论
//! （plan §1/§6）。共同基线按六路协调章程 v2 经 E0 冻结交付读取（plan §5）。

use super::*;
use experiment::{merge, pool};

/// plan §1/§6 冻结样本与段容量：C12 唯一缺边，原诊断 span 0/4 → s=4，临时程序 ≤385 点。
const EDGES: [(&str, usize); 1] = [("sumo::-10100_1_0", 4)];
/// design.md：每段最多 96 片；临时程序点上限 96s+1（s=6 的特例即 577 点）。
const PIECES_PER_CUBIC_MAX: usize = 96;

const C12_COMPONENT: &str = "sumo:-10100:--31480_0-to--30896_0::-10100_1_0.:-10100_4_0";
const C12_TARGET_PATHS: [&str; 1] = [C12_COMPONENT];
const C12_MISSING_EDGES: [&str; 1] = ["sumo::-10100_1_0"];
/// plan §2：span 0/1 接缝为规定倒圆切点（span 0 末弦见证对应位置），合段跨过须逐候选记录。
const PRESCRIBED_FILLET_SEAM_AFTER_SPAN: usize = 0;

fn normalized(value: &Value) -> Value {
    serde_json::from_str(&value.to_string()).unwrap()
}

fn frozen_inputs() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/inputs.json"
    ))
    .unwrap()
}

fn pool_counts(pools: &combinations::Pools) -> (usize, usize) {
    (pools.len(), pools.values().map(Vec::len).sum())
}

fn per_edge_counts(pools: &combinations::Pools) -> BTreeMap<String, usize> {
    pools
        .iter()
        .map(|(id, candidates)| (id.clone(), candidates.len()))
        .collect()
}

/// 分组从 span 0 起按组容量切块；span 0/1 落入同一组即合段跨过规定倒圆接缝。
fn crosses_prescribed_fillet_seam(group: &str, reference_cubics: usize) -> bool {
    // `reference` 为原参考链对照（保留全部分界），与 `keep` 同为不跨缝。
    let group_size = match group {
        "reference" | "keep" => 1,
        "pair" => 2,
        "whole" => reference_cubics,
        other => panic!("unknown seam group {other}"),
    };
    PRESCRIBED_FILLET_SEAM_AFTER_SPAN / group_size
        == (PRESCRIBED_FILLET_SEAM_AFTER_SPAN + 1) / group_size
}

/// 每边按冻结段容量 s 执行 design.md 口径的有限几何设计变量候选搜索；
/// 跨规定倒圆接缝的配置与候选逐处记录 `crosses_prescribed_fillet_seam`（plan §2）。
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
        let mut report = search.report.clone();
        for row in report["geometries"].as_array_mut().unwrap() {
            let group = row["description"]["group"].as_str().unwrap();
            row["crosses_prescribed_fillet_seam"] =
                json!(crosses_prescribed_fillet_seam(group, max_curves));
        }
        let mut candidates = recovery::candidates(&search);
        for candidate in candidates.as_array_mut().unwrap() {
            let index = candidate["spans"][0]["geometry"].as_u64().unwrap();
            let group = report["geometries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["geometry"] == index)
                .expect("geometry row")["description"]["group"]
                .as_str()
                .unwrap();
            candidate["crosses_prescribed_fillet_seam"] =
                json!(crosses_prescribed_fillet_seam(group, max_curves));
        }
        searches.insert(id, json!({"search":report,"candidates":candidates}));
    }
    (json!(searches), json!(timing))
}

/// plan §5：C12 单缺边单路径完整分量验收；只有整个分量接受才选用其赋值。
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
/// 保持不变、不等同源 `@length`；无接受编译时 compiler 列为空。
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
                "crosses_prescribed_fillet_seam":candidate["crosses_prescribed_fillet_seam"],
                "source_fit":candidate["spans"][0]["source_fit"],
                "canonical_source_fit":candidate["spans"][0]["canonical_source_fit"],
                "reference_length_estimates":candidate["spans"][0]["reference_length_estimates"],
                "canonical_f32_chord_sum_m":candidate["arc_m"]})).collect::<Vec<_>>(),
            "compiler_rows":compiler_rows})
    });
    json!(rows)
}

/// plan §5 两口径收益分列（章程 §1）：完整关联路径与 84 原目标分别计新增接受。
fn two_accounting_gains(baseline_result: &Value, new_result: &Value) -> Value {
    let mut associated_paths = 0_u64;
    let mut components = Vec::new();
    for (_, after) in baseline_result["components"]
        .as_array()
        .unwrap()
        .iter()
        .zip(new_result["components"].as_array().unwrap())
        .filter(|(before, after)| {
            after["result"]["status"] == "accepted" && before["result"]["status"] != "accepted"
        })
    {
        associated_paths += after["result"]["complete_component_paths"]
            .as_array()
            .unwrap()
            .len() as u64;
        components.push(after["component"].clone());
    }
    let targets = baseline_result["paths"]
        .as_array()
        .unwrap()
        .iter()
        .zip(new_result["paths"].as_array().unwrap())
        .filter(|(before, after)| {
            after["result"]["status"] == "accepted" && before["result"]["status"] != "accepted"
        })
        .count() as u64;
    json!({"newly_accepted_components":components,
        "complete_associated_paths":associated_paths,
        "original_targets_84":targets})
}

#[test]
#[ignore = "requires fresh CURVE_DESIGN_OUTPUT_DIR"]
fn fixed_c12_curve_design() {
    let out = PathBuf::from(std::env::var("CURVE_DESIGN_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh curve design output");
    fs::create_dir_all(&out).unwrap();
    let (searches, timing) = run_searches(&frozen_inputs());
    recovery::write_json(&out, "c12-searches.json", &searches);
    recovery::write_json(&out, "c12-timing.json", &timing);
}

#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, DIRECT_SUCCESSOR_BASELINE_DIR, SIX_STREAMS_BASELINE_DIR, CURVE_DESIGN_FROZEN_RESULT, CURVE_DESIGN_ORDER and fresh CURVE_DESIGN_OUTPUT_DIR"]
fn pinned_lust_c12_curve_experiment() {
    let out = PathBuf::from(std::env::var("CURVE_DESIGN_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh full curve design output");
    fs::create_dir_all(&out).unwrap();
    let order = std::env::var("CURVE_DESIGN_ORDER").unwrap();
    assert!(matches!(order.as_str(), "baseline-new" | "new-baseline"));
    let frozen: Value = serde_json::from_slice(
        &fs::read(std::env::var("CURVE_DESIGN_FROZEN_RESULT").unwrap()).unwrap(),
    )
    .unwrap();
    // plan §5 共同基线（Issue #1140 冻结交付）：经 `calibration::shared_baseline` 门面
    // 读取（钉版断言后返回 11 边 92 份池与 #1134 新增变体 92 池行锚，仅结果对照）。
    let baseline = calibration::shared_baseline::shared_pools();
    assert_eq!(
        pool_counts(&baseline),
        (11, 92),
        "frozen 11 edges 92 candidates"
    );
    let anchor = calibration::shared_baseline::results_anchor_pinned();
    assert_eq!(anchor["components"].as_array().unwrap().len(), 53);
    assert_eq!(anchor["paths"].as_array().unwrap().len(), 84);
    assert_eq!(anchor["path_counts"]["accepted"], 28, "frozen 28/84 anchor");
    let inventory: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/inventory.json"
    ))
    .unwrap();
    let (observation, source, mut timing) = capture_lust();
    // 锚的 `source` 为资产来源链（#1140 重钉版），非捕获源身份；捕获源一致性由
    // `snapshot()` 对 #1105 基线逐字节断言与钉版 LuST 校验承担。代际标签钉死
    // `92-池行`（锚代际错位事故的回归闸口）。
    assert_eq!(
        anchor["source"]["release_tag"],
        json!("research-evidence-1140-shared-baseline"),
        "pinned anchor provenance"
    );
    assert_eq!(
        anchor["source"]["asset"],
        json!("evidence/run-03/results.json")
    );
    assert_eq!(anchor["source"]["json_pointer"], json!("#variants.new"));
    assert_eq!(anchor["source"]["generation"], json!("92-池行"));
    assert!(
        anchor["source"]["comparison_only"].as_bool().unwrap(),
        "anchor is comparison only"
    );
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
    recovery::write_json(&out, "c12-searches.json", &searches);
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
            let file = format!("c12-{family}-component-{index:02}.lfca");
            if component.id == C12_COMPONENT {
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
            // plan §5：组合超 256 记 `assignment_budget_stopped` 与未访问赋值数
            //（= 总赋值 − 已试次数）。对照锚的分量行含该字段，先增强再逐值比较。
            let unvisited = result
                .get("total_assignments")
                .and_then(Value::as_u64)
                .map(|total| total - result["trials"].as_array().unwrap().len() as u64);
            result["unvisited_assignments"] = json!(unvisited);
            if family == "baseline" {
                let prior = &anchor["components"][index];
                assert_eq!(prior["component"], json!(component.id));
                assert_eq!(
                    normalized(&result),
                    prior["result"],
                    "baseline component matches 1134 anchor"
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
        for (current, previous) in paths.iter().zip(anchor["paths"].as_array().unwrap()) {
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
                anchor["paths"],
                "baseline target evidence matches 1134 anchor"
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
        let joint = joint_acceptance(
            &rows,
            &paths,
            C12_COMPONENT,
            &C12_TARGET_PATHS,
            &C12_MISSING_EDGES,
        );
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
    let trials_total = |variant: &Value| -> u64 {
        variant["components"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["result"]["trials"].as_array().unwrap().len() as u64)
            .sum()
    };
    // 实验独立（章程 §3）：新增 = 共同基线 + 本路候选；既有候选（sumo::-10564_3_0 等）
    // 随基线入池，本路新增收益单独统计。
    let gains = two_accounting_gains(&baseline_result, &new_result);
    let gain = json!({
        "new_candidates_per_edge":EDGES.map(|(id,_)|json!({"edge":id,
            "candidates":combined.get(id).map(Vec::len).unwrap_or(0)})),
        "components_newly_accepted":gains["newly_accepted_components"],
        "two_accounting_gains":{"complete_associated_paths":gains["complete_associated_paths"],
            "original_targets_84":gains["original_targets_84"]},
        "component_assignment_trials":{"baseline":trials_total(&baseline_result),
            "new":trials_total(&new_result)},
        "baseline_path_counts":baseline_result["path_counts"],
        "new_path_counts":new_result["path_counts"]});
    // 结果对照：基线重新验证与 #1134 对照锚逐值一致；锚只作结果对照，不作候选来源。
    assert_eq!(baseline_result["path_counts"]["accepted"], 28);
    let result_comparison = json!({
        "target":"E0 shared baseline results-anchor.json (#1134 new variant)",
        "role":"result comparison only; not a candidate source",
        "components_result_equal":true,"paths_equal":true,"path_counts_equal":true,
        "accepted_paths":baseline_result["path_counts"]["accepted"],
        "note":"28/84 为共同对照口径（章程 §1）；两口径收益分列见 two_accounting_gains"});
    recovery::write_json(
        &out,
        "c12-results.json",
        &json!({"schema":"laneflow-1136-c12-curve-design","source":source,
        "cohort_source":"research/issue-1136-c12-curve-design/edges.json",
        "method_source":"research/issue-1115-failed-curve-design/design.md",
        "segment_capacity":EDGES.map(|(id,max_curves)|json!({"edge":id,"reference_cubics":max_curves,
            "temp_program_points_max":PIECES_PER_CUBIC_MAX*max_curves+1})),
        "shared_baseline":{"source":"E0 package via SIX_STREAMS_BASELINE_DIR",
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
    recovery::write_json(&out, "c12-timing.json", &timing);
    println!("C12 curve design evidence saved={}", out.display());
}

#[test]
fn c12_cohort_and_capacities_match_the_frozen_plan() {
    let cohort: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1136-c12-curve-design/edges.json"
    ))
    .unwrap();
    assert_eq!(cohort["lane"], "A");
    let ids = cohort["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| edge["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids, EDGES.map(|(id, _)| id).to_vec());
    // plan §6：s=4 → 临时程序点上限 385 = 96s+1。
    assert_eq!(EDGES.map(|(_, s)| PIECES_PER_CUBIC_MAX * s + 1), [385]);
    for row in cohort["components"].as_array().unwrap() {
        assert_eq!(row["code"], "C12");
        assert_eq!(row["id"], C12_COMPONENT);
        assert_eq!(row["failed_edges"], json!(C12_MISSING_EDGES));
        assert_eq!(row["path_count"], 1);
    }
    let edge = &cohort["edges"][0];
    assert_eq!(edge["segment_capacity"]["reference_cubics"], 4);
    assert_eq!(edge["segment_capacity"]["temp_program_points_max"], 385);
    // plan §1：span 0 末弦局部见证 t=0.5 只是局部问题存在性记录。
    assert_eq!(
        edge["entry_basis"]["last_chord"]["witness_t_f64_bits"],
        json!(0.5_f64.to_bits())
    );
    assert_eq!(
        edge["entry_basis"]["first_chord"]["status"],
        "necessary_condition_excluded"
    );
    assert_eq!(edge["entry_basis"]["last_chord"]["status"], "local_witness");
}

#[test]
fn c12_fillet_seam_crossing_tracks_group_boundaries() {
    // s=4：reference/keep 保留全部分界不跨缝；pair/whole 合并 span 0/1 即跨规定倒圆接缝。
    assert!(!crosses_prescribed_fillet_seam("reference", 4));
    assert!(!crosses_prescribed_fillet_seam("keep", 4));
    assert!(crosses_prescribed_fillet_seam("pair", 4));
    assert!(crosses_prescribed_fillet_seam("whole", 4));
    assert!(!crosses_prescribed_fillet_seam("keep", 2));
    assert!(crosses_prescribed_fillet_seam("whole", 2));
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
