//! Issue/PR Markdown、permalink、Gate assertion 与结构化例外解析。

use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{codeql, external_review, lockfile_policy};

use super::model::*;

pub(super) fn completed_gate_line<'a>(body: &'a str, gate: &str) -> Result<&'a str, String> {
    let prefix = gate_ledger_prefix(gate)?;
    body.lines()
        .find(|line| line.starts_with(prefix))
        .ok_or_else(|| format!("body 缺少已勾选的 `{gate}` Gate Ledger 项"))
}

pub(super) fn pending_gate_line<'a>(body: &'a str, gate: &str) -> Result<&'a str, String> {
    let completed_prefix = gate_ledger_prefix(gate)?;
    let pending_prefix = completed_prefix.replacen("- [x]", "- [ ]", 1);
    body.lines()
        .find(|line| line.starts_with(pending_prefix.as_str()))
        .ok_or_else(|| format!("body 缺少未勾选的 `{gate}` Gate Ledger 项"))
}

pub(super) fn gate_ledger_prefix(gate: &str) -> Result<&'static str, String> {
    match gate {
        "G0" => Ok("- [x] G0 立项已记录："),
        "G1" => Ok("- [x] G1 设计判断已记录："),
        "G2" => Ok("- [x] G2 开工判断已记录："),
        "G3" => Ok("- [x] G3 合并判断已记录："),
        "G4" => Ok("- [x] G4 完成判断已记录："),
        _ => Err(format!("未知 Gate：{gate}")),
    }
}

pub(super) fn metadata_line<'a>(body: &'a str, field: &str) -> Result<&'a str, String> {
    body.lines()
        .find(|line| line.starts_with(&format!("- {field}：")))
        .ok_or_else(|| format!("body 缺少 `{field}` 元数据字段"))
}

pub(super) fn unique_metadata_line<'a>(body: &'a str, field: &str) -> Result<&'a str, String> {
    let lines = body
        .lines()
        .filter(|line| line.starts_with(&format!("- {field}：")))
        .collect::<Vec<_>>();
    match lines.as_slice() {
        [line] => Ok(line),
        [] => Err(format!("body 缺少 `{field}` 元数据字段")),
        _ => Err(format!("body 只能包含一个 `{field}` 元数据字段")),
    }
}

pub(super) fn metadata_issue_numbers(line: &str) -> BTreeSet<u64> {
    metadata_issue_numbers_in_order(line).into_iter().collect()
}

pub(super) fn metadata_issue_numbers_in_order(line: &str) -> Vec<u64> {
    line.split('#')
        .skip(1)
        .filter_map(|tail| {
            let digits = tail
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .collect::<String>();
            digits.parse::<u64>().ok().filter(|number| *number > 0)
        })
        .collect()
}

pub(super) fn validate_recovery_related_pr_order(
    line: &str,
    expected: &[u64],
) -> Result<(), String> {
    let actual = metadata_issue_numbers_in_order(line);
    let unique = actual.iter().copied().collect::<BTreeSet<_>>();
    if unique.len() != actual.len() {
        return Err("Issue 的 `Related PRs` 字段不得包含重复 PR".to_string());
    }
    if actual != expected {
        return Err(format!(
            "Issue 的 `Related PRs` 顺序与 G4 recovery 命令不一致：Issue 记录 [{}]；命令传入 [{}]",
            format_issue_number_sequence(&actual),
            format_issue_number_sequence(expected)
        ));
    }
    Ok(())
}

