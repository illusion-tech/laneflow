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

const TOOLCHAIN_ARG: &str = "+1.98.0";
/// 冻结的 cargo 构建命令（测量 provenance 的 build_parameters 记录本 xtask 入口，
/// 本常量记录脚本内部实际执行的命令）。
const CARGO_COMMAND: &str = "cargo +1.98.0 build -p laneflow-urban-harness --release --locked";
/// 固定 profile 钉值：覆盖采集机可能配置的环境或 config 覆盖。
const PROFILE_PINS: [(&str, &str); 8] = [
    ("CARGO_PROFILE_RELEASE_OPT_LEVEL", "3"),
    ("CARGO_PROFILE_RELEASE_DEBUG", "false"),
    ("CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS", "false"),
    ("CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS", "false"),
    ("CARGO_PROFILE_RELEASE_LTO", "false"),
    ("CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "16"),
    ("CARGO_PROFILE_RELEASE_PANIC", "unwind"),
    ("CARGO_PROFILE_RELEASE_STRIP", "false"),
];
/// 会替换编译器或包装编译器的环境变量：全部清除，改由 CARGO_BUILD_RUSTC 钉住
/// 工具链内的 rustc。
const COMPILER_ENV_UNSET: [&str; 5] = [
    "RUSTC",
    "RUSTC_WRAPPER",
    "CARGO_BUILD_RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
];

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

fn command_output(program: &str, args: &[&str], cwd: &Path) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| format!("无法执行 {program}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} {:?} 失败：{}",
            args,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn toolchain_sysroot(repo: &Path) -> Result<PathBuf, String> {
    let sysroot = command_output("rustc", &[TOOLCHAIN_ARG, "--print", "sysroot"], repo)?;
    let path = PathBuf::from(&sysroot);
    if !path.is_dir() {
        return Err(format!("rustup 工具链 sysroot 不可解析：{sysroot}"));
    }
    Ok(path)
}

fn home_dir() -> Result<PathBuf, String> {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| "无法定位用户目录（USERPROFILE/HOME）".to_string())
}

fn cargo_home() -> Result<PathBuf, String> {
    if let Some(value) = env::var_os("CARGO_HOME") {
        return Ok(PathBuf::from(value));
    }
    Ok(home_dir()?.join(".cargo"))
}

fn rustup_home() -> Result<PathBuf, String> {
    if let Some(value) = env::var_os("RUSTUP_HOME") {
        return Ok(PathBuf::from(value));
    }
    Ok(home_dir()?.join(".rustup"))
}

/// 小写化并统一分隔符后的前缀匹配：Windows 文件系统大小写不敏感，依赖来源
/// 检查不得因盘符或路径大小写差异放行逃逸。
fn path_starts_with_ci(path: &Path, prefix: &Path) -> bool {
    let normalize = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    normalize(path).starts_with(&normalize(prefix))
}

/// 依赖来源根检查：每个依赖包的 manifest 必须落在工作树或 Cargo 主目录内；
/// 目录替换（vendor）等来源会让 remap 覆盖不到，构建必须 fail closed。
fn check_dependency_roots(repo: &Path, cargo_home: &Path) -> Result<(), String> {
    let stdout = command_output(
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

/// UTF-16LE 编码的 ASCII 大小写折叠（高字节为 0 的 ASCII 大写转小写）。
fn utf16le_ascii_lower(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for pair in out.chunks_exact_mut(2) {
        if pair[1] == 0 && pair[0].is_ascii_uppercase() {
            pair[0] += 32;
        }
    }
    out
}

/// 每个根展开的搜索针：两种目录分隔符 × UTF-8/UTF-16LE，全部 ASCII 大小写折叠。
fn needles(path: &Path) -> Vec<(&'static str, Vec<u8>)> {
    let text = path
        .to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_owned();
    let mut out = Vec::new();
    for separator in ['\\', '/'] {
        let surface = text.replace(['\\', '/'], &separator.to_string());
        let utf8 = surface.as_bytes().to_vec();
        out.push((
            if separator == '\\' {
                "utf-8 \\"
            } else {
                "utf-8 /"
            },
            ascii_lower(&utf8),
        ));
        let wide: Vec<u8> = surface.encode_utf16().flat_map(u16::to_le_bytes).collect();
        out.push((
            if separator == '\\' {
                "utf-16le \\"
            } else {
                "utf-16le /"
            },
            utf16le_ascii_lower(&wide),
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
fn scan_binary(
    binary: &Path,
    roots: &[ScanRoot],
) -> Result<(Vec<ScanHit>, serde_json::Value), String> {
    let bytes =
        fs::read(binary).map_err(|e| format!("无法读取二进制 {}: {e}", binary.display()))?;
    let lowered = ascii_lower(&bytes);
    let mut hits = Vec::new();
    for root in roots {
        if !root.explicit && !root.path.is_dir() {
            return Err(format!(
                "派生扫描根 {} 在本机不存在；构建/采集机的扫描根必须可解析",
                root.placeholder
            ));
        }
        for (encoding_separator, needle) in needles(&root.path) {
            let found = find_all(&lowered, &needle);
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

fn derived_roots(repo: &Path) -> Result<Vec<ScanRoot>, String> {
    let sysroot = toolchain_sysroot(repo)?;
    Ok(vec![
        ScanRoot {
            placeholder: "<user-profile>".into(),
            path: home_dir()?,
            explicit: false,
        },
        ScanRoot {
            placeholder: "<cargo-home>".into(),
            path: cargo_home()?,
            explicit: false,
        },
        ScanRoot {
            placeholder: "<rustup-home>".into(),
            path: rustup_home()?,
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

fn write_receipt(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    text.push('\n');
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()))
        .map_err(|e| format!("回执必须写入新路径 {}: {e}", path.display()))
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
                receipt_path = Some(PathBuf::from(iter.next().ok_or("--receipt 缺少路径")?))
            }
            "--evidence-root" => {
                evidence_root = Some(PathBuf::from(
                    iter.next().ok_or("--evidence-root 缺少路径")?,
                ))
            }
            "--extra-root" => {
                extra_roots.push(PathBuf::from(iter.next().ok_or("--extra-root 缺少路径")?))
            }
            other => return Err(format!("未知 build-measurement 参数：{other}")),
        }
    }
    let repo = repo_root()?;
    let sysroot = toolchain_sysroot(&repo)?;
    let cargo_home = cargo_home()?;
    check_dependency_roots(&repo, &cargo_home)?;

    // 唯一映射表：实际 flag 与归一化记录同源于此，二者不可能漂移。
    let map = [
        RemapRoot {
            from: repo.clone(),
            placeholder: "<worktree>",
        },
        RemapRoot {
            from: cargo_home.clone(),
            placeholder: "<cargo-home>",
        },
        RemapRoot {
            from: sysroot.clone(),
            placeholder: "<rustup-toolchain>",
        },
    ];
    let actual = map
        .iter()
        .map(|r| format!("--remap-path-prefix={}={}", r.from.display(), r.placeholder))
        .collect::<Vec<_>>()
        .join("\u{1f}");
    let normalized = map
        .iter()
        .map(|r| format!("--remap-path-prefix={0}={0}", r.placeholder))
        .collect::<Vec<_>>()
        .join(" ");

    let target_dir = repo.join("target").join("release-measurement");
    let receipt_path = receipt_path.unwrap_or_else(|| target_dir.join("build-receipt.json"));
    let mut command = Command::new("cargo");
    command
        .args([
            TOOLCHAIN_ARG,
            "build",
            "-p",
            "laneflow-urban-harness",
            "--release",
            "--locked",
        ])
        .current_dir(&repo)
        .env("CARGO_ENCODED_RUSTFLAGS", &actual)
        .env("LANEFLOW_MEASUREMENT_RUSTFLAGS", &normalized)
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_TARGET_DIR", &target_dir)
        // CARGO_ENCODED_RUSTFLAGS 在 Cargo flag 来源中优先级最高，覆盖 config 的
        // build.rustflags；RUSTFLAGS 及编译器包装变量一律清除，编译器钉到工具链内 rustc。
        .env_remove("RUSTFLAGS")
        .env(
            "CARGO_BUILD_RUSTC",
            sysroot
                .join("bin")
                .join(if cfg!(windows) { "rustc.exe" } else { "rustc" }),
        );
    for name in COMPILER_ENV_UNSET {
        command.env_remove(name);
    }
    for (name, value) in PROFILE_PINS {
        command.env(name, value);
    }
    let status = command
        .status()
        .map_err(|e| format!("无法启动 cargo: {e}"))?;
    if !status.success() {
        return Err(format!("{CARGO_COMMAND} 失败：{status}"));
    }

    let binary = target_dir.join("release").join(if cfg!(windows) {
        "laneflow-urban-harness.exe"
    } else {
        "laneflow-urban-harness"
    });
    if !binary.is_file() {
        return Err(format!("预期二进制不存在：{}", binary.display()));
    }
    // 正向断言：二进制必须嵌入了期望的归一化 flag 串。option_env! 不进 cargo
    // 重建指纹，陈旧复用在本步暴露，而不是留到测量 provenance 才失败。
    let bytes = fs::read(&binary).map_err(|e| format!("无法读取二进制: {e}"))?;
    if find_all(&bytes, normalized.as_bytes()).is_empty() {
        return Err(format!(
            "二进制未嵌入期望的归一化 rustflags；请删除 {} 后重建",
            target_dir.display()
        ));
    }

    let mut roots = derived_roots(&repo)?;
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
        "build_parameters": "cargo +1.98.0 run --locked -p xtask -- build-measurement",
        "cargo_command": CARGO_COMMAND,
        "rustflags_normalized": normalized,
        "build_profile": "release;debug-assertions=off",
        "profile_pins": PROFILE_PINS.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>(),
        "compiler": "<rustup-toolchain>/bin/rustc",
        "binary": scan["binary"].clone(),
        "scan": scan,
    });
    write_receipt(&receipt_path, &receipt)?;
    println!(
        "build-measurement: pass; receipt written to {}",
        receipt_path.display()
    );
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
                receipt_path = Some(PathBuf::from(iter.next().ok_or("--receipt 缺少路径")?))
            }
            "--evidence-root" => {
                evidence_root = Some(PathBuf::from(
                    iter.next().ok_or("--evidence-root 缺少路径")?,
                ))
            }
            "--extra-root" => {
                extra_roots.push(PathBuf::from(iter.next().ok_or("--extra-root 缺少路径")?))
            }
            other if binary.is_none() && !other.starts_with("--") => {
                binary = Some(PathBuf::from(other))
            }
            other => return Err(format!("未知 check-binary-paths 参数：{other}")),
        }
    }
    let binary = binary.ok_or("缺少二进制路径")?;
    let repo = repo_root()?;
    let mut roots = derived_roots(&repo)?;
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
                .any(|(_, needle)| !find_all(&lowered, needle).is_empty())
        );
        let wide: Vec<u8> = "C:\\USERS\\mix"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert!(
            set.iter()
                .any(|(tag, needle)| tag.starts_with("utf-16le") && {
                    let mut padded = vec![0u8; 6];
                    padded.extend_from_slice(&wide);
                    !find_all(&ascii_lower(&padded), needle).is_empty()
                })
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
    }
}
