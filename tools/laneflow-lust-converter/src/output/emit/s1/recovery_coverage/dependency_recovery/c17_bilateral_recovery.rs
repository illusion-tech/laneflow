//! C17 双边恢复研究入口（#1137，B 路）：两程序齐备的完整路径联合验收。
//!
//! 新研究方案（S1 切向扰动族 / S2 新组序×逐组柄因子族 / J 双边联合验收）见
//! `research/issue-1137-c17-bilateral-recovery/design.md`（送审稿，评审冻结后才进
//! 实验）；方法事实源 `research/issue-1115-failed-curve-design/design.md` 的筛选、
//! 排序与预算口径逐值沿用。旧族（#1131 148 配置）只作对照列，不重跑充当研究。
//! 模块名带分量名，避免与其他并行研究入口冲突。

/// C17 分量与完整路径（冻结样本；`research/issue-1137-.../edges.json`）。
const C17_COMPONENT: &str = "sumo:-10564:--32172#4_0-to--31194_0::-10564_3_0.:-10564_6_0";
const C17_PATHS: [&str; 1] = ["sumo:-10564:--32172#4_0-to--31194_0::-10564_3_0.:-10564_6_0"];

/// C17 两条失败边（冻结样本）：`(边, 原参考链 cubic 数 s, 临时程序点上限 96s+1)`。
const C17_EDGES: [(&str, usize, usize); 2] =
    [("sumo::-10564_3_0", 2, 193), ("sumo::-10564_6_0", 3, 289)];

/// `sumo::-10564_3_0` 的既有候选（#1131 几何 80；按去重规则进边池，新增分列）。
const PRIOR_CANDIDATE_EDGE: &str = "sumo::-10564_3_0";
const PRIOR_CANDIDATE_GEOMETRY: u64 = 80;
const PRIOR_CANDIDATE_PIECES_PER_CUBIC: u64 = 22;

/// 短首段硬障碍复核证书的冻结输入：#1131 冻结记录的原参考控制点位（f64 bits），
/// 每边 `[a, c1, c2, b, dir_a, dir_b]` × 每点 `[x, y, z]`，多余 cubic 槽位补零。
type FrozenControls = [[[[u64; 3]; 6]; 3]; 2];

#[allow(clippy::unreadable_literal)]
const FROZEN_REFERENCE_CONTROLS: FrozenControls = [
    // sumo::-10564_3_0（s=2，第三槽补零）
    [
        [
            [13877348483060381778, 0, 13866689685399325901],
            [13877345486241484716, 0, 13866680438433146512],
            [13877342556268718226, 0, 13866670402143256247],
            [13877340258713406013, 0, 13866658371308166840],
            [4605307063872131583, 0, 4603676713425315769],
            [4603642952959233115, 0, 4605332979650819777],
        ],
        [
            [13877340258713406013, 0, 13866658371308166840],
            [13877338459644418656, 0, 13866648950728742133],
            [13877337635471176659, 0, 13866637107692290833],
            [13877336388432476242, 0, 13866626353529566003],
            [4603642952959233115, 0, 4605332979650819777],
            [4601251624040798569, 0, 4606346236788486668],
        ],
        [[0; 3]; 6],
    ],
    // sumo::-10564_6_0（s=3）
    [
        [
            [13877336388432476242, 0, 13866626353529566003],
            [13877336266924691829, 0, 13866625305675569592],
            [13877336053475297803, 0, 13866624340045690971],
            [13877336036588755354, 0, 13866623186936078008],
            [4601251624040798569, 0, 4606346236788486668],
            [4588588268676627364, 0, 4607167005197765791],
        ],
        [
            [13877336036588755354, 0, 13866623186936078008],
            [13877335847059112548, 0, 13866610244769040712],
            [13877335229848139814, 0, 13866597089414467497],
            [13877335772705964687, 0, 13866584308204919849],
            [4588588268676627364, 0, 4607167005197765791],
            [13818574609086382448, 0, 4607055177306663222],
        ],
        [
            [13877335772705964687, 0, 13866584308204919849],
            [13877336407152818409, 0, 13866555488517147752],
            [13877337823069872058, 0, 13866526642034661364],
            [13877339511045499126, 0, 13866499513868185764],
            [13818574609086382448, 0, 4607055177306663222],
            [13825071261084250887, 0, 4606238657439179098],
        ],
    ],
];

/// 证书复算值集合（自冻结控制点位独立计算；证明常量为计算使用值）。
struct CertificateComputed {
    legal_chord_floor: f64,
    weld_max_turn_deg: f64,
    pieces_per_cubic_min: f64,
    ulp: f64,
    total_margin: f64,
    chord_pair_lower: f64,
    span0: f64,
    worst_hull: f64,
    gap: f64,
    deviation: f64,
    forms: [f64; 9],
}

fn certificate_recompute() -> CertificateComputed {
    type V = [f64; 3];
    fn sub(a: V, b: V) -> V {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }
    fn add(a: V, b: V) -> V {
        [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
    }
    fn scale(a: V, k: f64) -> V {
        [a[0] * k, a[1] * k, a[2] * k]
    }
    fn dot(a: V, b: V) -> f64 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }
    fn norm(a: V) -> f64 {
        dot(a, a).sqrt()
    }
    fn dist(a: V, b: V) -> f64 {
        norm(sub(a, b))
    }
    fn unit(a: V) -> V {
        scale(a, 1.0 / norm(a))
    }
    fn hull_diameter(points: &[V]) -> f64 {
        let mut worst = 0.0_f64;
        for (i, a) in points.iter().enumerate() {
            for b in &points[i + 1..] {
                worst = worst.max(dist(*a, *b));
            }
        }
        worst
    }
    let controls = |edge: usize, cubic: usize, index: usize| -> V {
        FROZEN_REFERENCE_CONTROLS[edge][cubic][index].map(f64::from_bits)
    };
    // 证明常量（计算使用值；证书声明值必须逐项一致）。
    let legal_chord_floor = 0.099_999_f64;
    let weld_max_turn_deg = 1.95_f64;
    let pieces_per_cubic_min = 2.0_f64;
    // 余量：f32 量化 √3·ulp + 浮点曲线求值舍入（沿 feasibility/interval.rs 的
    // roundoff 口径 128·f64::EPSILON·尺度）。
    let max_coord = FROZEN_REFERENCE_CONTROLS
        .iter()
        .flatten()
        .flatten()
        .flatten()
        .map(|bits| f64::from_bits(*bits).abs())
        .fold(0.0_f64, f64::max);
    let ulp = 2f64.powi(max_coord.log2().floor() as i32 - 23);
    let quantization_margin = 3f64.sqrt() * ulp;
    let eval_roundoff_margin = 128.0 * f64::EPSILON * max_coord;
    let total_margin = quantization_margin + eval_roundoff_margin;
    let chord_pair_lower =
        2.0 * legal_chord_floor * (weld_max_turn_deg / 2.0_f64).to_radians().cos();

    // 结点与固定端点 cubic 形态的端距。
    let (e3, e6) = (0, 1);
    let p = |edge: usize, knot: usize| -> V {
        if knot == 0 {
            controls(edge, 0, 0)
        } else {
            controls(edge, knot - 1, 3)
        }
    };
    let span_distance = |edge: usize, a: usize, b: usize| dist(p(edge, a), p(edge, b));
    let e6_s0 = span_distance(e6, 0, 1);
    let forms = [
        span_distance(e6, 1, 2),
        span_distance(e6, 2, 3),
        span_distance(e6, 0, 2),
        span_distance(e6, 2, 3),
        span_distance(e6, 1, 3),
        span_distance(e6, 0, 3),
        span_distance(e3, 0, 1),
        span_distance(e3, 1, 2),
        span_distance(e3, 0, 2),
    ];
    // 结点 1 切向来源断言：interior CR 中心差分，非规定倒圆切向。
    let a = p(e6, 0);
    let b = p(e6, 1);
    let t0 = controls(e6, 0, 4);
    let t1 = controls(e6, 0, 5);
    let central = unit(sub(p(e6, 2), p(e6, 0)));
    assert!(
        dist(t1, central) < 1e-12,
        "knot-1 tangent is the interior CR central difference"
    );
    // 旋转平面第一层正交分量对 CR 结点必然退化；前一弦正交分量为正常路径。
    let primary = sub(central, scale(t1, dot(central, t1)));
    assert!(
        norm(primary) < 1e-9,
        "first orthogonal component degenerate for CR knot"
    );
    let chord_unit = unit(sub(b, a));
    let w_raw = sub(chord_unit, scale(t1, dot(chord_unit, t1)));
    assert!(norm(w_raw) >= 1e-9, "fallback rotation plane available");
    let w = unit(w_raw);
    let chord = dist(a, b);
    let handle = chord / 3.0;

    // S1 keep 首 cubic（5 个 δ 扰动）与 pair-R 首组（16 组柄因子）的凸包直径。
    let mut worst_hull = 0.0_f64;
    for degrees in [-12.0_f64, -6.0, 0.0, 6.0, 12.0] {
        let radians = degrees.to_radians();
        let t1_rotated = add(scale(t1, radians.cos()), scale(w, radians.sin()));
        let controls4 = [
            a,
            add(a, scale(t0, handle)),
            sub(b, scale(t1_rotated, handle)),
            b,
        ];
        worst_hull = worst_hull.max(hull_diameter(&controls4));
    }
    for f_start in [0.75, 1.0, 1.25, 1.5] {
        for f_finish in [0.75, 1.0, 1.25, 1.5] {
            let controls4 = [
                a,
                add(a, scale(t0, handle * f_start)),
                sub(b, scale(t1, handle * f_finish)),
                b,
            ];
            worst_hull = worst_hull.max(hull_diameter(&controls4));
        }
    }

    // 负对照判据反例锚：pair-R 首组 (1.5, 1)、等 t、n=2 首窗采样偏离。
    fn split_left(c: [V; 4], t: f64) -> [V; 4] {
        let lerp = |p: V, q: V| add(p, scale(sub(q, p), t));
        let p01 = lerp(c[0], c[1]);
        let p12 = lerp(c[1], c[2]);
        let p23 = lerp(c[2], c[3]);
        let p012 = lerp(p01, p12);
        let p123 = lerp(p12, p23);
        [c[0], p01, p012, lerp(p012, p123)]
    }
    fn quantize(p: V) -> V {
        [p[0] as f32 as f64, p[1] as f32 as f64, p[2] as f32 as f64]
    }
    let cubic_r = [
        a,
        add(a, scale(t0, handle * 1.5)),
        sub(b, scale(t1, handle)),
        b,
    ];
    let half = split_left(cubic_r, 0.5);
    let mid = scale(
        add(
            add(a, scale(cubic_r[1], 3.0)),
            add(scale(cubic_r[2], 3.0), b),
        ),
        0.125,
    );
    let from = quantize(a);
    let to = quantize(mid);
    let line = [
        from,
        add(from, scale(sub(to, from), 1.0 / 3.0)),
        add(from, scale(sub(to, from), 2.0 / 3.0)),
        to,
    ];
    let deviation = (0..4)
        .map(|i| dist(half[i], line[i]))
        .fold(0.0_f64, f64::max);

    let gap = chord_pair_lower - (worst_hull + total_margin);
    CertificateComputed {
        legal_chord_floor,
        weld_max_turn_deg,
        pieces_per_cubic_min,
        ulp,
        total_margin,
        chord_pair_lower,
        span0: e6_s0,
        worst_hull,
        gap,
        deviation,
        forms,
    }
}

