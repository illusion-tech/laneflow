//! #1175 块①②：#1154 的 8 份 pending 候选终验与三例跨边方向失败范围判断。
//!
//! 块①（终验分列，lead 批准预案）：8 份候选全部位于 `sumo:--30266_0`（C24）；
//! 已终验四项逐候选执行（同表示双向拟合复核〔修复后全 span 参考〕/规范端点与程序位/
//! edgeEnd 声明绑定/机动门绑定）；依赖项（编译/LFCA/编译后长度/联合锚点）列逐路径
//! 依赖清单并以 `target_check_not_completed` 示证（只补证）。单/双横线防混淆：
//! `sumo:-30266_0` 为成功边，与缺边 `sumo:--30266_0` 不同 ID。
//!
//! 块②（范围判断，只补证不重判）：三例 `cross_edge_direction_rejected` 逐例记录
//! 编译诊断（`DirectionDiscontinuity`：lhs=dot²、rhs=cos²θmax·‖L‖²·‖R‖²，ADR 0022
//! 固定逐运算图）与边界两侧源几何切向；范围判定判据 = 源边界弦向转角 θ_source 对
//! 冻结阈值 θ_max（Balanced2Deg 2°）：θ_source ≥ θ_max → 一般性质（源几何边界固有，
//! 任何该边界发射均拒）；θ_source < θ_max → 固定曲线范围（发射弦量化/参数化超限，
//! 候选相关）。编译判定本身不重开。

use super::coverage::edge_input;
use super::*;
use sampler::grid::WorkBudget;

// 以下几何助手为本路自足副本（与 `witness_guided_search` 同实现；本地化以保持
// 本 PR diff 仅含 #1175 本路文件，不改 #1154 已合入模块的可见性）。

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

fn from_bits3(value: Value) -> [f64; 3] {
    let bits: [u64; 3] = serde_json::from_value(value).unwrap();
    bits.map(f64::from_bits)
}

fn point3(p: [f64; 3]) -> re::RoadEditingPoint3 {
    re::RoadEditingPoint3::try_new(p[0], p[1], p[2]).unwrap()
}

/// 全 span 参考重建（独立于发射成败 + 覆盖三断言；段容量不设约束）。
fn full_reference_controls(input: &observe::EdgeInput) -> Vec<[[f64; 3]; 6]> {
    sampler::reshape::reference_with_curve_cap(input, usize::MAX)
}

/// B 口径基准（原参考的发射折线）：全 span 六点控制取前四点（cubic），513 节点
/// 均匀网格采样、f32 规范化、跨 span 去重链接。
fn reference_emitted_polyline(controls: &[[[f64; 3]; 6]]) -> Vec<[f64; 3]> {
    let mut points: Vec<[f64; 3]> = Vec::new();
    for c in controls {
        for k in 0..=512 {
            let p = quantize_f32(cubic_bezier(c[0], c[1], c[2], c[3], k as f64 / 512.0));
            if points.last() != Some(&p) {
                points.push(p);
            }
        }
    }
    points
}

/// 折线 → 逐弦退化 cubic（`quality::fit` 仅取前四点）。
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

fn candidates_rows() -> Value {
    let dir = PathBuf::from(std::env::var("G1_CANDIDATES_INPUT").expect("G1_CANDIDATES_INPUT"));
    serde_json::from_slice(&fs::read(dir.join("qualification.json")).unwrap()).unwrap()
}

/// 六点控制 → 弦向（前三后三同点位时即折线弦；此处取端点弦方向）。
fn chord_direction(points: &[[f64; 3]]) -> [f64; 3] {
    let a = points[points.len() - 2];
    let b = points[points.len() - 1];
    [b[0] - a[0], b[1] - a[1], b[2] - a[2]]
}

fn first_chord_direction(points: &[[f64; 3]]) -> [f64; 3] {
    [
        points[1][0] - points[0][0],
        points[1][1] - points[0][1],
        points[1][2] - points[0][2],
    ]
}

