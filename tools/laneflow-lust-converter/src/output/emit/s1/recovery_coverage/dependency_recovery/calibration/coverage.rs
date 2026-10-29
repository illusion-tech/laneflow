//! #1221：四个最小多边案例（C06/C08/C17/C29）的固定曲线搜索补齐、逐边局部必要条件
//! 诊断与候选齐备分量组合验收。执行口径全部冻结于
//! `research/issue-1121-coverage-diagnosis/plan.md`；本模块不改生产采样、阈值与原成功
//! 几何。计划与冻结实现不一致时停止执行并重新授权（计划 §7），不得单向迁就。

use super::super::experiment::{merge, pool};
use super::*;
use sampler::feasibility::EndChord;
use std::collections::BTreeSet;

/// §6.1 冻结搜索顺序：盘点编号序 C06、C08、C17、C29。
const EDGES: [&str; 8] = [
    "sumo::-10004_3_0",
    "sumo::-10004_8_0",
    "sumo:--30306#2_0",
    "sumo::-8806_4_0",
    "sumo::-10564_3_0",
    "sumo::-10564_6_0",
    "sumo:--30452_0",
    "sumo::-20112_0_0",
];

/// §2.3.2：扩展档 262,144 仅这两条有 `1103-expanded` 历史记录的边执行；其余边不跑。
const EXPANDED_TIER_EDGES: [&str; 2] = ["sumo::-10004_3_0", "sumo:--30452_0"];
const EXPANDED_TIER_PARTITIONS: u64 = 262_144;

#[derive(Clone, Copy)]
enum Direction {
    StartClamp,
    FinishClamp,
    /// 规定倒圆切向属曲线构造，不作离散弦方向条件（§3.1）。
    Fillet,
}

struct Problem {
    edge: &'static str,
    span_index_zero_based: usize,
    kinds: &'static [EndChord],
    direction: Direction,
}

/// §3.4 冻结逐边诊断问题表（顺序即 §6.2 诊断顺序）。
const PROBLEMS: [Problem; 8] = [
    Problem {
        edge: "sumo::-10564_6_0",
        span_index_zero_based: 0,
        kinds: &[EndChord::First],
        direction: Direction::StartClamp,
    },
    Problem {
        edge: "sumo:--30452_0",
        span_index_zero_based: 2,
        kinds: &[EndChord::Last],
        direction: Direction::FinishClamp,
    },
    Problem {
        edge: "sumo::-20112_0_0",
        span_index_zero_based: 0,
        kinds: &[EndChord::First],
        direction: Direction::StartClamp,
    },
    Problem {
        edge: "sumo::-10564_3_0",
        span_index_zero_based: 1,
        kinds: &[EndChord::Last],
        direction: Direction::FinishClamp,
    },
    Problem {
        edge: "sumo:--30306#2_0",
        span_index_zero_based: 0,
        kinds: &[EndChord::First],
        direction: Direction::StartClamp,
    },
    Problem {
        edge: "sumo::-8806_4_0",
        span_index_zero_based: 1,
        kinds: &[EndChord::Last],
        direction: Direction::Fillet,
    },
    Problem {
        edge: "sumo::-10004_3_0",
        span_index_zero_based: 1,
        kinds: &[EndChord::First, EndChord::Last],
        direction: Direction::Fillet,
    },
    Problem {
        edge: "sumo::-10004_8_0",
        span_index_zero_based: 1,
        kinds: &[EndChord::First, EndChord::Last],
        direction: Direction::Fillet,
    },
];

fn normalized(value: &Value) -> Value {
    serde_json::from_str(&value.to_string()).unwrap()
}

pub(super) fn coverage_output() -> PathBuf {
    let out = PathBuf::from(std::env::var("COVERAGE_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh coverage evidence output");
    fs::create_dir_all(&out).unwrap();
    out
}

/// §2.3.4 词汇映射：原始停止标签保留在 `report.stop_reason`，本函数只产出计划词汇。
/// `candidates_found`/`program_pool_full` 是结果维度（有候选），其余是终止原因维度。
pub(in super::super) fn plan_stop_label(report: &Value, candidate_programs: u64) -> &'static str {
    if candidate_programs > 0 {
        return if report["stop_reason"] == "program_pool" {
            "program_pool_full"
        } else {
            "candidates_found"
        };
    }
    match report["stop_reason"].as_str().unwrap() {
        "partitions" => "partitions_budget_stopped",
        "finite_family_exhausted" => "finite_family_exhausted",
        "retained_family_exhausted"
        | "preparation_or_straight_rejected"
        | "no_retained_grid_path" => "retained_family_exhausted",
        "work" | "evaluations" | "frames" | "retained_keys" | "retained_prefix"
        | "retained_points" | "sampling_work" | "source_fit_work" | "dedup_points" => {
            "work_budget_stopped"
        }
        other => panic!("unmapped stop label {other}"),
    }
}

