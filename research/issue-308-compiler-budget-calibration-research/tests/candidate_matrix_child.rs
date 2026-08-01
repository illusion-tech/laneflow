use issue_308_compiler_budget_calibration_research::{
    CANDIDATE_KERNEL_CHILD_SCHEMA, CANDIDATE_KERNEL_CHILD_SCHEMA_VERSION, CANDIDATE_MATRIX_SCOPE,
    CandidateDisposition, CandidateExecutionMode, CandidateKernelChildReport, CandidateKeyDomain,
    CandidatePerformanceScalePlan, CandidatePerformanceScopeContract, CandidatePipelineOutcome,
    CandidatePipelineStratum, CandidateSafetyAssessment, CandidateSafetyStatus, CandidateScaleRole,
    ExactRatio, TIMING_BINARY_ID, load_repository_contract,
    qualify_pipeline_candidate_roster_fresh_process, run_mechanism_candidate_matrix_fresh_process,
    run_pipeline_candidate_matrix_fresh_process,
};
#[cfg(feature = "candidate-hashbrown-xxh3")]
use issue_308_compiler_budget_calibration_research::{
    CANDIDATE_PIPELINE_CHILD_SCHEMA, CANDIDATE_PIPELINE_CHILD_SCHEMA_VERSION,
    CandidatePipelineChildReport, qualify_constant_hash_candidate_fresh_process,
};
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn timing_role_runs_a_candidate_kernel_in_a_fresh_process() {
    let executable = env!("CARGO_BIN_EXE_issue-308-compiler-budget-calibration-timing");
    let mut child = Command::new(executable)
        .arg("run-candidate-kernel")
        .arg("stable-vec-sort-v1")
        .arg("canonical-output-order")
        .arg("128")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn candidate timing role");
    let child_pid = child.id();
    child
        .stdin
        .take()
        .expect("candidate child stdin")
        .write_all(b"G")
        .expect("release candidate timing role");
    let output = child.wait_with_output().expect("wait for candidate child");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: CandidateKernelChildReport =
        serde_json::from_slice(&output.stdout).expect("candidate child report");
    assert_eq!(report.schema, CANDIDATE_KERNEL_CHILD_SCHEMA);
    assert_eq!(report.schema_version, CANDIDATE_KERNEL_CHILD_SCHEMA_VERSION);
    assert_eq!(report.binary_id, TIMING_BINARY_ID);
    assert_eq!(report.child_pid, child_pid);
    assert_ne!(report.child_pid, std::process::id());
    assert_eq!(report.measurement.scope, CANDIDATE_MATRIX_SCOPE);
}

