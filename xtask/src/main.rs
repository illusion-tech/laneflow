use std::env;
use std::process::{Command, ExitCode};

const ALLOWED_TYPES: &[&str] = &[
    "feat", "fix", "docs", "test", "refactor", "perf", "build", "ci", "chore", "revert",
];

const ALLOWED_SLICES: &[&str] = &[
    "docs-only",
    "governance",
    "core-runtime",
    "data-spec",
    "adapter",
    "authoring-tool",
    "example",
    "cross-layer",
];

const REQUIRED_FIELDS: &[&str] = &["Gate", "Slice", "Impact", "Scope", "Validation", "Docs"];

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("check-commit-messages") => check_commit_messages(args.get(1).map(String::as_str)),
        Some(command) => Err(format!("未知 xtask 命令: {command}")),
        None => Err("缺少 xtask 命令。可用命令: check-commit-messages".to_string()),
    }
}

fn check_commit_messages(explicit_range: Option<&str>) -> Result<(), String> {
    let commit_range = match explicit_range {
        Some(commit_range) => commit_range.to_string(),
        None => commit_range_from_env()?,
    };

    let commits = commit_hashes(&commit_range)?;
    if commits.is_empty() {
        println!("提交范围内没有非 merge commit: {commit_range}");
        return Ok(());
    }

    let mut errors = Vec::new();
    for commit_hash in &commits {
        let message = git(["log", "-1", "--format=%B", commit_hash])?;
        errors.extend(validate_message(commit_hash, &message));
    }

    if errors.is_empty() {
        println!(
            "已校验 {} 个 commit message，范围: {}",
            commits.len(),
            commit_range
        );
        Ok(())
    } else {
        let mut output = String::from("Commit message 校验失败:");
        for error in errors {
            output.push_str("\n- ");
            output.push_str(&error);
        }
        Err(output)
    }
}

fn commit_range_from_env() -> Result<String, String> {
    let event_payload = github_event_payload();
    let event_name = env::var("GITHUB_EVENT_NAME").ok();
    let base_ref = env::var("GITHUB_BASE_REF").ok();
    let github_sha = env::var("GITHUB_SHA").ok();

    commit_range_from_event(
        event_name.as_deref(),
        base_ref.as_deref(),
        github_sha.as_deref(),
        event_payload.as_deref(),
    )
}

fn commit_range_from_event(
    event_name: Option<&str>,
    base_ref: Option<&str>,
    github_sha: Option<&str>,
    event_payload: Option<&str>,
) -> Result<String, String> {
    match event_name {
        Some("pull_request") => {
            let base_ref =
                non_empty_value(base_ref).ok_or("pull_request 事件缺少 GITHUB_BASE_REF")?;
            Ok(format!("origin/{base_ref}..HEAD"))
        }
        Some("push") => {
            let event = event_payload.and_then(push_event_payload);
            let after = event
                .as_ref()
                .and_then(|event| event.after.as_deref())
                .filter(|value| is_non_zero_value(value))
                .map(ToOwned::to_owned)
                .or_else(|| {
                    non_empty_value(github_sha)
                        .filter(|value| is_non_zero_value(value))
                        .map(ToOwned::to_owned)
                })
                .ok_or("push 事件缺少可用的 after 或 GITHUB_SHA，无法推导 commit range")?;

            match event.as_ref().and_then(|event| event.before.as_deref()) {
                Some(before) if is_non_zero_value(before) => Ok(format!("{before}..{after}")),
                _ => Ok(format!("{after}^!")),
            }
        }
        Some(event_name) => Err(format!(
            "不支持从 GitHub event `{event_name}` 自动推导 commit range，请显式传入 rev-range，例如: cargo +1.96.0 run --locked -p xtask -- check-commit-messages origin/main..HEAD"
        )),
        None => Err(
            "非 CI 场景必须显式传入 commit rev-range，例如: cargo +1.96.0 run --locked -p xtask -- check-commit-messages origin/main..HEAD"
                .to_string(),
        ),
    }
}

fn github_event_payload() -> Option<String> {
    let path = env::var("GITHUB_EVENT_PATH").ok()?;
    std::fs::read_to_string(path).ok()
}

#[derive(serde::Deserialize)]
struct PushEventPayload {
    before: Option<String>,
    after: Option<String>,
}

