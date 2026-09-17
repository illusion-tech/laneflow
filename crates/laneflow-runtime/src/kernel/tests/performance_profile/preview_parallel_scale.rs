//! #705 机制级测量探针（非城市性能认证，#707 另行验收）。
//!
//! 同一合成 Waiting 密集场景（multi-gate，车辆停在 idle zone 入口 Gate 前）下对比
//! 融合（worker 1）与多 worker 分发（2/4/8/16）的整步墙钟、P2 诊断子段
//! （前置与输入准备 / 独立预览计算 / 分发与 join 等待 / 规范消费 / 后续组装）、
//! 调度统计、MotionPreview 缓存复用与并行暂存内存；并对票据式认领与块数倍数
//! 做有界 A/B。路线短、车辆会跑完，探针在每拍定步前把 Completed 车辆原位
//! 替换回 Gate 前，保持稳态活动车队与恒定 P2 工作量。预览局部收益不等于
//! 城市性能通过。
//!
//! 单独运行：
//! `cargo test --release -p laneflow-runtime --lib \
//!   preview_parallel_scale -- --ignored --nocapture`

use std::time::Instant;

use super::{CALLS, ENABLED, NANOS, STAGE_COUNT, Stage};
use crate::TickInput;
use crate::kernel::execution::{DispatchStats, WorldExecution, last_dispatch_stats};
use crate::kernel::tick::{self, MotionCacheUse, WaitingPreviewEntry};
use crate::kernel::waiting::{
    self, WaitingPreviewPathCounts, force_preview_dispatch, preview_stage,
    set_preview_chunk_multiplier,
};
use crate::{RouteHandle, VehicleSpawnInput, VehicleStatus};
use laneflow_static_contract::VehicleProfileOrdinal;

const VEHICLES: usize = 1_024;
const WARMUP_TICKS: usize = 24;
const MEASURED_TICKS: usize = 128;
const DELTA_MS: u64 = 100;

#[derive(Default)]
struct DispatchTotals {
    dispatched_chunks: usize,
    completed_chunks: usize,
    skipped_chunks: usize,
    extra_work_chunks: usize,
    ticket_grabs: usize,
    calls: usize,
}

impl DispatchTotals {
    fn accumulate(&mut self, stats: DispatchStats) {
        self.dispatched_chunks += stats.dispatched_chunks;
        self.completed_chunks += stats.completed_chunks;
        self.skipped_chunks += stats.skipped_chunks;
        self.extra_work_chunks += stats.extra_work_chunks;
        self.ticket_grabs += stats.ticket_grabs;
        self.calls += 1;
    }
}

struct ArmRow {
    whole_p50_ns: u128,
    whole_p95_ns: u128,
    waiting_mean_ns: u128,
    substage_ns: [u128; preview_stage::STAGE_COUNT],
    chunk_total_ns: u128,
    chunk_max_ns: u128,
    active_last: usize,
    active_sum: usize,
    cache: MotionCacheUse,
    paths: WaitingPreviewPathCounts,
    dispatch: DispatchTotals,
    participating_threads: usize,
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

#[allow(clippy::too_many_arguments)]
fn run_arm(
    workers: u32,
    multiplier: usize,
    vehicles: usize,
    round: usize,
    force_dispatch: bool,
) -> ArmRow {
    // 同一（规模, 轮）的所有臂共享世界身份：digest 跨臂可比（等价自检）；
    // 探针无故障注入，不需要按 worker 隔离身份。
    let world_id = 705_500 + vehicles as u64 + round as u64;
    set_preview_chunk_multiplier(multiplier);
    // 生产阈值保守（1_024 活动）；探针按臂选择强制分发，保证路径确定。
    let _force = force_dispatch.then(force_preview_dispatch);
    let (mut world, routes) =
        waiting::tests::multi_gate_world_partial(vehicles, vehicles, world_id);
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
    preview_stage::set_enabled(true);
    let mut whole = Vec::with_capacity(MEASURED_TICKS);
    let mut dispatch = DispatchTotals::default();
    let mut participating_threads = 0_usize;
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
            participating_threads = participating_threads.max(stats.participating_threads);
        }
    }
    ENABLED.with(|enabled| enabled.set(false));
    preview_stage::set_enabled(false);
    let (substage_total_ns, chunk_total_ns, chunk_max_ns) = preview_stage::take();
    let waiting_nanos = NANOS.with(|nanos| nanos.get())[Stage::WaitingPrepare as usize];
    let waiting_calls = CALLS.with(|calls| calls.get())[Stage::WaitingPrepare as usize];
    assert_eq!(
        waiting_calls, MEASURED_TICKS as u64,
        "waiting_prepare 每拍恰好一次"
    );
    let measured = MEASURED_TICKS as u128;
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
    set_preview_chunk_multiplier(2);
    ArmRow {
        whole_p50_ns: percentile(&sorted, 0.50),
        whole_p95_ns: percentile(&sorted, 0.95),
        waiting_mean_ns: waiting_nanos / measured,
        substage_ns: [
            substage_total_ns[preview_stage::PREAMBLE] / measured,
            substage_total_ns[preview_stage::FUSED_LOOP] / measured,
            substage_total_ns[preview_stage::DISPATCH_SCOPE] / measured,
            substage_total_ns[preview_stage::CONSUME] / measured,
            substage_total_ns[preview_stage::ASSEMBLY] / measured,
        ],
        chunk_total_ns: chunk_total_ns / measured,
        chunk_max_ns,
        active_last: world.state.derived.active_order.len(),
        active_sum,
        cache,
        paths,
        dispatch,
        participating_threads,
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
/// （workers=4 时块大小 8）中段：更早块完成，更晚任务在开始执行时
/// 查已错位置并整块跳过计算，scope 完整 join。
fn run_error_arm() {
    const WORKERS: u32 = 4;
    const ERROR_VEHICLES: usize = 64;
    let world_id = 705_999;
    // 64 车远低于生产阈值；错误臂观察分发调度统计，须强制真实分发。
    let _force = force_preview_dispatch();
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
         dispatched={} completed={} skipped={} extra_work={} participating_threads={} ticket_grabs={}",
        stats.dispatched_chunks,
        stats.completed_chunks,
        stats.skipped_chunks,
        stats.extra_work_chunks,
        stats.participating_threads,
        stats.ticket_grabs,
    );
}

