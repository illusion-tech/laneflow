//! #531 分配证据，与未插桩的墙钟二进制分开。
//! `cargo test --release --locked -p laneflow-runtime --test cutover_scale_allocation -- --ignored --nocapture`

use laneflow_runtime as runtime_types;

#[path = "cutover_scale.rs"]
mod support;

use laneflow_runtime::{
    CutoverPreflightLimits, CutoverTransactionLimits, LfcaOriginBinding, MigrationPolicyKind,
    NetworkRevisionCutoverDescriptor, deterministic_state_digest,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};
use std::alloc::System;
use std::sync::Arc;
use support::{revisions, source, world};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn report(count: u32, edges: u32, routes: u32, phase: &str, stats: Stats) {
    println!(
        "allocation count={count} edges={edges} routes={routes} phase={phase} allocs={} reallocs={} allocated_bytes={} deallocated_bytes={} reallocated_bytes={}",
        stats.allocations,
        stats.reallocations,
        stats.bytes_allocated,
        stats.bytes_deallocated,
        stats.bytes_reallocated,
    );
}

#[test]
#[ignore = "manual release cutover allocation evidence"]
fn cutover_scale_allocation() {
    let roots = revisions();
    for (count, edges, routes) in [
        (1_000, 256, 256),
        (10_000, 16, 16),
        (10_000, 256, 256),
        (10_000, 256, 10_000),
    ] {
        let mut world = world(&roots, count, edges, routes);
        let captured = world.capture_snapshot().unwrap();
        let region = Region::new(GLOBAL);
        std::hint::black_box(deterministic_state_digest(&captured).unwrap());
        let digest = region.change();
        let descriptor = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(*roots.base.canonical_origin()),
            LfcaOriginBinding::from_canonical_origin(*roots.target.canonical_origin()),
            Some(roots.diff_binding),
            MigrationPolicyKind::CrossRevisionDirect,
            world.world_binding(),
        );
        let region = Region::new(GLOBAL);
        let transaction = world
            .prepare_cross_revision_cutover(
                Arc::clone(&roots.target),
                source(&roots.target),
                &descriptor,
                &roots.diff,
                &CutoverPreflightLimits::new(16 * 1_024 * 1_024),
                &CutoverTransactionLimits::default(),
            )
            .unwrap();
        let prepare = region.change();
        let region = Region::new(GLOBAL);
        let _commit = transaction.commit(&mut world).unwrap();
        let commit = region.change();
        report(count, edges, routes, "digest", digest);
        report(count, edges, routes, "prepare", prepare);
        report(count, edges, routes, "commit", commit);
    }
}
