//! #963 首轮矩阵：阶段读取视图注入，保留真实 reader / builder / 比较 / 复算路径。
//! 所有替换对象都独立拥有，禁止借此模拟可变别名或声称覆盖物理页共用故障。

use super::*;
use laneflow_static_contract::ExactByteLength;
use std::sync::atomic::Ordering;

struct InjectionGuard;

impl Drop for InjectionGuard {
    fn drop(&mut self) {
        test_injection::reset();
        reset_allocation_failure_injection();
    }
}

fn changed_length(index: usize) -> Arc<SharedNetworkRevision> {
    let mut root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    distort(&mut root, |root| {
        root.traffic.lane_lengths_millimetres[index] ^= 1;
    });
    root
}

#[test]
fn every_static_contract_origin_field_is_rejected() {
    let fields = [
        OriginField::CanonicalFormatVersion,
        OriginField::IdentityEncodingVersion,
        OriginField::IdentityRegistryRevision,
        OriginField::NetworkRevisionDerivationVersion,
        OriginField::ConstraintContractVersion,
        OriginField::StaticExecutionContractVersion,
    ];
    for (index, field) in fields.into_iter().enumerate() {
        let mut root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
        let healthy = *root.canonical_origin();
        let contracts = healthy.static_contract_versions();
        let mut values = [
            contracts.canonical_format_version(),
            contracts.identity_encoding_version(),
            contracts.identity_registry_revision(),
            contracts.network_revision_derivation_version(),
            contracts.constraint_contract_version(),
            contracts.static_execution_contract_version(),
        ];
        values[index] += 1;
        let [
            format,
            encoding,
            registry,
            derivation,
            constraint,
            execution,
        ] = values;
        distort(&mut root, |root| {
            root.origin = CanonicalNetworkOrigin::new(
                healthy.canonical_artifact_digest(),
                healthy.canonical_artifact_byte_length(),
                healthy.network_revision(),
                StaticContractVersions::new(
                    format, encoding, registry, derivation, constraint, execution,
                ),
            );
        });
        let report = concluded(attest(
            &root,
            FULL_SPATIAL,
            expectation_from(&healthy),
            SpatialBuildOption::RetainAvailable,
        ));
        assert_eq!(report.verdict, AttestationVerdict::Mismatch, "{field:?}");
        assert_eq!(report.cause, Some(AttestationCause::Indeterminate));
        assert_eq!(report.disposition, Disposition::DisableRoot);
        assert!(
            matches!(report.mismatch.unwrap().site, MismatchSite::OriginField { field: actual, .. } if actual == field)
        );
    }
}

#[test]
fn origin_digest_length_and_combined_distortions_are_declared_misses() {
    for (digest_changed, length_changed) in [(true, false), (false, true), (true, true)] {
        let mut root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
        let healthy = *root.canonical_origin();
        let mut digest = *healthy.canonical_artifact_digest().as_bytes();
        if digest_changed {
            digest[0] ^= 0x80;
        }
        let length = healthy.canonical_artifact_byte_length().get() + u64::from(length_changed);
        distort(&mut root, |root| {
            root.origin = CanonicalNetworkOrigin::new(
                Sha256Digest::from_bytes(digest),
                ExactByteLength::new(length),
                healthy.network_revision(),
                healthy.static_contract_versions(),
            );
        });
        let report = concluded(attest(
            &root,
            FULL_SPATIAL,
            expectation_from(&healthy),
            SpatialBuildOption::RetainAvailable,
        ));
        assert_eq!(report.verdict, AttestationVerdict::Pass);
        assert_eq!(report.disposition, Disposition::None);
    }
}

#[test]
fn transient_root_and_moving_mismatch_are_untrusted_through_real_recompute() {
    for moving in [false, true] {
        let _guard = InjectionGuard;
        let root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
        let second = moving.then(|| changed_length(1));
        test_injection::comparisons(vec![(None, Some(changed_length(0))), (None, second)]);
        let report = concluded(attest(
            &root,
            FULL_SPATIAL,
            expectation_from(root.canonical_origin()),
            SpatialBuildOption::RetainAvailable,
        ));
        assert_eq!(report.verdict, AttestationVerdict::Untrusted);
        assert_eq!(report.disposition, Disposition::DisableRoot);
        assert!(matches!(
            report.mismatch.unwrap().site,
            MismatchSite::TableValue { index: 0, .. }
        ));
    }
}

