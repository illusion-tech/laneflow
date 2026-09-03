use super::*;

use crate::declaration::{
    CompiledFacilityBandGeometry, EdgeLength, OwnedEntityReference, TypedAstDeclaration,
};
use crate::road_editing as editing;
use crate::{
    AccessRegulationInput, AccessRuleInput, AccessRuleTargetInput, AuthoringLaneInput,
    CanonicalFrameInput, CanonicalPoint3F32Input, CompilationUnit, CompilationUnitBuilder,
    CompileLimits, CorridorElementReference, FacilityBandInput, FacilityBandReference,
    GeometryAccuracyProfile, GeometryDirectionProfile, IidmVehicleProfileInput, JunctionInput,
    JunctionReference, LaneEdgeGeometryInput, LaneEdgeInput, LaneEdgeReference, LaneGroupInput,
    LaneGroupReference, ManeuverGateInput, ManeuverGateReference, ManeuverPathInput,
    ManeuverPathReference, MovementInput, MovementReference, ParkingFacilityInput,
    ParkingFacilityReference, ParkingLaneAnchorInput, ParkingSpaceGeometryInput, ParkingSpaceInput,
    ParticipantClassInput, ParticipantClassReference, RoadCorridorInput, RoadSectionInput,
    RoadSectionReference, SignalControlInput, SignalControllerInput, SignalGroupInput,
    SignalGroupReference, SignalGroupStateInput, SignalPhaseInput, SourceModuleHeader,
    SourceModuleHeaderInput, StopLineInput, StopLineReference, SyntheticModule,
    SyntheticModuleBuilder, VehicleProfileInput, WaitingZoneInput,
};
use laneflow_static_contract::{CanonicalFrameKind, EntityKind};
use std::sync::Arc;

fn unit(modules: impl IntoIterator<Item = SyntheticModule>) -> CompilationUnit {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    for module in modules {
        builder.add_synthetic_module(module).unwrap();
    }
    builder.build().unwrap()
}

fn portable_fixture_builder(namespace: &str, document: &str) -> SyntheticModuleBuilder {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: namespace,
            source_document_key: document,
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: Some(42),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .unwrap();
    SyntheticModuleBuilder::new(header, &limits).unwrap()
}

fn canonical_iidm_profile() -> IidmVehicleProfileInput {
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

fn add_signal_control(builder: &mut SyntheticModuleBuilder) {
    let groups = [
        SignalGroupReference::local("group-entry"),
        SignalGroupReference::local("group-release"),
    ];
    let go_states = [
        SignalGroupStateInput {
            signal_group: SignalGroupReference::local("group-entry"),
            aspect: SignalAspect::Green,
        },
        SignalGroupStateInput {
            signal_group: SignalGroupReference::local("group-release"),
            aspect: SignalAspect::Red,
        },
    ];
    let clear_states = [
        SignalGroupStateInput {
            signal_group: SignalGroupReference::local("group-entry"),
            aspect: SignalAspect::Yellow,
        },
        SignalGroupStateInput {
            signal_group: SignalGroupReference::local("group-release"),
            aspect: SignalAspect::Green,
        },
    ];
    builder
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-entry",
            lane_edge: LaneEdgeReference::local("entry"),
        })
        .unwrap()
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-middle",
            lane_edge: LaneEdgeReference::local("middle"),
        })
        .unwrap()
        .add_signal_group(SignalGroupInput {
            signal_group_key: "group-entry",
        })
        .unwrap()
        .add_signal_group(SignalGroupInput {
            signal_group_key: "group-release",
        })
        .unwrap()
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-entry",
            maneuver_path: ManeuverPathReference::local("path-main"),
            transition_index: 0,
            stop_line: StopLineReference::local("stop-entry"),
            signal_control: SignalControlInput::Group(SignalGroupReference::local("group-entry")),
        })
        .unwrap()
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-release",
            maneuver_path: ManeuverPathReference::local("path-main"),
            transition_index: 1,
            stop_line: StopLineReference::local("stop-middle"),
            signal_control: SignalControlInput::Group(SignalGroupReference::local("group-release")),
        })
        .unwrap()
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: "controller-main",
            offset_ms: 1_000,
            signal_groups: &groups,
            phases: &[
                SignalPhaseInput {
                    signal_phase_key: "phase-go",
                    duration_ms: 30_000,
                    states: &go_states,
                },
                SignalPhaseInput {
                    signal_phase_key: "phase-clear",
                    duration_ms: 5_000,
                    states: &clear_states,
                },
            ],
        })
        .unwrap();
}

