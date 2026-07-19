use laneflow_core::{
    CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge, LaneGraph,
    Route, Speed, VehicleProfile, VehicleProfileHandle, VehicleProfileRegistry, VehicleSpawnInput,
};

pub const VEHICLE_COUNT: usize = 10_000;
pub const SCALING_VEHICLE_COUNT: usize = 100_000;
pub const STEP_COUNT: usize = 60;
pub const FIXED_DELTA_TIME_MS: u64 = 16;
pub const VEHICLE_LENGTH: f64 = 4.5;
pub const LOCALITY_EDGE_LENGTH: f64 = 10_000.0;

const MILLISECONDS_PER_SECOND: f64 = 1_000.0;
const FREE_FLOW_SPACING: f64 = 250.0;
const DENSE_SPACING: f64 = 6.5;
const PROJECTION_PAIR_SPACING: f64 = 64.0;
const TRANSITION_EDGE_LENGTH: f64 = 5.0;
#[allow(
    dead_code,
    reason = "#140 edge-cap benchmark imports the shared scenario module separately"
)]
const TRANSITION_PRESSURE_EDGE_LENGTH: f64 = 10.0;
#[allow(
    dead_code,
    reason = "#140 edge-cap benchmark imports the shared scenario module separately"
)]
const TRANSITION_PRESSURE_SPEED: f64 = 10.0;

fn edge_length(value: f64) -> EdgeLength {
    EdgeLength::try_new(value).expect("scenario edge length must be valid")
}

fn progress(value: f64) -> EdgeProgress {
    EdgeProgress::try_new(value).expect("scenario edge progress must be valid")
}

fn speed(value: f64) -> Speed {
    Speed::try_new(value).expect("scenario speed must be valid")
}

fn vehicle_external_id(index: usize) -> String {
    format!("vehicle-{index:06}-f0e1d2c3-b4a5-6789-0abc-def123456789")
}

fn edge_external_id(index: usize) -> String {
    format!("edge-{index:05}")
}

fn route_external_id(index: usize) -> String {
    format!("route-{index:05}")
}

fn profile_registry(desired_speed: f64) -> (VehicleProfileRegistry, VehicleProfileHandle) {
    let registry = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        "benchmark-profile",
        IidmProfileSpec {
            length: VEHICLE_LENGTH,
            desired_speed,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 1.4,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 4.0,
        },
    )
    .expect("scenario profile must be valid")])
    .expect("scenario profile registry must be valid");
    let profile = registry
        .profile_handle("benchmark-profile")
        .expect("scenario profile handle must exist");
    (registry, profile)
}

fn linear_platoon_world(
    vehicle_count: usize,
    spacing: f64,
    initial_speed: f64,
    desired_speed: f64,
    stopped_front: bool,
) -> CoreWorld {
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "platoon-edge",
        edge_length(spacing * vehicle_count as f64 + 1_000.0),
        std::iter::empty::<&str>(),
    )])
    .expect("linear scenario graph must be valid");
    let route =
        Route::try_new("platoon-route", ["platoon-edge"]).expect("scenario route must be valid");
    let (profiles, profile) = profile_registry(desired_speed);
    let traffic_data = InitialTrafficData::try_new(lane_graph, [route], profiles)
        .expect("linear scenario traffic data must be valid");
    let vehicles = (0..vehicle_count)
        .map(|index| {
            let front = progress(spacing * index as f64);
            if stopped_front && index + 1 == vehicle_count {
                VehicleSpawnInput::stopped(
                    vehicle_external_id(index),
                    profile,
                    "platoon-route",
                    0,
                    front,
                )
            } else {
                VehicleSpawnInput::active(
                    vehicle_external_id(index),
                    profile,
                    "platoon-route",
                    0,
                    front,
                    speed(initial_speed),
                )
            }
        })
        .collect();

    CoreWorld::with_traffic_data(FIXED_DELTA_TIME_MS, traffic_data, vehicles)
        .expect("linear scenario world must be valid")
}

