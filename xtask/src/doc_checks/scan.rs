//! 来源收集与仓库内路径解析。不跟随符号链接，也不离开仓库根。

use std::collections::BTreeSet;
use std::fs::{self, DirEntry};
use std::io;
use std::path::{Component, Path, PathBuf};

const HARD_SKIP: &[&str] = &[".git", "target", "vendor", "node_modules"];
const SOFT_SKIP: &[&str] = &["tests", "fixtures", "evidence"];

#[derive(Debug)]
pub(super) struct Resolved {
    pub(super) rel: String,
    /// 链接以 `/` 结尾，或本身是 `.` / `..`。存在性检查据此要求目录。
    pub(super) directory: bool,
}

#[derive(Debug)]
pub(super) enum ResolveError {
    Escape,
    Unsupported(&'static str),
}

#[derive(Debug)]
pub(super) enum Stat {
    File,
    Dir,
}

#[derive(Debug)]
pub(super) enum StatError {
    Missing,
    Symlink,
    NotDirectory,
    Special,
    Io(String),
}

pub(super) fn collect_sources(
    root: &Path,
    explicit: Option<&[PathBuf]>,
) -> Result<BTreeSet<String>, String> {
    match explicit {
        None => collect_default(root),
        Some(paths) => collect_explicit(root, paths),
    }
}

fn collect_default(root: &Path) -> Result<BTreeSet<String>, String> {
    let mut files = BTreeSet::new();
    for name in ["README.md", "CONTRIBUTING.md", "AGENTS.md"] {
        add_optional_file(root, &root.join(name), &mut files)?;
    }
    add_optional_file(root, &root.join("research/README.md"), &mut files)?;
    walk_optional(root, &root.join("docs"), true, None, &mut files)?;
    walk_optional(root, &root.join(".agents"), true, None, &mut files)?;
    walk_optional(
        root,
        &root.join(".cursor/skills"),
        true,
        Some("SKILL.md"),
        &mut files,
    )?;
    add_child_readmes(root, &root.join("crates"), &mut files)?;
    add_child_readmes(root, &root.join("tools"), &mut files)?;
    walk_optional(root, &root.join("schemas"), true, None, &mut files)?;
    add_top_level_md(root, &root.join(".github"), &mut files)?;
    Ok(files)
}

fn collect_explicit(root: &Path, paths: &[PathBuf]) -> Result<BTreeSet<String>, String> {
    let mut files = BTreeSet::new();
    for path in paths {
        let joined = if path.is_absolute() {
            path.clone()
        } else {
            root.join(path)
        };
        let rel = repo_rel(root, &joined)?;
        // 逐段 lstat。祖先是符号链接时，不能只看最终组件，否则会读到仓库外文件。
        match stat_rel(root, &rel) {
            Ok(Stat::File) => {
                if !is_markdown_rel(&rel) {
                    return Err(format!(
                        "check-docs 只接受 Markdown 文件：`{}`",
                        joined.display()
                    ));
                }
                files.insert(rel);
            }
            Ok(Stat::Dir) => {
                let soften = !rel_has_soft_exclusion(&rel);
                walk_dir(root, &abs_from_rel(root, &rel), soften, None, &mut files)?;
            }
            Err(StatError::Symlink) => {
                return Err(format!("check-docs 不跟随符号链接：`{}`", joined.display()));
            }
            Err(StatError::Missing) => {
                return Err(format!("无法读取路径 `{}`: 路径不存在", joined.display()));
            }
            Err(StatError::Io(message)) => return Err(message),
            Err(StatError::NotDirectory | StatError::Special) => {
                return Err(format!("不支持的路径类型：`{}`", joined.display()));
            }
        }
    }
    Ok(files)
}

fn walk_optional(
    root: &Path,
    dir: &Path,
    soft: bool,
    file_name: Option<&str>,
    files: &mut BTreeSet<String>,
) -> Result<(), String> {
    let Some(dir) = real_directory(root, dir) else {
        return Ok(());
    };
    walk_dir(root, &dir, soft, file_name, files)
}

fn walk_dir(
    root: &Path,
    dir: &Path,
    soft: bool,
    only_file_name: Option<&str>,
    files: &mut BTreeSet<String>,
) -> Result<(), String> {
    for entry in read_sorted(dir)? {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("无法读取路径 `{}`: {error}", path.display()))?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if skip_dir(&path, soft) {
                continue;
            }
            walk_dir(root, &path, soft, only_file_name, files)?;
            continue;
        }
        if file_type.is_file() && is_markdown(&path) && name_allowed(&path, only_file_name) {
            files.insert(repo_rel(root, &path)?);
        }
    }
    Ok(())
}

