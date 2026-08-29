//! #513 切片 C 跨修订切换墙钟基线（lfsd-migration 夹具，同机描述性）。
//!
//! 与 `cutover_cross_budget_evidence` 分体（沿 #441 先例：计数分配器的
//! 原子记账会污染墙钟，两维度不得同进程）。`#[ignore]` + release 手动
//! 运行，复现命令：
//!
//! ```text
//! cargo +1.98.0 test --release --locked -p laneflow-runtime \
//!   --test cutover_cross_wall_clock_evidence -- --ignored --nocapture
//! ```
//!
//! 中位数对偶数样本取中间两值平均；dev profile 计时不得登记。

use std::sync::Arc;
use std::time::Instant;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, CutoverPreflightLimits, CutoverTransactionLimits, LfcaOriginBinding,
    MigrationPolicyKind, NetworkRevisionCutoverDescriptor, PublishedLfcaReference,
    RouteRegisterInput, SemanticDiffOriginBinding, TickInput, TrafficWorld, VehicleSpawnInput,
    WorldConfig,
};
use laneflow_static_contract::{
    ExactByteLength, SEMANTIC_DIFF_FORMAT_VERSION, Sha256Digest, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use sha2::Digest as _;

const ORACLE_BASE: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/oracle-base.lfca"
);
const ORACLE_TARGET: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/oracle-target.lfca"
);
const ORACLE_LFSD: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/oracle-expected.lfsd"
);
const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024);
const DELTA_MS: u64 = 100;
const VEHICLES: u32 = 8;
const WINDOW_TICKS: u32 = 64;
const SAMPLES: usize = 15;

fn build(bytes: &[u8]) -> Arc<SharedNetworkRevision> {
    let input = check_canonical_network_input(bytes, FormatLimits::HARD).expect("checked");
    build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(SpatialBuildOption::Omit, BUILD_LIMITS),
    )
    .expect("build")
}

fn source_for(key: &str, bytes: &[u8]) -> CommittedNetworkSource {
    let input = check_canonical_network_input(bytes, FormatLimits::HARD).expect("checked");
    CommittedNetworkSource::Published {
        reference: PublishedLfcaReference::new(
            key,
            input.canonical_artifact_digest(),
            input.canonical_artifact_byte_length(),
            input.network_revision(),
        )
        .expect("non-empty fixture key"),
    }
}

fn descriptor_for(world: &TrafficWorld) -> NetworkRevisionCutoverDescriptor {
    let digest: [u8; 32] = sha2::Sha256::digest(ORACLE_LFSD).into();
    NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*world.revision().canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(*build(ORACLE_TARGET).canonical_origin()),
        Some(SemanticDiffOriginBinding::new(
            SEMANTIC_DIFF_FORMAT_VERSION,
            Sha256Digest::from_bytes(digest),
            ExactByteLength::new(ORACLE_LFSD.len() as u64),
        )),
        MigrationPolicyKind::CrossRevisionDirect,
        world.world_binding(),
    )
}

fn world_with_fleet() -> TrafficWorld {
    let revision = build(ORACLE_BASE);
    let mut world = TrafficWorld::install(
        revision,
        WorldConfig::new(VEHICLES, 4, 1_024, 1, DELTA_MS),
        source_for("fixture://cross-base", ORACLE_BASE),
        1,
    )
    .expect("install");
    let traffic = world.traffic();
    let mut pair = None;
    for raw in 0..traffic.lane_edge_count() {
        let edge = laneflow_static_contract::LaneEdgeOrdinal::from_raw(raw);
        if let Some(successor) = traffic
            .successors(edge)
            .and_then(|items| items.first().copied())
        {
            pair = Some((edge, successor));
            break;
        }
    }
    let (entry, exit) = pair.expect("connected pair");
    let route = world
        .register_route(RouteRegisterInput::new(vec![entry, exit]))
        .expect("route");
    for index in 0..VEHICLES {
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000 + 6_500 * u32::from(index),
                0,
            ))
            .expect("vehicle");
    }
    for _ in 0..4 {
        world.step(TickInput::new(DELTA_MS)).expect("warmup step");
    }
    world
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.total_cmp(b));
    let len = values.len();
    if len % 2 == 1 {
        values[len / 2]
    } else {
        (values[len / 2 - 1] + values[len / 2]) / 2.0
    }
}

