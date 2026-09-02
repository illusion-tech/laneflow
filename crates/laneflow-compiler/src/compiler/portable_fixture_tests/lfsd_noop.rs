use super::*;

const EXPECTED_LFSD: &[u8] =
    include_bytes!("../../../tests/fixtures/portable/lfsd-noop/expected.lfsd");

fn candidate() -> crate::PortablePublicationCandidate {
    let output = full_spatial_portable_fixture_output();
    let provenance = full_spatial_portable_fixture_provenance();
    let base = laneflow_format::preflight_object_values(
        FULL_SPATIAL_EXPECTED_LFCA,
        laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap();
    crate::emit_portable_candidate(
        &output,
        &provenance,
        laneflow_format::FormatLimits::HARD,
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
        exact_byte_length(633)
    );
    assert_eq!(
        candidate.semantic_diff().object_key(),
        "sha256/a9e8a973cee5d23f838a4f2c62ddf0e99166bd68bbaac9136fbf0f24b2882ca0"
    );
    assert_eq!(
        candidate.network_revision(),
        network_revision(FULL_SPATIAL_NETWORK_REVISION)
    );

    let diff = laneflow_format::preflight_object_values(
        EXPECTED_LFSD,
        laneflow_static_contract::PortableObjectKind::SemanticDiff,
        laneflow_format::FormatLimits::HARD,
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
            laneflow_format::RegistryCheckedFieldValue::U64(30_400)
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

#[test]
fn dump_portable_noop_when_requested() {
    if std::env::var_os("DUMP_PORTABLE").is_none() {
        return;
    }
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/portable/lfsd-noop");
    std::fs::create_dir_all(&dir).unwrap();
    let candidate = candidate();
    std::fs::write(dir.join("expected.lfsd"), candidate.semantic_diff().bytes()).unwrap();
    std::fs::write(
        dir.join("bindings.txt"),
        format!(
            "len={}\nkey={}\nlfca_len={}\n",
            candidate.semantic_diff().bytes().len(),
            candidate.semantic_diff().object_key(),
            candidate.canonical_artifact().bytes().len(),
        ),
    )
    .unwrap();
}
