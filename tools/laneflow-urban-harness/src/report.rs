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

mod log_io;
mod sustained;
mod timing;

const MEASUREMENTS_VERSION: &str = "urban-performance-measurements-v3";
const BUILD_PARAMETERS: &str = "cargo +1.98.0 build -p laneflow-urban-harness --release --locked";
const TIMING_RANGE: &str = "observation-window-only; command=sum-of-public-lifecycle-calls; step=public-call-only; observation=pre-and-post-step-inspection; caller-preparation-bookkeeping-snapshots-excluded";

/// 调用方显式选择详细日志；最小运行回执与错误不受此开关影响。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Diagnostics {
    /// 关闭逐拍、命令和事件文件及其编码。
    #[default]
    Disabled,
    /// 输出完整规范日志，供独立回放和验收比较。
    Enabled,
}

impl Diagnostics {
    fn is_enabled(self) -> bool {
        self == Self::Enabled
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunResult {
    pub version: String,
    pub status: String,
    pub purpose: String,
    pub diagnostics_enabled: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_load: Option<ActiveLoadSummary>,
    pub files: BTreeMap<String, crate::artifacts::FileDigest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveLoadSummary {
    pub target: u64,
    pub before_step: SampleSummary,
    pub after_step: SampleSummary,
    pub before_step_below_target_ticks: usize,
    pub after_step_below_target_ticks: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ComparedRun {
    pub execution_id: String,
    pub workers: u32,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SampleSummary {
    pub samples: usize,
    pub min: u64,
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub max: u64,
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

/// worker 合法域：CLI、measurements 读取/validate、diagnostics 读取三处
/// 统一使用，避免漂移。上限与 Runtime 执行配置一致。
pub const MAX_WORKERS: u32 = 16;

pub(crate) fn valid_workers(workers: u32) -> bool {
    (1..=MAX_WORKERS).contains(&workers)
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
    fn capture(hardware_role: String, power_role: String, workers: u32) -> Result<Self> {
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
            workers,
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
            || !valid_workers(self.workers)
            || self.build_parameters != BUILD_PARAMETERS
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
    /// 三轮已验证的共同 worker 数（provenance 全等保证一致）。
    pub workers: u32,
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

pub(crate) fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
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

/// 运行一个全新世界；受控步进失败保留最后提交的拍和已有观测。
/// correctness/probe 的计时只作诊断，正式 performance 计划另行输出测量封套。
///
/// # Errors
///
/// 计划验证、世界安装、输出创建/读写/编码、检查点或执行来源校验失败时返回错误。
/// correctness/performance 未显式启用诊断时拒绝执行。受控步进失败记入返回值的
/// `error`，初始化失败可能只留下准备文件。
pub fn run_to_directory(
    artifacts: &Artifacts,
    plan: &ResolvedPlan,
    output: &Path,
    execution: laneflow_runtime::ExecutionConfig,
    diagnostics: Diagnostics,
) -> Result<RunResult> {
    plan.validate(artifacts)?;
    if !diagnostics.is_enabled() && plan.window.purpose != "probe" {
        return Err(invalid(
            "correctness/performance 需要显式启用诊断（CLI: --diagnostics）",
        ));
    }
    let _span = tracing::info_span!("urban_run", case = %plan.case, scale = %plan.scale,
        workers = execution.worker_count().get(), diagnostics = diagnostics.is_enabled())
    .entered();
    tracing::info!("运行开始");
    let performance_context = if plan.window.purpose == "performance" {
        let checkout = command_output("git", &["rev-parse", "--show-toplevel"])
            .ok_or_else(|| invalid("unavailable checkout for performance output validation"))?;
        validate_performance_output(&std::path::absolute(output)?, Path::new(&checkout))?;
        Some(MeasurementProvenance::capture(
            std::env::var("LANEFLOW_HARDWARE_ROLE")
                .map_err(|_| invalid("performance requires LANEFLOW_HARDWARE_ROLE"))?,
            std::env::var("LANEFLOW_POWER_ROLE")
                .map_err(|_| invalid("performance requires LANEFLOW_POWER_ROLE"))?,
            execution.worker_count().get(),
        )?)
    } else {
        None
    };
    fs::create_dir(output)?;
    let execution_id = new_execution_id()?;
    let plan_digest = plan.write(&output.join("resolved-plan.toml"))?;
    let started = Instant::now();
    let mut harness = Harness::install(artifacts, plan, execution)?;
    let initial_counts = observe::counts(&harness)?;
    let mut result = RunResult {
        version: "urban-result-v7".into(),
        status: "failed".into(),
        purpose: plan.window.purpose.clone(),
        diagnostics_enabled: diagnostics.is_enabled(),
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
        active_load: None,
        files: BTreeMap::new(),
    };
    result.checkpoints.insert(0, harness.checkpoint()?);
    let mut logs = diagnostics
        .is_enabled()
        .then(|| log_io::Logs::new(output))
        .transpose()?;
    let mut tick_timings = diagnostics
        .is_enabled()
        .then(|| timing::Timings::new(plan.window.end()))
        .transpose()?;
    let mut times = Vec::new();
    let mut window_step_times = Vec::new();
    let mut command_times = Vec::new();
    let mut observation_times = Vec::new();
    let mut active_samples = Vec::new();
    let mut intent_samples = Vec::new();
    let run: Result<()> = (|| {
        for _ in 0..plan.window.end() {
            let tick_started = diagnostics.is_enabled().then(Instant::now);
            let record = harness.advance()?;
            if let Some(logs) = &mut logs {
                logs.write_tick(&record, &harness.commands, &harness.events)?;
            }
            times.push(harness.last_step_ns);
            if record.tick > plan.window.warm_up_ticks {
                window_step_times.push(harness.last_step_ns);
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
                tracing::debug!(
                    tick = record.tick,
                    end = plan.window.end(),
                    active = record.active,
                    parked = record.parked,
                    completed = record.completed,
                    elapsed_seconds = started.elapsed().as_secs_f64(),
                    "运行进度"
                );
            }
            if let (Some(timings), Some(tick_started)) = (&mut tick_timings, tick_started) {
                timings.push(timing::TickTiming {
                    tick: record.tick,
                    step_ns: harness.last_step_ns,
                    command_ns: harness.last_command_ns,
                    observation_ns: harness.last_observation_ns,
                    tick_elapsed_ns: tick_started.elapsed().as_nanos().min(u128::from(u64::MAX))
                        as u64,
                });
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
    if plan.recycling.is_some() && !active_samples.is_empty() {
        let target = u64::from(plan.initial_counts.active);
        result.active_load = Some(ActiveLoadSummary {
            target,
            before_step_below_target_ticks: intent_samples.iter().filter(|&&n| n < target).count(),
            after_step_below_target_ticks: active_samples.iter().filter(|&&n| n < target).count(),
            before_step: sample_summary(&mut intent_samples.clone())?,
            after_step: sample_summary(&mut active_samples.clone())?,
        });
    }
    if let Some(provenance) = &performance_context
        && result.error.is_none()
        && MeasurementProvenance::capture(
            provenance.hardware_role.clone(),
            provenance.power_role.clone(),
            provenance.workers,
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
        let failure = if diagnostics.is_enabled() {
            json!({"committed_world_tick":harness.world.tick_index(),
                "commands":harness.commands,"events":harness.events,"error":result.error})
        } else {
            json!({"committed_world_tick":harness.world.tick_index(),"error":result.error})
        };
        write_json(&output.join("failure.json"), &failure)?;
    }
    if let Some(logs) = &mut logs {
        logs.finish()?;
    }
    if let Some(timings) = &tick_timings {
        timings.write(output)?;
        result.files.insert(
            timing::FILE_NAME.into(),
            digest_file(&output.join(timing::FILE_NAME))?,
        );
    }
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
        if !diagnostics.is_enabled() && name != "resolved-plan.toml" {
            continue;
        }
        result
            .files
            .insert(name.into(), digest_file(&output.join(name))?);
    }
    if let Some(provenance) = performance_context
        && result.error.is_none()
    {
        let measured_steps = times
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
            command_ns: sample_summary(&mut command_times.clone())?,
            traffic_world_step_ns: sample_summary(&mut measured_steps.clone())?,
            observation_ns: sample_summary(&mut observation_times.clone())?,
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
    if result.error.is_none()
        && let Err(error) = sustained::validate_result(&result)
    {
        result.status = "performance-load-failed".into();
        result.error = Some(error.to_string());
        // 负载失败仍保留完整测量；日志已包含全部记录，不再次物化长窗日志副本。
        let failure = json!({"committed_world_tick":harness.world.tick_index(),
            "active_load":result.active_load,"error":result.error});
        let path = output.join("failure.json");
        write_json(&path, &failure)?;
        result
            .files
            .insert("failure.json".into(), digest_file(&path)?);
    }
    // diagnostics.json 的摘要纳入 result.files 完整性封套（worker 计数
    // 的证据封套绑定）：先写 diagnostics、登记摘要，再写 result.json。
    let mut sorted_times = times.clone();
    let mut sorted_window_step_times = window_step_times.clone();
    sorted_times.sort_unstable();
    sorted_window_step_times.sort_unstable();
    let percentile = |n: usize| {
        sorted_times
            .get((sorted_times.len() * n).div_ceil(100).saturating_sub(1))
            .copied()
    };
    // 预热之后的观察窗口单独给出，避免预热段稀释持续负载的耗时。
    let window_percentile = |n: usize| {
        sorted_window_step_times
            .get(
                (window_step_times.len() * n)
                    .div_ceil(100)
                    .saturating_sub(1),
            )
            .copied()
    };
    let diagnostics_path = output.join("diagnostics.json");
    write_json(
        &diagnostics_path,
        &json!({"purpose":if plan.window.purpose == "performance" {"execution-metadata; formal timings are in measurements.toml"} else {"diagnostic-only-not-performance-certification"}, "execution_id":execution_id, "elapsed_seconds":started.elapsed().as_secs_f64(),
        "verified_steps":times.len(), "step_ns_p50":percentile(50), "step_ns_p95":percentile(95), "step_ns_p99":percentile(99),
        "window_steps":window_step_times.len(), "window_step_ns_p50":window_percentile(50), "window_step_ns_p95":window_percentile(95), "window_step_ns_p99":window_percentile(99),
        "window_step_ns_max":window_percentile(100), "window_step_samples_ns":window_step_times,
        "window_command_samples_ns":command_times, "window_observation_samples_ns":observation_times,
        "tick_timings":if diagnostics.is_enabled() { Some(json!({"version":timing::VERSION,"file":timing::FILE_NAME,"order":"completed-tick","scope":"advance; per-tick buffered logs; bookkeeping; checkpoints; excludes initialization and final serialization/flush/hashes"})) } else { None },
        "os":std::env::consts::OS, "architecture":std::env::consts::ARCH, "workers":execution.worker_count().get(),
        "cpu":std::env::var("PROCESSOR_IDENTIFIER").ok(), "logical_cpus":std::thread::available_parallelism().map(|n| n.get()).ok(),
        "binary":std::env::current_exe().ok().and_then(|path| digest_file(&path).ok()),
        "invocation":std::env::args().collect::<Vec<_>>(), "memory_measurement":null,
        "diagnostics_enabled":diagnostics.is_enabled(),
        "git_commit_at_run":command_output("git", &["rev-parse", "HEAD"]),
        "git_status_at_run":command_output("git", &["status", "--porcelain"])}),
    )?;
    result
        .files
        .insert("diagnostics.json".into(), digest_file(&diagnostics_path)?);
    write_json(&output.join("result.json"), &result)?;
    tracing::info!(completed_ticks = result.completed_ticks, status = %result.status, "运行结束");
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
    if case == UrbanCase::SustainedActive {
        return Err(invalid("SUSTAINED-ACTIVE has no correctness witnesses"));
    }
    for (tile, e) in harness.evidence.iter().enumerate() {
        let missing = match case {
            UrbanCase::SustainedActive => unreachable!("rejected above"),
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

pub(crate) fn sample_summary(values: &mut [u64]) -> Result<SampleSummary> {
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
pub(crate) fn peak_resident_bytes() -> Option<u64> {
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
pub(crate) fn peak_resident_method() -> &'static str {
    "PowerShell Get-Process.PeakWorkingSet64"
}

#[cfg(not(windows))]
pub(crate) fn peak_resident_bytes() -> Option<u64> {
    None
}

#[cfg(not(windows))]
pub(crate) fn peak_resident_method() -> &'static str {
    "unavailable-on-this-platform"
}

pub(crate) fn digest_file(path: &Path) -> Result<crate::artifacts::FileDigest> {
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
    // workers 在两臂摘要校验之后读取（证据封套绑定）。
    let read = |dir: &Path| -> Result<(
        RunResult,
        crate::artifacts::FileDigest,
        Option<MeasurementProvenance>,
    )> {
        let bytes = fs::read(dir.join("result.json"))?;
        let result: RunResult = serde_json::from_slice(&bytes)?;
        let performance = matches!(
            (result.purpose.as_str(), result.status.as_str()),
            ("performance", "performance-round-complete")
        );
        if !result.diagnostics_enabled {
            return Err(invalid(
                "完整语义比较需要显式启用详细诊断日志（--diagnostics）",
            ));
        }
        if result.version != "urban-result-v7"
            || result.error.is_some()
            || result.completed_ticks != result.expected_ticks
            || performance && !matches!(result.case.as_str(), "MIXED-PEAK" | "SUSTAINED-ACTIVE")
            || !performance
                && !matches!(
                    (result.purpose.as_str(), result.status.as_str()),
                    ("probe", "probe-complete") | ("correctness", "case-pass-replay-required")
                )
        {
            return Err(invalid("cannot accept an incomplete or failed run"));
        }
        let mut expected_files = vec![
            "resolved-plan.toml",
            "ticks.jsonl",
            "commands.jsonl",
            "events.jsonl",
            // worker 计数的证据封套：摘要失配（复制/篡改）先于 workers 读取拒绝。
            "diagnostics.json",
        ];
        if performance {
            // 正式臂的测量封套同样纳入本臂文件摘要自校验。
            expected_files.push("measurements.toml");
        }
        if result.files.contains_key(timing::FILE_NAME) {
            expected_files.push(timing::FILE_NAME);
        }
        for name in expected_files {
            if result.files.get(name) != Some(&digest_file(&dir.join(name))?) {
                return Err(invalid(format!("run file changed: {name}")));
            }
        }
        require_diagnostics_marker(dir)?;
        if sha256(&fs::read(dir.join("resolved-plan.toml"))?) != result.plan_digest {
            return Err(invalid("plan digest differs"));
        }
        let mut count = 0;
        for line in BufReader::new(File::open(dir.join("ticks.jsonl"))?).lines() {
            let row: TickRecord = serde_json::from_str(&line?)?;
            count += 1;
            if row.tick != count
                || row
                    .active
                    .checked_add(row.parked)
                    .and_then(|n| n.checked_add(row.completed))
                    != Some(row.live)
            {
                return Err(invalid("tick sequence or lifecycle differs"));
            }
        }
        if count != result.completed_ticks {
            return Err(invalid("tick log is incomplete"));
        }
        // 正式臂逐臂独立校验测量封套：版本、执行编号与目录一致、worker
        // 合法、固定协议 provenance 通过、样本数与观测窗一致。封套内容
        // 本身不参与跨臂相等（计时与执行编号按定义不同）。
        let envelope = if performance {
            let measurement: RetainedMeasurements = toml::from_str(
                std::str::from_utf8(&fs::read(dir.join("measurements.toml"))?)
                    .map_err(|e| invalid(e.to_string()))?,
            )?;
            if measurement.version != MEASUREMENTS_VERSION {
                return Err(invalid(
                    "unsupported performance measurement version; rerun with the current timing protocol",
                ));
            }
            if measurement.execution_id != read_execution_id(dir)? {
                return Err(invalid("performance execution identity differs"));
            }
            measurement.provenance.validate()?;
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
            sustained::verify(dir, &result, &measurement)?;
            Some(measurement.provenance)
        } else {
            None
        };
        Ok((
            result,
            crate::artifacts::FileDigest {
                bytes: bytes.len() as u64,
                sha256: sha256(&bytes),
            },
            envelope,
        ))
    };
    let (a, left_digest, left_envelope) = read(left)?;
    let (b, right_digest, right_envelope) = read(right)?;
    let left_workers = read_diagnostics_workers(left)?;
    let right_workers = read_diagnostics_workers(right)?;
    if a.plan_digest != b.plan_digest {
        return Err(invalid("different resolved plans"));
    }
    match (&left_envelope, &right_envelope) {
        (Some(left), Some(right)) => {
            if left.workers != left_workers || right.workers != right_workers {
                return Err(invalid(
                    "performance arm worker records disagree between measurements and diagnostics",
                ));
            }
            // 跨臂 A/B 只用于不同 worker 的执行配置对照；同 worker 的
            // 重复运行不是跨 worker 证据，应走同 worker 三轮聚合
            // （compare <a> <b> <c>）。probe/correctness 路径不受此限制
            // （同 worker replay/确定性对拍是合法用途）。
            if left.workers == right.workers {
                return Err(invalid(
                    "performance arms use the same worker count; use three-round aggregation for same-worker evidence",
                ));
            }
            // 本切片只支持同源码、同工具链、同硬件/电源条件、仅 worker
            // 不同的 A/B：两臂 provenance 除 workers 外必须全等。
            let mut left = left.clone();
            let mut right = right.clone();
            left.workers = 0;
            right.workers = 0;
            if left != right {
                return Err(invalid(
                    "performance arms differ beyond workers; cross-arm compare requires identical source, toolchain and host conditions",
                ));
            }
        }
        (None, None) => {}
        _ => {
            return Err(invalid(
                "cannot compare performance and non-performance runs",
            ));
        }
    }
    // 语义比较排除已逐臂独立验证的执行封套文件：measurements.toml 测量
    // 封套与 diagnostics.json 执行元数据（计时、执行编号、workers 的载体；
    // 其摘要已在逐臂 files 校验中与各臂自身内容绑定）。probe/correctness
    // 臂无 measurements.toml，移除为空操作；其余字段（含检查点、角色见证、
    // 计数、plan 摘要、逐拍日志摘要）全部保留比较。
    // timings.jsonl 只保存非确定性的逐拍耗时，其摘要也已逐臂核对。
    let mut a_semantic = a.clone();
    a_semantic.files.remove("measurements.toml");
    a_semantic.files.remove("diagnostics.json");
    a_semantic.files.remove(timing::FILE_NAME);
    let mut b_semantic = b.clone();
    b_semantic.files.remove("measurements.toml");
    b_semantic.files.remove("diagnostics.json");
    b_semantic.files.remove(timing::FILE_NAME);
    if a_semantic != b_semantic {
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
        version: "urban-comparison-v2".into(),
        status: match a.purpose.as_str() {
            "correctness" => "case-pass",
            "performance" => "performance-match",
            _ => "probe-match",
        }
        .into(),
        purpose: a.purpose,
        case: a.case,
        scale: a.scale,
        plan_digest: a.plan_digest,
        completed_ticks: a.completed_ticks,
        left: ComparedRun {
            execution_id: left_execution,
            workers: left_workers,
            result: left_digest,
        },
        right: ComparedRun {
            execution_id: right_execution,
            workers: right_workers,
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
        if !result.diagnostics_enabled {
            return Err(invalid(
                "正式性能聚合需要显式启用详细诊断日志（--diagnostics）",
            ));
        }
        if result.version != "urban-result-v7"
            || result.purpose != "performance"
            || !matches!(result.case.as_str(), "MIXED-PEAK" | "SUSTAINED-ACTIVE")
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
            "diagnostics.json",
        ] {
            if result.files.get(name) != Some(&digest_file(&directory.join(name))?) {
                return Err(invalid(format!("performance run file changed: {name}")));
            }
        }
        if let Some(expected) = result.files.get(timing::FILE_NAME)
            && expected != &digest_file(&directory.join(timing::FILE_NAME))?
        {
            return Err(invalid("逐拍计时文件摘要不符"));
        }
        require_diagnostics_marker(directory)?;
        if sha256(&fs::read(directory.join("resolved-plan.toml"))?) != result.plan_digest {
            return Err(invalid("performance plan digest differs"));
        }
        let mut tick_count = 0;
        for line in BufReader::new(File::open(directory.join("ticks.jsonl"))?).lines() {
            let row: TickRecord = serde_json::from_str(&line?)?;
            tick_count += 1;
            if row.tick != tick_count
                || row
                    .active
                    .checked_add(row.parked)
                    .and_then(|n| n.checked_add(row.completed))
                    != Some(row.live)
            {
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
        // 与 compare_runs 同款交叉核对：diagnostics 与 measurements 的
        // worker 记录矛盾即拒绝（即使各摘要已一致重算）。
        if read_diagnostics_workers(directory)? != measurement.provenance.workers {
            return Err(invalid(
                "performance round worker records disagree between measurements and diagnostics",
            ));
        }
        if provenance
            .as_ref()
            .is_some_and(|expected| *expected != measurement.provenance)
        {
            return Err(invalid("performance rounds use different provenance"));
        }
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
        sustained::verify(directory, &result, &measurement)?;
        provenance.get_or_insert(measurement.provenance);
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
        // 三类计时/执行封套均已核对各轮自身摘要，其余语义轨迹完整比较。
        let mut semantic = result.clone();
        semantic.files.remove("measurements.toml");
        semantic.files.remove("diagnostics.json");
        semantic.files.remove(timing::FILE_NAME);
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
    let workers = provenance.expect("three validated rounds").workers;
    Ok(PerformanceComparisonReport {
        version: "urban-performance-comparison-v3".into(),
        workers,
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
pub(crate) fn new_execution_id() -> Result<String> {
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

fn require_diagnostics_marker(directory: &Path) -> Result<()> {
    let diagnostics: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("diagnostics.json"))?)?;
    if diagnostics["diagnostics_enabled"].as_bool() != Some(true) {
        return Err(invalid("执行封套的诊断标记缺失、关闭或与结果回执不符"));
    }
    Ok(())
}

/// 读取运行目录 diagnostics.json 的实际 worker 数；合法域 1..=16，
/// 缺失或越界拒绝（与 CLI、provenance validate 同一规则）。
pub(crate) fn read_diagnostics_workers(directory: &Path) -> Result<u32> {
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("diagnostics.json"))?)?;
    value["workers"]
        .as_u64()
        .and_then(|workers| u32::try_from(workers).ok())
        .filter(|workers| valid_workers(*workers))
        .ok_or_else(|| invalid("missing or invalid diagnostics workers"))
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

pub(crate) fn command_output(program: &str, args: &[&str]) -> Option<String> {
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

    pub(super) fn measurement_fixture() -> serde_json::Value {
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

    /// worker=4 的 Measurements 序列化/读取/validate 正路径；0/17 越界拒绝。
    #[test]
    fn measurement_provenance_accepts_workers_in_legal_range_only() {
        let mut fixture = measurement_fixture();
        fixture["workers"] = json!(4);
        let retained: RetainedMeasurements = serde_json::from_value(fixture.clone()).unwrap();
        retained.provenance.validate().unwrap();
        for illegal in [0, 17] {
            let mut bad = fixture.clone();
            bad["workers"] = json!(illegal);
            let retained: RetainedMeasurements = serde_json::from_value(bad).unwrap();
            assert!(
                retained.provenance.validate().is_err(),
                "workers={illegal} must be rejected by validate"
            );
        }
    }

    /// 三轮聚合：全 4 接受（新合法域真正被接受）、1/4/1 混臂拒绝、
    /// 全 0 / 全 17 因 worker 合法性拒绝（封套摘要已重算，拒绝来自
    /// validate 而非旧摘要不匹配）。
    #[test]
    fn performance_aggregation_validates_workers_range_and_uniformity() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("a");
        let b = temp.path().join("b");
        let c = temp.path().join("c");
        for (path, id) in [(&a, "a"), (&b, "b"), (&c, "c")] {
            let mut fixture = measurement_fixture();
            fixture["workers"] = json!(4);
            write_round(path, id, fixture);
        }
        let result = compare_performance_runs([&a, &b, &c]).unwrap();
        assert_eq!(result.rounds.len(), 3);
        assert_eq!(result.workers, 4, "聚合报告记录三轮共同 worker 数");
        // 混臂拒绝（a/c=4，b=1）。
        let mut mixed = measurement_fixture();
        mixed["workers"] = json!(1);
        write_round(&b, "b", mixed);
        assert!(compare_performance_runs([&a, &b, &c]).is_err());
        // 全 0 / 全 17：各自相等但非法，必须由 validate 拒绝。
        for illegal in [0, 17] {
            for (path, id) in [(&a, "a"), (&b, "b"), (&c, "c")] {
                let mut fixture = measurement_fixture();
                fixture["workers"] = json!(illegal);
                write_round(path, id, fixture);
            }
            assert!(
                compare_performance_runs([&a, &b, &c]).is_err(),
                "uniform workers={illegal} must be rejected by legality"
            );
        }
    }

    /// 线程 A：跨臂 A/B 只接受不同 worker 的 performance 臂——4v4 语义
    /// 相同也拒绝（同 worker 重复运行不是跨 worker 证据）；1v4 接受
    /// （既有用例）；probe 1v1 接受由 tests/harness.rs 的 replay 对拍
    /// 覆盖（同 worker 确定性对拍是合法用途）。
    #[test]
    fn compare_runs_rejects_same_worker_performance_arms() {
        let temp = tempfile::tempdir().unwrap();
        let four_a = temp.path().join("four-a");
        let four_b = temp.path().join("four-b");
        for (dir, execution) in [(&four_a, "exec-four-a"), (&four_b, "exec-four-b")] {
            let mut fixture = measurement_fixture();
            fixture["workers"] = json!(4);
            write_round(dir, execution, fixture);
        }
        assert!(
            compare_runs(&four_a, &four_b).is_err(),
            "同 worker 的 performance 臂必须拒绝并指向三轮聚合"
        );
    }

    /// 线程 C：三轮聚合逐臂交叉核对 diagnostics↔measurements worker——
    /// 矛盾记录即使各摘要已一致重算也拒绝。
    #[test]
    fn performance_aggregation_rejects_contradictory_worker_records() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("a");
        let b = temp.path().join("b");
        let c = temp.path().join("c");
        for (path, id) in [(&a, "a"), (&b, "b"), (&c, "c")] {
            let mut fixture = measurement_fixture();
            fixture["workers"] = json!(4);
            write_round(path, id, fixture);
        }
        // b 臂：diagnostics 改写为 workers=1 并一致重算 result.json 摘要，
        // 使拒绝来自交叉核对而非摘要失配。
        let diagnostics = b.join("diagnostics.json");
        write_json(
            &diagnostics,
            &json!({"execution_id":"b", "workers":1,"diagnostics_enabled":true}),
        )
        .unwrap();
        let result_path = b.join("result.json");
        let mut result: RunResult =
            serde_json::from_slice(&fs::read(&result_path).unwrap()).unwrap();
        result.files.insert(
            "diagnostics.json".into(),
            digest_file(&diagnostics).unwrap(),
        );
        fs::write(&result_path, serde_json::to_vec(&result).unwrap()).unwrap();
        assert!(
            compare_performance_runs([&a, &b, &c]).is_err(),
            "diagnostics 与 measurements 的 worker 矛盾必须被拒绝"
        );
    }

    /// diagnostics.json 的 worker 读取：合法域校验与缺失拒绝。
    #[test]
    fn read_diagnostics_workers_enforces_range() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("run");
        fs::create_dir(&dir).unwrap();
        for (workers, ok) in [(json!(4), true), (json!(0), false), (json!(17), false)] {
            write_json(
                &dir.join("diagnostics.json"),
                &json!({"execution_id":"e", "workers":workers}),
            )
            .unwrap();
            assert_eq!(
                super::read_diagnostics_workers(&dir).is_ok(),
                ok,
                "workers={workers}"
            );
        }
        write_json(&dir.join("diagnostics.json"), &json!({"execution_id":"e"})).unwrap();
        assert!(super::read_diagnostics_workers(&dir).is_err());
    }

    /// 正式 performance 跨臂 A/B：同源码/工具链/硬件电源、仅 worker 不同、
    /// 语义逐拍一致、计时样本不同 → 接受；日志/检查点不同（摘要已重算）
    /// → 拒绝；diagnostics 与 measurements 的 worker 记录矛盾 → 拒绝。
    #[test]
    fn compare_runs_accepts_and_rejects_performance_arms() {
        let temp = tempfile::tempdir().unwrap();
        let one = temp.path().join("one");
        let four = temp.path().join("four");
        for (dir, execution, workers, offset) in
            [(&one, "exec-one", 1, 0_u64), (&four, "exec-four", 4, 100)]
        {
            let mut fixture = measurement_fixture();
            fixture["workers"] = json!(workers);
            fixture["command_samples_ns"] = json!([1 + offset, 2 + offset]);
            fixture["traffic_world_step_samples_ns"] = json!([3 + offset, 4 + offset]);
            fixture["observation_samples_ns"] = json!([5 + offset, 6 + offset]);
            fixture["active_samples"] = json!([1, 1]);
            fixture["intent_samples"] = json!([1, 1]);
            write_round(dir, execution, fixture);
        }
        let report = compare_runs(&one, &four).unwrap();
        assert_eq!(report.status, "performance-match");
        assert_eq!(report.left.workers, 1);
        assert_eq!(report.right.workers, 4);

        // 负路径：检查点不同（重算自身 result.json 摘要后仍必须拒绝——
        // 语义差异不能被测量封套排除规则掩盖）。
        let divergent = temp.path().join("divergent");
        let mut fixture = measurement_fixture();
        fixture["workers"] = json!(4);
        write_round(&divergent, "exec-divergent", fixture);
        {
            let path = divergent.join("result.json");
            let mut result: RunResult = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            result.checkpoints.insert(1, "different-checkpoint".into());
            let bytes = serde_json::to_vec(&result).unwrap();
            fs::write(&path, &bytes).unwrap();
        }
        assert!(
            compare_runs(&four, &divergent).is_err(),
            "semantic divergence must not be masked by the measurement envelope exclusion"
        );

        // 负路径：diagnostics 与 measurements 的 worker 记录矛盾。
        let contradiction = temp.path().join("contradiction");
        let mut fixture = measurement_fixture();
        fixture["workers"] = json!(4);
        write_round(&contradiction, "exec-contradiction", fixture);
        write_json(
            &contradiction.join("diagnostics.json"),
            &json!({"execution_id":"exec-contradiction", "workers":1,"diagnostics_enabled":true}),
        )
        .unwrap();
        assert!(
            compare_runs(&four, &contradiction).is_err(),
            "contradictory worker records must be rejected"
        );
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
    pub(super) fn write_round(
        directory: &Path,
        execution: &str,
        mut measurement: serde_json::Value,
    ) {
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
            &json!({"execution_id":execution, "workers":measurement["workers"],"diagnostics_enabled":true}),
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
            "diagnostics.json",
        ]
        .into_iter()
        .map(|name| (name.into(), digest_file(&directory.join(name)).unwrap()))
        .collect();
        write_json(
            &directory.join("result.json"),
            &RunResult {
                version: "urban-result-v7".into(),
                diagnostics_enabled: true,
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
                active_load: None,
                files,
            },
        )
        .unwrap();
    }

    #[test]
    fn comparisons_reject_quiet_and_old_result_receipts() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("a");
        let b = temp.path().join("b");
        let c = temp.path().join("c");
        let four = temp.path().join("four");
        for (path, id) in [(&a, "a"), (&b, "b"), (&c, "c")] {
            write_round(path, id, measurement_fixture());
        }
        let mut fixture = measurement_fixture();
        fixture["workers"] = json!(4);
        write_round(&four, "four", fixture);
        assert!(compare_runs(&a, &four).is_ok());
        assert!(compare_performance_runs([&a, &b, &c]).is_ok());
        let path = a.join("result.json");
        let original: RunResult = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let mut quiet = original.clone();
        quiet.diagnostics_enabled = false;
        write_json(&path, &quiet).unwrap();
        assert!(
            compare_runs(&a, &four)
                .unwrap_err()
                .to_string()
                .contains("--diagnostics")
        );
        assert!(
            compare_performance_runs([&a, &b, &c])
                .unwrap_err()
                .to_string()
                .contains("--diagnostics")
        );
        let mut old = original.clone();
        old.version = "urban-result-v6".into();
        write_json(&path, &old).unwrap();
        assert!(compare_runs(&a, &four).is_err());
        assert!(compare_performance_runs([&a, &b, &c]).is_err());
        let mut missing = serde_json::to_value(&original).unwrap();
        missing
            .as_object_mut()
            .unwrap()
            .remove("diagnostics_enabled");
        write_json(&path, &missing).unwrap();
        assert!(compare_runs(&a, &four).is_err());
        assert!(compare_performance_runs([&a, &b, &c]).is_err());
        let diagnostics_path = a.join("diagnostics.json");
        let mut declared: serde_json::Value =
            serde_json::from_slice(&fs::read(&diagnostics_path).unwrap()).unwrap();
        declared["diagnostics_enabled"] = json!(false);
        write_json(&diagnostics_path, &declared).unwrap();
        let mut contradictory = original;
        contradictory.files.insert(
            "diagnostics.json".into(),
            digest_file(&diagnostics_path).unwrap(),
        );
        write_json(&path, &contradictory).unwrap();
        assert!(
            compare_runs(&a, &four)
                .unwrap_err()
                .to_string()
                .contains("诊断标记")
        );
        assert!(
            compare_performance_runs([&a, &b, &c])
                .unwrap_err()
                .to_string()
                .contains("诊断标记")
        );
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
