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
/// `[[example]]` 的显式 `path` 必须以 `.rs` 结尾（Cargo 会把任意后缀的显式 target
/// path 按 Rust 源编译），且 canonicalize 后仍在 package 根目录内；`[package]` 不得
/// 出现 build 脚本键；package 根目录不得存在 build.rs。TOML 点号键
/// （`package.build = "..."`、`lib.path = "..."`）、引号键（`["package"]`、
/// `"build" = "..."`）与内联表（`package = { build = "..." }`、
/// `lib = { path = "..." }`、`bin = [{ path = "..." }]`）均已用 cargo metadata 实测
/// 确认会被 Cargo 接受，解析时按段归一化（去引号、拆点号）并下钻内联表后套用同一
/// 检查。
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
        let mut effective_key = unquote_toml_key(key);
        if section.is_empty() {
            if let Some((table, leaf)) = split_dotted_key(key) {
                effective_section = table;
                effective_key = leaf;
            } else {
                // 内联表形态：`package = { build = "..." }` 与
                // `lib = { path = "..." }` / `bin = [{ path = "..." }]`（内联表数组
                // 与 [[bin]] 段在 Cargo 侧反序列化等价）。
                let trimmed_value = value.trim();
                if trimmed_value.starts_with('{') || trimmed_value.starts_with('[') {
                    if effective_key == "package"
                        && inline_table_value(trimmed_value, "build").is_some()
                    {
                        return Err(format!("wire manifest `{label}` 不得声明 build 脚本键"));
                    }
                    if WIRE_TARGET_SECTIONS.contains(&effective_key)
                        && let Some(target_value) = inline_table_value(trimmed_value, "path")
                    {
                        require_manifest_target_contained(
                            package_root,
                            &canonical_root,
                            unquote_toml_key(target_value),
                            label,
                        )?;
                    }
                }
            }
        }
        if section_is(effective_section, "package") && effective_key == "build" {
            return Err(format!("wire manifest `{label}` 不得声明 build 脚本键"));
        }
        if WIRE_TARGET_SECTIONS
            .iter()
            .any(|name| section_is(effective_section, name))
            && effective_key == "path"
        {
            let relative = value.trim().trim_matches('"');
            require_manifest_target_contained(package_root, &canonical_root, relative, label)?;
        }
    }
    if package_root.join("build.rs").is_file() {
        return Err(format!("wire package `{label}` 不得包含 build.rs"));
    }
    Ok(())
}

/// 断言 wire manifest 的显式 target path 以 `.rs` 结尾，且相对 package 根目录解析并
/// canonicalize 后仍落在根目录内；绝对路径与 `..` 逃逸由此一并拒绝。
fn require_manifest_target_contained(
    package_root: &Path,
    canonical_root: &Path,
    relative: &str,
    label: &str,
) -> Result<(), String> {
    if !relative.ends_with(".rs") {
        return Err(format!(
            "wire manifest `{label}` 的显式 target path `{relative}` 不是 .rs 文件"
        ));
    }
    let candidate = package_root
        .join(relative)
        .canonicalize()
        .map_err(|error| {
            format!("wire manifest `{label}` 的显式 target path `{relative}` 无法解析: {error}")
        })?;
    if !candidate.starts_with(canonical_root) {
        return Err(format!(
            "wire manifest `{label}` 的显式 target path `{relative}` 逃逸 package 根目录"
        ));
    }
    Ok(())
}

/// 在 TOML 内联表文本中查找 `key` 的值文本；键不存在返回 `None`。扫描以每个 `{`
/// 起算深度 1，字符串内的 `,` / `=` 与嵌套括号不干扰键定位；`bin = [{ ... }, { ... }]`
/// 这类内联表数组中的每个表都会参与（`,` 在深度 0 时不切分，下一个 `{` 重新开始
/// 计段）。
fn inline_table_value<'a>(value: &'a str, key: &str) -> Option<&'a str> {
    let bytes = value.as_bytes();
    let mut depth = 0i64;
    let mut segment_start = 0usize;
    let mut segments: Vec<&str> = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
            }
            b'\'' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\'' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            b'{' => {
                depth += 1;
                if depth == 1 {
                    segment_start = i + 1;
                }
                i += 1;
            }
            b'}' => {
                if depth == 1 {
                    segments.push(&value[segment_start..i]);
                }
                depth -= 1;
                i += 1;
            }
            b',' if depth == 1 => {
                segments.push(&value[segment_start..i]);
                segment_start = i + 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    for segment in segments {
        let Some((entry_key, entry_value)) = segment.split_once('=') else {
            continue;
        };
        if unquote_toml_key(entry_key) == key {
            return Some(entry_value.trim());
        }
    }
    None
}

