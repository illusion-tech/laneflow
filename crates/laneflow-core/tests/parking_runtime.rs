use laneflow_core::{
    CoreError, CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge,
    LaneGraph, LeaveParkingInput, ParkedVehicleSpawnInput, ParkingApproachState, ParkingArea,
    ParkingBindingKind, ParkingCommandEffect, ParkingRegistry, ParkingReleaseReason, ParkingSpace,
    ParkingSpaceGeometry, ParkingSpaceState, RebindReservedVehicleRouteInput, Route,
    SignalRegistry, Speed, TickInput, VehicleParkingState, VehicleProfile, VehicleProfileHandle,
    VehicleProfileRegistry, VehicleSpawnInput, VehicleStatus,
};

fn profile_registry() -> (VehicleProfileRegistry, VehicleProfileHandle) {
    let registry = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        "car",
        IidmProfileSpec {
            length: 5.0,
            desired_speed: 20.0,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 2.0,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 8.0,
        },
    )
    .expect("profile")])
    .expect("profiles");
    let handle = registry.profile_handle("car").expect("profile handle");
    (registry, handle)
}

fn parking_space(id: &str, area: Option<&str>, entry: &str, exit: &str) -> ParkingSpace {
    ParkingSpace::new(
        id,
        area.map(str::to_owned),
        entry,
        20.0,
        exit,
        40.0,
        ParkingSpaceGeometry::new(-3.0, 0.0, 5.0, 2.4),
    )
}

fn single_edge_world() -> (CoreWorld, VehicleProfileHandle) {
    let graph = LaneGraph::try_new([LaneEdge::new(
        "A",
        EdgeLength::try_new(100.0).expect("length"),
        std::iter::empty::<&str>(),
    )])
    .expect("graph");
    let parking = ParkingRegistry::try_new(
        &graph,
        [ParkingArea::new("lot")],
        [
            parking_space("S1", Some("lot"), "A", "A"),
            parking_space("S2", Some("lot"), "A", "A"),
        ],
    )
    .expect("parking");
    let (profiles, profile) = profile_registry();
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        graph,
        [Route::try_new("R", ["A"]).expect("route")],
        profiles,
        SignalRegistry::empty(),
        parking,
    )
    .expect("traffic");
    (
        CoreWorld::with_traffic_data(1_000, traffic, Vec::new()).expect("world"),
        profile,
    )
}

fn spawn_active(
    world: &mut CoreWorld,
    profile: VehicleProfileHandle,
    id: &str,
    progress: f64,
    speed: f64,
) -> laneflow_core::VehicleHandle {
    world
        .spawn_vehicle(VehicleSpawnInput::active(
            id,
            profile,
            "R",
            0,
            EdgeProgress::try_new(progress).expect("progress"),
            Speed::try_new(speed).expect("speed"),
        ))
        .expect("spawn")
}

#[test]
fn reservation_snapshot_counts_guard_and_cancel_are_atomic() {
    let (mut world, profile) = single_edge_world();
    let vehicle = spawn_active(&mut world, profile, "V", 0.0, 0.0);
    let area = world.parking().area_handle("lot").expect("area");
    let space = world.parking().space_handle("S1").expect("space");

    assert_eq!(world.parking_snapshot().counts().capacity, 2);
    assert_eq!(world.parking_snapshot().counts().vacant, 2);
    assert_eq!(
        world.parking_snapshot().area_counts(area).unwrap().vacant,
        2
    );
    assert_eq!(
        world.reserve_parking_space(vehicle, space).unwrap().effect,
        ParkingCommandEffect::Applied
    );
    assert_eq!(
        world.reserve_parking_space(vehicle, space).unwrap().effect,
        ParkingCommandEffect::AlreadySatisfied
    );
    assert_eq!(
        world.parking_snapshot().space_state(space),
        Some(ParkingSpaceState::Reserved { vehicle })
    );
    assert_eq!(world.parking_snapshot().counts().reserved, 1);
    std::assert_matches!(
        world.parking_snapshot().vehicle_state(vehicle),
        Some(VehicleParkingState::Reserved {
            space: bound_space,
            approach: ParkingApproachState::Approaching { .. },
        }) if bound_space == space
    );

    let before = world.clone();
    std::assert_matches!(
        world.step(TickInput::new(1_000)),
        Err(CoreError::ParkingVehicleCapabilityUnavailable)
    );
    assert_eq!(world, before);
    assert_eq!(
        world
            .cancel_parking_reservation(vehicle, space)
            .unwrap()
            .effect,
        ParkingCommandEffect::Applied
    );
    assert_eq!(
        world
            .cancel_parking_reservation(vehicle, space)
            .unwrap()
            .effect,
        ParkingCommandEffect::AlreadySatisfied
    );
    assert_eq!(world.parking_snapshot().counts().vacant, 2);
    world.step(TickInput::new(1_000)).expect("guard cleared");
}

