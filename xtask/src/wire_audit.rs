//! Wire / toolchain 审计边界（#376）。
//!
//! 不变量：workspace 内除钉版 flatc 生成物与唯一登记的手写 mmap 例外 crate
//! 外没有 unsafe 代码，且该边界不能被配置 / 环境注入削弱。机制：
//!
//! 1. Cargo.lock resolved 钉版唯一性（有界文本检查）：flatbuffers（wire 生成物
//!    运行时）与 memmap2/tempfile（mmap 例外 crate 的全部依赖）逐一断言
//!    version/source/checksum，`[patch.crates-io]` 换名替换或 registry 漂移
//!    均被拒绝。
//! 2. 两个 wire crate 只承载钉版生成物，全部在 required check（本审计）内闭合：
//!    manifest 卫生（真 TOML 解析两个已知 manifest——[package] 必须存在、不得
//!    声明 build 脚本键、auto* 四键必须显式关闭以禁绝 Cargo 自动 target 发现、
//!    [lib] 固定指向 src/lib.rs 包装器、不得声明 bin/test/bench/example
//!    target 段与 [target] 条件依赖段、[dependencies] 恰好一条
//!    `flatbuffers = { version = "=<钉版>", default-features = false,
//!    features = ["std"] }` 且无 dev/build 依赖段、package 根目录不得存在
//!    build.rs 与 src/bin/、tests/、benches/、examples/ 目录）；包装器 lib.rs
//!    与 `xtask/src/wire_pins/` 下钉版副本字节相等；
//!    生成 .rs 的 sha256 与钉版常量一致（字节正确性在本审计闭合；
//!    schema+flatc → bytes 的语义对应另由 schema-codegen.yml 的
//!    clean-regeneration 证明）。这些钉版叠加后，wire crate 内不存在任何
//!    可写入手写 unsafe 的载体。
//! 3. workspace 成员 lint 分类断言（真 TOML 解析每个成员 manifest）：继承
//!    workspace `unsafe_code = "forbid"` 的成员构成 forbid 集；
//!    `[lints.rust] unsafe_code = "allow"` 只允许两个 wire crate 与唯一手写
//!    例外 laneflow-format-mmap。allow 集逐一与登记名单比对，新增例外或
//!    改类一律 fail closed；`deny` 不再是可登记形态（中间档既允许文件级
//!    allow 覆盖、又制造模糊地带，已随 mmap 例外独立成 crate 删除）。
//! 4. forbid 成员的每个 target（lib/bin/test/bench/example）以 hermetic
//!    编译验证：剔除注入向量的环境（RUSTFLAGS / CARGO_ENCODED_RUSTFLAGS /
//!    CARGO_BUILD_RUSTFLAGS / CARGO_TARGET_*_RUSTFLAGS / RUSTC / *_WRAPPER /
//!    CARGO_BUILD_RUSTC* / RUSTC_BOOTSTRAP）、仓库外临时 cwd（仓库内
//!    .cargo/config.toml 因 cargo 配置按 cwd 向上发现而不可达）、默认特性集
//!    与 `--all-features` 全特性集各跑一遍（`cfg(feature)` /
//!    `cfg(not(feature))` 互补分支都必须进入编译单元）、尾参
//!    `-F unsafe_code`。cargo 尾参（`--` 之后）优先级高于 manifest [lints]、
//!    env rustflags 与 .cargo/config.toml rustflags（金丝雀逐次复核，见下），
//!    因此一切文本形态绕过（转义、拆分数组、宏元变量、shell 构造、
//!    include / #[path] / cfg_attr 形态）对本门禁无效：无论源码以何形态
//!    进入编译单元，都在同一 crate 编译中过 lint。
//! 5. 尾参优先级语义不依赖文档假设：正式检查前先跑三个金丝雀 crate
//!    （manifest [lints] 注入 / RUSTFLAGS 环境注入 / cwd .cargo/config.toml
//!    注入），三者都必须被尾参击败（编译失败且 stderr 指向 unsafe）；任一
//!    金丝雀编译成功或以非 unsafe 原因失败，即判定工具链行为改变，fail closed。
//! 6. forbid 集文本边界（与第 4 条叠加，二者职责不重叠）：成员 package 目录下
//!    全部 .rs 经 `schema_codegen::strip_non_code` 剥掉注释与字面量后，
//!    `unsafe` token 必须为 0 且不得出现 `allow(unsafe_code)`。文本与 cfg 无关，
//!    因此 feature 组合、`cfg(not(debug_assertions))`、`cfg(windows)` 等非活动
//!    分支里的 unsafe 无处可藏（编译扫描只覆盖活动分支，本扫描补齐）；宏展开
//!    unsafe 的 workspace 内定义体也必含 token（外部依赖宏归残余信任边界）。
//!    forbid 成员禁止 build 脚本（build.rs 可向 OUT_DIR 生成文本扫描不可见的
//!    Rust 源码；metadata custom-build target 与 package 根 build.rs 文件双
//!    通道断言），每个 target 的 src_path 必须是 .rs 扩展名（`[lib] path =
//!    "src/lib.txt"` 形态的非 .rs 源逃逸本扫描，fail closed），且源码加载
//!    指令同受限：`include!` 全面禁止，path 属性（含 cfg_attr 包裹形态）
//!    目标必须是以 .rs 结尾的包内相对路径——编译器可达源码全集即本扫描的
//!    .rs 全集。
//! 7. laneflow-format-mmap 的例外边界在本审计内闭合：manifest 卫生（与 wire
//!    同构——auto* 四键关闭、[target] 段与自动发现目录禁绝，[dependencies]
//!    恰好 memmap2/tempfile 两条钉版，resolved source/checksum 由第 1 条闭合）；
//!    `schema_codegen::check_audited_mmap_sources` 断言例外 lib.rs 恰好一次
//!    模块级 allow、一次固定只读映射调用、一处 unsafe token（strip 后计数），
//!    crate 内其他源文件零 unsafe、零 allow(unsafe_code)；crate 内全面禁止
//!    `#[path]` 模块属性、`include!` 宏、`cfg_attr`（cfg_attr 可包裹 path
//!    属性逃逸直接形态检测）与 `macro_rules!` 宏定义（元变量间接可把
//!    include! 藏出文本扫描）——禁绝后 .rs 全集即编译器可达源码全集。
//! 8. 仓库 cargo config 卫生：`.cargo/config.toml`（及旧式 `.cargo/config`）
//!    若存在，禁止一切可替换 CI 门禁执行语义的键——`[env]` 段（经
//!    `cargo run` 进程环境投毒 hermetic 嵌套 cargo 的 HOME/CARGO_HOME
//!    配置发现，装载仓库控制的 rustc-wrapper 剥离尾参 `-F`）、任意层级
//!    `runner` 键（替换 `cargo run`/`cargo test` 的可执行文件启动；立即
//!    退出 0 的 runner 让全部测试与审计空转通过）、`[build]` 的
//!    rustc/rustc-wrapper/rustc-workspace-wrapper（替换或包装编译器，
//!    可剥离 lint 参数或给审计器构建换源）。审计器自身引导同样免疫：
//!    ci.yml 的 wire audit 步骤从仓库外 cwd 以 --manifest-path 构建
//!    xtask（cargo 配置按 cwd 向上发现，仓库 config 不参与）并直接执行
//!    二进制（不经 runner）。
//!
//! 残余信任边界（本模块不尝试自审）：本检查步骤的定义（.github/workflows/）、
//! xtask 源码（含 wire_pins 钉版副本）与依赖政策配置（deny.toml）可被 PR
//! 修改，现由普通 PR review 守住；是否以 CODEOWNERS + code owner review
//! 强制，按 ADR 0027 另开治理 Issue 评估（#579）。mmap 例外 crate 的
//! backing 私有性防不同 UID 的外部进程与意外误用；同 UID 的恶意代码
//! （无论同进程或跨进程——枚举 /proc/self/fd 或 /proc/<pid>/fd 重开
//! backing、/proc/<pid>/mem 或 ptrace 读写地址空间）不在防御范围：
//! Unix 安全模型以 UID 为边界，同 UID 即同安全域，且此类内省对一切
//! Rust 抽象（含 owned memory）同样成立，Rust 生态一致视其为模型外
//! （详见 laneflow-format-mmap 模块文档的威胁模型节）。

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::schema_codegen;

