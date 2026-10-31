//! task #1193（#790 资源边界补证）：预算公式验证入口与完整恢复流程资源证据。
//!
//! 公式（维护者 2026-10-31 口径 + 本路三重验证）：fit 复核域实耗
//! `= 8,448 · W · C`——W = 来源段数（`input.points.len() − 1`）、C = 被评
//! 三次条数（`polyline_as_controls` 形态下 = 完整参考链弦数 `C_ref = 512×span`）。
//! 8,448 = 反向 `2·STEPS²` + 正向 `4·STEPS`（`STEPS = 64`）。记法统一
//! W / C_ref（双义根源：「S」在表行标签为参考 span 数、在公式为来源段数）。

use super::*;

/// fit 计数闭式（反向/正向分列；`distance()` 每点到线段 1 单位、STEPS=64）。
fn fit_count_closed_form(source_windows: u64, cubics: u64) -> (u64, u64, u64) {
    let reverse = 2 * 64 * 64 * source_windows * cubics;
    let forward = 4 * 64 * source_windows * cubics;
    (reverse, forward, reverse + forward)
}

/// 合成三次条（单位间距直线，退化安全）。
fn synthetic_cubics(count: usize) -> Vec<[[f64; 3]; 6]> {
    (0..count)
        .map(|i| {
            let x = i as f64;
            [
                [x, 0.0, 0.0],
                [x + 0.3, 0.0, 0.0],
                [x + 0.7, 0.0, 0.0],
                [x + 1.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
            ]
        })
        .collect()
}

/// 合成来源折线（`windows = W` ⇒ W+1 点；含折保证非零窗）。
fn synthetic_source(windows: usize) -> Vec<[f64; 3]> {
    (0..=windows)
        .map(|i| {
            let x = i as f64 * 0.9;
            [x, if i % 2 == 0 { 0.0 } else { 0.15 }, 0.0]
        })
        .collect()
}

/// 闭式 vs 机器计数器恒等式（多组 (W, C) 精确相等）。
#[test]
fn budget_formula_matches_machine_counters() {
    for (windows, cubics) in [
        (2_usize, 8_usize),
        (3, 16),
        (4, 32),
        (5, 24),
        (2, 512),
        (4, 256),
    ] {
        let controls = synthetic_cubics(cubics);
        let source = synthetic_source(windows);
        let mut handle = crate::output::geom::s1::grid::WorkBudget::per_edge_default();
        handle.source_fit_remaining = u64::MAX;
        sampler::reshape::quality::fit_with_handle(&controls, &source, &mut handle)
            .expect("bounded synthetic fit");
        let (reverse, forward, total) = fit_count_closed_form(windows as u64, cubics as u64);
        assert_eq!(
            handle.source_fit_consumed, total,
            "identity 8,448·W·C for W={windows} C={cubics}（反向 {reverse} + 正向 {forward}）"
        );
    }
}

/// #1174 成本表 7 行逐行复算（闭式 = 入库值），并断言双义差异记录。
#[test]
fn cohort_cost_table_recomputed() {
    // （参考 span 数 s、弦数 C_ref = 512·s、入库实耗）——s=4 组 4 行、s=5 组 3 行。
    let rows: [(&str, u64, u64, u64); 7] = [
        ("sumo::-10034_4_0", 4, 2_048, 69_206_016),
        ("sumo::-10046_0_0", 4, 2_048, 69_206_016),
        ("sumo::-1426_1_0", 4, 2_048, 69_206_016),
        ("sumo::-25490_4_0", 4, 2_048, 69_206_016),
        ("sumo::-1426_2_0", 5, 2_560, 86_507_520),
        ("sumo::-19726_2_0", 5, 2_560, 86_507_520),
        ("sumo::-22042_7_0", 5, 2_560, 86_507_520),
    ];
    let inputs: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/inputs.json"
    ))
    .unwrap();
    for (id, spans, chords, expected) in rows {
        assert_eq!(chords, 512 * spans, "C_ref = 512×参考 span 数 {id}");
        // W = 来源段数（冻结输入逐边核验：5 输入点 → W=4）。
        let windows = inputs[id]["points"].as_array().unwrap().len() - 1;
        assert_eq!(windows, 4, "cohort 来源段数 W=4（{id}）");
        let (_, _, total) = fit_count_closed_form(windows as u64, chords);
        assert_eq!(total, expected, "闭式复算 = 入库实耗（{id}）");
    }
    // 差异记录（归因：lead 派工消息算例笔误，非维护者计算）：把参考 span 数 5
    // 误代公式 S（来源段数）得 108,134,400 ≠ 表值 86,507,520；S 应代 W=4。
    let wrong = 8_448u64 * 5 * 2_560;
    assert_eq!(wrong, 108_134_400, "误代算例值（差异记录）");
    assert_ne!(wrong, 86_507_520, "误代值不等于表值");
    let (_, _, right) = fit_count_closed_form(4, 2_560);
    assert_eq!(right, 86_507_520, "正确代入 W=4 等于表值");
    // 33,792/弦 = 8,448×W 在 W=4 成立；W=5 形状每弦 42,240（通用预留公式注记）。
    assert_eq!(8_448 * 4, 33_792);
    assert_eq!(8_448 * 5, 42_240);
}

