use std::ops::Range;

use laneflow_format::{
    ExpectedSemanticDiffBase, FormatLimitConfig, FormatLimits, LimitDimension,
    PostEmissionCheckError, RegistryCheckedObjectView, check_post_emission_bundle,
    preflight_object_values,
};
use laneflow_static_contract::{ExactByteLength, PortableObjectKind};

use super::*;
use crate::compiler::portable_fixture_tests;

fn provenance(kind: PortablePublisherKind) -> PortablePublicationProvenance {
    PortablePublicationProvenance::new(kind, "laneflow-publisher-fixture-v2", None, None)
}

fn descriptor_view(bytes: &[u8]) -> RegistryCheckedObjectView<'_> {
    preflight_object_values(
        bytes,
        PortableObjectKind::CanonicalPublicationDescriptor,
        FormatLimits::HARD,
    )
    .unwrap()
    .registry_view()
}

fn field_bytes(bytes: &[u8], section: u32, tag: u16) -> &[u8] {
    descriptor_view(bytes)
        .section(section)
        .unwrap()
        .table(0)
        .unwrap()
        .row(0)
        .unwrap()
        .field_by_tag(tag)
        .unwrap()
        .value_bytes()
}

fn field_utf8(bytes: &[u8], section: u32, tag: u16) -> &str {
    std::str::from_utf8(field_bytes(bytes, section, tag)).unwrap()
}

fn checked_bundle(
    candidate: &PortablePublicationCandidate,
) -> laneflow_format::PostEmissionCheckedBundle<&[u8], &[u8], &[u8]> {
    check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .unwrap()
}

fn field_range(bytes: &[u8], kind: PortableObjectKind, section: u32, tag: u16) -> Range<usize> {
    let value = preflight_object_values(bytes, kind, FormatLimits::HARD)
        .unwrap()
        .registry_view()
        .section(section)
        .unwrap()
        .table(0)
        .unwrap()
        .row(0)
        .unwrap()
        .field_by_tag(tag)
        .unwrap()
        .value_bytes();
    let start = (value.as_ptr() as usize)
        .checked_sub(bytes.as_ptr() as usize)
        .unwrap();
    start..start + value.len()
}

fn mutate_field(bytes: &mut [u8], kind: PortableObjectKind, section: u32, tag: u16) {
    let range = field_range(bytes, kind, section, tag);
    bytes[range.start] ^= 1;
    portable_fixture_tests::refresh_portable_chunk_digest_containing(bytes, kind, range.start);
}

#[test]
fn checker_accepts_genesis_and_artifact_base_bundles() {
    let genesis = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    let genesis_checked = checked_bundle(&genesis);
    assert_eq!(
        genesis_checked.network_revision(),
        genesis.network_revision()
    );
    assert_eq!(
        genesis_checked.canonical_artifact_digest(),
        genesis.canonical_artifact().digest()
    );

    let artifact = portable_fixture_tests::full_spatial_portable_artifact_base_fixture_candidate();
    assert!(matches!(
        artifact.expected_semantic_diff_base(),
        ExpectedSemanticDiffBase::Artifact { .. }
    ));
    let artifact_checked = checked_bundle(&artifact);
    assert_eq!(
        artifact_checked.network_revision(),
        artifact.network_revision()
    );
}

#[test]
fn checker_reports_each_cross_object_binding_failure() {
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();

    let mut lfca = candidate.canonical_artifact().bytes().to_vec();
    mutate_field(&mut lfca, PortableObjectKind::CanonicalArtifact, 7, 1);
    assert_eq!(
        check_post_emission_bundle(
            lfca.as_slice(),
            candidate.source_map().bytes(),
            candidate.semantic_diff().bytes(),
            candidate.expected_semantic_diff_base(),
            FormatLimits::HARD,
        )
        .unwrap_err(),
        PostEmissionCheckError::NetworkRevisionMismatch
    );

    let mut lfsm = candidate.source_map().bytes().to_vec();
    mutate_field(&mut lfsm, PortableObjectKind::SourceMap, 0, 4);
    assert_eq!(
        check_post_emission_bundle(
            candidate.canonical_artifact().bytes(),
            lfsm.as_slice(),
            candidate.semantic_diff().bytes(),
            candidate.expected_semantic_diff_base(),
            FormatLimits::HARD,
        )
        .unwrap_err(),
        PostEmissionCheckError::SourceMapBindingMismatch
    );

    let mut target_lfsd = candidate.semantic_diff().bytes().to_vec();
    mutate_field(&mut target_lfsd, PortableObjectKind::SemanticDiff, 0, 8);
    assert_eq!(
        check_post_emission_bundle(
            candidate.canonical_artifact().bytes(),
            candidate.source_map().bytes(),
            target_lfsd.as_slice(),
            candidate.expected_semantic_diff_base(),
            FormatLimits::HARD,
        )
        .unwrap_err(),
        PostEmissionCheckError::SemanticDiffTargetBindingMismatch
    );

    assert_eq!(
        check_post_emission_bundle(
            candidate.canonical_artifact().bytes(),
            candidate.source_map().bytes(),
            candidate.semantic_diff().bytes(),
            ExpectedSemanticDiffBase::Artifact {
                network_revision_derivation_version: 1,
                network_revision: candidate.network_revision(),
                digest: candidate.canonical_artifact().digest(),
                byte_length: candidate.canonical_artifact().byte_length(),
            },
            FormatLimits::HARD,
        )
        .unwrap_err(),
        PostEmissionCheckError::SemanticDiffBaseBindingMismatch
    );
}

