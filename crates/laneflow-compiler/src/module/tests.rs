use std::sync::Arc;

use crate::declaration::TypedAstDeclaration;
use crate::*;

use super::*;
use crate::{DiagnosticCode, DiagnosticPayload, LaneEdgeReference, SourceModuleHeaderInput};

fn header(namespace: &str, document: &str) -> SourceModuleHeader {
    SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: namespace,
            source_document_key: document,
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: Some(42),
            provenance: "repository:laneflow",
        },
        &CompileLimits::p100_initial_v1(),
    )
    .unwrap()
}

fn module(namespace: &str, imports: &[&str]) -> SyntheticModule {
    module_with_document(namespace, namespace, imports)
}

fn module_with_document(namespace: &str, document: &str, imports: &[&str]) -> SyntheticModule {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(header(namespace, document), &limits).unwrap();
    for import in imports {
        builder.add_import(import).unwrap();
    }
    builder.finish().unwrap()
}

fn signal_module_with_document(namespace: &str, document: &str) -> SyntheticModule {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(header(namespace, document), &limits).unwrap();
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
        .unwrap()
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
            signal_group_key: "group-main",
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
            signal_control: SignalControlInput::Group(SignalGroupReference::local("group-main")),
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
            offset_ms: 0,
            signal_groups: &[
                SignalGroupReference::local("group-release"),
                SignalGroupReference::local("group-main"),
            ],
            phases: &[SignalPhaseInput {
                signal_phase_key: "phase-main",
                duration_ms: 30_000,
                states: &[
                    SignalGroupStateInput {
                        signal_group: SignalGroupReference::local("group-main"),
                        aspect: SignalAspect::Green,
                    },
                    SignalGroupStateInput {
                        signal_group: SignalGroupReference::local("group-release"),
                        aspect: SignalAspect::Red,
                    },
                ],
            }],
        })
        .unwrap()
        .add_waiting_zone(WaitingZoneInput {
            waiting_zone_key: "waiting-main",
            maneuver_path: ManeuverPathReference::local("path-main"),
            entry_gate: ManeuverGateReference::local("gate-entry"),
            release_gate: ManeuverGateReference::local("gate-release"),
            max_occupancy: 2,
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "cross-a",
            length_meters: 14.0,
            speed_limit_meters_per_second: 9.0,
            successors: &[LaneEdgeReference::local("cross-b")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "cross-b",
            length_meters: 16.0,
            speed_limit_meters_per_second: 9.0,
            successors: &[],
        })
        .unwrap()
        .add_facility_band(FacilityBandInput {
            facility_band_key: "sidewalk-left",
            kind_id: "sidewalk",
        })
        .unwrap()
        .add_lane_group(LaneGroupInput {
            lane_group_key: "through",
            road_section: RoadSectionReference::local("carriageway"),
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "carriageway",
            kind_id: "motorLane",
            lanes: &[AuthoringLaneInput {
                authoring_lane_key: "lane-main",
                edge_chain: &[
                    LaneEdgeReference::local("cross-a"),
                    LaneEdgeReference::local("cross-b"),
                ],
                lane_group: Some(LaneGroupReference::local("through")),
            }],
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "main-road",
            reference_section: RoadSectionReference::local("carriageway"),
            elements: &[
                CorridorElementReference::facility_band(FacilityBandReference::local(
                    "sidewalk-left",
                )),
                CorridorElementReference::road_section(RoadSectionReference::local("carriageway")),
            ],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "parking-entry",
            length_meters: 20.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "parking-exit",
            length_meters: 20.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[],
        })
        .unwrap()
        .add_parking_area(ParkingAreaInput {
            parking_area_key: "parking-main",
        })
        .unwrap()
        .add_parking_space(ParkingSpaceInput {
            parking_space_key: "space-main",
            parking_area: Some(ParkingAreaReference::local("parking-main")),
            entry: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("parking-entry"),
                progress_meters: 4.0,
            },
            exit: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("parking-exit"),
                progress_meters: 6.0,
            },
            geometry: ParkingSpaceGeometryInput {
                lateral_offset_meters: -3.0,
                heading_offset_radians: 0.25,
                length_meters: 5.5,
                width_meters: 2.6,
            },
        })
        .unwrap();
    builder.finish().unwrap()
}

fn expect_diagnostics<T>(result: Result<T, DiagnosticBundle>) -> DiagnosticBundle {
    match result {
        Ok(_) => panic!("expected structured diagnostics"),
        Err(bundle) => bundle,
    }
}

fn add_lane_edge_at<'a>(
    builder: &'a mut SyntheticModuleBuilder,
    input: LaneEdgeInput<'_>,
    line: u32,
) -> Result<&'a mut SyntheticModuleBuilder, DiagnosticBundle> {
    builder.add_lane_edge_at(input, SourceSpan::point(Arc::from("source.test"), line, 1))
}

#[test]
fn document_digest_and_length_are_bound_to_the_module() {
    let module = module("city/a", &["city/base"]);
    assert_eq!(
        module.descriptor().source_language(),
        SourceLanguage::SyntheticDsl
    );
    assert_eq!(module.descriptor().frontend_version(), 1);
    let document = module.source_documents().next().unwrap();
    assert_eq!(
        u64::from(document.source_record_byte_len()),
        module.admitted.resource_counts.source_bytes
    );
    assert_ne!(
        module.descriptor().source_document_set_digest(),
        document.source_document_digest()
    );
    assert_eq!(
        module.descriptor().imports().collect::<Vec<_>>(),
        ["city/base"]
    );
}

#[test]
fn empty_module_source_record_has_a_position_independent_known_vector() {
    let source_document_key = Arc::from("source.known-vector");
    let fixed_header = SourceModuleHeader {
        authoring_namespace_id: Arc::from("city/known-vector"),
        source_document_key: Arc::clone(&source_document_key),
        generator_build_id: Arc::from("git:0123456789abcdef"),
        parameters_and_inputs_digest: [0x11; 32],
        frontend_options_digest: [0x22; 32],
        random_seed: Some(42),
        provenance: Arc::from("repository:laneflow"),
        declaration_span: SourceSpan::point(source_document_key, 7, 11),
    };
    let module = SyntheticModuleBuilder::new(fixed_header, &CompileLimits::p100_initial_v1())
        .unwrap()
        .finish()
        .unwrap();

    let document = module.source_documents().next().unwrap();
    assert_eq!(document.source_record_byte_len(), 202);
    assert_eq!(
        *document.source_document_digest(),
        [
            0x30, 0x24, 0xae, 0x9e, 0xb4, 0xa2, 0xcd, 0x16, 0x59, 0x3b, 0xd9, 0x7a, 0xf3, 0xbe,
            0x54, 0xcb, 0x06, 0x61, 0x8d, 0xce, 0x2e, 0x24, 0x3a, 0xc7, 0xc1, 0xb2, 0x3a, 0x12,
            0xef, 0x02, 0xa3, 0x4a,
        ]
    );
    assert_eq!(
        *module.descriptor().source_document_set_digest(),
        [
            0xe2, 0x3b, 0x2c, 0xd1, 0x0c, 0xc8, 0xf7, 0x4c, 0x1f, 0x21, 0x28, 0x03, 0x73, 0xcf,
            0xff, 0x89, 0x16, 0x3f, 0x63, 0x00, 0x6f, 0x26, 0x2f, 0x63, 0x15, 0x06, 0xee, 0x4f,
            0xad, 0x2f, 0xcf, 0x7e,
        ]
    );
}

#[test]
fn lane_edge_accepts_terminal_and_self_loop_topology() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder =
        SyntheticModuleBuilder::new(header("city/a", "source.test"), &limits).unwrap();
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "terminal",
            length_meters: 10.0,
            speed_limit_meters_per_second: 13.0,
            successors: &[],
        })
        .unwrap();
    add_lane_edge_at(
        &mut builder,
        LaneEdgeInput {
            lane_edge_key: "loop",
            length_meters: 20.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[LaneEdgeReference::local("loop")],
        },
        20,
    )
    .unwrap();

    let module = builder.finish().unwrap();
    assert_eq!(module.admitted.resource_counts.declaration_count, 2);
    let TypedAstDeclaration::LaneEdge(terminal) = &module.admitted.declarations[0] else {
        panic!("expected LaneEdge declaration")
    };
    assert!(terminal.successors.is_empty());
    assert_eq!(terminal.header.span.source_document_key(), "source.test");
    let TypedAstDeclaration::LaneEdge(loop_edge) = &module.admitted.declarations[1] else {
        panic!("expected LaneEdge declaration")
    };
    assert_eq!(loop_edge.successors.len(), 1);
    assert_eq!(loop_edge.successors[0].declaration_key.as_ref(), "loop");
}

#[test]
fn lane_edge_rejects_non_finite_and_non_positive_scalars_without_mutation() {
    let limits = CompileLimits::p100_initial_v1();
    for (length, speed, expected_code) in [
        (f64::NAN, 1.0, DiagnosticCode::InvalidLaneEdgeLength),
        (f64::INFINITY, 1.0, DiagnosticCode::InvalidLaneEdgeLength),
        (0.0, 1.0, DiagnosticCode::InvalidLaneEdgeLength),
        (1.0e-9, 1.0, DiagnosticCode::InvalidLaneEdgeLength),
        (
            1.0,
            f64::NEG_INFINITY,
            DiagnosticCode::InvalidLaneEdgeSpeedLimit,
        ),
        (1.0, 0.0, DiagnosticCode::InvalidLaneEdgeSpeedLimit),
    ] {
        let mut builder =
            SyntheticModuleBuilder::new(header("city/a", "source.test"), &limits).unwrap();
        let failure = expect_diagnostics(add_lane_edge_at(
            &mut builder,
            LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: length,
                speed_limit_meters_per_second: speed,
                successors: &[],
            },
            10,
        ));
        assert_eq!(failure.diagnostics()[0].code(), expected_code);
        let module = builder.finish().unwrap();
        assert_eq!(module.admitted.resource_counts.declaration_count, 0);
    }
}

#[test]
fn lane_edge_requires_explicit_import_and_valid_reference_tokens() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder =
        SyntheticModuleBuilder::new(header("city/a", "source.test"), &limits).unwrap();
    let missing_import = expect_diagnostics(add_lane_edge_at(
        &mut builder,
        LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 1.0,
            speed_limit_meters_per_second: 1.0,
            successors: &[LaneEdgeReference::imported("city/base", "edge-b")],
        },
        10,
    ));
    assert_eq!(
        missing_import.diagnostics()[0].code(),
        DiagnosticCode::UnimportedReferenceModule
    );

    let invalid_namespace = expect_diagnostics(add_lane_edge_at(
        &mut builder,
        LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 1.0,
            speed_limit_meters_per_second: 1.0,
            successors: &[LaneEdgeReference::imported("city base", "edge-b")],
        },
        11,
    ));
    assert_eq!(
        invalid_namespace.diagnostics()[0].code(),
        DiagnosticCode::InvalidReferenceNamespace
    );

    let invalid_key = expect_diagnostics(add_lane_edge_at(
        &mut builder,
        LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 1.0,
            speed_limit_meters_per_second: 1.0,
            successors: &[LaneEdgeReference::local("edge b")],
        },
        12,
    ));
    assert_eq!(
        invalid_key.diagnostics()[0].code(),
        DiagnosticCode::InvalidReferenceKey
    );

    builder.add_import("city/base").unwrap();
    add_lane_edge_at(
        &mut builder,
        LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 1.0,
            speed_limit_meters_per_second: 1.0,
            successors: &[LaneEdgeReference::imported("city/base", "edge-b")],
        },
        13,
    )
    .unwrap();
}

#[test]
fn duplicate_lane_edge_and_successor_fail_without_mutating_prior_state() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder =
        SyntheticModuleBuilder::new(header("city/a", "source.test"), &limits).unwrap();
    let duplicate_successor = expect_diagnostics(add_lane_edge_at(
        &mut builder,
        LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 1.0,
            speed_limit_meters_per_second: 1.0,
            successors: &[
                LaneEdgeReference::local("edge-b"),
                LaneEdgeReference::imported("city/a", "edge-b"),
            ],
        },
        10,
    ));
    assert_eq!(
        duplicate_successor.diagnostics()[0].code(),
        DiagnosticCode::DuplicateLaneEdgeSuccessor
    );

    add_lane_edge_at(
        &mut builder,
        LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 1.0,
            speed_limit_meters_per_second: 1.0,
            successors: &[],
        },
        20,
    )
    .unwrap();
    let duplicate_declaration = expect_diagnostics(add_lane_edge_at(
        &mut builder,
        LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 2.0,
            speed_limit_meters_per_second: 2.0,
            successors: &[],
        },
        30,
    ));
    assert_eq!(
        duplicate_declaration.diagnostics()[0].code(),
        DiagnosticCode::DuplicateDeclaration
    );
    assert_eq!(
        duplicate_declaration.diagnostics()[0].related_spans().len(),
        1
    );

    let module = builder.finish().unwrap();
    assert_eq!(module.admitted.resource_counts.declaration_count, 1);
    let TypedAstDeclaration::LaneEdge(edge) = &module.admitted.declarations[0] else {
        panic!("expected LaneEdge declaration")
    };
    assert_eq!(edge.length.value(), 1.0);
}

#[test]
fn lane_edge_successor_order_is_not_source_identity() {
    let limits = CompileLimits::p100_initial_v1();
    let left_successors = [
        LaneEdgeReference::local("edge-c"),
        LaneEdgeReference::local("edge-b"),
    ];
    let right_successors = [
        LaneEdgeReference::local("edge-b"),
        LaneEdgeReference::local("edge-c"),
    ];
    let build = |successors: &[LaneEdgeReference<'_>]| {
        let mut builder =
            SyntheticModuleBuilder::new(header("city/a", "source.test"), &limits).unwrap();
        add_lane_edge_at(
            &mut builder,
            LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 12.5,
                speed_limit_meters_per_second: 13.75,
                successors,
            },
            10,
        )
        .unwrap();
        builder.finish().unwrap()
    };

    let left = build(&left_successors);
    let right = build(&right_successors);
    assert_eq!(
        left.source_documents()
            .next()
            .unwrap()
            .source_document_digest(),
        right
            .source_documents()
            .next()
            .unwrap()
            .source_document_digest()
    );
}

#[test]
fn lane_edge_source_record_has_a_known_vector() {
    let source_document_key = Arc::from("source.lane-edge-vector");
    let fixed_header = SourceModuleHeader {
        authoring_namespace_id: Arc::from("city/lane-edge-vector"),
        source_document_key: Arc::clone(&source_document_key),
        generator_build_id: Arc::from("git:0123456789abcdef"),
        parameters_and_inputs_digest: [0x11; 32],
        frontend_options_digest: [0x22; 32],
        random_seed: Some(42),
        provenance: Arc::from("repository:laneflow"),
        declaration_span: SourceSpan::point(Arc::clone(&source_document_key), 7, 11),
    };
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(fixed_header, &limits).unwrap();
    builder
        .add_lane_edge_at(
            LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 12.5,
                speed_limit_meters_per_second: 13.75,
                successors: &[
                    LaneEdgeReference::local("edge-c"),
                    LaneEdgeReference::local("edge-b"),
                ],
            },
            SourceSpan::point(source_document_key, 13, 17),
        )
        .unwrap();
    let module = builder.finish().unwrap();

    let document = module.source_documents().next().unwrap();
    assert_eq!(document.source_record_byte_len(), 360);
    assert_eq!(
        *document.source_document_digest(),
        [
            0xc9, 0x99, 0xb7, 0xae, 0x09, 0x12, 0xf4, 0x05, 0x31, 0x15, 0xfc, 0xbf, 0x3e, 0x59,
            0xa2, 0xa9, 0x85, 0xb4, 0xb4, 0x60, 0x42, 0x63, 0x13, 0xb2, 0xc4, 0xe2, 0x81, 0x7d,
            0xc7, 0xbc, 0x1b, 0x3c,
        ]
    );
}

#[test]
fn lane_edge_counters_follow_the_calibrated_record_formula() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder =
        SyntheticModuleBuilder::new(header("city/a", "source.test"), &limits).unwrap();
    builder.add_import("city/base").unwrap();
    add_lane_edge_at(
        &mut builder,
        LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 12.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[
                LaneEdgeReference::local("edge-b"),
                LaneEdgeReference::imported("city/base", "edge-c"),
            ],
        },
        10,
    )
    .unwrap();
    let module = builder.finish().unwrap();

    let counts = &module.admitted.resource_counts;
    assert_eq!(counts.declaration_count, 1);
    assert_eq!(counts.reference_count, 2);
    assert_eq!(counts.relation_occurrence_count, 2);
    assert_eq!(counts.identity_field_occurrence_count, 2);
    assert_eq!(counts.symbol_count, 1);
    assert_eq!(counts.typed_ast_record_count, 9);
    assert_eq!(counts.string_item_count, 7);
    assert_eq!(
        u64::from(
            module
                .source_documents()
                .next()
                .unwrap()
                .source_record_byte_len()
        ),
        counts.source_bytes
    );
}

#[test]
fn facility_kind_validation_matches_the_accepted_seed_and_extension_prefixes() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder =
        SyntheticModuleBuilder::new(header("city/a", "source.test"), &limits).unwrap();
    for (key, kind_id) in [
        ("band-sidewalk", "sidewalk"),
        ("band-median", "median"),
        ("band-planting", "plantingStrip"),
        ("band-facility", "facilityStrip"),
        ("band-shoulder", "shoulder"),
        ("band-custom", "x-platform"),
    ] {
        builder
            .add_facility_band(FacilityBandInput {
                facility_band_key: key,
                kind_id,
            })
            .unwrap();
    }

    for invalid in ["x-", "x-lane-"] {
        let diagnostics = expect_diagnostics(builder.add_facility_band(FacilityBandInput {
            facility_band_key: "invalid-band",
            kind_id: invalid,
        }));
        assert_eq!(
            diagnostics.diagnostics()[0].code(),
            DiagnosticCode::InvalidFacilityKind
        );
    }

    let module = builder.finish().unwrap();
    assert_eq!(module.admitted.resource_counts.declaration_count, 6);
}

#[test]
fn duplicate_and_self_imports_fail_with_source_context() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(header("city/a", "source.a"), &limits).unwrap();
    builder.add_import("city/base").unwrap();
    let duplicate = expect_diagnostics(builder.add_import("city/base"));
    assert_eq!(
        duplicate.diagnostics()[0].code(),
        DiagnosticCode::DuplicateImport
    );
    assert!(duplicate.diagnostics()[0].primary_span().is_some());
    assert_eq!(duplicate.diagnostics()[0].related_spans().len(), 1);

    let self_import = expect_diagnostics(builder.add_import("city/a"));
    assert_eq!(
        self_import.diagnostics()[0].code(),
        DiagnosticCode::ImportCycle
    );
}

#[test]
fn import_limit_failure_does_not_mutate_the_module() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(header("city/a", "source.a"), &limits).unwrap();
    let import_limit = limits.value(CompileLimitDimension::ImportEdgeCount);
    for index in 0..import_limit {
        builder
            .add_import(&format!("city/import/{index:04}"))
            .unwrap();
    }

    let failure = expect_diagnostics(builder.add_import("city/import/overflow"));
    assert!(matches!(
        failure.diagnostics()[0].payload(),
        DiagnosticPayload::CompileLimitExceeded {
            dimension: CompileLimitDimension::ImportEdgeCount,
            limit,
            observed,
        } if *limit == import_limit && *observed == import_limit + 1
    ));
    let module = builder.finish().unwrap();
    assert_eq!(
        u64::try_from(module.descriptor().imports().len()).unwrap(),
        import_limit
    );
}

#[test]
fn source_record_encoder_fails_before_over_limit_allocation() {
    let header = header("city/a", "source.a");
    let expected_len = encoded_source_record_len(&header, &[], &[]).unwrap();
    let failure = expect_diagnostics(encode_source_record(&header, &[], &[], expected_len - 1));
    assert!(matches!(
        failure.diagnostics()[0].payload(),
        DiagnosticPayload::CompileLimitExceeded {
            dimension: CompileLimitDimension::SourceBytesPerModule,
            limit,
            observed,
        } if *limit == expected_len - 1 && *observed == expected_len
    ));
}

#[test]
fn compilation_unit_rejects_unknown_imports_and_duplicate_namespaces() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = CompilationUnitBuilder::new(limits.clone());
    builder
        .add_synthetic_module(module("city/a", &["city/missing"]))
        .unwrap();
    let unknown = expect_diagnostics(builder.build());
    assert!(matches!(
        unknown.diagnostics()[0].payload(),
        DiagnosticPayload::UnknownImport { namespace } if namespace.as_ref() == "city/missing"
    ));

    let mut builder = CompilationUnitBuilder::new(limits);
    builder.add_synthetic_module(module("city/a", &[])).unwrap();
    let duplicate = expect_diagnostics(builder.add_synthetic_module(module("city/a", &[])));
    assert_eq!(
        duplicate.diagnostics()[0].code(),
        DiagnosticCode::DuplicateModuleNamespace
    );
    assert_eq!(duplicate.diagnostics()[0].related_spans().len(), 1);
}

#[test]
fn compilation_unit_rejects_duplicate_source_document_keys_atomically() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_synthetic_module(module_with_document("city/a", "shared.document", &[]))
        .unwrap();
    let duplicate = expect_diagnostics(builder.add_synthetic_module(module_with_document(
        "city/b",
        "shared.document",
        &[],
    )));
    assert!(matches!(
        duplicate.diagnostics()[0].payload(),
        DiagnosticPayload::DuplicateSourceDocumentKey {
            source_document_key
        } if source_document_key.as_ref() == "shared.document"
    ));
    assert_eq!(duplicate.diagnostics()[0].related_spans().len(), 1);

    // 重复文档失败发生在任何累计计数变更之前；修正文档键后，同一 namespace 可直接
    // 重试。这里同时防止未来把文档唯一性检查移到非原子的 build 阶段。
    builder
        .add_synthetic_module(module_with_document("city/b", "city-b.document", &[]))
        .unwrap();
    let unit = builder.build().unwrap();
    assert_eq!(unit.module_descriptors().len(), 2);
}

