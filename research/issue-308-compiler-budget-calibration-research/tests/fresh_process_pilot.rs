use issue_308_compiler_budget_calibration_research::{
    FRESH_PROCESS_PILOT_SAMPLE_COUNT, GraphProfileId, IdentityChildOutcome,
    IdentityChildTimingReport, load_repository_contract, run_identity_fresh_process_pilot,
};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

#[test]
fn identity_pilot_uses_seven_fresh_child_processes() {
    let executable = Path::new(env!(
        "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-research"
    ));
    let trusted = load_repository_contract().expect("frozen contract");
    let report = run_identity_fresh_process_pilot(
        &trusted,
        executable,
        "integration-fresh-process-pilot",
        GraphProfileId::WideStar,
        1,
        None,
    )
    .expect("fresh-process pilot");

    assert_eq!(report.samples.len(), FRESH_PROCESS_PILOT_SAMPLE_COUNT);
    assert!(report.samples.iter().all(|sample| {
        sample.child.child_pid > 0
            && sample.child.child_pid == sample.monitor.child_pid
            && sample.child.outcome == IdentityChildOutcome::Success
            && sample.child.peak_live_requested_bytes > 0
            && sample.monitor.observation_count > 0
            && sample.monitor.last_private_bytes.is_some()
            && sample.monitor.peak_private_bytes.is_some()
            && sample.monitor.exit_code == Some(0)
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
