//! GitHub CLI IO、远端对象读取与活动时序校验。

use std::process::Command;

use super::{document::*, model::*};

pub(super) fn gh_issue_comment(
    repo: &str,
    comment_id: u64,
) -> Result<GitHubIssueCommentRest, String> {
    gh_json(&[
        "api".to_string(),
        format!("repos/{repo}/issues/comments/{comment_id}"),
    ])
}

pub(super) fn gh_edit_timestamps(
    repo: &str,
    number: u64,
    target: GitHubEditTarget,
) -> Result<GitHubEditTimestamps, String> {
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| format!("`--repo` 格式不正确：{repo}，应为 `owner/repo`"))?;
    let (query, label) = match target {
        GitHubEditTarget::PullRequest => (
            r#"query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    target: pullRequest(number: $number) { createdAt lastEditedAt updatedAt }
  }
}"#,
            "PR",
        ),
        GitHubEditTarget::Issue => (
            r#"query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    target: issue(number: $number) { createdAt lastEditedAt updatedAt }
  }
}"#,
            "Issue",
        ),
    };
    let response: GitHubEditTimestampsResponse = gh_json(&[
        "api".to_string(),
        "graphql".to_string(),
        "-f".to_string(),
        format!("query={query}"),
        "-F".to_string(),
        format!("owner={owner}"),
        "-F".to_string(),
        format!("name={name}"),
        "-F".to_string(),
        format!("number={number}"),
    ])?;
    let repository = response
        .data
        .repository
        .ok_or("GitHub GraphQL 未返回目标 repository")?;
    repository
        .target
        .ok_or_else(|| format!("GitHub GraphQL 未返回 {label} #{number}"))
}

pub(super) fn gh_pr_user_content_edits(
    repo: &str,
    number: u64,
) -> Result<GitHubUserContentEditConnection, String> {
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| format!("`--repo` 格式不正确：{repo}，应为 `owner/repo`"))?;
    let query = r#"query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      userContentEdits(first: 100) {
        pageInfo { hasNextPage }
        nodes { editedAt editor { login } }
      }
    }
  }
}"#;
    let response: GitHubUserContentEditsResponse = gh_json(&[
        "api".to_string(),
        "graphql".to_string(),
        "-f".to_string(),
        format!("query={query}"),
        "-F".to_string(),
        format!("owner={owner}"),
        "-F".to_string(),
        format!("name={name}"),
        "-F".to_string(),
        format!("number={number}"),
    ])?;
    response
        .data
        .repository
        .ok_or("GitHub GraphQL 未返回目标 repository")?
        .pull_request
        .ok_or_else(|| format!("GitHub GraphQL 未返回 PR #{number}"))
        .map(|pull_request| pull_request.user_content_edits)
}

pub(super) fn gh_issue_timeline(
    repo: &str,
    number: u64,
) -> Result<Vec<GitHubTimelineItem>, String> {
    let pages: Vec<Vec<GitHubTimelineItem>> = gh_json(&[
        "api".to_string(),
        "--paginate".to_string(),
        "--slurp".to_string(),
        "-H".to_string(),
        "Accept: application/vnd.github+json".to_string(),
        format!("repos/{repo}/issues/{number}/timeline?per_page=100"),
    ])?;
    Ok(pages.into_iter().flatten().collect())
}

pub(super) fn validate_g3_evidence_marker_comment(
    marker: &GitHubIssueCommentRest,
    repo: &str,
    pr_number: u64,
) -> Result<(), String> {
    if marker.body.as_deref() != Some("g3-evidence: changed") {
        return Err("G3 evidence marker 正文必须精确为 `g3-evidence: changed`".to_string());
    }
    if marker.created_at != marker.updated_at {
        return Err("G3 evidence marker 在创建后被编辑；必须新增未编辑 marker".to_string());
    }
    let expected_issue_url = format!("https://api.github.com/repos/{repo}/issues/{pr_number}");
    if marker.issue_url != expected_issue_url {
        return Err(format!(
            "G3 evidence marker 不属于当前 PR #{pr_number}：{}",
            marker.issue_url
        ));
    }
    if parse_utc_timestamp_seconds(&marker.created_at).is_none() {
        return Err("G3 evidence marker createdAt 不是 UTC RFC3339 时间".to_string());
    }
    Ok(())
}

