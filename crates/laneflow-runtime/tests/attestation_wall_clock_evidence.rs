//! #987 计时段证据：共享根认证墙钟、直算摘要耗时与并发 step 干扰。
//!
//! 全部 `#[ignore]`——正式测量在安静窗口内以
//! `cargo +1.98.0 test --release --locked -p laneflow-runtime --test attestation_wall_clock_evidence -- --ignored --nocapture --test-threads=1`
//! 运行；输出为 `#987-evidence wall-clock ...` 行，供回填设计 §8 与
//! `docs/reference/` 基线对。无窗口时只用万档以下做功能验证。

#[path = "support/synthetic_network.rs"]
mod synthetic_network;
#[path = "support/policy.rs"]
mod test_policy;

use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use laneflow_compiler::{Compiler, PortableEmissionProvenance};
use laneflow_format::FormatLimits;
use laneflow_runtime::{
    CommittedNetworkSource, ExecutionConfig, PublishedLfcaReference, SnapshotDigestLimits,
    TickInput, TrafficWorld, WorldConfig, attest_published_shared_network_revision,
};
use laneflow_static_network::{
    AttestationOutcome, AttestationVerdict, SharedNetworkBuildLimits, SharedNetworkBuildOptions,
    SharedNetworkRevision, SpatialBuildOption, build_shared_network_revision,
};
use synthetic_network::{WorkloadShape, build_mixed_network};

/// 证据测试三档（#380 账本口径的稳定静态实体数）。
pub(crate) const TIERS: [u32; 3] = [10_000, 100_000, 1_000_000];
const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(2 * 1024 * 1024 * 1024, 2 * 1024 * 1024 * 1024);
const DIGEST_LIMIT_BYTES: u64 = 64 * 1024 * 1024;
const WARMUP: usize = 2;
const SAMPLES: usize = 5;
const CONCURRENT_TICKS: usize = 2_048;

/// 合成一档 LFCA 字节并构建共享根与引用（生成属重负载，窗口内调用）。
struct TierFixture {
    lfca: Vec<u8>,
    root: Arc<SharedNetworkRevision>,
    reference: PublishedLfcaReference,
    root_build_ns: u128,
}

fn build_tier(stable_entities: u32) -> TierFixture {
    let started = Instant::now();
    let unit = build_mixed_network(WorkloadShape::for_stable_entities(stable_entities), false);
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
    let origin = root.canonical_origin();
    let reference = PublishedLfcaReference::new(
        format!("fixture://#987/{stable_entities}"),
        origin.canonical_artifact_digest(),
        origin.canonical_artifact_byte_length(),
        origin.network_revision(),
    )
    .unwrap();
    TierFixture {
        lfca,
        root,
        reference,
        root_build_ns: started.elapsed().as_nanos(),
    }
}

fn attest_once(fixture: &TierFixture) -> AttestationOutcome {
    attest_published_shared_network_revision(
        &fixture.root,
        &fixture.reference,
        fixture.lfca.as_slice(),
        FormatLimits::HARD,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
        None,
    )
}

/// 认证墙钟（四时机语义同一入口；这里测「认证调用」本身的墙钟，安装 /
/// 恢复 / 绑定 / 切换前的停顿 = 该墙钟计入各自流程）。输出每档的
/// root 构建（生成+构建）与认证样本 min/median/max。
#[test]
#[ignore = "manual single-thread release wall-clock evidence; run inside a quiet window"]
fn attestation_wall_clock_evidence() {
    for tier in TIERS {
        let fixture = build_tier(tier);
        // 健康性：Pass 才有测量意义。
        assert!(
            matches!(attest_once(&fixture), AttestationOutcome::Concluded(report)
                if report.verdict == AttestationVerdict::Pass)
        );
        for _ in 0..WARMUP {
            let _ = black_box(attest_once(&fixture));
        }
        let mut samples: Vec<u128> = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let started = Instant::now();
            let outcome = attest_once(&fixture);
            samples.push(started.elapsed().as_nanos());
            assert!(matches!(outcome, AttestationOutcome::Concluded(report)
                if report.verdict == AttestationVerdict::Pass));
        }
        samples.sort_unstable();
        let median = samples[samples.len() / 2];
        println!(
            "#987-evidence wall-clock tier={tier} entities={tier} \
             lfca_bytes={} root_build_ns={} attest_samples={:?} attest_median_ns={median}",
            fixture.lfca.len(),
            fixture.root_build_ns,
            samples,
        );
    }
}

/// 直算摘要墙钟：装真实世界后测 `direct_state_digest`，并与现行
/// capture + digest 路径逐字节核对（健康性）与对照耗时。
#[test]
#[ignore = "manual single-thread release wall-clock evidence; run inside a quiet window"]
fn direct_digest_wall_clock_evidence() {
    for tier in TIERS {
        let fixture = build_tier(tier);
        let world = TrafficWorld::install(
            Arc::clone(&fixture.root),
            WorldConfig::new(8, 4, 1_024, 1_024, 100),
            ExecutionConfig::new(std::num::NonZeroU32::MIN),
            CommittedNetworkSource::Published {
                reference: fixture.reference.clone(),
            },
            987_000 + u64::from(tier % 1_000),
            test_policy::selection(&fixture.root),
        )
        .expect("install");
        let limits = SnapshotDigestLimits::new(DIGEST_LIMIT_BYTES);
        for _ in 0..WARMUP {
            black_box(world.direct_state_digest(limits).unwrap());
        }
        let mut samples: Vec<u128> = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let started = Instant::now();
            let digest = world.direct_state_digest(limits).expect("direct digest");
            samples.push(started.elapsed().as_nanos());
            // 与现行路径逐字节一致（健康性）。
            let captured = world.capture_snapshot().expect("capture");
            let legacy = laneflow_runtime::deterministic_state_digest(&captured).expect("legacy");
            assert_eq!(digest, legacy);
        }
        samples.sort_unstable();
        let median = samples[samples.len() / 2];
        println!(
            "#987-evidence direct-digest tier={tier} entities={tier} samples={:?} median_ns={median}",
            samples,
        );
    }
}

