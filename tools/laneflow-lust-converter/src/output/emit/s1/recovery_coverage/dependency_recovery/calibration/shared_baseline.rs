//! #1140：六路共同基线（11 边 92 份候选池）与最小公共测试助手（E0 交付）。
//!
//! 执行口径冻结于 `research/issue-1140-shared-baseline/plan.md`，共同口径沿六路协调
//! 章程 v2 与基线包合并需求单（Issue #1140 评论，含 A 路消费形状澄清）：包布局为
//! `pool.json`（与历史候选来源同构的行形状，`experiment::pool` 可直接消费）、
//! `results-anchor.json`（结果对照锚）与 `MANIFEST.json`（`files.<name>.sha256/bytes`
//! 与 `merge_order` 六项链），读取经 `SIX_STREAMS_BASELINE_DIR`，旧 9/10 边池禁用。
//! 合并语义逐字沿 `dependency_recovery::experiment::{pool, merge}`（规范 f32 点位
//! 去重、先到先留、每边上限 16）。本模块只新增研究入口与公共助手再导出，不改生产
//! 采样、compiler、阈值与原成功几何；计划与冻结实现不一致时停止执行并重新授权
//! （plan §7），不得单向迁就。

use super::super::experiment::{merge, pool};
use super::super::stream_a_curve_design_c29_c17::{per_edge_counts, pool_counts};
use super::cohort2_coverage::read_pinned_json;
use super::*;
use crate::output::digest::hex_sha256;
use std::collections::{BTreeMap, BTreeSet};

// ===== 公共助手最小集（基线包合并需求单；仅再导出，语义逐字不变）=====
// 词汇映射、载体三层链（fresh 两族 → 1107 history → repair_curve 重放回退）、
// 网格节点判定；池读取/断言由本模块 `shared_pools` 提供。
pub(in super::super) use super::coverage::{
    SpanControl, find_span_control, grid_node_member, plan_stop_label, replay_span_controls,
    span_control,
};

/// 需求单 merge_order 六项链（顺序不得更换）。
const MERGE_ORDER: [&str; 6] = ["1103", "1107", "1111", "1115", "1131", "1134"];

/// plan §1 逐级断言（边数/份数）：6/48 → 7/56 → 7/72 → 9/75 → 10/76 → 11/92。
const PER_STAGE_COUNTS: [[usize; 2]; 6] = [[6, 48], [7, 56], [7, 72], [9, 75], [10, 76], [11, 92]];

/// plan §1 最终逐边份数（11 边 92 份逐值，BTreeMap 键序）。
const FINAL_PER_EDGE: [(&str, usize); 11] = [
    ("sumo::-10004_5_1", 16),
    ("sumo::-10034_0_0", 8),
    ("sumo::-10130_16_0", 8),
    ("sumo::-10130_18_0", 8),
    ("sumo::-10130_8_0", 8),
    ("sumo::-10564_3_0", 1),
    ("sumo::-16502_0_0", 1),
    ("sumo::-25332_7_0", 16),
    ("sumo::-28860_3_0", 2),
    ("sumo::-29388_3_0", 8),
    ("sumo::-7348_3_0", 16),
];

/// 来源⑤钉版（#1134 plan §5）：Release 资产 `research-evidence-1134-cohort2-baseline`
/// → `cohort2-baseline-inputs.zip::stream-a-searches.json`，SHA-256 与字节数冻结。
const STREAM_A_SEARCHES_SHA256: &str =
    "1c6dd16244481d8fa757865305b2f05287ed74624dde5770998e752d18e26d7b";

/// 结果对照锚取材钉版（锚代际错位事故返工后）：#1134 run-03 `results.json`
/// （631,650 字节）`#variants.new` 整行 = 92 池行。
const RUN3_RESULTS_SHA256: &str =
    "b320cfe9452edb87cd17a2186338fc0fef077ba141c937a626c56686b970c25f";

