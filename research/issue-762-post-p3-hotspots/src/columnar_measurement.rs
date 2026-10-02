//! #814 测量前置工具；独占阶段闭合、同窗计划和硬件计数校验不改变生产 Runtime。
#[allow(dead_code)]
mod io;

// 编译冻结导出所用的模板，避免 include_str! 隐藏语法或类型错误。
#[cfg(test)]
#[allow(dead_code)]
#[path = "columnar_perf_window.rs"]
mod perf_window;

use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const BASE: &str = "cbfbb14a714d819cd5e608a1b759575d378f3d53";
const WORKERS: [u64; 5] = [1, 2, 4, 8, 16];
const NAMES: [&str; 16] = [
    "Preflight",
    "Occupancy",
    "WaitingPrepareSelf",
    "ConflictPrepareSelf",
    "MotionLoopSelf",
    "WaitingFinalize",
    "Signals",
    "ConflictFinalize",
    "WaitingOutputs",
    "Commit",
    "Frontier",
    "P4",
    "P5DispatchFullJoin",
    "P5Consume",
    "WaitingPreview",
    "P6Validation",
];

fn need(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned().into())
    }
}

fn repository_root() -> Result<PathBuf> {
    Ok(PathBuf::from(io::git(
        &std::env::current_dir()?,
        &["rev-parse", "--show-toplevel"],
    )?)
    .canonicalize()?)
}

fn numbers<const N: usize>(row: &Value, key: &str) -> Result<[u64; N]> {
    let values = row[key].as_array().ok_or("missing timing array")?;
    need(values.len() == N, "timing array length")?;
    let mut result = [0; N];
    for (output, value) in result.iter_mut().zip(values) {
        *output = value.as_u64().ok_or("invalid timing value")?;
    }
    Ok(result)
}

fn exclusive(row: &Value) -> Result<[u64; 17]> {
    let whole = row["step_ns"].as_u64().ok_or("missing step time")?;
    let inclusive = numbers::<16>(row, "stages_ns")?;
    let calls = numbers::<16>(row, "calls")?;
    need(
        whole > 0 && calls[..15].iter().all(|&n| n == 1) && calls[15] <= 1,
        "incomplete or repeated coordinator stages",
    )?;
    need(
        calls[15] != 0 || inclusive[15] == 0,
        "unattributed validation time",
    )?;
    let mut out = [0; 17];
    out[..16].copy_from_slice(&inclusive);
    for (parent, children) in [(2, &[14][..]), (3, &[10, 11][..]), (4, &[12, 13][..])] {
        let sum = children
            .iter()
            .try_fold(0_u64, |sum, &i| sum.checked_add(inclusive[i]))
            .ok_or("nested timing overflow")?;
        out[parent] = inclusive[parent]
            .checked_sub(sum)
            .ok_or("nested stages exceed parent")?;
    }
    let measured = out[..16]
        .iter()
        .try_fold(0_u64, |sum, &n| sum.checked_add(n))
        .ok_or("exclusive timing overflow")?;
    out[16] = whole
        .checked_sub(measured)
        .ok_or("exclusive stages exceed whole step")?;
    need(
        out.iter().map(|&n| u128::from(n)).sum::<u128>() == u128::from(whole),
        "exclusive stages do not close",
    )?;
    Ok(out)
}

