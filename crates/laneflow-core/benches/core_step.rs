//! Core step 性能基线。
//!
//! 运行：`cargo +1.96.0 bench -p laneflow-core --bench core_step --locked`。
//! 本 benchmark 不进入常规 CI；结果仅与同一基准机上的历史基线比较。

use std::{hint::black_box, time::Duration};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use laneflow_core::{
    CoreWorld, EdgeLength, EdgeProgress, LaneEdge, LaneGraph, Route, Speed, TickInput,
    VehicleSpawnInput,
};

const VEHICLE_COUNT: usize = 10_000;
const STEP_COUNT: usize = 60;
const FIXED_DELTA_TIME_MS: u64 = 16;
const MILLISECONDS_PER_SECOND: f64 = 1_000.0;
const STEADY_EDGE_LENGTH: f64 = 100.0;
const TRANSITION_EDGE_LENGTH: f64 = 1.0;
const TRANSITION_EVENT_COUNT: usize = VEHICLE_COUNT * STEP_COUNT;

fn edge_length(value: f64) -> EdgeLength {
    EdgeLength::try_new(value).expect("benchmark edge length must be valid")
}

fn speed(value: f64) -> Speed {
    Speed::try_new(value).expect("benchmark speed must be valid")
}

fn vehicle_external_id(index: usize) -> String {
    format!("vehicle-{index:05}-f0e1d2c3-b4a5-6789-0abc-def123456789")
}

fn vehicles(route_id: &str, speed: Speed) -> Vec<VehicleSpawnInput> {
    (0..VEHICLE_COUNT)
        .map(|index| {
            VehicleSpawnInput::active(
                vehicle_external_id(index),
                route_id,
                0,
                EdgeProgress::ZERO,
                speed,
            )
        })
        .collect()
}

fn steady_state_world() -> CoreWorld {
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "steady-edge",
        edge_length(STEADY_EDGE_LENGTH),
        std::iter::empty::<&str>(),
    )])
    .expect("steady-state graph must be valid");
    let route =
        Route::try_new("steady-route", ["steady-edge"]).expect("steady-state route must be valid");

    CoreWorld::with_traffic_data(
        FIXED_DELTA_TIME_MS,
        lane_graph,
        [route],
        vehicles("steady-route", speed(1.0)),
    )
    .expect("steady-state world must be valid")
}

fn transition_heavy_world() -> CoreWorld {
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "loop-edge",
        edge_length(TRANSITION_EDGE_LENGTH),
        ["loop-edge"],
    )])
    .expect("transition graph must be valid");
    let route = Route::try_new(
        "transition-route",
        std::iter::repeat_n("loop-edge", STEP_COUNT + 1),
    )
    .expect("transition route must be valid");
    let seconds_per_step = FIXED_DELTA_TIME_MS as f64 / MILLISECONDS_PER_SECOND;

    CoreWorld::with_traffic_data(
        FIXED_DELTA_TIME_MS,
        lane_graph,
        [route],
        vehicles(
            "transition-route",
            speed(TRANSITION_EDGE_LENGTH / seconds_per_step),
        ),
    )
    .expect("transition world must be valid")
}

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

fn benchmark_core_step(criterion: &mut Criterion) {
    let steady_state = steady_state_world();
    let transition_heavy = transition_heavy_world();

    assert_eq!(run_steps(&mut steady_state.clone()), 0);
    assert_eq!(
        run_steps(&mut transition_heavy.clone()),
        TRANSITION_EVENT_COUNT
    );

    let mut group = criterion.benchmark_group("core_step_10k_60");

    group.sample_size(20);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(5));
    group.throughput(Throughput::Elements((VEHICLE_COUNT * STEP_COUNT) as u64));

    group.bench_with_input(
        BenchmarkId::new("steady_state", "no_transition"),
        &steady_state,
        |benchmark, world| {
            benchmark.iter_batched(
                || world.clone(),
                |mut world| black_box(run_steps(&mut world)),
                BatchSize::LargeInput,
            );
        },
    );
    group.bench_with_input(
        BenchmarkId::new("transition_heavy", "self_loop"),
        &transition_heavy,
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

criterion_group!(benches, benchmark_core_step);
criterion_main!(benches);
