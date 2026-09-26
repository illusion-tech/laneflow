use crate::{BASE, ORDER, Result, STAGES, io, need, string};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

#[derive(Clone, Deserialize)]
pub(crate) struct Row {
    tick: u64,
    step_ns: u64,
    stages_ns: Vec<u64>,
    calls: Vec<u64>,
}

pub(crate) fn rows(text: &str, mode: &str) -> Result<Vec<Row>> {
    let data: Vec<Row> = text
        .lines()
        .filter_map(|line| line.strip_prefix("LF762 "))
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    need(data.len() == 256, "tick count")?;
    let count = if mode == "detail" { 18 } else { 12 };
    for (index, r) in data.iter().enumerate() {
        need(
            r.tick == index as u64 + 1 && r.step_ns > 0,
            "tick sequence/time",
        )?;
        let t = &r.stages_ns;
        let c = &r.calls;
        need(t.len() == count && c.len() == count, "stage shape")?;
        if mode == "plain" {
            need(t.iter().chain(c).all(|x| *x == 0), "plain instrumentation")?;
        } else {
            need(
                sum(&t[..10]) <= u128::from(r.step_ns),
                "overlapping outer stages",
            )?;
            if mode == "stages" {
                need(
                    c.iter().all(|x| *x == 1) && sum(&t[10..]) <= u128::from(t[3]),
                    "stage calls/nesting",
                )?;
            } else {
                need(
                    c[..13].iter().all(|x| *x == 1) && c[16] == 1 && c[13] == c[14],
                    "detail clocks",
                )?;
                need(
                    c[13] == u64::from(c[17] >= 1_024)
                        && c[15] == u64::from(c[17] > 0 && c[17] < 1_024),
                    "detail path",
                )?;
                need(
                    (0..17).all(|i| c[i] != 0 || t[i] == 0) && t[17] == 0,
                    "inactive clock",
                )?;
                need(sum(&t[10..17]) <= u128::from(t[3]), "detail partition")?;
            }
        }
    }
    Ok(data)
}

fn sum(values: &[u64]) -> u128 {
    values.iter().map(|v| u128::from(*v)).sum()
}

pub(crate) fn stats(mut values: Vec<u64>) -> Value {
    values.sort_unstable();
    let percentile = |p: usize| values[(values.len() * p).div_ceil(100) - 1] as f64 / 1e6;
    json!({"mean_ms":(sum(&values) as f64 / values.len() as f64) / 1e6,"p50_ms":percentile(50),"p95_ms":percentile(95),"p99_ms":percentile(99),"max_ms":*values.last().expect("nonempty validated window") as f64 / 1e6})
}

fn windows(rows: &[Row], mode: &str) -> Value {
    let mut out = json!({});
    for (name, start, end) in [("all", 0, 256), ("entry", 0, 64), ("screen", 64, 256)] {
        let part = &rows[start..end];
        let mut window = json!({"step":stats(part.iter().map(|r|r.step_ns).collect())});
        if mode == "stages" {
            window["stages"] = stage_stats(part, 12);
            window["unattributed"] = stats(
                part.iter()
                    .map(|r| r.step_ns - sum(&r.stages_ns[..10]) as u64)
                    .collect(),
            );
            let mut ranking = STAGES[..10].to_vec();
            ranking.sort_by(|a, b| {
                window["stages"][b]["mean_ms"]
                    .as_f64()
                    .unwrap()
                    .total_cmp(&window["stages"][a]["mean_ms"].as_f64().unwrap())
            });
            window["ranking"] = json!(ranking);
        }
        out[name] = window;
    }
    out
}

fn stage_stats(rows: &[Row], count: usize) -> Value {
    let mut out = json!({});
    for (i, stage) in STAGES.iter().enumerate().take(count) {
        out[*stage] = stats(rows.iter().map(|r| r.stages_ns[i]).collect());
    }
    out
}

fn matrix(root: &Path, expected: &BTreeSet<String>) -> Result<()> {
    let actual: BTreeSet<String> = fs::read_dir(root)?
        .map(|p| p.map(|p| p.file_name().to_string_lossy().into_owned()))
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .filter_map(|s| s.strip_suffix(".process.json").map(str::to_owned))
        .collect();
    need(actual == *expected, "matrix")
}

