#[path = "support/policy.rs"]
mod test_policy;

#[cfg(feature = "placement-fixtures")]
#[path = "support/conflict_review.rs"]
mod conflict_review;

#[cfg(feature = "placement-fixtures")]
#[path = "support/policy_acceptance.rs"]
mod policy_acceptance;

#[cfg(feature = "placement-fixtures")]
#[path = "support/eta_evidence.rs"]
mod eta_evidence;

#[path = "support/waiting_output_evidence.rs"]
mod waiting_output_evidence;

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use laneflow_compiler::road_editing as lfre;
use laneflow_compiler::{
    AccessRuleInput, AccessRuleTargetInput, CompilationOutput, CompilationUnitBuilder,
    CompileLimits, Compiler, DiagnosticCode, GeometryAccuracyProfile, GeometryDirectionProfile,
    IidmVehicleProfileInput, JunctionInput, JunctionReference, LaneEdgeInput, LaneEdgeReference,
    ManeuverGateInput, ManeuverPathInput, ManeuverPathReference, MovementInput, MovementReference,
    ParkingFacilityInput, ParkingFacilityReference, ParkingLaneAnchorInput,
    ParkingSpaceGeometryInput, ParkingSpaceInput, ParticipantClassInput, ParticipantClassReference,
    PortableDiffBase, PortableEmissionProvenance, SignalControlInput, SignalControllerInput,
    SignalGroupInput, SignalGroupReference, SignalGroupStateInput, SignalPhaseInput,
    SourceModuleHeader, SourceModuleHeaderInput, StopLineInput, StopLineReference,
    SyntheticModuleBuilder, VehicleProfileInput, derive_canonical_stable_id_v1,
    emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle, preflight_object_values};
#[cfg(feature = "placement-fixtures")]
use laneflow_runtime::VehicleHandle;
use laneflow_runtime::{
    AdmittedRouteRegisterInput, CandidateRouteInput, CommittedNetworkSource, CostModelKey,
    CutoverError, CutoverPreflightLimits, CutoverTransactionLimits, DynamicCostSnapshotBinding,
    InstallError, LeaveParkingTarget, LfcaOriginBinding, MigrationPolicyKind,
    NetworkRevisionCutoverDescriptor, ObservationExportMode, ObservationSelection,
    ParkedVehicleSpawnInput, ParkingBinding, ParkingError, ParkingTarget, PoseSource,
    PublishedLfcaReference, RebindParkingTarget, ReplaceError, ReserveParkingTarget, RouteError,
    RouteHandle, RouteRegisterInput, SemanticDiffOriginBinding, SnapshotLimitDimension,
    SnapshotRestoreError, SnapshotRestoreLimits, SpawnError, TickInput, TrafficWorld,
    VehicleSpawnInput, VehicleStatus, VirtualEntryAnchorSelector, VirtualExitAnchorSelector,
    WorldConfig, bind_observation_set, deterministic_state_digest, encode_lfrs, restore_lfrs,
};
#[cfg(feature = "placement-fixtures")]
use laneflow_static_contract::RightOfWayPolicySetOrdinal;
use laneflow_static_contract::{
    AccessEffect, ConflictZoneOrdinal, EntityKind, LaneEdgeId, ParkingFacilityOrdinal,
    ParkingSpaceOrdinal, ParticipantStreamOrdinal, PortableObjectKind,
    SEMANTIC_DIFF_FORMAT_VERSION, Sha256Digest, SignalAspect, VehicleProfileId,
    VehicleProfileOrdinal,
};
use laneflow_static_network::{
    ConflictPathAnchor, SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision,
    SpatialBuildOption, build_shared_network_revision,
};

use laneflow_runtime_snapshot_wire::generated::lane_flow::runtime_snapshot::v6 as snapshot_wire;

fn install_fixture(
    revision: std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
    config: laneflow_runtime::WorldConfig,
) -> Result<laneflow_runtime::TrafficWorld, laneflow_runtime::InstallError> {
    laneflow_runtime::TrafficWorld::install(
        Arc::clone(&revision),
        config,
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        published_source(&revision, "fixture://in-process"),
        0,
        test_policy::selection(&revision),
    )
}

fn published_source(revision: &SharedNetworkRevision, key: &str) -> CommittedNetworkSource {
    let origin = *revision.canonical_origin();
    CommittedNetworkSource::Published {
        reference: PublishedLfcaReference::new(
            key,
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .expect("non-empty fixture key"),
    }
}

fn wire_table_field_offset(
    table: laneflow_runtime_snapshot_wire::runtime::Table<'_>,
    field: laneflow_runtime_snapshot_wire::runtime::VOffsetT,
) -> usize {
    let relative = usize::from(table.vtable().get(field));
    assert_ne!(relative, 0, "fixture field must be present");
    table.loc() + relative
}

fn wire_clear_table_field(
    bytes: &mut [u8],
    table: usize,
    field: laneflow_runtime_snapshot_wire::runtime::VOffsetT,
) {
    let backwards = i32::from_le_bytes(
        bytes[table..table + 4]
            .try_into()
            .expect("table vtable offset"),
    );
    assert!(backwards > 0);
    let vtable = table - usize::try_from(backwards).expect("positive vtable offset");
    let entry = vtable + usize::from(field);
    bytes[entry..entry + 2].copy_from_slice(&0_u16.to_le_bytes());
}

fn iidm() -> IidmVehicleProfileInput {
    IidmVehicleProfileInput {
        length_meters: 4.5,
        desired_speed_meters_per_second: 13.75,
        min_gap_meters: 2.0,
        time_headway_seconds: 1.4,
        max_acceleration_meters_per_second_squared: 1.8,
        comfortable_deceleration_meters_per_second_squared: 2.0,
        emergency_deceleration_meters_per_second_squared: 4.5,
    }
}

fn compile_revision(
    configure: impl FnOnce(&mut SyntheticModuleBuilder),
) -> Arc<SharedNetworkRevision> {
    compile_revision_with_limits(CompileLimits::p100_initial_v1(), configure)
}

fn compile_revision_with_limits(
    limits: CompileLimits,
    configure: impl FnOnce(&mut SyntheticModuleBuilder),
) -> Arc<SharedNetworkRevision> {
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "city/runtime-coverage",
            source_document_key: "runtime-coverage.document",
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
        .unwrap_or_else(|bundle| {
            panic!(
                "compiled output diagnostics: {:?}",
                bundle
                    .diagnostics()
                    .iter()
                    .map(|diagnostic| (diagnostic.code(), diagnostic.payload()))
                    .collect::<Vec<_>>()
            )
        });
    let provenance = PortableEmissionProvenance::try_new("laneflow-runtime-coverage-v1")
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
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("shared network revision")
}

fn compile_road_editing_output(module: lfre::RoadEditingSourceModule) -> CompilationOutput {
    compile_road_editing_output_with_limits(module, CompileLimits::p100_initial_v2())
}

fn compile_road_editing_output_with_limits(
    module: lfre::RoadEditingSourceModule,
    limits: CompileLimits,
) -> CompilationOutput {
    let source = lfre::RoadEditingSourceWriter::new(&limits)
        .write(module)
        .expect("Road Editing source");
    let input =
        lfre::RoadEditingModuleInput::try_new("runtime-conflict.lfre", source.as_bytes(), None)
            .expect("Road Editing module input");
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_road_editing_module(input)
        .expect("Road Editing admission");
    Compiler::new()
        .compile(unit.build().expect("compilation unit"))
        .unwrap_or_else(|bundle| {
            panic!(
                "compiled Road Editing output diagnostics: {:?}",
                bundle
                    .diagnostics()
                    .iter()
                    .map(|diagnostic| (diagnostic.code(), diagnostic.payload()))
                    .collect::<Vec<_>>()
            )
        })
}

fn compile_road_editing_revision(
    module: lfre::RoadEditingSourceModule,
) -> Arc<SharedNetworkRevision> {
    compile_road_editing_revision_with_limits(module, CompileLimits::p100_initial_v2())
}

fn compile_road_editing_revision_with_limits(
    module: lfre::RoadEditingSourceModule,
    limits: CompileLimits,
) -> Arc<SharedNetworkRevision> {
    let output = compile_road_editing_output_with_limits(module, limits);
    let provenance = PortableEmissionProvenance::try_new("laneflow-runtime-conflict-v1")
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
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("shared network revision")
}

fn compile_conflict_cutover_pair(
    base_module: lfre::RoadEditingSourceModule,
    target_module: lfre::RoadEditingSourceModule,
) -> (
    Arc<SharedNetworkRevision>,
    Arc<SharedNetworkRevision>,
    Vec<u8>,
    SemanticDiffOriginBinding,
) {
    let base_output = compile_road_editing_output(base_module);
    let target_output = compile_road_editing_output(target_module);
    let provenance = PortableEmissionProvenance::try_new("laneflow-runtime-conflict-cutover-v1")
        .expect("portable provenance");
    let base_candidate = emit_portable_candidate(
        &base_output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .expect("base portable candidate");
    let base_values = preflight_object_values(
        base_candidate.canonical_artifact().bytes(),
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .expect("value-checked base artifact");
    let target_candidate = emit_portable_candidate(
        &target_output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Artifact(base_values),
    )
    .expect("target portable candidate");
    let base_checked = check_post_emission_bundle(
        base_candidate.canonical_artifact().bytes(),
        base_candidate.source_map().bytes(),
        base_candidate.semantic_diff().bytes(),
        base_candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .expect("checked base bundle");
    let target_checked = check_post_emission_bundle(
        target_candidate.canonical_artifact().bytes(),
        target_candidate.source_map().bytes(),
        target_candidate.semantic_diff().bytes(),
        target_candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .expect("checked target bundle");
    let options = || {
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        )
    };
    let base_revision =
        build_shared_network_revision(base_checked.canonical_network_input(), options())
            .expect("base shared network revision");
    let target_revision =
        build_shared_network_revision(target_checked.canonical_network_input(), options())
            .expect("target shared network revision");
    let semantic_diff = target_candidate.semantic_diff().bytes().to_vec();
    let semantic_diff_binding = SemanticDiffOriginBinding::new(
        SEMANTIC_DIFF_FORMAT_VERSION,
        target_candidate.semantic_diff().digest(),
        target_candidate.semantic_diff().byte_length(),
    );
    (
        base_revision,
        target_revision,
        semantic_diff,
        semantic_diff_binding,
    )
}

fn road_editing_line(start: (f64, f64), end: (f64, f64)) -> lfre::RoadEditingCurveProgram {
    lfre::RoadEditingCurveProgram::try_new(
        lfre::RoadEditingPoint3::try_new(start.0, 0.0, start.1).expect("curve start"),
        vec![lfre::RoadEditingCurveSegment::line(
            lfre::RoadEditingPoint3::try_new(end.0, 0.0, end.1).expect("curve end"),
        )],
    )
    .expect("line curve")
}

fn road_editing_loop() -> lfre::RoadEditingCurveProgram {
    let point = |x, z| lfre::RoadEditingPoint3::try_new(x, 0.0, z).expect("loop point");
    const K: f64 = 7.179_701_749;
    lfre::RoadEditingCurveProgram::try_new(
        point(13.0, 0.0),
        vec![
            lfre::RoadEditingCurveSegment::cubic_bezier(
                point(13.0 + K, 0.0),
                point(26.0, 13.0 - K),
                point(26.0, 13.0),
            ),
            lfre::RoadEditingCurveSegment::cubic_bezier(
                point(26.0, 13.0 + K),
                point(13.0 + K, 26.0),
                point(13.0, 26.0),
            ),
            lfre::RoadEditingCurveSegment::line(point(-13.0, 26.0)),
            lfre::RoadEditingCurveSegment::cubic_bezier(
                point(-13.0 - K, 26.0),
                point(-26.0, 13.0 + K),
                point(-26.0, 13.0),
            ),
            lfre::RoadEditingCurveSegment::cubic_bezier(
                point(-26.0, 13.0 - K),
                point(-13.0 - K, 0.0),
                point(-13.0, 0.0),
            ),
        ],
    )
    .expect("loop curve")
}

fn add_road_editing_approach(
    module: &mut lfre::RoadEditingSourceModuleBuilder<'_>,
    edge_key: &str,
    geometry: lfre::RoadEditingCurveProgram,
    successors: Vec<lfre::LaneEdgeReference>,
) {
    let alignment_key = format!("{edge_key}-alignment");
    let corridor_key = format!("{edge_key}-corridor");
    let corridor = lfre::RoadCorridorReference::local(&corridor_key).expect("corridor reference");
    let section = lfre::RoadSectionReference::owner_scoped(vec![corridor_key.clone()], "section")
        .expect("section reference");
    let lane = lfre::AuthoringLaneReference::owner_scoped(
        vec![corridor_key.clone(), "section".into()],
        "lane",
    )
    .expect("authoring lane reference");
    let edge = lfre::LaneEdgeReference::local(edge_key).expect("approach edge reference");

    module
        .add_alignment(
            lfre::RoadAlignmentInput::try_new(
                &alignment_key,
                lfre::CanonicalFrameReference::local("frame-main").expect("frame reference"),
                geometry,
            )
            .expect("road alignment"),
        )
        .expect("add road alignment")
        .add_declaration(lfre::RoadEditingDeclaration::RoadCorridor(
            lfre::RoadCorridorInput::try_new(
                &corridor_key,
                lfre::RoadAlignmentReference::try_new(&alignment_key).expect("alignment reference"),
                0.0,
                lfre::RoadEditingStationEnd::AlignmentEnd,
                section.clone(),
                lane.clone(),
                vec![lfre::RoadEditingCorridorElement::RoadSection(
                    section.clone(),
                )],
            )
            .expect("road corridor"),
        ))
        .expect("add road corridor")
        .add_declaration(lfre::RoadEditingDeclaration::RoadSection(
            lfre::RoadSectionInput::try_new("section", "motorLane", vec![lane], corridor)
                .expect("road section"),
        ))
        .expect("add road section")
        .add_declaration(lfre::RoadEditingDeclaration::AuthoringLane(
            lfre::AuthoringLaneInput::try_new(
                "lane",
                edge.clone(),
                lfre::RoadEditingLaneDirection::Forward,
                lfre::LinearWidthProfile::try_new(3.5, 3.5).expect("lane width"),
                None,
                section,
            )
            .expect("authoring lane"),
        ))
        .expect("add authoring lane")
        .add_declaration(lfre::RoadEditingDeclaration::LaneEdge(
            lfre::LaneEdgeInput::try_new(edge_key, 13.0, successors, None)
                .expect("approach lane edge"),
        ))
        .expect("add approach lane edge");
}

fn conflict_road_editing_module() -> lfre::RoadEditingSourceModule {
    conflict_road_editing_module_with_options(2, false, true)
}

fn terminal_conflict_road_editing_module() -> lfre::RoadEditingSourceModule {
    conflict_road_editing_module_with_options(2, true, true)
}

fn non_conflict_road_editing_module() -> lfre::RoadEditingSourceModule {
    conflict_road_editing_module_with_options(0, false, false)
}

fn conflict_multiplicity_road_editing_module() -> lfre::RoadEditingSourceModule {
    conflict_road_editing_module_with_shape(2, false, true, true)
}

fn conflict_road_editing_module_with_vehicle_speed(
    desired_speed_meters_per_second: f64,
) -> lfre::RoadEditingSourceModule {
    conflict_road_editing_module_with_shape_and_speed(
        2,
        false,
        true,
        false,
        desired_speed_meters_per_second,
        ConflictPolicyFixture::default(),
    )
}

fn conflict_yield_road_editing_module() -> lfre::RoadEditingSourceModule {
    conflict_road_editing_module_with_shape_and_speed(
        2,
        false,
        true,
        false,
        13.0,
        ConflictPolicyFixture {
            yielding: true,
            ..ConflictPolicyFixture::default()
        },
    )
}

/// 两条路共用同一条出口。长路只有一道门，车身会占到出口上；短路从另一侧进入同一条出口。
fn shared_downstream_road_editing_module() -> lfre::RoadEditingSourceModule {
    shared_downstream_road_editing_module_with_cross(false)
}

fn shared_downstream_road_editing_module_with_cross(
    cross_exit: bool,
) -> lfre::RoadEditingSourceModule {
    let limits = CompileLimits::p100_initial_v2();
    let header = lfre::RoadEditingModuleHeader::try_new(
        "city/runtime-conflict",
        "runtime-conflict.lfre",
        Vec::new(),
        lfre::RoadEditingProvenance::direct("runtime conflict fixture").expect("provenance"),
    )
    .expect("Road Editing header");
    let mut module = lfre::RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        &limits,
    )
    .expect("Road Editing builder");
    let junction = lfre::JunctionReference::local("crossing").expect("junction reference");
    let approach_zone =
        lfre::ConflictZoneReference::owner_scoped(vec!["crossing".to_owned()], "approach-zone")
            .expect("approach zone");
    let exit_zone =
        lfre::ConflictZoneReference::owner_scoped(vec!["crossing".to_owned()], "exit-zone")
            .expect("exit zone");
    let frame = lfre::CanonicalFrameReference::local("frame-main").expect("frame reference");
    module
        .add_declaration(lfre::RoadEditingDeclaration::CanonicalFrame(
            lfre::CanonicalFrameInput::try_new("frame-main").expect("canonical frame"),
        ))
        .expect("add canonical frame");
    for (edge, start, end) in [
        ("east-entry", (-13.0, 0.0), (0.0, 0.0)),
        ("west-approach", (0.0, 0.0), (13.0, 0.0)),
        ("west-exit", (13.0, 0.0), (26.0, 0.0)),
    ] {
        let successors = if edge == "west-approach" {
            vec![lfre::LaneEdgeReference::local("west-exit").expect("exit successor")]
        } else {
            Vec::new()
        };
        add_road_editing_approach(&mut module, edge, road_editing_line(start, end), successors);
    }
    module
        .add_declaration(lfre::RoadEditingDeclaration::LaneEdge(
            lfre::LaneEdgeInput::try_new(
                "east-internal",
                13.0,
                Vec::new(),
                Some(road_editing_line((0.0, 0.0), (13.0, 0.0))),
            )
            .expect("internal lane edge"),
        ))
        .expect("add internal lane edge");
    let mut approach_keys = vec!["east-entry", "west-approach", "west-exit"];
    let mut internal_keys = vec!["east-internal"];
    if cross_exit {
        add_road_editing_approach(
            &mut module,
            "north-entry",
            road_editing_line((0.0, -13.0), (0.0, 0.0)),
            Vec::new(),
        );
        module
            .add_declaration(lfre::RoadEditingDeclaration::LaneEdge(
                lfre::LaneEdgeInput::try_new(
                    "north-internal",
                    13.0,
                    Vec::new(),
                    Some(road_editing_line((0.0, 0.0), (0.0, 13.0))),
                )
                .expect("north internal lane edge"),
            ))
            .expect("add north internal lane edge");
        add_road_editing_approach(
            &mut module,
            "south-exit",
            road_editing_line((0.0, 13.0), (0.0, 26.0)),
            Vec::new(),
        );
        approach_keys.extend(["north-entry", "south-exit"]);
        internal_keys.push("north-internal");
    }
    module
        .add_declaration(lfre::RoadEditingDeclaration::Junction(
            lfre::JunctionInput::try_new(
                "crossing",
                approach_keys
                    .into_iter()
                    .map(|key| lfre::LaneEdgeReference::local(key).expect("approach edge"))
                    .collect(),
                internal_keys
                    .into_iter()
                    .map(|key| lfre::LaneEdgeReference::local(key).expect("internal edge"))
                    .collect(),
            )
            .expect("junction"),
        ))
        .expect("add junction")
        .add_declaration(lfre::RoadEditingDeclaration::ConflictZone(
            lfre::ConflictZoneInput::try_new("approach-zone", junction.clone())
                .expect("approach conflict zone"),
        ))
        .expect("add approach zone")
        .add_declaration(lfre::RoadEditingDeclaration::ConflictZone(
            lfre::ConflictZoneInput::try_new("exit-zone", junction.clone()).expect("exit zone"),
        ))
        .expect("add exit zone");
    let east_movement = lfre::MovementReference::owner_scoped(vec!["crossing".into()], "east-west")
        .expect("movement reference");
    let east_path = lfre::ManeuverPathReference::owner_scoped(
        vec!["crossing".into(), "east-west".into()],
        "east-west-path",
    )
    .expect("path reference");
    let exit_movement = lfre::MovementReference::owner_scoped(vec!["crossing".into()], "exit-move")
        .expect("exit movement reference");
    let exit_path = lfre::ManeuverPathReference::owner_scoped(
        vec!["crossing".into(), "exit-move".into()],
        "exit-path",
    )
    .expect("exit path reference");
    module
        .add_declaration(lfre::RoadEditingDeclaration::Movement(
            lfre::MovementInput::try_new("east-west", junction.clone(), "east-entry", "west-exit")
                .expect("movement"),
        ))
        .expect("add movement")
        .add_declaration(lfre::RoadEditingDeclaration::Movement(
            lfre::MovementInput::try_new(
                "exit-move",
                junction.clone(),
                "west-approach",
                "west-exit",
            )
            .expect("exit movement"),
        ))
        .expect("add exit movement")
        .add_declaration(lfre::RoadEditingDeclaration::ManeuverPath(
            lfre::ManeuverPathInput::try_new(
                "east-west-path",
                east_movement,
                lfre::LaneEdgeReference::local("east-entry").expect("entry edge"),
                vec![lfre::LaneEdgeReference::local("east-internal").expect("internal edge")],
                lfre::LaneEdgeReference::local("west-exit").expect("exit edge"),
            )
            .expect("maneuver path"),
        ))
        .expect("add maneuver path")
        .add_declaration(lfre::RoadEditingDeclaration::ManeuverPath(
            lfre::ManeuverPathInput::try_new(
                "exit-path",
                exit_movement,
                lfre::LaneEdgeReference::local("west-approach").expect("west approach"),
                Vec::new(),
                lfre::LaneEdgeReference::local("west-exit").expect("exit edge"),
            )
            .expect("exit maneuver path"),
        ))
        .expect("add exit maneuver path")
        .add_declaration(lfre::RoadEditingDeclaration::StopLine(
            lfre::StopLineInput::try_new(
                "east-stop",
                lfre::LaneEdgeReference::local("east-entry").expect("stop edge"),
            )
            .expect("stop line"),
        ))
        .expect("add stop line")
        .add_declaration(lfre::RoadEditingDeclaration::StopLine(
            lfre::StopLineInput::try_new(
                "west-stop",
                lfre::LaneEdgeReference::local("west-approach").expect("west stop edge"),
            )
            .expect("west stop line"),
        ))
        .expect("add west stop line")
        .add_declaration(lfre::RoadEditingDeclaration::ManeuverGate(
            lfre::ManeuverGateInput::try_new(
                "east-west-gate",
                east_path.clone(),
                0,
                lfre::StopLineReference::local("east-stop").expect("stop line reference"),
                lfre::RoadEditingSignalControl::None,
            )
            .expect("admission gate"),
        ))
        .expect("add admission gate")
        .add_declaration(lfre::RoadEditingDeclaration::ManeuverGate(
            lfre::ManeuverGateInput::try_new(
                "exit-gate",
                exit_path.clone(),
                0,
                lfre::StopLineReference::local("west-stop").expect("west stop reference"),
                lfre::RoadEditingSignalControl::None,
            )
            .expect("exit gate"),
        ))
        .expect("add exit gate");
    let cross_path = if cross_exit {
        let cross_movement =
            lfre::MovementReference::owner_scoped(vec!["crossing".into()], "north-south")
                .expect("cross movement reference");
        let cross_path = lfre::ManeuverPathReference::owner_scoped(
            vec!["crossing".into(), "north-south".into()],
            "north-south-path",
        )
        .expect("cross path reference");
        module
            .add_declaration(lfre::RoadEditingDeclaration::Movement(
                lfre::MovementInput::try_new(
                    "north-south",
                    junction.clone(),
                    "north-entry",
                    "south-exit",
                )
                .expect("cross movement"),
            ))
            .expect("add cross movement")
            .add_declaration(lfre::RoadEditingDeclaration::ManeuverPath(
                lfre::ManeuverPathInput::try_new(
                    "north-south-path",
                    cross_movement,
                    lfre::LaneEdgeReference::local("north-entry").expect("north entry"),
                    vec![lfre::LaneEdgeReference::local("north-internal").expect("north internal")],
                    lfre::LaneEdgeReference::local("south-exit").expect("south exit"),
                )
                .expect("cross maneuver path"),
            ))
            .expect("add cross maneuver path")
            .add_declaration(lfre::RoadEditingDeclaration::StopLine(
                lfre::StopLineInput::try_new(
                    "north-stop",
                    lfre::LaneEdgeReference::local("north-entry").expect("north stop edge"),
                )
                .expect("north stop line"),
            ))
            .expect("add north stop line")
            .add_declaration(lfre::RoadEditingDeclaration::ManeuverGate(
                lfre::ManeuverGateInput::try_new(
                    "north-gate",
                    cross_path.clone(),
                    0,
                    lfre::StopLineReference::local("north-stop").expect("north stop reference"),
                    lfre::RoadEditingSignalControl::None,
                )
                .expect("north gate"),
            ))
            .expect("add north gate");
        Some(cross_path)
    } else {
        None
    };
    let cross_zone = cross_path.as_ref().map(|_| exit_zone.clone());
    for (stream_key, zone, path, entry, exit) in [
        (
            "approach-a",
            approach_zone.clone(),
            east_path.clone(),
            (1_u32, 2.0),
            (1_u32, 12.5),
        ),
        ("approach-b", approach_zone, east_path, (1, 2.4), (1, 12.2)),
        (
            "exit-a",
            exit_zone.clone(),
            exit_path.clone(),
            (1, 1.0),
            (1, 4.0),
        ),
        ("exit-b", exit_zone, exit_path, (1, 1.4), (1, 4.4)),
    ] {
        module
            .add_declaration(lfre::RoadEditingDeclaration::ParticipantStream(
                lfre::ParticipantStreamInput::try_new(
                    stream_key,
                    junction.clone(),
                    path,
                    vec![lfre::ConflictPassageInput::new(
                        zone,
                        lfre::PathAnchorInput::interior(entry.0, entry.1).expect("entry anchor"),
                        lfre::PathAnchorInput::interior(exit.0, exit.1).expect("exit anchor"),
                    )],
                )
                .expect("participant stream"),
            ))
            .expect("add participant stream");
    }
    if let (Some(cross_path), Some(cross_zone)) = (cross_path, cross_zone) {
        module
            .add_declaration(lfre::RoadEditingDeclaration::ParticipantStream(
                lfre::ParticipantStreamInput::try_new(
                    "cross-c",
                    junction.clone(),
                    cross_path,
                    vec![lfre::ConflictPassageInput::new(
                        cross_zone,
                        lfre::PathAnchorInput::interior(1, 1.0).expect("cross entry"),
                        lfre::PathAnchorInput::interior(1, 5.0).expect("cross exit"),
                    )],
                )
                .expect("cross participant stream"),
            ))
            .expect("add cross participant stream");
    }
    for (zone, key_offset) in [
        (
            lfre::ConflictZoneReference::owner_scoped(vec!["crossing".to_owned()], "approach-zone")
                .expect("approach zone region"),
            0.0,
        ),
        (
            lfre::ConflictZoneReference::owner_scoped(vec!["crossing".to_owned()], "exit-zone")
                .expect("exit zone region"),
            20.0,
        ),
    ] {
        let shift = key_offset;
        module
            .add_conflict_zone_region(
                lfre::ConflictZoneRegionInput::try_new(
                    zone,
                    frame.clone(),
                    -1.0,
                    1.0,
                    [
                        (-1.0 + shift, -1.0),
                        (1.0 + shift, -1.0),
                        (1.0 + shift, 1.0),
                        (-1.0 + shift, 1.0),
                    ]
                    .into_iter()
                    .map(|(x, z)| lfre::RoadEditingPoint2::try_new(x, z).expect("region point"))
                    .collect(),
                )
                .expect("conflict zone region"),
            )
            .expect("add conflict zone region");
    }
    let participant =
        lfre::ParticipantClassReference::local("road-user").expect("participant class reference");
    module
        .add_declaration(lfre::RoadEditingDeclaration::ParticipantClass(
            lfre::ParticipantClassInput::try_new("road-user").expect("participant class"),
        ))
        .expect("add participant class")
        .add_declaration(lfre::RoadEditingDeclaration::VehicleProfile(
            lfre::VehicleProfileInput::try_new(
                "car",
                participant.clone(),
                lfre::IidmVehicleProfileInput::try_new(4.5, 13.0, 2.0, 1.5, 1.5, 2.0, 4.0)
                    .expect("car profile"),
            )
            .expect("car profile"),
        ))
        .expect("add car")
        .add_declaration(lfre::RoadEditingDeclaration::VehicleProfile(
            lfre::VehicleProfileInput::try_new(
                "dot",
                participant,
                lfre::IidmVehicleProfileInput::try_new(0.1, 13.0, 0.2, 1.5, 1.5, 2.0, 4.0)
                    .expect("dot profile"),
            )
            .expect("dot profile"),
        ))
        .expect("add dot");
    let mut policy_gates: Vec<_> = [
        (
            "east-west-gate",
            vec![
                "crossing".to_owned(),
                "east-west".to_owned(),
                "east-west-path".to_owned(),
            ],
        ),
        (
            "exit-gate",
            vec![
                "crossing".to_owned(),
                "exit-move".to_owned(),
                "exit-path".to_owned(),
            ],
        ),
    ]
    .into_iter()
    .map(|(gate, owners)| {
        lfre::PolicyGateRuleInput::try_new(
            gate,
            lfre::ManeuverGateReference::owner_scoped(owners, gate).expect("gate reference"),
            None,
            laneflow_compiler::GateInterpretation::Uncontrolled,
            laneflow_compiler::GateProhibition::None,
            vec![],
        )
        .expect("policy gate")
    })
    .collect();
    if cross_exit {
        policy_gates.push(
            lfre::PolicyGateRuleInput::try_new(
                "north-gate",
                lfre::ManeuverGateReference::owner_scoped(
                    vec![
                        "crossing".to_owned(),
                        "north-south".to_owned(),
                        "north-south-path".to_owned(),
                    ],
                    "north-gate",
                )
                .expect("north gate reference"),
                None,
                laneflow_compiler::GateInterpretation::Uncontrolled,
                laneflow_compiler::GateProhibition::None,
                vec![],
            )
            .expect("north policy gate"),
        );
    }
    let mut stream_keys = vec!["approach-a", "approach-b", "exit-a", "exit-b"];
    if cross_exit {
        stream_keys.push("cross-c");
    }
    let policy_streams = stream_keys
        .into_iter()
        .map(|key| {
            lfre::PolicyStreamRuleInput::try_new(
                key,
                lfre::ParticipantStreamReference::owner_scoped(vec!["crossing".into()], key)
                    .expect("stream reference"),
                None,
                0,
                Vec::new(),
                None,
                vec![],
            )
            .expect("policy stream")
        })
        .collect();
    module
        .add_declaration(lfre::RoadEditingDeclaration::RightOfWayPolicySet(
            lfre::RightOfWayPolicySetInput::try_new(
                "conflict-policy",
                laneflow_compiler::RegulationIdentity::try_new("engineering", "fixture-1")
                    .expect("regulation")
                    .with_source("repository:runtime-fixture-1")
                    .expect("regulation source"),
                vec![],
                Vec::new(),
                policy_streams,
                policy_gates,
            )
            .expect("policy set"),
        ))
        .expect("add policy");
    module.finish().expect("Road Editing module")
}

#[cfg(feature = "placement-fixtures")]
fn conflict_calibration_road_editing_module() -> lfre::RoadEditingSourceModule {
    conflict_road_editing_module_with_shape_and_speed(
        2,
        false,
        true,
        false,
        13.0,
        ConflictPolicyFixture {
            yielding: true,
            gap_values_ms: Some((5_000, 2_000, 500)),
            include_long_vehicle: true,
            ..ConflictPolicyFixture::default()
        },
    )
}

fn conflict_road_editing_module_with_stream_count(
    stream_count: usize,
) -> lfre::RoadEditingSourceModule {
    conflict_road_editing_module_with_options(stream_count, false, true)
}

fn conflict_road_editing_module_with_options(
    stream_count: usize,
    terminal_clearance: bool,
    include_conflict: bool,
) -> lfre::RoadEditingSourceModule {
    conflict_road_editing_module_with_shape(
        stream_count,
        terminal_clearance,
        include_conflict,
        false,
    )
}

fn conflict_road_editing_module_with_shape(
    stream_count: usize,
    terminal_clearance: bool,
    include_conflict: bool,
    multiplicity: bool,
) -> lfre::RoadEditingSourceModule {
    conflict_road_editing_module_with_shape_and_speed(
        stream_count,
        terminal_clearance,
        include_conflict,
        multiplicity,
        13.0,
        ConflictPolicyFixture::default(),
    )
}

#[derive(Clone, Copy, Default)]
struct ConflictPolicyFixture {
    loop_east: bool,
    deny: bool,
    yielding: bool,
    gap_values_ms: Option<(u64, u64, u64)>,
    include_long_vehicle: bool,
    waiting: bool,
    next_gate: bool,
    clearance: Option<(u32, f64)>,
    right_turn_signal: Option<laneflow_compiler::GateInterpretation>,
    signal_cycle_ms: Option<[u64; 2]>,
    signal_stop_aspect: Option<SignalAspect>,
    conflict_after_release: bool,
    waiting_on_north_only: bool,
    resource_free_release: bool,
    equal_priority: bool,
    short_vehicle: bool,
    /// 停车入口改到内部边 3 m 处，声明会越过它。
    early_parking_on_internal: bool,
    /// 排队区放在冲突后面的短出口上。这一拍够不着。
    late_waiting: bool,
    /// 北向冲突声明越过后面的排队入口。东向声明仍停在内部边里。
    late_claim_crosses: bool,
    /// 0 表示内部边仍是 13 m。更短时两道资源门可以落在同一拍行程里。
    close_internal_m: f64,
    /// 0 表示排队容量仍是 1。
    waiting_capacity: u32,
}

fn conflict_road_editing_module_with_shape_and_speed(
    stream_count: usize,
    terminal_clearance: bool,
    include_conflict: bool,
    multiplicity: bool,
    desired_speed_meters_per_second: f64,
    policy_fixture: ConflictPolicyFixture,
) -> lfre::RoadEditingSourceModule {
    let ConflictPolicyFixture {
        loop_east,
        deny,
        yielding,
        gap_values_ms,
        include_long_vehicle,
        waiting,
        next_gate,
        clearance,
        right_turn_signal,
        signal_cycle_ms,
        signal_stop_aspect,
        conflict_after_release,
        waiting_on_north_only,
        resource_free_release,
        equal_priority,
        short_vehicle,
        early_parking_on_internal,
        late_waiting,
        late_claim_crosses,
        close_internal_m,
        waiting_capacity,
    } = policy_fixture;
    let span = if close_internal_m > 0.0 {
        close_internal_m
    } else {
        13.0
    };
    let waiting_occupancy = if waiting_capacity == 0 {
        1
    } else {
        waiting_capacity
    };
    assert!(stream_count <= 2);
    assert!(!multiplicity || (include_conflict && stream_count == 2));
    let limits = CompileLimits::p100_initial_v2();
    let header = lfre::RoadEditingModuleHeader::try_new(
        "city/runtime-conflict",
        "runtime-conflict.lfre",
        Vec::new(),
        lfre::RoadEditingProvenance::direct("runtime conflict fixture").expect("provenance"),
    )
    .expect("Road Editing header");
    let mut module = lfre::RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        &limits,
    )
    .expect("Road Editing builder");

    if right_turn_signal.is_some() {
        let group = lfre::SignalGroupReference::local("right-turn-red").unwrap();
        let priority_group = lfre::SignalGroupReference::local("priority-signal").unwrap();
        let mut groups = vec![group.clone()];
        let mut phase_keys = vec!["red"];
        if signal_cycle_ms.is_some() {
            module
                .add_declaration(lfre::RoadEditingDeclaration::SignalGroup(
                    lfre::SignalGroupInput::try_new("priority-signal").unwrap(),
                ))
                .unwrap();
            groups.push(priority_group.clone());
            phase_keys.push("green");
        }
        module
            .add_declaration(lfre::RoadEditingDeclaration::SignalGroup(
                lfre::SignalGroupInput::try_new("right-turn-red").unwrap(),
            ))
            .unwrap()
            .add_declaration(lfre::RoadEditingDeclaration::SignalController(
                lfre::SignalControllerInput::try_new(
                    "right-turn-controller",
                    0,
                    groups,
                    phase_keys
                        .iter()
                        .map(|key| {
                            lfre::SignalPhaseReference::owner_scoped(
                                vec!["right-turn-controller".into()],
                                *key,
                            )
                            .unwrap()
                        })
                        .collect(),
                )
                .unwrap(),
            ))
            .unwrap();
        for (index, key) in phase_keys.into_iter().enumerate() {
            let mut states = vec![
                lfre::RoadEditingSignalPhaseState::try_new(
                    group.clone(),
                    if index == 0 {
                        signal_stop_aspect.unwrap_or(SignalAspect::Red)
                    } else {
                        SignalAspect::Green
                    },
                )
                .unwrap(),
            ];
            if signal_cycle_ms.is_some() {
                states.push(
                    lfre::RoadEditingSignalPhaseState::try_new(
                        priority_group.clone(),
                        if index == 0 {
                            SignalAspect::Green
                        } else {
                            SignalAspect::Red
                        },
                    )
                    .unwrap(),
                );
            }
            module
                .add_declaration(lfre::RoadEditingDeclaration::SignalPhase(
                    lfre::SignalPhaseInput::try_new(
                        key,
                        signal_cycle_ms.map_or(1_000, |durations| durations[index]),
                        states,
                        lfre::SignalControllerReference::local("right-turn-controller").unwrap(),
                    )
                    .unwrap(),
                ))
                .unwrap();
        }
    }

    let junction = lfre::JunctionReference::local("crossing").expect("junction reference");
    let zone =
        lfre::ConflictZoneReference::owner_scoped(vec!["crossing".to_owned()], "center-zone")
            .expect("zone reference");
    let secondary_zone =
        lfre::ConflictZoneReference::owner_scoped(vec!["crossing".to_owned()], "secondary-zone")
            .expect("secondary zone reference");
    let frame = lfre::CanonicalFrameReference::local("frame-main").expect("frame reference");

    module
        .add_declaration(lfre::RoadEditingDeclaration::CanonicalFrame(
            lfre::CanonicalFrameInput::try_new("frame-main").expect("canonical frame"),
        ))
        .expect("add canonical frame");
    let exit_span = if late_waiting { 5.0 } else { 13.0 };
    for (edge, start, end) in [
        ("east-entry", (-13.0, 0.0), (0.0, 0.0)),
        ("west-exit", (span, 0.0), (span + exit_span, 0.0)),
        ("north-entry", (0.0, -13.0), (0.0, 0.0)),
        ("south-exit", (0.0, span), (0.0, span + exit_span)),
    ] {
        if late_waiting && (edge == "west-exit" || edge == "south-exit") {
            continue;
        }
        let geometry = if (multiplicity || loop_east) && edge == "west-exit" {
            road_editing_loop()
        } else {
            road_editing_line(start, end)
        };
        let successors = if (multiplicity || loop_east) && edge == "west-exit" {
            vec![lfre::LaneEdgeReference::local("east-entry").expect("loop successor")]
        } else {
            Vec::new()
        };
        add_road_editing_approach(&mut module, edge, geometry, successors);
    }
    for (edge, start, end) in [
        ("east-internal", (0.0, 0.0), (span, 0.0)),
        ("north-internal", (0.0, 0.0), (0.0, span)),
        ("west-exit", (span, 0.0), (span + exit_span, 0.0)),
        ("south-exit", (0.0, span), (0.0, span + exit_span)),
    ] {
        if !late_waiting && (edge == "west-exit" || edge == "south-exit") {
            continue;
        }
        module
            .add_declaration(lfre::RoadEditingDeclaration::LaneEdge(
                lfre::LaneEdgeInput::try_new(
                    edge,
                    13.0,
                    Vec::new(),
                    Some(road_editing_line(start, end)),
                )
                .expect("lane edge"),
            ))
            .expect("add lane edge");
    }
    if late_waiting {
        for (edge, start, end) in [
            (
                "west-far",
                (span + exit_span, 0.0),
                (span + exit_span + 1.0, 0.0),
            ),
            (
                "south-far",
                (0.0, span + exit_span),
                (0.0, span + exit_span + 1.0),
            ),
        ] {
            add_road_editing_approach(&mut module, edge, road_editing_line(start, end), Vec::new());
        }
    }
    let approaches = if late_waiting {
        vec!["east-entry", "west-far", "north-entry", "south-far"]
    } else {
        vec!["east-entry", "west-exit", "north-entry", "south-exit"]
    };
    let internals = if late_waiting {
        vec!["east-internal", "west-exit", "north-internal", "south-exit"]
    } else {
        vec!["east-internal", "north-internal"]
    };
    module
        .add_declaration(lfre::RoadEditingDeclaration::Junction(
            lfre::JunctionInput::try_new(
                "crossing",
                approaches
                    .into_iter()
                    .map(|key| lfre::LaneEdgeReference::local(key).expect("approach edge"))
                    .collect(),
                internals
                    .into_iter()
                    .map(|key| lfre::LaneEdgeReference::local(key).expect("internal edge"))
                    .collect(),
            )
            .expect("junction"),
        ))
        .expect("add junction");
    if include_conflict {
        module
            .add_declaration(lfre::RoadEditingDeclaration::ConflictZone(
                lfre::ConflictZoneInput::try_new("center-zone", junction.clone())
                    .expect("conflict zone"),
            ))
            .expect("add conflict zone");
        if multiplicity || next_gate {
            module
                .add_declaration(lfre::RoadEditingDeclaration::ConflictZone(
                    lfre::ConflictZoneInput::try_new("secondary-zone", junction.clone())
                        .expect("secondary conflict zone"),
                ))
                .expect("add secondary conflict zone");
        }
    }

    for (
        stream_index,
        (
            movement_key,
            path_key,
            gate_key,
            stop_line_key,
            entry_edge,
            internal_edge,
            exit_edge,
            stream_key,
            entry_progress,
            exit_progress,
        ),
    ) in [
        (
            "east-west",
            "east-west-path",
            "east-west-gate",
            "east-stop",
            "east-entry",
            "east-internal",
            "west-exit",
            "east-west-stream",
            2.000_4,
            6.000_6,
        ),
        (
            "north-south",
            "north-south-path",
            "north-south-gate",
            "north-stop",
            "north-entry",
            "north-internal",
            "south-exit",
            "north-south-stream",
            1.500_4,
            5.500_6,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let movement = lfre::MovementReference::owner_scoped(vec!["crossing".into()], movement_key)
            .expect("movement reference");
        let path = lfre::ManeuverPathReference::owner_scoped(
            vec!["crossing".into(), movement_key.into()],
            path_key,
        )
        .expect("path reference");
        let admission_gate = lfre::ManeuverGateReference::owner_scoped(
            vec!["crossing".into(), movement_key.into(), path_key.into()],
            gate_key,
        )
        .expect("admission gate reference");
        let (entry_anchor, exit_anchor) = if terminal_clearance {
            (
                lfre::PathAnchorInput::gate(admission_gate.clone()),
                lfre::PathAnchorInput::edge_boundary(3),
            )
        } else if conflict_after_release {
            (
                lfre::PathAnchorInput::interior(2, 1.0).unwrap(),
                lfre::PathAnchorInput::interior(2, 5.0).unwrap(),
            )
        } else if late_waiting && late_claim_crosses && stream_index == 1 {
            (
                lfre::PathAnchorInput::interior(1, 10.0).expect("entry anchor"),
                lfre::PathAnchorInput::interior(1, 11.0).expect("exit anchor"),
            )
        } else if late_waiting {
            (
                lfre::PathAnchorInput::interior(1, 0.5).expect("entry anchor"),
                lfre::PathAnchorInput::interior(1, 1.5).expect("exit anchor"),
            )
        } else if span < 3.0 {
            (
                lfre::PathAnchorInput::interior(1, span * 0.2).expect("entry anchor"),
                lfre::PathAnchorInput::interior(1, span * 0.7).expect("exit anchor"),
            )
        } else {
            (
                lfre::PathAnchorInput::interior(1, entry_progress).expect("entry anchor"),
                lfre::PathAnchorInput::interior(
                    clearance.map_or(1, |value| value.0),
                    clearance.map_or(exit_progress, |value| value.1),
                )
                .expect("exit anchor"),
            )
        };
        let path_exit = if late_waiting {
            if exit_edge == "west-exit" {
                "west-far"
            } else {
                "south-far"
            }
        } else {
            exit_edge
        };
        let mut movement_input =
            lfre::MovementInput::try_new(movement_key, junction.clone(), entry_edge, path_exit)
                .expect("movement");
        if stream_index == 0 && right_turn_signal.is_some() {
            movement_input =
                movement_input.with_turn_direction(laneflow_compiler::ManeuverDirection::Right);
        }
        module
            .add_declaration(lfre::RoadEditingDeclaration::Movement(movement_input))
            .expect("add movement")
            .add_declaration(lfre::RoadEditingDeclaration::ManeuverPath(
                lfre::ManeuverPathInput::try_new(
                    path_key,
                    movement,
                    lfre::LaneEdgeReference::local(entry_edge).expect("entry edge"),
                    if late_waiting {
                        vec![
                            lfre::LaneEdgeReference::local(internal_edge).expect("internal edge"),
                            lfre::LaneEdgeReference::local(exit_edge).expect("middle edge"),
                        ]
                    } else {
                        vec![lfre::LaneEdgeReference::local(internal_edge).expect("internal edge")]
                    },
                    lfre::LaneEdgeReference::local(path_exit).expect("exit edge"),
                )
                .expect("maneuver path"),
            ))
            .expect("add maneuver path")
            .add_declaration(lfre::RoadEditingDeclaration::StopLine(
                lfre::StopLineInput::try_new(
                    stop_line_key,
                    lfre::LaneEdgeReference::local(entry_edge).expect("stop edge"),
                )
                .expect("stop line"),
            ))
            .expect("add stop line")
            .add_declaration(lfre::RoadEditingDeclaration::ManeuverGate(
                lfre::ManeuverGateInput::try_new(
                    gate_key,
                    path.clone(),
                    0,
                    lfre::StopLineReference::local(stop_line_key).expect("stop line reference"),
                    if stream_index == 0 && right_turn_signal.is_some() {
                        lfre::RoadEditingSignalControl::SignalGroup(
                            lfre::SignalGroupReference::local("right-turn-red").unwrap(),
                        )
                    } else if signal_cycle_ms.is_some() {
                        lfre::RoadEditingSignalControl::SignalGroup(
                            lfre::SignalGroupReference::local("priority-signal").unwrap(),
                        )
                    } else {
                        lfre::RoadEditingSignalControl::None
                    },
                )
                .expect("maneuver gate"),
            ))
            .expect("add maneuver gate");
        if waiting || next_gate || conflict_after_release || resource_free_release || late_waiting {
            let release_key = format!("{gate_key}-release");
            let release_stop = format!("{stop_line_key}-release");
            module
                .add_declaration(lfre::RoadEditingDeclaration::StopLine(
                    lfre::StopLineInput::try_new(
                        &release_stop,
                        lfre::LaneEdgeReference::local(internal_edge).unwrap(),
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(lfre::RoadEditingDeclaration::ManeuverGate(
                    lfre::ManeuverGateInput::try_new(
                        &release_key,
                        path.clone(),
                        1,
                        lfre::StopLineReference::local(&release_stop).unwrap(),
                        lfre::RoadEditingSignalControl::None,
                    )
                    .unwrap(),
                ))
                .unwrap();
            if late_waiting {
                let far_key = format!("{gate_key}-far");
                let far_stop = format!("{stop_line_key}-far");
                module
                    .add_declaration(lfre::RoadEditingDeclaration::StopLine(
                        lfre::StopLineInput::try_new(
                            &far_stop,
                            lfre::LaneEdgeReference::local(exit_edge).unwrap(),
                        )
                        .unwrap(),
                    ))
                    .unwrap()
                    .add_declaration(lfre::RoadEditingDeclaration::ManeuverGate(
                        lfre::ManeuverGateInput::try_new(
                            &far_key,
                            path.clone(),
                            2,
                            lfre::StopLineReference::local(&far_stop).unwrap(),
                            lfre::RoadEditingSignalControl::None,
                        )
                        .unwrap(),
                    ))
                    .unwrap();
            }
            if (waiting || late_waiting) && (!waiting_on_north_only || stream_index == 1) {
                let (entry_gate_ref, release_gate_ref) = if late_waiting {
                    (
                        lfre::ManeuverGateReference::owner_scoped(
                            vec!["crossing".into(), movement_key.into(), path_key.into()],
                            &release_key,
                        )
                        .unwrap(),
                        lfre::ManeuverGateReference::owner_scoped(
                            vec!["crossing".into(), movement_key.into(), path_key.into()],
                            format!("{gate_key}-far"),
                        )
                        .unwrap(),
                    )
                } else {
                    (
                        admission_gate.clone(),
                        lfre::ManeuverGateReference::owner_scoped(
                            vec!["crossing".into(), movement_key.into(), path_key.into()],
                            &release_key,
                        )
                        .unwrap(),
                    )
                };
                module
                    .add_declaration(lfre::RoadEditingDeclaration::WaitingZone(
                        lfre::WaitingZoneInput::try_new(
                            "waiting",
                            path.clone(),
                            entry_gate_ref,
                            release_gate_ref,
                            waiting_occupancy,
                        )
                        .unwrap(),
                    ))
                    .unwrap();
            }
        }
        if include_conflict && !multiplicity && stream_index < stream_count {
            module
                .add_declaration(lfre::RoadEditingDeclaration::ParticipantStream(
                    lfre::ParticipantStreamInput::try_new(stream_key, junction.clone(), path, {
                        let mut passages = vec![lfre::ConflictPassageInput::new(
                            zone.clone(),
                            entry_anchor,
                            exit_anchor,
                        )];
                        if next_gate {
                            let (second_entry, second_exit) =
                                if span < 3.0 { (0.3, 1.2) } else { (1.0, 4.0) };
                            passages.push(lfre::ConflictPassageInput::new(
                                secondary_zone.clone(),
                                lfre::PathAnchorInput::interior(2, second_entry).unwrap(),
                                lfre::PathAnchorInput::interior(2, second_exit).unwrap(),
                            ));
                        }
                        passages
                    })
                    .expect("participant stream"),
                ))
                .expect("add participant stream");
        }
    }

    if multiplicity {
        let path = lfre::ManeuverPathReference::owner_scoped(
            vec!["crossing".into(), "east-west".into()],
            "east-west-path",
        )
        .expect("multiplicity path reference");
        for (stream_key, intervals) in [
            ("east-west-stream-a", [(2.0, 6.0), (5.0, 11.0)]),
            ("east-west-stream-b", [(3.0, 7.0), (6.5, 10.0)]),
        ] {
            let passages = [zone.clone(), secondary_zone.clone()]
                .into_iter()
                .zip(intervals)
                .map(|(passage_zone, (entry, exit))| {
                    lfre::ConflictPassageInput::new(
                        passage_zone,
                        lfre::PathAnchorInput::interior(1, entry).expect("multiplicity entry"),
                        lfre::PathAnchorInput::interior(1, exit).expect("multiplicity exit"),
                    )
                })
                .collect();
            module
                .add_declaration(lfre::RoadEditingDeclaration::ParticipantStream(
                    lfre::ParticipantStreamInput::try_new(
                        stream_key,
                        junction.clone(),
                        path.clone(),
                        passages,
                    )
                    .expect("multiplicity participant stream"),
                ))
                .expect("add multiplicity participant stream");
        }
    }

    if include_conflict {
        module
            .add_conflict_zone_region(
                lfre::ConflictZoneRegionInput::try_new(
                    zone,
                    frame,
                    -1.000_000_000_1,
                    1.000_000_000_1,
                    [
                        (-1.000_000_000_1, -1.000_000_000_1),
                        (1.000_000_000_1, -1.000_000_000_1),
                        (1.000_000_000_1, 1.000_000_000_1),
                        (-1.000_000_000_1, 1.000_000_000_1),
                    ]
                    .into_iter()
                    .map(|(x, z)| lfre::RoadEditingPoint2::try_new(x, z).expect("region point"))
                    .collect(),
                )
                .expect("conflict zone region"),
            )
            .expect("add conflict zone region");
    }
    if span >= 11.0 && !late_waiting {
        module
            .add_declaration(lfre::RoadEditingDeclaration::ParkingFacility(
                lfre::ParkingFacilityInput::try_new("parking").expect("parking facility"),
            ))
            .expect("add parking facility")
            .add_declaration(lfre::RoadEditingDeclaration::ParkingSpace(
                lfre::ParkingSpaceInput::try_new(
                    "space",
                    lfre::ParkingLaneAnchor::try_new(
                        lfre::LaneEdgeReference::local(if early_parking_on_internal {
                            "east-internal"
                        } else {
                            "west-exit"
                        })
                        .expect("parking entry edge"),
                        if early_parking_on_internal { 3.0 } else { 12.0 },
                    )
                    .expect("parking entry"),
                    lfre::ParkingLaneAnchor::try_new(
                        lfre::LaneEdgeReference::local("east-internal").expect("parking exit edge"),
                        if early_parking_on_internal { 8.0 } else { 10.5 },
                    )
                    .expect("parking exit"),
                    lfre::ParkingSpaceGeometry::try_new(1.5, 0.0, 5.0, 2.5)
                        .expect("parking geometry"),
                )
                .expect("parking space")
                .with_parking_facility(
                    lfre::ParkingFacilityReference::local("parking")
                        .expect("parking facility reference"),
                ),
            ))
            .expect("add parking space");
    }
    let participant =
        lfre::ParticipantClassReference::local("road-user").expect("participant class reference");
    module
        .add_declaration(lfre::RoadEditingDeclaration::ParticipantClass(
            lfre::ParticipantClassInput::try_new("road-user").expect("participant class"),
        ))
        .expect("add participant class")
        .add_declaration(lfre::RoadEditingDeclaration::VehicleProfile(
            lfre::VehicleProfileInput::try_new(
                "car",
                participant.clone(),
                lfre::IidmVehicleProfileInput::try_new(
                    4.5,
                    desired_speed_meters_per_second,
                    2.0,
                    1.5,
                    1.5,
                    2.0,
                    4.0,
                )
                .expect("iidm profile"),
            )
            .expect("vehicle profile"),
        ))
        .expect("add vehicle profile");
    if short_vehicle {
        module
            .add_declaration(lfre::RoadEditingDeclaration::VehicleProfile(
                lfre::VehicleProfileInput::try_new(
                    "dot",
                    participant.clone(),
                    lfre::IidmVehicleProfileInput::try_new(
                        0.1,
                        desired_speed_meters_per_second,
                        0.2,
                        1.5,
                        1.5,
                        2.0,
                        4.0,
                    )
                    .expect("short vehicle iidm profile"),
                )
                .expect("short vehicle profile"),
            ))
            .expect("add short vehicle profile");
    }
    if include_long_vehicle {
        module
            .add_declaration(lfre::RoadEditingDeclaration::VehicleProfile(
                lfre::VehicleProfileInput::try_new(
                    "long-vehicle",
                    participant,
                    lfre::IidmVehicleProfileInput::try_new(
                        12.0,
                        desired_speed_meters_per_second,
                        3.0,
                        1.2,
                        1.5,
                        2.0,
                        4.0,
                    )
                    .expect("long vehicle iidm profile"),
                )
                .expect("long vehicle profile"),
            ))
            .expect("add long vehicle profile");
    }
    let mut policy_gates: Vec<_> = [
        ("east-west", "east-west-path", "east-west-gate"),
        ("north-south", "north-south-path", "north-south-gate"),
    ]
    .iter()
    .map(|(movement, path, gate)| {
        lfre::PolicyGateRuleInput::try_new(
            *gate,
            lfre::ManeuverGateReference::owner_scoped(
                vec!["crossing".into(), (*movement).into(), (*path).into()],
                *gate,
            )
            .unwrap(),
            None,
            if *movement == "east-west" {
                right_turn_signal.unwrap_or(laneflow_compiler::GateInterpretation::Uncontrolled)
            } else if signal_cycle_ms.is_some() {
                laneflow_compiler::GateInterpretation::ProtectedGroup
            } else {
                laneflow_compiler::GateInterpretation::Uncontrolled
            },
            if deny {
                laneflow_compiler::GateProhibition::Always
            } else {
                laneflow_compiler::GateProhibition::None
            },
            vec![],
        )
        .unwrap()
    })
    .collect();
    if waiting || next_gate || conflict_after_release || resource_free_release || late_waiting {
        for (movement, path, gate) in [
            ("east-west", "east-west-path", "east-west-gate-release"),
            (
                "north-south",
                "north-south-path",
                "north-south-gate-release",
            ),
        ] {
            policy_gates.push(
                lfre::PolicyGateRuleInput::try_new(
                    gate,
                    lfre::ManeuverGateReference::owner_scoped(
                        vec!["crossing".into(), movement.into(), path.into()],
                        gate,
                    )
                    .unwrap(),
                    None,
                    laneflow_compiler::GateInterpretation::Uncontrolled,
                    laneflow_compiler::GateProhibition::None,
                    vec![],
                )
                .unwrap(),
            );
        }
    }
    if late_waiting {
        for (movement, path, gate) in [
            ("east-west", "east-west-path", "east-west-gate-far"),
            ("north-south", "north-south-path", "north-south-gate-far"),
        ] {
            policy_gates.push(
                lfre::PolicyGateRuleInput::try_new(
                    gate,
                    lfre::ManeuverGateReference::owner_scoped(
                        vec!["crossing".into(), movement.into(), path.into()],
                        gate,
                    )
                    .unwrap(),
                    None,
                    laneflow_compiler::GateInterpretation::Uncontrolled,
                    laneflow_compiler::GateProhibition::None,
                    vec![],
                )
                .unwrap(),
            );
        }
    }
    let mut stream_keys = Vec::new();
    if include_conflict {
        if multiplicity {
            stream_keys.extend(["east-west-stream-a", "east-west-stream-b"]);
        } else {
            stream_keys.extend(
                ["east-west-stream", "north-south-stream"]
                    .into_iter()
                    .take(stream_count),
            );
        }
    }
    let gap_profile_key = if gap_values_ms.is_some() {
        "urban-conservative"
    } else {
        "urban-gap"
    };
    let policy_streams = stream_keys
        .iter()
        .enumerate()
        .map(|(index, key)| {
            let yield_to = if yielding && index == 0 {
                vec![
                    lfre::ParticipantStreamReference::owner_scoped(
                        vec!["crossing".into()],
                        "north-south-stream",
                    )
                    .unwrap(),
                ]
            } else {
                Vec::new()
            };
            lfre::PolicyStreamRuleInput::try_new(
                *key,
                lfre::ParticipantStreamReference::owner_scoped(vec!["crossing".into()], *key)
                    .unwrap(),
                None,
                if equal_priority {
                    0
                } else {
                    i32::try_from(index).expect("fixture stream priority")
                },
                yield_to,
                (yielding && index == 0).then(|| gap_profile_key.to_owned()),
                vec![],
            )
            .unwrap()
        })
        .collect();
    let gap_profiles = if yielding {
        let (lead, lag, clearance) = gap_values_ms.unwrap_or((500, 500, 0));
        vec![
            lfre::PolicyGapProfileInput::try_new(
                gap_profile_key,
                if gap_values_ms.is_some() {
                    "urban-conservative-v1"
                } else {
                    "fixture-1"
                },
                lead,
                lag,
                clearance,
            )
            .unwrap(),
        ]
    } else {
        Vec::new()
    };
    module
        .add_declaration(lfre::RoadEditingDeclaration::RightOfWayPolicySet(
            lfre::RightOfWayPolicySetInput::try_new(
                "conflict-policy",
                laneflow_compiler::RegulationIdentity::try_new("engineering", "fixture-1")
                    .unwrap()
                    .with_source("repository:runtime-fixture-1")
                    .unwrap(),
                vec![],
                gap_profiles,
                policy_streams,
                policy_gates,
            )
            .unwrap(),
        ))
        .unwrap();
    module.finish().expect("Road Editing module")
}

fn register_named(world: &mut TrafficWorld, keys: &[&str]) -> RouteHandle {
    const NS: &str = "city/runtime-coverage";
    let limits = CompileLimits::p100_initial_v1();
    let edges: Vec<_> = keys
        .iter()
        .map(|key| {
            let stable =
                derive_canonical_stable_id_v1(EntityKind::LaneEdge, NS, key, &limits).expect("id");
            world
                .revision()
                .identity()
                .ordinal(LaneEdgeId::from_untyped(stable))
                .expect(key)
        })
        .collect();
    world
        .register_route(RouteRegisterInput::new(edges))
        .expect("register")
}

fn add_standard_profiles(module: &mut SyntheticModuleBuilder) {
    module
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .expect("class")
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "car",
            participant_class: ParticipantClassReference::local("road-user"),
            iidm: iidm(),
        })
        .expect("profile");
}

fn compile_virtual_parking_revision(virtual_capacity: u32) -> Arc<SharedNetworkRevision> {
    compile_revision(|module| {
        let virtual_entries = [
            ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("edge"),
                progress_meters: 20.0,
            },
            ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("edge"),
                progress_meters: 30.0,
            },
        ];
        let virtual_exits = [
            ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("edge"),
                progress_meters: 70.0,
            },
            ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("edge"),
                progress_meters: 80.0,
            },
        ];
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge",
                length_meters: 100.0,
                speed_limit_meters_per_second: 15.0,
                successors: &[],
            })
            .expect("edge")
            .add_parking_facility(ParkingFacilityInput {
                parking_facility_key: "facility",
                virtual_capacity,
                virtual_entries: &virtual_entries,
                virtual_exits: &virtual_exits,
            })
            .expect("facility")
            .add_parking_space(ParkingSpaceInput {
                parking_space_key: "space",
                parking_facility: Some(ParkingFacilityReference::local("facility")),
                entry: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("edge"),
                    progress_meters: 90.0,
                },
                exit: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("edge"),
                    progress_meters: 95.0,
                },
                geometry: ParkingSpaceGeometryInput {
                    lateral_offset_meters: -3.0,
                    heading_offset_radians: 0.25,
                    length_meters: 5.5,
                    width_meters: 2.6,
                },
            })
            .expect("space");
    })
}

