//! Wire / toolchain 审计边界（#376）。
//!
//! 不变量：workspace 内除钉版 flatc 生成物外没有 unsafe 代码，且该边界不能被
//! 配置 / 环境注入削弱。机制：
//!
//! 1. Cargo.lock flatbuffers resolved 钉版唯一性（有界文本检查）。
//! 2. 两个 wire crate 只承载钉版生成物，全部在 required check（本审计）内闭合：
//!    manifest 卫生（真 TOML 解析两个已知 manifest——[package] 必须存在、不得
//!    声明 build 脚本键、[lib] 固定指向 src/lib.rs 包装器、不得声明
//!    bin/test/bench/example target 段、[dependencies] 恰好一条
//!    `flatbuffers = { version = "=<钉版>", default-features = false,
//!    features = ["std"] }` 且无 dev/build 依赖段、package 根目录不得存在
//!    build.rs）；包装器 lib.rs 与 `xtask/src/wire_pins/` 下钉版副本字节相等；
//!    生成 .rs 的 sha256 与钉版常量一致（字节正确性在本审计闭合；
//!    schema+flatc → bytes 的语义对应另由 schema-codegen.yml 的
//!    clean-regeneration 证明）。这些钉版叠加后，wire crate 内不存在任何
//!    可写入手写 unsafe 的载体。
//! 3. workspace 成员 lint 分类断言（真 TOML 解析每个成员 manifest）：继承
//!    workspace `unsafe_code = "forbid"` 的成员构成 forbid 集；唯一登记的
//!    deny 例外是 laneflow-format（已审计的只读 mmap，文件级 allow 机制，
//!    其例外文件的精确内容见第 6 条）；仅两个 wire crate 允许
//!    `[lints.rust] unsafe_code = "allow"`。deny 集与 allow 集逐一与登记
//!    名单比对，新增例外或改类一律 fail closed。
//! 4. forbid / deny 成员的每个 target（lib/bin/test/bench/example）以
//!    hermetic 编译验证：剔除注入向量的环境（RUSTFLAGS /
//!    CARGO_ENCODED_RUSTFLAGS / CARGO_BUILD_RUSTFLAGS /
//!    CARGO_TARGET_*_RUSTFLAGS / RUSTC / *_WRAPPER / CARGO_BUILD_RUSTC* /
//!    RUSTC_BOOTSTRAP）、仓库外临时 cwd（仓库内 .cargo/config.toml 因 cargo
//!    配置按 cwd 向上发现而不可达）、默认特性集与 `--all-features` 全特性集
//!    各跑一遍（`cfg(feature)` / `cfg(not(feature))` 互补分支都必须进入
//!    编译单元）、尾参 `-F unsafe_code`（forbid 集）/
//!    `-D unsafe_code`（deny 集）。cargo 尾参（`--` 之后）优先级高于
//!    manifest [lints]、env rustflags 与 .cargo/config.toml rustflags
//!    （金丝雀逐次复核，见下），因此一切文本形态绕过（转义、拆分数组、
//!    宏元变量、shell 构造、include / #[path] 形态）对本门禁无效：无论源码
//!    以何形态进入编译单元，都在同一 crate 编译中过 lint。
//! 5. 尾参优先级语义不依赖文档假设：正式检查前先跑三个金丝雀 crate
//!    （manifest [lints] 注入 / RUSTFLAGS 环境注入 / cwd .cargo/config.toml
//!    注入），三者都必须被尾参击败（编译失败且 stderr 指向 unsafe）；任一
//!    金丝雀编译成功或以非 unsafe 原因失败，即判定工具链行为改变，fail closed。
//! 6. laneflow-format 的 mmap 例外由
//!    `schema_codegen::check_audited_mmap_sources` 在本审计内复核：
//!    例外文件恰好一次模块级 allow 与一次固定只读映射调用，crate 内其他
//!    源文件零 unsafe、零 allow(unsafe_code)；crate 内全面禁止
//!    `#[path]` 模块属性与 `include!` 宏——rustc 仅经此二路加载 .rs 扫描面
//!    之外的源码（如 `#[path = "payload.txt"]`），禁绝后 .rs 全集即编译器
//!    可达源码全集。
//!
//! 残余信任边界（本模块不尝试自审）：本检查步骤的定义（.github/workflows/）、
//! xtask 源码（含 wire_pins 钉版副本）与依赖政策配置（deny.toml）可被 PR
//! 修改，现由普通 PR review 守住；是否以 CODEOWNERS + code owner review
//! 强制，按 ADR 0027 另开治理 Issue 评估（#579）。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::schema_codegen;

const FLATBUFFERS_LOCK_SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";
const FLATBUFFERS_LOCK_CHECKSUM: &str =
    "35f6839d7b3b98adde531effaf34f0c2badc6f4735d26fe74709d8e513a96ef3";

/// wire crate 包装器 lib.rs 的钉版副本（与 crate 内文件字节相等）。
const ROAD_EDITING_WIRE_LIB_RS_PIN: &str = include_str!("wire_pins/road_editing_wire_lib.rs");
const RUNTIME_SNAPSHOT_WIRE_LIB_RS_PIN: &str =
    include_str!("wire_pins/runtime_snapshot_wire_lib.rs");

/// wire crate 生成 .rs 的 sha256 钉版：字节正确性在 required check（本审计）
/// 内闭合；schema+flatc → bytes 的语义对应另由 clean-regeneration 证明。
/// 合法再生成（schema 或 flatc 钉版升级）需在同一 PR 更新本常量，diff 随评审可见。
const ROAD_EDITING_GENERATED_RS_SHA256: &str =
    "763a5419d93f46842152df2d5a71339a135cb207137a539e6266e8bdb970589d";
const RUNTIME_SNAPSHOT_GENERATED_RS_SHA256: &str =
    "4d40bdb2015771fa3ba3650b1eaa99ed52148f49d572dedb7e9a51ca65afe3ff";

