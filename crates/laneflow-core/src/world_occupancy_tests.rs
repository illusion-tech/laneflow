//! Occupancy index、leader detection 与 overlap validation 白盒测试。

use super::*;
use crate::{
    CoreError, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge, Speed,
    TickInput, VehicleProfile, VehicleProfileHandle, VehicleProfileRegistry,
};

fn traffic_data<I>(lane_graph: LaneGraph, routes: I) -> (InitialTrafficData, VehicleProfileHandle)
where
    I: IntoIterator<Item = Route>,
{
    let registry = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        "test-profile",
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
    .expect("valid profile")])
    .expect("valid profile registry");
    let profile = registry
        .profile_handle("test-profile")
        .expect("profile handle exists");
    let traffic_data =
        InitialTrafficData::try_new(lane_graph, routes, registry).expect("valid traffic data");
    (traffic_data, profile)
}
fn edge_length(value: f64) -> EdgeLength {
    EdgeLength::try_new(value).expect("valid edge length")
}

fn progress(value: f64) -> EdgeProgress {
    EdgeProgress::try_new(value).expect("valid edge progress")
}

fn speed(value: f64) -> Speed {
    Speed::try_new(value).expect("valid speed")
}

fn leader_of(world: &CoreWorld, vehicle_id: &str) -> Option<(String, f64)> {
    let vehicle = world
        .vehicle_handle(vehicle_id)
        .expect("vehicle handle must exist");
    world.occupancy_scratch.leader(vehicle).map(|observation| {
        (
            world
                .vehicle_external_id(observation.leader)
                .expect("leader external ID must exist")
                .to_owned(),
            observation.bumper_gap,
        )
    })
}

#[test]
fn occupancy_detects_same_edge_stopped_leader_and_excludes_completed_vehicle() {
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "A",
        edge_length(20.0),
        crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
        std::iter::empty::<&str>(),
    )])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A"]).expect("valid route");
    let (traffic_data, profile) = traffic_data(lane_graph, [route]);
    let mut world = CoreWorld::with_traffic_data(
        16,
        traffic_data,
        vec![
            VehicleSpawnInput::active("follower", profile, "R", 0, progress(0.0), Speed::ZERO),
            VehicleSpawnInput::stopped("leader", profile, "R", 0, progress(6.0)),
            VehicleSpawnInput::completed("completed", profile, "R", 0, progress(20.0)),
        ],
    )
    .expect("valid world");

    world
        .rebuild_occupancy_and_leaders()
        .expect("occupancy build succeeds");

    assert_eq!(world.occupancy_scratch.occupant_count(), 2);
    assert_eq!(
        leader_of(&world, "follower"),
        Some(("leader".to_owned(), 1.5))
    );
    assert_eq!(leader_of(&world, "leader"), None);
    assert_eq!(leader_of(&world, "completed"), None);
}

#[test]
fn leader_query_follows_selected_branch_and_observes_shared_edge() {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new(
            "A",
            edge_length(10.0),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["B"],
        ),
        LaneEdge::new(
            "C",
            edge_length(10.0),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["B"],
        ),
        LaneEdge::new(
            "B",
            edge_length(10.0),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("valid lane graph");
    let route_a = Route::try_new("route-a", ["A", "B"]).expect("valid route");
    let route_c = Route::try_new("route-c", ["C", "B"]).expect("valid route");
    let (traffic_data, profile) = traffic_data(lane_graph, [route_a, route_c]);
    let mut world = CoreWorld::with_traffic_data(
        16,
        traffic_data,
        vec![
            VehicleSpawnInput::active(
                "follower",
                profile,
                "route-a",
                0,
                progress(8.0),
                Speed::ZERO,
            ),
            VehicleSpawnInput::stopped("other-branch", profile, "route-c", 0, progress(0.0)),
            VehicleSpawnInput::stopped("shared-edge", profile, "route-c", 1, progress(3.0)),
        ],
    )
    .expect("valid world");

    world
        .rebuild_occupancy_and_leaders()
        .expect("occupancy build succeeds");

    assert_eq!(
        leader_of(&world, "follower"),
        Some(("shared-edge".to_owned(), 0.5))
    );
}

#[test]
fn repeated_edge_query_excludes_self_and_uses_next_occurrence() {
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "loop",
        edge_length(10.0),
        crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
        ["loop"],
    )])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["loop", "loop"]).expect("valid route");
    let (traffic_data, profile) = traffic_data(lane_graph, [route]);
    let mut world = CoreWorld::with_traffic_data(
        16,
        traffic_data,
        vec![
            VehicleSpawnInput::active("follower", profile, "R", 0, progress(8.0), Speed::ZERO),
            VehicleSpawnInput::stopped("leader", profile, "R", 0, progress(3.0)),
        ],
    )
    .expect("valid world");

    world
        .rebuild_occupancy_and_leaders()
        .expect("occupancy build succeeds");

    assert_eq!(
        leader_of(&world, "follower"),
        Some(("leader".to_owned(), 0.5))
    );
}

