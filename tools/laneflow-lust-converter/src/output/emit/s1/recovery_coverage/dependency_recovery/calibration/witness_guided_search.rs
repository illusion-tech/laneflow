//! #1154：见证引导整边搜索族——候选资格验证。对一次冻结的见证清单（47 项 =
//! 1 true / 7 false / 39 null；D 路 15 项 = 1/2/12，两组数字并列不得归并）执行
//! 注入变体整边搜索（首弦 `[0,t]+>t 节点`、移除 ≤t 旧节点；末弦对称；不强制见证弦），
//! 以现行接受口径全量验证候选资格。执行口径冻结于
//! `research/issue-1154-witness-guided-search/plan.md`；预算三计数器/共享作用域/
//! 遍历序按 Issue #1154 冻结草案 v2 与统筹裁决执行（池消费序 = 注入变体先行 →
//! 标准网格对照，注入先行为正式语义）。不改生产采样、compiler、阈值与原成功几何；
//! 见证变体候选不回写标准族池；不预期恢复 C24 或任何分量（plan §5）。

use super::coverage::{edge_input, plan_stop_label, replay_span_controls};
use super::shared_baseline::json_leaf_mismatch;
use super::*;
use crate::output::digest::hex_sha256;
use sampler::grid::{Injection, InjectionKind, WorkBudget};
use std::collections::BTreeSet;

/// P2-3：锚原文文本钉版（results-anchor.json 312,453 字节；数值对照一律走文本
/// token 域，不经解析域）。
const RESULTS_ANCHOR_RAW_SHA256: &str =
    "d025c4bf6cd2a6c815e9586124034d21c8232d6c9cb46266ad0a5727e658847b";

/// plan §3：注入搜索调用上限 = 清单条数 × 2（每见证首弦/末弦形态各至多一次）。
const INJECTION_CALLS_PER_WITNESS: usize = 2;

/// 统筹裁决的 ABBA 轴一：池消费序（注入先行为正式语义；网格先行轮次产出
/// 预算顺序敏感性证据）。
const CONSUMPTION_ORDERS: [&str; 2] = ["injection-first", "grid-first"];

fn inventory() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1154-witness-guided-search/witness-inventory.json"
    ))
    .unwrap()
}

fn witness_rows(inventory: &Value) -> &[Value] {
    inventory["witnesses"].as_array().unwrap()
}

/// 清单绑定的见证参数 t（首弦取参数位 [1]，末弦取 [0]；与冻结清单 `t_f64_bits` 一致）。
fn witness_t(row: &Value) -> f64 {
    f64::from_bits(row["t_f64_bits"].as_u64().unwrap())
}

/// 清单绑定的控制点（6×3 f64 位）。正式执行时与共享载体链解析的完整六点交叉核对
/// （plan §3：前四点一致，否则 §6 停止）。
fn witness_control(row: &Value) -> [[u64; 3]; 6] {
    serde_json::from_value(row["control_f64_bits"].clone()).unwrap()
}

/// 注入变体枚举（plan §3，统筹裁决）：每见证按首弦形态 → 末弦形态，各至多一次；
/// 形态参数域按 `InjectionKind` 冻结断言（首弦 t∈(0,1]，末弦 t∈[0,1)），域外形态记
/// `not_executed`、不发起调用。
fn injection_variants(t: f64) -> Vec<(&'static str, Option<InjectionKind>)> {
    vec![
        (
            "first_chord_form",
            (t > 0.0 && t <= 1.0).then_some(InjectionKind::FirstChord { t }),
        ),
        (
            "last_chord_form",
            ((0.0..1.0).contains(&t)).then_some(InjectionKind::LastChord { t }),
        ),
    ]
}

/// 每边共享网格工作预算（E 交付的 `WorkBudget` 句柄，plan §2）：标准网格与全部
/// 注入变体传**同一句柄**，入前按剩余预投影截断、退出按实耗扣减（`consumed+remaining`
/// 不变量由 E 单测钉版）；每边一池、不按见证重新给额度、先到即停。
/// 计费粒度：`work_budget_handle_shared_per_edge`（过渡期调用粒度计费标注保留为
/// 过渡记录，见结果 JSON `pool_accounting`）。
struct EdgeBudget {
    handle: WorkBudget,
}

/// (a) 资格验证消费的 typed 候选（见证变体产出；不回写标准族池）。
struct WitnessCandidate {
    variant: &'static str,
    program: re::RoadEditingCurveProgram,
    arc_m: f64,
}

impl EdgeBudget {
    fn new() -> Self {
        Self {
            handle: WorkBudget::per_edge_default(),
        }
    }

    fn grid_remaining(&self) -> u64 {
        self.handle.grid_work_remaining
    }

    fn exhausted(&self) -> bool {
        self.handle.grid_work_remaining == 0
    }
}

/// 单见证单形态的注入搜索结果记录（§3.6 证据口径；计时另行隔离）。同句柄计费。
fn run_injection(
    input: &observe::EdgeInput,
    variant: &'static str,
    kind: InjectionKind,
    budget: &mut EdgeBudget,
    control_override: [[u64; 3]; 6],
) -> (Value, Vec<WitnessCandidate>) {
    assert!(
        !budget.exhausted(),
        "grid work pool exhausted before injection call (plan §2 first-reach stop)"
    );
    let before = budget.handle.grid_work_consumed;
    let injection = Injection {
        control: control_override,
        kind,
    };
    let search = sampler::grid::search_injected_with_budget(input, injection, &mut budget.handle);
    let work = budget.handle.grid_work_consumed - before;
    let candidates = search
        .candidates
        .iter()
        .map(|c| WitnessCandidate {
            variant,
            program: c.repaired.program.clone(),
            arc_m: c.repaired.quantized_arc_meters,
        })
        .collect();
    (
        json!({
            "variant":variant,
            "search":search.report,
            "candidates":recovery::candidates(&search),
            "plan_stop_label":plan_stop_label(&search.report, search.candidates.len() as u64),
            "grid_work_consumed":work,
            "grid_work_consumed_total":budget.handle.grid_work_consumed,
            "grid_work_remaining":budget.handle.grid_work_remaining,
        }),
        candidates,
    )
}

