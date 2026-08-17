use std::{
    cell::{Cell, RefCell},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use laneflow_format::{
    FormatLimitConfig, FormatLimits, RegistryCheckedObjectView, preflight_object_values_v1,
};
use laneflow_static_contract::PortableObjectKind;

use super::*;
use crate::{PortableInstallOperation, compiler::portable_fixture_tests};

const RECEIPT_BYTES: &[u8] = b"test-only opaque #299 receipt bytes for LFCP binding v1";
static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new(name: &str) -> Self {
        let ordinal = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "laneflow-portable-publication-{name}-{:08x}-{ordinal:016x}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone)]
struct TestReceipt {
    bytes: Box<[u8]>,
    format_version: u16,
    kind: Box<str>,
    validator_build_id: Box<str>,
    subject_count: u32,
    artifact: Option<PortableArtifactSubjectBindingV1>,
    source_map: Option<PortableSourceMapSubjectBindingV1>,
}

impl TestReceipt {
    fn valid(candidate: &PortablePublicationCandidate) -> Self {
        Self {
            bytes: RECEIPT_BYTES.into(),
            format_version: 1,
            kind: "canonical-publication-v1".into(),
            validator_build_id: "laneflow-validator-fixture-v1".into(),
            subject_count: 2,
            artifact: Some(PortableArtifactSubjectBindingV1 {
                canonical_artifact_format_version: 1,
                network_revision_derivation_version: 1,
                network_revision: candidate.network_revision(),
                digest: candidate.canonical_artifact().digest(),
                byte_length: candidate.canonical_artifact().byte_length(),
            }),
            source_map: Some(PortableSourceMapSubjectBindingV1 {
                source_map_format_version: 1,
                digest: candidate.source_map().digest(),
                byte_length: candidate.source_map().byte_length(),
            }),
        }
    }
}

impl CanonicalPublicationReceiptViewV1 for TestReceipt {
    fn exact_bytes(&self) -> &[u8] {
        &self.bytes
    }

    fn validation_receipt_format_version(&self) -> u16 {
        self.format_version
    }

    fn receipt_kind(&self) -> &str {
        &self.kind
    }

    fn validator_build_id(&self) -> &str {
        &self.validator_build_id
    }

    fn subject_count(&self) -> u32 {
        self.subject_count
    }

    fn canonical_artifact_subject(&self) -> Option<PortableArtifactSubjectBindingV1> {
        self.artifact
    }

    fn source_map_subject(&self) -> Option<PortableSourceMapSubjectBindingV1> {
        self.source_map
    }
}

struct AlternatingReceipt {
    inner: TestReceipt,
    exact_calls: Cell<u32>,
}

impl CanonicalPublicationReceiptViewV1 for AlternatingReceipt {
    fn exact_bytes(&self) -> &[u8] {
        let call = self.exact_calls.get();
        self.exact_calls.set(call + 1);
        if call == 0 {
            self.inner.exact_bytes()
        } else {
            b"different bytes returned by an unstable trait implementation"
        }
    }

    fn validation_receipt_format_version(&self) -> u16 {
        self.inner.validation_receipt_format_version()
    }

    fn receipt_kind(&self) -> &str {
        self.inner.receipt_kind()
    }

    fn validator_build_id(&self) -> &str {
        self.inner.validator_build_id()
    }

    fn subject_count(&self) -> u32 {
        self.inner.subject_count()
    }

    fn canonical_artifact_subject(&self) -> Option<PortableArtifactSubjectBindingV1> {
        self.inner.canonical_artifact_subject()
    }

    fn source_map_subject(&self) -> Option<PortableSourceMapSubjectBindingV1> {
        self.inner.source_map_subject()
    }
}

fn provenance() -> PortablePublicationProvenanceV1 {
    PortablePublicationProvenanceV1::new(
        PortablePublisherKindV1::LocalTool,
        "laneflow-publisher-fixture-v1",
        None,
        None,
    )
}

struct RecordingManifest {
    calls: usize,
    failure: Option<PortableManifestCommitError>,
    descriptor_bytes: Option<Box<[u8]>>,
    descriptor_key: Option<Box<str>>,
}

impl RecordingManifest {
    fn succeeds() -> Self {
        Self {
            calls: 0,
            failure: None,
            descriptor_bytes: None,
            descriptor_key: None,
        }
    }
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
            candidate.descriptor().object_key(),
            candidate.descriptor_installation().object_key()
        );
        assert_eq!(
            candidate.canonical_artifact_installation().object_key(),
            field_utf8(candidate.descriptor().bytes(), 3, 3)
        );
        assert_eq!(
            candidate.source_map_installation().object_key(),
            field_utf8(candidate.descriptor().bytes(), 3, 4)
        );
        assert_eq!(
            candidate.receipt_installation().object_key(),
            field_utf8(candidate.descriptor().bytes(), 3, 5)
        );
        self.descriptor_bytes = Some(candidate.descriptor().bytes().into());
        self.descriptor_key = Some(candidate.descriptor().object_key().into());
        match self.failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
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

fn read_installed(store: &PortableObjectStore, object_key: &str) -> Vec<u8> {
    fs::read(store.object_path(object_key).unwrap()).unwrap()
}

#[test]
fn success_installs_all_objects_then_commits_exactly_one_manifest() {
    let root = TestRoot::new("success");
    let store = PortableObjectStore::try_open(root.path()).unwrap();
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    let receipt = TestReceipt::valid(&candidate);
    let mut manifest = RecordingManifest::succeeds();

    let committed = commit_portable_publication_v1(
        &store,
        &candidate,
        &receipt,
        &provenance(),
        FormatLimits::V1_HARD,
        &mut manifest,
    )
    .unwrap();

    assert_eq!(manifest.calls, 1);
    assert_eq!(
        committed.descriptor().bytes(),
        manifest.descriptor_bytes.as_deref().unwrap()
    );
    assert_eq!(&committed.descriptor().bytes()[..4], b"LFCP");
    assert_eq!(
        read_installed(&store, candidate.canonical_artifact().object_key()),
        candidate.canonical_artifact().bytes()
    );
    assert_eq!(
        read_installed(&store, candidate.source_map().object_key()),
        candidate.source_map().bytes()
    );
    assert_eq!(
        read_installed(&store, candidate.semantic_diff().object_key()),
        candidate.semantic_diff().bytes()
    );
    assert_eq!(
        read_installed(&store, committed.receipt_installation().object_key()),
        RECEIPT_BYTES
    );
    assert_eq!(
        read_installed(&store, committed.descriptor().object_key()),
        committed.descriptor().bytes()
    );

    let descriptor = committed.descriptor().bytes();
    assert_eq!(
        field_bytes(descriptor, 0, 3),
        candidate.network_revision().as_digest().as_bytes()
    );
    assert_eq!(
        field_bytes(descriptor, 0, 4),
        candidate.canonical_artifact().digest().as_bytes()
    );
    assert_eq!(
        field_bytes(descriptor, 1, 2),
        candidate.source_map().digest().as_bytes()
    );
    assert_eq!(field_utf8(descriptor, 2, 2), "canonical-publication-v1");
    assert_eq!(
        field_utf8(descriptor, 2, 3),
        "laneflow-validator-fixture-v1"
    );
    assert_eq!(
        field_utf8(descriptor, 3, 3),
        candidate.canonical_artifact().object_key()
    );
    assert_eq!(
        field_utf8(descriptor, 3, 4),
        candidate.source_map().object_key()
    );
    assert_eq!(
        field_utf8(descriptor, 3, 5),
        committed.receipt_installation().object_key()
    );
    let expected = decode_hex(include_str!(
        "../../tests/fixtures/portable-v1/lfcp-v1-min-bindings/expected.lfcp.hex"
    ));
    assert_eq!(committed.descriptor().bytes(), expected.as_ref());
    assert_eq!(
        committed.descriptor().byte_length(),
        ExactByteLength::new(1_050)
    );
    assert_eq!(
        committed.descriptor().object_key(),
        "sha256/7cbe21a42bca1f50f30e34de91db310e8d550e64f87a761cd1bec516010c4e05"
    );
}

#[test]
fn publisher_kind_and_optional_controlled_provenance_are_exact_inputs() {
    let root = TestRoot::new("publisher-provenance");
    let store = PortableObjectStore::try_open(root.path()).unwrap();
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    let receipt = TestReceipt::valid(&candidate);
    for (kind, code) in [
        (PortablePublisherKindV1::LocalTool, 0_u8),
        (PortablePublisherKindV1::Ci, 1),
        (PortablePublisherKindV1::ReleaseService, 2),
    ] {
        let provenance = PortablePublicationProvenanceV1::new(
            kind,
            "publisher-build",
            Some("ci-run-42".into()),
            Some("2026-08-17T00:00:00Z".into()),
        );
        let mut manifest = RecordingManifest::succeeds();
        let committed = commit_portable_publication_v1(
            &store,
            &candidate,
            &receipt,
            &provenance,
            FormatLimits::V1_HARD,
            &mut manifest,
        )
        .unwrap();
        assert_eq!(manifest.calls, 1);
        let descriptor = committed.descriptor().bytes();
        assert_eq!(field_bytes(descriptor, 3, 1), [code]);
        assert_eq!(field_utf8(descriptor, 3, 2), "publisher-build");
        assert_eq!(field_utf8(descriptor, 3, 6), "ci-run-42");
        assert_eq!(field_utf8(descriptor, 3, 7), "2026-08-17T00:00:00Z");
    }
}

#[test]
fn receipt_view_is_snapshotted_once_before_any_installation() {
    let root = TestRoot::new("receipt-snapshot");
    let store = PortableObjectStore::try_open(root.path()).unwrap();
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    let receipt = AlternatingReceipt {
        inner: TestReceipt::valid(&candidate),
        exact_calls: Cell::new(0),
    };
    let mut manifest = RecordingManifest::succeeds();

    let committed = commit_portable_publication_v1(
        &store,
        &candidate,
        &receipt,
        &provenance(),
        FormatLimits::V1_HARD,
        &mut manifest,
    )
    .unwrap();

    assert_eq!(receipt.exact_calls.get(), 1);
    assert_eq!(
        read_installed(&store, committed.receipt_installation().object_key()),
        RECEIPT_BYTES
    );
}

fn decode_hex(input: &str) -> Box<[u8]> {
    let mut digits = Vec::new();
    for byte in input.bytes() {
        match byte {
            b'0'..=b'9' | b'a'..=b'f' => digits.push(byte),
            b' ' | b'\t' | b'\r' | b'\n' => {}
            _ => panic!("expected lowercase hex fixture"),
        }
    }
    assert_eq!(digits.len() % 2, 0);
    digits
        .chunks_exact(2)
        .map(|pair| (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]))
        .collect()
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => unreachable!("validated lowercase ASCII hex"),
    }
}

