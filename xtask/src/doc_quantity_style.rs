use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::LineColumn;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr, ExprLit, ExprMacro, Lit, LitStr, Meta};

#[derive(Clone, Debug, Eq, PartialEq)]
struct Violation {
    line: usize,
    token: String,
}

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
    let mut violations = Vec::new();
    let mut code_block_depth = 0_usize;

    for (event, source_range) in Parser::new_ext(content, Options::all()).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => code_block_depth += 1,
            Event::End(TagEnd::CodeBlock) => {
                code_block_depth = code_block_depth.saturating_sub(1);
            }
            Event::Text(_) if code_block_depth == 0 => {
                let source = &content[source_range.clone()];
                let line = first_source_line + count_line_breaks(&content[..source_range.start]);
                scan_lines(source, line, &mut violations);
            }
            _ => {}
        }
    }

    violations
}

fn find_rustdoc_violations(
    content: &str,
    source_path: Option<&Path>,
) -> Result<Vec<Violation>, String> {
    let syntax = syn::parse_file(content).map_err(|error| error.to_string())?;
    let mut visitor = RustdocVisitor::new(source_path);
    visitor.visit_file(&syntax);
    if !visitor.errors.is_empty() {
        return Err(visitor.errors.join("; "));
    }
    visitor
        .fragments
        .sort_by_key(|fragment| fragment.start_line);

    let mut groups = Vec::<RustdocGroup>::new();
    for fragment in visitor.fragments {
        match groups.last_mut() {
            Some(group) if fragment.start_line <= group.last_source_line.saturating_add(1) => {
                group.push(fragment);
            }
            _ => groups.push(RustdocGroup::new(fragment)),
        }
    }

    let mut violations = groups
        .into_iter()
        .flat_map(|group| find_markdown_violations(&group.markdown, group.first_source_line))
        .collect::<Vec<_>>();
    for included in visitor.included_documents {
        violations.extend(find_markdown_violations(
            &included.markdown,
            included.attribute_line,
        ));
    }
    Ok(violations)
}

struct RustdocVisitor<'path> {
    source_path: Option<&'path Path>,
    fragments: Vec<RustdocFragment>,
    included_documents: Vec<IncludedRustdoc>,
    errors: Vec<String>,
}

impl<'path> RustdocVisitor<'path> {
    fn new(source_path: Option<&'path Path>) -> Self {
        Self {
            source_path,
            fragments: Vec::new(),
            included_documents: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn include_document(&mut self, attribute: &Attribute, expression: &ExprMacro) {
        let attribute_line = source_line(attribute.span().start());
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
            Ok(markdown) => self.included_documents.push(IncludedRustdoc {
                attribute_line,
                markdown,
            }),
            Err(error) => self.errors.push(format!(
                "第 {attribute_line} 行无法读取 Rustdoc include `{}`: {error}",
                target.display()
            )),
        }
    }
}

struct RustdocFragment {
    start_line: usize,
    end_line: usize,
    markdown: String,
}

struct IncludedRustdoc {
    attribute_line: usize,
    markdown: String,
}

struct RustdocGroup {
    first_source_line: usize,
    last_source_line: usize,
    markdown: String,
}

impl RustdocGroup {
    fn new(fragment: RustdocFragment) -> Self {
        Self {
            first_source_line: fragment.start_line,
            last_source_line: fragment.end_line,
            markdown: fragment.markdown,
        }
    }

    fn push(&mut self, fragment: RustdocFragment) {
        let represented_source_line = self.first_source_line + count_line_breaks(&self.markdown);
        let line_gap = fragment.start_line.saturating_sub(represented_source_line);
        for _ in 0..line_gap.max(1) {
            self.markdown.push('\n');
        }
        self.markdown.push_str(&fragment.markdown);
        self.last_source_line = fragment.end_line;
    }
}

impl<'ast> Visit<'ast> for RustdocVisitor<'_> {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        if attribute.path().is_ident("doc") {
            if let Meta::NameValue(meta) = &attribute.meta {
                match &meta.value {
                    Expr::Lit(ExprLit {
                        lit: Lit::Str(documentation),
                        ..
                    }) => {
                        let span = attribute.span();
                        self.fragments.push(RustdocFragment {
                            start_line: source_line(span.start()),
                            end_line: source_line(span.end()),
                            markdown: documentation.value(),
                        });
                    }
                    Expr::Macro(expression) if expression.mac.path.is_ident("include_str") => {
                        self.include_document(attribute, expression);
                    }
                    _ => self.errors.push(format!(
                        "第 {} 行 `#[doc = ...]` 必须使用字符串字面量或 `include_str!`，避免动态文档绕过数量书写门禁",
                        source_line(attribute.span().start())
                    )),
                }
            }
            return;
        }

        visit::visit_attribute(self, attribute);
    }
}

fn source_line(location: LineColumn) -> usize {
    location.line.max(1)
}

fn count_line_breaks(content: &str) -> usize {
    content.bytes().filter(|byte| *byte == b'\n').count()
}

fn scan_lines(content: &str, first_source_line: usize, violations: &mut Vec<Violation>) {
    for (line_offset, line) in content.lines().enumerate() {
        for token in quantity_abbreviations(line) {
            violations.push(Violation {
                line: first_source_line + line_offset,
                token,
            });
        }
    }
}

fn quantity_abbreviations(segment: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    scan_ascii_quantity_abbreviations(segment, &mut tokens);
    scan_mixed_chinese_quantity_style(segment, &mut tokens);
    tokens
}

fn scan_ascii_quantity_abbreviations(segment: &str, tokens: &mut Vec<String>) {
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

        if cursor >= bytes.len() || !matches!(bytes[cursor], b'k' | b'K' | b'm' | b'M') {
            continue;
        }
        cursor += 1;

        if cursor < bytes.len() && is_identifier_byte(bytes[cursor]) {
            continue;
        }

        tokens.push(segment[start..cursor].to_string());
    }
}

fn scan_mixed_chinese_quantity_style(segment: &str, tokens: &mut Vec<String>) {
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

        while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t') {
            cursor += 1;
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
        tokens.push(segment[start..end].to_string());
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
            find_markdown_violations("目标为 10k，研究边界为 1M。\n", 1),
            vec![
                Violation {
                    line: 1,
                    token: "10k".to_string()
                },
                Violation {
                    line: 1,
                    token: "1M".to_string()
                }
            ]
        );
    }

    #[test]
    fn allows_inline_and_fenced_exact_identifiers() {
        let content = "baseline 为 `run-10k-r1`。\n```text\nrun --scale 100k\n```\n";
        assert!(find_markdown_violations(content, 1).is_empty());
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
            find_markdown_violations("目标为 1 万，研究边界为 100 万，抽样 1.5 万。\n", 1),
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
}