pub(crate) fn analyze(root: &Path, parent: Option<&Path>) -> Result<Value> {
    let identity = io::read_json(&root.join("identity.json"))?;
    need(
        identity["completed"] == true
            && identity["base"] == BASE
            && identity["ticks"] == 256
            && identity["workers"] == 4,
        "identity/protocol",
    )?;
    let detail = parent.is_some();
    let reference = if let Some(p) = parent {
        need(
            io::sha(p)? == string(&identity["parent_results_sha256"])?,
            "detail parent",
        )?;
        need(
            io::sha(&root.join("source.json"))? == string(&identity["source_index_sha256"])?,
            "detail source",
        )?;
        need(identity["stage_names"] == json!(STAGES), "detail stages")?;
        Some(io::read_json(p)?)
    } else {
        need(identity["order"] == json!(ORDER), "matrix order")?;
        for mode in ["plain", "stages"] {
            need(
                io::sha(&root.join(format!("{mode}-source.json")))?
                    == string(&identity["source_indexes"][mode])?,
                "source index",
            )?;
        }
        None
    };
    let expected: BTreeSet<String> = if detail {
        (1..=3).map(|n| format!("100k-{n}-detail")).collect()
    } else {
        ["10k", "100k"]
            .into_iter()
            .flat_map(|s| {
                ORDER
                    .iter()
                    .enumerate()
                    .map(move |(n, m)| format!("{s}-{}-{m}", n + 1))
            })
            .collect()
    };
    matrix(root, &expected)?;
    let mut uuids = BTreeSet::new();
    let mut executions = BTreeSet::new();
    let mut semantics = BTreeMap::new();
    let mut runs = Vec::new();
    for label in expected {
        let parts: Vec<_> = label.split('-').collect();
        let (scale, mode) = (parts[0], parts[2]);
        let meta = io::read_json(&root.join(format!("{label}.process.json")))?;
        let path = root.join(&label);
        let result = io::read_json(&path.join("result.json"))?;
        let diagnostic = io::read_json(&path.join("diagnostics.json"))?;
        let binary = if detail {
            &identity["binary_sha256"]
        } else {
            &identity["binaries"][mode]["sha256"]
        };
        need(
            meta["label"] == label
                && meta["exit_code"] == 0
                && meta["status_after"] == ""
                && meta["head_after"] == identity["research_head"]
                && meta["binary_after"] == *binary,
            "process state",
        )?;
        if !detail {
            need(
                meta["scale"] == scale
                    && meta["mode"] == mode
                    && meta["research_head"] == identity["research_head"],
                "run identity",
            )?;
            need(
                uuids.insert(string(&meta["uuid"])?.to_owned()),
                "duplicate UUID",
            )?;
        }
        need(
            result["scale"] == scale
                && result["case"] == "MIXED-PEAK"
                && result["status"] == "probe-complete"
                && result["completed_ticks"] == 256
                && result["error"].is_null(),
            "native completion/case",
        )?;
        need(
            diagnostic["invocation"] == meta["command"]
                && diagnostic["workers"] == 4
                && diagnostic["verified_steps"] == 256
                && diagnostic["git_commit_at_run"] == identity["research_head"]
                && diagnostic["git_status_at_run"] == ""
                && diagnostic["binary"]["sha256"] == *binary,
            "native identity",
        )?;
        need(
            executions.insert(string(&diagnostic["execution_id"])?.to_owned()),
            "duplicate execution",
        )?;
        for (name, digest) in result["files"].as_object().ok_or("native files")? {
            let file = io::safe_child(&path, name)?;
            need(
                fs::metadata(&file)?.len() == digest["bytes"].as_u64().ok_or("file length")?
                    && io::sha(&file)? == string(&digest["sha256"])?,
                "native digest",
            )?;
        }
        let mut traffic = json!({});
        for name in ["ticks.jsonl", "commands.jsonl", "events.jsonl"] {
            traffic[name] = json!(io::sha(&path.join(name))?);
        }
        let mut semantic = result.clone();
        semantic
            .as_object_mut()
            .ok_or("native result")?
            .remove("files");
        let semantic = json!({"result":semantic,"traffic":traffic});
        if let Some(previous) = semantics.insert(scale.to_owned(), semantic.clone()) {
            need(previous == semantic, "traffic/result changed")?;
        }
        if let Some(parent) = &reference {
            let previous = parent["runs"]
                .as_array()
                .ok_or("parent runs")?
                .iter()
                .find(|r| r["scale"] == "100k")
                .ok_or("parent 100k")?;
            need(previous["traffic"] == traffic, "detail traffic")?;
        }
        let rows = rows(
            &fs::read_to_string(root.join(format!("{label}.stderr")))?,
            mode,
        )?;
        let mut ordered: Vec<_> = rows.iter().map(|r| r.step_ns).collect();
        ordered.sort_unstable();
        for p in [50, 95, 99] {
            need(
                diagnostic[format!("step_ns_p{p}")] == ordered[(256_usize * p).div_ceil(100) - 1],
                "native timing disagreement",
            )?;
        }
        if detail {
            let part = &rows[64..];
            let work: Vec<_> = part.iter().map(|r| r.calls[17]).collect();
            runs.push(json!({"label":label,"step":stats(part.iter().map(|r|r.step_ns).collect()),"stages":stage_stats(part,17),
                "p3_unattributed":stats(part.iter().map(|r|r.stages_ns[3]-sum(&r.stages_ns[10..17]) as u64).collect()),
                "workload_min":work.iter().min(),"workload_max":work.iter().max(),"workload_mean":sum(&work) as f64/work.len() as f64,
                "dispatch_ticks":part.iter().map(|r|r.calls[13]).sum::<u64>(),"fused_ticks":part.iter().map(|r|r.calls[15]).sum::<u64>()}));
        } else {
            runs.push(json!({"label":label,"mode":mode,"scale":scale,"windows":windows(&rows,mode),"initial_counts":result["initial_counts"],"final_counts":result["final_counts"],"traffic":traffic}));
        }
    }
    let mut out = json!({"identity":identity,"runs":runs,"files":io::file_index(root)?});
    if !detail {
        out["stage_names"] = json!(&STAGES[..12]);
    }
    Ok(out)
}

// JSON 表示可重排和缩进，统计数值、整数、身份和摘要必须严格相等。
pub(crate) fn same_json(actual: &Value, expected: &Value, path: &str) -> Result<()> {
    match (actual, expected) {
        (Value::Object(a), Value::Object(b)) => {
            need(a.len() == b.len(), &format!("object shape: {path}"))?;
            for (key, value) in a {
                same_json(
                    value,
                    b.get(key).ok_or_else(|| format!("missing {path}/{key}"))?,
                    &format!("{path}/{key}"),
                )?;
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            need(a.len() == b.len(), &format!("array shape: {path}"))?;
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                same_json(a, b, &format!("{path}/{i}"))?;
            }
        }
        _ => need(actual == expected, &format!("value: {path}"))?,
    }
    Ok(())
}

#[cfg(test)]
mod tests;