/// 标准网格对照搜索（同句柄计费；消费序由统筹裁决定，注入先行为正式语义，
/// 对照得剩余量并如实记 `work_budget_stopped`）。
fn run_standard_grid(input: &observe::EdgeInput, budget: &mut EdgeBudget) -> Value {
    let before = budget.handle.grid_work_consumed;
    let search = if budget.exhausted() {
        None
    } else {
        Some(sampler::grid::search_audited_with_budget(
            input,
            &mut budget.handle,
        ))
    };
    let work = budget.handle.grid_work_consumed - before;
    match search {
        Some(search) => json!({
            "search":search.report,
            "candidates":recovery::candidates(&search),
            "plan_stop_label":plan_stop_label(&search.report, search.candidates.len() as u64),
            "grid_work_consumed":work,
            "grid_work_consumed_total":budget.handle.grid_work_consumed,
            "grid_work_remaining":budget.handle.grid_work_remaining,
        }),
        None => json!({"status":"not_executed","reason":"grid_work_pool_exhausted",
            "grid_work_consumed":0,
            "grid_work_consumed_total":budget.handle.grid_work_consumed,
            "grid_work_remaining":0}),
    }
}

/// 驱动核心（plan §3 遍历序 + 统筹裁决池消费序）：见证清单序 → 每见证注入变体序
/// （首弦 → 末弦）；每边一个 `WorkBudget` 句柄，消费序按 `consumption_order`
/// （`injection-first` 为正式语义；`grid-first` 轮次产出预算顺序敏感性证据）。
/// 注入控制点取共享载体链（`repair_curve` 只读重放）解析的完整六点，与清单绑定
/// `control_f64_bits` 前四点位级交叉核对；不一致即停止。
#[allow(dead_code)]
fn drive_witness_guided_searches(
    observation: &observe::EmissionObservation,
    consumption_order: &str,
) -> (Value, Value, BTreeMap<String, Vec<WitnessCandidate>>) {
    assert!(
        CONSUMPTION_ORDERS.contains(&consumption_order),
        "frozen consumption order vocabulary"
    );
    let inventory = inventory();
    let mut budgets: BTreeMap<String, EdgeBudget> = BTreeMap::new();
    let mut rows_out = Vec::new();
    let mut timing = BTreeMap::<String, Value>::new();
    let mut typed: BTreeMap<String, Vec<WitnessCandidate>> = BTreeMap::new();
    let mut call_count = 0_usize;
    let call_cap = witness_rows(&inventory).len() * INJECTION_CALLS_PER_WITNESS;
    // 见证清单序分组到边（保持清单序；同边多见证只按清单序）。
    let mut edge_order: Vec<String> = Vec::new();
    let mut by_edge: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for row in witness_rows(&inventory) {
        let edge = row["edge"].as_str().unwrap().to_owned();
        if !by_edge.contains_key(&edge) {
            edge_order.push(edge.clone());
        }
        by_edge.entry(edge).or_default().push(row);
    }
    for edge in &edge_order {
        let rows = by_edge.remove(edge).unwrap();
        let input = edge_input(observation, edge);
        let budget = budgets.entry(edge.clone()).or_insert_with(EdgeBudget::new);
        let run_injections =
            |rows: &[&Value],
             budget: &mut EdgeBudget,
             rows_out: &mut Vec<Value>,
             timing: &mut BTreeMap<String, Value>,
             call_count: &mut usize,
             typed: &mut BTreeMap<String, Vec<WitnessCandidate>>| {
                for row in rows {
                    let span = row["span_index_zero_based"].as_u64().unwrap() as usize;
                    let replayed = replay_span_controls(input)
                        .remove(&span)
                        .expect("replay reaches the witness span (frozen lane records resolve it)")
                        .map(|p| p.map(f64::to_bits));
                    let bound = witness_control(row);
                    assert_eq!(
                        replayed[..4],
                        bound[..4],
                        "injection control cross-check for {edge} span {span} (plan §3)"
                    );
                    let t = witness_t(row);
                    for (variant, kind) in injection_variants(t) {
                        let Some(kind) = kind else {
                            rows_out.push(json!({"edge":edge,"span_index_zero_based":span,
                            "variant":variant,"status":"not_executed",
                            "reason":"t_out_of_form_domain","t_f64_bits":row["t_f64_bits"]}));
                            continue;
                        };
                        if budget.exhausted() {
                            rows_out.push(json!({"edge":edge,"span_index_zero_based":span,
                            "variant":variant,"status":"not_executed",
                            "reason":"grid_work_pool_exhausted","t_f64_bits":row["t_f64_bits"]}));
                            continue;
                        }
                        assert!(*call_count < call_cap, "injection call cap = inventory × 2");
                        *call_count += 1;
                        let started = Instant::now();
                        let (record, cands) = run_injection(input, variant, kind, budget, replayed);
                        typed.entry(edge.to_owned()).or_default().extend(cands);
                        timing.insert(
                            format!("{edge}:{span}:{variant}"),
                            json!(started.elapsed().as_secs_f64()),
                        );
                        rows_out.push(json!({"edge":edge,"span_index_zero_based":span,
                        "variant":variant,"status":"executed","t_f64_bits":row["t_f64_bits"],
                        "result":record}));
                    }
                }
            };
        if consumption_order == "injection-first" {
            run_injections(
                &rows,
                budget,
                &mut rows_out,
                &mut timing,
                &mut call_count,
                &mut typed,
            );
            let started = Instant::now();
            let record = run_standard_grid(input, budget);
            timing.insert(
                format!("{edge}:standard_grid"),
                json!(started.elapsed().as_secs_f64()),
            );
            rows_out.push(json!({"edge":edge,"variant":"standard_grid","result":record}));
        } else {
            let started = Instant::now();
            let record = run_standard_grid(input, budget);
            timing.insert(
                format!("{edge}:standard_grid"),
                json!(started.elapsed().as_secs_f64()),
            );
            rows_out.push(json!({"edge":edge,"variant":"standard_grid","result":record}));
            run_injections(
                &rows,
                budget,
                &mut rows_out,
                &mut timing,
                &mut call_count,
                &mut typed,
            );
        }
    }
    (
        json!({"inventory_summary":inventory["summary"],
            "consumption_order":consumption_order,
            "consumption_order_semantics":"injection-first is the formal semantics (plan §2, lead ruling); grid-first rounds produce budget-order-sensitivity evidence",
            "call_cap":call_cap,"calls_made":call_count,
            "pool_accounting":"work_budget_handle_shared_per_edge",
            "rows":rows_out}),
        json!(timing),
        typed,
    )
}

