//! #1134：第 2 批队列（11 个非信号分量、22 条缺边）的固定曲线覆盖补齐、逐边局部
//! 必要条件诊断与候选齐备分量组合验收。执行口径全部冻结于
//! `research/issue-1134-cohort2-coverage-diagnosis/plan.md`，方法事实源
//! `research/issue-1121-coverage-diagnosis/plan.md` 逐值沿用；本模块不改生产采样、
//! compiler 配置档、阈值与原成功几何。计划与冻结实现不一致时停止执行并重新授权
//! （计划 §7），不得单向迁就。

use super::super::experiment::{merge, pool};
use super::super::stream_a_curve_design_c29_c17::{per_edge_counts, pool_counts};
use super::coverage::{
    coverage_output, edge_input, grid_node_member, plan_stop_label, span_control,
};
use super::*;
use crate::output::digest::hex_sha256;
use sampler::feasibility::EndChord;
use std::collections::BTreeSet;

/// plan §1/§6 冻结样本：22 条缺边按 `edges.json` 边序（C07 → C41），即阶段一搜索顺序。
const EDGES: [&str; 22] = [
    "sumo:-32592#0_0",
    "sumo::-1000_2_0",
    "sumo::-1000_5_0",
    "sumo::-1000_7_0",
    "sumo::-10034_4_0",
    "sumo::-10046_0_0",
    "sumo::-10100_1_0",
    "sumo:--31624_0",
    "sumo:-30976#0_0",
    "sumo::-10414_4_0",
    "sumo::-22822_2_0",
    "sumo::-1426_1_0",
    "sumo::-1426_2_0",
    "sumo::-19726_2_0",
    "sumo::-22042_7_0",
    "sumo:-32770#6_1",
    "sumo:-32770#7_1",
    "sumo::-25332_7_0",
    "sumo::-25332_7_1",
    "sumo::-8018_4_0",
    "sumo::-8018_7_0",
    "sumo::-25490_4_0",
];

/// plan §2：扩展档 262,144 仅这两条有 `1103-expanded` 历史记录的边执行；其余 20 边
/// 不跑扩展档（新增搜索量须另授权）。
const EXPANDED_TIER_EDGES: [&str; 2] = ["sumo::-1000_2_0", "sumo::-10046_0_0"];
const EXPANDED_TIER_PARTITIONS: u64 = 262_144;

/// plan §5 钉版基线输入（Release 资产 `research-evidence-1134-cohort2-baseline`，
/// 资产 `cohort2-baseline-inputs.zip`）：SHA-256 与字节数冻结，身份不依赖仓库内文件。
const STREAM_A_SEARCHES_SHA256: &str =
    "1c6dd16244481d8fa757865305b2f05287ed74624dde5770998e752d18e26d7b";
const STREAM_A_RESULTS_SHA256: &str =
    "17d6bdce48e63cb26116ea6fc9882225a1aa95d7f67bb248bacfa15ac3b37786";

/// 读取钉版基线输入并在代码内断言 SHA-256 与 plan §5 钉版值一致（发行资产可验证为
/// 冻结输入）；写入什么字节即校验与解析什么字节。
fn read_pinned_json(path: &Path, expected_sha: &str) -> Value {
    let bytes = fs::read(path).expect("pinned baseline input present");
    assert_eq!(
        hex_sha256(&bytes),
        expected_sha,
        "pinned baseline input digest"
    );
    serde_json::from_slice(&bytes).unwrap()
}

/// plan §5：两份基线输入经 `COHORT2_BASELINE_DIR` 指向 Release 资产解压目录只读读取。
fn pinned_baseline_inputs() -> (Value, Value) {
    let dir = PathBuf::from(std::env::var("COHORT2_BASELINE_DIR").expect("COHORT2_BASELINE_DIR"));
    (
        read_pinned_json(
            &dir.join("stream-a-searches.json"),
            STREAM_A_SEARCHES_SHA256,
        ),
        read_pinned_json(&dir.join("stream-a-results.json"), STREAM_A_RESULTS_SHA256),
    )
}

fn normalized(value: &Value) -> Value {
    serde_json::from_str(&value.to_string()).unwrap()
}

fn cohort() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1134-cohort2-coverage-diagnosis/edges.json"
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

