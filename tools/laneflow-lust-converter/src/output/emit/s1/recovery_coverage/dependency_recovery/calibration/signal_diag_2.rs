//! #1139：信号诊断组 2（C19/C22/C24）的固定曲线覆盖补齐、逐边局部必要条件诊断、
//! 候选齐备分量组合验收与停止线三情形核对。执行口径全部冻结于
//! `research/issue-1139-signal-diag-2/plan.md`，样本与问题表冻结于同目录 `edges.json`；
//! 方法事实来源 `research/issue-1121-coverage-diagnosis/plan.md` 逐值沿用，实现模板为
//! `research/issue-1134-cohort2-coverage-diagnosis/`。本模块不改生产采样、compiler
//! 配置档、阈值与原成功几何。计划与冻结实现不一致时停止执行并重新授权（plan §8），
//! 不得单向迁就。
//!
//! 基线接线（plan §6）：共同基线 = E0（Issue #1140）冻结交付的 11 边 92 份
//! （六来源按序合并、规范 f32 点位去重），结果对照锚为 #1134 新增变体（53 分量、
//! 84 路径计数）逐值。经 E0 冻结门面读取：env `SIX_STREAMS_BASELINE_DIR` +
//! `shared_baseline::shared_pools()` / `results_anchor_pinned()`（代码内哈希、
//! 合并顺序与逐边份数断言）；不得使用旧 9/10 边池或任何替代来源。

use super::*;
use crate::output::model::{ManeuverGate, SignalControl, StopLine};
use std::collections::BTreeSet;

/// plan §1/§7 冻结样本：30 条缺边按 `edges.json` 边序（C19 → C22 → C24），即阶段一
/// 搜索顺序。
const EDGES: [&str; 30] = [
    "sumo:--30358#12_0",
    "sumo:--30358#12_1",
    "sumo::-13188_4_0",
    "sumo::-5126_2_0",
    "sumo::-5126_3_0",
    "sumo::-5126_5_0",
    "sumo::-5126_8_0",
    "sumo:--30316_0",
    "sumo::-1542_0_0",
    "sumo::-1542_10_0",
    "sumo::-1542_6_0",
    "sumo::-1542_7_0",
    "sumo::-1542_8_0",
    "sumo::-1542_8_1",
    "sumo::-25286_10_0",
    "sumo::-25286_11_0",
    "sumo::-25286_17_0",
    "sumo::-25286_18_0",
    "sumo::-25286_2_0",
    "sumo::-25286_3_0",
    "sumo::-25286_7_0",
    "sumo::-25286_8_0",
    "sumo:--30266_0",
    "sumo::-17348_0_0",
    "sumo::-17348_5_0",
    "sumo::-17348_6_0",
    "sumo::-17348_7_0",
    "sumo::-17348_8_0",
    "sumo::-29244_1_0",
    "sumo::-29244_4_0",
];

/// plan §2：扩展档 262,144 仅这两条有 `1103-expanded` 历史记录的边执行；其余 28 边
/// 不跑扩展档（新增搜索量须另授权）。
const EXPANDED_TIER_EDGES: [&str; 2] = ["sumo:--30358#12_1", "sumo:--30316_0"];

fn cohort() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/issue-1139-edges.json"
    ))
    .unwrap()
}

/// `edges.json` 的单字符串 / 数组两种冻结形态统一为列表。
fn row_list(value: &Value) -> Vec<String> {
    match value {
        Value::String(item) => vec![item.clone()],
        Value::Array(items) => items
            .iter()
            .map(|item| item.as_str().unwrap().to_owned())
            .collect(),
        other => panic!("unexpected frozen list {other}"),
    }
}

fn row_paths(row: &Value) -> Vec<String> {
    row_list(&row["paths"])
}

/// plan §3 冻结问题生成规则：span 端点切向为边界钳制 → 端弦问题（方向锥 = 对应端
/// 钳制原 f64 位）；规定倒圆 → 端弦问题（不作方向条件）；interior CR → 该端不取。
fn endpoint_problem(tangent: &str, kind: &str, clamp_source: &str) -> Option<Value> {
    if tangent.contains("boundary clamp") {
        Some(json!({"kind": kind, "direction_source": clamp_source}))
    } else if tangent.contains("fillet arc tangent") {
        Some(json!({"kind": kind, "direction_source": "fillet_not_a_direction_condition"}))
    } else {
        assert!(
            tangent.contains("interior Catmull-Rom tangent"),
            "frozen tangent mechanism"
        );
        None
    }
}

