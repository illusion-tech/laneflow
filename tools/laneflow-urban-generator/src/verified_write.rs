//! 写后回读校验与有界重写（#954）。
//!
//! 合同（G0 冻结）：
//!
//! 1. **唯一期望 = 登记时刻摘要**：[`SourceRegistration::of`] 在写前对源缓冲冻结
//!    SHA-256；之后一切读回判定只对照该登记摘要，绝不从可疑缓冲重算期望；
//! 2. **登记后源缓冲损坏即失败关闭**：每次写入/重写前复核源缓冲仍与登记摘要一致，
//!    不一致时不重试、直接失败并输出现场；
//! 3. **两路读回分歧即失败关闭**：读回 A（一次性读）与读回 B（flush 后独立流式读）
//!    必须同时命中登记摘要；两路不一致说明读路径本身不可信，失败关闭、不重试。
//!    平台无法绕过页缓存核对持久介质时，缓存—持久分歧面显式不覆盖（见
//!    [`ArtifactIo::read_persistent`]）；
//! 4. **重写有界**：一致失配（两路相同但与登记摘要不符）时允许从源缓冲重写，
//!    上限固定（[`WritePolicy::DEFAULT`] = 3 次）；
//! 5. **耗尽删残件、终态失败关闭**：任何失败路径不留可被消费的未验证文件。
//!
//! 保证范围：成功判定包含 flush 后读回；**不含断电耐久与目录项耐久**（rename、
//! 目录 fsync 与发布事务属宿主 / 发布边界，见 compiler 后发射设计）。
//!
//! 失配现场以 `[digest-probe] site=write-readback` 行输出，与
//! `docs/reference/validation-matrix.md` 判读口径一致；成功重写次数由
//! [`VerifiedWrite::rewrites`] 提供恢复遥测。

use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

/// 写后校验与重写策略：上限固定有界。
#[derive(Clone, Copy, Debug)]
pub struct WritePolicy {
    /// 成功前允许的重写次数上限。
    max_rewrites: u32,
}

impl WritePolicy {
    /// 默认策略：最多重写 3 次（#954 G2 落定）。
    pub const DEFAULT: Self = Self { max_rewrites: 3 };
}

/// 登记时刻摘要：写前冻结的唯一期望。
#[derive(Clone, Copy, Debug)]
pub struct SourceRegistration {
    digest: [u8; 32],
}

impl SourceRegistration {
    /// 在写前对源缓冲冻结摘要。登记时点之后的源缓冲损坏由写入路径检出并失败关闭。
    #[must_use]
    pub fn of(source: &[u8]) -> Self {
        Self {
            digest: Sha256::digest(source).into(),
        }
    }

    /// 登记摘要的十六进制形式（诊断输出用）。
    #[must_use]
    pub fn hex(&self) -> String {
        hex(&self.digest)
    }
}

/// 成功写入的见证：重写次数用于恢复遥测。
#[derive(Clone, Copy, Debug)]
pub struct VerifiedWrite {
    /// 成功前执行的重写次数（0 表示一次通过）。
    pub rewrites: u32,
}

/// 写后读回校验与重写的失败。
#[derive(Debug)]
pub enum VerifiedWriteError {
    /// 文件 I/O 失败；残件已尽力删除。
    Io(io::Error),
    /// 源缓冲在登记后被损坏（与登记摘要不符）：不重试，失败关闭。
    SourceCorrupted {
        /// 登记时刻摘要。
        registered: String,
        /// 实际源缓冲摘要。
        actual: String,
    },
    /// 两路读回不一致：读路径不可信，不重试，失败关闭。
    ReadbackDivergent {
        /// 第几次写入尝试。
        attempt: u32,
        /// 读回 A 摘要。
        first_sha: String,
        /// 读回 B 摘要。
        second_sha: String,
    },
    /// 重写耗尽仍一致失配：残件已删除，失败关闭。
    ReadbackMismatch {
        /// 已执行的重写次数。
        rewrites: u32,
        /// 登记时刻摘要。
        registered: String,
        /// 末次读回摘要。
        readback: String,
    },
}

