//! 平台私有临时文件 staging 与只读映射 backing。
//!
//! 本 crate 是 workspace 内唯一登记的手写 unsafe 例外。SAFETY 自证链完整封在
//! 本模块内，不依赖调用方守约：
//!
//! 1. [`PrivateStagedFile::create_in`] 经 `tempfile::tempfile_in` 创建 backing：
//!    Unix 上匿名/立即 unlink（无目录项即无路径可达），Windows 上以
//!    `share_mode(0)` + delete-on-close 打开（拒绝一切 reopen）。外部进程
//!    既无路径也无句柄。
//! 2. 写能力仅经 `&mut` 借出（[`PrivateStagedFile::file_mut`]）；
//!    [`PrivateStagedFile::seal`] 消费所有权并核对 exact length，之后本 crate
//!    外不存在任何写路径。
//! 3. [`SealedPrivateFile::map_read_only`] 在映射前再次核对 backing 长度
//!    （纵深防御），映射对象字段私有，只暴露只读字节视图。
//!
//! 本 crate 不实现安装、rename、目录耐久或发布事务；只承载 staging 生命周期
//! 与只读映射边界。

#![allow(unsafe_code)]

use std::{fs::File, io, ops::Deref, path::Path};

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
/// 只能通过 [`Self::file_mut`] 获得 `&mut File` 写能力；[`Self::seal`] 消费本值
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

    /// 借出独占写能力。
    pub fn file_mut(&mut self) -> &mut File {
        &mut self.file
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
