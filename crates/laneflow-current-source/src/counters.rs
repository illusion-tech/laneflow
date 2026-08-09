//! 单遍证明计数器（仅 debug 构建）：根 deserializer 次数、record token replay
//! 次数与起点唯一性、SHA-256 逐角色次数。
//!
//! 计数器只编译在 `debug_assertions` 下：集成测试链接的 lib 不带 `cfg(test)`，
//! 因此用 `debug_assertions` 让测试进程内（lib 单测）与常规 debug 构建都能
//! 观察；release 构建零开销。
//!
//! 状态为线程局部：测试进程内多个用例并行解析不同文档，字节区间起点只在
//! 单份文档内有意义；每次根驱动（一份文档的开始）清空 replay 起点集合。

use std::cell::RefCell;
use std::collections::HashSet;

use crate::error::CurrentDocumentRole;

#[derive(Default)]
struct Counters {
    root_drivers: u64,
    replays: u64,
    replay_starts: HashSet<u32>,
    /// 逐角色 digest 次数（有界：[Manifest, Traffic, Spatial]）。
    digests: [u64; 3],
}

thread_local! {
    static COUNTERS: RefCell<Counters> = RefCell::new(Counters::default());
}

/// `digests` 数组的角色下标（有界逐角色计数的固定布局）。
const fn role_index(role: CurrentDocumentRole) -> usize {
    match role {
        CurrentDocumentRole::Manifest => 0,
        CurrentDocumentRole::Traffic => 1,
        CurrentDocumentRole::Spatial => 2,
    }
}

/// 计数快照（仅测试消费；常规 debug 构建不读取）。`digests` 布局同
/// `role_index`：[Manifest, Traffic, Spatial]。
#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct CounterSnapshot {
    pub(crate) root_drivers: u64,
    pub(crate) replays: u64,
    pub(crate) digests: [u64; 3],
}

/// 记录一次根文档 deserializer 驱动（每份文档恰好一次）；新文档开始即清空
/// replay 起点集合（起点只按单文档字节空间判唯一）。
pub(crate) fn record_root_driver() {
    COUNTERS.with(|counters| {
        let mut counters = counters.borrow_mut();
        counters.root_drivers += 1;
        counters.replay_starts.clear();
    });
}

/// 记录一次 record token replay；同一文档内同一零基起点 replay 两次即违反
/// 「每 token 至多解码一次」，硬断言失败。
pub(crate) fn record_replay(start: u32) {
    COUNTERS.with(|counters| {
        let mut counters = counters.borrow_mut();
        counters.replays += 1;
        assert!(
            counters.replay_starts.insert(start),
            "record token 被重复 replay（零基起点 {start}）"
        );
    });
}

/// 记录一次对 `role` 文档的 SHA-256 计算（每份文档恰好一次）。
pub(crate) fn record_digest(role: CurrentDocumentRole) {
    COUNTERS.with(|counters| counters.borrow_mut().digests[role_index(role)] += 1);
}

/// 当前线程的计数快照。
#[cfg(test)]
pub(crate) fn snapshot() -> CounterSnapshot {
    COUNTERS.with(|counters| {
        let counters = counters.borrow();
        CounterSnapshot {
            root_drivers: counters.root_drivers,
            replays: counters.replays,
            digests: counters.digests,
        }
    })
}

/// 重置当前线程的全部计数（测试内断言基线用）。
#[cfg(test)]
pub(crate) fn reset() {
    COUNTERS.with(|counters| *counters.borrow_mut() = Counters::default());
}
