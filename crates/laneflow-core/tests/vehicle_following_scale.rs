use laneflow_core::{CoreEvent, CoreWorld, TickInput, VehicleHandle, VehicleStatus};

#[allow(dead_code)]
#[path = "support/vehicle_following_scenarios.rs"]
mod scenarios;

use scenarios::{
    FIXED_DELTA_TIME_MS, LOCALITY_EDGE_LENGTH, SCALING_VEHICLE_COUNT, SPEED_LIMIT_EDGE_LENGTH,
    VEHICLE_COUNT, VEHICLE_LENGTH, dense_platoon_world, dense_platoon_world_with_edge_cap,
    free_flow_world, free_flow_world_with_edge_cap, projection_event_count, projection_heavy_world,
    speed_limit_transition_world, stop_and_go_world, stop_and_go_world_with_edge_cap,
};

const EPSILON: f64 = 1.0e-9;
const REFERENCE_EQUIVALENCE_EPSILON: f64 = 1.0e-8;
const EQUIVALENCE_EDGE_LENGTH: f64 = 9_997.5;

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

fn assert_finite_and_no_overlap(world: &CoreWorld, uniform_route_edge_length: Option<f64>) {
    let vehicle_count = world.vehicles().count();
    let mut fronts = Vec::with_capacity(vehicle_count);
    for vehicle in world.vehicles() {
        assert!(matches!(
            vehicle.status,
            VehicleStatus::Active | VehicleStatus::Stopped
        ));
        assert!(vehicle.current_speed.value().is_finite());
        assert!(vehicle.current_speed.value() >= 0.0);
        assert!(vehicle.applied_acceleration.value().is_finite());
        assert!(vehicle.edge_progress.value().is_finite());
        let route_progress = uniform_route_edge_length.map_or_else(
            || vehicle.edge_progress.value(),
            |edge_length| {
                vehicle.route_edge_index as f64 * edge_length + vehicle.edge_progress.value()
            },
        );
        fronts.push(route_progress);
    }
    assert_eq!(fronts.len(), vehicle_count);

    fronts.sort_unstable_by(f64::total_cmp);
    for pair in fronts.windows(2) {
        assert!(pair[1] - pair[0] + EPSILON >= VEHICLE_LENGTH);
    }
}

fn assert_uniform_min_gap(world: &CoreWorld, uniform_route_edge_length: Option<f64>) {
    let mut fronts = world
        .vehicles()
        .map(|vehicle| {
            uniform_route_edge_length.map_or_else(
                || vehicle.edge_progress.value(),
                |edge_length| {
                    vehicle.route_edge_index as f64 * edge_length + vehicle.edge_progress.value()
                },
            )
        })
        .collect::<Vec<_>>();
    fronts.sort_unstable_by(f64::total_cmp);
    for pair in fronts.windows(2) {
        assert!(pair[1] - pair[0] - VEHICLE_LENGTH + EPSILON >= 2.0);
    }
}

fn assert_locality_preserving_equivalence(
    reference: &CoreWorld,
    locality: &CoreWorld,
    edge_length: f64,
) {
    assert!(
        locality
            .lane_graph()
            .edges()
            .all(|edge| edge.length().value() <= edge_length)
    );
    for (reference, locality) in reference.vehicles().zip(locality.vehicles()) {
        let locality_route_progress =
            locality.route_edge_index as f64 * edge_length + locality.edge_progress.value();
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
            (reference.current_speed.value() - locality.current_speed.value()).abs()
                <= REFERENCE_EQUIVALENCE_EPSILON
        );
        let acceleration_difference =
            (reference.applied_acceleration.value() - locality.applied_acceleration.value()).abs();
        assert!(
            acceleration_difference <= REFERENCE_EQUIVALENCE_EPSILON,
            "vehicle={:?} reference_acceleration={} locality_acceleration={} difference={}",
            reference.handle,
            reference.applied_acceleration.value(),
            locality.applied_acceleration.value(),
            acceleration_difference,
        );
    }
    assert_eq!(reference.vehicles().count(), locality.vehicles().count());
}

fn assert_speed_limit_compliance(world: &CoreWorld) {
    for vehicle in world.vehicles() {
        let edge = world.route_edges(vehicle.route).expect("route")[vehicle.route_edge_index];
        let limit = world
            .lane_graph()
            .edge_speed_limit(edge)
            .expect("edge limit")
            .value();
        assert!(
            vehicle.current_speed.value() <= limit,
            "vehicle={:?} speed={} limit={limit}",
            vehicle.handle,
            vehicle.current_speed.value(),
        );
    }
}

#[test]
fn ten_thousand_vehicle_scenarios_complete_functional_smoke() {
    let mut free_flow = free_flow_world(VEHICLE_COUNT);
    let mut dense_platoon = dense_platoon_world(VEHICLE_COUNT);
    let mut stop_and_go = stop_and_go_world(VEHICLE_COUNT);
    let mut projection_heavy = projection_heavy_world(VEHICLE_COUNT);
    let mut speed_limits = speed_limit_transition_world(VEHICLE_COUNT);

    assert_eq!(step_once(&mut free_flow), 0);
    assert_eq!(step_once(&mut dense_platoon), 0);
    step_once(&mut stop_and_go);
    assert_eq!(
        step_once(&mut projection_heavy),
        projection_event_count(VEHICLE_COUNT)
    );
    assert_eq!(step_once(&mut speed_limits), 0);

    for world in [&free_flow, &dense_platoon, &stop_and_go] {
        assert_finite_and_no_overlap(world, None);
        assert_uniform_min_gap(world, None);
    }
    assert_finite_and_no_overlap(&projection_heavy, Some(LOCALITY_EDGE_LENGTH));
    assert_finite_and_no_overlap(&speed_limits, Some(SPEED_LIMIT_EDGE_LENGTH));
    assert_speed_limit_compliance(&speed_limits);
}

#[test]
#[ignore = "100k speed-limit scaling is an explicit G3 validation"]
fn hundred_thousand_vehicle_speed_limit_smoke_is_compliant() {
    let mut world = speed_limit_transition_world(SCALING_VEHICLE_COUNT);

    assert_eq!(step_once(&mut world), 0);
    assert_eq!(world.vehicles().count(), SCALING_VEHICLE_COUNT);
    assert_finite_and_no_overlap(&world, Some(SPEED_LIMIT_EDGE_LENGTH));
    assert_speed_limit_compliance(&world);
}

#[test]
#[ignore = "100k minimum-gap scaling is an explicit #222 validation"]
fn hundred_thousand_vehicle_min_gap_smoke_preserves_clearance() {
    let mut world = dense_platoon_world(SCALING_VEHICLE_COUNT);

    assert_eq!(step_once(&mut world), 0);
    assert_eq!(world.vehicles().count(), SCALING_VEHICLE_COUNT);
    assert_finite_and_no_overlap(&world, None);
    assert_uniform_min_gap(&world, None);
}

#[test]
fn locality_preserving_platoons_match_single_edge_reference_for_sixty_steps() {
    let scenario_pairs = [
        (
            free_flow_world(VEHICLE_COUNT),
            free_flow_world_with_edge_cap(VEHICLE_COUNT, EQUIVALENCE_EDGE_LENGTH),
        ),
        (
            dense_platoon_world(VEHICLE_COUNT),
            dense_platoon_world_with_edge_cap(VEHICLE_COUNT, EQUIVALENCE_EDGE_LENGTH),
        ),
        (
            stop_and_go_world(VEHICLE_COUNT),
            stop_and_go_world_with_edge_cap(VEHICLE_COUNT, EQUIVALENCE_EDGE_LENGTH),
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
