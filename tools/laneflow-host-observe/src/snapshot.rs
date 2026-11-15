//! 进程快照（`host-observe.md` §4）：一次快照内部读取进程表两次，报告每个进程
//! 的区间 CPU 时间增量与第二次读数的驻留字节。
//!
//! 不记录命令行、完整路径、环境变量与用户名：快照会写入运行目录并可能随证据
//! 发布，这些内容可能含本机路径或敏感信息。

use std::{collections::BTreeMap, io, thread, time::Duration};

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

/// 默认读数间隔。
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(1);

/// 一次进程快照。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessSnapshot {
    /// 第二次读数的 UTC 时刻。
    pub captured_at: String,
    /// 两次读数的间隔毫秒数。
    pub interval_ms: u64,
    /// 累计 CPU 时间的粒度说明。
    pub cpu_time_unit: String,
    /// 第二次读数中的全部进程，按 PID 升序；不过滤任何进程。
    pub processes: Vec<ProcessEntry>,
}

/// 快照中的一个进程。进程以 PID 加启动时刻识别。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessEntry {
    /// 进程号。
    pub pid: u32,
    /// 启动时刻（Unix 秒）。
    pub start_time_s: u64,
    /// 父进程号。
    pub parent_pid: Option<u32>,
    /// 可执行文件名（不含路径）。
    pub name: String,
    /// 操作系统报告的进程状态文字。
    pub status: String,
    /// 是否为已退出未回收（Linux 僵尸进程）。
    pub exited_unreaped: bool,
    /// 第一次读数的累计 CPU 毫秒；该进程不在第一次读数中，或启动时刻不同时为 `None`。
    pub cpu_ms_first: Option<u64>,
    /// 第二次读数的累计 CPU 毫秒。
    pub cpu_ms_second: u64,
    /// 区间 CPU 毫秒增量；`cpu_ms_first` 为 `None` 或计数减少时为 `None`。
    pub cpu_delta_ms: Option<u64>,
    /// 第二次读数的驻留字节；读到 0（权限不足或已退出）时为 `None`。
    pub resident_bytes: Option<u64>,
}

/// 一次进程表读数中单个进程的原始值。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reading {
    pub pid: u32,
    pub start_time_s: u64,
    pub parent_pid: Option<u32>,
    pub name: String,
    pub status: String,
    pub exited_unreaped: bool,
    pub cpu_ms: u64,
    pub resident_bytes: u64,
}

pub(crate) fn refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_cpu()
        .with_memory()
        .without_tasks()
}

pub(crate) fn reading(pid: Pid, process: &sysinfo::Process) -> Reading {
    Reading {
        pid: pid.as_u32(),
        start_time_s: process.start_time(),
        parent_pid: process.parent().map(Pid::as_u32),
        name: process.name().to_string_lossy().into_owned(),
        status: process.status().to_string(),
        exited_unreaped: process.status() == ProcessStatus::Zombie,
        cpu_ms: process.accumulated_cpu_time(),
        resident_bytes: process.memory(),
    }
}

fn read_all(system: &mut System) -> Vec<Reading> {
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind());
    system
        .processes()
        .iter()
        .map(|(pid, process)| reading(*pid, process))
        .collect()
}

/// 采集一次进程快照：读取进程表，等待 `interval`，再读取一次。
///
/// # Errors
///
/// 无法初始化进程表读取时返回错误。
pub fn capture(interval: Duration) -> io::Result<ProcessSnapshot> {
    let mut system = System::new().map_err(io::Error::other)?;
    let first = read_all(&mut system);
    thread::sleep(interval);
    let second = read_all(&mut system);
    Ok(build(
        &first,
        second,
        crate::time::now_utc(),
        u64::try_from(interval.as_millis()).unwrap_or(u64::MAX),
    ))
}

/// 由两次读数构造快照。只出现在一次读数中，或两次启动时刻不同的进程，区间增量
/// 为 `None`。
pub(crate) fn build(
    first: &[Reading],
    second: Vec<Reading>,
    captured_at: String,
    interval_ms: u64,
) -> ProcessSnapshot {
    let earlier: BTreeMap<(u32, u64), u64> = first
        .iter()
        .map(|reading| ((reading.pid, reading.start_time_s), reading.cpu_ms))
        .collect();
    let mut processes: Vec<ProcessEntry> = second
        .into_iter()
        .map(|reading| {
            let cpu_ms_first = earlier.get(&(reading.pid, reading.start_time_s)).copied();
            ProcessEntry {
                pid: reading.pid,
                start_time_s: reading.start_time_s,
                parent_pid: reading.parent_pid,
                name: reading.name,
                status: reading.status,
                exited_unreaped: reading.exited_unreaped,
                cpu_ms_first,
                cpu_ms_second: reading.cpu_ms,
                cpu_delta_ms: cpu_ms_first.and_then(|first| reading.cpu_ms.checked_sub(first)),
                resident_bytes: (reading.resident_bytes > 0).then_some(reading.resident_bytes),
            }
        })
        .collect();
    processes.sort_by_key(|entry| (entry.pid, entry.start_time_s));
    ProcessSnapshot {
        captured_at,
        interval_ms,
        cpu_time_unit: "accumulated CPU milliseconds as reported by sysinfo".into(),
        processes,
    }
}
