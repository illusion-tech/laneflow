use issue_308_compiler_budget_calibration_research::{
    CANDIDATE_KERNEL_CHILD_SCHEMA, CANDIDATE_KERNEL_CHILD_SCHEMA_VERSION, CANDIDATE_MATRIX_SCOPE,
    CandidateExecutionMode, CandidateKernelChildReport, CandidateKeyDomain, ExactRatio,
    TIMING_BINARY_ID, load_repository_contract, run_mechanism_candidate_matrix_fresh_process,
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