/// plan §3 冻结问题生成规则：span 端点切向为边界钳制 → 端弦问题（方向锥 = 对应端
/// 钳制原 f64 位）；规定倒圆 → 端弦问题（不作方向条件）；interior CR → 跳过。
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

/// plan §2：默认档与历史 1103-base / 1107-old 记录逐值比较；`1103-expanded` 由扩展档
/// 步骤对照，`1107-grid` 仅定位版本（校准网格首次执行、无历史对照）。
fn assert_unchanged_history(
    legacy: &Value,
    old_rows: &Value,
    edge: &Value,
    id: &str,
    report: &Value,
    candidates: &Value,
) {
    for entry in edge["history"].as_array().unwrap() {
        let (record, label) = match entry["experiment"].as_str().unwrap() {
            "1103-base" => (
                legacy["tiers"]["32768"]["searches"]
                    .get(id)
                    .expect("1103-base record present"),
                "1103-base",
            ),
            "1107-old" => (
                old_rows.get(id).expect("1107-old record present"),
                "1107-old",
            ),
            "1103-expanded" | "1107-grid" => continue,
            other => panic!("unknown history experiment {other}"),
        };
        assert_eq!(
            normalized(report),
            record["search"],
            "unchanged {label} report for {id}"
        );
        assert_eq!(
            normalized(candidates),
            record["candidates"],
            "unchanged {label} candidates for {id}"
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

/// plan §5 冻结基线候选池：来源①→④（仓库内历史切片研究制品）按固定顺序合并并逐级
/// 断言（6/48→7/56→7/72→9/75），再合并 Release 资产 `stream-a-searches.json` 的新增
/// 候选（来源⑤，钉版输入）；总断言（10 边 76 份与每边份数）由 `pinned_baseline_pool`
/// 按钉版输入执行。任一来源缺失或逐值不符即停止执行并重新授权。
fn baseline_pool(stream_searches: &Value) -> combinations::Pools {
    let legacy: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1103-s1-recovery-budget/results.json"
    ))
    .unwrap();
    let old: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/old-searches.json"
    ))
    .unwrap();
    let grid: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1111-grid-calibration/positive-calibration.json"
    ))
    .unwrap();
    let curve: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1115-failed-curve-design/searches.json"
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
    merge(&mut pools, pool(stream_searches));
    pools
}

/// plan §5 钉版基线池：来源⑤为 Release 资产钉版输入（`COHORT2_BASELINE_DIR` 读取、
/// 代码内断言 SHA-256），合并后断言 10 边 76 份（边集与每边份数）。
fn pinned_baseline_pool(stream_searches: &Value) -> combinations::Pools {
    let pools = baseline_pool(stream_searches);
    assert_eq!(
        pool_counts(&pools),
        (10, 76),
        "source 5: 10 edges 76 candidates"
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
            ("sumo::-10564_3_0".to_owned(), 1),
            ("sumo::-7348_3_0".to_owned(), 16),
        ]),
        "frozen baseline per-edge candidate counts"
    );
    pools
}

/// plan §3 阶段二：零候选边按冻结问题表执行逐边局部必要条件诊断。三条
/// `endpoint_diagnosis_not_selected` 边不诊断（零候选只记搜索结果）；见证参数注入只把
/// 见证参数并入注入网格，不强制使用见证弦，尝试/使用证据由注入搜索逐条记录。
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