fn portable_fixture_full_spatial_module() -> SyntheticModule {
    let entry_points = [
        CanonicalPoint3F32Input {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        CanonicalPoint3F32Input {
            x: 10.0,
            y: 0.0,
            z: 0.0,
        },
    ];
    let middle_points = [
        CanonicalPoint3F32Input {
            x: 10.0,
            y: 0.0,
            z: 0.0,
        },
        CanonicalPoint3F32Input {
            x: 18.0,
            y: 0.0,
            z: 0.0,
        },
    ];
    let exit_points = [
        CanonicalPoint3F32Input {
            x: 18.0,
            y: 0.0,
            z: 0.0,
        },
        CanonicalPoint3F32Input {
            x: 30.0,
            y: 0.0,
            z: 0.0,
        },
    ];
    let geometries = [
        LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local("entry"),
            centerline_points: &entry_points,
        },
        LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local("middle"),
            centerline_points: &middle_points,
        },
        LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local("exit"),
            centerline_points: &exit_points,
        },
    ];
    let corridor_elements = [
        CorridorElementReference::facility_band(FacilityBandReference::local("spatial-band")),
        CorridorElementReference::road_section(RoadSectionReference::local("spatial-section")),
    ];
    let mut builder = portable_fixture_builder(
        "city/portable-full-spatial",
        "portable-full-spatial.document",
    );
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[LaneEdgeReference::local("middle")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "middle",
            length_meters: 8.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[LaneEdgeReference::local("exit")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit",
            length_meters: 12.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_facility_band(FacilityBandInput {
            facility_band_key: "spatial-band",
            kind_id: "sidewalk",
        })
        .unwrap()
        .add_lane_group(LaneGroupInput {
            lane_group_key: "through",
            road_section: RoadSectionReference::local("spatial-section"),
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "spatial-section",
            kind_id: "motorLane",
            lanes: &[AuthoringLaneInput {
                authoring_lane_key: "spatial-lane",
                edge_chain: &[
                    LaneEdgeReference::local("entry"),
                    LaneEdgeReference::local("middle"),
                    LaneEdgeReference::local("exit"),
                ],
                lane_group: Some(LaneGroupReference::local("through")),
            }],
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "spatial-corridor",
            reference_section: RoadSectionReference::local("spatial-section"),
            elements: &corridor_elements,
        })
        .unwrap()
        .add_junction(JunctionInput {
            junction_key: "junction-main",
        })
        .unwrap()
        .add_movement(MovementInput {
            movement_key: "movement-through",
            junction: JunctionReference::local("junction-main"),
            directed_entry_approach_key: "approach-westbound",
            directed_exit_approach_key: "approach-eastbound",
        })
        .unwrap()
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path-main",
            movement: MovementReference::local("movement-through"),
            entry_edge: LaneEdgeReference::local("entry"),
            internal_edges: &[LaneEdgeReference::local("middle")],
            exit_edge: LaneEdgeReference::local("exit"),
        })
        .unwrap();
    add_signal_control(&mut builder);
    builder
        .add_waiting_zone(WaitingZoneInput {
            waiting_zone_key: "waiting-main",
            maneuver_path: ManeuverPathReference::local("path-main"),
            entry_gate: ManeuverGateReference::local("gate-entry"),
            release_gate: ManeuverGateReference::local("gate-release"),
            max_occupancy: 3,
        })
        .unwrap()
        .add_parking_facility(ParkingFacilityInput {
            parking_facility_key: "area-main",
            virtual_capacity: 0,
            virtual_entries: &[],
            virtual_exits: &[],
        })
        .unwrap()
        .add_parking_space(ParkingSpaceInput {
            parking_space_key: "space-main",
            parking_facility: Some(ParkingFacilityReference::local("area-main")),
            entry: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("entry"),
                progress_meters: 4.0,
            },
            exit: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("exit"),
                progress_meters: 6.0,
            },
            geometry: ParkingSpaceGeometryInput {
                lateral_offset_meters: -3.0,
                heading_offset_radians: 0.25,
                length_meters: 5.5,
                width_meters: 2.6,
            },
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "passenger-car",
            extends: Some(ParticipantClassReference::local("road-user")),
        })
        .unwrap()
        .add_access_rule(AccessRuleInput {
            access_rule_key: "allow-passenger-cars",
            target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("entry")),
            effect: AccessEffect::Allow,
            participant_classes: &[ParticipantClassReference::local("passenger-car")],
            regulation: Some(AccessRegulationInput {
                jurisdiction: "CN-test",
                version: "2026-01",
                source: Some("portable fixture"),
            }),
            priority: -7,
        })
        .unwrap()
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "standard-car",
            participant_class: ParticipantClassReference::local("passenger-car"),
            iidm: canonical_iidm_profile(),
        })
        .unwrap()
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame-main",
            lane_edge_geometries: &geometries,
        })
        .unwrap();
    builder.finish().unwrap()
}