/// 资产钉版（`shared-baseline-inputs.zip` 三成员；plan §2 同步钉版）。
const POOL_JSON_SHA256: &str = "31fca69617e2fd5d818aafc64b6704451475e05bf3aa3918391e1a2f9feb1bb3";
const RESULTS_ANCHOR_SHA256: &str =
    "0973dd355f18be878b977f3e2caea65ba355aeb7f1d0282e624e4b62b8b1abe5";
const MANIFEST_SHA256: &str = "56edd1365c48ce68b39e260f5be01ecea8629ee92e424d58363bd718e2d00cf5";

/// 来源⑥钉版（#1134 冻结四轮 ABBA 逐字节一致的 run-03 输出；两文件各 157,603 /
/// 1,313,285 字节）。
const RUN3_OLD_SEARCHES_SHA256: &str =
    "011db49fad061d945ab7d21858d73961d8234e2955450fd4f040269d04309dba";
const RUN3_GRID_SEARCHES_SHA256: &str =
    "7b8b2e1f28f1d7e47aed6ff5222086971a1ae2838dad07559ff03efd94513fe9";

/// 候选去重键：整程序点位的规范 f32 位串（逐字沿 `experiment::merge`）。
fn canonical_key(program: &re::RoadEditingCurveProgram) -> Vec<[u32; 3]> {
    program_points(program)
        .into_iter()
        .map(|v| v.map(|x| (x as f32).to_bits()))
        .collect()
}

/// plan §1 六来源合并的机械过程：规范 f32 点位去重（先到先留、每边上限 16）、合并
/// 顺序保留。输出：① `pool.json` 行形状（`{edge: {"candidates": [候选原值]}}`，与历史
/// 候选来源同构、`experiment::pool` 可直接消费）；② 逐份来源标记（MANIFEST
/// `candidate_sources`：每边按包内顺序记首次引入来源）；③ 逐级计数。`sources` 为逐段
/// 输入 `(stage, rows)`；同一来源多文件（来源⑥原族/网格两份）只记一个断言级。
fn build_pool(sources: &[(usize, &Value)]) -> (Value, Value, Value) {
    let mut rows: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut provenance: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut seen: BTreeMap<String, BTreeSet<Vec<[u32; 3]>>> = BTreeMap::new();
    let mut per_stage = Vec::new();
    let mut stage_open = 0;
    for (stage, source) in sources {
        if *stage != stage_open {
            if stage_open != 0 {
                let edges = seen.len();
                let candidates = seen.values().map(BTreeSet::len).sum::<usize>();
                per_stage.push(json!([edges, candidates]));
            }
            stage_open = *stage;
        }
        for (id, row) in source.as_object().unwrap() {
            for candidate in row["candidates"].as_array().unwrap() {
                let program = successors::frozen_candidate(candidate).repaired.program;
                let key = canonical_key(&program);
                let keys = seen.entry(id.clone()).or_default();
                if keys.insert(key) {
                    assert!(keys.len() <= 16, "bounded merged pool");
                    rows.entry(id.clone()).or_default().push(candidate.clone());
                    provenance.entry(id.clone()).or_default().push(*stage);
                }
            }
        }
    }
    let edges = seen.len();
    let candidates = seen.values().map(BTreeSet::len).sum::<usize>();
    per_stage.push(json!([edges, candidates]));
    let rows = json!(
        rows.into_iter()
            .map(|(id, candidates)| (id, json!({"candidates":candidates})))
            .collect::<BTreeMap<String, Value>>()
    );
    let provenance = json!(provenance);
    (rows, provenance, json!(per_stage))
}

