//! LFCA/LFSM/LFSD 最终字节的无分配后发射闭合检查。

use laneflow_static_contract::{
    CANONICAL_ARTIFACT_FORMAT_VERSION, ExactByteLength, NETWORK_REVISION_DERIVATION_VERSION,
    NetworkRevisionId, PortableObjectKind, Sha256Digest,
};
use sha2::{Digest, Sha256};

use crate::{
    BoundedReReadableObjectSource, CanonicalNetworkInputError, CheckedCanonicalNetworkInput,
    FormatError, FormatLimits, FormatStructure, LimitDimension, ObjectSourceError,
    RegistryCheckedFieldValue, RegistryCheckedRowView, ValueCheckedObjectView,
    canonical_network::{CanonicalNetworkInputProof, check_canonical_network_input_binding},
    object_source::contiguous_bytes,
    preflight_object_values,
    value::ValueCheckProof,
};

/// 调用方从实际 LFSD base 输入保存的预期绑定。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpectedSemanticDiffBase {
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
    ObjectSource {
        object: PortableObjectKind,
        error: ObjectSourceError,
    },
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

impl From<CanonicalNetworkInputError> for PostEmissionCheckError {
    fn from(value: CanonicalNetworkInputError) -> Self {
        match value {
            CanonicalNetworkInputError::ObjectSource(error) => Self::ObjectSource {
                object: PortableObjectKind::CanonicalArtifact,
                error,
            },
            CanonicalNetworkInputError::Format(error) => Self::Format(error),
            CanonicalNetworkInputError::LimitExceeded {
                dimension,
                actual,
                limit,
            } => Self::LimitExceeded {
                dimension,
                actual,
                limit,
            },
            CanonicalNetworkInputError::NetworkRevisionMismatch => Self::NetworkRevisionMismatch,
            CanonicalNetworkInputError::ArithmeticOverflow => Self::ArithmeticOverflow,
        }
    }
}

#[derive(Clone, Debug)]
struct CheckedObject<S> {
    source: S,
    proof: ValueCheckProof,
    digest: Sha256Digest,
    byte_length: ExactByteLength,
}

/// 已从三份 LFCA/LFSM/LFSD 不可变来源重算并闭合必要 binding 的拥有型能力。
#[derive(Clone, Debug)]
pub struct PostEmissionCheckedBundle<L, M, D> {
    canonical_artifact: CheckedObject<L>,
    source_map: CheckedObject<M>,
    semantic_diff: CheckedObject<D>,
    canonical_network_proof: CanonicalNetworkInputProof,
    source_collection_digest_version: u16,
    source_collection_digest: Sha256Digest,
}

