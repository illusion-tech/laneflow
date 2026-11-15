//! 本进程与整机操作系统计数器读取（`docs/design/os-probe.md`）。
//!
//! 只返回原始值：时间为纳秒，字节与计数为整数，不返回百分比或速率。平台不
//! 提供的字段为 `None`，不填零；两个平台语义不同的字段分别命名。字段分为
//! 单调累计、可减累计与瞬时值三类，见各字段文档；可减累计原样返回，调用方
//! 取差值为负时应把该区间的派生量记为不可用。
//!
//! 只观测本进程与整机，不经服务进程（不用 WMI、PDH、ETW）。[`Probe`] 构造时
//! 分配缓冲区；之后 [`Probe::system_cpu`]、[`Probe::process_cpu`]、
//! [`Probe::thread_cpu`] 与 [`Probe::memory`] 不分配堆内存，可在帧间调用。
//! [`Probe::run_queue_wait_ns`] 与 [`Probe::power`] 需要枚举目录，只适合低频
//! 调用。
//!
//! `unsafe` 只出现在 Windows 模块（`windows.rs`）的 5 处固定调用，安全论证
//! 见该模块文档；本文件与 Linux 模块不含 `unsafe`。

#![deny(missing_docs)]

use std::io;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "linux", test))]
mod proc_text;
#[cfg(windows)]
mod windows;

#[cfg(test)]
mod tests;

/// 整机 CPU 时间（全部逻辑 CPU 之和）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SystemCpu {
    /// 全部处理器组的活动逻辑 CPU 数（瞬时值）。
    pub logical_cpus: Option<u32>,
    /// 整机忙碌纳秒（单调累计）。Windows 活动处理器组多于一个时不可用。
    pub busy_ns: Option<u64>,
    /// 整机空闲纳秒（单调累计），Linux 不含 iowait。Windows 活动处理器组多于
    /// 一个时不可用。
    pub idle_ns: Option<u64>,
    /// Linux 整机 iowait 纳秒（可减累计）。
    pub linux_iowait_ns: Option<u64>,
}

/// 本进程或当前线程的 CPU 时间（单调累计）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTime {
    /// 用户态纳秒。
    pub user_ns: Option<u64>,
    /// 内核态纳秒。
    pub kernel_ns: Option<u64>,
}

/// 本进程内存与缺页计数。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcessMemory {
    /// 驻留字节（瞬时值）：Windows `WorkingSetSize`，Linux `VmRSS`。
    pub resident_bytes: Option<u64>,
    /// 驻留峰值字节（单调累计，进程生命周期峰值）：Windows
    /// `PeakWorkingSetSize`，Linux `VmHWM`。
    pub peak_resident_bytes: Option<u64>,
    /// Windows private 字节（瞬时值）：`PrivateUsage`。
    pub windows_private_bytes: Option<u64>,
    /// Windows commit 字节（瞬时值）：`PagefileUsage`。
    pub windows_commit_bytes: Option<u64>,
    /// Windows commit 峰值字节（单调累计）：`PeakPagefileUsage`。
    pub windows_peak_commit_bytes: Option<u64>,
    /// Linux 匿名驻留字节（瞬时值）：`RssAnon`。
    pub linux_anonymous_resident_bytes: Option<u64>,
    /// 缺页总数（单调累计）：Windows `PageFaultCount`（含软缺页，32 位回绕已
    /// 展开为 64 位），Linux minflt + majflt。
    pub page_faults: Option<u64>,
    /// Linux 主缺页数（单调累计）：majflt。
    pub linux_major_page_faults: Option<u64>,
}

/// 交流电与电池状态（瞬时值）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PowerStatus {
    /// 交流电是否接通。
    pub ac_online: Option<bool>,
    /// 是否存在电池。
    pub battery_present: Option<bool>,
    /// 电池电量百分比。
    pub battery_percent: Option<u8>,
}

/// 读取器：持有构造时分配的缓冲区与缺页回绕展开状态。
#[derive(Debug)]
pub struct Probe {
    #[cfg(target_os = "linux")]
    linux: linux::State,
    #[cfg(windows)]
    page_faults: PageFaultCounter,
}