impl fmt::Display for VerifiedWriteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "写入 I/O 失败：{error}"),
            Self::SourceCorrupted { registered, actual } => write!(
                formatter,
                "源缓冲在登记后被损坏（登记 {registered}，实际 {actual}），失败关闭不重试"
            ),
            Self::ReadbackDivergent {
                attempt,
                first_sha,
                second_sha,
            } => write!(
                formatter,
                "第 {attempt} 次尝试两路读回不一致（{first_sha} ≠ {second_sha}），失败关闭不重试"
            ),
            Self::ReadbackMismatch {
                rewrites,
                registered,
                readback,
            } => write!(
                formatter,
                "重写 {rewrites} 次耗尽仍读回失配（登记 {registered}，读回 {readback}），残件已删除"
            ),
        }
    }
}

impl std::error::Error for VerifiedWriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// 写读操作接缝：生产走文件系统，测试注入受控故障（#954 故障注入矩阵）。
trait ArtifactIo {
    /// 写出完整内容（新建或覆盖）。
    fn write_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    /// 读回 A：一次性整体读（缓存路径）。
    fn read_once(&self, path: &Path) -> io::Result<Vec<u8>>;
    /// 把缓存内容刷向持久层。
    fn sync(&self, path: &Path) -> io::Result<()>;
    /// 读回 B：flush 后独立流式读。与读回 A 走不同实现路径；两路不一致即分歧信号。
    /// 平台限制：普通文件 API 无法绕过页缓存直接核对持久介质，缓存—持久分歧面
    /// 在该面不可见时显式不覆盖。
    fn read_persistent(&self, path: &Path) -> io::Result<Vec<u8>>;
    /// 删除残件。
    fn remove_file(&self, path: &Path) -> io::Result<()>;
}

struct FsIo;

impl ArtifactIo for FsIo {
    fn write_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        fs::write(path, bytes)
    }

    fn read_once(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }

    fn sync(&self, path: &Path) -> io::Result<()> {
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?
            .sync_all()
    }

    fn read_persistent(&self, path: &Path) -> io::Result<Vec<u8>> {
        let mut file = fs::File::open(path)?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;
        Ok(buffer)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }
}

/// 写入 `source` 并按 [`SourceRegistration`] 校验、有界重写。
///
/// # Errors
///
/// - [`VerifiedWriteError::SourceCorrupted`]：源缓冲在登记后损坏（不重试）；
/// - [`VerifiedWriteError::ReadbackDivergent`]：两路读回不一致（不重试）；
/// - [`VerifiedWriteError::ReadbackMismatch`]：重写耗尽仍失配（残件已删除）；
/// - [`VerifiedWriteError::Io`]：文件 I/O 失败（残件已尽力删除）。
pub fn write_verified(
    path: &Path,
    source: &[u8],
    registration: &SourceRegistration,
    policy: WritePolicy,
) -> Result<VerifiedWrite, VerifiedWriteError> {
    write_verified_with(&FsIo, path, source, registration, policy)
}

