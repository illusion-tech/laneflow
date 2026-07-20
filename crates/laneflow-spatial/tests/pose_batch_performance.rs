use std::{
    cell::Cell,
    hint::black_box,
    mem::size_of,
    time::{Duration, Instant},
};

use laneflow_spatial::{CanonicalPoseRecordF32, FramePlacementToken};

#[path = "support/pose_batch_scale.rs"]
mod scale_support;

use scale_support::{
    F64Batch, F64PoseRecord, F64Scratch, ONE_HUNDRED_THOUSAND, RuntimeFixture, TEN_THOUSAND,
};

const ROUNDS: usize = 5;
const SAMPLES_PER_ROUND: usize = 80;
const WARM_UP_ITERATIONS: usize = 8;
const TEN_K_P95_LIMIT: Duration = Duration::from_millis(2);
const ONE_HUNDRED_K_P95_LIMIT: Duration = Duration::from_millis(20);
const MAX_SCALE_RATIO: f64 = 12.0;
const MAX_F32_OVER_F64_RATIO: f64 = 1.05;
const MIN_RETAINED_REDUCTION: f64 = 0.25;

struct ScaleResult {
    count: usize,
    f32_p95_ns: u128,
    f64_p95_ns: u128,
    paired_ratio: f64,
    f32_retained_bytes: usize,
    f64_retained_bytes: usize,
    retained_reduction: f64,
}

struct PairedRound {
    f32_p95_ns: u128,
    f64_p95_ns: u128,
    f32_median_ns: u128,
    f64_median_ns: u128,
}

