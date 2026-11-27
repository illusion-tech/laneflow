use super::stop_line::road_lanes;
use super::*;
use laneflow_runtime::{
    ClearanceSpeedCurve, ClearanceSpeedPoint, HandoffDrivingConfig, RouteHandoffRequest,
    SnapshotRestoreLimits, TickInput, TrafficWorld, VehicleSpawnInput, deterministic_state_digest,
    encode_lfrs, restore_lfrs,
};
use laneflow_static_contract::MAX_SPEED_MM_S;
use std::num::NonZeroU32;

pub(super) fn drive_fixture(
    initial_speed: u32,
) -> (TrafficWorld, RouteHandoffRequest, WorldConfig) {
    drive_at_interval(initial_speed, 100)
}

pub(super) fn drive_at_interval(
    initial_speed: u32,
    dt: u64,
) -> (TrafficWorld, RouteHandoffRequest, WorldConfig) {
    drive_on(initial_speed, dt, module(0.))
}
fn drive_on(
    initial_speed: u32,
    dt: u64,
    source: RoadEditingSourceModule,
) -> (TrafficWorld, RouteHandoffRequest, WorldConfig) {
    let revision = compile_road_editing_revision(source);
    let config = WorldConfig::try_new(WorldConfigInput {
        vehicle_capacity: 2,
        route_capacity: 2,
        route_edge_occurrence_capacity: 32,
        route_conflict_occurrence_capacity: 16,
        fixed_delta_time_ms: dt,
    })
    .unwrap()
    .with_handoff_driving(HandoffDrivingConfig {
        curve: ClearanceSpeedCurve::try_new(vec![
            ClearanceSpeedPoint {
                clearance_mm: 0,
                speed_mm_s: 1_000,
            },
            ClearanceSpeedPoint {
                clearance_mm: 100,
                speed_mm_s: MAX_SPEED_MM_S,
            },
        ])
        .unwrap(),
        max_span_mm: NonZeroU32::new(3_000).unwrap(),
        travel_tolerance_mm: NonZeroU32::new(5).unwrap(),
    });
    let mut world = install_fixture(Arc::clone(&revision), config).unwrap();
    let (section, src, dst) = road_lanes(&revision);
    let occurrence = RouteHandoffOccurrence {
        revision: revision.network_revision(),
        source_index: 2,
        owner: BoundaryOwner::Section(section),
        left_index: 0,
        stations_mm: [24_000, 26_000],
    };
    let route = world
        .register_route(
            RouteRegisterInput::new(vec![src[0], src[1], src[2], dst[1], dst[2]])
                .with_handoffs(vec![occurrence]),
        )
        .unwrap();
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            conflict_profile(&revision, "car"),
            route,
            1,
            3_400,
            initial_speed,
        ))
        .unwrap();
    let request = RouteHandoffRequest {
        vehicle,
        route,
        revision: revision.network_revision(),
        source_index: 2,
        station_window_mm: [0, 50_000],
        rear_stations_mm: [10_000, 40_000],
        speed_m_s: 3.,
        limits: GeometryWorkLimits::STANDARD,
    };
    (world, request, config)
}

#[test]
fn a_red_target_approach_brakes_the_active_handoff_and_green_releases_it() {
    let source = road_module(
        0.,
        None,
        RoadEditingLaneDirection::Forward,
        Some(stop_line::SourceStop::TargetSignal),
    );
    let (mut world, request, _) = drive_on(3_000, 1_000, source);
    let owner = request.vehicle;
    world
        .step_with_route_handoffs(TickInput::new(1000), &[request])
        .unwrap();
    assert!(world.vehicle_route_handoffs(owner).next().is_some());
    let mut stopped = false;
    let mut proceeded = false;
    for tick in 2..=20 {
        world
            .step(TickInput::new(1000))
            .unwrap_or_else(|e| panic!("signal tick {tick}: {e}"));
        let state = world.vehicle(owner).unwrap();
        if tick <= 10 {
            assert!(state.route_edge_index() < 4);
            if state.speed_mm_s() == 0 {
                assert!(world.vehicle_route_handoffs(owner).next().is_some());
                stopped = true;
            }
        } else if state.route_edge_index() >= 4 {
            proceeded = true;
            break;
        }
    }
    assert!(stopped && proceeded);
}

