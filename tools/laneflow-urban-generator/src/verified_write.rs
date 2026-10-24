//! 写后回读校验与有界重写（#954）。
//!
//! 合同（G0 冻结）：
//!
//! 1. **唯一期望 = 登记时刻摘要**：[`SourceRegistration::of`] 在写前对源缓冲冻结
//!    SHA-256；之后一切读回判定只对照该登记摘要，绝不从可疑缓冲重算期望；
//! 2. **登记后源缓冲损坏即失败关闭**：每次写入/重写前复核源缓冲仍与登记摘要一致，
//!    不一致时不重试、直接失败并输出现场；
//! 3. **两路读回分歧即失败关闭**：读回 A（写后立即）与读回 B（flush 后独立重新打开）
//!    两次独立流式读必须给出同一内容且命中登记摘要；两路不一致说明读路径本身不可
//!    信，失败关闭、不重试。独立性边界如实声明：两路为同一读实现、不同时刻与不同
//!    打开；普通文件 API 无法绕过页缓存直接核对持久介质，缓存—持久分歧面在该面
//!    不可见时显式不覆盖（见 [`ArtifactIo::stream_file`]）；
//! 4. **重写有界**：两路一致但与登记摘要不符时允许从源缓冲重写，上限固定
//!    （[`WritePolicy::DEFAULT`] = 3 次）；
//! 5. **耗尽删残件、终态失败关闭**：`path` 语义为「已验证工件或空」——调用方应视
//!    路径为本操作所有；任何失败退出（含源缓冲损坏）都尽力清理路径上的全部字节
//!    （含预存文件），清理失败经 [`VerifiedWriteError`] 的 `residual` 字段与
//!    `[digest-probe]` 现场行显式上浮，不谎称已清理、不静默留可被消费的未验证文件。
//!
//! 成本契约：读回全程 64 KiB 窗口流式（增量哈希 + 增量分歧统计），不整体持有回读
//! 副本；验证峰值与工件大小无关（#956 审阅）。成功判定包含 flush 后读回；**不含断电
//! 耐久与目录项耐久**（rename、目录 fsync 与发布事务属宿主 / 发布边界，见 compiler
//! 后发射设计）。
//!
//! 失配与分歧现场均以 `[digest-probe] site=write-readback` 行输出（含分歧偏移、
//! 分歧字节数、长度差与 action），与 `docs/reference/validation-matrix.md` 判读
//! 口径一致；成功重写次数由 [`VerifiedWrite::rewrites`] 提供恢复遥测。

use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

/// 下游测试专用的写后故障注入；默认构建不包含此模块（#1043）。
#[cfg(feature = "verified-write-test-support")]
pub mod test_support;

/// 读回窗口大小：限界内存的流式比对单元。
const READ_WINDOW: usize = 64 * 1024;

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
    /// 文件 I/O 失败。
    Io {
        /// 底层 I/O 错误。
        error: io::Error,
        /// 残件是否删除失败（可能残留于路径）。
        residual: bool,
    },
    /// 源缓冲在登记后被损坏（与登记摘要不符）：不重试，失败关闭。
    SourceCorrupted {
        /// 登记时刻摘要。
        registered: String,
        /// 实际源缓冲摘要。
        actual: String,
        /// 残件是否删除失败（可能残留于路径）。
        residual: bool,
    },
    /// 两路读回不一致：读路径不可信，不重试，失败关闭。
    ReadbackDivergent {
        /// 第几次写入尝试。
        attempt: u32,
        /// 读回 A 摘要。
        first_sha: String,
        /// 读回 B 摘要。
        second_sha: String,
        /// 残件是否删除失败（可能残留于路径）。
        residual: bool,
    },
    /// 重写耗尽仍一致失配：失败关闭。
    ReadbackMismatch {
        /// 已执行的重写次数。
        rewrites: u32,
        /// 登记时刻摘要。
        registered: String,
        /// 末次读回摘要。
        readback: String,
        /// 残件是否删除失败（可能残留于路径）。
        residual: bool,
    },
}

