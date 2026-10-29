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
) -> Result<(), String> {
    let constants = &certificate["constants"];
    let claims = &certificate["claims"];
    let number = |value: &serde_json::Value, label: &str| -> Result<f64, String> {
        let parsed = value
            .as_f64()
            .ok_or_else(|| format!("{label}: missing numeric claim"))?;
        if parsed.is_finite() {
            Ok(parsed)
        } else {
            Err(format!("{label}: non-finite numeric claim"))
        }
    };
    let eq = |claim: f64, used: f64, label: &str| -> Result<(), String> {
        if claim.to_bits() != used.to_bits() {
            return Err(format!("{label}: claim {claim} != used {used}"));
        }
        Ok(())
    };
    let upper = |claim: f64, value: f64, label: &str| -> Result<(), String> {
        if claim < value {
            return Err(format!("{label}: upper bound {claim} < computed {value}"));
        }
        Ok(())
    };
    let lower = |claim: f64, value: f64, label: &str| -> Result<(), String> {
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
    let display = |claim: f64, value: f64, label: &str| -> Result<(), String> {
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
