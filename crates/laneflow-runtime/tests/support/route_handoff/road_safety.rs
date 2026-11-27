//! 实际编制、登记和步进的动态换道。两条外侧实车道可分别向公共中道横移。
#[path = "road_safety/scene.rs"]
mod scene;
use laneflow_runtime::{RouteHandoffEntrantReason, deterministic_state_digest};
use scene::{CUT, Scene, TICK, restore};
use std::num::NonZeroU32;

/// 两个速度共用同一独立几何定位，仅改变目标后车的已提交速度。
fn changer_yields_until_passage(workers: u32, speed: u32, required: u64) -> Vec<(u32, u32, bool)> {
    let mut scene = Scene::new(workers);
    let change = scene.changer(0, "car", 83_750, 10_000);
    let request = scene.request(&change);
    let initial = scene.approaches(&change, &request);
    let bounds = initial.swept_sides[1].as_ref().unwrap();
    let entry = CUT + bounds.addresses().next().unwrap().progress_mm[0];
    let straight = scene.straight(1, entry - 8_000, speed);
    let probe = scene.approaches(&change, &request);
    let blocker = probe.entrants.side_blockers[1].unwrap();
    assert_eq!((blocker.vehicle, blocker.distance_mm), (straight, 8_000));
    assert_eq!(
        blocker.reason,
        RouteHandoffEntrantReason::CannotStop {
            required_mm: Some(required),
            may_reach_this_tick: false,
        }
    );
    let mut trace = Vec::new();
    let mut previous_speed = speed;
    let mut entered = false;
    for tick in 0..220 {
        let requests = if entered {
            vec![]
        } else {
            vec![scene.request(&change)]
        };
        let before = scene.front(straight);
        let traffic = (!entered).then(|| scene.approaches(&change, &requests[0]));
        scene
            .world
            .step_with_route_handoffs(TICK, &requests)
            .unwrap();
        assert!(!scene.held(straight));
        let vehicle = scene.world.vehicle(straight).unwrap();
        assert!(vehicle.speed_mm_s() > 0 && previous_speed <= vehicle.speed_mm_s() + 450);
        previous_speed = vehicle.speed_mm_s();
        let active = scene.held(change.vehicle);
        if let Some(traffic) = traffic {
            if tick == 0 || traffic.entrants.side_blockers[1].is_some() {
                assert!(!active, "换道须让行，tick={tick}");
                assert_eq!(
                    scene
                        .world
                        .vehicle(change.vehicle)
                        .unwrap()
                        .route_edge_index(),
                    1
                );
            }
            if active {
                let end = CUT
                    + traffic.swept_sides[1]
                        .as_ref()
                        .unwrap()
                        .addresses()
                        .last()
                        .unwrap()
                        .progress_mm[1];
                assert!(
                    before - vehicle.length_mm() > end,
                    "直行车尾已过本拍申请的目标扫过范围"
                );
                entered = true;
            }
        }
        if let Some(geometry) = scene
            .world
            .committed_handoff_geometry(change.vehicle)
            .unwrap()
            && let Some(target) = &geometry.footprint.sides[1]
        {
            let guard = CUT + target.enclosure.addresses().last().unwrap().progress_mm[1];
            assert!(scene.front(straight) - vehicle.length_mm() > guard);
        }
        trace.push((scene.front(straight), scene.front(change.vehicle), active));
        if entered && !active {
            assert_eq!(
                scene
                    .world
                    .vehicle(change.vehicle)
                    .unwrap()
                    .route_edge_index(),
                2
            );
            return trace;
        }
    }
    panic!("直行通过后仍未完成横移，末拍 {:?}", trace.last());
}

#[test]
fn r1a_straight_keeps_priority_and_the_changer_retries_after_clearance() {
    assert_eq!(
        changer_yields_until_passage(1, 10_000, 15_612),
        changer_yields_until_passage(3, 10_000, 15_612)
    );
}

