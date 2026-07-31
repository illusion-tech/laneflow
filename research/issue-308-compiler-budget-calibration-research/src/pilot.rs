//! 编译器测量基准规模发现所需的新进程冷实例试运行基础。
//!
//! 本模块执行启动前停止护栏、七个受监控的独立 timing 子进程、同级独立 oracle
//! 子进程、冷实例结果核对、墙钟中位数/中位绝对偏差计算和基础规模严格二倍发现。
//! 只有 timing 摘要与预言机完整计数及完整有类型输出全部一致，级别才可能形成 `B`；
//! Evidence v1 写出与正式轮次仍由后续切片负责。

use crate::process_containment::ContainedChild;
use crate::{
    BASE_SCALE_STRING_PROFILE, BASELINE_CANDIDATE_ID, ChildProcessMemoryMonitor,
    ChildProcessMemoryObservation, GENERATOR_VERSION_V1, GraphProfileId,
    GuardCompletedLevelObservation, GuardError, GuardPreflightReport, GuardThresholds,
    IDENTITY_TIMING_CHILD_SCHEMA, IDENTITY_TIMING_CHILD_SCHEMA_VERSION, IdentityTimingChildReport,
    InvalidationReason, NullableObservation, ORACLE_BINARY_ID, ProcessObservation,
    ProcessProtocolError, RunStatus, SCALABLE_ORACLE_CHILD_SCHEMA,
    SCALABLE_ORACLE_CHILD_SCHEMA_VERSION, SCALABLE_TIMING_CHILD_SCHEMA,
    SCALABLE_TIMING_CHILD_SCHEMA_VERSION, ScalableGuardPlanner, ScalableOracleChildReport,
    ScalableOracleOutcome, ScalableTimingChildReport, ScalableTimingOutcome, ScalableWorkloadId,
    SystemMemoryMonitor, SystemMemoryObservation, TIMING_BINARY_ID, TimingError, TrustedContract,
    WORKLOAD_REVISION_V1, observe_clock_quantum_ns, validate_base_scale_contract,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const FRESH_PROCESS_PILOT_SAMPLE_COUNT: usize = 7;
pub const CLOCK_QUANTUM_MULTIPLIER: u64 = 10_000;
pub const MAXIMUM_RELATIVE_MAD_PERCENT: u64 = 2;
const CHILD_MONITOR_POLL_INTERVAL: Duration = Duration::from_millis(1);
const CHILD_MONITOR_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const CHILD_TERMINATION_TIMEOUT_MS: u64 = 5_000;
const CHILD_TERMINATION_TOTAL_TIMEOUT_MS: u64 = CHILD_TERMINATION_TIMEOUT_MS * 2;
const CHILD_TERMINATION_TIMEOUT: Duration = Duration::from_millis(CHILD_TERMINATION_TIMEOUT_MS);
const CHILD_TERMINATION_RETRY_INTERVAL: Duration = Duration::from_millis(10);
const CHILD_START_SIGNAL: u8 = b'G';

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChildMonitorTrigger {
    PrivateBytes,
    WallTime,
    AvailablePhysicalMemory,
    MonitoringGap,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildProcessMonitorReport {
    pub observation_count: u64,
    pub last_private_bytes: NullableObservation<u64>,
    pub peak_private_bytes: NullableObservation<u64>,
    pub elapsed_wall_time_ns: u64,
    pub trigger: Option<ChildMonitorTrigger>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityMonitoredChildSample {
    pub status: RunStatus,
    pub invalidation_reasons: Vec<InvalidationReason>,
    pub process: ProcessObservation,
    pub child: IdentityTimingChildReport,
    pub monitor: ChildProcessMonitorReport,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityFreshProcessPilot {
    pub pilot_id: String,
    pub graph_profile: String,
    pub n: u32,
    pub clock_quantum_ns: u64,
    pub required_median_wall_time_ns: u64,
    pub median_wall_time_ns: u64,
    pub median_absolute_deviation_ns: u64,
    pub clock_quantum_requirement_met: bool,
    pub relative_mad_requirement_met: bool,
    pub semantic_digest_consistent: bool,
    pub measurement_quality_met: bool,
    pub guard_preflight_evaluated: bool,
    pub guard_preflights: Vec<GuardPreflightReport>,
    pub samples: Vec<IdentityMonitoredChildSample>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityFreshProcessPilotStop {
    pub pilot_id: String,
    pub graph_profile: String,
    pub n: u32,
    pub sample_ordinal: usize,
    pub status: RunStatus,
    pub invalidation_reasons: Vec<InvalidationReason>,
    pub process: ProcessObservation,
    pub guard_preflight: GuardPreflightReport,
    pub child: Option<IdentityTimingChildReport>,
    pub monitor: Option<ChildProcessMonitorReport>,
    pub kill_error: Option<String>,
    pub monitor_error: Option<String>,
    pub stderr: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "disposition", rename_all = "kebab-case")]
pub enum IdentityFreshProcessPilotOutcome {
    Completed {
        pilot: IdentityFreshProcessPilot,
    },
    Stopped {
        stop: Box<IdentityFreshProcessPilotStop>,
    },
}

pub const FORMAL_PROTOCOL_ID: &str = "compiler-calibration-v1";
pub const BASE_SCALE_PILOT_CHECKPOINT_SCHEMA: &str =
    "laneflow.compiler-calibration-base-scale-pilot-checkpoint";
pub const BASE_SCALE_PILOT_CHECKPOINT_SCHEMA_VERSION: u32 = 2;
pub const BASE_SCALE_SELECTION_RULE: &str = "first-power-of-two-qualifying-seven-pilot-runs-v1";
pub const BASE_SCALE_AGGREGATION_METHOD: &str = "median-and-mad-of-seven-exact-integers-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BaseScalePilotRunKind {
    ColdInstance,
    GuardPreflight,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseScalePilotRun {
    pub run_id: String,
    pub attempt_id: String,
    pub retry_ordinal: u32,
    pub pilot_sample_position: u32,
    pub run_kind: BaseScalePilotRunKind,
    pub workload_id: ScalableWorkloadId,
    pub workload_revision: u32,
    pub graph_profile: String,
    pub string_profile: String,
    pub generator_version: u32,
    pub n: u32,
    pub status: RunStatus,
    pub invalidation_reasons: Vec<InvalidationReason>,
    pub process: ProcessObservation,
    pub guard_preflight: GuardPreflightReport,
    pub child: Option<ScalableTimingChildReport>,
    pub monitor: Option<ChildProcessMonitorReport>,
    pub kill_error: Option<String>,
    pub monitor_error: Option<String>,
    pub stderr: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseScaleOracleRun {
    pub run_id: String,
    pub workload_id: ScalableWorkloadId,
    pub workload_revision: u32,
    pub graph_profile: String,
    pub string_profile: String,
    pub generator_version: u32,
    pub n: u32,
    pub status: RunStatus,
    pub invalidation_reasons: Vec<BaseScaleOracleInvalidationReason>,
    pub process: ProcessObservation,
    pub guard_preflight: GuardPreflightReport,
    pub child: Option<ScalableOracleChildReport>,
    pub monitor: Option<ChildProcessMonitorReport>,
    pub kill_error: Option<String>,
    pub monitor_error: Option<String>,
    pub stderr: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BaseScaleOracleInvalidationReason {
    ResearchStopGuardrailTriggered,
    MonitoringGap,
    ChildAbnormalExit,
    IndependentOracleMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseScalePilotLevel {
    pub n: u32,
    pub contributing_run_ids: Vec<String>,
    pub aggregation_method: String,
    pub wall_time_median_ns: u64,
    pub wall_time_median_absolute_deviation_ns: u64,
    pub minimum_reliable_wall_time_ns: u64,
    pub semantic_digest: String,
    pub all_semantic_digests_equal: bool,
    pub oracle_run_id: String,
    pub oracle_semantic_digest: String,
    pub complete_counts_equal: bool,
    pub complete_typed_output_equal: bool,
    pub all_guards_clear: bool,
    pub completed_level_guard_observation: GuardCompletedLevelObservation,
    pub qualifies: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseScaleSelection {
    pub candidate_id: String,
    pub workload_id: ScalableWorkloadId,
    pub workload_revision: u32,
    pub graph_profile: String,
    pub string_profile: String,
    pub generator_version: u32,
    pub selection_rule: String,
    pub pilot_levels: Vec<BaseScalePilotLevel>,
    pub b: NullableObservation<u32>,
    pub terminal_guard_run_id: NullableObservation<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseScalePilotCheckpoint {
    pub schema: String,
    pub schema_version: u32,
    pub protocol_id: String,
    pub clock_quantum_ns: u64,
    pub required_median_wall_time_ns: u64,
    pub selections: Vec<BaseScaleSelection>,
    pub active_selection: Option<BaseScaleSelection>,
    pub runs: Vec<BaseScalePilotRun>,
    pub oracle_runs: Vec<BaseScaleOracleRun>,
}

struct BaseScaleDiscoveryRequest<'a> {
    timing_executable: &'a Path,
    oracle_executable: &'a Path,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    required_median_wall_time_ns: u64,
}

struct BaseScaleRunDescriptor {
    run_id: String,
    attempt_id: String,
    retry_ordinal: u32,
    pilot_sample_position: usize,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
}

struct ScalableChildExpectation<'a> {
    compiler_instance_id: &'a str,
    child_pid: u32,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    controlled_allocation_hard_ceiling_bytes: u64,
}

struct ScalableOracleExpectation<'a> {
    run_id: &'a str,
    child_pid: u32,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    primary_record_count: u64,
    controlled_allocation_hard_ceiling_bytes: u64,
}

struct IdentityFreshProcessPilotRequest<'a> {
    timing_executable: &'a Path,
    pilot_id: &'a str,
    graph_profile: GraphProfileId,
    n: u32,
    previous: Option<GuardCompletedLevelObservation>,
}

pub fn wait_for_parent_start_signal() -> Result<(), PilotError> {
    let mut signal = [0_u8; 1];
    std::io::stdin()
        .read_exact(&mut signal)
        .map_err(PilotError::ChildStartSignalRead)?;
    if signal[0] != CHILD_START_SIGNAL {
        return Err(PilotError::InvalidChildStartSignal { actual: signal[0] });
    }
    Ok(())
}

pub fn run_identity_fresh_process_pilot(
    trusted: &TrustedContract,
    timing_executable: &Path,
    pilot_id: &str,
    graph_profile: GraphProfileId,
    n: u32,
    previous: Option<GuardCompletedLevelObservation>,
) -> Result<IdentityFreshProcessPilotOutcome, PilotError> {
    let mut memory_monitor = SystemMemoryMonitor::new()?;
    run_identity_fresh_process_pilot_with_memory_observer(
        trusted,
        IdentityFreshProcessPilotRequest {
            timing_executable,
            pilot_id,
            graph_profile,
            n,
            previous,
        },
        || memory_monitor.observe(),
    )
}

fn run_identity_fresh_process_pilot_with_memory_observer(
    trusted: &TrustedContract,
    request: IdentityFreshProcessPilotRequest<'_>,
    mut observe_memory: impl FnMut() -> Result<SystemMemoryObservation, GuardError>,
) -> Result<IdentityFreshProcessPilotOutcome, PilotError> {
    let IdentityFreshProcessPilotRequest {
        timing_executable,
        pilot_id,
        graph_profile,
        n,
        previous,
    } = request;
    if pilot_id.is_empty() {
        return Err(PilotError::EmptyPilotId);
    }
    let clock_quantum_ns = observe_clock_quantum_ns()?;
    let required_median_wall_time_ns = clock_quantum_ns
        .checked_mul(CLOCK_QUANTUM_MULTIPLIER)
        .ok_or(PilotError::ArithmeticOverflow("required median wall time"))?;
    let guard_planner = ScalableGuardPlanner::from_trusted_contract(trusted)?;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(FRESH_PROCESS_PILOT_SAMPLE_COUNT)
        .map_err(|source| PilotError::AllocationFailed {
            field: "fresh-process pilot samples",
            source,
        })?;
    let mut guard_preflights = Vec::new();
    guard_preflights
        .try_reserve_exact(FRESH_PROCESS_PILOT_SAMPLE_COUNT)
        .map_err(|source| PilotError::AllocationFailed {
            field: "fresh-process pilot guard preflights",
            source,
        })?;

    for ordinal in 0..FRESH_PROCESS_PILOT_SAMPLE_COUNT {
        let guard = guard_planner.evaluate(
            ScalableWorkloadId::Identity,
            graph_profile,
            n,
            observe_memory()?,
            previous,
        )?;
        if !guard.allows_child_start {
            return Ok(IdentityFreshProcessPilotOutcome::Stopped {
                stop: Box::new(IdentityFreshProcessPilotStop {
                    pilot_id: pilot_id.to_owned(),
                    graph_profile: graph_profile.as_str().to_owned(),
                    n,
                    sample_ordinal: ordinal,
                    status: RunStatus::Guarded,
                    invalidation_reasons: Vec::new(),
                    process: ProcessObservation::guarded_before_start(
                        std::process::id(),
                        TIMING_BINARY_ID,
                    ),
                    guard_preflight: guard,
                    child: None,
                    monitor: None,
                    kill_error: None,
                    monitor_error: None,
                    stderr: String::new(),
                }),
            });
        }
        let thresholds = guard.thresholds;

        let compiler_instance_id = format!("{pilot_id}/compiler-instance-{ordinal}");
        let execution = run_monitored_identity_child(
            timing_executable,
            ordinal,
            &compiler_instance_id,
            graph_profile,
            n,
            thresholds,
        )?;
        let (child_pid, output, monitor, monitor_invalidation, kill_error, monitor_error) =
            match execution {
                MonitoredChildExecution::Exited {
                    child_pid,
                    output,
                    monitor,
                } => (child_pid, output, monitor, false, None, None),
                MonitoredChildExecution::InvalidatedByMonitor {
                    child_pid,
                    output,
                    monitor,
                    kill_error,
                    monitor_error,
                } => (child_pid, output, monitor, true, kill_error, monitor_error),
            };

        if monitor_invalidation {
            let invalidation_reasons =
                monitor_invalidation_reasons(monitor.trigger, output.status.success())
                    .ok_or(PilotError::MissingMonitorInvalidationTrigger { ordinal })?;
            let process = if output.status.success() {
                ProcessObservation::success(
                    std::process::id(),
                    child_pid,
                    TIMING_BINARY_ID,
                    output.status,
                )?
            } else {
                ProcessObservation::invalid_monitor_termination(
                    std::process::id(),
                    child_pid,
                    TIMING_BINARY_ID,
                    output.status,
                )?
            };
            return Ok(IdentityFreshProcessPilotOutcome::Stopped {
                stop: Box::new(IdentityFreshProcessPilotStop {
                    pilot_id: pilot_id.to_owned(),
                    graph_profile: graph_profile.as_str().to_owned(),
                    n,
                    sample_ordinal: ordinal,
                    status: RunStatus::Invalid,
                    invalidation_reasons,
                    process,
                    guard_preflight: guard,
                    child: None,
                    monitor: Some(monitor),
                    kill_error,
                    monitor_error,
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                }),
            });
        }
        if !output.status.success() {
            return Ok(IdentityFreshProcessPilotOutcome::Stopped {
                stop: Box::new(IdentityFreshProcessPilotStop {
                    pilot_id: pilot_id.to_owned(),
                    graph_profile: graph_profile.as_str().to_owned(),
                    n,
                    sample_ordinal: ordinal,
                    status: RunStatus::Invalid,
                    invalidation_reasons: vec![InvalidationReason::ChildAbnormalExit],
                    process: ProcessObservation::invalid_abnormal_exit(
                        std::process::id(),
                        child_pid,
                        TIMING_BINARY_ID,
                        output.status,
                    )?,
                    guard_preflight: guard,
                    child: None,
                    monitor: Some(monitor),
                    kill_error: None,
                    monitor_error: None,
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                }),
            });
        }
        let report = serde_json::from_slice::<IdentityTimingChildReport>(&output.stdout)
            .map_err(|source| PilotError::InvalidChildReport { ordinal, source })?;
        verify_child_report(
            &report,
            ordinal,
            &compiler_instance_id,
            child_pid,
            graph_profile,
            n,
        )?;
        let process = ProcessObservation::success(
            std::process::id(),
            child_pid,
            TIMING_BINARY_ID,
            output.status,
        )?;
        guard_preflights.push(guard);
        samples.push(IdentityMonitoredChildSample {
            status: RunStatus::Valid,
            invalidation_reasons: Vec::new(),
            process,
            child: report,
            monitor,
        });
    }

    let instance_ids = samples
        .iter()
        .map(|sample| sample.child.compiler_instance_id.as_str())
        .collect::<BTreeSet<_>>();
    if instance_ids.len() != FRESH_PROCESS_PILOT_SAMPLE_COUNT {
        return Err(PilotError::DuplicateCompilerInstanceId);
    }
    let semantic_digest_consistent = samples
        .windows(2)
        .all(|pair| pair[0].child.semantic_digest_sha256 == pair[1].child.semantic_digest_sha256);
    let (median_wall_time_ns, median_absolute_deviation_ns) =
        median_and_mad(samples.iter().map(|sample| sample.child.wall_time_ns))?;
    let clock_quantum_requirement_met = median_wall_time_ns >= required_median_wall_time_ns;
    let relative_mad_requirement_met =
        relative_mad_within_limit(median_wall_time_ns, median_absolute_deviation_ns);
    let measurement_quality_met =
        clock_quantum_requirement_met && relative_mad_requirement_met && semantic_digest_consistent;

    Ok(IdentityFreshProcessPilotOutcome::Completed {
        pilot: IdentityFreshProcessPilot {
            pilot_id: pilot_id.to_owned(),
            graph_profile: graph_profile.as_str().to_owned(),
            n,
            clock_quantum_ns,
            required_median_wall_time_ns,
            median_wall_time_ns,
            median_absolute_deviation_ns,
            clock_quantum_requirement_met,
            relative_mad_requirement_met,
            semantic_digest_consistent,
            measurement_quality_met,
            guard_preflight_evaluated: true,
            guard_preflights,
            samples,
        },
    })
}

pub fn run_base_scale_pilot_discovery(
    trusted: &TrustedContract,
    timing_executable: &Path,
    oracle_executable: &Path,
) -> Result<BaseScalePilotCheckpoint, PilotError> {
    run_base_scale_pilot_discovery_with_checkpoint_sink(
        trusted,
        timing_executable,
        oracle_executable,
        |_| Ok(()),
    )
}

pub fn run_base_scale_pilot_discovery_with_checkpoint_sink(
    trusted: &TrustedContract,
    timing_executable: &Path,
    oracle_executable: &Path,
    mut persist_checkpoint: impl FnMut(&BaseScalePilotCheckpoint) -> Result<(), PilotError>,
) -> Result<BaseScalePilotCheckpoint, PilotError> {
    validate_base_scale_contract(&trusted.workload_manifest)?;
    let clock_quantum_ns = observe_clock_quantum_ns()?;
    let required_median_wall_time_ns = clock_quantum_ns
        .checked_mul(CLOCK_QUANTUM_MULTIPLIER)
        .ok_or(PilotError::ArithmeticOverflow("required median wall time"))?;
    let guard_planner = ScalableGuardPlanner::from_trusted_contract(trusted)?;
    let mut memory_monitor = SystemMemoryMonitor::new()?;
    let mut selections = Vec::new();
    selections
        .try_reserve_exact(ScalableWorkloadId::ALL.len() * GraphProfileId::ALL.len())
        .map_err(|source| PilotError::AllocationFailed {
            field: "base-scale selections",
            source,
        })?;
    let mut runs = Vec::new();
    let mut oracle_runs = Vec::new();

    persist_base_scale_checkpoint(
        clock_quantum_ns,
        required_median_wall_time_ns,
        &selections,
        None,
        &runs,
        &oracle_runs,
        &mut persist_checkpoint,
    )?;

    for workload_id in ScalableWorkloadId::ALL {
        for graph_profile in GraphProfileId::ALL {
            let selection = {
                let mut persist_progress =
                    |runs: &[BaseScalePilotRun],
                     oracle_runs: &[BaseScaleOracleRun],
                     active_selection: &BaseScaleSelection| {
                        persist_base_scale_checkpoint(
                            clock_quantum_ns,
                            required_median_wall_time_ns,
                            &selections,
                            Some(active_selection),
                            runs,
                            oracle_runs,
                            &mut persist_checkpoint,
                        )
                    };
                discover_base_scale(
                    BaseScaleDiscoveryRequest {
                        timing_executable,
                        oracle_executable,
                        workload_id,
                        graph_profile,
                        required_median_wall_time_ns,
                    },
                    &guard_planner,
                    &mut memory_monitor,
                    &mut runs,
                    &mut oracle_runs,
                    &mut persist_progress,
                )?
            };
            selections.push(selection);
            persist_base_scale_checkpoint(
                clock_quantum_ns,
                required_median_wall_time_ns,
                &selections,
                None,
                &runs,
                &oracle_runs,
                &mut persist_checkpoint,
            )?;
        }
    }

    Ok(build_base_scale_pilot_checkpoint(
        clock_quantum_ns,
        required_median_wall_time_ns,
        &selections,
        None,
        &runs,
        &oracle_runs,
    ))
}

fn build_base_scale_pilot_checkpoint(
    clock_quantum_ns: u64,
    required_median_wall_time_ns: u64,
    selections: &[BaseScaleSelection],
    active_selection: Option<&BaseScaleSelection>,
    runs: &[BaseScalePilotRun],
    oracle_runs: &[BaseScaleOracleRun],
) -> BaseScalePilotCheckpoint {
    BaseScalePilotCheckpoint {
        schema: BASE_SCALE_PILOT_CHECKPOINT_SCHEMA.to_owned(),
        schema_version: BASE_SCALE_PILOT_CHECKPOINT_SCHEMA_VERSION,
        protocol_id: FORMAL_PROTOCOL_ID.to_owned(),
        clock_quantum_ns,
        required_median_wall_time_ns,
        selections: selections.to_vec(),
        active_selection: active_selection.cloned(),
        runs: runs.to_vec(),
        oracle_runs: oracle_runs.to_vec(),
    }
}

fn persist_base_scale_checkpoint(
    clock_quantum_ns: u64,
    required_median_wall_time_ns: u64,
    selections: &[BaseScaleSelection],
    active_selection: Option<&BaseScaleSelection>,
    runs: &[BaseScalePilotRun],
    oracle_runs: &[BaseScaleOracleRun],
    persist_checkpoint: &mut impl FnMut(&BaseScalePilotCheckpoint) -> Result<(), PilotError>,
) -> Result<(), PilotError> {
    persist_checkpoint(&build_base_scale_pilot_checkpoint(
        clock_quantum_ns,
        required_median_wall_time_ns,
        selections,
        active_selection,
        runs,
        oracle_runs,
    ))
}

fn discover_base_scale(
    request: BaseScaleDiscoveryRequest<'_>,
    guard_planner: &ScalableGuardPlanner,
    memory_monitor: &mut SystemMemoryMonitor,
    runs: &mut Vec<BaseScalePilotRun>,
    oracle_runs: &mut Vec<BaseScaleOracleRun>,
    persist_progress: &mut impl FnMut(
        &[BaseScalePilotRun],
        &[BaseScaleOracleRun],
        &BaseScaleSelection,
    ) -> Result<(), PilotError>,
) -> Result<BaseScaleSelection, PilotError> {
    let BaseScaleDiscoveryRequest {
        timing_executable,
        oracle_executable,
        workload_id,
        graph_profile,
        required_median_wall_time_ns,
    } = request;
    let mut selection = BaseScaleSelection {
        candidate_id: BASELINE_CANDIDATE_ID.to_owned(),
        workload_id,
        workload_revision: WORKLOAD_REVISION_V1,
        graph_profile: graph_profile.as_str().to_owned(),
        string_profile: BASE_SCALE_STRING_PROFILE.to_owned(),
        generator_version: GENERATOR_VERSION_V1,
        selection_rule: BASE_SCALE_SELECTION_RULE.to_owned(),
        pilot_levels: Vec::new(),
        b: NullableObservation::unavailable("base-scale-not-yet-selected"),
        terminal_guard_run_id: NullableObservation::unavailable("base-scale-not-yet-selected"),
    };
    persist_progress(runs, oracle_runs, &selection)?;
    let mut n = 1_u32;
    let mut previous_completed_level = None;

    loop {
        let mut retry_ordinal = 0_u32;
        let contributing_runs = loop {
            let attempt_id = base_scale_attempt_id(workload_id, graph_profile, n, retry_ordinal);
            let attempt_start = runs.len();
            let mut contributing_run_indexes = Vec::with_capacity(FRESH_PROCESS_PILOT_SAMPLE_COUNT);
            let mut retry_reasons = BTreeSet::new();

            for pilot_sample_position in 0..FRESH_PROCESS_PILOT_SAMPLE_COUNT {
                let run_id = format!("{attempt_id}/pilot-sample-{pilot_sample_position}");
                let descriptor = BaseScaleRunDescriptor {
                    run_id: run_id.clone(),
                    attempt_id: attempt_id.clone(),
                    retry_ordinal,
                    pilot_sample_position,
                    workload_id,
                    graph_profile,
                    n,
                };
                let guard = guard_planner.evaluate_pilot(
                    workload_id,
                    graph_profile,
                    n,
                    memory_monitor.observe()?,
                    previous_completed_level,
                )?;
                if !guard.allows_child_start {
                    invalidate_attempt_runs(
                        &mut runs[attempt_start..],
                        &[InvalidationReason::ResearchStopGuardrailTriggered],
                    );
                    runs.push(guarded_base_scale_run(descriptor, guard));
                    selection.b =
                        NullableObservation::unavailable("no-reliable-base-scale-before-guard");
                    selection.terminal_guard_run_id = NullableObservation::observed(run_id);
                    persist_progress(runs, oracle_runs, &selection)?;
                    return Ok(selection);
                }

                let compiler_instance_id = format!("{run_id}/compiler-instance");
                let execution = run_monitored_scalable_child(
                    timing_executable,
                    pilot_sample_position,
                    &compiler_instance_id,
                    workload_id,
                    graph_profile,
                    n,
                    guard.thresholds,
                )?;
                let run = scalable_execution_to_pilot_run(
                    descriptor,
                    compiler_instance_id,
                    guard,
                    execution,
                )?;
                let runtime_guard_triggered = run
                    .invalidation_reasons
                    .contains(&InvalidationReason::ResearchStopGuardrailTriggered)
                    || run.status == RunStatus::Guarded;
                if run.status == RunStatus::Valid {
                    contributing_run_indexes.push(runs.len());
                } else {
                    retry_reasons.extend(run.invalidation_reasons.iter().copied());
                }
                runs.push(run);
                if runtime_guard_triggered {
                    invalidate_attempt_runs(
                        &mut runs[attempt_start..],
                        &[InvalidationReason::ResearchStopGuardrailTriggered],
                    );
                    complete_selection_after_runtime_guard(&mut selection);
                    persist_progress(runs, oracle_runs, &selection)?;
                    return Ok(selection);
                }
                if !retry_reasons.is_empty() {
                    break;
                }
                persist_progress(runs, oracle_runs, &selection)?;
            }

            if retry_reasons.is_empty()
                && contributing_run_indexes.len() == FRESH_PROCESS_PILOT_SAMPLE_COUNT
            {
                break contributing_run_indexes;
            }
            let reasons = retry_reasons.into_iter().collect::<Vec<_>>();
            invalidate_attempt_runs(&mut runs[attempt_start..], &reasons);
            persist_progress(runs, oracle_runs, &selection)?;
            retry_ordinal = retry_ordinal
                .checked_add(1)
                .ok_or(PilotError::ArithmeticOverflow("base-scale retry ordinal"))?;
        };

        let oracle_run_id = format!(
            "{}/oracle",
            base_scale_attempt_id(workload_id, graph_profile, n, retry_ordinal)
        );
        let oracle_guard = guard_planner.evaluate_pilot(
            workload_id,
            graph_profile,
            n,
            memory_monitor.observe()?,
            previous_completed_level,
        )?;
        if !oracle_guard.allows_child_start {
            invalidate_contributing_runs(
                runs,
                &contributing_runs,
                &[InvalidationReason::ResearchStopGuardrailTriggered],
            )?;
            oracle_runs.push(guarded_base_scale_oracle_run(
                oracle_run_id.clone(),
                workload_id,
                graph_profile,
                n,
                oracle_guard,
            ));
            selection.b = NullableObservation::unavailable("no-reliable-base-scale-before-guard");
            selection.terminal_guard_run_id = NullableObservation::observed(oracle_run_id);
            persist_progress(runs, oracle_runs, &selection)?;
            return Ok(selection);
        }
        let expected_semantic_digest = contributing_run_semantic_digest(&contributing_runs, runs)?;
        let oracle_execution = run_monitored_scalable_oracle(
            oracle_executable,
            FRESH_PROCESS_PILOT_SAMPLE_COUNT,
            &oracle_run_id,
            workload_id,
            graph_profile,
            n,
            oracle_guard.thresholds,
        )?;
        let oracle_run = scalable_oracle_execution_to_run(
            oracle_run_id.clone(),
            workload_id,
            graph_profile,
            n,
            expected_semantic_digest,
            oracle_guard,
            oracle_execution,
        )?;
        let oracle_status = oracle_run.status;
        let oracle_runtime_guard_triggered = oracle_status == RunStatus::Guarded
            || oracle_run
                .invalidation_reasons
                .contains(&BaseScaleOracleInvalidationReason::ResearchStopGuardrailTriggered);
        oracle_runs.push(oracle_run);
        persist_progress(runs, oracle_runs, &selection)?;
        if oracle_status != RunStatus::Valid {
            if oracle_runtime_guard_triggered {
                invalidate_contributing_runs(
                    runs,
                    &contributing_runs,
                    &[InvalidationReason::ResearchStopGuardrailTriggered],
                )?;
                complete_selection_after_runtime_guard(&mut selection);
                persist_progress(runs, oracle_runs, &selection)?;
                return Ok(selection);
            }
            return Err(PilotError::OracleVerificationFailed {
                run_id: oracle_run_id,
                workload_id,
                graph_profile,
                n,
            });
        }

        let oracle_run = oracle_runs
            .last()
            .expect("the successful oracle run was just appended");
        let level = summarize_base_scale_level(
            n,
            &contributing_runs,
            runs,
            oracle_run,
            required_median_wall_time_ns,
        )?;
        let qualifies = level.qualifies;
        previous_completed_level = Some(level.completed_level_guard_observation);
        selection.pilot_levels.push(level);
        persist_progress(runs, oracle_runs, &selection)?;
        if qualifies {
            selection.b = NullableObservation::observed(n);
            selection.terminal_guard_run_id =
                NullableObservation::unavailable("base-scale-selected");
            persist_progress(runs, oracle_runs, &selection)?;
            return Ok(selection);
        }
        n = next_base_scale_n(n)?;
    }
}

fn next_base_scale_n(n: u32) -> Result<u32, PilotError> {
    n.checked_mul(2)
        .ok_or(PilotError::ArithmeticOverflow("base-scale strict doubling"))
}

fn complete_selection_after_runtime_guard(selection: &mut BaseScaleSelection) {
    selection.b = NullableObservation::unavailable("no-reliable-base-scale-runtime-guard");
    selection.terminal_guard_run_id =
        NullableObservation::unavailable("runtime-guard-is-not-preflight");
}

fn base_scale_attempt_id(
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    retry_ordinal: u32,
) -> String {
    format!(
        "pilot/{}/{}/n-{n}/attempt-{retry_ordinal}",
        workload_id.as_str(),
        graph_profile.as_str()
    )
}

fn guarded_base_scale_run(
    descriptor: BaseScaleRunDescriptor,
    guard_preflight: GuardPreflightReport,
) -> BaseScalePilotRun {
    let BaseScaleRunDescriptor {
        run_id,
        attempt_id,
        retry_ordinal,
        pilot_sample_position,
        workload_id,
        graph_profile,
        n,
    } = descriptor;
    BaseScalePilotRun {
        run_id,
        attempt_id,
        retry_ordinal,
        pilot_sample_position: u32::try_from(pilot_sample_position)
            .expect("seven pilot positions fit in u32"),
        run_kind: BaseScalePilotRunKind::GuardPreflight,
        workload_id,
        workload_revision: WORKLOAD_REVISION_V1,
        graph_profile: graph_profile.as_str().to_owned(),
        string_profile: BASE_SCALE_STRING_PROFILE.to_owned(),
        generator_version: GENERATOR_VERSION_V1,
        n,
        status: RunStatus::Guarded,
        invalidation_reasons: Vec::new(),
        process: ProcessObservation::guarded_before_start(std::process::id(), TIMING_BINARY_ID),
        guard_preflight,
        child: None,
        monitor: None,
        kill_error: None,
        monitor_error: None,
        stderr: String::new(),
    }
}

fn guarded_base_scale_oracle_run(
    run_id: String,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    guard_preflight: GuardPreflightReport,
) -> BaseScaleOracleRun {
    BaseScaleOracleRun {
        run_id,
        workload_id,
        workload_revision: WORKLOAD_REVISION_V1,
        graph_profile: graph_profile.as_str().to_owned(),
        string_profile: BASE_SCALE_STRING_PROFILE.to_owned(),
        generator_version: GENERATOR_VERSION_V1,
        n,
        status: RunStatus::Guarded,
        invalidation_reasons: Vec::new(),
        process: ProcessObservation::guarded_before_start(std::process::id(), ORACLE_BINARY_ID),
        guard_preflight,
        child: None,
        monitor: None,
        kill_error: None,
        monitor_error: None,
        stderr: String::new(),
    }
}

fn scalable_execution_to_pilot_run(
    descriptor: BaseScaleRunDescriptor,
    compiler_instance_id: String,
    guard_preflight: GuardPreflightReport,
    execution: MonitoredChildExecution,
) -> Result<BaseScalePilotRun, PilotError> {
    let BaseScaleRunDescriptor {
        run_id,
        attempt_id,
        retry_ordinal,
        pilot_sample_position,
        workload_id,
        graph_profile,
        n,
    } = descriptor;
    let (child_pid, output, monitor, monitor_invalid, kill_error, mut monitor_error) =
        match execution {
            MonitoredChildExecution::Exited {
                child_pid,
                output,
                monitor,
            } => (child_pid, output, monitor, false, None, None),
            MonitoredChildExecution::InvalidatedByMonitor {
                child_pid,
                output,
                monitor,
                kill_error,
                monitor_error,
            } => (child_pid, output, monitor, true, kill_error, monitor_error),
        };
    let mut invalidation_reasons = Vec::new();
    if monitor_invalid {
        invalidation_reasons =
            monitor_invalidation_reasons(monitor.trigger, output.status.success()).ok_or(
                PilotError::MissingMonitorInvalidationTrigger {
                    ordinal: pilot_sample_position,
                },
            )?;
    } else if !output.status.success() {
        invalidation_reasons.push(InvalidationReason::ChildAbnormalExit);
    }

    let mut report = None;
    let mut child_guarded = false;
    if !monitor_invalid && output.status.success() {
        match serde_json::from_slice::<ScalableTimingChildReport>(&output.stdout) {
            Ok(parsed) => match verify_scalable_child_report(
                &parsed,
                pilot_sample_position,
                &ScalableChildExpectation {
                    compiler_instance_id: &compiler_instance_id,
                    child_pid,
                    workload_id,
                    graph_profile,
                    n,
                    controlled_allocation_hard_ceiling_bytes: guard_preflight
                        .thresholds
                        .compiler_controlled_bytes,
                },
            ) {
                Ok(()) => {
                    child_guarded = parsed.outcome == ScalableTimingOutcome::GuardedInChild;
                    report = Some(parsed);
                }
                Err(error) => {
                    invalidation_reasons.push(InvalidationReason::ChildAbnormalExit);
                    monitor_error = Some(format!("invalid-child-report:{error}"));
                }
            },
            Err(error) => {
                invalidation_reasons.push(InvalidationReason::ChildAbnormalExit);
                monitor_error = Some(format!("invalid-child-report-json:{error}"));
            }
        }
    }
    if child_guarded {
        invalidation_reasons.push(InvalidationReason::ResearchStopGuardrailTriggered);
    }

    invalidation_reasons.sort_unstable();
    invalidation_reasons.dedup();
    let status = if child_guarded {
        RunStatus::Guarded
    } else if invalidation_reasons.is_empty() {
        RunStatus::Valid
    } else {
        RunStatus::Invalid
    };
    let process = if child_guarded {
        ProcessObservation::guarded_in_child(
            std::process::id(),
            child_pid,
            TIMING_BINARY_ID,
            output.status,
        )?
    } else if output.status.success() {
        ProcessObservation::success(
            std::process::id(),
            child_pid,
            TIMING_BINARY_ID,
            output.status,
        )?
    } else if monitor_invalid {
        ProcessObservation::invalid_monitor_termination(
            std::process::id(),
            child_pid,
            TIMING_BINARY_ID,
            output.status,
        )?
    } else {
        ProcessObservation::invalid_abnormal_exit(
            std::process::id(),
            child_pid,
            TIMING_BINARY_ID,
            output.status,
        )?
    };

    Ok(BaseScalePilotRun {
        run_id,
        attempt_id,
        retry_ordinal,
        pilot_sample_position: u32::try_from(pilot_sample_position)
            .expect("seven pilot positions fit in u32"),
        run_kind: BaseScalePilotRunKind::ColdInstance,
        workload_id,
        workload_revision: WORKLOAD_REVISION_V1,
        graph_profile: graph_profile.as_str().to_owned(),
        string_profile: BASE_SCALE_STRING_PROFILE.to_owned(),
        generator_version: GENERATOR_VERSION_V1,
        n,
        status,
        invalidation_reasons,
        process,
        guard_preflight,
        child: report,
        monitor: Some(monitor),
        kill_error,
        monitor_error,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn scalable_oracle_execution_to_run(
    run_id: String,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    expected_semantic_digest: &str,
    guard_preflight: GuardPreflightReport,
    execution: MonitoredChildExecution,
) -> Result<BaseScaleOracleRun, PilotError> {
    let (child_pid, output, monitor, monitor_invalid, kill_error, mut monitor_error) =
        match execution {
            MonitoredChildExecution::Exited {
                child_pid,
                output,
                monitor,
            } => (child_pid, output, monitor, false, None, None),
            MonitoredChildExecution::InvalidatedByMonitor {
                child_pid,
                output,
                monitor,
                kill_error,
                monitor_error,
            } => (child_pid, output, monitor, true, kill_error, monitor_error),
        };
    let mut invalidation_reasons = Vec::new();
    if monitor_invalid {
        invalidation_reasons =
            monitor_invalidation_reasons(monitor.trigger, output.status.success())
                .ok_or(PilotError::MissingMonitorInvalidationTrigger {
                    ordinal: FRESH_PROCESS_PILOT_SAMPLE_COUNT,
                })?
                .into_iter()
                .map(base_scale_oracle_invalidation_reason)
                .collect();
    } else if !output.status.success() {
        invalidation_reasons.push(BaseScaleOracleInvalidationReason::ChildAbnormalExit);
    }

    let mut report = None;
    let mut child_guarded = false;
    if !monitor_invalid && output.status.success() {
        match serde_json::from_slice::<ScalableOracleChildReport>(&output.stdout) {
            Ok(parsed) => match verify_scalable_oracle_report(
                &parsed,
                &ScalableOracleExpectation {
                    run_id: &run_id,
                    child_pid,
                    workload_id,
                    graph_profile,
                    n,
                    primary_record_count: guard_preflight.primary_record_count,
                    controlled_allocation_hard_ceiling_bytes: guard_preflight
                        .thresholds
                        .compiler_controlled_bytes,
                },
            ) {
                Ok(()) => {
                    child_guarded = parsed.outcome == ScalableOracleOutcome::GuardedInChild;
                    if parsed.outcome == ScalableOracleOutcome::Success
                        && parsed.semantic_digest_sha256.as_deref()
                            != Some(expected_semantic_digest)
                    {
                        invalidation_reasons
                            .push(BaseScaleOracleInvalidationReason::IndependentOracleMismatch);
                    }
                    report = Some(parsed);
                }
                Err(error) => {
                    invalidation_reasons.push(BaseScaleOracleInvalidationReason::ChildAbnormalExit);
                    monitor_error = Some(format!("invalid-oracle-report:{error}"));
                }
            },
            Err(error) => {
                invalidation_reasons.push(BaseScaleOracleInvalidationReason::ChildAbnormalExit);
                monitor_error = Some(format!("invalid-oracle-report-json:{error}"));
            }
        }
    }
    if child_guarded {
        invalidation_reasons
            .push(BaseScaleOracleInvalidationReason::ResearchStopGuardrailTriggered);
    }

    invalidation_reasons.sort_unstable();
    invalidation_reasons.dedup();
    let status = if child_guarded {
        RunStatus::Guarded
    } else if invalidation_reasons.is_empty() {
        RunStatus::Valid
    } else {
        RunStatus::Invalid
    };
    let process = if child_guarded {
        ProcessObservation::guarded_in_child(
            std::process::id(),
            child_pid,
            ORACLE_BINARY_ID,
            output.status,
        )?
    } else if output.status.success() {
        ProcessObservation::success(
            std::process::id(),
            child_pid,
            ORACLE_BINARY_ID,
            output.status,
        )?
    } else if monitor_invalid {
        ProcessObservation::invalid_monitor_termination(
            std::process::id(),
            child_pid,
            ORACLE_BINARY_ID,
            output.status,
        )?
    } else {
        ProcessObservation::invalid_abnormal_exit(
            std::process::id(),
            child_pid,
            ORACLE_BINARY_ID,
            output.status,
        )?
    };

    Ok(BaseScaleOracleRun {
        run_id,
        workload_id,
        workload_revision: WORKLOAD_REVISION_V1,
        graph_profile: graph_profile.as_str().to_owned(),
        string_profile: BASE_SCALE_STRING_PROFILE.to_owned(),
        generator_version: GENERATOR_VERSION_V1,
        n,
        status,
        invalidation_reasons,
        process,
        guard_preflight,
        child: report,
        monitor: Some(monitor),
        kill_error,
        monitor_error,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn base_scale_oracle_invalidation_reason(
    reason: InvalidationReason,
) -> BaseScaleOracleInvalidationReason {
    match reason {
        InvalidationReason::ResearchStopGuardrailTriggered => {
            BaseScaleOracleInvalidationReason::ResearchStopGuardrailTriggered
        }
        InvalidationReason::MonitoringGap => BaseScaleOracleInvalidationReason::MonitoringGap,
        InvalidationReason::ChildAbnormalExit => {
            BaseScaleOracleInvalidationReason::ChildAbnormalExit
        }
        InvalidationReason::PowerSourceChange
        | InvalidationReason::PowerPlanChange
        | InvalidationReason::VendorModeChange
        | InvalidationReason::SleepOrSessionLock
        | InvalidationReason::ThermalOrPowerThrottling
        | InvalidationReason::BackgroundCpuOverOneSecond
        | InvalidationReason::BackgroundWriteOver100Mib => {
            unreachable!("the child monitor cannot produce environment invalidation reasons")
        }
    }
}

fn invalidate_attempt_runs(runs: &mut [BaseScalePilotRun], reasons: &[InvalidationReason]) {
    for run in runs {
        if run.status != RunStatus::Valid {
            continue;
        }
        run.status = RunStatus::Invalid;
        run.invalidation_reasons.extend_from_slice(reasons);
        run.invalidation_reasons.sort_unstable();
        run.invalidation_reasons.dedup();
    }
}

fn invalidate_contributing_runs(
    runs: &mut [BaseScalePilotRun],
    indexes: &[usize],
    reasons: &[InvalidationReason],
) -> Result<(), PilotError> {
    for index in indexes {
        let run = runs
            .get_mut(*index)
            .ok_or(PilotError::InvalidContributingRunIndex { index: *index })?;
        if run.status == RunStatus::Valid {
            run.status = RunStatus::Invalid;
            run.invalidation_reasons.extend_from_slice(reasons);
            run.invalidation_reasons.sort_unstable();
            run.invalidation_reasons.dedup();
        }
    }
    Ok(())
}

fn contributing_run_semantic_digest<'a>(
    contributing_run_indexes: &[usize],
    runs: &'a [BaseScalePilotRun],
) -> Result<&'a str, PilotError> {
    let first_index = *contributing_run_indexes
        .first()
        .ok_or(PilotError::InvalidContributingRunSet)?;
    runs.get(first_index)
        .and_then(|run| run.child.as_ref())
        .and_then(|child| child.semantic_digest_sha256.as_deref())
        .ok_or(PilotError::InvalidContributingRunSet)
}

fn summarize_base_scale_level(
    n: u32,
    contributing_run_indexes: &[usize],
    runs: &[BaseScalePilotRun],
    oracle_run: &BaseScaleOracleRun,
    minimum_reliable_wall_time_ns: u64,
) -> Result<BaseScalePilotLevel, PilotError> {
    if contributing_run_indexes.len() != FRESH_PROCESS_PILOT_SAMPLE_COUNT {
        return Err(PilotError::WrongSampleCount {
            actual: contributing_run_indexes.len(),
        });
    }
    let contributing_runs = contributing_run_indexes
        .iter()
        .map(|index| {
            runs.get(*index)
                .ok_or(PilotError::InvalidContributingRunIndex { index: *index })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if contributing_runs.iter().any(|run| {
        run.status != RunStatus::Valid
            || run.run_kind != BaseScalePilotRunKind::ColdInstance
            || run.n != n
            || run.child.is_none()
    }) {
        return Err(PilotError::InvalidContributingRunSet);
    }
    let (wall_time_median_ns, wall_time_median_absolute_deviation_ns) =
        median_and_mad(contributing_runs.iter().map(|run| {
            run.child
                .as_ref()
                .expect("validated child")
                .wall_time_ns
                .expect("validated success child")
        }))?;
    let semantic_digest = contributing_runs[0]
        .child
        .as_ref()
        .expect("validated child")
        .semantic_digest_sha256
        .as_ref()
        .expect("validated success child")
        .clone();
    let all_semantic_digests_equal = contributing_runs.iter().all(|run| {
        run.child
            .as_ref()
            .and_then(|child| child.semantic_digest_sha256.as_ref())
            .is_some_and(|digest| digest == &semantic_digest)
    });
    if oracle_run.status != RunStatus::Valid
        || oracle_run.n != n
        || oracle_run.child.as_ref().is_none_or(|child| {
            child.outcome != ScalableOracleOutcome::Success
                || child.semantic_digest_sha256.as_deref() != Some(semantic_digest.as_str())
                || !child.complete_counts_equal
                || !child.complete_typed_output_equal
        })
    {
        return Err(PilotError::InvalidOracleRunSet);
    }
    let oracle_child = oracle_run.child.as_ref().expect("validated oracle child");
    let oracle_semantic_digest = oracle_child
        .semantic_digest_sha256
        .as_ref()
        .expect("validated oracle digest")
        .clone();
    let all_guards_clear = contributing_runs
        .iter()
        .all(|run| run.guard_preflight.allows_child_start)
        && oracle_run.guard_preflight.allows_child_start;
    let completed_level_guard_observation =
        completed_level_guard_observation(n, &contributing_runs)?;
    let qualifies = wall_time_median_ns >= minimum_reliable_wall_time_ns
        && relative_mad_within_limit(wall_time_median_ns, wall_time_median_absolute_deviation_ns)
        && all_semantic_digests_equal
        && oracle_child.complete_counts_equal
        && oracle_child.complete_typed_output_equal
        && all_guards_clear;
    Ok(BaseScalePilotLevel {
        n,
        contributing_run_ids: contributing_runs
            .iter()
            .map(|run| run.run_id.clone())
            .collect(),
        aggregation_method: BASE_SCALE_AGGREGATION_METHOD.to_owned(),
        wall_time_median_ns,
        wall_time_median_absolute_deviation_ns,
        minimum_reliable_wall_time_ns,
        semantic_digest,
        all_semantic_digests_equal,
        oracle_run_id: oracle_run.run_id.clone(),
        oracle_semantic_digest,
        complete_counts_equal: oracle_child.complete_counts_equal,
        complete_typed_output_equal: oracle_child.complete_typed_output_equal,
        all_guards_clear,
        completed_level_guard_observation,
        qualifies,
    })
}

fn completed_level_guard_observation(
    n: u32,
    contributing_runs: &[&BaseScalePilotRun],
) -> Result<GuardCompletedLevelObservation, PilotError> {
    let primary_record_count = contributing_runs
        .first()
        .ok_or(PilotError::InvalidContributingRunSet)?
        .guard_preflight
        .primary_record_count;
    if contributing_runs
        .iter()
        .any(|run| run.guard_preflight.primary_record_count != primary_record_count)
    {
        return Err(PilotError::InvalidContributingRunSet);
    }

    let guard_peaks = contributing_runs
        .iter()
        .map(|run| {
            run.child
                .as_ref()
                .and_then(|child| child.guard_peak_live_requested_bytes)
                .ok_or(PilotError::InvalidContributingRunSet)
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if guard_peaks.len() != 1 {
        return Err(PilotError::InconsistentGuardPeakObservation);
    }
    let peak_live_requested_bytes = *guard_peaks
        .first()
        .ok_or(PilotError::InvalidContributingRunSet)?;
    let private_bytes = contributing_runs
        .iter()
        .map(|run| {
            run.monitor
                .as_ref()
                .and_then(|monitor| monitor.peak_private_bytes.value)
                .ok_or(PilotError::InvalidContributingRunSet)
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .ok_or(PilotError::InvalidContributingRunSet)?;
    let wall_time_ns = contributing_runs
        .iter()
        .map(|run| {
            run.child
                .as_ref()
                .and_then(|child| child.wall_time_ns)
                .ok_or(PilotError::InvalidContributingRunSet)
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .ok_or(PilotError::InvalidContributingRunSet)?;
    if peak_live_requested_bytes == 0 || private_bytes == 0 || wall_time_ns == 0 {
        return Err(PilotError::InvalidContributingRunSet);
    }
    Ok(GuardCompletedLevelObservation {
        n,
        primary_record_count,
        peak_live_requested_bytes,
        private_bytes,
        wall_time_ns,
    })
}

#[derive(Debug)]
pub(crate) enum MonitoredChildExecution {
    Exited {
        child_pid: u32,
        output: std::process::Output,
        monitor: ChildProcessMonitorReport,
    },
    InvalidatedByMonitor {
        child_pid: u32,
        output: std::process::Output,
        monitor: ChildProcessMonitorReport,
        kill_error: Option<String>,
        monitor_error: Option<String>,
    },
}

#[derive(Clone, Copy, Debug)]
struct ChildMonitorSnapshot {
    observation_count: u64,
    last_private_bytes: u64,
    peak_private_bytes: u64,
    elapsed: Duration,
}

fn run_monitored_identity_child(
    timing_executable: &Path,
    ordinal: usize,
    compiler_instance_id: &str,
    graph_profile: GraphProfileId,
    n: u32,
    thresholds: GuardThresholds,
) -> Result<MonitoredChildExecution, PilotError> {
    let mut memory_monitor = ChildProcessMemoryMonitor::new()?;
    let mut command = Command::new(timing_executable);
    command
        .arg("run-identity-smoke")
        .arg(compiler_instance_id)
        .arg(graph_profile.as_str())
        .arg(n.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = ContainedChild::spawn(&mut command)
        .map_err(|source| PilotError::ChildSpawn { ordinal, source })?;
    run_monitored_child_with_observer(child, ordinal, thresholds, |child_pid| {
        memory_monitor.observe(child_pid)
    })
}

fn run_monitored_scalable_child(
    timing_executable: &Path,
    ordinal: usize,
    compiler_instance_id: &str,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    thresholds: GuardThresholds,
) -> Result<MonitoredChildExecution, PilotError> {
    let mut memory_monitor = ChildProcessMemoryMonitor::new()?;
    let mut command = Command::new(timing_executable);
    command
        .arg("run")
        .arg(compiler_instance_id)
        .arg(workload_id.as_str())
        .arg(graph_profile.as_str())
        .arg(n.to_string())
        .arg(thresholds.compiler_controlled_bytes.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = ContainedChild::spawn(&mut command)
        .map_err(|source| PilotError::ChildSpawn { ordinal, source })?;
    run_monitored_child_with_observer(child, ordinal, thresholds, |child_pid| {
        memory_monitor.observe(child_pid)
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_monitored_scalable_role_child(
    executable: &Path,
    ordinal: usize,
    command_name: &str,
    compiler_instance_id: &str,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    thresholds: GuardThresholds,
) -> Result<MonitoredChildExecution, PilotError> {
    let mut memory_monitor = ChildProcessMemoryMonitor::new()?;
    let mut command = Command::new(executable);
    command
        .arg(command_name)
        .arg(compiler_instance_id)
        .arg(workload_id.as_str())
        .arg(graph_profile.as_str())
        .arg(n.to_string())
        .arg(thresholds.compiler_controlled_bytes.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = ContainedChild::spawn(&mut command)
        .map_err(|source| PilotError::ChildSpawn { ordinal, source })?;
    run_monitored_child_with_observer(child, ordinal, thresholds, |child_pid| {
        memory_monitor.observe(child_pid)
    })
}

pub(crate) fn run_monitored_scalable_oracle(
    oracle_executable: &Path,
    ordinal: usize,
    oracle_run_id: &str,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    thresholds: GuardThresholds,
) -> Result<MonitoredChildExecution, PilotError> {
    let mut memory_monitor = ChildProcessMemoryMonitor::new()?;
    let mut command = Command::new(oracle_executable);
    command
        .arg("run-scalable")
        .arg(oracle_run_id)
        .arg(workload_id.as_str())
        .arg(graph_profile.as_str())
        .arg(n.to_string())
        .arg(thresholds.compiler_controlled_bytes.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = ContainedChild::spawn(&mut command)
        .map_err(|source| PilotError::ChildSpawn { ordinal, source })?;
    run_monitored_child_with_observer(child, ordinal, thresholds, |child_pid| {
        memory_monitor.observe(child_pid)
    })
}

fn run_monitored_child_with_observer(
    mut child: ContainedChild,
    ordinal: usize,
    thresholds: GuardThresholds,
    mut observe_child: impl FnMut(u32) -> Result<Option<ChildProcessMemoryObservation>, GuardError>,
) -> Result<MonitoredChildExecution, PilotError> {
    let child_pid = child.id();

    let handshake_started = Instant::now();
    let initial_observation = loop {
        match observe_child(child_pid) {
            Ok(Some(observation)) => break observation,
            Ok(None) => {}
            Err(error) => {
                let report = ChildProcessMonitorReport {
                    observation_count: 0,
                    last_private_bytes: NullableObservation::unavailable("monitoring-gap"),
                    peak_private_bytes: NullableObservation::unavailable("monitoring-gap"),
                    elapsed_wall_time_ns: duration_ns(handshake_started.elapsed())?,
                    trigger: Some(ChildMonitorTrigger::MonitoringGap),
                };
                return terminate_monitored_child(
                    child,
                    ordinal,
                    child_pid,
                    report,
                    Some(error.to_string()),
                );
            }
        }
        if let Some(status) = try_wait_child(&mut child, ordinal)? {
            let output = child
                .wait_with_output()
                .map_err(|source| PilotError::ChildWait { ordinal, source })?;
            debug_assert_eq!(status.code(), output.status.code());
            return Ok(MonitoredChildExecution::InvalidatedByMonitor {
                child_pid,
                output,
                monitor: ChildProcessMonitorReport {
                    observation_count: 0,
                    last_private_bytes: NullableObservation::unavailable("monitoring-gap"),
                    peak_private_bytes: NullableObservation::unavailable("monitoring-gap"),
                    elapsed_wall_time_ns: duration_ns(handshake_started.elapsed())?,
                    trigger: Some(ChildMonitorTrigger::MonitoringGap),
                },
                kill_error: None,
                monitor_error: Some("child-exited-before-initial-monitor-observation".to_owned()),
            });
        }
        if handshake_started.elapsed() >= CHILD_MONITOR_HANDSHAKE_TIMEOUT {
            let report = ChildProcessMonitorReport {
                observation_count: 0,
                last_private_bytes: NullableObservation::unavailable("monitoring-gap"),
                peak_private_bytes: NullableObservation::unavailable("monitoring-gap"),
                elapsed_wall_time_ns: duration_ns(handshake_started.elapsed())?,
                trigger: Some(ChildMonitorTrigger::MonitoringGap),
            };
            return terminate_monitored_child(
                child,
                ordinal,
                child_pid,
                report,
                Some("initial-monitor-observation-timeout".to_owned()),
            );
        }
        thread::sleep(CHILD_MONITOR_POLL_INTERVAL);
    };

    let mut observation_count = 1_u64;
    let mut last_private_bytes = initial_observation.private_bytes;
    let mut peak_private_bytes = initial_observation.private_bytes;
    if let Some(trigger) =
        evaluate_child_monitor_trigger(thresholds, initial_observation, Duration::ZERO)?
    {
        let report = ChildProcessMonitorReport {
            observation_count,
            last_private_bytes: NullableObservation::observed(last_private_bytes),
            peak_private_bytes: NullableObservation::observed(peak_private_bytes),
            elapsed_wall_time_ns: 0,
            trigger: Some(trigger),
        };
        return terminate_monitored_child(child, ordinal, child_pid, report, None);
    }

    let started = Instant::now();
    let Some(mut child_stdin) = child.take_stdin() else {
        let report = ChildProcessMonitorReport {
            observation_count,
            last_private_bytes: NullableObservation::observed(last_private_bytes),
            peak_private_bytes: NullableObservation::observed(peak_private_bytes),
            elapsed_wall_time_ns: duration_ns(started.elapsed())?,
            trigger: Some(ChildMonitorTrigger::MonitoringGap),
        };
        return terminate_monitored_child(
            child,
            ordinal,
            child_pid,
            report,
            Some("missing-child-stdin".to_owned()),
        );
    };
    if let Err(source) = child_stdin.write_all(&[CHILD_START_SIGNAL]) {
        let report = ChildProcessMonitorReport {
            observation_count,
            last_private_bytes: NullableObservation::observed(last_private_bytes),
            peak_private_bytes: NullableObservation::observed(peak_private_bytes),
            elapsed_wall_time_ns: duration_ns(started.elapsed())?,
            trigger: Some(ChildMonitorTrigger::MonitoringGap),
        };
        return terminate_monitored_child(
            child,
            ordinal,
            child_pid,
            report,
            Some(format!("child-start-signal-write:{source}")),
        );
    }
    drop(child_stdin);

    loop {
        let exit_status = try_wait_child(&mut child, ordinal)?;
        let elapsed = started.elapsed();
        if let Some(status) = exit_status {
            return finish_observed_child_exit(
                child,
                ordinal,
                child_pid,
                status,
                thresholds,
                ChildMonitorSnapshot {
                    observation_count,
                    last_private_bytes,
                    peak_private_bytes,
                    elapsed,
                },
            );
        }
        if wall_time_limit_reached(thresholds, elapsed)? {
            let report = ChildProcessMonitorReport {
                observation_count,
                last_private_bytes: NullableObservation::observed(last_private_bytes),
                peak_private_bytes: NullableObservation::observed(peak_private_bytes),
                elapsed_wall_time_ns: duration_ns(elapsed)?,
                trigger: Some(ChildMonitorTrigger::WallTime),
            };
            return terminate_monitored_child(child, ordinal, child_pid, report, None);
        }

        let observation = match observe_child(child_pid) {
            Ok(Some(observation)) => observation,
            Ok(None) => {
                if let Some(status) = try_wait_child(&mut child, ordinal)? {
                    let exit_poll_elapsed = started.elapsed();
                    return finish_observed_child_exit(
                        child,
                        ordinal,
                        child_pid,
                        status,
                        thresholds,
                        ChildMonitorSnapshot {
                            observation_count,
                            last_private_bytes,
                            peak_private_bytes,
                            elapsed: exit_poll_elapsed,
                        },
                    );
                }
                let report = ChildProcessMonitorReport {
                    observation_count,
                    last_private_bytes: NullableObservation::observed(last_private_bytes),
                    peak_private_bytes: NullableObservation::observed(peak_private_bytes),
                    elapsed_wall_time_ns: duration_ns(started.elapsed())?,
                    trigger: Some(ChildMonitorTrigger::MonitoringGap),
                };
                return terminate_monitored_child(
                    child,
                    ordinal,
                    child_pid,
                    report,
                    Some("child-process-disappeared-before-exit-status".to_owned()),
                );
            }
            Err(error) => {
                let report = ChildProcessMonitorReport {
                    observation_count,
                    last_private_bytes: NullableObservation::observed(last_private_bytes),
                    peak_private_bytes: NullableObservation::observed(peak_private_bytes),
                    elapsed_wall_time_ns: duration_ns(started.elapsed())?,
                    trigger: Some(ChildMonitorTrigger::MonitoringGap),
                };
                return terminate_monitored_child(
                    child,
                    ordinal,
                    child_pid,
                    report,
                    Some(error.to_string()),
                );
            }
        };
        observation_count =
            observation_count
                .checked_add(1)
                .ok_or(PilotError::ArithmeticOverflow(
                    "child monitor observation count",
                ))?;
        last_private_bytes = observation.private_bytes;
        peak_private_bytes = peak_private_bytes.max(observation.private_bytes);
        let elapsed = started.elapsed();
        if let Some(trigger) = evaluate_child_monitor_trigger(thresholds, observation, elapsed)? {
            let report = ChildProcessMonitorReport {
                observation_count,
                last_private_bytes: NullableObservation::observed(last_private_bytes),
                peak_private_bytes: NullableObservation::observed(peak_private_bytes),
                elapsed_wall_time_ns: duration_ns(elapsed)?,
                trigger: Some(trigger),
            };
            return terminate_monitored_child(child, ordinal, child_pid, report, None);
        }
        thread::sleep(CHILD_MONITOR_POLL_INTERVAL);
    }
}

fn finish_observed_child_exit(
    child: ContainedChild,
    ordinal: usize,
    child_pid: u32,
    status: std::process::ExitStatus,
    thresholds: GuardThresholds,
    snapshot: ChildMonitorSnapshot,
) -> Result<MonitoredChildExecution, PilotError> {
    let elapsed_wall_time_ns = duration_ns(snapshot.elapsed)?;
    let trigger = wall_time_limit_reached(thresholds, snapshot.elapsed)?
        .then_some(ChildMonitorTrigger::WallTime);
    let output = child
        .wait_with_output()
        .map_err(|source| PilotError::ChildWait { ordinal, source })?;
    debug_assert_eq!(status.code(), output.status.code());
    let monitor = ChildProcessMonitorReport {
        observation_count: snapshot.observation_count,
        last_private_bytes: NullableObservation::observed(snapshot.last_private_bytes),
        peak_private_bytes: NullableObservation::observed(snapshot.peak_private_bytes),
        elapsed_wall_time_ns,
        trigger,
    };
    if trigger.is_some() {
        Ok(MonitoredChildExecution::InvalidatedByMonitor {
            child_pid,
            output,
            monitor,
            kill_error: None,
            monitor_error: None,
        })
    } else {
        Ok(MonitoredChildExecution::Exited {
            child_pid,
            output,
            monitor,
        })
    }
}

fn evaluate_child_monitor_trigger(
    thresholds: GuardThresholds,
    observation: ChildProcessMemoryObservation,
    elapsed: Duration,
) -> Result<Option<ChildMonitorTrigger>, PilotError> {
    if observation.private_bytes >= thresholds.private_bytes {
        return Ok(Some(ChildMonitorTrigger::PrivateBytes));
    }
    if wall_time_limit_reached(thresholds, elapsed)? {
        return Ok(Some(ChildMonitorTrigger::WallTime));
    }
    if observation.available_physical_memory_bytes
        < thresholds.minimum_available_physical_memory_bytes
    {
        return Ok(Some(ChildMonitorTrigger::AvailablePhysicalMemory));
    }
    Ok(None)
}

fn wall_time_limit_reached(
    thresholds: GuardThresholds,
    elapsed: Duration,
) -> Result<bool, PilotError> {
    Ok(duration_ns(elapsed)? >= thresholds.wall_time_ns)
}

pub(crate) fn monitor_invalidation_reasons(
    trigger: Option<ChildMonitorTrigger>,
    child_exit_succeeded: bool,
) -> Option<Vec<InvalidationReason>> {
    let mut reasons = match trigger? {
        ChildMonitorTrigger::PrivateBytes
        | ChildMonitorTrigger::WallTime
        | ChildMonitorTrigger::AvailablePhysicalMemory => {
            vec![InvalidationReason::ResearchStopGuardrailTriggered]
        }
        ChildMonitorTrigger::MonitoringGap => vec![InvalidationReason::MonitoringGap],
    };
    if !child_exit_succeeded {
        reasons.push(InvalidationReason::ChildAbnormalExit);
    }
    Some(reasons)
}

fn terminate_monitored_child(
    mut child: ContainedChild,
    ordinal: usize,
    child_pid: u32,
    report: ChildProcessMonitorReport,
    monitor_error: Option<String>,
) -> Result<MonitoredChildExecution, PilotError> {
    let termination = terminate_child_with_deadline(
        &mut child,
        CHILD_TERMINATION_TIMEOUT,
        CHILD_TERMINATION_RETRY_INTERVAL,
    )
    .map_err(|failure| match failure {
        ChildTerminationFailure::StatusPoll(source) => PilotError::ChildTerminationStatusPoll {
            ordinal,
            child_pid,
            source,
        },
        ChildTerminationFailure::ContainmentEscalation(source) => {
            PilotError::ChildContainmentEscalation {
                ordinal,
                child_pid,
                source,
            }
        }
        ChildTerminationFailure::DeadlineExceeded {
            last_termination_error,
        } => PilotError::ChildTerminationDeadlineExceeded {
            ordinal,
            child_pid,
            total_timeout_ms: CHILD_TERMINATION_TOTAL_TIMEOUT_MS,
            last_termination_error,
        },
    })?;
    let output = child
        .wait_with_output()
        .map_err(|source| PilotError::ChildWait { ordinal, source })?;
    let monitor_error = if termination.containment_escalated {
        Some(match monitor_error {
            Some(error) => format!("{error};os-containment-escalated"),
            None => "os-containment-escalated".to_owned(),
        })
    } else {
        monitor_error
    };
    Ok(MonitoredChildExecution::InvalidatedByMonitor {
        child_pid,
        output,
        monitor: report,
        kill_error: termination.prior_termination_error,
        monitor_error,
    })
}

trait ChildTerminationControl {
    fn request_termination(&mut self) -> std::io::Result<()>;
    fn has_exited(&mut self) -> std::io::Result<bool>;
    fn escalate_containment(&mut self) -> std::io::Result<()>;
}

impl ChildTerminationControl for ContainedChild {
    fn request_termination(&mut self) -> std::io::Result<()> {
        ContainedChild::request_termination(self)
    }

    fn has_exited(&mut self) -> std::io::Result<bool> {
        self.try_wait().map(|status| status.is_some())
    }

    fn escalate_containment(&mut self) -> std::io::Result<()> {
        ContainedChild::escalate_containment(self).map(|_| ())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ChildTerminationCompletion {
    prior_termination_error: Option<String>,
    containment_escalated: bool,
}

#[derive(Debug)]
enum ChildTerminationFailure {
    StatusPoll(std::io::Error),
    ContainmentEscalation(std::io::Error),
    DeadlineExceeded {
        last_termination_error: Option<String>,
    },
}

fn terminate_child_with_deadline(
    child: &mut impl ChildTerminationControl,
    timeout: Duration,
    retry_interval: Duration,
) -> Result<ChildTerminationCompletion, ChildTerminationFailure> {
    let started = Instant::now();
    let mut termination_requested = false;
    let mut prior_termination_error = None;

    loop {
        if !termination_requested {
            match child.request_termination() {
                Ok(()) => termination_requested = true,
                Err(source) => prior_termination_error = Some(source.to_string()),
            }
        }

        if child
            .has_exited()
            .map_err(ChildTerminationFailure::StatusPoll)?
        {
            return Ok(ChildTerminationCompletion {
                prior_termination_error,
                containment_escalated: false,
            });
        }

        let elapsed = started.elapsed();
        if elapsed >= timeout {
            break;
        }
        thread::sleep(retry_interval.min(timeout.saturating_sub(elapsed)));
    }

    child
        .escalate_containment()
        .map_err(ChildTerminationFailure::ContainmentEscalation)?;
    let containment_started = Instant::now();
    loop {
        if child
            .has_exited()
            .map_err(ChildTerminationFailure::StatusPoll)?
        {
            return Ok(ChildTerminationCompletion {
                prior_termination_error,
                containment_escalated: true,
            });
        }
        let elapsed = containment_started.elapsed();
        if elapsed >= timeout {
            return Err(ChildTerminationFailure::DeadlineExceeded {
                last_termination_error: prior_termination_error,
            });
        }
        thread::sleep(retry_interval.min(timeout.saturating_sub(elapsed)));
    }
}

fn terminate_child_best_effort(child: &mut ContainedChild) {
    let _ = terminate_child_with_deadline(
        child,
        CHILD_TERMINATION_TIMEOUT,
        CHILD_TERMINATION_RETRY_INTERVAL,
    );
}

fn try_wait_child(
    child: &mut ContainedChild,
    ordinal: usize,
) -> Result<Option<std::process::ExitStatus>, PilotError> {
    match child.try_wait() {
        Ok(status) => Ok(status),
        Err(source) => {
            terminate_child_best_effort(child);
            Err(PilotError::ChildWait { ordinal, source })
        }
    }
}

fn duration_ns(duration: Duration) -> Result<u64, PilotError> {
    u64::try_from(duration.as_nanos())
        .map_err(|_| PilotError::ArithmeticOverflow("child monitor wall time"))
}

fn verify_child_report(
    report: &IdentityTimingChildReport,
    ordinal: usize,
    expected_instance_id: &str,
    expected_child_pid: u32,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<(), PilotError> {
    if report.schema != IDENTITY_TIMING_CHILD_SCHEMA {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "schema",
        });
    }
    if report.schema_version != IDENTITY_TIMING_CHILD_SCHEMA_VERSION {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "schemaVersion",
        });
    }
    if report.binary_id != TIMING_BINARY_ID {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "binaryId",
        });
    }
    if report.allocation_instrumentation_enabled {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "allocationInstrumentationEnabled",
        });
    }
    if report.compiler_instance_id != expected_instance_id {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "compilerInstanceId",
        });
    }
    if report.child_pid == 0 || report.child_pid != expected_child_pid {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "childPid",
        });
    }
    if report.graph_profile != graph_profile.as_str() {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "graphProfile",
        });
    }
    if report.n != n {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "n",
        });
    }
    if report.wall_time_ns == 0 {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "wallTimeNs",
        });
    }
    if report.semantic_digest_sha256.len() != 64
        || !report
            .semantic_digest_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "semanticDigestSha256",
        });
    }
    Ok(())
}

fn verify_scalable_child_report(
    report: &ScalableTimingChildReport,
    ordinal: usize,
    expected: &ScalableChildExpectation<'_>,
) -> Result<(), PilotError> {
    if report.schema != SCALABLE_TIMING_CHILD_SCHEMA {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "schema",
        });
    }
    if report.schema_version != SCALABLE_TIMING_CHILD_SCHEMA_VERSION {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "schemaVersion",
        });
    }
    if report.binary_id != TIMING_BINARY_ID {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "binaryId",
        });
    }
    if report.allocation_instrumentation_enabled {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "allocationInstrumentationEnabled",
        });
    }
    if report.compiler_instance_id != expected.compiler_instance_id {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "compilerInstanceId",
        });
    }
    if report.child_pid == 0 || report.child_pid != expected.child_pid {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "childPid",
        });
    }
    if report.workload_id != expected.workload_id {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "workloadId",
        });
    }
    if report.workload_revision != WORKLOAD_REVISION_V1 {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "workloadRevision",
        });
    }
    if report.graph_profile != expected.graph_profile.as_str() {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "graphProfile",
        });
    }
    if report.string_profile != BASE_SCALE_STRING_PROFILE {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "stringProfile",
        });
    }
    if report.generator_version != GENERATOR_VERSION_V1 {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "generatorVersion",
        });
    }
    if report.n != expected.n {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "n",
        });
    }
    if report.controlled_allocation_hard_ceiling_bytes
        != expected.controlled_allocation_hard_ceiling_bytes
    {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "controlledAllocationHardCeilingBytes",
        });
    }
    match report.outcome {
        ScalableTimingOutcome::Success => {
            if report.guard_peak_live_requested_bytes.is_none_or(|value| {
                value == 0 || value > expected.controlled_allocation_hard_ceiling_bytes
            }) {
                return Err(PilotError::ChildReportMismatch {
                    ordinal,
                    field: "guardPeakLiveRequestedBytes",
                });
            }
            if report.wall_time_ns.is_none_or(|value| value == 0) {
                return Err(PilotError::ChildReportMismatch {
                    ordinal,
                    field: "wallTimeNs",
                });
            }
            if report
                .semantic_digest_sha256
                .as_deref()
                .is_none_or(|digest| !valid_sha256(digest))
            {
                return Err(PilotError::ChildReportMismatch {
                    ordinal,
                    field: "semanticDigestSha256",
                });
            }
            if report.controlled_allocation_guard.is_some() {
                return Err(PilotError::ChildReportMismatch {
                    ordinal,
                    field: "controlledAllocationGuard",
                });
            }
        }
        ScalableTimingOutcome::GuardedInChild => {
            if report.guard_peak_live_requested_bytes.is_some()
                || report.wall_time_ns.is_some()
                || report.semantic_digest_sha256.is_some()
                || report
                    .controlled_allocation_guard
                    .as_ref()
                    .is_none_or(|guard| {
                        !valid_controlled_allocation_guard(
                            guard,
                            expected.controlled_allocation_hard_ceiling_bytes,
                        )
                    })
            {
                return Err(PilotError::ChildReportMismatch {
                    ordinal,
                    field: "guardedInChild",
                });
            }
        }
    }
    Ok(())
}