/// 正式实验输出目录（每次必须为不存在的新目录）。
fn witness_output() -> PathBuf {
    let out = PathBuf::from(std::env::var("WITNESS_GUIDED_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh witness evidence output");
    fs::create_dir_all(&out).unwrap();
    out
}

/// 正式实验入口（四轮 ABBA；plan §6）。ABBA 轴：池消费序（`WITNESS_GUIDED_ORDER`：
/// `injection-first` 正式语义 / `grid-first` 顺序敏感性）× 验证对照序
/// （`WITNESS_GUIDED_VALIDATION_ORDER`：`baseline-new` / `new-baseline`）。
/// 确定性判据（统筹裁决）：同序对逐字节（00≡03、01≡02）；跨序差异为预算顺序敏感性
/// 证据，不是确定性失败。验收分层：(a) 候选资格（搜索内接受 + 双向拟合复核域独立
/// 计费 + 编译/LFCA/锚点）、(b) 停止线三情形（D 冻结 26 行口径复用）、(c) 分量接受
/// 分列；「未执行」与几何拒绝分列。
#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, DIRECT_SUCCESSOR_BASELINE_DIR, SIX_STREAMS_BASELINE_DIR, WITNESS_GUIDED_OUTPUT_DIR, WITNESS_GUIDED_ORDER, WITNESS_GUIDED_VALIDATION_ORDER"]
fn pinned_lust_witness_guided_search_experiment() {
    let out = witness_output();
    let order = std::env::var("WITNESS_GUIDED_ORDER").unwrap();
    let validation_order = std::env::var("WITNESS_GUIDED_VALIDATION_ORDER").unwrap();
    assert!(CONSUMPTION_ORDERS.contains(&order.as_str()));
    assert!(matches!(
        validation_order.as_str(),
        "baseline-new" | "new-baseline"
    ));
    let reference = reference();
    let baseline = super::shared_baseline::shared_pools();
    let comparison = super::shared_baseline::results_anchor_pinned();
    let (observation, source, mut timing) = capture_lust();
    assert_eq!(source, reference["source"]);
    assert_eq!(
        comparison["source"]["comparison_only"], true,
        "comparison-only anchor"
    );
    // P2-3 前置：保锚原文文本（钉版哈希断言），数值对照一律走文本 token 域。
    let anchor_text = fs::read_to_string(
        PathBuf::from(std::env::var("SIX_STREAMS_BASELINE_DIR").unwrap())
            .join("results-anchor.json"),
    )
    .unwrap();
    assert_eq!(
        hex_sha256(anchor_text.as_bytes()),
        RESULTS_ANCHOR_RAW_SHA256,
        "anchor raw text identity"
    );
    let components_raw = raw_json_member_text(&anchor_text, "components");
    // P2-2 修复（Codex 评审）：见证涉及口径 = 冻结见证清单边集合（执行前确定；
    // 饿死/跳过执行的边仍在统计集并单独报出），不从执行后 map 派生。
    let frozen_witness_edges: BTreeSet<String> = witness_rows(&inventory())
        .iter()
        .map(|row| row["edge"].as_str().unwrap().to_owned())
        .collect();
    let originals = original_programs(&observation);
    snapshot(&out, &observation, &originals);
    let traffic = observation.traffic.as_ref().unwrap();
    let components = components::inventory(traffic, &originals, &target_ids(&reference));
    let frozen: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/inventory.json"
    ))
    .unwrap();
    assert_eq!(json!(components), frozen["components"]);
    assert_eq!(components.len(), 53, "frozen component inventory");
    assert_eq!(components.iter().map(|c| c.paths.len()).sum::<usize>(), 183);
    // 见证引导搜索（正式语义 = injection-first；grid-first 轮次为顺序敏感性轴）。
    let (search_evidence, search_timing, witness_candidates) =
        drive_witness_guided_searches(&observation, &order);
    timing["witness_searches"] = search_timing;
    recovery::write_json(&out, "witness-guided-searches.json", &search_evidence);
    // (a) 候选资格验证（P2-1 修复后口径）：搜索内接受（实际长度/sink 转角/0.01 m）
    // + 双向拟合（复核域独立计费）+ 编译/LFCA/锚点全量验证。全 span 参考（独立于
    // 发射成败、覆盖断言）；**正式判定 = 同表示 B 口径**（候选折线 vs 原参考发射
    // 折线，同一估计函数；维护者裁定 2026-10-30），跨表示（vs 参考曲线）仅作表示
    // 伪影证据列。复核域计数独立报出（lead 裁决）。
    let mut qualification_rows = Vec::new();
    let mut witness_pool: combinations::Pools = BTreeMap::new();
    let mut review_fit_calls = 0_u64;
    let mut review_fit_consumed_total = 0_u64;
    let mut qi = 0_usize;
    for (edge, cands) in &witness_candidates {
        let input = edge_input(&observation, edge);
        let mut review_handle = WorkBudget::per_edge_default();
        // 复核域计数器（lead 裁决 B 案：只记 consumed、不占搜索池）：验证算量为固定
        // 确定性负载（全 span 发射折线拟合），不设池上限截断，实耗如实单列。
        review_handle.source_fit_remaining = u64::MAX;
        let reference_controls = full_reference_controls(input);
        let reference_polyline = reference_emitted_polyline(&reference_controls);
        let reference_polyline_controls = polyline_as_controls(&reference_polyline);
        review_fit_calls += 1;
        let reference_curve_fit = sampler::reshape::quality::fit_with_handle(
            &reference_controls,
            &input.points,
            &mut review_handle,
        )
        .expect("bounded reference curve fit");
        review_fit_calls += 1;
        let reference_polyline_fit = sampler::reshape::quality::fit_with_handle(
            &reference_polyline_controls,
            &input.points,
            &mut review_handle,
        )
        .expect("bounded reference polyline fit");
        let component = components
            .iter()
            .find(|c| c.missing_edges.iter().any(|m| m == edge))
            .expect("witness edge belongs to a frozen component");
        let part = combinations::part(traffic, component);
        let path = &part.maneuver_paths[0];
        for cand in cands {
            let points = program_points(&cand.program);
            let candidate_controls = polyline_as_controls(&points);
            review_fit_calls += 1;
            let candidate_fit = sampler::reshape::quality::fit_with_handle(
                &candidate_controls,
                &input.points,
                &mut review_handle,
            );
            let (fit_row, fit_pass) = match candidate_fit {
                Some(fit) => {
                    let same_pass = fit.within(reference_polyline_fit);
                    let cross_pass = fit.within(reference_curve_fit);
                    (
                        json!({"status":"executed",
                        "same_representation":{"pass":same_pass,
                            "screen":"candidate polyline not worse than reference emitted polyline (same estimator; maintainer ruling 2026-10-30 B)",
                            "candidate":{"curve_to_source_estimate_m":fit.curve_to_source_estimate_m,
                                "source_to_curve_estimate_m":fit.source_to_curve_estimate_m},
                            "reference_emitted_polyline":{
                                "curve_to_source_estimate_m":reference_polyline_fit.curve_to_source_estimate_m,
                                "source_to_curve_estimate_m":reference_polyline_fit.source_to_curve_estimate_m,
                                "points":reference_polyline.len()}},
                        "cross_representation_artifact":{"pass":cross_pass,
                            "reference_curves":{
                                "curve_to_source_estimate_m":reference_curve_fit.curve_to_source_estimate_m,
                                "source_to_curve_estimate_m":reference_curve_fit.source_to_curve_estimate_m}}}),
                        same_pass,
                    )
                }
                None => (
                    json!({"status":"not_executed",
                        "reason":"review_domain_fit_budget_exhausted"}),
                    false,
                ),
            };
            let mut programs = originals.clone();
            programs.insert(edge.clone(), cand.program.clone());
            let file = format!("qualify-{qi:03}-{edge}.lfca");
            qi += 1;
            let result = compile_part(traffic, path, &part, &programs, Some(&out.join(&file)));
            let compile_outcome = match result["status"].as_str().unwrap() {
                "accepted" => "accepted",
                "target_check_not_completed" => "not_executed",
                _ => "geometry_rejected",
            };
            let outcome = match (
                fit_row["status"].as_str().unwrap(),
                fit_pass,
                compile_outcome,
            ) {
                ("not_executed", _, _) => "not_executed",
                (_, false, _) => "fit_rejected",
                (_, _, "geometry_rejected") => "geometry_rejected",
                (_, _, "not_executed") => "not_executed",
                _ => "qualified",
            };
            // 池入规则：几何/拟合拒绝排除；qualified 与 not_executed（依赖不齐、
            // 完整验证随 (c) 联合编译执行）入本轮新增池，不回写标准族池。
            if outcome == "qualified" || outcome == "not_executed" {
                witness_pool
                    .entry(edge.clone())
                    .or_default()
                    .push(cand.program.clone());
            }
            qualification_rows.push(json!({"edge":edge,"variant":cand.variant,
                "arc_m":cand.arc_m,"program":program_bits(&cand.program),
                "in_search_acceptance":"actual_length/sink_angle/0.01m_envelope",
                "source_fit_review_domain":fit_row,
                "compile_validation":result,"outcome":outcome}));
        }
        review_fit_consumed_total += review_handle.source_fit_consumed;
    }
    recovery::write_json(
        &out,
        "qualification.json",
        &json!({"rows":qualification_rows,
        "review_domain_counts":{"fit_evaluations":review_fit_calls,
            "fit_consumed_source_fit_units":review_fit_consumed_total,
            "compile_validations":qi,"lfca_artifacts":qi},
        "review_domain_billing":"验收层验证计入复核域、独立计数（lead 裁决，冻结计划复核独立计数条款）；不与搜索域三计数器混列、不占搜索域每边 32M source_fit 池"}),
    );
    // (c) 分量接受分列（基线/见证两组，验证对照序为 ABBA 轴二）。
    let mut combined = baseline.clone();
    for (edge, cands) in &witness_pool {
        combined
            .entry(edge.clone())
            .or_default()
            .extend(cands.iter().cloned());
    }
    let variants: Vec<(&str, &combinations::Pools)> = if validation_order == "baseline-new" {
        vec![("baseline", &baseline), ("witness", &combined)]
    } else {
        vec![("witness", &combined), ("baseline", &baseline)]
    };
    let mut results = BTreeMap::new();
    let mut final_programs = originals.clone();
    let mut evaluations = BTreeMap::new();
    let mut validation_times = BTreeMap::<String, Value>::new();
    for (variant, pools) in variants {
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
            let mut result = combinations::evaluate(
                traffic,
                component,
                &originals,
                pools,
                Some(&out.join(&file)),
            );
            let unvisited = result
                .get("total_assignments")
                .and_then(Value::as_u64)
                .map(|total| total - result["trials"].as_array().unwrap().len() as u64);
            result["unvisited_assignments"] = json!(unvisited);
            if variant == "baseline" {
                // P2-3 修复（Codex 评审）：锚原文文本按字节切片、文本 token 域比较
                // （`json_leaf_mismatch`，数字叶 f64::from_str 位级），不双侧 normalize
                // 进解析域（serde 1 ULP 坑）。组件行比 `result` 成员原文（行内 `cohort`
                // 标记随各路冻结分量不同，属行级元数据不入对照）。
                let prior_row_text = raw_json_array_element(components_raw, index);
                assert_eq!(
                    raw_json_value_text(raw_json_member_text(prior_row_text, "component")),
                    json!(component.id).to_string(),
                    "baseline component id matches the result comparison anchor (text domain)"
                );
                assert_eq!(
                    json_leaf_mismatch(
                        raw_json_value_text(raw_json_member_text(prior_row_text, "result")),
                        &result.to_string()
                    ),
                    None,
                    "baseline component matches the result comparison anchor (text token domain)"
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
            if variant == "witness" {
                evaluations.insert(component.id.clone(), result.clone());
            }
            let affected = component
                .missing_edges
                .iter()
                .any(|m| frozen_witness_edges.contains(m));
            rows.push(json!({"component":component.id,"result":result,
                "witness_affected":affected}));
        }
        if variant == "witness" {
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
        let counts = paths
            .iter()
            .fold(BTreeMap::<String, usize>::new(), |mut counts, row| {
                *counts
                    .entry(row["result"]["status"].as_str().unwrap().to_owned())
                    .or_default() += 1;
                counts
            });
        results.insert(
            variant,
            json!({"components":rows,"paths":paths,"path_counts":counts,
                "pool_sizes":pools.iter().map(|(id,p)|(id,p.len())).collect::<BTreeMap<_,_>>()}),
        );
        validation_times.insert(variant.to_owned(), json!(started.elapsed().as_secs_f64()));
    }
    // (b) 停止线三情形（D 冻结 26 行口径复用；见证涉及停止线边随本轮结果归情形；
    // 依赖不齐记「未执行」，不混入几何拒绝统计）。
    let d_cohort: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1139-signal-diag-2/edges.json"
    ))
    .unwrap();
    let checklist = super::signal_diag_2::stop_line_checklist(
        &d_cohort,
        traffic,
        &originals,
        &final_programs,
        &evaluations,
    );
    recovery::write_json(&out, "stop-line-checklist.json", &checklist);
    // 两口径收益分列（84 原目标 / 见证涉及关联路径）。
    let witness_result = results.get("witness").unwrap();
    let baseline_result = results.get("baseline").unwrap();
    let affected_paths = components
        .iter()
        .filter(|c| {
            c.missing_edges
                .iter()
                .any(|m| frozen_witness_edges.contains(m))
        })
        .flat_map(|c| c.paths.iter().cloned())
        .collect::<BTreeSet<_>>();
    // P2-2：跳过执行单独报出（饿死/域外跳过的冻结边，仍在统计集）。
    let executed_injection_edges: BTreeSet<String> = search_evidence["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["status"] == "executed" && row["variant"] != "standard_grid")
        .map(|row| row["edge"].as_str().unwrap().to_owned())
        .collect();
    let skipped_execution: Vec<&String> = frozen_witness_edges
        .iter()
        .filter(|edge| !executed_injection_edges.contains(*edge))
        .collect();
    let affected_rows = witness_result["components"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["witness_affected"] == true)
        .collect::<Vec<_>>();
    let newly_accepted = affected_rows
        .iter()
        .filter(|row| row["result"]["status"] == "accepted")
        .map(|row| row["component"].clone())
        .collect::<Vec<_>>();
    recovery::write_json(
        &out,
        "results.json",
        &json!({"schema":"laneflow-1154-witness-guided-search","source":source,
            "inventory_summary":inventory()["summary"],
            "abba_axes":{"consumption_order":order,"validation_order":validation_order,
                "determinism":"same-order pairs byte-equal (00==03, 01==02); cross-order differences are budget-order-sensitivity evidence, not determinism failures"},
            "pool_accounting":"work_budget_handle_shared_per_edge (search domain); review-domain fit billed independently per lead ruling",
            "witness_pool":{"edges":witness_pool.iter().map(|(id,p)|(id,p.len())).collect::<BTreeMap<_,_>>(),
                "rule":"witness-variant candidates never merge into standard-family pools; geometry-rejected excluded; dependency-incomplete marked pending full validation via (c) joint compile"},
            "tiers":{"a_qualification":"qualification.json","b_stop_line_checklist":"stop-line-checklist.json",
                "b_cases_summary":checklist["cases_summary"],
                "c_components":results.keys().cloned().collect::<Vec<_>>(),
                "c_newly_accepted_affected":newly_accepted,
                "c_witness_affected_components":affected_rows.len()},
            "gain_by_accounting":{
                "original_targets_84":{"baseline_accepted":baseline_result["path_counts"]["accepted"],
                    "witness_accepted":witness_result["path_counts"]["accepted"]},
                "witness_affected_paths":{"paths":affected_paths.len(),
                    "note":"见证涉及关联路径口径（含分量完整路径）；成员资格取冻结见证清单；两口径分列、新增收益不并入基线"}},
            "witness_statistics_set":{"frozen_edges":frozen_witness_edges.len(),
                "skipped_execution_edges":skipped_execution,
                "note":"P2-2：成员资格取冻结见证清单（执行前确定，不从执行后 map 派生）；跳过执行（饿死/域外）单独报出、仍在统计集"},
            "variants":results,
            "stop_line_note":"D 冻结 26 行三情形口径复用；见证涉及停止线边本轮结果归情形；依赖不齐记未执行",
            "all_original_geometry_and_controls_equal":true,"production_g1_accepted":false}),
    );
    timing["validation_order"] = json!(validation_order);
    timing["validation_seconds"] = json!(validation_times);
    recovery::write_json(&out, "timing.json", &timing);
    println!("witness guided search evidence saved={}", out.display());
}