fn locality_preserving_platoon_world(
    vehicle_count: usize,
    spacing: f64,
    initial_speed: f64,
    desired_speed: f64,
    stopped_front: bool,
    edge_cap: f64,
) -> CoreWorld {
    assert!(edge_cap.is_finite() && edge_cap > 0.0);
    let route_length = spacing * vehicle_count as f64 + 1_000.0;
    let edge_count = (route_length / edge_cap).ceil() as usize;
    let edge_ids: Vec<_> = (0..edge_count)
        .map(|index| format!("locality-edge-{index:05}"))
        .collect();
    let lane_graph = LaneGraph::try_new(edge_ids.iter().enumerate().map(|(index, edge_id)| {
        let length = if index + 1 == edge_count {
            route_length - edge_cap * index as f64
        } else {
            edge_cap
        };
        LaneEdge::new(
            edge_id.clone(),
            edge_length(length),
            edge_ids.get(index + 1).into_iter().cloned(),
        )
    }))
    .expect("locality-preserving scenario graph must be valid");
    let route = Route::try_new("locality-platoon-route", edge_ids)
        .expect("locality-preserving scenario route must be valid");
    let (profiles, profile) = profile_registry(desired_speed);
    let traffic_data = InitialTrafficData::try_new(lane_graph, [route], profiles)
        .expect("locality-preserving scenario traffic data must be valid");
    let vehicles = (0..vehicle_count)
        .map(|index| {
            let route_progress = spacing * index as f64;
            let route_edge_index = (route_progress / edge_cap).floor() as usize;
            let edge_progress = progress(route_progress - edge_cap * route_edge_index as f64);
            if stopped_front && index + 1 == vehicle_count {
                VehicleSpawnInput::stopped(
                    vehicle_external_id(index),
                    profile,
                    "locality-platoon-route",
                    route_edge_index,
                    edge_progress,
                )
            } else {
                VehicleSpawnInput::active(
                    vehicle_external_id(index),
                    profile,
                    "locality-platoon-route",
                    route_edge_index,
                    edge_progress,
                    speed(initial_speed),
                )
            }
        })
        .collect();

    CoreWorld::with_traffic_data(FIXED_DELTA_TIME_MS, traffic_data, vehicles)
        .expect("locality-preserving scenario world must be valid")
}

pub fn free_flow_world(vehicle_count: usize) -> CoreWorld {
    linear_platoon_world(vehicle_count, FREE_FLOW_SPACING, 10.0, 13.9, false)
}

pub fn dense_platoon_world(vehicle_count: usize) -> CoreWorld {
    linear_platoon_world(vehicle_count, DENSE_SPACING, 1.0, 13.9, false)
}

pub fn stop_and_go_world(vehicle_count: usize) -> CoreWorld {
    linear_platoon_world(vehicle_count, DENSE_SPACING, 8.0, 13.9, true)
}

pub fn locality_free_flow_world(vehicle_count: usize) -> CoreWorld {
    free_flow_world_with_edge_cap(vehicle_count, LOCALITY_EDGE_LENGTH)
}

pub fn locality_dense_platoon_world(vehicle_count: usize) -> CoreWorld {
    dense_platoon_world_with_edge_cap(vehicle_count, LOCALITY_EDGE_LENGTH)
}

pub fn locality_stop_and_go_world(vehicle_count: usize) -> CoreWorld {
    stop_and_go_world_with_edge_cap(vehicle_count, LOCALITY_EDGE_LENGTH)
}

pub fn free_flow_world_with_edge_cap(vehicle_count: usize, edge_cap: f64) -> CoreWorld {
    locality_preserving_platoon_world(
        vehicle_count,
        FREE_FLOW_SPACING,
        10.0,
        13.9,
        false,
        edge_cap,
    )
}

pub fn dense_platoon_world_with_edge_cap(vehicle_count: usize, edge_cap: f64) -> CoreWorld {
    locality_preserving_platoon_world(vehicle_count, DENSE_SPACING, 1.0, 13.9, false, edge_cap)
}

pub fn stop_and_go_world_with_edge_cap(vehicle_count: usize, edge_cap: f64) -> CoreWorld {
    locality_preserving_platoon_world(vehicle_count, DENSE_SPACING, 8.0, 13.9, true, edge_cap)
}

pub fn projection_heavy_world(vehicle_count: usize) -> CoreWorld {
    assert_eq!(vehicle_count % 2, 0);
    let pair_count = vehicle_count / 2;
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "projection-edge",
        edge_length(PROJECTION_PAIR_SPACING * pair_count as f64 + 100.0),
        std::iter::empty::<&str>(),
    )])
    .expect("projection graph must be valid");
    let route = Route::try_new("projection-route", ["projection-edge"])
        .expect("projection route must be valid");
    let (profiles, profile) = profile_registry(20.0);
    let traffic_data = InitialTrafficData::try_new(lane_graph, [route], profiles)
        .expect("projection traffic data must be valid");
    let vehicles = (0..pair_count)
        .flat_map(|pair_index| {
            let follower_index = pair_index * 2;
            let leader_index = follower_index + 1;
            let follower_front = PROJECTION_PAIR_SPACING * pair_index as f64;
            [
                VehicleSpawnInput::active(
                    vehicle_external_id(follower_index),
                    profile,
                    "projection-route",
                    0,
                    progress(follower_front),
                    speed(20.0),
                ),
                VehicleSpawnInput::stopped(
                    vehicle_external_id(leader_index),
                    profile,
                    "projection-route",
                    0,
                    progress(follower_front + VEHICLE_LENGTH),
                ),
            ]
        })
        .collect();

    CoreWorld::with_traffic_data(FIXED_DELTA_TIME_MS, traffic_data, vehicles)
        .expect("projection world must be valid")
}

