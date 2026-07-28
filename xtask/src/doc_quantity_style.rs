use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, LineColumn, TokenStream, TokenTree};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr, ExprLit, ExprMacro, Lit, LitStr, Meta, Token};

#[derive(Clone, Debug, Eq, PartialEq)]
struct Violation {
    line: usize,
    token: String,
}

const PROSE_BOUNDARY: char = '\u{fffc}';

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err(
            "用法：cargo +1.96.0 run --locked -p xtask -- check-doc-quantity-style <path...>"
                .to_string(),
        );
    }

    let mut files = BTreeSet::new();
    for target in args {
        collect_document_files(Path::new(target), &mut files)?;
    }

    let mut violation_count = 0_usize;
    for file in &files {
        let content = fs::read_to_string(file)
            .map_err(|error| format!("无法读取文档源文件 `{}`: {error}", file.display()))?;
        let violations = if is_rust_file(file) {
            find_rustdoc_violations(&content, Some(file)).map_err(|error| {
                format!(
                    "无法解析 Rustdoc 源文件 `{}`；数量书写门禁失败关闭: {error}",
                    file.display()
                )
            })?
        } else {
            find_markdown_violations(&content, 1)
        };
        for violation in violations {
            violation_count += 1;
            eprintln!(
                "forbidden reader-facing quantity style: {}:{}: `{}`",
                file.display(),
                violation.line,
                violation.token
            );
        }
    }

    if violation_count == 0 {
        println!(
            "已校验 {} 个 Markdown/Rustdoc 源文件：面向读者的数量使用规范中文或完整十进制",
            files.len()
        );
        Ok(())
    } else {
        Err(format!(
            "文档数量书写检查失败：发现 {violation_count} 处 k/M 缩写或阿拉伯数字与中文数量级混写；请改用规范中文数量或完整十进制数字，精确标识符必须放入代码格式"
        ))
    }
}

fn collect_document_files(path: &Path, files: &mut BTreeSet<PathBuf>) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("无法读取路径 `{}`: {error}", path.display()))?;

    if metadata.file_type().is_symlink() {
        return Err(format!(
            "check-doc-quantity-style 不跟随文档源路径中的符号链接：`{}`",
            path.display()
        ));
    }

    if metadata.is_file() {
        if is_document_source(path) {
            files.insert(path.to_path_buf());
        }
        return Ok(());
    }

    if !metadata.is_dir() {
        return Err(format!("不支持的路径类型：`{}`", path.display()));
    }

    let mut entries = fs::read_dir(path)
        .map_err(|error| format!("无法读取目录 `{}`: {error}", path.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法遍历目录 `{}`: {error}", path.display()))?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        collect_document_files(&entry.path(), files)?;
    }
    Ok(())
}

fn is_document_source(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("rs")
        })
}

fn is_rust_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("rs"))
}

fn find_markdown_violations(content: &str, first_source_line: usize) -> Vec<Violation> {
    find_markdown_violations_with_mapping(content, &SourceLineMapping::Linear { first_source_line })
}

fn find_markdown_violations_with_source_lines(
    content: &str,
    source_lines: &[usize],
) -> Vec<Violation> {
    find_markdown_violations_with_mapping(content, &SourceLineMapping::Explicit { source_lines })
}

enum SourceLineMapping<'lines> {
    Linear { first_source_line: usize },
    Explicit { source_lines: &'lines [usize] },
}

impl SourceLineMapping<'_> {
    fn source_line(&self, logical_line_offset: usize) -> usize {
        match self {
            Self::Linear { first_source_line } => first_source_line + logical_line_offset,
            Self::Explicit { source_lines } => source_lines
                .get(logical_line_offset)
                .copied()
                .or_else(|| source_lines.last().copied())
                .unwrap_or(1),
        }
    }
}

fn find_markdown_violations_with_mapping(
    content: &str,
    source_lines: &SourceLineMapping<'_>,
) -> Vec<Violation> {
    let mut prose_by_line = BTreeMap::<usize, ProseLine>::new();
    let mut rendered_breaks = Vec::<(usize, usize)>::new();
    let mut code_block_depth = 0_usize;
    let mut html_projection = HtmlProjection::default();

    for (event, source_range) in Parser::new_ext(content, Options::all()).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => code_block_depth += 1,
            Event::End(TagEnd::CodeBlock) => {
                code_block_depth = code_block_depth.saturating_sub(1);
            }
            Event::Text(text)
                if code_block_depth == 0 && !html_projection.suppresses_reader_prose() =>
            {
                let logical_line_offset = count_line_breaks(&content[..source_range.start]);
                append_prose_projection(
                    &mut prose_by_line,
                    &text,
                    logical_line_offset,
                    source_lines,
                );
            }
            Event::Code(_)
                if code_block_depth == 0 && !html_projection.suppresses_reader_prose() =>
            {
                let logical_line_offset = count_line_breaks(&content[..source_range.start]);
                let boundary = PROSE_BOUNDARY.to_string();
                append_prose_projection(
                    &mut prose_by_line,
                    &boundary,
                    logical_line_offset,
                    source_lines,
                );
            }
            Event::Html(html) | Event::InlineHtml(html) if code_block_depth == 0 => {
                let logical_line_offset = count_line_breaks(&content[..source_range.start]);
                let projection = html_projection.project(&html);
                append_prose_projection(
                    &mut prose_by_line,
                    &projection.visible_text,
                    logical_line_offset,
                    source_lines,
                );
                rendered_breaks.extend(projection.rendered_breaks.into_iter().filter_map(
                    |(before, after)| {
                        let before = logical_line_offset + before;
                        let after = logical_line_offset + after;
                        (before != after).then_some((before, after))
                    },
                ));
            }
            Event::SoftBreak | Event::HardBreak
                if code_block_depth == 0 && !html_projection.suppresses_reader_prose() =>
            {
                let logical_line_offset = count_line_breaks(&content[..source_range.start]);
                let next_logical_line_offset =
                    logical_line_offset + count_line_breaks(&content[source_range]).max(1);
                rendered_breaks.push((logical_line_offset, next_logical_line_offset));
            }
            _ => {}
        }
    }

    let mut cross_break_tokens = BTreeMap::<usize, Vec<String>>::new();
    for (logical_line, next_logical_line) in rendered_breaks {
        let (Some(before), Some(after)) = (
            prose_by_line.get(&logical_line),
            prose_by_line.get(&next_logical_line),
        ) else {
            continue;
        };
        cross_break_tokens.entry(logical_line).or_default().extend(
            quantity_abbreviations_across_break(&before.text, &after.text),
        );
    }

    let mut violations = Vec::new();
    for (logical_line, prose) in prose_by_line {
        for token in quantity_abbreviations(&prose.text) {
            violations.push(Violation {
                line: prose.source_line,
                token,
            });
        }
        for token in cross_break_tokens.remove(&logical_line).unwrap_or_default() {
            violations.push(Violation {
                line: prose.source_line,
                token,
            });
        }
    }
    violations
}

