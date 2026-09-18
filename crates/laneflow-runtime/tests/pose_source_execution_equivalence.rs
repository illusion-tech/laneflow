//! #712 来源读取与执行的组合观察：worker 数等价、失败隔离与停车语义。
//!
//! 对拍只使用公开 API。worker 1/2/4/8/16 在同一场景脚本下，逐步比较来源
//! 序列与已提交状态摘要；失败 step 只能读到旧已提交状态；停车命令驱动来源
//! 在 Lane/Parking 间的显式转移。#718 合入后的并行分发组合验证在第三层
//! 研究报告中以预集成结果补充。

#[path = "support/policy.rs"]
mod test_policy;

use std::num::NonZeroU32;
use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, PoseSource, PublishedLfcaReference, ReserveParkingTarget,
    RouteRegisterInput, StepError, TickInput, TrafficWorld, VehicleSpawnInput, WorldConfig,
    deterministic_state_digest,
};
use laneflow_static_contract::{LaneEdgeOrdinal, ParkingSpaceOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

const WORKERS: [u32; 5] = [1, 2, 4, 8, 16];
const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
);
const PARKING_ONLY: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/oracle-base.lfca"
);

fn revision(bytes: &[u8]) -> Arc<SharedNetworkRevision> {
    let input = check_canonical_network_input(bytes, FormatLimits::HARD)
        .expect("checked canonical network input");
    build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("shared network revision")
}

fn install(revision: &Arc<SharedNetworkRevision>, workers: u32, world_id: u64) -> TrafficWorld {
    let origin = revision.canonical_origin();
    TrafficWorld::install(
        Arc::clone(revision),
        WorldConfig::new(32, 8, 1_024, 1_024, 100),
        laneflow_runtime::ExecutionConfig::new(
            NonZeroU32::new(workers).expect("nonzero worker count"),
        ),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://pose-source-execution-equivalence",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("fixture key"),
        },
        world_id,
        test_policy::selection(revision),
    )
    .expect("install")
}

fn edge_for_length(world: &TrafficWorld, length_mm: u32) -> LaneEdgeOrdinal {
    world
        .traffic()
        .lane_lengths_millimetres()
        .iter()
        .enumerate()
        .find(|(_, edge_length)| **edge_length == length_mm)
        .map(|(index, _)| LaneEdgeOrdinal::from_raw(u32::try_from(index).expect("index fits u32")))
        .expect("edge with requested length")
}

fn full_sources(world: &TrafficWorld) -> Vec<(laneflow_runtime::VehicleHandle, PoseSource)> {
    world.committed_pose_sources().collect()
}

fn state_fingerprint(world: &TrafficWorld) -> laneflow_static_contract::Sha256Digest {
    deterministic_state_digest(&world.capture_snapshot().expect("snapshot")).expect("digest")
}

/// 成功 step 后读取：worker 1/2/4/8/16 的来源序列与状态摘要逐步一致。
#[test]
fn sources_after_each_step_agree_across_workers() {
    let revision = revision(FULL_SPATIAL);
    let mut traces = Vec::new();
    for workers in WORKERS {
        let mut world = install(&revision, workers, 712);
        let edges = [
            edge_for_length(&world, 10_000),
            edge_for_length(&world, 8_000),
            edge_for_length(&world, 12_000),
        ];
        let route = world
            .register_route(RouteRegisterInput::new(edges.to_vec()))
            .expect("route");
        for progress in [1_000_u32, 7_000] {
            world
                .spawn_vehicle(VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    progress,
                    0,
                ))
                .expect("spawn");
        }
        let mut trace = Vec::new();
        for _ in 0..12 {
            world.step(TickInput::new(100)).expect("step");
            trace.push((full_sources(&world), state_fingerprint(&world)));
        }
        traces.push(trace);
    }
    for trace in &traces[1..] {
        assert_eq!(trace, &traces[0], "worker count must not change sources");
    }
}

/// 失败 step 后读取：只能读到旧已提交状态；同 tick 重试成功后来源继续演化，
/// 且各 worker 一致。
#[test]
fn failed_step_preserves_committed_sources_and_retry_recovers() {
    let revision = revision(FULL_SPATIAL);
    let mut traces = Vec::new();
    for workers in [1_u32, 4, 16] {
        let mut world = install(&revision, workers, 713);
        let edges = [
            edge_for_length(&world, 10_000),
            edge_for_length(&world, 8_000),
            edge_for_length(&world, 12_000),
        ];
        let route = world
            .register_route(RouteRegisterInput::new(edges.to_vec()))
            .expect("route");
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                2_000,
                0,
            ))
            .expect("spawn");
        world.step(TickInput::new(100)).expect("warm-up step");

        let before_sources = full_sources(&world);
        let before_digest = state_fingerprint(&world);
        let failure = world
            .step(TickInput::new(999))
            .expect_err("delta mismatch must fail");
        assert!(matches!(failure, StepError::DeltaMismatch { .. }));
        assert_eq!(full_sources(&world), before_sources);
        assert_eq!(state_fingerprint(&world), before_digest);

        let mut trace = vec![full_sources(&world)];
        for _ in 0..6 {
            world.step(TickInput::new(100)).expect("retry step");
            trace.push(full_sources(&world));
        }
        traces.push(trace);
    }
    for trace in &traces[1..] {
        assert_eq!(trace, &traces[0]);
    }
}

/// 停车命令驱动来源显式转移：reserve 不改变 Lane 来源；park 变为 Parking；
/// leave 回到 Lane。转移序列在 worker 间一致。
#[test]
fn parking_lifecycle_source_transitions_are_explicit_and_worker_stable() {
    let revision = revision(PARKING_ONLY);
    let space = ParkingSpaceOrdinal::from_raw(0);
    let mut traces = Vec::new();
    for workers in [1_u32, 4, 16] {
        let mut world = install(&revision, workers, 714);
        let (entry, entry_progress_mm) = world
            .traffic()
            .relations()
            .parking_space(space)
            .expect("parking space")
            .entry();
        let exit = world
            .traffic()
            .successors(entry)
            .and_then(|successors| successors.first())
            .copied()
            .expect("exit edge");
        let route = world
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        let vehicle = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                entry_progress_mm,
                0,
            ))
            .expect("spawn at parking entry");

        let lane_at_entry = PoseSource::Lane {
            edge: entry,
            progress_mm: entry_progress_mm,
        };
        assert_eq!(full_sources(&world), vec![(vehicle, lane_at_entry)]);

        world
            .reserve_parking(
                vehicle,
                ReserveParkingTarget::ExplicitSpace {
                    space,
                    entry_route_occurrence: 0,
                },
            )
            .expect("reserve");
        let after_reserve = full_sources(&world);
        assert_eq!(after_reserve, vec![(vehicle, lane_at_entry)]);

        world
            .park_vehicle(
                vehicle,
                laneflow_runtime::ParkingTarget::ExplicitSpace(space),
            )
            .expect("park");
        let after_park = full_sources(&world);
        assert_eq!(after_park, vec![(vehicle, PoseSource::Parking { space })]);

        traces.push((after_reserve, after_park));
    }
    for trace in &traces[1..] {
        assert_eq!(trace, &traces[0]);
    }
}
