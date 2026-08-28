//! #512 快照侧预算基线（corridor 夹具，同机描述性）——未插桩墙钟面。
//!
//! 分别测量固定步进边界 capture 的主线程停顿、可后台执行的 LFRS encode、
//! published 绑定认证 + verifier/lowering/fresh restore 的整次同步 load，以及后台
//! encode 竞争期间的稳态 tick 中位数。数字只作同机 release 描述性初值，硬断言
//! 只覆盖逻辑摘要、工作负载和恢复行为；CI 只编译，不把 dev/共享 runner 墙钟当门槛。

use std::{
    hint::black_box,
    sync::{Arc, Barrier},
    time::Instant,
};

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CapturedSnapshot, CommittedNetworkSource, TickInput, TrafficWorld, WorldConfig,
    deterministic_state_digest, encode_lfrs, restore_lfrs,
};
use laneflow_static_contract::Sha256Digest;
use laneflow_static_network::SharedNetworkRevision;

#[path = "support/snapshot_evidence.rs"]
mod snapshot_evidence;

const WALL_WARMUP: usize = 3;
const WALL_SAMPLES: usize = 31;
const TICK_SAMPLES: usize = 128;
const BACKGROUND_ENCODES: usize = 4_096;

fn median(values: &mut [u128]) -> u128 {
    values.sort_unstable();
    let len = values.len();
    if len.is_multiple_of(2) {
        (values[len / 2 - 1] + values[len / 2]) / 2
    } else {
        values[len / 2]
    }
}

fn capture_clocks(world: &TrafficWorld) -> Vec<u128> {
    let mut clocks = Vec::with_capacity(WALL_SAMPLES);
    for index in 0..(WALL_WARMUP + WALL_SAMPLES) {
        let started = Instant::now();
        let snapshot = black_box(world.capture_snapshot());
        let elapsed = started.elapsed().as_nanos();
        drop(snapshot);
        if index >= WALL_WARMUP {
            clocks.push(elapsed);
        }
    }
    clocks
}

fn encode_clocks(snapshot: &CapturedSnapshot) -> Vec<u128> {
    let mut clocks = Vec::with_capacity(WALL_SAMPLES);
    for index in 0..(WALL_WARMUP + WALL_SAMPLES) {
        let started = Instant::now();
        let bytes = black_box(encode_lfrs(snapshot));
        let elapsed = started.elapsed().as_nanos();
        drop(bytes);
        if index >= WALL_WARMUP {
            clocks.push(elapsed);
        }
    }
    clocks
}

fn restore_clocks(
    bytes: &[u8],
    target_revision: &Arc<SharedNetworkRevision>,
    target_source: &CommittedNetworkSource,
    target_config: WorldConfig,
    expected_digest: Sha256Digest,
) -> Vec<u128> {
    let mut clocks = Vec::with_capacity(WALL_SAMPLES);
    for index in 0..(WALL_WARMUP + WALL_SAMPLES) {
        let revision = Arc::clone(target_revision);
        let source = target_source.clone();
        let started = Instant::now();
        let restored = restore_lfrs(
            bytes,
            revision,
            source,
            target_config,
            snapshot_evidence::limits(),
        )
        .expect("published restore");
        let elapsed = started.elapsed().as_nanos();
        assert_eq!(
            deterministic_state_digest(&restored.world().capture_snapshot()),
            expected_digest
        );
        drop(restored);
        if index >= WALL_WARMUP {
            clocks.push(elapsed);
        }
    }
    clocks
}

fn tick_clocks(world: &mut TrafficWorld) -> Vec<u128> {
    let mut clocks = Vec::with_capacity(TICK_SAMPLES);
    for _ in 0..TICK_SAMPLES {
        let started = Instant::now();
        world
            .step(TickInput::new(snapshot_evidence::DELTA_MS))
            .expect("tick");
        clocks.push(started.elapsed().as_nanos());
    }
    clocks
}

#[test]
#[ignore = "manual release wall-clock evidence; CI 不当墙钟基线"]
fn snapshot_side_wall_clock_baseline() {
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
    snapshot_evidence::assert_two_lane_poses(&world);
    let snapshot = world.capture_snapshot();
    let bytes = encode_lfrs(&snapshot);
    let expected_digest = deterministic_state_digest(&snapshot);

    let capture_ns = median(&mut capture_clocks(&world));
    let encode_ns = median(&mut encode_clocks(&snapshot));
    let target_revision = world.revision();
    let target_source = world.committed_source().clone();
    let target_config = world.config();
    drop(world);
    let restore_ns = median(&mut restore_clocks(
        &bytes,
        &target_revision,
        &target_source,
        target_config,
        expected_digest,
    ));

    let mut baseline_world = restore_lfrs(
        &bytes,
        Arc::clone(&target_revision),
        target_source.clone(),
        target_config,
        snapshot_evidence::limits(),
    )
    .expect("baseline restore")
    .into_world();
    let baseline_tick_ns = median(&mut tick_clocks(&mut baseline_world));
    let expected_after_ticks = deterministic_state_digest(&baseline_world.capture_snapshot());
    drop(baseline_world);

    let mut contended_world = restore_lfrs(
        &bytes,
        target_revision,
        target_source,
        target_config,
        snapshot_evidence::limits(),
    )
    .expect("contended restore")
    .into_world();
    let start = Arc::new(Barrier::new(2));
    let mut contended_clocks = Vec::new();
    std::thread::scope(|scope| {
        let worker_start = Arc::clone(&start);
        let snapshot = &snapshot;
        let worker = scope.spawn(move || {
            worker_start.wait();
            for _ in 0..BACKGROUND_ENCODES {
                black_box(encode_lfrs(snapshot));
            }
        });
        start.wait();
        contended_clocks = tick_clocks(&mut contended_world);
        worker.join().expect("background encoder");
    });
    let contended_tick_ns = median(&mut contended_clocks);
    assert_eq!(
        expected_after_ticks,
        deterministic_state_digest(&contended_world.capture_snapshot()),
        "background encoding must not alter tick semantics"
    );
    let tick_interference_ppm = contended_tick_ns
        .saturating_mul(1_000_000)
        .checked_div(baseline_tick_ns.max(1))
        .expect("non-zero denominator");

    println!(
        "snapshot-wall-clock-evidence workload=signalized-corridor-v1 lfca_exact={} artifact_digest={:x} network_revision={:x} routes={} vehicles={} tick={} command_cursor={} lfrs_exact_bytes={} samples={} capture_main_thread_ns_median={} encode_background_ns_median={} published_load_restore_ns_median={} tick_samples={} baseline_tick_ns_median={} background_encodes={} contended_tick_ns_median={} tick_interference_ppm={} classification=descriptive-initial-not-product-pass",
        snapshot_evidence::CORRIDOR.len(),
        artifact_digest,
        network_revision.into_digest(),
        snapshot.routes().len(),
        snapshot.vehicles().len(),
        snapshot.tick(),
        snapshot.command_cursor(),
        bytes.len(),
        WALL_SAMPLES,
        capture_ns,
        encode_ns,
        restore_ns,
        TICK_SAMPLES,
        baseline_tick_ns,
        BACKGROUND_ENCODES,
        contended_tick_ns,
        tick_interference_ppm,
    );
}
