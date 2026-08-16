//! 内容寻址可移植对象的文件系统原子安装。
//!
//! 本模块只安装上层能力已经关闭的 exact bytes。它不会生成 LFCP、验证收据
//! 或认证 manifest，也不会把“对象已安装”包装成“已发布”。最终路径永远由 exact bytes 的
//! SHA-256 派生；最终文件只通过同文件系统 hard-link no-replace 原语一次性出现。

mod platform;

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};

use crate::portable_emitter::{PortableObjectCandidate, object_key, sha256};

use self::platform::{AtomicInstallPlatform, AtomicLinkOutcome, NativeAtomicInstall};

const OBJECT_KEY_PREFIX: &str = "sha256/";
const OBJECT_KEY_HEX_BYTES: usize = 64;
const VERIFY_BUFFER_BYTES: usize = 64 * 1024;
static NEXT_STAGING_ID: AtomicU64 = AtomicU64::new(0);

/// 文件系统安装过程中的稳定操作分类。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortableInstallOperation {
    PrepareStoreRoot,
    PrepareObjectDirectory,
    PrepareStagingDirectory,
    CreateUniqueStagingDirectory,
    CreateStagingFile,
    WriteStagingFile,
    FlushStagingFile,
    CloseStagingFile,
    ReadStagingFile,
    InstallNoReplace,
    ReadWinner,
    SyncObjectDirectory,
    CleanupStaging,
}

/// 内容寻址对象安装失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortableInstallError {
    /// configured root 的固定子目录解析到根外，或不是目录。
    UnsafeStoreLayout,
    /// object key 不是相应摘要唯一的 lowercase canonical spelling。
    NonCanonicalObjectKey,
    /// 提供的 digest 与 exact bytes 不一致。
    ObjectDigestMismatch,
    /// 写后关闭并重读的暂存文件不再等于源 exact bytes。
    StagedObjectMismatch,
    /// 已存在 winner 的精确长度或 bytes 与候选不一致。
    ExistingObjectMismatch,
    /// 当前平台或文件系统不能提供 atomic + no-replace 安装。
    AtomicInstallUnsupported,
    /// 长度转换或唯一命名计数发生溢出。
    ArithmeticOverflow,
    /// 可恢复的底层文件系统错误。
    Io {
        operation: PortableInstallOperation,
        kind: io::ErrorKind,
    },
}

/// 本次对象安装的结果。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortableInstallDisposition {
    /// 本调用成为 atomic no-replace winner。
    Installed,
    /// 目标已由另一完成安装的相同 exact bytes 安装，本调用安全复用。
    Reused,
}

/// 一份已安装不可变对象的计算绑定。
///
/// 该值只证明 content-addressed object winner 的 exact bytes；它不是 #299 独立验证收据，
/// 也不表示 LFCP 或认证 manifest 已提交。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableObjectInstallation {
    digest: [u8; 32],
    byte_length: u64,
    object_key: Box<str>,
    disposition: PortableInstallDisposition,
}

impl PortableObjectInstallation {
    #[must_use]
    pub const fn digest(&self) -> [u8; 32] {
        self.digest
    }

    #[must_use]
    pub const fn byte_length(&self) -> u64 {
        self.byte_length
    }

    #[must_use]
    pub fn object_key(&self) -> &str {
        &self.object_key
    }

    #[must_use]
    pub const fn disposition(&self) -> PortableInstallDisposition {
        self.disposition
    }
}

/// 由调用方独占管理的内容寻址对象根。
#[derive(Clone, Debug)]
pub struct PortableObjectStore {
    root: PathBuf,
    object_directory: PathBuf,
    staging_directory: PathBuf,
}

impl PortableObjectStore {
    /// 建立或打开一份发布根，并验证固定子目录不会解析到根外。
    ///
    /// # Errors
    ///
    /// 目录不能创建/规范化，或 `sha256`/`.staging` 被重定向到根外时失败。
    pub fn try_open(root: impl AsRef<Path>) -> Result<Self, PortableInstallError> {
        let root = prepare_root(root.as_ref())?;
        let object_directory = prepare_child_directory(
            &root,
            "sha256",
            PortableInstallOperation::PrepareObjectDirectory,
        )?;
        let staging_directory = prepare_child_directory(
            &root,
            ".staging",
            PortableInstallOperation::PrepareStagingDirectory,
        )?;
        Ok(Self {
            root,
            object_directory,
            staging_directory,
        })
    }

    /// 已配置发布根的规范绝对路径。
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 把 canonical object key 安全映射到配置根下。
    ///
    /// # Errors
    ///
    /// key 不是 `sha256/<64 lowercase hex>` 时失败。
    pub fn object_path(&self, object_key: &str) -> Result<PathBuf, PortableInstallError> {
        let hex = parse_object_key(object_key)?;
        Ok(self.object_directory.join(hex))
    }

