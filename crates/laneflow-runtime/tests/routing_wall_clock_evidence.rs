//! #303 Routing G2 具名工作负载的 wall-clock 描述性证据（未插桩）。
//!
//! 全局计数分配器会污染墙钟，因此本二进制不安装分配器；分配/retained
//! 证据见 `routing_budget_evidence`。计时只覆盖被测调用，输入构造、结果
//! 析构以及候选 route 的移除均在计时区间外。
//!
//! 本测试默认 ignored，CI 只编译。正式取证使用 release、单测试线程，
//! 并以三个 fresh process 独立运行；这些数字只描述具名初始 case，不是
//! LF-SYNTH W1-W4 Product Pass。

use std::hint::black_box;
use std::time::Instant;

use laneflow_runtime::{ObservationExportMode, TickInput};

mod support {
    pub mod routing_evidence;
}

use support::routing_evidence::{
    DELTA_MS, EDGE_COUNT, LONG_ROUTE_EDGE_COUNT, ReceiverLimits, TYPICAL_ROUTE_EDGE_COUNT,
    WORKLOAD_ID, build_fixture, fixed_input_sequence_digest, receive_cost_snapshot,
};

const SAMPLE_WARMUP: usize = 3;
const SAMPLE_COUNT: usize = 21;
const FIXED_INPUT_SEQUENCE: &str = concat!(
    "fixture:install;step(4ms)x64;observation-full(all);",
    "receive-cost(entries=4096,bytes=98304);open-admission;",
    "measure-each(warmup=3,samples=21):step(4ms);observation-full;",
    "observation-delta-zero;receive-cost;candidate-register-remove(edges=1);",
    "candidate-register-remove(edges=128);candidate-register-remove(edges=4096);step(4ms);",
);

#[derive(Debug)]
struct ClockStats {
    raw_ns: Vec<u128>,
    median_ns: u128,
    mad_ns: u128,
    p95_ns: u128,
    p99_ns: u128,
    max_ns: u128,
}

impl ClockStats {
    fn from_raw(raw_ns: Vec<u128>) -> Self {
        assert_eq!(raw_ns.len(), SAMPLE_COUNT);
        let mut sorted = raw_ns.clone();
        sorted.sort_unstable();
        let median_ns = percentile_nearest_rank(&sorted, 50);
        let mut deviations: Vec<_> = sorted
            .iter()
            .map(|value| value.abs_diff(median_ns))
            .collect();
        deviations.sort_unstable();
        Self {
            median_ns,
            mad_ns: percentile_nearest_rank(&deviations, 50),
            p95_ns: percentile_nearest_rank(&sorted, 95),
            p99_ns: percentile_nearest_rank(&sorted, 99),
            max_ns: *sorted.last().expect("non-empty samples"),
            raw_ns,
        }
    }
}

fn percentile_nearest_rank(sorted: &[u128], percentile: usize) -> u128 {
    assert!(!sorted.is_empty());
    assert!((1..=100).contains(&percentile));
    let rank = sorted
        .len()
        .checked_mul(percentile)
        .and_then(|value| value.checked_add(99))
        .expect("percentile rank")
        / 100;
    sorted[rank.saturating_sub(1)]
}

fn measure<T>(mut operation: impl FnMut() -> T) -> ClockStats {
    measure_validated(&mut operation, |_| {})
}

fn measure_validated<T>(
    mut operation: impl FnMut() -> T,
    mut validate: impl FnMut(&T),
) -> ClockStats {
    let mut raw_ns = Vec::with_capacity(SAMPLE_COUNT);
    for index in 0..(SAMPLE_WARMUP + SAMPLE_COUNT) {
        let started = Instant::now();
        let output = operation();
        let elapsed = started.elapsed().as_nanos();
        validate(&output);
        black_box(&output);
        drop(output);
        if index >= SAMPLE_WARMUP {
            raw_ns.push(elapsed);
        }
    }
    ClockStats::from_raw(raw_ns)
}

fn print_stats(name: &str, stats: &ClockStats) {
    let raw = stats
        .raw_ns
        .iter()
        .map(u128::to_string)
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "routing-g2-wall-clock operation={name} samples={} median_ns={} mad_ns={} p95_ns={} p99_ns={} max_ns={} raw_ns={raw}",
        stats.raw_ns.len(),
        stats.median_ns,
        stats.mad_ns,
        stats.p95_ns,
        stats.p99_ns,
        stats.max_ns,
    );
}