struct ProseLine {
    source_line: usize,
    text: String,
}

fn append_prose_projection(
    prose_by_line: &mut BTreeMap<usize, ProseLine>,
    prose: &str,
    first_logical_line_offset: usize,
    source_lines: &SourceLineMapping<'_>,
) {
    for (line_offset, line) in prose.split('\n').enumerate() {
        let logical_line = first_logical_line_offset + line_offset;
        prose_by_line
            .entry(logical_line)
            .or_insert_with(|| ProseLine {
                source_line: source_lines.source_line(logical_line),
                text: String::new(),
            })
            .text
            .push_str(line.trim_end_matches('\r'));
    }
}

#[derive(Default)]
struct HtmlProjection {
    suppressed_elements: Vec<String>,
}

struct HtmlProjectionResult {
    visible_text: String,
    rendered_breaks: Vec<(usize, usize)>,
}

impl HtmlProjection {
    fn suppresses_reader_prose(&self) -> bool {
        !self.suppressed_elements.is_empty()
    }

    fn project(&mut self, html: &str) -> HtmlProjectionResult {
        let mut visible = String::new();
        let mut rendered_breaks = Vec::new();
        let mut source_line_offset = 0_usize;
        let mut cursor = 0_usize;

        while cursor < html.len() {
            if html.as_bytes()[cursor] != b'<' {
                let next_tag = html[cursor..]
                    .find('<')
                    .map_or(html.len(), |offset| cursor + offset);
                let text = &html[cursor..next_tag];
                if self.suppressed_elements.is_empty() {
                    append_html_text(
                        &mut visible,
                        text,
                        &mut source_line_offset,
                        &mut rendered_breaks,
                    );
                } else {
                    source_line_offset += preserve_line_breaks(&mut visible, text);
                }
                cursor = next_tag;
                continue;
            }

            let Some(tag) = parse_html_tag(html, cursor) else {
                if self.suppressed_elements.is_empty() {
                    visible.push('<');
                }
                cursor += 1;
                continue;
            };

            let was_suppressed = !self.suppressed_elements.is_empty();
            source_line_offset += preserve_line_breaks(&mut visible, &html[cursor..tag.end]);
            if self.suppressed_elements.last().is_some_and(|element| {
                is_html_raw_text_element(element) && !(tag.is_closing && element == &tag.name)
            }) {
                cursor = tag.end;
                continue;
            }

            let separates_visible_prose = is_exact_identifier_html_element(&tag.name);
            if tag.is_closing {
                if self
                    .suppressed_elements
                    .last()
                    .is_some_and(|element| element == &tag.name)
                {
                    self.suppressed_elements.pop();
                }
            } else if !tag.is_self_closing && suppresses_reader_prose(&tag.name) {
                self.suppressed_elements.push(tag.name.clone());
            }

            if !was_suppressed || self.suppressed_elements.is_empty() {
                if tag.name == "br" {
                    if !visible.ends_with(char::is_whitespace) {
                        visible.push(' ');
                    }
                } else if (is_html_text_boundary(&tag.name) || separates_visible_prose)
                    && !visible.ends_with(PROSE_BOUNDARY)
                {
                    visible.push(PROSE_BOUNDARY);
                }
            }
            cursor = tag.end;
        }

        HtmlProjectionResult {
            visible_text: visible,
            rendered_breaks,
        }
    }
}

struct HtmlTag {
    end: usize,
    name: String,
    is_closing: bool,
    is_self_closing: bool,
}

fn parse_html_tag(html: &str, start: usize) -> Option<HtmlTag> {
    let bytes = html.as_bytes();
    if bytes.get(start) != Some(&b'<') {
        return None;
    }

    if html[start..].starts_with("<!--") {
        let end = html[start + 4..]
            .find("-->")
            .map_or(html.len(), |offset| start + 4 + offset + 3);
        return Some(HtmlTag {
            end,
            name: String::new(),
            is_closing: false,
            is_self_closing: true,
        });
    }

    let mut cursor = start + 1;
    let is_closing = bytes.get(cursor) == Some(&b'/');
    if is_closing {
        cursor += 1;
    }
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }

    if matches!(bytes.get(cursor), Some(b'!') | Some(b'?')) {
        return find_html_tag_end(html, cursor + 1).map(|end| HtmlTag {
            end,
            name: String::new(),
            is_closing,
            is_self_closing: true,
        });
    }

    let name_start = cursor;
    while bytes
        .get(cursor)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b':' | b'_'))
    {
        cursor += 1;
    }
    if cursor == name_start {
        return None;
    }

    let end = find_html_tag_end(html, cursor)?;
    let before_close = html[..end.saturating_sub(1)].trim_end();
    Some(HtmlTag {
        end,
        name: html[name_start..cursor].to_ascii_lowercase(),
        is_closing,
        is_self_closing: before_close.ends_with('/'),
    })
}

