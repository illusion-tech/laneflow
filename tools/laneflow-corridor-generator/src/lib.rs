mod config;
mod error;
mod generator;
mod model;

use std::path::{Path, PathBuf};

pub use config::{CorridorConfig, ENDPOINT_CLEARANCE_METERS, MIN_SPAWN_SLOT_COUNT};
pub use error::Error;
use error::IoResultExt;
pub use generator::{GeneratedScenario, ScenarioCounts, generate};
pub use laneflow_scenario::signalized_corridor::{
    CorridorCatalog, PortalCatalogEntry, RouteCatalogEntry, SpawnSlotCatalogEntry,
};

#[derive(Clone, Debug)]
pub struct OutputPaths {
    pub traffic: PathBuf,
    pub spatial: PathBuf,
    pub manifest: PathBuf,
    pub catalog: PathBuf,
}

pub fn load_config(path: &Path) -> Result<CorridorConfig, Error> {
    let input = std::fs::read_to_string(path).at(path)?;
    CorridorConfig::parse(&input)
}

pub fn output_paths(config_path: &Path, config: &CorridorConfig) -> OutputPaths {
    let config_directory = config_path.parent().unwrap_or_else(|| Path::new("."));
    let directory = config_directory.join(&config.output.directory);
    OutputPaths {
        traffic: directory.join(&config.output.traffic_artifact_ref),
        spatial: directory.join(&config.output.spatial_artifact_ref),
        manifest: directory.join(&config.output.manifest_file_name),
        catalog: directory.join(&config.output.catalog_file_name),
    }
}

pub fn generate_files(config_path: &Path) -> Result<ScenarioCounts, Error> {
    let config = load_config(config_path)?;
    let generated = generate(&config)?;
    let paths = output_paths(config_path, &config);
    let parent = paths
        .traffic
        .parent()
        .expect("joined output file always has a parent");
    std::fs::create_dir_all(parent).at(parent)?;
    write(&paths.traffic, generated.traffic_bytes())?;
    write(&paths.spatial, generated.spatial_bytes())?;
    write(&paths.manifest, generated.manifest_bytes())?;
    write(&paths.catalog, generated.catalog_bytes())?;
    Ok(generated.counts())
}

pub fn check_files(config_path: &Path) -> Result<ScenarioCounts, Error> {
    let config = load_config(config_path)?;
    let generated = generate(&config)?;
    let paths = output_paths(config_path, &config);
    compare(&paths.traffic, generated.traffic_bytes())?;
    compare(&paths.spatial, generated.spatial_bytes())?;
    compare(&paths.manifest, generated.manifest_bytes())?;
    compare(&paths.catalog, generated.catalog_bytes())?;
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
