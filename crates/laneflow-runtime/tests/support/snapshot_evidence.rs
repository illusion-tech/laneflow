#[path = "policy.rs"]
mod test_policy;

use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, PoseSource, PublishedLfcaReference, SnapshotRestoreLimits,
    TrafficWorld, VehicleSpawnInput, WorldConfig,
};
use laneflow_scenario::signalized_corridor::{
    BoundCorridorCatalog, BoundSpawnSlot, CorridorCatalog, PASSENGER_CAR_PROFILE_KEY, bind,
};
use laneflow_static_contract::VehicleProfileOrdinal;
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

/// 信号化走廊 LFCA 夹具字节。
pub const CORRIDOR: &[u8] =
    include_bytes!("../../../../examples/data/v0.2-signalized-corridor.lfca");
const CORRIDOR_CATALOG: &str =
    include_str!("../../../../examples/data/v0.2-signalized-corridor.catalog.toml");
const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024);
/// 固定步长（毫秒）。
pub const DELTA_MS: u64 = 4;
/// 快照取证前的预热 tick 数。
pub const SNAPSHOT_WARMUP_TICKS: u32 = 64;

/// 构建走廊共享路网修订。
pub fn build() -> Arc<SharedNetworkRevision> {
    let input = check_canonical_network_input(CORRIDOR, FormatLimits::HARD).expect("checked");
    build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
    )
    .expect("build")
}

/// 以给定 asset key 构造走廊夹具的已提交路网来源。
pub fn source_for(key: &str) -> CommittedNetworkSource {
    let input = check_canonical_network_input(CORRIDOR, FormatLimits::HARD).expect("checked");
    CommittedNetworkSource::Published {
        reference: PublishedLfcaReference::new(
            key,
            input.canonical_artifact_digest(),
            input.canonical_artifact_byte_length(),
            input.network_revision(),
        )
        .expect("non-empty fixture key"),
    }
}

/// 安装走廊世界：注册目录路线并按槽位生成跟车两车。
pub fn install_corridor_world(revision: &Arc<SharedNetworkRevision>) -> TrafficWorld {
    let mut world = TrafficWorld::install(
        Arc::clone(revision),
        WorldConfig::new(8, 32, 1_024, 1_024, 1, DELTA_MS),
        source_for("fixture://snapshot-corridor"),
        512,
        test_policy::selection(revision),
    )
    .expect("install");
    let catalog: CorridorCatalog = toml::from_str(CORRIDOR_CATALOG).expect("catalog TOML");
    let bound = bind(&catalog, revision).expect("prepare bind");
    assert_eq!(bound.network_revision, revision.network_revision());
    let routes = bound
        .install_routes(&mut world)
        .expect("install catalog routes");
    let profile = *bound
        .profiles
        .get(PASSENGER_CAR_PROFILE_KEY)
        .expect("passenger-car profile");
    let (follower, leader) = follow_pair(&catalog, &bound);
    spawn_on_slot(&mut world, profile, leader, &routes);
    spawn_on_slot(&mut world, profile, follower, &routes);
    assert_two_lane_poses(&world);
    world
}

/// 测试固定的快照恢复上限。
pub const fn limits() -> SnapshotRestoreLimits {
    SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 8 * 1_024)
}

/// 断言两车均已提交 Lane 位姿来源。
pub fn assert_two_lane_poses(world: &TrafficWorld) {
    let poses = world.committed_pose_sources();
    assert_eq!(poses.as_slice().len(), 2);
    assert!(
        poses
            .as_slice()
            .iter()
            .all(|(_, source)| matches!(source, PoseSource::Lane { .. }))
    );
}

fn follow_pair<'a>(
    catalog: &CorridorCatalog,
    bound: &'a BoundCorridorCatalog,
) -> (&'a BoundSpawnSlot, &'a BoundSpawnSlot) {
    let lane = catalog
        .portals
        .first()
        .and_then(|portal| portal.lanes.first())
        .expect("portal lane");
    let follower = bound
        .spawn_slots
        .iter()
        .find(|slot| slot.slot_id == lane.entry_spawn_slot_id)
        .expect("entry spawn slot");
    let leader = bound
        .spawn_slots
        .iter()
        .find(|slot| {
            slot.portal_id == follower.portal_id
                && slot.lane_index == follower.lane_index
                && slot.edge == follower.edge
                && slot.progress_mm > follower.progress_mm
        })
        .expect("leader spawn slot");
    (follower, leader)
}

fn spawn_on_slot(
    world: &mut TrafficWorld,
    profile: VehicleProfileOrdinal,
    slot: &BoundSpawnSlot,
    routes: &[laneflow_runtime::RouteHandle],
) {
    let route = *routes
        .get(slot.route_index)
        .expect("catalog route must be registered");
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            profile,
            route,
            0,
            slot.progress_mm,
            0,
        ))
        .expect("catalog slot must spawn");
}
