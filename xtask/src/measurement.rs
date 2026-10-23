//! #876 正式测量二进制：构建入口与本机路径扫描。
//!
//! `build-measurement` 以冻结参数构建 harness release 二进制：冻结 cargo 命令、
//! 固定顺序的三条 `--remap-path-prefix`（工作树/Cargo 主目录/rustup 工具链映射为
//! 占位符）、钉住的 release profile 与编译器选择，使封存二进制不含采集机本机
//! 路径、可原样作为公开证据制品发布。同一张映射表同时写出归一化记录
//! `LANEFLOW_MEASUREMENT_RUSTFLAGS`（来源端为占位符），由 harness 编译期
//! `option_env!` 嵌入二进制；wire 审计禁止 harness 携带 build.rs。
//!
//! `check-binary-paths` 是本机路径扫描的独立入口，供构建后检查与发布前复扫
//! 共用。两个入口的输出与回执只含占位符、字节数与 SHA-256，不出现采集机实际
//! 路径，可随证据公开。

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

const TOOLCHAIN_ARG: &str = "+1.99.1";
/// 冻结的 cargo 构建命令（测量 provenance 的 build_parameters 记录本 xtask 入口，
/// 本常量记录脚本内部实际执行的命令）。
const CARGO_COMMAND: &str = "cargo +1.99.1 build -p laneflow-urban-harness --release --locked";
/// 固定 profile 钉值：覆盖采集机可能配置的环境或 config 覆盖。
const PROFILE_PINS: [(&str, &str); 10] = [
    ("CARGO_PROFILE_RELEASE_OPT_LEVEL", "3"),
    ("CARGO_PROFILE_RELEASE_DEBUG", "false"),
    ("CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS", "false"),
    ("CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS", "false"),
    ("CARGO_PROFILE_RELEASE_LTO", "false"),
    ("CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "16"),
    ("CARGO_PROFILE_RELEASE_PANIC", "unwind"),
    ("CARGO_PROFILE_RELEASE_STRIP", "false"),
    ("CARGO_PROFILE_RELEASE_RPATH", "false"),
    ("CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO", "false"),
];
/// 嵌套工具链调用（探测与正式构建）统一使用的环境白名单：只放回父环境真实
/// 存在的系统变量（缺席的不写成空串，空串会污染 GetTempPath/home 回退链），
/// 加上钉住的测量变量；未列出的采集机变量一律不存在于子进程。
/// ProgramFiles/ProgramData 是 find-msvc-tools 发现 MSVC 链接器的回退查找链
/// 必需项（COM 实例注册读 %ProgramData%\Microsoft\VisualStudio\...，vswhere
/// 定位读 ProgramFiles(x86)）；HOME/TMPDIR 是 Unix 侧回退；其余为无害系统
/// 布局常量。
const PASSTHROUGH_ENV: [&str; 20] = [
    "PATH",
    "PATHEXT",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMDATA",
    "PUBLIC",
    "COMSPEC",
    "OS",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "HOME",
    "TMPDIR",
];

#[derive(Clone)]
struct RemapRoot {
    from: PathBuf,
    placeholder: &'static str,
}

fn repo_root() -> Result<PathBuf, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("xtask 没有父目录，无法定位仓库根")?
        .to_path_buf();
    if !root.join("Cargo.lock").is_file() || !root.join("tools/laneflow-urban-harness").is_dir() {
        return Err("xtask 源码不在 LaneFlow 仓库内，无法定位仓库根".into());
    }
    Ok(root)
}

/// git 来源读取的卫生：清掉全部 `GIT_` 前缀变量与 `XDG_CONFIG_HOME`（Windows
/// 环境名大小写不敏感，前缀匹配必须折叠大小写），并显式回钉
/// `GIT_NO_REPLACE_OBJECTS=1`——前缀清除会顺带剥掉用户主动设置的该变量，
/// 而仓库内遗留的 replace ref 必须对来源核对不可见。
fn sanitize_git_env(command: &mut Command) {
    let doomed: Vec<std::ffi::OsString> = env::vars_os()
        .map(|(key, _)| key)
        .filter(|key| {
            let text = key.to_string_lossy();
            text.to_ascii_uppercase().starts_with("GIT_")
                || text.eq_ignore_ascii_case("XDG_CONFIG_HOME")
        })
        .collect();
    for key in doomed {
        command.env_remove(key);
    }
    command.env("GIT_NO_REPLACE_OBJECTS", "1");
}

/// git 来源读取：默认环境 + git 卫生；`--work-tree` 显式压住仓库本地 config 的
/// core.worktree，cwd 之外被配置指向别的 checkout 时读取不跑题。
fn git_output(args: &[&str], cwd: &Path) -> Result<String, String> {
    git_output_stdin(args, cwd, None)
}

/// 带 stdin 的 git 来源读取：批量 hash-object 等以 stdin 喂清单的调用走这里。
fn git_output_stdin(args: &[&str], cwd: &Path, input: Option<&str>) -> Result<String, String> {
    let mut command = Command::new("git");
    command
        .arg("--work-tree")
        .arg(cwd)
        .args(args)
        .current_dir(cwd);
    sanitize_git_env(&mut command);
    let roots: Vec<RemapRoot> = both_spellings(cwd)
        .into_iter()
        .map(|from| RemapRoot {
            from,
            placeholder: "<worktree>",
        })
        .collect();
    finish_output(command, "git", args, &roots, input)
}

