//! #706 增量 E3：提交边界（P7/commit）分配证据（审阅者 §七，独立证明）。
//!
//! 用真实分配器计数（stats_alloc 全局包装）逐场景证明 commit/P7 阶段
//! 零分配、零重分配。口径：Caller（worker=1）执行器 + 稳态预热——
//! P2/P3/P5 在 Caller 下走融合路径且热态零 LaneFlow 自有分配
//! （#705/#706 既有证据），Rayon 任务节点残差随 Caller 消失；故测量窗内
//! 的任何分配只能来自提交边界/事件处理。停车到达场景按到达观察次数
//! 精确断言：增长发生在 P5 的到达观察预留（`parking_arrivals`），
//! 不在 P7。journal 已武装/溢出场景的分配证据为 arena 容量不变性
//! （`admin::migration_journal` 的 lib 测试：武装期稳态 tick 写入预留
//! arena、不新增分配；溢出粘性停写、不回滚）。

#[path = "support/policy.rs"]
mod test_policy;

use std::alloc::System;
use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, ExecutionConfig, ParkingTarget, PublishedLfcaReference,
    ReserveParkingTarget, RouteRegisterInput, TickInput, TrafficWorld, VehicleSpawnInput,
    WorldConfig, WorldPolicySelection,
};
use laneflow_scenario::signalized_corridor::{CorridorCatalog, PASSENGER_CAR_PROFILE_KEY, bind};
use laneflow_static_contract::{ParkingSpaceOrdinal, VehicleProfileOrdinal};
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
const PARKING_ONLY: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/oracle-base.lfca"
);
const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024);

fn execution(workers: u32) -> ExecutionConfig {
    ExecutionConfig::new(std::num::NonZeroU32::new(workers).expect("nonzero workers"))
}

fn install_published(
    revision: &Arc<SharedNetworkRevision>,
    config: WorldConfig,
    world_id: u64,
    key: &str,
    selection: WorldPolicySelection,
) -> TrafficWorld {
    let origin = revision.canonical_origin();
    TrafficWorld::install(
        Arc::clone(revision),
        config,
        execution(1),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                key,
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("non-empty fixture key"),
        },
        world_id,
        selection,
    )
    .expect("install")
}

fn corridor_revision() -> Arc<SharedNetworkRevision> {
    let input = check_canonical_network_input(CORRIDOR, FormatLimits::HARD).expect("checked");
    build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(SpatialBuildOption::Omit, BUILD_LIMITS),
    )
    .expect("corridor revision")
}

fn parking_revision() -> Arc<SharedNetworkRevision> {
    let input =
        check_canonical_network_input(PARKING_ONLY, FormatLimits::HARD).expect("checked parking");
    build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(SpatialBuildOption::Omit, BUILD_LIMITS),
    )
    .expect("parking revision")
}

/// 信号走廊两条车道各 8 辆（同 parallel_preview_equivalence 场景二）。
fn corridor_world() -> TrafficWorld {
    let revision = corridor_revision();
    let catalog: CorridorCatalog = toml::from_str(CORRIDOR_CATALOG).expect("catalog TOML");
    let bound = bind(&catalog, &revision).expect("bind catalog");
    let mut world = install_published(
        &revision,
        WorldConfig::new(24, 32, 1_024, 1_024, 16),
        706_500,
        "fixture://p7-corridor",
        test_policy::selection(&revision),
    );
    let routes = bound.install_routes(&mut world).expect("install routes");
    let profile = *bound
        .profiles
        .get(PASSENGER_CAR_PROFILE_KEY)
        .expect("passenger-car profile");
    for lane_index in [0_usize, 1_usize] {
        let lane_slots: Vec<_> = bound
            .spawn_slots
            .iter()
            .filter(|slot| slot.portal_id == "portal-main-west" && slot.lane_index == lane_index)
            .take(8)
            .collect();
        assert_eq!(lane_slots.len(), 8, "corridor lane must expose 8 slots");
        for slot in lane_slots {
            world
                .spawn_vehicle(VehicleSpawnInput::new(
                    profile,
                    routes[slot.route_index],
                    0,
                    slot.progress_mm,
                    0,
                ))
                .expect("corridor spawn");
        }
    }
    world
}

