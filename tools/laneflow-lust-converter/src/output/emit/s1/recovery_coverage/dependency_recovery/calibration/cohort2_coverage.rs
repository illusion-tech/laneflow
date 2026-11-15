//! #1134：第 2 批队列（11 个非信号分量、22 条缺边）的固定曲线覆盖补齐、逐边局部
//! 必要条件诊断与候选齐备分量组合验收。执行口径全部冻结于
//! `research/issue-1134-cohort2-coverage-diagnosis/plan.md`，方法事实来源
//! `research/issue-1121-coverage-diagnosis/plan.md` 逐值沿用；本模块不改生产采样、
//! compiler 配置档、阈值与原成功几何。计划与冻结实现不一致时停止执行并重新授权
//! （计划 §7），不得单向迁就。

use super::super::experiment::{merge, pool};
use super::super::stream_a_curve_design_c29_c17::{per_edge_counts, pool_counts};
use super::coverage::span_control;
use super::*;
use crate::output::digest::hex_sha256;

/// plan §5 钉版基线输入（Release 资产 `research-evidence-1134-cohort2-baseline`，
/// 资产 `cohort2-baseline-inputs.zip`）：SHA-256 与字节数冻结，身份不依赖仓库内文件。
const STREAM_A_SEARCHES_SHA256: &str =
    "1c6dd16244481d8fa757865305b2f05287ed74624dde5770998e752d18e26d7b";

/// 读取钉版基线输入并在代码内断言 SHA-256 与 plan §5 钉版值一致（发行资产可验证为
/// 冻结输入）；写入什么字节即校验与解析什么字节。（#1140：仅可见性放宽供
/// `calibration::shared_baseline` 复用，语义逐字不变。）
pub(super) fn read_pinned_json(path: &Path, expected_sha: &str) -> Value {
    let bytes = fs::read(path).expect("pinned baseline input present");
    assert_eq!(
        hex_sha256(&bytes),
        expected_sha,
        "pinned baseline input digest"
    );
    serde_json::from_slice(&bytes).unwrap()
}

/// plan §5 冻结基线候选池：来源①→④（仓库内历史批次研究制品）按固定顺序合并并逐级
/// 断言（6/48→7/56→7/72→9/75），再合并 Release 资产 `stream-a-searches.json` 的新增
/// 候选（来源⑤，钉版输入）；总断言（10 边 76 份与每边份数）由 `pinned_baseline_pool`
/// 按钉版输入执行。任一来源缺失或逐值不符即停止执行并重新授权。
fn baseline_pool(stream_searches: &Value) -> combinations::Pools {
    let legacy: Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/issue-1103-s1-recovery-budget-results.json"
    ))
    .unwrap();
    let old: Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/issue-1107-old-searches.json"
    ))
    .unwrap();
    let grid: Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/issue-1111-positive-calibration.json"
    ))
    .unwrap();
    let curve: Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/issue-1115-searches.json"
    ))
    .unwrap();
    let mut pools = pool(&legacy["tiers"]["32768"]["searches"]);
    assert_eq!(
        pool_counts(&pools),
        (6, 48),
        "source 1: 6 edges 48 candidates"
    );
    merge(&mut pools, pool(&old));
    assert_eq!(
        pool_counts(&pools),
        (7, 56),
        "source 2: 7 edges 56 candidates"
    );
    merge(&mut pools, pool(&grid["edges"]));
    assert_eq!(
        pool_counts(&pools),
        (7, 72),
        "source 3: 7 edges 72 candidates"
    );
    merge(&mut pools, pool(&curve));
    assert_eq!(
        pool_counts(&pools),
        (9, 75),
        "source 4: 9 edges 75 candidates"
    );
    merge(&mut pools, pool(stream_searches));
    pools
}

