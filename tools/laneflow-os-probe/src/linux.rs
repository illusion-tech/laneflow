//! Linux 计数器读取：`/proc` 与 `/sys`，不含 `unsafe`。时钟中断每秒计数经
//! `rustix` 的安全接口在运行时取得，不假定为 100。

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use crate::proc_text::{
    parse_cpu_list_count, parse_pid_stat, parse_schedstat_wait, parse_stat_cpu, parse_status_bytes,
    ticks_to_ns,
};
use crate::{CpuTime, PowerStatus, ProcessMemory, SystemCpu};

/// 读取缓冲区大小。`/proc/stat` 只需首行，其余文件远小于此值。
const BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub(crate) struct State {
    buffer: Vec<u8>,
    ticks_per_second: u64,
}

impl State {
    pub fn new() -> Self {
        Self {
            buffer: vec![0; BUFFER_BYTES],
            ticks_per_second: rustix::param::clock_ticks_per_second(),
        }
    }

    /// 把文件读入预分配缓冲区。`prefix_only` 为真时允许文件超过缓冲区（只用
    /// 开头部分），否则超长视为错误。
    fn read(&mut self, path: &str, prefix_only: bool) -> io::Result<&str> {
        let mut file = File::open(path)?;
        let mut filled = 0;
        while filled < self.buffer.len() {
            let read = file.read(&mut self.buffer[filled..])?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        if filled == self.buffer.len() && !prefix_only {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "文件超过读取缓冲区",
            ));
        }
        std::str::from_utf8(&self.buffer[..filled])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "文件不是 UTF-8"))
    }

    fn ns(&self, ticks: u64) -> u64 {
        ticks_to_ns(ticks, self.ticks_per_second)
    }

    pub fn system_cpu(&mut self) -> io::Result<SystemCpu> {
        let logical = parse_cpu_list_count(self.read("/sys/devices/system/cpu/online", false)?)?;
        let stat = parse_stat_cpu(self.read("/proc/stat", true)?)?;
        Ok(SystemCpu {
            logical_cpus: Some(logical),
            busy_ns: Some(self.ns(stat.busy())),
            idle_ns: Some(self.ns(stat.idle)),
            linux_iowait_ns: Some(self.ns(stat.iowait)),
        })
    }

    fn cpu_time(&mut self, path: &str) -> io::Result<CpuTime> {
        let stat = parse_pid_stat(self.read(path, false)?)?;
        Ok(CpuTime {
            user_ns: Some(self.ns(stat.utime)),
            kernel_ns: Some(self.ns(stat.stime)),
        })
    }

    pub fn process_cpu(&mut self) -> io::Result<CpuTime> {
        self.cpu_time("/proc/self/stat")
    }

    pub fn thread_cpu(&mut self) -> io::Result<CpuTime> {
        self.cpu_time("/proc/thread-self/stat")
    }

    pub fn memory(&mut self) -> io::Result<ProcessMemory> {
        let stat = parse_pid_stat(self.read("/proc/self/stat", false)?)?;
        let status = self.read("/proc/self/status", false)?;
        Ok(ProcessMemory {
            resident_bytes: parse_status_bytes(status, "VmRSS")?,
            peak_resident_bytes: parse_status_bytes(status, "VmHWM")?,
            windows_private_bytes: None,
            windows_commit_bytes: None,
            windows_peak_commit_bytes: None,
            linux_anonymous_resident_bytes: parse_status_bytes(status, "RssAnon")?,
            page_faults: Some(stat.minflt + stat.majflt),
            linux_major_page_faults: Some(stat.majflt),
        })
    }

    /// 各线程 `schedstat` 第二列之和。读取期间退出的线程（目录项已列出但文件
    /// 不存在）不计入。
    pub fn run_queue_wait_ns(&mut self) -> io::Result<u64> {
        let mut total = 0_u64;
        for entry in fs::read_dir("/proc/self/task")? {
            let path = entry?.path().join("schedstat");
            let text = match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            total = total.saturating_add(parse_schedstat_wait(&text)?);
        }
        Ok(total)
    }
}

/// 读取 `/sys/class/power_supply`。目录不存在时全部字段不可用。
pub(crate) fn power() -> io::Result<PowerStatus> {
    let root = Path::new("/sys/class/power_supply");
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(PowerStatus::default());
        }
        Err(error) => return Err(error),
    };
    let mut status = PowerStatus {
        ac_online: None,
        battery_present: Some(false),
        battery_percent: None,
    };
    for entry in entries {
        let supply = entry?.path();
        let kind = match fs::read_to_string(supply.join("type")) {
            Ok(kind) => kind,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        match kind.trim() {
            "Mains" => {
                if let Ok(online) = fs::read_to_string(supply.join("online")) {
                    let online = online.trim() == "1";
                    status.ac_online = Some(status.ac_online.unwrap_or(false) || online);
                }
            }
            "Battery" => {
                status.battery_present = Some(true);
                if status.battery_percent.is_none() {
                    status.battery_percent = fs::read_to_string(supply.join("capacity"))
                        .ok()
                        .and_then(|capacity| capacity.trim().parse().ok());
                }
            }
            _ => {}
        }
    }
    Ok(status)
}
