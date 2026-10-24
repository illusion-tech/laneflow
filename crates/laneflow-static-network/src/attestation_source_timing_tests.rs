//! #963 发布来源时序矩阵；读取视图独立拥有，不制造不可变 backing 的可变别名。

use super::*;
use crate::attestation::{AttestationStage, TraceStep};
use crate::{BuildError, BuildStructure};
use laneflow_format::{preflight_object_framing, preflight_object_values};
use laneflow_static_contract::{
    CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH, NETWORK_REVISION_DOMAIN_PREFIX, PortableObjectKind,
    TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH,
};
use sha2::{Digest, Sha256};
use test_injection::BuilderRead;

pub(super) struct InjectionGuard;

impl Drop for InjectionGuard {
    fn drop(&mut self) {
        test_injection::reset();
        reset_allocation_failure_injection();
    }
}

#[derive(Default)]
struct TraceSink(Vec<TraceStep>);

impl AttestationTelemetry for TraceSink {
    fn emit(&mut self, event: AttestationEvent<'_>) -> Result<(), TelemetryDeliveryError> {
        if let AttestationEvent::Trace(step) = event {
            self.0.push(step);
        }
        Ok(())
    }
}

pub(super) fn observed(
    root: &SharedNetworkRevision,
    bytes: &[u8],
    expectation: RootAttestationExpectation,
) -> (AttestationOutcome, Vec<TraceStep>) {
    let mut sink = TraceSink::default();
    let outcome = attest_shared_network_revision(
        root,
        bytes,
        expectation,
        FormatLimits::HARD,
        SharedNetworkBuildOptions::new(SpatialBuildOption::Omit, BUILD_LIMITS),
        Some(&mut sink),
    );
    (outcome, sink.0)
}