fn find_html_tag_end(html: &str, mut cursor: usize) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut quote = None::<u8>;

    while let Some(byte) = bytes.get(cursor).copied() {
        match (quote, byte) {
            (Some(active), current) if active == current => quote = None,
            (None, b'\'' | b'"') => quote = Some(byte),
            (None, b'>') => return Some(cursor + 1),
            _ => {}
        }
        cursor += 1;
    }
    None
}

fn append_html_text(
    visible: &mut String,
    text: &str,
    source_line_offset: &mut usize,
    rendered_breaks: &mut Vec<(usize, usize)>,
) {
    let bytes = text.as_bytes();
    let mut cursor = 0_usize;

    while cursor < bytes.len() {
        if bytes[cursor] == b'&'
            && let Some((decoded, end)) = decode_html_reference(text, cursor)
        {
            if decoded.is_whitespace() {
                visible.push(' ');
            } else {
                visible.push(decoded);
            }
            cursor = end;
            continue;
        }

        let character = text[cursor..]
            .chars()
            .next()
            .expect("cursor is inside a UTF-8 string");
        visible.push(character);
        if character == '\n' {
            rendered_breaks.push((*source_line_offset, *source_line_offset + 1));
            *source_line_offset += 1;
        }
        cursor += character.len_utf8();
    }
}

fn decode_html_reference(text: &str, start: usize) -> Option<(char, usize)> {
    for (entity, decoded) in [
        ("&nbsp;", '\u{00a0}'),
        ("&ensp;", '\u{2002}'),
        ("&emsp;", '\u{2003}'),
        ("&thinsp;", '\u{2009}'),
        ("&hairsp;", '\u{200a}'),
        ("&MediumSpace;", '\u{205f}'),
        ("&Tab;", '\t'),
        ("&NewLine;", '\n'),
    ] {
        if text[start..].starts_with(entity) {
            return Some((decoded, start + entity.len()));
        }
    }

    let bytes = text.as_bytes();
    if bytes.get(start..start + 2)? != b"&#" {
        return None;
    }

    let mut cursor = start + 2;
    let radix = if matches!(bytes.get(cursor), Some(b'x') | Some(b'X')) {
        cursor += 1;
        16
    } else {
        10
    };
    let digits_start = cursor;
    while bytes.get(cursor).is_some_and(|byte| match radix {
        16 => byte.is_ascii_hexdigit(),
        _ => byte.is_ascii_digit(),
    }) {
        cursor += 1;
    }
    if cursor == digits_start {
        return None;
    }

    let value = u32::from_str_radix(&text[digits_start..cursor], radix).ok()?;
    if bytes.get(cursor) == Some(&b';') {
        cursor += 1;
    }
    char::from_u32(value).map(|decoded| (decoded, cursor))
}

fn preserve_line_breaks(output: &mut String, source: &str) -> usize {
    let line_breaks = source.chars().filter(|character| *character == '\n');
    let count = line_breaks.clone().count();
    output.extend(line_breaks);
    count
}

fn suppresses_reader_prose(name: &str) -> bool {
    matches!(name, "code" | "script" | "style")
}

fn is_exact_identifier_html_element(name: &str) -> bool {
    name == "code"
}

fn is_html_raw_text_element(name: &str) -> bool {
    matches!(name, "script" | "style")
}

fn is_html_text_boundary(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "dd"
            | "details"
            | "div"
            | "dl"
            | "dt"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "li"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "summary"
            | "table"
            | "tbody"
            | "td"
            | "tfoot"
            | "th"
            | "thead"
            | "tr"
            | "ul"
    )
}

fn find_rustdoc_violations(
    content: &str,
    source_path: Option<&Path>,
) -> Result<Vec<Violation>, String> {
    let syntax = syn::parse_file(content).map_err(|error| error.to_string())?;
    let mut visitor = RustdocVisitor::new(source_path, content);
    visitor.visit_file(&syntax);
    if !visitor.errors.is_empty() {
        return Err(visitor.errors.join("; "));
    }
    visitor
        .fragments
        .sort_by_key(|fragment| (fragment.group_id, fragment.first_source_line()));

    let mut groups = BTreeMap::<usize, RustdocGroup>::new();
    for fragment in visitor.fragments {
        if let Some(group) = groups.get_mut(&fragment.group_id) {
            group.push(fragment);
        } else {
            groups.insert(fragment.group_id, RustdocGroup::new(fragment));
        }
    }

    Ok(groups
        .into_values()
        .flat_map(|group| {
            find_markdown_violations_with_source_lines(&group.markdown, &group.source_lines)
        })
        .collect())
}

struct RustdocVisitor<'path, 'source> {
    source_path: Option<&'path Path>,
    source_content: &'source str,
    source_index: SourceIndex,
    current_ast_attribute_group: Option<AttributeRun>,
    next_group_id: usize,
    fragments: Vec<RustdocFragment>,
    errors: Vec<String>,
}

struct AttributeRun {
    group_id: usize,
    end_offset: usize,
    is_inner: bool,
}

struct SourceIndex {
    line_starts: Vec<usize>,
}