struct ArmSpec {
    workers: u32,
    multiplier: usize,
    vehicles: usize,
    rounds: usize,
    force_dispatch: bool,
}

fn run_matrix(specs: &[ArmSpec], references: &mut [((usize, usize), Option<String>)]) {
    for spec in specs {
        for round in 0..spec.rounds {
            let row = run_arm(
                spec.workers,
                spec.multiplier,
                spec.vehicles,
                round,
                spec.force_dispatch,
            );
            let reference_index = (spec.vehicles, round);
            let slot = references
                .iter_mut()
                .find(|(key, _)| *key == reference_index)
                .map(|(_, value)| value);
            if let Some(slot) = slot {
                if spec.workers == 1 {
                    *slot = Some(row.digest.clone());
                }
                assert_eq!(
                    row.digest,
                    slot.as_deref().unwrap(),
                    "workers={} vehicles={} round={round} 与同轮融合参考 digest 一致",
                    spec.workers,
                    spec.vehicles,
                );
            }
            assert!(
                row.active_last >= spec.vehicles.saturating_sub(128)
                    && row.active_sum >= (spec.vehicles.saturating_sub(128)) * MEASURED_TICKS,
                "workers={} vehicles={} round={round} 稳态活动车队",
                spec.workers,
                spec.vehicles,
            );
            if spec.workers == 1 {
                assert_eq!(row.paths.dispatched, 0, "worker=1 走融合路径");
                assert_eq!(row.paths.fused, MEASURED_TICKS, "worker=1 每拍融合");
            } else if spec.force_dispatch {
                assert_eq!(
                    row.paths.dispatched, MEASURED_TICKS,
                    "workers={} 每拍真实分发（强制）",
                    spec.workers,
                );
            } else {
                assert_eq!(
                    row.paths.fused, MEASURED_TICKS,
                    "workers={} 低于生产阈值，每拍融合",
                    spec.workers,
                );
            }
            assert_eq!(
                row.cache.hits + row.cache.misses,
                row.active_sum,
                "P5 每活动车一次复用判定"
            );
            println!(
                "preview-parallel scene=multi-gate-{vehicles} workers={workers} \
                 mult={multiplier} round={round} \
                 whole_p50_ns={} whole_p95_ns={} waiting_mean_ns={} \
                 preamble_mean_ns={} fused_loop_mean_ns={} chunk_total_mean_ns={} chunk_max_ns={} dispatch_scope_mean_ns={} consume_mean_ns={} assembly_mean_ns={} \
                 active_last={} active_sum={} cache_hits={} cache_misses={} \
                 path_dispatched={} path_fused={} path_fallback={} \
                 dispatch_dispatched={} dispatch_completed={} dispatch_skipped={} dispatch_extra_work={} ticket_grabs={} dispatch_calls={} participating_threads={} \
                 workspace_bytes_cold={} workspace_bytes_hot={} inputs_cap_cold={} inputs_len={} inputs_cap={} slots_cap_cold={} slots_len={} slots_cap={} digest={}",
                row.whole_p50_ns,
                row.whole_p95_ns,
                row.waiting_mean_ns,
                row.substage_ns[preview_stage::PREAMBLE],
                row.substage_ns[preview_stage::FUSED_LOOP],
                row.chunk_total_ns,
                row.chunk_max_ns,
                row.substage_ns[preview_stage::DISPATCH_SCOPE],
                row.substage_ns[preview_stage::CONSUME],
                row.substage_ns[preview_stage::ASSEMBLY],
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
                row.dispatch.ticket_grabs,
                row.dispatch.calls,
                row.participating_threads,
                row.workspace_bytes_cold,
                row.workspace_bytes_hot,
                row.inputs_cap_cold,
                row.inputs_len_hot,
                row.inputs_cap_hot,
                row.slots_cap_cold,
                row.slots_len_hot,
                row.slots_cap_hot,
                row.digest,
                vehicles = spec.vehicles,
                workers = spec.workers,
                multiplier = spec.multiplier,
            );
        }
    }
}

