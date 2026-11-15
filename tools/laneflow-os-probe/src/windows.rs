//! Windows 计数器读取：workspace 内登记的受审计 `unsafe` 例外
//! （`docs/design/os-probe.md` §4）。
//!
//! 本文件恰有 5 处 `unsafe`，分别对应整机 CPU、本进程 CPU、当前线程 CPU、
//! 本进程内存与电源状态；表达式由 check-wire-audit 逐处比对。五处共享同一
//! 安全论证：
//!
//! 1. 输出参数全部是本函数栈上按零初始化、类型与 `windows-sys` 声明一致的
//!    `FILETIME`、`PROCESS_MEMORY_COUNTERS_EX` 或 `SYSTEM_POWER_STATUS`；
//!    需要声明大小的调用传入该结构体的精确字节数。
//! 2. 被调函数只在调用期间写入这些位置，返回后不保留指针，没有回调。
//! 3. `GetCurrentProcess` 与 `GetCurrentThread` 返回伪句柄，不需要也不得关闭，
//!    不传出本函数。
//! 4. 返回值为失败时立即读取 `GetLastError` 并返回错误，不读取输出参数。
//! 5. 不涉及共享可变状态；当前线程字段只描述调用线程。
//!
//! `GetActiveProcessorGroupCount` 与 `GetActiveProcessorCount` 不接收指针，
//! 只返回整数。

use std::io;
use std::mem::size_of;

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows_sys::Win32::System::ProcessStatus::{
    K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows_sys::Win32::System::Threading::{
    ALL_PROCESSOR_GROUPS, GetActiveProcessorCount, GetActiveProcessorGroupCount, GetCurrentProcess,
    GetCurrentThread, GetProcessTimes, GetSystemTimes, GetThreadTimes,
};

use crate::{
    CpuTime, PowerStatus, ProcessMemory, SystemCpu, decode_windows_power, hundred_ns_to_ns,
    system_times_cover_machine,
};

const ZERO_FILETIME: FILETIME = FILETIME {
    dwLowDateTime: 0,
    dwHighDateTime: 0,
};

fn ns(time: FILETIME) -> u64 {
    hundred_ns_to_ns(time.dwLowDateTime, time.dwHighDateTime)
}

fn check(ok: i32) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(crate) fn system_cpu() -> io::Result<SystemCpu> {
    let (mut idle, mut kernel, mut user) = (ZERO_FILETIME, ZERO_FILETIME, ZERO_FILETIME);
    // SAFETY: 见模块文档第 1、2、4、5 条；前两个调用无指针参数。
    let (groups, logical, ok) = unsafe {
        (
            GetActiveProcessorGroupCount(),
            GetActiveProcessorCount(ALL_PROCESSOR_GROUPS),
            GetSystemTimes(&mut idle, &mut kernel, &mut user),
        )
    };
    check(ok)?;
    let covered = system_times_cover_machine(groups);
    let idle_ns = ns(idle);
    // 内核时间包含空闲时间。
    let busy_ns = ns(kernel).saturating_sub(idle_ns).saturating_add(ns(user));
    Ok(SystemCpu {
        logical_cpus: (logical != 0).then_some(logical),
        busy_ns: covered.then_some(busy_ns),
        idle_ns: covered.then_some(idle_ns),
        linux_iowait_ns: None,
    })
}

pub(crate) fn process_cpu() -> io::Result<CpuTime> {
    let (mut creation, mut exit) = (ZERO_FILETIME, ZERO_FILETIME);
    let (mut kernel, mut user) = (ZERO_FILETIME, ZERO_FILETIME);
    // SAFETY: 见模块文档第 1–5 条。
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    check(ok)?;
    Ok(CpuTime {
        user_ns: Some(ns(user)),
        kernel_ns: Some(ns(kernel)),
    })
}

pub(crate) fn thread_cpu() -> io::Result<CpuTime> {
    let (mut creation, mut exit) = (ZERO_FILETIME, ZERO_FILETIME);
    let (mut kernel, mut user) = (ZERO_FILETIME, ZERO_FILETIME);
    // SAFETY: 见模块文档第 1–5 条。
    let ok = unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    check(ok)?;
    Ok(CpuTime {
        user_ns: Some(ns(user)),
        kernel_ns: Some(ns(kernel)),
    })
}

/// 返回内存字段与未展开的 32 位缺页计数；回绕展开由调用方持有的状态完成。
pub(crate) fn memory() -> io::Result<(ProcessMemory, u32)> {
    const SIZE: u32 = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: SIZE,
        PageFaultCount: 0,
        PeakWorkingSetSize: 0,
        WorkingSetSize: 0,
        QuotaPeakPagedPoolUsage: 0,
        QuotaPagedPoolUsage: 0,
        QuotaPeakNonPagedPoolUsage: 0,
        QuotaNonPagedPoolUsage: 0,
        PagefileUsage: 0,
        PeakPagefileUsage: 0,
        PrivateUsage: 0,
    };
    let pointer = (&raw mut counters).cast::<PROCESS_MEMORY_COUNTERS>();
    // SAFETY: 见模块文档第 1–5 条；`SIZE` 为 `PROCESS_MEMORY_COUNTERS_EX` 的
    // 精确字节数，函数按 `cb` 写入扩展结构体。
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), pointer, SIZE) };
    check(ok)?;
    let bytes = |value: usize| Some(value as u64);
    Ok((
        ProcessMemory {
            resident_bytes: bytes(counters.WorkingSetSize),
            peak_resident_bytes: bytes(counters.PeakWorkingSetSize),
            windows_private_bytes: bytes(counters.PrivateUsage),
            windows_commit_bytes: bytes(counters.PagefileUsage),
            windows_peak_commit_bytes: bytes(counters.PeakPagefileUsage),
            linux_anonymous_resident_bytes: None,
            page_faults: None,
            linux_major_page_faults: None,
        },
        counters.PageFaultCount,
    ))
}

pub(crate) fn power() -> io::Result<PowerStatus> {
    let mut status = SYSTEM_POWER_STATUS {
        ACLineStatus: 0,
        BatteryFlag: 0,
        BatteryLifePercent: 0,
        SystemStatusFlag: 0,
        BatteryLifeTime: 0,
        BatteryFullLifeTime: 0,
    };
    // SAFETY: 见模块文档第 1、2、4、5 条。
    let ok = unsafe { GetSystemPowerStatus(&mut status) };
    check(ok)?;
    Ok(decode_windows_power(
        status.ACLineStatus,
        status.BatteryFlag,
        status.BatteryLifePercent,
    ))
}
