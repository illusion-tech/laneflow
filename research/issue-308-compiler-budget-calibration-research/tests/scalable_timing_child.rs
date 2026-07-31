use issue_308_compiler_budget_calibration_research::{
    BASE_SCALE_STRING_PROFILE, GENERATOR_VERSION_V1, GraphProfileId, SCALABLE_TIMING_CHILD_SCHEMA,
    SCALABLE_TIMING_CHILD_SCHEMA_VERSION, ScalableTimingChildReport, ScalableWorkloadId,
    TIMING_BINARY_ID, WORKLOAD_REVISION_V1,
};
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn timing_role_runs_each_scalable_workload_in_a_fresh_process() {
    let timing_executable = env!("CARGO_BIN_EXE_issue-308-compiler-budget-calibration-timing");

    for workload_id in ScalableWorkloadId::ALL {
        let compiler_instance_id =
            format!("integration/{}/compiler-instance", workload_id.as_str());
        let mut child = Command::new(timing_executable)
            .arg("run")
            .arg(&compiler_instance_id)
            .arg(workload_id.as_str())
            .arg(GraphProfileId::WideStar.as_str())
            .arg("1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn timing role");
        let mut stdin = child.stdin.take().expect("timing role stdin");
        stdin.write_all(b"G").expect("release timing role");
        drop(stdin);
        let output = child.wait_with_output().expect("wait for timing role");
        assert!(
            output.status.success(),
            "{} timing role failed: {}",
            workload_id.as_str(),
            String::from_utf8_lossy(&output.stderr)
        );

        let report: ScalableTimingChildReport =
            serde_json::from_slice(&output.stdout).expect("scalable timing report");
        assert_eq!(report.schema, SCALABLE_TIMING_CHILD_SCHEMA);
        assert_eq!(report.schema_version, SCALABLE_TIMING_CHILD_SCHEMA_VERSION);
        assert_eq!(report.binary_id, TIMING_BINARY_ID);
        assert!(!report.allocation_instrumentation_enabled);
        assert_eq!(report.compiler_instance_id, compiler_instance_id);
        assert!(report.child_pid > 0);
        assert_eq!(report.workload_id, workload_id);
        assert_eq!(report.workload_revision, WORKLOAD_REVISION_V1);
        assert_eq!(report.graph_profile, GraphProfileId::WideStar.as_str());
        assert_eq!(report.string_profile, BASE_SCALE_STRING_PROFILE);
        assert_eq!(report.generator_version, GENERATOR_VERSION_V1);
        assert_eq!(report.n, 1);
        assert!(report.wall_time_ns > 0);
        assert_eq!(report.semantic_digest_sha256.len(), 64);
    }
}
