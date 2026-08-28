//! #303 Routing G2 具名工作负载的分配/retained 描述性证据。
//!
//! 全局计数分配器与墙钟不能同进程测量；墙钟见 `routing_wall_clock_evidence`。
//! 本测试默认 ignored，CI 只编译；正式取证使用 release、单测试线程显式运行。

use std::alloc::System;
use std::hint::black_box;

use laneflow_runtime::{ObservationExportMode, TickInput};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

mod support {
    pub mod routing_evidence;
}

use support::routing_evidence::{
    EDGE_COUNT, LONG_ROUTE_EDGE_COUNT, ReceiverLimits, TYPICAL_ROUTE_EDGE_COUNT, WORKLOAD_ID,
    build_fixture, fixed_input_sequence_digest, receive_cost_snapshot,
};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const STEADY_TICK_SAMPLES: u32 = 32;
const FIXED_INPUT_SEQUENCE: &str = concat!(
    "fixture:install;step(4ms)x64;observation-full(all);",
    "receive-cost(entries=4096,bytes=98304);open-admission;",
    "measure:step(4ms)x32-warmup;step(4ms)x32-ledger;",
    "observation-full-warmup;observation-full-ledger;",
    "observation-delta-zero-warmup;observation-delta-zero-ledger;",
    "receive-cost-warmup;receive-cost-ledger;",
    "candidate-register-remove(edges=1)-warmup;candidate-register-remove(edges=1)-ledger;",
    "candidate-register-remove(edges=128)-warmup;candidate-register-remove(edges=128)-ledger;",
    "candidate-register-remove(edges=4096)-warmup;candidate-register-remove(edges=4096)-ledger;",
    "step(4ms)x32-ledger;",
);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Ledger {
    allocations: usize,
    reallocations: usize,
    allocated_bytes: usize,
    deallocated_bytes: usize,
    live_delta_bytes: i128,
}