fn push_event_payload(payload: &str) -> Option<PushEventPayload> {
    serde_json::from_str(payload).ok()
}

fn is_zero_oid(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|ch| ch == '0')
}

fn is_non_zero_value(value: &str) -> bool {
    !value.trim().is_empty() && !is_zero_oid(value)
}

fn non_empty_value(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

fn commit_hashes(commit_range: &str) -> Result<Vec<String>, String> {
    let output = git(["rev-list", "--no-merges", "--reverse", commit_range])?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn git<const N: usize>(args: [&str; N]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|err| format!("无法运行 git: {err}"))?;

    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|err| format!("git 输出不是 UTF-8: {err}"))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("git 命令失败: {}", stderr.trim()))
    }
}

fn validate_message(commit_hash: &str, message: &str) -> Vec<String> {
    let title = message.lines().next().unwrap_or_default();
    let mut errors = Vec::new();

    if !valid_conventional_title(title) {
        errors.push("标题不符合 Conventional Commits".to_string());
    }

    for field in REQUIRED_FIELDS {
        if !has_non_empty_field(message, field) {
            errors.push(format!("缺少 `{field}: ` 行"));
        }
    }

    if !has_valid_slice(message) {
        errors.push("`Slice` 缺失或不是支持的 LaneFlow 切片类型".to_string());
    }

    if !has_valid_impact(message) {
        errors.push("`Impact` 必须同时覆盖 core-api、data-format 和 adapter-api".to_string());
    }

    if !has_valid_docs(message) {
        errors.push("`Docs` 必须是 updated、not required 或 pending <reason>".to_string());
    }

    if !has_refs_or_closes(message) {
        errors.push("缺少 `Refs:` 或 `Closes:` footer".to_string());
    }

    errors
        .into_iter()
        .map(|error| {
            let short_hash = commit_hash.chars().take(12).collect::<String>();
            format!("{short_hash} {title}: {error}")
        })
        .collect()
}

fn valid_conventional_title(title: &str) -> bool {
    let Some((prefix, description)) = title.split_once(": ") else {
        return false;
    };
    if description.trim().is_empty() {
        return false;
    }

    let prefix = prefix.strip_suffix('!').unwrap_or(prefix);
    let (commit_type, scope) = match prefix.split_once('(') {
        Some((commit_type, scope_with_suffix)) => {
            let Some(scope) = scope_with_suffix.strip_suffix(')') else {
                return false;
            };
            (commit_type, Some(scope))
        }
        None => (prefix, None),
    };

    ALLOWED_TYPES.contains(&commit_type) && scope.is_none_or(valid_scope)
}

fn valid_scope(scope: &str) -> bool {
    let mut chars = scope.chars();
    match chars.next() {
        Some(ch) if ch.is_ascii_lowercase() || ch.is_ascii_digit() => {}
        _ => return false,
    }

    chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-'))
}

fn has_non_empty_field(message: &str, field: &str) -> bool {
    message
        .lines()
        .any(|line| field_value(line, field).is_some_and(|value| !value.trim().is_empty()))
}

fn field_value<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    line.strip_prefix(field)?.strip_prefix(": ")
}

fn has_valid_slice(message: &str) -> bool {
    message
        .lines()
        .any(|line| field_value(line, "Slice").is_some_and(|slice| ALLOWED_SLICES.contains(&slice)))
}

fn has_valid_impact(message: &str) -> bool {
    message.lines().any(|line| {
        let Some(value) = field_value(line, "Impact") else {
            return false;
        };
        let parts = value.split("; ").collect::<Vec<_>>();
        parts.len() == 3
            && matches!(parts[0], "core-api=none" | "core-api=changed")
            && matches!(parts[1], "data-format=none" | "data-format=changed")
            && matches!(parts[2], "adapter-api=none" | "adapter-api=changed")
    })
}

fn has_valid_docs(message: &str) -> bool {
    message.lines().any(|line| {
        field_value(line, "Docs").is_some_and(|docs| {
            matches!(docs, "updated" | "not required")
                || docs
                    .strip_prefix("pending ")
                    .is_some_and(|reason| !reason.trim().is_empty())
        })
    })
}

fn has_refs_or_closes(message: &str) -> bool {
    footer_lines(message)
        .last()
        .is_some_and(|line| valid_refs_footer_line(line) || valid_closes_footer_line(line))
}