/// plan §3 冻结链断言：逐级计数、11 边逐边份数、总数 92（与 MANIFEST 交叉一致）。
fn assert_frozen_chain(rows: &Value, manifest: &Value) {
    assert_eq!(
        manifest["merge_order"],
        json!(MERGE_ORDER),
        "frozen merge order"
    );
    assert_eq!(
        manifest["per_stage_counts"],
        json!(PER_STAGE_COUNTS),
        "frozen per-stage counts"
    );
    assert_eq!(
        manifest["final"]["edge_count"],
        FINAL_PER_EDGE.len(),
        "manifest edge count"
    );
    assert_eq!(
        manifest["final"]["candidate_count"], 92,
        "manifest candidate count"
    );
    let edges = rows.as_object().unwrap();
    assert_eq!(edges.len(), FINAL_PER_EDGE.len(), "frozen edge count");
    let mut total = 0;
    for (id, count) in FINAL_PER_EDGE {
        let candidates = edges.get(id).unwrap_or_else(|| panic!("frozen edge {id}"))["candidates"]
            .as_array()
            .unwrap();
        assert_eq!(candidates.len(), count, "frozen per-edge count {id}");
        assert_eq!(
            manifest["final"]["per_edge"][id], count,
            "manifest per-edge count {id}"
        );
        total += count;
    }
    assert_eq!(total, 92, "frozen candidate count");
}

/// plan §3 ④：包内候选逐份往返校验——`program` 位逐字经 `frozen_candidate` 往返、
/// 规范 f32 点位键逐边唯一（由程序点位重算，包内不另存）、每边 ≤16。校验通过后返回
/// 可消费的 `combinations::Pools`（程序顺序 = 冻结合并顺序）。
fn verified_pools(rows: &Value) -> combinations::Pools {
    let mut pools = combinations::Pools::new();
    for (id, row) in rows.as_object().unwrap() {
        let candidates = row["candidates"].as_array().unwrap();
        assert!(candidates.len() <= 16, "bounded merged pool {id}");
        let mut keys = BTreeSet::new();
        let mut programs = Vec::new();
        for candidate in candidates {
            let program = successors::frozen_candidate(candidate).repaired.program;
            assert!(
                keys.insert(canonical_key(&program)),
                "unique canonical key {id}"
            );
            programs.push(program);
        }
        pools.insert(id.clone(), programs);
    }
    pools
}

