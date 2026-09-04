#[path = "support/policy.rs"]
mod test_policy;

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
use laneflow_static_contract::{
    AccessEffect, ConflictZoneOrdinal, EntityKind, LaneEdgeId, ParkingFacilityOrdinal,
    ParkingSpaceOrdinal, ParticipantStreamOrdinal, PortableObjectKind,
    SEMANTIC_DIFF_FORMAT_VERSION, Sha256Digest, SignalAspect, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    ConflictPathAnchor, SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision,
    SpatialBuildOption, build_shared_network_revision,
};

use laneflow_runtime_snapshot_wire::generated::lane_flow::runtime_snapshot::v5 as snapshot_wire;

fn install_fixture(
    revision: std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
    config: laneflow_runtime::WorldConfig,
) -> Result<laneflow_runtime::TrafficWorld, laneflow_runtime::InstallError> {
    laneflow_runtime::TrafficWorld::install(
        Arc::clone(&revision),
        config,
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
    let limits = CompileLimits::p100_initial_v1();
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
    let limits = CompileLimits::p100_initial_v2();
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
    let output = compile_road_editing_output(module);
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
    )
}

fn conflict_road_editing_module_with_shape_and_speed(
    stream_count: usize,
    terminal_clearance: bool,
    include_conflict: bool,
    multiplicity: bool,
    desired_speed_meters_per_second: f64,
) -> lfre::RoadEditingSourceModule {
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
    for (edge, start, end) in [
        ("east-entry", (-13.0, 0.0), (0.0, 0.0)),
        ("west-exit", (13.0, 0.0), (26.0, 0.0)),
        ("north-entry", (0.0, -13.0), (0.0, 0.0)),
        ("south-exit", (0.0, 13.0), (0.0, 26.0)),
    ] {
        let geometry = if multiplicity && edge == "west-exit" {
            road_editing_loop()
        } else {
            road_editing_line(start, end)
        };
        let successors = if multiplicity && edge == "west-exit" {
            vec![lfre::LaneEdgeReference::local("east-entry").expect("loop successor")]
        } else {
            Vec::new()
        };
        add_road_editing_approach(&mut module, edge, geometry, successors);
    }
    for (edge, start, end) in [
        ("east-internal", (0.0, 0.0), (13.0, 0.0)),
        ("north-internal", (0.0, 0.0), (0.0, 13.0)),
    ] {
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
    module
        .add_declaration(lfre::RoadEditingDeclaration::Junction(
            lfre::JunctionInput::try_new(
                "crossing",
                ["east-entry", "west-exit", "north-entry", "south-exit"]
                    .into_iter()
                    .map(|key| lfre::LaneEdgeReference::local(key).expect("approach edge"))
                    .collect(),
                ["east-internal", "north-internal"]
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
        if multiplicity {
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
                lfre::PathAnchorInput::gate(admission_gate),
                lfre::PathAnchorInput::edge_boundary(3),
            )
        } else {
            (
                lfre::PathAnchorInput::interior(1, entry_progress).expect("entry anchor"),
                lfre::PathAnchorInput::interior(1, exit_progress).expect("exit anchor"),
            )
        };
        module
            .add_declaration(lfre::RoadEditingDeclaration::Movement(
                lfre::MovementInput::try_new(movement_key, junction.clone(), entry_edge, exit_edge)
                    .expect("movement"),
            ))
            .expect("add movement")
            .add_declaration(lfre::RoadEditingDeclaration::ManeuverPath(
                lfre::ManeuverPathInput::try_new(
                    path_key,
                    movement,
                    lfre::LaneEdgeReference::local(entry_edge).expect("entry edge"),
                    vec![lfre::LaneEdgeReference::local(internal_edge).expect("internal edge")],
                    lfre::LaneEdgeReference::local(exit_edge).expect("exit edge"),
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
                    lfre::RoadEditingSignalControl::None,
                )
                .expect("maneuver gate"),
            ))
            .expect("add maneuver gate");
        if include_conflict && !multiplicity && stream_index < stream_count {
            module
                .add_declaration(lfre::RoadEditingDeclaration::ParticipantStream(
                    lfre::ParticipantStreamInput::try_new(
                        stream_key,
                        junction.clone(),
                        path,
                        vec![lfre::ConflictPassageInput::new(
                            zone.clone(),
                            entry_anchor,
                            exit_anchor,
                        )],
                    )
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
    module
        .add_declaration(lfre::RoadEditingDeclaration::ParkingFacility(
            lfre::ParkingFacilityInput::try_new("parking").expect("parking facility"),
        ))
        .expect("add parking facility")
        .add_declaration(lfre::RoadEditingDeclaration::ParkingSpace(
            lfre::ParkingSpaceInput::try_new(
                "space",
                lfre::ParkingLaneAnchor::try_new(
                    lfre::LaneEdgeReference::local("west-exit").expect("parking entry edge"),
                    12.0,
                )
                .expect("parking entry"),
                lfre::ParkingLaneAnchor::try_new(
                    lfre::LaneEdgeReference::local("east-internal").expect("parking exit edge"),
                    10.5,
                )
                .expect("parking exit"),
                lfre::ParkingSpaceGeometry::try_new(1.5, 0.0, 5.0, 2.5).expect("parking geometry"),
            )
            .expect("parking space")
            .with_parking_facility(
                lfre::ParkingFacilityReference::local("parking")
                    .expect("parking facility reference"),
            ),
        ))
        .expect("add parking space");
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
                participant,
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
    let policy_gates = [
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
            laneflow_compiler::GateInterpretation::Uncontrolled,
            laneflow_compiler::GateProhibition::None,
            vec![],
        )
        .unwrap()
    })
    .collect();
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
    let policy_streams = stream_keys
        .iter()
        .map(|key| {
            lfre::PolicyStreamRuleInput::try_new(
                *key,
                lfre::ParticipantStreamReference::owner_scoped(vec!["crossing".into()], *key)
                    .unwrap(),
                None,
                0,
                vec![],
                None,
                vec![],
            )
            .unwrap()
        })
        .collect();
    module
        .add_declaration(lfre::RoadEditingDeclaration::RightOfWayPolicySet(
            lfre::RightOfWayPolicySetInput::try_new(
                "conflict-policy",
                laneflow_compiler::RegulationIdentity::try_new("engineering", "fixture-1")
                    .unwrap()
                    .with_source("repository:runtime-fixture-1")
                    .unwrap(),
                vec![],
                vec![],
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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 1, 100)).expect("install");
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

#[test]
fn conflict_routes_charge_independent_capacity_and_gate_active_spawn() {
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
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 0, 1, 100))
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

    let mut world = install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 1, 1, 100))
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

    let spawn_cursor = world.command_cursor();
    let error = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            0,
        ))
        .unwrap_err();
    let SpawnError::ConflictRuntimeUnavailable(unavailable) = error else {
        panic!("route start must be rejected by 3A: {error:?}");
    };
    assert_eq!(unavailable.route(), route);
    assert_eq!(unavailable.stream(), ParticipantStreamOrdinal::from_raw(0));
    assert_eq!(unavailable.passage_local_index(), 0);
    assert_eq!(unavailable.zone(), ConflictZoneOrdinal::from_raw(0));
    assert_eq!(world.command_cursor(), spawn_cursor);
    assert!(world.live_vehicles().is_empty());

    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            1,
            10_501,
            0,
        ))
        .expect("rear exactly at 6,001 mm clearance is allowed");
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

    let final_stream = streams
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

    let mut too_small =
        install_fixture(Arc::clone(&revision), WorldConfig::new(2, 2, 12, 3, 1, 100))
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

    let mut world = install_fixture(Arc::clone(&revision), WorldConfig::new(2, 2, 12, 4, 1, 100))
        .expect("install exact multiplicity fixture");
    let route = world
        .register_route(RouteRegisterInput::new(route_edges.clone()))
        .expect("four distinct passage occurrences");
    let error = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            2,
            2_000,
            0,
        ))
        .unwrap_err();
    let SpawnError::ConflictRuntimeUnavailable(unavailable) = error else {
        panic!("rear clears the last entry but not the maximum clearance: {error:?}");
    };
    assert_eq!(unavailable.stream(), final_stream);
    assert_eq!(unavailable.passage_local_index(), 1);
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            2,
            2_500,
            0,
        ))
        .expect("rear exactly at maximum clearance");

    let mut repeated_edges = route_edges.clone();
    repeated_edges.extend_from_slice(&route_edges);
    let mut repeated_too_small =
        install_fixture(Arc::clone(&revision), WorldConfig::new(0, 1, 6, 7, 1, 100))
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
    let mut repeated = install_fixture(revision, WorldConfig::new(0, 1, 6, 8, 1, 100))
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
    let mut world = install_fixture(revision, WorldConfig::new(0, 2, 6, 1, 1, 100))
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
            1,
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
    let config = WorldConfig::new(4, 4, 64, 1, 1, 100);
    let mut world = install_fixture(Arc::clone(&base_revision), config).expect("install base");
    let route = world
        .register_route(RouteRegisterInput::new(route_edges.clone()))
        .expect("register base conflict route");
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            1,
            10_501,
            0,
        ))
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
    let CutoverError::ConflictRuntimeUnavailable(unavailable) = error else {
        panic!("unexpected cutover error: {error:?}");
    };
    assert_eq!(unavailable.route(), route);
    assert_eq!(unavailable.stream(), ParticipantStreamOrdinal::from_raw(0));
    assert_eq!(unavailable.passage_local_index(), 0);
    assert_eq!(unavailable.zone(), ConflictZoneOrdinal::from_raw(0));
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
    assert!(matches!(retry, CutoverError::ConflictRuntimeUnavailable(_)));
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
        WorldConfig::new(4, 4, 64, 1, 1, 100),
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
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            1,
            10_501,
            0,
        ))
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
        WorldConfig::new(4, 4, 64, 4, 1, 100),
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
        WorldConfig::new(4, 4, 64, 2, 1, 100),
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
    let config = WorldConfig::new(4, 4, 64, 1, 1, 100);
    let mut world = install_fixture(Arc::clone(&revision), config).expect("install");
    let route = world
        .register_route(RouteRegisterInput::new(route_edges))
        .expect("register conflict route");
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            1,
            10_501,
            1,
        ))
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
            SnapshotRestoreLimits::new(1_048_576, 1_024),
        ),
        Err(SnapshotRestoreError::Vehicle {
            error: SpawnError::ConflictRuntimeUnavailable(_),
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
        SnapshotRestoreLimits::new(1_048_576, 1_024),
    )
    .expect("rear exactly at clearance restores");

    let smaller_target = WorldConfig::new(4, 4, 64, 0, 1, 100);
    assert_eq!(
        restore_lfrs(
            &bytes,
            Arc::clone(&revision),
            source.clone(),
            smaller_target,
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
    let config = WorldConfig::new(4, 4, 64, 1, 1, 100);
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
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 1, 1, 100))
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
        Err(ParkingError::ConflictRuntimeUnavailable(_))
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
    let mut world = install_fixture(terminal_revision, WorldConfig::new(4, 4, 64, 1, 1, 100))
        .expect("install terminal world");
    let old_route = world
        .register_route(RouteRegisterInput::new(vec![exit_edge]))
        .expect("register non-conflict suffix");
    let new_route = world
        .register_route(RouteRegisterInput::new(terminal_route_edges))
        .expect("register terminal-conflict route");
    let active = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            old_route,
            0,
            12_000,
            0,
        ))
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
        Err(ParkingError::ConflictRuntimeUnavailable(_))
    ));
    assert_eq!(world.vehicle(active), active_state);
    assert_eq!(world.parking_binding(active), active_binding);
    assert_eq!(world.command_cursor(), rebind_cursor);

    world.despawn_vehicle(active).expect("release reservation");
    let completed = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            old_route,
            0,
            exit_length,
            0,
        ))
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
            ),
        ),
        Err(ReplaceError::ConflictRuntimeUnavailable(_))
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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 1, 100)).expect("install");
    let route = register_named(&mut world, &["stem", "tail"]);
    assert_eq!(
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                0,
                0,
            ))
            .unwrap_err(),
        SpawnError::AccessDenied
    );
    assert!(world.committed_pose_sources().as_slice().is_empty());
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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 1, 100)).expect("install");
    let route = register_named(&mut world, &["edge"]);
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            4_000,
            0,
        ))
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
        install_fixture(revision, WorldConfig::new(16, 8, 1_024, 1_024, 1, 100)).expect("install");
    let route = register_named(&mut world, &["edge"]);
    let profile = VehicleProfileOrdinal::from_raw(0);
    let facility = ParkingFacilityOrdinal::from_raw(0);
    let space = ParkingSpaceOrdinal::from_raw(0);
    let spawn = |world: &mut TrafficWorld, progress_mm| {
        world
            .spawn_vehicle(VehicleSpawnInput::new(profile, route, 0, progress_mm, 0))
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
        WorldConfig::new(4, 4, 1_024, 1_024, 1, 100),
    )
    .expect("install completed fixture");
    let completed_route = register_named(&mut completed_world, &["edge"]);
    let completed = completed_world
        .spawn_vehicle(VehicleSpawnInput::new(
            profile,
            completed_route,
            0,
            100_000,
            0,
        ))
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
        install_fixture(revision, WorldConfig::new(4, 4, 1_024, 1_024, 1, 100)).expect("install");
    let route = register_named(&mut world, &["edge"]);
    let facility = ParkingFacilityOrdinal::from_raw(0);
    let target = ParkingTarget::VirtualPool(facility);
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            0,
        ))
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
    assert!(world.committed_pose_sources().as_slice().is_empty());
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
fn virtual_reserved_and_occupied_bindings_round_trip_in_snapshot_v5() {
    let revision = compile_virtual_parking_revision(2);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
    )
    .expect("install");
    let route = register_named(&mut world, &["edge"]);
    let profile = VehicleProfileOrdinal::from_raw(0);
    let facility = ParkingFacilityOrdinal::from_raw(0);
    let reserved = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, route, 0, 0, 0))
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

    assert_eq!(laneflow_runtime::SNAPSHOT_FORMAT_VERSION, 5);
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
            .as_slice()
            .iter()
            .any(|(vehicle, _)| *vehicle == restored_occupied)
    );
    let _ = (reserved, occupied);
}

