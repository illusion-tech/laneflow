use std::sync::Arc;

use laneflow_compiler::{
    CanonicalFrameInput, CompilationUnitBuilder, CompileLimits, Compiler, LaneEdgeInput,
    LaneEdgeReference, PortableDiffBase, PortableEmissionProvenance, SourceModuleHeader,
    SourceModuleHeaderInput, SyntheticModuleBuilder, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_static_contract::{EntityKind, LaneEdgeOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1024 * 1024, 16 * 1024 * 1024);

fn build_compiler_output(
    configure: impl FnOnce(&mut SyntheticModuleBuilder),
    spatial: SpatialBuildOption,
) -> Arc<SharedNetworkRevision> {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "city/static-network-test",
            source_document_key: "static-network-test.document",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: Some(42),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .expect("source header");
    let mut module = SyntheticModuleBuilder::new(header, &limits).expect("synthetic module");
    configure(&mut module);
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().expect("finished module"))
        .expect("compilation module");
    let output = Compiler::new()
        .compile(unit.build().expect("compilation unit"))
        .expect("compiled output");
    let provenance = PortableEmissionProvenance::try_new("laneflow-static-network-test-v1")
        .expect("portable provenance");
    let candidate = emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .expect("portable candidate");
    let checked = check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .expect("post-emission checked bundle");
    build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(spatial, BUILD_LIMITS),
    )
    .expect("shared network revision")
}

#[test]
fn compiler_frame_only_candidate_reaches_spatial_without_lane_pose() {
    let revision = build_compiler_output(
        |module| {
            module
                .add_canonical_frame(CanonicalFrameInput {
                    canonical_frame_key: "frame-main",
                    lane_edge_geometries: &[],
                })
                .expect("canonical frame");
        },
        SpatialBuildOption::RetainAvailable,
    );

    assert_eq!(
        revision.identity().entity_count(EntityKind::CanonicalFrame),
        1
    );
    assert_eq!(revision.traffic().lane_edge_count(), 0);
    let spatial = revision.spatial().expect("frame-only spatial component");
    assert!(spatial.lane_pose().is_none());
    assert_eq!(spatial.facility_geometry_count(), 0);
    assert_eq!(spatial.direction_profile(), 0);
}

#[test]
fn compiler_fan_in_candidate_builds_exact_reverse_csr() {
    let revision = build_compiler_output(
        |module| {
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "left",
                    length_meters: 11.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[LaneEdgeReference::local("merge")],
                })
                .expect("left lane")
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "right",
                    length_meters: 12.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[LaneEdgeReference::local("merge")],
                })
                .expect("right lane")
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "merge",
                    length_meters: 13.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[],
                })
                .expect("merge lane");
        },
        SpatialBuildOption::Omit,
    );
    let ordinal_for_length = |length| {
        let index = revision
            .traffic()
            .lane_lengths_millimetres()
            .iter()
            .position(|actual| *actual == length)
            .expect("fixture lane length");
        LaneEdgeOrdinal::try_from_usize(index).expect("fixture lane ordinal")
    };
    let left = ordinal_for_length(11_000);
    let right = ordinal_for_length(12_000);
    let merge = ordinal_for_length(13_000);

    assert_eq!(revision.traffic().successors(left), Some(&[merge][..]));
    assert_eq!(revision.traffic().successors(right), Some(&[merge][..]));
    let mut expected_predecessors = [left, right];
    expected_predecessors.sort_unstable_by_key(|ordinal| ordinal.raw());
    assert_eq!(
        revision.traffic().predecessors(merge),
        Some(&expected_predecessors[..])
    );
    assert_eq!(
        revision.planning_hints().edge_boundary_weights()[left.index()],
        1
    );
    assert_eq!(
        revision.planning_hints().edge_boundary_weights()[right.index()],
        1
    );
    assert_eq!(
        revision.planning_hints().edge_boundary_weights()[merge.index()],
        2
    );
}