/// 见证参数是否属于校准网格节点集合 {i/512}；按冻结算法直接计算，不查 `history`。
pub(in super::super) fn grid_node_member(t: f64) -> bool {
    let scaled = t * 512.0;
    (0.0..=512.0).contains(&scaled) && scaled.fract() == 0.0
}

pub(super) fn edge_input<'a>(
    observation: &'a observe::EmissionObservation,
    id: &str,
) -> &'a observe::EdgeInput {
    let index = observation
        .edges
        .iter()
        .position(|(edge, _)| edge == id)
        .unwrap();
    &observation.inputs[index]
}

pub(in super::super) fn find_span_control(search: &Value, span: usize) -> Option<[[f64; 3]; 6]> {
    if let Some(failures) = search["failures"].as_object() {
        for row in failures.values() {
            if row["span_index_zero_based"].as_u64() == Some(span as u64) {
                let bits: [[u64; 3]; 6] =
                    serde_json::from_value(row["control_f64_bits"].clone()).unwrap();
                return Some(bits.map(|p| p.map(f64::from_bits)));
            }
        }
    }
    if let Some(spans) = search["spans"].as_array() {
        for row in spans {
            if row["span_index_zero_based"].as_u64() == Some(span as u64) {
                let bits: [[u64; 3]; 4] =
                    serde_json::from_value(row["control_f64_bits"].clone()).unwrap();
                let mut full = [[0_u64; 3]; 6];
                full[..4].copy_from_slice(&bits);
                return Some(full.map(|p| p.map(f64::from_bits)));
            }
        }
    }
    None
}

/// 诊断目标 cubic 的控制点解析结果（#1134 §7 重新授权的三层载体链）。
pub(in super::super) struct SpanControl {
    /// 诊断用控制点：前四点为冻结参考 cubic 控制点，来源按 `source` 记录。
    pub(in super::super) control: [[f64; 3]; 6],
    /// 见证参数注入匹配用：完整六点（含 `dir_a`/`dir_b`），不再零填充。
    pub(in super::super) injection_control: [[f64; 3]; 6],
    /// `repair_curve` 重放的六点控制（交叉核对列；重放未到达该 span 时为 `None`）。
    pub(in super::super) replay_control: Option<[[f64; 3]; 6]>,
    /// `fresh_failures` / `history` / `replay_fallback` 三层来源标签。
    pub(in super::super) source: &'static str,
}

/// `repair_curve` 重放回退（#1134 §7 重新授权）：以测试观察逐 cubic 记录冻结输入原修复
/// 参考曲线的六点控制；只读重放，不改采样、阈值与预算。诊断 span 即原生产失败 span，
/// 重放必然到达并记录该 span。**截断语义**：遇发射失败即停、只保失败 span 及之前的
/// 控制点——诊断 span（=失败 span）必然被捕获，但**不可**用作全边参考；全边参考用
/// `geom::s1::reshape::reference_with_curve_cap`（观察模式全 span 构建 + 三断言）。
pub(in super::super) fn replay_span_controls(
    input: &observe::EdgeInput,
) -> BTreeMap<usize, [[f64; 3]; 6]> {
    let (_result, calls) = geom::sampling_observation::capture(|| {
        geom::repair_curve(
            &input.points,
            input.clamps[0],
            input.clamps[1],
            input.clamp_sources[0].as_ref(),
            input.clamp_sources[1].as_ref(),
        )
    });
    calls
        .iter()
        .filter(|call| call["depth"] == 0)
        .map(|call| {
            let bits: [[u64; 3]; 6] =
                serde_json::from_value(call["control_f64_bits"].clone()).unwrap();
            (
                call["span_index_zero_based"].as_u64().unwrap() as usize,
                bits.map(|p| p.map(f64::from_bits)),
            )
        })
        .collect()
}

