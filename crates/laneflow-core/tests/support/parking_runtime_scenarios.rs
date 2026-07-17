use laneflow_core::{
    CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge, LaneGraph,
    ParkedVehicleSpawnInput, ParkingCommandEffect, ParkingRegistry, ParkingSpace,
    ParkingSpaceGeometry, ParkingSpaceHandle, Route, SignalRegistry, Speed, VehicleHandle,
    VehicleProfile, VehicleProfileHandle, VehicleProfileRegistry, VehicleSpawnInput,
};

pub const FIXED_PARKING_COMMAND_COUNT: usize = 100;

pub struct ParkingCommandScenario {
    pub world: CoreWorld,
    pub pairs: Vec<(VehicleHandle, ParkingSpaceHandle)>,
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
