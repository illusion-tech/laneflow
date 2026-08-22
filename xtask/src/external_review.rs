//! External Review Check: trusted non-author `APPROVED`/`COMMENTED` on the PR
//! (any commit), or a 👍 (`+1`) on the PR body. Merge-group stamps onto `H_mg`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

const CHECK_NAME: &str = "External Review";
const CHECK_APP_SLUG: &str = "github-actions";
const DEFAULT_TRUSTED_REVIEWERS_PATH: &str = ".github/trusted-reviewers.json";
const TRUSTED_REVIEWERS_SCHEMA: &str = "laneflow.trusted-reviewers.v1";

pub fn run(args: &[String]) -> Result<(), String> {
    let args = parse_check_args(args)?;
    let roster = load_trusted_reviewers(&args.trusted_reviewers)?;
    let identity = load_pull_request(&args.repository, args.pr)?;
    let reviews = load_pull_request_reviews(&args.repository, args.pr)?;
    let reactions = load_issue_reactions(&args.repository, args.pr)?;
    let evaluation = evaluate(&identity, &reviews, &roster, None, &reactions);
    println!(
        "{}",
        serde_json::to_string_pretty(&evaluation)
            .map_err(|error| format!("无法序列化 External Review 结果：{error}"))?
    );
    if evaluation.passed {
        Ok(())
    } else {
        Err(evaluation.summary)
    }
}