/// plan §5 停止线三情形核对表：对冻结表 26 条停止线边逐边产出，每条恰好归入一情形
/// （原成功停止线边 / 本轮恢复的停止线边 / 未恢复）。情形 1 核对原程序与控制语义逐字节
/// 保持；情形 2 核对选中候选、规范端点、编译后长度、`edgeEnd` 解析与机动门绑定；
/// 情形 3 如实记录程序/编译检查未执行并保留来源绑定核对。本轮候选齐备即执行情形 2，
/// 不得留给后续设计任务。（#1154 见证引导部分复用同一核对口径，仅可见性放宽。）
pub(super) fn stop_line_checklist(
    cohort: &Value,
    traffic: &TrafficPackage,
    originals: &BTreeMap<String, re::RoadEditingCurveProgram>,
    programs: &BTreeMap<String, re::RoadEditingCurveProgram>,
    evaluations: &BTreeMap<String, Value>,
) -> Value {
    let edge_rows = cohort["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| (edge["id"].as_str().unwrap(), edge))
        .collect::<BTreeMap<_, _>>();
    // 冻结表 `stop_lines` 行以分量码（C19/C22/C24）标注归属；正式实验的评价表以完整
    // 分量 id 键入。fixture 冻结表无 `components` 段时按码直查。
    let code_ids = cohort
        .get("components")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    (
                        row["code"].as_str().unwrap().to_owned(),
                        row["id"].as_str().unwrap().to_owned(),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let evaluation_key = |code: &str| -> String {
        code_ids
            .get(code)
            .cloned()
            .unwrap_or_else(|| code.to_owned())
    };
    let gate_bindings = |stop_line_id: &str| -> Value {
        let gates = traffic
            .signals
            .maneuver_gates
            .iter()
            .filter(|gate| gate.stop_line_id == stop_line_id)
            .collect::<Vec<_>>();
        for gate in &gates {
            let path = traffic
                .maneuver_paths
                .iter()
                .find(|path| path.id == gate.maneuver_path_id)
                .expect("gate maneuver path in topology");
            let mut path_edges = vec![path.entry_edge_id.as_str()];
            path_edges.extend(path.internal_edge_ids.iter().map(String::as_str));
            path_edges.push(path.exit_edge_id.as_str());
            assert!(
                path_edges.contains(&edge_rows_edge_of(cohort, stop_line_id)),
                "机动门 {stop_line_id} 绑定路径穿越停止线来源边"
            );
        }
        json!(gates)
    };
    let mut missing_rows = Vec::new();
    let mut success_rows = Vec::new();
    for row in cohort["stop_lines"]["missing_programs"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        let edge = row["edge"].as_str().unwrap();
        let code = row["component"].as_str().unwrap();
        let declaration = traffic
            .signals
            .stop_lines
            .iter()
            .find(|stop| stop.id == id)
            .unwrap_or_else(|| panic!("stop line {id} declared in topology"));
        assert_eq!(
            declaration.edge_id, edge,
            "stop line {id} source edge binding"
        );
        assert_eq!(
            declaration.location, "edgeEnd",
            "frozen stop line location for {id}"
        );
        assert!(
            edge_rows.contains_key(edge),
            "stop line edge {edge} is a frozen missing edge"
        );
        let gates = gate_bindings(id);
        let edge_row = edge_rows[edge];
        let evaluation = evaluations
            .get(&evaluation_key(code))
            .expect("component evaluation present for stop line edge");
        let selected = evaluation
            .get("selected_assignment")
            .and_then(Value::as_object)
            .and_then(|assignment| assignment.get(edge));
        if let Some(choice) = selected {
            assert_eq!(
                evaluation["status"], "accepted",
                "selected candidate implies accepted component for {edge}"
            );
            let selected_result = &evaluation["selected_result"];
            assert_eq!(
                selected_result["status"], "accepted",
                "selected compile for {edge}"
            );
            let compile_row = selected_result["edges"]
                .as_array()
                .unwrap()
                .iter()
                .find(|compiled| compiled["edge"] == edge)
                .expect("compiled edge row for recovered stop line edge");
            assert_eq!(
                compile_row["all_points_match"], true,
                "canonical endpoints verified for {edge}"
            );
            let program = programs.get(edge).expect("recovered program present");
            let points = program_points(program);
            let anchors = selected_result
                .get("compiled_control_anchors")
                .cloned()
                .unwrap_or(Value::Null);
            missing_rows.push(json!({
                "case":"recovered_this_round","component":code,
                "stop_line":declaration,"gates":gates,
                "selected_candidate":{"edge":edge,"choice":choice,
                    "program":program_bits(program),
                    "canonical_endpoints":{"start":points[0],"end":points[points.len()-1]}},
                "compiled":{"length_mm":compile_row["length_mm"],
                    "canonical_arc_m":compile_row["canonical_arc_m"],
                    "source_length_m":compile_row["source_length_m"],
                    "source_length_delta_m":compile_row["source_length_delta_m"],
                    "all_points_match":compile_row["all_points_match"]},
                "edge_end_resolution":{"location":"edgeEnd","declared_on_edge":edge,
                    "compile_accepted":true,
                    "lfca_bytes":selected_result.get("lfca_bytes").cloned().unwrap_or(Value::Null)},
                "maneuver_gate_bindings":{"gates":gates,
                    "compiled_control_anchors":anchors},
                "note":"本轮候选齐备即执行：选中候选、规范端点、编译后长度、edgeEnd 解析、机动门绑定"}));
        } else {
            assert!(
                !programs.contains_key(edge),
                "not-recovered stop line edge {edge} has no program"
            );
            missing_rows.push(json!({
                "case":"not_recovered","component":code,
                "stop_line":declaration,"gates":gates,
                "program_compile_checks":"not_executed",
                "source_binding":{"declared_edge_id":declaration.edge_id,
                    "missing_edge_id":edge,"location":declaration.location,
                    "binding_consistent":declaration.edge_id == edge,
                    "all_dependency_users":edge_row["all_dependency_users"]},
                "note":"本轮未恢复：程序/编译检查未执行；保留来源绑定核对（停止线声明与来源边关系）"}));
        }
    }
    for row in cohort["stop_lines"]["original_success"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        let edge = row["edge"].as_str().unwrap();
        let code = row["component"].as_str().unwrap();
        let declaration = traffic
            .signals
            .stop_lines
            .iter()
            .find(|stop| stop.id == id)
            .unwrap_or_else(|| panic!("stop line {id} declared in topology"));
        assert_eq!(
            declaration.edge_id, edge,
            "stop line {id} source edge binding"
        );
        assert!(
            !edge_rows.contains_key(edge),
            "original-success stop line edge {edge} is not a missing edge"
        );
        let gates = gate_bindings(id);
        let original = originals.get(edge).expect("original successful program");
        let program = programs.get(edge).expect("unchanged program present");
        assert_eq!(
            program_bits(original),
            program_bits(program),
            "原成功停止线边程序逐字节保持 {edge}"
        );
        success_rows.push(json!({
            "case":"original_success","component":code,
            "stop_line":declaration,"gates":gates,
            "program_preserved":{"byte_equal_to_original":true,
                "program":program_bits(program)},
            "control_semantics":{"gates_unchanged":gates,
                "baseline_byte_comparison":"successful-programs.jsonl 与 #1105 基线全文件逐字节相等（全网捕获断言）"},
            "note":"原程序与控制语义保持（逐字节对照基线）"}));
    }
    let recovered = missing_rows
        .iter()
        .filter(|row| row["case"] == "recovered_this_round")
        .count();
    json!({"missing_programs":missing_rows,"original_success":success_rows,
        "cases_summary":{"original_success":success_rows.len(),
            "recovered_this_round":recovered,
            "not_recovered":missing_rows.len()-recovered},
        "note":"三情形缺一不可，每条停止线边恰好归入一情形；单/双横线边 ID（如 sumo:-30316_0 与 sumo:--30316_0）为不同边，按精确 ID 区分"})
}

/// 停止线声明的来源边（与冻结表 `stop_lines` 行的 `edge` 字段同源）。
fn edge_rows_edge_of<'a>(cohort: &'a Value, stop_line_id: &str) -> &'a str {
    for group in ["missing_programs", "original_success"] {
        for row in cohort["stop_lines"][group].as_array().unwrap() {
            if row["id"] == stop_line_id {
                return row["edge"].as_str().unwrap();
            }
        }
    }
    panic!("stop line {stop_line_id} not in frozen table");
}

