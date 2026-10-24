//! 写时摘要与写后读回核对（#1042）。
//!
//! 运行产物的记录摘要按**写出时**的字节计算，flush 后再从磁盘读回一次核对；
//! 不再把写后从磁盘读回的结果当作唯一期望，失真内容不会连同它的摘要一起被认证。
//!
//! - 流式无源（[`DigestWriter::finish`]）：失配即失败关闭，失配文件留在原位供取证；
//! - 源可重放（[`write_replayable`]）：从同一份内存数据有界重写，以首次写出的摘要为
//!   唯一期望；
//! - 全量写出（[`write_verified`]）：经 `verified_write`（#954）写后两路读回、有界重写。
//!
//! 记录摘要为文件字节摘要（[`Domain::File`]，BLAKE3-256）。失配输出
//! `[digest-probe] site=stream-readback` 现场行，按 4 KiB 文件页给出首个不一致页偏移与
//! 不一致页数；页指纹为瞬态指纹（XXH3-64，ADR 0033 §2）：只在本进程内定位，不写入
//! 任何产物，一致与否以完整摘要为准。

use std::{
    fs::File,
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
};

use laneflow_urban_generator::verified_write;
use twox_hash::XxHash3_64;

use crate::{
    Result,
    digest::{Domain, FileDigest},
    hex, invalid,
};

/// 页指纹粒度：与页级失真的观测粒度一致。
const PAGE: usize = 4 * 1024;
/// 读回窗口大小。
const READ_WINDOW: usize = 64 * 1024;
/// 未指定容量时的写缓冲大小（与 `BufWriter` 默认值相同）。
#[cfg(any(test, feature = "adapter"))]
const DEFAULT_CAPACITY: usize = 8 * 1024;
/// 可重放写出的重写上限，与 `verified_write::WritePolicy::DEFAULT` 一致。
const MAX_REWRITES: u32 = 3;

/// 一次写出（或读回）的内容指纹：文件字节摘要、长度与逐页指纹。
#[derive(Clone, PartialEq, Eq)]
struct Fingerprint {
    digest: [u8; 32],
    bytes: u64,
    pages: Vec<u64>,
}

impl Fingerprint {
    fn digest(&self) -> FileDigest {
        FileDigest {
            bytes: self.bytes,
            digest: hex(&self.digest),
        }
    }
}

/// 增量累计完整摘要与逐页指纹。
struct Tally {
    hash: blake3::Hasher,
    bytes: u64,
    pages: Vec<u64>,
    page: Vec<u8>,
}

impl Tally {
    fn new() -> Self {
        Self {
            hash: Domain::File.hasher(),
            bytes: 0,
            pages: Vec::new(),
            page: Vec::with_capacity(PAGE),
        }
    }

    fn push(&mut self, mut bytes: &[u8]) {
        self.hash.update(bytes);
        self.bytes += bytes.len() as u64;
        // 页缓冲为空时整页直接从切片取指纹，不拷贝。
        while self.page.is_empty() && bytes.len() >= PAGE {
            self.pages.push(page_fingerprint(&bytes[..PAGE]));
            bytes = &bytes[PAGE..];
        }
        while !bytes.is_empty() {
            let take = (PAGE - self.page.len()).min(bytes.len());
            self.page.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.page.len() == PAGE {
                self.pages.push(page_fingerprint(&self.page));
                self.page.clear();
            }
        }
    }

    fn finish(mut self) -> Fingerprint {
        if !self.page.is_empty() {
            self.pages.push(page_fingerprint(&self.page));
        }
        Fingerprint {
            digest: *self.hash.finalize().as_bytes(),
            bytes: self.bytes,
            pages: self.pages,
        }
    }
}

/// 页指纹：XXH3-64，长度参与哈希，截断的末页与原末页可区分。
fn page_fingerprint(page: &[u8]) -> u64 {
    XxHash3_64::oneshot(page)
}

/// 带写缓冲的文件写出器，对写入文件的全部字节同步累计摘要与页指纹。
///
/// 累计发生在缓冲之下的文件层：只对实际交给文件的字节计算，且每次是写缓冲刷出的
/// 大块，BLAKE3 可跨块并行、整页指纹免拷贝（逐行累计实测慢约一倍）。
pub(crate) struct DigestWriter {
    path: PathBuf,
    inner: BufWriter<TallyFile>,
}

/// 文件层写入：把成功写入文件的字节送入累计。
struct TallyFile {
    file: File,
    tally: Tally,
}

