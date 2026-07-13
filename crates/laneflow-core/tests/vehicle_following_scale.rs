use laneflow_core::{CoreWorld, TickInput, VehicleStatus};

#[allow(dead_code)]
#[path = "support/vehicle_following_scenarios.rs"]
mod scenarios;

use scenarios::{
    FIXED_DELTA_TIME_MS, VEHICLE_COUNT, VEHICLE_LENGTH, dense_platoon_world, free_flow_world,
    projection_event_count, projection_heavy_world, stop_and_go_world,
};

const EPSILON: f64 = 1.0e-9;

fn step_once(world: &mut CoreWorld) -> usize {
    world
        .step(TickInput::new(FIXED_DELTA_TIME_MS))
        .expect("10k smoke step must succeed")
        .events
        .len()
}

fn assert_finite_and_no_overlap(world: &CoreWorld) {
    let mut fronts = Vec::with_capacity(VEHICLE_COUNT);
    for vehicle in world.vehicles() {
        assert!(matches!(
            vehicle.status,
            VehicleStatus::Active | VehicleStatus::Stopped
        ));
        assert!(vehicle.current_speed.value().is_finite());
        assert!(vehicle.current_speed.value() >= 0.0);
        assert!(vehicle.applied_acceleration.value().is_finite());
        assert!(vehicle.edge_progress.value().is_finite());
        fronts.push(vehicle.edge_progress.value());
    }
    assert_eq!(fronts.len(), VEHICLE_COUNT);

    fronts.sort_unstable_by(f64::total_cmp);
    for pair in fronts.windows(2) {
        assert!(pair[1] - pair[0] + EPSILON >= VEHICLE_LENGTH);
    }
}

#[test]
fn ten_thousand_vehicle_scenarios_complete_functional_smoke() {
    let mut free_flow = free_flow_world(VEHICLE_COUNT);
    let mut dense_platoon = dense_platoon_world(VEHICLE_COUNT);
    let mut stop_and_go = stop_and_go_world(VEHICLE_COUNT);
    let mut projection_heavy = projection_heavy_world(VEHICLE_COUNT);

    assert_eq!(step_once(&mut free_flow), 0);
    assert_eq!(step_once(&mut dense_platoon), 0);
    step_once(&mut stop_and_go);
    assert_eq!(
        step_once(&mut projection_heavy),
        projection_event_count(VEHICLE_COUNT)
    );

    for world in [&free_flow, &dense_platoon, &stop_and_go, &projection_heavy] {
        assert_finite_and_no_overlap(world);
    }
}