pub(super) fn format_issue_number_sequence(numbers: &[u64]) -> String {
    if numbers.is_empty() {
        "N/A".to_string()
    } else {
        numbers
            .iter()
            .map(|number| format!("#{number}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub(super) fn format_issue_numbers(numbers: &BTreeSet<u64>) -> String {
    if numbers.is_empty() {
        "N/A".to_string()
    } else {
        numbers
            .iter()
            .map(|number| format!("#{number}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub(super) fn completed_gate_permalink(body: &str, gate: &str) -> Result<String, String> {
    let line = completed_gate_line(body, gate)?;
    resolve_comment_permalink(body, line).ok_or_else(|| {
        format!("已勾选的 `{gate}` Gate Ledger 项缺少 GitHub comment permalink（inline 或 reference-style）")
    })
}

pub(super) fn extract_comment_permalink(line: &str) -> Option<String> {
    let start = line.find("https://github.com/")?;
    let permalink = line[start..]
        .split(|character: char| character.is_whitespace() || character == ')' || character == '>')
        .next()?;
    permalink
        .contains("#issuecomment-")
        .then(|| permalink.to_string())
}

pub(super) fn resolve_comment_permalink(body: &str, line: &str) -> Option<String> {
    extract_comment_permalink(line).or_else(|| {
        markdown_reference_labels(line)
            .into_iter()
            .find_map(|label| reference_comment_permalink(body, label))
    })
}

pub(super) fn line_links_to_comment_permalink(body: &str, line: &str, permalink: &str) -> bool {
    line.contains(permalink)
        || markdown_reference_labels(line)
            .into_iter()
            .filter_map(|label| reference_comment_permalink(body, label))
            .any(|resolved| resolved == permalink)
}

pub(super) fn markdown_reference_labels(line: &str) -> Vec<&str> {
    let mut labels = Vec::new();
    let mut remainder = line;
    while let Some(start) = remainder.find("][") {
        let after_open = &remainder[start + 2..];
        let Some(end) = after_open.find(']') else {
            break;
        };
        let label = &after_open[..end];
        if !label.is_empty() {
            labels.push(label);
        }
        remainder = &after_open[end + 1..];
    }
    labels
}

pub(super) fn reference_comment_permalink(body: &str, label: &str) -> Option<String> {
    body.lines().find_map(|line| {
        let definition = line.trim().strip_prefix('[')?;
        let (candidate, value) = definition.split_once("]:")?;
        candidate
            .eq_ignore_ascii_case(label)
            .then(|| extract_comment_permalink(value.trim()))
            .flatten()
    })
}

pub(super) fn validate_comment(
    pr: &GitHubPullRequest,
    permalink: &str,
    required_fields: &[&str],
    label: &str,
    args: &GateEvidenceArgs,
) -> Result<(), String> {
    let comment = pr
        .comments
        .iter()
        .find(|comment| comment.url == permalink)
        .ok_or_else(|| format!("{label} permalink 未指向该 PR 的 comment"))?;
    let required_fields = if external_review_g3_active(&comment.created_at)? {
        if comment.includes_created_edit {
            return Err(format!(
                "{label} comment 在创建后被编辑；current G3 必须 append-only"
            ));
        }
        CURRENT_G3_COMMENT_FIELDS
    } else {
        required_fields
    };
    validate_comment_body(&comment.body, required_fields, label)?;
    if codeql_g3_active(&comment.created_at)? {
        validate_comment_body(&comment.body, &["- CodeQL："], label)?;
    }
    validate_gate_assertion(&comment.body, label, args, GateEvidencePhase::G3)
}

pub(super) fn comment_for_permalink<'a>(
    issue: &'a GitHubIssue,
    permalink: &str,
    label: &str,
) -> Result<&'a GitHubComment, String> {
    issue
        .comments
        .iter()
        .find(|comment| comment.url == permalink)
        .ok_or_else(|| format!("{label} permalink 未指向该 Issue 的 comment"))
}

pub(super) fn validate_comment_body(
    body: &str,
    required_fields: &[&str],
    label: &str,
) -> Result<(), String> {
    let missing_fields = required_fields
        .iter()
        .filter(|field| !body.contains(**field))
        .copied()
        .collect::<Vec<_>>();
    if missing_fields.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{label} comment 缺少必需字段：{}",
            missing_fields.join("、")
        ))
    }
}

pub(super) fn validate_gate_assertion(
    body: &str,
    label: &str,
    args: &GateEvidenceArgs,
    phase: GateEvidencePhase,
) -> Result<(), String> {
    let actual_commands = gate_assertion_commands(body, label, phase)?;
    let expected_command = expected_gate_command(args, phase);
    if !actual_commands.contains(&expected_command) {
        return Err(format!(
            "{label} comment 的 `Gate 断言` 命令与当前参数不一致：期望包含 `{expected_command}`；实际 [{}]",
            actual_commands.into_iter().collect::<Vec<_>>().join("；")
        ));
    }
    Ok(())
}

pub(super) fn validate_gate_assertion_set(
    body: &str,
    label: &str,
    args: &[GateEvidenceArgs],
    phase: GateEvidencePhase,
) -> Result<(), String> {
    let actual_commands = gate_assertion_commands(body, label, phase)?;
    let expected_commands = args
        .iter()
        .map(|args| expected_gate_command(args, phase))
        .collect::<BTreeSet<_>>();
    if expected_commands.len() != args.len() {
        return Err(format!("{label} target 解析出了重复的 `Gate 断言` 命令"));
    }
    if actual_commands != expected_commands {
        return Err(format!(
            "{label} comment 的完整 `Gate 断言` 命令集合与全部声明 Issue target 不一致：期望 [{}]；实际 [{}]",
            expected_commands.into_iter().collect::<Vec<_>>().join("；"),
            actual_commands.into_iter().collect::<Vec<_>>().join("；")
        ));
    }
    Ok(())
}

pub(super) fn gate_assertion_commands(
    body: &str,
    label: &str,
    phase: GateEvidencePhase,
) -> Result<BTreeSet<String>, String> {
    let assertion_lines = body
        .lines()
        .filter(|line| line.starts_with(GATE_ASSERTION_PREFIX))
        .collect::<Vec<_>>();
    if assertion_lines.is_empty() {
        return Err(format!("{label} comment 缺少独立的 `Gate 断言` 行"));
    }
    if phase == GateEvidencePhase::G4 && assertion_lines.len() != 1 {
        return Err(format!("{label} comment 的 G4 只能包含一条 `Gate 断言` 行"));
    }
    let mut unique_commands = BTreeSet::new();
    for assertion_line in assertion_lines {
        let value = assertion_line
            .strip_prefix(GATE_ASSERTION_PREFIX)
            .expect("filtered assertion line must have the prefix")
            .trim();
        let Some(command_and_result) = value.strip_prefix('`') else {
            return Err(format!(
                "{label} comment 的 `Gate 断言` 必须先用反引号记录规范命令"
            ));
        };
        let Some((actual_command, result)) = command_and_result.split_once('`') else {
            return Err(format!("{label} comment 的 `Gate 断言` 命令缺少闭合反引号"));
        };
        if !unique_commands.insert(actual_command.to_string()) {
            return Err(format!(
                "{label} comment 的 `Gate 断言` 不得重复同一规范命令：`{actual_command}`"
            ));
        }
        if !matches!(result.trim(), "已通过" | "已通过。") {
            return Err(format!(
                "{label} comment 的 `Gate 断言` 必须在规范命令后明确记录 `已通过`"
            ));
        }
    }
    Ok(unique_commands)
}

pub(super) fn expected_gate_command(args: &GateEvidenceArgs, phase: GateEvidencePhase) -> String {
    let phase = match phase {
        GateEvidencePhase::G3 => "g3",
        GateEvidencePhase::G4 => "g4",
    };
    let mut command = format!(
        "cargo +1.96.0 run --locked -p xtask -- check-gate-evidence {phase} --repo {} --issue {}",
        args.repo, args.issue
    );
    if let Some(delivery_pr) = args.delivery_pr {
        command.push_str(&format!(" --delivery-pr {delivery_pr}"));
    }
    for related_pr in &args.related_prs {
        command.push_str(&format!(" --related-pr {related_pr}"));
    }
    command
}

pub(super) fn validate_g3_timing(
    pr: &GitHubPullRequest,
    permalink: &str,
    label: &str,
) -> Result<(), String> {
    let Some(merged_at) = pr.merged_at.as_deref() else {
        return Ok(());
    };
    let comment = pr
        .comments
        .iter()
        .find(|comment| comment.url == permalink)
        .ok_or_else(|| format!("{label} permalink 未指向该 PR 的 comment"))?;
    let comment_time = lockfile_policy::parse_utc_rfc3339(&comment.created_at)
        .ok_or_else(|| format!("{label} comment createdAt 不是有效 UTC RFC3339 时间"))?;
    let merge_time = lockfile_policy::parse_utc_rfc3339(merged_at)
        .ok_or_else(|| format!("{label} PR mergedAt 不是有效 UTC RFC3339 时间"))?;
    if comment_time >= merge_time {
        return Err(format!("{label} comment 必须严格早于 PR 合并时间"));
    }
    Ok(())
}

pub(super) fn g3_current_head(body: &str) -> Result<&str, String> {
    let line = unique_metadata_line(body, "Current head")?;
    let value = line
        .strip_prefix("- Current head：")
        .expect("unique_metadata_line returned an unexpected prefix")
        .trim();
    if value.is_empty() {
        return Err("G3 comment 的 `Current head` 字段不能为空".to_string());
    }
    if let Some(value) = value.strip_prefix('`') {
        return value
            .strip_suffix('`')
            .filter(|value| !value.is_empty() && !value.contains('`'))
            .ok_or_else(|| "G3 comment 的 `Current head` 字段 backtick 格式无效".to_string());
    }
    if value.contains('`') {
        return Err("G3 comment 的 `Current head` 字段 backtick 格式无效".to_string());
    }
    Ok(value)
}

pub(super) fn validate_external_review_g3(
    repo: &str,
    issue_number: u64,
    number: u64,
    pr: &GitHubPullRequest,
    label: &str,
) -> Result<(), String> {
    let permalink = completed_gate_permalink(&pr.body, "G3")?;
    let comment = pr
        .comments
        .iter()
        .find(|comment| comment.url == permalink)
        .ok_or_else(|| format!("{label} G3 permalink 未指向该 PR 的 comment"))?;
    let gate_result = parse_g3_result(&comment.body)?;
    let result = match gate_result {
        G3Result::Waived => {
            let current_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| format!("系统时间早于 Unix epoch：{error}"))?
                .as_secs();
            let reference_time = gate_waiver_reference_time(pr, current_time)?;
            let waiver = parse_gate_waiver(comment, issue_number, reference_time)?;
            external_review::evaluate_live_with_waiver(repo, number, waiver)?
        }
        G3Result::Pass | G3Result::Bootstrap => external_review::evaluate_live(repo, number)?,
    };
    let expected_state = match gate_result {
        G3Result::Waived => external_review::ExternalReviewState::Waived,
        G3Result::Pass | G3Result::Bootstrap => external_review::ExternalReviewState::Pass,
    };
    if result.state != expected_state {
        return Err(format!(
            "{label} 的 G3 结果与 External Review Gate 不一致：G3={gate_result:?}，期望 {expected_state:?}，实际 {:?}",
            result.state
        ));
    }
    if g3_current_head(&comment.body)? != result.current_head_oid() {
        return Err(format!(
            "{label} G3 comment 未记录 External Review Gate 对应的完整 current head `{}`",
            result.current_head_oid()
        ));
    }
    if gate_result != G3Result::Waived {
        let completion_time = result
            .completion_time()
            .ok_or_else(|| format!("{label} pass 结果缺少 completion time"))?;
        validate_external_review_completion_order(label, &comment.created_at, completion_time)?;
    }
    Ok(())
}

