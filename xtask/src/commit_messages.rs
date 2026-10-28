//! 提交信息校验 Conventional Commits 标题、Refs/Closes、破坏性变更说明和成对的 Harness/Model。
//! Model 只接受已登记的规范 ID。trailer 身份由 `git interpret-trailers --parse --no-divider`
//! 确定；原文用于拒绝不规范写法。禁止使用 AI 工具署名 trailer。

use std::borrow::Cow;
use std::env;
use std::io::Write;
use std::process::Command;
use std::process::Stdio;

const ALLOWED_TYPES: &[&str] = &[
    "feat", "fix", "docs", "test", "refactor", "perf", "build", "ci", "chore", "revert",
];

const DEPENDABOT_AUTHOR_NAME: &str = "dependabot[bot]";
const DEPENDABOT_AUTHOR_EMAIL: &str = "49699333+dependabot[bot]@users.noreply.github.com";

// 显示名与邮箱 local-part 按 ASCII 词边界匹配；不匹配域名，避免误伤 `@openai.com` 等人类地址（#933）。
const AI_TOOL_COAUTHOR_TOKENS: &[&str] = &["cursor", "claude", "codex", "copilot"];
const AI_TOOL_COAUTHOR_EMAILS: &[&str] = &[
    "noreply@anthropic.com",
    "noreply@openai.com",
    "cursoragent@cursor.com",
];
const AI_COAUTHOR_REWRITE_HINT: &str = "`Co-authored-by` 不得标注 AI 工具或模型。请删除该行，并在空行后的 trailer 段成对写入 `Harness: <执行框架>` 和 `Model: <实际模型>`";
const AI_MADE_WITH_REWRITE_HINT: &str = "`Made-with:` 不得标注 AI 工具。请删除该行，并在空行后的 trailer 段成对写入 `Harness: <执行框架>` 和 `Model: <实际模型>`";

/// 读取并校验单个 commit message 文件（commit-msg 钩子入口）：归一化换行、剥离 Git 注释行后按合同校验。
pub(crate) fn check_commit_message_file(path: &str) -> Result<(), String> {
    let message = std::fs::read_to_string(path)
        .map_err(|err| format!("无法读取 commit message 文件 `{path}`: {err}"))?;
    let message = normalize_commit_message_line_endings(&message);
    let message = strip_commit_message_comments(message.as_ref());
    let errors = validate_message("commit-msg", &message);

    if errors.is_empty() {
        println!("已校验 commit message 文件: {path}");
        Ok(())
    } else {
        Err(format_errors(errors))
    }
}

fn normalize_commit_message_line_endings(message: &str) -> Cow<'_, str> {
    if !message.as_bytes().contains(&b'\r') {
        return Cow::Borrowed(message);
    }

    Cow::Owned(message.replace("\r\n", "\n").replace('\r', "\n"))
}

fn strip_commit_message_comments(message: &str) -> String {
    message
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 校验提交范围内全部非 merge commit 的 message；范围取显式 rev-range，缺省时从 CI 的 GitHub event 环境变量推导。
pub(crate) fn check_commit_messages(explicit_range: Option<&str>) -> Result<(), String> {
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
        let author_name = git(["log", "-1", "--format=%an", commit_hash])?;
        let author_email = git(["log", "-1", "--format=%ae", commit_hash])?;
        errors.extend(commit_message_errors(
            commit_hash,
            author_name.trim(),
            author_email.trim(),
            &message,
        ));
    }

    if errors.is_empty() {
        println!(
            "已校验 {} 个 commit message，范围: {}",
            commits.len(),
            commit_range
        );
        Ok(())
    } else {
        Err(format_errors(errors))
    }
}

fn format_errors(errors: Vec<String>) -> String {
    let mut output = String::from("Commit message 校验失败:");
    for error in errors {
        output.push_str("\n- ");
        output.push_str(&error);
    }
    output
}

