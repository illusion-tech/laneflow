//! LFCP v1 构造与认证 manifest 单提交点。
//!
//! #298 不定义或解析 #299 的验证收据 wire。未来独立验证器通过
//! [`CanonicalPublicationReceiptViewV1`] 暴露它已经验证的 exact bytes、metadata 与精确两个
//! subject bindings；本层只比较这些绑定、按顺序安装 immutable objects、构造/安装 LFCP，
//! 并恰好一次调用外部认证 manifest adapter。任何成功对象在提交点前都只是未引用对象。

mod lfcp;

use std::io;

use laneflow_format::{FormatError, FormatLimits};
use laneflow_static_contract::{
    CANONICAL_ARTIFACT_FORMAT_VERSION, ExactByteLength, NETWORK_REVISION_DERIVATION_VERSION,
    NetworkRevisionId, SOURCE_MAP_FORMAT_VERSION, Sha256Digest,
};

use crate::{
    PortableInstallError, PortableObjectCandidate, PortableObjectInstallation, PortableObjectStore,
    PortablePublicationCandidate,
    portable_emitter::{object_key, sha256},
};

use self::lfcp::build_lfcp_v1;

const VALIDATION_RECEIPT_FORMAT_VERSION_V1: u16 = 1;
const CANONICAL_PUBLICATION_RECEIPT_KIND_V1: &str = "canonical-publication-v1";

struct CheckedReceiptBindingV1<'a> {
    bytes: &'a [u8],
    format_version: u16,
    kind: &'a str,
    validator_build_id: &'a str,
}

/// #299 receipt 中唯一的 canonical artifact subject binding 投影。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortableArtifactSubjectBindingV1 {
    pub canonical_artifact_format_version: u16,
    pub network_revision_derivation_version: u16,
    pub network_revision: NetworkRevisionId,
    pub digest: Sha256Digest,
    pub byte_length: ExactByteLength,
}

/// #299 receipt 中唯一的 source-map subject binding 投影。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortableSourceMapSubjectBindingV1 {
    pub source_map_format_version: u16,
    pub digest: Sha256Digest,
    pub byte_length: ExactByteLength,
}

/// #299 独立验证视图必须提供给 #298 发布事务的最小接口。
///
/// 实现者负责从 receipt exact bytes 独立解析并证明：只存在一个 artifact subject 和一个
/// source-map subject、没有 diff/image/base/target/额外 subject，且 metadata 与 bytes 相符。
/// #298 不把本 trait 的任意实现自动升级为 trusted；最终真实性仍来自 manifest adapter 的
/// 外部信任根。
pub trait CanonicalPublicationReceiptViewV1 {
    fn exact_bytes(&self) -> &[u8];
    fn validation_receipt_format_version(&self) -> u16;
    fn receipt_kind(&self) -> &str;
    fn validator_build_id(&self) -> &str;
    fn subject_count(&self) -> u32;
    fn canonical_artifact_subject(&self) -> Option<PortableArtifactSubjectBindingV1>;
    fn source_map_subject(&self) -> Option<PortableSourceMapSubjectBindingV1>;
}

/// LFCP v1 的发布者种类。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortablePublisherKindV1 {
    LocalTool,
    Ci,
    ReleaseService,
}

impl PortablePublisherKindV1 {
    const fn code(self) -> u8 {
        match self {
            Self::LocalTool => 0,
            Self::Ci => 1,
            Self::ReleaseService => 2,
        }
    }
}

/// 显式、受控且进入 LFCP exact bytes 的发布 provenance。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortablePublicationProvenanceV1 {
    publisher_kind: PortablePublisherKindV1,
    publisher_build_id: Box<str>,
    controlled_build_provenance: Option<Box<str>>,
    controlled_timestamp: Option<Box<str>>,
}

