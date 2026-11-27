use super::*;
use laneflow_runtime::{
    RouteHandoffApproachError, RouteHandoffEntrantError, RouteHandoffEntrantReason, TickInput,
    TrafficWorld, VehicleSpawnInput, deterministic_state_digest,
};

#[test]
fn a_target_follower_can_require_a_stop_proof_before_it_can_reach_the_sweep_this_tick() {
    let revision = compile_road_editing_revision(module(0.));
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::try_new(WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 32,
            fixed_delta_time_ms: 100,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    let rel = revision.traffic().relations();
    let section = [0, 1]
        .into_iter()
        .map(RoadSectionOrdinal::from_raw)
        .find(|&s| rel.section_lanes(s).unwrap().len() == 2)
        .unwrap();
    let lanes = rel.section_lanes(section).unwrap();
    let src = rel.authoring_edge_chain(lanes[0]).unwrap();
    let dst = rel.authoring_edge_chain(lanes[1]).unwrap();
    let profile = conflict_profile(&revision, "car");
    let binding = world
        .bind_route_handoff(
            &RouteRegisterInput::new(vec![src[0], src[1], src[2], dst[1], dst[2]]),
            RouteHandoffOccurrence {
                revision: revision.network_revision(),
                source_index: 2,
                owner: BoundaryOwner::Section(section),
                left_index: 0,
                stations_nm: [24_000_000_000, 26_000_000_000],
            },
        )
        .unwrap();
    let trial = RouteHandoffTrial {
        profile: revision.identity().stable_id(profile).unwrap(),
        station_window_nm: [0, 50_000_000_000],
        rear_stations_nm: [10_000_000_000, 40_000_000_000],
        speed_m_s: 10.,
        limits: GeometryWorkLimits::STANDARD,
    };
    let probe = |world: &TrafficWorld, work| {
        world.preview_route_handoff_approaches(
            &binding,
            2,
            &trial,
            12_500_000..=13_500_000,
            5_000_000,
            work,
        )
    };
    let initial = probe(&world, u32::MAX).unwrap();
    assert_eq!(initial.entrants.side_blockers, [None; 2]);
    assert!(initial.swept_sides.iter().all(Option::is_some));
    let preview = world.preview_route_handoff_at(&binding, 2, &trial).unwrap();
    let travel_work = |distance_um| {
        preview
            .travel(
                HandoffTravelTarget::DistanceNm(distance_um * 1_000),
                5_000_000,
                trial.limits.tracks,
            )
            .unwrap()
            .work
    };
    assert_eq!(
        initial.travel_work,
        travel_work(12_500_000_u64) + travel_work(13_500_000)
    );
    let stopped = world
        .preview_route_handoff_approaches(
            &binding,
            2,
            &trial,
            12_500_000..=12_500_000,
            5_000_000,
            0,
        )
        .unwrap();
    assert_eq!(stopped.travel_work, travel_work(12_500_000));
    let first = initial.swept_sides[1]
        .as_ref()
        .unwrap()
        .addresses()
        .next()
        .unwrap();
    assert_eq!(first.edge, dst[1]);
    let front = first.progress_mm[0].checked_sub(8_000).unwrap();
    let target_route = world
        .register_route(RouteRegisterInput::new(dst.to_vec()))
        .unwrap();
    let target = world
        .spawn_vehicle(VehicleSpawnInput::new(
            profile,
            target_route,
            1,
            front,
            10_000,
        ))
        .unwrap();
    let digest = |world: &TrafficWorld| {
        (
            world.command_cursor(),
            world.event_cursor(),
            world.observation_state_sequence(),
            deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap(),
        )
    };
    let before = digest(&world);
    let check = probe(&world, u32::MAX).unwrap();
    let blocked = check.entrants.side_blockers[1].as_ref().unwrap();
    assert_eq!(blocked.vehicle, target);
    assert_eq!(blocked.distance_um, 8_000_000);
    assert_eq!(
        blocked.reason,
        RouteHandoffEntrantReason::CannotStop {
            required_um: Some(20_666_667),
            may_reach_this_tick: false,
        }
    );
    assert!(check.entrants.side_blockers[0].is_none());
    assert_eq!(probe(&world, check.entrants.work), Ok(check.clone()));
    let short = check.entrants.work - 1;
    assert_eq!(
        probe(&world, short),
        Err(RouteHandoffApproachError::Entrants(
            RouteHandoffEntrantError::Budget { work: short },
        ))
    );
    assert_eq!(digest(&world), before);
    // 普通车实际步进后重查同一个候选，使用新提交位置和速度。
    world.step(TickInput::new(100)).unwrap();
    let after = probe(&world, u32::MAX).unwrap();
    assert!(
        after.entrants.side_blockers[1]
            .as_ref()
            .unwrap()
            .distance_um
            < blocked.distance_um
    );
    world.despawn_vehicle(target).unwrap();
    let slow = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, target_route, 1, front, 0))
        .unwrap();
    assert_ne!(slow, target);
    assert_eq!(
        probe(&world, u32::MAX).unwrap().entrants.side_blockers,
        [None; 2]
    );

    // 车头跨过目标侧独立切点，车尾仍在被保护的物理区间内。
    world.despawn_vehicle(slow).unwrap();
    let overlapping = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, target_route, 2, 0, 0))
        .unwrap();
    let occupant = probe(&world, u32::MAX).unwrap().entrants.side_blockers[1].unwrap();
    assert_eq!(
        (occupant.vehicle, occupant.reason),
        (overlapping, RouteHandoffEntrantReason::Occupied)
    );
    for invalid in [0..=u64::MAX, std::ops::RangeInclusive::new(1, 0)] {
        assert_eq!(
            world.preview_route_handoff_approaches(
                &binding,
                2,
                &trial,
                invalid,
                5_000_000,
                u32::MAX
            ),
            Err(RouteHandoffApproachError::InvalidTravelRange),
        );
    }
}
