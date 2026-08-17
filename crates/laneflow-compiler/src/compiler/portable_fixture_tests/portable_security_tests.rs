use std::io::Cursor;

use laneflow_format::{
    FormatError, FormatLimitConfig, FormatLimits, LimitDimension, preflight_object_framing,
    preflight_object_values_v1,
};
use laneflow_static_contract::PortableObjectKind;
use sha2::{Digest, Sha256};

use super::*;

fn assert_each_section_bit_change_breaks_digest(
    bytes: &[u8],
    expected_digest: [u8; 32],
    kind: PortableObjectKind,
) {
    let framing = preflight_object_framing(bytes, kind, FormatLimits::V1_HARD).unwrap();
    let actual_digest: [u8; 32] = Sha256::digest(bytes).into();
    assert_eq!(actual_digest, expected_digest);
    for ordinal in 0..framing.section_count() {
        let section = framing.section(ordinal).unwrap();
        assert!(!section.bytes().is_empty());
        let offset = section.bytes().as_ptr() as usize - bytes.as_ptr() as usize;
        let mut corrupted = bytes.to_vec();
        corrupted[offset] ^= 1;
        let corrupted_digest: [u8; 32] = Sha256::digest(&corrupted).into();
        assert_ne!(
            corrupted_digest,
            expected_digest,
            "{kind:?} section {} first-bit corruption preserved the object digest",
            ordinal + 1
        );
    }
}

#[test]
fn every_candidate_section_single_bit_corruption_breaks_its_digest_binding() {
    let candidate = full_spatial_portable_fixture_candidate();
    assert_each_section_bit_change_breaks_digest(
        FULL_SPATIAL_EXPECTED_LFCA,
        candidate.canonical_artifact().digest().into_bytes(),
        PortableObjectKind::CanonicalArtifact,
    );
    assert_each_section_bit_change_breaks_digest(
        FULL_SPATIAL_EXPECTED_LFSM,
        candidate.source_map().digest().into_bytes(),
        PortableObjectKind::SourceMap,
    );
    assert_each_section_bit_change_breaks_digest(
        FULL_SPATIAL_EXPECTED_LFSD,
        candidate.semantic_diff().digest().into_bytes(),
        PortableObjectKind::SemanticDiff,
    );
}

#[test]
fn valid_object_above_caller_transport_limit_rejects_before_read() {
    preflight_object_values_v1(
        FULL_SPATIAL_EXPECTED_LFCA,
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::V1_HARD,
    )
    .unwrap();
    let mut config = FormatLimitConfig::V1_HARD;
    config.max_object_bytes = FULL_SPATIAL_EXPECTED_LFCA.len() as u64 - 1;
    let limits = FormatLimits::try_new(config).unwrap();
    let mut reader = Cursor::new(FULL_SPATIAL_EXPECTED_LFCA);
    assert_eq!(
        crate::read_portable_object_known_length(
            &mut reader,
            FULL_SPATIAL_EXPECTED_LFCA.len() as u64,
            limits,
        ),
        Err(crate::PortableReadError::LimitExceeded {
            actual: FULL_SPATIAL_EXPECTED_LFCA.len() as u64,
            limit: FULL_SPATIAL_EXPECTED_LFCA.len() as u64 - 1,
        })
    );
    assert_eq!(reader.position(), 0);
}

#[test]
fn value_preflight_honors_a_reduced_identity_ascii_limit() {
    let mut config = FormatLimitConfig::V1_HARD;
    config.max_identity_ascii_bytes = 1;
    let error = preflight_object_values_v1(
        FULL_SPATIAL_EXPECTED_LFCA,
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::try_new(config).unwrap(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        FormatError::LimitExceeded {
            dimension: LimitDimension::IdentityAsciiBytes,
            actual,
            limit: 1,
        } if actual > 1
    ));
}
