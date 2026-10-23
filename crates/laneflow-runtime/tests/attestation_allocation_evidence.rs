//! #987 计数段证据：认证、直算摘要 scratch 预算边界与
//! 分配计数、稳态保留实测为零。
//!
//! Region 是进程级计数器；本文件只有一个测试，避免 libtest 跨测试报告污染。正式测量在
//! 安静窗口以
//! `cargo +1.98.0 test --release --locked -p laneflow-runtime --test attestation_allocation_evidence -- --ignored --nocapture --test-threads=1`
//! 运行。

#[path = "support/synthetic_network.rs"]
mod synthetic_network;
#[path = "support/policy.rs"]
mod test_policy;

use std::alloc::System;
use std::hint::black_box;

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

fn build_tier(stable_entities: u32) -> (Vec<u8>, std::sync::Arc<SharedNetworkRevision>, u64) {
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
    let root_before = allocator_live_bytes();
    let root = build_shared_network_revision(
        laneflow_format::check_canonical_network_input(lfca.as_slice(), FormatLimits::HARD)
            .unwrap(),
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
    )
    .expect("build root");
    let root_live_bytes = allocator_live_bytes()
        .checked_sub(root_before)
        .expect("root live bytes");
    (lfca, root, root_live_bytes)
}

/// 认证期分配与稳态保留；峰值由独立 DHAT 二进制记录。
#[test]
#[ignore = "manual single-thread release allocation evidence; run inside a quiet window"]
fn attestation_allocation_evidence() {
    measure_attestation_allocation();
    direct_digest_scratch_and_allocation_evidence();
}

fn measure_attestation_allocation() {
    for tier in synthetic_network::evidence_tiers() {
        let (lfca, root, root_live_bytes) = build_tier(tier);
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
        let steady_delta = i128::from(allocator_live_bytes()) - i128::from(steady_before);
        assert!(matches!(outcome, AttestationOutcome::Concluded(report)
            if report.verdict == AttestationVerdict::Pass));

        // 分配计数单独测一次，暖机不进入 Region。
        let attest = || {
            black_box(attest_published_shared_network_revision(
                &root,
                &reference,
                lfca.as_slice(),
                FormatLimits::HARD,
                SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
                None,
            ))
        };
        let _ = attest();
        let region = Region::new(GLOBAL);
        let outcome = attest();
        let stats = region.change();
        assert!(matches!(outcome, AttestationOutcome::Concluded(report)
            if report.verdict == AttestationVerdict::Pass));
        assert_eq!(stats.bytes_allocated, stats.bytes_deallocated);

        println!(
            "#987-evidence allocation smoke={} tier={tier} lfca_bytes={} root_retained_logical_bytes={} root_live_bytes={root_live_bytes} attest_alloc_bytes={} attest_dealloc_bytes={} steady_delta_bytes={steady_delta}",
            synthetic_network::smoke_mode(),
            lfca.len(),
            root.retained_logical_bytes(),
            stats.bytes_allocated,
            stats.bytes_deallocated,
        );
        assert_eq!(steady_delta, 0, "稳态保留新增须实测为零");
    }
}

/// 直算摘要归一化 scratch 上界（限额二分）与分配计数：限额单调卡点即
/// 归一化 scratch 精确上界（切片①的边界测试在生产档的延伸），对照
/// 现行 capture + digest 路径的分配字节。
fn direct_digest_scratch_and_allocation_evidence() {
    for tier in synthetic_network::evidence_tiers() {
        let (lfca, root, _) = build_tier(tier);
        let origin = root.canonical_origin();
        let reference = PublishedLfcaReference::new(
            format!("fixture://#987/{tier}"),
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .unwrap();
        let vehicles = tier / 10;
        let routes = vehicles
            .min(synthetic_network::WorkloadShape::for_stable_entities(tier).synthetic_tiles);
        let mut world = TrafficWorld::install(
            std::sync::Arc::clone(&root),
            WorldConfig::new(vehicles, routes, u64::from(routes) * 3 + 1_024, 1_024, 100),
            ExecutionConfig::new(std::num::NonZeroU32::MIN),
            CommittedNetworkSource::Published {
                reference: reference.clone(),
            },
            987_500 + u64::from(tier % 1_000),
            test_policy::selection(&root),
        )
        .expect("install");
        // 与墙钟测试同口径的动态记录世界（档位 1/10 车辆），使 scratch 上界
        // 与分配对照覆盖记录表规模，而不是空世界常量。
        synthetic_network::populate(&mut world, vehicles, routes);

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
        assert!(
            world
                .direct_state_digest(SnapshotDigestLimits::new(high))
                .is_ok()
        );
        assert!(matches!(
            world.direct_state_digest(SnapshotDigestLimits::new(high - 1)),
            Err(laneflow_runtime::SnapshotDigestError::LimitExceeded)
        ));

        // 分配计数：直算 vs 现行 capture + digest。
        let limits = SnapshotDigestLimits::new(DIGEST_LIMIT_BYTES);
        world.direct_state_digest(limits).expect("warm");
        let region = Region::new(GLOBAL);
        let digest = world.direct_state_digest(limits).expect("direct");
        let direct_alloc = region.change().bytes_allocated;
        // 同一输入的独立计数必须一致，防止暖机或前一路径混入区间。
        let repeat_region = Region::new(GLOBAL);
        assert_eq!(
            digest,
            world.direct_state_digest(limits).expect("repeat direct")
        );
        assert_eq!(direct_alloc, repeat_region.change().bytes_allocated);
        let region = Region::new(GLOBAL);
        let captured = world.capture_snapshot().expect("capture");
        let legacy = laneflow_runtime::deterministic_state_digest(&captured).expect("legacy");
        let legacy_alloc = region.change().bytes_allocated;
        assert_eq!(digest, legacy, "直算与捕获路径逐字节一致");
        println!(
            "#987-evidence direct-scratch smoke={} tier={tier} vehicles={vehicles} routes={routes} \
             scratch_bound_bytes={scratch_bound_bytes} direct_alloc_bytes={direct_alloc} \
             legacy_alloc_bytes={legacy_alloc} lfca_bytes={}",
            synthetic_network::smoke_mode(),
            lfca.len(),
        );
    }
}
