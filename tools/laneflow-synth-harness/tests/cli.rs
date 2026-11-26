use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 带临时 git 仓库的测试环境：矩阵在该仓库内生成环境记录，实现提交取其 HEAD。
struct Fixture {
    root: tempfile::TempDir,
    repository: PathBuf,
    commit: String,
    input: PathBuf,
}

fn git(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(repository)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn fixture(declaration: serde_json::Value) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let repository = root.path().join("repository");
    fs::create_dir(&repository).unwrap();
    git(&repository, &["init", "-q"]);
    // 环境记录按 checkout 钉版的 channel 选工具链：临时仓库带上本仓库的钉版文件，
    // 并入首个提交保持工作树干净。
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rust-toolchain.toml"),
        repository.join("rust-toolchain.toml"),
    )
    .unwrap();
    git(&repository, &["add", "rust-toolchain.toml"]);
    git(
        &repository,
        &["commit", "-q", "--allow-empty", "-m", "fixture"],
    );
    let commit = git(&repository, &["rev-parse", "HEAD"]);
    let input = root.path().join("environment-input.json");
    fs::write(&input, serde_json::to_vec(&declaration).unwrap()).unwrap();
    Fixture {
        root,
        repository,
        commit,
        input,
    }
}

fn declaration(test_build: bool) -> serde_json::Value {
    let mut value = serde_json::json!({
        "environment":{"vendor_performance_mode":"无厂商性能模式","hardware_role":"uncertified"},
        "build_command":"fixture",
    });
    if test_build {
        value["test_build_command"] = "fixture".into();
    }
    value
}

