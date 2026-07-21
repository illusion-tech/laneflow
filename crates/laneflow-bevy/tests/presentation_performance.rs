use std::{
    hint::black_box,
    time::{Duration, Instant},
};

#[path = "support/presentation_scale.rs"]
mod scale_support;

use scale_support::{ONE_HUNDRED_THOUSAND, PresentationScaleFixture, TEN_THOUSAND};

const ROUNDS: usize = 5;
const SAMPLES_PER_ROUND: usize = 80;
const WARM_UP_ITERATIONS: usize = 8;
const TEN_K_P95_LIMIT: Duration = Duration::from_millis(4);
const ONE_HUNDRED_K_P95_LIMIT: Duration = Duration::from_millis(40);
const MAX_SCALE_RATIO: f64 = 12.0;

struct ScaleResult {
    count: usize,
    p95_ns: u128,
}

#[test]
#[ignore = "wall-clock gate runs only on the documented fixed performance machine"]
fn fixed_machine_post_update_p95_gate() {
    let mut results = Vec::new();

    for count in [TEN_THOUSAND, ONE_HUNDRED_THOUSAND] {
        let mut fixture = PresentationScaleFixture::new(count);
        for _ in 0..WARM_UP_ITERATIONS {
            fixture.run_post_update();
        }

        let mut round_p95 = Vec::with_capacity(ROUNDS);
        for round in 0..ROUNDS {
            let mut samples = Vec::with_capacity(SAMPLES_PER_ROUND);
            for _ in 0..SAMPLES_PER_ROUND {
                let started = Instant::now();
                black_box(&mut fixture).run_post_update();
                samples.push(started.elapsed().as_nanos());
            }
            samples.sort_unstable();
            let p95_index = (SAMPLES_PER_ROUND * 95).div_ceil(100) - 1;
            let p95_ns = samples[p95_index];
            let median_ns = samples[SAMPLES_PER_ROUND / 2];
            println!(
                "BEVY_PERF_ROUND count={count} round={} p95_ns={p95_ns} median_ns={median_ns}",
                round + 1
            );
            round_p95.push(p95_ns);
        }

        let p95_ns = median_u128(&mut round_p95);
        fixture.assert_presented(count);
        println!("BEVY_PERF count={count} p95_ns={p95_ns}");
        results.push(ScaleResult { count, p95_ns });
    }

    let ten_k = results
        .iter()
        .find(|result| result.count == TEN_THOUSAND)
        .expect("10k result");
    let one_hundred_k = results
        .iter()
        .find(|result| result.count == ONE_HUNDRED_THOUSAND)
        .expect("100k result");
    let scale_ratio = one_hundred_k.p95_ns as f64 / ten_k.p95_ns as f64;
    println!("BEVY_PERF scale_ratio={scale_ratio:.6}");

    if std::env::var_os("LANEFLOW_BEVY_PERF_GATE").is_some() {
        assert!(
            ten_k.p95_ns <= TEN_K_P95_LIMIT.as_nanos(),
            "10k p95 exceeded 4 ms: {} ns",
            ten_k.p95_ns
        );
        assert!(
            one_hundred_k.p95_ns <= ONE_HUNDRED_K_P95_LIMIT.as_nanos(),
            "100k p95 exceeded 40 ms: {} ns",
            one_hundred_k.p95_ns
        );
        assert!(
            scale_ratio <= MAX_SCALE_RATIO,
            "10k -> 100k ratio exceeded 12x: {scale_ratio:.6}"
        );
    }
}

fn median_u128(values: &mut [u128]) -> u128 {
    values.sort_unstable();
    values[values.len() / 2]
}
