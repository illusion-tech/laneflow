#![cfg(debug_assertions)]

use std::process::Command;

#[test]
fn formal_cli_rejects_debug_binary_before_creating_output() {
    let runner = env!("CARGO_BIN_EXE_issue-308-compiler-budget-calibration-research");
    let output_path = std::env::temp_dir().join(format!(
        "laneflow-issue-308-debug-formal-output-{}.json",
        std::process::id()
    ));
    let output = Command::new(runner)
        .arg("run")
        .arg("--protocol")
        .arg("compiler-calibration-v1")
        .arg("--output")
        .arg(&output_path)
        .output()
        .expect("run debug formal CLI");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("正式研究入口只接受 release 二进制"));
    assert!(!output_path.exists());
}
