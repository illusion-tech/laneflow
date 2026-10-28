//! 经真实 Git 提交与 xtask 二进制验证文件/range 入口的一致性（#1128）。

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TestRepository(PathBuf);

impl TestRepository {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "laneflow-harness-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let repo = Self(path);
        repo.git(&["init", "--quiet"]);
        repo
    }

    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(&self.0)
            .args([
                "-c",
                "user.name=Harness Test",
                "-c",
                "user.email=harness@example.com",
            ])
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.0.join("absent-config"))
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {}", stderr(&output));
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    fn commit(&self, message: &str, parent: Option<&str>) -> String {
        let tree = self.git(&["mktree"]);
        let mut args = vec!["commit-tree", tree.as_str(), "-m", message];
        if let Some(parent) = parent {
            args.extend(["-p", parent]);
        }
        self.git(&args)
    }

    fn xtask(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .current_dir(&self.0)
            .args(args)
            .output()
            .unwrap()
    }
}

impl Drop for TestRepository {
    fn drop(&mut self) {
        // 只清理本测试通过 create_dir 独占创建的临时仓库。
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn registered_harnesses_use_the_same_policy_in_file_and_real_commit_range() {
    let repo = TestRepository::new();
    for (harness, accepted) in [
        ("openai/codex/cli", true),
        ("openai/codex/web", true),
        ("openai/codex/desktop", true),
        ("openai/codex/ide-extension", true),
        ("anysphere/cursor/ide", true),
        ("Codex", false),
        ("Codex CLI", false),
        ("OPENAI/CODEX/CLI", false),
        ("openai/codex/ide", false),
        ("anysphere/cursor/ide-extension", false),
        ("vendor/unregistered/cli", false),
    ] {
        let message = format!(
            "docs(governance): 验证执行框架登记\n\nHarness: {harness}\nModel: openai/gpt-6.1-sol\nRefs: #1128\n"
        );
        std::fs::write(repo.0.join("message.txt"), &message).unwrap();
        let commit = repo.commit(&message, None);
        let file = repo.xtask(&["check-commit-message-file", "message.txt"]);
        let range = repo.xtask(&["check-commit-messages", &format!("{commit}^!")]);
        for output in [&file, &range] {
            assert_eq!(
                output.status.success(),
                accepted,
                "{harness}: {}",
                stderr(output)
            );
            if !accepted {
                assert!(
                    stderr(output).contains("未登记为规范 ID"),
                    "{}",
                    stderr(output)
                );
            }
        }
    }
}

#[test]
fn file_entry_keeps_git_comment_and_crlf_handling() {
    let repo = TestRepository::new();
    std::fs::write(
        repo.0.join("message.txt"),
        "docs(governance): 验证执行框架登记\r\n\r\nHarness: openai/codex/desktop\r\nModel: openai/gpt-6.1-sol\r\nRefs: #1128\r\n# Git 注释\r\n",
    ).unwrap();
    let output = repo.xtask(&["check-commit-message-file", "message.txt"]);
    assert!(output.status.success(), "{}", stderr(&output));
}

#[test]
fn range_checks_only_selected_new_commits_and_human_pair_remains_optional() {
    let repo = TestRepository::new();
    let old = repo.commit(
        "docs(governance): 历史提交\n\nHarness: Codex\nModel: openai/gpt-6.1-sol\nRefs: #1128\n",
        None,
    );
    let human = "docs(governance): 人工提交\n\nRefs: #1128\n";
    let new = repo.commit(human, Some(&old));
    std::fs::write(repo.0.join("message.txt"), human).unwrap();
    for args in [
        vec![
            "check-commit-message-file".to_string(),
            "message.txt".to_string(),
        ],
        vec!["check-commit-messages".to_string(), format!("{old}..{new}")],
    ] {
        let output = repo.xtask(&args.iter().map(String::as_str).collect::<Vec<_>>());
        assert!(output.status.success(), "{}", stderr(&output));
    }
    let output = repo.xtask(&["check-commit-messages", &format!("{old}^!")]);
    assert!(
        !output.status.success(),
        "legacy alias must fail in selected range"
    );
}
