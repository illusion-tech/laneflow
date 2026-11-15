//! Linux `/proc` 与 `/sys` 文本解析。纯函数、与平台无关，各平台都编译并测试；
//! 只有 Linux 模块在运行时调用。成功路径不分配堆内存。
//!
//! 输入按字节处理：进程与线程名 `comm` 是任意字节序列，内核 15 字节截断还可能
//! 切开多字节 UTF-8 字符，因此不对整份文件做 UTF-8 校验，只解码需要的数字片段。

use std::io;

/// Linux `ESRCH`（各架构均为 3）：线程在 procfs 文件打开后、读取完成前退出时
/// 返回。Rust 把它归为 `Uncategorized`，需按错误码识别。
const ESRCH: i32 = 3;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn ascii<'a>(bytes: &'a [u8], message: &'static str) -> io::Result<&'a str> {
    std::str::from_utf8(bytes).map_err(|_| invalid(message))
}

fn number(field: Option<&str>, message: &'static str) -> io::Result<u64> {
    field
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| invalid(message))
}

/// 线程在枚举后、读取完成前消失：打开时 `ENOENT`，读取中 `ESRCH`。
pub(crate) fn is_thread_gone(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound || error.raw_os_error() == Some(ESRCH)
}

/// `/proc/stat` 汇总行的各项累计时钟中断数。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct StatCpu {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
}

impl StatCpu {
    /// 整机忙碌时钟中断数；guest 时间已计入 user，不重复累加。
    pub fn busy(self) -> u64 {
        self.user + self.nice + self.system + self.irq + self.softirq + self.steal
    }
}

/// 解析 `/proc/stat` 的首行汇总行（`cpu  user nice system idle iowait irq softirq steal …`）。
pub(crate) fn parse_stat_cpu(bytes: &[u8]) -> io::Result<StatCpu> {
    let line = bytes
        .split(|&byte| byte == b'\n')
        .next()
        .filter(|line| !line.is_empty())
        .ok_or_else(|| invalid("/proc/stat 为空"))?;
    let line = ascii(line, "/proc/stat 首行不是 ASCII")?;
    let mut fields = line.split_ascii_whitespace();
    if fields.next() != Some("cpu") {
        return Err(invalid("/proc/stat 首行不是 cpu 汇总行"));
    }
    let mut next = || number(fields.next(), "/proc/stat 汇总行字段缺失或格式异常");
    Ok(StatCpu {
        user: next()?,
        nice: next()?,
        system: next()?,
        idle: next()?,
        iowait: next()?,
        irq: next()?,
        softirq: next()?,
        steal: next()?,
    })
}

/// `/proc/<pid>/stat` 或 `/proc/thread-self/stat` 中本设计使用的字段。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PidStat {
    pub minflt: u64,
    pub majflt: u64,
    pub utime: u64,
    pub stime: u64,
}

/// 解析 `stat` 行。进程名 `comm` 可含空格、括号与非 UTF-8 字节，因此在字节
/// 层面定位最后一个 `)`，只解码其后的数字部分：之后第 1 项为第 3 字段 state。
pub(crate) fn parse_pid_stat(bytes: &[u8]) -> io::Result<PidStat> {
    let close = bytes
        .iter()
        .rposition(|&byte| byte == b')')
        .ok_or_else(|| invalid("stat 缺少进程名结束括号"))?;
    let rest = ascii(&bytes[close + 1..], "stat 数字字段不是 ASCII")?;
    let mut fields = rest.split_ascii_whitespace();
    // 字段 n（从 1 计）位于 `)` 之后第 n − 2 项（从 1 计）。
    let mut field = |skip: usize| number(fields.nth(skip), "stat 字段缺失或格式异常");
    let minflt = field(7)?; // 第 10 字段
    let majflt = field(1)?; // 第 12 字段
    let utime = field(1)?; // 第 14 字段
    let stime = field(0)?; // 第 15 字段
    Ok(PidStat {
        minflt,
        majflt,
        utime,
        stime,
    })
}

/// 从 `/proc/self/status` 读取以 kB 计的字段（如 `VmRSS`），换算为字节；
/// 字段不存在时返回 `None`。逐行按字节匹配键名，`Name:` 等行的非 UTF-8
/// 内容不影响结果。
pub(crate) fn parse_status_bytes(bytes: &[u8], key: &str) -> io::Result<Option<u64>> {
    for line in bytes.split(|&byte| byte == b'\n') {
        let Some(rest) = line.strip_prefix(key.as_bytes()) else {
            continue;
        };
        let Some(rest) = rest.strip_prefix(b":") else {
            continue;
        };
        let rest = ascii(rest, "status 字段值不是 ASCII")?;
        let mut fields = rest.split_ascii_whitespace();
        let kib = number(fields.next(), "status 字段数值格式异常")?;
        if fields.next() != Some("kB") {
            return Err(invalid("status 字段单位不是 kB"));
        }
        return kib
            .checked_mul(1024)
            .map(Some)
            .ok_or_else(|| invalid("status 字段数值溢出"));
    }
    Ok(None)
}

/// 解析 `schedstat` 第二列：可运行但未获得 CPU 的累计纳秒。
pub(crate) fn parse_schedstat_wait(bytes: &[u8]) -> io::Result<u64> {
    let text = ascii(bytes, "schedstat 不是 ASCII")?;
    let mut fields = text.split_ascii_whitespace();
    let _ = fields.next().ok_or_else(|| invalid("schedstat 为空"))?;
    number(fields.next(), "schedstat 第二列缺失或格式异常")
}

/// 解析 CPU 列表（如 `/sys/devices/system/cpu/online` 的 `0-15,32-47`）的 CPU 数。
pub(crate) fn parse_cpu_list_count(bytes: &[u8]) -> io::Result<u32> {
    let text = ascii(bytes, "CPU 列表不是 ASCII")?.trim();
    if text.is_empty() {
        return Err(invalid("CPU 列表为空"));
    }
    let mut count = 0_u32;
    for range in text.split(',') {
        let (first, last) = match range.split_once('-') {
            Some((first, last)) => (first, last),
            None => (range, range),
        };
        let first: u32 = first.parse().map_err(|_| invalid("CPU 列表格式异常"))?;
        let last: u32 = last.parse().map_err(|_| invalid("CPU 列表格式异常"))?;
        if last < first {
            return Err(invalid("CPU 列表区间倒置"));
        }
        count = count
            .checked_add(last - first + 1)
            .ok_or_else(|| invalid("CPU 列表计数溢出"))?;
    }
    Ok(count)
}

/// 时钟中断数换算为纳秒。
pub(crate) fn ticks_to_ns(ticks: u64, ticks_per_second: u64) -> u64 {
    let ns = u128::from(ticks) * 1_000_000_000 / u128::from(ticks_per_second.max(1));
    u64::try_from(ns).unwrap_or(u64::MAX)
}
