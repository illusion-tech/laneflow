//! compiler 后发射检查、LFCP v2 构造与认证 manifest 单提交点。
//!
//! 三份 emitter 最终字节必须先由 `laneflow-format` 重算 digest、length、network revision
//! 并闭合 LFCA/LFSM/LFSD 的必要 binding。发布路径随后只消费该借用型能力，按顺序安装
//! content-addressed no-replace objects、构造/安装 LFCP v2，并恰好一次调用外部认证
//! manifest adapter。任何成功对象在提交点前都只是未引用对象。

mod lfcp;

use std::io;

use laneflow_format::{
    FormatError, FormatLimits, PostEmissionCheckError, check_post_emission_bundle_v1,
};
use laneflow_static_contract::{ExactByteLength, Sha256Digest};

use crate::{
    LocalPortableObjectInstaller, PortableInstallError, PortableObjectCandidate,
    PortableObjectInstallation, PortablePublicationCandidate, portable_emitter::object_key,
};

pub(crate) use self::lfcp::build_lfcp_v2;

/// LFCP v2 的发布者种类。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortablePublisherKindV2 {
    LocalTool,
    Ci,
    ReleaseService,
}

impl PortablePublisherKindV2 {
    const fn code(self) -> u8 {
        match self {
            Self::LocalTool => 0,
            Self::Ci => 1,
            Self::ReleaseService => 2,
        }
    }
}

/// 显式、受控且进入 LFCP v2 exact bytes 的发布 provenance。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortablePublicationProvenanceV2 {
    publisher_kind: PortablePublisherKindV2,
    publisher_build_id: Box<str>,
    controlled_build_provenance: Option<Box<str>>,
    controlled_timestamp: Option<Box<str>>,
}

impl PortablePublicationProvenanceV2 {
    /// 构造完整显式 provenance；本函数不读取环境、时钟或工作目录。
    #[must_use]
    pub fn new(
        publisher_kind: PortablePublisherKindV2,
        publisher_build_id: impl Into<Box<str>>,
        controlled_build_provenance: Option<Box<str>>,
        controlled_timestamp: Option<Box<str>>,
    ) -> Self {
        Self {
            publisher_kind,
            publisher_build_id: publisher_build_id.into(),
            controlled_build_provenance,
            controlled_timestamp,
        }
    }

    #[must_use]
    pub const fn publisher_kind(&self) -> PortablePublisherKindV2 {
        self.publisher_kind
    }

    #[must_use]
    pub fn publisher_build_id(&self) -> &str {
        &self.publisher_build_id
    }

    #[must_use]
    pub fn controlled_build_provenance(&self) -> Option<&str> {
        self.controlled_build_provenance.as_deref()
    }

    #[must_use]
    pub fn controlled_timestamp(&self) -> Option<&str> {
        self.controlled_timestamp.as_deref()
    }
}

/// 外部认证 manifest/pointer adapter 的失败分类。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortableManifestCommitError {
    AtomicCommitUnsupported,
    Rejected,
    Io(io::ErrorKind),
}

/// 交给外部认证 manifest adapter 的唯一提交候选。
#[derive(Clone, Copy, Debug)]
pub struct PortableManifestCommitCandidate<'a> {
    descriptor: &'a PortableObjectCandidate,
    descriptor_installation: &'a PortableObjectInstallation,
    canonical_artifact_installation: &'a PortableObjectInstallation,
    source_map_installation: &'a PortableObjectInstallation,
}

impl<'a> PortableManifestCommitCandidate<'a> {
    #[must_use]
    pub const fn descriptor(self) -> &'a PortableObjectCandidate {
        self.descriptor
    }

    #[must_use]
    pub const fn descriptor_installation(self) -> &'a PortableObjectInstallation {
        self.descriptor_installation
    }

    #[must_use]
    pub const fn canonical_artifact_installation(self) -> &'a PortableObjectInstallation {
        self.canonical_artifact_installation
    }

    #[must_use]
    pub const fn source_map_installation(self) -> &'a PortableObjectInstallation {
        self.source_map_installation
    }
}

/// 认证 manifest/pointer 的外部单提交点。
///
/// 实现必须把 candidate 的 LFCP exact digest/key 与自身外部信任根原子绑定；返回 `Ok(())`
/// 是本事务唯一的 committed 边界。adapter 的真实性、签名格式和 durable pointer 不由
/// compiler 自证。
pub trait PortableManifestCommitter {
    fn commit_authenticated_manifest(
        &mut self,
        candidate: PortableManifestCommitCandidate<'_>,
    ) -> Result<(), PortableManifestCommitError>;
}

/// LFCP v2 发布事务失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortablePublicationError {
    PostEmission(PostEmissionCheckError),
    InstallationBindingMismatch,
    ArithmeticOverflow,
    Format(FormatError),
    Install(PortableInstallError),
    Manifest(PortableManifestCommitError),
}

impl From<PostEmissionCheckError> for PortablePublicationError {
    fn from(value: PostEmissionCheckError) -> Self {
        Self::PostEmission(value)
    }
}

impl From<FormatError> for PortablePublicationError {
    fn from(value: FormatError) -> Self {
        Self::Format(value)
    }
}

impl From<PortableInstallError> for PortablePublicationError {
    fn from(value: PortableInstallError) -> Self {
        Self::Install(value)
    }
}

