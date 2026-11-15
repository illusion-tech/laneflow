//! LuST Scenario v2.0 converter（#253）。
//!
//! 公开面是 CLI（`src/main.rs`）所需的最小闭包：`verify_source` / `convert`
//! 两个入口、pinned 常量与错误类型。其余实现（拓扑转换、诊断清单、信号、
//! population/routes、provenance 等）为 crate 私有——「library caller 可
//! 冒名/滥用」一类问题在结构上不存在（#253 复审决议）。

mod config;
mod convert;
mod error;
mod output;
mod source;
mod sumo;

pub use error::Error;
pub(crate) use error::Result;
pub use source::{LUST_COMMIT, LUST_REPOSITORY, LUST_TAG, VerifiedSourceSet};

use std::path::Path;

use crate::{
    config::load_config_with_bytes,
    output::pipeline::{ConvertOutputPaths, convert_with_config as run_convert},
};

/// Verify the pinned LuST source set under `source_dir`.
///
/// # Errors
///
/// - `Error::MissingSourceFile` / `Error::SourceSizeMismatch` /
///   `Error::SourceDigestMismatch`：pinned 文件缺失、大小或 SHA-256 与
///   钉死值不符。
/// - `Error::SourceRevisionUnknown` / `Error::SourceRevisionMismatch`：
///   source checkout 的 revision 无法确定或与钉死值不符。
/// - `Error::Io`：读取 source 文件失败。
pub fn verify_source(source_dir: &Path) -> Result<VerifiedSourceSet> {
    crate::source::verify_source_dir(source_dir)
}

/// Verify pinned source and write the deterministic artifact set plus
/// provenance under the configured `output_dir`.
///
/// CLI `convert` 固定走诊断模式（`emit_infeasibility_report: true`）：交付
/// 不可行诊断清单、conversion report、manifest、source tar 与 provenance——
/// **不**产出 `network.lfca` / `routes.toml` / `lust-static.tar`
/// （`ConvertOutputPaths` 对应字段为 `None`）。
///
/// # Errors
///
/// - `Error::Toml` / `Error::Config`：config 解析或校验失败（含 config
///   bytes 与生效配置不一致）。
/// - `Error::MissingSourceFile` / `Error::SourceSizeMismatch` /
///   `Error::SourceDigestMismatch` / `Error::SourceChangedAfterVerification`
///   / `Error::SourceRevisionUnknown` / `Error::SourceRevisionMismatch`：
///   pinned LuST source 集验证或消费期 TOCTOU 重校验失败。
/// - `Error::XmlParse` / `Error::SumoModel`：SUMO 网络解析失败。
/// - `Error::Validation`：拓扑转换验收失败，或 publish 事务失败（含中断
///   恢复 fail-closed 与回滚不完整）。
/// - `Error::Json` / `Error::TomlSerialize`：report / manifest /
///   provenance 序列化失败。
/// - `Error::Io`：staging、publish 或备份恢复等文件系统操作失败。
pub fn convert(config_path: &Path) -> Result<ConvertOutputPaths> {
    let (config, config_bytes) = load_config_with_bytes(config_path)?;
    run_convert(&config, &config_bytes)
}