#[test]
fn initial_same_progress_overlap_is_rejected() {
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "A",
        edge_length(20.0),
        crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
        std::iter::empty::<&str>(),
    )])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A"]).expect("valid route");
    let (traffic_data, profile) = traffic_data(lane_graph, [route]);

    let error = CoreWorld::with_traffic_data(
        16,
        traffic_data,
        vec![
            VehicleSpawnInput::active("V1", profile, "R", 0, progress(5.0), Speed::ZERO),
            VehicleSpawnInput::active("V2", profile, "R", 0, progress(5.0), Speed::ZERO),
        ],
    )
    .expect_err("same progress must overlap");

    std::assert_matches!(
        error,
        CoreError::VehiclePhysicalOverlap {
            follower_id,
            leader_id,
            bumper_gap
        } if follower_id == "V1" && leader_id == "V2" && bumper_gap == -4.5
    );
}

#[test]
fn initial_cross_boundary_overlap_is_rejected() {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new(
            "A",
            edge_length(10.0),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["B"],
        ),
        LaneEdge::new(
            "B",
            edge_length(10.0),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A", "B"]).expect("valid route");
    let (traffic_data, profile) = traffic_data(lane_graph, [route]);

    let error = CoreWorld::with_traffic_data(
        16,
        traffic_data,
        vec![
            VehicleSpawnInput::active("follower", profile, "R", 0, progress(8.0), Speed::ZERO),
            VehicleSpawnInput::stopped("leader", profile, "R", 1, progress(1.0)),
        ],
    )
    .expect_err("cross-boundary body overlap must fail");

    std::assert_matches!(
        error,
        CoreError::VehiclePhysicalOverlap {
            follower_id,
            leader_id,
            bumper_gap
        } if follower_id == "follower" && leader_id == "leader" && bumper_gap == -1.5
    );
}

#[test]
fn min_gap_violation_without_physical_overlap_is_valid() {
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "A",
        edge_length(20.0),
        crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
        std::iter::empty::<&str>(),
    )])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A"]).expect("valid route");
    let (traffic_data, profile) = traffic_data(lane_graph, [route]);

    let world = CoreWorld::with_traffic_data(
        16,
        traffic_data,
        vec![
            VehicleSpawnInput::active("follower", profile, "R", 0, progress(0.0), Speed::ZERO),
            VehicleSpawnInput::stopped("leader", profile, "R", 0, progress(5.5)),
        ],
    )
    .expect("positive bumper gap below min_gap remains valid");

    assert_eq!(world.vehicles().count(), 2);
}

#[test]
fn spawn_cross_boundary_overlap_failure_is_atomic() {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new(
            "A",
            edge_length(10.0),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["B"],
        ),
        LaneEdge::new(
            "B",
            edge_length(10.0),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A", "B"]).expect("valid route");
    let (traffic_data, profile) = traffic_data(lane_graph, [route]);
    let mut world = CoreWorld::with_traffic_data(
        16,
        traffic_data,
        vec![VehicleSpawnInput::active(
            "existing",
            profile,
            "R",
            0,
            progress(8.0),
            Speed::ZERO,
        )],
    )
    .expect("valid world");
    let before = world.clone();

    let error = world
        .spawn_vehicle(VehicleSpawnInput::stopped(
            "candidate",
            profile,
            "R",
            1,
            progress(1.0),
        ))
        .expect_err("overlapping spawn must fail");

    std::assert_matches!(
        error,
        CoreError::VehiclePhysicalOverlap {
            follower_id,
            leader_id,
            bumper_gap
        } if follower_id == "existing" && leader_id == "candidate" && bumper_gap == -1.5
    );
    assert_eq!(world, before);
    assert_eq!(world.vehicle_handle("candidate"), None);
}

#[test]
fn occupancy_and_leader_results_ignore_initial_input_order() {
    fn build_world(reverse_input: bool) -> CoreWorld {
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            edge_length(20.0),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        )])
        .expect("valid lane graph");
        let route = Route::try_new("R", ["A"]).expect("valid route");
        let (traffic_data, profile) = traffic_data(lane_graph, [route]);
        let follower =
            VehicleSpawnInput::active("follower", profile, "R", 0, progress(0.0), Speed::ZERO);
        let leader = VehicleSpawnInput::stopped("leader", profile, "R", 0, progress(6.0));
        let vehicles = if reverse_input {
            vec![leader, follower]
        } else {
            vec![follower, leader]
        };

        CoreWorld::with_traffic_data(16, traffic_data, vehicles).expect("valid world")
    }

    let mut first = build_world(false);
    let mut second = build_world(true);
    first
        .rebuild_occupancy_and_leaders()
        .expect("first occupancy build succeeds");
    second
        .rebuild_occupancy_and_leaders()
        .expect("second occupancy build succeeds");

    let first_order: Vec<_> = first
        .vehicles()
        .map(|vehicle| {
            first
                .vehicle_external_id(vehicle.handle)
                .expect("ID exists")
        })
        .collect();
    let second_order: Vec<_> = second
        .vehicles()
        .map(|vehicle| {
            second
                .vehicle_external_id(vehicle.handle)
                .expect("ID exists")
        })
        .collect();
    assert_eq!(first_order, second_order);
    assert_eq!(
        leader_of(&first, "follower"),
        leader_of(&second, "follower")
    );
}

