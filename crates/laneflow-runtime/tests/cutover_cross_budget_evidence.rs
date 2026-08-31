//! #513 切片 C 跨修订切换预算基线（lfsd-migration 夹具，同机描述性）——
//! 分配账本面。
//!
//! 合同 `traffic-runtime-revision-cutover.md` §9 度量协议：同机描述性基线，
//! 量化切片登记初值。本文件是跨修订直移侧初值中分配维度的可复现证据：
//!
//! - 武装期在线准备干扰不变量：武装前后稳态 tick 分配账本相等（武装只
//!   写入已预留 arena，不新增分配）——硬断言；
//! - 迁移增量日志字节口径：每 tick 精确 = TICK 头 21 字节 + 43 字节 ×
//!   活跃车辆数（活跃车全速段每 tick 都变化）——结构性硬断言；
//! - Prepare 与静默提交（排空 + 占用重建 + 全量重验证 + 摘要复核 + 原地
//!   晋升）的分配账本、迁移期共存峰值输入——描述性输出，登记进合同
//!   §9 初值表。
//!
//! 墙钟维度在 `cutover_cross_wall_clock_evidence` 的未插桩二进制中采样
//!（沿 #441 先例，两维度不得同进程）。本文件沿 #511
//! `cutover_budget_evidence` 的纪律：单一默认测试承载全局计数分配器，
//! 账本数字为描述性输出，干扰不变量为硬断言。

use std::alloc::System;
use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, CutoverPreflightLimits, CutoverTransactionLimits,
    DEFAULT_MIGRATION_DELTA_JOURNAL_BYTES, LfcaOriginBinding, MigrationPolicyKind,
    NetworkRevisionCutoverDescriptor, PublishedLfcaReference, RouteRegisterInput,
    SemanticDiffOriginBinding, TickInput, TrafficWorld, VehicleSpawnInput, WorldConfig,
};
use laneflow_static_contract::{
    ExactByteLength, SEMANTIC_DIFF_FORMAT_VERSION, Sha256Digest, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use sha2::Digest as _;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

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
const STEADY_TICKS: u32 = 16;
const VEHICLES: u32 = 8;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Ledger {
    allocations: usize,
    reallocations: usize,
    allocated_bytes: usize,
    deallocated_bytes: usize,
    live_delta_bytes: i128,
}

impl Ledger {
    fn from_change(stats: stats_alloc::Stats) -> Self {
        Self {
            allocations: stats.allocations,
            reallocations: stats.reallocations,
            allocated_bytes: stats.bytes_allocated,
            deallocated_bytes: stats.bytes_deallocated,
            live_delta_bytes: stats.bytes_allocated as i128 - stats.bytes_deallocated as i128,
        }
    }
}

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

fn target_origin() -> laneflow_static_network::CanonicalNetworkOrigin {
    *build(ORACLE_TARGET).canonical_origin()
}

fn descriptor_for(world: &TrafficWorld) -> NetworkRevisionCutoverDescriptor {
    let digest: [u8; 32] = sha2::Sha256::digest(ORACLE_LFSD).into();
    NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*world.revision().canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(target_origin()),
        Some(SemanticDiffOriginBinding::new(
            SEMANTIC_DIFF_FORMAT_VERSION,
            Sha256Digest::from_bytes(digest),
            ExactByteLength::new(ORACLE_LFSD.len() as u64),
        )),
        MigrationPolicyKind::CrossRevisionDirect,
        world.world_binding(),
    )
}

fn entry_exit(
    world: &TrafficWorld,
) -> (
    laneflow_static_contract::LaneEdgeOrdinal,
    laneflow_static_contract::LaneEdgeOrdinal,
) {
    let traffic = world.traffic();
    for raw in 0..traffic.lane_edge_count() {
        let edge = laneflow_static_contract::LaneEdgeOrdinal::from_raw(raw);
        if let Some(successor) = traffic
            .successors(edge)
            .and_then(|items| items.first().copied())
        {
            return (edge, successor);
        }
    }
    panic!("fixture exposes a connected edge pair");
}

fn world_with_fleet() -> TrafficWorld {
    let revision = build(ORACLE_BASE);
    let mut world = TrafficWorld::install(
        revision,
        WorldConfig::new(VEHICLES, 4, 1_024, 1_024, 1, DELTA_MS),
        source_for("fixture://cross-base", ORACLE_BASE),
        1,
    )
    .expect("install");
    let (entry, exit) = entry_exit(&world);
    let route = world
        .register_route(RouteRegisterInput::new(vec![entry, exit]))
        .expect("route");
    for index in 0..VEHICLES {
        let progress = 1_000 + 6_500 * index;
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                progress,
                0,
            ))
            .expect("vehicle");
    }
    for _ in 0..4 {
        world.step(TickInput::new(DELTA_MS)).expect("warmup step");
    }
    world
}