#[test]
fn compilation_unit_uses_dependency_first_canonical_order() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_synthetic_module(module("city/z", &["city/a"]))
        .unwrap();
    builder.add_synthetic_module(module("city/b", &[])).unwrap();
    builder.add_synthetic_module(module("city/a", &[])).unwrap();
    let unit = builder.build().unwrap();
    assert_eq!(
        unit.module_descriptors()
            .map(SourceModuleDescriptor::authoring_namespace_id)
            .collect::<Vec<_>>(),
        ["city/a", "city/b", "city/z"]
    );
}

#[test]
fn compilation_unit_reports_canonical_cycle() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_synthetic_module(module("city/c", &["city/a"]))
        .unwrap();
    builder
        .add_synthetic_module(module("city/a", &["city/b"]))
        .unwrap();
    builder
        .add_synthetic_module(module("city/b", &["city/c"]))
        .unwrap();
    let bundle = expect_diagnostics(builder.build());
    assert!(matches!(
        bundle.diagnostics()[0].payload(),
        DiagnosticPayload::ImportCycle { namespaces }
            if namespaces.iter().map(AsRef::as_ref).collect::<Vec<&str>>()
                == ["city/a", "city/b", "city/c"]
    ));
}

#[test]
fn compilation_unit_reports_every_disjoint_cycle_in_canonical_order() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    for module in [
        module("city/d", &["city/c"]),
        module("city/a", &["city/b"]),
        module("city/c", &["city/d"]),
        module("city/b", &["city/a"]),
    ] {
        builder.add_synthetic_module(module).unwrap();
    }

    let bundle = expect_diagnostics(builder.build());
    assert_eq!(bundle.diagnostics().len(), 2);
    let cycles: Vec<_> = bundle
        .diagnostics()
        .iter()
        .map(|diagnostic| match diagnostic.payload() {
            DiagnosticPayload::ImportCycle { namespaces } => {
                namespaces.iter().map(AsRef::as_ref).collect::<Vec<&str>>()
            }
            other => panic!("expected import cycle, got {other:?}"),
        })
        .collect();
    assert_eq!(cycles, [["city/a", "city/b"], ["city/c", "city/d"]]);
    assert!(
        bundle
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.primary_span().is_some())
    );
}

#[test]
fn v1_rejects_a_multi_document_official_module_without_mutation() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    let test_module = TestOfficialModule::from_synthetic_with_documents(
        module_with_document("city/a", "source.primary", &[]),
        &[("source.secondary", b"secondary")],
    );
    let diagnostics = expect_diagnostics(builder.add_test_official_module(test_module));
    assert!(matches!(
        diagnostics.diagnostics()[0].payload(),
        DiagnosticPayload::CompileProfileIncompatible {
            profile_id,
            required_dimension: CompileLimitDimension::SourceDocumentCount,
        } if profile_id.as_ref() == "LF-COMP-P100-INITIAL-v1"
    ));

    builder
        .add_synthetic_module(module_with_document("city/a", "source.primary", &[]))
        .unwrap();
    assert_eq!(builder.build().unwrap().source_document_count(), 1);
}

#[test]
fn common_admission_rejects_duplicate_keys_inside_one_official_module_atomically() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v2());
    let duplicate = TestOfficialModule::from_synthetic_with_documents(
        module_with_document("city/a", "source/shared", &[]),
        &[("source/shared", b"duplicate")],
    );
    let diagnostics = expect_diagnostics(builder.add_test_official_module(duplicate));
    assert!(matches!(
        diagnostics.diagnostics()[0].payload(),
        DiagnosticPayload::DuplicateSourceDocumentKey { source_document_key }
            if source_document_key.as_ref() == "source/shared"
    ));

    builder
        .add_synthetic_module(module_with_document("city/a", "source/shared", &[]))
        .unwrap();
    assert_eq!(builder.build().unwrap().source_document_count(), 1);
}

#[test]
fn common_admission_enforces_canonical_document_order_before_index_commit() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v2());
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        TestOfficialModule::from_synthetic_with_unsorted_documents(
            module_with_document("city/a", "source/a", &[]),
            &[("source/a", b"duplicate"), ("source/b", b"b")],
        )
    }));
    let panic = match unwind {
        Ok(_) => panic!("unsorted official documents must violate admission invariant"),
        Err(panic) => panic,
    };
    let message = panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or_default();
    assert!(message.contains("canonically sort source documents"));

    builder
        .add_synthetic_module(module_with_document("city/a", "source/a", &[]))
        .unwrap();
    assert_eq!(builder.build().unwrap().source_document_count(), 1);
}

#[test]
fn common_admission_enforces_every_owned_cumulative_resource_dimension_atomically() {
    let dimensions = [
        CompileLimitDimension::ModuleCount,
        CompileLimitDimension::SourceDocumentCount,
        CompileLimitDimension::ImportEdgeCount,
        CompileLimitDimension::SourceBytesTotal,
        CompileLimitDimension::DeclarationCount,
        CompileLimitDimension::TypedAstRecordCount,
        CompileLimitDimension::ReferenceCount,
        CompileLimitDimension::RelationOccurrenceCount,
        CompileLimitDimension::IdentityFieldOccurrenceCount,
        CompileLimitDimension::SymbolCount,
        CompileLimitDimension::StringItemCount,
        CompileLimitDimension::TotalStringBytes,
        CompileLimitDimension::ManeuverGateCount,
        CompileLimitDimension::WaitingZoneCount,
        CompileLimitDimension::RouteOccurrenceCount,
        CompileLimitDimension::GeometryPointCount,
        CompileLimitDimension::CompilerControlledLiveBytes,
    ];

    for dimension in dimensions {
        let limits = CompileLimits::p100_initial_v2().with_test_admission_limit(dimension, 0);
        let mut builder = CompilationUnitBuilder::new(limits);
        let diagnostics = match dimension {
            CompileLimitDimension::ImportEdgeCount => {
                let frontend_limits = CompileLimits::p100_initial_v1();
                let mut module_builder =
                    SyntheticModuleBuilder::new(header("city/a", "source/a"), &frontend_limits)
                        .unwrap();
                module_builder.add_import("city/target").unwrap();
                expect_diagnostics(builder.add_synthetic_module(module_builder.finish().unwrap()))
            }
            CompileLimitDimension::DeclarationCount
            | CompileLimitDimension::TypedAstRecordCount
            | CompileLimitDimension::ReferenceCount
            | CompileLimitDimension::RelationOccurrenceCount
            | CompileLimitDimension::IdentityFieldOccurrenceCount
            | CompileLimitDimension::SymbolCount
            | CompileLimitDimension::StringItemCount
            | CompileLimitDimension::TotalStringBytes
            | CompileLimitDimension::ManeuverGateCount
            | CompileLimitDimension::WaitingZoneCount
            | CompileLimitDimension::RouteOccurrenceCount
            | CompileLimitDimension::GeometryPointCount => {
                let mut module = TestOfficialModule::from_synthetic_with_documents(
                    module_with_document("city/a", "source/a", &[]),
                    &[],
                );
                module.force_resource_count(dimension, 1);
                expect_diagnostics(builder.add_test_official_module(module))
            }
            _ => expect_diagnostics(builder.add_synthetic_module(module_with_document(
                "city/a",
                "source/a",
                &[],
            ))),
        };
        assert!(matches!(
            diagnostics.diagnostics()[0].payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: actual_dimension,
                limit: 0,
                observed,
            } if *actual_dimension == dimension && *observed > 0
        ));
        assert!(
            builder.modules.is_empty(),
            "failed dimension: {dimension:?}"
        );
        assert!(builder.module_index.is_empty());
        assert!(builder.source_document_index_is_empty());
        assert_eq!(builder.totals, Default::default());
    }
}

#[test]
fn admission_sizing_accounts_for_builder_indexes_wrappers_and_result_modules() {
    let limits = CompileLimits::p100_initial_v2();
    let mut builder = CompilationUnitBuilder::new(limits.clone());
    builder
        .add_synthetic_module(module_with_document("city/a", "source/a", &[]))
        .unwrap();

    let sizing = AdmissionSizing::from_totals(
        builder.totals,
        limits.value(CompileLimitDimension::DiagnosticCount),
    );
    let expected_document_index = source_document_index_requested_bytes(1);
    let expected_module_index = requested_hash_table_bytes::<Arc<str>, usize>(1);
    assert_eq!(
        sizing.builder_live_bytes,
        builder
            .totals
            .module_payload_live_bytes
            .saturating_add(expected_document_index)
            .saturating_add(expected_module_index)
            .saturating_add(size_bytes::<AdmittedOfficialModule>(1))
    );
    assert_eq!(
        sizing.result_live_bytes,
        builder
            .totals
            .module_payload_live_bytes
            .saturating_add(expected_document_index)
            .saturating_add(size_bytes::<TypedAstModule>(1))
            // 与 `modules` 平行的冻结几何载荷向量随结果存续。
            .saturating_add(size_bytes::<
                Option<super::geometry::FrozenGeometryModulePayload>,
            >(1))
    );
    assert!(sizing.build_scratch_bytes > 0);
    assert!(sizing.build_peak_live_bytes > sizing.builder_live_bytes);

    let unit = builder.build().unwrap();
    assert_eq!(unit.controlled_live_bytes, sizing.result_live_bytes);
}

#[test]
fn build_checks_admission_scratch_and_peak_boundaries_before_freezing() {
    fn sizing_for_one_module() -> AdmissionSizing {
        let limits = CompileLimits::p100_initial_v2();
        let mut builder = CompilationUnitBuilder::new(limits.clone());
        builder
            .add_synthetic_module(module_with_document("city/a", "source/a", &[]))
            .unwrap();
        AdmissionSizing::from_totals(
            builder.totals,
            limits.value(CompileLimitDimension::DiagnosticCount),
        )
    }

    fn build_with_limit(
        dimension: CompileLimitDimension,
        limit: u32,
    ) -> Result<CompilationUnit, DiagnosticBundle> {
        let limits = CompileLimits::p100_initial_v2().with_test_admission_limit(dimension, limit);
        let mut builder = CompilationUnitBuilder::new(limits);
        builder
            .add_synthetic_module(module_with_document("city/a", "source/a", &[]))
            .unwrap();
        builder.build()
    }

    let sizing = sizing_for_one_module();
    for (dimension, observed) in [
        (
            CompileLimitDimension::StageScratchBytes,
            sizing.build_scratch_bytes,
        ),
        (
            CompileLimitDimension::CompilerControlledLiveBytes,
            sizing.build_peak_live_bytes,
        ),
    ] {
        let boundary = u32::try_from(observed).unwrap();
        let diagnostics = expect_diagnostics(build_with_limit(dimension, boundary - 1));
        assert!(matches!(
            diagnostics.diagnostics()[0].payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: actual_dimension,
                limit,
                observed: actual_observed,
            } if *actual_dimension == dimension
                && *limit == u64::from(boundary - 1)
                && *actual_observed == observed
        ));

        let unit = build_with_limit(dimension, boundary).unwrap();
        assert_eq!(unit.module_count(), 1);
    }
}

#[test]
fn default_profile_can_freeze_its_maximum_module_count() {
    let limits = CompileLimits::p100_initial_v2();
    let mut builder = CompilationUnitBuilder::new(limits.clone());
    for index in 0..limits.value(CompileLimitDimension::ModuleCount) {
        let namespace = format!("city/{index:03}");
        let document = format!("source/{index:03}");
        builder
            .add_synthetic_module(module_with_document(&namespace, &document, &[]))
            .unwrap();
    }
    let sizing = AdmissionSizing::from_totals(
        builder.totals,
        limits.value(CompileLimitDimension::DiagnosticCount),
    );
    assert!(sizing.build_scratch_bytes <= limits.value(CompileLimitDimension::StageScratchBytes));
    assert!(
        sizing.build_peak_live_bytes
            <= limits.value(CompileLimitDimension::CompilerControlledLiveBytes)
    );

    let unit = builder.build().unwrap();
    assert_eq!(
        u64::try_from(unit.module_count()).unwrap(),
        limits.value(CompileLimitDimension::ModuleCount)
    );
}

#[test]
fn build_limit_diagnostic_uses_canonical_module_context() {
    fn add_modules(builder: &mut CompilationUnitBuilder, namespaces: [&str; 2]) {
        for namespace in namespaces {
            let document = format!("source/{namespace}");
            builder
                .add_synthetic_module(module_with_document(namespace, &document, &[]))
                .unwrap();
        }
    }

    let default_limits = CompileLimits::p100_initial_v2();
    let mut sizing_builder = CompilationUnitBuilder::new(default_limits.clone());
    add_modules(&mut sizing_builder, ["city/z", "city/a"]);
    let sizing = AdmissionSizing::from_totals(
        sizing_builder.totals,
        default_limits.value(CompileLimitDimension::DiagnosticCount),
    );
    let failing_limit = u32::try_from(sizing.build_scratch_bytes).unwrap() - 1;

    for namespaces in [["city/z", "city/a"], ["city/a", "city/z"]] {
        let limits = CompileLimits::p100_initial_v2()
            .with_test_admission_limit(CompileLimitDimension::StageScratchBytes, failing_limit);
        let mut builder = CompilationUnitBuilder::new(limits);
        add_modules(&mut builder, namespaces);
        let diagnostics = expect_diagnostics(builder.build());
        assert_eq!(diagnostics.diagnostics()[0].stable_key(), Some("city/a"));
    }
}

#[test]
fn v2_source_document_count_accepts_boundary_and_rejects_boundary_plus_one_atomically() {
    fn document_inputs(count: usize) -> (Vec<String>, Vec<Vec<u8>>) {
        let keys = (0..count)
            .map(|index| format!("extra/{index:04}"))
            .collect::<Vec<_>>();
        let records = (0..count)
            .map(|index| vec![u8::try_from(index % 251).unwrap()])
            .collect::<Vec<_>>();
        (keys, records)
    }

    let (boundary_keys, boundary_records) = document_inputs(1_565);
    let boundary_inputs = boundary_keys
        .iter()
        .zip(&boundary_records)
        .map(|(key, record)| (key.as_str(), record.as_slice()))
        .collect::<Vec<_>>();
    let mut boundary_builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v2());
    boundary_builder
        .add_test_official_module(TestOfficialModule::from_synthetic_with_documents(
            module_with_document("city/boundary", "primary", &[]),
            &boundary_inputs,
        ))
        .unwrap();
    let boundary_unit = boundary_builder.build().unwrap();
    assert_eq!(boundary_unit.source_document_count(), 1_566);
    let mut compiler = crate::Compiler::new();
    let boundary_output = compiler.compile(boundary_unit).unwrap();
    assert_eq!(
        boundary_output.source_map_input().source_documents().len(),
        1_566
    );

    let (over_keys, over_records) = document_inputs(1_566);
    let over_inputs = over_keys
        .iter()
        .zip(&over_records)
        .map(|(key, record)| (key.as_str(), record.as_slice()))
        .collect::<Vec<_>>();
    let mut over_builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v2());
    let diagnostics = expect_diagnostics(over_builder.add_test_official_module(
        TestOfficialModule::from_synthetic_with_documents(
            module_with_document("city/over", "primary", &[]),
            &over_inputs,
        ),
    ));
    assert!(matches!(
        diagnostics.diagnostics()[0].payload(),
        DiagnosticPayload::CompileLimitExceeded {
            dimension: CompileLimitDimension::SourceDocumentCount,
            limit: 1_566,
            observed: 1_567,
        }
    ));
    over_builder
        .add_synthetic_module(module_with_document("city/over", "primary", &[]))
        .unwrap();
    assert_eq!(over_builder.build().unwrap().source_document_count(), 1);
}

#[test]
fn document_set_digest_and_frozen_document_order_ignore_input_order() {
    let left = TestOfficialModule::from_synthetic_with_documents(
        module_with_document("city/a", "source/m", &[]),
        &[("source/z", b"z"), ("source/a", b"a")],
    );
    let right = TestOfficialModule::from_synthetic_with_documents(
        module_with_document("city/a", "source/m", &[]),
        &[("source/a", b"a"), ("source/z", b"z")],
    );
    assert_eq!(
        left.admitted.descriptor.source_document_set_digest(),
        right.admitted.descriptor.source_document_set_digest()
    );
    assert_eq!(
        left.admitted
            .source_documents
            .iter()
            .map(SourceDocumentDescriptor::source_document_key)
            .collect::<Vec<_>>(),
        ["source/a", "source/m", "source/z"]
    );
}

#[test]
fn source_document_index_and_flat_descriptors_are_in_peak_live_accounting() {
    fn compile_with_extra_documents(documents: &[(&str, &[u8])]) -> u64 {
        let limits = CompileLimits::p100_initial_v2();
        let module = TestOfficialModule::from_synthetic_with_documents(
            module_with_document("city/a", "source/primary", &[]),
            documents,
        );
        let mut builder = CompilationUnitBuilder::new(limits);
        builder.add_test_official_module(module).unwrap();
        crate::Compiler::new()
            .compile(builder.build().unwrap())
            .unwrap()
            .source_map_input()
            .peak_controlled_live_bytes()
    }

    let one_document_peak = compile_with_extra_documents(&[]);
    let extra_documents = [
        ("source/secondary", b"secondary".as_slice()),
        ("source/tertiary", b"tertiary".as_slice()),
    ];
    let three_document_peak = compile_with_extra_documents(&extra_documents);
    let extra_key_bytes = extra_documents.iter().fold(0_u64, |total, (key, _)| {
        total.saturating_add(u64::try_from(key.len()).unwrap_or(u64::MAX))
    });
    let descriptor_bytes = size_bytes::<SourceDocumentDescriptor>(2);
    let index_growth = source_document_index_requested_bytes(3)
        .saturating_sub(source_document_index_requested_bytes(1));
    let expected_growth = extra_key_bytes
        .saturating_add(descriptor_bytes)
        .saturating_add(index_growth)
        .saturating_add(descriptor_bytes);
    assert_eq!(
        three_document_peak.saturating_sub(one_document_peak),
        expected_growth
    );
}

#[test]
fn three_document_module_retains_distinct_entity_relation_and_cold_origins() {
    let limits = CompileLimits::p100_initial_v2();
    let mut synthetic =
        SyntheticModuleBuilder::new(header("city/a", "source/primary"), &limits).unwrap();
    synthetic
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 12.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[LaneEdgeReference::local("edge-b")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-b",
            length_meters: 8.0,
            speed_limit_meters_per_second: 6.0,
            successors: &[],
        })
        .unwrap();
    SOURCE_DOCUMENT_DIGEST_CALL_COUNT.with(|count| count.set(0));
    let mut test_module = {
        let secondary_record = b"secondary record".to_vec();
        let tertiary_record = b"tertiary record".to_vec();
        let secondary_origin = String::from("memory://secondary");
        let tertiary_origin = String::from("memory://tertiary");
        TestOfficialModule::from_synthetic_with_document_records(
            synthetic.finish().unwrap(),
            &[
                TestSourceDocument {
                    source_document_key: "source/secondary",
                    source_record: &secondary_record,
                    display_source: Some(&secondary_origin),
                },
                TestSourceDocument {
                    source_document_key: "source/tertiary",
                    source_record: &tertiary_record,
                    display_source: Some(&tertiary_origin),
                },
            ],
        )
    };
    SOURCE_DOCUMENT_DIGEST_CALL_COUNT.with(|count| assert_eq!(count.get(), 3));
    test_module.move_module_declaration_span_to("source/secondary");
    test_module.move_first_lane_edge_span_to("source/secondary");
    test_module.move_first_lane_edge_successor_span_to("source/tertiary");

    let mut unit_builder = CompilationUnitBuilder::new(limits);
    unit_builder.add_test_official_module(test_module).unwrap();
    let mut compiler = crate::Compiler::new();
    let output = compiler.compile(unit_builder.build().unwrap()).unwrap();
    assert_eq!(
        output
            .source_map_input()
            .source_module_sources()
            .map(|source| (
                source.descriptor().authoring_namespace_id(),
                source.primary_source().source_document_key(),
                source.primary_source().start().line(),
                source.primary_source().start().column(),
            ))
            .collect::<Vec<_>>(),
        [("city/a", "source/secondary", 37, 5)]
    );
    assert_eq!(
        output
            .source_map_input()
            .source_documents()
            .map(|document| document.source_document_key())
            .collect::<Vec<_>>(),
        ["source/primary", "source/secondary", "source/tertiary"]
    );
    assert_eq!(
        output
            .source_map_input()
            .lane_edge_sources()
            .filter(|source| {
                source.primary_source().source_document_key() == "source/secondary"
            })
            .count(),
        1
    );
    assert_eq!(
        output
            .source_map_input()
            .lane_edge_successor_sources()
            .next()
            .unwrap()
            .primary_source()
            .source_document_key(),
        "source/tertiary"
    );
    assert_eq!(
        output
            .source_map_input()
            .source_documents()
            .map(|document| (
                document.source_document_key(),
                document.origin().display_source()
            ))
            .collect::<Vec<_>>(),
        [
            ("source/primary", None),
            ("source/secondary", Some("memory://secondary")),
            ("source/tertiary", Some("memory://tertiary")),
        ]
    );
}

