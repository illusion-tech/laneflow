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

// ===== 文本拼接原语（位忠实导出；数值字符串逐字保留，不做浮点再序列化）=====
// 背景（README 差异记录「导出路径有损再序列化（1 ULP）」）：经浮点解析/写出的
// 数值字符串存在 ±1 ULP 漂移（如 `10.220051765441895` → `10.220051765441896`）。
// 锚导出按字节从源文本切片，数字原文逐字保留。

/// 跳过 JSON 空白。
fn skip_ws(text: &[u8], pos: &mut usize) {
    while *pos < text.len() && matches!(text[*pos], b' ' | b'\t' | b'\n' | b'\r') {
        *pos += 1;
    }
}

/// 跳过一个 JSON 值（对象/数组/字符串/数字/字面量），`pos` 停在值后。
fn skip_json_value(text: &[u8], pos: &mut usize) {
    match text[*pos] {
        b'{' | b'[' => {
            let (open, close) = if text[*pos] == b'{' {
                (b'{', b'}')
            } else {
                (b'[', b']')
            };
            let mut depth = 0usize;
            let mut in_string = false;
            let mut escape = false;
            loop {
                let byte = text[*pos];
                if in_string {
                    if escape {
                        escape = false;
                    } else if byte == b'\\' {
                        escape = true;
                    } else if byte == b'"' {
                        in_string = false;
                    }
                } else {
                    match byte {
                        b'"' => in_string = true,
                        b if b == open => depth += 1,
                        b if b == close => {
                            depth -= 1;
                            if depth == 0 {
                                *pos += 1;
                                return;
                            }
                        }
                        _ => {}
                    }
                }
                *pos += 1;
            }
        }
        b'"' => {
            *pos += 1;
            let mut escape = false;
            loop {
                match text[*pos] {
                    b'\\' if !escape => escape = true,
                    b'"' if !escape => {
                        *pos += 1;
                        return;
                    }
                    _ => escape = false,
                }
                *pos += 1;
            }
        }
        _ => {
            while *pos < text.len()
                && !matches!(
                    text[*pos],
                    b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r'
                )
            {
                *pos += 1;
            }
        }
    }
}

/// 解析一个 JSON 字符串键（`pos` 在 `"` 上），返回键内容字节切片并前进到 `:` 前。
fn json_key<'a>(text: &'a [u8], pos: &mut usize) -> &'a [u8] {
    let start = *pos;
    skip_json_value(text, pos);
    &text[start + 1..*pos - 1]
}

/// 按键路径定位对象内部字节区间（`key_path` 最后一个键所在的对象）：逐键匹配路径并
/// 深入，返回该对象 `{`/`}` 之间的内部区间（不含大括号）。
fn json_object_interior(text: &[u8], start: usize, key_path: &[&str]) -> (usize, usize) {
    let mut pos = start;
    assert_eq!(text[pos], b'{', "object expected");
    pos += 1;
    loop {
        skip_ws(text, &mut pos);
        assert!(pos < text.len(), "object not terminated");
        if text[pos] == b'}' {
            panic!("json key {:?} not found", key_path.first());
        }
        assert_eq!(text[pos], b'"', "object key expected");
        let key = json_key(text, &mut pos);
        skip_ws(text, &mut pos);
        assert_eq!(text[pos], b':', "key separator expected");
        pos += 1;
        skip_ws(text, &mut pos);
        if key == key_path[0].as_bytes() {
            if key_path.len() == 1 {
                assert_eq!(text[pos], b'{', "target value must be an object");
                let inner_start = pos + 1;
                let mut probe = pos;
                skip_json_value(text, &mut probe);
                return (inner_start, probe - 1);
            }
            return json_object_interior(text, pos, &key_path[1..]);
        }
        skip_json_value(text, &mut pos);
        skip_ws(text, &mut pos);
        if text[pos] == b',' {
            pos += 1;
        }
    }
}

