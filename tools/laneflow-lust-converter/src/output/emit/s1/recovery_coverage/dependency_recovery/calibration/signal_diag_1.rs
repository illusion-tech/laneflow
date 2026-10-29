//! #1138：信号诊断组 1（C14/C15/C18/C46，23 条缺边）的固定曲线覆盖补齐、逐边局部
//! 必要条件诊断与候选齐备分量完整控制器验收。执行口径全部冻结于
//! `research/issue-1138-signal-diag-1/plan.md`，方法事实源
//! `research/issue-1121-coverage-diagnosis/plan.md` 逐值沿用，实现模板为
//! `calibration::cohort2_coverage`；本模块不改生产采样、compiler 配置档、阈值与原
//! 成功几何，不改共享代码（协调章程：共享代码只由 E 修改）。计划与冻结实现不一致时
//! 停止执行并重新授权（计划 §7），不得单向迁就。

use super::super::experiment::{merge, pool};
use super::super::stream_a_curve_design_c29_c17::{per_edge_counts, pool_counts};
use super::coverage::{coverage_output, edge_input};
use super::shared_baseline::{grid_node_member, plan_stop_label, span_control};
use super::*;
use sampler::feasibility::EndChord;
use std::collections::BTreeSet;

/// plan §1/§6 冻结样本：23 条缺边按 `edges.json` 边序（C14 → C15 → C18 → C46，
/// 分量内按边 ID 序），即阶段一搜索顺序。
const EDGES: [&str; 23] = [
    "sumo::-10130_12_0",
    "sumo::-10130_13_0",
    "sumo::-10130_4_0",
    "sumo::-10130_5_0",
    "sumo::-10130_7_0",
    "sumo::-10156_2_0",
    "sumo::-10156_3_0",
    "sumo::-10156_4_0",
    "sumo::-10156_5_0",
    "sumo::-10156_8_0",
    "sumo::-11042_0_0",
    "sumo::-11042_10_0",
    "sumo::-11042_11_0",
    "sumo::-11042_14_0",
    "sumo::-11042_15_0",
    "sumo::-11042_16_0",
    "sumo::-11042_4_0",
    "sumo::-11042_6_0",
    "sumo::-29900_10_0",
    "sumo::-29900_11_0",
    "sumo::-29900_16_0",
    "sumo::-29900_3_0",
    "sumo::-29900_4_0",
];

/// plan §2：扩展档按历史清单为空——`1103-expanded` 锚只标记 #1103 的 6 条截断样本边，
/// 均不在本组；本组 0 条边执行扩展档（统筹裁决：补跑 262144 属新增搜索量，本轮不授权）。
const EXPANDED_TIER_EDGES: [&str; 0] = [];

/// plan §5 冻结共同基线池形状（11 边 92 份）：六来源按序合并、规范 f32 点位去重；
/// `sumo::-25332_7_0` 为 #1134 新增 16 份。禁止旧 9/10 边池。
fn shared_pool_shape() -> BTreeMap<String, usize> {
    BTreeMap::from([
        ("sumo::-10004_5_1".to_owned(), 16),
        ("sumo::-10034_0_0".to_owned(), 8),
        ("sumo::-10130_8_0".to_owned(), 8),
        ("sumo::-10130_16_0".to_owned(), 8),
        ("sumo::-10130_18_0".to_owned(), 8),
        ("sumo::-16502_0_0".to_owned(), 1),
        ("sumo::-28860_3_0".to_owned(), 2),
        ("sumo::-29388_3_0".to_owned(), 8),
        ("sumo::-10564_3_0".to_owned(), 1),
        ("sumo::-25332_7_0".to_owned(), 16),
        ("sumo::-7348_3_0".to_owned(), 16),
    ])
}

fn normalized(value: &Value) -> Value {
    serde_json::from_str(&value.to_string()).unwrap()
}

/// plan §6.1 运行局部字段显式归一化（统筹预授权）：分量行 `artifact` 文件名的运行/变体
/// 前缀（`baseline-` / `new-` / `stream-a-…-` 等）归一为规范后缀 `component-NN.lfca`；
/// 其余非计时语义字段一律逐值比较，不许静默跳过。
fn normalized_artifact(value: &Value) -> Value {
    match value.as_str() {
        Some(name) => json!(
            name.split("component-")
                .last()
                .map(|suffix| format!("component-{suffix}"))
                .unwrap_or_else(|| name.to_owned())
        ),
        None => value.clone(),
    }
}

