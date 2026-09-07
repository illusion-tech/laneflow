use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    Artifacts, Harness, ResolvedPlan, Result, TickRecord, UrbanCase, invalid, observe,
    runner::TileEvidence, sha256,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunResult {
    pub version: String,
    pub status: String,
    pub purpose: String,
    pub case: String,
    pub scale: String,
    pub network_revision: String,
    pub policy_id: String,
    pub world_id: u64,
    pub fixed_step_ms: u64,
    pub window: crate::Window,
    pub required_per_tile: BTreeMap<String, u64>,
    pub plan_digest: String,
    pub expected_ticks: u64,
    pub completed_ticks: u64,
    pub committed_world_tick: u64,
    pub error: Option<String>,
    pub checkpoints: BTreeMap<u64, String>,
    pub initial_counts: (usize, usize, usize),
    pub final_counts: (usize, usize, usize),
    pub tile_evidence: Vec<TileEvidence>,
    pub retry_reasons: BTreeMap<String, u64>,
    pub atomic_rejections: BTreeMap<String, u32>,
    pub replacements: u64,
    pub births: u64,
    pub removals: u64,
    pub pending_departures: usize,
    pub exhausted_departures: usize,
    pub files: BTreeMap<String, crate::artifacts::FileDigest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ComparedRun {
    pub execution_id: String,
    pub result: crate::artifacts::FileDigest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ComparisonReport {
    pub version: String,
    pub status: String,
    pub purpose: String,
    pub case: String,
    pub scale: String,
    pub plan_digest: String,
    pub completed_ticks: u64,
    pub left: ComparedRun,
    pub right: ComparedRun,
}

#[derive(Serialize)]
struct SampleSummary {
    samples: usize,
    min: u64,
    p50: u64,
    p95: u64,
    p99: u64,
    max: u64,
}

#[derive(Serialize)]
struct MemoryMeasurement {
    status: &'static str,
    method: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    peak_resident_bytes: Option<u64>,
}

#[derive(Serialize)]
struct Measurements {
    version: &'static str,
    execution_id: String,
    git_commit: String,
    git_status: String,
    rustc: String,
    cargo: String,
    target: String,
    build_parameters: &'static str,
    invocation: Vec<String>,
    os: &'static str,
    architecture: &'static str,
    hardware_role: String,
    power_role: String,
    workers: u32,
    timing_range: &'static str,
    command_ns: SampleSummary,
    traffic_world_step_ns: SampleSummary,
    observation_ns: SampleSummary,
    active: SampleSummary,
    intent: SampleSummary,
    memory: MemoryMeasurement,
    command_samples_ns: Vec<u64>,
    traffic_world_step_samples_ns: Vec<u64>,
    observation_samples_ns: Vec<u64>,
    active_samples: Vec<u64>,
    intent_samples: Vec<u64>,
}

#[derive(Deserialize)]
struct RetainedMeasurements {
    version: String,
    execution_id: String,
    command_samples_ns: Vec<u64>,
    traffic_world_step_samples_ns: Vec<u64>,
    observation_samples_ns: Vec<u64>,
    active_samples: Vec<u64>,
    intent_samples: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PerformanceRound {
    pub execution_id: String,
    pub result: crate::artifacts::FileDigest,
    pub measurements: crate::artifacts::FileDigest,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PerformanceComparisonReport {
    pub version: String,
    pub status: String,
    pub case: String,
    pub scale: String,
    pub plan_digest: String,
    pub rounds: Vec<PerformanceRound>,
    pub command_ns: BTreeMap<String, u64>,
    pub traffic_world_step_ns: BTreeMap<String, u64>,
    pub observation_ns: BTreeMap<String, u64>,
    pub active: BTreeMap<String, u64>,
    pub intent: BTreeMap<String, u64>,
}

impl ComparisonReport {
    /// Writes the comparison separately from the immutable per-run evidence.
    pub fn write(&self, path: &Path) -> Result<()> {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        let mut file = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut file, self)?;
        file.write_all(b"\n")?;
        file.flush()?;
        Ok(())
    }
}

impl PerformanceComparisonReport {
    pub fn write(&self, path: &Path) -> Result<()> {
        let mut file = BufWriter::new(
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)?,
        );
        file.write_all(toml::to_string_pretty(self)?.as_bytes())?;
        file.flush()?;
        Ok(())
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut file = BufWriter::new(File::create(path)?);
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

fn line(file: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *file, value)?;
    file.write_all(b"\n")?;
    Ok(())
}

/// Runs one fresh world. Controlled execution or validation failures retain the last committed tick.
/// Initialization errors return Err and may leave preparation files without a failed-run package.
/// Correctness/probe timing is diagnostic only. A formal performance plan emits measurements.
pub fn run_to_directory(
    artifacts: &Artifacts,
    plan: &ResolvedPlan,
    output: &Path,
) -> Result<RunResult> {
    plan.validate(artifacts)?;
    let performance_context = if plan.window.purpose == "performance" {
        Some((
            std::env::var("LANEFLOW_HARDWARE_ROLE")
                .map_err(|_| invalid("performance requires LANEFLOW_HARDWARE_ROLE"))?,
            std::env::var("LANEFLOW_POWER_ROLE")
                .map_err(|_| invalid("performance requires LANEFLOW_POWER_ROLE"))?,
        ))
    } else {
        None
    };
    fs::create_dir(output)?;
    let execution_id = new_execution_id()?;
    let plan_digest = plan.write(&output.join("resolved-plan.toml"))?;
    let started = Instant::now();
    let mut harness = Harness::install(artifacts, plan)?;
    let initial_counts = observe::counts(&harness)?;
    let mut result = RunResult {
        version: "urban-result-v3".into(),
        status: "failed".into(),
        purpose: plan.window.purpose.clone(),
        case: plan.case.clone(),
        scale: plan.scale.clone(),
        network_revision: artifacts.catalog.network_revision.clone(),
        policy_id: artifacts.catalog.policy_id.clone(),
        world_id: harness.world.world_id(),
        fixed_step_ms: plan.dt,
        window: plan.window.clone(),
        required_per_tile: plan.required_per_tile.clone(),
        plan_digest,
        expected_ticks: plan.window.end(),
        completed_ticks: 0,
        committed_world_tick: 0,
        error: None,
        checkpoints: BTreeMap::new(),
        initial_counts,
        final_counts: initial_counts,
        tile_evidence: Vec::new(),
        retry_reasons: BTreeMap::new(),
        atomic_rejections: BTreeMap::new(),
        replacements: 0,
        births: 0,
        removals: 0,
        pending_departures: 0,
        exhausted_departures: 0,
        files: BTreeMap::new(),
    };
    result.checkpoints.insert(0, harness.checkpoint()?);
    let mut ticks = BufWriter::new(File::create(output.join("ticks.jsonl"))?);
    let mut commands = BufWriter::new(File::create(output.join("commands.jsonl"))?);
    let mut events = BufWriter::new(File::create(output.join("events.jsonl"))?);
    let mut times = Vec::new();
    let mut command_times = Vec::new();
    let mut observation_times = Vec::new();
    let mut active_samples = Vec::new();
    let mut intent_samples = Vec::new();
    let run: Result<()> = (|| {
        for _ in 0..plan.window.end() {
            let record = harness.advance()?;
            line(&mut ticks, &record)?;
            for command in &harness.commands {
                line(&mut commands, command)?;
            }
            for event in &harness.events {
                line(&mut events, event)?;
            }
            times.push(harness.last_step_ns);
            if record.tick > plan.window.warm_up_ticks {
                command_times.push(harness.last_command_ns);
                observation_times.push(harness.last_observation_ns);
                active_samples.push(record.active as u64);
                intent_samples.push(record.intent as u64);
            }
            result.completed_ticks = record.tick;
            result.final_counts = (record.active, record.parked, record.completed);
            result.pending_departures = record.pending_departures;
            result.exhausted_departures = record.exhausted_departures;
            if record.tick == plan.window.warm_up_ticks
                || record.tick == plan.window.end()
                || (record.tick > plan.window.warm_up_ticks
                    && (record.tick - plan.window.warm_up_ticks).is_multiple_of(plan.cycle_ticks))
            {
                result
                    .checkpoints
                    .insert(record.tick, harness.checkpoint()?);
            }
            if record.tick.is_multiple_of(1_024) {
                eprintln!(
                    "tick {}/{}: active={} parked={} completed={} elapsed={:.1}s",
                    record.tick,
                    plan.window.end(),
                    record.active,
                    record.parked,
                    record.completed,
                    started.elapsed().as_secs_f64()
                );
            }
        }
        if plan.window.purpose == "correctness" {
            let case: UrbanCase = plan.case.parse()?;
            for (tile, e) in harness.evidence.iter().enumerate() {
                let missing = match case {
                    UrbanCase::MixedPeak => {
                        e.crossed_tile_completed < plan.required_per_tile["crossed_tile_completed"]
                            || e.red_wait_then_crossed
                                < plan.required_per_tile["red_wait_then_crossed"]
                            || e.leaves + e.explicit_parks + e.virtual_parks
                                < plan.required_per_tile["park_or_leave"]
                    }
                    UrbanCase::GarageEgress => {
                        e.garage_exit_0 < plan.required_per_tile["garage_exit_0"]
                            || e.garage_exit_1 < plan.required_per_tile["garage_exit_1"]
                            || e.safe_leave_rejections
                                < plan.required_per_tile["safe_leave_rejection"]
                            || e.retried_leave_successes
                                < plan.required_per_tile["retried_leave_success"]
                    }
                    UrbanCase::GarageIngress => {
                        e.explicit_parks < plan.required_per_tile["explicit_park"]
                            || e.virtual_parks < plan.required_per_tile["virtual_park"]
                            || e.exclusive_rejections
                                < plan.required_per_tile["exclusive_rejection"]
                            || e.full_rejections < plan.required_per_tile["full_rejection"]
                    }
                    UrbanCase::WaitingRelease => {
                        e.waiting_entries < plan.required_per_tile["waiting_entry"]
                            || e.waiting_capacity_rejections
                                < plan.required_per_tile["waiting_capacity_rejection"]
                            || e.waiting_storage_rejections
                                < plan.required_per_tile["waiting_storage_rejection"]
                            || e.waiting_releases < plan.required_per_tile["waiting_release"]
                            || !e.waiting_entry_order.starts_with(&e.waiting_release_order)
                    }
                    UrbanCase::PermissiveLeft => {
                        e.permissive_no_grants < plan.required_per_tile["permissive_no_grant"]
                            || e.permissive_grants < plan.required_per_tile["permissive_grant"]
                            || e.permissive_passes < plan.required_per_tile["permissive_pass"]
                    }
                    UrbanCase::UncontrolledYield => {
                        e.mainline_passes < plan.required_per_tile["mainline_pass"]
                            || e.yield_waits < plan.required_per_tile["yield_wait"]
                            || e.yield_passes < plan.required_per_tile["yield_pass"]
                    }
                    UrbanCase::BoundaryBurst => {
                        e.phase_changes < plan.required_per_tile["phase_change"]
                            || e.lifecycle_successes < plan.required_per_tile["lifecycle_success"]
                            || e.safe_leave_rejections < plan.required_per_tile["safe_rejection"]
                            || e.retried_leave_successes < plan.required_per_tile["retry_success"]
                            || e.before_boundary_commands
                                < plan.required_per_tile["before_boundary_command"]
                            || e.after_boundary_commands
                                < plan.required_per_tile["after_boundary_command"]
                    }
                };
                if missing {
                    return Err(invalid(format!(
                        "tile {tile}: missing required {} observation",
                        case.as_str()
                    )));
                }
                if case == UrbanCase::MixedPeak && e.planned_east * 3 != e.planned_west * 7 {
                    return Err(invalid(format!(
                        "tile {tile}: departure input is not 70:30"
                    )));
                }
            }
            result.status = "case-pass-replay-required".into();
        } else if plan.window.purpose == "performance" {
            result.status = "performance-round-complete".into();
        } else {
            result.status = "probe-complete".into();
        }
        Ok(())
    })();
    if let Err(error) = run {
        result.error = Some(error.to_string());
    }
    // Keep committed observations from a failed advance visible, without marking that tick passed.
    if result.error.is_some() {
        write_json(
            &output.join("failure.json"),
            &json!({"committed_world_tick":harness.world.tick_index(),
            "commands":harness.commands, "events":harness.events, "error":result.error}),
        )?;
    }
    ticks.flush()?;
    commands.flush()?;
    events.flush()?;
    result.tile_evidence = harness.evidence.clone();
    result.retry_reasons = harness.error_counts.clone();
    result.atomic_rejections = harness.atomic_rejections.clone();
    result.replacements = harness.replacements;
    result.births = harness.births;
    result.removals = harness.removals;
    result.committed_world_tick = harness.world.tick_index();
    for name in [
        "ticks.jsonl",
        "commands.jsonl",
        "events.jsonl",
        "resolved-plan.toml",
    ] {
        result
            .files
            .insert(name.into(), digest_file(&output.join(name))?);
    }
    if let Some((hardware_role, power_role)) = performance_context
        && result.error.is_none()
    {
        let mut measured_steps = times
            .iter()
            .skip(plan.window.warm_up_ticks as usize)
            .copied()
            .collect::<Vec<_>>();
        let memory = peak_resident_bytes();
        let measurements = Measurements {
            version: "urban-performance-measurements-v1",
            execution_id: execution_id.clone(),
            git_commit: command_output("git", &["rev-parse", "HEAD"])
                .unwrap_or_else(|| "unavailable".into()),
            git_status: command_output("git", &["status", "--porcelain"])
                .unwrap_or_else(|| "unavailable".into()),
            rustc: command_output("rustc", &["+1.98.0", "-Vv"])
                .unwrap_or_else(|| "unavailable".into()),
            cargo: command_output("cargo", &["+1.98.0", "-V"])
                .unwrap_or_else(|| "unavailable".into()),
            target: command_output("rustc", &["+1.98.0", "-vV"])
                .and_then(|text| {
                    text.lines()
                        .find_map(|line| line.strip_prefix("host: ").map(str::to_owned))
                })
                .unwrap_or_else(|| "unavailable".into()),
            build_parameters: "cargo +1.98.0 build -p laneflow-urban-harness --release --locked",
            invocation: std::env::args().collect(),
            os: std::env::consts::OS,
            architecture: std::env::consts::ARCH,
            hardware_role,
            power_role,
            workers: 1,
            timing_range: "observation-window-only; command, TrafficWorld::step, and observation measured separately",
            command_ns: sample_summary(&mut command_times)?,
            traffic_world_step_ns: sample_summary(&mut measured_steps)?,
            observation_ns: sample_summary(&mut observation_times)?,
            active: sample_summary(&mut active_samples)?,
            intent: sample_summary(&mut intent_samples)?,
            memory: MemoryMeasurement {
                status: if memory.is_some() {
                    "measured"
                } else {
                    "unmeasured"
                },
                method: peak_resident_method(),
                peak_resident_bytes: memory,
            },
            command_samples_ns: command_times.clone(),
            traffic_world_step_samples_ns: measured_steps.clone(),
            observation_samples_ns: observation_times.clone(),
            active_samples: active_samples.clone(),
            intent_samples: intent_samples.clone(),
        };
        let path = output.join("measurements.toml");
        let mut file = BufWriter::new(File::create(&path)?);
        file.write_all(toml::to_string_pretty(&measurements)?.as_bytes())?;
        file.flush()?;
        result
            .files
            .insert("measurements.toml".into(), digest_file(&path)?);
    }
    write_json(&output.join("result.json"), &result)?;
    times.sort_unstable();
    let percentile = |n: usize| {
        times
            .get((times.len() * n).div_ceil(100).saturating_sub(1))
            .copied()
    };
    write_json(
        &output.join("diagnostics.json"),
        &json!({"purpose":if plan.window.purpose == "performance" {"execution-metadata; formal timings are in measurements.toml"} else {"diagnostic-only-not-performance-certification"}, "execution_id":execution_id, "elapsed_seconds":started.elapsed().as_secs_f64(),
        "verified_steps":times.len(), "step_ns_p50":percentile(50), "step_ns_p95":percentile(95), "step_ns_p99":percentile(99),
        "os":std::env::consts::OS, "architecture":std::env::consts::ARCH, "workers":1,
        "cpu":std::env::var("PROCESSOR_IDENTIFIER").ok(), "logical_cpus":std::thread::available_parallelism().map(|n| n.get()).ok(),
        "binary":std::env::current_exe().ok().and_then(|path| digest_file(&path).ok()),
        "invocation":std::env::args().collect::<Vec<_>>(), "memory_measurement":null,
        "git_commit_at_run":command_output("git", &["rev-parse", "HEAD"]),
        "git_status_at_run":command_output("git", &["status", "--porcelain"])}),
    )?;
    Ok(result)
}

fn sample_summary(values: &mut [u64]) -> Result<SampleSummary> {
    if values.is_empty() {
        return Err(invalid("measurement window produced no samples"));
    }
    values.sort_unstable();
    let pick = |percent: usize| {
        values[(values.len() * percent)
            .div_ceil(100)
            .saturating_sub(1)
            .min(values.len() - 1)]
    };
    Ok(SampleSummary {
        samples: values.len(),
        min: values[0],
        p50: pick(50),
        p95: pick(95),
        p99: pick(99),
        max: values[values.len() - 1],
    })
}

#[cfg(windows)]
fn peak_resident_bytes() -> Option<u64> {
    let process_id = std::process::id().to_string();
    let script = format!("(Get-Process -Id {process_id}).PeakWorkingSet64");
    command_output(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
    )?
    .parse()
    .ok()
}

#[cfg(windows)]
fn peak_resident_method() -> &'static str {
    "PowerShell Get-Process.PeakWorkingSet64"
}

#[cfg(not(windows))]
fn peak_resident_bytes() -> Option<u64> {
    None
}

#[cfg(not(windows))]
fn peak_resident_method() -> &'static str {
    "unavailable-on-this-platform"
}

fn digest_file(path: &Path) -> Result<crate::artifacts::FileDigest> {
    use sha2::{Digest, Sha256};
    let mut reader = BufReader::new(File::open(path)?);
    let mut hash = Sha256::new();
    loop {
        let bytes = reader.fill_buf()?;
        if bytes.is_empty() {
            break;
        }
        hash.update(bytes);
        let len = bytes.len();
        reader.consume(len);
    }
    Ok(crate::artifacts::FileDigest {
        bytes: fs::metadata(path)?.len(),
        sha256: crate::hex(&hash.finalize()),
    })
}

/// Verifies retained files and returns a report binding both executions to the comparison.
pub fn compare_runs(left: &Path, right: &Path) -> Result<ComparisonReport> {
    if fs::canonicalize(left)? == fs::canonicalize(right)? {
        return Err(invalid("replay requires two distinct run directories"));
    }
    let left_execution = read_execution_id(left)?;
    let right_execution = read_execution_id(right)?;
    if left_execution == right_execution {
        return Err(invalid("replay requires distinct execution identities"));
    }
    let read = |dir: &Path| -> Result<(RunResult, crate::artifacts::FileDigest)> {
        let bytes = fs::read(dir.join("result.json"))?;
        let result: RunResult = serde_json::from_slice(&bytes)?;
        if result.version != "urban-result-v3"
            || result.error.is_some()
            || result.completed_ticks != result.expected_ticks
            || !matches!(
                (result.purpose.as_str(), result.status.as_str()),
                ("probe", "probe-complete") | ("correctness", "case-pass-replay-required")
            )
        {
            return Err(invalid("cannot accept an incomplete or failed run"));
        }
        for name in [
            "resolved-plan.toml",
            "ticks.jsonl",
            "commands.jsonl",
            "events.jsonl",
        ] {
            if result.files.get(name) != Some(&digest_file(&dir.join(name))?) {
                return Err(invalid(format!("run file changed: {name}")));
            }
        }
        if sha256(&fs::read(dir.join("resolved-plan.toml"))?) != result.plan_digest {
            return Err(invalid("plan digest differs"));
        }
        let mut count = 0;
        for line in BufReader::new(File::open(dir.join("ticks.jsonl"))?).lines() {
            let row: TickRecord = serde_json::from_str(&line?)?;
            count += 1;
            if row.tick != count || row.active + row.parked + row.completed != row.live {
                return Err(invalid("tick sequence or lifecycle differs"));
            }
        }
        if count != result.completed_ticks {
            return Err(invalid("tick log is incomplete"));
        }
        Ok((
            result,
            crate::artifacts::FileDigest {
                bytes: bytes.len() as u64,
                sha256: sha256(&bytes),
            },
        ))
    };
    let (a, left_digest) = read(left)?;
    let (b, right_digest) = read(right)?;
    if a.plan_digest != b.plan_digest {
        return Err(invalid("different resolved plans"));
    }
    if a != b {
        for (index, (left_row, right_row)) in BufReader::new(File::open(left.join("ticks.jsonl"))?)
            .lines()
            .zip(BufReader::new(File::open(right.join("ticks.jsonl"))?).lines())
            .enumerate()
        {
            let left_row = left_row?;
            let right_row = right_row?;
            if left_row != right_row {
                return Err(invalid(format!(
                    "first differing tick {}: left={left_row}; right={right_row}; inspect commands/events at the preceding boundary",
                    index + 1
                )));
            }
        }
        return Err(invalid(
            "independent runs differ; compare ticks and semantic logs",
        ));
    }
    Ok(ComparisonReport {
        version: "urban-comparison-v1".into(),
        status: if a.purpose == "correctness" {
            "case-pass"
        } else {
            "probe-match"
        }
        .into(),
        purpose: a.purpose,
        case: a.case,
        scale: a.scale,
        plan_digest: a.plan_digest,
        completed_ticks: a.completed_ticks,
        left: ComparedRun {
            execution_id: left_execution,
            result: left_digest,
        },
        right: ComparedRun {
            execution_id: right_execution,
            result: right_digest,
        },
    })
}

/// Verifies and combines exactly three independent formal performance rounds.
pub fn compare_performance_runs(directories: [&Path; 3]) -> Result<PerformanceComparisonReport> {
    let canonical = directories
        .map(fs::canonicalize)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if canonical[0] == canonical[1] || canonical[0] == canonical[2] || canonical[1] == canonical[2]
    {
        return Err(invalid(
            "performance comparison requires three distinct directories",
        ));
    }
    let mut rounds = Vec::new();
    let mut execution_ids = std::collections::BTreeSet::new();
    let mut combined_command = Vec::new();
    let mut combined_step = Vec::new();
    let mut combined_observation = Vec::new();
    let mut combined_active = Vec::new();
    let mut combined_intent = Vec::new();
    let mut identity: Option<(String, String, String)> = None;
    for directory in directories {
        let result_bytes = fs::read(directory.join("result.json"))?;
        let result: RunResult = serde_json::from_slice(&result_bytes)?;
        if result.version != "urban-result-v3"
            || result.purpose != "performance"
            || result.case != "MIXED-PEAK"
            || result.status != "performance-round-complete"
            || result.error.is_some()
            || result.completed_ticks != result.expected_ticks
        {
            return Err(invalid("cannot combine an incomplete performance round"));
        }
        for name in [
            "resolved-plan.toml",
            "ticks.jsonl",
            "commands.jsonl",
            "events.jsonl",
            "measurements.toml",
        ] {
            if result.files.get(name) != Some(&digest_file(&directory.join(name))?) {
                return Err(invalid(format!("performance run file changed: {name}")));
            }
        }
        if sha256(&fs::read(directory.join("resolved-plan.toml"))?) != result.plan_digest {
            return Err(invalid("performance plan digest differs"));
        }
        let mut tick_count = 0;
        for line in BufReader::new(File::open(directory.join("ticks.jsonl"))?).lines() {
            let row: TickRecord = serde_json::from_str(&line?)?;
            tick_count += 1;
            if row.tick != tick_count || row.active + row.parked + row.completed != row.live {
                return Err(invalid("performance tick sequence or lifecycle differs"));
            }
        }
        if tick_count != result.completed_ticks {
            return Err(invalid("performance tick log is incomplete"));
        }
        let measurement_bytes = fs::read(directory.join("measurements.toml"))?;
        let measurement: RetainedMeasurements = toml::from_str(
            std::str::from_utf8(&measurement_bytes).map_err(|e| invalid(e.to_string()))?,
        )?;
        let execution_id = read_execution_id(directory)?;
        if measurement.version != "urban-performance-measurements-v1"
            || measurement.execution_id != execution_id
            || !execution_ids.insert(execution_id.clone())
        {
            return Err(invalid(
                "performance execution identity differs or is duplicated",
            ));
        }
        let current = (
            result.case.clone(),
            result.scale.clone(),
            result.plan_digest.clone(),
        );
        if identity
            .as_ref()
            .is_some_and(|expected| *expected != current)
        {
            return Err(invalid("performance rounds use different plans"));
        }
        identity.get_or_insert(current);
        combined_command.extend(measurement.command_samples_ns);
        combined_step.extend(measurement.traffic_world_step_samples_ns);
        combined_observation.extend(measurement.observation_samples_ns);
        combined_active.extend(measurement.active_samples);
        combined_intent.extend(measurement.intent_samples);
        rounds.push(PerformanceRound {
            execution_id,
            result: crate::artifacts::FileDigest {
                bytes: result_bytes.len() as u64,
                sha256: sha256(&result_bytes),
            },
            measurements: crate::artifacts::FileDigest {
                bytes: measurement_bytes.len() as u64,
                sha256: sha256(&measurement_bytes),
            },
        });
    }
    let (case, scale, plan_digest) = identity.expect("three retained rounds");
    Ok(PerformanceComparisonReport {
        version: "urban-performance-comparison-v1".into(),
        status: "performance-three-rounds-complete".into(),
        case,
        scale,
        plan_digest,
        rounds,
        command_ns: summary_map(sample_summary(&mut combined_command)?),
        traffic_world_step_ns: summary_map(sample_summary(&mut combined_step)?),
        observation_ns: summary_map(sample_summary(&mut combined_observation)?),
        active: summary_map(sample_summary(&mut combined_active)?),
        intent: summary_map(sample_summary(&mut combined_intent)?),
    })
}

fn summary_map(summary: SampleSummary) -> BTreeMap<String, u64> {
    [
        ("samples".into(), summary.samples as u64),
        ("min".into(), summary.min),
        ("p50".into(), summary.p50),
        ("p95".into(), summary.p95),
        ("p99".into(), summary.p99),
        ("max".into(), summary.max),
    ]
    .into()
}

// Local copy/mix-up detection only; this metadata does not attest execution or enter semantic hashes.
fn new_execution_id() -> Result<String> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| invalid(format!("execution start time unavailable: {error}")))?;
    Ok(format!(
        "{}-{}-{}",
        std::process::id(),
        started.as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn read_execution_id(directory: &Path) -> Result<String> {
    let diagnostics: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("diagnostics.json"))?)?;
    diagnostics["execution_id"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| invalid("missing execution identity; rerun to produce current evidence"))
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_summary_uses_nearest_rank_percentiles() {
        let mut values = (1..=100).rev().collect::<Vec<_>>();
        let summary = sample_summary(&mut values).unwrap();
        assert_eq!(summary.samples, 100);
        assert_eq!(summary.min, 1);
        assert_eq!(summary.p50, 50);
        assert_eq!(summary.p95, 95);
        assert_eq!(summary.p99, 99);
        assert_eq!(summary.max, 100);
    }

    #[test]
    fn sample_summary_rejects_an_empty_window() {
        assert!(sample_summary(&mut []).is_err());
    }
}