#[test]
fn immediate_arrival_commit_and_leave_switch_position_authority() {
    let (mut world, profile) = single_edge_world();
    let vehicle = spawn_active(&mut world, profile, "V", 20.0, 0.0);
    let space = world.parking().space_handle("S1").expect("space");
    let route = world.route_handle("R").expect("route");

    world.reserve_parking_space(vehicle, space).unwrap();
    assert_eq!(
        world.parking_snapshot().vehicle_state(vehicle),
        Some(VehicleParkingState::Reserved {
            space,
            approach: ParkingApproachState::Arrived {
                route,
                route_edge_index: 0,
            },
        })
    );
    assert_eq!(
        world.commit_parking(vehicle, space).unwrap().effect,
        ParkingCommandEffect::Applied
    );
    assert_eq!(
        world.vehicle(vehicle).unwrap().status,
        VehicleStatus::Parked
    );
    assert_eq!(world.parking_snapshot().counts().occupied, 1);
    world
        .step(TickInput::new(1_000))
        .expect("occupied-only step");
    assert_eq!(world.vehicle(vehicle).unwrap().edge_progress.value(), 20.0);

    let input = LeaveParkingInput {
        vehicle,
        space,
        route,
        route_edge_index: 0,
    };
    assert_eq!(
        world.leave_parking(input).unwrap().effect,
        ParkingCommandEffect::Applied
    );
    let state = world.vehicle(vehicle).unwrap();
    assert_eq!(state.status, VehicleStatus::Active);
    assert_eq!(state.edge_progress.value(), 40.0);
    assert_eq!(state.current_speed, Speed::ZERO);
    assert_eq!(
        world.leave_parking(input).unwrap().effect,
        ParkingCommandEffect::AlreadySatisfied
    );
    assert_eq!(world.parking_snapshot().counts().vacant, 2);
}

#[test]
fn parked_spawn_and_despawn_preserve_identity_and_release_binding() {
    let (mut world, profile) = single_edge_world();
    let space = world.parking().space_handle("S1").expect("space");
    let route = world.route_handle("R").expect("route");
    let record = world
        .spawn_parked_vehicle(ParkedVehicleSpawnInput {
            id: "P".to_owned(),
            profile,
            route_id: "R".to_owned(),
            route_edge_index: 0,
            space,
        })
        .expect("parked spawn");
    assert_eq!(
        world.vehicle(record.vehicle).unwrap().status,
        VehicleStatus::Parked
    );
    std::assert_matches!(
        world.remove_route(route),
        Err(CoreError::RouteInUse { vehicle, .. }) if vehicle == record.vehicle
    );
    std::assert_matches!(
        world.spawn_vehicle(VehicleSpawnInput::new(
            "invalid-parked",
            profile,
            "R",
            0,
            EdgeProgress::try_new(20.0).unwrap(),
            Speed::ZERO,
            VehicleStatus::Parked,
        )),
        Err(CoreError::ParkedVehicleRequiresParkingCommand { .. })
    );

    let despawn = world
        .despawn_vehicle(record.vehicle)
        .expect("despawn parked");
    let release = despawn.parking_release.expect("occupied release");
    assert_eq!(release.space, space);
    assert_eq!(release.previous_binding, ParkingBindingKind::Occupied);
    assert_eq!(world.parking_snapshot().counts().vacant, 2);
    world
        .remove_route(route)
        .expect("route no longer referenced");
}

