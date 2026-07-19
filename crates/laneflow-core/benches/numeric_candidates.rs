//! #122/#127 研究专用标量候选性能基准。

use std::{hint::black_box, time::Duration};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

#[allow(dead_code)]
#[path = "../tests/support/numeric_precision_research.rs"]
mod candidates;
#[allow(dead_code)]
#[path = "../tests/support/numeric_contract_calibration.rs"]
mod contract;

use candidates::{
    CandidateLayout, CandidateScenario, CandidateWorld, CompensatedF32Mode, F64Mode, MixedF32Mode,
    PrecisionMode, RawF32Mode, ResidualAwareF32Mode, SCALING_VEHICLE_COUNT, STEP_COUNT,
    SensitiveControlMixedMode, VEHICLE_COUNT,
};
use contract::{ConstraintWorkload, run_command_conversion_workload, run_constraint_workload};

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

fn benchmark_constraint_mode(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    workload: ConstraintWorkload,
    item_count: usize,
    target_f32: bool,
) {
    let mode = if target_f32 {
        "target_f32"
    } else {
        "current_f64"
    };
    group.bench_function(
        BenchmarkId::new(workload.benchmark_name(), mode),
        |benchmark| {
            benchmark.iter(|| black_box(run_constraint_workload(target_f32, workload, item_count)));
        },
    );
}

fn benchmark_constraint_modes(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    workload: ConstraintWorkload,
    item_count: usize,
) {
    if reverse_candidate_order() {
        benchmark_constraint_mode(group, workload, item_count, true);
        benchmark_constraint_mode(group, workload, item_count, false);
    } else {
        benchmark_constraint_mode(group, workload, item_count, false);
        benchmark_constraint_mode(group, workload, item_count, true);
    }
}

fn benchmark_command_modes(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    item_count: usize,
) {
    let mut benchmark_mode = |target_f32: bool| {
        let mode = if target_f32 {
            "target_f32"
        } else {
            "current_f64"
        };
        group.bench_function(BenchmarkId::new("command_conversion", mode), |benchmark| {
            benchmark.iter(|| black_box(run_command_conversion_workload(target_f32, item_count)));
        });
    };
    if reverse_candidate_order() {
        benchmark_mode(true);
        benchmark_mode(false);
    } else {
        benchmark_mode(false);
        benchmark_mode(true);
    }
}

fn benchmark_extended_matrix(criterion: &mut Criterion) {
    let mut cross_edge = criterion.benchmark_group("numeric_candidate_cross_edge_10k_60");
    cross_edge.sample_size(20);
    cross_edge.warm_up_time(Duration::from_secs(1));
    cross_edge.measurement_time(Duration::from_secs(5));
    cross_edge.throughput(Throughput::Elements((VEHICLE_COUNT * STEP_COUNT) as u64));
    benchmark_modes(
        &mut cross_edge,
        CandidateScenario::FreeFlow,
        CandidateLayout::EdgeCap100M,
        VEHICLE_COUNT,
    );
    cross_edge.finish();

    let mut constraints = criterion.benchmark_group("numeric_constraint_kernel_10k");
    constraints.sample_size(20);
    constraints.warm_up_time(Duration::from_secs(1));
    constraints.measurement_time(Duration::from_secs(5));
    constraints.throughput(Throughput::Elements(VEHICLE_COUNT as u64));
    for workload in ConstraintWorkload::ALL {
        benchmark_constraint_modes(&mut constraints, workload, VEHICLE_COUNT);
    }
    constraints.finish();

    let mut commands = criterion.benchmark_group("numeric_command_conversion_10k");
    commands.sample_size(20);
    commands.warm_up_time(Duration::from_secs(1));
    commands.measurement_time(Duration::from_secs(5));
    commands.throughput(Throughput::Elements(VEHICLE_COUNT as u64));
    benchmark_command_modes(&mut commands, VEHICLE_COUNT);
    commands.finish();

    if std::env::var_os("LANEFLOW_NUMERIC_BENCH_100K").is_none() {
        return;
    }

    let mut cross_edge = criterion.benchmark_group("numeric_candidate_cross_edge_100k_60");
    cross_edge.sample_size(10);
    cross_edge.warm_up_time(Duration::from_secs(1));
    cross_edge.measurement_time(Duration::from_secs(10));
    cross_edge.throughput(Throughput::Elements(
        (SCALING_VEHICLE_COUNT * STEP_COUNT) as u64,
    ));
    benchmark_modes(
        &mut cross_edge,
        CandidateScenario::FreeFlow,
        CandidateLayout::EdgeCap100M,
        SCALING_VEHICLE_COUNT,
    );
    cross_edge.finish();

    let mut constraints = criterion.benchmark_group("numeric_constraint_kernel_100k");
    constraints.sample_size(10);
    constraints.warm_up_time(Duration::from_secs(1));
    constraints.measurement_time(Duration::from_secs(10));
    constraints.throughput(Throughput::Elements(SCALING_VEHICLE_COUNT as u64));
    for workload in ConstraintWorkload::ALL {
        benchmark_constraint_modes(&mut constraints, workload, SCALING_VEHICLE_COUNT);
    }
    constraints.finish();

    let mut commands = criterion.benchmark_group("numeric_command_conversion_100k");
    commands.sample_size(10);
    commands.warm_up_time(Duration::from_secs(1));
    commands.measurement_time(Duration::from_secs(10));
    commands.throughput(Throughput::Elements(SCALING_VEHICLE_COUNT as u64));
    benchmark_command_modes(&mut commands, SCALING_VEHICLE_COUNT);
    commands.finish();
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

    if std::env::var_os("LANEFLOW_NUMERIC_BENCH_100K").is_some() {
        let mut group =
            criterion.benchmark_group("numeric_candidate_dense_step_100k_60_observation");
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

    if std::env::var_os("LANEFLOW_NUMERIC_BENCH_EXTENDED").is_some() {
        benchmark_extended_matrix(criterion);
    }
}

criterion_group!(benches, benchmark_candidate_matrix);
criterion_main!(benches);
