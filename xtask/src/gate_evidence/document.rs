//! Issue/PR Markdown、permalink、Gate assertion 与结构化例外解析。

use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::external_review;
use sha2::{Digest, Sha256};

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

pub(super) fn g3_comment_effective_at<'a>(
    comment: &'a GitHubComment,
    label: &str,
) -> Result<&'a str, String> {
    let effective_at = if comment.includes_created_edit {
        comment
            .updated_at
            .as_deref()
            .ok_or_else(|| format!("{label} 已编辑但缺少 hydrated updatedAt"))?
    } else {
        &comment.created_at
    };
    let created_at = parse_utc_timestamp_seconds(&comment.created_at)
        .ok_or_else(|| format!("{label} createdAt 不是 UTC RFC3339 时间"))?;
    let effective_seconds = parse_utc_timestamp_seconds(effective_at)
        .ok_or_else(|| format!("{label} effectiveAt 不是 UTC RFC3339 时间"))?;
    if effective_seconds < created_at {
        return Err(format!("{label} effectiveAt 早于 createdAt"));
    }
    Ok(effective_at)
}

pub(super) fn validate_comment(
    pr: &GitHubPullRequest,
    permalink: &str,
    required_fields: &[&str],
    label: &str,
    args: &GateEvidenceArgs,
    allow_legacy_exception: bool,
) -> Result<(), String> {
    let comment = pr
        .comments
        .iter()
        .find(|comment| comment.url == permalink)
        .ok_or_else(|| format!("{label} permalink 未指向该 PR 的 comment"))?;
    let effective_at = g3_comment_effective_at(comment, label)?;
    let required_fields = if effective_at >= EXTERNAL_REVIEW_G3_ACTIVATION {
        CURRENT_G3_COMMENT_FIELDS
    } else {
        required_fields
    };
    validate_comment_body(&comment.body, required_fields, label)?;
    validate_gate_assertion_with_legacy_exception(
        &comment.body,
        label,
        args,
        GateEvidencePhase::G3,
        allow_legacy_exception,
    )
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

#[cfg(test)]
pub(super) fn validate_gate_assertion(
    body: &str,
    label: &str,
    args: &GateEvidenceArgs,
    phase: GateEvidencePhase,
) -> Result<(), String> {
    validate_gate_assertion_with_legacy_exception(body, label, args, phase, false)
}

pub(super) fn validate_gate_assertion_with_legacy_exception(
    body: &str,
    label: &str,
    args: &GateEvidenceArgs,
    phase: GateEvidencePhase,
    allow_legacy_exception: bool,
) -> Result<(), String> {
    let actual_commands = match semantic_gate_assertion_commands_with_legacy_exception(
        body,
        label,
        phase,
        Some((args.issue, allow_legacy_exception)),
    ) {
        Ok(commands) => commands,
        Err(_)
            if allow_legacy_exception
                && legacy_failed_gate_assertion_without_command(body, phase) =>
        {
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let expected_command = expected_gate_command(args, phase);
    if !actual_commands.contains(&expected_command) {
        return Err(format!(
            "{label} comment 的 `Gate 断言` 命令与当前参数不一致：期望包含 `{expected_command}`；实际 [{}]",
            actual_commands.into_iter().collect::<Vec<_>>().join("；")
        ));
    }
    Ok(())
}

fn legacy_failed_gate_assertion_without_command(body: &str, phase: GateEvidencePhase) -> bool {
    if phase != GateEvidencePhase::G3 {
        return false;
    }
    let lines = body
        .lines()
        .filter(|line| line.starts_with(GATE_ASSERTION_PREFIX))
        .collect::<Vec<_>>();
    lines.len() == 1
        && !lines[0].contains('`')
        && lines[0].contains("未通过")
        && !lines[0].contains("已通过")
}

#[cfg(test)]
pub(super) fn validate_gate_assertion_set(
    body: &str,
    label: &str,
    args: &[GateEvidenceArgs],
    phase: GateEvidencePhase,
) -> Result<(), String> {
    validate_gate_assertion_set_with_legacy_exception(body, label, args, phase, None)
}

pub(super) fn validate_gate_assertion_set_with_legacy_exception(
    body: &str,
    label: &str,
    args: &[GateEvidenceArgs],
    phase: GateEvidencePhase,
    result_scope: Option<(u64, bool)>,
) -> Result<(), String> {
    let expected_commands = args
        .iter()
        .map(|args| expected_gate_command(args, phase))
        .collect::<BTreeSet<_>>();
    if expected_commands.len() != args.len() {
        return Err(format!("{label} target 解析出了重复的 `Gate 断言` 命令"));
    }
    let actual_commands = match semantic_gate_assertion_commands_with_legacy_exception(
        body,
        label,
        phase,
        result_scope,
    ) {
        Ok(commands) => commands,
        Err(_)
            if args.len() == 1
                && result_scope == Some((args[0].issue, true))
                && legacy_failed_gate_assertion_without_command(body, phase) =>
        {
            return Ok(());
        }
        Err(error) => return Err(error),
    };
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
    gate_assertion_commands_with_legacy_exception(body, label, phase, None)
}

pub(super) fn gate_assertion_commands_with_legacy_exception(
    body: &str,
    label: &str,
    phase: GateEvidencePhase,
    result_scope: Option<(u64, bool)>,
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
        let scoped_legacy_exception =
            if let Some((scope_issue, allow_legacy_exception)) = result_scope {
                let command_args = parse_gate_assertion_command(actual_command, phase)
                    .map_err(|error| format!("{label} comment 的 `Gate 断言` 命令无效：{error}"))?;
                (command_args.issue == scope_issue).then_some(allow_legacy_exception)
            } else {
                None
            };
        let allow_legacy_exception = scoped_legacy_exception == Some(true);
        let expects_failed_assertion = allow_legacy_exception
            || match phase {
                GateEvidencePhase::G3 => {
                    body.lines()
                        .any(|line| line.trim_start().starts_with("- Gate 结果："))
                        && matches!(
                            parse_g3_result(body)?,
                            G3Result::Exception | G3Result::LegacyBlock
                        )
                }
                GateEvidencePhase::G4 => false,
            };
        let accepted_result = if scoped_legacy_exception.is_none() && result_scope.is_some() {
            matches!(result.trim(), "已通过" | "已通过。")
                || (phase == GateEvidencePhase::G3
                    && result.contains("未通过")
                    && !result.contains("已通过"))
        } else if allow_legacy_exception
            && phase == GateEvidencePhase::G3
            && expects_failed_assertion
        {
            result.contains("未通过") && !result.contains("已通过")
        } else if expects_failed_assertion {
            matches!(result.trim(), "未通过" | "未通过。")
        } else {
            matches!(result.trim(), "已通过" | "已通过。")
        };
        if !accepted_result {
            return Err(format!(
                "{label} comment 的 `Gate 断言` 必须在规范命令后明确记录 `{}`",
                if scoped_legacy_exception.is_none() && result_scope.is_some() {
                    "已通过` 或 `未通过"
                } else if expects_failed_assertion {
                    "未通过"
                } else {
                    "已通过"
                }
            ));
        }
    }
    Ok(unique_commands)
}

pub(super) fn semantic_gate_assertion_commands_with_legacy_exception(
    body: &str,
    label: &str,
    phase: GateEvidencePhase,
    result_scope: Option<(u64, bool)>,
) -> Result<BTreeSet<String>, String> {
    let raw_commands =
        gate_assertion_commands_with_legacy_exception(body, label, phase, result_scope)?;
    let raw_count = raw_commands.len();
    let semantic_commands = raw_commands
        .into_iter()
        .map(|command| {
            parse_gate_assertion_command(&command, phase)
                .map(|args| expected_gate_command(&args, phase))
                .map_err(|error| format!("{label} comment 的 `Gate 断言` 命令无效：{error}"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if semantic_commands.len() != raw_count {
        return Err(format!(
            "{label} comment 的 `Gate 断言` 不得用不同参数顺序重复同一语义命令"
        ));
    }
    Ok(semantic_commands)
}

pub(super) fn expected_gate_command(args: &GateEvidenceArgs, phase: GateEvidencePhase) -> String {
    expected_gate_command_with_rust_version(args, phase, env!("CARGO_PKG_RUST_VERSION"))
        .expect("CARGO_PKG_RUST_VERSION must be a stable Rust version")
}

pub(super) fn expected_gate_command_with_rust_version(
    args: &GateEvidenceArgs,
    phase: GateEvidencePhase,
    rust_version: &str,
) -> Result<String, String> {
    let rust_version = StableRustVersion::parse(rust_version)?;
    if !rust_version.is_gate_command_v1_release() {
        return Err(format!(
            "gate-command v1 只生成已纳入策略的 Rust `1.<minor>.0` stable toolchain，不接受 `{}`",
            rust_version.canonical()
        ));
    }
    let phase = match phase {
        GateEvidencePhase::G3 => "g3",
        GateEvidencePhase::G4 => "g4",
    };
    let mut command = format!(
        "cargo +{} run --locked -p xtask -- check-gate-evidence {phase} --repo {} --issue {}",
        rust_version.canonical(),
        args.repo,
        args.issue
    );
    if let Some(delivery_pr) = args.delivery_pr {
        command.push_str(&format!(" --delivery-pr {delivery_pr}"));
    }
    for related_pr in &args.related_prs {
        command.push_str(&format!(" --related-pr {related_pr}"));
    }
    Ok(command)
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct StableRustVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

impl StableRustVersion {
    pub(super) fn parse(value: &str) -> Result<Self, String> {
        let parts = value.split('.').collect::<Vec<_>>();
        if !matches!(parts.len(), 2 | 3)
            || parts.iter().any(|part| {
                part.is_empty()
                    || !part.bytes().all(|byte| byte.is_ascii_digit())
                    || (part.len() > 1 && part.starts_with('0'))
            })
        {
            return Err(format!(
                "Rust toolchain `{value}` 必须是无前导零的稳定 `major.minor[.patch]` 版本"
            ));
        }
        let parse = |part: &str| {
            part.parse::<u64>()
                .map_err(|_| format!("Rust toolchain 版本分量超出范围：{value}"))
        };
        Ok(Self {
            major: parse(parts[0])?,
            minor: parse(parts[1])?,
            patch: parts.get(2).map_or(Ok(0), |part| parse(part))?,
        })
    }

    pub(super) fn canonical(self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }

    fn is_gate_command_v1_release(self) -> bool {
        self.major == 1 && self.patch == 0
    }
}

pub(super) fn parse_gate_assertion_command(
    command: &str,
    expected_phase: GateEvidencePhase,
) -> Result<GateEvidenceArgs, String> {
    parse_gate_assertion_command_with_current_version(
        command,
        expected_phase,
        env!("CARGO_PKG_RUST_VERSION"),
    )
}

pub(super) fn parse_gate_assertion_command_with_current_version(
    command: &str,
    expected_phase: GateEvidencePhase,
    current_rust_version: &str,
) -> Result<GateEvidenceArgs, String> {
    let tokens = command.split_ascii_whitespace().collect::<Vec<_>>();
    if tokens.len() < 8 || tokens.first() != Some(&"cargo") || tokens.get(2) != Some(&"run") {
        return Err("规范命令必须以 `cargo +<stable> run` 开始".to_string());
    }
    let toolchain = tokens
        .get(1)
        .and_then(|token| token.strip_prefix('+'))
        .ok_or("规范命令必须显式记录 `+<stable toolchain>`")?;
    let actual_version = StableRustVersion::parse(toolchain)?;
    let minimum_version = StableRustVersion::parse("1.96.0")?;
    let current_version = StableRustVersion::parse(current_rust_version)?;
    if !current_version.is_gate_command_v1_release() {
        return Err(format!(
            "workspace Rust version `{}` 尚未纳入 gate-command v1 stable release 策略",
            current_version.canonical()
        ));
    }
    if !actual_version.is_gate_command_v1_release() {
        return Err(format!(
            "Rust toolchain `{}` 不是 gate-command v1 支持的 `1.<minor>.0` stable release",
            actual_version.canonical()
        ));
    }
    if actual_version < minimum_version || actual_version > current_version {
        return Err(format!(
            "Rust toolchain `{}` 超出 gate-command v1 历史兼容边界 `{}..={}`",
            actual_version.canonical(),
            minimum_version.canonical(),
            current_version.canonical()
        ));
    }

    let separator = tokens
        .iter()
        .position(|token| *token == "--")
        .ok_or("规范命令缺少 cargo/xtask 参数分隔符 `--`")?;
    let mut locked = false;
    let mut package = None;
    let mut index = 3;
    while index < separator {
        match tokens[index] {
            "--locked" => {
                if locked {
                    return Err("cargo `--locked` 只能指定一次".to_string());
                }
                locked = true;
                index += 1;
            }
            "-p" | "--package" => {
                let value = tokens
                    .get(index + 1)
                    .filter(|_| index + 1 < separator)
                    .ok_or("cargo package 参数缺少值")?;
                if package.replace(*value).is_some() {
                    return Err("cargo package 只能指定一次".to_string());
                }
                index += 2;
            }
            flag => return Err(format!("规范命令包含未知 cargo 参数：{flag}")),
        }
    }
    if !locked || package != Some("xtask") {
        return Err("规范命令必须包含唯一的 `--locked` 与 `-p xtask`".to_string());
    }
    if tokens.get(separator + 1) != Some(&"check-gate-evidence") {
        return Err("规范命令的 xtask 子命令必须是 `check-gate-evidence`".to_string());
    }
    let args = tokens[separator + 2..]
        .iter()
        .map(|token| (*token).to_string())
        .collect::<Vec<_>>();
    let args = super::args::parse_gate_evidence_args(&args)?;
    if args.phase != expected_phase {
        return Err(format!(
            "规范命令阶段与 comment 不一致：预期 {expected_phase:?}，实际 {:?}",
            args.phase
        ));
    }
    Ok(args)
}

pub(super) fn body_sha256(body: &str) -> String {
    let digest = Sha256::digest(body.as_bytes());
    let mut encoded = String::with_capacity("sha256:".len() + digest.len() * 2);
    encoded.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

pub(super) fn parse_g3_comment_correction_record(
    comment: &GitHubComment,
) -> Result<G3CommentCorrectionRecord, String> {
    if comment.includes_created_edit {
        return Err("G3 comment correction appendix 在创建后被编辑".to_string());
    }
    if comment.body.matches(G3_COMMENT_CORRECTION_START).count() != 1 {
        return Err(format!(
            "G3 comment correction appendix 必须包含且只包含一个 `{G3_COMMENT_CORRECTION_START}` 记录"
        ));
    }
    let (_, after_start) = comment
        .body
        .split_once(G3_COMMENT_CORRECTION_START)
        .ok_or("G3 comment correction appendix 缺少起始标记")?;
    let (json, _) = after_start
        .split_once(G3_COMMENT_CORRECTION_END)
        .ok_or("G3 comment correction appendix 缺少结束标记")?;
    let record = serde_json::from_str::<G3CommentCorrectionRecord>(json.trim())
        .map_err(|error| format!("G3 comment correction 不是 schema v1 JSON：{error}"))?;
    if record.schema_version != 1 {
        return Err(format!(
            "G3 comment correction schemaVersion 必须为 1，实际为 {}",
            record.schema_version
        ));
    }
    Ok(record)
}

fn canonicalize_g3_shadow_field(body: &str) -> Result<String, String> {
    let mut found = false;
    let mut canonical = String::with_capacity(body.len());
    for segment in body.split_inclusive('\n') {
        let (line, newline) = segment
            .strip_suffix('\n')
            .map_or((segment, ""), |line| (line, "\n"));
        if line.starts_with(G3_EVIDENCE_SHADOW_COMMENT_FIELD) {
            if found {
                return Err("G3 Evidence Gate Shadow 字段不能重复".to_string());
            }
            found = true;
            let value = line
                .strip_prefix(G3_EVIDENCE_SHADOW_COMMENT_FIELD)
                .expect("checked prefix");
            let value = canonicalize_g3_shadow_wrapper(value)?;
            canonical.push_str(G3_EVIDENCE_SHADOW_COMMENT_FIELD);
            canonical.push_str(&value);
            canonical.push_str(newline);
        } else {
            canonical.push_str(segment);
        }
    }
    if !found {
        return Err("G3 comment correction 的正文缺少 G3 Evidence Gate Shadow 字段".to_string());
    }
    Ok(canonical)
}

fn canonicalize_g3_shadow_wrapper(value: &str) -> Result<String, String> {
    if !value.starts_with('`') {
        return Ok(value.to_string());
    }

    let (wrapped, suffix) = if value.ends_with('`') {
        (value, "")
    } else if let Some(wrapped) = value
        .strip_suffix('。')
        .filter(|wrapped| wrapped.ends_with('`'))
    {
        (wrapped, "。")
    } else {
        return Err(
            "G3 Evidence Gate Shadow correction 只能移除完整值的一层反引号包裹".to_string(),
        );
    };
    if wrapped.len() < 2 {
        return Err("G3 Evidence Gate Shadow correction 的反引号包裹不能为空".to_string());
    }
    let inner = &wrapped[1..wrapped.len() - 1];
    if inner.is_empty() || inner.contains('`') {
        return Err(
            "G3 Evidence Gate Shadow correction 的完整反引号包裹必须非空且只能使用一层".to_string(),
        );
    }
    Ok(format!("{inner}{suffix}"))
}

struct ValidatedG3CommentCorrection {
    original_effective_at: String,
    original_body_sha256: String,
}

fn validate_g3_comment_correction(
    issue: u64,
    pr_number: u64,
    pr: &GitHubPullRequest,
    g3_comment: &GitHubComment,
    merged_at: &str,
) -> Result<ValidatedG3CommentCorrection, String> {
    let candidates = pr
        .comments
        .iter()
        .filter(|comment| {
            comment.body.contains(G3_COMMENT_CORRECTION_START)
                && is_trusted_g3_owner_comment(comment)
        })
        .map(|comment| parse_g3_comment_correction_record(comment).map(|record| (comment, record)))
        .collect::<Result<Vec<_>, _>>()?;
    let mut matching = candidates.into_iter().filter(|(_, record)| {
        record.issue == issue
            && record.pull_request == pr_number
            && record.g3_comment == g3_comment.url
    });
    let (appendix, record) = matching.next().ok_or_else(|| {
        format!(
            "post-merge edited G3 comment 缺少 Issue #{} / PR #{} 的 `{G3_COMMENT_CORRECTION_START}` appendix",
            issue, pr_number
        )
    })?;
    if matching.next().is_some() {
        return Err("同一 G3 comment 只能有一条 correction appendix".to_string());
    }
    for (field, value) in [
        ("id", record.id.as_str()),
        ("currentHeadOid", record.current_head_oid.as_str()),
        ("originalBodySha256", record.original_body_sha256.as_str()),
        ("newBodySha256", record.new_body_sha256.as_str()),
        ("editedAt", record.edited_at.as_str()),
        ("editor", record.editor.as_str()),
        ("reason", record.reason.as_str()),
        ("risk", record.risk.as_str()),
        ("acceptanceBoundary", record.acceptance_boundary.as_str()),
        ("followUpIssue", record.follow_up_issue.as_str()),
        ("cleanupOwner", record.cleanup_owner.as_str()),
        ("authorizedBy", record.authorized_by.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(format!("G3 comment correction `{field}` 不能为空"));
        }
    }
    if record.current_head_oid != pr.head_ref_oid {
        return Err("G3 comment correction currentHeadOid 与 PR headRefOid 不一致".to_string());
    }
    let author = appendix
        .author
        .as_ref()
        .map(|author| author.login.as_str())
        .ok_or("G3 comment correction appendix 缺少 author")?;
    if !author.eq_ignore_ascii_case(&record.authorized_by)
        || !G3_OWNER_ACTORS
            .iter()
            .any(|owner| owner.eq_ignore_ascii_case(author))
    {
        return Err(
            "G3 comment correction 必须由 trusted G3 Owner 以 authorizedBy 签署".to_string(),
        );
    }
    let follow_up = record
        .follow_up_issue
        .strip_prefix('#')
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|number| *number > 0)
        .ok_or("G3 comment correction followUpIssue 必须是 `#<positive number>`")?;
    if record.follow_up_issue != format!("#{follow_up}") || follow_up == issue {
        return Err(
            "G3 comment correction followUpIssue 必须是独立、无前导零的 `#<positive number>`"
                .to_string(),
        );
    }

    let edits = g3_comment
        .user_content_edits
        .as_ref()
        .ok_or("edited G3 comment 缺少 hydrated userContentEdits")?;
    if edits.page_info.has_next_page {
        return Err(
            "edited G3 comment 的 userContentEdits 超过 100 条，correction 失败关闭".to_string(),
        );
    }
    let updated_at = g3_comment
        .updated_at
        .as_deref()
        .ok_or("edited G3 comment 缺少 hydrated updatedAt")?;
    if record.edited_at != updated_at {
        return Err("G3 comment correction editedAt 与 REST updatedAt 不一致".to_string());
    }
    let mut ordered = edits
        .nodes
        .iter()
        .map(|edit| {
            parse_utc_timestamp_seconds(&edit.edited_at)
                .ok_or("G3 comment correction userContentEdit.editedAt 无效")
                .map(|timestamp| (timestamp, edit))
        })
        .collect::<Result<Vec<_>, _>>()?;
    ordered.sort_by_key(|(timestamp, _)| *timestamp);
    let current_index = ordered
        .iter()
        .position(|(_, edit)| edit.edited_at == record.edited_at)
        .ok_or("G3 comment correction editedAt 未命中 userContentEdits")?;
    if current_index + 1 != ordered.len() || current_index == 0 {
        return Err("G3 comment correction 必须绑定最新编辑及其紧邻前一版正文".to_string());
    }
    let (original_seconds, original_edit) = ordered[current_index - 1];
    let (corrected_seconds, corrected_edit) = ordered[current_index];
    let original_body = original_edit
        .diff
        .as_deref()
        .ok_or("G3 comment correction 原始 UserContentEdit 缺少 diff snapshot")?;
    let corrected_body = corrected_edit
        .diff
        .as_deref()
        .ok_or("G3 comment correction 最新 UserContentEdit 缺少 diff snapshot")?;
    if corrected_body != g3_comment.body {
        return Err("G3 comment correction 最新 diff snapshot 与当前正文不一致".to_string());
    }
    let editor = corrected_edit
        .editor
        .as_ref()
        .map(|editor| editor.login.as_str())
        .ok_or("G3 comment correction 最新 edit 缺少 editor")?;
    if !editor.eq_ignore_ascii_case(&record.editor)
        || !G3_OWNER_ACTORS
            .iter()
            .any(|owner| owner.eq_ignore_ascii_case(editor))
    {
        return Err(
            "G3 comment correction editor 必须与记录一致且属于 trusted G3 Owner".to_string(),
        );
    }
    if body_sha256(original_body) != record.original_body_sha256
        || body_sha256(corrected_body) != record.new_body_sha256
    {
        return Err(
            "G3 comment correction 原始/新正文 SHA-256 与 GitHub edit history 不一致".to_string(),
        );
    }
    if original_body == corrected_body
        || canonicalize_g3_shadow_field(original_body)?
            != canonicalize_g3_shadow_field(corrected_body)?
    {
        return Err("G3 comment correction 只允许完整 shadow 字段的一层反引号包裹差异".to_string());
    }
    let merged_seconds = parse_utc_timestamp_seconds(merged_at)
        .ok_or("G3 comment correction 的 PR mergedAt 无效")?;
    if original_seconds >= merged_seconds || corrected_seconds <= merged_seconds {
        return Err(
            "G3 comment correction 必须证明原始版严格早于 merge、格式编辑严格晚于 merge"
                .to_string(),
        );
    }
    let appendix_seconds = parse_utc_timestamp_seconds(&appendix.created_at)
        .ok_or("G3 comment correction appendix createdAt 无效")?;
    if appendix_seconds <= corrected_seconds {
        return Err("G3 comment correction appendix 必须严格晚于被签署的格式编辑".to_string());
    }
    Ok(ValidatedG3CommentCorrection {
        original_effective_at: original_edit.edited_at.clone(),
        original_body_sha256: body_sha256(original_body),
    })
}

pub(super) fn g3_effective_at_for_merge_validation(
    pr: &GitHubPullRequest,
    permalink: &str,
    label: &str,
    args: &GateEvidenceArgs,
) -> Result<String, String> {
    let comment = pr
        .comments
        .iter()
        .find(|comment| comment.url == permalink)
        .ok_or_else(|| format!("{label} permalink 未指向该 PR 的 comment"))?;
    let effective_at = g3_comment_effective_at(comment, label)?;
    let Some(merged_at) = pr.merged_at.as_deref() else {
        return Ok(effective_at.to_string());
    };
    if effective_at >= merged_at {
        let (_, pr_number) = super::args::selected_gate_evidence_pr(args)?;
        let correction =
            validate_g3_comment_correction(args.issue, pr_number, pr, comment, merged_at).map_err(
                |correction_error| {
                format!(
                    "{label} comment 生效时间必须严格早于 PR 合并时间；correction 验证失败：{correction_error}"
                )
            },
            )?;
        if correction.original_effective_at.as_str() >= merged_at {
            return Err(format!(
                "{label} correction 恢复的原始 comment 生效时间必须严格早于 PR 合并时间"
            ));
        }
        return Ok(correction.original_effective_at);
    }
    Ok(effective_at.to_string())
}

pub(super) fn validate_g3_timing(
    pr: &GitHubPullRequest,
    permalink: &str,
    label: &str,
    args: &GateEvidenceArgs,
) -> Result<(), String> {
    g3_effective_at_for_merge_validation(pr, permalink, label, args).map(|_| ())
}

pub(super) fn validate_external_review_g3(
    repo: &str,
    issue_number: u64,
    number: u64,
    pr: &GitHubPullRequest,
    label: &str,
    historical_appendix: Option<&GitHubComment>,
    validation_time: Option<&str>,
) -> Result<(), String> {
    let permalink = completed_gate_permalink(&pr.body, "G3")?;
    let comment = pr
        .comments
        .iter()
        .find(|comment| comment.url == permalink)
        .ok_or_else(|| format!("{label} G3 permalink 未指向该 PR 的 comment"))?;
    let effective_at = g3_comment_effective_at(comment, label)?;
    let gate_result = parse_g3_result(&comment.body)?;
    if validate_g3_exception(
        issue_number,
        number,
        pr,
        comment,
        gate_result,
        historical_appendix,
        validation_time,
    )? {
        println!(
            "G3 Gate 状态：accepted_exception；Issue #{issue_number}，PR #{number}；未映射为 pass"
        );
        return Ok(());
    }
    let result = match gate_result {
        G3Result::Waived => {
            let current_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| format!("系统时间早于 Unix epoch：{error}"))?
                .as_secs();
            let validation_time = waiver_validation_time(validation_time, current_time)?;
            let waiver = parse_gate_waiver(comment, issue_number, validation_time)?;
            external_review::evaluate_live_with_waiver(repo, number, waiver)?
        }
        G3Result::Pass | G3Result::Bootstrap => external_review::evaluate_live(repo, number)?,
        G3Result::Exception | G3Result::LegacyBlock => {
            return Err("G3 exception 状态未形成有效 accepted_exception".to_string());
        }
    };
    let expected_state = match gate_result {
        G3Result::Waived => external_review::ExternalReviewState::Waived,
        G3Result::Pass | G3Result::Bootstrap => external_review::ExternalReviewState::Pass,
        G3Result::Exception | G3Result::LegacyBlock => unreachable!("handled above"),
    };
    if result.state != expected_state {
        return Err(format!(
            "{label} 的 G3 结果与 External Review Gate 不一致：G3={gate_result:?}，期望 {expected_state:?}，实际 {:?}",
            result.state
        ));
    }
    if !comment.body.contains(result.current_head_oid()) {
        return Err(format!(
            "{label} G3 comment 未记录 External Review Gate 对应的完整 current head `{}`",
            result.current_head_oid()
        ));
    }
    if gate_result != G3Result::Waived {
        let completion_time = result
            .completion_time()
            .ok_or_else(|| format!("{label} pass 结果缺少 completion time"))?;
        validate_g3_comment_after_external_review_completion(effective_at, completion_time, label)?;
    }
    Ok(())
}

pub(super) fn waiver_validation_time(
    historical_related_merged_at: Option<&str>,
    current_time: u64,
) -> Result<u64, String> {
    match historical_related_merged_at {
        Some(merged_at) => parse_utc_timestamp_seconds(merged_at)
            .ok_or_else(|| "历史 Related PR mergedAt 不是 UTC RFC3339 时间".to_string()),
        None => Ok(current_time),
    }
}

pub(super) fn validate_g3_comment_after_external_review_completion(
    comment_effective_at: &str,
    completion_time: &str,
    label: &str,
) -> Result<(), String> {
    let comment_seconds = parse_utc_timestamp_seconds(comment_effective_at)
        .ok_or_else(|| format!("{label} G3 comment effectiveAt 不是 UTC RFC3339 时间"))?;
    let completion_seconds = parse_utc_timestamp_seconds(completion_time)
        .ok_or_else(|| format!("{label} external review completion 不是 UTC RFC3339 时间"))?;
    if comment_seconds <= completion_seconds {
        return Err(format!(
            "{label} G3 comment 生效时间必须严格晚于最终 external review completion；GitHub 同秒无法证明 completion 已完成：comment={comment_effective_at}，completion={completion_time}"
        ));
    }
    Ok(())
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
    let value = parse_optional_backtick_value(value, "Gate 结果")?;
    match value {
        "G3 Pass" => Ok(G3Result::Pass),
        "G3 Waived" => Ok(G3Result::Waived),
        "R0-R1 bootstrap" => Ok(G3Result::Bootstrap),
        "G3 Exception" => Ok(G3Result::Exception),
        "G3 Block" => Ok(G3Result::LegacyBlock),
        value
            if value
                .strip_prefix("G3 Block（")
                .and_then(|reason| reason.strip_suffix('）'))
                .is_some_and(|reason| !reason.trim().is_empty()) =>
        {
            Ok(G3Result::LegacyBlock)
        }
        _ => Err(format!(
            "current G3 comment 的 Gate 结果无效：`{value}`；应为 `G3 Pass`、`G3 Waived`、`G3 Exception`、legacy `G3 Block` 或 `R0-R1 bootstrap`"
        )),
    }
}

pub(super) fn parse_optional_backtick_value<'a>(
    value: &'a str,
    field: &str,
) -> Result<&'a str, String> {
    let value = value.trim().trim_end_matches('。').trim();
    let starts = value.starts_with('`');
    let ends = value.ends_with('`');
    match (starts, ends) {
        (true, true) => {
            if value.len() < 2 {
                return Err(format!("`{field}` 的完整反引号包裹必须包含起止反引号"));
            }
            let inner = &value[1..value.len() - 1];
            if inner.is_empty() || inner.contains('`') {
                return Err(format!("`{field}` 的完整反引号包裹必须非空且只能使用一层"));
            }
            Ok(inner)
        }
        (false, false) => Ok(value),
        _ => Err(format!("`{field}` 只能不加包裹，或使用一层完整反引号包裹")),
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
    parse_utc_timestamp_seconds(&record.delivery_merged_at).ok_or_else(|| {
        "G3 full-set recovery deliveryMergedAt 不是 UTC RFC3339 秒级时间".to_string()
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

pub(super) fn parse_g3_exception_records(
    appendix: &GitHubComment,
) -> Result<Vec<G3ExceptionRecord>, String> {
    let marker_count = appendix.body.matches(G3_EXCEPTION_START).count();
    if marker_count == 0 {
        return Ok(Vec::new());
    }
    if appendix.includes_created_edit {
        return Err("g3-exception appendix 在创建后被编辑".to_string());
    }
    let mut records = Vec::with_capacity(marker_count);
    let mut ids = BTreeSet::new();
    let mut targets = BTreeSet::new();
    for (index, after_start) in appendix.body.split(G3_EXCEPTION_START).skip(1).enumerate() {
        let (json, _) = after_start
            .split_once(G3_EXCEPTION_END)
            .ok_or_else(|| format!("g3-exception 第 {} 个记录缺少结束标记", index + 1))?;
        let record = serde_json::from_str::<G3ExceptionRecord>(json.trim()).map_err(|error| {
            format!(
                "g3-exception 第 {} 个记录不是 schema v1 JSON：{error}",
                index + 1
            )
        })?;
        if record.schema_version != 1 {
            return Err(format!(
                "g3-exception schemaVersion 必须为 1，实际为 {}",
                record.schema_version
            ));
        }
        if !matches!(
            record.exception_type.as_str(),
            "confirmed_gate_defect" | "legacy_evidence_reconstruction"
        ) {
            return Err(format!(
                "g3-exception exceptionType 不在首版 allowlist：{}",
                record.exception_type
            ));
        }
        if !ids.insert(record.id.clone()) {
            return Err(format!("g3-exception id 不能重复：{}", record.id));
        }
        if !targets.insert((record.issue, record.pull_request)) {
            return Err(format!(
                "g3-exception 每个 Issue/PR target 只能有一条记录：Issue #{} / PR #{}",
                record.issue, record.pull_request
            ));
        }
        records.push(record);
    }
    Ok(records)
}

fn exception_record_for_target<'a>(
    appendices: impl IntoIterator<Item = &'a GitHubComment>,
    issue: u64,
    pr_number: u64,
    g3_permalink: &str,
) -> Result<Option<(&'a GitHubComment, G3ExceptionRecord)>, String> {
    let mut matching = Vec::new();
    for appendix in appendices {
        if !is_trusted_g3_owner_comment(appendix) {
            continue;
        }
        for record in parse_g3_exception_records(appendix)? {
            if record.issue == issue
                && record.pull_request == pr_number
                && record.g3_comment == g3_permalink
            {
                matching.push((appendix, record));
            }
        }
    }
    match matching.len() {
        0 => Ok(None),
        1 => Ok(matching.pop()),
        _ => Err(format!(
            "Issue #{issue} / PR #{pr_number} 的 G3 comment 只能绑定一条 g3-exception 记录"
        )),
    }
}

fn is_trusted_g3_owner_comment(comment: &GitHubComment) -> bool {
    comment.author.as_ref().is_some_and(|author| {
        G3_OWNER_ACTORS
            .iter()
            .any(|owner| owner.eq_ignore_ascii_case(&author.login))
    })
}

pub(super) fn historical_exception_applies_to_target(
    appendix: &GitHubComment,
    issue: u64,
    pr_number: u64,
    g3_permalink: &str,
) -> Result<bool, String> {
    match exception_record_for_target(std::iter::once(appendix), issue, pr_number, g3_permalink)? {
        Some((_, record)) if record.exception_type == "legacy_evidence_reconstruction" => Ok(true),
        Some(_) => Err(format!(
            "Issue #{issue} / PR #{pr_number} 的 G4 historical appendix 只能使用 `legacy_evidence_reconstruction`"
        )),
        None => Ok(false),
    }
}

fn validate_g3_exception_record(
    appendix: &GitHubComment,
    record: &G3ExceptionRecord,
    pr: &GitHubPullRequest,
    g3_comment: &GitHubComment,
    gate_result: G3Result,
    validation_time: Option<&str>,
) -> Result<(), String> {
    for (field, value) in [
        ("id", record.id.as_str()),
        ("currentHeadOid", record.current_head_oid.as_str()),
        ("currentBaseOid", record.current_base_oid.as_str()),
        ("g3Comment", record.g3_comment.as_str()),
        (
            "g3CommentBodySha256",
            record.g3_comment_body_sha256.as_str(),
        ),
        ("reason", record.reason.as_str()),
        ("risk", record.risk.as_str()),
        ("acceptanceBoundary", record.acceptance_boundary.as_str()),
        ("acceptedAt", record.accepted_at.as_str()),
        ("expiresAt", record.expires_at.as_str()),
        ("followUpIssue", record.follow_up_issue.as_str()),
        ("cleanupOwner", record.cleanup_owner.as_str()),
        ("authorizedBy", record.authorized_by.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(format!("g3-exception `{field}` 不能为空"));
        }
    }
    if record.current_head_oid != pr.head_ref_oid || record.current_base_oid != pr.base_ref_oid {
        return Err("g3-exception current head/base 与 GitHub PR identity 不一致".to_string());
    }
    if record.g3_comment != g3_comment.url {
        return Err("g3-exception 未精确绑定目标 G3 comment permalink".to_string());
    }
    let correction = if record.g3_comment_body_sha256 != body_sha256(&g3_comment.body) {
        let merged_at = pr.merged_at.as_deref().ok_or(
            "g3-exception 未精确绑定目标 G3 comment body SHA-256，且当前 PR 尚未合并，不能使用 correction 恢复",
        )?;
        let correction = validate_g3_comment_correction(
            record.issue,
            record.pull_request,
            pr,
            g3_comment,
            merged_at,
        )
        .map_err(|error| {
            format!(
                "g3-exception 未精确绑定当前 G3 comment body SHA-256，correction 恢复失败：{error}"
            )
        })?;
        if record.g3_comment_body_sha256 != correction.original_body_sha256 {
            return Err(
                "g3-exception body SHA-256 未绑定当前正文或 correction 恢复的原始正文".to_string(),
            );
        }
        Some(correction)
    } else {
        None
    };
    let author = appendix
        .author
        .as_ref()
        .map(|author| author.login.as_str())
        .ok_or("g3-exception appendix 缺少 author")?;
    if !author.eq_ignore_ascii_case(&record.authorized_by)
        || !G3_OWNER_ACTORS
            .iter()
            .any(|owner| owner.eq_ignore_ascii_case(author))
    {
        return Err("g3-exception 必须由 trusted G3 Owner 以 authorizedBy 签署".to_string());
    }
    if record.accepted_at != appendix.created_at {
        return Err("g3-exception acceptedAt 必须等于未编辑 appendix 的 createdAt".to_string());
    }
    let accepted_at = parse_utc_timestamp_seconds(&record.accepted_at)
        .ok_or("g3-exception acceptedAt 必须是 UTC RFC3339 秒级时间")?;
    let expires_at = parse_utc_timestamp_seconds(&record.expires_at)
        .ok_or("g3-exception expiresAt 必须是 UTC RFC3339 秒级时间")?;
    if expires_at <= accepted_at || expires_at - accepted_at > G3_EXCEPTION_MAX_SECONDS {
        return Err("g3-exception 有效期必须晚于 acceptedAt 且不超过 24 小时".to_string());
    }
    let current_effective_at = g3_comment_effective_at(g3_comment, "g3-exception target comment")?;
    let g3_effective_at = parse_utc_timestamp_seconds(
        correction
            .as_ref()
            .map(|correction| correction.original_effective_at.as_str())
            .unwrap_or(current_effective_at),
    )
    .ok_or("g3-exception target effectiveAt 无效")?;
    if accepted_at <= g3_effective_at {
        return Err("g3-exception appendix 必须严格晚于目标 G3 comment 生效时间".to_string());
    }
    let follow_up = record
        .follow_up_issue
        .strip_prefix('#')
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|number| *number > 0)
        .ok_or("g3-exception followUpIssue 必须是 `#<positive number>`")?;
    if record.follow_up_issue != format!("#{follow_up}") || follow_up == record.issue {
        return Err(
            "g3-exception followUpIssue 必须是独立、无前导零的 `#<positive number>`".to_string(),
        );
    }
    let exception_line = appendix
        .body
        .lines()
        .find(|line| line.trim_start().starts_with("- 例外："))
        .ok_or("g3-exception appendix 缺少可见的 `- 例外：` 行")?;
    let visible_refs = markdown_reference_labels(exception_line)
        .into_iter()
        .map(str::to_ascii_lowercase)
        .collect::<BTreeSet<_>>();
    if record.evidence_refs.is_empty() {
        return Err("g3-exception evidenceRefs 不能为空".to_string());
    }
    let mut evidence_refs = BTreeSet::new();
    for evidence_ref in &record.evidence_refs {
        let normalized = evidence_ref.to_ascii_lowercase();
        if !evidence_refs.insert(normalized.clone()) {
            return Err(format!("g3-exception evidenceRefs 重复：{evidence_ref}"));
        }
        if !visible_refs.contains(&normalized)
            || reference_github_url(&appendix.body, evidence_ref).is_none()
        {
            return Err(format!(
                "g3-exception evidence ref `{evidence_ref}` 必须由 `- 例外：` 可见引用并解析为 GitHub HTTPS URL"
            ));
        }
    }

    match record.exception_type.as_str() {
        "confirmed_gate_defect" => {
            if gate_result != G3Result::Exception {
                return Err(
                    "confirmed_gate_defect 只能与 canonical `G3 Exception` 配对".to_string()
                );
            }
            let validation_time = match validation_time {
                Some(value) => {
                    parse_utc_timestamp_seconds(value).ok_or("g3-exception validation time 无效")?
                }
                None => SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|error| format!("系统时间早于 Unix epoch：{error}"))?
                    .as_secs(),
            };
            if accepted_at >= validation_time || expires_at <= validation_time {
                return Err(
                    "confirmed_gate_defect 必须在验证/merge 时点前被接受且当时未过期".to_string(),
                );
            }
        }
        "legacy_evidence_reconstruction" => {
            let merged_at = validation_time
                .and_then(parse_utc_timestamp_seconds)
                .ok_or("legacy_evidence_reconstruction 只允许历史 G4 replay")?;
            if !matches!(gate_result, G3Result::Pass | G3Result::LegacyBlock) {
                return Err(
                    "legacy_evidence_reconstruction 只兼容历史 `G3 Pass + 未通过` 或 `G3 Block`"
                        .to_string(),
                );
            }
            if accepted_at <= merged_at {
                return Err(
                    "legacy_evidence_reconstruction 必须明确发生在原 PR merge 后，且不追授原 merge 合规"
                        .to_string(),
                );
            }
        }
        _ => unreachable!("exception type checked while parsing"),
    }
    Ok(())
}

pub(super) fn validate_g3_exception(
    issue_number: u64,
    pr_number: u64,
    pr: &GitHubPullRequest,
    g3_comment: &GitHubComment,
    gate_result: G3Result,
    historical_appendix: Option<&GitHubComment>,
    validation_time: Option<&str>,
) -> Result<bool, String> {
    let current_record = exception_record_for_target(
        pr.comments
            .iter()
            .filter(|comment| comment.url != g3_comment.url),
        issue_number,
        pr_number,
        &g3_comment.url,
    )?;
    let historical_record = match historical_appendix {
        Some(appendix) => exception_record_for_target(
            std::iter::once(appendix),
            issue_number,
            pr_number,
            &g3_comment.url,
        )?,
        None => None,
    };
    if current_record
        .as_ref()
        .is_some_and(|(_, record)| record.exception_type == "legacy_evidence_reconstruction")
    {
        return Err(
            "legacy_evidence_reconstruction 只能来自 Issue G4 historical appendix，不能来自 PR comment"
                .to_string(),
        );
    }
    if historical_record
        .as_ref()
        .is_some_and(|(_, record)| record.exception_type != "legacy_evidence_reconstruction")
    {
        return Err(
            "Issue G4 historical appendix 只能承载 legacy_evidence_reconstruction".to_string(),
        );
    }
    let selected = match (current_record, historical_record) {
        (Some(_), Some(_)) => {
            return Err("同一 G3 target 不得同时声明 current 与 historical exception".to_string());
        }
        (Some(record), None) => Some(record),
        (None, Some(record)) => Some(record),
        (None, None) => None,
    };
    match (gate_result, selected) {
        (G3Result::Exception | G3Result::LegacyBlock, None) => Err(format!(
            "{} 缺少匹配的 `{G3_EXCEPTION_START}` 结构化记录",
            gate_result.machine_state()
        )),
        (G3Result::Pass, None) | (G3Result::Bootstrap, None) | (G3Result::Waived, None) => {
            Ok(false)
        }
        (G3Result::Waived | G3Result::Bootstrap, Some(_)) => {
            Err("G3 Waived/bootstrap 不得夹带 g3-exception 结构化记录".to_string())
        }
        (result, Some((appendix, record))) => {
            validate_g3_exception_record(
                appendix,
                &record,
                pr,
                g3_comment,
                result,
                validation_time,
            )?;
            Ok(true)
        }
    }
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
    if g3_comment_effective_at(comment, "G3 comment")? < EXTERNAL_REVIEW_G3_ACTIVATION {
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
        G3Result::Pass | G3Result::Bootstrap | G3Result::Exception | G3Result::LegacyBlock => {
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
    let effective_at =
        parse_utc_timestamp_seconds(g3_comment_effective_at(comment, "G3 Waived comment")?)
            .ok_or_else(|| "G3 Waived comment effectiveAt 不是 UTC RFC3339 秒级时间".to_string())?;
    let expires_at = parse_utc_timestamp_seconds(&record.expires_at)
        .ok_or_else(|| "G3 Waived expiresAt 必须是 UTC RFC3339 秒级时间".to_string())?;
    if expires_at <= effective_at {
        return Err("G3 Waived expiresAt 必须晚于 comment effectiveAt".to_string());
    }
    if expires_at - effective_at > EXTERNAL_REVIEW_WAIVER_MAX_SECONDS {
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
    let timestamp = value.strip_suffix('Z')?;
    let whole_seconds = if let Some((whole_seconds, fractional_seconds)) = timestamp.split_once('.')
    {
        if fractional_seconds.is_empty()
            || !fractional_seconds.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        whole_seconds
    } else {
        timestamp
    };
    let bytes = whole_seconds.as_bytes();
    if bytes.len() != 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let year = whole_seconds.get(0..4)?.parse::<u64>().ok()?;
    let month = whole_seconds.get(5..7)?.parse::<u64>().ok()?;
    let day = whole_seconds.get(8..10)?.parse::<u64>().ok()?;
    let hour = whole_seconds.get(11..13)?.parse::<u64>().ok()?;
    let minute = whole_seconds.get(14..16)?.parse::<u64>().ok()?;
    let second = whole_seconds.get(17..19)?.parse::<u64>().ok()?;
    if year < 1970 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let leap = is_leap_year(year);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let max_day = month_days[(month - 1) as usize];
    if day == 0 || day > max_day {
        return None;
    }
    let years_before = year - 1;
    let epoch_years_before = 1969;
    let leap_days_before = years_before / 4 - years_before / 100 + years_before / 400;
    let epoch_leap_days_before =
        epoch_years_before / 4 - epoch_years_before / 100 + epoch_years_before / 400;
    let days_before_year = (year - 1970) * 365 + leap_days_before - epoch_leap_days_before;
    let days_before_month = month_days
        .iter()
        .take((month - 1) as usize)
        .copied()
        .sum::<u64>();
    Some(
        (days_before_year + days_before_month + day - 1) * 86_400
            + hour * 3_600
            + minute * 60
            + second,
    )
}

pub(super) fn is_leap_year(year: u64) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

pub(super) fn g3_requires_external_review(pr: &GitHubPullRequest) -> Result<bool, String> {
    let permalink = completed_gate_permalink(&pr.body, "G3")?;
    let comment = pr
        .comments
        .iter()
        .find(|comment| comment.url == permalink)
        .ok_or_else(|| "G3 permalink 未指向该 PR 的 comment".to_string())?;
    Ok(g3_comment_effective_at(comment, "G3 comment")? >= EXTERNAL_REVIEW_G3_ACTIVATION)
}

pub(super) fn g3_requires_result_validation(
    issue_number: u64,
    pr_number: u64,
    pr: &GitHubPullRequest,
    historical_appendix: Option<&GitHubComment>,
) -> Result<bool, String> {
    let permalink = completed_gate_permalink(&pr.body, "G3")?;
    let comment = pr
        .comments
        .iter()
        .find(|comment| comment.url == permalink)
        .ok_or_else(|| "G3 permalink 未指向该 PR 的 comment".to_string())?;
    let matching_historical_exception = historical_appendix
        .map(|appendix| {
            historical_exception_applies_to_target(appendix, issue_number, pr_number, &permalink)
        })
        .transpose()?
        .unwrap_or(false);
    Ok(
        g3_comment_effective_at(comment, "G3 comment")? >= EXTERNAL_REVIEW_G3_ACTIVATION
            || matches!(
                parse_g3_result(&comment.body),
                Ok(G3Result::Exception | G3Result::LegacyBlock)
            )
            || matching_historical_exception,
    )
}