#[test]
fn root_mismatch_is_not_erased_by_recompute_allocation_failure_or_cancel() {
    for cancel in [false, true] {
        let _guard = InjectionGuard;
        let root = changed_length(0);
        let cancelled = Arc::new(AtomicBool::new(false));
        let injected_cancel = cancelled.clone();
        test_injection::on_recompute(move || {
            if cancel {
                injected_cancel.store(true, Ordering::Relaxed);
            } else {
                arm_allocation_failure_injection(1);
            }
        });
        let report = concluded(attest_with_options(
            &root,
            FULL_SPATIAL,
            expectation_from(root.canonical_origin()),
            FormatLimits::HARD,
            SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS)
                .with_cancellation(&cancelled),
        ));
        assert_eq!(report.verdict, AttestationVerdict::Untrusted);
        assert_eq!(report.disposition, Disposition::DisableRoot);
        assert!(matches!(
            report.mismatch.unwrap().site,
            MismatchSite::TableValue { index: 0, .. }
        ));
    }
}

#[test]
fn source_transient_and_different_failures_use_real_reader_results() {
    for unstable in [false, true] {
        let _guard = InjectionGuard;
        let root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
        let mut wrong_magic = FULL_SPATIAL.to_vec();
        wrong_magic[0] ^= 0xFF;
        let sources = if unstable {
            vec![wrong_magic, vec![0]]
        } else {
            vec![wrong_magic]
        };
        test_injection::sources(sources);
        let outcome = attest(
            &root,
            FULL_SPATIAL,
            expectation_from(root.canonical_origin()),
            SpatialBuildOption::RetainAvailable,
        );
        if unstable {
            let AttestationOutcome::OperationFailed(failure) = outcome else {
                panic!("expected unstable source failure");
            };
            assert_eq!(failure.kind, OperationFailureKind::SourceEvidenceUnstable);
            assert!(matches!(
                failure.source_audit.last(),
                Some(SourceAuditNote::SourceEvidenceUnstable { .. })
            ));
        } else {
            let report = concluded(outcome);
            assert_eq!(report.verdict, AttestationVerdict::Pass);
            assert_eq!(report.disposition, Disposition::None);
            assert!(matches!(
                report.source_audit.last(),
                Some(SourceAuditNote::TransientSourceFailure { .. })
            ));
        }
    }
}

#[test]
fn common_cause_equal_zero_columns_are_a_declared_comparison_miss() {
    let _guard = InjectionGuard;
    let root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let mut candidate_view = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let mut root_view = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    for view in [&mut candidate_view, &mut root_view] {
        distort(view, |root| root.traffic.lane_lengths_millimetres.fill(0));
    }
    test_injection::comparisons(vec![(Some(candidate_view), Some(root_view))]);
    let report = concluded(attest(
        &root,
        FULL_SPATIAL,
        expectation_from(root.canonical_origin()),
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Pass);
    // 只确认相等内容不可被比较器区分；两份 backing 独立，并非真实物理共页。
    assert_eq!(report.disposition, Disposition::None);
}

#[test]
fn zero_replaced_and_truncated_derived_columns_are_rejected() {
    for pattern in 0..3 {
        let mut root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
        let healthy = *root.canonical_origin();
        distort(&mut root, |root| {
            let values = &mut root.planning_hints.edge_boundary_weights;
            match pattern {
                0 => values.fill(0),
                1 => values.fill(u32::MAX),
                _ => {
                    *values = values[..values.len() - 1].to_vec().into_boxed_slice();
                }
            }
        });
        let report = concluded(attest(
            &root,
            FULL_SPATIAL,
            expectation_from(&healthy),
            SpatialBuildOption::RetainAvailable,
        ));
        assert_eq!(
            report.verdict,
            AttestationVerdict::Mismatch,
            "pattern {pattern}"
        );
        assert_eq!(report.cause, Some(AttestationCause::SharedStaticRoot));
        assert_eq!(report.disposition, Disposition::DisableRoot);
    }
}

#[test]
fn published_source_page_patterns_are_source_failures() {
    const PAGE: usize = 4_096;
    assert!(FULL_SPATIAL.len() > 3 * PAGE, "fixture needs three pages");
    let root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    for pattern in 0..5 {
        let mut bytes = FULL_SPATIAL.to_vec();
        match pattern {
            0 => bytes[PAGE..2 * PAGE].fill(0),
            1 => bytes.copy_within(2 * PAGE..3 * PAGE, PAGE),
            2 => bytes.rotate_left(PAGE),
            3 => bytes.rotate_right(PAGE),
            _ => bytes.truncate(bytes.len() - PAGE),
        }
        assert_ne!(bytes, FULL_SPATIAL, "pattern must change content");
        // 故意注入的摘要失配不进入 `[digest-probe]` 现场行与留证（#794）。
        let report = concluded(laneflow_format::without_digest_probe(|| {
            attest(
                &root,
                &bytes,
                expectation_from(root.canonical_origin()),
                SpatialBuildOption::RetainAvailable,
            )
        }));
        assert_eq!(
            report.verdict,
            AttestationVerdict::Mismatch,
            "pattern {pattern}"
        );
        assert_eq!(report.cause, Some(AttestationCause::Source));
        assert_eq!(report.disposition, Disposition::IsolateSource);
    }
}