#[test]
fn r2_fast_target_follower_blocks_beyond_one_tick_and_the_distant_control_allows_entry() {
    changer_yields_until_passage(1, 13_900, 26_748);
    for (speed, gap) in [(13_900, 26_748), (13_900, 35_000), (10_000, 15_612)] {
        check_able_to_yield(speed, gap);
    }
}

fn check_able_to_yield(speed: u32, gap: u32) {
    let mut scene = Scene::new(1);
    let change = scene.changer(0, "car", 83_750, 1_000);
    let request = scene.request(&change);
    let entry = CUT
        + scene.approaches(&change, &request).swept_sides[1]
            .as_ref()
            .unwrap()
            .addresses()
            .next()
            .unwrap()
            .progress_mm[0];
    let follower = scene.straight(1, entry - gap, speed);
    assert!(scene.approaches(&change, &request).entrants.side_blockers[1].is_none());
    scene
        .world
        .step_with_route_handoffs(TICK, &[request])
        .unwrap();
    assert!(scene.held(change.vehicle));
    let first_speed = scene.world.vehicle(follower).unwrap().speed_mm_s();
    assert!(speed <= first_speed + 451);
    if gap <= 26_748 {
        assert!(first_speed < speed, "恰足范围从准入拍开始服务制动");
    }
    for _ in 0..100 {
        if !scene.held(change.vehicle) {
            assert_eq!(
                scene
                    .world
                    .vehicle(change.vehicle)
                    .unwrap()
                    .route_edge_index(),
                2
            );
            return;
        }
        let prior = scene.world.vehicle(follower).unwrap().speed_mm_s();
        scene.world.step(TICK).unwrap_or_else(|e| {
            panic!(
                "step {e:?}, tick {}, follower {} @ {}, {:?}",
                scene.world.tick_index(),
                scene.front(follower),
                prior,
                scene.world.vehicle_route_handoffs(change.vehicle).next()
            )
        });
        let next = scene.world.vehicle(follower).unwrap().speed_mm_s();
        assert!(
            prior <= next + 451,
            "目标后车应可用服务制动，gap={gap}, {prior}->{next}"
        );
        if let Some(geometry) = scene
            .world
            .committed_handoff_geometry(change.vehicle)
            .unwrap()
            && let Some(target) = &geometry.footprint.sides[1]
        {
            assert!(
                scene.front(follower)
                    < CUT + target.enclosure.addresses().next().unwrap().progress_mm[0]
            );
        }
    }
    panic!("可停车的后车未放行换道");
}

#[test]
fn r1b_only_actual_entry_owns_the_window_and_stable_order_breaks_contention() {
    let run = |workers, reverse_requests, first_lane| {
        let mut scene = Scene::new(workers);
        let first = scene.changer(first_lane, "car", 83_750, 4_000);
        let second = scene.changer(2 - first_lane, "car", 83_750, 4_000);
        let handles = [first.vehicle, second.vehicle];
        let mut early = [scene.request(&first), scene.request(&second)];
        for (request, ahead) in early.iter_mut().zip([19_000, 27_000]) {
            request.rear_stations_mm = request.rear_stations_mm.map(|v| v + ahead);
        }
        scene.world.step_with_route_handoffs(TICK, &early).unwrap();
        assert!(handles.iter().all(|h| !scene.held(*h)));
        for change in [&first, &second] {
            let mut isolated = restore(&scene, NonZeroU32::MIN);
            let request = scene.request(change);
            isolated.step_with_route_handoffs(TICK, &[request]).unwrap();
            assert!(
                isolated
                    .vehicle_route_handoffs(change.vehicle)
                    .next()
                    .is_some(),
                "每个申请单独可进入"
            );
        }
        let mut starts = [None; 2];
        let mut clears = [None; 2];
        let mut trace = Vec::new();
        for tick in 2..220 {
            let mut requests = [&first, &second]
                .into_iter()
                .enumerate()
                .filter(|(i, _)| starts[*i].is_none())
                .map(|(_, change)| scene.request(change))
                .collect::<Vec<_>>();
            if reverse_requests {
                requests.reverse();
            }
            scene
                .world
                .step_with_route_handoffs(TICK, &requests)
                .unwrap();
            let held = handles.map(|h| scene.held(h));
            assert_ne!(held, [true; 2]);
            if tick == 2 {
                assert_eq!(held, [true, false], "稳定车辆顺序");
            }
            for i in 0..2 {
                if held[i] && starts[i].is_none() {
                    starts[i] = Some(tick);
                }
                if !held[i] && starts[i].is_some() && clears[i].is_none() {
                    clears[i] = Some(tick);
                }
            }
            trace.push(
                deterministic_state_digest(&scene.world.capture_snapshot().unwrap()).unwrap(),
            );
            if clears[1].is_some() {
                assert!(clears[0].unwrap() < starts[1].unwrap());
                return trace;
            }
        }
        panic!("败者未在前车清空后完成，starts={starts:?}, clears={clears:?}");
    };
    for first_lane in [0, 2] {
        let expected = run(1, false, first_lane);
        assert_eq!(expected, run(1, true, first_lane));
        assert_eq!(expected, run(3, true, first_lane));
    }
}

