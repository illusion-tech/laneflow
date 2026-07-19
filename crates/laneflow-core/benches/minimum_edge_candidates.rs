//! #127 最小 edge 长度候选的紧凑多跨界性能基准。

use std::{hint::black_box, time::Duration};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

#[allow(dead_code)]
#[path = "../tests/support/minimum_edge_research.rs"]
mod research;

use research::{compact_transition_kernel, transition_pressure_estimate};

fn benchmark_scale(criterion: &mut Criterion, vehicle_count: usize) {
    let mut group = criterion.benchmark_group(format!(
        "minimum_edge_compact_transition_{vehicle_count}_1tick"
    ));
    group.sample_size(if vehicle_count == 10_000 { 20 } else { 10 });
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(if vehicle_count == 10_000 {
        5
    } else {
        10
    }));
    for min_exclusive in [0.01, 0.1, 1.0] {
        let estimate = transition_pressure_estimate(min_exclusive);
        let crossing_count = estimate
            .crossings_per_vehicle
            .expect("finite minimum candidate must have bounded crossings")
            * vehicle_count as u64;
        group.throughput(Throughput::Elements(crossing_count));
        group.bench_with_input(
            BenchmarkId::new("min_exclusive_meters", min_exclusive),
            &min_exclusive,
            |benchmark, &min_exclusive| {
                benchmark.iter(|| {
                    black_box(compact_transition_kernel(
                        black_box(vehicle_count),
                        black_box(min_exclusive),
                    ))
                });
            },
        );
    }
    group.finish();
}

fn benchmark_minimum_edge_candidates(criterion: &mut Criterion) {
    let exact_positive = transition_pressure_estimate(0.0);
    assert!(exact_positive.crossings_per_vehicle.is_none());
    eprintln!(
        "minimum_edge_control min_exclusive=0 first_valid_f32={} crossings_per_vehicle=unbounded_u64",
        exact_positive.first_valid_edge_length_meters,
    );
    benchmark_scale(criterion, 10_000);
    if std::env::var_os("LANEFLOW_MIN_EDGE_BENCH_100K").is_some() {
        benchmark_scale(criterion, 100_000);
    }
}

criterion_group!(benches, benchmark_minimum_edge_candidates);
criterion_main!(benches);
