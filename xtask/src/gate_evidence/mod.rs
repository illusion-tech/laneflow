//! Gate evidence 命令门面与 G3/G4 共享编排。

mod args;
mod document;
#[cfg(test)]
mod fixtures;
mod g3;
mod g4;
mod github;
mod model;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::fs;

use args::*;
use document::*;
use g3::*;
use g4::validate_g4_evidence;
#[cfg(test)]
use g4::*;
use github::*;
use model::*;

fn gh_pr_view_for_gate_evidence(
    repo: &str,
    number: u64,
    phase: GateEvidencePhase,
) -> Result<GitHubPullRequest, String> {
    let mut pr = gh_pr_view_for_phase(repo, number, phase)?;
    if phase != GateEvidencePhase::G3 || parse_gate_evidence_target_metadata(&pr.body).is_ok() {
        return Ok(pr);
    }

    let metadata_error = parse_gate_evidence_target_metadata(&pr.body)
        .expect_err("checked missing or invalid PR target metadata");
    let permalink = completed_gate_permalink(&pr.body, "G3").map_err(|permalink_error| {
        format!(
            "{metadata_error}；Dependabot fallback 还要求当前 body 保留 G3 comment permalink：{permalink_error}"
        )
    })?;
    let comment = pr
        .comments
        .iter()
        .find(|comment| comment.url == permalink)
        .ok_or_else(|| format!("{metadata_error}；PR G3 permalink 未指向当前 PR comment"))?;
    let review = crate::external_review::evaluate_live(repo, number)?;
    if !review.uses_dependabot_lockfile_policy() {
        return Err(format!(
            "{metadata_error}；仅精确 dependabot-cargo-lock-only-v1 可从 append-only G3 comment 恢复 target 元数据"
        ));
    }
    let edits = gh_pr_user_content_edits(repo, number)?;
    validate_latest_body_edit_is_dependabot(&edits, &format!("PR #{number}"))?;
    let (role, issue_numbers) =
        parse_gate_evidence_target_metadata_from_g3_comment(repo, number, &comment.body)?;
    let issues = issue_numbers
        .iter()
        .map(|issue| format!("#{issue}"))
        .collect::<Vec<_>>()
        .join(", ");
    let role = match role {
        GateEvidencePrRole::Delivery => "Delivery PR",
        GateEvidencePrRole::Related => "Related PR",
    };
    let refs = issue_numbers
        .iter()
        .map(|issue| format!("Refs: #{issue}"))
        .collect::<Vec<_>>()
        .join("\n");
    pr.body = format!(
        "- 关联 Issue：{issues}\n- PR 角色：`{role}`\n- [x] G3 合并判断已记录：{permalink}\n{refs}"
    );
    Ok(pr)
}

fn check_gate_evidence_with_args(args: &GateEvidenceArgs) -> Result<(), String> {
    let issue = gh_issue_view_for_phase(&args.repo, args.issue, args.phase)?;
    validate_current_g3_issue(args.phase, &issue)?;
    let delivery_pr = args
        .delivery_pr
        .map(|number| gh_pr_view_for_gate_evidence(&args.repo, number, args.phase))
        .transpose()?;
    let related_prs = args
        .related_prs
        .iter()
        .map(|number| gh_pr_view_for_gate_evidence(&args.repo, *number, args.phase))
        .collect::<Result<Vec<_>, _>>()?;

    validate_current_g3_target(args, delivery_pr.as_ref(), &related_prs)?;

    if let (Some(delivery_number), Some(delivery_pr)) = (args.delivery_pr, delivery_pr.as_ref()) {
        validate_gate_g3_evidence(args, &issue, delivery_pr, &related_prs)?;
        if g3_requires_external_review(delivery_pr)? {
            validate_external_review_g3(
                &args.repo,
                args.issue,
                delivery_number,
                delivery_pr,
                "Delivery PR",
            )?;
        }
        for (number, related_pr) in args.related_prs.iter().zip(&related_prs) {
            if g3_requires_external_review(related_pr)? {
                validate_related_full_set_member(
                    &args.repo, *number, args.issue, args.phase, related_pr,
                )?;
                validate_external_review_g3(
                    &args.repo,
                    args.issue,
                    *number,
                    related_pr,
                    &format!("Related PR #{number}"),
                )?;
            }
        }
        if args.phase == GateEvidencePhase::G4 {
            validate_g4_evidence(args, &issue, delivery_pr, &related_prs)?;
        }
    } else {
        let related_number = args.related_prs[0];
        validate_related_g3_evidence(args, &issue, related_number, &related_prs[0])?;
        if g3_requires_external_review(&related_prs[0])? {
            validate_external_review_g3(
                &args.repo,
                args.issue,
                related_number,
                &related_prs[0],
                &format!("Related PR #{related_number}"),
            )?;
        }
    }
    Ok(())
}

pub(crate) fn check_gate_evidence(args: &[String]) -> Result<(), String> {
    let args = parse_gate_evidence_args(args)?;
    check_gate_evidence_with_args(&args)?;
    print_gate_evidence_success(&args);
    Ok(())
}

