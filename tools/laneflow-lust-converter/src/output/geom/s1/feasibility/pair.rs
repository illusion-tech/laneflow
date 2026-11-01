//! E1 #1141（v3.1 冻结方案）：首两弦 / 末两弦 / 单弦分支的局部必要条件诊断。
//!
//! 判据语义（§0）：0.099999 m 量化弦长下限与 0.0341 sink 正切上界**仅作排除余量**，
//! 不得用于接受见证；真实见证沿实际量化长度、sink 转角与适用 0.01 m 筛选口径
//! （`witness` 内建）；见证重放失败 ≠ 排除证据。执行条款（v3.1 三分写）内建于
//! `branch_verdict` 汇总：零维单弦单点域、一维边界随二维盒证书核验、任一必要分支
//! 未决即禁标全参数域排除。

use super::*;

/// 双弦问题族（P1-T 首两弦 / P2-T 末两弦）。
#[derive(Clone, Copy)]
pub(crate) enum PairKind {
    /// 首两弦 `[0,u]`、`[u,v]`；域 `0<u<v≤1`（`u=0` 零长出域，`v=1` 合法剖分）。
    FirstTwo,
    /// 末两弦 `[u,v]`、`[v,1]`；域 `0≤u<v<1`（`u=0` 合法剖分，`v=1` 零长出域）。
    LastTwo,
}

impl PairKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            PairKind::FirstTwo => "first_two_chords",
            PairKind::LastTwo => "last_two_chords",
        }
    }

    /// 见证参数序（witness 逐窗检查两弦 + 相邻 sink 转角）。
    fn params(self, u: f64, v: f64) -> [f64; 3] {
        match self {
            PairKind::FirstTwo => [0.0, u, v],
            PairKind::LastTwo => [u, v, 1.0],
        }
    }

    /// 出域退化（不生成两弦见证、不作边界排除叶）：P1-T `u=0`、P2-T `v=1`。
    fn out_of_domain(self, u: f64, v: f64) -> bool {
        match self {
            PairKind::FirstTwo => u == 0.0,
            PairKind::LastTwo => v == 1.0,
        }
    }
}

/// 两弦必要条件盒排除谓词（排除余量口径；点盒常量化时直接重放真实检查）。
fn pair_reject(
    kind: PairKind,
    a: interval::Box2,
    b: interval::Box2,
    c: interval::Box2,
) -> Option<&'static str> {
    let (Some(a), Some(b), Some(c)) = (constant_point(a), constant_point(b), constant_point(c))
    else {
        return None;
    };
    if quantized_chord_meters(quantize_vec3(a), quantize_vec3(b)) <= QUANTIZED_MIN_SEGMENT_METERS {
        return Some("constant_quantized_rejection");
    }
    if quantized_chord_meters(quantize_vec3(b), quantize_vec3(c)) <= QUANTIZED_MIN_SEGMENT_METERS {
        return Some("constant_quantized_rejection");
    }
    let first = unit(sub(b, a)).unwrap();
    let second = unit(sub(c, b)).unwrap();
    if angle_excluded(
        [Interval::point(first[0]), Interval::point(first[2])],
        [Interval::point(second[0]), Interval::point(second[2])],
        0.0341,
    ) {
        return Some("terminal_turn");
    }
    let _ = kind;
    None
}