pub(super) fn gate_waiver_reference_time(
    pr: &GitHubPullRequest,
    current_time: u64,
) -> Result<u64, String> {
    let Some(merged_at) = pr.merged_at.as_deref() else {
        return Ok(current_time);
    };
    lockfile_policy::parse_utc_rfc3339(merged_at)
        .map(lockfile_policy::UtcTimestamp::seconds)
        .ok_or_else(|| "已合并 PR 的 mergedAt 不是有效 UTC RFC3339 时间".to_string())
}

pub(super) fn validate_codeql_g3(
    repo: &str,
    number: u64,
    pr: &GitHubPullRequest,
    label: &str,
) -> Result<(), String> {
    let permalink = completed_gate_permalink(&pr.body, "G3")?;
    let comment = pr
        .comments
        .iter()
        .find(|comment| comment.url == permalink)
        .ok_or_else(|| format!("{label} G3 permalink 未指向该 PR 的 comment"))?;
    if !codeql_g3_active(&comment.created_at)? {
        return Ok(());
    }
    let codeql_lines = comment
        .body
        .lines()
        .filter(|line| line.trim_start().starts_with("- CodeQL："))
        .collect::<Vec<_>>();
    if codeql_lines.len() != 1 {
        return Err(format!("{label} G3 comment 必须恰好包含一条 `- CodeQL：`"));
    }
    let line = codeql_lines[0];
    let recorded_state = codeql_state(line)?;
    let evidence_url = codeql_evidence_url(line)?;
    let result = if pr.merged_at.is_some() {
        codeql::evaluate_live_recorded(repo, number, evidence_url)?
    } else {
        codeql::evaluate_live(repo, number)?
    };
    if !result.state.satisfies_g3() {
        return Err(format!(
            "{label} 的 CodeQL 未满足 G3：{}",
            result.state.as_str()
        ));
    }
    validate_codeql_completion_order(label, &comment.created_at, result.completion_time())?;
    if recorded_state != result.state {
        return Err(format!(
            "{label} G3 comment 的 CodeQL 状态与机器结果不一致：{}",
            result.state.as_str()
        ));
    }
    if let Some(evidence_url) = result.evidence_url() {
        if !codeql_evidence_matches(line, evidence_url)? {
            return Err(format!(
                "{label} G3 comment 的 CodeQL 行未回链机器结果 evidence URL"
            ));
        }
    }
    if result.state == codeql::CodeQlState::NotApplicable
        && (codeql_policy(line)? != "dependabot-cargo-lock-only-v1"
            || result.policy() != Some("dependabot-cargo-lock-only-v1"))
    {
        return Err(format!(
            "{label} CodeQL not_applicable 必须记录精确 `dependabot-cargo-lock-only-v1` policy"
        ));
    }
    Ok(())
}

