//! #768 当前主干 P2 成本诊断；只修改隔离导出树，保留历史研究协议。
#[allow(dead_code)]
#[path = "io.rs"]
pub(crate) mod io;
#[path = "p2_export.rs"]
mod p2_export;
#[allow(dead_code)]
#[path = "prepare.rs"]
pub(crate) mod prepare;

use serde_json::{Value, json};
use std::{
    error::Error,
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
#[derive(Clone, Copy)]
pub(crate) struct Protocol {
    pub(crate) baseline: &'static str,
    pub(crate) name: &'static str,
    pub(crate) stages: &'static [&'static str],
    pub(crate) validate: fn(&[u64], &[u64], u64) -> Result<()>,
}
const BASE: &str = "7bdf1f0ee4ae436ffc688903899ce9d16f89e41b";
const STAGES: [&str; 32] = [
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
    "P3Workload",
    "P2All",
    "P2Discover",
    "P2Slots",
    "P2Dispatch",
    "P2Consume",
    "P2Fused",
    "WaitingAssembly",
    "P2Entries",
    "P2NoGate",
    "P2Outside",
    "P2Preview",
    "P2Horizon",
    "P2GateCalc",
    "P2InputCount",
];
const LEGACY: Protocol = Protocol {
    baseline: BASE,
    name: "p2-cost-v1",
    stages: &STAGES,
    validate: validate_p2,
};

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

fn labels() -> Vec<(String, String, String)> {
    ["10k", "100k"]
        .into_iter()
        .flat_map(|scale| {
            ["plain", "detail", "detail", "plain", "plain", "detail"]
                .into_iter()
                .enumerate()
                .map(move |(i, mode)| {
                    (
                        format!("{scale}-{}-{mode}", i + 1),
                        scale.to_owned(),
                        mode.to_owned(),
                    )
                })
        })
        .collect()
}

