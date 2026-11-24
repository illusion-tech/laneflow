//! 提交消息正文：必需正文、篇幅上限与按显示宽度 72 列折行（#1536）。
//!
//! 断点取 UAX #14（`unicode-linebreak`），再排除四类断点：ASCII 非空白字符之间的断点
//! （连字符、斜杠与标识符）、反引号代码内的断点、含 `/` 的路径与 URL 内的断点，以及会让
//! 续行变成注释、trailer、受控字段、列表项或围栏的断点。显示宽度按 `textwrap`
//! 的 `display_width` 计算，中日韩字符计 2 列。hook 先折行再校验，CI 只校验；两者共用
//! 同一断点规则，所以通过宽度校验的正文就是折行的不动点。

use textwrap::core::{Word, display_width};
use textwrap::{Options, WordSeparator, WordSplitter};
use unicode_linebreak::linebreaks;

use crate::commit_messages;
use crate::markdown_tables::{is_closing_fence, opening_fence};

/// 正文行的显示宽度上限。
pub(crate) const BODY_MAX_WIDTH: usize = 72;
/// 正文（不含 `BREAKING CHANGE:` 行与末尾 trailer 段）的非空白字符上限。
pub(crate) const BODY_MAX_CHARS: usize = 800;
/// 必须写正文的提交类型；标题带 `!` 的提交同样必须写正文。
const BODY_REQUIRED_TYPES: &[&str] = &["feat", "fix", "refactor", "perf"];
/// `git commit --verbose` 写入的剪刀线；其后是 diff，不属于提交消息。
const SCISSORS_LINE: &str = "# ------------------------ >8 ------------------------";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineRole {
    Title,
    Blank,
    /// 折行并计宽度的正文行。
    Prose,
    /// 缩进代码与围栏代码：计入正文篇幅，不折行。
    Code,
    /// `BREAKING CHANGE:` 行与末尾 trailer 段：不计篇幅，不折行。
    Footer,
}

pub(crate) fn is_comment_line(line: &str) -> bool {
    line.trim_start().starts_with('#')
}

/// 去掉剪刀线及其后的 diff。
pub(crate) fn before_scissors(message: &str) -> &str {
    if message.starts_with(SCISSORS_LINE) {
        return "";
    }
    match message.find(&format!("\n{SCISSORS_LINE}")) {
        Some(index) => &message[..=index],
        None => message,
    }
}

/// 折行 commit message 文件文本中超宽的正文行。注释行、剪刀线之后的内容与非正文行
/// 原样保留，宽度未超限的行不改动，也不合并提交者自行断开的短行。
pub(crate) fn wrap_commit_message_text(text: &str) -> String {
    let head = before_scissors(text);
    let tail = &text[head.len()..];
    let lines: Vec<&str> = head.split('\n').collect();
    let content: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| !is_comment_line(line))
        .collect();
    let mut roles = line_roles(&content).into_iter();

    let mut output = Vec::with_capacity(lines.len());
    for line in lines {
        if is_comment_line(line) {
            output.push(line.to_string());
            continue;
        }
        match roles.next() {
            Some(LineRole::Prose) => output.extend(wrap_prose_line(line)),
            _ => output.push(line.to_string()),
        }
    }

    let mut wrapped = output.join("\n");
    wrapped.push_str(tail);
    wrapped
}

