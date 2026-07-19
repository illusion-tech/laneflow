//! #144 真实 production layout 数值迁移性能矩阵。
//!
//! 运行时必须用 `LANEFLOW_PRODUCTION_BENCH_SCALE=10000` 或 `100000`
//! 选择规模。每次进程只运行一个规模的七类预先冻结 step 工作负载，便于按
//! current-f64、候选 A、候选 B 轮换独立进程顺序。

use std::{hint::black_box, time::Duration};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use laneflow_core::{CoreWorld, TickInput};

#[path = "../tests/support/parking_runtime_scenarios.rs"]
#[allow(dead_code, reason = "共享 Parking helper 还提供本矩阵不使用的命令场景")]
mod parking_scenarios;
#[path = "../tests/support/signals_validation_scenarios.rs"]
#[allow(
    dead_code,
    reason = "共享 Signals helper 还提供本矩阵不使用的模式和计数"
)]
mod signal_scenarios;
#[path = "../tests/support/vehicle_following_scenarios.rs"]
#[allow(
    dead_code,
    reason = "共享 Vehicle Following helper 还提供其他基准与测试入口"
)]
mod vehicle_scenarios;

use signal_scenarios::{SignalScenarioMode, reserved_parking_scenario, signal_scenario};
use vehicle_scenarios::{
    FIXED_DELTA_TIME_MS, STEP_COUNT, locality_dense_platoon_world, locality_free_flow_world,
    locality_stop_and_go_world, projection_event_count, projection_heavy_world,
    transition_event_count, transition_heavy_world,
};

const TEN_THOUSAND: usize = 10_000;
const ONE_HUNDRED_THOUSAND: usize = 100_000;

fn selected_scale() -> usize {
    let value = std::env::var("LANEFLOW_PRODUCTION_BENCH_SCALE").unwrap_or_else(|_| {
        panic!("必须设置 LANEFLOW_PRODUCTION_BENCH_SCALE=10000 或 100000，禁止一次进程混合两档规模")
    });
    let scale = value
        .parse::<usize>()
        .unwrap_or_else(|_| panic!("无效 LANEFLOW_PRODUCTION_BENCH_SCALE: {value}"));
    assert!(
        matches!(scale, TEN_THOUSAND | ONE_HUNDRED_THOUSAND),
        "LANEFLOW_PRODUCTION_BENCH_SCALE 只接受 10000 或 100000"
    );
    scale
}

fn run_steps(world: &mut CoreWorld) -> usize {
    let mut event_count = 0;
    for _ in 0..STEP_COUNT {
        let result = world
            .step(TickInput::new(FIXED_DELTA_TIME_MS))
            .expect("production benchmark step must succeed");
        event_count += result.events.len();
    }
    event_count
}

fn run_single_step(world: &mut CoreWorld) -> usize {
    world
        .step(TickInput::new(FIXED_DELTA_TIME_MS))
        .expect("production benchmark step must succeed")
        .events
        .len()
}

fn configure_group<'a>(
    criterion: &'a mut Criterion,
    scenario: &str,
    scale: usize,
    step_count: usize,
) -> criterion::BenchmarkGroup<'a, criterion::measurement::WallTime> {
    let mut group =
        criterion.benchmark_group(format!("numeric_production_step_{scale}_{scenario}"));
    group.sample_size(if scale == TEN_THOUSAND { 15 } else { 10 });
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(if scale == TEN_THOUSAND {
        3
    } else {
        5
    }));
    group.throughput(Throughput::Elements((scale * step_count) as u64));
    group
}

fn benchmark_steps(criterion: &mut Criterion, scenario: &str, scale: usize, world: CoreWorld) {
    black_box(run_steps(&mut world.clone()));
    let mut group = configure_group(criterion, scenario, scale, STEP_COUNT);
    group.bench_with_input(
        BenchmarkId::new("production", scale),
        &world,
        |benchmark, world| {
            benchmark.iter_batched(
                || world.clone(),
                |mut world| black_box(run_steps(&mut world)),
                BatchSize::LargeInput,
            );
        },
    );
    group.finish();
}

fn benchmark_projection(criterion: &mut Criterion, scale: usize) {
    let world = projection_heavy_world(scale);
    assert_eq!(
        run_single_step(&mut world.clone()),
        projection_event_count(scale)
    );
    let mut group = configure_group(criterion, "projection", scale, 1);
    group.bench_with_input(
        BenchmarkId::new("production", scale),
        &world,
        |benchmark, world| {
            benchmark.iter_batched(
                || world.clone(),
                |mut world| black_box(run_single_step(&mut world)),
                BatchSize::LargeInput,
            );
        },
    );
    group.finish();
}

fn benchmark_production_matrix(criterion: &mut Criterion) {
    let scale = selected_scale();

    benchmark_steps(
        criterion,
        "free_flow",
        scale,
        locality_free_flow_world(scale),
    );
    benchmark_steps(
        criterion,
        "dense_platoon",
        scale,
        locality_dense_platoon_world(scale),
    );
    benchmark_steps(
        criterion,
        "stop_and_go",
        scale,
        locality_stop_and_go_world(scale),
    );
    benchmark_projection(criterion, scale);

    let transition = transition_heavy_world(scale);
    assert_eq!(
        run_steps(&mut transition.clone()),
        transition_event_count(scale)
    );
    benchmark_steps(criterion, "cross_edge", scale, transition);

    let signals = signal_scenario(scale, SignalScenarioMode::MixedOffsets);
    benchmark_steps(criterion, "signals", scale, signals.world);

    let parking = reserved_parking_scenario(scale, 100);
    benchmark_steps(criterion, "parking", scale, parking.world);
}

criterion_group!(benches, benchmark_production_matrix);
criterion_main!(benches);
