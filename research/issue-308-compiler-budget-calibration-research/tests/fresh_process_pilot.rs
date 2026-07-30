#[cfg(windows)]
use issue_308_compiler_budget_calibration_research::{
    FRESH_PROCESS_PILOT_SAMPLE_COUNT, GraphProfileId, IdentityFreshProcessPilotOutcome,
    ProcessExitKind, RunStatus, TerminationKind, load_repository_contract,
    run_identity_fresh_process_pilot, run_identity_fresh_process_pilot_with_allocation_ceiling_cap,
};
use issue_308_compiler_budget_calibration_research::{
    IdentityChildOutcome, IdentityChildTimingReport,
};
#[cfg(windows)]
use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

#[cfg(windows)]
#[test]
fn identity_pilot_uses_seven_fresh_child_processes() {
    let executable = Path::new(env!(
        "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-research"
    ));
    let trusted = load_repository_contract().expect("frozen contract");
    let outcome = run_identity_fresh_process_pilot(
        &trusted,
        executable,
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
            && sample.process.exit_kind == ProcessExitKind::Success
            && sample.process.exit_code.value == Some(0)
            && sample.child.outcome == IdentityChildOutcome::Success
            && sample.child.peak_live_requested_bytes > 0
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
    assert_eq!(
        report
            .samples
            .iter()
            .map(|sample| sample.child.peak_live_requested_bytes)
            .collect::<BTreeSet<_>>()
            .len(),
        1
    );
    assert!(report.semantic_digest_consistent);
    assert!(report.guard_preflight_evaluated);
    assert_eq!(
        report.guard_preflights.len(),
        FRESH_PROCESS_PILOT_SAMPLE_COUNT
    );
    assert!(
        report
            .guard_preflights
            .iter()
            .all(|guard| guard.allows_child_start && guard.triggers.is_empty())
    );
}

#[test]
fn controlled_allocation_guarded_child_exits_zero_with_structured_result() {
    let executable = Path::new(env!(
        "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-research"
    ));
    let mut child = Command::new(executable)
        .arg("identity-timing-child")
        .arg("integration-guarded-child")
        .arg("wide-star-v1")
        .arg("1")
        .arg("1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn guarded child");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(b"G")
        .expect("release child handshake");
    let output = child.wait_with_output().expect("wait for guarded child");
    assert!(
        output.status.success(),
        "guarded child must exit zero: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = serde_json::from_slice::<IdentityChildTimingReport>(&output.stdout)
        .expect("structured guarded child report");
    assert_eq!(report.outcome, IdentityChildOutcome::GuardedInChild);
    assert!(report.controlled_allocation_guard.is_some());
    assert_eq!(report.wall_time_ns, None);
    assert_eq!(report.semantic_digest_sha256, None);
}

#[cfg(windows)]
#[test]
fn parent_classifies_a_guarded_child_as_a_structured_guarded_stop() {
    let executable = Path::new(env!(
        "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-research"
    ));
    let trusted = load_repository_contract().expect("frozen contract");
    let outcome = run_identity_fresh_process_pilot_with_allocation_ceiling_cap(
        &trusted,
        executable,
        "integration-guarded-pilot",
        GraphProfileId::WideStar,
        1,
        None,
        1,
    )
    .expect("guarded pilot outcome");
    let IdentityFreshProcessPilotOutcome::Stopped { stop } = outcome else {
        panic!("allocation ceiling must stop the pilot");
    };

    assert_eq!(stop.status, RunStatus::Guarded);
    assert!(stop.invalidation_reasons.is_empty());
    assert_eq!(stop.process.exit_kind, ProcessExitKind::GuardedInChild);
    assert_eq!(stop.process.exit_code.value, Some(0));
    assert_eq!(stop.process.termination.kind, TerminationKind::ExitCode);
    assert!(stop.process.child_pid.value.is_some());
    assert_eq!(
        stop.child.as_ref().map(|child| child.outcome),
        Some(IdentityChildOutcome::GuardedInChild)
    );
    assert!(stop.monitor.is_some());
    assert_eq!(stop.kill_error, None);
    assert_eq!(stop.monitor_error, None);
}