fn finish_output(
    mut command: Command,
    program: &str,
    args: &[&str],
    roots: &[RemapRoot],
    input: Option<&str>,
) -> Result<String, String> {
    let output = match input {
        None => command
            .output()
            .map_err(|e| format!("无法执行 {program}: {e}"))?,
        Some(input) => {
            // communicate 模式：stdin 由写线程喂给，主线程 wait_with_output
            // 并发排空 stdout/stderr。同步写会死锁——清单可超百 KB，stdout
            // 管道一满子进程就停止消费 stdin，两侧互等（Windows 匿名管道
            // 缓冲小，本仓库清单 stdin/stdout 均远超其上限）。
            command
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            let mut child = command
                .spawn()
                .map_err(|e| format!("无法执行 {program}: {e}"))?;
            let mut stdin = child.stdin.take().expect("stdin 已接管");
            let input = input.to_owned();
            let writer =
                std::thread::spawn(move || std::io::Write::write_all(&mut stdin, input.as_bytes()));
            let output = child
                .wait_with_output()
                .map_err(|e| format!("无法等待 {program}: {e}"))?;
            let write_result = writer
                .join()
                .map_err(|_| format!("{program} 的 stdin 写线程异常终止"))?;
            // 子进程失败时写线程遇断管属预期，失败由状态码与 stderr 报告；
            // 子进程成功而写入失败才是真异常。
            if output.status.success() {
                write_result.map_err(|e| format!("无法向 {program} 写入 stdin: {e}"))?;
            }
            output
        }
    };
    if !output.status.success() {
        // 失败日志可留存为证据：子进程 stderr 常带绝对路径（git fatal、cargo
        // 配置诊断），与正式构建失败日志同规则先脱敏再入错误串。
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{program} {:?} 失败：{}",
            args,
            redact_roots(&stderr, roots)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// 嵌套工具链环境：白名单 + 锚定后的 CARGO_HOME/RUSTUP_HOME。
/// 探测与正式构建共用同一环境，审计/映射/扫描看到的目录与嵌套进程一致。
struct ToolEnv {
    cargo_home: PathBuf,
    rustup_home: PathBuf,
}

impl ToolEnv {
    fn resolve(repo: &Path) -> Result<Self, String> {
        Ok(Self {
            cargo_home: cargo_home(repo)?,
            rustup_home: rustup_home(repo)?,
        })
    }

    /// 白名单环境：只放回父环境存在的变量；未列出的采集机变量一律不存在。
    fn command(&self, program: &str, cwd: &Path) -> Command {
        let mut command = Command::new(program);
        command.current_dir(cwd);
        command.env_clear();
        for name in PASSTHROUGH_ENV {
            if let Some(value) = env::var_os(name) {
                command.env(name, value);
            }
        }
        command.env("CARGO_HOME", &self.cargo_home);
        command.env("RUSTUP_HOME", &self.rustup_home);
        sanitize_git_env(&mut command);
        command
    }

    fn output(&self, program: &str, args: &[&str], cwd: &Path) -> Result<String, String> {
        let mut command = self.command(program, cwd);
        command.args(args);
        // 探测失败日志同样可留存为证据：三个逻辑根（rustup-home 覆盖其下的
        // sysroot）的词法/规范拼写都进脱敏表。
        let roots: Vec<RemapRoot> = [
            (cwd, "<worktree>"),
            (self.cargo_home.as_path(), "<cargo-home>"),
            (self.rustup_home.as_path(), "<rustup-home>"),
        ]
        .into_iter()
        .flat_map(|(root, placeholder)| {
            both_spellings(root)
                .into_iter()
                .map(move |from| RemapRoot { from, placeholder })
        })
        .collect();
        finish_output(command, program, args, &roots, None)
    }
}

fn toolchain_sysroot(tool_env: &ToolEnv, repo: &Path) -> Result<PathBuf, String> {
    let sysroot = tool_env.output("rustc", &[TOOLCHAIN_ARG, "--print", "sysroot"], repo)?;
    let path = PathBuf::from(&sysroot);
    if !path.is_dir() {
        return Err("rustup 工具链 sysroot 不可解析（<rustup-toolchain> 非目录）".into());
    }
    Ok(path)
}

fn home_dir() -> Result<PathBuf, String> {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| "无法定位用户目录（USERPROFILE/HOME）".to_string())
}

/// 环境变量路径根的统一锚定（规则 3）：相对拼写锚定嵌套构建的 cwd（仓库根），
/// 再词法绝对化。CLI 参数锚的是进程 cwd，与此语义不同，不要混用。
fn env_root(name: &str, repo: &Path) -> Result<Option<PathBuf>, String> {
    match env::var_os(name) {
        None => Ok(None),
        Some(value) if value.is_empty() => Err(format!("{name} 为空串")),
        Some(value) => {
            let path = PathBuf::from(value);
            absolute_lexical(if path.is_absolute() {
                path
            } else {
                repo.join(path)
            })
            .map(Some)
        }
    }
}

fn cargo_home(repo: &Path) -> Result<PathBuf, String> {
    if let Some(path) = env_root("CARGO_HOME", repo)? {
        return Ok(path);
    }
    Ok(home_dir()?.join(".cargo"))
}

fn rustup_home(repo: &Path) -> Result<PathBuf, String> {
    if let Some(path) = env_root("RUSTUP_HOME", repo)? {
        return Ok(path);
    }
    Ok(home_dir()?.join(".rustup"))
}

/// 小写化并统一分隔符后的前缀匹配：Windows 文件系统大小写不敏感，依赖来源
/// 检查不得因盘符或路径大小写差异放行逃逸。按组件边界比较：根本身或其直接
/// 后代才命中，`D:\work\laneflow-modified` 不算在 `D:\work\laneflow` 内。
fn path_starts_with_ci(path: &Path, prefix: &Path) -> bool {
    let normalize = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    let (path, prefix) = (normalize(path), normalize(prefix));
    let prefix = prefix.trim_end_matches('\\');
    path == prefix || path.starts_with(&format!("{prefix}\\"))
}

/// include 条目的 label 形态：条目内容来自采集机配置文件本身，绝对/根相对
/// 路径原样进 label 会把本机路径带进错误串。已知根先归占位符（根内 include
/// 保持可读），仍是机器形态路径则整段占位。
fn sanitize_include_entry(entry: &str, cargo_home: &Path, repo: &Path) -> String {
    let roots: Vec<RemapRoot> = [(cargo_home, "<cargo-home>"), (repo, "<worktree>")]
        .into_iter()
        .flat_map(|(root, placeholder)| {
            both_spellings(root)
                .into_iter()
                .map(move |from| RemapRoot { from, placeholder })
        })
        .collect();
    let redacted = redact_roots(entry, &roots);
    let machine_shaped = redacted.starts_with(['/', '\\'])
        || Path::new(&redacted)
            .components()
            .next()
            .is_some_and(|c| matches!(c, std::path::Component::Prefix(_)));
    if machine_shaped {
        "<include 绝对路径>".to_string()
    } else {
        redacted
    }
}

/// 采集机 Cargo 配置审计：用户级（CARGO_HOME）与仓库各祖先目录的
/// `.cargo/config{,.toml}`（Cargo 从构建 cwd 逐级向上探测），并递归跟随
/// 顶层 `include` 键。出现 profile / paths / source / patch / env 表或
/// build.rustc*/rustflags 键即 fail closed——这些键可绕过命令行钉住
///（package 级 profile 覆盖、本地路径覆盖、wrapper、目录替换、换编译器、
/// 向构建脚本强制注入环境变量）。仓库自身无 `.cargo/config.toml`，仓库内
/// 配置由干净 checkout 要求覆盖。
fn check_cargo_config(cargo_home: &Path, repo: &Path) -> Result<(), String> {
    let mut candidates: Vec<(PathBuf, String)> = ["config.toml", "config"]
        .into_iter()
        .map(|name| (cargo_home.join(name), format!("<cargo-home>/{name}")))
        .collect();
    // 审计覆盖构建 cwd（仓库根）自身：被 .git/info/exclude 或全局 ignore 隐藏的
    // checkout 本地配置不改变 git 干净状态，但会被 Cargo 探测。
    for ancestor in repo.ancestors() {
        for name in [".cargo/config.toml", ".cargo/config"] {
            candidates.push((ancestor.join(name), format!("祖先目录的 {name}")));
        }
    }
    let mut visited = std::collections::HashSet::new();
    let mut queue: std::collections::VecDeque<(PathBuf, String)> = candidates.into();
    while let Some((path, label)) = queue.pop_front() {
        // 去重不折叠大小写：大小写敏感文件系统上仅大小写不同的两个 include
        // 都会被 Cargo 合并，必须各自接受审计。键先词法归一，否则
        // `sub/../config.toml` 类别名让同一文件无限递归。
        let key = absolute_lexical(path.clone())?
            .to_string_lossy()
            .into_owned();
        if !path.is_file() || !visited.insert(key) {
            continue;
        }
        let text =
            fs::read_to_string(&path).map_err(|e| format!("无法读取 Cargo 配置 {label}: {e}"))?;
        let value: toml::Value =
            toml::from_str(&text).map_err(|e| format!("Cargo 配置 {label} 解析失败: {e}"))?;
        for key in ["profile", "paths", "source", "patch", "env", "unstable"] {
            if value.get(key).is_some() {
                return Err(format!(
                    "Cargo 配置 {label} 含冻结构建不允许的 [{key}] 表；请移出测量机配置后重建"
                ));
            }
        }
        if value
            .get("resolver")
            .and_then(toml::Value::as_table)
            .is_some_and(|resolver| resolver.contains_key("lockfile-path"))
        {
            return Err(format!(
                "Cargo 配置 {label} 含冻结构建不允许的 resolver.lockfile-path（替换锁文件）；请移出测量机配置后重建"
            ));
        }
        if let Some(build) = value.get("build").and_then(toml::Value::as_table) {
            for key in [
                "rustc",
                "rustc-wrapper",
                "rustc-workspace-wrapper",
                "rustflags",
                "build-dir",
            ] {
                if build.contains_key(key) {
                    return Err(format!(
                        "Cargo 配置 {label} 含冻结构建不允许的 build.{key}；请移出测量机配置后重建"
                    ));
                }
            }
        }
        // target.* 整表拒绝：直达键（linker/runner/rustflags）与嵌套的
        // target.<triple>.<links> 构建脚本覆盖（rustc-link-lib/cfg/env 注入）
        // 都会改变产物。
        if value.get("target").is_some() {
            return Err(format!(
                "Cargo 配置 {label} 含冻结构建不允许的 [target] 表（含 target.<triple>.<links> 构建脚本覆盖）；请移出测量机配置后重建"
            ));
        }
        // 配置的 include 键递归并入审计（相对路径相对当前配置文件目录解析）。
        if let Some(include_value) = value.get("include") {
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            // Cargo 接受字符串、字符串数组与内联表（{ path = "..." }）三种
            // include 形态，缺 path 键的内联表由 Cargo 自身报错。
            let included_files: Vec<String> = match include_value {
                toml::Value::String(single) => vec![single.clone()],
                toml::Value::Array(list) => list
                    .iter()
                    .filter_map(|item| match item {
                        toml::Value::String(text) => Some(text.clone()),
                        toml::Value::Table(table) => table.get("path")?.as_str().map(str::to_owned),
                        _ => None,
                    })
                    .collect(),
                toml::Value::Table(table) => table
                    .get("path")
                    .and_then(|p| p.as_str().map(str::to_owned))
                    .into_iter()
                    .collect(),
                _ => vec![],
            };
            for entry in included_files {
                // label 保持占位符约定：条目是采集机配置内容，绝对路径须净化。
                let display = sanitize_include_entry(&entry, cargo_home, repo);
                queue.push_back((parent.join(&entry), format!("{label} 包含的 {display}")));
            }
        }
    }
    Ok(())
}

/// 依赖来源根检查：每个依赖包的 manifest 必须落在工作树或 Cargo 主目录内；
/// 目录替换（vendor）等来源会让 remap 覆盖不到，构建必须 fail closed。
fn check_dependency_roots(
    tool_env: &ToolEnv,
    repo: &Path,
    cargo_home: &Path,
) -> Result<(), String> {
    let stdout = tool_env.output(
        "cargo",
        &[
            TOOLCHAIN_ARG,
            "metadata",
            "--locked",
            "--format-version",
            "1",
        ],
        repo,
    )?;
    let metadata: serde_json::Value =
        serde_json::from_str(&stdout).map_err(|e| format!("cargo metadata 解析失败: {e}"))?;
    let packages = metadata["packages"]
        .as_array()
        .ok_or("cargo metadata 缺少 packages")?;
    for package in packages {
        let manifest = package["manifest_path"]
            .as_str()
            .ok_or("cargo metadata 包缺少 manifest_path")?;
        let path = Path::new(manifest);
        if !path_starts_with_ci(path, repo) && !path_starts_with_ci(path, cargo_home) {
            let name = package["name"].as_str().unwrap_or("<unknown>");
            return Err(format!(
                "依赖 {name} 的源码不在冻结根内（工作树 / Cargo 主目录），存在目录替换或未映射来源；请在映射表中显式扩展后再构建"
            ));
        }
    }
    Ok(())
}

/// 逐字节查找全部命中偏移（先按首字节过滤再验证）。
fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<u64> {
    let mut hits = Vec::new();
    if needle.is_empty() || needle.len() > haystack.len() {
        return hits;
    }
    let first = needle[0];
    let mut at = 0usize;
    while let Some(pos) = haystack[at..].iter().position(|&b| b == first) {
        let start = at + pos;
        if haystack.len() - start < needle.len() {
            break;
        }
        if &haystack[start..start + needle.len()] == needle {
            hits.push(start as u64);
        }
        at = start + 1;
    }
    hits
}

/// ASCII 大小写折叠：Windows 路径大小写不敏感，嵌入形态与登记根可能只差大小写。
fn ascii_lower(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(u8::to_ascii_lowercase).collect()
}

/// UTF-16LE 视图的 ASCII 折叠：只折高字节为 0 的码元，非 ASCII 码元原样保留
///（逐字节折叠会把非 ASCII 码元的高字节误判为 ASCII 大写而损坏）。
/// `offset` 覆盖奇偶两种对齐：宽字符串在文件内可从任意字节偏移开始。
fn utf16le_fold(bytes: &[u8], offset: usize) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for pair in out[offset..].as_chunks_mut::<2>().0 {
        if pair[1] == 0 && pair[0].is_ascii_uppercase() {
            pair[0] += 32;
        }
    }
    out
}

