use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Alignment {
    Left,
    Right,
    Center,
    None,
}

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let (check, targets) = parse_args(args)?;
    let mut files = BTreeSet::new();
    for target in targets {
        collect_markdown_files(&target, &mut files)?;
    }

    let mut unformatted = Vec::new();
    for file in &files {
        let original = fs::read_to_string(file)
            .map_err(|error| format!("无法读取 Markdown 文件 `{}`: {error}", file.display()))?;
        let formatted = format_markdown(&original);
        if original == formatted {
            continue;
        }

        if check {
            eprintln!("unformatted: {}", file.display());
            unformatted.push(file.clone());
        } else {
            fs::write(file, formatted)
                .map_err(|error| format!("无法写入 Markdown 文件 `{}`: {error}", file.display()))?;
            println!("formatted: {}", file.display());
        }
    }

    if unformatted.is_empty() {
        if check {
            println!("已校验 {} 个 Markdown 文件的表格格式", files.len());
        }
        Ok(())
    } else {
        Err(format!(
            "Markdown 表格格式检查失败：{} 个文件需要格式化",
            unformatted.len()
        ))
    }
}

fn parse_args(args: &[String]) -> Result<(bool, Vec<PathBuf>), String> {
    let mut check = false;
    let mut targets = Vec::new();

    for arg in args {
        match arg.as_str() {
            "--check" if !check => check = true,
            "--check" => return Err("`--check` 只能指定一次".to_string()),
            value if value.starts_with('-') => {
                return Err(format!("未知 format-md-tables 参数：{value}"));
            }
            value => targets.push(PathBuf::from(value)),
        }
    }

    if targets.is_empty() {
        return Err(
            "用法：cargo +1.96.0 run --locked -p xtask -- format-md-tables [--check] <path...>"
                .to_string(),
        );
    }

    Ok((check, targets))
}

fn collect_markdown_files(path: &Path, files: &mut BTreeSet<PathBuf>) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("无法读取路径 `{}`: {error}", path.display()))?;

    if metadata.file_type().is_symlink() {
        return Err(format!(
            "format-md-tables 不跟随符号链接：`{}`",
            path.display()
        ));
    }

    if metadata.is_file() {
        if is_markdown_file(path) {
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
        collect_markdown_files(&entry.path(), files)?;
    }
    Ok(())
}

fn is_markdown_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

fn format_markdown(content: &str) -> String {
    let line_ending = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let lines = content.split(line_ending).collect::<Vec<_>>();
    let mut output = Vec::with_capacity(lines.len());
    let mut index = 0;

    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();

        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let fence = if trimmed.starts_with("```") {
                "```"
            } else {
                "~~~"
            };
            output.push(line.to_string());
            index += 1;

            while index < lines.len() {
                output.push(lines[index].to_string());
                if lines[index].trim_start().starts_with(fence) {
                    index += 1;
                    break;
                }
                index += 1;
            }
            continue;
        }

        if index + 1 < lines.len() && line.contains('|') && is_separator_row(lines[index + 1]) {
            let mut block = Vec::new();
            while index < lines.len() && lines[index].contains('|') {
                block.push(lines[index]);
                index += 1;
            }
            output.extend(format_table(&block));
            continue;
        }

        output.push(line.to_string());
        index += 1;
    }

    output.join(line_ending)
}

