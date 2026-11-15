//! #1155：serde_json 浮点解析精度缺口的位级往返测试与已知反例回归。
//!
//! 比较语义沿文本 token 域（[`super::shared_baseline::json_leaf_mismatch`]，数字叶以
//! `f64::from_str` 标准库正确舍入 → `to_bits` 比较）：serde 解析域位值只作被测对象，
//! 不作比较基准——解析域折叠对忠实锚误报、对折叠对漏报。
//!
//! 修复生效的充分性证据是 [`fix_gate_mandatory_correctly_rounded_bits`]（无条件
//! 正确位值断言）：#1173 起本 crate 及相关工具清单均已显式请求 serde_json
//! `float_roundtrip`，单包正常构建下该测试必须通过，防测试构建 feature 合并掩盖。
//! 历史背景（默认解析误舍入位值与双态自适应对照、解析耗时取证）已随 #1385 退役，
//! 正本见基线提交；已知反例记录：
//!
//! - 已知反例（#1155 背景口径 / #1140 差异记录 4）实测对照：
//!   - `10.220051765441895`：正确舍入 bits `4621942995720077312`；serde_json 1.0.154
//!     默认解析曾误舍入 bits `4621942995720077313`（+1 ULP），再序列化丢失源 token
//!     变成 `10.220051765441896`；
//!   - `10.220051765441896`：bits `4621942995720077313`；
//!   - `0.009999999999999787`：正确舍入 bits `4576918229304087552`；默认解析曾误舍入
//!     bits `4576918229304087553`（+1 ULP），再序列化丢失源 token 变成
//!     `0.009999999999999789`；
//!   - `0.009999999999999789`：bits `4576918229304087553`。
//! - soak 同款时长链丢位实证（维护者 2026-10-29 复现，`target/issue-1155-review/probe.rs`）：
//!   `Duration::as_secs_f64` → JSON → `Value` 抽样 10,000 中 **1,444 个**默认解析丢位
//!   （±1 ULP 双向），启用 `float_roundtrip` 后 **0/10,000**；三个代表样本已入
//!   [`CORRECT_BITS`] 语料。

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

/// 探测当前构建形态 serde_json 浮点解析是否已启用 `float_roundtrip`（feature 统一
/// 随编译依赖图而异；本 crate 清单已按 #1173 显式启用）。
fn float_roundtrip_active() -> bool {
    serde_json::from_str::<f64>("10.220051765441895")
        .unwrap()
        .to_bits()
        == 4_621_942_995_720_077_312
}

/// 反例 token 的正确舍入位值与相邻 1 ULP 配对关系固定不变（文本 token 域正本）。
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