#[test]
fn registered_vehicle_executes_its_handoff_and_releases_only_after_the_tail_clears() {
    let (mut world, request, config) = drive_at_interval(3_000, 1_000);
    let vehicle = request.vehicle;
    world
        .step_with_route_handoffs(TickInput::new(1_000), &[request])
        .unwrap();
    let initial = world
        .vehicle_route_handoffs(vehicle)
        .next()
        .expect("first tick must enter");
    assert!(initial.rear_travel_mm > 0 && !initial.front_handed_off);
    let mut saw_target_while_held = false;
    let mut restored = None;
    for tick in 2..=160 {
        world
            .step(TickInput::new(1_000))
            .unwrap_or_else(|e| panic!("tick {tick}: {e}"));
        if let Some((other, owner)) = &mut restored {
            let other: &mut TrafficWorld = other;
            other.step(TickInput::new(1_000)).unwrap();
            assert_eq!(
                world.vehicle_route_handoffs(vehicle).next(),
                other.vehicle_route_handoffs(*owner).next()
            );
            assert_eq!(
                deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap(),
                deterministic_state_digest(&other.capture_snapshot().unwrap()).unwrap()
            );
        }
        let Some(p) = world.vehicle_route_handoffs(vehicle).next() else {
            assert!(saw_target_while_held);
            assert!(world.vehicle(vehicle).unwrap().route_edge_index() >= 3);
            return;
        };
        if p.front_handed_off {
            assert!(world.vehicle(vehicle).unwrap().route_edge_index() >= 3);
            if restored.is_none() {
                let rebuilt = restore_lfrs(
                    &encode_lfrs(&world.capture_snapshot().unwrap()),
                    world.revision(),
                    world.committed_source().clone(),
                    config,
                    world.execution_config(),
                    SnapshotRestoreLimits::new(4 << 20, 4096),
                )
                .unwrap();
                let owner = rebuilt.vehicle_handle(1).unwrap();
                restored = Some((rebuilt.into_world(), owner));
            }
            saw_target_while_held = true;
        }
    }
    panic!("vehicle did not complete its 30 m handoff");
}

#[test]
fn a_front_within_one_millimetre_keeps_moving_across_tick_79_and_restores_identically() {
    let (mut world, request, config) = drive_fixture(2_000);
    world
        .step_with_route_handoffs(TickInput::new(100), &[request])
        .unwrap();
    for tick in 2..79 {
        world
            .step(TickInput::new(100))
            .unwrap_or_else(|error| panic!("tick {tick}: {error}"));
    }
    let restored = restore_lfrs(
        &encode_lfrs(&world.capture_snapshot().unwrap()),
        world.revision(),
        world.committed_source().clone(),
        config,
        world.execution_config(),
        SnapshotRestoreLimits::new(4 << 20, 4096),
    )
    .unwrap();
    let restored_owner = restored.vehicle_handle(1).unwrap();
    let mut other = restored.into_world();
    let progress = world
        .vehicle_route_handoffs(request.vehicle)
        .next()
        .unwrap();
    assert!(progress.front_handed_off);
    for tick in 79..=150 {
        world
            .step(TickInput::new(100))
            .unwrap_or_else(|error| panic!("tick {tick}: {error}"));
        other.step(TickInput::new(100)).unwrap();
        let stage = world.vehicle_route_handoffs(request.vehicle).next();
        assert_eq!(stage, other.vehicle_route_handoffs(restored_owner).next());
        assert_eq!(
            deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap(),
            deterministic_state_digest(&other.capture_snapshot().unwrap()).unwrap()
        );
        if tick == 79 {
            assert!(stage.unwrap().rear_travel_mm > progress.rear_travel_mm);
            let view = world
                .committed_handoff_geometry(request.vehicle)
                .unwrap()
                .unwrap();
            let front = view.footprint.front.addresses().next().unwrap();
            assert_eq!(
                front.progress_mm[0],
                world.vehicle(request.vehicle).unwrap().progress_mm()
            );
            assert_eq!(world.vehicle(request.vehicle).unwrap().carry_um(), 0);
        }
        if stage.is_none() {
            return;
        }
    }
    panic!("横移应在已证整车清空后完成");
}