#[test]
fn signal_relations_keep_their_own_multi_document_locations() {
    use crate::identity::{IdentityFieldInput, encode_canonical_identity};
    use laneflow_static_contract::{EntityKind, FieldTag};

    let limits = CompileLimits::p100_initial_v2();
    let max_identity_field_bytes = limits.value(CompileLimitDimension::SingleStringBytes);
    let mut module = TestOfficialModule::from_synthetic_with_documents(
        signal_module_with_document("city/signal", "source/primary"),
        &[
            ("source/controller-groups", b"controller groups"),
            ("source/phase-states", b"phase states"),
            ("source/gate-signals", b"gate signals"),
        ],
    );
    module.move_signal_relation_spans_to(
        "source/controller-groups",
        "source/phase-states",
        "source/gate-signals",
    );

    let mut builder = CompilationUnitBuilder::new(limits);
    builder.add_test_official_module(module).unwrap();
    let output = crate::Compiler::new()
        .compile(builder.build().unwrap())
        .unwrap();

    let sources = output
        .source_map_input()
        .signal_relation_sources()
        .map(|source| {
            let primary = source.primary_source();
            (
                source.role(),
                source.local_index(),
                primary.source_document_key().to_owned(),
                primary.start().line(),
                primary.start().column(),
            )
        })
        .collect::<Vec<_>>();
    let controller_groups = sources
        .iter()
        .filter(|source| source.0 == SourceRelationRole::SignalControllerGroup)
        .collect::<Vec<_>>();
    let phase_states = sources
        .iter()
        .filter(|source| source.0 == SourceRelationRole::SignalPhaseState)
        .collect::<Vec<_>>();
    let lir = output.lir();
    let controller = lir.signal_controllers().next().unwrap();
    let group_key = |ordinal| {
        let group = lir.signal_group(ordinal).unwrap();
        let field = group
            .identity_fields()
            .find(|field| field.tag() == FieldTag::SignalGroupKey)
            .unwrap();
        std::str::from_utf8(field.value_bytes()).unwrap().to_owned()
    };
    let controller_group_keys = controller
        .signal_groups()
        .iter()
        .copied()
        .map(group_key)
        .collect::<Vec<_>>();
    let phase_group_keys = lir
        .signal_phases()
        .next()
        .unwrap()
        .states()
        .map(|state| group_key(state.signal_group()))
        .collect::<Vec<_>>();

    // 此夹具必须真正覆盖两种顺序不一致的反例：HIR 用 StableId 消除来源顺序，LIR
    // 则按完整 Identity v1 前像排序。
    let stable_id = |key: &str| {
        encode_canonical_identity(
            EntityKind::SignalGroup,
            &[
                IdentityFieldInput::new(FieldTag::AuthoringNamespaceId, b"city/signal"),
                IdentityFieldInput::new(FieldTag::SignalGroupKey, key.as_bytes()),
            ],
            max_identity_field_bytes,
        )
        .unwrap()
        .stable_id()
    };
    let mut hir_stage_group_keys = ["group-main", "group-release"];
    hir_stage_group_keys.sort_unstable_by_key(|key| stable_id(key));
    assert_ne!(
        hir_stage_group_keys,
        [
            controller_group_keys[0].as_str(),
            controller_group_keys[1].as_str()
        ],
        "fixture must keep HIR StableId order distinct from LIR identity order"
    );

    assert_eq!(controller_groups.len(), 2);
    assert_eq!(phase_states.len(), 2);
    assert_eq!(controller_group_keys, ["group-main", "group-release"]);
    assert_eq!(phase_group_keys, controller_group_keys);
    for ((controller_group, phase_state), group_key) in controller_groups
        .iter()
        .zip(&phase_states)
        .zip(&controller_group_keys)
    {
        assert_eq!(controller_group.1, phase_state.1);
        assert_eq!(controller_group.2, "source/controller-groups");
        assert_eq!(phase_state.2, "source/phase-states");
        let relation_offset = u32::from(group_key == "group-release");
        assert_eq!(controller_group.3, 51 + relation_offset);
        assert_eq!(phase_state.3, 61 + relation_offset);
        assert_eq!(controller_group.4, 3);
        assert_eq!(phase_state.4, 5);
    }

    let mut gate_signal_lines = sources
        .iter()
        .filter(|source| source.0 == SourceRelationRole::ManeuverGateSignalGroup)
        .map(|source| {
            assert_eq!(source.1, 0);
            assert_eq!(source.2, "source/gate-signals");
            assert_eq!(source.4, 7);
            source.3
        })
        .collect::<Vec<_>>();
    gate_signal_lines.sort_unstable();
    assert_eq!(gate_signal_lines, [71, 72]);
}

#[test]
fn every_authored_owner_relation_keeps_its_multi_document_location() {
    let limits = CompileLimits::p100_initial_v2();
    let mut module = TestOfficialModule::from_synthetic_with_documents(
        signal_module_with_document("city/authored-relations", "source/primary"),
        &[("source/authored-relations", b"authored relations")],
    );
    module.move_authored_relation_spans_to("source/authored-relations");

    let mut builder = CompilationUnitBuilder::new(limits);
    builder.add_test_official_module(module).unwrap();
    let output = crate::Compiler::new()
        .compile(builder.build().unwrap())
        .unwrap();
    let source_map = output.source_map_input();
    let mut documents = std::collections::BTreeMap::<SourceRelationRole, Vec<String>>::new();
    for source in source_map.cross_section_relation_sources() {
        documents
            .entry(source.role())
            .or_default()
            .push(source.primary_source().source_document_key().to_owned());
    }
    for source in source_map.junction_relation_sources() {
        documents
            .entry(source.role())
            .or_default()
            .push(source.primary_source().source_document_key().to_owned());
    }
    for source in source_map.signal_relation_sources() {
        documents
            .entry(source.role())
            .or_default()
            .push(source.primary_source().source_document_key().to_owned());
    }
    for source in source_map.parking_relation_sources() {
        documents
            .entry(source.role())
            .or_default()
            .push(source.primary_source().source_document_key().to_owned());
    }

    for (role, expected_count) in [
        (SourceRelationRole::RoadCorridorElement, 2),
        (SourceRelationRole::RoadSectionLane, 1),
        (SourceRelationRole::LaneGroupMember, 1),
        (SourceRelationRole::JunctionMovement, 1),
        (SourceRelationRole::MovementManeuverPath, 1),
        (SourceRelationRole::ManeuverPathGate, 2),
        (SourceRelationRole::ManeuverPathWaitingZone, 1),
        (SourceRelationRole::StopLineManeuverGate, 2),
        (SourceRelationRole::SignalControllerGroup, 2),
        (SourceRelationRole::SignalPhaseState, 2),
        (SourceRelationRole::ManeuverGateSignalGroup, 2),
        (SourceRelationRole::ParkingSpaceArea, 1),
        (SourceRelationRole::ParkingSpaceEntry, 1),
        (SourceRelationRole::ParkingSpaceExit, 1),
    ] {
        let actual = documents
            .get(&role)
            .unwrap_or_else(|| panic!("missing authored relation source records for {role:?}"));
        assert_eq!(actual.len(), expected_count, "relation count for {role:?}");
        assert!(
            actual
                .iter()
                .all(|document| document == "source/authored-relations"),
            "relation source document for {role:?}: {actual:?}",
        );
    }
}

#[test]
fn source_map_output_bytes_include_module_declaration_document_ordinal() {
    let limits = CompileLimits::p100_initial_v2();
    let mut module = TestOfficialModule::from_synthetic_with_documents(
        module_with_document("city/a", "source/primary", &[]),
        &[("source/secondary", b"secondary")],
    );
    module.move_module_declaration_span_to("source/secondary");

    let mut builder = CompilationUnitBuilder::new(limits);
    builder.add_test_official_module(module).unwrap();
    let mut unit = builder.build().unwrap();
    let source_map_logical_bytes = unit.modules.iter().fold(0_u64, |total, module| {
        module.source_documents.iter().fold(
            total
                .saturating_add(module.descriptor.source_map_logical_bytes())
                .saturating_add(4 + 8 + 8),
            |document_total, document| {
                document_total.saturating_add(document.source_map_logical_bytes())
            },
        )
    });
    let hir = crate::hir::build_hir(&unit).unwrap();
    let mir = crate::mir::lower_to_mir(&unit, &hir).unwrap();
    let lir_output_bytes = crate::lir::freeze_lir(&unit, &mir)
        .unwrap()
        .lir
        .output_bytes;
    let expected_output_bytes = lir_output_bytes.saturating_add(source_map_logical_bytes);
    let failing_limit = u32::try_from(expected_output_bytes - 1).unwrap();
    unit.limits = CompileLimits::p100_initial_v2().with_test_lir_limits(
        u32::MAX,
        u32::MAX,
        failing_limit,
        u32::MAX,
    );

    let diagnostics = expect_diagnostics(crate::Compiler::new().compile(unit));
    assert!(diagnostics.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.payload(),
        DiagnosticPayload::CompileLimitExceeded {
            dimension: CompileLimitDimension::OutputBytes,
            limit,
            observed,
        } if *limit == u64::from(failing_limit) && *observed == expected_output_bytes
    )));
}

#[test]
fn source_map_rejects_a_span_bound_to_another_modules_document() {
    let limits = CompileLimits::p100_initial_v2();
    let mut synthetic = SyntheticModuleBuilder::new(header("city/a", "source/a"), &limits).unwrap();
    synthetic
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 12.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[],
        })
        .unwrap();
    let mut test_module =
        TestOfficialModule::from_synthetic_with_documents(synthetic.finish().unwrap(), &[]);
    test_module.move_first_lane_edge_span_to("source/b");

    let mut unit_builder = CompilationUnitBuilder::new(limits);
    unit_builder.add_test_official_module(test_module).unwrap();
    unit_builder
        .add_synthetic_module(module_with_document("city/b", "source/b", &[]))
        .unwrap();
    let diagnostics =
        expect_diagnostics(crate::Compiler::new().compile(unit_builder.build().unwrap()));
    assert!(matches!(
        diagnostics.diagnostics()[0].payload(),
        DiagnosticPayload::SourceDocumentOwnershipMismatch {
            source_document_key,
            expected_authoring_namespace_id,
            actual_authoring_namespace_id: Some(actual),
        } if source_document_key.as_ref() == "source/b"
            && expected_authoring_namespace_id.as_ref() == "city/a"
            && actual.as_ref() == "city/b"
    ));
}

#[test]
fn source_map_rejects_an_unregistered_span_document_without_panicking() {
    let limits = CompileLimits::p100_initial_v2();
    let mut synthetic = SyntheticModuleBuilder::new(header("city/a", "source/a"), &limits).unwrap();
    synthetic
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 12.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[],
        })
        .unwrap();
    let mut test_module =
        TestOfficialModule::from_synthetic_with_documents(synthetic.finish().unwrap(), &[]);
    test_module.move_first_lane_edge_span_to("source/missing");

    let mut unit_builder = CompilationUnitBuilder::new(limits);
    unit_builder.add_test_official_module(test_module).unwrap();
    let diagnostics =
        expect_diagnostics(crate::Compiler::new().compile(unit_builder.build().unwrap()));
    assert!(matches!(
        diagnostics.diagnostics()[0].payload(),
        DiagnosticPayload::SourceDocumentOwnershipMismatch {
            source_document_key,
            expected_authoring_namespace_id,
            actual_authoring_namespace_id: None,
        } if source_document_key.as_ref() == "source/missing"
            && expected_authoring_namespace_id.as_ref() == "city/a"
    ));
}

#[test]
fn hir_rejects_cross_module_source_spans_before_semantic_diagnostics() {
    #[derive(Clone, Copy)]
    enum CorruptedSpan {
        Module,
        Declaration,
        Reference,
    }

    for corrupted_span in [
        CorruptedSpan::Module,
        CorruptedSpan::Declaration,
        CorruptedSpan::Reference,
    ] {
        let limits = CompileLimits::p100_initial_v2();
        let mut synthetic =
            SyntheticModuleBuilder::new(header("city/a", "source/a"), &limits).unwrap();
        synthetic
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 12.0,
                speed_limit_meters_per_second: 8.0,
                successors: &[LaneEdgeReference::local("missing-edge")],
            })
            .unwrap();
        let mut test_module =
            TestOfficialModule::from_synthetic_with_documents(synthetic.finish().unwrap(), &[]);
        match corrupted_span {
            CorruptedSpan::Module => test_module.move_module_declaration_span_to("source/b"),
            CorruptedSpan::Declaration => test_module.move_first_lane_edge_span_to("source/b"),
            CorruptedSpan::Reference => {
                test_module.move_first_lane_edge_successor_span_to("source/b");
            }
        }

        let mut unit_builder = CompilationUnitBuilder::new(limits);
        unit_builder.add_test_official_module(test_module).unwrap();
        unit_builder
            .add_synthetic_module(module_with_document("city/b", "source/b", &[]))
            .unwrap();
        let diagnostics =
            expect_diagnostics(crate::Compiler::new().compile(unit_builder.build().unwrap()));

        assert_eq!(diagnostics.diagnostics().len(), 1);
        assert!(matches!(
            diagnostics.diagnostics()[0].payload(),
            DiagnosticPayload::SourceDocumentOwnershipMismatch {
                source_document_key,
                expected_authoring_namespace_id,
                actual_authoring_namespace_id: Some(actual),
            } if source_document_key.as_ref() == "source/b"
                && expected_authoring_namespace_id.as_ref() == "city/a"
                && actual.as_ref() == "city/b"
        ));
    }
}

#[test]
fn hir_rejects_unregistered_relation_span_before_semantic_diagnostics() {
    let limits = CompileLimits::p100_initial_v2();
    let mut synthetic = SyntheticModuleBuilder::new(header("city/a", "source/a"), &limits).unwrap();
    synthetic
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 12.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[LaneEdgeReference::local("missing-edge")],
        })
        .unwrap();
    let mut test_module =
        TestOfficialModule::from_synthetic_with_documents(synthetic.finish().unwrap(), &[]);
    test_module.move_first_lane_edge_successor_span_to("source/missing");

    let mut unit_builder = CompilationUnitBuilder::new(limits);
    unit_builder.add_test_official_module(test_module).unwrap();
    let diagnostics =
        expect_diagnostics(crate::Compiler::new().compile(unit_builder.build().unwrap()));

    assert_eq!(diagnostics.diagnostics().len(), 1);
    assert!(matches!(
        diagnostics.diagnostics()[0].payload(),
        DiagnosticPayload::SourceDocumentOwnershipMismatch {
            source_document_key,
            expected_authoring_namespace_id,
            actual_authoring_namespace_id: None,
        } if source_document_key.as_ref() == "source/missing"
            && expected_authoring_namespace_id.as_ref() == "city/a"
    ));
}

#[test]
#[ignore = "measurement-only admission benchmark; run explicitly with --release --nocapture"]
fn benchmark_common_admission_only_reports_median_mad_and_memory() {
    use std::hint::black_box;
    use std::time::Instant;

    const MODULE_COUNT: usize = 100;
    const SAMPLE_COUNT: usize = 7;

    fn fixture() -> Vec<TestOfficialModule> {
        (0..MODULE_COUNT)
            .map(|index| {
                let namespace = format!("bench/module-{index:03}");
                let primary = format!("bench/{index:03}/primary");
                let secondary = format!("bench/{index:03}/secondary");
                let tertiary = format!("bench/{index:03}/tertiary");
                let secondary_origin = format!("memory://bench/{index:03}/secondary");
                let tertiary_origin = format!("memory://bench/{index:03}/tertiary");
                TestOfficialModule::from_synthetic_with_document_records(
                    module_with_document(&namespace, &primary, &[]),
                    &[
                        TestSourceDocument {
                            source_document_key: &secondary,
                            source_record: b"secondary",
                            display_source: Some(&secondary_origin),
                        },
                        TestSourceDocument {
                            source_document_key: &tertiary,
                            source_record: b"tertiary",
                            display_source: Some(&tertiary_origin),
                        },
                    ],
                )
            })
            .collect()
    }

    fn run_once(modules: Vec<TestOfficialModule>) -> (u128, AdmissionSizing, u64) {
        let origin_bytes = modules
            .iter()
            .flat_map(|module| module.admitted.source_documents.iter())
            .filter_map(|document| document.origin.display_source.as_ref())
            .fold(0_u64, |total, origin| {
                total.saturating_add(u64::try_from(origin.len()).unwrap_or(u64::MAX))
            });
        let start = Instant::now();
        let limits = CompileLimits::p100_initial_v2();
        let diagnostic_limit = limits.value(CompileLimitDimension::DiagnosticCount);
        let mut builder = CompilationUnitBuilder::new(limits);
        for module in modules {
            builder.add_test_official_module(module).unwrap();
        }
        let totals = builder.totals;
        let unit = builder.build().unwrap();
        let elapsed = start.elapsed().as_nanos();
        let sizing = AdmissionSizing::from_totals(totals, diagnostic_limit);
        let result_live_bytes = black_box(unit.controlled_live_bytes);
        assert_eq!(result_live_bytes, sizing.result_live_bytes);
        black_box(unit);
        (elapsed, sizing, origin_bytes)
    }

    let _warmup = run_once(fixture());
    let mut samples = Vec::with_capacity(SAMPLE_COUNT);
    let mut sizing = AdmissionSizing {
        builder_live_bytes: 0,
        result_live_bytes: 0,
        build_scratch_bytes: 0,
        build_peak_live_bytes: 0,
    };
    let mut origin_bytes = 0_u64;
    for _ in 0..SAMPLE_COUNT {
        let (elapsed, measured_sizing, measured_origin_bytes) = run_once(fixture());
        samples.push(elapsed);
        sizing = measured_sizing;
        origin_bytes = measured_origin_bytes;
    }
    samples.sort_unstable();
    let median_ns = samples[SAMPLE_COUNT / 2];
    let mut deviations = samples
        .iter()
        .map(|sample| sample.abs_diff(median_ns))
        .collect::<Vec<_>>();
    deviations.sort_unstable();
    let mad_ns = deviations[SAMPLE_COUNT / 2];
    println!(
        "admission-only modules={MODULE_COUNT} documents={} warmups=1 samples={SAMPLE_COUNT} median_ns={median_ns} mad_ns={mad_ns} origin_bytes={origin_bytes} builder_live_bytes={} result_live_bytes={} build_scratch_bytes={} admission_peak_live_bytes={}",
        MODULE_COUNT * 3,
        sizing.builder_live_bytes,
        sizing.result_live_bytes,
        sizing.build_scratch_bytes,
        sizing.build_peak_live_bytes,
    );
}

fn geometry_module(namespace: &str, document_key: &str, imports: &[&str]) -> GeometryModule {
    geometry_module_with_frame_ref(namespace, document_key, imports, "frame.main")
}

/// 与 `geometry_module` 同构，但 road 的 frame 引用由调用方给定（可指向导入模块的
/// frame，供跨模块 frame 闭包测试）。
fn geometry_module_with_frame_ref(
    namespace: &str,
    document_key: &str,
    imports: &[&str],
    frame_ref: &str,
) -> GeometryModule {
    geometry_module_with_frame_ref_and_profiles(
        namespace,
        document_key,
        imports,
        frame_ref,
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
    )
}

/// 与 `geometry_module` 同构，但位置/方向配置档由调用方给定（供混用配置档 HIR 检查测试）。
fn geometry_module_with_profiles(
    namespace: &str,
    document_key: &str,
    imports: &[&str],
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
) -> GeometryModule {
    geometry_module_with_frame_ref_and_profiles(
        namespace,
        document_key,
        imports,
        "frame.main",
        accuracy_profile,
        direction_profile,
    )
}

fn geometry_module_with_frame_ref_and_profiles(
    namespace: &str,
    document_key: &str,
    imports: &[&str],
    frame_ref: &str,
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
) -> GeometryModule {
    let imports_json = format!(
        "[{}]",
        imports
            .iter()
            .map(|import| format!("\"{import}\""))
            .collect::<Vec<_>>()
            .join(",")
    );
    let source = format!(
        concat!(
            "{{\"geometryVersion\":\"1\",\"module\":{{\"namespace\":\"{namespace}\",\"documentKey\":\"{document_key}\",",
            "\"imports\":{imports_json},\"provenance\":{{\"kind\":\"direct\",\"description\":\"test\"}}}},",
            "\"units\":{{\"distance\":\"meter\",\"angle\":\"radian\",\"speed\":\"meter-per-second\",\"time\":\"second\"}},",
            "\"frames\":[{{\"frameKey\":\"frame.main\"}}],",
            "\"roads\":[{{\"roadKey\":\"road.main\",\"frame\":\"{frame_ref}\",",
            "\"referenceLine\":{{\"start\":[0,0,0],\"segments\":[{{\"kind\":\"line\",\"end\":[10,0,0]}}]}},",
            "\"crossSectionSpans\":[{{\"spanKey\":\"span.main\",\"corridorKey\":\"corridor.main\",",
            "\"startStationMeters\":0,\"endStationMeters\":\"end\",\"referenceSectionKey\":\"section.main\",",
            "\"referenceLaneKey\":\"lane.main\",\"elements\":[{{\"kind\":\"roadSection\",\"sectionKey\":\"section.main\"}}],",
            "\"roadSections\":[{{\"sectionKey\":\"section.main\",\"kindId\":\"motorLane\",\"lanes\":[{{\"laneKey\":\"lane.main\",",
            "\"laneEdgeKey\":\"edge.main\",\"direction\":\"forward\",\"widthMeters\":3.5,",
            "\"speedLimitMetersPerSecond\":10,\"successors\":[]}}],\"laneGroups\":[]}}],\"facilityBands\":[]}}]}}],",
            "\"junctions\":[],\"overlays\":{{\"signalGroups\":[],\"signalControllers\":[],\"parkingAreas\":[],",
            "\"parkingSpaces\":[],\"participantClasses\":[],\"vehicleProfiles\":[],\"accessRules\":[],\"staticRoutes\":[],",
            "\"stopLines\":[],\"maneuverGates\":[],\"waitingZones\":[]}}}}"
        ),
        namespace = namespace,
        document_key = document_key,
        imports_json = imports_json,
        frame_ref = frame_ref
    );
    GeometryModuleBuilder::new(
        GeometryDocumentInput::new(document_key, source.as_bytes(), None),
        accuracy_profile,
        direction_profile,
        &CompileLimits::p100_initial_v1(),
    )
    .unwrap()
    .finish()
    .unwrap()
}

#[test]
fn compilation_unit_rejects_duplicate_geometry_namespaces_atomically() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_geometry_module(geometry_module("city/a", "doc.a", &[]))
        .unwrap();
    let duplicate =
        expect_diagnostics(builder.add_geometry_module(geometry_module("city/a", "doc.b", &[])));
    assert_eq!(
        duplicate.diagnostics()[0].code(),
        DiagnosticCode::DuplicateModuleNamespace
    );
    assert_eq!(duplicate.diagnostics()[0].related_spans().len(), 1);

    // 混合前端同样按 namespace 查重；失败不消耗构建器状态，修正后可直接重试。
    let duplicate = expect_diagnostics(builder.add_synthetic_module(module_with_document(
        "city/a",
        "doc.c",
        &[],
    )));
    assert_eq!(
        duplicate.diagnostics()[0].code(),
        DiagnosticCode::DuplicateModuleNamespace
    );
    builder
        .add_geometry_module(geometry_module("city/b", "doc.b", &[]))
        .unwrap();
    builder
        .add_synthetic_module(module_with_document("city/c", "doc.c", &[]))
        .unwrap();
    let unit = builder.build().unwrap();
    assert_eq!(unit.module_descriptors().len(), 3);
}

