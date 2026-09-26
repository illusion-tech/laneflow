use crate::{Result, need};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
};

pub(crate) fn read_json(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(text.trim_start_matches('\u{feff}'))?)
}

pub(crate) fn sha(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65_536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(crate) fn files(root: &Path) -> Result<Vec<PathBuf>> {
    fn walk(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for item in fs::read_dir(path)? {
            let item = item?;
            let kind = item.file_type()?;
            need(!kind.is_symlink(), "symlink in evidence/source")?;
            if kind.is_dir() {
                walk(&item.path(), out)?;
            } else if kind.is_file() {
                out.push(item.path());
            }
        }
        Ok(())
    }
    let mut found = Vec::new();
    walk(root, &mut found)?;
    found.sort_by_key(|p| {
        p.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    });
    Ok(found)
}

pub(crate) fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(crate) fn source_index(root: &Path) -> Result<Value> {
    let mut out = serde_json::Map::new();
    for path in files(root)? {
        let relative = path.strip_prefix(root)?;
        if relative.components().any(|p| p.as_os_str() == "target") {
            continue;
        }
        out.insert(slash(relative), json!(sha(&path)?));
    }
    Ok(Value::Object(out))
}

pub(crate) fn file_index(root: &Path) -> Result<Value> {
    let mut out = Vec::new();
    for path in files(root)? {
        out.push(json!({"path":slash(path.strip_prefix(root)?), "bytes":fs::metadata(&path)?.len(), "sha256":sha(&path)?}));
    }
    Ok(json!(out))
}

pub(crate) fn safe_child(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    need(
        !relative.is_empty()
            && path.components().all(|c| matches!(c, Component::Normal(_)))
            && !relative.contains('\\'),
        "unsafe relative evidence path",
    )?;
    Ok(root.join(path))
}

pub(crate) fn ensure_new(path: &Path) -> Result<()> {
    need(!path.try_exists()?, "output already exists")
}

pub(crate) fn write_new(path: &Path, value: &Value) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

// 只替换当前采集过程自己建立的未封存元数据；分析和验证从不覆盖文件。
pub(crate) fn replace_owned(path: &Path, value: &Value) -> Result<()> {
    let mut file = OpenOptions::new().write(true).truncate(true).open(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

pub(crate) fn outside(root: &Path, output: &Path) -> Result<()> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    need(
        !parent.canonicalize()?.starts_with(root.canonicalize()?),
        "output must be outside raw package",
    )
}

pub(crate) fn command(repo: &Path, program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(repo)
        .output()?;
    need(
        output.status.success(),
        &format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
    )?;
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

pub(crate) fn git(repo: &Path, args: &[&str]) -> Result<String> {
    command(repo, "git", args)
}
