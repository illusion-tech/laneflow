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

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use args::*;
use document::*;
use g3::*;
#[cfg(test)]
use g4::*;
use g4::{validate_g4_evidence, validate_live_merge_queue_g4_evidence};
use github::*;
use model::*;

struct G3TargetContext {
    pr_number: u64,
    pr: GitHubPullRequest,
    role: GateEvidencePrRole,
    issue_numbers: Vec<u64>,
    resolved_args: Vec<GateEvidenceArgs>,
    issues: BTreeMap<u64, GitHubIssue>,
}

#[derive(Clone, Copy)]
struct CachedGateEvidence<'a> {
    issue_number: u64,
    issue: &'a GitHubIssue,
    pr_number: u64,
    pr: &'a GitHubPullRequest,
}

fn gh_pr_view_for_gate_evidence(
    repo: &str,
    number: u64,
    phase: GateEvidencePhase,
) -> Result<GitHubPullRequest, String> {
    let mut pr = gh_pr_view_for_phase(repo, number, phase)?;
    if parse_gate_evidence_target_metadata(&pr.body).is_ok() {
        return Ok(pr);
    }

    // G4 会复核同一份 pre-merge G3 证据，因此也必须复用这条经过完整验证的窄
    // Dependabot 元数据恢复路径，不能重新读取已被 Dependabot 改坏的原始 body。
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
            "{metadata_error}；仅精确 dependabot-cargo-lock-only-v1 可从 current G3 comment 恢复 target 元数据"
        ));
    }
    let edits = gh_pr_user_content_edits(repo, number)?;
    let g3_effective_at = g3_comment_effective_at(comment, "current G3 comment")?;
    validate_dependabot_body_edits_after_g3_comment(
        g3_effective_at,
        &edits,
        &format!("PR #{number}"),
    )?;
    pr.body = recovered_gate_evidence_target_body(repo, number, &permalink, &comment.body)?;
    Ok(pr)
}

fn recovered_gate_evidence_target_body(
    repo: &str,
    number: u64,
    permalink: &str,
    comment_body: &str,
) -> Result<String, String> {
    let (role, issue_numbers) =
        parse_gate_evidence_target_metadata_from_g3_comment(repo, number, comment_body)?;
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
    Ok(format!(
        "- 关联 Issue：{issues}\n- PR 角色：`{role}`\n- [x] G3 合并判断已记录：{permalink}\n{refs}"
    ))
}

fn check_gate_evidence_with_args(args: &GateEvidenceArgs) -> Result<(), String> {
    check_gate_evidence_with_loaders(
        args,
        None,
        gh_issue_view_for_phase,
        gh_pr_view_for_gate_evidence,
    )?;
    if args.phase == GateEvidencePhase::G4 {
        validate_live_merge_queue_g4_evidence(args)?;
    }
    Ok(())
}

fn check_gate_evidence_with_target(
    args: &GateEvidenceArgs,
    target: &G3TargetContext,
) -> Result<(), String> {
    let issue = target.issues.get(&args.issue).ok_or_else(|| {
        format!(
            "G3 target 缺少已解析的 Issue #{} snapshot；拒绝退回重复远端读取",
            args.issue
        )
    })?;
    check_gate_evidence_with_loaders(
        args,
        Some(CachedGateEvidence {
            issue_number: args.issue,
            issue,
            pr_number: target.pr_number,
            pr: &target.pr,
        }),
        gh_issue_view_for_phase,
        gh_pr_view_for_gate_evidence,
    )
}

