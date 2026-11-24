//! CLI 集成测试：退出码、stdout/stderr 约定、hex 外层与限额。

mod common;

use std::process::Output;

use common::{lfsm_with_source_locations, minimal_object};

fn bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_laneflow-artifact-inspect"))
}

fn run(args: &[&str]) -> Output {
    std::process::Command::new(bin())
        .args(args)
        .output()
        .expect("启动 CLI")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout 为 UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr 为 UTF-8")
}

struct TempFile {
    path: std::path::PathBuf,
}

impl TempFile {
    fn new(name: &str, bytes: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "laneflow-artifact-inspect-test-{}-{name}",
            std::process::id()
        ));
        std::fs::write(&path, bytes).expect("写临时文件");
        Self { path }
    }

    fn path_str(&self) -> &str {
        self.path.to_str().expect("路径为 UTF-8")
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn hex_text(bytes: &[u8]) -> Vec<u8> {
    let mut text = String::with_capacity(bytes.len() * 2 + bytes.len() / 32);
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 && index % 32 == 0 {
            text.push('\n');
        }
        text.push_str(&format!("{byte:02x}"));
    }
    text.into_bytes()
}

#[test]
fn help_and_version_go_to_stdout_with_zero_exit() {
    let help = run(&["--help"]);
    assert!(help.status.success());
    assert!(stdout(&help).contains("用法："));
    assert!(stderr(&help).is_empty());

    let version = run(&["--version"]);
    assert!(version.status.success());
    assert!(stdout(&version).contains("LFCA 5 / LFSM 6 / LFSD 4 / LFCP 2"));
}

#[test]
fn inspect_valid_object_prints_report_and_exits_zero() {
    let file = TempFile::new(
        "lfcp.bin",
        &minimal_object(
            laneflow_static_contract::PortableObjectKind::CanonicalPublicationDescriptor,
        ),
    );
    let output = run(&["inspect", file.path_str()]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stdout(&output).contains("check: 单对象结构与直接值域检查 PASS"));
}

#[test]
fn inspect_corrupt_object_exits_two_with_stderr() {
    let mut bytes = minimal_object(laneflow_static_contract::PortableObjectKind::SemanticDiff);
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    let file = TempFile::new("corrupt.bin", &bytes);
    let output = run(&["inspect", file.path_str()]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!stderr(&output).is_empty());
    assert!(stdout(&output).is_empty());
}

#[test]
fn diff_same_object_exits_zero_and_different_exits_one() {
    let expected_bytes = lfsm_with_source_locations(&[3, 12]);
    let actual_bytes = lfsm_with_source_locations(&[3, 13]);
    let same = TempFile::new("same.lfsm", &expected_bytes);
    let other = TempFile::new("other.lfsm", &actual_bytes);
    let source = TempFile::new("src.lfsm", &expected_bytes);

    let output = run(&["diff", source.path_str(), same.path_str()]);
    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).contains("bytes: 相同"));

    let output = run(&["diff", source.path_str(), other.path_str()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("对象不同"));
    assert!(stdout(&output).contains("SourceLocation[1].startLine：12 → 13"));
}

#[test]
fn diff_same_invalid_objects_exit_two_not_zero() {
    let mut bytes = minimal_object(laneflow_static_contract::PortableObjectKind::SourceMap);
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    let file = TempFile::new("same-invalid.lfsm", &bytes);
    let output = run(&["diff", file.path_str(), file.path_str()]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stdout(&output).contains("bytes: 相同"));
    assert!(stdout(&output).contains("无法完成比较"));
    assert!(!stderr(&output).is_empty());
}

#[test]
fn diff_kind_mismatch_exits_two() {
    let lfca = TempFile::new(
        "a.lfca",
        &minimal_object(laneflow_static_contract::PortableObjectKind::CanonicalArtifact),
    );
    let lfsm = TempFile::new(
        "b.lfsm",
        &minimal_object(laneflow_static_contract::PortableObjectKind::SourceMap),
    );
    let output = run(&["diff", lfca.path_str(), lfsm.path_str()]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("对象种类不匹配"));
}

