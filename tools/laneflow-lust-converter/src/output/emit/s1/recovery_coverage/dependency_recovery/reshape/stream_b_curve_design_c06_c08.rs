//! 流 B（#1130）：C06+C08 失败边曲线/分段设计变量研究入口。
//!
//! 方法口径逐值沿用 `research/issue-1115-failed-curve-design/design.md` 与
//! `crate::output::geom::s1::reshape`（每边 148 个预定义几何配置、回折拒绝、
//! 双向来源拟合筛选、n∈2..96 双划分族、全局保留 8），不重定义口径；样本、
//! 段容量与流 B 特有约束见冻结计划 `research/issue-1130-curve-design-c06-c08/plan.md`。
//! 模块名带流名，避免与流 A（#1131）入口冲突。

use super::*;

/// 流 B 四条失败边（冻结计划 §6）：`(边, 原参考链 cubic 数 s, 临时程序点上限 96s+1)`。
const STREAM_B_EDGES: [(&str, usize, usize); 4] = [
    ("sumo::-10004_3_0", 3, 289),
    ("sumo::-10004_8_0", 4, 385),
    ("sumo:--30306#2_0", 9, 865),
    ("sumo::-8806_4_0", 5, 481),
];

/// C06 两条边的「跨倒圆切点」记录对象（冻结计划 §2 流 B 特有①）：相关内部 span
/// （0 基 span 1）两端为规定倒圆切向，原内部倒圆切点即原参考链 cubic 间结点 1 与 2。
const C06_EDGES: [&str; 2] = ["sumo::-10004_3_0", "sumo::-10004_8_0"];
const C06_FILLET_KNOTS: [usize; 2] = [1, 2];

/// C06/C08 分量与完整路径（冻结计划 §1；`research/issue-1130-.../edges.json`）。
const C06_COMPONENT: &str = "sumo:-10004:-30608#6_2-to--31916_0::-10004_3_0.:-10004_8_0";
const C06_PATHS: [&str; 1] = ["sumo:-10004:-30608#6_2-to--31916_0::-10004_3_0.:-10004_8_0"];
const C08_COMPONENT: &str = "sumo:-10034:--30306#2_0-to---30306#1_0::-10034_0_0";
const C08_PATHS: [&str; 4] = [
    "sumo:-10034:--30306#2_0-to---30306#1_0::-10034_0_0",
    "sumo:-10034:--30306#2_0-to---30592#6_0::-10034_1_0",
    "sumo:-8806:--30306#3_0-to---30306#2_0::-8806_1_0",
    "sumo:-8806:-31086_0-to---30306#2_0::-8806_4_0",
];

/// C08 的既有候选边（冻结计划 §5：组合必须包含其既有 8 候选）。
const C08_EXISTING_CANDIDATE_EDGE: &str = "sumo::-10034_0_0";

/// 基线候选池（冻结计划 §5）四来源合并后的边集与每边份数（规范 f32 点位去重）。
const BASELINE_POOL_COUNTS: [(&str, usize); 9] = [
    ("sumo::-10004_5_1", 16),
    ("sumo::-10034_0_0", 8),
    ("sumo::-10130_16_0", 8),
    ("sumo::-10130_18_0", 8),
    ("sumo::-10130_8_0", 8),
    ("sumo::-16502_0_0", 1),
    ("sumo::-28860_3_0", 2),
    ("sumo::-29388_3_0", 8),
    ("sumo::-7348_3_0", 16),
];

fn frozen_stream_b_inputs() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1130-curve-design-c06-c08/inputs.json"
    ))
    .unwrap()
}

/// 合并组跨过的原参考链内部结点（0 基：结点 k 位于 cubic k-1 与 k 之间）。
/// 组覆盖 cubic 区间 `[start, end)` 时，组内结点 `start+1..end` 被合段跨过；
/// 合并组的段数只减不增（冻结计划 §6）。
fn crossed_internal_knots(group_size: usize, cubics: usize) -> Vec<usize> {
    assert!(group_size >= 1 && cubics >= 1);
    let mut knots = Vec::new();
    for start in (0..cubics).step_by(group_size) {
        let end = (start + group_size).min(cubics);
        knots.extend((start + 1)..end);
    }
    knots
}