fn stage_table(trace: &Path) -> Result<Value> {
    let text = fs::read_to_string(trace)?;
    let rows: Vec<Value> = text
        .lines()
        .filter_map(|line| line.strip_prefix("LF762 "))
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    need(
        rows.len() == 256,
        "requires complete 256-tick diagnostic window",
    )?;
    let mut sums = [0_u128; 17];
    let mut whole = 0_u128;
    let mut validation_calls = 0;
    for (i, row) in rows.iter().enumerate() {
        need(row["tick"] == i + 1, "missing or reordered tick")?;
        for (sum, duration) in sums.iter_mut().zip(exclusive(row)?) {
            *sum += u128::from(duration);
        }
        whole += u128::from(row["step_ns"].as_u64().ok_or("step time")?);
        validation_calls += row["calls"][15].as_u64().ok_or("validation calls")?;
    }
    need(
        validation_calls == 0 || validation_calls == 256,
        "mixed stage schema",
    )?;
    let names: Vec<_> = NAMES
        .iter()
        .copied()
        .chain(std::iter::once("UnattributedFrameworkAndProbe"))
        .collect();
    Ok(
        json!({"schema":"lf814-exclusive-stage-table-v1", "analysis_only":true,
        "trace_sha256":io::sha(trace)?, "ticks":256, "integer_sum_equals_whole":true,
        "p6_separately_measured":validation_calls == 256,
        "whole_sum_ns":whole.to_string(), "whole_mean_ms":whole as f64 / 256.0 / 1e6,
        "stages":names.into_iter().zip(sums).map(|(name,n)| json!({"name":name,
            "sum_ns":n.to_string(),"mean_ms":n as f64 / 256.0 / 1e6,
            "percent":n as f64 * 100.0 / whole as f64})).collect::<Vec<_>>(),
        "limits":"diagnostic wall-clock only; residual is reported, not assigned; worker elapsed sums excluded; traffic and source verification required separately"}),
    )
}

fn logical_run(root: &Path) -> Result<Value> {
    let result = io::read_json(&root.join("result.json"))?;
    need(
        result["status"] == "probe-complete"
            && result["expected_ticks"] == 256
            && result["completed_ticks"] == 256
            && result["error"].is_null(),
        "incomplete traffic window",
    )?;
    for (name, digest) in result["files"].as_object().ok_or("result files")? {
        let path = io::safe_child(root, name)?;
        need(
            digest["sha256"] == io::sha(&path)? && digest["bytes"] == fs::metadata(path)?.len(),
            "traffic payload digest",
        )?;
    }
    let mut comparison = result.clone();
    let fields = comparison.as_object_mut().ok_or("result object")?;
    fields.remove("files");
    fields.remove("world_id");
    for name in [
        "commands.jsonl",
        "events.jsonl",
        "ticks.jsonl",
        "resolved-plan.toml",
    ] {
        let hash = result["files"][name]["sha256"]
            .as_str()
            .ok_or("required traffic payload")?;
        comparison[name] = json!(hash);
    }
    Ok(comparison)
}

fn traffic(reference: &Path, run: &Path) -> Result<Value> {
    need(
        logical_run(reference)? == logical_run(run)?,
        "state/decision/event/traffic disagreement",
    )?;
    Ok(
        json!({"schema":"lf814-measurement-traffic-check-v1", "traffic_verified":true,
        "reference_result_sha256":io::sha(&reference.join("result.json"))?,
        "run_result_sha256":io::sha(&run.join("result.json"))?, "ticks":256,
        "performance_acceptance":false, "limits":"payload and traffic equality only; source, binary, input, host and PMU windows require separate binding"}),
    )
}

fn plan(candidate: &str) -> Result<Value> {
    need(
        candidate.len() == 40 && candidate.bytes().all(|b| b.is_ascii_hexdigit()),
        "full candidate commit",
    )?;
    let mut runs = Vec::new();
    for workers in WORKERS {
        for (order_name, order) in [
            ("ABBA", ["aos", "columnar", "columnar", "aos"]),
            ("BAAB", ["columnar", "aos", "aos", "columnar"]),
        ] {
            for (position, arm) in order.into_iter().enumerate() {
                runs.push(
                    json!({"ordinal":runs.len()+1,"workers":workers,"order":order_name,
                    "position":position+1,"arm":arm}),
                );
            }
        }
    }
    Ok(
        json!({"schema":"lf814-pmu-scaling-plan-v1", "aos_source":BASE,"columnar_source":candidate,
        "columnar_backend":"avx2", "workers":WORKERS,"case":"MIXED-PEAK","seed":544,
        "dt_ms":33,"ticks":256,"warmup_ticks":0,"initial_total_vehicles":100_000,
        "initial_active_vehicles":75_000,"initial_parked_vehicles":25_000,
        "primary_metrics":["inherited_all_thread_cycles_per_tick","inherited_all_thread_instructions_per_tick"],
        "latency_metric":"public_step_wall_ns","fit_input":"wall latency, never sum of thread cycles",
        "affinity_policy":"same p distinct physical cores in both arms; record logical CPU list and socket/core topology",
        "frequency_policy":"verified fixed frequency or boost disabled; record actual state before and after",
        "perf_events":"{cycles,instructions}", "perf_delay":-1,"perf_control":"FIFO with acknowledgements around public step only",
        "controls_overhead":"separate empty-window calibration; report floor without silently subtracting it",
        "required_runtime_percent":100.0,"required_hardware_counters":true,
        "required_provenance":["complete source manifests","Rust and native toolchain","equal controlled build recipe",
            "actual binary hashes","input and plan hashes","CPU/power/affinity before and after","all traffic/checkpoint/event payloads"],
        "reject":["missing PMU","unsupported/not counted/zero counter","multiplexed or duplicate event",
            "frequency or affinity drift","incomplete window","traffic disagreement","missing same-window AoS"],
        "selective_reruns":0,"hardware_capture_complete":false,"runs":runs}),
    )
}

