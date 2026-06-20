use laneflow_core::{
    CoreError, CoreWorld, EdgeProgress, Speed, TickInput, VehicleState, VehicleStatus,
};

#[test]
fn fixed_tick_advances_post_step_time() {
    let mut world = CoreWorld::new(1000).expect("valid world");

    let result = world.step(TickInput::new(1000)).expect("step succeeds");

    assert_eq!(world.fixed_delta_time_ms(), 1000);
    assert_eq!(world.tick_index(), 1);
    assert_eq!(world.time_ms(), 1000);
    assert_eq!(result.tick_index, 1);
    assert_eq!(result.time_ms, 1000);
    assert!(result.events.is_empty());
}

#[test]
fn delta_mismatch_returns_error_and_keeps_world_unchanged() {
    let vehicle = VehicleState::active(
        "V1",
        "R1",
        0,
        EdgeProgress::try_new(3.0).expect("valid progress"),
        Speed::try_new(2.0).expect("valid speed"),
    );
    let mut world = CoreWorld::with_vehicles(1000, vec![vehicle]).expect("valid world");
    let before = world.clone();

    let error = world
        .step(TickInput::new(500))
        .expect_err("delta mismatch must fail");

    assert_eq!(
        error,
        CoreError::TickDeltaMismatch {
            expected_delta_time_ms: 1000,
            actual_delta_time_ms: 500
        }
    );
    assert_eq!(world, before);
}

#[test]
fn active_zero_speed_is_valid_and_does_not_change_progress() {
    let vehicle = VehicleState::active(
        "V1",
        "R1",
        0,
        EdgeProgress::try_new(7.5).expect("valid progress"),
        Speed::try_new(0.0).expect("valid speed"),
    );
    let mut world = CoreWorld::with_vehicles(1000, vec![vehicle]).expect("valid world");

    world.step(TickInput::new(1000)).expect("step succeeds");

    let vehicle = &world.vehicles()[0];
    assert_eq!(vehicle.status, VehicleStatus::Active);
    assert_eq!(vehicle.speed.value(), 0.0);
    assert_eq!(vehicle.edge_progress.value(), 7.5);
    assert_eq!(vehicle.effective_speed(), Speed::ZERO);
}

#[test]
fn stopped_and_completed_keep_speed_but_have_zero_effective_speed() {
    let stopped = VehicleState::stopped(
        "V1",
        "R1",
        0,
        EdgeProgress::try_new(2.0).expect("valid progress"),
        Speed::try_new(6.0).expect("valid speed"),
    );
    let completed = VehicleState::completed(
        "V2",
        "R1",
        1,
        EdgeProgress::try_new(10.0).expect("valid progress"),
        Speed::try_new(8.0).expect("valid speed"),
    );
    let mut world = CoreWorld::with_vehicles(1000, vec![stopped, completed]).expect("valid world");

    world.step(TickInput::new(1000)).expect("step succeeds");

    let stopped = &world.vehicles()[0];
    assert_eq!(stopped.status, VehicleStatus::Stopped);
    assert_eq!(stopped.speed.value(), 6.0);
    assert_eq!(stopped.edge_progress.value(), 2.0);
    assert_eq!(stopped.effective_speed(), Speed::ZERO);

    let completed = &world.vehicles()[1];
    assert_eq!(completed.status, VehicleStatus::Completed);
    assert_eq!(completed.speed.value(), 8.0);
    assert_eq!(completed.edge_progress.value(), 10.0);
    assert_eq!(completed.effective_speed(), Speed::ZERO);
}

#[test]
fn invalid_numeric_inputs_are_rejected() {
    assert_eq!(
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
        Speed::try_new(-1.0),
        Err(CoreError::InvalidSpeed { speed }) if speed == -1.0
    );

    std::assert_matches!(
        EdgeProgress::try_new(f64::NAN),
        Err(CoreError::InvalidEdgeProgress { .. })
    );
    std::assert_matches!(
        EdgeProgress::try_new(-0.5),
        Err(CoreError::InvalidEdgeProgress { edge_progress }) if edge_progress == -0.5
    );
}