pub(super) fn codeql_evidence_matches(line: &str, expected: &str) -> Result<bool, String> {
    Ok(codeql_evidence_url(line)? == expected)
}

pub(super) fn codeql_state(line: &str) -> Result<codeql::CodeQlState, String> {
    let tail = line
        .trim_start()
        .strip_prefix("- CodeQL：")
        .ok_or_else(|| "G3 comment 的 CodeQL 行缺少精确字段前缀".to_string())?
        .trim_start();
    let value_tail = tail
        .strip_prefix('`')
        .ok_or_else(|| "G3 comment 的 CodeQL 状态必须是字段后的首个 backtick 值".to_string())?;
    let end = value_tail
        .find('`')
        .ok_or_else(|| "G3 comment 的 CodeQL 状态缺少结束 backtick".to_string())?;
    let state = codeql::CodeQlState::parse(&value_tail[..end])?;
    let state_tokens = [
        "pass",
        "not_applicable",
        "pending",
        "failed",
        "missing",
        "provider_error",
    ]
    .iter()
    .map(|candidate| line.matches(&format!("`{candidate}`")).count())
    .sum::<usize>();
    if state_tokens != 1 {
        return Err("G3 comment 的 CodeQL 行必须恰好记录一个状态值".to_string());
    }
    Ok(state)
}