fn is_allowed_dependabot_commit(author_name: &str, author_email: &str, message: &str) -> bool {
    if author_name != DEPENDABOT_AUTHOR_NAME || author_email != DEPENDABOT_AUTHOR_EMAIL {
        return false;
    }

    let message = normalize_commit_message_line_endings(message);
    let title = message.lines().next().unwrap_or_default();
    title.starts_with("build(deps): ")
        && valid_conventional_title(title)
        && !title_has_breaking_bang(title)
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
        Some("merge_group") => {
            let event = event_payload
                .and_then(merge_group_event_payload)
                .ok_or("merge_group 事件缺少合法的 GITHUB_EVENT_PATH payload")?;
            let base_sha = event
                .merge_group
                .base_sha
                .as_deref()
                .filter(|value| is_non_zero_value(value))
                .ok_or("merge_group 事件缺少非零 merge_group.base_sha")?;
            let head_sha = event
                .merge_group
                .head_sha
                .as_deref()
                .filter(|value| is_non_zero_value(value))
                .ok_or("merge_group 事件缺少非零 merge_group.head_sha")?;

            if let Some(github_sha) =
                non_empty_value(github_sha).filter(|value| is_non_zero_value(value))
                && github_sha != head_sha
            {
                return Err(format!(
                    "merge_group.head_sha `{head_sha}` 与 GITHUB_SHA `{github_sha}` 不一致"
                ));
            }

            Ok(format!("{base_sha}..{head_sha}"))
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
            "不支持从 GitHub event `{event_name}` 自动推导 commit range，请显式传入 rev-range，例如: cargo +1.99.1 run --locked -p xtask -- check-commit-messages origin/main..HEAD"
        )),
        None => Err(
            "非 CI 场景必须显式传入 commit rev-range，例如: cargo +1.99.1 run --locked -p xtask -- check-commit-messages origin/main..HEAD"
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

#[derive(serde::Deserialize)]
struct MergeGroupEventPayload {
    merge_group: MergeGroupPayload,
}

#[derive(serde::Deserialize)]
struct MergeGroupPayload {
    base_sha: Option<String>,
    head_sha: Option<String>,
}

fn push_event_payload(payload: &str) -> Option<PushEventPayload> {
    serde_json::from_str(payload).ok()
}

fn merge_group_event_payload(payload: &str) -> Option<MergeGroupEventPayload> {
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

fn commit_message_errors(
    commit_hash: &str,
    author_name: &str,
    author_email: &str,
    message: &str,
) -> Vec<String> {
    let exempt_issue_footer = is_allowed_dependabot_commit(author_name, author_email, message);
    validate_message_with_footer_policy(commit_hash, message, exempt_issue_footer)
}

fn validate_message(commit_hash: &str, message: &str) -> Vec<String> {
    validate_message_with_footer_policy(commit_hash, message, false)
}

fn validate_message_with_footer_policy(
    commit_hash: &str,
    message: &str,
    exempt_issue_footer: bool,
) -> Vec<String> {
    let message = normalize_commit_message_line_endings(message);
    let message = message.as_ref();
    let title = message.lines().next().unwrap_or_default();
    let mut errors = Vec::new();
    let has_breaking_bang = title_has_breaking_bang(title);
    let breaking_change_footer_count = breaking_change_footer_count(message);
    let has_breaking_change_footer = breaking_change_footer_count > 0;

    if !valid_conventional_title(title) {
        errors.push("标题不符合 Conventional Commits".to_string());
    }

    if has_breaking_bang && !has_breaking_change_footer {
        errors.push("标题包含 `!` 时必须提供 `BREAKING CHANGE: ` footer".to_string());
    }

    if has_breaking_change_footer && !has_breaking_bang {
        errors.push("`BREAKING CHANGE: ` footer 必须与标题 `!` 同时使用".to_string());
    }

    let trailers = match git_parsed_trailers(message) {
        Ok(trailers) => trailers,
        Err(error) => {
            errors.push(error);
            return prefix_commit_errors(commit_hash, title, errors);
        }
    };

    if has_breaking_change_footer && !has_single_valid_breaking_change_footer(message, &trailers) {
        errors.push(
            "`BREAKING CHANGE: ` footer 格式不正确，必须在 Git trailer 段之前提供单行非空说明，并与该段空一行"
                .to_string(),
        );
    }

    errors.extend(issue_footer_errors(message, &trailers, exempt_issue_footer));
    errors.extend(canonical_trailer_errors(message, &trailers));
    errors.extend(unrecognized_controlled_trailers(message, &trailers));
    errors.extend(ai_attribution_errors_from_git(&trailers));

    prefix_commit_errors(commit_hash, title, errors)
}

fn prefix_commit_errors(commit_hash: &str, title: &str, errors: Vec<String>) -> Vec<String> {
    let short_hash = commit_hash.chars().take(12).collect::<String>();
    errors
        .into_iter()
        .map(|error| format!("{short_hash} {title}: {error}"))
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

fn title_has_breaking_bang(title: &str) -> bool {
    title
        .split_once(": ")
        .is_some_and(|(prefix, _description)| prefix.ends_with('!'))
}

fn valid_scope(scope: &str) -> bool {
    let mut chars = scope.chars();
    match chars.next() {
        Some(ch) if ch.is_ascii_lowercase() || ch.is_ascii_digit() => {}
        _ => return false,
    }

    chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-'))
}

fn valid_breaking_change_footer_line(line: &str) -> bool {
    line.strip_prefix("BREAKING CHANGE: ")
        .is_some_and(|description| !description.trim().is_empty())
}

fn breaking_change_footer_count(message: &str) -> usize {
    message
        .lines()
        .filter(|line| line.starts_with("BREAKING CHANGE:"))
        .count()
}

struct GitTrailer {
    key: String,
    value: String,
}

fn has_single_valid_breaking_change_footer(message: &str, trailers: &[GitTrailer]) -> bool {
    let lines = message.lines().collect::<Vec<_>>();
    let Some(last_non_empty_index) = lines.iter().rposition(|line| !line.trim().is_empty()) else {
        return false;
    };
    let Some(breaking_index) = lines
        .iter()
        .position(|line| line.starts_with("BREAKING CHANGE:"))
    else {
        return false;
    };

    breaking_change_footer_count(message) == 1
        && valid_breaking_change_footer_line(lines[breaking_index])
        && breaking_index < last_non_empty_index
        && git_trailer_block_follows_breaking(
            &lines,
            breaking_index,
            last_non_empty_index,
            trailers,
        )
}

fn git_trailer_block_follows_breaking(
    lines: &[&str],
    breaking_index: usize,
    last_index: usize,
    trailers: &[GitTrailer],
) -> bool {
    if trailers.is_empty() || breaking_index >= last_index {
        return false;
    }
    let mut block_start = last_index;
    while block_start > 0 && !lines[block_start - 1].trim().is_empty() {
        block_start -= 1;
    }
    if block_start <= breaking_index
        || !lines[breaking_index + 1..block_start]
            .iter()
            .all(|line| line.trim().is_empty())
    {
        return false;
    }
    let paragraph = lines[block_start..=last_index]
        .iter()
        .copied()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    let rendered = trailers
        .iter()
        .map(|trailer| format!("{}: {}", trailer.key, trailer.value))
        .collect::<Vec<_>>();
    paragraph.len() == rendered.len()
        && paragraph
            .iter()
            .zip(&rendered)
            .all(|(line, rendered)| *line == rendered)
}

fn git_parsed_trailers(message: &str) -> Result<Vec<GitTrailer>, String> {
    let mut command = Command::new("git");
    command
        .args([
            "-c",
            "trailer.separators=:",
            "interpret-trailers",
            "--parse",
            "--no-divider",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    isolate_git_config(&mut command);
    let mut child = command
        .spawn()
        .map_err(|err| format!("无法运行 git interpret-trailers: {err}"))?;
    child
        .stdin
        .take()
        .ok_or("git interpret-trailers 缺少 stdin")?
        .write_all(message.as_bytes())
        .map_err(|err| format!("无法把 commit message 传给 git interpret-trailers: {err}"))?;
    let output = child
        .wait_with_output()
        .map_err(|err| format!("git interpret-trailers 失败: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git interpret-trailers 失败: {}", stderr.trim()));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|err| format!("git interpret-trailers 输出不是 UTF-8: {err}"))?;
    Ok(stdout.lines().filter_map(parse_git_trailer_line).collect())
}

fn isolate_git_config(command: &mut Command) {
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", null_config_path())
        .env("GIT_CONFIG_SYSTEM", null_config_path())
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_CONFIG_PARAMETERS");
    for key in std::env::vars().map(|(key, _)| key) {
        if key.starts_with("GIT_CONFIG_KEY_") || key.starts_with("GIT_CONFIG_VALUE_") {
            command.env_remove(key);
        }
    }
}

fn null_config_path() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

fn parse_git_trailer_line(line: &str) -> Option<GitTrailer> {
    let (key, value) = line.split_once(':')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    Some(GitTrailer {
        key: key.to_string(),
        value: value.trim().to_string(),
    })
}

fn rendered_trailer(trailer: &GitTrailer) -> String {
    format!("{}: {}", trailer.key, trailer.value)
}

fn issue_footer_errors(
    message: &str,
    trailers: &[GitTrailer],
    exempt_missing: bool,
) -> Vec<String> {
    let mut errors = Vec::new();
    let last_line = message.lines().rev().find(|line| !line.trim().is_empty());
    let issue_position = trailers
        .iter()
        .rposition(|trailer| is_issue_trailer_key(&trailer.key));
    let Some(issue_position) = issue_position else {
        errors.extend(missing_issue_footer_errors(
            message,
            trailers,
            exempt_missing,
        ));
        return errors;
    };
    if issue_position + 1 != trailers.len() {
        errors.push("`Refs:` / `Closes:` footer 必须是提交信息最后一个非空行".to_string());
    }
    let trailer = &trailers[issue_position];
    if !valid_parsed_issue_footer(trailer) {
        errors.push(
            "`Refs:` / `Closes:` footer 格式不正确，应使用 `Refs: #<id>`、`Refs: pending, <reason>` 或 `Closes: #<id>`"
                .to_string(),
        );
    }
    let rendered = rendered_trailer(trailer);
    if message.lines().any(|line| line == rendered) && last_line != Some(rendered.as_str()) {
        errors.push("`Refs:` / `Closes:` footer 必须是提交信息最后一个非空行".to_string());
    }
    errors
}

fn is_issue_trailer_key(key: &str) -> bool {
    key == "Refs" || key == "Closes"
}

fn valid_parsed_issue_footer(trailer: &GitTrailer) -> bool {
    match trailer.key.as_str() {
        "Refs" => valid_issue_reference(&trailer.value) || valid_pending_reason(&trailer.value),
        "Closes" => valid_issue_reference(&trailer.value),
        _ => false,
    }
}

fn missing_issue_footer_errors(
    message: &str,
    trailers: &[GitTrailer],
    exempt_missing: bool,
) -> Vec<String> {
    let mut errors = Vec::new();
    let git_saw_issue_key = trailers.iter().any(|trailer| {
        trailer.key.eq_ignore_ascii_case("Refs") || trailer.key.eq_ignore_ascii_case("Closes")
    });
    let candidates = raw_keyed_lines(message)
        .into_iter()
        .filter(|line| is_issue_trailer_key_loose(&line.key))
        .collect::<Vec<_>>();
    if candidates.is_empty() && !git_saw_issue_key {
        if !exempt_missing {
            errors.push("缺少 `Refs:` 或 `Closes:` footer".to_string());
            if message.contains("Refs") || message.contains("Closes") || message.contains("Harness")
            {
                errors.push(unparsed_trailer_hint());
            }
        }
        return errors;
    }
    if git_saw_issue_key {
        return errors;
    }
    if candidate_footer_is_not_last(message) {
        errors.push("`Refs:` / `Closes:` footer 必须是提交信息最后一个非空行".to_string());
    }
    if candidates
        .iter()
        .any(|line| !candidate_footer_value_is_valid(line))
    {
        errors.push(
            "`Refs:` / `Closes:` footer 格式不正确，应使用 `Refs: #<id>`、`Refs: pending, <reason>` 或 `Closes: #<id>`"
                .to_string(),
        );
    }
    errors.push(unparsed_trailer_hint());
    errors
}

fn unparsed_trailer_hint() -> String {
    "Git 没有把提交末尾认成 trailer 段。trailer 段之前必须有空行，行首不能缩进，取值不能折行，`BREAKING CHANGE:` 不能写进同一段。".to_string()
}

fn candidate_footer_is_not_last(message: &str) -> bool {
    let lines = message
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    let Some((last, earlier)) = lines.split_last() else {
        return false;
    };
    earlier
        .iter()
        .any(|line| is_issue_footer_candidate_line(line))
        && !is_issue_footer_candidate_line(last)
}

fn is_issue_footer_candidate_line(line: &str) -> bool {
    raw_keyed_line(line).is_some_and(|keyed| is_issue_trailer_key_loose(&keyed.key))
}

fn is_issue_trailer_key_loose(key: &str) -> bool {
    key.eq_ignore_ascii_case("Refs") || key.eq_ignore_ascii_case("Closes")
}

fn candidate_footer_value_is_valid(line: &RawKeyedLine) -> bool {
    if line.key.eq_ignore_ascii_case("Refs") {
        valid_issue_reference(&line.value) || valid_pending_reason(&line.value)
    } else {
        valid_issue_reference(&line.value)
    }
}

fn canonical_trailer_errors(message: &str, trailers: &[GitTrailer]) -> Vec<String> {
    let mut errors = Vec::new();
    let lines = message.lines().collect::<Vec<_>>();
    for trailer in trailers {
        if let Some(expected) = required_trailer_key(&trailer.key)
            && trailer.key != expected
        {
            errors.push(format!("`{expected}:` 键名大小写必须精确"));
            continue;
        }
        let rendered = rendered_trailer(trailer);
        if !lines.contains(&rendered.as_str()) {
            errors.push(format!(
                "`{}` 的写法不规范。请写成单行 `{rendered}`：冒号必须紧跟键名，行首不能缩进，取值不能折行",
                trailer.key
            ));
        }
    }
    errors
}

fn required_trailer_key(key: &str) -> Option<&'static str> {
    match key.to_ascii_lowercase().as_str() {
        "harness" => Some("Harness"),
        "model" => Some("Model"),
        "refs" => Some("Refs"),
        "closes" => Some("Closes"),
        _ => None,
    }
}

struct RawKeyedLine {
    key: String,
    value: String,
}

fn raw_keyed_lines(message: &str) -> Vec<RawKeyedLine> {
    message.lines().filter_map(raw_keyed_line).collect()
}

fn raw_keyed_line(line: &str) -> Option<RawKeyedLine> {
    let (token, value) = line.trim().split_once(':')?;
    let token = token.trim();
    let key = controlled_key(token)?;
    Some(RawKeyedLine {
        key: key.to_string(),
        value: value.trim().to_string(),
    })
}

fn controlled_key(token: &str) -> Option<&'static str> {
    [
        "Harness",
        "Model",
        "Co-authored-by",
        "Made-with",
        "Refs",
        "Closes",
    ]
    .into_iter()
    .find(|key| token.eq_ignore_ascii_case(key))
}

fn unrecognized_controlled_trailers(message: &str, trailers: &[GitTrailer]) -> Vec<String> {
    let mut rendered = trailers.iter().map(rendered_trailer).collect::<Vec<_>>();
    let mut errors = Vec::new();
    let mut saw_ai_coauthor = false;
    let mut saw_ai_made_with = false;
    for line in message.lines().rev() {
        let Some(keyed) = raw_keyed_line(line) else {
            continue;
        };
        if !is_attribution_key(&keyed.key) {
            continue;
        }
        if let Some(position) = rendered.iter().rposition(|item| item == line) {
            rendered.remove(position);
            continue;
        }
        if keyed.key == "Co-authored-by" && is_ai_tool_coauthor(&keyed.value) {
            saw_ai_coauthor = true;
        } else if keyed.key == "Made-with" && contains_ai_tool_token(&keyed.value) {
            saw_ai_made_with = true;
        }
        errors.push(format!(
            "`{}:` 没有被 Git 认成 trailer。它必须位于末尾 trailer 段：段前必须有空行，中间不能空行，行首不能缩进，取值不能折行",
            keyed.key
        ));
    }
    if saw_ai_coauthor {
        errors.push(AI_COAUTHOR_REWRITE_HINT.to_string());
    }
    if saw_ai_made_with {
        errors.push(AI_MADE_WITH_REWRITE_HINT.to_string());
    }
    errors
}

fn is_attribution_key(key: &str) -> bool {
    matches!(key, "Harness" | "Model" | "Co-authored-by" | "Made-with")
}

fn ai_attribution_errors_from_git(trailers: &[GitTrailer]) -> Vec<String> {
    let mut errors = Vec::new();
    if trailers.iter().any(|trailer| {
        trailer.key.eq_ignore_ascii_case("Co-authored-by") && is_ai_tool_coauthor(&trailer.value)
    }) {
        errors.push(AI_COAUTHOR_REWRITE_HINT.to_string());
    }
    if trailers.iter().any(|trailer| {
        trailer.key.eq_ignore_ascii_case("Made-with") && contains_ai_tool_token(&trailer.value)
    }) {
        errors.push(AI_MADE_WITH_REWRITE_HINT.to_string());
    }
    errors.extend(harness_model_errors_from_git(trailers));
    errors
}

fn harness_model_errors_from_git(trailers: &[GitTrailer]) -> Vec<String> {
    let harness = trailer_positions(trailers, "Harness");
    let model = trailer_positions(trailers, "Model");
    let mut errors = Vec::new();
    if harness.len() > 1 {
        errors.push("`Harness:` 只能出现一次".to_string());
    }
    if model.len() > 1 {
        errors.push("`Model:` 只能出现一次".to_string());
    }
    if harness.is_empty() != model.is_empty() {
        errors.push("`Harness:` 与 `Model:` 必须成对出现".to_string());
    }
    if harness.len() == 1 && model.len() == 1 && model[0] != harness[0] + 1 {
        errors.push("`Harness:` 必须紧邻写在 `Model:` 的前一行".to_string());
    }
    for (key, positions) in [("Harness", &harness), ("Model", &model)] {
        if positions
            .iter()
            .any(|index| trailers[*index].value.is_empty())
        {
            errors.push(format!("`{key}:` 必须写成 `{key}: <非空取值>`"));
        }
    }
    if let [position] = model.as_slice()
        && !trailers[*position].value.is_empty()
        && let Err(error) = crate::commit_models::validate_model_id(&trailers[*position].value)
    {
        errors.push(error);
    }
    if let Some(footer) = trailers
        .iter()
        .rposition(|trailer| is_issue_trailer_key(&trailer.key))
        && harness
            .iter()
            .chain(model.iter())
            .any(|index| *index > footer)
    {
        errors.push("`Harness:` / `Model:` 必须位于 `Refs:` / `Closes:` 之前".to_string());
    }
    errors
}

fn trailer_positions(trailers: &[GitTrailer], key: &str) -> Vec<usize> {
    trailers
        .iter()
        .enumerate()
        .filter(|(_, trailer)| trailer.key.eq_ignore_ascii_case(key))
        .map(|(index, _)| index)
        .collect()
}

fn valid_issue_reference(value: &str) -> bool {
    value
        .strip_prefix('#')
        .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()))
}

