//! 本切片只支持 Windows x64 MSVC。子进程清空环境后解析工具，避免继承 CC/CFLAGS 等覆盖。
use crate::{Result, io, need};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, process::Command};

const TARGET: &str = "x86_64-pc-windows-msvc";
const ROLES: [&str; 4] = ["compiler", "archiver", "assembler", "linker"];
const NAMES: [&str; 4] = ["cl.exe", "lib.exe", "ml64.exe", "link.exe"];

fn map(values: &[(std::ffi::OsString, std::ffi::OsString)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(k, v)| {
            (
                k.to_string_lossy().to_ascii_uppercase(),
                v.to_string_lossy().into_owned(),
            )
        })
        .collect()
}
pub(crate) fn native_override(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    let roots = [
        "CC",
        "CXX",
        "AR",
        "AS",
        "RANLIB",
        "CFLAGS",
        "CXXFLAGS",
        "CPPFLAGS",
        "ARFLAGS",
        "ASFLAGS",
        "RANLIBFLAGS",
        "CXXSTDLIB",
        "NVCC",
    ];
    key.starts_with("BLAKE3_")
        || key.starts_with("CC_")
        || matches!(
            key.as_str(),
            "CL" | "_CL_" | "LINK" | "_LINK_" | "CRATE_CC_NO_DEFAULTS" | "CROSS_COMPILE"
        )
        || roots.iter().any(|root| {
            key == *root
                || key.starts_with(&format!("{root}_"))
                || key == format!("HOST_{root}")
                || key == format!("TARGET_{root}")
        })
}
pub(crate) fn infrastructure(key: &str) -> bool {
    matches!(
        key,
        "PATH"
            | "SYSTEMROOT"
            | "SYSTEMDRIVE"
            | "WINDIR"
            | "COMSPEC"
            | "PATHEXT"
            | "USERPROFILE"
            | "HOME"
            | "TEMP"
            | "TMP"
    )
}
fn pinned(tools: &[Value]) -> Result<Value> {
    let mut env = tools[0]["environment"]
        .as_object()
        .ok_or("native compiler environment")?
        .clone();
    let directory = Path::new(tools[0]["path"].as_str().ok_or("native compiler path")?)
        .parent()
        .ok_or("native compiler parent")?;
    let path = env
        .get("PATH")
        .and_then(Value::as_str)
        .ok_or("native compiler PATH")?;
    // blake3 用 CC 的字面值识别 MSVC；绝对 CC 路径会改变汇编分支。用 cl.exe + 首位 PATH 锁定。
    env.insert(
        "PATH".into(),
        json!(format!("{};{path}", directory.display())),
    );
    env.insert("CC".into(), json!("cl.exe"));
    env.insert("AR".into(), tools[1]["path"].clone());
    env.insert(
        "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER".into(),
        tools[3]["path"].clone(),
    );
    Ok(Value::Object(env))
}
pub(crate) fn snapshot() -> Result<Value> {
    need(
        cfg!(all(windows, target_arch = "x86_64")),
        "controlled native build requires Windows x64 MSVC",
    )?;
    let mut build = cc::Build::new();
    build
        .target(TARGET)
        .host(TARGET)
        .opt_level(3)
        .debug(false)
        .cargo_metadata(false);
    let compiler = build.try_get_compiler()?;
    need(compiler.is_like_msvc(), "native compiler must be MSVC")?;
    let mut tools = Vec::new();
    for (role, name) in ROLES.into_iter().zip(NAMES) {
        let tool = if role == "compiler" {
            compiler.clone()
        } else {
            cc::windows_registry::find_tool(TARGET, name).ok_or("native MSVC tool missing")?
        };
        let path = tool.path().canonicalize()?;
        need(
            path.file_name()
                .is_some_and(|v| v.to_string_lossy().eq_ignore_ascii_case(name)),
            "native MSVC tool name",
        )?;
        let environment = map(tool.env());
        let output = Command::new(&path).envs(&environment).arg("/?").output()?;
        let banner = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let banner = banner
            .lines()
            .filter(|line| !line.trim().is_empty())
            .take(3)
            .collect::<Vec<_>>()
            .join("\n");
        need(!banner.is_empty(), "native tool version banner")?;
        tools.push(json!({"role":role,"path":path,"sha256":io::sha(&path)?,"bytes":std::fs::metadata(&path)?.len(),"version_banner":banner,
            "arguments":tool.args().iter().map(|v|v.to_string_lossy().into_owned()).collect::<Vec<_>>(),"environment":environment}));
    }
    // 这些工具的解析必须来自同一 MSVC 安装；实际编译不使用继承的 SDK/VC 选择覆盖。
    let parent = Path::new(tools[0]["path"].as_str().ok_or("native path")?).parent();
    need(
        tools
            .iter()
            .all(|t| Path::new(t["path"].as_str().unwrap_or("")).parent() == parent),
        "mixed MSVC installations",
    )?;
    let value = json!({"schema":"p5-chunk-native-msvc-v1","target":TARGET,"resolver":"cc 1.2.66","tools":tools,"controlled_environment":pinned(&tools)?});
    validate(&value)?;
    Ok(value)
}
pub(crate) fn validate(value: &Value) -> Result<()> {
    let tools = value["tools"].as_array().ok_or("native tool records")?;
    need(
        value["schema"] == "p5-chunk-native-msvc-v1"
            && value["target"] == TARGET
            && value["resolver"] == "cc 1.2.66"
            && tools.len() == 4,
        "native toolchain schema",
    )?;
    for ((tool, role), name) in tools.iter().zip(ROLES).zip(NAMES) {
        let path = Path::new(tool["path"].as_str().ok_or("native path")?);
        need(
            tool["role"] == role
                && path.is_absolute()
                && path
                    .file_name()
                    .is_some_and(|v| v.to_string_lossy().eq_ignore_ascii_case(name))
                && tool["sha256"].as_str().is_some_and(|s| {
                    s.len() == 64
                        && s.bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
                && tool["bytes"].as_u64().is_some_and(|v| v > 0)
                && tool["version_banner"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty())
                && tool["arguments"]
                    .as_array()
                    .is_some_and(|a| a.iter().all(Value::is_string))
                && tool["environment"]
                    .as_object()
                    .is_some_and(|v| v.iter().all(|(k, v)| !native_override(k) && v.is_string())),
            "native toolchain record",
        )?;
    }
    need(
        value["controlled_environment"] == pinned(tools)?,
        "native pinned environment mismatch",
    )
}
pub(crate) fn probe(
    root: &Path,
    stem: &str,
    suffix: &str,
    environment: &BTreeMap<String, String>,
) -> Result<Value> {
    let output = root.join(format!("{stem}{suffix}"));
    io::ensure_new(&output)?;
    let result = Command::new(std::env::current_exe()?)
        .arg("native-toolchain")
        .arg(&output)
        .env_clear()
        .envs(environment)
        .output()?;
    need(
        result.status.success(),
        &format!(
            "controlled native toolchain probe failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ),
    )?;
    io::read_json(&output)
}

#[cfg(test)]
pub(crate) fn fixture(root: &Path) -> Value {
    let tools: Vec<_> = ROLES.into_iter().zip(NAMES).map(|(role,name)| json!({"role":role,"path":root.join(name),"sha256":"a".repeat(64),"bytes":1,"version_banner":"MSVC fixture","arguments":[],"environment":{"PATH":root}})).collect();
    json!({"schema":"p5-chunk-native-msvc-v1","target":TARGET,"resolver":"cc 1.2.66","controlled_environment":pinned(&tools).unwrap(),"tools":tools})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compiler_overrides_are_outside_the_cleared_child_allowlist() {
        for key in [
            "CC",
            "CC_x86_64-pc-windows-msvc",
            "CC_x86_64_pc_windows_msvc",
            "HOST_CC",
            "TARGET_CC",
            "CFLAGS",
            "CFLAGS_x86_64_pc_windows_msvc",
            "HOST_CFLAGS",
            "CC_FORCE_DISABLE",
            "ARFLAGS",
            "BLAKE3_CI",
            "CL",
            "_CL_",
            "LINK",
            "CRATE_CC_NO_DEFAULTS",
        ] {
            assert!(native_override(key), "{key}");
            assert!(!infrastructure(&key.to_ascii_uppercase()), "{key}");
        }
        assert!(infrastructure("PATH"));
        assert!(!infrastructure("INCLUDE"));
        assert!(!infrastructure("LIB"));
        assert!(!infrastructure("VCTOOLSINSTALLDIR"));
    }
}
