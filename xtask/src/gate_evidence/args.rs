//! CLI 参数、PR 角色与 Issue 元数据到验证目标的解析。

use std::collections::BTreeSet;
use std::path::PathBuf;

use super::{document::*, model::*};

pub(super) fn parse_gate_evidence_target_args(args: &[String]) -> Result<(String, u64), String> {
    let mut repo = None;
    let mut pr = None;
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        let value = args.get(index + 1).ok_or_else(|| {
            format!(
                "`{flag}` 缺少值。用法：check-gate-evidence-target --repo <owner/repo> --pr <number>"
            )
        })?;
        match flag.as_str() {
            "--repo" => {
                if repo.replace(value.clone()).is_some() {
                    return Err("`--repo` 只能指定一次".to_string());
                }
            }
            "--pr" => {
                if pr.replace(parse_issue_number("--pr", value)?).is_some() {
                    return Err("`--pr` 只能指定一次".to_string());
                }
            }
            _ => return Err(format!("未知 check-gate-evidence-target 参数：{flag}")),
        }
        index += 2;
    }

    let repo = repo.ok_or("缺少 `--repo <owner/repo>`")?;
    if !valid_repository_name(&repo) {
        return Err(format!("`--repo` 格式不正确：{repo}，应为 `owner/repo`"));
    }
    Ok((repo, pr.ok_or("缺少 `--pr <number>`")?))
}

pub(super) fn parse_g3_evidence_issue_event_target_args(
    args: &[String],
) -> Result<(String, PathBuf), String> {
    let mut repo = None;
    let mut event_path = None;
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        let value = args.get(index + 1).ok_or_else(|| {
            format!(
                "`{flag}` 缺少值。用法：resolve-g3-evidence-shadow-issue-event-targets --repo <owner/repo> --event-path <path>"
            )
        })?;
        match flag.as_str() {
            "--repo" => {
                if repo.replace(value.clone()).is_some() {
                    return Err("`--repo` 只能指定一次".to_string());
                }
            }
            "--event-path" => {
                if event_path.replace(PathBuf::from(value)).is_some() {
                    return Err("`--event-path` 只能指定一次".to_string());
                }
            }
            _ => {
                return Err(format!(
                    "未知 resolve-g3-evidence-shadow-issue-event-targets 参数：{flag}"
                ));
            }
        }
        index += 2;
    }

    let repo = repo.ok_or("缺少 `--repo <owner/repo>`")?;
    if !valid_repository_name(&repo) {
        return Err(format!("`--repo` 格式不正确：{repo}，应为 `owner/repo`"));
    }
    Ok((repo, event_path.ok_or("缺少 `--event-path <path>`")?))
}

pub(super) fn parse_g3_evidence_marker_args(args: &[String]) -> Result<(String, u64, u64), String> {
    let mut repo = None;
    let mut pr = None;
    let mut comment_id = None;
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        let value = args.get(index + 1).ok_or_else(|| {
            format!(
                "`{flag}` 缺少值。用法：check-g3-evidence-marker --repo <owner/repo> --pr <number> --comment-id <number>"
            )
        })?;
        match flag.as_str() {
            "--repo" => {
                if repo.replace(value.clone()).is_some() {
                    return Err("`--repo` 只能指定一次".to_string());
                }
            }
            "--pr" => {
                if pr.replace(parse_issue_number("--pr", value)?).is_some() {
                    return Err("`--pr` 只能指定一次".to_string());
                }
            }
            "--comment-id" => {
                if comment_id
                    .replace(parse_issue_number("--comment-id", value)?)
                    .is_some()
                {
                    return Err("`--comment-id` 只能指定一次".to_string());
                }
            }
            _ => return Err(format!("未知 check-g3-evidence-marker 参数：{flag}")),
        }
        index += 2;
    }

    let repo = repo.ok_or("缺少 `--repo <owner/repo>`")?;
    if !valid_repository_name(&repo) {
        return Err(format!("`--repo` 格式不正确：{repo}，应为 `owner/repo`"));
    }
    Ok((
        repo,
        pr.ok_or("缺少 `--pr <number>`")?,
        comment_id.ok_or("缺少 `--comment-id <number>`")?,
    ))
}

