//! Merge Queue G4 历史失败的最小恢复协议。

use std::collections::BTreeSet;

use super::{document::*, g4::*, github::*, model::*};

const HISTORICAL_FAILURE_MODE: &str = "historical_failure";
const ACCEPTED_EXCEPTION_DECISION: &str = "accepted_exception";

fn issue_comment_prefix(repo: &str, issue: u64) -> String {
    format!("https://github.com/{repo}/issues/{issue}#issuecomment-")
}

fn is_trusted_owner(login: &str) -> bool {
    G3_OWNER_ACTORS
        .iter()
        .any(|actor| actor.eq_ignore_ascii_case(login))
}

fn trusted_owner_comment<'a>(
    issue: &'a GitHubIssue,
    url: &str,
    label: &str,
) -> Result<&'a GitHubComment, String> {
    let comment = comment_for_permalink(issue, url, label)?;
    let trusted_author = comment
        .author
        .as_ref()
        .is_some_and(|author| is_trusted_owner(&author.login));
    if comment.includes_created_edit || !trusted_author {
        return Err(format!("{label} 必须由 trusted G3 Owner 创建且保持未编辑"));
    }
    Ok(comment)
}

pub(super) fn merge_queue_g4_recovery_record(
    comment: &GitHubComment,
    args: &GateEvidenceArgs,
) -> Result<Option<MergeQueueG4RecoveryRecord>, String> {
    let count = comment.body.matches(MERGE_QUEUE_G4_RECOVERY_START).count();
    if count == 0 {
        return Ok(None);
    }
    if count != 1 {
        return Err("Issue G4 必须包含且只包含一个 merge-queue-g4-recovery:v1 记录".to_string());
    }
    if comment.includes_created_edit {
        return Err("Merge Queue G4 recovery comment 在创建后被编辑".to_string());
    }
    let author = comment
        .author
        .as_ref()
        .map(|actor| actor.login.as_str())
        .ok_or("Merge Queue G4 recovery comment 缺少 author")?;
    let (_, after_start) = comment
        .body
        .split_once(MERGE_QUEUE_G4_RECOVERY_START)
        .expect("marker count guarantees a start marker");
    let (json, _) = after_start
        .split_once(MERGE_QUEUE_G4_RECOVERY_END)
        .ok_or("merge-queue-g4-recovery:v1 缺少结束 marker")?;
    let record = serde_json::from_str::<MergeQueueG4RecoveryRecord>(json.trim())
        .map_err(|error| format!("merge-queue-g4-recovery:v1 JSON 无效：{error}"))?;

    if record.schema_version != 1 || record.decision != ACCEPTED_EXCEPTION_DECISION {
        return Err(
            "Merge Queue G4 recovery schemaVersion / decision 必须为 1 / accepted_exception"
                .to_string(),
        );
    }
    if !author.eq_ignore_ascii_case(&record.authorized_by) || !is_trusted_owner(author) {
        return Err(
            "Merge Queue G4 recovery 必须由 trusted G3 Owner 以 authorizedBy 签署".to_string(),
        );
    }
    for (field, value) in [
        ("risk", record.risk.as_str()),
        ("acceptanceBoundary", record.acceptance_boundary.as_str()),
        ("cleanupOwner", record.cleanup_owner.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(format!("Merge Queue G4 recovery `{field}` 不能为空"));
        }
    }
    if record.failures.is_empty() {
        return Err("Merge Queue G4 recovery failures 不能为空".to_string());
    }

    let target_issue_prefix = issue_comment_prefix(&args.repo, args.issue);
    let mut failed_prs = BTreeSet::new();
    for failure in &record.failures {
        if !failed_prs.insert(failure.failed_pr) {
            return Err("Merge Queue G4 recovery failedPr 不得重复".to_string());
        }
        let expected_role = if args.delivery_pr == Some(failure.failed_pr) {
            "delivery"
        } else if args.related_prs.contains(&failure.failed_pr) {
            "related"
        } else {
            return Err("failedPr 必须属于当前 Delivery + Related PR 完整集合".to_string());
        };
        if failure.failed_role != expected_role {
            return Err("failedRole 与当前 Issue 的 PR 角色不一致".to_string());
        }
        if failure.remediation_issue == 0 || failure.remediation_issue == args.issue {
            return Err("remediationIssue 必须是不同于当前 Issue 的正整数".to_string());
        }
        if !failure
            .failure_evidence_url
            .starts_with(&target_issue_prefix)
            || failure.failure_evidence_url == comment.url
            || !failure
                .remediation_g4_url
                .starts_with(&issue_comment_prefix(&args.repo, failure.remediation_issue))
        {
            return Err(
                "Merge Queue G4 recovery evidence URL 未绑定当前 repo / target / remediation identity"
                    .to_string(),
            );
        }
    }
    unique_metadata_line(&comment.body, "Historical queue recovery")?;
    Ok(Some(record))
}

