//! task-11（#1137 B 路 / #1154）：自定义几何集搜索驱动与逐配置尝试拒绝明细。
//!
//! 判定口径逐值沿用本模块树既有机器（`monotone` / `quality::fit` 双向拟合筛选 /
//! `scan` 切向角表 / `attempt` 双划分族 n∈2..=MAX_PIECES / `ExactSeen` 规范 f32
//! 去重 / `Ranked` §2.J 排序保留），零重定义；预算三分量自 `WorkBudget` 句柄
//! 预投影、退出按实耗回扣（截断语义与 grid/reshape 同构）。构造（几何集本身）属
//! 使用方模块；`preclassified` 使用方标签命中即行落标签、零预算。

// B（#1137）/D（#1154）使用方接线消费本模块 API（交付契约面）；接线后移除本豁免。
#![allow(dead_code)]

use super::*;

/// B 侧候选 cubic 链规格（构造属使用方；每 cubic 6 点位 [起点,柄,柄,终点,dir_a,dir_b]）。
pub(crate) struct GeometrySpec {
    /// 配置序号（行元数据；§2.J 排序键第四位）。
    pub(crate) config_index: usize,
    /// 组标签（keep / pair-L / pair-R / whole）。
    pub(crate) group: &'static str,
    /// 每组一对 (f_start, f_finish)（按组序；S2 pair-L 两组各一对）。
    pub(crate) factors: Vec<[f64; 2]>,
    /// 候选 cubic 链（f64 原控制点域）。
    pub(crate) cubics: Vec<[[f64; 3]; 6]>,
    /// 使用方预分类标签命中即行落标签、零预算（负对照/跳过语义）。
    pub(crate) preclassified: Option<&'static str>,
}

/// 逐配置行（B 冻结形态：六字段 + source_fit/arc_m/source_length_estimate 补齐）。
///
/// **词汇映射（D/B 复核建议 2/3，防跨机器汇总错位）**：本驱动
/// `backtracking_rejected` ≡ 旧机 `backtracking_control_rejected`、
/// `completed_finite_family` ≡ 旧机 `completed_finite_geometry_family`
/// ——语义同、字面异；对照列取冻结记录不碰撞。
pub(crate) struct AttemptRow {
    pub(crate) geometry_index: usize,
    pub(crate) description: Value,
    pub(crate) status: &'static str,
    /// 拒绝阶段计数（含 sink 子原因 degenerate_chord / weld_turn）。
    pub(crate) partition_rejections: BTreeMap<String, u64>,
    /// 成功 (族, n) 记录（族 0=等分 t、1=切向角 512）。
    pub(crate) successful_partitions: Vec<Value>,
    pub(crate) source_fit: Option<quality::Fit>,
    pub(crate) controls_f64_bits: Vec<[[u64; 3]; 6]>,
    pub(crate) arc_m: f64,
    pub(crate) source_length_estimate: f64,
}

/// §2.J 排序保留结果（截前 8；既有候选不占名额由使用方层处理）。
pub(crate) struct RetainedEntry {
    pub(crate) score: [f64; 3],
    pub(crate) order: [usize; 3],
    pub(crate) candidate: Candidate,
    pub(crate) curves: Vec<Control>,
    /// 最终保留程序的规范折线→来源双向观察（计入来源拟合工作预算；
    /// 预算尽时为 `None` 且 `observation_complete=false`，未完成观察保留）。
    pub(crate) canonical_source_fit: Option<quality::Fit>,
    pub(crate) observation_complete: bool,
}

/// 逐配置尝试拒绝明细（R2/R3 直调：钉住输入与 (族,n) 后直接取拒绝标签与子原因证据）。
///
/// `measured` 证据通道（D/B 复核建议 1）：sink 拒绝携带错误信息原文（内含量化弦长 /
/// 转角实测值与 0.10 m / 1.95° 阈值口径），R2/R3 直调断言据实测值落证。
pub(crate) struct RejectionDetail {
    /// 拒绝阶段（与 `AttemptRow.partition_rejections` 同词汇；通过记 "accepted"）。
    pub(crate) stage: &'static str,
    pub(crate) sink_sub_reason: Option<&'static str>,
    /// 实测证据（sink 错误原文 + (族,n)；阈值 0.10 m / 1.95° 为排除余量口径）。
    pub(crate) measured: Value,
}

