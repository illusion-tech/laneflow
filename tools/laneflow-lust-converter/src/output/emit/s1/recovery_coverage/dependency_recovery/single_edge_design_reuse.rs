//! #1174 流 A-2：单缺边分量设计变量方法复用（C09/C11/C20/C21/C28/C37/C41）。
//!
//! 执行口径冻结于 `research/issue-1174-single-edge-design-reuse/plan.md`：#1136 同款
//! 148 配置族 + #1154 裁定 B 同表示资格闸（族内分析形态筛选与发射折线资格两关叠加、
//! 拒绝分列）+ 共享 `WorkBudget` 单句柄计费 + 文本 token 域位级比较。样本 ID 程序化
//! 回读 #1134 冻结记录断言（2026-10-30 纪律条：派工/Issue 样本 ID 禁止凭记忆书写）。
//! 本模块不改生产采样、compiler 配置档、原成功几何、拓扑与控制锚点；见证/排除/未决
//! 只作记录不引用旧结论；零恢复为有效结论（plan §1/§6）。

use super::*;
use calibration::shared_baseline::replay_span_controls;
use experiment::{merge, pool};

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
/// design.md：每段最多 96 片；临时程序点上限 96s+1（s=4 → 385，s=5 → 481）。
const PIECES_PER_CUBIC_MAX: usize = 96;

/// 分量表：单缺边单路径，两口径收益每分量 1 条完整关联路径 ↔ 1 个原目标。
const COMPONENTS: [(&str, &str, &str); 7] = [
    (
        "C09",
        "sumo:-10034:-30306#1_0-to---30592#6_0::-10034_4_0",
        "sumo::-10034_4_0",
    ),
    (
        "C11",
        "sumo:-10046:--30616#0_0-to--32414#2_0::-10046_0_0",
        "sumo::-10046_0_0",
    ),
    (
        "C20",
        "sumo:-1426:-31916_0-to---32280#1_0::-1426_1_0",
        "sumo::-1426_1_0",
    ),
    (
        "C21",
        "sumo:-1426:-31916_0-to--32280#2_0::-1426_2_0",
        "sumo::-1426_2_0",
    ),
    (
        "C28",
        "sumo:-19726:-32068#1_0-to--32668#0_0::-19726_2_0",
        "sumo::-19726_2_0",
    ),
    (
        "C37",
        "sumo:-22042:-32280#2_0-to--30608#6_2::-22042_7_0",
        "sumo::-22042_7_0",
    ),
    (
        "C41",
        "sumo:-25490:--31516_0-to---32668#0_0::-25490_4_0",
        "sumo::-25490_4_0",
    ),
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

/// plan §1：C20/C21 fan-out 钳制邻居对（钳制冻结，不随曲线改变）。
const FAN_OUT_PAIRS: [&str; 2] = [
    "sumo:-31916_0->sumo::-1426_1_0",
    "sumo:-31916_0->sumo::-1426_2_0",
];

fn frozen_inputs() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/inputs.json"
    ))
    .unwrap()
}

