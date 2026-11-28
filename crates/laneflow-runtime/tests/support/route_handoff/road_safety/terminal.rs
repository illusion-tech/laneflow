use super::{Scene, restore};
use crate::snapshot_wire as wire;
use laneflow_runtime::{
    SnapshotRestoreError, SnapshotRestoreLimits, TickInput, encode_lfrs, restore_lfrs,
};
use std::num::NonZeroU32;

#[test]
fn four_millisecond_handoff_remains_located_until_release_and_replays_at_tick_640() {
    let tick = TickInput::new(4);
    let mut scene = Scene::at_interval(1, 4);
    let change = scene.changer(0, "car", 44_000, 10_000);
    let request = scene.request(&change);
    let trial = scene.trial(&change, &request);
    let preview = scene
        .world
        .preview_route_handoff_at(&change.binding, request.source_index, &trial)
        .unwrap();
    let query = |work| {
        preview
            .travel(
                laneflow_runtime::RouteHandoffTravelTarget::DistanceMm(25_600),
                NonZeroU32::MIN,
                work,
            )
            .unwrap()
    };
    let located = query(request.limits.tracks);
    let bounds = located.bounds.unwrap().rear.millimetres();
    // 独立弧长积分给出起点后 25.312821 m。两端向外取整到毫米。
    assert!(
        bounds[0] <= request.rear_stations_mm[0] + 25_312
            && bounds[1] >= request.rear_stations_mm[0] + 25_313
    );
    assert!(bounds[1] - bounds[0] <= 2);
    assert_eq!(query(located.work), located);
    let short = query(located.work - 1);
    assert_eq!(short.work, located.work - 1);
    assert_eq!(
        short.bounds,
        Err(laneflow_static_network::vehicle::Failure::WorkLimit)
    );
    scene
        .world
        .step_with_route_handoffs(tick, &[request])
        .unwrap();
    for i in 2..=639 {
        scene
            .world
            .step(tick)
            .unwrap_or_else(|e| panic!("tick {i}: {e}"));
    }
    assert_eq!(
        scene
            .world
            .vehicle_route_handoffs(change.vehicle)
            .next()
            .unwrap()
            .rear_travel_mm,
        25_560
    );
    let mut replay = restore(&scene, NonZeroU32::new(3).unwrap());
    for i in 640..=760 {
        scene
            .world
            .step(tick)
            .unwrap_or_else(|e| panic!("tick {i}: {e}"));
        replay.step(tick).unwrap();
        assert_eq!(
            scene.world.vehicle(change.vehicle),
            replay.vehicle(change.vehicle)
        );
        if i == 640 {
            assert_eq!(
                scene
                    .world
                    .vehicle_route_handoffs(change.vehicle)
                    .next()
                    .unwrap()
                    .rear_travel_mm,
                25_600
            );
        }
        if !scene.held(change.vehicle) {
            assert_eq!(i, 758);
            assert!(
                replay
                    .vehicle_route_handoffs(change.vehicle)
                    .next()
                    .is_none()
            );
            return;
        }
    }
    panic!("车尾未清空");
}

#[test]
fn fast_terminal_tick_finishes_with_the_whole_continuation_and_replays() {
    let tick = TickInput::new(1000);
    let mut scene = Scene::at_interval(1, 1000);
    let change = scene.changer(0, "car", 83_750, 10_000);
    let request = scene.request(&change);
    scene
        .world
        .step_with_route_handoffs(tick, &[request])
        .unwrap();
    for _ in 2..=3 {
        scene.world.step(tick).unwrap();
    }
    let active = scene
        .world
        .vehicle_route_handoffs(change.vehicle)
        .next()
        .unwrap();
    assert_eq!(active.rear_travel_mm, 30_000);
    let before = scene.front(change.vehicle);
    let mut replay = restore(&scene, NonZeroU32::new(3).unwrap());
    let mut shortened = encode_lfrs(&scene.world.capture_snapshot().unwrap());
    let root = wire::size_prefixed_root_as_runtime_snapshot(&shortened).unwrap();
    let held = root.vehicles().get(0).handoffs().get(0);
    let row = held
        .protections()
        .iter()
        .find(|p| p.role() == wire::HandoffProtectionRole::Downstream)
        .unwrap();
    assert_eq!(row.end_mm() - row.start_mm(), 12_500);
    let offset = row.0.as_ptr().addr() - shortened.as_ptr().addr();
    let mut old = *row;
    old.set_end_mm(old.start_mm() + 7_000);
    shortened[offset..offset + old.0.len()].copy_from_slice(&old.0);
    assert_eq!(
        restore_lfrs(
            &shortened,
            scene.world.revision(),
            scene.world.committed_source().clone(),
            scene.world.config(),
            scene.world.execution_config(),
            SnapshotRestoreLimits::new(4 << 20, 4096),
        )
        .unwrap_err(),
        SnapshotRestoreError::InvalidHandoffState
    );
    scene.world.step(tick).unwrap();
    assert!(!scene.held(change.vehicle));
    // 曲线末端仅余约 0.29 m，已接近直行；完整的第四拍必须在目标车道走足 10 m。
    assert_eq!(
        scene.world.vehicle(change.vehicle).unwrap().speed_mm_s(),
        10_000
    );
    assert!(scene.front(change.vehicle).abs_diff(before + 10_000) <= 1);
    replay.step(tick).unwrap();
    assert_eq!(
        scene.world.vehicle(change.vehicle),
        replay.vehicle(change.vehicle)
    );
}

#[test]
fn fast_terminal_tick_waits_for_the_occupied_additional_exit_space() {
    let mut scene = Scene::at_interval(1, 1000);
    let change = scene.changer(0, "car", 83_750, 10_000);
    let request = scene.request(&change);
    let exit = super::CUT
        + scene.approaches(&change, &request).swept_sides[1]
            .as_ref()
            .unwrap()
            .addresses()
            .last()
            .unwrap()
            .progress_mm[1];
    // 前车车尾位于旧的 7 m 保护范围以外，但会被速度为 10 m/s 的末拍触及。
    let leader = scene.straight(1, exit + 9_000 + 4_500, 0);
    let tick = TickInput::new(1000);
    scene
        .world
        .step_with_route_handoffs(tick, &[request])
        .unwrap();
    assert!(!scene.held(change.vehicle));
    assert!(scene.front(leader) >= exit + 13_500);
}

#[test]
fn fast_terminal_tick_stops_at_the_route_end_or_the_next_registered_handoff() {
    let tick = TickInput::new(1000);
    for returns in [false, true] {
        let mut scene = Scene::at_interval(1, 1000);
        let change = scene.changer_on(0, "car", 310_500, 10_000, returns);
        let request = scene.request(&change);
        scene
            .world
            .step_with_route_handoffs(tick, &[request])
            .unwrap();
        assert!(scene.held(change.vehicle));
        for i in 2..=20 {
            scene
                .world
                .step(tick)
                .unwrap_or_else(|e| panic!("tick {i}, returns {returns}: {e}"));
            assert!(
                scene
                    .world
                    .vehicle(change.vehicle)
                    .unwrap()
                    .route_edge_index()
                    <= 2
            );
        }
        assert!(!scene.held(change.vehicle));
        let final_state = scene.world.vehicle(change.vehicle).unwrap();
        assert_eq!(final_state.route_edge_index(), 2);
        assert_eq!(final_state.speed_mm_s(), 0);
        assert_eq!(scene.front(change.vehicle), 350_000);
        assert_eq!(
            final_state.status() == laneflow_runtime::VehicleStatus::Completed,
            !returns
        );
    }
}