impl SourceIndex {
    fn new(content: &str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(
            content
                .bytes()
                .enumerate()
                .filter_map(|(offset, byte)| (byte == b'\n').then_some(offset + 1)),
        );
        Self { line_starts }
    }

    fn byte_offset(&self, location: LineColumn, content: &str) -> Option<usize> {
        let line_start = *self.line_starts.get(location.line.checked_sub(1)?)?;
        let line_end = self
            .line_starts
            .get(location.line)
            .copied()
            .unwrap_or(content.len());
        let line = content.get(line_start..line_end)?;
        let relative_offset = line
            .char_indices()
            .map(|(offset, _)| offset)
            .nth(location.column)
            .or_else(|| (line.chars().count() == location.column).then_some(line.len()))?;
        line_start.checked_add(relative_offset)
    }
}

impl<'path, 'source> RustdocVisitor<'path, 'source> {
    fn new(source_path: Option<&'path Path>, source_content: &'source str) -> Self {
        Self {
            source_path,
            source_content,
            source_index: SourceIndex::new(source_content),
            current_ast_attribute_group: None,
            next_group_id: 0,
            fragments: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn next_group_id(&mut self) -> usize {
        let group_id = self.next_group_id;
        self.next_group_id += 1;
        group_id
    }

    fn ast_attribute_group(&mut self, attribute: &Attribute) -> usize {
        let span = attribute.span();
        let is_inner = matches!(attribute.style, syn::AttrStyle::Inner(_));
        let start_offset = self
            .source_index
            .byte_offset(span.start(), self.source_content);
        let end_offset = self
            .source_index
            .byte_offset(span.end(), self.source_content);
        let existing_group = self.current_ast_attribute_group.as_ref().and_then(|run| {
            let start_offset = start_offset?;
            (is_inner == run.is_inner
                && start_offset >= run.end_offset
                && self.source_content[run.end_offset..start_offset]
                    .parse::<TokenStream>()
                    .is_ok_and(|tokens| tokens.is_empty()))
            .then_some(run.group_id)
        });
        let group_id = existing_group.unwrap_or_else(|| self.next_group_id());
        self.current_ast_attribute_group = end_offset.map(|end_offset| AttributeRun {
            group_id,
            end_offset,
            is_inner,
        });
        group_id
    }

    fn push_fragment(&mut self, group_id: usize, markdown: String, source_lines: Vec<usize>) {
        self.fragments.push(RustdocFragment {
            group_id,
            markdown,
            source_lines,
        });
    }

    fn literal_source_lines(meta: &Meta, markdown: &str) -> Vec<usize> {
        let span = meta.span();
        let start_line = source_line(span.start());
        let end_line = source_line(span.end()).max(start_line);
        (0..=count_line_breaks(markdown))
            .map(|offset| (start_line + offset).min(end_line))
            .collect()
    }

    fn include_document(&mut self, meta: &Meta, expression: &ExprMacro, group_id: usize) {
        let attribute_line = source_line(meta.span().start());
        let included_path = match syn::parse2::<LitStr>(expression.mac.tokens.clone()) {
            Ok(path) => path.value(),
            Err(error) => {
                self.errors.push(format!(
                    "第 {attribute_line} 行 `#[doc = include_str!(...)]` 必须使用单个字符串字面量路径: {error}"
                ));
                return;
            }
        };
        let Some(source_path) = self.source_path else {
            self.errors.push(format!(
                "第 {attribute_line} 行 `#[doc = include_str!(...)]` 缺少源文件路径上下文"
            ));
            return;
        };
        let Some(parent) = source_path.parent() else {
            self.errors.push(format!(
                "第 {attribute_line} 行无法解析 Rustdoc include：源文件 `{}` 没有父目录",
                source_path.display()
            ));
            return;
        };
        let target = parent.join(included_path);
        let metadata = match fs::symlink_metadata(&target) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.errors.push(format!(
                    "第 {attribute_line} 行无法读取 Rustdoc include `{}`: {error}",
                    target.display()
                ));
                return;
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            self.errors.push(format!(
                "第 {attribute_line} 行 Rustdoc include 必须是非符号链接的普通文件：`{}`",
                target.display()
            ));
            return;
        }
        match fs::read_to_string(&target) {
            Ok(markdown) => {
                let source_lines = vec![attribute_line; count_line_breaks(&markdown) + 1];
                self.push_fragment(group_id, markdown, source_lines);
            }
            Err(error) => self.errors.push(format!(
                "第 {attribute_line} 行无法读取 Rustdoc include `{}`: {error}",
                target.display()
            )),
        }
    }

    fn process_doc_meta(&mut self, meta: &Meta, allow_include: bool, group_id: usize) {
        let Meta::NameValue(name_value) = meta else {
            return;
        };

        match &name_value.value {
            Expr::Lit(ExprLit {
                lit: Lit::Str(documentation),
                ..
            }) => {
                let markdown = documentation.value();
                let source_lines = Self::literal_source_lines(meta, &markdown);
                self.push_fragment(group_id, markdown, source_lines);
            }
            Expr::Macro(expression) if expression.mac.path.is_ident("include_str") => {
                if allow_include {
                    self.include_document(meta, expression, group_id);
                } else {
                    self.errors.push(format!(
                        "第 {} 行宏 token 中的 `doc = include_str!(...)` 无法静态确定展开文件上下文；数量书写门禁失败关闭",
                        source_line(meta.span().start())
                    ));
                }
            }
            _ => self.errors.push(format!(
                "第 {} 行 `#[doc = ...]` 必须使用字符串字面量或 `include_str!`，避免动态文档绕过数量书写门禁",
                source_line(meta.span().start())
            )),
        }
    }

    fn process_cfg_attr(&mut self, attribute: &Attribute, group_id: usize) {
        let Meta::List(list) = &attribute.meta else {
            self.errors.push(format!(
                "第 {} 行 `cfg_attr` 必须使用列表语法",
                source_line(attribute.span().start())
            ));
            return;
        };
        let nested = match list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) {
            Ok(nested) => nested,
            Err(error) => {
                self.errors.push(format!(
                    "第 {} 行无法解析 `cfg_attr` 嵌套属性；数量书写门禁失败关闭: {error}",
                    source_line(attribute.span().start())
                ));
                return;
            }
        };
        let mut nested = nested.iter();
        if nested.next().is_none() {
            self.errors.push(format!(
                "第 {} 行 `cfg_attr` 缺少条件表达式",
                source_line(attribute.span().start())
            ));
            return;
        }
        for meta in nested {
            self.process_conditional_meta(meta, true, group_id);
        }
    }