pub(crate) fn check_gate_evidence_target(args: &[String]) -> Result<(), String> {
    let (repo, pr_number) = parse_gate_evidence_target_args(args)?;
    let pr = gh_pr_view_for_gate_evidence(&repo, pr_number, GateEvidencePhase::G3)?;
    let (role, issue_numbers) = parse_gate_evidence_target_metadata(&pr.body)?;
    validate_gate_evidence_target_pr(&repo, GateEvidencePhase::G3, &pr, role, &issue_numbers)?;
    let resolved_args = resolve_gate_evidence_targets(
        &repo,
        pr_number,
        role,
        &issue_numbers,
        GateEvidencePhase::G3,
    )?;
    validate_gate_evidence_target_assertions(&pr, &resolved_args)?;
    for args in &resolved_args {
        check_gate_evidence_with_args(args)?;
    }

    let final_pr = gh_pr_view_for_gate_evidence(&repo, pr_number, GateEvidencePhase::G3)?;
    let (final_role, final_issue_numbers) = parse_gate_evidence_target_metadata(&final_pr.body)?;
    validate_gate_evidence_target_pr(
        &repo,
        GateEvidencePhase::G3,
        &final_pr,
        final_role,
        &final_issue_numbers,
    )?;
    let final_args = resolve_gate_evidence_targets(
        &repo,
        pr_number,
        final_role,
        &final_issue_numbers,
        GateEvidencePhase::G3,
    )?;
    if final_role != role || final_issue_numbers != issue_numbers || final_args != resolved_args {
        return Err("PR / Issue Gate 元数据在 target 校验期间发生变化；请重新运行".to_string());
    }
    validate_gate_evidence_target_assertions(&final_pr, &final_args)?;
    for args in &final_args {
        check_gate_evidence_with_args(args)?;
        print_gate_evidence_success(args);
    }
    Ok(())
}

pub(crate) fn check_g3_shadow_success_eligibility(args: &[String]) -> Result<(), String> {
    let (repo, pr_number) = parse_gate_evidence_target_args(args)?;
    let pr = gh_pr_view_for_gate_evidence(&repo, pr_number, GateEvidencePhase::G3)?;
    let (role, issue_numbers) = parse_gate_evidence_target_metadata(&pr.body)?;
    validate_gate_evidence_target_pr(&repo, GateEvidencePhase::G3, &pr, role, &issue_numbers)?;
    let resolved_args = resolve_gate_evidence_targets(
        &repo,
        pr_number,
        role,
        &issue_numbers,
        GateEvidencePhase::G3,
    )?;
    validate_gate_evidence_target_assertions(&pr, &resolved_args)?;
    validate_g3_shadow_success_pr(&pr, &format!("PR #{pr_number}"))?;

    let mut checked_related_prs = BTreeSet::new();
    for resolved in &resolved_args {
        for related_number in &resolved.related_prs {
            if *related_number == pr_number {
                continue;
            }
            let related_pr =
                gh_pr_view_for_gate_evidence(&repo, *related_number, GateEvidencePhase::G3)?;
            if g3_requires_external_review(&related_pr)? {
                validate_related_full_set_member(
                    &repo,
                    *related_number,
                    resolved.issue,
                    GateEvidencePhase::G3,
                    &related_pr,
                )?;
                if checked_related_prs.insert(*related_number) {
                    validate_g3_shadow_success_pr(
                        &related_pr,
                        &format!("Related PR #{related_number}"),
                    )?;
                }
            }
        }
    }
    println!(
        "已确认 G3 shadow success eligibility：PR #{pr_number} 的完整 current-evidence PR 集合均使用非过期型 Gate 结果"
    );
    Ok(())
}