fn portable_fixture_full_spatial_conflict_source(
    limits: &CompileLimits,
) -> editing::OwnedRoadEditingSourceBuffer {
    const DOCUMENT_KEY: &str = "portable-full-spatial-conflict.document";
    const JUNCTION: &str = "conflict-junction";
    const ZONE: &str = "conflict-zone";
    const FRAME: &str = "conflict-frame";
    const ENTRY_A: &str = "conflict-entry-a";
    const INTERNAL_A: &str = "conflict-internal-a";
    const EXIT_A: &str = "conflict-exit-a";
    const ENTRY_B: &str = "conflict-entry-b";
    const INTERNAL_B: &str = "conflict-internal-b";
    const EXIT_B: &str = "conflict-exit-b";

    let header = editing::RoadEditingModuleHeader::try_new(
        "city/portable-full-spatial-conflict",
        DOCUMENT_KEY,
        Vec::new(),
        editing::RoadEditingProvenance::direct("LFCA full-spatial conflict fixture").unwrap(),
    )
    .unwrap();
    let mut builder = editing::RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        limits,
    )
    .unwrap();
    let edge = |key: &str| editing::LaneEdgeReference::local(key).unwrap();
    let curve = |start_x: f64, end_x: f64| {
        editing::RoadEditingCurveProgram::try_new(
            editing::RoadEditingPoint3::try_new(start_x, 0.0, 0.0).unwrap(),
            vec![editing::RoadEditingCurveSegment::line(
                editing::RoadEditingPoint3::try_new(end_x, 0.0, 0.0).unwrap(),
            )],
        )
        .unwrap()
    };
    let junction = editing::JunctionReference::local(JUNCTION).unwrap();
    let zone =
        editing::ConflictZoneReference::owner_scoped(vec![JUNCTION.to_owned()], ZONE).unwrap();

    builder
        .add_declaration(editing::RoadEditingDeclaration::CanonicalFrame(
            editing::CanonicalFrameInput::try_new(FRAME).unwrap(),
        ))
        .unwrap();
    for (suffix, edge_key, start_x, end_x) in [
        ("entry-a", ENTRY_A, 0.0, 10.0),
        ("exit-a", EXIT_A, 18.0, 30.0),
        ("entry-b", ENTRY_B, 0.0, 10.0),
        ("exit-b", EXIT_B, 18.0, 30.0),
    ] {
        let alignment_key = format!("conflict-alignment-{suffix}");
        let corridor_key = format!("conflict-corridor-{suffix}");
        let section_key = format!("conflict-section-{suffix}");
        let lane_key = format!("conflict-lane-{suffix}");
        let corridor = editing::RoadCorridorReference::local(&corridor_key).unwrap();
        let section =
            editing::RoadSectionReference::owner_scoped(vec![corridor_key.clone()], &section_key)
                .unwrap();
        let lane = editing::AuthoringLaneReference::owner_scoped(
            vec![corridor_key.clone(), section_key.clone()],
            &lane_key,
        )
        .unwrap();
        builder
            .add_alignment(
                editing::RoadAlignmentInput::try_new(
                    &alignment_key,
                    editing::CanonicalFrameReference::local(FRAME).unwrap(),
                    curve(start_x, end_x),
                )
                .unwrap(),
            )
            .unwrap()
            .add_declaration(editing::RoadEditingDeclaration::RoadCorridor(
                editing::RoadCorridorInput::try_new(
                    &corridor_key,
                    editing::RoadAlignmentReference::try_new(&alignment_key).unwrap(),
                    0.0,
                    editing::RoadEditingStationEnd::AlignmentEnd,
                    section.clone(),
                    lane.clone(),
                    vec![editing::RoadEditingCorridorElement::RoadSection(
                        section.clone(),
                    )],
                )
                .unwrap(),
            ))
            .unwrap()
            .add_declaration(editing::RoadEditingDeclaration::RoadSection(
                editing::RoadSectionInput::try_new(
                    &section_key,
                    "motorLane",
                    vec![lane.clone()],
                    corridor,
                )
                .unwrap(),
            ))
            .unwrap()
            .add_declaration(editing::RoadEditingDeclaration::AuthoringLane(
                editing::AuthoringLaneInput::try_new(
                    &lane_key,
                    edge(edge_key),
                    editing::RoadEditingLaneDirection::Forward,
                    editing::LinearWidthProfile::try_new(3.5, 3.5).unwrap(),
                    None,
                    section,
                )
                .unwrap(),
            ))
            .unwrap();
    }
    for (edge_key, explicit_geometry) in [
        (ENTRY_A, None),
        (INTERNAL_A, Some(curve(10.0, 18.0))),
        (EXIT_A, None),
        (ENTRY_B, None),
        (INTERNAL_B, Some(curve(10.0, 18.0))),
        (EXIT_B, None),
    ] {
        builder
            .add_declaration(editing::RoadEditingDeclaration::LaneEdge(
                editing::LaneEdgeInput::try_new(edge_key, 10.0, Vec::new(), explicit_geometry)
                    .unwrap(),
            ))
            .unwrap();
    }
    builder
        .add_declaration(editing::RoadEditingDeclaration::Junction(
            editing::JunctionInput::try_new(
                JUNCTION,
                vec![edge(ENTRY_A), edge(EXIT_A), edge(ENTRY_B), edge(EXIT_B)],
                vec![edge(INTERNAL_A), edge(INTERNAL_B)],
            )
            .unwrap(),
        ))
        .unwrap()
        .add_declaration(editing::RoadEditingDeclaration::ConflictZone(
            editing::ConflictZoneInput::try_new(ZONE, junction.clone()).unwrap(),
        ))
        .unwrap();
    for (suffix, entry, internal, exit) in [
        ("a", ENTRY_A, INTERNAL_A, EXIT_A),
        ("b", ENTRY_B, INTERNAL_B, EXIT_B),
    ] {
        let movement_key = format!("conflict-movement-{suffix}");
        let stream_key = format!("conflict-stream-{suffix}");
        let stop_key = format!("conflict-stop-{suffix}");
        let movement =
            editing::MovementReference::owner_scoped(vec![JUNCTION.to_owned()], &movement_key)
                .unwrap();
        let path = editing::ManeuverPathReference::owner_scoped(
            vec![JUNCTION.to_owned(), movement_key.clone()],
            "path",
        )
        .unwrap();
        let admission_gate = editing::ManeuverGateReference::owner_scoped(
            vec![JUNCTION.to_owned(), movement_key.clone(), "path".to_owned()],
            "admission",
        )
        .unwrap();
        builder
            .add_declaration(editing::RoadEditingDeclaration::Movement(
                editing::MovementInput::try_new(
                    &movement_key,
                    junction.clone(),
                    format!("approach-{suffix}-entry"),
                    format!("approach-{suffix}-exit"),
                )
                .unwrap(),
            ))
            .unwrap()
            .add_declaration(editing::RoadEditingDeclaration::ManeuverPath(
                editing::ManeuverPathInput::try_new(
                    "path",
                    movement,
                    edge(entry),
                    vec![edge(internal)],
                    edge(exit),
                )
                .unwrap(),
            ))
            .unwrap()
            .add_declaration(editing::RoadEditingDeclaration::StopLine(
                editing::StopLineInput::try_new(&stop_key, edge(entry)).unwrap(),
            ))
            .unwrap()
            .add_declaration(editing::RoadEditingDeclaration::ManeuverGate(
                editing::ManeuverGateInput::try_new(
                    "admission",
                    path.clone(),
                    0,
                    editing::StopLineReference::local(&stop_key).unwrap(),
                    editing::RoadEditingSignalControl::None,
                )
                .unwrap(),
            ))
            .unwrap()
            .add_declaration(editing::RoadEditingDeclaration::ParticipantStream(
                editing::ParticipantStreamInput::try_new(
                    stream_key,
                    junction.clone(),
                    path,
                    vec![editing::ConflictPassageInput::new(
                        zone.clone(),
                        editing::PathAnchorInput::gate(admission_gate),
                        editing::PathAnchorInput::edge_boundary(2),
                    )],
                )
                .unwrap(),
            ))
            .unwrap();
    }
    builder
        .add_conflict_zone_region(
            editing::ConflictZoneRegionInput::try_new(
                zone,
                editing::CanonicalFrameReference::local(FRAME).unwrap(),
                -1.0,
                1.0,
                vec![
                    editing::RoadEditingPoint2::try_new(9.0, -2.0).unwrap(),
                    editing::RoadEditingPoint2::try_new(19.0, -2.0).unwrap(),
                    editing::RoadEditingPoint2::try_new(19.0, 2.0).unwrap(),
                    editing::RoadEditingPoint2::try_new(9.0, 2.0).unwrap(),
                ],
            )
            .unwrap(),
        )
        .unwrap();

    editing::RoadEditingSourceWriter::new(limits)
        .write(builder.finish().unwrap())
        .unwrap()
}

