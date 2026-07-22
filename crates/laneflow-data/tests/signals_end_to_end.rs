use laneflow_core::{
    CoreEvent, CoreWorld, EdgeProgress, MovementGateKey, MovementGateSignalState, SignalAspect,
    SignalLayerPermission, Speed, TickInput, VehicleSpawnInput,
};
use laneflow_data::from_json_str;

const SIGNALS_BASELINE: &str =
    include_str!("../../../examples/data/v0.7-parking-signals-baseline.laneflow.json");
const DELTA_MS: u64 = 1_000;

fn baseline_world() -> CoreWorld {
    let loaded = from_json_str(SIGNALS_BASELINE).expect("canonical v0.7 fixture must load");
    CoreWorld::with_traffic_data(DELTA_MS, loaded.into_initial_traffic_data(), Vec::new())
        .expect("canonical Signals fixture must initialize CoreWorld")
}

#[test]
fn production_loader_drives_controlled_and_uncontrolled_gate_snapshots() {
    let world = baseline_world();
    let entry = world.edge_handle("entry").expect("entry");
    let through = world.edge_handle("through").expect("through");
    let bypass = world.edge_handle("bypass").expect("bypass");

    std::assert_matches!(
        world
            .movement_gate_state(MovementGateKey::new(entry, through))
            .expect("controlled Gate")
            .signal(),
        MovementGateSignalState::Controlled {
            aspect: SignalAspect::Green,
            permission: SignalLayerPermission::ProtectedAllow,
            ..
        }
    );
    std::assert_matches!(
        world
            .movement_gate_state(MovementGateKey::new(entry, bypass))
            .expect("uncontrolled Gate")
            .signal(),
        MovementGateSignalState::Uncontrolled
    );
}

#[test]
fn canonical_green_yellow_red_cycle_queues_and_releases_at_tick_start_authority() {
    let mut world = baseline_world();
    let controller = world
        .signals()
        .controller_handle("controller-main")
        .expect("controller");
    let group = world.signals().group_handle("main").expect("group");
    let profile = world
        .vehicle_profile_handle("passenger-car")
        .expect("profile");

    let mut phase_aspects = Vec::new();
    for _ in 0..29 {
        let result = world.step(TickInput::new(DELTA_MS)).expect("timing step");
        phase_aspects.extend(result.events.iter().filter_map(|event| match event {
            CoreEvent::SignalGroupAspectChanged(event) => Some(event.to_aspect),
            _ => None,
        }));
    }
    assert_eq!(world.time_ms(), 29_000);
    assert_eq!(
        world
            .signal_group_state(group)
            .expect("yellow state")
            .aspect(),
        SignalAspect::Yellow
    );
    assert_eq!(
        world
            .signal_controller_state(controller)
            .expect("controller state")
            .cycle_position_ms(),
        30_000
    );

    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::active(
            "queued-controlled-vehicle",
            profile,
            "controlled-route",
            0,
            EdgeProgress::try_new(100.0).unwrap(),
            Speed::try_new(10.0).unwrap(),
        ))
        .expect("vehicle at StopLine boundary must spawn");
    let mut denied_aspects = Vec::new();
    let mut red_was_observed_while_queued = false;
    for _ in 0..23 {
        let result = world.step(TickInput::new(DELTA_MS)).expect("denied step");
        for event in result.events {
            match event {
                CoreEvent::VehicleSignalStopProjectionApplied(event) => {
                    denied_aspects.push(event.aspect);
                }
                CoreEvent::SignalGroupAspectChanged(event) => {
                    phase_aspects.push(event.to_aspect);
                }
                _ => {}
            }
        }
        assert_eq!(
            world
                .vehicle(vehicle)
                .expect("queued vehicle")
                .route_edge_index,
            0
        );
        if world
            .signal_group_state(group)
            .expect("group state")
            .aspect()
            == SignalAspect::Red
        {
            red_was_observed_while_queued = true;
        }
    }
    assert_eq!(world.time_ms(), 52_000);
    assert_eq!(
        world
            .signal_group_state(group)
            .expect("green state")
            .aspect(),
        SignalAspect::Green
    );
    assert!(denied_aspects.contains(&SignalAspect::Yellow));
    assert!(red_was_observed_while_queued);
    assert_eq!(
        phase_aspects,
        [SignalAspect::Yellow, SignalAspect::Red, SignalAspect::Green]
    );

    let released = world
        .step(TickInput::new(DELTA_MS))
        .expect("green release step");
    assert_eq!(
        world
            .vehicle(vehicle)
            .expect("released vehicle")
            .route_edge_index,
        1
    );
    assert!(released.events.iter().any(|event| {
        matches!(event, CoreEvent::VehicleChangedEdge(changed) if changed.vehicle == vehicle)
    }));
    assert!(released.events.iter().all(|event| {
        !matches!(event, CoreEvent::VehicleSignalStopProjectionApplied(projection) if projection.vehicle == vehicle)
    }));
}

#[test]
fn signal_control_none_traverses_without_signal_stop_projection() {
    let mut world = baseline_world();
    let profile = world
        .vehicle_profile_handle("passenger-car")
        .expect("profile");
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::active(
            "uncontrolled-vehicle",
            profile,
            "uncontrolled-route",
            0,
            EdgeProgress::try_new(100.0).unwrap(),
            Speed::try_new(1.0).unwrap(),
        ))
        .expect("uncontrolled vehicle must spawn");

    let result = world
        .step(TickInput::new(DELTA_MS))
        .expect("uncontrolled step");

    assert_eq!(
        world
            .vehicle(vehicle)
            .expect("uncontrolled vehicle")
            .route_edge_index,
        1
    );
    assert!(result.events.iter().any(|event| {
        matches!(event, CoreEvent::VehicleChangedEdge(changed) if changed.vehicle == vehicle)
    }));
    assert!(result.events.iter().all(|event| {
        !matches!(event, CoreEvent::VehicleSignalStopProjectionApplied(projection) if projection.vehicle == vehicle)
    }));
}
