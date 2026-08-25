use super::*;

use crate::declaration::{
    CompiledFacilityBandGeometry, EdgeLength, OwnedEntityReference, TypedAstDeclaration,
};
use crate::{
    AccessRegulationInput, AccessRuleInput, AccessRuleTargetInput, AuthoringLaneInput,
    CanonicalFrameInput, CanonicalPoint3F32Input, CompilationUnit, CompilationUnitBuilder,
    CompileLimits, CorridorElementReference, FacilityBandInput, FacilityBandReference,
    IidmVehicleProfileInput, JunctionInput, JunctionReference, LaneEdgeGeometryInput,
    LaneEdgeInput, LaneEdgeReference, LaneGroupInput, LaneGroupReference, ManeuverGateInput,
    ManeuverGateReference, ManeuverPathInput, ManeuverPathReference, MovementInput,
    MovementReference, ParkingAreaInput, ParkingAreaReference, ParkingLaneAnchorInput,
    ParkingSpaceGeometryInput, ParkingSpaceInput, ParticipantClassInput, ParticipantClassReference,
    RoadCorridorInput, RoadSectionInput, RoadSectionReference, SignalControlInput,
    SignalControllerInput, SignalGroupInput, SignalGroupReference, SignalGroupStateInput,
    SignalPhaseInput, SourceModuleHeader, SourceModuleHeaderInput, StaticRouteInput, StopLineInput,
    StopLineReference, SyntheticModule, SyntheticModuleBuilder, VehicleProfileInput,
    WaitingZoneInput,
};
use laneflow_static_contract::CanonicalFrameKind;
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
        .add_parking_area(ParkingAreaInput {
            parking_area_key: "area-main",
        })
        .unwrap()
        .add_parking_space(ParkingSpaceInput {
            parking_space_key: "space-main",
            parking_area: Some(ParkingAreaReference::local("area-main")),
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
        .add_static_route(StaticRouteInput {
            static_route_key: "route-main",
            edge_sequence: &[
                LaneEdgeReference::local("entry"),
                LaneEdgeReference::local("middle"),
                LaneEdgeReference::local("exit"),
            ],
        })
        .unwrap()
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame-main",
            lane_edge_geometries: &geometries,
        })
        .unwrap();
    builder.finish().unwrap()
}

fn full_spatial_portable_fixture_unit() -> CompilationUnit {
    let mut fixture = unit([portable_fixture_full_spatial_module()]);
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

fn full_spatial_portable_fixture_output() -> CompilationOutput {
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
    include_bytes!("../../tests/fixtures/portable-v2/lfca-v2-full-spatial/expected.lfca");
const FULL_SPATIAL_EXPECTED_LFSM: &[u8] =
    include_bytes!("../../tests/fixtures/portable-v2/lfca-v2-full-spatial/expected.lfsm");
const FULL_SPATIAL_EXPECTED_LFSD: &[u8] =
    include_bytes!("../../tests/fixtures/portable-v2/lfca-v2-full-spatial/expected.lfsd");
const FULL_SPATIAL_NETWORK_REVISION: [u8; 32] = [
    0x74, 0x12, 0x3d, 0x7d, 0x3b, 0x79, 0x37, 0x7b, 0xa3, 0xee, 0x5b, 0x9e, 0xbf, 0xcd, 0x08, 0xb4,
    0x12, 0x00, 0x2a, 0xce, 0x17, 0x4d, 0x2e, 0xa7, 0x1a, 0xe5, 0x13, 0x0e, 0x7d, 0xc5, 0xee, 0x54,
];

#[test]
fn dump_portable_full_spatial_when_requested() {
    if std::env::var_os("DUMP_PORTABLE").is_none() {
        return;
    }
    let candidate = full_spatial_portable_fixture_candidate();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/portable-v2/lfca-v2-full-spatial");
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
        "sha256/7562dd4a2794216709c019116cd856c46f3c1f1152af47c98755a4af821e40dc"
    );
    assert_eq!(
        candidate.source_map().object_key(),
        "sha256/cd6e7bff62c6ab99ff16ce400f97ab27031a2bc97acd51650819a1f90a0aad93"
    );
    assert_eq!(
        candidate.semantic_diff().object_key(),
        "sha256/318201644f17b455524e9a8a434b0503b3103f52026c23b671d2ab83693125a1"
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
    assert_eq!(entity_tables.table_count(), 22);
    assert!((0..22).all(|ordinal| entity_tables.table(ordinal).unwrap().row_count() > 0));
    let relation_tables = artifact.section(3).unwrap();
    assert_eq!(relation_tables.table_count(), 5);
    assert!((0..5).all(|ordinal| relation_tables.table(ordinal).unwrap().row_count() > 0));
    let spatial_tables = artifact.section(4).unwrap();
    assert!(spatial_tables.table(1).unwrap().row_count() > 0);
    assert!(spatial_tables.table(2).unwrap().row_count() > 0);
}

// 后续向量放在子模块中，使本文件的语义输入构造保持原位稳定。
mod ci_evidence;
mod lfca_variants;
mod lfsd_change_set;
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
