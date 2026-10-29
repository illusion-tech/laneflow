//! #1154：见证引导整边搜索族——候选资格验证。对一次冻结的见证清单（47 项 =
//! 1 true / 7 false / 39 null；D 路 15 项 = 1/2/12，两组数字并列不得归并）执行
//! 注入变体整边搜索（首弦 `[0,t]+>t 节点`、移除 ≤t 旧节点；末弦对称；不强制见证弦），
//! 以现行接受口径全量验证候选资格。执行口径冻结于
//! `research/issue-1154-witness-guided-search/plan.md`；预算三计数器/共享作用域/
//! 遍历序按 Issue #1154 冻结草案 v2 与统筹裁决执行（池消费序 = 注入变体先行 →
//! 标准网格对照，注入先行为正式语义）。不改生产采样、compiler、阈值与原成功几何；
//! 见证变体候选不回写标准族池；不预期恢复 C24 或任何分量（plan §5）。

use super::coverage::{edge_input, plan_stop_label, replay_span_controls};
use super::*;
use sampler::grid::{Injection, InjectionKind, WorkBudget};
use std::collections::BTreeSet;

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

fn normalized(value: &Value) -> Value {
    serde_json::from_str(&value.to_string()).unwrap()
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
    // (a) 候选资格验证：搜索内接受（实际长度/sink 转角/0.01 m）+ 编译/LFCA/锚点
    // 全量验证；双向拟合为复核域独立计费（lead 裁决），E task-11 fit 入口落地前
    // 该列记 not_executed（未执行分列，不混入几何拒绝统计）。
    let mut qualification_rows = Vec::new();
    let mut witness_pool: combinations::Pools = BTreeMap::new();
    let review_fit_consumed_total = 0_u64;
    let mut qi = 0_usize;
    for (edge, cands) in &witness_candidates {
        let component = components
            .iter()
            .find(|c| c.missing_edges.iter().any(|m| m == edge))
            .expect("witness edge belongs to a frozen component");
        let part = combinations::part(traffic, component);
        let path = &part.maneuver_paths[0];
        for cand in cands {
            let mut programs = originals.clone();
            programs.insert(edge.clone(), cand.program.clone());
            let file = format!("qualify-{qi:03}-{edge}.lfca");
            qi += 1;
            let result = compile_part(traffic, path, &part, &programs, Some(&out.join(&file)));
            let outcome = match result["status"].as_str().unwrap() {
                "accepted" => "qualified",
                "target_check_not_completed" => "not_executed",
                _ => "geometry_rejected",
            };
            // 池入规则：几何拒绝排除；qualified / not_executed（依赖不齐、完整验证
            // 随 (c) 联合编译执行）入本轮新增池，不回写标准族池。
            if outcome != "geometry_rejected" {
                witness_pool
                    .entry(edge.clone())
                    .or_default()
                    .push(cand.program.clone());
            }
            qualification_rows.push(json!({"edge":edge,"variant":cand.variant,
                "arc_m":cand.arc_m,"program":program_bits(&cand.program),
                "in_search_acceptance":"actual_length/sink_angle/0.01m_envelope",
                "source_fit_review_domain":{"status":"not_executed",
                    "reason":"review_domain_fit_entry_pending_e_task11",
                    "billing":"review_domain_independent_per_lead_ruling"},
                "compile_validation":result,"outcome":outcome}));
        }
    }
    recovery::write_json(
        &out,
        "qualification.json",
        &json!({"rows":qualification_rows,
        "review_domain_counts":{"fit_evaluations":review_fit_consumed_total,
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
                let prior = &comparison["components"][index];
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
            if variant == "witness" {
                evaluations.insert(component.id.clone(), result.clone());
            }
            let affected = component
                .missing_edges
                .iter()
                .any(|m| witness_candidates.contains_key(m));
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
                .any(|m| witness_candidates.contains_key(m))
        })
        .flat_map(|c| c.paths.iter().cloned())
        .collect::<BTreeSet<_>>();
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
                    "note":"见证涉及关联路径口径（含分量完整路径）；两口径分列、新增收益不并入基线"}},
            "variants":results,
            "stop_line_note":"D 冻结 26 行三情形口径复用；见证涉及停止线边本轮结果归情形；依赖不齐记未执行",
            "all_original_geometry_and_controls_equal":true,"production_g1_accepted":false}),
    );
    timing["validation_order"] = json!(validation_order);
    timing["validation_seconds"] = json!(validation_times);
    recovery::write_json(&out, "timing.json", &timing);
    println!("witness guided search evidence saved={}", out.display());
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