fn check_gate_evidence_with_loaders<FI, FP>(
    args: &GateEvidenceArgs,
    cached: Option<CachedGateEvidence<'_>>,
    mut load_issue: FI,
    mut load_pr: FP,
) -> Result<(), String>
where
    FI: FnMut(&str, u64, GateEvidencePhase) -> Result<GitHubIssue, String>,
    FP: FnMut(&str, u64, GateEvidencePhase) -> Result<GitHubPullRequest, String>,
{
    let (selected_role, selected_pr_number) = selected_gate_evidence_pr(args)?;
    if let Some(cached) = cached {
        if cached.issue_number != args.issue {
            return Err(format!(
                "缓存的 Issue snapshot 与命令不一致：缓存 #{}；命令 #{}",
                cached.issue_number, args.issue
            ));
        }
        if cached.pr_number != selected_pr_number {
            return Err(format!(
                "缓存的 PR snapshot 与命令不一致：缓存 #{}；命令 #{}",
                cached.pr_number, selected_pr_number
            ));
        }
    }

    let loaded_issue;
    let issue = if let Some(cached) = cached {
        cached.issue
    } else {
        loaded_issue = load_issue(&args.repo, args.issue, args.phase)?;
        &loaded_issue
    };
    validate_current_g3_issue(args.phase, issue)?;
    let historical_exception_appendix = if args.phase == GateEvidencePhase::G4 {
        let permalink = completed_gate_permalink(&issue.body, "G4")?;
        Some(comment_for_permalink(
            issue,
            &permalink,
            "Issue G4 exception appendix",
        )?)
    } else {
        None
    };

    if selected_role == GateEvidencePrRole::Delivery {
        let delivery_number = selected_pr_number;
        let loaded_delivery_pr;
        let delivery_pr = if let Some(cached) = cached {
            cached.pr
        } else {
            loaded_delivery_pr = load_pr(&args.repo, delivery_number, args.phase)?;
            &loaded_delivery_pr
        };
        let related_prs = args
            .related_prs
            .iter()
            .map(|number| load_pr(&args.repo, *number, args.phase))
            .collect::<Result<Vec<_>, _>>()?;

        validate_current_g3_target(args, Some(delivery_pr), &related_prs)?;
        validate_gate_g3_evidence(args, issue, delivery_pr, &related_prs)?;
        if g3_requires_result_validation(
            args.issue,
            delivery_number,
            delivery_pr,
            historical_exception_appendix,
        )? {
            validate_external_review_g3(
                &args.repo,
                args.issue,
                delivery_number,
                delivery_pr,
                "Delivery PR",
                historical_exception_appendix,
                ExternalReviewG3Validation {
                    phase: args.phase,
                    exception_gate_time: (args.phase == GateEvidencePhase::G4)
                        .then_some(delivery_pr.merged_at.as_deref())
                        .flatten(),
                    ordinary_waiver_merged_at: None,
                },
            )?;
        }
        for (number, related_pr) in args.related_prs.iter().zip(&related_prs) {
            if g3_requires_result_validation(
                args.issue,
                *number,
                related_pr,
                historical_exception_appendix,
            )? {
                let related_permalink = completed_gate_permalink(&related_pr.body, "G3")?;
                let allow_legacy_exception = historical_exception_appendix
                    .map(|appendix| {
                        historical_exception_applies_to_target(
                            appendix,
                            args.issue,
                            *number,
                            &related_permalink,
                        )
                    })
                    .transpose()?
                    .unwrap_or(false);
                validate_related_full_set_member(
                    &args.repo,
                    *number,
                    args.issue,
                    args.phase,
                    related_pr,
                    allow_legacy_exception,
                )?;
                validate_external_review_g3(
                    &args.repo,
                    args.issue,
                    *number,
                    related_pr,
                    &format!("Related PR #{number}"),
                    historical_exception_appendix,
                    ExternalReviewG3Validation {
                        phase: args.phase,
                        exception_gate_time: related_pr.merged_at.as_deref(),
                        ordinary_waiver_merged_at: related_pr.merged_at.as_deref(),
                    },
                )?;
            }
        }
        if args.phase == GateEvidencePhase::G4 {
            validate_g4_evidence(args, issue, delivery_pr, &related_prs)?;
        }
    } else {
        let related_number = selected_pr_number;
        let loaded_related_pr;
        let related_pr = if let Some(cached) = cached {
            cached.pr
        } else {
            loaded_related_pr = load_pr(&args.repo, related_number, args.phase)?;
            &loaded_related_pr
        };
        validate_current_g3_target(args, None, std::slice::from_ref(related_pr))?;
        validate_related_g3_evidence(args, issue, related_number, related_pr)?;
        if g3_requires_result_validation(
            args.issue,
            related_number,
            related_pr,
            historical_exception_appendix,
        )? {
            validate_external_review_g3(
                &args.repo,
                args.issue,
                related_number,
                related_pr,
                &format!("Related PR #{related_number}"),
                historical_exception_appendix,
                ExternalReviewG3Validation {
                    phase: args.phase,
                    exception_gate_time: None,
                    ordinary_waiver_merged_at: None,
                },
            )?;
        }
    }
    Ok(())
}