/// 临时目录唯一命名；基线读取验证的同字节内容只写临时目录，不落仓库。
fn temp_probe_dir() -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "laneflow-cohort2-baseline-{}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn cohort2_baseline_pool_merges_frozen_sources_with_pinned_input_bytes() {
    // 来源①→④为 crate fixture 历史批次研究制品，逐级断言 6/48→7/56→7/72→9/75；来源⑤按
    // plan §5 为 Release 资产钉版输入——本测试以临时目录写入同字节内容验证读取、
    // 钉版断言与合并路径；真实 10 边 76 份与每边份数总断言由 `pinned_baseline_pool`
    // 在正式实验入口按钉版输入执行。
    let candidate = json!({"program":{"start":[0,0,0],
        "segments":[{"end":[10.0_f64.to_bits(),0,0],"canvas_selection":null}]},
        "arc_m":10.0,"spans":[]});
    let source = json!({"sumo::cohort2-synthetic":{"candidates":[candidate]}});
    let bytes = serde_json::to_vec(&source).unwrap();
    let dir = temp_probe_dir();
    let path = dir.join("stream-a-searches.json");
    fs::write(&path, &bytes).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes, "same-byte write and read");
    let loaded = read_pinned_json(&path, &hex_sha256(&bytes));
    assert_eq!(loaded, source, "pinned loader parses the written bytes");
    let pools = baseline_pool(&loaded);
    assert_eq!(
        pool_counts(&pools),
        (10, 76),
        "sources 1-4 chain plus source 5"
    );
    assert_eq!(per_edge_counts(&pools)["sumo::cohort2-synthetic"], 1);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[should_panic(expected = "pinned baseline input digest")]
fn cohort2_pinned_baseline_loader_rejects_unpinned_bytes() {
    let dir = temp_probe_dir();
    let path = dir.join("stream-a-searches.json");
    fs::write(&path, b"not the pinned baseline bytes").unwrap();
    let _ = read_pinned_json(&path, STREAM_A_SEARCHES_SHA256);
}

#[test]
fn cohort2_span_control_fallback_preserves_existing_carriers() {
    let inputs: Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/issue-1107-inputs.json"
    ))
    .unwrap();
    let grid_history: Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/issue-1107-grid-searches.json"
    ))
    .unwrap();
    let empty = BTreeMap::new();
    // 既有载体（历史 1107 记录）可解析的诊断 span：三层链加入回退后逐值不变、来源仍为
    // history；重放只作交叉核对列。fresh 报告载体的 12 条边由正式实验逐问题断言同口径。
    for (id, span) in [
        ("sumo::-10100_1_0", 0),
        ("sumo::-1426_2_0", 2),
        ("sumo::-19726_2_0", 1),
        ("sumo::-22042_7_0", 3),
    ] {
        let input = measurement::input(&inputs[id]);
        let before = coverage::find_span_control(&grid_history[id]["search"], span)
            .expect("existing history carrier");
        let resolved = span_control(&empty, &grid_history, id, span, &input);
        assert_eq!(
            resolved.source, "history",
            "{id} keeps the existing carrier"
        );
        assert_eq!(
            resolved.control, before,
            "{id} control unchanged by fallback"
        );
        let replay = resolved
            .replay_control
            .expect("replay reaches the diagnosis span");
        assert_eq!(
            replay[..4],
            resolved.control[..4],
            "{id} replay cross-check"
        );
    }
    // 既有载体缺失的两条边（sumo::-10034_4_0、sumo::-1426_1_0，均 span 0）由重放回退
    // 解析（#1134 §7 重新授权）；注入匹配用完整六点，不再零填充。
    for (id, span) in [("sumo::-10034_4_0", 0), ("sumo::-1426_1_0", 0)] {
        let input = measurement::input(&inputs[id]);
        let resolved = span_control(&empty, &grid_history, id, span, &input);
        assert_eq!(
            resolved.source, "replay_fallback",
            "{id} uses the replay fallback"
        );
        assert_eq!(
            Some(resolved.control),
            resolved.replay_control,
            "{id} replay"
        );
        assert_eq!(
            resolved.control, resolved.injection_control,
            "{id} complete six points"
        );
        assert!(
            resolved.control[4] != [0.0, 0.0, 0.0] && resolved.control[5] != [0.0, 0.0, 0.0],
            "{id} replay carries real endpoint tangents"
        );
        for point in resolved.control {
            for value in point {
                assert!(
                    value.is_finite() && value.abs() <= 32_000.0,
                    "{id} frozen control bounds"
                );
            }
        }
    }
}
