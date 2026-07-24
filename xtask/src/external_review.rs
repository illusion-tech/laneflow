use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

const SNAPSHOT_SCHEMA_VERSION: u64 = 1;
const RESULT_SCHEMA_VERSION: u64 = 1;
const CHECK_PUBLISH_RESULT_SCHEMA_VERSION: u64 = 1;
const EXTERNAL_REVIEW_CHECK_NAME: &str = "External Review Gate";
const EXPECTED_CHECK_APP_SLUG: &str = "github-actions";
const COPILOT_ACTOR: &str = "copilot-pull-request-reviewer";
const CODEX_ACTOR: &str = "chatgpt-codex-connector";
const TRUSTED_HUMAN_ACTORS: &[&str] = &["wangzishi"];

const EXTERNAL_REVIEW_QUERY: &str = r#"
query($owner:String!, $name:String!, $number:Int!) {
  repository(owner:$owner, name:$name) {
    pullRequest(number:$number) {
      number
      author { login }
      headRefOid
      baseRefOid
      isDraft
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
      isCrossRepository
      isDraft
      state
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
struct CheckRunsResponse {
    check_runs: Vec<CheckRunResponse>,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublishCheckReuse {
    schema_version: u64,
    repository: String,
    pull_request: u64,
    current_head_oid: String,
    state: ExternalReviewState,
    conclusion: String,
    reused: bool,
    check_run_id: u64,
    check_run_url: String,
    external_id: String,
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
    if let Some(existing) =
        find_existing_equivalent_check(&args.repository, &payload, &evaluation_key)?
    {
        let existing_external_id = existing
            .external_id
            .ok_or("等价 Check Run 缺少 external ID")?;
        println!(
            "{}",
            serde_json::to_string_pretty(&PublishCheckReuse {
                schema_version: CHECK_PUBLISH_RESULT_SCHEMA_VERSION,
                repository: args.repository,
                pull_request: args.pr,
                current_head_oid: result.current_head_oid,
                state: result.state,
                conclusion: payload.conclusion.to_string(),
                reused: true,
                check_run_id: existing.id,
                check_run_url: existing.html_url,
                external_id: existing_external_id,
            })
            .map_err(|error| format!("无法序列化 shadow Check 复用结果：{error}"))?
        );
        return Ok(());
    }
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

    let mut review_to_finding_threads = BTreeMap::<String, usize>::new();
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
        let Some(actor) = first_comment.author.as_ref() else {
            continue;
        };
        if trusted_provider(&actor.login, &author).is_none() {
            continue;
        }
        let Some(review) = first_comment.pull_request_review.as_ref() else {
            diagnostics.push(format!(
                "受信任 reviewer 的 thread `{}` 缺少 pullRequestReview 关联",
                thread.id
            ));
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
        finding_thread_ids.insert(thread.id.clone());
        *review_to_finding_threads
            .entry(review.id.clone())
            .or_default() += 1;
        if !thread.is_resolved && !thread.is_outdated {
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
            .copied()
            .unwrap_or_default();
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

    for comment in &pr.comments.nodes {
        let Some(actor) = comment.author.as_ref() else {
            continue;
        };
        if normalize_actor(&actor.login) != CODEX_ACTOR {
            continue;
        }
        if comment.body.contains("To use Codex here") {
            diagnostics.push(format!("Codex provider 报告环境不可用：{}", comment.url));
            continue;
        }
        if !comment.body.contains("Codex Review:") {
            continue;
        }
        if !comment.body.contains("Didn't find any major issues") {
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
            diagnostics.push(format!(
                "Codex clean comment `{}` 缺少可解析的 Reviewed commit",
                comment.id
            ));
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
        "confirmed_gate_defect",
    ];
    if !ALLOWED_TYPES.contains(&waiver.exception_type.as_str()) {
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
    if waiver.current_base_oid != pr.base_ref_oid {
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
        || initial.is_cross_repository != final_identity.is_cross_repository
        || initial.is_draft != final_identity.is_draft
        || initial.state != final_identity.state
    {
        return Err(format!(
            "PR identity 在 shadow Gate 运行期间发生变化：initial=({}, {}, {}, {}, {}, {}, {}) final=({}, {}, {}, {}, {}, {}, {})",
            initial.number,
            initial.head_ref_oid,
            initial.base_ref_oid,
            initial.base_ref_name,
            initial.is_cross_repository,
            initial.is_draft,
            initial.state,
            final_identity.number,
            final_identity.head_ref_oid,
            final_identity.base_ref_oid,
            final_identity.base_ref_name,
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
        name: EXTERNAL_REVIEW_CHECK_NAME,
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

fn find_existing_equivalent_check(
    repository: &str,
    payload: &CheckRunPayload,
    evaluation_key: &str,
) -> Result<Option<CheckRunResponse>, String> {
    let endpoint = format!("repos/{repository}/commits/{}/check-runs", payload.head_sha);
    let output = Command::new("gh")
        .args([
            "api",
            "--method",
            "GET",
            &endpoint,
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "X-GitHub-Api-Version: 2022-11-28",
            "-f",
            &format!("check_name={}", payload.name),
            "-f",
            "filter=latest",
            "-f",
            "per_page=100",
        ])
        .output()
        .map_err(|error| format!("无法运行 gh Check Run 查询：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh Check Run 查询失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let response: CheckRunsResponse = serde_json::from_slice(&output.stdout).map_err(|error| {
        format!(
            "gh Check Run 查询输出不是预期 JSON：{error}；原始输出：{}",
            String::from_utf8_lossy(&output.stdout).trim()
        )
    })?;
    select_existing_equivalent_check(response.check_runs, payload, evaluation_key)
}

fn select_existing_equivalent_check(
    check_runs: Vec<CheckRunResponse>,
    payload: &CheckRunPayload,
    evaluation_key: &str,
) -> Result<Option<CheckRunResponse>, String> {
    let expected_external_prefix = format!("{evaluation_key}:run-");
    let mut matching = check_runs
        .into_iter()
        .filter(|check| {
            check.app.slug == EXPECTED_CHECK_APP_SLUG
                && check
                    .external_id
                    .as_deref()
                    .is_some_and(|external_id| external_id.starts_with(&expected_external_prefix))
        })
        .collect::<Vec<_>>();
    for check in &matching {
        let conclusion = check.conclusion.as_deref().unwrap_or_default();
        if check.name != payload.name
            || check.head_sha != payload.head_sha
            || check.status != payload.status
            || conclusion != payload.conclusion
        {
            return Err(format!(
                "等价 fingerprint 已存在但 Check 绑定不一致：id={} name={} head={} status={} conclusion={} app={}",
                check.id, check.name, check.head_sha, check.status, conclusion, check.app.slug
            ));
        }
    }
    matching.sort_by_key(|check| check.id);
    Ok(matching.pop())
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
        || response.app.slug != EXPECTED_CHECK_APP_SLUG
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
            EXPECTED_CHECK_APP_SLUG
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
    let pull_request = repository_data
        .pull_request
        .ok_or_else(|| format!("GitHub PR 不存在或不可读：{repository}#{pr}"))?;
    Ok(ExternalReviewSnapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        repository: repository.to_string(),
        pull_request,
        provider_errors: Vec::new(),
        waiver: None,
    })
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
    let output = Command::new("gh")
        .args([
            "api",
            "graphql",
            "-F",
            &format!("owner={owner}"),
            "-F",
            &format!("name={name}"),
            "-F",
            &format!("number={pr}"),
            "-f",
            &format!("query={query}"),
        ])
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

        assert_eq!(payload.name, EXTERNAL_REVIEW_CHECK_NAME);
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
                slug: EXPECTED_CHECK_APP_SLUG.to_string(),
            },
        };

        assert!(verify_check_run_response(&response, &payload).is_ok());
        response.app.slug = "unexpected-app".to_string();
        assert!(verify_check_run_response(&response, &payload).is_err());
    }

    #[test]
    fn fingerprints_and_reuses_only_equivalent_completed_checks() {
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

        let payload = build_check_run_payload(
            &pass,
            "https://github.com/illusion-tech/laneflow/actions/runs/1",
            "evaluation-key:run-2-1".to_string(),
        );
        let equivalent = CheckRunResponse {
            id: 2,
            name: payload.name.to_string(),
            html_url: "https://github.com/illusion-tech/laneflow/runs/2".to_string(),
            head_sha: payload.head_sha.clone(),
            status: payload.status.to_string(),
            conclusion: Some(payload.conclusion.to_string()),
            external_id: Some("evaluation-key:run-1-1".to_string()),
            app: CheckRunApp {
                slug: EXPECTED_CHECK_APP_SLUG.to_string(),
            },
        };

        let reused = select_existing_equivalent_check(vec![equivalent], &payload, "evaluation-key")
            .expect("equivalent check query")
            .expect("equivalent check");
        assert_eq!(reused.id, 2);

        let mismatched = CheckRunResponse {
            id: 3,
            name: payload.name.to_string(),
            html_url: "https://github.com/illusion-tech/laneflow/runs/3".to_string(),
            head_sha: payload.head_sha.clone(),
            status: payload.status.to_string(),
            conclusion: Some("failure".to_string()),
            external_id: Some("evaluation-key:run-3-1".to_string()),
            app: CheckRunApp {
                slug: EXPECTED_CHECK_APP_SLUG.to_string(),
            },
        };
        assert!(
            select_existing_equivalent_check(vec![mismatched], &payload, "evaluation-key").is_err()
        );
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
            "schedule:",
            "workflow_dispatch:",
        ] {
            assert!(gate.contains(trigger), "missing trusted trigger: {trigger}");
        }
        assert!(gate.contains("External Review Signal"));
        assert!(gate.contains(
            "permissions:\n  contents: read\n  pull-requests: read\n  issues: read\n  checks: write"
        ));
        assert!(gate.contains("ref: refs/heads/main"));
        assert!(gate.contains("persist-credentials: false"));
        assert!(gate.contains("actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0"));
        assert!(gate.contains("group: external-review-gate-pr-${{ matrix.pr }}"));
        assert!(gate.contains("cancel-in-progress: true"));
        assert!(gate.contains("publish-external-review-check"));
        assert!(!gate.contains("github.event.pull_request.head.sha"));
        assert!(!gate.contains("github.event.comment.body"));
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
        ];

        for (contents, expected) in cases {
            assert_eq!(evaluate_snapshot(&fixture(contents)).state, expected);
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
        });
        assert_eq!(
            evaluate_snapshot(&snapshot).state,
            ExternalReviewState::Waived
        );
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
}