    fn process_conditional_meta(&mut self, meta: &Meta, allow_include: bool, group_id: usize) {
        if meta.path().is_ident("doc") {
            self.process_doc_meta(meta, allow_include, group_id);
            return;
        }
        if meta.path().is_ident("cfg_attr") {
            let Meta::List(list) = meta else {
                self.errors.push(format!(
                    "第 {} 行嵌套 `cfg_attr` 必须使用列表语法",
                    source_line(meta.span().start())
                ));
                return;
            };
            let nested = match list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            {
                Ok(nested) => nested,
                Err(error) => {
                    self.errors.push(format!(
                        "第 {} 行无法解析嵌套 `cfg_attr`；数量书写门禁失败关闭: {error}",
                        source_line(meta.span().start())
                    ));
                    return;
                }
            };
            let mut nested = nested.iter();
            if nested.next().is_none() {
                self.errors.push(format!(
                    "第 {} 行嵌套 `cfg_attr` 缺少条件表达式",
                    source_line(meta.span().start())
                ));
                return;
            }
            for nested_meta in nested {
                self.process_conditional_meta(nested_meta, allow_include, group_id);
            }
        }
    }

    fn process_macro_tokens(&mut self, tokens: TokenStream) {
        let tokens = tokens.into_iter().collect::<Vec<_>>();
        let mut cursor = 0_usize;
        let mut attribute_group = None::<usize>;

        while cursor < tokens.len() {
            if let TokenTree::Punct(pound) = &tokens[cursor]
                && pound.as_char() == '#'
            {
                let mut group_index = cursor + 1;
                if let Some(TokenTree::Punct(bang)) = tokens.get(group_index)
                    && bang.as_char() == '!'
                {
                    group_index += 1;
                }
                if let Some(TokenTree::Group(group)) = tokens.get(group_index)
                    && group.delimiter() == Delimiter::Bracket
                {
                    let group_id = *attribute_group.get_or_insert_with(|| self.next_group_id());
                    let attribute_tokens = group.stream();
                    match syn::parse2::<Meta>(attribute_tokens.clone()) {
                        Ok(meta)
                            if meta.path().is_ident("doc") || meta.path().is_ident("cfg_attr") =>
                        {
                            self.process_conditional_meta(&meta, false, group_id);
                        }
                        Ok(_) => {}
                        Err(error) if starts_with_document_meta(&attribute_tokens) => {
                            self.errors.push(format!(
                                "第 {} 行宏 token 中的 `doc`/`cfg_attr` 无法静态解析；数量书写门禁失败关闭: {error}",
                                source_line(group.span().start())
                            ));
                        }
                        Err(_) => {}
                    }
                    cursor = group_index + 1;
                    continue;
                }
            }

            attribute_group = None;
            if let TokenTree::Group(group) = &tokens[cursor] {
                self.process_macro_tokens(group.stream());
            }
            cursor += 1;
        }
    }
}

fn starts_with_document_meta(tokens: &TokenStream) -> bool {
    matches!(
        tokens.clone().into_iter().next(),
        Some(TokenTree::Ident(identifier))
            if identifier == "doc" || identifier == "cfg_attr"
    )
}

struct RustdocFragment {
    group_id: usize,
    markdown: String,
    source_lines: Vec<usize>,
}

impl RustdocFragment {
    fn first_source_line(&self) -> usize {
        self.source_lines.first().copied().unwrap_or(1)
    }
}

struct RustdocGroup {
    markdown: String,
    source_lines: Vec<usize>,
}

impl RustdocGroup {
    fn new(fragment: RustdocFragment) -> Self {
        Self {
            markdown: fragment.markdown,
            source_lines: fragment.source_lines,
        }
    }

    fn push(&mut self, fragment: RustdocFragment) {
        self.markdown.push('\n');
        self.markdown.push_str(&fragment.markdown);
        self.source_lines.extend(fragment.source_lines);
    }
}

impl<'ast> Visit<'ast> for RustdocVisitor<'_, '_> {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        let group_id = self.ast_attribute_group(attribute);
        if attribute.path().is_ident("doc") {
            self.process_doc_meta(&attribute.meta, true, group_id);
            return;
        }
        if attribute.path().is_ident("cfg_attr") {
            self.process_cfg_attr(attribute, group_id);
            return;
        }

        visit::visit_attribute(self, attribute);
    }

    fn visit_macro(&mut self, expression: &'ast syn::Macro) {
        self.process_macro_tokens(expression.tokens.clone());
        visit::visit_macro(self, expression);
    }
}

fn source_line(location: LineColumn) -> usize {
    location.line.max(1)
}

fn count_line_breaks(content: &str) -> usize {
    content.bytes().filter(|byte| *byte == b'\n').count()
}

fn quantity_abbreviations(segment: &str) -> Vec<String> {
    quantity_abbreviation_ranges(segment)
        .into_iter()
        .map(|range| segment[range].to_string())
        .collect()
}