fn inputs(root: &Path) -> Result<Value> {
    let mut out = json!({});
    for scale in ["10k", "100k"] {
        let plan = root.join(format!("plans/{scale}-smoke.toml"));
        let data: Value =
            serde_json::to_value(toml::from_str::<toml::Value>(&fs::read_to_string(&plan)?)?)?;
        validate_plan(&data, scale)?;
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

fn validate_plan(data: &Value, scale: &str) -> Result<()> {
    need(
        data["window"] == json!({"purpose":"probe","warm_up_ticks":0,"observation_ticks":256})
            && data["scale"] == scale
            && data["case"] == "MIXED-PEAK"
            && data["seed"] == 544
            && data["dt"] == if scale == "10k" { 16 } else { 33 },
        "plan protocol",
    )
}

fn capture(root: &Path, input: &Path, raw: &Path) -> Result<()> {
    capture_for(root, input, raw, LEGACY)
}

pub(crate) fn capture_for(root: &Path, input: &Path, raw: &Path, protocol: Protocol) -> Result<()> {
    let repo = std::env::current_dir()?;
    let head = io::git(&repo, &["rev-parse", "HEAD"])?;
    need(
        io::git(&repo, &["status", "--porcelain"])?.is_empty(),
        "dirty collector",
    )?;
    io::git(
        &repo,
        &["merge-base", "--is-ancestor", protocol.baseline, &head],
    )?;
    io::ensure_new(raw)?;
    fs::create_dir_all(raw.parent().unwrap_or(Path::new(".")))?;
    fs::create_dir(raw)?;
    let raw = raw.canonicalize()?;
    let input = input.canonicalize()?;
    let mut identity = json!({"protocol":protocol.name, "head":head, "tree":io::git(&repo,&["rev-parse","HEAD^{tree}"])?, "started":now()?, "workers":4, "ticks":256, "inputs":inputs(&input)?, "rustc":io::command(&repo,"rustc",&["+1.98.0","-Vv"])?, "sources":{}, "binaries":{}});
    for mode in ["plain", "detail"] {
        let source = io::read_json(&root.join(format!("{mode}-source.json")))?;
        need(
            source["base"] == protocol.baseline
                && source["mode"] == mode
                && source["source_files"]
                    == io::source_index(&root.join(format!("{mode}-source")))?,
            "source identity",
        )?;
        let binary = root
            .join(format!("{mode}{}", std::env::consts::EXE_SUFFIX))
            .canonicalize()?;
        identity["sources"][mode] = source;
        identity["binaries"][mode] = json!({"path":binary,"sha256":io::sha(&binary)?});
    }
    need(
        identity["binaries"]["plain"]["sha256"] != identity["binaries"]["detail"]["sha256"],
        "identical diagnostic binary",
    )?;
    io::write_new(&raw.join("identity.json"), &identity)?;
    for (label, scale, mode) in labels() {
        need(
            io::git(&repo, &["rev-parse", "HEAD"])? == head
                && io::git(&repo, &["status", "--porcelain"])?.is_empty(),
            "collector drift",
        )?;
        let binary = Path::new(string(&identity["binaries"][&mode]["path"])?);
        need(
            io::sha(binary)? == identity["binaries"][&mode]["sha256"],
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
        let mut meta = json!({"label":label,"scale":scale,"mode":mode,"command":command,"head":head,"uuid":uuid::Uuid::new_v4().to_string(),"started":now()?});
        io::write_new(&path, &meta)?;
        let file = |extension| -> Result<fs::File> {
            Ok(fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(raw.join(format!("{label}.{extension}")))?)
        };
        let status = Command::new(binary)
            .args(args)
            .stdout(Stdio::from(file("stdout")?))
            .stderr(Stdio::from(file("stderr")?))
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
                && meta["binary_after"] == identity["binaries"][&mode]["sha256"],
            "run failure/drift",
        )?;
        println!("{label} complete");
    }
    need(inputs(&input)? == identity["inputs"], "input drift")?;
    for mode in ["plain", "detail"] {
        need(
            io::source_index(&root.join(format!("{mode}-source")))?
                == identity["sources"][mode]["source_files"],
            "source drift",
        )?;
    }
    identity["completed"] = json!(true);
    identity["ended"] = json!(now()?);
    io::replace_owned(&raw.join("identity.json"), &identity)
}

fn stats(mut values: Vec<u64>) -> Value {
    values.sort_unstable();
    json!({"mean_ms":values.iter().map(|v|u128::from(*v)).sum::<u128>() as f64 / values.len() as f64 / 1e6,"p95_ms":values[(values.len()*95).div_ceil(100)-1] as f64 / 1e6})
}

#[cfg(test)]
fn validate_rows(rows: &[Value], ticks: &[Value], mode: &str) -> Result<()> {
    validate_rows_for(rows, ticks, mode, LEGACY)
}
pub(crate) fn validate_rows_for(
    rows: &[Value],
    ticks: &[Value],
    mode: &str,
    protocol: Protocol,
) -> Result<()> {
    need(
        ["plain", "detail"].contains(&mode) && rows.len() == 256 && ticks.len() == 256,
        "row protocol/count",
    )?;
    let count = if mode == "plain" {
        12
    } else {
        protocol.stages.len()
    };
    for (i, row) in rows.iter().enumerate() {
        need(
            row["tick"] == i + 1
                && ticks[i]["tick"] == i + 1
                && ticks[i]["N_active"].as_u64().is_some()
                && row["step_ns"].as_u64().is_some_and(|v| v > 0),
            "row sequence/time",
        )?;
        let read = |name: &str| -> Result<Vec<u64>> {
            let a = row[name].as_array().ok_or("row array")?;
            need(a.len() == count, "row shape")?;
            a.iter()
                .map(|v| v.as_u64().ok_or_else(|| "row noninteger".into()))
                .collect()
        };
        let times = read("stages_ns")?;
        let calls = read("calls")?;
        if mode == "plain" {
            need(times.iter().chain(&calls).all(|v| *v == 0), "plain clocks")?;
            continue;
        }
        let sum = |a: &[u64]| a.iter().map(|v| u128::from(*v)).sum::<u128>();
        need(
            sum(&times[..10]) <= u128::from(row["step_ns"].as_u64().unwrap())
                && sum(&times[10..17]) <= u128::from(times[3]),
            "clock nesting",
        )?;
        need(
            calls[..13].iter().all(|v| *v == 1)
                && calls[16] == 1
                && calls[13] == calls[14]
                && calls[13] == u64::from(calls[17] >= 1_024)
                && calls[15] == u64::from(calls[17] > 0 && calls[17] < 1_024),
            "outer clock calls",
        )?;
        (protocol.validate)(&times, &calls, (i + 1) as u64)?;
    }
    Ok(())
}
fn validate_p2(times: &[u64], calls: &[u64], _tick: u64) -> Result<()> {
    let sum = |a: &[u64]| a.iter().map(|v| u128::from(*v)).sum::<u128>();
    need(
        u128::from(times[18]) + u128::from(times[24]) <= u128::from(times[2])
            && sum(&times[19..24]) <= u128::from(times[18]),
        "P2 clock nesting",
    )?;
    need(
        calls[18] == 1
            && calls[24] == 1
            && calls[19] == 1
            && calls[20] == 1
            && calls[21] == 1
            && calls[22] == 1
            && calls[23] == 0
            && calls[25] == calls[31]
            && calls[31] >= 1_024
            && sum(&calls[26..29]) == u128::from(calls[25])
            && u128::from(calls[29]) + u128::from(calls[26]) == u128::from(calls[25])
            && calls[30] == calls[25]
            && times[25..].iter().all(|v| *v == 0),
        "P2 path/counts",
    )
}

fn analyze(raw: &Path) -> Result<Value> {
    analyze_for(raw, LEGACY)
}
pub(crate) fn analyze_for(raw: &Path, protocol: Protocol) -> Result<Value> {
    let identity = io::read_json(&raw.join("identity.json"))?;
    need(
        identity["protocol"] == protocol.name
            && identity["completed"] == true
            && identity["workers"] == 4
            && identity["ticks"] == 256
            && identity["sources"]["plain"]["base"] == protocol.baseline
            && identity["sources"]["detail"]["base"] == protocol.baseline,
        "protocol completion",
    )?;
    let mut ids = std::collections::BTreeSet::new();
    let mut semantics = std::collections::BTreeMap::new();
    let mut runs = Vec::new();
    for (label, scale, mode) in labels() {
        let dir = raw.join(&label);
        let meta = io::read_json(&raw.join(format!("{label}.process.json")))?;
        let native = io::read_json(&dir.join("result.json"))?;
        let diagnostic = io::read_json(&dir.join("diagnostics.json"))?;
        validate_process(&meta, &identity, &label, &scale, &mode)?;
        need(
            ids.insert(string(&meta["uuid"])?.to_owned()),
            "duplicate process uuid",
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
                && diagnostic["binary"]["sha256"] == identity["binaries"][&mode]["sha256"],
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
            need(previous == semantic, "diagnostic changed traffic")?;
        }
        let ticks: Vec<Value> = fs::read_to_string(dir.join("ticks.jsonl"))?
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let rows: Vec<Value> = fs::read_to_string(raw.join(format!("{label}.stderr")))?
            .lines()
            .filter_map(|s| s.strip_prefix("LF762 "))
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        validate_rows_for(&rows, &ticks, &mode, protocol)?;
        let mut windows = json!({});
        for (window, start, end) in [("all", 0, 256), ("entry", 0, 64), ("screen", 64, 256)] {
            let part = &rows[start..end];
            let active: Vec<_> = ticks[start..end]
                .iter()
                .map(|t| t["N_active"].as_u64().unwrap())
                .collect();
            let mut data = json!({"step":stats(part.iter().map(|r|r["step_ns"].as_u64().unwrap()).collect()),"active_min":active.iter().min(),"active_max":active.iter().max()});
            if mode == "detail" {
                for (i, name) in protocol.stages.iter().enumerate() {
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
        runs.push(json!({"label":label,"scale":scale,"mode":mode,"windows":windows,"traffic":traffic,"initial_counts":native["initial_counts"],"final_counts":native["final_counts"]}));
    }
    Ok(json!({"identity":identity,"runs":runs,"files":io::file_index(raw)?}))
}

fn validate_process(
    meta: &Value,
    identity: &Value,
    label: &str,
    scale: &str,
    mode: &str,
) -> Result<()> {
    need(
        meta["label"] == label
            && meta["scale"] == scale
            && meta["mode"] == mode
            && meta["exit_code"] == 0
            && meta["head"] == identity["head"]
            && meta["head_after"] == identity["head"]
            && meta["status_after"] == ""
            && meta["binary_after"] == identity["binaries"][mode]["sha256"]
            && uuid::Uuid::parse_str(string(&meta["uuid"])?)?.get_version_num() == 4,
        "process identity",
    )
}

fn run() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    match a.first().map(String::as_str) {
        Some("prepare") if a.len()==3 => p2_export::export(Path::new(&a[2]),&a[1]),
        Some("run") if a.len()==4 => capture(Path::new(&a[1]),Path::new(&a[2]),Path::new(&a[3])),
        Some("analyze"|"verify") if a.len()==3 => { let raw=Path::new(&a[1]); let out=Path::new(&a[2]); let value=analyze(raw)?; if a[0]=="verify" { need(value==io::read_json(out)?,"published mismatch")?; println!("verified 12 runs"); Ok(()) } else { io::outside(raw,out)?; io::write_new(out,&value) } },
        _ => Err("prepare <plain|detail> <root> | run <root> <inputs> <new-raw> | analyze|verify <raw> <results>".into())
    }
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Vec<Value>, Vec<Value>) {
        let ticks = (1..=256)
            .map(|tick| json!({"tick":tick,"N_active":2_048}))
            .collect();
        let rows = (1..=256)
            .map(|tick| {
                let mut calls = vec![0; 32];
                calls[..13].fill(1);
                calls[16] = 1;
                calls[18..23].fill(1);
                calls[24] = 1;
                calls[25] = 2_048;
                calls[26] = 1_024;
                calls[28] = 1_024;
                calls[29] = 1_024;
                calls[30] = 2_048;
                calls[31] = 2_048;
                json!({"tick":tick,"step_ns":100,"stages_ns":vec![0;32],"calls":calls})
            })
            .collect();
        (rows, ticks)
    }
    #[test]
    fn counters_nesting_and_rows_reject_corruption() {
        let (rows, ticks) = fixture();
        validate_rows(&rows, &ticks, "detail").unwrap();
        assert!(validate_rows(&rows[..255], &ticks, "detail").is_err());
        for (field, value) in [
            ("tick", json!(0)),
            ("step_ns", json!(-1)),
            ("calls", json!([0])),
            ("stages_ns", json!(vec![1; 32])),
        ] {
            let mut bad = rows.clone();
            bad[0][field] = value;
            assert!(validate_rows(&bad, &ticks, "detail").is_err(), "{field}");
        }
        for index in [19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30] {
            let mut bad = rows.clone();
            bad[0]["calls"][index] = json!(99);
            assert!(
                validate_rows(&bad, &ticks, "detail").is_err(),
                "counter {index}"
            );
        }
        let mut bad = rows.clone();
        bad[0]["stages_ns"][18] = json!(101);
        assert!(validate_rows(&bad, &ticks, "detail").is_err());
        bad[0]["calls"][26] = json!(u64::MAX);
        assert!(validate_rows(&bad, &ticks, "detail").is_err());
    }
    #[test]
    fn plain_rejects_instrumentation_and_unknown_mode() {
        let (mut rows, ticks) = fixture();
        for row in &mut rows {
            row["stages_ns"] = json!(vec![0; 12]);
            row["calls"] = json!(vec![0; 12]);
        }
        validate_rows(&rows, &ticks, "plain").unwrap();
        rows[0]["calls"][0] = json!(1);
        assert!(validate_rows(&rows, &ticks, "plain").is_err());
        assert!(validate_rows(&rows, &ticks, "unknown").is_err());
    }
    #[test]
    fn percentiles_and_matrix() {
        assert_eq!(
            stats(vec![1_000_000, 2_000_000, 3_000_000, 4_000_000]),
            json!({"mean_ms":2.5,"p95_ms":4.0})
        );
        assert_eq!(labels().len(), 12);
    }
    #[test]
    fn process_drift_and_invalid_uuid_fail() {
        let identity = json!({"head":"frozen", "binaries":{"detail":{"sha256":"digest"}}});
        let meta = json!({"label":"label","scale":"100k","mode":"detail","exit_code":0,"head":"frozen","head_after":"frozen","status_after":"","binary_after":"digest","uuid":uuid::Uuid::new_v4().to_string()});
        validate_process(&meta, &identity, "label", "100k", "detail").unwrap();
        for (field, value) in [
            ("head_after", json!("other")),
            ("status_after", json!("dirty")),
            ("binary_after", json!("other")),
            ("exit_code", json!(1)),
            ("uuid", json!("invalid")),
        ] {
            let mut bad = meta.clone();
            bad[field] = value;
            assert!(
                validate_process(&bad, &identity, "label", "100k", "detail").is_err(),
                "{field}"
            );
        }
    }
    #[test]
    fn probe_plan_rejects_window_and_workload_drift() {
        let plan = json!({"window":{"purpose":"probe","warm_up_ticks":0,"observation_ticks":256},"scale":"10k","case":"MIXED-PEAK","seed":544,"dt":16});
        validate_plan(&plan, "10k").unwrap();
        for (field, value) in [
            (
                "window",
                json!({"purpose":"probe","warm_up_ticks":64,"observation_ticks":256}),
            ),
            ("dt", json!(33)),
            ("seed", json!(0)),
            ("scale", json!("100k")),
        ] {
            let mut bad = plan.clone();
            bad[field] = value;
            assert!(validate_plan(&bad, "10k").is_err(), "{field}");
        }
    }
}
