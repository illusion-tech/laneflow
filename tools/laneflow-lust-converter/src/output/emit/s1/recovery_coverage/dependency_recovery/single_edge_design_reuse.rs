//! #1174 流 A-2：单缺边分量设计变量方法复用（C09/C11/C20/C21/C28/C37/C41）。
//!
//! 执行口径冻结于 `research/issue-1174-single-edge-design-reuse/plan.md`：#1136 同款
//! 148 配置族 + #1154 裁定 B 同表示资格闸（族内分析形态筛选与发射折线资格两关叠加、
//! 拒绝分列）+ 共享 `WorkBudget` 单句柄计费 + 文本 token 域位级比较。样本 ID 程序化
//! 回读 #1134 冻结记录断言（2026-10-30 纪律条：派工/Issue 样本 ID 禁止凭记忆书写）。
//! 本模块不改生产采样、compiler 配置档、原成功几何、拓扑与控制锚点；见证/排除/未决
//! 只作记录不引用旧结论；零恢复为有效结论（plan §1/§6）。

use super::*;

/// plan §1/§6 冻结样本与段容量表（程序化提取自 #1134 冻结记录，逐值见本目录 edges.json）。
const EDGES: [(&str, usize); 7] = [
    ("sumo::-10034_4_0", 4),
    ("sumo::-10046_0_0", 4),
    ("sumo::-1426_1_0", 4),
    ("sumo::-1426_2_0", 5),
    ("sumo::-19726_2_0", 5),
    ("sumo::-22042_7_0", 5),
    ("sumo::-25490_4_0", 4),
];

/// plan §2：各边规定倒圆结点（#1134 诊断记录钉版；结点 k = span k−1/k 接缝，
/// 编号 1..s−1）。C21 span2 末弦倒圆 ⇒ 结点 3；C28 span1 末弦倒圆 ⇒ 结点 2；
/// C37 span3 首/末弦倒圆 ⇒ 结点 3/4；其余边无记录的倒圆结点。
const PRESCRIBED_FILLET_KNOTS: &[(&str, &[usize])] = &[
    ("sumo::-10034_4_0", &[]),
    ("sumo::-10046_0_0", &[]),
    ("sumo::-1426_1_0", &[]),
    ("sumo::-1426_2_0", &[3]),
    ("sumo::-19726_2_0", &[2]),
    ("sumo::-22042_7_0", &[3, 4]),
    ("sumo::-25490_4_0", &[]),
];

fn frozen_inputs() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../../tests/fixtures/issue-1107-inputs.json"
    ))
    .unwrap()
}

/// 顶层键值的原文切片（文本 token 域比较的锚侧输入）：保持原文数字 token，不经
/// serde 解析再序列化——解析域 1 ULP 误舍入会烤进重序列化文本造成误报（#1155 口径）。
fn raw_top_level_value<'a>(text: &'a str, key: &str) -> &'a str {
    let bytes = text.as_bytes();
    let needle = format!("\"{key}\"");
    let start = text.find(&needle).expect("key present in anchor text") + needle.len();
    let mut pos = start;
    while pos < bytes.len()
        && !matches!(
            bytes[pos],
            b'{' | b'[' | b'"' | b'-' | b't' | b'f' | b'n' | b'0'..=b'9'
        )
    {
        pos += 1;
    }
    let value_start = pos;
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    while pos < bytes.len() {
        let byte = bytes[pos];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else {
            match byte {
                b'"' => in_string = true,
                b'{' | b'[' => depth += 1,
                b'}' | b']' => {
                    depth -= 1;
                    if depth == 0 {
                        pos += 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        pos += 1;
    }
    assert!(depth == 0, "balanced anchor subtree");
    &text[value_start..pos]
}

fn fillet_knots(edge: &str) -> &'static [usize] {
    PRESCRIBED_FILLET_KNOTS
        .iter()
        .find(|(id, _)| *id == edge)
        .expect("frozen fillet knot table")
        .1
}

/// plan §6 口径澄清（lead 裁定 A，#1174）：复核域无工作预算上限、只记实耗；
/// 搜索域每边句柄 32,768/32M/32M 零改动（`search_with_curve_cap_budget` 照旧）。
fn review_handle() -> sampler::grid::WorkBudget {
    let mut handle = sampler::grid::WorkBudget::per_edge_default();
    handle.source_fit_remaining = u64::MAX;
    handle
}

/// 分组从 span 0 起按组容量切块；被合段跨过的结点 = 非组边界的内部结点，
/// `crossed_fillet_knots` 只计入其中的规定倒圆结点（非倒圆内部结点不计入）。
fn crossed_fillet_knots(group: &str, reference_cubics: usize, edge: &str) -> Vec<usize> {
    let group_size = match group {
        "reference" | "keep" => 1,
        "pair" => 2,
        "whole" => reference_cubics,
        other => panic!("unknown seam group {other}"),
    };
    (1..reference_cubics)
        .filter(|k| k % group_size != 0)
        .filter(|k| fillet_knots(edge).contains(k))
        .collect()
}

fn crosses_prescribed_fillet_seam(group: &str, reference_cubics: usize, edge: &str) -> bool {
    !crossed_fillet_knots(group, reference_cubics, edge).is_empty()
}

// ===== #1154 裁定 B 同表示资格（语义逐值沿 qualification-recompute 钉版）=====

fn cubic_bezier(a: [f64; 3], c1: [f64; 3], c2: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    let u = 1.0 - t;
    let mut p = [0.0; 3];
    for i in 0..3 {
        p[i] =
            u * u * u * a[i] + 3.0 * u * u * t * c1[i] + 3.0 * u * t * t * c2[i] + t * t * t * b[i];
    }
    p
}

fn quantize_f32(p: [f64; 3]) -> [f64; 3] {
    [
        (p[0] as f32) as f64,
        (p[1] as f32) as f64,
        (p[2] as f32) as f64,
    ]
}

/// 折线 → 逐弦退化 cubic（`quality::fit` 仅取前四点；两端点线性插值即直线弦）。
fn polyline_as_controls(points: &[[f64; 3]]) -> Vec<[[f64; 3]; 6]> {
    points
        .windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let lerp = |t: f64| {
                [
                    a[0] + t * (b[0] - a[0]),
                    a[1] + t * (b[1] - a[1]),
                    a[2] + t * (b[2] - a[2]),
                ]
            };
            [a, lerp(1.0 / 3.0), lerp(2.0 / 3.0), b, [0.0; 3], [0.0; 3]]
        })
        .collect()
}