#[test]
#[ignore = "wall-clock gate runs only on the documented fixed performance machine"]
fn fixed_machine_pose_batch_p95_and_retained_memory_gate() {
    let fixture = RuntimeFixture::new();
    let candidate = fixture.f64_candidate();
    let mut results = Vec::new();

    for count in [TEN_THOUSAND, ONE_HUNDRED_THOUSAND] {
        let inputs = fixture.inputs(count);
        let mut f32_output = fixture.output(count);
        let mut f32_scratch = fixture.scratch(count);
        let mut f64_output = F64Batch::with_capacity(
            fixture.spatial.frame_id().clone(),
            FramePlacementToken::new(0),
            count,
        );
        let mut f64_scratch = F64Scratch::with_capacity(count);
        let token = Cell::new(1_u64);

        for _ in 0..WARM_UP_ITERATIONS {
            fixture
                .spatial
                .extract_pose_batch(
                    &fixture.parking,
                    FramePlacementToken::new(token.get()),
                    &inputs,
                    &mut f32_output,
                    &mut f32_scratch,
                )
                .expect("f32 warm-up extraction");
            candidate
                .extract(
                    FramePlacementToken::new(token.get()),
                    &inputs,
                    &mut f64_output,
                    &mut f64_scratch,
                )
                .expect("f64 warm-up extraction");
            token.set(token.get() + 1);
        }

        let mut f32_rounds = Vec::with_capacity(ROUNDS);
        let mut f64_rounds = Vec::with_capacity(ROUNDS);
        let mut paired_ratios = Vec::with_capacity(ROUNDS);
        for round in 0..ROUNDS {
            let mut measure_f32 = || {
                fixture
                    .spatial
                    .extract_pose_batch(
                        &fixture.parking,
                        FramePlacementToken::new(token.get()),
                        black_box(&inputs),
                        black_box(&mut f32_output),
                        black_box(&mut f32_scratch),
                    )
                    .expect("f32 measured extraction");
                black_box(f32_output.records());
                token.set(token.get() + 1);
            };
            let mut measure_f64 = || {
                candidate
                    .extract(
                        FramePlacementToken::new(token.get()),
                        black_box(&inputs),
                        black_box(&mut f64_output),
                        black_box(&mut f64_scratch),
                    )
                    .expect("f64 measured extraction");
                black_box(f64_output.records());
                token.set(token.get() + 1);
            };

            let measured = measure_paired_p95(round, &mut measure_f32, &mut measure_f64);
            let paired_throughput_ratio =
                measured.f32_median_ns as f64 / measured.f64_median_ns as f64;
            println!(
                "SPATIAL_PERF_ROUND count={count} round={} first={} f32_p95_ns={} f64_p95_ns={} f32_median_ns={} f64_median_ns={} paired_throughput_ratio={paired_throughput_ratio:.6}",
                round + 1,
                if round.is_multiple_of(2) {
                    "f32"
                } else {
                    "f64"
                },
                measured.f32_p95_ns,
                measured.f64_p95_ns,
                measured.f32_median_ns,
                measured.f64_median_ns,
            );
            f32_rounds.push(measured.f32_p95_ns);
            f64_rounds.push(measured.f64_p95_ns);
            paired_ratios.push(paired_throughput_ratio);
        }

        let f32_p95_ns = median_u128(&mut f32_rounds);
        let f64_p95_ns = median_u128(&mut f64_rounds);
        let paired_ratio = median_f64(&mut paired_ratios);
        let f32_retained_bytes =
            (f32_output.capacity() + f32_scratch.capacity()) * size_of::<CanonicalPoseRecordF32>();
        let f64_retained_bytes =
            (f64_output.capacity() + f64_scratch.capacity()) * size_of::<F64PoseRecord>();
        let retained_reduction = 1.0 - f32_retained_bytes as f64 / f64_retained_bytes as f64;

        assert_eq!(f32_output.len(), count);
        assert_eq!(f64_output.records().len(), count);
        assert!(f32_output.records().iter().all(|record| {
            let pose = record.pose();
            [
                pose.position().x(),
                pose.position().y(),
                pose.position().z(),
                pose.tangent().x(),
                pose.tangent().y(),
                pose.tangent().z(),
                pose.up().x(),
                pose.up().y(),
                pose.up().z(),
            ]
            .into_iter()
            .all(f32::is_finite)
        }));
        assert!(
            retained_reduction >= MIN_RETAINED_REDUCTION,
            "{count}: retained reduction {retained_reduction:.6}"
        );

        println!(
            "SPATIAL_PERF count={count} f32_p95_ns={f32_p95_ns} f64_p95_ns={f64_p95_ns} paired_throughput_ratio={paired_ratio:.6} f32_retained_bytes={f32_retained_bytes} f64_retained_bytes={f64_retained_bytes} retained_reduction={retained_reduction:.6}"
        );
        results.push(ScaleResult {
            count,
            f32_p95_ns,
            f64_p95_ns,
            paired_ratio,
            f32_retained_bytes,
            f64_retained_bytes,
            retained_reduction,
        });
    }

    let ten_k = results
        .iter()
        .find(|result| result.count == TEN_THOUSAND)
        .expect("10k result");
    let one_hundred_k = results
        .iter()
        .find(|result| result.count == ONE_HUNDRED_THOUSAND)
        .expect("100k result");
    let scale_ratio = one_hundred_k.f32_p95_ns as f64 / ten_k.f32_p95_ns as f64;
    println!("SPATIAL_PERF scale_ratio={scale_ratio:.6}");

    // Keep every evidence field observable so accidental report omissions are caught by lints.
    black_box((
        ten_k.f64_p95_ns,
        ten_k.f32_retained_bytes,
        ten_k.f64_retained_bytes,
        ten_k.retained_reduction,
        one_hundred_k.f64_p95_ns,
        one_hundred_k.f32_retained_bytes,
        one_hundred_k.f64_retained_bytes,
        one_hundred_k.retained_reduction,
    ));

    if std::env::var_os("LANEFLOW_SPATIAL_PERF_GATE").is_some() {
        assert!(
            ten_k.f32_p95_ns <= TEN_K_P95_LIMIT.as_nanos(),
            "10k p95 exceeded 2 ms: {} ns",
            ten_k.f32_p95_ns
        );
        assert!(
            one_hundred_k.f32_p95_ns <= ONE_HUNDRED_K_P95_LIMIT.as_nanos(),
            "100k p95 exceeded 20 ms: {} ns",
            one_hundred_k.f32_p95_ns
        );
        assert!(
            scale_ratio <= MAX_SCALE_RATIO,
            "10k -> 100k ratio exceeded 12x: {scale_ratio:.6}"
        );
        assert!(
            ten_k.paired_ratio <= MAX_F32_OVER_F64_RATIO,
            "10k f32/f64 ratio exceeded 1.05: {:.6}",
            ten_k.paired_ratio
        );
        assert!(
            one_hundred_k.paired_ratio <= MAX_F32_OVER_F64_RATIO,
            "100k f32/f64 ratio exceeded 1.05: {:.6}",
            one_hundred_k.paired_ratio
        );
    }
}

fn measure_paired_p95(
    round: usize,
    f32_operation: &mut impl FnMut(),
    f64_operation: &mut impl FnMut(),
) -> PairedRound {
    let mut f32_samples = Vec::with_capacity(SAMPLES_PER_ROUND);
    let mut f64_samples = Vec::with_capacity(SAMPLES_PER_ROUND);
    for sample in 0..SAMPLES_PER_ROUND {
        if (round + sample).is_multiple_of(2) {
            f32_samples.push(measure_once(f32_operation));
            f64_samples.push(measure_once(f64_operation));
        } else {
            f64_samples.push(measure_once(f64_operation));
            f32_samples.push(measure_once(f32_operation));
        }
    }
    f32_samples.sort_unstable();
    f64_samples.sort_unstable();
    let p95_index = (SAMPLES_PER_ROUND * 95).div_ceil(100) - 1;
    PairedRound {
        f32_p95_ns: f32_samples[p95_index],
        f64_p95_ns: f64_samples[p95_index],
        f32_median_ns: f32_samples[SAMPLES_PER_ROUND / 2],
        f64_median_ns: f64_samples[SAMPLES_PER_ROUND / 2],
    }
}

fn measure_once(operation: &mut impl FnMut()) -> u128 {
    let started = Instant::now();
    operation();
    started.elapsed().as_nanos()
}

fn median_u128(values: &mut [u128]) -> u128 {
    values.sort_unstable();
    values[values.len() / 2]
}

fn median_f64(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}