#[test]
fn r3_long_vehicle_protects_the_source_after_front_handoff_and_clears_on_both_worlds() {
    let mut scene = Scene::new(1);
    let change = scene.changer(0, "long", 46_000, 8_000);
    let follower = scene.straight(0, 17_000, 8_000);
    let first = scene.request(&change);
    assert_eq!(
        scene.world.vehicle(change.vehicle).unwrap().length_mm(),
        12_000
    );
    assert!(scene.front(change.vehicle) - 12_000 < CUT);
    scene
        .world
        .step_with_route_handoffs(TICK, &[first])
        .unwrap();
    assert!(scene.held(change.vehicle));
    let geometry = scene
        .world
        .committed_handoff_geometry(change.vehicle)
        .unwrap()
        .unwrap();
    let source = geometry.footprint.sides[0].as_ref().unwrap();
    assert!(
        source
            .witness
            .addresses()
            .any(|a| a.edge == scene.lanes[0][0])
    );
    assert_eq!(geometry.front_occurrence, 1);
    assert!(
        geometry
            .footprint
            .front
            .addresses()
            .any(|a| a.edge == scene.lanes[0][1])
    );
    let mut restored = None;
    let mut protected_after_front = false;
    let mut tail_cleared = false;
    let mut cleared = false;
    for _ in 0..220 {
        if let Some(geo) = scene
            .world
            .committed_handoff_geometry(change.vehicle)
            .unwrap()
        {
            if let Some(source) = &geo.footprint.sides[0] {
                let address = source.enclosure.addresses().next().unwrap();
                let start = address.progress_mm[0]
                    + if address.edge == scene.lanes[0][0] {
                        0
                    } else {
                        CUT
                    };
                assert!(scene.front(follower) < start);
                if geo.front_handed_off {
                    protected_after_front = true;
                    if restored.is_none() {
                        restored = Some(restore(&scene, NonZeroU32::new(2).unwrap()));
                    }
                }
            } else if geo.front_handed_off {
                tail_cleared = true;
            }
        } else {
            assert!(tail_cleared, "组合预约须覆盖整车离开源道的时刻");
            cleared = true;
            assert_eq!(
                scene
                    .world
                    .vehicle(change.vehicle)
                    .unwrap()
                    .route_edge_index(),
                2
            );
            if scene.front(follower) > 106_000 {
                break;
            }
        }
        scene.world.step(TICK).unwrap();
        if let Some(copy) = &mut restored {
            copy.step(TICK).unwrap();
            assert_eq!(
                deterministic_state_digest(&scene.world.capture_snapshot().unwrap()).unwrap(),
                deterministic_state_digest(&copy.capture_snapshot().unwrap()).unwrap()
            );
        }
    }
    assert!(
        protected_after_front && cleared && scene.front(follower) > 106_000,
        "长车尾清空后原源侧区间重新通行"
    );
    assert!(scene.world.committed_handoff_vehicles().next().is_none());
}
