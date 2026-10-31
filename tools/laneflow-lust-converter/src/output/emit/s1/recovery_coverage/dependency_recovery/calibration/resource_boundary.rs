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

/// 完整恢复流程资源证据（四域；独占测量时段跑 release 两轮——方案 §3b）。
#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, DIRECT_SUCCESSOR_BASELINE_DIR and an exclusive measurement window"]
fn four_domain_resource_evidence() {
    todo!("四域证据入口实现（测量窗口就绪后执行）")
}