fn parked_virtual_world() -> (
    TrafficWorld,
    RouteHandle,
    ParkingFacilityOrdinal,
    laneflow_runtime::VehicleHandle,
) {
    let revision = compile_virtual_parking_revision(1);
    let mut world =
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100)).expect("install");
    let route = register_named(&mut world, &["edge"]);
    let facility = ParkingFacilityOrdinal::from_raw(0);
    let parked = world
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0),
            ParkingTarget::VirtualPool(facility),
        )
        .expect("parked virtual")
        .vehicle;
    (world, route, facility, parked)
}

fn compile_exit_topology_revision() -> Arc<SharedNetworkRevision> {
    compile_revision(|module| {
        let entries = [ParkingLaneAnchorInput {
            lane_edge: LaneEdgeReference::local("loop"),
            progress_meters: 8.0,
        }];
        let exits = [
            ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("loop"),
                progress_meters: 2.0,
            },
            ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("middle"),
                progress_meters: 2.0,
            },
        ];
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "loop",
                length_meters: 10.0,
                speed_limit_meters_per_second: 15.0,
                successors: &[LaneEdgeReference::local("middle")],
            })
            .expect("loop")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "middle",
                length_meters: 10.0,
                speed_limit_meters_per_second: 15.0,
                successors: &[LaneEdgeReference::local("loop")],
            })
            .expect("middle")
            .add_parking_facility(ParkingFacilityInput {
                parking_facility_key: "facility",
                virtual_capacity: 1,
                virtual_entries: &entries,
                virtual_exits: &exits,
            })
            .expect("facility");
    })
}

fn compile_rebind_revision() -> Arc<SharedNetworkRevision> {
    compile_revision(|module| {
        let entries = [ParkingLaneAnchorInput {
            lane_edge: LaneEdgeReference::local("tail"),
            progress_meters: 8.0,
        }];
        let exits = [ParkingLaneAnchorInput {
            lane_edge: LaneEdgeReference::local("tail"),
            progress_meters: 9.0,
        }];
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "left",
                length_meters: 10.0,
                speed_limit_meters_per_second: 15.0,
                successors: &[LaneEdgeReference::local("current")],
            })
            .expect("left")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "right",
                length_meters: 10.0,
                speed_limit_meters_per_second: 15.0,
                successors: &[LaneEdgeReference::local("current")],
            })
            .expect("right")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "current",
                length_meters: 10.0,
                speed_limit_meters_per_second: 15.0,
                successors: &[LaneEdgeReference::local("tail")],
            })
            .expect("current")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "tail",
                length_meters: 10.0,
                speed_limit_meters_per_second: 15.0,
                successors: &[],
            })
            .expect("tail")
            .add_parking_facility(ParkingFacilityInput {
                parking_facility_key: "facility",
                virtual_capacity: 1,
                virtual_entries: &entries,
                virtual_exits: &exits,
            })
            .expect("facility");
    })
}

