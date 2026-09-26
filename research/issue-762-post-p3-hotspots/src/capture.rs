use crate::{BASE, ORDER, Result, STAGES, io, need, string};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

fn now() -> Result<String> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_nanos()
        .to_string())
}

fn inputs(root: &Path) -> Result<Value> {
    let mut result = json!({});
    for scale in ["10k", "100k"] {
        let plan = root.join(format!("plans/{scale}-smoke.toml"));
        let data: toml::Value = toml::from_str(&fs::read_to_string(&plan)?)?;
        let data = serde_json::to_value(data)?;
        need(
            data["window"] == json!({"purpose":"probe","warm_up_ticks":0,"observation_ticks":256})
                && data["scale"] == scale,
            "plan window/scale",
        )?;
        // 保持平台原生路径键，兼容原始 Windows 输入身份；读取和验证不依赖路径所在主机。
        result[plan.strip_prefix(root)?.to_string_lossy().as_ref()] = json!(io::sha(&plan)?);
        let artifacts = root.join(format!("inputs/urban-{scale}"));
        let manifest = artifacts.join("manifest.toml");
        need(
            io::sha(&manifest)? == string(&data["manifest_digest"])?,
            "manifest identity",
        )?;
        result[manifest.strip_prefix(root)?.to_string_lossy().as_ref()] =
            json!(io::sha(&manifest)?);
        for (name, digest) in data["files"].as_object().ok_or("plan files")? {
            let path = io::safe_child(&artifacts, name)?;
            need(
                fs::metadata(&path)?.len() == digest["bytes"].as_u64().ok_or("input length")?
                    && io::sha(&path)? == string(&digest["sha256"])?,
                "input digest",
            )?;
            result[path.strip_prefix(root)?.to_string_lossy().as_ref()] = digest["sha256"].clone();
        }
    }
    Ok(result)
}

