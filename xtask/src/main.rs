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

const REQUIRED_PREFIXES: &[&str] = &[
    "Gate:",
    "Slice:",
    "Impact:",
    "Scope:",
    "Validation:",
    "Docs:",
];

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
    match env::var("GITHUB_EVENT_NAME").ok().as_deref() {
        Some("pull_request") => {
            let base_ref = env::var("GITHUB_BASE_REF")
                .map_err(|_| "pull_request 事件缺少 GITHUB_BASE_REF".to_string())?;
            Ok(format!("origin/{base_ref}..HEAD"))
        }
        Some("push") => {
            let event = github_event_payload().unwrap_or_default();
            let after = json_string_value(&event, "after")
                .or_else(|| env::var("GITHUB_SHA").ok())
                .unwrap_or_else(|| "HEAD".to_string());

            match json_string_value(&event, "before") {
                Some(before) if !is_zero_oid(&before) => Ok(format!("{before}..{after}")),
                _ => Ok(after),
            }
        }
        _ => Ok("HEAD".to_string()),
    }
}

fn github_event_payload() -> Option<String> {
    let path = env::var("GITHUB_EVENT_PATH").ok()?;
    std::fs::read_to_string(path).ok()
}

fn json_string_value(payload: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{key}\"");
    let start = payload.find(&pattern)? + pattern.len();
    let after_key = payload[start..].trim_start();
    let after_colon = after_key.strip_prefix(':')?.trim_start();
    let value = after_colon.strip_prefix('"')?;
    let end = value.find('"')?;
    Some(value[..end].to_string())
}

fn is_zero_oid(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|ch| ch == '0')
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

    for prefix in REQUIRED_PREFIXES {
        if !has_non_empty_prefixed_line(message, prefix) {
            errors.push(format!("缺少 `{prefix}` 行"));
        }
    }

    if !has_valid_slice(message) {
        errors.push("`Slice` 缺失或不是支持的 LaneFlow 切片类型".to_string());
    }

    if !has_valid_impact(message) {
        errors.push("`Impact` 必须同时覆盖 core-api、data-format 和 adapter-api".to_string());
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

fn has_non_empty_prefixed_line(message: &str, prefix: &str) -> bool {
    message.lines().any(|line| {
        line.strip_prefix(prefix)
            .is_some_and(|value| !value.trim().is_empty())
    })
}

fn has_valid_slice(message: &str) -> bool {
    message.lines().any(|line| {
        line.strip_prefix("Slice: ")
            .is_some_and(|slice| ALLOWED_SLICES.contains(&slice))
    })
}

fn has_valid_impact(message: &str) -> bool {
    message.lines().any(|line| {
        let Some(value) = line.strip_prefix("Impact: ") else {
            return false;
        };
        let parts = value.split("; ").collect::<Vec<_>>();
        parts.len() == 3
            && matches!(parts[0], "core-api=none" | "core-api=changed")
            && matches!(parts[1], "data-format=none" | "data-format=changed")
            && matches!(parts[2], "adapter-api=none" | "adapter-api=changed")
    })
}

fn has_refs_or_closes(message: &str) -> bool {
    message.lines().any(|line| {
        line.strip_prefix("Refs: ")
            .or_else(|| line.strip_prefix("Closes: "))
            .is_some_and(|value| value.starts_with('#') || value.starts_with("pending"))
    })
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
    fn accepts_breaking_change_bang() {
        assert!(valid_conventional_title("feat(core)!: 调整 tick API"));
    }

    #[test]
    fn extracts_simple_json_string_value() {
        let payload = r#"{"before":"abc","after":"def"}"#;

        assert_eq!(
            json_string_value(payload, "before"),
            Some("abc".to_string())
        );
        assert_eq!(json_string_value(payload, "after"), Some("def".to_string()));
    }
}