fn read(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn assert_before_record(summary: &serde_json::Value, output: &Path) {
    let before = &summary["environment"]["before"];
    assert_eq!(before["file"], "environment-before.json");
    let record = read(&output.join("environment-before.json"));
    assert_eq!(record["format"], "laneflow-environment-record");
    assert_eq!(record["collection"]["phase"], "before");
    assert_eq!(record["source"]["value"]["clean"], true);
    assert_eq!(
        summary["environment"]["declaration"]["build_command"],
        "fixture"
    );
}

#[test]
fn worker_retry_matrix_stops_at_failed_normal_references_and_preserves_output() {
    let fixture = fixture(declaration(true));
    let output = fixture.root.path().join("retry");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_laneflow-synth-harness"))
            .arg("worker-retry-matrix")
            .arg(env!("CARGO_BIN_EXE_laneflow-synth-harness"))
            .arg(fixture.root.path().join("missing-inputs"))
            .arg(&fixture.commit)
            .arg(&fixture.input)
            .arg(&output)
            .current_dir(&fixture.repository)
            .output()
            .unwrap()
    };
    assert!(!run().status.success());
    let bytes = fs::read(output.join("summary.json")).unwrap();
    let summary: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(summary["complete"], false);
    assert_before_record(&summary, &output);
    assert!(summary["normal_runs"].as_array().unwrap().is_empty());
    assert!(summary["runs"].as_array().unwrap().is_empty());
    assert!(summary["comparisons"].as_array().unwrap().is_empty());
    let started = summary["execution_order"].as_array().unwrap();
    assert!(!started.is_empty() && started.len() <= 4);
    let errors = summary["errors"].as_array().unwrap();
    assert_eq!(errors.len(), started.len());
    for error in errors {
        assert_eq!(error["process"]["summary"]["exit"]["success"], false);
        assert_eq!(error["process"]["sample_interval_ms"], 1000);
    }
    assert!(started.iter().all(|row| row["normal"] == true));
    for task in started {
        let report = read(
            &output
                .join(task["run"].as_str().unwrap())
                .join("report.json"),
        );
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
    let fixture = fixture(declaration(false));
    let output = fixture.root.path().join("workers");
    let result = Command::new(env!("CARGO_BIN_EXE_laneflow-synth-harness"))
        .arg("worker-matrix")
        .arg(fixture.root.path().join("missing-inputs"))
        .arg(&fixture.commit)
        .arg(&fixture.input)
        .arg(&output)
        .current_dir(&fixture.repository)
        .output()
        .unwrap();
    assert!(!result.status.success());
    let summary = read(&output.join("summary.json"));
    assert_eq!(summary["complete"], false);
    assert_before_record(&summary, &output);
    assert_eq!(summary["max_concurrent_processes"], 4);
    assert_eq!(summary["max_configured_workers"], 32);
    assert!(summary["runs"].as_array().unwrap().is_empty());
    assert!(summary["comparisons"].as_array().unwrap().is_empty());
    let started = summary["execution_order"].as_array().unwrap();
    assert!(!started.is_empty() && started.len() <= 4);
    assert_eq!(summary["errors"].as_array().unwrap().len(), started.len());
    for job in started {
        let report = read(
            &output
                .join(job["run"].as_str().unwrap())
                .join("report.json"),
        );
        assert_eq!(report["complete"], false);
        assert_eq!(report["workers"], job["workers"]);
        assert!(report["error"].is_string());
    }
    let before = fs::read(output.join("summary.json")).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_laneflow-synth-harness"))
        .arg("worker-matrix")
        .arg(fixture.root.path())
        .arg(&fixture.commit)
        .arg(&fixture.input)
        .arg(&output)
        .current_dir(&fixture.repository)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(fs::read(output.join("summary.json")).unwrap(), before);
}

#[test]
fn worker_matrix_refuses_to_start_without_a_ready_environment() {
    let mut placeholder = declaration(false);
    placeholder["environment"]["vendor_performance_mode"] = "unknown".into();
    let mut auto_field = declaration(false);
    auto_field["environment"]["rustc_vv"] = "fixture".into();
    let mut no_build = declaration(false);
    no_build["build_command"] = " ".into();
    let cases = [
        ("placeholder", placeholder, false, false),
        ("auto-field", auto_field, false, false),
        ("no-build", no_build, false, false),
        ("dirty", declaration(false), true, false),
        ("other-commit", declaration(false), false, true),
    ];
    for (name, declaration, dirty, other_commit) in cases {
        let fixture = fixture(declaration);
        if dirty {
            fs::write(fixture.repository.join("untracked.txt"), b"x").unwrap();
        }
        let commit = if other_commit {
            "c".repeat(40)
        } else {
            fixture.commit.clone()
        };
        let output = fixture.root.path().join("workers");
        let result = Command::new(env!("CARGO_BIN_EXE_laneflow-synth-harness"))
            .arg("worker-matrix")
            .arg(fixture.root.path().join("missing-inputs"))
            .arg(&commit)
            .arg(&fixture.input)
            .arg(&output)
            .current_dir(&fixture.repository)
            .output()
            .unwrap();
        assert!(!result.status.success(), "{name}");
        let summary = read(&output.join("summary.json"));
        assert_eq!(summary["complete"], false, "{name}");
        assert!(
            summary["execution_order"].as_array().unwrap().is_empty(),
            "{name}: 环境不满足时不得启动子进程"
        );
        assert!(summary["error"].is_string(), "{name}");
    }
}

/// 生产路径上的工具链偏离（#1564）：测量环境的 `RUSTUP_TOOLCHAIN` 指向别的工具链时，
/// 环境记录如实记下工具链不可用，矩阵在启动任何子进程之前拒绝。用一个不存在的
/// 工具链名，不依赖机器上装了哪些工具链。
#[test]
fn worker_matrix_refuses_to_start_under_a_polluted_toolchain_environment() {
    let fixture = fixture(declaration(false));
    let output = fixture.root.path().join("workers");
    let result = Command::new(env!("CARGO_BIN_EXE_laneflow-synth-harness"))
        .arg("worker-matrix")
        .arg(fixture.root.path().join("missing-inputs"))
        .arg(&fixture.commit)
        .arg(&fixture.input)
        .arg(&output)
        .env("RUSTUP_TOOLCHAIN", "laneflow-nonexistent-toolchain")
        .current_dir(&fixture.repository)
        .output()
        .unwrap();
    assert!(!result.status.success());
    let summary = read(&output.join("summary.json"));
    assert_eq!(summary["complete"], false);
    assert!(
        summary["execution_order"].as_array().unwrap().is_empty(),
        "工具链不是钉版时不得启动子进程"
    );
    let error = summary["error"].as_str().unwrap();
    assert!(error.contains("工具链不可用"), "{error}");
    // 测量前记录照常写出，其中的工具链是不可用，而不是另查一次钉版得到的版本。
    let before = read(&output.join("environment-before.json"));
    assert!(before["toolchain"]["unavailable"].is_string(), "{before}");
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
    let summary = read(&output.join("summary.json"));
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
        let report = read(&directory.path().join("report.json"));
        assert_eq!(report["complete"], false);
        assert!(report["error"].is_string());
    }
}