#[test]
fn mixed_frontend_unit_keeps_dependency_order_and_parallel_geometry_payloads() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_geometry_module(geometry_module("city/main", "doc.main", &["city/base"]))
        .unwrap();
    builder
        .add_synthetic_module(module("city/base", &[]))
        .unwrap();
    let unit = builder.build().unwrap();

    assert_eq!(
        unit.module_descriptors()
            .map(SourceModuleDescriptor::authoring_namespace_id)
            .collect::<Vec<_>>(),
        ["city/base", "city/main"]
    );
    assert!(unit.geometry_payloads[0].is_none());
    let payload = unit.geometry_payloads[1].as_ref().unwrap();
    assert_eq!(payload.frozen.lateral_curves.len(), 1);
    assert_eq!(payload.frozen.geometry_point_count, 2);
    assert!(matches!(
        payload.accuracy_profile,
        GeometryAccuracyProfile::Balanced5Cm
    ));
    assert!(matches!(
        payload.direction_profile,
        GeometryDirectionProfile::Balanced2Deg
    ));
}

#[test]
fn geometry_module_with_internal_edges_carries_curves_through_admission() {
    let source = concat!(
        "{\"geometryVersion\":\"1\",\"module\":{\"namespace\":\"city/main\",\"documentKey\":\"doc.main\",",
        "\"imports\":[],\"provenance\":{\"kind\":\"direct\",\"description\":\"test\"}},",
        "\"units\":{\"distance\":\"meter\",\"angle\":\"radian\",\"speed\":\"meter-per-second\",\"time\":\"second\"},",
        "\"frames\":[{\"frameKey\":\"frame.main\"}],",
        "\"roads\":[{\"roadKey\":\"road.main\",\"frame\":\"frame.main\",",
        "\"referenceLine\":{\"start\":[0,0,0],\"segments\":[{\"kind\":\"line\",\"end\":[10,0,0]}]},",
        "\"crossSectionSpans\":[{\"spanKey\":\"span.main\",\"corridorKey\":\"corridor.main\",",
        "\"startStationMeters\":0,\"endStationMeters\":\"end\",\"referenceSectionKey\":\"section.main\",",
        "\"referenceLaneKey\":\"lane.a\",\"elements\":[{\"kind\":\"roadSection\",\"sectionKey\":\"section.main\"}],",
        "\"roadSections\":[{\"sectionKey\":\"section.main\",\"kindId\":\"motorLane\",\"lanes\":[",
        "{\"laneKey\":\"lane.a\",\"laneEdgeKey\":\"edge.a\",\"direction\":\"forward\",\"widthMeters\":3.5,",
        "\"speedLimitMetersPerSecond\":10,\"successors\":[]},",
        "{\"laneKey\":\"lane.b\",\"laneEdgeKey\":\"edge.b\",\"direction\":\"forward\",\"widthMeters\":3.5,",
        "\"speedLimitMetersPerSecond\":10,\"successors\":[]}],\"laneGroups\":[]}],\"facilityBands\":[]}]}],",
        "\"junctions\":[{\"junctionKey\":\"junction.main\",\"approachEdges\":[\"edge.a\",\"edge.b\"],",
        "\"internalEdges\":[{\"laneEdgeKey\":\"edge.internal\",\"speedLimitMetersPerSecond\":8,",
        "\"geometry\":{\"start\":[0,0,0],\"segments\":[{\"kind\":\"line\",\"end\":[5,0,5]}]}}],",
        "\"connections\":[{\"movementKey\":\"movement.main\",\"directedEntryApproachKey\":\"approach.in\",",
        "\"directedExitApproachKey\":\"approach.out\",\"maneuverPathKey\":\"path.main\",",
        "\"entryEdge\":\"edge.a\",\"internalEdgeSequence\":[\"edge.internal\"],\"exitEdge\":\"edge.b\"}]}],",
        "\"overlays\":{\"signalGroups\":[],\"signalControllers\":[],\"parkingAreas\":[],",
        "\"parkingSpaces\":[],\"participantClasses\":[],\"vehicleProfiles\":[],\"accessRules\":[],\"staticRoutes\":[],",
        "\"stopLines\":[],\"maneuverGates\":[],\"waitingZones\":[]}}"
    );
    let module = GeometryModuleBuilder::new(
        GeometryDocumentInput::new("doc.main", source.as_bytes(), None),
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        &CompileLimits::p100_initial_v1(),
    )
    .unwrap()
    .finish()
    .unwrap();
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder.add_geometry_module(module).unwrap();
    let unit = builder.build().unwrap();

    // 冻结载荷随共同 admission 不可分携带：internal 曲线按 (junction, laneEdgeKey) 可枚举。
    let payload = unit.geometry_payloads[0].as_ref().unwrap();
    assert_eq!(payload.frozen.lateral_curves.len(), 2);
    assert_eq!(payload.frozen.internal_edge_curves.len(), 1);
    let curve = &payload.frozen.internal_edge_curves[0];
    assert_eq!(curve.junction_key.as_ref(), "junction.main");
    assert_eq!(curve.lane_edge_key.as_ref(), "edge.internal");
    assert_eq!(curve.points.len(), 2);
    assert_eq!(payload.frozen.geometry_point_count, 6);
}

#[test]
fn road_only_geometry_module_builds_through_hir() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_geometry_module(geometry_module("city/main", "doc.main", &[]))
        .unwrap();
    let unit = builder.build().unwrap();
    let hir = crate::hir::build_hir(&unit).unwrap();

    assert_eq!(hir.canonical_frames.len(), 1);
    assert_eq!(hir.lane_edges.len(), 1);
    assert_eq!(hir.road_corridors.len(), 1);
    assert_eq!(hir.road_sections.len(), 1);
    assert_eq!(hir.authoring_lanes.len(), 1);
}

/// 收集模块全部稳定声明的派生身份输入（实体种类 + 稳定键）；Geometry intent 不产生
/// 稳定声明，不参与身份派生（§4.1）。
fn stable_identity_inputs(
    module: &GeometryModule,
) -> Vec<(laneflow_static_contract::EntityKind, &str)> {
    module
        .admitted
        .declarations
        .iter()
        .filter_map(|declaration| {
            let header = match declaration {
                TypedAstDeclaration::LaneEdge(declaration) => &declaration.header,
                TypedAstDeclaration::RoadCorridor(declaration) => &declaration.header,
                TypedAstDeclaration::RoadSection(declaration) => &declaration.header,
                TypedAstDeclaration::LaneGroup(declaration) => &declaration.header,
                TypedAstDeclaration::FacilityBand(declaration) => &declaration.header,
                TypedAstDeclaration::Junction(declaration) => &declaration.header,
                TypedAstDeclaration::Movement(declaration) => &declaration.header,
                TypedAstDeclaration::ManeuverPath(declaration) => &declaration.header,
                TypedAstDeclaration::StopLine(declaration) => &declaration.header,
                TypedAstDeclaration::ManeuverGate(declaration) => &declaration.header,
                TypedAstDeclaration::WaitingZone(declaration) => &declaration.header,
                TypedAstDeclaration::StaticRoute(declaration) => &declaration.header,
                TypedAstDeclaration::SignalGroup(declaration) => &declaration.header,
                TypedAstDeclaration::SignalController(declaration) => &declaration.header,
                TypedAstDeclaration::ParkingArea(declaration) => &declaration.header,
                TypedAstDeclaration::ParkingSpace(declaration) => &declaration.header,
                TypedAstDeclaration::ParticipantClass(declaration) => &declaration.header,
                TypedAstDeclaration::VehicleProfile(declaration) => &declaration.header,
                TypedAstDeclaration::CanonicalFrame(declaration) => &declaration.header,
                TypedAstDeclaration::AccessRule(declaration) => &declaration.header,
                TypedAstDeclaration::GeometryReferenceLine(_)
                | TypedAstDeclaration::GeometryCrossSectionSpan(_)
                | TypedAstDeclaration::GeometryConnection(_)
                | TypedAstDeclaration::GeometryInternalEdge(_) => return None,
            };
            Some((header.entity_kind, header.stable_key.as_ref()))
        })
        .collect()
}

#[test]
fn geometry_modules_with_matching_profiles_build_hir() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_geometry_module(geometry_module("city/geo.a", "doc.geo.a", &[]))
        .unwrap();
    builder
        .add_geometry_module(geometry_module("city/geo.b", "doc.geo.b", &[]))
        .unwrap();
    let unit = builder.build().unwrap();

    let hir = crate::hir::build_hir(&unit).unwrap();
    assert_eq!(hir.modules.len(), 2);
    assert_eq!(hir.canonical_frames.len(), 2);
    assert_eq!(hir.lane_edges.len(), 2);
}

#[test]
fn geometry_mixed_accuracy_profile_fails_hir_with_single_diagnostic() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_geometry_module(geometry_module_with_profiles(
            "city/geo.a",
            "doc.geo.a",
            &[],
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
        ))
        .unwrap();
    builder
        .add_geometry_module(geometry_module_with_profiles(
            "city/geo.b",
            "doc.geo.b",
            &[],
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Balanced2Deg,
        ))
        .unwrap();
    let unit = builder.build().unwrap();

    let diagnostics = match crate::hir::build_hir(&unit) {
        Ok(_) => panic!("mixed accuracy profiles must reject HIR construction"),
        Err(diagnostics) => diagnostics,
    };
    assert_eq!(diagnostics.diagnostics().len(), 1);
    assert!(!diagnostics.diagnostics_truncated());
    let diagnostic = &diagnostics.diagnostics()[0];
    assert_eq!(
        diagnostic.code(),
        DiagnosticCode::MixedGeometryAccuracyProfile
    );
    assert!(matches!(
        diagnostic.payload(),
        DiagnosticPayload::MixedGeometryAccuracyProfile {
            canonical_namespace,
            conflicting_namespace,
            canonical_profile,
            conflicting_profile,
        } if canonical_namespace.as_ref() == "city/geo.a"
            && conflicting_namespace.as_ref() == "city/geo.b"
            && canonical_profile.as_ref() == "balanced-5cm-v1"
            && conflicting_profile.as_ref() == "fine-2cm-v1"
    ));
    assert_eq!(diagnostic.stable_key(), Some("city/geo.b"));
    assert_eq!(
        diagnostic.primary_span().unwrap().source_document_key(),
        "doc.geo.b"
    );
    assert_eq!(diagnostic.related_spans().len(), 1);
    assert_eq!(
        diagnostic.related_spans()[0].source_document_key(),
        "doc.geo.a"
    );
    assert_eq!(
        diagnostic.to_string(),
        "LF-COMP-MIXED-GEOMETRY-ACCURACY-PROFILE: 编译单元内 Geometry 模块 city/geo.b 混用总位置误差配置档：规范 balanced-5cm-v1（city/geo.a），实际 fine-2cm-v1"
    );
}

#[test]
fn geometry_mixed_direction_profile_fails_hir_with_single_diagnostic() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_geometry_module(geometry_module_with_profiles(
            "city/geo.a",
            "doc.geo.a",
            &[],
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
        ))
        .unwrap();
    builder
        .add_geometry_module(geometry_module_with_profiles(
            "city/geo.b",
            "doc.geo.b",
            &[],
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Smooth1Deg,
        ))
        .unwrap();
    let unit = builder.build().unwrap();

    let diagnostics = match crate::hir::build_hir(&unit) {
        Ok(_) => panic!("mixed direction profiles must reject HIR construction"),
        Err(diagnostics) => diagnostics,
    };
    assert_eq!(diagnostics.diagnostics().len(), 1);
    assert!(!diagnostics.diagnostics_truncated());
    let diagnostic = &diagnostics.diagnostics()[0];
    assert_eq!(
        diagnostic.code(),
        DiagnosticCode::MixedGeometryDirectionProfile
    );
    assert!(matches!(
        diagnostic.payload(),
        DiagnosticPayload::MixedGeometryDirectionProfile {
            canonical_namespace,
            conflicting_namespace,
            canonical_profile,
            conflicting_profile,
        } if canonical_namespace.as_ref() == "city/geo.a"
            && conflicting_namespace.as_ref() == "city/geo.b"
            && canonical_profile.as_ref() == "balanced-2deg-v1"
            && conflicting_profile.as_ref() == "smooth-1deg-v1"
    ));
    assert_eq!(diagnostic.stable_key(), Some("city/geo.b"));
    assert_eq!(
        diagnostic.primary_span().unwrap().source_document_key(),
        "doc.geo.b"
    );
    assert_eq!(
        diagnostic.to_string(),
        "LF-COMP-MIXED-GEOMETRY-DIRECTION-PROFILE: 编译单元内 Geometry 模块 city/geo.b 混用方向跳变配置档：规范 balanced-2deg-v1（city/geo.a），实际 smooth-1deg-v1"
    );
}

#[test]
fn geometry_mixed_accuracy_and_direction_profiles_report_both_diagnostics() {
    // 位置与方向是正交约束（§3），同一模块两者都混用时不互相遮蔽、分别报告；§7.2 同阶段
    // 序在同模块同 span 下按诊断代码排序，accuracy 代码先于 direction。
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_geometry_module(geometry_module_with_profiles(
            "city/geo.a",
            "doc.geo.a",
            &[],
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
        ))
        .unwrap();
    builder
        .add_geometry_module(geometry_module_with_profiles(
            "city/geo.b",
            "doc.geo.b",
            &[],
            GeometryAccuracyProfile::Compact10Cm,
            GeometryDirectionProfile::Compact5Deg,
        ))
        .unwrap();
    let unit = builder.build().unwrap();

    let diagnostics = match crate::hir::build_hir(&unit) {
        Ok(_) => panic!("mixed profiles must reject HIR construction"),
        Err(diagnostics) => diagnostics,
    };
    assert_eq!(
        diagnostics
            .diagnostics()
            .iter()
            .map(Diagnostic::code)
            .collect::<Vec<_>>(),
        [
            DiagnosticCode::MixedGeometryAccuracyProfile,
            DiagnosticCode::MixedGeometryDirectionProfile,
        ]
    );
    assert!(diagnostics.diagnostics().iter().all(|diagnostic| {
        diagnostic.stable_key() == Some("city/geo.b")
            && diagnostic.primary_span().unwrap().source_document_key() == "doc.geo.b"
            && diagnostic.related_spans()[0].source_document_key() == "doc.geo.a"
    }));
}

#[test]
fn geometry_mixed_profile_reports_only_conflicting_module_against_canonical() {
    // 前两模块同档、第三模块冲突：只报告第三模块相对规范（首个 Geometry 模块）的混用。
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_geometry_module(geometry_module("city/geo.a", "doc.geo.a", &[]))
        .unwrap();
    builder
        .add_geometry_module(geometry_module("city/geo.b", "doc.geo.b", &[]))
        .unwrap();
    builder
        .add_geometry_module(geometry_module_with_profiles(
            "city/geo.c",
            "doc.geo.c",
            &[],
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Balanced2Deg,
        ))
        .unwrap();
    let unit = builder.build().unwrap();

    let diagnostics = match crate::hir::build_hir(&unit) {
        Ok(_) => panic!("mixed accuracy profiles must reject HIR construction"),
        Err(diagnostics) => diagnostics,
    };
    assert_eq!(diagnostics.diagnostics().len(), 1);
    let diagnostic = &diagnostics.diagnostics()[0];
    assert_eq!(
        diagnostic.code(),
        DiagnosticCode::MixedGeometryAccuracyProfile
    );
    assert!(matches!(
        diagnostic.payload(),
        DiagnosticPayload::MixedGeometryAccuracyProfile {
            canonical_namespace,
            conflicting_namespace,
            ..
        } if canonical_namespace.as_ref() == "city/geo.a"
            && conflicting_namespace.as_ref() == "city/geo.c"
    ));
    assert_eq!(diagnostic.stable_key(), Some("city/geo.c"));
    assert_eq!(
        diagnostic.primary_span().unwrap().source_document_key(),
        "doc.geo.c"
    );
    assert_eq!(
        diagnostic.related_spans()[0].source_document_key(),
        "doc.geo.a"
    );
}

#[test]
fn geometry_and_synthetic_mixed_unit_with_matching_profiles_builds_hir() {
    // 同档 Geometry 多模块与 Synthetic 模块共存：Synthetic 的 payload 槽位为 None，
    // 不参与混用比较，也不携带虚构配置档。
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_synthetic_module(synthetic_edge_module("city/base"))
        .unwrap();
    builder
        .add_geometry_module(geometry_module("city/geo.a", "doc.geo.a", &[]))
        .unwrap();
    builder
        .add_geometry_module(geometry_module("city/geo.b", "doc.geo.b", &[]))
        .unwrap();
    let unit = builder.build().unwrap();
    assert!(unit.geometry_payloads[0].is_none());

    let hir = crate::hir::build_hir(&unit).unwrap();
    assert_eq!(hir.modules.len(), 3);
    assert_eq!(hir.canonical_frames.len(), 3);
}

#[test]
fn geometry_profile_change_keeps_stable_identity_inputs() {
    // 同一文档在不同配置档下 finish：稳定声明键与派生身份输入完全一致，仅描述符
    // 配置档摘要与几何数值不同（§3：配置档差异不改变实体稳定标识）。
    let fine = geometry_module_with_profiles(
        "city/main",
        "doc.main",
        &[],
        GeometryAccuracyProfile::Fine2Cm,
        GeometryDirectionProfile::Smooth1Deg,
    );
    let compact = geometry_module_with_profiles(
        "city/main",
        "doc.main",
        &[],
        GeometryAccuracyProfile::Compact10Cm,
        GeometryDirectionProfile::Compact5Deg,
    );

    let fine_identities = stable_identity_inputs(&fine);
    assert!(!fine_identities.is_empty());
    assert_eq!(fine_identities, stable_identity_inputs(&compact));
    assert_eq!(
        fine.descriptor().source_document_set_digest(),
        compact.descriptor().source_document_set_digest()
    );
    assert_ne!(
        fine.descriptor().frontend_options_digest(),
        compact.descriptor().frontend_options_digest()
    );
}

#[test]
fn geometry_mixed_profile_failure_is_atomic_and_repeatable() {
    // 混用失败不返回部分输出：build_hir 只借用单元，失败后单元状态不变、重复检查得到
    // 相同诊断；compile 消费单元时同样原子失败，不暴露部分输出。
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_geometry_module(geometry_module_with_profiles(
            "city/geo.a",
            "doc.geo.a",
            &[],
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
        ))
        .unwrap();
    builder
        .add_geometry_module(geometry_module_with_profiles(
            "city/geo.b",
            "doc.geo.b",
            &[],
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Balanced2Deg,
        ))
        .unwrap();
    let unit = builder.build().unwrap();

    let first = match crate::hir::build_hir(&unit) {
        Ok(_) => panic!("mixed accuracy profiles must reject HIR construction"),
        Err(diagnostics) => diagnostics,
    };
    let second = match crate::hir::build_hir(&unit) {
        Ok(_) => panic!("mixed accuracy profiles must reject HIR construction"),
        Err(diagnostics) => diagnostics,
    };
    for diagnostics in [&first, &second] {
        assert_eq!(diagnostics.diagnostics().len(), 1);
        assert_eq!(
            diagnostics.diagnostics()[0].code(),
            DiagnosticCode::MixedGeometryAccuracyProfile
        );
    }
    // 失败不消耗单元状态：模块与 geometry payload 完整保留。
    assert_eq!(unit.modules.len(), 2);
    assert!(unit.geometry_payloads.iter().all(Option::is_some));

    let diagnostics = crate::Compiler::new()
        .compile(unit)
        .err()
        .expect("mixed accuracy profiles must fail the whole compile");
    assert_eq!(diagnostics.diagnostics().len(), 1);
    assert_eq!(
        diagnostics.diagnostics()[0].code(),
        DiagnosticCode::MixedGeometryAccuracyProfile
    );
}

fn quoted_csv(items: &[&str]) -> String {
    items
        .iter()
        .map(|item| format!("\"{item}\""))
        .collect::<Vec<_>>()
        .join(",")
}

fn geometry_lane_fragment(lane_key: &str, edge_key: &str, successors: &[&str]) -> String {
    format!(
        concat!(
            "{{\"laneKey\":\"{lane_key}\",\"laneEdgeKey\":\"{edge_key}\",\"direction\":\"forward\",",
            "\"widthMeters\":3.5,\"speedLimitMetersPerSecond\":10,\"successors\":[{successors}]}}"
        ),
        lane_key = lane_key,
        edge_key = edge_key,
        successors = quoted_csv(successors),
    )
}

/// 与 `geometry_lane_fragment` 同构，但 `widthMeters` 由调用方给定：known-vector
/// 用非正宽度注入阶段 3 局部值域错误（`InvalidWidth`）。
fn geometry_lane_fragment_with_width(
    lane_key: &str,
    edge_key: &str,
    successors: &[&str],
    width_meters: f64,
) -> String {
    format!(
        concat!(
            "{{\"laneKey\":\"{lane_key}\",\"laneEdgeKey\":\"{edge_key}\",\"direction\":\"forward\",",
            "\"widthMeters\":{width_meters},\"speedLimitMetersPerSecond\":10,\"successors\":[{successors}]}}"
        ),
        lane_key = lane_key,
        edge_key = edge_key,
        width_meters = width_meters,
        successors = quoted_csv(successors),
    )
}

