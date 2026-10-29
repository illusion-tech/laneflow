//! #1139：信号诊断组 2（C19/C22/C24）的固定曲线覆盖补齐、逐边局部必要条件诊断、
//! 候选齐备分量组合验收与停止线三情形核对。执行口径全部冻结于
//! `research/issue-1139-signal-diag-2/plan.md`，样本与问题表冻结于同目录 `edges.json`；
//! 方法事实源 `research/issue-1121-coverage-diagnosis/plan.md` 逐值沿用，实现模板为
//! `research/issue-1134-cohort2-coverage-diagnosis/`。本模块不改生产采样、compiler
//! 配置档、阈值与原成功几何。计划与冻结实现不一致时停止执行并重新授权（plan §8），
//! 不得单向迁就。
//!
//! 基线接线边界（plan §6）：共同基线 = E0（Issue #1140）冻结交付的 11 边 92 份
//! （六来源按序合并、规范 f32 点位去重），结果对照锚为 #1134 新增变体（53 分量、
//! 84 路径计数）逐值。读取入口与环境变量名以 E0 冻结交付为准（`*_BASELINE_DIR`
//! 模式），本路代码内断言其钉版哈希；`common_baseline` 在 E0 交付前保持停止占位
//! （Issue 锚点 #1140 / #1139），搜索、诊断与停止线三情形核对表机械均已实现。

use super::super::experiment::{merge, pool};
use super::super::stream_a_curve_design_c29_c17::{per_edge_counts, pool_counts};
use super::coverage::{
    coverage_output, edge_input, grid_node_member, plan_stop_label, span_control,
};
use super::*;
use crate::output::digest::hex_sha256;
use crate::output::model::{ManeuverGate, SignalControl, StopLine};
use sampler::feasibility::EndChord;
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
const EXPANDED_TIER_PARTITIONS: u64 = 262_144;

fn normalized(value: &Value) -> Value {
    serde_json::from_str(&value.to_string()).unwrap()
}

fn cohort() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1139-signal-diag-2/edges.json"
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

/// 读取钉版输入并在代码内断言 SHA-256 与冻结值一致（写入什么字节即校验与解析什么字节）。
fn read_pinned_json(path: &Path, expected_sha: &str) -> Value {
    let bytes = fs::read(path).expect("pinned baseline input present");
    assert_eq!(
        hex_sha256(&bytes),
        expected_sha,
        "pinned baseline input digest"
    );
    serde_json::from_slice(&bytes).unwrap()
}