fn quantity_abbreviations_across_break(before: &str, after: &str) -> Vec<String> {
    let mut combined = String::with_capacity(before.len() + 1 + after.len());
    combined.push_str(before);
    let break_start = combined.len();
    combined.push(' ');
    let after_start = combined.len();
    combined.push_str(after);

    quantity_abbreviation_ranges(&combined)
        .into_iter()
        .filter(|range| range.start < break_start && range.end > after_start)
        .map(|range| combined[range].to_string())
        .collect()
}

fn quantity_abbreviation_ranges(segment: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    scan_ascii_quantity_abbreviations(segment, &mut ranges);
    scan_mixed_chinese_quantity_style(segment, &mut ranges);
    ranges
}

fn scan_ascii_quantity_abbreviations(segment: &str, ranges: &mut Vec<Range<usize>>) {
    let bytes = segment.as_bytes();
    let mut cursor = 0;

    while cursor < bytes.len() {
        if !bytes[cursor].is_ascii_digit()
            || cursor
                .checked_sub(1)
                .is_some_and(|index| is_identifier_byte(bytes[index]))
        {
            cursor += 1;
            continue;
        }

        let start = cursor;
        cursor += 1;
        while cursor < bytes.len() && (bytes[cursor].is_ascii_digit() || bytes[cursor] == b'.') {
            cursor += 1;
        }

        let suffix_spacing_start = cursor;
        while let Some(character) = segment[cursor..].chars().next()
            && character.is_whitespace()
        {
            cursor += character.len_utf8();
        }

        let allowed_suffix = if cursor == suffix_spacing_start {
            matches!(bytes.get(cursor), Some(b'k' | b'K' | b'm' | b'M'))
        } else {
            matches!(bytes.get(cursor), Some(b'k' | b'M'))
        };
        if !allowed_suffix {
            continue;
        }
        cursor += 1;

        if cursor < bytes.len() && is_identifier_byte(bytes[cursor]) {
            continue;
        }

        ranges.push(start..cursor);
    }
}

