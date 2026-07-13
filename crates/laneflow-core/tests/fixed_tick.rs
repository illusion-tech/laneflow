mod common;

use common::world_with_test_profile;
use laneflow_core::{
    Acceleration, CoreError, CoreWorld, EdgeLength, EdgeProgress, LaneEdge, LaneGraph, Route,
    Speed, TickInput, VehicleProfileHandle, VehicleSpawnInput, VehicleStatus,
};

const EPSILON: f64 = 1.0e-9;

fn edge_length(value: f64) -> EdgeLength {
    EdgeLength::try_new(value).expect("valid edge length")
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= EPSILON,
        "actual={actual}, expected={expected}"
    );
}

fn single_edge_world(
    fixed_delta_time_ms: u64,
    vehicles: impl FnOnce(VehicleProfileHandle) -> Vec<VehicleSpawnInput>,
) -> CoreWorld {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new("A", edge_length(10.0), ["B"]),
        LaneEdge::new("B", edge_length(10.0), std::iter::empty::<&str>()),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R1", ["A", "B"]).expect("valid route");

    world_with_test_profile(fixed_delta_time_ms, lane_graph, [route], vehicles)
        .expect("valid world")
}

#[test]
fn fixed_tick_advances_post_step_time() {
    let mut world = CoreWorld::new(1_000).expect("valid world");

    let result = world.step(TickInput::new(1_000)).expect("step succeeds");

    assert_eq!(world.fixed_delta_time_ms(), 1_000);
    assert_eq!(world.tick_index(), 1);
    assert_eq!(world.time_ms(), 1_000);
    assert_eq!(result.tick_index, 1);
    assert_eq!(result.time_ms, 1_000);
    assert!(result.events.is_empty());
}

#[test]
fn delta_mismatch_returns_error_and_keeps_world_unchanged() {
    let mut world = single_edge_world(1_000, |profile| {
        vec![VehicleSpawnInput::active(
            "V1",
            profile,
            "R1",
            0,
            EdgeProgress::try_new(3.0).expect("valid progress"),
            Speed::try_new(2.0).expect("valid speed"),
        )]
    });
    let before = world.clone();

    let error = world
        .step(TickInput::new(500))
        .expect_err("delta mismatch must fail");

    std::assert_matches!(
        error,
        CoreError::TickDeltaMismatch {
            expected_delta_time_ms: 1_000,
            actual_delta_time_ms: 500
        }
    );
    assert_eq!(world, before);
}

#[test]
fn active_zero_speed_accelerates_under_free_road_iidm() {
    let mut world = single_edge_world(1_000, |profile| {
        vec![VehicleSpawnInput::active(
            "V1",
            profile,
            "R1",
            0,
            EdgeProgress::try_new(7.5).expect("valid progress"),
            Speed::ZERO,
        )]
    });

    world.step(TickInput::new(1_000)).expect("step succeeds");

    let vehicles = world.vehicles().collect::<Vec<_>>();
    let vehicle = vehicles[0];
    assert_eq!(vehicle.status, VehicleStatus::Active);
    assert_close(vehicle.current_speed.value(), 1.4);
    assert_close(vehicle.applied_acceleration.value(), 1.4);
    assert_close(vehicle.edge_progress.value(), 8.2);
}

#[test]
fn stopped_and_completed_have_zero_motion_state() {
    let mut world = single_edge_world(1_000, |profile| {
        vec![
            VehicleSpawnInput::stopped(
                "V1",
                profile,
                "R1",
                0,
                EdgeProgress::try_new(2.0).expect("valid progress"),
            ),
            VehicleSpawnInput::completed(
                "V2",
                profile,
                "R1",
                1,
                EdgeProgress::try_new(10.0).expect("valid progress"),
            ),
        ]
    });

    let result = world.step(TickInput::new(1_000)).expect("step succeeds");
    assert!(result.events.is_empty());

    let vehicles = world.vehicles().collect::<Vec<_>>();
    let stopped = vehicles[0];
    assert_eq!(stopped.status, VehicleStatus::Stopped);
    assert_eq!(stopped.current_speed, Speed::ZERO);
    assert_eq!(stopped.applied_acceleration, Acceleration::ZERO);
    assert_eq!(stopped.edge_progress.value(), 2.0);

    let completed = vehicles[1];
    assert_eq!(completed.status, VehicleStatus::Completed);
    assert_eq!(completed.current_speed, Speed::ZERO);
    assert_eq!(completed.applied_acceleration, Acceleration::ZERO);
    assert_eq!(completed.edge_progress.value(), 10.0);
}

