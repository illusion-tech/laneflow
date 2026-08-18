//! LFCA/LFSM/LFSD 最终字节的无分配后发射闭合检查。

use laneflow_static_contract::{
    CANONICAL_ARTIFACT_FORMAT_VERSION, ExactByteLength, NETWORK_REVISION_DERIVATION_VERSION,
    NETWORK_REVISION_DOMAIN_PREFIX, NetworkRevisionId, PortableObjectKind,
    SECTION_FORMAT_VERSION_V1, Sha256Digest,
};
use sha2::{Digest, Sha256};

use crate::{
    FormatError, FormatLimits, FormatStructure, LimitDimension, RegistryCheckedFieldValue,
    RegistryCheckedRowView, ValueCheckedObjectView, preflight_object_values_v1,
};

/// 调用方从实际 LFSD base 输入保存的预期绑定。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpectedSemanticDiffBaseV1 {
    Genesis,
    Artifact {
        network_revision_derivation_version: u16,
        network_revision: NetworkRevisionId,
        digest: Sha256Digest,
        byte_length: ExactByteLength,
    },
}

/// 后发射检查的稳定失败分类。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PostEmissionCheckError {
    Format(FormatError),
    LimitExceeded {
        dimension: LimitDimension,
        actual: u64,
        limit: u64,
    },
    NetworkRevisionMismatch,
    SourceMapBindingMismatch,
    SemanticDiffBaseBindingMismatch,
    SemanticDiffTargetBindingMismatch,
    ArithmeticOverflow,
}

impl From<FormatError> for PostEmissionCheckError {
    fn from(value: FormatError) -> Self {
        Self::Format(value)
    }
}

#[derive(Clone, Copy, Debug)]
struct CheckedObject<'a> {
    view: ValueCheckedObjectView<'a>,
    digest: Sha256Digest,
    byte_length: ExactByteLength,
}

/// 已从三份最终字节重算并闭合必要 binding 的借用型能力。
///
/// 字段私有且只有 [`check_post_emission_bundle_v1`] 能构造。本类型不证明完整路网语义、
/// 发布真实性或迁移授权。
#[derive(Clone, Copy, Debug)]
pub struct PostEmissionCheckedBundleV1<'a> {
    canonical_artifact: CheckedObject<'a>,
    source_map: CheckedObject<'a>,
    semantic_diff: CheckedObject<'a>,
    network_revision: NetworkRevisionId,
    compiler_build_id: &'a str,
    source_collection_digest_version: u16,
    source_collection_digest: Sha256Digest,
}

impl<'a> PostEmissionCheckedBundleV1<'a> {
    #[must_use]
    pub const fn canonical_artifact_view(self) -> ValueCheckedObjectView<'a> {
        self.canonical_artifact.view
    }

    #[must_use]
    pub const fn canonical_artifact_digest(self) -> Sha256Digest {
        self.canonical_artifact.digest
    }

    #[must_use]
    pub const fn canonical_artifact_byte_length(self) -> ExactByteLength {
        self.canonical_artifact.byte_length
    }

    #[must_use]
    pub const fn source_map_view(self) -> ValueCheckedObjectView<'a> {
        self.source_map.view
    }

    #[must_use]
    pub const fn source_map_digest(self) -> Sha256Digest {
        self.source_map.digest
    }

    #[must_use]
    pub const fn source_map_byte_length(self) -> ExactByteLength {
        self.source_map.byte_length
    }

    #[must_use]
    pub const fn semantic_diff_view(self) -> ValueCheckedObjectView<'a> {
        self.semantic_diff.view
    }

    #[must_use]
    pub const fn semantic_diff_digest(self) -> Sha256Digest {
        self.semantic_diff.digest
    }

    #[must_use]
    pub const fn semantic_diff_byte_length(self) -> ExactByteLength {
        self.semantic_diff.byte_length
    }

    #[must_use]
    pub const fn network_revision(self) -> NetworkRevisionId {
        self.network_revision
    }

    #[must_use]
    pub const fn compiler_build_id(self) -> &'a str {
        self.compiler_build_id
    }

    #[must_use]
    pub const fn source_collection_digest_version(self) -> u16 {
        self.source_collection_digest_version
    }

    #[must_use]
    pub const fn source_collection_digest(self) -> Sha256Digest {
        self.source_collection_digest
    }
}

