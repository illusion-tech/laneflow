use std::{hint::black_box, time::Duration};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

#[path = "../tests/support/presentation_scale.rs"]
mod scale_support;

use scale_support::{ONE_HUNDRED_THOUSAND, PresentationScaleFixture, TEN_THOUSAND};

fn benchmark_presentation(criterion: &mut Criterion) {
    for count in [TEN_THOUSAND, ONE_HUNDRED_THOUSAND] {
        let mut fixture = PresentationScaleFixture::new(count);
        let mut group = criterion.benchmark_group("bevy_post_update_presentation");
        group.sample_size(if count == TEN_THOUSAND { 20 } else { 10 });
        group.warm_up_time(Duration::from_secs(1));
        group.measurement_time(Duration::from_secs(if count == TEN_THOUSAND {
            5
        } else {
            10
        }));
        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(BenchmarkId::new("production", count), |benchmark| {
            benchmark.iter(|| {
                black_box(&mut fixture).run_post_update();
                black_box(fixture.presentation_report())
            });
        });
        group.finish();
    }
}

criterion_group!(benches, benchmark_presentation);
criterion_main!(benches);