#[test]
fn leave_rejects_overlap_and_unsafe_active_follower_without_mutation() {
    let (mut overlap, profile) = single_edge_world();
    let space = overlap.parking().space_handle("S1").unwrap();
    let route = overlap.route_handle("R").unwrap();
    let parked = overlap
        .spawn_parked_vehicle(ParkedVehicleSpawnInput {
            id: "P".into(),
            profile,
            route_id: "R".into(),
            route_edge_index: 0,
            space,
        })
        .unwrap()
        .vehicle;
    spawn_active(&mut overlap, profile, "leader", 42.0, 0.0);
    let input = LeaveParkingInput {
        vehicle: parked,
        space,
        route,
        route_edge_index: 0,
    };
    let before = overlap.clone();
    std::assert_matches!(
        overlap.leave_parking(input),
        Err(CoreError::VehiclePhysicalOverlap { .. })
    );
    assert_eq!(overlap, before);

    let (mut unsafe_world, profile) = single_edge_world();
    let space = unsafe_world.parking().space_handle("S1").unwrap();
    let route = unsafe_world.route_handle("R").unwrap();
    let parked = unsafe_world
        .spawn_parked_vehicle(ParkedVehicleSpawnInput {
            id: "P".into(),
            profile,
            route_id: "R".into(),
            route_edge_index: 0,
            space,
        })
        .unwrap()
        .vehicle;
    let follower = spawn_active(&mut unsafe_world, profile, "follower", 30.0, 10.0);
    let input = LeaveParkingInput {
        vehicle: parked,
        space,
        route,
        route_edge_index: 0,
    };
    let before = unsafe_world.clone();
    std::assert_matches!(
        unsafe_world.leave_parking(input),
        Err(CoreError::ParkingLeaveUnsafeFollower { follower: actual, .. }) if actual == follower
    );
    assert_eq!(unsafe_world, before);
}

#[test]
fn dormant_reservation_rebinds_without_teleport_and_keeps_route_references_exact() {
    let graph = LaneGraph::try_new([
        LaneEdge::new("A", EdgeLength::try_new(100.0).unwrap(), ["B"]),
        LaneEdge::new(
            "B",
            EdgeLength::try_new(100.0).unwrap(),
            std::iter::empty::<&str>(),
        ),
    ])
    .unwrap();
    let parking =
        ParkingRegistry::try_new(&graph, [], [parking_space("S", None, "B", "A")]).unwrap();
    let (profiles, profile) = profile_registry();
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        graph,
        [
            Route::try_new("short", ["A"]).unwrap(),
            Route::try_new("target", ["A", "B"]).unwrap(),
            Route::try_new("unreachable", ["A"]).unwrap(),
        ],
        profiles,
        SignalRegistry::empty(),
        parking,
    )
    .unwrap();
    let mut world = CoreWorld::with_traffic_data(1_000, traffic, Vec::new()).unwrap();
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::active(
            "V",
            profile,
            "short",
            0,
            EdgeProgress::try_new(10.0).unwrap(),
            Speed::ZERO,
        ))
        .unwrap();
    let space = world.parking().space_handle("S").unwrap();
    world.reserve_parking_space(vehicle, space).unwrap();
    assert_eq!(
        world.parking_snapshot().vehicle_state(vehicle),
        Some(VehicleParkingState::Reserved {
            space,
            approach: ParkingApproachState::Dormant,
        })
    );

    let unreachable = world.route_handle("unreachable").unwrap();
    let before = world.clone();
    std::assert_matches!(
        world.rebind_reserved_vehicle_route(RebindReservedVehicleRouteInput {
            vehicle,
            space,
            route: unreachable,
            route_edge_index: 0,
        }),
        Err(CoreError::ParkingEntryUnreachable { .. })
    );
    assert_eq!(world, before);

    let target = world.route_handle("target").unwrap();
    let record = world
        .rebind_reserved_vehicle_route(RebindReservedVehicleRouteInput {
            vehicle,
            space,
            route: target,
            route_edge_index: 0,
        })
        .unwrap();
    assert_eq!(record.effect, ParkingCommandEffect::Applied);
    assert_eq!(world.vehicle(vehicle).unwrap().edge_progress.value(), 10.0);
    assert_eq!(world.vehicle(vehicle).unwrap().route, target);
    assert_eq!(
        world.parking_snapshot().vehicle_state(vehicle),
        Some(VehicleParkingState::Reserved {
            space,
            approach: ParkingApproachState::Approaching {
                route: target,
                route_edge_index: 1,
            },
        })
    );
    let short = world.route_handle("short").unwrap();
    world.remove_route(short).expect("old route detached");
}