/// plan §6：共同基线 = E0（Issue #1140）冻结交付的 11 边 92 份，结果对照锚 = #1134
/// 新增变体逐值。E0 冻结接口（`research/issue-1140-shared-baseline/README.md` 形状定案）：
/// env `SIX_STREAMS_BASELINE_DIR` + `shared_baseline::shared_pools()`（返回
/// `combinations::Pools`）+ `shared_baseline::results_anchor_pinned()`，含代码内哈希、
/// 合并顺序与逐边份数断言。E0 助手入库后本函数接线其门面；此前保持停止占位
/// （Issue 锚点 #1140 / #1139），不得以旧 10 边 76 份池或任何替代来源充当共同基线。
fn common_baseline() -> (combinations::Pools, Value) {
    todo!(
        "G2 接线：E0（#1140）shared_baseline::shared_pools()/results_anchor_pinned()（Issue #1139）"
    )
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

/// plan §2：默认档与历史 `1103-base` 记录逐值比较；`1103-expanded` 由扩展档步骤对照。
/// 本路历史词汇仅 `1103-base`/`1103-expanded`（`edges.json` `history` 冻结）。
fn assert_unchanged_history(
    legacy: &Value,
    edge: &Value,
    id: &str,
    report: &Value,
    candidates: &Value,
) {
    for entry in edge["history"].as_array().unwrap() {
        if entry["experiment"] == "1103-expanded" {
            continue;
        }
        assert_eq!(
            entry["experiment"], "1103-base",
            "frozen history vocabulary"
        );
        let record = &legacy["tiers"]["32768"]["searches"][id];
        assert!(!record.is_null(), "1103-base record present for {id}");
        assert_eq!(
            normalized(report),
            record["search"],
            "unchanged 1103-base report for {id}"
        );
        assert_eq!(
            normalized(candidates),
            record["candidates"],
            "unchanged 1103-base candidates for {id}"
        );
    }
}

fn assert_unchanged_expanded(
    legacy: &Value,
    edge: &Value,
    id: &str,
    report: &Value,
    candidates: &Value,
) {
    assert!(
        edge["history"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["experiment"] == "1103-expanded"),
        "{id} carries 1103-expanded history"
    );
    let record = &legacy["tiers"]["262144"]["searches"][id];
    assert!(!record.is_null(), "1103-expanded record present for {id}");
    assert_eq!(
        normalized(report),
        record["search"],
        "unchanged 1103-expanded report for {id}"
    );
    assert_eq!(
        normalized(candidates),
        record["candidates"],
        "unchanged 1103-expanded candidates for {id}"
    );
}

/// plan §3 阶段二：零候选边按冻结问题表执行逐边局部必要条件诊断。`endpoint_diagnosis_not_selected`
/// 边不诊断（零候选只记搜索结果）；见证参数注入只把见证参数并入注入网格，不强制
/// 使用见证弦，尝试/使用证据由注入搜索逐条记录。
fn run_diagnosis(
    observation: &observe::EmissionObservation,
    searches: &BTreeMap<String, Value>,
    grid_history: &Value,
    zero_edges: &BTreeSet<&str>,
    cohort: &Value,
) -> (Value, Value) {
    let mut problems = Vec::new();
    let mut summaries = BTreeMap::<String, Value>::new();
    let mut timing = BTreeMap::<String, Value>::new();
    let mut control_sources = BTreeMap::<&str, usize>::new();
    let mut replay_fallback_edges = BTreeSet::new();
    for edge in cohort["edges"].as_array().unwrap() {
        let id = edge["id"].as_str().unwrap();
        let summary = summaries.entry(id.to_owned()).or_insert_with(|| {
            json!({"proven_fixed_conflicts":[],"local_witness_kinds":[],
                "injected_search_stop_labels":[],"has_stage1_candidates":!zero_edges.contains(id)})
        });
        if edge["diagnosis_status"] == "endpoint_diagnosis_not_selected" {
            summary["diagnosis"] = json!("endpoint_diagnosis_not_selected");
            continue;
        }
        assert_eq!(
            edge["diagnosis_status"], "selected",
            "frozen diagnosis status"
        );
        if !zero_edges.contains(id) {
            for problem in edge["problems"].as_array().unwrap() {
                problems.push(
                    json!({"edge":id,"span_index_zero_based":edge["span_index_zero_based"],
                    "problem_kind":problem["kind"],
                    "applicability":"not_applicable_edge_has_candidates"}),
                );
            }
            continue;
        }
        let input = edge_input(observation, id);
        // plan §3：span 取原诊断报告下标（0 基），为边级字段；`problems` 只冻结问题类型
        // 与方向条件来源，同一边的全部局部问题共享该 span。
        let span = edge["span_index_zero_based"].as_u64().unwrap() as usize;
        let resolved = span_control(searches, grid_history, id, span, input);
        *control_sources.entry(resolved.source).or_default() += 1;
        if resolved.source == "replay_fallback" {
            replay_fallback_edges.insert(id.to_owned());
        }
        let replay_agrees = resolved
            .replay_control
            .map(|replay| replay[..4] == resolved.control[..4]);
        assert!(
            replay_agrees.is_none_or(|agrees| agrees),
            "replay control cross-check for {id} span {span}"
        );
        for problem in edge["problems"].as_array().unwrap() {
            let kind_label = problem["kind"].as_str().unwrap();
            let kind = match kind_label {
                "first_chord" => EndChord::First,
                "last_chord" => EndChord::Last,
                other => panic!("unknown problem kind {other}"),
            };
            let (direction, direction_source) = match problem["direction_source"].as_str().unwrap()
            {
                "start_boundary_clamp" => {
                    let clamp = input.clamps[0];
                    assert!(clamp.is_some(), "plan §3.4 起始边界钳制缺失，§8 停止");
                    (clamp, "start_boundary_clamp")
                }
                "finish_boundary_clamp" => {
                    let clamp = input.clamps[1];
                    assert!(clamp.is_some(), "plan §3.4 末端边界钳制缺失，§8 停止");
                    (clamp, "finish_boundary_clamp")
                }
                "fillet_not_a_direction_condition" => (None, "fillet_not_a_direction_condition"),
                other => panic!("unknown direction source {other}"),
            };
            let control = resolved.control;
            let started = Instant::now();
            let result = sampler::feasibility::diagnose_end_chord(control, kind, direction);
            timing.insert(
                format!("{id}:{kind_label}"),
                json!(started.elapsed().as_secs_f64()),
            );
            let witness_t = result["witness"]["parameters_f64_bits"]
                .as_array()
                .map(|bits| {
                    let bits = bits
                        .iter()
                        .map(|b| f64::from_bits(b.as_u64().unwrap()))
                        .collect::<Vec<_>>();
                    match kind {
                        EndChord::First => bits[1],
                        EndChord::Last => bits[0],
                    }
                });
            let mut injected = Value::Null;
            if let Some(t) = witness_t {
                let injection = sampler::grid::Injection {
                    control: resolved.injection_control.map(|p| p.map(f64::to_bits)),
                    kind: match kind {
                        EndChord::First => sampler::grid::InjectionKind::FirstChord { t },
                        EndChord::Last => sampler::grid::InjectionKind::LastChord { t },
                    },
                };
                let started = Instant::now();
                let search = sampler::grid::search_injected(input, injection);
                let label = plan_stop_label(&search.report, search.candidates.len() as u64);
                timing.insert(
                    format!("{id}:{kind_label}:injected"),
                    json!(started.elapsed().as_secs_f64()),
                );
                let applied = search
                    .report
                    .get("witness_injection_records")
                    .and_then(Value::as_array)
                    .is_some_and(|records| !records.is_empty());
                assert!(applied, "见证参数注入未命中注入网格，§8 停止");
                summary["injected_search_stop_labels"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"kind":kind_label,"plan_stop_label":label}));
                injected = json!({"search":search.report,"candidates":recovery::candidates(&search),
                    "plan_stop_label":label});
            }
            match result["status"].as_str().unwrap() {
                "necessary_condition_excluded" => summary["proven_fixed_conflicts"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(kind_label)),
                "local_witness" => summary["local_witness_kinds"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(kind_label)),
                "unresolved" => {}
                other => panic!("unknown diagnosis status {other}"),
            }
            problems.push(json!({"edge":id,"span_index_zero_based":span,
                "problem_kind":kind_label,"direction_source":direction_source,
                "applicability":"diagnosed","local_problem_result":result,
                "span_control_source":resolved.source,
                "replay_control_f64_bits":resolved.replay_control.map(|c|c.map(|p|p.map(f64::to_bits))),
                "replay_agrees_with_control":replay_agrees,
                "family_membership":{"calibrated_grid_node":witness_t.map(grid_node_member),
                    "t_f64_bits":witness_t.map(f64::to_bits),
                    "original_partition_family":"not_applicable_partition_family",
                    "note":"成员关系由冻结算法与参数直接计算；history 摘要仅定位版本，不代表实际访问"},
                "actual_access_evidence":{"stage1_searches":"not_recorded",
                    "history":"summary_only_not_access_proof",
                    "note":"参数族成员关系不得推断实际访问；完整尝试轨迹未由现有实现记录"},
                "injected_whole_edge":injected}));
        }
    }
    let endpoints = cohort["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|edge| edge["diagnosis_status"] == "endpoint_diagnosis_not_selected")
        .map(|edge| json!({"edge":edge["id"]}))
        .collect::<Vec<_>>();
    (
        json!({"edges":summaries,"problems":problems,
        "endpoint_diagnosis_not_selected":{"edges":endpoints,
        "note":"本轮规则未选取端点诊断：CR 切向不使弦长必要条件失效，不是「端点必要条件不适用」；内部弦链/末两弦/接缝问题不在本轮冻结范围（另片）"},
        "span_control_sources":{"counts":control_sources,
            "replay_fallback_edges":replay_fallback_edges.iter().collect::<Vec<_>>(),
            "replay_fallback_edge_count":replay_fallback_edges.len(),
            "note":"三层载体链（fresh_failures / history / replay_fallback）；重放为只读交叉核对列，见证注入匹配用完整六点"},
        "policy":{"injection_semantics":"见证参数注入不强制使用见证弦",
        "exclusion_scope":"单个局部问题全参数域排除即否定该固定 cubic 的完整发射；局部见证不构成可行证明",
        "box_limit":32_000_000_u64,"depth_limit":48_u32,"certificate_leaf_limit":65_536_usize}}),
        json!(timing),
    )
}

