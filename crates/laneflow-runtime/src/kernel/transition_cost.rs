//! S4 过渡执行成本计数（#1233 对象 b/F4；#1243 修订）。
//!
//! 记录器逐世界（挂在 [`WorldBindingState`](crate::kernel::state::WorldBindingState)
//! 上）：融合路径经绑定引用直接登记，分发路径把同一计数体共享给池线程，
//! 计数完整落回所属世界，不串扰同进程内的其他世界。更新只在
//! `execution-evidence` 构建编译为原子累加；常规构建记录器为 ZST、
//! 所有登记内联为 no-op，未插桩测量不承担计数成本。

#[cfg(feature = "execution-evidence")]
use std::sync::Arc;
#[cfg(feature = "execution-evidence")]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "execution-evidence")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TransitionCostTotals {
    pub neighborhood_calls: u64,
    pub labels_scan_steps: u64,
    pub first_ring_scan_steps: u64,
    pub neighborhood_entries: u64,
    pub profile_calls: u64,
    pub profile_steps: u64,
}

#[cfg(feature = "execution-evidence")]
#[derive(Default)]
struct Counters {
    neighborhood_calls: AtomicU64,
    labels_scan_steps: AtomicU64,
    first_ring_scan_steps: AtomicU64,
    neighborhood_entries: AtomicU64,
    profile_calls: AtomicU64,
    profile_steps: AtomicU64,
}

/// 逐世界过渡成本记录器（#1243 T2）。`execution-evidence` 关闭时为 ZST。
pub(crate) struct Recorder {
    /// 计数体；克隆共享同一组原子量，池线程与协调线程写入同一世界。
    #[cfg(feature = "execution-evidence")]
    counters: Arc<Counters>,
}

impl Recorder {
    /// 新世界的记录器；计数从零开始。
    pub(crate) fn new() -> Self {
        Self {
            #[cfg(feature = "execution-evidence")]
            counters: Arc::new(Counters::default()),
        }
    }

    /// 登记一次潜在进入者邻域枚举的扫描成本（#1233）。
    #[inline]
    pub(crate) fn neighborhood(
        &self,
        labels_scan_steps: u64,
        first_ring_scan_steps: u64,
        entries: u64,
    ) {
        #[cfg(not(feature = "execution-evidence"))]
        let _ = (labels_scan_steps, first_ring_scan_steps, entries);
        #[cfg(feature = "execution-evidence")]
        {
            self.counters
                .neighborhood_calls
                .fetch_add(1, Ordering::Relaxed);
            self.counters
                .labels_scan_steps
                .fetch_add(labels_scan_steps, Ordering::Relaxed);
            self.counters
                .first_ring_scan_steps
                .fetch_add(first_ring_scan_steps, Ordering::Relaxed);
            self.counters
                .neighborhood_entries
                .fetch_add(entries, Ordering::Relaxed);
        }
    }

    /// 登记一次停止距离可达半径导出的 profile 遍历步数（#1233）。
    #[inline]
    pub(crate) fn profile(&self, profile_steps: u64) {
        #[cfg(not(feature = "execution-evidence"))]
        let _ = profile_steps;
        #[cfg(feature = "execution-evidence")]
        {
            self.counters.profile_calls.fetch_add(1, Ordering::Relaxed);
            self.counters
                .profile_steps
                .fetch_add(profile_steps, Ordering::Relaxed);
        }
    }

    /// 取出并清零**本世界**累计计数（#1243 T2：只影响所属世界，跨世界
    /// 归因不受同进程其他世界污染）。
    #[cfg(feature = "execution-evidence")]
    pub(crate) fn take(&self) -> TransitionCostTotals {
        TransitionCostTotals {
            neighborhood_calls: self.counters.neighborhood_calls.swap(0, Ordering::Relaxed),
            labels_scan_steps: self.counters.labels_scan_steps.swap(0, Ordering::Relaxed),
            first_ring_scan_steps: self
                .counters
                .first_ring_scan_steps
                .swap(0, Ordering::Relaxed),
            neighborhood_entries: self
                .counters
                .neighborhood_entries
                .swap(0, Ordering::Relaxed),
            profile_calls: self.counters.profile_calls.swap(0, Ordering::Relaxed),
            profile_steps: self.counters.profile_steps.swap(0, Ordering::Relaxed),
        }
    }
}
