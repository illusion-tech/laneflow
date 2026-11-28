use super::super::{
    BoundaryOwner, GeometryWorkLimits, RoadSectionOrdinal, RouteHandoffOccurrence,
    RouteHandoffTrial, RouteRegisterInput, TRAVEL_TOLERANCE_MM, conflict_profile,
};
use super::{Scene, deterministic_state_digest};
use laneflow_runtime::{
    ExecutionConfig, RouteHandoffApproachError, RouteHandoffApproaches, RouteHandoffEntrantError,
    RouteHandoffEntrantReason, RouteHandoffRequest, SnapshotRestoreLimits, TickInput,
    VehicleSpawnInput, encode_lfrs, restore_lfrs,
};
use std::num::NonZeroU32;

const TICK: TickInput = TickInput::new(1_000);

fn route_window(
    scene: &Scene,
    target_lane: usize,
    rear_mm: u32,
) -> (RouteRegisterInput, RouteHandoffOccurrence) {
    let source = &scene.lanes[1];
    let target = &scene.lanes[target_lane];
    let revision = scene.world.revision();
    let occurrence = RouteHandoffOccurrence {
        revision: revision.network_revision(),
        source_index: 1,
        owner: BoundaryOwner::Section(RoadSectionOrdinal::from_raw(0)),
        left_index: u32::from(target_lane == 2),
        stations_mm: [u64::from(rear_mm + 14_000), u64::from(rear_mm + 16_000)],
    };
    let input = RouteRegisterInput::new(vec![source[0], source[1], target[1]]);
    (input, occurrence)
}

fn prepare(scene: &mut Scene, target_lane: usize, rear_mm: u32) -> RouteHandoffRequest {
    let revision = scene.world.revision();
    let (input, occurrence) = route_window(scene, target_lane, rear_mm);
    let route = scene
        .world
        .register_route(input.with_handoffs(vec![occurrence]))
        .unwrap();
    let vehicle = scene
        .world
        .spawn_vehicle(VehicleSpawnInput::new(
            conflict_profile(&revision, "car"),
            route,
            1,
            rear_mm + (4_150 - 750) - 10_000,
            1_000,
        ))
        .unwrap();
    RouteHandoffRequest {
        vehicle,
        route,
        revision: revision.network_revision(),
        source_index: 1,
        station_window_mm: [0, 200_000],
        rear_stations_mm: [u64::from(rear_mm), u64::from(rear_mm + 30_000)],
        speed_m_s: 1.,
        limits: GeometryWorkLimits::STANDARD,
    }
}

#[test]
fn distant_simultaneous_handoffs_restore_and_continue_after_front_transfer() {
    let mut scene = Scene::split_middle_lane();

    let requests = [
        prepare(&mut scene, 2, 10_000),
        prepare(&mut scene, 0, 134_000),
    ];
    let vehicles = requests.each_ref().map(|req| req.vehicle);
    run_to(&mut scene, &requests, 12);
    for v in vehicles {
        let state = scene.world.vehicle(v).unwrap();
        let progress = scene.world.vehicle_route_handoffs(v).next().unwrap();
        assert_eq!(progress.acquired_tick, 1);
        assert!(progress.front_handed_off);
        assert_eq!(state.route_edge_index(), 2);
        assert!((1_250..=1_260).contains(&state.progress_mm()));
    }
    // 两条目标边的切点相隔 124 米；两车相同的边内进度不能把车尾放回共享源边的末端。
    let capture = scene.world.capture_snapshot();
    let next = scene.world.step(TICK);
    assert!(
        capture.is_ok() && next.is_ok(),
        "tick 12 capture: {:?}, tick 13 step: {:?}",
        capture.as_ref().err(),
        next.as_ref().err()
    );
    let mut replay = restore_lfrs(
        &encode_lfrs(&capture.unwrap()),
        scene.world.revision(),
        scene.world.committed_source().clone(),
        scene.world.config(),
        ExecutionConfig::new(NonZeroU32::new(3).unwrap()),
        SnapshotRestoreLimits::new(4 << 20, 4096),
    )
    .unwrap()
    .into_world();
    for i in 13..=32 {
        if i > 13 {
            scene.world.step(TICK).unwrap();
        }
        replay.step(TICK).unwrap();
        assert_eq!(
            deterministic_state_digest(&scene.world.capture_snapshot().unwrap()).unwrap(),
            deterministic_state_digest(&replay.capture_snapshot().unwrap()).unwrap(),
            "tick {i}"
        );
        for v in vehicles {
            assert_eq!(scene.world.vehicle(v), replay.vehicle(v));
            let phase = scene.world.vehicle_route_handoffs(v).next();
            assert_eq!(phase, replay.vehicle_route_handoffs(v).next());
            assert_eq!(
                phase.is_some(),
                i <= 30,
                "clearance tick for {v:?} at tick {i}"
            );
        }
    }
}

