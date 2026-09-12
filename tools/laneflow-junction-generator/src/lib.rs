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

/// 输出文件不得解析回源 config 自身、两个输出文件也不得互为别名（同名
/// 符号链接或大小写变体），否则 `generate` 会静默覆写编制输入或让后写的
/// LFCA 覆盖先写的 catalog。
fn reject_config_alias(config_path: &Path, paths: &OutputPaths) -> Result<(), Error> {
    let config_resolved = resolve_for_compare(config_path)?;
    let catalog_resolved = resolve_for_compare(&paths.catalog)?;
    let lfca_resolved = resolve_for_compare(&paths.lfca)?;
    if paths_alias(&catalog_resolved, &lfca_resolved) {
        return Err(Error::Config(format!(
            "output paths {:?} and {:?} resolve to the same file",
            paths.catalog, paths.lfca
        )));
    }
    for (resolved, output) in [
        (&catalog_resolved, &paths.catalog),
        (&lfca_resolved, &paths.lfca),
    ] {
        if paths_alias(resolved, &config_resolved) {
            return Err(Error::Config(format!(
                "output path {output:?} would overwrite the source config {config_path:?}"
            )));
        }
    }
    Ok(())
}

fn paths_alias(left: &Path, right: &Path) -> bool {
    left == right || case_insensitive_paths_equal(left, right)
}

/// 大小写不敏感文件系统（NTFS/APFS 默认）上的补充比较。
#[cfg(any(windows, target_os = "macos"))]
fn case_insensitive_paths_equal(left: &Path, right: &Path) -> bool {
    left.to_str().is_some_and(|left| {
        right
            .to_str()
            .is_some_and(|right| left.to_lowercase() == right.to_lowercase())
    })
}

/// 大小写敏感文件系统上仅按精确解析结果比较。
#[cfg(not(any(windows, target_os = "macos")))]
fn case_insensitive_paths_equal(_left: &Path, _right: &Path) -> bool {
    false
}

/// 按文件系统遍历顺序解析路径：逐组件推进，已存在的组件立即 canonicalize
/// （符号链接在随后的 `..` 之前解析）。`std::path::absolute` 与
/// `Path::components()` 都会词法折叠 `..`、Windows canonicalize 对已存在
/// 路径上的 `..` 也做词法折叠，都掩盖符号链接先序解析，因此盘符/根切分与
/// 分隔符切分必须手写（`..` 不得越过根）。悬空符号链接按 read_link 文本
/// 目标代入走查，链接层数封顶。非 UTF-8 路径超出本内部工具的支持范围，
/// 按 config 错误处理。
fn resolve_for_compare(path: &Path) -> Result<PathBuf, Error> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().at(path)?.join(path)
    };
    let text = absolute
        .to_str()
        .ok_or_else(|| Error::Config(format!("path {path:?} is not valid Unicode")))?
        .to_owned();
    let (mut resolved, rest) = split_off_root(&text).map_err(Error::Config)?;
    let mut pending: Vec<String> = rest
        .split(['/', '\\'])
        .map(str::to_owned)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let mut links_followed = 0_u32;
    while let Some(part) = pending.pop() {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if !resolved.pop() {
                return Err(Error::Config(format!(
                    "path {path:?} escapes its filesystem root"
                )));
            }
            continue;
        }
        resolved.push(&part);
        if resolved.exists() {
            resolved = std::fs::canonicalize(&resolved).at(&resolved)?;
            continue;
        }
        let Ok(metadata) = std::fs::symlink_metadata(&resolved) else {
            continue;
        };
        if !metadata.file_type().is_symlink() {
            continue;
        }
        // canonicalize 对悬空符号链接失败；按 read_link 的文本目标代入：
        // 相对目标相对于链接所在目录，绝对目标重新从根走查。
        links_followed += 1;
        if links_followed > 40 {
            return Err(Error::Config(format!(
                "path {path:?} exceeds the symlink depth limit"
            )));
        }
        let target = std::fs::read_link(&resolved).at(&resolved)?;
        resolved.pop();
        if target.is_absolute() {
            let target_text = target
                .to_str()
                .ok_or_else(|| {
                    Error::Config(format!("symlink target {target:?} is not valid Unicode"))
                })?
                .to_owned();
            let (root, rest) = split_off_root(&target_text).map_err(Error::Config)?;
            resolved = root;
            pending.extend(rest.split(['/', '\\']).map(str::to_owned).rev());
        } else {
            pending.extend(
                target
                    .to_str()
                    .ok_or_else(|| {
                        Error::Config(format!("symlink target {target:?} is not valid Unicode"))
                    })?
                    .split(['/', '\\'])
                    .map(str::to_owned)
                    .rev(),
            );
        }
    }
    Ok(resolved)
}

/// 切出路径的根部分（Unix `/`、Windows 盘符、UNC、`\\?\` verbatim）并返回
/// 余下文本；其余根形式不支持。
fn split_off_root(text: &str) -> Result<(PathBuf, &str), String> {
    if let Some(stripped) = text.strip_prefix('/') {
        return Ok((PathBuf::from("/"), stripped));
    }
    if let Some(stripped) = text.strip_prefix("\\\\?\\") {
        let bytes = stripped.as_bytes();
        if bytes.len() >= 3 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/') {
            return Ok((PathBuf::from(&text[..7]), &text[7..]));
        }
        return Err(format!("unsupported verbatim root: {text}"));
    }
    if let Some(stripped) = text.strip_prefix("\\\\") {
        // \\server\share\rest
        let mut ends = stripped.match_indices(['\\', '/']).map(|(index, _)| index);
        let Some(_server_end) = ends.next() else {
            return Err(format!("UNC path needs server and share: {text}"));
        };
        let Some(share_end) = ends.next() else {
            return Err(format!("UNC path needs server and share: {text}"));
        };
        let root_end = 2 + share_end + 1;
        return Ok((PathBuf::from(&text[..root_end]), &text[root_end..]));
    }
    let bytes = text.as_bytes();
    if bytes.len() >= 3
        && bytes[1] == b':'
        && bytes[0].is_ascii_alphabetic()
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return Ok((PathBuf::from(&text[..3]), &text[3..]));
    }
    Err(format!("unsupported root form: {text}"))
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