/// 三次 Bezier 求值（Bernstein 形态，本地实现——仅计数证据用）。
fn eval_cubic(p: [f64; 3], c1: [f64; 3], c2: [f64; 3], q: [f64; 3], t: f64) -> [f64; 3] {
    let u = 1.0 - t;
    std::array::from_fn(|i| {
        u * u * u * p[i] + 3.0 * u * u * t * c1[i] + 3.0 * u * t * t * c2[i] + t * t * t * q[i]
    })
}

/// f32 规范化回程（发射折线形态）。
fn quantize_f32(p: [f64; 3]) -> [f64; 3] {
    p.map(|v| f64::from(v as f32))
}

/// 发射折线（逐 span cubic 513 均匀参数节点、f32 规范化串接）。
fn emitted_polyline(controls: &[[[f64; 3]; 6]]) -> Vec<[f64; 3]> {
    let mut points: Vec<[f64; 3]> = Vec::new();
    for c in controls {
        for k in 0..=512 {
            let p = quantize_f32(eval_cubic(c[0], c[1], c[2], c[3], k as f64 / 512.0));
            if points.last() != Some(&p) {
                points.push(p);
            }
        }
    }
    points
}

/// 折线 → 三次条（每弦一退化 cubic，弦向为切向）。
fn polyline_as_controls(points: &[[f64; 3]]) -> Vec<[[f64; 3]; 6]> {
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
            let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            let dir = if n > 0.0 {
                [d[0] / n, d[1] / n, d[2] / n]
            } else {
                [1.0, 0.0, 0.0]
            };
            [a, lerp(1.0 / 3.0), lerp(2.0 / 3.0), b, dir, dir]
        })
        .collect()
}