fn geometry_internal_edge_fragment_with_polyline(edge_key: &str, points: &[[f64; 3]]) -> String {
    let start = points[0];
    let segments = points[1..]
        .iter()
        .map(|end| {
            format!(
                "{{\"kind\":\"line\",\"end\":[{},{},{}]}}",
                end[0], end[1], end[2]
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        concat!(
            "{{\"laneEdgeKey\":\"{edge_key}\",\"speedLimitMetersPerSecond\":8,",
            "\"geometry\":{{\"start\":[{},{},{}],\"segments\":[{segments}]}}}}"
        ),
        start[0],
        start[1],
        start[2],
        edge_key = edge_key,
        segments = segments,
    )
}

fn geometry_connection_fragment(
    movement_key: &str,
    path_key: &str,
    entry_edge: &str,
    internal_sequence: &[&str],
    exit_edge: &str,
) -> String {
    format!(
        concat!(
            "{{\"movementKey\":\"{movement_key}\",\"directedEntryApproachKey\":\"approach.in\",",
            "\"directedExitApproachKey\":\"approach.out\",\"maneuverPathKey\":\"{path_key}\",",
            "\"entryEdge\":\"{entry_edge}\",\"internalEdgeSequence\":[{sequence}],",
            "\"exitEdge\":\"{exit_edge}\"}}"
        ),
        movement_key = movement_key,
        path_key = path_key,
        entry_edge = entry_edge,
        sequence = quoted_csv(internal_sequence),
        exit_edge = exit_edge,
    )
}

fn geometry_junction_fragment(
    junction_key: &str,
    approach_edges: &[&str],
    internal_edges: &[String],
    connections: &[String],
) -> String {
    format!(
        concat!(
            "{{\"junctionKey\":\"{junction_key}\",\"approachEdges\":[{approaches}],",
            "\"internalEdges\":[{internal_edges}],\"connections\":[{connections}]}}"
        ),
        junction_key = junction_key,
        approaches = quoted_csv(approach_edges),
        internal_edges = internal_edges.join(","),
        connections = connections.join(","),
    )
}

/// 单条 road 片段：reference line 为 start→end 单线段直线。span/corridor/section 键取
/// road 键 `road.` 前缀后的后缀（`road.main` → `span.main`），与既有单 road 文档一致。
fn geometry_road_fragment(
    road_key: &str,
    frame_ref: &str,
    reference_start: [f64; 3],
    reference_end: [f64; 3],
    reference_lane_key: &str,
    lanes: &[String],
) -> String {
    let suffix = road_key.strip_prefix("road.").unwrap_or(road_key);
    format!(
        concat!(
            "{{\"roadKey\":\"{road_key}\",\"frame\":\"{frame_ref}\",",
            "\"referenceLine\":{{\"start\":[{},{},{}],\"segments\":[{{\"kind\":\"line\",",
            "\"end\":[{},{},{}]}}]}},",
            "\"crossSectionSpans\":[{{\"spanKey\":\"span.{suffix}\",\"corridorKey\":\"corridor.{suffix}\",",
            "\"startStationMeters\":0,\"endStationMeters\":\"end\",\"referenceSectionKey\":\"section.{suffix}\",",
            "\"referenceLaneKey\":\"{reference_lane_key}\",\"elements\":[{{\"kind\":\"roadSection\",",
            "\"sectionKey\":\"section.{suffix}\"}}],\"roadSections\":[{{\"sectionKey\":\"section.{suffix}\",",
            "\"kindId\":\"motorLane\",\"lanes\":[{lanes}],\"laneGroups\":[]}}],\"facilityBands\":[]}}]}}"
        ),
        reference_start[0],
        reference_start[1],
        reference_start[2],
        reference_end[0],
        reference_end[1],
        reference_end[2],
        road_key = road_key,
        frame_ref = frame_ref,
        suffix = suffix,
        reference_lane_key = reference_lane_key,
        lanes = lanes.join(","),
    )
}

/// 单条 road 片段（含一条 facility band）：elements 为 [roadSection, facilityBand]，
/// facility band 沿车道外侧排布；kindId 固定 `sidewalk`。
#[allow(clippy::too_many_arguments)]
fn geometry_road_fragment_with_facility_band(
    road_key: &str,
    frame_ref: &str,
    reference_start: [f64; 3],
    reference_end: [f64; 3],
    reference_lane_key: &str,
    lanes: &[String],
    facility_band_key: &str,
    facility_width: f64,
) -> String {
    let suffix = road_key.strip_prefix("road.").unwrap_or(road_key);
    format!(
        concat!(
            "{{\"roadKey\":\"{road_key}\",\"frame\":\"{frame_ref}\",",
            "\"referenceLine\":{{\"start\":[{},{},{}],\"segments\":[{{\"kind\":\"line\",",
            "\"end\":[{},{},{}]}}]}},",
            "\"crossSectionSpans\":[{{\"spanKey\":\"span.{suffix}\",\"corridorKey\":\"corridor.{suffix}\",",
            "\"startStationMeters\":0,\"endStationMeters\":\"end\",\"referenceSectionKey\":\"section.{suffix}\",",
            "\"referenceLaneKey\":\"{reference_lane_key}\",\"elements\":[{{\"kind\":\"roadSection\",",
            "\"sectionKey\":\"section.{suffix}\"}},{{\"kind\":\"facilityBand\",",
            "\"facilityBandKey\":\"{facility_band_key}\"}}],\"roadSections\":[{{\"sectionKey\":\"section.{suffix}\",",
            "\"kindId\":\"motorLane\",\"lanes\":[{lanes}],\"laneGroups\":[]}}],",
            "\"facilityBands\":[{{\"facilityBandKey\":\"{facility_band_key}\",",
            "\"kindId\":\"sidewalk\",\"widthMeters\":{}}}]}}]}}"
        ),
        reference_start[0],
        reference_start[1],
        reference_start[2],
        reference_end[0],
        reference_end[1],
        reference_end[2],
        facility_width,
        road_key = road_key,
        frame_ref = frame_ref,
        suffix = suffix,
        reference_lane_key = reference_lane_key,
        lanes = lanes.join(","),
        facility_band_key = facility_band_key,
    )
}

/// 带车道与路口的 Geometry 模块：单 frame，roads 与 junctions 内容由调用方给定。
/// 各 road 的 frame 引用、reference line 与 reference lane 由 road 片段自带。
fn geometry_roads_module(
    namespace: &str,
    document_key: &str,
    imports: &[&str],
    roads: &[String],
    junctions: &[String],
) -> GeometryModule {
    geometry_frames_roads_module(
        namespace,
        document_key,
        imports,
        &["frame.main"],
        roads,
        junctions,
    )
}

/// 与 `geometry_roads_module` 同构，但模块声明的 frame 集合由调用方给定，供
/// 多 frame 的路口 frame 闭包测试。
fn geometry_frames_roads_module(
    namespace: &str,
    document_key: &str,
    imports: &[&str],
    frames: &[&str],
    roads: &[String],
    junctions: &[String],
) -> GeometryModule {
    let source =
        geometry_document_source(namespace, document_key, imports, frames, roads, junctions);
    GeometryModuleBuilder::new(
        GeometryDocumentInput::new(document_key, source.as_bytes(), None),
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        &CompileLimits::p100_initial_v1(),
    )
    .unwrap()
    .finish()
    .unwrap()
}

/// `geometry_frames_roads_module` 的文档拼装部分：只返回来源字符串，供预期
/// `finish` 失败的 known-vector 自行驱动 `GeometryModuleBuilder` 并检查诊断。
fn geometry_document_source(
    namespace: &str,
    document_key: &str,
    imports: &[&str],
    frames: &[&str],
    roads: &[String],
    junctions: &[String],
) -> String {
    let frames_json = frames
        .iter()
        .map(|frame| format!("{{\"frameKey\":\"{frame}\"}}"))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        concat!(
            "{{\"geometryVersion\":\"1\",\"module\":{{\"namespace\":\"{namespace}\",\"documentKey\":\"{document_key}\",",
            "\"imports\":[{imports}],\"provenance\":{{\"kind\":\"direct\",\"description\":\"test\"}}}},",
            "\"units\":{{\"distance\":\"meter\",\"angle\":\"radian\",\"speed\":\"meter-per-second\",\"time\":\"second\"}},",
            "\"frames\":[{frames}],",
            "\"roads\":[{roads}],",
            "\"junctions\":[{junctions}],",
            "\"overlays\":{{\"signalGroups\":[],\"signalControllers\":[],\"parkingAreas\":[],",
            "\"parkingSpaces\":[],\"participantClasses\":[],\"vehicleProfiles\":[],\"accessRules\":[],",
            "\"staticRoutes\":[],\"stopLines\":[],\"maneuverGates\":[],\"waitingZones\":[]}}}}"
        ),
        namespace = namespace,
        document_key = document_key,
        imports = quoted_csv(imports),
        frames = frames_json,
        roads = roads.join(","),
        junctions = junctions.join(","),
    )
}

/// 带车道与路口的 Geometry 模块：单 frame/road/span/corridor/section，车道与路口内容由
/// 调用方给定；`referenceLaneKey` 固定 `lane.a`，因此车道列表必须以 `lane.a` 开头。
fn geometry_topology_module(
    namespace: &str,
    document_key: &str,
    imports: &[&str],
    lanes: &[String],
    junctions: &[String],
) -> GeometryModule {
    geometry_roads_module(
        namespace,
        document_key,
        imports,
        &[geometry_road_fragment(
            "road.main",
            "frame.main",
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            "lane.a",
            lanes,
        )],
        junctions,
    )
}

fn geometry_topology_unit(module: GeometryModule) -> CompilationUnit {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder.add_geometry_module(module).unwrap();
    builder.build().unwrap()
}

/// 带一条无 successor 边 `edge.x` 的 Synthetic 模块，供跨前端派生引用测试。
/// `edge.x` 携带 `frame.base` 下的显式中心线（[-10,0,0] → [0,0,0]，与声明长度
/// 10 一致），使含几何的单元满足完整空间覆盖与路口 frame 闭包。
fn synthetic_edge_module(namespace: &str) -> SyntheticModule {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(header(namespace, namespace), &limits).unwrap();
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge.x",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap();
    let centerline = [
        CanonicalPoint3F32Input {
            x: -10.0,
            y: 0.0,
            z: 0.0,
        },
        CanonicalPoint3F32Input {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
    ];
    let geometries = [LaneEdgeGeometryInput {
        lane_edge: LaneEdgeReference::local("edge.x"),
        centerline_points: &centerline,
    }];
    builder
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame.base",
            lane_edge_geometries: &geometries,
        })
        .unwrap();
    builder.finish().unwrap()
}

/// 目标边稳定键与合并后来源位置，按 `lane_edge_references` 中的存储序返回。
fn hir_successors(hir: &crate::hir::HirUnit, edge_key: &str) -> Vec<(String, SourceSpan)> {
    let edge = hir
        .lane_edges
        .iter()
        .find(|edge| edge.stable_key.as_ref() == edge_key)
        .unwrap_or_else(|| panic!("missing lane edge {edge_key}"));
    hir.lane_edge_references[edge.successors.as_usize_range()]
        .iter()
        .map(|reference| {
            (
                hir.lane_edges[reference.target.index()]
                    .stable_key
                    .to_string(),
                reference.source_span.clone(),
            )
        })
        .collect()
}

fn mir_successor_keys(mir: &crate::mir::MirUnit, edge_key: &str) -> Vec<String> {
    let edge = mir
        .lane_edges
        .iter()
        .find(|edge| edge.stable_key.as_ref() == edge_key)
        .unwrap_or_else(|| panic!("missing lane edge {edge_key}"));
    mir.lane_edge_connections[edge.connections.as_usize_range()]
        .iter()
        .map(|connection| {
            mir.lane_edges[connection.target.index()]
                .stable_key
                .to_string()
        })
        .collect()
}

