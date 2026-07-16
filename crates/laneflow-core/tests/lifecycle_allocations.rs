use std::{alloc::System, hint::black_box};

use laneflow_core::TickInput;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

#[path = "support/command_validation_scenarios.rs"]
#[allow(
    dead_code,
    reason = "shared benchmark helper exposes scenarios unused by this binary"
)]
mod command_scenarios;

use command_scenarios::{
    FIXED_COMMAND_COUNT, command_scenario, run_in_use_route_failure_batch,
    run_overlap_failure_batch, take_safe_input, warm_command_scenario,
};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

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
fn lifecycle_allocation_invariants_are_scale_independent() {
    let mut small_overlap = command_scenario(128, 8);
    warm_command_scenario(&mut small_overlap);
    let (_, small_overlap_stats) =
        measure(|| run_overlap_failure_batch(&mut small_overlap, FIXED_COMMAND_COUNT));

    let mut large_overlap = command_scenario(10_000, 512);
    warm_command_scenario(&mut large_overlap);
    let (_, large_overlap_stats) =
        measure(|| run_overlap_failure_batch(&mut large_overlap, FIXED_COMMAND_COUNT));

    assert_eq!(
        large_overlap_stats.allocations, small_overlap_stats.allocations,
        "owned overlap error allocation count must not scale with V or route length"
    );
    assert_eq!(
        large_overlap_stats.reallocations, small_overlap_stats.reallocations,
        "owned overlap error reallocation count must not scale with V or route length"
    );
    assert_eq!(
        large_overlap_stats.bytes_allocated, small_overlap_stats.bytes_allocated,
        "owned overlap error bytes must not scale with V or route length"
    );
    assert_eq!(small_overlap_stats.allocations, 200);
    assert_eq!(small_overlap_stats.reallocations, 0);
    assert_eq!(small_overlap_stats.bytes_allocated, 2_000);

    let mut scenario = command_scenario(10_000, 64);
    warm_command_scenario(&mut scenario);

    let (_, in_use_stats) =
        measure(|| run_in_use_route_failure_batch(&mut scenario, FIXED_COMMAND_COUNT));
    assert_zero_allocation("warm in-use route failure", in_use_stats);

    let input = take_safe_input(&mut scenario);
    let (spawned, spawn_stats) = measure(|| scenario.world.spawn_vehicle(input).expect("spawn"));
    assert_eq!(
        spawn_stats.allocations, 1,
        "successful spawn owned ID clone"
    );
    assert_eq!(
        spawn_stats.reallocations, 0,
        "successful spawn reallocation"
    );
    assert_eq!(spawn_stats.bytes_allocated, 9, "successful spawn ID bytes");

    let (_, despawn_stats) = measure(|| {
        scenario
            .world
            .despawn_vehicle(spawned)
            .expect("warm despawn")
    });
    assert_zero_allocation("warm despawn", despawn_stats);

    scenario
        .world
        .step(TickInput::new(16))
        .expect("step warm-up");
    let (_, step_stats) = measure(|| {
        scenario
            .world
            .step(TickInput::new(16))
            .expect("no-event steady step")
    });
    assert_zero_allocation("no-event steady step", step_stats);
}
