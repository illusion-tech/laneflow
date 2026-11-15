//! os-probe.md §6：构造后 CPU 时间与进程内存读取不分配堆内存。
//!
//! 本测试二进制只含这一个测试，是独立进程：nextest 每个测试一个进程，
//! `cargo test` 下本进程也没有其他测试并发分配。全局计数分配器的区间读数只
//! 反映被测读取。

use std::alloc::System;

use laneflow_os_probe::Probe;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

#[test]
fn reads_after_construction_do_not_allocate() {
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
}

fn read_all(probe: &mut Probe) {
    std::hint::black_box(probe.system_cpu().expect("整机 CPU"));
    std::hint::black_box(probe.process_cpu().expect("本进程 CPU"));
    std::hint::black_box(probe.thread_cpu().expect("当前线程 CPU"));
    std::hint::black_box(probe.memory().expect("本进程内存"));
}
