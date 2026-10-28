//! 编辑来源分支（#529；完整性设计 §2.2、§4.2）：重编译闭包的期望值、字节身份比对、
//! 失败分类与复算调用。闭包在本 crate 内只返回固定 LFCA 字节；真实重编译的组合测试
//! 位于 `laneflow-runtime`。

use std::cell::Cell;

use super::*;
use crate::attestation::{RecompileFailure, attest_recompiled_shared_network_revision};

fn attest_recompiled(
    root: &Arc<SharedNetworkRevision>,
    recompile: &mut dyn FnMut() -> Result<Vec<u8>, RecompileFailure>,
) -> AttestationOutcome {
    attest_recompiled_shared_network_revision(
        root,
        recompile,
        FormatLimits::HARD,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
        None,
    )
}

#[test]
fn healthy_recompiled_root_passes_with_one_recompile() {
    let root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let calls = Cell::new(0_u32);
    let report = concluded(attest_recompiled(&root, &mut || {
        calls.set(calls.get() + 1);
        Ok(FULL_SPATIAL.to_vec())
    }));
    assert_eq!(report.verdict, AttestationVerdict::Pass);
    assert_eq!(report.disposition, Disposition::None);
    // 编辑来源没有外部引用，不产生字节身份审计诊断。
    assert!(report.source_audit.is_empty());
    assert_eq!(calls.get(), 1);
}

/// 同修订、不同字节：编辑来源的字节身份有独立期望值，失配判 `无法判因`，
/// 并在复算中再次调用重编译。
#[test]
fn recompiled_byte_identity_mismatch_is_indeterminate_after_second_recompile() {
    let root = build(PROVENANCE_BASE, SpatialBuildOption::RetainAvailable);
    let other = build(PROVENANCE_BUILD, SpatialBuildOption::RetainAvailable);
    assert_eq!(root.network_revision(), other.network_revision());
    let calls = Cell::new(0_u32);
    let report = concluded(attest_recompiled(&root, &mut || {
        calls.set(calls.get() + 1);
        Ok(PROVENANCE_BUILD.to_vec())
    }));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::Indeterminate));
    assert_eq!(report.disposition, Disposition::DisableRoot);
    assert!(matches!(
        report.mismatch.as_ref().expect("evidence").site,
        MismatchSite::OriginField {
            field: OriginField::CanonicalArtifactDigest,
            ..
        }
    ));
    assert_eq!(calls.get(), 2);
}

/// 发布来源分支不比对字节身份：同一对字节在发布来源下只记来源审计诊断。
#[test]
fn published_branch_still_ignores_byte_identity() {
    let root = build(PROVENANCE_BASE, SpatialBuildOption::RetainAvailable);
    let report = concluded(attest(
        &root,
        PROVENANCE_BUILD,
        expectation_from(root.canonical_origin()),
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Pass);
    assert!(matches!(
        report.source_audit.as_slice(),
        [SourceAuditNote::ByteIdentityDiffersFromReference { .. }]
    ));
}

#[test]
fn recompiled_revision_mismatch_is_indeterminate() {
    let root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let report = concluded(attest_recompiled(&root, &mut || Ok(MIN_HEADLESS.to_vec())));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::Indeterminate));
    assert!(matches!(
        report.mismatch.as_ref().expect("evidence").site,
        MismatchSite::OriginField {
            field: OriginField::NetworkRevision,
            ..
        }
    ));
}

#[test]
fn recompiled_derived_table_distortion_is_shared_static_root() {
    let mut root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    distort(&mut root, |revision| {
        crate::attestation_fixtures::perturb_edge_boundary_weight(revision, 0);
    });
    let report = concluded(attest_recompiled(&root, &mut || Ok(FULL_SPATIAL.to_vec())));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::SharedStaticRoot));
    assert!(matches!(
        report.mismatch.as_ref().expect("evidence").site,
        MismatchSite::TableValue {
            component: RootComponent::PlanningHints,
            table: "edge_boundary_weights",
            index: 0,
            ..
        }
    ));
}

#[test]
fn recompile_content_failure_is_confirmed_source_failure() {
    let root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let calls = Cell::new(0_u32);
    let report = concluded(attest_recompiled(&root, &mut || {
        calls.set(calls.get() + 1);
        Err(RecompileFailure::Content("admission rejected".to_owned()))
    }));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::Source));
    assert_eq!(report.disposition, Disposition::IsolateSource);
    assert_eq!(
        report.mismatch.expect("evidence").site,
        MismatchSite::SourceFailure {
            detail: SourceFailureDetail::Recompile("admission rejected".to_owned()),
        }
    );
    assert_eq!(calls.get(), 2);
}

/// 首次内容失败、复算重编译成功并通过：按复算归类表判 `Pass`，首次证据记为来源审计诊断。
#[test]
fn transient_recompile_content_failure_passes_with_audit_note() {
    let root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let calls = Cell::new(0_u32);
    let report = concluded(attest_recompiled(&root, &mut || {
        calls.set(calls.get() + 1);
        if calls.get() == 1 {
            Err(RecompileFailure::Content("transient".to_owned()))
        } else {
            Ok(FULL_SPATIAL.to_vec())
        }
    }));
    assert_eq!(report.verdict, AttestationVerdict::Pass);
    assert!(matches!(
        report.source_audit.as_slice(),
        [SourceAuditNote::TransientSourceFailure { .. }]
    ));
}

#[test]
fn recompile_resource_failure_and_cancel_are_operation_failures() {
    let root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    for (failure, kind) in [
        (
            RecompileFailure::Resource("compile limit".to_owned()),
            OperationFailureKind::RecompileResourceFailure,
        ),
        (RecompileFailure::Cancelled, OperationFailureKind::Cancelled),
    ] {
        let outcome = attest_recompiled(&root, &mut || Err(failure.clone()));
        match outcome {
            AttestationOutcome::OperationFailed(operation) => assert_eq!(operation.kind, kind),
            AttestationOutcome::Concluded(report) => {
                panic!("expected operation failure, got {report:?}")
            }
        }
    }
}

#[test]
fn recompile_panic_is_source_preparation_failure() {
    let root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let report = concluded(attest_recompiled(&root, &mut || {
        panic!("injected recompile panic")
    }));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::Source));
    assert!(matches!(
        report.mismatch.expect("evidence").site,
        MismatchSite::SourceFailure {
            detail: SourceFailureDetail::Panic(_),
        }
    ));
}

/// 首次修订侧失配后复算重编译失败：修订侧证据不能被撤销，判 `Untrusted`。
#[test]
fn recompile_failure_after_root_mismatch_is_untrusted() {
    let root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let calls = Cell::new(0_u32);
    let report = concluded(attest_recompiled(&root, &mut || {
        calls.set(calls.get() + 1);
        if calls.get() == 1 {
            Ok(MIN_HEADLESS.to_vec())
        } else {
            Err(RecompileFailure::Resource("compile limit".to_owned()))
        }
    }));
    assert_eq!(report.verdict, AttestationVerdict::Untrusted);
    assert_eq!(report.cause, Some(AttestationCause::Indeterminate));
}