impl<L, M, D> PostEmissionCheckedBundle<L, M, D>
where
    L: BoundedReReadableObjectSource,
    M: BoundedReReadableObjectSource,
    D: BoundedReReadableObjectSource,
{
    #[must_use]
    pub fn canonical_artifact_view(&self) -> ValueCheckedObjectView<'_> {
        checked_view(&self.canonical_artifact)
    }

    #[must_use]
    pub const fn canonical_artifact_digest(&self) -> Sha256Digest {
        self.canonical_artifact.digest
    }

    #[must_use]
    pub const fn canonical_artifact_byte_length(&self) -> ExactByteLength {
        self.canonical_artifact.byte_length
    }

    /// 消费 bundle，只保留 LFCA 来源与已完成的检查证明。
    #[must_use]
    pub fn canonical_network_input(self) -> CheckedCanonicalNetworkInput<L> {
        let CheckedObject {
            source,
            proof,
            digest,
            byte_length,
        } = self.canonical_artifact;
        CheckedCanonicalNetworkInput::from_parts(
            source,
            proof,
            digest,
            byte_length,
            self.canonical_network_proof,
        )
    }

    #[must_use]
    pub fn source_map_view(&self) -> ValueCheckedObjectView<'_> {
        checked_view(&self.source_map)
    }

    #[must_use]
    pub const fn source_map_digest(&self) -> Sha256Digest {
        self.source_map.digest
    }

    #[must_use]
    pub const fn source_map_byte_length(&self) -> ExactByteLength {
        self.source_map.byte_length
    }

    #[must_use]
    pub fn semantic_diff_view(&self) -> ValueCheckedObjectView<'_> {
        checked_view(&self.semantic_diff)
    }

    #[must_use]
    pub const fn semantic_diff_digest(&self) -> Sha256Digest {
        self.semantic_diff.digest
    }

    #[must_use]
    pub const fn semantic_diff_byte_length(&self) -> ExactByteLength {
        self.semantic_diff.byte_length
    }

    #[must_use]
    pub const fn network_revision(&self) -> NetworkRevisionId {
        self.canonical_network_proof.network_revision()
    }

    #[must_use]
    pub fn compiler_build_id(&self) -> &str {
        let provenance = singleton_row(self.canonical_artifact_view(), 6)
            .expect("checked immutable LFCA must retain its provenance row");
        checked_utf8(provenance.field_by_tag(1))
            .expect("checked immutable LFCA must retain compiler build ID")
    }

    #[must_use]
    pub const fn source_collection_digest_version(&self) -> u16 {
        self.source_collection_digest_version
    }

    #[must_use]
    pub const fn source_collection_digest(&self) -> Sha256Digest {
        self.source_collection_digest
    }
}

/// 对 LFCA/LFSM/LFSD 最终不可变来源做后发射闭合检查。
pub fn check_post_emission_bundle<L, M, D>(
    lfca: L,
    lfsm: M,
    lfsd: D,
    expected_base: ExpectedSemanticDiffBase,
    limits: FormatLimits,
) -> Result<PostEmissionCheckedBundle<L, M, D>, PostEmissionCheckError>
where
    L: BoundedReReadableObjectSource,
    M: BoundedReReadableObjectSource,
    D: BoundedReReadableObjectSource,
{
    check_post_emission_bundle_at(lfca, lfsm, lfsd, expected_base, limits)
}

