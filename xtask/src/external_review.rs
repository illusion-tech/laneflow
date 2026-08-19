use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SNAPSHOT_SCHEMA_VERSION: u64 = 1;
const RESULT_SCHEMA_VERSION: u64 = 1;
const CHECK_PUBLISH_RESULT_SCHEMA_VERSION: u64 = 1;
const EXTERNAL_REVIEW_SHADOW_CHECK_NAME: &str = "External Review Gate Shadow";
const R1_SHADOW_CHECK_APP_SLUG: &str = "github-actions";
const COPILOT_ACTOR: &str = "copilot-pull-request-reviewer";
const CODEX_ACTOR: &str = "chatgpt-codex-connector";
const DEPENDABOT_AUTHOR_NAME: &str = "dependabot[bot]";
const DEPENDABOT_AUTHOR_EMAIL: &str = "49699333+dependabot[bot]@users.noreply.github.com";
const TRUSTED_HUMAN_ACTORS: &[&str] = &["wangzishi"];
const GITHUB_ACTIONS_ACTOR: &str = "github-actions";
const CODEX_REVIEW_REQUEST_MARKER: &str = "<!-- codex-review-request:v1 ";
const CODEX_CLEAN_BINDING_MARKER: &str = "<!-- codex-clean-binding:v1 ";
const HIDDEN_RECORD_SUFFIX: &str = " -->";
const BINDING_RECORD_SCHEMA_VERSION: u64 = 1;

const EXTERNAL_REVIEW_QUERY: &str = r#"
query($owner:String!, $name:String!, $number:Int!) {
  repository(owner:$owner, name:$name) {
    pullRequest(number:$number) {
      number
      author { login }
      headRefOid
      baseRefOid
      isDraft
      files(first:2) {
        nodes { path changeType }
        pageInfo { hasNextPage }
      }
      commits(first:2) {
        nodes {
          commit {
            oid
            committedDate
            url
            author { name email }
          }
        }
        pageInfo { hasNextPage }
      }
      reviewRequests(first:100) {
        nodes {
          requestedReviewer {
            ... on User { login }
            ... on Team { name }
          }
        }
        pageInfo { hasNextPage }
      }
      reviews(first:100) {
        nodes {
          id
          author { login }
          body
          state
          submittedAt
          url
          commit { oid }
        }
        pageInfo { hasNextPage }
      }
      comments(first:100) {
        nodes {
          id
          author { login }
          body
          createdAt
          updatedAt
          url
        }
        pageInfo { hasNextPage }
      }
      reviewThreads(first:100) {
        nodes {
          id
          isResolved
          isOutdated
          comments(first:100) {
            nodes {
              id
              author { login }
              body
              createdAt
              updatedAt
              url
              pullRequestReview {
                id
                author { login }
                state
                submittedAt
                commit { oid }
              }
            }
            pageInfo { hasNextPage }
          }
        }
        pageInfo { hasNextPage }
      }
    }
  }
}
"#;

const PULL_REQUEST_IDENTITY_QUERY: &str = r#"
query($owner:String!, $name:String!, $number:Int!) {
  repository(owner:$owner, name:$name) {
    pullRequest(number:$number) {
      number
      author { login }
      headRefOid
      baseRefOid
      baseRefName
      headRefName
      isCrossRepository
      isDraft
      state
    }
  }
}
"#;

const PULL_REQUEST_COMMENTS_QUERY: &str = r#"
query($owner:String!, $name:String!, $number:Int!, $cursor:String) {
  repository(owner:$owner, name:$name) {
    pullRequest(number:$number) {
      comments(first:100, after:$cursor) {
        nodes {
          id
          author { login }
          body
          createdAt
          updatedAt
          url
        }
        pageInfo { hasNextPage endCursor }
      }
    }
  }
}
"#;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalReviewState {
    Pass,
    AwaitingReview,
    ReviewPending,
    FindingsOpen,
    AwaitingRereview,
    Stale,
    ProviderError,
    Waived,
}

impl ExternalReviewState {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "pass" => Ok(Self::Pass),
            "awaiting_review" => Ok(Self::AwaitingReview),
            "review_pending" => Ok(Self::ReviewPending),
            "findings_open" => Ok(Self::FindingsOpen),
            "awaiting_rereview" => Ok(Self::AwaitingRereview),
            "stale" => Ok(Self::Stale),
            "provider_error" => Ok(Self::ProviderError),
            "waived" => Ok(Self::Waived),
            _ => Err(format!(
                "未知 external-review 状态 `{value}`；应为 pass、awaiting_review、review_pending、findings_open、awaiting_rereview、stale、provider_error 或 waived"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::AwaitingReview => "awaiting_review",
            Self::ReviewPending => "review_pending",
            Self::FindingsOpen => "findings_open",
            Self::AwaitingRereview => "awaiting_rereview",
            Self::Stale => "stale",
            Self::ProviderError => "provider_error",
            Self::Waived => "waived",
        }
    }