pub(crate) fn check_g3_evidence_marker(args: &[String]) -> Result<(), String> {
    let (repo, pr_number, comment_id) = parse_g3_evidence_marker_args(args)?;
    let marker_before = gh_issue_comment(&repo, comment_id)?;
    validate_g3_evidence_marker_comment(&marker_before, &repo, pr_number)?;

    let pr = gh_pr_view_for_gate_evidence(&repo, pr_number, GateEvidencePhase::G3)?;
    let (role, issue_numbers) = parse_gate_evidence_target_metadata(&pr.body)?;
    validate_gate_evidence_target_pr(&repo, GateEvidencePhase::G3, &pr, role, &issue_numbers)?;
    let resolved_args = resolve_gate_evidence_targets(
        &repo,
        pr_number,
        role,
        &issue_numbers,
        GateEvidencePhase::G3,
    )?;
    let g3_permalink = completed_gate_permalink(&pr.body, "G3")?;
    let g3_comment = pr
        .comments
        .iter()
        .find(|comment| comment.url == g3_permalink)
        .ok_or("PR G3 permalink 未指向当前 PR comment")?;
    validate_marker_is_strictly_later(
        &marker_before.created_at,
        &g3_comment.created_at,
        "current G3 comment",
    )?;

    let relevant_prs = relevant_g3_marker_prs(pr_number, &resolved_args);
    for issue_number in issue_numbers {
        let timestamps = gh_edit_timestamps(&repo, issue_number, GitHubEditTarget::Issue)?;
        validate_marker_after_edit_timestamps(
            &marker_before.created_at,
            &timestamps,
            &format!("Issue #{issue_number} body"),
        )?;
        validate_marker_after_activity_timestamp(
            &marker_before.created_at,
            &timestamps.updated_at,
            false,
            &format!("Issue #{issue_number} activity"),
        )?;
        let timeline = gh_issue_timeline(&repo, issue_number)?;
        validate_marker_after_timeline(
            comment_id,
            &marker_before.created_at,
            &timeline,
            GitHubTimelineTarget::Issue,
            false,
            &format!("Issue #{issue_number}"),
        )?;
    }
    for relevant_pr in relevant_prs {
        let timestamps = gh_edit_timestamps(&repo, relevant_pr, GitHubEditTarget::PullRequest)?;
        let edit_freshness = validate_marker_after_edit_timestamps(
            &marker_before.created_at,
            &timestamps,
            &format!("associated PR #{relevant_pr} body"),
        );
        let activity_freshness = validate_marker_after_activity_timestamp(
            &marker_before.created_at,
            &timestamps.updated_at,
            relevant_pr == pr_number,
            &format!("associated PR #{relevant_pr} activity"),
        );
        if edit_freshness.is_err() || activity_freshness.is_err() {
            let review = crate::external_review::evaluate_live(&repo, relevant_pr)?;
            if !review.uses_dependabot_lockfile_policy() {
                edit_freshness?;
                activity_freshness?;
            }
            let edits = gh_pr_user_content_edits(&repo, relevant_pr)?;
            validate_dependabot_body_edits_after_marker(
                &marker_before.created_at,
                &timestamps,
                &edits,
                &format!("associated PR #{relevant_pr}"),
            )?;
        }
        let timeline = gh_issue_timeline(&repo, relevant_pr)?;
        validate_marker_after_timeline(
            comment_id,
            &marker_before.created_at,
            &timeline,
            GitHubTimelineTarget::PullRequest,
            relevant_pr == pr_number,
            &format!("associated PR #{relevant_pr}"),
        )?;
    }

    let marker_after = gh_issue_comment(&repo, comment_id)?;
    if marker_after != marker_before {
        return Err("G3 evidence marker 在校验期间发生变化；请新增 marker 后重试".to_string());
    }
    println!(
        "已校验 G3 evidence marker：PR #{pr_number}，comment {comment_id} 晚于最终 G3 / body / lifecycle evidence"
    );
    Ok(())
}

pub(crate) fn resolve_g3_evidence_shadow_targets(args: &[String]) -> Result<(), String> {
    let (repo, pr_number) = parse_gate_evidence_target_args(args)?;
    let issue_pages: Vec<Vec<GitHubIssueListEntry>> = gh_json(&[
        "api".to_string(),
        "--paginate".to_string(),
        "--slurp".to_string(),
        format!("repos/{repo}/issues?state=open&per_page=100"),
    ])?;
    let issue_bodies = issue_pages
        .iter()
        .flatten()
        .filter(|issue| issue.pull_request.is_none())
        .filter_map(|issue| issue.body.as_deref())
        .collect::<Vec<_>>();
    let targets = discover_g3_evidence_shadow_targets(pr_number, &issue_bodies)?;
    println!(
        "{}",
        serde_json::to_string(&targets)
            .map_err(|error| format!("无法序列化 G3 evidence shadow targets: {error}"))?
    );
    Ok(())
}

pub(crate) fn resolve_g3_evidence_shadow_issue_event_targets(
    args: &[String],
) -> Result<(), String> {
    let (repo, event_path) = parse_g3_evidence_issue_event_target_args(args)?;
    let event_bytes = fs::read(&event_path).map_err(|error| {
        format!(
            "无法读取 GitHub issues event `{}`: {error}",
            event_path.display()
        )
    })?;
    let event: GitHubIssuesEvent = serde_json::from_slice(&event_bytes).map_err(|error| {
        format!(
            "GitHub issues event `{}` 不是预期 JSON: {error}",
            event_path.display()
        )
    })?;
    if event.repository.full_name != repo {
        return Err(format!(
            "GitHub issues event repository 与 --repo 不一致：event={}；repo={repo}",
            event.repository.full_name
        ));
    }
    if event.issue.number == 0 || event.issue.pull_request.is_some() {
        return Err("GitHub issues event 必须指向正整数编号的非 PR Issue".to_string());
    }

    let targets = discover_g3_evidence_shadow_issue_event_targets(&event)?;
    println!(
        "{}",
        serde_json::to_string(&targets).map_err(|error| format!(
            "无法序列化 G3 evidence shadow Issue event targets: {error}"
        ))?
    );
    Ok(())
}
