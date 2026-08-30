use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use laneflow_compiler::{
    CanonicalFrameInput, CompilationUnitBuilder, CompileLimits, Compiler, LaneEdgeInput,
    LaneEdgeReference, PortableDiffBase, PortableEmissionProvenance, SourceModuleHeader,
    SourceModuleHeaderInput, SyntheticModuleBuilder, check_portable_candidate,
    emit_portable_candidate_to_staging,
};
use laneflow_format::{FormatLimits, preflight_object_values};
use laneflow_static_contract::{EntityKind, LaneEdgeOrdinal, PortableObjectKind};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1024 * 1024, 16 * 1024 * 1024);
static NEXT_STAGING_DIRECTORY: AtomicU64 = AtomicU64::new(0);

fn build_compiler_output(
    configure: impl FnOnce(&mut SyntheticModuleBuilder),
    spatial: SpatialBuildOption,
    artifact_base: bool,
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
    let staging_directory = std::env::temp_dir().join(format!(
        "laneflow-compiler-shared-network-{}-{}",
        std::process::id(),
        NEXT_STAGING_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&staging_directory).expect("staging directory");
    let genesis = emit_portable_candidate_to_staging(
        &output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
        &staging_directory,
    )
    .expect("portable candidate");
    let candidate = if artifact_base {
        let base = preflight_object_values(
            genesis.canonical_artifact().bytes(),
            PortableObjectKind::CanonicalArtifact,
            FormatLimits::HARD,
        )
        .expect("artifact base");
        let artifact = emit_portable_candidate_to_staging(
            &output,
            &provenance,
            FormatLimits::HARD,
            PortableDiffBase::Artifact(base),
            &staging_directory,
        )
        .expect("artifact-base portable candidate");
        drop(genesis);
        artifact
    } else {
        genesis
    };
    assert!(candidate.canonical_artifact().is_file_backed());
    assert!(candidate.source_map().is_file_backed());
    assert!(candidate.semantic_diff().is_file_backed());
    let live_entries = fs::read_dir(&staging_directory).unwrap().count();
    #[cfg(windows)]
    assert_eq!(live_entries, 3);
    #[cfg(not(windows))]
    assert_eq!(live_entries, 0);
    let repeated = check_portable_candidate(candidate.clone(), FormatLimits::HARD)
        .expect("repeated post-emission checked bundle");
    let checked = check_portable_candidate(candidate, FormatLimits::HARD)
        .expect("post-emission checked bundle");
    assert_eq!(
        checked.canonical_artifact_view().bytes().as_ptr(),
        repeated.canonical_artifact_view().bytes().as_ptr()
    );
    assert_eq!(
        checked.canonical_artifact_digest(),
        repeated.canonical_artifact_digest()
    );
    assert_eq!(checked.network_revision(), repeated.network_revision());
    drop(repeated);
    let revision = build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(spatial, BUILD_LIMITS),
    )
    .expect("shared network revision");
    fs::remove_dir(staging_directory).expect("remove empty staging directory");
    revision
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
        false,
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
        false,
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

#[test]
fn compiler_artifact_base_file_backing_reaches_shared_root() {
    let revision = build_compiler_output(
        |module| {
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "only",
                    length_meters: 15.0,
                    speed_limit_meters_per_second: 8.0,
                    successors: &[],
                })
                .expect("lane edge");
        },
        SpatialBuildOption::Omit,
        true,
    );

    assert_eq!(revision.traffic().lane_edge_count(), 1);
    assert_eq!(revision.traffic().lane_lengths_millimetres(), &[15_000]);
}
