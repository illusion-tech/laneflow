//! 本地 file-backed 对象的写能力闭合适配器。
//!
//! 本模块只负责临时 backing 的类型状态转换；不实现安装、rename、no-replace、目录耐久、
//! winner 竞争或 manifest 事务。平台私有临时文件与只读映射的 unsafe 边界由
//! `laneflow-format-mmap` 承载，本 crate 保持 `unsafe_code = "forbid"`。

use core::fmt;
use std::{
    boxed::Box,
    io::{self, BufWriter, Seek, SeekFrom, Write},
    path::Path,
    sync::{Arc, OnceLock},
};

use laneflow_format_mmap::{BackingError, PrivateStagedFile, SealedPrivateFile};
use laneflow_static_contract::ExactByteLength;

use crate::{
    BoundedReReadableObjectSource, ObjectSourceError, PreparedObject,
    object_source::private::SealedImmutableBacking,
    writer::{ObjectWriteSink, encode_prepared_object_to_sink},
};

/// 建立或关闭本地 staged object 时的失败。
#[derive(Debug)]
pub enum StagedObjectError {
    Io(io::Error),
    ArithmeticOverflow,
    BackingChanged,
}

impl From<io::Error> for StagedObjectError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// 字段私有的顺序 file-backed writer。
///
/// 构造时只接受已经完成全对象预检的 [`PreparedObject`]；不公开路径、`File`、
/// raw handle/fd 或可写映射。调用 [`Self::finish`] 后本能力被消费。
#[derive(Debug)]
pub struct StagedObjectWriter {
    staged: Option<PrivateStagedFile>,
    exact_byte_length: ExactByteLength,
}

impl StagedObjectWriter {
    /// 在调用方选择的临时目录中顺序编码一份对象。
    pub fn create_in(
        directory: &Path,
        prepared: PreparedObject<'_>,
    ) -> Result<Self, StagedObjectError> {
        let exact_byte_length = ExactByteLength::new(prepared.byte_len());
        let mut staged = PrivateStagedFile::create_in(directory).map_err(stage_error)?;
        if let Err(error) =
            encode_prepared_object_to_sink(prepared, FileSink::new(staged.file_mut()))
        {
            return Err(StagedObjectError::Io(error));
        }
        Ok(Self {
            staged: Some(staged),
            exact_byte_length,
        })
    }

    /// 排空用户态写缓冲、固定 exact length、关闭全部 LaneFlow 写能力，并返回只保留
    /// 私有 backing 的来源。
    pub fn finish(mut self) -> Result<ClosedStagedObjectSource, StagedObjectError> {
        let mut staged = self
            .staged
            .take()
            .expect("unfinished staged writer retains its file");
        staged.file_mut().flush()?;
        let sealed = staged
            .seal(self.exact_byte_length.get())
            .map_err(stage_error)?;

        Ok(ClosedStagedObjectSource {
            inner: Arc::new(ClosedBacking {
                backing: sealed,
                map: OnceLock::new(),
            }),
        })
    }
}

impl Drop for StagedObjectWriter {
    fn drop(&mut self) {
        drop(self.staged.take());
    }
}

struct ClosedBacking {
    // `tempfile_in` uses an anonymous/unlinked file on Unix and an exclusive delete-on-close file
    // on Windows. The handle remains private inside `SealedPrivateFile`, and this type has no
    // mutating operation after `StagedObjectWriter::finish`.
    backing: SealedPrivateFile,
    map: OnceLock<Result<laneflow_format_mmap::ReadOnlyMap, ObjectSourceError>>,
}

/// 已关闭 LaneFlow 写能力、只保留同一临时 file backing 的不可变来源。
#[derive(Clone)]
pub struct ClosedStagedObjectSource {
    inner: Arc<ClosedBacking>,
}

impl ClosedStagedObjectSource {
    #[must_use]
    pub fn exact_byte_length(&self) -> ExactByteLength {
        ExactByteLength::new(self.inner.backing.exact_byte_length())
    }

    /// 返回只读 exact bytes；首次调用惰性建立 read-only mapping。
    pub fn as_bytes(&self) -> Result<&[u8], ObjectSourceError> {
        SealedImmutableBacking::contiguous_bytes(self)
    }
}

impl fmt::Debug for ClosedStagedObjectSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClosedStagedObjectSource")
            .field("exact_byte_length", &self.exact_byte_length())
            .finish_non_exhaustive()
    }
}

impl PartialEq for ClosedStagedObjectSource {
    fn eq(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.inner, &other.inner) {
            return true;
        }
        if self.exact_byte_length() != other.exact_byte_length() {
            return false;
        }
        match (self.as_bytes(), other.as_bytes()) {
            (Ok(left), Ok(right)) => left == right,
            _ => false,
        }
    }
}

impl Eq for ClosedStagedObjectSource {}

impl SealedImmutableBacking for ClosedStagedObjectSource {
    fn contiguous_bytes(&self) -> Result<&[u8], ObjectSourceError> {
        let result = self.inner.map.get_or_init(|| {
            // 长度核对与平台私有性论证封在 `laneflow-format-mmap` 内；
            // 本模块在 finish 之后没有任何可达写能力。
            self.inner.backing.map_read_only().map_err(map_error)
        });
        result.as_ref().map(|map| &map[..]).map_err(|error| *error)
    }
}

