//! #794：子进程诊断在测试目录清理前归档，每个原始 stderr 文件只汇总一次。

use std::{
    fs,
    io::{self, Write},
    ops::Deref,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize)]
pub(crate) struct RecoveryCounts {
    pub(crate) records: u64,
    pub(crate) write_readback_retries: u64,
    pub(crate) emission_retries: u64,
    pub(crate) stream_retries: u64,
    pub(crate) source_regenerations: u64,
    pub(crate) fail_closed: u64,
}

impl RecoveryCounts {
    fn add(&mut self, other: Self) {
        self.records += other.records;
        self.write_readback_retries += other.write_readback_retries;
        self.emission_retries += other.emission_retries;
        self.stream_retries += other.stream_retries;
        self.source_regenerations += other.source_regenerations;
        self.fail_closed += other.fail_closed;
    }
}

fn records(bytes: &[u8]) -> io::Result<(RecoveryCounts, Vec<&str>)> {
    let text = std::str::from_utf8(bytes).map_err(io::Error::other)?;
    let mut counts = RecoveryCounts::default();
    let mut records = Vec::new();
    for line in text
        .lines()
        .filter(|line| line.starts_with("[digest-probe] "))
    {
        records.push(line);
        counts.records += 1;
        // prior= 可嵌入另一条诊断；它不是第二次事件，也不能覆盖当前行的 action。
        let own = line.split(" prior=").next().unwrap_or(line);
        let field = |key: &str| {
            own.split_whitespace()
                .find_map(|word| word.strip_prefix(key))
        };
        // 清理失败行附带的 action 复述后续写后核对事件，不代表又执行一次重写。
        if field("reason=") == Some("cleanup-failed") {
            continue;
        }
        match (field("site="), field("action=")) {
            (Some("write-readback"), Some("retry")) => counts.write_readback_retries += 1,
            (Some("emission-retry"), Some("retry")) => counts.emission_retries += 1,
            (Some("stream-readback"), Some("retry")) => counts.stream_retries += 1,
            (Some("fixture-copy"), Some("regenerate")) => counts.source_regenerations += 1,
            (_, Some("fail-closed")) => counts.fail_closed += 1,
            (_, Some("retry" | "regenerate")) => {
                return Err(io::Error::other(format!("未分类的子进程恢复记录：{line}")));
            }
            (Some("fixture-copy"), Some("preserve")) => {}
            (Some("write-readback" | "emission-retry" | "stream-readback" | "fixture-copy"), _) => {
                return Err(io::Error::other(format!("不完整的子进程恢复记录：{line}")));
            }
            _ => {}
        }
    }
    Ok((counts, records))
}

fn stderr_files(directory: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            stderr_files(&entry.path(), files)?;
        } else if kind.is_file() && entry.file_name().to_string_lossy().ends_with(".stderr.log") {
            files.push(entry.path());
        }
    }
    Ok(())
}

fn publish(
    path: &Path,
    archive: Option<&Path>,
    sink: &mut impl Write,
) -> io::Result<RecoveryCounts> {
    let bytes = fs::read(path)?;
    let saved = archive
        .map(|root| {
            fs::create_dir_all(root)?;
            let directory = tempfile::Builder::new()
                .prefix(&format!("child-{}-", std::process::id()))
                .tempdir_in(root)?
                .keep();
            let raw = directory.join("stderr.log");
            fs::write(&raw, &bytes)?;
            if fs::read(&raw)? != bytes {
                return Err(io::Error::other("子进程 stderr 归档读回不一致"));
            }
            Ok(directory)
        })
        .transpose()?;
    let (counts, records) = records(&bytes)?;
    let receipt = serde_json::json!({
        "parent_pid":std::process::id(),
        "test":std::thread::current().name(),
        "source":path,
        "archive":saved,
        "bytes":bytes.len(),
        "counts":counts,
        "status":"complete",
    });
    if let Some(saved) = &saved {
        let receipt_bytes = serde_json::to_vec_pretty(&receipt)?;
        let receipt_path = saved.join("capture.json");
        fs::write(&receipt_path, &receipt_bytes)?;
        if fs::read(receipt_path)? != receipt_bytes {
            return Err(io::Error::other("子进程诊断回执归档读回不一致"));
        }
    }
    writeln!(sink, "[child-diagnostics] {receipt}")?;
    for record in records {
        writeln!(sink, "{record}")?;
    }
    Ok(counts)
}

/// 调用方须先等待/终止子进程，再清理目录；嵌套进程只写各自文件，不逐层转发。
pub(crate) struct DiagnosticDirectory(Option<tempfile::TempDir>);

impl DiagnosticDirectory {
    pub(crate) fn new() -> io::Result<Self> {
        Ok(Self(Some(tempfile::tempdir()?)))
    }

    pub(crate) fn finish_with(
        &mut self,
        archive: Option<&Path>,
        sink: &mut impl Write,
    ) -> io::Result<RecoveryCounts> {
        let Some(directory) = self.0.take() else {
            return Ok(RecoveryCounts::default());
        };
        let result = (|| {
            let mut files = Vec::new();
            stderr_files(directory.path(), &mut files)?;
            files.sort();
            let mut total = RecoveryCounts::default();
            for path in files {
                total.add(publish(&path, archive, sink)?);
            }
            Ok(total)
        })();
        if result.is_err() || std::thread::panicking() {
            let preserved = directory.keep();
            eprintln!(
                "[child-diagnostics] preserved directory={}",
                preserved.display()
            );
        }
        result
    }

    fn finish(&mut self) -> io::Result<RecoveryCounts> {
        let archive = archive_root();
        self.finish_with(archive.as_deref(), &mut io::stderr().lock())
    }
}

impl Deref for DiagnosticDirectory {
    type Target = tempfile::TempDir;

    fn deref(&self) -> &Self::Target {
        self.0
            .as_ref()
            .expect("diagnostic directory already collected")
    }
}

impl Drop for DiagnosticDirectory {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            if std::thread::panicking() {
                eprintln!("[child-diagnostics] collection failed: {error}");
            } else {
                panic!("child diagnostic collection failed: {error}");
            }
        }
    }
}

/// 保留 Output 的退出状态与原始 stdout/stderr，先归档再交给原测试断言。
pub(crate) fn output(command: &mut Command) -> io::Result<Output> {
    output_with(command, archive_root().as_deref(), &mut io::stderr().lock())
}

fn archive_root() -> Option<PathBuf> {
    std::env::var_os("LANEFLOW_DIGEST_PROBE_DIR")
        .map(PathBuf::from)
        .map(|path| path.join("child-diagnostics"))
}

pub(crate) fn output_with(
    command: &mut Command,
    archive: Option<&Path>,
    sink: &mut impl Write,
) -> io::Result<Output> {
    let mut directory = DiagnosticDirectory::new()?;
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let pid = child.id();
    let output = child.wait_with_output()?;
    fs::write(
        directory.path().join(format!("{pid}.stderr.log")),
        &output.stderr,
    )?;
    directory.finish_with(archive, sink)?;
    Ok(output)
}
