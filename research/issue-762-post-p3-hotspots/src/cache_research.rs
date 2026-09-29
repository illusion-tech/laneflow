//! #763 平衡整拍 A/B 与独立诊断；复用 #762 导出和身份校验，不改写历史证据。
#[allow(dead_code)]
#[path = "io.rs"]
pub(crate) mod io;
#[allow(dead_code)]
#[path = "prepare.rs"]
mod prepare;

use serde_json::{Value, json};
use std::{
    error::Error,
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const BASE: &str = "d6f5eee9e53f4a110dd6dcab64a4ab4458e5e426";
#[derive(Clone, Copy)]
pub(crate) struct Experiment {
    pub(crate) baseline: &'static str,
    pub(crate) protocol: &'static str,
    pub(crate) count_p2: bool,
}
const LEGACY: Experiment = Experiment {
    baseline: BASE,
    protocol: "p3-cache-abba-v1",
    count_p2: false,
};
const STAGES: [&str; 22] = [
    "Preflight",
    "Occupancy",
    "WaitingPrepare",
    "ConflictPrepare",
    "MotionLoop",
    "WaitingFinalize",
    "Signals",
    "ConflictFinalize",
    "WaitingOutputs",
    "Commit",
    "Frontier",
    "P4",
    "P3Discover",
    "P3Dispatch",
    "P3Consume",
    "P3Fused",
    "P3Tail",
    "Workload",
    "P2Independent",
    "MotionCalls",
    "GateCalculations",
    "CacheHits",
];

fn need(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(message.to_owned().into())
    }
}
fn string(v: &Value) -> Result<&str> {
    v.as_str().ok_or_else(|| "expected string".into())
}
fn now() -> Result<String> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_nanos()
        .to_string())
}
fn edit(root: &Path, relative: &str, old: &str, new: &str) -> Result<()> {
    let path = root.join(relative);
    let text = fs::read_to_string(&path)?.replace("\r\n", "\n");
    need(
        text.matches(old).count() == 1,
        &format!("anchor: {relative}: {old}"),
    )?;
    fs::write(path, text.replacen(old, new, 1))?;
    Ok(())
}

fn export(root: &Path, arm: &str, mode: &str, commit: &str) -> Result<()> {
    export_for(root, arm, mode, commit, LEGACY)
}

