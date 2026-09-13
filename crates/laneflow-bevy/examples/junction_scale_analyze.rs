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
    require(
        text(&left["validation"]["trajectory_sha256"])?
            == text(&right["validation"]["trajectory_sha256"])?,
        "per-identity per-tick trajectory mismatch",
    )?;
    require(
        text(&left["validation"]["domain_batches_sha256"])?
            == text(&right["validation"]["domain_batches_sha256"])?,
        "warmup/observation decision and event mismatch",
    )?;
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
    require(text(&row["name"])? == name, "process record name mismatch")?;
    require(
        text(&row["executable"])? == text(&freeze["executables"][binary]["path"])?,
        "process executable mismatch",
    )?;
    require(number(&row["pid"])? > 0, "missing process identity")?;
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
        text(&row["stableEnvironmentSha256"])?
            == text(&freeze["environment"]["stableEnvironmentSha256"])?,
        "execution environment differs from freeze",
    )?;
    require(
        row["powerMatchesFreeze"] == true,
        "execution power state changed",
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

fn same_file(recorded: &Value, expected: &Path) -> Result<()> {
    require(
        fs::canonicalize(text(recorded)?)? == fs::canonicalize(expected)?,
        "process argument path mismatch",
    )
}

fn bind_integrated_result(
    process: &Value,
    result: &Value,
    case: &Value,
    output: &Path,
) -> Result<()> {
    require(
        number(&process["pid"])? == number(&result["pid"])?,
        "result and process PID mismatch",
    )?;
    let arguments = process["arguments"].as_array().ok_or("missing arguments")?;
    require(arguments.len() == 7, "invalid integrated arguments")?;
    let input = Path::new(text(&case["directory"])?);
    same_file(&arguments[0], &input.join("network.lfca"))?;
    same_file(&arguments[1], &input.join("grid.catalog.toml"))?;
    same_file(&arguments[6], output)?;
    for (index, key) in [
        (2, "vehicles"),
        (3, "warmup_ticks"),
        (4, "observation_ticks"),
    ] {
        require(
            text(&arguments[index])? == number(&case["prepared"]["input"][key])?.to_string(),
            "process argument/input mismatch",
        )?;
    }
    require(arguments[5] == "catchup", "process frame mode mismatch")?;
    require(
        process["environmentOverrides"] == json!({}),
        "unexpected integrated environment overrides",
    )
}

fn bind_ledger_process(process: &Value, directory: &Path, case: &Value) -> Result<()> {
    require(
        process["arguments"]
            == json!([
                "kernel::junction_ledger::junction_reference_ledger",
                "--ignored",
                "--exact",
                "--nocapture",
                "--test-threads=1"
            ]),
        "ledger command mismatch",
    )?;
    let count = number(&case["vehicles"])?;
    let environment = &process["environmentOverrides"];
    same_file(
        &environment["JUNCTION_LEDGER_LFCA"],
        &Path::new(text(&case["directory"])?).join("network.lfca"),
    )?;
    same_file(
        &environment["JUNCTION_LEDGER_SNAPSHOT"],
        &directory.join(format!("render-{count}-round-1.warm.lfrs")),
    )?;
    same_file(
        &environment["JUNCTION_LEDGER_OUTPUT"],
        &directory.join(format!("ledger-{count}.csv")),
    )?;
    require(
        text(&environment["JUNCTION_LEDGER_VEHICLES"])? == count.to_string()
            && text(&environment["JUNCTION_LEDGER_CELLS"])? == number(&case["cells"])?.to_string(),
        "ledger scale mismatch",
    )
}

fn fidelity(result: &Value) -> Result<()> {
    let presentation = &result["presentation_validation"];
    let frames = number(&result["counts"]["frames"])?;
    require(
        frames > 0
            && number(&presentation["violations"])? == 0
            && number(&presentation["checked_pose_rows"])?
                == frames * number(&result["counts"]["pose_rows_per_frame"])?
            && number(&presentation["checked_transform_rows"])?
                == frames * number(&result["counts"]["transform_rows_per_frame"])?,
        "missing, incomplete or failing presentation validation",
    )?;
    let validation = &result["validation"];
    require(
        validation["schema"] == "junction-scale-validation-v1",
        "missing fidelity validation",
    )?;
    require(
        text(&validation["trajectory_sha256"])?.len() == 64,
        "missing per-tick trajectory digest",
    )?;
    require(
        text(&validation["domain_batches_sha256"])?.len() == 64,
        "missing all-tick decision/event digest",
    )?;
    let ticks =
        number(&result["input"]["warmup_ticks"])? + number(&result["input"]["observation_ticks"])?;
    require(
        number(&validation["checked_ticks"])? == ticks
            && number(&validation["checked_observation_ticks"])? == ticks
            && number(&validation["checked_vehicle_rows"])?
                == ticks * number(&result["input"]["vehicles"])?,
        "incomplete per-tick fidelity validation",
    )?;
    require(
        number(&validation["checked_gate_crossings"])? > 0
            && number(&validation["checked_events"])? > 0,
        "fidelity validation never observed transitions",
    )?;
    require(
        validation.get("failure").is_some_and(Value::is_null),
        "fidelity validation failed or incomplete",
    )?;
    for kind in [
        "overlap",
        "minimum_gap",
        "signal_stop_line",
        "numeric_geometry",
        "identity_route_lifecycle",
        "parking_binding",
        "signal_authority",
        "tick_time",
        "event_order",
        "event_causality",
        "conflict_exclusivity",
        "observation_projection",
    ] {
        require(number(&validation["violations"][kind])? == 0, kind)?;
    }
    Ok(())
}

fn resource_loads(row: &Value) -> Result<()> {
    for field in [
        "waiting_vehicle_ticks",
        "reservation_vehicle_ticks",
        "waiting_zones_with_repeated_requests",
        "vehicles_with_repeated_conflict_requests",
    ] {
        require(number(&row["resource_loads"][field])? > 0, field)?;
    }
    for field in [
        "longest_observed_waiting_hold_ticks",
        "longest_reservation_age_ticks",
    ] {
        require(number(&row["resource_loads"][field])? > 1, field)?;
    }
    Ok(())
}

fn authenticate_case(case: &Value) -> Result<()> {
    let files = case["files"]
        .as_object()
        .ok_or("missing frozen input hashes")?;
    require(
        text(&case["files"]["prepared.initial.lfrs"])?
            == text(&case["prepared"]["snapshot_sha256"])?,
        "initial snapshot is not authenticated",
    )?;
    text(&case["files"]["prepared.json"])?;
    for (file, hash) in files {
        require(
            digest(&Path::new(text(&case["directory"])?).join(file))? == text(hash)?,
            "frozen input changed",
        )?;
    }
    Ok(())
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

fn budget_comparison(timing: &Value, p95: u64, quantum: Option<u64>) -> Result<Value> {
    let p99 = p95 * 3 / 2;
    let maximum = quantum.map_or(p95 * 2, |quantum| (p95 * 2).min(quantum));
    Ok(
        json!({"p95_budget_ns": p95, "p99_budget_ns": p99, "max_budget_ns": maximum,
        "p95_within_budget": number(&timing["p95"])? <= p95,
        "p99_within_budget": number(&timing["p99"])? <= p99,
        "max_within_budget": number(&timing["max"])? <= maximum}),
    )
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
    require(
        freeze["protocol"]["maxCatchUpSteps"] == 2
            && freeze["protocol"]["frameInputQuanta"] == json!([0, 1, 2, 4, 0]),
        "noncanonical catch-up protocol",
    )?;
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
        authenticate_case(case)?;
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
            bind_integrated_result(
                processes.last().ok_or("missing process")?,
                &row,
                case,
                &directory.join(format!("{name}.json")),
            )?;
            fidelity(&row)?;
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
            resource_loads(&row)?;
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
        let allocation_process = process(
            directory,
            &freeze,
            &freeze_hash,
            &format!("allocation-{count}"),
            "junction_scale_allocation",
        )?;
        bind_integrated_result(
            &allocation_process,
            &allocation,
            case,
            &directory.join(format!("allocation-{count}.json")),
        )?;
        fidelity(&allocation)?;
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
        let ledger_process = process(
            directory,
            &freeze,
            &freeze_hash,
            &format!("ledger-{count}"),
            "junction_ledger",
        )?;
        bind_ledger_process(&ledger_process, directory, case)?;
        let ledger = ledger(directory, count, &rows[0]["checkpoints"])?;
        let timings = timing_groups([
            &rows[0]["nanoseconds"],
            &rows[1]["nanoseconds"],
            &rows[2]["nanoseconds"],
        ])?;
        let mut frame_classes = serde_json::Map::new();
        for group in ["zero_step", "one_step", "two_step"] {
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
        let integrated = &frame_classes["one_step"]["laneflow_frame_without_evidence"];
        output["scales"][count.to_string()] = json!({
            "runtime_p95_budget_ns": tick_budget,
            "runtime_p95_within_budget": number(&timings["tick_and_driver"]["p95"])? <= tick_budget,
            "spatial_adapter_one_step_p95_within_4ms": number(&frame_classes["one_step"]["spatial_adapter"]["p95"])? <= 4_000_000,
            "budget_comparisons": {
                "scope": "reference workload comparison, not product certification; 100k uses 16ms stretch quantum",
                "runtime": budget_comparison(&timings["tick_and_driver"], tick_budget, Some(16_000_000))?,
                "spatial_adapter_one_step": budget_comparison(&frame_classes["one_step"]["spatial_adapter"], 4_000_000, None)?,
                "laneflow_one_step": if count == 10000 { budget_comparison(integrated, 6_000_000, None)? } else { json!({"observation_threshold_ns": 16_667_000, "p95_exceeds_observation_threshold": number(&integrated["p95"])? > 16_667_000, "product_tail_budget": null}) }
            },
            "nanoseconds": timings, "frame_classes": frame_classes,
            "counts_by_round": rows.iter().map(|row| &row["counts"]).collect::<Vec<_>>(),
            "resource_loads_by_round": rows.iter().map(|row| &row["resource_loads"]).collect::<Vec<_>>(),
            "validation_by_round": rows.iter().map(|row| &row["validation"]).collect::<Vec<_>>(),
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
    fn warmup_batch_differences_are_rejected_after_state_convergence() {
        let baseline = json!({
            "initial_state_digest":"initial", "final_state_digest":"converged",
            "domain_event_digest":"same-observation", "checkpoints":[1,2,3],
            "validation":{"trajectory_sha256":"a".repeat(64), "domain_batches_sha256":"b".repeat(64)}
        });
        same_state(&baseline, &baseline).unwrap();
        let mut changed_warmup = baseline.clone();
        changed_warmup["validation"]["domain_batches_sha256"] = json!("c".repeat(64));
        assert!(same_state(&baseline, &changed_warmup).is_err());
    }

    #[test]
    fn failed_or_mismatched_process_evidence_is_rejected_in_release_too() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("junction-analysis-test-{}", std::process::id()));
        fs::create_dir(&directory)?;
        let path = directory.join("sample.process.json");
        let freeze = json!({"sourceCommit": "commit", "environment": {"stableEnvironmentSha256": "machine-hash"}, "executables": {"binary": {"sha256": "binary-hash", "path": "frozen.exe"}}});
        let mut row = json!({"name": "sample", "executable": "frozen.exe", "pid": 123, "exitCode": 0, "sourceCommit": "commit", "freezeSha256": "freeze-hash", "binarySha256": "binary-hash", "stableEnvironmentSha256": "machine-hash", "powerMatchesFreeze": true, "peakWorkingSetBytes": 100, "processCommitPeakBytes": 200});
        fs::write(&path, serde_json::to_vec(&row)?)?;
        assert!(process(&directory, &freeze, "freeze-hash", "sample", "binary").is_ok());
        row["name"] = json!("another-scale");
        fs::write(&path, serde_json::to_vec(&row)?)?;
        assert!(process(&directory, &freeze, "freeze-hash", "sample", "binary").is_err());
        row["name"] = json!("sample");
        row["exitCode"] = json!(1);
        fs::write(&path, serde_json::to_vec(&row)?)?;
        assert!(process(&directory, &freeze, "freeze-hash", "sample", "binary").is_err());
        row["exitCode"] = json!(0);
        row["powerMatchesFreeze"] = json!(false);
        fs::write(&path, serde_json::to_vec(&row)?)?;
        assert!(process(&directory, &freeze, "freeze-hash", "sample", "binary").is_err());
        row["powerMatchesFreeze"] = json!(true);
        row["stableEnvironmentSha256"] = json!("different-machine");
        fs::write(&path, serde_json::to_vec(&row)?)?;
        assert!(process(&directory, &freeze, "freeze-hash", "sample", "binary").is_err());
        row["stableEnvironmentSha256"] = json!("machine-hash");
        row["binarySha256"] = json!("stale-binary");
        fs::write(&path, serde_json::to_vec(&row)?)?;
        assert!(process(&directory, &freeze, "freeze-hash", "sample", "binary").is_err());
        fs::remove_file(path)?;
        fs::remove_dir(directory)?;
        Ok(())
    }

    #[test]
    fn result_pid_and_recorded_input_output_must_belong_to_the_same_run() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("junction-binding-test-{}", std::process::id()));
        fs::create_dir(&directory)?;
        for name in [
            "network.lfca",
            "grid.catalog.toml",
            "result.json",
            "other.json",
        ] {
            fs::write(directory.join(name), [])?;
        }
        let output = directory.join("result.json");
        let case = json!({"directory": directory, "prepared": {"input": {"vehicles": 10000, "warmup_ticks": 18012, "observation_ticks": 36024}}});
        let row = json!({"pid": 10});
        let original = json!({"pid": 10, "environmentOverrides": {}, "arguments": [directory.join("network.lfca"), directory.join("grid.catalog.toml"), "10000", "18012", "36024", "catchup", output]});
        assert!(bind_integrated_result(&original, &row, &case, &output).is_ok());
        for (index, replacement) in [
            (2, json!("100000")),
            (5, json!("normal")),
            (6, json!(directory.join("other.json"))),
        ] {
            let mut altered = original.clone();
            altered["arguments"][index] = replacement;
            assert!(bind_integrated_result(&altered, &row, &case, &output).is_err());
        }
        assert!(bind_integrated_result(&original, &json!({"pid": 11}), &case, &output).is_err());
        for name in [
            "network.lfca",
            "grid.catalog.toml",
            "result.json",
            "other.json",
        ] {
            fs::remove_file(directory.join(name))?;
        }
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

    #[test]
    fn prepared_snapshot_must_exist_and_match_its_frozen_digest() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("junction-snapshot-test-{}", std::process::id()));
        fs::create_dir(&directory)?;
        let snapshot = directory.join("prepared.initial.lfrs");
        let prepared = directory.join("prepared.json");
        fs::write(&snapshot, b"snapshot fixture")?;
        fs::write(&prepared, b"preparation fixture")?;
        let original = json!({"directory":directory,"prepared":{"snapshot_sha256":digest(&snapshot)?},
            "files":{"prepared.initial.lfrs":digest(&snapshot)?,"prepared.json":digest(&prepared)?}});
        authenticate_case(&original)?;
        let mut missing_manifest_entry = original.clone();
        missing_manifest_entry["files"]
            .as_object_mut()
            .unwrap()
            .remove("prepared.initial.lfrs");
        assert!(authenticate_case(&missing_manifest_entry).is_err());
        fs::write(&snapshot, b"corrupted snapshot")?;
        assert!(authenticate_case(&original).is_err());
        fs::remove_file(snapshot)?;
        assert!(authenticate_case(&original).is_err());
        fs::remove_file(prepared)?;
        fs::remove_dir(directory)?;
        Ok(())
    }

    #[test]
    fn transient_resource_acquisitions_do_not_prove_sustained_holds() -> Result<()> {
        let original = json!({"resource_loads":{"waiting_vehicle_ticks":100,"reservation_vehicle_ticks":100,
            "waiting_zones_with_repeated_requests":1,"vehicles_with_repeated_conflict_requests":1,
            "longest_observed_waiting_hold_ticks":2,"longest_reservation_age_ticks":2}});
        resource_loads(&original)?;
        for key in [
            "longest_observed_waiting_hold_ticks",
            "longest_reservation_age_ticks",
        ] {
            let mut transient = original.clone();
            transient["resource_loads"][key] = json!(1);
            assert!(resource_loads(&transient).is_err());
        }
        Ok(())
    }

    #[test]
    fn fidelity_rejects_missing_incomplete_or_violating_observations() -> Result<()> {
        let original = json!({"input":{"warmup_ticks":8,"observation_ticks":16,"vehicles":10},
            "counts":{"frames":12,"pose_rows_per_frame":10,"transform_rows_per_frame":10},
            "presentation_validation":{"checked_pose_rows":120,"checked_transform_rows":120,"violations":0},
            "validation":{"schema":"junction-scale-validation-v1","trajectory_sha256":"a".repeat(64),"domain_batches_sha256":"b".repeat(64),"checked_ticks":24,"checked_vehicle_rows":240,
                "checked_gate_crossings":1,"checked_events":1,"checked_observation_ticks":24,"failure":null,"violations":{
                    "overlap":0,"minimum_gap":0,"signal_stop_line":0,"numeric_geometry":0,"identity_route_lifecycle":0,
                    "parking_binding":0,"signal_authority":0,"tick_time":0,"event_order":0,"event_causality":0,"conflict_exclusivity":0,"observation_projection":0}}});
        fidelity(&original)?;
        for key in ["checked_pose_rows", "checked_transform_rows", "violations"] {
            let mut invalid = original.clone();
            invalid["presentation_validation"][key] = json!(1);
            assert!(fidelity(&invalid).is_err());
        }
        for kind in original["validation"]["violations"]
            .as_object()
            .unwrap()
            .keys()
        {
            let mut invalid = original.clone();
            invalid["validation"]["violations"][kind] = json!(1);
            assert!(fidelity(&invalid).is_err());
        }
        let mut incomplete = original.clone();
        incomplete["validation"]["checked_ticks"] = json!(23);
        assert!(fidelity(&incomplete).is_err());
        incomplete = original.clone();
        incomplete["validation"]["checked_observation_ticks"] = json!(23);
        assert!(fidelity(&incomplete).is_err());
        incomplete["validation"] = Value::Null;
        assert!(fidelity(&incomplete).is_err());
        Ok(())
    }
}
