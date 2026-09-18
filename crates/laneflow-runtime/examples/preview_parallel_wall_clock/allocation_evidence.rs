//! #705 热态分发分配预算证据（阻断一修法）。
//!
//! 口径：LaneFlow 自有分配在热态分发路径为零——票据认领不再每次分发
//! `collect()` 票据容器（锁直接保护切片迭代器），块级诊断记录在诊断未
//! 启用时不分配，输入/槽位暂存预热后复用。残差预算为每拍每辅助线程每
//! 分发阶段一个 Rayon scope 任务节点（执行配置 §3 明确豁免的进程级内部
//! 调度分配，与块数无关）；本场景每拍有三个分发阶段：P2 预览、P3 候选
//! 与 P5 运动（#706 增量 D 起 P3 候选在 Pool 执行器与阈值之上同样真实
//! 分发），故预算 = 拍数 × worker × 3（worker 为执行配置工作线程总数，
//! 含调用线程；调用线程在部分调度下也会得到一个 scope 任务节点）。
//! 再分配必须为零。`allocated_bytes` 只输出、不断言（残差字节随 Rayon
//! 实现，预算断言只锁计数与再分配）。

mod multi_gate_scene;

use std::alloc::System;

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const VEHICLES: usize = 2_048;
const WORKERS: u32 = 4;
const WARMUP_TICKS: u32 = 8;
const MEASURED_TICKS: u32 = 8;
/// 测量窗末仍须远高于分发阈值，保证全程真实分发。
const ACTIVE_FLOOR: usize = 1_024;

#[test]
fn preview_dispatch_steady_tick_allocation_budget_after_warmup() {
    let revision = multi_gate_scene::build_revision(VEHICLES);
    let mut world = multi_gate_scene::install(&revision, WORKERS, 705_282);
    let routes = multi_gate_scene::routes_of(&world);
    let boundaries = multi_gate_scene::route_boundaries(&world, &routes);
    for _ in 0..WARMUP_TICKS {
        multi_gate_scene::replenish(&mut world, &routes, &boundaries);
        multi_gate_scene::step(&mut world);
    }
    let region = Region::new(GLOBAL);
    for _ in 0..MEASURED_TICKS {
        multi_gate_scene::step(&mut world);
    }
    let stats = region.change();
    // LaneFlow 自有分配为零（预算口径）：票据认领锁直接保护切片迭代器
    // `collect()`；输入/槽位暂存预热后复用。残差上界为每拍每辅助线程一个
    // Rayon scope 任务节点（执行配置 §3 明确豁免的进程级内部调度分配，
    // 与块数无关、旧容器实现按块数倍增长）。再分配与增长必须为零。
    // 预算 = 拍数 × 阶段数 × 参与线程数（含调用线程；调用线程在部分调度下
    // 也会得到一个 scope 任务节点，实测约为预算的 77%）。阶段数 = 3：
    // P2 预览、P3 候选、P5 运动。残差全部归 Rayon
    // 内部调度（执行配置 §3 豁免），LaneFlow 自有增长已归零：reallocations
    // 必须为零，且分配数不随块数/车辆完成增长（固定为每拍常数级）。
    assert!(
        stats.allocations <= (MEASURED_TICKS * 3 * WORKERS) as usize,
        "steady dispatch ticks allocated beyond rayon task node budget: {}",
        stats.allocations
    );
    assert_eq!(stats.reallocations, 0, "steady dispatch ticks reallocated");
    let active = world
        .live_vehicles()
        .iter()
        .filter(|handle| {
            world
                .vehicle(**handle)
                .is_some_and(|state| state.status() == laneflow_runtime::VehicleStatus::Active)
        })
        .count();
    assert!(
        active >= ACTIVE_FLOOR,
        "measurement window must stay on the dispatch path, active={active}"
    );
    println!(
        "preview-dispatch-allocation-evidence steady_ticks={MEASURED_TICKS} allocations={} \
         reallocations={} allocated_bytes={} active_end={active}",
        stats.allocations, stats.reallocations, stats.bytes_allocated
    );
}
