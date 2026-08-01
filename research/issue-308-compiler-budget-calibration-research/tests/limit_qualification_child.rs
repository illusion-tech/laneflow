use issue_308_compiler_budget_calibration_research::{
    ATTRIBUTION_BINARY_ID, CLEANUP_CHILD_SCHEMA, CLEANUP_CHILD_SCHEMA_VERSION, CleanupChildReport,
    CleanupFailureCase, CleanupScaleRole, DUPLICATE_OWNER_ERROR_CODE, GraphProfileId,
    GuardThresholds, LimitQualificationScale, SEMANTIC_FAILURE_CHILD_SCHEMA,
    SEMANTIC_FAILURE_CHILD_SCHEMA_VERSION, ScalableWorkloadId, SemanticFailureChildReport,
    TIMING_BINARY_ID,
};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

#[test]
fn duplicate_owner_qualification_runs_in_the_timing_child() {
    let scale = scale(ScalableWorkloadId::Corridor, GraphProfileId::WideStar);
    let compiler_instance_id =
        "failure/semantic-duplicate-owner/wide-star-v1/calibration/n-1/compiler-instance";
    let scale_json = serde_json::to_string(&scale).expect("serialize scale");
    let (child_pid, output) = run_handshaken(
        Path::new(env!(
            "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-timing"
        )),
        &["run-duplicate-owner", compiler_instance_id, &scale_json],
    );
    let report = serde_json::from_slice::<SemanticFailureChildReport>(&output.stdout)
        .expect("semantic failure child report");
    assert_eq!(report.schema, SEMANTIC_FAILURE_CHILD_SCHEMA);
    assert_eq!(report.schema_version, SEMANTIC_FAILURE_CHILD_SCHEMA_VERSION);
    assert_eq!(report.binary_id, TIMING_BINARY_ID);
    assert_eq!(report.child_pid, child_pid);
    assert_eq!(report.compiler_instance_id, compiler_instance_id);
    assert_eq!(
        report.observation.stable_compiler_error_code.as_deref(),
        Some(DUPLICATE_OWNER_ERROR_CODE)
    );
    assert_eq!(report.observation.diagnostic_count, 1);
    assert_eq!(report.observation.live_requested_bytes_after_run, 0);
}

#[test]
fn repeated_failure_cleanup_runs_in_one_attribution_child() {
    let scale = scale(ScalableWorkloadId::Corridor, GraphProfileId::SharedFaninDag);
    let scale_json = serde_json::to_string(&scale).expect("serialize scale");
    let case_json = serde_json::to_string(&CleanupFailureCase::MissingReferencePerUnit)
        .expect("serialize case");
    let (child_pid, output) = run_handshaken(
        Path::new(env!(
            "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-attribution"
        )),
        &["run-cleanup", &scale_json, &case_json],
    );
    let report =
        serde_json::from_slice::<CleanupChildReport>(&output.stdout).expect("cleanup child report");
    assert_eq!(report.schema, CLEANUP_CHILD_SCHEMA);
    assert_eq!(report.schema_version, CLEANUP_CHILD_SCHEMA_VERSION);
    assert_eq!(report.binary_id, ATTRIBUTION_BINARY_ID);
    assert_eq!(report.child_pid, child_pid);
    assert_eq!(report.experiment.runs.len(), 35);
    assert_eq!(
        report.experiment.experiment_id,
        "cleanup/semantic/missing-reference-per-unit/calibration/n-1"
    );
    assert_eq!(
        report.experiment.case_id,
        CleanupFailureCase::MissingReferencePerUnit
    );
}

fn scale(
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
) -> LimitQualificationScale {
    LimitQualificationScale {
        workload_id,
        graph_profile,
        b: 1,
        scale_role: CleanupScaleRole::Calibration,
        n: 1,
        guard_thresholds: GuardThresholds::from_physical_memory_bytes(4 * 1024 * 1024 * 1024)
            .expect("guard thresholds"),
    }
}

fn run_handshaken(executable: &Path, arguments: &[&str]) -> (u32, Output) {
    let mut child = Command::new(executable)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn qualification child");
    let child_pid = child.id();
    child
        .stdin
        .take()
        .expect("qualification child stdin")
        .write_all(b"G")
        .expect("release qualification child handshake");
    let output = child
        .wait_with_output()
        .expect("wait for qualification child");
    assert!(
        output.status.success(),
        "{}: {}",
        executable.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    (child_pid, output)
}
