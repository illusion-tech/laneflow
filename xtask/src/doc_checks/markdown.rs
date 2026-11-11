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
    // HTML 块按行拆成多次事件。注释和 raw text 元素都可能跨事件。
    let mut html_scan = HtmlScan::default();
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
                Event::Html(html) | Event::InlineHtml(html) => {
                    if in_code == 0 {
                        html_scan = push_anchor_ids(&html, &mut anchors, html_scan);
                    }
                }
                Event::Start(Tag::Link {
                    link_type,
                    dest_url,
                    id,
                    ..
                }) if in_code == 0 => {
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
    // CommonMark 的空目标 `href=""` 指向当前文档。空图片地址仍然无效。
    let kind = if destination.dest_url.is_empty()
        && !destination.image
        && !matches!(
            destination.link_type,
            LinkType::ReferenceUnknown | LinkType::CollapsedUnknown
        ) {
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

#[derive(Clone, Debug, Default)]
struct HtmlScan {
    in_comment: bool,
    raw: Option<String>,
    template_depth: u32,
    plaintext: bool,
}

fn push_anchor_ids(html: &str, anchors: &mut BTreeSet<String>, mut scan: HtmlScan) -> HtmlScan {
    let bytes = html.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if scan.plaintext {
            return scan;
        }
        if let Some(name) = scan.raw.clone() {
            if let Some(after) = find_raw_end(html, index, &name) {
                scan.raw = None;
                index = after;
                continue;
            }
            return scan;
        }
        if scan.in_comment {
            if let Some(relative) = html[index..].find("-->") {
                scan.in_comment = false;
                index += relative + 3;
                continue;
            }
            return scan;
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
            scan.in_comment = true;
            return scan;
        }
        let closing = bytes.get(index + 1) == Some(&b'/');
        let name_at = index + 1 + usize::from(closing);
        if name_at > html.len() {
            break;
        }
        let rest = &html[name_at..];
        let name_len = rest
            .char_indices()
            .find(|(_, ch)| ch.is_ascii_whitespace() || *ch == '>' || *ch == '/')
            .map(|(offset, _)| offset)
            .unwrap_or(rest.len());
        let name = &rest[..name_len];
        let Some(close) = tag_end(html, index) else {
            break;
        };
        // HTML 不把这些非空元素的 `/>` 当成结束。plaintext 没有结束标签。
        let raw_text = matches!(
            name.to_ascii_lowercase().as_str(),
            "script" | "style" | "textarea" | "title" | "xmp" | "iframe" | "noembed" | "noframes"
        );
        if !closing && name.eq_ignore_ascii_case("template") {
            scan.template_depth = scan.template_depth.saturating_add(1);
        } else if closing && name.eq_ignore_ascii_case("template") {
            scan.template_depth = scan.template_depth.saturating_sub(1);
        } else if !closing && name.eq_ignore_ascii_case("plaintext") {
            scan.plaintext = true;
        } else if !closing && raw_text {
            scan.raw = Some(name.to_ascii_lowercase());
        } else if !closing && scan.template_depth == 0 && name.eq_ignore_ascii_case("a") {
            anchors.extend(
                quoted_anchor_values(&html[index..=close])
                    .into_iter()
                    .map(|value| decode_html_entities(&value))
                    .filter(|value| !value.is_empty()),
            );
        }
        index = close + 1;
    }
    scan
}

fn find_raw_end(html: &str, start: usize, name: &str) -> Option<usize> {
    let needle = format!("</{name}");
    let bytes = html.as_bytes();
    let mut index = start;
    while index < bytes.len() {
        let lower = html[index..].to_ascii_lowercase();
        let relative = lower.find(&needle)?;
        let after_name = index + relative + needle.len();
        let boundary = bytes.get(after_name).copied().unwrap_or(b'>');
        if boundary.is_ascii_whitespace() || boundary == b'>' || boundary == b'/' {
            let close = tag_end(html, index + relative)?;
            return Some(close + 1);
        }
        index = after_name;
    }
    None
}

fn decode_html_entities(value: &str) -> String {
    let mut decoded = String::new();
    let mut index = 0;
    while index < value.len() {
        let Some(start) = value[index..].find('&') else {
            decoded.push_str(&value[index..]);
            break;
        };
        let at = index + start;
        decoded.push_str(&value[index..at]);
        if let Some((ch, consumed)) = decode_numeric_reference(&value[at..]) {
            decoded.push(ch);
            index = at + consumed;
            continue;
        }
        let Some(end) = value[at + 1..].find(';') else {
            decoded.push('&');
            index = at + 1;
            continue;
        };
        let entity = &value[at + 1..at + 1 + end];
        if let Some(ch) = decode_html_entity(entity) {
            decoded.push(ch);
            index = at + 1 + end + 1;
        } else {
            decoded.push('&');
            index = at + 1;
        }
    }
    decoded
}

/// 数字字符引用可以不写分号。`&#65` 与 `&#65;` 都是 `A`。
fn decode_numeric_reference(input: &str) -> Option<(char, usize)> {
    let bytes = input.as_bytes();
    if bytes.first() != Some(&b'&') || bytes.get(1) != Some(&b'#') {
        return None;
    }
    let mut index = 2;
    let hex = matches!(bytes.get(index), Some(b'x' | b'X'));
    if hex {
        index += 1;
    }
    let digits_start = index;
    while index < bytes.len()
        && (if hex {
            bytes[index].is_ascii_hexdigit()
        } else {
            bytes[index].is_ascii_digit()
        })
    {
        index += 1;
    }
    if index == digits_start {
        return None;
    }
    let digits = &input[digits_start..index];
    let code = if hex {
        u32::from_str_radix(digits, 16).ok()?
    } else {
        digits.parse().ok()?
    };
    if bytes.get(index) == Some(&b';') {
        index += 1;
    }
    Some((html_numeric_character(code), index))
}

fn decode_html_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => None,
    }
}

