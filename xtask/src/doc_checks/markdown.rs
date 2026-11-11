//! 用 pulldown-cmark 抽取链接、图片、标题和行内代码。不扫描 `](...)` 正则。

use std::cell::RefCell;
use std::collections::BTreeSet;

use pulldown_cmark::{BrokenLink, Event, LinkType, Options, Parser, Tag, TagEnd};

use super::scan::{self, ResolveError, Resolved};
use super::slug::Slugger;
use super::{FindingKind, SkipKind};

#[derive(Debug)]
pub(super) struct Document {
    pub(super) links: Vec<LinkRec>,
    pub(super) anchors: BTreeSet<String>,
    pub(super) code_spans: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) struct LinkRec {
    pub(super) line: usize,
    pub(super) column: usize,
    pub(super) raw: String,
    pub(super) kind: LinkKind,
}

#[derive(Clone, Debug)]
pub(super) enum LinkKind {
    Skip {
        kind: SkipKind,
        reason: String,
        visible: bool,
    },
    Local {
        rel: String,
        directory: bool,
        fragment: Option<String>,
    },
    Failure {
        kind: FindingKind,
        reason: &'static str,
    },
}

pub(super) fn parse_document(text: &str, source_rel: &str) -> Document {
    let mut anchors = BTreeSet::new();
    let mut links = Vec::new();
    let mut code_spans = Vec::new();
    let mut slugger = Slugger::default();
    let mut heading = String::new();
    let mut in_heading = false;
    let mut in_code = 0_u32;
    // HTML 块按行拆成多次事件。注释可能在一行开始、在后面某行结束。
    let mut in_html_comment = false;
    let lines = LineIndex::new(text);
    let broken_links = RefCell::new(Vec::<(usize, String)>::new());
    {
        let mut callback = |link: BrokenLink<'_>| {
            // 未定义的 shortcut 在 CommonMark 里不是链接。引用和折叠引用没有定义则是坏链接。
            if !matches!(link.link_type, LinkType::Shortcut) {
                broken_links
                    .borrow_mut()
                    .push((link.span.start, link.reference.to_string()));
            }
            None
        };
        let parser =
            Parser::new_with_broken_link_callback(text, parser_options(), Some(&mut callback));

        for (event, range) in parser.into_offset_iter() {
            match event {
                Event::Start(Tag::CodeBlock(_)) => in_code += 1,
                Event::End(TagEnd::CodeBlock) => in_code = in_code.saturating_sub(1),
                Event::Start(Tag::Heading { .. }) => {
                    in_heading = true;
                    heading.clear();
                }
                Event::End(TagEnd::Heading(_)) => {
                    in_heading = false;
                    anchors.insert(slugger.next(&heading));
                    heading.clear();
                }
                Event::Text(text) if in_heading => heading.push_str(&text),
                Event::Code(text) => {
                    if in_heading {
                        heading.push_str(&text);
                    }
                    if in_code == 0 {
                        code_spans.push(text.to_string());
                    }
                }
                Event::InlineMath(text) | Event::DisplayMath(text) if in_heading => {
                    heading.push_str(&text);
                }
                Event::SoftBreak | Event::HardBreak if in_heading => heading.push(' '),
                Event::Html(html) | Event::InlineHtml(html) => {
                    if in_code == 0 {
                        in_html_comment = push_anchor_ids(&html, &mut anchors, in_html_comment);
                    }
                }
                Event::Start(Tag::Link {
                    link_type,
                    dest_url,
                    id,
                    ..
                })
                | Event::Start(Tag::Image {
                    link_type,
                    dest_url,
                    id,
                    ..
                }) if in_code == 0 && !matches!(link_type, LinkType::ShortcutUnknown) => {
                    let (line, column) = lines.position(text, range.start);
                    let raw = if dest_url.is_empty() {
                        id.to_string()
                    } else {
                        dest_url.to_string()
                    };
                    links.push(LinkRec {
                        line,
                        column,
                        kind: classify_destination(link_type, &dest_url, source_rel),
                        raw,
                    });
                }
                _ => {}
            }
        }
    }

    for (offset, reference) in broken_links.into_inner() {
        let (line, column) = lines.position(text, offset);
        links.push(LinkRec {
            line,
            column,
            raw: reference,
            kind: LinkKind::Failure {
                kind: FindingKind::Path,
                reason: "引用链接没有定义",
            },
        });
    }

    Document {
        links,
        anchors,
        code_spans,
    }
}

