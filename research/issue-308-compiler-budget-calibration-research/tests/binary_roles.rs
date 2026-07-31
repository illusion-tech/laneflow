use issue_308_compiler_budget_calibration_research::{
    ATTRIBUTION_BINARY_ID, GraphProfileId, IdentityAttributionChildReport,
    IdentityAttributionOutcome, IdentityTimingChildReport, ORACLE_BINARY_ID, RUNNER_BINARY_ID,
    SCALABLE_ORACLE_CHILD_SCHEMA, ScalableOracleChildReport, ScalableOracleOutcome,
    ScalableTimingChildReport, ScalableTimingOutcome, ScalableWorkloadId, TIMING_BINARY_ID,
};
use serde_json::Value;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

#[test]
fn binary_descriptors_expose_four_disjoint_roles() {
    let cases = [
        (
            Path::new(env!(
                "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-research"
            )),
            RUNNER_BINARY_ID,
            "runner",
            None,
            false,
        ),
        (
            Path::new(env!(
                "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-timing"
            )),
            TIMING_BINARY_ID,
            "timing",
            Some("timing"),
            false,
        ),
        (
            Path::new(env!(
                "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-attribution"
            )),
            ATTRIBUTION_BINARY_ID,
            "attribution",
            Some("attribution"),
            true,
        ),
        (
            Path::new(env!(
                "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-oracle"
            )),
            ORACLE_BINARY_ID,
            "oracle",
            Some("oracle"),
            false,
        ),
    ];

    for (executable, binary_id, role, evidence_mode, instrumentation) in cases {
        let output = Command::new(executable)
            .arg("describe-role")
            .output()
            .expect("run role descriptor");
        assert!(
            output.status.success(),
            "{}: {}",
            executable.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let descriptor = serde_json::from_slice::<Value>(&output.stdout).expect("role descriptor");
        assert_eq!(descriptor["binaryId"], binary_id);
        assert_eq!(descriptor["role"], role);
        assert_eq!(
            descriptor["evidenceMode"],
            evidence_mode.map_or(Value::Null, Value::from)
        );
        assert_eq!(
            descriptor["allocationInstrumentationEnabled"],
            instrumentation
        );
    }
}

#[test]
fn runner_and_timing_roles_reject_direct_attribution_commands() {
    let runner = Command::new(env!(
        "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-research"
    ))
    .arg("identity-timing-child")
    .output()
    .expect("run rejected runner command");
    assert!(!runner.status.success());

    let timing = Command::new(env!(
        "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-timing"
    ))
    .args(["run", "wrong-role", "wide-star-v1", "1", "1024"])
    .output()
    .expect("run rejected timing command");
    assert!(!timing.status.success());
}

#[test]
fn timing_attribution_and_oracle_roles_agree_without_mixing_metrics() {
    let timing = run_handshaken(
        Path::new(env!(
            "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-timing"
        )),
        &[
            "run-identity-smoke",
            "role-matrix/timing",
            "wide-star-v1",
            "1",
        ],
    );
    let timing_report =
        serde_json::from_slice::<IdentityTimingChildReport>(&timing.stdout).expect("timing report");
    assert_eq!(timing_report.binary_id, TIMING_BINARY_ID);
    assert!(!timing_report.allocation_instrumentation_enabled);
    assert!(timing_report.wall_time_ns > 0);

    let attribution = run_handshaken(
        Path::new(env!(
            "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-attribution"
        )),
        &[
            "run",
            "role-matrix/attribution",
            "wide-star-v1",
            "1",
            &u64::MAX.to_string(),
        ],
    );
    let attribution_report =
        serde_json::from_slice::<IdentityAttributionChildReport>(&attribution.stdout)
            .expect("attribution report");
    assert_eq!(attribution_report.binary_id, ATTRIBUTION_BINARY_ID);
    assert!(attribution_report.allocation_instrumentation_enabled);
    assert_eq!(
        attribution_report.outcome,
        IdentityAttributionOutcome::Success
    );
    assert!(attribution_report.allocation.allocation_count > 0);
    assert!(attribution_report.allocation.allocated_bytes > 0);
    assert!(attribution_report.allocation.live_requested_bytes > 0);
    assert!(attribution_report.allocation.peak_live_requested_bytes > 0);
    assert_eq!(
        attribution_report.allocation.live_requested_bytes,
        attribution_report
            .allocation
            .allocated_bytes
            .checked_add(attribution_report.allocation.reallocated_bytes)
            .and_then(|total| total.checked_sub(attribution_report.allocation.freed_bytes))
            .expect("allocation accounting identity")
    );
    assert!(
        attribution_report.allocation.peak_live_requested_bytes
            >= attribution_report.allocation.live_requested_bytes
    );
    assert!(attribution_report.retained_capacity_bytes.is_some());
    assert!(
        attribution_report
            .attribution_wall_time_ns_diagnostic
            .is_some()
    );

    let oracle = Command::new(env!(
        "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-oracle"
    ))
    .args(["run", "wide-star-v1", "1"])
    .output()
    .expect("run oracle role");
    assert!(
        oracle.status.success(),
        "{}",
        String::from_utf8_lossy(&oracle.stderr)
    );
    let oracle_report = serde_json::from_slice::<Value>(&oracle.stdout).expect("oracle report");
    assert_eq!(oracle_report["binaryId"], ORACLE_BINARY_ID);

    let expected_digest = timing_report.semantic_digest_sha256.as_str();
    assert_eq!(
        attribution_report.semantic_digest_sha256.as_deref(),
        Some(expected_digest)
    );
    assert_eq!(
        oracle_report["stageSummary"]["semanticDigestSha256"],
        expected_digest
    );
}

#[test]
fn attribution_guard_is_structured_and_exits_zero() {
    let output = run_handshaken(
        Path::new(env!(
            "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-attribution"
        )),
        &["run", "role-matrix/guarded", "wide-star-v1", "1", "1"],
    );
    let report = serde_json::from_slice::<IdentityAttributionChildReport>(&output.stdout)
        .expect("guarded attribution report");
    assert_eq!(report.outcome, IdentityAttributionOutcome::GuardedInChild);
    assert!(report.allocation_instrumentation_enabled);
    assert_eq!(report.controlled_allocation_hard_ceiling_bytes, 1);
    assert_eq!(report.attribution_wall_time_ns_diagnostic, None);
    assert_eq!(report.retained_capacity_bytes, None);
    assert_eq!(report.semantic_digest_sha256, None);
    let guard = report
        .controlled_allocation_guard
        .expect("controlled allocation guard");
    assert_eq!(guard.hard_ceiling_bytes, 1);
    assert!(guard.live_requested_bytes + guard.requested_bytes > guard.hard_ceiling_bytes);
}

#[test]
fn scalable_oracle_independently_validates_every_workload_and_matches_the_timing_digest() {
    let timing_executable = Path::new(env!(
        "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-timing"
    ));
    let oracle_executable = Path::new(env!(
        "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-oracle"
    ));
    let ceiling = u64::MAX.to_string();

    for workload_id in ScalableWorkloadId::ALL {
        let compiler_instance_id = format!("role-matrix/{}/timing", workload_id.as_str());
        let timing = run_handshaken(
            timing_executable,
            &[
                "run",
                &compiler_instance_id,
                workload_id.as_str(),
                GraphProfileId::WideStar.as_str(),
                "1",
                &ceiling,
            ],
        );
        let timing_report = serde_json::from_slice::<ScalableTimingChildReport>(&timing.stdout)
            .expect("scalable timing report");
        assert_eq!(timing_report.outcome, ScalableTimingOutcome::Success);

        let oracle_run_id = format!("role-matrix/{}/oracle", workload_id.as_str());
        let oracle = run_handshaken(
            oracle_executable,
            &[
                "run-scalable",
                &oracle_run_id,
                workload_id.as_str(),
                GraphProfileId::WideStar.as_str(),
                "1",
                &ceiling,
            ],
        );
        let oracle_report = serde_json::from_slice::<ScalableOracleChildReport>(&oracle.stdout)
            .expect("scalable oracle report");
        assert_eq!(oracle_report.schema, SCALABLE_ORACLE_CHILD_SCHEMA);
        assert_eq!(oracle_report.binary_id, ORACLE_BINARY_ID);
        assert_eq!(oracle_report.oracle_run_id, oracle_run_id);
        assert_eq!(oracle_report.outcome, ScalableOracleOutcome::Success);
        assert!(
            oracle_report
                .guard_peak_live_requested_bytes
                .is_some_and(|peak| peak > 0 && peak <= ceiling.parse::<u64>().unwrap())
        );
        assert!(oracle_report.complete_counts_equal);
        assert!(oracle_report.complete_typed_output_equal);
        assert_eq!(
            oracle_report.semantic_digest_sha256,
            timing_report.semantic_digest_sha256
        );
    }
}

#[test]
fn scalable_oracle_guard_is_structured_before_any_full_output_is_built() {
    let output = run_handshaken(
        Path::new(env!(
            "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-oracle"
        )),
        &[
            "run-scalable",
            "role-matrix/oracle-guarded",
            ScalableWorkloadId::JunctionGrid.as_str(),
            GraphProfileId::WideStar.as_str(),
            "1",
            "1",
        ],
    );
    let report = serde_json::from_slice::<ScalableOracleChildReport>(&output.stdout)
        .expect("guarded scalable oracle report");
    assert_eq!(report.outcome, ScalableOracleOutcome::GuardedInChild);
    assert_eq!(report.primary_record_count, None);
    assert_eq!(report.semantic_digest_sha256, None);
    assert_eq!(report.guard_peak_live_requested_bytes, None);
    assert!(!report.complete_counts_equal);
    assert!(!report.complete_typed_output_equal);
    let guard = report
        .controlled_allocation_guard
        .expect("structured oracle guard");
    assert_eq!(guard.hard_ceiling_bytes, 1);
    assert!(guard.requested_bytes > 1);
}

fn run_handshaken(executable: &Path, arguments: &[&str]) -> Output {
    let mut child = Command::new(executable)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn role child");
    child
        .stdin
        .take()
        .expect("role child stdin")
        .write_all(b"G")
        .expect("release role child handshake");
    let output = child.wait_with_output().expect("wait for role child");
    assert!(
        output.status.success(),
        "{}: {}",
        executable.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