pub(super) fn validate_marker_after_edit_timestamps(
    marker_created_at: &str,
    timestamps: &GitHubEditTimestamps,
    label: &str,
) -> Result<(), String> {
    let evidence_timestamp = timestamps
        .last_edited_at
        .as_deref()
        .unwrap_or(&timestamps.created_at);
    validate_marker_is_strictly_later(marker_created_at, evidence_timestamp, label)
}

pub(super) fn validate_marker_after_activity_timestamp(
    marker_created_at: &str,
    activity_timestamp: &str,
    allow_marker_same_second: bool,
    label: &str,
) -> Result<(), String> {
    let marker_seconds = parse_utc_timestamp_seconds(marker_created_at)
        .ok_or("G3 evidence marker createdAt 不是 UTC RFC3339 时间")?;
    let activity_seconds = parse_utc_timestamp_seconds(activity_timestamp)
        .ok_or_else(|| format!("{label} 时间不是 UTC RFC3339 时间"))?;
    let stale = marker_seconds < activity_seconds
        || (!allow_marker_same_second && marker_seconds == activity_seconds);
    if stale {
        return Err(format!(
            "G3 evidence marker 必须{} {label}；GitHub 同秒无法证明最终 activity 已完成",
            if allow_marker_same_second {
                "不早于"
            } else {
                "严格晚于"
            }
        ));
    }
    Ok(())
}

pub(super) fn validate_dependabot_body_edits_after_marker(
    marker_created_at: &str,
    timestamps: &GitHubEditTimestamps,
    edits: &GitHubUserContentEditConnection,
    label: &str,
) -> Result<(), String> {
    if edits.page_info.has_next_page {
        return Err(format!(
            "{label} body edit history 超过 100 条；无法证明 marker 后只有 Dependabot 自主改写"
        ));
    }
    let marker_seconds = parse_utc_timestamp_seconds(marker_created_at)
        .ok_or("G3 evidence marker createdAt 不是 UTC RFC3339 时间")?;
    let last_edited_at = timestamps
        .last_edited_at
        .as_deref()
        .ok_or_else(|| format!("{label} 缺少 lastEditedAt"))?;
    let last_edited_seconds = parse_utc_timestamp_seconds(last_edited_at)
        .ok_or_else(|| format!("{label} lastEditedAt 不是 UTC RFC3339 时间"))?;
    let updated_seconds = parse_utc_timestamp_seconds(&timestamps.updated_at)
        .ok_or_else(|| format!("{label} updatedAt 不是 UTC RFC3339 时间"))?;
    if updated_seconds != last_edited_seconds {
        return Err(format!(
            "{label} 在最后一次 body edit 之外还有无法归因的 PR activity；旧 marker 不得恢复 success"
        ));
    }

    let mut latest_history_edit = None;
    let mut later_edit_count = 0;
    for edit in &edits.nodes {
        let edited_seconds = parse_utc_timestamp_seconds(&edit.edited_at)
            .ok_or_else(|| format!("{label} userContentEdit.editedAt 不是 UTC RFC3339 时间"))?;
        latest_history_edit = Some(
            latest_history_edit.map_or(edited_seconds, |current: u64| current.max(edited_seconds)),
        );
        if edited_seconds < marker_seconds {
            continue;
        }
        later_edit_count += 1;
        let editor = edit
            .editor
            .as_ref()
            .ok_or_else(|| format!("{label} marker 后的 body edit 缺少 editor"))?;
        if !is_dependabot_actor(&editor.login) {
            return Err(format!(
                "{label} marker 后包含非 Dependabot body edit（editor={}）；必须新增 marker",
                editor.login
            ));
        }
    }
    if later_edit_count == 0 {
        return Err(format!(
            "{label} 没有可验证的 marker 后 Dependabot body edit；不得放宽常规 freshness"
        ));
    }
    if latest_history_edit != Some(last_edited_seconds) {
        return Err(format!(
            "{label} userContentEdits 与 lastEditedAt 不一致；body edit history 不完整"
        ));
    }
    Ok(())
}