impl fmt::Display for VerifiedWriteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let residual_note = |residual: &bool| {
            if *residual {
                "；残件删除失败，可能残留于路径"
            } else {
                "；残件已删除"
            }
        };
        match self {
            Self::Io { error, residual } => write!(
                formatter,
                "写入 I/O 失败：{error}{}",
                residual_note(residual)
            ),
            Self::SourceCorrupted {
                registered,
                actual,
                residual,
            } => write!(
                formatter,
                "源缓冲在登记后被损坏（登记 {registered}，实际 {actual}），失败关闭不重试{}",
                residual_note(residual)
            ),
            Self::ReadbackDivergent {
                attempt,
                first_sha,
                second_sha,
                residual,
            } => write!(
                formatter,
                "第 {attempt} 次尝试两路读回不一致（{first_sha} ≠ {second_sha}），失败关闭不重试{}",
                residual_note(residual)
            ),
            Self::ReadbackMismatch {
                rewrites,
                registered,
                readback,
                residual,
            } => write!(
                formatter,
                "重写 {rewrites} 次耗尽仍读回失配（登记 {registered}，读回 {readback}）{}",
                residual_note(residual)
            ),
        }
    }
}

impl std::error::Error for VerifiedWriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// 写读操作接缝：生产走文件系统，测试注入受控故障（#954 故障注入矩阵）。
trait ArtifactIo {
    /// 写出完整内容（新建或覆盖）。
    fn write_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    /// 流式读回：把文件内容按窗口交给 `sink`。实现不得整体持有文件副本（限界内存）；
    /// 同一实现的两次调用构成两路读回（不同时刻、不同打开）。
    fn stream_file(&self, path: &Path, sink: &mut dyn FnMut(&[u8])) -> io::Result<()>;
    /// 把缓存内容刷向持久层。
    fn sync(&self, path: &Path) -> io::Result<()>;
    /// 删除残件。
    fn remove_file(&self, path: &Path) -> io::Result<()>;
}

struct FsIo;

