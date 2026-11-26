//! 主干默认测试的只读影子对照。CI 仍照常执行该步骤。

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::{append_step_summary, workspace_tests};

const WORKFLOW_PATH: &str = ".github/workflows/ci.yml";
const DEFAULT_JOB: &str = "Rust default checks";
const TEST_STEP: &str = "Run workspace tests";

#[derive(Deserialize)]
struct Repository {
    full_name: String,
}

#[derive(Deserialize)]
struct Run {
    id: u64,
    run_attempt: u64,
    workflow_id: u64,
    event: String,
    path: String,
    head_sha: String,
    head_branch: String,
    repository: Repository,
    head_repository: Repository,
    conclusion: Option<String>,
}

impl Run {
    fn belongs_to(&self, repository: &str) -> bool {
        self.path == WORKFLOW_PATH
            && self.repository.full_name == repository
            && self.head_repository.full_name == repository
            && self.head_sha.len() == 40
            && self.head_sha.bytes().all(|c| c.is_ascii_hexdigit())
    }

    fn is_source_for(&self, main: &Self, repository: &str) -> bool {
        self.belongs_to(repository)
            && self.workflow_id == main.workflow_id
            && self.head_sha == main.head_sha
            && self.event == "merge_group"
            && self.head_branch.starts_with("gh-readonly-queue/main/")
            && self.conclusion.as_deref() == Some("success")
    }
}

#[derive(Deserialize)]
struct Runs {
    total_count: usize,
    workflow_runs: Vec<Run>,
}

#[derive(Deserialize)]
struct Step {
    name: String,
    conclusion: Option<String>,
}

#[derive(Deserialize)]
struct Job {
    id: u64,
    run_id: u64,
    run_attempt: u64,
    head_sha: String,
    name: String,
    conclusion: Option<String>,
    steps: Vec<Step>,
}

impl Job {
    fn step(&self, name: &str) -> Option<&str> {
        self.steps
            .iter()
            .find(|step| step.name == name)?
            .conclusion
            .as_deref()
    }

    fn nextest(&self) -> Option<&str> {
        let prefix = "Install cargo-nextest ";
        let step = self
            .steps
            .iter()
            .find(|step| step.name.starts_with(prefix))?;
        let version = step.name.strip_prefix(prefix)?;
        (step.conclusion.as_deref() == Some("success")
            && !version.is_empty()
            && version.bytes().all(|c| c.is_ascii_digit() || c == b'.')
            && self.step(&format!("Fetch cargo-nextest {version}")) == Some("success"))
        .then_some(version)
    }
}

#[derive(Deserialize)]
struct Jobs {
    jobs: Vec<Job>,
}

struct Github<'a> {
    repository: &'a str,
}

impl Github<'_> {
    fn get(&self, path: &str, subject: &str) -> Result<String, String> {
        let endpoint = format!("repos/{}/{path}", self.repository);
        let mut command = Command::new("gh");
        command.args(["api", &endpoint]);
        if path.ends_with("/logs") {
            // Runner 日志含终端颜色控制序列，子进程输出只供解析。
            command.arg("--allow-escape-sequences");
        }
        let result = command
            .output()
            .map_err(|_| "无法启动 GitHub CLI".to_owned())?;
        // 下载日志时 HTTP 错误可携带有时限的下载地址。
        if !result.status.success() {
            return Err(format!("GitHub API 无法读取{subject}"));
        }
        String::from_utf8(result.stdout).map_err(|_| format!("{subject}不是有效的 UTF-8"))
    }

    fn json<T: DeserializeOwned>(&self, path: &str, subject: &str) -> Result<T, String> {
        serde_json::from_str(&self.get(path, subject)?)
            .map_err(|_| format!("{subject}的 Actions 元数据格式无法识别"))
    }

    fn default_job(&self, run: &Run, event: &str) -> Result<Job, String> {
        let jobs: Jobs = self.json(
            &format!(
                "actions/runs/{}/attempts/{}/jobs?per_page=100",
                run.id, run.run_attempt
            ),
            &format!("{event}的 job"),
        )?;
        let mut defaults = jobs.jobs.into_iter().filter(|job| job.name == DEFAULT_JOB);
        let job = defaults
            .next()
            .ok_or_else(|| format!("{event}缺少独立的默认测试 job"))?;
        if defaults.next().is_some()
            || job.run_id != run.id
            || job.run_attempt != run.run_attempt
            || job.head_sha != run.head_sha
        {
            return Err(format!("{event}的默认测试 job 身份无法确认"));
        }
        if job.conclusion.as_deref() != Some("success") {
            return Err(format!("{event}的默认测试 job 没有成功记录"));
        }
        Ok(job)
    }
}