/// 证书逐声明核对（#1137 复审意见 3）：三个证明常量声明值与计算使用值逐项一致；
/// 边界按方向单向核对（上界声明 ≥ 复算值、下界声明 ≤ 复算值），不用对称容差；
/// 最终缺口不等式用核验后的保守界验证。返回 `Err` 描述首个不满足的声明。
fn verify_certificate(
    certificate: &serde_json::Value,
    computed: &CertificateComputed,
) -> std::result::Result<(), String> {
    let constants = &certificate["constants"];
    let claims = &certificate["claims"];
    let number = |value: &serde_json::Value, label: &str| -> std::result::Result<f64, String> {
        let parsed = value
            .as_f64()
            .ok_or_else(|| format!("{label}: missing numeric claim"))?;
        if parsed.is_finite() {
            Ok(parsed)
        } else {
            Err(format!("{label}: non-finite numeric claim"))
        }
    };
    let eq = |claim: f64, used: f64, label: &str| -> std::result::Result<(), String> {
        if claim.to_bits() != used.to_bits() {
            return Err(format!("{label}: claim {claim} != used {used}"));
        }
        Ok(())
    };
    let upper = |claim: f64, value: f64, label: &str| -> std::result::Result<(), String> {
        if claim < value {
            return Err(format!("{label}: upper bound {claim} < computed {value}"));
        }
        Ok(())
    };
    let lower = |claim: f64, value: f64, label: &str| -> std::result::Result<(), String> {
        if claim > value {
            return Err(format!("{label}: lower bound {claim} > computed {value}"));
        }
        Ok(())
    };
    // 证明常量与派生常量：声明值 vs 计算使用值（逐位一致）。
    eq(
        number(&constants["legal_chord_floor_m"], "legal_chord_floor_m")?,
        computed.legal_chord_floor,
        "legal_chord_floor_m",
    )?;
    eq(
        number(&constants["weld_max_turn_deg"], "weld_max_turn_deg")?,
        computed.weld_max_turn_deg,
        "weld_max_turn_deg",
    )?;
    eq(
        number(&constants["pieces_per_cubic_min"], "pieces_per_cubic_min")?,
        computed.pieces_per_cubic_min,
        "pieces_per_cubic_min",
    )?;
    eq(
        number(&constants["f32_ulp_m"], "f32_ulp_m")?,
        computed.ulp,
        "f32_ulp_m",
    )?;
    if computed.pieces_per_cubic_min < 2.0 {
        return Err("two-chord argument requires pieces_per_cubic_min >= 2".to_owned());
    }
    // 边界方向单向核对（上界 ≥ 复算值、下界 ≤ 复算值）。
    upper(
        number(&claims["affected_config_hull_upper_bound_m"], "hull_upper")?,
        computed.worst_hull,
        "affected_config_hull_upper_bound_m",
    )?;
    upper(
        number(&claims["total_margin_upper_bound_m"], "margin_upper")?,
        computed.total_margin,
        "total_margin_upper_bound_m",
    )?;
    lower(
        number(&claims["chord_pair_lower_bound_m"], "chord_lower")?,
        computed.chord_pair_lower,
        "chord_pair_lower_bound_m",
    )?;
    let hull_upper = number(&claims["affected_config_hull_upper_bound_m"], "hull_upper")?;
    let margin_upper = number(&claims["total_margin_upper_bound_m"], "margin_upper")?;
    let chord_lower = number(&claims["chord_pair_lower_bound_m"], "chord_lower")?;
    lower(
        number(&claims["gap_lower_bound_m"], "gap_lower")?,
        chord_lower - hull_upper - margin_upper,
        "gap_lower_bound_m",
    )?;
    // 最终缺口不等式用核验后的保守界。
    if hull_upper + margin_upper >= chord_lower {
        return Err("conservative bounds do not establish the obstruction".to_owned());
    }
    if number(&claims["gap_lower_bound_m"], "gap_lower")? <= 2.0e-3 {
        return Err("gap lower bound must stay above 2 mm".to_owned());
    }
    // 展示值（仅供阅读，宽松核对，不参与证明）。
    let display = |claim: f64, value: f64, label: &str| -> std::result::Result<(), String> {
        if (claim - value).abs() > 1e-6 {
            return Err(format!("{label}: display {claim} too far from {value}"));
        }
        Ok(())
    };
    display(
        number(
            &claims["span0_endpoint_distance_display_m"],
            "span0_display",
        )?,
        computed.span0,
        "span0_endpoint_distance_display_m",
    )?;
    display(
        number(&claims["gap_display_m"], "gap_display")?,
        computed.gap,
        "gap_display_m",
    )?;
    // 负对照判据文本与反例锚。
    if certificate["negative_control_criterion"]["expectation"].as_str()
        != Some(
            "负对照预期零合法程序；最早拒绝阶段如实记录（管线先采样偏离后 sink，不预期全 sink 拒绝）",
        )
    {
        return Err("negative control criterion text changed".to_owned());
    }
    let counter = &certificate["negative_control_criterion"]["counter_example"];
    lower(
        number(
            &counter["first_window_deviation_lower_bound_m"],
            "deviation_lower",
        )?,
        computed.deviation,
        "first_window_deviation_lower_bound_m",
    )?;
    display(
        number(
            &counter["first_window_deviation_display_m"],
            "deviation_display",
        )?,
        computed.deviation,
        "first_window_deviation_display_m",
    )?;
    if computed.deviation <= 0.01 {
        return Err("counter-example must exceed the 0.01 m filter".to_owned());
    }
    // 不受影响形态：下界声明 ≤ 复算值；即使取下界仍使两弦论据不适用。
    let forms = &certificate["unaffected_forms"];
    for (label, computed_form) in [
        ("e6_s1_lower_bound_m", computed.forms[0]),
        ("e6_s2_lower_bound_m", computed.forms[1]),
        ("e6_pair_l_first_group_lower_bound_m", computed.forms[2]),
        ("e6_pair_l_second_group_lower_bound_m", computed.forms[3]),
        ("e6_pair_r_second_group_lower_bound_m", computed.forms[4]),
        ("e6_whole_lower_bound_m", computed.forms[5]),
        ("e3_s0_lower_bound_m", computed.forms[6]),
        ("e3_s1_lower_bound_m", computed.forms[7]),
        ("e3_whole_lower_bound_m", computed.forms[8]),
    ] {
        let claim = number(&forms[label], label)?;
        lower(claim, computed_form, label)?;
        if claim + margin_upper <= chord_lower {
            return Err(format!("{label}: two-chord argument not silenced"));
        }
    }
    Ok(())
}