impl BoundedReReadableObjectSource for ClosedStagedObjectSource {
    fn exact_byte_length(&self) -> ExactByteLength {
        ExactByteLength::new(self.inner.backing.exact_byte_length())
    }

    fn read_exact_at(&self, offset: u64, destination: &mut [u8]) -> Result<(), ObjectSourceError> {
        let bytes = self.as_bytes()?;
        let start = usize::try_from(offset).map_err(|_| ObjectSourceError::OutOfBounds)?;
        let end = start
            .checked_add(destination.len())
            .ok_or(ObjectSourceError::OutOfBounds)?;
        let source = bytes
            .get(start..end)
            .ok_or(ObjectSourceError::OutOfBounds)?;
        destination.copy_from_slice(source);
        Ok(())
    }
}

/// compiler 本地候选可使用的统一只读 backing；百万级生产路径使用 `Staged`。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImmutableObjectSource {
    Owned(Arc<[u8]>),
    Staged(ClosedStagedObjectSource),
}

impl ImmutableObjectSource {
    #[must_use]
    pub fn from_boxed_bytes(bytes: Box<[u8]>) -> Self {
        Self::Owned(Arc::from(bytes))
    }

    #[must_use]
    pub fn from_staged(source: ClosedStagedObjectSource) -> Self {
        Self::Staged(source)
    }

    pub fn as_bytes(&self) -> Result<&[u8], ObjectSourceError> {
        SealedImmutableBacking::contiguous_bytes(self)
    }

    #[must_use]
    pub fn exact_byte_length(&self) -> ExactByteLength {
        BoundedReReadableObjectSource::exact_byte_length(self)
    }

    #[must_use]
    pub const fn is_file_backed(&self) -> bool {
        matches!(self, Self::Staged(_))
    }
}

impl SealedImmutableBacking for ImmutableObjectSource {
    fn contiguous_bytes(&self) -> Result<&[u8], ObjectSourceError> {
        match self {
            Self::Owned(bytes) => Ok(bytes),
            Self::Staged(source) => source.as_bytes(),
        }
    }
}

impl BoundedReReadableObjectSource for ImmutableObjectSource {
    fn exact_byte_length(&self) -> ExactByteLength {
        match self {
            Self::Owned(bytes) => ExactByteLength::new(bytes.len() as u64),
            Self::Staged(source) => source.exact_byte_length(),
        }
    }

    fn read_exact_at(&self, offset: u64, destination: &mut [u8]) -> Result<(), ObjectSourceError> {
        match self {
            Self::Owned(bytes) => (&bytes[..]).read_exact_at(offset, destination),
            Self::Staged(source) => source.read_exact_at(offset, destination),
        }
    }
}

struct FileSink<'a> {
    file: BufWriter<&'a mut std::fs::File>,
    position: u64,
}

impl<'a> FileSink<'a> {
    fn new(file: &'a mut std::fs::File) -> Self {
        Self {
            file: BufWriter::with_capacity(64 * 1024, file),
            position: 0,
        }
    }
}

impl ObjectWriteSink for FileSink<'_> {
    type Error = io::Error;

    fn position(&self) -> u64 {
        self.position
    }

    fn write_exact(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        self.file.write_all(bytes)?;
        self.position = self
            .position
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| io::Error::other("staged object position overflow"))?;
        Ok(())
    }

    fn patch_exact_at(&mut self, offset: u64, bytes: &[u8]) -> Result<(), Self::Error> {
        self.file.flush()?;
        self.file.get_mut().seek(SeekFrom::Start(offset))?;
        self.file.get_mut().write_all(bytes)?;
        self.file.get_mut().seek(SeekFrom::Start(self.position))?;
        Ok(())
    }

    fn finish(&mut self) -> Result<(), Self::Error> {
        self.file.flush()?;
        Ok(())
    }
}

fn stage_error(error: BackingError) -> StagedObjectError {
    match error {
        BackingError::Io(error) => StagedObjectError::Io(error),
        BackingError::LengthOverflow => StagedObjectError::ArithmeticOverflow,
        BackingError::BackingChanged => StagedObjectError::BackingChanged,
    }
}

fn map_error(error: BackingError) -> ObjectSourceError {
    match error {
        BackingError::Io(_) => ObjectSourceError::ReadFailed,
        BackingError::LengthOverflow => ObjectSourceError::OutOfBounds,
        BackingError::BackingChanged => ObjectSourceError::BackingChanged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        format, fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    fn create_test_directory(label: &str) -> PathBuf {
        for _ in 0..256 {
            let ordinal = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "laneflow-format-{label}-{}-{ordinal}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return path,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create test directory: {error}"),
            }
        }
        panic!("could not reserve a unique staged-object test directory");
    }

    #[test]
    fn unfinished_writer_drop_removes_its_private_path() {
        let directory = create_test_directory("unfinished-drop");
        let staged = PrivateStagedFile::create_in(&directory).expect("private staged file");
        let writer = StagedObjectWriter {
            staged: Some(staged),
            exact_byte_length: ExactByteLength::new(0),
        };

        drop(writer);

        assert_eq!(fs::read_dir(&directory).expect("test directory").count(), 0);
        fs::remove_dir(directory).expect("remove empty test directory");
    }
}