pub(super) fn codeql_policy(line: &str) -> Result<&str, String> {
    let marker = "policy `";
    let positions = line.match_indices(marker).collect::<Vec<_>>();
    let [position] = positions.as_slice() else {
        return Err("G3 comment 的 CodeQL 行必须恰好记录一个 `policy` 值".to_string());
    };
    let value_tail = &line[position.0 + marker.len()..];
    let end = value_tail
        .find('`')
        .ok_or_else(|| "G3 comment 的 CodeQL policy 缺少结束 backtick".to_string())?;
    let value = &value_tail[..end];
    if value.is_empty() {
        return Err("G3 comment 的 CodeQL policy 不能为空".to_string());
    }
    Ok(value)
}

pub(super) fn codeql_evidence_url(line: &str) -> Result<&str, String> {
    let marker = "https://github.com/";
    let positions = line.match_indices(marker).collect::<Vec<_>>();
    if positions.len() != 1 {
        return Err("G3 comment 的 CodeQL 行必须恰好包含一个 GitHub evidence URL".to_string());
    }
    let suffix = &line[positions[0].0..];
    let end = suffix
        .find(|character: char| {
            character.is_whitespace() || matches!(character, ',' | '，' | '。' | ')' | ']')
        })
        .unwrap_or(suffix.len());
    Ok(&suffix[..end])
}

pub(super) fn validate_codeql_completion_order(
    label: &str,
    comment_created_at: &str,
    completion_time: Option<&str>,
) -> Result<(), String> {
    let Some(completion_time_text) = completion_time else {
        return Ok(());
    };
    let comment_time = lockfile_policy::parse_utc_rfc3339(comment_created_at)
        .ok_or_else(|| format!("{label} G3 comment createdAt 不是有效 UTC RFC3339 时间"))?;
    let completion_time = lockfile_policy::parse_utc_rfc3339(completion_time_text)
        .ok_or_else(|| format!("{label} CodeQL completedAt 不是有效 UTC RFC3339 时间"))?;
    if comment_time <= completion_time {
        return Err(format!(
            "{label} G3 comment 未严格晚于 CodeQL 完成时间：comment={comment_created_at}，completion={}",
            completion_time_text
        ));
    }
    Ok(())
}

pub(super) fn validate_external_review_completion_order(
    label: &str,
    comment_created_at: &str,
    completion_time_text: &str,
) -> Result<(), String> {
    let comment_time = lockfile_policy::parse_utc_rfc3339(comment_created_at)
        .ok_or_else(|| format!("{label} G3 comment createdAt 不是有效 UTC RFC3339 时间"))?;
    let completion_time =
        lockfile_policy::parse_utc_rfc3339(completion_time_text).ok_or_else(|| {
            format!("{label} external review completion time 不是有效 UTC RFC3339 时间")
        })?;
    if comment_time <= completion_time {
        return Err(format!(
            "{label} G3 comment 未严格晚于最终 external review completion：comment={comment_created_at}，completion={completion_time_text}"
        ));
    }
    Ok(())
}

pub(super) fn external_review_g3_active(comment_created_at: &str) -> Result<bool, String> {
    let comment_time = lockfile_policy::parse_utc_rfc3339(comment_created_at)
        .ok_or_else(|| "G3 comment createdAt 不是有效 UTC RFC3339 时间".to_string())?;
    let activation_time = lockfile_policy::parse_utc_rfc3339(EXTERNAL_REVIEW_G3_ACTIVATION)
        .expect("external review G3 activation must be valid UTC RFC3339");
    Ok(comment_time >= activation_time)
}

pub(super) fn codeql_g3_active(comment_created_at: &str) -> Result<bool, String> {
    let comment_time = lockfile_policy::parse_utc_rfc3339(comment_created_at)
        .ok_or_else(|| "G3 comment createdAt 不是有效 UTC RFC3339 时间".to_string())?;
    let activation_time = lockfile_policy::parse_utc_rfc3339(CODEQL_G3_ACTIVATION)
        .expect("CodeQL G3 activation must be valid UTC RFC3339");
    Ok(comment_time >= activation_time)
}

pub(super) fn parse_g3_result(body: &str) -> Result<G3Result, String> {
    let prefix = "- Gate 结果：";
    let mut lines = body
        .lines()
        .filter(|line| line.trim_start().starts_with(prefix));
    let line = lines
        .next()
        .ok_or_else(|| "current G3 comment 缺少 `- Gate 结果：`".to_string())?;
    if lines.next().is_some() {
        return Err("current G3 comment 的 `- Gate 结果：` 不能重复".to_string());
    }
    let value = line
        .trim_start()
        .strip_prefix(prefix)
        .unwrap_or_default()
        .trim()
        .trim_end_matches('。')
        .trim();
    let value = value
        .strip_prefix('`')
        .and_then(|value| value.strip_suffix('`'))
        .unwrap_or(value);
    match value {
        "G3 Pass" => Ok(G3Result::Pass),
        "G3 Waived" => Ok(G3Result::Waived),
        "R0-R1 bootstrap" => Ok(G3Result::Bootstrap),
        _ => Err(format!(
            "current G3 comment 的 Gate 结果无效：`{value}`；应为 `G3 Pass`、`G3 Waived` 或 `R0-R1 bootstrap`"
        )),
    }
}