fn footer_lines(message: &str) -> Vec<&str> {
    let lines = message.lines().collect::<Vec<_>>();
    let Some(end) = lines.iter().rposition(|line| !line.trim().is_empty()) else {
        return Vec::new();
    };
    let start = lines[..end]
        .iter()
        .rposition(|line| line.trim().is_empty())
        .map_or(0, |index| index + 1);

    lines[start..=end].to_vec()
}

fn valid_refs_footer_line(line: &str) -> bool {
    line.strip_prefix("Refs: ")
        .is_some_and(|value| valid_issue_reference(value) || valid_pending_reason(value))
}

fn valid_closes_footer_line(line: &str) -> bool {
    line.strip_prefix("Closes: ")
        .is_some_and(valid_issue_reference)
}

fn valid_issue_reference(value: &str) -> bool {
    value
        .strip_prefix('#')
        .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()))
}

fn valid_pending_reason(value: &str) -> bool {
    value
        .strip_prefix("pending,")
        .is_some_and(|reason| !reason.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_MESSAGE: &str = "\
docs(governance): 对齐提交规范

Gate: G3 Pass
Slice: governance
Impact: core-api=none; data-format=none; adapter-api=none
Scope: 以 Conventional Commits 标题格式重写提交规范
Validation: cargo +1.96.0 test --workspace --locked
Docs: updated

Refs: #23
";

    #[test]
    fn accepts_lane_flow_commit_message() {
        assert!(validate_message("0123456789abcdef", VALID_MESSAGE).is_empty());
    }

    #[test]
    fn rejects_legacy_title_and_type_field() {
        let message = VALID_MESSAGE
            .replace("docs(governance): 对齐提交规范", "LF-23: 对齐提交规范")
            .replace("Slice: governance", "Type: governance");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("标题不符合")));
        assert!(errors.iter().any(|error| error.contains("`Slice`")));
    }

    #[test]
    fn rejects_required_field_without_space_after_colon() {
        let message = VALID_MESSAGE.replace("Gate: G3 Pass", "Gate:G3 Pass");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("Gate: ")));
    }

    #[test]
    fn rejects_slice_without_space_after_colon() {
        let message = VALID_MESSAGE.replace("Slice: governance", "Slice:governance");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("Slice")));
    }

    #[test]
    fn rejects_impact_without_separator_space() {
        let message = VALID_MESSAGE.replace(
            "Impact: core-api=none; data-format=none; adapter-api=none",
            "Impact: core-api=none;data-format=none; adapter-api=none",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("`Impact`")));
    }

    #[test]
    fn rejects_impact_without_space_after_colon() {
        let message = VALID_MESSAGE.replace(
            "Impact: core-api=none; data-format=none; adapter-api=none",
            "Impact:core-api=none; data-format=none; adapter-api=none",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("Impact")));
    }

    #[test]
    fn rejects_impact_fields_out_of_order() {
        let message = VALID_MESSAGE.replace(
            "Impact: core-api=none; data-format=none; adapter-api=none",
            "Impact: data-format=none; core-api=none; adapter-api=none",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("`Impact`")));
    }

    #[test]
    fn rejects_impact_with_missing_field() {
        let message = VALID_MESSAGE.replace(
            "Impact: core-api=none; data-format=none; adapter-api=none",
            "Impact: core-api=none; data-format=none",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("`Impact`")));
    }

    #[test]
    fn rejects_impact_with_extra_field() {
        let message = VALID_MESSAGE.replace(
            "Impact: core-api=none; data-format=none; adapter-api=none",
            "Impact: core-api=none; data-format=none; adapter-api=none; docs=changed",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("`Impact`")));
    }

    #[test]
    fn rejects_impact_with_invalid_value() {
        let message = VALID_MESSAGE.replace(
            "Impact: core-api=none; data-format=none; adapter-api=none",
            "Impact: core-api=maybe; data-format=none; adapter-api=none",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("`Impact`")));
    }

    #[test]
    fn accepts_docs_not_required() {
        let message = VALID_MESSAGE.replace("Docs: updated", "Docs: not required");

        assert!(validate_message("0123456789abcdef", &message).is_empty());
    }

    #[test]
    fn accepts_docs_pending_reason() {
        let message = VALID_MESSAGE.replace("Docs: updated", "Docs: pending 后续由 #25 跟踪补齐");

        assert!(validate_message("0123456789abcdef", &message).is_empty());
    }

    #[test]
    fn rejects_docs_pending_without_reason() {
        let message = VALID_MESSAGE.replace("Docs: updated", "Docs: pending");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("`Docs`")));
    }

    #[test]
    fn rejects_docs_unknown_value() {
        let message = VALID_MESSAGE.replace("Docs: updated", "Docs: maybe");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("`Docs`")));
    }

    #[test]
    fn rejects_non_numeric_issue_reference() {
        let message = VALID_MESSAGE.replace("Refs: #23", "Refs: #abc");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("Refs")));
    }

    #[test]
    fn accepts_pending_issue_reason() {
        let message = VALID_MESSAGE.replace(
            "Refs: #23",
            "Refs: pending, initial repository governance bootstrap",
        );

        assert!(validate_message("0123456789abcdef", &message).is_empty());
    }

    #[test]
    fn rejects_pending_without_reason() {
        let message = VALID_MESSAGE.replace("Refs: #23", "Refs: pending");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("Refs")));
    }

    #[test]
    fn accepts_closes_issue_reference() {
        let message = VALID_MESSAGE.replace("Refs: #23", "Closes: #23");

        assert!(validate_message("0123456789abcdef", &message).is_empty());
    }

    #[test]
    fn rejects_closes_pending_reason() {
        let message = VALID_MESSAGE.replace(
            "Refs: #23",
            "Closes: pending, initial repository governance bootstrap",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("Closes")));
    }

    #[test]
    fn rejects_issue_reference_outside_footer_block() {
        let message =
            VALID_MESSAGE.replace("Refs: #23\n", "Refs: #23\n\nNote: footer must stay last\n");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("Refs")));
    }

    #[test]
    fn rejects_issue_reference_followed_by_non_empty_footer_line() {
        let message =
            VALID_MESSAGE.replace("Refs: #23\n", "Refs: #23\nNote: footer must stay last\n");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("Refs")));
    }

    #[test]
    fn accepts_breaking_change_bang() {
        assert!(valid_conventional_title("feat(core)!: 调整 tick API"));
    }

    #[test]
    fn extracts_top_level_push_event_payload_fields() {
        let payload = r#"{
            "head_commit": {
                "message": "commit message mentions \"before\" and \"after\""
            },
            "commits": [
                {
                    "before": "nested-before",
                    "after": "nested-after"
                }
            ],
            "before": "abc",
            "after": "def"
        }"#;
        let event = push_event_payload(payload).expect("payload should parse");

        assert_eq!(event.before.as_deref(), Some("abc"));
        assert_eq!(event.after.as_deref(), Some("def"));
    }

    #[test]
    fn local_run_requires_explicit_commit_range() {
        let error = commit_range_from_event(None, None, None, None).unwrap_err();

        assert!(error.contains("显式传入 commit rev-range"));
    }

    #[test]
    fn unsupported_event_requires_explicit_commit_range() {
        let error = commit_range_from_event(Some("workflow_dispatch"), None, Some("def"), None)
            .unwrap_err();

        assert!(error.contains("显式传入 rev-range"));
    }

    #[test]
    fn pull_request_event_uses_base_branch_range() {
        let range =
            commit_range_from_event(Some("pull_request"), Some("main"), Some("def"), None).unwrap();

        assert_eq!(range, "origin/main..HEAD");
    }

    #[test]
    fn push_event_uses_before_after_range() {
        let payload = r#"{
            "head_commit": {
                "message": "commit message mentions \"before\" and \"after\""
            },
            "nested": {
                "before": "nested-before",
                "after": "nested-after"
            },
            "before": "abc",
            "after": "def"
        }"#;

        let range = commit_range_from_event(Some("push"), None, None, Some(payload)).unwrap();

        assert_eq!(range, "abc..def");
    }

    #[test]
    fn push_event_with_zero_before_checks_tip_commit_only() {
        let payload = r#"{"before":"0000000000000000000000000000000000000000","after":"def"}"#;

        let range = commit_range_from_event(Some("push"), None, None, Some(payload)).unwrap();

        assert_eq!(range, "def^!");
    }

    #[test]
    fn push_event_without_payload_checks_github_sha_only() {
        let range = commit_range_from_event(Some("push"), None, Some("def"), None).unwrap();

        assert_eq!(range, "def^!");
    }
}