/// 分类断言：整个 workspace 只允许这一个 crate 以 `deny` + 文件级例外承载
/// unsafe（laneflow-format 的已审计只读 mmap）。新增 deny crate 必须在此登记，
/// 登记变更随 PR 评审。
const EXPECTED_DENY_UNSAFE_PACKAGES: [&str; 1] = ["laneflow-format"];

/// hermetic 编译前必须剔除的精确环境变量：它们能把 rustc 二进制、wrapper 或
/// 额外 rustflags 注入编译命令行。
const HERMETIC_SCRUB_ENV_EXACT: [&str; 10] = [
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_BUILD_RUSTFLAGS",
    "CARGO_BUILD_RUSTC",
    "CARGO_BUILD_RUSTC_WRAPPER",
    "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
    "RUSTC",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "RUSTC_BOOTSTRAP",
];

/// 精确名单之外，按 cargo 配置环境变量命名规则剔除
/// `CARGO_TARGET_<TRIPLE>_RUSTFLAGS`（triple 任意）。
fn is_hermetic_scrubbed_env_key(key: &str) -> bool {
    HERMETIC_SCRUB_ENV_EXACT.contains(&key)
        || (key.starts_with("CARGO_TARGET_") && key.ends_with("_RUSTFLAGS"))
}

pub(crate) fn run() -> Result<(), String> {
    let repository_root =
        std::env::current_dir().map_err(|error| format!("无法解析仓库根目录: {error}"))?;
    require_repository_root(&repository_root)?;
    check_flatbuffers_lockfile_pin(&repository_root)?;
    check_wire_manifest_hygiene(&repository_root)?;
    check_wire_lib_rs_pins(&repository_root)?;
    check_generated_rs_pins(&repository_root)?;
    schema_codegen::check_audited_mmap_sources(&repository_root)?;
    check_workspace_unsafe_boundary(&repository_root)?;
    println!(
        "wire 工具链审计已通过：flatbuffers 钉版闭合，wire crate 包装器/生成物/依赖表钉版闭合，mmap 例外复核闭合（含 #[path]/include! 加载入口禁令），workspace unsafe 分类断言闭合，forbid/deny 成员全部 target（默认+全特性双配置，含 example）通过 hermetic 编译（含三路注入金丝雀复核）"
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

fn wire_families() -> [&'static schema_codegen::WireFamily; 2] {
    [
        &schema_codegen::ROAD_EDITING,
        &schema_codegen::RUNTIME_SNAPSHOT,
    ]
}

fn check_wire_manifest_hygiene(repository_root: &Path) -> Result<(), String> {
    for family in wire_families() {
        let manifest_text = fs::read_to_string(repository_root.join(family.wire_manifest_path))
            .map_err(|error| format!("无法读取 `{}`: {error}", family.wire_manifest_path))?;
        require_wire_manifest_hygiene(
            &manifest_text,
            &repository_root.join(family.wire_package_root),
            family.wire_manifest_path,
        )?;
    }
    Ok(())
}

/// wire manifest 卫生：真 TOML 解析（点号键 / 引号键 / 转义键 / 内联表统一由
/// 解析器归一化，解析失败 fail closed）；[package] 必须存在且不得声明 build
/// 脚本键；[lib] 必须固定指向 src/lib.rs 包装器（否则 lib.rs 钉版失去意义）；
/// package 根目录不得存在 build.rs。
fn require_wire_manifest_hygiene(
    manifest_text: &str,
    package_root: &Path,
    label: &str,
) -> Result<(), String> {
    let manifest: toml::Table = manifest_text
        .parse()
        .map_err(|error| format!("wire manifest `{label}` TOML 解析失败，无法静态审计: {error}"))?;
    let package = manifest
        .get("package")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| format!("wire manifest `{label}` 缺少 [package] 段"))?;
    if package.contains_key("build") {
        return Err(format!("wire manifest `{label}` 不得声明 build 脚本键"));
    }
    let lib_path = manifest
        .get("lib")
        .and_then(toml::Value::as_table)
        .and_then(|lib| lib.get("path"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| format!("wire manifest `{label}` 缺少 [lib].path"))?;
    if lib_path != "src/lib.rs" {
        return Err(format!(
            "wire manifest `{label}` 的 [lib].path 必须固定为 `src/lib.rs`，实际 `{lib_path}`"
        ));
    }
    // wire crate 只许 [lib] 包装器一个 target：AllowGenerated 分类不参与 hermetic
    // 编译，任何额外 target 段都会成为不受 lint 约束的编译入口。
    for section in ["bin", "test", "bench", "example"] {
        if manifest.get(section).is_some() {
            return Err(format!(
                "wire manifest `{label}` 不得声明 {section} target 段（wire crate 只许 [lib] 包装器）"
            ));
        }
    }
    require_wire_flatbuffers_dep(&manifest, label)?;
    if package_root.join("build.rs").is_file() {
        return Err(format!("wire package `{label}` 不得包含 build.rs"));
    }
    Ok(())
}

/// wire crate 依赖表钉版：[dependencies] 必须恰好一条 flatbuffers，且为恰三键
/// 内联表形态（version 精确钉版 / default-features = false / features = ["std"]）。
/// 键数不符即拒绝——`package = "..."` 改名注入会多出第四键；[dev-dependencies]
/// 与 [build-dependencies] 段必须不存在（wire crate 无 dev/build 依赖面）。
fn require_wire_flatbuffers_dep(manifest: &toml::Table, label: &str) -> Result<(), String> {
    let dependencies = manifest
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| format!("wire manifest `{label}` 缺少 [dependencies] 段"))?;
    if dependencies.len() != 1 {
        return Err(format!(
            "wire manifest `{label}` 的 [dependencies] 必须恰好一条 flatbuffers，实际 {} 条",
            dependencies.len()
        ));
    }
    let flatbuffers = dependencies
        .get("flatbuffers")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| format!("wire manifest `{label}` 的 flatbuffers 依赖必须是内联表形态"))?;
    if flatbuffers.len() != 3 {
        return Err(format!(
            "wire manifest `{label}` 的 flatbuffers 依赖必须恰好 version/default-features/features 三键（禁止 `package` 改名等注入），实际 {} 键",
            flatbuffers.len()
        ));
    }
    let expected_version = format!("={}", schema_codegen::FLATBUFFERS_VERSION);
    let version = flatbuffers
        .get("version")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| format!("wire manifest `{label}` 的 flatbuffers 依赖缺少 version 键"))?;
    if version != expected_version {
        return Err(format!(
            "wire manifest `{label}` 的 flatbuffers version 必须精确钉为 `{expected_version}`，实际 `{version}`"
        ));
    }
    if flatbuffers
        .get("default-features")
        .and_then(toml::Value::as_bool)
        != Some(false)
    {
        return Err(format!(
            "wire manifest `{label}` 的 flatbuffers 依赖必须 `default-features = false`"
        ));
    }
    let features = flatbuffers
        .get("features")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| format!("wire manifest `{label}` 的 flatbuffers 依赖缺少 features 键"))?;
    if features.len() != 1 || features[0].as_str() != Some("std") {
        return Err(format!(
            "wire manifest `{label}` 的 flatbuffers 依赖 features 必须恰好 `[\"std\"]`"
        ));
    }
    for section in ["dev-dependencies", "build-dependencies"] {
        if manifest.contains_key(section) {
            return Err(format!(
                "wire manifest `{label}` 不得声明 [{section}] 段（wire crate 无 dev/build 依赖面）"
            ));
        }
    }
    Ok(())
}

