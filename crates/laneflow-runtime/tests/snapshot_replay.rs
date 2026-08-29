//! #512 检查点回放身份与逐点摘要证据。
//!
//! Runtime 不拥有宿主输入日志。本测试只用公开 API 模拟宿主：检查点已有实体经
//! `snapshot local ID -> restored handle` 映射重绑；检查点后新增实体的耐久 ID 由
//! 命令载荷拥有；候选准入结果按稳定边序列经 `register_admitted_route` 重放。

use std::{collections::BTreeMap, sync::Arc};

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    AdmittedRouteRegisterInput, CommittedNetworkSource, PublishedLfcaReference, RestoredSnapshot,
    RouteError, RouteHandle, RouteRegisterInput, SnapshotRestoreLimits, TickInput, TrafficWorld,
    VehicleHandle, VehicleSpawnInput, WorldConfig, deterministic_state_digest, encode_lfrs,
    restore_lfrs,
};
use laneflow_static_contract::{
    LaneEdgeOrdinal, ParkingSpaceId, ParkingSpaceOrdinal, Sha256Digest, VehicleProfileId,
    VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReplayPoint {
    command_cursor: u64,
    tick: u64,
    digest: Sha256Digest,
}

#[derive(Default)]
struct HostIdentityMap {
    routes: BTreeMap<u64, RouteHandle>,
    vehicles: BTreeMap<u64, VehicleHandle>,
}

const CHECKPOINT_ROUTE_ID: u64 = 10_001;
const CHECKPOINT_VEHICLE_ID: u64 = 20_001;
const REPLAY_ROUTE_ID: u64 = 10_002;
const REPLAY_VEHICLE_ID: u64 = 20_002;

fn revision() -> Arc<SharedNetworkRevision> {
    let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
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

fn world() -> TrafficWorld {
    let revision = revision();
    let origin = *revision.canonical_origin();
    TrafficWorld::install(
        revision,
        WorldConfig::new(8, 4, 1_024, 1, 100),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://snapshot-replay",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("published fixture reference"),
        },
        77,
    )
    .expect("install")
}

fn edge_for_length(world: &TrafficWorld, length: u32) -> LaneEdgeOrdinal {
    let index = world
        .traffic()
        .lane_lengths_millimetres()
        .iter()
        .position(|actual| *actual == length)
        .expect("fixture lane length");
    LaneEdgeOrdinal::try_from_usize(index).expect("fixture lane ordinal")
}

fn fixture_edges(world: &TrafficWorld) -> Vec<LaneEdgeOrdinal> {
    vec![
        edge_for_length(world, 10_000),
        edge_for_length(world, 8_000),
        edge_for_length(world, 12_000),
    ]
}

fn capture_point(world: &TrafficWorld) -> ReplayPoint {
    let snapshot = world.capture_snapshot().expect("capture");
    ReplayPoint {
        command_cursor: snapshot.command_cursor(),
        tick: snapshot.tick(),
        digest: deterministic_state_digest(&snapshot).expect("digest"),
    }
}

fn replay_suffix(
    world: &mut TrafficWorld,
    host_ids: &mut HostIdentityMap,
    lane_edges: &[laneflow_static_contract::StableId128],
    parking_id: ParkingSpaceId,
    profile_id: VehicleProfileId,
    spawn_progress_mm: u32,
) -> Vec<ReplayPoint> {
    let checkpoint_vehicle = host_ids.vehicles[&CHECKPOINT_VEHICLE_ID];
    let parking = world
        .revision()
        .identity()
        .ordinal(parking_id)
        .expect("durable parking ID resolves in the bound revision");
    world
        .occupy_parking(checkpoint_vehicle, parking)
        .expect("park checkpoint vehicle");
    let mut points = vec![capture_point(world)];

    let revision = world.revision();
    let origin = revision.canonical_origin();
    let replay_route = world
        .register_admitted_route(AdmittedRouteRegisterInput::new(
            origin.network_revision(),
            origin
                .static_contract_versions()
                .network_revision_derivation_version(),
            lane_edges.to_vec(),
        ))
        .expect("replay admitted route without Routing provenance");
    assert!(
        host_ids
            .routes
            .insert(REPLAY_ROUTE_ID, replay_route)
            .is_none()
    );
    points.push(capture_point(world));

    let profile = world
        .revision()
        .identity()
        .ordinal(profile_id)
        .expect("durable profile ID resolves in the bound revision");
    let replay_vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            profile,
            host_ids.routes[&REPLAY_ROUTE_ID],
            0,
            spawn_progress_mm,
            0,
        ))
        .expect("spawn replay-owned vehicle");
    assert!(
        host_ids
            .vehicles
            .insert(REPLAY_VEHICLE_ID, replay_vehicle)
            .is_none()
    );
    points.push(capture_point(world));

    world.step(TickInput::new(100)).expect("replay tick");
    points.push(capture_point(world));
    points
}

