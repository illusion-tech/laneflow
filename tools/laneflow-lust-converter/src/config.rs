//! Converter TOML configuration.

use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use serde::Deserialize;

use crate::{Error, Result};

/// On-disk configuration for convert / future check commands.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LustConverterConfig {
    /// Absolute or relative path to a LuST Scenario checkout root.
    pub source_dir: PathBuf,
    /// Directory that will receive static / report outputs.
    pub output_dir: PathBuf,
    /// Optional assertion of the converter git commit: verified against the
    /// source checkout HEAD (mismatch fails closed); when unset the checkout
    /// HEAD is recorded (`LANEFLOW_CONVERTER_COMMIT` acts as the same assertion).
    #[serde(default)]
    pub converter_commit: Option<String>,
    /// Optional GitHub Release URL for the source tar asset.
    #[serde(default)]
    pub source_bundle_url: Option<String>,
    /// Optional GitHub Release URL for the static tar asset.
    #[serde(default)]
    pub static_bundle_url: Option<String>,
}

impl LustConverterConfig {
    /// Validate required fields after TOML deserialize.
    pub fn validate(&self) -> Result<()> {
        if self.source_dir.as_os_str().is_empty() {
            return Err(Error::Config("source_dir must not be empty".to_owned()));
        }
        if self.output_dir.as_os_str().is_empty() {
            return Err(Error::Config("output_dir must not be empty".to_owned()));
        }
        // #253：output_dir 拒绝 `..` 组件——发布锚点按词法拼回尚不存在的
        // 缺失组件，含 `..` 的词法锚点与发布后的 canonical 路径必然不等
        // （合法转换会在交付完成后误报锚点漂移）；`..` 穿过 symlink 时的
        // 真实目标也无法词法判定。相对路径仍支持，含 `..` 的一律
        // fail-closed，请调用方写规范化路径。
        if self
            .output_dir
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(Error::Config(format!(
                "output_dir must not contain '..' components: {}",
                self.output_dir.display()
            )));
        }
        Ok(())
    }
}

/// Load and validate converter TOML from `path`.
///
/// 当前仅验收测试消费（生产入口 `convert` 走 `load_config_with_bytes` 以
/// 同时取得 config digest 字节）；新增生产调用方时去掉 `cfg(test)` 即可。
#[cfg(test)]
pub fn load_config(path: &Path) -> Result<LustConverterConfig> {
    let text = fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let config: LustConverterConfig = toml::from_str(&text)?;
    config.validate()?;
    Ok(config)
}

/// Load config together with the exact TOML bytes used for config digest.
pub fn load_config_with_bytes(path: &Path) -> Result<(LustConverterConfig, Vec<u8>)> {
    let bytes = fs::read(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        Error::Config(format!(
            "converter config {} is not valid UTF-8",
            path.display()
        ))
    })?;
    let config: LustConverterConfig = toml::from_str(text)?;
    config.validate()?;
    Ok((config, bytes))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::LustConverterConfig;
    use crate::Error;

    #[test]
    fn validate_rejects_parent_dir_components_in_output_dir() {
        // #253：含 `..` 的 output_dir 让词法发布锚点与发布后的 canonical
        // 路径必然不等——合法转换会在交付完成后误报锚点漂移；validate 直接
        // fail-closed。
        let config = LustConverterConfig {
            source_dir: PathBuf::from("src"),
            output_dir: PathBuf::from("new/../out"),
            converter_commit: None,
            source_bundle_url: None,
            static_bundle_url: None,
        };
        let error = config.validate().expect_err("'..' must fail validation");
        match error {
            Error::Config(message) => assert!(message.contains("'..'"), "{message}"),
            other => panic!("unexpected error: {other}"),
        }
    }
}