/// 口径修订复算（维护者裁定 2026-10-30，B 同表示基准；plan §4 修订记录）：**只重算
/// (a) 列双向拟合资格**——候选折线 vs 原参考的**发射折线**（全 span 参考经 513 节点
/// 均匀网格采样后 f32 规范化链接；同一 `quality::fit` 估计函数、折线对折线同表示
/// 公平比较），"不劣于原参考" = `within`（双向上界估计均 ≤ 原参考发射折线）。不重跑
/// 搜索。历史注记：原跨表示 8/8 拒绝及参考反向 125.19 m 为**截断参考**所致（P2-1：
/// `replay_span_controls` 遇发射失败即截断，`--30266_0` 失败于 span 0/6 只捕获
/// span 0），作历史证据随输入原样保留（`cross_representation_artifact` 列）。复核域
/// 独立计费同 plan §4。若出现 qualified → (a) 入池 → (c) 终验 → 两口径收益更新；
/// 池空则 (c) 记 not_triggered。
#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, DIRECT_SUCCESSOR_BASELINE_DIR, SIX_STREAMS_BASELINE_DIR, WITNESS_GUIDED_RECOMPUTE_INPUT (run-00 evidence dir), fresh WITNESS_GUIDED_RECOMPUTE_OUTPUT"]
fn recompute_witness_qualification_same_representation() {
    let input_dir = PathBuf::from(std::env::var("WITNESS_GUIDED_RECOMPUTE_INPUT").unwrap());
    let out = PathBuf::from(std::env::var("WITNESS_GUIDED_RECOMPUTE_OUTPUT").unwrap());
    assert!(!out.exists(), "fresh recompute output");
    fs::create_dir_all(&out).unwrap();
    let prior: Value =
        serde_json::from_slice(&fs::read(input_dir.join("qualification.json")).unwrap()).unwrap();
    let (observation, _source, mut timing) = capture_lust();
    let started_all = Instant::now();
    let mut rows = Vec::new();
    let mut review_fit_calls = 0_u64;
    let mut review_fit_consumed = 0_u64;
    let mut qualified: BTreeMap<String, Vec<(String, re::RoadEditingCurveProgram, f64)>> =
        BTreeMap::new();
    for prior_row in prior["rows"].as_array().unwrap() {
        let edge = prior_row["edge"].as_str().unwrap();
        let variant = prior_row["variant"].as_str().unwrap().to_owned();
        let input = edge_input(&observation, edge);
        let mut review_handle = WorkBudget::per_edge_default();
        // 复核域计数器（lead B 案：只记 consumed 不占池，不截断验证算量）。
        review_handle.source_fit_remaining = u64::MAX;
        // 原参考的发射折线（P2-1 修复后）：全 span 参考（独立于发射成败、覆盖断言）
        // 经 513 网格采样 f32 规范化（`reference_emitted_polyline`，与主入口同一基元）。
        let reference_controls = full_reference_controls(input);
        let reference_points = reference_emitted_polyline(&reference_controls);
        // 候选折线：规范程序点位（start + 各 line 段 end；见证候选全为直线段，
        // canvas_selection 为 null——断言保持该形态）。
        let mut candidate_points = vec![from_bits3(prior_row["program"]["start"].clone())];
        for segment in prior_row["program"]["segments"].as_array().unwrap() {
            assert!(
                segment["canvas_selection"].is_null(),
                "line-segment candidates only (frozen run-00 shape)"
            );
            let p = from_bits3(segment["end"].clone());
            if candidate_points.last() != Some(&p) {
                candidate_points.push(p);
            }
        }
        let reference_controls = polyline_as_controls(&reference_points);
        let candidate_controls = polyline_as_controls(&candidate_points);
        review_fit_calls += 1;
        let reference_fit = sampler::reshape::quality::fit_with_handle(
            &reference_controls,
            &input.points,
            &mut review_handle,
        )
        .expect("bounded reference polyline fit");
        review_fit_calls += 1;
        let candidate_fit = sampler::reshape::quality::fit_with_handle(
            &candidate_controls,
            &input.points,
            &mut review_handle,
        );
        let (fit_row, fit_pass) = match candidate_fit {
            Some(fit) => (
                json!({"status":"executed","pass":fit.within(reference_fit),
                    "candidate":{"curve_to_source_estimate_m":fit.curve_to_source_estimate_m,
                        "source_to_curve_estimate_m":fit.source_to_curve_estimate_m},
                    "reference_emitted_polyline":{"curve_to_source_estimate_m":reference_fit.curve_to_source_estimate_m,
                        "source_to_curve_estimate_m":reference_fit.source_to_curve_estimate_m,
                        "points":reference_points.len()},
                    "screen":"candidate polyline not worse than reference emitted polyline (same estimator, same representation; maintainer ruling 2026-10-30)"}),
                fit.within(reference_fit),
            ),
            None => (
                json!({"status":"not_executed","reason":"review_domain_fit_budget_exhausted"}),
                false,
            ),
        };
        review_fit_consumed += review_handle.source_fit_consumed;
        let outcome = if fit_row["status"] == "not_executed" {
            "not_executed"
        } else if fit_pass {
            "qualified"
        } else {
            "fit_rejected"
        };
        if outcome == "qualified" {
            let start = from_bits3(prior_row["program"]["start"].clone());
            let segments = prior_row["program"]["segments"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| re::RoadEditingCurveSegment::line(point3(from_bits3(s["end"].clone()))))
                .collect::<Vec<_>>();
            let program = re::RoadEditingCurveProgram::try_new(point3(start), segments).unwrap();
            qualified.entry(edge.to_owned()).or_default().push((
                variant,
                program,
                prior_row["arc_m"].as_f64().unwrap(),
            ));
        }
        rows.push(
            json!({"edge":edge,"variant":prior_row["variant"],"arc_m":prior_row["arc_m"],
            "candidate_points":candidate_points.len(),
            "same_representation_fit":fit_row,
            "cross_representation_artifact":prior_row["source_fit_review_domain"],
            "compile_validation_reused":prior_row["compile_validation"],
            "outcome":outcome}),
        );
    }
    // (c) 终验：qualified 候选入池（不回写标准族池）→ 联合赋值/LFCA/锚点真实编译；
    // 池空记 not_triggered（lead 裁定执行序）。
    let mut c_record = json!({"c_final_validation":"not_triggered","reason":"no qualified candidates under same-representation screen"});
    if !qualified.is_empty() {
        let reference = reference();
        let baseline = super::shared_baseline::shared_pools();
        let mut combined = baseline.clone();
        for (edge, cands) in &qualified {
            combined
                .entry(edge.clone())
                .or_default()
                .extend(cands.iter().map(|(_, p, _)| p.clone()));
        }
        let originals = original_programs(&observation);
        let traffic = observation.traffic.as_ref().unwrap();
        let components = components::inventory(traffic, &originals, &target_ids(&reference));
        let mut evaluations = BTreeMap::new();
        let mut final_programs = originals.clone();
        let mut accepted_components = Vec::new();
        for (index, component) in components.iter().enumerate() {
            let file = format!("recompute-component-{index:02}.lfca");
            let mut result = combinations::evaluate(
                traffic,
                component,
                &originals,
                &combined,
                Some(&out.join(&file)),
            );
            let unvisited = result
                .get("total_assignments")
                .and_then(Value::as_u64)
                .map(|total| total - result["trials"].as_array().unwrap().len() as u64);
            result["unvisited_assignments"] = json!(unvisited);
            if result["status"] == "accepted" {
                for (id, choice) in result["selected_assignment"].as_object().unwrap() {
                    final_programs.insert(
                        id.clone(),
                        combined[id][choice.as_u64().unwrap() as usize].clone(),
                    );
                }
                accepted_components.push(json!(component.id));
            }
            evaluations.insert(component.id.clone(), result);
        }
        let d_cohort: Value = serde_json::from_str(include_str!(
            "../../../../../../../../../research/issue-1139-signal-diag-2/edges.json"
        ))
        .unwrap();
        let checklist = super::signal_diag_2::stop_line_checklist(
            &d_cohort,
            traffic,
            &originals,
            &final_programs,
            &evaluations,
        );
        recovery::write_json(&out, "stop-line-checklist-recompute.json", &checklist);
        c_record = json!({"c_final_validation":"executed",
            "accepted_components":accepted_components,
            "stop_line_checklist":"stop-line-checklist-recompute.json",
            "two_accounting_note":"两口径收益按接受组件与路径在结果中分列更新"});
    }
    recovery::write_json(
        &out,
        "qualification-recompute.json",
        &json!({"schema":"laneflow-1154-same-representation-recompute",
            "rule":"maintainer ruling 2026-10-30 B: candidate polyline vs reference emitted polyline (same estimator, same representation)",
            "reference_emitted_polyline_sampling":"per-span cubic first-four-points at 513 uniform parameter nodes, f32 canonicalized, chained (grid-family base emission form)",
            "rows":rows,
            "review_domain_counts":{"fit_evaluations":review_fit_calls,
                "fit_consumed_source_fit_units":review_fit_consumed},
            "c_stage":c_record}),
    );
    timing["recompute_seconds"] = json!(started_all.elapsed().as_secs_f64());
    recovery::write_json(&out, "timing.json", &timing);
    println!("same-representation recompute saved={}", out.display());
}