/// 校验已剥离注释的提交消息正文：必需正文、篇幅上限与行宽。
pub(crate) fn body_errors(message: &str) -> Vec<String> {
    let lines: Vec<&str> = message.lines().collect();
    let roles = line_roles(&lines);
    let mut errors = Vec::new();

    let body_chars: usize = lines
        .iter()
        .zip(&roles)
        .filter(|(_line, role)| matches!(role, LineRole::Prose | LineRole::Code))
        .map(|(line, _role)| line.chars().filter(|ch| !ch.is_whitespace()).count())
        .sum();

    let title = lines.first().copied().unwrap_or_default();
    if body_chars == 0 && body_required(title) {
        errors.push(
            "`feat`、`fix`、`refactor`、`perf` 与标题带 `!` 的提交必须有正文，写明原因、契约变化与影响"
                .to_string(),
        );
    }

    if body_chars > BODY_MAX_CHARS {
        errors.push(format!(
            "正文超过 {BODY_MAX_CHARS} 个非空白字符（当前 {body_chars}），细节写入 PR 或设计文档，或拆分提交"
        ));
    }

    for (index, (line, role)) in lines.iter().zip(&roles).enumerate() {
        if *role != LineRole::Prose {
            continue;
        }
        let width = display_width(line);
        if width > BODY_MAX_WIDTH && wrap_prose_line(line).len() > 1 {
            errors.push(format!(
                "正文第 {} 行显示宽度 {width} 列，超过 {BODY_MAX_WIDTH} 列；启用仓库 hook 后提交时会自动折行",
                index + 1
            ));
        }
    }

    errors
}

fn body_required(title: &str) -> bool {
    let Some((prefix, _description)) = title.split_once(": ") else {
        return false;
    };
    if prefix.ends_with('!') {
        return true;
    }
    let commit_type = prefix.split_once('(').map_or(prefix, |(kind, _scope)| kind);
    BODY_REQUIRED_TYPES.contains(&commit_type)
}

fn line_roles(lines: &[&str]) -> Vec<LineRole> {
    let mut roles = Vec::with_capacity(lines.len());
    let mut open_fence = None;
    for (index, line) in lines.iter().enumerate() {
        let role = if index == 0 {
            LineRole::Title
        } else if let Some(fence) = open_fence {
            if fence_indent_allowed(line) && is_closing_fence(line, fence) {
                open_fence = None;
            }
            LineRole::Code
        } else if let Some(fence) = fence_indent_allowed(line)
            .then(|| opening_fence(line))
            .flatten()
        {
            open_fence = Some(fence);
            LineRole::Code
        } else if line.trim().is_empty() {
            LineRole::Blank
        } else if line.starts_with('\t') || line.starts_with("    ") {
            LineRole::Code
        } else if line.starts_with("BREAKING CHANGE:") {
            LineRole::Footer
        } else {
            LineRole::Prose
        };
        roles.push(role);
    }

    if let Some((start, end)) = trailer_paragraph(lines, &roles) {
        for role in &mut roles[start..=end] {
            *role = LineRole::Footer;
        }
    }
    roles
}

/// CommonMark 规定开、关围栏最多缩进 3 列（Tab 按 4 列制表位计）；缩进更深的围栏符号
/// 属于代码内容。`markdown_tables` 的围栏判定不区分列表等容器，所以缩进门槛放在这里，
/// 字符与长度规则仍共用该判定。
fn fence_indent_allowed(line: &str) -> bool {
    let mut column = 0;
    for ch in line.chars() {
        match ch {
            ' ' => column += 1,
            '\t' => column = (column / 4 + 1) * 4,
            _ => break,
        }
        if column > 3 {
            return false;
        }
    }
    true
}

/// 末尾段落若每行都是 `Key: value` 形式，即为 trailer 段。
fn trailer_paragraph(lines: &[&str], roles: &[LineRole]) -> Option<(usize, usize)> {
    let end = roles.iter().rposition(|role| *role != LineRole::Blank)?;
    let mut start = end;
    while start > 0 && roles[start - 1] != LineRole::Blank && roles[start - 1] != LineRole::Title {
        start -= 1;
    }
    if start == 0 || roles[start - 1] != LineRole::Blank {
        return None;
    }
    lines[start..=end]
        .iter()
        .all(|line| is_trailer_like(line))
        .then_some((start, end))
}

fn is_trailer_like(line: &str) -> bool {
    let Some((key, value)) = line.split_once(':') else {
        return false;
    };
    let mut chars = key.chars();
    chars.next().is_some_and(|ch| ch.is_ascii_alphanumeric())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        && (value.is_empty() || value.starts_with(' '))
}

