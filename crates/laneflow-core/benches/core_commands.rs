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
    FIXED_COMMAND_COUNT, command_count_scenario, command_scenario, compaction_scenario,
    matched_command_scenario, remove_unused_route, repeated_command_scenario,
    run_in_use_route_failure_batch, run_mixed_churn_batch, run_overlap_failure_batch,
    run_safe_spawn_despawn_batch, warm_command_scenario,
};

fn benchmark_command_batch(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    name: &str,
    detail: impl std::fmt::Display,
    scenario: &CommandScenario,
    command_count: usize,
    routine: fn(&mut CommandScenario, usize) -> usize,
) {
    group.bench_with_input(
        BenchmarkId::new(name, detail),
        scenario,
        |benchmark, scenario| {
            benchmark.iter_batched_ref(
                || {
                    let mut scenario = scenario.clone();
                    warm_command_scenario(&mut scenario);
                    scenario
                },
                |scenario| black_box(routine(scenario, command_count)),
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
        FIXED_COMMAND_COUNT,
        run_safe_spawn_despawn_batch,
    );
    benchmark_command_batch(
        &mut group,
        "overlap_failure",
        DEFAULT_ROUTE_LENGTH,
        &scenario,
        FIXED_COMMAND_COUNT,
        run_overlap_failure_batch,
    );
    benchmark_command_batch(
        &mut group,
        "in_use_route_failure",
        DEFAULT_ROUTE_LENGTH,
        &scenario,
        FIXED_COMMAND_COUNT,
        run_in_use_route_failure_batch,
    );
    benchmark_command_batch(
        &mut group,
        "mixed_churn",
        DEFAULT_ROUTE_LENGTH,
        &scenario,
        FIXED_COMMAND_COUNT,
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
            FIXED_COMMAND_COUNT,
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
            benchmark.iter_batched_ref(
                || {
                    let mut scenario = scenario.clone();
                    warm_command_scenario(&mut scenario);
                    scenario
                },
                |scenario| black_box(remove_unused_route(scenario)),
                BatchSize::LargeInput,
            );
        },
    );
    remove_group.finish();
}

fn benchmark_extended_workloads(criterion: &mut Criterion) {
    let mut batch_group = criterion.benchmark_group("core_commands_batch_size_10000");
    batch_group.sample_size(20);
    batch_group.warm_up_time(Duration::from_secs(1));
    batch_group.measurement_time(Duration::from_secs(5));
    for command_count in [1, 100, 1_000] {
        let scenario =
            command_count_scenario(COMMAND_VEHICLE_COUNT, DEFAULT_ROUTE_LENGTH, command_count);
        batch_group.throughput(Throughput::Elements(command_count as u64));
        benchmark_command_batch(
            &mut batch_group,
            "overlap_failure",
            command_count,
            &scenario,
            command_count,
            run_overlap_failure_batch,
        );
    }
    batch_group.finish();

    let mut repeated_group = criterion.benchmark_group("core_commands_repeated_route_10000");
    repeated_group.sample_size(20);
    repeated_group.warm_up_time(Duration::from_secs(1));
    repeated_group.measurement_time(Duration::from_secs(5));
    repeated_group.throughput(Throughput::Elements(FIXED_COMMAND_COUNT as u64));
    for route_length in [8, 64, 512] {
        let scenario = repeated_command_scenario(COMMAND_VEHICLE_COUNT, route_length);
        benchmark_command_batch(
            &mut repeated_group,
            "overlap_failure",
            route_length,
            &scenario,
            FIXED_COMMAND_COUNT,
            run_overlap_failure_batch,
        );
    }
    repeated_group.finish();

    let mut matched_10k = criterion.benchmark_group("core_commands_matched_10000_100");
    matched_10k.sample_size(20);
    matched_10k.warm_up_time(Duration::from_secs(1));
    matched_10k.measurement_time(Duration::from_secs(5));
    matched_10k.throughput(Throughput::Elements(100));
    let matched_10k_scenario = matched_command_scenario(10_000, 8, 100, 10);
    benchmark_command_batch(
        &mut matched_10k,
        "overlap_failure",
        "routes=10_occurrences=8",
        &matched_10k_scenario,
        100,
        run_overlap_failure_batch,
    );
    matched_10k.finish();

    let mut compaction_group = criterion.benchmark_group("core_commands_compaction_threshold");
    compaction_group.sample_size(10);
    compaction_group.warm_up_time(Duration::from_secs(1));
    compaction_group.measurement_time(Duration::from_secs(5));
    compaction_group.throughput(Throughput::Elements(1));
    let compaction = compaction_scenario(COMMAND_VEHICLE_COUNT);
    compaction_group.bench_with_input(
        BenchmarkId::new("despawn_trigger", COMMAND_VEHICLE_COUNT),
        &compaction,
        |benchmark, scenario| {
            benchmark.iter_batched_ref(
                || command_scenarios::CompactionScenario {
                    world: scenario.world.clone(),
                    trigger: scenario.trigger,
                },
                |scenario| {
                    black_box(
                        scenario
                            .world
                            .despawn_vehicle(scenario.trigger)
                            .expect("threshold despawn"),
                    )
                },
                BatchSize::LargeInput,
            );
        },
    );
    compaction_group.finish();

    let mut cold_group = criterion.benchmark_group("core_commands_cold_build");
    cold_group.sample_size(10);
    cold_group.warm_up_time(Duration::from_secs(1));
    cold_group.measurement_time(Duration::from_secs(5));
    cold_group.bench_function("world_10000_routes_64", |benchmark| {
        benchmark.iter_batched(
            || (),
            |()| {
                black_box(command_scenario(
                    COMMAND_VEHICLE_COUNT,
                    DEFAULT_ROUTE_LENGTH,
                ))
            },
            BatchSize::LargeInput,
        );
    });
    cold_group.finish();
}

fn benchmark_100k_extended(criterion: &mut Criterion) {
    let mut matched = criterion.benchmark_group("core_commands_matched_100000_1000");
    matched.sample_size(10);
    matched.warm_up_time(Duration::from_secs(1));
    matched.measurement_time(Duration::from_secs(10));
    matched.throughput(Throughput::Elements(1_000));
    let scenario = matched_command_scenario(100_000, 80, 1_000, 100);
    benchmark_command_batch(
        &mut matched,
        "overlap_failure",
        "routes=100_occurrences=80",
        &scenario,
        1_000,
        run_overlap_failure_batch,
    );
    matched.finish();
}

fn benchmark_core_commands(criterion: &mut Criterion) {
    benchmark_fixed_commands(criterion, COMMAND_VEHICLE_COUNT, 20, 5);
    benchmark_extended_workloads(criterion);
    if std::env::var_os("LANEFLOW_BENCH_100K").is_some() {
        benchmark_fixed_commands(criterion, COMMAND_SCALING_VEHICLE_COUNT, 10, 10);
        benchmark_100k_extended(criterion);
    }
}

criterion_group!(benches, benchmark_core_commands);
criterion_main!(benches);