/// 每个根展开的搜索针：两种目录分隔符 × UTF-8/UTF-16LE，全部 ASCII 大小写折叠。
fn needles(path: &Path) -> Vec<(bool, &'static str, Vec<u8>)> {
    let text = path
        .to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_owned();
    let mut out = Vec::new();
    for separator in ['\\', '/'] {
        let surface = text.replace(['\\', '/'], &separator.to_string());
        out.push((
            false,
            if separator == '\\' {
                "utf-8 \\"
            } else {
                "utf-8 /"
            },
            ascii_lower(surface.as_bytes()),
        ));
        let wide: Vec<u8> = surface.encode_utf16().flat_map(u16::to_le_bytes).collect();
        out.push((
            true,
            if separator == '\\' {
                "utf-16le \\"
            } else {
                "utf-16le /"
            },
            utf16le_fold(&wide, 0),
        ));
    }
    out
}

struct ScanRoot {
    placeholder: String,
    path: PathBuf,
    /// 派生根在本机必须存在；显式指定的根总是登记扫描（扫描机可能不是采集机）。
    explicit: bool,
}

#[derive(Serialize)]
struct ScanHit {
    root: String,
    encoding_separator: &'static str,
    count: usize,
    first_byte: u64,
}

/// 扫描二进制中的本机路径；返回命中列表。回执与输出只含占位符。
/// UTF-8 针搜逐字节折叠视图；UTF-16LE 针搜码元折叠视图（奇偶两种对齐）。
fn scan_binary(
    binary: &Path,
    roots: &[ScanRoot],
) -> Result<(Vec<ScanHit>, serde_json::Value), String> {
    let bytes = fs::read(binary).map_err(|e| format!("无法读取二进制: {e}"))?;
    let lowered_utf8 = ascii_lower(&bytes);
    let lowered_wide = [utf16le_fold(&bytes, 0), utf16le_fold(&bytes, 1)];
    let mut hits = Vec::new();
    for root in roots {
        if !root.explicit && !root.path.is_dir() {
            return Err(format!(
                "派生扫描根 {} 在本机不存在；构建/采集机的扫描根必须可解析",
                root.placeholder
            ));
        }
        // 词法与规范两种拼写都扫：junction/subst/8.3 短名的拼写分叉不致共模漏扫。
        for spelling in both_spellings(&root.path) {
            for (wide, encoding_separator, needle) in needles(&spelling) {
                let found = if wide {
                    let mut offsets: Vec<u64> = lowered_wide
                        .iter()
                        .flat_map(|haystack| find_all(haystack, &needle))
                        .collect();
                    offsets.sort_unstable();
                    offsets.dedup();
                    offsets
                } else {
                    find_all(&lowered_utf8, &needle)
                };
                if !found.is_empty() {
                    hits.push(ScanHit {
                        root: root.placeholder.clone(),
                        encoding_separator,
                        count: found.len(),
                        first_byte: found[0],
                    });
                }
            }
        }
    }
    let digest = Sha256::digest(&bytes);
    let receipt = json!({
        "version": "urban-binary-local-path-scan-v1",
        "binary": {
            "name": binary.file_name().and_then(|n| n.to_str()).unwrap_or("<unknown>"),
            "sha256": digest.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "bytes": bytes.len(),
        },
        "roots": roots.iter().map(|r| &r.placeholder).collect::<Vec<_>>(),
        "hits": serde_json::to_value(&hits).map_err(|e| e.to_string())?,
        "status": if hits.is_empty() { "pass" } else { "fail" },
    });
    Ok((hits, receipt))
}

