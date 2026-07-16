//! Core lifecycle command 性能基线。
//!
//! 运行：`cargo +1.96.0 bench -p laneflow-core --bench core_commands --locked`。
//! 设置 `LANEFLOW_BENCH_100K=1` 额外运行 100k fixed-command isolation。

use std::{hint::black_box, time::Duration};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

#[path = "../tests/support/command_validation_scenarios.rs"]
mod command_scenarios;

use command_scenarios::{
    COMMAND_SCALING_VEHICLE_COUNT, COMMAND_VEHICLE_COUNT, CommandScenario, DEFAULT_ROUTE_LENGTH,
    FIXED_COMMAND_COUNT, command_scenario, remove_unused_route, run_in_use_route_failure_batch,
    run_mixed_churn_batch, run_overlap_failure_batch, run_safe_spawn_despawn_batch,
};

fn benchmark_command_batch(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    name: &str,
    detail: impl std::fmt::Display,
    scenario: &CommandScenario,
    routine: fn(&mut CommandScenario, usize) -> usize,
) {
    group.bench_with_input(
        BenchmarkId::new(name, detail),
        scenario,
        |benchmark, scenario| {
            benchmark.iter_batched(
                || scenario.clone(),
                |mut scenario| black_box(routine(&mut scenario, FIXED_COMMAND_COUNT)),
                BatchSize::LargeInput,
            );
        },
    );
}

fn benchmark_fixed_commands(
    criterion: &mut Criterion,
    vehicle_count: usize,
    sample_size: usize,
    measurement_seconds: u64,
) {
    let scenario = command_scenario(vehicle_count, DEFAULT_ROUTE_LENGTH);
    assert!(run_safe_spawn_despawn_batch(&mut scenario.clone(), FIXED_COMMAND_COUNT) > 0);
    assert!(run_overlap_failure_batch(&mut scenario.clone(), FIXED_COMMAND_COUNT) > 0);
    assert!(run_in_use_route_failure_batch(&mut scenario.clone(), FIXED_COMMAND_COUNT) > 0);
    assert!(run_mixed_churn_batch(&mut scenario.clone(), FIXED_COMMAND_COUNT) > 0);
    assert!(remove_unused_route(&mut scenario.clone()) > 0);

    let mut group = criterion.benchmark_group(format!("core_commands_fixed_100_{vehicle_count}"));
    group.sample_size(sample_size);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(measurement_seconds));
    group.throughput(Throughput::Elements(FIXED_COMMAND_COUNT as u64));
    benchmark_command_batch(
        &mut group,
        "safe_spawn_despawn",
        DEFAULT_ROUTE_LENGTH,
        &scenario,
        run_safe_spawn_despawn_batch,
    );
    benchmark_command_batch(
        &mut group,
        "overlap_failure",
        DEFAULT_ROUTE_LENGTH,
        &scenario,
        run_overlap_failure_batch,
    );
    benchmark_command_batch(
        &mut group,
        "in_use_route_failure",
        DEFAULT_ROUTE_LENGTH,
        &scenario,
        run_in_use_route_failure_batch,
    );
    benchmark_command_batch(
        &mut group,
        "mixed_churn",
        DEFAULT_ROUTE_LENGTH,
        &scenario,
        run_mixed_churn_batch,
    );
    group.finish();

    let mut route_group =
        criterion.benchmark_group(format!("core_commands_route_length_{vehicle_count}"));
    route_group.sample_size(sample_size);
    route_group.warm_up_time(Duration::from_secs(1));
    route_group.measurement_time(Duration::from_secs(measurement_seconds));
    route_group.throughput(Throughput::Elements(FIXED_COMMAND_COUNT as u64));
    for route_length in [8, 64, 512] {
        let route_scenario = command_scenario(vehicle_count, route_length);
        benchmark_command_batch(
            &mut route_group,
            "overlap_failure",
            route_length,
            &route_scenario,
            run_overlap_failure_batch,
        );
    }
    route_group.finish();

    let mut remove_group =
        criterion.benchmark_group(format!("core_commands_remove_unused_{vehicle_count}"));
    remove_group.sample_size(sample_size);
    remove_group.warm_up_time(Duration::from_secs(1));
    remove_group.measurement_time(Duration::from_secs(measurement_seconds));
    remove_group.throughput(Throughput::Elements(1));
    remove_group.bench_with_input(
        BenchmarkId::new("unused_route", DEFAULT_ROUTE_LENGTH),
        &scenario,
        |benchmark, scenario| {
            benchmark.iter_batched(
                || scenario.clone(),
                |mut scenario| black_box(remove_unused_route(&mut scenario)),
                BatchSize::LargeInput,
            );
        },
    );
    remove_group.finish();
}

fn benchmark_core_commands(criterion: &mut Criterion) {
    benchmark_fixed_commands(criterion, COMMAND_VEHICLE_COUNT, 20, 5);
    if std::env::var_os("LANEFLOW_BENCH_100K").is_some() {
        benchmark_fixed_commands(criterion, COMMAND_SCALING_VEHICLE_COUNT, 10, 10);
    }
}

criterion_group!(benches, benchmark_core_commands);
criterion_main!(benches);
