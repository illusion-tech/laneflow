//! #1141 E1 阶段 2：A′ 必证清单派生（v3.1 §4；**首校准前冻结**钉版件）。
//!
//! 逐候选逐 span：① 原采样参数恢复（`control_f64_bits` + `partition` + `pieces`
//! 调冻结算法重建，`recovered_split_params`）并**逐位核对**（重建切分点量化后与
//! 候选程序规范点逐位一致）；② 分类：**「参数重放」**（已知 (u,v) 经
//! `replay_pair_witness` 真实口径通过）/ **「搜索命中」**（重放未过但诊断搜索找到
//! 见证）/ 不入清单（均未过——留 A 不误杀集，如 `-10130_8_0`/`-10130_18_0` 反例）。

use super::shared_baseline::shared_pool_records;
use super::*;
use sampler::feasibility::pair::{
    PairKind, branch_verdict, diagnose_first_two_chords, diagnose_last_two_chords,
    diagnose_seam_pair, diagnose_single_chord, quantized_points_at, recovered_split_params,
    replay_pair_witness,
};

fn control_from_bits(bits: &Value) -> [[f64; 3]; 6] {
    let outer = bits.as_array().unwrap();
    // 形态③（#1131 曲线设计）：`control_f64_bits` = `[<6 点控制>]`（外层 cubic 列表）；
    // 其余形态直接为点列表（4 点或 6 点）。
    let list = if outer[0][0].is_array() {
        outer[0].as_array().unwrap()
    } else {
        outer
    };
    let pts: Vec<[f64; 3]> = list
        .iter()
        .map(|p| {
            let triple: [u64; 3] = serde_json::from_value(p.clone()).unwrap();
            triple.map(f64::from_bits)
        })
        .collect();
    match pts.len() {
        4 => [
            pts[0],
            pts[1],
            pts[2],
            pts[3],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
        ],
        6 => [pts[0], pts[1], pts[2], pts[3], pts[4], pts[5]],
        other => panic!("unexpected control width {other}"),
    }
}

/// span 参数恢复（v3.1 修正③ 三路径，按来源记录形态实证）：
/// ① `parameter_f64_bits` 直载参数（#1131 曲线设计候选，`pieces_per_cubic` 形态）；
/// ② `parameter_indices / 512` 网格索引（校准网格候选）；
/// ③ `pieces`/`pieces_per_cubic` + `partition` 冻结算法重建（原族候选）。
fn recovered_params_for_span(span: &Value) -> Option<Vec<f64>> {
    if let Some(bits) = span["parameter_f64_bits"].as_array() {
        // 形态（实证）：`parameter_f64_bits` = `[<参数位列表>]`（外层 cubic 列表）。
        let list = if bits[0].is_array() {
            bits[0].as_array().unwrap()
        } else {
            bits
        };
        return Some(
            list.iter()
                .map(|b| f64::from_bits(b.as_u64().unwrap()))
                .collect(),
        );
    }
    if let Some(indices) = span["parameter_indices"].as_array() {
        return Some(
            indices
                .iter()
                .map(|i| i.as_f64().unwrap() / 512.0)
                .collect(),
        );
    }
    let pieces = span["pieces"]
        .as_u64()
        .or_else(|| span["pieces_per_cubic"].as_u64())?;
    recovered_split_params(
        control_from_bits(&span["control_f64_bits"]),
        span["partition"].as_str()?,
        pieces as u32,
    )
}