/// 短首段硬障碍复核证书（#1137 维护者复审意见 2/3 的独立复算与绑定）。
///
/// 结论只覆盖列明配置（`sumo::-10564_6_0` keep 模式首 cubic 与 pair-R 首组），
/// 不写整条边无解。Bézier 采样点必在控制点凸包内；两条连续合法弦（量化弦长
/// 严格大于 0.1 m，保守下限 0.099999 m）的焊接全角 ≤ 1.95° ⇒ 其端距 ≥
/// 2×0.099999×cos(0.975°)；凸包上界 + 总余量上界 < 该下界 ⇒ 每 cubic ≥2 片的
/// 任何划分不可能产生两条连续合法弦。
#[test]
fn short_first_span_obstruction_certificate_recheck() {
    let computed = certificate_recompute();
    assert!(
        (computed.span0 - 0.196_977).abs() < 5e-7,
        "span0 endpoint distance anchor"
    );
    assert!(
        computed.deviation > 0.01,
        "sampling_deviation precedes sink"
    );
    assert!(
        computed.worst_hull + computed.total_margin < computed.chord_pair_lower,
        "intrinsic obstruction inequality"
    );
    let certificate: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1137-c17-bilateral-recovery/short-span-certificate.json"
    ))
    .unwrap();
    verify_certificate(&certificate, &computed).expect("certificate claims verified");
    // 诊断值改零 I/O 形态（CodeQL 日志写入告警消除；断言面不受影响）
    let _diagnostic = format!(
        "short-span certificate: span0={:.17} worst_hull={:.17} total_margin={:.17} \
         chord_pair_lower={:.17} gap={:.17} deviation={:.17}",
        computed.span0,
        computed.worst_hull,
        computed.total_margin,
        computed.chord_pair_lower,
        computed.gap,
        computed.deviation
    );
}

/// 参数化负验证清单（#1137 复审意见 3）：篡改任一证明常量、边界值或判据文本
/// 必须被逐声明核对拒绝。维护者可对清单外字段继续抽验。
#[test]
fn certificate_tampering_is_rejected() {
    let computed = certificate_recompute();
    let certificate: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1137-c17-bilateral-recovery/short-span-certificate.json"
    ))
    .unwrap();
    verify_certificate(&certificate, &computed).expect("intact certificate verifies");
    let tampers: Vec<(&str, serde_json::Value)> = vec![
        (
            "/constants/legal_chord_floor_m",
            serde_json::json!(0.000001),
        ),
        ("/constants/weld_max_turn_deg", serde_json::json!(179.0)),
        ("/constants/pieces_per_cubic_min", serde_json::json!(1.0)),
        ("/constants/f32_ulp_m", serde_json::json!(0.0001)),
        (
            "/claims/affected_config_hull_upper_bound_m",
            serde_json::json!(0.196977156),
        ),
        (
            "/claims/total_margin_upper_bound_m",
            serde_json::json!(0.0002114320238),
        ),
        (
            "/claims/chord_pair_lower_bound_m",
            serde_json::json!(0.19996905),
        ),
        ("/claims/gap_lower_bound_m", serde_json::json!(0.0027805)),
        (
            "/negative_control_criterion/counter_example/first_window_deviation_lower_bound_m",
            serde_json::json!(0.012008),
        ),
        (
            "/unaffected_forms/e6_s1_lower_bound_m",
            serde_json::json!(2.22),
        ),
    ];
    for (pointer, bogus) in tampers {
        let mut tampered = certificate.clone();
        *tampered.pointer_mut(pointer).expect("certificate pointer") = bogus;
        assert!(
            verify_certificate(&tampered, &computed).is_err(),
            "tamper {pointer} must be rejected"
        );
    }
    // 声明缺失同样必须失败。
    let mut deleted = certificate.clone();
    *deleted
        .pointer_mut("/constants/legal_chord_floor_m")
        .expect("certificate pointer") = serde_json::Value::Null;
    assert!(
        verify_certificate(&deleted, &computed).is_err(),
        "deleted claim must be rejected"
    );
}

#[test]
fn c17_cohort_and_capacities_match_the_frozen_sample() {
    assert_eq!(C17_EDGES.map(|(_, s, cap)| (s, cap)), [(2, 193), (3, 289)]);
    for (.., s, cap) in C17_EDGES {
        assert_eq!(96 * s + 1, cap);
    }
    assert_eq!(C17_PATHS.len(), 1);
    assert!(C17_COMPONENT.contains("-10564_3_0"));
    assert_eq!(PRIOR_CANDIDATE_EDGE, "sumo::-10564_3_0");
    assert_eq!(PRIOR_CANDIDATE_GEOMETRY, 80);
    assert_eq!(PRIOR_CANDIDATE_PIECES_PER_CUBIC, 22);
}

// ---------------------------------------------------------------------------
// S1/S2 新族构造与驱动接线（冻结方案 §2/§3；构造属使用方模块，判定走
// geom::s1::reshape::custom 共享驱动——判定口径零重定义）
// ---------------------------------------------------------------------------

use crate::output::geom::s1::reshape::custom::GeometrySpec;

/// 组构造：把原 span 区间 `[start, end]` 合并为单 cubic（柄 = 组端点弦长/3 × 因子，
/// 组首末切向取（已扰动）结点切向）。`groups` 为 `(起 span, 止 span)` 0 基闭区间。
fn grouped_cubics(
    knots: &[Vec3f],
    tangents: &[Vec3f],
    groups: &[(usize, usize)],
    factors: &[[f64; 2]],
) -> Vec<[[f64; 3]; 6]> {
    assert_eq!(groups.len(), factors.len());
    groups
        .iter()
        .zip(factors)
        .map(|(&(start, end), factor)| {
            let a = knots[start];
            let b = knots[end + 1];
            let handle = v_norm(v_sub(b, a)) / 3.0;
            [
                a,
                v_add(a, v_scale(tangents[start], handle * factor[0])),
                v_sub(b, v_scale(tangents[end + 1], handle * factor[1])),
                b,
                tangents[start],
                tangents[end + 1],
            ]
        })
        .collect()
}