#[test]
fn hex_input_decodes_into_official_reader_path() {
    let bytes = minimal_object(
        laneflow_static_contract::PortableObjectKind::CanonicalPublicationDescriptor,
    );
    let hex = TempFile::new("lfcp.hex", &hex_text(&bytes));
    let output = run(&["inspect", "--hex", hex.path_str()]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stdout(&output).contains("check: 单对象结构与直接值域检查 PASS"));
}

#[test]
fn hex_odd_digits_and_invalid_characters_fail_without_guessing() {
    let odd = TempFile::new("odd.hex", b"abc");
    let output = run(&["inspect", "--hex", odd.path_str()]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("奇数位"));

    let invalid = TempFile::new("bad.hex", b"zz");
    let output = run(&["inspect", "--hex", invalid.path_str()]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("非法"));

    // raw 失败不自动回退 hex：同名 hex 文件按 raw 读必须失败。
    let hex = TempFile::new(
        "raw.hex",
        &hex_text(&minimal_object(
            laneflow_static_contract::PortableObjectKind::CanonicalPublicationDescriptor,
        )),
    );
    let output = run(&["inspect", hex.path_str()]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn tighten_only_budgets_reject_looser_values() {
    let file = TempFile::new("small.lfsm", &lfsm_with_source_locations(&[1]));
    let loose = run(&["inspect", "--max-object-bytes=4294967297", file.path_str()]);
    assert_eq!(loose.status.code(), Some(2));
    assert!(stderr(&loose).contains("只能收紧"));

    let loose_chunks = run(&["inspect", "--max-chunks-per-section=65537", file.path_str()]);
    assert_eq!(loose_chunks.status.code(), Some(2));
    assert!(stderr(&loose_chunks).contains("只能收紧"));

    // 收紧到实际大小以下同样失败（资源限制错误），收紧到之上则成功。
    let tight = run(&["inspect", "--max-object-bytes=16", file.path_str()]);
    assert_eq!(tight.status.code(), Some(2));
    let workable = run(&["inspect", "--max-object-bytes=16777216", file.path_str()]);
    assert_eq!(workable.status.code(), Some(0));
}

#[test]
fn malformed_filter_args_fail_loudly() {
    let file = TempFile::new("f.lfsm", &lfsm_with_source_locations(&[1]));
    for args in [
        vec!["inspect", "--rows=5..2", file.path_str()],
        vec!["inspect", "--rows=abc", file.path_str()],
        vec!["inspect", "--section=NoSuchSection", file.path_str()],
        vec!["inspect", "--table=NoSuchTable", file.path_str()],
        vec!["inspect", "--expand=everything", file.path_str()],
    ] {
        let output = run(&args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "参数 {args:?} 应报错退出 2，stderr: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains("筛选参数错误")
                || stderr(&output).contains("expand")
                || stderr(&output).contains("rows")
        );
    }
}

#[test]
fn binary_output_is_deterministic_across_runs() {
    let expected_bytes = lfsm_with_source_locations(&[3, 12]);
    let actual_bytes = lfsm_with_source_locations(&[3, 14]);
    let left = TempFile::new("det-a.lfsm", &expected_bytes);
    let right = TempFile::new("det-b.lfsm", &actual_bytes);
    let first = run(&["diff", left.path_str(), right.path_str()]);
    let second = run(&["diff", left.path_str(), right.path_str()]);
    assert_eq!(stdout(&first), stdout(&second), "同机两次运行正文须一致");
}

#[test]
fn diff_enforces_tightened_budget_consistently_with_inspect() {
    // 评审 #1 的 CLI 面：同一收紧预算下 inspect 与 diff 行为一致（都退 2）。
    let bytes = minimal_object(laneflow_static_contract::PortableObjectKind::CanonicalArtifact);
    let file = TempFile::new("budget.lfca", &bytes);
    let inspect = run(&["inspect", "--max-object-bytes=1024", file.path_str()]);
    assert_eq!(inspect.status.code(), Some(2));
    let diff = run(&[
        "diff",
        "--max-object-bytes=1024",
        file.path_str(),
        file.path_str(),
    ]);
    assert_eq!(
        diff.status.code(),
        Some(2),
        "diff 须与会话收紧预算一致地拒绝：{}",
        stdout(&diff)
    );
}
