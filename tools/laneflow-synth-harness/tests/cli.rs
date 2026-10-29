use std::fs;
use std::process::Command;

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