/// 一条边的新族配置全集（冻结方案 §3 遍历序）：探索组先行（S1-pair-L、S2-pair-L），
/// 负对照组后行（S1-keep、S2-pair-R 照常走管线不预分类——证书预判不提前截断），
/// 旧族重叠 16 对角配置显式跳过零预算。返回（config_index → 新族元数据, 配置列表）。
fn edge_family_configs(
    knots: &[Vec3f],
    tangents: &[Vec3f],
    s: usize,
) -> (Vec<serde_json::Value>, Vec<GeometrySpec>) {
    let deltas: [f64; 5] = [-12.0, -6.0, 0.0, 6.0, 12.0];
    let domain: [f64; 4] = [0.75, 1.0, 1.25, 1.5];
    let mut metas: Vec<serde_json::Value> = Vec::new();
    let mut specs: Vec<GeometrySpec> = Vec::new();
    let s1_tangents = |knot: usize, delta: f64, base: &[Vec3f]| -> Vec<Vec3f> {
        let central = v_unit(v_sub(knots[knot + 1], knots[knot - 1]));
        let w = s1_rotation_plane(base[knot], central, base[knot - 1], base[knot + 1])
            .expect("rotation plane");
        let mut out = base.to_vec();
        out[knot] = rotate_tangent(base[knot], w, delta);
        out
    };
    match s {
        2 => {
            // edge3：S1-keep（δ1 扰动结点 1；5 配置）。
            let groups = [(0, 0), (1, 1)];
            let factors = [[1.0, 1.0], [1.0, 1.0]];
            for delta in deltas {
                let t = s1_tangents(1, delta, tangents);
                let config_index = metas.len();
                metas.push(serde_json::json!({"family":"s1-keep","deltas_deg":[delta]}));
                specs.push(GeometrySpec {
                    config_index,
                    group: "keep",
                    factors: factors.to_vec(),
                    cubics: grouped_cubics(knots, &t, &groups, &factors),
                    preclassified: None,
                });
            }
        }
        3 => {
            // 探索组先行。
            let pair_l_groups = [(0, 1), (2, 2)];
            let pair_l_factors = [[1.0, 1.0], [1.0, 1.0]];
            for delta in deltas {
                let t = s1_tangents(2, delta, tangents);
                let config_index = metas.len();
                metas.push(serde_json::json!({"family":"s1-pair-L","deltas_deg":[delta]}));
                specs.push(GeometrySpec {
                    config_index,
                    group: "pair-L",
                    factors: pair_l_factors.to_vec(),
                    cubics: grouped_cubics(knots, &t, &pair_l_groups, &pair_l_factors),
                    preclassified: None,
                });
            }
            for f1 in domain {
                for g1 in domain {
                    for f2 in domain {
                        for g2 in domain {
                            let factors = [[f1, g1], [f2, g2]];
                            let skip = f1 == f2 && g1 == g2;
                            let config_index = metas.len();
                            metas.push(serde_json::json!({"family":"s2-pair-L",
                                "group_factors":factors,"skipped_old_family_overlap":skip}));
                            specs.push(GeometrySpec {
                                config_index,
                                group: "pair-L",
                                factors: factors.to_vec(),
                                cubics: grouped_cubics(knots, tangents, &pair_l_groups, &factors),
                                preclassified: skip.then_some("skipped_old_family_overlap"),
                            });
                        }
                    }
                }
            }
            // 负对照组后行（照常走管线；预期零合法程序、最早拒绝阶段如实记录）。
            let keep_groups = [(0, 0), (1, 1), (2, 2)];
            let keep_factors = [[1.0, 1.0]; 3];
            for d1 in deltas {
                for d2 in deltas {
                    let mut t = s1_tangents(1, d1, tangents);
                    let central2 = v_unit(v_sub(knots[3], knots[1]));
                    let w2 = s1_rotation_plane(tangents[2], central2, tangents[1], tangents[3])
                        .expect("rotation plane");
                    t[2] = rotate_tangent(tangents[2], w2, d2);
                    let config_index = metas.len();
                    metas.push(serde_json::json!({"family":"s1-keep","deltas_deg":[d1, d2]}));
                    specs.push(GeometrySpec {
                        config_index,
                        group: "keep",
                        factors: keep_factors.to_vec(),
                        cubics: grouped_cubics(knots, &t, &keep_groups, &keep_factors),
                        preclassified: None,
                    });
                }
            }
            let pair_r_groups = [(0, 0), (1, 2)];
            for f1 in domain {
                for g1 in domain {
                    for f2 in domain {
                        for g2 in domain {
                            let factors = [[f1, g1], [f2, g2]];
                            let config_index = metas.len();
                            metas.push(serde_json::json!({"family":"s2-pair-R",
                                "group_factors":factors}));
                            specs.push(GeometrySpec {
                                config_index,
                                group: "pair-R",
                                factors: factors.to_vec(),
                                cubics: grouped_cubics(knots, tangents, &pair_r_groups, &factors),
                                preclassified: None,
                            });
                        }
                    }
                }
            }
        }
        other => panic!("unexpected reference cubic count {other}"),
    }
    assert_eq!(metas.len(), specs.len());
    (metas, specs)
}

// ---------------------------------------------------------------------------
// 新族驱动与 ABBA 入口（custom 共享驱动；构造在上节，判定口径零重定义）
// ---------------------------------------------------------------------------

/// JSON 规范化往返（与既有对照口径同：双侧同源序列化/解析）。
fn normalized(value: &Value) -> Value {
    serde_json::from_str(&value.to_string()).unwrap()
}

/// 池规模（边数、候选总数）。
fn pool_size(pools: &combinations::Pools) -> (usize, usize) {
    (pools.len(), pools.values().map(Vec::len).sum())
}

/// 冻结参考链的结点与切向（内部结点断言为 interior CR 中心差分，§S1 来源断言）。
fn frozen_knots_tangents(edge_index: usize, s: usize) -> (Vec<Vec3f>, Vec<Vec3f>) {
    let controls = &FROZEN_REFERENCE_CONTROLS[edge_index];
    let mut knots = vec![controls[0][0].map(f64::from_bits)];
    let mut tangents = vec![controls[0][4].map(f64::from_bits)];
    for control in controls.iter().take(s) {
        knots.push(control[3].map(f64::from_bits));
        tangents.push(control[5].map(f64::from_bits));
    }
    for k in 1..s {
        let central = v_unit(v_sub(knots[k + 1], knots[k - 1]));
        assert!(
            v_norm(v_sub(tangents[k], central)) < 1e-12,
            "knot {k} tangent is the interior CR central difference"
        );
    }
    (knots, tangents)
}

/// 原参考长度估计（4096 区间弦长 / 控制多边形；冻结方案 §4 清单）。
fn reference_length_estimates(edge_index: usize, s: usize) -> serde_json::Value {
    let controls = &FROZEN_REFERENCE_CONTROLS[edge_index];
    let mut chord = 0.0_f64;
    let mut polygon = 0.0_f64;
    for control in controls.iter().take(s) {
        let cubic = [0, 1, 2, 3].map(|k| control[k].map(f64::from_bits));
        for k in 0..4096_usize {
            let a = k as f64 / 4096.0;
            let b = (k + 1) as f64 / 4096.0;
            let part = split_range(cubic, a, b);
            chord += v_norm(v_sub(part[3], part[0]));
            polygon += (0..3)
                .map(|j| v_norm(v_sub(part[j + 1], part[j])))
                .sum::<f64>();
        }
    }
    serde_json::json!({"chord_estimate_m":chord,"control_polygon_estimate_m":polygon,
        "intervals_per_cubic":4096})
}

/// de Casteljau 区间截取（与共享机器 `restricted` 同口径）。
fn split_range(c: [[f64; 3]; 4], a: f64, b: f64) -> [[f64; 3]; 4] {
    let split = |c: [[f64; 3]; 4], t: f64| {
        let lerp = |p: Vec3f, q: Vec3f| v_add(p, v_scale(v_sub(q, p), t));
        let p01 = lerp(c[0], c[1]);
        let p12 = lerp(c[1], c[2]);
        let p23 = lerp(c[2], c[3]);
        let p012 = lerp(p01, p12);
        let p123 = lerp(p12, p23);
        (
            [c[0], p01, p012, lerp(p012, p123)],
            [lerp(p012, p123), p123, p23, c[3]],
        )
    };
    let left = if b == 1.0 { c } else { split(c, b).0 };
    if a == 0.0 { left } else { split(left, a / b).1 }
}

/// 两边缘 S1/S2 新族搜索（冻结计划 §3/§4；每边独立三分量预算共享先到即停）。
fn run_c17_new_family(inputs: &Value) -> (serde_json::Value, serde_json::Value) {
    let mut searches = BTreeMap::new();
    let mut timing = BTreeMap::new();
    for (edge_index, (id, s, cap)) in C17_EDGES.into_iter().enumerate() {
        assert_eq!(96 * s + 1, cap, "frozen temp program point cap");
        let input = measurement::input(&inputs[id]);
        let (knots, tangents) = frozen_knots_tangents(edge_index, s);
        let (config_table, specs) = edge_family_configs(&knots, &tangents, s);
        let started = Instant::now();
        let mut budget = crate::output::geom::s1::grid::WorkBudget::per_edge_default();
        let (rows, retained) = crate::output::geom::s1::reshape::custom::search_custom_geometries(
            &input,
            s,
            &specs,
            &mut budget,
        );
        timing.insert(id, started.elapsed().as_secs_f64());
        let json_rows = rows
            .iter()
            .map(|row| {
                serde_json::json!({"geometry":row.geometry_index,"description":row.description,
                    "status":row.status,"partition_rejections":row.partition_rejections,
                    "successful_partitions":row.successful_partitions,
                    "source_fit":row.source_fit,"controls_f64_bits":row.controls_f64_bits,
                    "arc_m":row.arc_m,"source_length_estimate":row.source_length_estimate})
            })
            .collect::<Vec<_>>();
        let json_candidates = retained
            .iter()
            .map(|entry| {
                serde_json::json!({"program":program_bits(&entry.candidate.repaired.program),
                    "arc_m":entry.candidate.repaired.quantized_arc_meters,
                    "spans":entry.candidate.spans,"score":entry.score,"order":entry.order,
                    "curves_f64_bits":entry.curves.iter().map(|c| c.map(|p| p.map(f64::to_bits))).collect::<Vec<_>>()})
            })
            .collect::<Vec<_>>();
        searches.insert(
            id,
            serde_json::json!({"config_table":config_table,"geometries":json_rows,
                "candidates":json_candidates,
                "frozen_segment_capacity":{"reference_cubics_s":s,"temp_program_points_cap":cap},
                "budget":{"partitions":budget.partitions_consumed,
                    "sampling_work":budget.grid_work_consumed,
                    "source_fit_evals":budget.source_fit_consumed}}),
        );
    }
    (
        serde_json::to_value(searches).unwrap(),
        serde_json::to_value(timing).unwrap(),
    )
}

