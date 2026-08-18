use std::{cell::Cell, cell::RefCell, ops::Range};

use laneflow_format::{
    ExpectedSemanticDiffBaseV1, FormatLimitConfig, FormatLimits, LimitDimension,
    PostEmissionCheckError, RegistryCheckedObjectView, check_post_emission_bundle_v1,
    preflight_object_values_v1,
};
use laneflow_static_contract::{ExactByteLength, PortableObjectKind};

use super::*;
use crate::{PortableInstallOperation, compiler::portable_fixture_tests};

fn provenance(kind: PortablePublisherKindV2) -> PortablePublicationProvenanceV2 {
    PortablePublicationProvenanceV2::new(kind, "laneflow-publisher-fixture-v2", None, None)
}

#[derive(Default)]
struct RecordingManifest {
    calls: usize,
    failure: Option<PortableManifestCommitError>,
    descriptor_bytes: Option<Box<[u8]>>,
    descriptor_key: Option<Box<str>>,
}

impl PortableManifestCommitter for RecordingManifest {
    fn commit_authenticated_manifest(
        &mut self,
        candidate: PortableManifestCommitCandidate<'_>,
    ) -> Result<(), PortableManifestCommitError> {
        self.calls += 1;
        assert_eq!(
            candidate.descriptor().digest(),
            candidate.descriptor_installation().digest()
        );
        assert_eq!(
            candidate.canonical_artifact_installation().object_key(),
            field_utf8(candidate.descriptor().bytes(), 2, 3)
        );
        assert_eq!(
            candidate.source_map_installation().object_key(),
            field_utf8(candidate.descriptor().bytes(), 2, 4)
        );
        self.descriptor_bytes = Some(candidate.descriptor().bytes().into());
        self.descriptor_key = Some(candidate.descriptor().object_key().into());
        match self.failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

struct RecordingInstaller {
    calls: Cell<usize>,
    order: RefCell<Vec<[u8; 4]>>,
    fail_on_call: Option<usize>,
    error: PortableInstallError,
}

impl RecordingInstaller {
    fn succeeds() -> Self {
        Self {
            calls: Cell::new(0),
            order: RefCell::new(Vec::new()),
            fail_on_call: None,
            error: PortableInstallError::AtomicInstallUnsupported,
        }
    }

    fn fails_on(call: usize, error: PortableInstallError) -> Self {
        Self {
            fail_on_call: Some(call),
            error,
            ..Self::succeeds()
        }
    }
}

impl PublicationObjectInstaller for RecordingInstaller {
    fn install_exact_bytes(
        &self,
        bytes: &[u8],
    ) -> Result<PortableObjectInstallation, PortableInstallError> {
        let call = self.calls.get();
        self.calls.set(call + 1);
        self.order.borrow_mut().push(bytes[..4].try_into().unwrap());
        if self.fail_on_call == Some(call) {
            return Err(self.error);
        }
        let digest = crate::portable_emitter::sha256(bytes);
        let byte_length = ExactByteLength::new(u64::try_from(bytes.len()).unwrap());
        Ok(PortableObjectInstallation::test_only(
            digest,
            byte_length,
            crate::portable_emitter::object_key(digest),
        ))
    }
}

fn descriptor_view(bytes: &[u8]) -> RegistryCheckedObjectView<'_> {
    preflight_object_values_v1(
        bytes,
        PortableObjectKind::CanonicalPublicationDescriptor,
        FormatLimits::V1_HARD,
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

fn checked_bundle<'a>(
    candidate: &'a PortablePublicationCandidate,
) -> laneflow_format::PostEmissionCheckedBundleV1<'a> {
    check_post_emission_bundle_v1(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::V1_HARD,
    )
    .unwrap()
}

fn field_range(bytes: &[u8], kind: PortableObjectKind, section: u32, tag: u16) -> Range<usize> {
    let value = preflight_object_values_v1(bytes, kind, FormatLimits::V1_HARD)
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
        ExpectedSemanticDiffBaseV1::Artifact { .. }
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
        check_post_emission_bundle_v1(
            &lfca,
            candidate.source_map().bytes(),
            candidate.semantic_diff().bytes(),
            candidate.expected_semantic_diff_base(),
            FormatLimits::V1_HARD,
        )
        .unwrap_err(),
        PostEmissionCheckError::NetworkRevisionMismatch
    );