pub(crate) fn export_for(
    root: &Path,
    arm: &str,
    mode: &str,
    commit: &str,
    experiment: Experiment,
) -> Result<()> {
    need(
        ["base", "candidate"].contains(&arm) && ["plain", "detail"].contains(&mode),
        "arm/mode",
    )?;
    let repo = std::env::current_dir()?;
    let commit = io::git(&repo, &["rev-parse", &format!("{commit}^{{commit}}")])?;
    io::git(&repo, &["merge-base", "--is-ancestor", &commit, "HEAD"])?;
    need(
        arm != "base" || commit == experiment.baseline,
        "baseline drift",
    )?;
    fs::create_dir_all(root)?;
    let source = root.join(format!("{arm}-{mode}-source"));
    let mut index = prepare::export_at(&repo, &source, mode, &commit)?;
    if mode == "detail" {
        let profile = "crates/laneflow-runtime/src/kernel/performance_profile.rs";
        for file in [profile, "crates/laneflow-runtime/src/lib.rs"] {
            let path = source.join(file);
            let text = fs::read_to_string(&path)?.replace("; 18]", "; 22]");
            fs::write(path, text)?;
        }
        edit(
            &source,
            profile,
            "    P3Tail,",
            "    P3Tail,\n    Workload,\n    P2Independent,",
        )?;
        edit(
            &source,
            profile,
            "    NANOS.set([0; 22]);",
            "    NANOS.set([0; 22]);\n    LOCAL_COUNTS.set([0; 3]);\n    for counter in &COUNTERS { counter.store(0, std::sync::atomic::Ordering::Relaxed); }",
        )?;
        edit(
            &source,
            profile,
            "    (NANOS.get(), CALLS.get())",
            "    flush_counts();\n    let mut calls = CALLS.get();\n    for (index, counter) in COUNTERS.iter().enumerate() { calls[19 + index] = counter.load(std::sync::atomic::Ordering::Relaxed); }\n    (NANOS.get(), calls)",
        )?;
        let mut text = fs::read_to_string(source.join(profile))?;
        text.push_str("\nstatic COUNTERS: [std::sync::atomic::AtomicU64; 3] = [const { std::sync::atomic::AtomicU64::new(0) }; 3];\nthread_local! { static LOCAL_COUNTS: Cell<[u64; 3]> = const { Cell::new([0; 3]) }; }\npub(crate) fn count(index: usize) { LOCAL_COUNTS.with(|cell| { let mut counts = cell.get(); counts[index] += 1; cell.set(counts); }); }\npub(crate) fn flush_counts() { let local = LOCAL_COUNTS.replace([0; 3]); for (counter, value) in COUNTERS.iter().zip(local) { if value != 0 { counter.fetch_add(value, std::sync::atomic::Ordering::Relaxed); } } }\n");
        fs::write(source.join(profile), text)?;
        edit(
            &source,
            "crates/laneflow-runtime/src/kernel/execution.rs",
            "    compute(view, start, chunk);",
            "    compute(view, start, chunk);\n    super::performance_profile::flush_counts();",
        )?;
        let lib = source.join("crates/laneflow-runtime/src/lib.rs");
        let mut text = fs::read_to_string(&lib)?;
        text.push_str("\n/// 研究专用；只在计时外报告私有暂存布局。\n#[doc(hidden)]\npub fn research_layout() -> [usize; 4] { [std::mem::size_of::<kernel::tick::MotionCacheEntry>(), std::mem::size_of::<kernel::tick::WaitingPreviewEntry>(), std::mem::size_of::<kernel::execution::DispatchSlot<kernel::tick::WaitingPreviewEntry>>(), std::mem::size_of::<kernel::execution::DispatchSlot<kernel::conflict_tick::CandidateReport>>()] }\n");
        fs::write(lib, text)?;
        let lib = source.join("crates/laneflow-runtime/src/lib.rs");
        let mut text = fs::read_to_string(&lib)?;
        text.push_str("\n/// 研究专用；四个缓存/分发缓冲的拍末容量字节，不是整个工作区或进程 RSS。\n#[doc(hidden)]\npub fn research_storage(world: &TrafficWorld) -> [u64; 5] { let workspace = &world.state.workspace; let layout = research_layout(); let bytes = workspace.motion_cache.capacity() * layout[0] + workspace.waiting_preview_slots.capacity() * layout[2] + workspace.conflict_inputs.capacity() * std::mem::size_of::<(VehicleHandle, u32, usize, VehicleState)>() + workspace.conflict_slots.capacity() * layout[3]; [workspace.motion_cache.capacity() as u64, workspace.waiting_preview_slots.capacity() as u64, workspace.conflict_inputs.capacity() as u64, workspace.conflict_slots.capacity() as u64, bytes as u64] }\n");
        fs::write(lib, text)?;
        edit(
            &source,
            "tools/laneflow-urban-harness/src/host.rs",
            "                let (stages, calls) = laneflow_runtime::research_take();",
            "                let (stages, calls) = laneflow_runtime::research_take();\n                if world.tick_index() == 1 { eprintln!(\"LF763_LAYOUT {:?}\", laneflow_runtime::research_layout()); }\n                eprintln!(\"LF763_STORAGE {:?}\", laneflow_runtime::research_storage(&world));",
        )?;
        edit(
            &source,
            "crates/laneflow-runtime/src/kernel/waiting.rs",
            "        match execution {\n            Some(resources @ crate::kernel::execution::ExecutionResources::Pool(_))",
            "        let p2_timer = super::performance_profile::begin(super::performance_profile::Stage::P2Independent);\n        match execution {\n            Some(resources @ crate::kernel::execution::ExecutionResources::Pool(_))",
        )?;
        edit(
            &source,
            "crates/laneflow-runtime/src/kernel/waiting.rs",
            "        #[cfg(test)]\n        let _assembly =",
            "        drop(p2_timer);\n        #[cfg(test)]\n        let _assembly =",
        )?;
        edit(
            &source,
            "crates/laneflow-runtime/src/kernel/tick.rs",
            "        #[cfg(test)]\n        MOTION_CALCULATIONS.set(MOTION_CALCULATIONS.get() + 1);",
            "        super::performance_profile::count(0);\n        #[cfg(test)]\n        MOTION_CALCULATIONS.set(MOTION_CALCULATIONS.get() + 1);",
        )?;
        let file = "crates/laneflow-runtime/src/kernel/conflict_tick.rs";
        let old = "    let Some(route) = read.compiled_route(state.route) else {\n        return true;\n    };";
        edit(
            &source,
            file,
            old,
            &format!("    super::performance_profile::count(1);\n{old}"),
        )?;
        if fs::read_to_string(source.join(file))?
            .contains("        .and_then(|entry| entry.gate_reachable)")
        {
            edit(
                &source,
                file,
                "        .and_then(|entry| entry.gate_reachable)",
                "        .and_then(|entry| entry.gate_reachable)\n        .inspect(|_| super::performance_profile::count(2))",
            )?;
        }
        if experiment.count_p2 {
            for relative in [profile, "crates/laneflow-runtime/src/lib.rs"] {
                let path = source.join(relative);
                let mut text = fs::read_to_string(&path)?.replace("; 22]", "; 23]");
                if relative == profile {
                    text = text.replace("[0; 3]", "[0; 4]").replace("; 3]", "; 4]");
                }
                fs::write(path, text)?;
            }
            edit(
                &source,
                "crates/laneflow-runtime/src/kernel/tick.rs",
                "        let preview = self\n            .preview_active_vehicle_with_waiting_stop(state, delta_s, None, Some(horizon))",
                "        super::performance_profile::count(3);\n        let preview = self\n            .preview_active_vehicle_with_waiting_stop(state, delta_s, None, Some(horizon))",
            )?;
        }
        index["source_files"] = io::source_index(&source)?;
    }
    index["arm"] = json!(arm);
    index["mode"] = json!(mode);
    io::write_new(&root.join(format!("{arm}-{mode}-source.json")), &index)
}