fn format_table(lines: &[&str]) -> Vec<String> {
    let Some(separator_index) = lines.iter().position(|line| is_separator_row(line)) else {
        return lines.iter().map(|line| (*line).to_string()).collect();
    };

    let rows = lines
        .iter()
        .map(|line| split_markdown_row(line))
        .collect::<Vec<_>>();
    let column_count = rows.iter().map(Vec::len).max().unwrap_or(0);
    let alignments = (0..column_count)
        .map(|index| {
            rows[separator_index]
                .get(index)
                .map_or(Alignment::None, |cell| parse_alignment(cell))
        })
        .collect::<Vec<_>>();
    let mut column_widths = vec![3; column_count];

    for (row_index, row) in rows.iter().enumerate() {
        if row_index == separator_index {
            continue;
        }
        for (column_index, cell) in row.iter().enumerate() {
            column_widths[column_index] = column_widths[column_index].max(display_width(cell));
        }
    }

    rows.iter()
        .enumerate()
        .map(|(row_index, row)| {
            let cells = (0..column_count)
                .map(|column_index| {
                    if row_index == separator_index {
                        build_separator_cell(column_widths[column_index], alignments[column_index])
                    } else {
                        pad_cell(
                            row.get(column_index).map_or("", String::as_str),
                            column_widths[column_index],
                            alignments[column_index],
                        )
                    }
                })
                .collect::<Vec<_>>();
            format!("| {} |", cells.join(" | "))
        })
        .collect()
}

fn split_markdown_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let stripped = inner.strip_suffix('|').unwrap_or(inner);
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    let mut active_code_ticks = None;
    let mut chars = stripped.chars().peekable();

    while let Some(ch) = chars.next() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        if ch == '\\' {
            current.push(ch);
            escaped = true;
            continue;
        }

        if ch == '`' {
            let mut tick_count = 1;
            current.push(ch);
            while chars.peek() == Some(&'`') {
                current.push(chars.next().expect("peeked backtick must exist"));
                tick_count += 1;
            }
            match active_code_ticks {
                None => active_code_ticks = Some(tick_count),
                Some(active) if active == tick_count => active_code_ticks = None,
                Some(_) => {}
            }
            continue;
        }

        if ch == '|' && active_code_ticks.is_none() {
            cells.push(current.trim().to_string());
            current.clear();
            continue;
        }

        current.push(ch);
    }

    cells.push(current.trim().to_string());
    cells
}

fn is_separator_row(line: &str) -> bool {
    let cells = split_markdown_row(line);
    !cells.is_empty() && cells.iter().all(|cell| is_separator_cell(cell))
}

fn is_separator_cell(cell: &str) -> bool {
    let inner = cell.strip_prefix(':').unwrap_or(cell);
    let inner = inner.strip_suffix(':').unwrap_or(inner);
    !inner.is_empty() && inner.chars().all(|ch| ch == '-')
}

fn parse_alignment(cell: &str) -> Alignment {
    match (cell.starts_with(':'), cell.ends_with(':')) {
        (true, true) => Alignment::Center,
        (false, true) => Alignment::Right,
        (true, false) => Alignment::Left,
        (false, false) => Alignment::None,
    }
}

fn build_separator_cell(width: usize, alignment: Alignment) -> String {
    match alignment {
        Alignment::Left => format!(":{}", "-".repeat(width - 1)),
        Alignment::Right => format!("{}:", "-".repeat(width - 1)),
        Alignment::Center => format!(":{}:", "-".repeat(width.saturating_sub(2).max(1))),
        Alignment::None => "-".repeat(width),
    }
}

fn pad_cell(value: &str, width: usize, alignment: Alignment) -> String {
    let padding = width.saturating_sub(display_width(value));
    match alignment {
        Alignment::Right => format!("{}{value}", " ".repeat(padding)),
        Alignment::Center => {
            let left = padding / 2;
            format!("{}{value}{}", " ".repeat(left), " ".repeat(padding - left))
        }
        Alignment::Left | Alignment::None => format!("{value}{}", " ".repeat(padding)),
    }
}

fn display_width(value: &str) -> usize {
    value
        .chars()
        .map(|ch| usize::from(is_wide(ch as u32)) + 1)
        .sum()
}