#[test]
fn road_editing_conflict_fixture_closes_integer_passages_and_f32_region() {
    let revision = compile_road_editing_revision(conflict_road_editing_module());
    assert_eq!(
        revision
            .traffic()
            .entity_counts()
            .count(EntityKind::ConflictZone),
        1
    );
    assert_eq!(
        revision
            .traffic()
            .entity_counts()
            .count(EntityKind::ParticipantStream),
        2
    );

    let zone = revision
        .conflict()
        .conflict_zone(ConflictZoneOrdinal::from_raw(0))
        .expect("conflict zone");
    assert_eq!(
        zone.participant_streams(),
        &[
            ParticipantStreamOrdinal::from_raw(0),
            ParticipantStreamOrdinal::from_raw(1),
        ]
    );
    let conflict = revision.conflict();
    let junction = zone.junction();
    assert_eq!(
        conflict.junction_conflict_zones(junction),
        Some(&[ConflictZoneOrdinal::from_raw(0)][..])
    );
    assert_eq!(
        conflict.junction_participant_streams(junction),
        Some(
            &[
                ParticipantStreamOrdinal::from_raw(0),
                ParticipantStreamOrdinal::from_raw(1),
            ][..]
        )
    );

    let expected_progress = [[2_000, 6_001], [1_500, 5_501]];
    for (stream_raw, expected) in expected_progress.into_iter().enumerate() {
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(
                u32::try_from(stream_raw).expect("stream ordinal"),
            ))
            .expect("participant stream");
        assert_eq!(
            conflict.maneuver_path_participant_streams(stream.maneuver_path()),
            Some(
                &[ParticipantStreamOrdinal::from_raw(
                    u32::try_from(stream_raw).expect("stream ordinal"),
                )][..]
            )
        );
        let [passage] = stream.passages() else {
            panic!("fixture stream has exactly one passage");
        };
        assert_eq!(
            passage.entry(),
            ConflictPathAnchor::Interior {
                path_edge_index: 1,
                progress_millimetres: expected[0],
            }
        );
        assert_eq!(
            passage.exit(),
            ConflictPathAnchor::Interior {
                path_edge_index: 1,
                progress_millimetres: expected[1],
            }
        );
        let path = revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .expect("maneuver path");
        assert_eq!(path.maneuver_gates(), &[passage.admission_gate()]);
    }

    let region = revision
        .spatial()
        .expect("retained spatial component")
        .conflict_zone_region(ConflictZoneOrdinal::from_raw(0))
        .expect("conflict zone region");
    assert_eq!(region.height_range(), (-1.0, 1.0));
    assert_eq!(
        region.ring_xz(),
        &[
            laneflow_static_network::CanonicalPointXZ { x: -1.0, z: -1.0 },
            laneflow_static_network::CanonicalPointXZ { x: 1.0, z: -1.0 },
            laneflow_static_network::CanonicalPointXZ { x: 1.0, z: 1.0 },
            laneflow_static_network::CanonicalPointXZ { x: -1.0, z: 1.0 },
        ]
    );
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn conflict_routes_charge_independent_capacity_and_use_the_production_gate_path() {
    let revision = compile_road_editing_revision(conflict_road_editing_module());
    let stream = revision
        .conflict()
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("east-west stream");
    let route_edges = revision
        .traffic()
        .maneuvers()
        .maneuver_path(stream.maneuver_path())
        .expect("east-west path")
        .edges()
        .to_vec();

    let mut zero_capacity =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 0, 100))
            .expect("install zero-conflict-capacity world");
    let cursor_before = zero_capacity.command_cursor();
    assert_eq!(
        zero_capacity
            .register_route(RouteRegisterInput::new(route_edges.clone()))
            .unwrap_err(),
        RouteError::ConflictOccurrenceCapacityExceeded {
            current: 0,
            added: 1,
            capacity: 0,
        }
    );
    assert_eq!(zero_capacity.command_cursor(), cursor_before);
    assert_eq!(zero_capacity.live_routes().count(), 0);

    let mut world = install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 1, 100))
        .expect("install exact-conflict-capacity world");
    let route = world
        .register_route(RouteRegisterInput::new(route_edges.clone()))
        .expect("one conflict occurrence fits exactly");
    let cursor_after_first = world.command_cursor();
    assert_eq!(
        world
            .register_route(RouteRegisterInput::new(route_edges.clone()))
            .unwrap_err(),
        RouteError::ConflictOccurrenceCapacityExceeded {
            current: 1,
            added: 1,
            capacity: 1,
        }
    );
    assert_eq!(world.command_cursor(), cursor_after_first);
    assert_eq!(world.live_routes().count(), 1);

    let gate_length = revision.traffic().lane_lengths_millimetres()[route_edges[0].index()];
    let vehicle = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                gate_length,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("Gate upstream/boundary spawn is handled by production arbitration");
    world
        .step(TickInput::new(100))
        .expect("acquire and cross Gate");
    assert!(world.conflict_reservation(vehicle).is_some());
    assert!(
        world
            .latest_conflict_decisions()
            .iter()
            .any(|decision| decision.vehicle() == vehicle
                && decision.outcome() == laneflow_runtime::ConflictDecisionOutcome::Granted)
    );
    for _ in 0..20 {
        world.step(TickInput::new(100)).expect("clear passage");
        if world.conflict_reservation(vehicle).is_none() {
            break;
        }
    }
    assert!(
        world.conflict_reservation(vehicle).is_none(),
        "tail clearance releases reservation/downstream authority"
    );
    world
        .despawn_vehicle(vehicle)
        .expect("despawn active vehicle");
    world
        .remove_route(route)
        .expect("remove route releases charge");
    world
        .register_route(RouteRegisterInput::new(route_edges))
        .expect("released conflict capacity is reusable");
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn conflict_tick_arbitrates_the_canonical_post_gate_zero_position() {
    let revision = compile_road_editing_revision(conflict_road_editing_module_with_stream_count(2));
    let route_edges = [0_u32, 1].map(|raw| {
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
            .expect("fixture stream");
        revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .expect("fixture path")
            .edges()
            .to_vec()
    });
    let mut world = install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100))
        .expect("install conflict world");
    let routes = route_edges.map(|edges| {
        world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route")
    });
    let vehicles = routes.map(|route| {
        world
            .place_existing_active_vehicle(
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 1, 0, 10_000)
                    .with_open_entrance(),
            )
            .expect("canonical post-Gate zero position is still a Gate boundary")
    });

    world.step(TickInput::new(100)).expect("arbitrate boundary");

    let decisions = world.latest_conflict_decisions();
    assert_eq!(decisions.len(), 2);
    assert!(
        decisions
            .iter()
            .all(|decision| decision.anchor().hop() == 0)
    );
    assert_eq!(
        decisions
            .iter()
            .find(|decision| decision.vehicle() == vehicles[1])
            .expect("formal winner decision")
            .outcome(),
        laneflow_runtime::ConflictDecisionOutcome::Granted
    );
    assert_eq!(
        decisions
            .iter()
            .find(|decision| decision.vehicle() == vehicles[0])
            .expect("formal loser decision")
            .outcome(),
        laneflow_runtime::ConflictDecisionOutcome::NoGrant(
            laneflow_runtime::ConflictNoGrantReason::ConflictOccupied,
        )
    );
    assert!(world.conflict_reservation(vehicles[1]).is_some());
    assert!(world.conflict_reservation(vehicles[0]).is_none());
    let loser = world.vehicle(vehicles[0]).expect("loser remains active");
    assert_eq!(loser.route_edge_index(), 1);
    assert_eq!(loser.progress_mm(), 0);
    world
        .step(TickInput::new(100))
        .expect("retained canonical-boundary eligibility remains valid");
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn conflict_tick_uses_stable_single_writer_winner_and_retries_the_loser() {
    let revision = compile_road_editing_revision(conflict_road_editing_module_with_stream_count(2));
    let routes = [0_u32, 1].map(|raw| {
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
            .expect("fixture stream");
        revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .expect("stream path")
            .edges()
            .to_vec()
    });
    let mut world = install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100))
        .expect("install conflict world");
    let routes = routes.map(|edges| {
        world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route")
    });
    let mut vehicles = Vec::new();
    for route in routes {
        let edge = world.route_edges(route).expect("route edges")[0];
        let boundary = world.traffic().lane_lengths_millimetres()[edge.index()];
        vehicles.push(
            world
                .place_existing_active_vehicle(
                    VehicleSpawnInput::new(
                        VehicleProfileOrdinal::from_raw(0),
                        route,
                        0,
                        boundary,
                        10_000,
                    )
                    .with_open_entrance(),
                )
                .expect("Gate-boundary candidate"),
        );
    }

    world.step(TickInput::new(100)).expect("arbitrate");
    let decisions = world.latest_conflict_decisions();
    assert_eq!(decisions.len(), 2);
    let winner = vehicles[1];
    let loser = vehicles[0];
    assert_eq!(
        decisions
            .iter()
            .find(|decision| decision.vehicle() == winner)
            .expect("higher-priority decision")
            .outcome(),
        laneflow_runtime::ConflictDecisionOutcome::Granted,
    );
    assert_eq!(
        decisions
            .iter()
            .find(|decision| decision.vehicle() == loser)
            .expect("lower-priority decision")
            .outcome(),
        laneflow_runtime::ConflictDecisionOutcome::NoGrant(
            laneflow_runtime::ConflictNoGrantReason::ConflictOccupied,
        ),
    );
    assert!(world.conflict_reservation(winner).is_some());
    let loser_state = world.vehicle(loser).expect("loser remains active");
    assert_eq!(loser_state.route_edge_index(), 0);
    assert_eq!(loser_state.speed_mm_s(), 0);

    for _ in 0..40 {
        world.step(TickInput::new(100)).expect("retry loser");
        if world.conflict_reservation(loser).is_some() {
            break;
        }
    }
    assert!(
        world.conflict_reservation(loser).is_some(),
        "loser retries after the earlier reservation clears"
    );
}

#[test]
fn fresh_spawn_before_blocked_downstream_storage_must_stop() {
    let revision = compile_road_editing_revision(conflict_road_editing_module());
    let stream = revision
        .conflict()
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("fixture stream");
    let edges = revision
        .traffic()
        .maneuvers()
        .maneuver_path(stream.maneuver_path())
        .expect("path")
        .edges()
        .to_vec();
    let mut open =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 2, 64, 1, 100)).expect("open");
    let open_route = open
        .register_route(RouteRegisterInput::new(edges.clone()))
        .expect("route");
    let open_entry = open.route_edges(open_route).expect("route")[0];
    let open_gate = open.traffic().lane_lengths_millimetres()[open_entry.index()];
    assert!(open_gate > 400, "approach must leave room before the gate");
    open.spawn_vehicle(
        VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            open_route,
            0,
            open_gate - 400,
            10_000,
        )
        .with_open_entrance(),
    )
    .expect("clear downstream is not a mandatory stop");

    let mut blocked =
        install_fixture(revision, WorldConfig::new(4, 2, 64, 1, 100)).expect("blocked");
    let route = blocked
        .register_route(RouteRegisterInput::new(edges))
        .expect("route");
    blocked
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 1, 10_501, 0)
                .with_open_entrance(),
        )
        .expect("leader rear exactly clears passage");
    let entry = blocked.route_edges(route).expect("route")[0];
    let gate = blocked.traffic().lane_lengths_millimetres()[entry.index()];
    assert_eq!(
        blocked.spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                gate - 400,
                10_000,
            )
            .with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable)
    );
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn conflict_tick_rejects_when_committed_downstream_storage_is_blocked() {
    let revision = compile_road_editing_revision(conflict_road_editing_module());
    let stream = revision
        .conflict()
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("fixture stream");
    let edges = revision
        .traffic()
        .maneuvers()
        .maneuver_path(stream.maneuver_path())
        .expect("path")
        .edges()
        .to_vec();
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 2, 64, 1, 100)).expect("world");
    let route = world
        .register_route(RouteRegisterInput::new(edges))
        .expect("route");
    let leader = world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 1, 10_501, 0)
                .with_open_entrance(),
        )
        .expect("leader rear exactly clears passage");
    let entry = world.route_edges(route).expect("route")[0];
    let gate = world.traffic().lane_lengths_millimetres()[entry.index()];
    let subject = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, gate, 10_000)
                .with_open_entrance(),
        )
        .expect("subject");

    world.step(TickInput::new(100)).expect("normal no-grant");
    let decision = world
        .latest_conflict_decisions()
        .iter()
        .find(|decision| decision.vehicle() == subject)
        .expect("subject decision");
    assert_eq!(
        decision.outcome(),
        laneflow_runtime::ConflictDecisionOutcome::NoGrant(
            laneflow_runtime::ConflictNoGrantReason::DownstreamStorageBoundary,
        )
    );
    assert!(world.conflict_reservation(subject).is_none());
    assert_eq!(
        world.vehicle(subject).expect("subject").route_edge_index(),
        0
    );
    assert!(world.vehicle(leader).is_some());
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn permissive_conflict_uses_the_compiled_gap_profile_and_approach_frontier() {
    use laneflow_runtime::ConflictNoGrantReason;
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let route_edges = [0_u32, 1].map(|raw| {
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
            .expect("stream");
        revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .expect("path")
            .edges()
            .to_vec()
    });
    for (distance, reason) in [
        (0, ConflictNoGrantReason::ConflictOccupied),
        (2_000, ConflictNoGrantReason::LeadGap),
    ] {
        let mut world = install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100))
            .expect("world");
        assert_eq!(world.policy_gap_profiles()[0].required_lead_ms(), 600);
        assert_eq!(world.policy_gap_profiles()[0].required_lag_ms(), 500);
        let routes = route_edges.clone().map(|edges| {
            world
                .register_route(RouteRegisterInput::new(edges))
                .expect("route")
        });
        let subject_edge = world.route_edges(routes[0]).expect("subject route")[0];
        let foe_edge = world.route_edges(routes[1]).expect("foe route")[0];
        let subject = world
            .place_existing_active_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    routes[0],
                    0,
                    world.traffic().lane_lengths_millimetres()[subject_edge.index()],
                    10_000,
                )
                .with_open_entrance(),
            )
            .expect("yielding subject");
        let foe = world
            .place_existing_active_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    routes[1],
                    0,
                    world.traffic().lane_lengths_millimetres()[foe_edge.index()] - distance,
                    10_000,
                )
                .with_open_entrance(),
            )
            .expect("priority foe");

        world.step(TickInput::new(100)).expect("gap arbitration");
        assert_eq!(
            world
                .latest_conflict_decisions()
                .iter()
                .find(|decision| decision.vehicle() == subject)
                .expect("subject decision")
                .outcome(),
            laneflow_runtime::ConflictDecisionOutcome::NoGrant(reason)
        );
        assert_eq!(
            world.vehicle(subject).expect("subject").route_edge_index(),
            0
        );
        assert!(world.conflict_reservation(subject).is_none());
        assert_eq!(world.conflict_reservation(foe).is_some(), distance == 0);
    }
}

#[test]
fn fresh_spawn_before_a_free_conflict_is_not_a_mandatory_stop() {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    let stream = revision
        .conflict()
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("stream");
    let edges = revision
        .traffic()
        .maneuvers()
        .maneuver_path(stream.maneuver_path())
        .expect("path")
        .edges()
        .to_vec();
    let route = world
        .register_route(RouteRegisterInput::new(edges))
        .expect("route");
    let edge = world.route_edges(route).expect("route edges")[0];
    let length = world.traffic().lane_lengths_millimetres()[edge.index()];
    assert!(length > 400, "approach must leave room before the gate");
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("free conflict is not a mandatory stop");
}

#[test]
fn fresh_spawn_before_a_same_tick_contended_conflict_must_stop() {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let route_edges = [0_u32, 1].map(|raw| {
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
            .expect("stream");
        revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .expect("path")
            .edges()
            .to_vec()
    });
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    let routes = route_edges.map(|edges| {
        world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route")
    });
    let foe_edge = world.route_edges(routes[1]).expect("foe route")[0];
    let foe_length = world.traffic().lane_lengths_millimetres()[foe_edge.index()];
    assert!(foe_length > 800, "approach must leave a near-gate pose");
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                foe_length - 800,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("first vehicle still sees a free zone");
    let subject_edge = world.route_edges(routes[0]).expect("subject route")[0];
    let subject_length = world.traffic().lane_lengths_millimetres()[subject_edge.index()];
    assert!(
        subject_length > 400,
        "approach must leave room before the gate"
    );
    assert_eq!(
        world.spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                subject_length - 400,
                10_000,
            )
            .with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable)
    );
}

#[test]
fn fresh_spawn_ignores_a_conflict_contender_beyond_this_tick() {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let route_edges = [0_u32, 1].map(|raw| {
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
            .expect("stream");
        revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .expect("path")
            .edges()
            .to_vec()
    });
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    let routes = route_edges.map(|edges| {
        world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route")
    });
    let foe_edge = world.route_edges(routes[1]).expect("foe route")[0];
    let foe_length = world.traffic().lane_lengths_millimetres()[foe_edge.index()];
    assert!(
        foe_length > 8_000,
        "approach must leave a pose beyond one tick"
    );
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                5_000,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("far vehicle");
    let subject_edge = world.route_edges(routes[0]).expect("subject route")[0];
    let subject_length = world.traffic().lane_lengths_millimetres()[subject_edge.index()];
    assert!(
        subject_length > 400,
        "approach must leave room before the gate"
    );
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                subject_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("a vehicle beyond this tick does not take the grant");
}

fn yield_routes(
    world: &mut laneflow_runtime::TrafficWorld,
    revision: &laneflow_static_network::SharedNetworkRevision,
) -> [laneflow_runtime::RouteHandle; 2] {
    let route_edges = [0_u32, 1].map(|raw| {
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
            .expect("stream");
        revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .expect("path")
            .edges()
            .to_vec()
    });
    route_edges.map(|edges| {
        world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route")
    })
}

#[test]
fn fresh_spawn_stops_for_a_yield_gap_beyond_this_tick() {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    let foe_edge = world.route_edges(routes[1]).expect("foe route")[0];
    let foe_length = world.traffic().lane_lengths_millimetres()[foe_edge.index()];
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                foe_length - 2_500,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("foe beyond one tick");
    let subject_edge = world.route_edges(routes[0]).expect("subject route")[0];
    let subject_length = world.traffic().lane_lengths_millimetres()[subject_edge.index()];
    assert_eq!(
        world.spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                subject_length - 400,
                10_000,
            )
            .with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable)
    );
}

#[test]
fn fresh_spawn_rejects_a_later_foe_that_would_hard_stop_the_yield_vehicle() {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    let subject_edge = world.route_edges(routes[0]).expect("subject route")[0];
    let subject_length = world.traffic().lane_lengths_millimetres()[subject_edge.index()];
    let subject = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                subject_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("yield vehicle is alone");
    let foe_edge = world.route_edges(routes[1]).expect("foe route")[0];
    let foe_length = world.traffic().lane_lengths_millimetres()[foe_edge.index()];
    assert_eq!(
        world.spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                foe_length - 2_500,
                10_000,
            )
            .with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable)
    );
    assert_eq!(
        world.vehicle(subject).map(|state| state.handle()),
        Some(subject)
    );
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn contender_reserve_failure_is_not_a_stop_constraint() {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    laneflow_runtime::set_contender_reserve_failure(true);
    let cursor = world.command_cursor();
    let result = world.spawn_vehicle(
        VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[0], 0, 0, 0)
            .with_open_entrance(),
    );
    laneflow_runtime::set_contender_reserve_failure(false);
    assert_eq!(result, Err(SpawnError::OccupancyAllocFailed));
    assert_eq!(world.command_cursor(), cursor);
    assert!(world.live_vehicles().is_empty());
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn contender_note_reserve_failure_is_not_a_stop_constraint() {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    laneflow_runtime::set_contender_note_reserve_failure(true);
    let cursor = world.command_cursor();
    let result = world.spawn_vehicle(
        VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[0], 0, 0, 0)
            .with_open_entrance(),
    );
    laneflow_runtime::set_contender_note_reserve_failure(false);
    assert_eq!(result, Err(SpawnError::OccupancyAllocFailed));
    assert_eq!(world.command_cursor(), cursor);
    assert!(world.live_vehicles().is_empty());
}

#[test]
fn fresh_spawn_allows_a_foe_that_reaches_the_stop_line_but_not_the_entrance() {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    let foe_edge = world.route_edges(routes[1]).expect("foe route")[0];
    let foe_length = world.traffic().lane_lengths_millimetres()[foe_edge.index()];
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                foe_length - 5_500,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("foe");
    let subject_edge = world.route_edges(routes[0]).expect("subject route")[0];
    let subject_length = world.traffic().lane_lengths_millimetres()[subject_edge.index()];
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                subject_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("time to the entrance is outside the lead");
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn fresh_spawn_ignores_a_downstream_claim_on_another_route() {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    let foe_edge = world.route_edges(routes[1]).expect("foe route")[0];
    let foe_length = world.traffic().lane_lengths_millimetres()[foe_edge.index()];
    let foe = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                foe_length,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("foe at the gate");
    world
        .step(TickInput::new(100))
        .expect("foe takes the reservation");
    assert!(
        world.conflict_reservation(foe).is_some(),
        "foe should hold a committed reservation"
    );
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[0], 0, 0, 0)
                .with_open_entrance(),
        )
        .expect("a claim on the other route does not block a far spawn");
}

#[test]
fn fresh_spawn_after_a_step_still_allows_a_free_conflict() {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    world.step(TickInput::new(100)).expect("empty step");
    let stream = revision
        .conflict()
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("stream");
    let edges = revision
        .traffic()
        .maneuvers()
        .maneuver_path(stream.maneuver_path())
        .expect("path")
        .edges()
        .to_vec();
    let route = world
        .register_route(RouteRegisterInput::new(edges))
        .expect("route");
    let edge = world.route_edges(route).expect("route edges")[0];
    let length = world.traffic().lane_lengths_millimetres()[edge.index()];
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("a dirty empty downstream index does not block a free conflict");
}

#[test]
fn fresh_spawn_is_not_denied_by_a_foe_who_can_stop_for_red() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                yielding: true,
                right_turn_signal: Some(laneflow_compiler::GateInterpretation::PermissiveGroup),
                signal_cycle_ms: Some([100, 10_000]),
                signal_stop_aspect: Some(SignalAspect::Red),
                gap_values_ms: Some((5_000, 500, 0)),
                ..ConflictPolicyFixture::default()
            },
        ));
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    world
        .step(TickInput::new(100))
        .expect("enter the red phase");
    let routes = yield_routes(&mut world, revision.as_ref());
    let foe_edge = world.route_edges(routes[1]).expect("foe route")[0];
    let foe_length = world.traffic().lane_lengths_millimetres()[foe_edge.index()];
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                foe_length - 2_000,
                3_000,
            )
            .with_open_entrance(),
        )
        .expect("foe can stop for red");
    let subject_edge = world.route_edges(routes[0]).expect("subject route")[0];
    let subject_length = world.traffic().lane_lengths_millimetres()[subject_edge.index()];
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                subject_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("a foe held at red does not take the gap");
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn fresh_spawn_stops_while_the_yield_lag_has_not_elapsed() {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    let foe_edge = world.route_edges(routes[1]).expect("foe route")[0];
    let foe_length = world.traffic().lane_lengths_millimetres()[foe_edge.index()];
    let foe = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                foe_length,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("foe already at the gate");
    let mut cleared = false;
    for _ in 0..40 {
        world.step(TickInput::new(100)).expect("step");
        if world.conflict_reservation(foe).is_none()
            && world
                .vehicle(foe)
                .is_some_and(|state| state.route_edge_index() > 0)
        {
            cleared = true;
            break;
        }
    }
    assert!(cleared, "foe should clear the conflict");
    let subject_edge = world.route_edges(routes[0]).expect("subject route")[0];
    let subject_length = world.traffic().lane_lengths_millimetres()[subject_edge.index()];
    assert_eq!(
        world.spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                subject_length - 400,
                10_000,
            )
            .with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable)
    );
}

#[test]
fn fresh_spawn_waiting_fullness_follows_this_tick_reach() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                waiting: true,
                waiting_on_north_only: true,
                conflict_after_release: true,
                ..ConflictPolicyFixture::default()
            },
        ));
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    let route = routes[1];
    let edge = world.route_edges(route).expect("route")[0];
    let length = world.traffic().lane_lengths_millimetres()[edge.index()];
    let occupant = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                length - 2_000,
                3_000,
            )
            .with_open_entrance(),
        )
        .expect("occupant can reach the entrance");
    for _ in 0..80 {
        world.step(TickInput::new(100)).expect("step");
        if world.vehicle(occupant).is_some_and(|state| {
            state.waiting_membership().is_some()
                && state.route_edge_index() >= 1
                && state.progress_mm() >= 8_000
        }) {
            break;
        }
    }
    assert!(
        world.vehicle(occupant).is_some_and(|state| {
            state.waiting_membership().is_some() && state.progress_mm() >= 8_000
        }),
        "occupant should be inside the waiting zone with the tail clear of the approach"
    );
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0, 1_000)
                .with_open_entrance(),
        )
        .expect("a full entrance beyond this tick still allows spawn");
    assert_eq!(
        world.spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                length - 200,
                10_000,
            )
            .with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable)
    );
}

#[test]
fn fresh_spawn_waiting_slot_loss_without_a_conflict_at_the_entrance() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                waiting: true,
                waiting_on_north_only: true,
                conflict_after_release: true,
                short_vehicle: true,
                ..ConflictPolicyFixture::default()
            },
        ));
    let dot = conflict_profile(revision.as_ref(), "dot");
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    let route = routes[1];
    let edge = world.route_edges(route).expect("route")[0];
    let length = world.traffic().lane_lengths_millimetres()[edge.index()];
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(dot, route, 0, length - 400, 10_000).with_open_entrance(),
        )
        .expect("farther short vehicle still holds the only waiting slot");
    assert_eq!(
        world.spawn_vehicle(
            VehicleSpawnInput::new(dot, route, 0, length - 150, 10_000,).with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable),
        "taking the only waiting slot must stop the spawn before the follower gap is reported"
    );
}

fn conflict_profile(revision: &SharedNetworkRevision, key: &str) -> VehicleProfileOrdinal {
    let stable = derive_canonical_stable_id_v1(
        EntityKind::VehicleProfile,
        "city/runtime-conflict",
        key,
        &CompileLimits::p100_initial_v2(),
    )
    .expect("profile identity");
    revision
        .identity()
        .ordinal(VehicleProfileId::from_untyped(stable))
        .expect("profile ordinal")
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn replace_keeps_the_completed_vehicle_rank() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                equal_priority: true,
                ..ConflictPolicyFixture::default()
            },
        ));
    let mut later =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 4, 100)).expect("later");
    let later_routes = yield_routes(&mut later, revision.as_ref());
    let foe_edge = later.route_edges(later_routes[1]).expect("foe route")[0];
    let foe_length = later.traffic().lane_lengths_millimetres()[foe_edge.index()];
    later
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                later_routes[1],
                0,
                foe_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("stopped contender already at the gate");
    let subject_edge = later.route_edges(later_routes[0]).expect("subject route")[0];
    let subject_length = later.traffic().lane_lengths_millimetres()[subject_edge.index()];
    assert_eq!(
        later.spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                later_routes[0],
                0,
                subject_length - 400,
                10_000,
            )
            .with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable),
        "a later rank loses to the stopped contender"
    );

    let mut world = install_fixture(revision, WorldConfig::new(4, 4, 64, 4, 100)).expect("replace");
    let held = world.revision();
    let routes = yield_routes(&mut world, held.as_ref());
    let edges = world.route_edges(routes[0]).expect("edges").to_vec();
    let last_index = u32::try_from(edges.len() - 1).expect("index fits u32");
    let last_length =
        world.traffic().lane_lengths_millimetres()[edges[last_index as usize].index()];
    let completed = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                last_index,
                last_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("spawn at the route end");
    world.step(TickInput::new(100)).expect("complete");
    assert_eq!(
        world.vehicle(completed).expect("retained").status(),
        VehicleStatus::Completed
    );
    let foe_edge = world.route_edges(routes[1]).expect("foe route")[0];
    let foe_length = world.traffic().lane_lengths_millimetres()[foe_edge.index()];
    let contender = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                foe_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("later stopped contender");
    assert_eq!(world.live_vehicles(), &[completed, contender]);
    let subject_edge = world.route_edges(routes[0]).expect("subject route")[0];
    let subject_length = world.traffic().lane_lengths_millimetres()[subject_edge.index()];
    let record = world
        .replace_completed_vehicle(
            completed,
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                subject_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("the completed vehicle's old rank still sorts first");
    assert!(world.vehicle(record.new).is_some());
}

fn assert_step_stays_in_emergency_envelope(before_speed: u32, after_speed: u32, travel_mm: u32) {
    assert!(
        after_speed.saturating_add(400) >= before_speed,
        "speed {before_speed} -> {after_speed} drops faster than 4 m/s²"
    );
    if before_speed > 400 {
        assert!(
            travel_mm >= 900,
            "hard clamp traveled {travel_mm} mm from {before_speed}"
        );
    }
}

#[test]
fn accepted_spawn_step_stays_inside_the_emergency_envelope() {
    let revision = compile_road_editing_revision(conflict_road_editing_module());
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    let handle = world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[0], 0, 0, 10_000)
                .with_open_entrance(),
        )
        .expect("far from the gate");
    let before = world.vehicle(handle).expect("spawned");
    world.step(TickInput::new(100)).expect("step");
    let after = world.vehicle(handle).expect("still live");
    let travel = after.progress_mm().saturating_sub(before.progress_mm());
    assert_step_stays_in_emergency_envelope(before.speed_mm_s(), after.speed_mm_s(), travel);
}

#[test]
fn leader_preview_includes_the_candidate_approach() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                yielding: true,
                ..ConflictPolicyFixture::default()
            },
        ));
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(8, 4, 64, 4, 100)).expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    let yield_edge = world.route_edges(routes[0]).expect("yield route")[0];
    let yield_length = world.traffic().lane_lengths_millimetres()[yield_edge.index()];
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                yield_length.saturating_sub(5),
                0,
            )
            .with_open_entrance(),
        )
        .expect("earlier yielding vehicle already at the gate");
    let priority_edges = world.route_edges(routes[1]).expect("priority").to_vec();
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[1], 2, 2_000, 0)
                .with_open_entrance(),
        )
        .unwrap_or_else(|error| {
            panic!(
                "stopped leader on the next edge, edges {}: {error}",
                priority_edges.len()
            )
        });
    let cursor = world.command_cursor();
    let admitted = world.spawn_vehicle(
        VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            routes[1],
            0,
            1_000,
            10_000,
        )
        .with_open_entrance(),
    );
    assert!(
        admitted.is_ok(),
        "candidate approach must keep the earlier yielder from inventing UnsafeLeader, got {admitted:?}, cursor {cursor}"
    );
    assert!(world.command_cursor() > cursor);
}

#[test]
fn priority_then_yield_does_not_newly_stop_the_first_vehicle_past_the_envelope() {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    let priority_edge = world.route_edges(routes[1]).expect("priority")[0];
    let priority_length = world.traffic().lane_lengths_millimetres()[priority_edge.index()];
    let priority = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                priority_length - 2_500,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("priority vehicle is far enough to enter");
    let yield_edge = world.route_edges(routes[0]).expect("yield")[0];
    let yield_length = world.traffic().lane_lengths_millimetres()[yield_edge.index()];
    let yield_spawn = world.spawn_vehicle(
        VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            routes[0],
            0,
            yield_length - 400,
            10_000,
        )
        .with_open_entrance(),
    );
    if yield_spawn.is_ok() {
        let before = world.vehicle(priority).expect("priority remains");
        let before_speed = before.speed_mm_s();
        let before_progress = before.progress_mm();
        world.step(TickInput::new(100)).expect("step");
        let after = world.vehicle(priority).expect("priority after step");
        assert_step_stays_in_emergency_envelope(
            before_speed,
            after.speed_mm_s(),
            after.progress_mm().saturating_sub(before_progress),
        );
    }
}