/// 单停车者（已预约、生成在入口前 5 m）：到达观察发生在预热后的某拍。
fn parking_world() -> TrafficWorld {
    let revision = parking_revision();
    let mut world = install_published(
        &revision,
        WorldConfig::new(10, 4, 1_024, 1_024, 100),
        706_501,
        "fixture://p7-parking",
        test_policy::selection(&revision),
    );
    let space = ParkingSpaceOrdinal::from_raw(0);
    let (entry_edge, entry_progress) = world
        .traffic()
        .relations()
        .parking_space(space)
        .expect("parking space")
        .entry();
    let exit_edge = world
        .traffic()
        .successors(entry_edge)
        .and_then(|successors| successors.first())
        .copied()
        .expect("parking fixture successor");
    let route = world
        .register_route(RouteRegisterInput::new(vec![entry_edge, exit_edge]))
        .expect("parking route");
    let parker = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            entry_progress.saturating_sub(5_000),
            10_000,
        ))
        .expect("parker spawn");
    world
        .reserve_parking(
            parker,
            ReserveParkingTarget::ExplicitSpace {
                space,
                entry_route_occurrence: 0,
            },
        )
        .expect("reserve");
    world
}

/// P7 零分配（场景一）：普通运动无事件——零决策零事件批次，车辆推进
/// 穿过路口（passage enter/leave、crossing/clear/release 转移段在提交
/// 边界 staging）。预热后 24 拍测量窗内分配与再分配必须为零。
#[test]
fn p7_corridor_steady_steps_zero_alloc() {
    let mut world = corridor_world();
    for _ in 0..8 {
        world.step(TickInput::new(16)).expect("warmup step");
    }
    let progress_before: u64 = world
        .live_vehicles()
        .iter()
        .map(|handle| u64::from(world.vehicle(*handle).expect("vehicle").progress_mm()))
        .sum();
    let region = Region::new(GLOBAL);
    let mut decision_batches = 0_usize;
    let mut event_batches = 0_usize;
    for _ in 0..24 {
        world.step(TickInput::new(16)).expect("measured step");
        if !world.latest_waiting_decisions().is_empty()
            || !world.latest_conflict_decisions().is_empty()
        {
            decision_batches += 1;
        }
        if !world.latest_transition_events().is_empty() {
            event_batches += 1;
        }
    }
    let stats = region.change();
    assert_eq!(
        stats.allocations, 0,
        "commit/P7 稳态步必须零分配: {:?}",
        stats
    );
    assert_eq!(stats.reallocations, 0, "commit/P7 稳态步必须零再分配");
    assert_eq!(
        decision_batches, 0,
        "本场景为普通运动无事件（零决策零事件批次）"
    );
    assert_eq!(event_batches, 0, "本场景零生命周期事件");
    let progress_after: u64 = world
        .live_vehicles()
        .iter()
        .map(|handle| u64::from(world.vehicle(*handle).expect("vehicle").progress_mm()))
        .sum();
    assert!(
        progress_after > progress_before,
        "测量窗必须覆盖车辆推进（路口通行转移）"
    );
    println!(
        "p7-corridor-zero-alloc ticks=24 allocations={} reallocations={} \
         decision_batches={} bytes={}",
        stats.allocations, stats.reallocations, decision_batches, stats.bytes_allocated
    );
}

/// P7 零分配（场景二）：停车到达的增长发生在 P5 到达观察预留，不在
/// P7。预热至到达前；到达拍分配数必须恰好等于到达观察数（1），其后
/// Active→Parked 转移拍与稳态拍必须零分配/零再分配。
#[test]
fn p7_parking_arrival_growth_is_in_p5_not_p7() {
    let mut world = parking_world();
    // 冷态首拍（工作区预热）不计入测量。
    world.step(TickInput::new(100)).expect("cold step");
    // 预热到到达前一刻（到达拍本身留进测量窗）。
    loop {
        let region = Region::new(GLOBAL);
        let outcome = world.step(TickInput::new(100)).expect("pre-arrival step");
        let stats = region.change();
        if !outcome.parking_arrivals().is_empty() {
            assert_eq!(
                stats.allocations,
                outcome.parking_arrivals().len(),
                "到达拍的唯一分配必须是 P5 到达观察预留"
            );
            assert_eq!(stats.reallocations, 0);
            break;
        }
        assert_eq!(stats.allocations, 0, "到达前稳态步必须零分配: {:?}", stats);
        assert_eq!(stats.reallocations, 0);
    }
    // 到达后：park 命令 + 稳态步零分配。
    let space = ParkingSpaceOrdinal::from_raw(0);
    let target = ParkingTarget::ExplicitSpace(space);
    world
        .park_vehicle(world.live_vehicles()[0], target)
        .expect("park");
    let region = Region::new(GLOBAL);
    for _ in 0..4 {
        world.step(TickInput::new(100)).expect("post-arrival step");
    }
    let stats = region.change();
    assert_eq!(
        stats.allocations, 0,
        "到达后（含 Active→Parked 转移）必须零分配: {:?}",
        stats
    );
    assert_eq!(stats.reallocations, 0);
    println!(
        "p7-parking-arrival allocations_after={} reallocations_after={}",
        stats.allocations, stats.reallocations
    );
}