#[test]
fn checker_rejects_truncated_appended_wrong_kind_and_wrong_version_objects() {
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    let originals = [
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
    ];

    for object_index in 0..3 {
        for mutation in 0..3 {
            let mut changed = originals[object_index].to_vec();
            match mutation {
                0 => {
                    changed.pop();
                }
                1 => changed.push(0),
                2 => changed[4..6].copy_from_slice(&9_u16.to_le_bytes()),
                _ => unreachable!(),
            }
            let mut objects = originals;
            objects[object_index] = &changed;
            assert!(matches!(
                check_post_emission_bundle(
                    objects[0],
                    objects[1],
                    objects[2],
                    candidate.expected_semantic_diff_base(),
                    FormatLimits::HARD,
                ),
                Err(PostEmissionCheckError::Format(_))
            ));
        }
    }

    assert!(matches!(
        check_post_emission_bundle(
            candidate.source_map().bytes(),
            candidate.canonical_artifact().bytes(),
            candidate.semantic_diff().bytes(),
            candidate.expected_semantic_diff_base(),
            FormatLimits::HARD,
        ),
        Err(PostEmissionCheckError::Format(_))
    ));
}

#[test]
fn checker_closes_all_variable_source_and_diff_digest_length_bindings() {
    let genesis = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    for tag in [2, 4, 5, 6, 8] {
        let mut lfsm = genesis.source_map().bytes().to_vec();
        mutate_field(&mut lfsm, PortableObjectKind::SourceMap, 0, tag);
        assert_eq!(
            check_post_emission_bundle(
                genesis.canonical_artifact().bytes(),
                lfsm.as_slice(),
                genesis.semantic_diff().bytes(),
                genesis.expected_semantic_diff_base(),
                FormatLimits::HARD,
            )
            .unwrap_err(),
            PostEmissionCheckError::SourceMapBindingMismatch
        );
    }

    let artifact = portable_fixture_tests::full_spatial_portable_artifact_base_fixture_candidate();
    for tag in [3, 4, 5] {
        let mut lfsd = artifact.semantic_diff().bytes().to_vec();
        mutate_field(&mut lfsd, PortableObjectKind::SemanticDiff, 0, tag);
        assert_eq!(
            check_post_emission_bundle(
                artifact.canonical_artifact().bytes(),
                artifact.source_map().bytes(),
                lfsd.as_slice(),
                artifact.expected_semantic_diff_base(),
                FormatLimits::HARD,
            )
            .unwrap_err(),
            PostEmissionCheckError::SemanticDiffBaseBindingMismatch
        );
    }
    for tag in [7, 8, 9] {
        let mut lfsd = artifact.semantic_diff().bytes().to_vec();
        mutate_field(&mut lfsd, PortableObjectKind::SemanticDiff, 0, tag);
        assert_eq!(
            check_post_emission_bundle(
                artifact.canonical_artifact().bytes(),
                artifact.source_map().bytes(),
                lfsd.as_slice(),
                artifact.expected_semantic_diff_base(),
                FormatLimits::HARD,
            )
            .unwrap_err(),
            PostEmissionCheckError::SemanticDiffTargetBindingMismatch
        );
    }
}

#[test]
fn checker_rejects_every_fixed_binding_version_during_value_preflight() {
    let genesis = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    for tag in [1, 3, 7] {
        let mut lfsm = genesis.source_map().bytes().to_vec();
        mutate_field(&mut lfsm, PortableObjectKind::SourceMap, 0, tag);
        assert!(matches!(
            check_post_emission_bundle(
                genesis.canonical_artifact().bytes(),
                lfsm.as_slice(),
                genesis.semantic_diff().bytes(),
                genesis.expected_semantic_diff_base(),
                FormatLimits::HARD,
            ),
            Err(PostEmissionCheckError::Format(_))
        ));
    }

    let artifact = portable_fixture_tests::full_spatial_portable_artifact_base_fixture_candidate();
    for tag in [2, 6] {
        let mut lfsd = artifact.semantic_diff().bytes().to_vec();
        mutate_field(&mut lfsd, PortableObjectKind::SemanticDiff, 0, tag);
        assert!(matches!(
            check_post_emission_bundle(
                artifact.canonical_artifact().bytes(),
                artifact.source_map().bytes(),
                lfsd.as_slice(),
                artifact.expected_semantic_diff_base(),
                FormatLimits::HARD,
            ),
            Err(PostEmissionCheckError::Format(_))
        ));
    }
}

