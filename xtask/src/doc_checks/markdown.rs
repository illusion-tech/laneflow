//! 用 pulldown-cmark 抽取链接、图片、标题和行内代码。不扫描 `](...)` 正则。
//! Markdown 事件里的 HTML 交给 `html5gum` 分词。

use std::cell::RefCell;
use std::collections::BTreeSet;

use html5gum::{StartTag, State, Token, Tokenizer};
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
    pub(super) image: bool,
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
    let mut in_image = 0_u32;
    // pulldown-cmark 会把同一段 HTML 拆成多次事件。先按文档顺序拼起来，
    // 再交给同一个分词器；事件边界不能当成 EOF，否则跨事件的注释和原始文本会断开。
    let mut html_bytes = Vec::new();
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
                    if in_code == 0 && in_image == 0 {
                        code_spans.push(text.to_string());
                    }
                }
                Event::InlineMath(text) | Event::DisplayMath(text) if in_heading => {
                    heading.push_str(&text);
                }
                Event::SoftBreak | Event::HardBreak if in_heading => heading.push(' '),
                Event::Html(html) | Event::InlineHtml(html) if in_code == 0 && in_image == 0 => {
                    html_bytes.extend_from_slice(html.as_bytes());
                }
                Event::Start(Tag::Link {
                    link_type,
                    dest_url,
                    id,
                    ..
                }) if in_code == 0 && in_image == 0 => {
                    record_link(
                        &mut links,
                        &lines,
                        text,
                        source_rel,
                        range.start,
                        Destination {
                            link_type,
                            dest_url: &dest_url,
                            id: &id,
                            image: false,
                        },
                    );
                }
                Event::Start(Tag::Image {
                    link_type,
                    dest_url,
                    id,
                    ..
                }) => {
                    in_image += 1;
                    if in_code != 0 {
                        continue;
                    }
                    record_link(
                        &mut links,
                        &lines,
                        text,
                        source_rel,
                        range.start,
                        Destination {
                            link_type,
                            dest_url: &dest_url,
                            id: &id,
                            image: true,
                        },
                    );
                }
                Event::End(TagEnd::Image) => in_image = in_image.saturating_sub(1),
                _ => {}
            }
        }
    }
    anchors.extend(explicit_html_anchors(&html_bytes));

    for (offset, reference) in broken_links.into_inner() {
        let (line, column) = lines.position(text, offset);
        links.push(LinkRec {
            line,
            column,
            raw: reference,
            image: false,
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

struct Destination<'a> {
    link_type: LinkType,
    dest_url: &'a str,
    id: &'a str,
    image: bool,
}

fn record_link(
    links: &mut Vec<LinkRec>,
    lines: &LineIndex,
    text: &str,
    source_rel: &str,
    offset: usize,
    destination: Destination<'_>,
) {
    if matches!(destination.link_type, LinkType::ShortcutUnknown) {
        return;
    }
    let (line, column) = lines.position(text, offset);
    let raw = if destination.dest_url.is_empty() {
        destination.id.to_string()
    } else {
        destination.dest_url.to_string()
    };
    // 围栏外的行内链接、行内图片和自动链接直接失败，不再解释目标。
    // 页脚引用的空目标 `href=""` 指向当前文档。空图片地址仍然无效。
    let kind =
        if let Some(reason) = disallowed_inline_reason(destination.link_type, destination.image) {
            LinkKind::Failure {
                kind: FindingKind::Inline,
                reason,
            }
        } else if destination.dest_url.is_empty()
            && !destination.image
            && !matches!(
                destination.link_type,
                LinkType::ReferenceUnknown | LinkType::CollapsedUnknown
            )
        {
            LinkKind::Local {
                rel: source_rel.to_string(),
                directory: false,
                fragment: None,
            }
        } else {
            classify_destination(destination.link_type, destination.dest_url, source_rel)
        };
    links.push(LinkRec {
        line,
        column,
        raw,
        image: destination.image,
        kind,
    });
}

fn parser_options() -> Options {
    // 不打开 ENABLE_HEADING_ATTRIBUTES：`{#id}` 只是标题文本，不是自定义锚点。
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

fn disallowed_inline_reason(link_type: LinkType, image: bool) -> Option<&'static str> {
    match link_type {
        LinkType::Inline if image => Some("围栏外不允许行内图片"),
        LinkType::Inline => Some("围栏外不允许行内链接"),
        LinkType::Autolink | LinkType::Email => Some("围栏外不允许自动链接"),
        _ => None,
    }
}

fn classify_destination(link_type: LinkType, dest: &str, source_rel: &str) -> LinkKind {
    match link_type {
        LinkType::ShortcutUnknown => protocol_skip(SkipKind::Unsupported),
        LinkType::ReferenceUnknown | LinkType::CollapsedUnknown => LinkKind::Failure {
            kind: FindingKind::Path,
            reason: "引用链接没有定义",
        },
        LinkType::WikiLink { .. } => visible_skip("不支持的维基链接语法"),
        LinkType::Reference | LinkType::Collapsed | LinkType::Shortcut => {
            classify_dest_text(dest, source_rel)
        }
        LinkType::Inline | LinkType::Autolink | LinkType::Email => LinkKind::Failure {
            kind: FindingKind::Inline,
            reason: disallowed_inline_reason(link_type, false).expect("inline form"),
        },
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
    if path.starts_with("//") {
        return protocol_skip(SkipKind::Http);
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
    if decoded.starts_with("//") {
        return protocol_skip(SkipKind::Http);
    }
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
        Err(ResolveError::GitMetadata) => LinkKind::Failure {
            kind: FindingKind::Path,
            reason: ".git 不是已提交内容",
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
    if scheme_of(span).is_some() || is_rustdoc_symbol(span) || span.starts_with("//") {
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
    if fragment.as_deref() != target_fragment {
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
    if decoded.starts_with("//") {
        return false;
    }
    if decoded.is_empty() {
        return source_rel == target_rel && fragment.as_deref() == target_fragment;
    }
    let base = scan::parent_rel(source_rel);
    let from_source = scan::resolve_decoded(base, &decoded);
    let from_root = scan::resolve_decoded("", &decoded);
    let matches = |resolved: Result<Resolved, ResolveError>| {
        resolved.is_ok_and(|resolved| {
            !resolved.directory
                && resolved.rel == target_rel
                && fragment_ok(target_fragment, fragment.as_deref())
        })
    };
    matches(from_source) || matches(from_root)
}

fn fragment_ok(required: Option<&str>, actual: Option<&str>) -> bool {
    required == actual
}

struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        // CommonMark 把 `\n`、单独的 `\r` 和 `\r\n` 都当成一次换行。`\r\n` 只记一行。
        let bytes = text.as_bytes();
        let mut starts = vec![0];
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'\n' => {
                    starts.push(index + 1);
                    index += 1;
                }
                b'\r' => {
                    let paired = bytes.get(index + 1) == Some(&b'\n');
                    let next = index + 1 + usize::from(paired);
                    starts.push(next);
                    index = next;
                }
                _ => index += 1,
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

/// 从一整段 HTML 字节里收集 `<a id>` / `<a name>`。
///
/// `html5gum` 只做分词，不会按树构建切换状态。看到会改变分词状态的开始标签后，
/// 立刻 `set_state`；`/>` 不结束这些非 void 元素。`noscript` 只在启用脚本时才是
/// raw text，这里不启用脚本。`<template>` 里的标记仍会被分词，但不登记为锚点。
fn explicit_html_anchors(html: &[u8]) -> BTreeSet<String> {
    let mut anchors = BTreeSet::new();
    if html.is_empty() {
        return anchors;
    }
    let mut template_depth = 0_u32;
    let mut tokenizer = Tokenizer::new(html);
    while let Some(token) = tokenizer.next() {
        let token = match token {
            Ok(token) => token,
            Err(error) => match error {},
        };
        match token {
            Token::StartTag(tag) => {
                if let Some(state) = element_tokenizer_state(&tag.name) {
                    tokenizer.set_state(state);
                }
                note_anchor_start_tag(&tag, &mut template_depth, &mut anchors);
            }
            Token::EndTag(tag) if tag.name == b"template" => {
                template_depth = template_depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    anchors
}

fn element_tokenizer_state(name: &[u8]) -> Option<State> {
    match name {
        b"textarea" | b"title" => Some(State::RcData),
        b"plaintext" => Some(State::PlainText),
        b"script" => Some(State::ScriptData),
        b"style" | b"xmp" | b"iframe" | b"noembed" | b"noframes" => Some(State::RawText),
        _ => None,
    }
}

fn note_anchor_start_tag(
    tag: &StartTag<()>,
    template_depth: &mut u32,
    anchors: &mut BTreeSet<String>,
) {
    if tag.name == b"template" {
        *template_depth = template_depth.saturating_add(1);
        return;
    }
    if *template_depth > 0 || tag.name != b"a" {
        return;
    }
    for name in [b"id".as_slice(), b"name"] {
        let Some(value) = tag.attributes.get(name) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        anchors.insert(String::from_utf8_lossy(value).into_owned());
    }
}
