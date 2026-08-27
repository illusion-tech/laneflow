//! #511 切换侧预算基线（corridor 夹具，同机描述性）——墙钟面（未插桩）。
//!
//! 整次同修订换根调用（v1 同步形态下即静默提交窗口的显式上界：含全部
//! Prepare，Quiescent Commit 段为无分配原地换绑）与旧修订回收（同构根
//! 最后借用退出后的析构）的墙钟中位。本二进制不安装全局分配器：沿
//! #441 先例，分配账本（`cutover_budget_evidence`）与墙钟分文件测量，
//! 计数分配器的原子记账会污染墙钟，两个维度不得混在同一进程。
//!
//! 调用墙钟按两种宿主持有形态分别采样：外部借用保持（宿主保留旧根
//! `Arc`，调用内不发生旧根回收）与世界持有最后强引用（交换时旧根在
//! 调用内同步析构，是整次调用的上界口径）。中位数对偶数样本取中间
//! 两值平均。数字为描述性输出（登记进合同 §9 初值表）；硬断言只覆盖
//! 行为有效性（切换成功、位姿保持）。
//!
//! 本测试 `#[ignore]`：墙钟证据为 manual release 运行，CI 的 dev profile
//! 计时不得被当作基线（沿 #441 `shared_network_resource_evidence` 先例）。
//! 复现命令见合同 §9 初值表。

use std::sync::Arc;
use std::time::Instant;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, CutoverPreflightLimits, LfcaOriginBinding, MigrationPolicyKind,
    NetworkRevisionCutoverDescriptor, PoseSource, PublishedLfcaReference, TickInput, TrafficWorld,
    VehicleSpawnInput, WorldBinding, WorldConfig,
};
use laneflow_scenario::signalized_corridor::{
    BoundCorridorCatalog, BoundSpawnSlot, CorridorCatalog, PASSENGER_CAR_PROFILE_KEY, bind,
};
use laneflow_static_contract::VehicleProfileOrdinal;
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

const CORRIDOR: &[u8] = include_bytes!("../../../examples/data/v0.2-signalized-corridor.lfca");
const CORRIDOR_CATALOG: &str =
    include_str!("../../../examples/data/v0.2-signalized-corridor.catalog.toml");
const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024);
const DELTA_MS: u64 = 4;
const STEADY_WARMUP_TICKS: u32 = 64;
const CUTOVER_WARMUP: usize = 1;
const CUTOVER_SAMPLES: usize = 14;
const IN_CALL_RECLAIM_WARMUP: usize = 1;
const IN_CALL_RECLAIM_SAMPLES: usize = 9;
const ROOT_DROP_WARMUP: usize = 1;
const ROOT_DROP_SAMPLES: usize = 9;

fn build(spatial: SpatialBuildOption) -> Arc<SharedNetworkRevision> {
    let input = check_canonical_network_input(CORRIDOR, FormatLimits::HARD).expect("checked");
    build_shared_network_revision(input, SharedNetworkBuildOptions::new(spatial, BUILD_LIMITS))
        .expect("build")
}

fn source_for(key: &str) -> CommittedNetworkSource {
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

fn descriptor_for(
    world: &TrafficWorld,
    target: &Arc<SharedNetworkRevision>,
) -> NetworkRevisionCutoverDescriptor {
    let base = *world.revision().canonical_origin();
    let target_origin = *target.canonical_origin();
    NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(base),
        LfcaOriginBinding::from_canonical_origin(target_origin),
        None,
        MigrationPolicyKind::SameRevisionRestore,
        WorldBinding::new(1, 0, 0),
    )
}

fn limits() -> CutoverPreflightLimits {
    CutoverPreflightLimits::new(1_048_576)
}

