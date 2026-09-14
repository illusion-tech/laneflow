//! 汇总有界可运行证据；预算比较只报告，数据完整性错误仍阻止输出。
use std::{error::Error, fs, io::Write, path::Path};

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

fn positive_real(value: &Value) -> Result<f64> {
    value
        .as_f64()
        .filter(|n| n.is_finite() && *n > 0.0)
        .ok_or_else(|| "expected positive finite duration".into())
}

fn text(value: &Value) -> Result<&str> {
    value
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "expected nonempty string".into())
}

fn process(directory: &Path, freeze: &Value, freeze_hash: &str, name: &str) -> Result<Value> {
    let row = load(&directory.join(format!("{name}.process.json")))?;
    let binary = &freeze["executables"]["junction_scale_render"];
    require(
        row["schema"] == "junction-scale-process-v2",
        "invalid process schema",
    )?;
    require(text(&row["name"])? == name, "process record name mismatch")?;
    require(
        text(&row["executable"])? == text(&binary["path"])?,
        "process executable mismatch",
    )?;
    require(number(&row["pid"])? > 0, "missing process identity")?;
    text(&row["startedUtc"])?;
    require(
        row["exitCode"] == 0 && row["timedOut"] == false,
        "failed or timed-out process",
    )?;
    require(
        text(&row["sourceCommit"])? == text(&freeze["sourceCommit"])?,
        "source commit mismatch",
    )?;
    require(
        text(&row["freezeSha256"])? == freeze_hash,
        "freeze hash mismatch",
    )?;
    require(
        text(&row["binarySha256"])? == text(&binary["sha256"])?,
        "binary hash mismatch",
    )?;
    let limit = number(&row["wallLimitMilliseconds"])?;
    require(
        (1..=600_000).contains(&limit)
            && positive_real(&row["elapsedSeconds"])? * 1000.0 <= limit as f64,
        "process exceeded wall-clock limit",
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
    require(arguments.len() == 8, "invalid integrated arguments")?;
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
    require(
        text(&arguments[7])? == number(&process["wallLimitMilliseconds"])?.to_string(),
        "process wall limit argument mismatch",
    )?;
    require(
        arguments[5] == "catchup" && result["frame_mode"] == "catchup",
        "process frame mode mismatch",
    )?;
    require(
        process["environmentOverrides"] == json!({}),
        "unexpected integrated environment overrides",
    )
}

fn bounded_window(result: &Value, process: &Value) -> Result<u64> {
    require(
        result["schema"] == "junction-scale-evidence-v2",
        "invalid result schema",
    )?;
    require(
        result["input"]["warmup_ticks"] == 0
            && result["input"]["observation_ticks"] == 4096
            && result["input"]["fixed_delta_ms"] == 16,
        "invalid bounded input window",
    )?;
    let run = &result["run"];
    let ticks = number(&run["completed_ticks"])?;
    require(
        (1..=4096).contains(&ticks)
            && run["requested_ticks"] == 4096
            && number(&result["counts"]["observed_ticks"])? == ticks
            && number(&run["simulated_milliseconds"])? == ticks * 16,
        "invalid actual tick count",
    )?;
    let limit = number(&run["wall_limit_ms"])?;
    require(
        (1..=600_000).contains(&limit) && number(&process["wallLimitMilliseconds"])? == limit,
        "result wall limit mismatch",
    )?;
    let measurement_limit = limit - 10_000.min(limit / 10);
    let elapsed = number(&run["measurement_elapsed_ms"])?;
    require(
        number(&run["measurement_limit_ms"])? == measurement_limit
            && elapsed as f64 <= positive_real(&process["elapsedSeconds"])? * 1000.0 + 1.0,
        "invalid measurement duration",
    )?;
    require(
        match text(&run["stop_reason"])? {
            "tick-limit" => ticks == 4096 && elapsed < measurement_limit,
            "time-limit" => elapsed >= measurement_limit,
            _ => false,
        },
        "stop reason does not match actual window",
    )?;
    Ok(ticks)
}

fn fidelity(result: &Value, ticks: u64) -> Result<()> {
    let presentation = &result["presentation_validation"];
    let frames = number(&result["counts"]["frames"])?;
    require(
        frames > 0
            && number(&presentation["violations"])? == 0
            && Some(number(&presentation["checked_pose_rows"])?)
                == frames.checked_mul(number(&result["counts"]["pose_rows_per_frame"])?)
            && Some(number(&presentation["checked_transform_rows"])?)
                == frames.checked_mul(number(&result["counts"]["transform_rows_per_frame"])?),
        "missing, incomplete or failing presentation validation",
    )?;
    let validation = &result["validation"];
    require(
        validation["schema"] == "junction-scale-validation-v1",
        "missing fidelity validation",
    )?;
    for key in ["trajectory_sha256", "domain_batches_sha256"] {
        require(
            text(&validation[key])?.len() == 64,
            "missing all-tick digest",
        )?;
    }
    require(
        number(&validation["checked_ticks"])? == ticks
            && number(&validation["checked_observation_ticks"])? == ticks
            && Some(number(&validation["checked_vehicle_rows"])?)
                == ticks.checked_mul(number(&result["input"]["vehicles"])?),
        "incomplete per-tick fidelity validation",
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

fn samples(result: &Value, ticks: u64) -> Result<()> {
    let raw = &result["samples_ns"];
    let frames = number(&result["counts"]["frames"])?;
    for (key, expected) in [
        ("tick_and_driver", ticks),
        ("domain_observation", ticks),
        ("evidence_collection", ticks),
        ("pose_extraction", frames),
        ("mapping_transform_apply", frames),
        ("laneflow_frame_without_evidence", frames),
        ("integrated_frame_with_evidence", frames),
        ("renderer_submit_and_gpu_wait", frames),
        ("presentation_validation", frames),
        ("frame_step_counts", frames),
        ("frame_input_quanta", frames),
        ("frame_backlog_quanta", frames),
    ] {
        let values = raw[key].as_array().ok_or("missing raw samples")?;
        require(values.len() as u64 == expected, "incomplete raw samples")?;
        for value in values {
            number(value)?;
        }
    }
    let steps = raw["frame_step_counts"]
        .as_array()
        .ok_or("missing frame steps")?;
    let mut actual_ticks = 0;
    let mut backlog = 0;
    for (index, step) in steps.iter().enumerate() {
        let input = number(&raw["frame_input_quanta"][index])?;
        let step = number(step)?;
        require(
            input <= 4 && step == (backlog + input).min(2),
            "invalid frame step count",
        )?;
        backlog = backlog + input - step;
        require(
            backlog <= 2 && number(&raw["frame_backlog_quanta"][index])? == backlog,
            "invalid frame backlog",
        )?;
        actual_ticks += step;
    }
    require(
        actual_ticks == ticks && number(&result["counts"]["remaining_backlog_quanta"])? == backlog,
        "frame/tick alignment mismatch",
    )?;
    for (class, count) in [("zero_step", 0), ("one_step", 1), ("two_step", 2)] {
        let n = steps
            .iter()
            .filter(|step| step.as_u64() == Some(count))
            .count() as u64;
        for key in [
            "integrated_frame_with_evidence",
            "laneflow_frame_without_evidence",
            "spatial_adapter",
            "renderer_submit_and_gpu_wait",
        ] {
            let timing = &result["frame_classes"][class][key];
            require(
                if n == 0 {
                    timing.is_null()
                } else {
                    number(&timing["count"])? == n
                },
                "frame class sample count mismatch",
            )?;
        }
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

fn batch_window(batch: &Value, freeze: &Value, freeze_hash: &str) -> Result<()> {
    require(
        batch["schema"] == "junction-scale-batch-v1"
            && batch["status"] == "completed"
            && batch.get("error").is_some_and(Value::is_null)
            && batch["completedScales"] == json!([10000, 100000]),
        "incomplete or failed batch",
    )?;
    require(
        text(&batch["sourceCommit"])? == text(&freeze["sourceCommit"])?
            && text(&batch["freezeSha256"])? == freeze_hash,
        "batch source mismatch",
    )?;
    let limit = number(&batch["wallLimitMilliseconds"])?;
    require(
        (1..=600_000).contains(&limit)
            && positive_real(&batch["elapsedMilliseconds"])? <= limit as f64,
        "batch exceeded wall-clock limit",
    )
}

fn budget_comparison(timing: &Value, p95: u64, quantum: Option<u64>) -> Result<Value> {
    if timing.is_null() {
        return Ok(Value::Null);
    }
    let p99 = p95 * 3 / 2;
    let maximum = quantum.map_or(p95 * 2, |quantum| (p95 * 2).min(quantum));
    Ok(
        json!({"p95_budget_ns": p95, "p99_budget_ns": p99, "max_budget_ns": maximum,
        "p95_within_budget": number(&timing["p95"])? <= p95,
        "p99_within_budget": number(&timing["p99"])? <= p99, "max_within_budget": number(&timing["max"])? <= maximum}),
    )
}

fn analyze(directory: &Path) -> Result<Value> {
    let freeze_path = directory.join("freeze.json");
    let freeze = load(&freeze_path)?;
    let freeze_hash = digest(&freeze_path)?;
    let protocol = &freeze["protocol"];
    require(
        freeze["schema"] == "junction-scale-freeze-v2"
            && protocol["rounds"] == 1
            && protocol["maxTicks"] == 4096
            && protocol["warmupTicks"] == 0
            && protocol["observationTicks"] == 4096
            && protocol["batchWallLimitMilliseconds"] == 600000
            && protocol["fixedDeltaMs"] == 16
            && protocol["maxCatchUpSteps"] == 2
            && protocol["frameInputQuanta"] == json!([0, 1, 2, 4, 0]),
        "noncanonical bounded protocol",
    )?;
    for binary in freeze["executables"]
        .as_object()
        .ok_or("missing frozen binaries")?
        .values()
    {
        require(
            digest(Path::new(text(&binary["path"])?))? == text(&binary["sha256"])?,
            "frozen binary changed",
        )?;
    }
    let batch = load(&directory.join("batch.json"))?;
    batch_window(&batch, &freeze, &freeze_hash)?;
    let mut output = json!({"schema":"junction-scale-summary-v2", "source_commit": text(&freeze["sourceCommit"])?, "freeze_sha256": freeze_hash,
        "acceptance":"bounded runnable evidence", "certification":"NOT_REQUESTED", "batch":batch, "environment":batch["environment"],
        "scope":"one cold-start process per scale; actual observed window only; budgets are descriptive, not acceptance gates",
        "not_measured":["steady-state allocation", "component retained/scratch ledgers", "three-round repeatability", "product hardware certification"], "scales":{}});
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
    let mut process_elapsed_ms = 0.0;
    for case in cases {
        let count = number(&case["vehicles"])?;
        let expected_cells = if count == 10000 { 32 } else { 320 };
        require(
            case["cells"] == expected_cells && case["prepared"]["input"]["cells"] == expected_cells,
            "incorrect cell count",
        )?;
        authenticate_case(case)?;
        let name = format!("render-{count}-round-1");
        let process = process(directory, &freeze, &freeze_hash, &name)?;
        process_elapsed_ms += positive_real(&process["elapsedSeconds"])? * 1000.0;
        let path = directory.join(format!("{name}.json"));
        let row = load(&path)?;
        bind_integrated_result(&process, &row, case, &path)?;
        let ticks = bounded_window(&row, &process)?;
        fidelity(&row, ticks)?;
        samples(&row, ticks)?;
        require(
            row["input"] == case["prepared"]["input"] && row["input"]["vehicles"] == count,
            "input mismatch",
        )?;
        require(
            text(&row["initial_state_digest"])? == text(&case["prepared"]["initial_state_digest"])?,
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
                && actual["presentable"] == count
                && actual["domain_vehicle_rows_per_tick"] == count
                && actual["pose_rows_per_frame"] == count,
            "incomplete population or consumption",
        )?;
        require(
            actual["transform_rows_per_frame"] == 10000
                && actual["minimum_renderer_visible_proxies"] == 10000
                && row["renderer"]["proxy_count"] == 10000,
            "incomplete presentation",
        )?;
        text(&row["renderer"]["adapter"]["backend"])?;
        same_file(
            &row["renderer"]["preview"],
            &directory.join(format!("{name}.png")),
        )?;
        let timings = &row["nanoseconds"];
        require(
            number(&timings["tick_and_driver"]["count"])? == ticks,
            "runtime sample count mismatch",
        )?;
        let classes = &row["frame_classes"];
        let integrated = &classes["one_step"]["laneflow_frame_without_evidence"];
        let tick_budget = if count == 10000 {
            2_000_000
        } else {
            16_000_000
        };
        output["scales"][count.to_string()] = json!({
            "status":"runnable", "run":row["run"], "counts":actual, "nanoseconds":timings, "frame_classes":classes,
            "resource_loads":row["resource_loads"], "validation":row["validation"], "presentation_validation":row["presentation_validation"],
            "renderer":row["renderer"], "process":process, "checkpoints":row["checkpoints"], "final_state_digest":row["final_state_digest"],
            "budget_comparisons":{"scope":"descriptive only; no product acceptance gate",
                "runtime":budget_comparison(&timings["tick_and_driver"], tick_budget, Some(16_000_000))?,
                "spatial_adapter_one_step":budget_comparison(&classes["one_step"]["spatial_adapter"], 4_000_000, None)?,
                "laneflow_one_step":if count == 10000 { budget_comparison(integrated, 6_000_000, None)? }
                    else if integrated.is_null() { Value::Null } else { json!({"observation_threshold_ns":16_667_000,
                        "p95_exceeds_observation_threshold":number(&integrated["p95"])? > 16_667_000, "product_tail_budget":null}) }},
            "missing_measurements":row["missing_measurements"]
        });
    }
    require(
        process_elapsed_ms <= positive_real(&batch["elapsedMilliseconds"])?,
        "process durations exceed batch duration",
    )?;
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

    fn partial() -> (Value, Value) {
        (
            json!({"schema":"junction-scale-evidence-v2", "input":{"warmup_ticks":0,"observation_ticks":4096,"fixed_delta_ms":16},
            "run":{"completed_ticks":120,"requested_ticks":4096,"simulated_milliseconds":1920,"wall_limit_ms":300000,
                "measurement_limit_ms":290000,"measurement_elapsed_ms":290050,"stop_reason":"time-limit"}, "counts":{"observed_ticks":120}}),
            json!({"wallLimitMilliseconds":300000,"elapsedSeconds":291.0}),
        )
    }

    #[test]
    fn partial_time_limit_is_valid_but_empty_overlong_or_mislabelled_windows_are_not() -> Result<()>
    {
        let (original, process) = partial();
        assert_eq!(bounded_window(&original, &process)?, 120);
        for (key, replacement) in [
            ("completed_ticks", json!(0)),
            ("completed_ticks", json!(4097)),
            ("stop_reason", json!("tick-limit")),
            ("measurement_elapsed_ms", json!(289999)),
            ("wall_limit_ms", json!(600001)),
        ] {
            let mut invalid = original.clone();
            invalid["run"][key] = replacement;
            assert!(bounded_window(&invalid, &process).is_err());
        }
        let mut complete = original;
        complete["run"]["completed_ticks"] = json!(4096);
        complete["run"]["simulated_milliseconds"] = json!(65536);
        complete["counts"]["observed_ticks"] = json!(4096);
        assert_eq!(bounded_window(&complete, &process)?, 4096);
        complete["run"]["stop_reason"] = json!("tick-limit");
        assert!(bounded_window(&complete, &process).is_err());
        complete["run"]["measurement_elapsed_ms"] = json!(289999);
        assert_eq!(bounded_window(&complete, &process)?, 4096);
        Ok(())
    }

    #[test]
    fn the_limit_applies_to_the_whole_batch() -> Result<()> {
        let freeze = json!({"sourceCommit":"commit"});
        let mut batch = json!({"schema":"junction-scale-batch-v1","sourceCommit":"commit","freezeSha256":"hash",
            "status":"completed","error":null,"completedScales":[10000,100000],"wallLimitMilliseconds":600000,"elapsedMilliseconds":598000.0});
        batch_window(&batch, &freeze, "hash")?;
        batch["elapsedMilliseconds"] = json!(600001);
        assert!(batch_window(&batch, &freeze, "hash").is_err());
        batch["elapsedMilliseconds"] = json!(598000);
        batch["status"] = json!("failed");
        assert!(batch_window(&batch, &freeze, "hash").is_err());
        Ok(())
    }

    #[test]
    fn failed_or_mismatched_process_evidence_is_rejected_in_release_too() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("junction-process-test-{}", std::process::id()));
        fs::create_dir(&directory)?;
        let path = directory.join("sample.process.json");
        let freeze = json!({"sourceCommit":"commit","executables":{"junction_scale_render":{"sha256":"binary-hash","path":"frozen.exe"}}});
        let original = json!({"schema":"junction-scale-process-v2","name":"sample","executable":"frozen.exe","pid":123,"startedUtc":"start",
            "exitCode":0,"timedOut":false,"elapsedSeconds":2.0,"wallLimitMilliseconds":3000,"sourceCommit":"commit","freezeSha256":"freeze-hash",
            "binarySha256":"binary-hash","powerMatchesFreeze":false,"stableEnvironmentSha256":"changed-but-recorded",
            "peakWorkingSetBytes":100,"processCommitPeakBytes":200});
        fs::write(&path, serde_json::to_vec(&original)?)?;
        process(&directory, &freeze, "freeze-hash", "sample")?;
        for (key, replacement) in [
            ("name", json!("another-scale")),
            ("exitCode", json!(1)),
            ("timedOut", json!(true)),
            ("elapsedSeconds", json!(3.1)),
            ("binarySha256", json!("stale")),
            ("freezeSha256", json!("wrong")),
        ] {
            let mut row = original.clone();
            row[key] = replacement;
            fs::write(&path, serde_json::to_vec(&row)?)?;
            assert!(process(&directory, &freeze, "freeze-hash", "sample").is_err());
        }
        fs::remove_file(path)?;
        fs::remove_dir(directory)?;
        Ok(())
    }

    #[test]
    fn result_pid_and_arguments_must_belong_to_the_same_run() -> Result<()> {
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
        let case = json!({"directory":directory,"prepared":{"input":{"vehicles":10000,"warmup_ticks":0,"observation_ticks":4096}}});
        let row = json!({"pid":10,"frame_mode":"catchup"});
        let original = json!({"pid":10,"environmentOverrides":{},"wallLimitMilliseconds":300000,
            "arguments":[directory.join("network.lfca"),directory.join("grid.catalog.toml"),"10000","0","4096","catchup",output,"300000"]});
        bind_integrated_result(&original, &row, &case, &output)?;
        for (index, replacement) in [
            (2, json!("100000")),
            (5, json!("normal")),
            (6, json!(directory.join("other.json"))),
            (7, json!("600000")),
        ] {
            let mut altered = original.clone();
            altered["arguments"][index] = replacement;
            assert!(bind_integrated_result(&altered, &row, &case, &output).is_err());
        }
        assert!(
            bind_integrated_result(
                &original,
                &json!({"pid":11,"frame_mode":"catchup"}),
                &case,
                &output
            )
            .is_err()
        );
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
        let mut missing = original.clone();
        missing["files"]
            .as_object_mut()
            .unwrap()
            .remove("prepared.initial.lfrs");
        assert!(authenticate_case(&missing).is_err());
        fs::write(&snapshot, b"corrupted snapshot")?;
        assert!(authenticate_case(&original).is_err());
        fs::remove_file(snapshot)?;
        assert!(authenticate_case(&original).is_err());
        fs::remove_file(prepared)?;
        fs::remove_dir(directory)?;
        Ok(())
    }

    #[test]
    fn fidelity_checks_actual_window_even_without_gate_transitions() -> Result<()> {
        let original = json!({"input":{"vehicles":10},"counts":{"frames":12,"pose_rows_per_frame":10,"transform_rows_per_frame":10},
            "presentation_validation":{"checked_pose_rows":120,"checked_transform_rows":120,"violations":0},
            "validation":{"schema":"junction-scale-validation-v1","trajectory_sha256":"a".repeat(64),"domain_batches_sha256":"b".repeat(64),
                "checked_ticks":16,"checked_vehicle_rows":160,"checked_gate_crossings":0,"checked_events":0,"checked_observation_ticks":16,"failure":null,
                "violations":{"overlap":0,"minimum_gap":0,"signal_stop_line":0,"numeric_geometry":0,"identity_route_lifecycle":0,
                    "parking_binding":0,"signal_authority":0,"tick_time":0,"event_order":0,"event_causality":0,"conflict_exclusivity":0,"observation_projection":0}}});
        fidelity(&original, 16)?;
        for key in ["checked_pose_rows", "checked_transform_rows", "violations"] {
            let mut invalid = original.clone();
            invalid["presentation_validation"][key] = json!(1);
            assert!(fidelity(&invalid, 16).is_err());
        }
        for kind in original["validation"]["violations"]
            .as_object()
            .unwrap()
            .keys()
        {
            let mut invalid = original.clone();
            invalid["validation"]["violations"][kind] = json!(1);
            assert!(fidelity(&invalid, 16).is_err());
        }
        for key in [
            "checked_ticks",
            "checked_observation_ticks",
            "checked_vehicle_rows",
        ] {
            let mut invalid = original.clone();
            invalid["validation"][key] = json!(1);
            assert!(fidelity(&invalid, 16).is_err());
        }
        assert!(fidelity(&original, 4096).is_err());
        Ok(())
    }

    #[test]
    fn actual_samples_preserve_unfinished_backlog_and_reject_incomplete_windows() -> Result<()> {
        let mut row = json!({"counts":{"frames":3,"remaining_backlog_quanta":2},"samples_ns":{},"frame_classes":{}});
        for key in [
            "tick_and_driver",
            "domain_observation",
            "evidence_collection",
        ] {
            row["samples_ns"][key] = json!([1, 2, 3]);
        }
        for key in [
            "pose_extraction",
            "mapping_transform_apply",
            "laneflow_frame_without_evidence",
            "integrated_frame_with_evidence",
            "renderer_submit_and_gpu_wait",
            "presentation_validation",
        ] {
            row["samples_ns"][key] = json!([1, 2, 3]);
        }
        row["samples_ns"]["frame_step_counts"] = json!([0, 1, 2]);
        row["samples_ns"]["frame_input_quanta"] = json!([0, 1, 4]);
        row["samples_ns"]["frame_backlog_quanta"] = json!([0, 0, 2]);
        for class in ["zero_step", "one_step", "two_step"] {
            for key in [
                "integrated_frame_with_evidence",
                "laneflow_frame_without_evidence",
                "spatial_adapter",
                "renderer_submit_and_gpu_wait",
            ] {
                row["frame_classes"][class][key] = json!({"count":1});
            }
        }
        samples(&row, 3)?;
        let mut lost_backlog = row.clone();
        lost_backlog["counts"]["remaining_backlog_quanta"] = json!(0);
        assert!(samples(&lost_backlog, 3).is_err());
        let mut missing = row.clone();
        missing["samples_ns"]["pose_extraction"] = json!([1, 2]);
        assert!(samples(&missing, 3).is_err());
        assert!(samples(&row, 4096).is_err());
        Ok(())
    }
}