/// wire crate 的手写载体只有包装器 lib.rs；它与钉版副本字节相等，因此
/// 其中不可能出现未审计内容。合法变更（flatc 升级导致模块形态变化等）必须
/// 在同一 PR 同步更新钉版副本，diff 随评审可见。
fn check_wire_lib_rs_pins(repository_root: &Path) -> Result<(), String> {
    for (family, pin) in [
        (&schema_codegen::ROAD_EDITING, ROAD_EDITING_WIRE_LIB_RS_PIN),
        (
            &schema_codegen::RUNTIME_SNAPSHOT,
            RUNTIME_SNAPSHOT_WIRE_LIB_RS_PIN,
        ),
    ] {
        let lib_rs_path = repository_root
            .join(family.wire_package_root)
            .join("src")
            .join("lib.rs");
        let actual = fs::read(&lib_rs_path)
            .map_err(|error| format!("无法读取 `{}`: {error}", lib_rs_path.display()))?;
        if actual != pin.as_bytes() {
            return Err(format!(
                "wire crate `{}` 的 src/lib.rs 与钉版副本 xtask/src/wire_pins/ 不符；合法变更需在同一 PR 更新钉版副本",
                family.wire_package_name
            ));
        }
    }
    Ok(())
}

/// 生成 .rs 钉版：逐字节读入两个 wire family 的生成物并比对 sha256 常量。
/// 生成物是 clean-regeneration 的对象，但本审计不依赖 CI 时再生成一次——
/// 常量即钉版；flatc 升级等合法再生成必须在同一 PR 更新常量，diff 随评审可见。
fn check_generated_rs_pins(repository_root: &Path) -> Result<(), String> {
    use sha2::Digest;
    for (family, expected) in [
        (
            &schema_codegen::ROAD_EDITING,
            ROAD_EDITING_GENERATED_RS_SHA256,
        ),
        (
            &schema_codegen::RUNTIME_SNAPSHOT,
            RUNTIME_SNAPSHOT_GENERATED_RS_SHA256,
        ),
    ] {
        let generated_path = repository_root.join(family.checked_rust_path);
        let bytes = fs::read(&generated_path)
            .map_err(|error| format!("无法读取 `{}`: {error}", generated_path.display()))?;
        let digest: [u8; 32] = sha2::Sha256::digest(&bytes).into();
        let mut actual = String::with_capacity(64);
        for byte in digest {
            use std::fmt::Write;
            write!(&mut actual, "{byte:02x}").expect("写入 String 不会失败");
        }
        if actual != expected {
            return Err(format!(
                "生成物 `{}` 的 sha256 与钉版常量不符（预期 `{expected}`，实际 `{actual}`）；合法再生成需在同一 PR 更新 xtask 审计常量",
                family.checked_rust_path
            ));
        }
    }
    Ok(())
}

/// workspace 成员的 unsafe lint 分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnsafeLevel {
    /// 继承 workspace `unsafe_code = "forbid"`：hermetic 尾参 `-F`。
    Forbid,
    /// 自有 `[lints.rust] unsafe_code = "deny"`（登记例外）：hermetic 尾参 `-D`，
    /// 保留其文件级 allow 例外机制。
    Deny,
    /// 自有 `[lints.rust] unsafe_code = "allow"`：只允许两个纯生成物 wire crate，
    /// 不参与 hermetic 编译（其边界由钉版与 clean-regeneration 闭合）。
    AllowGenerated,
}

impl UnsafeLevel {
    fn tail_flag(self) -> Option<&'static str> {
        match self {
            UnsafeLevel::Forbid => Some("-F"),
            UnsafeLevel::Deny => Some("-D"),
            UnsafeLevel::AllowGenerated => None,
        }
    }
}