pub(super) fn validate_dependabot_body_edits_after_g3_comment(
    g3_comment_created_at: &str,
    edits: &GitHubUserContentEditConnection,
    label: &str,
) -> Result<(), String> {
    if edits.page_info.has_next_page {
        return Err(format!(
            "{label} body edit history 超过 100 条；无法确认 G3 comment 后只有 Dependabot 自主改写"
        ));
    }
    let g3_comment_seconds = parse_utc_timestamp_seconds(g3_comment_created_at)
        .ok_or("current G3 comment createdAt 不是 UTC RFC3339 时间")?;
    let mut later_edit_count = 0;
    for edit in &edits.nodes {
        let edited_seconds = parse_utc_timestamp_seconds(&edit.edited_at)
            .ok_or_else(|| format!("{label} userContentEdit.editedAt 不是 UTC RFC3339 时间"))?;
        if edited_seconds < g3_comment_seconds {
            continue;
        }
        if edited_seconds == g3_comment_seconds {
            return Err(format!(
                "{label} body edit 与 current G3 comment 同秒；无法证明 Dependabot 在 comment 后自主改写"
            ));
        }
        later_edit_count += 1;
        let editor = edit
            .editor
            .as_ref()
            .ok_or_else(|| format!("{label} G3 comment 后的 body edit 缺少 editor"))?;
        if !is_dependabot_actor(&editor.login) {
            return Err(format!(
                "{label} G3 comment 后包含非 Dependabot body edit（editor={}）；不能恢复 target 元数据",
                editor.login
            ));
        }
    }
    if later_edit_count == 0 {
        return Err(format!(
            "{label} 缺少严格晚于 current G3 comment 的 Dependabot body edit；不能恢复 target 元数据"
        ));
    }
    Ok(())
}

fn is_dependabot_actor(actor: &str) -> bool {
    matches!(
        actor
            .trim()
            .trim_end_matches("[bot]")
            .to_ascii_lowercase()
            .as_str(),
        "dependabot" | "app/dependabot"
    )
}

pub(super) fn validate_marker_after_timeline(
    marker_comment_id: u64,
    marker_created_at: &str,
    timeline: &[GitHubTimelineItem],
    target: GitHubTimelineTarget,
    require_marker_event: bool,
    label: &str,
) -> Result<(), String> {
    let marker_positions = timeline
        .iter()
        .enumerate()
        .filter(|(_, item)| item.event == "commented" && item.id == Some(marker_comment_id))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let marker_position = match (require_marker_event, marker_positions.as_slice()) {
        (true, [position]) => Some(*position),
        (true, []) => {
            return Err(format!(
                "{label} timeline 未返回 marker comment {marker_comment_id}；freshness 失败关闭"
            ));
        }
        (true, _) => {
            return Err(format!(
                "{label} timeline 重复返回 marker comment {marker_comment_id}；freshness 失败关闭"
            ));
        }
        (false, []) => None,
        (false, _) => {
            return Err(format!(
                "非 marker 目标 {label} timeline 意外包含 marker comment {marker_comment_id}"
            ));
        }
    };

    for (index, item) in timeline.iter().enumerate() {
        if item.event == "commented" && item.id == Some(marker_comment_id) {
            continue;
        }
        let timestamp = match (target, item.event.as_str()) {
            (GitHubTimelineTarget::Issue, "closed" | "reopened") => item.created_at.as_deref(),
            (
                GitHubTimelineTarget::PullRequest,
                "closed"
                | "reopened"
                | "convert_to_draft"
                | "ready_for_review"
                | "review_requested"
                | "review_request_removed"
                | "review_dismissed"
                | "head_ref_force_pushed"
                | "head_ref_deleted"
                | "head_ref_restored"
                | "base_ref_changed",
            ) => item.created_at.as_deref(),
            (GitHubTimelineTarget::PullRequest, "commented") => {
                item.updated_at.as_deref().or(item.created_at.as_deref())
            }
            (GitHubTimelineTarget::PullRequest, "reviewed") => item.submitted_at.as_deref(),
            (GitHubTimelineTarget::PullRequest, "committed") => item
                .committer
                .as_ref()
                .map(|committer| committer.date.as_str()),
            _ => continue,
        }
        .ok_or_else(|| {
            format!(
                "{label} timeline 的 `{}` 事件缺少可验证时间；marker freshness 失败关闭",
                item.event
            )
        })?;
        if marker_position.is_some_and(|position| index > position) {
            return Err(format!(
                "{label} `{}` timeline event 排在 marker comment 之后；旧 marker 不得恢复 success",
                item.event
            ));
        }
        validate_marker_is_strictly_later(
            marker_created_at,
            timestamp,
            &format!("{label} `{}` timeline event", item.event),
        )?;
    }
    Ok(())
}