pub fn transition_heavy_world(vehicle_count: usize) -> CoreWorld {
    let edge_ids: Vec<_> = (0..vehicle_count).map(edge_external_id).collect();
    let lane_graph = LaneGraph::try_new(edge_ids.iter().map(|edge_id| {
        LaneEdge::new(
            edge_id.clone(),
            edge_length(TRANSITION_EDGE_LENGTH),
            [edge_id.clone()],
        )
    }))
    .expect("transition graph must be valid");
    let routes: Vec<_> = edge_ids
        .iter()
        .enumerate()
        .map(|(index, edge_id)| {
            Route::try_new(
                route_external_id(index),
                std::iter::repeat_n(edge_id.clone(), STEP_COUNT + 1),
            )
            .expect("transition route must be valid")
        })
        .collect();
    let seconds_per_step = FIXED_DELTA_TIME_MS as f64 / MILLISECONDS_PER_SECOND;
    let transition_speed = speed(TRANSITION_EDGE_LENGTH / seconds_per_step);
    let (profiles, profile) = profile_registry(transition_speed.value());
    let traffic_data = InitialTrafficData::try_new(lane_graph, routes, profiles)
        .expect("transition traffic data must be valid");
    let vehicles = (0..vehicle_count)
        .map(|index| {
            VehicleSpawnInput::active(
                vehicle_external_id(index),
                profile,
                route_external_id(index),
                0,
                EdgeProgress::ZERO,
                transition_speed,
            )
        })
        .collect();

    CoreWorld::with_traffic_data(FIXED_DELTA_TIME_MS, traffic_data, vehicles)
        .expect("transition world must be valid")
}

#[allow(
    dead_code,
    reason = "#140 edge-cap benchmark imports the shared scenario module separately"
)]
pub fn transition_pressure_world(vehicle_count: usize, crossing_percent: usize) -> CoreWorld {
    assert!(crossing_percent <= 100);
    let edge_ids: Vec<_> = (0..vehicle_count).map(edge_external_id).collect();
    let lane_graph = LaneGraph::try_new(edge_ids.iter().map(|edge_id| {
        LaneEdge::new(
            edge_id.clone(),
            edge_length(TRANSITION_PRESSURE_EDGE_LENGTH),
            [edge_id.clone()],
        )
    }))
    .expect("transition pressure graph must be valid");
    let routes: Vec<_> = edge_ids
        .iter()
        .enumerate()
        .map(|(index, edge_id)| {
            Route::try_new(route_external_id(index), [edge_id.clone(), edge_id.clone()])
                .expect("transition pressure route must be valid")
        })
        .collect();
    let (profiles, profile) = profile_registry(TRANSITION_PRESSURE_SPEED);
    let traffic_data = InitialTrafficData::try_new(lane_graph, routes, profiles)
        .expect("transition pressure traffic data must be valid");
    let crossing_count = vehicle_count * crossing_percent / 100;
    let vehicles = (0..vehicle_count)
        .map(|index| {
            VehicleSpawnInput::active(
                vehicle_external_id(index),
                profile,
                route_external_id(index),
                0,
                if index < crossing_count {
                    progress(TRANSITION_PRESSURE_EDGE_LENGTH - 0.1)
                } else {
                    EdgeProgress::ZERO
                },
                speed(TRANSITION_PRESSURE_SPEED),
            )
        })
        .collect();

    CoreWorld::with_traffic_data(FIXED_DELTA_TIME_MS, traffic_data, vehicles)
        .expect("transition pressure world must be valid")
}

pub const fn projection_event_count(vehicle_count: usize) -> usize {
    vehicle_count / 2
}

pub const fn transition_event_count(vehicle_count: usize) -> usize {
    vehicle_count * STEP_COUNT
}

#[allow(
    dead_code,
    reason = "#140 edge-cap benchmark imports the shared scenario module separately"
)]
pub const fn transition_pressure_event_count(
    vehicle_count: usize,
    crossing_percent: usize,
) -> usize {
    vehicle_count * crossing_percent / 100
}