/// 新族候选 → 边池（`frozen_candidate` 复用既有候选形态；`order` 保留秩序）。
fn pools_from_c17_searches(searches: &Value) -> combinations::Pools {
    let mut pools = combinations::Pools::new();
    for (id, row) in searches.as_object().unwrap() {
        let candidates = row["candidates"].as_array().unwrap();
        if !candidates.is_empty() {
            pools.insert(
                id.clone(),
                candidates
                    .iter()
                    .map(|c| successors::frozen_candidate(c).repaired.program)
                    .collect(),
            );
        }
    }
    pools
}

/// C17 双边联合验收记录（冻结方案 §2.J：池字典序枚举、接缝全角只作记录字段）。
fn c17_joint_record(rows: &[Value], pools: &combinations::Pools, missing_edges: &[&str]) -> Value {
    let row = rows
        .iter()
        .find(|row| row["component"] == C17_COMPONENT)
        .expect("C17 row");
    let result = &row["result"];
    let total = result["total_assignments"].as_u64();
    let trials = result["trials"].as_array().map(Vec::len).unwrap_or(0) as u64;
    // 逐赋值接缝全角（记录字段，不参与判定）：edge3 末弦与 edge6 首弦全角。
    let seam_trials = result["trials"]
        .as_array()
        .map(|trials| {
            trials
                .iter()
                .map(|trial| {
                    let idx3 = trial["assignment"]["sumo::-10564_3_0"].as_u64().unwrap() as usize;
                    let idx6 = trial["assignment"]["sumo::-10564_6_0"].as_u64().unwrap() as usize;
                    let pts3 = program_points(&pools["sumo::-10564_3_0"][idx3]);
                    let pts6 = program_points(&pools["sumo::-10564_6_0"][idx6]);
                    let last = pts3.len();
                    let chord3 = v_sub(pts3[last - 1], pts3[last - 2]);
                    let chord6 = v_sub(pts6[1], pts6[0]);
                    serde_json::json!({"trial":trial["trial"],
                        "seam_turn_deg":v_dot(v_unit(chord3), v_unit(chord6)).acos().to_degrees()})
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serde_json::json!({"component":C17_COMPONENT,"complete_paths":C17_PATHS,
        "missing_edges":missing_edges,"status":result["status"],
        "total_assignments":total,"tried_assignments":trials,
        "unvisited_assignments":total.map(|total| total - trials),
        "assignment_budget_stopped":result["assignment_budget_stopped"],
        "retained_pool_assignments_exhausted":result["retained_pool_assignments_exhausted"],
        "selected_assignment":result["selected_assignment"],
        "seam_turn_records":seam_trials})
}

/// 候选冻结入口（先跑；`c17-searches.json` 作 `CURVE_DESIGN_FROZEN_RESULT`）。
#[test]
#[ignore = "requires fresh CURVE_DESIGN_OUTPUT_DIR"]
fn fixed_c17_bilateral_design() {
    let out = PathBuf::from(std::env::var("CURVE_DESIGN_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh c17 curve design output");
    fs::create_dir_all(&out).unwrap();
    let (searches, timing) = run_c17_new_family(&frozen_calibration_inputs());
    recovery::write_json(&out, "c17-searches.json", &searches);
    recovery::write_json(&out, "c17-timing.json", &timing);
}

/// 旧族 148 配置对照列（冻结方案 §2：只作对照，不作候选点位来源）。
fn run_old_family_column(inputs: &Value) -> serde_json::Value {
    let mut out = BTreeMap::new();
    for (id, s, _) in C17_EDGES {
        let input = measurement::input(&inputs[id]);
        let search = sampler::reshape::search_with_curve_cap(&input, s);
        out.insert(id, search.report);
    }
    serde_json::to_value(out).unwrap()
}

/// 在该分量选中赋值下逐条完整路径真实编译（C17 单路径；齐备才可能接受）。
fn complete_path_checks(
    traffic: &TrafficPackage,
    originals: &BTreeMap<String, re::RoadEditingCurveProgram>,
    pools: &combinations::Pools,
    result: &Value,
) -> Value {
    let mut programs = originals.clone();
    if result["status"] == "accepted" {
        for (id, choice) in result["selected_assignment"].as_object().unwrap() {
            programs.insert(
                id.clone(),
                pools[id][choice.as_u64().unwrap() as usize].clone(),
            );
        }
    }
    C17_PATHS
        .iter()
        .map(|id| {
            let path = traffic
                .maneuver_paths
                .iter()
                .find(|path| path.id == *id)
                .expect("complete path identity");
            serde_json::json!({"path":path.id,"result":compile_path(traffic, path, &programs)})
        })
        .collect()
}

/// 冻结方案 §4 长度与拟合报告（未测项如实 null）。
fn c17_length_report(
    traffic: &TrafficPackage,
    inputs: &Value,
    old_column: &Value,
    searches: &Value,
) -> Value {
    C17_EDGES
        .iter()
        .enumerate()
        .map(|(edge_index, (id, s, _))| {
            let source = traffic
                .lane_graph
                .edges
                .iter()
                .find(|edge| edge.id == *id)
                .expect("c17 edge in lane graph");
            let points = measurement::input(&inputs[id]).points;
            let shape: f64 = points.windows(2).map(|w| v_norm(v_sub(w[1], w[0]))).sum();
            let old = &old_column[id];
            let candidates = searches[id]["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .map(|candidate| {
                    serde_json::json!({
                        "candidate_index":candidate["order"],
                        "arc_m":candidate["arc_m"],
                        "new_reference_chord_estimate_m":
                            (candidate["curves_f64_bits"].as_array().unwrap().len() as f64),
                        "source_fit":candidate["spans"][0]["source_fit"],
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({"edge":id,
                "source_at_length_m":source.length,
                "source_shape_polyline_m":shape,
                "old_reference":reference_length_estimates(edge_index, *s),
                "old_reference_source_fit":old["reference_fit"],
                "retained_candidates":candidates})
        })
        .collect()
}

/// 流 B 路正式实验（四轮 ABBA；`c17-*` 前缀输出、结果 JSON 禁计时、生成物外置）。
#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, DIRECT_SUCCESSOR_BASELINE_DIR, SIX_STREAMS_BASELINE_DIR, CURVE_DESIGN_FROZEN_RESULT, CURVE_DESIGN_ORDER and fresh CURVE_DESIGN_OUTPUT_DIR"]
fn pinned_lust_c17_bilateral_experiment() {
    let out = PathBuf::from(std::env::var("CURVE_DESIGN_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh full c17 curve design output");
    fs::create_dir_all(&out).unwrap();
    let order = std::env::var("CURVE_DESIGN_ORDER").unwrap();
    assert!(matches!(order.as_str(), "baseline-new" | "new-baseline"));
    let frozen: Value = serde_json::from_slice(
        &fs::read(std::env::var("CURVE_DESIGN_FROZEN_RESULT").unwrap()).unwrap(),
    )
    .unwrap();
    // 共同基线（E0 冻结交付）与结果对照锚（仅对照，不作候选点位来源）。
    let baseline = calibration::shared_baseline::shared_pools();
    assert_eq!(
        pool_size(&baseline),
        (11, 92),
        "frozen 11 edges 92 candidates"
    );
    let anchor = calibration::shared_baseline::results_anchor_pinned();
    assert_eq!(anchor["components"].as_array().unwrap().len(), 53);
    assert_eq!(anchor["paths"].as_array().unwrap().len(), 84);
    assert_eq!(anchor["path_counts"]["accepted"], 28, "frozen 28/84 anchor");
    let (observation, source, mut timing) = capture_lust();
    assert_eq!(
        source["lust_commit"],
        serde_json::json!("c4bd5bd3751d426d42a9a1749c815e47ea188549"),
        "pinned LuST source"
    );
    let originals = original_programs(&observation);
    snapshot(&out, &observation, &originals);
    let traffic = observation.traffic.as_ref().unwrap();
    let components = components::inventory(traffic, &originals, &target_ids(&reference()));
    assert_eq!(components.len(), 53, "frozen component count");
    assert_eq!(
        components.iter().map(|c| c.paths.len()).sum::<usize>(),
        183,
        "frozen component path total"
    );
    let c17 = components
        .iter()
        .find(|c| c.id == C17_COMPONENT)
        .expect("C17 component");
    assert_eq!(c17.paths, C17_PATHS, "C17 complete paths");
    let missing: Vec<&str> = c17.missing_edges.iter().map(String::as_str).collect();
    assert_eq!(
        missing,
        vec!["sumo::-10564_3_0", "sumo::-10564_6_0"],
        "C17 missing edges"
    );
    measurement::write_inputs(
        &out,
        &observation,
        &serde_json::json!({"samples":C17_EDGES.map(|(id, _, _)| serde_json::json!({"edge":id}))}),
    );
    let inputs: Value =
        serde_json::from_slice(&fs::read(out.join("inputs.json")).unwrap()).unwrap();
    let frozen_inputs = frozen_calibration_inputs();
    for (id, ..) in C17_EDGES {
        assert_eq!(
            inputs[id], frozen_inputs[id],
            "unchanged source and emission failure"
        );
        assert!(!originals.contains_key(id), "only failed edges redesigned");
    }
    // 新族搜索（冻结重放）+ 旧族对照列。
    let (searches, search_times) = run_c17_new_family(&inputs);
    assert_eq!(normalized(&searches), frozen, "frozen c17 candidate replay");
    recovery::write_json(&out, "c17-searches.json", &searches);
    let old_column = run_old_family_column(&inputs);
    recovery::write_json(&out, "c17-old-family-column.json", &old_column);
    // 池合成：基线（含既有候选 #1131 几何 80 于 edge3 池索引 0）+ 新族保留。
    assert_eq!(
        baseline["sumo::-10564_3_0"].len(),
        1,
        "prior candidate sits in baseline pool"
    );
    let mut combined = baseline.clone();
    experiment::merge(&mut combined, pools_from_c17_searches(&searches));
    // 基线/新增两组验证（ABBA 顺序；C17 记录 + 完整路径逐条）。
    let mut results = BTreeMap::new();
    let mut validation_times = BTreeMap::new();
    let mut joint = BTreeMap::new();
    for family in order.split('-') {
        let pools = if family == "baseline" {
            &baseline
        } else {
            &combined
        };
        let started = Instant::now();
        let mut programs = originals.clone();
        for (id, choice) in reference()["tiers"]["32768"]["best"]["assignment"]
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
            let file = format!("{family}-component-{index:02}.lfca");
            let mut result = combinations::evaluate(
                traffic,
                component,
                &originals,
                pools,
                Some(&out.join(&file)),
            );
            // 对照锚字段形态：`unvisited_assignments` 一律增强（无赋值时为 null）
            // 再逐值比较（c12 同款）。
            let unvisited = result
                .get("total_assignments")
                .and_then(Value::as_u64)
                .map(|total| total - result["trials"].as_array().unwrap().len() as u64);
            result["unvisited_assignments"] = serde_json::json!(unvisited);
            if result["status"] == "accepted" {
                for (id, choice) in result["selected_assignment"].as_object().unwrap() {
                    programs.insert(
                        id.clone(),
                        pools[id][choice.as_u64().unwrap() as usize].clone(),
                    );
                }
            }
            rows.push(serde_json::json!({"component":component.id,"result":result,
                "artifact":(result["status"] == "accepted").then_some(file)}));
        }
        let paths = inventory_paths(traffic, &inventory_rows(), &programs);
        let mut counts = BTreeMap::<String, usize>::new();
        for row in &paths {
            *counts
                .entry(row["result"]["status"].as_str().unwrap().to_owned())
                .or_default() += 1;
        }
        let c17_result = &rows
            .iter()
            .find(|row| row["component"] == C17_COMPONENT)
            .expect("C17 row")["result"];
        joint.insert(
            family,
            serde_json::json!({
                "record":c17_joint_record(&rows, pools, &missing),
                "complete_path_checks":complete_path_checks(traffic, &originals, pools, c17_result)}),
        );
        results.insert(
            family,
            serde_json::json!({"components":rows,"paths":paths,"path_counts":counts}),
        );
        validation_times.insert(family, started.elapsed().as_secs_f64());
    }
    // 结果对照（仅对照）：基线变体与重锚逐值一致（含 unvisited_assignments 字段形态）。
    let base = &results["baseline"];
    let anchor_components = anchor["components"].as_array().unwrap();
    let base_components = base["components"].as_array().unwrap();
    assert_eq!(base_components.len(), anchor_components.len());
    for (current, previous) in base_components.iter().zip(anchor_components) {
        assert_eq!(current["component"], previous["component"]);
        assert_eq!(
            normalized(&current["result"]),
            previous["result"],
            "baseline component equals 92-pool anchor"
        );
    }
    assert_eq!(
        normalized(&base["paths"]),
        anchor["paths"],
        "baseline paths equal anchor"
    );
    assert_eq!(
        normalized(&base["path_counts"]),
        anchor["path_counts"],
        "baseline counts equal anchor"
    );
    assert_eq!(
        base["path_counts"]["accepted"].as_u64(),
        Some(28),
        "frozen accepted total 28/84"
    );
    // 两口径收益（章程 §1 分列）与新增收益。
    let new_counts = results["new"]["path_counts"].clone();
    let gains = serde_json::json!({
        "added_candidates":pool_size(&combined).1 - pool_size(&baseline).1,
        "complete_paths_scope":1,
        "original_targets_scope":1,
        "path_counts_new":new_counts,
    });
    let mut policy = sampler::reshape::policy();
    policy["reference_cubics_max_per_edge"] = serde_json::json!(
        C17_EDGES
            .iter()
            .map(
                |(id, s, cap)| serde_json::json!({"edge":id,"reference_cubics_s":s,
            "temp_program_points_cap":cap})
            )
            .collect::<Vec<_>>()
    );
    timing["search_seconds"] = search_times;
    timing["old_family_column_seconds"] = serde_json::Value::Null;
    timing["validation_seconds"] = serde_json::to_value(validation_times).unwrap();
    timing["order"] = serde_json::json!(order);
    let _ = policy;
    recovery::write_json(
        &out,
        "c17-results.json",
        &serde_json::json!({"schema":"laneflow-1137-c17-bilateral-recovery",
        "source":source,"variants":results,
        "c17":{"joint":joint,
            "length_report":c17_length_report(traffic, &inputs, &old_column, &searches),
            "new_pool":{"edges":pool_size(&combined).0,"candidates":pool_size(&combined).1},
            "gains":gains},
        "result_comparison":{"baseline_equals_anchor_92_pool":true,
            "anchor_role":"comparison only; not a candidate source",
            "accepted_paths_baseline":"28/84"},
        "original_geometry_controls_and_failures_equal":true,
        "production_g1_accepted":false}),
    );
    recovery::write_json(&out, "c17-timing.json", &timing);
}

/// 84 目标路径逐条编译（与锚路径行同形）。
fn inventory_paths(
    traffic: &TrafficPackage,
    rows: &[Value],
    programs: &BTreeMap<String, re::RoadEditingCurveProgram>,
) -> Vec<Value> {
    rows.iter()
        .map(|row| {
            let path = traffic
                .maneuver_paths
                .iter()
                .find(|p| p.id == row["path"].as_str().unwrap())
                .unwrap();
            serde_json::json!({"path":path.id,"edges":path_edges(path),
                "result":compile_path(traffic, path, programs)})
        })
        .collect()
}

/// 84 目标行（issue-1105 `baseline_paths` 冻结清单，与 `target_ids` 同源）。
fn inventory_rows() -> Vec<Value> {
    reference()["baseline_paths"].as_array().unwrap().clone()
}

#[test]
fn c17_family_config_counts_match_the_frozen_plan() {
    // edge3：S1-keep 5；edge6：S1-pair-L 5 + S2-pair-L 256（16 对角跳过）+
    // S1-keep 25 + S2-pair-R 256 = 542 列明。
    let (knots3, tangents3) = frozen_knots_tangents(0, 2);
    let (metas3, specs3) = edge_family_configs(&knots3, &tangents3, 2);
    assert_eq!(specs3.len(), 5);
    assert_eq!(metas3.len(), 5);
    let (knots6, tangents6) = frozen_knots_tangents(1, 3);
    let (metas6, specs6) = edge_family_configs(&knots6, &tangents6, 3);
    assert_eq!(metas6.len(), specs6.len());
    assert_eq!(specs6.len(), 542);
    let skipped = specs6
        .iter()
        .filter(|spec| spec.preclassified == Some("skipped_old_family_overlap"))
        .count();
    assert_eq!(skipped, 16, "old-family overlap diagonals skipped");
    // 组序（冻结方案 §3）：探索组先行（s1-pair-L、s2-pair-L），负对照组后行。
    assert_eq!(specs6[0].group, "pair-L");
    assert_eq!(specs6[5].group, "pair-L");
    assert_eq!(specs6[261].group, "keep");
    assert_eq!(specs6[286].group, "pair-R");
    // δ=0 的 S1-keep 配置与未扰动 keep 构造逐位一致。
    let base_keep = grouped_cubics(
        &knots6,
        &tangents6,
        &[(0, 0), (1, 1), (2, 2)],
        &[[1.0, 1.0]; 3],
    );
    let zero_pos = metas6
        .iter()
        .position(|meta| {
            meta["family"] == "s1-keep" && meta["deltas_deg"] == serde_json::json!([0.0, 0.0])
        })
        .expect("zero-delta s1-keep config");
    let zero = &specs6[zero_pos];
    for (a, b) in zero.cubics.iter().zip(&base_keep) {
        for i in 0..6 {
            assert_eq!(
                a[i].map(f64::to_bits),
                b[i].map(f64::to_bits),
                "delta=0 bitwise"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// K1–K4 正例校准（冻结方案 §6；阶段 2 第一闸，全绿后才启动正式 ABBA）
// ---------------------------------------------------------------------------

use super::*;

/// 冻结输入（#1107 钉版捕获；含 C17 两边缘与 #1115 两正例边的完整输入——
/// 点位/钳制/钳制来源；K1「绑定完整输入」即取材于此）。
fn frozen_calibration_inputs() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/inputs.json"
    ))
    .unwrap()
}

/// K2 冻结对照（#1131 旧族逐配置记录摘录；判定口径防漂移）。
fn k2_frozen_records() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1137-c17-bilateral-recovery/k2-old-family-frozen.json"
    ))
    .unwrap()
}

/// 用完整冻结输入跑旧族搜索（判定链：构造→回折→双向拟合→划分→sink/端弦/
/// 0.01 m→去重），返回（几何行, 候选）。
fn replay_old_family(
    input: &observe::EdgeInput,
    s: usize,
) -> (serde_json::Value, serde_json::Value) {
    let search = sampler::reshape::search_with_curve_cap(input, s);
    (
        search.report["geometries"].clone(),
        recovery::candidates(&search),
    )
}

/// 候选投影：只保留判定链的离散身份（程序位、几何/划分/片数、控制与参数位）与
/// 微米级弧长；浮点拟合/长度观察不参与投影（防序列化往返噪声，判定口径逐值由
/// 几何行的整数/字符串字段精确核对）。
fn candidate_projection(candidates: &serde_json::Value) -> serde_json::Value {
    serde_json::Value::Array(
        candidates
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                let span = &c["spans"][0];
                serde_json::json!({
                    "program": c["program"],
                    "arc_micrometers": (c["arc_m"].as_f64().unwrap() * 1e6).round() as i64,
                    "geometry": span["geometry"],
                    "partition": span["partition"],
                    "pieces_per_cubic": span["pieces_per_cubic"],
                    "control_f64_bits": span["control_f64_bits"],
                    "parameter_f64_bits": span["parameter_f64_bits"],
                })
            })
            .collect(),
    )
}

/// K1 已知候选全链重放（绑定完整输入）：#1131 几何 80（`-10564_3_0`）与 #1115
/// 两正例边（`-16502_0_0` 1 份、`-28860_3_0` 2 份）经判定链必须复现接受状态。
#[test]
fn k1_known_candidates_replay_with_frozen_inputs() {
    let inputs = frozen_calibration_inputs();
    // #1131 几何 80（pair 合段、因子 [1.5, 1]、等 t 22 片）。
    let (rows, candidates) = replay_old_family(&measurement::input(&inputs["sumo::-10564_3_0"]), 2);
    assert_eq!(
        candidates.as_array().unwrap().len(),
        1,
        "prior candidate reproduced"
    );
    let span = &candidates[0]["spans"][0];
    assert_eq!(span["geometry"], serde_json::json!(80));
    assert_eq!(span["partition"], serde_json::json!("uniform"));
    assert_eq!(span["pieces_per_cubic"], serde_json::json!(22));
    let frozen = k2_frozen_records();
    assert_eq!(
        candidate_projection(&candidates),
        candidate_projection(&frozen["edges"]["sumo::-10564_3_0"]["candidates"]),
        "prior candidate identity"
    );
    assert_eq!(rows.as_array().unwrap().len(), 148);
    // #1115 两正例：候选与入库冻结记录逐值一致（同 #1115 冻结重放口径）。
    let frozen_1115: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1115-failed-curve-design/searches.json"
    ))
    .unwrap();
    for (id, s) in [("sumo::-16502_0_0", 4), ("sumo::-28860_3_0", 6)] {
        let (_, replayed) = replay_old_family(&measurement::input(&inputs[id]), s);
        let expected = &frozen_1115[id]["candidates"];
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&replayed.to_string()).unwrap(),
            serde_json::from_str::<serde_json::Value>(&expected.to_string()).unwrap(),
            "known positive replay {id}"
        );
    }
}

