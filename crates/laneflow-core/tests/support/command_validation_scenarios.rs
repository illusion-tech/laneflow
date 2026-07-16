use laneflow_core::{
    CoreError, CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge,
    LaneGraph, Route, RouteHandle, Speed, VehicleHandle, VehicleProfile, VehicleProfileHandle,
    VehicleProfileRegistry, VehicleSpawnInput,
};

pub const FIXED_COMMAND_COUNT: usize = 100;
pub const COMMAND_VEHICLE_COUNT: usize = 10_000;
pub const COMMAND_SCALING_VEHICLE_COUNT: usize = 100_000;
pub const DEFAULT_ROUTE_LENGTH: usize = 64;

const COMMAND_ROUTE_ID: &str = "command-route";
const BACKGROUND_ROUTE_ID: &str = "background-route";
const UNUSED_ROUTE_ID: &str = "unused-route";
const LOCAL_VEHICLE_ID: &str = "zz-local";
const LOCAL_PROGRESS: f64 = 5.0;
const SAFE_PROGRESS: f64 = 50.0;
const BACKGROUND_PROGRESS_START: f64 = 1_000.0;
const BACKGROUND_SPACING: f64 = 10.0;

#[derive(Clone)]
pub struct CommandScenario {
    pub world: CoreWorld,
    pub command_route: RouteHandle,
    pub unused_route: RouteHandle,
    pub local_vehicle: VehicleHandle,
    safe_inputs: Vec<VehicleSpawnInput>,
    overlap_inputs: Vec<VehicleSpawnInput>,
    mixed_inputs: Vec<VehicleSpawnInput>,
}

pub fn command_scenario(background_vehicle_count: usize, route_length: usize) -> CommandScenario {
    assert!(
        route_length > 0,
        "command benchmark route must not be empty"
    );

    let first_edge_length =
        BACKGROUND_PROGRESS_START + BACKGROUND_SPACING * background_vehicle_count as f64 + 1_000.0;
    let edge_ids = (0..route_length)
        .map(|index| format!("edge-{index:03}"))
        .collect::<Vec<_>>();
    let edges = edge_ids
        .iter()
        .enumerate()
        .map(|(index, id)| {
            let next = edge_ids.get(index + 1).into_iter().cloned();
            let length = if index == 0 { first_edge_length } else { 10.0 };
            LaneEdge::new(
                id,
                EdgeLength::try_new(length).expect("command benchmark edge length must be valid"),
                next,
            )
        })
        .collect::<Vec<_>>();
    let lane_graph = LaneGraph::try_new(edges).expect("command benchmark graph must be valid");
    let routes = [COMMAND_ROUTE_ID, BACKGROUND_ROUTE_ID, UNUSED_ROUTE_ID].map(|id| {
        Route::try_new(id, edge_ids.iter().cloned()).expect("command benchmark route must be valid")
    });

    let registry = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        "command-profile",
        IidmProfileSpec {
            length: 4.5,
            desired_speed: 13.9,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 1.4,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 4.0,
        },
    )
    .expect("command benchmark profile must be valid")])
    .expect("command benchmark profile registry must be valid");
    let profile = registry
        .profile_handle("command-profile")
        .expect("command benchmark profile handle must exist");
    let traffic_data = InitialTrafficData::try_new(lane_graph, routes, registry)
        .expect("command benchmark traffic data must be valid");

    let mut vehicles = (0..background_vehicle_count)
        .map(|index| {
            VehicleSpawnInput::active(
                format!("background-{index:06}"),
                profile,
                BACKGROUND_ROUTE_ID,
                0,
                EdgeProgress::try_new(
                    BACKGROUND_PROGRESS_START + BACKGROUND_SPACING * index as f64,
                )
                .expect("background progress must be valid"),
                Speed::ZERO,
            )
        })
        .collect::<Vec<_>>();
    vehicles.push(VehicleSpawnInput::active(
        LOCAL_VEHICLE_ID,
        profile,
        COMMAND_ROUTE_ID,
        0,
        EdgeProgress::try_new(LOCAL_PROGRESS).expect("local progress must be valid"),
        Speed::ZERO,
    ));

    let world = CoreWorld::with_traffic_data(16, traffic_data, vehicles)
        .expect("command benchmark world must be valid");
    let command_route = world
        .route_handle(COMMAND_ROUTE_ID)
        .expect("command route handle must exist");
    let unused_route = world
        .route_handle(UNUSED_ROUTE_ID)
        .expect("unused route handle must exist");
    let local_vehicle = world
        .vehicle_handle(LOCAL_VEHICLE_ID)
        .expect("local vehicle handle must exist");
    let safe_inputs = command_inputs("safe", FIXED_COMMAND_COUNT / 2, profile, SAFE_PROGRESS);
    let overlap_inputs = command_inputs("overlap", FIXED_COMMAND_COUNT, profile, LOCAL_PROGRESS);
    let mixed_inputs = command_inputs("mixed", FIXED_COMMAND_COUNT / 4, profile, SAFE_PROGRESS);

    CommandScenario {
        world,
        command_route,
        unused_route,
        local_vehicle,
        safe_inputs,
        overlap_inputs,
        mixed_inputs,
    }
}

