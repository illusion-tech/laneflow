//! #777 独立构建清单与平衡 A/B；复用 #682 的环境采样和字节核验。
use super::*;

const CASES: [&str; 6] = [
    "compact",
    "capacity",
    "active",
    "parked",
    "conflict-small",
    "conflict-large",
];
fn read(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn write(path: &Path, value: &Value) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
fn clean() -> Result<String> {
    require(
        command("git", &["status", "--porcelain"])?.is_empty(),
        "source must be clean",
    )?;
    let head = command("git", &["rev-parse", "HEAD"])?;
    require(
        command("git", &["rev-parse", "@{upstream}"])? == head,
        "source must be pushed",
    )?;
    Ok(head)
}
fn build(arm: &str, out: &Path) -> Result<()> {
    require(["base", "candidate"].contains(&arm), "unknown arm")?;
    require(
        !out.exists(),
        "build output exists; preserve it and use a new directory",
    )?;
    let source = clean()?;
    fs::create_dir_all(out)?;
    let mut manifest = json!({"schema":777,"arm":arm,"source":source,
        "tree":command("git", &["rev-parse","HEAD^{tree}"])?,"lock":hash(Path::new("Cargo.lock"))?,
        "manifest":hash(Path::new("crates/laneflow-runtime/Cargo.toml"))?,"rustc":command("rustc", &["-Vv"])?,
        "profile":"release","features":"placement-fixtures","completed":false});
    write(&out.join("build.json"), &manifest)?;
    for mode in ["wall", "diagnostic"] {
        let mut args = vec![
            "test",
            "-p",
            "laneflow-runtime",
            "--release",
            "--features",
            "placement-fixtures",
            "--locked",
            "--no-run",
            "-j",
            "2",
            "--message-format=json",
        ];
        if mode == "wall" {
            args.extend(["--test", "eligibility_commit_evidence"]);
        } else {
            args.push("--lib");
        }
        println!("building {arm}/{mode}");
        let output = Command::new("cargo")
            .args(&args)
            .env("CARGO_INCREMENTAL", "0")
            .output()?;
        fs::write(out.join(format!("{mode}-build.jsonl")), &output.stdout)?;
        fs::write(out.join(format!("{mode}-build.stderr")), &output.stderr)?;
        require(
            output.status.success(),
            "build failed; inspect saved cargo stderr",
        )?;
        let target = if mode == "wall" {
            "eligibility_commit_evidence"
        } else {
            "laneflow_runtime"
        };
        let exe = String::from_utf8(output.stdout)?
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find_map(|v| {
                (v["reason"] == "compiler-artifact" && v["target"]["name"] == target)
                    .then(|| v["executable"].as_str().map(str::to_owned))
                    .flatten()
            })
            .ok_or("missing test executable")?;
        let path = out.join(format!("laneflow-eligibility-{mode}.exe"));
        fs::copy(exe, &path)?;
        manifest[mode] = json!({"path":path.canonicalize()?.to_string_lossy(),"sha256":hash(&path)?,"args":args});
    }
    require(clean()? == source, "source changed during build")?;
    manifest["completed"] = json!(true);
    write(&out.join("build.json"), &manifest)?;
    println!("frozen {}", out.join("build.json").display());
    Ok(())
}
fn manifest(path: &Path) -> Result<Value> {
    let m = read(path)?;
    require(
        m["schema"] == 777
            && m["completed"] == true
            && ["base", "candidate"].contains(&m["arm"].as_str().ok_or("arm")?),
        "invalid build manifest",
    )?;
    let source = m["source"].as_str().ok_or("source")?;
    command("git", &["merge-base", "--is-ancestor", source, "HEAD"])?;
    require(
        command("git", &["rev-parse", &format!("{source}^{{tree}}")])? == m["tree"],
        "build tree mismatch",
    )?;
    Ok(m)
}
fn order(round: u64) -> [&'static str; 4] {
    if round == 1 {
        ["candidate", "base", "base", "candidate"]
    } else {
        ["base", "candidate", "candidate", "base"]
    }
}
fn identity(m: &Value) -> Result<String> {
    let arm = m["arm"].as_str().ok_or("arm")?;
    let mode = m["mode"].as_str().ok_or("mode")?;
    let case = m["case"].as_str().ok_or("case")?;
    let round = m["round"].as_u64().ok_or("round")?;
    let leg = m["leg"].as_u64().ok_or("leg")?;
    require(
        CASES.contains(&case) && round < 3 && ["base", "candidate"].contains(&arm),
        "invalid matrix case",
    )?;
    require(
        (mode == "wall" && leg < 4 && order(round)[leg as usize] == arm)
            || (mode == "diagnostic" && leg == 0),
        "invalid balanced leg",
    )?;
    Ok(format!("{case}-{round}-{mode}-{leg}-{arm}"))
}
fn metas(dir: &Path) -> Result<Vec<(PathBuf, Value)>> {
    let mut result = Vec::new();
    for entry in fs::read_dir(dir)? {
        let p = entry?.path();
        if p.extension().is_some_and(|v| v == "json")
            && p.file_name().unwrap().to_string_lossy().starts_with("run-")
        {
            result.push((p.clone(), read(&p)?));
        }
    }
    Ok(result)
}
fn capture(
    build_path: &Path,
    mode: &str,
    case: &str,
    round: usize,
    leg: usize,
    out: &Path,
) -> Result<()> {
    let head = clean()?;
    let b = manifest(build_path)?;
    let baseline = read(&out.join("baseline.json"))?;
    let warning = validate_baseline(&baseline)?;
    let mut m = json!({"schema":777,"arm":b["arm"],"mode":mode,"case":case,"round":round,"leg":leg,
        "build_sha256":hash(build_path)?,"baseline_sha256":hash(&out.join("baseline.json"))?,"controller_source":head,"accepted":false});
    let key = identity(&m)?;
    let existing: Vec<_> = metas(out)?
        .into_iter()
        .filter(|(_, v)| v["accepted"] == true && identity(v).ok().as_deref() == Some(&key))
        .collect();
    require(existing.len() <= 1, "duplicate accepted leg")?;
    if let Some((_, previous)) = existing.first() {
        require(
            previous["build_sha256"] == m["build_sha256"]
                && previous["baseline_sha256"] == m["baseline_sha256"],
            "resume uses different inputs",
        )?;
        return Ok(());
    }
    let exe = Path::new(b[mode]["path"].as_str().ok_or("binary path")?);
    require(hash(exe)? == b[mode]["sha256"], "binary modified")?;
    let stem = format!("run-{key}-{}", uuid::Uuid::new_v4());
    let meta_path = out.join(format!("{stem}.json"));
    let log = out.join(format!("{stem}.log"));
    let stderr = out.join(format!("{stem}.stderr"));
    m["started"] = json!(now());
    m["pre"] = idle(warning)?;
    write(&meta_path, &m)?;
    require(
        m["pre"]["no_competitors"] == true,
        "competing process; capture not started",
    )?;
    let filter = format!("eligibility_commit_{mode}");
    let mut child = Command::new(exe)
        .args([&filter, "--ignored", "--nocapture", "--test-threads=1"])
        .env("LANEFLOW_ELIGIBILITY_CASE", case)
        .stdout(Stdio::from(fs::File::create(&log)?))
        .stderr(Stdio::from(fs::File::create(&stderr)?))
        .spawn()?;
    let mut interference = Vec::new();
    let status = loop {
        match processes(Some(child.id())) {
            Ok(found) => interference.extend(found),
            Err(error) => {
                child.kill()?;
                child.wait()?;
                m["monitor_error"] = json!(error.to_string());
                write(&meta_path, &m)?;
                return Err(error);
            }
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_secs(1));
    };
    m["post"] = idle(warning)?;
    m["ended"] = json!(now());
    m["interference"] = json!(interference);
    m["exit_ok"] = json!(status.success());
    m["source_after"] = json!(command("git", &["rev-parse", "HEAD"])?);
    m["clean_after"] = json!(command("git", &["status", "--porcelain"])?.is_empty());
    for (field, path) in [("log", &log), ("stderr", &stderr)] {
        m[field] = json!(path.file_name().unwrap().to_string_lossy());
        m[format!("{field}_sha256")] = json!(hash(path)?);
    }
    m["accepted"] = json!(
        status.success()
            && interference.is_empty()
            && m["post"]["no_competitors"] == true
            && m["source_after"] == m["controller_source"]
            && m["clean_after"] == true
    );
    write(&meta_path, &m)?;
    require(
        m["accepted"] == true,
        &format!("capture rejected: {}", meta_path.display()),
    )?;
    println!(
        "accepted {key}; CPU warning={}",
        m["pre"]["cpu_warning"] == true || m["post"]["cpu_warning"] == true
    );
    Ok(())
}
fn measure(a: &Path, b: &Path, out: &Path) -> Result<()> {
    require(
        manifest(a)?["arm"] == "base" && manifest(b)?["arm"] == "candidate",
        "arm manifest mismatch",
    )?;
    fs::create_dir_all(out)?;
    for (name, path) in [("base-build.json", a), ("candidate-build.json", b)] {
        let target = out.join(name);
        if target.exists() {
            require(
                hash(&target)? == hash(path)?,
                "frozen build manifest changed",
            )?;
        } else {
            fs::copy(path, target)?;
        }
    }
    for round in 0..3 {
        let cases: Vec<_> = if round == 1 {
            CASES.iter().rev().copied().collect()
        } else {
            CASES.to_vec()
        };
        for case in cases {
            for (leg, arm) in order(round as u64).iter().enumerate() {
                capture(
                    if *arm == "base" { a } else { b },
                    "wall",
                    case,
                    round,
                    leg,
                    out,
                )?;
            }
            for arm in if round == 1 { [b, a] } else { [a, b] } {
                capture(arm, "diagnostic", case, round, 0, out)?;
            }
        }
    }
    Ok(())
}
fn parse_log(text: &str, mode: &str, case: &str) -> Result<Value> {
    require(
        text.contains("test result: ok. 1 passed;"),
        "test incomplete",
    )?;
    let windows = if case.starts_with("conflict-") { 8 } else { 1 };
    let mut ends = BTreeMap::new();
    let mut ticks = BTreeMap::new();
    let mut rows = BTreeMap::new();
    for line in text.lines().filter(|v| v.starts_with("elig-")) {
        let f = fields(line)?;
        require(
            f.get("case").map(String::as_str) == Some(case),
            "case mismatch",
        )?;
        let kind = line.split_whitespace().next().unwrap();
        match kind {
            "elig-end" => {
                let window: usize = f["window"].parse()?;
                require(
                    f["windows"].parse::<usize>()? == windows
                        && f["ticks"].parse::<usize>()? == 128 / windows
                        && f["digest"].len() == 64,
                    "invalid end",
                )?;
                require(ends.insert(window, f).is_none(), "duplicate end")?;
            }
            "elig-tick" => {
                let ns: u64 = f["ns"].parse()?;
                require(
                    ns > 0 && ticks.insert(f["tick"].parse::<usize>()?, ns).is_none(),
                    "duplicate/invalid tick",
                )?;
            }
            "elig-detail" | "elig-phase" | "elig-memory" | "elig-work" | "elig-table" => {
                let key = format!(
                    "{kind}:{}",
                    f.get("stage")
                        .or_else(|| f.get("window"))
                        .or_else(|| f.get("pattern"))
                        .map(String::as_str)
                        .unwrap_or("")
                );
                for (key, value) in &f {
                    if !["case", "stage", "pattern"].contains(&key.as_str()) {
                        let _: u64 = value.parse()?;
                    }
                }
                require(rows.insert(key, f).is_none(), "duplicate diagnostic")?;
            }
            _ => return Err("unknown log row".into()),
        }
    }
    require(ends.keys().copied().eq(0..windows), "missing end windows")?;
    if mode == "wall" {
        require(
            rows.is_empty() && ticks.keys().copied().eq(0..128),
            "incomplete wall ticks",
        )?;
        let samples: Vec<_> = ticks.values().copied().collect();
        let mut sorted = samples.clone();
        sorted.sort_unstable();
        Ok(
            json!({"ends":ends,"samples_ns":samples,"mean_ns":samples.iter().sum::<u64>() as f64/128.0,"p95_ns":sorted[121],"max_ns":sorted[127]}),
        )
    } else {
        require(
            ticks.is_empty()
                && rows.len() == 17 + windows + if case == "conflict-large" { 4 } else { 0 },
            "incomplete diagnostic",
        )?;
        for stage in [
            "clear_motion",
            "clear_eligibility",
            "copy",
            "scan",
            "eligibility_commit",
        ] {
            require(
                rows.contains_key(&format!("elig-detail:{stage}")),
                "missing detail",
            )?;
        }
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
                rows.contains_key(&format!("elig-phase:{stage}")),
                "missing phase",
            )?;
        }
        require(rows.contains_key("elig-work:"), "missing work")?;
        if case == "conflict-large" {
            for pattern in ["empty", "first", "last", "dense"] {
                let row = rows
                    .get(&format!("elig-table:{pattern}"))
                    .ok_or("missing table diagnostic")?;
                require(
                    row["slots"] == "100000" && row["calls"] == "128",
                    "table window mismatch",
                )?;
            }
        }
        if windows == 8 {
            require(
                rows["elig-work:"]["max_eligible"].parse::<u64>()? > 0,
                "resource fixture never eligible",
            )?;
        }
        for window in 0..windows {
            let row = rows
                .get(&format!("elig-memory:{window}"))
                .ok_or("missing memory")?;
            let sum: u64 = ["binding", "committed", "derived", "workspace", "admin"]
                .iter()
                .map(|key| row[*key].parse::<u64>())
                .collect::<std::result::Result<Vec<_>, _>>()?
                .iter()
                .sum();
            require(row["world"].parse::<u64>()? == sum, "unbalanced memory")?;
        }
        Ok(json!({"ends":ends,"rows":rows}))
    }
}
fn verify(dir: &Path, output: &Path) -> Result<()> {
    let baseline = read(&dir.join("baseline.json"))?;
    let warning = validate_baseline(&baseline)?;
    let baseline_hash = hash(&dir.join("baseline.json"))?;
    let mut builds = BTreeMap::new();
    for arm in ["base", "candidate"] {
        let path = dir.join(format!("{arm}-build.json"));
        let build = manifest(&path)?;
        require(build["arm"] == arm, "wrong build arm")?;
        builds.insert(arm, json!({"manifest":build,"sha256":hash(&path)?}));
    }
    for field in ["lock", "manifest", "rustc", "profile", "features"] {
        require(
            builds["base"]["manifest"][field] == builds["candidate"]["manifest"][field],
            "mixed build environment",
        )?;
    }
    let mut runs = BTreeMap::new();
    let mut rejected = Vec::new();
    for (_, m) in metas(dir)? {
        if m["accepted"] != true {
            rejected.push(m);
            continue;
        }
        let key = identity(&m)?;
        let arm = m["arm"].as_str().unwrap();
        let mode = m["mode"].as_str().unwrap();
        let case = m["case"].as_str().unwrap();
        require(
            m["schema"] == 777
                && m["build_sha256"] == builds[arm]["sha256"]
                && m["baseline_sha256"] == baseline_hash,
            "manifest binding mismatch",
        )?;
        require(
            environment_evidence(&m["pre"], warning)
                && environment_evidence(&m["post"], warning)
                && m["interference"] == json!([])
                && m["exit_ok"] == true
                && m["clean_after"] == true
                && m["source_after"] == m["controller_source"],
            "invalid accepted environment",
        )?;
        command(
            "git",
            &[
                "merge-base",
                "--is-ancestor",
                m["controller_source"].as_str().ok_or("controller source")?,
                "HEAD",
            ],
        )?;
        for field in ["log", "stderr"] {
            let name = m[field].as_str().ok_or("log name")?;
            require(
                Path::new(name).components().count() == 1
                    && m[format!("{field}_sha256")] == hash(&dir.join(name))?,
                "log digest mismatch",
            )?;
        }
        let parsed = parse_log(
            &fs::read_to_string(dir.join(m["log"].as_str().unwrap()))?,
            mode,
            case,
        )?;
        require(
            runs.insert(key, json!({"metadata":m,"result":parsed}))
                .is_none(),
            "duplicate accepted leg",
        )?;
    }
    require(runs.len() == 108, "incomplete balanced matrix")?;
    let mut summaries = BTreeMap::new();
    for case in CASES {
        let reference = &runs[&format!("{case}-0-wall-0-base")]["result"]["ends"];
        let mut blocks = Vec::new();
        for round in 0..3 {
            let mut wall = BTreeMap::<&str, Vec<f64>>::new();
            for (leg, arm) in order(round).iter().enumerate() {
                let r = &runs[&format!("{case}-{round}-wall-{leg}-{arm}")]["result"];
                require(&r["ends"] == reference, "state/input mismatch")?;
                wall.entry(arm)
                    .or_default()
                    .push(r["mean_ns"].as_f64().unwrap());
            }
            for arm in ["base", "candidate"] {
                require(
                    &runs[&format!("{case}-{round}-diagnostic-0-{arm}")]["result"]["ends"]
                        == reference,
                    "diagnostic state/input mismatch",
                )?;
            }
            let a = wall["base"].iter().sum::<f64>() / 2.0;
            let b = wall["candidate"].iter().sum::<f64>() / 2.0;
            blocks.push(json!({"round":round,"base_mean_ns":a,"candidate_mean_ns":b,"change_percent":100.0*(b/a-1.0)}));
        }
        summaries.insert(case, blocks);
    }
    write(
        output,
        &json!({"schema":777,"baseline":baseline,"builds":builds,"summaries":summaries,"runs":runs,"rejected":rejected}),
    )?;
    println!("verified 72 balanced wall and 36 diagnostic processes");
    Ok(())
}
pub(super) fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("build") if args.len()==3 => build(&args[1],Path::new(&args[2])),
        Some("calibrate") if args.len()==2 => calibrate(Path::new(&args[1])),
        Some("measure") if args.len()==4 => measure(Path::new(&args[1]),Path::new(&args[2]),Path::new(&args[3])),
        Some("verify") if args.len()==3 => verify(Path::new(&args[1]),Path::new(&args[2])),
        _ => Err("eligibility build ARM DIR | calibrate DIR | measure BASE_MANIFEST CANDIDATE_MANIFEST DIR | verify DIR JSON".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn balanced_identity_rejects_wrong_arm_and_leg() {
        let mut m = json!({"arm":"base","mode":"wall","case":"compact","round":0,"leg":0});
        assert!(identity(&m).is_ok());
        m["round"] = json!(1);
        assert!(identity(&m).is_err());
        m["arm"] = json!("candidate");
        assert!(identity(&m).is_ok());
        m["leg"] = json!(4);
        assert!(identity(&m).is_err());
    }
    #[test]
    fn wall_requires_complete_unique_samples_and_end() {
        let mut text = String::new();
        for tick in 0..128 {
            text.push_str(&format!("elig-tick case=compact tick={tick} ns=10\n"));
        }
        text.push_str(&format!("elig-end case=compact window=0 windows=1 ticks=128 input=a digest={}\ntest result: ok. 1 passed;\n", "a".repeat(64)));
        assert!(parse_log(&text, "wall", "compact").is_ok());
        assert!(parse_log(&text.replace("tick=127", "tick=126"), "wall", "compact").is_err());
        assert!(parse_log(&text.replace("windows=1", "windows=2"), "wall", "compact").is_err());
        assert!(parse_log(&text, "wall", "capacity").is_err());
        assert!(
            parse_log(
                &text.replace("test result: ok. 1 passed;", ""),
                "wall",
                "compact"
            )
            .is_err()
        );
    }
}