#[test]
fn a_far_spawn_is_not_rejected_for_someone_else_at_another_gate() {
    let revision = compile_road_editing_revision(conflict_road_editing_module());
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    let gate_edge = world.route_edges(routes[1]).expect("gate route")[0];
    let gate_length = world.traffic().lane_lengths_millimetres()[gate_edge.index()];
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                gate_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("stopped vehicle at the other gate");
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[0], 0, 0, 0)
                .with_open_entrance(),
        )
        .expect("a far vehicle is not blamed for the other gate");
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn later_live_rank_loses_and_an_earlier_rank_keeps_the_gate() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                equal_priority: true,
                ..ConflictPolicyFixture::default()
            },
        ));
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(6, 6, 64, 4, 100)).expect("world");
    let held = world.revision();
    let routes = yield_routes(&mut world, held.as_ref());
    let foe_edge = world.route_edges(routes[1]).expect("foe")[0];
    let foe_length = world.traffic().lane_lengths_millimetres()[foe_edge.index()];
    let contender = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                foe_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("contender at the front of the live order");
    let edges = world.route_edges(routes[0]).expect("edges").to_vec();
    let last_index = u32::try_from(edges.len() - 1).expect("index");
    let last_length =
        world.traffic().lane_lengths_millimetres()[edges[last_index as usize].index()];
    let completed = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                last_index,
                last_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("completed vehicle behind the contender");
    world.step(TickInput::new(100)).expect("complete");
    assert_eq!(
        world.vehicle(completed).expect("completed").status(),
        VehicleStatus::Completed
    );
    assert_eq!(world.live_vehicles(), &[contender, completed]);
    let subject_edge = world.route_edges(routes[0]).expect("subject")[0];
    let subject_length = world.traffic().lane_lengths_millimetres()[subject_edge.index()];
    let rejected = world.replace_completed_vehicle(
        completed,
        VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            routes[0],
            0,
            subject_length - 400,
            10_000,
        )
        .with_open_entrance(),
    );
    assert!(
        matches!(rejected, Err(ReplaceError::StopConstraintUnsatisfiable)),
        "the back of the live order loses to the earlier contender, got {rejected:?}"
    );
    assert!(world.vehicle(contender).is_some());
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn incremental_contender_update_matches_a_forced_rebuild_without_scanning_everyone() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                short_vehicle: true,
                ..ConflictPolicyFixture::default()
            },
        ));
    let dot = conflict_profile(revision.as_ref(), "dot");
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(8, 4, 64, 4, 100)).expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    for progress in [0_u32, 2_000, 4_000] {
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(dot, routes[1], 0, progress, 0).with_open_entrance(),
            )
            .expect("parked far from the north gate");
    }
    let east_edge = world.route_edges(routes[0]).expect("east")[0];
    let east_length = world.traffic().lane_lengths_millimetres()[east_edge.index()];
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(dot, routes[0], 0, east_length - 140, 1_000)
                .with_open_entrance(),
        )
        .expect("short vehicle still reaches the east gate");
    laneflow_runtime::reset_contender_update_counts();
    world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(dot, routes[0], 0, east_length - 20, 0).with_open_entrance(),
        )
        .expect("leader already in front");
    let visits = laneflow_runtime::incremental_contender_visits();
    let scans = laneflow_runtime::contender_rebuild_scans();
    assert!(
        scans < world.live_vehicles().len() as u64,
        "admission scanned {scans} of {} vehicles",
        world.live_vehicles().len()
    );
    assert!(
        visits < world.live_vehicles().len() as u64,
        "refresh visited {visits} of {} vehicles",
        world.live_vehicles().len()
    );
    let before = world.contender_fingerprint_for_test();
    world.force_rebuild_contenders_for_test();
    assert_eq!(before, world.contender_fingerprint_for_test());
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn each_admission_reserve_failure_is_occupancy_alloc_and_commits_nothing() {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    for kind in [
        laneflow_runtime::AdmissionReserve::Best,
        laneflow_runtime::AdmissionReserve::Cell,
        laneflow_runtime::AdmissionReserve::Waiting,
        laneflow_runtime::AdmissionReserve::Downstream,
        laneflow_runtime::AdmissionReserve::Order,
    ] {
        let mut world = install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100))
            .expect("world");
        let routes = yield_routes(&mut world, revision.as_ref());
        let edge = world.route_edges(routes[0]).expect("route")[0];
        let length = world.traffic().lane_lengths_millimetres()[edge.index()];
        if kind == laneflow_runtime::AdmissionReserve::Order {
            let foe = world.route_edges(routes[1]).expect("foe")[0];
            let foe_length = world.traffic().lane_lengths_millimetres()[foe.index()];
            world
                .place_existing_active_vehicle(
                    VehicleSpawnInput::new(
                        VehicleProfileOrdinal::from_raw(0),
                        routes[1],
                        0,
                        foe_length,
                        0,
                    )
                    .with_open_entrance(),
                )
                .expect("earlier contender");
        }
        let cursor = world.command_cursor();
        let live_before = world.live_vehicles().len();
        laneflow_runtime::set_admission_reserve_failure(kind, true);
        let progress = if kind == laneflow_runtime::AdmissionReserve::Downstream
            || kind == laneflow_runtime::AdmissionReserve::Order
        {
            length - 400
        } else {
            0
        };
        let speed = if progress == 0 { 0 } else { 10_000 };
        let result = world.spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                progress,
                speed,
            )
            .with_open_entrance(),
        );
        laneflow_runtime::set_admission_reserve_failure(kind, false);
        assert_eq!(
            result,
            Err(SpawnError::OccupancyAllocFailed),
            "reserve {kind:?} must not look like a stop"
        );
        assert_eq!(world.command_cursor(), cursor);
        assert_eq!(world.live_vehicles().len(), live_before);
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[0], 0, 0, 0)
                    .with_open_entrance(),
            )
            .expect("the same far spawn works after the fault clears");
    }
}

#[test]
fn cutover_does_not_reuse_the_previous_generation_contender_cache() {
    let (base, target, diff, binding) = compile_conflict_cutover_pair(
        conflict_road_editing_module(),
        conflict_road_editing_module_with_vehicle_speed(12.5),
    );
    let mut world =
        install_fixture(Arc::clone(&base), WorldConfig::new(4, 4, 64, 4, 100)).expect("base");
    let routes = yield_routes(&mut world, base.as_ref());
    let edge = world.route_edges(routes[0]).expect("route")[0];
    let length = world.traffic().lane_lengths_millimetres()[edge.index()];
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("build a contender cache before cutover");
    let remembered = world.observation_state_sequence();
    let descriptor = NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*base.canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(*target.canonical_origin()),
        Some(binding),
        MigrationPolicyKind::CrossRevisionDirect,
        world.world_binding(),
    );
    let mut transaction = world
        .prepare_cross_revision_cutover(
            Arc::clone(&target),
            published_source(&target, "fixture://admission-cutover"),
            &descriptor,
            &diff,
            &CutoverPreflightLimits::new(1_048_576),
            &CutoverTransactionLimits::default(),
        )
        .expect("prepare");
    assert!(transaction.pump(&mut world).expect("catch up").caught_up);
    let _commit = transaction.commit(&mut world).expect("commit");
    let mut steps = 0u32;
    while world.observation_state_sequence() != remembered && steps < 64 {
        world
            .step(TickInput::new(100))
            .expect("step without spawning");
        steps += 1;
    }
    assert_eq!(world.observation_state_sequence(), remembered);
    let mut fresh = install_fixture(target, WorldConfig::new(4, 4, 64, 4, 100)).expect("fresh");
    let fresh_held = fresh.revision();
    let fresh_routes = yield_routes(&mut fresh, fresh_held.as_ref());
    let carried_held = world.revision();
    let carried = yield_routes(&mut world, carried_held.as_ref());
    let on_carried = world.spawn_vehicle(
        VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), carried[0], 0, 0, 0)
            .with_open_entrance(),
    );
    let on_fresh = fresh.spawn_vehicle(
        VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), fresh_routes[0], 0, 0, 0)
            .with_open_entrance(),
    );
    assert_eq!(
        on_carried.is_ok(),
        on_fresh.is_ok(),
        "after the sequence comes back, spawn must match a rebuilt target world"
    );
}

fn close_resource_gates() -> Arc<SharedNetworkRevision> {
    compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
        2,
        false,
        true,
        false,
        13.0,
        ConflictPolicyFixture {
            next_gate: true,
            short_vehicle: true,
            close_internal_m: 0.8,
            ..ConflictPolicyFixture::default()
        },
    ))
}

fn assert_not_past_second_gate(edge: u32, progress: u32) {
    assert!(
        edge < 2 || progress == 0,
        "step entered past the second resource gate at edge {edge} progress {progress}"
    );
}

#[test]
fn second_resource_gate_within_reach_matches_the_following_step() {
    let revision = close_resource_gates();
    let dot = conflict_profile(revision.as_ref(), "dot");
    let mut world = install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 8, 1_000))
        .expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    let edge = world.route_edges(routes[0]).expect("route")[0];
    let length = world.traffic().lane_lengths_millimetres()[edge.index()];
    let progress = length - 200;
    let handle = world
        .spawn_vehicle(
            VehicleSpawnInput::new(dot, routes[0], 0, progress, 2_000).with_open_entrance(),
        )
        .expect("a slow vehicle can stop at the second gate");
    let before = world.vehicle(handle).expect("spawned");
    world.step(TickInput::new(1_000)).expect("step");
    let after = world.vehicle(handle).expect("still live");
    let traveled = if after.route_edge_index() == before.route_edge_index() {
        after.progress_mm().saturating_sub(before.progress_mm())
    } else {
        length
            .saturating_sub(before.progress_mm())
            .saturating_add(after.progress_mm())
    };
    assert!(
        after.speed_mm_s().saturating_add(4_000) >= before.speed_mm_s(),
        "speed dropped faster than 4 m/s² over one second"
    );
    if after.speed_mm_s() == 0 {
        assert!(
            traveled >= 500,
            "full stop traveled {traveled} mm from {}",
            before.speed_mm_s()
        );
    }
    assert!(
        after.route_edge_index() >= 1,
        "the first gate was clear, so the step should pass it"
    );
    assert_not_past_second_gate(after.route_edge_index(), after.progress_mm());
    #[cfg(feature = "placement-fixtures")]
    {
        let mut oracle =
            install_fixture(revision, WorldConfig::new(4, 4, 64, 8, 1_000)).expect("oracle");
        let oracle_held = oracle.revision();
        let oracle_routes = yield_routes(&mut oracle, oracle_held.as_ref());
        let placed = oracle
            .place_existing_active_vehicle(
                VehicleSpawnInput::new(dot, oracle_routes[0], 0, progress, 2_000)
                    .with_open_entrance(),
            )
            .expect("same pose without admission");
        oracle.step(TickInput::new(1_000)).expect("oracle step");
        let stepped = oracle.vehicle(placed).expect("oracle vehicle");
        assert_eq!(after.route_edge_index(), stepped.route_edge_index());
        assert_eq!(after.progress_mm(), stepped.progress_mm());
        assert_eq!(after.speed_mm_s(), stepped.speed_mm_s());
    }
}

#[test]
fn second_resource_gate_beyond_this_tick_still_allows_the_spawn() {
    let revision = close_resource_gates();
    let dot = conflict_profile(revision.as_ref(), "dot");
    let mut world = install_fixture(revision, WorldConfig::new(4, 4, 64, 8, 100)).expect("world");
    let held = world.revision();
    let routes = yield_routes(&mut world, held.as_ref());
    let edge = world.route_edges(routes[0]).expect("route")[0];
    let length = world.traffic().lane_lengths_millimetres()[edge.index()];
    let handle = world
        .spawn_vehicle(
            VehicleSpawnInput::new(dot, routes[0], 0, length - 400, 10_000).with_open_entrance(),
        )
        .expect("the second gate is beyond this tick");
    let before = world.vehicle(handle).expect("spawned");
    world.step(TickInput::new(100)).expect("step");
    let after = world.vehicle(handle).expect("still live");
    let traveled = if after.route_edge_index() == before.route_edge_index() {
        after.progress_mm().saturating_sub(before.progress_mm())
    } else {
        length
            .saturating_sub(before.progress_mm())
            .saturating_add(after.progress_mm())
    };
    assert_step_stays_in_emergency_envelope(before.speed_mm_s(), after.speed_mm_s(), traveled);
}

#[test]
fn fast_vehicle_reaching_the_second_gate_is_not_cleared_past_the_envelope() {
    let revision = close_resource_gates();
    let dot = conflict_profile(revision.as_ref(), "dot");
    let mut world = install_fixture(revision, WorldConfig::new(4, 4, 64, 8, 100)).expect("world");
    let held = world.revision();
    let routes = yield_routes(&mut world, held.as_ref());
    let edge = world.route_edges(routes[0]).expect("route")[0];
    let length = world.traffic().lane_lengths_millimetres()[edge.index()];
    match world.spawn_vehicle(
        VehicleSpawnInput::new(dot, routes[0], 0, length - 80, 10_000).with_open_entrance(),
    ) {
        Err(SpawnError::StopConstraintUnsatisfiable) => {}
        Ok(handle) => {
            let before = world.vehicle(handle).expect("spawned");
            world.step(TickInput::new(100)).expect("step");
            let after = world.vehicle(handle).expect("still live");
            let traveled = if after.route_edge_index() == before.route_edge_index() {
                after.progress_mm().saturating_sub(before.progress_mm())
            } else {
                length.saturating_sub(before.progress_mm()) + after.progress_mm()
            };
            assert_step_stays_in_emergency_envelope(
                before.speed_mm_s(),
                after.speed_mm_s(),
                traveled,
            );
            assert_not_past_second_gate(after.route_edge_index(), after.progress_mm());
        }
        other => panic!("unexpected admission result {other:?}"),
    }
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn unacquired_downstream_claim_does_not_block_the_other_exit() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                equal_priority: true,
                ..ConflictPolicyFixture::default()
            },
        ));
    let mut open =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 4, 100)).expect("open");
    let open_routes = yield_routes(&mut open, revision.as_ref());
    let open_east = open.route_edges(open_routes[0]).expect("east")[0];
    let open_east_length = open.traffic().lane_lengths_millimetres()[open_east.index()];
    open.place_existing_active_vehicle(
        VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            open_routes[0],
            0,
            open_east_length - 400,
            10_000,
        )
        .with_open_entrance(),
    )
    .expect("east can still acquire");
    let open_north = open.route_edges(open_routes[1]).expect("north")[0];
    let open_north_length = open.traffic().lane_lengths_millimetres()[open_north.index()];
    assert_eq!(
        open.spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                open_routes[1],
                0,
                open_north_length - 400,
                10_000,
            )
            .with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable),
        "an earlier vehicle who can acquire still blocks the other exit"
    );

    let mut blocked =
        install_fixture(revision, WorldConfig::new(4, 4, 64, 4, 100)).expect("blocked");
    let blocked_held = blocked.revision();
    let routes = yield_routes(&mut blocked, blocked_held.as_ref());
    let east_edges = blocked.route_edges(routes[0]).expect("east").to_vec();
    blocked
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[0], 1, 10_501, 0)
                .with_open_entrance(),
        )
        .expect("leader blocks east storage");
    let east_length = blocked.traffic().lane_lengths_millimetres()[east_edges[0].index()];
    blocked
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                east_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("east vehicle cannot acquire");
    let north_edge = blocked.route_edges(routes[1]).expect("north")[0];
    let north_length = blocked.traffic().lane_lengths_millimetres()[north_edge.index()];
    blocked
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                north_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("a claim that is not acquired does not block the other exit");
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn already_unstoppable_vehicle_is_not_charged_to_the_next_insert() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                deny: true,
                ..ConflictPolicyFixture::default()
            },
        ));
    let mut world = install_fixture(revision, WorldConfig::new(4, 4, 64, 4, 100)).expect("world");
    let held = world.revision();
    let routes = yield_routes(&mut world, held.as_ref());
    let edges = world.route_edges(routes[1]).expect("edges").to_vec();
    let last = u32::try_from(edges.len() - 1).expect("index");
    let last_length = world.traffic().lane_lengths_millimetres()[edges[last as usize].index()];
    let completed = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                last,
                last_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("completed later");
    world.step(TickInput::new(100)).expect("complete");
    assert_eq!(
        world.vehicle(completed).expect("completed").status(),
        VehicleStatus::Completed
    );
    let yield_edge = world.route_edges(routes[0]).expect("yield")[0];
    let yield_length = world.traffic().lane_lengths_millimetres()[yield_edge.index()];
    world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                yield_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("already cannot stop for the closed gate");
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[1], 0, 0, 0)
                .with_open_entrance(),
        )
        .expect("an unrelated spawn is not blamed for an old hard stop");
    world
        .replace_completed_vehicle(
            completed,
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[1], 0, 6_000, 0)
                .with_open_entrance(),
        )
        .expect("replace uses the same rule and does not blame the old hard stop");
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn replace_into_the_priority_pose_uses_the_same_yield_rule() {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let mut world = install_fixture(revision, WorldConfig::new(4, 4, 64, 4, 100)).expect("world");
    let held = world.revision();
    let routes = yield_routes(&mut world, held.as_ref());
    let edges = world.route_edges(routes[0]).expect("edges").to_vec();
    let last = u32::try_from(edges.len() - 1).expect("index");
    let last_length = world.traffic().lane_lengths_millimetres()[edges[last as usize].index()];
    let completed = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                last,
                last_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("completed vehicle");
    world.step(TickInput::new(100)).expect("complete");
    let yield_edge = world.route_edges(routes[0]).expect("yield")[0];
    let yield_length = world.traffic().lane_lengths_millimetres()[yield_edge.index()];
    world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                yield_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("yield vehicle");
    let priority_edge = world.route_edges(routes[1]).expect("priority")[0];
    let priority_length = world.traffic().lane_lengths_millimetres()[priority_edge.index()];
    let replaced = world.replace_completed_vehicle(
        completed,
        VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            routes[1],
            0,
            priority_length - 2_500,
            10_000,
        )
        .with_open_entrance(),
    );
    assert!(
        matches!(replaced, Err(ReplaceError::StopConstraintUnsatisfiable)),
        "replace must refuse the pose that would newly stop the yield vehicle, got {replaced:?}"
    );
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn failed_same_zone_contender_does_not_block_the_waiting_conflict_gate() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                waiting: true,
                waiting_on_north_only: true,
                conflict_after_release: true,
                equal_priority: true,
                short_vehicle: true,
                close_internal_m: 0.8,
                ..ConflictPolicyFixture::default()
            },
        ));
    let dot = conflict_profile(revision.as_ref(), "dot");
    let mut open =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 8, 1_000)).expect("open");
    let open_held = open.revision();
    let open_routes = yield_routes(&mut open, open_held.as_ref());
    let open_east = open.route_edges(open_routes[0]).expect("east")[0];
    let open_east_length = open.traffic().lane_lengths_millimetres()[open_east.index()];
    open.place_existing_active_vehicle(
        VehicleSpawnInput::new(dot, open_routes[0], 0, open_east_length - 200, 2_000)
            .with_open_entrance(),
    )
    .expect("east reaches the shared zone");
    let open_north = open.route_edges(open_routes[1]).expect("north")[0];
    let open_north_length = open.traffic().lane_lengths_millimetres()[open_north.index()];
    let open_north_vehicle = open
        .spawn_vehicle(
            VehicleSpawnInput::new(dot, open_routes[1], 0, open_north_length - 200, 2_000)
                .with_open_entrance(),
        )
        .expect("north can still stop for a zone someone else will take");
    open.step(TickInput::new(1_000)).expect("open step");
    assert!(
        open.conflict_reservation(open_north_vehicle).is_none(),
        "the earlier vehicle keeps the shared zone"
    );

    let mut blocked =
        install_fixture(revision, WorldConfig::new(4, 4, 64, 8, 1_000)).expect("blocked");
    let held = blocked.revision();
    let routes = yield_routes(&mut blocked, held.as_ref());
    let east_edges = blocked.route_edges(routes[0]).expect("east").to_vec();
    let east_length = blocked.traffic().lane_lengths_millimetres()[east_edges[0].index()];
    blocked
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(dot, routes[0], 1, 400, 0).with_open_entrance(),
        )
        .expect("leader blocks east storage");
    blocked
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(dot, routes[0], 0, east_length - 200, 2_000)
                .with_open_entrance(),
        )
        .expect("east cannot acquire");
    let north_edge = blocked.route_edges(routes[1]).expect("north")[0];
    let north_length = blocked.traffic().lane_lengths_millimetres()[north_edge.index()];
    let north = blocked
        .spawn_vehicle(
            VehicleSpawnInput::new(dot, routes[1], 0, north_length - 200, 2_000)
                .with_open_entrance(),
        )
        .expect("a contender who cannot acquire does not block the waiting approach");
    blocked.step(TickInput::new(1_000)).expect("blocked step");
    let north_state = blocked.vehicle(north).expect("north");
    assert!(
        north_state.speed_mm_s().saturating_add(4_000) >= 2_000,
        "speed dropped faster than 4 m/s² over one second"
    );
    assert!(
        blocked.conflict_reservation(north).is_some()
            || blocked.latest_conflict_decisions().iter().any(|decision| {
                decision.vehicle() == north
                    && decision.outcome() == laneflow_runtime::ConflictDecisionOutcome::Granted
            }),
        "north still gets the zone, decisions {:?}",
        blocked.latest_conflict_decisions()
    );
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn a_waiting_zone_beyond_this_tick_still_lets_the_conflict_acquire() {
    let revision = late_waiting_revision(false);
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 8, 100)).expect("world");
    let held = world.revision();
    let routes = yield_routes(&mut world, held.as_ref());
    let north_length = approach_length(&world, routes[1]);
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                north_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("a waiting entrance this tick cannot reach does not cancel the conflict");
    let east_length = approach_length(&world, routes[0]);
    assert_eq!(
        world.spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                east_length - 400,
                10_000,
            )
            .with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable),
        "the first vehicle still acquired the zone"
    );

    let mut stepped = install_fixture(revision, WorldConfig::new(4, 4, 64, 8, 100)).expect("step");
    let stepped_revision = stepped.revision();
    let stepped_routes = yield_routes(&mut stepped, stepped_revision.as_ref());
    let placed = stepped
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                stepped_routes[1],
                0,
                north_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("place");
    stepped.step(TickInput::new(100)).expect("official step");
    let state = stepped.vehicle(placed).expect("placed");
    assert_eq!(state.route_edge_index(), 1, "the conflict gate was reached");
    assert!(
        state.progress_mm() < 13_000,
        "the waiting entrance at the end of the internal is still ahead"
    );
    assert_eq!(
        stepped
            .latest_conflict_decisions()
            .iter()
            .find(|decision| decision.vehicle() == placed)
            .map(|decision| decision.outcome()),
        Some(laneflow_runtime::ConflictDecisionOutcome::Granted)
    );
    assert!(stepped.conflict_reservation(placed).is_some());
}

#[test]
fn a_full_waiting_zone_beyond_this_tick_still_lets_the_conflict_acquire() {
    let revision = late_waiting_revision(false);
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 8, 100)).expect("world");
    let held = world.revision();
    let routes = yield_routes(&mut world, held.as_ref());
    let north_length = approach_length(&world, routes[1]);
    let occupant = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                north_length - 2_000,
                4_000,
            )
            .with_open_entrance(),
        )
        .expect("occupant");
    for _ in 0..400 {
        world.step(TickInput::new(100)).expect("fill");
        let state = world.vehicle(occupant).expect("occupant stays");
        if state.waiting_membership().is_some()
            && state.route_edge_index() >= 2
            && world.conflict_reservation(occupant).is_none()
        {
            break;
        }
    }
    let parked = world.vehicle(occupant).expect("occupant");
    assert!(
        parked.waiting_membership().is_some() && parked.route_edge_index() >= 2,
        "occupant edge {} progress {}",
        parked.route_edge_index(),
        parked.progress_mm()
    );
    assert!(world.conflict_reservation(occupant).is_none());
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                north_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("a full waiting zone this tick cannot reach does not cancel the conflict");
    let east_length = approach_length(&world, routes[0]);
    assert_eq!(
        world.spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                east_length - 400,
                10_000,
            )
            .with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable)
    );
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn a_claim_past_the_waiting_entrance_does_not_keep_the_zone() {
    let revision = late_waiting_revision(true);
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 8, 100)).expect("world");
    let held = world.revision();
    let routes = yield_routes(&mut world, held.as_ref());
    let north_length = approach_length(&world, routes[1]);
    assert_eq!(
        world.spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                north_length - 400,
                10_000,
            )
            .with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable),
        "a claim the next tick will not grant is not admitted clear"
    );
    let north = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                north_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("placed contender");
    let east_length = approach_length(&world, routes[0]);
    let east = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                east_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("a contender who will not acquire does not keep the zone");
    world.step(TickInput::new(100)).expect("step");
    let north_state = world.vehicle(north).expect("north");
    assert!(world.conflict_reservation(north).is_none());
    assert_eq!(
        world
            .latest_conflict_decisions()
            .iter()
            .find(|decision| decision.vehicle() == north)
            .map(|decision| decision.outcome()),
        Some(laneflow_runtime::ConflictDecisionOutcome::NoGrant(
            laneflow_runtime::ConflictNoGrantReason::DownstreamStorageBoundary
        ))
    );
    assert_eq!(north_state.route_edge_index(), 0);
    let east_state = world.vehicle(east).expect("east");
    assert!(
        world.conflict_reservation(east).is_some()
            || world.latest_conflict_decisions().iter().any(|decision| {
                decision.vehicle() == east
                    && decision.outcome() == laneflow_runtime::ConflictDecisionOutcome::Granted
            })
    );
    let travel = if east_state.route_edge_index() == 0 {
        east_state.progress_mm().saturating_sub(east_length - 400)
    } else {
        400u32.saturating_add(east_state.progress_mm())
    };
    assert_step_stays_in_emergency_envelope(10_000, east_state.speed_mm_s(), travel);
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn refresh_allocation_failure_keeps_the_committed_vehicle() {
    let revision = compile_road_editing_revision(conflict_road_editing_module());
    let mut clean =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 2, 64, 2, 100)).expect("clean");
    let mut faulted =
        install_fixture(revision, WorldConfig::new(4, 2, 64, 2, 100)).expect("faulted");
    let clean_route = register_first_stream(&mut clean);
    let faulted_route = register_first_stream(&mut faulted);
    clean
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), clean_route, 0, 0, 0)
                .with_open_entrance(),
        )
        .expect("first");
    faulted
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), faulted_route, 0, 0, 0)
                .with_open_entrance(),
        )
        .expect("first");
    laneflow_runtime::set_admission_reserve_failure(
        laneflow_runtime::AdmissionReserve::Refresh,
        true,
    );
    faulted
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                faulted_route,
                0,
                8_000,
                0,
            )
            .with_open_entrance(),
        )
        .expect("allocation after commit still keeps the vehicle");
    laneflow_runtime::set_admission_reserve_failure(
        laneflow_runtime::AdmissionReserve::Refresh,
        false,
    );
    clean
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), clean_route, 0, 8_000, 0)
                .with_open_entrance(),
        )
        .expect("second");
    faulted.force_rebuild_contenders_for_test();
    assert_eq!(
        faulted.contender_fingerprint_for_test(),
        clean.contender_fingerprint_for_test()
    );
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn candidate_admission_does_not_grow_with_existing_vehicles() {
    let revision = compile_road_editing_revision(conflict_road_editing_module());
    let one = candidate_admission_calls_for(&revision, 1);
    let two = candidate_admission_calls_for(&revision, 2);
    assert_eq!(one, two);
}

#[cfg(feature = "placement-fixtures")]
fn candidate_admission_calls_for(revision: &Arc<SharedNetworkRevision>, existing: usize) -> u64 {
    let mut world =
        install_fixture(Arc::clone(revision), WorldConfig::new(8, 2, 64, 2, 100)).expect("world");
    let route = register_first_stream(&mut world);
    for index in 0..existing {
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    u32::try_from(index).expect("index"),
                    0,
                    0,
                )
                .with_open_entrance(),
            )
            .expect("existing");
    }
    laneflow_runtime::reset_candidate_admission_calls();
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                u32::try_from(existing).expect("index"),
                0,
                0,
            )
            .with_open_entrance(),
        )
        .expect("subject");
    laneflow_runtime::candidate_admission_calls()
}

#[cfg(feature = "placement-fixtures")]
fn register_first_stream(world: &mut TrafficWorld) -> RouteHandle {
    let revision = world.revision();
    let stream = revision
        .conflict()
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("stream");
    let edges = revision
        .traffic()
        .maneuvers()
        .maneuver_path(stream.maneuver_path())
        .expect("path")
        .edges()
        .to_vec();
    world
        .register_route(RouteRegisterInput::new(edges))
        .expect("route")
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn an_exit_only_route_still_blocks_downstream_storage() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                clearance: Some((1, 12.0)),
                ..ConflictPolicyFixture::default()
            },
        ));
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(8, 8, 64, 4, 100)).expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    let full_edges = world.route_edges(routes[0]).expect("full route").to_vec();
    let exit_edge = *full_edges.last().expect("exit edge");
    let exit_only = world
        .register_route(RouteRegisterInput::new(vec![exit_edge]))
        .expect("plain exit route");
    let entry_length = world.traffic().lane_lengths_millimetres()[full_edges[0].index()];
    let approacher = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                entry_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("downstream is free for the first vehicle");
    let cursor = world.command_cursor();
    let blocked = world.spawn_vehicle(
        VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), exit_only, 0, 5_000, 0)
            .with_open_entrance(),
    );
    assert!(
        matches!(blocked, Err(SpawnError::StopConstraintUnsatisfiable)),
        "an exit body must not let the approacher hard-stop past the envelope, got {blocked:?}"
    );
    assert_eq!(world.command_cursor(), cursor);
    assert_eq!(
        world.vehicle(approacher).expect("kept").speed_mm_s(),
        10_000
    );
    assert_eq!(
        laneflow_runtime::body_reserve_slots(),
        u64::from(4_500_u32.div_ceil(laneflow_static_contract::MIN_LANE_EDGE_LENGTH_MM))
            .saturating_add(1),
        "body scratch follows the shortest legal edge, not one slot per millimetre"
    );
    let accepted = world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), exit_only, 0, 11_000, 0)
                .with_open_entrance(),
        )
        .expect("a body past the stored downstream still fits");
    let before = world.vehicle(approacher).expect("approacher");
    let before_speed = before.speed_mm_s();
    world.step(TickInput::new(100)).expect("step");
    let after = world.vehicle(approacher).expect("approacher after");
    assert!(
        after.speed_mm_s().saturating_add(400) >= before_speed,
        "accepted exit body changed {} -> {}",
        before_speed,
        after.speed_mm_s()
    );
    assert!(world.vehicle(accepted).is_some());
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn an_exit_body_blocks_downstream_when_the_follower_cannot_stop() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                right_turn_signal: Some(laneflow_compiler::GateInterpretation::PermissiveGroup),
                signal_cycle_ms: Some([100, 10_000]),
                signal_stop_aspect: Some(SignalAspect::Red),
                ..ConflictPolicyFixture::default()
            },
        ));
    let mut world = install_fixture(revision, WorldConfig::new(4, 2, 64, 2, 100)).expect("world");
    world.step(TickInput::new(100)).expect("red");
    let held = world.revision();
    let routes = yield_routes(&mut world, held.as_ref());
    let edges = world.route_edges(routes[0]).expect("edges").to_vec();
    let entry_length = world.traffic().lane_lengths_millimetres()[edges[0].index()];
    world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                entry_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("follower is already at the gate");
    assert_eq!(
        world.spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[0], 2, 1_000, 0,)
                .with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable),
        "the exit body blocks downstream storage and the follower cannot stop"
    );
}

#[cfg(feature = "placement-fixtures")]
fn long_red_routes() -> (
    laneflow_runtime::TrafficWorld,
    [laneflow_runtime::RouteHandle; 2],
) {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                right_turn_signal: Some(laneflow_compiler::GateInterpretation::PermissiveGroup),
                signal_cycle_ms: Some([10_000, 100]),
                signal_stop_aspect: Some(SignalAspect::Red),
                conflict_after_release: true,
                ..ConflictPolicyFixture::default()
            },
        ));
    let mut world = install_fixture(revision, WorldConfig::new(8, 4, 64, 4, 100)).expect("world");
    let held = world.revision();
    let routes = yield_routes(&mut world, held.as_ref());
    (world, routes)
}

