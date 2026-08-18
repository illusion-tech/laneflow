//! G4 完成证据与 late Related PR recovery 验证。

use std::collections::BTreeSet;

use super::{args::*, document::*, g3::*, model::*};

pub(super) fn reject_inapplicable_g4_recovery_marker(issue: &GitHubIssue) -> Result<(), String> {
    let issue_g4_permalink = completed_gate_permalink(&issue.body, "G4")?;
    let g4_comment = comment_for_permalink(issue, &issue_g4_permalink, "Issue G4")?;
    if g4_comment.body.contains(G3_FULL_SET_RECOVERY_START) {
        return Err(
            "不存在 late Related PR 时，Issue G4 comment 不得包含 G3 full-set recovery 记录"
                .to_string(),
        );
    }
    Ok(())
}

pub(super) fn has_late_related_pr(
    args: &GateEvidenceArgs,
    delivery_pr: &GitHubPullRequest,
    related_prs: &[GitHubPullRequest],
) -> Result<bool, String> {
    if args.related_prs.len() != related_prs.len() {
        return Err("Related PR 参数与已读取 PR 数量不一致".to_string());
    }
    let delivery_merged_at = delivery_pr
        .merged_at
        .as_deref()
        .ok_or("Delivery PR 尚未合并，不能判断 late Related PR")?;
    let delivery_merged_at_seconds = parse_utc_timestamp_seconds(delivery_merged_at)
        .ok_or("Delivery PR mergedAt 不是 UTC RFC3339 秒级时间")?;
    for (number, related_pr) in args.related_prs.iter().zip(related_prs) {
        let related_created_at = parse_utc_timestamp_seconds(&related_pr.created_at)
            .ok_or_else(|| format!("Related PR #{number} createdAt 不是 UTC RFC3339 秒级时间"))?;
        if related_created_at == delivery_merged_at_seconds {
            return Err(format!(
                "Related PR #{number} createdAt 与 Delivery mergedAt 同秒，无法安全判断是否为 late Related PR"
            ));
        }
        if related_created_at > delivery_merged_at_seconds {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn validate_original_related_g3_before_delivery_merge(
    number: u64,
    related_g3_effective_at: u64,
    delivery_merged_at: u64,
) -> Result<(), String> {
    if related_g3_effective_at >= delivery_merged_at {
        return Err(format!(
            "original Related PR #{number} G3 comment 生效时间必须严格早于 Delivery PR 合并时间"
        ));
    }
    Ok(())
}

pub(super) fn validate_g4_g3_full_set_recovery(
    args: &GateEvidenceArgs,
    issue: &GitHubIssue,
    delivery_pr: &GitHubPullRequest,
    related_prs: &[GitHubPullRequest],
) -> Result<(), String> {
    if args.phase != GateEvidencePhase::G4 {
        return Err("G3 full-set recovery 只允许用于 G4".to_string());
    }
    let delivery_number = args
        .delivery_pr
        .ok_or("G3 full-set recovery 缺少 Delivery PR")?;
    let delivery_merged_at = delivery_pr
        .merged_at
        .as_deref()
        .ok_or("Delivery PR 尚未合并，不能使用 G3 full-set recovery")?;
    let delivery_merged_at_seconds = parse_utc_timestamp_seconds(delivery_merged_at)
        .ok_or("Delivery PR mergedAt 不是 UTC RFC3339 秒级时间")?;
    let issue_g3_line = completed_gate_line(&issue.body, "G3")?;
    let delivery_pr_line = metadata_line(&issue.body, "Delivery PR")?;
    if !delivery_pr_line.contains(&format!("#{delivery_number}")) {
        return Err(format!(
            "Issue 的 `Delivery PR` 字段未记录 Delivery PR #{}",
            delivery_number
        ));
    }
    let related_prs_line = metadata_line(&issue.body, "Related PRs")?;
    validate_recovery_related_pr_order(related_prs_line, &args.related_prs)?;
    let requested_related_prs = args.related_prs.iter().copied().collect::<BTreeSet<_>>();

    let issue_g4_permalink = completed_gate_permalink(&issue.body, "G4")?;
    let g4_comment = comment_for_permalink(issue, &issue_g4_permalink, "Issue G4")?;
    validate_historical_exception_appendix(
        g4_comment,
        args.issue,
        std::iter::once((delivery_number, delivery_pr))
            .chain(args.related_prs.iter().copied().zip(related_prs)),
    )?;
    let (record, evidence_urls) = parse_g3_full_set_recovery(g4_comment, args, delivery_merged_at)?;
    let mut reconstructed_related_prs = record.original_related_prs.clone();
    reconstructed_related_prs.extend(record.late_related_prs.iter().copied());
    if reconstructed_related_prs != args.related_prs {
        return Err(format!(
            "G3 full-set recovery 的 originalRelatedPrs + lateRelatedPrs 必须按顺序等于最终 Related PR 参数 [{}]",
            format_issue_numbers(&requested_related_prs)
        ));
    }

    let delivery_permalink = completed_gate_permalink(&delivery_pr.body, "G3")?;
    if !line_links_to_comment_permalink(&issue.body, issue_g3_line, &delivery_permalink) {
        return Err("Issue 的 G3 checkbox 未回链 Delivery PR 的 G3 comment permalink".to_string());
    }
    if !evidence_urls.contains(&delivery_permalink) {
        return Err(
            "G3 full-set recovery evidenceRefs 未覆盖 Delivery PR G3 permalink".to_string(),
        );
    }
    let original_args = GateEvidenceArgs {
        phase: GateEvidencePhase::G3,
        repo: args.repo.clone(),
        issue: args.issue,
        delivery_pr: Some(delivery_number),
        related_prs: record.original_related_prs.clone(),
    };
    let delivery_g3_effective_at =
        parse_utc_timestamp_seconds(&g3_effective_at_for_merge_validation(
            delivery_pr,
            &delivery_permalink,
            "Delivery PR original G3 comment",
            &original_args,
        )?)
        .ok_or("Delivery PR original G3 comment effectiveAt 不是 UTC RFC3339 秒级时间")?;
    if delivery_g3_effective_at >= delivery_merged_at_seconds {
        return Err(
            "G3 full-set recovery 的 Delivery PR original G3 comment 生效时间必须严格早于 Delivery merge"
                .to_string(),
        );
    }
    let allow_delivery_legacy_exception = historical_exception_applies_to_target(
        g4_comment,
        args.issue,
        delivery_number,
        &delivery_permalink,
    )?;
    validate_comment(
        delivery_pr,
        &delivery_permalink,
        G3_COMMENT_FIELDS,
        "Delivery PR original G3",
        &original_args,
        allow_delivery_legacy_exception,
    )?;
    if !delivery_pr
        .closing_issues_references
        .iter()
        .any(|reference| issue_reference_matches(reference, &args.repo, args.issue))
    {
        return Err(format!(
            "Delivery PR #{} 的 closingIssuesReferences 未覆盖 Issue #{}",
            delivery_number, args.issue
        ));
    }

    let original_related_prs = record
        .original_related_prs
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let late_related_prs = record
        .late_related_prs
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if original_related_prs.len() != record.original_related_prs.len()
        || late_related_prs.len() != record.late_related_prs.len()
        || !original_related_prs.is_disjoint(&late_related_prs)
    {
        return Err(
            "G3 full-set recovery 的 originalRelatedPrs / lateRelatedPrs 不得重复或重叠"
                .to_string(),
        );
    }
    if late_related_prs.is_empty() {
        return Err("G3 full-set recovery 至少需要一个 late Related PR".to_string());
    }

    for (number, related_pr) in args.related_prs.iter().zip(related_prs) {
        let related_args = GateEvidenceArgs {
            phase: GateEvidencePhase::G3,
            repo: args.repo.clone(),
            issue: args.issue,
            delivery_pr: None,
            related_prs: vec![*number],
        };
        let related_permalink = completed_gate_permalink(&related_pr.body, "G3")?;
        let allow_related_legacy_exception = historical_exception_applies_to_target(
            g4_comment,
            args.issue,
            *number,
            &related_permalink,
        )?;
        validate_related_pr_g3(
            &related_args,
            &issue.body,
            issue_g3_line,
            *number,
            related_pr,
            allow_related_legacy_exception,
        )?;
        if !evidence_urls.contains(&related_permalink) {
            return Err(format!(
                "G3 full-set recovery evidenceRefs 未覆盖 Related PR #{number} G3 permalink"
            ));
        }
        let related_created_at = parse_utc_timestamp_seconds(&related_pr.created_at)
            .ok_or_else(|| format!("Related PR #{number} createdAt 不是 UTC RFC3339 秒级时间"))?;
        if late_related_prs.contains(number) {
            if related_created_at <= delivery_merged_at_seconds {
                return Err(format!(
                    "late Related PR #{number} 必须在 Delivery PR 合并后创建"
                ));
            }
        } else if original_related_prs.contains(number) {
            if related_created_at > delivery_merged_at_seconds {
                return Err(format!(
                    "original Related PR #{number} 不得在 Delivery PR 合并后创建"
                ));
            }
            let related_g3_effective_at =
                parse_utc_timestamp_seconds(&g3_effective_at_for_merge_validation(
                    related_pr,
                    &related_permalink,
                    &format!("Related PR #{number} G3 comment"),
                    &related_args,
                )?)
                .ok_or_else(|| {
                    format!("Related PR #{number} G3 comment effectiveAt 不是 UTC RFC3339 秒级时间")
                })?;
            validate_original_related_g3_before_delivery_merge(
                *number,
                related_g3_effective_at,
                delivery_merged_at_seconds,
            )?;
        } else {
            return Err(format!(
                "Related PR #{number} 未归入 originalRelatedPrs 或 lateRelatedPrs"
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_g4_evidence(
    args: &GateEvidenceArgs,
    issue: &GitHubIssue,
    delivery_pr: &GitHubPullRequest,
    related_prs: &[GitHubPullRequest],
) -> Result<(), String> {
    validate_related_pr_snapshot_count(args, related_prs)?;
    if issue.state != "OPEN" {
        return Err("G4 断言必须在手动关闭 Issue 前运行".to_string());
    }
    let merged_at = delivery_pr
        .merged_at
        .as_deref()
        .ok_or("Delivery PR 尚未合并，不能通过 G4")?;
    if delivery_pr.state != "MERGED" {
        return Err("Delivery PR 状态不是 MERGED，不能通过 G4".to_string());
    }
    let mut latest_merge = merged_at;
    for (number, related_pr) in args.related_prs.iter().zip(related_prs) {
        let related_merged_at = related_pr
            .merged_at
            .as_deref()
            .ok_or_else(|| format!("Related PR #{number} 尚未合并，不能通过 G4"))?;
        if related_pr.state != "MERGED" {
            return Err(format!("Related PR #{number} 状态不是 MERGED，不能通过 G4"));
        }
        if related_merged_at > latest_merge {
            latest_merge = related_merged_at;
        }
    }

    let issue_g4_permalink = completed_gate_permalink(&issue.body, "G4")?;
    let g4_comment = comment_for_permalink(issue, &issue_g4_permalink, "Issue G4")?;
    validate_comment_body(&g4_comment.body, G4_COMMENT_FIELDS, "Issue G4")?;
    let delivery_number = args
        .delivery_pr
        .ok_or("G4 validation 缺少 Delivery PR 参数")?;
    validate_historical_exception_appendix(
        g4_comment,
        args.issue,
        std::iter::once((delivery_number, delivery_pr))
            .chain(args.related_prs.iter().copied().zip(related_prs)),
    )?;
    let delivery_permalink = completed_gate_permalink(&delivery_pr.body, "G3")?;
    let mut allow_g4_legacy_exception = historical_exception_applies_to_target(
        g4_comment,
        args.issue,
        delivery_number,
        &delivery_permalink,
    )?;
    for (number, related_pr) in args.related_prs.iter().zip(related_prs) {
        let related_permalink = completed_gate_permalink(&related_pr.body, "G3")?;
        allow_g4_legacy_exception |= historical_exception_applies_to_target(
            g4_comment,
            args.issue,
            *number,
            &related_permalink,
        )?;
    }
    validate_gate_assertion_with_legacy_exception(
        &g4_comment.body,
        "Issue G4",
        args,
        GateEvidencePhase::G4,
        allow_g4_legacy_exception,
    )?;
    if g4_comment.created_at.as_str() < latest_merge {
        return Err("Issue G4 comment 早于最后一个关联 PR 的合并时间".to_string());
    }
    if !delivery_pr.body.contains("G4 回写") || !delivery_pr.body.contains(&issue_g4_permalink) {
        return Err(
            "Delivery PR body 缺少指向 Issue G4 comment 的 `G4 回写` permalink".to_string(),
        );
    }
    for gate in ["G0", "G1", "G2", "G3", "G4"] {
        completed_gate_line(&issue.body, gate)?;
    }
    if !is_laneflow_project_done(&issue.project_items) {
        return Err("Issue 尚未处于 LaneFlow Project 的 Done 状态".to_string());
    }
    if !is_laneflow_project_done(&delivery_pr.project_items) {
        return Err("Delivery PR 尚未处于 LaneFlow Project 的 Done 状态".to_string());
    }
    for (number, related_pr) in args.related_prs.iter().zip(related_prs) {
        if !is_laneflow_project_done(&related_pr.project_items) {
            return Err(format!(
                "Related PR #{number} 尚未处于 LaneFlow Project 的 Done 状态"
            ));
        }
    }
    Ok(())
}

pub(super) fn is_laneflow_project_done(project_items: &[ProjectItem]) -> bool {
    project_items.iter().any(|item| {
        item.title == "LaneFlow"
            && item
                .status
                .as_ref()
                .is_some_and(|status| status.name == "Done")
    })
}