fn check_post_emission_bundle_at<L, M, D>(
    lfca: L,
    lfsm: M,
    lfsd: D,
    expected_base: ExpectedSemanticDiffBase,
    limits: FormatLimits,
) -> Result<PostEmissionCheckedBundle<L, M, D>, PostEmissionCheckError>
where
    L: BoundedReReadableObjectSource,
    M: BoundedReReadableObjectSource,
    D: BoundedReReadableObjectSource,
{
    let lfca_length = checked_object_length(&lfca, PortableObjectKind::CanonicalArtifact, limits)?;
    let lfsm_length = checked_object_length(&lfsm, PortableObjectKind::SourceMap, limits)?;
    let lfsd_length = checked_object_length(&lfsd, PortableObjectKind::SemanticDiff, limits)?;

    let (
        canonical_artifact_digest,
        source_map_digest,
        semantic_diff_digest,
        canonical_network_proof,
        lfca_proof,
        lfsm_proof,
        lfsd_proof,
        source_collection_digest_version,
        source_collection_digest,
    ) = {
        let lfca_bytes = source_bytes(&lfca, PortableObjectKind::CanonicalArtifact)?;
        let lfsm_bytes = source_bytes(&lfsm, PortableObjectKind::SourceMap)?;
        let lfsd_bytes = source_bytes(&lfsd, PortableObjectKind::SemanticDiff)?;
        let lfca_view =
            preflight_object_values(lfca_bytes, PortableObjectKind::CanonicalArtifact, limits)?;
        let lfsm_view = preflight_object_values(lfsm_bytes, PortableObjectKind::SourceMap, limits)?;
        let lfsd_view =
            preflight_object_values(lfsd_bytes, PortableObjectKind::SemanticDiff, limits)?;

        let canonical_artifact_digest = sha256(lfca_bytes);
        let source_map_digest = sha256(lfsm_bytes);
        let semantic_diff_digest = sha256(lfsd_bytes);
        let canonical_network_proof = check_canonical_network_input_binding(lfca_view)?;
        let network_revision = canonical_network_proof.network_revision();

        let lfca_provenance = singleton_row(lfca_view, 6)?;
        let compiler_build_id = checked_utf8(lfca_provenance.field_by_tag(1))?;
        let source_collection_digest_version = checked_u16(lfca_provenance.field_by_tag(2))?;
        let source_collection_digest = checked_sha256(lfca_provenance.field_by_tag(3))?;

        let source_map_binding = singleton_row(lfsm_view, 0)?;
        let source_map_matches = checked_u16(source_map_binding.field_by_tag(1))?
            == NETWORK_REVISION_DERIVATION_VERSION
            && checked_sha256(source_map_binding.field_by_tag(2))?
                == network_revision.into_digest()
            && checked_u16(source_map_binding.field_by_tag(3))?
                == CANONICAL_ARTIFACT_FORMAT_VERSION
            && checked_sha256(source_map_binding.field_by_tag(4))? == canonical_artifact_digest
            && checked_u64(source_map_binding.field_by_tag(5))? == lfca_length
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
            && checked_sha256(diff_binding.field_by_tag(8))? == canonical_artifact_digest
            && checked_u64(diff_binding.field_by_tag(9))? == lfca_length;
        if !target_matches {
            return Err(PostEmissionCheckError::SemanticDiffTargetBindingMismatch);
        }

        let base_matches = match expected_base {
            ExpectedSemanticDiffBase::Genesis => {
                checked_u8(diff_binding.field_by_tag(1))? == 0
                    && checked_u16(diff_binding.field_by_tag(2))? == 0
                    && checked_sha256(diff_binding.field_by_tag(3))? == Sha256Digest::ZERO
                    && checked_sha256(diff_binding.field_by_tag(4))? == Sha256Digest::ZERO
                    && checked_u64(diff_binding.field_by_tag(5))? == 0
            }
            ExpectedSemanticDiffBase::Artifact {
                network_revision_derivation_version,
                network_revision,
                digest,
                byte_length,
            } => {
                checked_u8(diff_binding.field_by_tag(1))? == 1
                    && checked_u16(diff_binding.field_by_tag(2))?
                        == network_revision_derivation_version
                    && checked_sha256(diff_binding.field_by_tag(3))?
                        == network_revision.into_digest()
                    && checked_sha256(diff_binding.field_by_tag(4))? == digest
                    && checked_u64(diff_binding.field_by_tag(5))? == byte_length.get()
            }
        };
        if !base_matches {
            return Err(PostEmissionCheckError::SemanticDiffBaseBindingMismatch);
        }

        (
            canonical_artifact_digest,
            source_map_digest,
            semantic_diff_digest,
            canonical_network_proof,
            lfca_view.proof(),
            lfsm_view.proof(),
            lfsd_view.proof(),
            source_collection_digest_version,
            source_collection_digest,
        )
    };

    Ok(PostEmissionCheckedBundle {
        canonical_artifact: CheckedObject {
            source: lfca,
            proof: lfca_proof,
            digest: canonical_artifact_digest,
            byte_length: ExactByteLength::new(lfca_length),
        },
        source_map: CheckedObject {
            source: lfsm,
            proof: lfsm_proof,
            digest: source_map_digest,
            byte_length: ExactByteLength::new(lfsm_length),
        },
        semantic_diff: CheckedObject {
            source: lfsd,
            proof: lfsd_proof,
            digest: semantic_diff_digest,
            byte_length: ExactByteLength::new(lfsd_length),
        },
        canonical_network_proof,
        source_collection_digest_version,
        source_collection_digest,
    })
}

