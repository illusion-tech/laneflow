//! Wire / toolchain 审计边界（#376）。
//!
//! 在两个 wire 家族的 codegen clean-regeneration 检查之外，独立审计工具链侧通道：
//! Cargo.lock resolved graph 钉版（version/source/checksum）、wire manifest 显式
//! target（含 TOML 点号键写法）不得逃逸 package 根目录且不得引入 build 脚本、
//! Rust 源经 include 宏（含注释间隔形态；use 别名导入一律拒绝）与 `#[path]` 属性
//! 引入的目标必须是 `.rs` 且 canonicalize 后仍在 workspace package 根目录内、仓库
//! Cargo 配置与 workflow rustflags（含 TOML 多行数组 / 三引号字符串与 YAML 块标量）
//! 不得削弱 workspace `unsafe_code = "forbid"` 边界。

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use crate::schema_codegen;

const FLATBUFFERS_LOCK_SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";
const FLATBUFFERS_LOCK_CHECKSUM: &str =
    "35f6839d7b3b98adde531effaf34f0c2badc6f4735d26fe74709d8e513a96ef3";

const WIRE_TARGET_SECTIONS: [&str; 5] = ["lib", "bin", "test", "bench", "example"];

/// 出现在 rustflags 值中即判定削弱 `unsafe_code = "forbid"` 的 token（小写匹配）。
/// `--force-warn` 会把 `#![forbid(unsafe_code)]` 降级为 warning 并以零状态码退出
/// （已用 rustc 实测确认），与 allow 类削弱同等处理。
const RUSTFLAGS_WEAKENING_TOKENS: [&str; 7] = [
    "--cap-lints",
    "-a unsafe",
    "-aunsafe",
    "--allow unsafe",
    "--allow=unsafe",
    "--force-warn unsafe",
    "--force-warn=unsafe",
];

pub(crate) fn run() -> Result<(), String> {
    let repository_root =
        std::env::current_dir().map_err(|error| format!("无法解析仓库根目录: {error}"))?;
    require_repository_root(&repository_root)?;
    check_flatbuffers_lockfile_pin(&repository_root)?;
    check_wire_manifest_targets(&repository_root)?;
    check_source_includes(&repository_root)?;
    check_rustflags_configs(&repository_root)?;
    println!(
        "wire 工具链审计已通过：Cargo.lock flatbuffers resolved 钉版闭合，wire manifest target 边界闭合，Rust 源 include 宏 / #[path] 属性目标闭合，Cargo 配置与 workflow rustflags 无削弱"
    );
    Ok(())
}

fn require_repository_root(root: &Path) -> Result<(), String> {
    for path in ["Cargo.toml", "Cargo.lock"] {
        if !root.join(path).is_file() {
            return Err(format!("必须从 LaneFlow 仓库根目录运行；缺少 `{path}`"));
        }
    }
    Ok(())
}

fn check_flatbuffers_lockfile_pin(repository_root: &Path) -> Result<(), String> {
    let lock_text = fs::read_to_string(repository_root.join("Cargo.lock"))
        .map_err(|error| format!("无法读取 workspace Cargo.lock: {error}"))?;
    require_flatbuffers_lockfile_pin(&lock_text)
}

/// 断言 workspace Cargo.lock 中 flatbuffers 恰好有一条 resolved 记录，且
/// version/source/checksum 与钉版常量完全一致。`research/` 下的独立 lock 不属于
/// 本 workspace，不在此扫描。
fn require_flatbuffers_lockfile_pin(lock_text: &str) -> Result<(), String> {
    let mut found = 0usize;
    for block in lock_text.split("[[package]]").skip(1) {
        if lock_string_value(block, "name") != Some("flatbuffers") {
            continue;
        }
        found += 1;
        let version = lock_string_value(block, "version")
            .ok_or_else(|| "Cargo.lock 的 flatbuffers package 缺少 version".to_string())?;
        if version != schema_codegen::FLATBUFFERS_VERSION {
            return Err(format!(
                "Cargo.lock flatbuffers resolved version 不匹配：预期 `{}`，实际 `{version}`",
                schema_codegen::FLATBUFFERS_VERSION
            ));
        }
        let source = lock_string_value(block, "source").ok_or_else(|| {
            "Cargo.lock 的 flatbuffers package 缺少 source（必须来自 crates.io registry）"
                .to_string()
        })?;
        if source != FLATBUFFERS_LOCK_SOURCE {
            return Err(format!(
                "Cargo.lock flatbuffers source 不匹配：预期 `{FLATBUFFERS_LOCK_SOURCE}`，实际 `{source}`"
            ));
        }
        let checksum = lock_string_value(block, "checksum")
            .ok_or_else(|| "Cargo.lock 的 flatbuffers package 缺少 checksum".to_string())?;
        if checksum != FLATBUFFERS_LOCK_CHECKSUM {
            return Err(format!(
                "Cargo.lock flatbuffers checksum 不匹配：预期 `{FLATBUFFERS_LOCK_CHECKSUM}`，实际 `{checksum}`；升级钉版时必须同步更新 xtask 审计常量"
            ));
        }
    }
    match found {
        0 => Err("Cargo.lock 缺少 flatbuffers package".to_string()),
        1 => Ok(()),
        _ => Err(format!(
            "Cargo.lock 含有 {found} 个 flatbuffers package，resolved 钉版审计要求唯一"
        )),
    }
}

