//! Core step 性能基线。
//!
//! 运行：`cargo +1.96.0 bench -p laneflow-core --bench core_step --locked`。
//! 设置 `LANEFLOW_BENCH_100K=1` 可额外运行 100k dense platoon 扩展性观察。
//! 本 benchmark 不进入常规 CI；结果仅与同一基准机上的历史基线比较。

use std::{hint::black_box, time::Duration};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use laneflow_core::{
    CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge, LaneGraph,
    Route, Speed, TickInput, VehicleProfile, VehicleProfileHandle, VehicleProfileRegistry,
    VehicleSpawnInput,
};

const VEHICLE_COUNT: usize = 10_000;
const SCALING_VEHICLE_COUNT: usize = 100_000;
const STEP_COUNT: usize = 60;
const FIXED_DELTA_TIME_MS: u64 = 16;
const MILLISECONDS_PER_SECOND: f64 = 1_000.0;
const VEHICLE_LENGTH: f64 = 4.5;
const VEHICLE_SPACING: f64 = 6.5;
const TRANSITION_EDGE_LENGTH: f64 = 5.0;
const TRANSITION_EVENT_COUNT: usize = VEHICLE_COUNT * STEP_COUNT;

fn edge_length(value: f64) -> EdgeLength {
    EdgeLength::try_new(value).expect("benchmark edge length must be valid")
}

fn progress(value: f64) -> EdgeProgress {
    EdgeProgress::try_new(value).expect("benchmark edge progress must be valid")
}

fn speed(value: f64) -> Speed {
    Speed::try_new(value).expect("benchmark speed must be valid")
}

fn vehicle_external_id(index: usize) -> String {
    format!("vehicle-{index:06}-f0e1d2c3-b4a5-6789-0abc-def123456789")
}

fn edge_external_id(index: usize) -> String {
    format!("edge-{index:05}")
}

fn route_external_id(index: usize) -> String {
    format!("route-{index:05}")
}

fn profile_registry() -> (VehicleProfileRegistry, VehicleProfileHandle) {
    let registry = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        "benchmark-profile",
        IidmProfileSpec {
            length: VEHICLE_LENGTH,
            desired_speed: 13.9,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 1.4,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 4.0,
        },
    )
    .expect("benchmark profile must be valid")])
    .expect("benchmark profile registry must be valid");
    let profile = registry
        .profile_handle("benchmark-profile")
        .expect("benchmark profile handle must exist");
    (registry, profile)
}

fn dense_platoon_world(vehicle_count: usize) -> CoreWorld {
    let edge_length_value = VEHICLE_SPACING * vehicle_count as f64 + 10.0;
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "platoon-edge",
        edge_length(edge_length_value),
        std::iter::empty::<&str>(),
    )])
    .expect("dense platoon graph must be valid");
    let route =
        Route::try_new("platoon-route", ["platoon-edge"]).expect("platoon route must be valid");
    let (profiles, profile) = profile_registry();
    let traffic_data = InitialTrafficData::try_new(lane_graph, [route], profiles)
        .expect("dense platoon traffic data must be valid");
    let vehicles = (0..vehicle_count)
        .map(|index| {
            VehicleSpawnInput::active(
                vehicle_external_id(index),
                profile,
                "platoon-route",
                0,
                progress(VEHICLE_SPACING * index as f64),
                speed(1.0),
            )
        })
        .collect();

    CoreWorld::with_traffic_data(FIXED_DELTA_TIME_MS, traffic_data, vehicles)
        .expect("dense platoon world must be valid")
}

fn transition_heavy_world() -> CoreWorld {
    let edge_ids: Vec<_> = (0..VEHICLE_COUNT).map(edge_external_id).collect();
    let lane_graph = LaneGraph::try_new(edge_ids.iter().map(|edge_id| {
        LaneEdge::new(
            edge_id.clone(),
            edge_length(TRANSITION_EDGE_LENGTH),
            [edge_id.clone()],
        )
    }))
    .expect("transition graph must be valid");
    let routes: Vec<_> = edge_ids
        .iter()
        .enumerate()
        .map(|(index, edge_id)| {
            Route::try_new(
                route_external_id(index),
                std::iter::repeat_n(edge_id.clone(), STEP_COUNT + 1),
            )
            .expect("transition route must be valid")
        })
        .collect();
    let (profiles, profile) = profile_registry();
    let traffic_data = InitialTrafficData::try_new(lane_graph, routes, profiles)
        .expect("transition traffic data must be valid");
    let seconds_per_step = FIXED_DELTA_TIME_MS as f64 / MILLISECONDS_PER_SECOND;
    let transition_speed = speed(TRANSITION_EDGE_LENGTH / seconds_per_step);
    let vehicles = (0..VEHICLE_COUNT)
        .map(|index| {
            VehicleSpawnInput::active(
                vehicle_external_id(index),
                profile,
                route_external_id(index),
                0,
                EdgeProgress::ZERO,
                transition_speed,
            )
        })
        .collect();

    CoreWorld::with_traffic_data(FIXED_DELTA_TIME_MS, traffic_data, vehicles)
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

fn benchmark_core_step(criterion: &mut Criterion) {
    let dense_platoon = dense_platoon_world(VEHICLE_COUNT);
    let transition_heavy = transition_heavy_world();

    assert_eq!(run_steps(&mut dense_platoon.clone()), 0);
    assert_eq!(
        run_steps(&mut transition_heavy.clone()),
        TRANSITION_EVENT_COUNT
    );

    let mut group = criterion.benchmark_group("core_step_10k_60");
    group.sample_size(20);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(5));
    group.throughput(Throughput::Elements((VEHICLE_COUNT * STEP_COUNT) as u64));

    benchmark_world(&mut group, "dense_platoon", VEHICLE_COUNT, &dense_platoon);
    benchmark_world(
        &mut group,
        "transition_heavy",
        "isolated_self_loop",
        &transition_heavy,
    );
    group.finish();

    if std::env::var_os("LANEFLOW_BENCH_100K").is_some() {
        let scaling_world = dense_platoon_world(SCALING_VEHICLE_COUNT);
        assert_eq!(run_steps(&mut scaling_world.clone()), 0);

        let mut group = criterion.benchmark_group("core_step_100k_60_observation");
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