/// 真 TOML 解析成员 manifest 的 lint 分类；未分类形态 fail closed。
fn classify_member_lints(manifest_text: &str, label: &str) -> Result<UnsafeLevel, String> {
    let manifest: toml::Table = manifest_text
        .parse()
        .map_err(|error| format!("成员 manifest `{label}` TOML 解析失败: {error}"))?;
    let Some(lints) = manifest.get("lints").and_then(toml::Value::as_table) else {
        return Err(format!(
            "成员 manifest `{label}` 没有 [lints] 段，无法分类 unsafe 边界，fail closed"
        ));
    };
    if lints.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
        return Ok(UnsafeLevel::Forbid);
    }
    match lints
        .get("rust")
        .and_then(toml::Value::as_table)
        .and_then(|rust| rust.get("unsafe_code"))
        .and_then(toml::Value::as_str)
    {
        Some("deny") => Ok(UnsafeLevel::Deny),
        Some("allow") => Ok(UnsafeLevel::AllowGenerated),
        _ => Err(format!(
            "成员 manifest `{label}` 的 [lints] 既非 workspace 继承也非已登记的 unsafe_code deny/allow 形态，fail closed"
        )),
    }
}

/// 断言 deny 集与 allow 集与登记名单完全一致；任何新增例外或改类都使审计失败。
fn require_expected_classification(classified: &[(String, UnsafeLevel)]) -> Result<(), String> {
    let mut deny: Vec<&str> = classified
        .iter()
        .filter(|(_, level)| *level == UnsafeLevel::Deny)
        .map(|(name, _)| name.as_str())
        .collect();
    deny.sort_unstable();
    if deny != EXPECTED_DENY_UNSAFE_PACKAGES {
        return Err(format!(
            "workspace unsafe_code = \"deny\" crate 集合 {deny:?} 与登记名单 {EXPECTED_DENY_UNSAFE_PACKAGES:?} 不符；新增例外必须在 xtask 审计常量登记并随 PR 评审"
        ));
    }
    let mut allow: Vec<&str> = classified
        .iter()
        .filter(|(_, level)| *level == UnsafeLevel::AllowGenerated)
        .map(|(name, _)| name.as_str())
        .collect();
    allow.sort_unstable();
    let mut expected_allow: Vec<&str> = wire_families()
        .map(|family| family.wire_package_name)
        .to_vec();
    expected_allow.sort_unstable();
    if allow != expected_allow {
        return Err(format!(
            "workspace unsafe_code = \"allow\" crate 集合 {allow:?} 与 wire crate 名单 {expected_allow:?} 不符；`allow` 只许钉版生成物 crate 使用"
        ));
    }
    Ok(())
}

/// 单个编译调用形态：`--lib`（无名字）或 `--bin` / `--test` / `--bench` /
/// `--example`（带名）。`required_features` 非空的 target 在默认特性集下不可
/// 编译（cargo 直接拒绝），默认集那遍跳过，由全特性集那遍覆盖。
#[derive(Debug, PartialEq, Eq)]
struct TargetInvocation {
    flag: &'static str,
    name: Option<String>,
    label: String,
    required_features: Vec<String>,
}

/// workspace 成员 package（来自 cargo metadata）。
#[derive(Debug)]
struct MemberPackage {
    name: String,
    manifest_path: PathBuf,
    targets: Vec<TargetInvocation>,
}

