use crate::admin::cutover_migration::tests::{
    conflict_scale_revision, conflict_scale_world_with_identity,
};

#[test]
fn a_slow_follower_accumulates_motion_across_ticks_without_losing_its_gap() {
    let mut world = conflict_scale_world_with_identity(conflict_scale_revision(), 2, 1, 1_511_021);
    let (leader, follower) = (world.live_vehicles()[0], world.live_vehicles()[1]);
    for (car, progress_mm) in [(leader, 20_000), (follower, 10_000)] {
        let mut slot = world
            .state
            .committed
            .vehicles
            .slot_mut(car.index() as usize);
        let state = slot.state.as_mut().unwrap();
        state.route_edge_index = 0;
        state.progress_mm = progress_mm;
        state.carry_um = 0;
        state.speed_mm_s = 0;
    }
    world.state.invalidate_occupancy_source();
    world.state.rebuild_occupancy_index().unwrap();
    let start = world.vehicle(follower).unwrap();
    let gap = world
        .traffic()
        .relations()
        .vehicle_profile(start.profile())
        .unwrap()
        .min_gap_mm();
    let mut previous = u64::from(start.progress_mm()) * 1_000;
    for _ in 0..16 {
        world.step(crate::TickInput::new(4)).unwrap();
        let front = world.vehicle(leader).unwrap();
        let rear = world.vehicle(follower).unwrap();
        let position = u64::from(rear.progress_mm()) * 1_000 + u64::from(rear.carry_um());
        assert!((1..1_000).contains(&(position - previous)));
        assert!(front.progress_mm() - front.length_mm >= rear.progress_mm() + gap);
        previous = position;
    }
    assert!(world.vehicle(follower).unwrap().progress_mm() > start.progress_mm());
}

fn at_follow_request(
    count: u32,
) -> (
    crate::TrafficWorld,
    crate::VehicleHandle,
    crate::VehicleHandle,
) {
    let mut world = conflict_scale_world_with_identity(
        conflict_scale_revision(),
        count,
        3,
        1_511_010 + u64::from(count),
    );
    let (leader, follower) = (world.live_vehicles()[0], world.live_vehicles()[1]);
    for car in [leader, follower] {
        world
            .state
            .committed
            .vehicles
            .slot_mut(car.index() as usize)
            .state
            .as_mut()
            .unwrap()
            .speed_mm_s = 2_000;
    }
    // 先让链首的完整车身过门，然后把后车放到尚未 crossing 的申请位置。
    let source = world.vehicle(leader).unwrap();
    let route = world.state.compiled_route(source.route()).unwrap();
    let hop = route.conflicts[0].admission_hop;
    let edge = route.edges[hop as usize];
    let line = world.traffic().lane_lengths_millimetres()[edge.index()];
    for _ in 0..1_200 {
        world.step(crate::TickInput::new(4)).unwrap();
        let head = world.vehicle(leader).unwrap();
        if head.route_edge_index() == hop + 1 && head.progress_mm() >= 8_000 {
            break;
        }
    }
    assert!(world.conflict_reservation(leader).is_some());
    {
        let mut slot = world
            .state
            .committed
            .vehicles
            .slot_mut(follower.index() as usize);
        let next = slot.state.as_mut().unwrap();
        next.route_edge_index = hop;
        next.progress_mm = line - 1;
        next.carry_um = 999;
        next.speed_mm_s = 2_000;
    }
    world.state.invalidate_occupancy_source();
    world.state.invalidate_spawn_contenders();
    (world, leader, follower)
}

#[test]
fn ordinary_followers_share_before_the_leader_clears_in_a_real_step() {
    let (mut world, leader, follower) = at_follow_request(2);
    let mut saw_shared = false;
    let mut follower_end = None;
    for _ in 0..1_200 {
        world
            .step(crate::TickInput::new(4))
            .expect("ordinary platoon step");
        let a = world.conflict_reservation(leader);
        let b = world.conflict_reservation(follower);
        if let (Some(a), Some(b)) = (a, b) {
            assert!(a.downstream_target() < b.downstream_target());
            assert!(world.state.conflict_platoons_valid().unwrap());
            if !saw_shared {
                let snapshot = world
                    .capture_snapshot()
                    .expect("capture simultaneous reservations");
                let disk = crate::encode_lfrs(&snapshot);
                let restored = crate::restore_lfrs(
                    &disk,
                    world.revision(),
                    world.committed_source().clone(),
                    world.config(),
                    world.execution_config(),
                    crate::SnapshotRestoreLimits::new(16 * 1024 * 1024, 4 * 1024),
                )
                .expect("restore simultaneous reservations");
                let loaded = restored.world();
                for (car, end) in loaded
                    .live_vehicles()
                    .iter()
                    .zip([a.downstream_target(), b.downstream_target()])
                {
                    assert_eq!(
                        loaded
                            .conflict_reservation(*car)
                            .unwrap()
                            .downstream_target(),
                        end
                    );
                }
                let state = world.vehicle(follower).unwrap();
                let single = world
                    .state
                    .reservation_downstream_claim_plan(b.passage_range(), state.length_mm)
                    .unwrap()
                    .target();
                assert!(
                    world
                        .state
                        .parking_preserves_conflict_storage(
                            state,
                            state,
                            single.route_edge_index(),
                            single.progress_mm()
                        )
                        .unwrap()
                );
                assert!(
                    !world
                        .state
                        .parking_preserves_conflict_storage(
                            state,
                            state,
                            single.route_edge_index(),
                            single.progress_mm() - 1
                        )
                        .unwrap()
                );
            }
            follower_end = Some(b.downstream_target());
            saw_shared = true;
        } else if saw_shared
            && a.is_none()
            && let Some(b) = b
        {
            assert_eq!(b.downstream_target(), follower_end.unwrap());
            assert!(world.state.conflict_platoons_valid().unwrap());
            let state = world.vehicle(leader).unwrap();
            let end = follower_end.unwrap();
            assert!(end.progress_mm() > 0);
            assert!(
                world
                    .state
                    .parking_preserves_conflict_storage(
                        state,
                        state,
                        end.route_edge_index(),
                        end.progress_mm()
                    )
                    .unwrap()
            );
            assert!(
                !world
                    .state
                    .parking_preserves_conflict_storage(
                        state,
                        state,
                        end.route_edge_index(),
                        end.progress_mm() - 1
                    )
                    .unwrap()
            );
            return;
        }
    }
    panic!("expected simultaneous reservations followed by independent leader clearance");
}