/// K2 旧族重放逐值一致（防判定器漂移）：C17 两边缘 148 配置的逐配置状态、
/// 拒绝计数、成功分区与候选身份与 #1131 冻结记录逐值一致。
#[test]
fn k2_old_family_replay_matches_frozen_records() {
    let inputs = frozen_calibration_inputs();
    let frozen = k2_frozen_records();
    for (id, s) in [("sumo::-10564_3_0", 2), ("sumo::-10564_6_0", 3)] {
        let (rows, candidates) = replay_old_family(&measurement::input(&inputs[id]), s);
        let expected = &frozen["edges"][id];
        let rows = rows.as_array().unwrap();
        let expected_rows = expected["geometries"].as_array().unwrap();
        assert_eq!(rows.len(), expected_rows.len(), "geometry row count {id}");
        for (row, want) in rows.iter().zip(expected_rows) {
            for key in [
                "geometry",
                "description",
                "status",
                "partition_rejections",
                "successful_partitions",
            ] {
                assert_eq!(row[key], want[key], "{id} geometry row {key}");
            }
        }
        assert_eq!(
            candidate_projection(&candidates),
            candidate_projection(&expected["candidates"]),
            "candidate identity {id}"
        );
    }
}

// ---------------------------------------------------------------------------
// S1 切向扰动构造（冻结方案 §S1；构造属使用方模块，不属共享机器）
// ---------------------------------------------------------------------------

