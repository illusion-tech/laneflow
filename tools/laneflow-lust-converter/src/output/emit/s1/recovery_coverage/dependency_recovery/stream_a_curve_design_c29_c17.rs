//! #1131 流 A：C29+C17 失败边的曲线/分段设计变量有限探索与完整组合验收。
//! 执行口径冻结于 `research/issue-1131-curve-design-c29-c17/plan.md`，方法事实来源为
//! `research/issue-1115-failed-curve-design/design.md`；本模块不改生产采样、compiler
//! 配置档、原成功几何、拓扑与控制锚点。计划与冻结实现不一致时停止该处执行并重新授权，
//! 不得单向迁就（plan §7）。三条未决边的定位是有限设计变量探索：零恢复、族耗尽、
//! 预算停止分别保存，均不写可行或不可行结论（plan §6）。

use super::*;
use experiment::{merge, pool};

/// plan §1/§6 冻结样本与段容量表：顺序即搜索顺序，`s` 为原修复参考链实际 cubic 数。
const EDGES: [(&str, usize); 4] = [
    ("sumo:--30452_0", 3),
    ("sumo::-20112_0_0", 4),
    ("sumo::-10564_3_0", 2),
    ("sumo::-10564_6_0", 3),
];

fn frozen_inputs() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../../tests/fixtures/issue-1107-inputs.json"
    ))
    .unwrap()
}

pub(super) fn pool_counts(pools: &combinations::Pools) -> (usize, usize) {
    (pools.len(), pools.values().map(Vec::len).sum())
}

pub(super) fn per_edge_counts(pools: &combinations::Pools) -> BTreeMap<String, usize> {
    pools
        .iter()
        .map(|(id, candidates)| (id.clone(), candidates.len()))
        .collect()
}

/// plan §5 冻结基线候选池：四来源固定顺序①→④合并，去重键为规范 f32 点位；
/// 断言 9 边 75 份（边集与每边份数）。任一来源缺失或逐值不符即停止执行并重新授权。
fn baseline_pool() -> combinations::Pools {
    let legacy: Value = serde_json::from_str(include_str!(
        "../../../../../../tests/fixtures/issue-1103-s1-recovery-budget-results.json"
    ))
    .unwrap();
    let old: Value = serde_json::from_str(include_str!(
        "../../../../../../tests/fixtures/issue-1107-old-searches.json"
    ))
    .unwrap();
    let grid: Value = serde_json::from_str(include_str!(
        "../../../../../../tests/fixtures/issue-1111-positive-calibration.json"
    ))
    .unwrap();
    let curve: Value = serde_json::from_str(include_str!(
        "../../../../../../tests/fixtures/issue-1115-searches.json"
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
            ("sumo::-7348_3_0".to_owned(), 16),
        ]),
        "frozen baseline per-edge candidate counts"
    );
    pools
}

/// 每边按冻结段容量 s 执行 design.md 口径的有限几何设计变量候选搜索。
fn run_searches(inputs: &Value) -> (Value, Value) {
    let mut searches = BTreeMap::new();
    let mut timing = BTreeMap::new();
    for (id, max_curves) in EDGES {
        let input = measurement::input(&inputs[id]);
        let started = Instant::now();
        let search = sampler::reshape::search_with_curve_cap(&input, max_curves);
        timing.insert(id, started.elapsed().as_secs_f64());
        // plan §6：段容量按 s 冻结；参考链实际 cubic 数与 96s+1 点上限即断言口径。
        assert_eq!(
            search.report["reference_controls_f64_bits"]
                .as_array()
                .unwrap()
                .len(),
            max_curves,
            "frozen reference cubic count"
        );
        assert_eq!(
            search.report["policy"]["reference_cubics_max"],
            json!(max_curves)
        );
        searches.insert(
            id,
            json!({"search":search.report,"candidates":recovery::candidates(&search)}),
        );
    }
    (json!(searches), json!(timing))
}

#[test]
#[ignore = "requires fresh CURVE_DESIGN_OUTPUT_DIR"]
fn fixed_stream_a_curve_design() {
    let out = PathBuf::from(std::env::var("CURVE_DESIGN_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh curve design output");
    fs::create_dir_all(&out).unwrap();
    let (searches, timing) = run_searches(&frozen_inputs());
    recovery::write_json(&out, "stream-a-searches.json", &searches);
    recovery::write_json(&out, "stream-a-timing.json", &timing);
}

#[test]
fn stream_a_baseline_pool_merges_four_frozen_sources_in_order() {
    let pools = baseline_pool();
    assert_eq!(pool_counts(&pools), (9, 75));
}

#[test]
fn curve_capacity_parameterization_keeps_the_frozen_six_default() {
    assert_eq!(sampler::reshape::policy()["reference_cubics_max"], 6);
    for (id, max_curves) in EDGES {
        assert_eq!(
            sampler::reshape::policy_with_curve_cap(max_curves)["reference_cubics_max"],
            json!(max_curves),
            "{id} curve capacity"
        );
    }
}
