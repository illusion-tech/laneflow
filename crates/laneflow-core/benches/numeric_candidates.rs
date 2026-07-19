//! #122/#127 研究专用标量候选性能基准。

use std::{hint::black_box, time::Duration};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

#[allow(dead_code)]
#[path = "../tests/support/numeric_precision_research.rs"]
mod candidates;

use candidates::{
    CandidateLayout, CandidateScenario, CandidateWorld, CompensatedF32Mode, F64Mode, MixedF32Mode,
    PrecisionMode, RawF32Mode, ResidualAwareF32Mode, SCALING_VEHICLE_COUNT, STEP_COUNT,
    SensitiveControlMixedMode, VEHICLE_COUNT,
};

fn benchmark_mode<M: PrecisionMode>(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    scenario: CandidateScenario,
    layout: CandidateLayout,
    vehicle_count: usize,
) {
    let world = CandidateWorld::<M>::new(vehicle_count, scenario, layout);
    group.bench_with_input(
        BenchmarkId::new(
            format!(
                "{}/{}/{}",
                layout.benchmark_name(),
                scenario.benchmark_name(),
                M::NAME
            ),
            vehicle_count,
        ),
        &world,
        |benchmark, world| {
            benchmark.iter_batched(
                || world.clone(),
                |mut world| black_box(world.run_steps(STEP_COUNT)),
                BatchSize::LargeInput,
            );
        },
    );
}

fn benchmark_modes(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    scenario: CandidateScenario,
    layout: CandidateLayout,
    vehicle_count: usize,
) {
    if reverse_candidate_order() {
        benchmark_mode::<MixedF32Mode>(group, scenario, layout, vehicle_count);
        benchmark_mode::<SensitiveControlMixedMode>(group, scenario, layout, vehicle_count);
        benchmark_mode::<ResidualAwareF32Mode>(group, scenario, layout, vehicle_count);
        benchmark_mode::<CompensatedF32Mode>(group, scenario, layout, vehicle_count);
        benchmark_mode::<RawF32Mode>(group, scenario, layout, vehicle_count);
        benchmark_mode::<F64Mode>(group, scenario, layout, vehicle_count);
    } else {
        benchmark_mode::<F64Mode>(group, scenario, layout, vehicle_count);
        benchmark_mode::<RawF32Mode>(group, scenario, layout, vehicle_count);
        benchmark_mode::<CompensatedF32Mode>(group, scenario, layout, vehicle_count);
        benchmark_mode::<ResidualAwareF32Mode>(group, scenario, layout, vehicle_count);
        benchmark_mode::<SensitiveControlMixedMode>(group, scenario, layout, vehicle_count);
        benchmark_mode::<MixedF32Mode>(group, scenario, layout, vehicle_count);
    }
}

fn reverse_candidate_order() -> bool {
    std::env::var_os("LANEFLOW_NUMERIC_BENCH_REVERSE_ORDER").is_some()
}

fn benchmark_candidate_matrix(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("numeric_candidate_step_10k_60");
    group.sample_size(20);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(5));
    group.throughput(Throughput::Elements((VEHICLE_COUNT * STEP_COUNT) as u64));
    for layout in [
        CandidateLayout::LegacySingleEdge,
        CandidateLayout::LocalityPreserving,
    ] {
        for scenario in [
            CandidateScenario::FreeFlow,
            CandidateScenario::DensePlatoon,
            CandidateScenario::StopAndGo,
        ] {
            benchmark_modes(&mut group, scenario, layout, VEHICLE_COUNT);
        }
    }
    group.finish();

    if std::env::var_os("LANEFLOW_NUMERIC_BENCH_100K").is_none() {
        return;
    }
    let mut group = criterion.benchmark_group("numeric_candidate_dense_step_100k_60_observation");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(10));
    group.throughput(Throughput::Elements(
        (SCALING_VEHICLE_COUNT * STEP_COUNT) as u64,
    ));
    for layout in [
        CandidateLayout::LegacySingleEdge,
        CandidateLayout::LocalityPreserving,
    ] {
        benchmark_modes(
            &mut group,
            CandidateScenario::DensePlatoon,
            layout,
            SCALING_VEHICLE_COUNT,
        );
    }
    group.finish();
}

criterion_group!(benches, benchmark_candidate_matrix);
criterion_main!(benches);
