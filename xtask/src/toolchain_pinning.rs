//! 工具链钉版（#1564）：仓库根 `rust-toolchain.toml` 的 `toolchain.channel` 是工具链
//! 版本的唯一事实来源，根 `Cargo.toml` 的 `rust-version` 与之相等，其余文件不写版本号。
//!
//! - [`require_active_toolchain`]：xtask 每个命令启动前核对当前 rustc 与钉版一致，
//!   拦住 `RUSTUP_TOOLCHAIN` 环境变量或 rustup 目录覆盖造成的偏离。
//! - [`run`]（`check-toolchain-pinning`）：核对两个事实来源与 workspace 成员的继承，
//!   并扫描全仓的硬编码写法；由 `Commit message` 必过检查运行。

use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    process::Command,
};

use serde::Deserialize;

/// 整体跳过扫描的文件清单；格式与约束见该文件头注释。
const EXEMPTIONS_TOML: &str = include_str!("../data/toolchain-pinning.toml");
const TOOLCHAIN_FILE: &str = "rust-toolchain.toml";
const ROOT_MANIFEST: &str = "Cargo.toml";
const LOCKFILE: &str = "Cargo.lock";
const EXEMPTION_FILE: &str = "xtask/data/toolchain-pinning.toml";
/// 经 rustup 代理执行、接受 `+<工具链>` 选择器的命令。
const PROXIED_TOOLS: [&str; 6] = [
    "cargo",
    "rustc",
    "rustdoc",
    "rustup",
    "rustfmt",
    "clippy-driver",
];
/// 单行文本上的写法判定：参数为（该行文本，当前钉版的 channel）。
type Detect = fn(&str, &str) -> bool;
/// 硬编码写法与它的判定；顺序即诊断里的列出顺序。
const PATTERNS: [(&str, Detect); 5] = [
    (
        "命令里的工具链选择器（如 cargo +<版本>）",
        has_tool_selector,
    ),
    ("带版本号的 rustup 管理命令", has_rustup_version_command),
    ("RUSTUP_TOOLCHAIN 环境变量里的版本号", has_environment_pin),
    ("缓存键里的版本号（msrv-<版本>）", has_cache_key_version),
    ("当前钉版的版本号", has_current_channel),
];

/// 仓库根：xtask 所属 workspace 的上一级，与进程 cwd 无关。
pub(crate) fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "xtask 没有父目录，无法定位仓库根".to_owned())
}

/// `rust-toolchain.toml` 钉住的 `toolchain.channel`。
pub(crate) fn pinned_channel(repo: &Path) -> Result<String, String> {
    parse_pin(&read_string(&repo.join(TOOLCHAIN_FILE))?)?
        .channel
        .ok_or_else(|| format!("{TOOLCHAIN_FILE} 缺少非空的 toolchain.channel"))
}

/// 核对当前 rustc 与钉版一致。rustc 以仓库根为 cwd、继承本进程环境执行，结果就是
/// xtask 派生的 cargo / rustc 调用会落到的工具链。
pub(crate) fn require_active_toolchain(repo: &Path) -> Result<(), String> {
    let channel = pinned_channel(repo)?;
    match active_rustc_release(repo) {
        Ok(release) if release == channel => Ok(()),
        Ok(release) => Err(format!(
            "当前 rustc 为 {release}，仓库钉版（{TOOLCHAIN_FILE} 的 toolchain.channel）为 {channel}；请检查 RUSTUP_TOOLCHAIN 环境变量与 rustup 的目录覆盖（rustup override list）"
        )),
        Err(error) => Err(format!(
            "无法核对工具链：{error}；xtask 要求 rustc 可用且与仓库钉版（{channel}）一致"
        )),
    }
}

/// `check-toolchain-pinning` 入口：树内检查逐项列出发现，环境偏离作为最后一项；
/// 有任一发现即失败。
pub(crate) fn run() -> Result<(), String> {
    let repo = workspace_root()?;
    let mut findings = check_tree(&repo)?;
    if let Err(error) = require_active_toolchain(&repo) {
        findings.push(error);
    }
    if findings.is_empty() {
        println!(
            "工具链钉版检查通过：两个事实来源一致，workspace 成员继承 rust-version，全仓无硬编码的工具链版本。"
        );
        return Ok(());
    }
    Err(format!("工具链钉版检查未通过：\n{}", findings.join("\n")))
}