#[test]
fn measurement_matrix_stops_on_first_failed_fresh_process_and_preserves_it() {
    let fixture = fixture(declaration(false));
    let output = fixture.root.path().join("measurement");
    let result = Command::new(env!("CARGO_BIN_EXE_measure"))
        .arg("matrix")
        .arg(fixture.root.path().join("missing-inputs"))
        .arg(fixture.root.path().join("missing-oracles"))
        .arg(&fixture.commit)
        .arg(&fixture.input)
        .arg(&output)
        .current_dir(&fixture.repository)
        .output()
        .unwrap();
    assert!(!result.status.success());
    let summary = read(&output.join("summary.json"));
    assert_eq!(summary["complete"], false);
    assert_before_record(&summary, &output);
    assert_eq!(summary["max_concurrent_measurements"], 1);
    let errors = summary["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["process"]["summary"]["exit"]["success"], false);
    assert_eq!(summary["execution_order"].as_array().unwrap().len(), 1);
    let failed = read(&output.join("10000-W1-16-p1-r1/report.json"));
    assert_eq!(failed["complete"], false);
    assert!(failed["error"].is_string());
}

#[test]
fn measurement_matrix_refuses_to_start_without_environment_input() {
    let fixture = fixture(declaration(false));
    let output = fixture.root.path().join("measurement");
    let result = Command::new(env!("CARGO_BIN_EXE_measure"))
        .arg("matrix")
        .arg(fixture.root.path().join("missing-inputs"))
        .arg(fixture.root.path().join("missing-oracles"))
        .arg(&fixture.commit)
        .arg(fixture.root.path().join("missing-environment-input.json"))
        .arg(&output)
        .current_dir(&fixture.repository)
        .output()
        .unwrap();
    assert!(!result.status.success());
    let summary = read(&output.join("summary.json"));
    assert_eq!(summary["complete"], false);
    assert!(summary["execution_order"].as_array().unwrap().is_empty());
    assert!(summary["errors"][0]["environment"].is_string());
}

/// `--rows` 子集矩阵：非标称行在创建输出前拒绝；合法子集记录筛选身份与非正式
/// 范围，只启动所选行。
#[test]
fn measurement_subset_matrix_records_filter_and_rejects_unknown_rows() {
    let fixture = fixture(declaration(false));
    let rejected = fixture.root.path().join("rejected");
    let result = Command::new(env!("CARGO_BIN_EXE_measure"))
        .arg("matrix")
        .arg(fixture.root.path().join("missing-inputs"))
        .arg(fixture.root.path().join("missing-oracles"))
        .arg(&fixture.commit)
        .arg(&fixture.input)
        .arg(&rejected)
        .args(["--rows", "10000-W1-16-p7"])
        .current_dir(&fixture.repository)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!rejected.exists());

    let output = fixture.root.path().join("subset");
    let result = Command::new(env!("CARGO_BIN_EXE_measure"))
        .arg("matrix")
        .arg(fixture.root.path().join("missing-inputs"))
        .arg(fixture.root.path().join("missing-oracles"))
        .arg(&fixture.commit)
        .arg(&fixture.input)
        .arg(&output)
        .args([
            "--rows",
            "100000-W1-33-p100,10000-W1-16-p100",
            "--no-load-sampling",
        ])
        .current_dir(&fixture.repository)
        .output()
        .unwrap();
    assert!(!result.status.success());
    let summary = read(&output.join("summary.json"));
    assert_eq!(
        summary["rows_filter"],
        serde_json::json!(["10000-W1-16-p100", "100000-W1-33-p100"])
    );
    assert_ne!(summary["matrix_scope"], "full_matrix");
    assert_eq!(summary["load_sampling"], false);
    let order = summary["execution_order"].as_array().unwrap();
    assert_eq!(order.len(), 1);
    assert_eq!(order[0]["label"], "10000-W1-16-p100-r1");
    assert_eq!(
        order[0]["arguments"].as_array().unwrap().last().unwrap(),
        "--no-load-sampling"
    );
}