struct Configuration<'a> {
    runner: [&'a str; 2],
    rustc: &'a str,
    cargo: &'a str,
    command: &'a str,
    environment: BTreeMap<&'a str, &'a str>,
}

impl Configuration<'_> {
    fn differences(&self, other: &Self) -> Vec<&'static str> {
        [
            ("runner 镜像或版本", self.runner != other.runner),
            ("rustc 版本", self.rustc != other.rustc),
            ("Cargo 版本", self.cargo != other.cargo),
            ("默认测试命令", self.command != other.command),
            ("构建与测试环境", self.environment != other.environment),
        ]
        .into_iter()
        .filter_map(|(field, differs)| differs.then_some(field))
        .collect()
    }
}

struct WorkspaceLog<'a> {
    configuration: Configuration<'a>,
    passed: BTreeSet<&'a str>,
    skipped: usize,
    elapsed: f64,
}

fn runner_line(line: &str) -> &str {
    line.split_once("Z ").map_or(line, |(_, body)| body).trim()
}

fn field<'a>(lines: &[&'a str], prefix: &str) -> Option<&'a str> {
    let mut values = lines.iter().filter_map(|line| line.strip_prefix(prefix));
    let first = values.next()?;
    values.next().is_none().then_some(first)
}

impl<'a> WorkspaceLog<'a> {
    fn parse(log: &'a str, sha: &str) -> Result<Self, &'static str> {
        let lines: Vec<_> = log.lines().map(runner_line).collect();
        let start = lines
            .iter()
            .position(|line| {
                line.starts_with("##[group]Run cargo ") && line.contains(" -- run-workspace-tests ")
            })
            .ok_or("缺少默认测试的启动记录")?;
        let setup = &lines[..start];
        if !setup
            .windows(2)
            .any(|pair| pair[0].contains("git log -1 --format=%H") && pair[1] == sha)
        {
            return Err("checkout 日志无法确认与 Actions 运行相同的提交");
        }
        let image = setup
            .iter()
            .position(|line| line.starts_with("Image: ubuntu-"))
            .and_then(|i| {
                let version = *setup.get(i + 1)?;
                version
                    .starts_with("Version: ")
                    .then_some([setup[i], version])
            })
            .ok_or("缺少 runner 镜像信息")?;
        let rustc = field(setup, "rustc 1.").ok_or("缺少实际 rustc 版本")?;
        let cargo = field(setup, "cargo 1.").ok_or("缺少实际 Cargo 版本")?;
        let step = &lines[start + 1..];
        let env_end = step
            .iter()
            .position(|line| *line == "##[endgroup]")
            .ok_or("缺少默认测试的步骤环境")?;
        let (env, output) = (&step[..env_end], &step[env_end + 1..]);
        if field(env, "WORKSPACE_PACKAGES: ") != Some(workspace_tests::ALL) {
            return Err("默认测试步骤的包范围不是完整 workspace");
        }
        let output = output
            .split(|line| line.starts_with("##[group]Run "))
            .next()
            .ok_or("缺少默认测试输出")?;
        if field(output, "workspace 默认测试的 Cargo 构建参数：")
            != Some(workspace_tests::workspace_args().join(" ").as_str())
            || field(output, "workspace 默认测试的 nextest 过滤条件：")
                != Some(workspace_tests::DEFAULT_FILTER)
        {
            return Err("Cargo 参数或 nextest 过滤条件未证明完整默认测试范围");
        }
        let environment = env
            .iter()
            .filter_map(|line| line.split_once(": "))
            .filter(|(name, _)| {
                ["RUST", "CARGO_", "NEXTEST_"]
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
            })
            .collect();
        let results = TestResults::parse(output)?;
        Ok(Self {
            configuration: Configuration {
                runner: image,
                rustc,
                cargo,
                command: lines[start],
                environment,
            },
            passed: results.passed,
            skipped: results.skipped,
            elapsed: results.elapsed,
        })
    }
}

struct TestResults<'a> {
    passed: BTreeSet<&'a str>,
    skipped: usize,
    elapsed: f64,
}