/// 读取 `[[package]]` block 内首个 `key = "..."` 的字符串值。
fn lock_string_value<'a>(block: &'a str, key: &str) -> Option<&'a str> {
    for raw_line in block.lines() {
        let line = raw_line.trim();
        let Some((actual_key, value)) = line.split_once('=') else {
            continue;
        };
        if actual_key.trim() != key {
            continue;
        }
        let value = value.trim();
        return value.strip_prefix('"')?.strip_suffix('"');
    }
    None
}

fn check_wire_manifest_targets(repository_root: &Path) -> Result<(), String> {
    for family in [
        &schema_codegen::ROAD_EDITING,
        &schema_codegen::RUNTIME_SNAPSHOT,
    ] {
        let manifest_text = fs::read_to_string(repository_root.join(family.wire_manifest_path))
            .map_err(|error| format!("无法读取 `{}`: {error}", family.wire_manifest_path))?;
        require_wire_manifest_targets(
            &manifest_text,
            &repository_root.join(family.wire_package_root),
            family.wire_manifest_path,
        )?;
    }
    Ok(())
}

/// 审计单个 wire manifest：`[lib]` / `[[bin]]` / `[[test]]` / `[[bench]]` /
/// `[[example]]` 的显式 `path` canonicalize 后必须仍在 package 根目录内；`[package]`
/// 不得出现 build 脚本键；package 根目录不得存在 build.rs。TOML 点号键
/// （`package.build = "..."`、`lib.path = "..."`，已用 cargo metadata 实测确认会被
/// 接受）在首个表头之前的根区与对应表头写法等效，解析时按段归一化后套用同一检查。
fn require_wire_manifest_targets(
    manifest_text: &str,
    package_root: &Path,
    label: &str,
) -> Result<(), String> {
    let canonical_root = package_root.canonicalize().map_err(|error| {
        format!(
            "无法解析 wire package 根目录 `{}`: {error}",
            package_root.display()
        )
    })?;
    let mut section = "";
    for raw_line in manifest_text.lines() {
        let line = raw_line.split('#').next().unwrap_or_default().trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line.trim_matches(&['[', ']'][..]).trim();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let mut effective_section = section;
        let mut effective_key = key.trim();
        if section.is_empty()
            && let Some((table, leaf)) = split_dotted_key(effective_key)
        {
            effective_section = table;
            effective_key = leaf;
        }
        if effective_section == "package" && effective_key == "build" {
            return Err(format!("wire manifest `{label}` 不得声明 build 脚本键"));
        }
        if WIRE_TARGET_SECTIONS.contains(&effective_section) && effective_key == "path" {
            let relative = value.trim().trim_matches('"');
            let candidate = package_root
                .join(relative)
                .canonicalize()
                .map_err(|error| {
                    format!(
                        "wire manifest `{label}` 的显式 target path `{relative}` 无法解析: {error}"
                    )
                })?;
            if !candidate.starts_with(&canonical_root) {
                return Err(format!(
                    "wire manifest `{label}` 的显式 target path `{relative}` 逃逸 package 根目录"
                ));
            }
        }
    }
    if package_root.join("build.rs").is_file() {
        return Err(format!("wire package `{label}` 不得包含 build.rs"));
    }
    Ok(())
}

/// 把 `package.build` / `lib . path` / `"package"."build"` 形态的 TOML 点号键归一化为
/// （表名，叶子键）；非两段点号键返回 `None`。
fn split_dotted_key(key: &str) -> Option<(&str, &str)> {
    let mut segments = key
        .split('.')
        .map(|segment| segment.trim().trim_matches('"').trim_matches('\'').trim());
    let table = segments.next()?;
    let leaf = segments.next()?;
    if segments.next().is_some() || table.is_empty() || leaf.is_empty() {
        return None;
    }
    Some((table, leaf))
}

fn check_source_includes(repository_root: &Path) -> Result<(), String> {
    let manifests = schema_codegen::workspace_manifest_paths(repository_root)?;
    let mut package_roots: Vec<PathBuf> = Vec::new();
    for manifest in manifests {
        let root = manifest
            .parent()
            .ok_or_else(|| format!("workspace manifest `{}` 没有父目录", manifest.display()))?
            .to_path_buf();
        if !package_roots.contains(&root) {
            package_roots.push(root);
        }
    }
    let mut canonical_roots = Vec::with_capacity(package_roots.len());
    for root in &package_roots {
        canonical_roots.push(root.canonicalize().map_err(|error| {
            format!(
                "无法解析 workspace package 根目录 `{}`: {error}",
                root.display()
            )
        })?);
    }
    let mut sources = Vec::new();
    for root in &package_roots {
        schema_codegen::collect_extension_files(root, OsStr::new("rs"), &mut sources)?;
    }
    for source in sources {
        let text = fs::read_to_string(&source)
            .map_err(|error| format!("无法读取 `{}`: {error}", source.display()))?;
        require_audited_source_includes(&text, &source, &canonical_roots)?;
    }
    Ok(())
}

/// 审计单个 Rust 源文本：include 宏的参数必须是静态字符串字面量、以 `.rs` 结尾，
/// 且 canonicalize 后仍落在任一 workspace package 根目录内（`include_bytes!` /
/// `include_str!` 不引入 Rust 源，不在此列）；`#[path]` 属性适用同一闭合规则。
/// 宏名与 `!` 之间允许空白与注释；`use ... include` 形态的别名导入与任何无法静态
/// 确认目标的用法一律拒绝。
fn require_audited_source_includes(
    text: &str,
    source: &Path,
    package_roots: &[PathBuf],
) -> Result<(), String> {
    let label = source.display().to_string();
    let code = code_mask(text);
    audit_include_macros(text, source, &label, package_roots, &code)?;
    audit_path_attributes(text, source, &label, package_roots, &code)?;
    Ok(())
}