impl ArtifactIo for FsIo {
    fn write_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        fs::write(path, bytes)?;
        #[cfg(feature = "verified-write-test-support")]
        test_support::after_write(path)?;
        Ok(())
    }

    fn stream_file(&self, path: &Path, sink: &mut dyn FnMut(&[u8])) -> io::Result<()> {
        let mut file = fs::File::open(path)?;
        let mut window = [0_u8; READ_WINDOW];
        loop {
            let read = file.read(&mut window)?;
            if read == 0 {
                return Ok(());
            }
            sink(&window[..read]);
        }
    }

    fn sync(&self, path: &Path) -> io::Result<()> {
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?
            .sync_all()
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
/// - [`VerifiedWriteError::ReadbackMismatch`]：重写耗尽仍失配；
/// - [`VerifiedWriteError::Io`]：文件 I/O 失败。
///
/// 各变体的 `residual` 表示残件是否删除失败：为真时被拒字节可能仍留在 `path`，
/// 调用方**必须检查该字段**决定清理或隔离；为假时残件已确认删除（仅
/// `ErrorKind::NotFound` 视为确认不存在）。
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
    let mut residual = false;
    loop {
        let attempt = rewrites + 1;
        // 登记后源缓冲损坏：失败关闭、不重写（内存失真可破坏 &[u8] 背后的物理页）。
        let source_now: [u8; 32] = Sha256::digest(source).into();
        if source_now != registration.digest {
            // 路径语义为「已验证工件或空」：失败退出前清理一切残件（含预存文件）。
            remove_residual(io, path, &name, attempt, &mut residual, "fail-closed");
            emit_probe(format_args!(
                "[digest-probe] site=write-readback name={name} attempt={attempt} registered_sha={} actual_sha={} reason=source-corrupted action=fail-closed residual={residual}",
                registration.hex(),
                hex(&source_now),
            ));
            return Err(VerifiedWriteError::SourceCorrupted {
                registered: registration.hex(),
                actual: hex(&source_now),
                residual,
            });
        }
        if let Err(error) = io.write_file(path, source) {
            remove_residual(io, path, &name, attempt, &mut residual, "fail-closed");
            return Err(VerifiedWriteError::Io { error, residual });
        }
        // 读回 A：写后立即，窗口流式，增量哈希 + 增量分歧统计。
        let mut first_hasher = Sha256::new();
        let mut first_divergence = Divergence::new(source);
        stream_or_clean(io, path, &name, attempt, &mut residual, &mut |window| {
            first_hasher.update(window);
            first_divergence.push(window);
        })?;
        let first_sha: [u8; 32] = first_hasher.finalize().into();
        io.sync(path).map_err(|error| {
            remove_residual(io, path, &name, attempt, &mut residual, "fail-closed");
            VerifiedWriteError::Io { error, residual }
        })?;
        // 读回 B：flush 后独立重新打开，窗口流式，增量哈希 + 增量分歧统计
        // （分歧现场必须同时量到 B 路，否则指纹被 A-vs-源的全零统计掩盖）。
        let mut second_hasher = Sha256::new();
        let mut second_divergence = Divergence::new(source);
        stream_or_clean(io, path, &name, attempt, &mut residual, &mut |window| {
            second_hasher.update(window);
            second_divergence.push(window);
        })?;
        let second_sha: [u8; 32] = second_hasher.finalize().into();
        if first_sha != second_sha {
            // 清理先于诊断：探针输出不可用时也不阻断失败关闭路径。
            remove_residual(io, path, &name, attempt, &mut residual, "fail-closed");
            emit_probe(format_args!(
                "[digest-probe] site=write-readback name={name} attempt={attempt} registered_sha={} first_sha={} second_sha={} a_first_diff_offset={:?} a_differing_bytes={} a_len_delta={} a_expected_head32={:02x?} a_readback_head32={:02x?} b_first_diff_offset={:?} b_differing_bytes={} b_len_delta={} b_expected_head32={:02x?} b_readback_head32={:02x?} reason=divergent-readback action=fail-closed",
                registration.hex(),
                hex(&first_sha),
                hex(&second_sha),
                first_divergence.first_diff_offset,
                first_divergence.differing_bytes(),
                first_divergence.len_delta(),
                first_divergence.expected_sample(),
                first_divergence.readback_sample(),
                second_divergence.first_diff_offset,
                second_divergence.differing_bytes(),
                second_divergence.len_delta(),
                second_divergence.expected_sample(),
                second_divergence.readback_sample(),
            ));
            return Err(VerifiedWriteError::ReadbackDivergent {
                attempt,
                first_sha: hex(&first_sha),
                second_sha: hex(&second_sha),
                residual,
            });
        }
        if first_sha == registration.digest {
            return Ok(VerifiedWrite { rewrites });
        }
        let action = if rewrites >= policy.max_rewrites {
            "fail-closed"
        } else {
            "retry"
        };
        remove_residual(io, path, &name, attempt, &mut residual, action);
        emit_probe(format_args!(
            "[digest-probe] site=write-readback name={name} attempt={attempt} registered_sha={} readback_sha={} first_diff_offset={:?} differing_bytes={} len_delta={} written_bytes={} readback_bytes={} expected_head32={:02x?} readback_head32={:02x?} reread_same=true reason=consistent-mismatch action={action}",
            registration.hex(),
            hex(&first_sha),
            first_divergence.first_diff_offset,
            first_divergence.differing_bytes(),
            first_divergence.len_delta(),
            source.len(),
            first_divergence.offset,
            first_divergence.expected_sample(),
            first_divergence.readback_sample(),
        ));
        if rewrites >= policy.max_rewrites {
            return Err(VerifiedWriteError::ReadbackMismatch {
                rewrites,
                registered: registration.hex(),
                readback: hex(&first_sha),
                residual,
            });
        }
        rewrites += 1;
    }
}

/// 尽力输出探针现场行：stderr 不可写（如管道被读端关闭）时静默跳过，
/// 诊断绝不 panic、不阻断失败关闭路径（#956 审阅）。
fn emit_probe(line: std::fmt::Arguments<'_>) {
    use std::io::Write as _;
    let _ = io::stderr().write_fmt(format_args!("{line}\n"));
}