fn prepare_transaction(world: &mut TrafficWorld) -> laneflow_runtime::CutoverTransaction {
    world
        .prepare_cross_revision_cutover(
            build(ORACLE_TARGET),
            source_for("fixture://cross-target", ORACLE_TARGET),
            &descriptor_for(world),
            ORACLE_LFSD,
            &CutoverPreflightLimits::new(1_048_576),
            &CutoverTransactionLimits::default(),
        )
        .expect("prepare")
}

#[test]
fn cross_revision_cutover_budget_evidence() {
    // —— 武装期在线准备干扰：武装前后稳态 tick 账本相等（硬断言）——
    let mut unarmed = world_with_fleet();
    for _ in 0..STEADY_TICKS {
        unarmed.step(TickInput::new(DELTA_MS)).expect("warmup step");
    }
    let unarmed_ledger = {
        let region = Region::new(GLOBAL);
        for _ in 0..STEADY_TICKS {
            unarmed.step(TickInput::new(DELTA_MS)).expect("step");
        }
        Ledger::from_change(region.change())
    };

    let mut armed = world_with_fleet();
    // 与未武装侧相同的预热步数：保证两侧在测量起点处于同一演化状态
    //（占用索引容量已长到位），账本差异只剩日志写入路径本身。
    for _ in 0..STEADY_TICKS {
        armed
            .step(TickInput::new(DELTA_MS))
            .expect("pre-prepare warmup");
    }
    let mut transaction = prepare_transaction(&mut armed);
    let armed_ledger = {
        // 新建测量区段：arena 预留与候选克隆留在区段外。
        let region = Region::new(GLOBAL);
        for _ in 0..STEADY_TICKS {
            armed.step(TickInput::new(DELTA_MS)).expect("armed step");
        }
        Ledger::from_change(region.change())
    };
    assert_eq!(
        unarmed_ledger, armed_ledger,
        "armed steady ticks must not allocate beyond unarmed steady ticks"
    );

    // —— 迁移增量日志字节口径（结构性硬断言）——
    let stats = armed.migration_journal_stats().expect("armed journal");
    assert!(!stats.overflowed);
    assert_eq!(stats.record_count, u64::from(STEADY_TICKS));
    // 每条 TICK 记录 = 21 字节头 + 43 字节 × 活跃车辆（全速段逐 tick 变化）。
    assert_eq!(
        stats.written_bytes,
        u64::from(STEADY_TICKS * (21 + 43 * VEHICLES)),
        "journal bytes are exactly header + 43 per changed vehicle"
    );
    println!(
        "journal: {written} bytes over {ticks} ticks ({per_tick} B/tick, {vehicles} vehicles)",
        written = stats.written_bytes,
        ticks = STEADY_TICKS,
        per_tick = 21 + 43 * VEHICLES,
        vehicles = VEHICLES,
    );

    // —— Prepare 分配账本（描述性：候选动态双份 + 日志 arena 预留等）——
    let mut probe = world_with_fleet();
    probe.step(TickInput::new(DELTA_MS)).expect("probe step");
    let prepare_ledger = {
        let region = Region::new(GLOBAL);
        let probe_transaction = prepare_transaction(&mut probe);
        std::mem::forget(probe_transaction);
        Ledger::from_change(region.change())
    };
    println!("prepare ledger: {prepare_ledger:?}");

    // —— 迁移期共存峰值输入（描述性；§7 清单在夹具口径下的可测投影）——
    println!(
        "coexistence: journal_written={written} arena_reserved={arena} target_root_retained={retained} lfsd_bytes={lfsd}",
        written = stats.written_bytes,
        arena = DEFAULT_MIGRATION_DELTA_JOURNAL_BYTES,
        retained = build(ORACLE_TARGET).retained_logical_bytes(),
        lfsd = ORACLE_LFSD.len(),
    );

    // —— 静默提交（排空 + 重建 + 重验证 + 摘要 + 原地晋升）账本 ——
    let commit_ledger = {
        let region = Region::new(GLOBAL);
        transaction.pump(&mut armed).expect("pump");
        let _ = transaction.commit(&mut armed).expect("commit");
        Ledger::from_change(region.change())
    };
    println!("drain+commit ledger: {commit_ledger:?}");
    assert!(armed.migration_journal_stats().is_none());
}
