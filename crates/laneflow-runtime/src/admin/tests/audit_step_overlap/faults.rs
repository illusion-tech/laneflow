//! 阶段视图注入：真实认证的首次比较与复算都发生在 step 提交前。
//! 世界和 Spatial 始终持有同一个健康根；不模拟活动借用期间的物理 backing 改写。

use laneflow_static_network::{
    AttestationEvent, AttestationStage, AttestationTelemetry, MismatchEvidence, MismatchSite,
    RootComponent, TelemetryDeliveryError, TraceStep,
    attestation_fixtures::{flip_lane_length, with_root_comparisons},
};

use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) enum Fault {
    Persistent,
    Transient,
    Moving,
}

#[derive(Default)]
struct Trace(Vec<TraceStep>);

impl AttestationTelemetry for Trace {
    fn emit(&mut self, event: AttestationEvent<'_>) -> Result<(), TelemetryDeliveryError> {
        if let AttestationEvent::Trace(step) = event {
            self.0.push(step);
        }
        Ok(())
    }
}

fn changed_root(index: usize) -> Arc<SharedNetworkRevision> {
    let input = check_canonical_network_input(LFCA, FormatLimits::HARD).unwrap();
    let mut view = build_shared_network_revision(input, options()).unwrap();
    flip_lane_length(Arc::get_mut(&mut view).unwrap(), index);
    view
}

pub(super) fn audit(root: &Arc<SharedNetworkRevision>, fault: Fault) -> AttestationOutcome {
    let original_lengths = root.traffic().lane_lengths_millimetres().to_vec();
    let first = changed_root(0);
    let second = match fault {
        Fault::Persistent => first.clone(),
        Fault::Transient => root.clone(),
        Fault::Moving => changed_root(1),
    };
    let mut trace = Trace::default();
    let outcome = with_root_comparisons(root, vec![first, second], || {
        attest_published_shared_network_revision(
            root,
            &reference(LFCA),
            LFCA,
            FormatLimits::HARD,
            options(),
            Some(&mut trace),
        )
    });
    let AttestationOutcome::Concluded(report) = &outcome else {
        panic!("revision comparison fault must produce a conclusion: {outcome:?}");
    };
    let (verdict, cause) = match fault {
        Fault::Persistent => (
            AttestationVerdict::Mismatch,
            AttestationCause::SharedStaticRoot,
        ),
        Fault::Transient | Fault::Moving => (
            AttestationVerdict::Untrusted,
            AttestationCause::Indeterminate,
        ),
    };
    assert_eq!(report.verdict, verdict);
    assert_eq!(report.cause, Some(cause));
    assert_eq!(report.disposition, Disposition::DisableRoot);
    assert_eq!(report.telemetry_dropped, 0);
    assert!(report.source_audit.is_empty());
    // 复算通过或失配移到 index 1 都不能覆盖首次 index 0 的完整证据。
    assert_eq!(
        report.mismatch,
        Some(MismatchEvidence {
            stage: AttestationStage::RootComparison,
            site: MismatchSite::TableValue {
                component: RootComponent::Traffic,
                table: "lane_lengths_millimetres",
                entity_kind: None,
                index: 0,
                field: "value",
                expected: original_lengths[0].to_string(),
                actual: (original_lengths[0] ^ 1).to_string(),
            },
        })
    );
    use TraceStep::*;
    let mut expected_trace = vec![
        SourcePreparationStarted,
        ReaderAccepted,
        SameRevisionCriterionChecked,
        CandidateRootBuilt,
        RootComparisonStarted,
        SourceRecheckStarted,
        RecomputeStarted,
        SourcePreparationStarted,
        ReaderAccepted,
        SameRevisionCriterionChecked,
        CandidateRootBuilt,
        RootComparisonStarted,
    ];
    if !matches!(fault, Fault::Transient) {
        expected_trace.push(SourceRecheckStarted);
    }
    assert_eq!(trace.0, expected_trace);
    assert_eq!(root.traffic().lane_lengths_millimetres(), original_lengths);
    // 同线程紧接着认证健康根必须通过，证明视图未泄漏到后续调用。
    let healthy = attest_published_shared_network_revision(
        root,
        &reference(LFCA),
        LFCA,
        FormatLimits::HARD,
        options(),
        None,
    );
    assert!(matches!(
        healthy,
        AttestationOutcome::Concluded(report) if report.verdict == AttestationVerdict::Pass
    ));
    outcome
}

#[test]
fn shared_static_root_mismatch_stops_calls_and_quarantines_late_step() {
    exercise(Case::Revision(Fault::Persistent));
}

#[test]
fn transient_root_untrusted_stops_calls_and_quarantines_late_step() {
    exercise(Case::Revision(Fault::Transient));
}

#[test]
fn moving_root_untrusted_preserves_first_evidence_and_quarantines_late_step() {
    exercise(Case::Revision(Fault::Moving));
}