fn pool_counts(pools: &combinations::Pools) -> (usize, usize) {
    (pools.len(), pools.values().map(Vec::len).sum())
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

fn per_edge_counts(pools: &combinations::Pools) -> BTreeMap<String, usize> {
    pools
        .iter()
        .map(|(id, candidates)| (id.clone(), candidates.len()))
        .collect()
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

/// 原参考的发射折线：逐 span 六点控制取前四点（cubic），513 节点均匀参数网格采样
/// （grid 族基网格）、f32 规范化、跨 span 去重链接（#1154 钉版语义）。
fn reference_emitted_polyline(input: &observe::EdgeInput) -> Vec<[f64; 3]> {
    let controls = replay_span_controls(input);
    let mut keys = controls.keys().copied().collect::<Vec<_>>();
    keys.sort_unstable();
    let mut points = Vec::new();
    for key in keys {
        let c = controls[&key];
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
        let reference_points = reference_emitted_polyline(&input);
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

/// plan §5：7 个单缺边单路径分量逐一分列验收；任一边出合格候选即整分量验收，
/// 只有整个分量接受才选用其赋值（局部候选不记分量恢复）。
fn joint_acceptance(
    rows: &[Value],
    paths: &[Value],
    code: &str,
    component: &str,
    missing_edge: &str,
) -> Value {
    let result = &rows
        .iter()
        .find(|row| row["component"] == component)
        .expect("cohort component evaluated")["result"];
    let accepted = result["status"] == "accepted";
    let path_row = paths
        .iter()
        .find(|row| row["path"] == component)
        .expect("target path row");
    let path_accepted = path_row["result"]["status"] == "accepted";
    assert_eq!(
        accepted, path_accepted,
        "component acceptance equals complete-path acceptance"
    );
    if accepted {
        let assignment = result["selected_assignment"].as_object().unwrap();
        assert_eq!(
            assignment.keys().map(String::as_str).collect::<Vec<_>>(),
            vec![missing_edge],
            "one assignment covers the missing edge"
        );
    }
    json!({"code":code,"component":component,"missing_edge":missing_edge,
        "accepted":accepted,"path_accepted":path_accepted,
        "selected_assignment":result["selected_assignment"],
        "trials":result["trials"].as_array().unwrap().len(),
        "assignment_budget_stopped":result["assignment_budget_stopped"],
        "unvisited_assignments":result["unvisited_assignments"]})
}

/// plan §4 长度报告清单与逐边资格/拒绝分层汇总（compiler 列无接受编译时为空）。
fn edge_reports(searches: &Value, traffic: &TrafficPackage, variant_paths: &[Value]) -> Value {
    let rows = EDGES.map(|(id, max_curves)| {
        let search = &searches[id];
        let edge = traffic
            .lane_graph
            .edges
            .iter()
            .find(|edge| edge.id == id)
            .expect("sample edge in topology");
        let compiler_rows = variant_paths
            .iter()
            .filter(|row| row["result"]["status"] == "accepted")
            .flat_map(|row| {
                row["result"]["edges"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|edge_row| edge_row["edge"] == id)
                    .map(|edge_row| {
                        json!({"path":row["path"],"length_mm":edge_row["length_mm"],
                            "canonical_arc_m":edge_row["canonical_arc_m"],
                            "source_length_delta_m":edge_row["source_length_delta_m"]})
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let mut status_counts = BTreeMap::<String, u64>::new();
        for row in search["search"]["geometries"].as_array().unwrap() {
            *status_counts
                .entry(row["status"].as_str().unwrap().to_owned())
                .or_default() += 1;
        }
        json!({"edge":id,"reference_cubics":max_curves,
            "temp_program_points_max":PIECES_PER_CUBIC_MAX*max_curves+1,
            "source_length_m":edge.length,
            "source_shape_polyline_length_m":search["search"]["source_shape_polyline_length_m"],
            "original_reference_length_estimates":search["search"]["reference_length_estimates"],
            "original_reference_source_fit":search["search"]["reference_fit"],
            "prescribed_fillet_knots":fillet_knots(id),
            "fan_out_pairs":if id == "sumo::-1426_1_0" || id == "sumo::-1426_2_0" {
                json!(FAN_OUT_PAIRS)
            } else {
                Value::Null
            },
            "geometry_status_counts":status_counts,
            "partition_attempts":search["search"]["counts"]["partitions"],
            "work_budget":search["work_budget"],
            "geometry_family_visited":search["search"]["geometry_family_visited"],
            "stop_reason":search["search"]["stop_reason"],
            "qualification":search["qualification"],
            "retained_candidates":search["candidates"].as_array().unwrap().iter().map(|candidate| json!({
                "geometry":candidate["spans"][0]["geometry"],
                "partition":candidate["spans"][0]["partition"],
                "pieces_per_cubic":candidate["spans"][0]["pieces_per_cubic"],
                "crossed_fillet_knots":candidate["crossed_fillet_knots"],
                "crosses_prescribed_fillet_seam":candidate["crosses_prescribed_fillet_seam"],
                "qualified":candidate["qualified"],
                "qualification_same_representation":candidate["qualification_same_representation"],
                "source_fit":candidate["spans"][0]["source_fit"],
                "canonical_source_fit":candidate["spans"][0]["canonical_source_fit"],
                "reference_length_estimates":candidate["spans"][0]["reference_length_estimates"],
                "canonical_f32_chord_sum_m":candidate["arc_m"]})).collect::<Vec<_>>(),
            "compiler_rows":compiler_rows})
    });
    json!(rows)
}

/// plan §5 两口径收益分列（章程 §1）：每分量 1 条完整关联路径 ↔ 1 个原目标，
/// 收益按 0/1 与 0/1 逐分量表述；新增接受跨两口径分别计。
fn two_accounting_gains(baseline_result: &Value, new_result: &Value) -> Value {
    let mut associated_paths = 0_u64;
    let mut components = Vec::new();
    for (_, after) in baseline_result["components"]
        .as_array()
        .unwrap()
        .iter()
        .zip(new_result["components"].as_array().unwrap())
        .filter(|(before, after)| {
            after["result"]["status"] == "accepted" && before["result"]["status"] != "accepted"
        })
    {
        associated_paths += after["result"]["complete_component_paths"]
            .as_array()
            .unwrap()
            .len() as u64;
        components.push(after["component"].clone());
    }
    let targets = baseline_result["paths"]
        .as_array()
        .unwrap()
        .iter()
        .zip(new_result["paths"].as_array().unwrap())
        .filter(|(before, after)| {
            after["result"]["status"] == "accepted" && before["result"]["status"] != "accepted"
        })
        .count() as u64;
    json!({"newly_accepted_components":components,
        "complete_associated_paths":associated_paths,
        "original_targets_84":targets,
        "denominator_note":"每分量 1↔1（7 分量合计 7↔7）；84 原目标为全局对照分母，不作本路收益分母"})
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
#[ignore = "requires pinned LUST_SOURCE_DIR, DIRECT_SUCCESSOR_BASELINE_DIR, SIX_STREAMS_BASELINE_DIR, CURVE_DESIGN_FROZEN_RESULT, CURVE_DESIGN_ORDER and fresh CURVE_DESIGN_OUTPUT_DIR"]
fn pinned_lust_single_edge_reuse_experiment() {
    let out = PathBuf::from(std::env::var("CURVE_DESIGN_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh full curve design output");
    fs::create_dir_all(&out).unwrap();
    let order = std::env::var("CURVE_DESIGN_ORDER").unwrap();
    assert!(matches!(order.as_str(), "baseline-new" | "new-baseline"));
    let frozen_text =
        String::from_utf8(fs::read(std::env::var("CURVE_DESIGN_FROZEN_RESULT").unwrap()).unwrap())
            .unwrap();
    let baseline = calibration::shared_baseline::shared_pools();
    assert_eq!(
        pool_counts(&baseline),
        (11, 92),
        "frozen 11 edges 92 candidates"
    );
    let anchor = calibration::shared_baseline::results_anchor_pinned();
    assert_eq!(anchor["components"].as_array().unwrap().len(), 53);
    assert_eq!(anchor["paths"].as_array().unwrap().len(), 84);
    assert_eq!(anchor["path_counts"]["accepted"], 28, "frozen 28/84 anchor");
    assert_eq!(
        anchor["source"]["generation"],
        json!("92-池行"),
        "pinned anchor generation"
    );
    // 锚侧对照一律用原文切片（results-anchor.json 原始字节文本；共享加载器已钉版
    // 断言该字节），不经 serde 解析再序列化——解析域 1 ULP 误舍入会烤进重序列化
    // 文本造成误报（#1155 口径）。
    let anchor_raw = String::from_utf8(
        fs::read(
            PathBuf::from(
                std::env::var("SIX_STREAMS_BASELINE_DIR").expect("SIX_STREAMS_BASELINE_DIR"),
            )
            .join("results-anchor.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let inventory: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1107-dependency-recovery/inventory.json"
    ))
    .unwrap();
    let (observation, _source, mut timing) = capture_lust();
    let originals = original_programs(&observation);
    snapshot(&out, &observation, &originals);
    let traffic = observation.traffic.as_ref().unwrap();
    let components = components::inventory(traffic, &originals, &target_ids(&reference()));
    assert_eq!(json!(components), inventory["components"]);
    assert_eq!(components.len(), 53, "frozen component inventory");
    assert_eq!(components.iter().map(|c| c.paths.len()).sum::<usize>(), 183);
    measurement::write_inputs(
        &out,
        &observation,
        &json!({"samples":EDGES.map(|(id, _)|json!({"edge":id}))}),
    );
    let inputs: Value =
        serde_json::from_slice(&fs::read(out.join("inputs.json")).unwrap()).unwrap();
    let old_inputs = frozen_inputs();
    for (id, _) in EDGES {
        assert_eq!(
            inputs[id], old_inputs[id],
            "unchanged source and emission failure"
        );
        assert!(!originals.contains_key(id), "only failed edges redesigned");
    }
    let (searches, search_times) = run_searches(&inputs);
    // plan §7：候选冻结重放按文本 token 域位级比较（serde 域不作基准）。
    let my_text = format!("{}\n", serde_json::to_string_pretty(&searches).unwrap());
    assert!(
        calibration::shared_baseline::json_leaf_mismatch(&frozen_text, &my_text).is_none(),
        "frozen candidate replay (text token domain)"
    );
    recovery::write_json(&out, "single-edge-searches.json", &searches);
    let mut combined = baseline.clone();
    merge(&mut combined, pool(&searches));
    let mut results = BTreeMap::new();
    let mut validation_times = BTreeMap::new();
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
            let file = format!("single-edge-{family}-component-{index:02}.lfca");
            if COMPONENTS.iter().any(|(_, id, _)| *id == component.id) {
                let part = combinations::part(traffic, component);
                assert!(
                    part.signals
                        .stop_lines
                        .iter()
                        .all(|s| originals.contains_key(&s.edge_id)),
                    "affected stop line geometry is original"
                );
            }
            let mut result = combinations::evaluate(
                traffic,
                component,
                &originals,
                pools,
                Some(&out.join(&file)),
            );
            // plan §5：组合超 256 记 `assignment_budget_stopped` 与未访问赋值数。
            let unvisited = result
                .get("total_assignments")
                .and_then(Value::as_u64)
                .map(|total| total - result["trials"].as_array().unwrap().len() as u64);
            result["unvisited_assignments"] = json!(unvisited);
            if family == "baseline" {
                let prior = &anchor["components"][index];
                assert_eq!(prior["component"], json!(component.id));
            }
            if result["status"] == "accepted" {
                for (id, choice) in result["selected_assignment"].as_object().unwrap() {
                    programs.insert(
                        id.clone(),
                        pools[id][choice.as_u64().unwrap() as usize].clone(),
                    );
                }
            }
            rows.push(json!({"component":component.id,"result":result,
                "artifact":(result["status"]=="accepted").then_some(file)}));
        }
        let paths = reference()["baseline_paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let path = traffic
                    .maneuver_paths
                    .iter()
                    .find(|p| p.id == row["path"].as_str().unwrap())
                    .unwrap();
                json!({"path":path.id,"edges":path_edges(path),"result":compile_path(traffic,path,&programs)})
            })
            .collect::<Vec<_>>();
        for (current, previous) in paths.iter().zip(anchor["paths"].as_array().unwrap()) {
            assert_eq!(current["path"], previous["path"]);
            if previous["result"]["status"] == "accepted" {
                assert_eq!(
                    current["result"]["status"], "accepted",
                    "accepted target retained"
                );
            }
        }
        let counts = paths
            .iter()
            .fold(BTreeMap::<String, usize>::new(), |mut counts, row| {
                *counts
                    .entry(row["result"]["status"].as_str().unwrap().to_owned())
                    .or_default() += 1;
                counts
            });
        if family == "baseline" {
            // 基线逐值对照（文本 token 域，锚侧原文切片 vs 我侧序列化原文）：分量
            // `result` 子树逐值全同（含 unvisited_assignments 字段形态）；行级
            // `artifact`/`cohort` 为 #1134 运行专属标记（文件名与轮次队列），取锚侧
            // 原值过比较（字符串/布尔无 ULP 风险），`component` 身份另行逐行断言。
            let anchor_rows = anchor["components"].as_array().unwrap();
            let shaped = rows
                .iter()
                .zip(anchor_rows)
                .map(|(mine, prior)| {
                    json!({"artifact":prior["artifact"],"cohort":prior["cohort"],
                        "component":mine["component"],"result":mine["result"]})
                })
                .collect::<Vec<_>>();
            for (key, mine) in [
                ("components", serde_json::to_string_pretty(&shaped).unwrap()),
                ("paths", serde_json::to_string_pretty(&paths).unwrap()),
                (
                    "path_counts",
                    serde_json::to_string_pretty(&counts).unwrap(),
                ),
            ] {
                let mismatch = calibration::shared_baseline::json_leaf_mismatch(
                    raw_top_level_value(&anchor_raw, key),
                    &mine,
                );
                assert!(
                    mismatch.is_none(),
                    "baseline {key} match anchor (text token domain): {:?}",
                    mismatch
                );
            }
        }
        let joint = COMPONENTS
            .iter()
            .map(|(code, component, missing_edge)| {
                joint_acceptance(&rows, &paths, code, component, missing_edge)
            })
            .collect::<Vec<_>>();
        results.insert(
            family,
            json!({"components":rows,"paths":paths,"path_counts":counts,
                "pool_sizes":pools.iter().map(|(id,p)|(id,p.len())).collect::<BTreeMap<_,_>>(),
                "joint_acceptance":joint}),
        );
        validation_times.insert(family, started.elapsed().as_secs_f64());
    }
    let baseline_result = results.get("baseline").unwrap().clone();
    let new_result = results.get("new").unwrap().clone();
    let trials_total = |variant: &Value| -> u64 {
        variant["components"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["result"]["trials"].as_array().unwrap().len() as u64)
            .sum()
    };
    let gains = two_accounting_gains(&baseline_result, &new_result);
    let gain = json!({
        "new_candidates_per_edge":EDGES.map(|(id,_)|json!({"edge":id,
            "qualified_candidates":searches[id]["qualification"]["qualified_candidates"],
            "retained_candidates":searches[id]["candidates"].as_array().unwrap().len()})),
        "components_newly_accepted":gains["newly_accepted_components"],
        "two_accounting_gains":{"complete_associated_paths":gains["complete_associated_paths"],
            "original_targets_84":gains["original_targets_84"],
            "denominator_note":gains["denominator_note"]},
        "component_assignment_trials":{"baseline":trials_total(&baseline_result),
            "new":trials_total(&new_result)},
        "baseline_path_counts":baseline_result["path_counts"],
        "new_path_counts":new_result["path_counts"]});
    assert_eq!(baseline_result["path_counts"]["accepted"], 28);
    let result_comparison = json!({
        "target":"E0 shared baseline results-anchor.json (#1134 new variant, generation 92-池行)",
        "role":"result comparison only; not a candidate source",
        "comparison_domain":"text token (json_leaf_mismatch; serde parse domain not a basis)",
        "components_result_equal":true,"paths_equal":true,"path_counts_equal":true,
        "accepted_paths":baseline_result["path_counts"]["accepted"],
        "note":"28/84 为共同对照口径（章程 §1）"});
    recovery::write_json(
        &out,
        "single-edge-results.json",
        &json!({"schema":"laneflow-1174-single-edge-design-reuse",
        "cohort_source":"research/issue-1174-single-edge-design-reuse/edges.json",
        "method_sources":["research/issue-1115-failed-curve-design/design.md",
            "research/issue-1136-c12-curve-design/plan.md",
            "#1154 maintainer ruling 2026-10-30 B"],
        "segment_capacity":EDGES.map(|(id,max_curves)|json!({"edge":id,"reference_cubics":max_curves,
            "temp_program_points_max":PIECES_PER_CUBIC_MAX*max_curves+1})),
        "shared_baseline":{"source":"E0 package via SIX_STREAMS_BASELINE_DIR",
            "edge_count":pool_counts(&baseline).0,"candidate_count":pool_counts(&baseline).1,
            "per_edge":per_edge_counts(&baseline)},
        "new_pool":{"edge_count":pool_counts(&combined).0,"candidate_count":pool_counts(&combined).1,
            "per_edge":per_edge_counts(&combined)},
        "edge_reports":edge_reports(&searches, traffic, new_result["paths"].as_array().unwrap()),
        "variants":results,"result_comparison":result_comparison,"new_gain":gain,
        "original_geometry_controls_and_failures_equal":true,"production_g1_accepted":false}),
    );
    timing["search_order"] = json!(order);
    timing["search_seconds"] = json!(search_times);
    timing["validation_seconds"] = json!(validation_times);
    recovery::write_json(&out, "single-edge-timing.json", &timing);
    println!("single edge design reuse evidence saved={}", out.display());
}

#[test]
fn reuse_cohort_matches_frozen_1134_records() {
    // 样本 ID 纪律：逐值程序化回读 #1134 冻结记录，禁止凭记忆书写。
    let frozen: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1134-cohort2-coverage-diagnosis/edges.json"
    ))
    .unwrap();
    let cohort: Value = serde_json::from_str(include_str!(
        "../../../../../../../../research/issue-1174-single-edge-design-reuse/edges.json"
    ))
    .unwrap();
    assert_eq!(cohort["lane"], "A-2");
    for (id, max_curves) in EDGES {
        let row = frozen["edges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == id)
            .expect("edge in frozen 1134 record");
        assert_eq!(row["span_total"], json!(max_curves), "{id} frozen s");
        let mine = cohort["edges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == id)
            .expect("edge in cohort record");
        assert_eq!(mine["start_tangent"], row["start_tangent"], "{id} start");
        assert_eq!(mine["finish_tangent"], row["finish_tangent"], "{id} finish");
        assert_eq!(mine["history"], row["history"], "{id} history");
    }
    for (code, component, missing_edge) in COMPONENTS {
        let row = frozen["components"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["code"] == code)
            .expect("component in frozen 1134 record");
        assert_eq!(row["id"], component, "{code} component identity");
        assert_eq!(
            row["missing_edges"],
            json!(missing_edge),
            "{code} missing edge"
        );
    }
    assert_eq!(
        EDGES.map(|(_, s)| PIECES_PER_CUBIC_MAX * s + 1),
        [385, 385, 385, 481, 481, 481, 385]
    );
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