#[test]
fn leave_failures_are_atomic_and_follow_the_one_millimetre_emergency_boundary() {
    let (mut overlap_world, route, facility, parked) = parked_virtual_world();
    let before_state = overlap_world.vehicle(parked);
    let before_binding = overlap_world.parking_binding(parked);
    let before_counts = overlap_world.parking_facility_counts(facility);
    let before_pose = overlap_world.committed_pose_sources();
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
    assert_eq!(overlap_world.committed_pose_sources(), before_pose);
    assert_eq!(overlap_world.command_cursor(), before_cursor);
    assert_eq!(overlap_world.observation_state_sequence(), before_sequence);

    let blocker = overlap_world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            70_000,
            0,
        ))
        .expect("physical blocker");
    let before_state = overlap_world.vehicle(parked);
    let before_binding = overlap_world.parking_binding(parked);
    let before_counts = overlap_world.parking_facility_counts(facility);
    let before_pose = overlap_world.committed_pose_sources();
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
    assert_eq!(overlap_world.committed_pose_sources(), before_pose);
    assert_eq!(overlap_world.command_cursor(), before_cursor);

    let (mut rejected_world, route, facility, parked) = parked_virtual_world();
    let one_mm_tolerance_follower = rejected_world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            63_499,
            1,
        ))
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
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            63_498,
            1,
        ))
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
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            74_500,
            0,
        ))
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
        WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
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
        .spawn_vehicle(VehicleSpawnInput::new(profile, cross_route, 0, 9_000, 0))
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

    let mut repeated_world =
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 1, 100))
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
        .spawn_vehicle(VehicleSpawnInput::new(profile, repeated_route, 0, 2_000, 0))
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
        install_fixture(revision, WorldConfig::new(8, 8, 1_024, 1_024, 1, 100)).expect("install");
    let old_route = register_named(&mut world, &["left", "current", "tail"]);
    let new_route = register_named(&mut world, &["right", "current", "tail"]);
    let profile = VehicleProfileOrdinal::from_raw(0);
    let facility = ParkingFacilityOrdinal::from_raw(0);
    let crossing = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, old_route, 1, 2_000, 0))
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
        .spawn_vehicle(VehicleSpawnInput::new(profile, old_route, 1, 5_000, 0))
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
}

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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 1, 100)).expect("install");
    let leader_route = world
        .register_route(RouteRegisterInput::new(vec![stem, left]))
        .expect("left route");
    let follower_route = world
        .register_route(RouteRegisterInput::new(vec![stem, right]))
        .expect("right route");
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            leader_route,
            1,
            500,
            0,
        ))
        .expect("leader on left, tail on stem");
    let follower = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            follower_route,
            0,
            5_000,
            10_000,
        ))
        .expect("follower on stem");
    world.step(TickInput::new(100)).expect("step");
    let PoseSource::Lane { progress_mm, .. } = world
        .committed_pose_sources()
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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 1, 1_000)).expect("install");
    let route = register_named(&mut world, &["edge"]);
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            0,
        ))
        .expect("spawn");
    for _ in 0..20 {
        world.step(TickInput::new(1_000)).expect("step");
    }
    let PoseSource::Lane { progress_mm, .. } = world.committed_pose_sources().as_slice()[0].1
    else {
        panic!("lane pose");
    };
    assert!(
        progress_mm <= 200_000,
        "travel must not exceed speed-limit envelope, progress={progress_mm}"
    );
}

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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 1, 1_000)).expect("install");
    let route = register_named(&mut world, &["fast", "slow"]);
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            18_000,
            10_000,
        ))
        .expect("spawn near fast/slow boundary");
    world.step(TickInput::new(1_000)).expect("approach/cross");
    let PoseSource::Lane {
        edge: after_first,
        progress_mm: first_progress,
    } = world
        .committed_pose_sources()
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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 1, 100)).expect("install");
    let route = register_named(&mut world, &["a", "b"]);
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            19_600,
            10_000,
        ))
        .expect("spawn near equal-limit boundary");
    world.step(TickInput::new(100)).expect("step");
    let PoseSource::Lane { edge, progress_mm } = world
        .committed_pose_sources()
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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 1, 1_000)).expect("install");
    let route = register_named(&mut world, &["fast", "slower"]);
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            9_000,
            10_000,
        ))
        .expect("spawn 1 m before a 10→8 drop");
    world.step(TickInput::new(1_000)).expect("step");
    let PoseSource::Lane { edge, progress_mm } = world
        .committed_pose_sources()
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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 1, 1_000)).expect("install");
    let route = register_named(&mut world, &["posted-fast", "mid"]);
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            9_000,
            2_000,
        ))
        .expect("spawn already slower than the 5 m/s next edge");
    world.step(TickInput::new(1_000)).expect("step");
    let PoseSource::Lane { edge, progress_mm } = world
        .committed_pose_sources()
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

fn add_signalized_corridor(module: &mut SyntheticModuleBuilder, phase_ms: u64) {
    let groups = [SignalGroupReference::local("group-entry")];
    let go_states = [SignalGroupStateInput {
        signal_group: SignalGroupReference::local("group-entry"),
        aspect: SignalAspect::Green,
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
        add_signalized_corridor(module, 8);
    });
    assert_eq!(
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 1, 16))
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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 1, 4)).expect("install");
    let route = register_named(&mut world, &["first", "second"]);
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            9_999,
            3_141,
        ))
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