/// 在用根审计对并发 step 的干扰：主线程 tick 计时（无审计基线 vs
/// 认证工作线程并发），对照模板取自 snapshot 干扰证据测试；末尾以
/// 确定性摘要断言并发不改变 tick 语义。
#[test]
#[ignore = "manual single-thread release wall-clock evidence; run inside a quiet window"]
fn concurrent_attest_tick_interference_evidence() {
    let tier = 100_000;
    let fixture = build_tier(tier);
    let mut world = TrafficWorld::install(
        Arc::clone(&fixture.root),
        WorldConfig::new(8, 4, 1_024, 1_024, 100),
        ExecutionConfig::new(std::num::NonZeroU32::MIN),
        CommittedNetworkSource::Published {
            reference: fixture.reference.clone(),
        },
        987_100,
        test_policy::selection(&fixture.root),
    )
    .expect("install");
    let input = TickInput::new(100);
    // 每段含 16 步暖机；影子世界与被测世界走完全相同的总步数（无并发），
    // 其摘要作 tick 语义不受并发认证影响的确定性参照。
    const WARMUP_TICKS: usize = 16;
    let tick_clocks = |world: &mut TrafficWorld, ticks: usize| -> Vec<u128> {
        for _ in 0..WARMUP_TICKS {
            world.step(input).expect("warmup step");
        }
        let mut clocks = Vec::with_capacity(ticks);
        for _ in 0..ticks {
            let started = Instant::now();
            world.step(input).expect("step");
            clocks.push(started.elapsed().as_nanos());
        }
        clocks
    };
    let baseline = tick_clocks(&mut world, CONCURRENT_TICKS);
    let baseline_median = median(&baseline);
    // 并发段：认证线程循环认证，主线程同步 tick 计时。
    let (contended, worker_cycles) = std::thread::scope(|scope| {
        let stamp = Arc::new(std::sync::Barrier::new(2));
        let worker = {
            let stamp = Arc::clone(&stamp);
            let fixture = &fixture;
            scope.spawn(move || {
                stamp.wait();
                let started = Instant::now();
                let mut cycles = 0_u64;
                while started.elapsed().as_secs() < 30 {
                    assert!(
                        matches!(attest_once(fixture), AttestationOutcome::Concluded(report)
                        if report.verdict == AttestationVerdict::Pass)
                    );
                    cycles += 1;
                }
                cycles
            })
        };
        stamp.wait();
        let clocks = tick_clocks(&mut world, CONCURRENT_TICKS);
        let cycles = worker.join().expect("attest worker");
        (clocks, cycles)
    });
    let contended_median = median(&contended);
    let after = world
        .direct_state_digest(SnapshotDigestLimits::new(DIGEST_LIMIT_BYTES))
        .unwrap();
    let mut shadow = TrafficWorld::install(
        Arc::clone(&fixture.root),
        WorldConfig::new(8, 4, 1_024, 1_024, 100),
        ExecutionConfig::new(std::num::NonZeroU32::MIN),
        CommittedNetworkSource::Published {
            reference: fixture.reference.clone(),
        },
        987_100,
        test_policy::selection(&fixture.root),
    )
    .expect("install shadow");
    for _ in 0..(2 * (WARMUP_TICKS + CONCURRENT_TICKS)) {
        shadow.step(input).expect("shadow step");
    }
    let reference = shadow
        .direct_state_digest(SnapshotDigestLimits::new(DIGEST_LIMIT_BYTES))
        .unwrap();
    assert_eq!(
        after, reference,
        "并发认证不得改变 tick 语义（与同程无并发参照世界的摘要一致）"
    );
    let baseline_p95 = percentile95(&baseline);
    let contended_p95 = percentile95(&contended);
    println!(
        "#987-evidence concurrent-interference tier={tier} ticks={CONCURRENT_TICKS} \
         baseline_median_ns={baseline_median} contended_median_ns={contended_median} \
         baseline_p95_ns={baseline_p95} contended_p95_ns={contended_p95} \
         interference_ppm={} interference_p95_ppm={} attest_cycles={worker_cycles}",
        contended_median
            .saturating_sub(baseline_median)
            .saturating_mul(1_000_000)
            / baseline_median.max(1),
        contended_p95
            .saturating_sub(baseline_p95)
            .saturating_mul(1_000_000)
            / baseline_p95.max(1),
    );
}

fn median(values: &[u128]) -> u128 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[sorted.len() / 2]
}

fn percentile95(values: &[u128]) -> u128 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[(sorted.len() * 95) / 100]
}