fn parser_options() -> Options {
    // 不打开 ENABLE_HEADING_ATTRIBUTES：`{#id}` 只是标题文本，不是自定义锚点。
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

fn classify_destination(link_type: LinkType, dest: &str, source_rel: &str) -> LinkKind {
    match link_type {
        LinkType::ShortcutUnknown => protocol_skip(SkipKind::Unsupported),
        LinkType::ReferenceUnknown | LinkType::CollapsedUnknown => LinkKind::Failure {
            kind: FindingKind::Path,
            reason: "引用链接没有定义",
        },
        LinkType::WikiLink { .. } => visible_skip("不支持的维基链接语法"),
        LinkType::Email => protocol_skip(SkipKind::Mailto),
        LinkType::Inline
        | LinkType::Reference
        | LinkType::Collapsed
        | LinkType::Shortcut
        | LinkType::Autolink => classify_dest_text(dest, source_rel),
    }
}

fn classify_dest_text(dest: &str, source_rel: &str) -> LinkKind {
    if dest.is_empty() {
        return LinkKind::Failure {
            kind: FindingKind::Path,
            reason: "链接目标为空",
        };
    }
    if is_rustdoc_symbol(dest) {
        return protocol_skip(SkipKind::Rustdoc);
    }
    if let Some(scheme) = scheme_of(dest) {
        if scheme.len() == 1 {
            return visible_skip("盘符路径不支持");
        }
        return if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
            protocol_skip(SkipKind::Http)
        } else if scheme.eq_ignore_ascii_case("mailto") {
            protocol_skip(SkipKind::Mailto)
        } else {
            visible_skip("其他协议不访问")
        };
    }
    if let Some(fragment) = dest.strip_prefix('#') {
        return finish_local(source_rel, "", Some(fragment), true);
    }
    let (path, fragment) = split_fragment(dest);
    if path.contains('?') {
        return visible_skip("查询字符串不支持");
    }
    if path.contains('\\') {
        return visible_skip("反斜线路径不支持");
    }
    finish_local(source_rel, path, fragment, false)
}

fn finish_local(
    source_rel: &str,
    path: &str,
    fragment: Option<&str>,
    fragment_only: bool,
) -> LinkKind {
    let fragment = match fragment {
        Some(value) => match scan::percent_decode(value) {
            Ok(decoded) if decoded.is_empty() => None,
            Ok(decoded) => Some(decoded),
            Err(()) => return visible_skip("百分号编码无效"),
        },
        None => None,
    };
    if fragment_only {
        return LinkKind::Local {
            rel: source_rel.to_string(),
            directory: false,
            fragment,
        };
    }
    let base_rel = scan::parent_rel(source_rel);
    let decoded = match scan::percent_decode(path) {
        Ok(decoded) => decoded,
        Err(()) => return visible_skip("百分号编码无效"),
    };
    match scan::resolve_decoded(base_rel, &decoded) {
        Ok(Resolved { rel, directory }) => LinkKind::Local {
            rel,
            directory,
            fragment,
        },
        Err(ResolveError::Escape) => LinkKind::Failure {
            kind: FindingKind::Path,
            reason: "路径越出仓库",
        },
        Err(ResolveError::Unsupported(reason)) => visible_skip(reason),
    }
}

fn protocol_skip(kind: SkipKind) -> LinkKind {
    LinkKind::Skip {
        kind,
        reason: String::new(),
        visible: false,
    }
}

fn visible_skip(reason: &'static str) -> LinkKind {
    LinkKind::Skip {
        kind: SkipKind::Unsupported,
        reason: reason.to_string(),
        visible: true,
    }
}

fn scheme_of(dest: &str) -> Option<&str> {
    let bytes = dest.as_bytes();
    if !bytes.first().is_some_and(|byte| byte.is_ascii_alphabetic()) {
        return None;
    }
    let mut index = 1;
    while index < bytes.len()
        && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'+' | b'.' | b'-'))
    {
        index += 1;
    }
    if bytes.get(index) == Some(&b':') {
        Some(&dest[..index])
    } else {
        None
    }
}

fn is_rustdoc_symbol(dest: &str) -> bool {
    dest.contains("::") && !dest.contains('/') && !dest.contains('\\') && !dest.contains('#')
}

fn split_fragment(dest: &str) -> (&str, Option<&str>) {
    match dest.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment)),
        None => (dest, None),
    }
}