fn is_wide(code_point: u32) -> bool {
    matches!(
        code_point,
        0x1100..=0x115F
            | 0x2329
            | 0x232A
            | 0x2E80..=0x303E
            | 0x3040..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0xA4CF
            | 0xA960..=0xA97F
            | 0xAC00..=0xD7FF
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE19
            | 0xFE30..=0xFE6F
            | 0xFF01..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F004..=0x1F0CF
            | 0x1F200..=0x1F2FF
            | 0x20000..=0x2FFFD
            | 0x30000..=0x3FFFD
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let id = TEST_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "laneflow-xtask-markdown-{label}-{}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("test directory must be created");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn formats_alignment_and_wide_characters() {
        let input = "| Name | 描述 | Value |\n| :--- | :---: | ---: |\n| a | 中文 | 12 |\n";
        let expected =
            "| Name | 描述 | Value |\n| :--- | :--: | ----: |\n| a    | 中文 |    12 |\n";

        assert_eq!(format_markdown(input), expected);
        assert_eq!(format_markdown(expected), expected);
    }

    #[test]
    fn keeps_escaped_and_code_span_pipes_inside_cells() {
        let input = "| A | B |\n| --- | --- |\n| a\\|b | ``x|y`` |\n";
        let expected = "| A    | B       |\n| ---- | ------- |\n| a\\|b | ``x|y`` |\n";

        assert_eq!(format_markdown(input), expected);
    }

    #[test]
    fn preserves_fenced_tables_and_formats_following_tables() {
        let input = "```md\n| A | B |\n| --- | --- |\n| x | y |\n```\n\n| A | B |\n| --- | --- |\n| long | y |\n";
        let expected = "```md\n| A | B |\n| --- | --- |\n| x | y |\n```\n\n| A    | B   |\n| ---- | --- |\n| long | y   |\n";

        assert_eq!(format_markdown(input), expected);
    }

    #[test]
    fn preserves_lf_crlf_and_final_newline() {
        let lf = "| A |\n| --- |\n| wide |";
        let crlf = "| A |\r\n| --- |\r\n| wide |\r\n";

        assert_eq!(format_markdown(lf), "| A    |\n| ---- |\n| wide |");
        assert_eq!(
            format_markdown(crlf),
            "| A    |\r\n| ---- |\r\n| wide |\r\n"
        );
    }

    #[test]
    fn check_is_read_only_and_format_recurses_over_markdown_only() {
        let directory = TestDirectory::new("command");
        let nested = directory.path().join("nested");
        fs::create_dir_all(&nested).expect("nested directory must be created");
        let markdown = nested.join("table.md");
        let text = nested.join("table.txt");
        let original = "| A |\n| --- |\n| wide |\n";
        fs::write(&markdown, original).expect("Markdown fixture must be written");
        fs::write(&text, original).expect("text fixture must be written");
        let target = directory.path().to_string_lossy().into_owned();

        let check_error = run(&["--check".to_string(), target.clone()])
            .expect_err("unformatted Markdown must fail check mode");
        assert!(check_error.contains("1 个文件"));
        assert_eq!(
            fs::read_to_string(&markdown).expect("Markdown fixture must remain readable"),
            original
        );

        run(std::slice::from_ref(&target)).expect("format mode must succeed");
        run(&["--check".to_string(), target]).expect("formatted Markdown must pass check mode");
        run(&[
            "--check".to_string(),
            markdown.to_string_lossy().into_owned(),
        ])
        .expect("a formatted file target must pass check mode");
        assert_eq!(
            fs::read_to_string(&markdown).expect("Markdown fixture must remain readable"),
            "| A    |\n| ---- |\n| wide |\n"
        );
        assert_eq!(
            fs::read_to_string(&text).expect("text fixture must remain readable"),
            original
        );
    }

    #[test]
    fn rejects_missing_targets_unknown_flags_and_paths() {
        assert!(parse_args(&[]).is_err());
        assert!(parse_args(&["--unknown".to_string()]).is_err());

        let directory = TestDirectory::new("missing-path");
        let missing = directory.path().join("missing.md");
        let error = run(&[missing.to_string_lossy().into_owned()])
            .expect_err("a missing path must fail before formatting");
        assert!(error.contains("无法读取路径"));
    }
}