/// 逐字节代码区掩码：`true` 表示该 byte 处于真实代码中（不在行注释、块注释或
/// 字符串 / raw 字符串字面量内）。审计只对代码区 token 生效，字符串与注释内容
/// 不参与判定；字符字面量不掩码（`';'` 无法出现在 use 声明路径内，无规避通道，
/// 而生命周期标注 `'a` 必须保留为代码）。
fn code_mask(text: &str) -> Vec<bool> {
    let bytes = text.as_bytes();
    let mut mask = vec![true; bytes.len()];
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    mask[i] = false;
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                mask[i] = false;
                mask[i + 1] = false;
                i += 2;
                let mut depth = 1usize;
                while i < bytes.len() && depth > 0 {
                    match (bytes.get(i), bytes.get(i + 1)) {
                        (Some(b'/'), Some(b'*')) => {
                            depth += 1;
                            mask[i] = false;
                            mask[i + 1] = false;
                            i += 2;
                        }
                        (Some(b'*'), Some(b'/')) => {
                            depth -= 1;
                            mask[i] = false;
                            mask[i + 1] = false;
                            i += 2;
                        }
                        _ => {
                            mask[i] = false;
                            i += 1;
                        }
                    }
                }
            }
            b'"' => {
                mask[i] = false;
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => {
                            mask[i] = false;
                            if let Some(next) = mask.get_mut(i + 1) {
                                *next = false;
                            }
                            i += 2;
                        }
                        b'"' => {
                            mask[i] = false;
                            i += 1;
                            break;
                        }
                        _ => {
                            mask[i] = false;
                            i += 1;
                        }
                    }
                }
            }
            b'r' => {
                // raw 字符串：`r"..."` / `r#"..."#`（# 数量可变）。
                let mut hashes = 0usize;
                while bytes.get(i + 1 + hashes) == Some(&b'#') {
                    hashes += 1;
                }
                if bytes.get(i + 1 + hashes) == Some(&b'"') {
                    for slot in mask.iter_mut().skip(i).take(1 + hashes + 1) {
                        *slot = false;
                    }
                    i += 1 + hashes + 1;
                    while i < bytes.len() {
                        if bytes[i] == b'"' {
                            let closing =
                                (1..=hashes).all(|offset| bytes.get(i + offset) == Some(&b'#'));
                            mask[i] = false;
                            i += 1;
                            if closing {
                                for slot in mask.iter_mut().skip(i).take(hashes) {
                                    *slot = false;
                                }
                                i += hashes;
                                break;
                            }
                        } else {
                            mask[i] = false;
                            i += 1;
                        }
                    }
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    mask
}

/// 掩码下标区间是否完全处于代码区。
fn is_code(mask: &[bool], start: usize, len: usize) -> bool {
    mask.iter().skip(start).take(len).all(|code| *code)
}

fn audit_include_macros(
    text: &str,
    source: &Path,
    label: &str,
    package_roots: &[PathBuf],
    code: &[bool],
) -> Result<(), String> {
    let bytes = text.as_bytes();
    let mut cursor = 0;
    while let Some(relative) = text[cursor..].find("include") {
        let start = cursor + relative;
        cursor = start + "include".len();
        let before = text[..start].chars().next_back();
        let after = text[cursor..].chars().next();
        if before.is_some_and(schema_codegen::is_identifier_character)
            || after.is_some_and(schema_codegen::is_identifier_character)
            || !is_code(code, start, "include".len())
        {
            continue;
        }
        skip_trivia(bytes, &mut cursor);
        if bytes.get(cursor) != Some(&b'!') {
            if inside_use_statement(text, start, code) {
                return Err(format!(
                    "`{label}` 含有 include 宏的 use 别名导入，别名调用无法静态审计"
                ));
            }
            continue;
        }
        cursor += 1;
        skip_trivia(bytes, &mut cursor);
        if !matches!(bytes.get(cursor), Some(b'(' | b'[' | b'{')) {
            return Err(format!("`{label}` 含有无法静态审计的 include 宏调用"));
        }
        cursor += 1;
        skip_trivia(bytes, &mut cursor);
        let Some((target, next)) = read_string_literal(text, cursor) else {
            return Err(format!(
                "`{label}` 的 include 宏参数不是字符串字面量，无法静态审计"
            ));
        };
        require_contained_rs_target(source, package_roots, &target, label, "include 宏")?;
        cursor = next;
    }
    Ok(())
}

fn audit_path_attributes(
    text: &str,
    source: &Path,
    label: &str,
    package_roots: &[PathBuf],
    code: &[bool],
) -> Result<(), String> {
    let bytes = text.as_bytes();
    let mut cursor = 0;
    while let Some(relative) = text[cursor..].find("#[") {
        let start = cursor + relative;
        cursor = start + 2;
        if !is_code(code, start, 2) {
            continue;
        }
        skip_trivia(bytes, &mut cursor);
        if !text[cursor..].starts_with("path") {
            continue;
        }
        cursor += "path".len();
        let after = text[cursor..].chars().next();
        if after.is_some_and(schema_codegen::is_identifier_character) {
            continue;
        }
        skip_trivia(bytes, &mut cursor);
        if bytes.get(cursor) != Some(&b'=') {
            // rustc 只接受字符串字面量形态的 path 属性，其他形态无法通过编译。
            continue;
        }
        cursor += 1;
        skip_trivia(bytes, &mut cursor);
        let Some((target, next)) = read_string_literal(text, cursor) else {
            return Err(format!(
                "`{label}` 的 #[path] 属性值不是字符串字面量，无法静态审计"
            ));
        };
        require_contained_rs_target(source, package_roots, &target, label, "#[path] 属性")?;
        cursor = next;
    }
    Ok(())
}

/// 断言 include / `#[path]` 目标以 `.rs` 结尾，且相对引入它的源文件解析并
/// canonicalize 后仍落在某个 workspace package 根目录内；绝对路径与 `..` 逃逸由此
/// 一并拒绝。
fn require_contained_rs_target(
    source: &Path,
    package_roots: &[PathBuf],
    target: &str,
    label: &str,
    kind: &str,
) -> Result<(), String> {
    if !target.ends_with(".rs") {
        return Err(format!("`{label}` 的 {kind} 引入非 .rs 目标 `{target}`"));
    }
    let base = source
        .parent()
        .ok_or_else(|| format!("`{label}` 没有父目录，无法解析 {kind} 目标"))?;
    let candidate = base
        .join(target)
        .canonicalize()
        .map_err(|error| format!("`{label}` 的 {kind} 目标 `{target}` 无法解析: {error}"))?;
    if !package_roots.iter().any(|root| candidate.starts_with(root)) {
        return Err(format!(
            "`{label}` 的 {kind} 目标 `{target}` 逃逸 workspace package 根目录"
        ));
    }
    Ok(())
}

/// 判断 `pos` 处的 token 是否处在未闭合的 `use` 声明内：上一个独立 `use` 词比上一个
/// `;` 更晚出现即视为仍在 use 声明（含 use 树 `{...}` 内部）中。`use` 与 `;` 都只计
/// 代码区位置，字符串 / 注释内容不干扰判定。
fn inside_use_statement(text: &str, pos: usize, code: &[bool]) -> bool {
    let prefix = &text[..pos];
    let last_semicolon = prefix
        .bytes()
        .enumerate()
        .rev()
        .find(|(at, byte)| *byte == b';' && code[*at])
        .map(|(at, _)| at);
    let mut last_use = None;
    let mut scan = 0;
    while let Some(relative) = prefix[scan..].find("use") {
        let at = scan + relative;
        scan = at + "use".len();
        let before_ok = prefix[..at]
            .chars()
            .next_back()
            .is_none_or(|ch| !schema_codegen::is_identifier_character(ch));
        let after_ok = prefix[at + "use".len()..]
            .chars()
            .next()
            .is_none_or(|ch| !schema_codegen::is_identifier_character(ch));
        if before_ok && after_ok && is_code(code, at, "use".len()) {
            last_use = Some(at);
        }
    }
    match (last_use, last_semicolon) {
        (Some(use_at), semicolon) => semicolon.is_none_or(|semi| use_at > semi),
        _ => false,
    }
}

/// 跳过空白与 Rust 注释（`//` 行注释与可嵌套的 `/* */` 块注释）。
fn skip_trivia(bytes: &[u8], cursor: &mut usize) {
    loop {
        skip_ascii_whitespace(bytes, cursor);
        if bytes.get(*cursor) == Some(&b'/') && bytes.get(*cursor + 1) == Some(&b'/') {
            while let Some(byte) = bytes.get(*cursor) {
                if *byte == b'\n' {
                    break;
                }
                *cursor += 1;
            }
        } else if bytes.get(*cursor) == Some(&b'/') && bytes.get(*cursor + 1) == Some(&b'*') {
            *cursor += 2;
            let mut depth = 1usize;
            while depth > 0 {
                match (bytes.get(*cursor), bytes.get(*cursor + 1)) {
                    (Some(b'/'), Some(b'*')) => {
                        depth += 1;
                        *cursor += 2;
                    }
                    (Some(b'*'), Some(b'/')) => {
                        depth -= 1;
                        *cursor += 2;
                    }
                    (Some(_), _) => *cursor += 1,
                    (None, _) => break,
                }
            }
        } else {
            return;
        }
    }
}

/// 读取普通或 raw 字符串字面量，返回（值，字面量结束后的下一个 byte 下标）。
fn read_string_literal(text: &str, start: usize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let mut cursor = start;
    let mut hashes = 0usize;
    if bytes.get(cursor) == Some(&b'r') {
        cursor += 1;
        while bytes.get(cursor) == Some(&b'#') {
            hashes += 1;
            cursor += 1;
        }
    }
    if bytes.get(cursor) != Some(&b'"') {
        return None;
    }
    cursor += 1;
    let value_start = cursor;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\\' if hashes == 0 => cursor += 2,
            b'"' => {
                let mut matched = 0usize;
                while matched < hashes && bytes.get(cursor + 1 + matched) == Some(&b'#') {
                    matched += 1;
                }
                if matched == hashes {
                    return Some((text[value_start..cursor].to_string(), cursor + 1 + hashes));
                }
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }
    None
}

fn skip_ascii_whitespace(bytes: &[u8], cursor: &mut usize) {
    while matches!(bytes.get(*cursor), Some(b' ' | b'\t' | b'\r' | b'\n')) {
        *cursor += 1;
    }
}

fn check_rustflags_configs(repository_root: &Path) -> Result<(), String> {
    for relative in [".cargo/config.toml", ".cargo/config"] {
        let path = repository_root.join(relative);
        if path.is_file() {
            let text = fs::read_to_string(&path)
                .map_err(|error| format!("无法读取 `{relative}`: {error}"))?;
            require_rustflags_respect_unsafe_forbid(&text, relative)?;
        }
    }
    let workflows_dir = repository_root.join(".github/workflows");
    let mut workflow_files = Vec::new();
    for entry in fs::read_dir(&workflows_dir)
        .map_err(|error| format!("无法枚举 `{}`: {error}", workflows_dir.display()))?
    {
        let path = entry
            .map_err(|error| format!("无法读取 workflows 目录项: {error}"))?
            .path();
        if path.is_file()
            && matches!(
                path.extension().and_then(OsStr::to_str),
                Some("yml" | "yaml")
            )
        {
            workflow_files.push(path);
        }
    }
    workflow_files.sort_unstable();
    for path in workflow_files {
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("无法读取 `{}`: {error}", path.display()))?;
        require_rustflags_respect_unsafe_forbid(&text, &path.display().to_string())?;
    }
    Ok(())
}