pub(crate) fn acquire(
    repo: &Path,
    mode: &str,
    output: &Path,
    input: &Path,
    parent: Option<&Path>,
) -> Result<()> {
    need(
        io::git(repo, &["status", "--porcelain"])?.is_empty(),
        "dirty research checkout",
    )?;
    let head = io::git(repo, &["rev-parse", "HEAD"])?;
    io::git(repo, &["merge-base", "--is-ancestor", BASE, &head])?;
    let input = input.canonicalize()?;
    io::ensure_new(output)?;
    fs::create_dir_all(output.parent().unwrap_or(Path::new(".")))?;
    fs::create_dir(output)?;
    let output = output.canonicalize()?;
    let detail = mode == "detail";
    let modes: &[&str] = if detail {
        &["detail"]
    } else {
        &["plain", "stages"]
    };
    let mut identity = json!({"base":BASE,"research_head":head,"research_tree":io::git(repo,&["rev-parse","HEAD^{tree}"])?,
        "workers":4,"ticks":256,"input_root":input,"inputs":inputs(&input)?,"started_unix_ns":now()?,
        "rustc":io::command(repo,"rustc",&["+1.98.0","-Vv"])?,"logical_cpus":std::thread::available_parallelism()?.get(),
        "processor":std::env::var("PROCESSOR_IDENTIFIER").ok(),"power":if cfg!(windows) {io::command(repo,"powercfg",&["/getactivescheme"]).ok()} else {None},
        "source_indexes":{},"binaries":{},"collector":"rust"});
    for m in modes {
        let recorded = io::read_json(&repo.join(format!("target/{m}-source.json")))?;
        need(
            recorded["base"] == BASE
                && recorded["mode"] == *m
                && io::source_index(&repo.join(format!("target/hotspot-{m}")))?
                    == recorded["source_files"],
            "export identity/drift",
        )?;
        let index_path = output.join(if detail {
            "source.json".to_owned()
        } else {
            format!("{m}-source.json")
        });
        io::write_new(&index_path, &recorded)?;
        identity["source_indexes"][*m] = json!(io::sha(&index_path)?);
        let binary = repo
            .join(format!(
                "target/hotspot-binaries/{m}{}",
                std::env::consts::EXE_SUFFIX
            ))
            .canonicalize()?;
        identity["binaries"][*m] = json!({"path":binary,"sha256":io::sha(&binary)?});
    }
    if let Some(parent) = parent {
        need(
            identity["inputs"] == io::read_json(parent)?["identity"]["inputs"],
            "detail input drift",
        )?;
        identity["parent_results_sha256"] = json!(io::sha(parent)?);
        identity["source_index_sha256"] = identity["source_indexes"]["detail"].clone();
        identity["binary_sha256"] = identity["binaries"]["detail"]["sha256"].clone();
        identity["stage_names"] = json!(STAGES);
    } else {
        need(
            identity["binaries"]["plain"]["sha256"] != identity["binaries"]["stages"]["sha256"],
            "same binary for different arms",
        )?;
        identity["order"] = json!(ORDER);
    }
    let identity_path = output.join("identity.json");
    io::write_new(&identity_path, &identity)?;
    let entries: Vec<(String, String)> = if detail {
        (1..=3)
            .map(|n| (format!("100k-{n}-detail"), "detail".to_owned()))
            .collect()
    } else {
        ["10k", "100k"]
            .into_iter()
            .flat_map(|s| {
                ORDER
                    .iter()
                    .enumerate()
                    .map(move |(n, m)| (format!("{s}-{}-{m}", n + 1), (*m).to_owned()))
            })
            .collect()
    };
    for (label, m) in entries {
        need(
            io::git(repo, &["rev-parse", "HEAD"])? == head
                && io::git(repo, &["status", "--porcelain"])?.is_empty(),
            "research changed",
        )?;
        let scale = label.split('-').next().ok_or("scale")?;
        let binary = PathBuf::from(string(&identity["binaries"][&m]["path"])?);
        need(
            io::sha(&binary)? == string(&identity["binaries"][&m]["sha256"])?,
            "binary drift",
        )?;
        let args = vec![
            "run".to_owned(),
            input
                .join(format!("inputs/urban-{scale}"))
                .to_string_lossy()
                .into_owned(),
            input
                .join(format!("plans/{scale}-smoke.toml"))
                .to_string_lossy()
                .into_owned(),
            output.join(&label).to_string_lossy().into_owned(),
            "--workers".to_owned(),
            "4".to_owned(),
        ];
        let command: Vec<_> = std::iter::once(binary.to_string_lossy().into_owned())
            .chain(args.iter().cloned())
            .collect();
        let started = now()?;
        let mut metadata = json!({"uuid":uuid::Uuid::new_v4().to_string(),"label":label,"scale":scale,"mode":m,"research_head":head,"started_unix_ns":started,"command":command});
        let meta_path = output.join(format!("{label}.process.json"));
        io::write_new(&meta_path, &metadata)?;
        let stdout = output.join(format!("{label}.stdout"));
        let stderr = output.join(format!("{label}.stderr"));
        let status = run_process(repo, &binary, &args, &stdout, &stderr);
        metadata["ended_unix_ns"] = json!(now()?);
        metadata["exit_code"] = json!(status.as_ref().ok().and_then(|s| s.code()));
        metadata["spawn_error"] = json!(status.as_ref().err().map(ToString::to_string));
        metadata["head_after"] = json!(io::git(repo, &["rev-parse", "HEAD"])?);
        metadata["status_after"] = json!(io::git(repo, &["status", "--porcelain"])?);
        metadata["binary_after"] = json!(io::sha(&binary)?);
        io::replace_owned(&meta_path, &metadata)?;
        need(
            status?.success()
                && metadata["head_after"] == head
                && metadata["status_after"] == ""
                && metadata["binary_after"] == identity["binaries"][&m]["sha256"],
            "run failed or drift",
        )?;
        println!("{label} complete");
    }
    need(inputs(&input)? == identity["inputs"], "input drift")?;
    for m in modes {
        let name = if detail {
            "source.json".to_owned()
        } else {
            format!("{m}-source.json")
        };
        need(
            io::source_index(&repo.join(format!("target/hotspot-{m}")))?
                == io::read_json(&output.join(name))?["source_files"],
            "source drift",
        )?;
    }
    identity["completed"] = json!(true);
    identity["ended_unix_ns"] = json!(now()?);
    io::replace_owned(&identity_path, &identity)?;
    Ok(())
}

fn run_process(
    repo: &Path,
    binary: &Path,
    args: &[String],
    stdout: &Path,
    stderr: &Path,
) -> Result<std::process::ExitStatus> {
    let stdout = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stdout)?;
    let stderr = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stderr)?;
    Ok(Command::new(binary)
        .args(args)
        .current_dir(repo)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .status()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failing_process_retains_streams_and_exit_status() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("lf762-process-{}-{}", std::process::id(), now()?));
        fs::create_dir(&root)?;
        let status = run_process(
            &root,
            Path::new("git"),
            &["not-a-real-subcommand-lf762".to_owned()],
            &root.join("stdout"),
            &root.join("stderr"),
        )?;
        assert!(!status.success());
        assert!(!fs::read(root.join("stderr"))?.is_empty());
        fs::remove_file(root.join("stdout"))?;
        fs::remove_file(root.join("stderr"))?;
        fs::remove_dir(root)?;
        Ok(())
    }
}
