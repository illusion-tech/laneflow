//! 子进程观测（`host-observe.md` §5）。
//!
//! 调用方用标准库启动子进程后交给 [`ChildObserver`]。观测范围为仅直接子进程，
//! 或直接子进程及其全部后代。成员以 PID 加启动时刻识别：某 PID 的启动时刻变化
//! 时旧成员移出；父进程是已知成员、且启动时刻不早于父进程的进程加入成员；成员
//! 重新挂靠到其他父进程后仍按原身份追踪。
//!
//! 采样峰值与采样 CPU 时间都是下界：两次采样之间的峰值和最后一次采样后的 CPU
//! 消耗无法观测，进程被回收后操作系统也不再提供其计数。

use std::{
    collections::BTreeMap,
    io,
    process::{Child, ExitStatus},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessStatus, ProcessesToUpdate, System};

/// 观测器被丢弃时终止进程组的期限。
pub const DROP_TERMINATION_DEADLINE: Duration = Duration::from_secs(3);

/// 汇总中对采样值口径的说明。
pub const SAMPLED_BOUNDS: &str = "sampled lower bounds: peaks between samples and CPU time after the last sample are not observed";

/// 观测范围。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// 每次采样只刷新直接子进程（与观测器自身），开销与本机进程数无关。
    DirectChild,
    /// 直接子进程及其全部后代，每次采样扫描整个进程表。
    ProcessGroup,
}

/// 一次采样中一个进程的读数。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemberSample {
    /// 进程号。
    pub pid: u32,
    /// 启动时刻（Unix 秒）。
    pub start_time_s: u64,
    /// 父进程号。
    pub parent_pid: Option<u32>,
    /// 驻留字节；读到 0 时为 `None`，不记为 0。
    pub resident_bytes: Option<u64>,
    /// 累计 CPU 毫秒。
    pub cpu_ms: u64,
    /// 是否为已退出未回收（Linux 僵尸进程）。Windows 恒为 `false`。
    pub exited_unreaped: bool,
}

/// 一次采样。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    /// 仍在进程表中的成员，按 PID 升序。
    pub members: Vec<MemberSample>,
    /// 观测器自身进程的读数。
    pub observer: Option<MemberSample>,
}

/// 子进程退出状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExitRecord {
    /// 是否成功退出。
    pub success: bool,
    /// 退出码；Unix 上被信号终止时为 `None`。
    pub code: Option<i32>,
    /// Unix 上终止进程的信号编号。
    pub signal: Option<i32>,
}

impl From<ExitStatus> for ExitRecord {
    fn from(status: ExitStatus) -> Self {
        #[cfg(unix)]
        let signal = std::os::unix::process::ExitStatusExt::signal(&status);
        #[cfg(not(unix))]
        let signal = None;
        Self {
            success: status.success(),
            code: status.code(),
            signal,
        }
    }
}

/// 子进程退出后的汇总。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildSummary {
    /// 观测范围。
    pub scope: Scope,
    /// 退出状态。
    pub exit: ExitRecord,
    /// 采样次数。
    pub sample_count: u64,
    /// 采样期间直接子进程驻留字节的最大值（下界）。
    pub sampled_peak_resident_bytes: Option<u64>,
    /// 最后一次采样的直接子进程累计 CPU 毫秒（下界）。
    pub sampled_cpu_ms: Option<u64>,
    /// 采样值口径，恒为 [`SAMPLED_BOUNDS`]。
    pub bounds: String,
}

/// 子进程观测器。被丢弃时按身份终止进程组。
pub struct ChildObserver {
    child: Child,
    scope: Scope,
    system: System,
    known: BTreeMap<Pid, u64>,
    root: Pid,
    root_start: Option<u64>,
    observer: Pid,
    finished: bool,
    exit: Option<ExitStatus>,
    sample_count: u64,
    peak: Option<u64>,
    cpu_ms: Option<u64>,
}

