//! #1221：四个最小多边案例（C06/C08/C17/C29）的固定曲线搜索补齐、逐边局部必要条件
//! 诊断与候选齐备分量组合验收。执行口径全部冻结于
//! `research/issue-1121-coverage-diagnosis/plan.md`；本模块不改生产采样、阈值与原成功
//! 几何。计划与冻结实现不一致时停止执行并重新授权（计划 §7），不得单向迁就。

use super::*;

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
fn grid_membership_is_computed_from_the_frozen_grid() {
    assert!(grid_node_member(0.0));
    assert!(grid_node_member(0.5));
    assert!(grid_node_member(1.0));
    assert!(grid_node_member(371.0 / 512.0));
    assert!(!grid_node_member(0.726_552_963_256_836));
}