fn add_optional_file(root: &Path, path: &Path, files: &mut BTreeSet<String>) -> Result<(), String> {
    let Ok(rel) = repo_rel(root, path) else {
        return Ok(());
    };
    if matches!(stat_rel(root, &rel), Ok(Stat::File)) && is_markdown_rel(&rel) {
        files.insert(rel);
    }
    Ok(())
}

fn real_directory(root: &Path, dir: &Path) -> Option<PathBuf> {
    let rel = repo_rel(root, dir).ok()?;
    match stat_rel(root, &rel) {
        Ok(Stat::Dir) => Some(abs_from_rel(root, &rel)),
        _ => None,
    }
}

fn add_child_readmes(root: &Path, dir: &Path, files: &mut BTreeSet<String>) -> Result<(), String> {
    let Some(dir) = real_directory(root, dir) else {
        return Ok(());
    };
    for entry in read_sorted(&dir)? {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("无法读取路径 `{}`: {error}", path.display()))?;
        if file_type.is_symlink() || !file_type.is_dir() || skip_dir(&path, true) {
            continue;
        }
        add_optional_file(root, &path.join("README.md"), files)?;
    }
    Ok(())
}

fn add_top_level_md(root: &Path, dir: &Path, files: &mut BTreeSet<String>) -> Result<(), String> {
    let Some(dir) = real_directory(root, dir) else {
        return Ok(());
    };
    for entry in read_sorted(&dir)? {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("无法读取路径 `{}`: {error}", path.display()))?;
        if file_type.is_symlink() || !file_type.is_file() || !is_markdown(&path) {
            continue;
        }
        files.insert(repo_rel(root, &path)?);
    }
    Ok(())
}

fn read_sorted(dir: &Path) -> Result<Vec<DirEntry>, String> {
    let mut entries = fs::read_dir(dir)
        .map_err(|error| format!("无法读取目录 `{}`: {error}", dir.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法遍历目录 `{}`: {error}", dir.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn skip_dir(path: &Path, soft: bool) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return true;
    };
    if HARD_SKIP.contains(&name) {
        return true;
    }
    if soft && SOFT_SKIP.contains(&name) {
        return true;
    }
    if soft && name == "generated" {
        return path
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|parent| parent.to_str())
            == Some("src");
    }
    false
}

fn rel_has_soft_exclusion(rel: &str) -> bool {
    let mut previous = "";
    for part in rel.split('/') {
        if SOFT_SKIP.contains(&part) || (part == "generated" && previous == "src") {
            return true;
        }
        previous = part;
    }
    false
}

fn name_allowed(path: &Path, only_file_name: Option<&str>) -> bool {
    only_file_name
        .is_none_or(|expected| path.file_name().and_then(|name| name.to_str()) == Some(expected))
}

pub(super) fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

pub(super) fn is_markdown_rel(rel: &str) -> bool {
    is_markdown(Path::new(rel))
}

pub(super) fn repo_rel(root: &Path, path: &Path) -> Result<String, String> {
    let root = normalize_lexical(root);
    let path = normalize_lexical(path);
    let relative = path
        .strip_prefix(&root)
        .map_err(|_| format!("路径越出仓库：`{}`", path.display()))?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| format!("路径不是 UTF-8：`{}`", path.display()))?;
                parts.push(part);
            }
            Component::CurDir => {}
            _ => {
                return Err(format!("路径越出仓库：`{}`", path.display()));
            }
        }
    }
    Ok(parts.join("/"))
}