/// 维护者修订 2：准入配额边界三档（充足 / 恰好够用 / 少一单位）+ 全转换聚合
/// 耗尽用例；断言未完成复核的候选不入池（`review_budget_stopped` 语义实测）。
///
/// 全转换聚合的选择范围与依据：**每转换 12 条失败边 = 冻结样本口径**（#1105/
/// #1107 缺口清单：全网 84 目标 / 53 分量 / 183 关联路径中的 12 条失败边），
/// 为冻结研究队列、**非生产比例投影**；全转换预算以此为聚合建模基准。
#[test]
fn review_quota_boundary_tiers() {
    let windows = 2_usize;
    let chords = 8_usize;
    let per_candidate = fit_count_closed_form(windows as u64, chords as u64).2;
    let candidates = 3_u64;
    let need = per_candidate * candidates;
    for (label, quota, expect_completed, expect_stopped) in [
        ("sufficient_quota", need + 1, 3_u64, false),
        ("exactly_enough_quota", need, 3, false),
        ("one_unit_short_quota", need - 1, 2, true),
    ] {
        let mut handle = crate::output::geom::s1::grid::WorkBudget::per_edge_default();
        handle.source_fit_remaining = quota;
        let mut pool = Vec::new();
        let mut stopped = false;
        for candidate in 0..candidates {
            let controls = synthetic_cubics(chords);
            let source = synthetic_source(windows);
            match sampler::reshape::quality::fit_with_handle(&controls, &source, &mut handle) {
                Some(_) => pool.push(candidate),
                None => {
                    // review_budget_stopped：未完成复核的候选不入池。
                    stopped = true;
                    break;
                }
            }
        }
        assert_eq!(pool.len() as u64, expect_completed, "{label} 复核完成数");
        assert_eq!(stopped, expect_stopped, "{label} review_budget_stopped");
    }
    // 全转换聚合耗尽：两邻边共享聚合句柄，总额少一单位——末边末候选复核
    // 未完成不入池并记 review_budget_stopped。
    let per_edge_need = per_candidate * 2;
    let mut aggregate = crate::output::geom::s1::grid::WorkBudget::per_edge_default();
    aggregate.source_fit_remaining = per_edge_need * 2 - 1;
    let mut pooled = Vec::new();
    let mut stopped_at = None;
    'edges: for edge in 0..2_u64 {
        for candidate in 0..2_u64 {
            let controls = synthetic_cubics(chords);
            let source = synthetic_source(windows);
            match sampler::reshape::quality::fit_with_handle(&controls, &source, &mut aggregate) {
                Some(_) => pooled.push((edge, candidate)),
                None => {
                    stopped_at = Some((edge, candidate));
                    break 'edges;
                }
            }
        }
    }
    assert_eq!(pooled.len(), 3, "聚合少一单位仅完成三份复核");
    assert_eq!(stopped_at, Some((1, 1)), "聚合耗尽于末边末候选");
    assert_eq!(
        aggregate.source_fit_consumed,
        per_edge_need * 2 - 1,
        "实耗 = 3 份完整 + 失败尝试的部分工作（如实计账）= 总配额"
    );
}