/// 族名映射（B 口径 `old`/`even` = 等分 t；`grid`/`tangent_angle` = 切向角 512）。
fn family_angle(family: &str) -> bool {
    match family {
        "old" | "even" => false,
        "grid" | "tangent_angle" => true,
        other => panic!("unknown family {other}"),
    }
}

/// §2.J 排序（逐值沿 `rank_compare`：score 分量 `total_cmp` 字典序 + order 平次）。
fn retained_compare(a: &RetainedEntry, b: &RetainedEntry) -> std::cmp::Ordering {
    for (x, y) in a.score.iter().zip(b.score) {
        let order = x.total_cmp(&y);
        if order != std::cmp::Ordering::Equal {
            return order;
        }
    }
    a.order.cmp(&b.order)
}

/// 自定义几何集逐配置搜索驱动（B 驱动）：每配置每族取首个完整程序；三分量预算
/// 自句柄预投影/回扣。`source_length_estimate` = 原参考弦长估计（§2.J |差| 键）。
pub(crate) fn search_custom_geometries(
    input: &EdgeInput,
    max_curves: usize,
    geometries: &[GeometrySpec],
    handle: &mut super::super::grid::WorkBudget,
) -> (Vec<AttemptRow>, Vec<RetainedEntry>) {
    assert!(max_curves >= 1, "nonzero curve cap");
    assert!(input.points.len() >= 2);
    let original = reference_with_curve_cap(input, max_curves);
    let mut budget = Budget {
        sample_limit: Some(handle.grid_work_remaining),
        quality_limit: Some(handle.source_fit_remaining),
        partition_limit: Some(handle.partitions_remaining),
        ..Budget::default()
    };
    let reference_fit =
        quality::fit(&original, &input.points, &mut budget).expect("bounded reference fit");
    let reference_length = quality::lengths(&original);
    let reference_arc = reference_length["chord_estimate_m"].as_f64().unwrap();
    let mut rows = Vec::new();
    let mut pool: Vec<RetainedEntry> = Vec::new();
    let mut seen = ExactSeen::default();
    for spec in geometries {
        let controls_f64_bits = spec
            .cubics
            .iter()
            .map(|c| c.map(|p| p.map(f64::to_bits)))
            .collect::<Vec<_>>();
        let mut row = AttemptRow {
            geometry_index: spec.config_index,
            description: json!({"group":spec.group,"handle_factors":spec.factors,
                "config_index":spec.config_index}),
            status: "pending",
            partition_rejections: BTreeMap::new(),
            successful_partitions: Vec::new(),
            source_fit: None,
            controls_f64_bits,
            arc_m: 0.0,
            source_length_estimate: reference_arc,
        };
        if let Some(label) = spec.preclassified {
            row.status = label;
            rows.push(row);
            continue;
        }
        let curves: Vec<Control> = spec.cubics.clone();
        if !monotone(&curves) {
            row.status = "backtracking_rejected";
            rows.push(row);
            continue;
        }
        let Some(fit) = quality::fit(&curves, &input.points, &mut budget) else {
            row.status = "budget_stopped";
            rows.push(row);
            break;
        };
        row.source_fit = Some(fit);
        if !fit.within(reference_fit) {
            row.status = "source_fit_rejected";
            rows.push(row);
            continue;
        }
        let mut tables = Vec::new();
        for c in &curves {
            if !budget.sample((SCAN_STEPS + 1) as u64) {
                break;
            }
            tables.push(
                scan(&BezierSpan {
                    a: c[0],
                    c1: c[1],
                    c2: c[2],
                    b: c[3],
                    dir_a: c[4],
                    dir_b: c[5],
                })
                .0,
            );
        }
        let mut successes = 0usize;
        for (family, angle) in [(0, false), (1, true)] {
            if budget.stop.is_some() {
                break;
            }
            for n in 2..=MAX_PIECES {
                match attempt(&curves, &tables, angle, n, input, &mut budget) {
                    Attempt::Budget => break,
                    Attempt::Rejected(reason, sub) => {
                        let key = match (reason, sub) {
                            ("sink", Some(sub)) => format!("sink_rejected:{}", sub.reason),
                            (reason, _) => reason.to_owned(),
                        };
                        *row.partition_rejections.entry(key).or_default() += 1;
                    }
                    Attempt::Accepted(mut candidate) => {
                        candidate.spans[0]["geometry"] = json!(spec.config_index);
                        candidate.spans[0]["source_fit"] = json!(fit);
                        row.successful_partitions
                            .push(json!({"family":family,"n":n}));
                        row.arc_m = candidate.repaired.quantized_arc_meters;
                        successes += 1;
                        let key =
                            canonical_key(program_points(&candidate.repaired.program).into_iter());
                        match seen.insert(key, FIT_KEY_POINT_LIMIT) {
                            Retain::Duplicate => {}
                            Retain::Budget => {
                                budget.stop = Some("dedup_points");
                            }
                            Retain::New => {
                                let score = fit.score();
                                pool.push(RetainedEntry {
                                    score: [
                                        score[0],
                                        score[1],
                                        (candidate.repaired.quantized_arc_meters - reference_arc)
                                            .abs(),
                                    ],
                                    order: [spec.config_index, family, n as usize],
                                    candidate,
                                    curves: curves.clone(),
                                    canonical_source_fit: None,
                                    observation_complete: false,
                                });
                                pool.sort_by(retained_compare);
                                pool.truncate(PROGRAM_LIMIT);
                            }
                        }
                        budget.retained_key_points = seen.points;
                        break;
                    }
                }
            }
        }
        row.status = if successes > 0 {
            "completed_finite_family"
        } else if budget.stop.is_some() {
            "budget_stopped"
        } else {
            "no_complete_program"
        };
        rows.push(row);
        // 预算尽即止（与 search_with_curve_cap_budget 同语义）：后续配置不再进入
        // 拟合/scan，避免把预算停止误记为 source_fit_rejected。
        if budget.stop.is_some() {
            break;
        }
    }
    // 最终保留程序的规范折线→来源双向观察（计入来源拟合工作预算；预算尽时保留
    // 未完成观察并由停止原因记录）。
    for entry in pool.iter_mut() {
        let points = program_points(&entry.candidate.repaired.program);
        entry.canonical_source_fit = quality::polyline_fit(&points, &input.points, &mut budget);
        entry.observation_complete = entry.canonical_source_fit.is_some();
    }
    handle.grid_work_remaining = handle
        .grid_work_remaining
        .saturating_sub(budget.sample_work);
    handle.grid_work_consumed += budget.sample_work;
    handle.source_fit_remaining = handle
        .source_fit_remaining
        .saturating_sub(budget.quality_work);
    handle.source_fit_consumed += budget.quality_work;
    handle.partitions_remaining = handle
        .partitions_remaining
        .saturating_sub(budget.partitions);
    handle.partitions_consumed += budget.partitions;
    (rows, pool)
}

