use super::*;
use RoadEditingLaneDirection::{Backward, Forward};
use laneflow_runtime::{
    BoundRouteHandoff, ClearanceSpeedCurve, ClearanceSpeedPoint, HandoffDrivingConfig,
    RouteHandoffSpeedPlanError, TrafficWorld, deterministic_state_digest,
};
use laneflow_static_contract::MAX_SPEED_MM_S;
use laneflow_static_network::vehicle::{Failure, SweepOutcome};
use std::num::NonZeroU64;

fn world_with_post(
    base_m: f64,
    offset_m: f64,
    direction: RoadEditingLaneDirection,
) -> (TrafficWorld, BoundRouteHandoff, RouteHandoffTrial) {
    let [left, right] = ["source", "target"].map(|key| {
        BoundaryMemberInput::AuthoringLane(
            AuthoringLaneReference::owner_scoped(vec!["road".into(), "section".into()], key)
                .unwrap(),
        )
    });
    let (low, high, anchor_role, start, end) = match direction {
        Forward => (
            BoundaryStation::Fixed(28.),
            BoundaryStation::CorridorEndpoint,
            1,
            10,
            40,
        ),
        Backward => (
            BoundaryStation::CorridorEndpoint,
            BoundaryStation::Fixed(22.),
            2,
            40,
            10,
        ),
    };
    let post = BoundaryExtentInput {
        left,
        right,
        kind: BoundaryExtentKind::Obstacle,
        local_key: "post".into(),
        low,
        high,
        payload: Some(BoundaryPayloadInput::Guardrail {
            anchor_role,
            lateral_offset_meters: offset_m,
            horizontal_length_meters: 0.2,
            radius_meters: 0.12,
            bottom_height_meters: base_m,
            top_height_meters: base_m + 1.5,
            parameter_error_meters: 0.,
        }),
        canvas_selection: None,
    };
    let revision = compile_road_editing_revision(module_with_boundary(0., Some(post), direction));
    let world = install_fixture(
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
    let relations = revision.traffic().relations();
    let section = [0, 1]
        .into_iter()
        .map(RoadSectionOrdinal::from_raw)
        .find(|&section| relations.section_lanes(section).unwrap().len() == 2)
        .unwrap();
    let lanes = relations.section_lanes(section).unwrap();
    let src = relations.authoring_edge_chain(lanes[0]).unwrap();
    let dst = relations.authoring_edge_chain(lanes[1]).unwrap();
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
        profile: revision
            .identity()
            .stable_id(conflict_profile(&revision, "car"))
            .unwrap(),
        station_window_nm: [0, 50_000_000_000],
        rear_stations_nm: [start, end].map(|s| s * 1_000_000_000),
        speed_m_s: 8.,
        limits: GeometryWorkLimits::STANDARD,
    };
    (world, binding, trial)
}

fn driving_config() -> HandoffDrivingConfig {
    HandoffDrivingConfig {
        curve: ClearanceSpeedCurve::try_new(vec![
            ClearanceSpeedPoint {
                clearance_um: 50_000,
                speed_mm_s: 1_000,
            },
            ClearanceSpeedPoint {
                clearance_um: 150_000,
                speed_mm_s: MAX_SPEED_MM_S,
            },
        ])
        .unwrap(),
        max_span_nm: NonZeroU64::new(3_000_000_000).unwrap(),
        travel_tolerance_nm: NonZeroU64::new(5_000_000).unwrap(),
    }
}

