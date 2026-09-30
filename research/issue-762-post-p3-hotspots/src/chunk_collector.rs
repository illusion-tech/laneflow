//! #801 从干净冻结提交受控构建采集器；普通命令只接受凭据绑定的实际 EXE。
use crate::{EXPERIMENT, Result, chunk_build, chunk_config, chunk_native, io, need};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const SOURCE: &str = "collector-source";
const ARCHIVE: &str = "collector-source.tar";
const INDEX: &str = "collector-source-index.json";
const RECEIPT: &str = "collector.json";
const LOGS: [&str; 2] = ["collector.build.stdout", "collector.build.stderr"];
const ARGS: [&str; 10] = [
    "+1.98.0",
    "build",
    "--release",
    "--locked",
    "--offline",
    "-p",
    "laneflow-post-p3-research",
    "--bin",
    "laneflow-p5-chunk-research",
    "--target-dir",
];

fn binary_name() -> String {
    format!("laneflow-p5-chunk-research{}", std::env::consts::EXE_SUFFIX)
}
fn recipe(source: &Path, target: &Path, environment: &Value) -> Value {
    let args: Vec<_> = ARGS
        .iter()
        .map(|s| (*s).to_owned())
        .chain(std::iter::once(target.to_string_lossy().into_owned()))
        .collect();
    json!({"program":"cargo","args":args,"working_directory":source,
        "environment_cleared":true,"environment":environment})
}
fn hex(value: &Value, length: usize) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.len() == length && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

pub(crate) fn validate(value: &Value) -> Result<()> {
    let source = PathBuf::from(
        value["command"]["working_directory"]
            .as_str()
            .ok_or("collector source directory")?,
    );
    let target = source
        .parent()
        .ok_or("collector source parent")?
        .join("collector-target");
    let environment = json!(chunk_build::controlled_environment(
        &value["inherited_environment"],
        &value["native_toolchain"]
    )?);
    chunk_native::validate(&value["native_toolchain"])?;
    chunk_config::validate(&value["cargo_configuration"], &source, &environment)?;
    need(
        value["schema"] == "p5-collector-controlled-build-v1"
            && value["protocol"] == EXPERIMENT.protocol
            && value["exit_code"] == 0
            && source.is_absolute()
            && source.file_name().is_some_and(|p| p == SOURCE)
            && value["command"] == recipe(&source, &target, &environment)
            && hex(&value["source_git_head"], 40)
            && hex(&value["source_git_tree"], 40)
            && hex(&value["source_archive_sha256"], 64)
            && hex(&value["source_files_sha256"], 64)
            && value["binary_name"] == binary_name()
            && hex(&value["binary_sha256"], 64)
            && value["binary_bytes"].as_u64().is_some_and(|n| n > 0)
            && value["rustc"]
                .as_str()
                .is_some_and(|s| s.starts_with("rustc 1.98.0 "))
            && value["cargo"]
                .as_str()
                .is_some_and(|s| s.starts_with("cargo 1.98.0 ")),
        "collector build attestation mismatch",
    )?;
    let logs = value["logs"].as_array().ok_or("collector build logs")?;
    need(
        logs.len() == 2
            && logs
                .iter()
                .zip(LOGS)
                .all(|(log, name)| log["path"] == name && hex(&log["sha256"], 64)),
        "collector build log identities",
    )
}

fn archive_head(root: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["get-tar-commit-id"])
        .stdin(Stdio::from(fs::File::open(root.join(ARCHIVE))?))
        .output()?;
    need(output.status.success(), "collector archive commit")?;
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

pub(crate) fn verify_at(root: &Path, executable: &Path) -> Result<Value> {
    let value = io::read_json(&root.join(RECEIPT))?;
    validate(&value)?;
    need(
        value["binary_sha256"] == io::sha(executable)?
            && value["binary_bytes"] == fs::metadata(executable)?.len(),
        "running collector executable mismatch",
    )?;
    let index = io::read_json(&root.join(INDEX))?;
    need(
        value["source_files_sha256"] == chunk_build::value_sha(&index)?
            && index == io::source_index(&root.join(SOURCE))?
            && value["source_archive_sha256"] == io::sha(&root.join(ARCHIVE))?
            && value["source_git_head"] == archive_head(root)?,
        "collector source/archive mismatch",
    )?;
    for log in value["logs"].as_array().ok_or("collector build logs")? {
        let path = io::safe_child(root, log["path"].as_str().ok_or("collector log path")?)?;
        need(
            log["sha256"] == io::sha(&path)?,
            "collector build log digest",
        )?;
    }
    Ok(value)
}
pub(crate) fn verify_running() -> Result<Value> {
    let executable = std::env::current_exe()?;
    verify_at(
        executable.parent().ok_or("collector executable parent")?,
        &executable,
    )
}
pub(crate) fn verify_root(root: &Path) -> Result<()> {
    verify_at(root, &root.join(binary_name()))?;
    Ok(())
}
pub(crate) fn verify_worktree(value: &Value) -> Result<()> {
    let repo = std::env::current_dir()?;
    need(
        value["source_git_head"] == io::git(&repo, &["rev-parse", "HEAD"])?
            && value["source_git_tree"] == io::git(&repo, &["rev-parse", "HEAD^{tree}"])?
            && io::git(&repo, &["status", "--porcelain"])?.is_empty(),
        "collector executable/source differs from current clean revision",
    )
}