    fn check_conclusion(self) -> &'static str {
        match self {
            Self::Pass => "success",
            Self::Waived => "action_required",
            Self::AwaitingReview
            | Self::ReviewPending
            | Self::FindingsOpen
            | Self::AwaitingRereview
            | Self::Stale
            | Self::ProviderError => "failure",
        }
    }

    fn check_title(self) -> &'static str {
        match self {
            Self::Pass => "External review passed",
            Self::Waived => "External review waived",
            Self::AwaitingReview => "External review is required",
            Self::ReviewPending => "External review is pending",
            Self::FindingsOpen => "External review findings remain open",
            Self::AwaitingRereview => "External review needs a clean re-review",
            Self::Stale => "External review evidence is stale",
            Self::ProviderError => "External review could not be evaluated",
        }
    }

    pub fn is_pass(self) -> bool {
        self == Self::Pass
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum EvidenceOutcome {
    Clean,
    Findings,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalReviewSnapshot {
    schema_version: u64,
    repository: String,
    pull_request: PullRequestSnapshot,
    #[serde(default)]
    provider_errors: Vec<String>,
    #[serde(default)]
    waiver: Option<WaiverInput>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PullRequestSnapshot {
    number: u64,
    author: Option<Actor>,
    head_ref_oid: String,
    base_ref_oid: String,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    files: Connection<ChangedFile>,
    #[serde(default)]
    commits: Connection<PullRequestCommit>,
    #[serde(default)]
    review_requests: Connection<ReviewRequest>,
    #[serde(default)]
    reviews: Connection<Review>,
    #[serde(default)]
    comments: Connection<IssueComment>,
    #[serde(default)]
    review_threads: Connection<ReviewThread>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Actor {
    login: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChangedFile {
    path: String,
    change_type: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PullRequestCommit {
    commit: CommitMetadata,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommitMetadata {
    oid: String,
    committed_date: String,
    url: String,
    #[serde(default)]
    author: Option<CommitAuthor>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommitAuthor {
    name: String,
    email: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    rename_all = "camelCase",
    deny_unknown_fields,
    bound(deserialize = "T: Deserialize<'de>", serialize = "T: Serialize")
)]
struct Connection<T> {
    #[serde(default)]
    nodes: Vec<T>,
    #[serde(default)]
    page_info: PageInfo,
}

impl<T> Default for Connection<T> {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            page_info: PageInfo::default(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PageInfo {
    #[serde(default)]
    has_next_page: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReviewRequest {
    requested_reviewer: Option<RequestedReviewer>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RequestedReviewer {
    #[serde(default)]
    login: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Review {
    id: String,
    author: Option<Actor>,
    #[serde(default)]
    body: String,
    state: String,
    #[serde(default)]
    submitted_at: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    commit: Option<CommitRef>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IssueComment {
    id: String,
    author: Option<Actor>,
    #[serde(default)]
    body: String,
    created_at: String,
    updated_at: String,
    url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReviewThread {
    id: String,
    is_resolved: bool,
    is_outdated: bool,
    #[serde(default)]
    comments: Connection<ReviewThreadComment>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReviewThreadComment {
    id: String,
    author: Option<Actor>,
    #[serde(default)]
    body: String,
    created_at: String,
    updated_at: String,
    url: String,
    #[serde(default)]
    pull_request_review: Option<ReviewReference>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReviewReference {
    id: String,
    author: Option<Actor>,
    state: String,
    #[serde(default)]
    submitted_at: Option<String>,
    #[serde(default)]
    commit: Option<CommitRef>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommitRef {
    oid: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WaiverInput {
    pub(crate) id: String,
    pub(crate) exception_type: String,
    pub(crate) current_head_oid: String,
    pub(crate) current_base_oid: String,
    pub(crate) reason: String,
    pub(crate) evidence_urls: Vec<String>,
    pub(crate) risk: String,
    pub(crate) acceptance_boundary: String,
    pub(crate) expires_at: String,
    pub(crate) follow_up_issue: String,
    pub(crate) cleanup_owner: String,
    pub(crate) authorized_by: String,
    #[serde(skip)]
    pub(crate) historical_base_replay: bool,
    #[serde(skip)]
    pub(crate) grandfathered_confirmed_gate_defect: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CodexReviewRequestRecord {
    schema_version: u64,
    id: String,
    pr: u64,
    request_head_oid: String,
    request_base_oid: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CodexCleanBindingRecord {
    schema_version: u64,
    id: String,
    pr: u64,
    clean_comment_id: String,
    clean_comment_created_at: String,
    clean_comment_url: String,
    request_marker_id: String,
    bound_head_oid: String,
    bound_base_oid: String,
    verified_at: String,
    run_url: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalReviewResult {
    schema_version: u64,
    repository: String,
    pull_request: u64,
    current_head_oid: String,
    current_base_oid: String,
    author: String,
    pub state: ExternalReviewState,
    provider: Option<String>,
    actor: Option<String>,
    reviewed_head_oid: Option<String>,
    completion_time: Option<String>,
    finding_count: usize,
    unresolved_actionable_threads: usize,
    requires_rereview: bool,
    pending_review_requests: usize,
    evidence: Vec<ReviewEvidence>,
    waiver_id: Option<String>,
    diagnostics: Vec<String>,
}

impl ExternalReviewResult {
    fn provider_error(repository: &str, pr: u64, diagnostic: String) -> Self {
        Self {
            schema_version: RESULT_SCHEMA_VERSION,
            repository: repository.to_string(),
            pull_request: pr,
            current_head_oid: String::new(),
            current_base_oid: String::new(),
            author: String::new(),
            state: ExternalReviewState::ProviderError,
            provider: None,
            actor: None,
            reviewed_head_oid: None,
            completion_time: None,
            finding_count: 0,
            unresolved_actionable_threads: 0,
            requires_rereview: false,
            pending_review_requests: 0,
            evidence: Vec::new(),
            waiver_id: None,
            diagnostics: vec![diagnostic],
        }
    }

    pub fn current_head_oid(&self) -> &str {
        &self.current_head_oid
    }

    pub fn completion_time(&self) -> Option<&str> {
        self.completion_time.as_deref()
    }

    pub(crate) fn uses_dependabot_lockfile_policy(&self) -> bool {
        self.state == ExternalReviewState::Pass
            && self.evidence.iter().any(|item| {
                item.provider == "dependabot_lockfile_policy"
                    && item.source_kind == "machine_verification"
                    && item.outcome == EvidenceOutcome::Clean
                    && oid_matches_current(&item.reviewed_head_oid, &self.current_head_oid)
            })
    }

    fn bind_identity_if_missing(&mut self, repository: &str, identity: &PullRequestIdentity) {
        if self.repository.is_empty() {
            self.repository = repository.to_string();
        }
        if self.current_head_oid.is_empty() {
            self.current_head_oid.clone_from(&identity.head_ref_oid);
        }
        if self.current_base_oid.is_empty() {
            self.current_base_oid.clone_from(&identity.base_ref_oid);
        }
        if self.author.is_empty() {
            self.author = identity
                .author
                .as_ref()
                .map(|author| author.login.clone())
                .unwrap_or_default();
        }
    }

    fn set_provider_error(&mut self, diagnostic: String) {
        self.state = ExternalReviewState::ProviderError;
        self.provider = None;
        self.actor = None;
        self.reviewed_head_oid = None;
        self.completion_time = None;
        self.requires_rereview = false;
        self.diagnostics.push(diagnostic);
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewEvidence {
    provider: String,
    actor: String,
    source_kind: String,
    reviewed_head_oid: String,
    reviewed_base_oid: String,
    outcome: EvidenceOutcome,
    submitted_at: String,
    evidence_url: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputFormat {
    Json,
    Summary,
}

#[derive(Debug)]
enum InputSource {
    Live { repository: String, pr: u64 },
    Snapshot(PathBuf),
}

#[derive(Debug)]
struct ExternalReviewArgs {
    source: InputSource,
    output_format: OutputFormat,
    expected_state: Option<ExternalReviewState>,
}

#[derive(Debug)]
struct PublishCheckArgs {
    repository: String,
    pr: u64,
    details_url: String,
    run_id: u64,
    run_attempt: u64,
    trusted_ref_oid: String,
}

#[derive(Debug)]
struct RequestCodexReviewArgs {
    repository: String,
    pr: u64,
    dry_run: bool,
}

#[derive(Debug)]
struct PublishCodexCleanBindingArgs {
    repository: String,
    pr: u64,
    clean_comment_id: String,
    run_url: String,
    dry_run: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexReviewRequestResult {
    schema_version: u64,
    repository: String,
    pull_request: u64,
    request_id: String,
    request_head_oid: String,
    request_base_oid: String,
    comment_url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexCleanBindingResult {
    schema_version: u64,
    repository: String,
    pull_request: u64,
    skipped: bool,
    reason: Option<String>,
    binding_id: Option<String>,
    bound_head_oid: Option<String>,
    comment_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RepoEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    payload: RepoEventPayload,
}

#[derive(Debug, Default, Deserialize)]
struct RepoEventPayload {
    #[serde(rename = "ref")]
    ref_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TimelineEvent {
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BranchInfo {
    commit: BranchCommit,
}

#[derive(Debug, Deserialize)]
struct BranchCommit {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct PostedComment {
    id: u64,
    node_id: String,
    body: Option<String>,
    user: Option<RestUser>,
    html_url: String,
}

#[derive(Debug, Deserialize)]
struct RestUser {
    login: String,
}

#[derive(Debug, Serialize)]
struct CheckRunPayload {
    name: &'static str,
    head_sha: String,
    status: &'static str,
    conclusion: &'static str,
    details_url: String,
    external_id: String,
    output: CheckRunOutput,
}

#[derive(Debug, Serialize)]
struct CheckRunOutput {
    title: &'static str,
    summary: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct CheckRunResponse {
    id: u64,
    name: String,
    html_url: String,
    head_sha: String,
    status: String,
    conclusion: Option<String>,
    external_id: Option<String>,
    app: CheckRunApp,
}

#[derive(Debug, Deserialize)]
struct CheckRunApp {
    slug: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublishCheckResult {
    schema_version: u64,
    repository: String,
    pull_request: u64,
    current_head_oid: String,
    current_base_oid: String,
    state: ExternalReviewState,
    conclusion: String,
    check_run_id: u64,
    check_run_url: String,
    source_app: String,
    external_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublishCheckSkip {
    schema_version: u64,
    repository: String,
    pull_request: u64,
    skipped: bool,
    reason: String,
}

pub fn run(args: &[String]) -> Result<(), String> {
    let args = parse_args(args)?;
    let result = match &args.source {
        InputSource::Live { repository, pr } => evaluate_live(repository, *pr)
            .unwrap_or_else(|error| ExternalReviewResult::provider_error(repository, *pr, error)),
        InputSource::Snapshot(path) => {
            let bytes = fs::read(path).map_err(|error| {
                format!(
                    "无法读取 external-review snapshot `{}`: {error}",
                    path.display()
                )
            })?;
            let snapshot =
                serde_json::from_slice::<ExternalReviewSnapshot>(&bytes).map_err(|error| {
                    format!(
                        "external-review snapshot `{}` 不是 schema v1 JSON: {error}",
                        path.display()
                    )
                })?;
            evaluate_snapshot(&snapshot)
        }
    };

    match args.output_format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&result)
                    .map_err(|error| format!("无法序列化 external-review JSON: {error}"))?
            );
        }
        OutputFormat::Summary => print_summary(&result),
    }

    match args.expected_state {
        Some(expected) if result.state == expected => Ok(()),
        Some(expected) => Err(format!(
            "external-review 状态与 --expect 不一致：期望 {:?}，实际 {:?}",
            expected, result.state
        )),
        None if result.state.is_pass() => Ok(()),
        None => Err(format!(
            "External Review Gate 未通过：状态为 {:?}",
            result.state
        )),
    }
}

pub fn run_publish_check(args: &[String]) -> Result<(), String> {
    let args = parse_publish_check_args(args)?;
    let initial_identity = load_live_identity(&args.repository, args.pr)?;
    if initial_identity.number != args.pr {
        return Err(format!(
            "GitHub PR identity number 不一致：请求 #{}，返回 #{}",
            args.pr, initial_identity.number
        ));
    }
    if let Some(reason) = shadow_skip_reason(&initial_identity) {
        println!(
            "{}",
            serde_json::to_string_pretty(&PublishCheckSkip {
                schema_version: CHECK_PUBLISH_RESULT_SCHEMA_VERSION,
                repository: args.repository,
                pull_request: args.pr,
                skipped: true,
                reason: reason.to_string(),
            })
            .map_err(|error| format!("无法序列化 shadow Check skip 结果：{error}"))?
        );
        return Ok(());
    }

    let mut result = evaluate_live(&args.repository, args.pr).unwrap_or_else(|error| {
        ExternalReviewResult::provider_error(&args.repository, args.pr, error)
    });
    let final_identity = load_live_identity(&args.repository, args.pr)?;
    ensure_identity_unchanged(&initial_identity, &final_identity)?;
    if final_identity.state != "OPEN" || final_identity.is_draft {
        return Err("PR identity 在发布前变为非 OPEN 或 Draft；拒绝发布 shadow Check".to_string());
    }
    result.bind_identity_if_missing(&args.repository, &final_identity);
    ensure_result_matches_identity(&result, &final_identity)?;

    let evaluation_fingerprint = evaluation_fingerprint(&result)?;
    let evaluation_key = format!(
        "laneflow-external-review:v1:{}#{}:{}:{}:{}:{}",
        args.repository,
        args.pr,
        result.current_head_oid,
        args.trusted_ref_oid,
        result.state.as_str(),
        evaluation_fingerprint
    );
    let external_id = format!("{evaluation_key}:run-{}-{}", args.run_id, args.run_attempt);
    let payload = build_check_run_payload(&result, &args.details_url, external_id.clone());
    let response = create_check_run(&args.repository, &payload)?;
    verify_check_run_response(&response, &payload)?;

    println!(
        "{}",
        serde_json::to_string_pretty(&PublishCheckResult {
            schema_version: CHECK_PUBLISH_RESULT_SCHEMA_VERSION,
            repository: args.repository,
            pull_request: args.pr,
            current_head_oid: result.current_head_oid,
            current_base_oid: result.current_base_oid,
            state: result.state,
            conclusion: payload.conclusion.to_string(),
            check_run_id: response.id,
            check_run_url: response.html_url,
            source_app: response.app.slug,
            external_id,
        })
        .map_err(|error| format!("无法序列化 shadow Check 发布结果：{error}"))?
    );
    Ok(())
}

pub fn evaluate_live(repository: &str, pr: u64) -> Result<ExternalReviewResult, String> {
    evaluate_live_with_optional_waiver(repository, pr, None)
}

pub(crate) fn evaluate_live_with_waiver(
    repository: &str,
    pr: u64,
    waiver: WaiverInput,
) -> Result<ExternalReviewResult, String> {
    evaluate_live_with_optional_waiver(repository, pr, Some(waiver))
}

fn evaluate_live_with_optional_waiver(
    repository: &str,
    pr: u64,
    waiver: Option<WaiverInput>,
) -> Result<ExternalReviewResult, String> {
    let snapshot = match waiver {
        Some(waiver) => load_live_waiver_snapshot(repository, pr, waiver)?,
        None => load_live_snapshot(repository, pr)?,
    };
    let initial_head = snapshot.pull_request.head_ref_oid.clone();
    let initial_base = snapshot.pull_request.base_ref_oid.clone();
    let mut result = evaluate_snapshot(&snapshot);
    let verified = load_live_identity(repository, pr)?;
    if verified.head_ref_oid != initial_head || verified.base_ref_oid != initial_base {
        result.set_provider_error(format!(
            "head/base 竞态：首次读取 {initial_head}/{initial_base}，发布前复核 {}/{}",
            verified.head_ref_oid, verified.base_ref_oid
        ));
    }
    Ok(result)
}

pub fn evaluate_snapshot(snapshot: &ExternalReviewSnapshot) -> ExternalReviewResult {
    let pr = &snapshot.pull_request;
    let author = pr
        .author
        .as_ref()
        .map(|actor| actor.login.clone())
        .unwrap_or_default();
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
    if author.is_empty() {
        diagnostics.push("PR author 缺失，无法排除 self-review".to_string());
    }
    if !valid_full_oid(&pr.head_ref_oid) {
        diagnostics.push("headRefOid 必须是 40 位十六进制 OID".to_string());
    }
    if !valid_full_oid(&pr.base_ref_oid) {
        diagnostics.push("baseRefOid 必须是 40 位十六进制 OID".to_string());
    }
    collect_pagination_errors(pr, &mut diagnostics);
    let dependabot_completion = dependabot_lockfile_completion(pr);
    let mut dependabot_completion_event = dependabot_completion
        .map(|completion| (completion.committed_date.as_str(), completion.url.as_str()));

    let mut review_to_finding_threads = BTreeMap::<String, BTreeSet<String>>::new();
    let mut finding_thread_ids = BTreeSet::<String>::new();
    let mut unresolved_actionable_threads = 0;
    let mut seen_thread_ids = BTreeSet::new();
    for thread in &pr.review_threads.nodes {
        if !seen_thread_ids.insert(thread.id.as_str()) {
            diagnostics.push(format!("重复 review thread id：{}", thread.id));
            continue;
        }
        let Some(first_comment) = thread.comments.nodes.first() else {
            diagnostics.push(format!("review thread `{}` 没有 comment", thread.id));
            continue;
        };
        if let Some(completion) = dependabot_completion
            && let Some(disposition) = dependabot_lockfile_false_positive_disposition(
                thread,
                completion,
                &pr.head_ref_oid,
                &author,
            )
        {
            let Some(review) = first_comment.pull_request_review.as_ref() else {
                diagnostics.push(format!(
                    "Dependabot 已知误报 thread `{}` 缺少 pullRequestReview 关联",
                    thread.id
                ));
                continue;
            };
            if let Err(error) =
                validate_review_reference_in_connection(&thread.id, review, &pr.reviews.nodes)
            {
                diagnostics.push(error);
                continue;
            }
            if dependabot_completion_event
                .is_none_or(|current| timestamp_second(disposition.0) > timestamp_second(current.0))
            {
                dependabot_completion_event = Some(disposition);
            }
            continue;
        }
        let has_authorless_comment = thread
            .comments
            .nodes
            .iter()
            .any(|comment| comment.author.is_none());
        let mut has_trusted_finding = false;
        for (index, comment) in thread.comments.nodes.iter().enumerate() {
            let Some(actor) = comment.author.as_ref() else {
                continue;
            };
            if trusted_provider(&actor.login, &author).is_none() {
                continue;
            }
            let Some(review) = comment.pull_request_review.as_ref() else {
                if index == 0 {
                    diagnostics.push(format!(
                        "受信任 reviewer 的 thread `{}` 缺少 pullRequestReview 关联",
                        thread.id
                    ));
                }
                continue;
            };
            let Some(review_actor) = review.author.as_ref() else {
                diagnostics.push(format!(
                    "受信任 reviewer 的 thread `{}` 关联 review 缺少 author",
                    thread.id
                ));
                continue;
            };
            if normalize_actor(&review_actor.login) != normalize_actor(&actor.login) {
                diagnostics.push(format!(
                    "review thread `{}` 的 comment actor 与 review actor 不一致",
                    thread.id
                ));
                continue;
            }
            has_trusted_finding = true;
            finding_thread_ids.insert(thread.id.clone());
            review_to_finding_threads
                .entry(review.id.clone())
                .or_default()
                .insert(thread.id.clone());
        }
        if !thread.is_resolved
            && !thread.is_outdated
            && (has_trusted_finding || (dependabot_completion.is_some() && has_authorless_comment))
        {
            unresolved_actionable_threads += 1;
        }
    }
    let review_ids = pr
        .reviews
        .nodes
        .iter()
        .map(|review| review.id.as_str())
        .collect::<BTreeSet<_>>();
    for review_id in review_to_finding_threads.keys() {
        if !review_ids.contains(review_id.as_str()) {
            diagnostics.push(format!(
                "review thread 引用了 reviews connection 中不存在的 review：{review_id}"
            ));
        }
    }

    let mut evidence = Vec::new();
    let mut unbound_clean_ambiguities = Vec::new();
    let mut stale_or_dismissed = false;
    let mut unthreaded_findings = 0;
    for review in &pr.reviews.nodes {
        let Some(actor) = review.author.as_ref() else {
            continue;
        };
        let Some(provider) = trusted_provider(&actor.login, &author) else {
            continue;
        };
        let actor_login = normalize_actor(&actor.login);
        let state = review.state.to_ascii_uppercase();
        if state == "DISMISSED" {
            stale_or_dismissed = true;
            continue;
        }

        let linked_findings = review_to_finding_threads
            .get(&review.id)
            .map_or(0, |threads| threads.len());
        let outcome = match provider {
            "copilot" if state == "COMMENTED" || state == "APPROVED" => {
                match copilot_outcome(&review.body, linked_findings) {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        diagnostics.push(format!("Copilot review `{}`: {error}", review.id));
                        None
                    }
                }
            }
            "codex" if state == "COMMENTED" && linked_findings > 0 => {
                Some(EvidenceOutcome::Findings)
            }
            "codex" if state == "APPROVED" => Some(EvidenceOutcome::Clean),
            "human" if state == "APPROVED" => Some(EvidenceOutcome::Clean),
            "human" if state == "CHANGES_REQUESTED" => Some(EvidenceOutcome::Findings),
            _ => None,
        };
        let Some(outcome) = outcome else {
            continue;
        };

        if outcome == EvidenceOutcome::Findings && linked_findings == 0 {
            unthreaded_findings += 1;
        }
        let Some(submitted_at) = review.submitted_at.as_deref() else {
            diagnostics.push(format!(
                "completion review `{}` 缺少 submittedAt",
                review.id
            ));
            continue;
        };
        let Some(reviewed_head) = review.commit.as_ref().map(|commit| commit.oid.as_str()) else {
            diagnostics.push(format!("completion review `{}` 缺少 commit OID", review.id));
            continue;
        };
        let Some(url) = review.url.as_deref() else {
            diagnostics.push(format!(
                "completion review `{}` 缺少 evidence URL",
                review.id
            ));
            continue;
        };
        push_evidence(
            &mut evidence,
            &mut diagnostics,
            EvidenceInput {
                provider,
                actor: &actor_login,
                source_kind: "review",
                reviewed_head,
                reviewed_base: &pr.base_ref_oid,
                outcome,
                submitted_at,
                evidence_url: url,
            },
        );
    }

    let binding_records = collect_codex_clean_binding_records(pr, &mut diagnostics);
    let bindings = adjudicate_codex_clean_bindings(pr, &binding_records, &mut diagnostics);
    let mut consumed_binding_records = BTreeSet::<String>::new();
    for comment in &pr.comments.nodes {
        let Some(actor) = comment.author.as_ref() else {
            continue;
        };
        if normalize_actor(&actor.login) != CODEX_ACTOR {
            continue;
        }
        if comment.body.contains("To use Codex here") && dependabot_completion.is_none() {
            diagnostics.push(format!("Codex provider 报告环境不可用：{}", comment.url));
            continue;
        }
        if comment.body.contains("To use Codex here") {
            continue;
        }
        if !codex_clean_comment_shape(&comment.body) {
            continue;
        }
        if comment.updated_at != comment.created_at {
            diagnostics.push(format!(
                "Codex clean comment `{}` 在创建后被编辑，不能作为 append-only completion",
                comment.id
            ));
            continue;
        }
        let Some(reviewed_head) = parse_reviewed_commit(&comment.body) else {
            match bindings.get(comment.id.as_str()) {
                Some(Some(bound)) => {
                    consumed_binding_records.insert(bound.record.id.clone());
                    push_evidence(
                        &mut evidence,
                        &mut diagnostics,
                        EvidenceInput {
                            provider: "codex",
                            actor: CODEX_ACTOR,
                            source_kind: "binding_record",
                            reviewed_head: &bound.record.bound_head_oid,
                            reviewed_base: &bound.record.bound_base_oid,
                            outcome: EvidenceOutcome::Clean,
                            // completion 排序使用被引用 clean comment 的创建时间；
                            // record 的创建时间只证明 head 关联与 record 自身合法性。
                            submitted_at: &comment.created_at,
                            evidence_url: &bound.url,
                        },
                    );
                }
                Some(None) => {}
                None => {
                    push_unbound_clean_ambiguity(
                        &mut unbound_clean_ambiguities,
                        &mut diagnostics,
                        comment,
                    );
                }
            }
            continue;
        };
        push_evidence(
            &mut evidence,
            &mut diagnostics,
            EvidenceInput {
                provider: "codex",
                actor: CODEX_ACTOR,
                source_kind: "issue_comment",
                reviewed_head,
                reviewed_base: &pr.base_ref_oid,
                outcome: EvidenceOutcome::Clean,
                submitted_at: &comment.created_at,
                evidence_url: &comment.url,
            },
        );
    }

    for bound in &binding_records {
        if !consumed_binding_records.contains(&bound.record.id) {
            diagnostics.push(format!(
                "Codex clean binding record `{}` 未绑定到任何无 SHA clean comment",
                bound.record.id
            ));
        }
    }

    if let (Some(completion), Some((completion_time, completion_url))) = (
        dependabot_completion.filter(|_| {
            finding_thread_ids.is_empty()
                && unthreaded_findings == 0
                && unresolved_actionable_threads == 0
                && !stale_or_dismissed
        }),
        dependabot_completion_event,
    ) {
        push_evidence(
            &mut evidence,
            &mut diagnostics,
            EvidenceInput {
                provider: "dependabot_lockfile_policy",
                actor: "github-metadata",
                source_kind: "machine_verification",
                reviewed_head: &completion.oid,
                reviewed_base: &pr.base_ref_oid,
                outcome: EvidenceOutcome::Clean,
                submitted_at: completion_time,
                evidence_url: completion_url,
            },
        );
    }

    evidence.sort_by(|left, right| {
        left.submitted_at
            .cmp(&right.submitted_at)
            .then_with(|| left.evidence_url.cmp(&right.evidence_url))
    });

    let pending_review_requests = pr
        .review_requests
        .nodes
        .iter()
        .filter(|request| {
            request
                .requested_reviewer
                .as_ref()
                .is_some_and(|reviewer| reviewer.login.is_some() || reviewer.name.is_some())
        })
        .count();

    let waiver_id = snapshot.waiver.as_ref().map(|waiver| waiver.id.clone());
    if let Some(waiver) = snapshot.waiver.as_ref() {
        validate_waiver(waiver, pr, &mut diagnostics);
    }

    let current_evidence = evidence
        .iter()
        .filter(|item| oid_matches_current(&item.reviewed_head_oid, &pr.head_ref_oid))
        .collect::<Vec<_>>();
    let latest_clean = current_evidence
        .iter()
        .rev()
        .find(|item| item.outcome == EvidenceOutcome::Clean)
        .copied();
    let latest_finding = current_evidence
        .iter()
        .rev()
        .find(|item| item.outcome == EvidenceOutcome::Findings)
        .copied();
    for ambiguity in &unbound_clean_ambiguities {
        let superseded = latest_clean.is_some_and(|clean| {
            timestamp_second(&clean.submitted_at) > timestamp_second(&ambiguity.created_at)
        });
        if !superseded {
            diagnostics.push(format!(
                "Codex clean comment `{}` 缺少可解析的 Reviewed commit，且没有严格晚于它的 current-head clean completion",
                ambiguity.id
            ));
        }
    }
    let finding_count = finding_thread_ids.len() + unthreaded_findings;

    let (state, requires_rereview, primary, state_diagnostic) = if !diagnostics.is_empty() {
        (
            ExternalReviewState::ProviderError,
            false,
            None,
            Some("provider/API/schema 歧义，按 fail-closed 处理".to_string()),
        )
    } else if pr.is_draft {
        (
            ExternalReviewState::ReviewPending,
            false,
            current_evidence.last().copied(),
            Some("Draft PR 尚未进入可计数的 external review Gate".to_string()),
        )
    } else if snapshot.waiver.is_some() {
        (
            ExternalReviewState::Waived,
            false,
            None,
            Some("存在完整结构化 waiver；不得映射为标准 pass".to_string()),
        )
    } else if let Some(finding) = latest_finding {
        let clean_after_finding =
            latest_clean.filter(|clean| clean.submitted_at > finding.submitted_at);
        if unresolved_actionable_threads > 0 {
            (
                ExternalReviewState::FindingsOpen,
                true,
                Some(finding),
                Some("current-head finding 仍有 unresolved actionable thread".to_string()),
            )
        } else if let Some(clean) = clean_after_finding {
            (ExternalReviewState::Pass, false, Some(clean), None)
        } else {
            (
                ExternalReviewState::AwaitingRereview,
                true,
                Some(finding),
                Some("finding 已处置，但缺少其后的 exact-head clean re-review".to_string()),
            )
        }
    } else if let Some(clean) = latest_clean {
        if unresolved_actionable_threads > 0 {
            (
                ExternalReviewState::FindingsOpen,
                true,
                Some(clean),
                Some("存在 unresolved actionable thread，clean completion 不足以放行".to_string()),
            )
        } else {
            (ExternalReviewState::Pass, false, Some(clean), None)
        }
    } else if !evidence.is_empty() || stale_or_dismissed {
        (
            ExternalReviewState::Stale,
            false,
            evidence.last(),
            Some("只有 old-head 或 dismissed completion".to_string()),
        )
    } else if pending_review_requests > 0 {
        (
            ExternalReviewState::ReviewPending,
            false,
            None,
            Some("存在 review request，但尚无有效 completion".to_string()),
        )
    } else {
        (
            ExternalReviewState::AwaitingReview,
            false,
            None,
            Some("尚无有效外部 review completion".to_string()),
        )
    };

    if let Some(diagnostic) = state_diagnostic {
        diagnostics.push(diagnostic);
    }

    ExternalReviewResult {
        schema_version: RESULT_SCHEMA_VERSION,
        repository: snapshot.repository.clone(),
        pull_request: pr.number,
        current_head_oid: pr.head_ref_oid.clone(),
        current_base_oid: pr.base_ref_oid.clone(),
        author,
        state,
        provider: primary.map(|item| item.provider.clone()),
        actor: primary.map(|item| item.actor.clone()),
        reviewed_head_oid: primary.map(|item| item.reviewed_head_oid.clone()),
        completion_time: primary.map(|item| item.submitted_at.clone()),
        finding_count,
        unresolved_actionable_threads,
        requires_rereview,
        pending_review_requests,
        evidence,
        waiver_id,
        diagnostics,
    }
}

pub fn run_request_codex_review(args: &[String]) -> Result<(), String> {
    let args = parse_request_codex_review_args(args)?;
    let identity = load_live_identity(&args.repository, args.pr)?;
    ensure_requestable_identity(&identity, args.pr)?;
    let record = CodexReviewRequestRecord {
        schema_version: BINDING_RECORD_SCHEMA_VERSION,
        id: format!("codex-review-request-{}-{}", args.pr, now_epoch_seconds()?),
        pr: args.pr,
        request_head_oid: identity.head_ref_oid.clone(),
        request_base_oid: identity.base_ref_oid.clone(),
    };
    let json = serde_json::to_string(&record)
        .map_err(|error| format!("无法序列化 codex-review-request 记录：{error}"))?;
    let body =
        format!("@codex review\n\n{CODEX_REVIEW_REQUEST_MARKER}{json}{HIDDEN_RECORD_SUFFIX}\n");
    if args.dry_run {
        println!("{body}");
        return Ok(());
    }
    let posted = post_issue_comment(&args.repository, args.pr, &body)?;
    ensure_trusted_comment_echo_or_delete(&posted, &body, |comment_id| {
        delete_issue_comment(&args.repository, comment_id)
    })?;
    println!(
        "{}",
        serde_json::to_string_pretty(&CodexReviewRequestResult {
            schema_version: BINDING_RECORD_SCHEMA_VERSION,
            repository: args.repository,
            pull_request: args.pr,
            request_id: record.id,
            request_head_oid: identity.head_ref_oid,
            request_base_oid: identity.base_ref_oid,
            comment_url: posted.html_url,
        })
        .map_err(|error| format!("无法序列化 codex-review-request 发布结果：{error}"))?
    );
    Ok(())
}

pub fn run_publish_codex_clean_binding(args: &[String]) -> Result<(), String> {
    let args = parse_publish_codex_clean_binding_args(args)?;
    let initial = load_live_identity(&args.repository, args.pr)?;
    ensure_requestable_identity(&initial, args.pr)?;
    let snapshot = load_live_snapshot(&args.repository, args.pr)?;
    if snapshot.pull_request.head_ref_oid != initial.head_ref_oid
        || snapshot.pull_request.base_ref_oid != initial.base_ref_oid
    {
        return Err(format!(
            "head/base 竞态：identity 读取 {}/{}，snapshot 读取 {}/{}",
            initial.head_ref_oid,
            initial.base_ref_oid,
            snapshot.pull_request.head_ref_oid,
            snapshot.pull_request.base_ref_oid
        ));
    }
    let pr = &snapshot.pull_request;
    let clean = pr
        .comments
        .nodes
        .iter()
        .find(|comment| comment.id == args.clean_comment_id)
        .ok_or_else(|| {
            format!(
                "clean comment `{}` 不在 PR #{} 的 comments connection 中",
                args.clean_comment_id, args.pr
            )
        })?;
    let clean_actor = clean
        .author
        .as_ref()
        .map_or("", |actor| actor.login.as_str());
    if normalize_actor(clean_actor) != CODEX_ACTOR {
        return Err(format!(
            "comment `{}` 的 actor `{clean_actor}` 不是受信 Codex provider",
            clean.id
        ));
    }
    if !codex_clean_comment_shape(&clean.body) {
        return Err(format!(
            "comment `{}` 不匹配封闭 clean grammar，拒绝绑定",
            clean.id
        ));
    }
    if clean.updated_at != clean.created_at {
        return Err(format!(
            "clean comment `{}` 在创建后被编辑，拒绝绑定",
            clean.id
        ));
    }
    if !valid_timestamp(&clean.created_at) || !valid_github_url(&clean.url) {
        return Err(format!(
            "clean comment `{}` 的 createdAt/URL 无效，拒绝绑定",
            clean.id
        ));
    }
    let mut record_diagnostics = Vec::new();
    let mut records = collect_codex_clean_binding_records(pr, &mut record_diagnostics);
    if !record_diagnostics.is_empty() {
        return Err(format!(
            "PR #{} 已存在 malformed binding record：{}",
            args.pr,
            record_diagnostics.join("；")
        ));
    }
    if parse_reviewed_commit(&clean.body).is_some() {
        print_codex_clean_binding_skip(
            &args,
            "clean comment 已携带 Reviewed commit marker，走标准路径",
        )?;
    } else if let Some(bound) = records
        .iter()
        .find(|bound| bound.record.clean_comment_id == clean.id)
    {
        print_codex_clean_binding_skip(
            &args,
            &format!("clean comment 已被 record `{}` 绑定", bound.record.id),
        )?;
    } else {
        let (marker_comment, marker) = select_request_marker(pr, clean, &records)?;
        publish_single_binding(
            &args,
            &initial,
            clean,
            marker_comment,
            &marker,
            &mut records,
        )?;
    }

    // 与 workflow concurrency 组配对：串行化保证同一 PR 任意时刻只有一个
    // publisher run（消除并发读-选-发竞态）；同组只允许一个 pending run，
    // 被挤掉的中间 issue_comment 事件由此处 sweep 补齐——每个 run 在处理完
    // 触发事件后，为其余所有未绑定的 SHA-less clean 补发 record。
    let mut sweep_diagnostics = Vec::new();
    while let Some((sweep_clean, marker_comment, marker)) =
        plan_next_sweep_binding(pr, &records, &mut sweep_diagnostics)
    {
        publish_single_binding(
            &args,
            &initial,
            sweep_clean,
            marker_comment,
            &marker,
            &mut records,
        )?;
    }
    for diagnostic in &sweep_diagnostics {
        eprintln!("sweep diagnostic: {diagnostic}");
    }
    Ok(())
}

/// 发布单条绑定 record：binds current head 时先做 no-push 窗口检查与发布前
/// identity 复核，然后发布（dry-run 仅打印）；无论哪条路径都把 record 记入
/// records，供同一次 run 内后续 clean 的 marker 消费判定使用。
fn publish_single_binding(
    args: &PublishCodexCleanBindingArgs,
    initial: &PullRequestIdentity,
    clean: &IssueComment,
    marker_comment: &IssueComment,
    marker: &CodexReviewRequestRecord,
    records: &mut Vec<BoundCleanRecord>,
) -> Result<(), String> {
    let binds_current_head = marker.request_head_oid == initial.head_ref_oid
        && marker.request_base_oid == initial.base_ref_oid;
    if binds_current_head {
        ensure_no_push_in_window(
            &args.repository,
            args.pr,
            &initial.head_ref_name,
            &initial.head_ref_oid,
            &marker_comment.created_at,
        )?;
        let final_identity = load_live_identity(&args.repository, args.pr)?;
        if final_identity.head_ref_oid != initial.head_ref_oid
            || final_identity.base_ref_oid != initial.base_ref_oid
        {
            return Err(format!(
                "head/base 竞态：绑定判定前读取 {}/{}，发布前复核 {}/{}",
                initial.head_ref_oid,
                initial.base_ref_oid,
                final_identity.head_ref_oid,
                final_identity.base_ref_oid
            ));
        }
    }
    // marker head 落后于 current head（跨 push 迟到的旧 head 响应）时仍发布：
    // record 携带 marker 的 head/base，evaluator 会落 stale；
    // 这消费掉迟到的旧 head 响应，避免后续 clean 永久卡死，无需 no-push 窗口检查。

    let record = CodexCleanBindingRecord {
        schema_version: BINDING_RECORD_SCHEMA_VERSION,
        id: format!("codex-clean-binding-{}", short_digest(&clean.id)),
        pr: args.pr,
        clean_comment_id: clean.id.clone(),
        clean_comment_created_at: clean.created_at.clone(),
        clean_comment_url: clean.url.clone(),
        request_marker_id: marker_comment.id.clone(),
        bound_head_oid: marker.request_head_oid.clone(),
        bound_base_oid: marker.request_base_oid.clone(),
        verified_at: now_rfc3339()?,
        run_url: args.run_url.clone(),
    };
    let json = serde_json::to_string(&record)
        .map_err(|error| format!("无法序列化 codex-clean-binding 记录：{error}"))?;
    let body = format!(
        "external-review: codex clean bound to `{}` via controlled request marker\n\n{CODEX_CLEAN_BINDING_MARKER}{json}{HIDDEN_RECORD_SUFFIX}\n",
        &marker.request_head_oid[..12]
    );
    if args.dry_run {
        println!("{body}");
        records.push(BoundCleanRecord {
            created_at: record.verified_at.clone(),
            url: String::new(),
            record,
        });
        return Ok(());
    }
    let posted = post_issue_comment(&args.repository, args.pr, &body)?;
    ensure_trusted_comment_echo_or_delete(&posted, &body, |comment_id| {
        delete_issue_comment(&args.repository, comment_id)
    })?;
    let comment_url = posted.html_url;
    println!(
        "{}",
        serde_json::to_string_pretty(&CodexCleanBindingResult {
            schema_version: BINDING_RECORD_SCHEMA_VERSION,
            repository: args.repository.clone(),
            pull_request: args.pr,
            skipped: false,
            reason: None,
            binding_id: Some(record.id.clone()),
            bound_head_oid: Some(record.bound_head_oid.clone()),
            comment_url: Some(comment_url.clone()),
        })
        .map_err(|error| format!("无法序列化 codex-clean-binding 发布结果：{error}"))?
    );
    records.push(BoundCleanRecord {
        created_at: record.verified_at.clone(),
        url: comment_url,
        record,
    });
    Ok(())
}

/// 为 sweep 计划下一条绑定：在 records 视角下按 created_at 升序找到第一个
/// 可绑定（与 evaluate_snapshot 的 comment 闸门同规则）且尚未绑定的 SHA-less
/// clean，并为其选择受控 request marker。无可绑定对象时返回 None；marker
/// 缺失/歧义的 clean 记入 diagnostics 并跳过（evaluator 会独立对其
/// fail-closed 诊断，publisher 不在此不完整证据下发布任何 record）。
fn plan_next_sweep_binding<'a>(
    pr: &'a PullRequestSnapshot,
    records: &[BoundCleanRecord],
    diagnostics: &mut Vec<String>,
) -> Option<(&'a IssueComment, &'a IssueComment, CodexReviewRequestRecord)> {
    let mut candidates: Vec<&IssueComment> = pr
        .comments
        .nodes
        .iter()
        .filter(|comment| sha_less_bindable_clean(comment))
        .filter(|comment| {
            !records
                .iter()
                .any(|bound| bound.record.clean_comment_id == comment.id)
        })
        .collect();
    candidates.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    for clean in candidates {
        match select_request_marker(pr, clean, records) {
            Ok((marker_comment, marker)) => return Some((clean, marker_comment, marker)),
            Err(error) => {
                diagnostics.push(format!("sweep 跳过 clean `{}`：{error}", clean.id));
            }
        }
    }
    None
}

fn print_codex_clean_binding_skip(
    args: &PublishCodexCleanBindingArgs,
    reason: &str,
) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(&CodexCleanBindingResult {
            schema_version: BINDING_RECORD_SCHEMA_VERSION,
            repository: args.repository.clone(),
            pull_request: args.pr,
            skipped: true,
            reason: Some(reason.to_string()),
            binding_id: None,
            bound_head_oid: None,
            comment_url: None,
        })
        .map_err(|error| format!("无法序列化 codex-clean-binding skip 结果：{error}"))?
    );
    Ok(())
}

/// publisher 与 evaluator 镜像同一确定性规则：候选 = clean 之前（秒粒度严格
/// 更早）未被既有合法 binding record 消费的受控 request marker。候选 head/base
/// 不全相同时无法证明 clean 归属，拒绝发布任何 record；全同（含只有一条）时
/// 绑定最早者，record 携带该 head/base。
fn select_request_marker<'a>(
    pr: &'a PullRequestSnapshot,
    clean: &IssueComment,
    existing_records: &[BoundCleanRecord],
) -> Result<(&'a IssueComment, CodexReviewRequestRecord), String> {
    let mut candidates = Vec::new();
    for comment in &pr.comments.nodes {
        if !comment.body.contains(CODEX_REVIEW_REQUEST_MARKER) {
            continue;
        }
        let marker = parse_request_marker_comment(comment, pr.number)?;
        if !valid_full_oid(&marker.request_head_oid) || !valid_full_oid(&marker.request_base_oid) {
            return Err(format!(
                "受控 request marker `{}` 的 request head/base 不是完整 OID",
                comment.id
            ));
        }
        candidates.push((comment, marker));
    }
    let clean_second = timestamp_second(&clean.created_at);
    if candidates
        .iter()
        .any(|(comment, _)| timestamp_second(&comment.created_at) == clean_second)
    {
        return Err("受控 request marker 与 clean comment 同秒，无法证明先后".to_string());
    }
    candidates.retain(|(comment, _)| timestamp_second(&comment.created_at) < clean_second);
    candidates.sort_by(|left, right| {
        left.0
            .created_at
            .cmp(&right.0.created_at)
            .then_with(|| left.0.id.cmp(&right.0.id))
    });
    let consumed: BTreeSet<&str> = existing_records
        .iter()
        .map(|bound| bound.record.request_marker_id.as_str())
        .collect();
    let had_candidates = !candidates.is_empty();
    let unconsumed: Vec<(&IssueComment, CodexReviewRequestRecord)> = candidates
        .into_iter()
        .filter(|(comment, _)| !consumed.contains(comment.id.as_str()))
        .collect();
    let Some((first_comment, first_marker)) = unconsumed.first() else {
        return Err(if had_candidates {
            format!(
                "clean comment `{}` 之前的受控 request marker 均已被既有 binding record 消费，拒绝绑定",
                clean.id
            )
        } else {
            format!(
                "clean comment `{}` 之前不存在受控 request marker，拒绝绑定",
                clean.id
            )
        });
    };
    let mixed_head_base = unconsumed.iter().any(|(_, marker)| {
        marker.request_head_oid != first_marker.request_head_oid
            || marker.request_base_oid != first_marker.request_base_oid
    });
    if mixed_head_base {
        return Err(format!(
            "clean comment `{}` 之前存在多个 head/base 不同的未消费受控 request marker，无法证明 clean 归属，拒绝绑定",
            clean.id
        ));
    }
    Ok((*first_comment, first_marker.clone()))
}

fn ensure_no_push_in_window(
    repository: &str,
    pr: u64,
    head_branch: &str,
    expected_head: &str,
    marker_created_at: &str,
) -> Result<(), String> {
    let marker_second = timestamp_second(marker_created_at);
    let events: Vec<RepoEvent> =
        fetch_paginated(&format!("repos/{repository}/events?per_page=100"))?;
    let window_uncovered = events.len() >= 300
        && events
            .last()
            .and_then(|event| event.created_at.as_deref())
            .is_none_or(|oldest| {
                !valid_timestamp(oldest) || timestamp_second(oldest) >= marker_second
            });
    if window_uncovered {
        return Err(
            "Events API 无法覆盖完整窗口（feed 截断或最旧事件仍在窗口内），放弃绑定".to_string(),
        );
    }
    let head_ref = format!("refs/heads/{head_branch}");
    for event in &events {
        if event.kind != "PushEvent" || event.payload.ref_name.as_deref() != Some(head_ref.as_str())
        {
            continue;
        }
        let Some(created) = event.created_at.as_deref() else {
            return Err(format!(
                "head 分支 `{head_branch}` 的 PushEvent 缺少 created_at，放弃绑定"
            ));
        };
        if !valid_timestamp(created) {
            return Err(format!(
                "head 分支 `{head_branch}` 的 PushEvent created_at 不是 UTC RFC3339：{created}"
            ));
        }
        if timestamp_second(created) >= marker_second {
            return Err(format!(
                "窗口内检测到 head 分支 `{head_branch}` 的 push（{created}），放弃绑定"
            ));
        }
    }
    let timeline: Vec<TimelineEvent> = fetch_paginated(&format!(
        "repos/{repository}/issues/{pr}/timeline?per_page=100"
    ))?;
    for event in &timeline {
        if event.event.as_deref() != Some("head_ref_force_pushed") {
            continue;
        }
        let Some(created) = event.created_at.as_deref() else {
            return Err("head_ref_force_pushed 事件缺少 created_at，放弃绑定".to_string());
        };
        if !valid_timestamp(created) {
            return Err(format!(
                "head_ref_force_pushed 事件 created_at 不是 UTC RFC3339：{created}"
            ));
        }
        if timestamp_second(created) >= marker_second {
            return Err(format!(
                "窗口内检测到 PR #{pr} 的 head_ref_force_pushed（{created}），放弃绑定"
            ));
        }
    }
    let branch_head = fetch_branch_head(repository, head_branch)?;
    if branch_head != expected_head {
        return Err(format!(
            "head 分支 `{head_branch}` 当前指向 {branch_head}，与 PR head {expected_head} 不一致，放弃绑定"
        ));
    }
    Ok(())
}

fn fetch_paginated<T: for<'de> Deserialize<'de>>(endpoint: &str) -> Result<Vec<T>, String> {
    let output = Command::new("gh")
        .args([
            "api",
            endpoint,
            "--paginate",
            "--slurp",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "X-GitHub-Api-Version: 2022-11-28",
        ])
        .output()
        .map_err(|error| format!("无法运行 gh REST API：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh REST API `{endpoint}` 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let pages = serde_json::from_slice::<Vec<Vec<T>>>(&output.stdout).map_err(|error| {
        format!(
            "gh REST API `{endpoint}` 输出不是分页 JSON：{error}；原始输出：{}",
            String::from_utf8_lossy(&output.stdout).trim()
        )
    })?;
    Ok(pages.into_iter().flatten().collect())
}

/// 分支名作为 URL path 参数时逐 segment percent-encode：`#`/`?` 等字符否则会被
/// 当作 fragment/query 截断；保留 `/` 以维持 GitHub branches endpoint 对多级
/// 分支名的匹配。
fn encode_path_segment_preserving_slashes(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                encoded.push(char::from(byte))
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn branch_endpoint(repository: &str, branch: &str) -> String {
    format!(
        "repos/{repository}/branches/{}",
        encode_path_segment_preserving_slashes(branch)
    )
}

fn fetch_branch_head(repository: &str, branch: &str) -> Result<String, String> {
    let output = Command::new("gh")
        .args([
            "api",
            &branch_endpoint(repository, branch),
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "X-GitHub-Api-Version: 2022-11-28",
        ])
        .output()
        .map_err(|error| format!("无法运行 gh branch API：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh branch API 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let info = serde_json::from_slice::<BranchInfo>(&output.stdout).map_err(|error| {
        format!(
            "gh branch API 输出不是预期 JSON：{error}；原始输出：{}",
            String::from_utf8_lossy(&output.stdout).trim()
        )
    })?;
    if !valid_full_oid(&info.commit.sha) {
        return Err(format!(
            "分支 `{branch}` 的 head 不是 40 位十六进制 OID：{}",
            info.commit.sha
        ));
    }
    Ok(info.commit.sha)
}

fn post_issue_comment(repository: &str, pr: u64, body: &str) -> Result<PostedComment, String> {
    let payload = serde_json::to_vec(&serde_json::json!({ "body": body }))
        .map_err(|error| format!("无法序列化 comment payload：{error}"))?;
    let endpoint = format!("repos/{repository}/issues/{pr}/comments");
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
        .map_err(|error| format!("无法启动 gh comment API：{error}"))?;
    child
        .stdin
        .as_mut()
        .ok_or("无法打开 gh comment API stdin")?
        .write_all(&payload)
        .map_err(|error| format!("无法写入 gh comment payload：{error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("无法等待 gh comment API：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh comment API 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        format!(
            "gh comment API 输出不是预期 JSON：{error}；原始输出：{}",
            String::from_utf8_lossy(&output.stdout).trim()
        )
    })
}

fn ensure_trusted_comment_echo(posted: &PostedComment, expected_body: &str) -> Result<(), String> {
    let login = posted.user.as_ref().map_or("", |user| user.login.as_str());
    if normalize_actor(login) != GITHUB_ACTIONS_ACTOR {
        return Err(format!(
            "comment 发布者 `{login}` 不是受信 publisher github-actions[bot]；拒绝承认该记录"
        ));
    }
    if posted.body.as_deref() != Some(expected_body) {
        return Err("comment echo body 与发布内容不一致".to_string());
    }
    if posted.node_id.is_empty() || !valid_github_url(&posted.html_url) {
        return Err("comment 响应缺少 node_id 或 GitHub HTTPS URL".to_string());
    }
    Ok(())
}

/// post→verify→delete：echo 校验失败时删除刚发布的 comment，避免不可信
/// marker/record 残留导致后续受信发布者 fail-closed；删除结果写进错误信息，
/// 命令仍然返回错误。
fn ensure_trusted_comment_echo_or_delete(
    posted: &PostedComment,
    expected_body: &str,
    mut delete_comment: impl FnMut(u64) -> Result<(), String>,
) -> Result<(), String> {
    if let Err(error) = ensure_trusted_comment_echo(posted, expected_body) {
        let cleanup = match delete_comment(posted.id) {
            Ok(()) => format!("已删除不可信 comment {}", posted.id),
            Err(delete_error) => format!(
                "删除不可信 comment {} 失败：{delete_error}；需人工删除 {}",
                posted.id, posted.html_url
            ),
        };
        return Err(format!("{error}；{cleanup}"));
    }
    Ok(())
}

fn issue_comment_endpoint(repository: &str, comment_id: u64) -> String {
    format!("repos/{repository}/issues/comments/{comment_id}")
}

fn delete_issue_comment(repository: &str, comment_id: u64) -> Result<(), String> {
    let endpoint = issue_comment_endpoint(repository, comment_id);
    let output = Command::new("gh")
        .args([
            "api",
            "--method",
            "DELETE",
            &endpoint,
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "X-GitHub-Api-Version: 2022-11-28",
        ])
        .output()
        .map_err(|error| format!("无法运行 gh comment DELETE API：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh comment DELETE API 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn ensure_requestable_identity(identity: &PullRequestIdentity, pr: u64) -> Result<(), String> {
    if identity.number != pr {
        return Err(format!(
            "GitHub PR identity number 不一致：请求 #{pr}，返回 #{}",
            identity.number
        ));
    }
    if identity.state != "OPEN" {
        return Err(format!("PR #{pr} 已不是 OPEN 状态"));
    }
    if identity.is_draft {
        return Err(format!("PR #{pr} 是 Draft，不进入 Codex clean 绑定路径"));
    }
    if identity.is_cross_repository {
        return Err(format!(
            "PR #{pr} 是 fork / cross-repository PR，Codex clean 绑定只支持 same-repository head"
        ));
    }
    if identity.base_ref_name != "main" {
        return Err(format!("PR #{pr} 不以 main 为 base，不属于本绑定路径范围"));
    }
    if identity.head_ref_name.is_empty() {
        return Err(format!("PR #{pr} 缺少 headRefName"));
    }
    if !valid_full_oid(&identity.head_ref_oid) || !valid_full_oid(&identity.base_ref_oid) {
        return Err(format!("PR #{pr} 的 head/base OID 无效"));
    }
    Ok(())
}

fn now_epoch_seconds() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("系统时间早于 Unix epoch：{error}"))
        .map(|duration| duration.as_secs())
}

fn now_rfc3339() -> Result<String, String> {
    Ok(epoch_seconds_to_rfc3339(now_epoch_seconds()?))
}

fn epoch_seconds_to_rfc3339(seconds: u64) -> String {
    let days = seconds / 86_400;
    let remaining = seconds % 86_400;
    let hour = remaining / 3_600;
    let minute = remaining % 3_600 / 60;
    let second = remaining % 60;
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    if month <= 2 {
        year += 1;
    }
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn short_digest(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct EvidenceInput<'a> {
    provider: &'a str,
    actor: &'a str,
    source_kind: &'a str,
    reviewed_head: &'a str,
    reviewed_base: &'a str,
    outcome: EvidenceOutcome,
    submitted_at: &'a str,
    evidence_url: &'a str,
}

struct UnboundCleanAmbiguity {
    id: String,
    created_at: String,
}

fn push_unbound_clean_ambiguity(
    ambiguities: &mut Vec<UnboundCleanAmbiguity>,
    diagnostics: &mut Vec<String>,
    comment: &IssueComment,
) {
    if !valid_timestamp(&comment.created_at) {
        diagnostics.push(format!(
            "Codex clean comment `{}` 的 createdAt 不是 UTC RFC3339：{}",
            comment.id, comment.created_at
        ));
        return;
    }
    if !valid_github_url(&comment.url) {
        diagnostics.push(format!(
            "Codex clean comment `{}` 的 evidence URL 不是 GitHub HTTPS URL：{}",
            comment.id, comment.url
        ));
        return;
    }
    ambiguities.push(UnboundCleanAmbiguity {
        id: comment.id.clone(),
        created_at: comment.created_at.clone(),
    });
}

fn push_evidence(
    evidence: &mut Vec<ReviewEvidence>,
    diagnostics: &mut Vec<String>,
    input: EvidenceInput<'_>,
) {
    if !valid_oid_fragment(input.reviewed_head) {
        diagnostics.push(format!(
            "{} evidence 的 reviewed head 不是 7-40 位十六进制 OID：{}",
            input.provider, input.reviewed_head
        ));
        return;
    }
    if !valid_timestamp(input.submitted_at) {
        diagnostics.push(format!(
            "{} evidence 的 completion time 不是 UTC RFC3339：{}",
            input.provider, input.submitted_at
        ));
        return;
    }
    if !valid_github_url(input.evidence_url) {
        diagnostics.push(format!(
            "{} evidence URL 不是 GitHub HTTPS URL：{}",
            input.provider, input.evidence_url
        ));
        return;
    }
    evidence.push(ReviewEvidence {
        provider: input.provider.to_string(),
        actor: input.actor.to_string(),
        source_kind: input.source_kind.to_string(),
        reviewed_head_oid: input.reviewed_head.to_ascii_lowercase(),
        reviewed_base_oid: input.reviewed_base.to_string(),
        outcome: input.outcome,
        submitted_at: input.submitted_at.to_string(),
        evidence_url: input.evidence_url.to_string(),
    });
}

struct BoundCleanRecord {
    record: CodexCleanBindingRecord,
    created_at: String,
    url: String,
}

fn codex_clean_comment_shape(body: &str) -> bool {
    body.contains("Codex Review:") && body.contains("Didn't find any major issues")
}

fn parse_hidden_record<T: for<'de> Deserialize<'de>>(
    body: &str,
    marker: &str,
) -> Option<Result<T, String>> {
    let start = body.find(marker)?;
    let rest = &body[start + marker.len()..];
    if rest.contains(marker) {
        return Some(Err(format!("包含多个 `{marker}` 记录起始标记")));
    }
    let Some(end) = rest.find(HIDDEN_RECORD_SUFFIX) else {
        return Some(Err("隐藏记录缺少 ` -->` 结束标记".to_string()));
    };
    Some(
        serde_json::from_str(rest[..end].trim())
            .map_err(|error| format!("隐藏记录不是合法 JSON：{error}")),
    )
}

fn collect_codex_clean_binding_records(
    pr: &PullRequestSnapshot,
    diagnostics: &mut Vec<String>,
) -> Vec<BoundCleanRecord> {
    let mut records: Vec<BoundCleanRecord> = Vec::new();
    let mut seen_keys = BTreeSet::new();
    let mut seen_ids = BTreeSet::new();
    let mut seen_clean = BTreeSet::new();
    for comment in &pr.comments.nodes {
        if !comment.body.contains(CODEX_CLEAN_BINDING_MARKER) {
            continue;
        }
        let record = match parse_hidden_record(&comment.body, CODEX_CLEAN_BINDING_MARKER) {
            Some(Ok(record)) => record,
            Some(Err(error)) => {
                diagnostics.push(format!(
                    "Codex clean binding comment `{}`：{error}",
                    comment.id
                ));
                continue;
            }
            None => continue,
        };
        let Some(bound) = validate_codex_clean_binding_record(comment, record, pr, diagnostics)
        else {
            continue;
        };
        // 发布重试/并发可能产生内容一致的重复 record，去重为一条；key 覆盖全部
        // 不可变 evidence 字段，仅排除随重试 run 变化的 verifiedAt/runUrl。
        // 创建者身份已由 validate 固定为受信 publisher，无需纳入 key；
        // schemaVersion 已由 validate 固定为 BINDING_RECORD_SCHEMA_VERSION。
        let record = &bound.record;
        let key = (
            record.id.clone(),
            record.pr,
            record.clean_comment_id.clone(),
            record.clean_comment_created_at.clone(),
            record.clean_comment_url.clone(),
            record.request_marker_id.clone(),
            record.bound_head_oid.clone(),
            record.bound_base_oid.clone(),
        );
        let record_id = record.id.clone();
        let clean_comment_id = record.clean_comment_id.clone();
        if !seen_keys.insert(key) {
            continue;
        }
        if !seen_ids.insert(record_id.clone()) {
            diagnostics.push(format!("重复 Codex clean binding record id：`{record_id}`"));
        }
        if !seen_clean.insert(clean_comment_id.clone()) {
            diagnostics.push(format!(
                "多个 Codex clean binding record 引用同一 clean comment `{clean_comment_id}`"
            ));
        }
        records.push(bound);
    }
    records
}

fn validate_codex_clean_binding_record(
    comment: &IssueComment,
    record: CodexCleanBindingRecord,
    pr: &PullRequestSnapshot,
    diagnostics: &mut Vec<String>,
) -> Option<BoundCleanRecord> {
    let actor = comment
        .author
        .as_ref()
        .map_or("", |actor| actor.login.as_str());
    if normalize_actor(actor) != GITHUB_ACTIONS_ACTOR {
        diagnostics.push(format!(
            "Codex clean binding comment `{}` 的 actor `{actor}` 不是受信 publisher",
            comment.id
        ));
        return None;
    }
    if comment.updated_at != comment.created_at {
        diagnostics.push(format!(
            "Codex clean binding comment `{}` 在创建后被编辑，不能作为 append-only 记录",
            comment.id
        ));
        return None;
    }
    if !valid_timestamp(&comment.created_at) {
        diagnostics.push(format!(
            "Codex clean binding comment `{}` 的 createdAt 不是 UTC RFC3339：{}",
            comment.id, comment.created_at
        ));
        return None;
    }
    if !valid_github_url(&comment.url) {
        diagnostics.push(format!(
            "Codex clean binding comment `{}` 的 URL 不是 GitHub HTTPS URL：{}",
            comment.id, comment.url
        ));
        return None;
    }
    let invalid = if record.schema_version != BINDING_RECORD_SCHEMA_VERSION {
        Some(format!(
            "schemaVersion 必须为 {BINDING_RECORD_SCHEMA_VERSION}"
        ))
    } else if record.id.is_empty() {
        Some("id 不能为空".to_string())
    } else if record.pr != pr.number {
        Some(format!("pr 与当前 PR #{} 不一致", pr.number))
    } else if !valid_full_oid(&record.bound_head_oid) {
        Some("boundHeadOid 必须是 40 位十六进制 OID".to_string())
    } else if !valid_full_oid(&record.bound_base_oid) {
        Some("boundBaseOid 必须是 40 位十六进制 OID".to_string())
    } else if !valid_timestamp(&record.clean_comment_created_at) {
        Some("cleanCommentCreatedAt 必须是 UTC RFC3339".to_string())
    } else if !valid_timestamp(&record.verified_at) {
        Some("verifiedAt 必须是 UTC RFC3339".to_string())
    } else if !valid_github_url(&record.clean_comment_url) {
        Some("cleanCommentUrl 必须是 GitHub HTTPS URL".to_string())
    } else if !valid_github_url(&record.run_url) {
        Some("runUrl 必须是 GitHub HTTPS URL".to_string())
    } else if record.request_marker_id.is_empty() {
        Some("requestMarkerId 不能为空".to_string())
    } else {
        None
    };
    if let Some(reason) = invalid {
        diagnostics.push(format!(
            "Codex clean binding record `{}` 字段无效：{reason}",
            record.id
        ));
        return None;
    }
    Some(BoundCleanRecord {
        record,
        created_at: comment.created_at.clone(),
        url: comment.url.clone(),
    })
}

/// 校验并解析受控 request marker comment 的隐藏记录；publisher 与 evaluator 共用。
fn parse_request_marker_comment(
    comment: &IssueComment,
    pr_number: u64,
) -> Result<CodexReviewRequestRecord, String> {
    let actor = comment
        .author
        .as_ref()
        .map_or("", |actor| actor.login.as_str());
    if normalize_actor(actor) != GITHUB_ACTIONS_ACTOR {
        return Err(format!(
            "受控 request marker `{}` 的 actor `{actor}` 不是受信 publisher",
            comment.id
        ));
    }
    if comment.updated_at != comment.created_at {
        return Err(format!(
            "受控 request marker `{}` 在创建后被编辑",
            comment.id
        ));
    }
    if !valid_timestamp(&comment.created_at) {
        return Err(format!(
            "受控 request marker `{}` 的 createdAt 不是 UTC RFC3339：{}",
            comment.id, comment.created_at
        ));
    }
    let marker = match parse_hidden_record(&comment.body, CODEX_REVIEW_REQUEST_MARKER) {
        Some(Ok(marker)) => marker,
        Some(Err(error)) => {
            return Err(format!("受控 request marker `{}`：{error}", comment.id));
        }
        None => {
            return Err(format!(
                "受控 request marker `{}` 不包含 `{CODEX_REVIEW_REQUEST_MARKER}` 记录",
                comment.id
            ));
        }
    };
    let marker: CodexReviewRequestRecord = marker;
    if marker.schema_version != BINDING_RECORD_SCHEMA_VERSION || marker.pr != pr_number {
        return Err(format!(
            "受控 request marker `{}` 的 schemaVersion/pr 与当前 PR #{} 不一致",
            comment.id, pr_number
        ));
    }
    Ok(marker)
}

fn resolve_request_marker(
    record: &CodexCleanBindingRecord,
    pr: &PullRequestSnapshot,
    clean: &IssueComment,
    diagnostics: &mut Vec<String>,
) -> bool {
    let marker_comment = pr
        .comments
        .nodes
        .iter()
        .find(|comment| comment.id == record.request_marker_id);
    let Some(marker_comment) = marker_comment else {
        diagnostics.push(format!(
            "Codex clean binding record `{}` 引用的受控 request marker `{}` 不存在",
            record.id, record.request_marker_id
        ));
        return false;
    };
    let marker = match parse_request_marker_comment(marker_comment, pr.number) {
        Ok(marker) => marker,
        Err(error) => {
            diagnostics.push(error);
            return false;
        }
    };
    if marker.request_head_oid != record.bound_head_oid
        || marker.request_base_oid != record.bound_base_oid
    {
        diagnostics.push(format!(
            "Codex clean binding record `{}` 的 bound head/base 与受控 marker `{}` 的 request head/base 不一致",
            record.id, marker_comment.id
        ));
        return false;
    }
    if timestamp_second(&marker_comment.created_at) >= timestamp_second(&clean.created_at) {
        diagnostics.push(format!(
            "受控 request marker `{}` 必须严格早于（秒粒度）clean comment `{}`",
            marker_comment.id, clean.id
        ));
        return false;
    }
    true
}

enum EarliestMarkerSelection<'a> {
    Selected(&'a IssueComment),
    Exhausted,
    Ambiguous,
    Invalid,
}

/// 返回 clean 之前（秒粒度严格更早）未被既有合法 binding record 消费的受控
/// request marker 候选；候选 malformed 时记诊断并返回 Invalid。无 SHA clean 与
/// 请求之间没有 provider 可验证的关联，候选集合的 head/base 不全相同时任何
/// 分配规则在乱序响应下都不安全，返回 Ambiguous 按歧义 fail-closed。
fn select_earliest_unconsumed_marker<'a>(
    pr: &'a PullRequestSnapshot,
    clean: &IssueComment,
    consumed_markers: &BTreeSet<&str>,
    diagnostics: &mut Vec<String>,
) -> EarliestMarkerSelection<'a> {
    let clean_second = timestamp_second(&clean.created_at);
    let mut candidates = Vec::new();
    for comment in &pr.comments.nodes {
        if !comment.body.contains(CODEX_REVIEW_REQUEST_MARKER) {
            continue;
        }
        let marker = match parse_request_marker_comment(comment, pr.number) {
            Ok(marker) => marker,
            Err(error) => {
                diagnostics.push(error);
                return EarliestMarkerSelection::Invalid;
            }
        };
        if !valid_full_oid(&marker.request_head_oid) || !valid_full_oid(&marker.request_base_oid) {
            diagnostics.push(format!(
                "受控 request marker `{}` 的 request head/base 不是完整 OID",
                comment.id
            ));
            return EarliestMarkerSelection::Invalid;
        }
        if timestamp_second(&comment.created_at) >= clean_second {
            continue;
        }
        candidates.push((comment, marker));
    }
    candidates.sort_by(|left, right| {
        left.0
            .created_at
            .cmp(&right.0.created_at)
            .then_with(|| left.0.id.cmp(&right.0.id))
    });
    let unconsumed: Vec<(&IssueComment, CodexReviewRequestRecord)> = candidates
        .into_iter()
        .filter(|(comment, _)| !consumed_markers.contains(comment.id.as_str()))
        .collect();
    let Some((first_comment, first_marker)) = unconsumed.first() else {
        return EarliestMarkerSelection::Exhausted;
    };
    let mixed_head_base = unconsumed.iter().any(|(_, marker)| {
        marker.request_head_oid != first_marker.request_head_oid
            || marker.request_base_oid != first_marker.request_base_oid
    });
    if mixed_head_base {
        return EarliestMarkerSelection::Ambiguous;
    }
    EarliestMarkerSelection::Selected(first_comment)
}

/// 与 evaluate_snapshot 的 comment 闸门一致：只有未编辑、Codex 发表、匹配封闭
/// clean grammar 且无 Reviewed commit marker 的 comment 才进入 record 绑定判定。
fn sha_less_bindable_clean(comment: &IssueComment) -> bool {
    comment
        .author
        .as_ref()
        .is_some_and(|actor| normalize_actor(&actor.login) == CODEX_ACTOR)
        && !comment.body.contains("To use Codex here")
        && codex_clean_comment_shape(&comment.body)
        && comment.updated_at == comment.created_at
        && parse_reviewed_commit(&comment.body).is_none()
}

/// 按 record 创建时间升序逐条判定无 SHA clean 的绑定，返回 clean comment id →
/// 判定结果（None 表示对应 record 不合法）。stale record 同样消费其 marker；
/// 未被任何 record 成功绑定的 clean 与未被消费的 record 由调用方分别诊断。
fn adjudicate_codex_clean_bindings<'a>(
    pr: &'a PullRequestSnapshot,
    records: &'a [BoundCleanRecord],
    diagnostics: &mut Vec<String>,
) -> BTreeMap<&'a str, Option<&'a BoundCleanRecord>> {
    let mut bindings = BTreeMap::new();
    let mut consumed_markers = BTreeSet::<&str>::new();
    let mut ordered: Vec<&BoundCleanRecord> = records.iter().collect();
    ordered.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.record.id.cmp(&right.record.id))
    });
    for bound in ordered {
        let record = &bound.record;
        let Some(clean) = pr
            .comments
            .nodes
            .iter()
            .find(|comment| comment.id == record.clean_comment_id)
        else {
            continue;
        };
        if !sha_less_bindable_clean(clean) {
            continue;
        }
        if bindings.contains_key(clean.id.as_str()) {
            // 一个 clean 只被一条 record 引用；重复引用已在 collect 阶段诊断
            bindings.insert(clean.id.as_str(), None);
            continue;
        }
        let verdict = adjudicate_binding_record(bound, clean, pr, &consumed_markers, diagnostics);
        if verdict.is_some() {
            consumed_markers.insert(record.request_marker_id.as_str());
        }
        bindings.insert(clean.id.as_str(), verdict);
    }
    bindings
}

fn adjudicate_binding_record<'a>(
    bound: &'a BoundCleanRecord,
    clean: &IssueComment,
    pr: &PullRequestSnapshot,
    consumed_markers: &BTreeSet<&str>,
    diagnostics: &mut Vec<String>,
) -> Option<&'a BoundCleanRecord> {
    let record = &bound.record;
    if record.clean_comment_created_at != clean.created_at {
        diagnostics.push(format!(
            "Codex clean binding record `{}` 的 cleanCommentCreatedAt 与 clean comment `{}` 不一致",
            record.id, clean.id
        ));
        return None;
    }
    if record.clean_comment_url != clean.url {
        diagnostics.push(format!(
            "Codex clean binding record `{}` 的 cleanCommentUrl 与 clean comment `{}` 不一致",
            record.id, clean.id
        ));
        return None;
    }
    if !resolve_request_marker(record, pr, clean, diagnostics) {
        return None;
    }
    match select_earliest_unconsumed_marker(pr, clean, consumed_markers, diagnostics) {
        EarliestMarkerSelection::Invalid => return None,
        EarliestMarkerSelection::Exhausted => {
            diagnostics.push(format!(
                "Codex clean binding record `{}` 引用的受控 request marker `{}` 已被更早的合法 binding record 消费",
                record.id, record.request_marker_id
            ));
            return None;
        }
        EarliestMarkerSelection::Ambiguous => {
            // 歧义不消费任何 marker；该 PR 的 SHA-less 路径 fail-closed，
            // 标准带 SHA 路径不受影响
            diagnostics.push(format!(
                "clean comment `{}` 之前存在多个 head/base 不同的未消费受控 request marker，无法证明 clean 归属，按歧义 fail-closed",
                clean.id
            ));
            return None;
        }
        EarliestMarkerSelection::Selected(expected) if expected.id != record.request_marker_id => {
            diagnostics.push(format!(
                "Codex clean binding record `{}` 引用的受控 request marker `{}` 不是 clean comment `{}` 之前最早未被消费的 marker（应为 `{}`）",
                record.id, record.request_marker_id, clean.id, expected.id
            ));
            return None;
        }
        EarliestMarkerSelection::Selected(_) => {}
    }
    if record.bound_head_oid == pr.head_ref_oid && record.bound_base_oid != pr.base_ref_oid {
        diagnostics.push(format!(
            "Codex clean binding record `{}` 绑定 current head 但 base 与 current base 不一致",
            record.id
        ));
        return None;
    }
    Some(bound)
}

fn dependabot_lockfile_completion(pr: &PullRequestSnapshot) -> Option<&CommitMetadata> {
    let author = normalize_actor(pr.author.as_ref()?.login.as_str());
    if author != "dependabot" && author != "app/dependabot" {
        return None;
    }
    if pr.files.page_info.has_next_page {
        return None;
    }
    let [file] = pr.files.nodes.as_slice() else {
        return None;
    };
    if file.path != "Cargo.lock" || file.change_type != "MODIFIED" {
        return None;
    }
    if pr.commits.page_info.has_next_page || pr.commits.nodes.len() != 1 {
        return None;
    }
    let commit = &pr.commits.nodes[0].commit;
    let commit_author = commit.author.as_ref()?;
    (commit.oid == pr.head_ref_oid
        && valid_full_oid(&commit.oid)
        && valid_timestamp(&commit.committed_date)
        && valid_github_url(&commit.url)
        && commit_author.name == DEPENDABOT_AUTHOR_NAME
        && commit_author.email == DEPENDABOT_AUTHOR_EMAIL)
        .then_some(commit)
}

fn dependabot_lockfile_false_positive_disposition<'a>(
    thread: &'a ReviewThread,
    completion: &CommitMetadata,
    current_head: &str,
    pr_author: &str,
) -> Option<(&'a str, &'a str)> {
    if !thread.is_resolved && !thread.is_outdated {
        return None;
    }
    let first = thread.comments.nodes.first()?;
    let review = first.pull_request_review.as_ref()?;
    let linked_head = review.commit.as_ref().map(|commit| commit.oid.as_str());
    let claimed_head = first
        .body
        .split('`')
        .find(|part| valid_oid_fragment(part.trim()))
        .map(str::trim);
    if normalize_actor(first.author.as_ref().map_or("", |actor| &actor.login)) != CODEX_ACTOR
        || normalize_actor(review.author.as_ref().map_or("", |actor| &actor.login)) != CODEX_ACTOR
        || !review.state.eq_ignore_ascii_case("COMMENTED")
        || linked_head != Some(current_head)
        || !valid_timestamp(&first.updated_at)
        || !valid_github_url(&first.url)
        || claimed_head.is_none_or(|claimed| oid_matches_current(claimed, current_head))
        || claimed_head.is_none_or(|claimed| {
            !known_dependabot_author_false_positive_body(&first.body, claimed)
        })
    {
        return None;
    }
    let mut latest_disposition = None;
    for reply in thread.comments.nodes.iter().skip(1) {
        let Some(provider) = trusted_provider(
            reply.author.as_ref().map_or("", |actor| &actor.login),
            pr_author,
        ) else {
            continue;
        };
        let accepted_disposition = provider == "human"
            && valid_timestamp(&reply.updated_at)
            && valid_github_url(&reply.url)
            && timestamp_second(&reply.updated_at) > timestamp_second(&first.updated_at)
            && reply.body.starts_with("Disposition:")
            && reply.body.contains(&completion.oid)
            && reply.body.contains(&format!(
                "{DEPENDABOT_AUTHOR_NAME} <{DEPENDABOT_AUTHOR_EMAIL}>"
            ));
        if !accepted_disposition {
            return None;
        }
        if latest_disposition.is_none_or(|current: (&str, &str)| {
            timestamp_second(&reply.updated_at) > timestamp_second(current.0)
        }) {
            latest_disposition = Some((reply.updated_at.as_str(), reply.url.as_str()));
        }
    }
    latest_disposition
}

fn validate_review_reference_in_connection(
    thread_id: &str,
    reference: &ReviewReference,
    reviews: &[Review],
) -> Result<(), String> {
    let review = reviews
        .iter()
        .find(|review| review.id == reference.id)
        .ok_or_else(|| {
            format!(
                "Dependabot 已知误报 thread `{thread_id}` 引用了 reviews connection 中不存在的 review：{}",
                reference.id
            )
        })?;
    let reference_actor = reference
        .author
        .as_ref()
        .map(|actor| normalize_actor(&actor.login));
    let review_actor = review
        .author
        .as_ref()
        .map(|actor| normalize_actor(&actor.login));
    let reference_commit = reference.commit.as_ref().map(|commit| commit.oid.as_str());
    let review_commit = review.commit.as_ref().map(|commit| commit.oid.as_str());
    if reference_actor != review_actor
        || !reference.state.eq_ignore_ascii_case(&review.state)
        || reference_commit != review_commit
    {
        return Err(format!(
            "Dependabot 已知误报 thread `{thread_id}` 的 review `{}` 与 reviews connection 的 actor/state/commit 不一致",
            reference.id
        ));
    }
    Ok(())
}

fn known_dependabot_author_false_positive_body(body: &str, claimed_head: &str) -> bool {
    const HEADING_DEPENDABOT: &str = "**<sub><sub>![P1 Badge](https://img.shields.io/badge/P1-orange?style=flat)</sub></sub>  Restore Dependabot author or add governance fields**";
    const HEADING_BOT: &str = "**<sub><sub>![P1 Badge](https://img.shields.io/badge/P1-orange?style=flat)</sub></sub>  Restore bot authorship or add governance fields**";
    const FOOTER: &str = "Useful? React with 👍 / 👎.";

    let candidates = [
        format!(
            "{HEADING_DEPENDABOT}\n\nFor this lockfile bump, the reviewed commit `{claimed_head}` is authored as `Codex <codex@openai.com>` while keeping a Dependabot-style message body that lacks the required LaneFlow `Gate`/`Slice`/`Impact`/`Scope`/`Validation`/`Docs`/`Refs` fields. The range checker only skips that validation when `xtask/src/main.rs:761-765` sees the exact Dependabot author/email and `xtask/src/main.rs:818-828` accepts the `build(deps)` title, so this commit will be rejected by the commit-message check unless it is re-authored as Dependabot or rewritten with the full governance block.\n\n{FOOTER}"
        ),
        format!(
            "{HEADING_BOT}\n\nWhen CI checks this PR's commit range, this commit is not eligible for the Dependabot exception: fresh evidence versus the earlier rebuttal is that the reviewed object `{claimed_head}` is actually authored by `Codex <codex@openai.com>`, while `xtask/src/main.rs:761-765` and `818-820` require the exact Dependabot name/email. The `Check commit messages` step in `.github/workflows/ci.yml:125-127` therefore validates the body and reports all required governance fields plus `Refs`/`Closes` as missing, blocking the required CI check; re-author this object as the bot or rewrite its message with the complete governance block.\n\n{FOOTER}"
        ),
        format!(
            "{HEADING_BOT}\n\nWhen the `Check commit messages` step in `.github/workflows/ci.yml:125-127` validates this PR range, this commit cannot use the Dependabot exception. Fresh evidence versus the earlier rebuttals is that the requested object `{claimed_head}` is authored by `Codex <codex@openai.com>`, whereas `xtask/src/main.rs:761-765` and `818-828` require the exact Dependabot name and email; its Dependabot-style body consequently lacks the required governance fields and `Refs`/`Closes`, blocking the governance check. Re-author this object as Dependabot or replace the body with the complete LaneFlow governance block.\n\n{FOOTER}"
        ),
    ];
    candidates.iter().any(|candidate| candidate == body)
}

fn collect_pagination_errors(pr: &PullRequestSnapshot, diagnostics: &mut Vec<String>) {
    if pr.review_requests.page_info.has_next_page {
        diagnostics.push("reviewRequests 超过 100 条，snapshot 被截断".to_string());
    }
    if pr.reviews.page_info.has_next_page {
        diagnostics.push("reviews 超过 100 条，snapshot 被截断".to_string());
    }
    if pr.comments.page_info.has_next_page {
        diagnostics.push("issue comments 超过 100 条，snapshot 被截断".to_string());
    }
    if pr.review_threads.page_info.has_next_page {
        diagnostics.push("reviewThreads 超过 100 条，snapshot 被截断".to_string());
    }
    for thread in &pr.review_threads.nodes {
        if thread.comments.page_info.has_next_page {
            diagnostics.push(format!(
                "review thread `{}` 的 comments 超过 100 条，snapshot 被截断",
                thread.id
            ));
        }
    }
}

fn validate_waiver(waiver: &WaiverInput, pr: &PullRequestSnapshot, diagnostics: &mut Vec<String>) {
    const ALLOWED_TYPES: &[&str] = &[
        "content_equivalent_rebase",
        "provider_platform_outage",
        "security_emergency_hotfix",
    ];
    let grandfathered_confirmed_gate_defect = waiver.grandfathered_confirmed_gate_defect
        && waiver.exception_type == "confirmed_gate_defect";
    if !ALLOWED_TYPES.contains(&waiver.exception_type.as_str())
        && !grandfathered_confirmed_gate_defect
    {
        diagnostics.push(format!(
            "waiver exceptionType 不在 allowlist：{}",
            waiver.exception_type
        ));
    }
    for (field, value) in [
        ("id", waiver.id.as_str()),
        ("reason", waiver.reason.as_str()),
        ("risk", waiver.risk.as_str()),
        ("acceptanceBoundary", waiver.acceptance_boundary.as_str()),
        ("expiresAt", waiver.expires_at.as_str()),
        ("followUpIssue", waiver.follow_up_issue.as_str()),
        ("cleanupOwner", waiver.cleanup_owner.as_str()),
        ("authorizedBy", waiver.authorized_by.as_str()),
    ] {
        if value.trim().is_empty() {
            diagnostics.push(format!("waiver `{field}` 不能为空"));
        }
    }
    if waiver.current_head_oid != pr.head_ref_oid {
        diagnostics.push("waiver currentHeadOid 与 PR current head 不一致".to_string());
    }
    if waiver.historical_base_replay {
        if !valid_full_oid(&waiver.current_base_oid) {
            diagnostics.push("historical waiver currentBaseOid 必须是完整 Git OID".to_string());
        }
    } else if waiver.current_base_oid != pr.base_ref_oid {
        diagnostics.push("waiver currentBaseOid 与 PR current base 不一致".to_string());
    }
    if waiver.evidence_urls.is_empty()
        || waiver
            .evidence_urls
            .iter()
            .any(|url| !valid_github_url(url))
    {
        diagnostics.push("waiver evidenceUrls 必须包含至少一个 GitHub HTTPS URL".to_string());
    }
    if !valid_timestamp(&waiver.expires_at) {
        diagnostics.push("waiver expiresAt 必须是 UTC RFC3339".to_string());
    }
}

fn copilot_outcome(body: &str, linked_findings: usize) -> Result<Option<EvidenceOutcome>, String> {
    if linked_findings > 0 {
        return Ok(Some(EvidenceOutcome::Findings));
    }
    let lower = body.to_ascii_lowercase();
    if lower.contains("generated no new comments") || lower.contains("generated no comments") {
        return Ok(Some(EvidenceOutcome::Clean));
    }
    let Some(generated) = lower.find("generated ") else {
        return Ok(None);
    };
    let tail = &lower[generated + "generated ".len()..];
    let digits = tail
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    if digits.is_empty() {
        return Err("无法解析 generated comment count".to_string());
    }
    let count = digits
        .parse::<usize>()
        .map_err(|error| format!("无法解析 generated comment count：{error}"))?;
    Ok(Some(if count == 0 {
        EvidenceOutcome::Clean
    } else {
        EvidenceOutcome::Findings
    }))
}

fn parse_reviewed_commit(body: &str) -> Option<&str> {
    let marker = "Reviewed commit:";
    let tail = body.get(body.find(marker)? + marker.len()..)?;
    let after_open = tail.get(tail.find('`')? + 1..)?;
    let candidate = after_open.get(..after_open.find('`')?)?.trim();
    valid_oid_fragment(candidate).then_some(candidate)
}

fn trusted_provider(actor: &str, author: &str) -> Option<&'static str> {
    let normalized = normalize_actor(actor);
    if normalized == normalize_actor(author) {
        return None;
    }
    match normalized.as_str() {
        COPILOT_ACTOR => Some("copilot"),
        CODEX_ACTOR => Some("codex"),
        actor if TRUSTED_HUMAN_ACTORS.contains(&actor) => Some("human"),
        _ => None,
    }
}

fn normalize_actor(actor: &str) -> String {
    actor.trim().trim_end_matches("[bot]").to_ascii_lowercase()
}

fn oid_matches_current(reviewed: &str, current: &str) -> bool {
    valid_oid_fragment(reviewed)
        && valid_full_oid(current)
        && current
            .to_ascii_lowercase()
            .starts_with(&reviewed.to_ascii_lowercase())
}

fn valid_full_oid(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn valid_oid_fragment(value: &str) -> bool {
    (7..=40).contains(&value.len()) && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn valid_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes.last() != Some(&b'Z')
    {
        return false;
    }
    let fixed_digits = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18];
    if fixed_digits
        .iter()
        .any(|index| !bytes[*index].is_ascii_digit())
    {
        return false;
    }
    if bytes.len() == 20 {
        return true;
    }
    bytes[19] == b'.' && bytes[20..bytes.len() - 1].iter().all(u8::is_ascii_digit)
}

fn timestamp_second(value: &str) -> &str {
    &value[..19]
}

fn valid_github_url(value: &str) -> bool {
    value.starts_with("https://github.com/")
}

fn valid_repository_name(value: &str) -> bool {
    let Some((owner, name)) = value.split_once('/') else {
        return false;
    };
    !owner.is_empty() && !name.is_empty() && !name.contains('/')
}

fn parse_args(args: &[String]) -> Result<ExternalReviewArgs, String> {
    let mut repository = None;
    let mut pr = None;
    let mut input = None;
    let mut output_format = OutputFormat::Json;
    let mut expected_state = None;
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("`{flag}` 缺少值"))?;
        match flag.as_str() {
            "--repo" => {
                if repository.replace(value.clone()).is_some() {
                    return Err("`--repo` 只能指定一次".to_string());
                }
            }
            "--pr" => {
                if pr.replace(parse_pr_number(value)?).is_some() {
                    return Err("`--pr` 只能指定一次".to_string());
                }
            }
            "--input" => {
                if input.replace(PathBuf::from(value)).is_some() {
                    return Err("`--input` 只能指定一次".to_string());
                }
            }
            "--format" => {
                output_format = match value.as_str() {
                    "json" => OutputFormat::Json,
                    "summary" => OutputFormat::Summary,
                    _ => return Err("`--format` 应为 `json` 或 `summary`".to_string()),
                };
            }
            "--expect" => {
                if expected_state
                    .replace(ExternalReviewState::parse(value)?)
                    .is_some()
                {
                    return Err("`--expect` 只能指定一次".to_string());
                }
            }
            _ => return Err(format!("未知 check-external-review 参数：{flag}")),
        }
        index += 2;
    }

    let source = match (input, repository, pr) {
        (Some(path), None, None) => InputSource::Snapshot(path),
        (None, Some(repository), Some(pr)) if valid_repository_name(&repository) => {
            InputSource::Live { repository, pr }
        }
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
            return Err("`--input` 不能与 `--repo` / `--pr` 同时使用".to_string());
        }
        (None, Some(repository), Some(_)) => {
            return Err(format!("`--repo` 格式不正确：{repository}"));
        }
        _ => {
            return Err(
                "用法：check-external-review --repo <owner/repo> --pr <number> [--format json|summary] [--expect <state>]；或 check-external-review --input <snapshot.json> [...]"
                    .to_string(),
            );
        }
    };
    Ok(ExternalReviewArgs {
        source,
        output_format,
        expected_state,
    })
}

fn parse_pr_number(value: &str) -> Result<u64, String> {
    value
        .strip_prefix('#')
        .unwrap_or(value)
        .parse::<u64>()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| format!("`--pr` 必须是正整数：{value}"))
}

fn print_summary(result: &ExternalReviewResult) {
    println!(
        "External Review Gate: {:?}\nPR: {}/pull/{}\nCurrent head/base: {}/{}\nProvider/actor: {}/{}\nReviewed head/completion: {}/{}\nFindings/unresolved/re-review: {}/{}/{}\nEvidence count: {}\nDiagnostics: {}",
        result.state,
        result.repository,
        result.pull_request,
        result.current_head_oid,
        result.current_base_oid,
        result.provider.as_deref().unwrap_or("N/A"),
        result.actor.as_deref().unwrap_or("N/A"),
        result.reviewed_head_oid.as_deref().unwrap_or("N/A"),
        result.completion_time.as_deref().unwrap_or("N/A"),
        result.finding_count,
        result.unresolved_actionable_threads,
        result.requires_rereview,
        result.evidence.len(),
        if result.diagnostics.is_empty() {
            "N/A".to_string()
        } else {
            result.diagnostics.join("；")
        }
    );
}

fn shadow_skip_reason(identity: &PullRequestIdentity) -> Option<&'static str> {
    if identity.base_ref_name != "main" {
        Some("PR 不以 main 为 base，不属于本 shadow Gate 范围")
    } else if identity.is_draft {
        Some("draft PR 不属于 R1 eligible sample")
    } else if identity.state != "OPEN" {
        Some("PR 已不是 OPEN 状态")
    } else if identity.is_cross_repository {
        Some(
            "fork / cross-repository PR head 无法由 base repository GITHUB_TOKEN 发布关联 Check；不计入 R1 sample，R2 前必须迁移到 same-repository PR",
        )
    } else {
        None
    }
}

fn parse_publish_check_args(args: &[String]) -> Result<PublishCheckArgs, String> {
    let mut repository = None;
    let mut pr = None;
    let mut details_url = None;
    let mut run_id = None;
    let mut run_attempt = None;
    let mut trusted_ref_oid = None;
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("`{flag}` 缺少值"))?;
        match flag.as_str() {
            "--repo" => set_once(&mut repository, value.clone(), flag)?,
            "--pr" => set_once(&mut pr, parse_positive_u64(value, flag)?, flag)?,
            "--details-url" => set_once(&mut details_url, value.clone(), flag)?,
            "--run-id" => set_once(&mut run_id, parse_positive_u64(value, flag)?, flag)?,
            "--run-attempt" => set_once(&mut run_attempt, parse_positive_u64(value, flag)?, flag)?,
            "--trusted-ref-oid" => set_once(&mut trusted_ref_oid, value.clone(), flag)?,
            _ => return Err(format!("未知 publish-external-review-check 参数：{flag}")),
        }
        index += 2;
    }

    let repository = repository.ok_or_else(|| {
        "用法：publish-external-review-check --repo <owner/repo> --pr <number> --details-url <workflow-run-url> --run-id <id> --run-attempt <number> --trusted-ref-oid <oid>"
            .to_string()
    })?;
    let (owner, name) = repository
        .split_once('/')
        .ok_or_else(|| format!("repository 格式不正确：{repository}"))?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return Err(format!("repository 格式不正确：{repository}"));
    }
    let details_url = details_url.ok_or("缺少 `--details-url`")?;
    let expected_details_prefix = format!("https://github.com/{repository}/actions/runs/");
    if !details_url.starts_with(&expected_details_prefix) {
        return Err(format!(
            "`--details-url` 必须指向当前 repository 的 GitHub Actions run：{expected_details_prefix}..."
        ));
    }
    let trusted_ref_oid = trusted_ref_oid.ok_or("缺少 `--trusted-ref-oid`")?;
    if !is_full_git_oid(&trusted_ref_oid) {
        return Err("`--trusted-ref-oid` 必须是 40 位小写十六进制 Git OID".to_string());
    }

    Ok(PublishCheckArgs {
        repository,
        pr: pr.ok_or("缺少 `--pr`")?,
        details_url,
        run_id: run_id.ok_or("缺少 `--run-id`")?,
        run_attempt: run_attempt.ok_or("缺少 `--run-attempt`")?,
        trusted_ref_oid,
    })
}

fn parse_request_codex_review_args(args: &[String]) -> Result<RequestCodexReviewArgs, String> {
    let mut repository = None;
    let mut pr = None;
    let mut dry_run = false;
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        if flag == "--dry-run" {
            if dry_run {
                return Err("`--dry-run` 只能指定一次".to_string());
            }
            dry_run = true;
            index += 1;
            continue;
        }
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("`{flag}` 缺少值"))?;
        match flag.as_str() {
            "--repo" => set_once(&mut repository, value.clone(), flag)?,
            "--pr" => set_once(&mut pr, parse_pr_number(value)?, flag)?,
            _ => return Err(format!("未知 request-codex-review 参数：{flag}")),
        }
        index += 2;
    }
    let repository = repository.ok_or_else(|| {
        "用法：request-codex-review --repo <owner/repo> --pr <number> [--dry-run]".to_string()
    })?;
    if !valid_repository_name(&repository) {
        return Err(format!("`--repo` 格式不正确：{repository}"));
    }
    Ok(RequestCodexReviewArgs {
        repository,
        pr: pr.ok_or("缺少 `--pr`")?,
        dry_run,
    })
}

fn parse_publish_codex_clean_binding_args(
    args: &[String],
) -> Result<PublishCodexCleanBindingArgs, String> {
    let mut repository = None;
    let mut pr = None;
    let mut clean_comment_id = None;
    let mut run_url = None;
    let mut dry_run = false;
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        if flag == "--dry-run" {
            if dry_run {
                return Err("`--dry-run` 只能指定一次".to_string());
            }
            dry_run = true;
            index += 1;
            continue;
        }
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("`{flag}` 缺少值"))?;
        match flag.as_str() {
            "--repo" => set_once(&mut repository, value.clone(), flag)?,
            "--pr" => set_once(&mut pr, parse_pr_number(value)?, flag)?,
            "--clean-comment-id" => set_once(&mut clean_comment_id, value.clone(), flag)?,
            "--run-url" => set_once(&mut run_url, value.clone(), flag)?,
            _ => return Err(format!("未知 publish-codex-clean-binding 参数：{flag}")),
        }
        index += 2;
    }
    let repository = repository.ok_or_else(|| {
        "用法：publish-codex-clean-binding --repo <owner/repo> --pr <number> --clean-comment-id <graphql-node-id> --run-url <workflow-run-url> [--dry-run]"
            .to_string()
    })?;
    if !valid_repository_name(&repository) {
        return Err(format!("`--repo` 格式不正确：{repository}"));
    }
    let clean_comment_id = clean_comment_id.ok_or("缺少 `--clean-comment-id`")?;
    if clean_comment_id.is_empty() || clean_comment_id.chars().any(char::is_whitespace) {
        return Err("`--clean-comment-id` 必须是非空且不含空白的 GraphQL node id".to_string());
    }
    let run_url = run_url.ok_or("缺少 `--run-url`")?;
    let expected_run_prefix = format!("https://github.com/{repository}/actions/runs/");
    if !run_url.starts_with(&expected_run_prefix) {
        return Err(format!(
            "`--run-url` 必须指向当前 repository 的 GitHub Actions run：{expected_run_prefix}..."
        ));
    }
    Ok(PublishCodexCleanBindingArgs {
        repository,
        pr: pr.ok_or("缺少 `--pr`")?,
        clean_comment_id,
        run_url,
        dry_run,
    })
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), String> {
    if slot.replace(value).is_some() {
        return Err(format!("参数 `{flag}` 不能重复"));
    }
    Ok(())
}

fn parse_positive_u64(value: &str, flag: &str) -> Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|error| format!("`{flag}` 必须是正整数：{error}"))?;
    if parsed == 0 {
        return Err(format!("`{flag}` 必须是正整数"));
    }
    Ok(parsed)
}

fn is_full_git_oid(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn ensure_identity_unchanged(
    initial: &PullRequestIdentity,
    final_identity: &PullRequestIdentity,
) -> Result<(), String> {
    if initial.number != final_identity.number
        || initial.head_ref_oid != final_identity.head_ref_oid
        || initial.base_ref_oid != final_identity.base_ref_oid
        || initial.base_ref_name != final_identity.base_ref_name
        || initial.head_ref_name != final_identity.head_ref_name
        || initial.is_cross_repository != final_identity.is_cross_repository
        || initial.is_draft != final_identity.is_draft
        || initial.state != final_identity.state
    {
        return Err(format!(
            "PR identity 在 shadow Gate 运行期间发生变化：initial=({}, {}, {}, {}, {}, {}, {}, {}) final=({}, {}, {}, {}, {}, {}, {}, {})",
            initial.number,
            initial.head_ref_oid,
            initial.base_ref_oid,
            initial.base_ref_name,
            initial.head_ref_name,
            initial.is_cross_repository,
            initial.is_draft,
            initial.state,
            final_identity.number,
            final_identity.head_ref_oid,
            final_identity.base_ref_oid,
            final_identity.base_ref_name,
            final_identity.head_ref_name,
            final_identity.is_cross_repository,
            final_identity.is_draft,
            final_identity.state
        ));
    }
    Ok(())
}

fn ensure_result_matches_identity(
    result: &ExternalReviewResult,
    identity: &PullRequestIdentity,
) -> Result<(), String> {
    if result.pull_request != identity.number
        || result.current_head_oid != identity.head_ref_oid
        || result.current_base_oid != identity.base_ref_oid
    {
        return Err(format!(
            "evaluator 结果与发布前 PR identity 不一致：result=(#{}, {}, {}) identity=(#{}, {}, {})",
            result.pull_request,
            result.current_head_oid,
            result.current_base_oid,
            identity.number,
            identity.head_ref_oid,
            identity.base_ref_oid
        ));
    }
    Ok(())
}

fn build_check_run_payload(
    result: &ExternalReviewResult,
    details_url: &str,
    external_id: String,
) -> CheckRunPayload {
    let provider = optional_value(result.provider.as_deref());
    let actor = optional_value(result.actor.as_deref());
    let reviewed_head = optional_value(result.reviewed_head_oid.as_deref());
    let completion = optional_value(result.completion_time.as_deref());
    let waiver = optional_value(result.waiver_id.as_deref());
    let summary = format!(
        "state=`{}`; head=`{}`; provider=`{provider}`; actor=`{actor}`; findings={}; unresolved={}; re-review={}; diagnostics={}",
        result.state.as_str(),
        result.current_head_oid,
        result.finding_count,
        result.unresolved_actionable_threads,
        result.requires_rereview,
        result.diagnostics.len()
    );

    let evidence_limit = 20;
    let evidence_labels = result
        .evidence
        .iter()
        .take(evidence_limit)
        .enumerate()
        .map(|(index, _)| format!("[evidence-{}]", index + 1))
        .collect::<Vec<_>>();
    let mut text = format!(
        "- Repository / PR：`{}` / `#{}`\n- Current head / base：`{}` / `{}`\n- Author：`{}`\n- State：`{}`\n- Provider / actor：`{provider}` / `{actor}`\n- Reviewed head / completion：`{reviewed_head}` / `{completion}`\n- Findings / unresolved threads / requires re-review：`{}` / `{}` / `{}`\n- Pending review requests：`{}`\n- Waiver：`{waiver}`\n- Evidence：{}\n- Diagnostics：`{}`（详情见 workflow run）",
        single_line(&result.repository),
        result.pull_request,
        result.current_head_oid,
        result.current_base_oid,
        single_line(&result.author),
        result.state.as_str(),
        result.finding_count,
        result.unresolved_actionable_threads,
        result.requires_rereview,
        result.pending_review_requests,
        if evidence_labels.is_empty() {
            "N/A".to_string()
        } else {
            evidence_labels.join("；")
        },
        result.diagnostics.len()
    );
    if result.evidence.len() > evidence_limit {
        text.push_str(&format!(
            "\n- Evidence truncation：显示前 `{evidence_limit}` / 共 `{}` 条；完整 evaluator JSON 保留在 workflow log。",
            result.evidence.len()
        ));
    }
    if !evidence_labels.is_empty() {
        text.push_str("\n\n");
        for (index, evidence) in result.evidence.iter().take(evidence_limit).enumerate() {
            text.push_str(&format!(
                "[evidence-{}]: {}\n",
                index + 1,
                evidence.evidence_url
            ));
        }
    }

    CheckRunPayload {
        name: EXTERNAL_REVIEW_SHADOW_CHECK_NAME,
        head_sha: result.current_head_oid.clone(),
        status: "completed",
        conclusion: result.state.check_conclusion(),
        details_url: details_url.to_string(),
        external_id,
        output: CheckRunOutput {
            title: result.state.check_title(),
            summary,
            text,
        },
    }
}

fn optional_value(value: Option<&str>) -> String {
    value.map(single_line).unwrap_or_else(|| "N/A".to_string())
}

fn single_line(value: &str) -> String {
    value.replace(['\r', '\n', '`'], " ").trim().to_string()
}

fn evaluation_fingerprint(result: &ExternalReviewResult) -> Result<String, String> {
    let bytes = serde_json::to_vec(result)
        .map_err(|error| format!("无法序列化 evaluator fingerprint 输入：{error}"))?;
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Ok(format!("{hash:016x}"))
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
        || response.app.slug != R1_SHADOW_CHECK_APP_SLUG
    {
        return Err(format!(
            "Check Run 发布结果不符合绑定要求：name={} head={} status={} conclusion={} external_id={:?} app={}；期望 name={} head={} status={} conclusion={} external_id={} app={}",
            response.name,
            response.head_sha,
            response.status,
            conclusion,
            response.external_id,
            response.app.slug,
            payload.name,
            payload.head_sha,
            payload.status,
            payload.conclusion,
            payload.external_id,
            R1_SHADOW_CHECK_APP_SLUG
        ));
    }
    Ok(())
}

fn load_live_snapshot(repository: &str, pr: u64) -> Result<ExternalReviewSnapshot, String> {
    let (owner, name) = repository
        .split_once('/')
        .ok_or_else(|| format!("repository 格式不正确：{repository}"))?;
    let response = gh_graphql::<ExternalReviewData>(EXTERNAL_REVIEW_QUERY, owner, name, pr)?;
    let repository_data = response
        .repository
        .ok_or_else(|| format!("GitHub repository 不存在或不可读：{repository}"))?;
    let mut pull_request = repository_data
        .pull_request
        .ok_or_else(|| format!("GitHub PR 不存在或不可读：{repository}#{pr}"))?;
    if pull_request.comments.page_info.has_next_page {
        // 绑定判定需要完整的 clean/marker/record 视图：截断时补齐全量分页；
        // 分页无法完成时 fail-closed，不得静默当作不存在。
        pull_request.comments = Connection {
            nodes: fetch_all_issue_comments(repository, pr)?,
            page_info: PageInfo::default(),
        };
    }
    Ok(ExternalReviewSnapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        repository: repository.to_string(),
        pull_request,
        provider_errors: Vec::new(),
        waiver: None,
    })
}

fn fetch_all_issue_comments(repository: &str, pr: u64) -> Result<Vec<IssueComment>, String> {
    let (owner, name) = repository
        .split_once('/')
        .ok_or_else(|| format!("repository 格式不正确：{repository}"))?;
    fetch_comment_pages(|cursor| load_issue_comments_page(owner, name, pr, cursor))
}

fn fetch_comment_pages(
    mut load_page: impl FnMut(Option<&str>) -> Result<CommentsConnection, String>,
) -> Result<Vec<IssueComment>, String> {
    let mut comments = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = load_page(cursor.as_deref())?;
        let fetched = page.nodes.len();
        comments.extend(page.nodes);
        let Some(next) = next_comment_page_cursor(&page.page_info, fetched)? else {
            break;
        };
        if cursor.as_deref() == Some(next.as_str()) {
            return Err(
                "issue comments 分页 cursor 未推进，无法完成分页，按 fail-closed 处理".to_string(),
            );
        }
        cursor = Some(next);
    }
    Ok(comments)
}