pub(super) fn parse_g3_full_set_recovery(
    comment: &GitHubComment,
    args: &GateEvidenceArgs,
    delivery_merged_at: &str,
) -> Result<(G3FullSetRecoveryRecord, BTreeSet<String>), String> {
    if comment.includes_created_edit {
        return Err("G3 full-set recovery 所在 G4 comment 在创建后被编辑".to_string());
    }
    let marker_count = comment.body.matches(G3_FULL_SET_RECOVERY_START).count();
    if marker_count != 1 {
        return Err(format!(
            "G3 full-set recovery 必须包含且只包含一个 `{G3_FULL_SET_RECOVERY_START}` 结构化记录"
        ));
    }
    let (_, after_start) = comment
        .body
        .split_once(G3_FULL_SET_RECOVERY_START)
        .ok_or_else(|| "G3 full-set recovery 缺少结构化记录起始标记".to_string())?;
    let (json, _) = after_start
        .split_once(G3_FULL_SET_RECOVERY_END)
        .ok_or_else(|| "G3 full-set recovery 缺少结构化记录结束标记".to_string())?;
    let record = serde_json::from_str::<G3FullSetRecoveryRecord>(json.trim())
        .map_err(|error| format!("G3 full-set recovery 不是 schema v1 JSON：{error}"))?;
    if record.schema_version != 1 {
        return Err(format!(
            "G3 full-set recovery schemaVersion 必须为 1，实际为 {}",
            record.schema_version
        ));
    }
    if record.exception_type != "late_related_after_delivery_merge" {
        return Err(
            "G3 full-set recovery exceptionType 必须为 `late_related_after_delivery_merge`"
                .to_string(),
        );
    }
    if record.issue != args.issue {
        return Err(format!(
            "G3 full-set recovery issue 必须为当前 Issue {}",
            args.issue
        ));
    }
    let delivery_number = args
        .delivery_pr
        .ok_or("G3 full-set recovery 缺少 Delivery PR 参数")?;
    if record.delivery_pr != delivery_number {
        return Err(format!(
            "G3 full-set recovery deliveryPr 必须为 {delivery_number}"
        ));
    }
    if record.delivery_merged_at != delivery_merged_at {
        return Err(format!(
            "G3 full-set recovery deliveryMergedAt 与 GitHub 当前值不一致：记录 `{}`；实际 `{delivery_merged_at}`",
            record.delivery_merged_at
        ));
    }
    lockfile_policy::parse_utc_rfc3339(&record.delivery_merged_at).ok_or_else(|| {
        "G3 full-set recovery deliveryMergedAt 不是有效 UTC RFC3339 时间".to_string()
    })?;
    for (field, value) in [
        ("reason", record.reason.as_str()),
        ("risk", record.risk.as_str()),
        ("acceptanceBoundary", record.acceptance_boundary.as_str()),
        ("followUpIssue", record.follow_up_issue.as_str()),
        ("cleanupOwner", record.cleanup_owner.as_str()),
        ("authorizedBy", record.authorized_by.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(format!("G3 full-set recovery `{field}` 不能为空"));
        }
    }
    let follow_up_number = record
        .follow_up_issue
        .strip_prefix('#')
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|number| *number > 0)
        .ok_or_else(|| {
            "G3 full-set recovery followUpIssue 必须是 `#<positive number>`".to_string()
        })?;
    if follow_up_number == args.issue {
        return Err("G3 full-set recovery followUpIssue 必须独立于当前交付 Issue".to_string());
    }

    let author = comment
        .author
        .as_ref()
        .map(|actor| actor.login.as_str())
        .ok_or_else(|| "G3 full-set recovery G4 comment 缺少 author".to_string())?;
    if !author.eq_ignore_ascii_case(&record.authorized_by) {
        return Err(format!(
            "G3 full-set recovery authorizedBy `{}` 与 comment author `{author}` 不一致",
            record.authorized_by
        ));
    }
    if !G3_OWNER_ACTORS
        .iter()
        .any(|actor| actor.eq_ignore_ascii_case(author))
    {
        return Err(format!(
            "G3 full-set recovery comment author `{author}` 不在 trusted G3 Owner allowlist"
        ));
    }

    let relation_line = comment
        .body
        .lines()
        .find(|line| line.trim_start().starts_with("- 关系："))
        .ok_or_else(|| "G3 full-set recovery G4 comment 缺少 `- 关系：`".to_string())?;
    let visible_refs = markdown_reference_labels(relation_line)
        .into_iter()
        .map(str::to_ascii_lowercase)
        .collect::<BTreeSet<_>>();
    if record.evidence_refs.is_empty() {
        return Err("G3 full-set recovery evidenceRefs 不能为空".to_string());
    }
    let mut seen_refs = BTreeSet::new();
    let mut evidence_urls = BTreeSet::new();
    for evidence_ref in &record.evidence_refs {
        let normalized = evidence_ref.to_ascii_lowercase();
        if !seen_refs.insert(normalized.clone()) {
            return Err(format!(
                "G3 full-set recovery evidenceRefs 重复：{evidence_ref}"
            ));
        }
        if !visible_refs.contains(&normalized) {
            return Err(format!(
                "G3 full-set recovery evidence ref `{evidence_ref}` 未由 `- 关系：` 行可见引用"
            ));
        }
        let evidence_url = reference_github_url(&comment.body, evidence_ref).ok_or_else(|| {
            format!(
                "G3 full-set recovery evidence ref `{evidence_ref}` 缺少 GitHub HTTPS 文末引用定义"
            )
        })?;
        evidence_urls.insert(evidence_url);
    }

    Ok((record, evidence_urls))
}

