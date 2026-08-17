//! G3 target、Shadow eligibility 与 Related/Delivery evidence 验证。

use std::collections::BTreeSet;

use super::g4::{
    has_late_related_pr, reject_inapplicable_g4_recovery_marker, validate_g4_g3_full_set_recovery,
};
use super::{args::*, document::*, github::*, model::*};

pub(super) fn validate_g3_shadow_success_pr(
    pr: &GitHubPullRequest,
    label: &str,
) -> Result<(), String> {
    let permalink = completed_gate_permalink(&pr.body, "G3")?;
    let comment = pr
        .comments
        .iter()
        .find(|comment| comment.url == permalink)
        .ok_or_else(|| format!("{label} G3 permalink 未指向当前 PR comment"))?;
    validate_g3_shadow_success_result(parse_g3_result(&comment.body)?)
        .map_err(|error| format!("{label}: {error}"))
}

pub(super) fn validate_g3_shadow_success_result(result: G3Result) -> Result<(), String> {
    match result {
        G3Result::Pass | G3Result::Bootstrap => Ok(()),
        G3Result::Waived => Err(
            "G3 Waived 是有到期时间的 action-required 证据，G3 Evidence Gate Shadow 不得发布 success"
                .to_string(),
        ),
    }
}

pub(super) fn relevant_g3_marker_prs(
    pr_number: u64,
    resolved_args: &[GateEvidenceArgs],
) -> BTreeSet<u64> {
    let mut relevant_prs = BTreeSet::from([pr_number]);
    for args in resolved_args {
        relevant_prs.extend(args.delivery_pr);
        relevant_prs.extend(args.related_prs.iter().copied());
    }
    relevant_prs
}

pub(super) fn discover_g3_evidence_shadow_targets(
    pr_number: u64,
    issue_bodies: &[&str],
) -> Result<Vec<u64>, String> {
    let mut targets = BTreeSet::from([pr_number]);
    for body in issue_bodies {
        let related_lines = body
            .lines()
            .filter(|line| line.starts_with("- Related PRs："))
            .collect::<Vec<_>>();
        if !related_lines
            .iter()
            .any(|line| metadata_issue_numbers(line).contains(&pr_number))
        {
            continue;
        }
        let related_line = unique_metadata_line(body, "Related PRs")?;
        let related_prs = parse_related_pr_selection(related_line)?;
        if !related_prs.contains(&pr_number) {
            return Err(format!(
                "Issue 的 `Related PRs` 字段无法精确确认 PR #{pr_number}；拒绝猜测级联目标"
            ));
        }
        let delivery_line = unique_metadata_line(body, "Delivery PR")?;
        targets.extend(parse_delivery_pr_selection(delivery_line, false)?);
    }
    Ok(targets.into_iter().collect())
}

pub(super) fn discover_g3_evidence_shadow_issue_targets(
    issue_body: &str,
) -> Result<Vec<u64>, String> {
    let delivery_count = issue_body
        .lines()
        .filter(|line| line.starts_with("- Delivery PR："))
        .count();
    let related_count = issue_body
        .lines()
        .filter(|line| line.starts_with("- Related PRs："))
        .count();
    if delivery_count == 0 && related_count == 0 {
        return Ok(Vec::new());
    }
    if delivery_count != 1 || related_count != 1 {
        return Err(
            "受治理 Issue 必须各包含一个 `Delivery PR` 与 `Related PRs` 元数据字段".to_string(),
        );
    }

    let delivery_line = unique_metadata_line(issue_body, "Delivery PR")?;
    let related_line = unique_metadata_line(issue_body, "Related PRs")?;
    let mut targets = parse_delivery_pr_selection(delivery_line, false)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    targets.extend(parse_related_pr_selection(related_line)?);
    Ok(targets.into_iter().collect())
}

pub(super) fn discover_g3_evidence_shadow_issue_event_targets(
    event: &GitHubIssuesEvent,
) -> Result<Vec<u64>, String> {
    let mut targets = BTreeSet::new();
    if let Some(previous_body) = event
        .changes
        .body
        .as_ref()
        .and_then(|change| change.from.as_deref())
    {
        targets.extend(discover_g3_evidence_shadow_issue_targets(previous_body)?);
    }
    targets.extend(discover_g3_evidence_shadow_issue_targets(
        event.issue.body.as_deref().unwrap_or_default(),
    )?);
    Ok(targets.into_iter().collect())
}