#[test]
fn receipt_metadata_shape_and_every_subject_binding_fail_before_install() {
    let root = TestRoot::new("receipt-negative");
    let store = PortableObjectStore::try_open(root.path()).unwrap();
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    let valid = TestReceipt::valid(&candidate);

    let mut limit_config = FormatLimitConfig::V1_HARD;
    limit_config.max_object_bytes = RECEIPT_BYTES.len() as u64 - 1;
    let reduced_limits = FormatLimits::try_new(limit_config).unwrap();
    let mut manifest = RecordingManifest::succeeds();
    assert_eq!(
        commit_portable_publication_v1(
            &store,
            &candidate,
            &valid,
            &provenance(),
            reduced_limits,
            &mut manifest,
        ),
        Err(PortablePublicationError::ReceiptLimitExceeded {
            actual: RECEIPT_BYTES.len() as u64,
            limit: RECEIPT_BYTES.len() as u64 - 1,
        })
    );
    assert_eq!(manifest.calls, 0);

    let mut invalid = Vec::new();
    let mut receipt = valid.clone();
    receipt.format_version = 2;
    invalid.push((
        receipt,
        PortablePublicationError::InvalidReceiptFormatVersion,
    ));
    let mut receipt = valid.clone();
    receipt.kind = "another-kind".into();
    invalid.push((receipt, PortablePublicationError::InvalidReceiptKind));
    let mut receipt = valid.clone();
    receipt.bytes = Box::new([]);
    invalid.push((receipt, PortablePublicationError::EmptyReceipt));
    for count in [0, 1, 3] {
        let mut receipt = valid.clone();
        receipt.subject_count = count;
        invalid.push((
            receipt,
            PortablePublicationError::ReceiptSubjectShapeMismatch,
        ));
    }
    let mut receipt = valid.clone();
    receipt.artifact = None;
    invalid.push((
        receipt,
        PortablePublicationError::ReceiptSubjectShapeMismatch,
    ));
    let mut receipt = valid.clone();
    receipt.source_map = None;
    invalid.push((
        receipt,
        PortablePublicationError::ReceiptSubjectShapeMismatch,
    ));

    let artifact_mutations = [
        PortableArtifactSubjectBindingV1 {
            canonical_artifact_format_version: 2,
            ..valid.artifact.unwrap()
        },
        PortableArtifactSubjectBindingV1 {
            network_revision_derivation_version: 2,
            ..valid.artifact.unwrap()
        },
        PortableArtifactSubjectBindingV1 {
            network_revision: NetworkRevisionId::from_digest(Sha256Digest::from_bytes([7; 32])),
            ..valid.artifact.unwrap()
        },
        PortableArtifactSubjectBindingV1 {
            digest: Sha256Digest::from_bytes([8; 32]),
            ..valid.artifact.unwrap()
        },
        PortableArtifactSubjectBindingV1 {
            byte_length: ExactByteLength::new(valid.artifact.unwrap().byte_length.get() + 1),
            ..valid.artifact.unwrap()
        },
    ];
    for artifact in artifact_mutations {
        let mut receipt = valid.clone();
        receipt.artifact = Some(artifact);
        invalid.push((
            receipt,
            PortablePublicationError::ReceiptSubjectBindingMismatch,
        ));
    }
    let source_map_mutations = [
        PortableSourceMapSubjectBindingV1 {
            source_map_format_version: 2,
            ..valid.source_map.unwrap()
        },
        PortableSourceMapSubjectBindingV1 {
            digest: Sha256Digest::from_bytes([9; 32]),
            ..valid.source_map.unwrap()
        },
        PortableSourceMapSubjectBindingV1 {
            byte_length: ExactByteLength::new(valid.source_map.unwrap().byte_length.get() + 1),
            ..valid.source_map.unwrap()
        },
    ];
    for source_map in source_map_mutations {
        let mut receipt = valid.clone();
        receipt.source_map = Some(source_map);
        invalid.push((
            receipt,
            PortablePublicationError::ReceiptSubjectBindingMismatch,
        ));
    }

    for (receipt, expected) in invalid {
        let mut manifest = RecordingManifest::succeeds();
        assert_eq!(
            commit_portable_publication_v1(
                &store,
                &candidate,
                &receipt,
                &provenance(),
                FormatLimits::V1_HARD,
                &mut manifest,
            ),
            Err(expected)
        );
        assert_eq!(manifest.calls, 0);
        assert!(
            !store
                .object_path(candidate.canonical_artifact().object_key())
                .unwrap()
                .exists()
        );
    }
}