pub(super) fn recovery_entry<'a>(
    recovery: &'a MergeQueueG4RecoveryRecord,
    number: u64,
    role: &str,
) -> Option<&'a MergeQueueG4RecoveryEntry> {
    recovery
        .failures
        .iter()
        .find(|failure| failure.failed_pr == number && failure.failed_role == role)
}

pub(super) fn validate_historical_failure_pr_record(
    number: u64,
    role: &str,
    pr: &GitHubPullRequest,
    record: &MergeQueueG4PullRequestRecord,
    recovery: &MergeQueueG4RecoveryEntry,
) -> Result<(), String> {
    if record.number != number
        || record.role != role
        || recovery.failed_pr != number
        || recovery.failed_role != role
    {
        return Err(format!(
            "historical_failure identity/order 不一致：期望 {role} PR #{number}"
        ));
    }
    let h_pr = full_commit_oid(&record.h_pr, &format!("PR #{number} H_pr"))?;
    let h_main = full_commit_oid(&record.h_main, &format!("PR #{number} H_main"))?;
    if h_pr != pr.head_ref_oid.to_ascii_lowercase() {
        return Err(format!(
            "historical_failure PR #{number} H_pr 与 GitHub headRefOid 不一致"
        ));
    }
    let merge_commit = pr
        .merge_commit
        .as_ref()
        .ok_or_else(|| format!("historical_failure PR #{number} 缺少 mergeCommit"))?;
    if h_main != merge_commit.oid.to_ascii_lowercase() {
        return Err(format!(
            "historical_failure PR #{number} H_main 与 mergeCommit OID 不一致"
        ));
    }
    if !merged_after_queue_activation(pr, &format!("historical_failure PR #{number}"))? {
        return Err(
            "historical_failure 只适用于 Merge Queue G4 activation 边界后的历史 PR".to_string(),
        );
    }
    full_commit_oid(
        record
            .h_mg
            .as_deref()
            .ok_or("historical_failure record 必须保留已观测的 H_mg")?,
        &format!("historical_failure PR #{number} H_mg"),
    )?;
    let valid_non_success = record.mode == HISTORICAL_FAILURE_MODE
        && record
            .reason
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        && record.failure_evidence_url.as_deref() == Some(recovery.failure_evidence_url.as_str())
        && record.recovery_evidence_url.as_deref() == Some(recovery.remediation_g4_url.as_str())
        && record.checks_conclusion.is_none()
        && record.checks_url.is_none()
        && record.chain.is_none()
        && record.inclusion_method.is_none()
        && record.inclusion_evidence_url.is_none()
        && record.bootstrap_evidence_url.is_none();
    if !valid_non_success {
        return Err(
            "historical_failure 必须保留失败 reason/evidence，不得声称 checks 或 inclusion success"
                .to_string(),
        );
    }
    Ok(())
}