/// 完整恢复流程资源证据（四域；独占测量时段跑 release 两轮——方案 §3b）。
///
/// ① 候选生成（有限完成/预算停止/既有候选三组样本）② 资格复核（参考基准 +
/// 候选拟合，`8,448·W·C` 恒等式实测）③ 赋值验证（C17 未进入/抽样分量）
/// ④ 编译/LFCA（每赋值试编译计数 + 选中终验）。结果 JSON 禁计时字段；耗时入
/// 独立计时文件；内存仅记**进程工作集采样峰值**（唯一内存指标，整管线口径
/// 注明；工作计数差不作内存增量解读）。
#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, DIRECT_SUCCESSOR_BASELINE_DIR, SIX_STREAMS_BASELINE_DIR and an exclusive measurement window"]
fn four_domain_resource_evidence() {
    let out = PathBuf::from(std::env::var("FOUR_DOMAIN_EVIDENCE_DIR").unwrap());
    assert!(!out.exists(), "fresh four-domain evidence output");
    fs::create_dir_all(&out).unwrap();
    let (observation, source, mut timing) = capture_lust();
    let originals = original_programs(&observation);
    let traffic = observation.traffic.as_ref().unwrap();
    let inputs: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/inputs.json"
    ))
    .unwrap();
    // `--30306#2_0`（预算停止样本）输入在 #1130 冻结集（边名键控）。
    let inputs_1130: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1130-curve-design-c06-c08/inputs.json"
    ))
    .unwrap();
    let sample_input = |id: &str| {
        let row = if id.contains("30306") {
            let key = inputs_1130
                .as_object()
                .unwrap()
                .keys()
                .find(|key| key.contains("30306"))
                .expect("30306 key");
            &inputs_1130[key]
        } else {
            &inputs[id]
        };
        assert!(!row.is_null(), "sample row present for {id}");
        measurement::input(row)
    };
    let mut domains = BTreeMap::<String, Value>::new();

    // ① 候选生成：有限完成（C17 edge6 旧族零候选）、预算停止（--30306#2_0）、
    // 既有候选（edge3）。
    let mut generation = BTreeMap::new();
    let mut generation_seconds = BTreeMap::new();
    for (id, s, label) in [
        (
            "sumo::-10564_6_0",
            3_usize,
            "finite_completion_zero_candidate",
        ),
        ("sumo::-30306#2_0", 3, "budget_stopped_sample"),
        ("sumo::-10564_3_0", 2, "candidate_found_sample"),
    ] {
        let input = sample_input(id);
        let mut handle = crate::output::geom::s1::grid::WorkBudget::per_edge_default();
        let started = Instant::now();
        let search =
            sampler::reshape::search_with_curve_cap_budget(&input, 96 * s + 1, &mut handle);
        generation_seconds.insert(id, started.elapsed().as_secs_f64());
        generation.insert(
            id,
            json!({"sample":label,
                "counters":{"partitions":handle.partitions_consumed,
                    "sampling_work":handle.grid_work_consumed,
                    "source_fit":handle.source_fit_consumed},
                "stop":{"budget_truncated":search.report["budget_truncated"],
                    "stop_reason":search.report["stop_reason"]},
                "successful_programs":search.candidates.len()}),
        );
    }
    domains.insert("1_candidate_generation".into(), json!(generation));
    timing["domain_seconds"]["1_candidate_generation"] = json!(generation_seconds);

    // ② 资格复核：参考基准（曲线拟合 + 发射折线拟合）+ 候选资格拟合；
    // `8,448·W·C` 恒等式实测列。
    let mut review = BTreeMap::new();
    let mut review_seconds = BTreeMap::new();
    for (id, s) in [("sumo::-10564_3_0", 2_usize), ("sumo::-10564_6_0", 3)] {
        let input = measurement::input(&inputs[id]);
        let reference = sampler::reshape::reference_with_curve_cap(&input, 96 * s + 1);
        let polyline = emitted_polyline(&reference);
        let poly_controls = polyline_as_controls(&polyline);
        let windows = input.points.len() - 1;
        let mut handle = crate::output::geom::s1::grid::WorkBudget::per_edge_default();
        handle.source_fit_remaining = u64::MAX;
        let started = Instant::now();
        sampler::reshape::quality::fit_with_handle(&reference, &input.points, &mut handle)
            .expect("bounded reference curve fit");
        let curve_consumed = handle.source_fit_consumed;
        sampler::reshape::quality::fit_with_handle(&poly_controls, &input.points, &mut handle)
            .expect("bounded reference polyline fit");
        let polyline_consumed = handle.source_fit_consumed - curve_consumed;
        let mut candidate_rows = Vec::new();
        if domains["1_candidate_generation"][id]["successful_programs"]
            .as_u64()
            .unwrap()
            > 0
        {
            let search = sampler::reshape::search_with_curve_cap(&input, s);
            for candidate in &search.candidates {
                let points = program_points(&candidate.repaired.program);
                let controls = polyline_as_controls(&points);
                sampler::reshape::quality::fit_with_handle(&controls, &input.points, &mut handle);
                candidate_rows.push(json!({"chords":points.len() - 1}));
            }
        }
        review_seconds.insert(id, started.elapsed().as_secs_f64());
        let expected_polyline = 8_448u64 * windows as u64 * poly_controls.len() as u64;
        review.insert(
            id,
            json!({"source_windows":windows,
                "reference_spans":reference.len(),
                "reference_chords":poly_controls.len(),
                "curve_baseline_consumed":curve_consumed,
                "polyline_baseline_consumed":polyline_consumed,
                "polyline_identity_expected":expected_polyline,
                "polyline_identity_holds":polyline_consumed == expected_polyline,
                "candidate_fits":candidate_rows,
                "total_consumed":handle.source_fit_consumed}),
        );
    }
    domains.insert("2_qualification_review".into(), json!(review));
    timing["domain_seconds"]["2_qualification_review"] = json!(review_seconds);

    // ③ 赋值验证 + ④ 编译/LFCA 计账合一（维护者修订 1：按实际阶段执行计数、
    // 保存返回状态、统一选中赋值与终验路径——evaluate 内部每赋值 1 次
    // `compile_part` 试编译、选中后 1 次带 save 的终验；缺程序检查早返零执行，
    // 不另行臆造编译）。耗时入计时文件（修订 3：结果 JSON 禁计时字段）。
    let pools = super::shared_baseline::shared_pools();
    let components = components::inventory(traffic, &originals, &target_ids(&reference()));
    let c17_id = "sumo:-10564:--32172#4_0-to--31194_0::-10564_3_0.:-10564_6_0";
    let mut assignment = BTreeMap::new();
    let mut compile_accounting = BTreeMap::new();
    let mut assignment_seconds = BTreeMap::new();
    for id in std::iter::once(c17_id.to_owned()).chain(
        components
            .iter()
            .filter(|c| c.id != c17_id)
            .take(4)
            .map(|c| c.id.clone()),
    ) {
        let component = components.iter().find(|c| c.id == id).expect("component");
        let started = Instant::now();
        let result = combinations::evaluate(traffic, component, &originals, &pools, None);
        assignment_seconds.insert(id.clone(), started.elapsed().as_secs_f64());
        let total = result["total_assignments"].as_u64();
        let trials = result["trials"].as_array().cloned().unwrap_or_default();
        let trial_compiles = trials.len() as u64;
        let accepted_trials = trials
            .iter()
            .filter(|trial| trial["result"]["status"] == "accepted")
            .count() as u64;
        // 选中终验：evaluate 内部对选中赋值再执行 1 次 `compile_part`（带 save
        // 路径），返回状态保存于 `selected_result`；缺程序早返时两者皆为零/空。
        let final_validation_compiles = u64::from(result["selected_result"].is_object());
        assignment.insert(
            id.clone(),
            json!({"status":result["status"],
                "total_assignments":total,
                "trial_compiles":trial_compiles,
                "unvisited_assignments":total.map(|total| total - trials.len() as u64),
                "assignment_budget_stopped":result["assignment_budget_stopped"]}),
        );
        compile_accounting.insert(
            id.clone(),
            json!({"status":result["status"],
                "trial_compile_part_calls":trial_compiles,
                "trial_accepted":accepted_trials,
                "trial_rejected":trial_compiles - accepted_trials,
                "post_selection_compile_part_calls":final_validation_compiles,
                "post_emit_checks":accepted_trials + final_validation_compiles,
                "selected_assignment":result["selected_assignment"],
                "selected_result_status":result["selected_result"]
                    .get("status")
                    .cloned()
                    .unwrap_or(Value::Null),
                "not_executed":(trial_compiles == 0 && final_validation_compiles == 0)
                    .then_some("missing_real_programs_early_return")}),
        );
    }
    domains.insert("3_assignment_validation".into(), json!(assignment));
    domains.insert("4_compile_lfca".into(), json!(compile_accounting));
    timing["domain_seconds"]["3_assignment_validation"] = json!(assignment_seconds);

    // 预期调用频率（生产每转换口径）。
    domains.insert(
        "call_frequency_model".into(),
        json!({
            "per_conversion":{"successful_edges_compile":16265,
                "failed_edges_search":12,"review_per_candidate":1,
                "components":53,"assignment_trial_compiles_per_component":"<=256",
                "post_selection_final_validation_per_component":1},
            "note":"墙钟/内存为测量指标，不进停止判定"}),
    );
    recovery::write_json(
        &out,
        "four-domain-measurement.json",
        &json!({"schema":"laneflow-1193-four-domain-resource-evidence",
            "source":source,"domains":domains}),
    );
    recovery::write_json(&out, "four-domain-timing.json", &timing);
    let _ = &mut timing;
}
