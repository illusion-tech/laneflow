//! 受检 LFCA 到共享静态路网之间的不可伪造进程内能力。

use laneflow_static_contract::{
    ExactByteLength, NETWORK_REVISION_DOMAIN_PREFIX, NetworkRevisionId, PortableObjectKind,
    Sha256Digest,
};
use sha2::{Digest, Sha256};

use crate::{
    BoundedReReadableObjectSource, FormatError, FormatLimits, FormatStructure, LimitDimension,
    ObjectSourceError, RegistryCheckedFieldValue, ValueCheckedObjectView,
    object_source::contiguous_bytes, preflight_object_values, value::ValueCheckProof,
};

/// 单份规范路网输入检查的稳定失败分类。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CanonicalNetworkInputError {
    ObjectSource(ObjectSourceError),
    Format(FormatError),
    LimitExceeded {
        dimension: LimitDimension,
        actual: u64,
        limit: u64,
    },
    NetworkRevisionMismatch,
    ArithmeticOverflow,
}

impl From<FormatError> for CanonicalNetworkInputError {
    fn from(value: FormatError) -> Self {
        Self::Format(value)
    }
}

/// 已完成 LFCA framing、registry、直接值域和修订绑定检查的来源拥有型能力。
///
/// 字段私有且只由本 crate 的检查入口构造。它不证明发布真实性；
/// `laneflow-static-network` 仍须完成跨表、身份和 Traffic/Spatial 闭合。
#[derive(Clone, Debug)]
pub struct CheckedCanonicalNetworkInput<S> {
    source: S,
    proof: ValueCheckProof,
    canonical_artifact_digest: Sha256Digest,
    canonical_artifact_byte_length: ExactByteLength,
    network_revision: NetworkRevisionId,
}

/// 成功 LFCA revision 闭合后可随同值域 proof 做 O(1) 重借用的 crate-private 证明。
#[derive(Clone, Copy, Debug)]
pub(crate) struct CanonicalNetworkInputProof {
    network_revision: NetworkRevisionId,
}

impl<S> CheckedCanonicalNetworkInput<S>
where
    S: BoundedReReadableObjectSource,
{
    pub(crate) fn from_parts(
        source: S,
        proof: ValueCheckProof,
        canonical_artifact_digest: Sha256Digest,
        canonical_artifact_byte_length: ExactByteLength,
        canonical_network_proof: CanonicalNetworkInputProof,
    ) -> Self {
        Self {
            source,
            proof,
            canonical_artifact_digest,
            canonical_artifact_byte_length,
            network_revision: canonical_network_proof.network_revision,
        }
    }

    /// 供共享静态路网构建器顺序消费的受检 LFCA view。
    #[must_use]
    pub fn value_checked_view(&self) -> ValueCheckedObjectView<'_> {
        let bytes = contiguous_bytes(&self.source)
            .expect("checked immutable canonical source cannot become unreadable");
        self.proof
            .reborrow(bytes)
            .expect("checked immutable canonical source length cannot drift")
    }

    #[must_use]
    pub const fn canonical_artifact_digest(&self) -> Sha256Digest {
        self.canonical_artifact_digest
    }

    #[must_use]
    pub const fn canonical_artifact_byte_length(&self) -> ExactByteLength {
        self.canonical_artifact_byte_length
    }

    #[must_use]
    pub const fn network_revision(&self) -> NetworkRevisionId {
        self.network_revision
    }
}

impl CanonicalNetworkInputProof {
    pub(crate) const fn network_revision(self) -> NetworkRevisionId {
        self.network_revision
    }
}

pub(crate) fn check_canonical_network_input_binding(
    view: ValueCheckedObjectView<'_>,
) -> Result<CanonicalNetworkInputProof, CanonicalNetworkInputError> {
    let network_revision = recompute_network_revision(view)?;
    let declared_revision = view
        .registry_view()
        .section(7)
        .and_then(|section| section.table(0))
        .and_then(|table| table.row(0))
        .and_then(|row| row.field_by_tag(1))
        .ok_or_else(binding_format_error)?;
    let declared_revision = match declared_revision.value()? {
        RegistryCheckedFieldValue::Sha256(value) => value,
        _ => return Err(binding_format_error()),
    };
    if declared_revision != network_revision.into_digest() {
        return Err(CanonicalNetworkInputError::NetworkRevisionMismatch);
    }

    Ok(CanonicalNetworkInputProof { network_revision })
}

/// 对一份 exact LFCA 建立共享静态路网构建所需的受检输入能力。
pub fn check_canonical_network_input<S>(
    lfca: S,
    limits: FormatLimits,
) -> Result<CheckedCanonicalNetworkInput<S>, CanonicalNetworkInputError>
where
    S: BoundedReReadableObjectSource,
{
    let byte_length = lfca.exact_byte_length().get();
    let limit = limits.max_object_bytes();
    if byte_length > limit {
        return Err(CanonicalNetworkInputError::LimitExceeded {
            dimension: LimitDimension::ObjectBytes,
            actual: byte_length,
            limit,
        });
    }

    let bytes = contiguous_bytes(&lfca).map_err(CanonicalNetworkInputError::ObjectSource)?;
    if usize::try_from(byte_length).ok() != Some(bytes.len()) {
        return Err(CanonicalNetworkInputError::ObjectSource(
            ObjectSourceError::BackingChanged,
        ));
    }
    let view = preflight_object_values(bytes, PortableObjectKind::CanonicalArtifact, limits)?;
    let digest = sha256(bytes);
    let proof = view.proof();
    let canonical_network_proof = check_canonical_network_input_binding(view)?;
    Ok(CheckedCanonicalNetworkInput::from_parts(
        lfca,
        proof,
        digest,
        ExactByteLength::new(byte_length),
        canonical_network_proof,
    ))
}

