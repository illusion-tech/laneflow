use laneflow_core::{
    CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge, LaneGraph,
    LeaveParkingInput, ParkedVehicleSpawnInput, ParkingCommandEffect, ParkingRegistry,
    ParkingSpace, ParkingSpaceGeometry, ParkingSpaceHandle, RebindReservedVehicleRouteInput, Route,
    RouteHandle, SignalRegistry, Speed, VehicleHandle, VehicleProfile, VehicleProfileHandle,
    VehicleProfileRegistry, VehicleSpawnInput,
};

pub const FIXED_PARKING_COMMAND_COUNT: usize = 100;

pub struct ParkingCommandScenario {
    pub world: CoreWorld,
    pub pairs: Vec<(VehicleHandle, ParkingSpaceHandle)>,
}

#[derive(Clone)]
pub struct ParkingSixCommandScenario {
    pub world: CoreWorld,
    pairs: Vec<ParkingSixCommandPair>,
    target_route: RouteHandle,
    profile: VehicleProfileHandle,
}

#[derive(Clone)]
pub struct ParkingPathologicalLeaveScenario {
    pub world: CoreWorld,
    fastest_first: Vec<VehicleHandle>,
    leave: LeaveParkingInput,
}

#[derive(Clone, Copy)]
struct ParkingSixCommandPair {
    cycle_vehicle: VehicleHandle,
    cycle_space: ParkingSpaceHandle,
    dormant_vehicle: VehicleHandle,
    dormant_space: ParkingSpaceHandle,
    spawn_space: ParkingSpaceHandle,
}

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
    .expect("parking profile")])
    .expect("parking profiles");
    let handle = registry
        .profile_handle("car")
        .expect("parking profile handle");
    (registry, handle)
}

fn parking_space(id: impl Into<String>) -> ParkingSpace {
    ParkingSpace::new(
        id,
        None,
        "A",
        20.0,
        "A",
        40.0,
        ParkingSpaceGeometry::new(-3.0, 0.0, 5.0, 2.4),
    )
}

fn traffic_with_spaces(
    spaces: impl IntoIterator<Item = ParkingSpace>,
) -> (InitialTrafficData, VehicleProfileHandle) {
    let graph = LaneGraph::try_new([LaneEdge::new(
        "A",
        EdgeLength::try_new(2_000.0).expect("parking edge length"),
        laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
        std::iter::empty::<&str>(),
    )])
    .expect("parking graph");
    let parking = ParkingRegistry::try_new(&graph, [], spaces).expect("parking registry");
    let (profiles, profile) = profile_registry();
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        graph,
        [Route::try_new("R", ["A"]).expect("parking route")],
        profiles,
        SignalRegistry::empty(),
        parking,
    )
    .expect("parking traffic");
    (traffic, profile)
}

pub fn single_parking_world(progress: f64) -> (CoreWorld, VehicleHandle, ParkingSpaceHandle) {
    let (traffic, profile) = traffic_with_spaces([parking_space("S")]);
    let mut world = CoreWorld::with_traffic_data(
        20,
        traffic,
        vec![VehicleSpawnInput::active(
            "V",
            profile,
            "R",
            0,
            EdgeProgress::try_new(progress).expect("vehicle progress"),
            Speed::ZERO,
        )],
    )
    .expect("single parking world");
    let vehicle = world.vehicle_handle("V").expect("vehicle handle");
    let space = world.parking().space_handle("S").expect("space handle");
    world
        .cancel_parking_reservation(vehicle, space)
        .expect("warm no-op cancellation");
    (world, vehicle, space)
}

pub fn parking_command_scenario(
    vehicle_count: usize,
    command_count: usize,
) -> ParkingCommandScenario {
    assert!(command_count > 0);
    assert!(vehicle_count >= command_count);
    let spaces = (0..command_count).map(|index| parking_space(format!("S{index:04}")));
    let (traffic, profile) = traffic_with_spaces(spaces);
    let completed_count = vehicle_count - command_count;
    let mut vehicles = Vec::with_capacity(vehicle_count);
    for index in 0..completed_count {
        vehicles.push(VehicleSpawnInput::completed(
            format!("B{index:06}"),
            profile,
            "R",
            0,
            EdgeProgress::try_new(2_000.0).expect("route end"),
        ));
    }
    for index in 0..command_count {
        vehicles.push(VehicleSpawnInput::active(
            format!("C{index:04}"),
            profile,
            "R",
            0,
            EdgeProgress::try_new(100.0 + 10.0 * index as f64).expect("command progress"),
            Speed::ZERO,
        ));
    }
    let world = CoreWorld::with_traffic_data(20, traffic, vehicles).expect("command world");
    let pairs = (0..command_count)
        .map(|index| {
            (
                world
                    .vehicle_handle(&format!("C{index:04}"))
                    .expect("command vehicle"),
                world
                    .parking()
                    .space_handle(&format!("S{index:04}"))
                    .expect("command space"),
            )
        })
        .collect();
    ParkingCommandScenario { world, pairs }
}

