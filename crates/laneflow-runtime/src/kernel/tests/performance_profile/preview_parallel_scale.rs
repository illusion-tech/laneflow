//! #705 机制级测量探针（非城市性能认证，#707 另行验收）。
//!
//! 同一合成 Waiting 密集场景（multi-gate，车辆停在 idle zone 入口 Gate 前）下对比
//! 融合（worker 1）与多 worker 分发（2/4/8/16）的整步墙钟、P2 段（WaitingPrepare
//! 插桩均值）、调度统计、MotionPreview 缓存复用与并行暂存内存。路线短、车辆会
//! 跑完，探针在每拍定步前把 Completed 车辆原位替换回 Gate 前，保持稳态活动车队
//! 与恒定 P2 工作量。预览局部收益不等于城市性能通过。
//!
//! 单独运行：
//! `cargo test --release -p laneflow-runtime --lib \
//!   preview_parallel_scale -- --ignored --nocapture`

use std::time::Instant;

use super::{CALLS, ENABLED, NANOS, STAGE_COUNT, Stage};
use crate::TickInput;
use crate::kernel::execution::{DispatchStats, WorldExecution, last_dispatch_stats};
use crate::kernel::tick::{self, MotionCacheUse};
use crate::kernel::waiting::{self, WaitingPreviewPathCounts};
use crate::{RouteHandle, VehicleSpawnInput, VehicleStatus};
use laneflow_static_contract::VehicleProfileOrdinal;

/// 车辆数：Waiting 密集且 P2 预览/运动计算占主导的规模。
const VEHICLES: usize = 1_024;
const WARMUP_TICKS: usize = 24;
const MEASURED_TICKS: usize = 128;
const ROUNDS: usize = 3;
const DELTA_MS: u64 = 100;
/// 补员后允许的瞬时活动数下界（一拍内跑完全路线的车辆数有界）。
const ACTIVE_FLOOR: usize = VEHICLES - 128;

#[derive(Default)]
struct DispatchTotals {
    dispatched_chunks: usize,
    completed_chunks: usize,
    skipped_chunks: usize,
    extra_work_chunks: usize,
    calls: usize,
}

impl DispatchTotals {
    fn accumulate(&mut self, stats: DispatchStats) {
        self.dispatched_chunks += stats.dispatched_chunks;
        self.completed_chunks += stats.completed_chunks;
        self.skipped_chunks += stats.skipped_chunks;
        self.extra_work_chunks += stats.extra_work_chunks;
        self.calls += 1;
    }
}

struct ArmRow {
    whole_p50_ns: u128,
    whole_p95_ns: u128,
    waiting_mean_ns: u128,
    active_last: usize,
    active_sum: usize,
    cache: MotionCacheUse,
    paths: WaitingPreviewPathCounts,
    dispatch: DispatchTotals,
    peak_threads: usize,
    workspace_bytes_cold: u64,
    workspace_bytes_hot: u64,
    inputs_cap_cold: usize,
    inputs_len_hot: usize,
    inputs_cap_hot: usize,
    slots_cap_cold: usize,
    slots_len_hot: usize,
    slots_cap_hot: usize,
    digest: String,
}

fn percentile(sorted: &[u128], fraction: f64) -> u128 {
    let index = (fraction * sorted.len() as f64).ceil() as usize;
    sorted[index.saturating_sub(1).min(sorted.len() - 1)]
}

fn exec_config(workers: u32) -> crate::ExecutionConfig {
    crate::ExecutionConfig::new(std::num::NonZeroU32::new(workers).unwrap())
}

fn step(world: &mut crate::TrafficWorld) {
    std::hint::black_box(world.step(TickInput::new(DELTA_MS)).unwrap());
}

/// 每条路线的首边末端毫米；补员把 Completed 车辆按夹具标准位置放回原路线。
fn route_boundaries(world: &crate::TrafficWorld, routes: &[RouteHandle]) -> Vec<u32> {
    let lengths = world.traffic().lane_lengths_millimetres();
    routes
        .iter()
        .map(|route| lengths[world.route_edges(*route).unwrap()[0].index()])
        .collect()
}

/// 稳态补员：把已完成车辆原位替换回 Gate 前，保持活动车队与 P2 工作量恒定。
fn replenish(world: &mut crate::TrafficWorld, routes: &[RouteHandle], boundaries: &[u32]) {
    for handle in world.live_vehicles().to_vec() {
        let state = world.vehicle(handle).expect("live handle");
        if state.status != VehicleStatus::Completed {
            continue;
        }
        let position = routes
            .iter()
            .position(|route| *route == state.route)
            .expect("completed vehicle keeps its registered route");
        world
            .replace_completed_vehicle(
                handle,
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    routes[position],
                    0,
                    boundaries[position] - 1,
                    10_000,
                ),
            )
            .expect("vacated route entry accepts replacement");
    }
}