impl PortablePublicationProvenanceV1 {
    /// 构造完整显式 provenance；本函数不读取环境、时钟或工作目录。
    #[must_use]
    pub fn new(
        publisher_kind: PortablePublisherKindV1,
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
    pub const fn publisher_kind(&self) -> PortablePublisherKindV1 {
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
    receipt_installation: &'a PortableObjectInstallation,
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

    #[must_use]
    pub const fn receipt_installation(self) -> &'a PortableObjectInstallation {
        self.receipt_installation
    }
}

/// 认证 manifest/pointer 的外部单提交点。
///
/// 实现必须把 candidate 的 LFCP exact digest/key 与自身外部信任根原子绑定；返回 `Ok(())`
/// 是本事务唯一的 committed 边界。adapter 的真实性、签名格式和 durable pointer 不由 #298
/// 自证。
pub trait PortableManifestCommitter {
    fn commit_authenticated_manifest(
        &mut self,
        candidate: PortableManifestCommitCandidate<'_>,
    ) -> Result<(), PortableManifestCommitError>;
}

/// LFCP 发布事务失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortablePublicationError {
    InvalidReceiptFormatVersion,
    InvalidReceiptKind,
    EmptyReceipt,
    ReceiptLimitExceeded { actual: u64, limit: u64 },
    ReceiptSubjectShapeMismatch,
    ReceiptSubjectBindingMismatch,
    InstallationBindingMismatch,
    ArithmeticOverflow,
    Format(FormatError),
    Install(PortableInstallError),
    Manifest(PortableManifestCommitError),
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
/// 诊断性 LFSD 虽在提交前安装，但不进入 LFCP 或此 capability；#299 独立验证后，才由
/// #302 的可信切换描述符绑定。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestCommittedPortablePublication {
    descriptor: PortableObjectCandidate,
    descriptor_installation: PortableObjectInstallation,
    canonical_artifact_installation: PortableObjectInstallation,
    source_map_installation: PortableObjectInstallation,
    receipt_installation: PortableObjectInstallation,
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

    #[must_use]
    pub const fn receipt_installation(&self) -> &PortableObjectInstallation {
        &self.receipt_installation
    }
}

/// 安装 LFCA/LFSM/LFSD 与 #299 receipt，随后构造/安装 LFCP，并恰好调用一次 manifest 提交。
///
/// # Errors
///
/// receipt metadata/subject 不匹配、任一对象安装、LFCP 编码/预检或 manifest 提交失败时，
/// 返回错误且不返回部分成功状态。已安装 immutable objects 可以作为未引用对象保留。
pub fn commit_portable_publication_v1<
    R: CanonicalPublicationReceiptViewV1 + ?Sized,
    M: PortableManifestCommitter + ?Sized,
>(
    store: &PortableObjectStore,
    candidate: &PortablePublicationCandidate,
    receipt: &R,
    provenance: &PortablePublicationProvenanceV1,
    limits: FormatLimits,
    manifest: &mut M,
) -> Result<ManifestCommittedPortablePublication, PortablePublicationError> {
    commit_with_installer(store, candidate, receipt, provenance, limits, manifest)
}

trait PublicationObjectInstaller {
    fn install_candidate(
        &self,
        candidate: &PortableObjectCandidate,
    ) -> Result<PortableObjectInstallation, PortableInstallError>;

    fn install_exact_bytes(
        &self,
        bytes: &[u8],
    ) -> Result<PortableObjectInstallation, PortableInstallError>;
}

impl PublicationObjectInstaller for PortableObjectStore {
    fn install_candidate(
        &self,
        candidate: &PortableObjectCandidate,
    ) -> Result<PortableObjectInstallation, PortableInstallError> {
        PortableObjectStore::install_candidate(self, candidate)
    }

    fn install_exact_bytes(
        &self,
        bytes: &[u8],
    ) -> Result<PortableObjectInstallation, PortableInstallError> {
        PortableObjectStore::install_exact_bytes(self, bytes)
    }
}

fn commit_with_installer<
    I: PublicationObjectInstaller + ?Sized,
    R: CanonicalPublicationReceiptViewV1 + ?Sized,
    M: PortableManifestCommitter + ?Sized,