pub(super) fn parse_gate_evidence_target_metadata(
    body: &str,
) -> Result<(GateEvidencePrRole, Vec<u64>), String> {
    let issue_line = unique_metadata_line(body, "关联 Issue")?;
    let issue_numbers = parse_concrete_pr_number_list(issue_line, "关联 Issue", true)?;

    let role_line = unique_metadata_line(body, "PR 角色")?;
    let role = role_line
        .strip_prefix("- PR 角色：")
        .expect("metadata_line matched the requested field")
        .trim()
        .trim_matches('`');
    let role = match role {
        "Delivery PR" => GateEvidencePrRole::Delivery,
        "Related PR" => GateEvidencePrRole::Related,
        _ => {
            return Err("PR 的 `PR 角色` 必须精确为 `Delivery PR` 或 `Related PR`".to_string());
        }
    };
    Ok((role, issue_numbers))
}

pub(super) fn parse_gate_evidence_target_metadata_from_g3_comment(
    repo: &str,
    pr_number: u64,
    body: &str,
) -> Result<(GateEvidencePrRole, Vec<u64>), String> {
    let commands = gate_assertion_commands(body, "PR G3", GateEvidencePhase::G3)?;
    let mut role = None;
    let mut issues = BTreeSet::new();

    for command in commands {
        let tokens = command.split_ascii_whitespace().collect::<Vec<_>>();
        let command_index = tokens
            .iter()
            .position(|token| *token == "check-gate-evidence")
            .ok_or("PR G3 comment 的 `Gate 断言` 缺少 `check-gate-evidence`")?;
        let parsed_tokens = tokens[command_index + 1..]
            .iter()
            .map(|token| (*token).to_string())
            .collect::<Vec<_>>();
        let args = parse_gate_evidence_args(&parsed_tokens)?;
        if args.phase != GateEvidencePhase::G3
            || args.repo != repo
            || expected_gate_command(&args, GateEvidencePhase::G3) != command
        {
            return Err(
                "PR G3 comment 的 `Gate 断言` 不是当前 repository 的规范 G3 命令".to_string(),
            );
        }

        let command_role = match (args.delivery_pr, args.related_prs.as_slice()) {
            (Some(number), _) if number == pr_number => GateEvidencePrRole::Delivery,
            (None, [number]) if *number == pr_number => GateEvidencePrRole::Related,
            _ => {
                return Err(format!(
                    "PR G3 comment 的 `Gate 断言` 未把当前 PR #{pr_number} 记录为 Delivery 或唯一 Related target"
                ));
            }
        };
        if role
            .replace(command_role)
            .is_some_and(|current| current != command_role)
        {
            return Err("PR G3 comment 的 `Gate 断言` 混用了 Delivery / Related 角色".to_string());
        }
        if !issues.insert(args.issue) {
            return Err("PR G3 comment 的 `Gate 断言` 重复记录同一关联 Issue".to_string());
        }
    }

    Ok((
        role.ok_or("PR G3 comment 未解析出当前 PR 角色")?,
        issues.into_iter().collect(),
    ))
}