fn install_corridor_world(revision: &Arc<SharedNetworkRevision>) -> TrafficWorld {
    let mut world = TrafficWorld::install(
        Arc::clone(revision),
        WorldConfig::new(8, 32, 1, DELTA_MS),
        source_for("fixture://corridor-base"),
        1,
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

fn assert_two_lane_poses(world: &TrafficWorld) {
    let poses = world.committed_pose_sources();
    assert_eq!(poses.as_slice().len(), 2);
    assert!(
        poses
            .as_slice()
            .iter()
            .all(|(_, source)| matches!(source, PoseSource::Lane { .. }))
    );
}

/// 真中位数：偶数样本取中间两值平均（u128 纳秒加法无溢出风险），
/// 奇数样本取正中值。
fn median(values: &mut [u128]) -> u128 {
    values.sort_unstable();
    let len = values.len();
    if len % 2 == 0 {
        (values[len / 2 - 1] + values[len / 2]) / 2
    } else {
        values[len / 2]
    }
}

#[test]
#[ignore = "manual release wall-clock evidence; CI 不当墙钟基线"]
fn cutover_side_wall_clock_baseline() {
    // 口径一：外部借用保持——宿主保留双根 Arc，交换时调用内不发生旧根
    // 回收。作用域结束后本地引用与 targets 迭代器全部释放，世界成为
    // 当前根唯一持有者，为口径二铺路。
    let borrow_median;
    let mut world;
    {
        let root_a = build(SpatialBuildOption::RetainAvailable);
        let root_b = build(SpatialBuildOption::RetainAvailable);
        world = install_corridor_world(&root_a);
        for _ in 0..STEADY_WARMUP_TICKS {
            world.step(TickInput::new(DELTA_MS)).expect("warmup step");
        }
        let source_base = source_for("fixture://corridor-root-a");
        let source_republished = source_for("fixture://corridor-root-b");
        let mut clocks = Vec::with_capacity(CUTOVER_SAMPLES);
        let mut targets = [Arc::clone(&root_a), Arc::clone(&root_b)]
            .into_iter()
            .cycle();
        for index in 0..(CUTOVER_WARMUP + CUTOVER_SAMPLES) {
            let target = targets.next().expect("alternating roots");
            let descriptor = descriptor_for(&world, &target);
            let source = if index % 2 == 0 {
                source_republished.clone()
            } else {
                source_base.clone()
            };
            let started = Instant::now();
            world
                .cutover_same_revision(Arc::clone(&target), source, &descriptor, &limits())
                .expect("same-revision cutover");
            let elapsed = started.elapsed().as_nanos();
            if index >= CUTOVER_WARMUP {
                clocks.push(elapsed);
            }
        }
        assert_two_lane_poses(&world);
        borrow_median = median(&mut clocks);
    }

    // 口径二：世界持有最后强引用——目标根按值移入调用，交换时旧根在
    // 调用内同步析构；这是整次调用的上界口径。
    let mut reclaim_clocks = Vec::with_capacity(IN_CALL_RECLAIM_SAMPLES);
    let source_base = source_for("fixture://corridor-root-a");
    let source_republished = source_for("fixture://corridor-root-b");
    for index in 0..(IN_CALL_RECLAIM_WARMUP + IN_CALL_RECLAIM_SAMPLES) {
        let fresh = build(SpatialBuildOption::RetainAvailable);
        let descriptor = descriptor_for(&world, &fresh);
        let source = if index % 2 == 0 {
            source_republished.clone()
        } else {
            source_base.clone()
        };
        let started = Instant::now();
        world
            .cutover_same_revision(fresh, source, &descriptor, &limits())
            .expect("same-revision cutover");
        let elapsed = started.elapsed().as_nanos();
        if index >= IN_CALL_RECLAIM_WARMUP {
            reclaim_clocks.push(elapsed);
        }
    }
    assert_two_lane_poses(&world);
    world.step(TickInput::new(DELTA_MS)).expect("step after");
    assert_two_lane_poses(&world);
    let reclaim_median = median(&mut reclaim_clocks);

    // 旧修订回收延迟：同构根最后借用退出后的孤立析构墙钟（隔离析构
    // 成本，与口径二的调用内混合形态互补）。
    let mut drop_clocks = Vec::with_capacity(ROOT_DROP_SAMPLES);
    for index in 0..(ROOT_DROP_WARMUP + ROOT_DROP_SAMPLES) {
        let root = build(SpatialBuildOption::RetainAvailable);
        let started = Instant::now();
        drop(root);
        let elapsed = started.elapsed().as_nanos();
        if index >= ROOT_DROP_WARMUP {
            drop_clocks.push(elapsed);
        }
    }
    let drop_median = median(&mut drop_clocks);

    println!(
        "cutover-wall-clock-evidence corridor cutover_borrow_samples={} cutover_wall_ns_median_borrow={} cutover_reclaim_samples={} cutover_wall_ns_median_reclaim={} root_drop_samples={} root_drop_ns_median={}",
        CUTOVER_SAMPLES,
        borrow_median,
        IN_CALL_RECLAIM_SAMPLES,
        reclaim_median,
        ROOT_DROP_SAMPLES,
        drop_median,
    );
}
