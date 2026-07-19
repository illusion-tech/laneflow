use std::collections::HashMap;

use laneflow_core::{CoreEvent, CoreWorld, EdgeHandle, TickInput, VehicleState};

#[path = "support/signals_validation_scenarios.rs"]
mod scenarios;

use scenarios::{
    GROUPS_PER_CONTROLLER, SIGNAL_FIXED_DELTA_TIME_MS, SIGNAL_SCALING_VEHICLE_COUNT,
    SIGNAL_STEP_COUNT, SIGNAL_VEHICLE_COUNT, SignalScenario, SignalScenarioMode, signal_scenario,
};

fn snapshot(world: &CoreWorld) -> Vec<VehicleState> {
    world.vehicles().cloned().collect()
}

fn assert_finite_and_non_overlapping(world: &CoreWorld) {
    let mut fronts_by_edge: HashMap<EdgeHandle, Vec<(f64, f64)>> = HashMap::new();
    for vehicle in world.vehicles() {
        assert!(vehicle.edge_progress.value().is_finite());
        assert!(vehicle.current_speed.value().is_finite());
        assert!(vehicle.applied_acceleration.value().is_finite());
        let edge = world
            .route_edges(vehicle.route)
            .expect("vehicle route must exist")[vehicle.route_edge_index];
        let length = world
            .vehicle_profile(vehicle.profile)
            .expect("vehicle profile must exist")
            .iidm()
            .length;
        fronts_by_edge
            .entry(edge)
            .or_default()
            .push((vehicle.edge_progress.value(), f64::from(length)));
    }

    for vehicles in fronts_by_edge.values_mut() {
        vehicles.sort_by(|left, right| left.0.total_cmp(&right.0));
        for pair in vehicles.windows(2) {
            let follower_front = pair[0].0;
            let (leader_front, leader_length) = pair[1];
            assert!(
                follower_front <= leader_front - leader_length + 1.0e-5,
                "vehicles must not overlap: follower={follower_front}, leader={leader_front}"
            );
        }
    }
}

fn assert_topology(scenario: &SignalScenario, mode: SignalScenarioMode, vehicle_count: usize) {
    assert!(!mode.benchmark_name().is_empty());
    let expected_routes = vehicle_count / scenarios::VEHICLES_PER_ROUTE;
    let controlled = !matches!(
        mode,
        SignalScenarioMode::NoSignals | SignalScenarioMode::AllNone
    );
    assert_eq!(scenario.route_count, expected_routes);
    assert_eq!(scenario.world.routes().count(), expected_routes);
    assert_eq!(scenario.world.vehicles().count(), vehicle_count);
    assert_eq!(
        scenario.controller_count,
        if controlled {
            expected_routes / GROUPS_PER_CONTROLLER
        } else {
            0
        }
    );
    assert_eq!(
        scenario.world.signal_controller_states().count(),
        scenario.controller_count
    );
    assert_eq!(
        scenario.group_count,
        if controlled { expected_routes } else { 0 }
    );
    assert_eq!(
        scenario.world.signal_group_states().count(),
        scenario.group_count
    );
    assert_eq!(
        scenario.gate_count,
        if mode == SignalScenarioMode::NoSignals {
            0
        } else {
            expected_routes
        }
    );
    assert_eq!(
        scenario.world.movement_gate_states().count(),
        scenario.gate_count
    );
}