struct FaultingInstaller<'a> {
    store: &'a PortableObjectStore,
    fail_on_call: Option<usize>,
    error: PortableInstallError,
    calls: Cell<usize>,
    order: RefCell<Vec<&'static str>>,
}

impl<'a> FaultingInstaller<'a> {
    fn new(
        store: &'a PortableObjectStore,
        fail_on_call: Option<usize>,
        error: PortableInstallError,
    ) -> Self {
        Self {
            store,
            fail_on_call,
            error,
            calls: Cell::new(0),
            order: RefCell::new(Vec::new()),
        }
    }

    fn before(&self, label: &'static str) -> Result<(), PortableInstallError> {
        let call = self.calls.get();
        self.calls.set(call + 1);
        self.order.borrow_mut().push(label);
        if self.fail_on_call == Some(call) {
            Err(self.error)
        } else {
            Ok(())
        }
    }
}

impl PublicationObjectInstaller for FaultingInstaller<'_> {
    fn install_candidate(
        &self,
        candidate: &PortableObjectCandidate,
    ) -> Result<PortableObjectInstallation, PortableInstallError> {
        let label = match &candidate.bytes()[..4] {
            b"LFCA" => "LFCA",
            b"LFSM" => "LFSM",
            b"LFSD" => "LFSD",
            b"LFCP" => "LFCP",
            _ => panic!("unexpected candidate kind"),
        };
        self.before(label)?;
        self.store.install_candidate(candidate)
    }

    fn install_exact_bytes(
        &self,
        bytes: &[u8],
    ) -> Result<PortableObjectInstallation, PortableInstallError> {
        assert_eq!(bytes, RECEIPT_BYTES);
        self.before("receipt")?;
        self.store.install_exact_bytes(bytes)
    }
}