fn verify_scalable_oracle_report(
    report: &ScalableOracleChildReport,
    expected: &ScalableOracleExpectation<'_>,
) -> Result<(), PilotError> {
    let mismatch = |field| PilotError::OracleReportMismatch { field };
    if report.schema != SCALABLE_ORACLE_CHILD_SCHEMA {
        return Err(mismatch("schema"));
    }
    if report.schema_version != SCALABLE_ORACLE_CHILD_SCHEMA_VERSION {
        return Err(mismatch("schemaVersion"));
    }
    if report.binary_id != ORACLE_BINARY_ID {
        return Err(mismatch("binaryId"));
    }
    if report.oracle_run_id != expected.run_id {
        return Err(mismatch("oracleRunId"));
    }
    if report.child_pid == 0 || report.child_pid != expected.child_pid {
        return Err(mismatch("childPid"));
    }
    if report.workload_id != expected.workload_id {
        return Err(mismatch("workloadId"));
    }
    if report.workload_revision != WORKLOAD_REVISION_V1 {
        return Err(mismatch("workloadRevision"));
    }
    if report.graph_profile != expected.graph_profile.as_str() {
        return Err(mismatch("graphProfile"));
    }
    if report.string_profile != BASE_SCALE_STRING_PROFILE {
        return Err(mismatch("stringProfile"));
    }
    if report.generator_version != GENERATOR_VERSION_V1 {
        return Err(mismatch("generatorVersion"));
    }
    if report.n != expected.n {
        return Err(mismatch("n"));
    }
    if report.controlled_allocation_hard_ceiling_bytes
        != expected.controlled_allocation_hard_ceiling_bytes
    {
        return Err(mismatch("controlledAllocationHardCeilingBytes"));
    }
    match report.outcome {
        ScalableOracleOutcome::Success => {
            if report.guard_peak_live_requested_bytes.is_none_or(|peak| {
                peak == 0 || peak > expected.controlled_allocation_hard_ceiling_bytes
            }) {
                return Err(mismatch("guardPeakLiveRequestedBytes"));
            }
            if report.primary_record_count != Some(expected.primary_record_count) {
                return Err(mismatch("primaryRecordCount"));
            }
            if report
                .semantic_digest_sha256
                .as_deref()
                .is_none_or(|digest| !valid_sha256(digest))
            {
                return Err(mismatch("semanticDigestSha256"));
            }
            if !report.complete_counts_equal || !report.complete_typed_output_equal {
                return Err(mismatch("completeOracleEquality"));
            }
            if report.controlled_allocation_guard.is_some() {
                return Err(mismatch("controlledAllocationGuard"));
            }
        }
        ScalableOracleOutcome::GuardedInChild => {
            if report.guard_peak_live_requested_bytes.is_some()
                || report.primary_record_count.is_some()
                || report.semantic_digest_sha256.is_some()
                || report.complete_counts_equal
                || report.complete_typed_output_equal
                || report
                    .controlled_allocation_guard
                    .as_ref()
                    .is_none_or(|guard| {
                        !valid_controlled_allocation_guard(
                            guard,
                            expected.controlled_allocation_hard_ceiling_bytes,
                        )
                    })
            {
                return Err(mismatch("guardedInChild"));
            }
        }
    }
    Ok(())
}

