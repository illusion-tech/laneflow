//! LuST Scenario v2.0 source/static converter (#253).
//!
//! Slice A currently ships pinned source verification and CLI scaffolding.
//! Static Traffic/Spatial/Manifest conversion lands in follow-up commits.

mod config;
mod error;
mod source;

pub use config::{LustConverterConfig, load_config};
pub use error::{Error, Result};
pub use source::{
    LUST_COMMIT, LUST_REPOSITORY, LUST_TAG, PINNED_SOURCE_FILES, PinnedSourceFile,
    VerifiedSourceFile, VerifiedSourceSet, verify_source_dir,
};

use std::path::Path;

/// Verify the pinned LuST source set under `source_dir`.
pub fn verify_source(source_dir: &Path) -> Result<VerifiedSourceSet> {
    verify_source_dir(source_dir)
}

/// Run convert after verifying pinned source; static conversion is not ready yet.
pub fn convert(config_path: &Path) -> Result<()> {
    let config = load_config(config_path)?;
    let _verified = verify_source_dir(&config.source_dir)?;
    let _ = config.output_dir;
    Err(Error::StaticConversionNotImplemented)
}