fn write_verified_with(
    io: &dyn ArtifactIo,
    path: &Path,
    source: &[u8],
    registration: &SourceRegistration,
    policy: WritePolicy,
) -> Result<VerifiedWrite, VerifiedWriteError> {
    let name = path.file_name().map_or_else(
        || "<unnamed>".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let mut rewrites = 0_u32;
    loop {
        // 登记后源缓冲损坏：失败关闭、不重写（内存失真可破坏 &[u8] 背后的物理页）。
        let source_now: [u8; 32] = Sha256::digest(source).into();
        if source_now != registration.digest {
            return Err(VerifiedWriteError::SourceCorrupted {
                registered: registration.hex(),
                actual: hex(&source_now),
            });
        }
        let attempt = rewrites + 1;
        if let Err(error) = io.write_file(path, source) {
            let _ = io.remove_file(path);
            return Err(VerifiedWriteError::Io(error));
        }
        let first = read_or_clean(io, path)?;
        let first_sha: [u8; 32] = Sha256::digest(&first).into();
        io.sync(path).map_err(|error| {
            let _ = io.remove_file(path);
            VerifiedWriteError::Io(error)
        })?;
        let second = io.read_persistent(path).map_err(|error| {
            let _ = io.remove_file(path);
            VerifiedWriteError::Io(error)
        })?;
        let second_sha: [u8; 32] = Sha256::digest(&second).into();
        if first_sha != second_sha {
            let _ = io.remove_file(path);
            return Err(VerifiedWriteError::ReadbackDivergent {
                attempt,
                first_sha: hex(&first_sha),
                second_sha: hex(&second_sha),
            });
        }
        if first_sha == registration.digest {
            return Ok(VerifiedWrite { rewrites });
        }
        let divergence = stream_divergence(source, &first);
        let action = if rewrites >= policy.max_rewrites {
            "fail-closed"
        } else {
            "retry"
        };
        std::eprintln!(
            "[digest-probe] site=write-readback name={name} attempt={attempt} registered_sha={} readback_sha={} first_diff_offset={:?} differing_bytes={} len_delta={} reread_same=true action={action}",
            registration.hex(),
            hex(&first_sha),
            divergence.first_diff_offset,
            divergence.differing_bytes,
            divergence.len_delta,
        );
        let _ = io.remove_file(path);
        if rewrites >= policy.max_rewrites {
            return Err(VerifiedWriteError::ReadbackMismatch {
                rewrites,
                registered: registration.hex(),
                readback: hex(&first_sha),
            });
        }
        rewrites += 1;
    }
}

fn read_or_clean(io: &dyn ArtifactIo, path: &Path) -> Result<Vec<u8>, VerifiedWriteError> {
    io.read_once(path).map_err(|error| {
        let _ = io.remove_file(path);
        VerifiedWriteError::Io(error)
    })
}

/// 与登记摘要一致的源缓冲的分歧统计（诊断用；源缓冲已复核与登记一致）。
struct Divergence {
    first_diff_offset: Option<usize>,
    differing_bytes: usize,
    len_delta: i64,
}

fn stream_divergence(expected: &[u8], readback: &[u8]) -> Divergence {
    let common = expected.len().min(readback.len());
    let first_diff_offset = (0..common).find(|&index| expected[index] != readback[index]);
    let differing_bytes = (0..common)
        .filter(|&index| expected[index] != readback[index])
        .count();
    Divergence {
        first_diff_offset,
        differing_bytes,
        len_delta: readback.len() as i64 - expected.len() as i64,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use tempfile::tempdir;

    #[derive(Clone, Copy)]
    enum Corruption {
        /// 用另一段内容顶替中段（页替换形态）。
        PageSubstitute,
        /// 中段清零（零页形态）。
        ZeroPage,
        /// 截断尾部。
        Truncate,
    }

    /// 可编程故障注入（#954 故障注入矩阵）。
    struct FaultyIo {
        corrupt_writes: Cell<u32>,
        corruption: Corruption,
        divergent_read_b: Cell<bool>,
        writes: Cell<u32>,
    }

    impl FaultyIo {
        fn new(corruption: Corruption) -> Self {
            Self {
                corrupt_writes: Cell::new(0),
                corruption,
                divergent_read_b: Cell::new(false),
                writes: Cell::new(0),
            }
        }
    }

    impl ArtifactIo for FaultyIo {
        fn write_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            self.writes.set(self.writes.get() + 1);
            let mut bytes = bytes.to_vec();
            if self.corrupt_writes.get() > 0 {
                self.corrupt_writes.set(self.corrupt_writes.get() - 1);
                corrupt(&mut bytes, self.corruption);
            }
            fs::write(path, bytes)
        }

        fn read_once(&self, path: &Path) -> io::Result<Vec<u8>> {
            fs::read(path)
        }

        fn sync(&self, _path: &Path) -> io::Result<()> {
            Ok(())
        }

        fn read_persistent(&self, path: &Path) -> io::Result<Vec<u8>> {
            let mut bytes = fs::read(path)?;
            if self.divergent_read_b.get()
                && let Some(byte) = bytes.first_mut()
            {
                *byte ^= 0xFF;
            }
            Ok(bytes)
        }

        fn remove_file(&self, path: &Path) -> io::Result<()> {
            fs::remove_file(path)
        }
    }

    fn corrupt(bytes: &mut Vec<u8>, mode: Corruption) {
        match mode {
            Corruption::PageSubstitute => {
                let source = bytes.len() / 2;
                let target = bytes.len() / 4;
                let span = bytes.len() / 8;
                bytes.copy_within(source..source + span, target);
            }
            Corruption::ZeroPage => {
                let target = bytes.len() / 4;
                let span = bytes.len() / 8;
                bytes[target..target + span].fill(0);
            }
            Corruption::Truncate => {
                bytes.truncate(bytes.len() / 2);
            }
        }
    }

    fn source() -> Vec<u8> {
        (0..8_192).map(|index| (index % 251) as u8).collect()
    }

    #[test]
    fn clean_write_passes_on_first_attempt() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("artifact.bin");
        let io = FaultyIo::new(Corruption::Truncate);
        let source = source();
        let written = write_verified_with(
            &io,
            &path,
            &source,
            &SourceRegistration::of(&source),
            WritePolicy::DEFAULT,
        )
        .expect("clean write");
        assert_eq!(written.rewrites, 0, "no rewrites");
        assert_eq!(io.writes.get(), 1, "single write");
        assert_eq!(fs::read(&path).expect("read back"), source, "content");
    }

    #[test]
    fn corrupted_write_recovers_by_rewrite_for_each_shape() {
        for mode in [
            Corruption::PageSubstitute,
            Corruption::ZeroPage,
            Corruption::Truncate,
        ] {
            let directory = tempdir().expect("tempdir");
            let path = directory.path().join("artifact.bin");
            let io = FaultyIo::new(mode);
            io.corrupt_writes.set(1);
            let source = source();
            let written = write_verified_with(
                &io,
                &path,
                &source,
                &SourceRegistration::of(&source),
                WritePolicy::DEFAULT,
            )
            .expect("recover by rewrite");
            assert_eq!(written.rewrites, 1, "one rewrite");
            assert_eq!(io.writes.get(), 2, "corrupted then clean write");
            assert_eq!(fs::read(&path).expect("read back"), source, "content");
        }
    }

    #[test]
    fn persistent_corruption_exhausts_and_removes_the_file() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("artifact.bin");
        let io = FaultyIo::new(Corruption::ZeroPage);
        io.corrupt_writes.set(u32::MAX);
        let source = source();
        let error = write_verified_with(
            &io,
            &path,
            &source,
            &SourceRegistration::of(&source),
            WritePolicy::DEFAULT,
        )
        .expect_err("exhaustion fails");
        assert!(
            matches!(
                error,
                VerifiedWriteError::ReadbackMismatch { rewrites: 3, .. }
            ),
            "bounded retries exhausted"
        );
        assert_eq!(io.writes.get(), 4, "initial write plus three rewrites");
        assert!(!path.exists(), "no unverified artifact left behind");
    }

    #[test]
    fn source_corruption_after_registration_fails_closed_without_write() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("artifact.bin");
        let io = FaultyIo::new(Corruption::Truncate);
        let registered_source = source();
        let registration = SourceRegistration::of(&registered_source);
        let mut corrupted = registered_source.clone();
        corrupt(&mut corrupted, Corruption::PageSubstitute);
        let error =
            write_verified_with(&io, &path, &corrupted, &registration, WritePolicy::DEFAULT)
                .expect_err("corrupted source fails");
        assert!(
            matches!(error, VerifiedWriteError::SourceCorrupted { .. }),
            "source corruption detected"
        );
        assert_eq!(io.writes.get(), 0, "never written");
        assert!(!path.exists(), "no artifact created");
    }

    #[test]
    fn divergent_readback_paths_fail_closed_without_retry() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("artifact.bin");
        let io = FaultyIo::new(Corruption::Truncate);
        io.divergent_read_b.set(true);
        let source = source();
        let error = write_verified_with(
            &io,
            &path,
            &source,
            &SourceRegistration::of(&source),
            WritePolicy::DEFAULT,
        )
        .expect_err("divergent readback fails");
        assert!(
            matches!(
                error,
                VerifiedWriteError::ReadbackDivergent { attempt: 1, .. }
            ),
            "divergence detected on first attempt"
        );
        assert_eq!(io.writes.get(), 1, "no retry");
        assert!(!path.exists(), "no artifact left behind");
    }

    #[test]
    fn rewrite_count_is_reported_for_recovery_telemetry() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("artifact.bin");
        let io = FaultyIo::new(Corruption::Truncate);
        io.corrupt_writes.set(2);
        let source = source();
        let written = write_verified_with(
            &io,
            &path,
            &source,
            &SourceRegistration::of(&source),
            WritePolicy::DEFAULT,
        )
        .expect("recover after two rewrites");
        assert_eq!(written.rewrites, 2, "two rewrites reported");
    }
}