fn inputs(root: &Path) -> Result<Value> {
    let mut out = json!({});
    for scale in ["10k", "100k"] {
        let plan = root.join(format!("plans/{scale}-smoke.toml"));
        let data: toml::Value = toml::from_str(&fs::read_to_string(&plan)?)?;
        let data = serde_json::to_value(data)?;
        need(
            data["scale"] == scale
                && data["window"]
                    == json!({"purpose":"probe","warm_up_ticks":0,"observation_ticks":256}),
            "plan contract",
        )?;
        out[io::slash(plan.strip_prefix(root)?)] = json!(io::sha(&plan)?);
        let artifacts = root.join(format!("inputs/urban-{scale}"));
        let manifest = artifacts.join("manifest.toml");
        need(
            io::sha(&manifest)? == string(&data["manifest_digest"])?,
            "manifest digest",
        )?;
        out[io::slash(manifest.strip_prefix(root)?)] = json!(io::sha(&manifest)?);
        for (name, digest) in data["files"].as_object().ok_or("plan files")? {
            let file = io::safe_child(&artifacts, name)?;
            need(
                fs::metadata(&file)?.len() == digest["bytes"].as_u64().ok_or("bytes")?
                    && io::sha(&file)? == string(&digest["sha256"])?,
                "input digest",
            )?;
            out[io::slash(file.strip_prefix(root)?)] = digest["sha256"].clone();
        }
    }
    Ok(out)
}

fn labels(mode: &str) -> Result<Vec<(String, String, String)>> {
    need(["plain", "detail"].contains(&mode), "mode")?;
    let order = if mode == "plain" {
        vec!["base", "candidate", "candidate", "base"]
    } else {
        vec!["base", "candidate"]
    };
    Ok(["10k", "100k"]
        .into_iter()
        .flat_map(|scale| {
            let order = order.clone();
            (1..=3).flat_map(move |group| {
                order
                    .clone()
                    .into_iter()
                    .enumerate()
                    .map(move |(position, arm)| {
                        (
                            format!("{scale}-{group}-{}-{arm}-{mode}", position + 1),
                            scale.to_owned(),
                            arm.to_owned(),
                        )
                    })
            })
        })
        .collect())
}

fn labels_for(mode: &str, experiment: Experiment) -> Result<Vec<(String, String, String)>> {
    let mut rows = labels(mode)?;
    if experiment.count_p2 && mode == "plain" {
        for (index, (label, _, arm)) in rows.iter_mut().enumerate() {
            if (index % 12) / 4 == 1 {
                let next = if arm == "base" { "candidate" } else { "base" };
                *label = label.replace(arm.as_str(), next);
                *arm = next.to_owned();
            }
        }
    }
    Ok(rows)
}

fn capture(mode: &str, root: &Path, input: &Path, raw: &Path) -> Result<()> {
    capture_for(mode, root, input, raw, LEGACY)
}

