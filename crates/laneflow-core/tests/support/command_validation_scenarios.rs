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
const COMMAND_EDGE_LENGTH: f64 = 100.0;
const BACKGROUND_EDGE_LENGTH: f64 = 10_000.0;
const BACKGROUND_PROGRESS_START: f64 = 5.0;
const BACKGROUND_SPACING: f64 = 10.0;
const BACKGROUND_VEHICLES_PER_EDGE: usize = 999;

#[derive(Clone)]
pub struct CommandScenario {
    pub world: CoreWorld,
    pub command_route: RouteHandle,
    pub unused_route: RouteHandle,
    pub local_vehicle: VehicleHandle,
    warm_input: VehicleSpawnInput,
    warm_overlap_input: VehicleSpawnInput,
    safe_inputs: Vec<VehicleSpawnInput>,
    overlap_inputs: Vec<VehicleSpawnInput>,
    mixed_inputs: Vec<VehicleSpawnInput>,
}

pub struct CompactionScenario {
    pub world: CoreWorld,
    pub trigger: VehicleHandle,
}

pub fn compaction_scenario(background_vehicle_count: usize) -> CompactionScenario {
    let mut scenario = command_scenario(background_vehicle_count, DEFAULT_ROUTE_LENGTH);
    let handles = scenario
        .world
        .vehicles()
        .map(|vehicle| vehicle.handle)
        .collect::<Vec<_>>();
    let trigger_count = handles.len().div_ceil(2);
    for handle in handles.iter().take(trigger_count - 1).copied() {
        scenario
            .world
            .despawn_vehicle(handle)
            .expect("pre-threshold vehicle must despawn");
    }
    CompactionScenario {
        world: scenario.world,
        trigger: handles[trigger_count - 1],
    }
}

pub fn command_scenario(background_vehicle_count: usize, route_length: usize) -> CommandScenario {
    build_command_scenario(
        background_vehicle_count,
        route_length,
        FIXED_COMMAND_COUNT,
        false,
        3,
    )
}

pub fn command_count_scenario(
    background_vehicle_count: usize,
    route_length: usize,
    command_count: usize,
) -> CommandScenario {
    build_command_scenario(
        background_vehicle_count,
        route_length,
        command_count,
        false,
        3,
    )
}

pub fn repeated_command_scenario(
    background_vehicle_count: usize,
    route_length: usize,
) -> CommandScenario {
    build_command_scenario(
        background_vehicle_count,
        route_length,
        FIXED_COMMAND_COUNT,
        true,
        3,
    )
}

pub fn matched_command_scenario(
    background_vehicle_count: usize,
    route_length: usize,
    command_count: usize,
    total_route_count: usize,
) -> CommandScenario {
    build_command_scenario(
        background_vehicle_count,
        route_length,
        command_count,
        false,
        total_route_count,
    )
}

fn build_command_scenario(
    background_vehicle_count: usize,
    route_length: usize,
    command_count: usize,
    repeated_edge: bool,
    total_route_count: usize,
) -> CommandScenario {
    assert!(
        route_length > 0,
        "command benchmark route must not be empty"
    );
    assert!(
        total_route_count >= 3,
        "three canonical routes are required"
    );

    let (command_edge_ids, mut edges) = if repeated_edge {
        let edge_id = "edge-000".to_owned();
        (
            std::iter::repeat_n(edge_id.clone(), route_length).collect(),
            vec![LaneEdge::new(
                edge_id.clone(),
                EdgeLength::try_from(COMMAND_EDGE_LENGTH)
                    .expect("command benchmark edge length must be valid"),
                [edge_id],
            )],
        )
    } else {
        let edge_ids = (0..route_length)
            .map(|index| format!("edge-{index:03}"))
            .collect::<Vec<_>>();
        let edges = edge_ids
            .iter()
            .enumerate()
            .map(|(index, id)| {
                let next = edge_ids.get(index + 1).into_iter().cloned();
                LaneEdge::new(
                    id,
                    EdgeLength::try_from(COMMAND_EDGE_LENGTH)
                        .expect("command benchmark edge length must be valid"),
                    next,
                )
            })
            .collect::<Vec<_>>();
        (edge_ids, edges)
    };
    let background_edge_count = background_vehicle_count
        .div_ceil(BACKGROUND_VEHICLES_PER_EDGE)
        .max(1);
    let background_edge_ids = (0..background_edge_count)
        .map(|index| format!("background-edge-{index:03}"))
        .collect::<Vec<_>>();
    edges.extend(background_edge_ids.iter().enumerate().map(|(index, id)| {
        LaneEdge::new(
            id,
            EdgeLength::try_from(BACKGROUND_EDGE_LENGTH)
                .expect("background edge length must be valid"),
            background_edge_ids.get(index + 1).into_iter().cloned(),
        )
    }));
    let lane_graph = LaneGraph::try_new(edges).expect("command benchmark graph must be valid");
    let mut route_ids = vec![
        COMMAND_ROUTE_ID.to_owned(),
        BACKGROUND_ROUTE_ID.to_owned(),
        UNUSED_ROUTE_ID.to_owned(),
    ];
    route_ids.extend((3..total_route_count).map(|index| format!("extra-route-{index:03}")));
    let routes = route_ids.into_iter().map(|id| {
        let route_edges = if id == BACKGROUND_ROUTE_ID {
            background_edge_ids.iter().cloned()
        } else {
            command_edge_ids.iter().cloned()
        };
        Route::try_new(id, route_edges).expect("command benchmark route must be valid")
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
                index / BACKGROUND_VEHICLES_PER_EDGE,
                EdgeProgress::try_new(
                    BACKGROUND_PROGRESS_START
                        + BACKGROUND_SPACING * (index % BACKGROUND_VEHICLES_PER_EDGE) as f64,
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
    let warm_input = VehicleSpawnInput::active(
        "warm-capacity",
        profile,
        COMMAND_ROUTE_ID,
        0,
        EdgeProgress::try_new(SAFE_PROGRESS).expect("warm progress must be valid"),
        Speed::ZERO,
    );
    let warm_overlap_input = VehicleSpawnInput::active(
        "warm-overlap",
        profile,
        COMMAND_ROUTE_ID,
        0,
        EdgeProgress::try_new(LOCAL_PROGRESS).expect("warm overlap progress must be valid"),
        Speed::ZERO,
    );
    let safe_inputs = command_inputs("safe", command_count / 2, profile, SAFE_PROGRESS);
    let overlap_inputs = command_inputs("overlap", command_count, profile, LOCAL_PROGRESS);
    let mixed_inputs = command_inputs("mixed", command_count / 4, profile, SAFE_PROGRESS);

    CommandScenario {
        world,
        command_route,
        unused_route,
        local_vehicle,
        warm_input,
        warm_overlap_input,
        safe_inputs,
        overlap_inputs,
        mixed_inputs,
    }
}

pub fn warm_command_scenario(scenario: &mut CommandScenario) {
    let handle = scenario
        .world
        .spawn_vehicle(scenario.warm_input.clone())
        .expect("capacity warm spawn must succeed");
    scenario
        .world
        .despawn_vehicle(handle)
        .expect("capacity warm despawn must succeed");
    scenario
        .world
        .spawn_vehicle(scenario.warm_overlap_input.clone())
        .expect_err("candidate scratch warm overlap must fail");
}

#[allow(
    dead_code,
    reason = "used only by the dedicated allocation test binary"
)]
pub fn take_safe_input(scenario: &mut CommandScenario) -> VehicleSpawnInput {
    scenario
        .safe_inputs
        .pop()
        .expect("safe command input must remain")
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
