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

const MEASUREMENTS_VERSION: &str = "urban-performance-measurements-v2";
const BUILD_PARAMETERS: &str = "cargo +1.98.0 build -p laneflow-urban-harness --release --locked";
const TIMING_RANGE: &str = "observation-window-only; command=sum-of-public-lifecycle-calls; step=public-call-only; observation=pre-and-post-step-inspection; caller-preparation-bookkeeping-snapshots-excluded";

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
    #[serde(flatten)]
    provenance: MeasurementProvenance,
    invocation: Vec<String>,
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
    #[serde(flatten)]
    provenance: MeasurementProvenance,
    command_samples_ns: Vec<u64>,
    traffic_world_step_samples_ns: Vec<u64>,
    observation_samples_ns: Vec<u64>,
    active_samples: Vec<u64>,
    intent_samples: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct MeasurementProvenance {
    git_commit: String,
    git_status: String,
    rustc: String,
    cargo: String,
    target: String,
    build_parameters: String,
    os: String,
    architecture: String,
    hardware_role: String,
    power_role: String,
    workers: u32,
    timing_range: String,
}

impl MeasurementProvenance {
    fn capture(hardware_role: String, power_role: String) -> Result<Self> {
        let read = |program, args: &[&str]| {
            command_output(program, args)
                .ok_or_else(|| invalid(format!("unavailable provenance: {program}")))
        };
        let rustc = read("rustc", &["+1.98.0", "-Vv"])?;
        let target = rustc
            .lines()
            .find_map(|line| line.strip_prefix("host: "))
            .ok_or_else(|| invalid("unavailable target provenance"))?
            .to_owned();
        let provenance = Self {
            git_commit: read("git", &["rev-parse", "HEAD"])?,
            git_status: read("git", &["status", "--porcelain"])?,
            rustc,
            cargo: read("cargo", &["+1.98.0", "-V"])?,
            target,
            build_parameters: BUILD_PARAMETERS.into(),
            os: std::env::consts::OS.into(),
            architecture: std::env::consts::ARCH.into(),
            hardware_role,
            power_role,
            workers: 1,
            timing_range: TIMING_RANGE.into(),
        };
        provenance.validate()?;
        Ok(provenance)
    }