    /// 原子安装一份 emitter 关闭的候选对象。
    ///
    /// # Errors
    ///
    /// 计算绑定、暂存复核、平台安装、winner 比较或持久化失败时返回错误。最终路径绝不
    /// 被覆盖或流式写入。
    pub fn install_candidate(
        &self,
        candidate: &PortableObjectCandidate,
    ) -> Result<PortableObjectInstallation, PortableInstallError> {
        self.install_bound_object(
            candidate.bytes(),
            candidate.digest(),
            candidate.object_key(),
            &NativeAtomicInstall,
        )
    }

    /// 原子安装一份由 crate 内上层能力已经关闭的 exact bytes。
    ///
    /// 内容存储不解释对象格式，也不把任意 bytes 包装成 validated/trusted view；它只从 bytes
    /// 内部计算 digest/key 并提供 immutable winner。receipt/LFCP 等结构证明继续由各自上层
    /// 能力拥有。
    ///
    /// # Errors
    ///
    /// 暂存复核、平台安装、winner 比较或持久化失败时返回错误。
    pub(crate) fn install_exact_bytes(
        &self,
        bytes: &[u8],
    ) -> Result<PortableObjectInstallation, PortableInstallError> {
        let digest = sha256(bytes);
        let key = object_key(digest);
        self.install_bound_object(bytes, digest, &key, &NativeAtomicInstall)
    }

    fn install_bound_object<P: AtomicInstallPlatform>(
        &self,
        bytes: &[u8],
        digest: [u8; 32],
        supplied_key: &str,
        platform: &P,
    ) -> Result<PortableObjectInstallation, PortableInstallError> {
        let canonical_key = object_key(digest);
        if supplied_key != canonical_key.as_ref() || parse_object_key(supplied_key).is_err() {
            return Err(PortableInstallError::NonCanonicalObjectKey);
        }
        if sha256(bytes) != digest {
            return Err(PortableInstallError::ObjectDigestMismatch);
        }

        let byte_length =
            u64::try_from(bytes.len()).map_err(|_| PortableInstallError::ArithmeticOverflow)?;
        let object_path = self.object_path(supplied_key)?;
        let mut staging = StagingGuard::create(&self.staging_directory)?;
        write_closed_staging_file(staging.file_path(), bytes)?;
        verify_file(
            staging.file_path(),
            bytes,
            digest,
            PortableInstallOperation::ReadStagingFile,
            PortableInstallError::StagedObjectMismatch,
        )?;

        let disposition = match platform.link_no_replace(staging.file_path(), &object_path)? {
            AtomicLinkOutcome::Installed => {
                platform.sync_object_directory(&self.object_directory)?;
                verify_file(
                    &object_path,
                    bytes,
                    digest,
                    PortableInstallOperation::ReadWinner,
                    PortableInstallError::ExistingObjectMismatch,
                )?;
                PortableInstallDisposition::Installed
            }
            AtomicLinkOutcome::AlreadyExists => {
                verify_file(
                    &object_path,
                    bytes,
                    digest,
                    PortableInstallOperation::ReadWinner,
                    PortableInstallError::ExistingObjectMismatch,
                )?;
                PortableInstallDisposition::Reused
            }
        };

        staging.cleanup()?;
        Ok(PortableObjectInstallation {
            digest,
            byte_length,
            object_key: canonical_key,
            disposition,
        })
    }
}

fn prepare_root(root: &Path) -> Result<PathBuf, PortableInstallError> {
    fs::create_dir_all(root).map_err(|error| PortableInstallError::Io {
        operation: PortableInstallOperation::PrepareStoreRoot,
        kind: error.kind(),
    })?;
    let canonical = fs::canonicalize(root).map_err(|error| PortableInstallError::Io {
        operation: PortableInstallOperation::PrepareStoreRoot,
        kind: error.kind(),
    })?;
    if !canonical.is_dir() {
        return Err(PortableInstallError::UnsafeStoreLayout);
    }
    Ok(canonical)
}

fn prepare_child_directory(
    root: &Path,
    name: &str,
    operation: PortableInstallOperation,
) -> Result<PathBuf, PortableInstallError> {
    let child = root.join(name);
    fs::create_dir_all(&child).map_err(|error| PortableInstallError::Io {
        operation,
        kind: error.kind(),
    })?;
    let canonical = fs::canonicalize(&child).map_err(|error| PortableInstallError::Io {
        operation,
        kind: error.kind(),
    })?;
    if !canonical.is_dir() || canonical != child || canonical.parent() != Some(root) {
        return Err(PortableInstallError::UnsafeStoreLayout);
    }
    Ok(canonical)
}

