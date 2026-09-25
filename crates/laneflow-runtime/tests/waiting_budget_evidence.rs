#![cfg(feature = "placement-fixtures")]
//! #282 WaitingZone 稳态 heap allocation 证据。
//!
//! 单一默认测试承载全局计数分配器。车辆已成功进入 WaitingZone、但尚未触达
//! release Gate 后，再测量连续固定步进；该窗口覆盖 membership、traversal phase、
//! occupancy 和本地存储约束的 steady path，硬断言 allocation / reallocation 均为零。
//!
//! 计量口径（#754）：计量窗口前先以一台同入口车辆完整走一遍「入门 → 暖机 →
//! 稳态窗口」并弃置其统计，吸收一次性懒初始化（线程局部状态、占用桶、通道缓冲）
//! 的分配；计量窗口只含逐拍复发的稳态路径，真实稳态分配必然在其中复发并被硬断言
//! 捕获。测试尾部向计量器注入一次真实分配自检，防止口径收紧把零断言变成永真。

#[path = "support/policy.rs"]
mod test_policy;

use std::alloc::System;
use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, PublishedLfcaReference, RouteRegisterInput, TickInput, TrafficWorld,
    VehicleSpawnInput, WorldConfig,
};
use laneflow_static_contract::{ManeuverPathOrdinal, VehicleProfileOrdinal};
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
const STEADY_TICKS: u32 = 16;

#[test]
fn waiting_steady_tick_has_zero_heap_allocation_after_warmup() {
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
        WorldConfig::new(8, 4, 1_024, 1_024, DELTA_MS),
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://waiting-budget",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("source"),
        },
        282,
        test_policy::selection(&revision),
    )
    .expect("world");
    let edges = world
        .traffic()
        .maneuvers()
        .maneuver_path(ManeuverPathOrdinal::from_raw(0))
        .expect("main path")
        .edges()
        .to_vec();
    let entry_length_mm = world.traffic().lane_lengths_millimetres()[edges[0].index()];
    let route = world
        .register_route(RouteRegisterInput::new(edges))
        .expect("route");
    let spawn_entry_vehicle = |world: &mut TrafficWorld| {
        world
            .place_existing_active_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    entry_length_mm - 1,
                    8_000,
                )
                .with_open_entrance(),
            )
            .expect("vehicle")
    };

    // 同形弃置暖机（#754）：一台同入口车辆完整走「入门 → 暖机 → 稳态窗口」，
    // 一次性懒初始化在其中吸收；其统计即弃，不作断言。
    {
        let _priming = Region::new(GLOBAL);
        let primed = spawn_entry_vehicle(&mut world);
        world
            .step(TickInput::new(DELTA_MS))
            .expect("priming admission");
        world
            .step(TickInput::new(DELTA_MS))
            .expect("priming settle");
        for _ in 0..STEADY_TICKS {
            world.step(TickInput::new(DELTA_MS)).expect("priming step");
        }
        world.despawn_vehicle(primed).expect("priming despawn");
    }

    let vehicle = spawn_entry_vehicle(&mut world);

    world.step(TickInput::new(DELTA_MS)).expect("admission");
    assert!(
        world
            .vehicle(vehicle)
            .and_then(|state| state.waiting_membership())
            .is_some(),
        "fixture vehicle must hold a Waiting membership before measurement"
    );
    // 进入等待区后的第一下移动，车身可能跨到下一条边，那条边的占用桶只长大一次。
    world
        .step(TickInput::new(DELTA_MS))
        .expect("settle occupancy");

    let stats = {
        let region = Region::new(GLOBAL);
        for _ in 0..STEADY_TICKS {
            world.step(TickInput::new(DELTA_MS)).expect("steady step");
        }
        region.change()
    };
    assert_eq!(stats.allocations, 0, "steady Waiting ticks allocated");
    assert_eq!(stats.reallocations, 0, "steady Waiting ticks reallocated");
    assert_eq!(
        stats.bytes_allocated, 0,
        "steady Waiting ticks allocated bytes"
    );
    assert!(
        world
            .vehicle(vehicle)
            .and_then(|state| state.waiting_membership())
            .is_some(),
        "measurement window must remain in the Waiting steady path"
    );
    println!(
        "waiting-g2-allocation-evidence steady_ticks={STEADY_TICKS} allocations={} \
         reallocations={} allocated_bytes={}",
        stats.allocations, stats.reallocations, stats.bytes_allocated
    );

    // 同一入门工作负载反复重建请求；重置在计时/计数窗外，两个事件缓冲都先暖机。
    let mut current = vehicle;
    for sample in 0..STEADY_TICKS + 4 {
        world.despawn_vehicle(current).unwrap();
        current = world
            .place_existing_active_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    entry_length_mm - 1,
                    8_000,
                )
                .with_open_entrance(),
            )
            .unwrap();
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
            world
                .vehicle(current)
                .unwrap()
                .waiting_membership()
                .is_some()
        );
    }
    println!(
        "waiting-g2-allocation-evidence repeated_admission_ticks={STEADY_TICKS} allocations=0 reallocations=0 allocated_bytes=0"
    );

    // 计量器自检（#754）：向计量面注入一次真实分配，计数器必须可见——
    // 防止口径收紧把上方零断言变成永真。
    let injected = {
        let region = Region::new(GLOBAL);
        let _hold = std::hint::black_box(vec![1_u8]);
        region.change()
    };
    assert!(
        injected.allocations > 0,
        "metering must observe an injected allocation"
    );
}
