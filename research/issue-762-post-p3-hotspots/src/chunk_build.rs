use crate::{EXPERIMENT, Result, chunk_collector, chunk_config, chunk_native, io, need};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const ARGS: [&str; 10] = [
    "+1.98.0",
    "build",
    "--release",
    "--locked",
    "--offline",
    "-p",
    "laneflow-urban-harness",
    "--bin",
    "laneflow-urban-harness",
    "--target-dir",
];
pub(crate) const ENV: [(&str, &str); 5] = [
    ("CARGO_INCREMENTAL", "0"),
    ("RUSTFLAGS", ""),
    ("CARGO_ENCODED_RUSTFLAGS", ""),
    ("RUSTC_WRAPPER", ""),
    ("RUSTC_WORKSPACE_WRAPPER", ""),
];
fn name(source: &Value) -> Result<String> {
    let arm = source["arm"].as_str().ok_or("build arm")?;
    let mode = source["mode"].as_str().ok_or("build mode")?;
    need(
        ["base", "layout", "candidate"].contains(&arm) && ["plain", "detail"].contains(&mode),
        "build arm/mode",
    )?;
    Ok(format!("{arm}-{mode}"))
}
pub(crate) fn value_sha(value: &Value) -> Result<String> {
    Ok(Sha256::digest(serde_json::to_vec(value)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
pub(crate) fn controlled_environment(
    inherited: &Value,
    native: &Value,
) -> Result<std::collections::BTreeMap<String, String>> {
    let mut values: std::collections::BTreeMap<String, String> =
        serde_json::from_value(inherited.clone())?;
    values.extend(ENV.into_iter().map(|(k, v)| (k.to_owned(), v.to_owned())));
    let native_values: std::collections::BTreeMap<String, String> =
        serde_json::from_value(native["controlled_environment"].clone())?;
    values.extend(native_values);
    Ok(values)
}
fn recipe(source: &Path, target: &Path, inherited: &Value, native: &Value) -> Result<Value> {
    let mut args: Vec<_> = ARGS
        .iter()
        .map(|s| (*s).to_owned())
        .chain(std::iter::once(target.to_string_lossy().into_owned()))
        .collect();
    // #814 的同布局 ISA 对照仅在安装时选择后端，不向普通 step 加入诊断代码。
    if EXPERIMENT.protocol == "columnar-motion-runtime-v1"
        && source
            .file_name()
            .is_some_and(|name| name == "candidate-plain-source")
    {
        args.extend(["--features".to_owned(), "motion-kernel-evidence".to_owned()]);
    }
    Ok(
        json!({"program":"cargo","args":args,"working_directory":source,"environment_cleared":true,
        "environment":controlled_environment(inherited,native)?}),
    )
}
fn build_environment(key: &str) -> bool {
    let normalized = key.to_ascii_uppercase();
    let key = normalized.as_str();
    // 离线构建来源只保存编译设置，不采集 registry token 或其它认证环境。
    !["TOKEN", "SECRET", "PASSWORD", "CREDENTIAL", "AUTH"]
        .iter()
        .any(|word| key.contains(word))
        && (matches!(
            key,
            "CARGO_HOME"
                | "CARGO_TARGET_DIR"
                | "RUSTUP_HOME"
                | "RUSTUP_TOOLCHAIN"
                | "RUSTC"
                | "RUSTDOC"
                | "RUST_LOG"
        ) || key.starts_with("CARGO_BUILD_")
            || key.starts_with("CARGO_PROFILE_")
            || (key.starts_with("CARGO_TARGET_")
                && (key.ends_with("_LINKER") || key.ends_with("_RUSTFLAGS"))))
}
pub(crate) fn inherited_environment() -> Value {
    let environment: std::collections::BTreeMap<_, _> = std::env::vars()
        .map(|(key, value)| {
            (
                if cfg!(windows) {
                    key.to_ascii_uppercase()
                } else {
                    key
                },
                value,
            )
        })
        .filter(|(key, _)| {
            (build_environment(key) || chunk_native::infrastructure(key))
                && !ENV.iter().any(|(k, _)| key == k)
        })
        .collect();
    json!(environment)
}
pub(crate) fn ensure_outputs(root: &Path, arm: &str, mode: &str) -> Result<()> {
    for suffix in [
        "-source.json",
        "-source",
        "-target",
        ".build.stdout",
        ".build.stderr",
        ".native.json",
        ".native-after.json",
        ".pdb",
        std::env::consts::EXE_SUFFIX,
    ] {
        io::ensure_new(&root.join(format!("{arm}-{mode}{suffix}")))?;
    }
    Ok(())
}
pub(crate) fn build(root: &Path, source: &Path, index: &Value) -> Result<Value> {
    let collector = chunk_collector::verify_running()?;
    let root = root.canonicalize()?;
    let source = source.canonicalize()?;
    let stem = name(index)?;
    let target = root.join(format!("{stem}-target"));
    io::ensure_new(&target)?;
    fs::create_dir(&target)?;
    need(!target.starts_with(&source), "target inside source")?;
    need(
        io::source_index(&source)? == index["source_files"],
        "source before build",
    )?;
    let rustc = io::command(&source, "rustc", &["+1.98.0", "-Vv"])?;
    let cargo = io::command(&source, "cargo", &["+1.98.0", "-V"])?;
    let logs = [
        format!("{stem}.build.stdout"),
        format!("{stem}.build.stderr"),
    ];
    let stdout = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(&logs[0]))?;
    let stderr = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(&logs[1]))?;
    let inherited = inherited_environment();
    let mut probe_environment: std::collections::BTreeMap<String, String> =
        serde_json::from_value(inherited.clone())?;
    let cleared_native_overrides: Vec<_> = std::env::vars_os()
        .map(|(key, _)| key.to_string_lossy().to_ascii_uppercase())
        .filter(|key| chunk_native::native_override(key))
        .collect();
    probe_environment.extend(ENV.into_iter().map(|(k, v)| (k.to_owned(), v.to_owned())));
    let native = chunk_native::probe(&root, &stem, ".native.json", &probe_environment)?;
    let configuration = chunk_config::snapshot(
        &source,
        &json!(controlled_environment(&inherited, &native)?),
    )?;
    let command = recipe(&source, &target, &inherited, &native)?;
    let args: Vec<String> = serde_json::from_value(command["args"].clone())?;
    let status = Command::new("cargo")
        .args(&args)
        .current_dir(&source)
        .env_clear()
        .envs(controlled_environment(&inherited, &native)?)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .status()?;
    need(
        status.success(),
        "controlled build failed; source and logs retained",
    )?;
    let native_after = chunk_native::probe(&root, &stem, ".native-after.json", &probe_environment)?;
    need(
        native == native_after,
        "native toolchain drift during build",
    )?;
    need(
        configuration
            == chunk_config::snapshot(
                &source,
                &json!(controlled_environment(&inherited, &native)?),
            )?,
        "Cargo configuration drift during build",
    )?;
    need(
        io::source_index(&source)? == index["source_files"],
        "source drift during build",
    )?;
    let compiled = target.join("release").join(format!(
        "laneflow-urban-harness{}",
        std::env::consts::EXE_SUFFIX
    ));
    let binary = root.join(format!("{stem}{}", std::env::consts::EXE_SUFFIX));
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&binary)?;
    std::io::copy(&mut fs::File::open(&compiled)?, &mut output)?;
    output.sync_all()?;
    let digest = io::sha(&compiled)?;
    need(io::sha(&binary)? == digest, "built/copy digest")?;
    let pdb = compiled.with_extension("pdb");
    if pdb.try_exists()? {
        fs::copy(pdb, binary.with_extension("pdb"))?;
    }
    let log_records: Vec<_> = logs
        .iter()
        .map(|file| -> Result<Value> {
            Ok(json!({"path":file,"sha256":io::sha(&root.join(file))?}))
        })
        .collect::<Result<_>>()?;
    Ok(
        json!({"schema":"p5-chunk-controlled-build-v3","protocol":EXPERIMENT.protocol,
        "exit_code":status.code(),
        "source_files_sha256":value_sha(&index["source_files"])?,"command":command,
        "inherited_environment":inherited,"native_toolchain":native,"cleared_native_overrides":cleared_native_overrides,"rustc":rustc,"cargo":cargo,
        "collector":collector,"cargo_configuration":configuration,
        "binary_sha256":digest,"binary_bytes":fs::metadata(&binary)?.len(),"logs":log_records}),
    )
}
fn verify_record(source: &Value, digest: &Value) -> Result<()> {
    let stem = name(source)?;
    let b = &source["build"];
    let cwd = PathBuf::from(
        b["command"]["working_directory"]
            .as_str()
            .ok_or("build cwd")?,
    );
    need(
        cwd.is_absolute()
            && cwd
                .file_name()
                .is_some_and(|n| n == format!("{stem}-source").as_str()),
        "build source directory",
    )?;
    let target = cwd
        .parent()
        .ok_or("build parent")?
        .join(format!("{stem}-target"));
    chunk_native::validate(&b["native_toolchain"])?;
    chunk_config::validate(
        &b["cargo_configuration"],
        &cwd,
        &b["command"]["environment"],
    )?;
    chunk_collector::validate(&b["collector"])?;
    need(
        b["schema"] == "p5-chunk-controlled-build-v3"
            && b["exit_code"] == 0
            && b["protocol"] == EXPERIMENT.protocol
            && b["command"]
                == recipe(
                    &cwd,
                    &target,
                    &b["inherited_environment"],
                    &b["native_toolchain"],
                )?
            && b["source_files_sha256"] == value_sha(&source["source_files"])?
            && b["binary_sha256"] == *digest
            && b["binary_bytes"].as_u64().is_some_and(|v| v > 0)
            && b["inherited_environment"].as_object().is_some_and(|vars| {
                vars.iter().all(|(k, v)| {
                    (build_environment(k) || chunk_native::infrastructure(k))
                        && !ENV.iter().any(|(key, _)| k == key)
                        && v.as_str().is_some()
                })
            })
            && b["rustc"]
                .as_str()
                .is_some_and(|s| s.starts_with("rustc 1.98.0 "))
            && b["cargo"]
                .as_str()
                .is_some_and(|s| s.starts_with("cargo 1.98.0 ")),
        "build attestation/source/binary mismatch",
    )?;
    let logs = b["logs"].as_array().ok_or("build logs")?;
    need(
        logs.len() == 2
            && logs[0]["path"] == format!("{stem}.build.stdout")
            && logs[1]["path"] == format!("{stem}.build.stderr"),
        "build log names",
    )
}
pub(crate) fn bind_capture(root: &Path, raw: &Path, source: &Value, binary: &Path) -> Result<()> {
    need(
        source["build"]["collector"] == chunk_collector::verify_running()?,
        "running collector differs from build collector",
    )?;
    verify_record(source, &json!(io::sha(binary)?))?;
    need(
        source["build"]["binary_bytes"] == fs::metadata(binary)?.len(),
        "built binary bytes",
    )?;
    for log in source["build"]["logs"].as_array().ok_or("build logs")? {
        let name = log["path"].as_str().ok_or("log path")?;
        let input = io::safe_child(root, name)?;
        need(log["sha256"] == io::sha(&input)?, "build log digest")?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(io::safe_child(raw, name)?)?;
        std::io::copy(&mut fs::File::open(input)?, &mut output)?;
        output.sync_all()?;
    }
    Ok(())
}
pub(crate) fn verify_raw(raw: &Path, source: &Value, digest: &Value) -> Result<()> {
    verify_record(source, digest)?;
    for log in source["build"]["logs"].as_array().ok_or("build logs")? {
        let file = io::safe_child(raw, log["path"].as_str().ok_or("log path")?)?;
        need(
            log["sha256"] == io::sha(&file)?,
            "archived build log digest",
        )?;
    }
    Ok(())
}