fn run_install_failure(
    name: &str,
    fail_on_call: usize,
    error: PortableInstallError,
) -> (Vec<&'static str>, usize) {
    let root = TestRoot::new(name);
    let store = PortableObjectStore::try_open(root.path()).unwrap();
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    let receipt = TestReceipt::valid(&candidate);
    let installer = FaultingInstaller::new(&store, Some(fail_on_call), error);
    let mut manifest = RecordingManifest::succeeds();
    assert_eq!(
        commit_with_installer(
            &installer,
            &candidate,
            &receipt,
            &provenance(),
            FormatLimits::V1_HARD,
            &mut manifest,
        ),
        Err(PortablePublicationError::Install(error))
    );
    (installer.order.into_inner(), manifest.calls)
}

#[test]
fn artifact_source_map_diff_and_lfcp_failures_expose_no_partial_success() {
    let cases = [
        (
            "artifact-write",
            0,
            PortableInstallError::Io {
                operation: PortableInstallOperation::WriteStagingFile,
                kind: io::ErrorKind::WriteZero,
            },
            vec!["LFCA"],
        ),
        (
            "source-map-flush",
            1,
            PortableInstallError::Io {
                operation: PortableInstallOperation::FlushStagingFile,
                kind: io::ErrorKind::Other,
            },
            vec!["LFCA", "LFSM"],
        ),
        (
            "source-map-close",
            1,
            PortableInstallError::Io {
                operation: PortableInstallOperation::CloseStagingFile,
                kind: io::ErrorKind::Other,
            },
            vec!["LFCA", "LFSM"],
        ),
        (
            "semantic-diff-install",
            2,
            PortableInstallError::AtomicInstallUnsupported,
            vec!["LFCA", "LFSM", "LFSD"],
        ),
        (
            "lfcp-install",
            4,
            PortableInstallError::ExistingObjectMismatch,
            vec!["LFCA", "LFSM", "LFSD", "receipt", "LFCP"],
        ),
    ];
    for (name, call, error, expected_order) in cases {
        let (order, manifest_calls) = run_install_failure(name, call, error);
        assert_eq!(order, expected_order);
        assert_eq!(manifest_calls, 0);
    }
}