/// 审计单份 Cargo 配置或 workflow 文本：任何 rustflags / RUSTFLAGS 赋值（含 TOML
/// 多行数组与三引号字符串、YAML 块标量）不得包含削弱 `unsafe_code = "forbid"` 的
/// token。YAML 块标量（`|` / `>` 及其 `±` 变体）的续行按缩进判定而非按行尾字符，
/// 键行缩进更浅的行结束标量并作为普通行重新参与判定。
fn require_rustflags_respect_unsafe_forbid(text: &str, label: &str) -> Result<(), String> {
    let mut continuation = RustflagsContinuation::None;
    for (number, raw_line) in text.lines().enumerate() {
        let lower = raw_line.to_lowercase();
        if let RustflagsContinuation::BlockScalar(indent) = continuation {
            let line_indent = raw_line.len() - raw_line.trim_start().len();
            if raw_line.trim().is_empty() || line_indent > indent {
                reject_weakening_token(&lower, label, number)?;
                continue;
            }
            continuation = RustflagsContinuation::None;
        }
        let opens = lower.contains("rustflags");
        let active = opens || !matches!(continuation, RustflagsContinuation::None);
        if active {
            reject_weakening_token(&lower, label, number)?;
        }
        continuation = match continuation {
            RustflagsContinuation::Brackets(balance) => {
                let next = bracket_balance(raw_line, balance);
                if next > 0 {
                    RustflagsContinuation::Brackets(next)
                } else {
                    RustflagsContinuation::None
                }
            }
            RustflagsContinuation::TripleQuote => {
                if raw_line.contains("\"\"\"") {
                    RustflagsContinuation::None
                } else {
                    RustflagsContinuation::TripleQuote
                }
            }
            RustflagsContinuation::None if opens => rustflags_value_continuation(raw_line),
            state => state,
        };
    }
    Ok(())
}

