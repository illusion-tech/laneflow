//! 校验冻结输入与完整独立运行后汇总；错误在 release 构建中同样阻止输出。
use std::{collections::BTreeMap, error::Error, fs, io::Write, path::Path};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn require(condition: bool, message: &str) -> Result<()> {
    if !condition {
        return Err(message.into());
    }
    Ok(())
}

fn load(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(text.trim_start_matches('\u{feff}'))?)
}

fn digest(path: &Path) -> Result<String> {
    Ok(Sha256::digest(fs::read(path)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn number(value: &Value) -> Result<u64> {
    value
        .as_u64()
        .ok_or_else(|| "expected unsigned integer".into())
}

fn text(value: &Value) -> Result<&str> {
    value
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "expected nonempty string".into())
}

fn same_state(left: &Value, right: &Value) -> Result<()> {
    for key in [
        "initial_state_digest",
        "final_state_digest",
        "domain_event_digest",
    ] {
        require(text(&left[key])? == text(&right[key])?, key)?;
    }
    require(
        left["checkpoints"]
            .as_array()
            .is_some_and(|rows| rows.len() == 3),
        "expected H/2H/4H checkpoints",
    )?;
    require(
        left["checkpoints"] == right["checkpoints"],
        "checkpoint mismatch",
    )
}

fn process(
    directory: &Path,
    freeze: &Value,
    freeze_hash: &str,
    name: &str,
    binary: &str,
) -> Result<Value> {
    let row = load(&directory.join(format!("{name}.process.json")))?;
    require(row["exitCode"] == 0, &format!("failed process: {name}"))?;
    require(
        text(&row["sourceCommit"])? == text(&freeze["sourceCommit"])?,
        "source commit mismatch",
    )?;
    require(
        text(&row["freezeSha256"])? == freeze_hash,
        "freeze hash mismatch",
    )?;
    require(
        text(&row["binarySha256"])? == text(&freeze["executables"][binary]["sha256"])?,
        "binary hash mismatch",
    )?;
    for key in ["peakWorkingSetBytes", "processCommitPeakBytes"] {
        require(
            number(&row[key])? > 0,
            "missing process lifetime memory peak",
        )?;
    }
    Ok(row)
}

fn aggregate(rows: [&Value; 3]) -> Result<Value> {
    let mut output = serde_json::Map::new();
    for key in ["p50", "p95", "p99", "max"] {
        let mut samples = [
            number(&rows[0][key])?,
            number(&rows[1][key])?,
            number(&rows[2][key])?,
        ];
        samples.sort_unstable();
        output.insert(key.into(), json!(samples[if key == "max" { 2 } else { 1 }]));
    }
    Ok(Value::Object(output))
}

fn timing_groups(rows: [&Value; 3]) -> Result<Value> {
    let first = rows[0].as_object().ok_or("missing timing groups")?;
    require(!first.is_empty(), "empty timing groups")?;
    let mut output = serde_json::Map::new();
    for key in first.keys() {
        output.insert(
            key.clone(),
            aggregate([&rows[0][key], &rows[1][key], &rows[2][key]])?,
        );
    }
    Ok(Value::Object(output))
}

fn ledger(directory: &Path, count: u64, expected: &Value) -> Result<Value> {
    let log = fs::read_to_string(directory.join(format!("ledger-{count}.stderr.log")))?;
    let mut checkpoints = BTreeMap::new();
    for line in log.lines() {
        if let Some((_, suffix)) = line.split_once("checkpoint=") {
            let (tick, hash) = suffix
                .split_once(" digest=")
                .ok_or("invalid checkpoint log")?;
            require(
                checkpoints
                    .insert(
                        tick.parse::<u64>()?,
                        hash.split_whitespace()
                            .next()
                            .ok_or("missing checkpoint digest")?
                            .to_owned(),
                    )
                    .is_none(),
                "duplicate checkpoint",
            )?;
        }
    }
    let expected_points = expected.as_array().ok_or("missing checkpoints")?;
    require(
        checkpoints.len() == 3 && expected_points.len() == 3,
        "missing ledger checkpoints",
    )?;
    for point in expected_points {
        require(
            checkpoints
                .get(&number(&point["observation_tick"])?)
                .map(String::as_str)
                == Some(text(&point["state_digest"])?),
            "ledger state digest mismatch",
        )?;
    }
    let csv = fs::read_to_string(directory.join(format!("ledger-{count}.csv")))?;
    let mut lines = csv.trim_start_matches('\u{feff}').lines();
    let header: Vec<_> = lines
        .next()
        .ok_or("missing ledger header")?
        .split(',')
        .collect();
    let mut maxima = BTreeMap::<String, u64>::new();
    let mut ticks = 0;
    for line in lines {
        let cells = line
            .split(',')
            .map(str::parse::<u64>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        require(cells.len() == header.len(), "invalid ledger column count")?;
        let row: BTreeMap<_, _> = header.iter().copied().zip(cells).collect();
        require(row.len() == header.len(), "duplicate ledger column")?;
        ticks += 1;
        require(
            row.get("observation_tick") == Some(&ticks),
            "noncontiguous ledger tick",
        )?;
        let mut owned = 0;
        for key in ["binding", "committed", "derived", "scratch", "admin"] {
            owned += row.get(key).ok_or("missing world ledger column")?;
        }
        let total = maxima
            .entry("world_owned_same_tick_total".into())
            .or_default();
        *total = (*total).max(owned);
        for (key, value) in row {
            if key != "observation_tick" {
                let maximum = maxima.entry(key.into()).or_default();
                *maximum = (*maximum).max(value);
            }
        }
    }
    require(ticks == 4096, "incomplete ledger window")?;
    Ok(serde_json::to_value(maxima)?)
}

fn analyze(directory: &Path) -> Result<Value> {
    let freeze_path = directory.join("freeze.json");
    let freeze = load(&freeze_path)?;
    let freeze_hash = digest(&freeze_path)?;
    let mut output = json!({"source_commit": text(&freeze["sourceCommit"])?, "freeze_sha256": freeze_hash,
        "certification": text(&freeze["environment"]["certification"])?, "scales": {}});
    let cases = freeze["inputs"].as_array().ok_or("missing input cases")?;
    require(
        cases.len() == 2 && cases[0]["vehicles"] == 10000 && cases[1]["vehicles"] == 100000,
        "expected both ordered scales",
    )?;
    require(
        text(&cases[0]["files"]["source-config.toml"])?
            == text(&cases[1]["files"]["source-config.toml"])?,
        "scale configurations differ",
    )?;
    for case in cases {
        let count = number(&case["vehicles"])?;
        let expected_cells = if count == 10000 { 32 } else { 320 };
        require(
            case["cells"] == expected_cells && case["prepared"]["input"]["cells"] == expected_cells,
            "incorrect cell count",
        )?;
        let files = case["files"]
            .as_object()
            .ok_or("missing frozen input hashes")?;
        for (file, hash) in files {
            require(
                digest(&Path::new(text(&case["directory"])?).join(file))? == text(hash)?,
                "frozen input changed",
            )?;
        }
        let mut rows = Vec::new();
        let mut processes = Vec::new();
        for round in 1..=3 {
            let name = format!("render-{count}-round-{round}");
            processes.push(process(
                directory,
                &freeze,
                &freeze_hash,
                &name,
                "junction_scale_render",
            )?);
            let row = load(&directory.join(format!("{name}.json")))?;
            require(
                row["input"] == case["prepared"]["input"] && row["input"]["vehicles"] == count,
                "input mismatch",
            )?;
            require(
                text(&row["initial_state_digest"])?
                    == text(&case["prepared"]["initial_state_digest"])?,
                "initial state mismatch",
            )?;
            require(
                row["allocation_instrumented"] == false,
                "instrumented timing run",
            )?;
            let actual = &row["counts"];
            require(
                actual["worlds"] == 1
                    && actual["minimum_active"] == count
                    && actual["observed_ticks"] == 36024,
                "invalid observed population or window",
            )?;
            require(
                actual["maximum_backlog_quanta"] == 2
                    && number(&actual["two_quantum_backlog_frames"])? > 0
                    && actual["max_backlog_recovery_frames"] == 1,
                "invalid backlog coverage",
            )?;
            require(
                actual["pose_rows_per_frame"] == count,
                "incomplete pose output",
            )?;
            let presented = if count == 10000 { count } else { count / 10 };
            require(
                actual["transform_rows_per_frame"] == presented
                    && actual["minimum_renderer_visible_proxies"] == presented,
                "incomplete presentation",
            )?;
            for field in [
                "waiting_vehicle_ticks",
                "reservation_vehicle_ticks",
                "waiting_zones_with_repeated_requests",
                "vehicles_with_repeated_conflict_requests",
            ] {
                require(number(&row["resource_loads"][field])? > 0, field)?;
            }
            rows.push(row);
        }
        let identities = processes
            .iter()
            .map(|row| Ok((number(&row["pid"])?, text(&row["startedUtc"])?)))
            .collect::<Result<std::collections::BTreeSet<_>>>()?;
        require(identities.len() == 3, "runs are not fresh processes")?;
        same_state(&rows[0], &rows[1])?;
        same_state(&rows[0], &rows[2])?;
        let allocation = load(&directory.join(format!("allocation-{count}.json")))?;
        process(
            directory,
            &freeze,
            &freeze_hash,
            &format!("allocation-{count}"),
            "junction_scale_allocation",
        )?;
        require(
            allocation["allocation_instrumented"] == true
                && allocation["input"] == rows[0]["input"],
            "invalid allocation input",
        )?;
        require(
            allocation["allocation"] == json!({"tick_allocations": 0, "tick_reallocations": 0}),
            "allocation after warmup",
        )?;
        same_state(&allocation, &rows[0])?;
        process(
            directory,
            &freeze,
            &freeze_hash,
            &format!("ledger-{count}"),
            "junction_ledger",
        )?;
        let ledger = ledger(directory, count, &rows[0]["checkpoints"])?;
        let timings = timing_groups([
            &rows[0]["nanoseconds"],
            &rows[1]["nanoseconds"],
            &rows[2]["nanoseconds"],
        ])?;
        let mut frame_classes = serde_json::Map::new();
        for group in ["zero_step", "one_step", "two_step", "eight_step"] {
            frame_classes.insert(
                group.into(),
                timing_groups([
                    &rows[0]["frame_classes"][group],
                    &rows[1]["frame_classes"][group],
                    &rows[2]["frame_classes"][group],
                ])?,
            );
        }
        let tick_budget = if count == 10000 {
            2_000_000
        } else {
            16_000_000
        };
        output["scales"][count.to_string()] = json!({
            "runtime_p95_budget_ns": tick_budget,
            "runtime_p95_within_budget": number(&timings["tick_and_driver"]["p95"])? <= tick_budget,
            "spatial_adapter_one_step_p95_within_4ms": number(&frame_classes["one_step"]["spatial_adapter"]["p95"])? <= 4_000_000,
            "nanoseconds": timings, "frame_classes": frame_classes,
            "counts_by_round": rows.iter().map(|row| &row["counts"]).collect::<Vec<_>>(),
            "resource_loads_by_round": rows.iter().map(|row| &row["resource_loads"]).collect::<Vec<_>>(),
            "final_state_digest": rows[0]["final_state_digest"], "domain_event_digest": rows[0]["domain_event_digest"],
            "ledger_high_water_over_4096_tick_replay": ledger,
            "process_memory_by_round": processes.iter().map(|row| json!({"peakWorkingSetBytes": row["peakWorkingSetBytes"], "privateBytesSampledPeak": row["privateBytesSampledPeak"], "processCommitPeakBytes": row["processCommitPeakBytes"]})).collect::<Vec<_>>(),
            "limits": ["logical component ledger covers H/2H/4H replay; not a full-window process peak",
                "renderer uses unlit vehicle proxies; no road assets or GPU timestamps",
                "native pose buffer capacities and ECS/renderer component capacity ledgers unavailable"]
        });
    }
    Ok(output)
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    require(
        args.len() == 1,
        "usage: junction_scale_analyze <evidence-directory>",
    )?;
    let directory = Path::new(&args[0]);
    let summary = analyze(directory)?;
    let path = directory.join("summary.json");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(serde_json::to_string_pretty(&summary)?.as_bytes())?;
    println!("{}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_or_mismatched_process_evidence_is_rejected_in_release_too() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("junction-analysis-test-{}", std::process::id()));
        fs::create_dir(&directory)?;
        let path = directory.join("sample.process.json");
        let freeze =
            json!({"sourceCommit": "commit", "executables": {"binary": {"sha256": "binary-hash"}}});
        let mut row = json!({"exitCode": 0, "sourceCommit": "commit", "freezeSha256": "freeze-hash", "binarySha256": "binary-hash", "peakWorkingSetBytes": 100, "processCommitPeakBytes": 200});
        fs::write(&path, serde_json::to_vec(&row)?)?;
        assert!(process(&directory, &freeze, "freeze-hash", "sample", "binary").is_ok());
        row["exitCode"] = json!(1);
        fs::write(&path, serde_json::to_vec(&row)?)?;
        assert!(process(&directory, &freeze, "freeze-hash", "sample", "binary").is_err());
        row["exitCode"] = json!(0);
        row["binarySha256"] = json!("stale-binary");
        fs::write(&path, serde_json::to_vec(&row)?)?;
        assert!(process(&directory, &freeze, "freeze-hash", "sample", "binary").is_err());
        fs::remove_file(path)?;
        fs::remove_dir(directory)?;
        Ok(())
    }

    #[test]
    fn aggregate_uses_median_percentiles_and_highest_maximum() -> Result<()> {
        let rows = [
            json!({"p50": 1, "p95": 10, "p99": 11, "max": 100}),
            json!({"p50": 3, "p95": 30, "p99": 31, "max": 40}),
            json!({"p50": 2, "p95": 20, "p99": 21, "max": 50}),
        ];
        assert_eq!(
            aggregate([&rows[0], &rows[1], &rows[2]])?,
            json!({"p50": 2, "p95": 20, "p99": 21, "max": 100})
        );
        Ok(())
    }
}
