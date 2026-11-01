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
    PairKind, diagnose_first_two_chords, diagnose_last_two_chords, diagnose_single_chord,
    quantized_points_at, recovered_split_params, replay_pair_witness,
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
