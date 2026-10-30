//! #1155：serde_json 浮点解析精度缺口的位级往返测试与已知反例回归。
//!
//! 比较语义沿文本 token 域（[`super::shared_baseline::json_leaf_mismatch`]，数字叶以
//! `f64::from_str` 标准库正确舍入 → `to_bits` 比较）：serde 解析域位值只作被测对象，
//! 不作比较基准——解析域折叠对忠实锚误报、对折叠对漏报。
//!
//! 构建形态双态自适应：serde_json `float_roundtrip` 是否生效取决于 feature 统一
//! （workspace 构建合并 `research/issue-762` 的请求，单包构建没有）。两种形态下本模块
//! 全绿，且把两侧行为都钉死：
//!
//! - 已知反例（#1155 背景口径 / #1140 差异记录 4）实测对照：
//!   - `10.220051765441895`：正确舍入 bits `4621942995720077312`；默认解析误舍入
//!     bits `4621942995720077313`（+1 ULP），再序列化丢失源 token 变成
//!     `10.220051765441896`；
//!   - `10.220051765441896`：bits `4621942995720077313`（两形态均正确）；
//!   - `0.009999999999999787`：正确舍入 bits `4576918229304087552`；默认解析误舍入
//!     bits `4576918229304087553`（+1 ULP），再序列化丢失源 token 变成
//!     `0.009999999999999789`；
//!   - `0.009999999999999789`：bits `4576918229304087553`（两形态均正确）。
//! - 默认解析下两组反例对各自折叠到同一 bits（解析域漏报形态的实证）。
//! - soak 同款时长链丢位实证（维护者 2026-10-29 复现，`target/issue-1155-review/probe.rs`）：
//!   `Duration::as_secs_f64` → JSON → `Value` 抽样 10,000 中 **1,444 个**默认解析丢位
//!   （±1 ULP 双向），启用 `float_roundtrip` 后 **0/10,000**；三个代表样本已入
//!   [`CORRECT_BITS`] / [`DEFAULT_PARSE_BITS`] 语料。
//!
//! 证明力边界（维护者 2026-10-29 审定）：双态自适应断言只构成**双态差异实证 + 自适应
//! 对照**，不构成修复生效证明——feature 缺失时误舍入分支同样通过。修复生效的充分性
//! 证据是 [`fix_gate_mandatory_correctly_rounded_bits`]（无条件正确位值断言，#1173 已
//! 启用）：本 crate 各 Cargo.toml 均请求 `float_roundtrip` 后，单包正常构建下它必须
//! 通过，防测试构建 feature 合并掩盖。

use super::shared_baseline::{JsonLeafToken, json_leaf_mismatch, json_leaf_tokens};

/// 已知反例文本 token → 标准库正确舍入位值（`f64::from_str` 语义）。前四项为
/// #1155 背景口径反例对；后三项为 soak 同款时长链丢位实证样本（维护者 2026-10-29
/// 复现，`target/issue-1155-review/probe.rs`：`Duration::as_secs_f64` → JSON →
/// `Value` 抽样 10,000 中 1,444 个默认解析丢位，启用 `float_roundtrip` 后 0）。
const CORRECT_BITS: &[(&str, u64)] = &[
    ("10.220051765441895", 4_621_942_995_720_077_312),
    ("10.220051765441896", 4_621_942_995_720_077_313),
    ("0.009999999999999787", 4_576_918_229_304_087_552),
    ("0.009999999999999789", 4_576_918_229_304_087_553),
    ("1.7743044000000001", 4_610_669_575_807_328_744),
    ("1.8793370999999999", 4_611_142_601_035_910_460),
    ("1.8715220000000001", 4_611_107_404_954_462_598),
];