fn merged_group_size(group: &str, cubics: usize) -> usize {
    match group {
        "reference" | "keep" => 1,
        "pair" => 2,
        "whole" => cubics,
        other => panic!("unknown geometry group: {other}"),
    }
}

/// 逐候选/逐几何「跨倒圆切点」记录（冻结计划 §2 流 B 特有①）。该标记只记录几何
/// 事实（合段是否跨过原内部倒圆切点），不表示跨倒圆切点的合段优于纯采样优化。
fn fillet_crossing_record(group: &str, cubics: usize) -> Value {
    let group_size = merged_group_size(group, cubics);
    let crossed = crossed_internal_knots(group_size, cubics);
    let crossed_fillet = C06_FILLET_KNOTS
        .iter()
        .copied()
        .filter(|k| crossed.contains(k))
        .collect::<Vec<_>>();
    json!({"group":group,"merged_group_size":group_size,
        "crossed_internal_knots":crossed,
        "crossed_fillet_tangent_knots":crossed_fillet,
        "crosses_fillet_tangent_point":!crossed_fillet.is_empty()})
}

/// 四条边的有限几何设计变量候选（冻结计划 §3）与流 B 特有记录。返回
/// （逐边搜索结果，逐边搜索耗时）；耗时只进 `timing.json`，不进结果 JSON。
fn run_stream_b_searches(inputs: &Value) -> (Value, Value) {
    let mut searches = BTreeMap::new();
    let mut timing = BTreeMap::new();
    for (id, s, cap) in STREAM_B_EDGES {
        assert_eq!(96 * s + 1, cap, "frozen temp program point cap");
        let input = measurement::input(&inputs[id]);
        let started = Instant::now();
        let search = sampler::reshape::search_with_curve_cap(&input, s);
        timing.insert(id, started.elapsed().as_secs_f64());
        assert_eq!(
            search.report["reference_controls_f64_bits"]
                .as_array()
                .unwrap()
                .len(),
            s,
            "frozen reference cubic count"
        );
        for candidate in &search.candidates {
            assert!(
                program_points(&candidate.repaired.program).len() <= cap,
                "frozen temp program point cap"
            );
        }
        let mut row = json!({"search":search.report,"candidates":recovery::candidates(&search),
            "frozen_segment_capacity":{"reference_cubics_s":s,"temp_program_points_cap":cap,
            "geometry_configurations_frozen":148}});
        if C06_EDGES.contains(&id) {
            let per_geometry = search.report["geometries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| {
                    let mut record =
                        fillet_crossing_record(row["description"]["group"].as_str().unwrap(), s);
                    record["geometry"] = row["geometry"].clone();
                    record
                })
                .collect::<Vec<_>>();
            let per_candidate = search
                .candidates
                .iter()
                .enumerate()
                .map(|(index, candidate)| {
                    let geometry = candidate.spans[0]["geometry"].as_u64().unwrap();
                    let mut record = per_geometry
                        .iter()
                        .find(|record| record["geometry"].as_u64().unwrap() == geometry)
                        .expect("visited geometry record")
                        .clone();
                    record["candidate_index"] = json!(index);
                    record
                })
                .collect::<Vec<_>>();
            row["fillet_tangent_crossings"] = json!({"definition":"合段跨过原参考链内部倒圆切点（相关内部 span 1 两端结点 1/2）即为 true；只记录几何事实，不表示跨倒圆切点的合段优于纯采样优化",
                "fillet_tangent_knots":C06_FILLET_KNOTS,
                "per_geometry":per_geometry,
                "per_candidate":per_candidate});
        }
        searches.insert(id, row);
    }
    (json!(searches), json!(timing))
}