fn scan_mixed_chinese_quantity_style(segment: &str, ranges: &mut Vec<Range<usize>>) {
    let bytes = segment.as_bytes();
    let mut cursor = 0;

    while cursor < bytes.len() {
        if !bytes[cursor].is_ascii_digit()
            || cursor
                .checked_sub(1)
                .is_some_and(|index| is_identifier_byte(bytes[index]) || bytes[index] == b'.')
        {
            cursor += 1;
            continue;
        }

        let start = cursor;
        while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
            cursor += 1;
        }
        if cursor + 1 < bytes.len() && bytes[cursor] == b'.' && bytes[cursor + 1].is_ascii_digit() {
            cursor += 1;
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                cursor += 1;
            }
        }

        while let Some(character) = segment[cursor..].chars().next()
            && character.is_whitespace()
        {
            cursor += character.len_utf8();
        }

        let Some(suffix) = segment.get(cursor..) else {
            continue;
        };
        let Some(first) = suffix.chars().next() else {
            continue;
        };
        if !matches!(first, '十' | '百' | '千' | '万' | '亿') {
            continue;
        }

        let end = cursor + first.len_utf8();
        ranges.push(start..end);
        cursor = end;
    }
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_reader_facing_quantity_abbreviations() {
        assert_eq!(
            find_markdown_violations(
                "目标为 10k，研究边界为 1M，带空格形式为 100 k、1\tM、2\u{00a0}k。\n",
                1
            ),
            vec![
                Violation {
                    line: 1,
                    token: "10k".to_string()
                },
                Violation {
                    line: 1,
                    token: "1M".to_string()
                },
                Violation {
                    line: 1,
                    token: "100 k".to_string()
                },
                Violation {
                    line: 1,
                    token: "1\tM".to_string()
                },
                Violation {
                    line: 1,
                    token: "2\u{00a0}k".to_string()
                }
            ]
        );
    }

    #[test]
    fn spaced_si_units_are_not_quantity_abbreviations() {
        assert!(find_markdown_violations("距离 13.9 m，温度 300 K，时延 100 ms。\n", 1).is_empty());
    }

    #[test]
    fn allows_inline_and_fenced_exact_identifiers() {
        let content = "baseline 为 `run-10k-r1`。\n```text\nrun --scale 100k\n```\n";
        assert!(find_markdown_violations(content, 1).is_empty());
    }

    #[test]
    fn exact_identifier_regions_do_not_join_surrounding_prose() {
        let content = concat!(
            "正文 100`run-id`k 不构成缩写。\n",
            "<div>正文 100<code>run-id</code>k 不构成缩写。</div>\n",
            "正文 100<code>run-id</code>k 仍不构成缩写。\n",
        );
        assert!(find_markdown_violations(content, 1).is_empty());
    }

    #[test]
    fn inline_html_code_suppresses_only_its_own_text() {
        let content = concat!(
            "baseline <code>run-100k-r1</code> 合法。\n",
            "嵌套 <code><span>run-100k-r1</span></code> 也合法，但正文 1M 非法。\n",
        );
        assert_eq!(
            find_markdown_violations(content, 1),
            vec![Violation {
                line: 2,
                token: "1M".to_string()
            }]
        );
    }

    #[test]
    fn allows_indented_and_container_nested_code_blocks() {
        let content = concat!(
            "命令：\n\n",
            "    run --scale 100k\n\n",
            "> ```text\n",
            "> run --scale 100k\n",
            "> ```\n\n",
            "- 示例：\n\n",
            "  ```text\n",
            "  run --scale 100k\n",
            "  ```\n",
        );
        assert!(find_markdown_violations(content, 1).is_empty());
    }

    #[test]
    fn container_code_does_not_hide_following_prose() {
        let content = concat!(
            "> ```text\n",
            "> run --scale 100k\n",
            "> ```\n\n",
            "正文仍禁止 100k。\n",
        );
        assert_eq!(
            find_markdown_violations(content, 1),
            vec![Violation {
                line: 5,
                token: "100k".to_string()
            }]
        );
    }

    #[test]
    fn markdown_breaks_preserve_rendered_quantity_continuity() {
        let content = concat!(
            "目标为 100\n",
            "k 个参与单元。\n\n",
            "研究边界为 1  \n",
            "M 个参与单元。\n\n",
            "HTML 换行为 10<br>k 个参与单元。\n\n",
            "段落边界前的 100\n\n",
            "k 不是同一个数量。\n",
        );
        assert_eq!(
            find_markdown_violations(content, 1),
            vec![
                Violation {
                    line: 1,
                    token: "100 k".to_string()
                },
                Violation {
                    line: 4,
                    token: "1 M".to_string()
                },
                Violation {
                    line: 7,
                    token: "10 k".to_string()
                }
            ]
        );
    }

    #[test]
    fn checks_visible_text_in_raw_html_blocks() {
        let content = concat!(
            "<table data-scale=\"100k\">\n",
            "<tr><td>支持 100k</td></tr>\n",
            "<tr><td><code>run-100k-r1</code></td></tr>\n",
            "</table>\n\n",
            "<details>\n",
            "<summary>研究边界为 1M</summary>\n",
            "<p>完整十进制为 1000000。</p>\n",
            "</details>\n\n",
            "<pre>预格式化正文仍禁止 10k。</pre>\n",
            "<pre><code>run --scale 10k</code></pre>\n",
        );
        assert_eq!(
            find_markdown_violations(content, 1),
            vec![
                Violation {
                    line: 2,
                    token: "100k".to_string()
                },
                Violation {
                    line: 7,
                    token: "1M".to_string()
                },
                Violation {
                    line: 11,
                    token: "10k".to_string()
                }
            ]
        );
    }

    #[test]
    fn raw_html_text_newlines_preserve_rendered_quantity_continuity() {
        let content = concat!(
            "<table>\n",
            "<tr><td>目标为 100\n",
            "k 个参与单元</td></tr>\n",
            "<tr><td>不同单元为 100</td><td>k 不跨单元</td></tr>\n",
            "</table>\n",
        );
        assert_eq!(
            find_markdown_violations(content, 1),
            vec![Violation {
                line: 2,
                token: "100 k".to_string()
            }]
        );
    }

    #[test]
    fn html_markup_does_not_split_visible_quantity_tokens() {
        let content = concat!(
            "<div>支持 100<span></span>k</div>\n\n",
            "正文也禁止 100<strong>k</strong>。\n",
        );
        assert_eq!(
            find_markdown_violations(content, 1),
            vec![
                Violation {
                    line: 1,
                    token: "100k".to_string()
                },
                Violation {
                    line: 3,
                    token: "100k".to_string()
                }
            ]
        );
    }

    #[test]
    fn raw_html_numeric_references_do_not_bypass_quantity_gate() {
        assert_eq!(
            find_markdown_violations(
                "<div>支持 100&#107;，混写 1&nbsp;万，正文 2&nbsp;k。</div>\n",
                1
            ),
            vec![
                Violation {
                    line: 1,
                    token: "100k".to_string()
                },
                Violation {
                    line: 1,
                    token: "2 k".to_string()
                },
                Violation {
                    line: 1,
                    token: "1 万".to_string()
                }
            ]
        );
    }

    #[test]
    fn raw_text_elements_do_not_hide_following_html_prose() {
        let content = concat!(
            "<script>const marker = \"<style>\";</script>\n",
            "<style>.marker::before { content: \"<script>\"; }</style>\n",
            "<p>后续正文仍禁止 100k。</p>\n",
        );
        assert_eq!(
            find_markdown_violations(content, 1),
            vec![Violation {
                line: 3,
                token: "100k".to_string()
            }]
        );
    }

    #[test]
    fn rejects_unformatted_hyphenated_identifier() {
        assert_eq!(
            find_markdown_violations("baseline 为 run-100k-r1。\n", 1),
            vec![Violation {
                line: 1,
                token: "100k".to_string()
            }]
        );
    }

    #[test]
    fn accepts_chinese_and_full_decimal_quantities() {
        assert!(
            find_markdown_violations("一万、十万、一百万；10000、100000、1000000。\n", 1)
                .is_empty()
        );
    }

    #[test]
    fn rejects_mixed_arabic_and_chinese_quantity_styles() {
        assert_eq!(
            find_markdown_violations(
                "目标为 1 万，研究边界为 100 万，抽样 1.5 万，不换行空格为 2\u{00a0}万。\n",
                1
            ),
            vec![
                Violation {
                    line: 1,
                    token: "1 万".to_string()
                },
                Violation {
                    line: 1,
                    token: "100 万".to_string()
                },
                Violation {
                    line: 1,
                    token: "1.5 万".to_string()
                },
                Violation {
                    line: 1,
                    token: "2\u{00a0}万".to_string()
                }
            ]
        );
    }

    #[test]
    fn accepts_versions_followed_by_chinese_quantities() {
        assert!(find_markdown_violations("v0.3 十万与 v0.4 十万。\n", 1).is_empty());
    }

    #[test]
    fn checks_line_block_and_literal_attribute_rustdoc() {
        let content = concat!(
            "//! 一万规模由 `BENCH_100K` 启用。\n",
            "/// 仍禁止 100k prose。\n",
            "/** 块文档仍禁止 1M prose。 */\n",
            "#[doc = \"属性文档仍禁止 10k prose。\"]\n",
            "pub struct Documented;\n",
            "const LABEL: &str = \"100k runtime identifier\";\n",
        );
        assert_eq!(
            find_rustdoc_violations(content, None).expect("valid Rust"),
            vec![
                Violation {
                    line: 2,
                    token: "100k".to_string()
                },
                Violation {
                    line: 3,
                    token: "1M".to_string()
                },
                Violation {
                    line: 4,
                    token: "10k".to_string()
                }
            ]
        );
    }

    #[test]
    fn groups_rustdoc_by_attribute_owner_instead_of_source_adjacency() {
        let content = concat!(
            "//! crate 文档末尾为 100\n",
            "/// k 是首个条目的普通开头。\n",
            "pub struct FirstItem;\n",
            "mod nested {\n",
            "    //! module 文档末尾为 100\n",
            "    /// k 是首个子条目的普通开头。\n",
            "    pub struct FirstChild;\n",
            "}\n",
            "/// 目标为 100\n",
            "#[allow(dead_code)]\n",
            "/// k 个参与单元。\n",
            "pub struct SameOwner;\n",
            "\n",
            "/// 前一条目末尾为 100\n",
            "pub struct PreviousOwner;\n",
            "/// k 是下一条目的普通开头。\n",
            "pub struct NextOwner;\n",
            "#[doc = \"100\\n```text\\nrun-id\\n```\\nk 由代码块分隔。\"]\n",
            "pub struct EscapedMultilineOwner;\n",
        );
        assert_eq!(
            find_rustdoc_violations(content, None).expect("valid owner-grouped Rustdoc"),
            vec![Violation {
                line: 9,
                token: "100 k".to_string()
            }]
        );
    }

    #[test]
    fn checks_inner_block_rustdoc_and_ignores_runtime_strings() {
        let content = concat!(
            "/*! 内部块文档仍禁止 100k prose。 */\n",
            "const LABEL: &str = \"100k runtime identifier\";\n",
        );
        assert_eq!(
            find_rustdoc_violations(content, None).expect("valid Rust"),
            vec![Violation {
                line: 1,
                token: "100k".to_string()
            }]
        );
    }

    #[test]
    fn rustdoc_markdown_code_blocks_keep_exact_identifiers() {
        let content = concat!(
            "/// 示例：\n",
            "///\n",
            "///     run --scale 100k\n",
            "///\n",
            "/// ```text\n",
            "/// run --scale 100k\n",
            "/// ```\n",
            "pub struct Documented;\n",
        );
        assert!(
            find_rustdoc_violations(content, None)
                .expect("valid Rust")
                .is_empty()
        );
    }

    #[test]
    fn invalid_rust_fails_closed() {
        assert!(find_rustdoc_violations("pub struct Broken {", None).is_err());
    }

    #[test]
    fn dynamic_doc_attributes_fail_closed() {
        let content = "#![doc = concat!(\"100\", \"k prose\")]\n";
        assert!(
            find_rustdoc_violations(content, None)
                .expect_err("dynamic Rustdoc must fail closed")
                .contains("避免动态文档绕过数量书写门禁")
        );
    }

    #[test]
    fn checks_conditional_and_nested_conditional_rustdoc() {
        let content = concat!(
            "#[cfg_attr(feature = \"docs\", doc = \"条件文档仍禁止 100k prose。\")]\n",
            "#[cfg_attr(\n",
            "    feature = \"zh-docs\",\n",
            "    cfg_attr(target_os = \"linux\", doc = \"嵌套条件文档仍禁止 1M prose。\")\n",
            ")]\n",
            "pub struct ConditionallyDocumented;\n",
        );
        assert_eq!(
            find_rustdoc_violations(content, None).expect("valid conditional Rustdoc"),
            vec![
                Violation {
                    line: 1,
                    token: "100k".to_string()
                },
                Violation {
                    line: 4,
                    token: "1M".to_string()
                }
            ]
        );
    }

    #[test]
    fn dynamic_conditional_doc_attributes_fail_closed() {
        let content = "#![cfg_attr(feature = \"docs\", doc = concat!(\"100\", \"k prose\"))]\n";
        assert!(
            find_rustdoc_violations(content, None)
                .expect_err("dynamic conditional Rustdoc must fail closed")
                .contains("避免动态文档绕过数量书写门禁")
        );
    }

    #[test]
    fn checks_literal_rustdoc_emitted_from_macro_tokens() {
        let content = concat!(
            "macro_rules! documented {\n",
            "    () => {\n",
            "        /// 宏生成的行文档仍禁止 100k prose。\n",
            "        #[cfg_attr(feature = \"docs\", doc = \"条件宏文档仍禁止 1M prose。\")]\n",
            "        pub struct Generated;\n",
            "    };\n",
            "}\n",
        );
        assert_eq!(
            find_rustdoc_violations(content, None).expect("valid macro-generated Rustdoc"),
            vec![
                Violation {
                    line: 3,
                    token: "100k".to_string()
                },
                Violation {
                    line: 4,
                    token: "1M".to_string()
                }
            ]
        );
    }

    #[test]
    fn dynamic_macro_generated_rustdoc_fails_closed() {
        let content = concat!(
            "macro_rules! documented {\n",
            "    ($documentation:literal) => {\n",
            "        #[doc = $documentation]\n",
            "        pub struct Generated;\n",
            "    };\n",
            "}\n",
        );
        assert!(
            find_rustdoc_violations(content, None)
                .expect_err("dynamic macro-generated Rustdoc must fail closed")
                .contains("宏 token 中的 `doc`/`cfg_attr` 无法静态解析")
        );
    }
}