/// 行内代码是否精确指向 `target_rel` 与可选 fragment。
pub(super) fn code_span_matches(
    source_rel: &str,
    span: &str,
    target_rel: &str,
    target_fragment: Option<&str>,
) -> bool {
    let span = span.trim();
    if span.is_empty()
        || span.chars().any(char::is_whitespace)
        || span.contains('\\')
        || span.contains('?')
    {
        return false;
    }
    if scheme_of(span).is_some() || is_rustdoc_symbol(span) {
        return false;
    }
    let (path, fragment) = if let Some(fragment) = span.strip_prefix('#') {
        ("", Some(fragment))
    } else {
        split_fragment(span)
    };
    let fragment = match fragment {
        Some(value) => match scan::percent_decode(value) {
            Ok(decoded) if decoded.is_empty() => None,
            Ok(decoded) => Some(decoded),
            Err(()) => return false,
        },
        None => None,
    };
    if fragment.as_deref() != target_fragment && target_fragment.is_some() {
        return false;
    }
    if target_fragment.is_none() && path.is_empty() {
        return false;
    }
    let decoded = if path.is_empty() {
        String::new()
    } else {
        match scan::percent_decode(path) {
            Ok(decoded) => decoded,
            Err(()) => return false,
        }
    };
    if decoded.is_empty() {
        return source_rel == target_rel && fragment.as_deref() == target_fragment;
    }
    let base = scan::parent_rel(source_rel);
    let from_source = scan::resolve_decoded(base, &decoded);
    let from_root = scan::resolve_decoded("", &decoded);
    let matches = |resolved: Result<Resolved, ResolveError>| {
        resolved.is_ok_and(|resolved| {
            resolved.rel == target_rel && fragment_ok(target_fragment, fragment.as_deref())
        })
    };
    matches(from_source) || matches(from_root)
}

fn fragment_ok(required: Option<&str>, actual: Option<&str>) -> bool {
    match required {
        None => true,
        Some(expected) => actual == Some(expected),
    }
}

struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let mut starts = vec![0];
        for (index, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(index + 1);
            }
        }
        Self { starts }
    }

    fn position(&self, text: &str, offset: usize) -> (usize, usize) {
        let offset = offset.min(text.len());
        let line_index = self.starts.partition_point(|start| *start <= offset) - 1;
        let line_start = self.starts[line_index];
        let column = text[line_start..offset].chars().count() + 1;
        (line_index + 1, column)
    }
}

fn push_anchor_ids(html: &str, anchors: &mut BTreeSet<String>, mut in_comment: bool) -> bool {
    let bytes = html.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if in_comment {
            if let Some(relative) = html[index..].find("-->") {
                in_comment = false;
                index += relative + 3;
                continue;
            }
            return true;
        }
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }
        if bytes.get(index..index + 4) == Some(b"<!--") {
            if let Some(relative) = html[index + 4..].find("-->") {
                index = index + 4 + relative + 3;
                continue;
            }
            return true;
        }
        let rest = &html[index + 1..];
        let name_len = rest
            .char_indices()
            .find(|(_, ch)| ch.is_ascii_whitespace() || *ch == '>' || *ch == '/')
            .map(|(offset, _)| offset)
            .unwrap_or(rest.len());
        let name = &rest[..name_len];
        let Some(relative_close) = html[index..].find('>') else {
            break;
        };
        let close = index + relative_close;
        if name.eq_ignore_ascii_case("a") {
            let tag = &html[index..=close];
            for key in ["id", "name"] {
                if let Some(value) = quoted_attr(tag, key)
                    && !value.is_empty()
                {
                    anchors.insert(value);
                }
            }
        }
        index = close + 1;
    }
    in_comment
}

fn quoted_attr(tag: &str, key: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii() {
            index += 1;
            continue;
        }
        if index + key.len() <= bytes.len()
            && tag.is_char_boundary(index)
            && tag.is_char_boundary(index + key.len())
            && tag[index..index + key.len()].eq_ignore_ascii_case(key)
            && attr_boundary(bytes, index)
        {
            let mut cursor = index + key.len();
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if bytes.get(cursor) != Some(&b'=') {
                index += 1;
                continue;
            }
            cursor += 1;
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            let quote = *bytes.get(cursor)?;
            if quote != b'"' && quote != b'\'' {
                return None;
            }
            let start = cursor + 1;
            let end = bytes[start..].iter().position(|byte| *byte == quote)?;
            return Some(tag[start..start + end].to_string());
        }
        index += 1;
    }
    None
}

fn attr_boundary(bytes: &[u8], index: usize) -> bool {
    index > 0 && (bytes[index - 1].is_ascii_whitespace() || bytes[index - 1] == b'<')
}
