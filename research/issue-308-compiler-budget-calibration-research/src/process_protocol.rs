//! #308 研究进程结果与终止状态协议。
//!
//! 本模块只实现 Evidence v1 已冻结的进程状态和值/原因观察形状。它不写出正式 Evidence，
//! 但保证试运行不会把 POSIX 信号、Windows 原生异常状态或未启动子进程压扁成模糊退出码。

use serde::{Deserialize, Serialize};
use std::process::ExitStatus;

pub const IDENTITY_PILOT_COMBINED_BINARY_ID: &str = "identity-pilot-combined-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NullableObservation<T> {
    pub value: Option<T>,
    pub reason: Option<String>,
}

impl<T> NullableObservation<T> {
    pub fn observed(value: T) -> Self {
        Self {
            value: Some(value),
            reason: None,
        }
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            value: None,
            reason: Some(reason.into()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunStatus {
    Valid,
    Invalid,
    Guarded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InvalidationReason {
    PowerSourceChange,
    PowerPlanChange,
    VendorModeChange,
    SleepOrSessionLock,
    ThermalOrPowerThrottling,
    BackgroundCpuOverOneSecond,
    #[serde(rename = "background-write-over-100-mib")]
    BackgroundWriteOver100Mib,
    MonitoringGap,
    ChildAbnormalExit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessExitKind {
    Success,
    GuardedBeforeStart,
    GuardedInChild,
    InvalidAbnormalExit,
    InvalidMonitorTermination,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TerminationKind {
    NotStarted,
    ExitCode,
    PosixSignal,
    PlatformStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminationObservation {
    pub kind: TerminationKind,
    pub signal_number: NullableObservation<u64>,
    pub raw_platform_status: NullableObservation<String>,
}

impl TerminationObservation {
    fn not_started() -> Self {
        Self {
            kind: TerminationKind::NotStarted,
            signal_number: NullableObservation::unavailable("child-not-started"),
            raw_platform_status: NullableObservation::unavailable("child-not-started"),
        }
    }

    fn exit_code() -> Self {
        Self {
            kind: TerminationKind::ExitCode,
            signal_number: NullableObservation::unavailable("not-signal-termination"),
            raw_platform_status: NullableObservation::unavailable("exit-code-is-authoritative"),
        }
    }

    #[cfg(unix)]
    fn posix_signal(signal_number: u64, raw_wait_status: u32) -> Self {
        Self {
            kind: TerminationKind::PosixSignal,
            signal_number: NullableObservation::observed(signal_number),
            raw_platform_status: NullableObservation::observed(format!(
                "posix-wait-status-hex-u32:{raw_wait_status:08x}"
            )),
        }
    }

    #[cfg(windows)]
    fn platform_status(raw_status: u64) -> Self {
        Self {
            kind: TerminationKind::PlatformStatus,
            signal_number: NullableObservation::unavailable("not-posix-signal"),
            raw_platform_status: NullableObservation::observed(format!(
                "native-status-hex-u64:{raw_status:016x}"
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessObservation {
    pub coordinator_pid: u32,
    pub child_pid: NullableObservation<u64>,
    pub binary_id: String,
    pub exit_kind: ProcessExitKind,
    pub exit_code: NullableObservation<u64>,
    pub termination: TerminationObservation,
}

impl ProcessObservation {
    pub fn guarded_before_start(coordinator_pid: u32, binary_id: impl Into<String>) -> Self {
        Self {
            coordinator_pid,
            child_pid: NullableObservation::unavailable("child-not-started"),
            binary_id: binary_id.into(),
            exit_kind: ProcessExitKind::GuardedBeforeStart,
            exit_code: NullableObservation::unavailable("child-not-started"),
            termination: TerminationObservation::not_started(),
        }
    }

    pub fn success(
        coordinator_pid: u32,
        child_pid: u32,
        binary_id: impl Into<String>,
        status: ExitStatus,
    ) -> Result<Self, ProcessProtocolError> {
        Self::from_started(
            coordinator_pid,
            child_pid,
            binary_id,
            ProcessExitKind::Success,
            status,
        )
    }

    pub fn guarded_in_child(
        coordinator_pid: u32,
        child_pid: u32,
        binary_id: impl Into<String>,
        status: ExitStatus,
    ) -> Result<Self, ProcessProtocolError> {
        Self::from_started(
            coordinator_pid,
            child_pid,
            binary_id,
            ProcessExitKind::GuardedInChild,
            status,
        )
    }

    pub fn invalid_abnormal_exit(
        coordinator_pid: u32,
        child_pid: u32,
        binary_id: impl Into<String>,
        status: ExitStatus,
    ) -> Result<Self, ProcessProtocolError> {
        Self::from_started(
            coordinator_pid,
            child_pid,
            binary_id,
            ProcessExitKind::InvalidAbnormalExit,
            status,
        )
    }

    pub fn invalid_monitor_termination(
        coordinator_pid: u32,
        child_pid: u32,
        binary_id: impl Into<String>,
        status: ExitStatus,
    ) -> Result<Self, ProcessProtocolError> {
        Self::from_started(
            coordinator_pid,
            child_pid,
            binary_id,
            ProcessExitKind::InvalidMonitorTermination,
            status,
        )
    }

    fn from_started(
        coordinator_pid: u32,
        child_pid: u32,
        binary_id: impl Into<String>,
        exit_kind: ProcessExitKind,
        status: ExitStatus,
    ) -> Result<Self, ProcessProtocolError> {
        if child_pid == 0 {
            return Err(ProcessProtocolError::ZeroChildPid);
        }
        let observed = observe_exit_status(status)?;
        match exit_kind {
            ProcessExitKind::Success | ProcessExitKind::GuardedInChild => {
                if observed.exit_code.value != Some(0)
                    || observed.termination.kind != TerminationKind::ExitCode
                {
                    return Err(ProcessProtocolError::SuccessfulKindRequiresZeroExitCode {
                        exit_kind,
                    });
                }
            }
            ProcessExitKind::InvalidAbnormalExit | ProcessExitKind::InvalidMonitorTermination => {
                if observed.termination.kind == TerminationKind::ExitCode
                    && observed.exit_code.value.is_none_or(|code| code == 0)
                {
                    return Err(
                        ProcessProtocolError::InvalidKindRequiresAbnormalTermination { exit_kind },
                    );
                }
            }
            ProcessExitKind::GuardedBeforeStart => {
                return Err(ProcessProtocolError::StartedProcessCannotBeGuardedBeforeStart);
            }
        }
        Ok(Self {
            coordinator_pid,
            child_pid: NullableObservation::observed(u64::from(child_pid)),
            binary_id: binary_id.into(),
            exit_kind,
            exit_code: observed.exit_code,
            termination: observed.termination,
        })
    }
}

#[derive(Debug)]
struct ExitStatusObservation {
    exit_code: NullableObservation<u64>,
    termination: TerminationObservation,
}

#[cfg(unix)]
fn observe_exit_status(status: ExitStatus) -> Result<ExitStatusObservation, ProcessProtocolError> {
    use std::os::unix::process::ExitStatusExt;

    if let Some(signal_number) = status.signal() {
        let signal_number = u64::try_from(signal_number)
            .map_err(|_| ProcessProtocolError::InvalidPosixSignalNumber)?;
        let raw_wait_status = status.into_raw() as u32;
        return Ok(ExitStatusObservation {
            exit_code: NullableObservation::unavailable("signal-termination"),
            termination: TerminationObservation::posix_signal(signal_number, raw_wait_status),
        });
    }
    let exit_code = status
        .code()
        .ok_or(ProcessProtocolError::UnavailablePlatformStatus)?;
    let exit_code = u64::try_from(exit_code)
        .map_err(|_| ProcessProtocolError::NegativeExitCodeWithoutStatus)?;
    Ok(ExitStatusObservation {
        exit_code: NullableObservation::observed(exit_code),
        termination: TerminationObservation::exit_code(),
    })
}

#[cfg(windows)]
fn observe_exit_status(status: ExitStatus) -> Result<ExitStatusObservation, ProcessProtocolError> {
    let exit_code = status
        .code()
        .ok_or(ProcessProtocolError::UnavailablePlatformStatus)?;
    if let Ok(exit_code) = u64::try_from(exit_code) {
        return Ok(ExitStatusObservation {
            exit_code: NullableObservation::observed(exit_code),
            termination: TerminationObservation::exit_code(),
        });
    }
    let raw_status = u64::from(u32::from_ne_bytes(exit_code.to_ne_bytes()));
    Ok(ExitStatusObservation {
        exit_code: NullableObservation::unavailable("platform-status-without-exit-code"),
        termination: TerminationObservation::platform_status(raw_status),
    })
}

#[cfg(not(any(unix, windows)))]
fn observe_exit_status(status: ExitStatus) -> Result<ExitStatusObservation, ProcessProtocolError> {
    let exit_code = status
        .code()
        .ok_or(ProcessProtocolError::UnavailablePlatformStatus)?;
    let exit_code = u64::try_from(exit_code)
        .map_err(|_| ProcessProtocolError::NegativeExitCodeWithoutStatus)?;
    Ok(ExitStatusObservation {
        exit_code: NullableObservation::observed(exit_code),
        termination: TerminationObservation::exit_code(),
    })
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessProtocolError {
    #[error("进程状态协议不允许 childPid 为零")]
    ZeroChildPid,
    #[error("已启动进程不能编码为 guarded-before-start")]
    StartedProcessCannotBeGuardedBeforeStart,
    #[error("进程状态 {exit_kind:?} 必须由正常退出码零形成")]
    SuccessfulKindRequiresZeroExitCode { exit_kind: ProcessExitKind },
    #[error("进程状态 {exit_kind:?} 必须由非零退出码、POSIX 信号或原生平台状态形成")]
    InvalidKindRequiresAbnormalTermination { exit_kind: ProcessExitKind },
    #[error("POSIX 信号号不能转换为正整数")]
    InvalidPosixSignalNumber,
    #[error("当前平台没有可用退出码或原生终止状态")]
    UnavailablePlatformStatus,
    #[error("当前平台返回负退出码但没有可保存的原生终止状态")]
    NegativeExitCodeWithoutStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarded_before_start_uses_only_child_not_started_observations() {
        let process =
            ProcessObservation::guarded_before_start(42, IDENTITY_PILOT_COMBINED_BINARY_ID);
        assert_eq!(process.exit_kind, ProcessExitKind::GuardedBeforeStart);
        assert_eq!(process.child_pid.value, None);
        assert_eq!(
            process.child_pid.reason.as_deref(),
            Some("child-not-started")
        );
        assert_eq!(process.termination.kind, TerminationKind::NotStarted);
        assert_eq!(process.exit_code.value, None);
    }

    #[test]
    fn invalidation_reason_tokens_match_the_closed_evidence_v1_enum() {
        let tokens = serde_json::to_value([
            InvalidationReason::PowerSourceChange,
            InvalidationReason::PowerPlanChange,
            InvalidationReason::VendorModeChange,
            InvalidationReason::SleepOrSessionLock,
            InvalidationReason::ThermalOrPowerThrottling,
            InvalidationReason::BackgroundCpuOverOneSecond,
            InvalidationReason::BackgroundWriteOver100Mib,
            InvalidationReason::MonitoringGap,
            InvalidationReason::ChildAbnormalExit,
        ])
        .expect("serialize invalidation reasons");
        assert_eq!(
            tokens,
            serde_json::json!([
                "power-source-change",
                "power-plan-change",
                "vendor-mode-change",
                "sleep-or-session-lock",
                "thermal-or-power-throttling",
                "background-cpu-over-one-second",
                "background-write-over-100-mib",
                "monitoring-gap",
                "child-abnormal-exit"
            ])
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_exit_codes_and_native_statuses_remain_distinct() {
        use std::os::windows::process::ExitStatusExt;

        let success = ProcessObservation::success(
            1,
            2,
            IDENTITY_PILOT_COMBINED_BINARY_ID,
            ExitStatus::from_raw(0),
        )
        .expect("zero exit");
        assert_eq!(success.exit_code.value, Some(0));
        assert_eq!(success.termination.kind, TerminationKind::ExitCode);

        let abnormal = ProcessObservation::invalid_abnormal_exit(
            1,
            2,
            IDENTITY_PILOT_COMBINED_BINARY_ID,
            ExitStatus::from_raw(5),
        )
        .expect("nonzero exit");
        assert_eq!(abnormal.exit_code.value, Some(5));
        assert_eq!(abnormal.termination.kind, TerminationKind::ExitCode);

        let native = ProcessObservation::invalid_monitor_termination(
            1,
            2,
            IDENTITY_PILOT_COMBINED_BINARY_ID,
            ExitStatus::from_raw(0xc000_0005),
        )
        .expect("Windows native status");
        assert_eq!(native.exit_code.value, None);
        assert_eq!(
            native.exit_code.reason.as_deref(),
            Some("platform-status-without-exit-code")
        );
        assert_eq!(native.termination.kind, TerminationKind::PlatformStatus);
        assert_eq!(
            native.termination.raw_platform_status.value.as_deref(),
            Some("native-status-hex-u64:00000000c0000005")
        );
    }

    #[cfg(unix)]
    #[test]
    fn posix_signal_preserves_signal_number_and_raw_wait_status() {
        use std::os::unix::process::ExitStatusExt;

        let process = ProcessObservation::invalid_monitor_termination(
            1,
            2,
            IDENTITY_PILOT_COMBINED_BINARY_ID,
            ExitStatus::from_raw(9),
        )
        .expect("POSIX signal");
        assert_eq!(process.exit_code.value, None);
        assert_eq!(
            process.exit_code.reason.as_deref(),
            Some("signal-termination")
        );
        assert_eq!(process.termination.kind, TerminationKind::PosixSignal);
        assert_eq!(process.termination.signal_number.value, Some(9));
        assert_eq!(
            process.termination.raw_platform_status.value.as_deref(),
            Some("posix-wait-status-hex-u32:00000009")
        );
    }

    #[cfg(unix)]
    #[test]
    fn success_and_invalid_kinds_reject_incompatible_exit_codes_on_posix() {
        use std::os::unix::process::ExitStatusExt;

        assert!(matches!(
            ProcessObservation::success(
                1,
                2,
                IDENTITY_PILOT_COMBINED_BINARY_ID,
                ExitStatus::from_raw(1 << 8),
            ),
            Err(ProcessProtocolError::SuccessfulKindRequiresZeroExitCode { .. })
        ));
        assert!(matches!(
            ProcessObservation::invalid_abnormal_exit(
                1,
                2,
                IDENTITY_PILOT_COMBINED_BINARY_ID,
                ExitStatus::from_raw(0),
            ),
            Err(ProcessProtocolError::InvalidKindRequiresAbnormalTermination { .. })
        ));
    }

    #[cfg(windows)]
    #[test]
    fn success_and_invalid_kinds_reject_incompatible_exit_codes_on_windows() {
        use std::os::windows::process::ExitStatusExt;

        assert!(matches!(
            ProcessObservation::success(
                1,
                2,
                IDENTITY_PILOT_COMBINED_BINARY_ID,
                ExitStatus::from_raw(1),
            ),
            Err(ProcessProtocolError::SuccessfulKindRequiresZeroExitCode { .. })
        ));
        assert!(matches!(
            ProcessObservation::invalid_abnormal_exit(
                1,
                2,
                IDENTITY_PILOT_COMBINED_BINARY_ID,
                ExitStatus::from_raw(0),
            ),
            Err(ProcessProtocolError::InvalidKindRequiresAbnormalTermination { .. })
        ));
    }
}