fn checked_object_length<S>(
    source: &S,
    object: PortableObjectKind,
    limits: FormatLimits,
) -> Result<u64, PostEmissionCheckError>
where
    S: BoundedReReadableObjectSource,
{
    let actual = source.exact_byte_length().get();
    let limit = limits.max_object_bytes();
    if actual > limit {
        return Err(PostEmissionCheckError::LimitExceeded {
            dimension: LimitDimension::ObjectBytes,
            actual,
            limit,
        });
    }
    let bytes = source_bytes(source, object)?;
    if usize::try_from(actual).ok() != Some(bytes.len()) {
        return Err(PostEmissionCheckError::ObjectSource {
            object,
            error: ObjectSourceError::BackingChanged,
        });
    }
    Ok(actual)
}

fn source_bytes<S>(source: &S, object: PortableObjectKind) -> Result<&[u8], PostEmissionCheckError>
where
    S: BoundedReReadableObjectSource + ?Sized,
{
    contiguous_bytes(source).map_err(|error| PostEmissionCheckError::ObjectSource { object, error })
}

fn checked_view<S>(object: &CheckedObject<S>) -> ValueCheckedObjectView<'_>
where
    S: BoundedReReadableObjectSource,
{
    let bytes = contiguous_bytes(&object.source).expect("checked immutable source cannot drift");
    object
        .proof
        .reborrow(bytes)
        .expect("checked immutable source length cannot drift")
}

