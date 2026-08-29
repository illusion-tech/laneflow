//! #512 快照侧预算基线（corridor 夹具，同机描述性）——分配账本面。
//!
//! 本二进制量化捕获、离线编码与 published fresh restore 的分配账本，同时硬断言
//! 保存前后稳态 tick 账本相等、保存点恢复后同 tick 序列的确定性摘要相等。
//! restore 的精确增量堆峰值由独立 `snapshot_peak_evidence` 二进制以 DHAT 测量；
//! 墙钟与并发编码干扰在未插桩的 `snapshot_wall_clock_evidence` 中单独测量，避免
//! 不同仪器互相污染。

use std::{alloc::System, hint::black_box, sync::Arc};

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CapturedSnapshot, CommittedNetworkSource, RestoredSnapshot, TickInput, TrafficWorld,
    WorldConfig, deterministic_state_digest, encode_lfrs, restore_lfrs,
};
use laneflow_static_network::SharedNetworkRevision;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[path = "support/snapshot_evidence.rs"]
mod snapshot_evidence;

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const STEADY_MEASURED_TICKS: u32 = 32;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Ledger {
    allocations: usize,
    reallocations: usize,
    allocated_bytes: usize,
    deallocated_bytes: usize,
    reallocated_bytes: isize,
    live_delta_bytes: i128,
}

impl Ledger {
    fn from_change(stats: stats_alloc::Stats) -> Self {
        Self {
            allocations: stats.allocations,
            reallocations: stats.reallocations,
            allocated_bytes: stats.bytes_allocated,
            deallocated_bytes: stats.bytes_deallocated,
            reallocated_bytes: stats.bytes_reallocated,
            live_delta_bytes: stats.bytes_allocated as i128 + stats.bytes_reallocated as i128
                - stats.bytes_deallocated as i128,
        }
    }
}

fn steady_tick_ledger(world: &mut TrafficWorld) -> Ledger {
    let region = Region::new(GLOBAL);
    for _ in 0..STEADY_MEASURED_TICKS {
        world
            .step(TickInput::new(snapshot_evidence::DELTA_MS))
            .expect("steady step");
    }
    Ledger::from_change(region.change())
}

fn measure_capture(world: &TrafficWorld) -> (CapturedSnapshot, Ledger) {
    let region = Region::new(GLOBAL);
    let snapshot = black_box(world.capture_snapshot().expect("capture"));
    let ledger = Ledger::from_change(region.change());
    (snapshot, ledger)
}

fn measure_encode(snapshot: &CapturedSnapshot) -> (Vec<u8>, Ledger) {
    let region = Region::new(GLOBAL);
    let bytes = black_box(encode_lfrs(snapshot));
    let ledger = Ledger::from_change(region.change());
    (bytes, ledger)
}

fn measure_restore(
    bytes: &[u8],
    target_revision: &Arc<SharedNetworkRevision>,
    target_source: &CommittedNetworkSource,
    target_config: WorldConfig,
) -> (RestoredSnapshot, Ledger) {
    let revision = Arc::clone(target_revision);
    let source = target_source.clone();
    let region = Region::new(GLOBAL);
    let restored = black_box(
        restore_lfrs(
            bytes,
            revision,
            source,
            target_config,
            snapshot_evidence::limits(),
        )
        .expect("published restore"),
    );
    let ledger = Ledger::from_change(region.change());
    (restored, ledger)
}

