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
// D3 轮数上限：产生过受信 findings 的不同 head OID 数超过该值后，
// 只允许精确的 round-cap record 收口（详见 validate_round_cap）。
const MAX_REVIEW_ROUNDS: usize = 3;

const EXTERNAL_REVIEW_QUERY: &str = r#"
query($owner:String!, $name:String!, $number:Int!) {
  repository(owner:$owner, name:$name) {
    pullRequest(number:$number) {
      number
      author { login }
      headRefOid
      baseRefOid
      isDraft
      files(first:100) {
        nodes { path changeType additions deletions }
        pageInfo { hasNextPage }
      }
      commits(last:100) {
        nodes {
          commit {
            oid
            committedDate
            message
            url
            author { name email }
            tree { oid }
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
          commit { oid tree { oid } }
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
                commit { oid tree { oid } }
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

const PULL_REQUEST_FILES_QUERY: &str = r#"
query($owner:String!, $name:String!, $number:Int!, $cursor:String) {
  repository(owner:$owner, name:$name) {
    pullRequest(number:$number) {
      headRefOid
      baseRefOid
      files(first:100, after:$cursor) {
        nodes { path changeType additions deletions }
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
    #[serde(default)]
    round_cap: Option<RoundCapInput>,
    // D4：旧 reviewed head 已不在 PR commits(last:100)（rebase/force-push 后从 PR
    // 历史消失）时，由追加 GraphQL 查询解析出的完整 oid→tree 映射；snapshot CLI
    // 输入可自行携带该字段（缺省为空 → 孤儿 head 不继承，fail-closed）
    #[serde(default)]
    resolved_commit_trees: BTreeMap<String, String>,
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
    // D1：快速通道共享完整性闸门需要精确 additions/deletions；旧快照 / fixture
    // 无此字段（serde default → None），字段缺失时新通道 fail-closed 不成立
    //（dependabot 通道不依赖这两个字段，不受影响）
    #[serde(default)]
    additions: Option<u64>,
    #[serde(default)]
    deletions: Option<u64>,
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
    // D4：commits(last:100) 携带的 tree OID（旧 fixture 无此字段，serde default 兼容）
    #[serde(default)]
    tree: Option<TreeRef>,
    // D1：governance-docs-v1 解析治理字段 `Slice` 所需，且属快速通道共享完整性
    // 闸门；旧快照 / fixture 无此字段（serde default → None），message 缺失时
    // 新通道 fail-closed 不成立
    #[serde(default)]
    message: Option<String>,
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
    #[serde(default)]
    tree: Option<TreeRef>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TreeRef {
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

/// `external-review-round-cap:v1` 的 evaluator 输入（D3）：gate_evidence 侧从
/// current G3 comment 解析构造；evaluator 只做与实测语义的精确匹配校验，
/// 任一不一致即 fail-closed（见 validate_round_cap）。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RoundCapInput {
    pub(crate) current_head_oid: String,
    pub(crate) round_count: usize,
    pub(crate) remaining_finding_urls: Vec<String>,
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
    current_head_tree_oid: Option<String>,
    current_base_oid: String,
    author: String,
    pub state: ExternalReviewState,
    provider: Option<String>,
    actor: Option<String>,
    reviewed_head_oid: Option<String>,
    completion_time: Option<String>,
    finding_count: usize,
    // D2 起语义为 blocking（P0/P1/无 badge）未闭环 thread 计数；P2/P3 deferred 不计入。
    // wire 键保留 v1 的 `unresolvedActionableThreads`（schemaVersion 仍为 1，
    // 既有消费者按旧键识别；语义变更已在 development-gates.md 披露）
    #[serde(rename = "unresolvedActionableThreads")]
    unresolved_blocking_threads: usize,
    deferred_findings: Vec<DeferredFinding>,
    unresolved_blocking_findings: Vec<BlockingFinding>,
    review_rounds: usize,
    round_cap: Option<RoundCapApplied>,
    requires_rereview: bool,
    pending_review_requests: usize,
    evidence: Vec<ReviewEvidence>,
    waiver_id: Option<String>,
    diagnostics: Vec<String>,
}

/// D2 deferred（P2/P3）finding 明细；按 thread id 排序保证序列化确定。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeferredFinding {
    thread_id: String,
    severity: String,
    url: String,
}

/// 未闭环 blocking（P0/P1/无 badge）finding 明细；按 thread id 排序保证序列化确定。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BlockingFinding {
    thread_id: String,
    url: String,
}

/// D3 round-cap 生效记录：轮数与遗留 findings 必须进 check output，不伪装 clean pass。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RoundCapApplied {
    rounds: usize,
    remaining_finding_urls: Vec<String>,
}

impl ExternalReviewResult {
    fn provider_error(repository: &str, pr: u64, diagnostic: String) -> Self {
        Self {
            schema_version: RESULT_SCHEMA_VERSION,
            repository: repository.to_string(),
            pull_request: pr,
            current_head_oid: String::new(),
            current_head_tree_oid: None,
            current_base_oid: String::new(),
            author: String::new(),
            state: ExternalReviewState::ProviderError,
            provider: None,
            actor: None,
            reviewed_head_oid: None,
            completion_time: None,
            finding_count: 0,
            unresolved_blocking_threads: 0,
            deferred_findings: Vec::new(),
            unresolved_blocking_findings: Vec::new(),
            review_rounds: 0,
            round_cap: None,
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

    /// D3 gate_evidence 侧只读投影：实测 review 轮数（产生过受信 findings 的不同 head OID 数）。
    pub(crate) fn review_rounds(&self) -> usize {
        self.review_rounds
    }

    /// D2 gate_evidence 侧只读投影：deferred（P2/P3）findings 的 URL 集合。
    pub(crate) fn deferred_finding_urls(&self) -> BTreeSet<&str> {
        self.deferred_findings
            .iter()
            .map(|finding| finding.url.as_str())
            .collect()
    }

    /// D3 gate_evidence 侧只读投影：round-cap 生效时的轮数与遗留 findings URL 清单；
    /// state 非 Pass 时强制 None（见 evaluate_snapshot 尾部构造）。
    pub(crate) fn round_cap(&self) -> Option<(usize, &[String])> {
        self.round_cap
            .as_ref()
            .map(|applied| (applied.rounds, applied.remaining_finding_urls.as_slice()))
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
    reviewed_head_tree_oid: Option<String>,
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
    evaluate_live_with_optional_waiver(repository, pr, None, true)
}

/// severity_deferral_active=false 用于 G4 replay pre-activation 的 G3 comment
/// （gate_evidence 侧以 disclosure_active 作为开关传入）：停用 D2 deferred 语义，
/// 不 retroactive 升级历史结论（见 evaluate_snapshot_with_policy）。
/// waiver / round-cap 通道不传此开关：前者走无 threads 的极简快照（语义无关），
/// 后者的 record 只在 disclosure 激活后解析（恒为激活路径）。
pub(crate) fn evaluate_live_with_policy(
    repository: &str,
    pr: u64,
    severity_deferral_active: bool,
) -> Result<ExternalReviewResult, String> {
    evaluate_live_with_optional_waiver(repository, pr, None, severity_deferral_active)
}

pub(crate) fn evaluate_live_with_waiver(
    repository: &str,
    pr: u64,
    waiver: WaiverInput,
) -> Result<ExternalReviewResult, String> {
    evaluate_live_with_optional_waiver(repository, pr, Some(waiver), true)
}

/// D3 round-cap record 的 live 注入通道：必须走全量快照（轮数与未闭环 blocking
/// findings 依赖 review threads，不能用 waiver 的极简快照）；gate_evidence 解析出的
/// record 由 evaluator 精确匹配校验，任一不一致即 fail-closed（validate_round_cap）。
pub(crate) fn evaluate_live_with_round_cap(
    repository: &str,
    pr: u64,
    round_cap: RoundCapInput,
) -> Result<ExternalReviewResult, String> {
    let mut snapshot = load_live_snapshot(repository, pr)?;
    snapshot.round_cap = Some(round_cap);
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

fn evaluate_live_with_optional_waiver(
    repository: &str,
    pr: u64,
    waiver: Option<WaiverInput>,
    severity_deferral_active: bool,
) -> Result<ExternalReviewResult, String> {
    let snapshot = match waiver {
        Some(waiver) => load_live_waiver_snapshot(repository, pr, waiver)?,
        None => load_live_snapshot(repository, pr)?,
    };
    let initial_head = snapshot.pull_request.head_ref_oid.clone();
    let initial_base = snapshot.pull_request.base_ref_oid.clone();
    let mut result = evaluate_snapshot_with_policy(&snapshot, severity_deferral_active);
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
    evaluate_snapshot_with_policy(snapshot, true)
}

/// severity_deferral_active=false（G4 replay pre-activation G3 comment）时停用 D1/D2/D3
/// 新语义：不注入 D1 快速通道机器 completion（dependabot 通道不受影响）、不做 badge
/// 分级（受信 finding 一律 blocking）、deferred_findings 恒空、round-cap input 视为
/// 不适用（diagnostics fail-closed），不 retroactive 升级历史结论。
pub(crate) fn evaluate_snapshot_with_policy(
    snapshot: &ExternalReviewSnapshot,
    severity_deferral_active: bool,
) -> ExternalReviewResult {
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
    // D4：commits(last:100) 的 oid→tree 映射；has_next_page（>100 commits）时映射
    // 可能不全，缺失一律不继承（fail-closed，见 commits_tree_for 调用方）
    let mut commits_by_oid = BTreeMap::<&str, &str>::new();
    for node in &pr.commits.nodes {
        if let Some(tree) = node.commit.tree.as_ref() {
            commits_by_oid.insert(node.commit.oid.as_str(), tree.oid.as_str());
        }
    }
    // D4：current head tree 取自同一 snapshot 的 commits(last:100) 映射（与 headRefOid
    // 同源读取，天然消除 force-push race）；head 不在映射内（>100 commits 截断等）
    // → None → 不继承（fail-closed）
    let current_head_tree_oid = commits_by_oid.get(pr.head_ref_oid.as_str()).copied();
    let dependabot_completion = dependabot_lockfile_completion(pr);
    let mut dependabot_completion_event = dependabot_completion
        .map(|completion| (completion.committed_date.as_str(), completion.url.as_str()));
    // D1 快速通道（docs-only-v1 / governance-docs-v1）：与
    // dependabot-cargo-lock-only-v1 同构的机器 completion；evidence 注入走下方
    // 与 dependabot 共享的失效闸门（machine_completion_open）。
    // 激活边界：复用 severity_deferral_active（G1 冻结 `2026-08-20T04:20:39Z`）——
    // pre-activation replay 不注入新通道机器 completion；dependabot 通道不受此闸门。
    // 同一边界对 D1 安全：G1 冻结至本 PR 部署之间唯一的 G3 comment（#453）触及
    // xtask/**，任何通道都不会命中。
    let fast_lane = fast_lane_completion(pr).filter(|_| severity_deferral_active);

    let mut review_to_finding_threads = BTreeMap::<String, BTreeSet<String>>::new();
    let mut finding_thread_ids = BTreeSet::<String>::new();
    let mut thread_severity = BTreeMap::<String, FindingSeverity>::new();
    let mut finding_round_oids = BTreeSet::<String>::new();
    let mut unresolved_blocking_threads = 0;
    let mut unresolved_blocking_findings = Vec::new();
    let mut deferred_findings = Vec::new();
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
        // D2：严重度取自首条被判定为受信 finding 的 comment（循环内首次置
        // has_trusted_finding 的那条）；该 comment 无 badge 即 blocking（fail-closed），
        // 前文不可信 comment 的 badge 文本不采信。deferred 语义未激活时不分级。
        let mut severity: Option<FindingSeverity> = None;
        // D4 回活判定用：首条受信 finding comment 关联 review 的 commit tree
        let mut first_finding_review_tree: Option<&str> = None;
        let mut first_finding_url: Option<&str> = None;
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
            if severity.is_none() {
                severity = Some(if severity_deferral_active {
                    finding_comment_severity(&comment.body)
                } else {
                    FindingSeverity::Blocking
                });
                first_finding_review_tree = review
                    .commit
                    .as_ref()
                    .and_then(|commit| commit.tree.as_ref())
                    .map(|tree| tree.oid.as_str());
            }
            has_trusted_finding = true;
            finding_thread_ids.insert(thread.id.clone());
            review_to_finding_threads
                .entry(review.id.clone())
                .or_default()
                .insert(thread.id.clone());
            // D3 轮数口径：产生过受信 findings 的不同 head OID（commit 缺失时跳过，
            // 该 thread 的其它歧义已由上面的 fail-closed 诊断覆盖）
            if let Some(commit) = review.commit.as_ref() {
                finding_round_oids.insert(commit.oid.clone());
            }
            if first_finding_url.is_none() {
                first_finding_url = Some(comment.url.as_str());
            }
        }
        if has_trusted_finding {
            thread_severity.insert(
                thread.id.clone(),
                severity.unwrap_or(FindingSeverity::Blocking),
            );
        }
        // D4 对称继承：content-equivalent force-push 会把旧 thread 标 isOutdated；
        // 其首条受信 finding 关联 review 的 commit tree 与 current head tree 逐字节
        // 相等时回活（按未 outdated 处理，未闭环 P1 同样继承，见 G1 对称继承契约）；
        // tree 缺失/不等保持丢弃（fail-closed）。isResolved 语义不变（已处置不回活）。
        let revived_by_tree =
            thread.is_outdated && tree_oids_equal(first_finding_review_tree, current_head_tree_oid);
        if !thread.is_resolved && (!thread.is_outdated || revived_by_tree) {
            if has_trusted_finding {
                match severity.unwrap_or(FindingSeverity::Blocking) {
                    FindingSeverity::Blocking => {
                        unresolved_blocking_threads += 1;
                        unresolved_blocking_findings.push(BlockingFinding {
                            thread_id: thread.id.clone(),
                            url: first_finding_url
                                .unwrap_or(first_comment.url.as_str())
                                .to_string(),
                        });
                    }
                    FindingSeverity::Deferred(digit) => {
                        deferred_findings.push(DeferredFinding {
                            thread_id: thread.id.clone(),
                            severity: format!("P{digit}"),
                            url: first_finding_url
                                .unwrap_or(first_comment.url.as_str())
                                .to_string(),
                        });
                    }
                }
            } else if (dependabot_completion.is_some() || fast_lane.is_some())
                && has_authorless_comment
            {
                // Dependabot 已知误报之外的 authorless thread 没有 badge 语义，保持
                // blocking；新通道与 dependabot 一致：authorless 未结 thread 按
                // blocking 处理（fail-closed，deleted/unavailable reviewer 身份不可
                // 核验）。两侧都用结构判定的原始值（不过闸门）——目的正是让这类
                // thread 关闭 machine_completion_open 闸门
                unresolved_blocking_threads += 1;
                unresolved_blocking_findings.push(BlockingFinding {
                    thread_id: thread.id.clone(),
                    url: first_comment.url.clone(),
                });
            }
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
    // D2：以 evidence URL 为键记录每条 finding completion 是否仍具 blocking 压力
    //（unthreaded 或任一关联 thread 为 blocking 严重度即为 true；缺失时按 true 处理）
    let mut finding_has_blocking = BTreeMap::<String, bool>::new();
    // D2/D3：unthreaded（review 级，无关联 finding thread）finding 的 evidence URL
    // 集合，供后续补入未闭环 blocking findings（见 validate_round_cap 上游）
    let mut unthreaded_finding_urls = BTreeSet::<String>::new();
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
        if outcome == EvidenceOutcome::Findings {
            // D3 轮数口径：受信 finding review 的 head 一律计轮；thread 挂载的
            // finding 已由 thread 循环按 review reference commit 计入（集合去重），
            // 这里补齐 unthreaded（review 级）finding 的 head
            finding_round_oids.insert(reviewed_head.to_string());
            if linked_findings == 0 {
                unthreaded_finding_urls.insert(url.to_string());
            }
            let has_blocking = review_to_finding_threads
                .get(&review.id)
                .is_none_or(|threads| {
                    threads.is_empty()
                        || threads.iter().any(|thread_id| {
                            thread_severity
                                .get(thread_id)
                                .copied()
                                .unwrap_or(FindingSeverity::Blocking)
                                == FindingSeverity::Blocking
                        })
                });
            finding_has_blocking.insert(url.to_string(), has_blocking);
        }
        push_evidence(
            &mut evidence,
            &mut diagnostics,
            EvidenceInput {
                provider,
                actor: &actor_login,
                source_kind: "review",
                reviewed_head,
                reviewed_head_tree: review
                    .commit
                    .as_ref()
                    .and_then(|commit| commit.tree.as_ref())
                    .map(|tree| tree.oid.as_str()),
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
    // D1 共享失效闸门：evaluate 时已存在任何受信 finding（含 P2/P3 deferred，
    // 即 findingCount 层面而非仅 blocking）、unresolved blocking thread 或
    // stale/dismissed 活动 → 全部机器 completion 通道失效，回标准路径
    //（此后 deferred/round-cap 等既有语义照常）。四个输入由线程循环与 reviews
    // 循环算出，comments 循环不修改它们，故闸门前置到此处供故障例外复用。
    let machine_completion_open = finding_thread_ids.is_empty()
        && unthreaded_findings == 0
        && unresolved_blocking_threads == 0
        && !stale_or_dismissed;
    for comment in &pr.comments.nodes {
        let Some(actor) = comment.author.as_ref() else {
            continue;
        };
        if normalize_actor(&actor.login) != CODEX_ACTOR {
            continue;
        }
        // Codex 故障注释（环境不可用）的 provider-error 诊断例外覆盖两条机器
        // completion 通道，但两侧不对称：dependabot 侧保持既有口径（未过闸门的
        // 原始判定，legacy 行为不动）；D1 快速通道侧只在机器 completion 实际
        // 可用（过闸门后）时生效——闸门已关时不吞故障歧义信号
        if comment.body.contains("To use Codex here")
            && dependabot_completion.is_none()
            && fast_lane.filter(|_| machine_completion_open).is_none()
        {
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
                            reviewed_head_tree: None,
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
                reviewed_head_tree: None,
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
        dependabot_completion.filter(|_| machine_completion_open),
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
                reviewed_head_tree: None,
                reviewed_base: &pr.base_ref_oid,
                outcome: EvidenceOutcome::Clean,
                submitted_at: completion_time,
                evidence_url: completion_url,
            },
        );
    }

    if let Some((lane, commit)) = fast_lane.filter(|_| machine_completion_open) {
        push_evidence(
            &mut evidence,
            &mut diagnostics,
            EvidenceInput {
                provider: lane,
                actor: "github-metadata",
                source_kind: "machine_verification",
                reviewed_head: &commit.oid,
                reviewed_head_tree: None,
                reviewed_base: &pr.base_ref_oid,
                outcome: EvidenceOutcome::Clean,
                submitted_at: &commit.committed_date,
                evidence_url: &commit.url,
            },
        );
    }

    // D4：evidence 缺 tree 时用其 reviewed head OID 查 commits 映射补齐（Codex clean
    // issue comment 等无 tree 来源的主路径由此获得 tree；commits 映射查不到时再查
    // 孤儿 head 追加解析结果）；查不到或短 SHA 前缀歧义一律不补（fail-closed，不继承）
    for item in &mut evidence {
        if item.reviewed_head_tree_oid.is_none() {
            item.reviewed_head_tree_oid = commits_tree_for(
                &commits_by_oid,
                &snapshot.resolved_commit_trees,
                &item.reviewed_head_oid,
            )
            .map(str::to_string);
        }
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
    let review_rounds = finding_round_oids.len();

    let current_evidence = evidence
        .iter()
        .filter(|item| {
            evidence_matches_current(
                &item.reviewed_head_oid,
                item.reviewed_head_tree_oid.as_deref(),
                &pr.head_ref_oid,
                current_head_tree_oid,
            )
        })
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

    // D2/D3：current-head（含 tree 等价）unthreaded finding 证据未被严格更晚的
    // current-head clean supersede 时，其 evidence URL 计入未闭环 blocking findings
    //（thread 级 finding 已在 thread 循环计入；此处补 review 级，与 thread URL 去重）。
    // unthreaded finding 没有 badge 语义，一律 blocking。
    for item in current_evidence.iter().copied().filter(|item| {
        item.outcome == EvidenceOutcome::Findings
            && unthreaded_finding_urls.contains(item.evidence_url.as_str())
    }) {
        let superseded = latest_clean.is_some_and(|clean| clean.submitted_at > item.submitted_at);
        if superseded
            || unresolved_blocking_findings
                .iter()
                .any(|finding| finding.url == item.evidence_url)
        {
            continue;
        }
        // 无 thread 可关联，thread_id 复用 evidence URL（仅用于排序与序列化披露）
        unresolved_blocking_findings.push(BlockingFinding {
            thread_id: item.evidence_url.clone(),
            url: item.evidence_url.clone(),
        });
    }

    if let Some(round_cap) = snapshot.round_cap.as_ref() {
        if severity_deferral_active {
            validate_round_cap(
                round_cap,
                pr,
                review_rounds,
                &unresolved_blocking_findings,
                &mut diagnostics,
            );
        } else {
            // D3 激活边界：deferred 语义未激活的 replay 中 round-cap record 不适用，
            // 一律 fail-closed（record 只能由 post-activation G3 comment 解析构造）
            diagnostics
                .push("round-cap record 在 deferred 语义激活前的 replay 中不适用".to_string());
        }
    }
    // 明细集合按 thread id 排序，保证 result 序列化（fingerprint）确定
    deferred_findings.sort_by(|left, right| left.thread_id.cmp(&right.thread_id));
    unresolved_blocking_findings.sort_by(|left, right| left.thread_id.cmp(&right.thread_id));

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

    let clean_after_finding = latest_finding
        .and_then(|finding| latest_clean.filter(|clean| clean.submitted_at > finding.submitted_at));
    // D2：current-head finding 是否仍具 blocking 压力；无记录时 fail-closed 按有处理
    let finding_blocks = |finding: &ReviewEvidence| {
        finding_has_blocking
            .get(finding.evidence_url.as_str())
            .copied()
            .unwrap_or(true)
    };
    // E（r3820473548）：独立跟踪最新 blocking finding（blocking 压力或 unthreaded）。
    // 防止最新 findings review 仅 deferred（P2/P3）时掩盖较早 blocking finding
    // 「处置后需其后的 current-head clean re-review」的要求。
    let latest_blocking_finding = current_evidence
        .iter()
        .rev()
        .find(|item| {
            item.outcome == EvidenceOutcome::Findings
                && (finding_blocks(item)
                    || unthreaded_finding_urls.contains(item.evidence_url.as_str()))
        })
        .copied();
    // D3：round-cap record 只在「本将落 FindingsOpen / AwaitingRereview」时生效；
    // 其它状态不受影响（record 校验失败已在上游 diagnostics fail-closed）。
    let would_be_findings_open =
        unresolved_blocking_threads > 0 && (latest_finding.is_some() || latest_clean.is_some());
    let would_be_awaiting_rereview = latest_finding.is_some_and(|finding| {
        if unresolved_blocking_threads > 0 || clean_after_finding.is_some() {
            return false;
        }
        // F1：与下方状态分支口径一致——blocking finding 仍在，或 deferred-only
        // 最新 review 背后存在未被更晚 clean 覆盖的更早 blocking finding（该形态下
        // 未闭环 blocking 集合为空，record 的空 remainingFindingUrls 语义自洽）。
        finding_blocks(finding)
            || !latest_blocking_finding.is_none_or(|blocking| {
                latest_clean.is_some_and(|clean| clean.submitted_at > blocking.submitted_at)
            })
    });
    let round_cap_applied = snapshot
        .round_cap
        .as_ref()
        .filter(|_| would_be_findings_open || would_be_awaiting_rereview)
        .map(|input| {
            let mut remaining_finding_urls = input.remaining_finding_urls.clone();
            remaining_finding_urls.sort();
            RoundCapApplied {
                rounds: input.round_count,
                remaining_finding_urls,
            }
        });

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
    } else if let Some(round_cap) = round_cap_applied.as_ref() {
        // primary 取 latest_finding 与 latest_clean 中时间最新者（G3：unresolved
        // blocking + 更晚 clean 形态下，Owner 收口决策不得早于最新 review 活动）；
        // round-cap 生效前提已保证二者至少其一存在（would_be_* 判定），
        // gate_evidence 对非 Waived pass 强制要求 completion time
        //（round-cap pass 不是 clean pass，但仍以最新证据计时）
        let primary = match (latest_finding, latest_clean) {
            (Some(finding), Some(clean)) if clean.submitted_at > finding.submitted_at => {
                Some(clean)
            }
            (Some(finding), _) => Some(finding),
            (None, clean) => clean,
        };
        (
            ExternalReviewState::Pass,
            false,
            primary,
            Some(format!(
                "review 轮数 {} 超过上限 {MAX_REVIEW_ROUNDS}，round-cap record 生效收口；遗留 {} 条未闭环 blocking findings，不得伪装为 clean pass",
                round_cap.rounds,
                round_cap.remaining_finding_urls.len()
            )),
        )
    } else if let Some(finding) = latest_finding {
        if unresolved_blocking_threads > 0 {
            (
                ExternalReviewState::FindingsOpen,
                true,
                Some(finding),
                Some("current-head finding 仍有 unresolved blocking thread".to_string()),
            )
        } else if let Some(clean) = clean_after_finding {
            (ExternalReviewState::Pass, false, Some(clean), None)
        } else if !finding_blocks(finding) {
            // D2：仅剩 deferred（P2/P3）findings 时不阻断，明细在 check output 披露。
            // E：前提是不存在未被覆盖的更早 blocking finding——存在时必须有严格
            // 晚于其 submitted_at 的 current-head clean，否则落 AwaitingRereview。
            let blocking_recovered = latest_blocking_finding.is_none_or(|blocking| {
                latest_clean.is_some_and(|clean| clean.submitted_at > blocking.submitted_at)
            });
            if blocking_recovered {
                (
                    ExternalReviewState::Pass,
                    false,
                    Some(finding),
                    Some("current-head 仅剩 deferred findings（P2/P3），不阻断合并".to_string()),
                )
            } else {
                (
                    ExternalReviewState::AwaitingRereview,
                    true,
                    latest_blocking_finding,
                    Some(
                        "blocking finding 处置后缺少其后的 exact-head clean re-review".to_string(),
                    ),
                )
            }
        } else {
            (
                ExternalReviewState::AwaitingRereview,
                true,
                Some(finding),
                Some("finding 已处置，但缺少其后的 exact-head clean re-review".to_string()),
            )
        }
    } else if let Some(clean) = latest_clean {
        if unresolved_blocking_threads > 0 {
            (
                ExternalReviewState::FindingsOpen,
                true,
                Some(clean),
                Some("存在 unresolved blocking thread，clean completion 不足以放行".to_string()),
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
        current_head_tree_oid: current_head_tree_oid.map(str::to_string),
        current_base_oid: pr.base_ref_oid.clone(),
        author,
        state,
        provider: primary.map(|item| item.provider.clone()),
        actor: primary.map(|item| item.actor.clone()),
        reviewed_head_oid: primary.map(|item| item.reviewed_head_oid.clone()),
        completion_time: primary.map(|item| item.submitted_at.clone()),
        finding_count,
        unresolved_blocking_threads,
        deferred_findings,
        unresolved_blocking_findings,
        review_rounds,
        // round-cap 只在实际生效（结论 Pass）时记录；校验失败/其它状态一律 None
        round_cap: if state == ExternalReviewState::Pass {
            round_cap_applied
        } else {
            None
        },
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
    }

    // 与 workflow concurrency 组配对：串行化保证同一 PR 任意时刻只有一个
    // publisher run（消除并发读-选-发竞态）；同组只允许一个 pending run，
    // 被挤掉的中间 issue_comment 事件由此处 sweep 补齐。所有未绑定 clean
    // （含触发 comment）统一按 created_at 时间序处理：旧 head clean 事件被
    // 挤掉后若先绑新 clean，新 clean 会同时看到新旧 head 的未消费 marker
    // 而歧义 fail-closed；时间序先为旧 clean 消费旧 marker，新 clean 才能
    // 确定性绑定新 marker。marker 缺失/歧义的 clean 记诊断跳过，evaluator
    // 会独立对其 fail-closed 诊断并由 shadow 刷新发布。
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
    reviewed_head_tree: Option<&'a str>,
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
        reviewed_head_tree_oid: input.reviewed_head_tree.map(str::to_ascii_lowercase),
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

/// 与 evaluate_snapshot 的 comment 闸门一致：只有未编辑、Codex 发表、匹配 clean
/// verdict 子串形状、无 Reviewed commit marker 且 createdAt/URL 有效的 comment
/// 才进入 record 绑定判定（publisher sweep 复用本闸门，不会为字段无效的 clean
/// 发布 evaluator 必拒的 malformed record）。
fn sha_less_bindable_clean(comment: &IssueComment) -> bool {
    comment
        .author
        .as_ref()
        .is_some_and(|actor| normalize_actor(&actor.login) == CODEX_ACTOR)
        && !comment.body.contains("To use Codex here")
        && codex_clean_comment_shape(&comment.body)
        && comment.updated_at == comment.created_at
        && parse_reviewed_commit(&comment.body).is_none()
        && valid_timestamp(&comment.created_at)
        && valid_github_url(&comment.url)
}

/// 按 record 创建时间升序逐条判定无 SHA clean 的绑定，返回 clean comment id →
/// 判定结果（None 表示对应 record 不合法）。同秒发布的 record 以引用 clean 的
/// (created_at, id) 恢复 publisher sweep 的真实分配顺序，hash 派生的 record id
/// 仅作最终稳定 tie-break。stale record 同样消费其 marker；未被任何 record
/// 成功绑定的 clean 与未被消费的 record 由调用方分别诊断。
fn adjudicate_codex_clean_bindings<'a>(
    pr: &'a PullRequestSnapshot,
    records: &'a [BoundCleanRecord],
    diagnostics: &mut Vec<String>,
) -> BTreeMap<&'a str, Option<&'a BoundCleanRecord>> {
    let mut bindings = BTreeMap::new();
    let mut consumed_markers = BTreeSet::<&str>::new();
    let mut ordered: Vec<&BoundCleanRecord> = records.iter().collect();
    let clean_order_key = |bound: &BoundCleanRecord| {
        pr.comments
            .nodes
            .iter()
            .find(|comment| comment.id == bound.record.clean_comment_id)
            .map(|comment| (comment.created_at.as_str(), comment.id.as_str()))
            .unwrap_or(("", ""))
    };
    ordered.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| clean_order_key(left).cmp(&clean_order_key(right)))
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

/// D1 快速通道（issue #406 PR-B）：`docs-only-v1` / `governance-docs-v1` 的
/// 机器 completion 判定，与 dependabot-cargo-lock-only-v1 同构——精确机器条件 +
/// evaluate 级联中的共享 evidence 注入闸门（任何受信 finding 抵达即失效，见
/// evaluate_snapshot 的 machine_completion_open）。多通道同时命中时按固定顺序
/// 取首个命名通道（证据等价，通道名只作 provider 标注）。
///
/// `pure-move-v1` 暂缓启用：GraphQL `PullRequestChangedFile` 不提供 rename 源
/// 路径，destination-only 校验无法防止语义路径文件的 0/0 改名逃逸（如
/// `.github/dependabot.yml` 改名为 `docs/` 下文件关停 Dependabot），见 #406
/// 审阅记录（PR #464）。
fn fast_lane_completion(pr: &PullRequestSnapshot) -> Option<(&'static str, &CommitMetadata)> {
    let files = fully_paginated_files(pr)?;
    // 共享前置校验一：RENAMED 排除——所有通道统一拒绝改名文件（见上）
    if files.iter().any(|file| file.change_type == "RENAMED") {
        return None;
    }
    // 共享前置校验二：snapshot 完整性——任一文件的 additions/deletions 或 head
    // commit 的 message 缺失（旧快照形态）即失效，回标准路径
    if files
        .iter()
        .any(|file| file.additions.is_none() || file.deletions.is_none())
    {
        return None;
    }
    let commit = head_commit_for_fast_lane(pr)?;
    commit.message.as_ref()?;
    docs_only_lane_completion(files, commit)
        .map(|commit| ("docs-only-v1", commit))
        .or_else(|| {
            governance_docs_lane_completion(files, commit)
                .map(|commit| ("governance-docs-v1", commit))
        })
}

/// 完整分页的非空 files 视图：分页溢出（snapshot 截断）或空 diff → None
///（全部快速通道失效，回标准路径，不接受人工修补）
fn fully_paginated_files(pr: &PullRequestSnapshot) -> Option<&[ChangedFile]> {
    if pr.files.page_info.has_next_page || pr.files.nodes.is_empty() {
        return None;
    }
    Some(pr.files.nodes.as_slice())
}

/// head commit metadata：快速通道机器 completion 的证据来源（reviewed head、
/// completion time、evidence URL）；head 不在 commits 连接内或证据字段无效
/// → None（fail-closed）
fn head_commit_for_fast_lane(pr: &PullRequestSnapshot) -> Option<&CommitMetadata> {
    let commit = &pr
        .commits
        .nodes
        .iter()
        .find(|node| node.commit.oid == pr.head_ref_oid)?
        .commit;
    (valid_full_oid(&commit.oid)
        && valid_timestamp(&commit.committed_date)
        && valid_github_url(&commit.url))
    .then_some(commit)
}

/// `docs-only-v1`：全部变更文件匹配 `docs/**/*.md`、根级 `*.md`
/// 或 `research/**/*.md`（共享前置校验已完成 RENAMED 排除与 snapshot 完整性检查）
fn docs_only_lane_completion<'a>(
    files: &[ChangedFile],
    commit: &'a CommitMetadata,
) -> Option<&'a CommitMetadata> {
    if !files.iter().all(|file| is_docs_only_path(&file.path)) {
        return None;
    }
    Some(commit)
}

fn is_docs_only_path(path: &str) -> bool {
    // AGENTS.md 是 agent 工作流 SSOT 入口（机器消费指令面，非惰性文档），
    // 与「门禁代码面不豁免」同源：精确排除（仅根级等值；.agents/、docs/ 下
    // 同名文件不受影响——前者本就不命中，后者由 docs/**/*.md 分支管辖）
    if path == "AGENTS.md" {
        return false;
    }
    (path.starts_with("docs/") && path.ends_with(".md"))
        || is_root_markdown(path)
        || (path.starts_with("research/") && path.ends_with(".md"))
}

fn is_root_markdown(path: &str) -> bool {
    !path.contains('/') && path.ends_with(".md")
}

/// `governance-docs-v1`：head commit 治理字段 `Slice: governance`（只认 head commit
/// 本身；message 缺失 / 无 Slice / 值非 governance 即不成立），且全部变更文件匹配
/// `docs/**/*.md`、根级 `*.md`、`.agents/**/*.md` 或 `.github/**/*.md`；
/// `.github/workflows/**`、`xtask/**`、`schemas/**`、`crates/**` 任一命中即不成立
///（门禁代码面与运行时代码不豁免）。
fn governance_docs_lane_completion<'a>(
    files: &[ChangedFile],
    commit: &'a CommitMetadata,
) -> Option<&'a CommitMetadata> {
    if head_commit_slice_value(commit.message.as_deref()?) != Some("governance") {
        return None;
    }
    if !files.iter().all(|file| is_governance_docs_path(&file.path)) {
        return None;
    }
    Some(commit)
}

fn is_governance_docs_path(path: &str) -> bool {
    if path.starts_with(".github/workflows/")
        || path.starts_with("xtask/")
        || path.starts_with("schemas/")
        || path.starts_with("crates/")
    {
        return false;
    }
    (path.starts_with("docs/") && path.ends_with(".md"))
        || is_root_markdown(path)
        || (path.starts_with(".agents/") && path.ends_with(".md"))
        || (path.starts_with(".github/") && path.ends_with(".md"))
}

/// head commit message 的治理字段 `Slice` 值（commit-convention.md §2 严格格式：
/// 字段名 + 冒号 + 一个空格）。无 Slice 行、空值或多行歧义 → None（fail-closed）。
fn head_commit_slice_value(message: &str) -> Option<&str> {
    let mut values = message
        .lines()
        .filter_map(|line| line.strip_prefix("Slice: "));
    let value = values.next()?.trim_end();
    if value.is_empty() || values.next().is_some() {
        return None;
    }
    Some(value)
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

/// D3 round-cap record 校验：任一字段与 evaluator 实测语义不一致即 fail-closed
///（diagnostics 非空 → ProviderError，与无效 waiver 的失败行为一致）。
fn validate_round_cap(
    round_cap: &RoundCapInput,
    pr: &PullRequestSnapshot,
    review_rounds: usize,
    unresolved_blocking_findings: &[BlockingFinding],
    diagnostics: &mut Vec<String>,
) {
    if round_cap.current_head_oid != pr.head_ref_oid {
        diagnostics.push("round-cap currentHeadOid 与 PR current head 不一致".to_string());
    }
    if round_cap.round_count != review_rounds {
        diagnostics.push(format!(
            "round-cap roundCount 与 evaluator 实测轮数不一致：record={} evaluator={review_rounds}",
            round_cap.round_count
        ));
    }
    if round_cap.round_count <= MAX_REVIEW_ROUNDS {
        diagnostics.push(format!(
            "round-cap record 仅在 review 轮数超过 {MAX_REVIEW_ROUNDS} 时适用，record 声明 {}",
            round_cap.round_count
        ));
    }
    let expected = unresolved_blocking_findings
        .iter()
        .map(|finding| finding.url.as_str())
        .collect::<BTreeSet<_>>();
    let actual = round_cap
        .remaining_finding_urls
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if actual != expected {
        diagnostics.push(
            "round-cap remainingFindingUrls 与 evaluator 实测未闭环 blocking findings 不一致"
                .to_string(),
        );
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

/// D2 严重度分级：P0/P1 与任何无法识别的输入一律 blocking（fail-closed）；
/// 仅 P2/P3 转 deferred，不阻断合并但必须在 check output 披露。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FindingSeverity {
    Blocking,
    Deferred(u8),
}

/// 解析 Codex 风格严重度 badge `![P<digit> Badge]`；多 badge 取首个。
/// 无 badge / 解析失败返回 None，由调用方按 blocking 处理。
fn parse_severity_badge(body: &str) -> Option<u8> {
    let start = body.find("![P")?;
    let tail = body.get(start + "![P".len()..)?;
    let digit = tail.chars().next()?.to_digit(10)? as u8;
    tail.get(1..)?.starts_with(" Badge]").then_some(digit)
}

/// 受信 finding comment 的严重度：P0/P1、无 badge、解析失败一律 blocking
///（fail-closed）；仅 P2/P3 转 deferred。trust 判定在调用方（comment actor 必须
/// 已通过受信 provider 与 review 一致性校验）；不可信来源的 badge 文本不采信，
/// Copilot 与人工 reviewer 不发 badge，自然保持 blocking。
fn finding_comment_severity(body: &str) -> FindingSeverity {
    match parse_severity_badge(body) {
        Some(digit @ (2 | 3)) => FindingSeverity::Deferred(digit),
        _ => FindingSeverity::Blocking,
    }
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

/// D4 内容等价继承：head OID 前缀匹配，或双方 tree OID 均为完整 OID 且逐字节
/// 相等（同 patchset 同审阅状态，clean 与未闭环 findings 对称继承）；
/// tree 任一缺失/非法时回退纯 OID 判定（旧行为）。
fn evidence_matches_current(
    reviewed_oid: &str,
    reviewed_tree_oid: Option<&str>,
    current_oid: &str,
    current_tree_oid: Option<&str>,
) -> bool {
    oid_matches_current(reviewed_oid, current_oid)
        || tree_oids_equal(reviewed_tree_oid, current_tree_oid)
}

fn tree_oids_equal(reviewed_tree_oid: Option<&str>, current_tree_oid: Option<&str>) -> bool {
    match (reviewed_tree_oid, current_tree_oid) {
        (Some(reviewed), Some(current)) => {
            valid_full_oid(reviewed) && valid_full_oid(current) && reviewed == current
        }
        _ => false,
    }
}

/// D4：查 reviewed head 的 tree OID：先查 commits(last:100) 映射，再查孤儿 head
/// 追加解析结果（resolved_commit_trees）；完整 OID 直接命中，短 SHA 前缀须唯一
/// 匹配（多命中歧义不补，fail-closed 不继承）
fn commits_tree_for<'a>(
    commits_by_oid: &BTreeMap<&str, &'a str>,
    resolved_commit_trees: &'a BTreeMap<String, String>,
    reviewed_head: &str,
) -> Option<&'a str> {
    let direct = commits_by_oid
        .get(reviewed_head)
        .copied()
        .or_else(|| resolved_commit_trees.get(reviewed_head).map(String::as_str));
    if direct.is_some() {
        return direct;
    }
    let mut matches = commits_by_oid
        .iter()
        .map(|(oid, tree)| (*oid, *tree))
        .chain(
            resolved_commit_trees
                .iter()
                .map(|(oid, tree)| (oid.as_str(), tree.as_str())),
        )
        .filter(|(oid, _)| oid_matches_current(reviewed_head, oid));
    match (matches.next(), matches.next()) {
        (Some((_, tree)), None) => Some(tree),
        _ => None,
    }
}

/// D4：收集需要追加查询解析 tree 的孤儿 reviewed head oid——PR 顶层 comments 中
/// 能成为受信 Codex clean 证据的 comment 的 `Reviewed commit:` marker 值（F2：
/// 受信 Codex actor + clean comment 形态 + 未被编辑；伪造 marker 不参与收集、
/// 不占用 16-OID 上限、不牵引追加查询），以及经校验的 Codex clean binding
/// record 的 boundHeadOid（F：SHA-less clean completion 的 head 只存在于该
/// 隐藏记录）；去重后仍无法由 commits 映射与已解析结果解析的部分；超过 16 个
/// 不同 oid → None（fail-closed 不解析，全部不继承）
fn orphan_reviewed_oids_for_tree_resolution(
    pr: &PullRequestSnapshot,
    commits_by_oid: &BTreeMap<&str, &str>,
    resolved_commit_trees: &BTreeMap<String, String>,
) -> Option<Vec<String>> {
    let mut oids = BTreeSet::new();
    for comment in &pr.comments.nodes {
        // F2：与 evaluate 的 clean comment 证据入口同一受信口径
        let trusted_clean = comment
            .author
            .as_ref()
            .is_some_and(|actor| normalize_actor(&actor.login) == CODEX_ACTOR)
            && codex_clean_comment_shape(&comment.body)
            && comment.updated_at == comment.created_at;
        if !trusted_clean {
            continue;
        }
        let Some(reviewed) = parse_reviewed_commit(&comment.body) else {
            continue;
        };
        if commits_tree_for(commits_by_oid, resolved_commit_trees, reviewed).is_some() {
            continue;
        }
        oids.insert(reviewed.to_string());
    }
    // F（r3820473559）：复用 binding record 的解析/校验；校验失败的 record 一律
    // 不收集（fail-closed，其 diagnostics 由 evaluate 阶段复核报告）
    let mut record_diagnostics = Vec::new();
    for bound in collect_codex_clean_binding_records(pr, &mut record_diagnostics) {
        let bound_head = &bound.record.bound_head_oid;
        if commits_tree_for(commits_by_oid, resolved_commit_trees, bound_head).is_some() {
            continue;
        }
        oids.insert(bound_head.clone());
    }
    (oids.len() <= 16).then(|| oids.into_iter().collect())
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
        "External Review Gate: {:?}\nPR: {}/pull/{}\nCurrent head/base: {}/{}\nProvider/actor: {}/{}\nReviewed head/completion: {}/{}\nFindings/unresolved-blocking/deferred/rounds/re-review: {}/{}/{}/{}/{}\nEvidence count: {}\nDiagnostics: {}",
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
        result.unresolved_blocking_threads,
        result.deferred_findings.len(),
        result.review_rounds,
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
    // `unresolved=` 键名保留（wire 稳定）；D2 起其值语义为 blocking 计数，
    // deferred（P2/P3）与轮数由独立键披露。
    let summary = format!(
        "state=`{}`; head=`{}`; provider=`{provider}`; actor=`{actor}`; findings={}; unresolved={}; deferred={}; rounds={}; re-review={}; diagnostics={}",
        result.state.as_str(),
        result.current_head_oid,
        result.finding_count,
        result.unresolved_blocking_threads,
        result.deferred_findings.len(),
        result.review_rounds,
        result.requires_rereview,
        result.diagnostics.len()
    );

    let evidence_limit = 20;
    let evidence_labels = result
        .evidence
        .iter()
        .take(evidence_limit)
        .enumerate()
        .map(|(index, evidence)| {
            // D4：tree 等价继承命中的 evidence 在可见行显式标注，保证可审计；
            // 标注不得进 reference 定义行（会被吞进 Markdown 链接目标）
            let inheritance = if evidence_inherits_by_tree(evidence, result) {
                "（tree-equivalent 继承）"
            } else {
                ""
            };
            format!("[evidence-{}]{inheritance}", index + 1)
        })
        .collect::<Vec<_>>();
    let mut text = format!(
        "- Repository / PR：`{}` / `#{}`\n- Current head / base：`{}` / `{}`\n- Author：`{}`\n- State：`{}`\n- Provider / actor：`{provider}` / `{actor}`\n- Reviewed head / completion：`{reviewed_head}` / `{completion}`\n- Findings / unresolved blocking threads / requires re-review：`{}` / `{}` / `{}`\n- Deferred findings / review rounds：`{}` / `{}`\n- Pending review requests：`{}`\n- Waiver：`{waiver}`\n- Evidence：{}\n- Diagnostics：`{}`（详情见 workflow run）",
        single_line(&result.repository),
        result.pull_request,
        result.current_head_oid,
        result.current_base_oid,
        single_line(&result.author),
        result.state.as_str(),
        result.finding_count,
        result.unresolved_blocking_threads,
        result.requires_rereview,
        result.deferred_findings.len(),
        result.review_rounds,
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
            // reference 定义行必须保持纯 URL；继承标注在可见 evidence 行尾部
            text.push_str(&format!(
                "[evidence-{}]: {}\n",
                index + 1,
                evidence.evidence_url
            ));
        }
    }
    if !result.deferred_findings.is_empty() {
        text.push_str("\nDeferred findings（P2/P3，不阻断合并）：");
        for finding in &result.deferred_findings {
            text.push_str(&format!("\n- {} {}", finding.severity, finding.url));
        }
    }
    if let Some(round_cap) = &result.round_cap {
        text.push_str(&format!(
            "\n\nReview round cap：review 轮数 `{}` 超过上限 `{MAX_REVIEW_ROUNDS}`，经 round-cap record 收口（非 clean pass）；遗留未闭环 blocking findings：",
            round_cap.rounds
        ));
        if round_cap.remaining_finding_urls.is_empty() {
            text.push_str("\n- （无未闭环 blocking thread；finding 仍待 clean re-review 闭环）");
        }
        for url in &round_cap.remaining_finding_urls {
            text.push_str(&format!("\n- {url}"));
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

/// D4：evidence 的 reviewed head OID 不是 current head、但 tree OID 与 current
/// head 逐字节相等时，该 evidence 经 tree 等价继承生效，输出必须标注。
fn evidence_inherits_by_tree(evidence: &ReviewEvidence, result: &ExternalReviewResult) -> bool {
    !oid_matches_current(&evidence.reviewed_head_oid, &result.current_head_oid)
        && tree_oids_equal(
            evidence.reviewed_head_tree_oid.as_deref(),
            result.current_head_tree_oid.as_deref(),
        )
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
    if pull_request.files.page_info.has_next_page {
        // D1 快速通道需要完整 files 视图：截断时补齐全量分页。分页无法完成时
        // 不传播错误——files 只服务快速通道判定，标准路径不依赖它；保留原始截断
        // 连接（has_next_page 保持 true），fully_paginated_files 返回 None，全部
        // 机器通道（含 dependabot）失效，评估继续走标准路径（G1：分页溢出 →
        // 通道失效并回标准路径）。这与 comments 不同：不完整评论会破坏绑定判定
        // 完整性，必须整体 fail-closed。
        adopt_files_refetch(
            &mut pull_request.files,
            &pull_request.head_ref_oid,
            &pull_request.base_ref_oid,
            fetch_all_pr_files(repository, pr),
        );
    }
    let mut snapshot = ExternalReviewSnapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        repository: repository.to_string(),
        pull_request,
        provider_errors: Vec::new(),
        waiver: None,
        round_cap: None,
        resolved_commit_trees: BTreeMap::new(),
    };
    resolve_orphan_commit_trees(repository, &mut snapshot);
    Ok(snapshot)
}

/// D1：files 补页绑定 current head 与 base——补页返回的 headRefOid/baseRefOid 与
/// snapshot 一致才采用补页结果；任一不一致（补读期间 force-push / base 重定向，
/// 如 A→B→A 拼接）或补页失败时保留截断连接（has_next_page 保持 true），全部
/// 机器通道失效回标准路径（与分页溢出/失败同语义，不传播 Err）
fn adopt_files_refetch(
    files: &mut Connection<ChangedFile>,
    snapshot_head: &str,
    snapshot_base: &str,
    refetch: Result<(String, String, Vec<ChangedFile>), String>,
) {
    let Ok((refetch_head, refetch_base, nodes)) = refetch else {
        return;
    };
    if refetch_head == snapshot_head && refetch_base == snapshot_base {
        *files = Connection {
            nodes,
            page_info: PageInfo::default(),
        };
    }
}

const COMMIT_TREE_QUERY: &str = r#"
query($owner:String!, $name:String!, $oid:GitObjectID!) {
  repository(owner:$owner, name:$name) {
    object(oid:$oid) {
      ... on Commit {
        oid
        tree { oid }
      }
    }
  }
}
"#;

/// D4：rebase/force-push 后旧 reviewed head 可能已从 PR commits(last:100) 消失；
/// 对 PR 顶层 comments 的 `Reviewed commit:` marker 中无法由 commits 映射解析的
/// 不同 oid（去重，超过 16 个 fail-closed 不解析）逐一发追加查询解析 tree。
/// 任一 oid 查询失败/非 Commit/无 tree/回应 oid 与请求不一致 → 该 oid 缺失不继承
///（尽力而为，fail-closed，不向 snapshot 注入歧义）。
fn resolve_orphan_commit_trees(repository: &str, snapshot: &mut ExternalReviewSnapshot) {
    let Some((owner, name)) = repository.split_once('/') else {
        return;
    };
    let pr = &snapshot.pull_request;
    let mut commits_by_oid = BTreeMap::<&str, &str>::new();
    for node in &pr.commits.nodes {
        if let Some(tree) = node.commit.tree.as_ref() {
            commits_by_oid.insert(node.commit.oid.as_str(), tree.oid.as_str());
        }
    }
    let Some(orphan_oids) = orphan_reviewed_oids_for_tree_resolution(
        pr,
        &commits_by_oid,
        &snapshot.resolved_commit_trees,
    ) else {
        return;
    };
    for orphan in orphan_oids {
        let Ok(data) = gh_graphql_commit_tree(owner, name, &orphan) else {
            continue;
        };
        let Some(object) = data.repository.and_then(|repository| repository.object) else {
            continue;
        };
        let Some(tree) = object.tree else { continue };
        // 只接受回应 oid 与请求 marker 一致（前缀语义）的完整 OID，拒绝歧义解析
        if !valid_full_oid(&object.oid)
            || !valid_full_oid(&tree.oid)
            || !oid_matches_current(&orphan, &object.oid)
        {
            continue;
        }
        snapshot.resolved_commit_trees.insert(object.oid, tree.oid);
    }
}

fn gh_graphql_commit_tree(owner: &str, name: &str, oid: &str) -> Result<CommitTreeData, String> {
    let output = Command::new("gh")
        .arg("api")
        .arg("graphql")
        .arg("-F")
        .arg(format!("owner={owner}"))
        .arg("-F")
        .arg(format!("name={name}"))
        .arg("-F")
        .arg(format!("oid={oid}"))
        .arg("-f")
        .arg(format!("query={COMMIT_TREE_QUERY}"))
        .output()
        .map_err(|error| format!("无法运行 gh GraphQL：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh GraphQL 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let envelope = serde_json::from_slice::<GraphQlEnvelope<CommitTreeData>>(&output.stdout)
        .map_err(|error| format!("gh GraphQL 输出不是预期 JSON：{error}"))?;
    if !envelope.errors.is_empty() {
        return Err("GitHub GraphQL errors".to_string());
    }
    envelope
        .data
        .ok_or_else(|| "GitHub GraphQL response 缺少 data".to_string())
}

fn fetch_all_issue_comments(repository: &str, pr: u64) -> Result<Vec<IssueComment>, String> {
    let (owner, name) = repository
        .split_once('/')
        .ok_or_else(|| format!("repository 格式不正确：{repository}"))?;
    fetch_comment_pages(|cursor| load_issue_comments_page(owner, name, pr, cursor))
}

fn fetch_comment_pages(
    load_page: impl FnMut(Option<&str>) -> Result<CommentsConnection, String>,
) -> Result<Vec<IssueComment>, String> {
    fetch_cursor_pages("issue comments", load_page)
}

/// 通用 cursor 分页循环（issue comments 与 PR files 共用）：cursor 不推进、
/// 空页但 hasNextPage、缺 endCursor 均无法证明分页完整，一律 fail-closed。
fn fetch_cursor_pages<T>(
    label: &str,
    mut load_page: impl FnMut(Option<&str>) -> Result<CursorConnection<T>, String>,
) -> Result<Vec<T>, String> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = load_page(cursor.as_deref())?;
        let fetched = page.nodes.len();
        items.extend(page.nodes);
        let Some(next) = next_cursor_page(label, &page.page_info, fetched)? else {
            break;
        };
        if cursor.as_deref() == Some(next.as_str()) {
            return Err(format!(
                "{label} 分页 cursor 未推进，无法完成分页，按 fail-closed 处理"
            ));
        }
        cursor = Some(next);
    }
    Ok(items)
}

fn next_cursor_page(
    label: &str,
    page_info: &CursorPageInfo,
    fetched: usize,
) -> Result<Option<String>, String> {
    if !page_info.has_next_page {
        return Ok(None);
    }
    if fetched == 0 {
        return Err(format!(
            "{label} 分页返回空页但 hasNextPage 为 true，无法完成分页，按 fail-closed 处理"
        ));
    }
    match page_info
        .end_cursor
        .as_deref()
        .filter(|cursor| !cursor.is_empty())
    {
        Some(cursor) => Ok(Some(cursor.to_string())),
        None => Err(format!(
            "{label} 分页缺少 endCursor，无法完成分页，按 fail-closed 处理"
        )),
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

/// D1：files 补页绑定 PR current head 与 base——逐页校验 headRefOid/baseRefOid
/// 一致并随结果返回，防补读期间 force-push / base 重定向（A→B 补页、判定前恢复
/// A）把 A 的元数据与 B 的文件路径拼接；任一页 head 或 base 不同即按补页失败
/// 处理（调用方保留截断连接）
fn fetch_all_pr_files(
    repository: &str,
    pr: u64,
) -> Result<(String, String, Vec<ChangedFile>), String> {
    let (owner, name) = repository
        .split_once('/')
        .ok_or_else(|| format!("repository 格式不正确：{repository}"))?;
    let mut refetch_identity: Option<(String, String)> = None;
    let files = fetch_file_pages(|cursor| {
        let page = load_pr_files_page(owner, name, pr, cursor)?;
        let page_identity = (page.head_ref_oid, page.base_ref_oid);
        match &refetch_identity {
            None => refetch_identity = Some(page_identity),
            Some(identity) if *identity == page_identity => {}
            Some(_) => {
                return Err("PR files 补页期间 head/base 发生变化，按 fail-closed 处理".to_string());
            }
        }
        Ok(page.files)
    })?;
    let (head, base) =
        refetch_identity.ok_or_else(|| "PR files 补页未返回 headRefOid/baseRefOid".to_string())?;
    Ok((head, base, files))
}

fn fetch_file_pages(
    load_page: impl FnMut(Option<&str>) -> Result<CursorConnection<ChangedFile>, String>,
) -> Result<Vec<ChangedFile>, String> {
    fetch_cursor_pages("PR files", load_page)
}

fn load_pr_files_page(
    owner: &str,
    name: &str,
    pr: u64,
    cursor: Option<&str>,
) -> Result<FilesPagePullRequest, String> {
    let data: FilesPageData =
        gh_graphql_with_cursor(PULL_REQUEST_FILES_QUERY, owner, name, pr, cursor)?;
    data.repository
        .and_then(|repository| repository.pull_request)
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
        round_cap: None,
        resolved_commit_trees: BTreeMap::new(),
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
struct CommitTreeData {
    repository: Option<CommitTreeRepository>,
}

#[derive(Debug, Deserialize)]
struct CommitTreeRepository {
    object: Option<CommitTreeObject>,
}

#[derive(Debug, Deserialize)]
struct CommitTreeObject {
    oid: String,
    tree: Option<TreeRef>,
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

/// 带 endCursor 的分页连接（issue comments 与 PR files 的补齐分页查询共用）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", bound(deserialize = "T: Deserialize<'de>"))]
struct CursorConnection<T> {
    #[serde(default)]
    nodes: Vec<T>,
    #[serde(default)]
    page_info: CursorPageInfo,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CursorPageInfo {
    #[serde(default)]
    has_next_page: bool,
    #[serde(default)]
    end_cursor: Option<String>,
}

type CommentsConnection = CursorConnection<IssueComment>;

#[derive(Debug, Deserialize)]
struct FilesPageData {
    repository: Option<FilesPageRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FilesPageRepository {
    pull_request: Option<FilesPagePullRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FilesPagePullRequest {
    head_ref_oid: String,
    base_ref_oid: String,
    files: CursorConnection<ChangedFile>,
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
            current_head_tree_oid: None,
            current_base_oid: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            author: "wangzishi".to_string(),
            state,
            provider: Some("codex".to_string()),
            actor: Some(CODEX_ACTOR.to_string()),
            reviewed_head_oid: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            completion_time: Some("2026-07-24T17:15:39Z".to_string()),
            finding_count: 2,
            unresolved_blocking_threads: 0,
            deferred_findings: Vec::new(),
            unresolved_blocking_findings: Vec::new(),
            review_rounds: 0,
            round_cap: None,
            requires_rereview: false,
            pending_review_requests: 0,
            evidence: vec![ReviewEvidence {
                provider: "codex".to_string(),
                actor: CODEX_ACTOR.to_string(),
                source_kind: "issue_comment".to_string(),
                reviewed_head_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                reviewed_head_tree_oid: None,
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
            "permissions:\n  contents: read\n  pull-requests: write\n  issues: write\n  checks: write"
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
        assert_eq!(authorless_result.unresolved_blocking_threads, 1);

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
        assert_eq!(untrusted_result.unresolved_blocking_threads, 0);

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
        assert_eq!(authorless_reply_result.unresolved_blocking_threads, 1);

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
        assert_eq!(trusted_reply_result.unresolved_blocking_threads, 1);

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
        assert_eq!(authorless_reply_result.unresolved_blocking_threads, 0);

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
    fn documents_the_lockfile_only_codeql_advanced_setup_semantics() {
        let gates = include_str!("../../docs/governance/development-gates.md");
        let scanning = include_str!("../../docs/governance/security-scanning.md");
        let dependency = include_str!("../../docs/governance/dependency-security.md");
        let template = include_str!("../../.github/pull_request_template.md");
        let agent_guide = include_str!("../../docs/governance/agent-development-guide.md");

        assert!(gates.contains("dependabot-cargo-lock-only-v1"));
        assert!(scanning.contains("dependabot-cargo-lock-only-v1"));
        assert!(scanning.contains("default setup 必须为 `not-configured`"));
        assert!(scanning.contains("`Analyze (actions)`"));
        assert!(scanning.contains("`Analyze (rust)`"));
        assert!(scanning.contains("不能复用于新 head、G3 或 Merge Group"));
        for entry_point in [dependency, template] {
            assert!(entry_point.contains("dependabot-cargo-lock-only-v1"));
        }
        // agent-development-guide 不逐个列通道名（防双写漂移），锁其引用 §6.1 通道集合
        assert!(agent_guide.contains("`development-gates.md` 第 6.1 节定义的快速通道"));
        assert!(dependency.contains("`Analyze (actions)`"));
        assert!(dependency.contains("`Analyze (rust)`"));
        assert!(dependency.contains("不再提供 CodeQL `not applicable`"));
        assert!(!dependency.contains("CodeQL `not applicable` 与机器 completion"));
    }

    #[test]
    fn documents_the_fast_lane_machine_completion_contract() {
        let gates = include_str!("../../docs/governance/development-gates.md");
        let template = include_str!("../../.github/pull_request_template.md");
        let matrix = include_str!("../../docs/reference/validation-matrix.md");

        for lane in [
            "lockfile-only-dependabot",
            "docs-only-v1",
            "governance-docs-v1",
        ] {
            assert!(
                gates.contains(&format!("`{lane}`")),
                "gates 缺少快速通道 {lane}"
            );
        }
        // pure-move-v1 暂缓启用（GraphQL 无 rename 源路径，destination-only 校验
        // 不可证伪语义路径逃逸）：gates 只允许以「暂缓启用」注记形式提及，
        // template 的可选通道列表不得包含
        assert!(gates.contains("`pure-move-v1` 暂缓启用"));
        assert!(
            !template.contains("pure-move-v1"),
            "template 不得列出暂缓启用的 pure-move-v1"
        );
        for lane in ["docs-only-v1", "governance-docs-v1"] {
            assert!(template.contains(lane), "template 缺少快速通道 {lane}");
            assert!(matrix.contains(lane), "matrix 缺少快速通道 {lane}");
        }
        // 失效闸门与 waiver 区分的权威表述（改文案时必须与本 pin 锁步）：
        // findings/unresolved blocking/dismissed/分页溢出闸门三通道共享
        //（stale clean review 不关闸是有意设计：代码 stale_or_dismissed 仅在
        // DISMISSED 置位，旧 head 的 finding 型 review 已由 findings 闸门覆盖）；
        // snapshot 字段完整性前置仅适用两条新通道（dependabot 字段口径不变）
        assert!(gates.contains(
            "三条通道共享同一失效闸门：任何受信 actor 的 finding（含 P2/P3 deferred，按 findingCount 层面判定）抵达、unresolved blocking thread、dismissed 活动或 files 分页溢出，通道立即失效并回标准路径，不接受人工修补；pre-activation（#406 G1 冻结 `2026-08-20T04:20:39Z` 前）replay 不适用新通道；回标准路径后 deferred、round-cap 等既有语义照常。"
        ));
        assert!(gates.contains(
            "snapshot 字段完整性前置（任一文件的 `additions`/`deletions` 或 head commit 的 `message` 缺失即失效）仅适用两条新通道；dependabot 通道字段口径不变（旧快照兼容）"
        ));
        assert!(
            matrix.contains("snapshot 字段缺失（`additions`/`deletions`/`message`，仅两条新通道）")
        );
        assert!(matrix.contains("dependabot 通道字段口径不变（旧快照兼容）"));
        // AGENTS.md 排除与 files 补页 head 绑定的权威表述（改文案时必须与本 pin 锁步）
        assert!(gates.contains("不含 `AGENTS.md`——agent 指令 SSOT 属机器消费面，非惰性文档"));
        assert!(gates.contains(
            "files 补页绑定 current head 与 base，补读期间 head 或 base 变化即通道失效回标准路径"
        ));
        assert!(matrix.contains("files 补页期间 current head/base 变化"));
        assert!(gates.contains("它是机器 completion 而非 waiver"));
        assert!(gates.contains("第 6.1 节的快速通道是机器 completion 而非 waiver"));
        assert!(
            template.contains("仅当 PR 精确满足 `development-gates.md` 第 6.1 节定义的快速通道")
        );
        assert!(matrix.contains("快速通道 PR 抵达任何受信 finding（含 P2/P3 deferred）"));
        assert!(matrix.contains("含任一 `changeType=RENAMED` 文件"));
        assert!(matrix.contains(
            "pre-activation（G1 冻结 `2026-08-20T04:20:39Z` 前）replay 不注入新通道机器 completion"
        ));
        assert!(matrix.contains("快速通道 files 分页溢出"));
        assert!(matrix.contains("门禁代码面与运行时代码不豁免"));
        // files 分页失败语义：通道失效回标准路径，不作整体 fail-closed
        assert!(
            matrix.contains("files 只服务快速通道判定：分页溢出时全部机器通道失效并回标准路径")
        );
    }

    /// D1：构造一条绑定 current head 的受信 Codex finding（review + thread），
    /// 用于快速通道共享失效闸门测试。
    fn fast_lane_trusted_finding(snapshot: &mut ExternalReviewSnapshot, badge_severity: &str) {
        let head = snapshot.pull_request.head_ref_oid.clone();
        snapshot.pull_request.reviews.nodes.push(Review {
            id: "PRR-codex-fast-lane-finding".to_string(),
            author: Some(Actor {
                login: CODEX_ACTOR.to_string(),
            }),
            body: String::new(),
            state: "COMMENTED".to_string(),
            submitted_at: Some("2026-08-20T09:00:00Z".to_string()),
            url: Some(
                "https://github.com/illusion-tech/laneflow/pull/460#pullrequestreview-9"
                    .to_string(),
            ),
            commit: Some(CommitRef {
                oid: head.clone(),
                tree: None,
            }),
        });
        snapshot
            .pull_request
            .review_threads
            .nodes
            .push(ReviewThread {
                id: "PRRT-codex-fast-lane-finding".to_string(),
                is_resolved: false,
                is_outdated: false,
                comments: Connection {
                    nodes: vec![ReviewThreadComment {
                        id: "PRRC-codex-fast-lane-finding".to_string(),
                        author: Some(Actor {
                            login: CODEX_ACTOR.to_string(),
                        }),
                        body: codex_badge_body(badge_severity, "Fast lane gate probe."),
                        created_at: "2026-08-20T09:00:00Z".to_string(),
                        updated_at: "2026-08-20T09:00:00Z".to_string(),
                        url: "https://github.com/illusion-tech/laneflow/pull/460#discussion_r9"
                            .to_string(),
                        pull_request_review: Some(ReviewReference {
                            id: "PRR-codex-fast-lane-finding".to_string(),
                            author: Some(Actor {
                                login: CODEX_ACTOR.to_string(),
                            }),
                            state: "COMMENTED".to_string(),
                            submitted_at: Some("2026-08-20T09:00:00Z".to_string()),
                            commit: Some(CommitRef {
                                oid: head.clone(),
                                tree: None,
                            }),
                        }),
                    }],
                    page_info: PageInfo::default(),
                },
            });
    }

    fn no_machine_completion_evidence(result: &ExternalReviewResult) -> bool {
        !result
            .evidence
            .iter()
            .any(|item| item.source_kind == "machine_verification")
    }

    #[test]
    fn fast_lane_docs_only_hits_and_fails_closed_on_boundary_mismatch() {
        let contents = include_str!("../fixtures/external-review/fast-lane-docs-only.json");
        let pass = evaluate_snapshot(&fixture(contents));
        assert_eq!(pass.state, ExternalReviewState::Pass);
        assert_eq!(pass.provider.as_deref(), Some("docs-only-v1"));
        assert_eq!(pass.actor.as_deref(), Some("github-metadata"));
        assert_eq!(
            pass.reviewed_head_oid.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(
            pass.completion_time.as_deref(),
            Some("2026-08-20T08:00:00Z")
        );
        assert_eq!(pass.finding_count, 0);
        assert_eq!(pass.review_rounds, 0);
        assert_eq!(
            pass.evidence[0].evidence_url,
            "https://github.com/illusion-tech/laneflow/commit/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );

        // 任一文件不匹配 docs/**/*.md、根级 *.md、research/**/*.md → 通道不成立；
        // AGENTS.md 是 agent 指令 SSOT（机器消费面），显式排除
        for path in [
            "src/lib.rs",
            "research/issue-406-fast-lanes/data.json",
            "guides/README.md",
            "docs-only/readme.md",
            "docs/reference/compiler-calibration-contract-v1.json",
            "AGENTS.md",
        ] {
            let mut snapshot = fixture(contents);
            snapshot.pull_request.files.nodes[0].path = path.to_string();
            let result = evaluate_snapshot(&snapshot);
            assert_eq!(result.state, ExternalReviewState::AwaitingReview, "{path}");
            assert!(no_machine_completion_evidence(&result), "{path}");
        }

        // 空 diff 不产生机器 completion
        let mut empty = fixture(contents);
        empty.pull_request.files.nodes.clear();
        assert_eq!(
            evaluate_snapshot(&empty).state,
            ExternalReviewState::AwaitingReview
        );
    }

    #[test]
    fn fast_lane_fails_closed_on_missing_snapshot_fields() {
        let contents = include_str!("../fixtures/external-review/fast-lane-docs-only.json");

        // head commit message 缺失（旧快照形态）→ 共享完整性闸门 fail-closed
        let mut no_message = fixture(contents);
        no_message.pull_request.commits.nodes[0].commit.message = None;
        let result = evaluate_snapshot(&no_message);
        assert_eq!(result.state, ExternalReviewState::AwaitingReview);
        assert!(no_machine_completion_evidence(&result));

        // 任一文件 additions 缺失 → 共享完整性闸门 fail-closed
        let mut no_additions = fixture(contents);
        no_additions.pull_request.files.nodes[0].additions = None;
        let result = evaluate_snapshot(&no_additions);
        assert_eq!(result.state, ExternalReviewState::AwaitingReview);
        assert!(no_machine_completion_evidence(&result));

        // 对称地，仅 deletions 缺失同样 fail-closed
        let mut no_deletions = fixture(contents);
        no_deletions.pull_request.files.nodes[0].deletions = None;
        let result = evaluate_snapshot(&no_deletions);
        assert_eq!(result.state, ExternalReviewState::AwaitingReview);
        assert!(no_machine_completion_evidence(&result));
    }

    #[test]
    fn fast_lane_renamed_files_are_excluded_from_all_fast_lanes() {
        // pure-move-v1 暂缓启用后的统一语义：任何 changeType=RENAMED 文件使全部
        // 快速通道失效（GraphQL PullRequestChangedFile 无 rename 源路径，
        // destination-only 校验无法防语义路径文件 0/0 改名逃逸，如
        // .github/dependabot.yml → docs/x.md 关停 Dependabot）
        let docs_contents = include_str!("../fixtures/external-review/fast-lane-docs-only.json");

        // destination 在 docs/ 下的 0/0 改名 → 所有通道失效
        let mut renamed_doc = fixture(docs_contents);
        renamed_doc.pull_request.files.nodes = vec![ChangedFile {
            path: "docs/governance/development-gates.md".to_string(),
            change_type: "RENAMED".to_string(),
            additions: Some(0),
            deletions: Some(0),
        }];
        let result = evaluate_snapshot(&renamed_doc);
        assert_eq!(result.state, ExternalReviewState::AwaitingReview);
        assert!(no_machine_completion_evidence(&result));

        // destination 在 crates/ 下的 0/0 改名 → 同样失效
        let mut renamed_code = fixture(docs_contents);
        renamed_code.pull_request.files.nodes = vec![ChangedFile {
            path: "crates/laneflow-core/src/lib.rs".to_string(),
            change_type: "RENAMED".to_string(),
            additions: Some(0),
            deletions: Some(0),
        }];
        let result = evaluate_snapshot(&renamed_code);
        assert_eq!(result.state, ExternalReviewState::AwaitingReview);
        assert!(no_machine_completion_evidence(&result));

        // governance-docs 通道同样排除：.agents/**/*.md 0/0 改名 +
        // Slice: governance → 失效
        let mut renamed_agents = fixture(include_str!(
            "../fixtures/external-review/fast-lane-governance-docs.json"
        ));
        renamed_agents.pull_request.files.nodes = vec![ChangedFile {
            path: ".agents/skills/laneflow-governance/SKILL.md".to_string(),
            change_type: "RENAMED".to_string(),
            additions: Some(0),
            deletions: Some(0),
        }];
        let result = evaluate_snapshot(&renamed_agents);
        assert_eq!(result.state, ExternalReviewState::AwaitingReview);
        assert!(no_machine_completion_evidence(&result));
    }

    #[test]
    fn fast_lane_governance_docs_requires_slice_and_allowed_paths() {
        let contents = include_str!("../fixtures/external-review/fast-lane-governance-docs.json");
        let pass = evaluate_snapshot(&fixture(contents));
        assert_eq!(pass.state, ExternalReviewState::Pass);
        assert_eq!(pass.provider.as_deref(), Some("governance-docs-v1"));

        // head commit message 缺失 / Slice 值非 governance → 不成立
        let mut no_message = fixture(contents);
        no_message.pull_request.commits.nodes[0].commit.message = None;
        assert_eq!(
            evaluate_snapshot(&no_message).state,
            ExternalReviewState::AwaitingReview
        );
        let mut docs_only_slice = fixture(contents);
        docs_only_slice.pull_request.commits.nodes[0].commit.message =
            docs_only_slice.pull_request.commits.nodes[0]
                .commit
                .message
                .as_ref()
                .map(|message| message.replace("Slice: governance", "Slice: docs-only"));
        assert_eq!(
            evaluate_snapshot(&docs_only_slice).state,
            ExternalReviewState::AwaitingReview
        );
        // 大小写变体（Slice: Governance）→ 值非精确 governance，fail-closed
        let mut capitalized_slice = fixture(contents);
        capitalized_slice.pull_request.commits.nodes[0]
            .commit
            .message = capitalized_slice.pull_request.commits.nodes[0]
            .commit
            .message
            .as_ref()
            .map(|message| message.replace("Slice: governance", "Slice: Governance"));
        assert_eq!(
            evaluate_snapshot(&capitalized_slice).state,
            ExternalReviewState::AwaitingReview
        );
        // 多个 Slice 行歧义 → fail-closed 不成立
        let mut ambiguous_slice = fixture(contents);
        ambiguous_slice.pull_request.commits.nodes[0].commit.message =
            ambiguous_slice.pull_request.commits.nodes[0]
                .commit
                .message
                .as_ref()
                .map(|message| format!("{message}\nSlice: governance"));
        assert_eq!(
            evaluate_snapshot(&ambiguous_slice).state,
            ExternalReviewState::AwaitingReview
        );

        // 门禁代码面与运行时代码不豁免：workflows/xtask/schemas/crates 任一命中即不成立；
        // research/**/*.md 与 docs/ 下非 .md 文件不在 governance-docs 允许集合内
        for path in [
            ".github/workflows/ci.yml",
            ".github/workflows/notes.md",
            "xtask/src/main.rs",
            "schemas/laneflow-data-v0.10.schema.json",
            "crates/laneflow-core/src/lib.rs",
            "research/issue-406-fast-lanes/notes.md",
            "docs/reference/compiler-calibration-contract-v1.json",
        ] {
            let mut snapshot = fixture(contents);
            snapshot.pull_request.files.nodes[0].path = path.to_string();
            let result = evaluate_snapshot(&snapshot);
            assert_eq!(result.state, ExternalReviewState::AwaitingReview, "{path}");
            assert!(no_machine_completion_evidence(&result), "{path}");
        }

        // 只认 head commit 本身：旧 commit 带 Slice: governance 而 head 没有 → 不成立
        let mut older_commit_only = fixture(contents);
        older_commit_only.pull_request.commits.nodes[0]
            .commit
            .message = None;
        let mut older = commit_node(
            "cccccccccccccccccccccccccccccccccccccccc",
            Some("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"),
        );
        older.commit.message =
            Some("docs(governance): 旧提交\n\nGate: G3 Candidate\nSlice: governance".to_string());
        older_commit_only
            .pull_request
            .commits
            .nodes
            .push(older.clone());
        assert_eq!(
            evaluate_snapshot(&older_commit_only).state,
            ExternalReviewState::AwaitingReview
        );
        // 反向：head 带 Slice: governance，旧 commit 没有 → 仍成立
        let mut head_has_slice = fixture(contents);
        older.commit.message = None;
        head_has_slice.pull_request.commits.nodes.push(older);
        assert_eq!(
            evaluate_snapshot(&head_has_slice).state,
            ExternalReviewState::Pass
        );
    }

    #[test]
    fn fast_lane_shared_gate_disables_machine_completion_on_any_trusted_finding() {
        let contents = include_str!("../fixtures/external-review/fast-lane-docs-only.json");

        // 受信 P1 blocking finding → 通道失效，FindingsOpen
        let mut p1 = fixture(contents);
        fast_lane_trusted_finding(&mut p1, "P1");
        let p1_result = evaluate_snapshot(&p1);
        assert_eq!(p1_result.state, ExternalReviewState::FindingsOpen);
        assert!(no_machine_completion_evidence(&p1_result));

        // 受信 P2 deferred finding → 通道同样失效（findingCount 层面）；
        // 回标准路径后由既有 D2 deferred 语义放行（非机器 completion）
        let mut p2 = fixture(contents);
        fast_lane_trusted_finding(&mut p2, "P2");
        let p2_result = evaluate_snapshot(&p2);
        assert_eq!(p2_result.state, ExternalReviewState::Pass);
        assert_eq!(p2_result.provider.as_deref(), Some("codex"));
        assert_eq!(p2_result.deferred_findings.len(), 1);
        assert!(no_machine_completion_evidence(&p2_result));

        // 已 resolved 的受信 finding 同样使通道失效（回标准路径，待 re-review）
        let mut resolved = fixture(contents);
        fast_lane_trusted_finding(&mut resolved, "P1");
        resolved.pull_request.review_threads.nodes[0].is_resolved = true;
        let resolved_result = evaluate_snapshot(&resolved);
        assert_eq!(resolved_result.state, ExternalReviewState::AwaitingRereview);
        assert!(no_machine_completion_evidence(&resolved_result));

        // dismissed review → stale_or_dismissed 关闭闸门
        let mut dismissed = fixture(contents);
        dismissed.pull_request.reviews.nodes.push(Review {
            id: "PRR-dismissed".to_string(),
            author: Some(Actor {
                login: "wangzishi".to_string(),
            }),
            body: String::new(),
            state: "DISMISSED".to_string(),
            submitted_at: Some("2026-08-20T09:00:00Z".to_string()),
            url: Some(
                "https://github.com/illusion-tech/laneflow/pull/460#pullrequestreview-10"
                    .to_string(),
            ),
            commit: Some(CommitRef {
                oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                tree: None,
            }),
        });
        let dismissed_result = evaluate_snapshot(&dismissed);
        assert_eq!(dismissed_result.state, ExternalReviewState::Stale);
        assert!(no_machine_completion_evidence(&dismissed_result));

        // 人工 unthreaded finding（review 级 CHANGES_REQUESTED，无关联 thread）→ 闸门同样关闭
        let mut unthreaded = fixture(contents);
        let head = unthreaded.pull_request.head_ref_oid.clone();
        unthreaded.pull_request.reviews.nodes.push(Review {
            id: "PRR-human-changes-fast-lane".to_string(),
            author: Some(Actor {
                login: "wangzishi".to_string(),
            }),
            body: String::new(),
            state: "CHANGES_REQUESTED".to_string(),
            submitted_at: Some("2026-08-20T09:00:00Z".to_string()),
            url: Some(
                "https://github.com/illusion-tech/laneflow/pull/460#pullrequestreview-12"
                    .to_string(),
            ),
            commit: Some(CommitRef {
                oid: head,
                tree: None,
            }),
        });
        let unthreaded_result = evaluate_snapshot(&unthreaded);
        assert_eq!(
            unthreaded_result.state,
            ExternalReviewState::AwaitingRereview
        );
        assert!(no_machine_completion_evidence(&unthreaded_result));
    }

    #[test]
    fn fast_lane_fails_closed_on_truncated_files_or_missing_head_commit() {
        let contents = include_str!("../fixtures/external-review/fast-lane-docs-only.json");

        // files 分页溢出（snapshot 截断）→ 通道失效，回标准路径
        let mut truncated = fixture(contents);
        truncated.pull_request.files.page_info.has_next_page = true;
        let result = evaluate_snapshot(&truncated);
        assert_eq!(result.state, ExternalReviewState::AwaitingReview);
        assert!(no_machine_completion_evidence(&result));

        // 分页溢出不是整体 fail-closed：存在有效人工 clean 时标准路径照常放行
        let mut truncated_with_review = fixture(contents);
        truncated_with_review
            .pull_request
            .files
            .page_info
            .has_next_page = true;
        truncated_with_review
            .pull_request
            .reviews
            .nodes
            .push(Review {
                id: "PRR-human-clean".to_string(),
                author: Some(Actor {
                    login: "wangzishi".to_string(),
                }),
                body: String::new(),
                state: "APPROVED".to_string(),
                submitted_at: Some("2026-08-20T09:00:00Z".to_string()),
                url: Some(
                    "https://github.com/illusion-tech/laneflow/pull/460#pullrequestreview-11"
                        .to_string(),
                ),
                commit: Some(CommitRef {
                    oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                    tree: None,
                }),
            });
        let result = evaluate_snapshot(&truncated_with_review);
        assert_eq!(result.state, ExternalReviewState::Pass);
        assert_eq!(result.provider.as_deref(), Some("human"));
        assert!(no_machine_completion_evidence(&result));

        // head commit 不在 commits 连接内 → 无证据来源，通道失效
        let mut no_commits = fixture(contents);
        no_commits.pull_request.commits.nodes.clear();
        assert_eq!(
            evaluate_snapshot(&no_commits).state,
            ExternalReviewState::AwaitingReview
        );

        // head commit 证据字段无效（URL 非 GitHub HTTPS）→ 通道失效
        let mut invalid_url = fixture(contents);
        invalid_url.pull_request.commits.nodes[0].commit.url =
            "https://example.com/commit/aaaa".to_string();
        assert_eq!(
            evaluate_snapshot(&invalid_url).state,
            ExternalReviewState::AwaitingReview
        );
    }

    #[test]
    fn files_refetch_adoption_requires_stable_head_and_base_match() {
        // 补页 seam 截获逻辑：补页返回 head/base 与 snapshot 任一不一致（补读期间
        // force-push / base 重定向，如 A→B→A 拼接）或补页失败 → 保留截断连接，
        // 通道失效（evaluator 层「截断 → AwaitingReview、无机器证据、非
        // ProviderError」由上一个测试钉住）
        const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        const BASE: &str = "cccccccccccccccccccccccccccccccccccccccc";
        let truncated = || Connection {
            nodes: vec![ChangedFile {
                path: "docs/governance/development-gates.md".to_string(),
                change_type: "MODIFIED".to_string(),
                additions: Some(8),
                deletions: Some(2),
            }],
            page_info: PageInfo {
                has_next_page: true,
            },
        };
        let refetched = || {
            vec![
                ChangedFile {
                    path: "docs/governance/development-gates.md".to_string(),
                    change_type: "MODIFIED".to_string(),
                    additions: Some(8),
                    deletions: Some(2),
                },
                ChangedFile {
                    path: "docs/reference/glossary.md".to_string(),
                    change_type: "ADDED".to_string(),
                    additions: Some(4),
                    deletions: Some(0),
                },
            ]
        };

        // head 与 base 均匹配：采用补页结果（分页标记清零，通道可判定）
        let mut files = truncated();
        adopt_files_refetch(
            &mut files,
            HEAD,
            BASE,
            Ok((HEAD.to_string(), BASE.to_string(), refetched())),
        );
        assert!(!files.page_info.has_next_page);
        assert_eq!(files.nodes.len(), 2);

        // head 不匹配（base 匹配）：保留截断连接（has_next_page 保持 true），通道失效
        let mut files = truncated();
        adopt_files_refetch(
            &mut files,
            HEAD,
            BASE,
            Ok((
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
                BASE.to_string(),
                refetched(),
            )),
        );
        assert!(files.page_info.has_next_page);
        assert_eq!(files.nodes.len(), 1);

        // base 不匹配（head 匹配）：同样保留截断连接，通道失效
        let mut files = truncated();
        adopt_files_refetch(
            &mut files,
            HEAD,
            BASE,
            Ok((
                HEAD.to_string(),
                "dddddddddddddddddddddddddddddddddddddddd".to_string(),
                refetched(),
            )),
        );
        assert!(files.page_info.has_next_page);
        assert_eq!(files.nodes.len(), 1);

        // 补页失败：同语义，不传播 Err
        let mut files = truncated();
        adopt_files_refetch(&mut files, HEAD, BASE, Err("network down".to_string()));
        assert!(files.page_info.has_next_page);
        assert_eq!(files.nodes.len(), 1);
    }

    #[test]
    fn fast_lane_hit_order_is_deterministic_across_overlapping_lanes() {
        // docs/**/*.md 修改且 Slice: governance 同时满足两条通道，
        // 按固定顺序 docs-only-v1 → governance-docs-v1 取首个
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/fast-lane-governance-docs.json"
        ));
        snapshot.pull_request.files.nodes = vec![ChangedFile {
            path: "docs/governance/development-gates.md".to_string(),
            change_type: "MODIFIED".to_string(),
            additions: Some(8),
            deletions: Some(2),
        }];
        let result = evaluate_snapshot(&snapshot);
        assert_eq!(result.state, ExternalReviewState::Pass);
        assert_eq!(result.provider.as_deref(), Some("docs-only-v1"));

        // docs-only 不命中而 governance-docs 命中（.agents/**/*.md 修改 +
        // Slice: governance）→ governance-docs 胜出
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/fast-lane-governance-docs.json"
        ));
        snapshot.pull_request.files.nodes = vec![ChangedFile {
            path: ".agents/skills/laneflow-governance/SKILL.md".to_string(),
            change_type: "MODIFIED".to_string(),
            additions: Some(3),
            deletions: Some(1),
        }];
        let result = evaluate_snapshot(&snapshot);
        assert_eq!(result.state, ExternalReviewState::Pass);
        assert_eq!(result.provider.as_deref(), Some("governance-docs-v1"));
    }

    #[test]
    fn pre_activation_replay_disables_fast_lane_machine_completion() {
        let snapshot = fixture(include_str!(
            "../fixtures/external-review/fast-lane-docs-only.json"
        ));
        // 激活：docs-only-v1 机器 completion
        let active = evaluate_snapshot_with_policy(&snapshot, true);
        assert_eq!(active.state, ExternalReviewState::Pass);
        assert_eq!(active.provider.as_deref(), Some("docs-only-v1"));
        // pre-activation replay：不注入新通道机器 completion，回标准路径
        let inactive = evaluate_snapshot_with_policy(&snapshot, false);
        assert_eq!(inactive.state, ExternalReviewState::AwaitingReview);
        assert!(no_machine_completion_evidence(&inactive));
    }

    #[test]
    fn fast_lane_completion_tolerates_codex_outage_comment_like_dependabot() {
        let contents = include_str!("../fixtures/external-review/fast-lane-docs-only.json");
        let outage_comment = || IssueComment {
            id: "IC-codex-outage".to_string(),
            author: Some(Actor {
                login: CODEX_ACTOR.to_string(),
            }),
            body: "To use Codex here, please ask the admin to install the app.".to_string(),
            created_at: "2026-08-20T09:00:00Z".to_string(),
            updated_at: "2026-08-20T09:00:00Z".to_string(),
            url: "https://github.com/illusion-tech/laneflow/pull/460#issuecomment-9".to_string(),
        };

        // 快速通道命中的 PR：Codex 故障注释不产生 provider-error 诊断
        //（与 dependabot 通道同构——机器 completion 不依赖 Codex 可用性）
        let mut hit = fixture(contents);
        hit.pull_request.comments.nodes.push(outage_comment());
        let result = evaluate_snapshot(&hit);
        assert_eq!(result.state, ExternalReviewState::Pass);
        assert_eq!(result.provider.as_deref(), Some("docs-only-v1"));
        assert!(
            !result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("环境不可用"))
        );

        // 通道不命中：故障诊断照常产生（既有行为不变）
        let mut miss = fixture(contents);
        miss.pull_request.files.nodes[0].path = "src/lib.rs".to_string();
        miss.pull_request.comments.nodes.push(outage_comment());
        let result = evaluate_snapshot(&miss);
        assert_eq!(result.state, ExternalReviewState::ProviderError);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("环境不可用"))
        );
    }

    #[test]
    fn fast_lane_outage_exception_yields_to_closed_machine_completion_gate() {
        let contents = include_str!("../fixtures/external-review/fast-lane-docs-only.json");
        // 结构命中 docs-only 但已有受信 P2 finding：闸门关闭 → 快速通道侧故障
        // 例外不生效，「环境不可用」诊断照常产生（不吞故障歧义信号）
        let mut gated = fixture(contents);
        fast_lane_trusted_finding(&mut gated, "P2");
        gated.pull_request.comments.nodes.push(IssueComment {
            id: "IC-codex-outage-gated".to_string(),
            author: Some(Actor {
                login: CODEX_ACTOR.to_string(),
            }),
            body: "To use Codex here, please ask the admin to install the app.".to_string(),
            created_at: "2026-08-20T09:05:00Z".to_string(),
            updated_at: "2026-08-20T09:05:00Z".to_string(),
            url: "https://github.com/illusion-tech/laneflow/pull/460#issuecomment-10".to_string(),
        });
        let result = evaluate_snapshot(&gated);
        assert_eq!(result.state, ExternalReviewState::ProviderError);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("环境不可用"))
        );
    }

    #[test]
    fn fast_lane_authorless_unresolved_thread_blocks_machine_completion() {
        let contents = include_str!("../fixtures/external-review/fast-lane-docs-only.json");
        // authorless（deleted/unavailable reviewer，身份不可核验）未结 thread：
        // 与 dependabot 一致按 blocking 处理——关闭闸门，机器 completion 不注入
        let mut snapshot = fixture(contents);
        snapshot
            .pull_request
            .review_threads
            .nodes
            .push(ReviewThread {
                id: "PRRT-authorless-fast-lane".to_string(),
                is_resolved: false,
                is_outdated: false,
                comments: Connection {
                    nodes: vec![ReviewThreadComment {
                        id: "PRRC-authorless-fast-lane".to_string(),
                        author: None,
                        body: "This looks wrong.".to_string(),
                        created_at: "2026-08-20T09:00:00Z".to_string(),
                        updated_at: "2026-08-20T09:00:00Z".to_string(),
                        url: "https://github.com/illusion-tech/laneflow/pull/460#discussion_r11"
                            .to_string(),
                        pull_request_review: None,
                    }],
                    page_info: PageInfo::default(),
                },
            });
        let result = evaluate_snapshot(&snapshot);
        // 无受信 finding 与 clean completion 时落 AwaitingReview；关键是 blocking
        // 计数关闸、机器 completion 证据不注入（修复前为 Pass）
        assert_eq!(result.state, ExternalReviewState::AwaitingReview);
        assert_eq!(result.unresolved_blocking_threads, 1);
        assert!(no_machine_completion_evidence(&result));
    }

    #[test]
    fn parses_head_commit_slice_value_fail_closed() {
        assert_eq!(
            head_commit_slice_value("Slice: governance"),
            Some("governance")
        );
        let message = "docs(governance): 标题\n\nGate: G3 Candidate\nSlice: governance\nImpact: core-api=none; data-format=none; adapter-api=none\nScope: x\nValidation: y\nDocs: updated\n\nRefs: #406";
        assert_eq!(head_commit_slice_value(message), Some("governance"));
        // 无 Slice 行 / 空值 / 多行歧义 / 非严格「冒号 + 一个空格」→ None
        assert_eq!(head_commit_slice_value("no slice here"), None);
        assert_eq!(head_commit_slice_value("Slice: "), None);
        assert_eq!(
            head_commit_slice_value("Slice: docs-only\nSlice: governance"),
            None
        );
        assert_eq!(head_commit_slice_value("Slice:governance"), None);
        assert_eq!(head_commit_slice_value(" Slice: governance"), None);
        // 大小写敏感：值按原样返回，由通道等值比较 fail-closed（Governance ≠ governance）
        assert_eq!(
            head_commit_slice_value("Slice: Governance"),
            Some("Governance")
        );
        // 行尾空白（如 CRLF）容忍，值内的内容保持精确
        assert_eq!(
            head_commit_slice_value("Slice: governance\r"),
            Some("governance")
        );
        assert_eq!(
            head_commit_slice_value("Slice: governance extra"),
            Some("governance extra")
        );
    }

    #[test]
    fn fast_lane_new_snapshot_fields_stay_deserialization_compatible() {
        // 旧快照无 additions/deletions/message 字段：serde default → None，不破坏
        // 既有评估（dependabot 窄通道照旧生效）
        let legacy = fixture(include_str!(
            "../fixtures/external-review/dependabot-lockfile-wrong-sha.json"
        ));
        assert!(legacy.pull_request.files.nodes[0].additions.is_none());
        assert!(legacy.pull_request.files.nodes[0].deletions.is_none());
        assert!(
            legacy.pull_request.commits.nodes[0]
                .commit
                .message
                .is_none()
        );
        assert_eq!(evaluate_snapshot(&legacy).state, ExternalReviewState::Pass);

        // 新字段在快照 JSON 中正常反序列化（deny_unknown_fields 不拒绝）
        let docs = fixture(include_str!(
            "../fixtures/external-review/fast-lane-docs-only.json"
        ));
        assert_eq!(docs.pull_request.files.nodes[0].additions, Some(12));
        assert_eq!(docs.pull_request.files.nodes[0].deletions, Some(3));
        assert!(
            docs.pull_request.commits.nodes[0]
                .commit
                .message
                .as_deref()
                .is_some_and(|message| message.contains("Slice: docs-only"))
        );
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
    fn result_json_keeps_v1_unresolved_actionable_threads_key() {
        // D4：v1 wire 键保持 unresolvedActionableThreads，下游消费者不受影响。
        let json = serde_json::to_value(sample_result(ExternalReviewState::Pass))
            .expect("result should serialize");

        assert_eq!(json["unresolvedActionableThreads"], 0);
        assert!(json.get("unresolvedBlockingThreads").is_none());
    }

    #[test]
    fn exact_head_clean_after_finding_passes() {
        let result = evaluate_snapshot(&fixture(include_str!(
            "../fixtures/external-review/history-pr-232-final.json"
        )));
        assert_eq!(result.state, ExternalReviewState::Pass);
        assert_eq!(result.finding_count, 2);
        assert_eq!(result.unresolved_blocking_threads, 0);
        assert!(!result.requires_rereview);
    }

    fn codex_badge_body(severity: &str, title: &str) -> String {
        format!(
            "**<sub><sub>![{severity} Badge](https://img.shields.io/badge/{severity}-yellow?style=flat)</sub></sub>  {title}"
        )
    }

    #[test]
    fn parses_severity_badges_fail_closed() {
        assert_eq!(parse_severity_badge(&codex_badge_body("P0", "t")), Some(0));
        assert_eq!(parse_severity_badge(&codex_badge_body("P1", "t")), Some(1));
        assert_eq!(parse_severity_badge(&codex_badge_body("P2", "t")), Some(2));
        assert_eq!(parse_severity_badge(&codex_badge_body("P3", "t")), Some(3));
        // 无 badge / 垃圾文本 / 形态不符 → None（调用方按 blocking 处理）
        assert_eq!(parse_severity_badge("no badge here"), None);
        assert_eq!(parse_severity_badge("![PX Badge]"), None);
        assert_eq!(parse_severity_badge("![P2Badge]"), None);
        assert_eq!(parse_severity_badge("P2 Badge"), None);
        // 多 badge 取首个
        assert_eq!(
            parse_severity_badge(&format!(
                "{}\n\n{}",
                codex_badge_body("P2", "first"),
                codex_badge_body("P1", "second")
            )),
            Some(2)
        );
    }

    #[test]
    fn p2_p3_findings_defer_without_blocking_merge() {
        let contents = include_str!("../fixtures/external-review/copilot-findings-open.json");
        // 基线：无 badge thread 保持 blocking（行为不变）
        let baseline = evaluate_snapshot(&fixture(contents));
        assert_eq!(baseline.state, ExternalReviewState::FindingsOpen);
        assert_eq!(baseline.unresolved_blocking_threads, 1);
        assert!(baseline.deferred_findings.is_empty());

        let with_badge = |severity: &str| {
            let mut snapshot = fixture(contents);
            snapshot.pull_request.review_threads.nodes[0].comments.nodes[0].body =
                codex_badge_body(severity, "Require closingIssuesReferences.");
            snapshot
        };
        for (severity, expected, blocking, deferred) in [
            ("P0", ExternalReviewState::FindingsOpen, 1, 0),
            ("P1", ExternalReviewState::FindingsOpen, 1, 0),
            ("P2", ExternalReviewState::Pass, 0, 1),
            ("P3", ExternalReviewState::Pass, 0, 1),
            // 未知严重度 fail-closed 按 blocking
            ("P4", ExternalReviewState::FindingsOpen, 1, 0),
        ] {
            let result = evaluate_snapshot(&with_badge(severity));
            assert_eq!(result.state, expected, "severity {severity}");
            assert_eq!(
                result.unresolved_blocking_threads, blocking,
                "severity {severity}"
            );
            assert_eq!(
                result.deferred_findings.len(),
                deferred,
                "severity {severity}"
            );
        }

        // deferred 明细与 check output 披露
        let p2 = evaluate_snapshot(&with_badge("P2"));
        assert_eq!(p2.deferred_findings.len(), 1);
        assert_eq!(p2.deferred_findings[0].thread_id, "PRRT-copilot-finding");
        assert_eq!(p2.deferred_findings[0].severity, "P2");
        assert_eq!(
            p2.deferred_findings[0].url,
            "https://github.com/illusion-tech/laneflow/pull/38#discussion_r3550034350"
        );
        let payload = build_check_run_payload(
            &p2,
            "https://github.com/illusion-tech/laneflow/actions/runs/1",
            "external-review:test".to_string(),
        );
        assert!(payload.output.summary.contains("deferred=1"));
        assert!(payload.output.summary.contains("rounds=1"));
        assert!(payload.output.text.contains("Deferred findings"));
        assert!(payload.output.text.contains(
            "P2 https://github.com/illusion-tech/laneflow/pull/38#discussion_r3550034350"
        ));
    }

    #[test]
    fn mixed_severity_keeps_p1_blocking_and_lists_only_p2_deferred() {
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/copilot-findings-open.json"
        ));
        snapshot.pull_request.review_threads.nodes[0].comments.nodes[0].body =
            codex_badge_body("P2", "Require closingIssuesReferences.");
        // 追加同一 review 的 P1 blocking thread
        let mut p1_thread = snapshot.pull_request.review_threads.nodes[0].clone();
        p1_thread.id = "PRRT-copilot-finding-p1".to_string();
        let p1_comment = &mut p1_thread.comments.nodes[0];
        p1_comment.id = "PRRC-copilot-finding-p1".to_string();
        p1_comment.url =
            "https://github.com/illusion-tech/laneflow/pull/38#discussion_r3550034351".to_string();
        p1_comment.body = codex_badge_body("P1", "Must fix before merge.");
        snapshot.pull_request.review_threads.nodes.push(p1_thread);

        let result = evaluate_snapshot(&snapshot);
        assert_eq!(result.state, ExternalReviewState::FindingsOpen);
        assert!(result.requires_rereview);
        assert_eq!(result.unresolved_blocking_threads, 1);
        assert_eq!(result.deferred_findings.len(), 1);
        assert_eq!(
            result.deferred_findings[0].thread_id,
            "PRRT-copilot-finding"
        );
        assert_eq!(result.unresolved_blocking_findings.len(), 1);
        assert_eq!(
            result.unresolved_blocking_findings[0].thread_id,
            "PRRT-copilot-finding-p1"
        );
    }

    #[test]
    fn untrusted_first_comment_badge_is_not_honored() {
        // 不可信 author 在首条 comment 放置 P2 badge 文本，受信 finding（无 badge）在后：
        // 严重度取自首条受信 finding comment，无 badge → blocking；
        // 前文不可信 badge 文本不采信
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/copilot-findings-open.json"
        ));
        let mut trusted_reply =
            snapshot.pull_request.review_threads.nodes[0].comments.nodes[0].clone();
        trusted_reply.id = "PRRC-copilot-reply".to_string();
        trusted_reply.url =
            "https://github.com/illusion-tech/laneflow/pull/38#discussion_r3550034352".to_string();
        trusted_reply.body = "Substantive concern without badge.".to_string();
        let thread = &mut snapshot.pull_request.review_threads.nodes[0];
        let first = &mut thread.comments.nodes[0];
        first.author = Some(Actor {
            login: "external-contributor".to_string(),
        });
        first.pull_request_review = None;
        first.body = codex_badge_body("P2", "fake deferred marker.");
        thread.comments.nodes.push(trusted_reply);

        let result = evaluate_snapshot(&snapshot);
        assert_eq!(result.state, ExternalReviewState::FindingsOpen);
        assert_eq!(result.unresolved_blocking_threads, 1);
        assert!(result.deferred_findings.is_empty());
    }

    #[test]
    fn untrusted_discussion_before_trusted_p2_finding_defers() {
        // 不可信讨论在前（无 badge 语义）+ 受信 P2 finding 在后：
        // 严重度取自首条受信 finding comment 的 badge → deferred
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/copilot-findings-open.json"
        ));
        let mut trusted_reply =
            snapshot.pull_request.review_threads.nodes[0].comments.nodes[0].clone();
        trusted_reply.id = "PRRC-copilot-reply".to_string();
        trusted_reply.url =
            "https://github.com/illusion-tech/laneflow/pull/38#discussion_r3550034352".to_string();
        trusted_reply.body = codex_badge_body("P2", "Deferrable concern.");
        let thread = &mut snapshot.pull_request.review_threads.nodes[0];
        let first = &mut thread.comments.nodes[0];
        first.author = Some(Actor {
            login: "external-contributor".to_string(),
        });
        first.pull_request_review = None;
        first.body = "untrusted discussion without bearing.".to_string();
        thread.comments.nodes.push(trusted_reply);

        let result = evaluate_snapshot(&snapshot);
        assert_eq!(result.state, ExternalReviewState::Pass);
        assert_eq!(result.deferred_findings.len(), 1);
        assert_eq!(result.deferred_findings[0].severity, "P2");
        // deferred URL 指向首条受信 finding comment，而非 thread 首条 comment
        assert_eq!(
            result.deferred_findings[0].url,
            "https://github.com/illusion-tech/laneflow/pull/38#discussion_r3550034352"
        );
    }

    #[test]
    fn deferred_review_does_not_mask_blocking_finding_rereview() {
        // E（r3820473548）：较早 blocking finding（thread 已 resolve、尚无后续 clean）
        // 之后来一轮仅 P2 的 deferred findings review，不得直接判 Pass。
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/copilot-findings-open.json"
        ));
        let head = snapshot.pull_request.head_ref_oid.clone();
        // PR author 换成非受信 actor，wangzishi 的人工 clean review 才计入受信证据
        snapshot.pull_request.author = Some(Actor {
            login: "contributor".to_string(),
        });
        // 既有 thread 升级为 P1 blocking 且已 resolve（处置完毕，尚无后续 clean）
        let blocking_thread = &mut snapshot.pull_request.review_threads.nodes[0];
        blocking_thread.is_resolved = true;
        blocking_thread.comments.nodes[0].body = codex_badge_body("P1", "Must fix before merge.");
        // 追加更晚的 P2-only deferred findings review 及其关联 thread
        snapshot.pull_request.reviews.nodes.push(Review {
            id: "PRR-copilot-findings-2".to_string(),
            author: Some(Actor {
                login: "copilot-pull-request-reviewer".to_string(),
            }),
            body: "Copilot reviewed 2 files and generated 1 comment.".to_string(),
            state: "COMMENTED".to_string(),
            submitted_at: Some("2026-07-10T08:00:00Z".to_string()),
            url: Some(
                "https://github.com/illusion-tech/laneflow/pull/38#pullrequestreview-4661194999"
                    .to_string(),
            ),
            commit: Some(CommitRef {
                oid: head.clone(),
                tree: None,
            }),
        });
        let mut p2_thread = snapshot.pull_request.review_threads.nodes[0].clone();
        p2_thread.id = "PRRT-copilot-finding-p2".to_string();
        p2_thread.is_resolved = false;
        let p2_comment = &mut p2_thread.comments.nodes[0];
        p2_comment.id = "PRRC-copilot-finding-p2".to_string();
        p2_comment.url =
            "https://github.com/illusion-tech/laneflow/pull/38#discussion_r3550034360".to_string();
        p2_comment.body = codex_badge_body("P2", "Deferrable concern.");
        let p2_review_ref = p2_comment.pull_request_review.as_mut().expect("review ref");
        p2_review_ref.id = "PRR-copilot-findings-2".to_string();
        p2_review_ref.submitted_at = Some("2026-07-10T08:00:00Z".to_string());
        snapshot.pull_request.review_threads.nodes.push(p2_thread);

        // deferred-only 最新 review 不得掩盖 P1 的 clean re-review 要求
        let result = evaluate_snapshot(&snapshot);
        assert_eq!(result.state, ExternalReviewState::AwaitingRereview);
        assert!(result.requires_rereview);
        assert_eq!(result.deferred_findings.len(), 1);

        // blocking finding 之前的更早 clean 不放行
        let human_clean = |id: &str, submitted_at: &str| Review {
            id: id.to_string(),
            author: Some(Actor {
                login: "wangzishi".to_string(),
            }),
            body: String::new(),
            state: "APPROVED".to_string(),
            submitted_at: Some(submitted_at.to_string()),
            url: Some(format!(
                "https://github.com/illusion-tech/laneflow/pull/38#pullrequestreview-{id}"
            )),
            commit: Some(CommitRef {
                oid: head.clone(),
                tree: None,
            }),
        };
        let mut earlier_clean = snapshot.clone();
        earlier_clean
            .pull_request
            .reviews
            .nodes
            .insert(0, human_clean("early", "2026-07-08T08:00:00Z"));
        assert_eq!(
            evaluate_snapshot(&earlier_clean).state,
            ExternalReviewState::AwaitingRereview
        );

        // 严格晚于 blocking finding（早于 deferred review）的 clean → 放行
        let mut mid_clean = snapshot.clone();
        mid_clean
            .pull_request
            .reviews
            .nodes
            .push(human_clean("mid", "2026-07-09T12:00:00Z"));
        assert_eq!(
            evaluate_snapshot(&mid_clean).state,
            ExternalReviewState::Pass
        );

        // 严格晚于 deferred review 的 clean → 放行（既有 clean_after_finding 路径）
        let mut late_clean = snapshot;
        late_clean
            .pull_request
            .reviews
            .nodes
            .push(human_clean("late", "2026-07-11T08:00:00Z"));
        assert_eq!(
            evaluate_snapshot(&late_clean).state,
            ExternalReviewState::Pass
        );
    }

    #[test]
    fn round_cap_applies_when_deferred_review_masks_blocking_round() {
        // F1：4 个不同 finding heads（> MAX_REVIEW_ROUNDS），current-head 上较早
        // blocking finding 已 resolve 但无后续 clean，最新 findings review 仅 deferred
        // → 状态本将落 AwaitingRereview，合法 round-cap record（空 remaining）应生效收口。
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/copilot-findings-open.json"
        ));
        let head = snapshot.pull_request.head_ref_oid.clone();
        // PR author 换成非受信 actor，wangzishi 的人工 review 才计入受信证据
        snapshot.pull_request.author = Some(Actor {
            login: "contributor".to_string(),
        });
        // current-head blocking finding（thread 已 resolve，无后续 clean）
        let blocking_thread = &mut snapshot.pull_request.review_threads.nodes[0];
        blocking_thread.is_resolved = true;
        blocking_thread.comments.nodes[0].body = codex_badge_body("P1", "Must fix before merge.");
        // 3 个更早 old-head 上的 unthreaded findings，凑足 4 个不同 finding heads
        for round in 1..=3u32 {
            snapshot.pull_request.reviews.nodes.push(Review {
                id: format!("PRR-human-changes-r{round}"),
                author: Some(Actor {
                    login: "wangzishi".to_string(),
                }),
                body: String::new(),
                state: "CHANGES_REQUESTED".to_string(),
                submitted_at: Some(format!("2026-07-0{round}T08:00:00Z")),
                url: Some(format!(
                    "https://github.com/illusion-tech/laneflow/pull/38#pullrequestreview-r{round}"
                )),
                commit: Some(CommitRef {
                    oid: format!("{round:040x}"),
                    tree: None,
                }),
            });
        }
        // 更晚的 P2-only deferred findings review 及其关联 thread
        snapshot.pull_request.reviews.nodes.push(Review {
            id: "PRR-copilot-findings-2".to_string(),
            author: Some(Actor {
                login: "copilot-pull-request-reviewer".to_string(),
            }),
            body: "Copilot reviewed 2 files and generated 1 comment.".to_string(),
            state: "COMMENTED".to_string(),
            submitted_at: Some("2026-07-10T08:00:00Z".to_string()),
            url: Some(
                "https://github.com/illusion-tech/laneflow/pull/38#pullrequestreview-4661194999"
                    .to_string(),
            ),
            commit: Some(CommitRef {
                oid: head.clone(),
                tree: None,
            }),
        });
        let mut p2_thread = snapshot.pull_request.review_threads.nodes[0].clone();
        p2_thread.id = "PRRT-copilot-finding-p2".to_string();
        p2_thread.is_resolved = false;
        let p2_comment = &mut p2_thread.comments.nodes[0];
        p2_comment.id = "PRRC-copilot-finding-p2".to_string();
        p2_comment.url =
            "https://github.com/illusion-tech/laneflow/pull/38#discussion_r3550034360".to_string();
        p2_comment.body = codex_badge_body("P2", "Deferrable concern.");
        let p2_review_ref = p2_comment.pull_request_review.as_mut().expect("review ref");
        p2_review_ref.id = "PRR-copilot-findings-2".to_string();
        p2_review_ref.submitted_at = Some("2026-07-10T08:00:00Z".to_string());
        snapshot.pull_request.review_threads.nodes.push(p2_thread);

        // 无 round-cap record：deferred 掩盖下仍落 AwaitingRereview
        let result = evaluate_snapshot(&snapshot);
        assert_eq!(result.review_rounds, 4);
        assert_eq!(result.state, ExternalReviewState::AwaitingRereview);
        assert!(result.round_cap.is_none());

        // 合法 round-cap record（remaining 为空，与实测未闭环集合一致）→ 生效收口
        snapshot.round_cap = Some(RoundCapInput {
            current_head_oid: head.clone(),
            round_count: 4,
            remaining_finding_urls: Vec::new(),
        });
        let result = evaluate_snapshot(&snapshot);
        assert_eq!(result.state, ExternalReviewState::Pass);
        let applied = result.round_cap.expect("round-cap applied");
        assert_eq!(applied.rounds, 4);
        assert!(applied.remaining_finding_urls.is_empty());
    }

    #[test]
    fn round_cap_completion_time_uses_newest_evidence() {
        // G3：unresolved blocking finding + 严格更晚的 current-head clean + 合法
        // round-cap record → 收口 Pass 的 completion_time 取更晚的 clean 时间
        //（Owner 决策不得早于最新 review 活动）
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/copilot-findings-open.json"
        ));
        let head = snapshot.pull_request.head_ref_oid.clone();
        // PR author 换成非受信 actor，wangzishi 的人工 review 才计入受信证据
        snapshot.pull_request.author = Some(Actor {
            login: "contributor".to_string(),
        });
        // 3 个更早 old-head 上的 unthreaded findings，凑足 4 个不同 finding heads
        for round in 1..=3u32 {
            snapshot.pull_request.reviews.nodes.push(Review {
                id: format!("PRR-human-changes-r{round}"),
                author: Some(Actor {
                    login: "wangzishi".to_string(),
                }),
                body: String::new(),
                state: "CHANGES_REQUESTED".to_string(),
                submitted_at: Some(format!("2026-07-0{round}T08:00:00Z")),
                url: Some(format!(
                    "https://github.com/illusion-tech/laneflow/pull/38#pullrequestreview-r{round}"
                )),
                commit: Some(CommitRef {
                    oid: format!("{round:040x}"),
                    tree: None,
                }),
            });
        }
        // 严格更晚的 current-head 人工 clean
        snapshot.pull_request.reviews.nodes.push(Review {
            id: "PRR-human-clean".to_string(),
            author: Some(Actor {
                login: "wangzishi".to_string(),
            }),
            body: String::new(),
            state: "APPROVED".to_string(),
            submitted_at: Some("2026-07-12T08:00:00Z".to_string()),
            url: Some(
                "https://github.com/illusion-tech/laneflow/pull/38#pullrequestreview-clean"
                    .to_string(),
            ),
            commit: Some(CommitRef {
                oid: head.clone(),
                tree: None,
            }),
        });
        // fixture 自带 current-head unresolved blocking thread（finding @ 07-09），
        // record 列出其 URL（与实测未闭环集合一致）
        snapshot.round_cap = Some(RoundCapInput {
            current_head_oid: head.clone(),
            round_count: 4,
            remaining_finding_urls: vec![
                "https://github.com/illusion-tech/laneflow/pull/38#discussion_r3550034350"
                    .to_string(),
            ],
        });
        let result = evaluate_snapshot(&snapshot);
        assert_eq!(result.state, ExternalReviewState::Pass);
        assert_eq!(result.review_rounds, 4);
        assert_eq!(
            result.completion_time.as_deref(),
            Some("2026-07-12T08:00:00Z")
        );
    }

    #[test]
    fn counts_review_rounds_by_distinct_finding_head_oids() {
        // 无 findings → 0 轮
        let clean = evaluate_snapshot(&fixture(include_str!(
            "../fixtures/external-review/codex-clean.json"
        )));
        assert_eq!(clean.review_rounds, 0);
        // 单 finding thread 单 head → 1 轮
        let one = evaluate_snapshot(&fixture(include_str!(
            "../fixtures/external-review/copilot-findings-open.json"
        )));
        assert_eq!(one.review_rounds, 1);
        // 两个 finding thread 分布在两个不同 head → 2 轮
        let two = evaluate_snapshot(&fixture(include_str!(
            "../fixtures/external-review/history-pr-232-final.json"
        )));
        assert_eq!(two.review_rounds, 2);
    }

    /// 4 个不同 head OID 上各有未闭环 blocking finding 的 snapshot（无 badge，
    /// 全 blocking）：current head 1 轮 + 追加 3 轮。
    fn four_round_findings_snapshot() -> ExternalReviewSnapshot {
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/copilot-findings-open.json"
        ));
        for round in 2..=4u32 {
            let oid = format!("{round:040x}");
            let submitted = format!("2026-07-09T08:0{round}:32Z");
            let mut review = snapshot.pull_request.reviews.nodes[0].clone();
            review.id = format!("PRR-copilot-findings-r{round}");
            review.submitted_at = Some(submitted.clone());
            review.url = Some(format!(
                "https://github.com/illusion-tech/laneflow/pull/38#pullrequestreview-r{round}"
            ));
            review.commit = Some(CommitRef {
                oid: oid.clone(),
                tree: None,
            });
            snapshot.pull_request.reviews.nodes.push(review);

            let mut thread = snapshot.pull_request.review_threads.nodes[0].clone();
            thread.id = format!("PRRT-copilot-finding-r{round}");
            let comment = &mut thread.comments.nodes[0];
            comment.id = format!("PRRC-copilot-finding-r{round}");
            comment.url =
                format!("https://github.com/illusion-tech/laneflow/pull/38#discussion_r{round}");
            comment.created_at = submitted.clone();
            comment.updated_at = submitted.clone();
            let reference = comment
                .pull_request_review
                .as_mut()
                .expect("fixture review reference");
            reference.id = format!("PRR-copilot-findings-r{round}");
            reference.submitted_at = Some(submitted);
            reference.commit = Some(CommitRef { oid, tree: None });
            snapshot.pull_request.review_threads.nodes.push(thread);
        }
        snapshot
    }

    fn four_round_finding_urls() -> Vec<String> {
        vec![
            "https://github.com/illusion-tech/laneflow/pull/38#discussion_r3550034350".to_string(),
            "https://github.com/illusion-tech/laneflow/pull/38#discussion_r2".to_string(),
            "https://github.com/illusion-tech/laneflow/pull/38#discussion_r3".to_string(),
            "https://github.com/illusion-tech/laneflow/pull/38#discussion_r4".to_string(),
        ]
    }

    #[test]
    fn round_cap_record_closes_over_cap_rounds_without_disguise() {
        // 基线：超过 3 轮且无 record → 保持 FindingsOpen，行为不变
        let baseline = evaluate_snapshot(&four_round_findings_snapshot());
        assert_eq!(baseline.state, ExternalReviewState::FindingsOpen);
        assert_eq!(baseline.review_rounds, 4);
        assert!(baseline.round_cap.is_none());

        // valid record：head/roundCount/遗留 URL 集合精确匹配 → Pass + round_cap 披露
        let mut snapshot = four_round_findings_snapshot();
        snapshot.round_cap = Some(RoundCapInput {
            current_head_oid: snapshot.pull_request.head_ref_oid.clone(),
            round_count: 4,
            remaining_finding_urls: four_round_finding_urls(),
        });
        let result = evaluate_snapshot(&snapshot);
        assert_eq!(result.state, ExternalReviewState::Pass);
        assert!(!result.requires_rereview);
        // round-cap pass 仍携带 completion evidence（gate_evidence 对非 Waived pass
        // 强制要求 completion time）：取最新 current-head 证据的 submitted_at
        assert_eq!(
            result.completion_time.as_deref(),
            Some("2026-07-09T08:18:32Z")
        );
        let cap = result.round_cap.as_ref().expect("round cap applied");
        assert_eq!(cap.rounds, 4);
        assert_eq!(cap.remaining_finding_urls.len(), 4);
        // check output 显式列出轮数与遗留 findings（不伪装 clean pass）
        let payload = build_check_run_payload(
            &result,
            "https://github.com/illusion-tech/laneflow/actions/runs/1",
            "external-review:test".to_string(),
        );
        assert_eq!(payload.conclusion, "success");
        assert!(payload.output.summary.contains("rounds=4"));
        assert!(payload.output.text.contains("Review round cap"));
        for url in &cap.remaining_finding_urls {
            assert!(payload.output.text.contains(url.as_str()));
        }

        // AwaitingRereview 形态（findings 已 resolve 但缺 clean re-review）同样可被
        // 收口，此时未闭环 blocking URL 集合为空
        let mut resolved = four_round_findings_snapshot();
        for thread in &mut resolved.pull_request.review_threads.nodes {
            thread.is_resolved = true;
        }
        let resolved_baseline = evaluate_snapshot(&resolved);
        assert_eq!(
            resolved_baseline.state,
            ExternalReviewState::AwaitingRereview
        );
        resolved.round_cap = Some(RoundCapInput {
            current_head_oid: resolved.pull_request.head_ref_oid.clone(),
            round_count: 4,
            remaining_finding_urls: Vec::new(),
        });
        let result = evaluate_snapshot(&resolved);
        assert_eq!(result.state, ExternalReviewState::Pass);
        // AwaitingRereview 形态同样携带 completion evidence（最新 current-head finding）
        assert_eq!(
            result.completion_time.as_deref(),
            Some("2026-07-09T08:18:32Z")
        );
        let cap = result.round_cap.as_ref().expect("round cap applied");
        assert_eq!(cap.rounds, 4);
        assert!(cap.remaining_finding_urls.is_empty());
    }

    #[test]
    fn round_cap_record_fails_closed_on_any_mismatch() {
        // head 不匹配
        let mut wrong_head = four_round_findings_snapshot();
        wrong_head.round_cap = Some(RoundCapInput {
            current_head_oid: "cccccccccccccccccccccccccccccccccccccccc".to_string(),
            round_count: 4,
            remaining_finding_urls: four_round_finding_urls(),
        });
        let result = evaluate_snapshot(&wrong_head);
        assert_eq!(result.state, ExternalReviewState::ProviderError);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("currentHeadOid 与 PR current head 不一致"))
        );
        assert!(result.round_cap.is_none());

        // roundCount 与实测轮数不等
        let mut wrong_count = four_round_findings_snapshot();
        wrong_count.round_cap = Some(RoundCapInput {
            current_head_oid: wrong_count.pull_request.head_ref_oid.clone(),
            round_count: 5,
            remaining_finding_urls: four_round_finding_urls(),
        });
        let result = evaluate_snapshot(&wrong_count);
        assert_eq!(result.state, ExternalReviewState::ProviderError);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("roundCount 与 evaluator 实测轮数不一致"))
        );

        // 遗留 URL 集合与实测未闭环 blocking findings 不等
        let mut wrong_urls = four_round_findings_snapshot();
        wrong_urls.round_cap = Some(RoundCapInput {
            current_head_oid: wrong_urls.pull_request.head_ref_oid.clone(),
            round_count: 4,
            remaining_finding_urls: four_round_finding_urls()[..3].to_vec(),
        });
        let result = evaluate_snapshot(&wrong_urls);
        assert_eq!(result.state, ExternalReviewState::ProviderError);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("remainingFindingUrls"))
        );

        // 轮数未超上限却提供 record（该 fixture 实测 1 轮）
        let mut premature = fixture(include_str!(
            "../fixtures/external-review/copilot-findings-open.json"
        ));
        premature.round_cap = Some(RoundCapInput {
            current_head_oid: premature.pull_request.head_ref_oid.clone(),
            round_count: 1,
            remaining_finding_urls: vec![
                "https://github.com/illusion-tech/laneflow/pull/38#discussion_r3550034350"
                    .to_string(),
            ],
        });
        let result = evaluate_snapshot(&premature);
        assert_eq!(result.state, ExternalReviewState::ProviderError);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("仅在 review 轮数超过 3 时适用"))
        );
    }

    /// 4 个不同 head OID 上的纯 unthreaded（review 级人工 CHANGES_REQUESTED）findings
    /// snapshot：无 review thread，round 4 落在 current head
    fn four_unthreaded_rounds_snapshot() -> ExternalReviewSnapshot {
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/copilot-findings-open.json"
        ));
        // PR author 换成非受信 actor，wangzishi 的人工 review 才计入受信证据
        snapshot.pull_request.author = Some(Actor {
            login: "contributor".to_string(),
        });
        snapshot.pull_request.review_threads.nodes.clear();
        let base_review = snapshot.pull_request.reviews.nodes[0].clone();
        snapshot.pull_request.reviews.nodes.clear();
        for round in 1..=4u32 {
            let mut review = base_review.clone();
            review.id = format!("PRR-human-changes-r{round}");
            review.author = Some(Actor {
                login: "wangzishi".to_string(),
            });
            review.body = String::new();
            review.state = "CHANGES_REQUESTED".to_string();
            review.submitted_at = Some(format!("2026-07-09T08:0{round}:32Z"));
            review.url = Some(format!(
                "https://github.com/illusion-tech/laneflow/pull/38#pullrequestreview-r{round}"
            ));
            review.commit = Some(CommitRef {
                oid: if round == 4 {
                    snapshot.pull_request.head_ref_oid.clone()
                } else {
                    format!("{round:040x}")
                },
                tree: None,
            });
            snapshot.pull_request.reviews.nodes.push(review);
        }
        snapshot
    }

    const UNTHREADED_CURRENT_FINDING_URL: &str =
        "https://github.com/illusion-tech/laneflow/pull/38#pullrequestreview-r4";

    #[test]
    fn unthreaded_findings_count_rounds_and_blocking_set() {
        let result = evaluate_snapshot(&four_unthreaded_rounds_snapshot());
        // 纯 unthreaded findings 同样按不同 head OID 计轮
        assert_eq!(result.review_rounds, 4);
        assert_eq!(result.unresolved_blocking_threads, 0);
        // current-head unthreaded finding 未被更晚 clean supersede → 计入未闭环 blocking 集合
        assert_eq!(result.unresolved_blocking_findings.len(), 1);
        assert_eq!(
            result.unresolved_blocking_findings[0].url,
            UNTHREADED_CURRENT_FINDING_URL
        );
        assert_eq!(result.state, ExternalReviewState::AwaitingRereview);

        // 严格更晚的 current-head clean supersede 后不再计入
        let mut superseded = four_unthreaded_rounds_snapshot();
        let head = superseded.pull_request.head_ref_oid.clone();
        superseded.pull_request.reviews.nodes.push(Review {
            id: "PRR-human-clean".to_string(),
            author: Some(Actor {
                login: "wangzishi".to_string(),
            }),
            body: String::new(),
            state: "APPROVED".to_string(),
            submitted_at: Some("2026-07-09T09:00:00Z".to_string()),
            url: Some(
                "https://github.com/illusion-tech/laneflow/pull/38#pullrequestreview-clean"
                    .to_string(),
            ),
            commit: Some(CommitRef {
                oid: head,
                tree: None,
            }),
        });
        let result = evaluate_snapshot(&superseded);
        assert_eq!(result.state, ExternalReviewState::Pass);
        assert!(result.unresolved_blocking_findings.is_empty());
    }

    #[test]
    fn round_cap_must_list_unthreaded_blocking_findings() {
        // current-head unthreaded finding 未列入 record → 实测不一致 fail-closed
        let mut snapshot = four_unthreaded_rounds_snapshot();
        snapshot.round_cap = Some(RoundCapInput {
            current_head_oid: snapshot.pull_request.head_ref_oid.clone(),
            round_count: 4,
            remaining_finding_urls: Vec::new(),
        });
        let result = evaluate_snapshot(&snapshot);
        assert_eq!(result.state, ExternalReviewState::ProviderError);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("remainingFindingUrls"))
        );
        assert!(result.round_cap.is_none());

        // record 列出该 review URL 后生效收口
        let mut snapshot = four_unthreaded_rounds_snapshot();
        snapshot.round_cap = Some(RoundCapInput {
            current_head_oid: snapshot.pull_request.head_ref_oid.clone(),
            round_count: 4,
            remaining_finding_urls: vec![UNTHREADED_CURRENT_FINDING_URL.to_string()],
        });
        let result = evaluate_snapshot(&snapshot);
        assert_eq!(result.state, ExternalReviewState::Pass);
        let cap = result.round_cap.as_ref().expect("round cap applied");
        assert_eq!(cap.rounds, 4);
        assert_eq!(
            cap.remaining_finding_urls,
            vec![UNTHREADED_CURRENT_FINDING_URL.to_string()]
        );
    }

    #[test]
    fn pre_activation_replay_disables_deferred_semantics() {
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/copilot-findings-open.json"
        ));
        snapshot.pull_request.review_threads.nodes[0].comments.nodes[0].body =
            codex_badge_body("P2", "Require closingIssuesReferences.");
        // deferred 语义激活：同一 fixture P2 → deferred，不阻断
        let active = evaluate_snapshot_with_policy(&snapshot, true);
        assert_eq!(active.state, ExternalReviewState::Pass);
        assert_eq!(active.deferred_findings.len(), 1);
        // pre-activation replay：不分级，受信 finding 一律 blocking
        let inactive = evaluate_snapshot_with_policy(&snapshot, false);
        assert_eq!(inactive.state, ExternalReviewState::FindingsOpen);
        assert_eq!(inactive.unresolved_blocking_threads, 1);
        assert!(inactive.deferred_findings.is_empty());

        // pre-activation replay 中 round-cap input 不适用（fail-closed）
        let mut with_cap = snapshot.clone();
        with_cap.round_cap = Some(RoundCapInput {
            current_head_oid: with_cap.pull_request.head_ref_oid.clone(),
            round_count: 1,
            remaining_finding_urls: vec![
                "https://github.com/illusion-tech/laneflow/pull/38#discussion_r3550034350"
                    .to_string(),
            ],
        });
        let result = evaluate_snapshot_with_policy(&with_cap, false);
        assert_eq!(result.state, ExternalReviewState::ProviderError);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("deferred 语义激活前的 replay 中不适用"))
        );
        assert!(result.round_cap.is_none());
    }

    /// commits(last:100) 节点构造（oid→tree 映射来源）
    fn commit_node(oid: &str, tree: Option<&str>) -> PullRequestCommit {
        PullRequestCommit {
            commit: CommitMetadata {
                oid: oid.to_string(),
                committed_date: "2026-07-24T02:00:00Z".to_string(),
                url: format!("https://github.com/illusion-tech/laneflow/commit/{oid}"),
                author: None,
                tree: tree.map(|oid| TreeRef {
                    oid: oid.to_string(),
                }),
                message: None,
            },
        }
    }

    #[test]
    fn tree_equivalent_head_inherits_review_outcome_symmetrically() {
        let tree_oid = "dddddddddddddddddddddddddddddddddddddddd";
        let old_head = "cccccccccccccccccccccccccccccccccccccccc";
        let commit_ref = |oid: &str, tree: Option<&str>| CommitRef {
            oid: oid.to_string(),
            tree: tree.map(|oid| TreeRef {
                oid: oid.to_string(),
            }),
        };
        // current head 的 tree 由同一 snapshot 的 commits(last:100) 映射提供
        let with_head_tree = |snapshot: &mut ExternalReviewSnapshot| {
            let head = snapshot.pull_request.head_ref_oid.clone();
            snapshot
                .pull_request
                .commits
                .nodes
                .push(commit_node(&head, Some(tree_oid)));
        };

        // clean 继承：老 head 的 APPROVED review 与 current head tree 相等 → Pass
        let mut clean = fixture(include_str!(
            "../fixtures/external-review/human-approved.json"
        ));
        clean.pull_request.reviews.nodes[0].commit = Some(commit_ref(old_head, Some(tree_oid)));
        with_head_tree(&mut clean);
        let clean_result = evaluate_snapshot(&clean);
        assert_eq!(clean_result.state, ExternalReviewState::Pass);
        // tree 继承命中：可见 evidence 行尾部标注，reference 定义行保持纯 URL
        let payload = build_check_run_payload(
            &clean_result,
            "https://github.com/illusion-tech/laneflow/actions/runs/1",
            "external-review:test".to_string(),
        );
        assert!(
            payload
                .output
                .text
                .contains("[evidence-1]（tree-equivalent 继承）")
        );
        let definition = payload
            .output
            .text
            .lines()
            .find(|line| line.starts_with("[evidence-1]:"))
            .expect("reference definition line");
        assert_eq!(
            definition,
            format!("[evidence-1]: {}", clean_result.evidence[0].evidence_url)
        );

        // findings 继承（对称）：老 head 的未闭环 blocking finding，tree 相等 → FindingsOpen
        let mut findings = fixture(include_str!(
            "../fixtures/external-review/copilot-findings-open.json"
        ));
        findings.pull_request.reviews.nodes[0].commit = Some(commit_ref(old_head, Some(tree_oid)));
        findings.pull_request.review_threads.nodes[0].comments.nodes[0]
            .pull_request_review
            .as_mut()
            .expect("fixture review reference")
            .commit = Some(commit_ref(old_head, Some(tree_oid)));
        with_head_tree(&mut findings);
        assert_eq!(
            evaluate_snapshot(&findings).state,
            ExternalReviewState::FindingsOpen
        );

        // tree 缺失 → 回退纯 OID 判定（旧行为 Stale）
        let mut legacy = fixture(include_str!(
            "../fixtures/external-review/human-approved.json"
        ));
        legacy.pull_request.reviews.nodes[0].commit = Some(commit_ref(old_head, None));
        assert_eq!(evaluate_snapshot(&legacy).state, ExternalReviewState::Stale);

        // current head 不在 commits 映射内（截断/缺失）→ fail-closed 不继承
        let mut no_head_commit = clean.clone();
        no_head_commit.pull_request.commits.nodes.clear();
        assert_eq!(
            evaluate_snapshot(&no_head_commit).state,
            ExternalReviewState::Stale
        );

        // tree 不相等 → 不继承
        let mut mismatched = clean.clone();
        mismatched.pull_request.commits.nodes[0].commit.tree = Some(TreeRef {
            oid: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string(),
        });
        assert_eq!(
            evaluate_snapshot(&mismatched).state,
            ExternalReviewState::Stale
        );
    }

    #[test]
    fn codex_clean_comment_inherits_via_commits_tree_map() {
        let tree_oid = "dddddddddddddddddddddddddddddddddddddddd";
        let old_head = "cccccccccccccccccccccccccccccccccccccccc";
        // Codex clean comment 的 Reviewed commit 指向老 head（短 SHA，无 tree 来源）
        let codex_clean_on_old_head = |snapshot: &mut ExternalReviewSnapshot| {
            snapshot.pull_request.comments.nodes[0].body = format!(
                "Codex Review: Didn't find any major issues. Keep them coming!\n\n**Reviewed commit:** `{}`",
                &old_head[..12]
            );
        };

        // 老 head 与 current head 同 tree：clean comment 证据经 commits 映射补齐 tree → 继承 Pass
        let mut snapshot = fixture(include_str!("../fixtures/external-review/codex-clean.json"));
        codex_clean_on_old_head(&mut snapshot);
        let head = snapshot.pull_request.head_ref_oid.clone();
        snapshot.pull_request.commits.nodes = vec![
            commit_node(old_head, Some(tree_oid)),
            commit_node(&head, Some(tree_oid)),
        ];
        let result = evaluate_snapshot(&snapshot);
        assert_eq!(result.state, ExternalReviewState::Pass);
        assert!(
            result
                .evidence
                .iter()
                .any(|item| item.reviewed_head_tree_oid.as_deref() == Some(tree_oid))
        );

        // 映射缺 reviewed head → 不补 tree → 不继承（fail-closed Stale）
        let mut missing_reviewed =
            fixture(include_str!("../fixtures/external-review/codex-clean.json"));
        codex_clean_on_old_head(&mut missing_reviewed);
        let head = missing_reviewed.pull_request.head_ref_oid.clone();
        missing_reviewed.pull_request.commits.nodes = vec![commit_node(&head, Some(tree_oid))];
        assert_eq!(
            evaluate_snapshot(&missing_reviewed).state,
            ExternalReviewState::Stale
        );

        // commits hasNextPage（>100）且 current head 不在映射内 → 不继承（fail-closed Stale）
        let mut truncated = fixture(include_str!("../fixtures/external-review/codex-clean.json"));
        codex_clean_on_old_head(&mut truncated);
        truncated.pull_request.commits.nodes = vec![commit_node(old_head, Some(tree_oid))];
        truncated.pull_request.commits.page_info.has_next_page = true;
        let result = evaluate_snapshot(&truncated);
        assert_eq!(result.state, ExternalReviewState::Stale);
        assert!(result.current_head_tree_oid.is_none());
    }

    #[test]
    fn orphan_head_clean_inherits_via_resolved_commit_trees() {
        let tree_oid = "dddddddddddddddddddddddddddddddddddddddd";
        let old_head = "cccccccccccccccccccccccccccccccccccccccc";
        // Codex clean comment 的 Reviewed commit 指向已从 PR 历史消失的孤儿 head（短 SHA）
        let mut snapshot = fixture(include_str!("../fixtures/external-review/codex-clean.json"));
        snapshot.pull_request.comments.nodes[0].body = format!(
            "Codex Review: Didn't find any major issues. Keep them coming!\n\n**Reviewed commit:** `{}`",
            &old_head[..12]
        );
        // commits(last:100) 只剩 current head；孤儿 head 的 tree 由追加查询解析提供
        let head = snapshot.pull_request.head_ref_oid.clone();
        snapshot.pull_request.commits.nodes = vec![commit_node(&head, Some(tree_oid))];
        snapshot.resolved_commit_trees =
            BTreeMap::from([(old_head.to_string(), tree_oid.to_string())]);
        let result = evaluate_snapshot(&snapshot);
        assert_eq!(result.state, ExternalReviewState::Pass);
        assert!(
            result
                .evidence
                .iter()
                .any(|item| item.reviewed_head_tree_oid.as_deref() == Some(tree_oid))
        );

        // 追加解析缺该 oid（查询失败形态）→ 不继承（fail-closed Stale）
        let mut unresolved = snapshot.clone();
        unresolved.resolved_commit_trees.clear();
        assert_eq!(
            evaluate_snapshot(&unresolved).state,
            ExternalReviewState::Stale
        );
    }

    #[test]
    fn orphan_oid_collection_is_deduped_resolvable_aware_and_bounded() {
        let tree_oid = "dddddddddddddddddddddddddddddddddddddddd";
        let in_map_head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let orphan_head = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let commits_by_oid = BTreeMap::from([(in_map_head, tree_oid)]);
        let resolved = BTreeMap::new();
        let comment = |body: String| IssueComment {
            id: String::new(),
            author: Some(Actor {
                login: CODEX_ACTOR.to_string(),
            }),
            body,
            created_at: "2026-08-20T00:00:00Z".to_string(),
            updated_at: "2026-08-20T00:00:00Z".to_string(),
            url: String::new(),
        };
        let marker = |oid: &str| {
            format!("Codex Review: Didn't find any major issues.\n\n**Reviewed commit:** `{oid}`")
        };
        let snapshot_with = |comments: Vec<IssueComment>| {
            let mut snapshot =
                fixture(include_str!("../fixtures/external-review/codex-clean.json"));
            snapshot.pull_request.comments.nodes = comments;
            snapshot
        };

        // commits 映射可解析（短 SHA 前缀）→ 不收集；无 marker / 形态不符 → 不收集；
        // 孤儿 oid 去重后只出现一次
        let comments = vec![
            comment(marker(&in_map_head[..12])),
            comment("no marker here".to_string()),
            comment(marker(&orphan_head[..12])),
            comment(marker(&orphan_head[..12])),
        ];
        assert_eq!(
            orphan_reviewed_oids_for_tree_resolution(
                &snapshot_with(comments).pull_request,
                &commits_by_oid,
                &resolved
            ),
            Some(vec![orphan_head[..12].to_string()])
        );

        // 超过 16 个不同孤儿 oid → None（fail-closed 不解析）
        let comments = (0..17u32)
            .map(|index| comment(marker(&format!("{index:040x}"))))
            .collect::<Vec<_>>();
        assert_eq!(
            orphan_reviewed_oids_for_tree_resolution(
                &snapshot_with(comments).pull_request,
                &commits_by_oid,
                &resolved
            ),
            None
        );
    }

    #[test]
    fn orphan_markers_only_collected_from_trusted_clean_comments() {
        // F2：不可信 author / 非 clean 形态 / 创建后被编辑的 comment 里的伪造
        // marker 一律不进入收集（不占 16-OID 上限，不牵引追加 GraphQL 查询）
        let orphan_head = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let commits_by_oid = BTreeMap::new();
        let resolved = BTreeMap::new();
        let codex_comment = |body: String| IssueComment {
            id: String::new(),
            author: Some(Actor {
                login: CODEX_ACTOR.to_string(),
            }),
            body,
            created_at: "2026-08-20T00:00:00Z".to_string(),
            updated_at: "2026-08-20T00:00:00Z".to_string(),
            url: String::new(),
        };
        let marker = |oid: &str| {
            format!("Codex Review: Didn't find any major issues.\n\n**Reviewed commit:** `{oid}`")
        };
        let snapshot_with = |comments: Vec<IssueComment>| {
            let mut snapshot =
                fixture(include_str!("../fixtures/external-review/codex-clean.json"));
            snapshot.pull_request.comments.nodes = comments;
            snapshot
        };

        // 仅受信 clean comment 的孤儿 marker 被收集；三类伪造 marker 各自不收集
        let mut untrusted = codex_comment(marker("cccccccccccc"));
        untrusted.author = Some(Actor {
            login: "external-contributor".to_string(),
        });
        let not_clean_shape = codex_comment(
            "Codex Review: found issues\n\n**Reviewed commit:** `dddddddddddd`".to_string(),
        );
        let mut edited = codex_comment(marker("eeeeeeeeeeee"));
        edited.updated_at = "2026-08-20T00:01:00Z".to_string();
        let comments = vec![
            untrusted,
            not_clean_shape,
            edited,
            codex_comment(marker(&orphan_head[..12])),
        ];
        assert_eq!(
            orphan_reviewed_oids_for_tree_resolution(
                &snapshot_with(comments).pull_request,
                &commits_by_oid,
                &resolved
            ),
            Some(vec![orphan_head[..12].to_string()])
        );

        // 不可信 commenter 投 17 个伪造 marker → 不触发上限（返回空集而非 None）
        let comments = (0..17u32)
            .map(|index| {
                let mut forged = codex_comment(marker(&format!("{index:040x}")));
                forged.author = Some(Actor {
                    login: "external-contributor".to_string(),
                });
                forged
            })
            .collect::<Vec<_>>();
        assert_eq!(
            orphan_reviewed_oids_for_tree_resolution(
                &snapshot_with(comments).pull_request,
                &commits_by_oid,
                &resolved
            ),
            Some(Vec::new())
        );
    }

    #[test]
    fn orphan_oid_collection_includes_validated_binding_bound_head() {
        // F（r3820473559）：SHA-less clean 的 head 只存在于 binding record 的
        // boundHeadOid → 收集；record 校验失败 → 不收集（fail-closed）
        let bound_head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let tree_oid = "dddddddddddddddddddddddddddddddddddddddd";
        let commits_by_oid = BTreeMap::new();
        let resolved = BTreeMap::new();
        let snapshot = fixture(include_str!(
            "../fixtures/external-review/codex-no-sha-bound-clean.json"
        ));
        assert_eq!(
            orphan_reviewed_oids_for_tree_resolution(
                &snapshot.pull_request,
                &commits_by_oid,
                &resolved
            ),
            Some(vec![bound_head.to_string()])
        );

        // binding record 创建后被编辑 → 校验失败 → 不收集
        let mut edited = snapshot.clone();
        edited.pull_request.comments.nodes[2].updated_at = "2026-08-19T01:12:00Z".to_string();
        assert_eq!(
            orphan_reviewed_oids_for_tree_resolution(
                &edited.pull_request,
                &commits_by_oid,
                &resolved
            ),
            Some(Vec::new())
        );

        // 已解析结果可解析该 head → 不重复收集
        let resolved = BTreeMap::from([(bound_head.to_string(), tree_oid.to_string())]);
        assert_eq!(
            orphan_reviewed_oids_for_tree_resolution(
                &snapshot.pull_request,
                &commits_by_oid,
                &resolved
            ),
            Some(Vec::new())
        );
    }

    #[test]
    fn orphan_bound_head_clean_inherits_via_resolved_commit_trees() {
        // F：rebase 后 SHA-less clean 的绑定 head 成为孤儿（只存在于 binding
        // record），其 tree 经追加解析与 current head tree 相等 → 继承 Pass；
        // 缺 tree → 不继承（fail-closed Stale）
        let tree_oid = "dddddddddddddddddddddddddddddddddddddddd";
        let bound_head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let new_head = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/codex-no-sha-bound-clean.json"
        ));
        snapshot.pull_request.head_ref_oid = new_head.to_string();
        snapshot.pull_request.commits.nodes = vec![commit_node(new_head, Some(tree_oid))];
        snapshot.resolved_commit_trees =
            BTreeMap::from([(bound_head.to_string(), tree_oid.to_string())]);
        let result = evaluate_snapshot(&snapshot);
        assert_eq!(result.state, ExternalReviewState::Pass);
        assert!(
            result
                .evidence
                .iter()
                .any(|item| item.reviewed_head_tree_oid.as_deref() == Some(tree_oid))
        );

        let mut unresolved = snapshot.clone();
        unresolved.resolved_commit_trees.clear();
        assert_eq!(
            evaluate_snapshot(&unresolved).state,
            ExternalReviewState::Stale
        );
    }

    #[test]
    fn tree_equivalent_outdated_thread_stays_blocking() {
        let tree_oid = "dddddddddddddddddddddddddddddddddddddddd";
        let old_head = "cccccccccccccccccccccccccccccccccccccccc";
        let commit_ref = |oid: &str, tree: Option<&str>| CommitRef {
            oid: oid.to_string(),
            tree: tree.map(|oid| TreeRef {
                oid: oid.to_string(),
            }),
        };
        // content-equivalent force-push：旧 thread 被标 isOutdated，但首条受信 finding
        // 关联 review 的 commit tree 与 current head tree 逐字节相等 → 回活仍 blocking
        let revived_snapshot = |reference_tree: Option<&str>| {
            let mut snapshot = fixture(include_str!(
                "../fixtures/external-review/copilot-findings-open.json"
            ));
            let thread = &mut snapshot.pull_request.review_threads.nodes[0];
            thread.is_outdated = true;
            thread.comments.nodes[0]
                .pull_request_review
                .as_mut()
                .expect("fixture review reference")
                .commit = Some(commit_ref(old_head, reference_tree));
            snapshot.pull_request.reviews.nodes[0].commit =
                Some(commit_ref(old_head, Some(tree_oid)));
            let head = snapshot.pull_request.head_ref_oid.clone();
            snapshot
                .pull_request
                .commits
                .nodes
                .push(commit_node(&head, Some(tree_oid)));
            snapshot
        };

        let result = evaluate_snapshot(&revived_snapshot(Some(tree_oid)));
        assert_eq!(result.state, ExternalReviewState::FindingsOpen);
        assert_eq!(result.unresolved_blocking_threads, 1);
        assert_eq!(result.unresolved_blocking_findings.len(), 1);
        assert_eq!(
            result.unresolved_blocking_findings[0].thread_id,
            "PRRT-copilot-finding"
        );

        // tree 不等 → 维持丢弃（fail-closed）：finding 只欠 clean re-review
        let result = evaluate_snapshot(&revived_snapshot(Some(
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        )));
        assert_eq!(result.state, ExternalReviewState::AwaitingRereview);
        assert_eq!(result.unresolved_blocking_threads, 0);
        assert!(result.unresolved_blocking_findings.is_empty());
    }

    #[test]
    fn round_cap_must_list_revived_outdated_blocking_findings() {
        // 4 轮 outdated 但 tree 等价的 thread 全部回活 → 空 remainingFindings 的 record
        // 与实测未闭环 blocking 集合不符 → fail-closed ProviderError
        let tree_oid = "dddddddddddddddddddddddddddddddddddddddd";
        let mut snapshot = four_round_findings_snapshot();
        for thread in &mut snapshot.pull_request.review_threads.nodes {
            thread.is_outdated = true;
            let reference = thread.comments.nodes[0]
                .pull_request_review
                .as_mut()
                .expect("fixture review reference");
            reference.commit.as_mut().expect("reference commit").tree = Some(TreeRef {
                oid: tree_oid.to_string(),
            });
        }
        let head = snapshot.pull_request.head_ref_oid.clone();
        snapshot
            .pull_request
            .commits
            .nodes
            .push(commit_node(&head, Some(tree_oid)));
        let baseline = evaluate_snapshot(&snapshot);
        assert_eq!(baseline.state, ExternalReviewState::FindingsOpen);
        assert_eq!(baseline.unresolved_blocking_findings.len(), 4);

        snapshot.round_cap = Some(RoundCapInput {
            current_head_oid: snapshot.pull_request.head_ref_oid.clone(),
            round_count: 4,
            remaining_finding_urls: Vec::new(),
        });
        let result = evaluate_snapshot(&snapshot);
        assert_eq!(result.state, ExternalReviewState::ProviderError);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("remainingFindingUrls"))
        );
        assert!(result.round_cap.is_none());
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
    fn sweep_skips_clean_with_invalid_evidence_fields() {
        // 与 evaluate_snapshot 闸门一致：createdAt/URL 无效的 clean 不进入 sweep
        // 候选，publisher 不会为其发布 record 字段必判无效的 malformed record
        // （malformed record 会让后续 run 在收集阶段持续 fail-closed 中止）。
        let mut snapshot = fixture(include_str!(
            "../fixtures/external-review/codex-no-sha-same-head-overlap-sequential.json"
        ));
        {
            let clean = snapshot
                .pull_request
                .comments
                .nodes
                .iter_mut()
                .find(|comment| comment.id == "IC-codex-clean-nosha-2")
                .expect("fixture 含 C2");
            clean.url = "https://example.com/not-a-github-url".to_string();
        }
        let pr = &snapshot.pull_request;
        let mut diagnostics = Vec::new();
        let records = collect_codex_clean_binding_records(pr, &mut diagnostics);
        assert!(diagnostics.is_empty());
        assert_eq!(records.len(), 2);

        // C1 已有 record 时 C2 本是唯一 sweep 候选（对照见
        // sweep_plans_unbound_clean_against_records_from_the_same_run）；
        // URL 无效后候选集合为空，sweep 无工作也不记诊断。
        let mut sweep_diagnostics = Vec::new();
        assert!(plan_next_sweep_binding(pr, &records[..1], &mut sweep_diagnostics).is_none());
        assert!(sweep_diagnostics.is_empty());
    }

    #[test]
    fn sweep_processes_displaced_old_head_clean_before_the_triggering_clean() {
        let snapshot = fixture(include_str!(
            "../fixtures/external-review/codex-no-sha-cross-push-displaced-then-current.json"
        ));
        let pr = &snapshot.pull_request;
        let records: Vec<BoundCleanRecord> = Vec::new();

        // 触发 comment 是 C2（new head），但 sweep 必须先为 C1 绑定旧 marker：
        // 否则 C2 会同时看到新旧 head 的未消费 marker 而歧义 fail-closed。
        let mut sweep_diagnostics = Vec::new();
        let (clean, marker_comment, marker) =
            plan_next_sweep_binding(pr, &records, &mut sweep_diagnostics).expect("C1 应先被计划");
        assert_eq!(clean.id, "IC-codex-clean-nosha-1");
        assert_eq!(marker_comment.id, "IC-marker-1");
        assert!(sweep_diagnostics.is_empty());

        // C1 绑定后，C2 确定性绑定新 head 的 M2。
        let records = vec![BoundCleanRecord {
            record: CodexCleanBindingRecord {
                schema_version: BINDING_RECORD_SCHEMA_VERSION,
                id: "codex-clean-binding-test".to_string(),
                pr: 430,
                clean_comment_id: clean.id.clone(),
                clean_comment_created_at: clean.created_at.clone(),
                clean_comment_url: clean.url.clone(),
                request_marker_id: marker_comment.id.clone(),
                bound_head_oid: marker.request_head_oid.clone(),
                bound_base_oid: marker.request_base_oid.clone(),
                verified_at: "2026-08-19T01:16:00Z".to_string(),
                run_url: "https://github.com/illusion-tech/laneflow/actions/runs/1".to_string(),
            },
            created_at: "2026-08-19T01:16:00Z".to_string(),
            url: "https://github.com/illusion-tech/laneflow/pull/430#issuecomment-110".to_string(),
        }];
        let mut sweep_diagnostics = Vec::new();
        let (clean, marker_comment, _) =
            plan_next_sweep_binding(pr, &records, &mut sweep_diagnostics).expect("C2 应绑定 M2");
        assert_eq!(clean.id, "IC-codex-clean-nosha-2");
        assert_eq!(marker_comment.id, "IC-marker-2");
        assert!(sweep_diagnostics.is_empty());
    }

    #[test]
    fn same_second_binding_records_follow_clean_chronology() {
        let result = evaluate_snapshot(&fixture(include_str!(
            "../fixtures/external-review/codex-no-sha-same-second-records.json"
        )));
        // 两条 record 同秒发布：次序键必须落到引用 clean 的 (created_at, id)，
        // 而非 hash 派生的 record id——fixture 对抗性构造为 R2 的 record id
        // 字典序更小且 comment 顺序更靠前，旧 tie-break 会反转分配顺序。
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
                page_info: CursorPageInfo {
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