/// 流 B 四条边的有限几何设计变量候选（冻结输入重放入口）。输出
/// `stream-b-searches.json` 作为 `CURVE_DESIGN_FROZEN_RESULT` 冻结对照结果。
#[test]
#[ignore = "requires fresh CURVE_DESIGN_OUTPUT_DIR"]
fn fixed_stream_b_curve_design() {
    let out = PathBuf::from(std::env::var("CURVE_DESIGN_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh stream B curve design output");
    fs::create_dir_all(&out).unwrap();
    let (searches, timing) = run_stream_b_searches(&frozen_stream_b_inputs());
    recovery::write_json(&out, "stream-b-searches.json", &searches);
    recovery::write_json(&out, "timing.json", &timing);
}

fn pool_size(pools: &combinations::Pools) -> (usize, usize) {
    (pools.len(), pools.values().map(Vec::len).sum())
}

/// 基线池冻结计数断言（冻结计划 §5：边集与每边份数）。
fn assert_baseline_pool_counts(pools: &combinations::Pools) {
    assert_eq!(pool_size(pools), (9, 75), "frozen baseline pool size");
    let actual = pools
        .iter()
        .map(|(id, rows)| (id.as_str(), rows.len()))
        .collect::<Vec<_>>();
    assert_eq!(actual, BASELINE_POOL_COUNTS, "frozen baseline pool counts");
}

/// 在该分量选中赋值下逐条完整路径真实编译（C06 单路径；C08 四条完整路径）。
fn complete_path_checks(
    traffic: &TrafficPackage,
    paths: &[&str],
    originals: &BTreeMap<String, re::RoadEditingCurveProgram>,
    pools: &combinations::Pools,
    result: &Value,
) -> Value {
    let mut programs = originals.clone();
    if result["status"] == "accepted" {
        for (id, choice) in result["selected_assignment"].as_object().unwrap() {
            programs.insert(
                id.clone(),
                pools[id][choice.as_u64().unwrap() as usize].clone(),
            );
        }
    }
    paths
        .iter()
        .map(|id| {
            let path = traffic
                .maneuver_paths
                .iter()
                .find(|path| path.id == *id)
                .expect("complete path identity");
            json!({"path":path.id,"result":compile_path(traffic, path, &programs)})
        })
        .collect()
}

/// 分量联合赋值记录（冻结计划 §5）：256 次上限、`assignment_budget_stopped` 与
/// 未访问赋值数（= 总赋值 − 已试次数）；未进入赋值时如实记录 `missing_real_programs`。
fn component_record(rows: &Value, component_id: &str, paths: &[&str], checks: Value) -> Value {
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["component"] == component_id)
        .expect("component row");
    let result = &row["result"];
    let total = result["total_assignments"].as_u64();
    let tried = result["trials"].as_array().map(Vec::len).unwrap_or(0) as u64;
    json!({"component":component_id,"complete_paths":paths,
        "status":result["status"],
        "missing_edges":result["missing_edges"].clone(),
        "assignment_budget_stopped":result["assignment_budget_stopped"].clone(),
        "retained_pool_assignments_exhausted":result["retained_pool_assignments_exhausted"],
        "total_assignments":total,
        "tried_assignments":tried,
        "unvisited_assignments":total.map(|total| total - tried),
        "selected_assignment":result["selected_assignment"].clone(),
        "complete_path_checks":checks})
}

/// 长度报告（冻结计划 §4）：来源 `@length`、来源 shape 折线长、原/新参考弧长
/// （4096 分段弦长与控制多边形估计）、规范 f32 弦长累计、compiler `length_mm`
/// 及差值。失败边新规范长度不声称保持不变；未测项如实为 `null`。
fn length_report(traffic: &TrafficPackage, searches: &Value, checks: &[Value]) -> Value {
    let edges = STREAM_B_EDGES
        .iter()
        .map(|(id, ..)| {
            let source = traffic
                .lane_graph
                .edges
                .iter()
                .find(|edge| edge.id == *id)
                .expect("stream B edge in lane graph");
            let search = &searches[id]["search"];
            let candidates = searches[id]["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(index, candidate)| {
                    let compiled = checks
                        .iter()
                        .flat_map(|check| {
                            check["result"]["edges"]
                                .as_array()
                                .cloned()
                                .unwrap_or_default()
                        })
                        .find(|row| row["edge"] == *id);
                    json!({
                        "candidate_index":index,
                        "new_reference":candidate["spans"][0]["reference_length_estimates"],
                        "canonical_chord_sum_m":candidate["arc_m"],
                        "canonical_source_fit":candidate["spans"][0]["canonical_source_fit"],
                        "compiler_length_mm":compiled.as_ref()
                            .and_then(|row| row["length_mm"].as_u64()),
                        "compiler_source_length_delta_m":compiled.as_ref()
                            .and_then(|row| row["source_length_delta_m"].as_f64()),
                    })
                })
                .collect::<Vec<_>>();
            json!({"edge":id,
                "source_at_length_m":source.length,
                "source_shape_polyline_m":search["source_shape_polyline_length_m"],
                "old_reference":search["reference_length_estimates"],
                "retained_candidates":candidates})
        })
        .collect::<Vec<_>>();
    json!(edges)
}