/// plan §5 停止线三情形核对表：对冻结表 26 条停止线边逐边产出，每条恰好归入一情形
/// （原成功停止线边 / 本轮恢复的停止线边 / 未恢复）。情形 1 核对原程序与控制语义逐字节
/// 保持；情形 2 核对选中候选、规范端点、编译后长度、`edgeEnd` 解析与机动门绑定；
/// 情形 3 如实记录程序/编译检查未执行并保留来源绑定核对。本轮候选齐备即执行情形 2，
/// 不得留给后续设计切片。
fn stop_line_checklist(
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
            .get(code)
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
        "../../../../../../../../../research/issue-1103-s1-recovery-budget/results.json"
    ))
    .unwrap();
    let old_rows: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/old-searches.json"
    ))
    .unwrap();
    let grid_history: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/grid-searches.json"
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

/// 临时目录唯一命名；基线读取验证的同字节内容只写临时目录，不落仓库。
fn temp_probe_dir() -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "laneflow-signal-diag-2-baseline-{}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn signal_diag_2_pinned_loader_parses_same_bytes_and_rejects_others() {
    let source = json!({"sumo::signal-diag-2-synthetic":{"candidates":[]}});
    let bytes = serde_json::to_vec(&source).unwrap();
    let dir = temp_probe_dir();
    let path = dir.join("pinned.json");
    fs::write(&path, &bytes).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes, "same-byte write and read");
    let loaded = read_pinned_json(&path, &hex_sha256(&bytes));
    assert_eq!(loaded, source, "pinned loader parses the written bytes");
    fs::remove_dir_all(&dir).unwrap();
    let dir = temp_probe_dir();
    let path = dir.join("pinned.json");
    fs::write(&path, b"not the pinned baseline bytes").unwrap();
    let result = std::panic::catch_unwind(|| {
        let _ = read_pinned_json(&path, &hex_sha256(&bytes));
    });
    assert!(result.is_err(), "unpinned bytes are rejected");
    fs::remove_dir_all(&dir).unwrap();
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

