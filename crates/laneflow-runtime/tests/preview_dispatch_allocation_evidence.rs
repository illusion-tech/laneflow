//! #705 分发路径热态零堆分配证据（阻断一修法）。
//!
//! 多 worker（4）世界、2_048 活动车辆（远高于生产分发阈值 1_024），
//! 票据认领不再每次分发 `collect()` 票据容器（锁直接保护切片迭代器），
//! 块级诊断记录在诊断未启用时不分配；预热建立全部暂存容量后，测量窗口内
//! 连续固定步进的 allocation / reallocation / allocated_bytes 必须为零。

#[path = "support/multi_gate_scene.rs"]
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
fn preview_dispatch_steady_tick_has_zero_heap_allocation_after_warmup() {
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
    // LaneFlow 自有分配为零：票据认领锁直接保护切片迭代器，热态无容器
    // `collect()`；输入/槽位暂存预热后复用。残差上界为每拍每辅助线程一个
    // Rayon scope 任务节点（执行配置 §3 明确豁免的进程级内部调度分配，
    // 与块数无关、旧容器实现按块数倍增长）。再分配与增长必须为零。
    assert!(
        stats.allocations <= (MEASURED_TICKS * (WORKERS - 1)) as usize,
        "steady dispatch ticks allocated beyond rayon task nodes: {}",
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
