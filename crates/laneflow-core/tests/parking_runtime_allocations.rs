use std::{alloc::System, hint::black_box, sync::Mutex};

use laneflow_core::{ParkingCommandEffect, TickInput};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

#[path = "support/parking_runtime_scenarios.rs"]
#[allow(dead_code, reason = "shared benchmark helper exposes scale scenarios")]
mod parking_scenarios;

use parking_scenarios::single_parking_world;

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;
static MEASUREMENT_LOCK: Mutex<()> = Mutex::new(());

fn measure<T>(operation: impl FnOnce() -> T) -> (T, Stats) {
    let region = Region::new(GLOBAL);
    let output = operation();
    black_box(&output);
    let change = black_box(region.change());
    (output, change)
}

fn assert_zero_allocation(label: &str, stats: Stats) {
    assert_eq!(stats.allocations, 0, "{label}: allocation count");
    assert_eq!(stats.reallocations, 0, "{label}: reallocation count");
    assert_eq!(stats.bytes_allocated, 0, "{label}: allocated bytes");
    assert_eq!(stats.bytes_reallocated, 0, "{label}: reallocated bytes");
}

#[test]
fn parking_snapshot_and_warm_commands_are_zero_allocation() {
    let _measurement_guard = MEASUREMENT_LOCK.lock().expect("measurement lock");
    let (mut world, vehicle, space) = single_parking_world(0.0);

    let (_, snapshot_stats) = measure(|| {
        let snapshot = world.parking_snapshot();
        black_box(snapshot.counts());
        black_box(snapshot.space_state(space));
        black_box(snapshot.vehicle_state(vehicle));
        black_box(snapshot.space_states().count());
    });
    assert_zero_allocation("borrowed parking snapshot", snapshot_stats);

    let (record, reserve_stats) = measure(|| world.reserve_parking_space(vehicle, space));
    assert_eq!(
        record.expect("reserve").effect,
        ParkingCommandEffect::Applied
    );
    assert_zero_allocation("warm reserve", reserve_stats);

    let (record, cancel_stats) = measure(|| world.cancel_parking_reservation(vehicle, space));
    assert_eq!(
        record.expect("cancel").effect,
        ParkingCommandEffect::Applied
    );
    assert_zero_allocation("warm cancel", cancel_stats);
}

#[test]
fn parking_commit_and_occupied_only_steady_step_are_zero_allocation() {
    let _measurement_guard = MEASUREMENT_LOCK.lock().expect("measurement lock");
    let (mut world, vehicle, space) = single_parking_world(20.0);
    world
        .reserve_parking_space(vehicle, space)
        .expect("arrived reservation");
    world
        .step(TickInput::new(20))
        .expect("arrived scratch warm-up");
    let (result, arrived_step_stats) = measure(|| world.step(TickInput::new(20)));
    assert!(result.expect("arrived steady step").events.is_empty());
    assert_zero_allocation("arrived sparse scratch steady step", arrived_step_stats);

    let (record, commit_stats) = measure(|| world.commit_parking(vehicle, space));
    assert_eq!(
        record.expect("commit").effect,
        ParkingCommandEffect::Applied
    );
    assert_zero_allocation("warm commit", commit_stats);

    world
        .step(TickInput::new(20))
        .expect("occupied-only warm-up step");
    let (result, step_stats) = measure(|| world.step(TickInput::new(20)));
    assert!(result.expect("occupied-only steady step").events.is_empty());
    assert_zero_allocation("occupied-only steady step", step_stats);
}

#[test]
fn reserved_approach_steady_step_is_zero_allocation_after_warm_up() {
    let _measurement_guard = MEASUREMENT_LOCK.lock().expect("measurement lock");
    let (mut world, vehicle, space) = single_parking_world(0.0);
    world
        .reserve_parking_space(vehicle, space)
        .expect("approaching reservation");
    world
        .step(TickInput::new(20))
        .expect("reserved warm-up step");

    let (result, step_stats) = measure(|| world.step(TickInput::new(20)));
    assert!(result.expect("reserved steady step").events.is_empty());
    assert_zero_allocation("reserved approach steady step", step_stats);
}

#[test]
fn parking_event_step_allocates_only_the_result_event_buffer() {
    let _measurement_guard = MEASUREMENT_LOCK.lock().expect("measurement lock");
    let (mut world, vehicle, space) = single_parking_world(19.0);
    world
        .reserve_parking_space(vehicle, space)
        .expect("approaching reservation");
    world
        .step(TickInput::new(20))
        .expect("warm reusable Parking scratch");

    for _ in 0..200 {
        let before = world.clone();
        let probe = world.step(TickInput::new(20)).expect("approach probe step");
        if probe.events.is_empty() {
            continue;
        }

        world = before;
        let (result, stats) = measure(|| world.step(TickInput::new(20)));
        let result = result.expect("replayed event step");
        assert_eq!(result.events, probe.events, "event tick replay parity");
        assert_eq!(stats.allocations, 1, "event result buffer allocation");
        assert_eq!(stats.reallocations, 0, "event result buffer growth");
        assert!(stats.bytes_allocated > 0, "event result buffer bytes");
        return;
    }

    panic!("Parking approach must eventually emit a projection or arrival event");
}
