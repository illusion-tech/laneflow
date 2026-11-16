//! #1134：第 2 批队列（11 个非信号分量、22 条缺边）的固定曲线覆盖补齐、逐边局部
//! 必要条件诊断与候选齐备分量组合验收。执行口径全部冻结于
//! `research/issue-1134-cohort2-coverage-diagnosis/plan.md`，方法事实来源
//! `research/issue-1121-coverage-diagnosis/plan.md` 逐值沿用；本模块不改生产采样、
//! compiler 配置档、阈值与原成功几何。计划与冻结实现不一致时停止执行并重新授权
//! （计划 §7），不得单向迁就。

use super::coverage::span_control;
use super::*;

#[test]
fn cohort2_span_control_fallback_preserves_existing_carriers() {
    let inputs: Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/issue-1107-inputs.json"
    ))
    .unwrap();
    let grid_history: Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/issue-1107-grid-searches.json"
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