fn wpr_plan(candidate: &str) -> Result<Value> {
    let mut value = plan(candidate)?;
    let fields = value.as_object_mut().ok_or("plan object")?;
    for key in [
        "perf_events",
        "perf_delay",
        "perf_control",
        "required_runtime_percent",
    ] {
        fields.remove(key);
    }
    value["schema"] = json!("lf814-wpr-pmu-scaling-plan-v1");
    value["primary_metrics"] = json!([
        "process_thread_cycles_per_tick",
        "process_thread_instructions_per_tick"
    ]);
    value["wpr_profile"] = json!("research/issue-814-columnar-runtime/pmu-cswitch.wprp");
    value["wpr_counters"] = json!(["TotalCycles", "InstructionsRetired"]);
    value["counter_recording"] =
        json!("hardware values on CSwitch; count deltas, never sampled event frequency");
    value["counter_window"] = json!(
        "requires independently verified step begin/end timestamps and thread ownership; report uncertainty from CSwitch intervals crossing window edges; process lifetime includes setup and output and is not the step window"
    );
    value["controls_overhead"] = json!(
        "separate WPR-off and marker calibration; retain observed overhead and boundary uncertainty"
    );
    value["required_etw_event_loss"] = json!(0);
    value["reject"] = json!([
        "missing or unsupported PMU",
        "zero or missing counter payloads",
        "lost ETW events or buffers",
        "missing thread ownership or measurement boundaries",
        "counter allocation conflict",
        "frequency or affinity drift",
        "traffic disagreement",
        "missing same-window AoS"
    ]);
    value["wpr_window_capture_implemented"] = json!(false);
    value["wpr_decoder_implemented"] = json!(false);
    Ok(value)
}

fn perf_counters(text: &str) -> Result<Value> {
    let mut events = BTreeMap::new();
    for row in serde_json::Deserializer::from_str(text).into_iter::<Value>() {
        let row = row?;
        let event = row["event"].as_str().ok_or("perf event name")?;
        need(
            ["cycles", "instructions"].contains(&event),
            "unexpected perf event",
        )?;
        let counter = row["counter-value"]
            .as_str()
            .ok_or("perf counter value")?
            .parse::<u64>()?;
        let running = row["pcnt-running"]
            .as_f64()
            .or_else(|| row["pcnt-running"].as_str()?.parse().ok())
            .ok_or("perf running percentage")?;
        let runtime = row["event-runtime"]
            .as_u64()
            .or_else(|| row["event-runtime"].as_str()?.parse().ok())
            .ok_or("perf event runtime")?;
        need(
            counter > 0 && running == 100.0 && runtime > 0,
            "unsupported, zero, scaled or multiplexed hardware count",
        )?;
        need(
            events.insert(event.to_owned(), counter).is_none(),
            "duplicate or split perf event",
        )?;
    }
    need(events.len() == 2, "both cycles and instructions required")?;
    Ok(
        json!({"cycles":events["cycles"],"instructions":events["instructions"],
        "counter_scope":"inherited all threads; controlled windows include FIFO command overhead",
        "running_percent":100.0,"analysis_only":true}),
    )
}