pub fn run_publish_check(args: &[String]) -> Result<(), String> {
    let mut args = parse_publish_args(args)?;
    if let Some(head_ref) = args.merge_group_head_ref.clone() {
        let (pr, queued_head) = parse_merge_group_head_ref(&head_ref)?;
        if args.pr == 0 {
            args.pr = pr;
        } else if args.pr != pr {
            return Err(format!(
                "--pr #{} 与 merge_group.head_ref PR #{pr} 不一致",
                args.pr
            ));
        }
        if args.queued_head.is_none() {
            args.queued_head = queued_head;
        }
        args.require_publish = true;
    }
    if args.pr == 0 {
        return Err("缺少 --pr 或 --merge-group-head-ref".to_string());
    }
    let roster = load_trusted_reviewers(&args.trusted_reviewers)?;
    let initial = load_pull_request(&args.repository, args.pr)?;
    let check_head = args
        .check_head_sha
        .clone()
        .unwrap_or_else(|| initial.head_sha.clone());
    if let Some(reason) = skip_reason(&initial, args.require_publish) {
        if args.require_publish {
            return publish_completed_check(
                &args,
                &check_head,
                false,
                reason.to_string(),
                Vec::new(),
            );
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&PublishSkip {
                repository: args.repository.clone(),
                pull_request: args.pr,
                skipped: true,
                reason: reason.to_string(),
            })
            .map_err(|error| format!("无法序列化 skip 结果：{error}"))?
        );
        return Ok(());
    }

    let reviews = load_pull_request_reviews(&args.repository, args.pr)?;
    let reactions = load_issue_reactions(&args.repository, args.pr)?;
    let evaluation = evaluate(
        &initial,
        &reviews,
        &roster,
        args.queued_head.as_deref(),
        &reactions,
    );
    let verified = load_pull_request(&args.repository, args.pr)?;
    if verified.head_sha != initial.head_sha {
        return Err(format!(
            "head 竞态：评估时 `{}`，发布前 `{}`",
            initial.head_sha, verified.head_sha
        ));
    }
    if let Some(reason) = skip_reason(&verified, args.require_publish) {
        if args.require_publish {
            return publish_completed_check(
                &args,
                &check_head,
                false,
                reason.to_string(),
                Vec::new(),
            );
        }
        return Err(format!("PR identity 在发布前不再可评估：{reason}"));
    }

    publish_completed_check(
        &args,
        &check_head,
        evaluation.passed,
        evaluation.summary.clone(),
        evaluation.accepted_reviews.clone(),
    )?;

    println!(
        "{}",
        serde_json::to_string_pretty(&evaluation)
            .map_err(|error| format!("无法序列化 External Review 发布结果：{error}"))?
    );
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CheckArgs {
    repository: String,
    pr: u64,
    trusted_reviewers: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PublishArgs {
    repository: String,
    pr: u64,
    details_url: String,
    run_id: String,
    run_attempt: String,
    trusted_ref_oid: String,
    trusted_reviewers: PathBuf,
    check_head_sha: Option<String>,
    queued_head: Option<String>,
    merge_group_head_ref: Option<String>,
    require_publish: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct PublishSkip {
    repository: String,
    pull_request: u64,
    skipped: bool,
    reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TrustedReviewers {
    pub schema: String,
    pub reviewers: Vec<TrustedReviewer>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TrustedReviewer {
    pub login: String,
    pub kind: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewerRoster {
    logins: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PullRequestIdentity {
    pub number: u64,
    pub author_login: String,
    pub head_sha: String,
    pub base_ref: String,
    pub head_repo: String,
    pub base_repo: String,
    pub is_draft: bool,
    pub state: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativeReview {
    pub id: u64,
    pub author_login: String,
    pub state: String,
    pub commit_id: String,
    pub submitted_at: Option<String>,
    pub html_url: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssueReaction {
    pub user_login: String,
    pub content: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Evaluation {
    pub repository: String,
    pub pull_request: u64,
    pub current_head_oid: String,
    pub passed: bool,
    pub summary: String,
    pub accepted_reviews: Vec<AcceptedReview>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AcceptedReview {
    pub reviewer: String,
    pub state: String,
    pub commit_id: String,
    pub html_url: String,
}

#[derive(Serialize)]
struct CheckRunPayload {
    name: &'static str,
    head_sha: String,
    status: &'static str,
    conclusion: &'static str,
    details_url: String,
    external_id: String,
    output: CheckRunOutput,
}

#[derive(Serialize)]
struct CheckRunOutput {
    title: String,
    summary: String,
}

#[derive(Deserialize)]
struct CheckRunResponse {
    id: u64,
    name: String,
    head_sha: String,
    status: String,
    conclusion: Option<String>,
    html_url: String,
    external_id: Option<String>,
    app: CheckRunApp,
}

#[derive(Deserialize)]
struct CheckRunApp {
    slug: String,
}

#[derive(Deserialize)]
struct RestPullRequest {
    number: u64,
    user: RestUser,
    head: RestRef,
    base: RestRef,
    draft: bool,
    state: String,
}

#[derive(Deserialize)]
struct RestRef {
    sha: String,
    #[serde(rename = "ref")]
    git_ref: String,
    repo: Option<RestRepo>,
}

#[derive(Deserialize)]
struct RestRepo {
    full_name: String,
}

#[derive(Deserialize)]
struct RestUser {
    login: String,
}

#[derive(Deserialize)]
struct RestReview {
    id: u64,
    user: Option<RestUser>,
    state: String,
    commit_id: Option<String>,
    submitted_at: Option<String>,
    html_url: String,
}

#[derive(Deserialize)]
struct RestReaction {
    user: Option<RestUser>,
    content: String,
}

pub fn load_trusted_reviewers(path: &Path) -> Result<ReviewerRoster, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("无法读取受信审阅者名单 `{}`: {error}", path.display()))?;
    let file: TrustedReviewers = serde_json::from_str(&contents)
        .map_err(|error| format!("受信审阅者名单 `{}` 不是合法 JSON：{error}", path.display()))?;
    roster_from_file(&file)
}

fn roster_from_file(file: &TrustedReviewers) -> Result<ReviewerRoster, String> {
    if file.schema != TRUSTED_REVIEWERS_SCHEMA {
        return Err(format!(
            "受信审阅者名单 schema 必须是 `{TRUSTED_REVIEWERS_SCHEMA}`，实际为 `{}`",
            file.schema
        ));
    }
    if file.reviewers.is_empty() {
        return Err("受信审阅者名单不能为空".to_string());
    }

    let mut logins = BTreeMap::new();
    for reviewer in &file.reviewers {
        if reviewer.kind != "human" && reviewer.kind != "bot" {
            return Err(format!(
                "受信审阅者 `{}` 的 kind 必须是 `human` 或 `bot`",
                reviewer.login
            ));
        }
        let normalized = normalize_login(&reviewer.login);
        if normalized.is_empty() {
            return Err("受信审阅者 login 不能为空".to_string());
        }
        if logins
            .insert(normalized.clone(), reviewer.kind.clone())
            .is_some()
        {
            return Err(format!("受信审阅者名单重复 login `{normalized}`"));
        }
    }
    Ok(ReviewerRoster { logins })
}

pub fn evaluate(
    identity: &PullRequestIdentity,
    reviews: &[NativeReview],
    roster: &ReviewerRoster,
    queued_head: Option<&str>,
    reactions: &[IssueReaction],
) -> Evaluation {
    if let Some(queued_head) = queued_head {
        if !oid_equal(queued_head, &identity.head_sha) {
            return Evaluation {
                repository: identity.base_repo.clone(),
                pull_request: identity.number,
                current_head_oid: identity.head_sha.clone(),
                passed: false,
                summary: format!(
                    "合并组排队 head `{queued_head}` 与当前 PR head `{}` 不一致",
                    identity.head_sha
                ),
                accepted_reviews: Vec::new(),
            };
        }
    }

    let author = normalize_login(&identity.author_login);
    let latest_by_reviewer = latest_submitted_reviews(reviews);
    let mut accepted = Vec::new();
    for review in latest_by_reviewer.values() {
        let reviewer = normalize_login(&review.author_login);
        if reviewer == author {
            continue;
        }
        if !roster.logins.contains_key(&reviewer) {
            continue;
        }
        if !is_accepted_review_state(&review.state) {
            continue;
        }
        accepted.push(AcceptedReview {
            reviewer: reviewer.clone(),
            state: review.state.to_ascii_uppercase(),
            commit_id: review.commit_id.clone(),
            html_url: review.html_url.clone(),
        });
    }

    let issue_url = format!(
        "https://github.com/{}/pull/{}",
        identity.base_repo, identity.number
    );
    for reaction in reactions {
        if reaction.content != "+1" {
            continue;
        }
        let reviewer = normalize_login(&reaction.user_login);
        if reviewer.is_empty() || reviewer == author {
            continue;
        }
        if !roster.logins.contains_key(&reviewer) {
            continue;
        }
        accepted.push(AcceptedReview {
            reviewer,
            state: "+1".to_string(),
            commit_id: String::new(),
            html_url: issue_url.clone(),
        });
    }

    let passed = !accepted.is_empty();
    let summary = if passed {
        format!(
            "PR 上存在 {} 条受信非作者 Approve/Comment 或正文点赞",
            accepted.len()
        )
    } else {
        "缺少受信非作者 Approve/Comment，且 PR 正文无受信点赞".to_string()
    };

    Evaluation {
        repository: identity.base_repo.clone(),
        pull_request: identity.number,
        current_head_oid: identity.head_sha.clone(),
        passed,
        summary,
        accepted_reviews: accepted,
    }
}

fn latest_submitted_reviews(reviews: &[NativeReview]) -> BTreeMap<String, NativeReview> {
    let mut latest = BTreeMap::new();
    for review in reviews {
        if review.state.eq_ignore_ascii_case("PENDING") {
            continue;
        }
        if review.submitted_at.is_none() {
            continue;
        }
        let key = normalize_login(&review.author_login);
        if key.is_empty() {
            continue;
        }
        latest
            .entry(key)
            .and_modify(|current: &mut NativeReview| {
                if review_is_newer(review, current) {
                    *current = review.clone();
                }
            })
            .or_insert_with(|| review.clone());
    }
    latest
}

fn review_is_newer(candidate: &NativeReview, current: &NativeReview) -> bool {
    match (
        candidate.submitted_at.as_deref(),
        current.submitted_at.as_deref(),
    ) {
        (Some(left), Some(right)) if left != right => left > right,
        _ => candidate.id > current.id,
    }
}

fn is_accepted_review_state(state: &str) -> bool {
    matches!(
        state.to_ascii_uppercase().as_str(),
        "APPROVED" | "COMMENTED"
    )
}

fn normalize_login(login: &str) -> String {
    let login = login.trim().trim_end_matches("[bot]").to_ascii_lowercase();
    login
}

fn oid_equal(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn skip_reason(identity: &PullRequestIdentity, require_publish: bool) -> Option<&'static str> {
    if !identity.state.eq_ignore_ascii_case("open") {
        return Some("PR 不是 OPEN");
    }
    if identity.is_draft && !require_publish {
        return Some("Draft PR 不发布 External Review Check");
    }
    if identity.is_draft && require_publish {
        return Some("Draft PR 不能盖章到合并组");
    }
    if identity.base_ref != "main" {
        return Some("PR 目标分支不是 main");
    }
    if identity.head_repo != identity.base_repo {
        return Some("fork / cross-repository PR 必须先迁到同仓 PR");
    }
    None
}

fn parse_check_args(args: &[String]) -> Result<CheckArgs, String> {
    let mut repository = None;
    let mut pr = None;
    let mut trusted_reviewers = PathBuf::from(DEFAULT_TRUSTED_REVIEWERS_PATH);
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--repo" => {
                repository = Some(require_value(args, index, "--repo")?.to_string());
                index += 2;
            }
            "--pr" => {
                pr = Some(parse_pr(require_value(args, index, "--pr")?)?);
                index += 2;
            }
            "--trusted-reviewers" => {
                trusted_reviewers =
                    PathBuf::from(require_value(args, index, "--trusted-reviewers")?);
                index += 2;
            }
            other => return Err(format!("未知 check-external-review 参数：{other}")),
        }
    }
    Ok(CheckArgs {
        repository: repository.ok_or("缺少 --repo")?,
        pr: pr.ok_or("缺少 --pr")?,
        trusted_reviewers,
    })
}

fn parse_publish_args(args: &[String]) -> Result<PublishArgs, String> {
    let mut repository = None;
    let mut pr = None;
    let mut details_url = None;
    let mut run_id = None;
    let mut run_attempt = None;
    let mut trusted_ref_oid = None;
    let mut trusted_reviewers = PathBuf::from(DEFAULT_TRUSTED_REVIEWERS_PATH);
    let mut check_head_sha = None;
    let mut queued_head = None;
    let mut merge_group_head_ref = None;
    let mut require_publish = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--repo" => {
                repository = Some(require_value(args, index, "--repo")?.to_string());
                index += 2;
            }
            "--pr" => {
                pr = Some(parse_pr(require_value(args, index, "--pr")?)?);
                index += 2;
            }
            "--details-url" => {
                details_url = Some(require_value(args, index, "--details-url")?.to_string());
                index += 2;
            }
            "--run-id" => {
                run_id = Some(require_value(args, index, "--run-id")?.to_string());
                index += 2;
            }
            "--run-attempt" => {
                run_attempt = Some(require_value(args, index, "--run-attempt")?.to_string());
                index += 2;
            }
            "--trusted-ref-oid" => {
                trusted_ref_oid =
                    Some(require_value(args, index, "--trusted-ref-oid")?.to_string());
                index += 2;
            }
            "--trusted-reviewers" => {
                trusted_reviewers =
                    PathBuf::from(require_value(args, index, "--trusted-reviewers")?);
                index += 2;
            }
            "--head-sha" => {
                check_head_sha = Some(require_value(args, index, "--head-sha")?.to_string());
                index += 2;
            }
            "--queued-head" => {
                queued_head = Some(require_value(args, index, "--queued-head")?.to_string());
                index += 2;
            }
            "--merge-group-head-ref" => {
                merge_group_head_ref =
                    Some(require_value(args, index, "--merge-group-head-ref")?.to_string());
                index += 2;
            }
            "--require-publish" => {
                require_publish = true;
                index += 1;
            }
            other => {
                return Err(format!("未知 publish-external-review-check 参数：{other}"));
            }
        }
    }
    Ok(PublishArgs {
        repository: repository.ok_or("缺少 --repo")?,
        pr: pr.unwrap_or(0),
        details_url: details_url.ok_or("缺少 --details-url")?,
        run_id: run_id.ok_or("缺少 --run-id")?,
        run_attempt: run_attempt.ok_or("缺少 --run-attempt")?,
        trusted_ref_oid: trusted_ref_oid.ok_or("缺少 --trusted-ref-oid")?,
        trusted_reviewers,
        check_head_sha,
        queued_head,
        merge_group_head_ref,
        require_publish,
    })
}

fn require_value<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a str, String> {
    args.get(index + 1)
        .map(String::as_str)
        .filter(|value| !value.is_empty() && !value.starts_with("--"))
        .ok_or_else(|| format!("{flag} 缺少值"))
}

fn parse_pr(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| format!("PR 号必须是正整数：{value}"))
}

pub fn parse_merge_group_head_ref(head_ref: &str) -> Result<(u64, Option<String>), String> {
    let value = head_ref.strip_prefix("refs/heads/").unwrap_or(head_ref);
    let rest = value
        .strip_prefix("gh-readonly-queue/")
        .ok_or_else(|| format!("merge_group.head_ref 不是 gh-readonly-queue 引用：{head_ref}"))?;
    let (_, pr_part) = rest
        .split_once("/pr-")
        .ok_or_else(|| format!("merge_group.head_ref 缺少 pr-<number> 段：{head_ref}"))?;
    let (number, suffix) = match pr_part.split_once('-') {
        Some((number, suffix)) => (number, Some(suffix)),
        None => (pr_part, None),
    };
    let number = parse_pr(number)?;
    let queued_head = suffix.and_then(|suffix| {
        let suffix = suffix.trim();
        if suffix.len() == 40 && suffix.chars().all(|ch| ch.is_ascii_hexdigit()) {
            Some(suffix.to_ascii_lowercase())
        } else {
            None
        }
    });
    Ok((number, queued_head))
}

fn load_pull_request(repository: &str, pr: u64) -> Result<PullRequestIdentity, String> {
    let endpoint = format!("repos/{repository}/pulls/{pr}");
    let payload = gh_api_get(&endpoint)?;
    let parsed: RestPullRequest =
        serde_json::from_str(&payload).map_err(|error| format!("无法解析 PR JSON：{error}"))?;
    rest_pull_request_identity(parsed, repository)
}

fn rest_pull_request_identity(
    parsed: RestPullRequest,
    fallback_repo: &str,
) -> Result<PullRequestIdentity, String> {
    let base_repo = parsed
        .base
        .repo
        .as_ref()
        .map(|repo| repo.full_name.clone())
        .unwrap_or_else(|| fallback_repo.to_string());
    let head_repo = parsed
        .head
        .repo
        .as_ref()
        .map(|repo| repo.full_name.clone())
        .unwrap_or_else(|| base_repo.clone());
    Ok(PullRequestIdentity {
        number: parsed.number,
        author_login: parsed.user.login,
        head_sha: parsed.head.sha,
        base_ref: parsed.base.git_ref,
        head_repo,
        base_repo,
        is_draft: parsed.draft,
        state: parsed.state,
    })
}

fn load_issue_reactions(repository: &str, pr: u64) -> Result<Vec<IssueReaction>, String> {
    let endpoint = format!("repos/{repository}/issues/{pr}/reactions?per_page=100");
    let payload = gh_api_get_paginate(&endpoint)?;
    let parsed: Vec<RestReaction> = serde_json::from_str(&payload)
        .map_err(|error| format!("无法解析 reactions JSON：{error}"))?;
    Ok(parsed
        .into_iter()
        .map(|reaction| IssueReaction {
            user_login: reaction.user.map(|user| user.login).unwrap_or_default(),
            content: reaction.content,
        })
        .collect())
}

fn load_pull_request_reviews(repository: &str, pr: u64) -> Result<Vec<NativeReview>, String> {
    let endpoint = format!("repos/{repository}/pulls/{pr}/reviews?per_page=100");
    let payload = gh_api_get_paginate(&endpoint)?;
    let parsed: Vec<RestReview> = serde_json::from_str(&payload)
        .map_err(|error| format!("无法解析 reviews JSON：{error}"))?;
    Ok(parsed
        .into_iter()
        .map(|review| NativeReview {
            id: review.id,
            author_login: review.user.map(|user| user.login).unwrap_or_default(),
            state: review.state,
            commit_id: review.commit_id.unwrap_or_default(),
            submitted_at: review.submitted_at,
            html_url: review.html_url,
        })
        .collect())
}

fn gh_api_get(endpoint: &str) -> Result<String, String> {
    gh_api(endpoint, false)
}

fn gh_api_get_paginate(endpoint: &str) -> Result<String, String> {
    gh_api(endpoint, true)
}

fn gh_api(endpoint: &str, paginate: bool) -> Result<String, String> {
    let mut command = Command::new("gh");
    command.arg("api");
    if paginate {
        command.arg("--paginate");
    }
    command.args([
        "-H",
        "Accept: application/vnd.github+json",
        "-H",
        "X-GitHub-Api-Version: 2022-11-28",
        endpoint,
    ]);
    let output = command
        .output()
        .map_err(|error| format!("无法启动 gh API：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh API 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("gh API 输出不是 UTF-8：{error}"))?;
    if paginate {
        merge_paginated_json_arrays(&stdout)
    } else {
        Ok(stdout)
    }
}

fn merge_paginated_json_arrays(stdout: &str) -> Result<String, String> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok("[]".to_string());
    }
    if trimmed.starts_with('[') && !trimmed.contains("][") {
        return Ok(trimmed.to_string());
    }

    let mut merged = Vec::new();
    for document in split_concatenated_json_arrays(trimmed)? {
        let items: Vec<serde_json::Value> = serde_json::from_str(&document)
            .map_err(|error| format!("无法解析分页 reviews JSON：{error}"))?;
        merged.extend(items);
    }
    serde_json::to_string(&merged).map_err(|error| format!("无法合并分页 reviews JSON：{error}"))
}

fn split_concatenated_json_arrays(input: &str) -> Result<Vec<String>, String> {
    let mut documents = Vec::new();
    let mut depth = 0_i32;
    let mut start = 0;
    for (index, ch) in input.char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    documents.push(input[start..=index].to_string());
                    start = index + 1;
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err("分页 reviews JSON 数组未闭合".to_string());
    }
    Ok(documents)
}