/// plan §6.1 解析域逐值比较（结构与非数值叶 + 双侧同解析器的数值叶）：浮点叶按
/// `to_bits`、整数叶按值且形态一致、字符串按字面、键序逐项一致。失配即 panic 首个失配
/// 路径与两侧位串。数值位级最终裁决以文本 token 域 `assert_raw_leaves_equal` 为准
/// （serde_json 默认解析对 17 位十进制浮点误舍入 1 ULP，见其文档注释）。
fn assert_bits_equal(left: &Value, right: &Value, path: &str) {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => {
            let equal = if a.is_f64() || b.is_f64() {
                a.is_f64()
                    && b.is_f64()
                    && a.as_f64().unwrap().to_bits() == b.as_f64().unwrap().to_bits()
            } else {
                a.as_u64() == b.as_u64() && a.as_i64() == b.as_i64()
            };
            assert!(
                equal,
                "bit mismatch at {path}: {a:?} vs {b:?} (f64 bits {} vs {})",
                a.as_f64().map(f64::to_bits).unwrap_or(u64::MAX),
                b.as_f64().map(f64::to_bits).unwrap_or(u64::MAX)
            );
        }
        (Value::String(a), Value::String(b)) => assert!(a == b, "string mismatch at {path}"),
        (Value::Bool(a), Value::Bool(b)) => assert!(a == b, "bool mismatch at {path}"),
        (Value::Null, Value::Null) => {}
        (Value::Array(a), Value::Array(b)) => {
            assert!(a.len() == b.len(), "array length mismatch at {path}");
            for (index, (x, y)) in a.iter().zip(b).enumerate() {
                assert_bits_equal(x, y, &format!("{path}[{index}]"));
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert!(
                a.len() == b.len(),
                "object key count mismatch at {path}: {a:?} vs {b:?}"
            );
            for ((k1, v1), (k2, v2)) in a.iter().zip(b) {
                assert!(k1 == k2, "object key order mismatch at {path}");
                assert_bits_equal(v1, v2, &format!("{path}.{k1}"));
            }
        }
        _ => panic!("type mismatch at {path}: {left:?} vs {right:?}"),
    }
}

/// JSON 文本叶子提取（路径 → 原始 token），文本 token 域位级比较的载体。背景（统筹裁决
/// B，2026-10-29）：serde_json 1.0.154 默认解析（未开 `float_roundtrip`）对 17 位十进制
/// 浮点误舍入 1 ULP（实证 `from_str("10.220051765441895")` → bits …313，真值 …312），
/// 解析域把相邻对（…895/…896）折叠到同一 f64——位级裁决必须保留原始 token，用正确舍入
/// 的 `f64::from_str` 比较。
fn raw_leaves(text: &str) -> BTreeMap<String, String> {
    let bytes = text.as_bytes();
    let mut leaves = BTreeMap::new();
    let mut pos = 0;
    parse_raw_value(text, bytes, &mut pos, "", &mut leaves);
    leaves
}