fn wrap_prose_line(line: &str) -> Vec<String> {
    if display_width(line) <= BODY_MAX_WIDTH {
        return vec![line.to_string()];
    }

    let content_start = list_content_start(line);
    let initial_indent = &line[..content_start];
    let subsequent_indent = " ".repeat(display_width(initial_indent));
    let options = Options::new(BODY_MAX_WIDTH)
        .initial_indent(initial_indent)
        .subsequent_indent(&subsequent_indent)
        .break_words(false)
        .word_separator(WordSeparator::Custom(protected_words))
        .word_splitter(WordSplitter::NoHyphenation);

    textwrap::wrap(&line[content_start..], options)
        .into_iter()
        .map(|wrapped| wrapped.trim_end().to_string())
        .collect()
}

/// 行首缩进与列表标记（`- `、`* `、`+ `、`1. `、`1) `）之后的正文起点。
fn list_content_start(line: &str) -> usize {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    let marker = if rest.starts_with("- ") || rest.starts_with("* ") || rest.starts_with("+ ") {
        1
    } else {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let after = &rest[digits..];
        if digits > 0 && (after.starts_with(". ") || after.starts_with(") ")) {
            digits + 1
        } else {
            0
        }
    };
    let after_marker = &line[indent + marker..];
    if marker == 0 {
        indent
    } else {
        indent + marker + (after_marker.len() - after_marker.trim_start_matches(' ').len())
    }
}

fn protected_words(line: &str) -> Box<dyn Iterator<Item = Word<'_>> + '_> {
    let mut protected = code_span_ranges(line);
    protected.extend(path_ranges(line, &protected));
    let mut words = Vec::new();
    let mut start = 0;
    for (offset, _opportunity) in linebreaks(line) {
        if offset >= line.len() || !break_permitted(line, offset, &protected) {
            continue;
        }
        if offset > start {
            words.push(Word::from(&line[start..offset]));
            start = offset;
        }
    }
    if start < line.len() {
        words.push(Word::from(&line[start..]));
    }
    Box::new(words.into_iter())
}

fn break_permitted(line: &str, offset: usize, protected: &[(usize, usize)]) -> bool {
    if protected
        .iter()
        .any(|&(start, end)| start < offset && offset < end)
    {
        return false;
    }
    let previous = line[..offset].chars().next_back();
    let next = line[offset..].chars().next();
    let ascii_token_char = |ch: Option<char>| ch.is_some_and(|ch| ch.is_ascii_graphic());
    !(ascii_token_char(previous) && ascii_token_char(next)) && !changes_line_role(&line[offset..])
}

/// 以该文本开头的续行会被 Git 或校验器识别为另一种行：注释、受控字段（与校验器共用
/// [`commit_messages::is_controlled_field_line`]）、其他 `Key:` 形式的 trailer、
/// `BREAKING CHANGE`、列表项或代码围栏。这样的断点一律不用，避免折行改变消息内容或语义。
fn changes_line_role(rest: &str) -> bool {
    let rest = rest.trim_start();
    if is_comment_line(rest)
        || commit_messages::is_controlled_field_line(rest)
        || rest.starts_with("BREAKING CHANGE")
        || rest.starts_with("```")
        || rest.starts_with("~~~")
        || list_content_start(rest) > 0
    {
        return true;
    }
    let token_len = rest
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
        .count();
    let after_token = rest[token_len..].trim_start_matches([' ', '\t']);
    token_len > 0
        && after_token
            .strip_prefix(':')
            .is_some_and(|value| value.is_empty() || value.starts_with([' ', '\t']))
}

