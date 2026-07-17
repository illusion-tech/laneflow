//! #122 research-only scalar candidate benchmark.

use std::{hint::black_box, time::Duration};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

#[allow(dead_code)]
#[path = "../tests/support/numeric_precision_research.rs"]
mod candidates;

use candidates::{
    CandidateLayout, CandidateScenario, CandidateWorld, CompensatedF32Mode, F64Mode, MixedF32Mode,
    PrecisionMode, RawF32Mode, SCALING_VEHICLE_COUNT, STEP_COUNT, VEHICLE_COUNT,
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
            benchmark_mode::<F64Mode>(&mut group, scenario, layout, VEHICLE_COUNT);
            benchmark_mode::<RawF32Mode>(&mut group, scenario, layout, VEHICLE_COUNT);
            benchmark_mode::<CompensatedF32Mode>(&mut group, scenario, layout, VEHICLE_COUNT);
            benchmark_mode::<MixedF32Mode>(&mut group, scenario, layout, VEHICLE_COUNT);
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
        benchmark_mode::<F64Mode>(
            &mut group,
            CandidateScenario::DensePlatoon,
            layout,
            SCALING_VEHICLE_COUNT,
        );
        benchmark_mode::<RawF32Mode>(
            &mut group,
            CandidateScenario::DensePlatoon,
            layout,
            SCALING_VEHICLE_COUNT,
        );
        benchmark_mode::<CompensatedF32Mode>(
            &mut group,
            CandidateScenario::DensePlatoon,
            layout,
            SCALING_VEHICLE_COUNT,
        );
        benchmark_mode::<MixedF32Mode>(
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