pub(super) fn parse_gate_waiver_records(
    comment: &GitHubComment,
) -> Result<Vec<GateWaiverRecord>, String> {
    let marker_count = comment.body.matches(EXTERNAL_REVIEW_WAIVER_START).count();
    if marker_count == 0 {
        return Err(format!(
            "G3 Waived comment 必须至少包含一个 `{EXTERNAL_REVIEW_WAIVER_START}` 结构化记录"
        ));
    }
    let mut records = Vec::with_capacity(marker_count);
    let mut seen_ids = BTreeSet::new();
    let mut seen_follow_up_issues = BTreeSet::new();
    for (index, after_start) in comment
        .body
        .split(EXTERNAL_REVIEW_WAIVER_START)
        .skip(1)
        .enumerate()
    {
        let (json, _) = after_start
            .split_once(EXTERNAL_REVIEW_WAIVER_END)
            .ok_or_else(|| format!("G3 Waived 第 {} 个结构化记录缺少结束标记", index + 1))?;
        let record = serde_json::from_str::<GateWaiverRecord>(json.trim()).map_err(|error| {
            format!(
                "G3 Waived 第 {} 个结构化记录不是 schema v1 JSON：{error}",
                index + 1
            )
        })?;
        if record.schema_version != 1 {
            return Err(format!(
                "G3 Waived schemaVersion 必须为 1，实际为 {}",
                record.schema_version
            ));
        }
        let follow_up_issue_number = gate_waiver_follow_up_issue_number(&record)?;
        if !seen_ids.insert(record.id.clone()) {
            return Err(format!("G3 Waived id 不能重复：{}", record.id));
        }
        if !seen_follow_up_issues.insert(follow_up_issue_number) {
            return Err(format!(
                "G3 Waived 每个 Issue 只能包含一个结构化记录：{}",
                record.follow_up_issue
            ));
        }
        records.push(record);
    }
    Ok(records)
}

pub(super) fn gate_waiver_follow_up_issue_number(record: &GateWaiverRecord) -> Result<u64, String> {
    let number = record
        .follow_up_issue
        .strip_prefix('#')
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|number| *number > 0)
        .ok_or_else(|| {
            format!(
                "G3 Waived followUpIssue 必须是明确的正整数 Issue 编号：{}",
                record.follow_up_issue
            )
        })?;
    if record.follow_up_issue != format!("#{number}") {
        return Err(format!(
            "G3 Waived followUpIssue 必须使用无前导零的规范 `#<positive number>`：{}",
            record.follow_up_issue
        ));
    }
    Ok(number)
}

pub(super) fn validate_gate_waiver_record_set(
    comment: &GitHubComment,
    declared_issues: &BTreeSet<u64>,
) -> Result<(), String> {
    if !external_review_g3_active(&comment.created_at)? {
        return Ok(());
    }
    match parse_g3_result(&comment.body)? {
        G3Result::Waived => {
            let records = parse_gate_waiver_records(comment)?;
            let recorded_issues = records
                .iter()
                .map(gate_waiver_follow_up_issue_number)
                .collect::<Result<BTreeSet<_>, _>>()?;
            if &recorded_issues != declared_issues {
                return Err(format!(
                    "G3 Waived 结构化记录的 followUpIssue 集合必须与 `关联 Issue` 精确一致：声明 [{}]；waiver [{}]",
                    format_issue_numbers(declared_issues),
                    format_issue_numbers(&recorded_issues)
                ));
            }
        }
        G3Result::Pass | G3Result::Bootstrap => {
            if comment.body.contains(EXTERNAL_REVIEW_WAIVER_START) {
                return Err(
                    "非 G3 Waived comment 不得夹带 `external-review-waiver:v1` 结构化记录"
                        .to_string(),
                );
            }
        }
    }
    Ok(())
}

