use std::{alloc::System, hint::black_box, sync::Mutex};

use laneflow_core::TickInput;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[path = "support/vehicle_following_scenarios.rs"]
#[allow(dead_code, reason = "shared benchmark helper exposes other scenarios")]
mod scenarios;

use scenarios::{FIXED_DELTA_TIME_MS, VEHICLE_COUNT, dense_platoon_world};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;
static MEASUREMENT_LOCK: Mutex<()> = Mutex::new(());

#[test]
#[ignore = "global allocator measurement requires explicit serial execution"]
fn warm_ten_thousand_vehicle_min_gap_step_is_zero_allocation() {
    let _measurement_guard = MEASUREMENT_LOCK.lock().expect("measurement lock");
    let mut world = dense_platoon_world(VEHICLE_COUNT);
    let warm_up = world
        .step(TickInput::new(FIXED_DELTA_TIME_MS))
        .expect("vehicle-following warm-up step");
    assert!(warm_up.events.is_empty());

    let region = Region::new(GLOBAL);
    let result = world
        .step(TickInput::new(FIXED_DELTA_TIME_MS))
        .expect("vehicle-following measured step");
    black_box(&result);
    let stats = black_box(region.change());

    assert!(result.events.is_empty());
    assert_eq!(stats.allocations, 0, "allocation count");
    assert_eq!(stats.reallocations, 0, "reallocation count");
    assert_eq!(stats.bytes_allocated, 0, "allocated bytes");
    assert_eq!(stats.bytes_reallocated, 0, "reallocated bytes");
}