#[cfg(feature = "placement-fixtures")]
fn entry_length_mm(world: &TrafficWorld, route: laneflow_runtime::RouteHandle) -> u32 {
    let edge = world.route_edges(route).expect("route")[0];
    world.traffic().lane_lengths_millimetres()[edge.index()]
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn leader_room_tighter_than_red_rejects_and_a_looser_room_does_not() {
    let (mut world, routes) = long_red_routes();
    let entry = entry_length_mm(&world, routes[0]);
    let follower_at = entry - 150;
    world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                follower_at,
                1_000,
            )
            .with_open_entrance(),
        )
        .expect("follower 150 mm before red");
    let cursor = world.command_cursor();
    let sequence = world.observation_state_sequence();
    let tight = world.spawn_vehicle(
        VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[0], 1, 4_550, 0)
            .with_open_entrance(),
    );
    assert_eq!(
        tight,
        Err(SpawnError::UnsafeFollower {
            follower: world.live_vehicles()[0]
        }),
        "entry {entry} mm, follower at {follower_at}, leader front 4550; net gap 200 mm is tighter than the 150 mm red"
    );
    assert_eq!(world.command_cursor(), cursor);
    assert_eq!(world.observation_state_sequence(), sequence);
    assert_eq!(world.live_vehicles().len(), 1);

    let (mut loose, routes) = long_red_routes();
    loose
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                follower_at,
                1_000,
            )
            .with_open_entrance(),
        )
        .expect("follower");
    loose
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[0], 1, 6_500, 0)
                .with_open_entrance(),
        )
        .expect("leader room is no tighter than the red");
    let before = loose.vehicle(loose.live_vehicles()[0]).expect("follower");
    let speed_before = before.speed_mm_s();
    let progress_before = before.progress_mm();
    let edge_before = before.route_edge_index();
    loose.step(TickInput::new(100)).expect("formal step");
    let after = loose
        .vehicle(loose.live_vehicles()[0])
        .expect("follower after");
    let travel = if after.route_edge_index() == edge_before {
        after.progress_mm().saturating_sub(progress_before)
    } else {
        entry
            .saturating_sub(progress_before)
            .saturating_add(after.progress_mm())
    };
    assert!(
        after.speed_mm_s().saturating_add(400) >= speed_before,
        "speed {speed_before} -> {} travel {travel} mm",
        after.speed_mm_s()
    );

    let (mut crossing, routes) = long_red_routes();
    let entry = entry_length_mm(&crossing, routes[0]);
    let follower_at = entry - 700;
    crossing
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                follower_at,
                1_000,
            )
            .with_open_entrance(),
        )
        .expect("follower behind a tail that crosses the edge");
    let cursor = crossing.command_cursor();
    let sequence = crossing.observation_state_sequence();
    let tail = crossing.spawn_vehicle(
        VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[0], 1, 4_000, 0)
            .with_open_entrance(),
    );
    assert_eq!(
        tail,
        Err(SpawnError::UnsafeFollower {
            follower: crossing.live_vehicles()[0]
        }),
        "entry {entry} mm, follower at {follower_at}, leader front 4000; the 500 mm tail on the previous edge leaves a 200 mm gap"
    );
    assert_eq!(crossing.command_cursor(), cursor);
    assert_eq!(crossing.observation_state_sequence(), sequence);
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn exclusion_work_stays_flat_when_less_urgent_owners_are_added() {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let measure = |extra: usize| {
        let mut world = install_fixture(Arc::clone(&revision), WorldConfig::new(16, 4, 64, 4, 100))
            .expect("world");
        let routes = yield_routes(&mut world, revision.as_ref());
        let foe_length = entry_length_mm(&world, routes[1]);
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    routes[1],
                    0,
                    foe_length - 2_500,
                    10_000,
                )
                .with_open_entrance(),
            )
            .expect("urgent foe");
        if extra > 0 {
            world
                .spawn_vehicle(
                    VehicleSpawnInput::new(
                        VehicleProfileOrdinal::from_raw(0),
                        routes[1],
                        0,
                        800,
                        500,
                    )
                    .with_open_entrance(),
                )
                .expect("less urgent approach behind the foe");
        }
        if extra > 1 {
            world
                .spawn_vehicle(
                    VehicleSpawnInput::new(
                        VehicleProfileOrdinal::from_raw(0),
                        routes[1],
                        2,
                        1_000,
                        500,
                    )
                    .with_open_entrance(),
                )
                .expect("less urgent vehicle past the conflict");
        }
        laneflow_runtime::reset_exclusion_counts();
        let subject_length = entry_length_mm(&world, routes[0]);
        let result = world.spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                subject_length - 400,
                10_000,
            )
            .with_open_entrance(),
        );
        let (calls, work) = laneflow_runtime::exclusion_counts();
        (result, calls, work)
    };
    let (base_result, base_calls, base_work) = measure(0);
    let (extra_result, extra_calls, extra_work) = measure(2);
    assert_eq!(
        base_result, extra_result,
        "a less urgent owner must not change the decision"
    );
    assert!(base_calls > 0, "the yield preflight should read a cell");
    assert_eq!(base_work, base_calls);
    assert_eq!(extra_work, extra_calls);
    assert_eq!(
        extra_work, base_work,
        "exclusion work grew from {base_work} to {extra_work} when less urgent owners were added"
    );
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn committed_parking_commands_drop_the_contender_snapshot() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                equal_priority: true,
                early_parking_on_internal: true,
                ..ConflictPolicyFixture::default()
            },
        ));
    let mut world = install_fixture(revision, WorldConfig::new(8, 4, 64, 4, 100)).expect("world");
    let held = world.revision();
    let routes = yield_routes(&mut world, held.as_ref());
    let east_length = entry_length_mm(&world, routes[0]);
    let east = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                east_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("east");
    let sequence = world.observation_state_sequence();
    world
        .reserve_parking(
            east,
            ReserveParkingTarget::ExplicitSpace {
                space: ParkingSpaceOrdinal::from_raw(0),
                entry_route_occurrence: 1,
            },
        )
        .expect("reserve");
    assert_eq!(world.observation_state_sequence(), sequence);
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[0], 0, 0, 0)
                .with_open_entrance(),
        )
        .expect("rebuild after reserve");
    assert!(
        laneflow_runtime::contender_rebuild_scans() >= 1,
        "a committed reserve must drop the snapshot"
    );

    let sequence = world.observation_state_sequence();
    assert!(
        world
            .reserve_parking(
                east,
                ReserveParkingTarget::ExplicitSpace {
                    space: ParkingSpaceOrdinal::from_raw(0),
                    entry_route_occurrence: 1,
                },
            )
            .expect("same reserve")
            .is_no_change()
    );
    assert_eq!(world.observation_state_sequence(), sequence);
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[0], 0, 6_000, 0)
                .with_open_entrance(),
        )
        .expect("reuse after no-change reserve");
    assert_eq!(
        laneflow_runtime::contender_rebuild_scans(),
        0,
        "an identical reserve may keep the snapshot"
    );

    let sequence = world.observation_state_sequence();
    world
        .cancel_parking(
            east,
            ParkingTarget::ExplicitSpace(ParkingSpaceOrdinal::from_raw(0)),
        )
        .expect("cancel");
    assert_eq!(world.observation_state_sequence(), sequence);
    let cursor = world.command_cursor();
    let north_length = entry_length_mm(&world, routes[1]);
    let north = world.spawn_vehicle(
        VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            routes[1],
            0,
            north_length - 400,
            10_000,
        )
        .with_open_entrance(),
    );
    assert_eq!(
        north,
        Err(SpawnError::StopConstraintUnsatisfiable),
        "cancelling the reservation must not keep treating the east vehicle as absent"
    );
    assert_eq!(world.command_cursor(), cursor);
    assert!(laneflow_runtime::contender_rebuild_scans() >= 1);
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn committed_rebind_drops_the_contender_snapshot() {
    let revision = compile_rebind_revision();
    let mut world =
        install_fixture(revision, WorldConfig::new(8, 8, 1_024, 1_024, 100)).expect("install");
    let old_route = register_named(&mut world, &["left", "current", "tail"]);
    let new_route = register_named(&mut world, &["right", "current", "tail"]);
    let profile = VehicleProfileOrdinal::from_raw(0);
    let facility = ParkingFacilityOrdinal::from_raw(0);
    let contained = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, old_route, 1, 5_000, 0).with_open_entrance())
        .expect("contained");
    world
        .reserve_parking(
            contained,
            ReserveParkingTarget::VirtualPool {
                facility,
                entry_anchor: VirtualEntryAnchorSelector::from_raw(0),
                entry_route_occurrence: 2,
            },
        )
        .expect("reserve");
    world
        .spawn_vehicle(VehicleSpawnInput::new(profile, old_route, 2, 1_000, 0).with_open_entrance())
        .expect("warm the snapshot");
    let sequence = world.observation_state_sequence();
    world
        .rebind_parking_route(
            contained,
            RebindParkingTarget::VirtualPool {
                facility,
                new_route,
                new_current_route_occurrence: 1,
                new_entry_anchor: VirtualEntryAnchorSelector::from_raw(0),
                new_entry_route_occurrence: 2,
            },
        )
        .expect("rebind");
    assert_eq!(world.observation_state_sequence(), sequence);
    world
        .spawn_vehicle(VehicleSpawnInput::new(profile, new_route, 0, 0, 0).with_open_entrance())
        .expect("spawn after rebind");
    assert!(
        laneflow_runtime::contender_rebuild_scans() >= 1,
        "a committed rebind must drop the snapshot"
    );
}

#[cfg(feature = "placement-fixtures")]
fn admission_work_after_far_vehicles(existing: usize) -> (u64, u64, u64, u64) {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let mut world =
        install_fixture(revision.clone(), WorldConfig::new(32, 4, 64, 4, 100)).expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    let mut placed = 0usize;
    for edge in [2_u32, 0] {
        for progress in [0_u32, 6_000] {
            if placed >= existing {
                break;
            }
            world
                .spawn_vehicle(
                    VehicleSpawnInput::new(
                        VehicleProfileOrdinal::from_raw(0),
                        routes[1],
                        edge,
                        progress,
                        0,
                    )
                    .with_open_entrance(),
                )
                .expect("far stopped vehicle");
            placed += 1;
        }
    }
    laneflow_runtime::reset_recheck_visits();
    laneflow_runtime::reset_acquisition_replays();
    laneflow_runtime::reset_admission_scratch_reserves();
    laneflow_runtime::reset_contender_update_counts();
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[0], 0, 0, 0)
                .with_open_entrance(),
        )
        .expect("unrelated spawn");
    (
        laneflow_runtime::recheck_visits(),
        laneflow_runtime::acquisition_replays(),
        laneflow_runtime::admission_scratch_reserves(),
        laneflow_runtime::incremental_contender_visits(),
    )
}

#[cfg(feature = "placement-fixtures")]
fn straight_road_work(existing: usize) -> (u64, u64, u64, u64) {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "road",
                length_meters: 250.0,
                speed_limit_meters_per_second: 15.0,
                successors: &[],
            })
            .expect("road");
    });
    let mut world = install_fixture(revision, WorldConfig::new(40, 4, 64, 4, 100)).expect("world");
    let route = register_named(&mut world, &["road"]);
    for index in 0..existing {
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    u32::try_from(index).expect("index") * 8_000,
                    0,
                )
                .with_open_entrance(),
            )
            .expect("stopped along the road");
    }
    laneflow_runtime::reset_recheck_visits();
    laneflow_runtime::reset_acquisition_replays();
    laneflow_runtime::reset_admission_scratch_reserves();
    laneflow_runtime::reset_contender_update_counts();
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                u32::try_from(existing).expect("index") * 8_000,
                0,
            )
            .with_open_entrance(),
        )
        .expect("next stopped vehicle");
    (
        laneflow_runtime::recheck_visits(),
        laneflow_runtime::acquisition_replays(),
        laneflow_runtime::admission_scratch_reserves(),
        laneflow_runtime::incremental_contender_visits(),
    )
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn unrelated_far_vehicles_do_not_grow_recheck_work() {
    let few = admission_work_after_far_vehicles(2);
    let many = admission_work_after_far_vehicles(4);
    assert_eq!(few, many, "few {few:?} many {many:?}");
    let short = straight_road_work(4);
    let long = straight_road_work(24);
    assert_eq!(short, long, "short {short:?} long {long:?}");
}

#[cfg(feature = "placement-fixtures")]
fn yield_stop_result(full: bool) -> Result<laneflow_runtime::VehicleHandle, SpawnError> {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let mut world =
        install_fixture(revision.clone(), WorldConfig::new(8, 4, 64, 4, 100)).expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    let subject_length = entry_length_mm(&world, routes[0]);
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                subject_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("yield vehicle");
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[1], 0, 1_000, 0)
                .with_open_entrance(),
        )
        .expect("unrelated stopped vehicle");
    laneflow_runtime::set_full_recheck(full);
    let foe_length = entry_length_mm(&world, routes[1]);
    let result = world.spawn_vehicle(
        VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            routes[1],
            0,
            foe_length - 2_500,
            10_000,
        )
        .with_open_entrance(),
    );
    laneflow_runtime::set_full_recheck(false);
    result
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn closure_recheck_matches_the_full_scan_oracle() {
    let closure = yield_stop_result(false);
    let oracle = yield_stop_result(true);
    assert_eq!(closure, oracle);
    assert!(
        closure.is_err(),
        "the near yield vehicle is inside the closure"
    );

    let harmless = |full: bool| {
        let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
        let mut world =
            install_fixture(revision.clone(), WorldConfig::new(8, 4, 64, 4, 100)).expect("world");
        let routes = yield_routes(&mut world, revision.as_ref());
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[1], 0, 0, 0)
                    .with_open_entrance(),
            )
            .expect("unrelated");
        let length = entry_length_mm(&world, routes[0]);
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    routes[0],
                    0,
                    length - 400,
                    1_000,
                )
                .with_open_entrance(),
            )
            .expect("inside a later closure");
        laneflow_runtime::set_full_recheck(full);
        let result = world.spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[0], 0, 1_000, 0)
                .with_open_entrance(),
        );
        laneflow_runtime::set_full_recheck(false);
        result.is_ok()
    };
    assert_eq!(harmless(false), harmless(true));
    assert!(
        harmless(false),
        "an unrelated follower of a slow spawn still agrees"
    );
}

fn late_waiting_revision(claim_crosses: bool) -> Arc<SharedNetworkRevision> {
    compile_road_editing_revision_with_limits(
        conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                equal_priority: true,
                late_waiting: true,
                late_claim_crosses: claim_crosses,
                waiting_on_north_only: true,
                waiting_capacity: 1,
                ..ConflictPolicyFixture::default()
            },
        ),
        CompileLimits::single_network_1m_v2(),
    )
}

fn approach_length(world: &TrafficWorld, route: RouteHandle) -> u32 {
    let edge = world.route_edges(route).expect("route")[0];
    world.traffic().lane_lengths_millimetres()[edge.index()]
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn reserved_parking_keeps_an_unacquired_zone_from_blocking_the_next_vehicle() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                equal_priority: true,
                early_parking_on_internal: true,
                ..ConflictPolicyFixture::default()
            },
        ));
    let mut open =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 4, 100)).expect("open");
    let open_routes = yield_routes(&mut open, revision.as_ref());
    let open_east = open.route_edges(open_routes[0]).expect("east")[0];
    let open_east_length = open.traffic().lane_lengths_millimetres()[open_east.index()];
    open.place_existing_active_vehicle(
        VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            open_routes[0],
            0,
            open_east_length - 400,
            10_000,
        )
        .with_open_entrance(),
    )
    .expect("east can still acquire");
    let open_north = open.route_edges(open_routes[1]).expect("north")[0];
    let open_north_length = open.traffic().lane_lengths_millimetres()[open_north.index()];
    assert_eq!(
        open.spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                open_routes[1],
                0,
                open_north_length - 400,
                10_000,
            )
            .with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable),
        "an earlier vehicle who can acquire still blocks the other approach"
    );

    let mut blocked =
        install_fixture(revision, WorldConfig::new(4, 4, 64, 4, 100)).expect("blocked");
    let held = blocked.revision();
    let routes = yield_routes(&mut blocked, held.as_ref());
    let east_edge = blocked.route_edges(routes[0]).expect("east")[0];
    let east_length = blocked.traffic().lane_lengths_millimetres()[east_edge.index()];
    let east = blocked
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                east_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("east contender");
    blocked
        .reserve_parking(
            east,
            ReserveParkingTarget::ExplicitSpace {
                space: ParkingSpaceOrdinal::from_raw(0),
                entry_route_occurrence: 1,
            },
        )
        .expect("parking anchor sits inside the downstream claim");
    let north_edge = blocked.route_edges(routes[1]).expect("north")[0];
    let north_length = blocked.traffic().lane_lengths_millimetres()[north_edge.index()];
    let north = blocked
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                north_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("a contender stopped by a parking anchor does not keep the zone");
    blocked.step(TickInput::new(100)).expect("step");
    assert!(
        blocked.conflict_reservation(north).is_some()
            || blocked
                .latest_conflict_decisions()
                .iter()
                .any(|decision| decision.vehicle() == north
                    && decision.outcome() == laneflow_runtime::ConflictDecisionOutcome::Granted),
        "the next vehicle still receives the zone the parked contender did not take"
    );
}

#[cfg(feature = "placement-fixtures")]
fn spawn_completed_at_end(world: &mut TrafficWorld, route: RouteHandle) -> VehicleHandle {
    let edges = world.route_edges(route).expect("edges").to_vec();
    let last = u32::try_from(edges.len() - 1).expect("index");
    let last_length = world.traffic().lane_lengths_millimetres()[edges[last as usize].index()];
    let completed = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                last,
                last_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("spawn at the route end");
    world.step(TickInput::new(100)).expect("complete");
    assert_eq!(
        world.vehicle(completed).expect("retained").status(),
        VehicleStatus::Completed
    );
    completed
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn middle_live_rank_keeps_the_gate_against_a_later_contender() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                equal_priority: true,
                ..ConflictPolicyFixture::default()
            },
        ));
    let mut world = install_fixture(revision, WorldConfig::new(6, 6, 64, 4, 100)).expect("world");
    let held = world.revision();
    let routes = yield_routes(&mut world, held.as_ref());
    world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[1], 0, 0, 0)
                .with_open_entrance(),
        )
        .expect("bystander at the front of the live order");
    let completed = spawn_completed_at_end(&mut world, routes[0]);
    let foe_edge = world.route_edges(routes[1]).expect("foe")[0];
    let foe_length = world.traffic().lane_lengths_millimetres()[foe_edge.index()];
    world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                foe_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("later contender");
    let subject_edge = world.route_edges(routes[0]).expect("subject")[0];
    let subject_length = world.traffic().lane_lengths_millimetres()[subject_edge.index()];
    world
        .replace_completed_vehicle(
            completed,
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                subject_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("the middle live rank still sorts ahead of the later contender");
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn completed_hole_keeps_its_rank_and_a_new_spawn_does_not_reuse_it() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                equal_priority: true,
                ..ConflictPolicyFixture::default()
            },
        ));
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(6, 6, 64, 4, 100)).expect("world");
    let held = world.revision();
    let routes = yield_routes(&mut world, held.as_ref());
    let hole = spawn_completed_at_end(&mut world, routes[0]);
    let foe_edge = world.route_edges(routes[1]).expect("foe")[0];
    let foe_length = world.traffic().lane_lengths_millimetres()[foe_edge.index()];
    world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                foe_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("contender after the completed hole");
    let subject_edge = world.route_edges(routes[0]).expect("subject")[0];
    let subject_length = world.traffic().lane_lengths_millimetres()[subject_edge.index()];
    assert_eq!(
        world.spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                subject_length - 400,
                10_000,
            )
            .with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable),
        "a new spawn keeps counting the completed hole, so it sorts after the contender"
    );
    world
        .replace_completed_vehicle(
            hole,
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                subject_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("replacing the hole keeps the earlier rank");
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn replacement_does_not_outrank_a_contender_eligible_this_tick() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                equal_priority: true,
                close_internal_m: 0.5,
                short_vehicle: true,
                ..ConflictPolicyFixture::default()
            },
        ));
    let mut world = install_fixture(revision, WorldConfig::new(8, 8, 64, 4, 100)).expect("world");
    let held = world.revision();
    let routes = yield_routes(&mut world, held.as_ref());
    let edges = world.route_edges(routes[0]).expect("edges").to_vec();
    let last = u32::try_from(edges.len() - 1).expect("index");
    let last_length = world.traffic().lane_lengths_millimetres()[edges[last as usize].index()];
    let completed = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                last,
                last_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("early live slot");
    let east_edge = world.route_edges(routes[0]).expect("east")[0];
    let east_length = world.traffic().lane_lengths_millimetres()[east_edge.index()];
    let winner = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(1),
                routes[0],
                0,
                east_length.saturating_sub(50),
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("fast vehicle clears the short passage");
    let north_edge = world.route_edges(routes[1]).expect("north")[0];
    let north_length = world.traffic().lane_lengths_millimetres()[north_edge.index()];
    let loser = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(1),
                routes[1],
                0,
                north_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("stopped rival");
    world.step(TickInput::new(100)).expect("step");
    assert_eq!(
        world.vehicle(completed).expect("retained").status(),
        VehicleStatus::Completed
    );
    assert_eq!(
        world.vehicle(loser).expect("loser").status(),
        VehicleStatus::Active
    );
    assert!(world.conflict_reservation(winner).is_none());
    assert!(world.conflict_reservation(loser).is_none());
    let cursor = world.command_cursor();
    let replaced = world.replace_completed_vehicle(
        completed,
        VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(1),
            routes[0],
            0,
            east_length.saturating_sub(400),
            10_000,
        )
        .with_open_entrance(),
    );
    assert!(
        matches!(replaced, Err(ReplaceError::StopConstraintUnsatisfiable)),
        "a rival first eligible on this tick stays ahead, got {replaced:?}"
    );
    assert_eq!(world.command_cursor(), cursor);
    assert_eq!(
        world.vehicle(completed).expect("unchanged").status(),
        VehicleStatus::Completed
    );
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn equal_distance_earlier_rank_keeps_the_gate() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                equal_priority: true,
                ..ConflictPolicyFixture::default()
            },
        ));
    let mut world = install_fixture(revision, WorldConfig::new(4, 4, 64, 4, 100)).expect("world");
    let held = world.revision();
    let routes = yield_routes(&mut world, held.as_ref());
    let completed = spawn_completed_at_end(&mut world, routes[0]);
    let foe_edge = world.route_edges(routes[1]).expect("foe")[0];
    let foe_length = world.traffic().lane_lengths_millimetres()[foe_edge.index()];
    let earlier = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                foe_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("same distance on the other approach");
    let subject_edge = world.route_edges(routes[0]).expect("subject")[0];
    let subject_length = world.traffic().lane_lengths_millimetres()[subject_edge.index()];
    let record = world
        .replace_completed_vehicle(
            completed,
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                subject_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("equal distance still yields to the earlier live rank");
    world.step(TickInput::new(100)).expect("arbitrate");
    let decisions = world.latest_conflict_decisions();
    assert_eq!(
        decisions
            .iter()
            .find(|decision| decision.vehicle() == record.new)
            .map(|decision| decision.outcome()),
        Some(laneflow_runtime::ConflictDecisionOutcome::Granted)
    );
    assert_ne!(
        decisions
            .iter()
            .find(|decision| decision.vehicle() == earlier)
            .map(|decision| decision.outcome()),
        Some(laneflow_runtime::ConflictDecisionOutcome::Granted)
    );
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn reused_slot_generation_keeps_the_old_live_rank() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                equal_priority: true,
                ..ConflictPolicyFixture::default()
            },
        ));
    let mut world = install_fixture(revision, WorldConfig::new(4, 4, 64, 4, 100)).expect("world");
    let held = world.revision();
    let routes = yield_routes(&mut world, held.as_ref());
    let mut current = spawn_completed_at_end(&mut world, routes[0]);
    let edges = world.route_edges(routes[0]).expect("edges").to_vec();
    let last = u32::try_from(edges.len() - 1).expect("index");
    let last_length = world.traffic().lane_lengths_millimetres()[edges[last as usize].index()];
    for _ in 0..2 {
        let record = world
            .replace_completed_vehicle(
                current,
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    routes[0],
                    last,
                    last_length,
                    0,
                )
                .with_open_entrance(),
            )
            .expect("reuse the completed slot");
        current = record.new;
        world.step(TickInput::new(100)).expect("complete again");
        assert_eq!(
            world.vehicle(current).expect("replaced").status(),
            VehicleStatus::Completed
        );
    }
    let foe_edge = world.route_edges(routes[1]).expect("foe")[0];
    let foe_length = world.traffic().lane_lengths_millimetres()[foe_edge.index()];
    world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                foe_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("later contender");
    let subject_edge = world.route_edges(routes[0]).expect("subject")[0];
    let subject_length = world.traffic().lane_lengths_millimetres()[subject_edge.index()];
    world
        .replace_completed_vehicle(
            current,
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                subject_length - 400,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("a reused slot keeps the original live rank, not the slot generation");
}

#[test]
fn waiting_entrants_keep_approach_order_across_spawn_order() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                waiting: true,
                waiting_on_north_only: true,
                conflict_after_release: true,
                short_vehicle: true,
                waiting_capacity: 2,
                ..ConflictPolicyFixture::default()
            },
        ));
    let dot = conflict_profile(revision.as_ref(), "dot");
    let mut world = install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 4, 1_000))
        .expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    let edge = world.route_edges(routes[1]).expect("north")[0];
    let length = world.traffic().lane_lengths_millimetres()[edge.index()];
    let far = world
        .spawn_vehicle(
            VehicleSpawnInput::new(dot, routes[1], 0, length - 2_500, 2_000).with_open_entrance(),
        )
        .expect("farther entrant first");
    let near = world
        .spawn_vehicle(
            VehicleSpawnInput::new(dot, routes[1], 0, length - 500, 2_000).with_open_entrance(),
        )
        .expect("nearer entrant second");
    #[cfg(feature = "placement-fixtures")]
    {
        let rows = world.contender_fingerprint_for_test();
        let mut waiting: Vec<(u32, u32)> = rows
            .iter()
            .filter(|row| row.3 > 0)
            .map(|row| (row.3 - 1, row.2))
            .collect();
        let sorted = waiting.clone();
        waiting.sort_unstable();
        assert_eq!(
            sorted, waiting,
            "waiting rows must already be in approach order"
        );
        world.force_rebuild_contenders_for_test();
        assert_eq!(rows, world.contender_fingerprint_for_test());
    }
    world.step(TickInput::new(1_000)).expect("step");
    let far_after = world.vehicle(far).expect("far");
    let near_after = world.vehicle(near).expect("near");
    assert_eq!(far_after.status(), VehicleStatus::Active);
    assert_eq!(near_after.status(), VehicleStatus::Active);
    let near_ahead = near_after.route_edge_index() > far_after.route_edge_index()
        || (near_after.route_edge_index() == far_after.route_edge_index()
            && near_after.progress_mm() > far_after.progress_mm());
    assert!(
        near_ahead,
        "the nearer vehicle remains ahead after the step"
    );
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn replace_reserve_failure_is_occupancy_alloc_and_commits_nothing() {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let mut world = install_fixture(revision, WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    let held = world.revision();
    let routes = yield_routes(&mut world, held.as_ref());
    let completed = spawn_completed_at_end(&mut world, routes[0]);
    let cursor = world.command_cursor();
    let live = world.live_vehicles().len();
    laneflow_runtime::set_admission_reserve_failure(laneflow_runtime::AdmissionReserve::Best, true);
    let result = world.replace_completed_vehicle(
        completed,
        VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), routes[0], 0, 0, 0)
            .with_open_entrance(),
    );
    laneflow_runtime::set_admission_reserve_failure(
        laneflow_runtime::AdmissionReserve::Best,
        false,
    );
    assert_eq!(result, Err(ReplaceError::OccupancyAllocFailed));
    assert_eq!(world.command_cursor(), cursor);
    assert_eq!(world.live_vehicles().len(), live);
    assert_eq!(
        world.vehicle(completed).expect("unchanged").status(),
        VehicleStatus::Completed
    );
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn fresh_spawn_rebuilds_contenders_when_the_world_generation_changes() {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    let routes = yield_routes(&mut world, revision.as_ref());
    let foe_edge = world.route_edges(routes[1]).expect("foe route")[0];
    let foe_length = world.traffic().lane_lengths_millimetres()[foe_edge.index()];
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                foe_length - 2_500,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("foe beyond one tick");
    assert!(
        world.detach_contender_cache_generation_for_test(),
        "generation must advance while the emptied cache still names the old one"
    );
    let subject_edge = world.route_edges(routes[0]).expect("subject route")[0];
    let subject_length = world.traffic().lane_lengths_millimetres()[subject_edge.index()];
    assert_eq!(
        world.spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                subject_length - 400,
                10_000,
            )
            .with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable)
    );
}

fn shared_downstream_paths(
    revision: &SharedNetworkRevision,
) -> (
    Vec<laneflow_static_contract::LaneEdgeOrdinal>,
    Vec<laneflow_static_contract::LaneEdgeOrdinal>,
) {
    let mut paths = Vec::new();
    for raw in 0..4 {
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
            .expect("stream");
        let edges = revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .expect("path")
            .edges()
            .to_vec();
        if !paths.iter().any(|existing: &Vec<_>| existing == &edges) {
            paths.push(edges);
        }
    }
    paths.sort_by_key(|edges| std::cmp::Reverse(edges.len()));
    let long = paths.remove(0);
    let short = paths.remove(0);
    (long, short)
}

#[test]
fn fresh_spawn_stops_for_an_earlier_vehicles_staged_downstream_claim() {
    let revision = compile_road_editing_revision(shared_downstream_road_editing_module());
    let car = conflict_profile(revision.as_ref(), "car");
    let dot = conflict_profile(revision.as_ref(), "dot");
    let (long_edges, short_edges) = shared_downstream_paths(revision.as_ref());
    let mut open =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 16, 100)).expect("open");
    let open_long = open
        .register_route(RouteRegisterInput::new(long_edges.clone()))
        .expect("long route");
    let open_short = open
        .register_route(RouteRegisterInput::new(short_edges.clone()))
        .expect("short route");
    let short_length = open.traffic().lane_lengths_millimetres()[short_edges[0].index()];
    open.spawn_vehicle(VehicleSpawnInput::new(car, open_long, 0, 0, 0).with_open_entrance())
        .expect("vehicle that does not reach the first gate");
    open.spawn_vehicle(
        VehicleSpawnInput::new(dot, open_short, 0, short_length - 400, 10_000).with_open_entrance(),
    )
    .expect("a vehicle that will not enter this tick does not stage a claim");

    let mut world =
        install_fixture(revision.clone(), WorldConfig::new(4, 4, 64, 16, 100)).expect("blocked");
    let long = world
        .register_route(RouteRegisterInput::new(long_edges.clone()))
        .expect("long route");
    let short = world
        .register_route(RouteRegisterInput::new(short_edges))
        .expect("short route");
    let entry_length = world.traffic().lane_lengths_millimetres()[long_edges[0].index()];
    let short_length = world.traffic().lane_lengths_millimetres()
        [world.route_edges(short).expect("short")[0].index()];
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(car, long, 0, entry_length - 400, 10_000).with_open_entrance(),
        )
        .expect("first vehicle acquires and stages the shared exit");
    assert_eq!(
        world.spawn_vehicle(
            VehicleSpawnInput::new(dot, short, 0, short_length - 400, 10_000,).with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable)
    );
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn earlier_rank_on_another_zone_does_not_hard_stop_the_shared_exit() {
    let revision = compile_road_editing_revision(shared_downstream_road_editing_module());
    let car = conflict_profile(revision.as_ref(), "car");
    let (long_edges, short_edges) = shared_downstream_paths(revision.as_ref());
    let mut world = install_fixture(revision, WorldConfig::new(6, 6, 64, 16, 100)).expect("world");
    let long = world
        .register_route(RouteRegisterInput::new(long_edges.clone()))
        .expect("long route");
    let short = world
        .register_route(RouteRegisterInput::new(short_edges.clone()))
        .expect("short route");
    let long_route_edges = world.route_edges(long).expect("long edges").to_vec();
    let last = u32::try_from(long_route_edges.len() - 1).expect("index");
    let last_length =
        world.traffic().lane_lengths_millimetres()[long_route_edges[last as usize].index()];
    let completed = world
        .spawn_vehicle(VehicleSpawnInput::new(car, long, last, last_length, 0).with_open_entrance())
        .expect("early slot at the exit end");
    world
        .step(TickInput::new(100))
        .expect("complete the early slot");
    let short_edge = world.route_edges(short).expect("short")[0];
    let short_length = world.traffic().lane_lengths_millimetres()[short_edge.index()];
    let rival = world
        .spawn_vehicle(
            VehicleSpawnInput::new(car, short, 0, short_length - 400, 10_000).with_open_entrance(),
        )
        .expect("later rival before its own gate");
    let entry_length = world.traffic().lane_lengths_millimetres()[long_edges[0].index()];
    let cursor = world.command_cursor();
    let replaced = world.replace_completed_vehicle(
        completed,
        VehicleSpawnInput::new(car, long, 0, entry_length - 400, 10_000).with_open_entrance(),
    );
    assert!(
        matches!(replaced, Err(ReplaceError::StopConstraintUnsatisfiable)),
        "the other zone's shared exit must stay inside the emergency envelope, got {replaced:?}"
    );
    assert_eq!(world.command_cursor(), cursor);
    assert_eq!(world.vehicle(rival).expect("rival").speed_mm_s(), 10_000);
    assert_eq!(
        world.vehicle(completed).expect("unchanged").status(),
        VehicleStatus::Completed
    );
    laneflow_runtime::set_full_recheck(true);
    let oracle = world.replace_completed_vehicle(
        completed,
        VehicleSpawnInput::new(car, long, 0, entry_length - 400, 10_000).with_open_entrance(),
    );
    laneflow_runtime::set_full_recheck(false);
    assert_eq!(oracle, replaced);
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn fresh_spawn_ignores_a_same_zone_contender_blocked_by_an_earlier_claim() {
    let revision = compile_road_editing_revision_with_limits(
        shared_downstream_road_editing_module_with_cross(true),
        CompileLimits::single_network_1m_v2(),
    );
    let car = conflict_profile(revision.as_ref(), "car");
    let dot = conflict_profile(revision.as_ref(), "dot");
    let mut paths = Vec::new();
    let mut zones_for_path = Vec::new();
    for raw in 0..5_u32 {
        let Some(stream) = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
        else {
            continue;
        };
        let edges = revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .expect("path")
            .edges()
            .to_vec();
        let zone = stream
            .passages()
            .first()
            .expect("passage")
            .conflict_zone()
            .index();
        if let Some(index) = paths
            .iter()
            .position(|existing: &Vec<_>| existing == &edges)
        {
            assert_eq!(zones_for_path[index], zone);
        } else {
            paths.push(edges);
            zones_for_path.push(zone);
        }
    }
    let short_index = paths
        .iter()
        .position(|edges| edges.len() == 2)
        .expect("short path");
    let shared_exit = *paths[short_index].last().expect("short exit");
    let east_index = paths
        .iter()
        .position(|edges| edges.len() == 3 && edges.last() == Some(&shared_exit))
        .expect("east path");
    let north_index = paths
        .iter()
        .position(|edges| edges.len() == 3 && edges.last() != Some(&shared_exit))
        .expect("north path");
    assert_eq!(
        zones_for_path[short_index], zones_for_path[north_index],
        "the short exit and the north path share one conflict zone"
    );
    assert_ne!(zones_for_path[east_index], zones_for_path[short_index]);
    let short_edges = paths[short_index].clone();
    let east_edges = paths[east_index].clone();
    let north_edges = paths[north_index].clone();

    let mut blocked = install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 16, 100))
        .expect("blocked");
    let blocked_short = blocked
        .register_route(RouteRegisterInput::new(short_edges.clone()))
        .expect("short route");
    let blocked_north = blocked
        .register_route(RouteRegisterInput::new(north_edges.clone()))
        .expect("north route");
    let short_length = blocked.traffic().lane_lengths_millimetres()[short_edges[0].index()];
    let north_length = blocked.traffic().lane_lengths_millimetres()[north_edges[0].index()];
    blocked
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(dot, blocked_short, 0, short_length - 400, 10_000)
                .with_open_entrance(),
        )
        .expect("same-zone contender already near the gate");
    assert_eq!(
        blocked.spawn_vehicle(
            VehicleSpawnInput::new(dot, blocked_north, 0, north_length - 400, 10_000,)
                .with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable),
        "a same-zone contender who can still acquire blocks the later spawn"
    );

    let mut world = install_fixture(revision, WorldConfig::new(4, 4, 64, 16, 100)).expect("open");
    let east = world
        .register_route(RouteRegisterInput::new(east_edges.clone()))
        .expect("east route");
    let short = world
        .register_route(RouteRegisterInput::new(short_edges))
        .expect("short route");
    let north = world
        .register_route(RouteRegisterInput::new(north_edges))
        .expect("north route");
    let entry_length = world.traffic().lane_lengths_millimetres()[east_edges[0].index()];
    let short_length = world.traffic().lane_lengths_millimetres()
        [world.route_edges(short).expect("short")[0].index()];
    let north_length = world.traffic().lane_lengths_millimetres()
        [world.route_edges(north).expect("north")[0].index()];
    world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(car, east, 0, entry_length - 400, 10_000).with_open_entrance(),
        )
        .expect("earlier vehicle already holds the shared exit");
    world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(dot, short, 0, short_length - 400, 10_000).with_open_entrance(),
        )
        .expect("same-zone contender already near the gate");
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(dot, north, 0, north_length - 400, 10_000).with_open_entrance(),
        )
        .expect("a contender who loses to the earlier claim does not block another exit");
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn fresh_spawn_before_an_owned_conflict_cannot_stop() {
    let revision = compile_road_editing_revision(conflict_yield_road_editing_module());
    let route_edges = [0_u32, 1].map(|raw| {
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
            .expect("stream");
        revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .expect("path")
            .edges()
            .to_vec()
    });
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 2, 100)).expect("world");
    let routes = route_edges.map(|edges| {
        world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route")
    });
    let foe_edge = world.route_edges(routes[1]).expect("foe route")[0];
    let foe_length = world.traffic().lane_lengths_millimetres()[foe_edge.index()];
    let foe = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[1],
                0,
                foe_length,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("foe already at the gate");
    world.step(TickInput::new(100)).expect("foe takes the zone");
    assert!(world.conflict_reservation(foe).is_some());
    let subject_edge = world.route_edges(routes[0]).expect("subject route")[0];
    let subject_length = world.traffic().lane_lengths_millimetres()[subject_edge.index()];
    assert!(
        subject_length > 400,
        "approach must leave room before the gate"
    );
    assert_eq!(
        world.spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[0],
                0,
                subject_length - 400,
                10_000,
            )
            .with_open_entrance()
        ),
        Err(SpawnError::StopConstraintUnsatisfiable)
    );
}

