#[allow(dead_code)]
#[path = "support/route_distance_candidates.rs"]
mod candidates;

use candidates::{
    CHUNK_OCCURRENCES, ChunkedLocalF32Index, F64PrefixIndex, QueryResult,
    collection_retained_bytes, repeated_route_indices,
};

const ROUTE_DISTANCE_BUDGET_METERS: f64 = 0.01;
const ROUTE_HEAVY_CURRENT_COMPLETE_10K_BYTES: usize = 14_918_671;
const ROUTE_HEAVY_CURRENT_COMPLETE_100K_BYTES: usize = 141_001_223;
const ROUTE_HEAVY_CURRENT_ROUTE_10K_BYTES: usize = 3_212_416;
const ROUTE_HEAVY_CURRENT_ROUTE_100K_BYTES: usize = 32_105_728;

#[derive(Clone, Copy)]
struct Workload {
    name: &'static str,
    route_count: usize,
    occurrences_per_route: usize,
    edge_length: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct ErrorSummary {
    f64_prefix_max_meters: f64,
    chunked_max_meters: f64,
    discrete_divergences: usize,
}

#[test]
fn candidates_match_f64_oracle_within_route_distance_budget() {
    let lengths = varied_lengths(4_096);
    let f64_prefix = F64PrefixIndex::build(&lengths);
    let chunked = ChunkedLocalF32Index::build(&lengths);
    let prefixes = oracle_prefixes(&lengths);
    let mut random = DeterministicRandom::new(0x127_0014_0141_0127);
    let mut summary = ErrorSummary::default();

    for _ in 0..100_000 {
        let from = random.index(lengths.len());
        let target = random.index(lengths.len());
        let from_progress = random.progress(lengths[from]);
        let target_progress = random.progress(lengths[target]);
        let exact = oracle_distance(&prefixes, from, from_progress, target, target_progress);
        let horizon = match exact {
            QueryResult::Within(distance) if random.next_u64() & 1 == 0 => distance + 0.02,
            QueryResult::Within(distance) => (distance - 0.02).max(0.0),
            QueryResult::Passed | QueryResult::BeyondHorizon => random.unit() * 1_000_000.0,
        };
        let expected = apply_horizon(exact, horizon);
        let f64_actual =
            f64_prefix.distance_within(from, from_progress, target, target_progress, horizon);
        let chunked_actual =
            chunked.distance_within(from, from_progress, target, target_progress, horizon);

        summary.discrete_divergences += usize::from(class_of(f64_actual) != class_of(expected));
        summary.discrete_divergences += usize::from(class_of(chunked_actual) != class_of(expected));
        if let (QueryResult::Within(expected), QueryResult::Within(actual)) = (expected, f64_actual)
        {
            summary.f64_prefix_max_meters =
                summary.f64_prefix_max_meters.max((actual - expected).abs());
        }
        if let (QueryResult::Within(expected), QueryResult::Within(actual)) =
            (expected, chunked_actual)
        {
            summary.chunked_max_meters = summary.chunked_max_meters.max((actual - expected).abs());
        }
    }

    println!(
        "route-distance-errors f64_prefix_max_m={:.12} chunked_max_m={:.12} discrete_divergences={}",
        summary.f64_prefix_max_meters, summary.chunked_max_meters, summary.discrete_divergences
    );
    assert_eq!(summary.discrete_divergences, 0);
    assert!(summary.f64_prefix_max_meters <= ROUTE_DISTANCE_BUDGET_METERS);
    assert!(summary.chunked_max_meters <= ROUTE_DISTANCE_BUDGET_METERS);
    assert!(chunked.max_local_offset_error(&lengths) <= 0.003_906_25);
}

#[test]
fn exact_boundaries_and_forced_segments_preserve_classification() {
    let lengths = vec![10.0_f32; 30];
    let f64_prefix = F64PrefixIndex::build_with_segment_occurrences(&lengths, 2);
    let chunked = ChunkedLocalF32Index::build_with_segment_chunks(&lengths, 1);
    assert_eq!(
        chunked.chunk_count(),
        lengths.len().div_ceil(CHUNK_OCCURRENCES)
    );
    assert_eq!(chunked.total_chunk_length(), 300.0);

    for candidate in [
        f64_prefix.distance_within(1, 2.0, 28, 7.0, 275.0),
        chunked.distance_within(1, 2.0, 28, 7.0, 275.0),
    ] {
        assert_eq!(candidate, QueryResult::Within(275.0));
    }
    for candidate in [
        f64_prefix.distance_within(1, 2.0, 28, 7.0, 275.0_f64.next_down()),
        chunked.distance_within(1, 2.0, 28, 7.0, 275.0_f64.next_down()),
    ] {
        assert_eq!(candidate, QueryResult::BeyondHorizon);
    }
    for candidate in [
        f64_prefix.distance_within(5, 3.0, 5, 2.0, f64::MAX),
        chunked.distance_within(5, 3.0, 5, 2.0, f64::MAX),
    ] {
        assert_eq!(candidate, QueryResult::Passed);
    }
    for candidate in [
        f64_prefix.distance_to_end_within(1, 2.0, 288.0),
        chunked.distance_to_end_within(1, 2.0, 288.0),
    ] {
        assert_eq!(candidate, QueryResult::Within(288.0));
    }
}

#[test]
fn route_candidate_memory_contribution_is_measured() {
    for scale in [10_000, 100_000] {
        for workload in workloads(scale) {
            let (f64_indices, chunked_indices) = repeated_route_indices(
                workload.route_count,
                workload.occurrences_per_route,
                workload.edge_length,
            );
            let f64_bytes = collection_retained_bytes(&f64_indices, F64PrefixIndex::retained_bytes);
            let chunked_bytes =
                collection_retained_bytes(&chunked_indices, ChunkedLocalF32Index::retained_bytes);
            println!(
                "route-distance-memory scale={} workload={} f64_prefix_bytes={} chunked_bytes={} chunked_vs_f64={:.6}",
                scale,
                workload.name,
                f64_bytes,
                chunked_bytes,
                ratio(chunked_bytes, f64_bytes)
            );
            assert!(f64_bytes > 0);
            assert!(chunked_bytes > 0);
        }
    }

    for (scale, current_complete, current_route) in [
        (
            10_000,
            ROUTE_HEAVY_CURRENT_COMPLETE_10K_BYTES,
            ROUTE_HEAVY_CURRENT_ROUTE_10K_BYTES,
        ),
        (
            100_000,
            ROUTE_HEAVY_CURRENT_COMPLETE_100K_BYTES,
            ROUTE_HEAVY_CURRENT_ROUTE_100K_BYTES,
        ),
    ] {
        let route_count = scale / 100;
        let (f64_indices, chunked_indices) = repeated_route_indices(route_count, 1_000, 10_000.0);
        let f64_route = collection_retained_bytes(&f64_indices, F64PrefixIndex::retained_bytes);
        let chunked_route =
            collection_retained_bytes(&chunked_indices, ChunkedLocalF32Index::retained_bytes);
        let f64_complete = current_complete - current_route + f64_route;
        let chunked_complete = current_complete - current_route + chunked_route;
        let reduction = 1.0 - ratio(chunked_complete, f64_complete);
        println!(
            "route-heavy-complete scale={} f64_prefix_bytes={} chunked_bytes={} chunked_reduction={:.6}",
            scale, f64_complete, chunked_complete, reduction
        );
        assert!(
            reduction > 0.0,
            "route-heavy chunked candidate should retain its measured memory benefit"
        );
    }
}

fn workloads(scale: usize) -> [Workload; 4] {
    [
        Workload {
            name: "vehicle-heavy",
            route_count: 1,
            occurrences_per_route: scale.div_ceil(1_000),
            edge_length: 10_000.0,
        },
        Workload {
            name: "route-heavy",
            route_count: scale / 100,
            occurrences_per_route: 1_000,
            edge_length: 10_000.0,
        },
        Workload {
            name: "balanced",
            route_count: scale / 25,
            occurrences_per_route: 25,
            edge_length: 200.0,
        },
        Workload {
            name: "signal-heavy",
            route_count: scale,
            occurrences_per_route: 2,
            edge_length: 200.0,
        },
    ]
}

fn varied_lengths(count: usize) -> Vec<f32> {
    let mut random = DeterministicRandom::new(0x127_0014_0141_0127);
    (0..count)
        .map(|index| match index % 8 {
            0 => 1.0_f32.next_up(),
            1 => 10_000.0,
            2 => 1_234.567_7,
            _ => 1.001 + (random.next_u64() % 9_998_999) as f32 / 1_000.0,
        })
        .collect()
}

fn oracle_prefixes(lengths: &[f32]) -> Vec<f64> {
    let mut total = 0.0;
    lengths
        .iter()
        .copied()
        .map(|length| {
            let prefix = total;
            total += f64::from(length);
            prefix
        })
        .collect()
}

fn oracle_distance(
    prefixes: &[f64],
    from: usize,
    from_progress: f64,
    target: usize,
    target_progress: f64,
) -> QueryResult {
    if target < from || (target == from && target_progress < from_progress) {
        QueryResult::Passed
    } else {
        QueryResult::Within((prefixes[target] + target_progress) - (prefixes[from] + from_progress))
    }
}

fn apply_horizon(result: QueryResult, horizon: f64) -> QueryResult {
    match result {
        QueryResult::Within(distance) if distance <= horizon => QueryResult::Within(distance),
        QueryResult::Within(_) => QueryResult::BeyondHorizon,
        QueryResult::Passed | QueryResult::BeyondHorizon => result,
    }
}

fn class_of(result: QueryResult) -> u8 {
    match result {
        QueryResult::Passed => 0,
        QueryResult::BeyondHorizon => 1,
        QueryResult::Within(_) => 2,
    }
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    numerator as f64 / denominator as f64
}

struct DeterministicRandom(u64);

impl DeterministicRandom {
    const fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn index(&mut self, len: usize) -> usize {
        self.next_u64() as usize % len
    }

    fn unit(&mut self) -> f64 {
        self.next_u64() as f64 / u64::MAX as f64
    }

    fn progress(&mut self, edge_length: f32) -> f64 {
        self.unit() * f64::from(edge_length)
    }
}