/// 逐边搜索摘要（拒绝阶段/尝试数/成功 n；冻结计划 §6 停止原因分列）。
fn edge_search_summary(searches: &Value) -> Value {
    STREAM_B_EDGES
        .iter()
        .map(|(id, ..)| {
            let search = &searches[id]["search"];
            let mut statuses = BTreeMap::<String, u64>::new();
            let mut successful_n = Vec::new();
            for row in search["geometries"].as_array().unwrap() {
                *statuses
                    .entry(row["status"].as_str().unwrap().to_owned())
                    .or_default() += 1;
                for success in row["successful_partitions"].as_array().unwrap() {
                    successful_n.push(
                        json!({"geometry":row["geometry"],"family":success["family"],"n":success["n"]}),
                    );
                }
            }
            json!({"edge":id,
                "stop_reason":search["stop_reason"],
                "geometry_configurations_visited":search["geometries"].as_array().unwrap().len(),
                "geometry_status_counts":statuses,
                "partition_attempts":search["counts"]["partitions"],
                "successful_partitions":successful_n,
                "candidate_programs":search["candidate_programs"],
                "counts":search["counts"]})
        })
        .collect()
}

/// 流 B 完整实验：全网捕获回归 + 四边设计变量候选 + 基线池四来源合并（结果对照
/// #1115 `variants.new`）+ 基线/新增两组分量与路径验收。`CURVE_DESIGN_ORDER` 为
/// ABBA 顺序中的一个取值；四轮非计时输出的逐字节比较在证据收集中执行。
#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, DIRECT_SUCCESSOR_BASELINE_DIR, CURVE_DESIGN_FROZEN_RESULT, CURVE_DESIGN_ORDER and fresh CURVE_DESIGN_OUTPUT_DIR"]
fn pinned_lust_stream_b_curve_experiment() {
    let out = PathBuf::from(std::env::var("CURVE_DESIGN_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh full stream B curve design output");
    fs::create_dir_all(&out).unwrap();
    let order = std::env::var("CURVE_DESIGN_ORDER").unwrap();
    assert!(matches!(order.as_str(), "baseline-new" | "new-baseline"));
    let frozen: Value = serde_json::from_slice(
        &fs::read(std::env::var("CURVE_DESIGN_FROZEN_RESULT").unwrap()).unwrap(),
    )
    .unwrap();
    let results_1115: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1115-failed-curve-design/results.json"
    ))
    .unwrap();
    let inventory: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/results.json"
    ))
    .unwrap();
    let (observation, source, mut timing) = capture_lust();
    assert_eq!(source, results_1115["source"], "frozen pinned source");
    let originals = original_programs(&observation);
    snapshot(&out, &observation, &originals);
    let traffic = observation.traffic.as_ref().unwrap();
    let components = components::inventory(traffic, &originals, &target_ids(&inventory));
    assert_eq!(
        json!(components),
        inventory["inventory"]["components"],
        "frozen component inventory"
    );
    assert_eq!(components.len(), 53, "frozen component count");
    assert_eq!(
        components.iter().map(|c| c.paths.len()).sum::<usize>(),
        183,
        "frozen component path total"
    );
    let c06 = components
        .iter()
        .find(|c| c.id == C06_COMPONENT)
        .expect("C06 component");
    assert_eq!(c06.paths, C06_PATHS, "C06 complete paths");
    let c08 = components
        .iter()
        .find(|c| c.id == C08_COMPONENT)
        .expect("C08 component");
    assert_eq!(c08.paths, C08_PATHS, "C08 complete paths");
    measurement::write_inputs(
        &out,
        &observation,
        &json!({"samples":STREAM_B_EDGES.map(|(id, _, _)| json!({"edge":id}))}),
    );
    let inputs: Value =
        serde_json::from_slice(&fs::read(out.join("inputs.json")).unwrap()).unwrap();
    let frozen_inputs = frozen_stream_b_inputs();
    for (id, ..) in STREAM_B_EDGES {
        assert_eq!(
            inputs[id], frozen_inputs[id],
            "unchanged source and emission failure"
        );
        assert!(!originals.contains_key(id), "only failed edges redesigned");
    }
    let (searches, search_times) = run_stream_b_searches(&inputs);
    assert_eq!(
        normalized(&searches),
        frozen,
        "frozen stream B candidate replay"
    );
    recovery::write_json(&out, "stream-b-searches.json", &searches);

    // 基线候选池（冻结计划 §5）：①→④ 固定顺序合并，规范 f32 点位去重，逐段断言。
    let legacy: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1103-s1-recovery-budget/results.json"
    ))
    .unwrap();
    let old: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/old-searches.json"
    ))
    .unwrap();
    let positive: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1111-grid-calibration/positive-calibration.json"
    ))
    .unwrap();
    let grid: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1111-grid-calibration/grid-searches.json"
    ))
    .unwrap();
    let curve: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1115-failed-curve-design/searches.json"
    ))
    .unwrap();
    for (id, row) in positive["edges"].as_object().unwrap() {
        assert_eq!(
            row["candidates"], grid[id]["candidates"],
            "calibration groups equal grid searches"
        );
    }
    assert_eq!(
        positive["edges"].as_object().unwrap().len(),
        2,
        "two calibration groups"
    );
    let mut baseline = pool(&legacy["tiers"]["32768"]["searches"]);
    assert_eq!(pool_size(&baseline), (6, 48), "frozen baseline source 1");
    merge(&mut baseline, pool(&old));
    assert_eq!(pool_size(&baseline), (7, 56), "frozen baseline source 2");
    merge(&mut baseline, pool(&positive["edges"]));
    assert_eq!(pool_size(&baseline), (7, 72), "frozen baseline source 3");
    merge(&mut baseline, pool(&curve));
    assert_baseline_pool_counts(&baseline);
    let mut combined = baseline.clone();
    merge(&mut combined, pool(&searches));
    // C08 组合必须包含 sumo::-10034_0_0 既有 8 候选（冻结计划 §5）。
    assert_eq!(
        combined[C08_EXISTING_CANDIDATE_EDGE].len(),
        8,
        "existing candidates retained"
    );
    for (old_program, new_program) in baseline[C08_EXISTING_CANDIDATE_EDGE]
        .iter()
        .zip(&combined[C08_EXISTING_CANDIDATE_EDGE])
    {
        assert_eq!(
            program_bits(old_program),
            program_bits(new_program),
            "unchanged existing candidate pool"
        );
    }

    // 分量级联合赋值与完整回归（基线/新增两组；结果对照另行执行）。
    let mut results = BTreeMap::new();
    let mut validation_times = BTreeMap::new();
    let mut checks_by_family = BTreeMap::new();
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
                .any(|id| STREAM_B_EDGES.iter().any(|(edge, ..)| edge == id))
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
            if result["status"] == "accepted" {
                for (id, choice) in result["selected_assignment"].as_object().unwrap() {
                    programs.insert(
                        id.clone(),
                        pools[id][choice.as_u64().unwrap() as usize].clone(),
                    );
                }
            }
            rows.push(json!({"component":component.id,"result":result,
                "artifact":(result["status"] == "accepted").then_some(file)}));
        }
        let paths = inventory["baseline_paths"].as_array().unwrap().iter().map(|row| {
            let path=traffic.maneuver_paths.iter().find(|p|p.id==row["path"].as_str().unwrap()).unwrap();
            json!({"path":path.id,"edges":path_edges(path),"result":compile_path(traffic,path,&programs)})
        }).collect::<Vec<_>>();
        let mut counts = BTreeMap::<String, usize>::new();
        for row in &paths {
            *counts
                .entry(row["result"]["status"].as_str().unwrap().to_owned())
                .or_default() += 1;
        }
        let c06_result = &rows
            .iter()
            .find(|row| row["component"] == C06_COMPONENT)
            .expect("C06 row")["result"];
        let c08_result = &rows
            .iter()
            .find(|row| row["component"] == C08_COMPONENT)
            .expect("C08 row")["result"];
        let checks = vec![
            complete_path_checks(traffic, &C06_PATHS, &originals, pools, c06_result),
            complete_path_checks(traffic, &C08_PATHS, &originals, pools, c08_result),
        ];
        checks_by_family.insert(family, checks);
        results.insert(
            family,
            json!({"components":rows,"paths":paths,"path_counts":counts}),
        );
        validation_times.insert(family, started.elapsed().as_secs_f64());
    }

    // 结果对照（冻结计划 §5）：基线重新验证的分量、路径与计数与 #1115
    // `results.json` `variants.new` 逐值比较（`variants.new` 不作候选点位来源）。
    let baseline_results = &results["baseline"];
    let frozen_new = &results_1115["variants"]["new"];
    let frozen_components = frozen_new["components"].as_array().unwrap();
    let baseline_components = baseline_results["components"].as_array().unwrap();
    assert_eq!(baseline_components.len(), frozen_components.len());
    for (current, previous) in baseline_components.iter().zip(frozen_components) {
        assert_eq!(current["component"], previous["component"]);
        assert_eq!(
            normalized(&current["result"]),
            previous["result"],
            "baseline component equals variants.new"
        );
    }
    assert_eq!(
        normalized(&baseline_results["paths"]),
        frozen_new["paths"],
        "baseline paths equal variants.new"
    );
    assert_eq!(
        normalized(&baseline_results["path_counts"]),
        frozen_new["path_counts"],
        "baseline counts equal variants.new"
    );
    assert_eq!(
        baseline_results["path_counts"]["accepted"].as_u64(),
        Some(28),
        "frozen accepted total 28/84"
    );

    // 流 B 分量级验收记录（C06 单路径；C08 四条完整路径 + 既有 8 候选组合）。
    let mut component_records = BTreeMap::new();
    for (family, results) in &results {
        let checks = &checks_by_family[*family];
        let c06_record = component_record(
            &results["components"],
            C06_COMPONENT,
            &C06_PATHS,
            checks[0].clone(),
        );
        let c08_record = component_record(
            &results["components"],
            C08_COMPONENT,
            &C08_PATHS,
            checks[1].clone(),
        );
        for record in [&c06_record, &c08_record] {
            if let Some(total) = record["total_assignments"].as_u64()
                && total > 256
            {
                assert_eq!(
                    record["assignment_budget_stopped"],
                    json!(true),
                    "assignment budget stop recorded"
                );
                assert_eq!(record["tried_assignments"], json!(256));
                assert_eq!(
                    record["unvisited_assignments"],
                    json!(total - 256),
                    "unvisited assignments recorded"
                );
            }
        }
        component_records.insert(*family, json!({"c06":c06_record,"c08":c08_record}));
    }

    // 本轮新增收益单独统计（不并入基线）。
    let new_components = results["new"]["components"].as_array().unwrap();
    let gained_components = new_components
        .iter()
        .zip(baseline_components)
        .filter(|(current, previous)| {
            current["result"]["status"] == "accepted" && previous["result"]["status"] != "accepted"
        })
        .map(|(current, _)| current["component"].clone())
        .collect::<Vec<_>>();
    let new_gains = json!({
        "added_candidates":pool_size(&combined).1 - pool_size(&baseline).1,
        "pool_edges_after":pool_size(&combined).0,
        "candidates_after":pool_size(&combined).1,
        "newly_accepted_components":gained_components,
        "new_path_counts":results["new"]["path_counts"],
    });

    let mut policy = sampler::reshape::policy();
    policy["reference_cubics_max_per_edge"] = json!(STREAM_B_EDGES
        .iter()
        .map(|(id, s, cap)| json!({"edge":id,"reference_cubics_s":s,"temp_program_points_cap":cap}))
        .collect::<Vec<_>>());
    timing["search_seconds"] = search_times;
    timing["validation_seconds"] = json!(validation_times);
    timing["order"] = json!(order);
    recovery::write_json(
        &out,
        "stream-b-results.json",
        &json!({"schema":"laneflow-1130-stream-b-curve-design-c06-c08",
        "source":source,"policy":policy,"variants":results,
        "stream_b":{
            "frozen_segment_capacities":STREAM_B_EDGES.map(|(id, s, cap)|
                json!({"edge":id,"reference_cubics_s":s,"temp_program_points_cap":cap})).to_vec(),
            "baseline_pool":{"merge_order":"issue-1103 results.json -> issue-1107 old-searches.json -> issue-1111 positive-calibration.json edges -> issue-1115 searches.json",
             "per_edge":BASELINE_POOL_COUNTS.iter().map(|(id, count)| json!({"edge":id,"candidates":count})).collect::<Vec<_>>(),
             "edges":pool_size(&baseline).0,"candidates":pool_size(&baseline).1},
            "new_pool":{"edges":pool_size(&combined).0,"candidates":pool_size(&combined).1},
            "edge_search_summary":edge_search_summary(&searches),
            "component_records":component_records,
            "length_report":length_report(traffic, &searches, &checks_by_family["new"]),
            "new_variant_gains":new_gains},
        "result_comparison":{"baseline_equals_variants_new":true,
            "variants_new_role":"result comparison only; not a candidate source",
            "accepted_paths_baseline":"28/84",
            "round1_no_curve_pool_reference":"26/84 (issue-1121 pool without curve candidates), noted for reference only"},
        "original_geometry_controls_and_failures_equal":true,
        "production_g1_accepted":false}),
    );
    recovery::write_json(&out, "timing.json", &timing);
}