#[test]
#[ignore = "墙钟证据：release 手动运行，沿 #441/#511 先例"]
fn cross_revision_cutover_wall_clock_evidence() {
    let mut prepare_ns = Vec::with_capacity(SAMPLES);
    let mut commit_ns = Vec::with_capacity(SAMPLES);
    let mut drain_commit_ns = Vec::with_capacity(SAMPLES);
    let mut pause_ns = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        // 在线形态：Prepare →（窗口步进 + 泵入）→ 静默提交。
        let mut world = world_with_fleet();
        let target = build(ORACLE_TARGET);
        let descriptor = descriptor_for(&world);
        let started = Instant::now();
        let mut transaction = world
            .prepare_cross_revision_cutover(
                Arc::clone(&target),
                source_for("fixture://cross-target", ORACLE_TARGET),
                &descriptor,
                ORACLE_LFSD,
                &CutoverPreflightLimits::new(1_048_576),
                &CutoverTransactionLimits::default(),
            )
            .expect("prepare");
        prepare_ns.push(started.elapsed().as_nanos() as f64);
        for _ in 0..WINDOW_TICKS {
            world.step(TickInput::new(DELTA_MS)).expect("window step");
            transaction.pump(&mut world).expect("pump");
        }
        let started = Instant::now();
        let _ = transaction.commit(&mut world).expect("commit");
        commit_ns.push(started.elapsed().as_nanos() as f64);
        assert!(world.migration_journal_stats().is_none());

        // 尾排空形态：全程不泵，静默提交一次吞下整本日志（最坏排空）。
        let mut drained = world_with_fleet();
        let descriptor = descriptor_for(&drained);
        let transaction = drained
            .prepare_cross_revision_cutover(
                build(ORACLE_TARGET),
                source_for("fixture://cross-drained", ORACLE_TARGET),
                &descriptor,
                ORACLE_LFSD,
                &CutoverPreflightLimits::new(1_048_576),
                &CutoverTransactionLimits::default(),
            )
            .expect("prepare");
        for _ in 0..WINDOW_TICKS {
            drained.step(TickInput::new(DELTA_MS)).expect("window step");
        }
        let started = Instant::now();
        let _ = transaction
            .commit(&mut drained)
            .expect("commit drains whole journal");
        drain_commit_ns.push(started.elapsed().as_nanos() as f64);

        // 维护暂停形态：停表内 Prepare → 排空 → 静默提交（完整停顿）。
        let mut paused = world_with_fleet();
        let descriptor = descriptor_for(&paused);
        let started = Instant::now();
        let mut transaction = paused
            .prepare_cross_revision_cutover(
                build(ORACLE_TARGET),
                source_for("fixture://cross-paused", ORACLE_TARGET),
                &descriptor,
                ORACLE_LFSD,
                &CutoverPreflightLimits::new(1_048_576),
                &CutoverTransactionLimits::default(),
            )
            .expect("prepare");
        transaction.pump(&mut paused).expect("paused pump");
        let _ = transaction.commit(&mut paused).expect("paused commit");
        pause_ns.push(started.elapsed().as_nanos() as f64);
    }
    println!("prepare median: {:.3} ms", median(&mut prepare_ns) / 1.0e6);
    println!(
        "quiescent commit median (tail already pumped; rebuild + revalidate + digest + swap): {:.3} ms",
        median(&mut commit_ns) / 1.0e6
    );
    println!(
        "quiescent commit median (draining full {}-tick journal tail in one call): {:.3} ms",
        WINDOW_TICKS,
        median(&mut drain_commit_ns) / 1.0e6
    );
    println!(
        "maintenance pause full median (prepare + drain + commit, no steps): {:.3} ms",
        median(&mut pause_ns) / 1.0e6
    );
}
