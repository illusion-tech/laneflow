use std::fs;
use std::process::Command;

#[test]
fn worker_retry_matrix_stops_at_failed_normal_references_and_preserves_output() {
    let root = tempfile::tempdir().unwrap();
    let environment = root.path().join("environment.json");
    fs::write(
        &environment,
        serde_json::to_vec(&serde_json::json!({"implementation_commit":"c".repeat(40),
        "source_clean":true,"rustc_vv":"fixture","cargo_version":"fixture","target":"fixture",
        "build_command":"fixture","test_build_command":"fixture"}))
        .unwrap(),
    )
    .unwrap();
    let output = root.path().join("retry");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_laneflow-synth-harness"))
            .arg("worker-retry-matrix")
            .arg(env!("CARGO_BIN_EXE_laneflow-synth-harness"))
            .arg(root.path().join("missing-inputs"))
            .arg("c".repeat(40))
            .arg(&environment)
            .arg(&output)
            .output()
            .unwrap()
    };
    assert!(!run().status.success());
    let bytes = fs::read(output.join("summary.json")).unwrap();
    let summary: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(summary["complete"], false);
    assert!(summary["normal_runs"].as_array().unwrap().is_empty());
    assert!(summary["runs"].as_array().unwrap().is_empty());
    assert!(summary["comparisons"].as_array().unwrap().is_empty());
    let started = summary["execution_order"].as_array().unwrap();
    assert!(!started.is_empty() && started.len() <= 4);
    assert_eq!(summary["errors"].as_array().unwrap().len(), started.len());
    assert!(started.iter().all(|row| row["normal"] == true));
    for task in started {
        let report: serde_json::Value = serde_json::from_slice(
            &fs::read(
                output
                    .join(task["run"].as_str().unwrap())
                    .join("report.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(report["complete"], false);
    }
    assert!(!run().status.success());
    assert_eq!(bytes, fs::read(output.join("summary.json")).unwrap());
}

#[test]
fn worker_cli_rejects_zero_and_invalid_counts_without_overwriting_output() {
    let root = tempfile::tempdir().unwrap();
    for count in ["0", "not-a-number", "4294967296"] {
        let output = root.path().join(count);
        let result = Command::new(env!("CARGO_BIN_EXE_laneflow-synth-harness"))
            .args(["worker-run", "missing", &"c".repeat(40), count])
            .arg(&output)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(!output.exists());
    }
}

#[test]
fn worker_matrix_stops_launching_after_failure_and_collects_started_processes() {
    let root = tempfile::tempdir().unwrap();
    let environment = root.path().join("environment.json");
    fs::write(
        &environment,
        serde_json::to_vec(&serde_json::json!({
            "implementation_commit":"c".repeat(40),"source_clean":true,"rustc_vv":"fixture",
            "cargo_version":"fixture","target":"fixture","build_command":"fixture"
        }))
        .unwrap(),
    )
    .unwrap();
    let output = root.path().join("workers");
    let result = Command::new(env!("CARGO_BIN_EXE_laneflow-synth-harness"))
        .arg("worker-matrix")
        .arg(root.path().join("missing-inputs"))
        .arg("c".repeat(40))
        .arg(&environment)
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    let summary: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("summary.json")).unwrap()).unwrap();
    assert_eq!(summary["complete"], false);
    assert_eq!(summary["max_concurrent_processes"], 4);
    assert_eq!(summary["max_configured_workers"], 32);
    assert!(summary["runs"].as_array().unwrap().is_empty());
    assert!(summary["comparisons"].as_array().unwrap().is_empty());
    let started = summary["execution_order"].as_array().unwrap();
    assert!(!started.is_empty() && started.len() <= 4);
    assert_eq!(summary["errors"].as_array().unwrap().len(), started.len());
    for job in started {
        let report: serde_json::Value = serde_json::from_slice(
            &fs::read(
                output
                    .join(job["run"].as_str().unwrap())
                    .join("report.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(report["complete"], false);
        assert_eq!(report["workers"], job["workers"]);
        assert!(report["error"].is_string());
    }
    let before = fs::read(output.join("summary.json")).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_laneflow-synth-harness"))
        .arg("worker-matrix")
        .arg(root.path())
        .arg("c".repeat(40))
        .arg(environment)
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(fs::read(output.join("summary.json")).unwrap(), before);
}

#[test]
fn matrix_collects_failed_children_without_publishing_success() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("results");
    let result = Command::new(env!("CARGO_BIN_EXE_laneflow-synth-harness"))
        .arg("matrix")
        .arg(root.path().join("missing-inputs"))
        .arg("c".repeat(40))
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    let summary: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("summary.json")).unwrap()).unwrap();
    assert_eq!(summary["complete"], false);
    assert_eq!(summary["max_concurrent_processes"], 4);
    assert_eq!(summary["errors"].as_array().unwrap().len(), 16);
    assert!(summary["rows"].as_array().unwrap().is_empty());
    let directories: Vec<_> = fs::read_dir(output)
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| entry.file_type().unwrap().is_dir())
        .collect();
    assert_eq!(directories.len(), 16);
    for directory in directories {
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.path().join("report.json")).unwrap())
                .unwrap();
        assert_eq!(report["complete"], false);
        assert!(report["error"].is_string());
    }
}

#[test]
fn measurement_matrix_stops_on_first_failed_fresh_process_and_preserves_it() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("measurement");
    let result = Command::new(env!("CARGO_BIN_EXE_measure"))
        .arg("matrix")
        .arg(root.path().join("missing-inputs"))
        .arg(root.path().join("missing-oracles"))
        .arg("c".repeat(40))
        .arg(root.path().join("environment.json"))
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    let summary: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("summary.json")).unwrap()).unwrap();
    assert_eq!(summary["complete"], false);
    assert_eq!(summary["max_concurrent_measurements"], 1);
    assert_eq!(summary["errors"].as_array().unwrap().len(), 1);
    assert_eq!(summary["execution_order"].as_array().unwrap().len(), 1);
    let failed: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("10000-W1-16-p1-r1/report.json")).unwrap())
            .unwrap();
    assert_eq!(failed["complete"], false);
    assert!(failed["error"].is_string());
}