impl Write for TallyFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.file.write(bytes)?;
        self.tally.push(&bytes[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl DigestWriter {
    #[cfg(any(test, feature = "adapter"))]
    pub(crate) fn create(path: &Path) -> Result<Self> {
        Self::with_capacity(path, DEFAULT_CAPACITY)
    }

    pub(crate) fn with_capacity(path: &Path, capacity: usize) -> Result<Self> {
        let file = TallyFile {
            file: File::create(path)?,
            tally: Tally::new(),
        };
        Ok(Self {
            path: path.to_owned(),
            inner: BufWriter::with_capacity(capacity, file),
        })
    }

    /// 刷盘并释放写缓冲，返回写出指纹。
    fn close(self) -> Result<(PathBuf, Fingerprint)> {
        let Self { path, inner } = self;
        let file = inner.into_inner().map_err(io::IntoInnerError::into_error)?;
        Ok((path, file.tally.finish()))
    }

    /// 刷盘后读回核对：一致时返回写出摘要；失配时输出现场行并失败关闭（流式无源，
    /// 不重写），失配文件留在原位供取证。
    pub(crate) fn finish(self) -> Result<FileDigest> {
        let (path, written) = self.close()?;
        after_write(&path);
        let readback = read_back(&path)?;
        if readback == written {
            return Ok(written.digest());
        }
        Err(mismatch(&path, 1, &written, &readback, "fail-closed"))
    }
}

impl Write for DigestWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.inner.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// 源数据仍在内存时的流式写出：失配后用 `fill` 从同一份数据重写，以首次写出的
/// 摘要为唯一期望，最多重写 [`MAX_REWRITES`] 次；重写时写出内容与首次不同说明
/// 源数据已变，失败关闭。
pub(crate) fn write_replayable(
    path: &Path,
    capacity: usize,
    mut fill: impl FnMut(&mut DigestWriter) -> Result<()>,
) -> Result<FileDigest> {
    let mut write = || -> Result<Fingerprint> {
        let mut writer = DigestWriter::with_capacity(path, capacity)?;
        fill(&mut writer)?;
        Ok(writer.close()?.1)
    };
    let expected = write()?;
    let mut attempt = 1;
    loop {
        after_write(path);
        let readback = read_back(path)?;
        if readback == expected {
            return Ok(expected.digest());
        }
        if attempt > MAX_REWRITES {
            return Err(mismatch(path, attempt, &expected, &readback, "fail-closed"));
        }
        let _ = mismatch(path, attempt, &expected, &readback, "retry");
        attempt += 1;
        let rewritten = write()?;
        if rewritten != expected {
            eprintln!(
                "[digest-probe] site=stream-readback name={} attempt={attempt} written_sha={} rewritten_sha={} reason=source-changed action=fail-closed",
                file_name(path),
                hex(&expected.digest),
                hex(&rewritten.digest),
            );
            return Err(invalid(format!(
                "{} 重写内容与首次写出不一致：源数据已变",
                file_name(path)
            )));
        }
    }
}

/// 全量写出：经 `verified_write` 写后两路读回、有界重写；记录摘要取自源字节。
pub(crate) fn write_verified(path: &Path, bytes: &[u8]) -> Result<FileDigest> {
    verified_write::write_verified(
        path,
        bytes,
        &verified_write::SourceRegistration::of(bytes),
        verified_write::WritePolicy::DEFAULT,
    )
    .map_err(|error| invalid(format!("write-readback {}: {error}", path.display())))?;
    Ok(FileDigest::of(bytes))
}

/// `serde_json::to_writer_pretty` 加换行的同字节序列化，经 [`write_verified`] 写出。
pub(crate) fn write_json_verified(
    path: &Path,
    value: &impl serde::Serialize,
) -> Result<FileDigest> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_verified(path, &bytes)
}

fn read_back(path: &Path) -> Result<Fingerprint> {
    let mut file = File::open(path)?;
    let mut tally = Tally::new();
    let mut window = vec![0_u8; READ_WINDOW];
    loop {
        let read = file.read(&mut window)?;
        if read == 0 {
            return Ok(tally.finish());
        }
        tally.push(&window[..read]);
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || "<unnamed>".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// 输出失配现场行并构造错误。
fn mismatch(
    path: &Path,
    attempt: u32,
    written: &Fingerprint,
    readback: &Fingerprint,
    action: &str,
) -> crate::Error {
    let pages = written.pages.len().max(readback.pages.len());
    let differing =
        (0..pages).filter(|&index| written.pages.get(index) != readback.pages.get(index));
    let first = differing.clone().next().map(|index| index * PAGE);
    let name = file_name(path);
    eprintln!(
        "[digest-probe] site=stream-readback name={name} attempt={attempt} written_sha={} readback_sha={} written_bytes={} readback_bytes={} first_diff_page_offset={first:?} differing_pages={} action={action}",
        hex(&written.digest),
        hex(&readback.digest),
        written.bytes,
        readback.bytes,
        differing.count(),
    );
    invalid(format!(
        "{name} 写后读回与写出内容不一致（written_sha={} readback_sha={}）",
        hex(&written.digest),
        hex(&readback.digest),
    ))
}

/// 测试注入：写出刷盘之后、读回核对之前对文件执行的篡改。
#[cfg(test)]
type AfterWrite = Box<dyn FnMut(&Path)>;

#[cfg(test)]
thread_local! {
    static AFTER_WRITE: std::cell::RefCell<Option<AfterWrite>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn after_write(path: &Path) {
    AFTER_WRITE.with_borrow_mut(|hook| {
        if let Some(hook) = hook {
            hook(path);
        }
    });
}

#[cfg(not(test))]
fn after_write(_: &Path) {}

/// 在调用线程上安装写后篡改，执行 `f` 后卸下（含 panic）。
#[cfg(test)]
pub(crate) fn with_after_write<R>(hook: impl FnMut(&Path) + 'static, f: impl FnOnce() -> R) -> R {
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            AFTER_WRITE.set(None);
        }
    }
    AFTER_WRITE.set(Some(Box::new(hook)));
    let _restore = Restore;
    f()
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, fs, rc::Rc};

    use super::*;

    fn content(len: usize) -> Vec<u8> {
        (0..len).map(|i| b'a' + (i % 23) as u8).collect()
    }

    fn stream(path: &Path, bytes: &[u8]) -> Result<FileDigest> {
        let mut writer = DigestWriter::create(path)?;
        // 小块写入，覆盖跨页累计。
        for chunk in bytes.chunks(1_000) {
            writer.write_all(chunk)?;
        }
        writer.finish()
    }

    type Tamper = (&'static str, fn(&mut Vec<u8>));

    /// 页替换（第 1 页变为第 2 页内容）、零页与截断三种篡改。
    fn tampers() -> [Tamper; 3] {
        [
            ("shifted", |bytes| {
                let next = bytes[2 * PAGE..3 * PAGE].to_vec();
                bytes[PAGE..2 * PAGE].copy_from_slice(&next);
            }),
            ("zero", |bytes| bytes[PAGE..2 * PAGE].fill(0)),
            ("truncated", |bytes| bytes.truncate(PAGE + 100)),
        ]
    }

    fn tamper_file(path: &Path, change: fn(&mut Vec<u8>)) {
        let mut bytes = fs::read(path).unwrap();
        change(&mut bytes);
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn stream_digest_matches_readback_digest_without_tampering() {
        let temp = tempfile::tempdir().unwrap();
        for len in [0, 1, PAGE - 1, PAGE, 3 * PAGE + 17] {
            let path = temp.path().join(format!("{len}.jsonl"));
            let bytes = content(len);
            let digest = stream(&path, &bytes).unwrap();
            assert_eq!(digest, crate::report::digest_file(&path).unwrap());
            assert_eq!(digest, FileDigest::of(&bytes));
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn stream_readback_mismatch_fails_closed_and_keeps_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let bytes = content(4 * PAGE);
        for (label, change) in tampers() {
            let path = temp.path().join(format!("{label}.jsonl"));
            let error = with_after_write(
                move |path| tamper_file(path, change),
                || stream(&path, &bytes),
            )
            .unwrap_err();
            assert!(
                error.to_string().contains("写后读回与写出内容不一致"),
                "{label}: {error}"
            );
            let mut expected = bytes.clone();
            change(&mut expected);
            assert_eq!(fs::read(&path).unwrap(), expected, "{label}");
        }
    }

    #[test]
    fn tally_is_independent_of_how_bytes_are_split() {
        let bytes = content(5 * PAGE + 123);
        let whole = {
            let mut tally = Tally::new();
            tally.push(&bytes);
            tally.finish()
        };
        // 整页免拷贝与跨页拷贝两条路径交替出现。
        for split in [1, 7, PAGE - 1, PAGE, PAGE + 1, 3 * PAGE + 5] {
            let mut tally = Tally::new();
            for chunk in bytes.chunks(split) {
                tally.push(chunk);
            }
            assert!(tally.finish() == whole, "split {split}");
        }
        let mut tally = Tally::new();
        tally.push(&bytes[..10]);
        tally.push(&bytes[10..]);
        assert!(tally.finish() == whole, "unaligned head then bulk");
    }

    #[test]
    fn page_fingerprints_locate_the_first_differing_page() {
        let bytes = content(4 * PAGE);
        let mut tally = Tally::new();
        tally.push(&bytes);
        let written = tally.finish();
        for (label, change, first, differing) in [
            ("shifted", tampers()[0].1, 1, 1),
            ("zero", tampers()[1].1, 1, 1),
            ("truncated", tampers()[2].1, 1, 3),
        ] {
            let mut changed = bytes.clone();
            change(&mut changed);
            let mut tally = Tally::new();
            tally.push(&changed);
            let readback = tally.finish();
            let pages = written.pages.len().max(readback.pages.len());
            let diffs = (0..pages)
                .filter(|&i| written.pages.get(i) != readback.pages.get(i))
                .collect::<Vec<_>>();
            assert_eq!(diffs.first(), Some(&first), "{label}");
            assert_eq!(diffs.len(), differing, "{label}");
        }
    }

    #[test]
    fn replayable_write_rewrites_after_a_mismatch_and_fails_closed_when_exhausted() {
        let temp = tempfile::tempdir().unwrap();
        let bytes = content(3 * PAGE);
        let fill = |bytes: Vec<u8>| {
            move |writer: &mut DigestWriter| -> Result<()> {
                writer.write_all(&bytes)?;
                Ok(())
            }
        };

        // 首次写出被篡改，重写一次后通过。
        let path = temp.path().join("recovered.jsonl");
        let calls = Rc::new(Cell::new(0));
        let seen = Rc::clone(&calls);
        let digest = with_after_write(
            move |path| {
                seen.set(seen.get() + 1);
                if seen.get() == 1 {
                    tamper_file(path, tampers()[1].1);
                }
            },
            || write_replayable(&path, DEFAULT_CAPACITY, fill(bytes.clone())),
        )
        .unwrap();
        assert_eq!(calls.get(), 2);
        assert_eq!(digest, FileDigest::of(&bytes));
        assert_eq!(fs::read(&path).unwrap(), bytes);

        // 每次写出都被篡改：首次加 3 次重写后失败关闭。
        let path = temp.path().join("exhausted.jsonl");
        let calls = Rc::new(Cell::new(0));
        let seen = Rc::clone(&calls);
        let error = with_after_write(
            move |path| {
                seen.set(seen.get() + 1);
                tamper_file(path, tampers()[0].1);
            },
            || write_replayable(&path, DEFAULT_CAPACITY, fill(bytes.clone())),
        )
        .unwrap_err();
        assert_eq!(calls.get(), 1 + MAX_REWRITES);
        assert!(error.to_string().contains("写后读回与写出内容不一致"));
    }

    #[test]
    fn replayable_write_fails_closed_when_the_source_changes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("changed.jsonl");
        let bytes = content(2 * PAGE);
        let mut round = 0_u8;
        let error = with_after_write(
            |path| tamper_file(path, |bytes| bytes[0] ^= 1),
            || {
                write_replayable(&path, DEFAULT_CAPACITY, |writer| {
                    round += 1;
                    writer.write_all(&bytes)?;
                    writer.write_all(&[round])?;
                    Ok(())
                })
            },
        )
        .unwrap_err();
        assert_eq!(round, 2);
        assert!(error.to_string().contains("源数据已变"), "{error}");
    }

    #[test]
    fn json_verified_write_matches_the_pretty_writer() {
        let temp = tempfile::tempdir().unwrap();
        let value = serde_json::json!({"b":[1,2,3],"a":{"nested":"值"}});
        let path = temp.path().join("value.json");
        let digest = write_json_verified(&path, &value).unwrap();
        let reference = temp.path().join("reference.json");
        crate::report::write_json(&reference, &value).unwrap();
        assert_eq!(fs::read(&path).unwrap(), fs::read(&reference).unwrap());
        assert_eq!(digest, crate::report::digest_file(&reference).unwrap());
    }
}
