//! Wire / toolchain 审计边界（#376）。
//!
//! 在两个 wire 家族的 codegen clean-regeneration 检查之外，独立审计工具链侧通道：
//! Cargo.lock resolved graph 钉版（version/source/checksum）、wire manifest 显式
//! target（真 TOML 解析，点号键 / 引号键 / 转义键 / 内联表 / 跨行数组统一归一）
//! 不得逃逸 package 根目录且不得引入 build 脚本、Rust 源经 include 宏（含注释间隔
//! 形态；use 别名导入与 macro_rules 元变量宏名调用一律拒绝）与 `#[path]` 属性引入
//! 的目标必须是 `.rs` 且 canonicalize 后仍在 workspace package 根目录内、仓库
//! Cargo 配置（真 TOML 解析）与 workflow（真 YAML 解析）中的 rustflags 不得削弱
//! workspace `unsafe_code = "forbid"` 边界。

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
/// 出现 build 脚本键；package 根目录不得存在 build.rs。manifest 用真 TOML 解析：
/// 点号键、引号键、转义键（如把字符写成 \u 转义形态的等价键）、内联表与跨行数组
/// 统一由解析器归一化，审计面对解析结果做判定；解析失败一律拒绝（fail closed）。
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
    let manifest: toml::Table = manifest_text
        .parse()
        .map_err(|error| format!("wire manifest `{label}` TOML 解析失败，无法静态审计: {error}"))?;
    if let Some(package) = manifest.get("package").and_then(toml::Value::as_table)
        && package.contains_key("build")
    {
        return Err(format!("wire manifest `{label}` 不得声明 build 脚本键"));
    }
    for name in WIRE_TARGET_SECTIONS {
        let Some(value) = manifest.get(name) else {
            continue;
        };
        // lib 是单表；bin/test/bench/example 是表数组。数组赋值形态
        // （`bin = [{ ... }]`）虽被 Cargo 忽略（cargo metadata 实测），仍按同一
        // 口径检查解析结果，保持审计保守性。
        let tables: Vec<&toml::Table> = match value {
            toml::Value::Table(table) => vec![table],
            toml::Value::Array(entries) => {
                entries.iter().filter_map(toml::Value::as_table).collect()
            }
            _ => Vec::new(),
        };
        for table in tables {
            let Some(path_value) = table.get("path") else {
                continue;
            };
            let Some(relative) = path_value.as_str() else {
                return Err(format!(
                    "wire manifest `{label}` 的 `{name}` target path 不是字符串，无法静态审计"
                ));
            };
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
    audit_macro_rules_metavariable_invocations(text, &label, &code)?;
    Ok(())
}

/// 审计 `macro_rules!` 定义：体内出现 `$ident!`（元变量直接作为宏名调用）或
/// `#[$ident]` / `#![$ident]`（元变量作为属性内容）即拒绝整个源文件——
/// `call_macro!(include, "../../outside.rs")` 与 `with_attr!(path = "../../outside.rs")`
/// 这类调用在展开期生成真实 `include!(...)` / `#[path = "..."]`，目标完全由调用方
/// 决定，include 闭合与 unsafe 扫描都会被绕过。matcher 侧的元变量（`$m:ident`）后随
/// `:`，重复展开 `$( ... )*` 的 `$` 后随 `(`，模板内具名属性（`#[derive(...)]`）
/// 均不误判。
fn audit_macro_rules_metavariable_invocations(
    text: &str,
    label: &str,
    code: &[bool],
) -> Result<(), String> {
    let bytes = text.as_bytes();
    let mut cursor = 0;
    while let Some(relative) = text[cursor..].find("macro_rules") {
        let start = cursor + relative;
        cursor = start + "macro_rules".len();
        let before_ok = text[..start]
            .chars()
            .next_back()
            .is_none_or(|ch| !schema_codegen::is_identifier_character(ch));
        let after_ok = text[cursor..]
            .chars()
            .next()
            .is_none_or(|ch| !schema_codegen::is_identifier_character(ch));
        if !before_ok || !after_ok || !is_code(code, start, "macro_rules".len()) {
            continue;
        }
        skip_trivia(bytes, &mut cursor);
        if bytes.get(cursor) != Some(&b'!') {
            // 非 `macro_rules!` 宏定义形态（如普通标识符），不在审计范围。
            continue;
        }
        cursor += 1;
        skip_trivia(bytes, &mut cursor);
        // 宏名。
        while bytes
            .get(cursor)
            .is_some_and(|byte| schema_codegen::is_identifier_character(*byte as char))
        {
            cursor += 1;
        }
        skip_trivia(bytes, &mut cursor);
        if bytes.get(cursor) != Some(&b'{') {
            return Err(format!(
                "`{label}` 的 macro_rules! 定义体不是 `{{` 形态，无法静态审计"
            ));
        }
        // 代码区括号配对取整个定义体（字符串 / 注释已被掩码排除）。
        let body_start = cursor;
        let mut depth = 0usize;
        while cursor < bytes.len() {
            if code[cursor] {
                match bytes[cursor] {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            cursor += 1;
        }
        if depth != 0 {
            return Err(format!(
                "`{label}` 的 macro_rules! 定义体括号不闭合，无法静态审计"
            ));
        }
        let body_end = cursor;
        let mut scan = body_start;
        while scan < body_end {
            if !code[scan] {
                scan += 1;
                continue;
            }
            if bytes[scan] == b'$' {
                let mut at = scan + 1;
                let ident_start = at;
                while at < body_end && schema_codegen::is_identifier_character(bytes[at] as char) {
                    at += 1;
                }
                if at > ident_start {
                    let mut after = at;
                    skip_trivia(bytes, &mut after);
                    if after < body_end && bytes[after] == b'!' {
                        return Err(format!(
                            "`{label}` 的 macro_rules! 模板以元变量作为宏名调用（`$ident!`），展开结果无法静态审计"
                        ));
                    }
                }
                scan = at.max(scan + 1);
                continue;
            }
            // 元变量作为属性内容（`#[$a]` / `#![$a]`）：`with_attr!(path = "...")`
            // 在展开期生成真实 #[path] 属性，同样绕开闭合审计。
            if bytes[scan] == b'#' {
                let mut at = scan + 1;
                skip_trivia(bytes, &mut at);
                if at < body_end && bytes[at] == b'!' {
                    at += 1;
                    skip_trivia(bytes, &mut at);
                }
                if at < body_end && bytes[at] == b'[' {
                    at += 1;
                    skip_trivia(bytes, &mut at);
                    if at < body_end && bytes[at] == b'$' {
                        return Err(format!(
                            "`{label}` 的 macro_rules! 模板以元变量作为属性内容（`#[$ident]`），展开结果无法静态审计"
                        ));
                    }
                }
                scan += 1;
                continue;
            }
            scan += 1;
        }
        cursor = body_end + 1;
    }
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
            require_toml_config_respects_unsafe_forbid(&text, relative)?;
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
        require_workflow_respects_unsafe_forbid(&text, &path.display().to_string())?;
    }
    Ok(())
}

/// 审计单份 Cargo 配置（TOML）：递归遍历解析树，任何层级出现 rustflags 键
/// （见 `is_rustflags_key`，覆盖 `[build]` / `[target.*]` / `[env]` 等所有 Cargo
/// 接受的位置及 `CARGO_*_RUSTFLAGS` 环境变量形态；键的引号与 `\u` 转义、多行数组、
/// 三引号字符串及 `\` 续行均由解析器归一化）即对其
/// 值套用统一检查。解析失败一律拒绝（fail closed）。
fn require_toml_config_respects_unsafe_forbid(text: &str, label: &str) -> Result<(), String> {
    let parsed: toml::Table = text
        .parse()
        .map_err(|error| format!("`{label}` TOML 解析失败，无法静态审计: {error}"))?;
    audit_toml_tree_for_rustflags(&toml::Value::Table(parsed), label)
}

fn audit_toml_tree_for_rustflags(value: &toml::Value, label: &str) -> Result<(), String> {
    match value {
        toml::Value::Table(table) => {
            for (key, child) in table {
                if is_rustflags_key(key) {
                    require_audited_toml_flag_value(child, label)?;
                }
                audit_toml_tree_for_rustflags(child, label)?;
            }
            Ok(())
        }
        toml::Value::Array(entries) => {
            for entry in entries {
                audit_toml_tree_for_rustflags(entry, label)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn require_audited_toml_flag_value(value: &toml::Value, label: &str) -> Result<(), String> {
    match value {
        toml::Value::String(text) => reject_weakening_flag_text(text, label),
        toml::Value::Array(entries) => {
            for entry in entries {
                let toml::Value::String(text) = entry else {
                    return Err(format!(
                        "`{label}` 的 rustflags 数组含非字符串元素，无法静态审计"
                    ));
                };
                reject_weakening_flag_text(text, label)?;
            }
            Ok(())
        }
        _ => Err(format!(
            "`{label}` 的 rustflags 值不是字符串或字符串数组，无法静态审计"
        )),
    }
}

/// rustflags 键名判定（TOML/YAML 两侧共用）：除精确的 `rustflags` 外，Cargo 的
/// 环境变量配置层把 `CARGO_BUILD_RUSTFLAGS` 映射为 `[build] rustflags`、
/// `CARGO_TARGET_<TRIPLE>_RUSTFLAGS` 映射为 target 专属 rustflags、
/// `CARGO_ENCODED_RUSTFLAGS` 直接生效（cargo check -vv 实测确认这些形态进入
/// rustc 命令行），全部同罪。大小写不敏感。
fn is_rustflags_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    lower == "rustflags"
        || lower == "cargo_build_rustflags"
        || lower == "cargo_encoded_rustflags"
        || (lower.starts_with("cargo_target_") && lower.ends_with("_rustflags"))
}

/// 审计单份 GitHub workflow（YAML）：递归遍历所有 document，任何层级出现
/// rustflags 键（见 `is_rustflags_key`；键的引号与转义、块标量（含显式缩进指示符）
/// 与多行标量折叠均由解析器归一化）即对其值套用统一检查；任何字符串标量（如
/// `run` 脚本）内的 rustflags 赋值形态一并拒绝。解析失败一律拒绝（fail closed）。
fn require_workflow_respects_unsafe_forbid(text: &str, label: &str) -> Result<(), String> {
    let documents = yaml_rust2::YamlLoader::load_from_str(text)
        .map_err(|error| format!("`{label}` YAML 解析失败，无法静态审计: {error}"))?;
    for document in documents {
        audit_yaml_tree_for_rustflags(&document, label)?;
    }
    Ok(())
}

fn audit_yaml_tree_for_rustflags(value: &yaml_rust2::Yaml, label: &str) -> Result<(), String> {
    match value {
        yaml_rust2::Yaml::Hash(hash) => {
            for (key, child) in hash {
                if key.as_str().is_some_and(is_rustflags_key) {
                    require_audited_yaml_flag_value(child, label)?;
                }
                audit_yaml_tree_for_rustflags(child, label)?;
            }
            Ok(())
        }
        yaml_rust2::Yaml::Array(entries) => {
            for entry in entries {
                audit_yaml_tree_for_rustflags(entry, label)?;
            }
            Ok(())
        }
        yaml_rust2::Yaml::String(text) => reject_rustflags_assignment_in_script(text, label),
        _ => Ok(()),
    }
}

/// workflow 字符串标量的兜底扫描：`run` 脚本等自由文本里的 rustflags 赋值
/// （shell 前缀 `RUSTFLAGS=... cmd`、`export RUSTFLAGS=...`、`env RUSTFLAGS=...`、
/// pwsh `$env:RUSTFLAGS = ...` 都以 `rustflags` 后随 `=` 为共同形态）无法做 shell
/// 语义级静态审计，一律拒绝。读取引用（`$RUSTFLAGS`、`${RUSTFLAGS}`）后随字符
/// 不是 `=`，不误伤。
fn reject_rustflags_assignment_in_script(text: &str, label: &str) -> Result<(), String> {
    let lower = text.to_lowercase();
    let mut search = 0;
    while let Some(relative) = lower[search..].find("rustflags") {
        let after_key = search + relative + "rustflags".len();
        if lower[after_key..].trim_start().starts_with('=') {
            return Err(format!(
                "`{label}` 的脚本文本含 rustflags 赋值，shell 语义无法静态审计"
            ));
        }
        search = after_key;
    }
    Ok(())
}

fn require_audited_yaml_flag_value(value: &yaml_rust2::Yaml, label: &str) -> Result<(), String> {
    match value {
        yaml_rust2::Yaml::String(text) => reject_weakening_flag_text(text, label),
        yaml_rust2::Yaml::Array(entries) => {
            for entry in entries {
                let yaml_rust2::Yaml::String(text) = entry else {
                    return Err(format!(
                        "`{label}` 的 rustflags 数组含非字符串元素，无法静态审计"
                    ));
                };
                reject_weakening_flag_text(text, label)?;
            }
            Ok(())
        }
        _ => Err(format!("`{label}` 的 rustflags 值不是字符串，无法静态审计")),
    }
}

/// 单条 rustflags 文本的弱化判定。TOML/YAML 解析器已完成转义解码、多行折叠与
/// 续行拼接，此处直接对最终生效值匹配：弱化 token 小写包含即拒绝；`@` 一律拒绝
/// （rustc 会把 `@file` 展开为换行分隔的响应文件参数，文件内容不在本审计覆盖
/// 范围内）；`${{` 一律拒绝（GitHub Actions 表达式由 runner 在运行时解析，来源
/// 值不受静态审计覆盖）。
fn reject_weakening_flag_text(text: &str, label: &str) -> Result<(), String> {
    let lower = text.to_lowercase();
    if lower.contains('@') {
        return Err(format!(
            "`{label}` 的 rustflags 值含 `@`，rustc `@file` 响应文件参数无法静态审计"
        ));
    }
    if lower.contains("${{") {
        return Err(format!(
            "`{label}` 的 rustflags 值含 GitHub Actions 表达式，来源值在 runner 运行时解析，无法静态审计"
        ));
    }
    if let Some(token) = RUSTFLAGS_WEAKENING_TOKENS
        .iter()
        .find(|token| lower.contains(**token))
    {
        return Err(format!(
            "`{label}` 的 rustflags 值含 `{token}`，会削弱 workspace `unsafe_code = \"forbid\"` 边界"
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
            require_toml_config_respects_unsafe_forbid("", "fixture"),
            Ok(())
        );
        assert_eq!(
            require_toml_config_respects_unsafe_forbid(
                "[build]\nrustflags = [\"-C\", \"opt-level=3\"]\n",
                "fixture"
            ),
            Ok(())
        );
        assert_eq!(
            require_workflow_respects_unsafe_forbid("env:\n  RUSTFLAGS: -Dwarnings\n", "workflow"),
            Ok(())
        );
    }

    #[test]
    fn rustflags_audit_rejects_unsafe_weakening() {
        assert!(
            require_toml_config_respects_unsafe_forbid(
                "[build]\nrustflags = [\"--cap-lints\", \"warn\"]\n",
                "fixture"
            )
            .is_err()
        );
        assert!(
            require_workflow_respects_unsafe_forbid(
                "env:\n  RUSTFLAGS: -A unsafe-code\n",
                "workflow"
            )
            .is_err()
        );
        assert!(
            require_toml_config_respects_unsafe_forbid(
                "rustflags = [\n  \"--cap-lints\",\n  \"allow\"\n]\n",
                "fixture"
            )
            .is_err()
        );
        assert!(
            require_workflow_respects_unsafe_forbid("RUSTFLAGS: >-\n  -Aunsafe_code\n", "workflow")
                .is_err()
        );
    }

    #[test]
    fn rustflags_audit_rejects_force_warn_downgrade() {
        // rustc 实测：`--force-warn unsafe_code` 使 `#![forbid(unsafe_code)]` 只产生
        // warning 并以零状态码退出，与 allow 类削弱同罪。
        assert!(
            require_workflow_respects_unsafe_forbid(
                "env:\n  RUSTFLAGS: --force-warn unsafe_code\n",
                "workflow"
            )
            .is_err()
        );
        assert!(
            require_toml_config_respects_unsafe_forbid(
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
            require_toml_config_respects_unsafe_forbid(
                "rustflags = [\"--cap\\u002dlints\", \"allow\"]\n",
                "fixture"
            )
            .is_err()
        );
        assert!(
            require_workflow_respects_unsafe_forbid(
                "env:\n  RUSTFLAGS: \"--force-warn=unsafe\\x2dcode\"\n",
                "workflow"
            )
            .is_err()
        );
        // 双反斜杠转义单趟解码：字面 `\\u002d` 解成 `\u002d` 后不再二次解码，
        // 不构成弱化 token。
        assert_eq!(
            require_toml_config_respects_unsafe_forbid(
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
            require_workflow_respects_unsafe_forbid(
                "env:\n  RUSTFLAGS: >-\n    -C opt-level=2\n    --cap-lints allow\n",
                "workflow"
            )
            .is_err()
        );
        assert!(
            require_workflow_respects_unsafe_forbid(
                "env:\n  RUSTFLAGS: | # 保留换行\n    --force-warn unsafe-code\n",
                "workflow"
            )
            .is_err()
        );
        // 缩进回到键级即结束标量；良性块标量整体放行。
        assert_eq!(
            require_workflow_respects_unsafe_forbid(
                "env:\n  RUSTFLAGS: >-\n    -C opt-level=2\n    -Dwarnings\n  OTHER: 1\n",
                "workflow"
            ),
            Ok(())
        );
    }

    #[test]
    fn rustflags_audit_scans_toml_triple_quote_string() {
        assert!(
            require_toml_config_respects_unsafe_forbid(
                "rustflags = \"\"\"\n--cap-lints allow\n\"\"\"\n",
                "fixture"
            )
            .is_err()
        );
        assert_eq!(
            require_toml_config_respects_unsafe_forbid(
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
            require_toml_config_respects_unsafe_forbid(
                "rustflags = [\"--cap\\U0000002dlints\", \"allow\"]\n",
                "fixture"
            )
            .is_err()
        );
        assert!(
            require_workflow_respects_unsafe_forbid(
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
            require_toml_config_respects_unsafe_forbid(
                "rustflags = [\"]\",\n  \"--cap-lints\", \"allow\"]\n",
                "fixture"
            )
            .is_err()
        );
        assert!(
            require_toml_config_respects_unsafe_forbid(
                "rustflags = [']',\n  \"--cap-lints\", \"warn\"]\n",
                "fixture"
            )
            .is_err()
        );
        assert_eq!(
            require_toml_config_respects_unsafe_forbid("rustflags = [\"]\", \"-C\"]\n", "fixture"),
            Ok(())
        );
    }

    #[test]
    fn rustflags_audit_rejects_response_file_argument() {
        // rustc 把 `@file` 展开为换行分隔的响应文件参数，文件内容不受本审计覆盖。
        assert!(
            require_workflow_respects_unsafe_forbid(
                "env:\n  RUSTFLAGS: @/tmp/flags.rsp\n",
                "workflow"
            )
            .is_err()
        );
        assert!(
            require_toml_config_respects_unsafe_forbid(
                "rustflags = [\"\\u0040/tmp/flags.rsp\"]\n",
                "fixture"
            )
            .is_err()
        );
    }

    #[test]
    fn rustflags_audit_activates_on_escape_encoded_key() {
        // TOML 双引号键的 \u 转义由 Cargo 解码后识别为 rustflags（cargo config get
        // 实测确认），激活判定与续行跟踪都必须基于解码行。
        assert!(
            require_toml_config_respects_unsafe_forbid(
                "[build]\n\"rust\\u0066lags\" = [\"--cap-lints\", \"allow\"]\n",
                "fixture"
            )
            .is_err()
        );
        // 编码键的多行数组：续行跟踪同样由解码行激活。
        assert!(
            require_toml_config_respects_unsafe_forbid(
                "\"rust\\u0066lags\" = [\n  \"-C\", \"opt-level=2\",\n  \"--cap-lints\", \"allow\"\n]\n",
                "fixture"
            )
            .is_err()
        );
        // YAML 双引号键同口径。
        assert!(
            require_workflow_respects_unsafe_forbid(
                "env:\n  \"RUST\\u0046LAGS\": --force-warn unsafe_code\n",
                "workflow"
            )
            .is_err()
        );
        // 编码键的良性值仍放行（不误伤）。
        assert_eq!(
            require_toml_config_respects_unsafe_forbid(
                "\"rust\\u0066lags\" = [\"-C\", \"opt-level=2\"]\n",
                "fixture"
            ),
            Ok(())
        );
    }

    #[test]
    fn wire_manifest_rejects_escape_encoded_keys_and_sections() {
        // TOML 双引号键 / 表头的转义由解析器解码（cargo metadata 实测确认转义后的
        // build 键生成 custom-build、lib section 改指 path），审计面对解析结果判定，
        // 编码写法与直白写法同罪。
        let root = temp_package_root("encoded-build");
        let encoded_build = "[package]\nname = \"fixture\"\n\"bu\\u0069ld\" = \"build.rs\"\n";
        assert!(require_wire_manifest_targets(encoded_build, &root, "fixture").is_err());
        fs::remove_dir_all(&root).expect("remove temporary wire package");

        let sandbox = std::env::temp_dir().join(format!(
            "laneflow-wire-audit-encoded-escape-{}",
            std::process::id()
        ));
        let root = sandbox.join("pkg");
        fs::create_dir_all(root.join("src")).expect("temporary wire package src directory");
        fs::write(root.join("src/lib.rs"), "pub fn placeholder() {}\n")
            .expect("temporary wire lib target");
        fs::write(sandbox.join("outside.rs"), "pub fn outside() {}\n")
            .expect("temporary escaping target");
        let encoded_lib = "[package]\nname = \"fixture\"\n\n[\"l\\u0069b\"]\n\"pa\\u0074h\" = \"../outside.rs\"\n";
        let error = require_wire_manifest_targets(encoded_lib, &root, "fixture").unwrap_err();
        assert!(error.contains("逃逸"));
        fs::remove_dir_all(&sandbox).expect("remove temporary encoded escape sandbox");
    }

    #[test]
    fn wire_manifest_rejects_multiline_target_array_escape() {
        // 跨行数组赋值形态（`bin = [\n { ... } \n]`）：cargo metadata 实测确认 Cargo
        // 忽略该形态，但审计按解析结果保守拒绝其中的逃逸 path。
        let sandbox = std::env::temp_dir().join(format!(
            "laneflow-wire-audit-multiline-array-{}",
            std::process::id()
        ));
        let root = sandbox.join("pkg");
        fs::create_dir_all(root.join("src")).expect("temporary wire package src directory");
        fs::write(root.join("src/lib.rs"), "pub fn placeholder() {}\n")
            .expect("temporary wire lib target");
        fs::write(sandbox.join("outside.rs"), "pub fn outside() {}\n")
            .expect("temporary escaping target");
        // 键值对在 TOML 中归属最近的表头：顶层 bin 必须写在任何表头之前。
        let multiline = "bin = [\n  { name = \"fixture-bin\", path = \"../outside.rs\" }\n]\n\n[package]\nname = \"fixture\"\n";
        let error = require_wire_manifest_targets(multiline, &root, "fixture").unwrap_err();
        assert!(error.contains("逃逸"));
        fs::remove_dir_all(&sandbox).expect("remove temporary multiline array sandbox");
    }

    #[test]
    fn rustflags_audit_toml_triple_quote_line_continuation() {
        // TOML 多行基本字符串的行尾 `\` 删除换行与下一行缩进：token 跨物理行拆分
        // 后解析值仍是完整弱化 flag，真解析按最终生效值检出。
        assert!(
            require_toml_config_respects_unsafe_forbid(
                "rustflags = \"\"\"\n--cap-\\\n    lints allow\n\"\"\"\n",
                "fixture"
            )
            .is_err()
        );
        assert_eq!(
            require_toml_config_respects_unsafe_forbid(
                "rustflags = \"\"\"\n-C opt-\\\n    level=2\n\"\"\"\n",
                "fixture"
            ),
            Ok(())
        );
    }

    #[test]
    fn rustflags_audit_rejects_github_expression_values() {
        // `${{ ... }}` 表达式由 runner 在运行时解析，来源值（matrix / env 等）不受
        // 静态审计覆盖，rustflags 值含表达式一律拒绝。
        assert!(
            require_workflow_respects_unsafe_forbid(
                "env:\n  RUSTFLAGS: ${{ matrix.flags }}\n",
                "workflow"
            )
            .is_err()
        );
        assert!(
            require_workflow_respects_unsafe_forbid(
                "jobs:\n  build:\n    steps:\n      - env:\n          RUSTFLAGS: ${{ env.USER_FLAGS }}\n",
                "workflow"
            )
            .is_err()
        );
    }

    #[test]
    fn rustflags_audit_yaml_explicit_indent_indicator() {
        // YAML 块标量显式缩进指示符（数字与 chomping 两顺序均合法）由解析器折叠，
        // 真解析按最终生效值检出。
        assert!(
            require_workflow_respects_unsafe_forbid(
                "env:\n  RUSTFLAGS: >2-\n    --cap-lints allow\n",
                "workflow"
            )
            .is_err()
        );
        assert!(
            require_workflow_respects_unsafe_forbid(
                "env:\n  RUSTFLAGS: |2\n    --force-warn unsafe_code\n",
                "workflow"
            )
            .is_err()
        );
        assert_eq!(
            require_workflow_respects_unsafe_forbid(
                "env:\n  RUSTFLAGS: >2-\n    -C opt-level=2\n",
                "workflow"
            ),
            Ok(())
        );
    }

    #[test]
    fn source_includes_reject_macro_rules_metavariable_invocation() {
        // 元变量作为宏名（`$m!`）在展开期生成任意宏调用，include 闭合与 unsafe
        // 扫描都会被绕过，整个源文件拒绝。
        let (sandbox, source, roots) = temp_source_fixture(
            "macro-meta",
            &[("pkg/src/payload.rs", "pub fn payload() {}\n")],
        );
        let evil = "macro_rules! call_macro { ($m:ident, $p:literal) => { $m!($p) } }\ncall_macro!(\"payload.rs\")\n";
        assert_source_err(&sandbox, &source, &roots, evil);
        // 正常 macro_rules（matcher 元变量、模板内具名宏调用、重复展开）不误伤。
        let (sandbox, source, roots) = temp_source_fixture("macro-ok", &[]);
        let benign = "macro_rules! twice { ($x:expr) => { $x + $x } }\nmacro_rules! with_vec { ($($x:expr),*) => { vec![$($x),*] } }\n";
        assert_source_ok(&sandbox, &source, &roots, benign);
    }

    #[test]
    fn rustflags_audit_rejects_script_assignments() {
        // run 脚本内的 rustflags 赋值（shell 前缀 / export / pwsh 形态）无法做
        // shell 语义级静态审计，一律拒绝。
        assert!(
            require_workflow_respects_unsafe_forbid(
                "jobs:\n  build:\n    steps:\n      - run: RUSTFLAGS='--cap-lints allow' cargo check\n",
                "workflow"
            )
            .is_err()
        );
        assert!(
            require_workflow_respects_unsafe_forbid(
                "jobs:\n  build:\n    steps:\n      - run: export RUSTFLAGS=\"--force-warn unsafe_code\"\n",
                "workflow"
            )
            .is_err()
        );
        assert!(
            require_workflow_respects_unsafe_forbid(
                "jobs:\n  build:\n    steps:\n      - run: $env:RUSTFLAGS = '-A unsafe_code'\n",
                "workflow"
            )
            .is_err()
        );
        // 读取引用（`$RUSTFLAGS`）与普通脚本不误伤。
        assert_eq!(
            require_workflow_respects_unsafe_forbid(
                "jobs:\n  build:\n    steps:\n      - run: echo \"$RUSTFLAGS\" && cargo check\n",
                "workflow"
            ),
            Ok(())
        );
    }

    #[test]
    fn rustflags_audit_recognizes_cargo_environment_spellings() {
        // CARGO_BUILD_RUSTFLAGS / CARGO_TARGET_<TRIPLE>_RUSTFLAGS /
        // CARGO_ENCODED_RUSTFLAGS 经 Cargo 环境变量配置层生效（cargo check -vv
        // 实测确认进入 rustc 命令行），与 RUSTFLAGS 同罪。
        assert!(
            require_workflow_respects_unsafe_forbid(
                "env:\n  CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS: --cap-lints allow\n",
                "workflow"
            )
            .is_err()
        );
        assert!(
            require_workflow_respects_unsafe_forbid(
                "env:\n  CARGO_BUILD_RUSTFLAGS: --force-warn unsafe_code\n",
                "workflow"
            )
            .is_err()
        );
        assert!(
            require_toml_config_respects_unsafe_forbid(
                "[env]\nCARGO_ENCODED_RUSTFLAGS = \"--cap-lints\\u001fallow\"\n",
                "fixture"
            )
            .is_err()
        );
        // 非 rustflags 的 CARGO_* 变量不误伤。
        assert_eq!(
            require_workflow_respects_unsafe_forbid("env:\n  CARGO_BUILD_JOBS: 4\n", "workflow"),
            Ok(())
        );
    }

    #[test]
    fn source_includes_reject_macro_rules_metavariable_attribute() {
        // 元变量作为属性内容（`#[$a]`）：`with_attr!(path = "...")` 在展开期生成
        // 真实 #[path] 属性，与元变量宏名调用同罪。
        let (sandbox, source, roots) = temp_source_fixture(
            "macro-attr",
            &[("pkg/src/payload.rs", "pub fn payload() {}\n")],
        );
        let evil = "macro_rules! with_attr { ($a:meta) => { #[$a] mod payload; } }\nwith_attr!(path = \"payload.rs\");\n";
        assert_source_err(&sandbox, &source, &roots, evil);
        let (sandbox, source, roots) = temp_source_fixture("macro-attr", &[]);
        let evil_inner = "macro_rules! with_attr { ($a:meta) => { #![$a] } }\n";
        assert_source_err(&sandbox, &source, &roots, evil_inner);
        // 模板内具名属性不误伤。
        let (sandbox, source, roots) = temp_source_fixture("macro-attr-ok", &[]);
        let benign = "macro_rules! stamped { ($x:item) => { #[allow(dead_code)] $x } }\n";
        assert_source_ok(&sandbox, &source, &roots, benign);
    }
}