#[test]
fn uncontrolled_and_green_10k_worlds_remain_behaviorally_equivalent() {
    let mut no_signals = signal_scenario(SIGNAL_VEHICLE_COUNT, SignalScenarioMode::NoSignals);
    let mut all_none = signal_scenario(SIGNAL_VEHICLE_COUNT, SignalScenarioMode::AllNone);
    let mut all_green = signal_scenario(SIGNAL_VEHICLE_COUNT, SignalScenarioMode::AllGreen);
    assert_topology(
        &no_signals,
        SignalScenarioMode::NoSignals,
        SIGNAL_VEHICLE_COUNT,
    );
    assert_topology(&all_none, SignalScenarioMode::AllNone, SIGNAL_VEHICLE_COUNT);
    assert_topology(
        &all_green,
        SignalScenarioMode::AllGreen,
        SIGNAL_VEHICLE_COUNT,
    );

    for _ in 0..SIGNAL_STEP_COUNT {
        let no_signal_result = no_signals
            .world
            .step(TickInput::new(SIGNAL_FIXED_DELTA_TIME_MS))
            .expect("no-Signals step must succeed");
        let all_none_result = all_none
            .world
            .step(TickInput::new(SIGNAL_FIXED_DELTA_TIME_MS))
            .expect("signalControl:none step must succeed");
        let all_green_result = all_green
            .world
            .step(TickInput::new(SIGNAL_FIXED_DELTA_TIME_MS))
            .expect("all-green step must succeed");
        assert_eq!(all_none_result, no_signal_result);
        assert_eq!(all_green_result, no_signal_result);
        assert_eq!(snapshot(&all_none.world), snapshot(&no_signals.world));
        assert_eq!(snapshot(&all_green.world), snapshot(&no_signals.world));
    }

    assert_finite_and_non_overlapping(&no_signals.world);
    assert_finite_and_non_overlapping(&all_none.world);
    assert_finite_and_non_overlapping(&all_green.world);
}

#[test]
fn controlled_10k_modes_preserve_topology_geometry_and_event_semantics() {
    for mode in SignalScenarioMode::ALL.into_iter().filter(|mode| {
        matches!(
            mode,
            SignalScenarioMode::AllRed
                | SignalScenarioMode::StopRelease
                | SignalScenarioMode::MixedOffsets
        )
    }) {
        let mut scenario = signal_scenario(SIGNAL_VEHICLE_COUNT, mode);
        assert_topology(&scenario, mode, SIGNAL_VEHICLE_COUNT);
        let mut signal_stops = 0;
        let mut changed_edges = 0;
        let mut phase_changes = 0;
        let mut aspect_changes = 0;
        for _ in 0..SIGNAL_STEP_COUNT {
            for event in scenario
                .world
                .step(TickInput::new(SIGNAL_FIXED_DELTA_TIME_MS))
                .expect("controlled signal step must succeed")
                .events
            {
                match event {
                    CoreEvent::VehicleSignalStopProjectionApplied(_) => signal_stops += 1,
                    CoreEvent::VehicleChangedEdge(_) => changed_edges += 1,
                    CoreEvent::SignalPhaseChanged(_) => phase_changes += 1,
                    CoreEvent::SignalGroupAspectChanged(_) => aspect_changes += 1,
                    _ => {}
                }
            }
        }

        assert!(signal_stops > 0, "{mode:?} must exercise SignalStop");
        match mode {
            SignalScenarioMode::AllRed => {
                assert_eq!(changed_edges, 0);
                assert_eq!(phase_changes, 0);
                assert_eq!(aspect_changes, 0);
            }
            SignalScenarioMode::StopRelease | SignalScenarioMode::MixedOffsets => {
                assert!(changed_edges > 0, "{mode:?} must release some vehicles");
                assert!(phase_changes > 0, "{mode:?} must change phases");
                assert!(aspect_changes > 0, "{mode:?} must change aspects");
            }
            _ => unreachable!(),
        }
        assert_finite_and_non_overlapping(&scenario.world);
    }
}

#[test]
#[ignore = "100k pressure smoke is an explicit G3 validation, not a regular CI test"]
fn mixed_offset_100k_pressure_smoke() {
    let mut scenario = signal_scenario(
        SIGNAL_SCALING_VEHICLE_COUNT,
        SignalScenarioMode::MixedOffsets,
    );
    assert_topology(
        &scenario,
        SignalScenarioMode::MixedOffsets,
        SIGNAL_SCALING_VEHICLE_COUNT,
    );
    for _ in 0..SIGNAL_STEP_COUNT {
        scenario
            .world
            .step(TickInput::new(SIGNAL_FIXED_DELTA_TIME_MS))
            .expect("100k pressure step must succeed");
    }
    assert_finite_and_non_overlapping(&scenario.world);
}