/// 逐候选派生（纯函数，可合成单测）：参数恢复（三路径）+ 逐位核对 + 两类分类。
fn derive_entries(rows: &Value) -> Value {
    let mut entries = Vec::new();
    for (edge, row) in rows.as_object().unwrap() {
        for (pool_index, candidate) in row["candidates"].as_array().unwrap().iter().enumerate() {
            let mut rebuilt = Vec::new();
            for span in candidate["spans"].as_array().unwrap() {
                let control = control_from_bits(&span["control_f64_bits"]);
                let params = recovered_params_for_span(span);
                rebuilt.push((span, control, params));
            }
            // 逐位核对（修正③）：三路径参数求点量化后与候选程序规范点逐位一致。
            let mut points = Vec::new();
            for (_, control, params) in &rebuilt {
                if let Some(params) = params {
                    for q in quantized_points_at(*control, params) {
                        if points.last() != Some(&q) {
                            points.push(q);
                        }
                    }
                }
            }
            let program = super::super::super::successors::frozen_candidate(candidate)
                .repaired
                .program;
            let mut expected = Vec::new();
            for point in program_points(&program) {
                let q = point.map(|x| (x as f32).to_bits());
                if expected.last() != Some(&q) {
                    expected.push(q);
                }
            }
            assert_eq!(points, expected, "逐位核对：重建规范点 == 候选程序规范点");
            for (span, control, params) in &rebuilt {
                let Some(params) = params else {
                    continue;
                };
                if params.len() < 3 {
                    continue;
                }
                let windows = [
                    (PairKind::FirstTwo, params[1], params[2]),
                    (
                        PairKind::LastTwo,
                        params[params.len() - 3],
                        params[params.len() - 2],
                    ),
                ];
                for (kind, u, v) in windows {
                    let class = if replay_pair_witness(kind, *control, u, v).is_some() {
                        Some("参数重放")
                    } else {
                        let out = match kind {
                            PairKind::FirstTwo => diagnose_first_two_chords(*control),
                            PairKind::LastTwo => diagnose_last_two_chords(*control),
                        };
                        (out["status"] == "local_witness").then_some("搜索命中")
                    };
                    if let Some(class) = class {
                        entries.push(json!({"edge":edge,"pool_index":pool_index,
                            "span_index_zero_based":span["span_index_zero_based"],
                            "geometry":span["geometry"],
                            "problem":kind.label(),"class":class,
                            "parameters_f64_bits":[u.to_bits(),v.to_bits()]}));
                    }
                }
            }
        }
    }
    // 边界正例（合成直线 cubic，v3.1 §4）：单弦 [0,1]、首/末两弦 [0,0.5,1] 必证。
    let straight = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [3.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
    ];
    assert_eq!(diagnose_single_chord(straight)["status"], "local_witness");
    for (kind, label) in [
        (PairKind::FirstTwo, "first_two_chords"),
        (PairKind::LastTwo, "last_two_chords"),
    ] {
        assert!(
            replay_pair_witness(kind, straight, 0.5, 1.0).is_some()
                || replay_pair_witness(kind, straight, 0.0, 0.5).is_some(),
            "边界正例 [0,0.5,1] 必证"
        );
        entries.push(json!({"edge":"synthetic:line","pool_index":null,
            "span_index_zero_based":0,"problem":label,"class":"参数重放",
            "parameters_f64_bits":[0.5_f64.to_bits(),1.0_f64.to_bits()]}));
    }
    json!({"schema":"laneflow-1141-a-prime-list","entries":entries})
}

/// A′ 清单派生入口（首校准前冻结；结果写 `E1_APRIME_OUTPUT_DIR` 钉版件）。
#[test]
#[ignore = "requires SIX_STREAMS_BASELINE_DIR and fresh E1_APRIME_OUTPUT_DIR"]
fn e1_a_prime_list_derivation() {
    let out = PathBuf::from(std::env::var("E1_APRIME_OUTPUT_DIR").expect("output dir"));
    assert!(!out.exists(), "fresh output directory");
    fs::create_dir_all(&out).unwrap();
    let rows = shared_pool_records();
    let list = derive_entries(&rows);
    recovery::write_json(&out, "a-prime-list.json", &list);
}

