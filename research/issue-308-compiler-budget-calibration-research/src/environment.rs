//! 正式 R0 研究环境与每次子进程的外部状态观察。
//!
//! 可跨平台稳定取得的 OS、CPU、物理内存与后台进程计数由 Rust 直接采集。厂商性能
//! 模式、BIOS 标识以及缺少统一操作系统接口的睡眠/锁屏和热/功耗观察由操作者在一次
//! 正式执行前声明；声明会进入原始检查点，不由 Evidence 写出器补造。

use crate::NullableObservation;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Instant, SystemTime};
use sysinfo::{ProcessesToUpdate, System};
use thiserror::Error;

const CPU_INVALIDATION_THRESHOLD_NS: u64 = 1_000_000_000;
const WRITE_INVALIDATION_THRESHOLD_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormalEnvironmentDeclaration {
    pub vendor_performance_mode: String,
    pub bios_firmware: String,
    pub sleep_or_session_lock_observed: bool,
    pub thermal_or_power_throttling_observed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalEnvironmentSnapshot {
    pub os: String,
    pub os_build: String,
    pub cpu: String,
    pub logical_processor_count: u64,
    pub physical_memory_bytes: u64,
    pub target_triple: String,
    pub rustc: String,
    pub llvm: String,
    pub power_source: String,
    pub vendor_performance_mode: String,
    pub power_plan: String,
    pub bios_firmware: String,
    pub monitoring_provider: String,
    pub background_process_audit: Vec<BackgroundProcessAudit>,
    pub operator_declaration: FormalEnvironmentDeclaration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundProcessAudit {
    pub name: String,
    pub pid: u32,
    pub cpu_time_ns: u64,
    pub write_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundProcessDelta {
    pub name: String,
    pub pid: u32,
    pub cpu_time_delta_ns: u64,
    pub write_bytes_delta: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalStateObservation {
    pub power_source: String,
    pub vendor_performance_mode: String,
    pub power_plan: String,
    pub sleep_or_session_lock: bool,
    pub thermal_or_power_throttling: bool,
    pub background_cpu_time_ns: NullableObservation<u64>,
    pub background_write_bytes: NullableObservation<u64>,
    pub monitoring_gap: bool,
    pub background_process_deltas: Vec<BackgroundProcessDelta>,
}

#[derive(Clone, Debug)]
struct ProcessCounters {
    name: String,
    cpu_time_ms: u64,
    write_bytes: u64,
}

#[derive(Debug)]
pub(crate) struct ExternalStateMonitor {
    started_instant: Instant,
    started_system_time: SystemTime,
    child_pid: u32,
    initial_processes: Option<BTreeMap<u32, ProcessCounters>>,
}

static FORMAL_ENVIRONMENT: OnceLock<FormalEnvironmentSnapshot> = OnceLock::new();

pub fn load_and_install_formal_environment(
    path: &Path,
) -> Result<&'static FormalEnvironmentSnapshot, EnvironmentError> {
    let bytes = fs::read(path).map_err(|source| EnvironmentError::ReadDeclaration {
        path: path.to_path_buf(),
        source,
    })?;
    let declaration =
        serde_json::from_slice::<FormalEnvironmentDeclaration>(&bytes).map_err(|source| {
            EnvironmentError::InvalidDeclaration {
                path: path.to_path_buf(),
                source,
            }
        })?;
    validate_declaration(&declaration)?;
    let snapshot = capture_environment_snapshot(declaration)?;
    FORMAL_ENVIRONMENT
        .set(snapshot)
        .map_err(|_| EnvironmentError::AlreadyInstalled)?;
    Ok(FORMAL_ENVIRONMENT
        .get()
        .expect("formal environment was installed"))
}

pub fn installed_formal_environment() -> Option<&'static FormalEnvironmentSnapshot> {
    FORMAL_ENVIRONMENT.get()
}

impl ExternalStateMonitor {
    pub(crate) fn start(child_pid: u32) -> Option<Self> {
        installed_formal_environment()?;
        Some(Self {
            started_instant: Instant::now(),
            started_system_time: SystemTime::now(),
            child_pid,
            initial_processes: process_counters(&[std::process::id(), child_pid]).ok(),
        })
    }

    pub(crate) fn finish(self) -> ExternalStateObservation {
        let environment = installed_formal_environment()
            .expect("an external-state monitor is created only after environment installation");
        // 必须先冻结后台进程测量区间，再执行较慢的平台状态查询；否则 PowerShell/
        // powercfg 的采集器自身时延会被错误归入工作负载。
        let final_processes = process_counters(&[std::process::id(), self.child_pid]).ok();
        let (final_power_source, power_source_gap) = current_power_source().map_or_else(
            |_| (environment.power_source.clone(), true),
            |value| (value, false),
        );
        let (final_power_plan, power_plan_gap) = current_power_plan().map_or_else(
            |_| (environment.power_plan.clone(), true),
            |value| (value, false),
        );
        let (background_process_deltas, process_gap) =
            match (&self.initial_processes, &final_processes) {
                (Some(initial), Some(final_processes)) => {
                    let (deltas, missing_process) =
                        derive_background_deltas(initial, final_processes);
                    (deltas, missing_process)
                }
                _ => (Vec::new(), true),
            };
        let background_cpu_time_ns = background_process_deltas
            .iter()
            .try_fold(0_u64, |total, delta| {
                total.checked_add(delta.cpu_time_delta_ns)
            });
        let background_write_bytes = background_process_deltas
            .iter()
            .try_fold(0_u64, |total, delta| {
                total.checked_add(delta.write_bytes_delta)
            });
        let elapsed_monotonic = self.started_instant.elapsed();
        let (sleep_or_clock_gap, clock_gap) = SystemTime::now()
            .duration_since(self.started_system_time)
            .map_or((false, true), |elapsed_wall| {
                (
                    elapsed_wall.abs_diff(elapsed_monotonic).as_secs() >= 1,
                    false,
                )
            });
        let monitoring_gap = process_gap
            || power_source_gap
            || power_plan_gap
            || clock_gap
            || background_cpu_time_ns.is_none()
            || background_write_bytes.is_none();
        ExternalStateObservation {
            power_source: final_power_source,
            vendor_performance_mode: environment.vendor_performance_mode.clone(),
            power_plan: final_power_plan,
            sleep_or_session_lock: environment
                .operator_declaration
                .sleep_or_session_lock_observed
                || sleep_or_clock_gap,
            thermal_or_power_throttling: environment
                .operator_declaration
                .thermal_or_power_throttling_observed,
            background_cpu_time_ns: if monitoring_gap {
                NullableObservation::unavailable("monitoring-gap")
            } else {
                NullableObservation::observed(
                    background_cpu_time_ns.expect("checked background CPU total"),
                )
            },
            background_write_bytes: if monitoring_gap {
                NullableObservation::unavailable("monitoring-gap")
            } else {
                NullableObservation::observed(
                    background_write_bytes.expect("checked background write total"),
                )
            },
            monitoring_gap,
            background_process_deltas,
        }
    }
}

impl ExternalStateObservation {
    pub fn invalidation_reasons(
        &self,
        environment: &FormalEnvironmentSnapshot,
    ) -> Vec<crate::InvalidationReason> {
        let mut reasons = Vec::new();
        if self.power_source != environment.power_source {
            reasons.push(crate::InvalidationReason::PowerSourceChange);
        }
        if self.power_plan != environment.power_plan {
            reasons.push(crate::InvalidationReason::PowerPlanChange);
        }
        if self.vendor_performance_mode != environment.vendor_performance_mode {
            reasons.push(crate::InvalidationReason::VendorModeChange);
        }
        if self.sleep_or_session_lock {
            reasons.push(crate::InvalidationReason::SleepOrSessionLock);
        }
        if self.thermal_or_power_throttling {
            reasons.push(crate::InvalidationReason::ThermalOrPowerThrottling);
        }
        if self
            .background_cpu_time_ns
            .value
            .is_some_and(|value| value > CPU_INVALIDATION_THRESHOLD_NS)
        {
            reasons.push(crate::InvalidationReason::BackgroundCpuOverOneSecond);
        }
        if self
            .background_write_bytes
            .value
            .is_some_and(|value| value > WRITE_INVALIDATION_THRESHOLD_BYTES)
        {
            reasons.push(crate::InvalidationReason::BackgroundWriteOver100Mib);
        }
        if self.monitoring_gap {
            reasons.push(crate::InvalidationReason::MonitoringGap);
        }
        reasons
    }
}

fn capture_environment_snapshot(
    declaration: FormalEnvironmentDeclaration,
) -> Result<FormalEnvironmentSnapshot, EnvironmentError> {
    let mut system = System::new_all();
    system.refresh_all();
    let cpu = system
        .cpus()
        .first()
        .map(|cpu| cpu.brand().trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or(EnvironmentError::MissingSystemFact("cpu"))?;
    let rustc_verbose = command_stdout("rustc", ["+1.96.0", "-vV"])?;
    let rustc = command_stdout("rustc", ["+1.96.0", "--version"])?;
    let target_triple = rustc_verbose_field(&rustc_verbose, "host")?;
    let llvm = rustc_verbose_field(&rustc_verbose, "LLVM version")?;
    let background_process_audit = process_counters(&[std::process::id()])?
        .into_iter()
        .map(|(pid, counters)| BackgroundProcessAudit {
            name: counters.name,
            pid,
            cpu_time_ns: counters.cpu_time_ms.saturating_mul(1_000_000),
            write_bytes: counters.write_bytes,
        })
        .collect();
    Ok(FormalEnvironmentSnapshot {
        os: System::name().ok_or(EnvironmentError::MissingSystemFact("os"))?,
        os_build: System::long_os_version()
            .ok_or(EnvironmentError::MissingSystemFact("osBuild"))?,
        cpu,
        logical_processor_count: u64::try_from(system.cpus().len())
            .map_err(|_| EnvironmentError::ProcessorCountOverflow)?,
        physical_memory_bytes: system.total_memory(),
        target_triple,
        rustc,
        llvm,
        power_source: current_power_source()?,
        vendor_performance_mode: declaration.vendor_performance_mode.clone(),
        power_plan: current_power_plan()?,
        bios_firmware: declaration.bios_firmware.clone(),
        monitoring_provider: "sysinfo-0.39.6-process-snapshot-v1".to_owned(),
        background_process_audit,
        operator_declaration: declaration,
    })
}

fn validate_declaration(
    declaration: &FormalEnvironmentDeclaration,
) -> Result<(), EnvironmentError> {
    if declaration.vendor_performance_mode.trim().is_empty() {
        return Err(EnvironmentError::EmptyDeclarationField(
            "vendorPerformanceMode",
        ));
    }
    if declaration.bios_firmware.trim().is_empty() {
        return Err(EnvironmentError::EmptyDeclarationField("biosFirmware"));
    }
    Ok(())
}

fn process_counters(
    excluded_pids: &[u32],
) -> Result<BTreeMap<u32, ProcessCounters>, EnvironmentError> {
    if !sysinfo::IS_SUPPORTED_SYSTEM {
        return Err(EnvironmentError::UnsupportedProcessProvider);
    }
    let excluded = excluded_pids.iter().copied().collect::<BTreeSet<_>>();
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let mut counters = BTreeMap::new();
    for (pid, process) in system.processes() {
        let pid = pid.as_u32();
        if excluded.contains(&pid) {
            continue;
        }
        let name = os_str_to_non_empty(process.name(), pid);
        counters.insert(
            pid,
            ProcessCounters {
                name,
                cpu_time_ms: process.accumulated_cpu_time(),
                write_bytes: process.disk_usage().total_written_bytes,
            },
        );
    }
    Ok(counters)
}

fn derive_background_deltas(
    before: &BTreeMap<u32, ProcessCounters>,
    after: &BTreeMap<u32, ProcessCounters>,
) -> (Vec<BackgroundProcessDelta>, bool) {
    let missing_process = before.keys().any(|pid| !after.contains_key(pid));
    let mut deltas = Vec::new();
    for (pid, final_counters) in after {
        let initial = before.get(pid);
        let cpu_time_delta_ms = initial.map_or(final_counters.cpu_time_ms, |initial| {
            final_counters
                .cpu_time_ms
                .saturating_sub(initial.cpu_time_ms)
        });
        let write_bytes_delta = initial.map_or(final_counters.write_bytes, |initial| {
            final_counters
                .write_bytes
                .saturating_sub(initial.write_bytes)
        });
        if cpu_time_delta_ms != 0 || write_bytes_delta != 0 {
            deltas.push(BackgroundProcessDelta {
                name: final_counters.name.clone(),
                pid: *pid,
                cpu_time_delta_ns: cpu_time_delta_ms.saturating_mul(1_000_000),
                write_bytes_delta,
            });
        }
    }
    (deltas, missing_process)
}

fn os_str_to_non_empty(value: &OsStr, pid: u32) -> String {
    let value = value.to_string_lossy().trim().to_owned();
    if value.is_empty() {
        format!("pid-{pid}")
    } else {
        value
    }
}

fn command_stdout<const N: usize>(
    executable: &str,
    arguments: [&str; N],
) -> Result<String, EnvironmentError> {
    let output = Command::new(executable)
        .args(arguments)
        .output()
        .map_err(|source| EnvironmentError::CommandLaunch {
            executable: executable.to_owned(),
            source,
        })?;
    if !output.status.success() {
        return Err(EnvironmentError::CommandFailed {
            executable: executable.to_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let stdout = String::from_utf8(output.stdout).map_err(|source| {
        EnvironmentError::CommandOutputNotUtf8 {
            executable: executable.to_owned(),
            source,
        }
    })?;
    let stdout = stdout.trim().to_owned();
    if stdout.is_empty() {
        return Err(EnvironmentError::CommandOutputEmpty {
            executable: executable.to_owned(),
        });
    }
    Ok(stdout)
}

fn rustc_verbose_field(output: &str, name: &'static str) -> Result<String, EnvironmentError> {
    output
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{name}: ")))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or(EnvironmentError::MissingRustcField(name))
}

#[cfg(windows)]
fn current_power_plan() -> Result<String, EnvironmentError> {
    command_stdout("powercfg", ["/GETACTIVESCHEME"])
}

#[cfg(not(windows))]
fn current_power_plan() -> Result<String, EnvironmentError> {
    Err(EnvironmentError::UnsupportedPowerPlanProvider)
}

#[cfg(windows)]
fn current_power_source() -> Result<String, EnvironmentError> {
    let value = command_stdout(
        "powershell.exe",
        [
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SystemInformation]::PowerStatus.PowerLineStatus.ToString().ToLowerInvariant()",
        ],
    )?;
    parse_windows_power_line_status(&value)
}

#[cfg(windows)]
fn parse_windows_power_line_status(value: &str) -> Result<String, EnvironmentError> {
    match value {
        "online" => Ok("ac".to_owned()),
        "offline" => Ok("battery".to_owned()),
        _ => Err(EnvironmentError::UnknownPowerSource),
    }
}

#[cfg(not(windows))]
fn current_power_source() -> Result<String, EnvironmentError> {
    Err(EnvironmentError::UnsupportedPowerSourceProvider)
}

#[derive(Debug, Error)]
pub enum EnvironmentError {
    #[error("无法读取正式环境声明 {path}: {source}")]
    ReadDeclaration {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("正式环境声明不是合法 JSON {path}: {source}")]
    InvalidDeclaration {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("正式环境声明字段 {0} 不能为空")]
    EmptyDeclarationField(&'static str),
    #[error("正式环境已在本进程安装")]
    AlreadyInstalled,
    #[error("正式环境尚未安装")]
    NotInstalled,
    #[error("系统事实 {0} 不可用")]
    MissingSystemFact(&'static str),
    #[error("逻辑处理器数量无法表示")]
    ProcessorCountOverflow,
    #[error("当前平台不支持进程背景观察")]
    UnsupportedProcessProvider,
    #[error("当前平台不支持活动电源计划观察")]
    UnsupportedPowerPlanProvider,
    #[error("当前平台不支持电源来源观察")]
    UnsupportedPowerSourceProvider,
    #[error("Windows 返回未知电源来源")]
    UnknownPowerSource,
    #[error("运行 {executable} 失败：{source}")]
    CommandLaunch {
        executable: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{executable} 返回失败：{stderr}")]
    CommandFailed { executable: String, stderr: String },
    #[error("{executable} 输出不是 UTF-8：{source}")]
    CommandOutputNotUtf8 {
        executable: String,
        #[source]
        source: std::string::FromUtf8Error,
    },
    #[error("{executable} 没有输出")]
    CommandOutputEmpty { executable: String },
    #[error("rustc -vV 缺少 {0}")]
    MissingRustcField(&'static str),
    #[error("系统墙钟向后跳变")]
    SystemClockMovedBackwards,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counters(name: &str, cpu_time_ms: u64, write_bytes: u64) -> ProcessCounters {
        ProcessCounters {
            name: name.to_owned(),
            cpu_time_ms,
            write_bytes,
        }
    }

    #[test]
    fn background_deltas_use_accumulated_counters_and_detect_missing_processes() {
        let before = BTreeMap::from([
            (10, counters("stable", 100, 200)),
            (20, counters("exited", 50, 60)),
        ]);
        let after = BTreeMap::from([
            (10, counters("stable", 125, 260)),
            (30, counters("new", 7, 11)),
        ]);
        let (deltas, missing) = derive_background_deltas(&before, &after);
        assert!(missing);
        assert_eq!(deltas.len(), 2);
        assert_eq!(deltas[0].cpu_time_delta_ns, 25_000_000);
        assert_eq!(deltas[0].write_bytes_delta, 60);
        assert_eq!(deltas[1].cpu_time_delta_ns, 7_000_000);
        assert_eq!(deltas[1].write_bytes_delta, 11);
    }

    #[cfg(windows)]
    #[test]
    fn windows_power_line_status_maps_only_explicit_online_and_offline_values() {
        assert_eq!(
            parse_windows_power_line_status("online").expect("online power status"),
            "ac"
        );
        assert_eq!(
            parse_windows_power_line_status("offline").expect("offline power status"),
            "battery"
        );
        assert!(matches!(
            parse_windows_power_line_status("unknown"),
            Err(EnvironmentError::UnknownPowerSource)
        ));
    }

    #[test]
    fn external_thresholds_are_strict_and_monitoring_gap_is_structured() {
        let clear = ExternalStateObservation {
            power_source: "ac".to_owned(),
            vendor_performance_mode: "performance".to_owned(),
            power_plan: "balanced".to_owned(),
            sleep_or_session_lock: false,
            thermal_or_power_throttling: false,
            background_cpu_time_ns: NullableObservation::observed(CPU_INVALIDATION_THRESHOLD_NS),
            background_write_bytes: NullableObservation::observed(
                WRITE_INVALIDATION_THRESHOLD_BYTES,
            ),
            monitoring_gap: false,
            background_process_deltas: Vec::new(),
        };
        let environment = FormalEnvironmentSnapshot {
            os: "test".to_owned(),
            os_build: "test".to_owned(),
            cpu: "test".to_owned(),
            logical_processor_count: 1,
            physical_memory_bytes: 1,
            target_triple: "test".to_owned(),
            rustc: "test".to_owned(),
            llvm: "test".to_owned(),
            power_source: "ac".to_owned(),
            vendor_performance_mode: "performance".to_owned(),
            power_plan: "balanced".to_owned(),
            bios_firmware: "test".to_owned(),
            monitoring_provider: "test".to_owned(),
            background_process_audit: Vec::new(),
            operator_declaration: FormalEnvironmentDeclaration {
                vendor_performance_mode: "performance".to_owned(),
                bios_firmware: "test".to_owned(),
                sleep_or_session_lock_observed: false,
                thermal_or_power_throttling_observed: false,
            },
        };
        assert!(clear.invalidation_reasons(&environment).is_empty());
        let mut invalid = clear;
        invalid.background_cpu_time_ns.value = Some(CPU_INVALIDATION_THRESHOLD_NS + 1);
        invalid.background_write_bytes.value = Some(WRITE_INVALIDATION_THRESHOLD_BYTES + 1);
        invalid.monitoring_gap = true;
        assert_eq!(
            invalid.invalidation_reasons(&environment),
            vec![
                crate::InvalidationReason::BackgroundCpuOverOneSecond,
                crate::InvalidationReason::BackgroundWriteOver100Mib,
                crate::InvalidationReason::MonitoringGap,
            ]
        );
    }
}