/// 从 workspace cargo metadata（--no-deps）解析全部成员的 name/manifest_path/
/// 编译 target。custom-build（build 脚本 target 不可单独编译）跳过；lib/bin/
/// test/bench/example 全部纳入 hermetic 编译；未知 kind fail closed。
fn parse_workspace_members(metadata: &serde_json::Value) -> Result<Vec<MemberPackage>, String> {
    let packages = metadata
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "workspace cargo metadata 缺少 packages 数组".to_string())?;
    let mut members = Vec::new();
    for package in packages {
        let name = package
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "workspace metadata package 缺少 name".to_string())?
            .to_string();
        let manifest_path = package
            .get("manifest_path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("workspace metadata package `{name}` 缺少 manifest_path"))?;
        let targets = package
            .get("targets")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| format!("workspace metadata package `{name}` 缺少 targets 数组"))?;
        let mut invocations = Vec::new();
        for target in targets {
            let target_name = target
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    format!("workspace metadata package `{name}` 的 target 缺少 name")
                })?;
            let kinds: Vec<&str> = target
                .get("kind")
                .and_then(serde_json::Value::as_array)
                .map(|kinds| kinds.iter().filter_map(serde_json::Value::as_str).collect())
                .ok_or_else(|| {
                    format!(
                        "workspace metadata package `{name}` 的 target `{target_name}` 缺少 kind"
                    )
                })?;
            if kinds.contains(&"custom-build") {
                continue;
            }
            let required_features = match target.get("required-features") {
                None => Vec::new(),
                Some(value) => value
                    .as_array()
                    .ok_or_else(|| {
                        format!(
                            "workspace metadata package `{name}` 的 target `{target_name}` required-features 不是数组，fail closed"
                        )
                    })?
                    .iter()
                    .map(|feature| {
                        feature.as_str().map(str::to_string).ok_or_else(|| {
                            format!(
                                "workspace metadata package `{name}` 的 target `{target_name}` required-features 含非字符串项，fail closed"
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            };
            let (flag, name, label) = if kinds
                .iter()
                .any(|kind| *kind == "lib" || *kind == "proc-macro")
            {
                ("--lib", None, "lib".to_string())
            } else if kinds.contains(&"bin") {
                (
                    "--bin",
                    Some(target_name.to_string()),
                    format!("bin `{target_name}`"),
                )
            } else if kinds.contains(&"test") {
                (
                    "--test",
                    Some(target_name.to_string()),
                    format!("test `{target_name}`"),
                )
            } else if kinds.contains(&"bench") {
                (
                    "--bench",
                    Some(target_name.to_string()),
                    format!("bench `{target_name}`"),
                )
            } else if kinds.contains(&"example") {
                (
                    "--example",
                    Some(target_name.to_string()),
                    format!("example `{target_name}`"),
                )
            } else {
                return Err(format!(
                    "workspace package `{name}` 的 target `{target_name}` kind {kinds:?} 未知，fail closed"
                ));
            };
            invocations.push(TargetInvocation {
                flag,
                name,
                label,
                required_features,
            });
        }
        members.push(MemberPackage {
            name,
            manifest_path: PathBuf::from(manifest_path),
            targets: invocations,
        });
    }
    if members.is_empty() {
        return Err("workspace cargo metadata 不含任何 package，fail closed".to_string());
    }
    Ok(members)
}

fn check_workspace_unsafe_boundary(repository_root: &Path) -> Result<(), String> {
    let audit_root =
        std::env::temp_dir().join(format!("laneflow-wire-audit-{}", std::process::id()));
    if audit_root.exists() {
        fs::remove_dir_all(&audit_root)
            .map_err(|error| format!("无法清理审计临时目录 `{}`: {error}", audit_root.display()))?;
    }
    fs::create_dir_all(&audit_root)
        .map_err(|error| format!("无法创建审计临时目录 `{}`: {error}", audit_root.display()))?;
    let result = run_workspace_unsafe_boundary(repository_root, &audit_root);
    let _ = fs::remove_dir_all(&audit_root);
    result
}

fn run_workspace_unsafe_boundary(repository_root: &Path, audit_root: &Path) -> Result<(), String> {
    run_injection_canaries(audit_root)?;
    let root_manifest = repository_root.join("Cargo.toml");
    let metadata = run_hermetic_metadata(&root_manifest, audit_root)?;
    let members = parse_workspace_members(&metadata)?;
    let mut classified = Vec::with_capacity(members.len());
    for member in &members {
        let manifest_text = fs::read_to_string(&member.manifest_path).map_err(|error| {
            format!(
                "无法读取成员 manifest `{}`: {error}",
                member.manifest_path.display()
            )
        })?;
        let level =
            classify_member_lints(&manifest_text, &member.manifest_path.display().to_string())?;
        classified.push((member.name.clone(), level));
    }
    require_expected_classification(&classified)?;
    let target_dir = repository_root.join("target");
    for (member, (_, level)) in members.iter().zip(classified.iter()) {
        let Some(flag) = level.tail_flag() else {
            continue;
        };
        for target in &member.targets {
            // 默认特性集与全特性集各跑一遍：`cfg(feature)` 与
            // `cfg(not(feature))` 互补分支都要经过不可覆盖的 lint。
            // 带 required-features 的 target 默认集下不可编译，由全集那遍覆盖。
            for (all_features, config_label) in [(false, "默认特性集"), (true, "全特性集")]
            {
                if !all_features && !target.required_features.is_empty() {
                    continue;
                }
                let output = run_unsafe_level_compile(UnsafeCompile {
                    root_manifest: &root_manifest,
                    package_name: &member.name,
                    target_dir: &target_dir,
                    work_dir: audit_root,
                    target_flag: target.flag,
                    target_name: target.name.as_deref(),
                    level_flag: flag,
                    extra_env: &[],
                    locked: true,
                    all_features,
                })?;
                if !output.status.success() {
                    return Err(format!(
                        "workspace 成员 `{}` target `{}`（{config_label}）未通过 hermetic `{flag} unsafe_code` 编译：\n{}",
                        member.name,
                        target.label,
                        stderr_tail(&output)
                    ));
                }
            }
        }
    }
    Ok(())
}

/// 构造剔除注入向量后的 cargo 命令。cwd 固定为审计临时目录（仓库外），
/// 使仓库内 .cargo/config.toml 不参与配置发现。
fn hermetic_cargo_command(work_dir: &Path) -> Command {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command.current_dir(work_dir);
    for key in HERMETIC_SCRUB_ENV_EXACT {
        command.env_remove(key);
    }
    let patterned: Vec<std::ffi::OsString> = std::env::vars_os()
        .map(|(key, _)| key)
        .filter(|key| key.to_str().is_some_and(is_hermetic_scrubbed_env_key))
        .collect();
    for key in patterned {
        command.env_remove(key);
    }
    command
}

struct UnsafeCompile<'a> {
    root_manifest: &'a Path,
    package_name: &'a str,
    target_dir: &'a Path,
    work_dir: &'a Path,
    target_flag: &'a str,
    target_name: Option<&'a str>,
    level_flag: &'a str,
    extra_env: &'a [(&'a str, &'a str)],
    locked: bool,
    all_features: bool,
}

/// 以尾参 `-F` / `-D unsafe_code` 编译指定成员 target。尾参位于 rustc 命令行
/// 最末，优先级高于 manifest [lints]、env rustflags 与 .cargo/config.toml
/// rustflags（由金丝雀逐次运行复核）。`all_features` 控制是否带
/// `--all-features`：默认特性集与全特性集互补，`cfg(feature)` /
/// `cfg(not(feature))` 两支都可能藏 unsafe，调用方须两种配置各跑一遍。
/// 返回完整 Output 由调用方判定成败。
fn run_unsafe_level_compile(request: UnsafeCompile<'_>) -> Result<Output, String> {
    let mut command = hermetic_cargo_command(request.work_dir);
    command.args(["rustc", "--profile", "check"]);
    if request.all_features {
        command.arg("--all-features");
    }
    if request.locked {
        command.arg("--locked");
    }
    command
        .arg("--manifest-path")
        .arg(request.root_manifest)
        .arg("-p")
        .arg(request.package_name)
        .arg(request.target_flag);
    if let Some(name) = request.target_name {
        command.arg(name);
    }
    command
        .arg("--target-dir")
        .arg(request.target_dir)
        .arg("--")
        .args([request.level_flag, "unsafe_code"]);
    for (key, value) in request.extra_env {
        command.env(key, value);
    }
    command
        .output()
        .map_err(|error| format!("无法启动 hermetic cargo 编译: {error}"))
}

fn run_hermetic_metadata(
    root_manifest: &Path,
    work_dir: &Path,
) -> Result<serde_json::Value, String> {
    let output = hermetic_cargo_command(work_dir)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--locked",
            "--manifest-path",
        ])
        .arg(root_manifest)
        .output()
        .map_err(|error| format!("无法读取 workspace cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "workspace cargo metadata 失败，exit={}，stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("workspace cargo metadata 不是有效 JSON: {error}"))
}

