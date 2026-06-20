use laneflow_core::{CoreError, CoreWorld, TickInput, VehicleState, VehicleStatus};

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
    let vehicle = VehicleState::active("V1", "R1", 0, 3.0, 2.0);
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
    let vehicle = VehicleState::active("V1", "R1", 0, 7.5, 0.0);
    let mut world = CoreWorld::with_vehicles(1000, vec![vehicle]).expect("valid world");

    world.step(TickInput::new(1000)).expect("step succeeds");

    let vehicle = &world.vehicles()[0];
    assert_eq!(vehicle.status, VehicleStatus::Active);
    assert_eq!(vehicle.speed, 0.0);
    assert_eq!(vehicle.edge_progress, 7.5);
    assert_eq!(vehicle.effective_speed(), 0.0);
}

#[test]
fn stopped_and_completed_keep_speed_but_have_zero_effective_speed() {
    let stopped = VehicleState::stopped("V1", "R1", 0, 2.0, 6.0);
    let completed = VehicleState::completed("V2", "R1", 1, 10.0, 8.0);
    let mut world = CoreWorld::with_vehicles(1000, vec![stopped, completed]).expect("valid world");

    world.step(TickInput::new(1000)).expect("step succeeds");

    let stopped = &world.vehicles()[0];
    assert_eq!(stopped.status, VehicleStatus::Stopped);
    assert_eq!(stopped.speed, 6.0);
    assert_eq!(stopped.edge_progress, 2.0);
    assert_eq!(stopped.effective_speed(), 0.0);

    let completed = &world.vehicles()[1];
    assert_eq!(completed.status, VehicleStatus::Completed);
    assert_eq!(completed.speed, 8.0);
    assert_eq!(completed.edge_progress, 10.0);
    assert_eq!(completed.effective_speed(), 0.0);
}

#[test]
fn invalid_world_inputs_are_rejected() {
    assert_eq!(
        CoreWorld::new(0).expect_err("zero fixed delta must fail"),
        CoreError::InvalidFixedDeltaTime {
            fixed_delta_time_ms: 0
        }
    );

    let invalid_speed = VehicleState::active("V1", "R1", 0, 0.0, f64::INFINITY);
    assert!(matches!(
        CoreWorld::with_vehicles(1000, vec![invalid_speed]),
        Err(CoreError::InvalidVehicleSpeed { .. })
    ));

    let invalid_progress = VehicleState::active("V1", "R1", 0, f64::NAN, 1.0);
    assert!(matches!(
        CoreWorld::with_vehicles(1000, vec![invalid_progress]),
        Err(CoreError::InvalidVehicleEdgeProgress { .. })
    ));
}