#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, DIRECT_SUCCESSOR_BASELINE_DIR, E0 common baseline (#1140), COVERAGE_ORDER, COVERAGE_DIAGNOSIS_ORDER and fresh COVERAGE_OUTPUT_DIR"]
fn pinned_lust_signal_diag_2_coverage_diagnosis_experiment() {
    let out = coverage_output();
    let order = std::env::var("COVERAGE_ORDER").unwrap();
    assert!(matches!(order.as_str(), "old-grid" | "grid-old"));
    let diagnosis_order = std::env::var("COVERAGE_DIAGNOSIS_ORDER").unwrap();
    assert!(matches!(
        diagnosis_order.as_str(),
        "diagnosis-first" | "baseline-first"
    ));
    let reference = reference();
    let cohort = cohort();
    let (baseline, comparison) = common_baseline();
    let legacy: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1103-s1-recovery-budget/results.json"
    ))
    .unwrap();
    let grid_history: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/grid-searches.json"
    ))
    .unwrap();
    let frozen: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/inventory.json"
    ))
    .unwrap();
    let (observation, source, mut timing) = capture_lust();
    assert_eq!(source, reference["source"]);
    assert_eq!(source, comparison["source"], "result comparison source");
    let originals = original_programs(&observation);
    snapshot(&out, &observation, &originals);
    let traffic = observation.traffic.as_ref().unwrap();
    let components = components::inventory(traffic, &originals, &target_ids(&reference));
    assert_eq!(json!(components), frozen["components"]);
    assert_eq!(components.len(), 53, "frozen component inventory");
    assert_eq!(components.iter().map(|c| c.paths.len()).sum::<usize>(), 183);
    let cohort_ids = cohort["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_owned())
        .collect::<BTreeSet<_>>();
    let cohort_components = components
        .iter()
        .filter(|component| cohort_ids.contains(&component.id))
        .collect::<Vec<_>>();
    assert_eq!(cohort_components.len(), 3, "three cohort components");
    assert_eq!(
        cohort_components
            .iter()
            .map(|component| component.paths.len())
            .sum::<usize>(),
        47,
        "complete associated paths"
    );
    assert_eq!(
        cohort_components
            .iter()
            .map(|component| component.target_paths.len())
            .sum::<usize>(),
        12,
        "original targets behind the 47 paths"
    );
    for row in cohort["components"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        let component = components
            .iter()
            .find(|component| component.id == id)
            .expect("cohort component in frozen inventory");
        assert_eq!(row_paths(row), component.paths, "frozen paths for {id}");
        assert_eq!(
            row_list(&row["missing_edges"]),
            component.missing_edges,
            "待赋值边 equals inventory missing edges for {id}"
        );
        assert_eq!(
            row_list(&row["controllers"])
                .into_iter()
                .collect::<BTreeSet<_>>(),
            component
                .controllers
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>(),
            "frozen controllers for {id}"
        );
    }
    measurement::write_inputs(
        &out,
        &observation,
        &json!({"samples":EDGES.iter().map(|id|json!({"edge":id})).collect::<Vec<_>>()}),
    );
    let edge_rows = cohort["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| (edge["id"].as_str().unwrap(), edge))
        .collect::<BTreeMap<_, _>>();
    let mut searches = BTreeMap::<String, Value>::new();
    let mut search_times = BTreeMap::<String, Value>::new();
    for family in order.split('-') {
        let mut rows = BTreeMap::new();
        let mut times = BTreeMap::new();
        for id in EDGES {
            let input = edge_input(&observation, id);
            let started = Instant::now();
            let search = if family == "old" {
                sampler::search(input, sampler::SearchBudget::default())
            } else {
                sampler::grid::search_audited(input)
            };
            times.insert(id, started.elapsed().as_secs_f64());
            let candidates = recovery::candidates(&search);
            if family == "old" {
                assert_unchanged_history(&legacy, edge_rows[id], id, &search.report, &candidates);
            }
            rows.insert(
                id,
                json!({"search":search.report,"candidates":candidates,
                    "plan_stop_label":plan_stop_label(&search.report, search.candidates.len() as u64)}),
            );
        }
        if family == "old" {
            let mut expanded = BTreeMap::new();
            let mut expanded_times = BTreeMap::new();
            for id in EXPANDED_TIER_EDGES {
                let input = edge_input(&observation, id);
                let started = Instant::now();
                let search = sampler::search(
                    input,
                    sampler::SearchBudget {
                        partitions: EXPANDED_TIER_PARTITIONS,
                    },
                );
                expanded_times.insert(id, started.elapsed().as_secs_f64());
                let candidates = recovery::candidates(&search);
                assert_unchanged_expanded(&legacy, edge_rows[id], id, &search.report, &candidates);
                expanded.insert(
                    id,
                    json!({"search":search.report,"candidates":candidates,
                        "plan_stop_label":plan_stop_label(&search.report, search.candidates.len() as u64)}),
                );
            }
            recovery::write_json(&out, "old-expanded-searches.json", &json!(expanded));
            searches.insert("old-expanded".to_owned(), json!(expanded));
            search_times.insert("old-expanded".to_owned(), json!(expanded_times));
        }
        let rows = json!(rows);
        recovery::write_json(&out, &format!("{family}-searches.json"), &rows);
        searches.insert(family.to_owned(), rows);
        search_times.insert(family.to_owned(), json!(times));
    }
    let zero_edges: BTreeSet<&str> = EDGES
        .iter()
        .copied()
        .filter(|&id| {
            ["old", "old-expanded", "grid"]
                .iter()
                .filter_map(|family| {
                    searches
                        .get(*family)
                        .and_then(|rows| rows.get(id))
                        .map(|row| row["candidates"].as_array().unwrap().len())
                })
                .all(|count| count == 0)
        })
        .collect();
    let mut diagnosis = Value::Null;
    let mut diagnosis_timing = Value::Null;
    let mut diagnosis_seconds = 0.0;
    if diagnosis_order == "diagnosis-first" {
        let started = Instant::now();
        (diagnosis, diagnosis_timing) =
            run_diagnosis(&observation, &searches, &grid_history, &zero_edges, &cohort);
        diagnosis_seconds = started.elapsed().as_secs_f64();
    }
    let mut combined = baseline.clone();
    merge(&mut combined, pool(&searches["old"]));
    if let Some(rows) = searches.get("old-expanded") {
        merge(&mut combined, pool(rows));
    }
    merge(&mut combined, pool(&searches["grid"]));
    let mut results = BTreeMap::new();
    let mut validation_times = BTreeMap::new();
    let mut final_programs = originals.clone();
    let mut evaluations = BTreeMap::new();
    for (variant, pools) in [("baseline", &baseline), ("new", &combined)] {
        let started = Instant::now();
        let mut programs = originals.clone();
        for (id, choice) in reference["tiers"]["32768"]["best"]["assignment"]
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
            let file = format!("{variant}-component-{index:02}.lfca");
            if cohort_ids.contains(&component.id) {
                assert!(
                    !component.controllers.is_empty(),
                    "cohort component carries its frozen controllers"
                );
            }
            let mut result = combinations::evaluate(
                traffic,
                component,
                &originals,
                pools,
                Some(&out.join(&file)),
            );
            // plan §4：组合超 256 记 `assignment_budget_stopped` 与未访问赋值数；
            // 总组合按全部待赋值边的去重后池大小连乘计算。
            let unvisited = result
                .get("total_assignments")
                .and_then(Value::as_u64)
                .map(|total| total - result["trials"].as_array().unwrap().len() as u64);
            result["unvisited_assignments"] = json!(unvisited);
            if variant == "baseline" {
                let prior = &comparison["variants"]["new"]["components"][index];
                assert_eq!(prior["component"], json!(component.id));
                assert_eq!(
                    normalized(&result),
                    prior["result"],
                    "baseline component matches the result comparison anchor"
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
            if variant == "new" {
                evaluations.insert(component.id.clone(), result.clone());
            }
            rows.push(json!({"component":component.id,"result":result,
                "cohort":cohort_ids.contains(&component.id),
                "artifact":(result["status"]=="accepted").then_some(file)}));
        }
        if variant == "new" {
            final_programs = programs.clone();
        }
        let paths = reference["baseline_paths"]
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
        if variant == "baseline" {
            assert_eq!(
                normalized(&json!(paths)),
                comparison["variants"]["new"]["paths"],
                "baseline target evidence matches the result comparison anchor"
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
        let cohort_paths = cohort["components"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(row_paths)
            .map(|id| {
                let path = traffic
                    .maneuver_paths
                    .iter()
                    .find(|p| p.id == id)
                    .expect("cohort path in topology");
                json!({"path":path.id,"edges":path_edges(path),"result":compile_path(traffic,path,&programs)})
            })
            .collect::<Vec<_>>();
        for row in &cohort_paths {
            if let Some(target) = paths.iter().find(|p| p["path"] == row["path"]) {
                assert_eq!(
                    normalized(row),
                    normalized(target),
                    "cohort path row equals target row"
                );
            }
        }
        let cohort_counts =
            cohort_paths
                .iter()
                .fold(BTreeMap::<String, usize>::new(), |mut counts, row| {
                    *counts
                        .entry(row["result"]["status"].as_str().unwrap().to_owned())
                        .or_default() += 1;
                    counts
                });
        let joint = cohort["components"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let id = row["id"].as_str().unwrap();
                let component = components
                    .iter()
                    .find(|component| component.id == id)
                    .unwrap();
                let result = &rows
                    .iter()
                    .find(|r| r["component"] == id)
                    .expect("cohort component evaluated")["result"];
                let accepted = result["status"] == "accepted";
                let statuses = row_paths(row)
                    .into_iter()
                    .map(|path| {
                        let path_row = cohort_paths
                            .iter()
                            .find(|r| r["path"] == path.as_str())
                            .expect("cohort path row");
                        json!({"path":path,"accepted":path_row["result"]["status"] == "accepted"})
                    })
                    .collect::<Vec<_>>();
                assert!(
                    !accepted || statuses.iter().all(|row| row["accepted"] == true),
                    "component acceptance implies simultaneous complete-path acceptance"
                );
                if accepted {
                    let assignment = result["selected_assignment"].as_object().unwrap();
                    assert_eq!(
                        assignment.keys().map(String::as_str).collect::<Vec<_>>(),
                        component
                            .missing_edges
                            .iter()
                            .map(String::as_str)
                            .collect::<Vec<_>>(),
                        "one shared assignment covers every 待赋值边"
                    );
                }
                json!({"code":row["code"],"component":id,"accepted":accepted,
                    "assignment_edges":component.missing_edges,
                    "controllers":component.controllers,
                    "complete_paths":statuses,
                    "original_targets":component.target_paths.len(),
                    "all_paths_accepted":statuses.iter().all(|row| row["accepted"] == true),
                    "trials":result["trials"].as_array().unwrap().len(),
                    "total_assignments":result["total_assignments"],
                    "assignment_budget_stopped":result["assignment_budget_stopped"],
                    "unvisited_assignments":result["unvisited_assignments"]})
            })
            .collect::<Vec<_>>();
        results.insert(
            variant,
            json!({"components":rows,"paths":paths,"path_counts":counts,
            "cohort_paths":cohort_paths,"cohort_path_counts":cohort_counts,
            "cohort_joint_acceptance":joint,
            "pool_sizes":pools.iter().map(|(id,p)|(id,p.len())).collect::<BTreeMap<_,_>>()}),
        );
        validation_times.insert(variant, started.elapsed().as_secs_f64());
    }
    if diagnosis_order == "baseline-first" {
        let started = Instant::now();
        (diagnosis, diagnosis_timing) =
            run_diagnosis(&observation, &searches, &grid_history, &zero_edges, &cohort);
        diagnosis_seconds = started.elapsed().as_secs_f64();
    }
    recovery::write_json(&out, "diagnosis.json", &diagnosis);
    // plan §5：停止线三情形核对表逐边产出，本轮候选齐备即执行；缺一不可。
    let checklist =
        stop_line_checklist(&cohort, traffic, &originals, &final_programs, &evaluations);
    for row in checklist["missing_programs"].as_array().unwrap() {
        if row["case"] == "recovered_this_round" {
            assert_eq!(row["edge_end_resolution"]["compile_accepted"], true);
            assert!(
                row["edge_end_resolution"]["lfca_bytes"]
                    .as_u64()
                    .unwrap_or(0)
                    > 0,
                "recovered stop line edge carries its LFCA evidence"
            );
        }
    }
    assert_eq!(
        checklist["cases_summary"]["original_success"].as_u64(),
        Some(22)
    );
    assert_eq!(
        checklist["cases_summary"]["recovered_this_round"]
            .as_u64()
            .unwrap()
            + checklist["cases_summary"]["not_recovered"]
                .as_u64()
                .unwrap(),
        4
    );
    recovery::write_json(&out, "stop-line-checklist.json", &checklist);
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
    let cohort_accepted = |variant: &Value| -> u64 {
        variant["cohort_path_counts"]["accepted"]
            .as_u64()
            .unwrap_or(0)
    };
    let with_history = cohort["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|edge| !edge["history"].as_array().unwrap().is_empty())
        .count();
    let gain = json!({
        "new_candidates_per_edge":EDGES.iter().map(|id|json!({"edge":id,
            "baseline":baseline.get(*id).map(Vec::len).unwrap_or(0),
            "new":combined.get(*id).map(Vec::len).unwrap_or(0)})).collect::<Vec<_>>(),
        "components_newly_accepted":newly_accepted,
        "component_assignment_trials":{"baseline":trials_total(&baseline_result),
            "new":trials_total(&new_result)},
        "baseline_path_counts":baseline_result["path_counts"],
        "new_path_counts":new_result["path_counts"],
        "gain_by_accounting":{
            "original_targets_84":{"baseline_accepted":baseline_result["path_counts"]["accepted"],
                "new_accepted":new_result["path_counts"]["accepted"]},
            "complete_associated_paths":{"paths":47,"original_targets":12,
                "baseline_accepted":cohort_accepted(&baseline_result),
                "new_accepted":cohort_accepted(&new_result),
                "note":"47 条完整关联路径 ↔ 12 条原目标（两口径分列）"}}});
    recovery::write_json(
        &out,
        "results.json",
        &json!({"schema":"laneflow-1139-signal-diag-2-coverage-diagnosis","source":source,
        "edges":EDGES,"expanded_tier_edges":EXPANDED_TIER_EDGES,
        "zero_candidate_edges":zero_edges,"grid_policy":sampler::grid::policy(),
        "old_policy":sampler::policy(sampler::SearchBudget::default()),
        "expanded_tier_partitions":EXPANDED_TIER_PARTITIONS,
        "cohort_source":"research/issue-1139-signal-diag-2/edges.json",
        "method_source":"research/issue-1121-coverage-diagnosis/plan.md",
        "history_cross_check":{"edges_with_history":with_history,
            "first_run_edges":EDGES.len()-with_history,
            "value_comparisons":"1103-base/1103-expanded 逐值对照通过"},
        "baseline_pool":{"source":"E0 共同基线（Issue #1140）11 边 92 份，钉版读取",
            "edge_count":pool_counts(&baseline).0,"candidate_count":pool_counts(&baseline).1,
            "per_edge":per_edge_counts(&baseline)},
        "new_pool":{"edge_count":pool_counts(&combined).0,"candidate_count":pool_counts(&combined).1,
            "per_edge":per_edge_counts(&combined)},
        "result_comparison":{"target":"E0 共同基线结果对照（#1134 新增变体逐值）",
            "role":"result comparison only; not a candidate point source",
            "components_result_equal":true,"paths_equal":true,"path_counts_equal":true,
            "accepted_paths":baseline_result["path_counts"]["accepted"]},
        "stop_line_checklist":{"cases_summary":checklist["cases_summary"],
            "table":"stop-line-checklist.json","policy":"三情形缺一不可，本轮候选齐备即执行"},
        "variants":results,"new_gain":gain,
        "all_original_geometry_and_controls_equal":true,"production_g1_accepted":false}),
    );
    timing["search_order"] = json!(order);
    timing["diagnosis_order"] = json!(diagnosis_order);
    timing["diagnosis_seconds"] = json!(diagnosis_seconds);
    timing["diagnosis"] = diagnosis_timing;
    timing["edge_search_seconds"] = json!(search_times);
    timing["validation_seconds"] = json!(validation_times);
    recovery::write_json(&out, "timing.json", &timing);
    println!(
        "signal diag 2 coverage diagnosis evidence saved={}",
        out.display()
    );
}