fn derived_roots(tool_env: &ToolEnv, repo: &Path) -> Result<Vec<ScanRoot>, String> {
    let sysroot = toolchain_sysroot(tool_env, repo)?;
    Ok(vec![
        ScanRoot {
            placeholder: "<user-profile>".into(),
            path: home_dir()?,
            explicit: false,
        },
        ScanRoot {
            placeholder: "<cargo-home>".into(),
            path: cargo_home(repo)?,
            explicit: false,
        },
        ScanRoot {
            placeholder: "<rustup-home>".into(),
            path: rustup_home(repo)?,
            explicit: false,
        },
        ScanRoot {
            placeholder: "<rustup-toolchain>".into(),
            path: sysroot,
            explicit: false,
        },
        ScanRoot {
            placeholder: "<worktree>".into(),
            path: repo.to_path_buf(),
            explicit: false,
        },
    ])
}

/// 控制台与错误信息里的路径一律以占位符形式出现（输出可留存为证据）。
fn display_placeholder(repo: &Path, path: &Path) -> String {
    path.strip_prefix(repo)
        .map(|rest| format!("<worktree>/{}", rest.to_string_lossy().replace('\\', "/")))
        .unwrap_or_else(|_| "<path>".into())
}

/// 显式指定的扫描根做词法绝对化（不触文件系统，扫描机上根可以不存在），
/// 相对拼写否则会被原样当成 needle 而漏扫实际嵌入的绝对路径。
/// `std::path::absolute` 只前接当前目录、不消除 `.`/`..`，再按组件词法归一。
fn absolute_lexical(path: PathBuf) -> Result<PathBuf, String> {
    let absolute = std::path::absolute(&path).map_err(|e| format!("无法绝对化路径: {e}"))?;
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            // 越过根的词法 `..` 与根本身等价（如 `C:\..` 即 `C:\`）。
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

/// 派生根的词法与规范两种拼写：junction/subst 盘/8.3 短名会让工具链嵌入的
/// 拼写与词法值分叉；两种都进 remap 表与 needle，以覆盖这两种拼写下
/// remap 漏映射且扫描漏扫的共模失效。
fn both_spellings(path: &Path) -> Vec<PathBuf> {
    let mut out = vec![path.to_path_buf()];
    if let Ok(canonical) = fs::canonicalize(path) {
        let text = canonical.to_string_lossy();
        // `\\?\UNC\server\share` 还原为 `\\server\share`；普通 `\\?\` 前缀剥掉。
        let text = text
            .strip_prefix("\\\\?\\UNC\\")
            .map(|rest| format!("\\\\{rest}"))
            .unwrap_or_else(|| text.trim_start_matches("\\\\?\\").to_owned());
        let canonical = PathBuf::from(text);
        if canonical != out[0] {
            out.push(canonical);
        }
    }
    out
}

/// 文本脱敏：把根目录的登记拼写替换为占位符。`\` 与 `/` 两种分隔符拼写都
/// 生成——根的存储拼写本身可能来自任一种（env 值、词法/canonical 不一），
/// 不能只按存储拼写推导变体。
/// 正式构建失败日志与探测辅助的 stderr 共用同一实现，输出才可留存为证据。
/// Windows 路径身份大小写不敏感（工具诊断的驱动器号/组件大小写可与注册根
/// 分叉），替换按 ASCII 折叠大小写——与扫描针的 `ascii_lower` 折叠同级；
/// 非 ASCII 的 Unicode 折叠出界。重叠根按拼写长度降序替换：细粒度占位符
/// 优先（sysroot 先于其父 rustup-home），与二进制 remap 的精确映射一致。
fn redact_roots(text: &str, roots: &[RemapRoot]) -> String {
    let mut roots: Vec<&RemapRoot> = roots.iter().collect();
    roots.sort_by_key(|root| std::cmp::Reverse(root.from.as_os_str().len()));
    let mut out = text.to_owned();
    for root in roots {
        let from = root.from.to_string_lossy();
        let back = from.replace('/', "\\");
        let forward = from.replace('\\', "/");
        for spelling in [from.as_ref(), back.as_str(), forward.as_str()] {
            out = replace_ascii_ci(&out, spelling, root.placeholder);
        }
    }
    out
}

/// ASCII 大小写不敏感替换：命中区间取自原串，非路径文本（含非 ASCII）
/// 原样保留；落在多字节字符内部的字节巧合命中跳过，不替换。
fn replace_ascii_ci(haystack: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return haystack.to_owned();
    }
    // to_ascii_lowercase 逐字节映射 A-Z，其余字节不动：折叠视图与原串
    // 字节长度一致，命中区间可按字节位置切回原串。
    let lowered = haystack.to_ascii_lowercase();
    let needle = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(haystack.len());
    let mut cursor = 0;
    let mut search = 0;
    while let Some(at) = lowered[search..].find(&needle) {
        let start = search + at;
        let end = start + needle.len();
        if haystack.is_char_boundary(start) && haystack.is_char_boundary(end) {
            out.push_str(&haystack[cursor..start]);
            out.push_str(replacement);
            cursor = end;
            search = end;
        } else {
            search = start + 1;
        }
    }
    out.push_str(&haystack[cursor..]);
    out
}

