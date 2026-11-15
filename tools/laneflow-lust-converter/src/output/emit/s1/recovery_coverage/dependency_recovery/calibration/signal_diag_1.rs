//! #1138：信号诊断组 1（C14/C15/C18/C46，23 条缺边）的固定曲线覆盖补齐、逐边局部
//! 必要条件诊断与候选齐备分量完整控制器验收。执行口径全部冻结于
//! `research/issue-1138-signal-diag-1/plan.md`，方法事实来源
//! `research/issue-1121-coverage-diagnosis/plan.md` 逐值沿用，实现模板为
//! `calibration::cohort2_coverage`；本模块不改生产采样、compiler 配置档、阈值与原
//! 成功几何，不改共享代码（协调章程：共享代码只由 E 修改）。计划与冻结实现不一致时
//! 停止执行并重新授权（计划 §7），不得单向迁就。

use super::super::experiment::pool;

use super::*;

/// plan §5 冻结共同基线池形状（11 边 92 份）：六来源按序合并、规范 f32 点位去重；
/// `sumo::-25332_7_0` 为 #1134 新增 16 份。禁止旧 9/10 边池。
fn shared_pool_shape() -> BTreeMap<String, usize> {
    BTreeMap::from([
        ("sumo::-10004_5_1".to_owned(), 16),
        ("sumo::-10034_0_0".to_owned(), 8),
        ("sumo::-10130_8_0".to_owned(), 8),
        ("sumo::-10130_16_0".to_owned(), 8),
        ("sumo::-10130_18_0".to_owned(), 8),
        ("sumo::-16502_0_0".to_owned(), 1),
        ("sumo::-28860_3_0".to_owned(), 2),
        ("sumo::-29388_3_0".to_owned(), 8),
        ("sumo::-10564_3_0".to_owned(), 1),
        ("sumo::-25332_7_0".to_owned(), 16),
        ("sumo::-7348_3_0".to_owned(), 16),
    ])
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).expect("shared baseline input present")).unwrap()
}

#[test]
fn signal_diag_1_shared_pool_shape_matches_the_frozen_common_baseline() {
    // plan §5 冻结共同基线形状：11 边 92 份（含 #1134 sumo::-25332_7_0 的 16 份），
    // 每边份数逐值冻结；真实 pool.json 由 E0 冻结交付并在正式入口按钉版读取断言。
    let shape = shared_pool_shape();
    assert_eq!(shape.len(), 11);
    assert_eq!(shape.values().sum::<usize>(), 92);
    assert_eq!(shape["sumo::-25332_7_0"], 16);
    assert_eq!(shape["sumo::-10130_8_0"], 8);
    assert_eq!(shape["sumo::-10130_16_0"], 8);
    assert_eq!(shape["sumo::-10130_18_0"], 8);
    // 临时目录同字节内容验证：写入什么字节即读回什么字节；pool.json 行形状与 E0 冻结
    // 交付一致（`experiment::pool` 直接可消费）。
    let candidate = json!({"program":{"start":[0,0,0],
        "segments":[{"end":[10.0_f64.to_bits(),0,0],"canvas_selection":null}]},
        "arc_m":10.0,"spans":[]});
    let source = json!({"sumo::signal-diag-synthetic":{"candidates":[candidate]}});
    let bytes = serde_json::to_vec(&source).unwrap();
    let dir = std::env::temp_dir().join(format!("laneflow-signal-diag-1-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pool.json");
    fs::write(&path, &bytes).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes, "same-byte write and read");
    let loaded = read_json(&path);
    assert_eq!(loaded, source);
    assert_eq!(
        pool(&loaded)["sumo::signal-diag-synthetic"].len(),
        1,
        "pinned pool rows deserialize to complete programs"
    );
    fs::remove_dir_all(&dir).unwrap();
}