/// 含 `/` 的路径与 URL 的字节区间，区间内不断行。区间向两侧延伸到空白、中日韩标点、
/// 反引号或已保护区间为止，区间内部的中英文切换不视为边界，所以 `docs/v2道路说明.md`
/// 整段受保护。紧贴中文正文、没有空白或标点分隔的路径会连同相邻中文一起受保护；
/// 不含 ASCII 字母数字、也没有扩展名的“读取/写入”不按路径处理。
fn path_ranges(line: &str, protected: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let in_protected = |at: usize| {
        protected
            .iter()
            .any(|&(start, end)| start <= at && at < end)
    };
    let is_path_char = |index: usize| {
        let (at, ch) = chars[index];
        !(ch.is_whitespace() || ch == '`' || is_cjk_punctuation(ch) || in_protected(at))
    };
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for (index, &(at, ch)) in chars.iter().enumerate() {
        if ch != '/'
            || in_protected(at)
            || ranges.iter().any(|&(start, end)| start <= at && at < end)
        {
            continue;
        }
        let mut first = index;
        while first > 0 && is_path_char(first - 1) {
            first -= 1;
        }
        let mut last = index;
        while last + 1 < chars.len() && is_path_char(last + 1) {
            last += 1;
        }
        let start = chars[first].0;
        let end = chars.get(last + 1).map_or(line.len(), |&(at, _ch)| at);
        let span = &line[start..end];
        let has_extension = span.rsplit_once('.').is_some_and(|(_stem, ext)| {
            !ext.is_empty() && ext.bytes().all(|b| b.is_ascii_alphanumeric())
        });
        if span.contains("://") || span.bytes().any(|b| b.is_ascii_alphanumeric()) || has_extension
        {
            ranges.push((start, end));
        }
    }
    ranges
}

fn is_cjk_punctuation(ch: char) -> bool {
    matches!(ch,
        '\u{3000}'..='\u{303F}'
            | '\u{FF01}'..='\u{FF0F}'
            | '\u{FF1A}'..='\u{FF20}'
            | '\u{FF3B}'..='\u{FF40}'
            | '\u{FF5B}'..='\u{FF65}'
            | '\u{2014}'..='\u{2027}')
}