/// serde_json 1.0.154 默认解析（无 `float_roundtrip`）实测位值：两组反例对中先行
/// token 各 +1 ULP 误舍入、后继 token 正确（反例对折叠到同一 bits）；三个时长链
/// token ±1 ULP 双向误舍入（−1 ULP 与 +1 ULP 均有实例）。
const DEFAULT_PARSE_BITS: &[(&str, u64)] = &[
    ("10.220051765441895", 4_621_942_995_720_077_313),
    ("10.220051765441896", 4_621_942_995_720_077_313),
    ("0.009999999999999787", 4_576_918_229_304_087_553),
    ("0.009999999999999789", 4_576_918_229_304_087_553),
    ("1.7743044000000001", 4_610_669_575_807_328_743),
    ("1.8793370999999999", 4_611_142_601_035_910_461),
    ("1.8715220000000001", 4_611_107_404_954_462_597),
];

/// 默认解析会丢位、再序列化会丢源 token 的全部反例与时长链样本。
const LOSSY_UNDER_DEFAULT: &[&str] = &[
    "10.220051765441895",
    "0.009999999999999787",
    "1.7743044000000001",
    "1.8793370999999999",
    "1.8715220000000001",
];

/// 探测当前构建形态 serde_json 浮点解析是否已启用 `float_roundtrip`（feature 统一
/// 因构建形态而异：workspace 构建合并 `research/issue-762` 的请求，单包构建没有）。
fn float_roundtrip_active() -> bool {
    serde_json::from_str::<f64>("10.220051765441895")
        .unwrap()
        .to_bits()
        == 4_621_942_995_720_077_312
}

/// 反例 token 的正确舍入位值与相邻 1 ULP 配对关系固定不变（文本 token 域权威）。
#[test]
fn counterexample_tokens_have_pinned_correctly_rounded_bits() {
    for (text, bits) in CORRECT_BITS {
        assert_eq!(
            text.parse::<f64>().unwrap().to_bits(),
            *bits,
            "oracle {text}"
        );
    }
    // 两个已知反例对各自恰为相邻 1 ULP。
    assert_eq!(CORRECT_BITS[0].1 + 1, CORRECT_BITS[1].1, "895/896 adjacent");
    assert_eq!(CORRECT_BITS[2].1 + 1, CORRECT_BITS[3].1, "787/789 adjacent");
}

/// 反例经 serde 写侧序列化后文本 token 逐字保真、位值往返恒等（写侧无损）。
#[test]
fn counterexample_tokens_round_trip_faithfully_through_serde_write() {
    for (text, bits) in CORRECT_BITS {
        let value = f64::from_bits(*bits);
        let emitted = serde_json::to_string(&value).unwrap();
        assert_eq!(emitted, *text, "shortest token for {text}");
        let doc = format!("{{\"x\":{text}}}");
        assert_eq!(
            json_leaf_tokens(doc.as_bytes()),
            vec![
                JsonLeafToken::Str(b"\"x\""),
                JsonLeafToken::Number(text.as_bytes())
            ],
            "leaf token bytes for {text}"
        );
    }
}

/// serde 解析位值按当前构建形态钉死：默认解析 = 1 ULP 误舍入钉版值；`float_roundtrip`
/// 生效 = 全部正确舍入。任何新静默漂移（依赖版本漂移等）都会在此失败。
#[test]
fn serde_parse_bits_match_pinned_regime() {
    let active = float_roundtrip_active();
    for ((text, default_bits), (_, correct_bits)) in DEFAULT_PARSE_BITS.iter().zip(CORRECT_BITS) {
        let actual = serde_json::from_str::<f64>(text).unwrap().to_bits();
        let expected = if active { *correct_bits } else { *default_bits };
        assert_eq!(actual, expected, "{text} under float_roundtrip={active}");
    }
}

/// 默认解析下两组反例对各自折叠到同一 bits（解析域漏报形态实证）；`float_roundtrip`
/// 生效后位值相邻可区分。
#[test]
fn parse_domain_collapses_known_pairs_under_default() {
    let active = float_roundtrip_active();
    for [left, right] in [
        ["10.220051765441895", "10.220051765441896"],
        ["0.009999999999999787", "0.009999999999999789"],
    ] {
        let lbits = serde_json::from_str::<f64>(left).unwrap().to_bits();
        let rbits = serde_json::from_str::<f64>(right).unwrap().to_bits();
        if active {
            assert_ne!(lbits, rbits, "roundtrip must distinguish {left} vs {right}");
        } else {
            assert_eq!(lbits, rbits, "default parse collapses {left} vs {right}");
        }
    }
}

