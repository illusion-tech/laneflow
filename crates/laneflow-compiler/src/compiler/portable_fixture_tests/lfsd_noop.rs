use super::*;

const EXPECTED_LFSD: &[u8] =
    include_bytes!("../../../tests/fixtures/portable-v1/lfsd-v1-noop/expected.lfsd");

fn candidate() -> crate::PortablePublicationCandidate {
    let output = full_spatial_portable_fixture_output();
    let provenance = full_spatial_portable_fixture_provenance();
    let base = laneflow_format::preflight_object_values_v1(
        FULL_SPATIAL_EXPECTED_LFCA,
        laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
        laneflow_format::FormatLimits::V1_HARD,
    )
    .unwrap();
    crate::emit_portable_candidate(
        &output,
        &provenance,
        laneflow_format::FormatLimits::V1_HARD,
        crate::PortableDiffBase::Artifact(base),
    )
    .unwrap()
}

#[test]
fn portable_full_spatial_noop_diff_matches_frozen_exact_bytes() {
    let candidate = candidate();
    assert_eq!(
        candidate.canonical_artifact().bytes(),
        FULL_SPATIAL_EXPECTED_LFCA
    );
    assert_eq!(candidate.source_map().bytes(), FULL_SPATIAL_EXPECTED_LFSM);
    assert_eq!(candidate.semantic_diff().bytes(), EXPECTED_LFSD);
    assert_eq!(
        candidate.semantic_diff().byte_length(),
        exact_byte_length(569)
    );
    assert_eq!(
        candidate.semantic_diff().object_key(),
        "sha256/5d72d97e935aa2ecddf2cc1c3cc6af033b7c115166d78de9e95526bc78d7f818"
    );
    assert_eq!(
        candidate.network_revision(),
        network_revision(FULL_SPATIAL_NETWORK_REVISION)
    );

    let diff = laneflow_format::preflight_object_values_v1(
        EXPECTED_LFSD,
        laneflow_static_contract::PortableObjectKind::SemanticDiff,
        laneflow_format::FormatLimits::V1_HARD,
    )
    .unwrap()
    .registry_view();
    let binding = diff.section(0).unwrap().table(0).unwrap().row(0).unwrap();
    assert!(matches!(
        binding.field_by_tag(1).unwrap().value().unwrap(),
        laneflow_format::RegistryCheckedFieldValue::U8(1)
    ));
    for tag in [2, 6] {
        assert!(matches!(
            binding.field_by_tag(tag).unwrap().value().unwrap(),
            laneflow_format::RegistryCheckedFieldValue::U16(1)
        ));
    }
    for tag in [3, 7] {
        assert_eq!(
            binding.field_by_tag(tag).unwrap().value_bytes(),
            FULL_SPATIAL_NETWORK_REVISION
        );
    }
    let artifact_digest = candidate.canonical_artifact().digest();
    for tag in [4, 8] {
        assert_eq!(
            binding.field_by_tag(tag).unwrap().value_bytes(),
            artifact_digest.as_bytes()
        );
    }
    for tag in [5, 9] {
        assert!(matches!(
            binding.field_by_tag(tag).unwrap().value().unwrap(),
            laneflow_format::RegistryCheckedFieldValue::U64(14_649)
        ));
    }
    for section_ordinal in 1..6 {
        assert_eq!(
            diff.section(section_ordinal)
                .unwrap()
                .table(0)
                .unwrap()
                .row_count(),
            0
        );
    }
}