#[test]
fn the_ground_post_limits_its_passage_and_the_raised_copy_leaves_it_unrestricted() {
    const PASSING_OFFSET_M: f64 = -2.443;
    let config = driving_config();
    let (world, binding, trial) = world_with_post(0., PASSING_OFFSET_M, Forward);
    let snapshot = || {
        (
            world.command_cursor(),
            world.event_cursor(),
            world.observation_state_sequence(),
            deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap(),
        )
    };
    let before = snapshot();
    let compute = |cfg: &HandoffDrivingConfig| {
        world.preview_route_handoff_speed_plan(&binding, 2, &trial, cfg)
    };
    let plan = compute(&config).unwrap();
    assert_eq!(compute(&config).unwrap(), plan);
    let [slow] = plan.windows() else {
        panic!("{plan:?}")
    };
    assert_eq!(slow.speed_mm_s, 1_000);
    // 先于路墩的候选头部、车尾越过路墩的候选末部都允许较高车速。
    assert!(slow.travel_um[0] > 2_000_000, "{plan:?}");
    assert!(
        slow.travel_um[1] + 2_000_000 < plan.length_lower_um(),
        "{plan:?}"
    );
    assert_eq!(
        plan.preview_braking_step(0, 2_000, 2_000)
            .unwrap()
            .next_speed_mm_s,
        2_000
    );
    assert_eq!(
        plan.preview_braking_step(slow.travel_um[1], 1_000, 1_200)
            .unwrap()
            .next_speed_mm_s,
        1_000
    );
    assert_eq!(
        plan.preview_braking_step(slow.travel_um[1] + 1, 1_000, 1_200)
            .unwrap()
            .next_speed_mm_s,
        1_200
    );
    // 2 m/s 降至 1 m/s，服务减速度 3 m/s² 时需要整 0.5 m。100 ms 拍末为 1.7 m/s。
    let braking = plan
        .preview_braking_step(slow.travel_um[0] - 500_000, 2_000, 2_000)
        .unwrap();
    assert_eq!(
        (braking.next_speed_mm_s, braking.travel_upper_um),
        (1_700, 185_000)
    );

    let mut tiny_segments = config.clone();
    tiny_segments.max_span_nm = NonZeroU64::MIN;
    assert!(matches!(
        compute(&tiny_segments),
        Err(RouteHandoffSpeedPlanError::Clearance {
            reason: Failure::WorkLimit,
            ..
        })
    ));
    assert_eq!(snapshot(), before);

    // 路墩向外平移 1 mm，使已证余量从约 49.28 mm 增至 50.26 mm，进入插值段。
    let (wider, binding, trial) = world_with_post(0., PASSING_OFFSET_M - 0.001, Forward);
    let wider_plan = wider
        .preview_route_handoff_speed_plan(&binding, 2, &trial, &config)
        .unwrap();
    let [wider_window] = wider_plan.windows() else {
        panic!("{wider_plan:?}")
    };
    assert!(
        (1_001..2_000).contains(&wider_window.speed_mm_s),
        "{wider_window:?}"
    );

    let (other, binding, trial) = world_with_post(4., PASSING_OFFSET_M, Forward);
    let raised = other
        .preview_route_handoff_speed_plan(&binding, 2, &trial, &config)
        .unwrap();
    assert!(raised.windows().is_empty(), "{raised:?}");
    assert_eq!(
        raised
            .preview_braking_step(slow.travel_um[0], 2_000, 2_000)
            .unwrap()
            .next_speed_mm_s,
        2_000
    );
}

#[test]
fn a_positive_creep_speed_does_not_admit_a_same_height_body_contact() {
    let config = driving_config();
    let (blocking, binding, trial) = world_with_post(0., 0., Forward);
    let preview = blocking.preview_route_handoff(&binding, &trial).unwrap();
    let swept = preview.report.geometry.unwrap().sweep.outcome;
    assert!(
        matches!(swept, SweepOutcome::Contact { boundary, .. } if boundary.key == Some("post")),
        "{swept:?}"
    );
    assert_eq!(
        blocking.preview_route_handoff_speed_plan(&binding, 2, &trial, &config),
        Err(RouteHandoffSpeedPlanError::Unlocated(
            HandoffOutcome::GeometryRejected
        ))
    );
}

#[test]
fn decreasing_station_brakes_before_the_post_and_releases_after_rear_clearance() {
    let (world, binding, trial) = world_with_post(0., -2.443, Backward);
    let config = driving_config();
    let plan = world
        .preview_route_handoff_speed_plan(&binding, 2, &trial, &config)
        .unwrap();
    let [slow] = plan.windows() else {
        panic!("{plan:?}")
    };
    assert_eq!(slow.speed_mm_s, 1_000);
    let [entry, exit] = slow.travel_um;
    // 车从 40 米站位驶向 10 米；22 米端帽之前、整车仍与它并行和车尾清空时，
    // 三个后轴站位分别落在限速窗口之前、之中和之后。
    let preview = world.preview_route_handoff(&binding, &trial).unwrap();
    let at_station = |metres: u64| {
        preview
            .travel(
                HandoffTravelTarget::RearStationNm(metres * 1_000_000_000),
                config.travel_tolerance_nm.get(),
                trial.limits.tracks,
            )
            .unwrap()
            .bounds
            .unwrap()
            .distance_nm
    };
    assert!(at_station(23)[1] < entry * 1_000);
    let alongside = at_station(21);
    assert!(entry * 1_000 <= alongside[0] && alongside[1] <= exit * 1_000);
    assert!(exit * 1_000 < at_station(18)[0]);
    assert_eq!(
        plan.preview_braking_step(entry - 500_000, 2_000, 2_000)
            .unwrap()
            .next_speed_mm_s,
        1_700
    );
    assert_eq!(
        plan.preview_braking_step(exit, 1_000, 1_200)
            .unwrap()
            .next_speed_mm_s,
        1_000
    );
    assert_eq!(
        plan.preview_braking_step(exit + 1, 1_000, 1_200)
            .unwrap()
            .next_speed_mm_s,
        1_200
    );
}