/// resolved 依赖钉版四元组：package/version/checksum 钉死，source 必须来自
/// crates.io registry。`[patch.crates-io]` 换名替换或 registry 漂移都会被拒绝。
struct LockfilePin {
    package: &'static str,
    version: &'static str,
    checksum: &'static str,
}

const LOCKFILE_REGISTRY_SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// resolved 钉版清单：flatbuffers 是 wire 生成物运行时；memmap2/tempfile 是
/// mmap 例外 crate 的全部依赖（manifest 拼写钉版之外，resolved source/checksum
/// 也在此闭合）。升级任一钉版必须在同一 PR 更新本表，diff 随评审可见。
const LOCKFILE_PINS: [LockfilePin; 3] = [
    LockfilePin {
        package: "flatbuffers",
        version: schema_codegen::FLATBUFFERS_VERSION,
        checksum: "35f6839d7b3b98adde531effaf34f0c2badc6f4735d26fe74709d8e513a96ef3",
    },
    LockfilePin {
        package: "memmap2",
        version: "0.9.11",
        checksum: "d1219ed1b7f229ee7104d281dd01d6802fe28bb6e95d292942c4daacdeb798c0",
    },
    LockfilePin {
        package: "tempfile",
        version: "3.27.0",
        checksum: "32497e9a4c7b38532efcdebeef879707aa9f794296a4f0244f6f69e9bc8574bd",
    },
];

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

/// 分类断言：workspace 内唯一登记的手写 unsafe 例外 crate（平台私有临时文件
/// staging + 只读映射）。它与两个 wire crate 构成 `allow` 登记名单；新增
/// allow crate 必须在 `require_expected_classification` 登记，登记变更随 PR 评审。
const AUDITED_MMAP_PACKAGE_NAME: &str = "laneflow-format-mmap";

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
    check_repo_cargo_config_hygiene(&repository_root)?;
    check_lockfile_pins(&repository_root)?;
    check_wire_manifest_hygiene(&repository_root)?;
    check_mmap_manifest_hygiene(&repository_root)?;
    check_wire_lib_rs_pins(&repository_root)?;
    check_generated_rs_pins(&repository_root)?;
    schema_codegen::check_audited_mmap_sources(&repository_root)?;
    check_workspace_unsafe_boundary(&repository_root)?;
    println!(
        "wire 工具链审计已通过：仓库 cargo config 卫生闭合（禁 `[env]` 段、`runner` 键与 `[build]` 编译器替换/包装键，env 继承链与执行语义替换无从投毒门禁），flatbuffers/memmap2/tempfile resolved 钉版闭合（version+source+checksum），wire crate 与 mmap 例外 crate manifest 卫生闭合（auto* 自动 target 发现关闭、[target] 段与自动发现目录禁绝、依赖表钉版），wire 包装器/生成物钉版闭合，mmap 例外源码复核闭合（含 #[path]/include!/cfg_attr/macro_rules 加载与间接禁令、path 属性 walker 兜底），workspace unsafe 分类断言闭合（forbid/allow 两级），forbid 成员禁 build 脚本且 target 源一律 .rs，全 .rs 文本扫描零 unsafe token（strip 注释与字面量，覆盖全部 cfg 分支）且源码加载指令收口（include! 禁绝、path 目标限包内相对 .rs），forbid 成员全部 target（默认+全特性双配置，含 example）通过 hermetic `-F` 编译（含三路注入金丝雀复核）"
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

/// 仓库 cargo config 卫生：`.cargo/config.toml`（及旧式 `.cargo/config`）若
/// 存在，禁止一切可替换 CI 门禁执行语义的键：
///
/// - `[env]` 段：cargo 把它注入它启动的进程（含 `cargo run` 启动的 xtask
///   自身）；hermetic 嵌套 cargo 继承被投毒的 HOME / CARGO_HOME 后会发现
///   仓库控制的 `$HOME/.cargo/config.toml`，进而装载 rustc-wrapper 剥离
///   尾参 `-F`。嵌套命令的仓库外 cwd 只挡住 cwd 向上的配置发现，挡不住
///   这条 env 继承链——故在源头禁绝。
/// - 任意层级的 `runner` 键（`[target.<triple>]` / `[target.'cfg()']`）：
///   runner 替换 `cargo run`/`cargo test` 的可执行文件启动方式；一个立即
///   退出 0 的 runner 脚本可让全部 workspace 测试与审计二进制空转通过，
///   required check 保持绿色。
/// - `[build]` 的 `rustc` / `rustc-wrapper` / `rustc-workspace-wrapper`：
///   替换或包装编译器，可剥离 lint 参数或给 `cargo build -p xtask` 的
///   审计器二进制换源。
///
/// 审计器自身引导（构建并运行 xtask 的 workflow 步骤）同样免疫上述配置：
/// 见 .github/workflows/ci.yml 的 Check wire toolchain audit 步骤（仓库外
/// cwd 构建 + 直接执行二进制）。存在但不可读、TOML 解析失败均 fail closed。
fn check_repo_cargo_config_hygiene(repository_root: &Path) -> Result<(), String> {
    for name in ["config.toml", "config"] {
        let path = repository_root.join(".cargo").join(name);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("无法读取 `{}`: {error}", path.display())),
        };
        let config: toml::Table = text.parse().map_err(|error| {
            format!(
                "仓库 cargo config `{}` TOML 解析失败（fail closed）: {error}",
                path.display()
            )
        })?;
        if config.contains_key("env") {
            return Err(format!(
                "仓库 cargo config `{}` 禁止 `[env]` 段：其可经 `cargo run` 进程环境投毒 hermetic 嵌套 cargo 的配置发现（HOME/CARGO_HOME），装载仓库控制的 rustc-wrapper 剥离尾参 `-F`",
                path.display()
            ));
        }
        if let Some(runner_path) = find_runner_key(&config, ".cargo") {
            return Err(format!(
                "仓库 cargo config `{}` 禁止 `{runner_path}`：runner 可让 `cargo run`/`cargo test` 空转退出 0，全部测试与审计在 required check 绿色下被跳过",
                path.display()
            ));
        }
        if let Some(build) = config.get("build").and_then(toml::Value::as_table) {
            for key in ["rustc", "rustc-wrapper", "rustc-workspace-wrapper"] {
                if build.contains_key(key) {
                    return Err(format!(
                        "仓库 cargo config `{}` 禁止 `[build] {key}`：替换或包装编译器可剥离 lint 参数或给审计器构建换源",
                        path.display()
                    ));
                }
            }
        }
    }
    Ok(())
}