fn publish_completed_check(
    args: &PublishArgs,
    head_sha: &str,
    passed: bool,
    summary: String,
    accepted: Vec<AcceptedReview>,
) -> Result<(), String> {
    let conclusion = if passed { "success" } else { "failure" };
    let title = if passed {
        "External Review passed".to_string()
    } else {
        "External Review required".to_string()
    };
    let mut text = summary.clone();
    for review in &accepted {
        text.push_str(&format!(
            "\n- {} {} {}",
            review.reviewer, review.state, review.html_url
        ));
    }
    let external_id = format!(
        "laneflow-external-review:v1:{}#{}:{}:{}:run-{}-{}",
        args.repository, args.pr, head_sha, args.trusted_ref_oid, args.run_id, args.run_attempt
    );
    let payload = CheckRunPayload {
        name: CHECK_NAME,
        head_sha: head_sha.to_string(),
        status: "completed",
        conclusion,
        details_url: args.details_url.clone(),
        external_id: external_id.clone(),
        output: CheckRunOutput {
            title,
            summary: text,
        },
    };
    let response = create_check_run(&args.repository, &payload)?;
    verify_check_run_response(&response, &payload)?;
    println!(
        "published check_run_id={} url={}",
        response.id, response.html_url
    );
    Ok(())
}