fn command_inputs(
    prefix: &str,
    count: usize,
    profile: VehicleProfileHandle,
    progress: f64,
) -> Vec<VehicleSpawnInput> {
    (0..count)
        .map(|index| {
            VehicleSpawnInput::active(
                format!("{prefix}-{index:04}"),
                profile,
                COMMAND_ROUTE_ID,
                0,
                EdgeProgress::try_new(progress).expect("command progress must be valid"),
                Speed::ZERO,
            )
        })
        .collect()
}

pub fn run_safe_spawn_despawn_batch(scenario: &mut CommandScenario, command_count: usize) -> usize {
    assert_eq!(command_count % 2, 0);
    assert_eq!(scenario.safe_inputs.len(), command_count / 2);
    let mut checksum = 0;
    let inputs = std::mem::take(&mut scenario.safe_inputs);
    for input in inputs {
        let handle = scenario
            .world
            .spawn_vehicle(input)
            .expect("safe command spawn must succeed");
        let record = scenario
            .world
            .despawn_vehicle(handle)
            .expect("safe command despawn must succeed");
        checksum += record.external_id.len();
    }
    checksum
}

pub fn run_overlap_failure_batch(scenario: &mut CommandScenario, command_count: usize) -> usize {
    assert_eq!(scenario.overlap_inputs.len(), command_count);
    let mut checksum = 0;
    let inputs = std::mem::take(&mut scenario.overlap_inputs);
    for input in inputs {
        let error = scenario
            .world
            .spawn_vehicle(input)
            .expect_err("overlap command must fail");
        match error {
            CoreError::VehiclePhysicalOverlap {
                follower_id,
                leader_id,
                ..
            } => checksum += follower_id.len() + leader_id.len(),
            error => panic!("unexpected overlap error: {error:?}"),
        }
    }
    checksum
}

pub fn run_in_use_route_failure_batch(
    scenario: &mut CommandScenario,
    command_count: usize,
) -> usize {
    let mut checksum = 0;
    for _ in 0..command_count {
        let error = scenario
            .world
            .remove_route(scenario.command_route)
            .expect_err("in-use route removal must fail");
        match error {
            CoreError::RouteInUse { route, vehicle } => {
                assert_eq!(route, scenario.command_route);
                assert_eq!(vehicle, scenario.local_vehicle);
                checksum += 1;
            }
            error => panic!("unexpected route removal error: {error:?}"),
        }
    }
    checksum
}

pub fn run_mixed_churn_batch(scenario: &mut CommandScenario, command_count: usize) -> usize {
    assert_eq!(command_count % 4, 0);
    assert_eq!(scenario.mixed_inputs.len(), command_count / 4);
    assert!(scenario.overlap_inputs.len() >= command_count / 4);
    let mut checksum = 0;
    let inputs = std::mem::take(&mut scenario.mixed_inputs);
    let mut overlap_inputs = std::mem::take(&mut scenario.overlap_inputs).into_iter();
    for input in inputs {
        let handle = scenario
            .world
            .spawn_vehicle(input)
            .expect("mixed command spawn must succeed");
        let overlap_error = scenario
            .world
            .spawn_vehicle(
                overlap_inputs
                    .next()
                    .expect("mixed overlap input must exist"),
            )
            .expect_err("mixed overlap command must fail");
        match overlap_error {
            CoreError::VehiclePhysicalOverlap {
                follower_id,
                leader_id,
                ..
            } => checksum += follower_id.len() + leader_id.len(),
            error => panic!("unexpected mixed overlap error: {error:?}"),
        }
        checksum += scenario
            .world
            .despawn_vehicle(handle)
            .expect("mixed command despawn must succeed")
            .external_id
            .len();
        checksum += run_in_use_route_failure_batch(scenario, 1);
    }
    checksum
}

pub fn remove_unused_route(scenario: &mut CommandScenario) -> usize {
    scenario
        .world
        .remove_route(scenario.unused_route)
        .expect("unused route removal must succeed")
        .external_id
        .len()
}
