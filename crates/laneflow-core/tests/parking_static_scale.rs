use std::{alloc::System, hint::black_box};

use laneflow_core::{CoreWorld, TickInput, VehicleState};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

#[path = "support/signals_validation_scenarios.rs"]
#[allow(
    dead_code,
    reason = "shared benchmark helper exposes Signals modes unused by this binary"
)]
mod scenarios;

use scenarios::{
    SIGNAL_FIXED_DELTA_TIME_MS, SIGNAL_STEP_COUNT, SIGNAL_VEHICLE_COUNT, SignalScenarioMode,
    signal_scenario_with_parking,
};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn snapshot(world: &CoreWorld) -> Vec<VehicleState> {
    world.vehicles().cloned().collect()
}

fn measured_step(world: &mut CoreWorld) -> Stats {
    let region = Region::new(GLOBAL);
    let result = world
        .step(TickInput::new(SIGNAL_FIXED_DELTA_TIME_MS))
        .expect("measured step");
    assert!(result.events.is_empty());
    black_box(result);
    black_box(region.change())
}

fn assert_zero_allocation(label: &str, stats: Stats) {
    assert_eq!(stats.allocations, 0, "{label}: allocation count");
    assert_eq!(stats.reallocations, 0, "{label}: reallocation count");
    assert_eq!(stats.bytes_allocated, 0, "{label}: allocated bytes");
    assert_eq!(stats.bytes_reallocated, 0, "{label}: reallocated bytes");
}

#[test]
fn ten_thousand_vacant_spaces_are_step_inert_and_allocation_free() {
    let mut empty =
        signal_scenario_with_parking(SIGNAL_VEHICLE_COUNT, SignalScenarioMode::NoSignals, 0);
    let mut all_vacant = signal_scenario_with_parking(
        SIGNAL_VEHICLE_COUNT,
        SignalScenarioMode::NoSignals,
        SIGNAL_VEHICLE_COUNT,
    );
    assert_eq!(empty.world.parking().spaces().count(), 0);
    assert_eq!(all_vacant.world.parking().spaces().count(), 10_000);

    for _ in 0..SIGNAL_STEP_COUNT {
        let empty_result = empty
            .world
            .step(TickInput::new(SIGNAL_FIXED_DELTA_TIME_MS))
            .expect("empty-registry step");
        let vacant_result = all_vacant
            .world
            .step(TickInput::new(SIGNAL_FIXED_DELTA_TIME_MS))
            .expect("all-vacant step");
        assert_eq!(empty_result, vacant_result);
        assert_eq!(snapshot(&empty.world), snapshot(&all_vacant.world));
    }

    assert_zero_allocation("empty registry", measured_step(&mut empty.world));
    assert_zero_allocation(
        "10k all-vacant registry",
        measured_step(&mut all_vacant.world),
    );
}
