//! #814：旧权威与同布局三 ISA 的受控整拍采集，原件只写外部目录。
#[allow(dead_code)]
mod cache_research;
mod chunk_build;
mod chunk_collector;
mod chunk_config;
mod chunk_native;
mod environment;
use cache_research::{Experiment, io, prepare};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    error::Error,
    fs::{self, OpenOptions},
    path::Path,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const BASE: &str = "cbfbb14a714d819cd5e608a1b759575d378f3d53";
const COLLECTOR_BIN: &str = "laneflow-columnar-runtime-research";
const EXPERIMENT: Experiment = Experiment {
    baseline: BASE,
    protocol: "columnar-motion-runtime-v1",
    count_p2: false,
};
const ARMS: [&str; 4] = ["base", "scalar", "avx2", "avx512"];

fn need(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(message.to_owned().into())
    }
}
fn now() -> Result<String> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_nanos()
        .to_string())
}
fn matrix() -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for scale in ["10k", "100k"] {
        for (group, order) in [
            [0, 1, 2, 3, 3, 2, 1, 0],
            [1, 2, 3, 0, 0, 3, 2, 1],
            [2, 3, 0, 1, 1, 0, 3, 2],
        ]
        .into_iter()
        .enumerate()
        {
            for (position, arm) in order.into_iter().enumerate() {
                let arm = ARMS[arm].to_owned();
                out.push((
                    format!("{scale}-{}-{}-{arm}", group + 1, position + 1),
                    scale.to_owned(),
                    arm,
                ));
            }
        }
    }
    out
}
fn inputs(root: &Path) -> Result<Value> {
    let mut index = serde_json::Map::new();
    for scale in ["10k", "100k"] {
        let plan_name = format!("plans/{scale}-smoke.toml");
        let plan = root.join(&plan_name);
        let parsed: toml::Value = toml::from_str(&fs::read_to_string(&plan)?)?;
        need(
            parsed["case"].as_str() == Some("MIXED-PEAK")
                && parsed["seed"].as_integer() == Some(544)
                && parsed["window"]["warm_up_ticks"].as_integer() == Some(0)
                && parsed["window"]["observation_ticks"].as_integer() == Some(256),
            "input plan",
        )?;
        index.insert(plan_name, json!(io::sha(&plan)?));
        let relative = format!("inputs/urban-{scale}/manifest.toml");
        let manifest = root.join(&relative);
        index.insert(relative, json!(io::sha(&manifest)?));
        for (name, digest) in parsed["files"].as_table().ok_or("input files")? {
            let relative = format!("inputs/urban-{scale}/{name}");
            let path = io::safe_child(root, &relative)?;
            need(
                digest["sha256"].as_str() == Some(io::sha(&path)?.as_str())
                    && digest["bytes"].as_integer() == Some(fs::metadata(&path)?.len() as i64),
                "input digest",
            )?;
            index.insert(relative, json!(io::sha(&path)?));
        }
    }
    Ok(json!(index))
}
fn export(root: &Path, arm: &str) -> Result<()> {
    need(["base", "candidate"].contains(&arm), "source arm")?;
    let repo = std::env::current_dir()?;
    let head = io::git(&repo, &["rev-parse", "HEAD"])?;
    need(
        io::git(&repo, &["status", "--porcelain"])?.is_empty(),
        "dirty source",
    )?;
    let commit = if arm == "base" { BASE } else { &head };
    io::git(&repo, &["merge-base", "--is-ancestor", commit, "HEAD"])?;
    chunk_build::ensure_outputs(root, arm, "plain")?;
    fs::create_dir_all(root)?;
    let source = root.join(format!("{arm}-plain-source"));
    let mut index = prepare::export_at(&repo, &source, "plain", commit)?;
    index["arm"] = json!(arm);
    index["mode"] = json!("plain");
    index["protocol"] = json!(EXPERIMENT.protocol);
    index["source_git_head"] = json!(commit);
    index["source_git_tree"] = json!(io::git(
        &repo,
        &["rev-parse", &format!("{commit}^{{tree}}")]
    )?);
    index["source_files"] = io::source_index(&source)?;
    index["build"] = chunk_build::build(root, &source, &index)?;
    io::write_new(&root.join(format!("{arm}-plain-source.json")), &index)
}
fn capture(builds: &Path, input: &Path, raw: &Path) -> Result<()> {
    let repo = std::env::current_dir()?;
    let collector = chunk_collector::verify_running()?;
    chunk_collector::verify_worktree(&collector)?;
    io::ensure_new(raw)?;
    fs::create_dir_all(raw)?;
    let head = io::git(&repo, &["rev-parse", "HEAD"])?;
    let mut identity = json!({"protocol":EXPERIMENT.protocol,"head":head,"collector":collector,
        "matrix":matrix(),"workers":4,"ticks":256,"started":now()?,"inputs":inputs(input)?,
        "sources":{},"binaries":{},"completed":false});
    for arm in ["base", "candidate"] {
        let source = io::read_json(&builds.join(format!("{arm}-plain-source.json")))?;
        need(
            source["source_files"]
                == io::source_index(&builds.join(format!("{arm}-plain-source")))?,
            "source drift",
        )?;
        let binary = builds
            .join(format!("{arm}-plain{}", std::env::consts::EXE_SUFFIX))
            .canonicalize()?;
        chunk_build::bind_capture(builds, raw, &source, &binary)?;
        identity["sources"][arm] = source;
        identity["binaries"][arm] = json!({"path":binary,"sha256":io::sha(&binary)?});
    }
    chunk_build::validate_pair(&identity)?;
    let identity_path = raw.join("identity.json");
    io::write_new(&identity_path, &identity)?;
    for (label, scale, arm) in matrix() {
        environment::observe(raw, &label, "before")?;
        need(
            io::git(&repo, &["rev-parse", "HEAD"])? == head
                && io::git(&repo, &["status", "--porcelain"])?.is_empty(),
            "coordinator drift",
        )?;
        let source_arm = if arm == "base" { "base" } else { "candidate" };
        let binary = Path::new(
            identity["binaries"][source_arm]["path"]
                .as_str()
                .ok_or("binary path")?,
        );
        need(
            identity["binaries"][source_arm]["sha256"] == io::sha(binary)?,
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
            raw.join(&label).to_string_lossy().into_owned(),
            "--workers".to_owned(),
            "4".to_owned(),
        ];
        let mut env: BTreeMap<String, String> = serde_json::from_value(
            identity["sources"][source_arm]["build"]["inherited_environment"].clone(),
        )?;
        // 清空运行环境后只恢复机器基础字段；唯一变量仅在安装时选择 ISA。
        env.retain(|key, _| chunk_native::infrastructure(key));
        env.insert(
            "LANEFLOW_MOTION_BACKEND".to_owned(),
            if arm == "base" { "auto" } else { &arm }.to_owned(),
        );
        let path = raw.join(format!("{label}.process.json"));
        let mut process = json!({"label":label,"scale":scale,"arm":arm,"args":args,"environment":env,
            "started":now()?,"binary_sha256":io::sha(binary)?,"source_arm":source_arm});
        io::write_new(&path, &process)?;
        let stdout = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(raw.join(format!("{label}.stdout")))?;
        let stderr = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(raw.join(format!("{label}.stderr")))?;
        let status = Command::new(binary)
            .args(&args)
            .current_dir(&repo)
            .env_clear()
            .envs(&env)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .status()?;
        process["exit_code"] = json!(status.code());
        process["ended"] = json!(now()?);
        process["head_after"] = json!(io::git(&repo, &["rev-parse", "HEAD"])?);
        process["status_after"] = json!(io::git(&repo, &["status", "--porcelain"])?);
        process["binary_after"] = json!(io::sha(binary)?);
        io::replace_owned(&path, &process)?;
        environment::observe(raw, &label, "after")?;
        need(
            status.success()
                && process["head_after"] == head
                && process["status_after"] == ""
                && process["binary_after"] == process["binary_sha256"],
            "run failure/drift; raw retained",
        )?;
        println!("{label} complete");
    }
    need(inputs(input)? == identity["inputs"], "input drift")?;
    for arm in ["base", "candidate"] {
        need(
            identity["sources"][arm]["source_files"]
                == io::source_index(&builds.join(format!("{arm}-plain-source")))?,
            "source drift after capture",
        )?;
    }
    identity["ended"] = json!(now()?);
    identity["completed"] = json!(true);
    io::replace_owned(&identity_path, &identity)
}
fn summary(samples: &[u64]) -> Value {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let percentile = |p: usize| sorted[(sorted.len() * p).div_ceil(100) - 1];
    json!({"samples":samples.len(),"mean_ms":samples.iter().map(|&v|v as f64).sum::<f64>()/samples.len() as f64/1e6,
        "p95_ms":percentile(95) as f64/1e6,"p99_ms":percentile(99) as f64/1e6})
}
fn analyze(raw: &Path) -> Result<Value> {
    let identity = io::read_json(&raw.join("identity.json"))?;
    need(
        identity["protocol"] == EXPERIMENT.protocol
            && identity["completed"] == true
            && identity["matrix"] == json!(matrix())
            && identity["workers"] == 4
            && identity["sources"]["base"]["source_git_head"] == BASE
            && identity["sources"]["candidate"]["source_git_head"] == identity["head"],
        "capture identity",
    )?;
    chunk_build::validate_pair(&identity)?;
    for arm in ["base", "candidate"] {
        chunk_build::verify_raw(
            raw,
            &identity["sources"][arm],
            &identity["binaries"][arm]["sha256"],
        )?;
    }
    let mut runs = Vec::new();
    let mut logical: BTreeMap<String, Value> = BTreeMap::new();
    for (label, scale, arm) in matrix() {
        environment::verify(raw, &label)?;
        let process = io::read_json(&raw.join(format!("{label}.process.json")))?;
        need(
            process["label"] == label
                && process["arm"] == arm
                && process["exit_code"] == 0
                && process["head_after"] == identity["head"]
                && process["status_after"] == ""
                && process["binary_after"] == process["binary_sha256"],
            "process identity",
        )?;
        let result = io::read_json(&raw.join(&label).join("result.json"))?;
        need(
            result["status"] == "probe-complete"
                && result["expected_ticks"] == 256
                && result["completed_ticks"] == 256
                && result["error"].is_null(),
            "incomplete run",
        )?;
        for (name, digest) in result["files"].as_object().ok_or("result files")? {
            let file = io::safe_child(&raw.join(&label), name)?;
            need(
                digest["sha256"] == io::sha(&file)? && digest["bytes"] == fs::metadata(file)?.len(),
                "run payload digest",
            )?;
        }
        let mut comparison = result.clone();
        comparison.as_object_mut().ok_or("result")?.remove("files");
        comparison
            .as_object_mut()
            .ok_or("result")?
            .remove("world_id");
        for name in [
            "commands.jsonl",
            "events.jsonl",
            "ticks.jsonl",
            "resolved-plan.toml",
        ] {
            comparison[name] = result["files"][name]["sha256"].clone();
        }
        if let Some(expected) = logical.get(&scale) {
            need(
                expected == &comparison,
                "state/decision/event/traffic disagreement",
            )?;
        } else {
            logical.insert(scale.clone(), comparison);
        }
        let rows: Vec<Value> = fs::read_to_string(raw.join(format!("{label}.stderr")))?
            .lines()
            .filter_map(|line| line.strip_prefix("LF762 "))
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        need(
            rows.len() == 256 && rows.iter().enumerate().all(|(i, row)| row["tick"] == i + 1),
            "tick trace",
        )?;
        let samples: Vec<u64> = rows
            .iter()
            .map(|row| row["step_ns"].as_u64().ok_or("step sample"))
            .collect::<std::result::Result<_, _>>()?;
        runs.push(json!({"label":label,"scale":scale,"arm":arm,"windows":{
            "enter":summary(&samples[..64]),"filtered":summary(&samples[64..]),"all":summary(&samples)},
            "initial_counts":result["initial_counts"],"final_counts":result["final_counts"]}));
    }
    Ok(
        json!({"protocol":EXPERIMENT.protocol,"status":"48-runs-verified","head":identity["head"],
        "baseline":BASE,"runs":runs,"raw_files":io::file_index(raw)?,"limits":"bounded 256 ticks; not 100k Active or final certification"}),
    )
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|v| v == "build-collector") && args.len() == 2 {
        return chunk_collector::build(Path::new(&args[1]));
    }
    if args.first().is_some_and(|v| v == "native-toolchain") && args.len() == 2 {
        return io::write_new(Path::new(&args[1]), &chunk_native::snapshot()?);
    }
    chunk_collector::verify_running()?;
    match args.first().map(String::as_str) {
        Some("verify-collector") if args.len() == 2 => chunk_collector::verify_root(Path::new(&args[1])),
        Some("prepare") if args.len() == 3 => export(Path::new(&args[2]), &args[1]),
        Some("run") if args.len() == 4 => capture(Path::new(&args[1]), Path::new(&args[2]), Path::new(&args[3])),
        Some("analyze" | "verify") if args.len() == 3 => {
            let raw = Path::new(&args[1]); let output = Path::new(&args[2]); let value = analyze(raw)?;
            if args[0] == "verify" { need(value == io::read_json(output)?, "published mismatch") }
            else { io::outside(raw, output)?; io::write_new(output, &value) }
        },
        _ => Err("build-collector <new-root> | prepare <base|candidate> <builds> | run <builds> <inputs> <new-raw> | analyze|verify <raw> <result>".into()),
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matrix_is_balanced_by_scale_group_arm_and_position() {
        let rows = matrix();
        assert_eq!(rows.len(), 48);
        for scale in ["10k", "100k"] {
            for arm in ARMS {
                assert_eq!(
                    rows.iter()
                        .filter(|(_, s, a)| s == scale && a == arm)
                        .count(),
                    6
                );
            }
        }
        for group in rows.chunks(8) {
            assert!(
                group
                    .iter()
                    .take(4)
                    .zip(group.iter().rev().take(4))
                    .all(|(a, b)| a.2 == b.2)
            );
        }
    }
}