pub(super) struct ResolvedGateEvidenceIssue {
    pub(super) args: GateEvidenceArgs,
    pub(super) issue: GitHubIssue,
}

pub(super) fn resolve_gate_evidence_target_issues(
    repo: &str,
    pr_number: u64,
    role: GateEvidencePrRole,
    issue_numbers: &[u64],
    issue_phase: GateEvidencePhase,
) -> Result<Vec<ResolvedGateEvidenceIssue>, String> {
    issue_numbers
        .iter()
        .map(|issue_number| {
            let issue = gh_issue_view_for_phase(repo, *issue_number, issue_phase)?;
            validate_current_g3_issue(issue_phase, &issue)?;
            let args = resolve_gate_evidence_target_args(
                repo.to_string(),
                pr_number,
                role,
                *issue_number,
                &issue.body,
            )?;
            Ok(ResolvedGateEvidenceIssue { args, issue })
        })
        .collect()
}

pub(super) fn resolve_gate_evidence_targets(
    repo: &str,
    pr_number: u64,
    role: GateEvidencePrRole,
    issue_numbers: &[u64],
    issue_phase: GateEvidencePhase,
) -> Result<Vec<GateEvidenceArgs>, String> {
    resolve_gate_evidence_target_issues(repo, pr_number, role, issue_numbers, issue_phase)
        .map(|resolved| resolved.into_iter().map(|resolved| resolved.args).collect())
}

pub(super) fn validate_g3_target(
    mode: G3ValidationMode,
    repo: &str,
    pr_number: u64,
    issue_phase: GateEvidencePhase,
    pr: &GitHubPullRequest,
    resolved_args: &[GateEvidenceArgs],
) -> Result<(GateEvidencePrRole, Vec<u64>), String> {
    let validation = || {
        let (declared_role, issue_numbers) = parse_gate_evidence_target_metadata(&pr.body)?;
        if mode == G3ValidationMode::RelatedOnly && declared_role != GateEvidencePrRole::Related {
            return Err(format!(
                "Related-only 的预期 PR 角色为 Related PR；实际声明为 {}",
                format_pr_role(declared_role)
            ));
        }
        if resolved_args.is_empty() {
            return Err("预期至少一个解析后的 G3 target 命令；实际为 []".to_string());
        }
        if let Some(args) = resolved_args.iter().find(|args| args.repo != repo) {
            return Err(format!(
                "解析后的 G3 target 仓库不一致：预期 `{repo}`；实际 `{}`",
                args.repo
            ));
        }
        if let Some(args) = resolved_args
            .iter()
            .find(|args| args.phase != GateEvidencePhase::G3)
        {
            return Err(format!(
                "解析后的 target 命令阶段不一致：预期 G3；实际 {:?}",
                args.phase
            ));
        }

        let resolved_issue_numbers = resolved_args
            .iter()
            .map(|args| args.issue)
            .collect::<BTreeSet<_>>();
        let declared_issue_numbers = issue_numbers.iter().copied().collect::<BTreeSet<_>>();
        if resolved_issue_numbers.len() != resolved_args.len()
            || resolved_issue_numbers != declared_issue_numbers
        {
            return Err(format!(
                "G3 target Issue 集合不一致：预期声明 [{}]；实际解析 [{}]；实际命令 [{}]",
                format_issue_numbers(&declared_issue_numbers),
                format_issue_numbers(&resolved_issue_numbers),
                format_g3_target_commands(resolved_args)
            ));
        }

        let resolved_role = resolve_g3_target_role(pr_number, resolved_args)?;
        if resolved_role != declared_role {
            return Err(format!(
                "G3 target PR 角色不一致：预期声明 {}；实际命令解析为 {}；实际命令 [{}]",
                format_pr_role(declared_role),
                format_pr_role(resolved_role),
                format_g3_target_commands(resolved_args)
            ));
        }

        validate_gate_evidence_target_pr(repo, issue_phase, pr, declared_role, &issue_numbers)?;
        validate_gate_evidence_target_assertions(pr, resolved_args)?;
        Ok((declared_role, issue_numbers))
    };

    validation().map_err(|error| {
        format!(
            "G3 target 校验失败：mode={}；PR #{}；{error}",
            mode.label(),
            pr_number
        )
    })
}