pub(super) fn validate_gate_evidence_target_pr(
    repo: &str,
    phase: GateEvidencePhase,
    pr: &GitHubPullRequest,
    role: GateEvidencePrRole,
    issue_numbers: &[u64],
) -> Result<(), String> {
    let declared_issues = issue_numbers.iter().copied().collect::<BTreeSet<_>>();
    let foreign_closing_issues = pr
        .closing_issues_references
        .iter()
        .filter(|reference| !issue_reference_matches_repository(reference, repo))
        .map(format_issue_reference)
        .collect::<BTreeSet<_>>();
    let closing_issues = pr
        .closing_issues_references
        .iter()
        .filter(|reference| issue_reference_matches_repository(reference, repo))
        .map(|reference| reference.number)
        .collect::<BTreeSet<_>>();
    match role {
        GateEvidencePrRole::Delivery if !foreign_closing_issues.is_empty() => {
            return Err(format!(
                "Delivery PR 的 closingIssuesReferences 必须全部属于 `{repo}`；发现跨仓库引用 [{}]",
                foreign_closing_issues
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        GateEvidencePrRole::Delivery if closing_issues != declared_issues => {
            return Err(format!(
                "Delivery PR 的完整 closingIssuesReferences 必须与 `关联 Issue` 精确一致：声明 [{}]；closing [{}]",
                format_issue_numbers(&declared_issues),
                format_issue_numbers(&closing_issues)
            ));
        }
        GateEvidencePrRole::Related if !pr.closing_issues_references.is_empty() => {
            return Err(format!(
                "Related PR 不得关闭任何 Issue；closingIssuesReferences 实际为 [{}]",
                pr.closing_issues_references
                    .iter()
                    .map(format_issue_reference)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        _ => {}
    }

    let permalink = completed_gate_permalink(&pr.body, "G3")?;
    let comment = pr
        .comments
        .iter()
        .find(|comment| comment.url == permalink)
        .ok_or_else(|| {
            let actual = pr
                .comments
                .iter()
                .map(|comment| comment.url.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "PR G3 permalink 未指向当前 PR comment：预期 `{permalink}`；实际 comment URLs [{actual}]"
            )
        })?;
    if phase == GateEvidencePhase::G3
        && g3_comment_effective_at(comment, "PR G3 comment")? >= G3_EVIDENCE_SHADOW_ACTIVATION
    {
        validate_g3_evidence_shadow_comment_field(&comment.body)?;
    }
    validate_gate_waiver_record_set(comment, &declared_issues)
}

pub(super) fn validate_g3_evidence_shadow_comment_field(body: &str) -> Result<(), String> {
    let line = unique_metadata_line(body, "G3 Evidence Gate Shadow")?;
    let value = metadata_value(line, "G3 Evidence Gate Shadow")?;
    let supported = if let Some(url) = value.strip_prefix("Check URL：") {
        let url = url.trim().trim_end_matches('。').trim_matches('`');
        url.starts_with("https://github.com/")
            && (url.contains("/actions/runs/") || url.contains("/runs/"))
            && !url.chars().any(char::is_whitespace)
    } else if let Some(reason) = value.strip_prefix("R1 non-required：") {
        reason.chars().any(char::is_alphanumeric)
    } else if let Some(boundary) = value.strip_prefix("候选 workflow bootstrap：") {
        boundary.chars().any(char::is_alphanumeric)
    } else {
        false
    };
    if !supported {
        return Err(format!(
            "`{G3_EVIDENCE_SHADOW_COMMENT_FIELD}` 必须唯一且使用 `Check URL：https://github.com/...`、`R1 non-required：<原因>` 或 `候选 workflow bootstrap：<边界>`"
        ));
    }
    Ok(())
}

pub(super) fn issue_reference_matches_repository(reference: &IssueReference, repo: &str) -> bool {
    let Some((owner, name)) = repo.split_once('/') else {
        return false;
    };
    reference.repository.owner.login.eq_ignore_ascii_case(owner)
        && reference.repository.name.eq_ignore_ascii_case(name)
}

pub(super) fn issue_reference_matches(reference: &IssueReference, repo: &str, number: u64) -> bool {
    reference.number == number && issue_reference_matches_repository(reference, repo)
}

pub(super) fn format_issue_reference(reference: &IssueReference) -> String {
    format!(
        "{}/{}#{}",
        reference.repository.owner.login, reference.repository.name, reference.number
    )
}

pub(super) fn validate_gate_evidence_target_assertions(
    pr: &GitHubPullRequest,
    resolved_args: &[GateEvidenceArgs],
) -> Result<(), String> {
    let permalink = completed_gate_permalink(&pr.body, "G3")?;
    let comment = pr
        .comments
        .iter()
        .find(|comment| comment.url == permalink)
        .ok_or("PR G3 permalink 未指向当前 PR comment")?;
    validate_gate_assertion_set(&comment.body, "PR G3", resolved_args, GateEvidencePhase::G3)
}

pub(super) fn resolve_gate_evidence_target_args(
    repo: String,
    pr_number: u64,
    role: GateEvidencePrRole,
    issue_number: u64,
    issue_body: &str,
) -> Result<GateEvidenceArgs, String> {
    let delivery_line = unique_metadata_line(issue_body, "Delivery PR")?;
    let delivery_prs =
        parse_delivery_pr_selection(delivery_line, role == GateEvidencePrRole::Delivery)?;
    let related_line = unique_metadata_line(issue_body, "Related PRs")?;
    let related_prs = parse_related_pr_selection(related_line)?;
    let recorded_related_prs = related_prs.iter().copied().collect::<BTreeSet<_>>();

    let args = match role {
        GateEvidencePrRole::Delivery => {
            if delivery_prs.as_slice() != [pr_number] {
                return Err(format!(
                    "Issue 的 `Delivery PR` 字段必须唯一记录当前 Delivery PR #{pr_number}"
                ));
            }
            GateEvidenceArgs {
                phase: GateEvidencePhase::G3,
                repo,
                issue: issue_number,
                delivery_pr: Some(pr_number),
                related_prs,
            }
        }
        GateEvidencePrRole::Related => {
            if !recorded_related_prs.contains(&pr_number) {
                return Err(format!(
                    "Issue 的 `Related PRs` 字段未记录当前 Related PR #{pr_number}"
                ));
            }
            GateEvidenceArgs {
                phase: GateEvidencePhase::G3,
                repo,
                issue: issue_number,
                delivery_pr: None,
                related_prs: vec![pr_number],
            }
        }
    };
    validate_gate_evidence_args(&args)?;
    Ok(args)
}

pub(super) fn parse_delivery_pr_selection(
    line: &str,
    require_concrete: bool,
) -> Result<Vec<u64>, String> {
    let value = metadata_value(line, "Delivery PR")?;
    if value == "pending" {
        return if require_concrete {
            Err("Issue 的 `Delivery PR` 字段仍为 `pending`".to_string())
        } else {
            Ok(Vec::new())
        };
    }
    if valid_na_reason(value) {
        return if require_concrete {
            Err("Issue 的 `Delivery PR` 字段不能为 `N/A`".to_string())
        } else {
            Ok(Vec::new())
        };
    }
    parse_concrete_pr_number_list(line, "Delivery PR", false)
}

pub(super) fn parse_related_pr_selection(line: &str) -> Result<Vec<u64>, String> {
    let value = metadata_value(line, "Related PRs")?;
    if valid_na_reason(value) {
        return Ok(Vec::new());
    }
    parse_concrete_pr_number_list(line, "Related PRs", true)
}

pub(super) fn parse_concrete_pr_number_list(
    line: &str,
    field: &str,
    allow_multiple: bool,
) -> Result<Vec<u64>, String> {
    let value = metadata_value(line, field)?;
    if value.contains("pending")
        || value.contains("N/A")
        || value.contains("#<")
        || value.contains('/')
    {
        return Err(format!("`{field}` 包含未清理的互斥模板选项：{value}"));
    }
    let tokens = value
        .split(['、', '，', ','])
        .map(str::trim)
        .collect::<Vec<_>>();
    if tokens.is_empty() || tokens.iter().any(|token| token.is_empty()) {
        return Err(format!("`{field}` 必须记录明确的 `#<number>`"));
    }
    if !allow_multiple && tokens.len() != 1 {
        return Err(format!("`{field}` 只能记录一个 PR"));
    }

    let numbers = tokens
        .iter()
        .map(|token| {
            let digits = token
                .strip_prefix('#')
                .ok_or_else(|| format!("`{field}` 的具体值必须使用 `#<number>`：{token}"))?;
            if digits.is_empty() || !digits.chars().all(|character| character.is_ascii_digit()) {
                return Err(format!("`{field}` 包含未清理的模板或说明：{token}"));
            }
            digits
                .parse::<u64>()
                .ok()
                .filter(|number| *number > 0)
                .ok_or_else(|| format!("`{field}` 包含无效 PR 编号：{token}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let unique = numbers.iter().copied().collect::<BTreeSet<_>>();
    if unique.len() != numbers.len() {
        return Err(format!("`{field}` 不得包含重复 PR"));
    }
    Ok(numbers)
}

pub(super) fn metadata_value<'a>(line: &'a str, field: &str) -> Result<&'a str, String> {
    line.strip_prefix(&format!("- {field}："))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("`{field}` 元数据字段缺少明确值"))
}

pub(super) fn valid_na_reason(value: &str) -> bool {
    value
        .strip_prefix("N/A，原因：")
        .is_some_and(|reason| !reason.trim().is_empty() && !reason.contains('#'))
}

pub(super) fn parse_gate_evidence_args(args: &[String]) -> Result<GateEvidenceArgs, String> {
    let phase = args
        .first()
        .ok_or_else(|| "缺少 Gate evidence 阶段，应为 `g3` 或 `g4`".to_string())
        .and_then(|value| GateEvidencePhase::parse(value))?;

    let mut repo = None;
    let mut issue = None;
    let mut delivery_pr = None;
    let mut related_prs = Vec::new();
    let mut index = 1;
    while index < args.len() {
        let flag = &args[index];
        let value = args.get(index + 1).ok_or_else(|| {
            format!(
                "`{flag}` 缺少值。用法：check-gate-evidence g3 --repo <owner/repo> --issue <number> --related-pr <number>；或 check-gate-evidence <g3|g4> --repo <owner/repo> --issue <number> --delivery-pr <number> [--related-pr <number>]..."
            )
        })?;

        match flag.as_str() {
            "--repo" => {
                if repo.replace(value.clone()).is_some() {
                    return Err("`--repo` 只能指定一次".to_string());
                }
            }
            "--issue" => {
                if issue
                    .replace(parse_issue_number("--issue", value)?)
                    .is_some()
                {
                    return Err("`--issue` 只能指定一次".to_string());
                }
            }
            "--delivery-pr" => {
                if delivery_pr
                    .replace(parse_issue_number("--delivery-pr", value)?)
                    .is_some()
                {
                    return Err("`--delivery-pr` 只能指定一次".to_string());
                }
            }
            "--related-pr" => related_prs.push(parse_issue_number("--related-pr", value)?),
            _ => return Err(format!("未知 check-gate-evidence 参数：{flag}")),
        }
        index += 2;
    }

    let repo = repo.ok_or("缺少 `--repo <owner/repo>`")?;
    if !valid_repository_name(&repo) {
        return Err(format!("`--repo` 格式不正确：{repo}，应为 `owner/repo`"));
    }
    let issue = issue.ok_or("缺少 `--issue <number>`")?;
    let parsed = GateEvidenceArgs {
        phase,
        repo,
        issue,
        delivery_pr,
        related_prs,
    };
    validate_gate_evidence_args(&parsed)?;
    Ok(parsed)
}

pub(super) fn validate_gate_evidence_args(args: &GateEvidenceArgs) -> Result<(), String> {
    let mut all_prs = args.related_prs.iter().copied().collect::<BTreeSet<_>>();
    if all_prs.len() != args.related_prs.len() {
        return Err("Related PR 不能重复".to_string());
    }
    if args
        .delivery_pr
        .is_some_and(|number| !all_prs.insert(number))
    {
        return Err("Delivery PR 与 Related PR 不能重复".to_string());
    }
    match (args.phase, args.delivery_pr, args.related_prs.as_slice()) {
        (GateEvidencePhase::G4, None, _) => {
            return Err("G4 必须指定 `--delivery-pr <number>`".to_string());
        }
        (GateEvidencePhase::G3, None, [_]) => {}
        (GateEvidencePhase::G3, None, []) => {
            return Err(
                "G3 必须指定 Delivery PR，或用一个 `--related-pr <number>` 独立校验 Related PR"
                    .to_string(),
            );
        }
        (GateEvidencePhase::G3, None, _) => {
            return Err("不含 Delivery PR 的 G3 独立模式只能指定一个 Related PR".to_string());
        }
        (_, Some(_), _) => {}
    }
    Ok(())
}

pub(super) fn selected_gate_evidence_pr(
    args: &GateEvidenceArgs,
) -> Result<(GateEvidencePrRole, u64), String> {
    validate_gate_evidence_args(args)?;
    match (args.delivery_pr, args.related_prs.as_slice()) {
        (Some(number), _) => Ok((GateEvidencePrRole::Delivery, number)),
        (None, [number]) => Ok((GateEvidencePrRole::Related, *number)),
        _ => Err("Gate evidence 命令缺少唯一可校验的 Delivery / Related PR".to_string()),
    }
}

pub(super) fn validate_related_pr_snapshot_count(
    args: &GateEvidenceArgs,
    related_prs: &[GitHubPullRequest],
) -> Result<(), String> {
    if args.related_prs.len() != related_prs.len() {
        return Err(format!(
            "Related PR 参数数量 ({}) 与已读取 snapshot 数量 ({}) 不一致",
            args.related_prs.len(),
            related_prs.len()
        ));
    }
    Ok(())
}

pub(super) fn validate_current_g3_target(
    args: &GateEvidenceArgs,
    delivery_pr: Option<&GitHubPullRequest>,
    related_prs: &[GitHubPullRequest],
) -> Result<(), String> {
    validate_related_pr_snapshot_count(args, related_prs)?;
    if args.phase != GateEvidencePhase::G3 {
        return Ok(());
    }
    let (role, _) = selected_gate_evidence_pr(args)?;
    let (label, target) = match role {
        GateEvidencePrRole::Delivery => (
            "Delivery PR",
            delivery_pr.ok_or("标准 G3 Delivery 模式缺少已读取的 Delivery PR snapshot")?,
        ),
        GateEvidencePrRole::Related => (
            "Related PR",
            related_prs
                .first()
                .ok_or("标准 G3 Related-only 模式缺少已读取的 Related PR snapshot")?,
        ),
    };
    if target.state != "OPEN" || target.is_draft || target.merged_at.is_some() {
        return Err(format!(
            "标准 G3 只能校验合并前仍为 OPEN 且非 Draft 的当前 {label}；历史合并证据只能由 G4 复核"
        ));
    }
    if delivery_pr.is_some() {
        for (number, related_pr) in args.related_prs.iter().zip(related_prs) {
            let is_open_candidate = related_pr.state == "OPEN"
                && !related_pr.is_draft
                && related_pr.merged_at.is_none();
            let is_merged_history = related_pr.state == "MERGED"
                && !related_pr.is_draft
                && related_pr.merged_at.is_some();
            if !is_open_candidate && !is_merged_history {
                return Err(format!(
                    "Delivery full-set G3 的 Related PR #{number} 必须是非 Draft OPEN current target，或带 mergedAt 的 MERGED 历史证据；CLOSED / 状态不一致的 Related PR 失败关闭"
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn validate_current_g3_issue(
    phase: GateEvidencePhase,
    issue: &GitHubIssue,
) -> Result<(), String> {
    if phase == GateEvidencePhase::G3 && issue.state != "OPEN" {
        return Err(
            "标准 G3 只能校验仍为 OPEN 的关联 Issue；Issue 关闭必须发生在 G4 完成后".to_string(),
        );
    }
    Ok(())
}

pub(super) fn parse_issue_number(flag: &str, value: &str) -> Result<u64, String> {
    value
        .strip_prefix('#')
        .unwrap_or(value)
        .parse::<u64>()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| format!("`{flag}` 必须是正整数 Issue / PR 编号：{value}"))
}

pub(super) fn valid_repository_name(repo: &str) -> bool {
    let Some((owner, name)) = repo.split_once('/') else {
        return false;
    };
    !owner.is_empty() && !name.is_empty() && !name.contains('/')
}
