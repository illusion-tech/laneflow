//! 在主线程同步执行，避免 libtest 监控线程的分配进入全局计数窗口。

#[path = "support/policy.rs"]
mod test_policy;

use std::{alloc::System, sync::Arc};

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, PublishedLfcaReference, RouteRegisterInput, SpawnError, TickInput,
    TrafficWorld, VehicleSpawnInput, WorldConfig,
};
use laneflow_static_contract::{LaneEdgeOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn main() {
    let mut args = libtest_mimic::Arguments::from_args();
    // 全进程计数不能与框架调度并发；命令行指定更多线程也不能改变测量边界。
    args.test_threads = Some(1);
    let main_thread = std::thread::current().id();
    let tests = vec![
        libtest_mimic::Trial::test(
            "warm_overlap_queries_do_not_allocate_routes_or_intervals",
            move || {
                assert_eq!(std::thread::current().id(), main_thread);
                warm_overlap_queries_do_not_allocate_routes_or_intervals();
                Ok(())
            },
        ),
        libtest_mimic::Trial::test("allocation_guard_rejects_real_heap_operations", || {
            allocation_guard_rejects_real_heap_operations();
            Ok(())
        }),
    ];
    libtest_mimic::run(&args, tests).exit();
}

fn warm_overlap_queries_do_not_allocate_routes_or_intervals() {
    let checked = check_canonical_network_input(
        include_bytes!(
            "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
        ),
        FormatLimits::HARD,
    )
    .unwrap();
    let revision = build_shared_network_revision(
        checked,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .unwrap();
    let origin = *revision.canonical_origin();
    let source = CommittedNetworkSource::Published {
        reference: PublishedLfcaReference::new(
            "fixture://overlap-allocation",
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .unwrap(),
    };
    let mut world = TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(8, 4, 1_024, 1_024, 100),
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        source,
        528,
        test_policy::selection(&revision),
    )
    .unwrap();
    let first = LaneEdgeOrdinal::from_raw(0);
    let mut edges = vec![first];
    if let Some(second) = world.traffic().successors(first).unwrap_or(&[]).first() {
        edges.push(*second);
    }
    let route = world
        .register_route(RouteRegisterInput::new(edges))
        .unwrap();
    let profile = VehicleProfileOrdinal::from_raw(0);
    let edge_len = world.traffic().lane_lengths_millimetres()[first.index()];
    let vehicle_len = world
        .traffic()
        .relations()
        .vehicle_profile(profile)
        .expect("profile")
        .length_mm();
    // 前保险杠不小于车长，车尾才完全落在这条边上，入口检查不扫描前驱。
    let progress = vehicle_len.min(edge_len);
    let blocker = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, route, 0, progress, 0).with_open_entrance())
        .unwrap();
    world.step(TickInput::new(100)).unwrap();
    let state = world.vehicle(blocker).unwrap();
    let query = VehicleSpawnInput::new(
        profile,
        route,
        state.route_edge_index(),
        state.progress_mm(),
        0,
    )
    .with_open_entrance();
    for _ in 0..8 {
        assert_eq!(world.spawn_vehicle(query), Err(SpawnError::Overlap));
    }

    let region = Region::new(GLOBAL);
    for _ in 0..256 {
        assert_eq!(world.spawn_vehicle(query), Err(SpawnError::Overlap));
    }
    assert_no_allocations(region.change());
}

fn assert_no_allocations(stats: Stats) {
    assert_eq!(stats.allocations, 0, "warm overlap queries allocated");
    assert_eq!(stats.reallocations, 0, "warm overlap queries reallocated");
}

fn allocation_guard_rejects_real_heap_operations() {
    let region = Region::new(GLOBAL);
    let mut bytes = std::hint::black_box(Vec::<u8>::with_capacity(8));
    let allocated = region.change();
    assert!(
        allocated.allocations > 0,
        "allocation control must allocate"
    );
    assert!(std::panic::catch_unwind(|| assert_no_allocations(allocated)).is_err());

    let region = Region::new(GLOBAL);
    bytes.reserve_exact(bytes.capacity() + 1);
    std::hint::black_box(&bytes);
    let reallocated = region.change();
    assert_eq!(reallocated.allocations, 0);
    assert!(
        reallocated.reallocations > 0,
        "reallocation control must grow"
    );
    assert!(std::panic::catch_unwind(|| assert_no_allocations(reallocated)).is_err());
}
