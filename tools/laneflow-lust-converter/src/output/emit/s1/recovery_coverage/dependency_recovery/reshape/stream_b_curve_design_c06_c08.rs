//! 流 B（#1130）：C06+C08 失败边曲线/分段设计变量研究入口。
//!
//! 方法口径逐值沿用 `research/issue-1115-failed-curve-design/design.md` 与
//! `crate::output::geom::s1::reshape`（每边 148 个预定义几何配置、回折拒绝、
//! 双向来源拟合筛选、n∈2..96 双划分族、全局保留 8），不重定义口径；样本、
//! 段容量与流 B 特有约束见冻结计划 `research/issue-1130-curve-design-c06-c08/plan.md`。
//! 模块名带流名，避免与流 A（#1131）入口冲突。

use super::*;

/// 流 B 四条失败边（冻结计划 §6）：`(边, 原参考链 cubic 数 s, 临时程序点上限 96s+1)`。
const STREAM_B_EDGES: [(&str, usize, usize); 4] = [
    ("sumo::-10004_3_0", 3, 289),
    ("sumo::-10004_8_0", 4, 385),
    ("sumo:--30306#2_0", 9, 865),
    ("sumo::-8806_4_0", 5, 481),
];

const C06_FILLET_KNOTS: [usize; 2] = [1, 2];

/// 基线候选池（冻结计划 §5）四来源合并后的边集与每边份数（规范 f32 点位去重）。
const BASELINE_POOL_COUNTS: [(&str, usize); 9] = [
    ("sumo::-10004_5_1", 16),
    ("sumo::-10034_0_0", 8),
    ("sumo::-10130_16_0", 8),
    ("sumo::-10130_18_0", 8),
    ("sumo::-10130_8_0", 8),
    ("sumo::-16502_0_0", 1),
    ("sumo::-28860_3_0", 2),
    ("sumo::-29388_3_0", 8),
    ("sumo::-7348_3_0", 16),
];

/// 合并组跨过的原参考链内部结点（0 基：结点 k 位于 cubic k-1 与 k 之间）。
/// 组覆盖 cubic 区间 `[start, end)` 时，组内结点 `start+1..end` 被合段跨过；
/// 合并组的段数只减不增（冻结计划 §6）。
fn crossed_internal_knots(group_size: usize, cubics: usize) -> Vec<usize> {
    assert!(group_size >= 1 && cubics >= 1);
    let mut knots = Vec::new();
    for start in (0..cubics).step_by(group_size) {
        let end = (start + group_size).min(cubics);
        knots.extend((start + 1)..end);
    }
    knots
}

fn merged_group_size(group: &str, cubics: usize) -> usize {
    match group {
        "reference" | "keep" => 1,
        "pair" => 2,
        "whole" => cubics,
        other => panic!("unknown geometry group: {other}"),
    }
}

/// 逐候选/逐几何「跨倒圆切点」记录（冻结计划 §2 流 B 特有①）。该标记只记录几何
/// 事实（合段是否跨过原内部倒圆切点），不表示跨倒圆切点的合段优于纯采样优化。
fn fillet_crossing_record(group: &str, cubics: usize) -> Value {
    let group_size = merged_group_size(group, cubics);
    let crossed = crossed_internal_knots(group_size, cubics);
    let crossed_fillet = C06_FILLET_KNOTS
        .iter()
        .copied()
        .filter(|k| crossed.contains(k))
        .collect::<Vec<_>>();
    json!({"group":group,"merged_group_size":group_size,
        "crossed_internal_knots":crossed,
        "crossed_fillet_tangent_knots":crossed_fillet,
        "crosses_fillet_tangent_point":!crossed_fillet.is_empty()})
}

fn pool_size(pools: &combinations::Pools) -> (usize, usize) {
    (pools.len(), pools.values().map(Vec::len).sum())
}

