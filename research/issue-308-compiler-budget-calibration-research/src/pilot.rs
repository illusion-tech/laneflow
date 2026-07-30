//! 编译器测量基准规模发现所需的新进程冷实例试运行基础。
//!
//! 本模块执行启动前停止护栏、七个受监控的独立子进程、冷实例结果核对和墙钟中位数/
//! 中位绝对偏差计算。基准规模选择、完整跨平台终止编码、Evidence v1 写出与正式轮次
//! 仍由后续切片负责。

use crate::{
    ChildProcessMemoryMonitor, ChildProcessMemoryObservation, GraphProfileId,
    GuardCompletedLevelObservation, GuardError, GuardPreflightReport, GuardThresholds,
    IdentityCompilerInstance, StageGenerationError, SystemMemoryMonitor, SystemMemoryObservation,
    TimingError, TrustedContract, evaluate_identity_guard_preflight, observe_clock_quantum_ns,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const IDENTITY_TIMING_CHILD_SCHEMA: &str =
    "laneflow.compiler-calibration-identity-timing-child";
pub const IDENTITY_TIMING_CHILD_SCHEMA_VERSION: u32 = 2;
pub const FRESH_PROCESS_PILOT_SAMPLE_COUNT: usize = 7;
pub const CLOCK_QUANTUM_MULTIPLIER: u64 = 10_000;
pub const MAXIMUM_RELATIVE_MAD_PERCENT: u64 = 2;
const CHILD_MONITOR_POLL_INTERVAL: Duration = Duration::from_millis(1);
const CHILD_MONITOR_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const CHILD_START_SIGNAL: u8 = b'G';

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdentityChildOutcome {
    Success,
    GuardedInChild,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlledAllocationGuardReport {
    pub field: String,
    pub hard_ceiling_bytes: u64,
    pub live_requested_bytes: u64,
    pub requested_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityChildTimingReport {
    pub schema: String,
    pub schema_version: u32,
    pub compiler_instance_id: String,
    pub child_pid: u32,
    pub graph_profile: String,
    pub n: u32,
    pub outcome: IdentityChildOutcome,
    pub controlled_allocation_hard_ceiling_bytes: u64,
    pub peak_live_requested_bytes: u64,
    pub wall_time_ns: Option<u64>,
    pub semantic_digest_sha256: Option<String>,
    pub controlled_allocation_guard: Option<ControlledAllocationGuardReport>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChildMonitorTrigger {
    PrivateBytes,
    WallTime,
    AvailablePhysicalMemory,
    MonitoringGap,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildProcessMonitorReport {
    pub child_pid: u32,
    pub observation_count: u64,
    pub last_private_bytes: Option<u64>,
    pub peak_private_bytes: Option<u64>,
    pub elapsed_wall_time_ns: u64,
    pub exit_code: Option<i32>,
    pub trigger: Option<ChildMonitorTrigger>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityMonitoredChildSample {
    pub child: IdentityChildTimingReport,
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

pub fn measure_identity_timing_child(
    trusted: &TrustedContract,
    compiler_instance_id: String,
    graph_profile: GraphProfileId,
    n: u32,
    controlled_allocation_hard_ceiling_bytes: u64,
) -> Result<IdentityChildTimingReport, PilotError> {
    let mut instance =
        IdentityCompilerInstance::from_trusted_contract_with_id_and_allocation_ceiling(
            trusted,
            compiler_instance_id.clone(),
            controlled_allocation_hard_ceiling_bytes,
        )?;
    let result = instance.measure(graph_profile, n);
    let peak_live_requested_bytes = instance.peak_live_requested_bytes();
    let base = || IdentityChildTimingReport {
        schema: IDENTITY_TIMING_CHILD_SCHEMA.to_owned(),
        schema_version: IDENTITY_TIMING_CHILD_SCHEMA_VERSION,
        compiler_instance_id: compiler_instance_id.clone(),
        child_pid: std::process::id(),
        graph_profile: graph_profile.as_str().to_owned(),
        n,
        outcome: IdentityChildOutcome::Success,
        controlled_allocation_hard_ceiling_bytes,
        peak_live_requested_bytes,
        wall_time_ns: None,
        semantic_digest_sha256: None,
        controlled_allocation_guard: None,
    };
    match result {
        Ok(sample) => Ok(IdentityChildTimingReport {
            wall_time_ns: Some(sample.wall_time_ns),
            semantic_digest_sha256: Some(sample.stage_summary.semantic_digest_sha256),
            ..base()
        }),
        Err(TimingError::StageGeneration(
            StageGenerationError::ControlledAllocationHardCeiling {
                field,
                hard_ceiling_bytes,
                live_requested_bytes,
                requested_bytes,
            },
        )) => Ok(IdentityChildTimingReport {
            outcome: IdentityChildOutcome::GuardedInChild,
            controlled_allocation_guard: Some(ControlledAllocationGuardReport {
                field: field.to_owned(),
                hard_ceiling_bytes,
                live_requested_bytes,
                requested_bytes,
            }),
            ..base()
        }),
        Err(error) => Err(error.into()),
    }
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
    executable: &Path,
    pilot_id: &str,
    graph_profile: GraphProfileId,
    n: u32,
    previous: Option<GuardCompletedLevelObservation>,
) -> Result<IdentityFreshProcessPilot, PilotError> {
    let mut memory_monitor = SystemMemoryMonitor::new()?;
    run_identity_fresh_process_pilot_with_memory_observer(
        trusted,
        executable,
        pilot_id,
        graph_profile,
        n,
        previous,
        || memory_monitor.observe(),
    )
}

fn run_identity_fresh_process_pilot_with_memory_observer(
    trusted: &TrustedContract,
    executable: &Path,
    pilot_id: &str,
    graph_profile: GraphProfileId,
    n: u32,
    previous: Option<GuardCompletedLevelObservation>,
    mut observe_memory: impl FnMut() -> Result<SystemMemoryObservation, GuardError>,
) -> Result<IdentityFreshProcessPilot, PilotError> {
    if pilot_id.is_empty() {
        return Err(PilotError::EmptyPilotId);
    }
    let clock_quantum_ns = observe_clock_quantum_ns()?;
    let required_median_wall_time_ns = clock_quantum_ns
        .checked_mul(CLOCK_QUANTUM_MULTIPLIER)
        .ok_or(PilotError::ArithmeticOverflow("required median wall time"))?;
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
        let guard = evaluate_identity_guard_preflight(
            trusted,
            graph_profile,
            n,
            observe_memory()?,
            previous,
        )?;
        if !guard.allows_child_start {
            return Err(PilotError::GuardRejected {
                ordinal,
                report: Box::new(guard),
            });
        }
        let thresholds = guard.thresholds;
        let controlled_allocation_hard_ceiling_bytes = thresholds.compiler_controlled_bytes;
        guard_preflights.push(guard);

        let compiler_instance_id = format!("{pilot_id}/compiler-instance-{ordinal}");
        let (output, monitor) = run_monitored_identity_child(
            executable,
            ordinal,
            &compiler_instance_id,
            graph_profile,
            n,
            thresholds,
        )?;
        if !output.status.success() {
            return Err(PilotError::ChildFailed {
                ordinal,
                exit_code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        let report = serde_json::from_slice::<IdentityChildTimingReport>(&output.stdout)
            .map_err(|source| PilotError::InvalidChildReport { ordinal, source })?;
        verify_child_report(
            &report,
            ordinal,
            &compiler_instance_id,
            monitor.child_pid,
            graph_profile,
            n,
            controlled_allocation_hard_ceiling_bytes,
        )?;
        if monitor.exit_code != output.status.code() {
            return Err(PilotError::ChildReportMismatch {
                ordinal,
                field: "monitor.exitCode",
            });
        }
        let monitored_sample = IdentityMonitoredChildSample {
            child: report,
            monitor,
        };
        if monitored_sample.child.outcome == IdentityChildOutcome::GuardedInChild {
            return Err(PilotError::ChildGuarded {
                ordinal,
                sample: Box::new(monitored_sample),
            });
        }
        samples.push(monitored_sample);
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
    let (median_wall_time_ns, median_absolute_deviation_ns) = median_and_mad(
        samples
            .iter()
            .filter_map(|sample| sample.child.wall_time_ns),
    )?;
    let clock_quantum_requirement_met = median_wall_time_ns >= required_median_wall_time_ns;
    let relative_mad_requirement_met =
        relative_mad_within_limit(median_wall_time_ns, median_absolute_deviation_ns);
    let measurement_quality_met =
        clock_quantum_requirement_met && relative_mad_requirement_met && semantic_digest_consistent;

    Ok(IdentityFreshProcessPilot {
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
    })
}

fn run_monitored_identity_child(
    executable: &Path,
    ordinal: usize,
    compiler_instance_id: &str,
    graph_profile: GraphProfileId,
    n: u32,
    thresholds: GuardThresholds,
) -> Result<(std::process::Output, ChildProcessMonitorReport), PilotError> {
    let mut memory_monitor = ChildProcessMemoryMonitor::new()?;
    let child = Command::new(executable)
        .arg("identity-timing-child")
        .arg(compiler_instance_id)
        .arg(graph_profile.as_str())
        .arg(n.to_string())
        .arg(thresholds.compiler_controlled_bytes.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| PilotError::ChildSpawn { ordinal, source })?;
    run_monitored_child_with_observer(child, ordinal, thresholds, |child_pid| {
        memory_monitor.observe(child_pid)
    })
}

fn run_monitored_child_with_observer(
    mut child: std::process::Child,
    ordinal: usize,
    thresholds: GuardThresholds,
    mut observe_child: impl FnMut(u32) -> Result<Option<ChildProcessMemoryObservation>, GuardError>,
) -> Result<(std::process::Output, ChildProcessMonitorReport), PilotError> {
    let child_pid = child.id();

    let handshake_started = Instant::now();
    let initial_observation = loop {
        match observe_child(child_pid) {
            Ok(Some(observation)) => break observation,
            Ok(None) => {}
            Err(error) => {
                terminate_child_best_effort(&mut child);
                return Err(error.into());
            }
        }
        if let Some(status) = try_wait_child(&mut child, ordinal)? {
            let output = child
                .wait_with_output()
                .map_err(|source| PilotError::ChildWait { ordinal, source })?;
            return Err(PilotError::ChildExitedBeforeStart {
                ordinal,
                exit_code: status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        if handshake_started.elapsed() >= CHILD_MONITOR_HANDSHAKE_TIMEOUT {
            let report = ChildProcessMonitorReport {
                child_pid,
                observation_count: 0,
                last_private_bytes: None,
                peak_private_bytes: None,
                elapsed_wall_time_ns: duration_ns(handshake_started.elapsed())?,
                exit_code: None,
                trigger: Some(ChildMonitorTrigger::MonitoringGap),
            };
            return terminate_monitored_child(child, ordinal, report);
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
            child_pid,
            observation_count,
            last_private_bytes: Some(last_private_bytes),
            peak_private_bytes: Some(peak_private_bytes),
            elapsed_wall_time_ns: 0,
            exit_code: None,
            trigger: Some(trigger),
        };
        return terminate_monitored_child(child, ordinal, report);
    }

    let started = Instant::now();
    let Some(mut child_stdin) = child.stdin.take() else {
        terminate_child_best_effort(&mut child);
        return Err(PilotError::MissingChildStdin { ordinal });
    };
    if let Err(source) = child_stdin.write_all(&[CHILD_START_SIGNAL]) {
        terminate_child_best_effort(&mut child);
        return Err(PilotError::ChildStartSignalWrite { ordinal, source });
    }
    drop(child_stdin);

    loop {
        if let Some(status) = try_wait_child(&mut child, ordinal)? {
            let elapsed_wall_time_ns = duration_ns(started.elapsed())?;
            let output = child
                .wait_with_output()
                .map_err(|source| PilotError::ChildWait { ordinal, source })?;
            return Ok((
                output,
                ChildProcessMonitorReport {
                    child_pid,
                    observation_count,
                    last_private_bytes: Some(last_private_bytes),
                    peak_private_bytes: Some(peak_private_bytes),
                    elapsed_wall_time_ns,
                    exit_code: status.code(),
                    trigger: None,
                },
            ));
        }

        let elapsed = started.elapsed();
        let observation = match observe_child(child_pid) {
            Ok(Some(observation)) => observation,
            Ok(None) => {
                if try_wait_child(&mut child, ordinal)?.is_some() {
                    continue;
                }
                let report = ChildProcessMonitorReport {
                    child_pid,
                    observation_count,
                    last_private_bytes: Some(last_private_bytes),
                    peak_private_bytes: Some(peak_private_bytes),
                    elapsed_wall_time_ns: duration_ns(elapsed)?,
                    exit_code: None,
                    trigger: Some(ChildMonitorTrigger::MonitoringGap),
                };
                return terminate_monitored_child(child, ordinal, report);
            }
            Err(error) => {
                terminate_child_best_effort(&mut child);
                return Err(error.into());
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
        if let Some(trigger) = evaluate_child_monitor_trigger(thresholds, observation, elapsed)? {
            let report = ChildProcessMonitorReport {
                child_pid,
                observation_count,
                last_private_bytes: Some(last_private_bytes),
                peak_private_bytes: Some(peak_private_bytes),
                elapsed_wall_time_ns: duration_ns(elapsed)?,
                exit_code: None,
                trigger: Some(trigger),
            };
            return terminate_monitored_child(child, ordinal, report);
        }
        thread::sleep(CHILD_MONITOR_POLL_INTERVAL);
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
    if duration_ns(elapsed)? >= thresholds.wall_time_ns {
        return Ok(Some(ChildMonitorTrigger::WallTime));
    }
    if observation.available_physical_memory_bytes
        < thresholds.minimum_available_physical_memory_bytes
    {
        return Ok(Some(ChildMonitorTrigger::AvailablePhysicalMemory));
    }
    Ok(None)
}

fn terminate_monitored_child(
    mut child: std::process::Child,
    ordinal: usize,
    mut report: ChildProcessMonitorReport,
) -> Result<(std::process::Output, ChildProcessMonitorReport), PilotError> {
    let kill_error = child.kill().err().map(|error| error.to_string());
    let output = child
        .wait_with_output()
        .map_err(|source| PilotError::ChildWait { ordinal, source })?;
    report.exit_code = output.status.code();
    Err(PilotError::ChildMonitorTerminated {
        ordinal,
        report: Box::new(report),
        kill_error,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn terminate_child_best_effort(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn try_wait_child(
    child: &mut std::process::Child,
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
    report: &IdentityChildTimingReport,
    ordinal: usize,
    expected_instance_id: &str,
    expected_child_pid: u32,
    graph_profile: GraphProfileId,
    n: u32,
    controlled_allocation_hard_ceiling_bytes: u64,
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
    if report.controlled_allocation_hard_ceiling_bytes != controlled_allocation_hard_ceiling_bytes {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "controlledAllocationHardCeilingBytes",
        });
    }
    match report.outcome {
        IdentityChildOutcome::Success => {
            if report.peak_live_requested_bytes == 0
                || report.peak_live_requested_bytes
                    > report.controlled_allocation_hard_ceiling_bytes
            {
                return Err(PilotError::ChildReportMismatch {
                    ordinal,
                    field: "peakLiveRequestedBytes",
                });
            }
            if report
                .wall_time_ns
                .is_none_or(|wall_time_ns| wall_time_ns == 0)
            {
                return Err(PilotError::ChildReportMismatch {
                    ordinal,
                    field: "wallTimeNs",
                });
            }
            let Some(semantic_digest_sha256) = report.semantic_digest_sha256.as_deref() else {
                return Err(PilotError::ChildReportMismatch {
                    ordinal,
                    field: "semanticDigestSha256",
                });
            };
            if semantic_digest_sha256.len() != 64
                || !semantic_digest_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
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
        IdentityChildOutcome::GuardedInChild => {
            if report.wall_time_ns.is_some()
                || report.semantic_digest_sha256.is_some()
                || report.controlled_allocation_guard.is_none()
            {
                return Err(PilotError::ChildReportMismatch {
                    ordinal,
                    field: "guardedInChild",
                });
            }
            let Some(guard) = report.controlled_allocation_guard.as_ref() else {
                return Err(PilotError::ChildReportMismatch {
                    ordinal,
                    field: "controlledAllocationGuard",
                });
            };
            if guard.hard_ceiling_bytes != report.controlled_allocation_hard_ceiling_bytes
                || guard
                    .live_requested_bytes
                    .checked_add(guard.requested_bytes)
                    .is_none_or(|would_be| would_be <= guard.hard_ceiling_bytes)
                || report.peak_live_requested_bytes > guard.live_requested_bytes
            {
                return Err(PilotError::ChildReportMismatch {
                    ordinal,
                    field: "controlledAllocationGuard",
                });
            }
        }
    }
    Ok(())
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
    #[error("新进程试运行样本 {ordinal} 缺少父子进程启动握手输入管道")]
    MissingChildStdin { ordinal: usize },
    #[error("冷实例子进程读取父进程启动信号失败")]
    ChildStartSignalRead(#[source] std::io::Error),
    #[error("冷实例子进程收到无效启动信号：0x{actual:02x}")]
    InvalidChildStartSignal { actual: u8 },
    #[error("无法向新进程试运行样本 {ordinal} 写入启动信号")]
    ChildStartSignalWrite {
        ordinal: usize,
        #[source]
        source: std::io::Error,
    },
    #[error("等待新进程试运行样本 {ordinal} 失败")]
    ChildWait {
        ordinal: usize,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "新进程试运行样本 {ordinal} 在监控握手完成前退出：exitCode={exit_code:?}, stderr={stderr}"
    )]
    ChildExitedBeforeStart {
        ordinal: usize,
        exit_code: Option<i32>,
        stderr: String,
    },
    #[error("新进程试运行样本 {ordinal} 异常退出：exitCode={exit_code:?}, stderr={stderr}")]
    ChildFailed {
        ordinal: usize,
        exit_code: Option<i32>,
        stderr: String,
    },
    #[error("无法解析新进程试运行样本 {ordinal}")]
    InvalidChildReport {
        ordinal: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("新进程试运行样本 {ordinal} 字段不匹配：{field}")]
    ChildReportMismatch { ordinal: usize, field: &'static str },
    #[error("新进程试运行重复使用编译器实例身份")]
    DuplicateCompilerInstanceId,
    #[error("新进程试运行样本 {ordinal} 在启动前触发研究停止护栏：{report:?}")]
    GuardRejected {
        ordinal: usize,
        report: Box<GuardPreflightReport>,
    },
    #[error("新进程试运行样本 {ordinal} 在子进程内正常触发受控分配硬上限：{sample:?}")]
    ChildGuarded {
        ordinal: usize,
        sample: Box<IdentityMonitoredChildSample>,
    },
    #[error(
        "新进程试运行样本 {ordinal} 被父进程监控终止：{report:?}, killError={kill_error:?}, stderr={stderr}"
    )]
    ChildMonitorTerminated {
        ordinal: usize,
        report: Box<ChildProcessMonitorReport>,
        kill_error: Option<String>,
        stderr: String,
    },
    #[error("新进程试运行需要恰好七个样本，实际为 {actual}")]
    WrongSampleCount { actual: usize },
    #[error("新进程试运行算术溢出：{0}")]
    ArithmeticOverflow(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
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
    fn rejected_preflight_never_attempts_to_spawn_the_child() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let error = run_identity_fresh_process_pilot_with_memory_observer(
            &trusted,
            Path::new("this-child-must-never-be-spawned"),
            "guard-rejection",
            GraphProfileId::WideStar,
            1,
            None,
            || {
                Ok(SystemMemoryObservation {
                    physical_memory_bytes: 64 * 1_073_741_824,
                    available_physical_memory_bytes: 1,
                })
            },
        )
        .expect_err("guard must reject");
        assert!(matches!(
            error,
            PilotError::GuardRejected { ordinal: 0, .. }
        ));
    }

    #[test]
    fn child_controlled_allocation_guard_returns_a_normal_structured_report() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let report = measure_identity_timing_child(
            &trusted,
            "guarded-child".to_owned(),
            GraphProfileId::WideStar,
            1,
            1,
        )
        .expect("controlled guard is a normal child report");
        assert_eq!(report.outcome, IdentityChildOutcome::GuardedInChild);
        assert_eq!(report.controlled_allocation_hard_ceiling_bytes, 1);
        assert_eq!(report.wall_time_ns, None);
        assert_eq!(report.semantic_digest_sha256, None);
        let guard = report.controlled_allocation_guard.expect("guard details");
        assert_eq!(guard.hard_ceiling_bytes, 1);
        assert!(guard.live_requested_bytes + guard.requested_bytes > guard.hard_ceiling_bytes);
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
    fn parent_monitor_terminates_and_reaps_a_child_at_the_private_byte_limit() {
        let executable = std::env::current_exe().expect("current test executable");
        let child = Command::new(executable)
            .arg("pilot::tests::parent_monitor_termination_helper")
            .arg("--exact")
            .arg("--nocapture")
            .env(MONITOR_TERMINATION_HELPER_ENV, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn monitor termination helper");
        let thresholds =
            GuardThresholds::from_physical_memory_bytes(64 * 1_073_741_824).expect("thresholds");
        let observation = ChildProcessMemoryObservation {
            private_bytes: thresholds.private_bytes,
            available_physical_memory_bytes: thresholds.minimum_available_physical_memory_bytes,
        };

        let error =
            run_monitored_child_with_observer(child, 0, thresholds, |_| Ok(Some(observation)))
                .expect_err("private-byte threshold must terminate the child");
        let PilotError::ChildMonitorTerminated {
            report, kill_error, ..
        } = error
        else {
            panic!("unexpected monitor result: {error}");
        };
        assert_eq!(report.trigger, Some(ChildMonitorTrigger::PrivateBytes));
        assert_eq!(report.observation_count, 1);
        assert_eq!(report.last_private_bytes, Some(thresholds.private_bytes));
        assert_eq!(report.peak_private_bytes, Some(thresholds.private_bytes));
        assert_eq!(kill_error, None);
    }

    #[test]
    fn parent_monitor_termination_helper() {
        if std::env::var_os(MONITOR_TERMINATION_HELPER_ENV).is_none() {
            return;
        }
        let mut signal = [0_u8; 1];
        std::io::stdin()
            .read_exact(&mut signal)
            .expect("helper waits for a parent start signal or process termination");
        thread::sleep(Duration::from_secs(60));
    }
}