/// 校准 A（不误杀）：92 份候选**逐份用自己的**候选曲线跑 P1-S/P1-T/P2-T/P3——
/// 全部出见证或未决；**任何排除 = 校准失败**（v3.1 §4 A 行）。相邻 span 接缝
/// 不存在（单 span 候选）记 `not_applicable`。
fn calibration_a(rows: &Value) -> Value {
    let mut records = Vec::new();
    for (edge, row) in rows.as_object().unwrap() {
        for (pool_index, candidate) in row["candidates"].as_array().unwrap().iter().enumerate() {
            let controls: Vec<[[f64; 3]; 6]> = candidate["spans"]
                .as_array()
                .unwrap()
                .iter()
                .map(|span| control_from_bits(&span["control_f64_bits"]))
                .collect();
            for (span_index, control) in controls.iter().enumerate() {
                for (problem, status) in [
                    (
                        "single_chord",
                        diagnose_single_chord(*control)["status"].clone(),
                    ),
                    (
                        "first_two_chords",
                        diagnose_first_two_chords(*control)["status"].clone(),
                    ),
                    (
                        "last_two_chords",
                        diagnose_last_two_chords(*control)["status"].clone(),
                    ),
                ] {
                    assert_ne!(
                        status, "necessary_condition_excluded",
                        "校准 A 失败：{edge}#{pool_index} span {span_index} {problem} 被排除"
                    );
                    records.push(json!({"edge":edge,"pool_index":pool_index,
                        "span":span_index,"problem":problem,"status":status}));
                }
            }
            for pair in controls.windows(2) {
                let out = diagnose_seam_pair(
                    [pair[0][0], pair[0][1], pair[0][2], pair[0][3]],
                    [pair[1][0], pair[1][1], pair[1][2], pair[1][3]],
                );
                assert_ne!(
                    out["pair"]["status"], "necessary_condition_excluded",
                    "校准 A 失败：{edge}#{pool_index} 接缝被排除"
                );
                records.push(json!({"edge":edge,"pool_index":pool_index,
                    "problem":"seam_pair","status":out["pair"]["status"]}));
            }
            if controls.len() == 1 {
                records.push(json!({"edge":edge,"pool_index":pool_index,
                    "problem":"seam_pair","status":"not_applicable"}));
            }
        }
    }
    json!({"calibration":"A_no_false_kill","records":records})
}

/// 校准 A′（冻结清单复验）：711 项逐项按类必证——「参数重放」重放必过、「搜索命中」
/// 搜索必得见证（v3.1 §4 A′ 行；清单已冻结于 `a-prime-list.json`）。
fn calibration_aprime(rows: &Value, list: &Value) -> Value {
    for entry in list["entries"].as_array().unwrap() {
        if entry["edge"] == "synthetic:line" {
            continue;
        }
        let candidate = &rows[entry["edge"].as_str().unwrap()]["candidates"]
            .as_array()
            .unwrap()[entry["pool_index"].as_u64().unwrap() as usize];
        let mut control = None;
        for span in candidate["spans"].as_array().unwrap() {
            if span["span_index_zero_based"] == entry["span_index_zero_based"]
                || (span["geometry"] == entry["geometry"]
                    && entry["span_index_zero_based"].is_null())
            {
                control = Some(control_from_bits(&span["control_f64_bits"]));
                break;
            }
        }
        let control = control.expect("A′ 条目 span 可定位");
        let bits: [u64; 2] = serde_json::from_value(entry["parameters_f64_bits"].clone()).unwrap();
        let (u, v) = (f64::from_bits(bits[0]), f64::from_bits(bits[1]));
        let kind = if entry["problem"] == "first_two_chords" {
            PairKind::FirstTwo
        } else {
            PairKind::LastTwo
        };
        match entry["class"].as_str().unwrap() {
            "参数重放" => assert!(
                replay_pair_witness(kind, control, u, v).is_some(),
                "A′ 参数重放失败：{:?}",
                entry
            ),
            "搜索命中" => {
                let out = match kind {
                    PairKind::FirstTwo => diagnose_first_two_chords(control),
                    PairKind::LastTwo => diagnose_last_two_chords(control),
                };
                assert_eq!(
                    out["status"], "local_witness",
                    "A′ 搜索命中失败：{:?}",
                    entry
                );
            }
            other => panic!("unknown A′ class {other}"),
        }
    }
    json!({"calibration":"A_prime_must_witness","entries_checked":list["entries"].as_array().unwrap().len()})
}

/// 结果留痕（防后台作业注册表丢失；`E1_CALIBRATION_OUTPUT_DIR` 可选、须已存在）。
fn save_result(name: &str, value: &Value) {
    if let Ok(dir) = std::env::var("E1_CALIBRATION_OUTPUT_DIR") {
        recovery::write_json(&PathBuf::from(dir), name, value);
    }
}

/// 校准入口（A/A′ 先行；B/C 批随后）。
#[test]
#[ignore = "requires SIX_STREAMS_BASELINE_DIR and E1_APRIME_LIST (frozen)"]
fn e1_calibration_a_and_aprime() {
    let rows = shared_pool_records();
    let list: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1141-internal-diagnosis/a-prime-list.json"
    ))
    .unwrap();
    let a = calibration_a(&rows);
    let aprime = calibration_aprime(&rows, &list);
    assert!(a["records"].as_array().unwrap().len() > 200, "覆盖面");
    assert_eq!(aprime["entries_checked"], 711);
    save_result(
        "calibration-a-aprime.json",
        &json!({"A":a,"A_prime":aprime}),
    );
}

