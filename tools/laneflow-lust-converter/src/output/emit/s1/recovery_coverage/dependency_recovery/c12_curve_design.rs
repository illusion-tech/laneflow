//! #1136 流 A：C12 单缺边（`sumo::-10100_1_0`）的曲线/分段设计变量有限探索与完整分量验收。
//! 执行口径冻结于 `research/issue-1136-c12-curve-design/plan.md`，方法事实来源为
//! `research/issue-1115-failed-curve-design/design.md`；本模块不改生产采样、compiler
//! 配置档、原成功几何、拓扑与控制锚点。首弦全参数域排除只对原固定 cubic 成立，span 0
//! 末弦见证仅为局部问题存在性、与排除正交并存；不预设恢复，零恢复为有效结论
//! （plan §1/§6）。共同基线按六路协调章程 v2 经 E0 冻结交付读取（plan §5）。

use super::*;

/// plan §1/§6 冻结样本与段容量：C12 唯一缺边，原诊断 span 0/4 → s=4，临时程序 ≤385 点。
const EDGES: [(&str, usize); 1] = [("sumo::-10100_1_0", 4)];
/// plan §2：span 0/1 接缝为规定倒圆切点（span 0 末弦见证对应位置），合段跨过须逐候选记录。
const PRESCRIBED_FILLET_SEAM_AFTER_SPAN: usize = 0;

fn frozen_inputs() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../../tests/fixtures/issue-1107-inputs.json"
    ))
    .unwrap()
}

/// 分组从 span 0 起按组容量切块；span 0/1 落入同一组即合段跨过规定倒圆接缝。
fn crosses_prescribed_fillet_seam(group: &str, reference_cubics: usize) -> bool {
    // `reference` 为原参考链对照（保留全部分界），与 `keep` 同为不跨缝。
    let group_size = match group {
        "reference" | "keep" => 1,
        "pair" => 2,
        "whole" => reference_cubics,
        other => panic!("unknown seam group {other}"),
    };
    PRESCRIBED_FILLET_SEAM_AFTER_SPAN / group_size
        == (PRESCRIBED_FILLET_SEAM_AFTER_SPAN + 1) / group_size
}

/// 每边按冻结段容量 s 执行 design.md 口径的有限几何设计变量候选搜索；
/// 跨规定倒圆接缝的配置与候选逐处记录 `crosses_prescribed_fillet_seam`（plan §2）。
fn run_searches(inputs: &Value) -> (Value, Value) {
    let mut searches = BTreeMap::new();
    let mut timing = BTreeMap::new();
    for (id, max_curves) in EDGES {
        let input = measurement::input(&inputs[id]);
        let started = Instant::now();
        let search = sampler::reshape::search_with_curve_cap(&input, max_curves);
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
            row["crosses_prescribed_fillet_seam"] =
                json!(crosses_prescribed_fillet_seam(group, max_curves));
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
            candidate["crosses_prescribed_fillet_seam"] =
                json!(crosses_prescribed_fillet_seam(group, max_curves));
        }
        searches.insert(id, json!({"search":report,"candidates":candidates}));
    }
    (json!(searches), json!(timing))
}

#[test]
#[ignore = "requires fresh CURVE_DESIGN_OUTPUT_DIR"]
fn fixed_c12_curve_design() {
    let out = PathBuf::from(std::env::var("CURVE_DESIGN_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh curve design output");
    fs::create_dir_all(&out).unwrap();
    let (searches, timing) = run_searches(&frozen_inputs());
    recovery::write_json(&out, "c12-searches.json", &searches);
    recovery::write_json(&out, "c12-timing.json", &timing);
}

#[test]
fn c12_fillet_seam_crossing_tracks_group_boundaries() {
    // s=4：reference/keep 保留全部分界不跨缝；pair/whole 合并 span 0/1 即跨规定倒圆接缝。
    assert!(!crosses_prescribed_fillet_seam("reference", 4));
    assert!(!crosses_prescribed_fillet_seam("keep", 4));
    assert!(crosses_prescribed_fillet_seam("pair", 4));
    assert!(crosses_prescribed_fillet_seam("whole", 4));
    assert!(!crosses_prescribed_fillet_seam("keep", 2));
    assert!(crosses_prescribed_fillet_seam("whole", 2));
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
