//! #987 计数段证据：认证峰值（增量 / 共存分报）、直算摘要 scratch 与
//! 分配计数、稳态保留实测为零。
//!
//! 单测试文件纪律（Region 是进程级计数器，一文件一测试）。正式测量在
//! 安静窗口以
//! `cargo +1.98.0 test --release --locked -p laneflow-runtime --test attestation_allocation_evidence -- --ignored --nocapture --test-threads=1`
//! 运行。

#[path = "support/synthetic_network.rs"]
mod synthetic_network;
#[path = "support/policy.rs"]
mod test_policy;

use std::alloc::System;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use laneflow_compiler::{Compiler, PortableEmissionProvenance};
use laneflow_format::FormatLimits;
use laneflow_runtime::{
    CommittedNetworkSource, ExecutionConfig, PublishedLfcaReference, SnapshotDigestLimits,
    TrafficWorld, WorldConfig, attest_published_shared_network_revision,
};
use laneflow_static_network::{
    AttestationOutcome, AttestationVerdict, SharedNetworkBuildLimits, SharedNetworkBuildOptions,
    SharedNetworkRevision, SpatialBuildOption, build_shared_network_revision,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

/// 证据测试档位（#380 账本口径的稳定静态实体数）。
const TIERS: [u32; 3] = [10_000, 100_000, 1_000_000];
const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(2 * 1024 * 1024 * 1024, 2 * 1024 * 1024 * 1024);
const DIGEST_LIMIT_BYTES: u64 = 64 * 1024 * 1024;

fn allocator_live_bytes() -> u64 {
    let stats = GLOBAL.stats();
    u64::try_from(
        stats
            .bytes_allocated
            .saturating_sub(stats.bytes_deallocated),
    )
    .unwrap_or(u64::MAX)
}

fn build_tier(stable_entities: u32) -> (Vec<u8>, std::sync::Arc<SharedNetworkRevision>) {
    let unit = synthetic_network::build_mixed_network(
        synthetic_network::WorkloadShape::for_stable_entities(stable_entities),
        false,
    );
    let output = Compiler::new().compile(unit).expect("compile");
    let provenance =
        PortableEmissionProvenance::try_new(format!("laneflow-987-evidence-{stable_entities}"))
            .unwrap();
    let candidate = laneflow_compiler::emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        laneflow_compiler::PortableDiffBase::Genesis,
    )
    .expect("emit candidate");
    let lfca: Vec<u8> = candidate.canonical_artifact().bytes().to_vec();
    let root = build_shared_network_revision(
        laneflow_format::check_canonical_network_input(lfca.as_slice(), FormatLimits::HARD)
            .unwrap(),
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
    )
    .expect("build root");
    (lfca, root)
}