/// ADR 0022 固定逐运算图方向检查（与 `spatial_freeze::check_spatial_direction`
/// 同公式复算；θmax=Balanced2Deg 2°，cos²=0.998781…）。返回 (dot, lhs, rhs)。
fn scaled_direction_check(left: [f64; 3], right: [f64; 3]) -> (f64, f64, f64) {
    let scale = |v: [f64; 3]| {
        let s = v[0].abs().max(v[1].abs()).max(v[2].abs());
        [v[0] / s, v[1] / s, v[2] / s]
    };
    let (l, r) = (scale(left), scale(right));
    let dot = l[0] * r[0] + l[1] * r[1] + l[2] * r[2];
    let l_norm = l[0] * l[0] + l[1] * l[1] + l[2] * l[2];
    let r_norm = r[0] * r[0] + r[1] * r[1] + r[2] * r[2];
    (
        dot,
        dot * dot,
        (2.0_f64.to_radians().cos().powi(2)) * l_norm * r_norm,
    )
}

fn angle_deg(_dot: f64, lhs: f64, rhs: f64) -> f64 {
    // cos²θ = lhs / (‖L‖²‖R‖²)；rhs = cos²θmax·‖L‖²‖R‖² → ‖L‖²‖R‖² = rhs/cos²θmax。
    let cos_sq = lhs / (rhs / (2.0_f64.to_radians().cos().powi(2)));
    (cos_sq.sqrt().clamp(-1.0, 1.0)).acos().to_degrees()
}