#[test]
fn inactive_nonzero_initial_speed_is_rejected() {
    let error = world_with_test_profile(
        1_000,
        LaneGraph::try_new([LaneEdge::new(
            "A",
            edge_length(10.0),
            std::iter::empty::<&str>(),
        )])
        .expect("valid lane graph"),
        [Route::try_new("R1", ["A"]).expect("valid route")],
        |profile| {
            vec![VehicleSpawnInput::new(
                "V1",
                profile,
                "R1",
                0,
                EdgeProgress::ZERO,
                Speed::try_new(1.0).expect("valid speed"),
                VehicleStatus::Stopped,
            )]
        },
    )
    .expect_err("inactive nonzero speed must fail");

    std::assert_matches!(
        error,
        CoreError::InvalidInactiveVehicleMotion {
            vehicle_id,
            status: VehicleStatus::Stopped,
            initial_speed
        } if vehicle_id == "V1" && initial_speed == 1.0
    );
}

#[test]
fn invalid_numeric_inputs_are_rejected() {
    std::assert_matches!(
        CoreWorld::new(0).expect_err("zero fixed delta must fail"),
        CoreError::InvalidFixedDeltaTime {
            fixed_delta_time_ms: 0
        }
    );

    std::assert_matches!(
        Speed::try_new(f64::INFINITY),
        Err(CoreError::InvalidSpeed { .. })
    );
    std::assert_matches!(
        Speed::try_new(f64::NEG_INFINITY),
        Err(CoreError::InvalidSpeed { speed }) if speed == f64::NEG_INFINITY
    );
    std::assert_matches!(
        Speed::try_new(f64::NAN),
        Err(CoreError::InvalidSpeed { speed }) if speed.is_nan()
    );
    std::assert_matches!(
        Speed::try_new(-1.0),
        Err(CoreError::InvalidSpeed { speed }) if speed == -1.0
    );

    for invalid_acceleration in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        std::assert_matches!(
            Acceleration::try_new(invalid_acceleration),
            Err(CoreError::InvalidAcceleration { acceleration })
                if acceleration.is_nan() && invalid_acceleration.is_nan()
                    || acceleration == invalid_acceleration
        );
    }
    assert_eq!(
        Acceleration::try_new(-2.5)
            .expect("signed acceleration is valid")
            .value(),
        -2.5
    );
    assert_eq!(
        Speed::try_new(-0.0)
            .expect("zero speed is valid")
            .value()
            .to_bits(),
        0.0_f64.to_bits()
    );
    assert_eq!(
        Acceleration::try_new(-0.0)
            .expect("zero acceleration is valid")
            .value()
            .to_bits(),
        0.0_f64.to_bits()
    );
    assert_eq!(
        EdgeProgress::try_new(-0.0)
            .expect("zero progress is valid")
            .value()
            .to_bits(),
        0.0_f64.to_bits()
    );

    std::assert_matches!(
        EdgeProgress::try_new(f64::NAN),
        Err(CoreError::InvalidEdgeProgress { edge_progress }) if edge_progress.is_nan()
    );
    std::assert_matches!(
        EdgeProgress::try_new(f64::INFINITY),
        Err(CoreError::InvalidEdgeProgress { edge_progress }) if edge_progress == f64::INFINITY
    );
    std::assert_matches!(
        EdgeProgress::try_new(f64::NEG_INFINITY),
        Err(CoreError::InvalidEdgeProgress { edge_progress })
            if edge_progress == f64::NEG_INFINITY
    );
    std::assert_matches!(
        EdgeProgress::try_new(-0.5),
        Err(CoreError::InvalidEdgeProgress { edge_progress }) if edge_progress == -0.5
    );
}
