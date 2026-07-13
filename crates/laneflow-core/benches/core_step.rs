//! Core step 性能基线。
//!
//! 运行：`cargo +1.96.0 bench -p laneflow-core --bench core_step --locked`。
//! 设置 `LANEFLOW_BENCH_100K=1` 可额外运行 100k dense platoon 扩展性观察。
//! 本 benchmark 不进入常规 CI；结果仅与同一基准机上的历史基线比较。

use std::{hint::black_box, time::Duration};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use laneflow_core::{CoreWorld, TickInput};

#[path = "../tests/support/vehicle_following_scenarios.rs"]
mod scenarios;

use scenarios::{
    FIXED_DELTA_TIME_MS, SCALING_VEHICLE_COUNT, STEP_COUNT, VEHICLE_COUNT, dense_platoon_world,
    free_flow_world, projection_event_count, projection_heavy_world, stop_and_go_world,
    transition_event_count, transition_heavy_world,
};

fn run_steps(world: &mut CoreWorld) -> usize {
    let mut event_count = 0;
    for _ in 0..STEP_COUNT {
        let result = world
            .step(TickInput::new(FIXED_DELTA_TIME_MS))
            .expect("benchmark step must succeed");
        event_count += result.events.len();
    }
    event_count
}

fn run_single_step(world: &mut CoreWorld) -> usize {
    world
        .step(TickInput::new(FIXED_DELTA_TIME_MS))
        .expect("benchmark step must succeed")
        .events
        .len()
}

fn benchmark_world(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    name: &str,
    detail: impl std::fmt::Display,
    world: &CoreWorld,
) {
    group.bench_with_input(BenchmarkId::new(name, detail), world, |benchmark, world| {
        benchmark.iter_batched(
            || world.clone(),
            |mut world| black_box(run_steps(&mut world)),
            BatchSize::LargeInput,
        );
    });
}

fn benchmark_single_step(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    name: &str,
    detail: impl std::fmt::Display,
    world: &CoreWorld,
) {
    group.bench_with_input(BenchmarkId::new(name, detail), world, |benchmark, world| {
        benchmark.iter_batched(
            || world.clone(),
            |mut world| black_box(run_single_step(&mut world)),
            BatchSize::LargeInput,
        );
    });
}

fn benchmark_core_step(criterion: &mut Criterion) {
    let free_flow = free_flow_world(VEHICLE_COUNT);
    let dense_platoon = dense_platoon_world(VEHICLE_COUNT);
    let stop_and_go = stop_and_go_world(VEHICLE_COUNT);
    let projection_heavy = projection_heavy_world(VEHICLE_COUNT);
    let transition_heavy = transition_heavy_world(VEHICLE_COUNT);

    assert_eq!(run_steps(&mut free_flow.clone()), 0);
    assert_eq!(run_steps(&mut dense_platoon.clone()), 0);
    black_box(run_steps(&mut stop_and_go.clone()));
    assert_eq!(
        run_single_step(&mut projection_heavy.clone()),
        projection_event_count(VEHICLE_COUNT)
    );
    assert_eq!(
        run_steps(&mut transition_heavy.clone()),
        transition_event_count(VEHICLE_COUNT)
    );

    let mut group = criterion.benchmark_group("vehicle_following_step_10k_60");
    group.sample_size(20);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(5));
    group.throughput(Throughput::Elements((VEHICLE_COUNT * STEP_COUNT) as u64));
    benchmark_world(&mut group, "free_flow", VEHICLE_COUNT, &free_flow);
    benchmark_world(&mut group, "dense_platoon", VEHICLE_COUNT, &dense_platoon);
    benchmark_world(&mut group, "stop_and_go", VEHICLE_COUNT, &stop_and_go);
    group.finish();

    let mut projection_group = criterion.benchmark_group("vehicle_following_projection_10k_1");
    projection_group.sample_size(20);
    projection_group.warm_up_time(Duration::from_secs(1));
    projection_group.measurement_time(Duration::from_secs(5));
    projection_group.throughput(Throughput::Elements(VEHICLE_COUNT as u64));
    benchmark_single_step(
        &mut projection_group,
        "projection_heavy",
        VEHICLE_COUNT,
        &projection_heavy,
    );
    projection_group.finish();

    let mut transition_group = criterion.benchmark_group("vehicle_following_transition_10k_60");
    transition_group.sample_size(20);
    transition_group.warm_up_time(Duration::from_secs(1));
    transition_group.measurement_time(Duration::from_secs(5));
    transition_group.throughput(Throughput::Elements((VEHICLE_COUNT * STEP_COUNT) as u64));
    benchmark_world(
        &mut transition_group,
        "transition_heavy",
        "isolated_self_loop",
        &transition_heavy,
    );
    transition_group.finish();

    if std::env::var_os("LANEFLOW_BENCH_100K").is_some() {
        let scaling_world = dense_platoon_world(SCALING_VEHICLE_COUNT);
        assert_eq!(run_steps(&mut scaling_world.clone()), 0);

        let mut group = criterion.benchmark_group("vehicle_following_step_100k_60_observation");
        group.sample_size(10);
        group.warm_up_time(Duration::from_secs(1));
        group.measurement_time(Duration::from_secs(5));
        group.throughput(Throughput::Elements(
            (SCALING_VEHICLE_COUNT * STEP_COUNT) as u64,
        ));
        benchmark_world(
            &mut group,
            "dense_platoon",
            SCALING_VEHICLE_COUNT,
            &scaling_world,
        );
        group.finish();
    }
}

criterion_group!(benches, benchmark_core_step);
criterion_main!(benches);