pub(super) fn validate_remediation_issue<'a>(
    remediation: &'a GitHubIssue,
    failure: &MergeQueueG4RecoveryEntry,
) -> Result<&'a GitHubComment, String> {
    if remediation.state != "CLOSED" || !is_laneflow_project_done(&remediation.project_items) {
        return Err("remediation Issue 必须先完成 G4、Project Done 并手动关闭".to_string());
    }
    if completed_gate_permalink(&remediation.body, "G4")? != failure.remediation_g4_url {
        return Err("remediationG4Url 与 live remediation Issue Gate Ledger 不一致".to_string());
    }
    let g4_comment = trusted_owner_comment(
        remediation,
        &failure.remediation_g4_url,
        "remediation Issue G4",
    )?;
    validate_comment_body(&g4_comment.body, G4_COMMENT_FIELDS, "remediation Issue G4")?;
    if !unique_metadata_line(&g4_comment.body, "Project")?.contains("Done") {
        return Err("remediation Issue G4 `- Project：` 必须记录 Done".to_string());
    }
    let expected_backlink = format!(
        "- Historical failure evidence：{}",
        failure.failure_evidence_url
    );
    if unique_metadata_line(&g4_comment.body, "Historical failure evidence")? != expected_backlink {
        return Err(
            "remediation Issue G4 必须以 `- Historical failure evidence：` 精确回链对应 failure evidence"
                .to_string(),
        );
    }
    Ok(g4_comment)
}

pub(super) fn validate_failure_evidence<'a>(
    target_issue: &'a GitHubIssue,
    failure: &MergeQueueG4RecoveryEntry,
    pr_record: &MergeQueueG4PullRequestRecord,
) -> Result<&'a GitHubComment, String> {
    let evidence = trusted_owner_comment(
        target_issue,
        &failure.failure_evidence_url,
        "historical failure evidence",
    )?;
    if evidence.body.trim().is_empty() {
        return Err("historical failure evidence 不能为空".to_string());
    }
    let h_mg = full_commit_oid(
        pr_record
            .h_mg
            .as_deref()
            .ok_or("historical_failure record 必须保留已观测的 H_mg")?,
        &format!("historical_failure PR #{} H_mg", failure.failed_pr),
    )?;
    let reason = pr_record
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("historical_failure record 必须保留非空 reason")?;
    let expected_identity = format!("- Historical failed PR：#{}", failure.failed_pr);
    let expected_h_mg = format!("- Historical failed H_mg：`{h_mg}`");
    let expected_reason = format!("- Historical failure reason：{reason}");
    if unique_metadata_line(&evidence.body, "Historical failed PR")? != expected_identity
        || unique_metadata_line(&evidence.body, "Historical failed H_mg")? != expected_h_mg
        || unique_metadata_line(&evidence.body, "Historical failure reason")? != expected_reason
    {
        return Err(
            "historical failure evidence 必须精确绑定 failed PR、record H_mg 与 failure reason"
                .to_string(),
        );
    }
    Ok(evidence)
}

pub(super) fn validate_remediation_closure(
    timeline: &[GitHubTimelineItem],
    remediation_g4_at: u64,
    acceptance_at: u64,
) -> Result<(), String> {
    let lifecycle = timeline
        .iter()
        .enumerate()
        .filter(|(_, item)| item.event == "closed" || item.event == "reopened")
        .map(|(position, item)| {
            let created_at = item
                .created_at
                .as_deref()
                .and_then(parse_utc_timestamp_seconds)
                .ok_or("remediation Issue close/reopen event createdAt 无效")?;
            Ok((created_at, position, item))
        })
        .collect::<Result<Vec<_>, String>>()?;
    if lifecycle
        .iter()
        .any(|(created_at, _, item)| item.event == "reopened" && *created_at >= remediation_g4_at)
    {
        return Err(
            "remediation Issue 普通 G4 必须严格晚于最后一次 reopen，G4 后不得再次 reopen"
                .to_string(),
        );
    }
    let latest_lifecycle = lifecycle
        .into_iter()
        .max_by_key(|(created_at, position, _)| (*created_at, *position))
        .ok_or("remediation Issue 缺少手动关闭事件")?;
    let actor_is_trusted = latest_lifecycle
        .2
        .actor
        .as_ref()
        .is_some_and(|actor| is_trusted_owner(&actor.login));
    if latest_lifecycle.2.event != "closed"
        || !actor_is_trusted
        || latest_lifecycle.0 <= remediation_g4_at
        || latest_lifecycle.0 >= acceptance_at
    {
        return Err(
            "remediation Issue 必须由 trusted G3 Owner 在普通 G4 后、accepted_exception 前手动关闭"
                .to_string(),
        );
    }
    Ok(())
}

