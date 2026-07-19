//! #127 路线派生距离候选的构造与 O(1) 查询性能基准。

use std::{hint::black_box, time::Duration};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

#[allow(dead_code)]
#[path = "../tests/support/route_distance_candidates.rs"]
mod candidates;

use candidates::{ChunkedLocalF32Index, F64PrefixIndex, QueryResult};

const ROUTE_COUNT: usize = 100;
const OCCURRENCES_PER_ROUTE: usize = 1_000;
const QUERY_COUNT: usize = 16_384;

#[derive(Clone, Copy)]
struct Query {
    from_occurrence: usize,
    from_progress: f64,
    target_occurrence: usize,
    target_progress: f64,
    horizon: f64,
}

fn benchmark_build(criterion: &mut Criterion) {
    let lengths = vec![10_000.0_f32; OCCURRENCES_PER_ROUTE];
    let mut group = criterion.benchmark_group("route_distance_build_100_routes_x1000");
    group.sample_size(20);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(5));
    group.throughput(Throughput::Elements(
        (ROUTE_COUNT * OCCURRENCES_PER_ROUTE) as u64,
    ));
    group.bench_function(BenchmarkId::new("layout", "f64_prefix"), |benchmark| {
        benchmark.iter(|| {
            black_box(
                (0..ROUTE_COUNT)
                    .map(|_| F64PrefixIndex::build(black_box(&lengths)))
                    .collect::<Vec<_>>(),
            )
        });
    });
    group.bench_function(
        BenchmarkId::new("layout", "chunked_local_f32"),
        |benchmark| {
            benchmark.iter(|| {
                black_box(
                    (0..ROUTE_COUNT)
                        .map(|_| ChunkedLocalF32Index::build(black_box(&lengths)))
                        .collect::<Vec<_>>(),
                )
            });
        },
    );
    group.finish();
}

fn benchmark_query(criterion: &mut Criterion) {
    let lengths = vec![10_000.0_f32; OCCURRENCES_PER_ROUTE];
    let f64_prefix = F64PrefixIndex::build(&lengths);
    let chunked = ChunkedLocalF32Index::build(&lengths);
    let queries = query_matrix();
    let mut group = criterion.benchmark_group("route_distance_query_16384");
    group.sample_size(30);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(5));
    group.throughput(Throughput::Elements(QUERY_COUNT as u64));
    group.bench_function(BenchmarkId::new("layout", "f64_prefix"), |benchmark| {
        benchmark.iter(|| run_queries(black_box(&f64_prefix), black_box(&queries)));
    });
    group.bench_function(
        BenchmarkId::new("layout", "chunked_local_f32"),
        |benchmark| {
            benchmark.iter(|| run_queries(black_box(&chunked), black_box(&queries)));
        },
    );
    group.finish();
}

fn run_queries<I: RouteDistanceQuery>(index: &I, queries: &[Query]) -> u64 {
    queries.iter().copied().fold(0_u64, |checksum, query| {
        checksum.rotate_left(7)
            ^ result_bits(index.distance_within(
                query.from_occurrence,
                query.from_progress,
                query.target_occurrence,
                query.target_progress,
                query.horizon,
            ))
    })
}

trait RouteDistanceQuery {
    fn distance_within(
        &self,
        from_occurrence: usize,
        from_progress: f64,
        target_occurrence: usize,
        target_progress: f64,
        horizon: f64,
    ) -> QueryResult;
}

impl RouteDistanceQuery for F64PrefixIndex {
    fn distance_within(
        &self,
        from_occurrence: usize,
        from_progress: f64,
        target_occurrence: usize,
        target_progress: f64,
        horizon: f64,
    ) -> QueryResult {
        self.distance_within(
            from_occurrence,
            from_progress,
            target_occurrence,
            target_progress,
            horizon,
        )
    }
}

impl RouteDistanceQuery for ChunkedLocalF32Index {
    fn distance_within(
        &self,
        from_occurrence: usize,
        from_progress: f64,
        target_occurrence: usize,
        target_progress: f64,
        horizon: f64,
    ) -> QueryResult {
        self.distance_within(
            from_occurrence,
            from_progress,
            target_occurrence,
            target_progress,
            horizon,
        )
    }
}

fn result_bits(result: QueryResult) -> u64 {
    match result {
        QueryResult::Passed => 0x5041_5353_4544,
        QueryResult::BeyondHorizon => 0x4245_594f_4e44,
        QueryResult::Within(distance) => distance.to_bits(),
    }
}

fn query_matrix() -> Vec<Query> {
    (0..QUERY_COUNT)
        .map(|index| {
            let from = index.wrapping_mul(37) % OCCURRENCES_PER_ROUTE;
            let remaining = OCCURRENCES_PER_ROUTE - from;
            let forward = index.wrapping_mul(97) % remaining;
            let target = from + forward;
            let from_progress = (index.wrapping_mul(131) % 10_000) as f64;
            let target_progress = (index.wrapping_mul(193) % 10_000) as f64;
            match index % 3 {
                0 => Query {
                    from_occurrence: from,
                    from_progress,
                    target_occurrence: target,
                    target_progress,
                    horizon: f64::MAX,
                },
                1 => Query {
                    from_occurrence: from,
                    from_progress,
                    target_occurrence: target,
                    target_progress,
                    horizon: 100.0,
                },
                _ => Query {
                    from_occurrence: from,
                    from_progress,
                    target_occurrence: from.saturating_sub(1),
                    target_progress,
                    horizon: f64::MAX,
                },
            }
        })
        .collect()
}

fn benchmark_candidates(criterion: &mut Criterion) {
    benchmark_build(criterion);
    benchmark_query(criterion);
}

criterion_group!(benches, benchmark_candidates);
criterion_main!(benches);