pub(crate) fn build(root: &Path) -> Result<()> {
    let repo = std::env::current_dir()?;
    need(
        io::git(&repo, &["status", "--porcelain"])?.is_empty(),
        "dirty collector build source",
    )?;
    let head = io::git(&repo, &["rev-parse", "HEAD"])?;
    let tree = io::git(&repo, &["rev-parse", "HEAD^{tree}"])?;
    io::ensure_new(root)?;
    fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    let source = root.join(SOURCE);
    fs::create_dir(&source)?;
    let export = Command::new("git")
        .current_dir(&repo)
        .args(["archive", "--format=tar", &head, "--output"])
        .arg(root.join(ARCHIVE))
        .status()?;
    need(export.success(), "collector git archive failed")?;
    need(
        archive_head(&root)? == head,
        "collector git archive head mismatch",
    )?;
    let status = Command::new("tar")
        .arg("-xf")
        .arg(root.join(ARCHIVE))
        .arg("-C")
        .arg(&source)
        .status()?;
    need(status.success(), "collector source extraction failed")?;
    let index = io::source_index(&source)?;
    io::write_new(&root.join(INDEX), &index)?;
    let inherited = chunk_build::inherited_environment();
    let mut probe: std::collections::BTreeMap<String, String> =
        serde_json::from_value(inherited.clone())?;
    probe.extend(
        chunk_build::ENV
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned())),
    );
    let native = chunk_native::probe(&root, "collector", ".native.json", &probe)?;
    let environment = json!(chunk_build::controlled_environment(&inherited, &native)?);
    let configuration = chunk_config::snapshot(&source, &environment)?;
    let target = root.join("collector-target");
    let command = recipe(&source, &target, &environment);
    let stdout = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(LOGS[0]))?;
    let stderr = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(LOGS[1]))?;
    let status = Command::new("cargo")
        .args(ARGS)
        .arg(&target)
        .current_dir(&source)
        .env_clear()
        .envs(serde_json::from_value::<
            std::collections::BTreeMap<String, String>,
        >(environment.clone())?)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .status()?;
    need(
        status.success(),
        "controlled collector build failed; source and logs retained",
    )?;
    need(
        native == chunk_native::probe(&root, "collector", ".native-after.json", &probe)?,
        "collector native toolchain drift",
    )?;
    need(
        configuration == chunk_config::snapshot(&source, &environment)?,
        "collector Cargo configuration drift",
    )?;
    need(
        index == io::source_index(&source)?,
        "collector source drift during build",
    )?;
    let compiled = target.join("release").join(binary_name());
    let binary = root.join(binary_name());
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&binary)?;
    std::io::copy(&mut fs::File::open(&compiled)?, &mut output)?;
    output.sync_all()?;
    need(
        io::sha(&binary)? == io::sha(&compiled)?,
        "collector copy digest",
    )?;
    let logs: Vec<_> = LOGS
        .into_iter()
        .map(|name| -> Result<Value> {
            Ok(json!({"path":name,"sha256":io::sha(&root.join(name))?}))
        })
        .collect::<Result<_>>()?;
    let value = json!({"schema":"p5-collector-controlled-build-v1","protocol":EXPERIMENT.protocol,"exit_code":status.code(),
        "source_git_head":head,"source_git_tree":tree,"source_archive_sha256":io::sha(&root.join(ARCHIVE))?,
        "source_files_sha256":chunk_build::value_sha(&index)?,"command":command,"inherited_environment":inherited,
        "native_toolchain":native,"cargo_configuration":configuration,
        "rustc":io::command(&source,"rustc",&["+1.98.0","-Vv"])?,"cargo":io::command(&source,"cargo",&["+1.98.0","-V"])?,
        "binary_name":binary_name(),"binary_sha256":io::sha(&binary)?,"binary_bytes":fs::metadata(&binary)?.len(),"logs":logs});
    validate(&value)?;
    io::write_new(&root.join(RECEIPT), &value)?;
    verify_at(&root, &binary)?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn fixture(root: &Path) -> Value {
    let inherited = json!({"CARGO_HOME":root.join("cargo-home")});
    let native = chunk_native::fixture(root);
    let environment = json!(chunk_build::controlled_environment(&inherited, &native).unwrap());
    let source = root.join(SOURCE);
    json!({"schema":"p5-collector-controlled-build-v1","protocol":EXPERIMENT.protocol,"exit_code":0,
        "source_git_head":"1".repeat(40),"source_git_tree":"2".repeat(40),
        "source_archive_sha256":"3".repeat(64),"source_files_sha256":"4".repeat(64),
        "command":recipe(&source,&root.join("collector-target"),&environment),"inherited_environment":inherited,
        "native_toolchain":native,"cargo_configuration":chunk_config::fixture(&source,&environment),
        "rustc":"rustc 1.98.0 (fixture)","cargo":"cargo 1.98.0 (fixture)","binary_name":binary_name(),
        "binary_sha256":"5".repeat(64),"binary_bytes":8,"logs":LOGS.map(|path| json!({"path":path,"sha256":"6".repeat(64)}))})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_executable_cannot_borrow_a_new_source_receipt() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&root).unwrap();
        let value = fixture(&root);
        validate(&value).unwrap();
        io::write_new(&root.join(RECEIPT), &value).unwrap();
        let binary = root.join(binary_name());
        fs::write(&binary, b"stale collector").unwrap();
        assert!(
            verify_at(&root, &binary)
                .unwrap_err()
                .to_string()
                .contains("running collector executable mismatch")
        );
        fs::remove_file(binary).unwrap();
        fs::remove_file(root.join(RECEIPT)).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