fn next_comment_page_cursor(
    page_info: &CommentsPageInfo,
    fetched: usize,
) -> Result<Option<String>, String> {
    if !page_info.has_next_page {
        return Ok(None);
    }
    if fetched == 0 {
        return Err(
            "issue comments 分页返回空页但 hasNextPage 为 true，无法完成分页，按 fail-closed 处理"
                .to_string(),
        );
    }
    match page_info
        .end_cursor
        .as_deref()
        .filter(|cursor| !cursor.is_empty())
    {
        Some(cursor) => Ok(Some(cursor.to_string())),
        None => {
            Err("issue comments 分页缺少 endCursor，无法完成分页，按 fail-closed 处理".to_string())
        }
    }
}

fn load_issue_comments_page(
    owner: &str,
    name: &str,
    pr: u64,
    cursor: Option<&str>,
) -> Result<CommentsConnection, String> {
    let data: CommentsPageData =
        gh_graphql_with_cursor(PULL_REQUEST_COMMENTS_QUERY, owner, name, pr, cursor)?;
    data.repository
        .and_then(|repository| repository.pull_request)
        .map(|pull_request| pull_request.comments)
        .ok_or_else(|| format!("GitHub PR 不存在或不可读：{owner}/{name}#{pr}"))
}

fn load_live_waiver_snapshot(
    repository: &str,
    pr: u64,
    waiver: WaiverInput,
) -> Result<ExternalReviewSnapshot, String> {
    let identity = load_live_identity(repository, pr)?;
    if identity.number != pr {
        return Err(format!(
            "GitHub PR identity number 不一致：请求 #{pr}，返回 #{}",
            identity.number
        ));
    }
    Ok(ExternalReviewSnapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        repository: repository.to_string(),
        pull_request: PullRequestSnapshot {
            number: identity.number,
            author: identity.author,
            head_ref_oid: identity.head_ref_oid,
            base_ref_oid: identity.base_ref_oid,
            is_draft: identity.is_draft,
            files: Connection::default(),
            commits: Connection::default(),
            review_requests: Connection::default(),
            reviews: Connection::default(),
            comments: Connection::default(),
            review_threads: Connection::default(),
        },
        provider_errors: Vec::new(),
        waiver: Some(waiver),
    })
}

