use std::{alloc::System, hint::black_box, sync::Mutex};

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

#[path = "support/presentation_scale.rs"]
mod scale_support;

use scale_support::{ONE_HUNDRED_THOUSAND, PresentationScaleFixture, TEN_THOUSAND};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;
static MEASUREMENT_LOCK: Mutex<()> = Mutex::new(());

#[test]
#[ignore = "global allocator measurement requires explicit serial execution"]
fn preallocated_10k_and_100k_post_update_is_zero_allocation() {
    let _measurement_guard = MEASUREMENT_LOCK.lock().expect("measurement lock");

    for count in [TEN_THOUSAND, ONE_HUNDRED_THOUSAND] {
        let mut fixture = PresentationScaleFixture::new(count);
        fixture.run_post_update();

        let region = Region::new(GLOBAL);
        black_box(&mut fixture).run_post_update();
        let stats = black_box(region.change());

        assert_zero_allocation(count, stats);
        fixture.assert_presented(count);
    }
}

fn assert_zero_allocation(count: usize, stats: Stats) {
    assert_eq!(stats.allocations, 0, "{count}: allocation count");
    assert_eq!(stats.reallocations, 0, "{count}: reallocation count");
    assert_eq!(stats.bytes_allocated, 0, "{count}: allocated bytes");
    assert_eq!(stats.bytes_reallocated, 0, "{count}: reallocated bytes");
}