/// 递归查找 TOML 表中任意层级的 `runner` 键，返回点分路径供报错。
/// cargo config 中 `runner` 只在 `[target.*]` 下有意义，但递归全表扫描
/// 简单且 fail closed——其他段不存在合法的 `runner` 键。
fn find_runner_key(table: &toml::Table, prefix: &str) -> Option<String> {
    for (key, value) in table {
        let path = format!("{prefix}.{key}");
        if key == "runner" {
            return Some(path);
        }
        if let Some(nested) = value.as_table()
            && let Some(found) = find_runner_key(nested, &path)
        {
            return Some(found);
        }
    }
    None
}

fn check_lockfile_pins(repository_root: &Path) -> Result<(), String> {
    let lock_text = fs::read_to_string(repository_root.join("Cargo.lock"))
        .map_err(|error| format!("无法读取 workspace Cargo.lock: {error}"))?;
    for pin in &LOCKFILE_PINS {
        require_lockfile_pin(&lock_text, pin)?;
    }
    Ok(())
}

/// 断言 workspace Cargo.lock 中被钉版依赖恰好有一条 resolved 记录，且
/// version/source/checksum 与钉版常量完全一致——`[patch.crates-io]` 换名替换
/// 或 registry 漂移都在这里被拒绝。`research/` 下的独立 lock 不属于
/// 本 workspace，不在此扫描。
fn require_lockfile_pin(lock_text: &str, pin: &LockfilePin) -> Result<(), String> {
    let mut found = 0usize;
    for block in lock_text.split("[[package]]").skip(1) {
        if lock_string_value(block, "name") != Some(pin.package) {
            continue;
        }
        found += 1;
        let version = lock_string_value(block, "version")
            .ok_or_else(|| format!("Cargo.lock 的 {} package 缺少 version", pin.package))?;
        if version != pin.version {
            return Err(format!(
                "Cargo.lock {} resolved version 不匹配：预期 `{}`，实际 `{version}`",
                pin.package, pin.version
            ));
        }
        let source = lock_string_value(block, "source").ok_or_else(|| {
            format!(
                "Cargo.lock 的 {} package 缺少 source（必须来自 crates.io registry）",
                pin.package
            )
        })?;
        if source != LOCKFILE_REGISTRY_SOURCE {
            return Err(format!(
                "Cargo.lock {} source 不匹配：预期 `{LOCKFILE_REGISTRY_SOURCE}`，实际 `{source}`",
                pin.package
            ));
        }
        let checksum = lock_string_value(block, "checksum")
            .ok_or_else(|| format!("Cargo.lock 的 {} package 缺少 checksum", pin.package))?;
        if checksum != pin.checksum {
            return Err(format!(
                "Cargo.lock {} checksum 不匹配：预期 `{}`，实际 `{checksum}`；升级钉版时必须同步更新 xtask 审计常量",
                pin.package, pin.checksum
            ));
        }
    }
    match found {
        0 => Err(format!("Cargo.lock 缺少 {} package", pin.package)),
        1 => Ok(()),
        _ => Err(format!(
            "Cargo.lock 含有 {found} 个 {} package，resolved 钉版审计要求唯一",
            pin.package
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
/// 脚本键，auto* 四键必须显式关闭（Cargo 自动 target 发现不经 manifest 段
/// 即成编译入口）；[lib] 必须固定指向 src/lib.rs 包装器（否则 lib.rs 钉版
/// 失去意义）；不得声明 [target] 段（target 条件依赖表逃逸顶层依赖钉版）；
/// package 根目录不得存在 build.rs 与 src/bin/、tests/、benches/、examples/
/// 目录（纵深：即使 auto* 键被移除，目录缺席也使自动发现无物可发现）。
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
    for key in ["autobins", "autotests", "autoexamples", "autobenches"] {
        if package.get(key).and_then(toml::Value::as_bool) != Some(false) {
            return Err(format!(
                "wire manifest `{label}` 的 [package] 必须显式 `{key} = false`（关闭 Cargo 自动 target 发现）"
            ));
        }
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
    if manifest.contains_key("target") {
        return Err(format!(
            "wire manifest `{label}` 不得声明 [target] 段（target 条件依赖表逃逸顶层依赖钉版）"
        ));
    }
    require_wire_flatbuffers_dep(&manifest, label)?;
    if package_root.join("build.rs").is_file() {
        return Err(format!("wire package `{label}` 不得包含 build.rs"));
    }
    for directory in ["src/bin", "tests", "benches", "examples"] {
        if package_root.join(directory).is_dir() {
            return Err(format!(
                "wire package `{label}` 不得包含 `{directory}/` 目录（自动发现的 target 不经 manifest 段即成编译入口）"
            ));
        }
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

/// mmap 例外 crate 的 manifest 卫生：与 wire crate 同构——[package] 必须存在、
/// 不得声明 build 脚本键、auto* 四键必须显式关闭；[lib] 固定指向 src/lib.rs、
/// 不得声明额外 target 段与 [target] 段；package 根目录不得存在 build.rs 与
/// src/bin/、tests/、benches/、examples/ 目录；[dependencies] 必须恰好两条钉版：
/// `memmap2 = { version = "=0.9.11", default-features = false }`（恰两键）与
/// `tempfile = "=3.27.0"`；无 dev/build 依赖段。resolved source/checksum 钉版
/// 由 `check_lockfile_pins` 闭合。
fn check_mmap_manifest_hygiene(repository_root: &Path) -> Result<(), String> {
    let manifest_path = "crates/laneflow-format-mmap/Cargo.toml";
    let manifest_text = fs::read_to_string(repository_root.join(manifest_path))
        .map_err(|error| format!("无法读取 `{manifest_path}`: {error}"))?;
    let manifest: toml::Table = manifest_text.parse().map_err(|error| {
        format!("mmap 例外 manifest `{manifest_path}` TOML 解析失败，无法静态审计: {error}")
    })?;
    let package = manifest
        .get("package")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| format!("mmap 例外 manifest `{manifest_path}` 缺少 [package] 段"))?;
    if package.contains_key("build") {
        return Err(format!(
            "mmap 例外 manifest `{manifest_path}` 不得声明 build 脚本键"
        ));
    }
    for key in ["autobins", "autotests", "autoexamples", "autobenches"] {
        if package.get(key).and_then(toml::Value::as_bool) != Some(false) {
            return Err(format!(
                "mmap 例外 manifest `{manifest_path}` 的 [package] 必须显式 `{key} = false`（关闭 Cargo 自动 target 发现）"
            ));
        }
    }
    let lib_path = manifest
        .get("lib")
        .and_then(toml::Value::as_table)
        .and_then(|lib| lib.get("path"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| format!("mmap 例外 manifest `{manifest_path}` 缺少 [lib].path"))?;
    if lib_path != "src/lib.rs" {
        return Err(format!(
            "mmap 例外 manifest `{manifest_path}` 的 [lib].path 必须固定为 `src/lib.rs`，实际 `{lib_path}`"
        ));
    }
    for section in ["bin", "test", "bench", "example"] {
        if manifest.get(section).is_some() {
            return Err(format!(
                "mmap 例外 manifest `{manifest_path}` 不得声明 {section} target 段（例外 crate 只许 [lib] 一个编译入口）"
            ));
        }
    }
    if manifest.contains_key("target") {
        return Err(format!(
            "mmap 例外 manifest `{manifest_path}` 不得声明 [target] 段（target 条件依赖表逃逸顶层依赖钉版）"
        ));
    }
    let dependencies = manifest
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| format!("mmap 例外 manifest `{manifest_path}` 缺少 [dependencies] 段"))?;
    if dependencies.len() != 2 {
        return Err(format!(
            "mmap 例外 manifest `{manifest_path}` 的 [dependencies] 必须恰好 memmap2/tempfile 两条，实际 {} 条",
            dependencies.len()
        ));
    }
    let memmap2 = dependencies
        .get("memmap2")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| {
            format!("mmap 例外 manifest `{manifest_path}` 的 memmap2 依赖必须是内联表形态")
        })?;
    if memmap2.len() != 2
        || memmap2.get("version").and_then(toml::Value::as_str) != Some("=0.9.11")
        || memmap2
            .get("default-features")
            .and_then(toml::Value::as_bool)
            != Some(false)
    {
        return Err(format!(
            "mmap 例外 manifest `{manifest_path}` 的 memmap2 依赖必须恰好为 `version = \"=0.9.11\", default-features = false` 两键（禁止 `package` 改名等注入）"
        ));
    }
    if dependencies.get("tempfile").and_then(toml::Value::as_str) != Some("=3.27.0") {
        return Err(format!(
            "mmap 例外 manifest `{manifest_path}` 的 tempfile 依赖必须精确钉为 `\"=3.27.0\"`"
        ));
    }
    for section in ["dev-dependencies", "build-dependencies"] {
        if manifest.contains_key(section) {
            return Err(format!(
                "mmap 例外 manifest `{manifest_path}` 不得声明 [{section}] 段（例外 crate 无 dev/build 依赖面）"
            ));
        }
    }
    if repository_root
        .join("crates/laneflow-format-mmap")
        .join("build.rs")
        .is_file()
    {
        return Err(format!(
            "mmap 例外 package `{manifest_path}` 不得包含 build.rs"
        ));
    }
    for directory in ["src/bin", "tests", "benches", "examples"] {
        if repository_root
            .join("crates/laneflow-format-mmap")
            .join(directory)
            .is_dir()
        {
            return Err(format!(
                "mmap 例外 package `{manifest_path}` 不得包含 `{directory}/` 目录（自动发现的 target 不经 manifest 段即成编译入口）"
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
    /// 自有 `[lints.rust] unsafe_code = "allow"` 的登记例外：两个纯生成物 wire
    /// crate（边界由钉版与 clean-regeneration 闭合）与唯一手写例外
    /// laneflow-format-mmap（边界由 mmap 例外复核闭合）。不参与 hermetic 编译。
    Allow,
}

impl UnsafeLevel {
    fn tail_flag(self) -> Option<&'static str> {
        match self {
            UnsafeLevel::Forbid => Some("-F"),
            UnsafeLevel::Allow => None,
        }
    }
}

/// 真 TOML 解析成员 manifest 的 lint 分类；未分类形态 fail closed。
/// `unsafe_code = "deny"` 不再是可登记形态：中间档既允许文件级 allow 覆盖，
/// 又制造"非 forbid 也非钉版"的模糊地带，已随 mmap 例外独立成 crate 删除。
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
        Some("allow") => Ok(UnsafeLevel::Allow),
        _ => Err(format!(
            "成员 manifest `{label}` 的 [lints] 既非 workspace 继承也非已登记的 unsafe_code allow 形态，fail closed"
        )),
    }
}

/// 断言 allow 集与登记名单（两个 wire crate + 唯一手写 mmap 例外 crate）
/// 完全一致；任何新增例外或改类都使审计失败。
fn require_expected_classification(classified: &[(String, UnsafeLevel)]) -> Result<(), String> {
    let mut allow: Vec<&str> = classified
        .iter()
        .filter(|(_, level)| *level == UnsafeLevel::Allow)
        .map(|(name, _)| name.as_str())
        .collect();
    allow.sort_unstable();
    let mut expected_allow: Vec<&str> = wire_families()
        .map(|family| family.wire_package_name)
        .to_vec();
    expected_allow.push(AUDITED_MMAP_PACKAGE_NAME);
    expected_allow.sort_unstable();
    if allow != expected_allow {
        return Err(format!(
            "workspace unsafe_code = \"allow\" crate 集合 {allow:?} 与登记名单 {expected_allow:?} 不符；`allow` 只许钉版生成物 crate 与已审计 mmap 例外 crate 使用，新增例外必须在 xtask 审计常量登记并随 PR 评审"
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
    /// metadata 中出现 custom-build target 即本 package 含 build 脚本（自动
    /// 发现或 manifest `build` 键均在此显形）。forbid 成员禁止 build 脚本：
    /// build.rs 可向 OUT_DIR 生成文本扫描不可见的 Rust 源码。
    has_build_script: bool,
}

/// 从 workspace cargo metadata（--no-deps）解析全部成员的 name/manifest_path/
/// 编译 target。每个 target 的 src_path 必须是 .rs 扩展名（非 .rs 源逃逸
/// forbid 文本扫描，fail closed）；custom-build 不可单独编译，记录到
/// has_build_script 后跳过；lib/bin/test/bench/example 全部纳入 hermetic
/// 编译；未知 kind fail closed。
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
        let mut has_build_script = false;
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
            let src_path = target
                .get("src_path")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    format!(
                        "workspace metadata package `{name}` 的 target `{target_name}` 缺少 src_path"
                    )
                })?;
            // 非 .rs target 源（如 `[lib] path = "src/lib.txt"`）逃逸 forbid 文本
            // 扫描：cfg 门控 unsafe 可藏在扫描面之外进入其他平台的编译单元。
            if Path::new(src_path).extension() != Some(OsStr::new("rs")) {
                return Err(format!(
                    "workspace package `{name}` 的 target `{target_name}` 源文件 `{src_path}` 不是 .rs 扩展名，fail closed"
                ));
            }
            if kinds.contains(&"custom-build") {
                has_build_script = true;
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
            has_build_script,
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
    check_forbid_textual_boundary(&members, &classified)?;
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

/// forbid 集文本边界：每个 forbid 成员 package 目录下全部 .rs（含 tests、
/// benches、examples——cfg 门控、release profile 与非 Linux 平台分支在文本
/// 层面无差别覆盖），strip 注释与字符串/字符字面量后不得出现 `unsafe`
/// token 或 `allow(unsafe_code)`。与 hermetic `-F` 编译（宏展开维度 + lint
/// 不可覆盖语义）叠加：宏展开 unsafe 由编译抓，cfg 门控 unsafe 由本扫描抓。
///
/// forbid 成员一律禁止 build 脚本：build.rs 可向 OUT_DIR 生成文本扫描不可见
/// 的 Rust 源码，cfg 门控 unsafe 经 `include!` 挂接即逃逸本扫描。metadata
/// custom-build target 与 package 根 build.rs 文件双通道断言（前者覆盖
/// manifest `build = "..."` 改名路径，后者兜底自动发现）。
///
/// 源码加载指令同受限：`include!` 全面禁止（workspace 内零合法用法）；
/// path 属性（含 `cfg_attr(...)` 包裹形态）的目标必须是以 `.rs` 结尾的包内
/// 相对路径（无 `..`、无盘符/绝对前缀、无反斜杠）——保证编译器可达源码
/// 全集就是本扫描的 .rs 全集。检测经词法感知 walker
/// （`schema_codegen::check_path_attribute_values`），注释与字面量里的
/// 字样不触发误判。
fn check_forbid_textual_boundary(
    members: &[MemberPackage],
    classified: &[(String, UnsafeLevel)],
) -> Result<(), String> {
    for (member, (_, level)) in members.iter().zip(classified.iter()) {
        if *level != UnsafeLevel::Forbid {
            continue;
        }
        let package_root = member
            .manifest_path
            .parent()
            .ok_or_else(|| format!("成员 `{}` 的 manifest 路径没有父目录", member.name))?;
        if member.has_build_script {
            return Err(format!(
                "forbid 成员 `{}` 不得含 build 脚本（metadata 出现 custom-build target）：build.rs 可生成文本扫描不可见的 Rust 源码，fail closed",
                member.name
            ));
        }
        if package_root.join("build.rs").is_file() {
            return Err(format!(
                "forbid 成员 `{}` 的 package 根目录不得存在 build.rs：build 脚本可生成文本扫描不可见的 Rust 源码",
                member.name
            ));
        }
        let mut sources = Vec::new();
        schema_codegen::collect_extension_files(package_root, OsStr::new("rs"), &mut sources)?;
        for source in sources {
            let text = fs::read_to_string(&source)
                .map_err(|error| format!("无法读取 `{}`: {error}", source.display()))?;
            let code = schema_codegen::strip_non_code(&text);
            if schema_codegen::count_unsafe_tokens(&code) != 0
                || code.contains("allow(unsafe_code)")
            {
                return Err(format!(
                    "forbid 成员 `{}` 的源文件含 unsafe token 或 allow(unsafe_code)（strip 注释与字面量后判定）：`{}`",
                    member.name,
                    source.display()
                ));
            }
            if schema_codegen::contains_include_macro(&code) {
                return Err(format!(
                    "forbid 成员 `{}` 的源文件含 `include!` 宏（可加载文本扫描面之外的源码；workspace 内零合法用法，一律禁止）：`{}`",
                    member.name,
                    source.display()
                ));
            }
            schema_codegen::check_path_attribute_values(
                &text,
                &format!("forbid 成员 `{}`", member.name),
                schema_codegen::PathAttributePolicy::PackageRelativeRs,
            )?;
        }
    }
    Ok(())
}

/// 构造剔除注入向量后的 cargo 命令。cwd 固定为审计临时目录（仓库外），
/// 使仓库内 .cargo/config.toml 不参与配置发现；`[env]` 经 `cargo run`
/// 进程环境继承的 HOME/CARGO_HOME 投毒链路由
/// [`check_repo_cargo_config_hygiene`] 在源头禁绝。
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
    tail_truncate(stderr.trim())
}

/// 超长 stderr 截尾。from_utf8_lossy 保证有效 UTF-8，但字节切口可能落在
/// 多字节字符中间：向前推进到下一个 char 边界再切，避免 slice panic。
fn tail_truncate(trimmed: &str) -> String {
    const TAIL_LIMIT: usize = 4000;
    if trimmed.len() > TAIL_LIMIT {
        let mut start = trimmed.len() - TAIL_LIMIT;
        while !trimmed.is_char_boundary(start) {
            start += 1;
        }
        format!("…（截断）\n{}", &trimmed[start..])
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_config_hygiene_forbids_env_section() {
        let root = std::env::temp_dir().join(format!(
            "laneflow-wire-audit-cargo-config-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        // 无 .cargo 目录：放行。
        check_repo_cargo_config_hygiene(&root).unwrap();
        let cargo_dir = root.join(".cargo");
        fs::create_dir_all(&cargo_dir).unwrap();
        // 普通段（target-dir 等）：放行。
        fs::write(
            cargo_dir.join("config.toml"),
            "[build]\ntarget-dir = \"target\"\n",
        )
        .unwrap();
        check_repo_cargo_config_hygiene(&root).unwrap();
        // [env] 段：拒绝（新旧文件名同样拒绝）。
        fs::write(
            cargo_dir.join("config.toml"),
            "[env]\nHOME = { value = \"..\", relative = true, force = true }\n",
        )
        .unwrap();
        let error = check_repo_cargo_config_hygiene(&root).unwrap_err();
        assert!(error.contains("[env]"), "{error}");
        fs::remove_file(cargo_dir.join("config.toml")).unwrap();
        fs::write(cargo_dir.join("config"), "[env]\nCARGO_HOME = \"x\"\n").unwrap();
        let error = check_repo_cargo_config_hygiene(&root).unwrap_err();
        assert!(error.contains("[env]"), "{error}");
        fs::remove_file(cargo_dir.join("config")).unwrap();
        // runner 键（替换 cargo run/test 的可执行文件启动，空转退出 0 即
        // 假绿）：直接 target 三元组与 cfg 表达式形态都拒绝。
        fs::write(
            cargo_dir.join("config.toml"),
            "[target.x86_64-unknown-linux-gnu]\nrunner = \"./noop.sh\"\n",
        )
        .unwrap();
        let error = check_repo_cargo_config_hygiene(&root).unwrap_err();
        assert!(error.contains("runner"), "{error}");
        fs::write(
            cargo_dir.join("config.toml"),
            "[target.'cfg(windows)']\nrunner = \"noop\"\n",
        )
        .unwrap();
        let error = check_repo_cargo_config_hygiene(&root).unwrap_err();
        assert!(error.contains("cfg(windows)"), "{error}");
        // [build] 编译器替换/包装键：拒绝。
        fs::write(
            cargo_dir.join("config.toml"),
            "[build]\nrustc-wrapper = \"./wrap.sh\"\n",
        )
        .unwrap();
        let error = check_repo_cargo_config_hygiene(&root).unwrap_err();
        assert!(error.contains("rustc-wrapper"), "{error}");
        fs::remove_file(cargo_dir.join("config.toml")).unwrap();
        // TOML 解析失败 fail closed。
        fs::write(cargo_dir.join("config.toml"), "[env\n").unwrap();
        assert!(check_repo_cargo_config_hygiene(&root).is_err());
        let _ = fs::remove_dir_all(&root);
    }

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

    const FLATBUFFERS_PIN: &LockfilePin = &LOCKFILE_PINS[0];

    fn flatbuffers_lock_block() -> String {
        format!(
            "[[package]]\nname = \"flatbuffers\"\nversion = \"{}\"\nsource = \"{LOCKFILE_REGISTRY_SOURCE}\"\nchecksum = \"{}\"\n",
            schema_codegen::FLATBUFFERS_VERSION,
            FLATBUFFERS_PIN.checksum
        )
    }

    #[test]
    fn lockfile_pin_accepts_exact_match() {
        require_lockfile_pin(&flatbuffers_lock_block(), FLATBUFFERS_PIN).unwrap();
    }

    #[test]
    fn lockfile_pin_rejects_checksum_drift() {
        let lock = format!(
            "[[package]]\nname = \"flatbuffers\"\nversion = \"{}\"\nsource = \"{LOCKFILE_REGISTRY_SOURCE}\"\nchecksum = \"{0000}\"\n",
            schema_codegen::FLATBUFFERS_VERSION
        );
        let error = require_lockfile_pin(&lock, FLATBUFFERS_PIN).unwrap_err();
        assert!(error.contains("checksum"), "{error}");
    }

    #[test]
    fn lockfile_pin_rejects_missing_and_duplicate() {
        let error =
            require_lockfile_pin("[[package]]\nname = \"serde\"\n", FLATBUFFERS_PIN).unwrap_err();
        assert!(error.contains("缺少 flatbuffers"), "{error}");
        let doubled = format!("{}{}", flatbuffers_lock_block(), flatbuffers_lock_block());
        let error = require_lockfile_pin(&doubled, FLATBUFFERS_PIN).unwrap_err();
        assert!(error.contains("唯一"), "{error}");
    }

    #[test]
    fn lockfile_pins_match_checked_in_lockfile() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask 必须有父目录（仓库根）");
        check_lockfile_pins(repository_root).unwrap();
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
            "[package]\nname = \"w\"\nversion = \"0.0.0\"\nedition = \"2021\"\nautobins = false\nautotests = false\nautoexamples = false\nautobenches = false\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = { version = \"=25.12.19\", default-features = false, features = [\"std\"] }\n",
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
            "[package]\nname = \"w\"\nautobins = false\nautotests = false\nautoexamples = false\nautobenches = false\n\n[lib]\npath = \"src/generated.rs\"\n\n[dependencies]\nflatbuffers = { version = \"=25.12.19\", default-features = false, features = [\"std\"] }\n",
            &root,
            "w/Cargo.toml",
        )
        .unwrap_err();
        assert!(error.contains("src/lib.rs"), "{error}");
        fs::write(root.join("build.rs"), "fn main() {}\n").unwrap();
        let error = require_wire_manifest_hygiene(
            "[package]\nname = \"w\"\nautobins = false\nautotests = false\nautoexamples = false\nautobenches = false\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = { version = \"=25.12.19\", default-features = false, features = [\"std\"] }\n",
            &root,
            "w/Cargo.toml",
        )
        .unwrap_err();
        assert!(error.contains("build.rs"), "{error}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn manifest_hygiene_rejects_auto_discovery_target_table_and_directories() {
        let root = std::env::temp_dir().join(format!(
            "laneflow-wire-audit-test-hygiene-auto-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        // auto* 键缺失（默认开启自动发现）即拒绝。
        let error = require_wire_manifest_hygiene(
            "[package]\nname = \"w\"\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = { version = \"=25.12.19\", default-features = false, features = [\"std\"] }\n",
            &root,
            "w/Cargo.toml",
        )
        .unwrap_err();
        assert!(error.contains("autobins"), "{error}");
        // 任一键为 true 即拒绝。
        for enabled in ["autobins", "autotests", "autoexamples", "autobenches"] {
            let mut keys = String::new();
            for key in ["autobins", "autotests", "autoexamples", "autobenches"] {
                keys.push_str(&format!("{key} = {}\n", key == enabled));
            }
            let manifest = format!(
                "[package]\nname = \"w\"\n{keys}\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = {{ version = \"=25.12.19\", default-features = false, features = [\"std\"] }}\n"
            );
            let error =
                require_wire_manifest_hygiene(&manifest, &root, "w/Cargo.toml").unwrap_err();
            assert!(error.contains(enabled), "{error}");
        }
        // [target] 条件依赖段逃逸顶层依赖表钉版，整键禁绝。
        let error = require_wire_manifest_hygiene(
            "[package]\nname = \"w\"\nautobins = false\nautotests = false\nautoexamples = false\nautobenches = false\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = { version = \"=25.12.19\", default-features = false, features = [\"std\"] }\n\n[target.'cfg(unix)'.dependencies]\nshim = { path = \"shim\" }\n",
            &root,
            "w/Cargo.toml",
        )
        .unwrap_err();
        assert!(error.contains("[target]"), "{error}");
        // 自动发现目录存在即拒绝（即使 auto* 键已关闭，纵深防御）。
        fs::create_dir_all(root.join("src/bin")).unwrap();
        let error = require_wire_manifest_hygiene(
            "[package]\nname = \"w\"\nautobins = false\nautotests = false\nautoexamples = false\nautobenches = false\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = { version = \"=25.12.19\", default-features = false, features = [\"std\"] }\n",
            &root,
            "w/Cargo.toml",
        )
        .unwrap_err();
        assert!(error.contains("src/bin"), "{error}");
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
                "[package]\nname = \"w\"\nautobins = false\nautotests = false\nautoexamples = false\nautobenches = false\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = {{ version = \"=25.12.19\", default-features = false, features = [\"std\"] }}\n\n{section}"
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
            "[package]\nname = \"w\"\nautobins = false\nautotests = false\nautoexamples = false\nautobenches = false\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = { version = \"=25.12.19\", default-features = false, features = [\"std\"], package = \"shim\" }\n",
            root,
            "w/Cargo.toml",
        )
        .unwrap_err();
        assert!(error.contains("三键"), "{error}");
        // 多余依赖条目。
        let error = require_wire_manifest_hygiene(
            "[package]\nname = \"w\"\nautobins = false\nautotests = false\nautoexamples = false\nautobenches = false\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = { version = \"=25.12.19\", default-features = false, features = [\"std\"] }\nserde = \"1\"\n",
            root,
            "w/Cargo.toml",
        )
        .unwrap_err();
        assert!(error.contains("恰好一条"), "{error}");
        // 版本未精确钉版。
        let error = require_wire_manifest_hygiene(
            "[package]\nname = \"w\"\nautobins = false\nautotests = false\nautoexamples = false\nautobenches = false\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = { version = \"25.12.19\", default-features = false, features = [\"std\"] }\n",
            root,
            "w/Cargo.toml",
        )
        .unwrap_err();
        assert!(error.contains("精确钉"), "{error}");
        // dev-dependencies 段不允许存在。
        let error = require_wire_manifest_hygiene(
            "[package]\nname = \"w\"\nautobins = false\nautotests = false\nautoexamples = false\nautobenches = false\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = { version = \"=25.12.19\", default-features = false, features = [\"std\"] }\n\n[dev-dependencies]\ntempfile = \"3\"\n",
            root,
            "w/Cargo.toml",
        )
        .unwrap_err();
        assert!(error.contains("dev-dependencies"), "{error}");
        // build-dependencies 段不允许存在。
        let error = require_wire_manifest_hygiene(
            "[package]\nname = \"w\"\nautobins = false\nautotests = false\nautoexamples = false\nautobenches = false\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nflatbuffers = { version = \"=25.12.19\", default-features = false, features = [\"std\"] }\n\n[build-dependencies]\ncc = \"1\"\n",
            root,
            "w/Cargo.toml",
        )
        .unwrap_err();
        assert!(error.contains("build-dependencies"), "{error}");
    }

    #[test]
    fn mmap_manifest_hygiene_accepts_checked_in_manifest() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask 必须有父目录（仓库根）");
        check_mmap_manifest_hygiene(repository_root).unwrap();
    }

    #[test]
    fn mmap_manifest_hygiene_rejects_dependency_and_target_drift() {
        let root = std::env::temp_dir().join(format!(
            "laneflow-wire-audit-mmap-hygiene-{}",
            std::process::id()
        ));
        let package_root = root.join("crates/laneflow-format-mmap");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&package_root).unwrap();
        let write_manifest = |body: &str| {
            fs::write(package_root.join("Cargo.toml"), body).unwrap();
        };
        // auto* 键缺失（默认开启自动发现）即拒绝。
        write_manifest(
            "[package]\nname = \"m\"\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nmemmap2 = { version = \"=0.9.11\", default-features = false }\ntempfile = \"=3.27.0\"\n",
        );
        let error = check_mmap_manifest_hygiene(&root).unwrap_err();
        assert!(error.contains("autobins"), "{error}");
        // 依赖表多一条。
        write_manifest(
            "[package]\nname = \"m\"\nautobins = false\nautotests = false\nautoexamples = false\nautobenches = false\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nmemmap2 = { version = \"=0.9.11\", default-features = false }\ntempfile = \"=3.27.0\"\nserde = \"1\"\n",
        );
        let error = check_mmap_manifest_hygiene(&root).unwrap_err();
        assert!(error.contains("恰好 memmap2/tempfile 两条"), "{error}");
        // memmap2 未精确钉版。
        write_manifest(
            "[package]\nname = \"m\"\nautobins = false\nautotests = false\nautoexamples = false\nautobenches = false\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nmemmap2 = { version = \"0.9.11\", default-features = false }\ntempfile = \"=3.27.0\"\n",
        );
        let error = check_mmap_manifest_hygiene(&root).unwrap_err();
        assert!(error.contains("memmap2"), "{error}");
        // 额外 target 段。
        write_manifest(
            "[package]\nname = \"m\"\nautobins = false\nautotests = false\nautoexamples = false\nautobenches = false\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nmemmap2 = { version = \"=0.9.11\", default-features = false }\ntempfile = \"=3.27.0\"\n\n[[bin]]\nname = \"x\"\n",
        );
        let error = check_mmap_manifest_hygiene(&root).unwrap_err();
        assert!(error.contains("只许 [lib] 一个编译入口"), "{error}");
        // [target] 条件依赖段逃逸顶层依赖表钉版。
        write_manifest(
            "[package]\nname = \"m\"\nautobins = false\nautotests = false\nautoexamples = false\nautobenches = false\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nmemmap2 = { version = \"=0.9.11\", default-features = false }\ntempfile = \"=3.27.0\"\n\n[target.'cfg(unix)'.dependencies]\nshim = { path = \"shim\" }\n",
        );
        let error = check_mmap_manifest_hygiene(&root).unwrap_err();
        assert!(error.contains("[target]"), "{error}");
        // build.rs 存在。
        write_manifest(
            "[package]\nname = \"m\"\nautobins = false\nautotests = false\nautoexamples = false\nautobenches = false\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nmemmap2 = { version = \"=0.9.11\", default-features = false }\ntempfile = \"=3.27.0\"\n",
        );
        fs::write(package_root.join("build.rs"), "fn main() {}\n").unwrap();
        let error = check_mmap_manifest_hygiene(&root).unwrap_err();
        assert!(error.contains("build.rs"), "{error}");
        fs::remove_file(package_root.join("build.rs")).unwrap();
        // 自动发现目录存在即拒绝（即使 auto* 键已关闭，纵深防御）。
        fs::create_dir_all(package_root.join("examples")).unwrap();
        let error = check_mmap_manifest_hygiene(&root).unwrap_err();
        assert!(error.contains("examples"), "{error}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn forbid_textual_boundary_catches_cfg_gated_unsafe() {
        let root = std::env::temp_dir().join(format!(
            "laneflow-wire-audit-textual-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        // cfg 门控不改变文本：平台分支里的 unsafe 必须被文本扫描抓到。
        fs::write(
            root.join("src/lib.rs"),
            "#[cfg(windows)]\nmod win { unsafe fn platform() {} }\n",
        )
        .unwrap();
        let members = vec![MemberPackage {
            name: "demo".to_string(),
            manifest_path: root.join("Cargo.toml"),
            targets: Vec::new(),
            has_build_script: false,
        }];
        let classified = vec![("demo".to_string(), UnsafeLevel::Forbid)];
        let error = check_forbid_textual_boundary(&members, &classified).unwrap_err();
        assert!(error.contains("unsafe token"), "{error}");
        // 字面量与注释里的 unsafe 字样不误报。
        fs::write(
            root.join("src/lib.rs"),
            "fn f() { assert!(\"unsafe\".len() == 6); } // unsafe naming\n",
        )
        .unwrap();
        check_forbid_textual_boundary(&members, &classified).unwrap();
        // allow 级成员不参与文本扫描（wire/mmap 由各自边界闭合）。
        let classified_allow = vec![("demo".to_string(), UnsafeLevel::Allow)];
        fs::write(root.join("src/lib.rs"), "unsafe fn allowed_here() {}\n").unwrap();
        check_forbid_textual_boundary(&members, &classified_allow).unwrap();
        // build 脚本禁令：metadata custom-build target 与 package 根 build.rs
        // 文件双通道（build.rs 可生成文本扫描不可见的 Rust 源码）。
        let members_with_build = vec![MemberPackage {
            name: "demo".to_string(),
            manifest_path: root.join("Cargo.toml"),
            targets: Vec::new(),
            has_build_script: true,
        }];
        let error = check_forbid_textual_boundary(&members_with_build, &classified).unwrap_err();
        assert!(error.contains("build 脚本"), "{error}");
        fs::write(root.join("build.rs"), "fn main() {}\n").unwrap();
        let error = check_forbid_textual_boundary(&members, &classified).unwrap_err();
        assert!(error.contains("build.rs"), "{error}");
        fs::remove_file(root.join("build.rs")).unwrap();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn forbid_textual_boundary_rejects_include_and_escaping_path() {
        let root = std::env::temp_dir().join(format!(
            "laneflow-wire-audit-source-loading-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        let members = vec![MemberPackage {
            name: "demo".to_string(),
            manifest_path: root.join("Cargo.toml"),
            targets: Vec::new(),
            has_build_script: false,
        }];
        let classified = vec![("demo".to_string(), UnsafeLevel::Forbid)];
        // include! 宏可挂接文本扫描面之外的源码，forbid 成员全面禁止
        // （include_str!/include_bytes! 只加载数据，豁免）。
        fs::write(root.join("src/lib.rs"), "include!(\"payload.rs\");\n").unwrap();
        let error = check_forbid_textual_boundary(&members, &classified).unwrap_err();
        assert!(error.contains("include!"), "{error}");
        fs::write(
            root.join("src/lib.rs"),
            "const DATA: &[u8] = include_bytes!(\"data.bin\");\n",
        )
        .unwrap();
        check_forbid_textual_boundary(&members, &classified).unwrap();
        // path 属性目标越界（.. 逃逸包根）fail closed。
        fs::write(
            root.join("src/lib.rs"),
            "#[path = \"../sibling.rs\"]\nmod sibling;\n",
        )
        .unwrap();
        let error = check_forbid_textual_boundary(&members, &classified).unwrap_err();
        assert!(error.contains("path 属性目标"), "{error}");
        // 包内相对 .rs 挂接是合法用法（tests 指向包内 support 文件），放行。
        fs::write(
            root.join("src/lib.rs"),
            "#[path = \"support/evidence.rs\"]\nmod evidence;\n",
        )
        .unwrap();
        check_forbid_textual_boundary(&members, &classified).unwrap();
        // 注释与字面量里的字样不误报。
        fs::write(
            root.join("src/lib.rs"),
            "// include!(\"payload.rs\")\nconst FIXTURE: &str = \"#[path = \\\"../x.rs\\\"]\";\n",
        )
        .unwrap();
        check_forbid_textual_boundary(&members, &classified).unwrap();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn classify_member_lints_covers_two_classes_and_fail_closed() {
        assert_eq!(
            classify_member_lints("[lints]\nworkspace = true\n", "a").unwrap(),
            UnsafeLevel::Forbid
        );
        assert_eq!(
            classify_member_lints("[lints.rust]\nunsafe_code = \"allow\"\n", "a").unwrap(),
            UnsafeLevel::Allow
        );
        // deny 不再是可登记形态：中间档已随 mmap 例外独立成 crate 删除。
        let error =
            classify_member_lints("[lints.rust]\nunsafe_code = \"deny\"\n", "a").unwrap_err();
        assert!(error.contains("fail closed"), "{error}");
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
            ("laneflow-format".to_string(), UnsafeLevel::Forbid),
            (AUDITED_MMAP_PACKAGE_NAME.to_string(), UnsafeLevel::Allow),
            (
                schema_codegen::ROAD_EDITING.wire_package_name.to_string(),
                UnsafeLevel::Allow,
            ),
            (
                schema_codegen::RUNTIME_SNAPSHOT
                    .wire_package_name
                    .to_string(),
                UnsafeLevel::Allow,
            ),
        ];
        require_expected_classification(&classified).unwrap();

        let mut extra_allow = classified.clone();
        extra_allow.push(("evil".to_string(), UnsafeLevel::Allow));
        let error = require_expected_classification(&extra_allow).unwrap_err();
        assert!(error.contains("allow"), "{error}");

        let missing_allow: Vec<(String, UnsafeLevel)> = classified
            .iter()
            .filter(|(_, level)| *level != UnsafeLevel::Allow)
            .cloned()
            .collect();
        assert!(require_expected_classification(&missing_allow).is_err());
    }

    #[test]
    fn parse_workspace_members_maps_kinds_and_records_custom_build() {
        let metadata = serde_json::json!({
            "packages": [{
                "name": "demo",
                "manifest_path": "/repo/crates/demo/Cargo.toml",
                "targets": [
                    { "name": "demo", "kind": ["lib"], "src_path": "/repo/crates/demo/src/lib.rs" },
                    { "name": "integration", "kind": ["test"], "src_path": "/repo/crates/demo/tests/integration.rs" },
                    { "name": "tool", "kind": ["bin"], "src_path": "/repo/crates/demo/src/bin/tool.rs" },
                    { "name": "bench1", "kind": ["bench"], "src_path": "/repo/crates/demo/benches/bench1.rs" },
                    { "name": "ex", "kind": ["example"], "required-features": ["native-example"], "src_path": "/repo/crates/demo/examples/ex.rs" },
                    { "name": "build-script", "kind": ["custom-build"], "src_path": "/repo/crates/demo/build.rs" }
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
        assert!(members[0].has_build_script);
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
                "targets": [{ "name": "mystery", "kind": ["cdylib"], "src_path": "/repo/crates/demo/src/lib.rs" }]
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
                "targets": [{ "name": "demo", "kind": ["proc-macro"], "src_path": "/repo/crates/demo/src/lib.rs" }]
            }]
        });
        let members = parse_workspace_members(&proc_macro).unwrap();
        assert_eq!(members[0].targets[0].flag, "--lib");
        assert!(!members[0].has_build_script);
    }

    #[test]
    fn parse_workspace_members_rejects_non_rs_target_source() {
        // `[lib] path = "src/lib.txt"` 形态：非 .rs 源逃逸 forbid 文本扫描。
        let non_rs = serde_json::json!({
            "packages": [{
                "name": "demo",
                "manifest_path": "/repo/crates/demo/Cargo.toml",
                "targets": [{ "name": "demo", "kind": ["lib"], "src_path": "/repo/crates/demo/src/lib.txt" }]
            }]
        });
        let error = parse_workspace_members(&non_rs).unwrap_err();
        assert!(error.contains(".rs 扩展名"), "{error}");

        let no_extension = serde_json::json!({
            "packages": [{
                "name": "demo",
                "manifest_path": "/repo/crates/demo/Cargo.toml",
                "targets": [{ "name": "demo", "kind": ["lib"], "src_path": "/repo/crates/demo/src/lib" }]
            }]
        });
        let error = parse_workspace_members(&no_extension).unwrap_err();
        assert!(error.contains(".rs 扩展名"), "{error}");

        let missing_src_path = serde_json::json!({
            "packages": [{
                "name": "demo",
                "manifest_path": "/repo/crates/demo/Cargo.toml",
                "targets": [{ "name": "demo", "kind": ["lib"] }]
            }]
        });
        let error = parse_workspace_members(&missing_src_path).unwrap_err();
        assert!(error.contains("src_path"), "{error}");
    }

    #[test]
    fn canary_source_contains_unsafe_usage() {
        assert!(CANARY_LIB_RS.contains("unsafe"));
    }

    #[test]
    fn tail_truncate_respects_char_boundary() {
        // 切口正落在三字节字符 `界`（字节 101..104）中间：旧实现按字节切片
        // 会 panic，新实现必须推进到 char 边界并把半个字符整体排除。
        let mut text = "x".repeat(101);
        text.push('界');
        text.push_str(&"y".repeat(3998));
        let tail = tail_truncate(&text);
        let prefix = "…（截断）\n";
        assert!(tail.starts_with(prefix), "{tail}");
        assert_eq!(&tail[prefix.len()..], &"y".repeat(3998), "{tail}");

        assert_eq!(tail_truncate("abc"), "abc");
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