fn parse_raw_value(
    text: &str,
    bytes: &[u8],
    pos: &mut usize,
    path: &str,
    leaves: &mut BTreeMap<String, String>,
) {
    skip_raw_ws(bytes, pos);
    match bytes[*pos] {
        b'{' => {
            *pos += 1;
            loop {
                skip_raw_ws(bytes, pos);
                if bytes[*pos] == b'}' {
                    *pos += 1;
                    break;
                }
                let key = parse_raw_string(text, bytes, pos);
                skip_raw_ws(bytes, pos);
                assert_eq!(bytes[*pos], b':', "object colon at {path}");
                *pos += 1;
                parse_raw_value(text, bytes, pos, &format!("{path}.{key}"), leaves);
                skip_raw_ws(bytes, pos);
                match bytes[*pos] {
                    b',' => *pos += 1,
                    b'}' => {
                        *pos += 1;
                        break;
                    }
                    other => panic!("object delimiter {other} at {path}"),
                }
            }
        }
        b'[' => {
            *pos += 1;
            let mut index = 0;
            loop {
                skip_raw_ws(bytes, pos);
                if bytes[*pos] == b']' {
                    *pos += 1;
                    break;
                }
                parse_raw_value(text, bytes, pos, &format!("{path}[{index}]"), leaves);
                index += 1;
                skip_raw_ws(bytes, pos);
                match bytes[*pos] {
                    b',' => *pos += 1,
                    b']' => {
                        *pos += 1;
                        break;
                    }
                    other => panic!("array delimiter {other} at {path}"),
                }
            }
        }
        b'"' => {
            let token = parse_raw_string(text, bytes, pos);
            leaves.insert(format!("{path}.string"), format!("\"{token}\""));
        }
        b'-' | b'0'..=b'9' => {
            let start = *pos;
            while *pos < bytes.len()
                && (bytes[*pos].is_ascii_digit()
                    || matches!(bytes[*pos], b'.' | b'e' | b'E' | b'+' | b'-'))
            {
                *pos += 1;
            }
            leaves.insert(format!("{path}.number"), text[start..*pos].to_owned());
        }
        _ => {
            let start = *pos;
            while *pos < bytes.len() && bytes[*pos].is_ascii_alphabetic() {
                *pos += 1;
            }
            leaves.insert(format!("{path}.literal"), text[start..*pos].to_owned());
        }
    }
}

fn parse_raw_string(text: &str, bytes: &[u8], pos: &mut usize) -> String {
    assert_eq!(bytes[*pos], b'"', "string open");
    *pos += 1;
    let mut token = String::new();
    while *pos < bytes.len() {
        match bytes[*pos] {
            b'\\' => {
                token.push_str(&text[*pos..*pos + 2]);
                *pos += 2;
            }
            b'"' => {
                *pos += 1;
                break;
            }
            _ => {
                token.push(text[*pos..].chars().next().unwrap());
                *pos += text[*pos..].chars().next().unwrap().len_utf8();
            }
        }
    }
    token
}

fn skip_raw_ws(bytes: &[u8], pos: &mut usize) {
    while *pos < bytes.len() && bytes[*pos].is_ascii_whitespace() {
        *pos += 1;
    }
}

/// plan §6.1 文本 token 域位级硬门（统筹裁决 B）：锚文件**原始 token** vs 本路内存值
/// 序列化 token 逐叶比较——数值叶以正确舍入 `f64::from_str` 解析后按 `to_bits` 相等，
/// 字面叶按原文；锚侧 `artifact`（运行局部）按前缀归一化后比较、`source`（来源注）显式
/// 声明不入比较；锚侧每个叶必须在本路侧找到对应值——不许静默跳过字段（本路附加分析列
/// cohort / stop_line_checks 允许超出锚侧）。
fn assert_raw_leaves_equal(mine: &Value, anchor_text: &str) {
    let mine_text = serde_json::to_string(mine).unwrap();
    let mine_leaves = raw_leaves(&mine_text);
    let anchor_leaves = raw_leaves(anchor_text);
    let mut checked = 0;
    for (path, token) in &anchor_leaves {
        if path.starts_with(".source.") {
            continue; // 来源注（plan §6.1 显式声明不入逐值比较）
        }
        let own = mine_leaves
            .get(path)
            .unwrap_or_else(|| panic!("missing leaf at {path} in computed result"));
        if path.ends_with(".artifact.string") {
            let normalize = |raw: &str| {
                raw.trim_matches('"')
                    .split("component-")
                    .last()
                    .map(|suffix| format!("component-{suffix}"))
                    .unwrap_or_else(|| raw.to_owned())
            };
            assert_eq!(
                normalize(token),
                normalize(own),
                "artifact run-local prefix mismatch at {path}"
            );
        } else if let (Ok(a), Ok(b)) = (token.parse::<f64>(), own.parse::<f64>()) {
            assert_eq!(
                token.parse::<u64>().is_ok(),
                own.parse::<u64>().is_ok(),
                "number form mismatch at {path}: {token} vs {own}"
            );
            assert!(
                a.to_bits() == b.to_bits(),
                "raw bit mismatch at {path}: {token} vs {own} (f64 bits {} vs {})",
                a.to_bits(),
                b.to_bits()
            );
        } else {
            assert!(
                token == own,
                "raw leaf mismatch at {path}: {token} vs {own}"
            );
        }
        checked += 1;
    }
    assert!(
        checked >= 1_000,
        "raw leaf comparison coverage implausible: {checked}"
    );
}

