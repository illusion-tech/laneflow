use std::{alloc::System, hint::black_box, sync::Mutex};

use laneflow_spatial::FramePlacementToken;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

#[path = "support/pose_batch_scale.rs"]
mod scale_support;

use scale_support::{ONE_HUNDRED_THOUSAND, RuntimeFixture, TEN_THOUSAND};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;
static MEASUREMENT_LOCK: Mutex<()> = Mutex::new(());

#[test]
#[ignore = "global allocator measurement requires explicit serial execution"]
fn preallocated_10k_and_100k_pose_batches_are_zero_allocation() {
    let _measurement_guard = MEASUREMENT_LOCK.lock().expect("measurement lock");
    let fixture = RuntimeFixture::new();

    for count in [TEN_THOUSAND, ONE_HUNDRED_THOUSAND] {
        let inputs = fixture.inputs(count);
        let mut output = fixture.output(count);
        let mut scratch = fixture.scratch(count);
        fixture
            .spatial
            .extract_pose_batch(
                &fixture.parking,
                FramePlacementToken::new(1),
                &inputs,
                &mut output,
                &mut scratch,
            )
            .expect("warm-up extraction");

        let region = Region::new(GLOBAL);
        fixture
            .spatial
            .extract_pose_batch(
                &fixture.parking,
                FramePlacementToken::new(2),
                black_box(&inputs),
                black_box(&mut output),
                black_box(&mut scratch),
            )
            .expect("measured extraction");
        black_box(output.records());
        let stats = black_box(region.change());

        assert_zero_allocation(count, stats);
        assert_eq!(output.len(), count);
        assert!(output.capacity() >= count);
        assert!(scratch.capacity() >= count);
        assert!(scratch.is_empty());
    }
}

fn assert_zero_allocation(count: usize, stats: Stats) {
    assert_eq!(stats.allocations, 0, "{count}: allocation count");
    assert_eq!(stats.reallocations, 0, "{count}: reallocation count");
    assert_eq!(stats.bytes_allocated, 0, "{count}: allocated bytes");
    assert_eq!(stats.bytes_reallocated, 0, "{count}: reallocated bytes");
}