#[test]
#[ignore = "manual release wall-clock evidence; CI 不当 Routing 产品基线"]
fn routing_g2_wall_clock_evidence() {
    let mut fixture = build_fixture();
    assert_eq!(fixture.initial_full.entry_count(), EDGE_COUNT as u64);

    let steady_before = measure(|| {
        fixture
            .world
            .step(TickInput::new(DELTA_MS))
            .expect("steady tick before")
    });

    let observation_full = measure_validated(
        || {
            fixture
                .world
                .export_observation(
                    &mut fixture.observation_session,
                    ObservationExportMode::Full,
                )
                .expect("full observation")
        },
        |batch| assert_eq!(batch.entry_count(), EDGE_COUNT as u64),
    );

    let observation_delta_zero = measure_validated(
        || {
            fixture
                .world
                .export_observation(
                    &mut fixture.observation_session,
                    ObservationExportMode::Delta,
                )
                .expect("zero-change observation")
        },
        |batch| assert_eq!(batch.entry_count(), 0),
    );

    let receiver = measure(|| {
        receive_cost_snapshot(
            fixture.cost_binding,
            &fixture.cost_payload,
            ReceiverLimits::EXACT_WORKLOAD,
        )
        .expect("receive cost snapshot")
    });

    let candidate_one = measure_candidate(&mut fixture, 1);
    let candidate_typical = measure_candidate(&mut fixture, TYPICAL_ROUTE_EDGE_COUNT);
    let candidate_long = measure_candidate(&mut fixture, LONG_ROUTE_EDGE_COUNT);

    let steady_after = measure(|| {
        fixture
            .world
            .step(TickInput::new(DELTA_MS))
            .expect("steady tick after")
    });

    let config = fixture.world.config();
    let observation_set = fixture.cost_binding.observation_set();
    let cost_model = fixture.cost_binding.cost_model();
    println!(
        "routing-g2-wall-clock workload_id={WORKLOAD_ID} seed={} lfca_exact_bytes={} artifact_digest={:x} topology_network_revision={:x} workload_manifest_digest={:x} state_digest={:x} selection_digest={:x} fixed_input_sequence_digest={:x} edge_count={} world_config_vehicle_capacity={} world_config_route_capacity={} world_config_route_edge_occurrence_capacity={} world_config_worker_count={} world_config_fixed_delta_ms={} binding_version={} observation_tick={} observation_state_sequence={} observation_set_digest={:x} cost_model_id={:x} cost_model_version={} valid_through_tick={} snapshot_sha256={:x} cost_entry_count={} cost_exact_bytes={} receiver_limits_entries={} receiver_limits_bytes={} sample_warmup={} samples={} classification=descriptive-initial-not-product-pass",
        support::routing_evidence::WORKLOAD_SEED,
        fixture.lfca_exact_bytes,
        fixture.artifact_digest,
        fixture.network_revision.as_digest(),
        fixture.workload_manifest_digest,
        fixture.state_digest,
        fixture.initial_full.selection_digest(),
        fixed_input_sequence_digest(FIXED_INPUT_SEQUENCE),
        EDGE_COUNT,
        config.vehicle_capacity(),
        config.route_capacity(),
        config.route_edge_occurrence_capacity(),
        config.worker_count(),
        config.fixed_delta_time_ms(),
        fixture.cost_binding.binding_version(),
        observation_set.observation_tick(),
        observation_set.observation_state_sequence().get(),
        observation_set.digest(),
        cost_model.model_id(),
        cost_model.model_version(),
        fixture.cost_binding.valid_through_tick(),
        fixture.cost_binding.snapshot_sha256(),
        fixture.cost_binding.entry_count(),
        fixture.cost_binding.exact_byte_length(),
        ReceiverLimits::EXACT_WORKLOAD.max_entry_count,
        ReceiverLimits::EXACT_WORKLOAD.max_exact_byte_length,
        SAMPLE_WARMUP,
        SAMPLE_COUNT,
    );
    print_stats("steady_tick_before", &steady_before);
    print_stats("observation_full", &observation_full);
    print_stats("observation_delta_zero", &observation_delta_zero);
    print_stats("receiver", &receiver);
    print_stats("candidate_register_1", &candidate_one);
    print_stats("candidate_register_128", &candidate_typical);
    print_stats("candidate_register_4096", &candidate_long);
    print_stats("steady_tick_after", &steady_after);
}

fn measure_candidate(
    fixture: &mut support::routing_evidence::RoutingEvidenceFixture,
    edge_count: usize,
) -> ClockStats {
    let mut raw_ns = Vec::with_capacity(SAMPLE_COUNT);
    for index in 0..(SAMPLE_WARMUP + SAMPLE_COUNT) {
        let input = fixture.candidate_input(edge_count);
        let started = Instant::now();
        let route = fixture
            .world
            .register_candidate_route(&fixture.admission, input)
            .expect("register candidate route");
        let elapsed = started.elapsed().as_nanos();
        black_box(route);
        assert_eq!(
            fixture.world.route_edges(route).map(<[_]>::len),
            Some(edge_count)
        );
        fixture
            .world
            .remove_route(route)
            .expect("remove measured route");
        if index >= SAMPLE_WARMUP {
            raw_ns.push(elapsed);
        }
    }
    ClockStats::from_raw(raw_ns)
}