/// 仓库树层面的检查，返回发现清单（空表示通过）；无法完成检查时返回错误。
fn check_tree(repo: &Path) -> Result<Vec<String>, String> {
    let pin = parse_pin(&read_string(&repo.join(TOOLCHAIN_FILE))?)?;
    let Some(channel) = pin.channel else {
        return Ok(vec![format!(
            "{TOOLCHAIN_FILE} 缺少非空的 toolchain.channel；工具链版本只在这里声明"
        )]);
    };
    let mut findings = Vec::new();
    if !is_exact_version(&channel) {
        findings.push(format!(
            "{TOOLCHAIN_FILE} 的 toolchain.channel `{channel}` 须为三段式版本号（如 1.0.0），才能与根 {ROOT_MANIFEST} 的 rust-version 对账"
        ));
    }
    if !pin.extra_keys.is_empty() {
        findings.push(format!(
            "{TOOLCHAIN_FILE} 只允许 [toolchain] 的 channel 一项：CI 与测量工具按固定形态解析，多出的 components / targets 还会让 rustup 自动安装；多出：{}",
            pin.extra_keys.join("、")
        ));
    }
    findings.extend(check_root_manifest(
        &read_string(&repo.join(ROOT_MANIFEST))?,
        &channel,
    ));
    findings.extend(scan_repository(repo, &channel)?);
    Ok(findings)
}

struct Pin {
    channel: Option<String>,
    /// `toolchain.channel` 之外的键，含其他表。
    extra_keys: Vec<String>,
}

/// 解析 `rust-toolchain.toml`；语法错误返回错误，缺少 channel 留给调用方报告。
fn parse_pin(text: &str) -> Result<Pin, String> {
    let value: toml::Value =
        toml::from_str(text).map_err(|error| format!("{TOOLCHAIN_FILE} 解析失败：{error}"))?;
    let channel = value
        .get("toolchain")
        .and_then(|toolchain| toolchain.get("channel"))
        .and_then(toml::Value::as_str)
        .filter(|channel| !channel.is_empty())
        .map(str::to_owned);
    let mut extra_keys = Vec::new();
    for (key, item) in value.as_table().into_iter().flatten() {
        let toolchain = match (key.as_str(), item) {
            ("toolchain", toml::Value::Table(toolchain)) => toolchain,
            _ => {
                extra_keys.push(key.clone());
                continue;
            }
        };
        for (name, item) in toolchain {
            if name == "channel" && item.as_str().is_some_and(|channel| !channel.is_empty()) {
                continue;
            }
            extra_keys.push(format!("toolchain.{name}"));
        }
    }
    Ok(Pin {
        channel,
        extra_keys,
    })
}