/// rustflags 赋值的续行状态。
enum RustflagsContinuation {
    None,
    /// TOML 数组 / 内联表未闭合的 `[` `{` 括号余额。
    Brackets(i64),
    /// YAML 块标量：键所在行的前导空白宽度。
    BlockScalar(usize),
    /// TOML `"""` 多行字符串未闭合。
    TripleQuote,
}

/// 对含 rustflags 的行分析其值的续行形态：三引号字符串、YAML 块标量指示符、
/// 未闭合括号；行内闭合则为 `None`。
fn rustflags_value_continuation(line: &str) -> RustflagsContinuation {
    let lower = line.to_lowercase();
    let Some(key_at) = lower.find("rustflags") else {
        return RustflagsContinuation::None;
    };
    let after_key = &line[key_at + "rustflags".len()..];
    let Some(separator_at) = after_key.find(['=', ':']) else {
        return RustflagsContinuation::None;
    };
    let value = &after_key[separator_at + 1..];
    if value.matches("\"\"\"").count() % 2 == 1 {
        return RustflagsContinuation::TripleQuote;
    }
    // 去掉行尾注释后再判定块标量指示符（`RUSTFLAGS: >- # 注释`）。
    let without_comment = value.split(" #").next().unwrap_or_default();
    let tail = without_comment.trim_end();
    for indicator in ["|-", "|+", ">-", ">+", "|", ">"] {
        if tail.ends_with(indicator) {
            return RustflagsContinuation::BlockScalar(line.len() - line.trim_start().len());
        }
    }
    let balance = bracket_balance(value, 0);
    if balance > 0 {
        RustflagsContinuation::Brackets(balance)
    } else {
        RustflagsContinuation::None
    }
}

/// 统计 `[` `{` 相对 `]` `}` 的净余额（从 `initial` 累加）。
fn bracket_balance(text: &str, initial: i64) -> i64 {
    text.chars().fold(initial, |balance, ch| match ch {
        '[' | '{' => balance + 1,
        ']' | '}' => balance - 1,
        _ => balance,
    })
}

