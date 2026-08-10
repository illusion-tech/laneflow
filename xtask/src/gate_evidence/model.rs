//! Gate evidence 的领域参数、GitHub wire model 与结构化记录模型。

// Issue #230 G2-B incremental start record. Older G3 comments remain historical evidence.
pub(super) const EXTERNAL_REVIEW_G3_ACTIVATION: &str = "2026-07-24T15:16:21Z";
// PR #324 merge time. Earlier G3 comments cannot be retroactively required to carry this field.
pub(super) const G3_EVIDENCE_SHADOW_ACTIVATION: &str = "2026-08-06T10:49:21Z";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GateEvidencePhase {
    G3,
    G4,
}

impl GateEvidencePhase {
    pub(super) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "g3" => Ok(Self::G3),
            "g4" => Ok(Self::G4),
            _ => Err(format!(
                "未知 Gate evidence 阶段 `{value}`，应为 `g3` 或 `g4`"
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct GateEvidenceArgs {
    pub(super) phase: GateEvidencePhase,
    pub(super) repo: String,
    pub(super) issue: u64,
    pub(super) delivery_pr: Option<u64>,
    pub(super) related_prs: Vec<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GateEvidencePrRole {
    Delivery,
    Related,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubIssue {
    pub(super) body: String,
    pub(super) state: String,
    #[serde(rename = "projectItems", default)]
    pub(super) project_items: Vec<ProjectItem>,
    pub(super) comments: Vec<GitHubComment>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubIssueListEntry {
    pub(super) body: Option<String>,
    #[serde(default)]
    pub(super) pull_request: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubIssuesEvent {
    pub(super) issue: GitHubIssuesEventIssue,
    #[serde(default)]
    pub(super) changes: GitHubIssuesEventChanges,
    pub(super) repository: GitHubIssuesEventRepository,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubIssuesEventIssue {
    pub(super) number: u64,
    pub(super) body: Option<String>,
    #[serde(default)]
    pub(super) pull_request: Option<serde_json::Value>,
}

#[derive(Debug, Default, serde::Deserialize)]
pub(super) struct GitHubIssuesEventChanges {
    pub(super) body: Option<GitHubIssuesEventBodyChange>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubIssuesEventBodyChange {
    pub(super) from: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubIssuesEventRepository {
    pub(super) full_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub(super) struct GitHubIssueCommentRest {
    pub(super) body: Option<String>,
    #[serde(rename = "created_at")]
    pub(super) created_at: String,
    #[serde(rename = "updated_at")]
    pub(super) updated_at: String,
    #[serde(rename = "issue_url")]
    pub(super) issue_url: String,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubEditTimestampsResponse {
    pub(super) data: GitHubEditTimestampsData,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubEditTimestampsData {
    pub(super) repository: Option<GitHubEditTimestampsRepository>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubEditTimestampsRepository {
    pub(super) target: Option<GitHubEditTimestamps>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubEditTimestamps {
    #[serde(rename = "createdAt")]
    pub(super) created_at: String,
    #[serde(rename = "lastEditedAt")]
    pub(super) last_edited_at: Option<String>,
    #[serde(rename = "updatedAt")]
    pub(super) updated_at: String,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubUserContentEditsResponse {
    pub(super) data: GitHubUserContentEditsData,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubUserContentEditsData {
    pub(super) repository: Option<GitHubUserContentEditsRepository>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubUserContentEditsRepository {
    #[serde(rename = "pullRequest")]
    pub(super) pull_request: Option<GitHubUserContentEditsPullRequest>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubUserContentEditsPullRequest {
    #[serde(rename = "userContentEdits")]
    pub(super) user_content_edits: GitHubUserContentEditConnection,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubUserContentEditConnection {
    #[serde(rename = "pageInfo")]
    pub(super) page_info: GitHubPageInfo,
    pub(super) nodes: Vec<GitHubUserContentEdit>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubPageInfo {
    #[serde(rename = "hasNextPage")]
    pub(super) has_next_page: bool,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubUserContentEdit {
    #[serde(rename = "editedAt")]
    pub(super) edited_at: String,
    pub(super) editor: Option<GitHubActor>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubTimelineItem {
    #[serde(default)]
    pub(super) id: Option<u64>,
    pub(super) event: String,
    #[serde(default)]
    pub(super) created_at: Option<String>,
    #[serde(default)]
    pub(super) updated_at: Option<String>,
    #[serde(default)]
    pub(super) submitted_at: Option<String>,
    #[serde(default)]
    pub(super) committer: Option<GitHubTimelineCommitter>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubTimelineCommitter {
    pub(super) date: String,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum GitHubTimelineTarget {
    PullRequest,
    Issue,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum GitHubEditTarget {
    PullRequest,
    Issue,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubPullRequest {
    pub(super) body: String,
    pub(super) state: String,
    #[serde(rename = "isDraft")]
    pub(super) is_draft: bool,
    #[serde(rename = "createdAt")]
    pub(super) created_at: String,
    #[serde(rename = "mergedAt")]
    pub(super) merged_at: Option<String>,
    #[serde(rename = "closingIssuesReferences")]
    pub(super) closing_issues_references: Vec<IssueReference>,
    #[serde(rename = "projectItems", default)]
    pub(super) project_items: Vec<ProjectItem>,
    pub(super) comments: Vec<GitHubComment>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct ProjectItem {
    pub(super) title: String,
    pub(super) status: Option<ProjectStatus>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct ProjectStatus {
    pub(super) name: String,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct IssueReference {
    pub(super) number: u64,
    pub(super) repository: IssueReferenceRepository,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct IssueReferenceRepository {
    pub(super) name: String,
    pub(super) owner: GitHubActor,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubComment {
    pub(super) url: String,
    pub(super) body: String,
    #[serde(default)]
    pub(super) author: Option<GitHubActor>,
    #[serde(rename = "createdAt")]
    pub(super) created_at: String,
    #[serde(skip)]
    pub(super) updated_at: Option<String>,
    #[serde(rename = "includesCreatedEdit", default)]
    pub(super) includes_created_edit: bool,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GitHubActor {
    pub(super) login: String,
}

pub(super) const G3_COMMENT_FIELDS: &[&str] = &[
    "## G3 合并判断",
    "- Checks：",
    "- 审阅：",
    "- 验证：",
    "- 风险：",
    "- 例外：",
    "- 合并方式：",
    "- Gate 断言：",
];

pub(super) const CURRENT_G3_COMMENT_FIELDS: &[&str] = &[
    "## G3 合并判断",
    "- Gate 结果：",
    "- Rollout phase：",
    "- Current head：",
    "- Checks：",
    "- External Review Gate：",
    "- 审阅：",
    "- Findings disposition / clean re-review：",
    "- Review threads：",
    "- 验证：",
    "- 风险：",
    "- 例外：",
    "- 合并方式：",
    "- Gate 断言：",
];

pub(super) const G3_EVIDENCE_SHADOW_COMMENT_FIELD: &str = "- G3 Evidence Gate Shadow：";

pub(super) const EXTERNAL_REVIEW_WAIVER_START: &str = "<!-- external-review-waiver:v1";
pub(super) const EXTERNAL_REVIEW_WAIVER_END: &str = "-->";
pub(super) const EXTERNAL_REVIEW_WAIVER_MAX_SECONDS: u64 = 24 * 60 * 60;
pub(super) const G3_FULL_SET_RECOVERY_START: &str = "<!-- g3-full-set-recovery:v1";
pub(super) const G3_FULL_SET_RECOVERY_END: &str = "-->";
pub(super) const G3_OWNER_ACTORS: &[&str] = &["wangzishi"];

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct G3FullSetRecoveryRecord {
    pub(super) schema_version: u64,
    pub(super) exception_type: String,
    pub(super) issue: u64,
    pub(super) delivery_pr: u64,
    pub(super) delivery_merged_at: String,
    pub(super) original_related_prs: Vec<u64>,
    pub(super) late_related_prs: Vec<u64>,
    pub(super) reason: String,
    pub(super) evidence_refs: Vec<String>,
    pub(super) risk: String,
    pub(super) acceptance_boundary: String,
    pub(super) follow_up_issue: String,
    pub(super) cleanup_owner: String,
    pub(super) authorized_by: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum G3Result {
    Pass,
    Waived,
    Bootstrap,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct GateWaiverRecord {
    pub(super) schema_version: u64,
    pub(super) id: String,
    pub(super) exception_type: String,
    pub(super) current_head_oid: String,
    pub(super) current_base_oid: String,
    pub(super) reason: String,
    pub(super) evidence_refs: Vec<String>,
    pub(super) risk: String,
    pub(super) acceptance_boundary: String,
    pub(super) expires_at: String,
    pub(super) follow_up_issue: String,
    pub(super) cleanup_owner: String,
    pub(super) authorized_by: String,
}

pub(super) const G4_COMMENT_FIELDS: &[&str] = &[
    "## G4 完成判断",
    "- 合并：",
    "- main CI：",
    "- 验收：",
    "- Project：",
    "- 关系：",
    "- 分支：",
    "- 权限 / bypass：",
    "- Gate 断言：",
];

pub(super) const GATE_ASSERTION_PREFIX: &str = "- Gate 断言：";
