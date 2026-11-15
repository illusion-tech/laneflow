//! os-probe.md §6：构造后 CPU 时间与进程内存读取不分配堆内存。
//!
//! 计数口径：`harness = false`，本文件的 `main` 即测试进程入口，不经 libtest。
//! 整个进程只有 `main` 一个线程，没有 libtest 主线程与测试线程并发，也没有
//! 输出捕获；`stats_alloc` 全局计数分配器在测量区间内的读数就是被测读取本身
//! 的分配。参数解析与输出都在测量区间之外完成。
//!
//! 为兼容 cargo-nextest，`main` 实现自定义测试框架的最小协议：
//! `--list --format terse` 列出唯一一项 `NAME: test`；同时带 `--ignored` 时
//! 列表为空（本项不是 ignored 测试）；位置参数是名称过滤（带 `--exact` 时须
//! 全名相同，否则为子串匹配），`--skip` 排除匹配项；不带参数时直接运行。
//! `--nocapture`、`--test-threads` 等其他参数不影响行为，宽松忽略。失败时
//! panic，以非零状态退出并打印区间计数。

use std::alloc::System;

use laneflow_os_probe::Probe;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const NAME: &str = "reads_after_construction_do_not_allocate";

/// 带取值的 libtest 选项：其后一个参数是取值，不是名称过滤。
const OPTIONS_WITH_VALUE: [&str; 5] = ["--format", "--test-threads", "--color", "--logfile", "-Z"];

fn main() {
    let mut list = false;
    let mut exact = false;
    let mut ignored_only = false;
    let mut filters = Vec::new();
    let mut skips = Vec::new();
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--list" => list = true,
            "--exact" => exact = true,
            "--ignored" => ignored_only = true,
            "--skip" => skips.extend(arguments.next()),
            option if OPTIONS_WITH_VALUE.contains(&option) => {
                arguments.next();
            }
            option if option.starts_with('-') => {}
            _ => filters.push(argument),
        }
    }
    let matches = |pattern: &String| {
        if exact {
            pattern == NAME
        } else {
            NAME.contains(pattern.as_str())
        }
    };
    let selected = !ignored_only
        && (filters.is_empty() || filters.iter().any(matches))
        && !skips.iter().any(matches);
    if list {
        if selected {
            println!("{NAME}: test");
        }
        return;
    }
    if !selected {
        println!("running 0 tests");
        return;
    }
    println!("running 1 test");
    reads_after_construction_do_not_allocate();
    println!("test {NAME} ... ok");
}

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