fn recompute_network_revision(
    view: ValueCheckedObjectView<'_>,
) -> Result<NetworkRevisionId, CanonicalNetworkInputError> {
    let registry = view.registry_view();
    let mut hasher = Sha256::new();
    hasher.update(NETWORK_REVISION_DOMAIN_PREFIX);
    for ordinal in 0..6_u32 {
        let section = registry.section(ordinal).ok_or_else(binding_format_error)?;
        let section_length = u64::try_from(section.bytes().len())
            .map_err(|_| CanonicalNetworkInputError::ArithmeticOverflow)?;
        hasher.update(section.kind().to_le_bytes());
        hasher.update(
            PortableObjectKind::CanonicalArtifact
                .section_format_version()
                .to_le_bytes(),
        );
        hasher.update(section_length.to_le_bytes());
        hasher.update(section.bytes());
    }
    Ok(NetworkRevisionId::from_digest(Sha256Digest::from_bytes(
        hasher.finalize().into(),
    )))
}

fn sha256(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
}

const fn binding_format_error() -> CanonicalNetworkInputError {
    CanonicalNetworkInputError::Format(FormatError::BindingMismatch {
        structure: FormatStructure::FieldValue,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExpectedSemanticDiffBase, check_post_emission_bundle};

    const MIN_HEADLESS: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-variants/min-headless.lfca"
    );
    const CLAIM_MISMATCH: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-variants/claim-mismatch.lfca"
    );
    const FULL_LFCA: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
    );
    const FULL_LFSM: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfsm"
    );
    const FULL_LFSD: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfsd"
    );

    #[test]
    fn checks_minimal_headless_lfca_and_binds_actual_bytes() {
        let checked = check_canonical_network_input(MIN_HEADLESS, FormatLimits::HARD)
            .expect("canonical input");

        assert_eq!(checked.value_checked_view().bytes(), MIN_HEADLESS);
        assert_eq!(
            checked.canonical_artifact_byte_length().get(),
            u64::try_from(MIN_HEADLESS.len()).expect("fixture length")
        );
        assert_ne!(checked.canonical_artifact_digest(), Sha256Digest::ZERO);
        assert_ne!(checked.network_revision().into_digest(), Sha256Digest::ZERO);
    }

    #[test]
    fn rejects_lfca_with_mismatched_revision_claim() {
        assert!(matches!(
            check_canonical_network_input(CLAIM_MISMATCH, FormatLimits::HARD),
            Err(CanonicalNetworkInputError::NetworkRevisionMismatch)
        ));
    }

    #[test]
    fn rejects_non_current_format_version() {
        let mut bytes = FULL_LFCA.to_vec();
        bytes[4..6].copy_from_slice(&1_u16.to_le_bytes());
        assert!(check_canonical_network_input(bytes.as_slice(), FormatLimits::HARD).is_err());
        bytes[4..6].copy_from_slice(&2_u16.to_le_bytes());
        assert!(check_canonical_network_input(bytes.as_slice(), FormatLimits::HARD).is_err());
    }

    #[test]
    fn accepts_current_artifact() {
        let checked =
            check_canonical_network_input(FULL_LFCA, FormatLimits::HARD).expect("canonical input");
        assert_eq!(checked.value_checked_view().bytes(), FULL_LFCA);
        let checked_bundle = check_post_emission_bundle(
            FULL_LFCA,
            FULL_LFSM,
            FULL_LFSD,
            ExpectedSemanticDiffBase::Genesis,
            FormatLimits::HARD,
        )
        .expect("bundle");
        assert_eq!(
            checked.network_revision(),
            checked_bundle.canonical_network_input().network_revision()
        );
    }

    #[test]
    fn bundle_accessor_is_identical_to_single_object_check() {
        let direct = check_canonical_network_input(FULL_LFCA, FormatLimits::HARD)
            .expect("direct canonical input");
        let checked_bundle = check_post_emission_bundle(
            FULL_LFCA,
            FULL_LFSM,
            FULL_LFSD,
            ExpectedSemanticDiffBase::Genesis,
            FormatLimits::HARD,
        )
        .expect("checked bundle");
        let bundle = checked_bundle.canonical_network_input();

        assert_eq!(
            direct.canonical_artifact_digest(),
            bundle.canonical_artifact_digest()
        );
        assert_eq!(
            direct.canonical_artifact_byte_length(),
            bundle.canonical_artifact_byte_length()
        );
        assert_eq!(direct.network_revision(), bundle.network_revision());
        assert_eq!(
            direct.value_checked_view().bytes(),
            bundle.value_checked_view().bytes()
        );
    }
}