#[test]
fn snapshot_side_budget_baseline() {
    let checked = check_canonical_network_input(snapshot_evidence::CORRIDOR, FormatLimits::HARD)
        .expect("checked");
    let artifact_digest = checked.canonical_artifact_digest();
    let network_revision = checked.network_revision();
    let revision = snapshot_evidence::build();
    let mut world = snapshot_evidence::install_corridor_world(&revision);
    for _ in 0..snapshot_evidence::SNAPSHOT_WARMUP_TICKS {
        world
            .step(TickInput::new(snapshot_evidence::DELTA_MS))
            .expect("warmup step");
    }

    // 每条账本丢一次 warmup，随后双样本必须精确相等。
    drop(measure_capture(&world));
    let (snapshot, capture_one) = measure_capture(&world);
    let (second_snapshot, capture_two) = measure_capture(&world);
    assert_eq!(capture_one, capture_two, "capture ledger drift");
    assert_eq!(snapshot, second_snapshot, "capture logical state drift");
    drop(second_snapshot);

    drop(measure_encode(&snapshot));
    let (bytes, encode_one) = measure_encode(&snapshot);
    let (second_bytes, encode_two) = measure_encode(&snapshot);
    assert_eq!(encode_one, encode_two, "encode ledger drift");
    assert_eq!(bytes, second_bytes, "LFRS bytes drift");
    drop(second_bytes);

    let target_revision = world.revision();
    let target_source = world.committed_source().clone();
    let target_config = world.config();
    drop(world);
    drop(measure_restore(
        &bytes,
        &target_revision,
        &target_source,
        target_config,
    ));
    let (restored, restore_one) =
        measure_restore(&bytes, &target_revision, &target_source, target_config);
    let restored_digest =
        deterministic_state_digest(&restored.world().capture_snapshot().expect("capture"))
            .expect("digest");
    drop(restored);
    let (second_restored, restore_two) =
        measure_restore(&bytes, &target_revision, &target_source, target_config);
    assert_eq!(restore_one, restore_two, "restore ledger drift");
    assert_eq!(
        deterministic_state_digest(&snapshot).expect("digest"),
        restored_digest,
        "restore logical digest"
    );
    assert_eq!(
        restored_digest,
        deterministic_state_digest(&second_restored.world().capture_snapshot().expect("capture"))
            .expect("digest"),
        "second restore logical digest"
    );
    drop(second_restored);

    // 保存干扰硬约束：捕获与离线编码不改变后续稳态 tick 的分配形态或结果。
    let mut interference_world = snapshot_evidence::install_corridor_world(&revision);
    for _ in 0..snapshot_evidence::SNAPSHOT_WARMUP_TICKS {
        interference_world
            .step(TickInput::new(snapshot_evidence::DELTA_MS))
            .expect("interference warmup step");
    }
    black_box(steady_tick_ledger(&mut interference_world));
    let steady_before = steady_tick_ledger(&mut interference_world);
    let save_point = interference_world.capture_snapshot().expect("capture");
    let save_bytes = encode_lfrs(&save_point);
    black_box(save_bytes.len());
    let steady_after = steady_tick_ledger(&mut interference_world);
    assert_eq!(
        steady_before, steady_after,
        "snapshot save must not change steady tick allocation shape"
    );
    let expected_after_save =
        deterministic_state_digest(&interference_world.capture_snapshot().expect("capture"))
            .expect("digest");
    let oracle_revision = interference_world.revision();
    let oracle_source = interference_world.committed_source().clone();
    let oracle_config = interference_world.config();
    drop(interference_world);
    let mut oracle = restore_lfrs(
        &save_bytes,
        oracle_revision,
        oracle_source,
        oracle_config,
        snapshot_evidence::limits(),
    )
    .expect("restore save point")
    .into_world();
    for _ in 0..STEADY_MEASURED_TICKS {
        oracle
            .step(TickInput::new(snapshot_evidence::DELTA_MS))
            .expect("oracle step");
    }
    assert_eq!(
        expected_after_save,
        deterministic_state_digest(&oracle.capture_snapshot().expect("capture")).expect("digest"),
        "save activity must not alter committed tick semantics"
    );

    println!(
        "snapshot-budget-evidence workload=signalized-corridor-v1 lfca_exact={} artifact_digest={:x} network_revision={:x} routes={} vehicles={} tick={} command_cursor={} lfrs_exact_bytes={} steady_ticks={} steady_before={steady_before:?} steady_after={steady_after:?} capture={capture_one:?} encode={encode_one:?} restore={restore_one:?} classification=descriptive-initial-not-product-pass",
        snapshot_evidence::CORRIDOR.len(),
        artifact_digest,
        network_revision.into_digest(),
        snapshot.routes().len(),
        snapshot.vehicles().len(),
        snapshot.tick(),
        snapshot.command_cursor(),
        bytes.len(),
        STEADY_MEASURED_TICKS,
    );
}
