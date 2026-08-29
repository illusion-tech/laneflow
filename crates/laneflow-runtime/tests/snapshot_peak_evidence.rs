//! #512 published fresh restore 增量堆峰值证据。
//!
//! 单测试独立 integration binary 遵循 DHAT heap-usage testing 的隔离要求：目标根、
//! source、配置和 LFRS 输入在 profiler 启动前准备；profiler 生命周期只包围一次
//! `restore_lfrs`，因此 `max_bytes` 是恢复调用新增堆块的实际高水位，不把原有共享根
//! 或输入制品常驻内存重复记入。数值为同机描述性初值，不是产品 Pass 阈值。

use std::hint::black_box;

use laneflow_runtime::{TickInput, deterministic_state_digest, encode_lfrs, restore_lfrs};

#[path = "support/snapshot_evidence.rs"]
mod snapshot_evidence;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[test]
fn published_restore_peak_heap_baseline() {
    let revision = snapshot_evidence::build();
    let mut world = snapshot_evidence::install_corridor_world(&revision);
    for _ in 0..snapshot_evidence::SNAPSHOT_WARMUP_TICKS {
        world
            .step(TickInput::new(snapshot_evidence::DELTA_MS))
            .expect("warmup step");
    }
    let snapshot = world.capture_snapshot().expect("capture");
    let expected_digest = deterministic_state_digest(&snapshot).expect("digest");
    let bytes = encode_lfrs(&snapshot);
    let target_revision = world.revision();
    let target_source = world.committed_source().clone();
    let target_config = world.config();
    drop(world);

    let profiler = dhat::Profiler::builder().testing().build();
    let restored = black_box(
        restore_lfrs(
            &bytes,
            target_revision,
            target_source,
            target_config,
            snapshot_evidence::limits(),
        )
        .expect("published restore"),
    );
    let stats = dhat::HeapStats::get();
    drop(profiler);

    assert!(stats.total_blocks > 0);
    assert!(stats.total_bytes >= stats.max_bytes as u64);
    assert!(stats.max_bytes >= stats.curr_bytes);
    assert_eq!(
        deterministic_state_digest(&restored.world().capture_snapshot().expect("capture"))
            .expect("digest"),
        expected_digest
    );
    println!(
        "snapshot-peak-evidence workload=signalized-corridor-v1 routes={} vehicles={} lfrs_exact_bytes={} restore_total_blocks={} restore_total_bytes={} restore_peak_increment_bytes={} restore_peak_blocks={} restore_return_live_bytes={} restore_return_live_blocks={} classification=descriptive-initial-not-product-pass",
        snapshot.routes().len(),
        snapshot.vehicles().len(),
        bytes.len(),
        stats.total_blocks,
        stats.total_bytes,
        stats.max_bytes,
        stats.max_blocks,
        stats.curr_bytes,
        stats.curr_blocks,
    );
}