#[test]
#[ignore = "#705 机制级测量：单独 release 运行，见文件头命令"]
fn preview_parallel_scale() {
    println!(
        "preview-parallel-meta slot_bytes={}",
        std::mem::size_of::<crate::kernel::execution::DispatchSlot<WaitingPreviewEntry>>()
    );
    // 基线矩阵：票据认领、块数 = 线程数 × 2。
    let baseline: Vec<ArmSpec> = [1_u32, 2, 4, 8, 16]
        .into_iter()
        .map(|workers| ArmSpec {
            workers,
            multiplier: 2,
            vehicles: VEHICLES,
            rounds: if workers == 1 { 3 } else { 2 },
            force_dispatch: workers > 1,
        })
        .collect();
    // 有界扫描：块数倍数（1×/2×/4× 线程数）与阈值证据（小/大工作集交叉）。
    let candidates: Vec<ArmSpec> = vec![
        ArmSpec {
            workers: 4,
            multiplier: 1,
            vehicles: VEHICLES,
            rounds: 2,

            force_dispatch: true,
        },
        ArmSpec {
            workers: 8,
            multiplier: 1,
            vehicles: VEHICLES,
            rounds: 2,

            force_dispatch: true,
        },
        ArmSpec {
            workers: 16,
            multiplier: 1,
            vehicles: VEHICLES,
            rounds: 2,

            force_dispatch: true,
        },
        ArmSpec {
            workers: 8,
            multiplier: 4,
            vehicles: VEHICLES,
            rounds: 2,

            force_dispatch: true,
        },
        ArmSpec {
            workers: 16,
            multiplier: 4,
            vehicles: VEHICLES,
            rounds: 2,

            force_dispatch: true,
        },
        // 阈值证据：小/大工作集下融合与分发的交叉。
        ArmSpec {
            workers: 1,
            multiplier: 2,
            vehicles: 256,
            rounds: 2,

            force_dispatch: false,
        },
        ArmSpec {
            workers: 4,
            multiplier: 2,
            vehicles: 256,
            rounds: 2,

            force_dispatch: false,
        },
        ArmSpec {
            workers: 1,
            multiplier: 2,
            vehicles: 4_096,
            rounds: 2,

            force_dispatch: true,
        },
        ArmSpec {
            workers: 4,
            multiplier: 2,
            vehicles: 4_096,
            rounds: 2,

            force_dispatch: true,
        },
        ArmSpec {
            workers: 8,
            multiplier: 2,
            vehicles: 4_096,
            rounds: 2,

            force_dispatch: true,
        },
    ];
    let all: Vec<ArmSpec> = baseline.into_iter().chain(candidates).collect();
    // 轮次展开后引用槽按 (vehicles, round) 全量建。
    let mut references: Vec<((usize, usize), Option<String>)> = Vec::new();
    for spec in &all {
        for round in 0..spec.rounds {
            let key = (spec.vehicles, round);
            if !references.iter().any(|(existing, _)| *existing == key) {
                references.push((key, None));
            }
        }
    }
    run_matrix(&all, &mut references);
    run_error_arm();
    println!("preview-parallel-done");
}
