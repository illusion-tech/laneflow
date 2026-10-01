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
pub fn verify_source(source_dir: &Path) -> Result<VerifiedSourceSet> {
    crate::source::verify_source_dir(source_dir)
}

/// Verify pinned source and write static/source bundles plus provenance.
pub fn convert(config_path: &Path) -> Result<ConvertOutputPaths> {
    let (config, config_bytes) = load_config_with_bytes(config_path)?;
    run_convert(&config, &config_bytes)
}
