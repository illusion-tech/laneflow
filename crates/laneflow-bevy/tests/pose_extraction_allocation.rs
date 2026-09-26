//! #721 借用来源提取的稳态零新增分配冒烟（机制级；完整性能取证在 #712 第三层）。
//!
//! Session 与 output 暖机后，一次完整 Adapter 提取不应产生任何分配或重分配：
//! 来源迭代器借用世界、两组候选缓冲与输出缓冲复用容量。夹具与暖机在计数
//! 区间外。

use std::{num::NonZeroU32, sync::Arc};

#[global_allocator]
static ALLOCATOR: &stats_alloc::StatsAlloc<std::alloc::System> = &stats_alloc::INSTRUMENTED_SYSTEM;

use laneflow_bevy::{LaneFlowCommittedPoseBatch, LaneFlowSession, LaneFlowSessionConfig};
use laneflow_compiler::{
    CanonicalFrameInput, CanonicalPoint3F32Input, CompilationUnitBuilder, CompileLimits, Compiler,
    IidmVehicleProfileInput, LaneEdgeGeometryInput, LaneEdgeInput, LaneEdgeReference,
    ParticipantClassInput, ParticipantClassReference, PortableDiffBase, PortableEmissionProvenance,
    SourceModuleHeader, SourceModuleHeaderInput, SyntheticModuleBuilder, VehicleProfileInput,
    emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_runtime::{
    CommittedNetworkSource, PublishedLfcaReference, RouteRegisterInput, TrafficWorld,
    VehicleSpawnInput, WorldConfig, WorldPolicySelection,
};
use laneflow_spatial::{FramePlacementToken, SpatialSession};
use laneflow_static_contract::VehicleProfileOrdinal;
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

fn compile_single_frame() -> laneflow_compiler::CompilationOutput {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "bevy/pose-extraction-allocation",
            source_document_key: "pose-extraction-allocation.document",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x73; 32],
            frontend_options_digest: [0x13; 32],
            random_seed: Some(722),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .expect("header");
    let mut module = SyntheticModuleBuilder::new(header, &limits).expect("module");
    let point = |x: f32, y: f32| CanonicalPoint3F32Input { x, y, z: 0.0 };
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
        .expect("profile")
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge",
            length_meters: 1_000.0,
            speed_limit_meters_per_second: 15.0,
            successors: &[],
        })
        .expect("edge")
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame",
            lane_edge_geometries: &[LaneEdgeGeometryInput {
                lane_edge: LaneEdgeReference::local("edge"),
                centerline_points: &[point(0.0, 0.0), point(1_000.0, 0.0)],
            }],
        })
        .expect("frame");
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().expect("module"))
        .expect("unit module");
    Compiler::new()
        .compile(unit.build().expect("unit"))
        .expect("compile")
}

fn root() -> Arc<SharedNetworkRevision> {
    let output = compile_single_frame();
    let provenance = PortableEmissionProvenance::try_new("bevy-pose-extraction-allocation-v1")
        .expect("provenance");
    let candidate = emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .expect("candidate");
    let checked = check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .expect("bundle");
    build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("root")
}

#[test]
fn warm_extraction_path_has_no_new_allocations() {
    let root = root();
    let origin = root.canonical_origin();
    let mut world = TrafficWorld::install(
        Arc::clone(&root),
        WorldConfig::new(8, 4, 1_024, 1_024, 100),
        laneflow_runtime::ExecutionConfig::new(NonZeroU32::MIN),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://pose-extraction-allocation",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("source"),
        },
        722,
        WorldPolicySelection::NotRequired,
    )
    .expect("install");
    let edge = world
        .traffic()
        .lane_lengths_millimetres()
        .iter()
        .enumerate()
        .find(|(_, length)| **length == 1_000_000)
        .map(|(index, _)| laneflow_static_contract::LaneEdgeOrdinal::from_raw(index as u32))
        .expect("single edge");
    let route = world
        .register_route(RouteRegisterInput::new(vec![edge]))
        .expect("route");
    for progress in [1_000_u32, 50_000, 120_000, 300_000, 600_000] {
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, progress, 0)
                    .with_open_entrance(),
            )
            .expect("spawn");
    }
    let spatial = SpatialSession::bind(Arc::clone(&root))
        .expect("bind")
        .expect("spatial");
    let mut session = LaneFlowSession::new(
        world,
        Some(spatial),
        LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero")),
    )
    .expect("session");
    let mut output = LaneFlowCommittedPoseBatch::new();

    // 暖机：Session 双候选缓冲、Spatial 双缓冲与 output 两侧都达到容量。
    for round in 0..4 {
        session
            .extract_committed_pose_batch(FramePlacementToken::new(round), &mut output)
            .expect("warm-up extract");
    }

    let region = stats_alloc::Region::new(ALLOCATOR);
    session
        .extract_committed_pose_batch(FramePlacementToken::new(99), &mut output)
        .expect("measured extract");
    let stats = region.change();
    assert_eq!(stats.allocations, 0, "steady extract must not allocate");
    assert_eq!(stats.reallocations, 0, "steady extract must not reallocate");
    assert_eq!(output.vehicles().len(), 5);
    assert_eq!(output.batch().records().len(), 5);
    let selection = output.vehicles().iter().rev().copied().collect::<Vec<_>>();
    let context = session.consumption_context();
    let mut second_output = LaneFlowCommittedPoseBatch::new();
    for round in 0..8 {
        let destination = if round % 2 == 0 {
            &mut output
        } else {
            &mut second_output
        };
        session
            .extract_selected_committed_pose_batch(
                context,
                &selection,
                FramePlacementToken::new(round),
                destination,
            )
            .unwrap();
    }
    for count in [5, 1, 0, 5, 1] {
        let region = stats_alloc::Region::new(ALLOCATOR);
        session
            .extract_selected_committed_pose_batch(
                context,
                &selection[..count],
                FramePlacementToken::new(100),
                &mut output,
            )
            .unwrap();
        let stats = region.change();
        assert_eq!(stats.allocations, 0, "warm selected allocation");
        assert_eq!(stats.reallocations, 0, "warm selected reallocation");
        assert_eq!(output.vehicles(), &selection[..count]);
    }
}