/// 归一化 TOML 键 / 表单段：去首尾空白与一层配对引号（`"..."` 或 `'...'`）。
fn unquote_toml_key(segment: &str) -> &str {
    let segment = segment.trim();
    segment
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .or_else(|| {
            segment
                .strip_prefix('\'')
                .and_then(|inner| inner.strip_suffix('\''))
        })
        .unwrap_or(segment)
}

/// 表头名是否等于目标名：只有单段表头参与（`[package.metadata]` 这类多段不匹配），
/// 引号写法（`["package"]`）归一化后比较。
fn section_is(section: &str, name: &str) -> bool {
    let mut segments = section.split('.');
    let Some(only) = segments.next() else {
        return false;
    };
    if segments.next().is_some() {
        return false;
    }
    unquote_toml_key(only) == name
}

/// 把 `package.build` / `lib . path` / `"package"."build"` 形态的 TOML 点号键归一化为
/// （表名，叶子键）；非两段点号键返回 `None`。
fn split_dotted_key(key: &str) -> Option<(&str, &str)> {
    let mut segments = key.split('.').map(unquote_toml_key);
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
/// 且 canonicalize 后仍落在**源文件自己所属的** workspace package 根目录内
/// （`include_bytes!` / `include_str!` 不引入 Rust 源，不在此列）；`#[path]` 属性
/// 与 `#[cfg_attr]` 嵌套的 path 属性适用同一闭合规则，`#` 与 `[` 之间的空白 / 注释
/// 与 `#![...]` 内部属性写法不改变判定。宏名与 `!` 之间允许空白与
/// 注释；`use ... include` 形态的别名导入与任何无法静态确认目标的用法一律拒绝。
fn require_audited_source_includes(
    text: &str,
    source: &Path,
    package_roots: &[PathBuf],
) -> Result<(), String> {
    let label = source.display().to_string();
    // wire crate 的 unsafe 扫描只覆盖各自包根，include 闭合因此也必须限定在
    // 源文件自己所属的 package 根内，而不是任一 workspace package。
    let canonical_source = source
        .canonicalize()
        .map_err(|error| format!("无法解析 `{label}`: {error}"))?;
    let own_root = package_roots
        .iter()
        .find(|root| canonical_source.starts_with(root))
        .ok_or_else(|| format!("`{label}` 不在任何 workspace package 根目录内"))?;
    let code = code_mask(text);
    audit_include_macros(text, source, &label, own_root, &code)?;
    audit_path_attributes(text, source, &label, own_root, &code)?;
    Ok(())
}

/// 逐字节代码区掩码：`true` 表示该 byte 处于真实代码中（不在行注释、块注释、
/// 字符串 / raw 字符串字面量或字符字面量内）。审计只对代码区 token 生效，字符串与
/// 注释内容不参与判定。字符字面量必须掩码：`'"'` / `'\''` 内的引号若被当作字符串
/// 起点会使掩码脱同步；生命周期标注 `'a` 找不到邻近闭合引号，自然保持代码。
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
            b'\'' => {
                // 字符字面量：单行、内容最长 `'{char}'`（含 \u{10FFFF} 共 12 byte）。
                // 生命周期标注找不到邻近闭合引号，落入 else 保持代码。
                let mut j = i + 1;
                let mut char_literal = false;
                while j < bytes.len() {
                    match bytes[j] {
                        b'\\' => j += 2,
                        b'\'' => {
                            char_literal = j - i <= 12;
                            break;
                        }
                        b'\n' => break,
                        _ => j += 1,
                    }
                }
                if char_literal {
                    for slot in mask.iter_mut().skip(i).take(j + 1 - i) {
                        *slot = false;
                    }
                    i = j + 1;
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
    own_root: &Path,
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
        require_contained_rs_target(source, own_root, &target, label, "include 宏")?;
        cursor = next;
    }
    Ok(())
}

fn audit_path_attributes(
    text: &str,
    source: &Path,
    label: &str,
    own_root: &Path,
    code: &[bool],
) -> Result<(), String> {
    let bytes = text.as_bytes();
    let mut cursor = 0;
    // `#` 与 `[` 之间允许空白与注释（`# [path = "..."]`），crate 级内部属性
    // `#![...]` 套用同一检查；宏 / raw 字符串内的 `#` 已被掩码排除。
    while let Some(relative) = text[cursor..].find('#') {
        let start = cursor + relative;
        cursor = start + 1;
        if !is_code(code, start, 1) {
            continue;
        }
        skip_trivia(bytes, &mut cursor);
        if bytes.get(cursor) == Some(&b'!') {
            cursor += 1;
            skip_trivia(bytes, &mut cursor);
        }
        if bytes.get(cursor) != Some(&b'[') {
            continue;
        }
        cursor += 1;
        skip_trivia(bytes, &mut cursor);
        if text[cursor..].starts_with("cfg_attr")
            && !text[cursor + "cfg_attr".len()..]
                .chars()
                .next()
                .is_some_and(schema_codegen::is_identifier_character)
        {
            // cfg_attr 激活时嵌套属性生效（含嵌套 cfg_attr），下钻括号范围审计
            // 其中的 path 属性；谓词是否启用不影响静态拒绝口径。
            cursor += "cfg_attr".len();
            skip_trivia(bytes, &mut cursor);
            if bytes.get(cursor) != Some(&b'(') {
                continue;
            }
            cursor = audit_cfg_attr_range(text, source, label, own_root, code, cursor)?;
            continue;
        }
        if !text[cursor..].starts_with("path") {
            continue;
        }
        cursor = audit_path_value(text, source, label, own_root, bytes, cursor, "#[path] 属性")?;
    }
    Ok(())
}

/// 审计 `cfg_attr(...)` 括号范围内的全部嵌套 `path = "..."` 属性，返回括号闭合后的
/// 下一个 byte 下标。括号配对只计代码区，字符串 / 注释内的括号不干扰。
fn audit_cfg_attr_range(
    text: &str,
    source: &Path,
    label: &str,
    own_root: &Path,
    code: &[bool],
    open_paren: usize,
) -> Result<usize, String> {
    let bytes = text.as_bytes();
    let mut depth = 1usize;
    let mut cursor = open_paren + 1;
    while depth > 0 && cursor < bytes.len() {
        if !code[cursor] {
            cursor += 1;
            continue;
        }
        match bytes[cursor] {
            b'(' => {
                depth += 1;
                cursor += 1;
            }
            b')' => {
                depth -= 1;
                cursor += 1;
            }
            _ => {
                if text[cursor..].starts_with("path")
                    && !text[cursor + "path".len()..]
                        .chars()
                        .next()
                        .is_some_and(schema_codegen::is_identifier_character)
                    && (cursor == 0
                        || !text[..cursor]
                            .chars()
                            .next_back()
                            .is_some_and(schema_codegen::is_identifier_character))
                {
                    cursor = audit_path_value(
                        text,
                        source,
                        label,
                        own_root,
                        bytes,
                        cursor,
                        "#[cfg_attr] 嵌套 path 属性",
                    )?;
                } else {
                    cursor += 1;
                }
            }
        }
    }
    Ok(cursor)
}

/// 审计 `path` 属性 token（`cursor` 指向 `path` 起点）的 `= "..."` 值，返回值结束后的
/// 下一个 byte 下标；非 `=` 形态无法通过 rustc 编译，跳过。
fn audit_path_value(
    text: &str,
    source: &Path,
    label: &str,
    own_root: &Path,
    bytes: &[u8],
    mut cursor: usize,
    kind: &str,
) -> Result<usize, String> {
    cursor += "path".len();
    skip_trivia(bytes, &mut cursor);
    if bytes.get(cursor) != Some(&b'=') {
        // rustc 只接受字符串字面量形态的 path 属性，其他形态无法通过编译。
        return Ok(cursor);
    }
    cursor += 1;
    skip_trivia(bytes, &mut cursor);
    let Some((target, next)) = read_string_literal(text, cursor) else {
        return Err(format!(
            "`{label}` 的 {kind} 值不是字符串字面量，无法静态审计"
        ));
    };
    require_contained_rs_target(source, own_root, &target, label, kind)?;
    Ok(next)
}

/// 断言 include / `#[path]`（含 `#[cfg_attr]` 嵌套）目标以 `.rs` 结尾，且相对引入它的
/// 源文件解析并 canonicalize 后仍落在源文件自己所属的 package 根目录内；绝对路径、
/// `..` 逃逸与跨 package 引用由此一并拒绝。
fn require_contained_rs_target(
    source: &Path,
    own_root: &Path,
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
    if !candidate.starts_with(own_root) {
        return Err(format!(
            "`{label}` 的 {kind} 目标 `{target}` 逃逸本 package 根目录"
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
        // 必须先解码再小写：`\U0000002D` 若先小写会折叠成 `\u0000002d`，解码器只消费
        // 4 位十六进制而把 8 位 `\U` 序列截断成不可匹配的内容。
        let decoded = decode_flag_escapes(raw_line).to_lowercase();
        if let RustflagsContinuation::BlockScalar(indent) = continuation {
            let line_indent = raw_line.len() - raw_line.trim_start().len();
            if raw_line.trim().is_empty() || line_indent > indent {
                reject_weakening_token(&lower, &decoded, label, number)?;
                continue;
            }
            continuation = RustflagsContinuation::None;
        }
        let opens = lower.contains("rustflags");
        let active = opens || !matches!(continuation, RustflagsContinuation::None);
        if active {
            reject_weakening_token(&lower, &decoded, label, number)?;
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

/// 统计 `[` `{` 相对 `]` `}` 的净余额（从 `initial` 累加）。双引号（含 `\` 转义）与
/// 单引号字符串字面量的内容不参与计数：`rustflags = ["]",` 中字符串内的 `]` 不会
/// 抵消未闭合的 `[` 而导致后续续行漏检。
fn bracket_balance(text: &str, initial: i64) -> i64 {
    let bytes = text.as_bytes();
    let mut balance = initial;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
            }
            b'\'' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\'' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            b'[' | b'{' => {
                balance += 1;
                i += 1;
            }
            b']' | b'}' => {
                balance -= 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    balance
}

/// 对原始行与转义解码行各匹配一次弱化 token。TOML/YAML 双引号字符串会把
/// `\u002d` 这类转义解码成 `-`（TOML/Cargo 侧按解码后的值生效），解码后匹配
/// 使标准字符串转义无法绕过边界。解码值中的 `@` 一律拒绝：rustc 会把 `@file`
/// 展开为响应文件参数（换行分隔），文件内容不在本审计覆盖范围内。
fn reject_weakening_token(
    lowered_line: &str,
    decoded_line: &str,
    label: &str,
    number: usize,
) -> Result<(), String> {
    if decoded_line.contains('@') {
        return Err(format!(
            "`{label}` 第 {} 行 rustflags 含 `@`，rustc `@file` 响应文件参数无法静态审计",
            number + 1
        ));
    }
    if let Some(token) = RUSTFLAGS_WEAKENING_TOKENS
        .iter()
        .find(|token| lowered_line.contains(**token) || decoded_line.contains(**token))
    {
        return Err(format!(
            "`{label}` 第 {} 行 rustflags 含 `{token}`，会削弱 workspace `unsafe_code = \"forbid\"` 边界",
            number + 1
        ));
    }
    Ok(())
}

/// 单趟解码 TOML/YAML 双引号字符串中的常见转义：`\\`、`\"`、`\'`、`\x`、`\u`、
/// `\U`。只做一趟（`\\u002d` 解成字面 `\u002d` 后不再二次解码）；未知转义原样保留。
fn decode_flag_escapes(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(kind) = chars.next() else {
            out.push('\\');
            break;
        };
        let decode_hex = |chars: &mut std::iter::Peekable<std::str::Chars>, digits: usize| {
            let mut value = String::with_capacity(digits);
            for _ in 0..digits {
                match chars.next_if(|c| c.is_ascii_hexdigit()) {
                    Some(c) => value.push(c),
                    None => return None,
                }
            }
            u32::from_str_radix(&value, 16)
                .ok()
                .and_then(char::from_u32)
        };
        match kind {
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            'x' => match decode_hex(&mut chars, 2) {
                Some(decoded) => out.push(decoded),
                None => out.push_str("\\x"),
            },
            'u' => match decode_hex(&mut chars, 4) {
                Some(decoded) => out.push(decoded),
                None => out.push_str("\\u"),
            },
            'U' => match decode_hex(&mut chars, 8) {
                Some(decoded) => out.push(decoded),
                None => out.push_str("\\U"),
            },
            _ => {
                out.push('\\');
                out.push(kind);
            }
        }
    }
    out
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

    #[test]
    fn wire_manifest_rejects_quoted_keys_and_sections() {
        // Cargo 接受引号键 / 引号表头：`["package"]` + `"build"`、`["lib"]` +
        // `"path"` 与无引号写法等效，审计必须归一化后匹配。
        let root = temp_package_root("quoted-keys");
        let quoted_build = "[\"package\"]\nname = \"fixture\"\n\"build\" = \"build.rs\"\n";
        assert!(require_wire_manifest_targets(quoted_build, &root, "fixture").is_err());

        let sandbox = std::env::temp_dir().join(format!(
            "laneflow-wire-audit-quoted-escape-{}",
            std::process::id()
        ));
        let root = sandbox.join("pkg");
        fs::create_dir_all(root.join("src")).expect("temporary wire package src directory");
        fs::write(root.join("src/lib.rs"), "pub fn placeholder() {}\n")
            .expect("temporary wire lib target");
        fs::write(sandbox.join("outside.rs"), "pub fn outside() {}\n")
            .expect("temporary escaping target");
        let quoted_path = "[\"lib\"]\n\"path\" = \"../outside.rs\"\n";
        let error = require_wire_manifest_targets(quoted_path, &root, "fixture").unwrap_err();
        assert!(error.contains("逃逸"));
        let quoted_contained = "[\"lib\"]\n\"path\" = \"src/lib.rs\"\n";
        assert_eq!(
            require_wire_manifest_targets(quoted_contained, &root, "fixture"),
            Ok(())
        );
        fs::remove_dir_all(&sandbox).expect("remove temporary quoted escape sandbox");
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
        fs::write(&source, "pub fn audited_source() {}\n").expect("temporary audited source");
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
    fn source_includes_reject_cross_package_and_cfg_attr_targets() {
        // 跨 package include：目标在另一个 workspace package 内，但 wire crate 的
        // unsafe 扫描只覆盖本包根，必须拒绝。
        let sandbox = std::env::temp_dir().join(format!(
            "laneflow-wire-audit-crosspkg-{}",
            std::process::id()
        ));
        let pkg = sandbox.join("pkg");
        let other = sandbox.join("other");
        let source = pkg.join("src/lib.rs");
        fs::create_dir_all(source.parent().expect("source parent")).expect("pkg src dir");
        fs::write(&source, "pub fn audited_source() {}\n").expect("pkg source file");
        fs::create_dir_all(other.join("src")).expect("other package src dir");
        fs::write(other.join("src/dormant.rs"), "pub fn dormant() {}\n")
            .expect("other package file");
        let roots = vec![
            pkg.canonicalize().expect("canonical pkg root"),
            other.canonicalize().expect("canonical other root"),
        ];
        let cross = concat!("include", "!(\"../../other/src/dormant.rs\")");
        assert!(require_audited_source_includes(cross, &source, &roots).is_err());
        fs::remove_dir_all(&sandbox).expect("remove temporary cross-package sandbox");

        // #[cfg_attr] 嵌套 path 属性（含谓词与多属性）套用同一闭合规则。
        let (sandbox, source, roots) = temp_source_fixture(
            "cfg-attr",
            &[("pkg/src/support.rs", "pub fn support() {}\n")],
        );
        let cfg_ok = concat!("#[", "cfg_attr(unix, path = \"support.rs\")]\nmod support;");
        assert_source_ok(&sandbox, &source, &roots, cfg_ok);
        let (sandbox, source, roots) =
            temp_source_fixture("cfg-attr", &[("outside.rs", "pub fn outside() {}\n")]);
        let cfg_escape = concat!(
            "#[",
            "cfg_attr(unix, path = \"../../outside.rs\")]\nmod outside;"
        );
        assert_source_err(&sandbox, &source, &roots, cfg_escape);
        let (sandbox, source, roots) =
            temp_source_fixture("cfg-attr", &[("outside.rs", "pub fn outside() {}\n")]);
        let cfg_nested = concat!(
            "#[",
            "cfg_attr(unix, derive(Debug), cfg_attr(windows, path = \"../../outside.rs\"))]\nmod outside;"
        );
        assert_source_err(&sandbox, &source, &roots, cfg_nested);
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
    fn rustflags_audit_rejects_escape_encoded_weakening() {
        // TOML 双引号字符串的 \u 转义在 Cargo 侧解码后生效，解码前匹配会漏检。
        assert!(
            require_rustflags_respect_unsafe_forbid(
                "rustflags = [\"--cap\\u002dlints\", \"allow\"]\n",
                "fixture"
            )
            .is_err()
        );
        assert!(
            require_rustflags_respect_unsafe_forbid(
                "env:\n  RUSTFLAGS: \"--force-warn=unsafe\\x2dcode\"\n",
                "workflow"
            )
            .is_err()
        );
        // 双反斜杠转义单趟解码：字面 `\\u002d` 解成 `\u002d` 后不再二次解码，
        // 不构成弱化 token。
        assert_eq!(
            require_rustflags_respect_unsafe_forbid(
                "rustflags = [\"-C\", \"link-arg=\\\\u002d\"]\n",
                "fixture"
            ),
            Ok(())
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

    #[test]
    fn wire_manifest_rejects_inline_table_build_key() {
        // cargo metadata 实测确认 `package = { build = "..." }` 内联表同样生成
        // custom-build target。
        let root = temp_package_root("inline-build");
        let inline = "package = { name = \"fixture\", build = \"build.rs\" }\n";
        assert!(require_wire_manifest_targets(inline, &root, "fixture").is_err());
        let clean = "package = { name = \"fixture\" }\nlib = { path = \"src/lib.rs\" }\n";
        assert_eq!(
            require_wire_manifest_targets(clean, &root, "fixture"),
            Ok(())
        );
        fs::remove_dir_all(&root).expect("remove temporary wire package");
    }

    #[test]
    fn wire_manifest_rejects_inline_table_target_path_escape() {
        let sandbox = std::env::temp_dir().join(format!(
            "laneflow-wire-audit-inline-escape-{}",
            std::process::id()
        ));
        let root = sandbox.join("pkg");
        fs::create_dir_all(root.join("src")).expect("temporary wire package src directory");
        fs::write(root.join("src/lib.rs"), "pub fn placeholder() {}\n")
            .expect("temporary wire lib target");
        fs::write(sandbox.join("outside.rs"), "pub fn outside() {}\n")
            .expect("temporary escaping target");

        let inline = "lib = { path = \"../outside.rs\" }\n";
        let error = require_wire_manifest_targets(inline, &root, "fixture").unwrap_err();
        assert!(error.contains("逃逸"));
        // 内联表数组与 [[bin]] 段在 Cargo 侧反序列化等价，套用同一闭合规则。
        let inline_array = "bin = [{ name = \"fixture-bin\", path = \"../outside.rs\" }]\n";
        let error = require_wire_manifest_targets(inline_array, &root, "fixture").unwrap_err();
        assert!(error.contains("逃逸"));
        let contained = "bin = [{ name = \"fixture-bin\", path = \"src/lib.rs\" }]\n";
        assert_eq!(
            require_wire_manifest_targets(contained, &root, "fixture"),
            Ok(())
        );
        fs::remove_dir_all(&sandbox).expect("remove temporary inline escape sandbox");
    }

    #[test]
    fn wire_manifest_rejects_non_rs_target_path() {
        // Cargo 会把任意后缀的显式 target path 按 Rust 源编译，审计只放行 .rs。
        let root = temp_package_root("non-rs-target");
        let section_form = "[lib]\npath = \"src/lib.txt\"\n";
        assert!(require_wire_manifest_targets(section_form, &root, "fixture").is_err());
        let inline_form = "lib = { path = \"src/lib.txt\" }\n";
        assert!(require_wire_manifest_targets(inline_form, &root, "fixture").is_err());
        fs::remove_dir_all(&root).expect("remove temporary wire package");
    }

    #[test]
    fn source_includes_audit_hash_bracket_separated_path_attribute() {
        // rustc 接受 `#` 与 `[` 之间的空白 / 注释，审计必须按同一形态归一。
        let (sandbox, source, roots) = temp_source_fixture(
            "spaced-attr",
            &[("pkg/src/support.rs", "pub fn support() {}\n")],
        );
        let spaced_ok = concat!("#", " [path = \"support.rs\"]\nmod support;");
        assert_source_ok(&sandbox, &source, &roots, spaced_ok);
        let (sandbox, source, roots) =
            temp_source_fixture("spaced-attr", &[("outside.rs", "pub fn outside() {}\n")]);
        let spaced_escape = concat!("#", " [path = \"../../outside.rs\"]\nmod outside;");
        assert_source_err(&sandbox, &source, &roots, spaced_escape);
        let (sandbox, source, roots) =
            temp_source_fixture("spaced-attr", &[("outside.rs", "pub fn outside() {}\n")]);
        let comment_gap = concat!(
            "#",
            " /* 间隔 */ [path = \"../../outside.rs\"]\nmod outside;"
        );
        assert_source_err(&sandbox, &source, &roots, comment_gap);
    }

    #[test]
    fn rustflags_audit_decodes_before_lowercasing() {
        // `\U` 八位转义若先小写会折叠成 `\u` 四位序列而被截断漏解，必须先解码再小写。
        assert!(
            require_rustflags_respect_unsafe_forbid(
                "rustflags = [\"--cap\\U0000002dlints\", \"allow\"]\n",
                "fixture"
            )
            .is_err()
        );
        assert!(
            require_rustflags_respect_unsafe_forbid(
                "env:\n  RUSTFLAGS: \"--force-warn=unsafe\\U0000002Dcode\"\n",
                "workflow"
            )
            .is_err()
        );
    }

    #[test]
    fn rustflags_audit_ignores_brackets_inside_strings() {
        // 字符串内的 `]` 不抵消未闭合的 `[`：数组实际跨行时续行必须仍被扫描。
        assert!(
            require_rustflags_respect_unsafe_forbid(
                "rustflags = [\"]\",\n  \"--cap-lints\", \"allow\"]\n",
                "fixture"
            )
            .is_err()
        );
        assert!(
            require_rustflags_respect_unsafe_forbid(
                "rustflags = [']',\n  \"--cap-lints\", \"warn\"]\n",
                "fixture"
            )
            .is_err()
        );
        assert_eq!(
            require_rustflags_respect_unsafe_forbid("rustflags = [\"]\", \"-C\"]\n", "fixture"),
            Ok(())
        );
    }

    #[test]
    fn rustflags_audit_rejects_response_file_argument() {
        // rustc 把 `@file` 展开为换行分隔的响应文件参数，文件内容不受本审计覆盖。
        assert!(
            require_rustflags_respect_unsafe_forbid(
                "env:\n  RUSTFLAGS: @/tmp/flags.rsp\n",
                "workflow"
            )
            .is_err()
        );
        assert!(
            require_rustflags_respect_unsafe_forbid(
                "rustflags = [\"\\u0040/tmp/flags.rsp\"]\n",
                "fixture"
            )
            .is_err()
        );
    }
}