#[test]
#[cfg(feature = "candidate-hashbrown-xxh3")]
fn timing_role_runs_a_real_candidate_pipeline_in_a_fresh_process() {
    let executable = env!("CARGO_BIN_EXE_issue-308-compiler-budget-calibration-timing");
    let mut child = Command::new(executable)
        .arg("run-candidate-pipeline")
        .arg("candidate-pipeline-test")
        .arg("hashbrown-xxh3-fixed-v1")
        .arg("validated-fixed-key")
        .arg("LF-COMP-CORRIDOR-v1")
        .arg("wide-star-v1")
        .arg("1")
        .arg(u64::MAX.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn candidate pipeline timing role");
    let child_pid = child.id();
    child
        .stdin
        .take()
        .expect("candidate pipeline child stdin")
        .write_all(b"G")
        .expect("release candidate pipeline timing role");
    let output = child
        .wait_with_output()
        .expect("wait for candidate pipeline child");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: CandidatePipelineChildReport =
        serde_json::from_slice(&output.stdout).expect("candidate pipeline child report");
    assert_eq!(report.schema, CANDIDATE_PIPELINE_CHILD_SCHEMA);
    assert_eq!(
        report.schema_version,
        CANDIDATE_PIPELINE_CHILD_SCHEMA_VERSION
    );
    assert_eq!(report.binary_id, TIMING_BINARY_ID);
    assert_eq!(report.child_pid, child_pid);
    assert_eq!(report.outcome, CandidatePipelineOutcome::Success);
    assert!(report.wall_time_ns.is_some_and(|value| value > 0));
    assert!(report.semantic_digest_sha256.is_some());
    assert!(report.candidate_pipeline_checksums.is_some());
    assert!(
        report
            .guard_peak_live_requested_bytes
            .is_some_and(|value| value > 0)
    );
    assert!(report.controlled_allocation_guard.is_none());
}

#[test]
fn balanced_matrix_uses_a_new_timing_child_for_every_sample() {
    let trusted = load_repository_contract().expect("frozen contract");
    let executable = std::path::Path::new(env!(
        "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-timing"
    ));
    let execution = run_mechanism_candidate_matrix_fresh_process(
        &trusted,
        executable,
        CandidateKeyDomain::CanonicalOutputOrder,
        128,
        &[],
        ExactRatio::new(11, 10).expect("envelope"),
    )
    .expect("fresh-process candidate matrix");

    assert_eq!(
        execution.execution_mode,
        CandidateExecutionMode::FreshProcessTiming
    );
    assert_eq!(execution.samples.len(), 36);
    assert!(execution.samples.iter().all(|sample| {
        sample
            .child_pid
            .is_some_and(|pid| pid != std::process::id())
            && sample.binary_id.as_deref() == Some(TIMING_BINARY_ID)
    }));
}

#[test]
fn balanced_full_pipeline_matrix_uses_real_fresh_process_runs() {
    let trusted = load_repository_contract().expect("frozen contract");
    let executable = std::path::Path::new(env!(
        "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-timing"
    ));
    let oracle_executable = std::path::Path::new(env!(
        "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-oracle"
    ));
    let safety = [
        "stable-vec-sort-v1",
        "deterministic-radix-sort-v1",
        "deterministic-bucket-sort-v1",
    ]
    .into_iter()
    .map(|candidate_id| CandidateSafetyAssessment {
        candidate_id: candidate_id.to_owned(),
        status: CandidateSafetyStatus::Passed,
        evidence: "integration-test".to_owned(),
    })
    .collect::<Vec<_>>();
    let scope = CandidatePerformanceScopeContract::from_trusted_contract(&trusted)
        .expect("performance scope");
    let stratum = CandidatePipelineStratum::from_scope(
        &scope,
        CandidateKeyDomain::CanonicalOutputOrder,
        CandidatePerformanceScalePlan {
            scale_role: CandidateScaleRole::Base,
            n: 1,
            b: 1,
        },
    )
    .expect("test stratum");
    let roster = qualify_pipeline_candidate_roster_fresh_process(
        &trusted,
        executable,
        oracle_executable,
        stratum,
        &safety,
        &[],
    )
    .expect("full pipeline candidate qualification");
    assert!(
        roster.entries.iter().all(|entry| matches!(
            entry.disposition,
            CandidateDisposition::BaselineParticipant
                | CandidateDisposition::PerformanceParticipant
        )),
        "entries={:?}; oracle={:?}",
        roster.entries,
        roster.oracle_run
    );
    let execution = run_pipeline_candidate_matrix_fresh_process(executable, roster)
        .expect("fresh-process full pipeline matrix");

    assert!(execution.complete);
    assert_eq!(execution.schedule.len(), 12);
    assert_eq!(execution.samples.len(), 36);
    assert_eq!(execution.attempts.len(), 36);
    assert!(execution.samples.iter().all(|sample| {
        sample.child.child_pid != std::process::id()
            && sample.child.binary_id == TIMING_BINARY_ID
            && sample.child.outcome == CandidatePipelineOutcome::Success
    }));
}

#[test]
#[cfg(feature = "candidate-hashbrown-xxh3")]
fn constant_hash_qualification_retains_six_fresh_process_runs() {
    let trusted = load_repository_contract().expect("frozen contract");
    let timing = std::path::Path::new(env!(
        "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-timing"
    ));
    let oracle = std::path::Path::new(env!(
        "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-oracle"
    ));
    let execution = qualify_constant_hash_candidate_fresh_process(
        &trusted,
        timing,
        oracle,
        "hashbrown-xxh3-fixed-v1",
        0,
    )
    .expect("fresh process qualification");

    assert!(execution.qualification.passed);
    assert_eq!(execution.runs.len(), 6);
    assert_eq!(execution.qualification.observations.len(), 6);
    assert!(execution.runs.iter().all(|run| {
        run.child
            .as_ref()
            .is_some_and(|child| child.child_pid != std::process::id())
    }));
}