#[cfg(feature = "placement-fixtures")]
#[derive(Clone, Copy, Debug, Default)]
struct CalibrationRejects {
    occupied: u32,
    lag_gap: u32,
    lead_gap: u32,
    downstream: u32,
    other: u32,
}

#[cfg(feature = "placement-fixtures")]
impl CalibrationRejects {
    fn record(&mut self, reason: laneflow_runtime::ConflictNoGrantReason) {
        match reason {
            laneflow_runtime::ConflictNoGrantReason::ConflictOccupied => self.occupied += 1,
            laneflow_runtime::ConflictNoGrantReason::LagGap => self.lag_gap += 1,
            laneflow_runtime::ConflictNoGrantReason::LeadGap => self.lead_gap += 1,
            laneflow_runtime::ConflictNoGrantReason::DownstreamStorageBoundary => {
                self.downstream += 1;
            }
            _ => self.other += 1,
        }
    }
}

#[cfg(feature = "placement-fixtures")]
fn calibration_profile(revision: &SharedNetworkRevision, key: &str) -> VehicleProfileOrdinal {
    let stable = derive_canonical_stable_id_v1(
        EntityKind::VehicleProfile,
        "city/runtime-conflict",
        key,
        &CompileLimits::p100_initial_v2(),
    )
    .expect("calibration profile identity");
    revision
        .identity()
        .ordinal(VehicleProfileId::from_untyped(stable))
        .expect("calibration profile ordinal")
}

#[cfg(feature = "placement-fixtures")]
fn calibration_world(revision: Arc<SharedNetworkRevision>) -> (TrafficWorld, [RouteHandle; 2]) {
    let mut world = install_fixture(Arc::clone(&revision), WorldConfig::new(8, 4, 64, 2, 100))
        .expect("calibration world");
    let routes = [0_u32, 1].map(|raw| {
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
            .expect("calibration stream");
        let edges = revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .expect("calibration path")
            .edges()
            .to_vec();
        world
            .register_route(RouteRegisterInput::new(edges))
            .expect("calibration route")
    });
    (world, routes)
}

#[cfg(feature = "placement-fixtures")]
fn spawn_calibration_vehicle(
    world: &mut TrafficWorld,
    profile: VehicleProfileOrdinal,
    route: RouteHandle,
    progress_mm: u32,
    speed_mm_s: u32,
) -> VehicleHandle {
    world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(profile, route, 0, progress_mm, speed_mm_s).with_open_entrance(),
        )
        .expect("calibration vehicle")
}

#[cfg(feature = "placement-fixtures")]
fn calibration_gate(world: &TrafficWorld, route: RouteHandle) -> u32 {
    let edge = world.route_edges(route).expect("calibration route edges")[0];
    world.traffic().lane_lengths_millimetres()[edge.index()]
}

#[cfg(feature = "placement-fixtures")]
fn calibration_outcome(
    world: &TrafficWorld,
    subject: VehicleHandle,
) -> laneflow_runtime::ConflictDecisionOutcome {
    world
        .latest_conflict_decisions()
        .iter()
        .find(|decision| decision.vehicle() == subject)
        .expect("Gate frontier decision")
        .outcome()
}

#[cfg(feature = "placement-fixtures")]
fn wait_for_calibration_grant(
    world: &mut TrafficWorld,
    subject: VehicleHandle,
    max_ticks: u32,
    rejects: &mut CalibrationRejects,
) -> u64 {
    for tick in 1..=max_ticks {
        world.step(TickInput::new(100)).expect("calibration tick");
        match calibration_outcome(world, subject) {
            laneflow_runtime::ConflictDecisionOutcome::Granted => return u64::from(tick) * 100,
            laneflow_runtime::ConflictDecisionOutcome::NoGrant(reason) => rejects.record(reason),
            outcome => panic!("unexpected calibration outcome: {outcome:?}"),
        }
        assert_eq!(
            world
                .vehicle(subject)
                .expect("queued subject")
                .route_edge_index(),
            0,
            "no-grant subject must remain on the Gate approach"
        );
    }
    panic!("calibration subject did not receive a grant within {max_ticks} ticks: {rejects:?}");
}

#[cfg(feature = "placement-fixtures")]
fn calibration_percentiles(mut waits_ms: Vec<u64>) -> (u64, u64) {
    waits_ms.sort_unstable();
    let p50 = waits_ms[(waits_ms.len() - 1) / 2];
    let p95 = waits_ms[((waits_ms.len() - 1) * 95) / 100];
    (p50, p95)
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn conservative_gap_profile_calibration_matrix_uses_the_formal_solver() {
    const TRIALS: u32 = 8;
    let revision = compile_road_editing_revision(conflict_calibration_road_editing_module());
    let policy = revision
        .policy()
        .policy(RightOfWayPolicySetOrdinal::from_raw(0))
        .expect("calibration policy");
    let gap = policy.gap_profiles().first().expect("calibration gap");
    assert_eq!(gap.key(), "urban-conservative");
    assert_eq!(gap.parameter_version(), "urban-conservative-v1");
    assert_eq!(gap.minimum_lead_ms(), 5_000);
    assert_eq!(gap.minimum_lag_ms(), 2_000);
    assert_eq!(gap.clearance_ms(), 500);
    let car = calibration_profile(&revision, "car");
    let long_vehicle = calibration_profile(&revision, "long-vehicle");

    assert_eq!(
        revision
            .traffic()
            .relations()
            .vehicle_profile(car)
            .unwrap()
            .length_mm(),
        4_500
    );
    assert_eq!(
        revision
            .traffic()
            .relations()
            .vehicle_profile(car)
            .unwrap()
            .min_gap_mm(),
        2_000
    );
    assert_eq!(
        revision
            .traffic()
            .relations()
            .vehicle_profile(long_vehicle)
            .unwrap()
            .length_mm(),
        12_000
    );
    assert_eq!(
        revision
            .traffic()
            .relations()
            .vehicle_profile(long_vehicle)
            .unwrap()
            .min_gap_mm(),
        3_000
    );

    let mut clear_short = Vec::new();
    let mut clear_long = Vec::new();
    let mut accepted_gap = Vec::new();
    let mut unprotected_turn = Vec::new();
    let mut unprotected_rejects = CalibrationRejects::default();
    let mut saturated = Vec::new();
    let mut saturated_rejects = CalibrationRejects::default();
    let mut downstream_rejects = CalibrationRejects::default();

    for _ in 0..TRIALS {
        for (profile, waits) in [(car, &mut clear_short), (long_vehicle, &mut clear_long)] {
            let (mut world, routes) = calibration_world(Arc::clone(&revision));
            assert_eq!(world.policy_gap_profiles()[0].required_lead_ms(), 5_600);
            assert_eq!(world.policy_gap_profiles()[0].required_lag_ms(), 2_500);
            let gate = calibration_gate(&world, routes[0]);
            let subject = spawn_calibration_vehicle(&mut world, profile, routes[0], gate, 10_000);
            waits.push(wait_for_calibration_grant(
                &mut world,
                subject,
                1,
                &mut CalibrationRejects::default(),
            ));
        }

        let (mut world, routes) = calibration_world(Arc::clone(&revision));
        let subject_gate = calibration_gate(&world, routes[0]);
        let subject = spawn_calibration_vehicle(&mut world, car, routes[0], subject_gate, 10_000);
        accepted_gap.push(wait_for_calibration_grant(
            &mut world,
            subject,
            1,
            &mut CalibrationRejects::default(),
        ));

        let (mut world, routes) = calibration_world(Arc::clone(&revision));
        let subject_gate = calibration_gate(&world, routes[0]);
        let priority_gate = calibration_gate(&world, routes[1]);
        spawn_calibration_vehicle(&mut world, car, routes[1], priority_gate, 13_000);
        let subject = spawn_calibration_vehicle(&mut world, car, routes[0], subject_gate, 10_000);
        unprotected_turn.push(wait_for_calibration_grant(
            &mut world,
            subject,
            80,
            &mut unprotected_rejects,
        ));

        let (mut world, routes) = calibration_world(Arc::clone(&revision));
        let subject_gate = calibration_gate(&world, routes[0]);
        let priority_gate = calibration_gate(&world, routes[1]);
        spawn_calibration_vehicle(&mut world, car, routes[1], priority_gate, 13_000);
        spawn_calibration_vehicle(&mut world, car, routes[1], priority_gate - 6_500, 13_000);
        spawn_calibration_vehicle(&mut world, car, routes[1], priority_gate - 13_000, 13_000);
        let subject = spawn_calibration_vehicle(&mut world, car, routes[0], subject_gate, 10_000);
        saturated.push(wait_for_calibration_grant(
            &mut world,
            subject,
            120,
            &mut saturated_rejects,
        ));

        let (mut world, routes) = calibration_world(Arc::clone(&revision));
        let subject_gate = calibration_gate(&world, routes[0]);
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(car, routes[0], 1, 10_501, 0).with_open_entrance(),
            )
            .expect("downstream blocker");
        let subject = spawn_calibration_vehicle(&mut world, car, routes[0], subject_gate, 10_000);
        world
            .step(TickInput::new(100))
            .expect("blocked calibration tick");
        match calibration_outcome(&world, subject) {
            laneflow_runtime::ConflictDecisionOutcome::NoGrant(reason) => {
                downstream_rejects.record(reason);
            }
            outcome => panic!("blocked calibration subject was not rejected: {outcome:?}"),
        }
        assert_eq!(world.vehicle(subject).unwrap().route_edge_index(), 0);
    }

    assert_eq!(downstream_rejects.downstream, TRIALS);
    assert_eq!(downstream_rejects.other, 0);
    assert!(unprotected_rejects.occupied > 0);
    assert!(unprotected_rejects.lag_gap > 0);
    assert_eq!(unprotected_rejects.other, 0);
    assert!(saturated_rejects.occupied > 0);
    assert!(saturated_rejects.lag_gap > 0);
    assert_eq!(saturated_rejects.other, 0);

    for (name, waits, vehicle_length_mm, minimum_gap_mm, stable_queue) in [
        ("short-clear", clear_short, 4_500, 2_000, "none"),
        ("long-clear", clear_long, 12_000, 3_000, "none"),
        (
            "yielding-merge-open-gap",
            accepted_gap,
            4_500,
            2_000,
            "none",
        ),
        (
            "unprotected-turn-closing-gap",
            unprotected_turn,
            4_500,
            2_000,
            "bounded-at-gate",
        ),
        (
            "saturated-mainline",
            saturated,
            4_500,
            2_000,
            "bounded-at-gate",
        ),
    ] {
        let passed = waits.len();
        let observation_ms = waits.iter().copied().max().unwrap_or(0);
        let (p50_ms, p95_ms) = calibration_percentiles(waits);
        println!(
            "conflict-gap-calibration scenario={name} demand={TRIALS}-controlled-arrivals \
             vehicle_length_mm={vehicle_length_mm} minimum_gap_mm={minimum_gap_mm} fixed_dt_ms=100 \
             observation_ms={observation_ms} passed={passed} \
             wait_p50_ms={p50_ms} wait_p95_ms={p95_ms} stable_queue={stable_queue}"
        );
    }
    println!(
        "conflict-gap-calibration scenario=unprotected-turn-closing-gap \
         rejects=occupied:{},lag-gap:{},lead-gap:{}",
        unprotected_rejects.occupied, unprotected_rejects.lag_gap, unprotected_rejects.lead_gap
    );
    println!(
        "conflict-gap-calibration scenario=saturated-mainline \
         rejects=occupied:{},lag-gap:{},lead-gap:{}",
        saturated_rejects.occupied, saturated_rejects.lag_gap, saturated_rejects.lead_gap
    );
    println!(
        "conflict-gap-calibration scenario=downstream-blockage \
         demand={TRIALS}-controlled-arrivals vehicle_length_mm=4500 minimum_gap_mm=2000 \
         fixed_dt_ms=100 observation_ms=100 passed=0 \
         wait_p50_ms=100 wait_p95_ms=100 rejects=downstream-storage:{} stable_queue=true",
        downstream_rejects.downstream
    );
}

#[test]
fn conflict_multiplicity_preserves_owner_local_and_repeated_occurrences() {
    let revision = compile_road_editing_revision(conflict_multiplicity_road_editing_module());
    let conflict = revision.conflict();
    let first_stream = conflict
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("first stream");
    let path = first_stream.maneuver_path();
    let streams = conflict
        .maneuver_path_participant_streams(path)
        .expect("path streams");
    assert_eq!(streams.len(), 2, "one path retains both streams");
    assert!(streams.iter().all(|stream| {
        conflict
            .participant_stream(*stream)
            .is_some_and(|view| view.maneuver_path() == path && view.passages().len() == 2)
    }));

    let _final_stream = streams
        .iter()
        .copied()
        .find(|stream| {
            conflict
                .participant_stream(*stream)
                .and_then(|view| view.passages().get(1))
                .is_some_and(|passage| {
                    passage.exit()
                        == ConflictPathAnchor::Interior {
                            path_edge_index: 1,
                            progress_millimetres: 11_000,
                        }
                })
        })
        .expect("earlier-entry passage owns the maximum clearance");
    let route_edges = revision
        .traffic()
        .maneuvers()
        .maneuver_path(path)
        .expect("shared maneuver path")
        .edges()
        .to_vec();

    let mut too_small = install_fixture(Arc::clone(&revision), WorldConfig::new(2, 2, 12, 3, 100))
        .expect("install multiplicity capacity fixture");
    assert_eq!(
        too_small
            .register_route(RouteRegisterInput::new(route_edges.clone()))
            .unwrap_err(),
        RouteError::ConflictOccurrenceCapacityExceeded {
            current: 0,
            added: 4,
            capacity: 3,
        },
    );

    let mut world = install_fixture(Arc::clone(&revision), WorldConfig::new(2, 2, 12, 4, 100))
        .expect("install exact multiplicity fixture");
    let route = world
        .register_route(RouteRegisterInput::new(route_edges.clone()))
        .expect("four distinct passage occurrences");
    let error = world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 2, 2_000, 0)
                .with_open_entrance(),
        )
        .unwrap_err();
    assert_eq!(
        error,
        SpawnError::ConflictAuthorityRequired,
        "rear clears the last entry but not the maximum clearance",
    );
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 2, 2_500, 0)
                .with_open_entrance(),
        )
        .expect("rear exactly at maximum clearance");

    let mut repeated_edges = route_edges.clone();
    repeated_edges.extend_from_slice(&route_edges);
    let mut repeated_too_small =
        install_fixture(Arc::clone(&revision), WorldConfig::new(0, 1, 6, 7, 100))
            .expect("install repeated capacity fixture");
    assert_eq!(
        repeated_too_small
            .register_route(RouteRegisterInput::new(repeated_edges.clone()))
            .unwrap_err(),
        RouteError::ConflictOccurrenceCapacityExceeded {
            current: 0,
            added: 8,
            capacity: 7,
        },
    );
    let mut repeated = install_fixture(revision, WorldConfig::new(0, 1, 6, 8, 100))
        .expect("install exact repeated fixture");
    let static_cell_count = repeated.conflict_passage_cell_count();
    let repeated_route = repeated
        .register_route(RouteRegisterInput::new(repeated_edges))
        .expect("two maneuver occurrences retain eight passage occurrences");
    assert_eq!(
        repeated.conflict_passage_cell_count(),
        static_cell_count,
        "dynamic repeated occurrences must not copy retained static frontier cells"
    );
    let first = repeated
        .conflict_passage_occurrence_locator(repeated_route, 0)
        .expect("first exact locator");
    let second = repeated
        .conflict_passage_occurrence_locator(repeated_route, 4)
        .expect("repeated exact locator");
    assert_eq!(first.address(), second.address());
    assert_eq!(first.stable_locator(), second.stable_locator());
    assert_eq!(
        repeated.conflict_passage_locator(first.address()),
        Some(first.stable_locator())
    );
    assert_eq!(first.conflict_occurrence_index(), 0);
    assert_eq!(second.conflict_occurrence_index(), 4);
    assert_ne!(
        first.maneuver_occurrence_index(),
        second.maneuver_occurrence_index(),
        "dynamic occurrence identity must not collapse to the static passage address"
    );
}

#[test]
fn route_gate_preserves_repeated_gate_occurrences() {
    let revision = compile_road_editing_revision(conflict_multiplicity_road_editing_module());
    let first_stream = revision
        .conflict()
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("first stream");
    let route_edges = revision
        .traffic()
        .maneuvers()
        .maneuver_path(first_stream.maneuver_path())
        .expect("shared maneuver path")
        .edges()
        .to_vec();
    let mut repeated_edges = route_edges.clone();
    repeated_edges.extend_from_slice(&route_edges);
    let mut world = install_fixture(revision, WorldConfig::new(0, 1, 6, 8, 100))
        .expect("install repeated fixture");
    let route = world
        .register_route(RouteRegisterInput::new(repeated_edges))
        .expect("doubled route registers two maneuver occurrences");
    let half = u32::try_from(route_edges.len()).expect("hop count fits u32");
    let hop_count = 2 * half;

    let located = (0..hop_count)
        .filter(|hop| world.route_gate(route, *hop).is_some())
        .count();
    assert!(located >= 2, "multiplicity fixture route must carry Gates");
    for hop in 0..half {
        let first_pass = world.route_gate(route, hop);
        let second_pass = world.route_gate(route, half + hop);
        assert_eq!(
            first_pass.is_some(),
            second_pass.is_some(),
            "重复经过的同一静态 Gate 在两个 hop 上同有同无"
        );
        if let (Some(first_pass), Some(second_pass)) = (first_pass, second_pass) {
            assert_eq!(first_pass.gate(), second_pass.gate());
            assert_eq!(first_pass.edge(), second_pass.edge());
            assert_eq!(first_pass.progress_mm(), second_pass.progress_mm());
            assert_eq!(first_pass.route(), route);
            assert_eq!(second_pass.route(), route);
            assert_eq!(first_pass.hop(), hop);
            assert_eq!(second_pass.hop(), half + hop);
            assert_ne!(
                first_pass, second_pass,
                "同一静态 Gate 的重复 occurrence 不得折叠"
            );
        }
    }
}

#[test]
fn direct_candidate_and_admitted_routes_share_conflict_capacity() {
    let revision = compile_road_editing_revision(conflict_road_editing_module());
    let stream = revision
        .conflict()
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("east-west stream");
    let route_edges = revision
        .traffic()
        .maneuvers()
        .maneuver_path(stream.maneuver_path())
        .expect("east-west path")
        .edges()
        .to_vec();
    let stable_edges = route_edges
        .iter()
        .map(|edge| {
            revision
                .identity()
                .stable_id(*edge)
                .expect("edge stable id")
                .into_untyped()
        })
        .collect::<Vec<_>>();
    let origin = *revision.canonical_origin();
    let mut world = install_fixture(revision, WorldConfig::new(0, 2, 6, 1, 100))
        .expect("install three-entry fixture");

    let direct = world
        .register_route(RouteRegisterInput::new(route_edges.clone()))
        .expect("direct route");
    world.remove_route(direct).expect("remove direct route");

    let mut observation = world
        .open_observation_export(ObservationSelection::AllLaneEdges)
        .expect("open observation");
    let batch = world
        .export_observation(&mut observation, ObservationExportMode::Full)
        .expect("full observation");
    let observation_set = bind_observation_set(&[&batch]).expect("bind observation");
    let model = CostModelKey::new(Sha256Digest::from_bytes([7; 32]), 1);
    let cost = DynamicCostSnapshotBinding::new(
        observation_set,
        model,
        world.tick_index(),
        0,
        0,
        Sha256Digest::from_bytes([9; 32]),
    )
    .expect("cost binding");
    let admission = world.open_routing_admission(model);
    let candidate = world
        .register_candidate_route(
            &admission,
            CandidateRouteInput::new(cost, stable_edges.clone()),
        )
        .expect("candidate route");
    assert_eq!(
        world
            .register_route(RouteRegisterInput::new(route_edges.clone()))
            .unwrap_err(),
        RouteError::ConflictOccurrenceCapacityExceeded {
            current: 1,
            added: 1,
            capacity: 1,
        },
    );
    world
        .remove_route(candidate)
        .expect("remove candidate route");

    let admitted = world
        .register_admitted_route(AdmittedRouteRegisterInput::new(
            origin.network_revision(),
            origin
                .static_contract_versions()
                .network_revision_derivation_version(),
            stable_edges,
        ))
        .expect("admitted replay route");
    assert_eq!(
        world
            .register_route(RouteRegisterInput::new(route_edges))
            .unwrap_err(),
        RouteError::ConflictOccurrenceCapacityExceeded {
            current: 1,
            added: 1,
            capacity: 1,
        },
    );
    world.remove_route(admitted).expect("remove admitted route");
}

fn sample_conflict_route_registration(
    revision: Arc<SharedNetworkRevision>,
    route_edges: &[laneflow_static_contract::LaneEdgeOrdinal],
    route_count: u32,
) -> Duration {
    let edge_occurrences = u64::from(route_count)
        .checked_mul(u64::try_from(route_edges.len()).expect("edge count fits u64"))
        .expect("scale fixture edge count");
    let mut world = install_fixture(
        revision,
        WorldConfig::new(
            1,
            route_count,
            edge_occurrences,
            u64::from(route_count),
            100,
        ),
    )
    .expect("install scale world");
    let started = Instant::now();
    for _ in 0..route_count {
        world
            .register_route(RouteRegisterInput::new(route_edges.to_vec()))
            .expect("register one-conflict route");
    }
    let elapsed = started.elapsed();
    assert_eq!(
        u32::try_from(world.live_routes().count()).expect("live route count fits u32"),
        route_count,
    );
    black_box(world);
    elapsed
}

#[test]
#[ignore = "manual release wall-clock evidence; CI 不把共享 runner 当产品基线"]
fn conflict_route_registration_10k_100k_wall_clock_evidence() {
    let revision = compile_road_editing_revision(conflict_road_editing_module());
    let stream = revision
        .conflict()
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("east-west stream");
    let route_edges = revision
        .traffic()
        .maneuvers()
        .maneuver_path(stream.maneuver_path())
        .expect("east-west path")
        .edges()
        .to_vec();
    black_box(sample_conflict_route_registration(
        Arc::clone(&revision),
        &route_edges,
        100,
    ));
    let product = sample_conflict_route_registration(Arc::clone(&revision), &route_edges, 10_000);
    let scaling = sample_conflict_route_registration(revision, &route_edges, 100_000);
    let generous_near_linear_budget = product.as_nanos().saturating_mul(20).max(50_000_000);
    assert!(
        scaling.as_nanos() <= generous_near_linear_budget,
        "10x occurrence load should stay within a realistic 20x wall-clock envelope: product={product:?} scaling={scaling:?}",
    );
    println!(
        "conflict-route-scale-evidence profile=release occurrences=10000/100000 product_us={} scaling_us={} ratio={:.3}",
        product.as_micros(),
        scaling.as_micros(),
        scaling.as_secs_f64() / product.as_secs_f64(),
    );
}

#[test]
fn conflict_cutover_recompiles_same_and_rejects_target_extension_atomically() {
    let (base_revision, target_revision, semantic_diff, semantic_diff_binding) =
        compile_conflict_cutover_pair(
            conflict_road_editing_module(),
            terminal_conflict_road_editing_module(),
        );
    let stream = base_revision
        .conflict()
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("east-west stream");
    let route_edges = base_revision
        .traffic()
        .maneuvers()
        .maneuver_path(stream.maneuver_path())
        .expect("east-west path")
        .edges()
        .to_vec();
    let config = WorldConfig::new(4, 4, 64, 1, 100);
    let mut world = install_fixture(Arc::clone(&base_revision), config).expect("install base");
    let route = world
        .register_route(RouteRegisterInput::new(route_edges.clone()))
        .expect("register base conflict route");
    let vehicle = world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 1, 10_501, 0)
                .with_open_entrance(),
        )
        .expect("base clearance is exactly satisfied");

    // 同修订换根仍重编译路线和保护 Active；可由公开 API 产生的安全态应保持恒等。
    let base_origin = *base_revision.canonical_origin();
    let same_descriptor = NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(base_origin),
        LfcaOriginBinding::from_canonical_origin(base_origin),
        None,
        MigrationPolicyKind::SameRevisionRestore,
        world.world_binding(),
    );
    let before_same = world.vehicle(vehicle);
    let _commit = world
        .cutover_same_revision(
            Arc::clone(&base_revision),
            published_source(&base_revision, "fixture://conflict-same-target"),
            &same_descriptor,
            &CutoverPreflightLimits::new(1_048_576),
        )
        .expect("same-revision conflict cutover");
    assert_eq!(world.vehicle(vehicle), before_same);
    assert_eq!(
        world
            .register_route(RouteRegisterInput::new(route_edges))
            .unwrap_err(),
        RouteError::ConflictOccurrenceCapacityExceeded {
            current: 1,
            added: 1,
            capacity: 1,
        }
    );

    // 跨修订把同一 passage clearance 延长到路线终点；旧 Active 立即变为不安全。
    // Prepare 必须整体失败并解除日志武装，不能替换 world 或推进任何游标。
    let target_origin = *target_revision.canonical_origin();
    let cross_descriptor = NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*world.revision().canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(target_origin),
        Some(semantic_diff_binding),
        MigrationPolicyKind::CrossRevisionDirect,
        world.world_binding(),
    );
    let before_generation = world.world_generation();
    let before_binding = world.world_binding();
    let before_source_revision = world.committed_source().network_revision();
    let before_vehicle = world.vehicle(vehicle);
    let error = match world.prepare_cross_revision_cutover(
        Arc::clone(&target_revision),
        published_source(&target_revision, "fixture://conflict-cross-target"),
        &cross_descriptor,
        &semantic_diff,
        &CutoverPreflightLimits::new(1_048_576),
        &CutoverTransactionLimits::default(),
    ) {
        Ok(_) => panic!("extended conflict clearance must reject cutover"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        CutoverError::VehicleRevalidationFailed { vehicle: 0 },
    );
    assert_eq!(world.world_generation(), before_generation);
    assert_eq!(world.world_binding(), before_binding);
    assert_eq!(
        world.committed_source().network_revision(),
        before_source_revision
    );
    assert_eq!(world.vehicle(vehicle), before_vehicle);

    // 再次 Prepare 仍得到领域失败而非 InFlightTransaction，证明失败路径已解除日志武装。
    let retry_source = published_source(&target_revision, "fixture://conflict-cross-retry");
    let retry = match world.prepare_cross_revision_cutover(
        target_revision,
        retry_source,
        &cross_descriptor,
        &semantic_diff,
        &CutoverPreflightLimits::new(1_048_576),
        &CutoverTransactionLimits::default(),
    ) {
        Ok(_) => panic!("extended conflict clearance retry must reject cutover"),
        Err(error) => error,
    };
    assert_eq!(
        retry,
        CutoverError::VehicleRevalidationFailed { vehicle: 0 },
    );
}

#[test]
fn cutover_rebuilds_exact_conflict_count_for_decrease_and_increase() {
    let (conflict_revision, plain_revision, remove_diff, remove_binding) =
        compile_conflict_cutover_pair(
            conflict_road_editing_module(),
            non_conflict_road_editing_module(),
        );
    let stream = conflict_revision
        .conflict()
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("east-west stream");
    let route_edges = conflict_revision
        .traffic()
        .maneuvers()
        .maneuver_path(stream.maneuver_path())
        .expect("east-west path")
        .edges()
        .to_vec();
    let mut world = install_fixture(
        Arc::clone(&conflict_revision),
        WorldConfig::new(4, 4, 64, 1, 100),
    )
    .expect("install conflict base");
    let route = world
        .register_route(RouteRegisterInput::new(route_edges.clone()))
        .expect("register conflict route");
    let source_passage = world
        .conflict_passage_occurrence_locator(route, 0)
        .expect("source conflict passage")
        .address();
    let vehicle = world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 1, 10_501, 0)
                .with_open_entrance(),
        )
        .expect("safe conflict vehicle");

    let remove_descriptor = NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*conflict_revision.canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(*plain_revision.canonical_origin()),
        Some(remove_binding),
        MigrationPolicyKind::CrossRevisionDirect,
        world.world_binding(),
    );
    let transaction = world
        .prepare_cross_revision_cutover(
            Arc::clone(&plain_revision),
            published_source(&plain_revision, "fixture://conflict-count-decrease"),
            &remove_descriptor,
            &remove_diff,
            &CutoverPreflightLimits::new(1_048_576),
            &CutoverTransactionLimits::default(),
        )
        .expect("prepare conflict-count decrease");
    let _commit = transaction
        .commit(&mut world)
        .expect("commit conflict-count decrease");
    assert_eq!(
        world.vehicle(vehicle).expect("retained vehicle").route(),
        route
    );
    assert!(
        world
            .revision()
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(0))
            .is_none()
    );
    assert_eq!(world.conflict_passage_cell_count(), 0);
    assert_eq!(
        world.conflict_passage_locator(source_passage),
        None,
        "a source-world address must not resolve after target promotion"
    );

    // 归零后的计数允许再注册一条无冲突路线；随后 target 为两条路线各重建一项。
    world
        .register_route(RouteRegisterInput::new(route_edges))
        .expect("zero conflict count permits a second route");
    assert_eq!(world.live_routes().count(), 2);

    let (plain_base, conflict_target, add_diff, add_binding) = compile_conflict_cutover_pair(
        non_conflict_road_editing_module(),
        conflict_road_editing_module(),
    );
    assert_eq!(
        plain_base.canonical_origin(),
        world.revision().canonical_origin(),
        "reverse pair must bind the exact current plain LFCA",
    );
    let add_descriptor = NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*plain_base.canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(*conflict_target.canonical_origin()),
        Some(add_binding),
        MigrationPolicyKind::CrossRevisionDirect,
        world.world_binding(),
    );
    let before_binding = world.world_binding();
    let before_source = world.committed_source().network_revision();
    let before_vehicle = world.vehicle(vehicle);
    let error = match world.prepare_cross_revision_cutover(
        Arc::clone(&conflict_target),
        published_source(&conflict_target, "fixture://conflict-count-increase"),
        &add_descriptor,
        &add_diff,
        &CutoverPreflightLimits::new(1_048_576),
        &CutoverTransactionLimits::default(),
    ) {
        Ok(_) => panic!("two rebuilt conflicts must exceed capacity one"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        CutoverError::ConflictOccurrenceCapacityExceeded {
            total: 2,
            capacity: 1,
        }
    );
    assert_eq!(world.world_binding(), before_binding);
    assert_eq!(world.committed_source().network_revision(), before_source);
    assert_eq!(world.vehicle(vehicle), before_vehicle);
    assert_eq!(world.live_routes().count(), 2);
}

