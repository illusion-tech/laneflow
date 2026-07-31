//! 父协调进程对子进程的操作系统级存续边界。
//!
//! Windows 正式研究路径把每个一次性子进程放入独立的 kill-on-close Job Object。
//! 子进程在完成分配与启动信号握手前不会进入受测管线，因此 Job 分配失败时仍可通过
//! 关闭 stdin 和普通终止请求失败关闭。非 Windows 平台只用于开发测试；正式私有字节
//! 监控会在护栏层拒绝这些平台。

use std::io;
use std::process::{Child, ChildStdin, Command, ExitStatus, Output};

#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(windows)]
use win32job::{ExtendedLimitInfo, Job};

#[derive(Debug)]
pub(crate) struct ContainedChild {
    #[cfg(windows)]
    containment: Option<Job>,
    child: Child,
}

impl ContainedChild {
    pub(crate) fn spawn(command: &mut Command) -> io::Result<Self> {
        #[cfg(windows)]
        {
            let mut limits = ExtendedLimitInfo::new();
            limits.limit_kill_on_job_close();
            let containment = Job::create_with_limit_info(&limits).map_err(io::Error::from)?;
            let mut child = command.spawn()?;
            if let Err(source) = containment.assign_process(child.as_raw_handle() as isize) {
                // 所有研究子进程都在 stdin 启动信号上等待。先关闭信号管道，再请求普通
                // 终止；即使请求失败，子进程也不能进入受测管线。
                drop(child.stdin.take());
                let _ = child.kill();
                return Err(io::Error::from(source));
            }
            Ok(Self {
                containment: Some(containment),
                child,
            })
        }

        #[cfg(not(windows))]
        {
            command.spawn().map(|child| Self { child })
        }
    }

    pub(crate) fn id(&self) -> u32 {
        self.child.id()
    }

    pub(crate) fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    pub(crate) fn request_termination(&mut self) -> io::Result<()> {
        self.child.kill()
    }

    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    pub(crate) fn escalate_containment(&mut self) -> io::Result<bool> {
        #[cfg(windows)]
        {
            let Some(containment) = self.containment.take() else {
                return Ok(false);
            };
            // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE guarantees that closing the final handle
            // terminates every process associated with this per-child Job Object.
            drop(containment);
            Ok(true)
        }

        #[cfg(not(windows))]
        {
            self.child.kill()?;
            Ok(false)
        }
    }

    pub(crate) fn wait_with_output(self) -> io::Result<Output> {
        #[cfg(windows)]
        let Self { containment, child } = self;
        #[cfg(not(windows))]
        let Self { child } = self;

        let output = child.wait_with_output();
        #[cfg(windows)]
        drop(containment);
        output
    }
}