/// P2-3 原文切片基元：值尾定位（字符串感知的括号配平；标量截到分隔符）。不经解析域，
/// 数字拼写逐字保留（serde 1 ULP 坑免疫）。
fn raw_json_value_end(bytes: &[u8], start: usize) -> usize {
    match bytes[start] {
        b'"' => {
            let mut i = start + 1;
            while bytes[i] != b'"' {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i + 1
        }
        b'{' | b'[' => {
            let mut i = start;
            let mut depth = 0_i32;
            loop {
                match bytes[i] {
                    b'"' => {
                        i += 1;
                        while bytes[i] != b'"' {
                            if bytes[i] == b'\\' {
                                i += 1;
                            }
                            i += 1;
                        }
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return i + 1;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
        }
        _ => {
            let mut i = start;
            while i < bytes.len()
                && !matches!(bytes[i], b',' | b'}' | b']')
                && !bytes[i].is_ascii_whitespace()
            {
                i += 1;
            }
            i
        }
    }
}

/// 键成员值原文切片（`"key":` 后的完整值域）。
fn raw_json_member_text<'a>(text: &'a str, key: &str) -> &'a str {
    let pattern = format!("\"{key}\":");
    let start = text
        .find(&pattern)
        .unwrap_or_else(|| panic!("member {key} in anchor text"))
        + pattern.len();
    let bytes = text.as_bytes();
    let mut i = start;
    while bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    &text[i..raw_json_value_end(bytes, i)]
}

/// 值原文收紧（容器整体保留；标量截到值尾）。
fn raw_json_value_text(value: &str) -> &str {
    let bytes = value.as_bytes();
    match bytes[0] {
        b'{' | b'[' => value,
        _ => &value[..raw_json_value_end(bytes, 0)],
    }
}

/// 数组第 index 个元素原文切片（顶层元素序）。
fn raw_json_array_element(array_text: &str, index: usize) -> &str {
    let bytes = array_text.as_bytes();
    assert_eq!(bytes[0], b'[', "array text");
    let mut i = 1;
    let mut current = 0_usize;
    loop {
        while bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes[i] == b']' {
            panic!("array element {index} out of range");
        }
        let end = raw_json_value_end(bytes, i);
        if current == index {
            return &array_text[i..end];
        }
        current += 1;
        i = end;
        while bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        assert_eq!(bytes[i], b',', "array element separator");
        i += 1;
    }
}

fn cubic_bezier(a: [f64; 3], c1: [f64; 3], c2: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    let u = 1.0 - t;
    let mut p = [0.0; 3];
    for i in 0..3 {
        p[i] =
            u * u * u * a[i] + 3.0 * u * u * t * c1[i] + 3.0 * u * t * t * c2[i] + t * t * t * b[i];
    }
    p
}

fn quantize_f32(p: [f64; 3]) -> [f64; 3] {
    [
        (p[0] as f32) as f64,
        (p[1] as f32) as f64,
        (p[2] as f32) as f64,
    ]
}

pub(super) fn from_bits3(value: Value) -> [f64; 3] {
    let bits: [u64; 3] = serde_json::from_value(value).unwrap();
    bits.map(f64::from_bits)
}

pub(super) fn point3(p: [f64; 3]) -> re::RoadEditingPoint3 {
    re::RoadEditingPoint3::try_new(p[0], p[1], p[2]).unwrap()
}

/// 折线 → 逐弦退化 cubic（`quality::fit` 仅取前四点；两端点线性插值即直线弦）。
pub(super) fn polyline_as_controls(points: &[[f64; 3]]) -> Vec<[[f64; 3]; 6]> {
    points
        .windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let lerp = |t: f64| {
                [
                    a[0] + t * (b[0] - a[0]),
                    a[1] + t * (b[1] - a[1]),
                    a[2] + t * (b[2] - a[2]),
                ]
            };
            [a, lerp(1.0 / 3.0), lerp(2.0 / 3.0), b, [0.0; 3], [0.0; 3]]
        })
        .collect()
}