/// 金丝雀：一个含 unsafe 代码的最小 crate，分别以三种注入向量尝试削弱
/// forbid；三种场景都必须编译失败且 stderr 指向 unsafe，否则门禁 fail closed。
const CANARY_LIB_RS: &str =
    "pub fn canary() { unsafe { let _ = std::ptr::null::<u8>().read(); } }\n";

struct InjectionCanary {
    dir_name: &'static str,
    /// 追加在 [package]/[workspace] 之后的 manifest 片段（注入向量之一）。
    manifest_extra: &'static str,
    /// 写入 <crate>/.cargo/config.toml 的内容（注入向量之一）。
    cwd_config: Option<&'static str>,
    extra_env: &'static [(&'static str, &'static str)],
    vector_label: &'static str,
}

fn run_injection_canaries(audit_root: &Path) -> Result<(), String> {
    let canaries = [
        InjectionCanary {
            dir_name: "canary-manifest-lints",
            manifest_extra: "\n[lints.rust]\nunsafe_code = \"allow\"\n",
            cwd_config: None,
            extra_env: &[],
            vector_label: "manifest [lints] 注入",
        },
        InjectionCanary {
            dir_name: "canary-env-rustflags",
            manifest_extra: "",
            cwd_config: None,
            extra_env: &[("RUSTFLAGS", "-A unsafe_code")],
            vector_label: "RUSTFLAGS 环境注入",
        },
        InjectionCanary {
            dir_name: "canary-cwd-config",
            manifest_extra: "",
            cwd_config: Some("[build]\nrustflags = [\"-A\", \"unsafe_code\"]\n"),
            extra_env: &[],
            vector_label: "cwd .cargo/config.toml 注入",
        },
    ];
    for canary in &canaries {
        let crate_dir = audit_root.join(canary.dir_name);
        fs::create_dir_all(crate_dir.join("src"))
            .map_err(|error| format!("无法创建金丝雀目录 `{}`: {error}", crate_dir.display()))?;
        let manifest = format!(
            "[package]\nname = \"{}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n{}",
            canary.dir_name, canary.manifest_extra
        );
        fs::write(crate_dir.join("Cargo.toml"), manifest)
            .map_err(|error| format!("无法写入金丝雀 manifest: {error}"))?;
        fs::write(crate_dir.join("src").join("lib.rs"), CANARY_LIB_RS)
            .map_err(|error| format!("无法写入金丝雀源码: {error}"))?;
        if let Some(config) = canary.cwd_config {
            fs::create_dir_all(crate_dir.join(".cargo"))
                .map_err(|error| format!("无法创建金丝雀 .cargo 目录: {error}"))?;
            fs::write(crate_dir.join(".cargo").join("config.toml"), config)
                .map_err(|error| format!("无法写入金丝雀 .cargo/config.toml: {error}"))?;
        }
        let output = run_unsafe_level_compile(UnsafeCompile {
            root_manifest: &crate_dir.join("Cargo.toml"),
            package_name: canary.dir_name,
            target_dir: &audit_root.join(format!("{}-target", canary.dir_name)),
            work_dir: &crate_dir,
            target_flag: "--lib",
            target_name: None,
            level_flag: "-F",
            extra_env: canary.extra_env,
            locked: false,
            all_features: true,
        })?;
        if output.status.success() {
            return Err(format!(
                "wire unsafe 金丝雀（{}）编译成功：尾参 `-F unsafe_code` 未能压制注入，工具链优先级语义已改变，门禁 fail closed",
                canary.vector_label
            ));
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("unsafe") {
            return Err(format!(
                "wire unsafe 金丝雀（{}）以非 unsafe 原因失败，无法确认门禁语义，fail closed:\n{}",
                canary.vector_label,
                stderr.trim()
            ));
        }
    }
    Ok(())
}

fn stderr_tail(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let trimmed = stderr.trim();
    const TAIL_LIMIT: usize = 4000;
    if trimmed.len() > TAIL_LIMIT {
        format!("…（截断）\n{}", &trimmed[trimmed.len() - TAIL_LIMIT..])
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hermetic_scrub_env_key_covers_exact_and_target_patterns() {
        for key in HERMETIC_SCRUB_ENV_EXACT {
            assert!(is_hermetic_scrubbed_env_key(key), "{key} 必须剔除");
        }
        assert!(is_hermetic_scrubbed_env_key(
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS"
        ));
        assert!(is_hermetic_scrubbed_env_key("CARGO_TARGET_A_RUSTFLAGS"));
        for safe in [
            "PATH",
            "CARGO_HOME",
            "CARGO_TARGET_DIR",
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER",
            "RUSTUP_TOOLCHAIN",
            "CARGO_PROFILE_CHECK_DEBUG",
        ] {
            assert!(!is_hermetic_scrubbed_env_key(safe), "{safe} 不得剔除");
        }
    }

    #[test]
    fn lockfile_pin_accepts_exact_match() {
        let lock = format!(
            "[[package]]\nname = \"flatbuffers\"\nversion = \"{}\"\nsource = \"{FLATBUFFERS_LOCK_SOURCE}\"\nchecksum = \"{FLATBUFFERS_LOCK_CHECKSUM}\"\n",
            schema_codegen::FLATBUFFERS_VERSION
        );
        require_flatbuffers_lockfile_pin(&lock).unwrap();
    }

    #[test]
    fn lockfile_pin_rejects_checksum_drift() {
        let lock = format!(
            "[[package]]\nname = \"flatbuffers\"\nversion = \"{}\"\nsource = \"{FLATBUFFERS_LOCK_SOURCE}\"\nchecksum = \"{0000}\"\n",
            schema_codegen::FLATBUFFERS_VERSION
        );
        let error = require_flatbuffers_lockfile_pin(&lock).unwrap_err();
        assert!(error.contains("checksum"), "{error}");
    }

    #[test]
    fn lockfile_pin_rejects_missing_and_duplicate() {
        let error =
            require_flatbuffers_lockfile_pin("[[package]]\nname = \"serde\"\n").unwrap_err();
        assert!(error.contains("缺少 flatbuffers"), "{error}");
        let block = format!(
            "[[package]]\nname = \"flatbuffers\"\nversion = \"{}\"\nsource = \"{FLATBUFFERS_LOCK_SOURCE}\"\nchecksum = \"{FLATBUFFERS_LOCK_CHECKSUM}\"\n",
            schema_codegen::FLATBUFFERS_VERSION
        );
        let doubled = format!("{block}{block}");
        let error = require_flatbuffers_lockfile_pin(&doubled).unwrap_err();
        assert!(error.contains("唯一"), "{error}");
    }

    #[test]
    fn manifest_hygiene_accepts_plain_data_crate() {
        let root = std::env::temp_dir().join(format!(
            "laneflow-wire-audit-test-hygiene-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        require_wire_manifest_hygiene(
            "[package]\nname = \"w\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = { version = \"=25.12.19\", default-features = false, features = [\"std\"] }\n",
            &root,
            "w/Cargo.toml",
        )
        .unwrap();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn manifest_hygiene_rejects_build_key_build_rs_and_lib_path_drift() {
        let root = std::env::temp_dir().join(format!(
            "laneflow-wire-audit-test-hygiene-reject-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let error = require_wire_manifest_hygiene(
            "[package]\nname = \"w\"\nbuild = \"build.rs\"\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = { version = \"=25.12.19\", default-features = false, features = [\"std\"] }\n",
            &root,
            "w/Cargo.toml",
        )
        .unwrap_err();
        assert!(error.contains("build 脚本键"), "{error}");
        let error = require_wire_manifest_hygiene(
            "[package]\nname = \"w\"\n\n[lib]\npath = \"src/generated.rs\"\n\n[dependencies]\nflatbuffers = { version = \"=25.12.19\", default-features = false, features = [\"std\"] }\n",
            &root,
            "w/Cargo.toml",
        )
        .unwrap_err();
        assert!(error.contains("src/lib.rs"), "{error}");
        fs::write(root.join("build.rs"), "fn main() {}\n").unwrap();
        let error = require_wire_manifest_hygiene(
            "[package]\nname = \"w\"\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = { version = \"=25.12.19\", default-features = false, features = [\"std\"] }\n",
            &root,
            "w/Cargo.toml",
        )
        .unwrap_err();
        assert!(error.contains("build.rs"), "{error}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn manifest_hygiene_fails_closed_on_unparseable_toml() {
        let root = Path::new(".");
        let error = require_wire_manifest_hygiene("[package\n", root, "w/Cargo.toml").unwrap_err();
        assert!(error.contains("TOML 解析失败"), "{error}");
        let error =
            require_wire_manifest_hygiene("[dependencies]\n", root, "w/Cargo.toml").unwrap_err();
        assert!(error.contains("[package]"), "{error}");
    }

    #[test]
    fn manifest_hygiene_rejects_extra_target_sections() {
        let root = Path::new(".");
        for section in [
            "[[bin]]\nname = \"x\"\n",
            "[[test]]\nname = \"x\"\n",
            "[[bench]]\nname = \"x\"\n",
            "[[example]]\nname = \"x\"\n",
        ] {
            let manifest = format!(
                "[package]\nname = \"w\"\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = {{ version = \"=25.12.19\", default-features = false, features = [\"std\"] }}\n\n{section}"
            );
            let error = require_wire_manifest_hygiene(&manifest, root, "w/Cargo.toml").unwrap_err();
            assert!(error.contains("只许 [lib] 包装器"), "{error}");
        }
    }

    #[test]
    fn manifest_hygiene_rejects_dependency_table_drift() {
        let root = Path::new(".");
        // package 改名注入：第四键触发拒绝。
        let error = require_wire_manifest_hygiene(
            "[package]\nname = \"w\"\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = { version = \"=25.12.19\", default-features = false, features = [\"std\"], package = \"shim\" }\n",
            root,
            "w/Cargo.toml",
        )
        .unwrap_err();
        assert!(error.contains("三键"), "{error}");
        // 多余依赖条目。
        let error = require_wire_manifest_hygiene(
            "[package]\nname = \"w\"\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = { version = \"=25.12.19\", default-features = false, features = [\"std\"] }\nserde = \"1\"\n",
            root,
            "w/Cargo.toml",
        )
        .unwrap_err();
        assert!(error.contains("恰好一条"), "{error}");
        // 版本未精确钉版。
        let error = require_wire_manifest_hygiene(
            "[package]\nname = \"w\"\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = { version = \"25.12.19\", default-features = false, features = [\"std\"] }\n",
            root,
            "w/Cargo.toml",
        )
        .unwrap_err();
        assert!(error.contains("精确钉"), "{error}");
        // dev-dependencies 段不允许存在。
        let error = require_wire_manifest_hygiene(
            "[package]\nname = \"w\"\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = { version = \"=25.12.19\", default-features = false, features = [\"std\"] }\n\n[dev-dependencies]\ntempfile = \"3\"\n",
            root,
            "w/Cargo.toml",
        )
        .unwrap_err();
        assert!(error.contains("dev-dependencies"), "{error}");
        // build-dependencies 段不允许存在。
        let error = require_wire_manifest_hygiene(
            "[package]\nname = \"w\"\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = { version = \"=25.12.19\", default-features = false, features = [\"std\"] }\n\n[build-dependencies]\ncc = \"1\"\n",
            root,
            "w/Cargo.toml",
        )
        .unwrap_err();
        assert!(error.contains("build-dependencies"), "{error}");
    }

    #[test]
    fn classify_member_lints_covers_three_classes_and_fail_closed() {
        assert_eq!(
            classify_member_lints("[lints]\nworkspace = true\n", "a").unwrap(),
            UnsafeLevel::Forbid
        );
        assert_eq!(
            classify_member_lints("[lints.rust]\nunsafe_code = \"deny\"\n", "a").unwrap(),
            UnsafeLevel::Deny
        );
        assert_eq!(
            classify_member_lints("[lints.rust]\nunsafe_code = \"allow\"\n", "a").unwrap(),
            UnsafeLevel::AllowGenerated
        );
        let error = classify_member_lints("[dependencies]\n", "a").unwrap_err();
        assert!(error.contains("没有 [lints] 段"), "{error}");
        let error =
            classify_member_lints("[lints.rust]\nunsafe_code = \"warn\"\n", "a").unwrap_err();
        assert!(error.contains("fail closed"), "{error}");
        let error = classify_member_lints("[lints\n", "a").unwrap_err();
        assert!(error.contains("TOML 解析失败"), "{error}");
    }

    #[test]
    fn expected_classification_matches_registered_sets() {
        let classified: Vec<(String, UnsafeLevel)> = vec![
            ("laneflow-core".to_string(), UnsafeLevel::Forbid),
            ("laneflow-format".to_string(), UnsafeLevel::Deny),
            (
                schema_codegen::ROAD_EDITING.wire_package_name.to_string(),
                UnsafeLevel::AllowGenerated,
            ),
            (
                schema_codegen::RUNTIME_SNAPSHOT
                    .wire_package_name
                    .to_string(),
                UnsafeLevel::AllowGenerated,
            ),
        ];
        require_expected_classification(&classified).unwrap();

        let mut extra_deny = classified.clone();
        extra_deny.push(("evil".to_string(), UnsafeLevel::Deny));
        let error = require_expected_classification(&extra_deny).unwrap_err();
        assert!(error.contains("deny"), "{error}");

        let mut extra_allow = classified.clone();
        extra_allow.push(("evil".to_string(), UnsafeLevel::AllowGenerated));
        let error = require_expected_classification(&extra_allow).unwrap_err();
        assert!(error.contains("allow"), "{error}");

        let missing_wire: Vec<(String, UnsafeLevel)> = classified
            .iter()
            .filter(|(_, level)| *level != UnsafeLevel::AllowGenerated)
            .cloned()
            .collect();
        assert!(require_expected_classification(&missing_wire).is_err());
    }

    #[test]
    fn parse_workspace_members_maps_kinds_and_skips_custom_build_only() {
        let metadata = serde_json::json!({
            "packages": [{
                "name": "demo",
                "manifest_path": "/repo/crates/demo/Cargo.toml",
                "targets": [
                    { "name": "demo", "kind": ["lib"] },
                    { "name": "integration", "kind": ["test"] },
                    { "name": "tool", "kind": ["bin"] },
                    { "name": "bench1", "kind": ["bench"] },
                    { "name": "ex", "kind": ["example"], "required-features": ["native-example"] },
                    { "name": "build-script", "kind": ["custom-build"] }
                ]
            }]
        });
        let members = parse_workspace_members(&metadata).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].name, "demo");
        assert_eq!(
            members[0].manifest_path,
            PathBuf::from("/repo/crates/demo/Cargo.toml")
        );
        assert_eq!(
            members[0].targets,
            vec![
                TargetInvocation {
                    flag: "--lib",
                    name: None,
                    label: "lib".to_string(),
                    required_features: Vec::new()
                },
                TargetInvocation {
                    flag: "--test",
                    name: Some("integration".to_string()),
                    label: "test `integration`".to_string(),
                    required_features: Vec::new()
                },
                TargetInvocation {
                    flag: "--bin",
                    name: Some("tool".to_string()),
                    label: "bin `tool`".to_string(),
                    required_features: Vec::new()
                },
                TargetInvocation {
                    flag: "--bench",
                    name: Some("bench1".to_string()),
                    label: "bench `bench1`".to_string(),
                    required_features: Vec::new()
                },
                TargetInvocation {
                    flag: "--example",
                    name: Some("ex".to_string()),
                    label: "example `ex`".to_string(),
                    required_features: vec!["native-example".to_string()]
                },
            ]
        );
    }

    #[test]
    fn parse_workspace_members_fails_closed_on_unknown_kind_and_empty() {
        let unknown = serde_json::json!({
            "packages": [{
                "name": "demo",
                "manifest_path": "/repo/crates/demo/Cargo.toml",
                "targets": [{ "name": "mystery", "kind": ["cdylib"] }]
            }]
        });
        let error = parse_workspace_members(&unknown).unwrap_err();
        assert!(error.contains("未知"), "{error}");

        let empty = serde_json::json!({ "packages": [] });
        let error = parse_workspace_members(&empty).unwrap_err();
        assert!(error.contains("不含任何 package"), "{error}");

        let proc_macro = serde_json::json!({
            "packages": [{
                "name": "demo",
                "manifest_path": "/repo/crates/demo/Cargo.toml",
                "targets": [{ "name": "demo", "kind": ["proc-macro"] }]
            }]
        });
        let members = parse_workspace_members(&proc_macro).unwrap();
        assert_eq!(members[0].targets[0].flag, "--lib");
    }

    #[test]
    fn canary_source_contains_unsafe_usage() {
        assert!(CANARY_LIB_RS.contains("unsafe"));
    }

    #[test]
    fn wire_lib_rs_pins_match_checked_in_files() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask 必须有父目录（仓库根）");
        check_wire_lib_rs_pins(repository_root).unwrap();
    }

    #[test]
    fn generated_rs_pins_match_checked_in_files() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask 必须有父目录（仓库根）");
        check_generated_rs_pins(repository_root).unwrap();
    }
}
