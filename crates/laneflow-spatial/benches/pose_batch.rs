use std::{cell::Cell, hint::black_box, time::Duration};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use laneflow_spatial::{FramePlacementToken, PoseSource};

#[path = "../tests/support/pose_batch_scale.rs"]
mod scale_support;

use scale_support::{F64Batch, F64Scratch, ONE_HUNDRED_THOUSAND, RuntimeFixture, TEN_THOUSAND};

fn benchmark_pose_batch(criterion: &mut Criterion) {
    let fixture = RuntimeFixture::new();
    let candidate = fixture.f64_candidate();

    for count in [TEN_THOUSAND, ONE_HUNDRED_THOUSAND] {
        let inputs = fixture.inputs(count);
        let token = Cell::new(1_u64);
        let mut f32_output = fixture.output(count);
        let mut f32_scratch = fixture.scratch(count);
        let mut f64_output = F64Batch::with_capacity(
            fixture.spatial.frame_id().clone(),
            FramePlacementToken::new(0),
            count,
        );
        let mut f64_scratch = F64Scratch::with_capacity(count);
        let mut group = criterion.benchmark_group("spatial_pose_batch_extract");
        group.sample_size(if count == TEN_THOUSAND { 20 } else { 10 });
        group.warm_up_time(Duration::from_secs(1));
        group.measurement_time(Duration::from_secs(if count == TEN_THOUSAND {
            5
        } else {
            10
        }));
        group.throughput(Throughput::Elements(count as u64));

        group.bench_function(BenchmarkId::new("production_f32", count), |benchmark| {
            benchmark.iter(|| {
                fixture
                    .spatial
                    .extract_pose_batch(
                        &fixture.parking,
                        FramePlacementToken::new(token.get()),
                        black_box(&inputs),
                        black_box(&mut f32_output),
                        black_box(&mut f32_scratch),
                    )
                    .expect("valid production benchmark batch");
                token.set(token.get() + 1);
                black_box(f32_output.records().last().copied())
            });
        });
        group.bench_function(BenchmarkId::new("same_layout_f64", count), |benchmark| {
            benchmark.iter(|| {
                candidate
                    .extract(
                        FramePlacementToken::new(token.get()),
                        black_box(&inputs),
                        black_box(&mut f64_output),
                        black_box(&mut f64_scratch),
                    )
                    .expect("valid f64 benchmark batch");
                token.set(token.get() + 1);
                black_box(f64_output.records().last().copied())
            });
        });
        group.finish();
    }

    let diagnostic_inputs = fixture.inputs(ONE_HUNDRED_THOUSAND);
    let lane_progress: Vec<_> = diagnostic_inputs
        .iter()
        .map(|input| match input.source() {
            PoseSource::Lane { progress, .. } => progress,
            PoseSource::Parking { .. } => unreachable!("scale fixture contains lane records"),
            _ => unreachable!("scale fixture uses a frozen source variant"),
        })
        .collect();
    let slot = candidate
        .edge_slot(fixture.edge)
        .expect("diagnostic edge slot");
    let mut diagnostics = criterion.benchmark_group("spatial_lookup_diagnostics_100k");
    diagnostics.sample_size(20);
    diagnostics.warm_up_time(Duration::from_secs(1));
    diagnostics.measurement_time(Duration::from_secs(5));
    diagnostics.throughput(Throughput::Elements(ONE_HUNDRED_THOUSAND as u64));
    diagnostics.bench_function("edge_handle_to_slot_same_structure", |benchmark| {
        benchmark.iter(|| {
            for input in black_box(&diagnostic_inputs) {
                black_box(candidate.edge_slot(match input.source() {
                    PoseSource::Lane { edge, .. } => edge,
                    PoseSource::Parking { .. } => unreachable!(),
                    _ => unreachable!(),
                }));
            }
        });
    });
    diagnostics.bench_function("slot_resolved_f64_sampling", |benchmark| {
        benchmark.iter(|| {
            for progress in black_box(&lane_progress) {
                black_box(
                    candidate
                        .sample_resolved(slot, *progress)
                        .expect("valid slot-resolved diagnostic sample"),
                );
            }
        });
    });
    diagnostics.finish();
}

criterion_group!(benches, benchmark_pose_batch);
criterion_main!(benches);
