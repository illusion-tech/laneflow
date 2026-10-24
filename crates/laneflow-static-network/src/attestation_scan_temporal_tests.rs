//! #963 扫描与瞬态变化边界；阶段视图独立拥有，不模拟实际物理 backing 改写。

use super::source_timing::{InjectionGuard, changed_speed, observed, occurrences};
use super::*;
use crate::attestation::{AttestationReport, AttestationStage, TraceStep};
use test_injection::BuilderRead;

fn traffic_evidence(table: &'static str, expected: u32, actual: u32) -> MismatchEvidence {
    MismatchEvidence {
        stage: AttestationStage::RootComparison,
        site: MismatchSite::TableValue {
            component: RootComponent::Traffic,
            table,
            entity_kind: None,
            index: 0,
            field: "value",
            expected: expected.to_string(),
            actual: actual.to_string(),
        },
    }
}

fn assert_pass(report: &AttestationReport) {
    assert_eq!(report.verdict, AttestationVerdict::Pass);
    assert_eq!(report.cause, None);
    assert_eq!(report.disposition, Disposition::None);
    assert!(report.mismatch.is_none());
    assert!(report.source_audit.is_empty());
}

#[test]
fn already_read_column_is_not_revisited_in_same_scan() {
    let _guard = InjectionGuard;
    let root = build(FULL_SPATIAL, SpatialBuildOption::Omit);
    let expectation = expectation_from(root.canonical_origin());
    let mut later = build(FULL_SPATIAL, SpatialBuildOption::Omit);
    let original = root.traffic.lane_lengths_millimetres[0];
    distort(&mut later, |root| {
        root.traffic.lane_lengths_millimetres[0] ^= 1;
    });
    let changed = later.traffic.lane_lengths_millimetres[0];

    // 长度列已读完，剩余 traffic 列来自 later；变化只在已经读过的列。
    test_injection::after_lane_lengths(vec![Arc::clone(&later)]);
    let (outcome, trace) = observed(&root, FULL_SPATIAL, expectation);
    assert_pass(&concluded(outcome));
    assert_eq!(test_injection::pending_after_lane_lengths(), 0);
    assert_eq!(occurrences(&trace, TraceStep::RootComparisonStarted), 1);
    assert!(!trace.contains(&TraceStep::SourceRecheckStarted));
    assert_eq!(root.traffic.lane_lengths_millimetres[0], original);
    assert_ne!(original, changed);

    // 下一轮从 later 的首列开始读，必须检出该同一变化。
    let (outcome, trace) = observed(&later, FULL_SPATIAL, expectation);
    let report = concluded(outcome);
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::SharedStaticRoot));
    assert_eq!(report.disposition, Disposition::DisableRoot);
    assert_eq!(
        report.mismatch,
        Some(traffic_evidence(
            "lane_lengths_millimetres",
            original,
            changed
        ))
    );
    assert_eq!(occurrences(&trace, TraceStep::RootComparisonStarted), 2);
    assert_eq!(occurrences(&trace, TraceStep::SourceRecheckStarted), 2);
}

#[test]
fn unread_column_change_is_detected_through_both_comparisons() {
    let _guard = InjectionGuard;
    let root = build(FULL_SPATIAL, SpatialBuildOption::Omit);
    let expectation = expectation_from(root.canonical_origin());
    let original = root.traffic.lane_speed_limits_millimetres_per_second[0];
    let mut later = build(FULL_SPATIAL, SpatialBuildOption::Omit);
    distort(&mut later, |root| {
        root.traffic.lane_speed_limits_millimetres_per_second[0] ^= 1;
    });
    let changed = later.traffic.lane_speed_limits_millimetres_per_second[0];
    test_injection::after_lane_lengths(vec![Arc::clone(&later), Arc::clone(&later)]);

    let (outcome, trace) = observed(&root, FULL_SPATIAL, expectation);
    let report = concluded(outcome);
    assert_eq!(test_injection::pending_after_lane_lengths(), 0);
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::SharedStaticRoot));
    assert_eq!(report.disposition, Disposition::DisableRoot);
    assert_eq!(
        report.mismatch,
        Some(traffic_evidence(
            "lane_speed_limits_millimetres_per_second",
            original,
            changed,
        ))
    );
    assert!(report.source_audit.is_empty());
    assert_eq!(occurrences(&trace, TraceStep::ReaderAccepted), 2);
    assert_eq!(occurrences(&trace, TraceStep::CandidateRootBuilt), 2);
    assert_eq!(occurrences(&trace, TraceStep::RootComparisonStarted), 2);
    assert_eq!(occurrences(&trace, TraceStep::SourceRecheckStarted), 2);
}

#[test]
fn returned_pass_does_not_cover_later_revision_changes() {
    let mut root = build(FULL_SPATIAL, SpatialBuildOption::Omit);
    let expectation = expectation_from(root.canonical_origin());
    let original = root.traffic.lane_lengths_millimetres[0];
    let first = concluded(attest(
        &root,
        FULL_SPATIAL,
        expectation,
        SpatialBuildOption::Omit,
    ));
    assert_pass(&first);

    // 前一调用与借用均已结束，唯一 Arc 可安全修改；不在认证持有借用时写内存。
    distort(&mut root, |root| {
        root.traffic.lane_lengths_millimetres[0] ^= 1;
    });
    let changed = root.traffic.lane_lengths_millimetres[0];
    let next = concluded(attest(
        &root,
        FULL_SPATIAL,
        expectation,
        SpatialBuildOption::Omit,
    ));
    assert_pass(&first);
    assert_eq!(next.verdict, AttestationVerdict::Mismatch);
    assert_eq!(next.cause, Some(AttestationCause::SharedStaticRoot));
    assert_eq!(next.disposition, Disposition::DisableRoot);
    assert_eq!(
        next.mismatch,
        Some(traffic_evidence(
            "lane_lengths_millimetres",
            original,
            changed
        ))
    );
}

#[test]
fn source_aba_is_not_identified_by_equal_endpoint_hashes() {
    let _guard = InjectionGuard;
    let root = build(FULL_SPATIAL, SpatialBuildOption::Omit);
    let expectation = expectation_from(root.canonical_origin());
    let original = root.traffic.lane_speed_limits_millimetres_per_second[0];

    // reader 读取 A，首次 builder 实际消费 B，来源复核再次真实计算 A 的身份。
    // 第二次完整尝试不注入；修订侧首次证据因此不可复现，而非来源复核失配。
    test_injection::builder_reads(vec![Some(BuilderRead::Bytes(changed_speed(1)))]);
    test_injection::recheck_sources(vec![FULL_SPATIAL.to_vec()]);
    let (outcome, trace) = observed(&root, FULL_SPATIAL, expectation);
    let report = concluded(outcome);
    assert_eq!(report.verdict, AttestationVerdict::Untrusted);
    assert_eq!(report.cause, Some(AttestationCause::Indeterminate));
    assert_eq!(report.disposition, Disposition::DisableRoot);
    assert_eq!(
        report.mismatch,
        Some(traffic_evidence(
            "lane_speed_limits_millimetres_per_second",
            original - 1,
            original,
        ))
    );
    assert!(report.source_audit.is_empty());
    assert_eq!(occurrences(&trace, TraceStep::ReaderAccepted), 2);
    assert_eq!(occurrences(&trace, TraceStep::CandidateRootBuilt), 2);
    assert_eq!(occurrences(&trace, TraceStep::RootComparisonStarted), 2);
    assert_eq!(occurrences(&trace, TraceStep::SourceRecheckStarted), 1);
}