/// 原参考的发射折线（同表示基准，**完整参考链**）：逐 span 六点控制取前四点（cubic），
/// 513 节点均匀参数网格采样（grid 族基网格）、f32 规范化、跨 span 去重链接（#1154
/// 钉版语义）。基准用 `reference_with_curve_cap` 全 span 重建（#1171 全边参考口径，
/// 观察模式全 span 构建 + 三断言）——`replay_span_controls` 为失败截断前缀，按其
/// 文档「不可用作全边参考」（#1180 P2-2 修正）。
fn reference_emitted_polyline(input: &observe::EdgeInput, max_curves: usize) -> Vec<[f64; 3]> {
    let controls = sampler::reshape::reference_with_curve_cap(input, max_curves);
    let mut points = Vec::new();
    for c in &controls {
        for k in 0..=512 {
            let p = quantize_f32(cubic_bezier(c[0], c[1], c[2], c[3], k as f64 / 512.0));
            if points.last() != Some(&p) {
                points.push(p);
            }
        }
    }
    points
}

/// 候选发射折线：规范程序点位（start + 各 line 段 end），相邻重复点去重。
fn candidate_polyline(program: &Value) -> Vec<[f64; 3]> {
    let start: [u64; 3] = serde_json::from_value(program["start"].clone()).unwrap();
    let mut points = vec![start.map(f64::from_bits)];
    for segment in program["segments"].as_array().unwrap() {
        assert!(
            segment["canvas_selection"].is_null(),
            "line-segment candidates only"
        );
        let end: [u64; 3] = serde_json::from_value(segment["end"].clone()).unwrap();
        let p = end.map(f64::from_bits);
        if points.last() != Some(&p) {
            points.push(p);
        }
    }
    points
}

