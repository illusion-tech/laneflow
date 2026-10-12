//! 分配计量窗口逐拍账本探针（#794）。
//!
//! 零分配证据目标的偶发失败只给出窗口总数（如 `left: 4`），无法判别污染是
//! 懒物化落点漂移、他面串扰还是计量边界错误。本探针把每个计量拍的
//! `stats_alloc` 差值记入固定容量数组：记录路径不分配、不锁、不格式化
//! （`&'static str` 阶段标与拍号都是编译期值），只在计量段结束后整段输出，
//! 因此不进入任何计量窗口、不扰动被测计数。命中偶发时日志直接给出
//! 「哪一拍、多少分配/字节」，配合暖机段记录可读出懒物化距窗口边界的余量。

use stats_alloc::Stats;

/// 单拍计量差值；字段口径与 `stats_alloc::Stats` 的窗口差值一致。
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct TickStats {
    /// 本拍 alloc 调用数。
    pub allocations: usize,
    /// 本拍 realloc 调用数。
    pub reallocations: usize,
    /// 本拍新分配字节数。
    pub bytes_allocated: usize,
    /// 本拍释放字节数。
    pub bytes_deallocated: usize,
}

impl TickStats {
    fn from_stats(stats: Stats) -> Self {
        Self {
            allocations: stats.allocations,
            reallocations: stats.reallocations,
            bytes_allocated: stats.bytes_allocated,
            bytes_deallocated: stats.bytes_deallocated,
        }
    }

    /// 是否存在任何分配活动（含 realloc 与释放字节变化）。
    #[must_use]
    pub const fn is_active(self) -> bool {
        self.allocations != 0
            || self.reallocations != 0
            || self.bytes_allocated != 0
            || self.bytes_deallocated != 0
    }
}

const MAX_ENTRIES: usize = 128;

/// 固定容量逐拍账本；溢出后停止记录（末条被标记），不影响被测断言。
pub struct TickLedger {
    entries: [(&'static str, u32, TickStats); MAX_ENTRIES],
    len: usize,
    overflowed: bool,
}

impl TickLedger {
    /// 建立空账本。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: [(
                "",
                0,
                TickStats {
                    allocations: 0,
                    reallocations: 0,
                    bytes_allocated: 0,
                    bytes_deallocated: 0,
                },
            ); MAX_ENTRIES],
            len: 0,
            overflowed: false,
        }
    }

    /// 记录一拍；`phase` 为编译期阶段标，`tick` 为阶段内拍号。
    pub fn record(&mut self, phase: &'static str, tick: u32, stats: Stats) {
        if self.len == MAX_ENTRIES {
            self.overflowed = true;
            return;
        }
        self.entries[self.len] = (phase, tick, TickStats::from_stats(stats));
        self.len += 1;
    }

    /// 全账本合计；与整段 `Region::change()` 同口径，供断言前对照。
    #[must_use]
    pub const fn totals(&self) -> TickStats {
        let mut sum = TickStats {
            allocations: 0,
            reallocations: 0,
            bytes_allocated: 0,
            bytes_deallocated: 0,
        };
        let mut index = 0;
        while index < self.len {
            let entry = self.entries[index].2;
            sum.allocations += entry.allocations;
            sum.reallocations += entry.reallocations;
            sum.bytes_allocated += entry.bytes_allocated;
            sum.bytes_deallocated += entry.bytes_deallocated;
            index += 1;
        }
        sum
    }

    /// 整账本输出到 stderr：只列有活动的拍与阶段合计，末尾带溢出标记。
    ///
    /// 仅在断言失败前调用；格式化分配发生在全部计量窗口之外。
    pub fn dump(&self, label: &str) {
        let totals = self.totals();
        eprintln!(
            "[tick-ledger] {label}: entries={} totals(alloc={} realloc={} bytes={} dealloc_bytes={})",
            self.len,
            totals.allocations,
            totals.reallocations,
            totals.bytes_allocated,
            totals.bytes_deallocated
        );
        for (phase, tick, stats) in self.entries[..self.len].iter() {
            if stats.is_active() {
                eprintln!(
                    "[tick-ledger]   {phase}-{tick}: alloc={} realloc={} bytes={} dealloc_bytes={}",
                    stats.allocations,
                    stats.reallocations,
                    stats.bytes_allocated,
                    stats.bytes_deallocated
                );
            }
        }
        if self.overflowed {
            eprintln!("[tick-ledger]   (capacity {MAX_ENTRIES} exceeded; tail not recorded)");
        }
    }
}
