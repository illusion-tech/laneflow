//! os-probe.md §6：构造后 CPU 时间与进程内存读取不分配堆内存。
//!
//! `harness = false`：本文件即独立测试进程的 `main`，没有 libtest 线程与输出
//! 并发分配，全局计数分配器的区间读数只反映被测读取。

use std::alloc::System;

use laneflow_os_probe::Probe;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn main() {
    let mut probe = Probe::new().expect("构造读取器");
    // 预热一次：首次调用可能触发运行时惰性初始化。
    read_all(&mut probe);
    let region = Region::new(GLOBAL);
    for _ in 0..16 {
        read_all(&mut probe);
    }
    let change = region.change();
    assert_eq!(
        (change.allocations, change.reallocations),
        (0, 0),
        "构造后读取发生堆分配：{change:?}"
    );
    println!("read_without_allocation: ok");
}

fn read_all(probe: &mut Probe) {
    std::hint::black_box(probe.system_cpu().expect("整机 CPU"));
    std::hint::black_box(probe.process_cpu().expect("本进程 CPU"));
    std::hint::black_box(probe.thread_cpu().expect("当前线程 CPU"));
    std::hint::black_box(probe.memory().expect("本进程内存"));
}
