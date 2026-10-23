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

const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(2 * 1024 * 1024 * 1024, 2 * 1024 * 1024 * 1024);
const DIGEST_LIMIT_BYTES: u64 = 64 * 1024 * 1024;
const WARMUP: usize = 2;
const SAMPLES: usize = 5;
const CONCURRENT_TICKS: usize = 256;

/// 合成一档 LFCA 字节并构建共享根与引用（生成属重负载，窗口内调用）。
struct TierFixture {
    lfca: Vec<u8>,
    root: Arc<SharedNetworkRevision>,
    reference: PublishedLfcaReference,
    fixture_prepare_ns: u128,
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
        fixture_prepare_ns: started.elapsed().as_nanos(),
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
/// 夹具准备（生成、编译、发射、构建）与认证样本 min/median/max。
#[test]
#[ignore = "manual single-thread release wall-clock evidence; run inside a quiet window"]
fn attestation_wall_clock_evidence() {
    for tier in synthetic_network::evidence_tiers() {
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
            "#987-evidence wall-clock smoke={} tier={tier} entities={tier} \
             lfca_bytes={} fixture_prepare_ns={} attest_samples={:?} attest_median_ns={median}",
            synthetic_network::smoke_mode(),
            fixture.lfca.len(),
            fixture.fixture_prepare_ns,
            samples,
        );
    }
}

/// 直算摘要墙钟（动态记录世界）：按档位 1/10 装载车辆（`populate`，三边链
/// 路线轮转、同路线每边一车），测 `direct_state_digest`；现行路径按
/// capture 与 digest 两段分别计时作对照；直算与现行路径逐字节核对
/// （健康性）。
#[test]
#[ignore = "manual single-thread release wall-clock evidence; run inside a quiet window"]
fn direct_digest_wall_clock_evidence() {
    for tier in synthetic_network::evidence_tiers() {
        let vehicles = tier / 10;
        let routes = vehicles.min(WorkloadShape::for_stable_entities(tier).synthetic_tiles);
        let fixture = build_tier(tier);
        let mut world = TrafficWorld::install(
            Arc::clone(&fixture.root),
            WorldConfig::new(vehicles, routes, u64::from(routes) * 3 + 1_024, 1_024, 100),
            ExecutionConfig::new(std::num::NonZeroU32::MIN),
            CommittedNetworkSource::Published {
                reference: fixture.reference.clone(),
            },
            987_000 + u64::from(tier % 1_000),
            test_policy::selection(&fixture.root),
        )
        .expect("install");
        synthetic_network::populate(&mut world, vehicles, routes);
        let limits = SnapshotDigestLimits::new(DIGEST_LIMIT_BYTES);
        for _ in 0..WARMUP {
            black_box(world.direct_state_digest(limits).unwrap());
            let captured = world.capture_snapshot().expect("capture warmup");
            black_box(laneflow_runtime::deterministic_state_digest(&captured).unwrap());
        }
        let mut direct: Vec<u128> = Vec::with_capacity(SAMPLES);
        let mut capture: Vec<u128> = Vec::with_capacity(SAMPLES);
        let mut legacy_digest: Vec<u128> = Vec::with_capacity(SAMPLES);
        let mut legacy_total: Vec<u128> = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let started = Instant::now();
            let digest = world.direct_state_digest(limits).expect("direct digest");
            direct.push(started.elapsed().as_nanos());
            // 同一样本连续 capture → digest，快照析构不计入任一段。
            let total_started = Instant::now();
            let captured = world.capture_snapshot().expect("capture");
            let capture_ns = total_started.elapsed().as_nanos();
            let digest_started = Instant::now();
            let legacy = laneflow_runtime::deterministic_state_digest(&captured).expect("legacy");
            let digest_ns = digest_started.elapsed().as_nanos();
            let total_ns = total_started.elapsed().as_nanos();
            assert!(total_ns >= capture_ns + digest_ns);
            capture.push(capture_ns);
            legacy_digest.push(digest_ns);
            legacy_total.push(total_ns);
            assert_eq!(digest, legacy);
        }
        let direct_median = median(&direct);
        let capture_median = median(&capture);
        let legacy_digest_median = median(&legacy_digest);
        let legacy_total_median = median(&legacy_total);
        println!(
            "#987-evidence direct-digest smoke={} tier={tier} entities={tier} vehicles={vehicles} \
             routes={routes} direct_median_ns={direct_median} capture_median_ns={capture_median} \
             legacy_digest_median_ns={legacy_digest_median} direct_samples={direct:?} \
             capture_samples={capture:?} legacy_digest_samples={legacy_digest:?} \
             legacy_total_median_ns={legacy_total_median} legacy_total_samples={legacy_total:?}",
            synthetic_network::smoke_mode(),
        );
    }
}