/// 每边按冻结段容量 s 执行 design.md 口径的有限几何设计变量候选搜索（共享
/// `WorkBudget` 单句柄计费），保留池候选逐份过 #1154 裁定 B 同表示资格闸；
/// 跨缝标记与结点索引、两关拒绝、实耗计数逐处记录（plan §2/§3/§6）。
fn run_searches(inputs: &Value) -> (Value, Value) {
    let mut searches = BTreeMap::new();
    let mut timing = BTreeMap::new();
    for (id, max_curves) in EDGES {
        let input = measurement::input(&inputs[id]);
        let mut handle = sampler::grid::WorkBudget::per_edge_default();
        let started = Instant::now();
        let search =
            sampler::reshape::search_with_curve_cap_budget(&input, max_curves, &mut handle);
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
        let mut report = search.report.clone();
        for row in report["geometries"].as_array_mut().unwrap() {
            let group = row["description"]["group"].as_str().unwrap();
            let knots = json!(crossed_fillet_knots(group, max_curves, id));
            let crosses = json!(crosses_prescribed_fillet_seam(group, max_curves, id));
            row["crossed_fillet_knots"] = knots;
            row["crosses_prescribed_fillet_seam"] = crosses;
        }
        let mut candidates = recovery::candidates(&search);
        for candidate in candidates.as_array_mut().unwrap() {
            let index = candidate["spans"][0]["geometry"].as_u64().unwrap();
            let group = report["geometries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["geometry"] == index)
                .expect("geometry row")["description"]["group"]
                .as_str()
                .unwrap();
            candidate["crossed_fillet_knots"] = json!(crossed_fillet_knots(group, max_curves, id));
            candidate["crosses_prescribed_fillet_seam"] =
                json!(crosses_prescribed_fillet_seam(group, max_curves, id));
        }
        // plan §3/§4：#1154 裁定 B 同表示资格闸——候选发射折线 vs 原参考发射折线
        // （同估计器同表示；跨表示比较已证伪影，不采用）。复核域评估独立记账不占池。
        let reference_points = reference_emitted_polyline(&input, max_curves);
        let reference_controls = polyline_as_controls(&reference_points);
        let mut reference_review = review_handle();
        let reference_fit = sampler::reshape::quality::fit_with_handle(
            &reference_controls,
            &input.points,
            &mut reference_review,
        )
        .expect("bounded reference polyline fit");
        let mut qualified = 0_u64;
        for candidate in candidates.as_array_mut().unwrap() {
            let points = candidate_polyline(&candidate["program"]);
            let controls = polyline_as_controls(&points);
            let mut review = review_handle();
            let fit =
                sampler::reshape::quality::fit_with_handle(&controls, &input.points, &mut review);
            let (record, pass) = match fit {
                Some(fit) => (
                    json!({"status":"executed","pass":fit.within(reference_fit),
                        "candidate":{"curve_to_source_estimate_m":fit.curve_to_source_estimate_m,
                            "source_to_curve_estimate_m":fit.source_to_curve_estimate_m},
                        "reference_emitted_polyline":{"curve_to_source_estimate_m":
                            reference_fit.curve_to_source_estimate_m,
                            "source_to_curve_estimate_m":reference_fit.source_to_curve_estimate_m,
                            "points":reference_points.len()},
                        "screen":"candidate polyline not worse than reference emitted polyline \
                            (same estimator, same representation; maintainer ruling 2026-10-30)"}),
                    fit.within(reference_fit),
                ),
                None => (
                    json!({"status":"not_executed",
                        "reason":"review_domain_fit_budget_exhausted"}),
                    false,
                ),
            };
            candidate["qualification_same_representation"] = record;
            candidate["qualified"] = json!(pass);
            if pass {
                qualified += 1;
            }
        }
        searches.insert(
            id,
            json!({"search":report,"candidates":candidates,
                "qualification":{"rule":"maintainer ruling 2026-10-30 B: candidate polyline \
                    vs reference emitted polyline (same estimator, same representation)",
                    "reference_emitted_polyline_sampling":"per-span cubic first-four-points \
                        at 513 uniform parameter nodes, f32 canonicalized, chained \
                        (grid-family base emission form)",
                    "qualified_candidates":qualified,
                    "review_domain":{"reference_fit_consumed":
                        reference_review.source_fit_consumed,
                        "note":"复核域无工作预算上限、只记实耗，不占搜索域每边 32M \
                            source_fit 池（lead 裁定 A，#1174）"}},
                "work_budget":{"accounting":"work_budget_handle_shared_per_edge",
                    "partitions_consumed":handle.partitions_consumed,
                    "grid_work_consumed":handle.grid_work_consumed,
                    "source_fit_consumed":handle.source_fit_consumed}}),
        );
    }
    (json!(searches), json!(timing))
}

#[test]
#[ignore = "requires fresh CURVE_DESIGN_OUTPUT_DIR"]
fn fixed_single_edge_design_reuse() {
    let out = PathBuf::from(std::env::var("CURVE_DESIGN_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh curve design output");
    fs::create_dir_all(&out).unwrap();
    let (searches, timing) = run_searches(&frozen_inputs());
    recovery::write_json(&out, "single-edge-searches.json", &searches);
    recovery::write_json(&out, "single-edge-timing.json", &timing);
}

#[test]
fn fillet_knot_crossing_uses_the_frozen_knot_table() {
    // s=4：keep/reference 不跨缝；pair 跨结点 1、3；whole 跨 1..3——只计倒圆结点。
    assert!(!crosses_prescribed_fillet_seam(
        "keep",
        4,
        "sumo::-1426_2_0"
    ));
    assert!(crosses_prescribed_fillet_seam("pair", 4, "sumo::-1426_2_0"));
    // C21 倒圆结点 = [3]；pair 在 s=5 跨结点 1、3 ⇒ [3]；whole 跨 1..4 ⇒ [3]。
    assert_eq!(crossed_fillet_knots("pair", 5, "sumo::-1426_2_0"), vec![3]);
    assert_eq!(crossed_fillet_knots("whole", 5, "sumo::-1426_2_0"), vec![3]);
    assert_eq!(
        crossed_fillet_knots("keep", 5, "sumo::-1426_2_0"),
        Vec::<usize>::new()
    );
    // C37 倒圆结点 = [3, 4]；pair 在 s=5 跨 1、3 ⇒ [3]；whole 跨 1..4 ⇒ [3, 4]。
    assert_eq!(crossed_fillet_knots("pair", 5, "sumo::-22042_7_0"), vec![3]);
    assert_eq!(
        crossed_fillet_knots("whole", 5, "sumo::-22042_7_0"),
        vec![3, 4]
    );
    // C28 倒圆结点 = [2]；pair 在 s=5 跨 1、3 ⇒ []；whole ⇒ [2]。
    assert_eq!(
        crossed_fillet_knots("pair", 5, "sumo::-19726_2_0"),
        Vec::<usize>::new()
    );
    assert_eq!(
        crossed_fillet_knots("whole", 5, "sumo::-19726_2_0"),
        vec![2]
    );
}

#[test]
fn qualification_polyline_semantics_match_the_pinned_rule() {
    // 单弦直线 → 双向估计为 0（同估计器语义基准）。
    let straight = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
    let controls = polyline_as_controls(&straight);
    assert_eq!(controls.len(), 2);
    assert_eq!(controls[0][3], straight[1]);
    let mut budget = sampler::grid::WorkBudget::per_edge_default();
    let single = polyline_as_controls(&[straight[0], straight[2]]);
    let fit = sampler::reshape::quality::fit_with_handle(
        &single,
        &[straight[0], straight[2]],
        &mut budget,
    )
    .unwrap();
    assert!(fit.curve_to_source_estimate_m.abs() < 1e-12);
    assert!(fit.source_to_curve_estimate_m.abs() < 1e-12);
    // 离散对齐语义回归钉版：双弦折线 vs 单段来源的反向估计 = 1/64（来源子段端点与
    // 参考弦网格错位的固有粒度；两侧同估计器同表示，比较公平——#1154 钉版行为）。
    let mut budget = sampler::grid::WorkBudget::per_edge_default();
    let fit = sampler::reshape::quality::fit_with_handle(
        &controls,
        &[straight[0], straight[2]],
        &mut budget,
    )
    .unwrap();
    assert!(fit.curve_to_source_estimate_m.abs() < 1e-12);
    assert!((fit.source_to_curve_estimate_m - 0.015_625).abs() < 1e-12);
    // 候选折线构造：相邻重复点去重、canvas_selection 必为 null。
    let program = json!({"start":[0_u64, 0, 0],
        "segments":[{"end":[0, 0, 0], "canvas_selection":null},
            {"end":[4_607_182_418_800_017_408_u64, 0, 0], "canvas_selection":null}]});
    let points = candidate_polyline(&program);
    assert_eq!(points.len(), 2);
}

#[test]
fn raw_top_level_value_extracts_faithful_subtrees() {
    // 字符串内括号/引号不干扰平衡扫描；数字 token 原文保真（17 位十进制不重序列化）。
    let text = "{\"source\":{\"reason\":\"a ] b \\\" c\"},\"components\":[{\"x\":10.220051765441895}],\"path_counts\":{\"accepted\":28}}";
    assert_eq!(
        raw_top_level_value(text, "components"),
        "[{\"x\":10.220051765441895}]"
    );
    assert_eq!(
        raw_top_level_value(text, "path_counts"),
        "{\"accepted\":28}"
    );
    assert_eq!(
        raw_top_level_value(text, "source"),
        "{\"reason\":\"a ] b \\\" c\"}"
    );
    // 原文切片经文本 token 域比较对 1 ULP 误差如实判负。
    assert!(
        calibration::shared_baseline::json_leaf_mismatch(
            raw_top_level_value(text, "components"),
            "[{\"x\":10.220051765441896}]"
        )
        .is_some()
    );
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