    let mut lfsm = candidate.source_map().bytes().to_vec();
    mutate_field(&mut lfsm, PortableObjectKind::SourceMap, 0, 4);
    assert_eq!(
        check_post_emission_bundle_v1(
            candidate.canonical_artifact().bytes(),
            &lfsm,
            candidate.semantic_diff().bytes(),
            candidate.expected_semantic_diff_base(),
            FormatLimits::V1_HARD,
        )
        .unwrap_err(),
        PostEmissionCheckError::SourceMapBindingMismatch
    );

    let mut target_lfsd = candidate.semantic_diff().bytes().to_vec();
    mutate_field(&mut target_lfsd, PortableObjectKind::SemanticDiff, 0, 8);
    assert_eq!(
        check_post_emission_bundle_v1(
            candidate.canonical_artifact().bytes(),
            candidate.source_map().bytes(),
            &target_lfsd,
            candidate.expected_semantic_diff_base(),
            FormatLimits::V1_HARD,
        )
        .unwrap_err(),
        PostEmissionCheckError::SemanticDiffTargetBindingMismatch
    );

    assert_eq!(
        check_post_emission_bundle_v1(
            candidate.canonical_artifact().bytes(),
            candidate.source_map().bytes(),
            candidate.semantic_diff().bytes(),
            ExpectedSemanticDiffBaseV1::Artifact {
                network_revision_derivation_version: 1,
                network_revision: candidate.network_revision(),
                digest: candidate.canonical_artifact().digest(),
                byte_length: candidate.canonical_artifact().byte_length(),
            },
            FormatLimits::V1_HARD,
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
                check_post_emission_bundle_v1(
                    objects[0],
                    objects[1],
                    objects[2],
                    candidate.expected_semantic_diff_base(),
                    FormatLimits::V1_HARD,
                ),
                Err(PostEmissionCheckError::Format(_))
            ));
        }
    }

    assert!(matches!(
        check_post_emission_bundle_v1(
            candidate.source_map().bytes(),
            candidate.canonical_artifact().bytes(),
            candidate.semantic_diff().bytes(),
            candidate.expected_semantic_diff_base(),
            FormatLimits::V1_HARD,
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
            check_post_emission_bundle_v1(
                genesis.canonical_artifact().bytes(),
                &lfsm,
                genesis.semantic_diff().bytes(),
                genesis.expected_semantic_diff_base(),
                FormatLimits::V1_HARD,
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
            check_post_emission_bundle_v1(
                artifact.canonical_artifact().bytes(),
                artifact.source_map().bytes(),
                &lfsd,
                artifact.expected_semantic_diff_base(),
                FormatLimits::V1_HARD,
            )
            .unwrap_err(),
            PostEmissionCheckError::SemanticDiffBaseBindingMismatch
        );
    }
    for tag in [7, 8, 9] {
        let mut lfsd = artifact.semantic_diff().bytes().to_vec();
        mutate_field(&mut lfsd, PortableObjectKind::SemanticDiff, 0, tag);
        assert_eq!(
            check_post_emission_bundle_v1(
                artifact.canonical_artifact().bytes(),
                artifact.source_map().bytes(),
                &lfsd,
                artifact.expected_semantic_diff_base(),
                FormatLimits::V1_HARD,
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
            check_post_emission_bundle_v1(
                genesis.canonical_artifact().bytes(),
                &lfsm,
                genesis.semantic_diff().bytes(),
                genesis.expected_semantic_diff_base(),
                FormatLimits::V1_HARD,
            ),
            Err(PostEmissionCheckError::Format(_))
        ));
    }

    let artifact = portable_fixture_tests::full_spatial_portable_artifact_base_fixture_candidate();
    for tag in [2, 6] {
        let mut lfsd = artifact.semantic_diff().bytes().to_vec();
        mutate_field(&mut lfsd, PortableObjectKind::SemanticDiff, 0, tag);
        assert!(matches!(
            check_post_emission_bundle_v1(
                artifact.canonical_artifact().bytes(),
                artifact.source_map().bytes(),
                &lfsd,
                artifact.expected_semantic_diff_base(),
                FormatLimits::V1_HARD,
            ),
            Err(PostEmissionCheckError::Format(_))
        ));
    }
}

#[test]
fn lfcp_v2_contains_only_artifact_source_and_publication_bindings() {
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    for (kind, code) in [
        (PortablePublisherKindV2::LocalTool, 0_u8),
        (PortablePublisherKindV2::Ci, 1),
        (PortablePublisherKindV2::ReleaseService, 2),
    ] {
        let descriptor = build_lfcp_v2(
            checked_bundle(&candidate),
            &PortablePublicationProvenanceV2::new(
                kind,
                "laneflow-publisher-fixture-v2",
                Some("controlled-build".into()),
                Some("2026-08-18T00:00:00Z".into()),
            ),
            FormatLimits::V1_HARD,
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
fn publication_checks_before_installing_and_commits_once_in_order() {
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    let installer = RecordingInstaller::succeeds();
    let mut manifest = RecordingManifest::default();

    let committed = commit_with_installer(
        &installer,
        &candidate,
        &provenance(PortablePublisherKindV2::LocalTool),
        FormatLimits::V1_HARD,
        &mut manifest,
    )
    .unwrap();

    assert_eq!(
        installer.order.into_inner(),
        [*b"LFCA", *b"LFSM", *b"LFSD", *b"LFCP"]
    );
    assert_eq!(manifest.calls, 1);
    assert_eq!(
        committed.descriptor().bytes(),
        manifest.descriptor_bytes.as_deref().unwrap()
    );
    assert_eq!(committed.descriptor().bytes()[4..6], 2_u16.to_le_bytes());
}

#[test]
fn checker_limit_failure_has_zero_installer_and_manifest_side_effects() {
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    let largest = [
        candidate.canonical_artifact().byte_length().get(),
        candidate.source_map().byte_length().get(),
        candidate.semantic_diff().byte_length().get(),
    ]
    .into_iter()
    .max()
    .unwrap();
    let mut config = FormatLimitConfig::V1_HARD;
    config.max_object_bytes = largest - 1;
    let installer = RecordingInstaller::succeeds();
    let mut manifest = RecordingManifest::default();

    assert!(matches!(
        commit_with_installer(
            &installer,
            &candidate,
            &provenance(PortablePublisherKindV2::LocalTool),
            FormatLimits::try_new(config).unwrap(),
            &mut manifest,
        ),
        Err(PortablePublicationError::PostEmission(
            PostEmissionCheckError::LimitExceeded {
                dimension: LimitDimension::ObjectBytes,
                ..
            }
        ))
    ));
    assert_eq!(installer.calls.get(), 0);
    assert_eq!(manifest.calls, 0);
}

#[test]
fn checker_closes_exact_candidate_staging_boundary_before_side_effects() {
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    let total = candidate
        .canonical_artifact()
        .byte_length()
        .get()
        .checked_add(candidate.source_map().byte_length().get())
        .and_then(|value| value.checked_add(candidate.semantic_diff().byte_length().get()))
        .unwrap();

    let mut exact_config = FormatLimitConfig::V1_HARD;
    exact_config.max_candidate_staging_bytes = total;
    check_post_emission_bundle_v1(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::try_new(exact_config).unwrap(),
    )
    .unwrap();

    let mut rejected_config = exact_config;
    rejected_config.max_candidate_staging_bytes = total - 1;
    let installer = RecordingInstaller::succeeds();
    let mut manifest = RecordingManifest::default();
    assert_eq!(
        commit_with_installer(
            &installer,
            &candidate,
            &provenance(PortablePublisherKindV2::LocalTool),
            FormatLimits::try_new(rejected_config).unwrap(),
            &mut manifest,
        ),
        Err(PortablePublicationError::PostEmission(
            PostEmissionCheckError::LimitExceeded {
                dimension: LimitDimension::CandidateStagingBytes,
                actual: total,
                limit: total - 1,
            }
        ))
    );
    assert_eq!(installer.calls.get(), 0);
    assert_eq!(manifest.calls, 0);
}

#[test]
fn each_object_install_failure_prevents_manifest_commit() {
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    let error = PortableInstallError::Io {
        operation: PortableInstallOperation::WriteStagingFile,
        kind: std::io::ErrorKind::WriteZero,
    };
    let expected = [*b"LFCA", *b"LFSM", *b"LFSD", *b"LFCP"];

    for fail_on in 0..4 {
        let installer = RecordingInstaller::fails_on(fail_on, error);
        let mut manifest = RecordingManifest::default();
        assert_eq!(
            commit_with_installer(
                &installer,
                &candidate,
                &provenance(PortablePublisherKindV2::LocalTool),
                FormatLimits::V1_HARD,
                &mut manifest,
            ),
            Err(PortablePublicationError::Install(error))
        );
        assert_eq!(installer.order.into_inner(), expected[..=fail_on]);
        assert_eq!(manifest.calls, 0);
    }
}

#[test]
fn manifest_failure_returns_no_committed_capability() {
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    let installer = RecordingInstaller::succeeds();
    let mut manifest = RecordingManifest {
        failure: Some(PortableManifestCommitError::Rejected),
        ..RecordingManifest::default()
    };

    assert_eq!(
        commit_with_installer(
            &installer,
            &candidate,
            &provenance(PortablePublisherKindV2::Ci),
            FormatLimits::V1_HARD,
            &mut manifest,
        ),
        Err(PortablePublicationError::Manifest(
            PortableManifestCommitError::Rejected
        ))
    );
    assert_eq!(
        installer.order.into_inner(),
        [*b"LFCA", *b"LFSM", *b"LFSD", *b"LFCP"]
    );
    assert_eq!(manifest.calls, 1);
    assert!(manifest.descriptor_key.is_some());
}

#[test]
fn lfcp_v2_exact_bytes_are_deterministic() {
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    let checked = checked_bundle(&candidate);
    let provenance = PortablePublicationProvenanceV2::new(
        PortablePublisherKindV2::ReleaseService,
        "laneflow-publisher-fixture-v2",
        Some("controlled-build".into()),
        Some("2026-08-18T00:00:00Z".into()),
    );
    let first = build_lfcp_v2(checked, &provenance, FormatLimits::V1_HARD).unwrap();
    let second = build_lfcp_v2(checked, &provenance, FormatLimits::V1_HARD).unwrap();
    let expected_hex =
        include_str!("../../tests/fixtures/portable-v2/lfcp-v2-min-bindings/expected.lfcp.hex");
    let digits = expected_hex
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    let expected = digits
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16).unwrap();
            let low = (pair[1] as char).to_digit(16).unwrap();
            u8::try_from((high << 4) | low).unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(first.bytes(), expected);
    assert_eq!(first.byte_length(), ExactByteLength::new(812));
    assert_eq!(
        first.object_key(),
        "sha256/54f6ffd55c7f08f20a2f04bf273bb1b98e96ad38155f7bd027136a347ab3e763"
    );
    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.digest(), second.digest());
}