/// 块①：8 份 pending 候选终验分列（已终验四项 8/8 + 逐路径依赖清单 + 示证记录）。
#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, DIRECT_SUCCESSOR_BASELINE_DIR, G1_CANDIDATES_INPUT (post-fix run-00 evidence dir) and fresh G1_EVIDENCE_OUTPUT"]
fn g1_candidates_final_validation() {
    let out = PathBuf::from(std::env::var("G1_EVIDENCE_OUTPUT").expect("G1_EVIDENCE_OUTPUT"));
    assert!(!out.exists(), "fresh evidence output");
    fs::create_dir_all(&out).unwrap();
    let rows = candidates_rows();
    let (observation, _source, _timing) = capture_lust();
    let traffic = observation.traffic.as_ref().unwrap();
    let reference = reference();
    let originals = original_programs(&observation);
    let components = components::inventory(traffic, &originals, &target_ids(&reference));
    let mut report_rows = Vec::new();
    for (index, prior) in rows["rows"].as_array().unwrap().iter().enumerate() {
        let edge = prior["edge"].as_str().unwrap();
        assert_eq!(
            edge, "sumo:--30266_0",
            "8 candidates all on the frozen edge"
        );
        // 候选程序重建（位级；全直线段形态断言随 1154 冻结）。
        let start = from_bits3(prior["program"]["start"].clone());
        let mut points = vec![start];
        for segment in prior["program"]["segments"].as_array().unwrap() {
            assert!(segment["canvas_selection"].is_null(), "line segments only");
            points.push(from_bits3(segment["end"].clone()));
        }
        let program = {
            let segments = prior["program"]["segments"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| re::RoadEditingCurveSegment::line(point3(from_bits3(s["end"].clone()))))
                .collect::<Vec<_>>();
            re::RoadEditingCurveProgram::try_new(point3(start), segments).unwrap()
        };
        // 已终验①：同表示双向拟合复核（修复后全 span 参考 + 发射折线基准）。
        let mut review_handle = WorkBudget::per_edge_default();
        review_handle.source_fit_remaining = u64::MAX;
        let input = edge_input(&observation, edge);
        let reference_controls = full_reference_controls(input);
        let reference_polyline = reference_emitted_polyline(&reference_controls);
        let reference_fit = sampler::reshape::quality::fit_with_handle(
            &polyline_as_controls(&reference_polyline),
            &input.points,
            &mut review_handle,
        )
        .expect("bounded reference polyline fit");
        let candidate_fit = sampler::reshape::quality::fit_with_handle(
            &polyline_as_controls(&points),
            &input.points,
            &mut review_handle,
        )
        .expect("bounded candidate fit");
        let fit_pass = candidate_fit.within(reference_fit);
        let fit_status = if fit_pass { "validated" } else { "failed" };
        // 已终验②：程序位核对（位级回环：bits → 程序 → bits 逐位一致）。
        // 注：「规范端点（canonical f32）」属**发射/编译域**属性（编译后规范多段线），
        // 与编译后长度同类随依赖链阻断——如实归入 blocked_items（预案把「规范端点/
        // 程序位」并列一项，实现发现分属两域：程序位可验、规范端点属编译域）。
        let round_trip_ok = program_bits(&program) == prior["program"];
        // 已终验③：edgeEnd 声明绑定（停止线声明逐值核对）。
        let stop_line = traffic
            .signals
            .stop_lines
            .iter()
            .find(|s| s.edge_id == edge)
            .expect("stop-line declaration on the edge");
        let edge_end_ok = stop_line.location == "edgeEnd";
        // 已终验④：机动门绑定（门→机动路径→边成员）。
        let gate_paths: Vec<&str> = traffic
            .signals
            .maneuver_gates
            .iter()
            .filter(|g| g.stop_line_id == stop_line.id)
            .map(|g| g.maneuver_path_id.as_str())
            .collect();
        let gate_ok = !gate_paths.is_empty()
            && gate_paths.iter().all(|id| {
                traffic
                    .maneuver_paths
                    .iter()
                    .find(|p| p.id == *id)
                    .is_some_and(|p| path_edges(p).contains(&edge))
            });
        // 依赖清单：逐路径阻断边（该边 4 条机动路径各有且仅有 1 条其他缺边）。
        let component = components
            .iter()
            .find(|c| c.missing_edges.iter().any(|m| m == edge))
            .expect("component containing the edge");
        let mut dependency_paths = Vec::new();
        for path in traffic
            .maneuver_paths
            .iter()
            .filter(|p| component.paths.contains(&p.id))
            .filter(|p| path_edges(p).contains(&edge))
        {
            let ordered = path_edges(path);
            let mut programs = originals.clone();
            programs.insert(edge.to_owned(), program.clone());
            let blockers: Vec<&&str> = ordered
                .iter()
                .filter(|e| !programs.contains_key(**e))
                .collect();
            dependency_paths.push(json!({"path":path.id,
                "ordered_edges":ordered,
                "blockers":blockers}));
        }
        // 示证记录：目标路径真实编译 → target_check_not_completed（缺程序清单）。
        let part = combinations::part(traffic, component);
        let mut programs = originals.clone();
        programs.insert(edge.to_owned(), program.clone());
        let demonstration = compile_part(
            traffic,
            &part.maneuver_paths[0],
            &part,
            &programs,
            Some(&out.join(format!("candidate-{index:02}-compile-evidence.lfca"))),
        );
        assert_eq!(
            demonstration["status"], "target_check_not_completed",
            "blocked chain demonstration"
        );
        report_rows.push(json!({
            "candidate_index":index,"edge":edge,
            "arc_m":prior["arc_m"],
            "validated_items":{
                "same_representation_fit":{"status":fit_status,"pass":fit_pass,
                    "candidate":{"curve_to_source_estimate_m":candidate_fit.curve_to_source_estimate_m,
                        "source_to_curve_estimate_m":candidate_fit.source_to_curve_estimate_m},
                    "reference_emitted_polyline":{"curve_to_source_estimate_m":reference_fit.curve_to_source_estimate_m,
                        "source_to_curve_estimate_m":reference_fit.source_to_curve_estimate_m,
                        "points":reference_polyline.len()}},
                "program_bits_round_trip":{"status":if round_trip_ok {"validated"} else {"failed"}},
                "edge_end_declaration":{"status":if edge_end_ok {"validated"} else {"failed"},
                    "stop_line":stop_line.id,"location":stop_line.location},
                "maneuver_gate_binding":{"status":if gate_ok {"validated"} else {"failed"},
                    "gates":gate_paths}},
            "blocked_items":{"compile":"target_check_not_completed",
                "lfca/compiled_length/canonical_endpoints/joint_anchors":"blocked_by_dependencies（规范端点为发射/编译域属性）",
                "demonstration":{"status":demonstration["status"],
                    "stage":demonstration["stage"],
                    "missing_real_programs":demonstration["missing_real_programs"]}},
            "dependency_paths":dependency_paths,
            // P2-2 修复（评审）：结论由四项**合取**导出——任一失败即该候选未过
            // 对应项，分列记录，不合并记 pending。
            "outcome":if fit_pass && round_trip_ok && edge_end_ok && gate_ok {
                "validated_four_items_pending_dependency"
            } else {
                "failed_items_present_pending_dependency"
            }}));
    }
    recovery::write_json(
        &out,
        "candidates-final-validation.json",
        &json!({"schema":"laneflow-1175-g1-candidates-final-validation",
            "rule":"已终验四项逐候选执行；依赖项列逐路径清单并 target_check_not_completed 示证（只补证）",
            "note":"单/双横线防混淆：sumo:-30266_0 为成功边，与缺边 sumo:--30266_0 不同 ID",
            "rows":report_rows}),
    );
    println!("g1 candidates final validation saved={}", out.display());
}