fn load_live_identity(repository: &str, pr: u64) -> Result<PullRequestIdentity, String> {
    let (owner, name) = repository
        .split_once('/')
        .ok_or_else(|| format!("repository 格式不正确：{repository}"))?;
    let response = gh_graphql::<IdentityData>(PULL_REQUEST_IDENTITY_QUERY, owner, name, pr)?;
    let identity = response
        .repository
        .and_then(|repository| repository.pull_request)
        .ok_or_else(|| format!("发布前无法复核 GitHub PR identity：{repository}#{pr}"))?;
    Ok(identity)
}

fn gh_graphql<T: for<'de> Deserialize<'de>>(
    query: &str,
    owner: &str,
    name: &str,
    pr: u64,
) -> Result<T, String> {
    gh_graphql_with_cursor(query, owner, name, pr, None)
}

fn gh_graphql_with_cursor<T: for<'de> Deserialize<'de>>(
    query: &str,
    owner: &str,
    name: &str,
    pr: u64,
    cursor: Option<&str>,
) -> Result<T, String> {
    let mut command = Command::new("gh");
    command
        .arg("api")
        .arg("graphql")
        .arg("-F")
        .arg(format!("owner={owner}"))
        .arg("-F")
        .arg(format!("name={name}"))
        .arg("-F")
        .arg(format!("number={pr}"))
        .arg("-f")
        .arg(format!("query={query}"));
    if let Some(cursor) = cursor {
        command.arg("-f").arg(format!("cursor={cursor}"));
    }
    let output = command
        .output()
        .map_err(|error| format!("无法运行 gh GraphQL：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh GraphQL 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let envelope =
        serde_json::from_slice::<GraphQlEnvelope<T>>(&output.stdout).map_err(|error| {
            format!(
                "gh GraphQL 输出不是预期 JSON：{error}；原始输出：{}",
                String::from_utf8_lossy(&output.stdout).trim()
            )
        })?;
    if !envelope.errors.is_empty() {
        return Err(format!(
            "GitHub GraphQL errors：{}",
            envelope
                .errors
                .iter()
                .map(|error| error.message.as_str())
                .collect::<Vec<_>>()
                .join("；")
        ));
    }
    envelope
        .data
        .ok_or_else(|| "GitHub GraphQL response 缺少 data".to_string())
}

#[derive(Debug, Deserialize)]
struct GraphQlEnvelope<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GraphQlError>,
}

#[derive(Debug, Deserialize)]
struct GraphQlError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct ExternalReviewData {
    repository: Option<ExternalReviewRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExternalReviewRepository {
    pull_request: Option<PullRequestSnapshot>,
}

#[derive(Debug, Deserialize)]
struct CommentsPageData {
    repository: Option<CommentsPageRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentsPageRepository {
    pull_request: Option<CommentsPagePullRequest>,
}

#[derive(Debug, Deserialize)]
struct CommentsPagePullRequest {
    comments: CommentsConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentsConnection {
    #[serde(default)]
    nodes: Vec<IssueComment>,
    #[serde(default)]
    page_info: CommentsPageInfo,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentsPageInfo {
    #[serde(default)]
    has_next_page: bool,
    #[serde(default)]
    end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IdentityData {
    repository: Option<IdentityRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdentityRepository {
    pull_request: Option<PullRequestIdentity>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestIdentity {
    number: u64,
    author: Option<Actor>,
    head_ref_oid: String,
    base_ref_oid: String,
    base_ref_name: String,
    head_ref_name: String,
    is_cross_repository: bool,
    is_draft: bool,
    state: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(contents: &str) -> ExternalReviewSnapshot {
        serde_json::from_str(contents).expect("fixture must match snapshot schema")
    }

    fn sample_identity(head: &str) -> PullRequestIdentity {
        PullRequestIdentity {
            number: 239,
            author: Some(Actor {
                login: "wangzishi".to_string(),
            }),
            head_ref_oid: head.to_string(),
            base_ref_oid: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            base_ref_name: "main".to_string(),
            head_ref_name: "feature-x".to_string(),
            is_cross_repository: false,
            is_draft: false,
            state: "OPEN".to_string(),
        }
    }

    fn sample_result(state: ExternalReviewState) -> ExternalReviewResult {
        ExternalReviewResult {
            schema_version: RESULT_SCHEMA_VERSION,
            repository: "illusion-tech/laneflow".to_string(),
            pull_request: 239,
            current_head_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            current_base_oid: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            author: "wangzishi".to_string(),
            state,
            provider: Some("codex".to_string()),
            actor: Some(CODEX_ACTOR.to_string()),
            reviewed_head_oid: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            completion_time: Some("2026-07-24T17:15:39Z".to_string()),
            finding_count: 2,
            unresolved_actionable_threads: 0,
            requires_rereview: false,
            pending_review_requests: 0,
            evidence: vec![ReviewEvidence {
                provider: "codex".to_string(),
                actor: CODEX_ACTOR.to_string(),
                source_kind: "issue_comment".to_string(),
                reviewed_head_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                reviewed_base_oid: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
                outcome: EvidenceOutcome::Clean,
                submitted_at: "2026-07-24T17:15:39Z".to_string(),
                evidence_url: "https://github.com/illusion-tech/laneflow/pull/239#issuecomment-1"
                    .to_string(),
            }],
            waiver_id: None,
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn maps_external_review_states_to_shadow_check_conclusions() {
        assert_eq!(ExternalReviewState::Pass.check_conclusion(), "success");
        assert_eq!(
            ExternalReviewState::Waived.check_conclusion(),
            "action_required"
        );
        for state in [
            ExternalReviewState::AwaitingReview,
            ExternalReviewState::ReviewPending,
            ExternalReviewState::FindingsOpen,
            ExternalReviewState::AwaitingRereview,
            ExternalReviewState::Stale,
            ExternalReviewState::ProviderError,
        ] {
            assert_eq!(state.check_conclusion(), "failure");
        }
    }

    #[test]
    fn builds_head_bound_check_payload_with_reference_style_evidence() {
        let result = sample_result(ExternalReviewState::Pass);
        let payload = build_check_run_payload(
            &result,
            "https://github.com/illusion-tech/laneflow/actions/runs/1",
            "external-review:test".to_string(),
        );

        assert_eq!(payload.name, EXTERNAL_REVIEW_SHADOW_CHECK_NAME);
        assert_eq!(payload.head_sha, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(payload.status, "completed");
        assert_eq!(payload.conclusion, "success");
        assert!(payload.output.text.contains("[evidence-1]"));
        assert!(
            payload
                .output
                .text
                .contains("\n\n[evidence-1]: https://github.com/")
        );
        assert!(
            payload
                .output
                .text
                .lines()
                .filter(|line| line.contains("https://github.com/"))
                .all(|line| line.starts_with("[evidence-"))
        );
    }

    #[test]
    fn rejects_identity_changes_before_check_publication() {
        let initial = sample_identity("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let unchanged = sample_identity("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let changed = sample_identity("cccccccccccccccccccccccccccccccccccccccc");

        assert!(ensure_identity_unchanged(&initial, &unchanged).is_ok());
        assert!(ensure_identity_unchanged(&initial, &changed).is_err());
        assert!(
            ensure_result_matches_identity(&sample_result(ExternalReviewState::Pass), &unchanged)
                .is_ok()
        );
        assert!(
            ensure_result_matches_identity(&sample_result(ExternalReviewState::Pass), &changed)
                .is_err()
        );
    }

    #[test]
    fn binds_provider_errors_to_a_stable_identity_and_filters_ineligible_prs() {
        let identity = sample_identity("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let mut result = ExternalReviewResult::provider_error(
            "illusion-tech/laneflow",
            239,
            "provider unavailable".to_string(),
        );
        result.bind_identity_if_missing("illusion-tech/laneflow", &identity);

        assert!(ensure_result_matches_identity(&result, &identity).is_ok());
        assert_eq!(shadow_skip_reason(&identity), None);

        let mut draft = sample_identity("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        draft.is_draft = true;
        assert_eq!(
            shadow_skip_reason(&draft),
            Some("draft PR 不属于 R1 eligible sample")
        );

        let mut other_base = sample_identity("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        other_base.base_ref_name = "release".to_string();
        assert_eq!(
            shadow_skip_reason(&other_base),
            Some("PR 不以 main 为 base，不属于本 shadow Gate 范围")
        );

        let mut fork = sample_identity("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        fork.is_cross_repository = true;
        assert_eq!(
            shadow_skip_reason(&fork),
            Some(
                "fork / cross-repository PR head 无法由 base repository GITHUB_TOKEN 发布关联 Check；不计入 R1 sample，R2 前必须迁移到 same-repository PR"
            )
        );
    }

    #[test]
    fn verifies_published_head_conclusion_and_source_app() {
        let result = sample_result(ExternalReviewState::Pass);
        let payload = build_check_run_payload(
            &result,
            "https://github.com/illusion-tech/laneflow/actions/runs/1",
            "external-review:test".to_string(),
        );
        let mut response = CheckRunResponse {
            id: 1,
            name: payload.name.to_string(),
            html_url: "https://github.com/illusion-tech/laneflow/runs/1".to_string(),
            head_sha: payload.head_sha.clone(),
            status: payload.status.to_string(),
            conclusion: Some(payload.conclusion.to_string()),
            external_id: Some(payload.external_id.clone()),
            app: CheckRunApp {
                slug: R1_SHADOW_CHECK_APP_SLUG.to_string(),
            },
        };

        assert!(verify_check_run_response(&response, &payload).is_ok());
        response.app.slug = "unexpected-app".to_string();
        assert!(verify_check_run_response(&response, &payload).is_err());
    }

    #[test]
    fn fingerprints_state_and_never_reuses_existing_shadow_checks() {
        let pass = sample_result(ExternalReviewState::Pass);
        let awaiting = sample_result(ExternalReviewState::AwaitingRereview);
        assert_eq!(
            evaluation_fingerprint(&pass).expect("pass fingerprint"),
            evaluation_fingerprint(&pass).expect("stable pass fingerprint")
        );
        assert_ne!(
            evaluation_fingerprint(&pass).expect("pass fingerprint"),
            evaluation_fingerprint(&awaiting).expect("awaiting fingerprint")
        );
        let source = include_str!("external_review.rs");
        let forbidden_find = ["find_existing_", "equivalent_check"].concat();
        let forbidden_select = ["select_existing_", "equivalent_check"].concat();
        assert!(!source.contains(&forbidden_find));
        assert!(!source.contains(&forbidden_select));
        assert!(source.contains("let response = create_check_run(&args.repository, &payload)?;"));
    }

    #[test]
    fn parses_bounded_shadow_check_publish_arguments() {
        let args = [
            "--repo",
            "illusion-tech/laneflow",
            "--pr",
            "239",
            "--details-url",
            "https://github.com/illusion-tech/laneflow/actions/runs/1",
            "--run-id",
            "1",
            "--run-attempt",
            "2",
            "--trusted-ref-oid",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ]
        .map(str::to_string);
        let parsed = parse_publish_check_args(&args).expect("valid publisher arguments");

        assert_eq!(parsed.repository, "illusion-tech/laneflow");
        assert_eq!(parsed.pr, 239);
        assert_eq!(parsed.run_id, 1);
        assert_eq!(parsed.run_attempt, 2);

        let mut wrong_url = args.clone();
        wrong_url[5] = "https://example.com/actions/runs/1".to_string();
        assert!(parse_publish_check_args(&wrong_url).is_err());

        let mut short_oid = args;
        short_oid[11] = "aaaa".to_string();
        assert!(parse_publish_check_args(&short_oid).is_err());
    }

    #[test]
    fn shadow_workflows_preserve_the_trusted_ref_boundary() {
        let gate = include_str!("../../.github/workflows/external-review-gate.yml");
        let signal = include_str!("../../.github/workflows/external-review-signal.yml");

        for trigger in [
            "pull_request_target:",
            "issue_comment:",
            "workflow_run:",
            "workflow_dispatch:",
        ] {
            assert!(gate.contains(trigger), "missing trusted trigger: {trigger}");
        }
        assert!(!gate.contains("schedule:"));
        assert!(gate.contains("External Review Signal"));
        assert!(gate.contains("issue_comment:\n    types:\n      - created\n  workflow_run:"));
        assert!(gate.contains("github.event.action == 'created'"));
        assert!(
            gate.contains("github.event.comment.body == 'external-review: thread-state-changed'")
        );
        assert!(gate.contains("repos/${REPOSITORY}/commits/${head_sha}/pulls?per_page=100"));
        assert!(gate.contains("workflow_run head_sha must be a full lowercase Git OID"));
        assert!(!gate.contains(".workflow_run.head_branch"));
        assert!(gate.contains(
            "permissions:\n  contents: read\n  pull-requests: read\n  issues: read\n  checks: write"
        ));
        assert!(gate.contains("ref: refs/heads/main"));
        assert!(gate.contains("persist-credentials: false"));
        assert!(gate.contains("actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"));
        assert!(gate.contains("group: external-review-gate-pr-${{ matrix.pr }}"));
        assert!(gate.contains("cancel-in-progress: true"));
        assert!(gate.contains("publish-external-review-check"));
        assert!(!gate.contains("github.event.pull_request.head.sha"));
        assert!(!gate.contains("refs/pull/"));
        assert!(!gate.contains("secrets."));
        assert!(!gate.lines().any(|line| {
            matches!(
                line.trim_start(),
                "pull_request_review:" | "pull_request_review_comment:"
            )
        }));

        assert!(signal.contains("pull_request_review:"));
        assert!(signal.contains("pull_request_review_comment:"));
        assert!(signal.contains("permissions: {}"));
        assert!(!signal.contains("actions/checkout"));
        assert!(!signal.contains("gh api"));
        assert!(!signal.contains("cargo "));
        assert!(!signal.contains("secrets."));
    }

    #[test]
    fn binding_workflow_preserves_the_trusted_ref_boundary() {
        let binding = include_str!("../../.github/workflows/codex-clean-binding.yml");

        assert!(binding.contains("issue_comment:\n    types:\n      - created"));
        assert!(!binding.lines().any(|line| {
            matches!(
                line.trim_start(),
                "pull_request:"
                    | "pull_request_target:"
                    | "pull_request_review:"
                    | "pull_request_review_comment:"
                    | "schedule:"
                    | "workflow_run:"
                    | "workflow_dispatch:"
                    | "push:"
            )
        }));
        assert!(binding.contains(
            "permissions:\n  contents: read\n  pull-requests: read\n  issues: write\n  checks: write"
        ));
        assert!(binding.contains("ref: refs/heads/main"));
        assert!(binding.contains("persist-credentials: false"));
        assert!(binding.contains("actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"));
        assert!(binding.contains("Verify trusted checkout"));
        // 串行化 + sweep 配对：concurrency 组保证同一 PR 任意时刻只有一个
        // publisher run（消除并发读-选-发竞态）；同组只允许一个 pending run，
        // 被挤掉的中间事件由 publisher 的 unbound sweep 补齐，二者缺一不可。
        assert!(binding.contains("group: codex-clean-binding-pr-${{ github.event.issue.number }}"));
        assert!(binding.contains("cancel-in-progress: false"));
        assert!(binding.contains("github.event.issue.pull_request != null"));
        assert!(binding.contains("github.event.comment.user.login == 'wangzishi'"));
        assert!(
            binding
                .contains("github.event.comment.body == 'external-review: request-codex-review'")
        );
        assert!(
            binding.contains("github.event.comment.user.login == 'chatgpt-codex-connector[bot]'")
        );
        assert!(binding.contains("request-codex-review"));
        assert!(binding.contains("publish-codex-clean-binding"));
        assert!(binding.contains("--pr \"$PR_NUMBER\""));
        assert!(binding.contains("CLEAN_COMMENT_NODE_ID: ${{ github.event.comment.node_id }}"));
        assert!(binding.contains("--clean-comment-id \"$CLEAN_COMMENT_NODE_ID\""));
        assert!(!binding.contains("github.event.comment.id"));
        assert!(binding.contains("--run-url \"$RUN_URL\""));
        // GITHUB_TOKEN 发布的 record 评论不会再触发 Gate workflow（GitHub 递归
        // 防护），绑定发布后必须由本 workflow 在同一 run 内自行刷新 shadow check。
        assert!(binding.contains("Refresh External Review Gate Shadow"));
        // 绑定命令对不可绑定 comment fail-closed 退出非零时刷新仍须执行。
        assert!(binding.contains(
            "if: always() && github.event.comment.user.login == 'chatgpt-codex-connector[bot]'"
        ));
        assert!(binding.contains("publish-external-review-check"));
        assert!(binding.contains("--trusted-ref-oid \"$trusted_ref_oid\""));
        assert!(binding.contains("RUN_ATTEMPT: ${{ github.run_attempt }}"));
        assert!(binding.contains("RUN_ID: ${{ github.run_id }}"));
        assert!(binding.contains("rustup toolchain install 1.96.0"));
        assert!(!binding.contains("secrets."));
        assert!(!binding.contains("refs/pull/"));
        assert!(!binding.contains("github.event.pull_request.head.sha"));
        assert!(!binding.contains("pull_request.head"));
    }

    #[test]
    fn shadow_check_contract_requires_a_dedicated_app_before_r2() {
        let workflow_contract = include_str!("../../docs/governance/github-workflow.md");

        assert_eq!(
            EXTERNAL_REVIEW_SHADOW_CHECK_NAME,
            "External Review Gate Shadow"
        );
        assert!(workflow_contract.contains("`External Review Gate Shadow` 永远不得加入 ruleset"));
        assert!(workflow_contract.contains(
            "R2 激活前必须注册并仅在 base repository 安装专用 External Review Gate GitHub App"
        ));
        assert!(workflow_contract.contains("ruleset 中同时绑定 Check 名和 expected source App"));
        assert!(workflow_contract.contains("external-review: thread-state-changed"));
        assert!(workflow_contract.contains("订阅 `pull_request_review_thread` webhook"));
        assert!(workflow_contract.contains("若平台只交付文档化的 `resolved` action"));
    }

    #[test]
    fn replays_provider_and_lifecycle_fixtures() {
        let cases = [
            (
                include_str!("../fixtures/external-review/copilot-clean.json"),
                ExternalReviewState::Pass,
            ),
            (
                include_str!("../fixtures/external-review/copilot-findings-open.json"),
                ExternalReviewState::FindingsOpen,
            ),
            (
                include_str!("../fixtures/external-review/codex-clean.json"),
                ExternalReviewState::Pass,
            ),
            (
                include_str!("../fixtures/external-review/codex-awaiting-rereview.json"),
                ExternalReviewState::AwaitingRereview,
            ),
            (
                include_str!("../fixtures/external-review/human-approved.json"),
                ExternalReviewState::Pass,
            ),
            (
                include_str!("../fixtures/external-review/stale-old-head.json"),
                ExternalReviewState::Stale,
            ),
            (
                include_str!("../fixtures/external-review/wrong-actor.json"),
                ExternalReviewState::AwaitingReview,
            ),
            (
                include_str!("../fixtures/external-review/review-pending.json"),
                ExternalReviewState::ReviewPending,
            ),
            (
                include_str!("../fixtures/external-review/codex-no-sha.json"),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-old-no-sha-then-current-clean.json"
                ),
                ExternalReviewState::Pass,
            ),
            (
                include_str!("../fixtures/external-review/codex-current-clean-then-no-sha.json"),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-old-no-sha-same-second-current-clean.json"
                ),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!("../fixtures/external-review/codex-no-sha-bound-clean.json"),
                ExternalReviewState::Pass,
            ),
            (
                include_str!("../fixtures/external-review/codex-no-sha-bound-old-head.json"),
                ExternalReviewState::Stale,
            ),
            (
                include_str!("../fixtures/external-review/codex-no-sha-binding-edited-clean.json"),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!("../fixtures/external-review/codex-no-sha-edited-record.json"),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!("../fixtures/external-review/codex-no-sha-wrong-actor-record.json"),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!("../fixtures/external-review/codex-no-sha-duplicate-records.json"),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!("../fixtures/external-review/codex-no-sha-record-missing-clean.json"),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!("../fixtures/external-review/codex-no-sha-record-missing-marker.json"),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!("../fixtures/external-review/codex-clean-with-marker-and-record.json"),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-no-sha-bound-clean-then-unbound.json"
                ),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-no-sha-record-marker-head-mismatch.json"
                ),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-no-sha-record-marker-same-second.json"
                ),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-no-sha-bound-clean-finding-before-record.json"
                ),
                ExternalReviewState::AwaitingRereview,
            ),
            (
                include_str!("../fixtures/external-review/codex-no-sha-cross-push-late-clean.json"),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-no-sha-cross-push-stale-then-current.json"
                ),
                ExternalReviewState::Pass,
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-no-sha-marker-consumed-out-of-order.json"
                ),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-no-sha-overlapping-markers-two-cleans.json"
                ),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-no-sha-same-head-overlap-sequential.json"
                ),
                ExternalReviewState::Pass,
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-no-sha-duplicate-records-identical.json"
                ),
                ExternalReviewState::Pass,
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-no-sha-duplicate-records-conflict.json"
                ),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-no-sha-duplicate-records-url-conflict.json"
                ),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!("../fixtures/external-review/provider-error.json"),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!("../fixtures/external-review/duplicate-thread.json"),
                ExternalReviewState::ProviderError,
            ),
            (
                include_str!("../fixtures/external-review/history-pr-232-final.json"),
                ExternalReviewState::Pass,
            ),
            (
                include_str!("../fixtures/external-review/dependabot-lockfile-wrong-sha.json"),
                ExternalReviewState::Pass,
            ),
            (
                include_str!("../fixtures/external-review/dependabot-lockfile-unreviewable.json"),
                ExternalReviewState::Pass,
            ),
        ];

        for (contents, expected) in cases {
            assert_eq!(evaluate_snapshot(&fixture(contents)).state, expected);
        }
    }

    #[test]
    fn limits_dependabot_lockfile_completion_to_the_exact_policy_boundary() {
        let contents =
            include_str!("../fixtures/external-review/dependabot-lockfile-wrong-sha.json");
        let pass = evaluate_snapshot(&fixture(contents));
        assert_eq!(pass.state, ExternalReviewState::Pass);
        assert_eq!(pass.provider.as_deref(), Some("dependabot_lockfile_policy"));
        assert!(pass.uses_dependabot_lockfile_policy());
        assert_eq!(pass.finding_count, 0);
        assert_eq!(
            pass.completion_time.as_deref(),
            Some("2026-08-06T03:05:00Z")
        );
        assert_eq!(
            pass.evidence[0].evidence_url,
            "https://github.com/illusion-tech/laneflow/pull/313#discussion_r2"
        );

        let mut missing_review = fixture(contents);
        missing_review.pull_request.reviews.nodes.clear();
        let missing_review_result = evaluate_snapshot(&missing_review);
        assert_eq!(
            missing_review_result.state,
            ExternalReviewState::ProviderError
        );
        assert!(
            missing_review_result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("reviews connection 中不存在的 review"))
        );

        let mut conflicting_actor = fixture(contents);
        conflicting_actor.pull_request.reviews.nodes[0].author = Some(Actor {
            login: "copilot-pull-request-reviewer[bot]".to_string(),
        });
        let mut conflicting_state = fixture(contents);
        conflicting_state.pull_request.reviews.nodes[0].state = "APPROVED".to_string();
        let mut conflicting_commit = fixture(contents);
        conflicting_commit.pull_request.reviews.nodes[0]
            .commit
            .as_mut()
            .expect("fixture review commit")
            .oid = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
        for conflicting in [conflicting_actor, conflicting_state, conflicting_commit] {
            let result = evaluate_snapshot(&conflicting);
            assert_eq!(result.state, ExternalReviewState::ProviderError);
            assert!(result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("与 reviews connection 的 actor/state/commit 不一致")
            }));
        }

        let mut later_clean_review = fixture(contents);
        let mut clean_review = later_clean_review.pull_request.reviews.nodes[0].clone();
        clean_review.id = "PRR-codex-clean-after-machine".to_string();
        clean_review.state = "APPROVED".to_string();
        clean_review.submitted_at = Some("2026-08-06T03:06:00Z".to_string());
        clean_review.url = Some(
            "https://github.com/illusion-tech/laneflow/pull/313#pullrequestreview-2".to_string(),
        );
        later_clean_review
            .pull_request
            .reviews
            .nodes
            .push(clean_review);
        let later_clean_result = evaluate_snapshot(&later_clean_review);
        assert_eq!(later_clean_result.state, ExternalReviewState::Pass);
        assert_eq!(later_clean_result.provider.as_deref(), Some("codex"));
        assert!(later_clean_result.uses_dependabot_lockfile_policy());

        let mut source_change = fixture(contents);
        source_change.pull_request.files.nodes[0].path = "src/lib.rs".to_string();
        assert!(!evaluate_snapshot(&source_change).state.is_pass());

        let mut multiple_commits = fixture(contents);
        multiple_commits
            .pull_request
            .commits
            .nodes
            .push(multiple_commits.pull_request.commits.nodes[0].clone());
        assert!(!evaluate_snapshot(&multiple_commits).state.is_pass());

        let mut human_authored_commit = fixture(contents);
        human_authored_commit.pull_request.commits.nodes[0]
            .commit
            .author
            .as_mut()
            .expect("fixture commit author")
            .name = "Maintainer".to_string();
        assert!(!evaluate_snapshot(&human_authored_commit).state.is_pass());

        let mut dismissed_review = fixture(contents);
        let mut dismissed = dismissed_review.pull_request.reviews.nodes[0].clone();
        dismissed.id = "PRR-dismissed-body-finding".to_string();
        dismissed.state = "DISMISSED".to_string();
        dismissed.body = "Substantive body-only finding".to_string();
        dismissed_review.pull_request.reviews.nodes.push(dismissed);
        assert_eq!(
            evaluate_snapshot(&dismissed_review).state,
            ExternalReviewState::Stale
        );

        let mut unresolved = fixture(contents);
        unresolved.pull_request.review_threads.nodes[0].is_resolved = false;
        assert_eq!(
            evaluate_snapshot(&unresolved).state,
            ExternalReviewState::FindingsOpen
        );

        let mut authorless = fixture(contents);
        authorless.pull_request.review_threads.nodes[0].is_resolved = false;
        authorless.pull_request.review_threads.nodes[0]
            .comments
            .nodes[0]
            .author = None;
        let authorless_result = evaluate_snapshot(&authorless);
        assert!(!authorless_result.state.is_pass());
        assert_eq!(authorless_result.unresolved_actionable_threads, 1);

        let mut untrusted_thread = fixture(contents);
        let thread = &mut untrusted_thread.pull_request.review_threads.nodes[0];
        thread.is_resolved = false;
        thread.is_outdated = false;
        let first_comment = &mut thread.comments.nodes[0];
        first_comment.author = Some(Actor {
            login: "external-contributor".to_string(),
        });
        let thread_review = first_comment
            .pull_request_review
            .as_mut()
            .expect("fixture thread review");
        thread_review.author = Some(Actor {
            login: "external-contributor".to_string(),
        });
        let untrusted_result = evaluate_snapshot(&untrusted_thread);
        assert!(untrusted_result.state.is_pass());
        assert_eq!(untrusted_result.unresolved_actionable_threads, 0);

        let mut authorless_reply_to_untrusted = untrusted_thread.clone();
        let thread = &mut authorless_reply_to_untrusted
            .pull_request
            .review_threads
            .nodes[0];
        let mut authorless_reply = thread.comments.nodes[0].clone();
        authorless_reply.id = "PRRC-authorless-reply-to-untrusted".to_string();
        authorless_reply.author = None;
        thread.comments.nodes.push(authorless_reply);
        let authorless_reply_result = evaluate_snapshot(&authorless_reply_to_untrusted);
        assert!(!authorless_reply_result.state.is_pass());
        assert_eq!(authorless_reply_result.unresolved_actionable_threads, 1);

        let mut trusted_reply_to_untrusted = fixture(contents);
        let mut trusted_reply = trusted_reply_to_untrusted.pull_request.review_threads.nodes[0]
            .comments
            .nodes[0]
            .clone();
        trusted_reply.id = "PRRC-codex-reply-to-untrusted".to_string();
        trusted_reply.body = "Additional substantive concern.".to_string();
        trusted_reply.created_at = "2026-08-06T03:06:00Z".to_string();
        trusted_reply.updated_at = "2026-08-06T03:06:00Z".to_string();
        trusted_reply.url =
            "https://github.com/illusion-tech/laneflow/pull/313#discussion_r3".to_string();
        let trusted_review = trusted_reply
            .pull_request_review
            .as_mut()
            .expect("fixture thread review");
        trusted_review.id = "PRR-codex-reply-to-untrusted".to_string();
        trusted_review.submitted_at = Some("2026-08-06T03:06:00Z".to_string());

        let mut trusted_review_connection =
            trusted_reply_to_untrusted.pull_request.reviews.nodes[0].clone();
        trusted_review_connection.id = "PRR-codex-reply-to-untrusted".to_string();
        trusted_review_connection.submitted_at = Some("2026-08-06T03:06:00Z".to_string());
        trusted_review_connection.url = Some(
            "https://github.com/illusion-tech/laneflow/pull/313#pullrequestreview-2".to_string(),
        );
        trusted_reply_to_untrusted
            .pull_request
            .reviews
            .nodes
            .push(trusted_review_connection);

        let thread = &mut trusted_reply_to_untrusted.pull_request.review_threads.nodes[0];
        thread.is_resolved = false;
        thread.is_outdated = false;
        let first_comment = &mut thread.comments.nodes[0];
        first_comment.author = Some(Actor {
            login: "external-contributor".to_string(),
        });
        first_comment
            .pull_request_review
            .as_mut()
            .expect("fixture thread review")
            .author = Some(Actor {
            login: "external-contributor".to_string(),
        });
        thread.comments.nodes.push(trusted_reply);
        let trusted_reply_result = evaluate_snapshot(&trusted_reply_to_untrusted);
        assert_eq!(
            trusted_reply_result.state,
            ExternalReviewState::FindingsOpen
        );
        assert_eq!(trusted_reply_result.finding_count, 1);
        assert_eq!(trusted_reply_result.unresolved_actionable_threads, 1);

        let mut trusted_reply_after_authorless = trusted_reply_to_untrusted.clone();
        let thread = &mut trusted_reply_after_authorless
            .pull_request
            .review_threads
            .nodes[0];
        thread.is_resolved = true;
        thread.comments.nodes[0].author = None;
        let authorless_reply_result = evaluate_snapshot(&trusted_reply_after_authorless);
        assert_eq!(
            authorless_reply_result.state,
            ExternalReviewState::AwaitingRereview
        );
        assert_eq!(authorless_reply_result.finding_count, 1);
        assert_eq!(authorless_reply_result.unresolved_actionable_threads, 0);

        let mut human_commented = fixture(contents);
        let mut review = human_commented.pull_request.reviews.nodes[0].clone();
        review.id = "PRR-human-commented-body-only".to_string();
        review.state = "COMMENTED".to_string();
        review.body = "Informational review note; no change requested.".to_string();
        review.author = Some(Actor {
            login: "wangzishi".to_string(),
        });
        human_commented.pull_request.reviews.nodes.push(review);
        assert!(evaluate_snapshot(&human_commented).state.is_pass());

        let mut appended_finding = fixture(contents);
        appended_finding.pull_request.review_threads.nodes[0]
            .comments
            .nodes[0]
            .body
            .push_str("\n\nAdditional substantive concern.");
        assert!(!evaluate_snapshot(&appended_finding).state.is_pass());

        let mut trusted_reply_finding = fixture(contents);
        let mut reply = trusted_reply_finding.pull_request.review_threads.nodes[0]
            .comments
            .nodes[0]
            .clone();
        reply.id = "PRRC-codex-follow-up-finding".to_string();
        reply.body = "Additional substantive concern.".to_string();
        reply.created_at = "2026-08-06T03:06:00Z".to_string();
        reply.updated_at = "2026-08-06T03:06:00Z".to_string();
        reply.url = "https://github.com/illusion-tech/laneflow/pull/313#discussion_r3".to_string();
        trusted_reply_finding.pull_request.review_threads.nodes[0]
            .comments
            .nodes
            .push(reply);
        assert!(!evaluate_snapshot(&trusted_reply_finding).state.is_pass());

        let mut missing_disposition = fixture(contents);
        missing_disposition.pull_request.review_threads.nodes[0]
            .comments
            .nodes[1]
            .body
            .replace_range(.."Disposition:".len(), "Rejected:");
        assert!(!evaluate_snapshot(&missing_disposition).state.is_pass());

        let mut claimed_current_head = fixture(contents);
        let head = claimed_current_head.pull_request.head_ref_oid.clone();
        claimed_current_head.pull_request.review_threads.nodes[0]
            .comments
            .nodes[0]
            .body = claimed_current_head.pull_request.review_threads.nodes[0]
            .comments
            .nodes[0]
            .body
            .replace("91f05a4", &head);
        assert!(!evaluate_snapshot(&claimed_current_head).state.is_pass());

        let mut edited_comments = fixture(contents);
        edited_comments.pull_request.review_threads.nodes[0]
            .comments
            .nodes[0]
            .updated_at = "2026-08-06T03:01:00Z".to_string();
        edited_comments.pull_request.review_threads.nodes[0]
            .comments
            .nodes[1]
            .updated_at = "2026-08-06T03:06:00Z".to_string();
        let edited = evaluate_snapshot(&edited_comments);
        assert!(edited.state.is_pass());
        assert_eq!(
            edited.completion_time.as_deref(),
            Some("2026-08-06T03:06:00Z")
        );

        let mut finding_edited_after_disposition = fixture(contents);
        finding_edited_after_disposition
            .pull_request
            .review_threads
            .nodes[0]
            .comments
            .nodes[0]
            .updated_at = "2026-08-06T03:06:00Z".to_string();
        assert!(
            !evaluate_snapshot(&finding_edited_after_disposition)
                .state
                .is_pass()
        );
    }

    #[test]
    fn documents_the_lockfile_only_codeql_not_applicable_semantics() {
        let gates = include_str!("../../docs/governance/development-gates.md");
        let scanning = include_str!("../../docs/governance/security-scanning.md");
        let dependency = include_str!("../../docs/governance/dependency-security.md");
        let template = include_str!("../../.github/pull_request_template.md");
        let agent_guide = include_str!("../../docs/governance/agent-development-guide.md");

        assert!(gates.contains("dependabot-cargo-lock-only-v1"));
        assert!(scanning.contains("dependabot-cargo-lock-only-v1"));
        assert!(scanning.contains("NEUTRAL"));
        assert!(scanning.contains("2 configurations not found"));
        assert!(scanning.contains("not applicable"));
        for entry_point in [dependency, template, agent_guide] {
            assert!(entry_point.contains("dependabot-cargo-lock-only-v1"));
        }
    }

    #[test]
    fn rejects_self_review_even_for_trusted_human() {
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/human-approved.json"
        ));
        snapshot.pull_request.author = Some(Actor {
            login: "wangzishi".to_string(),
        });
        assert_eq!(
            evaluate_snapshot(&snapshot).state,
            ExternalReviewState::AwaitingReview
        );
    }

    #[test]
    fn unresolved_zero_without_completion_is_not_pass() {
        let mut snapshot = fixture(include_str!("../fixtures/external-review/codex-clean.json"));
        snapshot.pull_request.comments.nodes.clear();
        assert_eq!(
            evaluate_snapshot(&snapshot).state,
            ExternalReviewState::AwaitingReview
        );
    }

    #[test]
    fn edited_codex_clean_comment_fails_closed() {
        let mut snapshot = fixture(include_str!("../fixtures/external-review/codex-clean.json"));
        snapshot.pull_request.comments.nodes[0].updated_at = "2026-07-24T14:47:49Z".to_string();
        let result = evaluate_snapshot(&snapshot);

        assert_eq!(result.state, ExternalReviewState::ProviderError);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("在创建后被编辑"))
        );
    }

    #[test]
    fn only_strictly_later_current_head_clean_supersedes_unbound_clean() {
        let superseded = evaluate_snapshot(&fixture(include_str!(
            "../fixtures/external-review/codex-old-no-sha-then-current-clean.json"
        )));
        let late_ambiguity = evaluate_snapshot(&fixture(include_str!(
            "../fixtures/external-review/codex-current-clean-then-no-sha.json"
        )));
        let same_second = evaluate_snapshot(&fixture(include_str!(
            "../fixtures/external-review/codex-old-no-sha-same-second-current-clean.json"
        )));
        let mut edited_snapshot = fixture(include_str!(
            "../fixtures/external-review/codex-old-no-sha-then-current-clean.json"
        ));
        edited_snapshot.pull_request.comments.nodes[0].updated_at =
            "2026-08-05T07:00:01Z".to_string();
        let edited = evaluate_snapshot(&edited_snapshot);

        assert_eq!(superseded.state, ExternalReviewState::Pass);
        assert!(superseded.diagnostics.is_empty());
        assert_eq!(late_ambiguity.state, ExternalReviewState::ProviderError);
        assert_eq!(same_second.state, ExternalReviewState::ProviderError);
        for result in [late_ambiguity, same_second] {
            assert!(result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("没有严格晚于它的 current-head clean completion")
            }));
        }
        assert_eq!(edited.state, ExternalReviewState::ProviderError);
        assert!(
            edited
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("在创建后被编辑"))
        );
    }

    #[test]
    fn draft_pr_never_passes() {
        let mut snapshot = fixture(include_str!("../fixtures/external-review/codex-clean.json"));
        snapshot.pull_request.is_draft = true;

        assert_eq!(
            evaluate_snapshot(&snapshot).state,
            ExternalReviewState::ReviewPending
        );
    }

    #[test]
    fn live_api_error_serializes_as_provider_error() {
        let result = ExternalReviewResult::provider_error(
            "illusion-tech/laneflow",
            232,
            "network unavailable".to_string(),
        );
        let json = serde_json::to_value(&result).expect("result should serialize");

        assert_eq!(result.state, ExternalReviewState::ProviderError);
        assert_eq!(json["state"], "provider_error");
        assert_eq!(json["diagnostics"][0], "network unavailable");
    }

    #[test]
    fn exact_head_clean_after_finding_passes() {
        let result = evaluate_snapshot(&fixture(include_str!(
            "../fixtures/external-review/history-pr-232-final.json"
        )));
        assert_eq!(result.state, ExternalReviewState::Pass);
        assert_eq!(result.finding_count, 2);
        assert_eq!(result.unresolved_actionable_threads, 0);
        assert!(!result.requires_rereview);
    }

    #[test]
    fn valid_waiver_stays_separate_from_pass() {
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/stale-old-head.json"
        ));
        snapshot.waiver = Some(WaiverInput {
            id: "waiver-230-1".to_string(),
            exception_type: "content_equivalent_rebase".to_string(),
            current_head_oid: snapshot.pull_request.head_ref_oid.clone(),
            current_base_oid: snapshot.pull_request.base_ref_oid.clone(),
            reason: "validated equivalent rebase".to_string(),
            evidence_urls: vec!["https://github.com/illusion-tech/laneflow/issues/230".to_string()],
            risk: "reviewed commit identity changed".to_string(),
            acceptance_boundary: "exact paths and blobs only".to_string(),
            expires_at: "2026-07-25T00:00:00Z".to_string(),
            follow_up_issue: "#230".to_string(),
            cleanup_owner: "wangzishi".to_string(),
            authorized_by: "wangzishi".to_string(),
            historical_base_replay: false,
            grandfathered_confirmed_gate_defect: false,
        });
        assert_eq!(
            evaluate_snapshot(&snapshot).state,
            ExternalReviewState::Waived
        );
    }

    #[test]
    fn historical_ordinary_waiver_preserves_recorded_pre_merge_base() {
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/stale-old-head.json"
        ));
        snapshot.waiver = Some(WaiverInput {
            id: "waiver-230-historical-outage".to_string(),
            exception_type: "provider_platform_outage".to_string(),
            current_head_oid: snapshot.pull_request.head_ref_oid.clone(),
            current_base_oid: "cccccccccccccccccccccccccccccccccccccccc".to_string(),
            reason: "the provider was unavailable before merge".to_string(),
            evidence_urls: vec!["https://github.com/illusion-tech/laneflow/issues/230".to_string()],
            risk: "review coverage was unavailable at merge".to_string(),
            acceptance_boundary: "historical G4 replay only".to_string(),
            expires_at: "2026-07-25T00:00:00Z".to_string(),
            follow_up_issue: "#230".to_string(),
            cleanup_owner: "wangzishi".to_string(),
            authorized_by: "wangzishi".to_string(),
            historical_base_replay: true,
            grandfathered_confirmed_gate_defect: false,
        });

        assert_eq!(
            evaluate_snapshot(&snapshot).state,
            ExternalReviewState::Waived
        );
    }

    #[test]
    fn post_policy_replay_does_not_grandfather_confirmed_gate_defect() {
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/stale-old-head.json"
        ));
        snapshot.waiver = Some(WaiverInput {
            id: "waiver-230-confirmed-defect".to_string(),
            exception_type: "confirmed_gate_defect".to_string(),
            current_head_oid: snapshot.pull_request.head_ref_oid.clone(),
            current_base_oid: "cccccccccccccccccccccccccccccccccccccccc".to_string(),
            reason: "the Gate has a confirmed false block".to_string(),
            evidence_urls: vec!["https://github.com/illusion-tech/laneflow/issues/405".to_string()],
            risk: "the automated assertion remains failed".to_string(),
            acceptance_boundary: "must use G3 Exception instead".to_string(),
            expires_at: "2026-07-25T00:00:00Z".to_string(),
            follow_up_issue: "#405".to_string(),
            cleanup_owner: "wangzishi".to_string(),
            authorized_by: "wangzishi".to_string(),
            historical_base_replay: true,
            grandfathered_confirmed_gate_defect: false,
        });

        let result = evaluate_snapshot(&snapshot);
        assert_eq!(result.state, ExternalReviewState::ProviderError);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("confirmed_gate_defect"))
        );
    }

    #[test]
    fn historical_confirmed_gate_defect_waiver_remains_replayable() {
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/stale-old-head.json"
        ));
        snapshot.waiver = Some(WaiverInput {
            id: "waiver-230-historical-confirmed-defect".to_string(),
            exception_type: "confirmed_gate_defect".to_string(),
            current_head_oid: snapshot.pull_request.head_ref_oid.clone(),
            current_base_oid: "cccccccccccccccccccccccccccccccccccccccc".to_string(),
            reason: "the Gate had a confirmed false block before the policy change".to_string(),
            evidence_urls: vec!["https://github.com/illusion-tech/laneflow/issues/405".to_string()],
            risk: "the automated assertion remained failed at merge".to_string(),
            acceptance_boundary: "historical G4 replay only".to_string(),
            expires_at: "2026-07-25T00:00:00Z".to_string(),
            follow_up_issue: "#405".to_string(),
            cleanup_owner: "wangzishi".to_string(),
            authorized_by: "wangzishi".to_string(),
            historical_base_replay: true,
            grandfathered_confirmed_gate_defect: true,
        });

        assert_eq!(
            evaluate_snapshot(&snapshot).state,
            ExternalReviewState::Waived
        );
    }

    #[test]
    fn historical_replay_context_is_not_part_of_snapshot_schema_v1() {
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/stale-old-head.json"
        ));
        snapshot.waiver = Some(WaiverInput {
            id: "waiver-230-historical-confirmed-defect".to_string(),
            exception_type: "confirmed_gate_defect".to_string(),
            current_head_oid: snapshot.pull_request.head_ref_oid.clone(),
            current_base_oid: "cccccccccccccccccccccccccccccccccccccccc".to_string(),
            reason: "the Gate had a confirmed false block before the policy change".to_string(),
            evidence_urls: vec!["https://github.com/illusion-tech/laneflow/issues/405".to_string()],
            risk: "the automated assertion remained failed at merge".to_string(),
            acceptance_boundary: "historical G4 replay only".to_string(),
            expires_at: "2026-07-25T00:00:00Z".to_string(),
            follow_up_issue: "#405".to_string(),
            cleanup_owner: "wangzishi".to_string(),
            authorized_by: "wangzishi".to_string(),
            historical_base_replay: true,
            grandfathered_confirmed_gate_defect: true,
        });

        let schema_v1 = serde_json::to_string(&snapshot).expect("snapshot must serialize");
        assert!(!schema_v1.contains("historicalReplay"));
        assert!(!schema_v1.contains("historicalBaseReplay"));
        assert!(!schema_v1.contains("grandfatheredConfirmedGateDefect"));
        let restored: ExternalReviewSnapshot =
            serde_json::from_str(&schema_v1).expect("schema-v1 waiver must remain readable");
        let restored_waiver = restored.waiver.unwrap();
        assert!(!restored_waiver.historical_base_replay);
        assert!(!restored_waiver.grandfathered_confirmed_gate_defect);
    }

    #[test]
    fn draft_pr_cannot_be_waived() {
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/stale-old-head.json"
        ));
        snapshot.pull_request.is_draft = true;
        snapshot.waiver = Some(WaiverInput {
            id: "waiver-230-2".to_string(),
            exception_type: "provider_platform_outage".to_string(),
            current_head_oid: snapshot.pull_request.head_ref_oid.clone(),
            current_base_oid: snapshot.pull_request.base_ref_oid.clone(),
            reason: "all configured providers unavailable".to_string(),
            evidence_urls: vec!["https://github.com/illusion-tech/laneflow/issues/230".to_string()],
            risk: "review coverage unavailable".to_string(),
            acceptance_boundary: "metadata-only governance change".to_string(),
            expires_at: "2026-07-25T00:00:00Z".to_string(),
            follow_up_issue: "#230".to_string(),
            cleanup_owner: "wangzishi".to_string(),
            authorized_by: "wangzishi".to_string(),
            historical_base_replay: false,
            grandfathered_confirmed_gate_defect: false,
        });

        assert_eq!(
            evaluate_snapshot(&snapshot).state,
            ExternalReviewState::ReviewPending
        );
    }

    #[test]
    fn parses_codex_reviewed_commit_prefix() {
        assert_eq!(
            parse_reviewed_commit(
                "Codex Review: Didn't find any major issues.\n\n**Reviewed commit:** `c22802bb6b`"
            ),
            Some("c22802bb6b")
        );
        assert_eq!(
            parse_reviewed_commit("Codex Review: Didn't find any major issues."),
            None
        );
    }

    #[test]
    fn parses_cli_sources_and_expected_state() {
        let args = vec![
            "--repo".to_string(),
            "illusion-tech/laneflow".to_string(),
            "--pr".to_string(),
            "232".to_string(),
            "--expect".to_string(),
            "pass".to_string(),
        ];
        let parsed = parse_args(&args).expect("live args should parse");
        assert_eq!(parsed.expected_state, Some(ExternalReviewState::Pass));
        assert!(matches!(parsed.source, InputSource::Live { pr: 232, .. }));
    }

    #[test]
    fn binds_codex_clean_comment_via_trusted_binding_record() {
        let result = evaluate_snapshot(&fixture(include_str!(
            "../fixtures/external-review/codex-no-sha-bound-clean.json"
        )));
        assert_eq!(result.state, ExternalReviewState::Pass);
        assert_eq!(result.provider.as_deref(), Some("codex"));
        assert_eq!(
            result.reviewed_head_oid.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(
            result.completion_time.as_deref(),
            Some("2026-08-19T01:10:00Z")
        );
        let binding = result
            .evidence
            .iter()
            .find(|item| item.source_kind == "binding_record")
            .expect("binding_record evidence");
        assert_eq!(
            binding.reviewed_head_oid,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(
            binding.evidence_url,
            "https://github.com/illusion-tech/laneflow/pull/430#issuecomment-102"
        );
    }

    #[test]
    fn rejects_binding_record_fail_closed_variants() {
        let cases = [
            (
                include_str!("../fixtures/external-review/codex-no-sha-binding-edited-clean.json"),
                "不能作为 append-only completion",
            ),
            (
                include_str!("../fixtures/external-review/codex-no-sha-edited-record.json"),
                "在创建后被编辑，不能作为 append-only 记录",
            ),
            (
                include_str!("../fixtures/external-review/codex-no-sha-wrong-actor-record.json"),
                "不是受信 publisher",
            ),
            (
                include_str!("../fixtures/external-review/codex-no-sha-duplicate-records.json"),
                "引用同一 clean comment",
            ),
            (
                include_str!("../fixtures/external-review/codex-no-sha-record-missing-clean.json"),
                "未绑定到任何无 SHA clean comment",
            ),
            (
                include_str!("../fixtures/external-review/codex-no-sha-record-missing-marker.json"),
                "受控 request marker `IC-marker-missing` 不存在",
            ),
            (
                include_str!("../fixtures/external-review/codex-clean-with-marker-and-record.json"),
                "未绑定到任何无 SHA clean comment",
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-no-sha-bound-clean-then-unbound.json"
                ),
                "缺少可解析的 Reviewed commit",
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-no-sha-record-marker-head-mismatch.json"
                ),
                "request head/base 不一致",
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-no-sha-record-marker-same-second.json"
                ),
                "必须严格早于",
            ),
            (
                include_str!("../fixtures/external-review/codex-no-sha-cross-push-late-clean.json"),
                "无法证明 clean 归属",
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-no-sha-marker-consumed-out-of-order.json"
                ),
                "无法证明 clean 归属",
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-no-sha-overlapping-markers-two-cleans.json"
                ),
                "无法证明 clean 归属",
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-no-sha-duplicate-records-conflict.json"
                ),
                "重复 Codex clean binding record id",
            ),
            (
                include_str!(
                    "../fixtures/external-review/codex-no-sha-duplicate-records-url-conflict.json"
                ),
                "重复 Codex clean binding record id",
            ),
        ];
        for (contents, expected_diagnostic) in cases {
            let result = evaluate_snapshot(&fixture(contents));
            assert_eq!(result.state, ExternalReviewState::ProviderError);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.contains(expected_diagnostic)),
                "诊断应包含 `{expected_diagnostic}`，实际：{:?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn bound_old_head_clean_goes_stale_without_ambiguity() {
        let result = evaluate_snapshot(&fixture(include_str!(
            "../fixtures/external-review/codex-no-sha-bound-old-head.json"
        )));
        assert_eq!(result.state, ExternalReviewState::Stale);
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.contains("未绑定到任何无 SHA clean comment"))
        );
    }

    #[test]
    fn binding_record_evidence_keeps_clean_comment_completion_time() {
        let result = evaluate_snapshot(&fixture(include_str!(
            "../fixtures/external-review/codex-no-sha-bound-clean-finding-before-record.json"
        )));
        // finding 落在 clean comment 与 binding record 之间：completion 排序必须用
        // clean comment 的创建时间（01:10），否则旧 clean 会错误覆盖新 finding 变成 Pass
        assert_eq!(result.state, ExternalReviewState::AwaitingRereview);
        assert!(result.requires_rereview);
        let binding = result
            .evidence
            .iter()
            .find(|item| item.source_kind == "binding_record")
            .expect("binding_record evidence");
        assert_eq!(binding.submitted_at, "2026-08-19T01:10:00Z");
    }

    #[test]
    fn stale_binding_consumes_marker_so_late_clean_can_bind_current_head() {
        let result = evaluate_snapshot(&fixture(include_str!(
            "../fixtures/external-review/codex-no-sha-cross-push-stale-then-current.json"
        )));
        assert_eq!(result.state, ExternalReviewState::Pass);
        assert_eq!(
            result.reviewed_head_oid.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(
            result.completion_time.as_deref(),
            Some("2026-08-19T01:25:00Z")
        );
        assert_eq!(
            result
                .evidence
                .iter()
                .filter(|item| item.source_kind == "binding_record")
                .count(),
            2
        );
    }

    #[test]
    fn identical_duplicate_binding_records_dedupe_to_one() {
        let result = evaluate_snapshot(&fixture(include_str!(
            "../fixtures/external-review/codex-no-sha-duplicate-records-identical.json"
        )));
        assert_eq!(result.state, ExternalReviewState::Pass);
        assert_eq!(
            result
                .evidence
                .iter()
                .filter(|item| item.source_kind == "binding_record")
                .count(),
            1
        );
    }

    #[test]
    fn same_head_overlapping_markers_are_consumed_in_order() {
        let result = evaluate_snapshot(&fixture(include_str!(
            "../fixtures/external-review/codex-no-sha-same-head-overlap-sequential.json"
        )));
        // 未消费候选 head/base 全同（M1、M2 均为 current head）：无歧义，
        // C1 绑最早的 M1，M1 消费后 C2 绑 M2
        assert_eq!(result.state, ExternalReviewState::Pass);
        assert_eq!(
            result.completion_time.as_deref(),
            Some("2026-08-19T01:15:00Z")
        );
        assert_eq!(
            result
                .evidence
                .iter()
                .filter(|item| item.source_kind == "binding_record")
                .count(),
            2
        );
    }

    #[test]
    fn sweep_plans_unbound_clean_against_records_from_the_same_run() {
        let snapshot = fixture(include_str!(
            "../fixtures/external-review/codex-no-sha-same-head-overlap-sequential.json"
        ));
        let pr = &snapshot.pull_request;
        let mut diagnostics = Vec::new();
        let records = collect_codex_clean_binding_records(pr, &mut diagnostics);
        assert!(diagnostics.is_empty());
        assert_eq!(records.len(), 2);

        // 模拟 C2 的 issue_comment 事件被同组 pending 挤掉：records 里只有
        // C1 的绑定时，sweep 应为 C2 计划绑定 M2（与 evaluator 消费语义一致）。
        let mut sweep_diagnostics = Vec::new();
        let (clean, marker_comment, _) =
            plan_next_sweep_binding(pr, &records[..1], &mut sweep_diagnostics)
                .expect("C2 待 sweep 补绑");
        assert_eq!(clean.id, "IC-codex-clean-nosha-2");
        assert_eq!(marker_comment.id, "IC-marker-2");
        assert!(sweep_diagnostics.is_empty());

        // 两条 clean 均已绑定后 sweep 无剩余工作。
        let mut sweep_diagnostics = Vec::new();
        assert!(plan_next_sweep_binding(pr, &records, &mut sweep_diagnostics).is_none());
        assert!(sweep_diagnostics.is_empty());
    }

    #[test]
    fn sweep_skips_clean_without_consumable_marker_with_diagnostic() {
        let snapshot = fixture(include_str!(
            "../fixtures/external-review/codex-no-sha-bound-clean-then-unbound.json"
        ));
        let pr = &snapshot.pull_request;
        let mut diagnostics = Vec::new();
        let records = collect_codex_clean_binding_records(pr, &mut diagnostics);
        assert!(diagnostics.is_empty());
        assert_eq!(records.len(), 1);

        // C2 之前唯一的 marker 已被 C1 的 record 消费：sweep 跳过并记诊断，
        // 不发布任何 record（evaluator 会独立对 C2 fail-closed 诊断）。
        let mut sweep_diagnostics = Vec::new();
        assert!(plan_next_sweep_binding(pr, &records, &mut sweep_diagnostics).is_none());
        assert_eq!(sweep_diagnostics.len(), 1);
        assert!(sweep_diagnostics[0].contains("IC-codex-clean-nosha-2"));
        assert!(sweep_diagnostics[0].contains("均已被既有 binding record 消费"));
    }

    #[test]
    fn untrusted_comment_echo_deletes_the_posted_comment() {
        let posted = |login: &str| PostedComment {
            id: 42,
            node_id: "IC_node".to_string(),
            body: Some("expected body".to_string()),
            user: Some(RestUser {
                login: login.to_string(),
            }),
            html_url: "https://github.com/illusion-tech/laneflow/pull/430#issuecomment-42"
                .to_string(),
        };

        // 受信 echo：不触发删除
        let mut deleted = Vec::new();
        ensure_trusted_comment_echo_or_delete(
            &posted("github-actions[bot]"),
            "expected body",
            |id| {
                deleted.push(id);
                Ok(())
            },
        )
        .expect("trusted echo");
        assert!(deleted.is_empty());

        // 不可信 echo：删除成功，错误信息包含删除结果
        let mut deleted = Vec::new();
        let error =
            ensure_trusted_comment_echo_or_delete(&posted("some-dev"), "expected body", |id| {
                deleted.push(id);
                Ok(())
            })
            .expect_err("untrusted echo must fail");
        assert_eq!(deleted, vec![42]);
        assert!(error.contains("不是受信 publisher"));
        assert!(error.contains("已删除不可信 comment 42"));

        // 不可信 echo 且删除失败：错误信息要求人工删除
        let error =
            ensure_trusted_comment_echo_or_delete(&posted("some-dev"), "expected body", |_| {
                Err("permission denied".to_string())
            })
            .expect_err("untrusted echo must fail");
        assert!(error.contains("删除不可信 comment 42 失败：permission denied"));
        assert!(error.contains("需人工删除"));
    }

    #[test]
    fn branch_endpoint_encodes_special_characters() {
        assert_eq!(
            branch_endpoint("illusion-tech/laneflow", "feature/#430"),
            "repos/illusion-tech/laneflow/branches/feature/%23430"
        );
        assert_eq!(
            branch_endpoint("illusion-tech/laneflow", "430-codex-clean-binding"),
            "repos/illusion-tech/laneflow/branches/430-codex-clean-binding"
        );
        assert_eq!(
            branch_endpoint("illusion-tech/laneflow", "feature/100%?x"),
            "repos/illusion-tech/laneflow/branches/feature/100%25%3Fx"
        );
        assert_eq!(
            issue_comment_endpoint("illusion-tech/laneflow", 42),
            "repos/illusion-tech/laneflow/issues/comments/42"
        );
    }

    #[test]
    fn truncated_comments_connection_fails_closed_in_snapshot_replay() {
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/codex-no-sha-bound-clean.json"
        ));
        snapshot.pull_request.comments.page_info.has_next_page = true;
        let result = evaluate_snapshot(&snapshot);
        assert_eq!(result.state, ExternalReviewState::ProviderError);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("issue comments 超过 100 条"))
        );
    }

    #[test]
    fn comment_pagination_accumulates_pages_and_fails_closed() {
        let comment = |id: &str| IssueComment {
            id: id.to_string(),
            author: None,
            body: String::new(),
            created_at: "2026-08-19T01:00:00Z".to_string(),
            updated_at: "2026-08-19T01:00:00Z".to_string(),
            url: format!("https://github.com/illusion-tech/laneflow/pull/430#issuecomment-{id}"),
        };
        let page =
            |nodes: Vec<IssueComment>, has_next: bool, cursor: Option<&str>| CommentsConnection {
                nodes,
                page_info: CommentsPageInfo {
                    has_next_page: has_next,
                    end_cursor: cursor.map(str::to_string),
                },
            };

        let mut pages = vec![
            page(vec![comment("1")], true, Some("cursor-1")),
            page(vec![comment("2"), comment("3")], false, None),
        ]
        .into_iter();
        let collected =
            fetch_comment_pages(|_| Ok(pages.next().expect("page"))).expect("paginated comments");
        assert_eq!(collected.len(), 3);
        assert_eq!(collected[2].id, "3");

        let mut missing_cursor = vec![page(vec![comment("1")], true, None)].into_iter();
        let error = fetch_comment_pages(|_| Ok(missing_cursor.next().expect("page")))
            .expect_err("missing endCursor must fail closed");
        assert!(error.contains("缺少 endCursor"));

        let mut empty_page = vec![page(Vec::new(), true, Some("cursor-1"))].into_iter();
        let error = fetch_comment_pages(|_| Ok(empty_page.next().expect("page")))
            .expect_err("empty page with hasNextPage must fail closed");
        assert!(error.contains("空页"));

        let mut stuck_cursor = vec![
            page(vec![comment("1")], true, Some("cursor-1")),
            page(vec![comment("2")], true, Some("cursor-1")),
        ]
        .into_iter();
        let error = fetch_comment_pages(|_| Ok(stuck_cursor.next().expect("page")))
            .expect_err("non-advancing cursor must fail closed");
        assert!(error.contains("未推进"));
    }

    #[test]
    fn parses_hidden_records_strictly() {
        let body = "prefix\n\n<!-- codex-review-request:v1 {\"schemaVersion\":1,\"id\":\"req-1\",\"pr\":430,\"requestHeadOid\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"requestBaseOid\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"} -->\n";
        let record: CodexReviewRequestRecord =
            parse_hidden_record(body, CODEX_REVIEW_REQUEST_MARKER)
                .expect("marker present")
                .expect("valid record");
        assert_eq!(record.id, "req-1");
        assert!(
            parse_hidden_record::<CodexReviewRequestRecord>(
                "no marker here",
                CODEX_REVIEW_REQUEST_MARKER
            )
            .is_none()
        );
        assert!(parse_hidden_record::<CodexReviewRequestRecord>(
            "<!-- codex-review-request:v1 {\"schemaVersion\":1} --> tail <!-- codex-review-request:v1 {} -->",
            CODEX_REVIEW_REQUEST_MARKER
        )
        .expect("marker present")
        .is_err());
        assert!(
            parse_hidden_record::<CodexReviewRequestRecord>(
                "<!-- codex-review-request:v1 {\"schemaVersion\":1}",
                CODEX_REVIEW_REQUEST_MARKER
            )
            .expect("marker present")
            .is_err()
        );
    }

    #[test]
    fn formats_epoch_seconds_as_rfc3339() {
        assert_eq!(epoch_seconds_to_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(
            epoch_seconds_to_rfc3339(1_704_067_200),
            "2024-01-01T00:00:00Z"
        );
        assert_eq!(
            epoch_seconds_to_rfc3339(1_787_101_240),
            "2026-08-19T01:00:40Z"
        );
        assert_eq!(
            epoch_seconds_to_rfc3339(4_102_444_799),
            "2099-12-31T23:59:59Z"
        );
    }

    #[test]
    fn parses_codex_binding_subcommand_arguments() {
        let request_args = [
            "--repo".to_string(),
            "illusion-tech/laneflow".to_string(),
            "--pr".to_string(),
            "430".to_string(),
            "--dry-run".to_string(),
        ];
        let parsed = parse_request_codex_review_args(&request_args).expect("valid request args");
        assert_eq!(parsed.pr, 430);
        assert!(parsed.dry_run);
        assert!(
            parse_request_codex_review_args(&request_args[..request_args.len() - 1])
                .is_ok_and(|parsed| !parsed.dry_run)
        );
        assert!(
            parse_request_codex_review_args(&[
                "--repo".to_string(),
                "illusion-tech/laneflow".to_string(),
            ])
            .is_err()
        );

        let binding_args = [
            "--repo".to_string(),
            "illusion-tech/laneflow".to_string(),
            "--pr".to_string(),
            "430".to_string(),
            "--clean-comment-id".to_string(),
            "IC_kwDOtest".to_string(),
            "--run-url".to_string(),
            "https://github.com/illusion-tech/laneflow/actions/runs/1".to_string(),
        ];
        let parsed = parse_publish_codex_clean_binding_args(&binding_args).expect("valid args");
        assert_eq!(parsed.clean_comment_id, "IC_kwDOtest");
        assert!(!parsed.dry_run);

        let mut wrong_run_url = binding_args.clone();
        wrong_run_url[7] = "https://example.com/actions/runs/1".to_string();
        assert!(parse_publish_codex_clean_binding_args(&wrong_run_url).is_err());

        let mut whitespace_id = binding_args.clone();
        whitespace_id[5] = "IC_kwDO test".to_string();
        assert!(parse_publish_codex_clean_binding_args(&whitespace_id).is_err());
    }
}