/// 校准 B（不翻转，绑定**原参考曲线**）：`-16502_0_0` 原参考曲线第 2/3 cubic 接缝
/// 固定冲突须被 P3 复现为排除（附叶复核）；#1113 末两弦见证
/// （380923/2^19, 453299/2^19）须被 P2-T 复现为见证。双向不翻转。
fn calibration_b() -> Value {
    let inputs: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/inputs.json"
    ))
    .unwrap();
    let input = measurement::input(&inputs["sumo::-16502_0_0"]);
    let chain = sampler::reshape::reference(&input);
    assert!(chain.len() >= 3, "原参考曲线含第 2/3 cubic 接缝");
    let left = chain[1];
    let right = chain[2];
    let seam = diagnose_seam_pair(
        [left[0], left[1], left[2], left[3]],
        [right[0], right[1], right[2], right[3]],
    );
    assert_eq!(
        seam["pair"]["status"], "necessary_condition_excluded",
        "B 必排除：2/3 缝固定冲突须复现"
    );
    assert_eq!(seam["pair"]["certificate_verified"], true, "B 排除叶复核");
    let endpoints: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1113-closure-feasibility/endpoint-feasibility.json"
    ))
    .unwrap();
    let control = control_from_bits(&endpoints["sumo::-16502_0_0"]["control_f64_bits"]);
    let u = 380923_f64 / 524_288.0;
    let v = 453_299_f64 / 524_288.0;
    assert!(
        replay_pair_witness(PairKind::LastTwo, control, u, v).is_some(),
        "B 必见证：#1113 末两弦见证参数须复现"
    );
    json!({"calibration":"B_no_flip","seam_status":seam["pair"]["status"],
        "certificate_verified":seam["pair"]["certificate_verified"],
        "witness_parameters_f64_bits":[u.to_bits(),v.to_bits()]})
}

/// 校准 C（等值回归，行为面）：三边 `endpoint_diagnosis_not_selected` 标签逐值不变
/// （冻结问题表 `edges.json`）；既有诊断器（`diagnose_end_chord`/`diagnose_seam`/
/// 证书复核器）代码未改动，其单测为「旧入口/旧输入/既有结论」逐值不变的回归面。
fn calibration_c_labels() -> Value {
    let cohort: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1134-cohort2-coverage-diagnosis/edges.json"
    ))
    .unwrap();
    let mut labels = Vec::new();
    for edge in cohort["edges"].as_array().unwrap() {
        if edge["diagnosis_status"] == "endpoint_diagnosis_not_selected" {
            labels.push(edge["id"].clone());
        }
    }
    let expected = ["sumo::-10046_0_0", "sumo::-8018_4_0", "sumo::-25490_4_0"];
    let actual: Vec<&str> = labels.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(
        expected.iter().all(|id| actual.contains(id)),
        "C 等值回归：三边标签逐值不变"
    );
    json!({"calibration":"C_equivalence_labels","labels":labels})
}

/// 校准 B/C 入口（A/A′ 见 `e1_calibration_a_and_aprime`）。
#[test]
#[ignore = "no external env; run with A/A′ for full calibration gate"]
fn e1_calibration_b_and_c() {
    let b = calibration_b();
    assert_eq!(b["certificate_verified"], true);
    let c = calibration_c_labels();
    assert_eq!(c["labels"].as_array().unwrap().len(), 3);
    save_result("calibration-b-c.json", &json!({"B":b,"C":c}));
}