pub(super) fn abs_from_rel(root: &Path, rel: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    if !rel.is_empty() {
        path.push(rel);
    }
    path
}

pub(super) fn stat_rel(root: &Path, rel: &str) -> Result<Stat, StatError> {
    if rel.is_empty() {
        return Ok(Stat::Dir);
    }
    let mut current = root.to_path_buf();
    let parts: Vec<&str> = rel.split('/').filter(|part| !part.is_empty()).collect();
    for (index, part) in parts.iter().enumerate() {
        // 用目录项的原始拼写比较，避免大小写不敏感的文件系统把 Docs 当成 docs。
        let kind = exact_entry(&current, part)?;
        let last = index + 1 == parts.len();
        if kind.symlink {
            return Err(StatError::Symlink);
        }
        if kind.dir {
            current.push(part);
            if last {
                return Ok(Stat::Dir);
            }
            continue;
        }
        if kind.file {
            if last {
                return Ok(Stat::File);
            }
            return Err(StatError::NotDirectory);
        }
        return Err(StatError::Special);
    }
    Err(StatError::Missing)
}

struct EntryKind {
    symlink: bool,
    dir: bool,
    file: bool,
}

fn exact_entry(dir: &Path, name: &str) -> Result<EntryKind, StatError> {
    let wanted = std::ffi::OsStr::new(name);
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(StatError::Missing),
        Err(error) => {
            return Err(StatError::Io(format!(
                "无法读取目录 `{}`: {error}",
                dir.display()
            )));
        }
    };
    for entry in entries {
        let entry = entry
            .map_err(|error| StatError::Io(format!("无法读取目录 `{}`: {error}", dir.display())))?;
        if entry.file_name() != wanted {
            continue;
        }
        let file_type = entry.file_type().map_err(|error| {
            StatError::Io(format!(
                "无法读取路径 `{}`: {error}",
                entry.path().display()
            ))
        })?;
        return Ok(EntryKind {
            symlink: file_type.is_symlink(),
            dir: file_type.is_dir(),
            file: file_type.is_file(),
        });
    }
    Err(StatError::Missing)
}

/// 把已百分号解码的本地路径解析成仓库相对路径。`base_rel` 是源文件所在目录。
pub(super) fn resolve_decoded(base_rel: &str, decoded: &str) -> Result<Resolved, ResolveError> {
    if decoded.is_empty() {
        return Err(ResolveError::Unsupported("空路径"));
    }
    if decoded.contains('\\') {
        return Err(ResolveError::Unsupported("反斜线路径不支持"));
    }
    if decoded.contains('\0') {
        return Err(ResolveError::Unsupported("路径含空字符"));
    }
    if decoded.contains(':') {
        return Err(ResolveError::Unsupported("目标含冒号，不能当作仓库路径"));
    }
    let directory = decoded.ends_with('/') || ends_with_dir_dot(decoded);
    let absolute = decoded.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    if !absolute {
        parts.extend(base_rel.split('/').filter(|part| !part.is_empty()));
    }
    for part in decoded.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if parts.pop().is_none() {
                return Err(ResolveError::Escape);
            }
            continue;
        }
        parts.push(part);
    }
    Ok(Resolved {
        rel: parts.join("/"),
        directory,
    })
}

fn ends_with_dir_dot(path: &str) -> bool {
    path == "." || path == ".." || path.ends_with("/.") || path.ends_with("/..")
}

pub(super) fn parent_rel(rel: &str) -> &str {
    match rel.rfind('/') {
        Some(index) => &rel[..index],
        None => "",
    }
}

pub(super) fn percent_decode(input: &str) -> Result<String, ()> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(());
            }
            let high = from_hex(bytes[index + 1]).ok_or(())?;
            let low = from_hex(bytes[index + 2]).ok_or(())?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| ())
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push("..");
                }
            }
            Component::Normal(part) => normalized.push(part),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
        }
    }
    normalized
}
