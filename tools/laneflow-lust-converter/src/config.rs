//! Converter TOML configuration.

use std::{fs, path::Path, path::PathBuf};

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
        Ok(())
    }
}

/// Load and validate converter TOML from `path`.
pub fn load_config(path: &Path) -> Result<LustConverterConfig> {
    let text = fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let config: LustConverterConfig = toml::from_str(&text)?;
    config.validate()?;
    Ok(config)
}