#[test]
fn cutover_conflict_floor_uses_final_commit_time_and_survives_continuous_recutover() {
    let (plain_revision, conflict_revision, add_diff, add_binding) = compile_conflict_cutover_pair(
        non_conflict_road_editing_module(),
        conflict_road_editing_module(),
    );
    let mut world = install_fixture(
        Arc::clone(&plain_revision),
        WorldConfig::new(4, 4, 64, 4, 100),
    )
    .expect("install plain base");
    let descriptor = NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*plain_revision.canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(*conflict_revision.canonical_origin()),
        Some(add_binding),
        MigrationPolicyKind::CrossRevisionDirect,
        world.world_binding(),
    );
    let prepare_time = world.time_ms();
    let mut transaction = world
        .prepare_cross_revision_cutover(
            Arc::clone(&conflict_revision),
            published_source(&conflict_revision, "fixture://conflict-floor-add"),
            &descriptor,
            &add_diff,
            &CutoverPreflightLimits::new(1_048_576),
            &CutoverTransactionLimits::default(),
        )
        .expect("prepare conflict addition");
    world
        .step(TickInput::new(100))
        .expect("source continues after Prepare");
    assert!(transaction.pump(&mut world).expect("catch up").caught_up);
    world
        .step(TickInput::new(100))
        .expect("source advances again before commit");
    let final_commit_time = world.time_ms();
    assert!(final_commit_time > prepare_time);
    let commit = transaction
        .commit(&mut world)
        .expect("commit conflict addition");
    assert_eq!(commit.events.as_slice().len(), 1);

    let first_snapshot = encode_lfrs(&world.capture_snapshot().expect("capture first cutover"));
    let first_root = snapshot_wire::size_prefixed_root_as_runtime_snapshot(&first_snapshot)
        .expect("verified first snapshot");
    assert_eq!(
        first_root.conflict_lag_states().len(),
        world.conflict_passage_cell_count()
    );
    for row in first_root.conflict_lag_states() {
        assert_eq!(
            row.reference_kind(),
            snapshot_wire::ConflictLagReferenceKind::CutoverFloor
        );
        assert_eq!(row.reference_time_ms(), final_commit_time);
    }

    // 只改变不相关的车型期望速度；passage 的稳定身份、路径和锚点连续。
    // 第二次跨修订必须继承第一次的 floor，不能从新的提交时刻重新起算。
    let (continuous_base, continuous_target, continuous_diff, continuous_binding) =
        compile_conflict_cutover_pair(
            conflict_road_editing_module(),
            conflict_road_editing_module_with_vehicle_speed(12.5),
        );
    assert_eq!(
        continuous_base.canonical_origin(),
        world.revision().canonical_origin()
    );
    let continuous_descriptor = NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*continuous_base.canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(*continuous_target.canonical_origin()),
        Some(continuous_binding),
        MigrationPolicyKind::CrossRevisionDirect,
        world.world_binding(),
    );
    let transaction = world
        .prepare_cross_revision_cutover(
            Arc::clone(&continuous_target),
            published_source(&continuous_target, "fixture://conflict-floor-continuous"),
            &continuous_descriptor,
            &continuous_diff,
            &CutoverPreflightLimits::new(1_048_576),
            &CutoverTransactionLimits::default(),
        )
        .expect("prepare continuous conflict cutover");
    world
        .step(TickInput::new(100))
        .expect("advance before continuous commit");
    assert!(world.time_ms() > final_commit_time);
    let _commit = transaction
        .commit(&mut world)
        .expect("commit continuous conflict cutover");
    let second_snapshot = encode_lfrs(&world.capture_snapshot().expect("capture recutover"));
    let second_root = snapshot_wire::size_prefixed_root_as_runtime_snapshot(&second_snapshot)
        .expect("verified second snapshot");
    for row in second_root.conflict_lag_states() {
        assert_eq!(
            row.reference_kind(),
            snapshot_wire::ConflictLagReferenceKind::CutoverFloor
        );
        assert_eq!(row.reference_time_ms(), final_commit_time);
    }
}

#[test]
fn cutover_journal_replays_exact_conflict_count_through_slot_reuse() {
    let (base_revision, target_revision, semantic_diff, semantic_diff_binding) =
        compile_conflict_cutover_pair(
            conflict_road_editing_module(),
            terminal_conflict_road_editing_module(),
        );
    let stream = base_revision
        .conflict()
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("east-west stream");
    let route_edges = base_revision
        .traffic()
        .maneuvers()
        .maneuver_path(stream.maneuver_path())
        .expect("east-west path")
        .edges()
        .to_vec();
    let mut world = install_fixture(
        Arc::clone(&base_revision),
        WorldConfig::new(4, 4, 64, 2, 100),
    )
    .expect("install base");
    let initial = world
        .register_route(RouteRegisterInput::new(route_edges.clone()))
        .expect("register initial route");
    let descriptor = NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*base_revision.canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(*target_revision.canonical_origin()),
        Some(semantic_diff_binding),
        MigrationPolicyKind::CrossRevisionDirect,
        world.world_binding(),
    );
    let mut transaction = world
        .prepare_cross_revision_cutover(
            Arc::clone(&target_revision),
            published_source(&target_revision, "fixture://conflict-journal-target"),
            &descriptor,
            &semantic_diff,
            &CutoverPreflightLimits::new(1_048_576),
            &CutoverTransactionLimits::default(),
        )
        .expect("prepare conflict cutover");

    let window_route = world
        .register_route(RouteRegisterInput::new(route_edges.clone()))
        .expect("window registration");
    assert!(
        transaction
            .pump(&mut world)
            .expect("pump register")
            .caught_up
    );
    world.remove_route(initial).expect("window remove");
    assert!(transaction.pump(&mut world).expect("pump remove").caught_up);
    let replacement = world
        .register_route(RouteRegisterInput::new(route_edges.clone()))
        .expect("reuse released slot and conflict charge");
    assert_ne!(
        replacement, initial,
        "slot generation must advance on reuse"
    );
    assert!(transaction.pump(&mut world).expect("pump reuse").caught_up);
    let _commit = transaction
        .commit(&mut world)
        .expect("commit replayed conflict counts");
    assert_eq!(world.live_routes().count(), 2);
    assert_eq!(
        world
            .register_route(RouteRegisterInput::new(route_edges.clone()))
            .unwrap_err(),
        RouteError::ConflictOccurrenceCapacityExceeded {
            current: 2,
            added: 1,
            capacity: 2,
        }
    );
    world
        .remove_route(window_route)
        .expect("committed window route releases exact charge");
    world
        .register_route(RouteRegisterInput::new(route_edges))
        .expect("released journaled charge is reusable after promotion");
}

#[test]
fn conflict_snapshot_restore_uses_saved_carry_and_exact_rebuilt_count() {
    let revision = compile_road_editing_revision(conflict_road_editing_module());
    let stream = revision
        .conflict()
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("east-west stream");
    let route_edges = revision
        .traffic()
        .maneuvers()
        .maneuver_path(stream.maneuver_path())
        .expect("east-west path")
        .edges()
        .to_vec();
    let config = WorldConfig::new(4, 4, 64, 1, 100);
    let mut world = install_fixture(Arc::clone(&revision), config).expect("install");
    let route = world
        .register_route(RouteRegisterInput::new(route_edges))
        .expect("register conflict route");
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 1, 10_501, 1)
                .with_open_entrance(),
        )
        .expect("safe active vehicle");
    world
        .step(TickInput::new(100))
        .expect("materialize non-zero carry in captured state");
    let snapshot = world.capture_snapshot().expect("capture");
    assert_ne!(snapshot.vehicles()[0].carry_um(), 0);
    let source = world.committed_source().clone();
    let mut bytes = encode_lfrs(&snapshot);
    let (progress_offset, carry_offset, conflict_capacity_offset) = {
        let root = snapshot_wire::size_prefixed_root_as_runtime_snapshot(&bytes)
            .expect("verified snapshot");
        let vehicle = root.vehicles().get(0);
        (
            wire_table_field_offset(vehicle._tab, snapshot_wire::SnapshotVehicle::VT_PROGRESS_MM),
            wire_table_field_offset(vehicle._tab, snapshot_wire::SnapshotVehicle::VT_CARRY_UM),
            wire_table_field_offset(
                root.world_config()._tab,
                snapshot_wire::WorldConfigBinding::VT_ROUTE_CONFLICT_OCCURRENCE_CAPACITY,
            ),
        )
    };

    // 车长 4,500 mm，passage clearance 位于 internal edge 6,001 mm：
    // front=10,500 mm + 999 um 时，车尾只差 1 um，restore 必须在发布前拒绝。
    bytes[progress_offset..progress_offset + 4].copy_from_slice(&10_500_u32.to_le_bytes());
    bytes[carry_offset..carry_offset + 2].copy_from_slice(&999_u16.to_le_bytes());
    assert!(matches!(
        restore_lfrs(
            &bytes,
            Arc::clone(&revision),
            source.clone(),
            config,
            laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
            SnapshotRestoreLimits::new(1_048_576, 1_024),
        ),
        Err(SnapshotRestoreError::Vehicle {
            error: SpawnError::ConflictAuthorityRequired,
            ..
        })
    ));

    // 相等边界允许；这同时证明 carry 在 Active 提交前参与校验，而不是事后覆写。
    bytes[progress_offset..progress_offset + 4].copy_from_slice(&10_501_u32.to_le_bytes());
    bytes[carry_offset..carry_offset + 2].copy_from_slice(&0_u16.to_le_bytes());
    restore_lfrs(
        &bytes,
        Arc::clone(&revision),
        source.clone(),
        config,
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        SnapshotRestoreLimits::new(1_048_576, 1_024),
    )
    .expect("rear exactly at clearance restores");

    let smaller_target = WorldConfig::new(4, 4, 64, 0, 100);
    assert_eq!(
        restore_lfrs(
            &bytes,
            Arc::clone(&revision),
            source.clone(),
            smaller_target,
            laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
            SnapshotRestoreLimits::new(1_048_576, 1_024),
        )
        .unwrap_err(),
        SnapshotRestoreError::TargetCapacitySmaller {
            dimension: SnapshotLimitDimension::RouteConflictOccurrences,
            snapshot: 1,
            target: 0,
        }
    );

    // 伪造保存容量 0；目标仍可容纳。restore 必须先完整重编译所有路线，再以 actual=1
    // 报告快照自身损坏，不能在第一条路线处只给部分计数或普通 RouteError。
    bytes[conflict_capacity_offset..conflict_capacity_offset + 8]
        .copy_from_slice(&0_u64.to_le_bytes());
    assert_eq!(
        restore_lfrs(
            &bytes,
            revision,
            source,
            config,
            laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
            SnapshotRestoreLimits::new(1_048_576, 1_024),
        )
        .unwrap_err(),
        SnapshotRestoreError::LimitExceeded {
            dimension: SnapshotLimitDimension::RouteConflictOccurrences,
            limit: 0,
            actual: 1,
        }
    );
}

#[test]
fn completed_restore_never_passes_through_transient_active_three_a() {
    let revision = compile_road_editing_revision(terminal_conflict_road_editing_module());
    let stream = revision
        .conflict()
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("east-west stream");
    let route_edges = revision
        .traffic()
        .maneuvers()
        .maneuver_path(stream.maneuver_path())
        .expect("east-west path")
        .edges()
        .to_vec();
    let config = WorldConfig::new(4, 4, 64, 1, 100);
    let mut world = install_fixture(Arc::clone(&revision), config).expect("install");
    let route = world
        .register_route(RouteRegisterInput::new(route_edges.clone()))
        .expect("register terminal-conflict route");
    let terminal_index = u32::try_from(route_edges.len() - 1).expect("route index");
    let terminal_length = revision.traffic().lane_lengths_millimetres()
        [route_edges.last().expect("terminal edge").index()];
    world
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                terminal_index,
                terminal_length,
            ),
            ParkingTarget::ExplicitSpace(ParkingSpaceOrdinal::from_raw(0)),
        )
        .expect("parked state is allowed before #284");
    let snapshot = world.capture_snapshot().expect("capture");
    let snapshot_vehicle_id = snapshot.vehicles()[0].snapshot_vehicle_id();
    let mut bytes = encode_lfrs(&snapshot);
    let (vehicle_table, status_offset) = {
        let root = snapshot_wire::size_prefixed_root_as_runtime_snapshot(&bytes)
            .expect("verified snapshot");
        let vehicle = root.vehicles().get(0);
        (
            vehicle._tab.loc(),
            wire_table_field_offset(vehicle._tab, snapshot_wire::SnapshotVehicle::VT_STATUS),
        )
    };
    bytes[status_offset] = snapshot_wire::VehicleStatusKind::Completed.0;
    wire_clear_table_field(
        &mut bytes,
        vehicle_table,
        snapshot_wire::SnapshotVehicle::VT_PARKING,
    );

    let restored = restore_lfrs(
        &bytes,
        revision,
        world.committed_source().clone(),
        config,
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        SnapshotRestoreLimits::new(1_048_576, 1_024),
    )
    .expect("Completed is restored directly without transient Active");
    let vehicle = restored
        .vehicle_handle(snapshot_vehicle_id)
        .expect("restored vehicle mapping");
    let state = restored.world().vehicle(vehicle).expect("restored state");
    assert_eq!(state.status(), VehicleStatus::Completed);
    assert_eq!(state.route_edge_index(), terminal_index);
    assert_eq!(state.progress_mm(), terminal_length);
}

#[test]
fn conflict_three_a_covers_replace_leave_and_rebind_atomically() {
    let revision = compile_road_editing_revision(conflict_road_editing_module());
    let stream = revision
        .conflict()
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("east-west stream");
    let route_edges = revision
        .traffic()
        .maneuvers()
        .maneuver_path(stream.maneuver_path())
        .expect("east-west path")
        .edges()
        .to_vec();
    let mut leave_world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 1, 100))
            .expect("install leave world");
    let leave_route = leave_world
        .register_route(RouteRegisterInput::new(route_edges))
        .expect("register leave route");
    let parked = leave_world
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), leave_route, 0, 0),
            ParkingTarget::ExplicitSpace(ParkingSpaceOrdinal::from_raw(0)),
        )
        .expect("parked spawn remains allowed")
        .vehicle;
    let parked_state = leave_world.vehicle(parked);
    let parked_binding = leave_world.parking_binding(parked);
    let leave_cursor = leave_world.command_cursor();
    assert!(matches!(
        leave_world.leave_parking(
            parked,
            LeaveParkingTarget::ExplicitSpace {
                space: ParkingSpaceOrdinal::from_raw(0),
                route: leave_route,
                exit_route_occurrence: 1,
            },
        ),
        Err(ParkingError::ConflictAuthorityRequired)
    ));
    assert_eq!(leave_world.vehicle(parked), parked_state);
    assert_eq!(leave_world.parking_binding(parked), parked_binding);
    assert_eq!(leave_world.command_cursor(), leave_cursor);

    let terminal_revision = compile_road_editing_revision(terminal_conflict_road_editing_module());
    let terminal_stream = terminal_revision
        .conflict()
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("terminal stream");
    let terminal_route_edges = terminal_revision
        .traffic()
        .maneuvers()
        .maneuver_path(terminal_stream.maneuver_path())
        .expect("terminal path")
        .edges()
        .to_vec();
    let exit_edge = *terminal_route_edges.last().expect("exit edge");
    let exit_length = terminal_revision.traffic().lane_lengths_millimetres()[exit_edge.index()];
    let mut world = install_fixture(terminal_revision, WorldConfig::new(4, 4, 64, 1, 100))
        .expect("install terminal world");
    let old_route = world
        .register_route(RouteRegisterInput::new(vec![exit_edge]))
        .expect("register non-conflict suffix");
    let new_route = world
        .register_route(RouteRegisterInput::new(terminal_route_edges))
        .expect("register terminal-conflict route");
    let active = world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), old_route, 0, 12_000, 0)
                .with_open_entrance(),
        )
        .expect("spawn on non-conflict suffix");
    world
        .reserve_parking(
            active,
            ReserveParkingTarget::ExplicitSpace {
                space: ParkingSpaceOrdinal::from_raw(0),
                entry_route_occurrence: 0,
            },
        )
        .expect("reserve at exact entry");
    let active_state = world.vehicle(active);
    let active_binding = world.parking_binding(active);
    let rebind_cursor = world.command_cursor();
    assert!(matches!(
        world.rebind_parking_route(
            active,
            RebindParkingTarget::ExplicitSpace {
                space: ParkingSpaceOrdinal::from_raw(0),
                new_route,
                new_current_route_occurrence: 2,
                new_entry_route_occurrence: 2,
            },
        ),
        Err(ParkingError::ConflictAuthorityRequired)
    ));
    assert_eq!(world.vehicle(active), active_state);
    assert_eq!(world.parking_binding(active), active_binding);
    assert_eq!(world.command_cursor(), rebind_cursor);

    world.despawn_vehicle(active).expect("release reservation");
    let completed = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                old_route,
                0,
                exit_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("spawn at suffix terminal");
    world.step(TickInput::new(100)).expect("complete vehicle");
    assert_eq!(
        world.vehicle(completed).expect("completed state").status(),
        VehicleStatus::Completed
    );
    let replace_cursor = world.command_cursor();
    assert!(matches!(
        world.replace_completed_vehicle(
            completed,
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                new_route,
                2,
                exit_length,
                0,
            )
            .with_open_entrance(),
        ),
        Err(ReplaceError::ConflictAuthorityRequired)
    ));
    assert_eq!(
        world
            .vehicle(completed)
            .expect("old handle remains")
            .status(),
        VehicleStatus::Completed
    );
    assert_eq!(world.command_cursor(), replace_cursor);
}

#[test]
fn road_editing_conflict_zone_requires_two_distinct_streams() {
    let limits = CompileLimits::p100_initial_v2();
    let source = lfre::RoadEditingSourceWriter::new(&limits)
        .write(conflict_road_editing_module_with_stream_count(1))
        .expect("Road Editing source");
    let input =
        lfre::RoadEditingModuleInput::try_new("runtime-conflict.lfre", source.as_bytes(), None)
            .expect("Road Editing module input");
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_road_editing_module(input)
        .expect("Road Editing admission");

    let error = match Compiler::new().compile(unit.build().expect("compilation unit")) {
        Ok(_) => panic!("one stream cannot close a ConflictZone"),
        Err(error) => error,
    };
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::InvalidRoadEditingSource)
    );
}

#[test]
fn spawn_access_denied_on_registered_route_leaves_no_vehicle() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "stem",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[LaneEdgeReference::local("tail")],
            })
            .expect("stem")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "tail",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .expect("tail")
            .add_access_rule(AccessRuleInput {
                access_rule_key: "deny-on-tail",
                target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("tail")),
                effect: AccessEffect::Deny,
                participant_classes: &[ParticipantClassReference::local("road-user")],
                regulation: None,
                priority: 0,
            })
            .expect("deny rule");
    });
    let mut world =
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100)).expect("install");
    let route = register_named(&mut world, &["stem", "tail"]);
    assert_eq!(
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0, 0,)
                    .with_open_entrance()
            )
            .unwrap_err(),
        SpawnError::AccessDenied
    );
    assert!(
        world
            .committed_pose_sources()
            .collect::<Vec<_>>()
            .as_slice()
            .is_empty()
    );
}

#[test]
fn park_other_target_fails_when_already_parked() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge",
                length_meters: 20.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .expect("edge")
            .add_parking_space(ParkingSpaceInput {
                parking_space_key: "space-a",
                parking_facility: None,
                entry: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("edge"),
                    progress_meters: 4.0,
                },
                exit: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("edge"),
                    progress_meters: 5.0,
                },
                geometry: ParkingSpaceGeometryInput {
                    lateral_offset_meters: -3.0,
                    heading_offset_radians: 0.25,
                    length_meters: 5.5,
                    width_meters: 2.6,
                },
            })
            .expect("space-a")
            .add_parking_space(ParkingSpaceInput {
                parking_space_key: "space-b",
                parking_facility: None,
                entry: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("edge"),
                    progress_meters: 12.0,
                },
                exit: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("edge"),
                    progress_meters: 13.0,
                },
                geometry: ParkingSpaceGeometryInput {
                    lateral_offset_meters: -3.0,
                    heading_offset_radians: 0.25,
                    length_meters: 5.5,
                    width_meters: 2.6,
                },
            })
            .expect("space-b");
    });
    let mut world =
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100)).expect("install");
    let route = register_named(&mut world, &["edge"]);
    let vehicle = world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 4_000, 0)
                .with_open_entrance(),
        )
        .expect("spawn");
    let first = ParkingSpaceOrdinal::from_raw(0);
    world
        .reserve_parking(
            vehicle,
            ReserveParkingTarget::ExplicitSpace {
                space: first,
                entry_route_occurrence: 0,
            },
        )
        .expect("reserve first space");
    world
        .park_vehicle(vehicle, ParkingTarget::ExplicitSpace(first))
        .expect("park first space");
    assert_eq!(
        world
            .park_vehicle(
                vehicle,
                ParkingTarget::ExplicitSpace(ParkingSpaceOrdinal::from_raw(1)),
            )
            .unwrap_err(),
        ParkingError::NotReserved
    );
}

#[test]
fn virtual_parking_capacity_mixed_pools_leave_and_despawn_are_exact() {
    let revision = compile_virtual_parking_revision(2);
    let mut world =
        install_fixture(revision, WorldConfig::new(16, 8, 1_024, 1_024, 100)).expect("install");
    let route = register_named(&mut world, &["edge"]);
    let profile = VehicleProfileOrdinal::from_raw(0);
    let facility = ParkingFacilityOrdinal::from_raw(0);
    let space = ParkingSpaceOrdinal::from_raw(0);
    let spawn = |world: &mut TrafficWorld, progress_mm| {
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(profile, route, 0, progress_mm, 0).with_open_entrance(),
            )
            .expect("spawn")
    };
    let first = spawn(&mut world, 0);
    let second = spawn(&mut world, 7_000);
    let third = spawn(&mut world, 14_000);
    let unbound = spawn(&mut world, 21_000);

    let first_reserve = ReserveParkingTarget::VirtualPool {
        facility,
        entry_anchor: VirtualEntryAnchorSelector::from_raw(0),
        entry_route_occurrence: 0,
    };
    let second_reserve = ReserveParkingTarget::VirtualPool {
        facility,
        entry_anchor: VirtualEntryAnchorSelector::from_raw(1),
        entry_route_occurrence: 0,
    };
    world
        .reserve_parking(first, first_reserve)
        .expect("first virtual reservation");
    world
        .reserve_parking(second, second_reserve)
        .expect("second virtual reservation");

    let cursor_before_no_change = world.command_cursor();
    let sequence_before_no_change = world.observation_state_sequence();
    assert!(
        world
            .reserve_parking(first, first_reserve)
            .expect("exact reserve no-change")
            .is_no_change()
    );
    assert_eq!(world.command_cursor(), cursor_before_no_change + 1);
    assert_eq!(
        world.observation_state_sequence(),
        sequence_before_no_change
    );

    assert_eq!(
        world
            .reserve_parking(
                third,
                ReserveParkingTarget::VirtualPool {
                    facility,
                    entry_anchor: VirtualEntryAnchorSelector::from_raw(0),
                    entry_route_occurrence: 0,
                },
            )
            .unwrap_err(),
        ParkingError::VirtualCapacityExhausted
    );
    assert_eq!(
        world
            .reserve_parking(
                unbound,
                ReserveParkingTarget::VirtualPool {
                    facility,
                    entry_anchor: VirtualEntryAnchorSelector::from_raw(99),
                    entry_route_occurrence: 0,
                },
            )
            .unwrap_err(),
        ParkingError::EntrySelectorNotOwned
    );
    world
        .reserve_parking(
            third,
            ReserveParkingTarget::ExplicitSpace {
                space,
                entry_route_occurrence: 0,
            },
        )
        .expect("explicit pool remains independent");
    let counts = world
        .parking_facility_counts(facility)
        .expect("facility counts");
    assert_eq!(
        (counts.virtual_pool.capacity, counts.virtual_pool.reserved),
        (2, 2)
    );
    assert_eq!((counts.explicit.capacity, counts.explicit.reserved), (1, 1));
    assert_eq!((counts.total.capacity, counts.total.vacant), (3, 0));

    assert_eq!(
        world
            .park_vehicle(first, ParkingTarget::VirtualPool(facility))
            .unwrap_err(),
        ParkingError::NotArrived
    );
    world
        .cancel_parking(second, ParkingTarget::VirtualPool(facility))
        .expect("cancel exact virtual reservation");
    assert_eq!(
        world
            .cancel_parking(second, ParkingTarget::VirtualPool(facility))
            .unwrap_err(),
        ParkingError::NotReserved
    );

    let parked_virtual = world
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(profile, route, 0, 0),
            ParkingTarget::VirtualPool(facility),
        )
        .expect("sparse occupied virtual member")
        .vehicle;
    assert_eq!(
        world
            .vehicle(parked_virtual)
            .expect("parked state")
            .status(),
        VehicleStatus::Parked
    );
    assert!(
        !world
            .committed_pose_sources()
            .collect::<Vec<_>>()
            .as_slice()
            .iter()
            .any(|(vehicle, _)| *vehicle == parked_virtual)
    );
    let counts = world
        .parking_facility_counts(facility)
        .expect("facility counts");
    assert_eq!(
        (counts.virtual_pool.reserved, counts.virtual_pool.occupied),
        (1, 1)
    );

    let leave = world
        .leave_parking(
            parked_virtual,
            LeaveParkingTarget::VirtualPool {
                facility,
                route,
                exit_anchor: VirtualExitAnchorSelector::from_raw(0),
                exit_route_occurrence: 0,
            },
        )
        .expect("leave at caller-selected virtual exit");
    assert_eq!(leave.route, route);
    assert_eq!(leave.exit_route_occurrence, 0);
    assert_eq!(
        leave.virtual_exit_selector,
        Some(VirtualExitAnchorSelector::from_raw(0))
    );
    let departed = world.vehicle(parked_virtual).expect("departed state");
    assert_eq!(departed.status(), VehicleStatus::Active);
    assert_eq!(departed.progress_mm(), 70_000);
    assert_eq!(world.parking_binding(parked_virtual), None);
    assert!(matches!(
        world
            .committed_pose_sources()
            .collect::<Vec<_>>()
            .as_slice()
            .iter()
            .find(|(vehicle, _)| *vehicle == parked_virtual),
        Some((
            _,
            PoseSource::Lane {
                progress_mm: 70_000,
                ..
            }
        ))
    ));
    assert_eq!(
        world
            .leave_parking(
                parked_virtual,
                LeaveParkingTarget::VirtualPool {
                    facility,
                    route,
                    exit_anchor: VirtualExitAnchorSelector::from_raw(0),
                    exit_route_occurrence: 0,
                },
            )
            .unwrap_err(),
        ParkingError::InvalidVehicleStatus
    );

    world
        .cancel_parking(third, ParkingTarget::ExplicitSpace(space))
        .expect("release explicit reservation");
    let parked_explicit = world
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(profile, route, 0, 0),
            ParkingTarget::ExplicitSpace(space),
        )
        .expect("parked explicit member")
        .vehicle;
    assert!(matches!(
        world
            .committed_pose_sources()
            .collect::<Vec<_>>()
            .as_slice()
            .iter()
            .find(|(vehicle, _)| *vehicle == parked_explicit),
        Some((_, PoseSource::Parking { space: found })) if *found == space
    ));

    let reserved_record = world.despawn_vehicle(first).expect("despawn reserved");
    assert!(matches!(
        reserved_record.parking_binding,
        Some(ParkingBinding::Reserved(reservation))
            if reservation.target() == ParkingTarget::VirtualPool(facility)
    ));
    assert_eq!(
        world.despawn_vehicle(first).unwrap_err(),
        ParkingError::StaleVehicle
    );
    let parked_record = world
        .despawn_vehicle(parked_explicit)
        .expect("despawn parked");
    assert_eq!(parked_record.status, VehicleStatus::Parked);
    assert_eq!(world.committed_parking_occupant(space), None);
    let active_record = world.despawn_vehicle(unbound).expect("despawn active");
    assert_eq!(active_record.status, VehicleStatus::Active);
    assert_eq!(active_record.parking_binding, None);

    let mut completed_world = install_fixture(
        compile_virtual_parking_revision(1),
        WorldConfig::new(4, 4, 1_024, 1_024, 100),
    )
    .expect("install completed fixture");
    let completed_route = register_named(&mut completed_world, &["edge"]);
    let completed = completed_world
        .spawn_vehicle(
            VehicleSpawnInput::new(profile, completed_route, 0, 100_000, 0).with_open_entrance(),
        )
        .expect("spawn at route end");
    completed_world
        .step(TickInput::new(100))
        .expect("commit completed status");
    assert_eq!(
        completed_world
            .vehicle(completed)
            .expect("completed")
            .status(),
        VehicleStatus::Completed
    );
    assert_eq!(
        completed_world
            .despawn_vehicle(completed)
            .expect("despawn completed")
            .status,
        VehicleStatus::Completed
    );
}

#[test]
fn virtual_arrival_is_observed_once_then_park_is_pose_less_and_narrowly_idempotent() {
    let revision = compile_virtual_parking_revision(1);
    let mut world =
        install_fixture(revision, WorldConfig::new(4, 4, 1_024, 1_024, 100)).expect("install");
    let route = register_named(&mut world, &["edge"]);
    let facility = ParkingFacilityOrdinal::from_raw(0);
    let target = ParkingTarget::VirtualPool(facility);
    let vehicle = world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0, 0)
                .with_open_entrance(),
        )
        .expect("spawn");
    let reserve = world
        .reserve_parking(
            vehicle,
            ReserveParkingTarget::VirtualPool {
                facility,
                entry_anchor: VirtualEntryAnchorSelector::from_raw(0),
                entry_route_occurrence: 0,
            },
        )
        .expect("reserve")
        .into_record();
    assert_eq!(reserve.route, route);
    assert!(!reserve.arrived);

    let mut observed = Vec::new();
    for _ in 0..1_000 {
        let outcome = world.step(TickInput::new(100)).expect("approach step");
        observed.extend_from_slice(outcome.parking_arrivals());
        if world.parking_arrived(vehicle, target) {
            break;
        }
    }
    assert_eq!(
        observed,
        vec![laneflow_runtime::ParkingArrivalObservation { vehicle, target }]
    );
    let arrived = world.vehicle(vehicle).expect("arrived state");
    assert_eq!(arrived.status(), VehicleStatus::Active);
    assert_eq!(arrived.route_edge_index(), 0);
    assert_eq!(arrived.progress_mm(), 20_000);
    assert_eq!(arrived.speed_mm_s(), 0);
    assert_eq!(arrived.carry_um(), 0);
    assert!(
        world
            .step(TickInput::new(100))
            .expect("arrival remains committed")
            .parking_arrivals()
            .is_empty()
    );
    assert_eq!(
        world
            .park_vehicle(
                vehicle,
                ParkingTarget::ExplicitSpace(ParkingSpaceOrdinal::from_raw(0)),
            )
            .unwrap_err(),
        ParkingError::NotReserved
    );

    world
        .park_vehicle(vehicle, target)
        .expect("park exact pair");
    assert_eq!(
        world.vehicle(vehicle).expect("parked state").status(),
        VehicleStatus::Parked
    );
    assert!(
        world
            .committed_pose_sources()
            .collect::<Vec<_>>()
            .as_slice()
            .is_empty()
    );
    let cursor_before_no_change = world.command_cursor();
    let sequence_before_no_change = world.observation_state_sequence();
    assert!(
        world
            .park_vehicle(vehicle, target)
            .expect("exact park no-change")
            .is_no_change()
    );
    assert_eq!(world.command_cursor(), cursor_before_no_change + 1);
    assert_eq!(
        world.observation_state_sequence(),
        sequence_before_no_change
    );
    let parked_state = world.vehicle(vehicle).expect("parked before empty tick");
    world
        .step(TickInput::new(100))
        .expect("parked-only fixed tick");
    assert_eq!(world.vehicle(vehicle), Some(parked_state));
}

#[test]
fn virtual_reserved_and_occupied_bindings_round_trip_in_snapshot_v6() {
    let revision = compile_virtual_parking_revision(2);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(8, 4, 1_024, 1_024, 100),
    )
    .expect("install");
    let route = register_named(&mut world, &["edge"]);
    let profile = VehicleProfileOrdinal::from_raw(0);
    let facility = ParkingFacilityOrdinal::from_raw(0);
    let reserved = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, route, 0, 0, 0).with_open_entrance())
        .expect("spawn reserved vehicle");
    world
        .reserve_parking(
            reserved,
            ReserveParkingTarget::VirtualPool {
                facility,
                entry_anchor: VirtualEntryAnchorSelector::from_raw(1),
                entry_route_occurrence: 0,
            },
        )
        .expect("reserve virtual entry one");
    let occupied = world
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(profile, route, 0, 0),
            ParkingTarget::VirtualPool(facility),
        )
        .expect("spawn occupied virtual")
        .vehicle;

    assert_eq!(laneflow_runtime::SNAPSHOT_FORMAT_VERSION, 6);
    assert_eq!(laneflow_runtime::RUNTIME_STATE_VERSION, 5);
    assert_eq!(laneflow_runtime::RUNTIME_STATE_DIGEST_VERSION, 7);
    let snapshot = world.capture_snapshot().expect("capture");
    let digest = deterministic_state_digest(&snapshot).expect("snapshot digest");
    let reserved_id = snapshot
        .vehicles()
        .iter()
        .find(|vehicle| vehicle.status() == VehicleStatus::Active)
        .expect("captured reserved")
        .snapshot_vehicle_id();
    let occupied_id = snapshot
        .vehicles()
        .iter()
        .find(|vehicle| vehicle.status() == VehicleStatus::Parked)
        .expect("captured occupied")
        .snapshot_vehicle_id();
    let bytes = encode_lfrs(&snapshot);
    let restored = restore_lfrs(
        &bytes,
        revision,
        world.committed_source().clone(),
        world.config(),
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
    )
    .expect("restore v2");
    assert_eq!(
        deterministic_state_digest(
            &restored
                .world()
                .capture_snapshot()
                .expect("recapture restored world")
        )
        .expect("restored digest"),
        digest
    );
    let restored_reserved = restored
        .vehicle_handle(reserved_id)
        .expect("restored reserved handle");
    let restored_occupied = restored
        .vehicle_handle(occupied_id)
        .expect("restored occupied handle");
    assert!(matches!(
        restored.world().parking_binding(restored_reserved),
        Some(ParkingBinding::Reserved(reservation))
            if reservation.target() == ParkingTarget::VirtualPool(facility)
                && reservation.route() == restored.world().vehicle(restored_reserved).expect("state").route()
                && reservation.entry_route_occurrence() == 0
                && reservation.virtual_entry_selector() == Some(VirtualEntryAnchorSelector::from_raw(1))
    ));
    assert_eq!(
        restored.world().parking_binding(restored_occupied),
        Some(ParkingBinding::Occupied(ParkingTarget::VirtualPool(
            facility
        )))
    );
    assert_eq!(
        restored
            .world()
            .parking_facility_counts(facility)
            .expect("restored counts")
            .virtual_pool,
        laneflow_runtime::ParkingPoolCounts {
            capacity: 2,
            reserved: 1,
            occupied: 1,
            vacant: 0,
        }
    );
    assert!(
        !restored
            .world()
            .committed_pose_sources()
            .collect::<Vec<_>>()
            .as_slice()
            .iter()
            .any(|(vehicle, _)| *vehicle == restored_occupied)
    );
    let _ = (reserved, occupied);
}

