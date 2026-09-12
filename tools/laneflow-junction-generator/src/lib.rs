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
    let config_resolved = resolve_for_compare(config_path)?;
    for output in [&paths.catalog, &paths.lfca] {
        if resolve_for_compare(output)? == config_resolved {
            return Err(Error::Config(format!(
                "output path {output:?} would overwrite the source config {config_path:?}"
            )));
        }
    }
    Ok(())
}

/// 与文件系统存在性无关的路径解析：先做词法归一（折叠 `.` / `..`，`..`
/// 不得越过根），再从最近存在的祖先 canonicalize（解析符号链接）并拼接
/// 剩余组件。`missing/..` 之类写法因此不能绕过别名检查。
fn resolve_for_compare(path: &Path) -> Result<PathBuf, Error> {
    let absolute = std::path::absolute(path).at(path)?;
    let mut lexical = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !lexical.pop() {
                    return Err(Error::Config(format!(
                        "path {path:?} escapes its filesystem root"
                    )));
                }
            }
            other => lexical.push(other.as_os_str()),
        }
    }
    let mut probe = lexical.clone();
    let mut tail = Vec::new();
    loop {
        if let Ok(canonical) = std::fs::canonicalize(&probe) {
            let mut resolved = canonical;
            for part in tail.iter().rev() {
                resolved.push(part);
            }
            return Ok(resolved);
        }
        let name = probe
            .file_name()
            .ok_or_else(|| Error::Config(format!("path {path:?} has no resolvable ancestor")))?
            .to_os_string();
        tail.push(name);
        if !probe.pop() {
            return Err(Error::Config(format!(
                "path {path:?} has no existing ancestor"
            )));
        }
    }
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