/// 基线池冻结计数断言（冻结计划 §5：边集与每边份数）。
fn assert_baseline_pool_counts(pools: &combinations::Pools) {
    assert_eq!(pool_size(pools), (9, 75), "frozen baseline pool size");
    let actual = pools
        .iter()
        .map(|(id, rows)| (id.as_str(), rows.len()))
        .collect::<Vec<_>>();
    assert_eq!(actual, BASELINE_POOL_COUNTS, "frozen baseline pool counts");
}

#[test]
fn frozen_segment_capacity_matches_plan() {
    assert_eq!(
        STREAM_B_EDGES.map(|(_, s, cap)| (s, cap)),
        [(3, 289), (4, 385), (9, 865), (5, 481)]
    );
    for (.., s, cap) in STREAM_B_EDGES {
        assert_eq!(96 * s + 1, cap);
    }
}

#[test]
fn merged_segments_record_fillet_tangent_crossing() {
    // C06 s=3：倒圆切点为结点 1/2；合段跨过其一即为 true。
    assert_eq!(crossed_internal_knots(1, 3), Vec::<usize>::new());
    assert_eq!(crossed_internal_knots(2, 3), vec![1]);
    assert_eq!(crossed_internal_knots(3, 3), vec![1, 2]);
    assert!(
        !fillet_crossing_record("keep", 3)["crosses_fillet_tangent_point"]
            .as_bool()
            .unwrap()
    );
    assert!(
        fillet_crossing_record("pair", 3)["crosses_fillet_tangent_point"]
            .as_bool()
            .unwrap()
    );
    assert!(
        fillet_crossing_record("whole", 3)["crosses_fillet_tangent_point"]
            .as_bool()
            .unwrap()
    );
    // C06 s=4：结点 3 是内部结点但不是冻结的倒圆切点，不计入标记。
    let pair = fillet_crossing_record("pair", 4);
    assert_eq!(pair["crossed_internal_knots"], json!([1, 3]));
    assert_eq!(pair["crossed_fillet_tangent_knots"], json!([1]));
    assert!(pair["crosses_fillet_tangent_point"].as_bool().unwrap());
    let whole = fillet_crossing_record("whole", 4);
    assert_eq!(whole["crossed_fillet_tangent_knots"], json!([1, 2]));
    assert_eq!(whole["merged_group_size"], json!(4));
}

#[test]
fn baseline_pool_merges_sources_in_frozen_order() {
    let legacy: Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/issue-1103-s1-recovery-budget-results.json"
    ))
    .unwrap();
    let old: Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/issue-1107-old-searches.json"
    ))
    .unwrap();
    let positive: Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/issue-1111-positive-calibration.json"
    ))
    .unwrap();
    let grid: Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/issue-1111-grid-searches.json"
    ))
    .unwrap();
    let curve: Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/issue-1115-searches.json"
    ))
    .unwrap();
    for (id, row) in positive["edges"].as_object().unwrap() {
        assert_eq!(row["candidates"], grid[id]["candidates"]);
    }
    assert_eq!(positive["edges"].as_object().unwrap().len(), 2);
    let mut baseline = pool(&legacy["tiers"]["32768"]["searches"]);
    assert_eq!(pool_size(&baseline), (6, 48));
    merge(&mut baseline, pool(&old));
    assert_eq!(pool_size(&baseline), (7, 56));
    merge(&mut baseline, pool(&positive["edges"]));
    assert_eq!(pool_size(&baseline), (7, 72));
    merge(&mut baseline, pool(&curve));
    assert_baseline_pool_counts(&baseline);
}

#[test]
fn default_curve_cap_keeps_the_1115_policy() {
    assert_eq!(
        sampler::reshape::policy()["reference_cubics_max"],
        json!(6),
        "default policy unchanged"
    );
    assert_eq!(
        sampler::reshape::policy_with_curve_cap(9)["reference_cubics_max"],
        json!(9)
    );
}