/// 诊断目标 cubic 的控制点来源：本轮两族搜索记录优先，历史 1107 网格记录兜底，最后以
/// `repair_curve` 重放回退（#1134 §7 重新授权，追加在既有两层之后、解析结果不变）；
/// 三层全失才按计划 §7 停止并重新授权。
pub(in super::super) fn span_control(
    searches: &BTreeMap<String, Value>,
    grid_history: &Value,
    id: &str,
    span: usize,
    input: &observe::EdgeInput,
) -> SpanControl {
    let replay = replay_span_controls(input).remove(&span);
    let chain = ["grid", "old", "old-expanded"]
        .iter()
        .find_map(|family| {
            searches
                .get(*family)
                .and_then(|rows| rows.get(id))
                .and_then(|row| find_span_control(&row["search"], span))
                .map(|control| (control, "fresh_failures"))
        })
        .or_else(|| {
            grid_history
                .get(id)
                .and_then(|row| find_span_control(&row["search"], span))
                .map(|control| (control, "history"))
        });
    if let Some((control, source)) = chain {
        return SpanControl {
            control,
            injection_control: replay.unwrap_or(control),
            replay_control: replay,
            source,
        };
    }
    if let Some(control) = replay {
        return SpanControl {
            control,
            injection_control: control,
            replay_control: replay,
            source: "replay_fallback",
        };
    }
    panic!("span {span} control missing for {id}: 计划 §7 三层载体全失，停止该处执行并重新授权");
}