/// 双弦问题二维盒二分诊断（P1-T/P2-T 共用；遍历/包围/见证探查逐值沿
/// `diagnose_with_limits` 机器，问题定义为 v3.1 新增）。
fn diagnose_pair(kind: PairKind, control: [Vec3; 6]) -> Value {
    let c = [control[0], control[1], control[2], control[3]];
    assert!(
        c.iter()
            .all(|p| p[1] == 0.0 && p.iter().all(|x| x.is_finite() && x.abs() <= 32_000.0))
    );
    let domain = Interval { lo: 0.0, hi: 1.0 };
    let mut stack = vec![(domain, domain, 0_u32, 1_u64)];
    let mut leaves = Vec::new();
    let mut counts = BTreeMap::<&str, u64>::new();
    let mut visited = 0;
    let mut deepest = 0;
    let mut found = None;
    let mut unresolved = Vec::new();
    while let Some((u, v, depth, code)) = stack.pop() {
        if visited == BOX_LIMIT || leaves.len() == certificate::LIMIT {
            stack.push((u, v, depth, code));
            break;
        }
        visited += 1;
        deepest = deepest.max(depth);
        let reason = if u.lo >= v.hi {
            Some("parameter_order")
        } else {
            let (pa, pb, pc) = match kind {
                PairKind::FirstTwo => (point(quantize_vec3(c[0])), curve(c, u), curve(c, v)),
                PairKind::LastTwo => (curve(c, u), curve(c, v), point(quantize_vec3(c[3]))),
            };
            if short(difference(pb, pa)) || short(difference(pc, pb)) {
                Some("minimum_chord")
            } else {
                pair_reject(kind, pa, pb, pc)
            }
        };
        if let Some(reason) = reason {
            *counts.entry(reason).or_default() += 1;
            leaves.push((
                code,
                match reason {
                    "constant_quantized_rejection" => 1,
                    "minimum_chord" => 2,
                    "terminal_turn" => 3,
                    _ => 0,
                },
            ));
            continue;
        }
        let um = (u.lo + u.hi) * 0.5;
        let vm = (v.lo + v.hi) * 0.5;
        if !kind.out_of_domain(um, vm)
            && let Some(candidate) = witness(c, &kind.params(um, vm), None)
        {
            found = Some(candidate);
            break;
        }
        if depth == DEPTH_LIMIT {
            *counts.entry("unresolved_depth").or_default() += 1;
            if unresolved.len() < 16 {
                unresolved.push(json!({"u":[u.lo,u.hi],"v":[v.lo,v.hi]}));
            }
            continue;
        }
        if u.hi - u.lo > v.hi - v.lo {
            stack.push((Interval { lo: um, hi: u.hi }, v, depth + 1, code * 2 + 1));
            stack.push((Interval { lo: u.lo, hi: um }, v, depth + 1, code * 2));
        } else {
            stack.push((u, Interval { lo: vm, hi: v.hi }, depth + 1, code * 2 + 1));
            stack.push((u, Interval { lo: v.lo, hi: vm }, depth + 1, code * 2));
        }
    }
    let excluded = found.is_none() && stack.is_empty() && unresolved.is_empty();
    json!({"control_f64_bits":control.map(|p|p.map(f64::to_bits)),"kind":kind.label(),
        "status":if found.is_some(){"local_witness"}else if excluded{"necessary_condition_excluded"}else{"unresolved"},
        "witness":found,"visited_boxes":visited,"deepest":deepest,"excluded_boxes":counts,
        "pending_boxes":stack.len(),"first_unresolved_boxes":unresolved,
        "policy":{"box_limit":BOX_LIMIT,"depth_limit":DEPTH_LIMIT,"certificate_leaf_limit":certificate::LIMIT,
            "length_exclusion_m":0.099_999,"turn_tangent_upper":0.0341,
            "direction_and_length_exclusion_are_relaxed":true,"deviation_omitted_from_exclusion":true},
        "scope":"fixed planar cubic two-chord local necessary condition (E1 v3.1); witness is local existence only"})
}

/// 零维单弦判定（v3.1 执行条款 1 三分写）：允许判据证实不可能 ⇒ 排除该分支
/// （本分支允许判据仅量化弦长下限 0.099999；单弦无内部转角条件）；通过真实检查 ⇒
/// 见证；仅 0.01 m 筛选失败 ⇒ 未决（`0.01m_filter`）；sink 真实检查失败而未违
/// 排除余量 ⇒ 未决（`sink_real_check`，§0：真实失败不得回填排除叶）。
fn single_chord_verdict(c: [Vec3; 4]) -> Value {
    let q0 = quantize_vec3(c[0]);
    let q3 = quantize_vec3(c[3]);
    let chord_m = quantized_chord_meters(q0, q3);
    if chord_m <= QUANTIZED_MIN_SEGMENT_METERS {
        return json!({"status":"necessary_condition_excluded","reason":"minimum_chord",
            "chord_meters":chord_m,
            "scope":"zero-dimensional single chord; exclusion margin criterion only"});
    }
    let deviation = grid::deviation_for_probe(c, 0.0, 1.0, q0.map(f64::from), q3.map(f64::from));
    if deviation > 0.01 {
        return json!({"status":"unresolved","unresolved_reason":"0.01m_filter",
            "chord_meters":chord_m,"deviation_m":deviation});
    }
    match witness(c, &[0.0, 1.0], None) {
        Some(w) => json!({"status":"local_witness","witness":w,
            "chord_meters":chord_m,"deviation_m":deviation}),
        None => json!({"status":"unresolved","unresolved_reason":"sink_real_check",
            "chord_meters":chord_m,"deviation_m":deviation}),
    }
}

/// 零维单弦分支 `[0,1]`（P1-S/P2-S 共享、只执行一次；v3.1 执行条款 1 三分写）。
#[allow(dead_code)]
pub(crate) fn diagnose_single_chord(control: [Vec3; 6]) -> Value {
    let c = [control[0], control[1], control[2], control[3]];
    let mut result = single_chord_verdict(c);
    result["control_f64_bits"] = json!(control.map(|p| p.map(f64::to_bits)));
    result["kind"] = json!("single_chord");
    result
}