/// P2-1 修复（Codex 评审）：**全 span 参考重建**——独立于发射成败（reshape 观察模式
/// 全链构建，starts-at-source / ends-at-source / chain-continuity 三断言钉覆盖完整）。
/// `replay_span_controls` 为逐 span 控制点载体链（诊断 span = 原失败 span 必然被捕获），
/// 但**遇发射失败即截断**（`--30266_0` 失败于 span 0/6 只捕获 span 0），不得用作全边参考。
fn full_reference_controls(input: &observe::EdgeInput) -> Vec<[[f64; 3]; 6]> {
    // 段容量不设约束（全 span 参考重建语义；`MAX_CURVES`=6 与源段数上界均为其他
    // 口径——修复构造的 cubic 数可超源段数，捕获断言随容量走）。参考链完整性由
    // reference_with_curve_cap 内三断言钉版。
    sampler::reshape::reference_with_curve_cap(input, usize::MAX)
}

/// B 口径基准（原参考的发射折线）：全 span 六点控制取前四点（cubic），513 节点均匀
/// 参数网格采样（grid 族基网格）、f32 规范化、跨 span 去重链接。
fn reference_emitted_polyline(controls: &[[[f64; 3]; 6]]) -> Vec<[f64; 3]> {
    let mut points: Vec<[f64; 3]> = Vec::new();
    for c in controls {
        for k in 0..=512 {
            let p = quantize_f32(cubic_bezier(c[0], c[1], c[2], c[3], k as f64 / 512.0));
            if points.last() != Some(&p) {
                points.push(p);
            }
        }
    }
    points
}

