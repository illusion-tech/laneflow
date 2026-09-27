//! #682 Windows 有限研究采集与独立核验。无后台服务，不修改其他进程。
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const CASES: [&str; 9] = [
    "compact",
    "vacant",
    "parked",
    "capacity",
    "active",
    "edges-small",
    "edges-large",
    "ring-10m",
    "ring-1m",
];
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn require(ok: bool, reason: &str) -> Result<()> {
    if ok { Ok(()) } else { Err(reason.into()) }
}
fn command(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program).args(args).output()?;
    require(
        out.status.success(),
        &format!("{program}: {}", String::from_utf8_lossy(&out.stderr)),
    )?;
    Ok(String::from_utf8(out.stdout)?.trim().into())
}
fn hash(path: &Path) -> Result<String> {
    Ok(Sha256::digest(fs::read(path)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn processes(exempt: Option<u32>) -> Result<Vec<String>> {
    let text = command("tasklist.exe", &["/FO", "CSV", "/NH"])?;
    let mut found = Vec::new();
    for line in text.lines() {
        let fields: Vec<_> = line.trim_matches('"').split("\",\"").collect();
        require(fields.len() >= 2, "tasklist format")?;
        let pid: u32 = fields[1].parse()?;
        if Some(pid) == exempt || pid == std::process::id() {
            continue;
        }
        let name = fields[0].to_ascii_lowercase();
        if ["cargo", "rustc", "link", "cl"]
            .iter()
            .any(|n| name == format!("{n}.exe"))
            || name.contains("harness")
            || name.starts_with("laneflow")
            || name.starts_with("runtime_profile")
            || name.starts_with("sparse_cost")
        {
            found.push(format!("{pid}:{}", fields[0]));
        }
    }
    Ok(found)
}
fn cpu() -> Result<Vec<f64>> {
    let text = command(
        "typeperf.exe",
        &[r"\Processor(_Total)\% Processor Time", "-sc", "2"],
    )?;
    let values: Vec<f64> = text
        .lines()
        .filter_map(|line| line.rsplit_once(',')?.1.trim_matches('"').parse().ok())
        .collect();
    require(
        values.len() == 2
            && values
                .iter()
                .all(|v| v.is_finite() && (0.0..=100.0).contains(v)),
        "CPU sample unavailable",
    )?;
    Ok(values)
}
fn idle() -> Result<Value> {
    let before = processes(None)?;
    let load = cpu()?;
    let after = processes(None)?;
    Ok(json!({"before":before, "cpu_percent":load, "after":after,
        "quiet": before.is_empty() && after.is_empty() && load.iter().all(|v| *v <= 20.0)}))
}
fn capture(exe: &Path, mode: &str, case: &str, round: usize, out: &Path) -> Result<()> {
    require(
        (CASES.contains(&case) && ["wall", "diagnostic"].contains(&mode))
            || (case == "resources" && mode == "resource" && round == 0),
        "unknown case/mode",
    )?;
    require(round < 3, "round outside matrix")?;
    require(
        command("git", &["status", "--porcelain"])?.is_empty(),
        "source must be clean",
    )?;
    let head = command("git", &["rev-parse", "HEAD"])?;
    require(
        command("git", &["rev-parse", "@{upstream}"])? == head,
        "source must be pushed to upstream",
    )?;
    fs::create_dir_all(out)?;
    let id = uuid::Uuid::new_v4().to_string();
    let stem = format!("{mode}-{case}-{round}-{id}");
    let meta_path = out.join(format!("{stem}.json"));
    let log_path = out.join(format!("{stem}.log"));
    let err_path = out.join(format!("{stem}.stderr"));
    let pre = idle()?;
    let mut meta = json!({"schema":1,"id":id,"mode":mode,"case":case,"round":round,"source":head,
        "tree":command("git", &["rev-parse","HEAD^{tree}"])?,
        "lock":hash(Path::new("Cargo.lock"))?, "manifest":hash(Path::new("crates/laneflow-runtime/Cargo.toml"))?,
        "binary":exe.canonicalize()?.to_string_lossy(),"binary_sha256":hash(exe)?,
        "rustc":command("rustc", &["-Vv"])?,"started":now(),"pre":pre,"accepted":false});
    fs::write(&meta_path, serde_json::to_vec_pretty(&meta)?)?;
    require(
        pre["quiet"] == true,
        &format!("同机负载干扰，未启动测量；记录 {}", meta_path.display()),
    )?;
    let filter = match mode {
        "wall" => "sparse_cost_wall",
        "resource" => "resource_sort_diagnostic",
        _ => "sparse_cost_diagnostic",
    };
    let mut child = Command::new(exe)
        .args([filter, "--ignored", "--nocapture", "--test-threads=1"])
        .env("LANEFLOW_SPARSE_CASE", case)
        .stdout(Stdio::from(fs::File::create(&log_path)?))
        .stderr(Stdio::from(fs::File::create(&err_path)?))
        .spawn()?;
    let mut interference = Vec::new();
    let status = loop {
        match processes(Some(child.id())) {
            Ok(found) => interference.extend(found),
            Err(error) => {
                child.kill()?;
                child.wait()?;
                meta["monitor_error"] = json!(error.to_string());
                fs::write(&meta_path, serde_json::to_vec_pretty(&meta)?)?;
                return Err(error);
            }
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_secs(1));
    };
    let post = idle()?;
    meta["ended"] = json!(now());
    meta["post"] = post.clone();
    meta["interference"] = json!(interference);
    meta["exit_ok"] = json!(status.success());
    meta["source_after"] = json!(command("git", &["rev-parse", "HEAD"])?);
    meta["clean_after"] = json!(command("git", &["status", "--porcelain"])?.is_empty());
    meta["log"] = json!(log_path.file_name().unwrap().to_string_lossy());
    meta["stderr"] = json!(err_path.file_name().unwrap().to_string_lossy());
    meta["log_sha256"] = json!(hash(&log_path)?);
    meta["stderr_sha256"] = json!(hash(&err_path)?);
    meta["accepted"] = json!(
        status.success()
            && interference.is_empty()
            && post["quiet"] == true
            && meta["source_after"] == meta["source"]
            && meta["clean_after"] == true
    );
    fs::write(&meta_path, serde_json::to_vec_pretty(&meta)?)?;
    require(
        meta["accepted"] == true,
        &format!("测量未接受，检查负载/退出记录 {}", meta_path.display()),
    )?;
    println!("accepted {mode} {case} {round}: {}", meta_path.display());
    Ok(())
}

fn fields(line: &str) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    for token in line.split_whitespace().skip(1) {
        let (key, value) = token.split_once('=').ok_or("invalid evidence field")?;
        require(
            result.insert(key.into(), value.into()).is_none(),
            "duplicate evidence field",
        )?;
    }
    Ok(result)
}
fn parse(text: &str, mode: &str, case: &str) -> Result<Value> {
    require(
        text.contains("test result: ok. 1 passed;"),
        "test did not finish exactly one case",
    )?;
    if mode == "resource" {
        let mut rows = BTreeMap::new();
        for line in text
            .lines()
            .filter(|line| line.starts_with("resource-sort "))
        {
            let f = fields(line)?;
            let round: usize = f["round"].parse()?;
            let stage: usize = f["stage"].parse()?;
            require(
                round < 3 && (2..5).contains(&stage) && f["steps"] == "16",
                "resource window",
            )?;
            require(
                ["waiting-membership-1", "conflict-reservation-retry-2"]
                    .contains(&f["scene"].as_str()),
                "resource scene",
            )?;
            for key in ["calls", "items", "ns"] {
                let _: u64 = f[key].parse()?;
            }
            let key = format!("{}-{round}-{stage}", f["scene"]);
            require(rows.insert(key, f).is_none(), "duplicate resource row")?;
        }
        require(rows.len() == 18, "incomplete resource rows")?;
        return Ok(json!({"rows":rows}));
    }
    let mut ticks = BTreeMap::new();
    let mut rows = BTreeMap::new();
    let mut end = None;
    for line in text.lines() {
        if !line.starts_with("sparse-") {
            continue;
        }
        let f = fields(line)?;
        require(
            f.get("case").map(String::as_str) == Some(case),
            "case mismatch",
        )?;
        let kind = line.split_whitespace().next().unwrap();
        match kind {
            "sparse-tick" => {
                let tick: usize = f["tick"].parse()?;
                let ns: u64 = f["ns"].parse()?;
                require(
                    ns > 0 && ticks.insert(tick, ns).is_none(),
                    "invalid/duplicate tick",
                )?;
            }
            "sparse-end" => {
                require(end.replace(f).is_none(), "duplicate end")?;
            }
            _ => {
                let key = format!(
                    "{kind}:{}",
                    f.get("stage").map(String::as_str).unwrap_or("")
                );
                require(rows.insert(key, f).is_none(), "duplicate diagnostic row")?;
            }
        }
    }
    let end = end.ok_or("missing end")?;
    require(
        end["steps"] == "128" && end["warm"] == "40",
        "window mismatch",
    )?;
    require(end["digest"].len() == 64, "invalid digest")?;
    if mode == "wall" {
        require(
            rows.is_empty() && ticks.keys().copied().eq(0..128),
            "incomplete wall ticks",
        )?;
        let mut samples: Vec<_> = ticks.values().copied().collect();
        samples.sort_unstable();
        Ok(
            json!({"end":end,"samples_ns":ticks.values().collect::<Vec<_>>(),"mean_ns":samples.iter().sum::<u64>() as f64/128.0,"p50_ns":samples[63],"p95_ns":samples[121],"p99_ns":samples[126],"max_ns":samples[127]}),
        )
    } else {
        require(
            ticks.is_empty() && rows.len() == 18,
            "incomplete diagnostic",
        )?;
        for stage in [
            "whole_step",
            "preflight",
            "occupancy",
            "waiting_prepare",
            "conflict_prepare",
            "motion_loop",
            "waiting_finalize",
            "signals",
            "conflict_finalize",
            "waiting_outputs",
            "commit",
        ] {
            require(
                rows.contains_key(&format!("sparse-stage:{stage}")),
                "missing phase",
            )?;
        }
        for stage in ["count", "layout", "fill", "sort_suffix"] {
            require(
                rows.contains_key(&format!("sparse-occupancy:{stage}")),
                "missing occupancy phase",
            )?;
        }
        for row in ["sparse-clear:", "sparse-memory:", "sparse-work:"] {
            require(rows.contains_key(row), "missing ledger")?;
        }
        Ok(json!({"end":end,"rows":rows}))
    }
}
fn verify(dir: &Path, output: &Path) -> Result<()> {
    let mut runs = Vec::new();
    let mut keys = BTreeMap::new();
    let mut rejected = Vec::new();
    for file in fs::read_dir(dir)? {
        let path = file?.path();
        if path.extension().is_none_or(|v| v != "json") {
            continue;
        }
        let meta: Value = serde_json::from_slice(&fs::read(&path)?)?;
        if meta["accepted"] != true {
            rejected.push(meta);
            continue;
        }
        let mode = meta["mode"].as_str().ok_or("mode")?;
        let case = meta["case"].as_str().ok_or("case")?;
        let round = meta["round"].as_u64().ok_or("round")?;
        require(
            (CASES.contains(&case) && round < 3 && ["wall", "diagnostic"].contains(&mode))
                || (case == "resources" && mode == "resource" && round == 0),
            "matrix identity",
        )?;
        require(
            keys.insert((case.to_string(), round, mode.to_string()), runs.len())
                .is_none(),
            "duplicate accepted run",
        )?;
        for (field, digest) in [("log", "log_sha256"), ("stderr", "stderr_sha256")] {
            let name = meta[field].as_str().ok_or("file field")?;
            require(
                Path::new(name).components().count() == 1,
                "non-local evidence path",
            )?;
            require(
                meta[digest] == hash(&dir.join(name))?,
                "file digest mismatch",
            )?;
        }
        require(
            quiet_evidence(&meta["pre"])
                && quiet_evidence(&meta["post"])
                && meta["interference"] == json!([])
                && meta["exit_ok"] == true
                && meta["clean_after"] == true
                && meta["source_after"] == meta["source"],
            "invalid accepted environment",
        )?;
        let log = fs::read_to_string(dir.join(meta["log"].as_str().unwrap()))?;
        runs.push(json!({"metadata":meta,"result":parse(&log,mode,case)?}));
    }
    require(runs.len() == CASES.len() * 6 + 1, "incomplete matrix")?;
    for case in CASES {
        let reference = &runs[keys[&(case.into(), 0, "wall".into())]];
        for round in 0..3 {
            for mode in ["wall", "diagnostic"] {
                let run = &runs[keys[&(case.into(), round, mode.into())]];
                require(
                    run["result"]["end"] == reference["result"]["end"],
                    "input/count/digest differs between rounds or binaries",
                )?;
                require(
                    run["metadata"]["source"] == runs[0]["metadata"]["source"]
                        && run["metadata"]["lock"] == runs[0]["metadata"]["lock"],
                    "mixed source",
                )?;
            }
        }
    }
    fs::write(
        output,
        serde_json::to_vec_pretty(&json!({"schema":1,"runs":runs,"rejected":rejected}))?,
    )?;
    println!(
        "verified 54 matrix runs and one resource run: {}",
        output.display()
    );
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("capture") if args.len() == 7 => capture(
            Path::new(&args[2]),
            &args[3],
            &args[4],
            args[5].parse()?,
            Path::new(&args[6]),
        ),
        Some("matrix") if args.len() == 5 => {
            let out = PathBuf::from(&args[4]);
            for round in 0..3 {
                let cases: Vec<_> = if round == 1 {
                    CASES.iter().rev().copied().collect()
                } else {
                    CASES.to_vec()
                };
                for case in cases {
                    for (mode, exe) in [("wall", &args[2]), ("diagnostic", &args[3])] {
                        capture(Path::new(exe), mode, case, round, &out)?;
                    }
                }
            }
            capture(Path::new(&args[3]), "resource", "resources", 0, &out)?;
            Ok(())
        }
        Some("verify") if args.len() == 4 => verify(Path::new(&args[2]), Path::new(&args[3])),
        _ => Err(
            "usage: capture EXE MODE CASE ROUND DIR | matrix WALL DIAGNOSTIC DIR | verify DIR JSON"
                .into(),
        ),
    }
}

fn quiet_evidence(value: &Value) -> bool {
    value["quiet"] == true
        && value["before"] == json!([])
        && value["after"] == json!([])
        && value["cpu_percent"].as_array().is_some_and(|a| {
            a.len() == 2
                && a.iter().all(|v| {
                    v.as_f64()
                        .is_some_and(|n| n.is_finite() && (0.0..=20.0).contains(&n))
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wall() -> String {
        let mut log = String::new();
        for tick in 0..128 {
            log.push_str(&format!("sparse-tick case=compact tick={tick} ns=10\n"));
        }
        log.push_str(&format!(
            "sparse-end case=compact steps=128 warm=40 digest={}\ntest result: ok. 1 passed;\n",
            "a".repeat(64)
        ));
        log
    }
    #[test]
    fn rejects_truncated_duplicate_and_wrong_case() {
        let log = wall();
        assert!(parse(&log, "wall", "compact").is_ok());
        assert!(
            parse(
                &log.replace("sparse-tick case=compact tick=127 ns=10\n", ""),
                "wall",
                "compact"
            )
            .is_err()
        );
        assert!(
            parse(
                &(log.clone() + "sparse-tick case=compact tick=0 ns=10\n"),
                "wall",
                "compact"
            )
            .is_err()
        );
        assert!(parse(&log, "wall", "vacant").is_err());
        assert!(
            parse(
                &log.replace("test result: ok. 1 passed;", ""),
                "wall",
                "compact"
            )
            .is_err()
        );
    }
}
