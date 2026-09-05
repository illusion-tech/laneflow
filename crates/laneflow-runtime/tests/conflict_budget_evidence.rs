//! #569 Conflict 正式固定步进稳态 heap allocation 证据。
//!
//! 两条冲突流从真实 LFCA 完成 install -> register -> spawn -> tick；首次仲裁后，
//! 在一个车辆持有冲突区预约、另一个车辆继续重试的窗口测量固定步进。

#[path = "support/policy.rs"]
mod test_policy;

use std::alloc::System;
use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, PublishedLfcaReference, RouteRegisterInput, TickInput, TrafficWorld,
    VehicleSpawnInput, WorldConfig,
};
use laneflow_static_contract::{ParticipantStreamOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
);
const DELTA_MS: u64 = 4;
const WARM_TICKS: u32 = 8;
const STEADY_TICKS: u32 = 16;

#[test]
fn conflict_steady_tick_has_zero_heap_allocation_after_warmup() {
    let input =
        check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).expect("checked fixture");
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("revision");
    let origin = *revision.canonical_origin();
    let mut world = TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(4, 4, 64, 2, 1, DELTA_MS),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://conflict-budget",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("source"),
        },
        569,
        test_policy::selection(&revision),
    )
    .expect("world");

    let routes = [0_u32, 1].map(|raw| {
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
            .expect("participant stream");
        let edges = revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .expect("maneuver path")
            .edges()
            .to_vec();
        world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route")
    });
    let vehicles = routes.map(|route| {
        let entry = world.route_edges(route).expect("route edges")[0];
        let gate_progress = world.traffic().lane_lengths_millimetres()[entry.index()];
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                gate_progress,
                8_000,
            ))
            .expect("Gate-boundary vehicle")
    });

    world
        .step(TickInput::new(DELTA_MS))
        .expect("warm arbitration");
    assert!(
        vehicles
            .iter()
            .any(|vehicle| world.conflict_reservation(*vehicle).is_some()),
        "warm tick must exercise the formal Conflict reservation path"
    );
    for _ in 0..WARM_TICKS {
        world
            .step(TickInput::new(DELTA_MS))
            .expect("warm steady step");
    }

    let stats = {
        let region = Region::new(GLOBAL);
        for _ in 0..STEADY_TICKS {
            world.step(TickInput::new(DELTA_MS)).expect("steady step");
        }
        region.change()
    };
    assert_eq!(stats.allocations, 0, "steady Conflict ticks allocated");
    assert_eq!(stats.reallocations, 0, "steady Conflict ticks reallocated");
    assert_eq!(
        stats.bytes_allocated, 0,
        "steady Conflict ticks allocated bytes"
    );
    assert!(
        vehicles
            .iter()
            .any(|vehicle| world.conflict_reservation(*vehicle).is_some()),
        "measurement window must remain in the Conflict steady path"
    );
    println!(
        "conflict-g2-allocation-evidence steady_ticks={STEADY_TICKS} allocations={} \
         reallocations={} allocated_bytes={}",
        stats.allocations, stats.reallocations, stats.bytes_allocated
    );

    let mut current = vehicles;
    for sample in 0..STEADY_TICKS + 4 {
        for vehicle in current {
            world.despawn_vehicle(vehicle).unwrap();
        }
        current = routes.map(|route| {
            let edge = world.route_edges(route).unwrap()[0];
            let boundary = world.traffic().lane_lengths_millimetres()[edge.index()];
            world
                .spawn_vehicle(VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    boundary,
                    8_000,
                ))
                .unwrap()
        });
        let region = Region::new(GLOBAL);
        world.step(TickInput::new(DELTA_MS)).unwrap();
        let stats = region.change();
        if sample >= 4 {
            assert_eq!(
                (
                    stats.allocations,
                    stats.reallocations,
                    stats.bytes_allocated
                ),
                (0, 0, 0)
            );
        }
        assert!(
            current
                .iter()
                .any(|vehicle| world.conflict_reservation(*vehicle).is_some())
        );
    }
    println!(
        "conflict-g2-allocation-evidence repeated_acquisition_ticks={STEADY_TICKS} allocations=0 reallocations=0 allocated_bytes=0"
    );
    resource_free_gate_allocation_evidence();
}

// 与上面的分配窗口串行运行，避免全局 allocator 被并发测试污染。
fn resource_free_gate_allocation_evidence() {
    let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).unwrap();
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .unwrap();
    let origin = *revision.canonical_origin();
    let mut world = TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(2, 2, 64, 2, 1, 100),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://gate-evaluation-budget",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .unwrap(),
        },
        570,
        test_policy::selection(&revision),
    )
    .unwrap();
    let edges = revision
        .traffic()
        .maneuvers()
        .maneuver_path(laneflow_static_contract::ManeuverPathOrdinal::from_raw(0))
        .unwrap()
        .edges()
        .to_vec();
    let boundary = revision.traffic().lane_lengths_millimetres()[edges[0].index()];
    let route = world
        .register_route(RouteRegisterInput::new(edges))
        .unwrap();
    let mut observed = [0; 2];
    let mut current = None;
    // path-main 的 release Gate（hop 1）既无 Conflict coverage，也无新 Waiting admission。
    // 经正式 Waiting 生命周期到达该 Gate，分别预热拒绝和放行的两套输出缓冲。
    for _ in 0..1_600 {
        let vehicle = *current.get_or_insert_with(|| {
            world
                .spawn_vehicle(VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    boundary,
                    8_000,
                ))
                .unwrap()
        });
        let region = Region::new(GLOBAL);
        world.step(TickInput::new(100)).unwrap();
        let stats = region.change();
        if let Some(decision) = world
            .latest_conflict_decisions()
            .iter()
            .find(|decision| decision.vehicle() == vehicle && decision.anchor().hop() == 1)
        {
            let index = match decision.outcome() {
                laneflow_runtime::ConflictDecisionOutcome::NotEvaluated => 0,
                laneflow_runtime::ConflictDecisionOutcome::NotRequired => 1,
                outcome => panic!("resource-free Gate outcome: {outcome:?}"),
            };
            observed[index] += 1;
            if observed[index] > 2 {
                assert_eq!(
                    (
                        stats.allocations,
                        stats.reallocations,
                        stats.bytes_allocated
                    ),
                    (0, 0, 0)
                );
            }
        }
        assert!(world.conflict_reservation(vehicle).is_none());
        if world.vehicle(vehicle).unwrap().route_edge_index() > 1 {
            world.despawn_vehicle(vehicle).unwrap();
            current = None;
        }
    }
    assert!(observed.into_iter().all(|count| count > 2));
    println!(
        "gate-evaluation-allocation-evidence denied_ticks={} not_required_ticks={} allocations=0 reallocations=0 allocated_bytes=0",
        observed[0] - 2,
        observed[1] - 2
    );
}
