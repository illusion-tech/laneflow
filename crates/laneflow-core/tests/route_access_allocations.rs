//! 绑定期两平面查表的零分配证据（SSOT §11）。
//!
//! 除 access registry 内容外完全相同的一对 world，成功 spawn（绑定）的分配
//! 统计必须逐项一致：resolved 表查表不引入 per-vehicle allocation。
//! 单测试 binary：stats_alloc 的 Region 统计是进程全局的，并发测试会互相污染。

use std::{alloc::System, hint::black_box};

use laneflow_core::{
    AccessEffect, AccessRegistry, AccessRule, AccessTargetId, CoreWorld, CrossSectionRegistry,
    EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, Junction, JunctionRegistry,
    LaneEdge, LaneGraph, ManeuverPath, Movement, ParkingRegistry, ParticipantClass,
    ParticipantClassRegistry, Route, SignalRegistry, Speed, SpeedLimit, VehicleHandle,
    VehicleProfile, VehicleProfileHandle, VehicleProfileRegistry, VehicleSpawnInput,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn edge(id: &str, next: &[&str]) -> LaneEdge {
    LaneEdge::new(
        id,
        EdgeLength::try_new(100.0).expect("test edge length"),
        SpeedLimit::try_new(f64::MAX).expect("test speed limit"),
        next.iter().copied(),
    )
}

fn classes() -> ParticipantClassRegistry {
    ParticipantClassRegistry::try_new(vec![
        ParticipantClass::new("motorVehicle", None),
        ParticipantClass::new("car", Some("motorVehicle")),
    ])
    .expect("test classes")
}

fn profile() -> VehicleProfile {
    let classes = classes();
    VehicleProfile::try_new_iidm(
        "car-profile",
        classes.class_handle("car").expect("car class"),
        IidmProfileSpec {
            length: 4.5,
            desired_speed: 13.9,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 1.4,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 4.0,
        },
    )
    .expect("test profile")
}

/// junction fixture：route `RJ=[e0,e1,j1,e2,e3]` 含一个 entry=1/exit=3 的
/// `path-1` occurrence，成功绑定同时走 edge 与 path 两个平面的查表。
fn junction_world(rules: Vec<AccessRule>) -> (CoreWorld, VehicleProfileHandle) {
    let graph = LaneGraph::try_new([
        edge("e0", &["e1"]),
        edge("e1", &["j1"]),
        edge("j1", &["e2"]),
        edge("e2", &["e3"]),
        edge("e3", &[]),
    ])
    .expect("test graph");
    let junctions = JunctionRegistry::try_new(
        &graph,
        [Junction::new("junction-1")],
        [Movement::new("movement-1", "junction-1")],
        [ManeuverPath::new(
            "path-1",
            "movement-1",
            "e1",
            ["j1"],
            "e2",
        )],
    )
    .expect("test junctions");
    let classes = classes();
    let access = AccessRegistry::try_new(
        &graph,
        &junctions,
        &CrossSectionRegistry::empty(),
        &classes,
        rules,
    )
    .expect("valid access registry");
    let profiles = VehicleProfileRegistry::try_new(&classes, [profile()]).expect("test profiles");
    let car = profiles.profile_handle("car-profile").expect("car profile");
    let traffic = InitialTrafficData::try_new(
        graph,
        vec![Route::try_new("RJ", ["e0", "e1", "j1", "e2", "e3"]).expect("junction route")],
        profiles,
        junctions,
        SignalRegistry::empty(),
        ParkingRegistry::empty(),
        classes,
        CrossSectionRegistry::empty(),
        access,
    )
    .expect("valid traffic");
    (
        CoreWorld::with_traffic_data(1_000, traffic, Vec::new()).expect("test world"),
        car,
    )
}

#[test]
fn binding_time_table_lookup_is_allocation_free() {
    let mut allow_rules = vec![AccessRule::new(
        "rule-allow-path",
        AccessTargetId::maneuver_path("path-1"),
        AccessEffect::Allow,
        ["car"],
    )];
    for edge_id in ["e0", "e1", "j1", "e2", "e3"] {
        allow_rules.push(AccessRule::new(
            format!("rule-allow-{edge_id}"),
            AccessTargetId::lane_edge(edge_id),
            AccessEffect::Allow,
            ["car"],
        ));
    }
    let (mut with_rules, car) = junction_world(allow_rules);
    let (mut without_rules, _) = junction_world(Vec::new());

    let spawn = |world: &mut CoreWorld, id: &str, progress: f64| {
        world.spawn_vehicle(VehicleSpawnInput::active(
            id,
            car,
            "RJ",
            0,
            EdgeProgress::try_new(progress).expect("progress"),
            Speed::ZERO,
        ))
    };

    // 预热 slot/resolver/update-order 容量，使测量只含新 ID 的确定性分配。
    for world in [&mut with_rules, &mut without_rules] {
        let warm = spawn(world, "warm", 0.0).expect("warm spawn");
        world.despawn_vehicle(warm).expect("warm despawn");
    }

    let measure = |world: &mut CoreWorld, id: &str, progress: f64| {
        let region = Region::new(GLOBAL);
        let output: VehicleHandle = spawn(world, id, progress).expect("measured spawn");
        black_box(&output);
        black_box(region.change())
    };
    // 测两轮：首轮吸收残留的一次性容量增长，两轮都必须逐项一致。
    for (id, progress) in [("v1", 0.0), ("v2", 50.0)] {
        let with_stats: Stats = measure(&mut with_rules, id, progress);
        let without_stats: Stats = measure(&mut without_rules, id, progress);
        assert_eq!(
            with_stats.allocations, without_stats.allocations,
            "access lookup must not add allocations"
        );
        assert_eq!(
            with_stats.reallocations, without_stats.reallocations,
            "access lookup must not add reallocations"
        );
        assert_eq!(
            with_stats.bytes_allocated, without_stats.bytes_allocated,
            "access lookup must not add allocated bytes"
        );
        assert_eq!(
            with_stats.bytes_reallocated, without_stats.bytes_reallocated,
            "access lookup must not add reallocated bytes"
        );
    }
}