#[test]
fn signal_diag_2_frozen_sample_and_problem_table_stay_consistent() {
    let cohort = cohort();
    assert_eq!(
        cohort["schema"],
        "laneflow-round4-signal-diag-2-coverage-diagnosis"
    );
    assert_eq!(
        cohort["method_source"],
        "research/issue-1121-coverage-diagnosis/plan.md"
    );
    assert_eq!(cohort["summary"]["components"].as_u64(), Some(3));
    assert_eq!(cohort["summary"]["missing_edges"].as_u64(), Some(30));
    assert_eq!(cohort["summary"]["complete_paths"].as_u64(), Some(47));
    assert_eq!(cohort["summary"]["original_targets"].as_u64(), Some(12));
    assert_eq!(cohort["summary"]["controllers"].as_u64(), Some(4));
    assert_eq!(
        cohort["summary"]["stop_line_edges_missing_programs"].as_u64(),
        Some(4)
    );
    assert_eq!(
        cohort["summary"]["stop_line_edges_original_success"].as_u64(),
        Some(22)
    );
    assert_eq!(cohort["summary"]["diagnosis_problems"].as_f64(), Some(36.0));
    assert_eq!(
        cohort["summary"]["endpoint_diagnosis_not_selected_edges"].as_u64(),
        Some(1)
    );
    assert_eq!(cohort["summary"]["edges_with_history"].as_u64(), Some(3));
    assert_eq!(cohort["summary"]["edges_first_search"].as_u64(), Some(27));
    assert_eq!(cohort["summary"]["expanded_tier_edges"].as_u64(), Some(2));
    let components = cohort["components"].as_array().unwrap();
    assert_eq!(
        components
            .iter()
            .map(|row| row["code"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["C19", "C22", "C24"],
        "frozen component code order"
    );
    assert_eq!(
        components
            .iter()
            .map(|row| row["path_count"].as_u64().unwrap() as usize)
            .sum::<usize>(),
        47
    );
    assert_eq!(
        components
            .iter()
            .map(|row| row["original_target_count"].as_u64().unwrap() as usize)
            .sum::<usize>(),
        12
    );
    for row in components {
        assert_eq!(
            row["path_count"].as_u64().unwrap() as usize,
            row_paths(row).len(),
            "path_count matches frozen paths"
        );
    }
    let mut owner = BTreeSet::new();
    for row in components {
        for id in row_list(&row["missing_edges"]) {
            assert!(owner.insert(id), "unique missing edge owner");
        }
    }
    assert_eq!(
        owner,
        EDGES
            .iter()
            .map(|id| (*id).to_owned())
            .collect::<BTreeSet<_>>(),
        "missing edge partition equals the frozen edge set"
    );
    assert_eq!(
        components
            .iter()
            .map(|row| row_list(&row["missing_edges"]).len())
            .collect::<Vec<_>>(),
        [7, 15, 8],
        "C19 七条 / C22 十五条 / C24 八条待赋值边逐值冻结"
    );
    let ids = cohort["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| edge["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids, EDGES, "frozen edge order equals search order");
    let mut problem_count = 0;
    for edge in cohort["edges"].as_array().unwrap() {
        let mut expected = Vec::new();
        expected.extend(endpoint_problem(
            edge["start_tangent"].as_str().unwrap(),
            "first_chord",
            "start_boundary_clamp",
        ));
        expected.extend(endpoint_problem(
            edge["finish_tangent"].as_str().unwrap(),
            "last_chord",
            "finish_boundary_clamp",
        ));
        assert_eq!(
            json!(expected),
            edge["problems"],
            "problem rule reproduces the frozen table for {}",
            edge["id"]
        );
        problem_count += expected.len();
        assert_eq!(
            edge["diagnosis_status"] == "selected",
            !expected.is_empty(),
            "diagnosis status follows the frozen rule"
        );
    }
    assert_eq!(problem_count, 36, "36 local problems");
    assert_eq!(
        cohort["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|edge| edge["diagnosis_status"] == "endpoint_diagnosis_not_selected")
            .map(|edge| edge["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["sumo::-17348_6_0"],
        "one endpoint-diagnosis-not-selected edge"
    );
    for id in EXPANDED_TIER_EDGES {
        assert!(EDGES.contains(&id), "expanded tier is a cohort subset");
    }
    assert_eq!(
        cohort["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|edge| {
                edge["history"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|entry| entry["experiment"] == "1103-expanded")
            })
            .map(|edge| edge["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        EXPANDED_TIER_EDGES,
        "expanded tier equals the 1103-expanded history edges"
    );
    let legacy: Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/issue-1103-s1-recovery-budget-results.json"
    ))
    .unwrap();
    let old_rows: Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/issue-1107-old-searches.json"
    ))
    .unwrap();
    let grid_history: Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/issue-1107-grid-searches.json"
    ))
    .unwrap();
    let mut first_run = 0;
    let mut with_history = 0;
    for edge in cohort["edges"].as_array().unwrap() {
        let id = edge["id"].as_str().unwrap();
        let history = edge["history"].as_array().unwrap();
        if history.is_empty() {
            first_run += 1;
            assert!(
                legacy["tiers"]["32768"]["searches"].get(id).is_none()
                    && legacy["tiers"]["262144"]["searches"].get(id).is_none()
                    && old_rows.get(id).is_none()
                    && grid_history.get(id).is_none(),
                "{id} has no recorded history search"
            );
        } else {
            with_history += 1;
        }
        for entry in history {
            let (record, pointer_tier) = match entry["experiment"].as_str().unwrap() {
                "1103-base" => (&legacy["tiers"]["32768"]["searches"][id], "32768"),
                "1103-expanded" => (&legacy["tiers"]["262144"]["searches"][id], "262144"),
                other => panic!("unknown history experiment {other}"),
            };
            assert!(!record.is_null(), "history record present for {id}");
            assert_eq!(
                record["search"]["stop_reason"], entry["stop_reason"],
                "history stop reason for {id}"
            );
            assert_eq!(
                json!(record["candidates"].as_array().unwrap().len()),
                entry["candidate_count"],
                "history candidate count for {id}"
            );
            assert_eq!(
                entry["file"], "research/issue-1103-s1-recovery-budget/results.json",
                "history source file for {id}"
            );
            assert_eq!(
                entry["json_pointer"],
                format!("/tiers/{pointer_tier}/searches/{id}"),
                "history json pointer for {id}"
            );
        }
    }
    assert_eq!(
        (first_run, with_history),
        (27, 3),
        "27 first-run edges and three with history"
    );
}