#[test]
fn frozen_segment_capacity_matches_plan() {
    assert_eq!(
        STREAM_B_EDGES.map(|(_, s, cap)| (s, cap)),
        [(3, 289), (4, 385), (9, 865), (5, 481)]
    );
    for (.., s, cap) in STREAM_B_EDGES {
        assert_eq!(96 * s + 1, cap);
    }
}

#[test]
fn merged_segments_record_fillet_tangent_crossing() {
    // C06 s=3：倒圆切点为结点 1/2；合段跨过其一即为 true。
    assert_eq!(crossed_internal_knots(1, 3), Vec::<usize>::new());
    assert_eq!(crossed_internal_knots(2, 3), vec![1]);
    assert_eq!(crossed_internal_knots(3, 3), vec![1, 2]);
    assert!(
        !fillet_crossing_record("keep", 3)["crosses_fillet_tangent_point"]
            .as_bool()
            .unwrap()
    );
    assert!(
        fillet_crossing_record("pair", 3)["crosses_fillet_tangent_point"]
            .as_bool()
            .unwrap()
    );
    assert!(
        fillet_crossing_record("whole", 3)["crosses_fillet_tangent_point"]
            .as_bool()
            .unwrap()
    );
    // C06 s=4：结点 3 是内部结点但不是冻结的倒圆切点，不计入标记。
    let pair = fillet_crossing_record("pair", 4);
    assert_eq!(pair["crossed_internal_knots"], json!([1, 3]));
    assert_eq!(pair["crossed_fillet_tangent_knots"], json!([1]));
    assert!(pair["crosses_fillet_tangent_point"].as_bool().unwrap());
    let whole = fillet_crossing_record("whole", 4);
    assert_eq!(whole["crossed_fillet_tangent_knots"], json!([1, 2]));
    assert_eq!(whole["merged_group_size"], json!(4));
}