#[test]
fn transferred_front_keeps_the_actual_source_tail_as_an_entrant_blocker() {
    let mut scene = Scene::split_middle_lane();
    let request = prepare(&mut scene, 2, 10_000);
    run_to(&mut scene, &[request], 12);
    let phase = scene
        .world
        .vehicle_route_handoffs(request.vehicle)
        .next()
        .unwrap();
    assert!(phase.front_handed_off);
    // 把另一候选放到已进入车辆的同一拍位置，当前前杠在目标侧，实体车尾仍占源侧。
    let body = probe(&scene, 10_000, phase.rear_travel_mm, u32::MAX).unwrap();
    for (side, blocker) in body.entrants.side_blockers.into_iter().enumerate() {
        let blocker = blocker.unwrap_or_else(|| panic!("侧 {side} 未检出实际车身"));
        assert_eq!(blocker.vehicle, request.vehicle);
        assert_eq!(blocker.reason, RouteHandoffEntrantReason::Occupied);
        assert_eq!(blocker.distance_mm, 0);
    }
}

fn run_to(scene: &mut Scene, requests: &[RouteHandoffRequest], tick: u64) {
    scene
        .world
        .step_with_route_handoffs(TICK, requests)
        .unwrap();
    for _ in 2..=tick {
        scene.world.step(TICK).unwrap();
    }
}

fn probe(
    scene: &Scene,
    rear_mm: u32,
    at: u64,
    work: u32,
) -> Result<RouteHandoffApproaches, RouteHandoffApproachError> {
    let (route, occurrence) = route_window(scene, 2, rear_mm);
    let binding = scene.world.bind_route_handoff(&route, occurrence).unwrap();
    let revision = scene.world.revision();
    let trial = RouteHandoffTrial {
        profile: revision
            .identity()
            .stable_id(conflict_profile(&revision, "car"))
            .unwrap(),
        rear_stations_mm: [u64::from(rear_mm), u64::from(rear_mm + 30_000)],
        station_window_mm: [0, 200_000],
        speed_m_s: 1.,
        limits: GeometryWorkLimits::STANDARD,
    };
    scene.world.preview_route_handoff_approaches(
        &binding,
        1,
        &trial,
        at..=at,
        TRAVEL_TOLERANCE_MM,
        work,
    )
}

#[test]
fn approaching_active_handoff_distinguishes_stopping_room_and_work_limit() {
    let mut scene = Scene::split_middle_lane();
    let request = prepare(&mut scene, 2, 10_000);
    run_to(&mut scene, &[request], 12);
    let near = probe(&scene, 10_000, 18_000, u32::MAX).unwrap();
    let work = near.entrants.work;
    assert_eq!(probe(&scene, 10_000, 18_000, work).unwrap(), near);
    assert_eq!(
        probe(&scene, 10_000, 18_000, work - 1),
        Err(RouteHandoffApproachError::Entrants(
            RouteHandoffEntrantError::Budget { work: work - 1 }
        ))
    );
    let limited = near.entrants.side_blockers[1].expect("目标侧入口在沿换道轨迹的制动范围内");
    assert_eq!(limited.vehicle, request.vehicle);
    let RouteHandoffEntrantReason::CannotStop {
        required_mm: Some(required),
        ..
    } = limited.reason
    else {
        panic!("当前车身尚未重叠，应按制动距离裁决：{limited:?}");
    };
    // 1 m/s 反应两拍，加上 4.5 m/s² 的服务制动距离和 2.5 m 净距，向上取毫米。
    assert_eq!(required, 4_612);
    assert!(limited.distance_mm > 0 && u64::from(limited.distance_mm) < required);
    let far = probe(&scene, 10_000, 28_000, u32::MAX).unwrap();
    assert_eq!(far.entrants.side_blockers, [None; 2]);

    for _ in 13..=30 {
        scene.world.step(TICK).unwrap();
    }
    assert!(scene.held(request.vehicle));
    // 查询另一个向前平移 10 米的候选，其目标侧地址全部在本车清尾前杠 43.4 米之后。
    let after_end = probe(&scene, 20_000, 27_000, u32::MAX).unwrap();
    let entry = after_end.swept_sides[1]
        .as_ref()
        .unwrap()
        .addresses()
        .next()
        .unwrap();
    assert_eq!(entry.edge, scene.lanes[2][1]);
    let ordinary_mm = (24_000 + entry.progress_mm[0]).checked_sub(43_400).unwrap();
    assert!(ordinary_mm > 0);
    let blocker = after_end.entrants.side_blockers[1].unwrap();
    assert_eq!(blocker.vehicle, request.vehicle);
    assert!(matches!(
        blocker.reason,
        RouteHandoffEntrantReason::CannotStop {
            required_mm: Some(4_612),
            ..
        }
    ));
    // 五次横移全长约 30.28914 米，第 30 拍后轴已走 30 米。停车距离还须算上
    // 到清尾前杠的约 289 毫米，保留计划的 5 毫米求界和整数毫米外取整。
    assert!((ordinary_mm + 283..=ordinary_mm + 290).contains(&blocker.distance_mm));
    let farther = probe(&scene, 20_000, 30_000, u32::MAX).unwrap();
    assert_eq!(farther.entrants.side_blockers, [None; 2]);
}