fn write_receipt(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    text.push('\n');
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()))
        .map_err(|e| format!("回执必须写入新路径: {e}"))
}

/// `git ls-files -s -z` 的一条索引记录：`mode SP sha SP stage TAB path`。
struct IndexEntry<'a> {
    mode: &'a str,
    blob: &'a str,
    path: &'a str,
}

fn parse_index_entries(text: &str) -> Result<Vec<IndexEntry<'_>>, String> {
    let mut entries = Vec::new();
    for record in text.split('\0') {
        if record.is_empty() {
            continue;
        }
        let (meta, path) = record
            .split_once('\t')
            .ok_or("git ls-files 输出缺少 TAB 分隔")?;
        let mut meta = meta.split(' ');
        let mode = meta.next().ok_or("git ls-files 输出缺少 mode")?;
        let blob = meta.next().ok_or("git ls-files 输出缺少 blob 摘要")?;
        entries.push(IndexEntry { mode, blob, path });
    }
    Ok(entries)
}

/// 干净判定的字节层：gitattributes 的 text/eol 转换与 filter.clean 都会让
/// status 在转换后的视图上判净（本仓库 `* text=auto eol=lf`，编辑器残留
/// CRLF 即隐身），而 Cargo 编译的是工作树原始字节。`hash-object
/// --no-filters` 绕过全部转换与过滤器，与索引 blob 逐文件比对；gitlink
/// 无文件内容，跳过。
fn check_tree_matches_index(repo: &Path) -> Result<(), String> {
    let ls_files_s = git_output(&["ls-files", "-s", "-z"], repo)?;
    let entries = parse_index_entries(&ls_files_s)?;
    let mut paths = Vec::new();
    let mut blobs = Vec::new();
    for entry in &entries {
        if entry.mode == "160000" {
            continue;
        }
        if entry.path.contains('\n') {
            return Err("文件名含换行，批量哈希不可判定；干净判定 fail closed".into());
        }
        paths.push(entry.path);
        blobs.push(entry.blob);
    }
    let input = paths.join("\n");
    let hashed = git_output_stdin(
        &["hash-object", "--stdin-paths", "--no-filters"],
        repo,
        Some(&input),
    )?;
    let actuals: Vec<&str> = hashed.lines().collect();
    if actuals.len() != paths.len() {
        return Err("git hash-object 输出条数与索引不符；干净判定 fail closed".into());
    }
    let mismatched: Vec<&str> = actuals
        .iter()
        .zip(blobs.iter())
        .zip(paths.iter())
        .filter_map(|((actual, blob), path)| (actual != blob).then_some(*path))
        .collect();
    if !mismatched.is_empty() {
        return Err(format!(
            "工作树原始字节与索引 blob 不一致（{} 个文件，text/eol 或 filter 转换遮蔽）：{}；请以未转换字节重新检出",
            mismatched.len(),
            mismatched.join(", ")
        ));
    }
    Ok(())
}

