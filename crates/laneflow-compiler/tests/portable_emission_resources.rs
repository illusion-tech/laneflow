use std::{alloc::System, hint::black_box, time::Instant};

use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, LaneEdgeInput, LaneEdgeReference,
    PortableDiffBase, PortableEmissionProvenanceV1, SourceModuleHeader, SourceModuleHeaderInput,
    SyntheticModuleBuilder, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, preflight_object_values_v1};
use laneflow_static_contract::PortableObjectKind;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const EDGE_COUNT: u32 = 1_024;

fn compile_native_chain() -> laneflow_compiler::CompilationOutput {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "city/portable-resource-probe",
            source_document_key: "portable-resource-probe.document",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: Some(42),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .unwrap();
    let mut module = SyntheticModuleBuilder::new(header, &limits).unwrap();
    for ordinal in 0..EDGE_COUNT {
        let key = format!("edge-{ordinal:04}");
        if ordinal + 1 < EDGE_COUNT {
            let successor_key = format!("edge-{:04}", ordinal + 1);
            let successors = [LaneEdgeReference::local(&successor_key)];
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: &key,
                    length_meters: 10.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &successors,
                })
                .unwrap();
        } else {
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: &key,
                    length_meters: 10.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[],
                })
                .unwrap();
        }
    }

    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().unwrap()).unwrap();
    Compiler::new().compile(unit.build().unwrap()).unwrap()
}

#[derive(Clone, Copy, Debug)]
struct Measurement {
    elapsed_ns: u128,
    stats: Stats,
}

impl Measurement {
    fn live_delta_bytes(self) -> i128 {
        self.stats.bytes_allocated as i128 - self.stats.bytes_deallocated as i128
    }
}

fn measure<T>(operation: impl FnOnce() -> T) -> (T, Measurement) {
    let region = Region::new(GLOBAL);
    let started = Instant::now();
    let output = operation();
    let elapsed_ns = started.elapsed().as_nanos();
    black_box(&output);
    let stats = black_box(region.change());
    (output, Measurement { elapsed_ns, stats })
}

fn staging_bytes(candidate: &laneflow_compiler::PortablePublicationCandidate) -> u64 {
    candidate.canonical_artifact().byte_length()
        + candidate.source_map().byte_length()
        + candidate.semantic_diff().byte_length()
}

fn print_measurement(mode: &str, measurement: Measurement, staging_bytes: u64) {
    println!(
        "portable-emission-resource mode={mode} workload=native-chain edges={EDGE_COUNT} elapsed_ns={} allocations={} reallocations={} allocated_bytes={} deallocated_bytes={} reallocated_delta_bytes={} live_delta_bytes={} staging_exact_bytes={staging_bytes}",
        measurement.elapsed_ns,
        measurement.stats.allocations,
        measurement.stats.reallocations,
        measurement.stats.bytes_allocated,
        measurement.stats.bytes_deallocated,
        measurement.stats.bytes_reallocated,
        measurement.live_delta_bytes(),
    );
}

#[test]
#[ignore = "manual single-thread release allocation and wall-clock evidence"]
fn portable_emitter_reports_genesis_and_artifact_resource_metrics() {
    let output = compile_native_chain();
    let provenance = PortableEmissionProvenanceV1::try_new("laneflow-resource-probe-v1").unwrap();

    let (genesis, genesis_measurement) = measure(|| {
        emit_portable_candidate(
            &output,
            &provenance,
            FormatLimits::V1_HARD,
            PortableDiffBase::Genesis,
        )
        .unwrap()
    });
    let genesis_staging = staging_bytes(&genesis);
    assert!(genesis_measurement.stats.bytes_allocated as u64 >= genesis_staging);
    print_measurement("genesis", genesis_measurement, genesis_staging);

    let base = preflight_object_values_v1(
        genesis.canonical_artifact().bytes(),
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::V1_HARD,
    )
    .unwrap();
    let (artifact, artifact_measurement) = measure(|| {
        emit_portable_candidate(
            &output,
            &provenance,
            FormatLimits::V1_HARD,
            PortableDiffBase::Artifact(base),
        )
        .unwrap()
    });
    let artifact_staging = staging_bytes(&artifact);
    assert!(artifact_measurement.stats.bytes_allocated as u64 >= artifact_staging);
    assert_eq!(
        artifact.canonical_artifact().bytes(),
        genesis.canonical_artifact().bytes()
    );
    assert!(artifact.semantic_diff().byte_length() < genesis.semantic_diff().byte_length());
    print_measurement("artifact-noop", artifact_measurement, artifact_staging);
}