#[test]
fn derived_scratch_history_is_ignored_by_world_equality() {
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "A",
        edge_length(20.0),
        crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
        std::iter::empty::<&str>(),
    )])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A"]).expect("valid route");
    let (traffic_data, profile) = traffic_data(lane_graph, [route]);
    let mut world = CoreWorld::with_traffic_data(
        16,
        traffic_data,
        vec![
            VehicleSpawnInput::active("follower", profile, "R", 0, progress(0.0), Speed::ZERO),
            VehicleSpawnInput::stopped("leader", profile, "R", 0, progress(6.0)),
        ],
    )
    .expect("valid world");
    let before = world.clone();

    world.candidate_state_scratch.begin(&world.vehicles);
    world.occupancy_scratch.begin(0, 0);
    world.longitudinal_scratch.begin(0);

    assert_eq!(world.candidate_state_scratch.states.len(), 2);
    assert_eq!(world.occupancy_scratch.occupant_count(), 0);
    assert_eq!(world, before);
}

#[test]
fn epsilon_overlap_is_normalized_to_zero_bumper_gap() {
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "A",
        edge_length(20.0),
        crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
        std::iter::empty::<&str>(),
    )])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A"]).expect("valid route");
    let (traffic_data, profile) = traffic_data(lane_graph, [route]);
    let mut world = CoreWorld::with_traffic_data(
        16,
        traffic_data,
        vec![
            VehicleSpawnInput::active("follower", profile, "R", 0, progress(0.0), Speed::ZERO),
            VehicleSpawnInput::stopped(
                "leader",
                profile,
                "R",
                0,
                progress(4.5 - PHYSICAL_GAP_TOLERANCE_METERS / 2.0),
            ),
        ],
    )
    .expect("epsilon overlap is tolerated");

    world
        .rebuild_occupancy_and_leaders()
        .expect("occupancy build succeeds");

    assert_eq!(
        leader_of(&world, "follower"),
        Some(("leader".to_owned(), 0.0))
    );
}

#[test]
fn scaled_braking_distance_avoids_false_square_overflow() {
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "A",
        edge_length(f64::MAX),
        crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
        std::iter::empty::<&str>(),
    )])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A"]).expect("valid route");
    let profiles = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        "large-values",
        IidmProfileSpec {
            length: 4.5,
            desired_speed: 1.0e200,
            min_gap: 0.0,
            time_headway: 1.0,
            max_acceleration: 1.0,
            comfortable_deceleration: 1.0e200,
            emergency_deceleration: 1.0e200,
        },
    )
    .expect("valid large-value profile")])
    .expect("valid profile registry");
    let profile = profiles
        .profile_handle("large-values")
        .expect("profile handle exists");
    let traffic_data =
        InitialTrafficData::try_new(lane_graph, [route], profiles).expect("valid traffic data");
    let world = CoreWorld::with_traffic_data(
        16,
        traffic_data,
        vec![
            VehicleSpawnInput::active("follower", profile, "R", 0, progress(0.0), speed(1.0e200)),
            VehicleSpawnInput::stopped("leader", profile, "R", 0, progress(1.0e201)),
        ],
    )
    .expect("valid world");
    let follower = world
        .vehicle_handle("follower")
        .and_then(|handle| world.vehicle(handle))
        .expect("follower exists");

    let horizon = world
        .leader_horizon(follower)
        .expect("mathematically finite horizon must remain finite");

    assert!(horizon.is_finite());
    assert!(horizon > 0.0);
    assert!(CoreWorld::braking_distance(f64::MAX, f64::MAX * 0.75).is_finite());
    assert!(CoreWorld::half_product(f64::from_bits(1), f64::MAX) > 0.0);
}

#[test]
fn non_finite_leader_horizon_keeps_authority_state_unchanged() {
    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "A",
        edge_length(f64::MAX),
        crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
        std::iter::empty::<&str>(),
    )])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A"]).expect("valid route");
    let (traffic_data, profile) = traffic_data(lane_graph, [route]);
    let mut world = CoreWorld::with_traffic_data(
        2_000,
        traffic_data,
        vec![
            VehicleSpawnInput::active("follower", profile, "R", 0, progress(0.0), speed(f64::MAX)),
            VehicleSpawnInput::stopped("leader", profile, "R", 0, progress(10.0)),
        ],
    )
    .expect("valid world");
    let follower = world
        .vehicle_handle("follower")
        .expect("follower handle exists");
    let before = world.clone();

    let error = world
        .step(TickInput::new(2_000))
        .expect_err("non-finite leader horizon must fail");

    std::assert_matches!(
        error,
        CoreError::NonFiniteLeaderComputation {
            vehicle,
            stage: "travel_upper",
            value
        } if vehicle == follower && value.is_infinite()
    );
    assert_eq!(world, before);
    assert_eq!(world.tick_index(), 0);
    assert_eq!(world.time_ms(), 0);
}