fn restored_checkpoint(checkpoint_bytes: &[u8], original: &TrafficWorld) -> RestoredSnapshot {
    restore_lfrs(
        checkpoint_bytes,
        original.revision(),
        original.committed_source().clone(),
        original.config(),
        SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
    )
    .expect("restore checkpoint")
}

fn first_divergence_interval(
    checkpoint: ReplayPoint,
    expected: &[ReplayPoint],
    actual: &[ReplayPoint],
) -> Option<((u64, u64), (u64, u64))> {
    expected
        .iter()
        .zip(actual)
        .enumerate()
        .find(|(_, (expected_point, actual_point))| expected_point != actual_point)
        .map(|(index, (expected_point, _))| {
            let matched = index
                .checked_sub(1)
                .map_or(checkpoint, |previous| expected[previous]);
            (
                (matched.command_cursor, matched.tick),
                (expected_point.command_cursor, expected_point.tick),
            )
        })
}

#[test]
fn replay_divergence_under_capacity_mismatch_is_a_desync_signal() {
    let mut original = world();
    let edges = fixture_edges(&original);
    // 填满原配置路线容量（4/4）并放一辆检查点车辆。
    let mut routes = Vec::new();
    for _ in 0..4 {
        routes.push(
            original
                .register_route(RouteRegisterInput::new(edges.clone()))
                .expect("route within original capacity"),
        );
    }
    original
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            routes[0],
            0,
            0,
            0,
        ))
        .expect("checkpoint vehicle");
    let checkpoint = original.capture_snapshot().expect("capture");
    let checkpoint_point = ReplayPoint {
        command_cursor: checkpoint.command_cursor(),
        tick: checkpoint.tick(),
        digest: deterministic_state_digest(&checkpoint).expect("digest"),
    };
    let cursor = checkpoint.command_cursor();
    let bytes = encode_lfrs(&checkpoint);

    // 宿主按 §2 判据以放大语义容量恢复（只许放大，允许；dt/occurrence 不变）。
    let restored = restore_lfrs(
        &bytes,
        original.revision(),
        original.committed_source().clone(),
        WorldConfig::new(8, 8, 1_024, 1, 100),
        SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
    )
    .expect("restore with enlarged route capacity");
    let mut replayed = restored.into_world();

    // 恢复时刻即可检测：语义容量是摘要输入，放大容量的恢复世界与原
    // 检查点摘要已经不同——「容量不同不冒充 exact replay」在恢复边界
    // 即成立，分歧先于任何重放命令存在。
    let restored_checkpoint = capture_point(&replayed);
    assert_ne!(restored_checkpoint, checkpoint_point);
    // 因此首个分歧区间就是检查点本身：两侧流此前没有相等点可锚定，
    // 定位器把检查点作为参照原点报告零宽区间。
    assert_eq!(
        first_divergence_interval(
            checkpoint_point,
            &[checkpoint_point],
            &[restored_checkpoint]
        ),
        Some((
            (cursor, checkpoint_point.tick),
            (cursor, checkpoint_point.tick)
        ))
    );

    // 同一条注册命令：原世界容量已满被拒（命令未应用、游标不动），
    // 放大配置的重放世界成功（游标推进）。§2：容量差异改变重放中生命
    // 周期命令的成败，分歧按失同步信号处理，不判为实现缺陷、不冒充
    // exact replay。
    let rejected = original.register_route(RouteRegisterInput::new(edges.clone()));
    assert!(matches!(rejected, Err(RouteError::CapacityExceeded)));
    let expected = vec![capture_point(&original)];

    replayed
        .register_route(RouteRegisterInput::new(edges))
        .expect("enlarged capacity accepts the same command");
    let actual = vec![capture_point(&replayed)];

    assert_eq!(original.command_cursor(), cursor);
    assert_eq!(replayed.command_cursor(), cursor + 1);
    // 首个分歧区间锚定在检查点（最后一个相等点），分歧点即容量边界
    // 命令之后的对拍点（期望侧命令未应用，游标停在检查点值）；锚点取
    // 恢复侧检查点——它是恢复侧流的真实最后已知点，两侧流没有更早的
    // 相等点。
    assert_eq!(
        first_divergence_interval(restored_checkpoint, &expected, &actual),
        Some((
            (cursor, checkpoint_point.tick),
            (cursor, checkpoint_point.tick)
        ))
    );
}