impl Probe {
    /// 构造读取器并分配所需缓冲区。
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            #[cfg(target_os = "linux")]
            linux: linux::State::new(),
            #[cfg(windows)]
            page_faults: PageFaultCounter::default(),
        })
    }

    /// 读取整机 CPU 时间。
    pub fn system_cpu(&mut self) -> io::Result<SystemCpu> {
        #[cfg(windows)]
        {
            windows::system_cpu()
        }
        #[cfg(target_os = "linux")]
        {
            self.linux.system_cpu()
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            Ok(SystemCpu::default())
        }
    }

    /// 读取本进程 CPU 时间。
    pub fn process_cpu(&mut self) -> io::Result<CpuTime> {
        #[cfg(windows)]
        {
            windows::process_cpu()
        }
        #[cfg(target_os = "linux")]
        {
            self.linux.process_cpu()
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            Ok(CpuTime::default())
        }
    }

    /// 读取调用线程的 CPU 时间。
    pub fn thread_cpu(&mut self) -> io::Result<CpuTime> {
        #[cfg(windows)]
        {
            windows::thread_cpu()
        }
        #[cfg(target_os = "linux")]
        {
            self.linux.thread_cpu()
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            Ok(CpuTime::default())
        }
    }

    /// 读取本进程内存与缺页计数。
    pub fn memory(&mut self) -> io::Result<ProcessMemory> {
        #[cfg(windows)]
        {
            let (mut memory, raw_faults) = windows::memory()?;
            memory.page_faults = Some(self.page_faults.extend(raw_faults));
            Ok(memory)
        }
        #[cfg(target_os = "linux")]
        {
            self.linux.memory()
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            Ok(ProcessMemory::default())
        }
    }

    /// 读取本进程全部线程的就绪等待纳秒之和（可减累计，仅 Linux）。
    ///
    /// 两次读取之间有线程退出时，和可能减少；读取期间退出的线程不计入。
    /// 需要枚举目录，可能分配。
    pub fn run_queue_wait_ns(&mut self) -> io::Result<Option<u64>> {
        #[cfg(target_os = "linux")]
        {
            self.linux.run_queue_wait_ns().map(Some)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(None)
        }
    }

    /// 读取交流电与电池状态。需要枚举目录（Linux），可能分配。
    pub fn power(&mut self) -> io::Result<PowerStatus> {
        #[cfg(windows)]
        {
            windows::power()
        }
        #[cfg(target_os = "linux")]
        {
            linux::power()
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            Ok(PowerStatus::default())
        }
    }
}

/// 把 32 位回绕计数展开为 64 位累计值；要求两次读取之间的增量小于 2³²。
#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, Default)]
struct PageFaultCounter {
    last: Option<u32>,
    total: u64,
}

#[cfg(any(windows, test))]
impl PageFaultCounter {
    fn extend(&mut self, raw: u32) -> u64 {
        match self.last {
            None => self.total = u64::from(raw),
            Some(last) => self.total += u64::from(raw.wrapping_sub(last)),
        }
        self.last = Some(raw);
        self.total
    }
}

/// Windows `FILETIME`（100 ns 单位）换算为纳秒。
#[cfg(any(windows, test))]
fn hundred_ns_to_ns(low: u32, high: u32) -> u64 {
    ((u64::from(high) << 32) | u64::from(low)).saturating_mul(100)
}

/// 按活动处理器组数决定 `GetSystemTimes` 结果是否覆盖整机。
#[cfg(any(windows, test))]
fn system_times_cover_machine(active_groups: u16) -> bool {
    active_groups == 1
}

/// 解读 Windows `SYSTEM_POWER_STATUS` 的三个字段（os-probe.md §3.5）。
#[cfg(any(windows, test))]
fn decode_windows_power(ac_line: u8, battery_flag: u8, battery_percent: u8) -> PowerStatus {
    const UNKNOWN: u8 = 255;
    const NO_BATTERY: u8 = 128;
    let ac_online = match ac_line {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    };
    if battery_flag == UNKNOWN {
        return PowerStatus {
            ac_online,
            battery_present: None,
            battery_percent: None,
        };
    }
    let battery_present = battery_flag & NO_BATTERY == 0;
    PowerStatus {
        ac_online,
        battery_present: Some(battery_present),
        battery_percent: (battery_present && battery_percent != UNKNOWN).then_some(battery_percent),
    }
}