/// P1-T 首两弦 `[0,u],[u,v]`（域 `0<u<v≤1`）。
#[allow(dead_code)]
pub(crate) fn diagnose_first_two_chords(control: [Vec3; 6]) -> Value {
    diagnose_pair(PairKind::FirstTwo, control)
}

/// P2-T 真实末两弦 `[u,v],[v,1]`（域 `0≤u<v<1`；`v=1` 零长不作两弦见证，
/// 去重 `[u,1]` 只记独立末弦诊断）。
#[allow(dead_code)]
pub(crate) fn diagnose_last_two_chords(control: [Vec3; 6]) -> Value {
    diagnose_pair(PairKind::LastTwo, control)
}

/// 已知参数重放（v3.1 §4 A′「参数重放」类）：给定 (u,v) 直接按真实接受口径验两弦
/// 见证（`witness` 内建 0.01 筛选 + sink 真实检查 + 相邻转角），不经搜索；
/// 出域退化（P1-T `u=0` / P2-T `v=1`）返回 `None`。
#[allow(dead_code)]
pub(crate) fn replay_pair_witness(
    kind: PairKind,
    control: [Vec3; 6],
    u: f64,
    v: f64,
) -> Option<Value> {
    if kind.out_of_domain(u, v) {
        return None;
    }
    let c = [control[0], control[1], control[2], control[3]];
    witness(c, &kind.params(u, v), None)
}

/// 问题汇总规则（v3.1 执行条款 3）：单弦分支 + 对应两弦分支均排除才记
/// 「该固定 cubic 的完整发射被该必要条件否定」；任一必要分支未决即禁标全参数域排除。
#[allow(dead_code)]
pub(crate) fn branch_verdict(single: &Value, pair: &Value) -> &'static str {
    let excluded = |v: &Value| v["status"] == "necessary_condition_excluded";
    let unresolved = |v: &Value| v["status"] == "unresolved";
    if excluded(single) && excluded(pair) {
        "complete_emission_negated_by_necessary_condition"
    } else if unresolved(single) || unresolved(pair) {
        "unresolved_blocks_full_domain_exclusion"
    } else {
        "branch_partial_record_only"
    }
}

/// P3 跨 cubic 接缝对 `[s,1]│[0,t]`（v3.1）：两侧弦同时合法的必要条件。沿
/// `seam::diagnose` 模型（缝点一致性**直接核对**共同端点与规范量化点由其
/// `assert_eq!(left[3], right[0])` + `quantize_vec3(joint)` 承担，不用区间包含
/// 判定）；边界分支**单列记录**（s=0 左侧整 cubic 单弦 / t=1 右侧整 cubic 单弦，
/// 零维语义按执行条款 1 三分写）；汇总按执行条款 3。
#[allow(dead_code)]
pub(crate) fn diagnose_seam_pair(left: [Vec3; 4], right: [Vec3; 4]) -> Value {
    let pair = super::seam::diagnose(left, right);
    let boundaries = json!({
        "s0_left_single_chord": single_chord_verdict(left),
        "t1_right_single_chord": single_chord_verdict(right),
    });
    let pair_excluded = pair["status"] == "necessary_condition_excluded";
    let pair_unresolved = pair["status"] == "unresolved";
    let boundary_unresolved = boundaries["s0_left_single_chord"]["status"] == "unresolved"
        || boundaries["t1_right_single_chord"]["status"] == "unresolved";
    let boundary_excluded = boundaries["s0_left_single_chord"]["status"]
        == "necessary_condition_excluded"
        && boundaries["t1_right_single_chord"]["status"] == "necessary_condition_excluded";
    let verdict = if pair_unresolved || boundary_unresolved {
        "unresolved_blocks_full_domain_exclusion"
    } else if pair_excluded && boundary_excluded {
        "complete_emission_negated_by_necessary_condition"
    } else {
        "branch_partial_record_only"
    };
    json!({"kind":"seam_pair","pair":pair,"boundaries":boundaries,"branch_verdict":verdict,
        "scope":"fixed adjacent planar cubics seam pair (E1 v3.1); s=0/t=1 boundary single-chord branches recorded separately"})
}

