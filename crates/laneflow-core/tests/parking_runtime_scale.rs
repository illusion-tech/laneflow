use laneflow_core::TickInput;

#[path = "support/parking_runtime_scenarios.rs"]
#[allow(
    dead_code,
    reason = "shared parking helper exposes allocation scenarios"
)]
mod parking_scenarios;

use parking_scenarios::{
    FIXED_PARKING_COMMAND_COUNT, occupied_parking_world, parking_command_scenario,
    run_reserve_cancel_batch,
};
#[path = "support/signals_validation_scenarios.rs"]
#[allow(dead_code, reason = "shared helper also exposes signal scenarios")]
mod signal_scenarios;

use signal_scenarios::reserved_parking_scenario;

#[test]
fn ten_thousand_vehicle_parking_runtime_smoke_preserves_counts() {
    let mut commands = parking_command_scenario(10_000, FIXED_PARKING_COMMAND_COUNT);
    assert_eq!(
        run_reserve_cancel_batch(&mut commands),
        FIXED_PARKING_COMMAND_COUNT * 2
    );
    let counts = commands.world.parking_snapshot().counts();
    assert_eq!(counts.capacity, FIXED_PARKING_COMMAND_COUNT);
    assert_eq!(counts.vacant, FIXED_PARKING_COMMAND_COUNT);
    assert_eq!(counts.reserved, 0);
    assert_eq!(counts.occupied, 0);
    assert_eq!(commands.world.vehicles().count(), 10_000);

    let mut occupied = occupied_parking_world(10_000, 20);
    let counts = occupied.parking_snapshot().counts();
    assert_eq!(counts.capacity, 10_000);
    assert_eq!(counts.vacant, 0);
    assert_eq!(counts.reserved, 0);
    assert_eq!(counts.occupied, 10_000);
    assert!(
        occupied
            .step(TickInput::new(20))
            .expect("occupied-only 10k step")
            .events
            .is_empty()
    );
}

#[test]
fn ten_thousand_vehicle_reserved_ratios_step_without_full_state_fallback() {
    for percent in [1_usize, 10, 100] {
        let mut scenario = reserved_parking_scenario(10_000, percent);
        assert_eq!(
            scenario.world.parking_snapshot().counts().reserved,
            10_000 * percent / 100
        );
        scenario
            .world
            .step(TickInput::new(16))
            .expect("reserved ratio step");
        assert_eq!(
            scenario.world.parking_snapshot().counts().reserved,
            10_000 * percent / 100
        );
    }
}

#[test]
#[ignore = "100k Parking runtime scaling is an explicit G3 validation"]
fn hundred_thousand_vehicle_parking_runtime_smoke_preserves_counts() {
    let mut commands = parking_command_scenario(100_000, FIXED_PARKING_COMMAND_COUNT);
    assert_eq!(
        run_reserve_cancel_batch(&mut commands),
        FIXED_PARKING_COMMAND_COUNT * 2
    );
    assert_eq!(commands.world.vehicles().count(), 100_000);

    let mut occupied = occupied_parking_world(100_000, 20);
    assert_eq!(occupied.parking_snapshot().counts().occupied, 100_000);
    assert!(
        occupied
            .step(TickInput::new(20))
            .expect("occupied-only 100k step")
            .events
            .is_empty()
    );

    let mut reserved = reserved_parking_scenario(100_000, 100);
    reserved
        .world
        .step(TickInput::new(16))
        .expect("reserved-only 100k step");
    assert_eq!(reserved.world.parking_snapshot().counts().reserved, 100_000);
}