pub fn run_reserve_cancel_batch(scenario: &mut ParkingCommandScenario) -> usize {
    let mut applied = 0;
    for (vehicle, space) in scenario.pairs.iter().copied() {
        let reserved = scenario
            .world
            .reserve_parking_space(vehicle, space)
            .expect("benchmark reserve");
        applied += usize::from(reserved.effect == ParkingCommandEffect::Applied);
        let cancelled = scenario
            .world
            .cancel_parking_reservation(vehicle, space)
            .expect("benchmark cancel");
        applied += usize::from(cancelled.effect == ParkingCommandEffect::Applied);
    }
    applied
}

pub fn parking_six_command_scenario(
    vehicle_count: usize,
    command_count: usize,
) -> ParkingSixCommandScenario {
    assert!(command_count > 0);
    assert!(vehicle_count >= command_count * 2);
    let graph = LaneGraph::try_new([
        LaneEdge::new(
            "A",
            EdgeLength::try_new(10_000.0).expect("six-command A length"),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["B"],
        ),
        LaneEdge::new(
            "B",
            EdgeLength::try_new(10_000.0).expect("six-command B length"),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("six-command graph");
    let spaces = (0..command_count).flat_map(|index| {
        let base = 100.0 + 40.0 * index as f64;
        [
            ParkingSpace::new(
                format!("cycle-{index:04}"),
                None,
                "A",
                base,
                "A",
                base + 10.0,
                ParkingSpaceGeometry::new(-3.0, 0.0, 5.0, 2.4),
            ),
            ParkingSpace::new(
                format!("dormant-{index:04}"),
                None,
                "B",
                base,
                "A",
                base + 30.0,
                ParkingSpaceGeometry::new(3.0, 0.0, 5.0, 2.4),
            ),
            ParkingSpace::new(
                format!("spawn-{index:04}"),
                None,
                "B",
                base + 20.0,
                "B",
                base + 30.0,
                ParkingSpaceGeometry::new(-3.0, 0.0, 5.0, 2.4),
            ),
        ]
    });
    let parking = ParkingRegistry::try_new(&graph, [], spaces).expect("six-command parking");
    let (profiles, profile) = profile_registry();
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        graph,
        [
            Route::try_new("short", ["A"]).expect("short route"),
            Route::try_new("target", ["A", "B"]).expect("target route"),
        ],
        profiles,
        SignalRegistry::empty(),
        parking,
    )
    .expect("six-command traffic");
    let mut vehicles = Vec::with_capacity(vehicle_count);
    for index in 0..vehicle_count - command_count * 2 {
        vehicles.push(VehicleSpawnInput::completed(
            format!("background-{index:06}"),
            profile,
            "short",
            0,
            EdgeProgress::try_new(10_000.0).expect("route end"),
        ));
    }
    for index in 0..command_count {
        let base = 100.0 + 40.0 * index as f64;
        vehicles.push(VehicleSpawnInput::active(
            format!("cycle-vehicle-{index:04}"),
            profile,
            "short",
            0,
            EdgeProgress::try_new(base).expect("cycle progress"),
            Speed::ZERO,
        ));
        vehicles.push(VehicleSpawnInput::active(
            format!("dormant-vehicle-{index:04}"),
            profile,
            "short",
            0,
            EdgeProgress::try_new(base + 20.0).expect("dormant progress"),
            Speed::ZERO,
        ));
    }
    let world = CoreWorld::with_traffic_data(20, traffic, vehicles).expect("six-command world");
    let target_route = world.route_handle("target").expect("target route handle");
    let pairs = (0..command_count)
        .map(|index| ParkingSixCommandPair {
            cycle_vehicle: world
                .vehicle_handle(&format!("cycle-vehicle-{index:04}"))
                .expect("cycle vehicle"),
            cycle_space: world
                .parking()
                .space_handle(&format!("cycle-{index:04}"))
                .expect("cycle space"),
            dormant_vehicle: world
                .vehicle_handle(&format!("dormant-vehicle-{index:04}"))
                .expect("dormant vehicle"),
            dormant_space: world
                .parking()
                .space_handle(&format!("dormant-{index:04}"))
                .expect("dormant space"),
            spawn_space: world
                .parking()
                .space_handle(&format!("spawn-{index:04}"))
                .expect("spawn space"),
        })
        .collect::<Vec<_>>();
    ParkingSixCommandScenario {
        world,
        pairs,
        target_route,
        profile,
    }
}

pub fn run_six_command_batch(scenario: &mut ParkingSixCommandScenario) -> usize {
    let mut applied = 0;
    let short_route = scenario
        .world
        .route_handle("short")
        .expect("short route handle");
    for (index, pair) in scenario.pairs.iter().copied().enumerate() {
        let reserved = scenario
            .world
            .reserve_parking_space(pair.cycle_vehicle, pair.cycle_space)
            .expect("six-command reserve cycle");
        applied += usize::from(reserved.effect == ParkingCommandEffect::Applied);
        let committed = scenario
            .world
            .commit_parking(pair.cycle_vehicle, pair.cycle_space)
            .expect("six-command commit");
        applied += usize::from(committed.effect == ParkingCommandEffect::Applied);
        let left = scenario
            .world
            .leave_parking(LeaveParkingInput {
                vehicle: pair.cycle_vehicle,
                space: pair.cycle_space,
                route: short_route,
                route_edge_index: 0,
            })
            .expect("six-command leave");
        applied += usize::from(left.effect == ParkingCommandEffect::Applied);

        let reserved = scenario
            .world
            .reserve_parking_space(pair.dormant_vehicle, pair.dormant_space)
            .expect("six-command reserve dormant");
        applied += usize::from(reserved.effect == ParkingCommandEffect::Applied);
        let rebound = scenario
            .world
            .rebind_reserved_vehicle_route(RebindReservedVehicleRouteInput {
                vehicle: pair.dormant_vehicle,
                space: pair.dormant_space,
                route: scenario.target_route,
                route_edge_index: 0,
            })
            .expect("six-command rebind");
        applied += usize::from(rebound.effect == ParkingCommandEffect::Applied);
        let cancelled = scenario
            .world
            .cancel_parking_reservation(pair.dormant_vehicle, pair.dormant_space)
            .expect("six-command cancel");
        applied += usize::from(cancelled.effect == ParkingCommandEffect::Applied);

        scenario
            .world
            .spawn_parked_vehicle(ParkedVehicleSpawnInput {
                id: format!("spawned-{index:04}"),
                profile: scenario.profile,
                route_id: "target".to_owned(),
                route_edge_index: 1,
                space: pair.spawn_space,
            })
            .expect("six-command parked spawn");
        applied += 1;
    }
    applied
}

pub fn warm_six_command_batch(scenario: &mut ParkingSixCommandScenario) -> usize {
    let mut applied = 0;
    for pair in scenario.pairs.iter().copied() {
        let reserved = scenario
            .world
            .reserve_parking_space(pair.cycle_vehicle, pair.cycle_space)
            .expect("warm reserve cycle");
        applied += usize::from(reserved.effect == ParkingCommandEffect::Applied);
        let cancelled = scenario
            .world
            .cancel_parking_reservation(pair.cycle_vehicle, pair.cycle_space)
            .expect("warm cancel cycle");
        applied += usize::from(cancelled.effect == ParkingCommandEffect::Applied);

        let reserved = scenario
            .world
            .reserve_parking_space(pair.dormant_vehicle, pair.dormant_space)
            .expect("warm reserve dormant");
        applied += usize::from(reserved.effect == ParkingCommandEffect::Applied);
        let cancelled = scenario
            .world
            .cancel_parking_reservation(pair.dormant_vehicle, pair.dormant_space)
            .expect("warm cancel dormant");
        applied += usize::from(cancelled.effect == ParkingCommandEffect::Applied);
    }
    applied
}

pub fn warm_six_command_spawn_capacity(scenario: &mut ParkingSixCommandScenario) -> usize {
    let mut applied = 0;
    for (index, pair) in scenario.pairs.iter().copied().enumerate() {
        let warm = scenario
            .world
            .spawn_parked_vehicle(ParkedVehicleSpawnInput {
                id: format!("warm-spawned-{index:04}"),
                profile: scenario.profile,
                route_id: "target".to_owned(),
                route_edge_index: 1,
                space: pair.spawn_space,
            })
            .expect("warm parked spawn");
        applied += 1;
        scenario
            .world
            .despawn_vehicle(warm.vehicle)
            .expect("warm parked despawn");
        applied += 1;
    }
    applied
}

pub fn parking_pathological_leave_scenario(
    background_count: usize,
    fast_count: usize,
) -> ParkingPathologicalLeaveScenario {
    assert!(background_count > 0);
    assert!(fast_count > 0);
    let edge_length = 10.0 * background_count as f64 + 100.0;
    let exit_progress = edge_length - 10.0;
    let fast_edge_length = 10.0 * fast_count as f64 + 100.0;
    let graph = LaneGraph::try_new([
        LaneEdge::new(
            "A",
            EdgeLength::try_new(edge_length).expect("pathological A length"),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
        LaneEdge::new(
            "C",
            EdgeLength::try_new(fast_edge_length).expect("pathological C length"),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("pathological graph");
    let parking = ParkingRegistry::try_new(
        &graph,
        [],
        [ParkingSpace::new(
            "pathological-space",
            None,
            "A",
            exit_progress - 10.0,
            "A",
            exit_progress,
            ParkingSpaceGeometry::new(-3.0, 0.0, 4.5, 2.4),
        )],
    )
    .expect("pathological parking");
    let (profiles, profile) = profile_registry();
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        graph,
        [
            Route::try_new("R", ["A"]).expect("pathological route"),
            Route::try_new("fast-route", ["C"]).expect("pathological fast route"),
        ],
        profiles,
        SignalRegistry::empty(),
        parking,
    )
    .expect("pathological traffic");
    let mut vehicles = Vec::with_capacity(background_count + fast_count);
    for index in 0..background_count {
        vehicles.push(VehicleSpawnInput::active(
            format!("background-{index:06}"),
            profile,
            "R",
            0,
            EdgeProgress::try_new(5.0 + 10.0 * index as f64).expect("background progress"),
            Speed::ZERO,
        ));
    }
    for index in 0..fast_count {
        vehicles.push(VehicleSpawnInput::active(
            format!("fast-{index:04}"),
            profile,
            "fast-route",
            0,
            EdgeProgress::try_new(5.0 + 10.0 * index as f64).expect("fast progress"),
            Speed::try_new(10_000_000.0 - index as f64).expect("pathological speed"),
        ));
    }
    let mut world =
        CoreWorld::with_traffic_data(20, traffic, vehicles).expect("pathological world");
    let space = world
        .parking()
        .space_handle("pathological-space")
        .expect("pathological space");
    let route = world.route_handle("R").expect("pathological route handle");
    let parked = world
        .spawn_parked_vehicle(ParkedVehicleSpawnInput {
            id: "pathological-parked".to_owned(),
            profile,
            route_id: "R".to_owned(),
            route_edge_index: 0,
            space,
        })
        .expect("pathological parked vehicle")
        .vehicle;
    let fastest_first = (0..fast_count)
        .map(|index| {
            world
                .vehicle_handle(&format!("fast-{index:04}"))
                .expect("fast vehicle")
        })
        .collect();
    ParkingPathologicalLeaveScenario {
        world,
        fastest_first,
        leave: LeaveParkingInput {
            vehicle: parked,
            space,
            route,
            route_edge_index: 0,
        },
    }
}

pub fn run_pathological_leave_batch(scenario: &mut ParkingPathologicalLeaveScenario) -> usize {
    for vehicle in scenario.fastest_first.iter().copied() {
        scenario
            .world
            .despawn_vehicle(vehicle)
            .expect("pathological max-speed removal");
    }
    let left = scenario
        .world
        .leave_parking(scenario.leave)
        .expect("pathological leave must remain local and safe");
    scenario.fastest_first.len() + usize::from(left.effect == ParkingCommandEffect::Applied)
}

pub fn occupied_parking_world(vehicle_count: usize, fixed_delta_time_ms: u64) -> CoreWorld {
    let spaces = (0..vehicle_count).map(|index| parking_space(format!("P{index:06}")));
    let (traffic, profile) = traffic_with_spaces(spaces);
    let mut world = CoreWorld::with_traffic_data(fixed_delta_time_ms, traffic, Vec::new())
        .expect("occupied world");
    for index in 0..vehicle_count {
        let space = world
            .parking()
            .space_handle(&format!("P{index:06}"))
            .expect("occupied space");
        world
            .spawn_parked_vehicle(ParkedVehicleSpawnInput {
                id: format!("P{index:06}"),
                profile,
                route_id: "R".to_owned(),
                route_edge_index: 0,
                space,
            })
            .expect("parked spawn");
    }
    world
}