fn valid_controlled_allocation_guard(
    guard: &crate::ControlledAllocationGuardReport,
    expected_hard_ceiling_bytes: u64,
) -> bool {
    guard.hard_ceiling_bytes == expected_hard_ceiling_bytes
        && !guard.field.is_empty()
        && guard.requested_bytes > 0
        && guard
            .live_requested_bytes
            .checked_add(guard.requested_bytes)
            .is_none_or(|total| total > guard.hard_ceiling_bytes)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn median_and_mad(values: impl IntoIterator<Item = u64>) -> Result<(u64, u64), PilotError> {
    let mut ordered = values.into_iter().collect::<Vec<_>>();
    if ordered.len() != FRESH_PROCESS_PILOT_SAMPLE_COUNT {
        return Err(PilotError::WrongSampleCount {
            actual: ordered.len(),
        });
    }
    ordered.sort_unstable();
    let median = ordered[FRESH_PROCESS_PILOT_SAMPLE_COUNT / 2];
    let mut deviations = ordered
        .iter()
        .map(|value| value.abs_diff(median))
        .collect::<Vec<_>>();
    deviations.sort_unstable();
    Ok((median, deviations[FRESH_PROCESS_PILOT_SAMPLE_COUNT / 2]))
}

fn relative_mad_within_limit(median: u64, mad: u64) -> bool {
    median > 0
        && u128::from(mad) * 100 <= u128::from(median) * u128::from(MAXIMUM_RELATIVE_MAD_PERCENT)
}

#[derive(Debug, thiserror::Error)]
pub enum PilotError {
    #[error(transparent)]
    Timing(#[from] TimingError),
    #[error(transparent)]
    Guard(#[from] GuardError),
    #[error(transparent)]
    ProcessProtocol(#[from] ProcessProtocolError),
    #[error(transparent)]
    WorkloadContract(#[from] crate::ScalableWorkloadContractError),
    #[error("新进程试运行标识符不能为空")]
    EmptyPilotId,
    #[error("新进程试运行容量预留失败：{field}")]
    AllocationFailed {
        field: &'static str,
        #[source]
        source: std::collections::TryReserveError,
    },
    #[error("无法启动新进程试运行样本 {ordinal}")]
    ChildSpawn {
        ordinal: usize,
        #[source]
        source: std::io::Error,
    },
    #[error("冷实例子进程读取父进程启动信号失败")]
    ChildStartSignalRead(#[source] std::io::Error),
    #[error("冷实例子进程收到无效启动信号：0x{actual:02x}")]
    InvalidChildStartSignal { actual: u8 },
    #[error("等待新进程试运行样本 {ordinal} 失败")]
    ChildWait {
        ordinal: usize,
        #[source]
        source: std::io::Error,
    },
    #[error("终止新进程试运行样本 {ordinal}（PID {child_pid}）后无法轮询退出状态")]
    ChildTerminationStatusPoll {
        ordinal: usize,
        child_pid: u32,
        #[source]
        source: std::io::Error,
    },
    #[error("无法升级新进程试运行样本 {ordinal}（PID {child_pid}）的操作系统级存续边界")]
    ChildContainmentEscalation {
        ordinal: usize,
        child_pid: u32,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "新进程试运行样本 {ordinal}（PID {child_pid}）在普通终止与操作系统级存续边界升级合计 {total_timeout_ms} ms 的截止时间内仍未退出；最后终止错误：{last_termination_error:?}"
    )]
    ChildTerminationDeadlineExceeded {
        ordinal: usize,
        child_pid: u32,
        total_timeout_ms: u64,
        last_termination_error: Option<String>,
    },
    #[error("无法解析新进程试运行样本 {ordinal}")]
    InvalidChildReport {
        ordinal: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("新进程试运行样本 {ordinal} 由父进程监控判为无效，但缺少具名监控触发条件")]
    MissingMonitorInvalidationTrigger { ordinal: usize },
    #[error("新进程试运行样本 {ordinal} 字段不匹配：{field}")]
    ChildReportMismatch { ordinal: usize, field: &'static str },
    #[error("新进程试运行重复使用编译器实例身份")]
    DuplicateCompilerInstanceId,
    #[error("新进程试运行需要恰好七个样本，实际为 {actual}")]
    WrongSampleCount { actual: usize },
    #[error("基础规模试运行贡献索引不存在：{index}")]
    InvalidContributingRunIndex { index: usize },
    #[error("基础规模试运行贡献集合包含无效、非冷实例、跨规模或缺少子报告的运行")]
    InvalidContributingRunSet,
    #[error("基础规模试运行缺少同级有效独立预言机运行，或其完整计数/完整有类型输出不相等")]
    InvalidOracleRunSet,
    #[error("基础规模独立预言机报告字段不匹配：{field}")]
    OracleReportMismatch { field: &'static str },
    #[error("同一完整基础规模级别的七个受控分配安全峰值不一致")]
    InconsistentGuardPeakObservation,
    #[error(
        "基础规模独立预言机 {run_id} 未能验证 {workload_id:?}/{graph_profile:?}/N={n}；已持久化运行并禁止该级别形成候选 B"
    )]
    OracleVerificationFailed {
        run_id: String,
        workload_id: ScalableWorkloadId,
        graph_profile: GraphProfileId,
        n: u32,
    },
    #[error("无法持久化基础规模试运行检查点：{detail}")]
    CheckpointPersistence { detail: String },
    #[error("新进程试运行算术溢出：{0}")]
    ArithmeticOverflow(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io::{Read, Write};
    use std::path::Path;
    use std::process::{Command, Stdio};

    const MONITOR_TERMINATION_HELPER_ENV: &str = "LANEFLOW_ISSUE_308_MONITOR_TERMINATION_HELPER";

    #[test]
    fn exact_median_and_mad_use_all_seven_integer_samples() {
        let (median, mad) =
            median_and_mad([109, 100, 103, 101, 102, 104, 150]).expect("seven samples");
        assert_eq!(median, 103);
        assert_eq!(mad, 2);
        assert!(relative_mad_within_limit(10_000, 200));
        assert!(!relative_mad_within_limit(10_000, 201));
        assert!(!relative_mad_within_limit(0, 0));
    }

    #[test]
    fn median_rejects_any_non_protocol_sample_count() {
        assert!(matches!(
            median_and_mad([1, 2, 3]),
            Err(PilotError::WrongSampleCount { actual: 3 })
        ));
    }

    #[test]
    fn runtime_guard_completes_only_the_current_identity_without_fabricating_a_preflight() {
        let mut selection = BaseScaleSelection {
            candidate_id: BASELINE_CANDIDATE_ID.to_owned(),
            workload_id: ScalableWorkloadId::Identity,
            workload_revision: WORKLOAD_REVISION_V1,
            graph_profile: GraphProfileId::SharedFaninDag.as_str().to_owned(),
            string_profile: BASE_SCALE_STRING_PROFILE.to_owned(),
            generator_version: GENERATOR_VERSION_V1,
            selection_rule: BASE_SCALE_SELECTION_RULE.to_owned(),
            pilot_levels: Vec::new(),
            b: NullableObservation::unavailable("base-scale-not-yet-selected"),
            terminal_guard_run_id: NullableObservation::unavailable("base-scale-not-yet-selected"),
        };
        complete_selection_after_runtime_guard(&mut selection);
        assert_eq!(selection.b.value, None);
        assert_eq!(
            selection.b.reason.as_deref(),
            Some("no-reliable-base-scale-runtime-guard")
        );
        assert_eq!(selection.terminal_guard_run_id.value, None);
        assert_eq!(
            selection.terminal_guard_run_id.reason.as_deref(),
            Some("runtime-guard-is-not-preflight")
        );
    }

    #[test]
    fn base_scale_level_uses_only_seven_valid_cold_runs() {
        let guard = clear_pilot_guard();
        let wall_times = [9_800, 9_900, 9_950, 10_000, 10_050, 10_100, 10_200];
        let mut runs = wall_times
            .into_iter()
            .enumerate()
            .map(|(ordinal, wall_time_ns)| {
                synthetic_pilot_run(ordinal, wall_time_ns, &guard, RunStatus::Valid)
            })
            .collect::<Vec<_>>();
        let contributing = (0..FRESH_PROCESS_PILOT_SAMPLE_COUNT).collect::<Vec<_>>();
        let oracle = synthetic_oracle_run(&guard, RunStatus::Valid);

        let level = summarize_base_scale_level(1, &contributing, &runs, &oracle, 10_000)
            .expect("pilot level");
        assert_eq!(level.wall_time_median_ns, 10_000);
        assert_eq!(level.wall_time_median_absolute_deviation_ns, 100);
        assert!(level.all_semantic_digests_equal);
        assert!(level.all_guards_clear);
        assert!(level.qualifies);

        runs[0].status = RunStatus::Invalid;
        runs[0]
            .invalidation_reasons
            .push(InvalidationReason::MonitoringGap);
        assert!(matches!(
            summarize_base_scale_level(1, &contributing, &runs, &oracle, 10_000),
            Err(PilotError::InvalidContributingRunSet)
        ));
    }

    #[test]
    fn seven_identical_wrong_digests_cannot_qualify_without_the_independent_oracle() {
        let guard = clear_pilot_guard();
        let mut runs = (0..FRESH_PROCESS_PILOT_SAMPLE_COUNT)
            .map(|ordinal| synthetic_pilot_run(ordinal, 10_000, &guard, RunStatus::Valid))
            .collect::<Vec<_>>();
        for run in &mut runs {
            run.child
                .as_mut()
                .expect("synthetic child")
                .semantic_digest_sha256 = Some("1".repeat(64));
        }
        let contributing = (0..FRESH_PROCESS_PILOT_SAMPLE_COUNT).collect::<Vec<_>>();
        let oracle = synthetic_oracle_run(&guard, RunStatus::Valid);

        assert!(matches!(
            summarize_base_scale_level(1, &contributing, &runs, &oracle, 10_000),
            Err(PilotError::InvalidOracleRunSet)
        ));
    }

    #[test]
    fn completed_level_requires_one_deterministic_guard_peak_across_all_seven_samples() {
        let guard = clear_pilot_guard();
        let mut runs = (0..FRESH_PROCESS_PILOT_SAMPLE_COUNT)
            .map(|ordinal| synthetic_pilot_run(ordinal, 10_000, &guard, RunStatus::Valid))
            .collect::<Vec<_>>();
        runs[3]
            .child
            .as_mut()
            .expect("synthetic child")
            .guard_peak_live_requested_bytes = Some(2);
        let contributing = (0..FRESH_PROCESS_PILOT_SAMPLE_COUNT).collect::<Vec<_>>();
        let oracle = synthetic_oracle_run(&guard, RunStatus::Valid);

        assert!(matches!(
            summarize_base_scale_level(1, &contributing, &runs, &oracle, 10_000),
            Err(PilotError::InconsistentGuardPeakObservation)
        ));
    }

    #[test]
    fn timing_success_report_cannot_claim_a_peak_above_its_hard_ceiling() {
        let guard = clear_pilot_guard();
        let run = synthetic_pilot_run(0, 10_000, &guard, RunStatus::Valid);
        let mut report = run.child.expect("synthetic child");
        report.controlled_allocation_hard_ceiling_bytes = 1;
        report.guard_peak_live_requested_bytes = Some(2);
        let expected = ScalableChildExpectation {
            compiler_instance_id: &report.compiler_instance_id,
            child_pid: report.child_pid,
            workload_id: report.workload_id,
            graph_profile: GraphProfileId::WideStar,
            n: 1,
            controlled_allocation_hard_ceiling_bytes: 1,
        };
        assert!(matches!(
            verify_scalable_child_report(&report, 0, &expected),
            Err(PilotError::ChildReportMismatch {
                field: "guardPeakLiveRequestedBytes",
                ..
            })
        ));
    }

    #[test]
    fn oracle_report_primary_count_is_bound_to_the_parent_preflight() {
        let guard = clear_pilot_guard();
        let run = synthetic_oracle_run(&guard, RunStatus::Valid);
        let mut report = run.child.expect("synthetic oracle child");
        report.primary_record_count = Some(guard.primary_record_count + 1);
        let expected = ScalableOracleExpectation {
            run_id: &report.oracle_run_id,
            child_pid: report.child_pid,
            workload_id: report.workload_id,
            graph_profile: GraphProfileId::WideStar,
            n: 1,
            primary_record_count: guard.primary_record_count,
            controlled_allocation_hard_ceiling_bytes: guard.thresholds.compiler_controlled_bytes,
        };
        assert!(matches!(
            verify_scalable_oracle_report(&report, &expected),
            Err(PilotError::OracleReportMismatch {
                field: "primaryRecordCount"
            })
        ));
    }

    #[test]
    fn invalid_attempts_are_retained_but_excluded_from_retry_summary() {
        let guard = clear_pilot_guard();
        let mut runs = vec![synthetic_pilot_run(0, 10_000, &guard, RunStatus::Invalid)];
        runs[0]
            .invalidation_reasons
            .push(InvalidationReason::ChildAbnormalExit);
        for ordinal in 0..FRESH_PROCESS_PILOT_SAMPLE_COUNT {
            let mut run = synthetic_pilot_run(ordinal, 10_000, &guard, RunStatus::Valid);
            run.retry_ordinal = 1;
            run.attempt_id = "retry-attempt".to_owned();
            run.run_id = format!("retry-attempt/pilot-sample-{ordinal}");
            runs.push(run);
        }
        let contributing = (1..=FRESH_PROCESS_PILOT_SAMPLE_COUNT).collect::<Vec<_>>();
        let oracle = synthetic_oracle_run(&guard, RunStatus::Valid);
        let level = summarize_base_scale_level(1, &contributing, &runs, &oracle, 10_000)
            .expect("retry summary");

        assert_eq!(runs.len(), FRESH_PROCESS_PILOT_SAMPLE_COUNT + 1);
        assert_eq!(
            level.contributing_run_ids.len(),
            FRESH_PROCESS_PILOT_SAMPLE_COUNT
        );
        assert!(
            level
                .contributing_run_ids
                .iter()
                .all(|run_id| run_id.starts_with("retry-attempt/"))
        );
    }

    #[test]
    fn invalidating_an_attempt_propagates_the_same_reason_to_prior_valid_runs() {
        let guard = clear_pilot_guard();
        let mut runs = (0..3)
            .map(|ordinal| synthetic_pilot_run(ordinal, 10_000, &guard, RunStatus::Valid))
            .collect::<Vec<_>>();
        invalidate_attempt_runs(&mut runs, &[InvalidationReason::MonitoringGap]);

        assert!(runs.iter().all(|run| {
            run.status == RunStatus::Invalid
                && run.invalidation_reasons == vec![InvalidationReason::MonitoringGap]
        }));
    }

    #[test]
    fn base_scale_candidates_use_strict_checked_doubling() {
        let mut n = 1;
        for expected in [2, 4, 8, 16] {
            n = next_base_scale_n(n).expect("next base-scale candidate");
            assert_eq!(n, expected);
        }
        assert!(matches!(
            next_base_scale_n(1_u32 << 31),
            Err(PilotError::ArithmeticOverflow("base-scale strict doubling"))
        ));
    }

    #[test]
    fn rejected_preflight_never_attempts_to_spawn_the_child() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let outcome = run_identity_fresh_process_pilot_with_memory_observer(
            &trusted,
            IdentityFreshProcessPilotRequest {
                timing_executable: Path::new("this-child-must-never-be-spawned"),
                pilot_id: "guard-rejection",
                graph_profile: GraphProfileId::WideStar,
                n: 1,
                previous: None,
            },
            || {
                Ok(SystemMemoryObservation {
                    physical_memory_bytes: 64 * 1_073_741_824,
                    available_physical_memory_bytes: 1,
                })
            },
        )
        .expect("guard rejection is a structured pilot outcome");
        let IdentityFreshProcessPilotOutcome::Stopped { stop } = outcome else {
            panic!("guard rejection must stop the pilot");
        };
        assert_eq!(stop.sample_ordinal, 0);
        assert_eq!(stop.status, RunStatus::Guarded);
        assert!(stop.invalidation_reasons.is_empty());
        assert_eq!(
            stop.process.exit_kind,
            crate::ProcessExitKind::GuardedBeforeStart
        );
        assert_eq!(stop.process.child_pid.value, None);
        assert_eq!(
            stop.process.child_pid.reason.as_deref(),
            Some("child-not-started")
        );
        assert_eq!(stop.process.exit_code.value, None);
        assert_eq!(
            stop.process.termination.kind,
            crate::TerminationKind::NotStarted
        );
        assert!(stop.child.is_none());
        assert!(stop.monitor.is_none());
    }

    #[test]
    fn parent_monitor_thresholds_trigger_at_exact_limits() {
        let thresholds =
            GuardThresholds::from_physical_memory_bytes(64 * 1_073_741_824).expect("thresholds");
        let safe = ChildProcessMemoryObservation {
            private_bytes: thresholds.private_bytes - 1,
            available_physical_memory_bytes: thresholds.minimum_available_physical_memory_bytes,
        };
        assert_eq!(
            evaluate_child_monitor_trigger(thresholds, safe, Duration::from_secs(59))
                .expect("safe monitor"),
            None
        );
        assert_eq!(
            evaluate_child_monitor_trigger(
                thresholds,
                ChildProcessMemoryObservation {
                    private_bytes: thresholds.private_bytes,
                    ..safe
                },
                Duration::ZERO,
            )
            .expect("private trigger"),
            Some(ChildMonitorTrigger::PrivateBytes)
        );
        assert_eq!(
            evaluate_child_monitor_trigger(thresholds, safe, Duration::from_secs(60))
                .expect("wall trigger"),
            Some(ChildMonitorTrigger::WallTime)
        );
        assert!(
            !wall_time_limit_reached(
                thresholds,
                Duration::from_nanos(thresholds.wall_time_ns - 1)
            )
            .expect("below wall-time limit")
        );
        assert!(
            wall_time_limit_reached(thresholds, Duration::from_nanos(thresholds.wall_time_ns))
                .expect("exact wall-time limit")
        );
        assert_eq!(
            evaluate_child_monitor_trigger(
                thresholds,
                ChildProcessMemoryObservation {
                    available_physical_memory_bytes: thresholds
                        .minimum_available_physical_memory_bytes
                        - 1,
                    ..safe
                },
                Duration::ZERO,
            )
            .expect("available-memory trigger"),
            Some(ChildMonitorTrigger::AvailablePhysicalMemory)
        );
    }

    #[test]
    fn clean_exit_after_a_resource_trigger_never_claims_a_monitoring_gap() {
        for trigger in [
            ChildMonitorTrigger::PrivateBytes,
            ChildMonitorTrigger::WallTime,
            ChildMonitorTrigger::AvailablePhysicalMemory,
        ] {
            assert_eq!(
                monitor_invalidation_reasons(Some(trigger), true),
                Some(vec![InvalidationReason::ResearchStopGuardrailTriggered])
            );
        }
        assert_eq!(
            monitor_invalidation_reasons(Some(ChildMonitorTrigger::MonitoringGap), true),
            Some(vec![InvalidationReason::MonitoringGap])
        );
        assert_eq!(
            monitor_invalidation_reasons(Some(ChildMonitorTrigger::PrivateBytes), false),
            Some(vec![
                InvalidationReason::ResearchStopGuardrailTriggered,
                InvalidationReason::ChildAbnormalExit
            ])
        );
        assert_eq!(monitor_invalidation_reasons(None, true), None);
    }

    #[derive(Debug)]
    struct ScriptedTerminationControl {
        termination_failures: usize,
        exit_observations: VecDeque<bool>,
        exit_after_containment: bool,
        termination_attempts: usize,
        status_polls: usize,
        containment_escalations: usize,
    }

    impl ScriptedTerminationControl {
        fn new(termination_failures: usize, exit_observations: impl Into<VecDeque<bool>>) -> Self {
            Self {
                termination_failures,
                exit_observations: exit_observations.into(),
                exit_after_containment: false,
                termination_attempts: 0,
                status_polls: 0,
                containment_escalations: 0,
            }
        }

        fn with_exit_after_containment(mut self) -> Self {
            self.exit_after_containment = true;
            self
        }
    }

    impl ChildTerminationControl for ScriptedTerminationControl {
        fn request_termination(&mut self) -> std::io::Result<()> {
            self.termination_attempts += 1;
            if self.termination_attempts <= self.termination_failures {
                return Err(std::io::Error::other("scripted-termination-refusal"));
            }
            Ok(())
        }

        fn has_exited(&mut self) -> std::io::Result<bool> {
            self.status_polls += 1;
            Ok(self
                .exit_observations
                .pop_front()
                .unwrap_or(self.exit_after_containment && self.containment_escalations > 0))
        }

        fn escalate_containment(&mut self) -> std::io::Result<()> {
            self.containment_escalations += 1;
            Ok(())
        }
    }

    #[test]
    fn failed_termination_never_enters_an_unbounded_wait() {
        let mut child = ScriptedTerminationControl::new(usize::MAX, [false]);
        let failure = terminate_child_with_deadline(&mut child, Duration::ZERO, Duration::ZERO)
            .expect_err("a permanently running child must hit the bounded deadline");
        let ChildTerminationFailure::DeadlineExceeded {
            last_termination_error,
        } = failure
        else {
            panic!("persistent termination refusal must reach the bounded deadline");
        };
        assert_eq!(
            last_termination_error.as_deref(),
            Some("scripted-termination-refusal")
        );
        assert_eq!(child.termination_attempts, 1);
        assert_eq!(child.status_polls, 2);
        assert_eq!(child.containment_escalations, 1);
    }

    #[test]
    fn persistent_termination_refusal_escalates_containment_before_returning() {
        let mut child =
            ScriptedTerminationControl::new(usize::MAX, [false]).with_exit_after_containment();
        let completion = terminate_child_with_deadline(&mut child, Duration::ZERO, Duration::ZERO)
            .expect("containment escalation must make the child exit observable");
        assert_eq!(
            completion.prior_termination_error.as_deref(),
            Some("scripted-termination-refusal")
        );
        assert!(completion.containment_escalated);
        assert_eq!(child.termination_attempts, 1);
        assert_eq!(child.status_polls, 2);
        assert_eq!(child.containment_escalations, 1);
    }

    #[test]
    fn termination_is_retried_until_exit_is_confirmed() {
        let mut child = ScriptedTerminationControl::new(2, [false, false, false, true]);
        let completion =
            terminate_child_with_deadline(&mut child, Duration::from_secs(1), Duration::ZERO)
                .expect("a later successful termination must be observed");
        assert_eq!(
            completion.prior_termination_error.as_deref(),
            Some("scripted-termination-refusal")
        );
        assert_eq!(child.termination_attempts, 3);
        assert_eq!(child.status_polls, 4);
        assert_eq!(child.containment_escalations, 0);
        assert!(!completion.containment_escalated);
    }

    #[test]
    fn a_naturally_observed_exit_preserves_prior_termination_failure() {
        let mut child = ScriptedTerminationControl::new(usize::MAX, [true]);
        let completion =
            terminate_child_with_deadline(&mut child, Duration::from_secs(1), Duration::ZERO)
                .expect("an observed exit is bounded even when termination was refused");
        assert_eq!(
            completion.prior_termination_error.as_deref(),
            Some("scripted-termination-refusal")
        );
        assert_eq!(child.termination_attempts, 1);
        assert_eq!(child.status_polls, 1);
        assert_eq!(child.containment_escalations, 0);
        assert!(!completion.containment_escalated);
    }

    #[test]
    fn parent_monitor_terminates_and_reaps_a_child_at_the_private_byte_limit() {
        let child = spawn_monitor_termination_helper("wait");
        let thresholds =
            GuardThresholds::from_physical_memory_bytes(64 * 1_073_741_824).expect("thresholds");
        let observation = ChildProcessMemoryObservation {
            private_bytes: thresholds.private_bytes,
            available_physical_memory_bytes: thresholds.minimum_available_physical_memory_bytes,
        };

        let execution =
            run_monitored_child_with_observer(child, 0, thresholds, |_| Ok(Some(observation)))
                .expect("private-byte threshold must form a structured invalidation");
        let MonitoredChildExecution::InvalidatedByMonitor {
            child_pid,
            output,
            monitor,
            kill_error,
            monitor_error,
        } = execution
        else {
            panic!("private-byte threshold must invalidate through the monitor");
        };
        assert_eq!(monitor.trigger, Some(ChildMonitorTrigger::PrivateBytes));
        assert_eq!(monitor.observation_count, 1);
        assert_eq!(
            monitor.last_private_bytes.value,
            Some(thresholds.private_bytes)
        );
        assert_eq!(
            monitor.peak_private_bytes.value,
            Some(thresholds.private_bytes)
        );
        assert_eq!(kill_error, None);
        assert_eq!(monitor_error, None);
        let process = ProcessObservation::invalid_monitor_termination(
            std::process::id(),
            child_pid,
            TIMING_BINARY_ID,
            output.status,
        )
        .expect("monitor kill must have an abnormal platform status");
        assert_eq!(
            process.exit_kind,
            crate::ProcessExitKind::InvalidMonitorTermination
        );
    }

    #[test]
    fn monitor_provider_failure_is_a_structured_monitoring_gap() {
        let child = spawn_monitor_termination_helper("wait");
        let thresholds =
            GuardThresholds::from_physical_memory_bytes(64 * 1_073_741_824).expect("thresholds");
        let execution = run_monitored_child_with_observer(child, 0, thresholds, |_| {
            Err(GuardError::UnsupportedPrivateMemoryProvider)
        })
        .expect("monitor provider failure must form a structured invalidation");
        let MonitoredChildExecution::InvalidatedByMonitor {
            child_pid,
            output,
            monitor,
            monitor_error,
            ..
        } = execution
        else {
            panic!("monitor provider failure must invalidate through the monitor");
        };
        assert_eq!(monitor.trigger, Some(ChildMonitorTrigger::MonitoringGap));
        assert_eq!(monitor.last_private_bytes.value, None);
        assert_eq!(
            monitor.last_private_bytes.reason.as_deref(),
            Some("monitoring-gap")
        );
        assert!(monitor_error.is_some());
        let process = ProcessObservation::invalid_monitor_termination(
            std::process::id(),
            child_pid,
            TIMING_BINARY_ID,
            output.status,
        )
        .expect("monitor kill must have an abnormal platform status");
        assert_eq!(
            process.exit_kind,
            crate::ProcessExitKind::InvalidMonitorTermination
        );
    }

    #[test]
    fn nonzero_child_exit_is_distinct_from_monitor_termination() {
        let child = spawn_monitor_termination_helper("exit-7");
        let thresholds =
            GuardThresholds::from_physical_memory_bytes(64 * 1_073_741_824).expect("thresholds");
        let safe_observation = ChildProcessMemoryObservation {
            private_bytes: thresholds.private_bytes - 1,
            available_physical_memory_bytes: thresholds.minimum_available_physical_memory_bytes,
        };
        let execution =
            run_monitored_child_with_observer(child, 0, thresholds, |_| Ok(Some(safe_observation)))
                .expect("nonzero exit must remain observable");
        let MonitoredChildExecution::Exited {
            child_pid,
            output,
            monitor,
        } = execution
        else {
            panic!("nonzero child exit must not be classified as a monitor termination");
        };
        assert_eq!(monitor.trigger, None);
        let process = ProcessObservation::invalid_abnormal_exit(
            std::process::id(),
            child_pid,
            TIMING_BINARY_ID,
            output.status,
        )
        .expect("nonzero exit code must form an abnormal-exit observation");
        assert_eq!(
            process.exit_kind,
            crate::ProcessExitKind::InvalidAbnormalExit
        );
        assert_eq!(process.exit_code.value, Some(7));
    }

    #[cfg(windows)]
    #[test]
    fn windows_job_close_terminates_and_reaps_the_contained_child() {
        let mut child = spawn_monitor_termination_helper("wait");
        let child_pid = child.id();
        let mut stdin = child.take_stdin().expect("contained child stdin");
        stdin
            .write_all(&[CHILD_START_SIGNAL])
            .expect("release contained helper");
        drop(stdin);

        assert!(
            child
                .escalate_containment()
                .expect("close kill-on-close Job Object")
        );
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().expect("poll contained helper") {
                break status;
            }
            assert!(
                started.elapsed() < CHILD_TERMINATION_TIMEOUT,
                "contained child {child_pid} outlived the Job close deadline"
            );
            thread::sleep(CHILD_TERMINATION_RETRY_INTERVAL);
        };
        let output = child
            .wait_with_output()
            .expect("reap Job-terminated contained helper");
        assert_eq!(output.status.code(), status.code());
    }

    fn spawn_monitor_termination_helper(mode: &str) -> ContainedChild {
        let executable = std::env::current_exe().expect("current test executable");
        let mut command = Command::new(executable);
        command
            .arg("pilot::tests::parent_monitor_termination_helper")
            .arg("--exact")
            .arg("--nocapture")
            .env(MONITOR_TERMINATION_HELPER_ENV, mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        ContainedChild::spawn(&mut command).expect("spawn contained monitor termination helper")
    }

    #[test]
    fn parent_monitor_termination_helper() {
        let Some(mode) = std::env::var_os(MONITOR_TERMINATION_HELPER_ENV) else {
            return;
        };
        let mut signal = [0_u8; 1];
        std::io::stdin()
            .read_exact(&mut signal)
            .expect("helper waits for a parent start signal or process termination");
        if mode == "exit-7" {
            std::process::exit(7);
        }
        thread::sleep(Duration::from_secs(60));
    }

    fn clear_pilot_guard() -> GuardPreflightReport {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        ScalableGuardPlanner::from_trusted_contract(&trusted)
            .expect("guard planner")
            .evaluate_pilot(
                ScalableWorkloadId::Identity,
                GraphProfileId::WideStar,
                1,
                SystemMemoryObservation {
                    physical_memory_bytes: 64 * 1_073_741_824,
                    available_physical_memory_bytes: 48 * 1_073_741_824,
                },
                None,
            )
            .expect("clear pilot guard")
    }

    fn synthetic_pilot_run(
        ordinal: usize,
        wall_time_ns: u64,
        guard_preflight: &GuardPreflightReport,
        status: RunStatus,
    ) -> BaseScalePilotRun {
        let run_id = format!("attempt/pilot-sample-{ordinal}");
        BaseScalePilotRun {
            run_id: run_id.clone(),
            attempt_id: "attempt".to_owned(),
            retry_ordinal: 0,
            pilot_sample_position: u32::try_from(ordinal).expect("test ordinal"),
            run_kind: BaseScalePilotRunKind::ColdInstance,
            workload_id: ScalableWorkloadId::Identity,
            workload_revision: WORKLOAD_REVISION_V1,
            graph_profile: GraphProfileId::WideStar.as_str().to_owned(),
            string_profile: BASE_SCALE_STRING_PROFILE.to_owned(),
            generator_version: GENERATOR_VERSION_V1,
            n: 1,
            status,
            invalidation_reasons: Vec::new(),
            process: ProcessObservation::guarded_before_start(std::process::id(), TIMING_BINARY_ID),
            guard_preflight: guard_preflight.clone(),
            child: Some(ScalableTimingChildReport {
                schema: SCALABLE_TIMING_CHILD_SCHEMA.to_owned(),
                schema_version: SCALABLE_TIMING_CHILD_SCHEMA_VERSION,
                binary_id: TIMING_BINARY_ID.to_owned(),
                allocation_instrumentation_enabled: false,
                compiler_instance_id: format!("{run_id}/compiler-instance"),
                child_pid: 1,
                workload_id: ScalableWorkloadId::Identity,
                workload_revision: WORKLOAD_REVISION_V1,
                graph_profile: GraphProfileId::WideStar.as_str().to_owned(),
                string_profile: BASE_SCALE_STRING_PROFILE.to_owned(),
                generator_version: GENERATOR_VERSION_V1,
                n: 1,
                outcome: ScalableTimingOutcome::Success,
                controlled_allocation_hard_ceiling_bytes: guard_preflight
                    .thresholds
                    .compiler_controlled_bytes,
                guard_peak_live_requested_bytes: Some(1),
                wall_time_ns: Some(wall_time_ns),
                semantic_digest_sha256: Some("0".repeat(64)),
                controlled_allocation_guard: None,
            }),
            monitor: Some(ChildProcessMonitorReport {
                observation_count: 1,
                last_private_bytes: NullableObservation::observed(1),
                peak_private_bytes: NullableObservation::observed(1),
                elapsed_wall_time_ns: wall_time_ns,
                trigger: None,
            }),
            kill_error: None,
            monitor_error: None,
            stderr: String::new(),
        }
    }

    fn synthetic_oracle_run(
        guard_preflight: &GuardPreflightReport,
        status: RunStatus,
    ) -> BaseScaleOracleRun {
        BaseScaleOracleRun {
            run_id: "attempt/oracle".to_owned(),
            workload_id: ScalableWorkloadId::Identity,
            workload_revision: WORKLOAD_REVISION_V1,
            graph_profile: GraphProfileId::WideStar.as_str().to_owned(),
            string_profile: BASE_SCALE_STRING_PROFILE.to_owned(),
            generator_version: GENERATOR_VERSION_V1,
            n: 1,
            status,
            invalidation_reasons: Vec::new(),
            process: ProcessObservation::guarded_before_start(std::process::id(), ORACLE_BINARY_ID),
            guard_preflight: guard_preflight.clone(),
            child: Some(ScalableOracleChildReport {
                schema: SCALABLE_ORACLE_CHILD_SCHEMA.to_owned(),
                schema_version: SCALABLE_ORACLE_CHILD_SCHEMA_VERSION,
                binary_id: ORACLE_BINARY_ID.to_owned(),
                oracle_run_id: "attempt/oracle".to_owned(),
                child_pid: 1,
                workload_id: ScalableWorkloadId::Identity,
                workload_revision: WORKLOAD_REVISION_V1,
                graph_profile: GraphProfileId::WideStar.as_str().to_owned(),
                string_profile: BASE_SCALE_STRING_PROFILE.to_owned(),
                generator_version: GENERATOR_VERSION_V1,
                n: 1,
                outcome: ScalableOracleOutcome::Success,
                controlled_allocation_hard_ceiling_bytes: guard_preflight
                    .thresholds
                    .compiler_controlled_bytes,
                guard_peak_live_requested_bytes: Some(1),
                primary_record_count: Some(1),
                semantic_digest_sha256: Some("0".repeat(64)),
                complete_counts_equal: true,
                complete_typed_output_equal: true,
                controlled_allocation_guard: None,
            }),
            monitor: Some(ChildProcessMonitorReport {
                observation_count: 1,
                last_private_bytes: NullableObservation::observed(1),
                peak_private_bytes: NullableObservation::observed(1),
                elapsed_wall_time_ns: 1,
                trigger: None,
            }),
            kill_error: None,
            monitor_error: None,
            stderr: String::new(),
        }
    }
}