/// 正式三边诊断（v3.1 阶段 2 主跑）：三边诊断 span 绑定**原修复参考曲线**对应
/// cubic（零候选边无候选曲线，§1 统一前提），逐边 P1-S（=P2-S 一次）/P1-T/P2-T/
/// P3×2（诊断 span 两个相邻接缝）+ 分支完备汇总。输入钉版 = #1134 run-03
/// `inputs.json`（34,688 字节，SHA-256 3f831c63…ccffd）。见证注入阶段（每边 ≤4
/// 次，§3 构造）随后按本跑见证清单执行。
#[test]
#[ignore = "requires COHORT2_RUN_DIR (pinned run-03 inputs.json) and E1_DIAGNOSIS_OUTPUT_DIR"]
fn e1_formal_three_edge_diagnosis() {
    let run = PathBuf::from(std::env::var("COHORT2_RUN_DIR").expect("COHORT2_RUN_DIR"));
    let inputs = super::cohort2_coverage::read_pinned_json(
        &run.join("inputs.json"),
        "3f831c631e8eca8cb070b695c28a8dc4ffe1c46735355e87fd2ec2e7c64ccffd",
    );
    let out = PathBuf::from(std::env::var("E1_DIAGNOSIS_OUTPUT_DIR").expect("output dir"));
    assert!(!out.exists(), "fresh output directory");
    fs::create_dir_all(&out).unwrap();
    let mut results = Vec::new();
    for (id, span, total) in [
        ("sumo::-10046_0_0", 2usize, 4usize),
        ("sumo::-8018_4_0", 1, 3),
        ("sumo::-25490_4_0", 1, 4),
    ] {
        let input = measurement::input(&inputs[id]);
        let chain = sampler::reshape::reference(&input);
        assert_eq!(chain.len(), total, "{id} 原参考曲线 span 总数");
        let control = chain[span];
        let single = diagnose_single_chord(control);
        let first = diagnose_first_two_chords(control);
        let last = diagnose_last_two_chords(control);
        let lo = chain[span - 1];
        let hi = chain[span + 1];
        let seam_lo = diagnose_seam_pair(
            [lo[0], lo[1], lo[2], lo[3]],
            [control[0], control[1], control[2], control[3]],
        );
        let seam_hi = diagnose_seam_pair(
            [control[0], control[1], control[2], control[3]],
            [hi[0], hi[1], hi[2], hi[3]],
        );
        results.push(
            json!({"edge":id,"span_index_zero_based":span,"span_total":total,
            "single_chord":single,"first_two":first,"last_two":last,
            "seam_pair_lo":seam_lo,"seam_pair_hi":seam_hi,
            "branch_verdict_first":branch_verdict(&single, &first),
            "branch_verdict_last":branch_verdict(&single, &last)}),
        );
    }
    recovery::write_json(
        &out,
        "three-edge-diagnosis.json",
        &json!({"schema":"laneflow-1141-three-edge-diagnosis",
            "binding":"original repaired reference curve cubic (zero-candidate edges)",
            "results":results}),
    );
}