fn connection_intent_span(unit: &CompilationUnit, path_key: &str) -> SourceSpan {
    unit.modules
        .iter()
        .flat_map(|module| module.declarations.iter())
        .find_map(|declaration| match declaration {
            TypedAstDeclaration::GeometryConnection(intent)
                if intent.maneuver_path.declaration_key.as_ref() == path_key =>
            {
                Some(intent.span.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing connection intent for path {path_key}"))
}

fn lane_successor_span(
    unit: &CompilationUnit,
    edge_key: &str,
    successor_index: usize,
) -> SourceSpan {
    unit.modules
        .iter()
        .flat_map(|module| module.declarations.iter())
        .find_map(|declaration| match declaration {
            TypedAstDeclaration::LaneEdge(edge) if edge.header.stable_key.as_ref() == edge_key => {
                Some(edge.successors[successor_index].span.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing lane edge declaration {edge_key}"))
}

#[test]
fn geometry_connection_derives_successors_through_hir_and_mir() {
    // 全部边共线且首尾相接：edge.a [0,0,0]→[10,0,0]，internal [10,0,0]→[13.5,0,0]，
    // edge.b 是第二条 road 的参考车道 [13.5,0,0]→[23.5,0,0]，行进方向同为 +x。
    let unit = geometry_topology_unit(geometry_roads_module(
        "city/main",
        "doc.main",
        &[],
        &[
            geometry_road_fragment(
                "road.a",
                "frame.main",
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                "lane.a",
                &[geometry_lane_fragment("lane.a", "edge.a", &[])],
            ),
            geometry_road_fragment(
                "road.b",
                "frame.main",
                [13.5, 0.0, 0.0],
                [23.5, 0.0, 0.0],
                "lane.b",
                &[geometry_lane_fragment("lane.b", "edge.b", &[])],
            ),
        ],
        &[geometry_junction_fragment(
            "junction.main",
            &["edge.a", "edge.b"],
            &[geometry_internal_edge_fragment_with_polyline(
                "edge.internal",
                &[[10.0, 0.0, 0.0], [13.5, 0.0, 0.0]],
            )],
            &[geometry_connection_fragment(
                "movement.main",
                "path.main",
                "edge.a",
                &["edge.internal"],
                "edge.b",
            )],
        )],
    ));
    let connection_span = connection_intent_span(&unit, "path.main");
    let hir = crate::hir::build_hir(&unit).unwrap();

    // §4.4：权威路径 entry + internal + exit 的每个相邻 pair 派生一条 successor。
    assert_eq!(hir.lane_edges.len(), 3);
    assert_eq!(
        hir_successors(&hir, "edge.a"),
        [("edge.internal".to_string(), connection_span.clone())]
    );
    assert_eq!(
        hir_successors(&hir, "edge.internal"),
        [("edge.b".to_string(), connection_span.clone())]
    );
    assert!(hir_successors(&hir, "edge.b").is_empty());
    assert_eq!(hir.derived_transition_occurrences.len(), 2);
    assert!(
        hir.derived_transition_occurrences
            .iter()
            .all(|occurrence| occurrence.source_span == connection_span)
    );
    // 资源口径：两条 road 各 1 车道的 admission 计数 + geometry_point 6（三条曲线
    // 各 2 点）+ 空间 segment 3（每条曲线 1 段）+ junction approach 引用 2
    // + §4.4 派生 transition 上界 2 × 2（合并引用与 occurrence 各计一条）。
    assert_eq!(hir.hir_record_count, 101);

    let mir = crate::mir::lower_to_mir(&unit, &hir).unwrap();
    assert_eq!(mir_successor_keys(&mir, "edge.a"), ["edge.internal"]);
    assert_eq!(mir_successor_keys(&mir, "edge.internal"), ["edge.b"]);
    assert!(mir_successor_keys(&mir, "edge.b").is_empty());
    assert_eq!(mir.lane_edge_connections.len(), 2);
    assert_eq!(mir.derived_transition_occurrences.len(), 2);
    assert!(mir.derived_transition_occurrences.iter().all(|occurrence| {
        occurrence.source_span == connection_span
            && mir.modules[occurrence.module.index()]
                .authoring_namespace_id
                .as_ref()
                == "city/main"
    }));
}

#[test]
fn geometry_connection_with_empty_internal_sequence_derives_entry_to_exit() {
    // 空 internal 序列派生 entry -> exit 直接接合：两条 road 首尾相接，edge.a 末端
    // [10,0,0] 与 edge.b 起点 [10,0,0] 重合且行进方向同为 +x。
    let unit = geometry_topology_unit(geometry_roads_module(
        "city/main",
        "doc.main",
        &[],
        &[
            geometry_road_fragment(
                "road.a",
                "frame.main",
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                "lane.a",
                &[geometry_lane_fragment("lane.a", "edge.a", &[])],
            ),
            geometry_road_fragment(
                "road.b",
                "frame.main",
                [10.0, 0.0, 0.0],
                [20.0, 0.0, 0.0],
                "lane.b",
                &[geometry_lane_fragment("lane.b", "edge.b", &[])],
            ),
        ],
        &[geometry_junction_fragment(
            "junction.main",
            &["edge.a", "edge.b"],
            &[],
            &[geometry_connection_fragment(
                "movement.main",
                "path.main",
                "edge.a",
                &[],
                "edge.b",
            )],
        )],
    ));
    let connection_span = connection_intent_span(&unit, "path.main");
    let hir = crate::hir::build_hir(&unit).unwrap();

    // 空 internal 序列的权威路径退化为 entry + exit，恰好派生一条 entry -> exit。
    assert_eq!(
        hir_successors(&hir, "edge.a"),
        [("edge.b".to_string(), connection_span.clone())]
    );
    assert!(hir_successors(&hir, "edge.b").is_empty());
    assert_eq!(hir.derived_transition_occurrences.len(), 1);
    assert_eq!(
        hir.derived_transition_occurrences[0].source_span,
        connection_span
    );

    let mir = crate::mir::lower_to_mir(&unit, &hir).unwrap();
    assert_eq!(mir_successor_keys(&mir, "edge.a"), ["edge.b"]);
    assert_eq!(mir.derived_transition_occurrences.len(), 1);
}

#[test]
fn geometry_connection_merges_explicit_and_derived_successors_in_canonical_order() {
    // 全部边共线且首尾相接：lane.c 所在 road 的起点与 edge.a 末端 [10,0,0] 重合
    //（显式 successor 接合），internal 接合 edge.a 与第二条 road 的 lane.b。
    let unit = geometry_topology_unit(geometry_roads_module(
        "city/main",
        "doc.main",
        &[],
        &[
            geometry_road_fragment(
                "road.a",
                "frame.main",
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                "lane.a",
                &[geometry_lane_fragment("lane.a", "edge.a", &["edge.c"])],
            ),
            geometry_road_fragment(
                "road.b",
                "frame.main",
                [13.5, 0.0, 0.0],
                [23.5, 0.0, 0.0],
                "lane.b",
                &[geometry_lane_fragment("lane.b", "edge.b", &[])],
            ),
            geometry_road_fragment(
                "road.c",
                "frame.main",
                [10.0, 0.0, 0.0],
                [20.0, 0.0, 0.0],
                "lane.c",
                &[geometry_lane_fragment("lane.c", "edge.c", &[])],
            ),
        ],
        &[geometry_junction_fragment(
            "junction.main",
            &["edge.a", "edge.b"],
            &[geometry_internal_edge_fragment_with_polyline(
                "edge.internal",
                &[[10.0, 0.0, 0.0], [13.5, 0.0, 0.0]],
            )],
            &[geometry_connection_fragment(
                "movement.main",
                "path.main",
                "edge.a",
                &["edge.internal"],
                "edge.b",
            )],
        )],
    ));
    let explicit_span = lane_successor_span(&unit, "edge.a", 0);
    let connection_span = connection_intent_span(&unit, "path.main");
    let hir = crate::hir::build_hir(&unit).unwrap();

    // §4.4：受影响边的最终 successors = 显式普通 successor ∪ 派生 transition，按
    // (module namespace, declaration key) 合并进同一张 `lane_edge_references`；
    // 显式项保留自身引用位置，派生项携带 connection 记录位置。
    assert_eq!(
        hir_successors(&hir, "edge.a"),
        [
            ("edge.c".to_string(), explicit_span),
            ("edge.internal".to_string(), connection_span),
        ]
    );
    assert_eq!(
        hir_successors(&hir, "edge.internal")
            .into_iter()
            .map(|(key, _)| key)
            .collect::<Vec<_>>(),
        ["edge.b"]
    );

    let mir = crate::mir::lower_to_mir(&unit, &hir).unwrap();
    assert_eq!(
        mir_successor_keys(&mir, "edge.a"),
        ["edge.c", "edge.internal"]
    );
}

#[test]
fn geometry_plain_successor_and_path_derived_conflict_fails() {
    let unit = geometry_topology_unit(geometry_topology_module(
        "city/main",
        "doc.main",
        &[],
        &[
            geometry_lane_fragment("lane.a", "edge.a", &["edge.b"]),
            geometry_lane_fragment("lane.b", "edge.b", &[]),
        ],
        &[geometry_junction_fragment(
            "junction.main",
            &["edge.a", "edge.b"],
            &[],
            &[geometry_connection_fragment(
                "movement.main",
                "path.main",
                "edge.a",
                &[],
                "edge.b",
            )],
        )],
    ));
    let connection_span = connection_intent_span(&unit, "path.main");
    let explicit_span = lane_successor_span(&unit, "edge.a", 0);
    let bundle = expect_diagnostics(crate::hir::build_hir(&unit));

    // §4.4 互斥来源：同一有向 transition 同时来自普通 successor 与连接路径时失败关闭；
    // primary 是 connection 记录位置，related 是显式 successor 引用位置。
    assert_eq!(bundle.diagnostics().len(), 1);
    let diagnostic = &bundle.diagnostics()[0];
    assert_eq!(
        diagnostic.code(),
        DiagnosticCode::LaneEdgeSuccessorPathConflict
    );
    assert_eq!(diagnostic.primary_span(), Some(&connection_span));
    assert_eq!(diagnostic.related_spans(), [explicit_span].as_slice());
    assert_eq!(diagnostic.stable_key(), Some("edge.a"));
    let DiagnosticPayload::LaneEdgeSuccessorPathConflict {
        edge_key,
        successor_key,
        junction_key,
        maneuver_path_key,
    } = diagnostic.payload()
    else {
        panic!("expected LaneEdgeSuccessorPathConflict payload");
    };
    assert_eq!(edge_key.as_ref(), "edge.a");
    assert_eq!(successor_key.as_ref(), "edge.b");
    assert_eq!(junction_key.as_ref(), "junction.main");
    assert_eq!(maneuver_path_key.as_ref(), "path.main");
}

#[test]
fn geometry_same_transition_from_different_junctions_fails() {
    let unit = geometry_topology_unit(geometry_topology_module(
        "city/main",
        "doc.main",
        &[],
        &[
            geometry_lane_fragment("lane.a", "edge.a", &[]),
            geometry_lane_fragment("lane.b", "edge.b", &[]),
        ],
        &[
            geometry_junction_fragment(
                "junction.one",
                &["edge.a", "edge.b"],
                &[],
                &[geometry_connection_fragment(
                    "movement.one",
                    "path.one",
                    "edge.a",
                    &[],
                    "edge.b",
                )],
            ),
            geometry_junction_fragment(
                "junction.two",
                &["edge.a", "edge.b"],
                &[],
                &[geometry_connection_fragment(
                    "movement.two",
                    "path.two",
                    "edge.a",
                    &[],
                    "edge.b",
                )],
            ),
        ],
    ));
    let first_span = connection_intent_span(&unit, "path.one");
    let duplicate_span = connection_intent_span(&unit, "path.two");
    let bundle = expect_diagnostics(crate::hir::build_hir(&unit));

    // §4.4：同一有向 transition 只能由唯一 Junction 的连接路径派生。derive 阶段失败使
    // 路口子阶段不再执行（既有 staged fail-fast），因此只有一条冲突诊断；canonical
    // 首簇（junction.one）进入 related，冲突簇（junction.two）是 primary。
    assert_eq!(bundle.diagnostics().len(), 1);
    let diagnostic = &bundle.diagnostics()[0];
    assert_eq!(
        diagnostic.code(),
        DiagnosticCode::DerivedTransitionJunctionConflict
    );
    assert_eq!(diagnostic.primary_span(), Some(&duplicate_span));
    assert_eq!(diagnostic.related_spans(), [first_span].as_slice());
    assert_eq!(diagnostic.stable_key(), Some("edge.a"));
    let DiagnosticPayload::DerivedTransitionJunctionConflict {
        predecessor_key,
        successor_key,
        first_junction_key,
        duplicate_junction_key,
    } = diagnostic.payload()
    else {
        panic!("expected DerivedTransitionJunctionConflict payload");
    };
    assert_eq!(predecessor_key.as_ref(), "edge.a");
    assert_eq!(successor_key.as_ref(), "edge.b");
    assert_eq!(first_junction_key.as_ref(), "junction.one");
    assert_eq!(duplicate_junction_key.as_ref(), "junction.two");
}

#[test]
fn geometry_shared_internal_edge_dedups_same_junction_occurrences() {
    // 两条连接共享同一 internal edge：lane.b/lane.c 各为一条独立 road 的参考车道，
    // 两条曲线起点同为 internal 末端 [13.5,0,0]，两个 exit 接合都成立。
    let unit = geometry_topology_unit(geometry_roads_module(
        "city/main",
        "doc.main",
        &[],
        &[
            geometry_road_fragment(
                "road.a",
                "frame.main",
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                "lane.a",
                &[geometry_lane_fragment("lane.a", "edge.a", &[])],
            ),
            geometry_road_fragment(
                "road.b",
                "frame.main",
                [13.5, 0.0, 0.0],
                [23.5, 0.0, 0.0],
                "lane.b",
                &[geometry_lane_fragment("lane.b", "edge.b", &[])],
            ),
            geometry_road_fragment(
                "road.c",
                "frame.main",
                [13.5, 0.0, 0.0],
                [23.5, 0.0, 0.0],
                "lane.c",
                &[geometry_lane_fragment("lane.c", "edge.c", &[])],
            ),
        ],
        &[geometry_junction_fragment(
            "junction.main",
            &["edge.a", "edge.b", "edge.c"],
            &[geometry_internal_edge_fragment_with_polyline(
                "edge.internal",
                &[[10.0, 0.0, 0.0], [13.5, 0.0, 0.0]],
            )],
            &[
                geometry_connection_fragment(
                    "movement.one",
                    "path.one",
                    "edge.a",
                    &["edge.internal"],
                    "edge.b",
                ),
                geometry_connection_fragment(
                    "movement.two",
                    "path.two",
                    "edge.a",
                    &["edge.internal"],
                    "edge.c",
                ),
            ],
        )],
    ));
    let first_span = connection_intent_span(&unit, "path.one");
    let second_span = connection_intent_span(&unit, "path.two");
    let hir = crate::hir::build_hir(&unit).unwrap();

    // §4.4 规范去重：同一 (junction identity, predecessor, successor) 只合并进一条
    // successor，来源位置取最小 occurrence span；全部 occurrence span 保留在侧表。
    let merged = hir_successors(&hir, "edge.a");
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].0, "edge.internal");
    assert_eq!(merged[0].1, first_span.clone().min(second_span.clone()));
    assert_eq!(
        hir_successors(&hir, "edge.internal")
            .into_iter()
            .map(|(key, _)| key)
            .collect::<Vec<_>>(),
        ["edge.b", "edge.c"]
    );
    assert_eq!(hir.derived_transition_occurrences.len(), 4);
    let shared: Vec<&SourceSpan> = hir
        .derived_transition_occurrences
        .iter()
        .filter(|occurrence| {
            hir.lane_edges[occurrence.predecessor.index()]
                .stable_key
                .as_ref()
                == "edge.a"
        })
        .map(|occurrence| &occurrence.source_span)
        .collect();
    assert_eq!(shared.len(), 2);
    assert_ne!(first_span, second_span);
    assert!(shared.contains(&&first_span));
    assert!(shared.contains(&&second_span));

    let mir = crate::mir::lower_to_mir(&unit, &hir).unwrap();
    assert_eq!(mir_successor_keys(&mir, "edge.a"), ["edge.internal"]);
    assert_eq!(
        mir_successor_keys(&mir, "edge.internal"),
        ["edge.b", "edge.c"]
    );
    assert_eq!(mir.derived_transition_occurrences.len(), 4);
}

#[test]
fn geometry_connection_references_imported_edges_across_frontends() {
    // 路口 frame 闭包要求全部 approach 绑定同一 frame：city/geo 与 city/main 的 road
    // 都引用导入的 city/base::frame.base，internal edge 的几何因此落在导入 frame 上。
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_synthetic_module(synthetic_edge_module("city/base"))
        .unwrap();
    builder
        .add_geometry_module(geometry_module_with_frame_ref(
            "city/geo",
            "doc.geo",
            &["city/base"],
            "city/base::frame.base",
        ))
        .unwrap();
    builder
        .add_geometry_module(geometry_roads_module(
            "city/main",
            "doc.main",
            &["city/base", "city/geo"],
            &[
                geometry_road_fragment(
                    "road.a",
                    "city/base::frame.base",
                    [3.5, 0.0, 0.0],
                    [13.5, 0.0, 0.0],
                    "lane.a",
                    &[geometry_lane_fragment("lane.a", "edge.a", &[])],
                ),
                geometry_road_fragment(
                    "road.b",
                    "city/base::frame.base",
                    [13.5, 0.0, 0.0],
                    [23.5, 0.0, 0.0],
                    "lane.b",
                    &[geometry_lane_fragment("lane.b", "edge.b", &[])],
                ),
            ],
            &[geometry_junction_fragment(
                "junction.main",
                &[
                    "city/base::edge.x",
                    "city/geo::edge.main",
                    "edge.a",
                    "edge.b",
                ],
                &[
                    // 全部边共线且首尾相接：i1 接合 edge.x 末端 [0,0,0] 与 edge.a
                    // 起点 [3.5,0,0]；i2 接合 edge.main 末端 [10,0,0] 与 edge.b
                    // 起点 [13.5,0,0]。
                    geometry_internal_edge_fragment_with_polyline(
                        "edge.i1",
                        &[[0.0, 0.0, 0.0], [3.5, 0.0, 0.0]],
                    ),
                    geometry_internal_edge_fragment_with_polyline(
                        "edge.i2",
                        &[[10.0, 0.0, 0.0], [13.5, 0.0, 0.0]],
                    ),
                ],
                &[
                    geometry_connection_fragment(
                        "movement.one",
                        "path.one",
                        "city/base::edge.x",
                        &["edge.i1"],
                        "edge.a",
                    ),
                    geometry_connection_fragment(
                        "movement.two",
                        "path.two",
                        "city/geo::edge.main",
                        &["edge.i2"],
                        "edge.b",
                    ),
                ],
            )],
        ))
        .unwrap();
    let unit = builder.build().unwrap();
    let hir = crate::hir::build_hir(&unit).unwrap();

    // 派生 transition 的起始边可属于其它模块乃至另一前端；occurrence 的模块归属始终是
    // 声明该 connection 的模块，不能由起始边归属反推。
    let successor_target = |edge_key: &str| {
        let edge = hir
            .lane_edges
            .iter()
            .find(|edge| edge.stable_key.as_ref() == edge_key)
            .unwrap_or_else(|| panic!("missing lane edge {edge_key}"));
        let reference = &hir.lane_edge_references[edge.successors.as_usize_range()][0];
        let target = &hir.lane_edges[reference.target.index()];
        (
            hir.modules[target.module.index()]
                .authoring_namespace_id
                .to_string(),
            target.stable_key.to_string(),
        )
    };
    assert_eq!(
        successor_target("edge.x"),
        ("city/main".to_string(), "edge.i1".to_string())
    );
    assert_eq!(
        successor_target("edge.main"),
        ("city/main".to_string(), "edge.i2".to_string())
    );
    assert_eq!(
        successor_target("edge.i1"),
        ("city/main".to_string(), "edge.a".to_string())
    );
    assert_eq!(
        successor_target("edge.i2"),
        ("city/main".to_string(), "edge.b".to_string())
    );
    assert_eq!(hir.derived_transition_occurrences.len(), 4);
    assert!(hir.derived_transition_occurrences.iter().all(|occurrence| {
        hir.modules[occurrence.module.index()]
            .authoring_namespace_id
            .as_ref()
            == "city/main"
    }));

    let mir = crate::mir::lower_to_mir(&unit, &hir).unwrap();
    assert_eq!(mir_successor_keys(&mir, "edge.x"), ["edge.i1"]);
    assert_eq!(mir_successor_keys(&mir, "edge.main"), ["edge.i2"]);
    assert_eq!(mir.derived_transition_occurrences.len(), 4);
}

#[test]
fn synthetic_only_unit_has_no_derived_transition_occurrences() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_synthetic_module(signal_module_with_document("city/main", "doc.main"))
        .unwrap();
    let unit = builder.build().unwrap();
    let hir = crate::hir::build_hir(&unit).unwrap();

    // 无 Geometry 连接意图时派生输入为空，successor 合并退化为既有显式路径。
    assert!(hir.derived_transition_occurrences.is_empty());
    assert_eq!(
        hir_successors(&hir, "entry")
            .into_iter()
            .map(|(key, _)| key)
            .collect::<Vec<_>>(),
        ["middle"]
    );
    assert_eq!(
        hir_successors(&hir, "middle")
            .into_iter()
            .map(|(key, _)| key)
            .collect::<Vec<_>>(),
        ["exit"]
    );

    let mir = crate::mir::lower_to_mir(&unit, &hir).unwrap();
    assert!(mir.derived_transition_occurrences.is_empty());
    assert_eq!(mir_successor_keys(&mir, "entry"), ["middle"]);
    assert_eq!(mir_successor_keys(&mir, "middle"), ["exit"]);
}

/// 带一条无 successor 边 `edge.x`、但不声明任何 canonical frame/中心线的 Synthetic
/// 模块，供混合单元的绑定缺失与路口 frame 闭包测试。
fn synthetic_unbound_edge_module(namespace: &str) -> SyntheticModule {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(header(namespace, namespace), &limits).unwrap();
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge.x",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap();
    builder.finish().unwrap()
}

/// 带一条 Synthetic facility band 的模块：`edge.s` 携带 `frame.base` 下的显式中心线
///（[0,0,0] → [10,0,0]，与声明长度 10 一致），band.s 经 section.s/corridor.s 挂载。
/// Synthetic 声明的 band 进入全局 facility 表但不产生几何行（§5.4 稀疏表）。
fn synthetic_facility_module(namespace: &str) -> SyntheticModule {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(header(namespace, namespace), &limits).unwrap();
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge.s",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap();
    let centerline = [
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
    let geometries = [LaneEdgeGeometryInput {
        lane_edge: LaneEdgeReference::local("edge.s"),
        centerline_points: &centerline,
    }];
    builder
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame.base",
            lane_edge_geometries: &geometries,
        })
        .unwrap();
    builder
        .add_facility_band(FacilityBandInput {
            facility_band_key: "band.s",
            kind_id: "sidewalk",
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "section.s",
            kind_id: "motorLane",
            lanes: &[AuthoringLaneInput {
                authoring_lane_key: "lane.s",
                edge_chain: &[LaneEdgeReference::local("edge.s")],
                lane_group: None,
            }],
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "corridor.s",
            reference_section: RoadSectionReference::local("section.s"),
            elements: &[
                CorridorElementReference::facility_band(FacilityBandReference::local("band.s")),
                CorridorElementReference::road_section(RoadSectionReference::local("section.s")),
            ],
        })
        .unwrap();
    builder.finish().unwrap()
}

/// 从 LIR lane edge 视图的规范身份字段取 LaneEdgeKey 字符串。
fn lir_lane_edge_key(edge: &CanonicalLaneEdgeView<'_>) -> String {
    edge.identity_fields()
        .find(|field| field.tag() == laneflow_static_contract::FieldTag::LaneEdgeKey)
        .map(|field| String::from_utf8(field.value_bytes().to_vec()).unwrap())
        .expect("lane edge must carry the LaneEdgeKey identity field")
}

#[test]
fn geometry_spatial_binds_lane_and_internal_curves_bit_exact() {
    // 三条共线相接的直线边：edge.a/edge.b 各为一条 road 的参考车道，edge.internal
    // 为路口内部边；全部落在同一 frame.main 下。
    let unit = geometry_topology_unit(geometry_roads_module(
        "city/main",
        "doc.main",
        &[],
        &[
            geometry_road_fragment(
                "road.a",
                "frame.main",
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                "lane.a",
                &[geometry_lane_fragment("lane.a", "edge.a", &[])],
            ),
            geometry_road_fragment(
                "road.b",
                "frame.main",
                [13.5, 0.0, 0.0],
                [23.5, 0.0, 0.0],
                "lane.b",
                &[geometry_lane_fragment("lane.b", "edge.b", &[])],
            ),
        ],
        &[geometry_junction_fragment(
            "junction.main",
            &["edge.a", "edge.b"],
            &[geometry_internal_edge_fragment_with_polyline(
                "edge.internal",
                &[[10.0, 0.0, 0.0], [13.5, 0.0, 0.0]],
            )],
            &[geometry_connection_fragment(
                "movement.main",
                "path.main",
                "edge.a",
                &["edge.internal"],
                "edge.b",
            )],
        )],
    ));
    let hir = crate::hir::build_hir(&unit).unwrap();

    // §5.4/§6.2：每条 geometry 边一行几何，按 frame 聚簇；单 frame 时保持派生追加序
    //（road.a 车道、road.b 车道、路口内部边），点与 segment 区间依同一顺序排布。
    assert_eq!(hir.canonical_frames.len(), 1);
    assert_eq!(
        hir.canonical_frames[0]
            .lane_edge_geometries
            .as_usize_range(),
        0..3
    );
    assert_eq!(hir.canonical_points.len(), 6);
    assert_eq!(hir.spatial_segments.len(), 3);
    assert!(hir.facility_band_geometries.is_empty());

    let payload = unit.geometry_payloads[0].as_ref().unwrap();
    let lane_curve = |span_key: &str, lane_key: &str| -> &[FrozenCanonicalPoint] {
        &payload
            .frozen
            .lateral_curves
            .iter()
            .find(|curve| {
                curve.kind != LateralIntentKind::FacilityBand
                    && curve.span_key.as_ref() == span_key
                    && curve.key.as_ref() == lane_key
            })
            .unwrap_or_else(|| panic!("missing lane curve {span_key}/{lane_key}"))
            .points
    };
    let expected: [(&str, &[FrozenCanonicalPoint], f32); 3] = [
        ("edge.a", lane_curve("span.a", "lane.a"), 10.0),
        ("edge.b", lane_curve("span.b", "lane.b"), 10.0),
        (
            "edge.internal",
            &payload.frozen.internal_edge_curves[0].points,
            3.5,
        ),
    ];
    for (row_index, (edge_key, curve, arc_length)) in expected.iter().enumerate() {
        let row = &hir.lane_edge_geometries[row_index];
        assert_eq!(
            hir.lane_edges[row.lane_edge.index()].stable_key.as_ref(),
            *edge_key
        );
        let point_range = row.points.as_usize_range();
        assert_eq!(point_range, row_index * 2..row_index * 2 + 2);
        // 几何点逐位等于 finish 的 numeric freeze 输出（§5.4 不得再次求值）。
        for (point, frozen) in hir.canonical_points[point_range].iter().zip(curve.iter()) {
            assert_eq!(point.x.to_bits(), frozen.x.to_bits(), "{edge_key}");
            assert_eq!(point.y.to_bits(), frozen.y.to_bits(), "{edge_key}");
            assert_eq!(point.z.to_bits(), frozen.z.to_bits(), "{edge_key}");
        }
        assert_eq!(row.arc_length_meters, *arc_length, "{edge_key}");
        let segment_range = row.segments.as_usize_range();
        assert_eq!(segment_range, row_index..row_index + 1);
        assert_eq!(
            hir.spatial_segments[segment_range.start].tangent,
            [1.0, 0.0, 0.0],
            "{edge_key}"
        );
    }
}

#[test]
fn geometry_closure_fails_when_approaches_use_different_frames() {
    // road.a 引用 frame.main、road.b 引用 frame.alt：同一 Junction 的 entry/exit
    // 引道边未解析到同一 canonical frame。闭包诊断在内部边循环结束时直接失败关闭
    //（staged fail-fast），因此只有一条冲突诊断。
    let unit = geometry_topology_unit(geometry_frames_roads_module(
        "city/main",
        "doc.main",
        &[],
        &["frame.main", "frame.alt"],
        &[
            geometry_road_fragment(
                "road.a",
                "frame.main",
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                "lane.a",
                &[geometry_lane_fragment("lane.a", "edge.a", &[])],
            ),
            geometry_road_fragment(
                "road.b",
                "frame.alt",
                [10.0, 0.0, 0.0],
                [20.0, 0.0, 0.0],
                "lane.b",
                &[geometry_lane_fragment("lane.b", "edge.b", &[])],
            ),
        ],
        &[geometry_junction_fragment(
            "junction.main",
            &["edge.a", "edge.b"],
            &[geometry_internal_edge_fragment_with_polyline(
                "edge.internal",
                &[[10.0, 0.0, 0.0], [13.5, 0.0, 0.0]],
            )],
            &[geometry_connection_fragment(
                "movement.main",
                "path.main",
                "edge.a",
                &["edge.internal"],
                "edge.b",
            )],
        )],
    ));
    let bundle = expect_diagnostics(crate::hir::build_hir(&unit));

    assert_eq!(bundle.diagnostics().len(), 1);
    let diagnostic = &bundle.diagnostics()[0];
    assert!(matches!(
        diagnostic.payload(),
        DiagnosticPayload::InvalidSpatialGeometry {
            canonical_frame_key: Some(frame),
            lane_edge_key,
            related_lane_edge_key: Some(related),
            violation: SpatialGeometryViolation::ApproachFrameConflict,
        } if frame.as_ref() == "frame.main"
            && lane_edge_key.as_ref() == "edge.a"
            && related.as_ref() == "edge.b"
    ));
}

#[test]
fn geometry_closure_fails_when_approach_lacks_geometry_binding() {
    // entry 引道边来自无几何绑定的 Synthetic 模块：闭包无法导出 internal 边的
    // frame。缺失诊断同样在内部边循环结束时失败关闭，只有一条诊断。
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_synthetic_module(synthetic_unbound_edge_module("city/base"))
        .unwrap();
    builder
        .add_geometry_module(geometry_roads_module(
            "city/main",
            "doc.main",
            &["city/base"],
            &[geometry_road_fragment(
                "road.a",
                "frame.main",
                [3.5, 0.0, 0.0],
                [13.5, 0.0, 0.0],
                "lane.a",
                &[geometry_lane_fragment("lane.a", "edge.a", &[])],
            )],
            &[geometry_junction_fragment(
                "junction.main",
                &["city/base::edge.x", "edge.a"],
                &[geometry_internal_edge_fragment_with_polyline(
                    "edge.internal",
                    &[[0.0, 0.0, 0.0], [3.5, 0.0, 0.0]],
                )],
                &[geometry_connection_fragment(
                    "movement.main",
                    "path.main",
                    "city/base::edge.x",
                    &["edge.internal"],
                    "edge.a",
                )],
            )],
        ))
        .unwrap();
    let unit = builder.build().unwrap();
    let bundle = expect_diagnostics(crate::hir::build_hir(&unit));

    // related 指向受影响的路口内部边；无绑定的 approach 没有候选 frame，frame 键为空。
    assert_eq!(bundle.diagnostics().len(), 1);
    let diagnostic = &bundle.diagnostics()[0];
    assert!(matches!(
        diagnostic.payload(),
        DiagnosticPayload::InvalidSpatialGeometry {
            canonical_frame_key: None,
            lane_edge_key,
            related_lane_edge_key: Some(related),
            violation: SpatialGeometryViolation::ApproachGeometryMissing,
        } if lane_edge_key.as_ref() == "edge.x" && related.as_ref() == "edge.internal"
    ));
}

#[test]
fn geometry_closure_reports_conflict_per_disjoint_connection_component() {
    // 同一 Junction 的两条 connection 各自跨 frame（edge.a/edge.c 用 frame.main，
    // edge.b/edge.d 用 frame.alt）：每个互不相交的 connection component 各报一次冲突。
    let road = |road_key: &str, frame_ref: &str, lane_key: &str, edge_key: &str| {
        geometry_road_fragment(
            road_key,
            frame_ref,
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            lane_key,
            &[geometry_lane_fragment(lane_key, edge_key, &[])],
        )
    };
    let unit = geometry_topology_unit(geometry_frames_roads_module(
        "city/main",
        "doc.main",
        &[],
        &["frame.main", "frame.alt"],
        &[
            road("road.a", "frame.main", "lane.a", "edge.a"),
            road("road.b", "frame.alt", "lane.b", "edge.b"),
            road("road.c", "frame.main", "lane.c", "edge.c"),
            road("road.d", "frame.alt", "lane.d", "edge.d"),
        ],
        &[geometry_junction_fragment(
            "junction.main",
            &["edge.a", "edge.b", "edge.c", "edge.d"],
            &[
                geometry_internal_edge_fragment_with_polyline(
                    "edge.i1",
                    &[[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]],
                ),
                geometry_internal_edge_fragment_with_polyline(
                    "edge.i2",
                    &[[10.0, 0.0, 0.0], [20.0, 0.0, 0.0]],
                ),
            ],
            &[
                geometry_connection_fragment(
                    "movement.one",
                    "path.one",
                    "edge.a",
                    &["edge.i1"],
                    "edge.b",
                ),
                geometry_connection_fragment(
                    "movement.two",
                    "path.two",
                    "edge.c",
                    &["edge.i2"],
                    "edge.d",
                ),
            ],
        )],
    ));
    let bundle = expect_diagnostics(crate::hir::build_hir(&unit));

    assert_eq!(bundle.diagnostics().len(), 2);
    for (diagnostic, (lane_edge, related)) in bundle
        .diagnostics()
        .iter()
        .zip([("edge.a", "edge.b"), ("edge.c", "edge.d")])
    {
        assert!(matches!(
            diagnostic.payload(),
            DiagnosticPayload::InvalidSpatialGeometry {
                canonical_frame_key: Some(frame),
                lane_edge_key,
                related_lane_edge_key: Some(actual_related),
                violation: SpatialGeometryViolation::ApproachFrameConflict,
            } if frame.as_ref() == "frame.main"
                && lane_edge_key.as_ref() == lane_edge
                && actual_related.as_ref() == related
        ));
    }
}

#[test]
fn geometry_closure_allows_disjoint_components_in_different_frames() {
    let road = |road_key: &str, frame_ref: &str, lane_key: &str, edge_key: &str| {
        let (start, end) = if road_key.ends_with(".b") || road_key.ends_with(".d") {
            ([20.0, 0.0, 0.0], [30.0, 0.0, 0.0])
        } else {
            ([0.0, 0.0, 0.0], [10.0, 0.0, 0.0])
        };
        geometry_road_fragment(
            road_key,
            frame_ref,
            start,
            end,
            lane_key,
            &[geometry_lane_fragment(lane_key, edge_key, &[])],
        )
    };
    let unit = geometry_topology_unit(geometry_frames_roads_module(
        "city/main",
        "doc.main",
        &[],
        &["frame.main", "frame.alt"],
        &[
            road("road.a", "frame.main", "lane.a", "edge.a"),
            road("road.b", "frame.main", "lane.b", "edge.b"),
            road("road.c", "frame.alt", "lane.c", "edge.c"),
            road("road.d", "frame.alt", "lane.d", "edge.d"),
        ],
        &[geometry_junction_fragment(
            "junction.main",
            &["edge.a", "edge.b", "edge.c", "edge.d"],
            &[
                geometry_internal_edge_fragment_with_polyline(
                    "edge.i1",
                    &[[10.0, 0.0, 0.0], [20.0, 0.0, 0.0]],
                ),
                geometry_internal_edge_fragment_with_polyline(
                    "edge.i2",
                    &[[10.0, 0.0, 0.0], [20.0, 0.0, 0.0]],
                ),
            ],
            &[
                geometry_connection_fragment(
                    "movement.one",
                    "path.one",
                    "edge.a",
                    &["edge.i1"],
                    "edge.b",
                ),
                geometry_connection_fragment(
                    "movement.two",
                    "path.two",
                    "edge.c",
                    &["edge.i2"],
                    "edge.d",
                ),
            ],
        )],
    ));
    let hir = crate::hir::build_hir(&unit).unwrap();

    for (internal_edge, expected_frame) in [("edge.i1", "frame.main"), ("edge.i2", "frame.alt")] {
        let geometry = hir
            .lane_edge_geometries
            .iter()
            .find(|geometry| {
                hir.lane_edges[geometry.lane_edge.index()]
                    .stable_key
                    .as_ref()
                    == internal_edge
            })
            .unwrap_or_else(|| panic!("missing geometry for {internal_edge}"));
        assert_eq!(
            hir.canonical_frames[geometry.canonical_frame.index()]
                .stable_key
                .as_ref(),
            expected_frame
        );
    }
}

#[test]
fn geometry_mixed_unit_fails_when_synthetic_edge_lacks_binding() {
    // 单元一旦启用空间几何就必须完整覆盖：无 frame/中心线声明的 Synthetic 边
    // 恰好收到一条 MissingEdgeBinding。
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_synthetic_module(synthetic_unbound_edge_module("city/base"))
        .unwrap();
    builder
        .add_geometry_module(geometry_module("city/geo", "doc.geo", &[]))
        .unwrap();
    let unit = builder.build().unwrap();
    let bundle = expect_diagnostics(crate::hir::build_hir(&unit));

    assert_eq!(bundle.diagnostics().len(), 1);
    let diagnostic = &bundle.diagnostics()[0];
    assert!(matches!(
        diagnostic.payload(),
        DiagnosticPayload::InvalidSpatialGeometry {
            canonical_frame_key: None,
            lane_edge_key,
            related_lane_edge_key: None,
            violation: SpatialGeometryViolation::MissingEdgeBinding,
        } if lane_edge_key.as_ref() == "edge.x"
    ));
}

#[test]
fn geometry_direction_jump_fails_on_explicit_successor() {
    // edge.b 偏转 atan(5/10) ≈ 26.6°：超出全部方向档。显式 successor 转换上
    // 失败关闭，载荷携带冻结数值语义下的点积/界限/阈值位模式（§6.1）。
    let unit = geometry_topology_unit(geometry_roads_module(
        "city/main",
        "doc.main",
        &[],
        &[
            geometry_road_fragment(
                "road.a",
                "frame.main",
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                "lane.a",
                &[geometry_lane_fragment("lane.a", "edge.a", &["edge.b"])],
            ),
            geometry_road_fragment(
                "road.b",
                "frame.main",
                [10.0, 0.0, 0.0],
                [20.0, 0.0, 5.0],
                "lane.b",
                &[geometry_lane_fragment("lane.b", "edge.b", &[])],
            ),
        ],
        &[],
    ));
    let bundle = expect_diagnostics(crate::hir::build_hir(&unit));

    // 弦 (10,0,0) 与 (10,0,5)：dot = 100，弦长平方乘积 = 12500。
    let threshold = GeometryDirectionProfile::Balanced2Deg.runtime_cos_squared();
    assert_eq!(bundle.diagnostics().len(), 1);
    let diagnostic = &bundle.diagnostics()[0];
    assert!(matches!(
        diagnostic.payload(),
        DiagnosticPayload::InvalidSpatialGeometry {
            canonical_frame_key: Some(frame),
            lane_edge_key,
            related_lane_edge_key: Some(related),
            violation:
                SpatialGeometryViolation::DirectionJumpExceeded {
                    dot_bits,
                    bound_bits,
                    threshold_bits,
                },
        } if frame.as_ref() == "frame.main"
            && lane_edge_key.as_ref() == "edge.a"
            && related.as_ref() == "edge.b"
            && *dot_bits == 100.0_f64.to_bits()
            && *bound_bits == (12500.0_f64 * threshold).to_bits()
            && *threshold_bits == threshold.to_bits()
    ));
}

#[test]
fn geometry_direction_jump_fails_on_path_transition() {
    // internal 边偏转 atan(2/3.5) ≈ 29.7°：两个路径转换（edge.a → internal、
    // internal → edge.b）都超出最终方向档，各自失败关闭。
    let unit = geometry_topology_unit(geometry_roads_module(
        "city/main",
        "doc.main",
        &[],
        &[
            geometry_road_fragment(
                "road.a",
                "frame.main",
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                "lane.a",
                &[geometry_lane_fragment("lane.a", "edge.a", &[])],
            ),
            geometry_road_fragment(
                "road.b",
                "frame.main",
                [13.5, 0.0, 2.0],
                [23.5, 0.0, 2.0],
                "lane.b",
                &[geometry_lane_fragment("lane.b", "edge.b", &[])],
            ),
        ],
        &[geometry_junction_fragment(
            "junction.main",
            &["edge.a", "edge.b"],
            &[geometry_internal_edge_fragment_with_polyline(
                "edge.internal",
                &[[10.0, 0.0, 0.0], [13.5, 0.0, 2.0]],
            )],
            &[geometry_connection_fragment(
                "movement.main",
                "path.main",
                "edge.a",
                &["edge.internal"],
                "edge.b",
            )],
        )],
    ));
    let bundle = expect_diagnostics(crate::hir::build_hir(&unit));

    assert_eq!(bundle.diagnostics().len(), 2);
    let mut transitions: Vec<(&str, &str)> = bundle
        .diagnostics()
        .iter()
        .filter_map(|diagnostic| match diagnostic.payload() {
            DiagnosticPayload::InvalidSpatialGeometry {
                canonical_frame_key: Some(frame),
                lane_edge_key,
                related_lane_edge_key: Some(related),
                violation: SpatialGeometryViolation::DirectionJumpExceeded { threshold_bits, .. },
            } if frame.as_ref() == "frame.main"
                && *threshold_bits
                    == GeometryDirectionProfile::Balanced2Deg
                        .runtime_cos_squared()
                        .to_bits() =>
            {
                Some((lane_edge_key.as_ref(), related.as_ref()))
            }
            _ => None,
        })
        .collect();
    transitions.sort_unstable();
    assert_eq!(
        transitions,
        [("edge.a", "edge.internal"), ("edge.internal", "edge.b")]
    );
}

#[test]
fn geometry_facility_band_geometry_row_follows_lane_points_bit_exact() {
    // 单 road 带一条 facility band：band 行只携带点范围，非空范围排在全部
    // lane edge 点之后拼入同一平面表（§5.4）。
    let unit = geometry_topology_unit(geometry_roads_module(
        "city/main",
        "doc.main",
        &[],
        &[geometry_road_fragment_with_facility_band(
            "road.main",
            "frame.main",
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            "lane.main",
            &[geometry_lane_fragment("lane.main", "edge.main", &[])],
            "facility.walk",
            2.0,
        )],
        &[],
    ));
    let hir = crate::hir::build_hir(&unit).unwrap();

    assert_eq!(hir.facility_bands.len(), 1);
    assert_eq!(hir.facility_band_geometries.len(), 1);
    assert_eq!(hir.canonical_points.len(), 4);
    assert_eq!(hir.spatial_segments.len(), 1);
    let row = &hir.facility_band_geometries[0];
    assert_eq!(
        hir.facility_bands[row.facility_band.index()]
            .stable_key
            .as_ref(),
        "facility.walk"
    );
    assert_eq!(
        hir.canonical_frames[row.canonical_frame.index()]
            .stable_key
            .as_ref(),
        "frame.main"
    );
    assert_eq!(row.points.as_usize_range(), 2..4);

    // 点逐位等于 payload 中 kind == FacilityBand 的冻结曲线。
    let payload = unit.geometry_payloads[0].as_ref().unwrap();
    let band_curve = &payload
        .frozen
        .lateral_curves
        .iter()
        .find(|curve| curve.kind == LateralIntentKind::FacilityBand)
        .expect("missing facility band curve")
        .points;
    for (point, frozen) in hir.canonical_points[2..4].iter().zip(band_curve.iter()) {
        assert_eq!(point.x.to_bits(), frozen.x.to_bits());
        assert_eq!(point.y.to_bits(), frozen.y.to_bits());
        assert_eq!(point.z.to_bits(), frozen.z.to_bits());
    }

    // MIR 镜像同一表形：一行、同一点区间。
    let mir = crate::mir::lower_to_mir(&unit, &hir).unwrap();
    assert_eq!(mir.facility_band_geometries.len(), 1);
    assert_eq!(
        mir.facility_band_geometries[0].points.as_usize_range(),
        2..4
    );
}

#[test]
fn mixed_unit_keeps_synthetic_facility_band_sparse() {
    // Synthetic 声明的 band.s 与 geometry 派生的 facility.walk 同单元共存：
    // 全局 facility 表两行，几何表只覆盖 geometry 派生的一行（§5.4 稀疏表）。
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_synthetic_module(synthetic_facility_module("city/base"))
        .unwrap();
    builder
        .add_geometry_module(geometry_roads_module(
            "city/geo",
            "doc.geo",
            &[],
            &[geometry_road_fragment_with_facility_band(
                "road.main",
                "frame.main",
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                "lane.main",
                &[geometry_lane_fragment("lane.main", "edge.main", &[])],
                "facility.walk",
                2.0,
            )],
            &[],
        ))
        .unwrap();
    let unit = builder.build().unwrap();
    let hir = crate::hir::build_hir(&unit).unwrap();

    assert_eq!(hir.facility_bands.len(), 2);
    assert_eq!(hir.facility_band_geometries.len(), 1);
    let row = &hir.facility_band_geometries[0];
    assert_eq!(
        hir.facility_bands[row.facility_band.index()]
            .stable_key
            .as_ref(),
        "facility.walk"
    );
}

#[test]
fn compiler_compiles_pure_geometry_unit_with_bit_exact_lir_spatial() {
    // 同一构造闭包建两次单元：compile 消耗其一，冻结曲线从另一份取（§5.4 逐位一致）。
    let build_unit = || {
        geometry_topology_unit(geometry_roads_module(
            "city/main",
            "doc.main",
            &[],
            &[
                geometry_road_fragment(
                    "road.a",
                    "frame.main",
                    [0.0, 0.0, 0.0],
                    [10.0, 0.0, 0.0],
                    "lane.a",
                    &[geometry_lane_fragment("lane.a", "edge.a", &[])],
                ),
                geometry_road_fragment(
                    "road.b",
                    "frame.main",
                    [13.5, 0.0, 0.0],
                    [23.5, 0.0, 0.0],
                    "lane.b",
                    &[geometry_lane_fragment("lane.b", "edge.b", &[])],
                ),
            ],
            &[geometry_junction_fragment(
                "junction.main",
                &["edge.a", "edge.b"],
                &[geometry_internal_edge_fragment_with_polyline(
                    "edge.internal",
                    &[[10.0, 0.0, 0.0], [13.5, 0.0, 0.0]],
                )],
                &[geometry_connection_fragment(
                    "movement.main",
                    "path.main",
                    "edge.a",
                    &["edge.internal"],
                    "edge.b",
                )],
            )],
        ))
    };
    let unit = build_unit();
    let reference = build_unit();
    let output = crate::Compiler::new().compile(unit).unwrap();

    let payload = reference.geometry_payloads[0].as_ref().unwrap();
    let lir = output.lir();
    assert_eq!(lir.lane_edges().len(), 3);
    assert_eq!(lir.canonical_frames().len(), 1);
    for edge in lir.lane_edges() {
        let key = lir_lane_edge_key(&edge);
        let expected: &[FrozenCanonicalPoint] = match key.as_str() {
            "edge.a" => payload
                .frozen
                .lateral_curves
                .iter()
                .find(|curve| curve.span_key.as_ref() == "span.a" && curve.key.as_ref() == "lane.a")
                .map(|curve| &curve.points),
            "edge.b" => payload
                .frozen
                .lateral_curves
                .iter()
                .find(|curve| curve.span_key.as_ref() == "span.b" && curve.key.as_ref() == "lane.b")
                .map(|curve| &curve.points),
            "edge.internal" => payload
                .frozen
                .internal_edge_curves
                .first()
                .map(|curve| &curve.points),
            other => panic!("unexpected lane edge {other}"),
        }
        .unwrap_or_else(|| panic!("missing frozen curve for {key}"));
        let geometry = edge
            .spatial_geometry()
            .expect("geometry edge must carry spatial geometry");
        let points: Vec<_> = geometry.points().collect();
        assert_eq!(points.len(), expected.len(), "{key}");
        for (actual, frozen) in points.iter().zip(expected.iter()) {
            assert_eq!(actual.x.to_bits(), frozen.x.to_bits(), "{key}");
            assert_eq!(actual.y.to_bits(), frozen.y.to_bits(), "{key}");
            assert_eq!(actual.z.to_bits(), frozen.z.to_bits(), "{key}");
        }
        assert_eq!(geometry.segments().count(), 1, "{key}");
    }
}

#[test]
fn compiler_compiles_mixed_synthetic_and_geometry_unit() {
    // Synthetic 显式中心线与 geometry 冻结曲线在同一单元内共存编译：edge.x 用
    // city/base 自声明的 frame.base，geometry road 引用导入的同一 frame。
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_synthetic_module(synthetic_edge_module("city/base"))
        .unwrap();
    builder
        .add_geometry_module(geometry_module_with_frame_ref(
            "city/geo",
            "doc.geo",
            &["city/base"],
            "city/base::frame.base",
        ))
        .unwrap();
    let unit = builder.build().unwrap();
    let output = crate::Compiler::new().compile(unit).unwrap();

    let lir = output.lir();
    assert_eq!(lir.lane_edges().len(), 2);
    for edge in lir.lane_edges() {
        let key = lir_lane_edge_key(&edge);
        let geometry = edge
            .spatial_geometry()
            .expect("every edge carries spatial geometry");
        let points: Vec<_> = geometry.points().collect();
        let expected: [(f32, f32, f32); 2] = match key.as_str() {
            // synthetic 显式中心线 [-10,0,0] → [0,0,0]。
            "edge.x" => [(-10.0, 0.0, 0.0), (0.0, 0.0, 0.0)],
            // geometry 参考线 [0,0,0] → [10,0,0]（5cm 量化下精确）。
            "edge.main" => [(0.0, 0.0, 0.0), (10.0, 0.0, 0.0)],
            other => panic!("unexpected lane edge {other}"),
        };
        assert_eq!(points.len(), 2, "{key}");
        for (actual, (x, y, z)) in points.iter().zip(expected) {
            assert_eq!(actual.x.to_bits(), x.to_bits(), "{key}");
            assert_eq!(actual.y.to_bits(), y.to_bits(), "{key}");
            assert_eq!(actual.z.to_bits(), z.to_bits(), "{key}");
        }
    }
}

#[test]
fn geometry_facility_band_lir_row_follows_lane_points_bit_exact() {
    // §5.4 LIR 表形：facility band 几何行按 FacilityBand 规范序排列，非空点范围排在
    // 全部 lane edge 点之后拼入同一 `canonical_points` 平面表。
    let unit = geometry_topology_unit(geometry_roads_module(
        "city/main",
        "doc.main",
        &[],
        &[geometry_road_fragment_with_facility_band(
            "road.main",
            "frame.main",
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            "lane.main",
            &[geometry_lane_fragment("lane.main", "edge.main", &[])],
            "facility.walk",
            2.0,
        )],
        &[],
    ));
    let hir = crate::hir::build_hir(&unit).unwrap();
    let mir = crate::mir::lower_to_mir(&unit, &hir).unwrap();
    let lir = crate::lir::freeze_lir(&unit, &mir).unwrap().lir;

    assert_eq!(lir.lane_edge_geometries.len(), 1);
    assert_eq!(lir.facility_bands.len(), 1);
    assert_eq!(lir.facility_band_geometries.len(), 1);
    assert_eq!(lir.canonical_points.len(), 4);
    assert_eq!(lir.spatial_segments.len(), 1);
    // lane edge 点占 [0,2)，band 范围紧随其后。
    assert_eq!(lir.lane_edge_geometries[0].points.as_usize_range(), 0..2);
    let row = &lir.facility_band_geometries[0];
    assert_eq!(row.facility_band.raw(), 0);
    assert_eq!(row.canonical_frame.raw(), 0);
    assert_eq!(row.points.as_usize_range(), 2..4);

    // 点逐位等于 payload 中 kind == FacilityBand 的冻结曲线。
    let payload = unit.geometry_payloads[0].as_ref().unwrap();
    let band_curve = &payload
        .frozen
        .lateral_curves
        .iter()
        .find(|curve| curve.kind == LateralIntentKind::FacilityBand)
        .expect("missing facility band curve")
        .points;
    for (point, frozen) in lir.canonical_points[row.points.as_usize_range()]
        .iter()
        .zip(band_curve.iter())
    {
        assert_eq!(point.x.to_bits(), frozen.x.to_bits());
        assert_eq!(point.y.to_bits(), frozen.y.to_bits());
        assert_eq!(point.z.to_bits(), frozen.z.to_bits());
    }
}

#[test]
fn facility_band_geometry_view_reads_sparse_rows_only() {
    // 公共视图与 §5.4 稀疏表一致：Synthetic 声明的 band.s 没有几何行，geometry 派生的
    // facility.walk 返回不可遍历中心线，来源关系经同一 permutation 指向 band 模块文档。
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_synthetic_module(synthetic_facility_module("city/base"))
        .unwrap();
    builder
        .add_geometry_module(geometry_roads_module(
            "city/geo",
            "doc.geo",
            &[],
            &[geometry_road_fragment_with_facility_band(
                "road.main",
                "frame.main",
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                "lane.main",
                &[geometry_lane_fragment("lane.main", "edge.main", &[])],
                "facility.walk",
                2.0,
            )],
            &[],
        ))
        .unwrap();
    let unit = builder.build().unwrap();
    let output = crate::Compiler::new().compile(unit).unwrap();
    let lir = output.lir();

    let band_key = |band: &CanonicalFacilityBandView<'_>| -> String {
        band.identity_fields()
            .find(|field| field.tag() == laneflow_static_contract::FieldTag::FacilityBandKey)
            .map(|field| String::from_utf8(field.value_bytes().to_vec()).unwrap())
            .expect("facility band must carry the FacilityBandKey identity field")
    };
    let bands: Vec<_> = lir.facility_bands().collect();
    assert_eq!(bands.len(), 2);
    let mut walk_frame = None;
    for band in &bands {
        match band_key(band).as_str() {
            "band.s" => assert!(band.geometry().is_none()),
            "facility.walk" => {
                let geometry = band.geometry().expect("geometry band must carry a row");
                assert_eq!(geometry.facility_band(), band.ordinal());
                assert_eq!(geometry.points().len(), 2);
                walk_frame = Some(geometry.canonical_frame());
            }
            other => panic!("unexpected facility band {other}"),
        }
    }
    let walk_frame = walk_frame.expect("missing geometry facility band");

    // frame 的 facility geometry 关系恰好一行，owner 为 band 中心线所在 frame，
    // 来源位置解析到声明 band 的 geometry 模块文档。
    let facility_relations: Vec<_> = output
        .source_map_input()
        .spatial_relation_sources()
        .filter(|relation| {
            relation.role() == SourceRelationRole::CanonicalFrameFacilityBandGeometry
        })
        .collect();
    assert_eq!(facility_relations.len(), 1);
    let relation = facility_relations[0];
    assert_eq!(relation.owner_ordinal(), walk_frame);
    assert_eq!(relation.local_index(), 0);
    assert_eq!(relation.primary_source().source_document_key(), "doc.geo");
}

#[test]
fn derived_successor_source_map_resolves_declaring_module_document() {
    // §4.4：派生 transition 的唯一 LIR successor 来源解析到声明该 connection 的模块
    // 文档（doc.main），不能回落到起始边所属模块（city/base 或 doc.geo）。构造与
    // `geometry_connection_references_imported_edges_across_frontends` 相同，推进到 LIR。
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_synthetic_module(synthetic_edge_module("city/base"))
        .unwrap();
    builder
        .add_geometry_module(geometry_module_with_frame_ref(
            "city/geo",
            "doc.geo",
            &["city/base"],
            "city/base::frame.base",
        ))
        .unwrap();
    builder
        .add_geometry_module(geometry_roads_module(
            "city/main",
            "doc.main",
            &["city/base", "city/geo"],
            &[
                geometry_road_fragment(
                    "road.a",
                    "city/base::frame.base",
                    [3.5, 0.0, 0.0],
                    [13.5, 0.0, 0.0],
                    "lane.a",
                    &[geometry_lane_fragment("lane.a", "edge.a", &[])],
                ),
                geometry_road_fragment(
                    "road.b",
                    "city/base::frame.base",
                    [13.5, 0.0, 0.0],
                    [23.5, 0.0, 0.0],
                    "lane.b",
                    &[geometry_lane_fragment("lane.b", "edge.b", &[])],
                ),
            ],
            &[geometry_junction_fragment(
                "junction.main",
                &[
                    "city/base::edge.x",
                    "city/geo::edge.main",
                    "edge.a",
                    "edge.b",
                ],
                &[
                    geometry_internal_edge_fragment_with_polyline(
                        "edge.i1",
                        &[[0.0, 0.0, 0.0], [3.5, 0.0, 0.0]],
                    ),
                    geometry_internal_edge_fragment_with_polyline(
                        "edge.i2",
                        &[[10.0, 0.0, 0.0], [13.5, 0.0, 0.0]],
                    ),
                ],
                &[
                    geometry_connection_fragment(
                        "movement.one",
                        "path.one",
                        "city/base::edge.x",
                        &["edge.i1"],
                        "edge.a",
                    ),
                    geometry_connection_fragment(
                        "movement.two",
                        "path.two",
                        "city/geo::edge.main",
                        &["edge.i2"],
                        "edge.b",
                    ),
                ],
            )],
        ))
        .unwrap();
    let unit = builder.build().unwrap();
    let output = crate::Compiler::new().compile(unit).unwrap();
    let lir = output.lir();

    // 四条派生 successor（edge.x→i1、edge.main→i2、i1→a、i2→b）的唯一来源都解析到
    // 声明 connection 的 city/main 文档，即使起始边属于 city/base 或 city/geo。
    let rows: Vec<(String, String)> = output
        .source_map_input()
        .lane_edge_successor_sources()
        .map(|source| {
            let owner = lir
                .lane_edge(source.owner_ordinal())
                .expect("successor owner must be a LIR lane edge");
            (
                lir_lane_edge_key(&owner),
                source.primary_source().source_document_key().to_owned(),
            )
        })
        .collect();
    assert_eq!(rows.len(), 4);
    for (owner, document) in &rows {
        assert_eq!(document, "doc.main", "{owner}");
        assert!(
            ["edge.x", "edge.main", "edge.i1", "edge.i2"].contains(&owner.as_str()),
            "unexpected successor owner {owner}"
        );
    }
}

/// §7.2 阶段遮蔽 known-vector 的公共断言：`finish` 只返回阶段 3 的规范最小诊断，
/// 即恰好一条 `widthMeters` 非正局部值域错误（`InvalidGeometryDocument` /
/// `FieldValue` / `InvalidWidth`），不多不少；尚不可执行的 import、跨模块归属与
/// 配置档混用诊断被规范遮蔽。对照场景已分别证明这些后续阶段错误真实存在。
fn assert_finish_reports_only_stage3_width_violation(
    bundle: &DiagnosticBundle,
    document_key: &str,
) {
    assert_eq!(
        bundle.diagnostics().len(),
        1,
        "阶段 3 规范最小诊断必须恰好一条，不得附带被遮蔽阶段的诊断"
    );
    assert!(!bundle.diagnostics_truncated(), "单诊断不得触发诊断截断");
    let diagnostic = &bundle.diagnostics()[0];
    assert_eq!(
        diagnostic.code(),
        DiagnosticCode::InvalidGeometryDocument,
        "阶段 3 局部值域错误必须是 InvalidGeometryDocument"
    );
    assert!(matches!(
        diagnostic.payload(),
        DiagnosticPayload::InvalidGeometryDocument {
            violation: GeometryDocumentViolation::FieldValue,
            field: Some(field),
            actual: Some(actual),
            expected: Some(expected),
        } if field.as_ref() == "widthMeters"
            && actual.as_ref() == "InvalidWidth"
            && expected.as_ref() == "complete bounded Geometry v1 canonical geometry payload"
    ));
    assert_eq!(
        diagnostic.primary_span().unwrap().source_document_key(),
        document_key,
        "诊断必须锚定注入错误的来源文档"
    );
}

#[test]
fn geometry_finish_numeric_freeze_error_shadows_unknown_import() {
    // §7.2 阶段遮蔽 known-vector（未知 import）：文档声明 import "city/ghost" 并把
    // road frame 指向 city/ghost::frame.base；finish 只按已声明 import 解析引用，
    // 未知 import 要到模块进入编译单元后的共同 admission 才可判定。同一文档注入
    // widthMeters 非正（阶段 3 局部值域错误），证明 finish 只返回阶段 3 诊断。
    let build_source = |width_meters: f64| {
        geometry_document_source(
            "city/main",
            "doc.main",
            &["city/ghost"],
            &["frame.main"],
            &[geometry_road_fragment(
                "road.main",
                "city/ghost::frame.base",
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                "lane.main",
                &[geometry_lane_fragment_with_width(
                    "lane.main",
                    "edge.main",
                    &[],
                    width_meters,
                )],
            )],
            &[],
        )
    };
    let finish = |source: String| {
        GeometryModuleBuilder::new(
            GeometryDocumentInput::new("doc.main", source.as_bytes(), None),
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &CompileLimits::p100_initial_v1(),
        )
        .expect("known-vector 文档必须解析成功")
        .finish()
    };

    // 对照：无阶段 3 错误时 finish 通过（import 已声明即满足引用解析），同一模块
    // 进入编译单元后，未知 import 在共同 admission 失败关闭。
    let module = finish(build_source(3.5)).expect("无局部错误的对照文档必须 finish 成功");
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder.add_geometry_module(module).unwrap();
    let shadowed = expect_diagnostics(builder.build());
    assert_eq!(shadowed.diagnostics().len(), 1);
    assert!(matches!(
        shadowed.diagnostics()[0].payload(),
        DiagnosticPayload::UnknownImport { namespace } if namespace.as_ref() == "city/ghost"
    ));

    // known-vector：同一来源同时含阶段 3 局部值域错误；finish 在模块存在前失败，
    // 未知 import 诊断被规范遮蔽。
    let bundle = expect_diagnostics(finish(build_source(-1.0)));
    assert_finish_reports_only_stage3_width_violation(&bundle, "doc.main");
}

#[test]
fn geometry_finish_numeric_freeze_error_shadows_cross_module_owner_conflict() {
    // §7.2 阶段遮蔽 known-vector（跨模块归属）：文档 import 真实存在的 city/base，
    // 路口 approach 同时含 city/base::edge.x（绑定 city/base::frame.base）与本地
    // edge.a（绑定本地 frame.main）；引道边的 frame 归属冲突要到 HIR 路口 frame
    // 闭包才可判定。构造探明记录：laneEdgeKey 拼写 "city/base::edge.x" 只是合法
    // 本地键（`:` 属于外部 token 字符集），不产生跨模块引用，因此本场景改用路口
    // frame 归属冲突。同一文档注入阶段 3 局部值域错误，证明 finish 只返回阶段 3
    // 诊断。
    let build_source = |width_meters: f64| {
        geometry_document_source(
            "city/main",
            "doc.main",
            &["city/base"],
            &["frame.main"],
            &[geometry_road_fragment(
                "road.a",
                "frame.main",
                [3.5, 0.0, 0.0],
                [13.5, 0.0, 0.0],
                "lane.a",
                &[geometry_lane_fragment_with_width(
                    "lane.a",
                    "edge.a",
                    &[],
                    width_meters,
                )],
            )],
            &[geometry_junction_fragment(
                "junction.main",
                &["city/base::edge.x", "edge.a"],
                &[geometry_internal_edge_fragment_with_polyline(
                    "edge.internal",
                    &[[0.0, 0.0, 0.0], [3.5, 0.0, 0.0]],
                )],
                &[geometry_connection_fragment(
                    "movement.main",
                    "path.main",
                    "city/base::edge.x",
                    &["edge.internal"],
                    "edge.a",
                )],
            )],
        )
    };
    let finish = |source: String| {
        GeometryModuleBuilder::new(
            GeometryDocumentInput::new("doc.main", source.as_bytes(), None),
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &CompileLimits::p100_initial_v1(),
        )
        .expect("known-vector 文档必须解析成功")
        .finish()
    };

    // 对照：无阶段 3 错误时 finish 通过、单元 build 成功，跨模块 frame 归属冲突
    // 在 HIR 失败关闭。
    let module = finish(build_source(3.5)).expect("无局部错误的对照文档必须 finish 成功");
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_synthetic_module(synthetic_edge_module("city/base"))
        .unwrap();
    builder.add_geometry_module(module).unwrap();
    let unit = builder.build().unwrap();
    let shadowed = expect_diagnostics(crate::hir::build_hir(&unit));
    assert_eq!(shadowed.diagnostics().len(), 1);
    assert!(matches!(
        shadowed.diagnostics()[0].payload(),
        DiagnosticPayload::InvalidSpatialGeometry {
            violation: SpatialGeometryViolation::ApproachFrameConflict,
            ..
        }
    ));

    // known-vector：同一来源同时含阶段 3 局部值域错误；finish 在模块存在前失败，
    // 跨模块归属诊断被规范遮蔽。
    let bundle = expect_diagnostics(finish(build_source(-1.0)));
    assert_finish_reports_only_stage3_width_violation(&bundle, "doc.main");
}

#[test]
fn geometry_finish_numeric_freeze_error_shadows_mixed_profiles() {
    // §7.2 阶段遮蔽 known-vector（配置档混用）：unit 先收 Balanced5Cm/Balanced2Deg
    // 的 Geometry 模块；同一来源再以 Fine2Cm/Smooth1Deg 构建并注入阶段 3 局部值域
    // 错误。MixedGeometryAccuracyProfile/MixedGeometryDirectionProfile 只在成功模块
    // 进入编译单元后的 HIR 产生，finish 只返回阶段 3 诊断。
    let build_source = |width_meters: f64| {
        geometry_document_source(
            "city/geo.b",
            "doc.geo.b",
            &[],
            &["frame.main"],
            &[geometry_road_fragment(
                "road.main",
                "frame.main",
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                "lane.main",
                &[geometry_lane_fragment_with_width(
                    "lane.main",
                    "edge.main",
                    &[],
                    width_meters,
                )],
            )],
            &[],
        )
    };
    let finish = |source: String| {
        GeometryModuleBuilder::new(
            GeometryDocumentInput::new("doc.geo.b", source.as_bytes(), None),
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            &CompileLimits::p100_initial_v1(),
        )
        .expect("known-vector 文档必须解析成功")
        .finish()
    };

    // 对照：无阶段 3 错误的同来源文档 finish 成功，加入单元后混用检查在 HIR 失败
    // （accuracy 与 direction 正交，同阶段按诊断代码序分别报告）。
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_geometry_module(geometry_module_with_profiles(
            "city/geo.a",
            "doc.geo.a",
            &[],
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
        ))
        .unwrap();
    builder
        .add_geometry_module(finish(build_source(3.5)).expect("对照文档必须 finish 成功"))
        .unwrap();
    let unit = builder.build().unwrap();
    let shadowed = expect_diagnostics(crate::hir::build_hir(&unit));
    assert_eq!(
        shadowed
            .diagnostics()
            .iter()
            .map(Diagnostic::code)
            .collect::<Vec<_>>(),
        [
            DiagnosticCode::MixedGeometryAccuracyProfile,
            DiagnosticCode::MixedGeometryDirectionProfile,
        ]
    );

    // known-vector：含阶段 3 错误的同一来源在 finish 失败，配置档混用诊断被规范
    // 遮蔽。
    let bundle = expect_diagnostics(finish(build_source(-1.0)));
    assert_finish_reports_only_stage3_width_violation(&bundle, "doc.geo.b");
}

#[test]
fn geometry_frontend_repeated_compile_is_bit_exact() {
    // §8 确定性行：同一 Geometry unit 构造两次、由同一 Compiler 顺序 compile，
    // 语义指纹与全部 lane edge 空间几何点 bit pattern 完全一致。
    let build_unit = || {
        geometry_topology_unit(geometry_roads_module(
            "city/main",
            "doc.main",
            &[],
            &[
                geometry_road_fragment(
                    "road.a",
                    "frame.main",
                    [0.0, 0.0, 0.0],
                    [10.0, 0.0, 0.0],
                    "lane.a",
                    &[geometry_lane_fragment("lane.a", "edge.a", &[])],
                ),
                geometry_road_fragment(
                    "road.b",
                    "frame.main",
                    [13.5, 0.0, 0.0],
                    [23.5, 0.0, 0.0],
                    "lane.b",
                    &[geometry_lane_fragment("lane.b", "edge.b", &[])],
                ),
            ],
            &[geometry_junction_fragment(
                "junction.main",
                &["edge.a", "edge.b"],
                &[geometry_internal_edge_fragment_with_polyline(
                    "edge.internal",
                    &[[10.0, 0.0, 0.0], [13.5, 0.0, 0.0]],
                )],
                &[geometry_connection_fragment(
                    "movement.main",
                    "path.main",
                    "edge.a",
                    &["edge.internal"],
                    "edge.b",
                )],
            )],
        ))
    };
    let mut compiler = crate::Compiler::new();
    let first = compiler.compile(build_unit()).unwrap();
    let second = compiler.compile(build_unit()).unwrap();

    assert_eq!(
        first.metrics().semantic_fingerprint(),
        second.metrics().semantic_fingerprint(),
        "重复 clean compile 的语义指纹必须一致"
    );
    let edge_point_bits = |output: &crate::CompilationOutput| {
        output
            .lir()
            .lane_edges()
            .map(|edge| {
                (
                    lir_lane_edge_key(&edge),
                    edge.spatial_geometry()
                        .expect("geometry edge must carry spatial geometry")
                        .points()
                        .map(|point| [point.x.to_bits(), point.y.to_bits(), point.z.to_bits()])
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        edge_point_bits(&first),
        edge_point_bits(&second),
        "重复 clean compile 的 lane edge 点 bit pattern 必须一致"
    );
}

#[test]
fn geometry_frontend_module_add_order_does_not_change_fingerprint() {
    // §8 确定性行：2 个 Geometry 模块 + 1 个 Synthetic 模块按不同 add 顺序加入
    // CompilationUnitBuilder（依赖 city/base ← city/geo ← city/main 在两种顺序下
    // 都合法）；规范依赖序使两次 compile 的语义指纹一致。
    let junction_module = || {
        geometry_roads_module(
            "city/main",
            "doc.main",
            &["city/base", "city/geo"],
            &[geometry_road_fragment(
                "road.a",
                "city/base::frame.base",
                [13.5, 0.0, 0.0],
                [23.5, 0.0, 0.0],
                "lane.a",
                &[geometry_lane_fragment("lane.a", "edge.a", &[])],
            )],
            &[geometry_junction_fragment(
                "junction.main",
                &["city/base::edge.x", "city/geo::edge.main", "edge.a"],
                &[
                    // i1 接合 edge.x 末端 [0,0,0] 与 edge.a 起点 [13.5,0,0]；
                    // i2 接合 edge.main 末端 [10,0,0] 与 edge.a 起点 [13.5,0,0]。
                    geometry_internal_edge_fragment_with_polyline(
                        "edge.i1",
                        &[[0.0, 0.0, 0.0], [13.5, 0.0, 0.0]],
                    ),
                    geometry_internal_edge_fragment_with_polyline(
                        "edge.i2",
                        &[[10.0, 0.0, 0.0], [13.5, 0.0, 0.0]],
                    ),
                ],
                &[
                    geometry_connection_fragment(
                        "movement.one",
                        "path.one",
                        "city/base::edge.x",
                        &["edge.i1"],
                        "edge.a",
                    ),
                    geometry_connection_fragment(
                        "movement.two",
                        "path.two",
                        "city/geo::edge.main",
                        &["edge.i2"],
                        "edge.a",
                    ),
                ],
            )],
        )
    };
    let geo_module = || {
        geometry_module_with_frame_ref(
            "city/geo",
            "doc.geo",
            &["city/base"],
            "city/base::frame.base",
        )
    };
    let build_unit = |dependency_last: bool| {
        let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
        if dependency_last {
            builder.add_geometry_module(junction_module()).unwrap();
            builder.add_geometry_module(geo_module()).unwrap();
            builder
                .add_synthetic_module(synthetic_edge_module("city/base"))
                .unwrap();
        } else {
            builder
                .add_synthetic_module(synthetic_edge_module("city/base"))
                .unwrap();
            builder.add_geometry_module(geo_module()).unwrap();
            builder.add_geometry_module(junction_module()).unwrap();
        }
        builder.build().unwrap()
    };

    let first = crate::Compiler::new().compile(build_unit(false)).unwrap();
    let second = crate::Compiler::new().compile(build_unit(true)).unwrap();
    assert_eq!(
        first.metrics().semantic_fingerprint(),
        second.metrics().semantic_fingerprint(),
        "模块 add 顺序不得改变 LIR 语义指纹"
    );
}

#[test]
fn geometry_frontend_declaration_array_reorder_follows_array_semantics() {
    // §8 确定性行：语义无序的来源数组重排不改变 LIR 语义；显式有序的横断面
    // lanes 数组（§4.3 横向顺序语义）重排必须改变语义指纹。
    let two_lane_road = |lanes: &[String]| {
        geometry_road_fragment(
            "road.e",
            "frame.main",
            [60.0, 0.0, 0.0],
            [70.0, 0.0, 0.0],
            "lane.e1",
            lanes,
        )
    };
    let lane_e1 = || geometry_lane_fragment("lane.e1", "edge.e1", &[]);
    let lane_e2 = || geometry_lane_fragment("lane.e2", "edge.e2", &[]);
    let plain_roads = || {
        vec![
            geometry_road_fragment(
                "road.a",
                "frame.main",
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                "lane.a",
                &[geometry_lane_fragment("lane.a", "edge.a", &[])],
            ),
            geometry_road_fragment(
                "road.b",
                "frame.main",
                [13.5, 0.0, 0.0],
                [23.5, 0.0, 0.0],
                "lane.b",
                &[geometry_lane_fragment("lane.b", "edge.b", &[])],
            ),
            geometry_road_fragment(
                "road.c",
                "frame.main",
                [27.0, 0.0, 0.0],
                [37.0, 0.0, 0.0],
                "lane.c",
                &[geometry_lane_fragment("lane.c", "edge.c", &[])],
            ),
            geometry_road_fragment(
                "road.d",
                "frame.main",
                [40.5, 0.0, 0.0],
                [50.5, 0.0, 0.0],
                "lane.d",
                &[geometry_lane_fragment("lane.d", "edge.d", &[])],
            ),
        ]
    };
    let junction_one = |connections: &[String]| {
        geometry_junction_fragment(
            "junction.one",
            &["edge.a", "edge.b"],
            &[
                geometry_internal_edge_fragment_with_polyline(
                    "edge.i1",
                    &[[10.0, 0.0, 0.0], [13.5, 0.0, 0.0]],
                ),
                geometry_internal_edge_fragment_with_polyline(
                    "edge.i3",
                    &[[10.0, 0.0, 0.0], [13.5, 0.0, 0.0]],
                ),
            ],
            connections,
        )
    };
    let junction_two = || {
        geometry_junction_fragment(
            "junction.two",
            &["edge.c", "edge.d"],
            &[geometry_internal_edge_fragment_with_polyline(
                "edge.i2",
                &[[37.0, 0.0, 0.0], [40.5, 0.0, 0.0]],
            )],
            &[geometry_connection_fragment(
                "movement.three",
                "path.three",
                "edge.c",
                &["edge.i2"],
                "edge.d",
            )],
        )
    };
    let connection_one = || {
        geometry_connection_fragment("movement.one", "path.one", "edge.a", &["edge.i1"], "edge.b")
    };
    let connection_two = || {
        geometry_connection_fragment("movement.two", "path.two", "edge.a", &["edge.i3"], "edge.b")
    };
    let fingerprint = |roads: Vec<String>, junctions: Vec<String>| {
        crate::Compiler::new()
            .compile(geometry_topology_unit(geometry_roads_module(
                "city/main",
                "doc.main",
                &[],
                &roads,
                &junctions,
            )))
            .unwrap()
            .metrics()
            .semantic_fingerprint()
    };

    let baseline = fingerprint(
        [plain_roads(), vec![two_lane_road(&[lane_e1(), lane_e2()])]].concat(),
        vec![
            junction_one(&[connection_one(), connection_two()]),
            junction_two(),
        ],
    );

    // roads 数组重排（语义无序）：指纹不变。
    let mut reordered_roads = plain_roads();
    reordered_roads.reverse();
    reordered_roads.insert(0, two_lane_road(&[lane_e1(), lane_e2()]));
    assert_eq!(
        fingerprint(
            reordered_roads,
            vec![
                junction_one(&[connection_one(), connection_two()]),
                junction_two(),
            ]
        ),
        baseline,
        "roads 数组顺序重排不得改变语义指纹"
    );

    // junctions 数组与 junction 内 connections 数组重排（语义无序）：指纹不变。
    assert_eq!(
        fingerprint(
            [plain_roads(), vec![two_lane_road(&[lane_e1(), lane_e2()])]].concat(),
            vec![
                junction_two(),
                junction_one(&[connection_two(), connection_one()]),
            ],
        ),
        baseline,
        "junctions/connections 数组顺序重排不得改变语义指纹"
    );

    // 同一 section 的 lanes 数组重排（§4.3 横向顺序是显式语义，lane.e2 的横向
    // 偏移随顺序翻转）：指纹必须改变。
    let lanes_reordered = fingerprint(
        [plain_roads(), vec![two_lane_road(&[lane_e2(), lane_e1()])]].concat(),
        vec![
            junction_one(&[connection_one(), connection_two()]),
            junction_two(),
        ],
    );
    assert_ne!(
        lanes_reordered, baseline,
        "lanes 数组顺序是横断面横向显式语义，重排必须改变语义指纹"
    );
}
