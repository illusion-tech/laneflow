//! #987 逐分配事件的堆峰值；独立二进制、单测试，不与墙钟或 Region 计数混用。
//! DHAT 只追踪测量区间内的新分配；宿主持有的 LFCA 和被认证修订分别加回合同账本。
//! 堆请求字节不等于 RSS，也不包含分配器内部元数据、栈或分析器自身开销。

#[path = "support/synthetic_network.rs"]
mod synthetic_network;
#[path = "support/policy.rs"]
mod test_policy;

use laneflow_compiler::{Compiler, PortableEmissionProvenance};
use laneflow_format::FormatLimits;
use laneflow_runtime::{
    CommittedNetworkSource, ExecutionConfig, PublishedLfcaReference, SnapshotDigestLimits,
    TrafficWorld, WorldConfig, attest_published_shared_network_revision,
};
use laneflow_static_network::{
    AttestationOutcome, AttestationVerdict, SharedNetworkBuildLimits, SharedNetworkBuildOptions,
    SpatialBuildOption, build_shared_network_revision,
};

#[global_allocator]
static GLOBAL: dhat::Alloc = dhat::Alloc;

const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(2 * 1024 * 1024 * 1024, 2 * 1024 * 1024 * 1024);

fn measure_heap<T>(operation: impl FnOnce() -> T) -> (T, dhat::HeapStats) {
    let profiler = dhat::Profiler::builder()
        .testing()
        .trim_backtraces(Some(4))
        .build();
    let result = operation();
    let stats = dhat::HeapStats::get();
    drop(profiler);
    (result, stats)
}

#[test]
#[ignore = "manual release heap evidence; run inside an approved counting window"]
fn attestation_and_digest_heap_peak_evidence() {
    // 短命分配也必须入峰值；同时验证相邻区间不累计。
    for size in [8_192, 4_096] {
        let ((), stats) = measure_heap(|| {
            let bytes = vec![0_u8; size];
            std::hint::black_box(&bytes);
        });
        assert_eq!(stats.max_bytes, size);
        assert_eq!(stats.curr_bytes, 0);
    }
    for tier in synthetic_network::evidence_tiers() {
        let shape = synthetic_network::WorkloadShape::for_stable_entities(tier);
        let vehicles = tier / 10;
        let routes = vehicles.min(shape.synthetic_tiles);
        let output = Compiler::new()
            .compile(synthetic_network::build_mixed_network(shape, false))
            .unwrap();
        let provenance =
            PortableEmissionProvenance::try_new(format!("laneflow-987-evidence-{tier}")).unwrap();
        let candidate = laneflow_compiler::emit_portable_candidate(
            &output,
            &provenance,
            FormatLimits::HARD,
            laneflow_compiler::PortableDiffBase::Genesis,
        )
        .unwrap();
        let lfca = candidate.canonical_artifact().bytes().to_vec();
        drop(candidate);
        drop(output);
        let options =
            SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS);
        let (root, root_stats) = measure_heap(|| {
            build_shared_network_revision(
                laneflow_format::check_canonical_network_input(lfca.as_slice(), FormatLimits::HARD)
                    .unwrap(),
                options,
            )
            .unwrap()
        });
        let origin = root.canonical_origin();
        let reference = PublishedLfcaReference::new(
            format!("fixture://#987/{tier}"),
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .unwrap();
        let (outcome, attestation) = measure_heap(|| {
            attest_published_shared_network_revision(
                &root,
                &reference,
                lfca.as_slice(),
                FormatLimits::HARD,
                SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
                None,
            )
        });
        assert!(matches!(outcome, AttestationOutcome::Concluded(report)
            if report.verdict == AttestationVerdict::Pass));
        assert_eq!(
            attestation.curr_bytes, 0,
            "no retained attestation allocation"
        );
        assert!(
            attestation.max_bytes >= root_stats.curr_bytes,
            "candidate root must be counted"
        );
        let increment = lfca.len() + attestation.max_bytes;
        let coexist = increment + root_stats.curr_bytes;
        let mut world = TrafficWorld::install(
            root.clone(),
            WorldConfig::new(vehicles, routes, u64::from(routes) * 3 + 1_024, 1_024, 100),
            ExecutionConfig::new(std::num::NonZeroU32::MIN),
            CommittedNetworkSource::Published { reference },
            987_500,
            test_policy::selection(&root),
        )
        .unwrap();
        synthetic_network::populate(&mut world, vehicles, routes);
        let (direct, direct_stats) = measure_heap(|| {
            world
                .direct_state_digest(SnapshotDigestLimits::new(64 * 1024 * 1024))
                .unwrap()
        });
        let (legacy, legacy_stats) = measure_heap(|| {
            let snapshot = world.capture_snapshot().unwrap();
            laneflow_runtime::deterministic_state_digest(&snapshot).unwrap()
        });
        assert_eq!(direct, legacy);
        assert_eq!(direct_stats.curr_bytes, 0);
        assert_eq!(legacy_stats.curr_bytes, 0);
        println!(
            "#987-evidence heap-peak smoke={} tier={tier} vehicles={vehicles} routes={routes} \
            source_bytes={} root_heap_bytes={} additional_peak_bytes={} \
            contract_increment_peak_bytes={increment} contract_coexist_peak_bytes={coexist} \
            steady_retained_bytes={} direct_heap_peak_bytes={} legacy_heap_peak_bytes={}",
            synthetic_network::smoke_mode(),
            lfca.len(),
            root_stats.curr_bytes,
            attestation.max_bytes,
            attestation.curr_bytes,
            direct_stats.max_bytes,
            legacy_stats.max_bytes
        );
    }
}