#[test]
fn repeated_leave_rejections_do_not_reuse_an_occupant_from_a_retired_incarnation() {
    let (mut world, route, facility, parked) = parked_virtual_world();
    let leave = LeaveParkingTarget::VirtualPool {
        facility,
        route,
        exit_anchor: VirtualExitAnchorSelector::from_raw(0),
        exit_route_occurrence: 0,
    };
    let first = world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 63_499, 1)
                .with_open_entrance(),
        )
        .unwrap();
    let before = world.capture_snapshot().unwrap();
    for _ in 0..3 {
        assert_eq!(
            world.leave_parking(parked, leave),
            Err(ParkingError::LeaveUnsafeFollower { follower: first })
        );
        assert_eq!(world.capture_snapshot().unwrap(), before);
    }
    assert_eq!(
        world.leave_parking(
            parked,
            LeaveParkingTarget::VirtualPool {
                facility,
                route,
                exit_anchor: VirtualExitAnchorSelector::from_raw(99),
                exit_route_occurrence: 0,
            }
        ),
        Err(ParkingError::ExitSelectorNotOwned)
    );
    world.despawn_vehicle(first).unwrap();
    let replacement = world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 61_000, 10_000)
                .with_open_entrance(),
        )
        .unwrap();
    assert_ne!(replacement, first);
    assert!(world.vehicle(first).is_none());
    let before = world.capture_snapshot().unwrap();
    // 旧 incarnation 的 footprint 在新 follower 前方。若沿用旧索引，它会被
    // 错认成遮住停车 candidate 的前车，从而跳过这次本应失败的安全检查。
    assert_eq!(
        world.leave_parking(parked, leave),
        Err(ParkingError::LeaveUnsafeFollower {
            follower: replacement
        })
    );
    assert_eq!(world.capture_snapshot().unwrap(), before);
    world.despawn_vehicle(replacement).unwrap();
    world.leave_parking(parked, leave).unwrap();
    assert_eq!(
        world.vehicle(parked).unwrap().status(),
        VehicleStatus::Active
    );
}

#[test]
fn leave_failures_are_atomic_and_follow_the_one_millimetre_emergency_boundary() {
    let (mut overlap_world, route, facility, parked) = parked_virtual_world();
    let before_state = overlap_world.vehicle(parked);
    let before_binding = overlap_world.parking_binding(parked);
    let before_counts = overlap_world.parking_facility_counts(facility);
    let before_pose = overlap_world.committed_pose_sources().collect::<Vec<_>>();
    let before_cursor = overlap_world.command_cursor();
    let before_sequence = overlap_world.observation_state_sequence();
    assert_eq!(
        overlap_world
            .leave_parking(
                parked,
                LeaveParkingTarget::VirtualPool {
                    facility,
                    route,
                    exit_anchor: VirtualExitAnchorSelector::from_raw(99),
                    exit_route_occurrence: 0,
                },
            )
            .unwrap_err(),
        ParkingError::ExitSelectorNotOwned
    );
    assert_eq!(overlap_world.vehicle(parked), before_state);
    assert_eq!(overlap_world.parking_binding(parked), before_binding);
    assert_eq!(
        overlap_world.parking_facility_counts(facility),
        before_counts
    );
    assert_eq!(
        overlap_world.committed_pose_sources().collect::<Vec<_>>(),
        before_pose
    );
    assert_eq!(overlap_world.command_cursor(), before_cursor);
    assert_eq!(overlap_world.observation_state_sequence(), before_sequence);

    let blocker = overlap_world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 70_000, 0)
                .with_open_entrance(),
        )
        .expect("physical blocker");
    let before_state = overlap_world.vehicle(parked);
    let before_binding = overlap_world.parking_binding(parked);
    let before_counts = overlap_world.parking_facility_counts(facility);
    let before_pose = overlap_world.committed_pose_sources().collect::<Vec<_>>();
    let before_cursor = overlap_world.command_cursor();
    assert_eq!(
        overlap_world
            .leave_parking(
                parked,
                LeaveParkingTarget::VirtualPool {
                    facility,
                    route,
                    exit_anchor: VirtualExitAnchorSelector::from_raw(0),
                    exit_route_occurrence: 0,
                },
            )
            .unwrap_err(),
        ParkingError::LeavePhysicalOverlap { blocker }
    );
    assert_eq!(overlap_world.vehicle(parked), before_state);
    assert_eq!(overlap_world.parking_binding(parked), before_binding);
    assert_eq!(
        overlap_world.parking_facility_counts(facility),
        before_counts
    );
    assert_eq!(
        overlap_world.committed_pose_sources().collect::<Vec<_>>(),
        before_pose
    );
    assert_eq!(overlap_world.command_cursor(), before_cursor);

    let (mut rejected_world, route, facility, parked) = parked_virtual_world();
    let one_mm_tolerance_follower = rejected_world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 63_499, 1)
                .with_open_entrance(),
        )
        .expect("moving follower at preserved-gap plus one millimetre");
    let rejected_state = rejected_world.vehicle(parked);
    let rejected_binding = rejected_world.parking_binding(parked);
    let rejected_counts = rejected_world.parking_facility_counts(facility);
    let rejected_cursor = rejected_world.command_cursor();
    assert_eq!(
        rejected_world
            .leave_parking(
                parked,
                LeaveParkingTarget::VirtualPool {
                    facility,
                    route,
                    exit_anchor: VirtualExitAnchorSelector::from_raw(0),
                    exit_route_occurrence: 0,
                },
            )
            .unwrap_err(),
        ParkingError::LeaveUnsafeFollower {
            follower: one_mm_tolerance_follower
        }
    );
    assert_eq!(rejected_world.vehicle(parked), rejected_state);
    assert_eq!(rejected_world.parking_binding(parked), rejected_binding);
    assert_eq!(
        rejected_world.parking_facility_counts(facility),
        rejected_counts
    );
    assert_eq!(rejected_world.command_cursor(), rejected_cursor);

    let (mut accepted_world, route, facility, parked) = parked_virtual_world();
    accepted_world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 63_498, 1)
                .with_open_entrance(),
        )
        .expect("moving follower outside one millimetre tolerance");
    accepted_world
        .leave_parking(
            parked,
            LeaveParkingTarget::VirtualPool {
                facility,
                route,
                exit_anchor: VirtualExitAnchorSelector::from_raw(0),
                exit_route_occurrence: 0,
            },
        )
        .expect("two available millimetres admit emergency-feasible follower");
    assert_eq!(
        accepted_world.vehicle(parked).expect("left").progress_mm(),
        70_000
    );

    let (mut stationary_world, route, facility, parked) = parked_virtual_world();
    stationary_world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 74_500, 0)
                .with_open_entrance(),
        )
        .expect("stationary follower with sub-comfort gap");
    stationary_world
        .leave_parking(
            parked,
            LeaveParkingTarget::VirtualPool {
                facility,
                route,
                exit_anchor: VirtualExitAnchorSelector::from_raw(1),
                exit_route_occurrence: 0,
            },
        )
        .expect("stationary follower needs only physical non-overlap");
    assert_eq!(
        stationary_world.spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 80_000, 0,)
                .with_open_entrance()
        ),
        Err(laneflow_runtime::SpawnError::Overlap),
        "successful leave registers the new road footprint immediately",
    );
    assert_eq!(
        stationary_world
            .vehicle(parked)
            .expect("left from second selector")
            .progress_mm(),
        80_000
    );
}

#[test]
fn leave_overlap_detects_cross_predecessor_and_repeated_occurrence_geometry() {
    let revision = compile_exit_topology_revision();
    let profile = VehicleProfileOrdinal::from_raw(0);
    let facility = ParkingFacilityOrdinal::from_raw(0);

    let mut cross_world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(8, 4, 1_024, 1_024, 100),
    )
    .expect("install cross-edge world");
    let cross_route = register_named(&mut cross_world, &["loop", "middle", "loop"]);
    let cross_parked = cross_world
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(profile, cross_route, 0, 0),
            ParkingTarget::VirtualPool(facility),
        )
        .expect("cross-edge parked")
        .vehicle;
    let predecessor_blocker = cross_world
        .spawn_vehicle(
            VehicleSpawnInput::new(profile, cross_route, 0, 9_000, 0).with_open_entrance(),
        )
        .expect("predecessor blocker");
    assert_eq!(
        cross_world
            .leave_parking(
                cross_parked,
                LeaveParkingTarget::VirtualPool {
                    facility,
                    route: cross_route,
                    exit_anchor: VirtualExitAnchorSelector::from_raw(1),
                    exit_route_occurrence: 1,
                },
            )
            .unwrap_err(),
        ParkingError::LeavePhysicalOverlap {
            blocker: predecessor_blocker
        }
    );

    let mut repeated_world = install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100))
        .expect("install repeated-edge world");
    let repeated_route = register_named(&mut repeated_world, &["loop", "middle", "loop"]);
    let repeated_parked = repeated_world
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(profile, repeated_route, 1, 5_000),
            ParkingTarget::VirtualPool(facility),
        )
        .expect("repeated parked")
        .vehicle;
    let repeated_blocker = repeated_world
        .spawn_vehicle(
            VehicleSpawnInput::new(profile, repeated_route, 0, 4_500, 0).with_open_entrance(),
        )
        .expect("same physical edge on earlier occurrence");
    assert_eq!(
        repeated_world
            .leave_parking(
                repeated_parked,
                LeaveParkingTarget::VirtualPool {
                    facility,
                    route: repeated_route,
                    exit_anchor: VirtualExitAnchorSelector::from_raw(0),
                    exit_route_occurrence: 2,
                },
            )
            .unwrap_err(),
        ParkingError::LeavePhysicalOverlap {
            blocker: repeated_blocker
        }
    );
}

#[test]
fn rebind_compares_the_complete_cross_edge_body_footprint() {
    let revision = compile_rebind_revision();
    let mut world =
        install_fixture(revision, WorldConfig::new(8, 8, 1_024, 1_024, 100)).expect("install");
    let old_route = register_named(&mut world, &["left", "current", "tail"]);
    let new_route = register_named(&mut world, &["right", "current", "tail"]);
    let profile = VehicleProfileOrdinal::from_raw(0);
    let facility = ParkingFacilityOrdinal::from_raw(0);
    let crossing = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, old_route, 1, 2_000, 0).with_open_entrance())
        .expect("crossing predecessor vehicle");
    world
        .reserve_parking(
            crossing,
            ReserveParkingTarget::VirtualPool {
                facility,
                entry_anchor: VirtualEntryAnchorSelector::from_raw(0),
                entry_route_occurrence: 2,
            },
        )
        .expect("reserve old route");
    let state_before = world.vehicle(crossing);
    let binding_before = world.parking_binding(crossing);
    let counts_before = world.parking_facility_counts(facility);
    let cursor_before = world.command_cursor();
    assert_eq!(
        world
            .rebind_parking_route(
                crossing,
                RebindParkingTarget::VirtualPool {
                    facility,
                    new_route,
                    new_current_route_occurrence: 1,
                    new_entry_anchor: VirtualEntryAnchorSelector::from_raw(0),
                    new_entry_route_occurrence: 2,
                },
            )
            .unwrap_err(),
        ParkingError::RebindBodyFootprintMismatch
    );
    assert_eq!(world.vehicle(crossing), state_before);
    assert_eq!(world.parking_binding(crossing), binding_before);
    assert_eq!(world.parking_facility_counts(facility), counts_before);
    assert_eq!(world.command_cursor(), cursor_before);

    world
        .despawn_vehicle(crossing)
        .expect("release crossing reservation");
    let contained = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, old_route, 1, 5_000, 0).with_open_entrance())
        .expect("body fully on current edge");
    world
        .reserve_parking(
            contained,
            ReserveParkingTarget::VirtualPool {
                facility,
                entry_anchor: VirtualEntryAnchorSelector::from_raw(0),
                entry_route_occurrence: 2,
            },
        )
        .expect("reserve contained vehicle");
    let rebound = world
        .rebind_parking_route(
            contained,
            RebindParkingTarget::VirtualPool {
                facility,
                new_route,
                new_current_route_occurrence: 1,
                new_entry_anchor: VirtualEntryAnchorSelector::from_raw(0),
                new_entry_route_occurrence: 2,
            },
        )
        .expect("physically identical rebind")
        .into_record();
    assert_eq!(rebound.old_route, old_route);
    assert_eq!(rebound.new_route, new_route);
    assert_eq!(rebound.old_current_route_occurrence, 1);
    assert_eq!(rebound.new_current_route_occurrence, 1);
    assert_eq!(rebound.new_entry_route_occurrence, 2);
    assert_eq!(
        rebound.virtual_entry_selector,
        Some(VirtualEntryAnchorSelector::from_raw(0))
    );
    let cursor_before_no_change = world.command_cursor();
    let sequence_before_no_change = world.observation_state_sequence();
    assert!(
        world
            .rebind_parking_route(
                contained,
                RebindParkingTarget::VirtualPool {
                    facility,
                    new_route,
                    new_current_route_occurrence: 1,
                    new_entry_anchor: VirtualEntryAnchorSelector::from_raw(0),
                    new_entry_route_occurrence: 2,
                },
            )
            .expect("exact rebind no-change")
            .is_no_change()
    );
    assert_eq!(world.command_cursor(), cursor_before_no_change + 1);
    assert_eq!(
        world.observation_state_sequence(),
        sequence_before_no_change
    );
    assert!(matches!(
        world.parking_binding(contained),
        Some(ParkingBinding::Reserved(reservation)) if reservation.route() == new_route
    ));
    assert_eq!(
        world.spawn_vehicle(
            VehicleSpawnInput::new(profile, old_route, 1, 5_000, 0).with_open_entrance()
        ),
        Err(laneflow_runtime::SpawnError::Overlap),
        "rebind preserves physical registration"
    );
    world.despawn_vehicle(contained).unwrap();
    world
        .spawn_vehicle(VehicleSpawnInput::new(profile, old_route, 1, 5_000, 0).with_open_entrance())
        .expect("despawn removes the rebound vehicle footprint");
}

#[test]
fn leave_research_includes_both_upstream_merge_routes_and_committed_prefix() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        for key in ["left", "right"] {
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: 10.0,
                    speed_limit_meters_per_second: 15.0,
                    successors: &[LaneEdgeReference::local("shared")],
                })
                .unwrap();
        }
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "shared",
                length_meters: 10.0,
                speed_limit_meters_per_second: 15.0,
                successors: &[],
            })
            .unwrap();
        let anchors = [ParkingLaneAnchorInput {
            lane_edge: LaneEdgeReference::local("shared"),
            progress_meters: 6.0,
        }];
        module
            .add_parking_facility(ParkingFacilityInput {
                parking_facility_key: "facility",
                virtual_capacity: 2,
                virtual_entries: &anchors,
                virtual_exits: &anchors,
            })
            .unwrap();
    });
    let mut world = install_fixture(revision, WorldConfig::new(8, 4, 32, 1, 100)).unwrap();
    let route = register_named(&mut world, &["shared"]);
    let left = register_named(&mut world, &["left", "shared"]);
    let right = register_named(&mut world, &["right", "shared"]);
    let facility = ParkingFacilityOrdinal::from_raw(0);
    let profile = VehicleProfileOrdinal::from_raw(0);
    let parked: Vec<_> = (0..2)
        .map(|_| {
            world
                .spawn_parked_vehicle(
                    ParkedVehicleSpawnInput::new(profile, route, 0, 0),
                    ParkingTarget::VirtualPool(facility),
                )
                .unwrap()
                .vehicle
        })
        .collect();
    let first = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, left, 0, 9_000, 10_000).with_open_entrance())
        .unwrap();
    let second = world
        .spawn_vehicle(
            VehicleSpawnInput::new(profile, right, 0, 9_000, 10_000).with_open_entrance(),
        )
        .unwrap();
    let leave = LeaveParkingTarget::VirtualPool {
        facility,
        route,
        exit_anchor: VirtualExitAnchorSelector::from_raw(0),
        exit_route_occurrence: 0,
    };
    let before = world.capture_snapshot().unwrap();
    assert_eq!(
        world.leave_parking(parked[0], leave),
        Err(ParkingError::LeaveUnsafeFollower { follower: first })
    );
    assert_eq!(world.capture_snapshot().unwrap(), before);
    world.despawn_vehicle(first).unwrap();
    let replacement = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, left, 0, 9_000, 10_000).with_open_entrance())
        .unwrap();
    assert_ne!(replacement, first);
    // 同一槽位重新生成后位于 live 顺序末端，首错仍是右侧上游车辆。
    assert_eq!(
        world.leave_parking(parked[0], leave),
        Err(ParkingError::LeaveUnsafeFollower { follower: second })
    );
    world.despawn_vehicle(second).unwrap();
    assert_eq!(
        world.leave_parking(parked[0], leave),
        Err(ParkingError::LeaveUnsafeFollower {
            follower: replacement
        })
    );
    world.despawn_vehicle(replacement).unwrap();
    world.leave_parking(parked[0], leave).unwrap();
    let committed = world.capture_snapshot().unwrap();
    assert_eq!(
        world.leave_parking(parked[1], leave),
        Err(ParkingError::LeavePhysicalOverlap { blocker: parked[0] })
    );
    assert_eq!(world.capture_snapshot().unwrap(), committed);
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn follower_on_diverge_respects_leader_overhang_on_shared_stem() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "stem",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[
                    LaneEdgeReference::local("left"),
                    LaneEdgeReference::local("right"),
                ],
            })
            .expect("stem")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "left",
                length_meters: 20.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .expect("left")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "right",
                length_meters: 20.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .expect("right");
    });
    let traffic = revision.traffic();
    let count = traffic.lane_edge_count();
    let stem = (0..count)
        .map(laneflow_static_contract::LaneEdgeOrdinal::from_raw)
        .find(|edge| {
            traffic
                .successors(*edge)
                .is_some_and(|successors| successors.len() == 2)
        })
        .expect("stem");
    let branches = traffic.successors(stem).expect("branches");
    let left = branches[0];
    let right = branches[1];
    let mut world =
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100)).expect("install");
    let leader_route = world
        .register_route(RouteRegisterInput::new(vec![stem, left]))
        .expect("left route");
    let follower_route = world
        .register_route(RouteRegisterInput::new(vec![stem, right]))
        .expect("right route");
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), leader_route, 1, 500, 0)
                .with_open_entrance(),
        )
        .expect("leader on left, tail on stem");
    let follower = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                follower_route,
                0,
                5_000,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("follower on stem");
    world.step(TickInput::new(100)).expect("step");
    let PoseSource::Lane { progress_mm, .. } = world
        .committed_pose_sources()
        .collect::<Vec<_>>()
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == follower)
        .expect("follower pose")
        .1
    else {
        panic!("follower must stay on lane");
    };
    assert!(
        progress_mm < 6_000,
        "follower must not enter leader overhang on stem, progress={progress_mm}"
    );
}

#[test]
fn large_delta_travel_does_not_exceed_speed_limit_envelope() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge",
                length_meters: 1_000.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .expect("edge");
    });
    let mut world =
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 1_000)).expect("install");
    let route = register_named(&mut world, &["edge"]);
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0, 0)
                .with_open_entrance(),
        )
        .expect("spawn");
    for _ in 0..20 {
        world.step(TickInput::new(1_000)).expect("step");
    }
    let PoseSource::Lane { progress_mm, .. } = world
        .committed_pose_sources()
        .collect::<Vec<_>>()
        .as_slice()[0]
        .1
    else {
        panic!("lane pose");
    };
    assert!(
        progress_mm <= 200_000,
        "travel must not exceed speed-limit envelope, progress={progress_mm}"
    );
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn speed_down_transition_caps_next_tick_travel() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "fast",
                length_meters: 20.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[LaneEdgeReference::local("slow")],
            })
            .expect("fast")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "slow",
                length_meters: 100.0,
                speed_limit_meters_per_second: 1.0,
                successors: &[],
            })
            .expect("slow");
    });
    let mut world =
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 1_000)).expect("install");
    let route = register_named(&mut world, &["fast", "slow"]);
    let vehicle = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 18_000, 10_000)
                .with_open_entrance(),
        )
        .expect("spawn near fast/slow boundary");
    world.step(TickInput::new(1_000)).expect("approach/cross");
    let PoseSource::Lane {
        edge: after_first,
        progress_mm: first_progress,
    } = world
        .committed_pose_sources()
        .collect::<Vec<_>>()
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == vehicle)
        .expect("pose")
        .1
    else {
        panic!("lane pose");
    };
    world.step(TickInput::new(1_000)).expect("slow edge tick");
    let PoseSource::Lane {
        edge: after_second,
        progress_mm: second_progress,
    } = world
        .committed_pose_sources()
        .collect::<Vec<_>>()
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == vehicle)
        .expect("pose")
        .1
    else {
        panic!("lane pose");
    };
    let travelled = if after_second == after_first {
        second_progress - first_progress
    } else {
        second_progress
    };
    assert!(
        travelled <= 1_000,
        "tick on or after 1 m/s edge must not keep a 10 m/s envelope, travelled={travelled}, first={first_progress:?} {after_first:?}, second={second_progress:?} {after_second:?}"
    );
}

#[test]
fn equal_limit_edge_boundary_does_not_stop_the_vehicle() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "a",
                length_meters: 20.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[LaneEdgeReference::local("b")],
            })
            .expect("a")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "b",
                length_meters: 100.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .expect("b");
    });
    let mut world =
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100)).expect("install");
    let route = register_named(&mut world, &["a", "b"]);
    let vehicle = world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 19_600, 10_000)
                .with_open_entrance(),
        )
        .expect("spawn near equal-limit boundary");
    world.step(TickInput::new(100)).expect("step");
    let PoseSource::Lane { edge, progress_mm } = world
        .committed_pose_sources()
        .collect::<Vec<_>>()
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == vehicle)
        .expect("pose")
        .1
    else {
        panic!("lane pose");
    };
    assert!(
        progress_mm != 20_000,
        "equal-limit crossing must not stop at the first-edge end, edge={edge:?} progress={progress_mm}"
    );
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn infeasible_stop_before_lower_limit_still_enters() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "fast",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[LaneEdgeReference::local("slower")],
            })
            .expect("fast")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "slower",
                length_meters: 100.0,
                speed_limit_meters_per_second: 8.0,
                successors: &[],
            })
            .expect("slower");
    });
    let mut world =
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 1_000)).expect("install");
    let route = register_named(&mut world, &["fast", "slower"]);
    let vehicle = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 9_000, 10_000)
                .with_open_entrance(),
        )
        .expect("spawn 1 m before a 10→8 drop");
    world.step(TickInput::new(1_000)).expect("step");
    let PoseSource::Lane { edge, progress_mm } = world
        .committed_pose_sources()
        .collect::<Vec<_>>()
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == vehicle)
        .expect("pose")
        .1
    else {
        panic!("lane pose");
    };
    let first = world.route_edges(route).expect("edges")[0];
    assert!(
        edge != first || progress_mm != 10_000,
        "must enter when even a stop this tick overshoots the slower-edge start, edge={edge:?} progress={progress_mm}"
    );
}

#[test]
fn already_below_downstream_limit_does_not_stop_at_boundary() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "posted-fast",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[LaneEdgeReference::local("mid")],
            })
            .expect("posted-fast")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "mid",
                length_meters: 100.0,
                speed_limit_meters_per_second: 5.0,
                successors: &[],
            })
            .expect("mid");
    });
    let mut world =
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 1_000)).expect("install");
    let route = register_named(&mut world, &["posted-fast", "mid"]);
    let vehicle = world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 9_000, 2_000)
                .with_open_entrance(),
        )
        .expect("spawn already slower than the 5 m/s next edge");
    world.step(TickInput::new(1_000)).expect("step");
    let PoseSource::Lane { edge, progress_mm } = world
        .committed_pose_sources()
        .collect::<Vec<_>>()
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == vehicle)
        .expect("pose")
        .1
    else {
        panic!("lane pose");
    };
    let first = world.route_edges(route).expect("edges")[0];
    assert!(
        edge != first || progress_mm != 10_000,
        "already-legal speed must not be clamped to a stop at the posted drop, edge={edge:?} progress={progress_mm}"
    );
}

fn add_signalized_corridor(
    module: &mut SyntheticModuleBuilder,
    phase_ms: u64,
    aspect: SignalAspect,
) {
    let groups = [SignalGroupReference::local("group-entry")];
    let go_states = [SignalGroupStateInput {
        signal_group: SignalGroupReference::local("group-entry"),
        aspect,
    }];
    module
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[LaneEdgeReference::local("middle")],
        })
        .expect("entry")
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "middle",
            length_meters: 8.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[LaneEdgeReference::local("exit")],
        })
        .expect("middle")
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit",
            length_meters: 12.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .expect("exit")
        .add_junction(JunctionInput {
            junction_key: "junction-main",
        })
        .expect("junction")
        .add_movement(MovementInput {
            turn_direction: None,
            movement_key: "movement-through",
            junction: JunctionReference::local("junction-main"),
            directed_entry_approach_key: "approach-westbound",
            directed_exit_approach_key: "approach-eastbound",
        })
        .expect("movement")
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path-main",
            movement: MovementReference::local("movement-through"),
            entry_edge: LaneEdgeReference::local("entry"),
            internal_edges: &[LaneEdgeReference::local("middle")],
            exit_edge: LaneEdgeReference::local("exit"),
        })
        .expect("path")
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-entry",
            lane_edge: LaneEdgeReference::local("entry"),
        })
        .expect("stop")
        .add_signal_group(SignalGroupInput {
            signal_group_key: "group-entry",
        })
        .expect("group")
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-entry",
            maneuver_path: ManeuverPathReference::local("path-main"),
            transition_index: 0,
            stop_line: StopLineReference::local("stop-entry"),
            signal_control: SignalControlInput::Group(SignalGroupReference::local("group-entry")),
        })
        .expect("gate")
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: "controller-main",
            offset_ms: 0,
            signal_groups: &groups,
            phases: &[SignalPhaseInput {
                signal_phase_key: "phase-go",
                duration_ms: phase_ms,
                states: &go_states,
            }],
        })
        .expect("controller");
    test_policy::add_gate_policy(
        module,
        "signal-policy",
        &[(
            "gate-entry",
            laneflow_compiler::GateInterpretation::ProtectedGroup,
        )],
    );
}

#[test]
fn install_rejects_phase_shorter_than_tick() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        add_signalized_corridor(module, 8, SignalAspect::Green);
    });
    assert_eq!(
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 16))
            .map(|_| ())
            .unwrap_err(),
        InstallError::PhaseShorterThanTick
    );
}

#[test]
fn hop_preserves_active_state_and_does_not_force_zero_carry() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "first",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[LaneEdgeReference::local("second")],
            })
            .expect("first")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "second",
                length_meters: 100.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .expect("second");
    });
    let mut world =
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 4)).expect("install");
    let route = register_named(&mut world, &["first", "second"]);
    let vehicle = world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 9_999, 3_141)
                .with_open_entrance(),
        )
        .expect("spawn 1 mm before hop at 3.141 m/s");
    world.step(TickInput::new(4)).expect("step");
    let state = world.vehicle(vehicle).expect("state");
    assert_eq!(state.route_edge_index(), 1);
    assert_eq!(state.status(), VehicleStatus::Active);
    assert_ne!(state.speed_mm_s(), 0);
    assert_ne!(
        state.carry_um(),
        0,
        "permitted hop must keep sub-millimetre remainder"
    );
}

#[test]
fn sub_millimetre_boundary_restart_respects_signal_and_restores() {
    for aspect in [SignalAspect::Green, SignalAspect::Red] {
        let revision = compile_revision(|module| {
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
                        max_acceleration_meters_per_second_squared: 2.5,
                        ..iidm()
                    },
                })
                .expect("profile");
            add_signalized_corridor(module, 1_024, aspect);
        });
        let config = WorldConfig::new(1, 1, 3, 0, 16);
        let mut world = install_fixture(Arc::clone(&revision), config).expect("install");
        let route = register_named(&mut world, &["entry", "middle", "exit"]);
        let vehicle = world
            .spawn_vehicle(
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 10_000, 0)
                    .with_open_entrance(),
            )
            .expect("spawn stopped at Gate boundary");
        world.step(TickInput::new(16)).expect("restart at boundary");

        let state = world.vehicle(vehicle).expect("state");
        let permitted = aspect == SignalAspect::Green;
        // 0.5 * 2.5 m/s² * (16 ms)² = 320 um; no whole millimetre is available.
        assert_eq!(
            (
                state.route_edge_index(),
                state.progress_mm(),
                state.carry_um(),
                state.speed_mm_s(),
            ),
            if permitted {
                (1, 0, 320, 40)
            } else {
                (0, 10_000, 0, 0)
            },
            "{aspect:?} must commit a valid cursor"
        );
        assert_eq!(state.status(), VehicleStatus::Active);
        assert_eq!(
            world
                .latest_transition_events()
                .iter()
                .filter(|event| matches!(
                    event.kind(),
                    laneflow_runtime::TrafficTransitionKind::GateCrossed { .. }
                ))
                .count(),
            usize::from(permitted)
        );

        let snapshot = world.capture_snapshot().expect("capture boundary tick");
        let mut restored = restore_lfrs(
            &encode_lfrs(&snapshot),
            revision,
            world.committed_source().clone(),
            config,
            laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
            SnapshotRestoreLimits::new(1_048_576, 1_024),
        )
        .expect("the boundary tick must restore")
        .into_world();
        assert_eq!(
            deterministic_state_digest(&restored.capture_snapshot().expect("recapture"))
                .expect("restored digest"),
            deterministic_state_digest(&snapshot).expect("original digest")
        );
        world.step(TickInput::new(16)).expect("continue original");
        restored
            .step(TickInput::new(16))
            .expect("continue restored");
        assert_eq!(
            deterministic_state_digest(&restored.capture_snapshot().expect("restored next tick"))
                .expect("restored digest"),
            deterministic_state_digest(&world.capture_snapshot().expect("original next tick"))
                .expect("original digest")
        );
        assert!(
            world.latest_transition_events().is_empty(),
            "no repeated Gate crossing"
        );
        assert!(restored.latest_transition_events().is_empty());
    }
}

#[test]
fn sub_millimetre_boundary_restart_commits_only_the_conflict_winner() {
    let revision = compile_road_editing_revision(conflict_road_editing_module_with_stream_count(2));
    let config = WorldConfig::new(2, 2, 64, 2, 16);
    let mut world = install_fixture(Arc::clone(&revision), config).expect("install");
    let vehicles = [0_u32, 1].map(|raw| {
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
            .expect("stream");
        let edges = revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .expect("path")
            .edges()
            .to_vec();
        let boundary = revision.traffic().lane_lengths_millimetres()[edges[0].index()];
        let route = world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route");
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, boundary, 0)
                    .with_open_entrance(),
            )
            .expect("spawn at conflict Gate")
    });
    world
        .step(TickInput::new(16))
        .expect("arbitrate boundary restart");
    let winner = world.vehicle(vehicles[1]).expect("winner");
    assert_eq!(winner.route_edge_index(), 1);
    assert_eq!(winner.progress_mm(), 0);
    assert!(winner.carry_um() > 0);
    assert!(winner.speed_mm_s() > 0);
    assert!(world.conflict_reservation(vehicles[1]).is_some());
    let loser = world.vehicle(vehicles[0]).expect("loser");
    assert_eq!(loser.route_edge_index(), 0);
    assert_eq!(loser.carry_um(), 0);
    assert_eq!(loser.speed_mm_s(), 0);
    assert!(world.conflict_reservation(vehicles[0]).is_none());
    let crossing_vehicles: Vec<_> = world
        .latest_transition_events()
        .iter()
        .filter(|event| {
            matches!(
                event.kind(),
                laneflow_runtime::TrafficTransitionKind::GateCrossed { .. }
            )
        })
        .map(|event| event.vehicle())
        .collect();
    assert_eq!(crossing_vehicles, vec![vehicles[1]]);
    restore_lfrs(
        &encode_lfrs(
            &world
                .capture_snapshot()
                .expect("capture conflict boundary tick"),
        ),
        revision,
        world.committed_source().clone(),
        config,
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        SnapshotRestoreLimits::new(1_048_576, 1_024),
    )
    .expect("restore acquired conflict authority at the sub-millimetre cursor");
}