    fn validate(&self) -> Result<()> {
        for value in [
            &self.git_commit,
            &self.rustc,
            &self.cargo,
            &self.target,
            &self.build_parameters,
            &self.os,
            &self.architecture,
            &self.hardware_role,
            &self.power_role,
            &self.timing_range,
        ] {
            if value.trim().is_empty() || value.trim().eq_ignore_ascii_case("unavailable") {
                return Err(invalid("missing or unavailable performance provenance"));
            }
        }
        if !self.git_status.is_empty() {
            return Err(invalid("formal performance requires a clean checkout"));
        }
        if self.git_commit.len() != 40
            || !self.git_commit.bytes().all(|b| b.is_ascii_hexdigit())
            || !self.rustc.starts_with("rustc 1.98.0 ")
            || !self.cargo.starts_with("cargo 1.98.0 ")
            || self
                .rustc
                .lines()
                .find_map(|line| line.strip_prefix("host: "))
                != Some(self.target.as_str())
            || self.build_parameters != BUILD_PARAMETERS
            || self.workers != 1
            || self.timing_range != TIMING_RANGE
        {
            return Err(invalid(
                "performance provenance does not match the fixed protocol",
            ));
        }
        Ok(())
    }
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
    pub aggregation: String,
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
        let checkout = command_output("git", &["rev-parse", "--show-toplevel"])
            .ok_or_else(|| invalid("unavailable checkout for performance output validation"))?;
        validate_performance_output(&std::path::absolute(output)?, Path::new(&checkout))?;
        Some(MeasurementProvenance::capture(
            std::env::var("LANEFLOW_HARDWARE_ROLE")
                .map_err(|_| invalid("performance requires LANEFLOW_HARDWARE_ROLE"))?,
            std::env::var("LANEFLOW_POWER_ROLE")
                .map_err(|_| invalid("performance requires LANEFLOW_POWER_ROLE"))?,
        )?)
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
        version: "urban-result-v4".into(),
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
            validate_case(&harness)?;
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
    if let Some(provenance) = &performance_context
        && result.error.is_none()
        && MeasurementProvenance::capture(
            provenance.hardware_role.clone(),
            provenance.power_role.clone(),
        )
        .as_ref()
        .ok()
            != Some(provenance)
    {
        result.status = "failed".into();
        result.error =
            Some("performance provenance changed or became unavailable during the run".into());
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
    if let Some(provenance) = performance_context
        && result.error.is_none()
    {
        let mut measured_steps = times
            .iter()
            .skip(plan.window.warm_up_ticks as usize)
            .copied()
            .collect::<Vec<_>>();
        let memory = peak_resident_bytes();
        let measurements = Measurements {
            version: MEASUREMENTS_VERSION,
            execution_id: execution_id.clone(),
            provenance,
            invocation: std::env::args().collect(),
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

// Reject a future untracked evidence directory before installing or advancing the world.
// Ignored output (for example target/) is safe without excluding any source from git status.
fn validate_performance_output(output: &Path, checkout: &Path) -> Result<()> {
    let parent = fs::canonicalize(
        output
            .parent()
            .ok_or_else(|| invalid("performance output requires a parent directory"))?,
    )?;
    let checkout = fs::canonicalize(checkout)?;
    if let Ok(relative) = parent.strip_prefix(&checkout) {
        let relative = relative.join(
            output
                .file_name()
                .ok_or_else(|| invalid("performance output requires a new directory name"))?,
        );
        let ignored = std::process::Command::new("git")
            .current_dir(&checkout)
            .args(["check-ignore", "--quiet", "--"])
            .arg(relative)
            .status()?;
        if !ignored.success() {
            return Err(invalid(
                "formal performance output must be outside the checkout or git-ignored (for example target/)",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_case(harness: &Harness<'_>) -> Result<()> {
    let plan = harness.plan;
    harness.validate_required_role_evidence()?;
    let case: UrbanCase = plan.case.parse()?;
    for (tile, e) in harness.evidence.iter().enumerate() {
        let missing = match case {
            UrbanCase::MixedPeak => {
                e.crossed_tile_completed < plan.required_per_tile["crossed_tile_completed"]
                    || e.red_wait_then_crossed < plan.required_per_tile["red_wait_then_crossed"]
                    || e.leaves + e.explicit_parks + e.virtual_parks
                        < plan.required_per_tile["park_or_leave"]
            }
            UrbanCase::GarageEgress => {
                e.garage_exit_0 < plan.required_per_tile["garage_exit_0"]
                    || e.garage_exit_1 < plan.required_per_tile["garage_exit_1"]
                    || e.safe_leave_rejections < plan.required_per_tile["safe_leave_rejection"]
                    || e.retried_leave_successes < plan.required_per_tile["retried_leave_success"]
            }
            UrbanCase::GarageIngress => {
                e.explicit_parks < plan.required_per_tile["explicit_park"]
                    || e.virtual_parks < plan.required_per_tile["virtual_park"]
                    || e.exclusive_rejections < plan.required_per_tile["exclusive_rejection"]
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
                    || e.leaves < plan.required_per_tile["garage_leave"]
            }
            UrbanCase::BoundaryBurst => {
                e.phase_changes < plan.required_per_tile["phase_change"]
                    || e.lifecycle_successes < plan.required_per_tile["lifecycle_success"]
                    || e.safe_leave_rejections < plan.required_per_tile["safe_rejection"]
                    || e.retried_leave_successes < plan.required_per_tile["retry_success"]
                    || e.before_boundary_commands
                        < plan.required_per_tile["before_boundary_command"]
                    || e.after_boundary_commands < plan.required_per_tile["after_boundary_command"]
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
    Ok(())
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
        if result.version != "urban-result-v4"
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
    let mut provenance = None;
    let mut semantic_result = None;
    for directory in directories {
        let result_bytes = fs::read(directory.join("result.json"))?;
        let result: RunResult = serde_json::from_slice(&result_bytes)?;
        if result.version != "urban-result-v4"
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
        let mut measurement: RetainedMeasurements = toml::from_str(
            std::str::from_utf8(&measurement_bytes).map_err(|e| invalid(e.to_string()))?,
        )?;
        let execution_id = read_execution_id(directory)?;
        if measurement.version != MEASUREMENTS_VERSION {
            return Err(invalid(
                "unsupported performance measurement version; rerun with the current timing protocol",
            ));
        }
        if measurement.execution_id != execution_id || !execution_ids.insert(execution_id.clone()) {
            return Err(invalid(
                "performance execution identity differs or is duplicated",
            ));
        }
        measurement.provenance.validate()?;
        if provenance
            .as_ref()
            .is_some_and(|expected| *expected != measurement.provenance)
        {
            return Err(invalid("performance rounds use different provenance"));
        }
        provenance.get_or_insert(measurement.provenance);
        for samples in [
            &measurement.command_samples_ns,
            &measurement.traffic_world_step_samples_ns,
            &measurement.observation_samples_ns,
            &measurement.active_samples,
            &measurement.intent_samples,
        ] {
            if samples.len() as u64 != result.window.observation_ticks {
                return Err(invalid(
                    "performance sample count differs from the observation window",
                ));
            }
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
        // Only the measurement envelope is non-semantic. Retained log digests, complete
        // checkpoints, counts and all other result fields must match across the three worlds.
        let mut semantic = result.clone();
        semantic.files.remove("measurements.toml");
        if semantic_result
            .as_ref()
            .is_some_and(|expected| *expected != semantic)
        {
            return Err(invalid(
                "performance rounds use different semantic traces or results; inspect ticks, commands, events and checkpoints",
            ));
        }
        semantic_result.get_or_insert(semantic);
        combined_command.push(sample_summary(&mut measurement.command_samples_ns)?);
        combined_step.push(sample_summary(
            &mut measurement.traffic_world_step_samples_ns,
        )?);
        combined_observation.push(sample_summary(&mut measurement.observation_samples_ns)?);
        combined_active.push(sample_summary(&mut measurement.active_samples)?);
        combined_intent.push(sample_summary(&mut measurement.intent_samples)?);
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
        version: "urban-performance-comparison-v2".into(),
        aggregation: "median-of-three-round-percentiles; worst-round-max; total-sample-count"
            .into(),
        status: "performance-three-rounds-complete".into(),
        case,
        scale,
        plan_digest,
        rounds,
        command_ns: three_round_summary(&combined_command)?,
        traffic_world_step_ns: three_round_summary(&combined_step)?,
        observation_ns: three_round_summary(&combined_observation)?,
        active: three_round_summary(&combined_active)?,
        intent: three_round_summary(&combined_intent)?,
    })
}

fn three_round_summary(rounds: &[SampleSummary]) -> Result<BTreeMap<String, u64>> {
    if rounds.len() != 3 {
        return Err(invalid("summary requires exactly three rounds"));
    }
    let median = |pick: fn(&SampleSummary) -> u64| {
        let mut values = [pick(&rounds[0]), pick(&rounds[1]), pick(&rounds[2])];
        values.sort_unstable();
        values[1]
    };
    Ok([
        (
            "samples".into(),
            rounds.iter().map(|round| round.samples as u64).sum(),
        ),
        (
            "min".into(),
            rounds
                .iter()
                .map(|round| round.min)
                .min()
                .expect("three rounds"),
        ),
        ("p50".into(), median(|round| round.p50)),
        ("p95".into(), median(|round| round.p95)),
        ("p99".into(), median(|round| round.p99)),
        (
            "max".into(),
            rounds
                .iter()
                .map(|round| round.max)
                .max()
                .expect("three rounds"),
        ),
    ]
    .into())
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
    fn performance_output_rejects_future_untracked_directories_before_creation() {
        let temp = tempfile::tempdir().unwrap();
        let checkout = temp.path().join("checkout");
        fs::create_dir(&checkout).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .arg(&checkout)
                .status()
                .unwrap()
                .success()
        );
        fs::write(checkout.join(".gitignore"), "target/\n").unwrap();
        fs::create_dir(checkout.join("target")).unwrap();
        let untracked = checkout.join("run-output");
        assert!(validate_performance_output(&untracked, &checkout).is_err());
        assert!(!untracked.exists());
        let ignored = checkout.join("target/run-output");
        validate_performance_output(&ignored, &checkout).unwrap();
        assert!(!ignored.exists());
        let outside = temp.path().join("outside-output");
        validate_performance_output(&outside, &checkout).unwrap();
        assert!(!outside.exists());
    }

    fn measurement_fixture() -> serde_json::Value {
        json!({
            "version":MEASUREMENTS_VERSION, "execution_id":"fixture",
            "git_commit":"a".repeat(40), "git_status":"",
            "rustc":"rustc 1.98.0 (fixture)\nhost: x86_64-pc-windows-msvc\nLLVM version: 22.1.8",
            "cargo":"cargo 1.98.0 (fixture)", "target":"x86_64-pc-windows-msvc",
            "build_parameters":BUILD_PARAMETERS, "os":"windows", "architecture":"x86_64",
            "hardware_role":"fixture-machine", "power_role":"balanced", "workers":1,
            "timing_range":TIMING_RANGE,
            "command_samples_ns":[1,2], "traffic_world_step_samples_ns":[3,4],
            "observation_samples_ns":[5,6], "active_samples":[1,1], "intent_samples":[1,1]
        })
    }

    #[test]
    fn measurement_writer_preserves_flat_provenance_for_comparison() {
        let retained: RetainedMeasurements = serde_json::from_value(measurement_fixture()).unwrap();
        let summary = || sample_summary(&mut [1, 2]).unwrap();
        let written = Measurements {
            version: MEASUREMENTS_VERSION,
            execution_id: retained.execution_id.clone(),
            provenance: retained.provenance.clone(),
            invocation: vec!["test-writer".into()],
            command_ns: summary(),
            traffic_world_step_ns: summary(),
            observation_ns: summary(),
            active: summary(),
            intent: summary(),
            memory: MemoryMeasurement {
                status: "unmeasured",
                method: "test",
                peak_resident_bytes: None,
            },
            command_samples_ns: retained.command_samples_ns.clone(),
            traffic_world_step_samples_ns: retained.traffic_world_step_samples_ns.clone(),
            observation_samples_ns: retained.observation_samples_ns.clone(),
            active_samples: retained.active_samples.clone(),
            intent_samples: retained.intent_samples.clone(),
        };
        let bytes = toml::to_string(&written).unwrap();
        let recovered: RetainedMeasurements = toml::from_str(&bytes).unwrap();
        assert_eq!(recovered.provenance, retained.provenance);
        assert_eq!(recovered.version, MEASUREMENTS_VERSION);
        assert_eq!(recovered.command_samples_ns, retained.command_samples_ns);
        assert_eq!(
            recovered.traffic_world_step_samples_ns,
            retained.traffic_world_step_samples_ns
        );
    }

    // Synthetic file-envelope test only: no simulation or formal performance evidence.
    fn write_round(directory: &Path, execution: &str, mut measurement: serde_json::Value) {
        fs::create_dir_all(directory).unwrap();
        measurement["execution_id"] = json!(execution);
        fs::write(
            directory.join("measurements.toml"),
            toml::to_string(&measurement).unwrap(),
        )
        .unwrap();
        fs::write(
            directory.join("resolved-plan.toml"),
            "synthetic-envelope-test",
        )
        .unwrap();
        fs::write(directory.join("commands.jsonl"), "").unwrap();
        fs::write(directory.join("events.jsonl"), "").unwrap();
        write_json(
            &directory.join("diagnostics.json"),
            &json!({"execution_id":execution}),
        )
        .unwrap();
        let mut ticks = File::create(directory.join("ticks.jsonl")).unwrap();
        for tick in 1..=2 {
            line(
                &mut ticks,
                &TickRecord {
                    tick,
                    time_ms: tick * 16,
                    domain: "road_motor_vehicle".into(),
                    live: 1,
                    active: 1,
                    parked: 0,
                    completed: 0,
                    intent: 1,
                    intent_basis: "exact_active_before_step".into(),
                    presented: 0,
                    aggregate_records: 0,
                    aggregate_equivalent: 0,
                    future_departures: 0,
                    pending_departures: 0,
                    exhausted_departures: 0,
                    command_cursor: 0,
                    event_cursor: 0,
                    state_digest: "fixture".into(),
                    event_digest: "fixture".into(),
                    commands_digest: "fixture".into(),
                },
            )
            .unwrap();
        }
        drop(ticks);
        let files = [
            "resolved-plan.toml",
            "ticks.jsonl",
            "commands.jsonl",
            "events.jsonl",
            "measurements.toml",
        ]
        .into_iter()
        .map(|name| (name.into(), digest_file(&directory.join(name)).unwrap()))
        .collect();
        write_json(
            &directory.join("result.json"),
            &RunResult {
                version: "urban-result-v4".into(),
                status: "performance-round-complete".into(),
                purpose: "performance".into(),
                case: "MIXED-PEAK".into(),
                scale: "10k".into(),
                network_revision: "fixture".into(),
                policy_id: "fixture".into(),
                world_id: 1,
                fixed_step_ms: 16,
                window: crate::Window {
                    purpose: "performance".into(),
                    warm_up_ticks: 0,
                    observation_ticks: 2,
                },
                required_per_tile: BTreeMap::new(),
                plan_digest: sha256(b"synthetic-envelope-test"),
                expected_ticks: 2,
                completed_ticks: 2,
                committed_world_tick: 2,
                error: None,
                checkpoints: BTreeMap::new(),
                initial_counts: (1, 0, 0),
                final_counts: (1, 0, 0),
                tile_evidence: vec![],
                retry_reasons: BTreeMap::new(),
                atomic_rejections: BTreeMap::new(),
                replacements: 0,
                births: 0,
                removals: 0,
                pending_departures: 0,
                exhausted_departures: 0,
                files,
            },
        )
        .unwrap();
    }

    #[test]
    fn performance_comparison_rejects_mixed_dirty_missing_and_old_provenance() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("a");
        let b = temp.path().join("b");
        let c = temp.path().join("c");
        for (path, id) in [(&a, "a"), (&b, "b"), (&c, "c")] {
            write_round(path, id, measurement_fixture());
        }
        let result = compare_performance_runs([&a, &b, &c]).unwrap();
        assert_eq!(result.command_ns["samples"], 6);
        assert_eq!(result.rounds.len(), 3);
        for (field, value) in [
            ("git_commit", json!("b".repeat(40))),
            ("git_status", json!(" M source.rs")),
            (
                "rustc",
                json!("rustc 1.98.0 (different)\nhost: x86_64-pc-windows-msvc"),
            ),
            ("cargo", json!("cargo 1.98.0 (different)")),
            ("target", json!("aarch64-unknown-linux-gnu")),
            ("build_parameters", json!("debug build")),
            ("os", json!("linux")),
            ("architecture", json!("aarch64")),
            ("hardware_role", json!("other-machine")),
            ("power_role", json!("power-saver")),
            ("workers", json!(2)),
            ("timing_range", json!("whole-command-phase")),
            ("version", json!("urban-performance-measurements-v1")),
            ("command_samples_ns", json!([1])),
        ] {
            let mut changed = measurement_fixture();
            changed[field] = value;
            // Recompute the enclosing hashes: rejection must be semantic, not a stale digest.
            write_round(&b, "b", changed);
            assert!(
                compare_performance_runs([&a, &b, &c]).is_err(),
                "accepted changed {field}"
            );
        }
        for field in [
            "git_commit",
            "git_status",
            "rustc",
            "cargo",
            "target",
            "build_parameters",
            "os",
            "architecture",
            "hardware_role",
            "power_role",
            "workers",
            "timing_range",
        ] {
            let mut changed = measurement_fixture();
            changed.as_object_mut().unwrap().remove(field);
            write_round(&b, "b", changed);
            assert!(
                compare_performance_runs([&a, &b, &c]).is_err(),
                "accepted missing {field}"
            );
        }
        for field in [
            "git_commit",
            "git_status",
            "rustc",
            "cargo",
            "target",
            "build_parameters",
            "os",
            "architecture",
            "hardware_role",
            "power_role",
            "timing_range",
        ] {
            for unavailable in ["", "unavailable"] {
                if field == "git_status" && unavailable.is_empty() {
                    continue;
                }
                let mut changed = measurement_fixture();
                changed[field] = json!(unavailable);
                // All three rounds share the invalid value; equality alone must not accept it.
                for (path, id) in [(&a, "a"), (&b, "b"), (&c, "c")] {
                    write_round(path, id, changed.clone());
                }
                assert!(
                    compare_performance_runs([&a, &b, &c]).is_err(),
                    "accepted unavailable {field}"
                );
            }
        }
    }

    #[test]
    fn performance_comparison_requires_same_semantics_but_allows_different_timings() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("a");
        let b = temp.path().join("b");
        let c = temp.path().join("c");
        write_round(&a, "a", measurement_fixture());
        write_round(&c, "c", measurement_fixture());
        let mut slower = measurement_fixture();
        slower["traffic_world_step_samples_ns"] = json!([30, 40]);
        write_round(&b, "b", slower);
        assert_eq!(
            compare_performance_runs([&a, &b, &c])
                .unwrap()
                .traffic_world_step_ns["max"],
            40
        );
        for filename in [
            "ticks.jsonl",
            "commands.jsonl",
            "events.jsonl",
            "checkpoint",
        ] {
            write_round(&b, "b", measurement_fixture());
            let result_path = b.join("result.json");
            let mut result: RunResult =
                serde_json::from_slice(&fs::read(&result_path).unwrap()).unwrap();
            if filename == "checkpoint" {
                result.checkpoints.insert(2, "different-world-state".into());
            } else {
                let path = b.join(filename);
                let changed = if filename == "ticks.jsonl" {
                    fs::read_to_string(&path)
                        .unwrap()
                        .replace("fixture", "different-state")
                } else {
                    "{\"different\":true}\n".into()
                };
                fs::write(&path, changed).unwrap();
                // Keep each round's own hashes valid; cross-round semantics must still reject it.
                result
                    .files
                    .insert(filename.into(), digest_file(&path).unwrap());
            }
            write_json(&result_path, &result).unwrap();
            let error = compare_performance_runs([&a, &b, &c])
                .unwrap_err()
                .to_string();
            assert!(error.contains("different semantic"), "{filename}: {error}");
        }
    }

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

    #[test]
    fn three_round_percentiles_use_the_round_median_not_pooled_samples() {
        let rounds = (0..3)
            .map(|round| {
                let mut samples = vec![round; 99];
                samples.push(1_000 + round);
                sample_summary(&mut samples).unwrap()
            })
            .collect::<Vec<_>>();
        let combined = three_round_summary(&rounds).unwrap();
        assert_eq!(combined["samples"], 300);
        assert_eq!(combined["min"], 0);
        assert_eq!(combined["p50"], 1);
        assert_eq!(combined["p95"], 1); // Pooling would report 2.
        assert_eq!(combined["p99"], 1); // Pooling would report 2.
        assert_eq!(combined["max"], 1_002);
        assert!(three_round_summary(&rounds[..2]).is_err());
    }
}