#[test]
fn witness_inventory_stays_frozen() {
    let inventory = inventory();
    assert_eq!(
        inventory["schema"],
        "laneflow-1154-witness-guided-search-witness-inventory"
    );
    assert_eq!(inventory["issue"], "#1154");
    let summary = &inventory["summary"];
    assert_eq!(summary["total"].as_u64(), Some(47));
    assert_eq!(summary["true"].as_u64(), Some(1));
    assert_eq!(summary["false"].as_u64(), Some(7));
    assert_eq!(summary["null"].as_u64(), Some(39));
    // 分路三类计数（全集 1/7/39 与 D 路子集 1/2/12 并列，不得归并）。
    let expected = [
        ("round1-1121", 4, 0, 0, 4),
        ("cohort2-1134", 13, 0, 2, 11),
        ("signal-diag-1-1138", 15, 0, 3, 12),
        ("signal-diag-2-1139", 15, 1, 2, 12),
    ];
    for (lane, total, t, f, n) in expected {
        let row = &summary["by_lane"][lane];
        assert_eq!(row["total"].as_u64(), Some(total), "{lane} total");
        assert_eq!(row["true"].as_u64(), Some(t), "{lane} true");
        assert_eq!(row["false"].as_u64(), Some(f), "{lane} false");
        assert_eq!(row["null"].as_u64(), Some(n), "{lane} null");
    }
    let rows = witness_rows(&inventory);
    assert_eq!(rows.len(), 47);
    // 清单序 = 文件序；分路块序冻结（第1轮 → cohort2 → C → D）。
    let lane_order = rows
        .iter()
        .map(|row| row["lane"].as_str().unwrap())
        .collect::<Vec<_>>();
    let mut seen = Vec::new();
    for lane in &lane_order {
        if seen.last() != Some(lane) {
            seen.push(lane);
        }
    }
    assert_eq!(
        seen,
        [
            "round1-1121",
            "cohort2-1134",
            "signal-diag-1-1138",
            "signal-diag-2-1139"
        ],
        "lane block order frozen"
    );
    // D 路 15 项边序逐值冻结（1/2/12 起点）。
    let d_rows = rows
        .iter()
        .filter(|row| row["lane"] == "signal-diag-2-1139")
        .collect::<Vec<_>>();
    assert_eq!(
        d_rows
            .iter()
            .map(|row| (
                row["edge"].as_str().unwrap(),
                row["problem_kind"].as_str().unwrap()
            ))
            .collect::<Vec<_>>(),
        [
            ("sumo:--30358#12_1", "last_chord"),
            ("sumo::-5126_3_0", "last_chord"),
            ("sumo::-5126_8_0", "first_chord"),
            ("sumo:--30316_0", "first_chord"),
            ("sumo::-1542_0_0", "first_chord"),
            ("sumo::-1542_10_0", "first_chord"),
            ("sumo::-1542_7_0", "first_chord"),
            ("sumo::-1542_8_0", "first_chord"),
            ("sumo::-25286_11_0", "first_chord"),
            ("sumo::-25286_2_0", "first_chord"),
            ("sumo::-25286_3_0", "last_chord"),
            ("sumo::-25286_7_0", "first_chord"),
            ("sumo::-25286_8_0", "last_chord"),
            ("sumo:--30266_0", "first_chord"),
            ("sumo::-17348_0_0", "first_chord"),
        ],
        "D lane frozen witness order"
    );
    assert_eq!(
        d_rows
            .iter()
            .filter(|row| row["in_selected_path_class"] == "true")
            .map(|row| row["edge"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["sumo:--30266_0"]
    );
    assert_eq!(
        d_rows
            .iter()
            .filter(|row| row["in_selected_path_class"] == "false")
            .map(|row| row["edge"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["sumo::-1542_7_0", "sumo::-17348_0_0"]
    );
    // 绑定完好性与 null 原因。
    let mut owners = BTreeSet::new();
    for row in rows {
        assert!(
            owners.insert(format!(
                "{}|{}|{}",
                row["lane"].as_str().unwrap(),
                row["edge"].as_str().unwrap(),
                row["problem_kind"].as_str().unwrap()
            )),
            "unique witness key"
        );
        let _control: [[u64; 3]; 6] =
            serde_json::from_value(row["control_f64_bits"].clone()).unwrap();
        let params: [u64; 2] = serde_json::from_value(row["witness_parameters_f64_bits"].clone())
            .expect("two witness parameter words");
        let t_bits = row["t_f64_bits"].as_u64().unwrap();
        let kind = row["problem_kind"].as_str().unwrap();
        assert_eq!(
            t_bits,
            if kind == "first_chord" {
                params[1]
            } else {
                params[0]
            },
            "t bit binding follows problem kind"
        );
        let t = f64::from_bits(t_bits);
        assert!((0.0..=1.0).contains(&t) && t.is_finite(), "t in domain");
        assert_eq!(
            row["source_sha256"].as_str().unwrap().len(),
            64,
            "source hash bound"
        );
        if row["in_selected_path_class"] == "null" {
            assert!(
                row["null_reason"].as_str().is_some(),
                "null rows carry reasons"
            );
        }
    }
}

#[test]
fn injection_variants_follow_frozen_order_and_domains() {
    // 形态序：首弦 → 末弦；各至多一次；域外形态不发起调用（清单 t 域断言）。
    let first = injection_variants(0.25);
    assert_eq!(
        first.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        ["first_chord_form", "last_chord_form"]
    );
    assert!(first.iter().all(|(_, kind)| kind.is_some()));
    // t=1.0：首弦合法（整 cubic 单弦），末弦域外 → not_executed。
    let edge_t = injection_variants(1.0);
    assert!(edge_t[0].1.is_some());
    assert!(edge_t[1].1.is_none());
    // t=0.0：末弦合法（整 cubic 单弦），首弦域外。
    let zero_t = injection_variants(0.0);
    assert!(zero_t[0].1.is_none());
    assert!(zero_t[1].1.is_some());
    // 调用上限 = 清单条数 × 2。
    assert_eq!(
        inventory()["summary"]["total"].as_u64().unwrap() as usize * INJECTION_CALLS_PER_WITNESS,
        94
    );
}

#[test]
fn edge_budget_handle_shares_per_edge_without_refill() {
    let budget = EdgeBudget::new();
    // E 交付的每边缺省：划分 32,768 / 网格工作 32,000,000 / 来源拟合 32,000,000。
    assert_eq!(budget.handle.partitions_remaining, 32_768);
    assert_eq!(budget.grid_remaining(), 32_000_000);
    assert_eq!(budget.handle.source_fit_remaining, 32_000_000);
    assert_eq!(budget.handle.grid_work_consumed, 0);
    assert!(!budget.exhausted());
    // 句柄不变量（consumed + remaining == 初始值）由 E 单测钉版；本路共享语义 =
    // 每边一个句柄传全部调用（run_injection/run_standard_grid 同句柄），池尽即
    // not_executed（先到即停、不按见证续杯）。
    let mut spent = EdgeBudget::new();
    spent.handle.grid_work_remaining = 0;
    spent.handle.grid_work_consumed = 32_000_000;
    assert!(spent.exhausted());
    // 消费序词汇冻结（ABBA 轴一）。
    assert_eq!(CONSUMPTION_ORDERS, ["injection-first", "grid-first"]);
}