pub(crate) fn full_spatial_portable_fixture_unit() -> CompilationUnit {
    let limits = CompileLimits::p100_initial_v1();
    let conflict_source = portable_fixture_full_spatial_conflict_source(&limits);
    let mut builder = CompilationUnitBuilder::new(limits);
    builder
        .add_synthetic_module(portable_fixture_full_spatial_module())
        .unwrap()
        .add_road_editing_module(
            editing::RoadEditingModuleInput::try_new(
                "portable-full-spatial-conflict.document",
                conflict_source.as_bytes(),
                None,
            )
            .unwrap(),
        )
        .unwrap();
    let mut fixture = builder.build().unwrap();
    let module = fixture
        .modules
        .iter_mut()
        .find(|module| module.descriptor().authoring_namespace_id() == "city/portable-full-spatial")
        .expect("fixture contains its spatial module");
    let namespace: Arc<str> = module.descriptor().authoring_namespace_id().into();
    for declaration in &mut module.declarations {
        let TypedAstDeclaration::FacilityBand(band) = declaration else {
            continue;
        };
        band.compiled_geometry = Some(CompiledFacilityBandGeometry {
            length: EdgeLength::try_new(10.0).unwrap(),
            canonical_frame: OwnedEntityReference::<CanonicalFrameKind>::new(
                Arc::clone(&namespace),
                Arc::from("frame-main"),
                band.header.span.clone(),
            ),
            centerline_points: [
                CanonicalPoint3F32Input {
                    x: 0.0,
                    y: -2.0,
                    z: 0.0,
                },
                CanonicalPoint3F32Input {
                    x: 10.0,
                    y: -2.0,
                    z: 0.0,
                },
            ]
            .into(),
            source_ranges: Box::new([]),
        });
    }
    fixture
}

