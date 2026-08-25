//! 受检 LFCA 到共享静态路网之间的不可伪造进程内能力。

use laneflow_static_contract::{
    ExactByteLength, NETWORK_REVISION_DOMAIN_PREFIX, NetworkRevisionId, PortableObjectKind,
    SECTION_FORMAT_VERSION, Sha256Digest,
};
use sha2::{Digest, Sha256};

use crate::{
    FormatError, FormatLimits, FormatStructure, LimitDimension, RegistryCheckedFieldValue,
    ValueCheckedObjectView, preflight_object_values,
};

/// 单份规范路网输入检查的稳定失败分类。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CanonicalNetworkInputError {
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

/// 已完成 LFCA framing、registry、直接值域和修订绑定检查的借用型能力。
///
/// 字段私有且只由本 crate 的检查入口构造。它不证明发布真实性；
/// `laneflow-static-network` 仍须完成跨表、身份和 Traffic/Spatial 闭合。
#[derive(Clone, Copy, Debug)]
pub struct CheckedCanonicalNetworkInput<'a> {
    view: ValueCheckedObjectView<'a>,
    canonical_artifact_digest: Sha256Digest,
    canonical_artifact_byte_length: ExactByteLength,
    network_revision: NetworkRevisionId,
}

impl<'a> CheckedCanonicalNetworkInput<'a> {
    /// 供共享静态路网构建器顺序消费的受检 LFCA view。
    #[must_use]
    pub const fn value_checked_view(self) -> ValueCheckedObjectView<'a> {
        self.view
    }

    #[must_use]
    pub const fn canonical_artifact_digest(self) -> Sha256Digest {
        self.canonical_artifact_digest
    }

    #[must_use]
    pub const fn canonical_artifact_byte_length(self) -> ExactByteLength {
        self.canonical_artifact_byte_length
    }

    #[must_use]
    pub const fn network_revision(self) -> NetworkRevisionId {
        self.network_revision
    }
}

pub(crate) fn checked_canonical_network_input_from_parts(
    view: ValueCheckedObjectView<'_>,
    canonical_artifact_digest: Sha256Digest,
    canonical_artifact_byte_length: ExactByteLength,
) -> Result<CheckedCanonicalNetworkInput<'_>, CanonicalNetworkInputError> {
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

    Ok(CheckedCanonicalNetworkInput {
        view,
        canonical_artifact_digest,
        canonical_artifact_byte_length,
        network_revision,
    })
}

/// 对一份 exact LFCA 建立共享静态路网构建所需的受检输入能力。
pub fn check_canonical_network_input(
    lfca: &[u8],
    limits: FormatLimits,
) -> Result<CheckedCanonicalNetworkInput<'_>, CanonicalNetworkInputError> {
    let byte_length =
        u64::try_from(lfca.len()).map_err(|_| CanonicalNetworkInputError::ArithmeticOverflow)?;
    let limit = limits.max_object_bytes();
    if byte_length > limit {
        return Err(CanonicalNetworkInputError::LimitExceeded {
            dimension: LimitDimension::ObjectBytes,
            actual: byte_length,
            limit,
        });
    }

    let view = preflight_object_values(lfca, PortableObjectKind::CanonicalArtifact, limits)?;
    checked_canonical_network_input_from_parts(
        view,
        sha256(lfca),
        ExactByteLength::new(byte_length),
    )
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
        hasher.update(SECTION_FORMAT_VERSION.to_le_bytes());
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
        "../../laneflow-compiler/tests/fixtures/portable-v2/lfca-v2-variants/min-headless.lfca"
    );
    const CLAIM_MISMATCH: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable-v2/lfca-v2-variants/claim-mismatch.lfca"
    );
    const FULL_LFCA: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable-v2/lfca-v2-full-spatial/expected.lfca"
    );
    const FULL_LFSM: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable-v2/lfca-v2-full-spatial/expected.lfsm"
    );
    const FULL_LFSD: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable-v2/lfca-v2-full-spatial/expected.lfsd"
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
        assert!(check_canonical_network_input(&bytes, FormatLimits::HARD).is_err());
        bytes[4..6].copy_from_slice(&3_u16.to_le_bytes());
        assert!(check_canonical_network_input(&bytes, FormatLimits::HARD).is_err());
    }

    #[test]
    fn accepts_current_artifact() {
        let checked =
            check_canonical_network_input(FULL_LFCA, FormatLimits::HARD).expect("canonical input");
        assert_eq!(checked.value_checked_view().bytes(), FULL_LFCA);
        let bundle = check_post_emission_bundle(
            FULL_LFCA,
            FULL_LFSM,
            FULL_LFSD,
            ExpectedSemanticDiffBase::Genesis,
            FormatLimits::HARD,
        )
        .expect("bundle");
        assert_eq!(
            checked.network_revision(),
            bundle.canonical_network_input().network_revision()
        );
    }

    #[test]
    fn bundle_accessor_is_identical_to_single_object_check() {
        let direct = check_canonical_network_input(FULL_LFCA, FormatLimits::HARD)
            .expect("direct canonical input");
        let bundle = check_post_emission_bundle(
            FULL_LFCA,
            FULL_LFSM,
            FULL_LFSD,
            ExpectedSemanticDiffBase::Genesis,
            FormatLimits::HARD,
        )
        .expect("checked bundle")
        .canonical_network_input();

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