#[test]
fn every_receipt_storage_failure_prevents_lfcp_and_manifest() {
    let failures = [
        PortableInstallError::Io {
            operation: PortableInstallOperation::CreateStagingFile,
            kind: io::ErrorKind::PermissionDenied,
        },
        PortableInstallError::Io {
            operation: PortableInstallOperation::WriteStagingFile,
            kind: io::ErrorKind::WriteZero,
        },
        PortableInstallError::Io {
            operation: PortableInstallOperation::FlushStagingFile,
            kind: io::ErrorKind::Other,
        },
        PortableInstallError::Io {
            operation: PortableInstallOperation::CloseStagingFile,
            kind: io::ErrorKind::Other,
        },
        PortableInstallError::StagedObjectMismatch,
        PortableInstallError::AtomicInstallUnsupported,
        PortableInstallError::ExistingObjectMismatch,
        PortableInstallError::Io {
            operation: PortableInstallOperation::SyncObjectDirectory,
            kind: io::ErrorKind::Other,
        },
    ];
    for (index, error) in failures.into_iter().enumerate() {
        let name = format!("receipt-failure-{index}");
        let (order, manifest_calls) = run_install_failure(&name, 3, error);
        assert_eq!(order, ["LFCA", "LFSM", "LFSD", "receipt"]);
        assert_eq!(manifest_calls, 0);
    }
}

#[test]
fn manifest_failure_keeps_complete_objects_unreferenced_without_commit_capability() {
    let root = TestRoot::new("manifest-failure");
    let store = PortableObjectStore::try_open(root.path()).unwrap();
    let candidate = portable_fixture_tests::full_spatial_portable_fixture_candidate();
    let receipt = TestReceipt::valid(&candidate);
    let mut manifest = RecordingManifest {
        failure: Some(PortableManifestCommitError::Rejected),
        ..RecordingManifest::succeeds()
    };

    assert_eq!(
        commit_portable_publication_v1(
            &store,
            &candidate,
            &receipt,
            &provenance(),
            FormatLimits::V1_HARD,
            &mut manifest,
        ),
        Err(PortablePublicationError::Manifest(
            PortableManifestCommitError::Rejected
        ))
    );
    assert_eq!(manifest.calls, 1);
    let descriptor_key = manifest.descriptor_key.unwrap();
    let descriptor_bytes = manifest.descriptor_bytes.unwrap();
    assert_eq!(
        read_installed(&store, &descriptor_key).as_slice(),
        descriptor_bytes.as_ref()
    );
    assert_eq!(
        read_installed(&store, candidate.canonical_artifact().object_key()),
        candidate.canonical_artifact().bytes()
    );
}
