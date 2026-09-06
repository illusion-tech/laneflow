//! #531 同机规模墙钟证据；无计数分配器，不设置跨机器耗时门禁。
//! `cargo test --release --locked -p laneflow-runtime --test cutover_scale_evidence -- --ignored --nocapture`

use laneflow_runtime as runtime_types;

#[path = "cutover_scale.rs"]
mod support;

use laneflow_runtime::{
    CutoverPreflightLimits, CutoverTransactionLimits, LfcaOriginBinding, MigrationPolicyKind,
    NetworkRevisionCutoverDescriptor, TickInput, deterministic_state_digest,
};
use std::sync::Arc;
use std::time::Instant;
use support::{revisions, source, world};

#[test]
#[ignore = "manual release cutover scale evidence"]
fn cutover_scale_evidence() {
    let roots = revisions();
    let mut cases = Vec::new();
    for count in [1_000, 4_000, 10_000] {
        for edges in [16, 256] {
            cases.push((count, edges, edges, "online"));
        }
    }
    cases.extend([
        (10_000, 256, 10_000, "online"),
        (10_000, 256, 256, "drain"),
        (10_000, 256, 256, "paused"),
    ]);
    for (count, edges, routes, mode) in cases {
        for sample in 0..3 {
            let mut world = world(&roots, count, edges, routes);
            let snapshot = world.capture_snapshot().expect("capture");
            let started = Instant::now();
            std::hint::black_box(deterministic_state_digest(&snapshot).expect("digest"));
            let digest_us = started.elapsed().as_micros();
            let descriptor = NetworkRevisionCutoverDescriptor::new(
                LfcaOriginBinding::from_canonical_origin(*roots.base.canonical_origin()),
                LfcaOriginBinding::from_canonical_origin(*roots.target.canonical_origin()),
                Some(roots.diff_binding),
                MigrationPolicyKind::CrossRevisionDirect,
                world.world_binding(),
            );
            let pause_started = Instant::now();
            let started = Instant::now();
            let mut transaction = world
                .prepare_cross_revision_cutover(
                    Arc::clone(&roots.target),
                    source(&roots.target),
                    &descriptor,
                    &roots.diff,
                    &CutoverPreflightLimits::new(16 * 1_024 * 1_024),
                    &CutoverTransactionLimits::default(),
                )
                .expect("prepare");
            let prepare_us = started.elapsed().as_micros();
            let mut pump_us = 0;
            // 两拍万车全量变化约 1.5 MiB，仍在默认 8 MiB 日志范围内。
            if mode != "paused" {
                for _ in 0..2 {
                    world.step(TickInput::new(100)).expect("step");
                    if mode == "online" {
                        let started = Instant::now();
                        transaction.pump(&mut world).expect("pump");
                        pump_us += started.elapsed().as_micros();
                    }
                }
            }
            let journal_bytes = world
                .migration_journal_stats()
                .expect("journal")
                .written_bytes;
            let started = Instant::now();
            let _commit = transaction.commit(&mut world).expect("commit");
            let commit_us = started.elapsed().as_micros();
            let paused_us = if mode == "paused" {
                pause_started.elapsed().as_micros()
            } else {
                0
            };
            assert_eq!(
                world.revision().canonical_origin(),
                roots.target.canonical_origin()
            );
            assert!(world.migration_journal_stats().is_none());
            println!(
                "cutover count={count} active={count} edges={edges} routes={routes} mode={mode} sample={sample} prepare_us={prepare_us} pump_us={pump_us} commit_us={commit_us} digest_us={digest_us} paused_us={paused_us} journal_bytes={journal_bytes}"
            );
        }
    }
}