fn fit(points: &Value) -> Result<Value> {
    let rows = points.as_array().ok_or("expected worker points")?;
    need(rows.len() == 5, "exactly five worker means required")?;
    let mut samples = BTreeMap::new();
    for row in rows {
        let workers = row["workers"].as_u64().ok_or("workers")?;
        let wall = row["wall_mean_ns"].as_f64().ok_or("wall latency")?;
        need(
            WORKERS.contains(&workers) && wall.is_finite() && wall > 0.0 && wall <= u64::MAX as f64,
            "invalid worker point",
        )?;
        need(
            samples.insert(workers, wall).is_none(),
            "duplicate worker point",
        )?;
    }
    let x_mean = WORKERS.iter().map(|&p| 1.0 / p as f64).sum::<f64>() / 5.0;
    let y_mean = samples.values().sum::<f64>() / 5.0;
    let variance = WORKERS
        .iter()
        .map(|&p| (1.0 / p as f64 - x_mean).powi(2))
        .sum::<f64>();
    let parallel = WORKERS
        .iter()
        .map(|&p| (1.0 / p as f64 - x_mean) * (samples[&p] - y_mean))
        .sum::<f64>()
        / variance;
    let serial = y_mean - parallel * x_mean;
    let residuals: Vec<_> = WORKERS.iter().map(|&p| {
        let predicted = serial + parallel / p as f64;
        json!({"workers":p,"observed_ns":samples[&p],"predicted_ns":predicted,"residual_ns":samples[&p]-predicted})
    }).collect();
    let sse = residuals
        .iter()
        .map(|r| r["residual_ns"].as_f64().unwrap_or_default().powi(2))
        .sum::<f64>();
    let sst = samples.values().map(|&y| (y - y_mean).powi(2)).sum::<f64>();
    let r2 = (sst > 0.0).then(|| 1.0 - sse / sst);
    let max_relative = residuals
        .iter()
        .map(|r| {
            r["residual_ns"].as_f64().unwrap_or_default().abs()
                / r["observed_ns"].as_f64().unwrap_or(1.0)
        })
        .fold(0.0_f64, f64::max);
    Ok(
        json!({"schema":"lf814-amdahl-fit-v1","model":"T(p)=S_effective+P/p","input":"wall-clock latency",
        "serial_effective_ns":serial,"parallel_ns":parallel,"r_squared":r2,"max_relative_residual":max_relative,
        "model_plausible":serial >= 0.0 && parallel > 0.0 && r2.is_some_and(|r| r >= 0.95) && max_relative <= 0.05,
        "points":residuals,"analysis_only":true,"measured_serial_time_certified":false,
        "limits":"effective intercept includes scheduling, memory and cache effects; compare independent coordinator timings and replicated confidence intervals before interpreting it as serial work"}),
    )
}

fn preflight() -> Value {
    let pmus: Vec<_> = ["cpu", "cpu_core", "cpu_atom"]
        .into_iter()
        .filter(|pmu| {
            Path::new("/sys/bus/event_source/devices")
                .join(pmu)
                .exists()
        })
        .collect();
    let frequency = Path::new("/sys/devices/system/cpu/cpufreq").exists();
    let perf = Command::new("perf")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
    json!({"schema":"lf814-pmu-preflight-v1","backend":"linux-perf","os":std::env::consts::OS,"cpu_pmus":pmus,
        "cpufreq_directory":frequency,"perf_version":perf,"hardware_capture_complete":false,
        "ready":false,"reason":"inventory only; must prove positive grouped hardware counts, physical affinity and power state on the capture host"})
}

