//! #583 独立分配计数二进制；不把本二进制耗时写入墙钟报告。

use laneflow_runtime as runtime_types;
#[path = "runtime_profile.rs"]
mod support;

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::alloc::System;
use support::{CASES, Fixtures};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

#[test]
#[ignore = "manual release current Runtime allocation profile"]
fn runtime_profile_allocation() {
    let fixtures = Fixtures::new();
    for scene in CASES {
        let mut harness = fixtures.world(scene);
        harness.validate();
        let region = Region::new(GLOBAL);
        for _ in 0..harness.steps {
            harness.step();
        }
        let stats = region.change();
        harness.validate();
        println!(
            "profile-allocation scene={} steps={} allocations={} reallocations={} allocated_bytes={} deallocated_bytes={} reallocated_bytes={} journal_bytes={} digest={}",
            scene.name(),
            harness.steps,
            stats.allocations,
            stats.reallocations,
            stats.bytes_allocated,
            stats.bytes_deallocated,
            stats.bytes_reallocated,
            harness.journal_bytes(),
            harness.digest(),
        );
    }
}