fn parse_object_key(object_key: &str) -> Result<&str, PortableInstallError> {
    let Some(hex) = object_key.strip_prefix(OBJECT_KEY_PREFIX) else {
        return Err(PortableInstallError::NonCanonicalObjectKey);
    };
    if hex.len() != OBJECT_KEY_HEX_BYTES
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(PortableInstallError::NonCanonicalObjectKey);
    }
    Ok(hex)
}

fn write_closed_staging_file(path: &Path, bytes: &[u8]) -> Result<(), PortableInstallError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| PortableInstallError::Io {
            operation: PortableInstallOperation::CreateStagingFile,
            kind: error.kind(),
        })?;
    file.write_all(bytes)
        .map_err(|error| PortableInstallError::Io {
            operation: PortableInstallOperation::WriteStagingFile,
            kind: error.kind(),
        })?;
    file.flush().map_err(|error| PortableInstallError::Io {
        operation: PortableInstallOperation::FlushStagingFile,
        kind: error.kind(),
    })?;
    file.sync_all().map_err(|error| PortableInstallError::Io {
        // Safe `File` has no separate fallible close. `sync_all` is the
        // durable close barrier; only after it succeeds is the handle dropped.
        operation: PortableInstallOperation::CloseStagingFile,
        kind: error.kind(),
    })?;
    drop(file);
    Ok(())
}

fn verify_file(
    path: &Path,
    expected: &[u8],
    expected_digest: [u8; 32],
    operation: PortableInstallOperation,
    mismatch: PortableInstallError,
) -> Result<(), PortableInstallError> {
    let expected_length =
        u64::try_from(expected.len()).map_err(|_| PortableInstallError::ArithmeticOverflow)?;
    let metadata = fs::symlink_metadata(path).map_err(|error| PortableInstallError::Io {
        operation,
        kind: error.kind(),
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != expected_length
    {
        return Err(mismatch);
    }

    let mut file = File::open(path).map_err(|error| PortableInstallError::Io {
        operation,
        kind: error.kind(),
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; VERIFY_BUFFER_BYTES];
    let mut offset = 0_usize;
    while offset < expected.len() {
        let wanted = (expected.len() - offset).min(buffer.len());
        let read = read_retry(&mut file, &mut buffer[..wanted]).map_err(|error| {
            PortableInstallError::Io {
                operation,
                kind: error.kind(),
            }
        })?;
        if read == 0 || buffer[..read] != expected[offset..offset + read] {
            return Err(mismatch);
        }
        hasher.update(&buffer[..read]);
        offset = offset
            .checked_add(read)
            .ok_or(PortableInstallError::ArithmeticOverflow)?;
    }
    let mut trailing = [0_u8; 1];
    if read_retry(&mut file, &mut trailing).map_err(|error| PortableInstallError::Io {
        operation,
        kind: error.kind(),
    })? != 0
        || <[u8; 32]>::from(hasher.finalize()) != expected_digest
    {
        return Err(mismatch);
    }
    Ok(())
}

fn read_retry(reader: &mut File, buffer: &mut [u8]) -> io::Result<usize> {
    loop {
        match reader.read(buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            result => return result,
        }
    }
}

struct StagingGuard {
    directory: PathBuf,
    file: PathBuf,
    active: bool,
}

impl StagingGuard {
    fn create(staging_root: &Path) -> Result<Self, PortableInstallError> {
        let process = std::process::id();
        let epoch_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        loop {
            let ordinal = NEXT_STAGING_ID
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                    current.checked_add(1)
                })
                .map_err(|_| PortableInstallError::ArithmeticOverflow)?;
            let directory = staging_root.join(format!(
                "install-{process:08x}-{epoch_nanos:032x}-{ordinal:016x}"
            ));
            match fs::create_dir(&directory) {
                Ok(()) => {
                    let file = directory.join("object.closed");
                    return Ok(Self {
                        directory,
                        file,
                        active: true,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(PortableInstallError::Io {
                        operation: PortableInstallOperation::CreateUniqueStagingDirectory,
                        kind: error.kind(),
                    });
                }
            }
        }
    }

    fn file_path(&self) -> &Path {
        &self.file
    }

    fn cleanup(&mut self) -> Result<(), PortableInstallError> {
        remove_if_present(&self.file, false)?;
        remove_if_present(&self.directory, true)?;
        self.active = false;
        Ok(())
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = fs::remove_file(&self.file);
            let _ = fs::remove_dir(&self.directory);
        }
    }
}

fn remove_if_present(path: &Path, directory: bool) -> Result<(), PortableInstallError> {
    let result = if directory {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    };
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(PortableInstallError::Io {
            operation: PortableInstallOperation::CleanupStaging,
            kind: error.kind(),
        }),
    }
}

#[cfg(test)]
mod tests;
