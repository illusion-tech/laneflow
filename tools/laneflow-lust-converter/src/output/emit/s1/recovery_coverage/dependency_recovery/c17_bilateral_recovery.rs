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
    println!(
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