type Vec3f = [f64; 3];

fn v_sub(a: Vec3f, b: Vec3f) -> Vec3f {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn v_add(a: Vec3f, b: Vec3f) -> Vec3f {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn v_scale(a: Vec3f, k: f64) -> Vec3f {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn v_dot(a: Vec3f, b: Vec3f) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn v_norm(a: Vec3f) -> f64 {
    v_dot(a, a).sqrt()
}
fn v_unit(a: Vec3f) -> Vec3f {
    v_scale(a, 1.0 / v_norm(a))
}

/// 单位向量 `d` 对 `t` 的正交分量；模 < 1e-9 视为退化。
fn orthogonal_component(d: Vec3f, t: Vec3f) -> Option<Vec3f> {
    let w = v_sub(d, v_scale(t, v_dot(d, t)));
    (v_norm(w) >= 1e-9).then(|| v_unit(w))
}

/// S1 旋转平面（冻结方案 §S1 三路回退）：中央弦正交分量（CR 结点必然退化）→
/// 前一弦正交分量 → 后一弦正交分量；全退化返回 `None`（δ 强制 0 并记
/// `tangent_rotation_degenerate`）。
fn s1_rotation_plane(
    t: Vec3f,
    central_chord: Vec3f,
    previous_chord: Vec3f,
    next_chord: Vec3f,
) -> Option<Vec3f> {
    orthogonal_component(v_unit(central_chord), t)
        .or_else(|| orthogonal_component(v_unit(previous_chord), t))
        .or_else(|| orthogonal_component(v_unit(next_chord), t))
}

/// 切向旋转 t' = t·cos δ + w·sin δ（w 已单位化；δ=0 时与 t 逐位一致）。
fn rotate_tangent(t: Vec3f, w: Vec3f, delta_deg: f64) -> Vec3f {
    let radians = delta_deg.to_radians();
    v_add(v_scale(t, radians.cos()), v_scale(w, radians.sin()))
}

/// keep 分段的 S1 控制点构造：柄 = 各 cubic 自身弦长 / 3（不乘因子），
/// 两端切向取（已扰动）结点切向。
fn s1_keep_controls(knots: &[Vec3f], tangents: &[Vec3f]) -> Vec<[Vec3f; 6]> {
    assert_eq!(tangents.len(), knots.len());
    knots
        .windows(2)
        .enumerate()
        .map(|(i, w)| {
            let chord = v_norm(v_sub(w[1], w[0])) / 3.0;
            [
                w[0],
                v_add(w[0], v_scale(tangents[i], chord)),
                v_sub(w[1], v_scale(tangents[i + 1], chord)),
                w[1],
                tangents[i],
                tangents[i + 1],
            ]
        })
        .collect()
}

/// K3 边界自检：旋转平面三路回退与全退化分支、δ=0 逐位对照、s=2 分组退化。
#[test]
fn k3_tangent_rotation_boundary_self_checks() {
    // CR 中心差分切向与中央弦同向 ⇒ 第一层正交分量必然退化。
    let t = v_unit([1.0, 0.0, 0.0]);
    let central = v_unit([2.0, 0.0, 0.0]);
    assert!(orthogonal_component(central, t).is_none());
    // 前一弦正交分量为正常路径；全共线时三路全退化 → None。
    let previous = v_unit([1.0, 1.0, 0.0]);
    assert!(s1_rotation_plane(t, central, previous, central).is_some());
    assert!(s1_rotation_plane(t, central, central, central).is_none());
    // δ=0 逐位对照：与未扰动构造逐位一致。
    let knots = [[0.0, 0.0, 0.0], [1.0, 0.2, 0.0], [2.0, 0.5, 0.0]];
    let tangents = [
        v_unit([1.0, 0.1, 0.0]),
        v_unit([1.0, 0.2, 0.0]),
        v_unit([1.0, 0.3, 0.0]),
    ];
    let w = s1_rotation_plane(
        tangents[1],
        v_unit([1.0, 0.3, 0.0]),
        v_unit([1.0, 0.2, 0.0]),
        v_unit([1.0, 0.3, 0.0]),
    )
    .expect("rotation plane");
    let rotated_zero = [
        tangents[0],
        rotate_tangent(tangents[1], w, 0.0),
        tangents[2],
    ];
    let unrotated = s1_keep_controls(&knots, &tangents);
    let zeroed = s1_keep_controls(&knots, &rotated_zero);
    for (a, b) in unrotated.iter().zip(&zeroed) {
        for i in 0..6 {
            assert_eq!(
                a[i].map(f64::to_bits),
                b[i].map(f64::to_bits),
                "delta=0 bitwise"
            );
        }
    }
    // s=2 分组退化：pair/whole 均为单组合并（#1131 已证），组数断言。
    let group_count = |mode: &str, s: usize| match mode {
        "keep" => s,
        "pair" => s.div_ceil(2),
        "whole" => 1,
        other => panic!("unknown mode {other}"),
    };
    assert_eq!(group_count("pair", 2), group_count("whole", 2));
}

// ---------------------------------------------------------------------------
// K4 合成正反例（冻结方案 §6 K4；特定标签用例钉完整输入与报告字段，
// R2/R3 的判定器直调入口待 E 公共扩展落地，现以全链报告字段交叉断言）
// ---------------------------------------------------------------------------

fn synthetic_input(points: Vec<Vec3f>) -> observe::EdgeInput {
    observe::EdgeInput {
        points,
        clamps: [None, None],
        clamp_sources: [None, None],
    }
}

/// 旧族报告中指定分组（keep/reference 为短首段证书覆盖的形态）的行。
fn rows_of_group<'a>(rows: &'a [serde_json::Value], group: &str) -> Vec<&'a serde_json::Value> {
    rows.iter()
        .filter(|row| row["description"]["group"] == group)
        .collect()
}

#[test]
fn k4_synthetic_positive_and_negative_cases() {
    // P1 预期通过：三点共线直线链（0.5 m/段）经全链筛选必有完整程序。
    let straight = synthetic_input(vec![[0.0, 0.0, 0.0], [0.5, 0.0, 0.0], [1.0, 0.0, 0.0]]);
    let (rows, candidates) = replay_old_family(&straight, 2);
    assert!(
        !candidates.as_array().unwrap().is_empty(),
        "P1 straight chain accepted"
    );
    assert_eq!(rows.as_array().unwrap().len(), 148);
    // R1 预期拒绝（短首段复刻，证书覆盖形态）：keep/reference 行零合法程序、
    // 采样偏离 0（该形态首 cubic 只可能死在 sink 层，与证书一致）。
    let short_span = synthetic_input(vec![
        [0.0, 0.0, 0.0],
        [0.196_977_156_035_898_77, 0.0, 0.0],
        [2.5, 0.0, 0.0],
    ]);
    let (rows, candidates) = replay_old_family(&short_span, 2);
    let keep_rows = rows_of_group(rows.as_array().unwrap(), "keep")
        .into_iter()
        .chain(rows_of_group(rows.as_array().unwrap(), "reference"))
        .collect::<Vec<_>>();
    assert!(!keep_rows.is_empty(), "R1 keep/reference rows present");
    for row in keep_rows {
        // 冻结口径：零合法程序、最早拒绝阶段如实记录，不钉特定标签。
        assert!(
            row["successful_partitions"].as_array().unwrap().is_empty(),
            "R1 zero legal programs in certified scope"
        );
        assert_ne!(row["status"], "candidate_found", "R1 no accepted status");
        let rejections = &row["partition_rejections"];
        if !rejections.as_object().unwrap().is_empty() {
            // 到达划分层的配置与证书反例锚交叉一致：偏离检查先过、拒于 sink
            // （因子 ≠1 的配置可能先被双向拟合的弦近似误差项拒绝，如实记录）。
            assert!(
                rejections["sampling_deviation"].as_u64().unwrap_or(0) == 0
                    && rejections["sink"].as_u64().unwrap_or(0) > 0,
                "R1 reached-layer rows reject at sink; row={:?}",
                row
            );
        }
    }
    // 合并形态（pair/whole）如实记录各自阶段（不受证书覆盖，不预设）。
    let _ = candidates;
    // R2 预期拒绝（weld_turn 口径；直调入口待 E 扩展，交叉断言 sink 路径到达）：
    // 半径 3 m、圆心角 20° 合成弧——窗弦转角 L/r 超 1.95° 的 n 区间存在。
    let arc = synthetic_input(vec![
        [0.0, 0.0, 0.0],
        [
            3.0 * (10.0_f64.to_radians()).sin(),
            0.0,
            3.0 * (1.0 - (10.0_f64.to_radians()).cos()),
        ],
        [
            3.0 * (20.0_f64.to_radians()).sin(),
            0.0,
            3.0 * (1.0 - (20.0_f64.to_radians()).cos()),
        ],
    ]);
    let (rows, _) = replay_old_family(&arc, 2);
    let sink_seen = rows
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["partition_rejections"]["sink"].as_u64().unwrap_or(0) > 0);
    assert!(sink_seen, "R2 sink path reached (weld_turn expected mid-n)");
    // R3 预期拒绝（degenerate_chord 口径）：相邻点距 0.09 m 合成折线，量化弦
    // 必 ≤ 0.1 m → sink（退化）拒绝；交叉断言 sink 路径到达。
    let short_chords = synthetic_input(vec![[0.0, 0.0, 0.0], [0.09, 0.0, 0.0], [0.18, 0.0, 0.0]]);
    let (rows, _) = replay_old_family(&short_chords, 2);
    let sink_seen = rows
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["partition_rejections"]["sink"].as_u64().unwrap_or(0) > 0);
    assert!(sink_seen, "R3 sink path reached (degenerate expected)");
    // R4 预期拒绝（回折）：L 形输入下回折配置被 backtracking_control_rejected。
    // 90° 折角经修复生成倒圆弧（line+arc+line，s=3）。
    let zigzag = synthetic_input(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]]);
    let (rows, _) = replay_old_family(&zigzag, 3);
    let fold_rows = rows
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["status"] == "backtracking_control_rejected")
        .count();
    assert!(fold_rows > 0, "R4 backtracking rejection recorded");
    // R5 预期拒绝（source_fit）：浅弧源（矢高 0.1 m 量级）下外摆整边合段被
    // 双向来源拟合筛选拒绝（判定在划分前）。
    let shallow_arc = synthetic_input(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.1], [2.0, 0.0, 0.0]]);
    let (rows, _) = replay_old_family(&shallow_arc, 2);
    let fit_rows = rows
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["status"] == "source_fit_rejected")
        .count();
    assert!(fit_rows > 0, "R5 source-fit rejection recorded");
}
