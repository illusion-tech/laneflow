//! Wire / toolchain 审计边界（#376）。
//!
//! 在两个 wire 家族的 codegen clean-regeneration 检查之外，独立审计工具链侧通道：
//! Cargo.lock resolved graph 钉版（version/source/checksum）、wire manifest 显式
//! target 不得逃逸 package 根目录且不得引入 build 脚本、Rust 源经 include 宏与
//! `#[path]` 属性引入的目标必须是 `.rs`、仓库 Cargo 配置与 workflow rustflags 不得
//! 削弱 workspace `unsafe_code = "forbid"` 边界。

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use crate::schema_codegen;

const FLATBUFFERS_LOCK_SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";
const FLATBUFFERS_LOCK_CHECKSUM: &str =
    "35f6839d7b3b98adde531effaf34f0c2badc6f4735d26fe74709d8e513a96ef3";

const WIRE_TARGET_SECTIONS: [&str; 5] = ["lib", "bin", "test", "bench", "example"];

/// 出现在 rustflags 值中即判定削弱 `unsafe_code = "forbid"` 的 token（小写匹配）。
const RUSTFLAGS_WEAKENING_TOKENS: [&str; 5] = [
    "--cap-lints",
    "-a unsafe",
    "-aunsafe",
    "--allow unsafe",
    "--allow=unsafe",
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
/// 不得出现 build 脚本键；package 根目录不得存在 build.rs。
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
        if section == "package" && key.trim() == "build" {
            return Err(format!("wire manifest `{label}` 不得声明 build 脚本键"));
        }
        if WIRE_TARGET_SECTIONS.contains(&section) && key.trim() == "path" {
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
    let mut sources = Vec::new();
    for root in &package_roots {
        schema_codegen::collect_extension_files(root, OsStr::new("rs"), &mut sources)?;
    }
    for source in sources {
        let text = fs::read_to_string(&source)
            .map_err(|error| format!("无法读取 `{}`: {error}", source.display()))?;
        require_audited_source_includes(&text, &source.display().to_string())?;
    }
    Ok(())
}

/// 审计单个 Rust 源文本：include 宏的参数必须是静态字符串字面量且以 `.rs` 结尾
/// （`include_bytes!` / `include_str!` 不引入 Rust 源，不在此列）；`#[path]` 属性
/// 引入的目标必须以 `.rs` 结尾。无法静态确认目标时一律拒绝。
fn require_audited_source_includes(text: &str, label: &str) -> Result<(), String> {
    audit_include_macros(text, label)?;
    audit_path_attributes(text, label)?;
    Ok(())
}

fn audit_include_macros(text: &str, label: &str) -> Result<(), String> {
    let bytes = text.as_bytes();
    let mut cursor = 0;
    while let Some(relative) = text[cursor..].find("include") {
        let start = cursor + relative;
        cursor = start + "include".len();
        let before = text[..start].chars().next_back();
        let after = text[cursor..].chars().next();
        if before.is_some_and(schema_codegen::is_identifier_character)
            || after.is_some_and(schema_codegen::is_identifier_character)
        {
            continue;
        }
        skip_ascii_whitespace(bytes, &mut cursor);
        if bytes.get(cursor) != Some(&b'!') {
            continue;
        }
        cursor += 1;
        skip_ascii_whitespace(bytes, &mut cursor);
        if !matches!(bytes.get(cursor), Some(b'(' | b'[' | b'{')) {
            return Err(format!("`{label}` 含有无法静态审计的 include 宏调用"));
        }
        cursor += 1;
        skip_ascii_whitespace(bytes, &mut cursor);
        let Some((target, next)) = read_string_literal(text, cursor) else {
            return Err(format!(
                "`{label}` 的 include 宏参数不是字符串字面量，无法静态审计"
            ));
        };
        if !target.ends_with(".rs") {
            return Err(format!("`{label}` 的 include 宏引入非 .rs 目标 `{target}`"));
        }
        cursor = next;
    }
    Ok(())
}

fn audit_path_attributes(text: &str, label: &str) -> Result<(), String> {
    let bytes = text.as_bytes();
    let mut cursor = 0;
    while let Some(relative) = text[cursor..].find("#[") {
        let start = cursor + relative;
        cursor = start + 2;
        skip_ascii_whitespace(bytes, &mut cursor);
        if !text[cursor..].starts_with("path") {
            continue;
        }
        cursor += "path".len();
        let after = text[cursor..].chars().next();
        if after.is_some_and(schema_codegen::is_identifier_character) {
            continue;
        }
        skip_ascii_whitespace(bytes, &mut cursor);
        if bytes.get(cursor) != Some(&b'=') {
            // rustc 只接受字符串字面量形态的 path 属性，其他形态无法通过编译。
            continue;
        }
        cursor += 1;
        skip_ascii_whitespace(bytes, &mut cursor);
        let Some((target, next)) = read_string_literal(text, cursor) else {
            return Err(format!(
                "`{label}` 的 #[path] 属性值不是字符串字面量，无法静态审计"
            ));
        };
        if !target.ends_with(".rs") {
            return Err(format!(
                "`{label}` 的 #[path] 属性引入非 .rs 目标 `{target}`"
            ));
        }
        cursor = next;
    }
    Ok(())
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

/// 审计单份 Cargo 配置或 workflow 文本：任何 rustflags / RUSTFLAGS 赋值（含多行
/// 数组与 YAML 折叠标量的延续行）不得包含削弱 `unsafe_code = "forbid"` 的 token。
fn require_rustflags_respect_unsafe_forbid(text: &str, label: &str) -> Result<(), String> {
    let mut pending = false;
    for (number, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        let lower = line.to_lowercase();
        let opens = lower.contains("rustflags");
        if (opens || pending)
            && let Some(token) = RUSTFLAGS_WEAKENING_TOKENS
                .iter()
                .find(|token| lower.contains(**token))
        {
            return Err(format!(
                "`{label}` 第 {} 行 rustflags 含 `{token}`，会削弱 workspace `unsafe_code = \"forbid\"` 边界",
                number + 1
            ));
        }
        pending = (opens || pending) && continues_flag_value(line);
    }
    Ok(())
}

/// 判断一行是否把 rustflags 值延续到下一行（TOML 数组 / 内联表 / YAML 块标量）。
fn continues_flag_value(line: &str) -> bool {
    let mut chars = line.chars().rev();
    let Some(last) = chars.next() else {
        return false;
    };
    if matches!(last, '[' | ',' | '{' | '=' | '|' | '>') {
        return true;
    }
    matches!(last, '-' | '+') && matches!(chars.next(), Some('|' | '>'))
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
    fn source_includes_accept_rs_targets_and_unrelated_tokens() {
        assert_eq!(
            require_audited_source_includes("mod generated;\nfn main() {}\n", "fixture"),
            Ok(())
        );
        let include_rs = concat!("include", "!(\"payload.rs\")");
        assert_eq!(
            require_audited_source_includes(include_rs, "fixture"),
            Ok(())
        );
        let include_bytes = concat!("include", "_bytes!(\"payload.bin\")");
        assert_eq!(
            require_audited_source_includes(include_bytes, "fixture"),
            Ok(())
        );
        let path_rs = concat!("#[", "path = \"support/helper.rs\"]\nmod helper;");
        assert_eq!(require_audited_source_includes(path_rs, "fixture"), Ok(()));
        let raw_path_rs = concat!("#[", "path = r\"support.rs\"]\nmod support;");
        assert_eq!(
            require_audited_source_includes(raw_path_rs, "fixture"),
            Ok(())
        );
    }

    #[test]
    fn source_includes_reject_non_rs_and_unauditable_targets() {
        let bad_include = concat!("include", "!(\"payload.bin\")");
        assert!(require_audited_source_includes(bad_include, "fixture").is_err());
        let concat_include = concat!("include", "!(concat!(env!(\"OUT_DIR\"), \"/x.rs\"))");
        assert!(require_audited_source_includes(concat_include, "fixture").is_err());
        let bare_include = concat!("include", "! module");
        assert!(require_audited_source_includes(bare_include, "fixture").is_err());
        let bad_path = concat!("#[", "path = \"payload.bin\"]\nmod payload;");
        assert!(require_audited_source_includes(bad_path, "fixture").is_err());
        let bad_raw_path = concat!("#[", "path = r#\"payload.dat\"#]\nmod payload;");
        assert!(require_audited_source_includes(bad_raw_path, "fixture").is_err());
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
}