/// 原采样参数恢复（v3.1 修正③：f32 量化多对一，「由规范点位反解」不成立）：
/// 原族控制点 + `partition` + `pieces` 调**冻结算法**重建切分参数（tangent_angle_512
/// → `scan` 累计切向表 + `split_params`；等分族 → (0..=n)/n）；`scan` 仅用 4 点位
/// （dir 不参与），重建仅需 `control_f64_bits`。零转角（直线）无切向表 ⇒ `None`
/// （对应 `angle_partition_unavailable`）。重建后须与候选程序生成规范点**逐位核对**
/// （调用方断言）。
#[allow(dead_code)]
pub(crate) fn recovered_split_params(
    control: [Vec3; 6],
    partition: &str,
    pieces: u32,
) -> Option<Vec<f64>> {
    let c = [control[0], control[1], control[2], control[3]];
    match partition {
        "tangent_angle_512" => {
            let (table, _) = scan(&BezierSpan {
                a: c[0],
                c1: c[1],
                c2: c[2],
                b: c[3],
                dir_a: control[4],
                dir_b: control[5],
            });
            table.map(|table| split_params(&table, pieces))
        }
        "even" | "uniform" => Some(
            (0..=pieces)
                .map(|i| f64::from(i) / f64::from(pieces))
                .collect(),
        ),
        other => panic!("unknown frozen partition {other}"),
    }
}

/// 重建切分点的规范量化 f32 位（逐位核对用；修正③冻结算法重建 → eval → 量化）。
#[allow(dead_code)]
pub(crate) fn rebuilt_quantized_points(
    control: [Vec3; 6],
    partition: &str,
    pieces: u32,
) -> Option<Vec<[u32; 3]>> {
    let params = recovered_split_params(control, partition, pieces)?;
    let c = [control[0], control[1], control[2], control[3]];
    Some(
        params
            .iter()
            .map(|t| {
                let p = if *t == 0.0 {
                    c[0]
                } else if *t == 1.0 {
                    c[3]
                } else {
                    cubic_at(c[0], c[1], c[2], c[3], *t)
                };
                quantize_vec3(p).map(f32::to_bits)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn straight() -> [Vec3; 6] {
        [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
        ]
    }

    #[test]
    fn straight_cubic_must_witness_all_branches() {
        // 必证正例（lead 批 2 规格）：合成直线 cubic——单弦 [0,1]、首两弦/末两弦
        // 边界 [0,0.5,1] 形态均须出见证（不误杀 + 必证）。
        let control = straight();
        assert_eq!(diagnose_single_chord(control)["status"], "local_witness");
        assert_eq!(
            diagnose_first_two_chords(control)["status"],
            "local_witness"
        );
        assert_eq!(diagnose_last_two_chords(control)["status"], "local_witness");
        let left = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
        ];
        let right = [
            [3.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
            [5.0, 0.0, 0.0],
            [6.0, 0.0, 0.0],
        ];
        let out = diagnose_seam_pair(left, right);
        assert_eq!(out["pair"]["status"], "local_witness");
        assert_eq!(
            out["boundaries"]["s0_left_single_chord"]["status"],
            "local_witness"
        );
        assert_eq!(
            out["boundaries"]["t1_right_single_chord"]["status"],
            "local_witness"
        );
    }

    #[test]
    fn branch_verdict_blocks_exclusion_when_unresolved() {
        // 执行条款 3：任一必要分支未决即禁标全参数域排除。
        let excluded = json!({"status":"necessary_condition_excluded"});
        let witness = json!({"status":"local_witness"});
        let unresolved = json!({"status":"unresolved"});
        assert_eq!(
            branch_verdict(&excluded, &excluded),
            "complete_emission_negated_by_necessary_condition"
        );
        assert_eq!(
            branch_verdict(&unresolved, &excluded),
            "unresolved_blocks_full_domain_exclusion"
        );
        assert_eq!(
            branch_verdict(&witness, &excluded),
            "branch_partial_record_only"
        );
    }

    #[test]
    fn short_single_chord_is_excluded_only_by_margin() {
        // 执行条款 1 三分写：量化弦长 ≤ 0.099999（排除余量）⇒ 排除该分支。
        let tiny = [
            [0.0, 0.0, 0.0],
            [0.01, 0.0, 0.0],
            [0.02, 0.0, 0.0],
            [0.03, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
        ];
        assert_eq!(
            diagnose_single_chord(tiny)["status"],
            "necessary_condition_excluded"
        );
    }

    #[test]
    fn recovered_split_params_rebuilds_frozen_families() {
        // 参数恢复（修正③）：tangent_angle_512 经 scan/split_params 冻结算法重建、
        // 端点 0/1 保持且严格递增；等分族 (0..=n)/n；直线（零转角）无切向表 ⇒ None。
        let curved = [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
        ];
        let params = recovered_split_params(curved, "tangent_angle_512", 5).unwrap();
        assert_eq!(params.len(), 6);
        assert_eq!(params[0], 0.0);
        assert_eq!(params[5], 1.0);
        assert!(
            params.windows(2).all(|w| w[0] < w[1]),
            "strictly increasing"
        );
        let even = recovered_split_params(straight(), "even", 4).unwrap();
        assert_eq!(even, vec![0.0, 0.25, 0.5, 0.75, 1.0]);
        assert!(
            recovered_split_params(straight(), "tangent_angle_512", 5).is_none(),
            "零转角无切向表"
        );
    }
}