#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, DIRECT_SUCCESSOR_BASELINE_DIR, COHORT2_BASELINE_DIR (release asset extract), COVERAGE_ORDER, COVERAGE_DIAGNOSIS_ORDER and fresh COVERAGE_OUTPUT_DIR"]
fn pinned_lust_cohort2_coverage_diagnosis_experiment() {
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
    let (baseline_searches, stream_a_results) = pinned_baseline_inputs();
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
    let frozen: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/inventory.json"
    ))
    .unwrap();
    let (observation, source, mut timing) = capture_lust();
    assert_eq!(source, reference["source"]);
    assert_eq!(
        source, stream_a_results["source"],
        "result comparison source"
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
    assert_eq!(cohort_components.len(), 11, "eleven cohort components");
    assert_eq!(
        cohort_components
            .iter()
            .map(|component| component.paths.len())
            .sum::<usize>(),
        25,
        "complete associated paths"
    );
    assert_eq!(
        cohort_components
            .iter()
            .map(|component| component.target_paths.len())
            .sum::<usize>(),
        14,
        "original targets behind the 25 paths"
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
                assert_unchanged_history(
                    &legacy,
                    &old_rows,
                    edge_rows[id],
                    id,
                    &search.report,
                    &candidates,
                );
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
    let baseline = pinned_baseline_pool(&baseline_searches);
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
            if cohort_ids.contains(&component.id) {
                assert!(
                    component.controllers.is_empty(),
                    "cohort component has no controllers"
                );
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
            // plan §4：组合超 256 记 `assignment_budget_stopped` 与未访问赋值数；
            // 总组合按全部待赋值边的去重后池大小连乘计算。
            let unvisited = result
                .get("total_assignments")
                .and_then(Value::as_u64)
                .map(|total| total - result["trials"].as_array().unwrap().len() as u64);
            result["unvisited_assignments"] = json!(unvisited);
            if variant == "baseline" {
                let prior = &stream_a_results["variants"]["new"]["components"][index];
                assert_eq!(prior["component"], json!(component.id));
                assert_eq!(
                    normalized(&result),
                    prior["result"],
                    "baseline component matches stream-a variants.new"
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
                "cohort":cohort_ids.contains(&component.id),
                "artifact":(result["status"]=="accepted").then_some(file)}));
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
        for (current, previous) in paths.iter().zip(
            stream_a_results["variants"]["new"]["paths"]
                .as_array()
                .unwrap(),
        ) {
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
                stream_a_results["variants"]["new"]["paths"],
                "baseline target evidence matches stream-a variants.new"
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
            "complete_associated_paths":{"paths":25,"original_targets":14,
                "baseline_accepted":cohort_accepted(&baseline_result),
                "new_accepted":cohort_accepted(&new_result),
                "note":"25 条完整关联路径 ↔ 14 条原目标；7 个信号分量另对应 31 条原目标（14+31=45 为 #790 剩余全部原目标）"}}});
    recovery::write_json(
        &out,
        "results.json",
        &json!({"schema":"laneflow-1134-cohort2-coverage-diagnosis","source":source,
        "edges":EDGES,"expanded_tier_edges":EXPANDED_TIER_EDGES,
        "zero_candidate_edges":zero_edges,"grid_policy":sampler::grid::policy(),
        "old_policy":sampler::policy(sampler::SearchBudget::default()),
        "expanded_tier_partitions":EXPANDED_TIER_PARTITIONS,
        "cohort_source":"research/issue-1134-cohort2-coverage-diagnosis/edges.json",
        "method_source":"research/issue-1121-coverage-diagnosis/plan.md",
        "history_cross_check":{"edges_with_history":with_history,
            "first_run_edges":EDGES.len()-with_history,
            "value_comparisons":"1103-base/1107-old/1103-expanded 逐值对照通过；1107-grid 仅版本定位"},
        "baseline_pool":{"sources":["research/issue-1103-s1-recovery-budget/results.json",
            "research/issue-1107-dependency-recovery/old-searches.json",
            "research/issue-1111-grid-calibration/positive-calibration.json#edges",
            "research/issue-1115-failed-curve-design/searches.json",
            "research/issue-1134-cohort2-coverage-diagnosis/baseline/stream-a-searches.json"],
            "edge_count":pool_counts(&baseline).0,"candidate_count":pool_counts(&baseline).1,
            "per_edge":per_edge_counts(&baseline)},
        "new_pool":{"edge_count":pool_counts(&combined).0,"candidate_count":pool_counts(&combined).1,
            "per_edge":per_edge_counts(&combined)},
        "result_comparison":{"target":"research/issue-1134-cohort2-coverage-diagnosis/baseline/stream-a-results.json#variants.new",
            "role":"result comparison only; not a candidate point source",
            "components_result_equal":true,"paths_equal":true,"path_counts_equal":true,
            "accepted_paths":baseline_result["path_counts"]["accepted"],
            "note":"28/84 含 #1131 曲线候选；基线池 10 边 76 份含 baseline/stream-a-searches.json 的 1 份候选"},
        "coupling_note":cohort["coupling_note"],
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
        "cohort2 coverage diagnosis evidence saved={}",
        out.display()
    );
}

#[test]
fn cohort2_frozen_sample_and_problem_table_stay_consistent() {
    let cohort = cohort();
    assert_eq!(
        cohort["schema"],
        "laneflow-round3-cohort2-coverage-diagnosis"
    );
    assert_eq!(
        cohort["method_source"],
        "research/issue-1121-coverage-diagnosis/plan.md"
    );
    assert_eq!(cohort["summary"]["components"], 11);
    assert_eq!(cohort["summary"]["missing_edges"], 22);
    assert_eq!(cohort["summary"]["complete_paths"], 25);
    assert_eq!(cohort["summary"]["candidate_edges"], 1);
    assert_eq!(cohort["summary"]["controllers"], 0);
    assert_eq!(cohort["summary"]["stop_line_edges"], 0);
    assert_eq!(cohort["summary"]["single_missing_components"], 8);
    let components = cohort["components"].as_array().unwrap();
    assert_eq!(
        components
            .iter()
            .map(|row| row["code"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "C07", "C09", "C11", "C12", "C16", "C20", "C21", "C28", "C37", "C40", "C41"
        ],
        "frozen component code order"
    );
    assert_eq!(
        components
            .iter()
            .map(|row| row["path_count"].as_u64().unwrap() as usize)
            .sum::<usize>(),
        25
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
    let candidates = components
        .iter()
        .flat_map(|row| row_list(&row["candidate_edges"]))
        .collect::<Vec<_>>();
    assert_eq!(
        candidates,
        ["sumo::-29388_3_0"],
        "one frozen candidate edge"
    );
    assert_eq!(
        components
            .iter()
            .filter(|row| row_list(&row["missing_edges"]).len() == 1)
            .count(),
        8,
        "eight single-missing components"
    );
    assert_eq!(
        components
            .iter()
            .map(|row| row_assignment_edges(row).len())
            .collect::<Vec<_>>(),
        [5, 1, 1, 1, 4, 1, 1, 1, 1, 6, 1],
        "C07 五条待赋值边（8⁵ 口径）与 C16/C40 连乘口径逐值冻结"
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
    assert_eq!(problem_count, 21, "21 local problems");
    assert_eq!(
        cohort["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|edge| edge["diagnosis_status"] == "endpoint_diagnosis_not_selected")
            .map(|edge| edge["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["sumo::-10046_0_0", "sumo::-8018_4_0", "sumo::-25490_4_0"],
        "three endpoint-diagnosis-not-selected edges"
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
            let record = match entry["experiment"].as_str().unwrap() {
                "1103-base" => &legacy["tiers"]["32768"]["searches"][id],
                "1103-expanded" => &legacy["tiers"]["262144"]["searches"][id],
                "1107-old" => &old_rows[id],
                "1107-grid" => &grid_history[id],
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
        }
    }
    assert_eq!(
        (first_run, with_history),
        (9, 13),
        "nine first-run edges and thirteen with history"
    );
    let frozen: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/inventory.json"
    ))
    .unwrap();
    let c07 = &components[0];
    let record = frozen["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|component| component["id"] == c07["id"])
        .expect("C07 in frozen inventory");
    assert_eq!(
        record["missing_edges"].as_array().unwrap().len(),
        5,
        "C07五条待赋值边"
    );
    assert!(
        record["missing_edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge == "sumo::-29388_3_0"),
        "C07 includes the existing candidate edge"
    );
}

/// 临时目录唯一命名；基线读取验证的同字节内容只写临时目录，不落仓库。
fn temp_probe_dir() -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "laneflow-cohort2-baseline-{}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn cohort2_baseline_pool_merges_frozen_sources_with_pinned_input_bytes() {
    // plan §5 钉版值与计划文本一致（身份不依赖仓库内文件）。
    let plan = include_str!(
        "../../../../../../../../../research/issue-1134-cohort2-coverage-diagnosis/plan.md"
    );
    assert!(
        plan.contains(STREAM_A_SEARCHES_SHA256) && plan.contains(STREAM_A_RESULTS_SHA256),
        "plan §5 pins both baseline digests"
    );
    assert!(
        plan.contains("1,204,062") && plan.contains("584,379"),
        "plan §5 pins both baseline byte sizes"
    );
    // 来源①→④为仓库内历史切片研究制品，逐级断言 6/48→7/56→7/72→9/75；来源⑤按
    // plan §5 为 Release 资产钉版输入——本测试以临时目录写入同字节内容验证读取、
    // 钉版断言与合并路径；真实 10 边 76 份与每边份数总断言由 `pinned_baseline_pool`
    // 在正式实验入口按钉版输入执行。
    let candidate = json!({"program":{"start":[0,0,0],
        "segments":[{"end":[10.0_f64.to_bits(),0,0],"canvas_selection":null}]},
        "arc_m":10.0,"spans":[]});
    let source = json!({"sumo::cohort2-synthetic":{"candidates":[candidate]}});
    let bytes = serde_json::to_vec(&source).unwrap();
    let dir = temp_probe_dir();
    let path = dir.join("stream-a-searches.json");
    fs::write(&path, &bytes).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes, "same-byte write and read");
    let loaded = read_pinned_json(&path, &hex_sha256(&bytes));
    assert_eq!(loaded, source, "pinned loader parses the written bytes");
    let pools = baseline_pool(&loaded);
    assert_eq!(
        pool_counts(&pools),
        (10, 76),
        "sources 1-4 chain plus source 5"
    );
    assert_eq!(per_edge_counts(&pools)["sumo::cohort2-synthetic"], 1);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[should_panic(expected = "pinned baseline input digest")]
fn cohort2_pinned_baseline_loader_rejects_unpinned_bytes() {
    let dir = temp_probe_dir();
    let path = dir.join("stream-a-searches.json");
    fs::write(&path, b"not the pinned baseline bytes").unwrap();
    let _ = read_pinned_json(&path, STREAM_A_SEARCHES_SHA256);
}

#[test]
fn cohort2_span_control_fallback_preserves_existing_carriers() {
    let inputs: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/inputs.json"
    ))
    .unwrap();
    let grid_history: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/grid-searches.json"
    ))
    .unwrap();
    let empty = BTreeMap::new();
    // 既有载体（历史 1107 记录）可解析的诊断 span：三层链加入回退后逐值不变、来源仍为
    // history；重放只作交叉核对列。fresh 报告载体的 12 条边由正式实验逐问题断言同口径。
    for (id, span) in [
        ("sumo::-10100_1_0", 0),
        ("sumo::-1426_2_0", 2),
        ("sumo::-19726_2_0", 1),
        ("sumo::-22042_7_0", 3),
    ] {
        let input = measurement::input(&inputs[id]);
        let before = coverage::find_span_control(&grid_history[id]["search"], span)
            .expect("existing history carrier");
        let resolved = span_control(&empty, &grid_history, id, span, &input);
        assert_eq!(
            resolved.source, "history",
            "{id} keeps the existing carrier"
        );
        assert_eq!(
            resolved.control, before,
            "{id} control unchanged by fallback"
        );
        let replay = resolved
            .replay_control
            .expect("replay reaches the diagnosis span");
        assert_eq!(
            replay[..4],
            resolved.control[..4],
            "{id} replay cross-check"
        );
    }
    // 既有载体缺失的两条边（sumo::-10034_4_0、sumo::-1426_1_0，均 span 0）由重放回退
    // 解析（#1134 §7 重新授权）；注入匹配用完整六点，不再零填充。
    for (id, span) in [("sumo::-10034_4_0", 0), ("sumo::-1426_1_0", 0)] {
        let input = measurement::input(&inputs[id]);
        let resolved = span_control(&empty, &grid_history, id, span, &input);
        assert_eq!(
            resolved.source, "replay_fallback",
            "{id} uses the replay fallback"
        );
        assert_eq!(
            Some(resolved.control),
            resolved.replay_control,
            "{id} replay"
        );
        assert_eq!(
            resolved.control, resolved.injection_control,
            "{id} complete six points"
        );
        assert!(
            resolved.control[4] != [0.0, 0.0, 0.0] && resolved.control[5] != [0.0, 0.0, 0.0],
            "{id} replay carries real endpoint tangents"
        );
        for point in resolved.control {
            for value in point {
                assert!(
                    value.is_finite() && value.abs() <= 32_000.0,
                    "{id} frozen control bounds"
                );
            }
        }
    }
}
