use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::lockfile_policy::{
    self, ChangedFile, CommitAuthor, PullRequestCommit, PullRequestMetadata,
};

const SNAPSHOT_SCHEMA_VERSION: u64 = 1;
const RESULT_SCHEMA_VERSION: u64 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CodeQlState {
    Pass,
    NotApplicable,
    Pending,
    Failed,
    Missing,
    ProviderError,
}

impl CodeQlState {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "pass" => Ok(Self::Pass),
            "not_applicable" => Ok(Self::NotApplicable),
            "pending" => Ok(Self::Pending),
            "failed" => Ok(Self::Failed),
            "missing" => Ok(Self::Missing),
            "provider_error" => Ok(Self::ProviderError),
            _ => Err(format!(
                "未知 CodeQL 状态 `{value}`；应为 pass、not_applicable、pending、failed、missing 或 provider_error"
            )),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::NotApplicable => "not_applicable",
            Self::Pending => "pending",
            Self::Failed => "failed",
            Self::Missing => "missing",
            Self::ProviderError => "provider_error",
        }
    }

    pub(crate) fn satisfies_g3(self) -> bool {
        matches!(self, Self::Pass | Self::NotApplicable)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CodeQlSnapshot {
    schema_version: u64,
    repository: String,
    pull_request: PullRequestSnapshot,
    #[serde(default)]
    provider_errors: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestSnapshot {
    number: u64,
    #[serde(default)]
    author: Option<Actor>,
    head_ref_oid: String,
    base_ref_oid: String,
    url: String,
    #[serde(default)]
    is_draft: bool,
    state: String,
    #[serde(default)]
    files: Vec<ChangedFileSnapshot>,
    #[serde(default)]
    commits: Vec<CommitSnapshot>,
    #[serde(default)]
    status_check_rollup: Vec<CheckRunSnapshot>,
    #[serde(default = "default_true")]
    files_complete: bool,
    #[serde(default = "default_true")]
    commits_complete: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Actor {
    login: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChangedFileSnapshot {
    path: String,
    change_type: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CommitSnapshot {
    oid: String,
    committed_date: String,
    message_headline: String,
    #[serde(default)]
    authors: Vec<CommitAuthorSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct CommitAuthorSnapshot {
    #[serde(default)]
    login: Option<String>,
    name: String,
    email: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckRunSnapshot {
    #[serde(rename = "__typename", default)]
    typename: String,
    name: String,
    status: String,
    #[serde(default)]
    conclusion: String,
    #[serde(default)]
    details_url: String,
    #[serde(default)]
    app_slug: String,
    #[serde(default)]
    pull_requests: Vec<CheckPullRequestSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckPullRequestSnapshot {
    number: u64,
    head_oid: String,
    base_oid: String,
}

#[derive(Deserialize)]
struct RestCheckRuns {
    total_count: usize,
    check_runs: Vec<RestCheckRun>,
}

#[derive(Deserialize)]
struct RestCheckRun {
    name: String,
    status: String,
    conclusion: Option<String>,
    details_url: String,
    head_sha: String,
    app: RestCheckApp,
    #[serde(default)]
    pull_requests: Vec<RestCheckPullRequest>,
}

#[derive(Deserialize)]
struct RestCheckApp {
    slug: String,
}

#[derive(Deserialize)]
struct RestCheckPullRequest {
    number: u64,
    head: RestGitRef,
    base: RestGitRef,
}

#[derive(Deserialize)]
struct RestGitRef {
    sha: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodeQlResult {
    schema_version: u64,
    repository: String,
    pull_request: u64,
    current_head_oid: String,
    current_base_oid: String,
    pub(crate) state: CodeQlState,
    evidence_url: Option<String>,
    policy: Option<String>,
    diagnostics: Vec<String>,
}

impl CodeQlResult {
    pub(crate) fn evidence_url(&self) -> Option<&str> {
        self.evidence_url.as_deref()
    }

    pub(crate) fn policy(&self) -> Option<&str> {
        self.policy.as_deref()
    }
}

enum InputSource {
    Live {
        repository: String,
        pr: u64,
        evidence_url: Option<String>,
    },
    Fixture(PathBuf),
}

enum OutputFormat {
    Human,
    Json,
}

struct Args {
    input: InputSource,
    format: OutputFormat,
    expected: Option<CodeQlState>,
}

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let args = parse_args(args)?;
    let result = match args.input {
        InputSource::Live {
            repository,
            pr,
            evidence_url,
        } => match evidence_url {
            Some(evidence_url) => evaluate_live_recorded(&repository, pr, &evidence_url)?,
            None => evaluate_live(&repository, pr)?,
        },
        InputSource::Fixture(path) => {
            let contents = fs::read_to_string(&path)
                .map_err(|error| format!("无法读取 CodeQL fixture {}：{error}", path.display()))?;
            let snapshot = serde_json::from_str::<CodeQlSnapshot>(&contents)
                .map_err(|error| format!("CodeQL fixture JSON 无效：{error}"))?;
            evaluate_snapshot(&snapshot)
        }
    };

    match args.format {
        OutputFormat::Human => print_summary(&result),
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&result)
                .map_err(|error| format!("无法序列化 CodeQL 结果：{error}"))?
        ),
    }

    if let Some(expected) = args.expected {
        if result.state == expected {
            return Ok(());
        }
        return Err(format!(
            "CodeQL fixture 状态不匹配：期望 `{}`，实际 `{}`",
            expected.as_str(),
            result.state.as_str()
        ));
    }
    if result.state.satisfies_g3() {
        Ok(())
    } else {
        Err(format!("CodeQL 未满足 G3：{}", result.state.as_str()))
    }
}

pub(crate) fn evaluate_live(repository: &str, pr: u64) -> Result<CodeQlResult, String> {
    evaluate_live_with_evidence(repository, pr, None)
}

pub(crate) fn evaluate_live_recorded(
    repository: &str,
    pr: u64,
    evidence_url: &str,
) -> Result<CodeQlResult, String> {
    evaluate_live_with_evidence(repository, pr, Some(evidence_url))
}

fn evaluate_live_with_evidence(
    repository: &str,
    pr: u64,
    evidence_url: Option<&str>,
) -> Result<CodeQlResult, String> {
    let snapshot = load_live_snapshot(repository, pr, evidence_url)?;
    let initial_head = snapshot.pull_request.head_ref_oid.clone();
    let initial_base = snapshot.pull_request.base_ref_oid.clone();
    let mut result = evaluate_snapshot(&snapshot);
    let identity = load_live_identity(repository, pr)?;
    if identity.head_ref_oid != initial_head || identity.base_ref_oid != initial_base {
        result.state = CodeQlState::ProviderError;
        result.evidence_url = None;
        result.policy = None;
        result.diagnostics.push(format!(
            "head/base 竞态：首次读取 {initial_head}/{initial_base}，复核 {}/{}",
            identity.head_ref_oid, identity.base_ref_oid
        ));
    }
    Ok(result)
}

fn evaluate_snapshot(snapshot: &CodeQlSnapshot) -> CodeQlResult {
    let pr = &snapshot.pull_request;
    let mut diagnostics = snapshot.provider_errors.clone();
    if snapshot.schema_version != SNAPSHOT_SCHEMA_VERSION {
        diagnostics.push(format!(
            "snapshot schemaVersion 必须为 {SNAPSHOT_SCHEMA_VERSION}，实际为 {}",
            snapshot.schema_version
        ));
    }
    if !valid_repository_name(&snapshot.repository) {
        diagnostics.push(format!("repository 格式不正确：{}", snapshot.repository));
    }
    if pr.number == 0 {
        diagnostics.push("pullRequest.number 必须是正整数".to_string());
    }
    if !valid_full_oid(&pr.head_ref_oid) || !valid_full_oid(&pr.base_ref_oid) {
        diagnostics.push("headRefOid/baseRefOid 必须是 40 位十六进制 OID".to_string());
    }
    if !valid_github_url(&pr.url) {
        diagnostics.push("pullRequest.url 必须是 GitHub HTTPS URL".to_string());
    }
    if pr.is_draft || !matches!(pr.state.as_str(), "OPEN" | "MERGED") {
        diagnostics
            .push("CodeQL G3 只接受 OPEN 或带历史 checks 的 MERGED、非 Draft PR".to_string());
    }

    let lockfile_metadata = lockfile_metadata(pr);
    let lockfile_eligibility = lockfile_policy::verify_dependabot_lockfile_only(&lockfile_metadata);
    let aggregate = pr
        .status_check_rollup
        .iter()
        .filter(|check| {
            check.typename == "CheckRun"
                && check.name == "CodeQL"
                && check.app_slug == "github-advanced-security"
                && (check.pull_requests.iter().any(|association| {
                    association.number == pr.number
                        && association.head_oid == pr.head_ref_oid
                        && association.base_oid == pr.base_ref_oid
                }) || (pr.state == "MERGED" && check.pull_requests.is_empty()))
        })
        .collect::<Vec<_>>();
    if aggregate.len() > 1 {
        diagnostics.push("current head 存在多个 CodeQL aggregate Check，无法唯一判定".to_string());
    }

    let (state, evidence_url, policy, state_diagnostic) = if !diagnostics.is_empty() {
        (CodeQlState::ProviderError, None, None, None)
    } else if let Some(check) = aggregate.first() {
        if !valid_github_url(&check.details_url) {
            (
                CodeQlState::ProviderError,
                None,
                None,
                Some("CodeQL aggregate Check 缺少有效 GitHub evidence URL".to_string()),
            )
        } else if check.status != "COMPLETED" {
            (
                CodeQlState::Pending,
                Some(check.details_url.clone()),
                None,
                Some("CodeQL aggregate Check 尚未完成".to_string()),
            )
        } else if check.conclusion == "SUCCESS" {
            (
                CodeQlState::Pass,
                Some(check.details_url.clone()),
                Some("codeql-current-head-analysis".to_string()),
                None,
            )
        } else if matches!(check.conclusion.as_str(), "NEUTRAL" | "SKIPPED") {
            match lockfile_eligibility {
                Ok(_) => (
                    CodeQlState::NotApplicable,
                    Some(check.details_url.clone()),
                    Some("dependabot-cargo-lock-only-v1".to_string()),
                    Some(
                        "CodeQL neutral/no-analysis；精确 Dependabot Cargo.lock-only policy 判定为不适用"
                            .to_string(),
                    ),
                ),
                Err(reason) => (
                    CodeQlState::Failed,
                    Some(check.details_url.clone()),
                    None,
                    Some(format!(
                        "CodeQL neutral/no-analysis 且不满足 lockfile-only 替代规则：{reason}"
                    )),
                ),
            }
        } else {
            (
                CodeQlState::Failed,
                Some(check.details_url.clone()),
                None,
                Some(format!(
                    "CodeQL aggregate Check conclusion={}，不能进入 G3",
                    check.conclusion
                )),
            )
        }
    } else {
        match lockfile_eligibility {
            Ok(_) => (
                CodeQlState::NotApplicable,
                Some(pr.url.clone()),
                Some("dependabot-cargo-lock-only-v1".to_string()),
                Some(
                    "current head 未生成 CodeQL aggregate Check；精确 Dependabot Cargo.lock-only policy 判定为不适用"
                        .to_string(),
                ),
            ),
            Err(reason) => (
                CodeQlState::Missing,
                None,
                None,
                Some(format!("current head 缺少 CodeQL aggregate Check：{reason}")),
            ),
        }
    };
    if let Some(diagnostic) = state_diagnostic {
        diagnostics.push(diagnostic);
    }

    CodeQlResult {
        schema_version: RESULT_SCHEMA_VERSION,
        repository: snapshot.repository.clone(),
        pull_request: pr.number,
        current_head_oid: pr.head_ref_oid.clone(),
        current_base_oid: pr.base_ref_oid.clone(),
        state,
        evidence_url,
        policy,
        diagnostics,
    }
}

fn lockfile_metadata(pr: &PullRequestSnapshot) -> PullRequestMetadata {
    PullRequestMetadata {
        author_login: pr
            .author
            .as_ref()
            .map(|author| author.login.clone())
            .unwrap_or_default(),
        head_oid: pr.head_ref_oid.clone(),
        files: pr
            .files
            .iter()
            .map(|file| ChangedFile {
                path: file.path.clone(),
                change_type: file.change_type.clone(),
            })
            .collect(),
        commits: pr
            .commits
            .iter()
            .map(|commit| PullRequestCommit {
                oid: commit.oid.clone(),
                committed_at: commit.committed_date.clone(),
                url: format!("{}/commits/{}", pr.url, commit.oid),
                message_headline: commit.message_headline.clone(),
                authors: commit
                    .authors
                    .iter()
                    .map(|author| CommitAuthor {
                        login: author.login.clone(),
                        name: author.name.clone(),
                        email: author.email.clone(),
                    })
                    .collect(),
            })
            .collect(),
        files_complete: pr.files_complete,
        commits_complete: pr.commits_complete,
    }
}

fn load_live_snapshot(
    repository: &str,
    pr: u64,
    recorded_evidence_url: Option<&str>,
) -> Result<CodeQlSnapshot, String> {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            &pr.to_string(),
            "--repo",
            repository,
            "--json",
            "number,author,headRefOid,baseRefOid,url,isDraft,state,files,commits,statusCheckRollup",
        ])
        .output()
        .map_err(|error| format!("无法启动 gh pr view：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh pr view 读取 CodeQL snapshot 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let mut pull_request = serde_json::from_slice::<PullRequestSnapshot>(&output.stdout)
        .map_err(|error| format!("无法解析 gh CodeQL snapshot：{error}"))?;
    pull_request.status_check_rollup = match recorded_evidence_url {
        Some(evidence_url) if evidence_url == pull_request.url => Vec::new(),
        Some(evidence_url) => vec![load_recorded_codeql_check_run(
            repository,
            &pull_request.head_ref_oid,
            evidence_url,
        )?],
        None => load_codeql_check_runs(
            repository,
            &pull_request.head_ref_oid,
            std::mem::take(&mut pull_request.status_check_rollup),
        )?,
    };
    Ok(CodeQlSnapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        repository: repository.to_string(),
        pull_request,
        provider_errors: Vec::new(),
    })
}

fn load_codeql_check_runs(
    repository: &str,
    head_oid: &str,
    mut pr_bound_checks: Vec<CheckRunSnapshot>,
) -> Result<Vec<CheckRunSnapshot>, String> {
    let endpoint = format!(
        "repos/{repository}/commits/{head_oid}/check-runs?check_name=CodeQL&filter=latest&per_page=100"
    );
    let output = Command::new("gh")
        .args(["api", &endpoint])
        .output()
        .map_err(|error| format!("无法启动 gh api CodeQL check-runs：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh api 读取 CodeQL check-runs 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let response = serde_json::from_slice::<RestCheckRuns>(&output.stdout)
        .map_err(|error| format!("无法解析 CodeQL check-runs API：{error}"))?;
    if response.total_count != response.check_runs.len() {
        return Err(format!(
            "CodeQL check-runs 响应被截断：total_count={}，returned={}",
            response.total_count,
            response.check_runs.len()
        ));
    }
    let rest_checks = response
        .check_runs
        .into_iter()
        .map(|check| rest_check_run_snapshot(check, head_oid))
        .collect::<Result<Vec<_>, _>>()?;
    for check in &mut pr_bound_checks {
        if check.typename != "CheckRun" || check.name != "CodeQL" {
            continue;
        }
        let matches = rest_checks
            .iter()
            .filter(|candidate| candidate.details_url == check.details_url)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(format!(
                "PR-bound CodeQL rollup 无法唯一匹配 REST check-run：detailsUrl={}，matches={}",
                check.details_url,
                matches.len()
            ));
        }
        let trusted = matches[0];
        if check.status.to_ascii_uppercase() != trusted.status
            || check.conclusion.to_ascii_uppercase() != trusted.conclusion
        {
            return Err(format!(
                "PR-bound CodeQL rollup 与 REST check-run 状态不一致：detailsUrl={}",
                check.details_url
            ));
        }
        check.app_slug.clone_from(&trusted.app_slug);
        check.pull_requests.clone_from(&trusted.pull_requests);
    }
    Ok(pr_bound_checks)
}

fn load_recorded_codeql_check_run(
    repository: &str,
    head_oid: &str,
    evidence_url: &str,
) -> Result<CheckRunSnapshot, String> {
    let check_run_id = recorded_check_run_id(repository, evidence_url)?;
    let endpoint = format!("repos/{repository}/check-runs/{check_run_id}");
    let output = Command::new("gh")
        .args(["api", &endpoint])
        .output()
        .map_err(|error| format!("无法启动 gh api recorded CodeQL check-run：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh api 读取 recorded CodeQL check-run 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let check = serde_json::from_slice::<RestCheckRun>(&output.stdout)
        .map_err(|error| format!("无法解析 recorded CodeQL check-run API：{error}"))?;
    if check.details_url != evidence_url {
        return Err("recorded CodeQL check-run 的 details URL 与 G3 evidence 不一致".to_string());
    }
    rest_check_run_snapshot(check, head_oid)
}

fn rest_check_run_snapshot(
    check: RestCheckRun,
    expected_head_oid: &str,
) -> Result<CheckRunSnapshot, String> {
    if check.head_sha != expected_head_oid {
        return Err(format!(
            "CodeQL check-run head 不属于目标 PR：期望 {expected_head_oid}，实际 {}",
            check.head_sha
        ));
    }
    Ok(CheckRunSnapshot {
        typename: "CheckRun".to_string(),
        name: check.name,
        status: check.status.to_ascii_uppercase(),
        conclusion: check.conclusion.unwrap_or_default().to_ascii_uppercase(),
        details_url: check.details_url,
        app_slug: check.app.slug,
        pull_requests: check
            .pull_requests
            .into_iter()
            .map(|association| CheckPullRequestSnapshot {
                number: association.number,
                head_oid: association.head.sha,
                base_oid: association.base.sha,
            })
            .collect(),
    })
}

fn recorded_check_run_id(repository: &str, evidence_url: &str) -> Result<u64, String> {
    let prefix = format!("https://github.com/{repository}/runs/");
    let value = evidence_url
        .strip_prefix(&prefix)
        .ok_or_else(|| format!("recorded CodeQL evidence URL 必须匹配 `{prefix}<check-run-id>`"))?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("recorded CodeQL evidence URL 必须以纯数字 check-run ID 结尾".to_string());
    }
    value
        .parse::<u64>()
        .map_err(|error| format!("recorded CodeQL check-run ID 无效：{error}"))
}

fn load_live_identity(repository: &str, pr: u64) -> Result<PullRequestIdentity, String> {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            &pr.to_string(),
            "--repo",
            repository,
            "--json",
            "headRefOid,baseRefOid",
        ])
        .output()
        .map_err(|error| format!("无法启动 gh pr view identity：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh pr view 复核 CodeQL identity 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("无法解析 gh CodeQL identity：{error}"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestIdentity {
    head_ref_oid: String,
    base_ref_oid: String,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut repository = None;
    let mut pr = None;
    let mut input = None;
    let mut evidence_url = None;
    let mut format = OutputFormat::Human;
    let mut expected = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--repo" => repository = Some(next_value(args, &mut index, "--repo")?),
            "--pr" => {
                pr = Some(
                    next_value(args, &mut index, "--pr")?
                        .parse::<u64>()
                        .map_err(|error| format!("--pr 必须是正整数：{error}"))?,
                )
            }
            "--input" => input = Some(PathBuf::from(next_value(args, &mut index, "--input")?)),
            "--evidence-url" => {
                evidence_url = Some(next_value(args, &mut index, "--evidence-url")?)
            }
            "--format" => {
                format = match next_value(args, &mut index, "--format")?.as_str() {
                    "human" => OutputFormat::Human,
                    "json" => OutputFormat::Json,
                    value => return Err(format!("未知 --format `{value}`")),
                }
            }
            "--expect" => {
                expected = Some(CodeQlState::parse(&next_value(
                    args, &mut index, "--expect",
                )?)?)
            }
            value => return Err(format!("未知 check-codeql 参数：{value}")),
        }
        index += 1;
    }
    let source = match (repository, pr, input) {
        (Some(repository), Some(pr), None) if pr > 0 => InputSource::Live {
            repository,
            pr,
            evidence_url,
        },
        (None, None, Some(path)) if evidence_url.is_none() => InputSource::Fixture(path),
        _ => {
            return Err(
                "用法：check-codeql (--repo <owner/repo> --pr <number> [--evidence-url <recorded-url>] | --input <snapshot.json>) [--format human|json] [--expect <state>]"
                    .to_string(),
            )
        }
    };
    Ok(Args {
        input: source,
        format,
        expected,
    })
}

fn next_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} 缺少值"))
}

fn print_summary(result: &CodeQlResult) {
    println!(
        "CodeQL: state={} head={} policy={} evidence={}",
        result.state.as_str(),
        result.current_head_oid,
        result.policy.as_deref().unwrap_or("N/A"),
        result.evidence_url.as_deref().unwrap_or("N/A")
    );
    for diagnostic in &result.diagnostics {
        println!("- {diagnostic}");
    }
}

fn valid_repository_name(value: &str) -> bool {
    value
        .split_once('/')
        .is_some_and(|(owner, name)| !owner.is_empty() && !name.is_empty() && !name.contains('/'))
}

fn valid_full_oid(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn valid_github_url(value: &str) -> bool {
    value.starts_with("https://github.com/") && !value.chars().any(char::is_whitespace)
}

const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(contents: &str) -> CodeQlSnapshot {
        serde_json::from_str(contents).expect("fixture must match CodeQL snapshot schema")
    }

    #[test]
    fn replays_codeql_applicability_fixtures() {
        for (contents, expected) in [
            (
                include_str!("../fixtures/codeql/source-success.json"),
                CodeQlState::Pass,
            ),
            (
                include_str!("../fixtures/codeql/lockfile-neutral.json"),
                CodeQlState::NotApplicable,
            ),
            (
                include_str!("../fixtures/codeql/lockfile-no-analysis.json"),
                CodeQlState::NotApplicable,
            ),
            (
                include_str!("../fixtures/codeql/source-neutral.json"),
                CodeQlState::Failed,
            ),
            (
                include_str!("../fixtures/codeql/lockfile-wrong-author.json"),
                CodeQlState::Failed,
            ),
            (
                include_str!("../fixtures/codeql/source-spoofed-codeql.json"),
                CodeQlState::Missing,
            ),
        ] {
            assert_eq!(evaluate_snapshot(&fixture(contents)).state, expected);
        }
    }

    #[test]
    fn rejects_codeql_runs_from_another_pr_or_base() {
        let mut snapshot = fixture(include_str!("../fixtures/codeql/source-success.json"));
        snapshot.pull_request.status_check_rollup[0].pull_requests[0].number = 330;
        assert_eq!(evaluate_snapshot(&snapshot).state, CodeQlState::Missing);

        let mut snapshot = fixture(include_str!("../fixtures/codeql/source-success.json"));
        snapshot.pull_request.status_check_rollup[0].pull_requests[0].base_oid =
            "cccccccccccccccccccccccccccccccccccccccc".to_string();
        assert_eq!(evaluate_snapshot(&snapshot).state, CodeQlState::Missing);

        let mut snapshot = fixture(include_str!("../fixtures/codeql/source-success.json"));
        snapshot.pull_request.state = "MERGED".to_string();
        snapshot.pull_request.status_check_rollup[0].pull_requests[0].number = 330;
        assert_eq!(evaluate_snapshot(&snapshot).state, CodeQlState::Missing);
    }

    #[test]
    fn accepts_pr_bound_merged_run_when_github_omits_rest_association() {
        let mut snapshot = fixture(include_str!("../fixtures/codeql/source-success.json"));
        snapshot.pull_request.state = "MERGED".to_string();
        snapshot.pull_request.status_check_rollup[0]
            .pull_requests
            .clear();
        assert_eq!(evaluate_snapshot(&snapshot).state, CodeQlState::Pass);
    }

    #[test]
    fn parses_only_exact_repository_check_run_urls() {
        assert_eq!(
            recorded_check_run_id(
                "illusion-tech/laneflow",
                "https://github.com/illusion-tech/laneflow/runs/92519398933"
            ),
            Ok(92_519_398_933)
        );
        assert!(
            recorded_check_run_id(
                "illusion-tech/laneflow",
                "https://github.com/other/laneflow/runs/92519398933"
            )
            .is_err()
        );
        assert!(
            recorded_check_run_id(
                "illusion-tech/laneflow",
                "https://github.com/illusion-tech/laneflow/runs/92519398933?attempt=2"
            )
            .is_err()
        );
    }
}