pub(crate) fn compatible(a: &Value, b: &Value) -> Result<()> {
    for field in [
        "inherited_environment",
        "rustc",
        "cargo",
        "native_toolchain",
        "collector",
    ] {
        need(
            !a[field].is_null() && a[field] == b[field],
            "paired build environment/toolchain mismatch",
        )?;
    }
    need(
        a["command"]["environment"].is_object()
            && a["command"]["environment"] == b["command"]["environment"],
        "paired build controlled environment mismatch",
    )
}
pub(crate) fn validate_pair(identity: &Value) -> Result<()> {
    compatible(
        &identity["sources"]["base"]["build"],
        &identity["sources"]["candidate"]["build"],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inherited_optimization_flags_and_toolchains_must_match() {
        let build = json!({"inherited_environment":{},"rustc":"rustc 1.98.0 host","cargo":"cargo 1.98.0",
            "collector":chunk_collector::fixture(&std::env::temp_dir()),
            "native_toolchain":chunk_native::fixture(&std::env::temp_dir()),
            "command":{"environment":{"CARGO_INCREMENTAL":"0"}}});
        let good = json!({"sources":{"base":{"build":build},"candidate":{"build":build}}});
        validate_pair(&good).unwrap();
        for key in [
            "CARGO_PROFILE_RELEASE_LTO",
            "CARGO_PROFILE_RELEASE_CODEGEN_UNITS",
            "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS",
        ] {
            let mut bad = good.clone();
            bad["sources"]["candidate"]["build"]["inherited_environment"][key] = json!("different");
            assert!(validate_pair(&bad).is_err(), "{key}");
        }
        for field in ["rustc", "cargo", "native_toolchain", "collector"] {
            let mut bad = good.clone();
            bad["sources"]["candidate"]["build"][field] = json!("other toolchain");
            assert!(validate_pair(&bad).is_err(), "{field}");
        }
    }
    #[test]
    fn build_provenance_excludes_authentication_environment() {
        for key in [
            "CARGO_HOME",
            "cargo_profile_release_lto",
            "RUST_LOG",
            "CARGO_PROFILE_RELEASE_LTO",
            "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER",
        ] {
            assert!(build_environment(key), "{key}");
        }
        for key in [
            "CARGO_REGISTRY_TOKEN",
            "CARGO_REGISTRIES_PRIVATE_TOKEN",
            "RUST_SECRET",
            "CARGO_BUILD_AUTH_TOKEN",
        ] {
            assert!(!build_environment(key), "{key}");
        }
    }
    #[test]
    fn stale_swapped_binary_source_and_recipe_fail_closed() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&root).unwrap();
        let binary = root.join(format!("base-plain{}", std::env::consts::EXE_SUFFIX));
        fs::write(&binary, b"fresh controlled binary").unwrap();
        let mut source =
            json!({"arm":"base","mode":"plain","source_files":{"Cargo.lock":"frozen"}});
        let native = chunk_native::fixture(&root);
        let environment = json!({"CARGO_HOME":root.join("cargo-home")});
        source["build"] = json!({"schema":"p5-chunk-controlled-build-v3","protocol":EXPERIMENT.protocol,
            "exit_code":0,
            "inherited_environment":environment,
            "native_toolchain":native,
            "collector":chunk_collector::fixture(&root),
            "cargo_configuration":chunk_config::fixture(&root.join("base-plain-source"),&environment),
            "command":recipe(&root.join("base-plain-source"),&root.join("base-plain-target"),&environment,&native).unwrap(),
            "source_files_sha256":value_sha(&source["source_files"]).unwrap(),"binary_sha256":io::sha(&binary).unwrap(),
            "binary_bytes":23,"rustc":"rustc 1.98.0 (fixture)","cargo":"cargo 1.98.0 (fixture)",
            "logs":[{"path":"base-plain.build.stdout"},{"path":"base-plain.build.stderr"}]});
        verify_record(&source, &json!(io::sha(&binary).unwrap())).unwrap();
        assert!(ensure_outputs(&root, "base", "plain").is_err());
        fs::write(&binary, b"old or swapped binary").unwrap();
        assert!(verify_record(&source, &json!(io::sha(&binary).unwrap())).is_err());
        let digest = source["build"]["binary_sha256"].clone();
        for field in ["source", "command", "environment", "toolchain", "missing"] {
            let mut bad = source.clone();
            match field {
                "source" => bad["source_files"]["Cargo.lock"] = json!("other"),
                "command" => bad["build"]["command"]["args"][2] = json!("--debug"),
                "environment" => {
                    bad["build"]["command"]["environment"]["RUSTFLAGS"] = json!("-C opt-level=0")
                }
                "toolchain" => bad["build"]["rustc"] = json!("rustc 1.97.0 (fixture)"),
                _ => bad["build"] = Value::Null,
            }
            assert!(verify_record(&bad, &digest).is_err(), "{field}");
        }
        fs::remove_file(binary).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