pub(crate) fn capture_for(
    mode: &str,
    root: &Path,
    input: &Path,
    raw: &Path,
    experiment: Experiment,
) -> Result<()> {
    let repo = std::env::current_dir()?;
    let head = io::git(&repo, &["rev-parse", "HEAD"])?;
    need(
        io::git(&repo, &["status", "--porcelain"])?.is_empty(),
        "dirty collector",
    )?;
    io::ensure_new(raw)?;
    fs::create_dir_all(raw.parent().unwrap_or(Path::new(".")))?;
    fs::create_dir(raw)?;
    let raw = raw.canonicalize()?;
    let input = input.canonicalize()?;
    let mut identity = json!({"protocol":experiment.protocol,"mode":mode,"head":head,"tree":io::git(&repo,&["rev-parse","HEAD^{tree}"] )?,"inputs":inputs(&input)?,"started":now()?,"workers":4,"ticks":256,"rustc":io::command(&repo,"rustc",&["+1.98.0","-Vv"] )?,"sources":{},"binaries":{}});
    for arm in ["base", "candidate"] {
        let source = io::read_json(&root.join(format!("{arm}-{mode}-source.json")))?;
        need(
            source["arm"] == arm
                && source["mode"] == mode
                && source["source_files"]
                    == io::source_index(&root.join(format!("{arm}-{mode}-source")))?,
            "source identity",
        )?;
        io::git(
            &repo,
            &[
                "merge-base",
                "--is-ancestor",
                string(&source["base"])?,
                &head,
            ],
        )?;
        let path = root
            .join(format!("{arm}-{mode}{}", std::env::consts::EXE_SUFFIX))
            .canonicalize()?;
        identity["sources"][arm] = source;
        identity["binaries"][arm] = json!({"path":path,"sha256":io::sha(&path)?});
    }
    need(
        identity["sources"]["base"]["base"] == experiment.baseline
            && identity["binaries"]["base"]["sha256"]
                != identity["binaries"]["candidate"]["sha256"],
        "arms identity",
    )?;
    let identity_path = raw.join("identity.json");
    io::write_new(&identity_path, &identity)?;
    for (label, scale, arm) in labels_for(mode, experiment)? {
        need(
            io::git(&repo, &["rev-parse", "HEAD"])? == head
                && io::git(&repo, &["status", "--porcelain"])?.is_empty(),
            "collector drift",
        )?;
        let binary = Path::new(string(&identity["binaries"][&arm]["path"])?);
        need(
            io::sha(binary)? == string(&identity["binaries"][&arm]["sha256"])?,
            "binary drift",
        )?;
        let args = [
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
        let command: Vec<_> = std::iter::once(binary.to_string_lossy().into_owned())
            .chain(args.iter().cloned())
            .collect();
        let path = raw.join(format!("{label}.process.json"));
        let mut meta = json!({"label":label,"scale":scale,"arm":arm,"command":command,"head":head,"started":now()?,"uuid":uuid::Uuid::new_v4().to_string()});
        io::write_new(&path, &meta)?;
        let stdout = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(raw.join(format!("{label}.stdout")))?;
        let stderr = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(raw.join(format!("{label}.stderr")))?;
        let status = Command::new(binary)
            .args(&args)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .status()?;
        meta["exit_code"] = json!(status.code());
        meta["ended"] = json!(now()?);
        meta["head_after"] = json!(io::git(&repo, &["rev-parse", "HEAD"])?);
        meta["status_after"] = json!(io::git(&repo, &["status", "--porcelain"])?);
        meta["binary_after"] = json!(io::sha(binary)?);
        io::replace_owned(&path, &meta)?;
        need(
            status.success()
                && meta["head_after"] == head
                && meta["status_after"] == ""
                && meta["binary_after"] == identity["binaries"][&arm]["sha256"],
            "run failure/drift",
        )?;
        println!("{label} complete");
    }
    need(inputs(&input)? == identity["inputs"], "input drift")?;
    for arm in ["base", "candidate"] {
        need(
            io::source_index(&root.join(format!("{arm}-{mode}-source")))?
                == identity["sources"][arm]["source_files"],
            "source drift",
        )?;
    }
    identity["completed"] = json!(true);
    identity["ended"] = json!(now()?);
    io::replace_owned(&identity_path, &identity)
}

fn stats(mut values: Vec<u64>) -> Value {
    values.sort_unstable();
    json!({"mean_ms":values.iter().map(|v|u128::from(*v)).sum::<u128>() as f64 / values.len() as f64 / 1e6,"p95_ms":values[(values.len()*95).div_ceil(100)-1] as f64 / 1e6})
}

#[cfg(test)]
fn validate_rows(rows: &[Value], ticks: &[Value], mode: &str) -> Result<()> {
    validate_rows_for(rows, ticks, mode, LEGACY)
}

fn validate_rows_for(
    rows: &[Value],
    ticks: &[Value],
    mode: &str,
    experiment: Experiment,
) -> Result<()> {
    need(rows.len() == 256 && ticks.len() == 256, "row count")?;
    let count = if mode == "plain" {
        12
    } else if experiment.count_p2 {
        23
    } else {
        22
    };
    for (i, row) in rows.iter().enumerate() {
        need(
            row["tick"] == i + 1
                && ticks[i]["tick"] == i + 1
                && ticks[i]["N_active"].as_u64().is_some()
                && row["step_ns"].as_u64().is_some_and(|n| n > 0),
            "row sequence/time",
        )?;
        let read = |name: &str| -> Result<Vec<u64>> {
            let values = row[name].as_array().ok_or("row array")?;
            need(values.len() == count, "row shape")?;
            values
                .iter()
                .map(|v| v.as_u64().ok_or_else(|| "row noninteger".into()))
                .collect()
        };
        let times = read("stages_ns")?;
        let calls = read("calls")?;
        if mode == "plain" {
            need(times.iter().chain(&calls).all(|v| *v == 0), "plain clocks")?;
        } else {
            let sum = |values: &[u64]| values.iter().map(|v| u128::from(*v)).sum::<u128>();
            need(
                sum(&times[..10]) <= u128::from(row["step_ns"].as_u64().unwrap())
                    && sum(&times[10..17]) <= u128::from(times[3])
                    && times[18] <= times[2],
                "clock nesting",
            )?;
            need(
                calls[..13].iter().all(|v| *v == 1)
                    && calls[16] == 1
                    && calls[18] == 1
                    && calls[13] == calls[14]
                    && calls[13] == u64::from(calls[17] >= 1_024)
                    && calls[15] == u64::from(calls[17] > 0 && calls[17] < 1_024),
                "clock calls/path",
            )?;
            if experiment.count_p2 {
                need(
                    times[22] == 0 && calls[22] <= calls[19],
                    "P2 preview counter",
                )?;
            }
        }
    }
    Ok(())
}

fn analyze(raw: &Path) -> Result<Value> {
    analyze_for(raw, LEGACY)
}

pub(crate) fn analyze_for(raw: &Path, experiment: Experiment) -> Result<Value> {
    let identity = io::read_json(&raw.join("identity.json"))?;
    let mode = string(&identity["mode"])?;
    need(
        identity["completed"] == true
            && identity["protocol"] == experiment.protocol
            && identity["workers"] == 4
            && identity["ticks"] == 256
            && identity["sources"]["base"]["base"] == experiment.baseline,
        "protocol completion",
    )?;
    let mut semantics = std::collections::BTreeMap::new();
    let mut ids = std::collections::BTreeSet::new();
    let mut runs = Vec::new();
    for (label, scale, arm) in labels_for(mode, experiment)? {
        let dir = raw.join(&label);
        let meta = io::read_json(&raw.join(format!("{label}.process.json")))?;
        let native = io::read_json(&dir.join("result.json"))?;
        let diagnostic = io::read_json(&dir.join("diagnostics.json"))?;
        need(
            meta["label"] == label
                && meta["scale"] == scale
                && meta["arm"] == arm
                && meta["exit_code"] == 0
                && meta["head"] == identity["head"]
                && meta["head_after"] == identity["head"]
                && meta["status_after"] == ""
                && meta["binary_after"] == identity["binaries"][&arm]["sha256"]
                && ids.insert(string(&meta["uuid"])?.to_owned()),
            "process identity",
        )?;
        need(
            native["status"] == "probe-complete"
                && native["completed_ticks"] == 256
                && native["error"].is_null()
                && native["scale"] == scale
                && native["case"] == "MIXED-PEAK",
            "native completion",
        )?;
        need(
            diagnostic["invocation"] == meta["command"]
                && diagnostic["workers"] == 4
                && diagnostic["verified_steps"] == 256
                && diagnostic["git_commit_at_run"] == identity["head"]
                && diagnostic["git_status_at_run"] == ""
                && diagnostic["binary"]["sha256"] == identity["binaries"][&arm]["sha256"],
            "native identity",
        )?;
        for (name, digest) in native["files"].as_object().ok_or("files")? {
            let path = io::safe_child(&dir, name)?;
            need(
                io::sha(&path)? == string(&digest["sha256"])?
                    && fs::metadata(path)?.len() == digest["bytes"].as_u64().ok_or("bytes")?,
                "native hash",
            )?;
        }
        let mut traffic = json!({});
        for name in ["ticks.jsonl", "commands.jsonl", "events.jsonl"] {
            traffic[name] = json!(io::sha(&dir.join(name))?);
        }
        let mut semantic = native.clone();
        semantic.as_object_mut().ok_or("result")?.remove("files");
        let semantic = json!({"result":semantic,"traffic":traffic});
        if let Some(previous) = semantics.insert(scale.clone(), semantic.clone()) {
            need(previous == semantic, "traffic/result changed")?;
        }
        let ticks: Vec<Value> = fs::read_to_string(dir.join("ticks.jsonl"))?
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let log = fs::read_to_string(raw.join(format!("{label}.stderr")))?;
        let rows: Vec<Value> = log
            .lines()
            .filter_map(|s| s.strip_prefix("LF762 "))
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        validate_rows_for(&rows, &ticks, mode, experiment)?;
        let layouts: Vec<Value> = log
            .lines()
            .filter_map(|s| s.strip_prefix("LF763_LAYOUT "))
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let storage: Vec<Value> = log
            .lines()
            .filter_map(|s| s.strip_prefix("LF763_STORAGE "))
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        need(
            (mode == "plain" && storage.is_empty())
                || (mode == "detail"
                    && storage.len() == 256
                    && storage.iter().all(|row| {
                        row.as_array()
                            .is_some_and(|a| a.len() == 5 && a.iter().all(|v| v.as_u64().is_some()))
                    })),
            "storage report",
        )?;
        let peak: Vec<u64> = (0..5)
            .map(|i| {
                storage
                    .iter()
                    .filter_map(|r| r[i].as_u64())
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        need(
            (mode == "plain" && layouts.is_empty())
                || (mode == "detail"
                    && layouts.len() == 1
                    && layouts[0].as_array().is_some_and(|a| {
                        a.len() == 4 && a.iter().all(|v| v.as_u64().is_some_and(|n| n > 0))
                    })),
            "layout report",
        )?;
        let mut windows = json!({});
        for (window, start, end) in [("all", 0, 256), ("entry", 0, 64), ("screen", 64, 256)] {
            let part = &rows[start..end];
            let mut data = json!({"step":stats(part.iter().map(|r|r["step_ns"].as_u64().unwrap()).collect()),"active_min":ticks[start..end].iter().map(|t|t["N_active"].as_u64().ok_or("active")).collect::<std::result::Result<Vec<_>,_>>()?.iter().min(),"active_max":ticks[start..end].iter().map(|t|t["N_active"].as_u64().ok_or("active")).collect::<std::result::Result<Vec<_>,_>>()?.iter().max()});
            if mode == "detail" {
                let names: Vec<_> = STAGES
                    .iter()
                    .copied()
                    .chain(experiment.count_p2.then_some("P2Previews"))
                    .collect();
                for (i, name) in names.iter().enumerate() {
                    data["stages"][*name] = stats(
                        part.iter()
                            .map(|r| r["stages_ns"][i].as_u64().unwrap())
                            .collect(),
                    );
                    data["calls"][*name] = json!(
                        part.iter()
                            .map(|r| r["calls"][i].as_u64().unwrap())
                            .sum::<u64>()
                    );
                }
            }
            windows[window] = data;
        }
        let mut ordered: Vec<_> = rows
            .iter()
            .map(|r| r["step_ns"].as_u64().unwrap())
            .collect();
        ordered.sort_unstable();
        need(
            diagnostic["step_ns_p95"] == ordered[(256_usize * 95).div_ceil(100) - 1],
            "native timing",
        )?;
        runs.push(json!({"label":label,"scale":scale,"arm":arm,"windows":windows,"traffic":traffic,"layout":layouts.first(),"peak_storage":peak,"initial_counts":native["initial_counts"],"final_counts":native["final_counts"]}));
    }
    Ok(json!({"identity":identity,"runs":runs,"files":io::file_index(raw)?}))
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let usage = "prepare <base|candidate> <plain|detail> <commit> <root> | run <plain|detail> <root> <inputs> <new-raw> | analyze|verify <raw> <results>";
    match args.first().map(String::as_str) {
        Some("prepare") if args.len() == 5 => {
            export(Path::new(&args[4]), &args[1], &args[2], &args[3])
        }
        Some("run") if args.len() == 5 => capture(
            &args[1],
            Path::new(&args[2]),
            Path::new(&args[3]),
            Path::new(&args[4]),
        ),
        Some("analyze" | "verify") if args.len() == 3 => {
            let raw = Path::new(&args[1]);
            let out = Path::new(&args[2]);
            let value = analyze(raw)?;
            if args[0] == "verify" {
                need(value == io::read_json(out)?, "published mismatch")?;
                println!(
                    "verified {} runs",
                    value["runs"].as_array().ok_or("runs")?.len()
                );
                Ok(())
            } else {
                io::outside(raw, out)?;
                io::write_new(out, &value)
            }
        }
        _ => Err(usage.into()),
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
    fn p2_protocol_rejects_legacy_shape_and_inconsistent_counter() {
        let experiment = Experiment {
            baseline: "baseline",
            protocol: "p2-scope-abba-v1",
            count_p2: true,
        };
        let matrix = labels_for("plain", experiment).unwrap();
        assert_eq!(
            matrix[4..8]
                .iter()
                .map(|r| r.2.as_str())
                .collect::<Vec<_>>(),
            ["candidate", "base", "base", "candidate"]
        );
        assert_eq!(
            matrix[16..20]
                .iter()
                .map(|r| r.2.as_str())
                .collect::<Vec<_>>(),
            ["candidate", "base", "base", "candidate"]
        );
        let ticks: Vec<_> = (1..=256)
            .map(|tick| json!({"tick":tick,"N_active":1}))
            .collect();
        let rows: Vec<_> = (1..=256)
            .map(|tick| {
                let mut calls = vec![0; 23];
                calls[..13].fill(1);
                calls[16] = 1;
                calls[18] = 1;
                calls[19] = 1;
                calls[22] = 1;
                json!({"tick":tick,"step_ns":100,"stages_ns":vec![0;23],"calls":calls})
            })
            .collect();
        validate_rows_for(&rows, &ticks, "detail", experiment).unwrap();
        assert!(validate_rows_for(&rows, &ticks, "detail", LEGACY).is_err());
        let mut bad = rows.clone();
        bad[0]["calls"][22] = json!(2);
        assert!(validate_rows_for(&bad, &ticks, "detail", experiment).is_err());
    }
    #[test]
    fn matrix_is_three_balanced_blocks_per_scale() {
        let rows = labels("plain").unwrap();
        assert_eq!(rows.len(), 24);
        for block in rows.as_chunks::<4>().0 {
            assert_eq!(
                block.iter().map(|r| r.2.as_str()).collect::<Vec<_>>(),
                ["base", "candidate", "candidate", "base"]
            );
        }
        assert_eq!(labels("detail").unwrap().len(), 12);
        assert!(labels("unknown").is_err());
    }
    #[test]
    fn stats_use_nearest_rank_and_no_pooled_run_percentile() {
        assert_eq!(
            stats(vec![1_000_000, 2_000_000, 3_000_000, 4_000_000]),
            json!({"mean_ms":2.5,"p95_ms":4.0})
        );
    }

    #[test]
    fn corrupt_rows_fail_closed_without_panicking() {
        let ticks: Vec<_> = (1..=256)
            .map(|tick| json!({"tick":tick,"N_active":1}))
            .collect();
        let rows: Vec<_> = (1..=256)
            .map(
                |tick| json!({"tick":tick,"step_ns":100,"stages_ns":vec![0;12],"calls":vec![0;12]}),
            )
            .collect();
        validate_rows(&rows, &ticks, "plain").unwrap();
        assert!(validate_rows(&rows[..255], &ticks, "plain").is_err());
        for (field, value) in [
            ("tick", json!(0)),
            ("step_ns", json!(-1)),
            ("stages_ns", json!(["wrong"])),
            ("calls", json!(vec![1; 12])),
        ] {
            let mut corrupt = rows.clone();
            corrupt[0][field] = value;
            assert!(validate_rows(&corrupt, &ticks, "plain").is_err(), "{field}");
        }
    }
}