fn export_window(commit: &str, root: &Path) -> Result<()> {
    need(
        commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "full source commit required",
    )?;
    let repo = repository_root()?;
    need(
        io::git(&repo, &["cat-file", "-t", commit])? == "commit",
        "source commit missing",
    )?;
    io::outside(&repo, root)?;
    io::ensure_new(root)?;
    fs::create_dir(root)?;
    let archive = root.join("original-source.tar");
    let status = Command::new("git")
        .args(["archive", "--format=tar", commit, "--output"])
        .arg(&archive)
        .status()?;
    need(status.success(), "source archive failed")?;
    let source = root.join("source");
    fs::create_dir(&source)?;
    need(
        Command::new("tar")
            .arg("-xf")
            .arg(&archive)
            .arg("-C")
            .arg(&source)
            .status()?
            .success(),
        "source extraction failed",
    )?;
    let host = source.join("tools/laneflow-urban-harness/src/host.rs");
    let text = fs::read_to_string(&host)?.replace("\r\n", "\n");
    let clock = "                let started = Instant::now();\n                let result = world.step(input);\n                let elapsed = nanos(started.elapsed());";
    need(
        text.matches(clock).count() == 1,
        "public step clock anchor differs",
    )?;
    let instrumented = format!(
        "                let calibration = lf814_perf::before_step()?;\n{clock}\n                lf814_perf::after_step(calibration)?;\n                eprintln!(\"LF814_PMU {{{{\\\"tick\\\":{{}},\\\"step_ns\\\":{{}},\\\"calibration\\\":{{}}}}}}\", world.tick_index(), elapsed, calibration);"
    );
    // format! 只展开 clock；导出源中的 JSON 和格式参数各保留一层 Rust 转义。
    fs::write(
        &host,
        format!(
            "{}\n#[path = \"columnar_perf_window.rs\"]\nmod lf814_perf;\n",
            text.replacen(clock, &instrumented, 1)
        ),
    )?;
    fs::write(
        source.join("tools/laneflow-urban-harness/src/columnar_perf_window.rs"),
        include_str!("columnar_perf_window.rs"),
    )?;
    io::write_new(
        &root.join("source-index.json"),
        &json!({"schema":"lf814-perf-window-export-v1",
        "source_git_head":commit,"source_git_tree":io::git(&repo,&["rev-parse",&format!("{commit}^{{tree}}")])?,
        "original_archive_sha256":io::sha(&archive)?,"source_files":io::source_index(&source)?,
        "exporter_sha256":io::sha(&std::env::current_exe()?)?,"production_source_changed":false,
        "limits":"export only; Linux controlled build and PMU capture are not verified by this record"}),
    )
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let command = args
        .first()
        .map(String::as_str)
        .ok_or("missing measurement command")?;
    if command == "export-perf" && args.len() == 3 {
        return export_window(&args[1], Path::new(&args[2]));
    }
    let (output, value) = match (command,args.len()) {
        ("stages",3) => (Path::new(&args[2]),stage_table(Path::new(&args[1]))?),
        ("plan" | "plan-wpr",3) => {
            let repo = repository_root()?;
            need(io::git(&repo,&["cat-file","-t",&args[1]])? == "commit", "candidate commit missing")?;
            let value = if command == "plan-wpr" { wpr_plan(&args[1])? } else { plan(&args[1])? };
            (Path::new(&args[2]),value)
        },
        ("perf",4) => {
            let ticks: u64 = args[2].parse()?;
            need(ticks == 256,"complete frozen window required")?;
            let path = Path::new(&args[1]);
            let mut value = perf_counters(&fs::read_to_string(path)?)?;
            value["raw_sha256"] = json!(io::sha(path)?);
            value["cycles_per_tick"] = json!(value["cycles"].as_u64().ok_or("cycles")? as f64 / ticks as f64);
            value["instructions_per_tick"] = json!(value["instructions"].as_u64().ok_or("instructions")? as f64 / ticks as f64);
            value["ticks"] = json!(ticks);
            (Path::new(&args[3]),value)
        },
        ("fit",3) => (Path::new(&args[2]),fit(&io::read_json(Path::new(&args[1]))?)?),
        ("traffic",4) => (Path::new(&args[3]),traffic(Path::new(&args[1]),Path::new(&args[2]))?),
        ("preflight",2) => (Path::new(&args[1]),preflight()),
        _ => return Err("stages <trace.stderr> <new-json> | plan|plan-wpr <full-candidate-sha> <new-json> | perf <perf-json> 256 <new-json> | fit <worker-means-json> <new-json> | traffic <reference-dir> <run-dir> <new-json> | preflight <new-json> | export-perf <full-source-sha> <new-root>".into()),
    };
    io::outside(&repository_root()?, output)?;
    io::write_new(output, &value)
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
    fn row() -> Value {
        json!({"step_ns":150,"stages_ns":[10,10,20,30,30,10,10,10,10,5,10,10,10,10,10,0],
            "calls":[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,0]})
    }
    #[test]
    fn exclusive_clock_closes_without_counting_nested_or_worker_time_twice() {
        let value = exclusive(&row()).unwrap();
        assert_eq!(value.iter().sum::<u64>(), 150);
        assert_eq!((value[2], value[3], value[4], value[16]), (10, 10, 10, 5));
    }
    #[test]
    fn invalid_clock_and_call_coverage_fail_closed() {
        let mut value = row();
        value["stages_ns"][14] = json!(21);
        assert!(exclusive(&value).is_err());
        value = row();
        value["step_ns"] = json!(140);
        assert!(exclusive(&value).is_err());
        value = row();
        value["calls"][5] = json!(0);
        assert!(exclusive(&value).is_err());
        value = row();
        value["stages_ns"][15] = json!(2);
        assert!(exclusive(&value).is_err());
    }
    #[test]
    fn validation_is_an_independent_top_level_span() {
        let mut value = row();
        value["calls"][15] = json!(1);
        value["stages_ns"][15] = json!(3);
        let out = exclusive(&value).unwrap();
        assert_eq!((out[15], out[16]), (3, 2));
    }
    #[test]
    fn every_worker_point_has_two_balanced_same_window_aos_orders() {
        let p = plan("0123456789012345678901234567890123456789").unwrap();
        let runs = p["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 40);
        for workers in WORKERS {
            for arm in ["aos", "columnar"] {
                assert_eq!(
                    runs.iter()
                        .filter(|r| r["workers"] == workers && r["arm"] == arm)
                        .count(),
                    4
                );
            }
        }
        for group in runs.as_chunks::<4>().0 {
            assert_eq!(group[0]["arm"], group[3]["arm"]);
            assert_eq!(group[1]["arm"], group[2]["arm"]);
        }
        let wpr = wpr_plan("0123456789012345678901234567890123456789").unwrap();
        assert_eq!(wpr["runs"], p["runs"]);
        assert!(wpr.get("perf_events").is_none());
        assert_eq!(wpr["wpr_window_capture_implemented"], false);
        assert_eq!(wpr["wpr_decoder_implemented"], false);
    }
    fn counters() -> String {
        ["cycles", "instructions"]
            .into_iter()
            .map(|event| {
                json!({"event":event,"counter-value":"1000",
            "event-runtime":1_000,"pcnt-running":100.0})
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn missing_unsupported_multiplexed_and_duplicate_counters_are_rejected() {
        let good = counters();
        assert!(perf_counters(&good).is_ok());
        for invalid in [
            good.replace("1000", "0"),
            good.replace("1000", "<not supported>"),
            good.replace("100.0", "99.0"),
            format!("{good}\n{good}"),
            good.lines().next().unwrap().to_owned(),
            good.replace("1000", "18446744073709551616"),
            good.replace("1000", "1000.5"),
        ] {
            assert!(perf_counters(&invalid).is_err());
        }
        let large = perf_counters(&good.replace("1000", "9007199254740993")).unwrap();
        assert_eq!(large["cycles"].as_u64(), Some(9_007_199_254_740_993));
    }
    #[test]
    fn fit_recovers_effective_intercept_and_rejects_bad_amdahl_interpretation() {
        let points = json!(WORKERS.map(|p| json!({"workers":p,"wall_mean_ns":10.0+80.0/p as f64})));
        let result = fit(&points).unwrap();
        assert!((result["serial_effective_ns"].as_f64().unwrap() - 10.0).abs() < 1e-10);
        assert_eq!(result["model_plausible"], true);
        let slow = json!(WORKERS.map(|p| json!({"workers":p,"wall_mean_ns":p as f64})));
        assert_eq!(fit(&slow).unwrap()["model_plausible"], false);
        let duplicate = json!(vec![json!({"workers":1,"wall_mean_ns":10.0}); 5]);
        assert!(fit(&duplicate).is_err());
        assert!(fit(&json!([{"workers":1,"cycles":10}])).is_err());
    }
}
