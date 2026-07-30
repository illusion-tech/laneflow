//! 编译器测量基准规模发现所需的新进程冷实例试运行基础。
//!
//! 本模块只执行七个独立子进程、核对冷实例结果并计算墙钟中位数与中位绝对偏差。
//! 停止护栏、基准规模选择、Evidence v1 写出与正式轮次仍由后续切片负责。

use crate::{
    GraphProfileId, GuardCompletedLevelObservation, GuardError, GuardPreflightReport,
    IdentityCompilerInstance, SystemMemoryMonitor, SystemMemoryObservation, TimingError,
    TrustedContract, evaluate_identity_guard_preflight, observe_clock_quantum_ns,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

pub const IDENTITY_TIMING_CHILD_SCHEMA: &str =
    "laneflow.compiler-calibration-identity-timing-child";
pub const IDENTITY_TIMING_CHILD_SCHEMA_VERSION: u32 = 1;
pub const FRESH_PROCESS_PILOT_SAMPLE_COUNT: usize = 7;
pub const CLOCK_QUANTUM_MULTIPLIER: u64 = 10_000;
pub const MAXIMUM_RELATIVE_MAD_PERCENT: u64 = 2;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityChildTimingReport {
    pub schema: String,
    pub schema_version: u32,
    pub compiler_instance_id: String,
    pub child_pid: u32,
    pub graph_profile: String,
    pub n: u32,
    pub wall_time_ns: u64,
    pub semantic_digest_sha256: String,
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
    pub samples: Vec<IdentityChildTimingReport>,
}

pub fn measure_identity_timing_child(
    trusted: &TrustedContract,
    compiler_instance_id: String,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<IdentityChildTimingReport, PilotError> {
    let mut instance = IdentityCompilerInstance::from_trusted_contract_with_id(
        trusted,
        compiler_instance_id.clone(),
    )?;
    let sample = instance.measure(graph_profile, n)?;
    Ok(IdentityChildTimingReport {
        schema: IDENTITY_TIMING_CHILD_SCHEMA.to_owned(),
        schema_version: IDENTITY_TIMING_CHILD_SCHEMA_VERSION,
        compiler_instance_id,
        child_pid: std::process::id(),
        graph_profile: graph_profile.as_str().to_owned(),
        n,
        wall_time_ns: sample.wall_time_ns,
        semantic_digest_sha256: sample.stage_summary.semantic_digest_sha256,
    })
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
        guard_preflights.push(guard);

        let compiler_instance_id = format!("{pilot_id}/compiler-instance-{ordinal}");
        let output = Command::new(executable)
            .arg("identity-timing-child")
            .arg(&compiler_instance_id)
            .arg(graph_profile.as_str())
            .arg(n.to_string())
            .output()
            .map_err(|source| PilotError::ChildSpawn { ordinal, source })?;
        if !output.status.success() {
            return Err(PilotError::ChildFailed {
                ordinal,
                exit_code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        let report = serde_json::from_slice::<IdentityChildTimingReport>(&output.stdout)
            .map_err(|source| PilotError::InvalidChildReport { ordinal, source })?;
        verify_child_report(&report, ordinal, &compiler_instance_id, graph_profile, n)?;
        samples.push(report);
    }

    let instance_ids = samples
        .iter()
        .map(|sample| sample.compiler_instance_id.as_str())
        .collect::<BTreeSet<_>>();
    if instance_ids.len() != FRESH_PROCESS_PILOT_SAMPLE_COUNT {
        return Err(PilotError::DuplicateCompilerInstanceId);
    }
    let semantic_digest_consistent = samples
        .windows(2)
        .all(|pair| pair[0].semantic_digest_sha256 == pair[1].semantic_digest_sha256);
    let (median_wall_time_ns, median_absolute_deviation_ns) =
        median_and_mad(samples.iter().map(|sample| sample.wall_time_ns))?;
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

fn verify_child_report(
    report: &IdentityChildTimingReport,
    ordinal: usize,
    expected_instance_id: &str,
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
    if report.compiler_instance_id != expected_instance_id {
        return Err(PilotError::ChildReportMismatch {
            ordinal,
            field: "compilerInstanceId",
        });
    }
    if report.child_pid == 0 {
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
    #[error("新进程试运行需要恰好七个样本，实际为 {actual}")]
    WrongSampleCount { actual: usize },
    #[error("新进程试运行算术溢出：{0}")]
    ArithmeticOverflow(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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
}
