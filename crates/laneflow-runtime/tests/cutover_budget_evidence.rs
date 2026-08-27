//! #511 切换侧预算基线（corridor 夹具，同机描述性）。
//!
//! 合同 `traffic-runtime-revision-cutover.md` §9 度量协议：同机描述性基线
//! （沿 `LF-P100-REF-01` 先例），量化切片登记初值。本文件是切换侧初值的
//! 可复现证据：
//!
//! - 同修订换根调用的分配账本与墙钟（v1 同步形态下整次调用即静默提交窗口
//!   上界：Quiescent Commit 段为无分配的原地换绑，其余为 Prepare 段）；
//! - 在线准备干扰不变量的 v1 可测形式：切换前后稳态 tick 的分配账本相等
//!   （「稳态 tick 不因准备新增分配」）；账本跨轮确定性为硬断言；
//! - 候选共存峰值输入：双根并存期间的 `retained_logical_bytes` 与调用
//!   live 增量；
//! - 旧修订回收延迟代理：同构根对象最后借用退出后的析构墙钟。
//!
//! 分配账本沿 #441 `shared_network_allocation_evidence` 的纪律：独立
//! integration test 承载全局计数分配器，单一默认测试避免 Region 串账；
//! 每条账本先丢一次 warmup。账本与墙钟数字是描述性输出（登记进合同
//! §9 初值表），确定性与干扰不变量是硬断言。

use std::alloc::System;
use std::hint::black_box;
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
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const CORRIDOR: &[u8] = include_bytes!("../../../examples/data/v0.2-signalized-corridor.lfca");
const CORRIDOR_CATALOG: &str =
    include_str!("../../../examples/data/v0.2-signalized-corridor.catalog.toml");
const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024);
const DELTA_MS: u64 = 4;
const STEADY_WARMUP_TICKS: u32 = 64;
const STEADY_MEASURED_TICKS: u32 = 32;
const CUTOVER_WARMUP: usize = 1;
const CUTOVER_SAMPLES: usize = 7;
const RECLAIM_WARMUP: usize = 1;
const RECLAIM_SAMPLES: usize = 9;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Ledger {
    allocations: usize,
    reallocations: usize,
    allocated_bytes: usize,
    deallocated_bytes: usize,
    live_delta_bytes: usize,
}

impl Ledger {
    fn from_change(stats: stats_alloc::Stats) -> Self {
        Self {
            allocations: stats.allocations,
            reallocations: stats.reallocations,
            allocated_bytes: stats.bytes_allocated,
            deallocated_bytes: stats.bytes_deallocated,
            live_delta_bytes: stats
                .bytes_allocated
                .saturating_sub(stats.bytes_deallocated),
        }
    }
}

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
    let poses = world.committed_pose_sources();
    assert_eq!(poses.as_slice().len(), 2);
    assert!(
        poses
            .as_slice()
            .iter()
            .all(|(_, source)| matches!(source, PoseSource::Lane { .. }))
    );
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

fn steady_tick_ledger(world: &mut TrafficWorld) -> Ledger {
    let region = Region::new(GLOBAL);
    let input = TickInput::new(DELTA_MS);
    for _ in 0..STEADY_MEASURED_TICKS {
        world.step(input).expect("steady step");
    }
    Ledger::from_change(region.change())
}