pub(crate) fn full_spatial_portable_fixture_output() -> CompilationOutput {
    Compiler::new()
        .compile(full_spatial_portable_fixture_unit())
        .unwrap()
}

fn full_spatial_portable_fixture_provenance() -> crate::PortableEmissionProvenance {
    crate::PortableEmissionProvenance::try_new("laneflow-fixture-298-full-spatial-v1").unwrap()
}

pub(crate) fn full_spatial_portable_fixture_candidate() -> crate::PortablePublicationCandidate {
    let output = full_spatial_portable_fixture_output();
    let provenance = full_spatial_portable_fixture_provenance();
    crate::emit_portable_candidate(
        &output,
        &provenance,
        laneflow_format::FormatLimits::HARD,
        crate::PortableDiffBase::Genesis,
    )
    .unwrap()
}

pub(crate) fn full_spatial_portable_artifact_base_fixture_candidate()
-> crate::PortablePublicationCandidate {
    let output = full_spatial_portable_fixture_output();
    let provenance = full_spatial_portable_fixture_provenance();
    let genesis = full_spatial_portable_fixture_candidate();
    let base = laneflow_format::preflight_object_values(
        genesis.canonical_artifact().bytes(),
        laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap();
    crate::emit_portable_candidate(
        &output,
        &provenance,
        laneflow_format::FormatLimits::HARD,
        crate::PortableDiffBase::Artifact(base),
    )
    .unwrap()
}

const FULL_SPATIAL_EXPECTED_LFCA: &[u8] =
    include_bytes!("../../tests/fixtures/portable/lfca-full-spatial/expected.lfca");
const FULL_SPATIAL_EXPECTED_LFSM: &[u8] =
    include_bytes!("../../tests/fixtures/portable/lfca-full-spatial/expected.lfsm");
const FULL_SPATIAL_EXPECTED_LFSD: &[u8] =
    include_bytes!("../../tests/fixtures/portable/lfca-full-spatial/expected.lfsd");
const FULL_SPATIAL_NETWORK_REVISION: [u8; 32] = [
    0xdb, 0xad, 0x94, 0x3c, 0x1a, 0x9f, 0x9d, 0xd6, 0x27, 0x90, 0x01, 0x1d, 0xad, 0x06, 0x18, 0xb6,
    0x3f, 0x71, 0xd7, 0xd3, 0xaf, 0x59, 0x98, 0xda, 0x1d, 0x65, 0xbf, 0x28, 0x35, 0xe5, 0xac, 0x4d,
];

#[test]
fn dump_portable_full_spatial_when_requested() {
    if std::env::var_os("DUMP_PORTABLE").is_none() {
        return;
    }
    let candidate = full_spatial_portable_fixture_candidate();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/portable/lfca-full-spatial");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("expected.lfca"),
        candidate.canonical_artifact().bytes(),
    )
    .unwrap();
    std::fs::write(dir.join("expected.lfsm"), candidate.source_map().bytes()).unwrap();
    std::fs::write(dir.join("expected.lfsd"), candidate.semantic_diff().bytes()).unwrap();
    let revision = candidate.network_revision().into_digest().into_bytes();
    std::fs::write(
        dir.join("bindings.txt"),
        format!(
            "lfca {}\nlfsm {}\nlfsd {}\nrevision {}\n",
            candidate.canonical_artifact().object_key(),
            candidate.source_map().object_key(),
            candidate.semantic_diff().object_key(),
            revision
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
        ),
    )
    .unwrap();
}

