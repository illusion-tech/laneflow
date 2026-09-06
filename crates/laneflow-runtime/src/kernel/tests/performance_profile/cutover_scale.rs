//! #531 固定规模夹具；只生成输入，不计时或计数分配。

use std::sync::Arc;

use super::runtime_types::{
    CommittedNetworkSource, PublishedLfcaReference, RouteRegisterInput, SemanticDiffOriginBinding,
    TrafficWorld, VehicleSpawnInput, WorldConfig, WorldPolicySelection,
};
use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, IidmVehicleProfileInput, LaneEdgeInput,
    ParticipantClassInput, ParticipantClassReference, PortableDiffBase, PortableEmissionProvenance,
    SourceModuleHeader, SourceModuleHeaderInput, SyntheticModuleBuilder, VehicleProfileInput,
    emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle, preflight_object_values};
use laneflow_static_contract::{
    ExactByteLength, LaneEdgeOrdinal, PortableObjectKind, SEMANTIC_DIFF_FORMAT_VERSION,
    Sha256Digest, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use sha2::Digest as _;

pub struct Revisions {
    pub base: Arc<SharedNetworkRevision>,
    pub target: Arc<SharedNetworkRevision>,
    pub diff: Vec<u8>,
    pub diff_binding: SemanticDiffOriginBinding,
}

pub fn revisions() -> Revisions {
    let compile = |speed_limit| {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: "runtime/cutover-scale",
                source_document_key: "cutover.document",
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x53; 32],
                frontend_options_digest: [0x31; 32],
                random_seed: Some(531),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .expect("header");
        let mut module = SyntheticModuleBuilder::new(header, &limits).expect("module");
        module
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "road-user",
                extends: None,
            })
            .expect("class")
            .add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: "car",
                participant_class: ParticipantClassReference::local("road-user"),
                iidm: IidmVehicleProfileInput {
                    length_meters: 4.5,
                    desired_speed_meters_per_second: 13.75,
                    min_gap_meters: 2.0,
                    time_headway_seconds: 1.4,
                    max_acceleration_meters_per_second_squared: 1.8,
                    comfortable_deceleration_meters_per_second_squared: 2.0,
                    emergency_deceleration_meters_per_second_squared: 4.5,
                },
            })
            .expect("profile");
        for edge in 0..256 {
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: &format!("edge-{edge}"),
                    length_meters: 8_000.0,
                    speed_limit_meters_per_second: speed_limit,
                    successors: &[],
                })
                .expect("edge");
        }
        let mut unit = CompilationUnitBuilder::new(limits);
        unit.add_synthetic_module(module.finish().expect("module"))
            .expect("unit module");
        Compiler::new()
            .compile(unit.build().expect("unit"))
            .expect("compile")
    };
    let provenance =
        PortableEmissionProvenance::try_new("runtime-cutover-scale-v1").expect("provenance");
    let base_output = compile(15.0);
    let target_output = compile(16.0);
    let base = emit_portable_candidate(
        &base_output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .expect("base artifact");
    let values = preflight_object_values(
        base.canonical_artifact().bytes(),
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .expect("base values");
    let target = emit_portable_candidate(
        &target_output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Artifact(values),
    )
    .expect("target artifact");
    let mut roots = Vec::new();
    for artifact in [&base, &target] {
        let checked = check_post_emission_bundle(
            artifact.canonical_artifact().bytes(),
            artifact.source_map().bytes(),
            artifact.semantic_diff().bytes(),
            artifact.expected_semantic_diff_base(),
            FormatLimits::HARD,
        )
        .expect("checked bundle");
        roots.push(
            build_shared_network_revision(
                checked.canonical_network_input(),
                SharedNetworkBuildOptions::new(
                    SpatialBuildOption::Omit,
                    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
                ),
            )
            .expect("root"),
        );
    }
    let diff = target.semantic_diff().bytes().to_vec();
    let diff_binding = SemanticDiffOriginBinding::new(
        SEMANTIC_DIFF_FORMAT_VERSION,
        Sha256Digest::from_bytes(sha2::Sha256::digest(&diff).into()),
        ExactByteLength::new(diff.len() as u64),
    );
    Revisions {
        base: roots.remove(0),
        target: roots.remove(0),
        diff,
        diff_binding,
    }
}

pub fn source(root: &SharedNetworkRevision) -> CommittedNetworkSource {
    let origin = root.canonical_origin();
    CommittedNetworkSource::Published {
        reference: PublishedLfcaReference::new(
            "fixture://cutover-scale",
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .expect("source"),
    }
}

pub fn world(roots: &Revisions, count: u32, edges: u32, routes: u32) -> TrafficWorld {
    let mut world = TrafficWorld::install(
        Arc::clone(&roots.base),
        WorldConfig::new(count, routes, routes as u64, 1_024, 1, 100),
        source(&roots.base),
        531,
        WorldPolicySelection::NotRequired,
    )
    .expect("install");
    let handles: Vec<_> = (0..routes)
        .map(|route| {
            world
                .register_route(RouteRegisterInput::new(vec![LaneEdgeOrdinal::from_raw(
                    route % edges,
                )]))
                .expect("route")
        })
        .collect();
    for vehicle in 0..count {
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                handles[(vehicle % routes) as usize],
                0,
                (vehicle / edges + 1) * 10_000,
                0,
            ))
            .expect("spaced spawn");
    }
    world
}