/// 有损再序列化（#1140 重锚事故形态：parse → re-serialize 丢 1 ULP 与源 token）在文本
/// token 域可被检出：默认解析下两个先行反例的再序列化文档与源文档不匹配，其余匹配；
/// `float_roundtrip` 生效后全部匹配。
#[test]
fn lossy_re_serialization_is_detected_in_text_domain() {
    let active = float_roundtrip_active();
    for (text, _) in CORRECT_BITS {
        let source = format!("{{\"x\":{text}}}");
        let value: serde_json::Value = serde_json::from_str(&source).unwrap();
        let re = serde_json::to_string(&value).unwrap();
        let drifted = json_leaf_mismatch(&source, &re).is_some();
        let expect_drift = !active && LOSSY_UNDER_DEFAULT.contains(text);
        assert_eq!(drifted, expect_drift, "drift detection for {text}");
    }
}

/// 已知反例对在文本 token 域恒可区分（严格域不随解析形态折叠）。
#[test]
fn counterexample_pairs_never_collapse_in_text_domain() {
    for [left, right] in [
        ["10.220051765441895", "10.220051765441896"],
        ["0.009999999999999787", "0.009999999999999789"],
    ] {
        let l = format!("{{\"x\":{left}}}");
        let r = format!("{{\"x\":{right}}}");
        assert!(
            json_leaf_mismatch(&l, &r).is_some(),
            "text domain must distinguish {left} vs {right}"
        );
    }
}

/// 修复阶段强制检查面（维护者 2026-10-29 审定，#1173 启用）：无条件断言全部已知反例
/// 解析为正确舍入位值——feature 缺失时本测试必失败，是 `float_roundtrip` 修复生效的
/// 充分性证据；须在**单包正常构建**下运行通过，防测试构建 feature 合并掩盖。
#[test]
fn fix_gate_mandatory_correctly_rounded_bits() {
    assert!(float_roundtrip_active(), "float_roundtrip must be active");
    for (text, bits) in CORRECT_BITS {
        assert_eq!(
            serde_json::from_str::<f64>(text).unwrap().to_bits(),
            *bits,
            "mandatory correct bits for {text}"
        );
    }
}

/// #1155 ③ 固定输入解析耗时对照取证（同机描述性基线；上游注明 `float_roundtrip` 有
/// 解析成本）。同一确定性输入文本在两种构建形态（默认 / `float_roundtrip` 统一）下
/// 各自测量 `Vec<f64>` 解析中位耗时；`--release -- --ignored --nocapture` 手动取证。
#[test]
#[ignore = "#1155 ③ 手动取证：cargo test --release --lib fixed_input_parse_cost_evidence -- --ignored --nocapture"]
fn fixed_input_parse_cost_evidence() {
    use std::time::Instant;

    // 固定输入：确定性 50,000 个浮点 token 的 JSON 数组文本（两形态字节相同）。
    let values: Vec<f64> = (0..50_000_u64)
        .map(|i| i as f64 * 7.3e-3 + 10.220051765441895)
        .collect();
    let text = serde_json::to_string(&values).unwrap();
    let tokens = values.len();
    drop(values);
    std::hint::black_box(&text);

    let mut samples = Vec::with_capacity(200);
    for round in 0..220_u32 {
        let from = Instant::now();
        let parsed: Vec<f64> = serde_json::from_str(&text).unwrap();
        let elapsed = from.elapsed();
        std::hint::black_box(&parsed);
        if round >= 20 {
            samples.push(elapsed);
        }
    }
    samples.sort();
    let median = samples[samples.len() / 2];
    let min = samples[0];
    println!(
        "parse_cost float_roundtrip={} tokens={tokens} bytes={} median={:?} median_ns_per_token={} min={:?}",
        float_roundtrip_active(),
        text.len(),
        median,
        median.as_nanos() as u64 / tokens as u64,
        min,
    );
}