#[test]
fn eligible_competing_stream_truncates_only_a_follower_that_can_still_stop() {
    let (mut world, _, follower) = at_follow_request(3);
    let other = world.live_vehicles()[2];
    let state = world.vehicle(follower).unwrap();
    let current = world.state.compiled_route(state.route()).unwrap().conflicts[0];
    let revision = world.revision();
    let topology = revision.conflict();
    let other_stream = *topology
        .conflict_zone(current.static_zone().unwrap())
        .unwrap()
        .participant_streams()
        .iter()
        .find(|s| **s != current.stream)
        .unwrap();
    let path = topology
        .participant_stream(other_stream)
        .unwrap()
        .maneuver_path();
    let edges = revision
        .traffic()
        .maneuvers()
        .maneuver_path(path)
        .unwrap()
        .edges()
        .to_vec();
    let other_route = world
        .register_route(crate::RouteRegisterInput::new(edges))
        .unwrap();
    let compiled = world.state.compiled_route(other_route).unwrap();
    let admission = compiled.conflicts[0].admission_hop;
    let boundary =
        revision.traffic().lane_lengths_millimetres()[compiled.edges[admission as usize].index()];
    world.state.committed.routes[state.route().index() as usize].live_vehicles -= 1;
    world.state.committed.routes[other_route.index() as usize].live_vehicles += 1;
    let other_state = world.vehicle(other).unwrap();
    world
        .state
        .committed
        .vehicles
        .slot_mut(other.index() as usize)
        .state = Some(crate::VehicleState {
        route: other_route,
        route_edge_index: admission,
        progress_mm: boundary,
        carry_um: 0,
        speed_mm_s: 0,
        maneuver_traversal: Some(crate::ManeuverTraversalState {
            route: other_route,
            maneuver_occurrence_index: 0,
            phase: crate::ManeuverTraversalPhase::PreGate {
                next_gate_hop: admission,
            },
        }),
        ..other_state
    });
    let loc = world
        .conflict_passage_occurrence_locator(other_route, 0)
        .unwrap();
    world.state.committed.conflict_eligibility.resize(3, None);
    world.state.committed.conflict_eligibility[other.index() as usize] =
        crate::ConflictEligibilityState::update(None, loc, true, world.tick_index());
    let mut stopped = state;
    stopped.progress_mm += 1;
    stopped.carry_um = 0;
    stopped.speed_mm_s = 0;
    world
        .state
        .committed
        .vehicles
        .slot_mut(follower.index() as usize)
        .state = Some(stopped);
    world.state.invalidate_occupancy_source();
    world.state.rebuild_occupancy_index().unwrap();
    let read = world.state.read_view();
    // 本夹具紧急减速度 4 m/s²，1 m/s 的连续停车距离恰为 125 mm。
    // 两种离散积分余数均须让行；把车头再向前移一毫米才进入不可停区。
    for carry_um in [0, 1, 999] {
        let approaching = crate::VehicleState {
            progress_mm: stopped.progress_mm - 125,
            speed_mm_s: 1_000,
            carry_um,
            ..stopped
        };
        assert_eq!(
            read.ordinary_platoon_truncation(
                approaching,
                current.admission_hop,
                read.conflict_read()
            )
            .unwrap(),
            Some(true)
        );
        assert_eq!(
            read.ordinary_platoon_truncation(
                crate::VehicleState {
                    progress_mm: approaching.progress_mm + 1,
                    ..approaching
                },
                current.admission_hop,
                read.conflict_read()
            )
            .unwrap(),
            Some(false)
        );
    }
    assert_eq!(
        read.ordinary_platoon_truncation(stopped, current.admission_hop, read.conflict_read())
            .unwrap(),
        Some(true)
    );
    assert_eq!(
        read.ordinary_platoon_truncation(
            crate::VehicleState {
                speed_mm_s: 2_000,
                ..stopped
            },
            current.admission_hop,
            read.conflict_read()
        )
        .unwrap(),
        Some(false)
    );
    world.step(crate::TickInput::new(4)).unwrap();
    assert!(
        world
            .latest_conflict_decisions()
            .iter()
            .any(|decision| decision.vehicle() == follower
                && decision.outcome()
                    == crate::ConflictDecisionOutcome::NoGrant(
                        crate::ConflictNoGrantReason::PlatoonTruncated
                    ))
    );
}