/// 逐配置尝试拒绝明细（R2/R3 直调用例）：钉住输入与 (族, n) 取拒绝阶段、sink 子原因
/// 与实测证据。独立临时预算（复核域/单次诊断口径，不占共享句柄）。
pub(crate) fn attempt_reject_detail(
    input: &EdgeInput,
    curves: &GeometrySpec,
    family: &str,
    n: u32,
) -> RejectionDetail {
    let angle = family_angle(family);
    let chain: Vec<Control> = curves.cubics.clone();
    let mut budget = Budget::default();
    let mut tables = Vec::new();
    for c in &chain {
        tables.push(
            scan(&BezierSpan {
                a: c[0],
                c1: c[1],
                c2: c[2],
                b: c[3],
                dir_a: c[4],
                dir_b: c[5],
            })
            .0,
        );
    }
    match attempt(&chain, &tables, angle, n, input, &mut budget) {
        Attempt::Accepted(_) => RejectionDetail {
            stage: "accepted",
            sink_sub_reason: None,
            measured: json!({}),
        },
        Attempt::Budget => RejectionDetail {
            stage: "budget_stopped",
            sink_sub_reason: None,
            measured: json!({}),
        },
        Attempt::Rejected(reason, sub) => RejectionDetail {
            stage: reason,
            sink_sub_reason: sub.as_ref().map(|s| s.reason),
            measured: match &sub {
                Some(s) => json!({"family":family,"n":n,"error_message":s.message}),
                None => json!({"family":family,"n":n}),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_mapping_matches_frozen_vocabulary() {
        assert!(!family_angle("old"));
        assert!(!family_angle("even"));
        assert!(family_angle("grid"));
        assert!(family_angle("tangent_angle"));
    }

    #[test]
    fn sink_sub_reason_taxonomy_maps_error_messages() {
        assert_eq!(
            sink_reject_sub_reason(
                "repaired curve emits a degenerate canonical segment: quantized length 0.05 m"
            ),
            "degenerate_chord"
        );
        assert_eq!(
            sink_reject_sub_reason(
                "repaired curve exceeds the quantized weld budget: 2.10 deg > 1.95 deg"
            ),
            "weld_turn"
        );
        assert_eq!(sink_reject_sub_reason("other"), "sink_other");
    }

    #[test]
    fn reject_detail_carries_measured_evidence() {
        // D/B 复核建议 1：实测证据通道——sink 拒绝携带错误原文（内含量化弦长实测值），
        // R2/R3 直调断言据实测值落证（R3 0.09m 相邻点距即 degenerate_chord 形态）。
        let input = EdgeInput {
            points: vec![
                [0.0, 0.0, 0.0],
                [4.0, 0.2, 0.0],
                [8.0, 0.2, 0.0],
                [12.0, 0.0, 0.0],
            ],
            clamps: [None, None],
            clamp_sources: [None, None],
        };
        let tiny = GeometrySpec {
            config_index: 0,
            group: "whole",
            factors: vec![[1.0, 1.0]],
            cubics: vec![[
                [0.0, 0.0, 0.0],
                [0.01, 0.0, 0.0],
                [0.02, 0.0, 0.0],
                [0.03, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
            ]],
            preclassified: None,
        };
        let detail = attempt_reject_detail(&input, &tiny, "even", 2);
        assert_eq!(detail.stage, "sink");
        assert_eq!(detail.sink_sub_reason, Some("degenerate_chord"));
        let message = detail.measured["error_message"].as_str().unwrap();
        assert!(
            message.contains("degenerate canonical segment"),
            "实测原文落证"
        );
    }

    #[test]
    fn preclassified_rows_consume_zero_budget() {
        // 负对照/跳过语义：preclassified 命中即行落标签、零预算（句柄恒等式不变）。
        let input = EdgeInput {
            points: vec![
                [0.0, 0.0, 0.0],
                [4.0, 0.2, 0.0],
                [8.0, 0.2, 0.0],
                [12.0, 0.0, 0.0],
            ],
            clamps: [None, None],
            clamp_sources: [None, None],
        };
        let specs = [GeometrySpec {
            config_index: 0,
            group: "whole",
            factors: vec![[1.0, 1.0]],
            cubics: vec![[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [3.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
            ]],
            preclassified: Some("skipped_old_family_overlap"),
        }];
        let mut handle = super::super::grid::WorkBudget::per_edge_default();
        let (p0, w0, f0) = (
            handle.partitions_remaining,
            handle.grid_work_remaining,
            handle.source_fit_remaining,
        );
        let (rows, pool) = search_custom_geometries(&input, 4, &specs, &mut handle);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, "skipped_old_family_overlap");
        assert!(pool.is_empty());
        // 句柄仍扣参考拟合与参考捕获实耗（驱动公共前置），但 preclassified 行本身零增量；
        // 恒等式保持。
        assert_eq!(
            handle.partitions_consumed + handle.partitions_remaining,
            p0,
            "partitions 恒等式"
        );
        assert_eq!(
            handle.grid_work_consumed + handle.grid_work_remaining,
            w0,
            "工作池恒等式"
        );
        assert_eq!(
            handle.source_fit_consumed + handle.source_fit_remaining,
            f0,
            "source_fit 恒等式"
        );
    }
}