#[test]
fn reserved_dual_side_front_keeps_its_source_cursor_until_the_target_is_proven() {
    let source = road_module_with_width(0., None, RoadEditingLaneDirection::Forward, None, 3.5004);
    let (mut world, request, config) = drive_on(2_000, 100, source);
    let tick = TickInput::new(100);
    world.step_with_route_handoffs(tick, &[request]).unwrap();
    for _ in 2..=41 {
        world.step(tick).unwrap();
    }
    let view = world
        .committed_handoff_geometry(request.vehicle)
        .unwrap()
        .unwrap();
    let target_front = view
        .footprint
        .possible_target_front
        .as_ref()
        .expect("毫米误差外包跨共边");
    assert!(view.footprint.sides.iter().all(Option::is_some));
    assert_eq!(view.front_occurrence, request.source_index);
    assert!(
        !world
            .vehicle_route_handoffs(request.vehicle)
            .next()
            .unwrap()
            .front_handed_off
    );
    let src = view.footprint.front.addresses().next().unwrap();
    let dst = *target_front;
    assert_ne!(src.edge, dst.edge);
    let revision = world.revision();
    let (section, _, target_chain) = road_lanes(&revision);
    let candidate = world
        .bind_route_handoff(
            &RouteRegisterInput::new(world.route_edges(request.route).unwrap().to_vec()),
            RouteHandoffOccurrence {
                revision: request.revision,
                source_index: request.source_index,
                owner: BoundaryOwner::Section(section),
                left_index: 0,
                stations_mm: [24_000, 26_000],
            },
        )
        .unwrap();
    let mut trial = stop_line::car_trial(&revision, request.rear_stations_mm);
    trial.speed_m_s = request.speed_m_s;
    let strict = world
        .preview_route_handoff(&candidate, &trial)
        .unwrap()
        .sample_after_travel(
            view.rear_travel_mm,
            0,
            std::num::NonZeroU32::MIN,
            request.limits,
        )
        .unwrap();
    assert_eq!(
        strict.sample.unwrap_err(),
        laneflow_runtime::RouteHandoffSampleError::Footprint(
            laneflow_static_network::vehicle::Failure::Boundary
        )
    );
    let index = target_chain.iter().position(|&e| e == dst.edge).unwrap();
    let target = world
        .register_route(RouteRegisterInput::new(target_chain.to_vec()))
        .unwrap();
    let other_car = VehicleSpawnInput::new(
        conflict_profile(&revision, "car"),
        target,
        u32::try_from(index).unwrap(),
        dst.progress_mm[1] + 2_000,
        0,
    );
    assert_eq!(
        world.spawn_vehicle(other_car),
        Err(laneflow_runtime::SpawnError::StopConstraintUnsatisfiable)
    );
    let bytes = encode_lfrs(&world.capture_snapshot().unwrap());
    let restored = restore_lfrs(
        &bytes,
        world.revision(),
        world.committed_source().clone(),
        config,
        world.execution_config(),
        SnapshotRestoreLimits::new(4 << 20, 4096),
    )
    .unwrap();
    let restored_owner = restored.vehicle_handle(1).unwrap();
    let mut other = restored.into_world();
    let restored_view = other
        .committed_handoff_geometry(restored_owner)
        .unwrap()
        .unwrap();
    assert_eq!(restored_view.footprint, view.footprint);
    world.step(tick).unwrap();
    other.step(tick).unwrap();
    assert!(
        world
            .vehicle_route_handoffs(request.vehicle)
            .next()
            .unwrap()
            .front_handed_off
    );
    assert_eq!(
        deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap(),
        deterministic_state_digest(&other.capture_snapshot().unwrap()).unwrap()
    );
}