fn create_check_run(
    repository: &str,
    payload: &CheckRunPayload,
) -> Result<CheckRunResponse, String> {
    let payload_bytes = serde_json::to_vec(payload)
        .map_err(|error| format!("无法序列化 Check Run payload：{error}"))?;
    let endpoint = format!("repos/{repository}/check-runs");
    let mut child = Command::new("gh")
        .args([
            "api",
            "--method",
            "POST",
            &endpoint,
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "X-GitHub-Api-Version: 2022-11-28",
            "--input",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("无法启动 gh Check Run API：{error}"))?;
    child
        .stdin
        .as_mut()
        .ok_or("无法打开 gh Check Run API stdin")?
        .write_all(&payload_bytes)
        .map_err(|error| format!("无法写入 gh Check Run payload：{error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("无法等待 gh Check Run API：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh Check Run API 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        format!(
            "gh Check Run API 输出不是预期 JSON：{error}；原始输出：{}",
            String::from_utf8_lossy(&output.stdout).trim()
        )
    })
}

fn verify_check_run_response(
    response: &CheckRunResponse,
    payload: &CheckRunPayload,
) -> Result<(), String> {
    let conclusion = response.conclusion.as_deref().unwrap_or_default();
    if response.name != payload.name
        || response.head_sha != payload.head_sha
        || response.status != payload.status
        || conclusion != payload.conclusion
        || response.external_id.as_deref() != Some(payload.external_id.as_str())
        || response.app.slug != CHECK_APP_SLUG
    {
        return Err(format!(
            "Check Run 发布结果不符合绑定要求：name={} head={} status={} conclusion={} external_id={:?} app={}",
            response.name,
            response.head_sha,
            response.status,
            conclusion,
            response.external_id,
            response.app.slug
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "2324858a2324858a2324858a2324858a2324858a";
    const OLD_HEAD: &str = "183b675e183b675e183b675e183b675e183b675e";

    fn roster() -> ReviewerRoster {
        roster_from_file(&TrustedReviewers {
            schema: TRUSTED_REVIEWERS_SCHEMA.to_string(),
            reviewers: vec![
                TrustedReviewer {
                    login: "wangzishi".to_string(),
                    kind: "human".to_string(),
                },
                TrustedReviewer {
                    login: "copilot-pull-request-reviewer".to_string(),
                    kind: "bot".to_string(),
                },
            ],
        })
        .expect("roster should parse")
    }

    fn identity(author: &str) -> PullRequestIdentity {
        PullRequestIdentity {
            number: 278,
            author_login: author.to_string(),
            head_sha: HEAD.to_string(),
            base_ref: "main".to_string(),
            head_repo: "illusion-tech/laneflow".to_string(),
            base_repo: "illusion-tech/laneflow".to_string(),
            is_draft: false,
            state: "open".to_string(),
        }
    }

    fn review(id: u64, login: &str, state: &str, commit: &str) -> NativeReview {
        NativeReview {
            id,
            author_login: login.to_string(),
            state: state.to_string(),
            commit_id: commit.to_string(),
            submitted_at: Some(format!("2026-08-21T00:00:{id:02}Z")),
            html_url: format!("https://github.com/illusion-tech/laneflow/pull/278#review-{id}"),
        }
    }

    fn eval(
        identity: &PullRequestIdentity,
        reviews: &[NativeReview],
        roster: &ReviewerRoster,
        queued_head: Option<&str>,
    ) -> Evaluation {
        super::evaluate(identity, reviews, roster, queued_head, &[])
    }

    fn plus_one(login: &str) -> IssueReaction {
        IssueReaction {
            user_login: login.to_string(),
            content: "+1".to_string(),
        }
    }

    #[test]
    fn human_approval_on_current_head_passes() {
        let evaluation = eval(
            &identity("alice"),
            &[review(1, "wangzishi", "APPROVED", HEAD)],
            &roster(),
            None,
        );
        assert!(evaluation.passed);
        assert_eq!(evaluation.accepted_reviews[0].reviewer, "wangzishi");
    }

    #[test]
    fn copilot_commented_review_on_current_head_passes() {
        let evaluation = eval(
            &identity("wangzishi"),
            &[review(
                2,
                "copilot-pull-request-reviewer[bot]",
                "COMMENTED",
                HEAD,
            )],
            &roster(),
            None,
        );
        assert!(evaluation.passed);
    }

    #[test]
    fn author_self_review_does_not_count() {
        let evaluation = eval(
            &identity("wangzishi"),
            &[review(1, "wangzishi", "APPROVED", HEAD)],
            &roster(),
            None,
        );
        assert!(!evaluation.passed);
    }

    #[test]
    fn old_head_approval_still_counts() {
        let evaluation = eval(
            &identity("alice"),
            &[review(1, "wangzishi", "APPROVED", OLD_HEAD)],
            &roster(),
            None,
        );
        assert!(evaluation.passed);
    }

    #[test]
    fn trusted_pr_body_thumbs_up_passes() {
        let evaluation = super::evaluate(
            &identity("alice"),
            &[],
            &roster(),
            None,
            &[plus_one("copilot-pull-request-reviewer[bot]")],
        );
        assert!(evaluation.passed);
        assert_eq!(evaluation.accepted_reviews[0].state, "+1");
    }

    #[test]
    fn author_thumbs_up_does_not_count() {
        let evaluation = super::evaluate(
            &identity("wangzishi"),
            &[],
            &roster(),
            None,
            &[plus_one("wangzishi")],
        );
        assert!(!evaluation.passed);
    }

    #[test]
    fn changes_requested_does_not_pass() {
        let evaluation = eval(
            &identity("alice"),
            &[review(1, "wangzishi", "CHANGES_REQUESTED", HEAD)],
            &roster(),
            None,
        );
        assert!(!evaluation.passed);
    }

    #[test]
    fn dismissed_review_does_not_pass() {
        let evaluation = eval(
            &identity("alice"),
            &[review(1, "wangzishi", "DISMISSED", HEAD)],
            &roster(),
            None,
        );
        assert!(!evaluation.passed);
    }

    #[test]
    fn untrusted_bot_does_not_count_even_with_native_review() {
        let evaluation = eval(
            &identity("alice"),
            &[review(1, "chatgpt-codex-connector[bot]", "COMMENTED", HEAD)],
            &roster(),
            None,
        );
        assert!(!evaluation.passed);
    }

    #[test]
    fn latest_changes_requested_supersedes_earlier_approval() {
        let evaluation = eval(
            &identity("alice"),
            &[
                review(1, "wangzishi", "APPROVED", HEAD),
                review(2, "wangzishi", "CHANGES_REQUESTED", HEAD),
            ],
            &roster(),
            None,
        );
        assert!(!evaluation.passed);
    }

    #[test]
    fn queued_head_mismatch_fails_merge_group_stamp() {
        let evaluation = eval(
            &identity("alice"),
            &[review(1, "wangzishi", "APPROVED", HEAD)],
            &roster(),
            Some(OLD_HEAD),
        );
        assert!(!evaluation.passed);
        assert!(evaluation.summary.contains("排队 head"));
    }

    #[test]
    fn queued_head_match_passes_merge_group_stamp() {
        let evaluation = eval(
            &identity("alice"),
            &[review(1, "wangzishi", "APPROVED", HEAD)],
            &roster(),
            Some(HEAD),
        );
        assert!(evaluation.passed);
    }

    #[test]
    fn rejects_unknown_trusted_reviewers_schema() {
        let error = roster_from_file(&TrustedReviewers {
            schema: "laneflow.trusted-reviewers.v0".to_string(),
            reviewers: vec![TrustedReviewer {
                login: "wangzishi".to_string(),
                kind: "human".to_string(),
            }],
        })
        .expect_err("unknown schema should fail");
        assert!(error.contains("laneflow.trusted-reviewers.v1"));
    }

    #[test]
    fn parses_merge_group_head_ref_with_full_oid() {
        let head_ref = format!("refs/heads/gh-readonly-queue/main/pr-278-{HEAD}");
        let (number, queued) = parse_merge_group_head_ref(&head_ref).expect("ref should parse");
        assert_eq!(number, 278);
        assert_eq!(queued.as_deref(), Some(HEAD));
    }

    #[test]
    fn parses_merge_group_head_ref_without_binding_short_suffix() {
        let (number, queued) =
            parse_merge_group_head_ref("gh-readonly-queue/main/pr-278-abc1234").expect("parse");
        assert_eq!(number, 278);
        assert_eq!(queued, None);
    }

    #[test]
    fn skip_reason_rejects_forks() {
        let mut pr = identity("alice");
        pr.head_repo = "fork/laneflow".to_string();
        assert_eq!(
            skip_reason(&pr, false),
            Some("fork / cross-repository PR 必须先迁到同仓 PR")
        );
    }

    #[test]
    fn empty_reviews_and_reactions_fail() {
        let evaluation = eval(&identity("alice"), &[], &roster(), None);
        assert!(!evaluation.passed);
        assert!(evaluation.summary.contains("点赞"));
    }

    #[test]
    fn repository_trusted_reviewers_file_parses() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(".github/trusted-reviewers.json");
        let roster = load_trusted_reviewers(&path).expect("checked-in roster should parse");
        assert!(roster.logins.contains_key("wangzishi"));
        assert!(roster.logins.contains_key("copilot-pull-request-reviewer"));
        assert!(roster.logins.contains_key("chatgpt-codex-connector"));
        assert!(roster.logins.contains_key("kody-ai"));
    }
}
