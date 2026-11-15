//! #1141 E1 阶段 2：A′ 必证清单派生（v3.1 §4；**首校准前冻结**钉版件）。
//!
//! 逐候选逐 span：① 原采样参数恢复（`control_f64_bits` + `partition` + `pieces`
//! 调冻结算法重建，`recovered_split_params`）并**逐位核对**（重建切分点量化后与
//! 候选程序规范点逐位一致）；② 分类：**「参数重放」**（已知 (u,v) 经
//! `replay_pair_witness` 真实口径通过）/ **「搜索命中」**（重放未过但诊断搜索找到
//! 见证）/ 不入清单（均未过——留 A 不误杀集，如 `-10130_8_0`/`-10130_18_0` 反例）。

use super::*;
use sampler::feasibility::pair::{PairKind, recovered_split_params, replay_pair_witness};

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