fn field_offset(bytes: &[u8], section: u32, table: u32, tag: u16) -> usize {
    let view = preflight_object_values(
        bytes,
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap();
    let field = view
        .registry_view()
        .section(section)
        .unwrap()
        .table(table)
        .unwrap()
        .row(0)
        .unwrap()
        .field_by_tag(tag)
        .unwrap();
    field.value_bytes().as_ptr().addr() - bytes.as_ptr().addr()
}

pub(super) fn changed_speed(delta: u32) -> Vec<u8> {
    let mut bytes = FULL_SPATIAL.to_vec();
    let offset = field_offset(&bytes, 2, 3, 4);
    let speed = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    bytes[offset..offset + 4].copy_from_slice(&(speed - delta).to_le_bytes());
    refresh_chunk_digests(&mut bytes);
    // 改写发生于 reader 之后：保留原修订声明，builder 的阶段视图只做直接值域预检。
    assert!(check_canonical_network_input(bytes.as_slice(), FormatLimits::HARD).is_err());
    bytes
}

// 测试变体只改变语义字段；重登记 chunk 摘要，避免 framing 拒绝掩盖要验证的后续阶段。
fn refresh_chunk_digests(bytes: &mut [u8]) {
    let framing = preflight_object_framing(
        bytes,
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap();
    let mut chunks = Vec::new();
    for ordinal in 0..framing.section_count() {
        let section = framing.section(ordinal).unwrap().bytes();
        let base = section.as_ptr().addr() - bytes.as_ptr().addr();
        let count = u32::from_le_bytes(section[..4].try_into().unwrap());
        for chunk in 0..count {
            let entry = CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH as usize
                + chunk as usize * TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH as usize;
            let offset = usize::try_from(u64::from_le_bytes(
                section[entry + 24..entry + 32].try_into().unwrap(),
            ))
            .unwrap();
            let length = usize::try_from(u64::from_le_bytes(
                section[entry + 32..entry + 40].try_into().unwrap(),
            ))
            .unwrap();
            chunks.push((base + entry + 40, base + offset, length));
        }
    }
    for (digest, offset, length) in chunks {
        let hash = Sha256::digest(&bytes[offset..offset + length]);
        bytes[digest..digest + 32].copy_from_slice(&hash);
    }
}

fn changed_evidence(bytes: &[u8]) -> MismatchEvidence {
    MismatchEvidence {
        stage: AttestationStage::SourcePreparation,
        site: MismatchSite::SourceFailure {
            detail: SourceFailureDetail::SourceBytesChanged {
                prepared_digest: Sha256Digest::from_bytes(Sha256::digest(FULL_SPATIAL).into()),
                prepared_byte_length: FULL_SPATIAL.len() as u64,
                recheck_digest: Sha256Digest::from_bytes(Sha256::digest(bytes).into()),
                recheck_byte_length: bytes.len() as u64,
            },
        },
    }
}

pub(super) fn occurrences(trace: &[TraceStep], step: TraceStep) -> usize {
    trace.iter().filter(|actual| **actual == step).count()
}

#[test]
fn repeated_post_validation_source_change_is_source_mismatch() {
    for length_changed in [false, true] {
        let _guard = InjectionGuard;
        let root = build(FULL_SPATIAL, SpatialBuildOption::Omit);
        let changed = changed_speed(1);
        test_injection::builder_reads(vec![Some(BuilderRead::Bytes(changed.clone())); 2]);
        let mut rechecked = changed;
        if length_changed {
            // 构建之后来源继续漂移；复核须保留新长度，不能只留下摘要差异。
            rechecked.push(0);
        }
        test_injection::recheck_sources(vec![rechecked.clone(); 2]);
        let (outcome, trace) = observed(
            &root,
            FULL_SPATIAL,
            expectation_from(root.canonical_origin()),
        );
        let report = concluded(outcome);
        assert_eq!(report.verdict, AttestationVerdict::Mismatch);
        assert_eq!(report.cause, Some(AttestationCause::Source));
        assert_eq!(report.disposition, Disposition::IsolateSource);
        assert_eq!(report.mismatch, Some(changed_evidence(&rechecked)));
        assert_eq!(occurrences(&trace, TraceStep::ReaderAccepted), 2);
        assert_eq!(occurrences(&trace, TraceStep::CandidateRootBuilt), 2);
        assert_eq!(occurrences(&trace, TraceStep::SourceRecheckStarted), 2);
    }
}

#[test]
fn transient_post_validation_source_change_passes_only_after_full_recompute() {
    let _guard = InjectionGuard;
    let root = build(FULL_SPATIAL, SpatialBuildOption::Omit);
    let changed = changed_speed(1);
    test_injection::builder_reads(vec![Some(BuilderRead::Bytes(changed.clone()))]);
    test_injection::recheck_sources(vec![changed.clone()]);
    let (outcome, trace) = observed(
        &root,
        FULL_SPATIAL,
        expectation_from(root.canonical_origin()),
    );
    let report = concluded(outcome);
    assert_eq!(report.verdict, AttestationVerdict::Pass);
    assert_eq!(report.disposition, Disposition::None);
    assert_eq!(
        report.source_audit,
        vec![SourceAuditNote::TransientSourceFailure {
            evidence: Box::new(changed_evidence(&changed)),
        }]
    );
    assert_eq!(occurrences(&trace, TraceStep::ReaderAccepted), 2);
    assert_eq!(occurrences(&trace, TraceStep::RootComparisonStarted), 2);
}

#[test]
fn different_post_validation_changes_preserve_both_source_evidences() {
    let _guard = InjectionGuard;
    let root = build(FULL_SPATIAL, SpatialBuildOption::Omit);
    let first = changed_speed(1);
    let second = changed_speed(2);
    test_injection::builder_reads(vec![
        Some(BuilderRead::Bytes(first.clone())),
        Some(BuilderRead::Bytes(second.clone())),
    ]);
    test_injection::recheck_sources(vec![first.clone(), second.clone()]);
    let (outcome, _) = observed(
        &root,
        FULL_SPATIAL,
        expectation_from(root.canonical_origin()),
    );
    let AttestationOutcome::OperationFailed(failure) = outcome else {
        panic!("expected unstable source evidence");
    };
    assert_eq!(failure.kind, OperationFailureKind::SourceEvidenceUnstable);
    assert_eq!(
        failure.source_audit.last(),
        Some(&SourceAuditNote::SourceEvidenceUnstable {
            first: Box::new(changed_evidence(&first)),
            recompute: Box::new(changed_evidence(&second)),
        })
    );
}

#[test]
fn established_root_evidence_survives_recompute_source_change() {
    let _guard = InjectionGuard;
    let mut root = build(FULL_SPATIAL, SpatialBuildOption::Omit);
    distort(&mut root, |root| {
        root.traffic.lane_lengths_millimetres[0] ^= 1
    });
    let expectation = expectation_from(root.canonical_origin());
    let first_evidence = concluded(attest(
        &root,
        FULL_SPATIAL,
        expectation,
        SpatialBuildOption::Omit,
    ))
    .mismatch
    .unwrap();
    let changed = changed_speed(1);
    test_injection::builder_reads(vec![None, Some(BuilderRead::Bytes(changed.clone()))]);
    test_injection::recheck_sources(vec![FULL_SPATIAL.to_vec(), changed]);
    let (outcome, trace) = observed(&root, FULL_SPATIAL, expectation);
    let report = concluded(outcome);
    assert_eq!(report.verdict, AttestationVerdict::Untrusted);
    assert_eq!(report.cause, Some(AttestationCause::Indeterminate));
    assert_eq!(report.disposition, Disposition::DisableRoot);
    assert_eq!(report.mismatch, Some(first_evidence));
    assert_eq!(occurrences(&trace, TraceStep::SourceRecheckStarted), 2);
}

#[test]
fn panic_during_builder_reread_stays_on_source_side() {
    for transient in [false, true] {
        let _guard = InjectionGuard;
        let root = build(FULL_SPATIAL, SpatialBuildOption::Omit);
        let panic = BuilderRead::Panic {
            read: 4,
            payload: "injected builder reread panic",
        };
        test_injection::builder_reads(vec![Some(panic); if transient { 1 } else { 2 }]);
        let (outcome, trace) = observed(
            &root,
            FULL_SPATIAL,
            expectation_from(root.canonical_origin()),
        );
        let report = concluded(outcome);
        if transient {
            assert_eq!(report.verdict, AttestationVerdict::Pass);
            assert_eq!(report.disposition, Disposition::None);
            assert!(
                matches!(report.source_audit.last(), Some(SourceAuditNote::TransientSourceFailure { evidence })
                if matches!(&evidence.site, MismatchSite::SourceFailure { detail: SourceFailureDetail::Panic(payload) } if payload == "injected builder reread panic"))
            );
        } else {
            assert_eq!(report.verdict, AttestationVerdict::Mismatch);
            assert_eq!(report.cause, Some(AttestationCause::Source));
            assert_eq!(report.disposition, Disposition::IsolateSource);
            assert_eq!(
                report.mismatch,
                Some(MismatchEvidence {
                    stage: AttestationStage::SourcePreparation,
                    site: MismatchSite::SourceFailure {
                        detail: SourceFailureDetail::Panic("injected builder reread panic".into())
                    },
                })
            );
        }
        assert_eq!(occurrences(&trace, TraceStep::ReaderAccepted), 2);
        assert_eq!(
            occurrences(&trace, TraceStep::CandidateRootBuilt),
            usize::from(transient)
        );
    }
}

fn reader_accepted_unclosed_identity() -> Vec<u8> {
    let mut bytes = FULL_SPATIAL.to_vec();
    let ordinal = field_offset(&bytes, 1, 0, 2);
    bytes[ordinal..ordinal + 4].copy_from_slice(&1_u32.to_le_bytes());
    refresh_chunk_digests(&mut bytes);
    // 仅测试输入重新登记修订声明，使真实 reader 接纳、真实 builder 检出跨表身份不闭合。
    let view = preflight_object_values(
        &bytes,
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap();
    let mut hasher = Sha256::new();
    hasher.update(NETWORK_REVISION_DOMAIN_PREFIX);
    for ordinal in 0..6 {
        let section = view.registry_view().section(ordinal).unwrap();
        hasher.update(section.kind().to_le_bytes());
        hasher.update(
            PortableObjectKind::CanonicalArtifact
                .section_format_version()
                .to_le_bytes(),
        );
        hasher.update((section.bytes().len() as u64).to_le_bytes());
        hasher.update(section.bytes());
    }
    let declaration = field_offset(&bytes, 7, 0, 1);
    bytes[declaration..declaration + 32].copy_from_slice(&hasher.finalize());
    refresh_chunk_digests(&mut bytes);
    bytes
}

#[test]
fn reader_accepted_builder_closure_failure_precedes_root_comparison() {
    let bytes = reader_accepted_unclosed_identity();
    let checked = check_canonical_network_input(bytes.as_slice(), FormatLimits::HARD).unwrap();
    let expectation = RootAttestationExpectation::new(
        checked.network_revision(),
        checked.canonical_artifact_digest(),
        checked.canonical_artifact_byte_length(),
    );
    let builder_failure = build_shared_network_revision(
        checked,
        SharedNetworkBuildOptions::new(SpatialBuildOption::Omit, BUILD_LIMITS),
    )
    .err()
    .expect("reader-accepted input must fail builder closure");
    assert!(matches!(
        builder_failure,
        BuildError::UnexpectedOrdinal {
            structure: BuildStructure::CanonicalIdentity,
            expected: 0,
            actual: 1
        }
    ));
    for root_corrupt in [false, true] {
        let mut root = build(FULL_SPATIAL, SpatialBuildOption::Omit);
        if root_corrupt {
            distort(&mut root, |root| {
                root.traffic.lane_lengths_millimetres[0] ^= 1
            });
        }
        let (outcome, trace) = observed(&root, &bytes, expectation);
        let report = concluded(outcome);
        assert_eq!(report.verdict, AttestationVerdict::Mismatch);
        assert_eq!(report.cause, Some(AttestationCause::Source));
        assert_eq!(report.disposition, Disposition::IsolateSource);
        assert_eq!(
            report.mismatch.unwrap(),
            MismatchEvidence {
                stage: AttestationStage::SourcePreparation,
                site: MismatchSite::SourceFailure {
                    detail: SourceFailureDetail::Builder(builder_failure)
                },
            }
        );
        assert_eq!(occurrences(&trace, TraceStep::ReaderAccepted), 2);
        assert!(!trace.contains(&TraceStep::RootComparisonStarted));
    }
}