/// 从 LFCA/LFSM/LFSD 最终字节重算 digest、length、revision 并闭合跨对象 binding。
pub fn check_post_emission_bundle_v1<'a>(
    lfca: &'a [u8],
    lfsm: &'a [u8],
    lfsd: &'a [u8],
    expected_base: ExpectedSemanticDiffBaseV1,
    limits: FormatLimits,
) -> Result<PostEmissionCheckedBundleV1<'a>, PostEmissionCheckError> {
    let lfca_length = checked_object_length(lfca, limits)?;
    let lfsm_length = checked_object_length(lfsm, limits)?;
    let lfsd_length = checked_object_length(lfsd, limits)?;
    let total = lfca_length
        .checked_add(lfsm_length)
        .and_then(|value| value.checked_add(lfsd_length))
        .ok_or(PostEmissionCheckError::ArithmeticOverflow)?;
    let staging_limit = limits.max_candidate_staging_bytes();
    if total > staging_limit {
        return Err(PostEmissionCheckError::LimitExceeded {
            dimension: LimitDimension::CandidateStagingBytes,
            actual: total,
            limit: staging_limit,
        });
    }

    let lfca_view =
        preflight_object_values_v1(lfca, PortableObjectKind::CanonicalArtifact, limits)?;
    let lfsm_view = preflight_object_values_v1(lfsm, PortableObjectKind::SourceMap, limits)?;
    let lfsd_view = preflight_object_values_v1(lfsd, PortableObjectKind::SemanticDiff, limits)?;

    let canonical_artifact = CheckedObject {
        view: lfca_view,
        digest: sha256(lfca),
        byte_length: ExactByteLength::new(lfca_length),
    };
    let source_map = CheckedObject {
        view: lfsm_view,
        digest: sha256(lfsm),
        byte_length: ExactByteLength::new(lfsm_length),
    };
    let semantic_diff = CheckedObject {
        view: lfsd_view,
        digest: sha256(lfsd),
        byte_length: ExactByteLength::new(lfsd_length),
    };

    let network_revision = recompute_network_revision(lfca_view)?;
    if checked_sha256(singleton_row(lfca_view, 7)?.field_by_tag(1))?
        != network_revision.into_digest()
    {
        return Err(PostEmissionCheckError::NetworkRevisionMismatch);
    }

    let lfca_provenance = singleton_row(lfca_view, 6)?;
    let compiler_build_id = checked_utf8(lfca_provenance.field_by_tag(1))?;
    let source_collection_digest_version = checked_u16(lfca_provenance.field_by_tag(2))?;
    let source_collection_digest = checked_sha256(lfca_provenance.field_by_tag(3))?;

    let source_map_binding = singleton_row(lfsm_view, 0)?;
    let source_map_matches = checked_u16(source_map_binding.field_by_tag(1))?
        == NETWORK_REVISION_DERIVATION_VERSION
        && checked_sha256(source_map_binding.field_by_tag(2))? == network_revision.into_digest()
        && checked_u16(source_map_binding.field_by_tag(3))? == CANONICAL_ARTIFACT_FORMAT_VERSION
        && checked_sha256(source_map_binding.field_by_tag(4))? == canonical_artifact.digest
        && checked_u64(source_map_binding.field_by_tag(5))? == canonical_artifact.byte_length.get()
        && checked_utf8(source_map_binding.field_by_tag(6))? == compiler_build_id
        && checked_u16(source_map_binding.field_by_tag(7))? == source_collection_digest_version
        && checked_sha256(source_map_binding.field_by_tag(8))? == source_collection_digest;
    if !source_map_matches {
        return Err(PostEmissionCheckError::SourceMapBindingMismatch);
    }

    let diff_binding = singleton_row(lfsd_view, 0)?;
    let target_matches = checked_u16(diff_binding.field_by_tag(6))?
        == NETWORK_REVISION_DERIVATION_VERSION
        && checked_sha256(diff_binding.field_by_tag(7))? == network_revision.into_digest()
        && checked_sha256(diff_binding.field_by_tag(8))? == canonical_artifact.digest
        && checked_u64(diff_binding.field_by_tag(9))? == canonical_artifact.byte_length.get();
    if !target_matches {
        return Err(PostEmissionCheckError::SemanticDiffTargetBindingMismatch);
    }

    let base_matches = match expected_base {
        ExpectedSemanticDiffBaseV1::Genesis => {
            checked_u8(diff_binding.field_by_tag(1))? == 0
                && checked_u16(diff_binding.field_by_tag(2))? == 0
                && checked_sha256(diff_binding.field_by_tag(3))? == Sha256Digest::ZERO
                && checked_sha256(diff_binding.field_by_tag(4))? == Sha256Digest::ZERO
                && checked_u64(diff_binding.field_by_tag(5))? == 0
        }
        ExpectedSemanticDiffBaseV1::Artifact {
            network_revision_derivation_version,
            network_revision,
            digest,
            byte_length,
        } => {
            checked_u8(diff_binding.field_by_tag(1))? == 1
                && checked_u16(diff_binding.field_by_tag(2))? == network_revision_derivation_version
                && checked_sha256(diff_binding.field_by_tag(3))? == network_revision.into_digest()
                && checked_sha256(diff_binding.field_by_tag(4))? == digest
                && checked_u64(diff_binding.field_by_tag(5))? == byte_length.get()
        }
    };
    if !base_matches {
        return Err(PostEmissionCheckError::SemanticDiffBaseBindingMismatch);
    }

    Ok(PostEmissionCheckedBundleV1 {
        canonical_artifact,
        source_map,
        semantic_diff,
        network_revision,
        compiler_build_id,
        source_collection_digest_version,
        source_collection_digest,
    })
}