impl<'a> TestResults<'a> {
    fn parse(output: &[&'a str]) -> Result<Self, &'static str> {
        let incomplete = "缺少完整且成功的 nextest 测试结果";
        let summary = field(output, "Summary [").ok_or(incomplete)?;
        let (elapsed, counts) = summary.split_once("s]").ok_or(incomplete)?;
        let elapsed = elapsed.trim().parse::<f64>().map_err(|_| incomplete)?;
        let (total, result) = counts.trim().split_once(" tests run: ").ok_or(incomplete)?;
        let total = total.parse::<usize>().map_err(|_| incomplete)?;
        let (passed, rest) = result.split_once(" passed").ok_or(incomplete)?;
        if total == 0
            || passed.parse::<usize>() != Ok(total)
            || !elapsed.is_finite()
            || elapsed < 0.0
        {
            return Err(incomplete);
        }
        let rest = if let Some(slow) = rest.strip_prefix(" (") {
            let (count, rest) = slow.split_once(" slow)").ok_or(incomplete)?;
            count.parse::<usize>().map_err(|_| incomplete)?;
            rest
        } else {
            rest
        };
        let skipped = if rest.is_empty() {
            0
        } else {
            rest.strip_prefix(", ")
                .and_then(|rest| rest.strip_suffix(" skipped"))
                .and_then(|count| count.parse::<usize>().ok())
                .ok_or(incomplete)?
        };
        let mut cases = BTreeSet::new();
        for (index, line) in output
            .iter()
            .filter_map(|line| line.strip_prefix("PASS ["))
            .enumerate()
        {
            let (_, verdict) = line.split_once("] (").ok_or(incomplete)?;
            let (ordinal, case) = verdict.split_once(") ").ok_or(incomplete)?;
            let (number, reported_total) = ordinal.split_once('/').ok_or(incomplete)?;
            if number.trim().parse::<usize>() != Ok(index + 1)
                || reported_total.trim().parse::<usize>() != Ok(total)
                || case.split_once(' ').is_none()
                || !cases.insert(case)
            {
                return Err(incomplete);
            }
        }
        if cases.len() != total {
            return Err(incomplete);
        }
        Ok(Self {
            passed: cases,
            skipped,
            elapsed,
        })
    }
}

fn audit(
    github: &Github<'_>,
    run_id: u64,
    attempt: u64,
    details: &mut Vec<String>,
) -> Result<&'static str, String> {
    let main: Run = github.json(
        &format!("actions/runs/{run_id}/attempts/{attempt}"),
        "待核对的 workflow run",
    )?;
    if !main.belongs_to(github.repository)
        || main.id != run_id
        || main.run_attempt != attempt
        || main.event != "push"
        || main.head_branch != "main"
    {
        return Err("输入运行不是本仓库 CI 的 main push".to_owned());
    }
    details.push(format!("- 提交 `{}`。", main.head_sha));
    let main_job = github.default_job(&main, "main")?;
    details.push(format!(
        "- [main 默认检查](https://github.com/{}/actions/runs/{run_id}/job/{})（第 {attempt} 次尝试）。",
        github.repository, main_job.id
    ));
    match main_job.step(TEST_STEP) {
        Some("skipped") => return Ok("不适用。本次 main 默认测试步骤已由路径门跳过。"),
        Some("success") => {}
        _ => return Err("main 的默认测试步骤没有成功记录".to_owned()),
    }
    let candidates: Runs = github.json(
        &format!(
            "actions/workflows/{}/runs?event=merge_group&head_sha={}&per_page=2",
            main.workflow_id, main.head_sha,
        ),
        "同提交的合并组运行",
    )?;
    if candidates.total_count == 0 {
        return Err("未找到同提交的合并组 CI".to_owned());
    }
    let [group] = candidates.workflow_runs.as_slice() else {
        return Err("同提交的合并组 CI 无法唯一确认".to_owned());
    };
    if candidates.total_count != 1 || !group.is_source_for(&main, github.repository) {
        return Err("合并组 CI 尚未成功或仓库、工作流、提交身份不符".to_owned());
    }
    let group_job = github.default_job(group, "合并组")?;
    details.push(format!(
        "- [合并组默认检查](https://github.com/{}/actions/runs/{}/job/{})（第 {} 次尝试）。",
        github.repository, group.id, group_job.id, group.run_attempt,
    ));
    if group_job.step(TEST_STEP) != Some("success") {
        return Err("合并组的默认测试步骤没有成功记录".to_owned());
    }
    let nextest = main_job.nextest().ok_or("无法确认 main 的 nextest 版本")?;
    if group_job.nextest() != Some(nextest) {
        return Err("两边的 nextest 安装记录或版本不同".to_owned());
    }
    let load_log = |job: &Job| {
        let path = format!("actions/jobs/{}/logs", job.id);
        let subject = format!(" job {} 的日志", job.id);
        github
            .get(&path, &subject)
            .or_else(|_| github.get(&path, &subject))
    };
    let main_log = load_log(&main_job)?;
    let group_log = load_log(&group_job)?;
    let main =
        WorkspaceLog::parse(&main_log, &main.head_sha).map_err(|error| format!("main {error}"))?;
    let group = WorkspaceLog::parse(&group_log, &group.head_sha)
        .map_err(|error| format!("合并组{error}"))?;
    let differences = main.configuration.differences(&group.configuration);
    if !differences.is_empty() {
        return Err(format!("两边的 {} 不同", differences.join("、")));
    }
    if main.passed != group.passed || main.skipped != group.skipped {
        return Err("通过用例清单或跳过用例数量不同".to_owned());
    }
    details.push(format!(
        "- 镜像及版本、Rust / Cargo、nextest {nextest}、完整包范围、过滤条件和日志中的构建与测试环境一致。"
    ));
    details.push(format!(
        "- 通过的 {} 项用例逐项一致，两边均跳过 {} 项。main 实际测试执行 {:.3} 秒。",
        main.passed.len(),
        main.skipped,
        main.elapsed,
    ));
    Ok("可复用。提交、观测到的配置和完整默认测试结果一致。")
}

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let [run_id, attempt] = args else {
        return Err("audit-workspace-test-reuse 需要 workflow run ID 与 attempt".to_owned());
    };
    let run_id = run_id
        .parse::<u64>()
        .map_err(|_| "无效的 workflow run ID")?;
    let attempt = attempt
        .parse::<u64>()
        .map_err(|_| "无效的 workflow run attempt")?;
    let repository = std::env::var("GITHUB_REPOSITORY").map_err(|_| "缺少 GITHUB_REPOSITORY")?;
    let github = Github {
        repository: &repository,
    };
    let mut details = Vec::new();
    let decision = match audit(&github, run_id, attempt, &mut details) {
        Ok(decision) => decision.to_owned(),
        Err(reason) => format!("回退。{reason}。"),
    };
    if std::env::var_os("GITHUB_STEP_SUMMARY").is_some() {
        println!("{decision}");
    }
    let report = format!(
        "### workspace 默认测试复用影子判定\n\n{decision}\n\n{}",
        details.join("\n")
    );
    append_step_summary(&report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0ebc8a2f1cf5d31120038cc9963df4ae3b866b20";
    const LOG: &str = "\
Image: ubuntu-26.04
Version: 20261116.88.1
[command]/usr/bin/git log -1 --format=%H
0ebc8a2f1cf5d31120038cc9963df4ae3b866b20
rustc 1.99.1 (58cf8f9c3 2026-10-13)
cargo 1.99.1 (5f94df478 2026-08-27)
##[group]Run cargo run --quiet --locked -p xtask -- run-workspace-tests \"$WORKSPACE_PACKAGES\"
env:
  WORKSPACE_PACKAGES: *
  RUSTFLAGS: -C link-arg=-fuse-ld=lld
  SCCACHE_GHA_RW_MODE: READ_ONLY
##[endgroup]
workspace 默认测试的 Cargo 构建参数：--workspace --exclude laneflow-urban-harness --exclude laneflow-urban-generator
workspace 默认测试的 nextest 过滤条件：not (binary(corridor_traffic_smoke))
  Starting 2 tests across 1 binary (91 tests and 1 binary skipped)
     PASS [   1.000s] (   1/2) laneflow-example test::first
     PASS [   2.000s] (   2/2) laneflow-example test::second
  Summary [   3.000s] 2 tests run: 2 passed (1 slow), 91 skipped
##[group]Run cargo test --workspace --doc --locked
  Summary [   2.000s] 1 test run: 1 passed";

    #[test]
    fn workspace_logs_compare_config_and_passed_cases() {
        let main = LOG.replace("READ_ONLY", "READ_WRITE");
        let group = WorkspaceLog::parse(LOG, SHA).unwrap();
        assert_eq!(group.passed.len(), 2);
        assert_eq!(group.skipped, 91);
        let log = WorkspaceLog::parse(&main, SHA).unwrap();
        assert!(
            group
                .configuration
                .differences(&log.configuration)
                .is_empty()
        );
        assert_eq!(group.passed, log.passed);

        let changed = main.replace("Version: 20261116.88.1", "Version: 20261116.202.1");
        let log = WorkspaceLog::parse(&changed, SHA).unwrap();
        assert_eq!(
            group.configuration.differences(&log.configuration),
            ["runner 镜像或版本"]
        );
        let changed = main.replace("test::second", "test::third");
        let log = WorkspaceLog::parse(&changed, SHA).unwrap();
        assert_ne!(group.passed, log.passed);
    }

    #[test]
    fn workspace_logs_require_full_scope_checkout_and_complete_results() {
        for (from, to) in [
            (
                "WORKSPACE_PACKAGES: *",
                "WORKSPACE_PACKAGES: laneflow-runtime",
            ),
            ("--workspace --exclude", "-p laneflow-runtime --exclude"),
            ("test::second", "test::first"),
            ("PASS [   1.000s]", "FAIL [   1.000s]"),
            ("2 passed (1 slow)", "1 passed, 1 failed"),
            (SHA, "6ed1fe7d54f6646b6a9f984effad57ad1e5d7f97"),
        ] {
            assert!(
                WorkspaceLog::parse(&LOG.replace(from, to), SHA).is_err(),
                "{from}"
            );
        }
    }
}