/// 反引号代码的字节区间（含两端反引号）；未闭合的反引号不形成区间。
fn code_span_ranges(line: &str) -> Vec<(usize, usize)> {
    let bytes = line.as_bytes();
    let run_len = |at: usize| bytes[at..].iter().take_while(|&&b| b == b'`').count();
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'`' {
            index += 1;
            continue;
        }
        let open = run_len(index);
        let mut search = index + open;
        let mut close = None;
        while search < bytes.len() {
            if bytes[search] == b'`' {
                let run = run_len(search);
                if run == open {
                    close = Some(search + run);
                    break;
                }
                search += run;
            } else {
                search += 1;
            }
        }
        match close {
            Some(end) => {
                ranges.push((index, end));
                index = end;
            }
            None => index += open,
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG_CJK: &str = "道路编辑预检在构建器的准入账本上开启一次编译遍，计划在读取任何字段之前按已通过校验的向量长度求出峰值，计划超出操作资源预算时报告记账缺陷。";

    fn widths(lines: &[String]) -> Vec<usize> {
        lines.iter().map(|line| display_width(line)).collect()
    }

    #[test]
    fn wraps_cjk_prose_within_width_without_losing_text() {
        let wrapped = wrap_prose_line(LONG_CJK);

        assert!(wrapped.len() > 1);
        assert!(
            widths(&wrapped)
                .iter()
                .all(|&width| width <= BODY_MAX_WIDTH),
            "{wrapped:?}"
        );
        assert_eq!(wrapped.concat(), LONG_CJK);
    }

    #[test]
    fn does_not_start_a_line_with_closing_punctuation() {
        for shift in 0..12 {
            let line = format!("{}{LONG_CJK}{LONG_CJK}", "前".repeat(shift));
            for wrapped in wrap_prose_line(&line).iter().skip(1) {
                let first = wrapped.chars().next().unwrap();
                assert!(!"，。、；：）」』".contains(first), "{wrapped:?}");
            }
        }
    }

    #[test]
    fn keeps_identifiers_paths_urls_and_code_spans_whole() {
        let tokens = [
            "placement-fixtures",
            "xtask/src/commit_messages.rs",
            "https://github.com/illusion-tech/laneflow/issues/1536",
            "`git log --format=%B`",
            "derive_canonical_stable_id_v1",
        ];
        for shift in 0..40 {
            let line = format!("{}{}", "字".repeat(shift), tokens.join(" 以及 "));
            let wrapped = wrap_prose_line(&line);
            for token in tokens {
                assert!(
                    wrapped.iter().any(|wrapped| wrapped.contains(token)),
                    "`{token}` 被拆开: {wrapped:?}"
                );
            }
        }
    }

    #[test]
    fn breaks_english_only_at_spaces() {
        let line = "The canonical identity derivation no longer receives a compile profile, so every caller passes the entity kind directly.";
        let wrapped = wrap_prose_line(line);

        assert_eq!(wrapped.join(" "), line);
        assert!(
            widths(&wrapped)
                .iter()
                .all(|&width| width <= BODY_MAX_WIDTH)
        );
    }

    #[test]
    fn indents_list_continuations_under_the_marker() {
        for marker in ["- ", "12. ", "  - "] {
            let line = format!("{marker}{LONG_CJK}");
            let wrapped = wrap_prose_line(&line);
            let indent = " ".repeat(display_width(marker));

            assert!(wrapped[0].starts_with(marker));
            assert!(
                wrapped[1..]
                    .iter()
                    .all(|wrapped| wrapped.starts_with(&indent)
                        && !wrapped[indent.len()..].starts_with(' '))
            );
        }
    }

    #[test]
    fn leaves_unbreakable_long_units_on_their_own_line() {
        let url = format!("https://example.com/{}", "a".repeat(80));
        assert_eq!(wrap_prose_line(&url), vec![url.clone()]);

        let message = format!("docs: 记录来源\n\n{url}\n\nRefs: #1\n");
        assert!(body_errors(&message).is_empty());
    }

    #[test]
    fn wraps_only_prose_lines_of_a_message_file() {
        let long_title = format!("feat(compiler): {}", "题".repeat(40));
        let breaking = format!("BREAKING CHANGE: {LONG_CJK}");
        let text = format!(
            "{long_title}\n\n{LONG_CJK}\n\n```text\n{LONG_CJK}\n```\n\n    {LONG_CJK}\n\n{breaking}\n\nHarness: anthropic/claude-code/desktop\nModel: anthropic/claude-opus-5-5\nRefs: #1536\n# {LONG_CJK}\n{SCISSORS_LINE}\n{LONG_CJK}\n"
        );

        let wrapped = wrap_commit_message_text(&text);
        let lines: Vec<&str> = wrapped.lines().collect();

        assert_eq!(lines[0], long_title);
        assert!(lines[2].len() < LONG_CJK.len() && lines[3].len() < LONG_CJK.len());
        for kept in [
            LONG_CJK.to_string(),
            format!("    {LONG_CJK}"),
            breaking,
            format!("# {LONG_CJK}"),
        ] {
            assert!(lines.contains(&kept.as_str()), "缺少原样保留的行: {kept}");
        }
        assert!(wrapped.ends_with(&format!("{SCISSORS_LINE}\n{LONG_CJK}\n")));
        assert!(wrapped.contains("```text\n"));
    }

    #[test]
    fn wrapping_is_idempotent_and_satisfies_the_check() {
        let message = format!(
            "fix(compiler): 补齐停车计划\n\n{LONG_CJK}{LONG_CJK}\n\n- {LONG_CJK} `CompilerControlledLiveBytes` 报告。\n\nRefs: #1536\n"
        );

        let once = wrap_commit_message_text(&message);
        assert_eq!(wrap_commit_message_text(&once), once);
        assert!(body_errors(&once).is_empty(), "{:?}", body_errors(&once));
        assert!(!body_errors(&message).is_empty());
    }

    #[test]
    fn reports_overwide_prose_line_number() {
        let message = format!("fix(compiler): 补齐停车计划\n\n短行。\n{LONG_CJK}\n\nRefs: #1536\n");

        let errors = body_errors(&message);

        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("正文第 4 行"), "{errors:?}");
    }

    #[test]
    fn requires_body_for_behavioural_types_and_breaking_titles() {
        for title in [
            "feat(runtime): 实现 fixed-step tick",
            "fix: 拒绝非有限长度",
            "refactor(compiler): 统一索引",
            "perf(compiler): 跳过扫描",
            "docs(governance)!: 调整规范",
        ] {
            let message = format!("{title}\n\nBREAKING CHANGE: 说明。\n\nRefs: #1\n");
            let errors = body_errors(&message);
            assert_eq!(errors.len(), 1, "{title}: {errors:?}");
            assert!(errors[0].contains("必须有正文"));
        }

        for title in [
            "docs(governance): 对齐提交规范",
            "test: 补充回归",
            "chore: 清理",
        ] {
            assert!(body_errors(&format!("{title}\n\nRefs: #1\n")).is_empty());
        }
    }

    #[test]
    fn counts_prose_and_code_but_not_footers_toward_the_limit() {
        let paragraph = "字".repeat(36);
        let body = vec![paragraph.as_str(); BODY_MAX_CHARS / 36].join("\n");
        let footer = format!("BREAKING CHANGE: {}", "长".repeat(200));
        let within = format!("feat!: 调整接口\n\n{body}\n\n{footer}\n\nRefs: #1\n");
        assert!(
            body_errors(&within).is_empty(),
            "{:?}",
            body_errors(&within)
        );

        let over = format!(
            "feat!: 调整接口\n\n{body}\n\n```\nabcdefghijklmnopqrstuvwxyz\n```\n\n{footer}\n\nRefs: #1\n"
        );
        let errors = body_errors(&over);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("正文超过 800"));
    }

    #[test]
    fn never_starts_a_continuation_with_role_changing_text() {
        for trigger in [
            "#1536 记录了原始失败。",
            "Harness: anthropic/claude-code/cli 只是示例。",
            "harness : 大小写与空格变体。",
            "Model:openai/gpt-6-astra 冒号后没有空格。",
            "MODEL\t:\topenai/gpt-6-astra 是 Tab 变体。",
            "Co-authored-by:Ada 也只是示例。",
            "BREAKING CHANGE: 只是引用。",
            "Refs: #1 也只是引用。",
            "- 不是列表项。",
            "12. 不是列表项。",
            "```text 不是围栏。",
            "~~~ 不是围栏。",
        ] {
            let prefix = trigger.split(' ').next().unwrap();
            for shift in 0..72 {
                let line = format!("{}前文说明 {trigger}{LONG_CJK}", "字".repeat(shift));
                let wrapped = wrap_prose_line(&line);
                assert!(
                    wrapped[1..]
                        .iter()
                        .all(|wrapped| !wrapped.trim_start().starts_with(prefix)),
                    "{trigger}: {wrapped:?}"
                );
                let rejoined: String = wrapped.concat().split_whitespace().collect();
                assert_eq!(rejoined, line.split_whitespace().collect::<String>());
            }
        }
    }

    #[test]
    fn keeps_tilde_and_longer_backtick_fences_verbatim() {
        let long_command = format!("cargo run -- {}", "argument ".repeat(12));
        let message = format!(
            "fix(compiler): 补齐停车计划\n\n原因说明。\n\n~~~text\n{long_command}\n{LONG_CJK}\n~~~\n\n````text\n```\n{LONG_CJK}\n````\n\nRefs: #1536\n"
        );

        assert_eq!(wrap_commit_message_text(&message), message);
        assert!(
            body_errors(&message).is_empty(),
            "{:?}",
            body_errors(&message)
        );
    }

    #[test]
    fn closes_fences_only_with_matching_marker_and_length() {
        let message =
            format!("docs: 说明\n\n````\n```\n{LONG_CJK}\n````\n{LONG_CJK}\n\nRefs: #1\n");
        let roles = line_roles(&message.lines().collect::<Vec<_>>());

        assert_eq!(&roles[2..6], &[LineRole::Code; 4]);
        assert_eq!(roles[6], LineRole::Prose);
    }

    #[test]
    fn keeps_non_ascii_paths_and_urls_whole() {
        let path = "docs/道路编辑预检构建器准入账本编译字段向量长度峰值说明.md";
        let url = "https://example.com/文档/道路编辑说明";
        for shift in 0..72 {
            for token in [path, url] {
                let line = format!(
                    "{}文档移至 {token}，调用方按新路径读取。{LONG_CJK}",
                    "字".repeat(shift)
                );
                let wrapped = wrap_prose_line(&line);
                assert!(
                    wrapped.iter().any(|wrapped| wrapped.contains(token)),
                    "`{token}` 被拆开: {wrapped:?}"
                );
            }
        }

        let attached = "说明已移至docs/a.md调用方按新路径读取，其余不变";
        let ranges = path_ranges(attached, &[]);
        assert_eq!(ranges.len(), 1);
        assert_eq!(
            &attached[ranges[0].0..ranges[0].1],
            "说明已移至docs/a.md调用方按新路径读取"
        );
        assert!(path_ranges("支持读取/写入两种模式", &[]).is_empty());
    }

    #[test]
    fn keeps_mixed_script_file_names_whole() {
        for token in [
            "docs/道路编辑说明.md",
            "docs/v2道路编辑说明.md",
            "docs/道路v2编辑说明.md",
            "https://example.com/v2道路编辑说明",
        ] {
            let line = format!("文档移至 {token}，调用方按新路径读取。");
            let ranges = path_ranges(&line, &[]);
            assert_eq!(ranges.len(), 1, "{token}");
            assert_eq!(&line[ranges[0].0..ranges[0].1], token);

            for shift in 0..72 {
                let line = format!("{}{line}{LONG_CJK}", "字".repeat(shift));
                let wrapped = wrap_prose_line(&line);
                assert!(
                    wrapped.iter().any(|wrapped| wrapped.contains(token)),
                    "`{token}` 被拆开: {wrapped:?}"
                );
            }
        }

        let url = format!("https://example.com/v2{}", "道路".repeat(30));
        assert_eq!(display_width(&url), 142);
        assert_eq!(wrap_prose_line(&url), vec![url.clone()]);
        let message = format!("docs: 记录来源\n\n{url}\n\nRefs: #1\n");
        assert_eq!(wrap_commit_message_text(&message), message);
        assert!(
            body_errors(&message).is_empty(),
            "{:?}",
            body_errors(&message)
        );
    }

    #[test]
    fn deeply_indented_fence_markers_are_code_content() {
        let long_command = format!("cargo run -- {}", "argument ".repeat(10));
        for pseudo_close in ["    ```", "\t```", "  \t```", "    ~~~~"] {
            let fence = if pseudo_close.ends_with('~') {
                "~~~"
            } else {
                "```"
            };
            let message = format!(
                "docs: 说明\n\n{fence}text\n{pseudo_close}\n{long_command}\n{fence}\n\nRefs: #1\n"
            );
            let roles = line_roles(&message.lines().collect::<Vec<_>>());

            assert_eq!(&roles[2..6], &[LineRole::Code; 4], "{pseudo_close:?}");
            assert_eq!(roles[6], LineRole::Blank, "{pseudo_close:?}");
            assert_eq!(wrap_commit_message_text(&message), message);
        }

        for close in ["```", " ```", "   ```"] {
            let message = format!("docs: 说明\n\n```\ncode\n{close}\n{LONG_CJK}\n\nRefs: #1\n");
            let roles = line_roles(&message.lines().collect::<Vec<_>>());
            assert_eq!(roles[5], LineRole::Prose, "{close:?}");
        }

        let roles = line_roles(&["docs: 说明", "", "    ```", "x"]);
        assert_eq!(roles[2], LineRole::Code);
        assert_eq!(roles[3], LineRole::Prose);
    }

    #[test]
    fn ignores_scissors_section() {
        assert_eq!(before_scissors(&format!("a\n{SCISSORS_LINE}\ndiff")), "a\n");
        assert_eq!(before_scissors(&format!("{SCISSORS_LINE}\ndiff")), "");
        assert_eq!(before_scissors("a\nb\n"), "a\nb\n");
    }
}
