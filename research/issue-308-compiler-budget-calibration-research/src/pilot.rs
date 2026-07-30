//! 编译器测量基准规模发现所需的新进程冷实例试运行基础。
//!
//! 本模块执行启动前停止护栏、七个受监控的独立子进程、冷实例结果核对和墙钟中位数/
//! 中位绝对偏差计算。基准规模选择、正式二进制角色分离、Evidence v1 写出与正式轮次
//! 仍由后续切片负责。

use crate::{
    ChildProcessMemoryMonitor, ChildProcessMemoryObservation, GraphProfileId,
    GuardCompletedLevelObservation, GuardError, GuardPreflightReport, GuardThresholds,
    IDENTITY_PILOT_COMBINED_BINARY_ID, IdentityCompilerInstance, InvalidationReason,
    NullableObservation, ProcessObservation, ProcessProtocolError, RunStatus, StageGenerationError,
    SystemMemoryMonitor, SystemMemoryObservation, TimingError, TrustedContract,
    evaluate_identity_guard_preflight, observe_clock_quantum_ns,
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
    pub child: Option<IdentityChildTimingReport>,
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

struct IdentityFreshProcessPilotRequest<'a> {
    executable: &'a Path,
    pilot_id: &'a str,
    graph_profile: GraphProfileId,
    n: u32,
    previous: Option<GuardCompletedLevelObservation>,
    controlled_allocation_hard_ceiling_cap_bytes: Option<u64>,
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
) -> Result<IdentityFreshProcessPilotOutcome, PilotError> {
    let mut memory_monitor = SystemMemoryMonitor::new()?;
    run_identity_fresh_process_pilot_with_memory_observer(
        trusted,
        IdentityFreshProcessPilotRequest {
            executable,
            pilot_id,
            graph_profile,
            n,
            previous,
            controlled_allocation_hard_ceiling_cap_bytes: None,
        },
        || memory_monitor.observe(),
    )
}