impl ChildObserver {
    /// 接管已启动的子进程。
    ///
    /// # Errors
    ///
    /// 无法初始化进程表读取时返回错误；此时子进程已被终止并回收。
    pub fn new(mut child: Child, scope: Scope) -> io::Result<Self> {
        let system = match System::new() {
            Ok(system) => system,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::other(error));
            }
        };
        let root = Pid::from_u32(child.id());
        let mut observer = Self {
            child,
            scope,
            system,
            known: BTreeMap::new(),
            root,
            root_start: None,
            observer: Pid::from_u32(std::process::id()),
            finished: false,
            exit: None,
            sample_count: 0,
            peak: None,
            cpu_ms: None,
        };
        observer.refresh();
        Ok(observer)
    }

    /// 直接子进程的 PID。
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// 观测范围。
    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// 直接子进程，供调用方读写其标准输入输出。
    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    /// 非阻塞查询直接子进程是否已退出；退出后记录退出状态。
    ///
    /// # Errors
    ///
    /// 查询失败时返回错误。
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if let Some(status) = self.exit {
            return Ok(Some(status));
        }
        let status = self.child.try_wait()?;
        self.exit = status;
        Ok(status)
    }

    fn refresh(&mut self) {
        let kind = crate::snapshot::refresh_kind();
        match self.scope {
            Scope::ProcessGroup => {
                self.system
                    .refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
            }
            Scope::DirectChild => {
                self.system.refresh_processes_specifics(
                    ProcessesToUpdate::Some(&[self.root, self.observer]),
                    true,
                    kind,
                );
            }
        }
        self.known.retain(|pid, start| {
            self.system
                .process(*pid)
                .is_some_and(|process| process.start_time() == *start)
        });
        if let Some(process) = self.system.process(self.root) {
            let expected = *self.root_start.get_or_insert(process.start_time());
            if expected == process.start_time() {
                self.known.entry(self.root).or_insert(expected);
            }
        }
        if self.scope == Scope::DirectChild {
            return;
        }
        loop {
            let before = self.known.len();
            for (pid, process) in self.system.processes() {
                if let Some(parent) = process.parent().and_then(|pid| self.known.get(&pid))
                    && process.start_time() >= *parent
                {
                    self.known.entry(*pid).or_insert(process.start_time());
                }
            }
            if self.known.len() == before {
                break;
            }
        }
    }

    fn member(&self, pid: Pid) -> Option<MemberSample> {
        self.system.process(pid).map(|process| MemberSample {
            pid: pid.as_u32(),
            start_time_s: process.start_time(),
            parent_pid: process.parent().map(Pid::as_u32),
            resident_bytes: (process.memory() > 0).then_some(process.memory()),
            cpu_ms: process.accumulated_cpu_time(),
            exited_unreaped: process.status() == ProcessStatus::Zombie,
        })
    }

    /// 采样一次：刷新进程表，更新成员，返回各成员与观测器自身的读数，并更新
    /// 直接子进程的采样峰值与采样 CPU 时间。
    pub fn sample(&mut self) -> Sample {
        self.refresh();
        let members: Vec<MemberSample> = self
            .known
            .keys()
            .filter_map(|pid| self.member(*pid))
            .collect();
        if let Some(root) = members
            .iter()
            .find(|member| Pid::from_u32(member.pid) == self.root)
        {
            if let Some(bytes) = root.resident_bytes {
                self.peak = Some(self.peak.map_or(bytes, |peak| peak.max(bytes)));
            }
            self.cpu_ms = Some(self.cpu_ms.map_or(root.cpu_ms, |cpu| cpu.max(root.cpu_ms)));
        }
        self.sample_count += 1;
        Sample {
            observer: self.member(self.observer),
            members,
        }
    }

    /// 直接子进程退出后的汇总；尚未观察到退出时为 `None`。
    pub fn summary(&self) -> Option<ChildSummary> {
        self.exit.map(|status| ChildSummary {
            scope: self.scope,
            exit: status.into(),
            sample_count: self.sample_count,
            sampled_peak_resident_bytes: self.peak,
            sampled_cpu_ms: self.cpu_ms,
            bounds: SAMPLED_BOUNDS.into(),
        })
    }

    /// 按身份终止进程组：先终止直接子进程以阻止继续派生，再逐个终止 PID 与启动
    /// 时刻仍匹配的已知成员，直到直接子进程被回收且其余成员都已退出。
    ///
    /// # Errors
    ///
    /// 超过 `deadline` 仍未确认完成，或查询失败时返回错误，不报告成功。
    pub fn terminate(&mut self, deadline: Duration) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        self.refresh();
        let _ = self.child.kill();
        let deadline = Instant::now() + deadline;
        loop {
            for pid in self.known.keys() {
                if let Some(process) = self.system.process(*pid) {
                    let _ = process.kill();
                }
            }
            let reaped = self.try_wait()?.is_some();
            self.refresh();
            let remaining = self.known.keys().any(|pid| {
                self.system
                    .process(*pid)
                    .is_some_and(|process| process.status() != ProcessStatus::Zombie)
            });
            if reaped && !remaining {
                self.finished = true;
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "进程组终止未在期限内确认完成",
                ));
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for ChildObserver {
    fn drop(&mut self) {
        let _ = self.terminate(DROP_TERMINATION_DEADLINE);
    }
}