fn checked_object_length(
    bytes: &[u8],
    limits: FormatLimits,
) -> Result<u64, PostEmissionCheckError> {
    let actual =
        u64::try_from(bytes.len()).map_err(|_| PostEmissionCheckError::ArithmeticOverflow)?;
    let limit = limits.max_object_bytes();
    if actual > limit {
        return Err(PostEmissionCheckError::LimitExceeded {
            dimension: LimitDimension::ObjectBytes,
            actual,
            limit,
        });
    }
    Ok(actual)
}

fn sha256(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
}

fn recompute_network_revision(
    view: ValueCheckedObjectView<'_>,
) -> Result<NetworkRevisionId, PostEmissionCheckError> {
    let registry = view.registry_view();
    let mut hasher = Sha256::new();
    hasher.update(NETWORK_REVISION_DOMAIN_PREFIX);
    for ordinal in 0..6_u32 {
        let section = registry.section(ordinal).ok_or_else(binding_format_error)?;
        let section_length = u64::try_from(section.bytes().len())
            .map_err(|_| PostEmissionCheckError::ArithmeticOverflow)?;
        hasher.update(section.kind().to_le_bytes());
        hasher.update(SECTION_FORMAT_VERSION_V1.to_le_bytes());
        hasher.update(section_length.to_le_bytes());
        hasher.update(section.bytes());
    }
    Ok(NetworkRevisionId::from_digest(Sha256Digest::from_bytes(
        hasher.finalize().into(),
    )))
}

fn singleton_row<'a>(
    view: ValueCheckedObjectView<'a>,
    section_ordinal: u32,
) -> Result<RegistryCheckedRowView<'a>, PostEmissionCheckError> {
    view.registry_view()
        .section(section_ordinal)
        .and_then(|section| section.table(0))
        .and_then(|table| table.row(0))
        .ok_or_else(binding_format_error)
}

fn checked_u8(
    field: Option<crate::RegistryCheckedFieldView<'_>>,
) -> Result<u8, PostEmissionCheckError> {
    match field.ok_or_else(binding_format_error)?.value()? {
        RegistryCheckedFieldValue::U8(value) => Ok(value),
        _ => Err(binding_format_error()),
    }
}

fn checked_u16(
    field: Option<crate::RegistryCheckedFieldView<'_>>,
) -> Result<u16, PostEmissionCheckError> {
    match field.ok_or_else(binding_format_error)?.value()? {
        RegistryCheckedFieldValue::U16(value) => Ok(value),
        _ => Err(binding_format_error()),
    }
}

fn checked_u64(
    field: Option<crate::RegistryCheckedFieldView<'_>>,
) -> Result<u64, PostEmissionCheckError> {
    match field.ok_or_else(binding_format_error)?.value()? {
        RegistryCheckedFieldValue::U64(value) => Ok(value),
        _ => Err(binding_format_error()),
    }
}

fn checked_sha256(
    field: Option<crate::RegistryCheckedFieldView<'_>>,
) -> Result<Sha256Digest, PostEmissionCheckError> {
    match field.ok_or_else(binding_format_error)?.value()? {
        RegistryCheckedFieldValue::Sha256(value) => Ok(value),
        _ => Err(binding_format_error()),
    }
}

fn checked_utf8<'a>(
    field: Option<crate::RegistryCheckedFieldView<'a>>,
) -> Result<&'a str, PostEmissionCheckError> {
    match field.ok_or_else(binding_format_error)?.value()? {
        RegistryCheckedFieldValue::Utf8(value) => Ok(value),
        _ => Err(binding_format_error()),
    }
}

const fn binding_format_error() -> PostEmissionCheckError {
    PostEmissionCheckError::Format(FormatError::BindingMismatch {
        structure: FormatStructure::FieldValue,
    })
}