pub fn run_identity_fresh_process_pilot_with_allocation_ceiling_cap(
    trusted: &TrustedContract,
    executable: &Path,
    pilot_id: &str,
    graph_profile: GraphProfileId,
    n: u32,
    previous: Option<GuardCompletedLevelObservation>,
    controlled_allocation_hard_ceiling_cap_bytes: u64,
) -> Result<IdentityFreshProcessPilotOutcome, PilotError> {
    if controlled_allocation_hard_ceiling_cap_bytes == 0 {
        return Err(PilotError::ZeroControlledAllocationCeilingCap);
    }
    let mut memory_monitor = SystemMemoryMonitor::new()?;
    run_identity_fresh_process_pilot_with_memory_observer(
        trusted,
        IdentityFreshProcessPilotRequest {
            executable,
            pilot_id,
            graph_profile,
            n,
            previous,
            controlled_allocation_hard_ceiling_cap_bytes: Some(
                controlled_allocation_hard_ceiling_cap_bytes,
            ),
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
        executable,
        pilot_id,
        graph_profile,
        n,
        previous,
        controlled_allocation_hard_ceiling_cap_bytes,
    } = request;
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
                        IDENTITY_PILOT_COMBINED_BINARY_ID,
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
        let controlled_allocation_hard_ceiling_bytes = controlled_allocation_hard_ceiling_cap_bytes
            .map_or(thresholds.compiler_controlled_bytes, |cap| {
                cap.min(thresholds.compiler_controlled_bytes)
            });

        let compiler_instance_id = format!("{pilot_id}/compiler-instance-{ordinal}");
        let execution = run_monitored_identity_child(
            executable,
            ordinal,
            &compiler_instance_id,
            graph_profile,
            n,
            thresholds,
            controlled_allocation_hard_ceiling_bytes,
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
            let mut invalidation_reasons = vec![InvalidationReason::ChildAbnormalExit];
            if monitor.trigger == Some(ChildMonitorTrigger::MonitoringGap) {
                invalidation_reasons.push(InvalidationReason::MonitoringGap);
            }
            return Ok(IdentityFreshProcessPilotOutcome::Stopped {
                stop: Box::new(IdentityFreshProcessPilotStop {
                    pilot_id: pilot_id.to_owned(),
                    graph_profile: graph_profile.as_str().to_owned(),
                    n,
                    sample_ordinal: ordinal,
                    status: RunStatus::Invalid,
                    invalidation_reasons,
                    process: ProcessObservation::invalid_monitor_termination(
                        std::process::id(),
                        child_pid,
                        IDENTITY_PILOT_COMBINED_BINARY_ID,
                        output.status,
                    )?,
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
                        IDENTITY_PILOT_COMBINED_BINARY_ID,
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
        let report = serde_json::from_slice::<IdentityChildTimingReport>(&output.stdout)
            .map_err(|source| PilotError::InvalidChildReport { ordinal, source })?;
        verify_child_report(
            &report,
            ordinal,
            &compiler_instance_id,
            child_pid,
            graph_profile,
            n,
            controlled_allocation_hard_ceiling_bytes,
        )?;
        if report.outcome == IdentityChildOutcome::GuardedInChild {
            return Ok(IdentityFreshProcessPilotOutcome::Stopped {
                stop: Box::new(IdentityFreshProcessPilotStop {
                    pilot_id: pilot_id.to_owned(),
                    graph_profile: graph_profile.as_str().to_owned(),
                    n,
                    sample_ordinal: ordinal,
                    status: RunStatus::Guarded,
                    invalidation_reasons: Vec::new(),
                    process: ProcessObservation::guarded_in_child(
                        std::process::id(),
                        child_pid,
                        IDENTITY_PILOT_COMBINED_BINARY_ID,
                        output.status,
                    )?,
                    guard_preflight: guard,
                    child: Some(report),
                    monitor: Some(monitor),
                    kill_error: None,
                    monitor_error: None,
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                }),
            });
        }
        let process = ProcessObservation::success(
            std::process::id(),
            child_pid,
            IDENTITY_PILOT_COMBINED_BINARY_ID,
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

#[derive(Debug)]
enum MonitoredChildExecution {
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

fn run_monitored_identity_child(
    executable: &Path,
    ordinal: usize,
    compiler_instance_id: &str,
    graph_profile: GraphProfileId,
    n: u32,
    thresholds: GuardThresholds,
    controlled_allocation_hard_ceiling_bytes: u64,
) -> Result<MonitoredChildExecution, PilotError> {
    let mut memory_monitor = ChildProcessMemoryMonitor::new()?;
    let child = Command::new(executable)
        .arg("identity-timing-child")
        .arg(compiler_instance_id)
        .arg(graph_profile.as_str())
        .arg(n.to_string())
        .arg(controlled_allocation_hard_ceiling_bytes.to_string())
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
    let Some(mut child_stdin) = child.stdin.take() else {
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
        if let Some(status) = try_wait_child(&mut child, ordinal)? {
            let elapsed_wall_time_ns = duration_ns(started.elapsed())?;
            let output = child
                .wait_with_output()
                .map_err(|source| PilotError::ChildWait { ordinal, source })?;
            debug_assert_eq!(status.code(), output.status.code());
            return Ok(MonitoredChildExecution::Exited {
                child_pid,
                output,
                monitor: ChildProcessMonitorReport {
                    observation_count,
                    last_private_bytes: NullableObservation::observed(last_private_bytes),
                    peak_private_bytes: NullableObservation::observed(peak_private_bytes),
                    elapsed_wall_time_ns,
                    trigger: None,
                },
            });
        }

        let elapsed = started.elapsed();
        let observation = match observe_child(child_pid) {
            Ok(Some(observation)) => observation,
            Ok(None) => {
                if try_wait_child(&mut child, ordinal)?.is_some() {
                    continue;
                }
                let report = ChildProcessMonitorReport {
                    observation_count,
                    last_private_bytes: NullableObservation::observed(last_private_bytes),
                    peak_private_bytes: NullableObservation::observed(peak_private_bytes),
                    elapsed_wall_time_ns: duration_ns(elapsed)?,
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
                    elapsed_wall_time_ns: duration_ns(elapsed)?,
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
    child_pid: u32,
    report: ChildProcessMonitorReport,
    monitor_error: Option<String>,
) -> Result<MonitoredChildExecution, PilotError> {
    let kill_error = child.kill().err().map(|error| error.to_string());
    let output = child
        .wait_with_output()
        .map_err(|source| PilotError::ChildWait { ordinal, source })?;
    Ok(MonitoredChildExecution::InvalidatedByMonitor {
        child_pid,
        output,
        monitor: report,
        kill_error,
        monitor_error,
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
    #[error(transparent)]
    ProcessProtocol(#[from] ProcessProtocolError),
    #[error("新进程试运行标识符不能为空")]
    EmptyPilotId,
    #[error("受控分配硬上限测试上限必须大于零")]
    ZeroControlledAllocationCeilingCap,
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
        let outcome = run_identity_fresh_process_pilot_with_memory_observer(
            &trusted,
            IdentityFreshProcessPilotRequest {
                executable: Path::new("this-child-must-never-be-spawned"),
                pilot_id: "guard-rejection",
                graph_profile: GraphProfileId::WideStar,
                n: 1,
                previous: None,
                controlled_allocation_hard_ceiling_cap_bytes: None,
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
            IDENTITY_PILOT_COMBINED_BINARY_ID,
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
            IDENTITY_PILOT_COMBINED_BINARY_ID,
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
            IDENTITY_PILOT_COMBINED_BINARY_ID,
            output.status,
        )
        .expect("nonzero exit code must form an abnormal-exit observation");
        assert_eq!(
            process.exit_kind,
            crate::ProcessExitKind::InvalidAbnormalExit
        );
        assert_eq!(process.exit_code.value, Some(7));
    }

    fn spawn_monitor_termination_helper(mode: &str) -> std::process::Child {
        let executable = std::env::current_exe().expect("current test executable");
        Command::new(executable)
            .arg("pilot::tests::parent_monitor_termination_helper")
            .arg("--exact")
            .arg("--nocapture")
            .env(MONITOR_TERMINATION_HELPER_ENV, mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn monitor termination helper")
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
}