/// 块②：三例跨边方向失败范围判断（只补证不重判）。
#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, DIRECT_SUCCESSOR_BASELINE_DIR and fresh G1_CROSS_EDGE_OUTPUT"]
fn g1_cross_edge_scope_records() {
    let out = PathBuf::from(std::env::var("G1_CROSS_EDGE_OUTPUT").expect("G1_CROSS_EDGE_OUTPUT"));
    assert!(!out.exists(), "fresh evidence output");
    fs::create_dir_all(&out).unwrap();
    let (observation, _source, _timing) = capture_lust();
    let reference = reference();
    let originals = original_programs(&observation);
    let pool = super::shared_baseline::shared_pools();
    let mut rows = Vec::new();
    for target in reference["baseline_paths"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["result"]["status"] == "cross_edge_direction_rejected")
    {
        let diagnostic = target["result"]["diagnostics"][0]
            .as_str()
            .unwrap()
            .to_owned();
        let field = |key: &str| -> u64 {
            let marker = format!("{key}: ");
            let start = diagnostic.find(&marker).unwrap() + marker.len();
            let tail = &diagnostic[start..];
            let end = tail.find([',', ' ', '}']).unwrap_or(tail.len());
            tail[..end].parse().unwrap()
        };
        let dot = f64::from_bits(field("dot_bits"));
        let lhs = f64::from_bits(field("lhs_bits"));
        let rhs = f64::from_bits(field("rhs_bits"));
        let keys = |key: &str| -> String {
            let marker = format!("{key}: ");
            let start = diagnostic.find(&marker).unwrap() + marker.len();
            let tail = &diagnostic[start..];
            let tail = tail.trim_start_matches("Some(");
            let end = tail.find('"').unwrap()
                + 1
                + tail[tail.find('"').unwrap() + 1..].find('"').unwrap();
            tail[..end].trim_matches('"').to_owned()
        };
        let left_key = keys("lane_edge_key");
        let right_key = keys("related_lane_edge_key");
        // 诊断键（规范车道边/内连边）→ 观测边 ID：`int:`→SUMO 内连边 `:` 前缀
        // （观测 ID `sumo::-19726_1_0` 形态）；`x` 前缀剥离后 `.`→`#` 还原
        // （规范键形态 `x--32668.0_0` = `x` + `--32668.0_0`，无连字符）。
        let to_edge = |key: &str| -> String {
            if let Some(stripped) = key.strip_prefix("int:") {
                format!("sumo::{stripped}")
            } else {
                let stripped = key.strip_prefix('x').unwrap();
                format!("sumo:{}", stripped.replace('.', "#"))
            }
        };
        let (left_id, right_id) = (to_edge(&left_key), to_edge(&right_key));
        // —— P2-1 重推（评审）：范围判断以**发射边界方向**为据——编译器比较编译后
        // 几何的首/末弦（`hir/spatial.rs:1219–1235`：predecessor 末弦 outgoing vs
        // successor 首弦 incoming），源切向非判据（仅作背景记录）。固定侧 = 原成功
        // 程序（不可改）；候选侧 = 池内候选发射。判定基元 = 固定侧边界弦 × 池内候选
        // 发射弦的转角谱（程序点位边界弦；直线段程序=编译弦精确，弧段为顶点弦近似）。
        let frozen_angle = angle_deg(dot, lhs, rhs);
        let left_fixed = originals.contains_key(&left_id);
        let right_fixed = originals.contains_key(&right_id);
        let scope;
        let scope_note;
        let emission_row;
        if left_fixed && right_fixed {
            // 两侧均固定：编译后边界弦对为原成功固定曲线的固有事实——**属固定曲线**
            //（排除是这两条固定曲线的属性；候选空间不存在，恢复侧不可修复）。P2-1
            // 重推修正：撤回原「一般性质（源几何固有、任何发射均拒）」表述。
            scope = "fixed_curve_property";
            scope_note = "两侧均为原成功固定曲线，编译后边界弦对不可改——排除属固定曲线（非源几何「任何发射均拒」类一般性质；P2-1 重推修正）".to_owned();
            let lpts = program_points(&originals[&left_id]);
            let rpts = program_points(&originals[&right_id]);
            let (d, l, r) =
                scaled_direction_check(chord_direction(&lpts), first_chord_direction(&rpts));
            emission_row = json!({"basis":"编译后几何首/末弦（spatial.rs:1219-1235）",
                "fixed_pair":true,
                "fixed_sides":[left_id,right_id],
                "program_chord_turn_deg":angle_deg(d,l,r),
                "program_chord_note":"程序点位边界弦转角（发射域口径；弧段为顶点弦近似），与冻结编译弦角互证"});
        } else if !left_fixed && !right_fixed {
            scope = "both_candidate_sides";
            scope_note = "两侧均为候选发射（联合候选空间，未穷尽）；不外推普遍拒".to_owned();
            emission_row = json!({"basis":"编译后几何首/末弦（spatial.rs:1219-1235）",
                "candidate_pair":true});
        } else {
            let (fixed_id, candidate_id, fixed_is_left) = if left_fixed {
                (left_id.clone(), right_id.clone(), true)
            } else {
                (right_id.clone(), left_id.clone(), false)
            };
            let fixed_points = program_points(&originals[&fixed_id]);
            let fixed_chord = if fixed_is_left {
                chord_direction(&fixed_points)
            } else {
                first_chord_direction(&fixed_points)
            };
            let candidates = pool.get(&candidate_id).cloned().unwrap_or_default();
            let mut turns: Vec<f64> = candidates
                .iter()
                .map(|cand| {
                    let pts = program_points(cand);
                    let cand_chord = if fixed_is_left {
                        first_chord_direction(&pts)
                    } else {
                        chord_direction(&pts)
                    };
                    let (d, l, r) = scaled_direction_check(fixed_chord, cand_chord);
                    angle_deg(d, l, r)
                })
                .collect();
            turns.sort_by(|a, b| a.total_cmp(b));
            match turns.first().copied() {
                Some(min) if min <= 2.0 => {
                    scope = "candidate_dependent";
                    scope_note = format!(
                        "池内存在边界弦转角 <= 2° 的候选发射（min={min:.3}°）——拒绝系具体候选发射所致，非边界固有；固定侧边界弦不可改"
                    );
                }
                Some(min) => {
                    scope = "pool_universal_reject";
                    scope_note = format!(
                        "候选池内所有发射弦转角均 > 2°（min={min:.3}°，池 n={}）；池外候选空间未测，不外推「任何发射均拒」",
                        candidates.len()
                    );
                }
                None => {
                    scope = "no_candidate_evidence";
                    scope_note = "候选侧池内无候选（发射空间无证据）".to_owned();
                }
            }
            emission_row = json!({"basis":"编译后几何首/末弦（spatial.rs:1219-1235）；程序点位边界弦为发射域口径",
                "fixed_side":fixed_id,"candidate_side":candidate_id,
                "fixed_boundary_chord":fixed_chord,
                "pool_candidates":candidates.len(),
                "candidate_chord_turns_deg":turns,
                "threshold_deg":2.0});
        }
        // 源边界弦向转角仅作背景记录（非编译判据；P2-1 修正）。
        let left_input = edge_input(&observation, &left_id);
        let right_input = edge_input(&observation, &right_id);
        let source_check = scaled_direction_check(
            chord_direction(&left_input.points),
            first_chord_direction(&right_input.points),
        );
        let source_angle = angle_deg(source_check.0, source_check.1, source_check.2);
        rows.push(json!({"target":target["path"],
            "diagnostic":diagnostic,
            "boundary":{"lane_edge_key":left_key,"related_lane_edge_key":right_key,
                "left_edge":left_id,"right_edge":right_id},
            "frozen_check":{"dot_bits":field("dot_bits"),"lhs_bits":field("lhs_bits"),
                "rhs_bits":field("rhs_bits"),"angle_deg":frozen_angle,
                "note":"编译后边界弦权威证据（lhs=dot²、rhs=cos²θmax·‖L‖²·‖R‖²，ADR 0022 固定逐运算图）"},
            "emission_boundary_recheck":emission_row,
            "scope_judgment":scope,
            "scope_note":scope_note,
            "source_boundary_background":{"angle_deg":source_angle,
                "threshold_deg":2.0,"profile":"Balanced2Deg",
                "note":"源边界弦向转角仅作背景记录，非编译判据（编译器比较编译后几何首/末弦）"}}));
    }
    recovery::write_json(
        &out,
        "cross-edge-scope.json",
        &json!({"schema":"laneflow-1175-g1-cross-edge-scope",
            "rule":"只补证不重判；范围判断判据=源边界弦向转角 vs Balanced2Deg 2° 阈值",
            "rows":rows}),
    );
    println!("g1 cross-edge scope records saved={}", out.display());
}