fn run_arm(workers: u32, round: usize) -> ArmRow {
    // 同一轮的所有 worker 臂共享世界身份：digest 跨臂可比（等价自检）；
    // 探针无故障注入，不需要按 worker 隔离身份。
    let _ = workers;
    let world_id = 705_500 + round as u64;
    let (mut world, routes) =
        waiting::tests::multi_gate_world_partial(VEHICLES, VEHICLES, world_id);
    world.execution = WorldExecution::start_private(exec_config(workers), &world.state);
    let boundaries = route_boundaries(&world, &routes);
    let workspace_bytes_cold = world.state.workspace.retained_logical_bytes();
    let inputs_cap_cold = world.state.workspace.waiting_preview_inputs.capacity();
    let slots_cap_cold = world.state.workspace.waiting_preview_slots.capacity();
    for _ in 0..WARMUP_TICKS {
        replenish(&mut world, &routes, &boundaries);
        step(&mut world);
    }
    let workspace_bytes_hot = world.state.workspace.retained_logical_bytes();
    let inputs_len_hot = world.state.workspace.waiting_preview_inputs.len();
    let inputs_cap_hot = world.state.workspace.waiting_preview_inputs.capacity();
    let slots_len_hot = world.state.workspace.waiting_preview_slots.len();
    let slots_cap_hot = world.state.workspace.waiting_preview_slots.capacity();
    let paths_before = waiting::preview_path_counts();
    let cache_before = tick::motion_cache_use();
    ENABLED.with(|enabled| enabled.set(true));
    NANOS.with(|nanos| nanos.set([0; STAGE_COUNT]));
    CALLS.with(|calls| calls.set([0; STAGE_COUNT]));
    let mut whole = Vec::with_capacity(MEASURED_TICKS);
    let mut dispatch = DispatchTotals::default();
    let mut peak_threads = 0_usize;
    let mut active_sum = 0_usize;
    for _ in 0..MEASURED_TICKS {
        replenish(&mut world, &routes, &boundaries);
        // 采样本拍 P5 实际处理的活动数（定步前）；完成车辆在提交后才离开投影。
        active_sum += world.state.derived.active_order.len();
        let started = Instant::now();
        step(&mut world);
        whole.push(started.elapsed().as_nanos());
        if let Some(stats) = last_dispatch_stats() {
            dispatch.accumulate(stats);
            peak_threads = peak_threads.max(stats.threads);
        }
    }
    ENABLED.with(|enabled| enabled.set(false));
    let waiting_nanos = NANOS.with(|nanos| nanos.get())[Stage::WaitingPrepare as usize];
    let waiting_calls = CALLS.with(|calls| calls.get())[Stage::WaitingPrepare as usize];
    assert_eq!(
        waiting_calls, MEASURED_TICKS as u64,
        "waiting_prepare 每拍恰好一次"
    );
    let cache_after = tick::motion_cache_use();
    let paths_after = waiting::preview_path_counts();
    let cache = MotionCacheUse {
        hits: cache_after.hits - cache_before.hits,
        misses: cache_after.misses - cache_before.misses,
    };
    let paths = WaitingPreviewPathCounts {
        dispatched: paths_after.dispatched - paths_before.dispatched,
        fused: paths_after.fused - paths_before.fused,
        slot_fallback: paths_after.slot_fallback - paths_before.slot_fallback,
    };
    let mut sorted = whole;
    sorted.sort_unstable();
    let digest = format!(
        "{:x}",
        crate::deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap()
    );
    ArmRow {
        whole_p50_ns: percentile(&sorted, 0.50),
        whole_p95_ns: percentile(&sorted, 0.95),
        waiting_mean_ns: waiting_nanos / MEASURED_TICKS as u128,
        active_last: world.state.derived.active_order.len(),
        active_sum,
        cache,
        paths,
        dispatch,
        peak_threads,
        workspace_bytes_cold,
        workspace_bytes_hot,
        inputs_cap_cold,
        inputs_len_hot,
        inputs_cap_hot,
        slots_cap_cold,
        slots_len_hot,
        slots_cap_hot,
        digest,
    }
}

