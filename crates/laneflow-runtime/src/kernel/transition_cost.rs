//! S4 过渡执行成本计数（#1233 对象 b/F4）。
//!
//! 计数器为进程级原子量：融合路径与分发路径（池线程）都完整累计，避免
//! thread-local 账本丢失 worker 线程计数。仅在测量作用域读取并清零。

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

struct Counters {
    neighborhood_calls: AtomicU64,
    labels_scan_steps: AtomicU64,
    first_ring_scan_steps: AtomicU64,
    neighborhood_entries: AtomicU64,
    profile_calls: AtomicU64,
    profile_steps: AtomicU64,
}

static COUNTERS: Counters = Counters {
    neighborhood_calls: AtomicU64::new(0),
    labels_scan_steps: AtomicU64::new(0),
    first_ring_scan_steps: AtomicU64::new(0),
    neighborhood_entries: AtomicU64::new(0),
    profile_calls: AtomicU64::new(0),
    profile_steps: AtomicU64::new(0),
};

/// 登记一次潜在进入者邻域枚举的扫描成本（#1233）。
pub(crate) fn neighborhood(labels_scan_steps: u64, first_ring_scan_steps: u64, entries: u64) {
    COUNTERS.neighborhood_calls.fetch_add(1, Ordering::Relaxed);
    COUNTERS
        .labels_scan_steps
        .fetch_add(labels_scan_steps, Ordering::Relaxed);
    COUNTERS
        .first_ring_scan_steps
        .fetch_add(first_ring_scan_steps, Ordering::Relaxed);
    COUNTERS
        .neighborhood_entries
        .fetch_add(entries, Ordering::Relaxed);
}

/// 登记一次停止距离可达半径导出的 profile 遍历步数（#1233）。
pub(crate) fn profile(profile_steps: u64) {
    COUNTERS.profile_calls.fetch_add(1, Ordering::Relaxed);
    COUNTERS
        .profile_steps
        .fetch_add(profile_steps, Ordering::Relaxed);
}

/// 取出并清零累计计数（#1233）。
#[cfg(feature = "execution-evidence")]
pub(crate) fn take() -> TransitionCostTotals {
    TransitionCostTotals {
        neighborhood_calls: COUNTERS.neighborhood_calls.swap(0, Ordering::Relaxed),
        labels_scan_steps: COUNTERS.labels_scan_steps.swap(0, Ordering::Relaxed),
        first_ring_scan_steps: COUNTERS.first_ring_scan_steps.swap(0, Ordering::Relaxed),
        neighborhood_entries: COUNTERS.neighborhood_entries.swap(0, Ordering::Relaxed),
        profile_calls: COUNTERS.profile_calls.swap(0, Ordering::Relaxed),
        profile_steps: COUNTERS.profile_steps.swap(0, Ordering::Relaxed),
    }
}