/// `build-measurement [ --receipt <path> ] [ --evidence-root <dir> ] [ --extra-root <dir> ]...`
pub(crate) fn build_measurement(args: &[String]) -> Result<(), String> {
    let mut receipt_path = None;
    let mut evidence_root = None;
    let mut extra_roots = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--receipt" => {
                receipt_path = Some(absolute_lexical(PathBuf::from(
                    iter.next().ok_or("--receipt 缺少路径")?,
                ))?)
            }
            "--evidence-root" => {
                evidence_root = Some(absolute_lexical(PathBuf::from(
                    iter.next().ok_or("--evidence-root 缺少路径")?,
                ))?)
            }
            "--extra-root" => extra_roots.push(absolute_lexical(PathBuf::from(
                iter.next().ok_or("--extra-root 缺少路径")?,
            ))?),
            other => return Err(format!("未知 build-measurement 参数：{other}")),
        }
    }
    let repo = repo_root()?;
    let tool_env = ToolEnv::resolve(&repo)?;
    let sysroot = toolchain_sysroot(&tool_env, &repo)?;
    let cargo_home = tool_env.cargo_home.clone();
    check_cargo_config(&cargo_home, &repo)?;
    check_dependency_roots(&tool_env, &repo, &cargo_home)?;
    // 正式测量二进制必须来自干净 checkout：构建提交与树写入回执，运行侧
    // provenance 在测量窗口两端复查干净状态。干净判定三层互补：
    // git status 尊重用户/全局 showUntrackedFiles 配置，显式钉住 all，抓
    // 未跟踪/删除/真实编辑；skip-worktree/assume-unchanged 索引位让被改的
    // tracked 文件在 status 里隐身，ls-files -v 的小写标签或 S 即 fail closed；
    // text/eol 与 filter.clean 的转换遮蔽由字节层比对检查。
    let git_status =
        |repo: &Path| git_output(&["status", "--porcelain", "--untracked-files=all"], repo);
    let source_commit = git_output(&["rev-parse", "HEAD"], &repo)?;
    let source_tree = git_output(&["rev-parse", "HEAD^{tree}"], &repo)?;
    if !git_status(&repo)?.is_empty() {
        return Err("正式测量构建要求干净 checkout；请先提交或清理工作树".into());
    }
    let ls_files = git_output(&["ls-files", "-v"], &repo)?;
    if ls_files
        .lines()
        .any(|line| line.starts_with(|c: char| c.is_ascii_lowercase()) || line.starts_with('S'))
    {
        return Err("检出含 skip-worktree/assume-unchanged 条目；干净判定不可信".into());
    }
    check_tree_matches_index(&repo)?;

    // 唯一映射表：实际 flag 与归一化记录同源于此，二者不可能漂移。
    // 每个逻辑根登记词法与规范两种拼写（避免这两种拼写在 remap 与扫描中同时遗漏）；归一化记录仍按三个
    // 逻辑根各一条——它描述映射语义，不是逐 flag 回放配方。
    let logical = [
        (repo.clone(), "<worktree>"),
        (cargo_home.clone(), "<cargo-home>"),
        (sysroot.clone(), "<rustup-toolchain>"),
    ];
    let map: Vec<RemapRoot> = logical
        .iter()
        .flat_map(|(root, placeholder)| {
            both_spellings(root)
                .into_iter()
                .map(move |from| RemapRoot { from, placeholder })
        })
        .collect();
    let actual = map
        .iter()
        .map(|r| format!("--remap-path-prefix={}={}", r.from.display(), r.placeholder))
        .collect::<Vec<_>>()
        .join("\u{1f}");
    let normalized = logical
        .iter()
        .map(|(_, placeholder)| format!("--remap-path-prefix={0}={0}", placeholder))
        .collect::<Vec<_>>()
        .join(" ");

    let final_dir = repo.join("target").join("release-measurement");
    // 隔离构建：二进制在全部证明（嵌入断言/扫描/漂移复核）通过前不得落在
    // 正式测量路径上——否则失败轮留下的半成品会在检出清理干净后被 harness
    // 当作合格正式二进制接受（运行时只核对内嵌提交）。全部通过后 rename
    // 发布，上一轮合格二进制在发布前保持原位。
    let quarantine_dir = repo.join("target").join("release-measurement-quarantine");
    let receipt_path = receipt_path.unwrap_or_else(|| final_dir.join("build-receipt.json"));
    // 回执只新增不覆盖：先失败——既不在完成构建与扫描后才报错，也不让随后的
    // 目录清理把既有回执吞掉。
    if receipt_path.exists() {
        let display = display_placeholder(&repo, &receipt_path);
        return Err(format!(
            "回执路径已存在：{display}；请显式给一个全新的 --receipt 路径"
        ));
    }
    // 每次正式构建使用全新 target 目录：Cargo 只按 build.rs 声明的
    // rerun-if-* 跟踪构建脚本输入，陈旧复用可携带未被声明的本机残留；
    // 全新目录把这类风险整体消除（代价仅是正式构建多花几十秒）。
    if quarantine_dir.exists() {
        fs::remove_dir_all(&quarantine_dir).map_err(|e| {
            format!(
                "无法清空 {}: {e}",
                display_placeholder(&repo, &quarantine_dir)
            )
        })?;
    }
    // 钉住目标三元组：采集机的 build.target 配置 / CARGO_BUILD_TARGET 不得改变
    // 产物位置或目标平台。
    let verbose = tool_env.output("rustc", &[TOOLCHAIN_ARG, "-Vv"], &repo)?;
    let host = verbose
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(str::to_owned)
        .ok_or("rustc -Vv 缺少 host 行")?;
    let mut command = tool_env.command("cargo", &repo);
    command.args([
        TOOLCHAIN_ARG,
        "build",
        "-p",
        "laneflow-urban-harness",
        "--release",
        "--locked",
        "--target",
        &host,
    ]);

    // 白名单环境（ToolEnv::command 已施加）：钉住的测量变量之外，
    // CARGO_ENCODED_RUSTFLAGS 在 Cargo flag 来源中优先级最高，覆盖 config 的
    // build.rustflags；编译器钉到工具链内 rustc；中间产物目录钉进全新清理的
    // 目录内（build.build-dir 或 CARGO_BUILD_BUILD_DIR 不得把可复用缓存挪到
    // 未扫描处）。采集机的 RUSTC/wrapper/CC 家族/代理/其他一切变量一律缺席。
    command
        .env("CARGO_ENCODED_RUSTFLAGS", &actual)
        .env("LANEFLOW_MEASUREMENT_RUSTFLAGS", &normalized)
        .env("LANEFLOW_MEASUREMENT_COMMIT", &source_commit)
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_TARGET_DIR", &quarantine_dir)
        .env("CARGO_BUILD_BUILD_DIR", quarantine_dir.join("build"))
        .env(
            "CARGO_BUILD_RUSTC",
            sysroot
                .join("bin")
                .join(if cfg!(windows) { "rustc.exe" } else { "rustc" }),
        );
    for (name, value) in PROFILE_PINS {
        command.env(name, value);
    }
    let output = command
        .output()
        .map_err(|e| format!("无法启动 cargo: {e}"))?;
    if !output.status.success() {
        // 输出可留存为证据：映射根先替换为占位符，控制台不留本机路径。
        // 链接器/构建脚本诊断还会引用 rustup-home、用户目录与临时目录
        // （白名单放回 TEMP/TMP/USERPROFILE），一并进脱敏表。
        let mut log = String::from_utf8_lossy(&output.stdout).into_owned();
        log.push_str(&String::from_utf8_lossy(&output.stderr));
        let mut redact = map.clone();
        let mut extras = vec![
            (tool_env.rustup_home.clone(), "<rustup-home>"),
            (env::temp_dir(), "<temp>"),
        ];
        if let Ok(profile) = home_dir() {
            extras.push((profile, "<user-profile>"));
        }
        for (root, placeholder) in extras {
            redact.extend(
                both_spellings(&root)
                    .into_iter()
                    .map(|from| RemapRoot { from, placeholder }),
            );
        }
        let log = redact_roots(&log, &redact);
        return Err(format!("{CARGO_COMMAND} 失败：{}\n{log}", output.status));
    }

    let exe_name = if cfg!(windows) {
        "laneflow-urban-harness.exe"
    } else {
        "laneflow-urban-harness"
    };
    let binary = quarantine_dir.join(&host).join("release").join(exe_name);
    if !binary.is_file() {
        return Err(format!(
            "预期二进制不存在：{}",
            display_placeholder(&repo, &binary)
        ));
    }
    // 正向断言：二进制必须嵌入了期望的归一化 flag 串。rustc 会把
    // env!/option_env! 读取的变量记入 dep-info、Cargo 据以触发重编，
    // 本断言是其上的纵深防御（例如同一目录的手工构建或指纹边界）。
    let bytes = fs::read(&binary).map_err(|e| format!("无法读取二进制: {e}"))?;
    if find_all(&bytes, normalized.as_bytes()).is_empty() {
        return Err(format!(
            "二进制未嵌入期望的归一化 rustflags；请删除 {} 后重建",
            display_placeholder(&repo, &quarantine_dir)
        ));
    }

    let mut roots = derived_roots(&tool_env, &repo)?;
    if let Some(root) = evidence_root {
        roots.push(ScanRoot {
            placeholder: "<evidence-root>".into(),
            path: root,
            explicit: true,
        });
    }
    for (index, root) in extra_roots.into_iter().enumerate() {
        roots.push(ScanRoot {
            placeholder: format!("<extra-{}>", index + 1),
            path: root,
            explicit: true,
        });
    }
    let (hits, scan) = scan_binary(&binary, &roots)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&scan).map_err(|e| e.to_string())?
    );
    if !hits.is_empty() {
        return Err("本机路径扫描失败：二进制不得发布".into());
    }

    let receipt = json!({
        "version": "urban-measurement-build-v1",
        "build_parameters": "cargo +1.99.1 run --locked -p xtask -- build-measurement",
        "cargo_command": format!("{CARGO_COMMAND} --target {host}"),
        "source_commit": source_commit,
        "source_tree": source_tree,
        "rustflags_normalized": normalized,
        "build_profile": "release;debug-assertions=off",
        "target": host,
        "profile_pins": PROFILE_PINS.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>(),
        "compiler": "<rustup-toolchain>/bin/rustc",
        "binary": scan["binary"].clone(),
        "scan": scan,
    });
    // 构建与扫描完成后复核源码树未漂移（构建期间的改动不得进入已封存二进制）；
    // 字节层比对同构建前口径。
    if git_output(&["rev-parse", "HEAD"], &repo)? != source_commit
        || git_output(&["rev-parse", "HEAD^{tree}"], &repo)? != source_tree
        || !git_status(&repo)?.is_empty()
    {
        return Err("构建期间源码树发生漂移；二进制未发布，回执未写出".into());
    }
    check_tree_matches_index(&repo)?;
    // 全部证明通过才把二进制发布到正式测量路径（同卷 rename）：此前任何
    // 失败分支都不会在正式路径留下未过检二进制，上一轮合格二进制此刻才被
    // 替换。
    let published = final_dir.join(&host).join("release").join(exe_name);
    let publish_parent = published.parent().expect("发布路径有父目录");
    fs::create_dir_all(publish_parent).map_err(|e| {
        format!(
            "无法创建 {}: {e}",
            display_placeholder(&repo, publish_parent)
        )
    })?;
    if published.exists() {
        fs::remove_file(&published).map_err(|e| {
            format!(
                "无法替换旧二进制 {}: {e}",
                display_placeholder(&repo, &published)
            )
        })?;
    }
    fs::rename(&binary, &published).map_err(|e| {
        format!(
            "无法发布二进制到 {}: {e}",
            display_placeholder(&repo, &published)
        )
    })?;
    write_receipt(&receipt_path, &receipt)?;
    // 控制台输出遵循占位符约定：路径一律以 <worktree> 形式出现。
    let display = display_placeholder(&repo, &receipt_path);
    println!("build-measurement: pass; receipt written to {display}");
    Ok(())
}