#[test]
fn lfcp_v2_contains_only_artifact_source_and_publication_bindings() {
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    for (kind, code) in [
        (PortablePublisherKind::LocalTool, 0_u8),
        (PortablePublisherKind::Ci, 1),
        (PortablePublisherKind::ReleaseService, 2),
    ] {
        let descriptor = build_lfcp(
            &checked_bundle(&candidate),
            &PortablePublicationProvenance::new(
                kind,
                "laneflow-publisher-fixture-v2",
                Some("controlled-build".into()),
                Some("2026-08-18T00:00:00Z".into()),
            ),
            FormatLimits::HARD,
        )
        .unwrap();
        let view = descriptor_view(descriptor.bytes());
        assert_eq!(view.section_count(), 3);
        assert_eq!(field_bytes(descriptor.bytes(), 2, 1), [code]);
        assert_eq!(
            field_utf8(descriptor.bytes(), 2, 3),
            candidate.canonical_artifact().object_key()
        );
        assert_eq!(
            field_utf8(descriptor.bytes(), 2, 4),
            candidate.source_map().object_key()
        );
        assert_eq!(field_utf8(descriptor.bytes(), 2, 5), "controlled-build");
        assert_eq!(field_utf8(descriptor.bytes(), 2, 6), "2026-08-18T00:00:00Z");
    }
}

#[test]
fn descriptor_construction_stops_at_candidate_check_failure() {
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    let largest = [
        candidate.canonical_artifact().byte_length().get(),
        candidate.source_map().byte_length().get(),
        candidate.semantic_diff().byte_length().get(),
    ]
    .into_iter()
    .max()
    .unwrap();
    let mut config = FormatLimitConfig::HARD;
    config.max_object_bytes = largest - 1;

    assert!(matches!(
        build_portable_publication_descriptor(
            candidate,
            &provenance(PortablePublisherKind::LocalTool),
            FormatLimits::try_new(config).unwrap(),
        ),
        Err(PortablePublicationError::PostEmission(
            PostEmissionCheckError::LimitExceeded {
                dimension: LimitDimension::ObjectBytes,
                ..
            }
        ))
    ));
}

#[test]
fn checker_does_not_treat_complete_bundle_as_staged_chunk_scratch() {
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    let total = candidate
        .canonical_artifact()
        .byte_length()
        .get()
        .checked_add(candidate.source_map().byte_length().get())
        .and_then(|value| value.checked_add(candidate.semantic_diff().byte_length().get()))
        .unwrap();

    let mut config = FormatLimitConfig::HARD;
    config.max_staged_chunk_bytes = total - 1;
    check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::try_new(config).unwrap(),
    )
    .unwrap();
}

#[test]
fn lfcp_v2_exact_bytes_are_deterministic() {
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    let checked = checked_bundle(&candidate);
    let provenance = PortablePublicationProvenance::new(
        PortablePublisherKind::ReleaseService,
        "laneflow-publisher-fixture-v2",
        Some("controlled-build".into()),
        Some("2026-08-18T00:00:00Z".into()),
    );
    let first = build_lfcp(&checked, &provenance, FormatLimits::HARD).unwrap();
    let second = build_lfcp(&checked, &provenance, FormatLimits::HARD).unwrap();
    if std::env::var_os("DUMP_PORTABLE").is_some() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/portable/lfcp-min-bindings");
        let hex = first
            .bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        std::fs::write(dir.join("expected.lfcp.hex"), format!("{hex}\n")).unwrap();
        std::fs::write(
            dir.join("bindings.txt"),
            format!("len={}\nkey={}\n", first.bytes().len(), first.object_key()),
        )
        .unwrap();
        return;
    }
    let expected_hex =
        include_str!("../../tests/fixtures/portable/lfcp-min-bindings/expected.lfcp.hex");
    let digits = expected_hex
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    let (pairs, remainder) = digits.as_chunks::<2>();
    assert!(remainder.is_empty());
    let expected = pairs
        .iter()
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16).unwrap();
            let low = (pair[1] as char).to_digit(16).unwrap();
            u8::try_from((high << 4) | low).unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(first.bytes(), expected);
    assert_eq!(first.byte_length(), ExactByteLength::new(800));
    assert_eq!(
        first.object_key(),
        "sha256/4e5a1be12f627ae5040bce32caddbbb47b0a942775611f4131ed22673002dd59"
    );
    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.digest(), second.digest());
}
