use std::{alloc::System, hint::black_box, sync::Mutex};

use laneflow_core::TickInput;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

#[path = "support/vehicle_following_scenarios.rs"]
#[allow(dead_code, reason = "shared benchmark helper exposes other scenarios")]
mod scenarios;

use scenarios::{FIXED_DELTA_TIME_MS, VEHICLE_COUNT, speed_limit_transition_world};

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

#[test]
#[ignore = "global allocator measurement requires explicit serial execution"]
fn ten_thousand_vehicle_speed_limit_steady_step_is_zero_allocation() {
    let _measurement_guard = MEASUREMENT_LOCK.lock().expect("measurement lock");
    let mut world = speed_limit_transition_world(VEHICLE_COUNT);
    world
        .step(TickInput::new(FIXED_DELTA_TIME_MS))
        .expect("speed-limit warm-up step");

    let (result, stats) = measure(|| world.step(TickInput::new(FIXED_DELTA_TIME_MS)));

    assert!(result.expect("speed-limit steady step").events.is_empty());
    assert_eq!(stats.allocations, 0, "allocation count");
    assert_eq!(stats.reallocations, 0, "reallocation count");
    assert_eq!(stats.bytes_allocated, 0, "allocated bytes");
    assert_eq!(stats.bytes_reallocated, 0, "reallocated bytes");
}