fn sha256(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FormatLimitConfig, object_source::private::SealedImmutableBacking};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    const FULL_LFCA: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
    );
    const FULL_LFSM: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfsm"
    );
    const FULL_LFSD: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfsd"
    );

    #[derive(Debug)]
    struct MustNotMap {
        length: ExactByteLength,
    }

    impl SealedImmutableBacking for MustNotMap {
        fn contiguous_bytes(&self) -> Result<&[u8], ObjectSourceError> {
            panic!("object limit must be checked before mapping the backing")
        }
    }

    impl BoundedReReadableObjectSource for MustNotMap {
        fn exact_byte_length(&self) -> ExactByteLength {
            self.length
        }

        fn read_exact_at(
            &self,
            _offset: u64,
            _destination: &mut [u8],
        ) -> Result<(), ObjectSourceError> {
            panic!("object limit must be checked before reading the backing")
        }
    }

    #[test]
    fn known_length_limit_precedes_backing_map_hash_and_parse() {
        let mut config = FormatLimitConfig::HARD;
        config.max_object_bytes = 1;
        let error = check_post_emission_bundle(
            MustNotMap {
                length: ExactByteLength::new(2),
            },
            &[][..],
            &[][..],
            ExpectedSemanticDiffBase::Genesis,
            FormatLimits::try_new(config).unwrap(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            PostEmissionCheckError::LimitExceeded {
                dimension: LimitDimension::ObjectBytes,
                actual: 2,
                limit: 1,
            }
        );
    }

    #[derive(Debug)]
    struct InconsistentLength {
        bytes: &'static [u8],
        length: ExactByteLength,
    }

    impl SealedImmutableBacking for InconsistentLength {
        fn contiguous_bytes(&self) -> Result<&[u8], ObjectSourceError> {
            Ok(self.bytes)
        }
    }

    impl BoundedReReadableObjectSource for InconsistentLength {
        fn exact_byte_length(&self) -> ExactByteLength {
            self.length
        }

        fn read_exact_at(
            &self,
            _offset: u64,
            _destination: &mut [u8],
        ) -> Result<(), ObjectSourceError> {
            panic!("known contiguous length mismatch must fail before structured reads")
        }
    }

    #[test]
    fn source_length_mismatch_precedes_hash_and_parse() {
        let error = check_post_emission_bundle(
            InconsistentLength {
                bytes: &[0xa5],
                length: ExactByteLength::new(2),
            },
            &[][..],
            &[][..],
            ExpectedSemanticDiffBase::Genesis,
            FormatLimits::HARD,
        )
        .unwrap_err();
        assert_eq!(
            error,
            PostEmissionCheckError::ObjectSource {
                object: PortableObjectKind::CanonicalArtifact,
                error: ObjectSourceError::BackingChanged,
            }
        );
    }

    #[derive(Debug)]
    struct FailingSource {
        error: ObjectSourceError,
    }

    impl SealedImmutableBacking for FailingSource {
        fn contiguous_bytes(&self) -> Result<&[u8], ObjectSourceError> {
            Err(self.error)
        }
    }

    impl BoundedReReadableObjectSource for FailingSource {
        fn exact_byte_length(&self) -> ExactByteLength {
            ExactByteLength::new(0)
        }

        fn read_exact_at(
            &self,
            _offset: u64,
            _destination: &mut [u8],
        ) -> Result<(), ObjectSourceError> {
            Err(self.error)
        }
    }

    #[test]
    fn failure_in_each_bundle_object_returns_no_checked_capability() {
        let lfca_error = check_post_emission_bundle(
            FailingSource {
                error: ObjectSourceError::ReadFailed,
            },
            &[][..],
            &[][..],
            ExpectedSemanticDiffBase::Genesis,
            FormatLimits::HARD,
        )
        .unwrap_err();
        assert_eq!(
            lfca_error,
            PostEmissionCheckError::ObjectSource {
                object: PortableObjectKind::CanonicalArtifact,
                error: ObjectSourceError::ReadFailed,
            }
        );

        let lfsm_error = check_post_emission_bundle(
            &[][..],
            FailingSource {
                error: ObjectSourceError::ReadFailed,
            },
            &[][..],
            ExpectedSemanticDiffBase::Genesis,
            FormatLimits::HARD,
        )
        .unwrap_err();
        assert_eq!(
            lfsm_error,
            PostEmissionCheckError::ObjectSource {
                object: PortableObjectKind::SourceMap,
                error: ObjectSourceError::ReadFailed,
            }
        );

        let lfsd_error = check_post_emission_bundle(
            &[][..],
            &[][..],
            FailingSource {
                error: ObjectSourceError::ReadFailed,
            },
            ExpectedSemanticDiffBase::Genesis,
            FormatLimits::HARD,
        )
        .unwrap_err();
        assert_eq!(
            lfsd_error,
            PostEmissionCheckError::ObjectSource {
                object: PortableObjectKind::SemanticDiff,
                error: ObjectSourceError::ReadFailed,
            }
        );
    }

    #[derive(Debug)]
    struct DropTrackedSource {
        bytes: &'static [u8],
        drops: Arc<AtomicUsize>,
    }

    impl Drop for DropTrackedSource {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
    }

    impl SealedImmutableBacking for DropTrackedSource {
        fn contiguous_bytes(&self) -> Result<&[u8], ObjectSourceError> {
            Ok(self.bytes)
        }
    }

    impl BoundedReReadableObjectSource for DropTrackedSource {
        fn exact_byte_length(&self) -> ExactByteLength {
            ExactByteLength::new(self.bytes.len() as u64)
        }

        fn read_exact_at(
            &self,
            offset: u64,
            destination: &mut [u8],
        ) -> Result<(), ObjectSourceError> {
            self.bytes.read_exact_at(offset, destination)
        }
    }

    #[test]
    fn narrowing_bundle_drops_auxiliary_sources_and_keeps_lfca_source() {
        let drops = Arc::new(AtomicUsize::new(0));
        let tracked = |bytes| DropTrackedSource {
            bytes,
            drops: Arc::clone(&drops),
        };
        let bundle = check_post_emission_bundle(
            tracked(FULL_LFCA),
            tracked(FULL_LFSM),
            tracked(FULL_LFSD),
            ExpectedSemanticDiffBase::Genesis,
            FormatLimits::HARD,
        )
        .expect("checked bundle");
        assert_eq!(drops.load(Ordering::Relaxed), 0);

        let canonical = bundle.canonical_network_input();
        assert_eq!(drops.load(Ordering::Relaxed), 2);
        assert_eq!(canonical.value_checked_view().bytes(), FULL_LFCA);

        drop(canonical);
        assert_eq!(drops.load(Ordering::Relaxed), 3);
    }
}