/// 数值字面量逐个提取（位级比较用；不含字符串内数字）。
fn json_numeric_literals(text: &[u8]) -> Vec<&[u8]> {
    let mut literals = Vec::new();
    let mut pos = 0;
    let mut in_string = false;
    let mut escape = false;
    while pos < text.len() {
        let byte = text[pos];
        if in_string {
            if escape {
                escape = false;
            } else if byte == b'\\' {
                escape = true;
            } else if byte == b'"' {
                in_string = false;
            }
            pos += 1;
        } else if byte == b'"' {
            in_string = true;
            pos += 1;
        } else if byte == b'-' || byte.is_ascii_digit() {
            let start = pos;
            while pos < text.len()
                && matches!(text[pos], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
            {
                pos += 1;
            }
            literals.push(&text[start..pos]);
        } else {
            pos += 1;
        }
    }
    literals
}

/// 资产钉版（`shared-baseline-inputs.zip` 三成员；plan §2 同步钉版）。
const POOL_JSON_SHA256: &str = "31fca69617e2fd5d818aafc64b6704451475e05bf3aa3918391e1a2f9feb1bb3";
const RESULTS_ANCHOR_SHA256: &str =
    "d025c4bf6cd2a6c815e9586124034d21c8232d6c9cb46266ad0a5727e658847b";
const MANIFEST_SHA256: &str = "656fb97e66f0b8829664082a9e231c8eef35cb129d5496ddc3262de2375a2bcc";

/// 来源⑥钉版（#1134 冻结四轮 ABBA 逐字节一致的 run-03 输出；两文件各 157,603 /
/// 1,313,285 字节）。
const RUN3_OLD_SEARCHES_SHA256: &str =
    "011db49fad061d945ab7d21858d73961d8234e2955450fd4f040269d04309dba";
const RUN3_GRID_SEARCHES_SHA256: &str =
    "7b8b2e1f28f1d7e47aed6ff5222086971a1ae2838dad07559ff03efd94513fe9";

/// program 对象 u64 位逐个提取（lead ④ 裁决硬门 1：几何/判定唯一权威输入的位级断言）。
/// 入参为 `{"start":[…],"segments":[{"end":[…]}]}` 形态的 program 对象。
fn program_u64_bits(program_value: &Value) -> Vec<u64> {
    let mut bits = Vec::new();
    for value in program_value["start"].as_array().unwrap() {
        bits.push(value.as_u64().unwrap());
    }
    for segment in program_value["segments"].as_array().unwrap() {
        for value in segment["end"].as_array().unwrap() {
            bits.push(value.as_u64().unwrap());
        }
    }
    bits
}

/// JSON 文本叶 token（文本 token 域位级比较原语；serde_json 1.0.154 默认解析对 17 位
/// 十进制存在 1 ULP 误舍入，解析域位比较对忠实锚误报、对折叠对漏报——数值比较一律
/// 走本域）。
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(in super::super) enum JsonLeafToken<'a> {
    /// 字符串字面量原文（含引号；键同此形态，逐字比较）。
    Str(&'a [u8]),
    /// 数字 token 原文。
    Number(&'a [u8]),
    /// `true`/`false`/`null` 原文。
    Literal(&'a [u8]),
}

/// 逐叶提取 JSON 文本 token（文档序；结构字符与空白忽略）。
pub(in super::super) fn json_leaf_tokens(text: &[u8]) -> Vec<JsonLeafToken<'_>> {
    let mut tokens = Vec::new();
    let mut pos = 0;
    while pos < text.len() {
        match text[pos] {
            b'"' => {
                let start = pos;
                skip_json_value(text, &mut pos);
                tokens.push(JsonLeafToken::Str(&text[start..pos]));
            }
            b'-' | b'0'..=b'9' => {
                let start = pos;
                skip_json_value(text, &mut pos);
                tokens.push(JsonLeafToken::Number(&text[start..pos]));
            }
            b't' | b'f' | b'n' => {
                let start = pos;
                skip_json_value(text, &mut pos);
                tokens.push(JsonLeafToken::Literal(&text[start..pos]));
            }
            _ => pos += 1,
        }
    }
    tokens
}

/// 文本 token 域位级比较（lead 插队规格）：数字 token 以 `f64::from_str`（标准库正确
/// 舍入）→ `to_bits` 比较（整数叶 u64 逐位）、字符串/字面量逐字比较；两文本叶流按文档
/// 序逐叶对齐，返回首个不匹配描述（`None` = 全等）。锚 vs 源切片、重算值 vs 锚统一走
/// 本域（全路复用）。
pub(in super::super) fn json_leaf_mismatch(expected: &str, actual: &str) -> Option<String> {
    let left = json_leaf_tokens(expected.as_bytes());
    let right = json_leaf_tokens(actual.as_bytes());
    if left.len() != right.len() {
        return Some(format!("leaf count {} vs {}", left.len(), right.len()));
    }
    for (index, (l, r)) in left.iter().zip(right.iter()).enumerate() {
        match (l, r) {
            (JsonLeafToken::Str(a), JsonLeafToken::Str(b))
            | (JsonLeafToken::Literal(a), JsonLeafToken::Literal(b)) => {
                if a != b {
                    return Some(format!(
                        "leaf {index}: string/literal {} vs {}",
                        String::from_utf8_lossy(a),
                        String::from_utf8_lossy(b)
                    ));
                }
            }
            (JsonLeafToken::Number(a), JsonLeafToken::Number(b)) => {
                if a == b {
                    continue;
                }
                let av = std::str::from_utf8(a).unwrap();
                let bv = std::str::from_utf8(b).unwrap();
                let floats = av.contains(['.', 'e', 'E']) || bv.contains(['.', 'e', 'E']);
                let (abits, bbits) = if floats {
                    (
                        av.parse::<f64>().unwrap().to_bits(),
                        bv.parse::<f64>().unwrap().to_bits(),
                    )
                } else if let (Ok(x), Ok(y)) = (av.parse::<u64>(), bv.parse::<u64>()) {
                    (x, y)
                } else {
                    (
                        av.parse::<f64>().unwrap().to_bits(),
                        bv.parse::<f64>().unwrap().to_bits(),
                    )
                };
                if abits != bbits {
                    return Some(format!(
                        "leaf {index}: number {av} (bits {abits:#018x}) vs {bv} (bits {bbits:#018x})"
                    ));
                }
            }
            _ => return Some(format!("leaf {index}: token kind mismatch")),
        }
    }
    None
}

/// 派生元数据非逐字清单（lead ④ 裁决硬门 2；来源/候选/字段级，17 组 60 条）：
/// `pool.json` 的 `arc_m`/`spans` 经浮点再序列化相对来源存在非逐字字符串（1 ULP 家族）；
/// `program` u64 位逐字精确。清单随 MANIFEST 发布、README 差异记录同列。
const METADATA_DRIFT_GROUPS: &str = r#"[{"source":"1103","edge":"sumo::-10004_5_1","field":"arc_m","pool_indices":[2]},{"source":"1103","edge":"sumo::-10004_5_1","field":"spans","pool_indices":[0,1,2,3,4,5,6,7]},{"source":"1103","edge":"sumo::-10034_0_0","field":"spans","pool_indices":[0,1,2,3,4,5,6,7]},{"source":"1103","edge":"sumo::-10130_16_0","field":"arc_m","pool_indices":[1,3,4,7]},{"source":"1103","edge":"sumo::-10130_16_0","field":"spans","pool_indices":[6]},{"source":"1103","edge":"sumo::-10130_8_0","field":"arc_m","pool_indices":[5]},{"source":"1103","edge":"sumo::-10130_8_0","field":"spans","pool_indices":[0,1,2,3,4,5,6,7]},{"source":"1103","edge":"sumo::-29388_3_0","field":"arc_m","pool_indices":[5,6]},{"source":"1107","edge":"sumo::-7348_3_0","field":"spans","pool_indices":[0,1,2,3,4,5,6,7]},{"source":"1111","edge":"sumo::-10004_5_1","field":"arc_m","pool_indices":[11,13,15]},{"source":"1111","edge":"sumo::-7348_3_0","field":"arc_m","pool_indices":[15]},{"source":"1111","edge":"sumo::-7348_3_0","field":"spans","pool_indices":[8,9,10,11,12,13,14,15]},{"source":"1115","edge":"sumo::-16502_0_0","field":"spans","pool_indices":[0]},{"source":"1115","edge":"sumo::-28860_3_0","field":"spans","pool_indices":[1]},{"source":"1134-old","edge":"sumo::-25332_7_0","field":"arc_m","pool_indices":[3,6]},{"source":"1134-old","edge":"sumo::-25332_7_0","field":"spans","pool_indices":[2,3]},{"source":"1134-grid","edge":"sumo::-25332_7_0","field":"spans","pool_indices":[14]}]"#;

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

/// 钉版文本读取（SHA-256 断言语义同 `read_pinned_json`，panic 口径一致；文本原样
/// 返回，供位忠实切片）。
fn read_pinned_text(path: &Path, expected_sha: &str) -> String {
    let bytes = fs::read(path).expect("pinned baseline input present");
    assert_eq!(
        hex_sha256(&bytes),
        expected_sha,
        "pinned baseline input digest"
    );
    String::from_utf8(bytes).expect("pinned input is utf-8")
}

/// 锚 `source` 溯源键（取材身份三件套 + 代际 + 变更记录；无浮点字段，序列化无漂移）。
fn anchor_source_value() -> Value {
    json!({
        "asset":"evidence/run-03/results.json","sha256":RUN3_RESULTS_SHA256,"bytes":631650,
        "json_pointer":"#variants.new","generation":"92-池行",
        "release_tag":"research-evidence-1140-shared-baseline","comparison_only":true,
        "changed_from":{
            "asset":"stream-a-results.json","json_pointer":"#variants.new",
            "release_tag":"research-evidence-1134-cohort2-baseline","generation":"76-池行",
            "reason":"锚代际错位事故：原取材 #1131 代 stream-a-results.json（76 池行）；章程 §1 要求 #1134 新增变体 = run-03 results.json#variants.new（92 池行）；两代唯一语义差为 C40 missing_edges 6→5",
        },
    })
}

/// 锚语义守卫（结构/整数断言，浮点漂移免疫）：92 池行代际 + 计数逐值。
fn assert_anchor_semantics(variant: &Value) {
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
}

/// plan §2 结果对照锚（**位忠实文本拼接**，lead 复验硬门）：#1134 run-03
/// `results.json#variants.new` **整行按字节切片**（92 池行），数字字符串逐字保留、
/// 不做浮点再序列化（差异记录「导出路径有损再序列化（1 ULP）」）；仅外包 `source`
/// 溯源键——除 `source` 外锚文件与源切片+包装**字节可证等值**。语义守卫（C40
/// `missing_edges`=5 代际 + 计数）以结构断言执行（浮点漂移免疫）。
fn results_anchor_text(source_text: &str) -> String {
    let source: Value = serde_json::from_str(source_text).unwrap();
    assert_anchor_semantics(&source["variants"]["new"]);
    let bytes = source_text.as_bytes();
    let (inner_start, inner_end) = json_object_interior(bytes, 0, &["variants", "new"]);
    let interior = &source_text[inner_start..inner_end];
    let source_json = serde_json::to_string(&anchor_source_value()).unwrap();
    format!("{{\n  \"source\": {source_json},{interior}\n}}\n")
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
/// 锚取材 = run-03 `results.json` **原文文本**，供位忠实切片）。
fn pinned_sources() -> (Value, String, Value, Value) {
    let base = PathBuf::from(std::env::var("COHORT2_BASELINE_DIR").expect("COHORT2_BASELINE_DIR"));
    let run = PathBuf::from(std::env::var("COHORT2_RUN_DIR").expect("COHORT2_RUN_DIR"));
    let stream_searches = read_pinned_json(
        &base.join("stream-a-searches.json"),
        STREAM_A_SEARCHES_SHA256,
    );
    let run_results = read_pinned_text(&run.join("results.json"), RUN3_RESULTS_SHA256);
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
                program_u64_bits(&candidate["program"]),
                program_u64_bits(&json!(program_bits(program))),
                "rebuild candidate program u64 bits exact {id}"
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
    let anchor_text = results_anchor_text(&run_results);
    recovery::write_json(&out, "pool.json", &pool_rows);
    fs::write(out.join("results-anchor.json"), &anchor_text).unwrap();
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
        "derived_metadata":{
            "fields":["arc_m","spans"],
            "authority":"derived metadata, not authoritative; program u64 bits are the sole authoritative geometry input",
            "non_verbatim_strings":serde_json::from_str::<Value>(METADATA_DRIFT_GROUPS).unwrap(),
        },
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
/// 结果对照锚**字节可证等值**（源切片+包装，92 池行；数字字符串逐字）。
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
    let _ = results_anchor_pinned();
    let expected_anchor = results_anchor_text(&run_results);
    let anchor_bytes = fs::read(dir.join("results-anchor.json")).unwrap();
    assert_eq!(
        anchor_bytes,
        expected_anchor.as_bytes(),
        "anchor 字节可证等值（源切片+包装，数值字面量逐字）"
    );
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

#[test]
fn shared_baseline_program_fields_are_bit_exact() {
    // 硬门 1（lead ④ 裁决）：program 为几何/判定唯一权威输入，u64 位逐位精确——
    // 大位模式（1 ULP 相邻对）经合并/池文本往返后逐位相等，锁死不经浮点化。
    let a = json!({"program":{"start":[4624643347766747136u64,0,0],
        "segments":[{"end":[4624643347766747137u64,0,0],"canvas_selection":null}]},
        "arc_m":10.220051765441895,"spans":[]});
    let source = json!({"sumo::e0-synthetic":{"candidates":[a.clone()]}});
    let (rows, _provenance, _per_stage) = build_pool(&[(1, &source)]);
    let reparsed: Value = serde_json::from_str(&rows.to_string()).unwrap();
    let record = &reparsed["sumo::e0-synthetic"]["candidates"][0];
    assert_eq!(
        program_u64_bits(&record["program"]),
        program_u64_bits(&a["program"]),
        "program u64 bits exact"
    );
    assert_eq!(
        program_u64_bits(&record["program"])[0],
        4_624_643_347_766_747_136,
        "large bit pattern"
    );
    assert_eq!(
        program_u64_bits(&record["program"])[3],
        4_624_643_347_766_747_137,
        "1 ULP neighbour"
    );
}

#[test]
fn shared_baseline_text_token_domain_distinguishes_ulp_pair() {
    // 回归（lead 插队规格 3）：17 位十进制相邻对 …895/…896 在文本 token 域必须区分
    // （标准库正确舍入；serde_json 默认解析对二者误舍入折叠 → 漏报）。
    let faithful = "{\"x\":10.220051765441895}";
    assert_eq!(
        json_leaf_mismatch(faithful, "{\"x\":10.220051765441895}"),
        None
    );
    assert!(
        json_leaf_mismatch(faithful, "{\"x\":10.220051765441896}").is_some(),
        "ULP pair must be distinguished"
    );
    // 整数叶 u64 位逐位（program 位模式相邻对）。
    assert!(
        json_leaf_mismatch("{\"p\":4624643347766747136}", "{\"p\":4624643347766747137}").is_some()
    );
    assert_eq!(
        json_leaf_mismatch("{\"p\":4624643347766747136}", "{\"p\":4624643347766747136}"),
        None
    );
    // 字符串字面量逐字。
    assert!(json_leaf_mismatch("{\"s\":\"a\"}", "{\"s\":\"b\"}").is_some());
    assert_eq!(json_leaf_mismatch("{\"s\":\"a\"}", "{\"s\":\"a\"}"), None);
    // 与位忠实拼接衔接：文本 token 埲与逐字域在忠实文本上同判全等。
    let anchor_like = "{\"source\":{\"a\":1},\"k\":10.220051765441895}";
    assert_eq!(
        json_leaf_mismatch(
            "{\"source\":{\"a\":1},\"k\":10.220051765441895}",
            anchor_like
        ),
        None,
        "leaf domain ties to verbatim splice"
    );
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
    let missing = ["{\"edge\":\"sumo::e0-synthetic\"}"; 6].join(",");
    let source_text = format!(
        "{{\"variants\":{{\"new\":{{\"components\":[{{\"component\":\
         \"sumo:-25332:--32278#0_1-to--32770#7_1::-25332_0_1\",\"result\":\
         {{\"missing_edges\":[{missing}]}}}}]}}}}}}"
    );
    let _ = results_anchor_text(&source_text);
}

#[test]
fn shared_baseline_anchor_text_splice_is_byte_faithful() {
    // 位忠实硬门（差异记录「导出路径有损再序列化（1 ULP）」）：canonical_arc_m 回归
    // 样例数字逐字保留、全数值字面量逐个相等、锚 = 包装前缀 + 源切片 + 包装后缀。
    let components: String = (1..53)
        .map(|i| format!("{{\"component\":\"c{i}\",\"result\":{{\"missing_edges\":[]}}}}"))
        .collect::<Vec<_>>()
        .join(",");
    let missing = ["{\"edge\":\"sumo::e0-synthetic\"}"; 5].join(",");
    let paths: String = (0..84)
        .map(|i| format!("{{\"path\":\"p{i}\"}}"))
        .collect::<Vec<_>>()
        .join(",");
    let source_text = format!(
        "{{\"variants\":{{\"new\":{{\"components\":[\
         {{\"component\":\"sumo:-25332:--32278#0_1-to--32770#7_1::-25332_0_1\",\
         \"result\":{{\"missing_edges\":[{missing}]}}}},{components}],\
         \"paths\":[{paths}],\"path_counts\":{{\"accepted\":28,\
         \"cross_edge_direction_rejected\":3,\"target_check_not_completed\":53}},\
         \"regression\":{{\"canonical_arc_m\":10.220051765441895,\
         \"source_length_delta_m\":0.009999999999999787}}}}}},\"other\":1}}"
    );
    let anchor = results_anchor_text(&source_text);
    assert!(
        anchor.contains("10.220051765441895"),
        "canonical_arc_m regression literal verbatim"
    );
    assert!(
        anchor.contains("0.009999999999999787"),
        "source_length_delta_m literal verbatim"
    );
    assert!(
        !anchor.contains("10.220051765441896"),
        "no re-serialization drift"
    );
    let bytes = source_text.as_bytes();
    let (inner_start, inner_end) = json_object_interior(bytes, 0, &["variants", "new"]);
    let interior = &source_text[inner_start..inner_end];
    let source_json = serde_json::to_string(&anchor_source_value()).unwrap();
    let expected = format!("{{\n  \"source\": {source_json},{interior}\n}}\n");
    assert_eq!(anchor, expected, "byte-provable wrapper equality");
    // 全数值字段位级比较：锚内除 source 外的数值字面量与源切片逐个相等。
    let anchor_literals = json_numeric_literals(anchor.as_bytes());
    let interior_literals = json_numeric_literals(interior.as_bytes());
    let offset = anchor_literals.len() - interior_literals.len();
    assert_eq!(
        anchor_literals[offset..],
        interior_literals[..],
        "numeric literals verbatim"
    );
}