#[test]
fn command_error_priority_and_pair_mismatches_are_atomic() {
    let (mut world, profile) = single_edge_world();
    let first = spawn_active(&mut world, profile, "first", 0.0, 0.0);
    let second = spawn_active(&mut world, profile, "second", 80.0, 0.0);
    let first_space = world.parking().space_handle("S1").unwrap();
    let second_space = world.parking().space_handle("S2").unwrap();
    world.reserve_parking_space(first, first_space).unwrap();

    let before = world.clone();
    std::assert_matches!(
        world.reserve_parking_space(first, second_space),
        Err(CoreError::ParkingVehicleAlreadyBound {
            vehicle,
            current_space,
            ..
        }) if vehicle == first && current_space == first_space
    );
    assert_eq!(world, before);

    let before = world.clone();
    std::assert_matches!(
        world.reserve_parking_space(second, first_space),
        Err(CoreError::ParkingSpaceUnavailable {
            requested_vehicle,
            current_vehicle,
            ..
        }) if requested_vehicle == second && current_vehicle == first
    );
    assert_eq!(world, before);

    let before = world.clone();
    std::assert_matches!(
        world.cancel_parking_reservation(first, second_space),
        Err(CoreError::ParkingReservationMismatch { vehicle, space, .. })
            if vehicle == first && space == second_space
    );
    assert_eq!(world, before);

    let before = world.clone();
    std::assert_matches!(
        world.commit_parking(first, second_space),
        Err(CoreError::ParkingReservationMismatch { vehicle, space, .. })
            if vehicle == first && space == second_space
    );
    assert_eq!(world, before);

    let before = world.clone();
    std::assert_matches!(
        world.commit_parking(first, first_space),
        Err(CoreError::ParkingVehicleNotArrived { vehicle, space })
            if vehicle == first && space == first_space
    );
    assert_eq!(world, before);
}

#[test]
fn reserved_despawn_releases_binding_and_clears_step_guard() {
    let (mut world, profile) = single_edge_world();
    let vehicle = spawn_active(&mut world, profile, "reserved", 0.0, 0.0);
    let space = world.parking().space_handle("S1").unwrap();
    world.reserve_parking_space(vehicle, space).unwrap();

    let record = world.despawn_vehicle(vehicle).expect("despawn reserved");
    assert_eq!(
        record.parking_release,
        Some(laneflow_core::ParkingReleaseRecord {
            vehicle,
            space,
            previous_binding: ParkingBindingKind::Reserved,
            reason: ParkingReleaseReason::VehicleDespawn,
        })
    );
    assert_eq!(
        world.parking_snapshot().space_state(space),
        Some(ParkingSpaceState::Vacant)
    );
    assert_eq!(world.parking_snapshot().vehicle_state(vehicle), None);
    world
        .step(TickInput::new(1_000))
        .expect("reservation guard cleared by despawn");
}

#[test]
fn stopped_direct_follower_allows_safe_leave() {
    let (mut world, profile) = single_edge_world();
    let space = world.parking().space_handle("S1").unwrap();
    let route = world.route_handle("R").unwrap();
    let parked = world
        .spawn_parked_vehicle(ParkedVehicleSpawnInput {
            id: "parked".into(),
            profile,
            route_id: "R".into(),
            route_edge_index: 0,
            space,
        })
        .unwrap()
        .vehicle;
    world
        .spawn_vehicle(VehicleSpawnInput::stopped(
            "stopped-follower",
            profile,
            "R",
            0,
            EdgeProgress::try_new(30.0).unwrap(),
        ))
        .expect("stopped follower");

    let record = world
        .leave_parking(LeaveParkingInput {
            vehicle: parked,
            space,
            route,
            route_edge_index: 0,
        })
        .expect("stopped follower has no emergency travel");
    assert_eq!(record.effect, ParkingCommandEffect::Applied);
    assert_eq!(world.vehicle(parked).unwrap().status, VehicleStatus::Active);
}

#[test]
fn stale_vehicle_handle_is_rejected_before_parking_state_checks() {
    let (mut world, profile) = single_edge_world();
    let stale = spawn_active(&mut world, profile, "old", 0.0, 0.0);
    world.despawn_vehicle(stale).expect("despawn old");
    let replacement = spawn_active(&mut world, profile, "new", 0.0, 0.0);
    assert_ne!(replacement, stale);
    let space = world.parking().space_handle("S1").unwrap();
    let before = world.clone();

    std::assert_matches!(
        world.reserve_parking_space(stale, space),
        Err(CoreError::UnknownVehicleHandle { vehicle }) if vehicle == stale
    );
    assert_eq!(world, before);
}
