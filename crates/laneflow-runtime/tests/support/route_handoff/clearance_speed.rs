use super::*;
use RoadEditingLaneDirection::{Backward, Forward};
use laneflow_runtime::{
    BoundRouteHandoff, ClearanceSpeedCurve, ClearanceSpeedPoint, HandoffDrivingConfig,
    RouteHandoffSpeedPlanError, TrafficWorld, deterministic_state_digest,
};
use laneflow_static_contract::MAX_SPEED_MM_S;
use laneflow_static_network::vehicle::{Failure, SweepOutcome};
use std::num::NonZeroU32;

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
                stations_mm: [24_000, 26_000],
            },
        )
        .unwrap();
    let trial = RouteHandoffTrial {
        profile: revision
            .identity()
            .stable_id(conflict_profile(&revision, "car"))
            .unwrap(),
        station_window_mm: [0, 50_000],
        rear_stations_mm: [start, end].map(|s| s * 1_000),
        speed_m_s: 8.,
        limits: GeometryWorkLimits::STANDARD,
    };
    (world, binding, trial)
}

fn driving_config() -> HandoffDrivingConfig {
    HandoffDrivingConfig {
        curve: ClearanceSpeedCurve::try_new(vec![
            ClearanceSpeedPoint {
                clearance_mm: 50,
                speed_mm_s: 1_000,
            },
            ClearanceSpeedPoint {
                clearance_mm: 150,
                speed_mm_s: MAX_SPEED_MM_S,
            },
        ])
        .unwrap(),
        max_span_mm: NonZeroU32::new(3_000).unwrap(),
        travel_tolerance_mm: NonZeroU32::new(5).unwrap(),
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
    assert!(slow.travel_mm[0] > 2_000, "{plan:?}");
    assert!(
        slow.travel_mm[1] + 2_000 < plan.length_lower_mm(),
        "{plan:?}"
    );
    assert_eq!(
        plan.preview_braking_step(0, 2_000, 2_000)
            .unwrap()
            .next_speed_mm_s,
        2_000
    );
    assert_eq!(
        plan.preview_braking_step(slow.travel_mm[1], 1_000, 1_200)
            .unwrap()
            .next_speed_mm_s,
        1_000
    );
    assert_eq!(
        plan.preview_braking_step(slow.travel_mm[1] + 1, 1_000, 1_200)
            .unwrap()
            .next_speed_mm_s,
        1_200
    );
    // 2 m/s 降至 1 m/s，服务减速度 3 m/s² 时需要整 0.5 m。100 ms 拍末为 1.7 m/s。
    let braking = plan
        .preview_braking_step(slow.travel_mm[0] - 500, 2_000, 2_000)
        .unwrap();
    assert_eq!(
        (braking.next_speed_mm_s, braking.travel_upper_mm),
        (1_700, 185)
    );

    let mut tiny_segments = config;
    tiny_segments.max_span_mm = NonZeroU32::MIN;
    assert!(matches!(
        compute(&tiny_segments),
        Err(RouteHandoffSpeedPlanError::Clearance {
            reason: Failure::WorkLimit,
            ..
        })
    ));
    assert_eq!(snapshot(), before);

    // 逐次把立柱外移一毫米。整毫米下界从 50 变为 51 时，
    // 曲线由首点的 1,000 毫米/秒进入首个插值档 1,990 毫米/秒。
    let mut previous = slow.speed_mm_s;
    let mut crossed = false;
    for offset_mm in 1..=8 {
        let offset_m = PASSING_OFFSET_M - f64::from(offset_mm) / 1_000.;
        let (wider, binding, trial) = world_with_post(0., offset_m, Forward);
        let plan = wider
            .preview_route_handoff_speed_plan(&binding, 2, &trial, &config)
            .unwrap();
        let [window] = plan.windows() else {
            panic!("{plan:?}")
        };
        assert!(window.speed_mm_s >= previous);
        crossed |= (previous, window.speed_mm_s) == (1_000, 1_990);
        previous = window.speed_mm_s;
    }
    assert!(crossed);

    let (other, binding, trial) = world_with_post(4., PASSING_OFFSET_M, Forward);
    let raised = other
        .preview_route_handoff_speed_plan(&binding, 2, &trial, &config)
        .unwrap();
    assert!(raised.windows().is_empty(), "{raised:?}");
    assert_eq!(
        raised
            .preview_braking_step(slow.travel_mm[0], 2_000, 2_000)
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
fn restoring_an_active_handoff_checks_its_speed_at_the_saved_rear_travel() {
    use crate::{snapshot_wire as wire, wire_table_field_offset};
    use laneflow_runtime::{
        SnapshotRestoreError, SnapshotRestoreLimits, TickInput, encode_lfrs, restore_lfrs,
    };
    let (fixture, binding, mut trial) = world_with_post(0., -2.443, Forward);
    let (mut world, request, config) =
        execution::drive_revision(2_000, 100, fixture.revision(), driving_config());
    trial.speed_m_s = request.speed_m_s;
    let plan = world
        .preview_route_handoff_speed_plan(&binding, 2, &trial, config.handoff_driving().unwrap())
        .unwrap();
    let [slow] = plan.windows() else {
        panic!("{plan:?}")
    };
    let tick = TickInput::new(100);
    world.step_with_route_handoffs(tick, &[request]).unwrap();
    let check = |world: &TrafficWorld, valid: u32, invalid: u32| {
        let mut bytes = encode_lfrs(&world.capture_snapshot().unwrap());
        let root = wire::size_prefixed_root_as_runtime_snapshot(&bytes).unwrap();
        let vehicle = root.vehicles().get(0);
        let offset = wire_table_field_offset(vehicle._tab, wire::SnapshotVehicle::VT_SPEED_MM_S);
        let restore = |bytes: &[u8]| {
            restore_lfrs(
                bytes,
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                world.execution_config(),
                SnapshotRestoreLimits::new(4 << 20, 4096),
            )
        };
        bytes[offset..offset + 4].copy_from_slice(&invalid.to_le_bytes());
        assert_eq!(
            restore(&bytes).unwrap_err(),
            SnapshotRestoreError::InvalidHandoffState
        );
        bytes[offset..offset + 4].copy_from_slice(&valid.to_le_bytes());
        restore(&bytes).unwrap().into_world().step(tick).unwrap();
    };
    assert!(
        world
            .vehicle_route_handoffs(request.vehicle)
            .next()
            .unwrap()
            .rear_travel_mm
            < slow.travel_mm[0]
    );
    check(&world, 3_000, 3_001);
    let mut checked_approach = false;
    for _ in 0..300 {
        world.step(tick).unwrap();
        let travel = world
            .vehicle_route_handoffs(request.vehicle)
            .next()
            .unwrap()
            .rear_travel_mm;
        // 服务减速度 3 m/s²，从 3 m/s 降到 1 m/s 需要超过一米。
        // 剩余不足半米时，3 m/s 虽仍在本格上限以内，已不能安全接近下一格。
        if !checked_approach && travel < slow.travel_mm[0] && slow.travel_mm[0] - travel < 500 {
            check(&world, 1_000, 3_000);
            checked_approach = true;
        }
        if (slow.travel_mm[0]..=slow.travel_mm[1]).contains(&travel) {
            assert!(checked_approach);
            assert_eq!(slow.speed_mm_s, 1_000);
            check(&world, slow.speed_mm_s, slow.speed_mm_s + 1);
            return;
        }
    }
    panic!("车辆未进入立柱旁的局部限速区间");
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
    let [entry, exit] = slow.travel_mm;
    // 车从 40 米站位驶向 10 米；22 米端帽之前、整车仍与它并行和车尾清空时，
    // 三个后轴站位分别落在限速窗口之前、之中和之后。
    let preview = world.preview_route_handoff(&binding, &trial).unwrap();
    let at_station = |metres: u64| {
        preview
            .travel(
                RouteHandoffTravelTarget::RearStationMm(metres * 1_000),
                config.travel_tolerance_mm,
                trial.limits.tracks,
            )
            .unwrap()
            .bounds
            .unwrap()
            .distance
            .millimetres()
    };
    assert!(at_station(23)[1] < entry);
    let alongside = at_station(21);
    assert!(entry <= alongside[0] && alongside[1] <= exit);
    assert!(exit < at_station(18)[0]);
    assert_eq!(
        plan.preview_braking_step(entry - 500, 2_000, 2_000)
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