/// HTML 数字字符引用的替换：C1 控制区按 Windows-1252，非法码位换成 U+FFFD。
fn html_numeric_character(code: u32) -> char {
    let replaced = match code {
        0x00 | 0xD800..=0xDFFF | 0x0011_0000.. => 0xFFFD,
        0x80 => 0x20AC,
        0x82 => 0x201A,
        0x83 => 0x0192,
        0x84 => 0x201E,
        0x85 => 0x2026,
        0x86 => 0x2020,
        0x87 => 0x2021,
        0x88 => 0x02C6,
        0x89 => 0x2030,
        0x8A => 0x0160,
        0x8B => 0x2039,
        0x8C => 0x0152,
        0x8E => 0x017D,
        0x91 => 0x2018,
        0x92 => 0x2019,
        0x93 => 0x201C,
        0x94 => 0x201D,
        0x95 => 0x2022,
        0x96 => 0x2013,
        0x97 => 0x2014,
        0x98 => 0x02DC,
        0x99 => 0x2122,
        0x9A => 0x0161,
        0x9B => 0x203A,
        0x9C => 0x0153,
        0x9E => 0x017E,
        0x9F => 0x0178,
        other => other,
    };
    char::from_u32(replaced).unwrap_or('\u{FFFD}')
}

fn tag_end(html: &str, start: usize) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut index = start;
    let mut quote = 0_u8;
    while index < bytes.len() {
        let byte = bytes[index];
        if quote != 0 {
            if byte == quote {
                quote = 0;
            }
            index += 1;
            continue;
        }
        match byte {
            b'"' | b'\'' => quote = byte,
            b'>' => return Some(index),
            _ => {}
        }
        index += 1;
    }
    None
}

/// 只读取 `<a>` 上带引号的 `id` / `name`。引号值内部不再扫描，避免把 `title="id='hidden'"` 当成锚点。
fn quoted_anchor_values(tag: &str) -> Vec<String> {
    let bytes = tag.as_bytes();
    let mut index = 1;
    while index < bytes.len()
        && !bytes[index].is_ascii_whitespace()
        && bytes[index] != b'>'
        && bytes[index] != b'/'
    {
        index += 1;
    }
    let mut values = Vec::new();
    let mut seen_names = BTreeSet::new();
    while index < bytes.len() && bytes[index] != b'>' {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] == b'>' || bytes[index] == b'/' {
            break;
        }
        let name_start = index;
        while index < bytes.len()
            && !bytes[index].is_ascii_whitespace()
            && bytes[index] != b'='
            && bytes[index] != b'>'
            && bytes[index] != b'/'
        {
            index += 1;
        }
        let name = &tag[name_start..index];
        let first = seen_names.insert(name.to_ascii_lowercase());
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            continue;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let quote = bytes[index];
        if quote == b'"' || quote == b'\'' {
            index += 1;
            let start = index;
            while index < bytes.len() && bytes[index] != quote {
                index += 1;
            }
            if first
                && (name.eq_ignore_ascii_case("id") || name.eq_ignore_ascii_case("name"))
                && index > start
                && tag.is_char_boundary(start)
                && tag.is_char_boundary(index)
            {
                values.push(tag[start..index].to_string());
            }
            if index < bytes.len() {
                index += 1;
            }
        } else {
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b'>'
            {
                index += 1;
            }
        }
    }
    values
}