fn run_diagnosis(
    observation: &observe::EmissionObservation,
    searches: &BTreeMap<String, Value>,
    grid_history: &Value,
    zero_edges: &BTreeSet<&str>,
) -> (Value, Value) {
    let mut problems = Vec::new();
    let mut summaries = BTreeMap::<String, Value>::new();
    let mut timing = BTreeMap::<String, Value>::new();
    for problem in PROBLEMS {
        let id = problem.edge;
        let summary = summaries.entry(id.to_owned()).or_insert_with(|| {
            json!({"proven_fixed_conflicts":[],"local_witness_kinds":[],
                "injected_search_stop_labels":[],"has_stage1_candidates":!zero_edges.contains(id)})
        });
        if !zero_edges.contains(id) {
            problems.push(
                json!({"edge":id,"span_index_zero_based":problem.span_index_zero_based,
                "applicability":"not_applicable_edge_has_candidates"}),
            );
            continue;
        }
        let input = edge_input(observation, id);
        let direction = match problem.direction {
            Direction::StartClamp => {
                let clamp = input.clamps[0];
                assert!(clamp.is_some(), "计划 §3.4 起始边界钳制缺失，§7 停止");
                clamp
            }
            Direction::FinishClamp => {
                let clamp = input.clamps[1];
                assert!(clamp.is_some(), "计划 §3.4 末端边界钳制缺失，§7 停止");
                clamp
            }
            Direction::Fillet => None,
        };
        let direction_source = match problem.direction {
            Direction::StartClamp => "start_boundary_clamp",
            Direction::FinishClamp => "finish_boundary_clamp",
            Direction::Fillet => "fillet_not_a_direction_condition",
        };
        let span = span_control(
            searches,
            grid_history,
            id,
            problem.span_index_zero_based,
            input,
        );
        for &kind in problem.kinds {
            let kind_label = match kind {
                EndChord::First => "first_chord",
                EndChord::Last => "last_chord",
            };
            let started = Instant::now();
            let result = sampler::feasibility::diagnose_end_chord(span.control, kind, direction);
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
                    control: span.injection_control.map(|p| p.map(f64::to_bits)),
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
            problems.push(json!({"edge":id,"span_index_zero_based":problem.span_index_zero_based,
                "problem_kind":kind_label,"direction_source":direction_source,
                "applicability":"diagnosed","local_problem_result":result,
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
        "policy":{"injection_semantics":"见证参数注入不强制使用见证弦",
        "exclusion_scope":"单个局部问题全参数域排除即否定该固定 cubic 的完整发射；局部见证不构成可行证明",
        "box_limit":32_000_000_u64,"depth_limit":48_u32,"certificate_leaf_limit":65_536_usize}}),
        json!(timing),
    )
}

#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, DIRECT_SUCCESSOR_BASELINE_DIR, COVERAGE_ORDER, COVERAGE_DIAGNOSIS_ORDER and fresh COVERAGE_OUTPUT_DIR"]
fn pinned_lust_coverage_diagnosis_experiment() {
    let out = coverage_output();
    let order = std::env::var("COVERAGE_ORDER").unwrap();
    assert!(matches!(order.as_str(), "old-grid" | "grid-old"));
    let diagnosis_order = std::env::var("COVERAGE_DIAGNOSIS_ORDER").unwrap();
    assert!(matches!(
        diagnosis_order.as_str(),
        "diagnosis-first" | "baseline-first"
    ));
    let reference = reference();
    let prior: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/results.json"
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
    let legacy: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1103-s1-recovery-budget/results.json"
    ))
    .unwrap();
    let frozen: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/inventory.json"
    ))
    .unwrap();
    let cohort: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1121-coverage-diagnosis/edge-cohort.json"
    ))
    .unwrap();
    let (observation, source, mut timing) = capture_lust();
    assert_eq!(source, reference["source"]);
    assert_eq!(cohort["baseline"]["lust_commit"], source["lust_commit"]);
    let originals = original_programs(&observation);
    snapshot(&out, &observation, &originals);
    let traffic = observation.traffic.as_ref().unwrap();
    let components = components::inventory(traffic, &originals, &target_ids(&reference));
    assert_eq!(json!(components), frozen["components"]);
    let missing: BTreeSet<String> = cohort["components"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|row| row["failed_edges"].as_array().unwrap().iter())
        .filter(|edge| edge["currently_missing"].as_bool().unwrap())
        .map(|edge| edge["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        missing,
        EDGES
            .iter()
            .map(|id| (*id).to_owned())
            .collect::<BTreeSet<_>>(),
        "frozen cohort matches search set"
    );
    let cohort_ids = cohort["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_owned())
        .collect::<BTreeSet<_>>();
    let live_ids = json!(components)
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_owned())
        .collect::<BTreeSet<_>>();
    assert!(
        cohort_ids.is_subset(&live_ids),
        "cohort components present in frozen inventory"
    );
    measurement::write_inputs(
        &out,
        &observation,
        &json!({"samples":EDGES.iter().map(|id|json!({"edge":id})).collect::<Vec<_>>()}),
    );
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
                if let Some(reference) = old_rows.get(id) {
                    assert_eq!(
                        normalized(&search.report),
                        reference["search"],
                        "unchanged 1107 original-family report"
                    );
                    assert_eq!(
                        normalized(&candidates),
                        reference["candidates"],
                        "unchanged 1107 original candidates"
                    );
                }
                if let Some(reference) = legacy["tiers"]["32768"]["searches"].get(id) {
                    assert_eq!(
                        normalized(&search.report),
                        reference["search"],
                        "unchanged 1103 original-family report"
                    );
                    assert_eq!(
                        normalized(&candidates),
                        reference["candidates"],
                        "unchanged 1103 original candidates"
                    );
                }
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
                if let Some(reference) = legacy["tiers"]["262144"]["searches"].get(id) {
                    assert_eq!(
                        normalized(&search.report),
                        reference["search"],
                        "unchanged 1103-expanded report"
                    );
                    assert_eq!(
                        normalized(&candidates),
                        reference["candidates"],
                        "unchanged 1103-expanded candidates"
                    );
                }
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
            run_diagnosis(&observation, &searches, &grid_history, &zero_edges);
        diagnosis_seconds = started.elapsed().as_secs_f64();
    }
    let mut baseline = pool(&legacy["tiers"]["32768"]["searches"]);
    merge(&mut baseline, pool(&old_rows));
    assert_eq!(baseline.len(), 7);
    let mut combined = baseline.clone();
    merge(&mut combined, pool(&searches["old"]));
    if let Some(rows) = searches.get("old-expanded") {
        merge(&mut combined, pool(rows));
    }
    merge(&mut combined, pool(&searches["grid"]));
    let mut results = BTreeMap::new();
    let mut validation_times = BTreeMap::new();
    for (family, pools) in [("baseline", baseline), ("combined", combined)] {
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
            let file = format!("{family}-component-{index:02}.lfca");
            let mut result = combinations::evaluate(
                traffic,
                component,
                &originals,
                &pools,
                Some(&out.join(&file)),
            );
            if family == "baseline" {
                assert_eq!(
                    normalized(&result),
                    prior["variants"]["combined"]["components"][index]["result"],
                    "unchanged complete component baseline"
                );
            }
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
        for (row, prior_path) in paths
            .iter()
            .zip(prior["variants"]["combined"]["paths"].as_array().unwrap())
        {
            assert_eq!(row["path"], prior_path["path"]);
            if prior_path["result"]["status"] == "accepted" {
                assert_eq!(
                    row["result"]["status"], "accepted",
                    "no accepted target regression"
                );
            }
        }
        if family == "baseline" {
            assert_eq!(
                normalized(&json!(paths)),
                prior["variants"]["combined"]["paths"],
                "unchanged baseline target evidence"
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
        results.insert(
            family,
            json!({"components":rows,"paths":paths,"path_counts":counts,
            "pool_sizes":pools.iter().map(|(id,p)|(id,p.len())).collect::<BTreeMap<_,_>>()}),
        );
        validation_times.insert(family, started.elapsed().as_secs_f64());
    }
    if diagnosis_order == "baseline-first" {
        let started = Instant::now();
        (diagnosis, diagnosis_timing) =
            run_diagnosis(&observation, &searches, &grid_history, &zero_edges);
        diagnosis_seconds = started.elapsed().as_secs_f64();
    }
    recovery::write_json(&out, "diagnosis.json", &diagnosis);
    timing["search_order"] = json!(order);
    timing["diagnosis_order"] = json!(diagnosis_order);
    timing["diagnosis_seconds"] = json!(diagnosis_seconds);
    timing["diagnosis"] = diagnosis_timing;
    timing["edge_search_seconds"] = json!(search_times);
    timing["validation_seconds"] = json!(validation_times);
    recovery::write_json(
        &out,
        "results.json",
        &json!({"schema":"laneflow-1121-coverage-diagnosis","source":source,
        "edges":EDGES,"expanded_tier_edges":EXPANDED_TIER_EDGES,
        "zero_candidate_edges":zero_edges,"grid_policy":sampler::grid::policy(),
        "old_policy":sampler::policy(sampler::SearchBudget::default()),
        "expanded_tier_partitions":EXPANDED_TIER_PARTITIONS,
        "cohort_source":"research/issue-1121-coverage-diagnosis/edge-cohort.json",
        "variants":results,"all_original_geometry_and_controls_equal":true,
        "production_g1_accepted":false}),
    );
    recovery::write_json(&out, "timing.json", &timing);
    println!("coverage diagnosis evidence saved={}", out.display());
}

#[test]
fn plan_stop_labels_map_frozen_vocabulary() {
    assert_eq!(
        plan_stop_label(&json!({"stop_reason":"program_pool"}), 8),
        "program_pool_full"
    );
    assert_eq!(
        plan_stop_label(&json!({"stop_reason":"retained_family_exhausted"}), 3),
        "candidates_found"
    );
    assert_eq!(
        plan_stop_label(&json!({"stop_reason":"partitions"}), 0),
        "partitions_budget_stopped"
    );
    assert_eq!(
        plan_stop_label(&json!({"stop_reason":"work"}), 0),
        "work_budget_stopped"
    );
    assert_eq!(
        plan_stop_label(&json!({"stop_reason":"retained_keys"}), 0),
        "work_budget_stopped"
    );
    assert_eq!(
        plan_stop_label(&json!({"stop_reason":"finite_family_exhausted"}), 0),
        "finite_family_exhausted"
    );
    assert_eq!(
        plan_stop_label(&json!({"stop_reason":"retained_family_exhausted"}), 0),
        "retained_family_exhausted"
    );
}

#[test]
fn frozen_cohort_and_problem_table_stay_consistent() {
    let cohort: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1121-coverage-diagnosis/edge-cohort.json"
    ))
    .unwrap();
    let missing: BTreeSet<String> = cohort["components"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|row| row["failed_edges"].as_array().unwrap().iter())
        .filter(|edge| edge["currently_missing"].as_bool().unwrap())
        .map(|edge| edge["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(missing.len(), 8);
    for id in EDGES {
        assert!(missing.contains(id), "{id} in frozen cohort");
    }
    for id in EXPANDED_TIER_EDGES {
        assert!(EDGES.contains(&id), "expanded tier is a cohort subset");
    }
    let mut seen = BTreeSet::new();
    for problem in PROBLEMS {
        assert!(EDGES.contains(&problem.edge), "problem edge in cohort");
        assert!(seen.insert(problem.edge), "one problem per edge");
    }
    assert_eq!(seen.len(), 8);
}

#[test]
fn grid_membership_is_computed_from_the_frozen_grid() {
    assert!(grid_node_member(0.0));
    assert!(grid_node_member(0.5));
    assert!(grid_node_member(1.0));
    assert!(grid_node_member(371.0 / 512.0));
    assert!(!grid_node_member(0.726_552_963_256_836));
}