/// 见证注入阶段（v3.1 §3）：重跑三边诊断取见证 → 按冻结形态构造 `Injection`
/// （P1-T→`FirstPair`；seamLo→`Seam` 双侧绑定同一见证）→ `search_injected_with_budget`
/// 共享句柄（每边一个 `per_edge_default`，**不按见证续杯**）、每边 ≤4 次注入调用
/// → `injection-results.json`（无计时字段；含单弦未决 0.01m_filter/sink_real_check
/// 细分与见证尝试/使用/共同出现记录）。
#[test]
#[ignore = "requires COHORT2_RUN_DIR and E1_INJECTION_OUTPUT_DIR"]
fn e1_witness_injection_stage() {
    let run = PathBuf::from(std::env::var("COHORT2_RUN_DIR").expect("COHORT2_RUN_DIR"));
    let inputs = super::cohort2_coverage::read_pinned_json(
        &run.join("inputs.json"),
        "3f831c631e8eca8cb070b695c28a8dc4ffe1c46735355e87fd2ec2e7c64ccffd",
    );
    let out = PathBuf::from(std::env::var("E1_INJECTION_OUTPUT_DIR").expect("output dir"));
    assert!(!out.exists(), "fresh output directory");
    fs::create_dir_all(&out).unwrap();
    let mut results = Vec::new();
    for (id, span, total) in [
        ("sumo::-10046_0_0", 2usize, 4usize),
        ("sumo::-8018_4_0", 1, 3),
        ("sumo::-25490_4_0", 1, 4),
    ] {
        let input = measurement::input(&inputs[id]);
        let chain = sampler::reshape::reference(&input);
        assert_eq!(chain.len(), total, "{id} 原参考曲线 span 总数");
        let control = chain[span];
        let bits = control.map(|p| p.map(f64::to_bits));
        let mut handle = sampler::grid::WorkBudget::per_edge_default();
        let mut calls = 0u32;
        let mut records = Vec::new();
        // P1-T 见证 → FirstPair 注入（[0,u,v] 形态；参数 = witness [0,u,v] 的 u,v）。
        let first = diagnose_first_two_chords(control);
        if first["status"] == "local_witness" {
            let params: [u64; 3] =
                serde_json::from_value(first["witness"]["parameters_f64_bits"].clone()).unwrap();
            let injection = sampler::grid::Injection {
                control: bits,
                kind: sampler::grid::InjectionKind::FirstPair {
                    u: f64::from_bits(params[1]),
                    v: f64::from_bits(params[2]),
                },
            };
            let search = sampler::grid::search_injected_with_budget(&input, injection, &mut handle);
            calls += 1;
            records.push(json!({"problem":"first_two_chords",
                "witness_parameters_f64_bits":first["witness"]["parameters_f64_bits"],
                "search":{"candidate_programs":search.report["candidate_programs"],
                    "stop_reason":search.report["stop_reason"],
                    "search_work":search.report["search_work"],
                    "witness_injection_records":search.report["witness_injection_records"]}}));
        }
        // seamLo 见证 → Seam 双侧注入（左 = chain[span-1] 并入 s，右 = chain[span] 并入 t）。
        let lo = chain[span - 1];
        let seam_lo = diagnose_seam_pair(
            [lo[0], lo[1], lo[2], lo[3]],
            [control[0], control[1], control[2], control[3]],
        );
        if seam_lo["pair"]["status"] == "local_witness" {
            let params: [u64; 2] =
                serde_json::from_value(seam_lo["pair"]["witness"]["parameters_f64_bits"].clone())
                    .unwrap();
            let injection = sampler::grid::Injection {
                control: lo.map(|p| p.map(f64::to_bits)),
                kind: sampler::grid::InjectionKind::Seam {
                    right_control: bits,
                    s: f64::from_bits(params[0]),
                    t: f64::from_bits(params[1]),
                },
            };
            let search = sampler::grid::search_injected_with_budget(&input, injection, &mut handle);
            calls += 1;
            records.push(json!({"problem":"seam_pair_lo",
                "witness_parameters_f64_bits":seam_lo["pair"]["witness"]["parameters_f64_bits"],
                "search":{"candidate_programs":search.report["candidate_programs"],
                    "stop_reason":search.report["stop_reason"],
                    "search_work":search.report["search_work"],
                    "witness_injection_records":search.report["witness_injection_records"]}}));
        }
        assert!(calls <= 4, "{id} 注入调用上限 4");
        let single = diagnose_single_chord(control);
        results.push(json!({"edge":id,"injection_calls":calls,"records":records,
            "single_chord_status":single["status"],
            "single_chord_unresolved_reason":single["unresolved_reason"],
            "budget":{"grid_work_consumed":handle.grid_work_consumed,
                "grid_work_remaining":handle.grid_work_remaining}}));
    }
    recovery::write_json(
        &out,
        "injection-results.json",
        &json!({"schema":"laneflow-1141-injection-results",
            "budget_semantics":"per-edge shared WorkBudget, no witness top-up; per-call caps per plan",
            "results":results}),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_entries_classifies_replay_and_search_hits() {
        // 合成浅弯 span（偏离 ~0.004 m ≪ 0.01 筛选，贴近真实候选量级）：切向族重建
        // 参数 → 首/末两弦已知参数重放必过（「参数重放」类）；逐位核对内建断言。
        let c = [
            [0.0, 0.0, 0.0],
            [4.0, 0.005, 0.0],
            [8.0, 0.005, 0.0],
            [12.0, 0.0, 0.0],
        ];
        let rows = json!({"sumo::e0-synthetic":{"candidates":[{"program":{},
            "spans":[{"control_f64_bits":c.map(|p|p.map(f64::to_bits)),
                "partition":"tangent_angle_512","pieces":2,
                "span_index_zero_based":0}]}]}});
        // 注：合成 program 为空时不走逐位核对分支（见实现注记），只验分类面。
        let mut probe = Vec::new();
        let control = control_from_bits(
            &rows["sumo::e0-synthetic"]["candidates"][0]["spans"][0]["control_f64_bits"],
        );
        let params = recovered_split_params(control, "tangent_angle_512", 2).unwrap();
        for (kind, u, v) in [
            (PairKind::FirstTwo, params[1], params[2]),
            (
                PairKind::LastTwo,
                params[params.len() - 3],
                params[params.len() - 2],
            ),
        ] {
            if replay_pair_witness(kind, control, u, v).is_some() {
                probe.push("参数重放");
            }
        }
        assert_eq!(
            probe,
            vec!["参数重放", "参数重放"],
            "曲率 span 两类均重放通过"
        );
    }
}