fn valid_pending_reason(value: &str) -> bool {
    value
        .strip_prefix("pending, ")
        .is_some_and(|reason| !reason.trim().is_empty())
}

fn is_ai_tool_coauthor(value: &str) -> bool {
    let (name, email) = split_coauthor(value);
    if let Some(email) = email
        && (AI_TOOL_COAUTHOR_EMAILS
            .iter()
            .any(|candidate| email.eq_ignore_ascii_case(candidate))
            || contains_ai_tool_token(email_local_part(email)))
    {
        return true;
    }
    contains_ai_tool_token(name)
}

fn split_coauthor(value: &str) -> (&str, Option<&str>) {
    let value = value.trim();
    if let Some((name, rest)) = value.split_once('<') {
        let email = match rest.split_once('>') {
            Some((email, _)) => email.trim(),
            None => rest.trim(),
        };
        let email = if email.is_empty() { None } else { Some(email) };
        return (name.trim(), email);
    }
    if value.contains('@') && !value.contains(char::is_whitespace) {
        return ("", Some(value));
    }
    (value, None)
}

fn email_local_part(email: &str) -> &str {
    email
        .split_once('@')
        .map(|(local, _)| local)
        .unwrap_or(email)
}

fn contains_ai_tool_token(value: &str) -> bool {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .any(|token| {
            AI_TOOL_COAUTHOR_TOKENS
                .iter()
                .any(|tool| token.eq_ignore_ascii_case(tool))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_MESSAGE: &str = "\
docs(governance): 对齐提交规范

Refs: #23
";

    const LEGACY_G3_MESSAGE: &str = "\
docs(governance): 对齐提交规范

Gate: G3 Candidate
Slice: governance
Impact: core-api=none; data-format=none; adapter-api=none
Scope: 以 Conventional Commits 标题格式重写提交规范
Validation: cargo +1.99.1 test --workspace --locked
Docs: updated

Refs: #23
";

    const BREAKING_MESSAGE: &str = "\
feat(runtime)!: 调整 tick API

BREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填，调用方必须显式传入 tick 间隔。

Refs: #12
";

    #[test]
    fn accepts_conventional_title_and_refs_footer() {
        assert!(validate_message("0123456789abcdef", VALID_MESSAGE).is_empty());
    }

    #[test]
    fn ignores_legacy_g3_body_fields() {
        assert!(validate_message("0123456789abcdef", LEGACY_G3_MESSAGE).is_empty());
    }

    #[test]
    fn accepts_commit_message_with_crlf_line_endings() {
        let message = VALID_MESSAGE.replace('\n', "\r\n");

        assert!(validate_message("0123456789abcdef", &message).is_empty());
    }

    #[test]
    fn accepts_commit_message_with_lone_cr_line_endings() {
        let message = VALID_MESSAGE.replace('\n', "\r");

        assert!(validate_message("0123456789abcdef", &message).is_empty());
    }

    #[test]
    fn accepts_breaking_change_with_bang_and_footer() {
        assert!(validate_message("0123456789abcdef", BREAKING_MESSAGE).is_empty());
    }

    #[test]
    fn rejects_breaking_bang_without_breaking_change_footer() {
        let message = BREAKING_MESSAGE.replace(
            "\nBREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填，调用方必须显式传入 tick 间隔。",
            "",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("必须提供")));
    }

    #[test]
    fn rejects_breaking_change_footer_without_bang() {
        let message = BREAKING_MESSAGE.replace("feat(runtime)!:", "feat(runtime):");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(
            errors
                .iter()
                .any(|error| error.contains("必须与标题 `!` 同时使用"))
        );
    }

    #[test]
    fn rejects_empty_breaking_change_footer() {
        let message = BREAKING_MESSAGE.replace(
            "BREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填，调用方必须显式传入 tick 间隔。",
            "BREAKING CHANGE: ",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("格式不正确")));
    }

    #[test]
    fn rejects_breaking_change_after_issue_footer() {
        let message = BREAKING_MESSAGE.replace(
            "BREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填，调用方必须显式传入 tick 间隔。\n\nRefs: #12",
            "Refs: #12\nBREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填，调用方必须显式传入 tick 间隔。",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("最后一个非空行")));
    }

    #[test]
    fn rejects_multiple_breaking_change_footers() {
        let message = BREAKING_MESSAGE.replace(
            "BREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填，调用方必须显式传入 tick 间隔。",
            "BREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填，调用方必须显式传入 tick 间隔。\nBREAKING CHANGE: 第二条破坏性说明",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("格式不正确")));
    }

    #[test]
    fn accepts_valid_commit_message_file() {
        let path = temp_message_path("valid");
        std::fs::write(&path, VALID_MESSAGE).expect("test commit message should be written");

        let result = check_commit_message_file(path.to_str().expect("path should be UTF-8"));

        std::fs::remove_file(&path).expect("test commit message should be removed");
        assert!(result.is_ok());
    }

    #[test]
    fn accepts_valid_commit_message_file_with_git_comments() {
        let path = temp_message_path("valid-with-comments");
        let message = format!(
            "{VALID_MESSAGE}\n# Please enter the commit message for your changes.\n  # On branch docs/23-conventional-commits\n# Changes to be committed:\n"
        );
        std::fs::write(&path, message).expect("test commit message should be written");

        let result = check_commit_message_file(path.to_str().expect("path should be UTF-8"));

        std::fs::remove_file(&path).expect("test commit message should be removed");
        assert!(result.is_ok());
    }

    #[test]
    fn rejects_non_comment_line_after_issue_footer_in_commit_message_file() {
        let path = temp_message_path("invalid-after-footer");
        let message = VALID_MESSAGE.replace(
            "Refs: #23\n",
            "Refs: #23\nnot a Git comment\n# This comment should be ignored\n",
        );
        std::fs::write(&path, message).expect("test commit message should be written");

        let result = check_commit_message_file(path.to_str().expect("path should be UTF-8"));

        std::fs::remove_file(&path).expect("test commit message should be removed");
        let error = result.expect_err("non-comment content after issue footer should fail");
        assert!(error.contains("最后一个非空行"));
    }

    #[test]
    fn rejects_invalid_commit_message_file() {
        let path = temp_message_path("invalid");
        std::fs::write(&path, "update files\n").expect("test commit message should be written");

        let result = check_commit_message_file(path.to_str().expect("path should be UTF-8"));

        std::fs::remove_file(&path).expect("test commit message should be removed");
        let error = result.expect_err("invalid commit message should fail");
        assert!(error.contains("标题不符合 Conventional Commits"));
    }

    #[test]
    fn rejects_legacy_title() {
        let message =
            VALID_MESSAGE.replace("docs(governance): 对齐提交规范", "LF-23: 对齐提交规范");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("标题不符合")));
    }

    #[test]
    fn accepts_current_component_scopes_and_omitted_scope() {
        for title in [
            "feat(runtime): 实现 fixed-step tick",
            "refactor(static-network): 统一共享路网索引",
            "feat(compiler): 接入新的编译遍",
            "docs(design): 冻结运行时契约",
            "docs(governance): 对齐提交规范",
            "chore: 清理维护脚本",
        ] {
            assert!(valid_conventional_title(title), "should accept `{title}`");
        }
    }

    #[test]
    fn rejects_invalid_scope_syntax() {
        for title in [
            "feat(): scope 不能为空",
            "feat(Runtime): scope 不能包含大写字母",
            "feat(_runtime): scope 不能以下划线开头",
            "feat(runtime/ui): scope 不能包含斜杠",
        ] {
            assert!(!valid_conventional_title(title), "should reject `{title}`");
        }
    }

    #[test]
    fn rejects_non_numeric_issue_reference() {
        let message = VALID_MESSAGE.replace("Refs: #23", "Refs: #abc");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("格式不正确")));
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

        assert!(errors.iter().any(|error| error.contains("格式不正确")));
    }

    #[test]
    fn rejects_pending_without_space_after_comma() {
        let message = VALID_MESSAGE.replace("Refs: #23", "Refs: pending,missing space");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("格式不正确")));
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

        assert!(errors.iter().any(|error| error.contains("格式不正确")));
    }

    #[test]
    fn rejects_missing_issue_footer() {
        let message = VALID_MESSAGE.replace("\nRefs: #23\n", "\n");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("缺少 `Refs:`")));
    }

    #[test]
    fn rejects_issue_reference_outside_footer_block() {
        let message =
            VALID_MESSAGE.replace("Refs: #23\n", "Refs: #23\n\nNote: footer must stay last\n");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("最后一个非空行")));
    }

    #[test]
    fn accepts_breaking_change_bang() {
        assert!(valid_conventional_title("feat(runtime)!: 调整 tick API"));
    }

    #[test]
    fn accepts_dependabot_dependency_commit_without_refs_footer() {
        assert!(is_allowed_dependabot_commit(
            DEPENDABOT_AUTHOR_NAME,
            DEPENDABOT_AUTHOR_EMAIL,
            "build(deps): bump serde from 1.0.227 to 1.0.228\n"
        ));
    }

    #[test]
    fn rejects_human_commit_that_mimics_dependabot_title() {
        assert!(!is_allowed_dependabot_commit(
            "LaneFlow Maintainer",
            "maintainer@example.com",
            "build(deps): bump serde from 1.0.227 to 1.0.228\n"
        ));
    }

    #[test]
    fn rejects_dependabot_commit_outside_dependency_scope() {
        assert!(!is_allowed_dependabot_commit(
            DEPENDABOT_AUTHOR_NAME,
            DEPENDABOT_AUTHOR_EMAIL,
            "fix(runtime): change runtime behavior\n"
        ));
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
    fn extracts_nested_merge_group_payload_fields() {
        let payload = r#"{
            "base_sha": "outer-base",
            "head_sha": "outer-head",
            "merge_group": {
                "base_sha": "abc",
                "head_sha": "def"
            }
        }"#;
        let event = merge_group_event_payload(payload).expect("payload should parse");

        assert_eq!(event.merge_group.base_sha.as_deref(), Some("abc"));
        assert_eq!(event.merge_group.head_sha.as_deref(), Some("def"));
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
    fn merge_group_event_uses_payload_range() {
        let payload = r#"{
            "merge_group": {
                "base_sha": "abc",
                "head_sha": "def"
            }
        }"#;

        let range =
            commit_range_from_event(Some("merge_group"), None, Some("def"), Some(payload)).unwrap();

        assert_eq!(range, "abc..def");
    }

    #[test]
    fn merge_group_event_requires_payload() {
        let error =
            commit_range_from_event(Some("merge_group"), None, Some("def"), None).unwrap_err();

        assert!(error.contains("GITHUB_EVENT_PATH payload"));
    }

    #[test]
    fn merge_group_event_rejects_zero_base_sha() {
        let payload = r#"{
            "merge_group": {
                "base_sha": "0000000000000000000000000000000000000000",
                "head_sha": "def"
            }
        }"#;

        let error = commit_range_from_event(Some("merge_group"), None, Some("def"), Some(payload))
            .unwrap_err();

        assert!(error.contains("merge_group.base_sha"));
    }

    #[test]
    fn merge_group_event_rejects_github_sha_mismatch() {
        let payload = r#"{
            "merge_group": {
                "base_sha": "abc",
                "head_sha": "def"
            }
        }"#;

        let error =
            commit_range_from_event(Some("merge_group"), None, Some("other"), Some(payload))
                .unwrap_err();

        assert!(error.contains("与 GITHUB_SHA `other` 不一致"));
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

    fn message_with_body(body: &str) -> String {
        format!("docs(governance): 对齐提交规范\n\n{body}\nRefs: #23\n")
    }

    fn assert_accepted(message: &str) {
        let errors = validate_message("0123456789abcdef", message);
        assert!(errors.is_empty(), "{message:?} => {errors:?}");
    }

    fn assert_rejected(message: &str, needle: &str) {
        let errors = validate_message("0123456789abcdef", message);
        assert!(
            errors.iter().any(|error| error.contains(needle)),
            "{message:?} => {errors:?}, missing {needle:?}"
        );
    }

    #[test]
    fn accepts_human_coauthors_and_paired_ai_trailers() {
        for message in [
            message_with_body("Co-authored-by: Ada Lovelace <ada@example.com>"),
            message_with_body("Co-Authored-By: Ada Lovelace <ada@example.com>"),
            message_with_body("Co-authored-by: Claudette Morel <claudette@example.com>"),
            message_with_body("Co-authored-by: Researcher <researcher@openai.com>"),
            message_with_body("Co-authored-by: Ada Lovelace <ada@cursor.com>"),
            message_with_body("Harness: Cursor\nModel: openai/gpt-6.1-sol"),
            message_with_body(
                "Co-authored-by: Ada Lovelace <ada@example.com>\nHarness: Claude Code\nModel: anthropic/claude-opus-5-5",
            ),
            "feat(runtime)!: 调整 tick API\n\nBREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填，调用方必须显式传入 tick 间隔。\n\nHarness: Claude Code\nModel: anthropic/claude-opus-5-5\nRefs: #12\n".to_string(),
            VALID_MESSAGE.to_string(),
        ] {
            assert_accepted(&message);
        }
    }

    #[test]
    fn rejects_noncanonical_model_ids_in_commit_messages() {
        for (value, hint) in [
            ("claude-opus-5.5", "anthropic/claude-opus-5-5"),
            ("OPENAI/GPT-6.1-SOL", "openai/gpt-6.1-sol"),
            ("kimi-code/k3", "moonshotai/kimi-k3"),
            ("GPT-6", "未登记为规范 ID"),
            ("lab/unregistered-model", "未登记为规范 ID"),
        ] {
            assert_rejected(
                &message_with_body(&format!("Harness: Codex\nModel: {value}")),
                hint,
            );
        }
    }

    #[test]
    fn model_registration_applies_to_file_and_range_validation() {
        let path = temp_message_path("registered-model");
        for (value, accepted) in [
            ("openai/gpt-6.1-sol", true),
            ("gpt-6.1-sol", false),
            ("openai/unregistered-model", false),
        ] {
            let message = message_with_body(&format!("Harness: Codex\nModel: {value}"));
            std::fs::write(&path, &message).expect("write test message");
            let file_result = check_commit_message_file(path.to_str().unwrap());
            let range_errors =
                commit_message_errors("0123456789abcdef", "Human", "human@example.com", &message);
            assert_eq!(file_result.is_ok(), accepted, "{value}: {file_result:?}");
            assert_eq!(
                range_errors.is_empty(),
                accepted,
                "{value}: {range_errors:?}"
            );
        }
        std::fs::remove_file(path).expect("remove test message");
    }

    #[test]
    fn dependabot_exception_does_not_exempt_model_registration() {
        let errors = commit_message_errors(
            "0123456789abcdef",
            DEPENDABOT_AUTHOR_NAME,
            DEPENDABOT_AUTHOR_EMAIL,
            "build(deps): bump dependency\n\nHarness: Codex\nModel: unknown-model\n",
        );
        assert!(
            errors.iter().any(|error| error.contains("未登记为规范 ID")),
            "{errors:?}"
        );
        assert!(
            errors.iter().all(|error| !error.contains("缺少 `Refs:`")),
            "{errors:?}"
        );
    }

    #[test]
    fn rejects_lowercase_harness_and_model_keys() {
        assert_rejected(
            &message_with_body("harness: Cursor\nmodel: openai/gpt-6.1-sol"),
            "键名大小写必须精确",
        );
    }

    #[test]
    fn rejects_space_before_attribution_colon() {
        assert_rejected(
            &message_with_body("Harness : Codex CLI\nModel : GPT-5.6"),
            "冒号必须紧跟键名",
        );
    }

    #[test]
    fn accepts_trailers_after_markdown_divider() {
        assert_accepted("docs(governance): 对齐提交规范\n\n正文\n\n---\n\nRefs: #23\n");
    }

    #[test]
    fn parses_trailers_when_host_git_config_changes_separators() {
        if std::env::var_os("LANEFLOW_TRAILER_ISOLATION_CHILD").is_some() {
            assert_accepted(VALID_MESSAGE);
            return;
        }

        let output = Command::new(std::env::current_exe().expect("test binary path"))
            .args([
                "--exact",
                "parses_trailers_when_host_git_config_changes_separators",
                "--test-threads=1",
            ])
            .env("LANEFLOW_TRAILER_ISOLATION_CHILD", "1")
            .env("GIT_CONFIG_COUNT", "2")
            .env("GIT_CONFIG_KEY_0", "trailer.separators")
            .env("GIT_CONFIG_VALUE_0", "=")
            .env("GIT_CONFIG_KEY_1", "trailer.refs.key")
            .env("GIT_CONFIG_VALUE_1", "Ticket")
            .output()
            .expect("isolation child should spawn");

        assert!(
            output.status.success(),
            "stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn rejects_ai_coauthor_hidden_before_human_trailer_block() {
        assert_rejected(
            "docs(governance): 对齐提交规范\n\nCo-authored-by: Codex <noreply@openai.com>\n\nCo-authored-by: Ada Lovelace <ada@example.com>\nRefs: #23\n",
            "不得标注 AI 工具",
        );
    }

    #[test]
    fn accepts_human_coauthor_after_breaking_change_without_ai_attribution() {
        assert_accepted(
            "feat(runtime)!: 调整 tick API\n\nBREAKING CHANGE: 调用方必须显式传入 tick 间隔。\n\nCo-authored-by: Ada Lovelace <ada@example.com>\nRefs: #12\n",
        );
    }

    #[test]
    fn rejects_unclosed_ai_coauthor_address() {
        assert_rejected(
            &message_with_body("Co-authored-by: <noreply@openai.com"),
            "不得标注 AI 工具",
        );
    }

    #[test]
    fn does_not_treat_unclosed_human_address_as_ai_tool() {
        let errors = validate_message(
            "0123456789abcdef",
            &message_with_body("Co-authored-by: <ada@example.com"),
        );

        assert!(
            errors
                .iter()
                .all(|error| !error.contains("不得标注 AI 工具")),
            "{errors:?}"
        );
    }

    #[test]
    fn rejects_folded_ai_coauthor() {
        assert_rejected(
            "docs(governance): 对齐提交规范\n\nCo-authored-by:\n Codex <noreply@openai.com>\nRefs: #23\n",
            "不得标注 AI 工具",
        );
    }

    #[test]
    fn rejects_indented_trailer_block() {
        assert_rejected(
            "docs(governance): 对齐提交规范\n\n Co-authored-by: Ada <ada@example.com>\nHarness: Cursor\nModel: openai/gpt-6.1-sol\nRefs: #23\n",
            "没有把提交末尾认成 trailer",
        );
    }

    #[test]
    fn rejects_ai_tool_coauthors_with_rewrite_hint() {
        for trailer in [
            "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>",
            "Co-authored-by: Codex <noreply@openai.com>",
            "Co-authored-by: Cursor <cursoragent@cursor.com>",
            "Co-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>",
            "co-authored-by: Assistant <noreply@openai.com>",
            "Co-authored-by: cursoragent@cursor.com",
        ] {
            assert_rejected(&message_with_body(trailer), "不得标注 AI 工具");
        }
    }

    #[test]
    fn rejects_ai_tool_coauthor_even_with_harness_model_pair() {
        assert_rejected(
            &message_with_body(
                "Co-authored-by: Codex <noreply@openai.com>\nHarness: Codex CLI\nModel: openai/gpt-6.1-sol",
            ),
            "不得标注 AI 工具",
        );
    }

    #[test]
    fn rejects_unpaired_harness_or_model() {
        assert_rejected(&message_with_body("Harness: Cursor"), "必须成对出现");
        assert_rejected(
            &message_with_body("Model: openai/gpt-6.1-sol"),
            "必须成对出现",
        );
    }

    #[test]
    fn rejects_empty_harness_or_model_value() {
        assert_rejected(
            &message_with_body("Harness:\nModel: openai/gpt-6.1-sol"),
            "`Harness:` 必须写成",
        );
        assert_rejected(
            &message_with_body("Harness:   \nModel: openai/gpt-6.1-sol"),
            "`Harness:` 必须写成",
        );
        assert_rejected(
            &message_with_body("Harness: Cursor\nModel:"),
            "`Model:` 必须写成",
        );
        assert_rejected(
            &message_with_body("Harness: Cursor\nModel:   "),
            "`Model:` 必须写成",
        );
    }

    #[test]
    fn rejects_harness_model_that_are_not_adjacent() {
        assert_rejected(
            &message_with_body("Harness: Cursor\n\nModel: openai/gpt-6.1-sol"),
            "必须成对出现",
        );
        assert_rejected(
            &message_with_body("Model: openai/gpt-6.1-sol\nHarness: Cursor"),
            "必须紧邻",
        );
    }

    #[test]
    fn rejects_repeated_harness_or_model() {
        assert_rejected(
            &message_with_body("Harness: Cursor\nHarness: Claude Code\nModel: openai/gpt-6.1-sol"),
            "`Harness:` 只能出现一次",
        );
        assert_rejected(
            &message_with_body("Harness: Cursor\nModel: openai/gpt-6.1-sol\nModel: other-model"),
            "`Model:` 只能出现一次",
        );
    }

    #[test]
    fn rejects_harness_model_after_issue_footer() {
        assert_rejected(
            &format!("{VALID_MESSAGE}Harness: Cursor\nModel: openai/gpt-6.1-sol\n"),
            "必须位于 `Refs:` / `Closes:` 之前",
        );
    }

    #[test]
    fn rejects_breaking_change_inside_attribution_trailer_block() {
        let message = BREAKING_MESSAGE.replace(
            "BREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填，调用方必须显式传入 tick 间隔。\n\nRefs: #12",
            "BREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填，调用方必须显式传入 tick 间隔。\nHarness: Cursor\nModel: openai/gpt-6.1-sol\nRefs: #12",
        );

        assert_rejected(&message, "trailer 段之前必须有空行");
    }

    #[test]
    fn rejects_attribution_trailer_without_leading_blank_line() {
        assert_rejected(
            "docs(governance): 对齐提交规范\nHarness: Cursor\nModel: openai/gpt-6.1-sol\nRefs: #23\n",
            "trailer 段之前必须有空行",
        );
    }

    #[test]
    fn rejects_coauthored_by_with_space_before_colon() {
        assert_rejected(
            &message_with_body("Co-authored-by : Codex <noreply@openai.com>"),
            "不得标注 AI 工具",
        );
    }

    #[test]
    fn rejects_made_with_cursor_trailer() {
        assert_rejected(&message_with_body("Made-with: Cursor"), "`Made-with:`");
        assert_rejected(&message_with_body("Made-with : Cursor"), "`Made-with:`");
    }

    #[test]
    fn rejects_unparsed_malformed_dependabot_footer_followed_by_junk() {
        let errors = commit_message_errors(
            "0123456789abcdef",
            DEPENDABOT_AUTHOR_NAME,
            DEPENDABOT_AUTHOR_EMAIL,
            "build(deps): bump serde from 1.0.227 to 1.0.228\n\nRefs: malformed\njunk\n",
        );

        assert!(
            errors.iter().any(|error| error.contains("格式不正确")),
            "{errors:?}"
        );
        assert!(
            errors.iter().all(|error| !error.contains("缺少 `Refs:`")),
            "{errors:?}"
        );
    }

    #[test]
    fn rejects_spaced_malformed_dependabot_issue_footer() {
        let errors = commit_message_errors(
            "0123456789abcdef",
            DEPENDABOT_AUTHOR_NAME,
            DEPENDABOT_AUTHOR_EMAIL,
            "build(deps): bump serde from 1.0.227 to 1.0.228\n\nRefs : malformed\n",
        );

        assert!(
            errors.iter().any(|error| error.contains("格式不正确")),
            "{errors:?}"
        );
        assert!(
            errors.iter().all(|error| !error.contains("缺少 `Refs:`")),
            "{errors:?}"
        );
    }

    #[test]
    fn rejects_malformed_dependabot_issue_footer() {
        let errors = commit_message_errors(
            "0123456789abcdef",
            DEPENDABOT_AUTHOR_NAME,
            DEPENDABOT_AUTHOR_EMAIL,
            "build(deps): bump serde from 1.0.227 to 1.0.228\n\nRefs: malformed\n",
        );

        assert!(
            errors.iter().any(|error| error.contains("格式不正确")),
            "{errors:?}"
        );
    }

    #[test]
    fn rejects_dependabot_issue_footer_that_is_not_last() {
        let errors = commit_message_errors(
            "0123456789abcdef",
            DEPENDABOT_AUTHOR_NAME,
            DEPENDABOT_AUTHOR_EMAIL,
            "build(deps): bump serde from 1.0.227 to 1.0.228\n\nRefs: #12\njunk\n",
        );

        assert!(
            errors.iter().any(|error| error.contains("最后一个非空行")),
            "{errors:?}"
        );
    }

    #[test]
    fn accepts_dependabot_message_without_issue_footer() {
        let errors = commit_message_errors(
            "0123456789abcdef",
            DEPENDABOT_AUTHOR_NAME,
            DEPENDABOT_AUTHOR_EMAIL,
            "build(deps): bump serde from 1.0.227 to 1.0.228\n",
        );

        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn rejects_ai_tool_coauthor_on_dependabot_commit() {
        let message = "build(deps): bump serde from 1.0.227 to 1.0.228\n\nCo-authored-by: Codex <noreply@openai.com>\n";
        let errors = commit_message_errors(
            "0123456789abcdef",
            DEPENDABOT_AUTHOR_NAME,
            DEPENDABOT_AUTHOR_EMAIL,
            message,
        );

        assert!(
            errors
                .iter()
                .any(|error| error.contains("不得标注 AI 工具")),
            "{errors:?}"
        );
        assert!(
            errors.iter().all(|error| !error.contains("缺少 `Refs:`")),
            "{errors:?}"
        );
    }

    #[test]
    fn rejects_blank_line_inside_attribution_trailer_block() {
        assert_rejected(
            "docs(governance): 对齐提交规范\n\nHarness: Cursor\nModel: openai/gpt-6.1-sol\n\nRefs: #23\n",
            "中间不能空行",
        );
    }

    #[test]
    fn rejects_ai_tool_coauthor_in_commit_message_file() {
        let path = temp_message_path("ai-coauthor");
        let message = VALID_MESSAGE.replace(
            "Refs: #23\n",
            "Co-authored-by: Codex <noreply@openai.com>\nRefs: #23\n",
        );
        std::fs::write(&path, message).expect("test commit message should be written");

        let result = check_commit_message_file(path.to_str().expect("path should be UTF-8"));

        std::fs::remove_file(&path).expect("test commit message should be removed");
        let error = result.expect_err("AI tool co-author should fail");
        assert!(error.contains("不得标注 AI 工具"));
    }

    fn temp_message_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("laneflow-xtask-{name}-{}.txt", std::process::id()))
    }
}
