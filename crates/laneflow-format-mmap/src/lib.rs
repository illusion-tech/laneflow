//! 平台私有临时文件 staging 与只读映射 backing。
//!
//! 本 crate 是 workspace 内唯一登记的手写 unsafe 例外。SAFETY 自证链完整封在
//! 本模块内，不依赖调用方守约：
//!
//! 1. [`PrivateStagedFile::create_in`] 经 `tempfile::tempfile_in` 创建 backing：
//!    Unix 上匿名/立即 unlink（无目录项即无路径可达），Windows 上以
//!    `share_mode(0)` + delete-on-close 打开（拒绝一切 reopen）。外部进程
//!    既无路径也无句柄。
//! 2. 写能力仅经受限接口暴露：[`io::Write`] 实现（顺序写/flush）与
//!    [`PrivateStagedFile::patch_exact_at`]（定点覆写后恢复写位置）；`File`
//!    句柄本身永不外借，调用方无法 `try_clone` 出 seal 后仍存活的写副本。
//!    [`PrivateStagedFile::seal`] 消费所有权并核对 exact length，之后本 crate
//!    外不存在任何写路径。
//! 3. [`SealedPrivateFile::map_read_only`] 在映射前再次核对 backing 长度
//!    （纵深防御），映射对象字段私有，只暴露只读字节视图。
//!
//! 本 crate 不实现安装、rename、目录耐久或发布事务；只承载 staging 生命周期
//! 与只读映射边界。

#![allow(unsafe_code)]

use std::{
    fs::File,
    io::{self, Seek, SeekFrom, Write},
    ops::Deref,
    path::Path,
};

use memmap2::{Mmap, MmapOptions};

/// staging 生命周期与只读映射的失败。
#[derive(Debug)]
pub enum BackingError {
    Io(io::Error),
    /// u64 exact length 无法装入本平台 usize。
    LengthOverflow,
    /// 核对时 backing 长度与登记长度不符。
    BackingChanged,
}

impl From<io::Error> for BackingError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl std::fmt::Display for BackingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "staged backing I/O: {error}"),
            Self::LengthOverflow => write!(formatter, "staged backing length overflows usize"),
            Self::BackingChanged => write!(formatter, "staged backing length changed"),
        }
    }
}

impl std::error::Error for BackingError {}

/// 写窗口开放中的平台私有临时 backing。
///
/// 写能力仅经 [`io::Write`] 实现与 [`Self::patch_exact_at`] 暴露，`File` 句柄
/// 不可达（无法 `try_clone` 出 seal 后仍存活的写副本）；[`Self::seal`] 消费本值
/// 后写能力在本 crate 外不再存在。
#[derive(Debug)]
pub struct PrivateStagedFile {
    file: File,
}

impl PrivateStagedFile {
    /// 在调用方选择的目录中创建平台私有临时 backing（Unix 匿名/unlink，
    /// Windows `share_mode(0)` + delete-on-close）。
    pub fn create_in(directory: &Path) -> Result<Self, BackingError> {
        Ok(Self {
            file: tempfile::tempfile_in(directory)?,
        })
    }

    /// 在 `offset` 处定点覆写已 staged 的字节，随后把内核写位置恢复到
    /// `resume`（调用方跟踪的顺序写末尾）。覆写直达句柄，不经用户态缓冲。
    pub fn patch_exact_at(&mut self, offset: u64, bytes: &[u8], resume: u64) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(bytes)?;
        self.file.seek(SeekFrom::Start(resume))?;
        Ok(())
    }

    /// 核对 backing 当前长度与登记的 exact length 一致后消费本值，关闭写窗口。
    pub fn seal(self, exact_byte_length: u64) -> Result<SealedPrivateFile, BackingError> {
        if self.file.metadata()?.len() != exact_byte_length {
            return Err(BackingError::BackingChanged);
        }
        Ok(SealedPrivateFile {
            file: self.file,
            exact_byte_length,
        })
    }
}

/// 顺序写能力经标准 [`io::Write`] trait 暴露：`File` 句柄本身不可达，
/// 调用方无法 `try_clone` 出 seal 后仍存活的写副本，也无法 seek 改变写位置
/// （定点覆写只能走 [`PrivateStagedFile::patch_exact_at`]）。
impl io::Write for PrivateStagedFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.file.write_all(buf)
    }
}

/// 写窗口已关闭的平台私有 backing；只保留只读映射能力。
#[derive(Debug)]
pub struct SealedPrivateFile {
    file: File,
    exact_byte_length: u64,
}

impl SealedPrivateFile {
    /// 构造时登记的 exact length。
    #[must_use]
    pub fn exact_byte_length(&self) -> u64 {
        self.exact_byte_length
    }

    /// 建立只读映射。可重复调用；每次调用前重新核对 backing 长度。
    pub fn map_read_only(&self) -> Result<ReadOnlyMap, BackingError> {
        if self.file.metadata()?.len() != self.exact_byte_length {
            return Err(BackingError::BackingChanged);
        }
        let expected =
            usize::try_from(self.exact_byte_length).map_err(|_| BackingError::LengthOverflow)?;
        // SAFETY: 见模块文档的自证链——backing 由 tempfile_in 创建（平台级私有），
        // 写窗口经 seal 消费关闭，本次调用前刚完成长度核对；映射只读且字段私有。
        let map = unsafe { MmapOptions::new().len(expected).map(&self.file) }?;
        if map.len() != expected {
            return Err(BackingError::BackingChanged);
        }
        Ok(ReadOnlyMap { map })
    }
}

/// 只读映射的持有型字节视图。
#[derive(Debug)]
pub struct ReadOnlyMap {
    map: Mmap,
}

impl Deref for ReadOnlyMap {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.map[..]
    }
}

#[cfg(test)]
mod tests;