pub(super) fn validate_remediation_project_done(
    evidence: &[GitHubProjectStatusEvidence],
    remediation_g4_at: u64,
    acceptance_at: u64,
) -> Result<(), String> {
    let matching = evidence
        .iter()
        .filter(|item| item.project_title == "LaneFlow")
        .collect::<Vec<_>>();
    let [status] = matching.as_slice() else {
        return Err("remediation Issue 必须恰有一个 LaneFlow Project Status evidence".to_string());
    };
    let updated_at = parse_utc_timestamp_seconds(&status.updated_at)
        .ok_or("remediation Issue Project Status updatedAt 无效")?;
    if status.status_name != "Done"
        || updated_at >= remediation_g4_at
        || updated_at >= acceptance_at
    {
        return Err(
            "remediation Issue 必须在普通 G4 与 accepted_exception 前已进入 LaneFlow Project Done"
                .to_string(),
        );
    }
    Ok(())
}

pub(super) fn validate_live_merge_queue_recovery(
    args: &GateEvidenceArgs,
    target_issue: &GitHubIssue,
    evidence_record: &MergeQueueG4Record,
    record: &MergeQueueG4RecoveryRecord,
) -> Result<(), String> {
    let target_g4_url = completed_gate_permalink(&target_issue.body, "G4")?;
    let target_g4 = comment_for_permalink(target_issue, &target_g4_url, "target Issue G4")?;
    let target_g4_at = parse_utc_timestamp_seconds(&target_g4.created_at)
        .ok_or("target Issue G4 createdAt 无效")?;

    for failure in &record.failures {
        let pr_record = evidence_record
            .pull_requests
            .iter()
            .find(|entry| entry.number == failure.failed_pr && entry.role == failure.failed_role)
            .ok_or("accepted_exception failure 缺少对应 historical_failure PR record")?;
        let evidence = validate_failure_evidence(target_issue, failure, pr_record)?;
        let evidence_at = parse_utc_timestamp_seconds(&evidence.created_at)
            .ok_or("historical failure evidence createdAt 无效")?;
        let failed_pr = gh_pr_view_for_phase(&args.repo, failure.failed_pr, GateEvidencePhase::G4)?;
        let failed_merged_at = failed_pr
            .merged_at
            .as_deref()
            .and_then(parse_utc_timestamp_seconds)
            .ok_or("historical_failure PR mergedAt 无效")?;

        let remediation =
            gh_issue_view_for_phase(&args.repo, failure.remediation_issue, GateEvidencePhase::G4)?;
        let remediation_g4 = validate_remediation_issue(&remediation, failure)?;
        let remediation_g4_at = parse_utc_timestamp_seconds(&remediation_g4.created_at)
            .ok_or("remediation Issue G4 createdAt 无效")?;
        validate_merge_queue_recovery_timing(
            failed_merged_at,
            evidence_at,
            remediation_g4_at,
            target_g4_at,
        )?;
        let remediation_project =
            gh_issue_project_status_evidence(&args.repo, failure.remediation_issue)?;
        validate_remediation_project_done(&remediation_project, remediation_g4_at, target_g4_at)?;
        let remediation_timeline = gh_issue_timeline(&args.repo, failure.remediation_issue)?;
        validate_remediation_closure(&remediation_timeline, remediation_g4_at, target_g4_at)?;
    }
    Ok(())
}

pub(super) fn validate_merge_queue_recovery_timing(
    failed_merged_at: u64,
    evidence_at: u64,
    remediation_g4_at: u64,
    accepted_exception_at: u64,
) -> Result<(), String> {
    if failed_merged_at >= evidence_at
        || evidence_at >= remediation_g4_at
        || remediation_g4_at >= accepted_exception_at
    {
        return Err(
            "historical recovery 时间必须严格满足：失败 PR merge < failure evidence < remediation Issue G4 < accepted_exception"
                .to_string(),
        );
    }
    Ok(())
}