fn cohort() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1138-signal-diag-1/edges.json"
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

/// 待赋值边 = 缺边 + 既有候选边；去重排序后与分量盘点 `missing_edges` 同序。
fn row_assignment_edges(row: &Value) -> Vec<String> {
    let mut edges = row_list(&row["missing_edges"]);
    edges.extend(row_list(&row["candidate_edges"]));
    edges.sort();
    edges.dedup();
    edges
}

/// plan §3 冻结问题生成规则（逐端判定）：span 端点切向为边界钳制 → 端弦问题（方向锥 =
/// 对应端钳制原 f64 位）；规定倒圆 → 端弦问题（不作方向条件）；interior CR → **该端不取**。
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

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).expect("shared baseline input present")).unwrap()
}

/// plan §2：默认档与历史 `1103-base` 记录逐值比较（本组唯一历史锚；`1107-*` 记录不涉及
/// 本组边）。无记录边为首次执行，由一致性单测与记录文件双向核对。
fn assert_unchanged_history(
    legacy: &Value,
    edge: &Value,
    id: &str,
    report: &Value,
    candidates: &Value,
) {
    for entry in edge["history"].as_array().unwrap() {
        assert_eq!(
            entry["experiment"], "1103-base",
            "this group only anchors on 1103-base records"
        );
        let record = legacy["tiers"]["32768"]["searches"]
            .get(id)
            .expect("1103-base record present");
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

/// plan §3 阶段二：零候选边按冻结问题表执行逐边局部必要条件诊断（逐端判定）。见证
/// 参数注入只把见证参数并入注入网格，不强制使用见证弦，尝试/使用证据逐条记录。
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
                    assert!(clamp.is_some(), "计划 §3.4 起始边界钳制缺失，§7 停止");
                    (clamp, "start_boundary_clamp")
                }
                "finish_boundary_clamp" => {
                    let clamp = input.clamps[1];
                    assert!(clamp.is_some(), "计划 §3.4 末端边界钳制缺失，§7 停止");
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
                assert!(applied, "见证参数注入未命中注入网格，§7 停止");
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
    (
        json!({"edges":summaries,"problems":problems,
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

/// plan §4「原成功停止线边」情形核对（本组 0 条缺程序停止线边）：全部停止线落在成功
/// 边，逐条核对原程序保持、来源边关系与机动门/信号组/转换下标绑定。
fn stop_line_checks(
    part: &TrafficPackage,
    originals: &BTreeMap<String, re::RoadEditingCurveProgram>,
) -> Value {
    let rows = part
        .signals
        .stop_lines
        .iter()
        .map(|stop| {
            let gates = part
                .signals
                .maneuver_gates
                .iter()
                .filter(|gate| gate.stop_line_id == stop.id)
                .map(|gate| {
                    json!({"gate":gate.id,"path":gate.maneuver_path_id,
                        "signal_group":gate.signal_control.group_id,
                        "transition_index":gate.transition_index})
                })
                .collect::<Vec<_>>();
            json!({"case":"original_success_stop_line","stop_line":stop.id,
                "edge":stop.edge_id,
                "original_program_preserved":originals.contains_key(&stop.edge_id),
                "gates":gates})
        })
        .collect::<Vec<_>>();
    assert!(
        rows.iter()
            .all(|row| row["original_program_preserved"] == true),
        "all stop lines sit on original successful edges"
    );
    json!({"case_note":"本组 0 条缺程序停止线边，仅出现「原成功停止线边」情形；恢复/未恢复情形不适用",
        "rows":rows,
        "controllers":part.signals.controllers.iter().map(|controller| json!({
            "controller":controller.id,"groups":controller.group_ids})).collect::<Vec<_>>()})
}

#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, DIRECT_SUCCESSOR_BASELINE_DIR, SIX_STREAMS_BASELINE_DIR (shared baseline extract), COVERAGE_ORDER, COVERAGE_DIAGNOSIS_ORDER and fresh COVERAGE_OUTPUT_DIR"]
fn pinned_lust_signal_diag_1_experiment() {
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
    let baseline = shared_baseline::shared_pools();
    assert_eq!(
        per_edge_counts(&baseline),
        shared_pool_shape(),
        "frozen shared pool shape: 11 edges 92 candidates"
    );
    let anchor = shared_baseline::results_anchor_pinned();
    let variant_anchor = &anchor;
    // 锚文件原始文本：文本 token 域位级终裁的基准（serde 解析会误舍入，见 raw_leaves）。
    let anchor_text = fs::read_to_string(
        PathBuf::from(std::env::var("SIX_STREAMS_BASELINE_DIR").expect("SIX_STREAMS_BASELINE_DIR"))
            .join("results-anchor.json"),
    )
    .expect("anchor raw text");
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
    // 锚的 `source` 为 E0 资产来源注（代际/事故/哈希），非 LuST 源对象；语义面为
    // comparison_only 标记（plan §6.1：来源注不入逐值比较，显式声明不跳语义字段）。
    assert_eq!(
        variant_anchor["source"]["comparison_only"],
        json!(true),
        "anchor is result comparison only"
    );
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
    assert_eq!(cohort_components.len(), 4, "four cohort components");
    assert_eq!(
        cohort_components
            .iter()
            .map(|component| component.paths.len())
            .sum::<usize>(),
        54,
        "complete associated paths"
    );
    assert_eq!(
        cohort_components
            .iter()
            .map(|component| component.target_paths.len())
            .sum::<usize>(),
        19,
        "original targets behind the 54 paths"
    );
    for row in cohort["components"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        let component = components
            .iter()
            .find(|component| component.id == id)
            .expect("cohort component in frozen inventory");
        assert_eq!(row_paths(row), component.paths, "frozen paths for {id}");
        assert_eq!(
            row_assignment_edges(row),
            component.missing_edges,
            "待赋值边 equals inventory missing edges for {id}"
        );
        // plan §4 完整控制器验收前置：分量即控制器全集闭包（full_controller_members）。
        let members = row["full_controller_members"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|member| row_list(&member["all_paths"]))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            members,
            component.paths.iter().cloned().collect::<BTreeSet<_>>(),
            "component equals the complete controller membership for {id}"
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
        // plan §2：扩展档按历史清单为空（0 条边），写出空记录保持输出模式齐整。
        if family == "old" {
            recovery::write_json(&out, "old-expanded-searches.json", &json!({}));
            searches.insert("old-expanded".to_owned(), json!({}));
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
    // plan §5：共同基线池 = E0 冻结交付的 11 边 92 份（pool.json 已按六来源合并去重）；
    // 新增池 = 共同基线 + 本轮两族新候选（实验独立：不吸收其他路候选）。
    let mut combined = baseline.clone();
    merge(&mut combined, pool(&searches["old"]));
    if let Some(rows) = searches.get("old-expanded") {
        merge(&mut combined, pool(rows));
    }
    merge(&mut combined, pool(&searches["grid"]));
    let mut results = BTreeMap::new();
    let mut validation_times = BTreeMap::new();
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
            let mut stop_checks = Value::Null;
            if cohort_ids.contains(&component.id) {
                let part = combinations::part(traffic, component);
                stop_checks = stop_line_checks(&part, &originals);
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
            if result["status"] == "accepted" {
                for (id, choice) in result["selected_assignment"].as_object().unwrap() {
                    programs.insert(
                        id.clone(),
                        pools[id][choice.as_u64().unwrap() as usize].clone(),
                    );
                }
            }
            let artifact = (result["status"] == "accepted").then_some(file);
            if variant == "baseline" {
                let prior = &variant_anchor["components"][index];
                assert_eq!(prior["component"], json!(component.id));
                // 解析域逐值（双侧同解析器）；数值位级由 assert_raw_leaves_equal 终裁。
                assert_bits_equal(
                    &normalized(&result),
                    &prior["result"],
                    "components[].result",
                );
                assert_eq!(
                    normalized_artifact(&json!(artifact)),
                    normalized_artifact(&prior["artifact"]),
                    "artifact run-local prefix normalized equal"
                );
            }
            rows.push(json!({"component":component.id,"result":result,
                "cohort":cohort_ids.contains(&component.id),
                "stop_line_checks":stop_checks,
                "artifact":artifact}));
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
            .zip(variant_anchor["paths"].as_array().unwrap())
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
            assert_bits_equal(
                &normalized(&json!(paths)),
                &variant_anchor["paths"],
                "paths",
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
        if variant == "baseline" {
            // plan §6.1 裁决 B：文本 token 域位级终裁（原始 token + f64::from_str）。
            assert_raw_leaves_equal(
                &json!({"components":rows,"paths":paths,"path_counts":counts}),
                &anchor_text,
            );
        }
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
            "complete_associated_paths":{"paths":54,"original_targets":19,
                "baseline_accepted":cohort_accepted(&baseline_result),
                "new_accepted":cohort_accepted(&new_result),
                "note":"54 条完整关联路径 ↔ 19 条原目标（C14/C15/C18 各 1 + C46 16）；其余信号分量（D 组）另片"}}});
    recovery::write_json(
        &out,
        "results.json",
        &json!({"schema":"laneflow-1138-signal-diag-1","source":source,
        "edges":EDGES,"expanded_tier_edges":EXPANDED_TIER_EDGES,
        "zero_candidate_edges":zero_edges,"grid_policy":sampler::grid::policy(),
        "old_policy":sampler::policy(sampler::SearchBudget::default()),
        "cohort_source":"research/issue-1138-signal-diag-1/edges.json",
        "method_source":"research/issue-1121-coverage-diagnosis/plan.md",
        "coordination":"research/six-streams-20261029/coordination.md",
        "history_cross_check":{"edges_with_history":with_history,
            "first_run_edges":EDGES.len()-with_history,
            "value_comparisons":"1103-base 逐值对照通过；扩展档按历史清单为空（0 边）"},
        "shared_baseline":{"source":"SIX_STREAMS_BASELINE_DIR:pool.json",
            "edge_count":pool_counts(&baseline).0,"candidate_count":pool_counts(&baseline).1,
            "per_edge":per_edge_counts(&baseline)},
        "new_pool":{"edge_count":pool_counts(&combined).0,"candidate_count":pool_counts(&combined).1,
            "per_edge":per_edge_counts(&combined)},
        "result_comparison":{"target":"SIX_STREAMS_BASELINE_DIR:results-anchor.json（#1134 新增变体）",
            "role":"result comparison only; not a candidate point source",
            "components_result_equal":true,"paths_equal":true,"path_counts_equal":true,
            "accepted_paths":baseline_result["path_counts"]["accepted"],
            "note":"对照锚接受 28/84、53 分量、84 路径逐值；禁止旧 9/10 边池"},
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
    println!("signal diag 1 evidence saved={}", out.display());
}

#[test]
fn signal_diag_1_frozen_sample_and_problem_table_stay_consistent() {
    let cohort = cohort();
    assert_eq!(
        cohort["schema"],
        "laneflow-round4-signal-diag-1-coverage-diagnosis"
    );
    assert_eq!(
        cohort["method_source"],
        "research/issue-1121-coverage-diagnosis/plan.md"
    );
    assert_eq!(cohort["summary"]["components"], 4);
    assert_eq!(cohort["summary"]["missing_edges"], 23);
    assert_eq!(cohort["summary"]["complete_paths"], 54);
    assert_eq!(cohort["summary"]["original_targets"], 19);
    assert_eq!(cohort["summary"]["candidate_edges"], 3);
    assert_eq!(cohort["summary"]["controllers"], 4);
    assert_eq!(cohort["summary"]["stop_line_edges"], 0);
    assert_eq!(cohort["summary"]["single_missing_components"], 0);
    assert_eq!(cohort["summary"]["expanded_tier_edges"], 0);
    assert_eq!(cohort["summary"]["first_run_edges"], 8);
    let components = cohort["components"].as_array().unwrap();
    assert_eq!(
        components
            .iter()
            .map(|row| row["code"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["C14", "C15", "C18", "C46"],
        "frozen component code order"
    );
    assert_eq!(
        components
            .iter()
            .map(|row| row["path_count"].as_u64().unwrap() as usize)
            .sum::<usize>(),
        54
    );
    for row in components {
        assert_eq!(
            row["path_count"].as_u64().unwrap() as usize,
            row_paths(row).len(),
            "path_count matches frozen paths"
        );
        assert_eq!(
            row_list(&row["stop_line_edges"]).len(),
            0,
            "no missing stop edges"
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
    let candidates = components
        .iter()
        .flat_map(|row| row_list(&row["candidate_edges"]))
        .collect::<Vec<_>>();
    assert_eq!(
        candidates,
        ["sumo::-10130_16_0", "sumo::-10130_18_0", "sumo::-10130_8_0"],
        "three frozen candidate edges (C14)"
    );
    assert_eq!(
        components
            .iter()
            .map(|row| row_assignment_edges(row).len())
            .collect::<Vec<_>>(),
        [8, 5, 8, 5],
        "C14=8 / C15=5 / C18=8 / C46=5 待赋值边"
    );
    assert!(
        EXPANDED_TIER_EDGES.is_empty(),
        "expanded tier history list is empty for this group"
    );
    let ids = cohort["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| edge["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids, EDGES, "frozen edge order equals search order");
    let mut problem_count = 0;
    let mut both_taken = 0;
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
            "per-end rule reproduces the frozen table for {}",
            edge["id"]
        );
        problem_count += expected.len();
        if expected.len() == 2 {
            both_taken += 1;
        }
        assert_eq!(
            edge["diagnosis_status"] == "selected",
            !expected.is_empty(),
            "diagnosis status follows the per-end rule"
        );
    }
    assert_eq!(problem_count, 30, "30 local problems");
    assert_eq!(both_taken, 7, "seven both-end problems, sixteen single-end");
    assert!(
        cohort["edges"]
            .as_array()
            .unwrap()
            .iter()
            .all(|edge| edge["diagnosis_status"] != "endpoint_diagnosis_not_selected"),
        "no both-interior-CR edge in this group"
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
            assert_eq!(
                entry["experiment"], "1103-base",
                "this group only anchors on 1103-base records"
            );
            let record = &legacy["tiers"]["32768"]["searches"][id];
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
        }
    }
    assert_eq!(
        (first_run, with_history),
        (8, 15),
        "eight first-run edges and fifteen with history"
    );
}

#[test]
fn signal_diag_1_shared_pool_shape_matches_the_frozen_common_baseline() {
    // plan §5 冻结共同基线形状：11 边 92 份（含 #1134 sumo::-25332_7_0 的 16 份），
    // 每边份数逐值冻结；真实 pool.json 由 E0 冻结交付并在正式入口按钉版读取断言。
    let shape = shared_pool_shape();
    assert_eq!(shape.len(), 11);
    assert_eq!(shape.values().sum::<usize>(), 92);
    assert_eq!(shape["sumo::-25332_7_0"], 16);
    assert_eq!(shape["sumo::-10130_8_0"], 8);
    assert_eq!(shape["sumo::-10130_16_0"], 8);
    assert_eq!(shape["sumo::-10130_18_0"], 8);
    // 临时目录同字节内容验证：写入什么字节即读回什么字节；pool.json 行形状与 E0 冻结
    // 交付一致（`experiment::pool` 直接可消费）。
    let candidate = json!({"program":{"start":[0,0,0],
        "segments":[{"end":[10.0_f64.to_bits(),0,0],"canvas_selection":null}]},
        "arc_m":10.0,"spans":[]});
    let source = json!({"sumo::signal-diag-synthetic":{"candidates":[candidate]}});
    let bytes = serde_json::to_vec(&source).unwrap();
    let dir = std::env::temp_dir().join(format!("laneflow-signal-diag-1-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pool.json");
    fs::write(&path, &bytes).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes, "same-byte write and read");
    let loaded = read_json(&path);
    assert_eq!(loaded, source);
    assert_eq!(
        pool(&loaded)["sumo::signal-diag-synthetic"].len(),
        1,
        "pinned pool rows deserialize to complete programs"
    );
    fs::remove_dir_all(&dir).unwrap();
}