pub(super) fn validate_marker_is_strictly_later(
    marker_created_at: &str,
    evidence_timestamp: &str,
    label: &str,
) -> Result<(), String> {
    let marker_seconds = parse_utc_timestamp_seconds(marker_created_at)
        .ok_or("G3 evidence marker createdAt 不是 UTC RFC3339 时间")?;
    let evidence_seconds = parse_utc_timestamp_seconds(evidence_timestamp)
        .ok_or_else(|| format!("{label} 时间不是 UTC RFC3339 时间"))?;
    if marker_seconds <= evidence_seconds {
        return Err(format!(
            "G3 evidence marker 必须严格晚于 {label}；GitHub 同秒无法证明最终 evidence 已完成"
        ));
    }
    Ok(())
}

pub(super) fn gh_issue_view_for_phase(
    repo: &str,
    number: u64,
    phase: GateEvidencePhase,
) -> Result<GitHubIssue, String> {
    let fields = gh_issue_fields(phase);
    gh_json(&[
        "issue".to_string(),
        "view".to_string(),
        number.to_string(),
        "--repo".to_string(),
        repo.to_string(),
        "--json".to_string(),
        fields.to_string(),
    ])
}

pub(super) fn gh_pr_view_for_phase(
    repo: &str,
    number: u64,
    phase: GateEvidencePhase,
) -> Result<GitHubPullRequest, String> {
    let fields = gh_pr_fields(phase);
    gh_json(&[
        "pr".to_string(),
        "view".to_string(),
        number.to_string(),
        "--repo".to_string(),
        repo.to_string(),
        "--json".to_string(),
        fields.to_string(),
    ])
}

pub(super) fn gh_issue_fields(phase: GateEvidencePhase) -> &'static str {
    match phase {
        GateEvidencePhase::G3 => "body,state,comments",
        GateEvidencePhase::G4 => "body,state,projectItems,comments",
    }
}

pub(super) fn gh_pr_fields(phase: GateEvidencePhase) -> &'static str {
    match phase {
        GateEvidencePhase::G3 => {
            "body,state,isDraft,createdAt,mergedAt,closingIssuesReferences,comments"
        }
        GateEvidencePhase::G4 => {
            "body,state,isDraft,createdAt,mergedAt,closingIssuesReferences,projectItems,comments"
        }
    }
}

pub(super) fn gh_json<T: serde::de::DeserializeOwned>(args: &[String]) -> Result<T, String> {
    let output = Command::new("gh")
        .args(args)
        .output()
        .map_err(|err| format!("无法运行 gh: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh 命令失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    serde_json::from_slice(&output.stdout).map_err(|err| {
        format!(
            "gh 输出不是预期 JSON: {err}; 原始输出：{}",
            String::from_utf8_lossy(&output.stdout).trim()
        )
    })
}