/// 删除残件并把结果写入 `residual`：删除失败经 `[digest-probe]` 现场行显式上浮，
/// 不静默丢弃；删除成功则清除残留标记（状态以末次清理结果为准）。`action` 如实
/// 反映本次清理后的流程走向（重试或失败关闭），不把仍会重试的清理失败标成
/// `fail-closed`。
fn remove_residual(
    io: &dyn ArtifactIo,
    path: &Path,
    name: &str,
    attempt: u32,
    residual: &mut bool,
    action: &str,
) {
    // 仅 `NotFound` 视为「已确认不存在」；其余删除错误一律按残留上浮——
    // 元数据查询同样失败时 `Path::exists()` 会误报不存在，不能充当删除已确认的证据。
    let failed = match io.remove_file(path) {
        Ok(()) => false,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(_) => true,
    };
    *residual = failed;
    if failed {
        emit_probe(format_args!(
            "[digest-probe] site=write-readback name={name} attempt={attempt} reason=cleanup-failed action={action}"
        ));
    }
}

fn stream_or_clean(
    io: &dyn ArtifactIo,
    path: &Path,
    name: &str,
    attempt: u32,
    residual: &mut bool,
    sink: &mut dyn FnMut(&[u8]),
) -> Result<(), VerifiedWriteError> {
    io.stream_file(path, sink).map_err(|error| {
        remove_residual(io, path, name, attempt, residual, "fail-closed");
        VerifiedWriteError::Io {
            error,
            residual: *residual,
        }
    })
}

/// 期望侧分歧统计累加器（限界内存；期望为已复核与登记摘要一致的源缓冲）。
/// 首个分歧处同时保留两侧 ≤32 字节样本，供零页 / 页位移指纹判读（失配文件会被
/// 删除，日志样本是唯一残留现场）。
struct Divergence<'a> {
    expected: &'a [u8],
    offset: usize,
    first_diff_offset: Option<usize>,
    differing_bytes: usize,
    expected_sample: [u8; 32],
    readback_sample: [u8; 32],
    expected_sample_len: usize,
    readback_sample_len: usize,
}

impl<'a> Divergence<'a> {
    fn new(expected: &'a [u8]) -> Self {
        Self {
            expected,
            offset: 0,
            first_diff_offset: None,
            differing_bytes: 0,
            expected_sample: [0; 32],
            readback_sample: [0; 32],
            expected_sample_len: 0,
            readback_sample_len: 0,
        }
    }

    fn push(&mut self, window: &[u8]) {
        for (index, byte) in window.iter().enumerate() {
            let index = self.offset + index;
            if index < self.expected.len() && self.expected[index] != *byte {
                self.differing_bytes += 1;
                if self.first_diff_offset.is_none() {
                    self.first_diff_offset = Some(index);
                    let expected_len = (self.expected.len() - index).min(32);
                    self.expected_sample[..expected_len]
                        .copy_from_slice(&self.expected[index..index + expected_len]);
                    self.expected_sample_len = expected_len;
                    let window_at = index - self.offset;
                    let readback_len = (window.len() - window_at).min(32);
                    self.readback_sample[..readback_len]
                        .copy_from_slice(&window[window_at..window_at + readback_len]);
                    self.readback_sample_len = readback_len;
                }
            }
        }
        self.offset += window.len();
    }

    fn differing_bytes(&self) -> usize {
        self.differing_bytes
    }

    fn len_delta(&self) -> i64 {
        self.offset as i64 - self.expected.len() as i64
    }

    /// 首个分歧处的期望侧样本（≤32 字节）。
    fn expected_sample(&self) -> &[u8] {
        &self.expected_sample[..self.expected_sample_len]
    }

    /// 首个分歧处的读回侧样本（≤32 字节；首窗内截断）。
    fn readback_sample(&self) -> &[u8] {
        &self.readback_sample[..self.readback_sample_len]
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
        fail_write: Cell<bool>,
        fail_remove: Cell<bool>,
        writes: Cell<u32>,
        streams: Cell<u32>,
    }