/// plan §2 结果对照锚（锚代际错位事故返工，取材身份三件套钉版）：#1134 run-03
/// `results.json#variants.new` **整行逐值**（92 池行：53 分量、84 路径、接受 28/84）；
/// **仅结果对照，不作候选点位来源**。代际守卫：C40 组件行 `missing_edges`=5
/// （76 池旧代为 6，拒收）。
fn results_anchor(run_results: &Value) -> Value {
    let variant = &run_results["variants"]["new"];
    let c40 = variant["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["component"] == "sumo:-25332:--32278#0_1-to--32770#7_1::-25332_0_1")
        .expect("C40 cohort component row present");
    assert_eq!(
        c40["result"]["missing_edges"].as_array().unwrap().len(),
        5,
        "anchor generation: 92-pool row (C40 missing_edges=5)"
    );
    assert_eq!(
        variant["components"].as_array().unwrap().len(),
        53,
        "anchor carries 53 components"
    );
    assert_eq!(
        variant["paths"].as_array().unwrap().len(),
        84,
        "anchor carries 84 paths"
    );
    assert_eq!(variant["path_counts"]["accepted"], 28, "anchor accepts 28");
    assert_eq!(
        variant["path_counts"]["cross_edge_direction_rejected"], 3,
        "anchor rejected count"
    );
    assert_eq!(
        variant["path_counts"]["target_check_not_completed"], 53,
        "anchor unfinished count"
    );
    let mut anchor = json!({});
    anchor["source"] = json!({
        "asset":"evidence/run-03/results.json","sha256":RUN3_RESULTS_SHA256,"bytes":631650,
        "json_pointer":"#variants.new","generation":"92-池行",
        "release_tag":"research-evidence-1140-shared-baseline","comparison_only":true,
        "changed_from":{
            "asset":"stream-a-results.json","json_pointer":"#variants.new",
            "release_tag":"research-evidence-1134-cohort2-baseline","generation":"76-池行",
            "reason":"锚代际错位事故：原取材 #1131 代 stream-a-results.json（76 池行）；章程 §1 要求 #1134 新增变体 = run-03 results.json#variants.new（92 池行）；两代唯一语义差为 C40 missing_edges 6→5",
        },
    });
    // `variants.new` 整行逐值展开（`source` 与行内 7 键无冲突）。
    for (key, value) in variant.as_object().unwrap() {
        anchor[key] = value.clone();
    }
    anchor
}

/// plan §1 来源装载：①–④为仓库内历史切片研究制品（include_str 钉版），⑤经
/// `COHORT2_BASELINE_DIR` 钉版读取，⑥经 `COHORT2_RUN_DIR` 读取 #1134 run-03 输出
/// （仅 `sumo::-25332_7_0` 有候选，原族 8 份在前、网格 8 份在后）。
fn source_rows(stream_searches: Value, run_old: Value, run_grid: Value) -> Vec<(usize, Value)> {
    let legacy: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1103-s1-recovery-budget/results.json"
    ))
    .unwrap();
    let old: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1107-dependency-recovery/old-searches.json"
    ))
    .unwrap();
    let grid: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1111-grid-calibration/positive-calibration.json"
    ))
    .unwrap();
    let curve: Value = serde_json::from_str(include_str!(
        "../../../../../../../../../research/issue-1115-failed-curve-design/searches.json"
    ))
    .unwrap();
    // 来源⑥只允许 sumo::-25332_7_0 出候选（合并链冻结）。
    for (label, rows) in [("run3-old", &run_old), ("run3-grid", &run_grid)] {
        for (id, row) in rows.as_object().unwrap() {
            if id != "sumo::-25332_7_0" {
                assert!(
                    row["candidates"].as_array().unwrap().is_empty(),
                    "{label} carries no candidates outside sumo::-25332_7_0"
                );
            }
        }
    }
    vec![
        (1, legacy["tiers"]["32768"]["searches"].clone()),
        (2, old),
        (3, grid["edges"].clone()),
        (4, curve),
        (5, stream_searches),
        (6, run_old),
        (6, run_grid),
    ]
}

/// 来源⑤/⑥与结果对照锚的钉版输入装载（`COHORT2_BASELINE_DIR` + `COHORT2_RUN_DIR`；
/// 锚取材 = run-03 `results.json`，锚代际错位事故返工后）。
fn pinned_sources() -> (Value, Value, Value, Value) {
    let base = PathBuf::from(std::env::var("COHORT2_BASELINE_DIR").expect("COHORT2_BASELINE_DIR"));
    let run = PathBuf::from(std::env::var("COHORT2_RUN_DIR").expect("COHORT2_RUN_DIR"));
    let stream_searches = read_pinned_json(
        &base.join("stream-a-searches.json"),
        STREAM_A_SEARCHES_SHA256,
    );
    let run_results = read_pinned_json(&run.join("results.json"), RUN3_RESULTS_SHA256);
    let run_old = read_pinned_json(&run.join("old-searches.json"), RUN3_OLD_SEARCHES_SHA256);
    let run_grid = read_pinned_json(&run.join("grid-searches.json"), RUN3_GRID_SEARCHES_SHA256);
    (stream_searches, run_results, run_old, run_grid)
}