fn resolve_and_validate_g3_target(
    repo: &str,
    pr_number: u64,
    mode: G3ValidationMode,
    issue_phase: GateEvidencePhase,
) -> Result<G3TargetContext, String> {
    let pr = gh_pr_view_for_gate_evidence(repo, pr_number, issue_phase)?;
    let (role, issue_numbers) = parse_gate_evidence_target_metadata(&pr.body)?;
    let resolved_issues =
        resolve_gate_evidence_target_issues(repo, pr_number, role, &issue_numbers, issue_phase)?;
    let resolved_args = resolved_issues
        .iter()
        .map(|resolved| resolved.args.clone())
        .collect::<Vec<_>>();
    validate_g3_target(mode, repo, pr_number, issue_phase, &pr, &resolved_args)?;
    let issues = resolved_issues
        .into_iter()
        .map(|resolved| (resolved.args.issue, resolved.issue))
        .collect();
    Ok(G3TargetContext {
        pr_number,
        pr,
        role,
        issue_numbers,
        resolved_args,
        issues,
    })
}

pub(crate) fn check_gate_evidence(args: &[String]) -> Result<(), String> {
    let args = parse_gate_evidence_args(args)?;
    if args.phase == GateEvidencePhase::G3 {
        let (role, pr_number) = selected_gate_evidence_pr(&args)?;
        let mode = match role {
            GateEvidencePrRole::Delivery => G3ValidationMode::DeliveryFullSet,
            GateEvidencePrRole::Related => G3ValidationMode::RelatedOnly,
        };
        let target = resolve_and_validate_g3_target(&args.repo, pr_number, mode, args.phase)?;
        if !target.resolved_args.contains(&args) {
            return Err(format!(
                "G3 target 参数不一致：mode={}；PR #{}；预期 target 命令 [{}]；实际调用 [{}]",
                mode.label(),
                pr_number,
                format_g3_target_commands(&target.resolved_args),
                expected_gate_command(&args, GateEvidencePhase::G3)
            ));
        }
        check_gate_evidence_with_target(&args, &target)?;
    } else {
        check_gate_evidence_with_args(&args)?;
    }
    print_gate_evidence_success(&args)?;
    Ok(())
}

pub(crate) fn check_gate_evidence_target(args: &[String]) -> Result<(), String> {
    let (repo, pr_number) = parse_gate_evidence_target_args(args)?;
    let target = resolve_and_validate_g3_target(
        &repo,
        pr_number,
        G3ValidationMode::ShadowTarget,
        GateEvidencePhase::G3,
    )?;
    for args in &target.resolved_args {
        check_gate_evidence_with_target(args, &target)?;
    }

    let final_target = resolve_and_validate_g3_target(
        &repo,
        pr_number,
        G3ValidationMode::ShadowTarget,
        GateEvidencePhase::G3,
    )?;
    if final_target.role != target.role
        || final_target.issue_numbers != target.issue_numbers
        || final_target.resolved_args != target.resolved_args
    {
        return Err("PR / Issue Gate 元数据在 target 校验期间发生变化；请重新运行".to_string());
    }
    for args in &final_target.resolved_args {
        check_gate_evidence_with_target(args, &final_target)?;
        print_gate_evidence_success(args)?;
    }
    Ok(())
}

pub(crate) fn check_g3_shadow_success_eligibility(args: &[String]) -> Result<(), String> {
    let (repo, pr_number) = parse_gate_evidence_target_args(args)?;
    let target = resolve_and_validate_g3_target(
        &repo,
        pr_number,
        G3ValidationMode::ShadowTarget,
        GateEvidencePhase::G3,
    )?;
    validate_g3_shadow_success_pr(&target.pr, &format!("PR #{pr_number}"))?;

    let mut checked_related_prs = BTreeSet::new();
    for resolved in &target.resolved_args {
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
                    false,
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

    let target = resolve_and_validate_g3_target(
        &repo,
        pr_number,
        G3ValidationMode::ShadowTarget,
        GateEvidencePhase::G3,
    )?;
    let G3TargetContext {
        pr,
        issue_numbers,
        resolved_args,
        ..
    } = target;
    let g3_permalink = completed_gate_permalink(&pr.body, "G3")?;
    let g3_comment = pr
        .comments
        .iter()
        .find(|comment| comment.url == g3_permalink)
        .ok_or("PR G3 permalink 未指向当前 PR comment")?;
    let g3_effective_at = g3_comment_effective_at(g3_comment, "current G3 comment")?;
    validate_marker_is_strictly_later(
        &marker_before.created_at,
        g3_effective_at,
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