fn reject_weakening_token(lowered_line: &str, label: &str, number: usize) -> Result<(), String> {
    if let Some(token) = RUSTFLAGS_WEAKENING_TOKENS
        .iter()
        .find(|token| lowered_line.contains(**token))
    {
        return Err(format!(
            "`{label}` 第 {} 行 rustflags 含 `{token}`，会削弱 workspace `unsafe_code = \"forbid\"` 边界",
            number + 1
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flatbuffers_lock_fixture() -> String {
        format!(
            "# This file is automatically @generated by Cargo.\nversion = 4\n\n[[package]]\nname = \"other\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"flatbuffers\"\nversion = \"{}\"\nsource = \"{}\"\nchecksum = \"{}\"\ndependencies = [\n \"bitflags\",\n]\n",
            schema_codegen::FLATBUFFERS_VERSION,
            FLATBUFFERS_LOCK_SOURCE,
            FLATBUFFERS_LOCK_CHECKSUM
        )
    }

    #[test]
    fn lockfile_pin_accepts_current_resolved_flatbuffers() {
        assert_eq!(
            require_flatbuffers_lockfile_pin(&flatbuffers_lock_fixture()),
            Ok(())
        );
    }

    #[test]
    fn lockfile_pin_rejects_version_source_and_checksum_drift() {
        let good = flatbuffers_lock_fixture();
        let bad_version = good.replace(
            &format!("\"{}\"", schema_codegen::FLATBUFFERS_VERSION),
            "\"0.0.0\"",
        );
        assert!(require_flatbuffers_lockfile_pin(&bad_version).is_err());
        let bad_source = good.replace(
            &format!("\"{FLATBUFFERS_LOCK_SOURCE}\""),
            "\"git+https://example.invalid/flatbuffers\"",
        );
        assert!(require_flatbuffers_lockfile_pin(&bad_source).is_err());
        let bad_checksum = good.replace(FLATBUFFERS_LOCK_CHECKSUM, &"0".repeat(64));
        assert!(require_flatbuffers_lockfile_pin(&bad_checksum).is_err());
    }

    #[test]
    fn lockfile_pin_requires_exactly_one_flatbuffers_package() {
        assert!(require_flatbuffers_lockfile_pin("version = 4\n").is_err());
        let doubled = format!("{0}{0}", flatbuffers_lock_fixture());
        assert!(require_flatbuffers_lockfile_pin(&doubled).is_err());
        let missing_checksum: String = flatbuffers_lock_fixture()
            .lines()
            .filter(|line| !line.starts_with("checksum"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(require_flatbuffers_lockfile_pin(&missing_checksum).is_err());
    }

    fn temp_package_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("laneflow-wire-audit-{name}-{}", std::process::id()));
        fs::create_dir_all(root.join("src")).expect("temporary wire package src directory");
        fs::write(root.join("src/lib.rs"), "pub fn placeholder() {}\n")
            .expect("temporary wire lib target");
        root
    }

    #[test]
    fn wire_manifest_targets_accept_contained_lib() {
        let root = temp_package_root("contained");
        let manifest = "[package]\nname = \"fixture\"\n\n[lib]\npath = \"src/lib.rs\"\n";
        assert_eq!(
            require_wire_manifest_targets(manifest, &root, "fixture"),
            Ok(())
        );
        fs::remove_dir_all(&root).expect("remove temporary wire package");
    }

    #[test]
    fn wire_manifest_targets_reject_escaping_path() {
        let sandbox =
            std::env::temp_dir().join(format!("laneflow-wire-audit-escape-{}", std::process::id()));
        let root = sandbox.join("pkg");
        fs::create_dir_all(root.join("src")).expect("temporary wire package src directory");
        fs::write(root.join("src/lib.rs"), "pub fn placeholder() {}\n")
            .expect("temporary wire lib target");
        fs::write(sandbox.join("outside.rs"), "pub fn outside() {}\n")
            .expect("temporary escaping target");

        let manifest = "[package]\nname = \"fixture\"\n\n[lib]\npath = \"../outside.rs\"\n";
        let error = require_wire_manifest_targets(manifest, &root, "fixture").unwrap_err();
        assert!(error.contains("逃逸"));
        fs::remove_dir_all(&sandbox).expect("remove temporary escape sandbox");
    }

    #[test]
    fn wire_manifest_rejects_build_script_key_and_file() {
        let root = temp_package_root("build-script");
        let with_key = "[package]\nname = \"fixture\"\nbuild = \"build.rs\"\n";
        assert!(require_wire_manifest_targets(with_key, &root, "fixture").is_err());

        let clean = "[package]\nname = \"fixture\"\n";
        assert_eq!(
            require_wire_manifest_targets(clean, &root, "fixture"),
            Ok(())
        );
        fs::write(root.join("build.rs"), "fn main() {}\n").expect("temporary build script");
        assert!(require_wire_manifest_targets(clean, &root, "fixture").is_err());
        fs::remove_dir_all(&root).expect("remove temporary wire package");
    }

    #[test]
    fn wire_manifest_rejects_dotted_build_script_key() {
        let root = temp_package_root("dotted-build");
        // cargo metadata 实测确认根区点号键 `package.build` 会生成 custom-build target。
        let dotted = "package.name = \"fixture\"\npackage.build = \"build.rs\"\n";
        assert!(require_wire_manifest_targets(dotted, &root, "fixture").is_err());
        let spaced = "package.name = \"fixture\"\npackage . build = \"build.rs\"\n";
        assert!(require_wire_manifest_targets(spaced, &root, "fixture").is_err());
        let quoted = "\"package\".\"build\" = \"build.rs\"\n";
        assert!(require_wire_manifest_targets(quoted, &root, "fixture").is_err());
        fs::remove_dir_all(&root).expect("remove temporary wire package");
    }

    #[test]
    fn wire_manifest_rejects_dotted_target_path_escape() {
        let sandbox = std::env::temp_dir().join(format!(
            "laneflow-wire-audit-dotted-escape-{}",
            std::process::id()
        ));
        let root = sandbox.join("pkg");
        fs::create_dir_all(root.join("src")).expect("temporary wire package src directory");
        fs::write(root.join("src/lib.rs"), "pub fn placeholder() {}\n")
            .expect("temporary wire lib target");
        fs::write(sandbox.join("outside.rs"), "pub fn outside() {}\n")
            .expect("temporary escaping target");

        let dotted = "package.name = \"fixture\"\nlib.path = \"../outside.rs\"\n";
        let error = require_wire_manifest_targets(dotted, &root, "fixture").unwrap_err();
        assert!(error.contains("逃逸"));
        let contained = "package.name = \"fixture\"\nlib.path = \"src/lib.rs\"\n";
        assert_eq!(
            require_wire_manifest_targets(contained, &root, "fixture"),
            Ok(())
        );
        fs::remove_dir_all(&sandbox).expect("remove temporary dotted escape sandbox");
    }

    fn temp_source_fixture(name: &str, files: &[(&str, &str)]) -> (PathBuf, PathBuf, Vec<PathBuf>) {
        let sandbox = std::env::temp_dir().join(format!(
            "laneflow-wire-audit-source-{name}-{}",
            std::process::id()
        ));
        let root = sandbox.join("pkg");
        let source = root.join("src/lib.rs");
        fs::create_dir_all(source.parent().expect("source parent"))
            .expect("temporary source package src directory");
        for (relative, text) in files {
            let path = sandbox.join(relative);
            fs::create_dir_all(path.parent().expect("fixture parent"))
                .expect("temporary fixture directory");
            fs::write(&path, text).expect("temporary fixture file");
        }
        let canonical_root = root.canonicalize().expect("canonical fixture root");
        (sandbox, source, vec![canonical_root])
    }

    fn assert_source_ok(sandbox: &Path, source: &Path, roots: &[PathBuf], text: &str) {
        assert_eq!(require_audited_source_includes(text, source, roots), Ok(()));
        fs::remove_dir_all(sandbox).expect("remove temporary source sandbox");
    }

    fn assert_source_err(sandbox: &Path, source: &Path, roots: &[PathBuf], text: &str) {
        assert!(require_audited_source_includes(text, source, roots).is_err());
        fs::remove_dir_all(sandbox).expect("remove temporary source sandbox");
    }

    #[test]
    fn source_includes_accept_rs_targets_and_unrelated_tokens() {
        let (sandbox, source, roots) = temp_source_fixture(
            "accept",
            &[
                ("pkg/src/payload.rs", "pub fn payload() {}\n"),
                ("pkg/src/support/helper.rs", "pub fn helper() {}\n"),
            ],
        );
        assert_source_ok(&sandbox, &source, &roots, "mod generated;\nfn main() {}\n");
        let (sandbox, source, roots) =
            temp_source_fixture("accept", &[("pkg/src/payload.rs", "pub fn payload() {}\n")]);
        let include_rs = concat!("include", "!(\"payload.rs\")");
        assert_source_ok(&sandbox, &source, &roots, include_rs);
        let (sandbox, source, roots) =
            temp_source_fixture("accept", &[("pkg/src/payload.rs", "pub fn payload() {}\n")]);
        let comment_gap = concat!("include", " /* 间隔 */ !(\"payload.rs\")");
        assert_source_ok(&sandbox, &source, &roots, comment_gap);
        let (sandbox, source, roots) = temp_source_fixture("accept", &[]);
        let include_bytes = concat!("include", "_bytes!(\"payload.bin\")");
        assert_source_ok(&sandbox, &source, &roots, include_bytes);
        let (sandbox, source, roots) = temp_source_fixture(
            "accept",
            &[("pkg/src/support/helper.rs", "pub fn helper() {}\n")],
        );
        let path_rs = concat!("#[", "path = \"support/helper.rs\"]\nmod helper;");
        assert_source_ok(&sandbox, &source, &roots, path_rs);
        let (sandbox, source, roots) =
            temp_source_fixture("accept", &[("pkg/src/support.rs", "pub fn support() {}\n")]);
        let raw_path_rs = concat!("#[", "path = r\"support.rs\"]\nmod support;");
        assert_source_ok(&sandbox, &source, &roots, raw_path_rs);
    }

    #[test]
    fn source_includes_reject_non_rs_and_unauditable_targets() {
        let (sandbox, source, roots) = temp_source_fixture("reject", &[]);
        let bad_include = concat!("include", "!(\"payload.bin\")");
        assert_source_err(&sandbox, &source, &roots, bad_include);
        let (sandbox, source, roots) = temp_source_fixture("reject", &[]);
        let concat_include = concat!("include", "!(concat!(env!(\"OUT_DIR\"), \"/x.rs\"))");
        assert_source_err(&sandbox, &source, &roots, concat_include);
        let (sandbox, source, roots) = temp_source_fixture("reject", &[]);
        let bare_include = concat!("include", "! module");
        assert_source_err(&sandbox, &source, &roots, bare_include);
        let (sandbox, source, roots) = temp_source_fixture("reject", &[]);
        let bad_path = concat!("#[", "path = \"payload.bin\"]\nmod payload;");
        assert_source_err(&sandbox, &source, &roots, bad_path);
        let (sandbox, source, roots) = temp_source_fixture("reject", &[]);
        let bad_raw_path = concat!("#[", "path = r#\"payload.dat\"#]\nmod payload;");
        assert_source_err(&sandbox, &source, &roots, bad_raw_path);
        let (sandbox, source, roots) = temp_source_fixture("reject", &[]);
        let missing = concat!("include", "!(\"nonexistent.rs\")");
        assert_source_err(&sandbox, &source, &roots, missing);
    }

    #[test]
    fn source_includes_reject_use_alias_and_escapes() {
        // `use std::include as load;` 之后的 `load!(...)` 不再出现 include 字样，
        // 别名导入本身必须被拒绝。
        let (sandbox, source, roots) =
            temp_source_fixture("alias", &[("pkg/src/payload.rs", "pub fn payload() {}\n")]);
        let alias = concat!("use std::", "include as load;\nload!(\"payload.rs\")\n");
        assert_source_err(&sandbox, &source, &roots, alias);
        let (sandbox, source, roots) =
            temp_source_fixture("alias", &[("pkg/src/payload.rs", "pub fn payload() {}\n")]);
        let alias_tree = concat!("use std::{", "include};\n");
        assert_source_err(&sandbox, &source, &roots, alias_tree);

        // include / #[path] 目标逃逸 package 根目录（绝对路径与 .. 形态）。
        let (sandbox, source, roots) =
            temp_source_fixture("escape", &[("outside.rs", "pub fn outside() {}\n")]);
        let escaping = concat!("include", "!(\"../../outside.rs\")");
        assert_source_err(&sandbox, &source, &roots, escaping);
        let (sandbox, source, roots) =
            temp_source_fixture("escape", &[("outside.rs", "pub fn outside() {}\n")]);
        let escaping_path = concat!("#[", "path = \"../../outside.rs\"]\nmod outside;");
        assert_source_err(&sandbox, &source, &roots, escaping_path);
    }

    #[test]
    fn rustflags_audit_accepts_absent_or_benign_flags() {
        assert_eq!(
            require_rustflags_respect_unsafe_forbid("", "fixture"),
            Ok(())
        );
        assert_eq!(
            require_rustflags_respect_unsafe_forbid(
                "[build]\nrustflags = [\"-C\", \"opt-level=3\"]\n",
                "fixture"
            ),
            Ok(())
        );
        assert_eq!(
            require_rustflags_respect_unsafe_forbid("env:\n  RUSTFLAGS: -Dwarnings\n", "workflow"),
            Ok(())
        );
    }

    #[test]
    fn rustflags_audit_rejects_unsafe_weakening() {
        assert!(
            require_rustflags_respect_unsafe_forbid(
                "[build]\nrustflags = [\"--cap-lints\", \"warn\"]\n",
                "fixture"
            )
            .is_err()
        );
        assert!(
            require_rustflags_respect_unsafe_forbid(
                "env:\n  RUSTFLAGS: -A unsafe-code\n",
                "workflow"
            )
            .is_err()
        );
        assert!(
            require_rustflags_respect_unsafe_forbid(
                "rustflags = [\n  \"--cap-lints\",\n  \"allow\"\n]\n",
                "fixture"
            )
            .is_err()
        );
        assert!(
            require_rustflags_respect_unsafe_forbid("RUSTFLAGS: >-\n  -Aunsafe_code\n", "workflow")
                .is_err()
        );
    }

    #[test]
    fn rustflags_audit_rejects_force_warn_downgrade() {
        // rustc 实测：`--force-warn unsafe_code` 使 `#![forbid(unsafe_code)]` 只产生
        // warning 并以零状态码退出，与 allow 类削弱同罪。
        assert!(
            require_rustflags_respect_unsafe_forbid(
                "env:\n  RUSTFLAGS: --force-warn unsafe_code\n",
                "workflow"
            )
            .is_err()
        );
        assert!(
            require_rustflags_respect_unsafe_forbid(
                "rustflags = [\"--force-warn=unsafe-code\"]\n",
                "fixture"
            )
            .is_err()
        );
    }

    #[test]
    fn rustflags_audit_scans_entire_yaml_block_scalar() {
        // YAML 块标量按缩进续行：良性首行之后出现的削弱 token 必须仍被检查。
        assert!(
            require_rustflags_respect_unsafe_forbid(
                "env:\n  RUSTFLAGS: >-\n    -C opt-level=2\n    --cap-lints allow\n",
                "workflow"
            )
            .is_err()
        );
        assert!(
            require_rustflags_respect_unsafe_forbid(
                "env:\n  RUSTFLAGS: | # 保留换行\n    --force-warn unsafe-code\n",
                "workflow"
            )
            .is_err()
        );
        // 缩进回到键级即结束标量；良性块标量整体放行。
        assert_eq!(
            require_rustflags_respect_unsafe_forbid(
                "env:\n  RUSTFLAGS: >-\n    -C opt-level=2\n    -Dwarnings\n  OTHER: 1\n",
                "workflow"
            ),
            Ok(())
        );
    }

    #[test]
    fn rustflags_audit_scans_toml_triple_quote_string() {
        assert!(
            require_rustflags_respect_unsafe_forbid(
                "rustflags = \"\"\"\n--cap-lints allow\n\"\"\"\n",
                "fixture"
            )
            .is_err()
        );
        assert_eq!(
            require_rustflags_respect_unsafe_forbid(
                "rustflags = \"\"\"\n-C opt-level=2\n\"\"\"\n",
                "fixture"
            ),
            Ok(())
        );
    }
}
