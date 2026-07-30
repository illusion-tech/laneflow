#[cfg(windows)]
use issue_308_compiler_budget_calibration_research::{
    FRESH_PROCESS_PILOT_SAMPLE_COUNT, GraphProfileId, IdentityFreshProcessPilotOutcome,
    ProcessExitKind, RunStatus, TIMING_BINARY_ID, TerminationKind, load_repository_contract,
    run_identity_fresh_process_pilot,
};
#[cfg(windows)]
use std::collections::BTreeSet;
#[cfg(windows)]
use std::path::Path;

#[cfg(windows)]
#[test]
fn identity_pilot_uses_seven_fresh_uninstrumented_timing_processes() {
    let timing_executable = Path::new(env!(
        "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-timing"
    ));
    let trusted = load_repository_contract().expect("frozen contract");
    let outcome = run_identity_fresh_process_pilot(
        &trusted,
        timing_executable,
        "integration-fresh-process-pilot",
        GraphProfileId::WideStar,
        1,
        None,
    )
    .expect("fresh-process pilot");
    let IdentityFreshProcessPilotOutcome::Completed { pilot: report } = outcome else {
        panic!("N=1 smoke must complete: {outcome:?}");
    };

    assert_eq!(report.samples.len(), FRESH_PROCESS_PILOT_SAMPLE_COUNT);
    assert!(report.samples.iter().all(|sample| {
        sample.child.child_pid > 0
            && sample.process.child_pid.value == Some(u64::from(sample.child.child_pid))
            && sample.status == RunStatus::Valid
            && sample.invalidation_reasons.is_empty()
            && sample.process.binary_id == TIMING_BINARY_ID
            && sample.process.exit_kind == ProcessExitKind::Success
            && sample.process.exit_code.value == Some(0)
            && sample.process.termination.kind == TerminationKind::ExitCode
            && sample.child.binary_id == TIMING_BINARY_ID
            && !sample.child.allocation_instrumentation_enabled
            && sample.child.wall_time_ns > 0
            && sample.monitor.observation_count > 0
            && sample.monitor.last_private_bytes.value.is_some()
            && sample.monitor.peak_private_bytes.value.is_some()
            && sample.monitor.trigger.is_none()
    }));
    assert_eq!(
        report
            .samples
            .iter()
            .map(|sample| sample.child.compiler_instance_id.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        FRESH_PROCESS_PILOT_SAMPLE_COUNT
    );
    assert!(report.guard_preflight_evaluated);
    assert_eq!(
        report.guard_preflights.len(),
        FRESH_PROCESS_PILOT_SAMPLE_COUNT
    );
    assert!(report.guard_preflights.iter().all(|guard| {
        guard.allows_child_start
            && guard.triggers.is_empty()
            && guard.predicted_private_bytes.is_none()
            && guard.predicted_wall_time_ns.is_none()
    }));
    assert!(report.semantic_digest_consistent);
}