#[test]
fn checkpoint_replay_is_pointwise_equal_and_locates_first_desync_interval() {
    let mut original = world();
    let edges = fixture_edges(&original);
    let route = original
        .register_route(RouteRegisterInput::new(edges.clone()))
        .expect("checkpoint route");
    let vehicle = original
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            0,
        ))
        .expect("checkpoint vehicle");
    let checkpoint = original.capture_snapshot().expect("capture");
    let checkpoint_point = ReplayPoint {
        command_cursor: checkpoint.command_cursor(),
        tick: checkpoint.tick(),
        digest: deterministic_state_digest(&checkpoint).expect("digest"),
    };
    let checkpoint_route_id = checkpoint.routes()[0].snapshot_route_id();
    let checkpoint_vehicle_id = checkpoint.vehicles()[0].snapshot_vehicle_id();
    let stable_edges = checkpoint.routes()[0].edges().to_vec();
    let revision = original.revision();
    let identity = revision.identity();
    let parking_id = identity
        .stable_id(ParkingSpaceOrdinal::from_raw(0))
        .expect("parking stable ID");
    let profile_id = identity
        .stable_id(VehicleProfileOrdinal::from_raw(0))
        .expect("profile stable ID");
    let bytes = encode_lfrs(&checkpoint);

    let expected = replay_suffix(
        &mut original,
        &mut HostIdentityMap {
            routes: BTreeMap::from([(CHECKPOINT_ROUTE_ID, route)]),
            vehicles: BTreeMap::from([(CHECKPOINT_VEHICLE_ID, vehicle)]),
        },
        &stable_edges,
        parking_id,
        profile_id,
        1_000,
    );

    let restored = restored_checkpoint(&bytes, &original);
    assert_eq!(restored.route_mappings().len(), 1);
    assert_eq!(restored.vehicle_mappings().len(), 1);
    let restored_route = restored
        .route_handle(checkpoint_route_id)
        .expect("checkpoint route mapping");
    let restored_vehicle = restored
        .vehicle_handle(checkpoint_vehicle_id)
        .expect("checkpoint vehicle mapping");
    assert_eq!(
        restored.route_mappings(),
        &[(checkpoint_route_id, restored_route)]
    );
    assert_eq!(
        restored.vehicle_mappings(),
        &[(checkpoint_vehicle_id, restored_vehicle)]
    );
    let mut replayed = restored.into_world();
    let mut replayed_ids = HostIdentityMap {
        routes: BTreeMap::from([(CHECKPOINT_ROUTE_ID, restored_route)]),
        vehicles: BTreeMap::from([(CHECKPOINT_VEHICLE_ID, restored_vehicle)]),
    };
    let actual = replay_suffix(
        &mut replayed,
        &mut replayed_ids,
        &stable_edges,
        parking_id,
        profile_id,
        1_000,
    );
    assert_eq!(actual, expected);
    assert_eq!(
        first_divergence_interval(checkpoint_point, &expected, &actual),
        None
    );

    let restored = restored_checkpoint(&bytes, &original);
    let restored_route = restored
        .route_handle(checkpoint_route_id)
        .expect("checkpoint route mapping");
    let restored_vehicle = restored
        .vehicle_handle(checkpoint_vehicle_id)
        .expect("checkpoint vehicle mapping");
    let mut divergent = restored.into_world();
    let mut divergent_ids = HostIdentityMap {
        routes: BTreeMap::from([(CHECKPOINT_ROUTE_ID, restored_route)]),
        vehicles: BTreeMap::from([(CHECKPOINT_VEHICLE_ID, restored_vehicle)]),
    };
    let divergent_points = replay_suffix(
        &mut divergent,
        &mut divergent_ids,
        &stable_edges,
        parking_id,
        profile_id,
        2_000,
    );
    assert_eq!(
        first_divergence_interval(checkpoint_point, &expected, &divergent_points),
        Some((
            (checkpoint_point.command_cursor + 2, checkpoint_point.tick),
            (checkpoint_point.command_cursor + 3, checkpoint_point.tick)
        ))
    );
}