    impl FaultyIo {
        fn new(corruption: Corruption) -> Self {
            Self {
                corrupt_writes: Cell::new(0),
                corruption,
                divergent_read_b: Cell::new(false),
                fail_write: Cell::new(false),
                fail_remove: Cell::new(false),
                writes: Cell::new(0),
                streams: Cell::new(0),
            }
        }
    }

    impl ArtifactIo for FaultyIo {
        fn write_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            self.writes.set(self.writes.get() + 1);
            if self.fail_write.get() {
                return Err(io::Error::other("injected write failure"));
            }
            let mut bytes = bytes.to_vec();
            if self.corrupt_writes.get() > 0 {
                self.corrupt_writes.set(self.corrupt_writes.get() - 1);
                corrupt(&mut bytes, self.corruption);
            }
            fs::write(path, bytes)
        }

        fn stream_file(&self, path: &Path, sink: &mut dyn FnMut(&[u8])) -> io::Result<()> {
            self.streams.set(self.streams.get() + 1);
            let mut bytes = fs::read(path)?;
            if self.divergent_read_b.get()
                && self.streams.get() == 2
                && let Some(byte) = bytes.first_mut()
            {
                *byte ^= 0xFF;
            }
            for window in bytes.chunks(READ_WINDOW) {
                sink(window);
            }
            Ok(())
        }

        fn sync(&self, _path: &Path) -> io::Result<()> {
            Ok(())
        }

        fn remove_file(&self, path: &Path) -> io::Result<()> {
            if self.fail_remove.get() {
                return Err(io::Error::other("injected cleanup failure"));
            }
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
    fn preexisting_destination_is_cleared_on_source_corruption() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("artifact.bin");
        fs::write(&path, b"stale artifact").expect("stale file");
        let io = FaultyIo::new(Corruption::Truncate);
        let registered_source = source();
        let registration = SourceRegistration::of(&registered_source);
        let mut corrupted = registered_source;
        corrupt(&mut corrupted, Corruption::ZeroPage);
        let error =
            write_verified_with(&io, &path, &corrupted, &registration, WritePolicy::DEFAULT)
                .expect_err("corrupted source fails");
        assert!(
            matches!(
                error,
                VerifiedWriteError::SourceCorrupted {
                    residual: false,
                    ..
                }
            ),
            "cleared path reports no residual"
        );
        assert!(!path.exists(), "owned path cleared on failure");
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

    #[test]
    fn cleanup_failure_is_surfaced_not_swallowed() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("artifact.bin");
        let io = FaultyIo::new(Corruption::ZeroPage);
        io.corrupt_writes.set(u32::MAX);
        io.fail_remove.set(true);
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
                VerifiedWriteError::ReadbackMismatch {
                    rewrites: 3,
                    residual: true,
                    ..
                }
            ),
            "residual surfaced"
        );
        assert!(path.exists(), "rejected bytes remain on disk");
        assert!(
            error.to_string().contains("残件删除失败"),
            "display stays honest"
        );
    }

    #[test]
    fn io_error_reports_residual_when_cleanup_fails() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("artifact.bin");
        fs::write(&path, b"stale bytes").expect("stale artifact");
        let io = FaultyIo::new(Corruption::Truncate);
        io.fail_write.set(true);
        io.fail_remove.set(true);
        let source = source();
        let error = write_verified_with(
            &io,
            &path,
            &source,
            &SourceRegistration::of(&source),
            WritePolicy::DEFAULT,
        )
        .expect_err("write failure");
        assert!(
            matches!(error, VerifiedWriteError::Io { residual: true, .. }),
            "residual surfaced in io error"
        );
        assert!(path.exists(), "rejected bytes remain on disk");
    }

    #[test]
    fn confirmed_absence_is_not_reported_as_residual() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("missing.bin");
        let io = FaultyIo::new(Corruption::Truncate);
        let mut residual = false;
        remove_residual(&io, &path, "missing.bin", 1, &mut residual, "fail-closed");
        assert!(!residual, "not-found is confirmed absence");
    }
}