impl From<PortableManifestCommitError> for PortablePublicationError {
    fn from(value: PortableManifestCommitError) -> Self {
        Self::Manifest(value)
    }
}

/// manifest adapter 已报告单提交成功后的受认证对象绑定。
///
/// 该类型不内置签名或 trust anchor；调用方必须从实际 adapter 保存的外部认证状态判断真实性。
/// 诊断性 LFSD 虽在提交前安装，但不进入 LFCP 或此 capability。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestCommittedPortablePublication {
    descriptor: PortableObjectCandidate,
    descriptor_installation: PortableObjectInstallation,
    canonical_artifact_installation: PortableObjectInstallation,
    source_map_installation: PortableObjectInstallation,
}

impl ManifestCommittedPortablePublication {
    #[must_use]
    pub const fn descriptor(&self) -> &PortableObjectCandidate {
        &self.descriptor
    }

    #[must_use]
    pub const fn descriptor_installation(&self) -> &PortableObjectInstallation {
        &self.descriptor_installation
    }

    #[must_use]
    pub const fn canonical_artifact_installation(&self) -> &PortableObjectInstallation {
        &self.canonical_artifact_installation
    }

    #[must_use]
    pub const fn source_map_installation(&self) -> &PortableObjectInstallation {
        &self.source_map_installation
    }
}

/// 检查并安装 LFCA/LFSM/LFSD，随后构造/安装 LFCP v2，并恰好调用一次 manifest 提交。
///
/// # Errors
///
/// 后发射闭合检查、任一对象安装、LFCP 编码/预检或 manifest 提交失败时，返回错误且不返回
/// 部分成功状态。检查失败保证没有安装或 manifest 副作用；检查后的已安装对象在后续失败时
/// 可以作为未引用对象保留。
pub fn commit_portable_publication_v2<M: PortableManifestCommitter + ?Sized>(
    installer: &LocalPortableObjectInstaller,
    candidate: &PortablePublicationCandidate,
    provenance: &PortablePublicationProvenanceV2,
    limits: FormatLimits,
    manifest: &mut M,
) -> Result<ManifestCommittedPortablePublication, PortablePublicationError> {
    commit_with_installer(installer, candidate, provenance, limits, manifest)
}

trait PublicationObjectInstaller {
    fn install_exact_bytes(
        &self,
        bytes: &[u8],
    ) -> Result<PortableObjectInstallation, PortableInstallError>;
}

impl PublicationObjectInstaller for LocalPortableObjectInstaller {
    fn install_exact_bytes(
        &self,
        bytes: &[u8],
    ) -> Result<PortableObjectInstallation, PortableInstallError> {
        LocalPortableObjectInstaller::install_exact_bytes(self, bytes)
    }
}

fn commit_with_installer<
    I: PublicationObjectInstaller + ?Sized,
    M: PortableManifestCommitter + ?Sized,
>(
    installer: &I,
    candidate: &PortablePublicationCandidate,
    provenance: &PortablePublicationProvenanceV2,
    limits: FormatLimits,
    manifest: &mut M,
) -> Result<ManifestCommittedPortablePublication, PortablePublicationError> {
    let checked = check_post_emission_bundle_v1(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        limits,
    )?;

    let canonical_artifact_installation =
        installer.install_exact_bytes(checked.canonical_artifact_view().bytes())?;
    verify_checked_installation(
        &canonical_artifact_installation,
        checked.canonical_artifact_digest(),
        checked.canonical_artifact_byte_length(),
    )?;

    let source_map_installation =
        installer.install_exact_bytes(checked.source_map_view().bytes())?;
    verify_checked_installation(
        &source_map_installation,
        checked.source_map_digest(),
        checked.source_map_byte_length(),
    )?;

    let semantic_diff_installation =
        installer.install_exact_bytes(checked.semantic_diff_view().bytes())?;
    verify_checked_installation(
        &semantic_diff_installation,
        checked.semantic_diff_digest(),
        checked.semantic_diff_byte_length(),
    )?;

    let descriptor = build_lfcp_v2(checked, provenance, limits)?;
    let descriptor_installation = installer.install_exact_bytes(descriptor.bytes())?;
    verify_candidate_installation(&descriptor_installation, &descriptor)?;

    manifest.commit_authenticated_manifest(PortableManifestCommitCandidate {
        descriptor: &descriptor,
        descriptor_installation: &descriptor_installation,
        canonical_artifact_installation: &canonical_artifact_installation,
        source_map_installation: &source_map_installation,
    })?;

    Ok(ManifestCommittedPortablePublication {
        descriptor,
        descriptor_installation,
        canonical_artifact_installation,
        source_map_installation,
    })
}

fn verify_checked_installation(
    installation: &PortableObjectInstallation,
    digest: Sha256Digest,
    byte_length: ExactByteLength,
) -> Result<(), PortablePublicationError> {
    if installation.digest() != digest
        || installation.byte_length() != byte_length
        || installation.object_key() != object_key(digest).as_ref()
    {
        return Err(PortablePublicationError::InstallationBindingMismatch);
    }
    Ok(())
}

fn verify_candidate_installation(
    installation: &PortableObjectInstallation,
    object: &PortableObjectCandidate,
) -> Result<(), PortablePublicationError> {
    if installation.digest() != object.digest()
        || installation.byte_length() != object.byte_length()
        || installation.object_key() != object.object_key()
    {
        return Err(PortablePublicationError::InstallationBindingMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
