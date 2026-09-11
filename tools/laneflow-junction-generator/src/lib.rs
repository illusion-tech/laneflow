mod catalog;
mod compile;
mod config;
mod error;
mod generator;
mod topology;

use std::path::{Path, PathBuf};

pub use config::JunctionConfig;
pub use error::Error;
use error::IoResultExt;
pub use generator::{GeneratedScenario, ScenarioCounts, generate};
pub use laneflow_scenario::complex_junction::{
    JunctionCatalog, PortalCatalogEntry, PortalLaneCatalogEntry, RouteCatalogEntry,
    SpawnSlotCatalogEntry, WeightedRouteChoiceCatalogEntry,
};

#[derive(Clone, Debug)]
pub struct OutputPaths {
    pub catalog: PathBuf,
    pub lfca: PathBuf,
}

pub fn load_config(path: &Path) -> Result<JunctionConfig, Error> {
    let input = std::fs::read_to_string(path).at(path)?;
    JunctionConfig::parse(&input)
}

pub fn output_paths(config_path: &Path, config: &JunctionConfig) -> OutputPaths {
    let config_directory = config_path.parent().unwrap_or_else(|| Path::new("."));
    let directory = config_directory.join(&config.output.directory);
    OutputPaths {
        catalog: directory.join(&config.output.catalog_file_name),
        lfca: directory.join(&config.output.lfca_file_name),
    }
}

pub fn generate_files(config_path: &Path) -> Result<ScenarioCounts, Error> {
    let config = load_config(config_path)?;
    let paths = output_paths(config_path, &config);
    reject_config_alias(config_path, &paths)?;
    let generated = generate(&config)?;
    let parent = paths
        .catalog
        .parent()
        .expect("joined output file always has a parent");
    std::fs::create_dir_all(parent).at(parent)?;
    write(&paths.catalog, generated.catalog_bytes())?;
    write(&paths.lfca, generated.lfca_bytes())?;
    Ok(generated.counts())
}

/// 输出文件不得解析回源 config 自身（例如 `directory = "."` 加同名文件），
/// 否则 `generate` 会静默覆写编制输入。
fn reject_config_alias(config_path: &Path, paths: &OutputPaths) -> Result<(), Error> {
    let config_canonical = std::fs::canonicalize(config_path).at(config_path)?;
    for output in [&paths.catalog, &paths.lfca] {
        // 输出文件可能尚不存在：canonicalize 其父目录后拼接文件名再比较。
        let parent = output
            .parent()
            .expect("joined output file always has a parent");
        let resolved = std::fs::canonicalize(parent)
            .unwrap_or_else(|_| parent.to_path_buf())
            .join(output.file_name().expect("output file name"));
        if resolved == config_canonical {
            return Err(Error::Config(format!(
                "output path {output:?} would overwrite the source config {config_path:?}"
            )));
        }
    }
    Ok(())
}

pub fn check_files(config_path: &Path) -> Result<ScenarioCounts, Error> {
    let config = load_config(config_path)?;
    let generated = generate(&config)?;
    let paths = output_paths(config_path, &config);
    compare(&paths.catalog, generated.catalog_bytes())?;
    compare(&paths.lfca, generated.lfca_bytes())?;
    Ok(generated.counts())
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    std::fs::write(path, bytes).at(path)
}

fn compare(path: &Path, expected: &[u8]) -> Result<(), Error> {
    let actual = std::fs::read(path).at(path)?;
    if actual == expected {
        return Ok(());
    }
    let detail = actual
        .iter()
        .zip(expected)
        .position(|(actual, expected)| actual != expected)
        .map_or_else(
            || {
                format!(
                    "byte lengths differ (checked-in {}, generated {})",
                    actual.len(),
                    expected.len()
                )
            },
            |index| {
                format!(
                    "first difference at byte {index} (checked-in {}, generated {})",
                    actual[index], expected[index]
                )
            },
        );
    Err(Error::OutputMismatch {
        path: path.to_owned(),
        detail,
    })
}