/// 三段式且各段非空的纯数字版本号；根清单的 `rust-version` 取它的逐字对应值。
fn is_exact_version(channel: &str) -> bool {
    let parts: Vec<&str> = channel.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn active_rustc_release(cwd: &Path) -> Result<String, String> {
    let output = Command::new("rustc")
        .arg("-vV")
        .current_dir(cwd)
        .output()
        .map_err(|error| format!("无法执行 rustc -vV：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "rustc -vV 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|error| format!("rustc -vV 输出非 UTF-8：{error}"))?;
    text.lines()
        .find_map(|line| line.strip_prefix("release: "))
        .map(str::to_owned)
        .ok_or_else(|| "rustc -vV 输出缺少 release 行".to_owned())
}

/// 根 `Cargo.toml` 的 `workspace.package.rust-version` 须与钉版逐字相等。
fn check_root_manifest(text: &str, channel: &str) -> Vec<String> {
    let Ok(value) = toml::from_str::<toml::Value>(text) else {
        return vec![format!("根 {ROOT_MANIFEST} 解析失败")];
    };
    let declared = value
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("rust-version"))
        .and_then(toml::Value::as_str);
    match declared {
        Some(declared) if declared == channel => Vec::new(),
        Some(declared) => vec![format!(
            "根 {ROOT_MANIFEST} 的 workspace.package.rust-version 为 `{declared}`，与 {TOOLCHAIN_FILE} 的 toolchain.channel `{channel}` 不同；升级时两处一起改"
        )],
        None => vec![format!(
            "根 {ROOT_MANIFEST} 缺少 workspace.package.rust-version，无法与 {TOOLCHAIN_FILE} 的 toolchain.channel `{channel}` 对账"
        )],
    }
}

/// 清单里的 `package.rust-version` 不是 `workspace = true` 时的发现。workspace 成员
/// 必须有这一项（`required`）；其他清单（如 fuzz/）只在写了字面量时报告。
fn literal_rust_version(owner: &str, text: &str, required: bool) -> Option<String> {
    let value: toml::Value = toml::from_str(text).ok()?;
    let declared = value
        .get("package")
        .and_then(|package| package.get("rust-version"));
    let Some(declared) = declared else {
        return required.then(|| {
            format!("`{owner}` 缺少 package.rust-version；workspace 成员须写 `rust-version.workspace = true`")
        });
    };
    let inherits = declared
        .as_table()
        .is_some_and(|table| table.len() == 1 && table["workspace"].as_bool() == Some(true));
    (!inherits).then(|| {
        format!(
            "`{owner}` 的 package.rust-version 须为 `workspace = true`；工具链版本只写在根 {ROOT_MANIFEST}"
        )
    })
}

/// workspace 成员清单的仓库相对路径：把根清单 `members` 的 glob 套在仓库内的清单
/// 目录上，再扣除 `exclude`。返回（成员清单，发现）：按字面量写出的成员找不到
/// 清单文件时记为发现；glob 没有命中任何目录不报，与 cargo 一致。
fn member_manifests(root_manifest: &str, tracked: &[String]) -> (BTreeSet<String>, Vec<String>) {
    let Ok(root) = toml::from_str::<toml::Value>(root_manifest) else {
        return (
            BTreeSet::new(),
            vec![format!("根 {ROOT_MANIFEST} 解析失败")],
        );
    };
    let Some(workspace) = root.get("workspace").and_then(toml::Value::as_table) else {
        return (
            BTreeSet::new(),
            vec![format!("根 {ROOT_MANIFEST} 缺少 [workspace]")],
        );
    };
    let list = |key: &str| -> Vec<String> {
        workspace
            .get(key)
            .and_then(toml::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(toml::Value::as_str)
                    .map(|item| item.trim_end_matches('/').to_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    let members = list("members");
    let mut findings = Vec::new();
    if members.is_empty() {
        findings.push(format!(
            "根 {ROOT_MANIFEST} 的 workspace.members 缺失、为空或不是字符串数组"
        ));
    }
    let exclude = list("exclude");
    let directories: BTreeSet<&str> = tracked
        .iter()
        .filter_map(|path| path.strip_suffix("/Cargo.toml"))
        .collect();
    let mut manifests = BTreeSet::new();
    for pattern in &members {
        let matched: Vec<String> = directories
            .iter()
            .copied()
            .filter(|directory| {
                crate::ci_plan::glob_match(pattern, directory)
                    && !exclude
                        .iter()
                        .any(|excluded| crate::ci_plan::glob_match(excluded, directory))
            })
            .map(|directory| format!("{directory}/Cargo.toml"))
            .collect();
        if matched.is_empty() && !pattern.contains('*') {
            findings.push(format!("workspace 成员 `{pattern}` 下没有 Cargo.toml"));
        }
        manifests.extend(matched);
    }
    (manifests, findings)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExemptionFile {
    exempt: Vec<Exemption>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Exemption {
    path: String,
    reason: String,
}

/// 豁免清单里的仓库相对路径（正斜杠）。
fn exemptions() -> Result<BTreeSet<String>, String> {
    let file: ExemptionFile = toml::from_str(EXEMPTIONS_TOML)
        .map_err(|error| format!("{EXEMPTION_FILE} 解析失败：{error}"))?;
    let mut paths = BTreeSet::new();
    for exemption in file.exempt {
        let path = exemption.path.trim();
        if path.is_empty() || exemption.reason.trim().is_empty() {
            return Err(format!(
                "{EXEMPTION_FILE} 的豁免条目须同时写 path 与 reason"
            ));
        }
        if path.contains('\\') || path.starts_with('/') || path.split('/').any(|part| part == "..")
        {
            return Err(format!(
                "{EXEMPTION_FILE} 的豁免路径须是仓库相对路径（正斜杠）：{path}"
            ));
        }
        if !paths.insert(path.to_owned()) {
            return Err(format!("{EXEMPTION_FILE} 的豁免条目重复：{path}"));
        }
    }
    Ok(paths)
}

/// 扫描纳入版本控制的全部文本文件并核对成员清单的继承；豁免条目须指向其中的
/// 文件。只读文件与 `git ls-files`，不运行 cargo：钉版的工具链未安装时检查照样可做。
fn scan_repository(repo: &Path, channel: &str) -> Result<Vec<String>, String> {
    let exempt = exemptions()?;
    let tracked = tracked_files(repo)?;
    let (members, mut findings) =
        member_manifests(&read_string(&repo.join(ROOT_MANIFEST))?, &tracked);
    let mut files = Vec::new();
    for path in &tracked {
        if exempt.contains(path) {
            continue;
        }
        if let Some(text) = read_text(&repo.join(path))? {
            files.push((path.clone(), text));
        }
    }
    findings.extend(scan_files(&files, channel, &exempt, &members));
    for manifest in &members {
        let text = read_string(&repo.join(manifest))?;
        findings.extend(literal_rust_version(manifest, &text, true));
    }
    findings.extend(
        exempt
            .iter()
            .filter(|path| !tracked.contains(*path))
            .map(|path| format!("豁免条目 `{path}` 不是纳入版本控制的文件；条目失效后请删除")),
    );
    Ok(findings)
}

/// 对一组（仓库相对路径，文本）执行扫描。两个事实来源与 `Cargo.lock` 不参与：
/// 前者由专属检查核对，后者的版本号属于依赖而非工具链；成员清单由调用方单独核对。
fn scan_files(
    files: &[(String, String)],
    channel: &str,
    exempt: &BTreeSet<String>,
    members: &BTreeSet<String>,
) -> Vec<String> {
    let mut findings = Vec::new();
    for (path, text) in files {
        if matches!(path.as_str(), ROOT_MANIFEST | LOCKFILE | TOOLCHAIN_FILE)
            || exempt.contains(path)
        {
            continue;
        }
        for (line, patterns, excerpt) in scan_text(text, channel) {
            findings.push(format!(
                "{path}:{line}: {patterns}；工具链版本只写在 {TOOLCHAIN_FILE}：{excerpt}"
            ));
        }
        // 不在 workspace 内的清单（如 fuzz/）没有成员检查兜底。
        if path.ends_with("/Cargo.toml") && !members.contains(path) {
            findings.extend(literal_rust_version(path, text, false));
        }
    }
    findings
}

/// 已跟踪的文件与未忽略的待加入文件（提交前新增的文件同样在扫描与豁免核对
/// 范围内）；干净检出下等于全部被跟踪的文件。
fn tracked_files(repo: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(repo)
        .output()
        .map_err(|error| format!("无法启动 git ls-files：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git ls-files 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|error| format!("git ls-files 输出非 UTF-8：{error}"))?;
    Ok(text
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect())
}

/// 文件全文；二进制文件（开头 8 KiB 内含 NUL）与不存在的文件返回 `None`，
/// 二进制文件不读入其余部分。
fn read_text(path: &Path) -> Result<Option<String>, String> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("无法打开 {}：{error}", path.display())),
    };
    let mut bytes = vec![0u8; 8 * 1024];
    let filled = fill(&mut file, &mut bytes)
        .map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
    bytes.truncate(filled);
    if bytes.contains(&0) {
        return Ok(None);
    }
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

/// 读满缓冲区或读到文件尾，返回读到的字节数（`read` 一次可能读不满）。
fn fill(reader: &mut impl Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        let read = reader.read(&mut buffer[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}

/// 一行命中的写法与该行摘录：`(行号, 写法, 摘录)`。
type Hit = (usize, String, String);

/// 逐行扫描：一行命中几种写法合并为一条，行号从 1 起。
fn scan_text(text: &str, channel: &str) -> Vec<Hit> {
    text.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let patterns: Vec<&str> = PATTERNS
                .iter()
                .filter(|(_, detect)| detect(line, channel))
                .map(|(description, _)| *description)
                .collect();
            (!patterns.is_empty()).then(|| (index + 1, patterns.join("、"), excerpt(line)))
        })
        .collect()
}

fn excerpt(line: &str) -> String {
    const LIMIT: usize = 100;
    let trimmed = line.trim();
    if trimmed.chars().count() <= LIMIT {
        return trimmed.to_owned();
    }
    let mut cut: String = trimmed.chars().take(LIMIT).collect();
    cut.push('…');
    cut
}

/// `word` 的出现位置，其前一个字符须不是标识符字符（`mycargo` 不算 `cargo`）。
fn word_offsets<'a>(line: &'a str, word: &'a str) -> impl Iterator<Item = usize> + 'a {
    line.match_indices(word).filter_map(move |(at, _)| {
        let boundary = line[..at].chars().next_back().is_none_or(|previous| {
            !previous.is_ascii_alphanumeric() && !matches!(previous, '_' | '-')
        });
        boundary.then_some(at)
    })
}

/// `<数字>.<数字>` 开头的记号：`1.0.0`、`1.0.0-x86_64` 与两段式 `1.0` 都算，
/// `nightly` 一类的通道名不算。
fn starts_with_version(token: &str) -> bool {
    token.split_once('.').is_some_and(|(major, rest)| {
        !major.is_empty()
            && major.bytes().all(|byte| byte.is_ascii_digit())
            && rest.starts_with(|lead: char| lead.is_ascii_digit())
    })
}

fn has_tool_selector(line: &str, _channel: &str) -> bool {
    PROXIED_TOOLS.iter().any(|tool| {
        word_offsets(line, tool).any(|at| {
            line[at + tool.len()..]
                .trim_start_matches([' ', '\t'])
                .strip_prefix('+')
                .and_then(|rest| rest.split_whitespace().next())
                .is_some_and(starts_with_version)
        })
    })
}

fn has_rustup_version_command(line: &str, _channel: &str) -> bool {
    word_offsets(line, "rustup").any(|at| {
        let words: Vec<&str> = line[at + "rustup".len()..].split_whitespace().collect();
        let version_position = match words.as_slice() {
            ["toolchain", "install" | "uninstall", ..] | ["override", "set", ..] => 2,
            ["default" | "run" | "update", _, ..] => 1,
            _ => return false,
        };
        words
            .get(version_position)
            .copied()
            .is_some_and(starts_with_version)
    })
}

fn has_environment_pin(line: &str, _channel: &str) -> bool {
    const KEY: &str = "RUSTUP_TOOLCHAIN";
    word_offsets(line, KEY).any(|at| {
        line[at + KEY.len()..]
            .trim_start_matches(['"', '\''])
            .trim_start()
            .strip_prefix([':', '='])
            .is_some_and(|rest| {
                rest.trim_start_matches([' ', '\t', '"', '\''])
                    .starts_with(|lead: char| lead.is_ascii_digit())
            })
    })
}

fn has_cache_key_version(line: &str, _channel: &str) -> bool {
    const KEY: &str = "msrv-";
    line.match_indices(KEY)
        .any(|(at, _)| line[at + KEY.len()..].starts_with(|lead: char| lead.is_ascii_digit()))
}

/// 钉版版本号本身；两侧紧邻的字符不得是数字、`.` 或 `_`。`11.0.0` 与 `1.0.0.1`
/// 是别的版本号，`1.0.0-x86_64` 这种带主机后缀的写法算命中。
fn has_current_channel(line: &str, channel: &str) -> bool {
    if channel.is_empty() {
        return false;
    }
    let other = |neighbor: Option<char>| {
        neighbor.is_none_or(|neighbor| !neighbor.is_ascii_digit() && !matches!(neighbor, '.' | '_'))
    };
    line.match_indices(channel).any(|(at, found)| {
        other(line[..at].chars().next_back()) && other(line[at + found.len()..].chars().next())
    })
}

fn read_string(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("无法读取 {}：{error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 固定虚拟版本：升级 Rust 不触及这些测试。
    const CHANNEL: &str = "1.0.0";
    const OTHER: &str = "9.9.9";

    #[test]
    fn pin_reads_the_channel_and_reports_extra_keys() {
        let pin = parse_pin("# 钉版\n[toolchain]\nchannel = \"1.0.0\"\n").unwrap();
        assert_eq!(pin.channel.as_deref(), Some("1.0.0"));
        assert!(pin.extra_keys.is_empty());
        let pin = parse_pin(
            "[toolchain]\nchannel = \"1.0.0\"\ntargets = [\"wasm32-unknown-unknown\"]\n[net]\n",
        )
        .unwrap();
        // toml 表按键名排序，发现的顺序随之固定。
        assert_eq!(pin.extra_keys, ["net", "toolchain.targets"]);
        assert!(
            parse_pin("[toolchain]\nchannel = \"\"\n")
                .unwrap()
                .channel
                .is_none()
        );
        assert!(parse_pin("[toolchain]\n").unwrap().channel.is_none());
        assert!(parse_pin("[toolchain\n").is_err());
    }

    #[test]
    fn exact_version_requires_three_numeric_parts() {
        for version in ["1.0.0", "10.20.30"] {
            assert!(is_exact_version(version), "{version}");
        }
        for version in [
            "",
            "1",
            "1.0",
            "1.0.0.0",
            "1.0.0-beta",
            "stable",
            "1..0",
            "v1.0.0",
        ] {
            assert!(!is_exact_version(version), "{version}");
        }
    }

    #[test]
    fn root_manifest_must_match_the_pin_verbatim() {
        let manifest =
            |declared: &str| format!("[workspace.package]\nrust-version = \"{declared}\"\n");
        assert_eq!(
            check_root_manifest(&manifest(CHANNEL), CHANNEL),
            Vec::<String>::new()
        );
        // 钉版是三段式：Cargo 眼中等值的 `1.0` 也不算逐字相等，升级时两处写法须一致。
        assert_eq!(check_root_manifest(&manifest("1.0"), CHANNEL).len(), 1);
        assert_eq!(check_root_manifest(&manifest(OTHER), CHANNEL).len(), 1);
        assert_eq!(
            check_root_manifest("[workspace.package]\n", CHANNEL).len(),
            1
        );
        assert_eq!(check_root_manifest("[workspace.", CHANNEL).len(), 1);
    }

    #[test]
    fn rust_version_inheritance_is_required_for_members_only() {
        let inherits = "[package]\nname = \"m\"\nrust-version.workspace = true\n";
        assert_eq!(literal_rust_version("m", inherits, true), None);
        assert_eq!(literal_rust_version("m", inherits, false), None);
        let bare = "[package]\nname = \"m\"\n";
        assert_eq!(literal_rust_version("m", bare, false), None);
        assert!(literal_rust_version("m", bare, true).is_some());
        for literal in [
            "[package]\nname = \"m\"\nrust-version = \"1.0.0\"\n",
            "[package]\nname = \"m\"\nrust-version = { workspace = false }\n",
            "[package]\nname = \"m\"\nrust-version = { workspace = true, x = 1 }\n",
        ] {
            assert!(
                literal_rust_version("m", literal, false).is_some(),
                "{literal}"
            );
            assert!(
                literal_rust_version("m", literal, true).is_some(),
                "{literal}"
            );
        }
    }

    #[test]
    fn member_manifests_follow_root_members_and_exclude() {
        let tracked: Vec<String> = [
            "Cargo.toml",
            "xtask/Cargo.toml",
            "crates/a/Cargo.toml",
            "crates/skip/Cargo.toml",
            "fuzz/Cargo.toml",
            "tools/t/Cargo.toml",
        ]
        .map(str::to_owned)
        .to_vec();
        let root =
            "[workspace]\nmembers = [\"xtask/\", \"crates/*\"]\nexclude = [\"crates/skip\"]\n";
        let (members, findings) = member_manifests(root, &tracked);
        assert_eq!(
            members,
            BTreeSet::from([
                "crates/a/Cargo.toml".to_owned(),
                "xtask/Cargo.toml".to_owned()
            ])
        );
        assert!(findings.is_empty(), "{findings:#?}");
        // 跨目录 glob 命中嵌套的清单；根清单自身不是成员。
        let deep = "[workspace]\nmembers = [\"**\"]\nexclude = [\"fuzz\"]\n";
        let (members, _) = member_manifests(deep, &tracked);
        assert_eq!(members.len(), 4, "{members:#?}");
        assert!(!members.contains("Cargo.toml"));
        // 字面量成员找不到清单文件、members 缺失或清单语法错误：都记为发现。
        let (members, findings) = member_manifests("[workspace]\nmembers = [\"gone\"]\n", &tracked);
        assert!(members.is_empty());
        assert_eq!(findings.len(), 1, "{findings:#?}");
        let (_, findings) = member_manifests("[workspace]\nmembers = [\"nothing/*\"]\n", &tracked);
        assert!(findings.is_empty(), "{findings:#?}");
        for broken in [
            "[workspace]\n",
            "[workspace]\nmembers = 1\n",
            "[workspace\n",
            "[package]\n",
        ] {
            let (_, findings) = member_manifests(broken, &tracked);
            assert_eq!(findings.len(), 1, "{broken:?} {findings:#?}");
        }
    }

    #[test]
    fn command_forms_are_flagged_whatever_the_version() {
        // 钉版取一个与样例无关的值：这里只验证与钉版无关的命令写法。
        let text = concat!(
            "cargo +1.0.0 build\n",
            "rustc +1.0.0-x86_64 -Vv\n",
            "cargo   +1.0 test\n",
            "rustfmt +1.0.0 --check\n",
            "rustup toolchain install 1.0.0 --profile minimal\n",
            "rustup toolchain uninstall 1.0.0\n",
            "rustup default 1.0.0\n",
            "rustup override set 1.0.0\n",
            "rustup run 1.0.0 cargo build\n",
            "rustup update 1.0.0\n",
            "RUSTUP_TOOLCHAIN: 1.0.0\n",
            "RUSTUP_TOOLCHAIN=1.0.0\n",
            "RUSTUP_TOOLCHAIN=\"1.0.0\"\n",
            "shared-key: laneflow-msrv-1.0.0-rust\n",
            "前缀 cargo +1.0.0 build 后缀\n",
        );
        let hits = scan_text(text, OTHER);
        assert_eq!(
            hits.iter().map(|(line, _, _)| *line).collect::<Vec<_>>(),
            (1..=15).collect::<Vec<_>>()
        );
        assert!(
            hits.iter()
                .all(|(_, patterns, _)| !patterns.contains("当前钉版"))
        );
        assert_eq!(hits[0].1, PATTERNS[0].0);
        assert_eq!(hits[0].2, "cargo +1.0.0 build");
        assert_eq!(hits[14].2, "前缀 cargo +1.0.0 build 后缀");
    }

    #[test]
    fn placeholders_named_channels_and_unrelated_text_are_not_flagged() {
        let text = concat!(
            "cargo +nightly fuzz run\n",
            "cargo run --locked -p xtask\n",
            "cargo +\"$CHANNEL\" build\n",
            "rustup toolchain install \"$CHANNEL\" --profile minimal\n",
            "rustup show active-toolchain\n",
            "rustup default stable\n",
            "RUSTUP_TOOLCHAIN=\"$CHANNEL\" rustc --version\n",
            "RUSTUP_TOOLCHAIN 环境变量\n",
            "请检查 RUSTUP_TOOLCHAIN 环境变量与 rustup override\n",
            "laneflow-msrv-${{ steps.setup-rust.outputs.toolchain }}-rust\n",
            "Rust 1.89 已稳定相关 intrinsics，Bevy 0.20.0 的 MSRV 为 Rust 1.97.1\n",
        );
        assert_eq!(scan_text(text, OTHER), Vec::<Hit>::new());
        assert_eq!(scan_text(text, CHANNEL), Vec::<Hit>::new());
        // 工具名前面紧贴标识符字符的不算该命令。
        let not_tools = concat!(
            "mycargo +9.9.9 x\n",
            "cargo-nextest +9.9.9\n",
            "laneflow_msrv_9_9_9\n"
        );
        assert_eq!(scan_text(not_tools, CHANNEL), Vec::<Hit>::new());
    }

    #[test]
    fn current_channel_is_flagged_with_version_boundaries() {
        let text = concat!(
            "需要 Rust 1.0.0：\n",
            "版本 11.0.0 与 1.0.0.1 不是它\n",
            "工具链目录 1.0.0-x86_64 下\n",
            "Rust 1.0（两段式写法不在扫描内）\n",
            "x1.0.0\n",
        );
        let hits = scan_text(text, CHANNEL);
        assert_eq!(
            hits.iter().map(|(line, _, _)| *line).collect::<Vec<_>>(),
            [1, 3, 5]
        );
        assert!(
            hits.iter()
                .all(|(_, patterns, _)| patterns == PATTERNS[4].0)
        );
        assert_eq!(scan_text("需要 Rust 1.0.0\n", ""), Vec::<Hit>::new());
    }

    #[test]
    fn several_patterns_on_one_line_merge_into_one_hit() {
        let hits = scan_text("RUSTUP_TOOLCHAIN=1.0.0 cargo +1.0.0 build\n", CHANNEL);
        assert_eq!(hits.len(), 1);
        let patterns: Vec<&str> = hits[0].1.split('、').collect();
        assert_eq!(patterns, [PATTERNS[0].0, PATTERNS[2].0, PATTERNS[4].0]);
    }

    #[test]
    fn long_lines_are_cut_in_the_excerpt() {
        let long = format!("cargo {} build", "x".repeat(300));
        assert_eq!(excerpt(&long).chars().count(), 101);
        assert!(excerpt(&long).ends_with('…'));
        assert_eq!(excerpt("  cargo build  "), "cargo build");
    }

    #[test]
    fn scan_skips_sources_of_truth_lockfile_and_exempt_files() {
        let files = [
            (
                "rust-toolchain.toml".to_owned(),
                "channel = \"1.0.0\"\n".to_owned(),
            ),
            (
                "Cargo.toml".to_owned(),
                "rust-version = \"1.0.0\"\n".to_owned(),
            ),
            ("Cargo.lock".to_owned(), "version = \"1.0.0\"\n".to_owned()),
            ("docs/current.md".to_owned(), "需要 Rust 1.0.0\n".to_owned()),
            ("docs/old.md".to_owned(), "cargo +1.0.0 build\n".to_owned()),
            (
                "fuzz/Cargo.toml".to_owned(),
                "[package]\nname = \"f\"\n".to_owned(),
            ),
            (
                "tools/x/Cargo.toml".to_owned(),
                format!("[package]\nname = \"x\"\nrust-version = \"{OTHER}\"\n"),
            ),
            (
                "crates/m/Cargo.toml".to_owned(),
                "[package]\nname = \"m\"\n".to_owned(),
            ),
        ];
        let exempt = BTreeSet::from(["docs/old.md".to_owned()]);
        let members = BTreeSet::from(["crates/m/Cargo.toml".to_owned()]);
        let findings = scan_files(&files, CHANNEL, &exempt, &members);
        assert_eq!(findings.len(), 2, "{findings:#?}");
        assert!(
            findings[0].starts_with("docs/current.md:1: 当前钉版的版本号"),
            "{findings:#?}"
        );
        assert!(
            findings[1].starts_with("`tools/x/Cargo.toml` 的 package.rust-version"),
            "{findings:#?}"
        );
    }

    #[test]
    fn exemption_list_is_well_formed() {
        let paths = exemptions().unwrap();
        assert!(paths.contains("xtask/src/toolchain_pinning.rs"));
    }

    #[test]
    fn text_reader_skips_binary_and_missing_files() {
        let root = std::env::temp_dir().join(format!(
            "laneflow-xtask-toolchain-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("text.txt"), "cargo build\n").unwrap();
        fs::write(root.join("long-text.txt"), vec![b'a'; 9_000]).unwrap();
        fs::write(root.join("binary.bin"), vec![0u8; 16]).unwrap();
        assert_eq!(
            read_text(&root.join("text.txt")).unwrap().as_deref(),
            Some("cargo build\n")
        );
        assert_eq!(read_text(&root.join("binary.bin")).unwrap(), None);
        // 开头 8 KiB 之后的内容照常读入：只探测开头，不截断文本。
        assert!(read_text(&root.join("long-text.txt")).unwrap().is_some());
        assert_eq!(read_text(&root.join("missing.txt")).unwrap(), None);
        let _ = fs::remove_dir_all(&root);
    }

    /// 开发期护栏，与 `check-toolchain-pinning` 命令检查同一棵真实仓库；CI 由该命令
    /// 在 `Commit message` job 常驻执行。
    #[test]
    fn real_tree_is_pinned_consistently() {
        let repo = workspace_root().unwrap();
        // 成员清单不能是空集：静态展开须真的找到 workspace 成员，否则继承检查空转。
        let (members, findings) = member_manifests(
            &read_string(&repo.join(ROOT_MANIFEST)).unwrap(),
            &tracked_files(&repo).unwrap(),
        );
        assert!(findings.is_empty(), "{findings:#?}");
        for expected in ["xtask/Cargo.toml", "crates/laneflow-runtime/Cargo.toml"] {
            assert!(members.contains(expected), "缺少 {expected}");
        }
        assert_eq!(check_tree(&repo).unwrap(), Vec::<String>::new());
        require_active_toolchain(&repo).expect("测试须在仓库钉版的工具链下运行");
    }
}