impl Ledger {
    fn from_stats(stats: stats_alloc::Stats) -> Self {
        Self {
            allocations: stats.allocations,
            reallocations: stats.reallocations,
            allocated_bytes: stats.bytes_allocated,
            deallocated_bytes: stats.bytes_deallocated,
            live_delta_bytes: stats.bytes_allocated as i128 - stats.bytes_deallocated as i128,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ObservationLedger {
    allocation: Ledger,
    entry_count: u64,
    logical_bytes: u64,
    retained_bytes: u64,
    session_logical_bytes: u64,
    session_retained_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CandidateLedger {
    allocation: Ledger,
    edge_count: usize,
    caller_input_bytes: usize,
    adjusted_route_live_bytes: i128,
}

fn tick_ledger(fixture: &mut support::routing_evidence::RoutingEvidenceFixture) -> Ledger {
    let region = Region::new(GLOBAL);
    for _ in 0..STEADY_TICK_SAMPLES {
        fixture
            .world
            .step(TickInput::new(support::routing_evidence::DELTA_MS))
            .expect("steady tick");
    }
    Ledger::from_stats(region.change())
}

fn observation_ledger(
    fixture: &mut support::routing_evidence::RoutingEvidenceFixture,
    mode: ObservationExportMode,
) -> ObservationLedger {
    let region = Region::new(GLOBAL);
    let batch = fixture
        .world
        .export_observation(&mut fixture.observation_session, mode)
        .expect("observation evidence batch");
    black_box(&batch);
    let allocation = Ledger::from_stats(region.change());
    ObservationLedger {
        allocation,
        entry_count: batch.entry_count(),
        logical_bytes: batch.logical_bytes(),
        retained_bytes: batch.retained_bytes(),
        session_logical_bytes: fixture
            .observation_session
            .logical_bytes()
            .expect("session logical bytes"),
        session_retained_bytes: fixture
            .observation_session
            .retained_bytes()
            .expect("session retained bytes"),
    }
}

fn receiver_ledger(fixture: &support::routing_evidence::RoutingEvidenceFixture) -> Ledger {
    let region = Region::new(GLOBAL);
    black_box(
        receive_cost_snapshot(
            fixture.cost_binding,
            &fixture.cost_payload,
            ReceiverLimits::EXACT_WORKLOAD,
        )
        .expect("receive cost"),
    );
    Ledger::from_stats(region.change())
}

fn candidate_ledger(
    fixture: &mut support::routing_evidence::RoutingEvidenceFixture,
    edge_count: usize,
) -> CandidateLedger {
    let input = fixture.candidate_input(edge_count);
    let caller_input_bytes = edge_count
        .checked_mul(core::mem::size_of::<laneflow_static_contract::StableId128>())
        .expect("candidate input bytes");
    let region = Region::new(GLOBAL);
    let route = fixture
        .world
        .register_candidate_route(&fixture.admission, input)
        .expect("register candidate route");
    assert_eq!(
        fixture.world.route_edges(route).map(<[_]>::len),
        Some(edge_count)
    );
    let allocation = Ledger::from_stats(region.change());
    let adjusted_route_live_bytes = allocation.live_delta_bytes + caller_input_bytes as i128;
    fixture
        .world
        .remove_route(route)
        .expect("remove measured route");
    CandidateLedger {
        allocation,
        edge_count,
        caller_input_bytes,
        adjusted_route_live_bytes,
    }
}

#[test]
#[ignore = "manual release allocation evidence; CI 只编译具名 Routing workload"]
fn routing_g2_budget_evidence() {
    let mut fixture = build_fixture();
    assert_eq!(fixture.initial_full.entry_count(), EDGE_COUNT as u64);

    black_box(tick_ledger(&mut fixture));
    let steady_before = tick_ledger(&mut fixture);
    assert_eq!(steady_before, Ledger::default());

    black_box(observation_ledger(
        &mut fixture,
        ObservationExportMode::Full,
    ));
    let full = observation_ledger(&mut fixture, ObservationExportMode::Full);
    assert_eq!(full.entry_count, EDGE_COUNT as u64);
    assert!(full.retained_bytes >= full.logical_bytes);

    black_box(observation_ledger(
        &mut fixture,
        ObservationExportMode::Delta,
    ));
    let delta = observation_ledger(&mut fixture, ObservationExportMode::Delta);
    assert_eq!(delta.entry_count, 0);
    assert!(delta.retained_bytes >= delta.logical_bytes);

    black_box(receiver_ledger(&fixture));
    let receiver = receiver_ledger(&fixture);
    assert_eq!(receiver, Ledger::default());

    black_box(candidate_ledger(&mut fixture, 1));
    let candidate_one = candidate_ledger(&mut fixture, 1);
    black_box(candidate_ledger(&mut fixture, TYPICAL_ROUTE_EDGE_COUNT));
    let candidate_typical = candidate_ledger(&mut fixture, TYPICAL_ROUTE_EDGE_COUNT);
    black_box(candidate_ledger(&mut fixture, LONG_ROUTE_EDGE_COUNT));
    let candidate_long = candidate_ledger(&mut fixture, LONG_ROUTE_EDGE_COUNT);
    assert!(candidate_one.adjusted_route_live_bytes > 0);
    assert!(candidate_typical.adjusted_route_live_bytes > candidate_one.adjusted_route_live_bytes);
    assert!(candidate_long.adjusted_route_live_bytes > candidate_typical.adjusted_route_live_bytes);

    let steady_after = tick_ledger(&mut fixture);
    assert_eq!(steady_after, Ledger::default());

    let config = fixture.world.config();
    let observation_set = fixture.cost_binding.observation_set();
    let cost_model = fixture.cost_binding.cost_model();
    println!(
        "routing-g2-budget-evidence workload_id={WORKLOAD_ID} seed={} lfca_exact_bytes={} artifact_digest={:x} topology_network_revision={:x} workload_manifest_digest={:x} state_digest={:x} selection_digest={:x} fixed_input_sequence_digest={:x} edge_count={} world_config_vehicle_capacity={} world_config_route_capacity={} world_config_route_edge_occurrence_capacity={} world_config_worker_count={} world_config_fixed_delta_ms={} binding_version={} observation_tick={} observation_state_sequence={} observation_set_digest={:x} cost_model_id={:x} cost_model_version={} valid_through_tick={} snapshot_sha256={:x} cost_entry_count={} cost_exact_bytes={} receiver_limits_entries={} receiver_limits_bytes={} steady_tick_samples={} steady_before={steady_before:?} observation_full={full:?} observation_delta_zero={delta:?} receiver={receiver:?} candidate_one={candidate_one:?} candidate_typical={candidate_typical:?} candidate_long={candidate_long:?} steady_after={steady_after:?}",
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
        STEADY_TICK_SAMPLES,
    );
}
