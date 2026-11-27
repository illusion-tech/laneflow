use super::*;
use laneflow_runtime::{
    RouteHandoffSampleError, RouteHandoffTrafficError, TrafficWorld, VehicleSpawnInput,
    deterministic_state_digest,
};
use laneflow_static_network::vehicle::HandoffLaneSide;

#[test]
fn external_trial_checks_current_bodies_on_both_physical_lanes_until_the_tail_clears() {
    let revision = compile_road_editing_revision(module(0.));
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 32, 100)).unwrap();
    let rel = revision.traffic().relations();
    let section = [0, 1]
        .into_iter()
        .map(RoadSectionOrdinal::from_raw)
        .find(|&s| rel.section_lanes(s).unwrap().len() == 2)
        .unwrap();
    let lanes = rel.section_lanes(section).unwrap();
    let src = rel.authoring_edge_chain(lanes[0]).unwrap();
    let dst = rel.authoring_edge_chain(lanes[1]).unwrap();
    let route = RouteRegisterInput::new(vec![src[0], src[1], src[2], dst[1], dst[2]]);
    let occurrence = RouteHandoffOccurrence {
        revision: revision.network_revision(),
        source_index: 2,
        owner: BoundaryOwner::Section(section),
        left_index: 0,
        stations_nm: [24_000_000_000, 26_000_000_000],
    };
    let binding = world.bind_route_handoff(&route, occurrence).unwrap();
    let profile = conflict_profile(&revision, "car");
    let trial = RouteHandoffTrial {
        profile: revision.identity().stable_id(profile).unwrap(),
        station_window_nm: [0, 50_000_000_000],
        rear_stations_nm: [10_000_000_000, 40_000_000_000],
        speed_m_s: 1.,
        limits: GeometryWorkLimits::STANDARD,
    };
    let traffic = |world: &mut TrafficWorld, rear, step| {
        world
            .preview_route_handoff_traffic(&binding, 2, &trial, rear, step, 5_000_000)
            .unwrap()
    };
    let clear = traffic(&mut world, 12_000_000, 3_000_000);
    assert_eq!(clear.side_blockers, [None; 2]);
    assert_eq!(clear.sample.footprint.front_side, HandoffLaneSide::Target);
    let source = clear.sample.footprint.sides[0].as_ref().unwrap();
    assert!(
        source
            .enclosure
            .addresses()
            .any(|a| a.edge == src[3] && a.progress_mm[1] >= 100),
        "{source:?}"
    );
    assert!(!route.edges().contains(&src[3]));

    // 目标车很长一段在 t1 上，前杠已经到 t2。源侧车辆整车位于路线未经过的 s3。
    let target_route = world
        .register_route(RouteRegisterInput::new(dst.to_vec()))
        .unwrap();
    let target = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, target_route, 2, 1_000, 0))
        .unwrap();
    let source_route = world
        .register_route(RouteRegisterInput::new(vec![src[3]]))
        .unwrap();
    let source = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, source_route, 0, 4_250, 0))
        .unwrap();
    let state = |world: &TrafficWorld| {
        (
            world.command_cursor(),
            world.event_cursor(),
            world.observation_state_sequence(),
            deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap(),
        )
    };
    let before = state(&world);
    let both = traffic(&mut world, 12_000_000, 3_000_000);
    assert_eq!(both.sample, clear.sample);
    assert_eq!(both.side_blockers, [Some(source), Some(target)]);
    assert_eq!(both.blockers().collect::<Vec<_>>(), [source, target]);
    let same_vehicle = laneflow_runtime::RouteHandoffTraffic {
        side_blockers: [Some(target); 2],
        ..both.clone()
    };
    assert_eq!(same_vehicle.blockers().collect::<Vec<_>>(), [target]);
    assert!(both.checked_edges.into_iter().all(|n| (1..=8).contains(&n)));
    assert_eq!(traffic(&mut world, 15_000_000, 0), both);

    // 车尾已清空后，即使源车还在，源侧也无需查桶。
    let late = traffic(&mut world, 20_000_000, 3_000_000);
    assert!(late.sample.footprint.sides[0].is_none());
    assert_eq!((late.side_blockers[0], late.checked_edges[0]), (None, 0));
    assert_eq!(late.side_blockers[1], None);
    let early = traffic(&mut world, 0, 0);
    assert_eq!(early.side_blockers, [None; 2]);
    assert_eq!(early.checked_edges[1], 0);
    assert_eq!(
        world
            .preview_route_handoff_traffic(&binding, 1, &trial, 0, 0, 5_000_000)
            .unwrap_err(),
        RouteHandoffTrafficError::Trial(RouteHandoffTrialError::UnboundOccurrence)
    );
    assert_eq!(
        world
            .preview_route_handoff_traffic(&binding, 2, &trial, u64::MAX, 1, 5_000_000)
            .unwrap_err(),
        RouteHandoffTrafficError::Sample(RouteHandoffSampleError::DistanceOverflow)
    );
    let fixed = RouteHandoffTrial {
        profile: revision
            .identity()
            .stable_id(conflict_profile(&revision, "fixed"))
            .unwrap(),
        ..trial
    };
    assert_eq!(
        world
            .preview_route_handoff_traffic(&binding, 2, &fixed, 0, 0, 5_000_000)
            .unwrap_err(),
        RouteHandoffTrafficError::Unlocated(HandoffOutcome::GeometryRejected)
    );
    assert_eq!(state(&world), before);

    world.despawn_vehicle(source).unwrap();
    assert_eq!(
        traffic(&mut world, 15_000_000, 0).side_blockers,
        [None, Some(target)]
    );
    let replacement = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, source_route, 0, 4_250, 0))
        .unwrap();
    assert_ne!(replacement, source);
    assert_eq!(
        traffic(&mut world, 15_000_000, 0).side_blockers,
        [Some(replacement), Some(target)]
    );
}