/// 注入一次预览错误，记录完整 join 的调度统计（错误后多做工作）。
/// 注入掩码按 u64 武装，错误臂用 64 车的同形场景，位置 32 恰在块边界
/// （workers=4 时块大小 8）中段，便于观察早块完成/晚块跳过。
fn run_error_arm() {
    const WORKERS: u32 = 4;
    const ERROR_VEHICLES: usize = 64;
    let world_id = 705_999;
    let (mut world, routes) =
        waiting::tests::multi_gate_world_partial(ERROR_VEHICLES, ERROR_VEHICLES, world_id);
    world.execution = WorldExecution::start_private(exec_config(WORKERS), &world.state);
    let boundaries = route_boundaries(&world, &routes);
    for _ in 0..WARMUP_TICKS {
        replenish(&mut world, &routes, &boundaries);
        step(&mut world);
    }
    let position = ERROR_VEHICLES / 2;
    let before = world.capture_snapshot().unwrap();
    let guard = waiting::inject_preview_errors(world_id, &[position], &[]);
    replenish(&mut world, &routes, &boundaries);
    let result = world.step(TickInput::new(DELTA_MS));
    drop(guard);
    assert_eq!(result, Err(crate::StepError::NonFiniteMotion));
    assert_eq!(world.capture_snapshot().unwrap(), before);
    let stats = last_dispatch_stats().expect("dispatch arm records stats");
    println!(
        "preview-parallel-error scene=multi-gate-{ERROR_VEHICLES} workers={WORKERS} position={position} \
         dispatched={} completed={} skipped={} extra_work={} threads={}",
        stats.dispatched_chunks,
        stats.completed_chunks,
        stats.skipped_chunks,
        stats.extra_work_chunks,
        stats.threads,
    );
}

#[test]
#[ignore = "#705 机制级测量：单独 release 运行，见文件头命令"]
fn preview_parallel_scale() {
    let mut reference_digest = [const { None::<String> }; ROUNDS];
    for &workers in &[1_u32, 2, 4, 8, 16] {
        for (round, reference) in reference_digest.iter_mut().enumerate() {
            let row = run_arm(workers, round);
            if workers == 1 {
                *reference = Some(row.digest.clone());
            }
            assert_eq!(
                row.digest,
                reference.as_deref().unwrap(),
                "worker={workers} round={round} 与同轮融合参考 digest 一致"
            );
            assert!(
                row.active_last >= ACTIVE_FLOOR && row.active_sum >= ACTIVE_FLOOR * MEASURED_TICKS,
                "worker={workers} round={round} 稳态活动车队（last={} sum={}）",
                row.active_last,
                row.active_sum,
            );
            if workers == 1 {
                assert_eq!(row.paths.dispatched, 0, "worker=1 走融合路径");
                assert_eq!(row.paths.fused, MEASURED_TICKS, "worker=1 每拍融合");
            } else {
                assert_eq!(
                    row.paths.dispatched, MEASURED_TICKS,
                    "workers={workers} 每拍真实分发"
                );
            }
            assert_eq!(
                row.cache.hits + row.cache.misses,
                row.active_sum,
                "P5 每活动车一次复用判定"
            );
            println!(
                "preview-parallel scene=multi-gate-{VEHICLES} workers={workers} round={round} \
                 whole_p50_ns={} whole_p95_ns={} waiting_mean_ns={} active_last={} active_sum={} \
                 cache_hits={} cache_misses={} path_dispatched={} path_fused={} path_fallback={} \
                 dispatch_dispatched={} dispatch_completed={} dispatch_skipped={} dispatch_extra_work={} dispatch_calls={} peak_threads={} \
                 workspace_bytes_cold={} workspace_bytes_hot={} inputs_cap_cold={} inputs_len={} inputs_cap={} slots_cap_cold={} slots_len={} slots_cap={} digest={}",
                row.whole_p50_ns,
                row.whole_p95_ns,
                row.waiting_mean_ns,
                row.active_last,
                row.active_sum,
                row.cache.hits,
                row.cache.misses,
                row.paths.dispatched,
                row.paths.fused,
                row.paths.slot_fallback,
                row.dispatch.dispatched_chunks,
                row.dispatch.completed_chunks,
                row.dispatch.skipped_chunks,
                row.dispatch.extra_work_chunks,
                row.dispatch.calls,
                row.peak_threads,
                row.workspace_bytes_cold,
                row.workspace_bytes_hot,
                row.inputs_cap_cold,
                row.inputs_len_hot,
                row.inputs_cap_hot,
                row.slots_cap_cold,
                row.slots_len_hot,
                row.slots_cap_hot,
                row.digest,
            );
        }
    }
    run_error_arm();
    println!("preview-parallel-done");
}
