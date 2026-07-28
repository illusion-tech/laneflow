use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Fence {
    marker: char,
    length: usize,
}

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
            find_rustdoc_violations(&content)
        } else {
            find_violations(&content)
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

fn find_violations(content: &str) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut fence = None;

    for (line_index, line) in content.lines().enumerate() {
        if let Some(opening) = opening_fence(line) {
            match fence {
                None => fence = Some(opening),
                Some(current)
                    if opening.marker == current.marker
                        && opening.length >= current.length
                        && is_closing_fence(line, current) =>
                {
                    fence = None;
                }
                Some(_) => {}
            }
            continue;
        }

        if fence.is_some() {
            continue;
        }

        for token in quantity_abbreviations_outside_inline_code(line) {
            violations.push(Violation {
                line: line_index + 1,
                token,
            });
        }
    }

    violations
}

fn find_rustdoc_violations(content: &str) -> Vec<Violation> {
    let mut rustdoc = String::with_capacity(content.len());
    for line in content.lines() {
        let trimmed = line.trim_start();
        let documentation = trimmed
            .strip_prefix("//!")
            .or_else(|| trimmed.strip_prefix("///"));
        if let Some(documentation) = documentation {
            rustdoc.push_str(documentation.strip_prefix(' ').unwrap_or(documentation));
        }
        rustdoc.push('\n');
    }
    find_violations(&rustdoc)
}

fn opening_fence(line: &str) -> Option<Fence> {
    let trimmed = line.trim_start();
    let marker = match trimmed.chars().next()? {
        marker @ ('`' | '~') => marker,
        _ => return None,
    };
    let length = trimmed.chars().take_while(|ch| *ch == marker).count();
    (length >= 3).then_some(Fence { marker, length })
}

fn is_closing_fence(line: &str, fence: Fence) -> bool {
    let trimmed = line.trim_start();
    let marker_length = trimmed.chars().take_while(|ch| *ch == fence.marker).count();
    marker_length >= fence.length && trimmed[marker_length..].trim().is_empty()
}

fn quantity_abbreviations_outside_inline_code(line: &str) -> Vec<String> {
    let bytes = line.as_bytes();
    let mut tokens = Vec::new();
    let mut cursor = 0;

    while cursor < bytes.len() {
        if bytes[cursor] == b'`' {
            let delimiter_length = backtick_run_length(bytes, cursor);
            let content_start = cursor + delimiter_length;
            if let Some(closing_start) =
                find_matching_backtick_run(bytes, content_start, delimiter_length)
            {
                cursor = closing_start + delimiter_length;
                continue;
            }
        }

        let segment_end = bytes[cursor..]
            .iter()
            .position(|byte| *byte == b'`')
            .map_or(bytes.len(), |offset| cursor + offset);
        scan_segment(&line[cursor..segment_end], &mut tokens);
        cursor = if segment_end == cursor {
            cursor + backtick_run_length(bytes, cursor)
        } else {
            segment_end
        };
    }

    tokens
}

fn backtick_run_length(bytes: &[u8], start: usize) -> usize {
    bytes[start..]
        .iter()
        .take_while(|byte| **byte == b'`')
        .count()
}

fn find_matching_backtick_run(
    bytes: &[u8],
    mut cursor: usize,
    delimiter_length: usize,
) -> Option<usize> {
    while cursor < bytes.len() {
        if bytes[cursor] != b'`' {
            cursor += 1;
            continue;
        }
        let run_length = backtick_run_length(bytes, cursor);
        if run_length == delimiter_length {
            return Some(cursor);
        }
        cursor += run_length;
    }
    None
}

fn scan_segment(segment: &str, tokens: &mut Vec<String>) {
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

    scan_mixed_chinese_quantity_style(segment, tokens);
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
            find_violations("目标为 10k，研究边界为 1M。\n"),
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
    fn allows_exact_identifiers_in_code_format() {
        let content = "baseline 为 `run-10k-r1`。\n```text\nrun --scale 100k\n```\n";
        assert!(find_violations(content).is_empty());
    }

    #[test]
    fn rejects_unformatted_hyphenated_identifier() {
        assert_eq!(
            find_violations("baseline 为 run-100k-r1。\n"),
            vec![Violation {
                line: 1,
                token: "100k".to_string()
            }]
        );
    }

    #[test]
    fn accepts_chinese_and_full_decimal_quantities() {
        assert!(find_violations("一万、十万、一百万；10000、100000、1000000。\n").is_empty());
    }

    #[test]
    fn rejects_mixed_arabic_and_chinese_quantity_styles() {
        assert_eq!(
            find_violations("目标为 1 万，研究边界为 100 万，抽样 1.5 万。\n"),
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
        assert!(find_violations("v0.3 十万与 v0.4 十万。\n").is_empty());
    }

    #[test]
    fn checks_rustdoc_without_treating_runtime_strings_as_documentation() {
        let content = concat!(
            "//! 十万规模由 `BENCH_100K` 启用。\n",
            "/// 仍禁止 100k prose。\n",
            "const LABEL: &str = \"100k runtime identifier\";\n",
        );
        assert_eq!(
            find_rustdoc_violations(content),
            vec![Violation {
                line: 2,
                token: "100k".to_string()
            }]
        );
    }
}
