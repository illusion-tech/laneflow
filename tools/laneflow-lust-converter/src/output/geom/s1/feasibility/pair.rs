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
    fn label(self) -> &'static str {
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

/// 零维单弦分支 `[0,1]`（P1-S/P2-S 共享、只执行一次；v3.1 执行条款 1）：
/// 单点域——按允许判据证实不可能 ⇒ 排除该分支；通过真实检查 ⇒ 见证；仅 0.01 m
/// 筛选失败 ⇒ 未决（`unresolved_reason = 0.01m_filter`，不生成排除叶）。
#[allow(dead_code)]
pub(crate) fn diagnose_single_chord(control: [Vec3; 6]) -> Value {
    let c = [control[0], control[1], control[2], control[3]];
    let verdict = witness(c, &[0.0, 1.0], None);
    let status = if verdict.is_some() {
        "local_witness"
    } else {
        "unresolved"
    };
    json!({"control_f64_bits":control.map(|p|p.map(f64::to_bits)),"kind":"single_chord",
        "status":status,"witness":verdict,
        "unresolved_reason":if verdict.is_none(){Some("0.01m_filter_or_sink")}else{None},
        "single_chord_deviation_m":grid::deviation_for_probe(c,0.0,1.0,quantize_vec3(c[0]).map(f64::from),quantize_vec3(c[3]).map(f64::from)),
        "scope":"zero-dimensional single-chord branch (E1 v3.1 execution clause 1)"})
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