>(
    installer: &I,
    candidate: &PortablePublicationCandidate,
    receipt: &R,
    provenance: &PortablePublicationProvenanceV1,
    limits: FormatLimits,
    manifest: &mut M,
) -> Result<ManifestCommittedPortablePublication, PortablePublicationError> {
    let receipt = validate_receipt(candidate, receipt, limits)?;

    let canonical_artifact_installation =
        installer.install_candidate(candidate.canonical_artifact())?;
    verify_installation(
        &canonical_artifact_installation,
        candidate.canonical_artifact(),
    )?;
    let source_map_installation = installer.install_candidate(candidate.source_map())?;
    verify_installation(&source_map_installation, candidate.source_map())?;
    let semantic_diff_installation = installer.install_candidate(candidate.semantic_diff())?;
    verify_installation(&semantic_diff_installation, candidate.semantic_diff())?;

    let receipt_installation = installer.install_exact_bytes(receipt.bytes)?;
    verify_exact_installation(&receipt_installation, receipt.bytes)?;

    let descriptor = build_lfcp_v1(
        candidate,
        &receipt,
        &receipt_installation,
        provenance,
        limits,
    )?;
    let descriptor_installation = installer.install_candidate(&descriptor)?;
    verify_installation(&descriptor_installation, &descriptor)?;

    manifest.commit_authenticated_manifest(PortableManifestCommitCandidate {
        descriptor: &descriptor,
        descriptor_installation: &descriptor_installation,
        canonical_artifact_installation: &canonical_artifact_installation,
        source_map_installation: &source_map_installation,
        receipt_installation: &receipt_installation,
    })?;

    Ok(ManifestCommittedPortablePublication {
        descriptor,
        descriptor_installation,
        canonical_artifact_installation,
        source_map_installation,
        receipt_installation,
    })
}

fn validate_receipt<'a, R: CanonicalPublicationReceiptViewV1 + ?Sized>(
    candidate: &PortablePublicationCandidate,
    receipt: &'a R,
    limits: FormatLimits,
) -> Result<CheckedReceiptBindingV1<'a>, PortablePublicationError> {
    let bytes = receipt.exact_bytes();
    let format_version = receipt.validation_receipt_format_version();
    let kind = receipt.receipt_kind();
    let validator_build_id = receipt.validator_build_id();
    let subject_count = receipt.subject_count();
    let artifact = receipt.canonical_artifact_subject();
    let source_map = receipt.source_map_subject();

    if format_version != VALIDATION_RECEIPT_FORMAT_VERSION_V1 {
        return Err(PortablePublicationError::InvalidReceiptFormatVersion);
    }
    if kind != CANONICAL_PUBLICATION_RECEIPT_KIND_V1 {
        return Err(PortablePublicationError::InvalidReceiptKind);
    }
    let receipt_length =
        u64::try_from(bytes.len()).map_err(|_| PortablePublicationError::ArithmeticOverflow)?;
    if receipt_length == 0 {
        return Err(PortablePublicationError::EmptyReceipt);
    }
    let limit = limits.max_object_bytes();
    if receipt_length > limit {
        return Err(PortablePublicationError::ReceiptLimitExceeded {
            actual: receipt_length,
            limit,
        });
    }
    if subject_count != 2 {
        return Err(PortablePublicationError::ReceiptSubjectShapeMismatch);
    }
    let artifact = artifact.ok_or(PortablePublicationError::ReceiptSubjectShapeMismatch)?;
    let source_map = source_map.ok_or(PortablePublicationError::ReceiptSubjectShapeMismatch)?;
    let expected_artifact = PortableArtifactSubjectBindingV1 {
        canonical_artifact_format_version: CANONICAL_ARTIFACT_FORMAT_VERSION,
        network_revision_derivation_version: NETWORK_REVISION_DERIVATION_VERSION,
        network_revision: candidate.network_revision(),
        digest: candidate.canonical_artifact().digest(),
        byte_length: candidate.canonical_artifact().byte_length(),
    };
    let expected_source_map = PortableSourceMapSubjectBindingV1 {
        source_map_format_version: SOURCE_MAP_FORMAT_VERSION,
        digest: candidate.source_map().digest(),
        byte_length: candidate.source_map().byte_length(),
    };
    if artifact != expected_artifact || source_map != expected_source_map {
        return Err(PortablePublicationError::ReceiptSubjectBindingMismatch);
    }
    Ok(CheckedReceiptBindingV1 {
        bytes,
        format_version,
        kind,
        validator_build_id,
    })
}

fn verify_installation(
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

fn verify_exact_installation(
    installation: &PortableObjectInstallation,
    bytes: &[u8],
) -> Result<(), PortablePublicationError> {
    let digest = sha256(bytes);
    let length = ExactByteLength::new(
        u64::try_from(bytes.len()).map_err(|_| PortablePublicationError::ArithmeticOverflow)?,
    );
    if installation.digest() != digest
        || installation.byte_length() != length
        || installation.object_key() != object_key(digest).as_ref()
    {
        return Err(PortablePublicationError::InstallationBindingMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
