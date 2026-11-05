//! LaneFlow 统一五轴汇总 Gate 执行器（#1268）。
//!
//! 以 `LF-SYNTH-v2` 与中国特色城市负载各轴既有正式记录为声明式输入
//! （`lf-unified-gate-manifest-1`，TOML），产出统一汇总回执
//! （`lf-unified-gate-receipt-1`）：逐格携带来源提交、被测二进制、输入身份、
//! 覆盖边界与例外状态，供 #305 最终验收消费。
//!
//! 边界（#1268 非目标）：不运行正式汇总、不替代各轴分立 harness、不重跑正式
//! 测量、不批准例外。来源身份一律从既有记录携带（ADR 0033 §3：本工具不新增
//! 摘要角色，不重算任何摘要）；离线运行，不联网核验。`research/` 等仓库文件
//! 只在运行时读取，不编译期嵌入（`docs/governance/documentation-policy.md` §3）。
//!
//! 另交付 `check-transitions`：对 LF-SYNTH 输入包执行 #1271 拓扑核查——按运行时
//! 同口径枚举横向过渡段，扫描其汇入窗口内「避开可见邻域的平行边路线」与
//! 「汇入点前分岔」使能结构，产出结构化记录（`lf-l1-topology-check-1`）供
//! manifest 消费。

pub mod evidence;
pub mod l1check;
pub mod manifest;
pub mod receipt;
pub mod summary;

/// 工具级错误：`Invalid` 携带面向人的中文诊断，其余为底层失败透传。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO 失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("TOML 解析失败：{0}")]
    Toml(#[from] toml::de::Error),
    #[error("JSON 解析失败：{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}

/// 构造 `Error::Invalid` 的简写。
pub fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
