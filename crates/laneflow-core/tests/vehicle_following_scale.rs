use laneflow_core::{CoreEvent, CoreWorld, TickInput, VehicleHandle, VehicleStatus};

#[allow(dead_code)]
#[path = "support/vehicle_following_scenarios.rs"]
mod scenarios;

use scenarios::{
    FIXED_DELTA_TIME_MS, LOCALITY_EDGE_LENGTH, VEHICLE_COUNT, VEHICLE_LENGTH, dense_platoon_world,
    free_flow_world, projection_event_count, projection_heavy_world,
    single_edge_dense_platoon_world, single_edge_free_flow_world, single_edge_stop_and_go_world,
    stop_and_go_world,
};

const PHYSICAL_GAP_TOLERANCE_METERS: f64 = 1.0e-5;
const REFERENCE_EQUIVALENCE_EPSILON: f64 = 1.0e-5;
const EQUIVALENCE_VEHICLE_COUNT: usize = 32;
const EQUIVALENCE_EDGE_LENGTH: f64 = 100.0;

fn step_once(world: &mut CoreWorld) -> usize {
    world
        .step(TickInput::new(FIXED_DELTA_TIME_MS))
        .expect("10k smoke step must succeed")
        .events
        .len()
}

fn step_vehicle_following_summary(
    world: &mut CoreWorld,
) -> (Vec<(VehicleHandle, VehicleHandle)>, usize) {
    let result = world
        .step(TickInput::new(FIXED_DELTA_TIME_MS))
        .expect("10k vehicle-following step must succeed");
    let mut safety_projections = Vec::new();
    let mut edge_changes = 0;
    for event in result.events {
        match event {
            CoreEvent::VehicleFollowingSafetyProjectionApplied(event) => {
                safety_projections.push((event.vehicle, event.leader));
            }
            CoreEvent::VehicleChangedEdge(event) => {
                assert_eq!(event.from_route_edge_index + 1, event.to_route_edge_index);
                edge_changes += 1;
            }
            unexpected => panic!("unexpected vehicle-following event: {unexpected:?}"),
        }
    }
    (safety_projections, edge_changes)
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
        fronts.push(
            vehicle.route_edge_index as f64 * LOCALITY_EDGE_LENGTH + vehicle.edge_progress.value(),
        );
    }
    assert_eq!(fronts.len(), VEHICLE_COUNT);

    fronts.sort_unstable_by(f64::total_cmp);
    for pair in fronts.windows(2) {
        assert!(pair[1] - pair[0] + PHYSICAL_GAP_TOLERANCE_METERS >= f64::from(VEHICLE_LENGTH));
    }
}

fn assert_locality_preserving_equivalence(
    reference: &CoreWorld,
    locality: &CoreWorld,
    locality_edge_length: f64,
) {
    assert!(
        locality
            .lane_graph()
            .edges()
            .all(|edge| f64::from(edge.length().value()) <= locality_edge_length)
    );
    for (reference, locality) in reference.vehicles().zip(locality.vehicles()) {
        let locality_route_progress = locality.route_edge_index as f64 * locality_edge_length
            + locality.edge_progress.value();
        assert_eq!(reference.handle, locality.handle);
        assert_eq!(reference.profile, locality.profile);
        assert_eq!(reference.route, locality.route);
        assert_eq!(reference.status, locality.status);
        let progress_difference = (reference.edge_progress.value() - locality_route_progress).abs();
        assert!(
            progress_difference <= REFERENCE_EQUIVALENCE_EPSILON,
            "vehicle={:?} reference_progress={} locality_route_progress={} locality_edge_index={} locality_edge_progress={} difference={}",
            reference.handle,
            reference.edge_progress.value(),
            locality_route_progress,
            locality.route_edge_index,
            locality.edge_progress.value(),
            progress_difference,
        );
        assert!(
            f64::from((reference.current_speed.value() - locality.current_speed.value()).abs(),)
                <= REFERENCE_EQUIVALENCE_EPSILON
        );
        let acceleration_difference =
            (reference.applied_acceleration.value() - locality.applied_acceleration.value()).abs();
        assert!(
            f64::from(acceleration_difference) <= REFERENCE_EQUIVALENCE_EPSILON,
            "vehicle={:?} reference_acceleration={} locality_acceleration={} difference={}",
            reference.handle,
            reference.applied_acceleration.value(),
            locality.applied_acceleration.value(),
            acceleration_difference,
        );
    }
    assert_eq!(reference.vehicles().count(), locality.vehicles().count());
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

#[test]
fn locality_preserving_platoons_match_single_edge_reference_for_sixty_steps() {
    let scenario_pairs = [
        (
            single_edge_free_flow_world(EQUIVALENCE_VEHICLE_COUNT),
            scenarios::free_flow_world_with_edge_cap(
                EQUIVALENCE_VEHICLE_COUNT,
                EQUIVALENCE_EDGE_LENGTH,
            ),
        ),
        (
            single_edge_dense_platoon_world(EQUIVALENCE_VEHICLE_COUNT),
            scenarios::dense_platoon_world_with_edge_cap(
                EQUIVALENCE_VEHICLE_COUNT,
                EQUIVALENCE_EDGE_LENGTH,
            ),
        ),
        (
            single_edge_stop_and_go_world(EQUIVALENCE_VEHICLE_COUNT),
            scenarios::stop_and_go_world_with_edge_cap(
                EQUIVALENCE_VEHICLE_COUNT,
                EQUIVALENCE_EDGE_LENGTH,
            ),
        ),
    ];

    let mut locality_edge_changes = 0;
    for (mut reference, mut locality) in scenario_pairs {
        assert_locality_preserving_equivalence(&reference, &locality, EQUIVALENCE_EDGE_LENGTH);
        for _ in 0..scenarios::STEP_COUNT {
            let (reference_safety_projections, reference_edge_changes) =
                step_vehicle_following_summary(&mut reference);
            let (locality_safety_projections, edge_changes) =
                step_vehicle_following_summary(&mut locality);
            assert_eq!(reference_safety_projections, locality_safety_projections);
            assert_eq!(reference_edge_changes, 0);
            locality_edge_changes += edge_changes;
            assert_locality_preserving_equivalence(&reference, &locality, EQUIVALENCE_EDGE_LENGTH);
        }
    }
    assert!(locality_edge_changes > 0);
}