/// 一轮完整测量：稳态基线 → 交替换根账本/墙钟 → 切换后稳态复核。
fn measure_round() -> (Ledger, Ledger, Ledger, Vec<u128>) {
    let root_a = build(SpatialBuildOption::RetainAvailable);
    let root_b = build(SpatialBuildOption::RetainAvailable);
    let mut world = install_corridor_world(&root_a);
    for _ in 0..STEADY_WARMUP_TICKS {
        world.step(TickInput::new(DELTA_MS)).expect("warmup step");
    }
    // 稳态基线丢一次 warmup 再取样。
    black_box(steady_tick_ledger(&mut world));
    let steady_before = steady_tick_ledger(&mut world);

    // 交替换根：每次调用都是完整的同修订换根事务（根对象与来源都换绑）。
    // 两个 key 等长：来源 String 的析构字节数进入账本，等长保证跨样本确定性。
    let source_base = source_for("fixture://corridor-root-a");
    let source_republished = source_for("fixture://corridor-root-b");
    let mut clocks = Vec::with_capacity(CUTOVER_SAMPLES);
    let mut ledgers = Vec::with_capacity(CUTOVER_SAMPLES);
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
        let region = Region::new(GLOBAL);
        let started = Instant::now();
        world
            .cutover_same_revision(Arc::clone(&target), source, &descriptor, &limits())
            .expect("same-revision cutover");
        let elapsed = started.elapsed().as_nanos();
        let ledger = Ledger::from_change(region.change());
        if index >= CUTOVER_WARMUP {
            clocks.push(elapsed);
            ledgers.push(ledger);
        }
    }
    assert!(
        ledgers.windows(2).all(|pair| pair[0] == pair[1]),
        "cutover allocation ledger must be deterministic across samples: {ledgers:?}"
    );
    let cutover = ledgers[0];

    let steady_after = steady_tick_ledger(&mut world);
    (steady_before, cutover, steady_after, clocks)
}

fn median(values: &mut [u128]) -> u128 {
    values.sort_unstable();
    values[values.len() / 2]
}

#[test]
fn cutover_side_budget_baseline() {
    let checked = check_canonical_network_input(CORRIDOR, FormatLimits::HARD).expect("checked");
    let artifact_digest = checked.canonical_artifact_digest();
    let network_revision = checked.network_revision();

    // 轮内与跨轮的账本确定性 + 在线准备干扰不变量（稳态 tick 零新增分配）。
    let round_one = measure_round();
    let round_two = measure_round();
    assert_eq!(
        round_one.0, round_two.0,
        "steady tick ledger must be deterministic across rounds"
    );
    assert_eq!(
        round_one.1, round_two.1,
        "cutover ledger must be deterministic across rounds"
    );
    assert_eq!(
        round_one.0, round_one.2,
        "steady tick ledger must not grow allocations across cutover"
    );
    assert_eq!(
        round_two.0, round_two.2,
        "steady tick ledger must not grow allocations across cutover"
    );

    // 候选共存峰值输入：双根并存逻辑字节（两根同 origin，各自完整保留）。
    let root_a = build(SpatialBuildOption::RetainAvailable);
    let root_b = build(SpatialBuildOption::RetainAvailable);
    let coexistence_logical_bytes =
        root_a.retained_logical_bytes() + root_b.retained_logical_bytes();

    // 旧修订回收延迟代理：同构根对象最后借用退出后的析构墙钟。
    let mut reclaim_clocks = Vec::with_capacity(RECLAIM_SAMPLES);
    for index in 0..(RECLAIM_WARMUP + RECLAIM_SAMPLES) {
        let root = build(SpatialBuildOption::RetainAvailable);
        let started = Instant::now();
        drop(root);
        let elapsed = started.elapsed().as_nanos();
        if index >= RECLAIM_WARMUP {
            reclaim_clocks.push(elapsed);
        }
    }

    let mut cutover_clocks = round_one.3.clone();
    cutover_clocks.extend_from_slice(&round_two.3);
    println!(
        "cutover-budget-evidence corridor lfca_exact={} artifact_digest={:x} network_revision={:x} steady_ticks={} steady_allocations={} steady_allocated_bytes={} cutover_samples={} cutover_allocations={} cutover_allocated_bytes={} cutover_live_delta_bytes={} cutover_reallocations={} cutover_wall_ns_median={} coexistence_logical_bytes={} reclaim_ns_median={}",
        CORRIDOR.len(),
        artifact_digest,
        network_revision.into_digest(),
        STEADY_MEASURED_TICKS,
        round_one.0.allocations,
        round_one.0.allocated_bytes,
        CUTOVER_SAMPLES,
        round_one.1.allocations,
        round_one.1.allocated_bytes,
        round_one.1.live_delta_bytes,
        round_one.1.reallocations,
        median(&mut cutover_clocks),
        coexistence_logical_bytes,
        median(&mut reclaim_clocks),
    );
}