#[test]
fn four_millisecond_ticks_accumulate_low_speed_travel_across_a_millimetre() {
    let (mut world, mut request, config) = drive_at_interval(100, 4);
    request.speed_m_s = 0.1;
    world
        .step_with_route_handoffs(TickInput::new(4), &[request])
        .unwrap();
    let first = world
        .vehicle_route_handoffs(request.vehicle)
        .next()
        .unwrap();
    let start = world.vehicle(request.vehicle).unwrap().progress_mm();
    assert_eq!(first.rear_travel_mm, 0);
    // 第一拍仅走 0.4 毫米。冷恢复后第三拍也应进入第一个完整毫米。
    let restored = restore_lfrs(
        &encode_lfrs(&world.capture_snapshot().unwrap()),
        world.revision(),
        world.committed_source().clone(),
        config,
        world.execution_config(),
        SnapshotRestoreLimits::new(4 << 20, 4096),
    )
    .unwrap();
    let restored_owner = restored.vehicle_handle(1).unwrap();
    let mut resumed = restored.into_world();
    for expected_mm in [0, 1] {
        world.step(TickInput::new(4)).unwrap();
        resumed.step(TickInput::new(4)).unwrap();
        let progress = world
            .vehicle_route_handoffs(request.vehicle)
            .next()
            .unwrap();
        assert_eq!(progress.rear_travel_mm, expected_mm);
        assert_eq!(
            resumed.vehicle_route_handoffs(restored_owner).next(),
            Some(progress)
        );
    }
    assert_eq!(
        deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap(),
        deterministic_state_digest(&resumed.capture_snapshot().unwrap()).unwrap()
    );
    for _ in 3..250 {
        world.step(TickInput::new(4)).unwrap();
    }
    let end = world
        .vehicle_route_handoffs(request.vehicle)
        .next()
        .unwrap();
    assert_eq!(end.rear_travel_mm, 100);
    let front = world.vehicle(request.vehicle).unwrap();
    assert!((99..=101).contains(&(front.progress_mm() - start)));
    assert_eq!(front.carry_um(), 0);
}

#[test]
fn a_proven_last_millimetre_preserves_the_following_ticks_target_travel() {
    // 五次横移从 10.000 m 到 39.709 m，后轴弧长约 30.000927 m。
    // 第 30 拍累计 30 m，仍在候选上；下一拍在目标边延续接近一整米。
    let (mut world, mut request, _) = drive_at_interval(1_000, 1_000);
    request.speed_m_s = 1.;
    request.rear_stations_mm[1] = 39_709;
    world
        .step_with_route_handoffs(TickInput::new(1000), &[request])
        .unwrap();
    for tick in 2..=30 {
        world
            .step(TickInput::new(1000))
            .unwrap_or_else(|e| panic!("tick {tick}: {e}"));
    }
    let last = world
        .vehicle_route_handoffs(request.vehicle)
        .next()
        .unwrap();
    assert!(last.front_handed_off);
    assert_eq!(last.rear_travel_mm, 30_000);
    let before = world.vehicle(request.vehicle).unwrap();
    world.step(TickInput::new(1000)).unwrap();
    assert!(
        world
            .vehicle_route_handoffs(request.vehicle)
            .next()
            .is_none()
    );
    let after = world.vehicle(request.vehicle).unwrap();
    assert_eq!(before.route_edge_index(), after.route_edge_index());
    assert!((998..=1002).contains(&(after.progress_mm() - before.progress_mm())));
}
