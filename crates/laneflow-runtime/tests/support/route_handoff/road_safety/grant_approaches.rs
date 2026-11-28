use super::{CUT, Scene, restore, scene::Change};
use laneflow_runtime::{RouteHandoffRequest, TickInput, encode_lfrs};
use std::num::NonZeroU32;

const TICK: TickInput = TickInput::new(1_000);

fn follower_request(scene: &Scene, change: &Change) -> RouteHandoffRequest {
    let mut request = scene.request(change);
    request.rear_stations_mm[1] = request.rear_stations_mm[0] + 18_000;
    request
}

#[test]
fn simultaneous_grants_check_stopping_room_beyond_the_selected_target_paths() {
    for ahead_first in [false, true] {
        for (ahead_front, safe) in [(120_000, false), (136_000, true)] {
            let mut scene = Scene::at_interval(1, 1_000);
            let (follower, leader) = if ahead_first {
                let leader = scene.changer(2, "car", ahead_front, 1_000);
                (scene.changer(0, "car", 83_750, 10_000), leader)
            } else {
                let follower = scene.changer(0, "car", 83_750, 10_000);
                (follower, scene.changer(2, "car", ahead_front, 1_000))
            };
            let rear = follower_request(&scene, &follower);
            let mut front = scene.request(&leader);
            front.speed_m_s = 1.;
            // 后车单独获准时，从其存档读取清尾后保护的末端。两车准入的目标
            // 区间已经分开；后车仍须为清尾后沿中道接近前车的行程留足制动距离。
            let mut one = restore(&scene, NonZeroU32::MIN);
            one.step_with_route_handoffs(TICK, &[rear]).unwrap();
            let bytes = encode_lfrs(&one.capture_snapshot().unwrap());
            let root =
                crate::snapshot_wire::size_prefixed_root_as_runtime_snapshot(&bytes).unwrap();
            let downstream_end = CUT
                + root
                    .vehicles()
                    .iter()
                    .flat_map(|v| v.handoffs().iter())
                    .flat_map(|h| h.protections().iter())
                    .filter(|p| p.role().0 == 4)
                    .map(|p| p.end_mm())
                    .max()
                    .unwrap();
            let targets = scene.approaches(&leader, &front).swept_sides;
            let target_start = CUT
                + targets[1]
                    .as_ref()
                    .unwrap()
                    .addresses()
                    .next()
                    .unwrap()
                    .progress_mm[0];
            assert!(downstream_end < target_start);
            // 18 米纵向跨度的实际轨迹在 18–19 米之间。前车目标区间从后车
            // 终点前杠 101.75 米算起，近例不够 33.612 米，远例足够。
            let suffix = target_start - 101_750;
            if safe {
                assert!(18_000 + suffix >= 33_612);
            } else {
                assert!(19_000 + suffix < 33_612);
            }
            scene
                .world
                .step_with_route_handoffs(TICK, &[rear, front])
                .unwrap();
            assert_eq!(
                (scene.held(follower.vehicle), scene.held(leader.vehicle)),
                (!ahead_first || safe, ahead_first || safe),
                "ahead_first={ahead_first} ahead_front={ahead_front}"
            );
        }
    }
}

#[test]
fn new_target_approach_can_stop_before_an_already_held_handoff() {
    for (ahead_front, safe) in [(129_000, false), (145_000, true)] {
        let mut scene = Scene::at_interval(1, 1_000);
        let leader = scene.changer(2, "car", ahead_front, 1_000);
        let follower = scene.changer(0, "car", 83_750, 10_000);
        let mut front = scene.request(&leader);
        front.speed_m_s = 1.;
        scene
            .world
            .step_with_route_handoffs(TICK, &[front])
            .unwrap();
        assert!(scene.held(leader.vehicle));
        let rear = follower_request(&scene, &follower);
        scene.world.step_with_route_handoffs(TICK, &[rear]).unwrap();
        assert!(scene.held(leader.vehicle));
        assert_eq!(
            scene.held(follower.vehicle),
            safe,
            "ahead_front={ahead_front}"
        );
    }
}