/// 在用根审计对并发 step 的干扰：主线程 tick 计时（无审计基线 vs
/// 认证工作线程并发），对照模板取自 snapshot 干扰证据测试；末尾以
/// 确定性摘要断言并发不改变 tick 语义。
#[test]
#[ignore = "manual single-thread release wall-clock evidence; run inside a quiet window"]
fn concurrent_attest_tick_interference_evidence() {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    let tier = if synthetic_network::smoke_mode() {
        1_000
    } else {
        100_000
    };
    let vehicles = tier / 10;
    let routes = vehicles.min(WorkloadShape::for_stable_entities(tier).synthetic_tiles);
    let fixture = build_tier(tier);
    let make_world = || {
        let mut world = TrafficWorld::install(
            Arc::clone(&fixture.root),
            WorldConfig::new(vehicles, routes, u64::from(routes) * 3 + 1_024, 1_024, 4),
            ExecutionConfig::new(std::num::NonZeroU32::MIN),
            CommittedNetworkSource::Published {
                reference: fixture.reference.clone(),
            },
            987_100,
            test_policy::selection(&fixture.root),
        )
        .expect("install");
        synthetic_network::populate(&mut world, vehicles, routes);
        world
    };
    let mut baseline_world = make_world();
    let mut contended_world = make_world();
    // 4 ms 步长避免短链车辆在测量期间全部离场；两世界同初态、同输入、同拍数。
    let input = TickInput::new(4);
    for _ in 0..16 {
        baseline_world.step(input).expect("warmup");
        contended_world.step(input).expect("warmup");
    }
    let tick_clocks = |world: &mut TrafficWorld| {
        let mut clocks = Vec::with_capacity(CONCURRENT_TICKS);
        for _ in 0..CONCURRENT_TICKS {
            let started = Instant::now();
            world.step(input).expect("step");
            clocks.push(started.elapsed().as_nanos());
        }
        clocks
    };
    let baseline = tick_clocks(&mut baseline_world);
    let stop = AtomicBool::new(false);
    let cycles = AtomicU64::new(0);
    let barrier = std::sync::Barrier::new(2);
    let (contended, overlap_ns, overlap_cycles, total_cycles) = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            barrier.wait();
            while !stop.load(Ordering::Acquire) {
                assert!(
                    matches!(attest_once(&fixture), AttestationOutcome::Concluded(report)
                    if report.verdict == AttestationVerdict::Pass)
                );
                cycles.fetch_add(1, Ordering::Release);
            }
        });
        barrier.wait();
        // unwind 时同样通知工作线程退出，避免 scope join 永久等待。
        struct StopOnDrop<'a>(&'a AtomicBool);
        impl Drop for StopOnDrop<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let guard = StopOnDrop(&stop);
        let started = Instant::now();
        let clocks = tick_clocks(&mut contended_world);
        let overlap_ns = started.elapsed().as_nanos();
        let overlap_cycles = cycles.load(Ordering::Acquire);
        drop(guard);
        worker.join().expect("attest worker");
        (
            clocks,
            overlap_ns,
            overlap_cycles,
            cycles.load(Ordering::Acquire),
        )
    });
    assert!(
        overlap_cycles > 0,
        "no complete attestation during tick window"
    );
    let limits = SnapshotDigestLimits::new(DIGEST_LIMIT_BYTES);
    assert_eq!(
        baseline_world.direct_state_digest(limits).unwrap(),
        contended_world.direct_state_digest(limits).unwrap()
    );
    let remaining = contended_world.capture_snapshot().unwrap().vehicles().len();
    assert_eq!(
        remaining, vehicles as usize,
        "measured population must remain loaded"
    );
    let baseline_median = median(&baseline);
    let contended_median = median(&contended);
    let baseline_p95 = percentile95(&baseline);
    let contended_p95 = percentile95(&contended);
    let signed_ppm = |measured: u128, reference: u128| {
        (measured as i128 - reference as i128) * 1_000_000 / reference.max(1) as i128
    };
    println!(
        "#987-evidence concurrent-interference smoke={} tier={tier} vehicles={vehicles} routes={routes} \
         ticks={CONCURRENT_TICKS} tick_ms=4 overlap_ns={overlap_ns} overlap_cycles={overlap_cycles} total_cycles={total_cycles} \
         baseline_median_ns={baseline_median} contended_median_ns={contended_median} \
         baseline_p95_ns={baseline_p95} contended_p95_ns={contended_p95} \
         interference_ppm={} interference_p95_ppm={}",
        synthetic_network::smoke_mode(),
        signed_ppm(contended_median, baseline_median),
        signed_ppm(contended_p95, baseline_p95),
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