fn resolve_g3_target_role(
    pr_number: u64,
    resolved_args: &[GateEvidenceArgs],
) -> Result<GateEvidencePrRole, String> {
    let all_delivery = resolved_args
        .iter()
        .all(|args| args.delivery_pr == Some(pr_number));
    let all_related = resolved_args
        .iter()
        .all(|args| args.delivery_pr.is_none() && args.related_prs.as_slice() == [pr_number]);
    match (all_delivery, all_related) {
        (true, false) => Ok(GateEvidencePrRole::Delivery),
        (false, true) => Ok(GateEvidencePrRole::Related),
        _ => Err(format!(
            "无法从解析后的命令唯一确认当前 PR 角色：预期所有命令都以 PR #{pr_number} 作为 Delivery 或唯一 Related；实际命令 [{}]",
            format_g3_target_commands(resolved_args)
        )),
    }
}

fn format_pr_role(role: GateEvidencePrRole) -> &'static str {
    match role {
        GateEvidencePrRole::Delivery => "Delivery PR",
        GateEvidencePrRole::Related => "Related PR",
    }
}

pub(super) fn format_g3_target_commands(resolved_args: &[GateEvidenceArgs]) -> String {
    resolved_args
        .iter()
        .map(|args| expected_gate_command(args, GateEvidencePhase::G3))
        .collect::<Vec<_>>()
        .join("；")
}

pub(super) fn validate_related_full_set_member(
    repo: &str,
    pr_number: u64,
    current_issue: u64,
    issue_phase: GateEvidencePhase,
    pr: &GitHubPullRequest,
) -> Result<(), String> {
    let (role, issue_numbers) = parse_gate_evidence_target_metadata(&pr.body)?;
    if role != GateEvidencePrRole::Related {
        return Err("Delivery full-set 成员的 `PR 角色` 必须精确为 `Related PR`".to_string());
    }
    if !issue_numbers.contains(&current_issue) {
        return Err(format!(
            "Delivery full-set 的 Related PR `关联 Issue` 未包含当前 Issue #{current_issue}"
        ));
    }
    let resolved_args =
        resolve_gate_evidence_targets(repo, pr_number, role, &issue_numbers, issue_phase)?;
    validate_g3_target(
        G3ValidationMode::DeliveryFullSet,
        repo,
        pr_number,
        issue_phase,
        pr,
        &resolved_args,
    )
    .map(|_| ())
}

#[cfg(test)]
pub(super) fn validate_related_full_set_member_metadata(
    repo: &str,
    issue_phase: GateEvidencePhase,
    current_issue: u64,
    pr: &GitHubPullRequest,
) -> Result<Vec<u64>, String> {
    let (role, issue_numbers) = parse_gate_evidence_target_metadata(&pr.body)?;
    if role != GateEvidencePrRole::Related {
        return Err("Delivery full-set 成员的 `PR 角色` 必须精确为 `Related PR`".to_string());
    }
    if !issue_numbers.contains(&current_issue) {
        return Err(format!(
            "Delivery full-set 的 Related PR `关联 Issue` 未包含当前 Issue #{current_issue}"
        ));
    }
    validate_gate_evidence_target_pr(repo, issue_phase, pr, role, &issue_numbers)?;
    Ok(issue_numbers)
}

pub(super) fn print_gate_evidence_success(args: &GateEvidenceArgs) {
    if let Some(delivery_number) = args.delivery_pr {
        println!(
            "已校验 Gate {} 远端证据：Issue #{}，Delivery PR #{}",
            match args.phase {
                GateEvidencePhase::G3 => "G3",
                GateEvidencePhase::G4 => "G4",
            },
            args.issue,
            delivery_number
        );
    } else {
        println!(
            "已校验 Gate G3 远端证据：Issue #{}，Related PR #{}",
            args.issue, args.related_prs[0]
        );
    }
}