/// plan §3 ⑤ 六来源重建与包内候选逐值逐序对照（`experiment::{pool, merge}` 双记账）。
fn assert_rebuild_matches_package(rows: &Value, sources: &[(usize, Value)]) {
    let mut pools = combinations::Pools::new();
    for (_, source) in sources {
        merge(&mut pools, pool(source));
    }
    assert_eq!(pool_counts(&pools), (11, 92), "rebuild pool totals");
    assert_eq!(
        per_edge_counts(&pools),
        BTreeMap::from(FINAL_PER_EDGE.map(|(id, count)| (id.to_owned(), count))),
        "rebuild per-edge counts"
    );
    for (id, programs) in &pools {
        let candidates = rows[id]["candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), programs.len(), "rebuild length {id}");
        for (candidate, program) in candidates.iter().zip(programs) {
            assert_eq!(
                candidate["program"],
                json!(program_bits(program)),
                "rebuild candidate order {id}"
            );
        }
    }
}

fn pinned_baseline_dir() -> PathBuf {
    PathBuf::from(std::env::var("SIX_STREAMS_BASELINE_DIR").expect("SIX_STREAMS_BASELINE_DIR"))
}

/// 需求单「池读取/断言」入口：A–D 唯一基线读取点。三资产 SHA-256 钉版断言 + 代码内
/// 哈希/合并顺序/逐边份数断言 + 逐份往返校验后返回 `combinations::Pools`
/// （旧 9/10 边池禁用）。
pub(in super::super) fn shared_pools() -> combinations::Pools {
    let dir = pinned_baseline_dir();
    let rows = read_pinned_json(&dir.join("pool.json"), POOL_JSON_SHA256);
    let _anchor = read_pinned_json(&dir.join("results-anchor.json"), RESULTS_ANCHOR_SHA256);
    let manifest = read_pinned_json(&dir.join("MANIFEST.json"), MANIFEST_SHA256);
    for (name, expected) in [
        ("pool.json", POOL_JSON_SHA256),
        ("results-anchor.json", RESULTS_ANCHOR_SHA256),
    ] {
        assert_eq!(
            manifest["files"][name]["sha256"], expected,
            "manifest files.{name}.sha256"
        );
        assert_eq!(
            manifest["files"][name]["bytes"],
            fs::read(dir.join(name)).unwrap().len(),
            "manifest files.{name}.bytes"
        );
    }
    assert_frozen_chain(&rows, &manifest);
    verified_pools(&rows)
}

/// 需求单「结果对照锚」读取（仅对照，不作候选点位来源）。取材身份三件套
/// （asset/json_pointer/release_tag）+ 代际标签断言（A 路入口同款钉版断言）。
pub(in super::super) fn results_anchor_pinned() -> Value {
    let dir = pinned_baseline_dir();
    let anchor = read_pinned_json(&dir.join("results-anchor.json"), RESULTS_ANCHOR_SHA256);
    let source = &anchor["source"];
    assert_eq!(source["comparison_only"], true, "comparison-only anchor");
    assert_eq!(
        source["asset"], "evidence/run-03/results.json",
        "anchor source asset"
    );
    assert_eq!(
        source["json_pointer"], "#variants.new",
        "anchor source json_pointer"
    );
    assert_eq!(
        source["release_tag"], "research-evidence-1140-shared-baseline",
        "anchor source release_tag"
    );
    assert_eq!(source["generation"], "92-池行", "anchor generation label");
    assert_eq!(
        source["sha256"], RUN3_RESULTS_SHA256,
        "anchor source digest"
    );
    anchor
}

/// plan §5 导出入口：按合并链重建 92 份池并写三资产（结果 JSON 无计时字段）。
#[test]
#[ignore = "requires fresh SHARED_BASELINE_EXPORT_DIR, COHORT2_BASELINE_DIR and COHORT2_RUN_DIR"]
fn shared_baseline_export() {
    let out = PathBuf::from(std::env::var("SHARED_BASELINE_EXPORT_DIR").expect("export dir"));
    assert!(!out.exists(), "fresh export directory");
    fs::create_dir_all(&out).unwrap();
    let (stream_searches, run_results, run_old, run_grid) = pinned_sources();
    let rows = source_rows(stream_searches, run_old, run_grid);
    let sources: Vec<(usize, &Value)> = rows
        .iter()
        .map(|(stage, source)| (*stage, source))
        .collect();
    let (pool_rows, provenance, per_stage) = build_pool(&sources);
    let anchor = results_anchor(&run_results);
    recovery::write_json(&out, "pool.json", &pool_rows);
    recovery::write_json(&out, "results-anchor.json", &anchor);
    let pool_bytes = fs::read(out.join("pool.json")).unwrap();
    let anchor_bytes = fs::read(out.join("results-anchor.json")).unwrap();
    let mut manifest_sources = Vec::new();
    for (index, (stage, source)) in rows.iter().enumerate() {
        let candidates = source
            .as_object()
            .unwrap()
            .values()
            .map(|row| row["candidates"].as_array().unwrap().len())
            .sum::<usize>();
        manifest_sources.push(json!({
            "merge_order_index":index,"stage":stage,
            "source_label":MERGE_ORDER[stage - 1],
            "raw_candidate_count":candidates,
        }));
    }
    let manifest = json!({
        "schema":"laneflow-1140-shared-baseline-manifest",
        "files":{
            "pool.json":{"sha256":hex_sha256(&pool_bytes),"bytes":pool_bytes.len()},
            "results-anchor.json":{"sha256":hex_sha256(&anchor_bytes),"bytes":anchor_bytes.len()},
        },
        "merge_order":MERGE_ORDER,"per_stage_counts":per_stage,
        "final":{"edge_count":FINAL_PER_EDGE.len(),"candidate_count":92,
            "per_edge":FINAL_PER_EDGE.iter().map(|(id,count)|(id.to_string(),count)).collect::<BTreeMap<String,_>>()},
        "sources":manifest_sources,
        "candidate_sources":provenance,
        "anchor_source":{
            "asset":"evidence/run-03/results.json","sha256":RUN3_RESULTS_SHA256,"bytes":631650,
            "json_pointer":"#variants.new","generation":"92-池行",
            "release_tag":"research-evidence-1140-shared-baseline",
            "changed_from":"#variants.new@stream-a-results.json（research-evidence-1134-cohort2-baseline，76-池行）→ #variants.new@evidence/run-03/results.json（92-池行）；根因=锚代际错位事故，两代唯一语义差为 C40 missing_edges 6→5",
        },
        "generation":"E0 export entry shared_baseline_export; pool merge semantics = dependency_recovery::experiment::{pool,merge}; pool.json rows are source-shaped ({edge:{candidates:[…]}}); results anchor = run-03 results.json#variants.new 整行（92-池行，锚代际错位事故返工后取材），comparison-only",
    });
    // 冻结链断言先于写 MANIFEST（per_stage/ final 与 build 产物交叉一致）。
    assert_frozen_chain(&pool_rows, &manifest);
    assert_rebuild_matches_package(&pool_rows, &rows);
    recovery::write_json(&out, "MANIFEST.json", &manifest);
}

/// plan §5 校验入口：`SIX_STREAMS_BASELINE_DIR` 三资产全断言 + 六来源重建逐值逐序 +
/// 结果对照锚与 `run-03 results.json#variants.new` 整行逐值（92 池行）。
#[test]
#[ignore = "requires SIX_STREAMS_BASELINE_DIR, COHORT2_BASELINE_DIR and COHORT2_RUN_DIR"]
fn shared_baseline_package_matches_frozen_chain() {
    let pools = shared_pools();
    assert_eq!(pool_counts(&pools), (11, 92), "verified pool totals");
    let dir = pinned_baseline_dir();
    let rows = read_pinned_json(&dir.join("pool.json"), POOL_JSON_SHA256);
    let (stream_searches, run_results, run_old, run_grid) = pinned_sources();
    let rows_sources = source_rows(stream_searches, run_old, run_grid);
    assert_rebuild_matches_package(&rows, &rows_sources);
    let anchor = results_anchor_pinned();
    assert_eq!(anchor, results_anchor(&run_results), "anchor 逐值");
}

/// 临时目录唯一命名；钉版读取验证的同字节内容只写临时目录，不落仓库。
fn temp_probe_dir() -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "laneflow-shared-baseline-{}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn synthetic_candidate(end_x: f64, arc_m: f64) -> Value {
    json!({"program":{"start":[0,0,0],
        "segments":[{"end":[end_x.to_bits(),0,0],"canvas_selection":null}]},
        "arc_m":arc_m,"spans":[]})
}