#[test]
fn portable_full_spatial_candidate_matches_frozen_exact_bytes() {
    let candidate = full_spatial_portable_fixture_candidate();
    assert_eq!(
        candidate.canonical_artifact().bytes(),
        FULL_SPATIAL_EXPECTED_LFCA
    );
    assert_eq!(candidate.source_map().bytes(), FULL_SPATIAL_EXPECTED_LFSM);
    assert_eq!(
        candidate.semantic_diff().bytes(),
        FULL_SPATIAL_EXPECTED_LFSD
    );
    assert_eq!(
        candidate.canonical_artifact().object_key(),
        "sha256/b8209061c8a0b112b95551d0c62c5470f07f8f43e2271efe95fd775415d521d2"
    );
    assert_eq!(
        candidate.source_map().object_key(),
        "sha256/adb1f581805c9680155d2eb8e70514d49621f8812527bc1646f26b0302b5c97f"
    );
    assert_eq!(
        candidate.semantic_diff().object_key(),
        "sha256/91a8746b14e930aa7ed6c791eb7035be7be1677e6ae524e11405db576bb97ff7"
    );
    assert_eq!(
        candidate.network_revision(),
        network_revision(FULL_SPATIAL_NETWORK_REVISION)
    );

    let artifact = laneflow_format::preflight_object_values(
        FULL_SPATIAL_EXPECTED_LFCA,
        laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap()
    .registry_view();
    let entity_tables = artifact.section(2).unwrap();
    assert_eq!(entity_tables.table_count(), 24);
    assert!((0..23).all(|ordinal| entity_tables.table(ordinal).unwrap().row_count() > 0));
    assert_eq!(entity_tables.table(20).unwrap().row_count(), 1);
    assert_eq!(entity_tables.table(22).unwrap().row_count(), 2);
    assert_eq!(entity_tables.table(23).unwrap().row_count(), 0);
    let relation_tables = artifact.section(3).unwrap();
    assert_eq!(relation_tables.table_count(), 5);
    assert!(relation_tables.table(0).unwrap().row_count() > 0);
    assert!((1..5).all(|ordinal| relation_tables.table(ordinal).unwrap().row_count() == 0));
    let spatial_tables = artifact.section(4).unwrap();
    assert!(spatial_tables.table(1).unwrap().row_count() > 0);
    assert!(spatial_tables.table(2).unwrap().row_count() > 0);
    assert_eq!(spatial_tables.table(3).unwrap().row_count(), 1);

    let semantic_diff = laneflow_format::preflight_object_values(
        FULL_SPATIAL_EXPECTED_LFSD,
        laneflow_static_contract::PortableObjectKind::SemanticDiff,
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap()
    .registry_view();
    let geometry_changes = semantic_diff.section(3).unwrap().table(0).unwrap();
    assert!((0..geometry_changes.row_count()).any(|ordinal| {
        matches!(
            geometry_changes
                .row(ordinal)
                .unwrap()
                .field_by_tag(2)
                .unwrap()
                .value()
                .unwrap(),
            laneflow_format::RegistryCheckedFieldValue::U16(kind)
                if kind == EntityKind::ConflictZone.code()
        )
    }));
}

// 后续向量放在子模块中，使本文件的语义输入构造保持原位稳定。
mod ci_evidence;
mod lfca_variants;
mod lfsd_change_set;
mod lfsd_migration;
mod lfsd_noop;
mod portable_matrix_tests;
mod portable_security_tests;

fn sha256_digest(bytes: [u8; 32]) -> laneflow_static_contract::Sha256Digest {
    laneflow_static_contract::Sha256Digest::from_bytes(bytes)
}

fn network_revision(bytes: [u8; 32]) -> laneflow_static_contract::NetworkRevisionId {
    laneflow_static_contract::NetworkRevisionId::from_digest(sha256_digest(bytes))
}

fn exact_byte_length(value: u64) -> laneflow_static_contract::ExactByteLength {
    laneflow_static_contract::ExactByteLength::new(value)
}

pub(crate) fn refresh_portable_chunk_digest_containing(
    bytes: &mut [u8],
    kind: laneflow_static_contract::PortableObjectKind,
    absolute_offset: usize,
) {
    use laneflow_static_contract::{
        CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH, OBJECT_PREAMBLE_BYTE_LENGTH,
        SECTION_DIRECTORY_ENTRY_BYTE_LENGTH, TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH,
    };
    use sha2::Digest as _;

    for section_ordinal in 0..kind.section_count() {
        let section_entry = usize::from(OBJECT_PREAMBLE_BYTE_LENGTH)
            + usize::try_from(section_ordinal).unwrap()
                * usize::try_from(SECTION_DIRECTORY_ENTRY_BYTE_LENGTH).unwrap();
        let section_start = usize::try_from(u64::from_le_bytes(
            bytes[section_entry + 8..section_entry + 16]
                .try_into()
                .unwrap(),
        ))
        .unwrap();
        let section_length = usize::try_from(u64::from_le_bytes(
            bytes[section_entry + 16..section_entry + 24]
                .try_into()
                .unwrap(),
        ))
        .unwrap();
        if absolute_offset < section_start || absolute_offset >= section_start + section_length {
            continue;
        }

        let chunk_count =
            u32::from_le_bytes(bytes[section_start..section_start + 4].try_into().unwrap());
        for chunk_ordinal in 0..chunk_count {
            let chunk_entry = section_start
                + usize::try_from(CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH).unwrap()
                + usize::try_from(chunk_ordinal).unwrap()
                    * usize::try_from(TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH).unwrap();
            let chunk_offset = usize::try_from(u64::from_le_bytes(
                bytes[chunk_entry + 24..chunk_entry + 32]
                    .try_into()
                    .unwrap(),
            ))
            .unwrap();
            let chunk_length = usize::try_from(u64::from_le_bytes(
                bytes[chunk_entry + 32..chunk_entry + 40]
                    .try_into()
                    .unwrap(),
            ))
            .unwrap();
            let chunk_start = section_start + chunk_offset;
            let chunk_end = chunk_start + chunk_length;
            if (chunk_start..chunk_end).contains(&absolute_offset) {
                let digest = sha2::Sha256::digest(&bytes[chunk_start..chunk_end]);
                bytes[chunk_entry + 40..chunk_entry + 72].copy_from_slice(&digest);
                return;
            }
        }
    }
    panic!("offset {absolute_offset} is not inside a physical table chunk");
}

#[test]
fn conflict_reverse_closure_scratch_is_exact_bounded_and_not_retained() {
    let mut unit = full_spatial_portable_fixture_unit();
    let hir = crate::hir::build_hir(&unit).expect("full-spatial HIR");
    let mut mir = crate::mir::lower_to_mir(&unit, &hir).expect("full-spatial MIR");
    let plan = crate::lir::LirFreezePlan::analyze(&unit, &mir);

    assert_eq!(plan.conflict.zones, 1);
    assert_eq!(plan.conflict.streams, 2);
    assert_eq!(plan.conflict.passages, 2);
    assert_eq!(plan.conflict.zone_streams, 2);
    assert_eq!(plan.conflict.max_zone_streams, 2);

    let zone_ranges = mir
        .conflict_zones
        .iter()
        .map(|zone| zone.participant_streams)
        .collect::<Vec<_>>();
    let zone_streams = core::mem::take(&mut mir.conflict_zone_streams);
    for zone in &mut mir.conflict_zones {
        zone.participant_streams = crate::arena::TableRange::empty();
    }
    let without_reverse_closure = crate::lir::LirFreezePlan::analyze(&unit, &mir);
    let expected_reverse_membership_bytes = u64::try_from(
        core::mem::size_of::<laneflow_static_contract::ParticipantStreamOrdinal>() * 4,
    )
    .expect("small fixed test size");
    assert_eq!(
        plan.stage_scratch_bytes - without_reverse_closure.stage_scratch_bytes,
        expected_reverse_membership_bytes
    );
    assert_eq!(
        plan.controlled_live_bytes - without_reverse_closure.controlled_live_bytes,
        expected_reverse_membership_bytes
    );
    assert_eq!(
        plan.output_owned_bytes,
        without_reverse_closure.output_owned_bytes
    );

    let conflict_zones = core::mem::take(&mut mir.conflict_zones);
    let without_conflict_zones = crate::lir::LirFreezePlan::analyze(&unit, &mir);
    let expected_zone_scratch_bytes = u64::try_from(
        core::mem::size_of::<u32>() * 2
            + core::mem::size_of::<Vec<laneflow_static_contract::ParticipantStreamOrdinal>>(),
    )
    .expect("small fixed test size");
    assert_eq!(
        without_reverse_closure.stage_scratch_bytes - without_conflict_zones.stage_scratch_bytes,
        expected_zone_scratch_bytes
    );
    mir.conflict_zones = conflict_zones;
    mir.conflict_zone_streams = zone_streams;
    for (zone, range) in mir.conflict_zones.iter_mut().zip(zone_ranges) {
        zone.participant_streams = range;
    }

    let exact_scratch = u32::try_from(plan.stage_scratch_bytes).expect("fixture scratch fits u32");
    unit.limits = CompileLimits::p100_initial_v1().with_test_lir_limits(
        u32::MAX,
        exact_scratch,
        u32::MAX,
        u32::MAX,
    );
    let frozen = crate::lir::freeze_lir(&unit, &mir)
        .expect("exact LIR scratch boundary")
        .lir;
    assert_eq!(frozen.controlled_live_bytes, plan.output_owned_bytes);
    assert_eq!(
        frozen.peak_controlled_live_bytes,
        plan.controlled_live_bytes
    );

    unit.limits = CompileLimits::p100_initial_v1().with_test_lir_limits(
        u32::MAX,
        exact_scratch - 1,
        u32::MAX,
        u32::MAX,
    );
    let failure = match crate::lir::freeze_lir(&unit, &mir) {
        Ok(_) => panic!("one byte below LIR scratch must fail closed"),
        Err(diagnostics) => diagnostics,
    };
    assert!(failure.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.payload(),
        crate::DiagnosticPayload::CompileLimitExceeded {
            dimension: crate::CompileLimitDimension::StageScratchBytes,
            limit,
            observed,
        } if *limit == plan.stage_scratch_bytes - 1
            && *observed == plan.stage_scratch_bytes
    )));
}