#[test]
fn baseline_pool_merges_sources_in_frozen_order() {
    let legacy: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1103-s1-recovery-budget/results.json"
    ))
    .unwrap();
    let old: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/old-searches.json"
    ))
    .unwrap();
    let positive: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1111-grid-calibration/positive-calibration.json"
    ))
    .unwrap();
    let grid: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1111-grid-calibration/grid-searches.json"
    ))
    .unwrap();
    let curve: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1115-failed-curve-design/searches.json"
    ))
    .unwrap();
    for (id, row) in positive["edges"].as_object().unwrap() {
        assert_eq!(row["candidates"], grid[id]["candidates"]);
    }
    assert_eq!(positive["edges"].as_object().unwrap().len(), 2);
    let mut baseline = pool(&legacy["tiers"]["32768"]["searches"]);
    assert_eq!(pool_size(&baseline), (6, 48));
    merge(&mut baseline, pool(&old));
    assert_eq!(pool_size(&baseline), (7, 56));
    merge(&mut baseline, pool(&positive["edges"]));
    assert_eq!(pool_size(&baseline), (7, 72));
    merge(&mut baseline, pool(&curve));
    assert_baseline_pool_counts(&baseline);
}

#[test]
fn default_curve_cap_keeps_the_1115_policy() {
    assert_eq!(
        sampler::reshape::policy()["reference_cubics_max"],
        json!(6),
        "default policy unchanged"
    );
    assert_eq!(
        sampler::reshape::policy_with_curve_cap(9)["reference_cubics_max"],
        json!(9)
    );
}