#[test]
fn shared_baseline_merge_dedups_canonical_points_in_frozen_order() {
    // 合并机械：规范 f32 点位去重、先到先留（重复项不覆盖首份原值）、合并顺序与逐份
    // 来源标记保留、逐级计数记录；pool.json 行形状与历史候选来源同构。
    let a = synthetic_candidate(10.0, 10.0);
    let b = synthetic_candidate(20.0, 20.0);
    let dup = json!({"program":a["program"],"arc_m":99.0,"spans":[{"probe":1}]});
    let c = synthetic_candidate(30.0, 30.0);
    let source_1 = json!({"sumo::e0-synthetic":{"candidates":[a.clone(), b.clone()]}});
    let source_2 = json!({"sumo::e0-synthetic":{"candidates":[dup, c.clone()]}});
    let (rows, provenance, per_stage) = build_pool(&[(1, &source_1), (2, &source_2)]);
    assert_eq!(per_stage, json!([[1, 2], [1, 3]]), "per-stage counts");
    let candidates = rows["sumo::e0-synthetic"]["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 3, "duplicate canonical key dropped");
    assert_eq!(
        candidates[0], a,
        "source-shaped rows; first representative wins"
    );
    assert_eq!(candidates[1], b, "order preserved");
    assert_eq!(candidates[2], c, "order preserved");
    assert_eq!(
        provenance["sumo::e0-synthetic"],
        json!([1, 1, 2]),
        "per-candidate provenance"
    );
    let pools = verified_pools(&rows);
    assert_eq!(pool_counts(&pools), (1, 3), "verified synthetic pool");
}

#[test]
fn shared_baseline_facade_exposes_required_helper_set() {
    // 需求单最小集门面冒烟 + 契约签名钉版（语义由既有 coverage/cohort2 单测覆盖）：
    // 词汇映射、网格节点、载体三层链三构件与 SpanControl 结果形态。
    let _: fn(&Value, u64) -> &'static str = plan_stop_label;
    let _: fn(f64) -> bool = grid_node_member;
    let _: fn(&Value, usize) -> Option<[[f64; 3]; 6]> = find_span_control;
    let _: fn(&observe::EdgeInput) -> BTreeMap<usize, [[f64; 3]; 6]> = replay_span_controls;
    let _: fn(&BTreeMap<String, Value>, &Value, &str, usize, &observe::EdgeInput) -> SpanControl =
        span_control;
}

#[test]
#[should_panic(expected = "pinned baseline input digest")]
fn shared_baseline_loader_rejects_unpinned_bytes() {
    let dir = temp_probe_dir();
    let path = dir.join("pool.json");
    fs::write(&path, b"not the pinned shared baseline bytes").unwrap();
    let _ = read_pinned_json(&path, POOL_JSON_SHA256);
}

#[test]
#[should_panic(expected = "anchor generation")]
fn shared_baseline_anchor_rejects_old_generation_row() {
    // 锚代际守卫（锚代际错位事故返工）：76 池旧代行（C40 missing_edges=6）拒收，
    // 只接受 92 池行（C40 missing_edges=5）。
    let missing = vec![json!({"edge":"sumo::e0-synthetic"}); 6];
    let run_results = json!({"variants":{"new":{
        "components":[{
            "component":"sumo:-25332:--32278#0_1-to--32770#7_1::-25332_0_1",
            "result":{"missing_edges":missing},
        }],
    }}});
    let _ = results_anchor(&run_results);
}