#[test]
fn signal_diag_2_stop_line_table_pins_exact_edge_ids() {
    let cohort = cohort();
    let missing = cohort["stop_lines"]["missing_programs"].as_array().unwrap();
    let success = cohort["stop_lines"]["original_success"].as_array().unwrap();
    assert_eq!(missing.len(), 4, "four missing-program stop line edges");
    assert_eq!(success.len(), 22, "twenty-two original-success stop lines");
    assert_eq!(
        missing
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "sumo:stop:--30358#12_0",
            "sumo:stop:--30358#12_1",
            "sumo:stop:--30316_0",
            "sumo:stop:--30266_0"
        ],
        "frozen missing-program stop line order"
    );
    for row in missing.iter().chain(success.iter()) {
        let id = row["id"].as_str().unwrap();
        let edge = row["edge"].as_str().unwrap();
        assert_eq!(
            id,
            format!("sumo:stop:{}", &edge["sumo:".len()..]),
            "stop id derives from edge id"
        );
        assert!(
            ["C19", "C22", "C24"].contains(&row["component"].as_str().unwrap()),
            "stop line owner is a cohort component"
        );
    }
    for row in missing {
        assert_eq!(
            row["location"], "edgeEnd",
            "missing stop lines resolve at edgeEnd"
        );
        assert!(
            EDGES.contains(&row["edge"].as_str().unwrap()),
            "missing stop line edge is a frozen missing edge"
        );
    }
    let frozen_edges = EDGES
        .iter()
        .map(|id| (*id).to_owned())
        .collect::<BTreeSet<_>>();
    for row in success {
        assert!(
            !frozen_edges.contains(row["edge"].as_str().unwrap()),
            "original-success stop line edge carries original programs"
        );
    }
    // 关键防御（统筹冻结记录）：单/双横线为不同边 ID，核对表按精确 ID 区分。
    let success_edges = success
        .iter()
        .map(|row| row["edge"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    let missing_edges = missing
        .iter()
        .map(|row| row["edge"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert!(
        success_edges.contains("sumo:-30316_0") && missing_edges.contains("sumo:--30316_0"),
        "single/double dash pair sumo:-30316_0 vs sumo:--30316_0 stays distinct"
    );
    assert!(
        success_edges.contains("sumo:-31496#1_0") && success_edges.contains("sumo:--31496#1_0"),
        "single/double dash pair sumo:-31496#1_0 vs sumo:--31496#1_0 stays distinct"
    );
    assert!(
        success_edges.contains("sumo:-32230#6_0"),
        "single dash edge sumo:-32230#6_0 stays distinct from missing edge ids"
    );
}

#[test]
fn signal_diag_2_stop_line_three_case_checklist_covers_each_case() {
    let observation = small_observation();
    let traffic = observation.traffic.as_ref().unwrap().clone();
    // 三条停止线边动态选取（互异优先道路边 entry/exit，其次内部边），各配一条穿越路径。
    let mut candidates: Vec<(String, String)> = Vec::new();
    for path in &traffic.maneuver_paths {
        candidates.push((path.entry_edge_id.clone(), path.id.clone()));
        candidates.push((path.exit_edge_id.clone(), path.id.clone()));
    }
    for path in &traffic.maneuver_paths {
        for internal in &path.internal_edge_ids {
            candidates.push((internal.clone(), path.id.clone()));
        }
    }
    let mut picked: Vec<(String, String)> = Vec::new();
    for (edge, path) in candidates {
        if picked.iter().all(|(seen, _)| seen != &edge) {
            picked.push((edge, path));
        }
        if picked.len() == 3 {
            break;
        }
    }
    assert_eq!(picked.len(), 3, "three distinct fixture edges");
    // 情形 1 = 原成功边（保留原程序）；情形 2 = 本轮恢复边（去掉原程序、以同一几何
    // 候选恢复）；情形 3 = 未恢复边（去掉原程序、无候选）。
    let (success_edge, success_path) = picked[0].clone();
    let (recovered_edge, recovered_path) = picked[1].clone();
    let (missing_edge, missing_path) = picked[2].clone();
    let mut fixture_programs = small_programs(&observation);
    let recovered_program = fixture_programs.remove(&recovered_edge).unwrap();
    fixture_programs.remove(&missing_edge).unwrap();
    let originals = fixture_programs.clone();
    let mut programs = fixture_programs.clone();
    programs.insert(recovered_edge.clone(), recovered_program.clone());
    // 冻结表形状（fixture 自带三行）：stop id 派生自边 id。
    let frozen = json!({
        "edges":[
            {"id":recovered_edge,"all_dependency_users":[recovered_path]},
            {"id":missing_edge,"all_dependency_users":[missing_path]}],
        "stop_lines":{
            "missing_programs":[
                {"id":format!("sumo:stop:{recovered_edge}"),"edge":recovered_edge,
                    "component":"fixture-recovered"},
                {"id":format!("sumo:stop:{missing_edge}"),"edge":missing_edge,
                    "component":"fixture-missing"}],
            "original_success":[
                {"id":format!("sumo:stop:{success_edge}"),"edge":success_edge,
                    "component":"fixture-success"}]}});
    // fixture 停止线/机动门补声明（克隆既有声明并改绑），控制语义形状不变。
    let mut patched = traffic.clone();
    let template_gate = patched.signals.maneuver_gates[0].clone();
    for (stop_id, edge, gate_path) in [
        (
            format!("sumo:stop:{recovered_edge}"),
            &recovered_edge,
            &recovered_path,
        ),
        (
            format!("sumo:stop:{missing_edge}"),
            &missing_edge,
            &missing_path,
        ),
        (
            format!("sumo:stop:{success_edge}"),
            &success_edge,
            &success_path,
        ),
    ] {
        patched.signals.stop_lines.push(StopLine {
            id: stop_id.clone(),
            edge_id: edge.clone(),
            location: "edgeEnd",
        });
        patched.signals.maneuver_gates.push(ManeuverGate {
            id: format!("sumo:gate:{stop_id}"),
            maneuver_path_id: gate_path.clone(),
            transition_index: template_gate.transition_index,
            stop_line_id: stop_id,
            signal_control: SignalControl {
                kind: template_gate.signal_control.kind,
                group_id: template_gate.signal_control.group_id.clone(),
            },
        });
    }
    // 评价行按 `combinations::evaluate` 结果形状合成：fixture 只测核对表分类、证据
    // 投影与绑定核对；真实证据由正式实验的联合编译产出，其正确性由 compile_part
    // 内部断言（完整规范程序点位、编译后长度、控制锚点）保证。
    let mut recovered_evaluation = json!({"status":"accepted","selected_result":{
        "status":"accepted","lfca_bytes":4096_u64,
        "edges":[{"edge":recovered_edge,"length_mm":12_345_i64,
            "canonical_arc_m":12.345_f64,"source_length_m":12.0_f64,
            "source_length_delta_m":0.345_f64,"all_points_match":true}]}});
    recovered_evaluation["selected_assignment"][recovered_edge.as_str()] = json!(0);
    let evaluations = BTreeMap::from([
        ("fixture-recovered".to_owned(), recovered_evaluation),
        (
            "fixture-missing".to_owned(),
            json!({"status":"missing_real_programs",
                "missing_edges":[missing_edge],"trials":[]}),
        ),
    ]);
    assert_eq!(
        evaluations["fixture-recovered"]["status"], "accepted",
        "recovered fixture component accepts its candidate"
    );
    assert_eq!(
        evaluations["fixture-missing"]["status"],
        "missing_real_programs"
    );
    let checklist = stop_line_checklist(&frozen, &patched, &originals, &programs, &evaluations);
    assert_eq!(
        checklist["cases_summary"],
        json!({"original_success":1,"recovered_this_round":1,"not_recovered":1}),
        "three cases each covered exactly once"
    );
    let recovered_row = &checklist["missing_programs"][0];
    assert_eq!(recovered_row["case"], "recovered_this_round");
    assert_eq!(recovered_row["selected_candidate"]["edge"], recovered_edge);
    assert_eq!(recovered_row["compiled"]["all_points_match"], true);
    assert!(recovered_row["compiled"]["length_mm"].as_i64().unwrap() > 0);
    assert!(
        recovered_row["compiled"]["canonical_arc_m"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    assert_eq!(recovered_row["edge_end_resolution"]["location"], "edgeEnd");
    assert_eq!(
        recovered_row["edge_end_resolution"]["compile_accepted"],
        true
    );
    assert!(
        !recovered_row["maneuver_gate_bindings"]["gates"]
            .as_array()
            .unwrap()
            .is_empty(),
        "recovered row keeps maneuver gate bindings"
    );
    let missing_row = &checklist["missing_programs"][1];
    assert_eq!(missing_row["case"], "not_recovered");
    assert_eq!(missing_row["program_compile_checks"], "not_executed");
    assert_eq!(missing_row["source_binding"]["binding_consistent"], true);
    assert_eq!(
        missing_row["source_binding"]["declared_edge_id"],
        missing_edge
    );
    let success_row = &checklist["original_success"][0];
    assert_eq!(success_row["case"], "original_success");
    assert_eq!(
        success_row["program_preserved"]["byte_equal_to_original"],
        true
    );
    assert!(
        !success_row["control_semantics"]["gates_unchanged"]
            .as_array()
            .unwrap()
            .is_empty(),
        "success row keeps its gate bindings"
    );
}