pub(super) fn parse_gate_waiver(
    comment: &GitHubComment,
    issue_number: u64,
    now: u64,
) -> Result<external_review::WaiverInput, String> {
    let records = parse_gate_waiver_records(comment)?;
    let expected_follow_up_issue = format!("#{issue_number}");
    let record = records
        .into_iter()
        .find(|record| record.follow_up_issue == expected_follow_up_issue)
        .ok_or_else(|| {
            format!(
                "G3 Waived multi-Issue comment 缺少当前 Issue `{expected_follow_up_issue}` 的唯一结构化记录"
            )
        })?;
    let author = comment
        .author
        .as_ref()
        .map(|actor| actor.login.as_str())
        .ok_or_else(|| "G3 Waived comment 缺少 author，无法验证授权人".to_string())?;
    if !author.eq_ignore_ascii_case(&record.authorized_by) {
        return Err(format!(
            "G3 Waived authorizedBy `{}` 与 comment author `{author}` 不一致",
            record.authorized_by
        ));
    }
    if !G3_OWNER_ACTORS
        .iter()
        .any(|actor| actor.eq_ignore_ascii_case(author))
    {
        return Err(format!(
            "G3 Waived comment author `{author}` 不在 trusted G3 Owner allowlist"
        ));
    }
    let created_at = parse_utc_timestamp_seconds(&comment.created_at)
        .ok_or_else(|| "G3 Waived comment createdAt 不是 UTC RFC3339 秒级时间".to_string())?;
    let expires_at = parse_utc_timestamp_seconds(&record.expires_at)
        .ok_or_else(|| "G3 Waived expiresAt 必须是 UTC RFC3339 秒级时间".to_string())?;
    if expires_at <= created_at {
        return Err("G3 Waived expiresAt 必须晚于 comment createdAt".to_string());
    }
    if expires_at - created_at > EXTERNAL_REVIEW_WAIVER_MAX_SECONDS {
        return Err("G3 Waived 有效期不得超过 24 小时".to_string());
    }
    if expires_at <= now {
        return Err("G3 Waived 已过期，不能满足当前 Gate".to_string());
    }

    let exception_line = comment
        .body
        .lines()
        .find(|line| line.trim_start().starts_with("- 例外："))
        .ok_or_else(|| "G3 Waived comment 缺少 `- 例外：`".to_string())?;
    let visible_refs = markdown_reference_labels(exception_line)
        .into_iter()
        .map(str::to_ascii_lowercase)
        .collect::<BTreeSet<_>>();
    if record.evidence_refs.is_empty() {
        return Err("G3 Waived evidenceRefs 不能为空".to_string());
    }
    let mut seen_refs = BTreeSet::new();
    let mut evidence_urls = Vec::with_capacity(record.evidence_refs.len());
    for evidence_ref in &record.evidence_refs {
        let normalized = evidence_ref.to_ascii_lowercase();
        if !seen_refs.insert(normalized.clone()) {
            return Err(format!("G3 Waived evidenceRefs 重复：{evidence_ref}"));
        }
        if !visible_refs.contains(&normalized) {
            return Err(format!(
                "G3 Waived evidence ref `{evidence_ref}` 未由 `- 例外：` 行可见引用"
            ));
        }
        evidence_urls.push(
            reference_github_url(&comment.body, evidence_ref).ok_or_else(|| {
                format!("G3 Waived evidence ref `{evidence_ref}` 缺少 GitHub HTTPS 文末引用定义")
            })?,
        );
    }

    Ok(external_review::WaiverInput {
        id: record.id,
        exception_type: record.exception_type,
        current_head_oid: record.current_head_oid,
        current_base_oid: record.current_base_oid,
        reason: record.reason,
        evidence_urls,
        risk: record.risk,
        acceptance_boundary: record.acceptance_boundary,
        expires_at: record.expires_at,
        follow_up_issue: record.follow_up_issue,
        cleanup_owner: record.cleanup_owner,
        authorized_by: record.authorized_by,
    })
}

pub(super) fn reference_github_url(body: &str, label: &str) -> Option<String> {
    body.lines().find_map(|line| {
        let definition = line.trim().strip_prefix('[')?;
        let (candidate, value) = definition.split_once("]:")?;
        if !candidate.eq_ignore_ascii_case(label) {
            return None;
        }
        let value = value.trim();
        let value = value
            .strip_prefix('<')
            .and_then(|value| value.strip_suffix('>'))
            .unwrap_or(value);
        (value.starts_with("https://github.com/") && !value.chars().any(char::is_whitespace))
            .then(|| value.to_string())
    })
}

pub(super) fn parse_utc_timestamp_seconds(value: &str) -> Option<u64> {
    lockfile_policy::parse_utc_rfc3339(value).map(lockfile_policy::UtcTimestamp::seconds)
}

pub(super) fn g3_requires_external_review(pr: &GitHubPullRequest) -> Result<bool, String> {
    let permalink = completed_gate_permalink(&pr.body, "G3")?;
    let comment = pr
        .comments
        .iter()
        .find(|comment| comment.url == permalink)
        .ok_or_else(|| "G3 permalink 未指向该 PR 的 comment".to_string())?;
    external_review_g3_active(&comment.created_at)
}