/// `check-binary-paths <binary> [ --receipt <path> ] [ --evidence-root <dir> ] [ --extra-root <dir> ]...`
pub(crate) fn check_binary_paths(args: &[String]) -> Result<(), String> {
    let mut binary = None;
    let mut receipt_path = None;
    let mut evidence_root = None;
    let mut extra_roots = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--receipt" => {
                receipt_path = Some(absolute_lexical(PathBuf::from(
                    iter.next().ok_or("--receipt 缺少路径")?,
                ))?)
            }
            "--evidence-root" => {
                evidence_root = Some(absolute_lexical(PathBuf::from(
                    iter.next().ok_or("--evidence-root 缺少路径")?,
                ))?)
            }
            "--extra-root" => extra_roots.push(absolute_lexical(PathBuf::from(
                iter.next().ok_or("--extra-root 缺少路径")?,
            ))?),
            other if binary.is_none() && !other.starts_with("--") => {
                binary = Some(PathBuf::from(other))
            }
            other => return Err(format!("未知 check-binary-paths 参数：{other}")),
        }
    }
    let binary = binary.ok_or("缺少二进制路径")?;
    let repo = repo_root()?;
    let tool_env = ToolEnv::resolve(&repo)?;
    let mut roots = derived_roots(&tool_env, &repo)?;
    if let Some(root) = evidence_root {
        roots.push(ScanRoot {
            placeholder: "<evidence-root>".into(),
            path: root,
            explicit: true,
        });
    }
    for (index, root) in extra_roots.into_iter().enumerate() {
        roots.push(ScanRoot {
            placeholder: format!("<extra-{}>", index + 1),
            path: root,
            explicit: true,
        });
    }
    let (hits, receipt) = scan_binary(&binary, &roots)?;
    let text = serde_json::to_string_pretty(&receipt).map_err(|e| e.to_string())?;
    println!("{text}");
    if let Some(path) = receipt_path {
        write_receipt(&path, &receipt)?;
    }
    if !hits.is_empty() {
        return Err("本机路径扫描失败：二进制不得发布".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_all_reports_every_hit() {
        let haystack = b"abXXabXXab";
        assert_eq!(find_all(haystack, b"ab"), vec![0, 4, 8]);
        assert_eq!(find_all(haystack, b""), Vec::<u64>::new());
        assert_eq!(find_all(b"short", b"longer-needle"), Vec::<u64>::new());
    }

    #[test]
    fn needles_cover_separators_encodings_and_case() {
        let path = Path::new("C:\\Users\\Mix");
        let set = needles(path);
        assert_eq!(set.len(), 4);
        let haystack = b"prefix c:/users/mix/suffix".to_vec();
        let lowered = ascii_lower(&haystack);
        assert!(
            set.iter()
                .any(|(wide, _, needle)| !*wide && !find_all(&lowered, needle).is_empty())
        );
        let wide_bytes: Vec<u8> = "C:\\USERS\\mix"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert!(set.iter().any(|(wide, _, needle)| *wide && {
            let mut padded = vec![0u8; 6];
            padded.extend_from_slice(&wide_bytes);
            !find_all(&utf16le_fold(&padded, 0), needle).is_empty()
        }));
    }

    /// 非 ASCII 路径成分（如中文目录名）的 UTF-16 高字节不得被折叠损坏：
    /// 码元折叠视图必须仍能命中。中（U+4E2D）的高字节 0x4E 即 ASCII 'N'，
    /// 逐字节折叠会把它损坏为 0x6E。
    #[test]
    fn utf16le_fold_preserves_non_ascii_units() {
        let path = Path::new("D:\\中\\Mix");
        let set = needles(path);
        let wide: Vec<u8> = "d:\\中\\mix"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let mut file = vec![0xAAu8];
        file.extend_from_slice(&wide);
        file.push(0);
        let folded = utf16le_fold(&file, 1);
        assert!(
            set.iter()
                .any(|(wide, _, needle)| *wide && !find_all(&folded, needle).is_empty()),
            "非 ASCII UTF-16 路径必须可被扫描命中"
        );
        // 逐字节折叠会损坏 0x4E（'N'）类高字节，不得回退到那种视图。
        let corrupted = ascii_lower(&file);
        assert!(corrupted != folded);
    }

    #[test]
    fn absolute_lexical_collapses_parent_components() {
        let out = absolute_lexical(PathBuf::from("base/sub/../evidence")).unwrap();
        let text = out.to_string_lossy().replace('\\', "/");
        assert!(text.ends_with("base/evidence"), "{text}");
        assert!(!text.contains(".."), "{text}");
    }

    #[test]
    fn redact_roots_replaces_native_and_forward_spellings() {
        let roots = vec![RemapRoot {
            from: PathBuf::from("E:\\work\\laneflow"),
            placeholder: "<worktree>",
        }];
        // 伪造含绝对路径的子进程 stderr：原生与 `/` 两种拼写都不得残留。
        let stderr = "fatal: cannot chdir to E:\\work\\laneflow; see E:/work/laneflow/config";
        let out = redact_roots(stderr, &roots);
        assert_eq!(
            out,
            "fatal: cannot chdir to <worktree>; see <worktree>/config"
        );
    }

    #[test]
    fn redact_roots_folds_ascii_case() {
        let roots = vec![RemapRoot {
            from: PathBuf::from("E:\\Work\\LaneFlow"),
            placeholder: "<worktree>",
        }];
        // 工具诊断的大小写与注册根分叉（驱动器号/组件/8.3 短名）时仍须命中；
        // 非路径文本原样保留。
        let stderr = "fatal: e:\\work\\laneflow broken; E:/WORK/LANEFLOW missing; keep E:x literal";
        let out = redact_roots(stderr, &roots);
        assert_eq!(
            out,
            "fatal: <worktree> broken; <worktree> missing; keep E:x literal"
        );
    }

    #[test]
    fn replace_ascii_ci_preserves_non_path_text_and_utf8() {
        assert_eq!(
            replace_ascii_ci("日志 C:\\users\\ynh2 失败", "c:\\Users\\YNH2", "<u>"),
            "日志 <u> 失败"
        );
        assert_eq!(replace_ascii_ci("无命中", "x:\\y", "<p>"), "无命中");
    }

    #[test]
    fn redact_roots_prefers_finer_placeholder_for_nested_roots() {
        // sysroot 是 rustup-home 的子路径：细粒度占位符须优先于父根。
        let roots = vec![
            RemapRoot {
                from: PathBuf::from("E:/rustup"),
                placeholder: "<rustup-home>",
            },
            RemapRoot {
                from: PathBuf::from("E:/rustup/toolchains/1.99.1-x86_64"),
                placeholder: "<rustup-toolchain>",
            },
        ];
        let log = "error: E:\\rustup\\toolchains\\1.99.1-x86_64\\lib missing; E:/rustup/x";
        assert_eq!(
            redact_roots(log, &roots),
            "error: <rustup-toolchain>\\lib missing; <rustup-home>/x"
        );
    }

    #[test]
    fn parse_index_entries_reads_ls_files_z_records() {
        let text = "100644 abc123 0\tcrates/x/src/lib.rs\x00160644 def456 0\tREADME.md\x00";
        let entries = parse_index_entries(text).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].mode, "100644");
        assert_eq!(entries[0].blob, "abc123");
        assert_eq!(entries[0].path, "crates/x/src/lib.rs");
        assert_eq!(entries[1].path, "README.md");
        assert!(parse_index_entries("100644 abc123 0\n").is_err());
    }

    #[test]
    fn sanitize_include_entry_redacts_machine_paths() {
        let cargo_home = Path::new("E:/cargo-home");
        let repo = Path::new("E:/work/laneflow");
        // 相对条目原样保留（不含本机路径）。
        assert_eq!(
            sanitize_include_entry("sub/extra.toml", cargo_home, repo),
            "sub/extra.toml"
        );
        // 根内绝对路径归占位符，保持可读。
        assert_eq!(
            sanitize_include_entry("E:/cargo-home/shared.toml", cargo_home, repo),
            "<cargo-home>/shared.toml"
        );
        // 根外绝对路径与根相对路径整段占位。
        assert_eq!(
            sanitize_include_entry("/opt/shared/team.toml", cargo_home, repo),
            "<include 绝对路径>"
        );
        assert_eq!(
            sanitize_include_entry("\\\\server\\share\\team.toml", cargo_home, repo),
            "<include 绝对路径>"
        );
    }

    #[test]
    fn path_prefix_match_is_case_and_separator_insensitive() {
        assert!(path_starts_with_ci(
            Path::new("E:/.cargo/registry/src/x"),
            Path::new("e:\\.cargo")
        ));
        assert!(!path_starts_with_ci(
            Path::new("D:/vendor/x"),
            Path::new("e:\\.cargo")
        ));
        // 组件边界：兄弟目录的前缀共享不算命中。
        assert!(!path_starts_with_ci(
            Path::new("D:/work/laneflow-modified/crate"),
            Path::new("D:\\work\\laneflow")
        ));
        assert!(path_starts_with_ci(
            Path::new("D:/work/laneflow/crate"),
            Path::new("D:\\work\\laneflow")
        ));
    }
}