/// 认证峰值分报（设计 §6：增量 = 来源字节 + 候选根 + builder scratch；
/// 共存总量 = 增量 + 被认证根）与认证期分配字节；稳态保留实测为零
/// （认证前后 live-bytes 差为 0）。
#[test]
#[ignore = "manual single-thread release allocation evidence; run inside a quiet window"]
fn attestation_allocation_evidence() {
    for tier in TIERS {
        let (lfca, root) = build_tier(tier);
        let origin = root.canonical_origin();
        let reference = PublishedLfcaReference::new(
            format!("fixture://#987/{tier}"),
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .unwrap();

        // 稳态保留：认证前后 live-bytes 差（期望 0）。
        let steady_before = allocator_live_bytes();
        let outcome = attest_published_shared_network_revision(
            &root,
            &reference,
            lfca.as_slice(),
            FormatLimits::HARD,
            SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
            None,
        );
        let steady_delta = allocator_live_bytes().saturating_sub(steady_before);
        assert!(matches!(outcome, AttestationOutcome::Concluded(report)
            if report.verdict == AttestationVerdict::Pass));

        // 认证期增量与共存：Region 记账 + 采样峰值（共存放开头读数）。
        let region = Region::new(GLOBAL);
        let coexist_before = allocator_live_bytes();
        let running = AtomicBool::new(true);
        let peak = AtomicU64::new(0);
        thread::scope(|scope| {
            let running = &running;
            let peak = &peak;
            scope.spawn(move || {
                while running.load(Ordering::Relaxed) {
                    peak.fetch_max(allocator_live_bytes(), Ordering::Relaxed);
                    thread::sleep(Duration::from_millis(1));
                }
            });
            let region = &region;
            let root = &root;
            let reference = &reference;
            let lfca = &lfca;
            let attest = || {
                black_box(attest_published_shared_network_revision(
                    root,
                    reference,
                    lfca.as_slice(),
                    FormatLimits::HARD,
                    SharedNetworkBuildOptions::new(
                        SpatialBuildOption::RetainAvailable,
                        BUILD_LIMITS,
                    ),
                    None,
                ))
            };
            let _ = attest();
            let _ = attest();
            let _ = region.change();
            let outcome = attest();
            running.store(false, Ordering::Relaxed);
            assert!(matches!(outcome, AttestationOutcome::Concluded(report)
                if report.verdict == AttestationVerdict::Pass));
        });
        let stats = region.change();
        let coexist_delta = allocator_live_bytes().saturating_sub(coexist_before);
        let _ = &region; // Region 析构前读取完成
        println!(
            "#987-evidence allocation tier={tier} lfca_bytes={} root_retained_bytes={} \
             attest_alloc_bytes={} attest_dealloc_bytes={} \
             sampled_peak_live_bytes={} coexist_delta_bytes={coexist_delta} \
             steady_delta_bytes={steady_delta}",
            lfca.len(),
            root.retained_logical_bytes(),
            stats.bytes_allocated,
            stats.bytes_deallocated,
            peak.load(Ordering::Relaxed),
        );
        assert_eq!(steady_delta, 0, "稳态保留新增须实测为零");
    }
}

/// 直算摘要归一化 scratch 上界（限额二分）与分配计数：限额单调卡点即
/// 归一化 scratch 精确上界（切片①的边界测试在生产档的延伸），对照
/// 现行 capture + digest 路径的分配字节。
#[test]
#[ignore = "manual single-thread release allocation evidence; run inside a quiet window"]
fn direct_digest_scratch_and_allocation_evidence() {
    for tier in TIERS {
        let (lfca, root) = build_tier(tier);
        let origin = root.canonical_origin();
        let reference = PublishedLfcaReference::new(
            format!("fixture://#987/{tier}"),
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .unwrap();
        let world = TrafficWorld::install(
            std::sync::Arc::clone(&root),
            WorldConfig::new(8, 4, 1_024, 1_024, 100),
            ExecutionConfig::new(std::num::NonZeroU32::MIN),
            CommittedNetworkSource::Published {
                reference: reference.clone(),
            },
            987_500 + u64::from(tier % 1_000),
            test_policy::selection(&root),
        )
        .expect("install");

        // 限额二分：直算 scratch 精确上界。
        let mut low = 0_u64;
        let mut high = 2 * 1024 * 1024 * 1024_u64;
        while high - low > 1 {
            let middle = low + (high - low) / 2;
            match world.direct_state_digest(SnapshotDigestLimits::new(middle)) {
                Ok(_) => high = middle,
                Err(laneflow_runtime::SnapshotDigestError::LimitExceeded) => low = middle,
                Err(other) => panic!("unexpected error: {other:?}"),
            }
        }
        let scratch_bound_bytes = high;

        // 分配计数：直算 vs 现行 capture + digest。
        let region = Region::new(GLOBAL);
        let limits = SnapshotDigestLimits::new(DIGEST_LIMIT_BYTES);
        world.direct_state_digest(limits).expect("warm");
        let _ = region.change();
        let digest = world.direct_state_digest(limits).expect("direct");
        let direct_alloc = region.change().bytes_allocated;
        let _ = region.change();
        let captured = world.capture_snapshot().expect("capture");
        let legacy = laneflow_runtime::deterministic_state_digest(&captured).expect("legacy");
        let legacy_alloc = region.change().bytes_allocated;
        assert_eq!(digest, legacy, "直算与捕获路径逐字节一致");
        println!(
            "#987-evidence direct-scratch tier={tier} scratch_bound_bytes={scratch_bound_bytes} \
             direct_alloc_bytes={direct_alloc} legacy_alloc_bytes={legacy_alloc} lfca_bytes={}",
            lfca.len(),
        );
    }
}