pub(super) fn validate_g3_evidence(
    args: &GateEvidenceArgs,
    issue: &GitHubIssue,
    delivery_pr: &GitHubPullRequest,
    related_prs: &[GitHubPullRequest],
) -> Result<(), String> {
    let delivery_number = args
        .delivery_pr
        .expect("full-set G3 validation requires a Delivery PR");
    let issue_g3_line = completed_gate_line(&issue.body, "G3")?;
    let delivery_pr_line = metadata_line(&issue.body, "Delivery PR")?;
    if !delivery_pr_line.contains(&format!("#{delivery_number}")) {
        return Err(format!(
            "Issue 的 `Delivery PR` 字段未记录 Delivery PR #{}",
            delivery_number
        ));
    }
    let related_prs_line = metadata_line(&issue.body, "Related PRs")?;
    let recorded_related_prs = metadata_issue_numbers(related_prs_line);
    let requested_related_prs = args.related_prs.iter().copied().collect::<BTreeSet<_>>();
    if recorded_related_prs != requested_related_prs {
        return Err(format!(
            "Issue 的 `Related PRs` 字段与命令参数不一致：Issue 记录 [{}]；命令传入 [{}]",
            format_issue_numbers(&recorded_related_prs),
            format_issue_numbers(&requested_related_prs)
        ));
    }
    let delivery_permalink = completed_gate_permalink(&delivery_pr.body, "G3")?;
    if !line_links_to_comment_permalink(&issue.body, issue_g3_line, &delivery_permalink) {
        return Err(format!(
            "Issue 的 G3 checkbox 未回链 Delivery PR 的 G3 comment permalink：预期 `{delivery_permalink}`；实际 Gate Ledger `{issue_g3_line}`"
        ));
    }
    validate_comment(
        delivery_pr,
        &delivery_permalink,
        G3_COMMENT_FIELDS,
        "Delivery PR G3",
        args,
    )?;
    validate_g3_timing(delivery_pr, &delivery_permalink, "Delivery PR")?;
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

    for (number, related_pr) in args.related_prs.iter().zip(related_prs) {
        let related_args = GateEvidenceArgs {
            phase: GateEvidencePhase::G3,
            repo: args.repo.clone(),
            issue: args.issue,
            delivery_pr: None,
            related_prs: vec![*number],
        };
        validate_related_pr_g3(
            &related_args,
            &issue.body,
            issue_g3_line,
            *number,
            related_pr,
        )?;
    }
    Ok(())
}

pub(super) fn validate_gate_g3_evidence(
    args: &GateEvidenceArgs,
    issue: &GitHubIssue,
    delivery_pr: &GitHubPullRequest,
    related_prs: &[GitHubPullRequest],
) -> Result<(), String> {
    if args.phase == GateEvidencePhase::G4 {
        if has_late_related_pr(args, delivery_pr, related_prs)? {
            return validate_g4_g3_full_set_recovery(args, issue, delivery_pr, related_prs);
        }
        reject_inapplicable_g4_recovery_marker(issue)?;
    }
    validate_g3_evidence(args, issue, delivery_pr, related_prs)
}

pub(super) fn validate_related_g3_evidence(
    args: &GateEvidenceArgs,
    issue: &GitHubIssue,
    related_number: u64,
    related_pr: &GitHubPullRequest,
) -> Result<(), String> {
    metadata_line(&issue.body, "Delivery PR")?;
    let related_prs_line = metadata_line(&issue.body, "Related PRs")?;
    let recorded_related_prs = metadata_issue_numbers(related_prs_line);
    if !recorded_related_prs.contains(&related_number) {
        return Err(format!(
            "Issue 的 `Related PRs` 字段未记录 Related PR #{related_number}"
        ));
    }
    let issue_g3_line = pending_gate_line(&issue.body, "G3")?;
    validate_related_pr_g3(args, &issue.body, issue_g3_line, related_number, related_pr)
}

pub(super) fn validate_related_pr_g3(
    args: &GateEvidenceArgs,
    issue_body: &str,
    issue_g3_line: &str,
    number: u64,
    related_pr: &GitHubPullRequest,
) -> Result<(), String> {
    let permalink = completed_gate_permalink(&related_pr.body, "G3")?;
    if !line_links_to_comment_permalink(issue_body, issue_g3_line, &permalink) {
        return Err(format!(
            "Issue 的 G3 Gate Ledger 未回链 Related PR #{number} 的 G3 comment permalink：预期 `{permalink}`；实际 Gate Ledger `{issue_g3_line}`"
        ));
    }
    let label = format!("Related PR #{number} G3");
    validate_comment(related_pr, &permalink, G3_COMMENT_FIELDS, &label, args)?;
    validate_g3_timing(related_pr, &permalink, &label)?;
    if related_pr
        .closing_issues_references
        .iter()
        .any(|reference| reference.number == args.issue)
    {
        return Err(format!(
            "Related PR #{number} 不得以 closing keyword 覆盖 Issue #{}",
            args.issue
        ));
    }
    if !related_pr.body.contains(&format!("Refs: #{}", args.issue)) {
        return Err(format!(
            "Related PR #{number} 缺少 `Refs: #{}` 关系记录",
            args.issue
        ));
    }
    Ok(())
}
