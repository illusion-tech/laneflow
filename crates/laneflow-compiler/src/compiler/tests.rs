use super::*;
use crate::declaration::{
    CompiledFacilityBandGeometry, EdgeLength, OwnedEntityReference, TypedAstDeclaration,
};
use crate::{
    AccessCapability, AccessRegulationInput, AccessRuleInput, AccessRuleTargetInput,
    AuthoringLaneInput, CanonicalFrameInput, CanonicalPoint3F32Input, CompilationUnitBuilder,
    CompileLimitDimension, CompileLimits, CorridorElementReference, DiagnosticCode,
    DiagnosticPayload, FacilityBandInput, FacilityBandReference, IidmVehicleProfileInput,
    JunctionInput, JunctionReference, LaneEdgeGeometryInput, LaneEdgeInput, LaneEdgeReference,
    LaneGroupInput, LaneGroupReference, ManeuverGateInput, ManeuverGateReference,
    ManeuverPathInput, ManeuverPathReference, MovementInput, MovementReference, ParkingAreaInput,
    ParkingAreaReference, ParkingLaneAnchorInput, ParkingSpaceGeometryInput, ParkingSpaceInput,
    ParticipantClassInput, ParticipantClassReference, RoadCorridorInput, RoadSectionInput,
    RoadSectionReference, SignalControlInput, SignalControllerInput, SignalGroupInput,
    SignalGroupReference, SignalGroupStateInput, SignalPhaseInput, SourceModuleDescriptor,
    SourceModuleHeader, SourceModuleHeaderInput, SourceRelationRole, SourceSpan, StaticRouteInput,
    StopLineInput, StopLineReference, SyntheticModule, SyntheticModuleBuilder, VehicleProfileInput,
    WaitingZoneInput,
};
use laneflow_static_contract::CanonicalFrameKind;
use std::sync::Arc;

fn module(
    namespace: &str,
    document: &str,
    imports: &[&str],
    edges: &[(&str, f64, &[LaneEdgeReference<'_>])],
) -> SyntheticModule {
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
    let mut builder = SyntheticModuleBuilder::new(header, &limits).unwrap();
    for import in imports {
        builder.add_import(import).unwrap();
    }
    for (key, length_meters, successors) in edges {
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: key,
                length_meters: *length_meters,
                speed_limit_meters_per_second: 13.75,
                successors,
            })
            .unwrap();
    }
    builder.finish().unwrap()
}

fn unit(modules: impl IntoIterator<Item = SyntheticModule>) -> CompilationUnit {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    for module in modules {
        builder.add_synthetic_module(module).unwrap();
    }
    builder.build().unwrap()
}

fn cross_section_module(permuted: bool) -> SyntheticModule {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "city/cross-section",
            source_document_key: "cross-section.document",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: Some(42),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .unwrap();
    let mut builder = SyntheticModuleBuilder::new(header, &limits).unwrap();
    let add_edges = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 10.0,
                speed_limit_meters_per_second: 12.0,
                successors: &[LaneEdgeReference::local("edge-b")],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-b",
                length_meters: 12.0,
                speed_limit_meters_per_second: 12.0,
                successors: &[],
            })
            .unwrap();
    };
    let add_band = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_facility_band(FacilityBandInput {
                facility_band_key: "sidewalk-left",
                kind_id: "sidewalk",
            })
            .unwrap();
    };
    let add_group = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_lane_group(LaneGroupInput {
                lane_group_key: "through",
                road_section: RoadSectionReference::local("carriageway"),
            })
            .unwrap();
    };
    let add_section = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_road_section(RoadSectionInput {
                road_section_key: "carriageway",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-main",
                    edge_chain: &[
                        LaneEdgeReference::local("edge-a"),
                        LaneEdgeReference::local("edge-b"),
                    ],
                    lane_group: Some(LaneGroupReference::local("through")),
                }],
            })
            .unwrap();
    };
    let add_corridor = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: "main-road",
                reference_section: RoadSectionReference::local("carriageway"),
                elements: &[
                    CorridorElementReference::facility_band(FacilityBandReference::local(
                        "sidewalk-left",
                    )),
                    CorridorElementReference::road_section(RoadSectionReference::local(
                        "carriageway",
                    )),
                ],
            })
            .unwrap();
    };

    if permuted {
        add_corridor(&mut builder);
        add_section(&mut builder);
        add_group(&mut builder);
        add_band(&mut builder);
        add_edges(&mut builder);
    } else {
        add_edges(&mut builder);
        add_band(&mut builder);
        add_group(&mut builder);
        add_section(&mut builder);
        add_corridor(&mut builder);
    }
    builder.finish().unwrap()
}

fn spatial_cross_section_unit(
    permuted: bool,
    facility_a_z: f32,
    include_facility_geometry: bool,
) -> CompilationUnit {
    spatial_cross_section_unit_with_frame(permuted, facility_a_z, include_facility_geometry, false)
}

fn spatial_cross_section_unit_with_frame(
    permuted: bool,
    facility_a_z: f32,
    include_facility_geometry: bool,
    imported_facility_frame: bool,
) -> CompilationUnit {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "city/spatial-cross-section",
            source_document_key: "spatial-cross-section.document",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: Some(42),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .unwrap();
    let mut builder = SyntheticModuleBuilder::new(header, &limits).unwrap();
    if imported_facility_frame {
        builder.add_import("city/base").unwrap();
    }
    let lane_points = [
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
    let lane_geometry = [LaneEdgeGeometryInput {
        lane_edge: LaneEdgeReference::local("edge-main"),
        centerline_points: &lane_points,
    }];
    let corridor_elements = [
        CorridorElementReference::facility_band(FacilityBandReference::local("band-z")),
        CorridorElementReference::road_section(RoadSectionReference::local("carriageway")),
        CorridorElementReference::facility_band(FacilityBandReference::local("band-a")),
    ];
    let add_edge = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-main",
                length_meters: 10.0,
                speed_limit_meters_per_second: 12.0,
                successors: &[],
            })
            .unwrap();
    };
    let add_bands = |builder: &mut SyntheticModuleBuilder, reverse: bool| {
        let keys = if reverse {
            ["band-z", "band-a"]
        } else {
            ["band-a", "band-z"]
        };
        for key in keys {
            builder
                .add_facility_band(FacilityBandInput {
                    facility_band_key: key,
                    kind_id: "sidewalk",
                })
                .unwrap();
        }
    };
    let add_section = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_road_section(RoadSectionInput {
                road_section_key: "carriageway",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-main",
                    edge_chain: &[LaneEdgeReference::local("edge-main")],
                    lane_group: None,
                }],
            })
            .unwrap();
    };
    let add_corridor = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: "main-road",
                reference_section: RoadSectionReference::local("carriageway"),
                elements: &corridor_elements,
            })
            .unwrap();
    };
    let add_frame = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_canonical_frame(CanonicalFrameInput {
                canonical_frame_key: "frame-main",
                lane_edge_geometries: &lane_geometry,
            })
            .unwrap();
    };

    if permuted {
        add_corridor(&mut builder);
        add_frame(&mut builder);
        add_bands(&mut builder, true);
        add_section(&mut builder);
        add_edge(&mut builder);
    } else {
        add_edge(&mut builder);
        add_bands(&mut builder, false);
        add_section(&mut builder);
        add_corridor(&mut builder);
        add_frame(&mut builder);
    }

    let source_module = builder.finish().unwrap();
    let mut unit = if imported_facility_frame {
        let base_header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: "city/base",
                source_document_key: "base.document",
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: Some(42),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .unwrap();
        let mut base = SyntheticModuleBuilder::new(base_header, &limits).unwrap();
        base.add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "world",
            lane_edge_geometries: &[],
        })
        .unwrap();
        unit([source_module, base.finish().unwrap()])
    } else {
        unit([source_module])
    };
    if include_facility_geometry {
        let module = unit
            .modules
            .iter_mut()
            .find(|module| {
                module.descriptor().authoring_namespace_id() == "city/spatial-cross-section"
            })
            .expect("fixture contains its cross-section module");
        let namespace: Arc<str> = module.descriptor().authoring_namespace_id().into();
        for declaration in &mut module.declarations {
            let TypedAstDeclaration::FacilityBand(band) = declaration else {
                continue;
            };
            let z = if band.header.stable_key.as_ref() == "band-a" {
                facility_a_z
            } else {
                4.0
            };
            band.compiled_geometry = Some(CompiledFacilityBandGeometry {
                length: EdgeLength::try_new(10.0).unwrap(),
                canonical_frame: OwnedEntityReference::<CanonicalFrameKind>::new(
                    if imported_facility_frame {
                        Arc::from("city/base")
                    } else {
                        Arc::clone(&namespace)
                    },
                    if imported_facility_frame {
                        Arc::from("world")
                    } else {
                        Arc::from("frame-main")
                    },
                    band.header.span.clone(),
                ),
                centerline_points: [
                    CanonicalPoint3F32Input { x: 0.0, y: 0.0, z },
                    CanonicalPoint3F32Input { x: 10.0, y: 0.0, z },
                ]
                .into(),
                source_ranges: Box::new([]),
            });
        }
    }
    unit
}

fn junction_builder(document: &str) -> SyntheticModuleBuilder {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "city/junction",
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

fn junction_module(permuted: bool, selected_internal: &'static str) -> SyntheticModule {
    let mut builder = junction_builder(if permuted {
        "junction-permuted.document"
    } else {
        "junction.document"
    });
    let add_edges = |builder: &mut SyntheticModuleBuilder| {
        let internal_successors = [LaneEdgeReference::local("exit")];
        let entry_successors = [
            LaneEdgeReference::local("internal-a"),
            LaneEdgeReference::local("internal-b"),
        ];
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "entry-a",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &entry_successors,
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "entry-b",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &entry_successors,
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "internal-a",
                length_meters: 8.0,
                speed_limit_meters_per_second: 8.0,
                successors: &internal_successors,
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "internal-b",
                length_meters: 8.0,
                speed_limit_meters_per_second: 8.0,
                successors: &internal_successors,
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "exit",
                length_meters: 12.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap();
    };
    let add_junction = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_junction(JunctionInput {
                junction_key: "junction-main",
            })
            .unwrap();
    };
    let add_movement = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_movement(MovementInput {
                movement_key: "movement-through",
                junction: JunctionReference::local("junction-main"),
                directed_entry_approach_key: "approach-westbound",
                directed_exit_approach_key: "approach-eastbound",
            })
            .unwrap();
    };
    let add_path = |builder: &mut SyntheticModuleBuilder, key: &str, entry: &str| {
        let internal = [LaneEdgeReference::local(selected_internal)];
        builder
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: key,
                movement: MovementReference::local("movement-through"),
                entry_edge: LaneEdgeReference::local(entry),
                internal_edges: &internal,
                exit_edge: LaneEdgeReference::local("exit"),
            })
            .unwrap();
    };

    if permuted {
        add_path(&mut builder, "path-b", "entry-b");
        add_path(&mut builder, "path-a", "entry-a");
        add_movement(&mut builder);
        add_junction(&mut builder);
        add_edges(&mut builder);
    } else {
        add_edges(&mut builder);
        add_junction(&mut builder);
        add_movement(&mut builder);
        add_path(&mut builder, "path-a", "entry-a");
        add_path(&mut builder, "path-b", "entry-b");
    }
    builder.finish().unwrap()
}

fn control_builder(document: &str) -> SyntheticModuleBuilder {
    let mut builder = junction_builder(document);
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
        .unwrap();
    builder
}

fn branched_control_builder(document: &str, include_right_path: bool) -> SyntheticModuleBuilder {
    let mut builder = junction_builder(document);
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[
                LaneEdgeReference::local("middle-left"),
                LaneEdgeReference::local("middle-right"),
            ],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "middle-left",
            length_meters: 8.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[LaneEdgeReference::local("exit-left")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "middle-right",
            length_meters: 8.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[LaneEdgeReference::local("exit-right")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit-left",
            length_meters: 12.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit-right",
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
            maneuver_path_key: "path-left",
            movement: MovementReference::local("movement-through"),
            entry_edge: LaneEdgeReference::local("entry"),
            internal_edges: &[LaneEdgeReference::local("middle-left")],
            exit_edge: LaneEdgeReference::local("exit-left"),
        })
        .unwrap();
    if include_right_path {
        builder
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: "path-right",
                movement: MovementReference::local("movement-through"),
                entry_edge: LaneEdgeReference::local("entry"),
                internal_edges: &[LaneEdgeReference::local("middle-right")],
                exit_edge: LaneEdgeReference::local("exit-right"),
            })
            .unwrap();
    }
    builder
}

fn route_validation_builder(document: &str) -> SyntheticModuleBuilder {
    let mut builder = junction_builder(document);
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[LaneEdgeReference::local("middle")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "other",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[LaneEdgeReference::local("middle")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "middle",
            length_meters: 8.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[
                LaneEdgeReference::local("exit"),
                LaneEdgeReference::local("detour"),
            ],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit",
            length_meters: 12.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "detour",
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
        .unwrap();
    builder
}

fn add_valid_control(builder: &mut SyntheticModuleBuilder, permuted: bool) {
    let add_stops = |builder: &mut SyntheticModuleBuilder| {
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
            .unwrap();
    };
    let add_gates = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: "gate-entry",
                maneuver_path: ManeuverPathReference::local("path-main"),
                transition_index: 0,
                stop_line: StopLineReference::local("stop-entry"),
                signal_control: SignalControlInput::None,
            })
            .unwrap()
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: "gate-release",
                maneuver_path: ManeuverPathReference::local("path-main"),
                transition_index: 1,
                stop_line: StopLineReference::local("stop-middle"),
                signal_control: SignalControlInput::None,
            })
            .unwrap();
    };
    let add_waiting = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_waiting_zone(WaitingZoneInput {
                waiting_zone_key: "waiting-main",
                maneuver_path: ManeuverPathReference::local("path-main"),
                entry_gate: ManeuverGateReference::local("gate-entry"),
                release_gate: ManeuverGateReference::local("gate-release"),
                max_occupancy: 3,
            })
            .unwrap();
    };
    if permuted {
        add_waiting(builder);
        add_gates(builder);
        add_stops(builder);
    } else {
        add_stops(builder);
        add_gates(builder);
        add_waiting(builder);
    }
}

fn signal_module(permuted: bool) -> SyntheticModule {
    let mut builder = control_builder(if permuted {
        "signal-permuted.document"
    } else {
        "signal.document"
    });
    let add_stops = |builder: &mut SyntheticModuleBuilder| {
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
            .unwrap();
    };
    let add_groups = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_signal_group(SignalGroupInput {
                signal_group_key: "group-entry",
            })
            .unwrap()
            .add_signal_group(SignalGroupInput {
                signal_group_key: "group-release",
            })
            .unwrap();
    };
    let add_gates = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: "gate-entry",
                maneuver_path: ManeuverPathReference::local("path-main"),
                transition_index: 0,
                stop_line: StopLineReference::local("stop-entry"),
                signal_control: SignalControlInput::Group(SignalGroupReference::local(
                    "group-entry",
                )),
            })
            .unwrap()
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: "gate-release",
                maneuver_path: ManeuverPathReference::local("path-main"),
                transition_index: 1,
                stop_line: StopLineReference::local("stop-middle"),
                signal_control: SignalControlInput::Group(SignalGroupReference::local(
                    "group-release",
                )),
            })
            .unwrap();
    };
    let add_controller = |builder: &mut SyntheticModuleBuilder, reverse_sets: bool| {
        let groups = if reverse_sets {
            [
                SignalGroupReference::local("group-release"),
                SignalGroupReference::local("group-entry"),
            ]
        } else {
            [
                SignalGroupReference::local("group-entry"),
                SignalGroupReference::local("group-release"),
            ]
        };
        let go_states = if reverse_sets {
            [
                SignalGroupStateInput {
                    signal_group: SignalGroupReference::local("group-release"),
                    aspect: SignalAspect::Red,
                },
                SignalGroupStateInput {
                    signal_group: SignalGroupReference::local("group-entry"),
                    aspect: SignalAspect::Green,
                },
            ]
        } else {
            [
                SignalGroupStateInput {
                    signal_group: SignalGroupReference::local("group-entry"),
                    aspect: SignalAspect::Green,
                },
                SignalGroupStateInput {
                    signal_group: SignalGroupReference::local("group-release"),
                    aspect: SignalAspect::Red,
                },
            ]
        };
        let clear_states = if reverse_sets {
            [
                SignalGroupStateInput {
                    signal_group: SignalGroupReference::local("group-release"),
                    aspect: SignalAspect::Green,
                },
                SignalGroupStateInput {
                    signal_group: SignalGroupReference::local("group-entry"),
                    aspect: SignalAspect::Yellow,
                },
            ]
        } else {
            [
                SignalGroupStateInput {
                    signal_group: SignalGroupReference::local("group-entry"),
                    aspect: SignalAspect::Yellow,
                },
                SignalGroupStateInput {
                    signal_group: SignalGroupReference::local("group-release"),
                    aspect: SignalAspect::Green,
                },
            ]
        };
        builder
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
    };
    if permuted {
        add_controller(&mut builder, true);
        add_gates(&mut builder);
        add_groups(&mut builder);
        add_stops(&mut builder);
    } else {
        add_stops(&mut builder);
        add_groups(&mut builder);
        add_gates(&mut builder);
        add_controller(&mut builder, false);
    }
    builder.finish().unwrap()
}

fn single_signal_group_builder(document: &str) -> SyntheticModuleBuilder {
    let mut builder = control_builder(document);
    builder
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-entry",
            lane_edge: LaneEdgeReference::local("entry"),
        })
        .unwrap()
        .add_signal_group(SignalGroupInput {
            signal_group_key: "group-main",
        })
        .unwrap()
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-entry",
            maneuver_path: ManeuverPathReference::local("path-main"),
            transition_index: 0,
            stop_line: StopLineReference::local("stop-entry"),
            signal_control: SignalControlInput::Group(SignalGroupReference::local("group-main")),
        })
        .unwrap();
    builder
}

fn stable_key<'a>(
    mut fields: impl Iterator<Item = CanonicalIdentityFieldView<'a>>,
    tag: FieldTag,
) -> String {
    fields
        .find(|field| field.tag() == tag)
        .map(|field| String::from_utf8(field.value_bytes().to_vec()).unwrap())
        .unwrap()
}

fn compile_diagnostic_codes(builder: SyntheticModuleBuilder) -> Vec<DiagnosticCode> {
    match Compiler::new().compile(unit([builder.finish().unwrap()])) {
        Ok(_) => panic!("expected junction topology validation failure"),
        Err(diagnostics) => diagnostics
            .diagnostics()
            .iter()
            .map(Diagnostic::code)
            .collect(),
    }
}

fn parking_builder(document: &str) -> SyntheticModuleBuilder {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "city/parking",
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

fn access_builder(document: &str) -> SyntheticModuleBuilder {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "city/access",
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
    let mut builder = SyntheticModuleBuilder::new(header, &limits).unwrap();
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-main",
            length_meters: 20.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap();
    builder
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

fn access_semantics_module(permuted: bool) -> SyntheticModule {
    let mut builder = access_builder("access-semantic.document");
    let add_root = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "road-user",
                extends: None,
            })
            .unwrap();
    };
    let add_child = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "car",
                extends: Some(ParticipantClassReference::local("road-user")),
            })
            .unwrap();
    };
    let add_allow = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_access_rule(AccessRuleInput {
                access_rule_key: "allow-road-users",
                target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-main")),
                effect: AccessEffect::Allow,
                participant_classes: &[ParticipantClassReference::local("road-user")],
                regulation: Some(AccessRegulationInput {
                    jurisdiction: "CN-test",
                    version: "2026-01",
                    source: Some("fixture"),
                }),
                priority: 0,
            })
            .unwrap();
    };
    let add_deny = |builder: &mut SyntheticModuleBuilder| {
        builder
            .add_access_rule(AccessRuleInput {
                access_rule_key: "deny-cars",
                target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-main")),
                effect: AccessEffect::Deny,
                participant_classes: &[ParticipantClassReference::local("car")],
                regulation: None,
                priority: 0,
            })
            .unwrap();
    };
    if permuted {
        add_deny(&mut builder);
        add_allow(&mut builder);
        add_child(&mut builder);
        add_root(&mut builder);
    } else {
        add_root(&mut builder);
        add_child(&mut builder);
        add_allow(&mut builder);
        add_deny(&mut builder);
    }
    builder.finish().unwrap()
}

fn add_parking_edges(builder: &mut SyntheticModuleBuilder) {
    for key in ["parking-entry", "parking-exit"] {
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: key,
                length_meters: 20.0,
                speed_limit_meters_per_second: 8.0,
                successors: &[],
            })
            .unwrap();
    }
}

fn add_parking_space(builder: &mut SyntheticModuleBuilder, key: &str, area: Option<&str>) {
    builder
        .add_parking_space(ParkingSpaceInput {
            parking_space_key: key,
            parking_area: area.map(ParkingAreaReference::local),
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
}

fn parking_module(document: &str, area_key: &str, permuted: bool) -> SyntheticModule {
    let mut builder = parking_builder(document);
    if permuted {
        add_parking_space(&mut builder, "space-independent", None);
        add_parking_space(&mut builder, "space-owned", Some(area_key));
        builder
            .add_parking_area(ParkingAreaInput {
                parking_area_key: area_key,
            })
            .unwrap();
        add_parking_edges(&mut builder);
    } else {
        add_parking_edges(&mut builder);
        builder
            .add_parking_area(ParkingAreaInput {
                parking_area_key: area_key,
            })
            .unwrap();
        add_parking_space(&mut builder, "space-owned", Some(area_key));
        add_parking_space(&mut builder, "space-independent", None);
    }
    builder.finish().unwrap()
}

fn edge_key(edge: CanonicalLaneEdgeView<'_>) -> String {
    edge.identity_fields()
        .find(|field| field.tag() == FieldTag::LaneEdgeKey)
        .map(|field| String::from_utf8(field.value_bytes().to_vec()).unwrap())
        .unwrap()
}

#[test]
fn compiler_atomically_returns_lir_source_map_and_success_diagnostics() {
    let successors = [LaneEdgeReference::imported("city/base", "edge-b")];
    let input = unit([
        module(
            "city/app",
            "app.document",
            &["city/base"],
            &[("edge-a", 10.0, &successors)],
        ),
        module("city/base", "base.document", &[], &[("edge-b", 20.0, &[])]),
    ]);
    let mut compiler = Compiler::new();
    let output = compiler.compile(input).unwrap();
    let candidate = crate::emit_portable_candidate(
        &output,
        &crate::PortableEmissionProvenance::try_new("laneflow-test-build").unwrap(),
        laneflow_format::FormatLimits::HARD,
        crate::PortableDiffBase::Genesis,
    )
    .unwrap();

    assert!(output.diagnostics().is_empty());
    assert_eq!(&candidate.canonical_artifact().bytes()[..4], b"LFCA");
    assert_eq!(&candidate.source_map().bytes()[..4], b"LFSM");
    assert_eq!(&candidate.semantic_diff().bytes()[..4], b"LFSD");
    let wrong_base = laneflow_format::preflight_object_values(
        candidate.source_map().bytes(),
        laneflow_static_contract::PortableObjectKind::SourceMap,
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap();
    assert_eq!(
        crate::emit_portable_candidate(
            &output,
            &crate::PortableEmissionProvenance::try_new("laneflow-test-build").unwrap(),
            laneflow_format::FormatLimits::HARD,
            crate::PortableDiffBase::Artifact(wrong_base),
        ),
        Err(crate::PortableEmissionError::InvalidDiffBaseKind)
    );
    let base = laneflow_format::preflight_object_values(
        candidate.canonical_artifact().bytes(),
        laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap();
    let same_artifact_diff = crate::emit_portable_candidate(
        &output,
        &crate::PortableEmissionProvenance::try_new("laneflow-test-build").unwrap(),
        laneflow_format::FormatLimits::HARD,
        crate::PortableDiffBase::Artifact(base),
    )
    .unwrap();
    let diff = laneflow_format::preflight_object_values(
        same_artifact_diff.semantic_diff().bytes(),
        laneflow_static_contract::PortableObjectKind::SemanticDiff,
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap()
    .registry_view();
    assert!(matches!(
        diff.section(0)
            .unwrap()
            .table(0)
            .unwrap()
            .row(0)
            .unwrap()
            .field_by_tag(1)
            .unwrap()
            .value()
            .unwrap(),
        laneflow_format::RegistryCheckedFieldValue::U8(1)
    ));
    for section_ordinal in 1..6 {
        assert_eq!(
            diff.section(section_ordinal)
                .unwrap()
                .table(0)
                .unwrap()
                .row_count(),
            0
        );
    }
    let metrics = output.metrics();
    assert_eq!(
        metrics.lir_record_count(),
        output.lir.inner.lir_record_count
    );
    assert_eq!(
        metrics.output_logical_bytes(),
        output.lir.inner.output_bytes
    );
    assert!(metrics.compiler_controlled_peak_bytes() >= output.lir.inner.controlled_live_bytes);
    assert_eq!(
        metrics.semantic_fingerprint(),
        output.lir.inner.semantic_digest
    );
    assert_eq!(compiler.retained_capacity_bytes(), 0);
    let edges = output.lir().lane_edges().collect::<Vec<_>>();
    assert_eq!(edges.len(), 2);
    assert_eq!(edge_key(edges[0]), "edge-a");
    assert_eq!(edges[0].ordinal().raw(), 0);
    assert_eq!(edges[0].successors(), [LaneEdgeOrdinal::from_raw(1)]);
    assert_eq!(edges[0].length_meters(), 10.0);
    assert_eq!(edges[0].speed_limit_meters_per_second(), 13.75);
    assert_eq!(
        output
            .lir()
            .lane_edge(edges[1].ordinal())
            .unwrap()
            .stable_id(),
        edges[1].stable_id()
    );

    let modules = output
        .source_map_input()
        .source_modules()
        .map(SourceModuleDescriptor::authoring_namespace_id)
        .collect::<Vec<_>>();
    assert_eq!(modules, ["city/base", "city/app"]);
    let documents = output
        .source_map_input()
        .source_documents()
        .map(|document| document.source_document_key())
        .collect::<Vec<_>>();
    assert_eq!(documents, ["base.document", "app.document"]);

    let entity_sources = output
        .source_map_input()
        .lane_edge_sources()
        .collect::<Vec<_>>();
    assert_eq!(entity_sources.len(), 2);
    for (edge, source) in edges.iter().zip(entity_sources) {
        assert_eq!(source.ordinal(), edge.ordinal());
        assert_eq!(source.stable_id(), edge.stable_id());
        assert!(source.contributing_sources().next().is_none());
    }
    assert_eq!(
        output
            .source_map_input()
            .lane_edge_successor_sources()
            .map(|source| (
                source.owner_ordinal().raw(),
                source.role(),
                source.local_index(),
                source.primary_source().source_document_key().to_owned(),
            ))
            .collect::<Vec<_>>(),
        [(
            0,
            SourceRelationRole::LaneEdgeSuccessor,
            0,
            "app.document".to_owned(),
        )]
    );
}

#[test]
fn portable_artifact_diff_classifies_retained_fields_and_relations() {
    let base_output = Compiler::new()
        .compile(unit([
            module(
                "city/app",
                "app.document",
                &["city/base"],
                &[("edge-a", 10.0, &[])],
            ),
            module("city/base", "base.document", &[], &[("edge-b", 20.0, &[])]),
        ]))
        .unwrap();
    let provenance = crate::PortableEmissionProvenance::try_new("laneflow-test-build").unwrap();
    let base_candidate = crate::emit_portable_candidate(
        &base_output,
        &provenance,
        laneflow_format::FormatLimits::HARD,
        crate::PortableDiffBase::Genesis,
    )
    .unwrap();
    let base = laneflow_format::preflight_object_values(
        base_candidate.canonical_artifact().bytes(),
        laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap();

    let successors = [LaneEdgeReference::imported("city/base", "edge-b")];
    let target_output = Compiler::new()
        .compile(unit([
            module(
                "city/app",
                "app.document",
                &["city/base"],
                &[("edge-a", 11.0, &successors)],
            ),
            module("city/base", "base.document", &[], &[("edge-b", 20.0, &[])]),
        ]))
        .unwrap();
    let candidate = crate::emit_portable_candidate(
        &target_output,
        &provenance,
        laneflow_format::FormatLimits::HARD,
        crate::PortableDiffBase::Artifact(base),
    )
    .unwrap();
    let diff = laneflow_format::preflight_object_values(
        candidate.semantic_diff().bytes(),
        laneflow_static_contract::PortableObjectKind::SemanticDiff,
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap()
    .registry_view();
    let entity_changes = diff.section(1).unwrap().table(0).unwrap();
    let relation_changes = diff.section(2).unwrap().table(0).unwrap();
    assert_eq!(entity_changes.row_count(), 1);
    assert_eq!(relation_changes.row_count(), 1);
    assert!(matches!(
        entity_changes
            .row(0)
            .unwrap()
            .field_by_tag(1)
            .unwrap()
            .value()
            .unwrap(),
        laneflow_format::RegistryCheckedFieldValue::U8(2)
    ));
    let relation = relation_changes.row(0).unwrap();
    assert!(matches!(
        relation.field_by_tag(1).unwrap().value().unwrap(),
        laneflow_format::RegistryCheckedFieldValue::U8(0)
    ));
    assert!(matches!(
        relation.field_by_tag(5).unwrap().value().unwrap(),
        laneflow_format::RegistryCheckedFieldValue::U8(1)
    ));
}

#[test]
fn portable_artifact_diff_rejects_cross_revision_stable_id_collisions() {
    let output = Compiler::new()
        .compile(unit([module(
            "city/collision",
            "collision.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]))
        .unwrap();
    let provenance = crate::PortableEmissionProvenance::try_new("laneflow-test-build").unwrap();
    let candidate = crate::emit_portable_candidate(
        &output,
        &provenance,
        laneflow_format::FormatLimits::HARD,
        crate::PortableDiffBase::Genesis,
    )
    .unwrap();
    let mut colliding_base = candidate.canonical_artifact().bytes().to_vec();
    let key_offset = colliding_base
        .windows(b"edge-a".len())
        .position(|window| window == b"edge-a")
        .unwrap();
    colliding_base[key_offset + b"edge-".len()] = b'z';
    let base = laneflow_format::preflight_object_values(
        &colliding_base,
        laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap();

    assert_eq!(
        crate::emit_portable_candidate(
            &output,
            &provenance,
            laneflow_format::FormatLimits::HARD,
            crate::PortableDiffBase::Artifact(base),
        ),
        Err(crate::PortableEmissionError::CrossRevisionStableIdCollision)
    );
}

#[test]
fn portable_emitter_closes_every_current_relation_family_and_spatial_projection() {
    let provenance = crate::PortableEmissionProvenance::try_new("laneflow-test-build").unwrap();
    let mut emitted_roles = std::collections::BTreeSet::new();
    let mut emit = |output: &CompilationOutput| {
        let candidate = crate::emit_portable_candidate(
            output,
            &provenance,
            laneflow_format::FormatLimits::HARD,
            crate::PortableDiffBase::Genesis,
        )
        .unwrap();
        let source_map = laneflow_format::preflight_object_values(
            candidate.source_map().bytes(),
            laneflow_static_contract::PortableObjectKind::SourceMap,
            laneflow_format::FormatLimits::HARD,
        )
        .unwrap()
        .registry_view();
        let owner_local = source_map.section(3).unwrap().table(0).unwrap();
        for ordinal in 0..owner_local.row_count() {
            let row = owner_local.row(ordinal).unwrap();
            let role = match row.field_by_tag(3).unwrap().value().unwrap() {
                laneflow_format::RegistryCheckedFieldValue::U8(role) => role,
                value => panic!("expected owner-local role, got {value:?}"),
            };
            emitted_roles.insert(role);
        }
        candidate
    };

    let rich_relations = Compiler::new()
        .compile(unit([
            cross_section_module(false),
            junction_module(false, "internal-a"),
            parking_module("portable-parking.document", "area-main", false),
            access_semantics_module(false),
        ]))
        .unwrap();
    emit(&rich_relations);

    let signal = Compiler::new()
        .compile(unit([signal_module(false)]))
        .unwrap();
    emit(&signal);

    let mut route_builder = control_builder("portable-route.document");
    add_valid_control(&mut route_builder, false);
    route_builder
        .add_static_route(StaticRouteInput {
            static_route_key: "route-main",
            edge_sequence: &[
                LaneEdgeReference::local("entry"),
                LaneEdgeReference::local("middle"),
                LaneEdgeReference::local("exit"),
            ],
        })
        .unwrap();
    let route = Compiler::new()
        .compile(unit([route_builder.finish().unwrap()]))
        .unwrap();
    emit(&route);

    let mut vehicle_builder = access_builder("portable-vehicle.document");
    vehicle_builder
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "passenger-car",
            extends: None,
        })
        .unwrap()
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "standard-car",
            participant_class: ParticipantClassReference::local("passenger-car"),
            iidm: canonical_iidm_profile(),
        })
        .unwrap();
    let vehicle = Compiler::new()
        .compile(unit([vehicle_builder.finish().unwrap()]))
        .unwrap();
    emit(&vehicle);

    let spatial = Compiler::new()
        .compile(spatial_cross_section_unit(false, 1.0, true))
        .unwrap();
    emit(&spatial);

    assert_eq!(emitted_roles, (1_u8..=29).collect());
}

#[test]
fn compiler_freezes_fixed_time_signal_program_bindings_and_source_relations() {
    let output = Compiler::new()
        .compile(unit([signal_module(false)]))
        .unwrap();
    let lir = output.lir();
    let groups = lir.signal_groups().collect::<Vec<_>>();
    let controller = lir.signal_controllers().next().unwrap();
    let phases = controller
        .phases()
        .iter()
        .map(|ordinal| lir.signal_phase(*ordinal).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(groups.len(), 2);
    assert_eq!(controller.offset_ms(), 1_000);
    assert_eq!(controller.cycle_duration_ms(), 35_000);
    assert_eq!(controller.signal_groups().len(), 2);
    assert_eq!(
        phases
            .iter()
            .map(|phase| phase.duration_ms())
            .collect::<Vec<_>>(),
        [30_000, 5_000]
    );
    assert!(phases.iter().all(|phase| {
        phase.controller() == controller.ordinal()
            && phase
                .states()
                .map(|state| state.signal_group())
                .collect::<Vec<_>>()
                == controller.signal_groups()
    }));
    assert!(groups.iter().all(|group| {
        group.controller() == controller.ordinal() && group.maneuver_gates().len() == 1
    }));
    assert!(
        lir.maneuver_gates()
            .all(|gate| matches!(gate.signal_control(), CanonicalSignalControl::Group(_)))
    );
    assert_eq!(
        phases[0]
            .identity_fields()
            .map(|field| field.tag())
            .collect::<Vec<_>>(),
        [
            FieldTag::AuthoringNamespaceId,
            FieldTag::SignalControllerStableId,
            FieldTag::PhaseKey,
        ]
    );

    let source_map = output.source_map_input();
    assert_eq!(source_map.signal_group_sources().len(), 2);
    assert_eq!(source_map.signal_controller_sources().len(), 1);
    assert_eq!(source_map.signal_phase_sources().len(), 2);
    assert_eq!(source_map.signal_relation_sources().len(), 10);
    assert_eq!(
        source_map
            .signal_relation_sources()
            .fold([0_u32; 4], |mut counts, source| {
                let index = match source.role() {
                    SourceRelationRole::SignalControllerGroup => 0,
                    SourceRelationRole::SignalControllerPhase => 1,
                    SourceRelationRole::SignalPhaseState => 2,
                    SourceRelationRole::ManeuverGateSignalGroup => 3,
                    _ => unreachable!("unexpected signal relation role"),
                };
                counts[index] += 1;
                counts
            }),
        [2, 2, 4, 2]
    );
}

#[test]
fn signal_set_permutation_does_not_change_lir_semantics() {
    let baseline = Compiler::new()
        .compile(unit([signal_module(false)]))
        .unwrap();
    let permuted = Compiler::new()
        .compile(unit([signal_module(true)]))
        .unwrap();
    assert_eq!(
        baseline.lir.inner.semantic_digest,
        permuted.lir.inner.semantic_digest
    );
}

#[test]
fn signal_controller_rejects_empty_group_and_phase_programs() {
    let mut builder = control_builder("signal-invalid.document");
    builder
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: "controller-empty",
            offset_ms: 0,
            signal_groups: &[],
            phases: &[],
        })
        .unwrap();
    assert_eq!(
        compile_diagnostic_codes(builder),
        [
            DiagnosticCode::EmptySignalControllerGroups,
            DiagnosticCode::EmptySignalControllerPhases,
        ]
    );
}

#[test]
fn signal_program_validation_closes_phase_time_and_ownership_boundaries() {
    let valid_state = [SignalGroupStateInput {
        signal_group: SignalGroupReference::local("group-main"),
        aspect: SignalAspect::Red,
    }];

    let mut missing = single_signal_group_builder("signal-missing-state.document");
    missing
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: "controller-main",
            offset_ms: 0,
            signal_groups: &[SignalGroupReference::local("group-main")],
            phases: &[SignalPhaseInput {
                signal_phase_key: "phase-main",
                duration_ms: 100,
                states: &[],
            }],
        })
        .unwrap();
    assert_eq!(
        compile_diagnostic_codes(missing),
        [DiagnosticCode::MissingSignalPhaseGroup]
    );

    let duplicate_states = [valid_state[0], valid_state[0]];
    let mut duplicate = single_signal_group_builder("signal-duplicate-state.document");
    duplicate
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: "controller-main",
            offset_ms: 0,
            signal_groups: &[
                SignalGroupReference::local("group-main"),
                SignalGroupReference::local("group-main"),
            ],
            phases: &[SignalPhaseInput {
                signal_phase_key: "phase-main",
                duration_ms: 100,
                states: &duplicate_states,
            }],
        })
        .unwrap();
    let duplicate_codes = compile_diagnostic_codes(duplicate);
    assert!(duplicate_codes.contains(&DiagnosticCode::DuplicateSignalControllerGroup));
    assert!(duplicate_codes.contains(&DiagnosticCode::DuplicateSignalPhaseGroup));

    let mut invalid_duration = single_signal_group_builder("signal-invalid-duration.document");
    invalid_duration
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: "controller-main",
            offset_ms: 0,
            signal_groups: &[SignalGroupReference::local("group-main")],
            phases: &[SignalPhaseInput {
                signal_phase_key: "phase-main",
                duration_ms: 0,
                states: &valid_state,
            }],
        })
        .unwrap();
    assert_eq!(
        compile_diagnostic_codes(invalid_duration),
        [DiagnosticCode::InvalidSignalPhaseDuration]
    );

    let mut invalid_offset = single_signal_group_builder("signal-invalid-offset.document");
    invalid_offset
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: "controller-main",
            offset_ms: 100,
            signal_groups: &[SignalGroupReference::local("group-main")],
            phases: &[SignalPhaseInput {
                signal_phase_key: "phase-main",
                duration_ms: 100,
                states: &valid_state,
            }],
        })
        .unwrap();
    assert_eq!(
        compile_diagnostic_codes(invalid_offset),
        [DiagnosticCode::InvalidSignalControllerOffset]
    );

    let mut cycle_overflow = single_signal_group_builder("signal-cycle-overflow.document");
    cycle_overflow
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: "controller-main",
            offset_ms: 0,
            signal_groups: &[SignalGroupReference::local("group-main")],
            phases: &[
                SignalPhaseInput {
                    signal_phase_key: "phase-long",
                    duration_ms: 9_007_199_254_740_991,
                    states: &valid_state,
                },
                SignalPhaseInput {
                    signal_phase_key: "phase-overflow",
                    duration_ms: 1,
                    states: &valid_state,
                },
            ],
        })
        .unwrap();
    assert_eq!(
        compile_diagnostic_codes(cycle_overflow),
        [DiagnosticCode::SignalCycleDurationOverflow]
    );

    let mut multiple_owner = single_signal_group_builder("signal-owner.document");
    for controller_key in ["controller-a", "controller-b"] {
        multiple_owner
            .add_signal_controller(SignalControllerInput {
                signal_controller_key: controller_key,
                offset_ms: 0,
                signal_groups: &[SignalGroupReference::local("group-main")],
                phases: &[SignalPhaseInput {
                    signal_phase_key: "phase-main",
                    duration_ms: 100,
                    states: &valid_state,
                }],
            })
            .unwrap();
    }
    assert!(
        compile_diagnostic_codes(multiple_owner)
            .contains(&DiagnosticCode::SignalGroupMultipleControllers)
    );
}

#[test]
fn signal_group_reference_failure_is_reported_even_without_signal_entities() {
    let mut builder = control_builder("signal-unknown-group.document");
    builder
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-entry",
            lane_edge: LaneEdgeReference::local("entry"),
        })
        .unwrap()
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-entry",
            maneuver_path: ManeuverPathReference::local("path-main"),
            transition_index: 0,
            stop_line: StopLineReference::local("stop-entry"),
            signal_control: SignalControlInput::Group(SignalGroupReference::local("group-missing")),
        })
        .unwrap();
    assert_eq!(
        compile_diagnostic_codes(builder),
        [DiagnosticCode::UnknownReferenceTarget]
    );
}

#[test]
fn signal_validation_reports_local_identity_and_global_closure_failures() {
    let valid_state = [SignalGroupStateInput {
        signal_group: SignalGroupReference::local("group-main"),
        aspect: SignalAspect::Red,
    }];

    let mut duplicate_phase = single_signal_group_builder("signal-duplicate-phase.document");
    duplicate_phase
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: "controller-main",
            offset_ms: 0,
            signal_groups: &[SignalGroupReference::local("group-main")],
            phases: &[
                SignalPhaseInput {
                    signal_phase_key: "phase-main",
                    duration_ms: 100,
                    states: &valid_state,
                },
                SignalPhaseInput {
                    signal_phase_key: "phase-main",
                    duration_ms: 100,
                    states: &valid_state,
                },
            ],
        })
        .unwrap();
    assert_eq!(
        compile_diagnostic_codes(duplicate_phase),
        [DiagnosticCode::DuplicateSignalPhaseKey]
    );

    let mut foreign_phase_group =
        single_signal_group_builder("signal-foreign-phase-group.document");
    foreign_phase_group
        .add_signal_group(SignalGroupInput {
            signal_group_key: "group-foreign",
        })
        .unwrap();
    let states = [
        valid_state[0],
        SignalGroupStateInput {
            signal_group: SignalGroupReference::local("group-foreign"),
            aspect: SignalAspect::Green,
        },
    ];
    foreign_phase_group
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: "controller-main",
            offset_ms: 0,
            signal_groups: &[SignalGroupReference::local("group-main")],
            phases: &[SignalPhaseInput {
                signal_phase_key: "phase-main",
                duration_ms: 100,
                states: &states,
            }],
        })
        .unwrap();
    let foreign_group_codes = compile_diagnostic_codes(foreign_phase_group);
    assert!(foreign_group_codes.contains(&DiagnosticCode::UnknownSignalPhaseGroup));
    assert!(foreign_group_codes.contains(&DiagnosticCode::UnownedSignalGroup));
    assert!(foreign_group_codes.contains(&DiagnosticCode::UnusedSignalGroup));

    let mut orphan_group = control_builder("signal-orphan-group.document");
    orphan_group
        .add_signal_group(SignalGroupInput {
            signal_group_key: "group-orphan",
        })
        .unwrap();
    let orphan_codes = compile_diagnostic_codes(orphan_group);
    assert!(orphan_codes.contains(&DiagnosticCode::UnownedSignalGroup));
    assert!(orphan_codes.contains(&DiagnosticCode::UnusedSignalGroup));
}

#[test]
fn compiler_freezes_complete_cross_section_owner_tree_and_source_relations() {
    let output = Compiler::new()
        .compile(unit([cross_section_module(false)]))
        .unwrap();
    let lir = output.lir();
    let corridor = lir.road_corridors().next().unwrap();
    let section = lir.road_sections().next().unwrap();
    let lane = lir.authoring_lanes().next().unwrap();
    let group = lir.lane_groups().next().unwrap();
    let band = lir.facility_bands().next().unwrap();

    assert_eq!(corridor.reference_section(), section.ordinal());
    assert_eq!(
        corridor.elements().collect::<Vec<_>>(),
        [
            CanonicalCorridorElement::FacilityBand(band.ordinal()),
            CanonicalCorridorElement::RoadSection(section.ordinal()),
        ]
    );
    assert_eq!(section.road_corridor(), corridor.ordinal());
    assert_eq!(section.kind_id(), "motorLane");
    assert_eq!(section.lanes(), [lane.ordinal()]);
    assert_eq!(lane.road_section(), section.ordinal());
    assert_eq!(lane.edge_chain().len(), 2);
    assert_eq!(lane.lane_group(), Some(group.ordinal()));
    assert_eq!(group.road_section(), section.ordinal());
    assert_eq!(group.members(), [lane.ordinal()]);
    assert_eq!(band.road_corridor(), corridor.ordinal());
    assert_eq!(band.kind_id(), "sidewalk");

    let section_fields = section
        .identity_fields()
        .map(|field| (field.tag(), field.value_bytes().to_vec()))
        .collect::<Vec<_>>();
    assert_eq!(
        section_fields
            .iter()
            .map(|field| field.0)
            .collect::<Vec<_>>(),
        [
            FieldTag::AuthoringNamespaceId,
            FieldTag::SectionKey,
            FieldTag::RoadCorridorStableId,
        ]
    );
    assert_eq!(
        section_fields[2].1,
        corridor.stable_id().as_untyped().as_bytes()
    );

    let source_map = output.source_map_input();
    assert_eq!(source_map.road_corridor_sources().len(), 1);
    assert_eq!(source_map.road_section_sources().len(), 1);
    assert_eq!(source_map.authoring_lane_sources().len(), 1);
    assert_eq!(source_map.lane_group_sources().len(), 1);
    assert_eq!(source_map.facility_band_sources().len(), 1);
    assert_eq!(
        source_map
            .cross_section_relation_sources()
            .map(|source| {
                (
                    source.owner().entity_kind(),
                    source.role(),
                    source.local_index(),
                )
            })
            .collect::<Vec<_>>(),
        [
            (
                laneflow_static_contract::EntityKind::RoadCorridor,
                SourceRelationRole::RoadCorridorElement,
                0,
            ),
            (
                laneflow_static_contract::EntityKind::RoadCorridor,
                SourceRelationRole::RoadCorridorElement,
                1,
            ),
            (
                laneflow_static_contract::EntityKind::RoadSection,
                SourceRelationRole::RoadSectionLane,
                0,
            ),
            (
                laneflow_static_contract::EntityKind::AuthoringLane,
                SourceRelationRole::AuthoringLaneEdge,
                0,
            ),
            (
                laneflow_static_contract::EntityKind::AuthoringLane,
                SourceRelationRole::AuthoringLaneEdge,
                1,
            ),
            (
                laneflow_static_contract::EntityKind::LaneGroup,
                SourceRelationRole::LaneGroupMember,
                0,
            ),
        ]
    );
}

#[test]
fn cross_section_lir_semantics_ignore_top_level_declaration_order() {
    let baseline = Compiler::new()
        .compile(unit([cross_section_module(false)]))
        .unwrap();
    let permuted = Compiler::new()
        .compile(unit([cross_section_module(true)]))
        .unwrap();

    assert_eq!(
        baseline.lir.inner.semantic_digest,
        permuted.lir.inner.semantic_digest
    );
    assert_eq!(
        baseline
            .lir()
            .road_corridors()
            .map(|corridor| corridor.stable_id())
            .collect::<Vec<_>>(),
        permuted
            .lir()
            .road_corridors()
            .map(|corridor| corridor.stable_id())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        baseline
            .lir()
            .authoring_lanes()
            .map(|lane| lane.stable_id())
            .collect::<Vec<_>>(),
        permuted
            .lir()
            .authoring_lanes()
            .map(|lane| lane.stable_id())
            .collect::<Vec<_>>()
    );
}

#[test]
fn compiler_freezes_complete_junction_topology_and_source_relations() {
    let output = Compiler::new()
        .compile(unit([junction_module(false, "internal-a")]))
        .unwrap();
    let lir = output.lir();
    let junction = lir.junctions().next().unwrap();
    let movement = lir.movements().next().unwrap();
    let paths = lir.maneuver_paths().collect::<Vec<_>>();
    let edges = lir
        .lane_edges()
        .map(|edge| (edge_key(edge), edge.ordinal()))
        .collect::<std::collections::HashMap<_, _>>();

    assert_eq!(junction.movements(), [movement.ordinal()]);
    assert_eq!(movement.junction(), junction.ordinal());
    assert_eq!(movement.directed_entry_approach_key(), "approach-westbound");
    assert_eq!(movement.directed_exit_approach_key(), "approach-eastbound");
    assert_eq!(movement.maneuver_paths().len(), 2);
    assert_eq!(paths.len(), 2);
    assert_eq!(
        stable_key(paths[0].identity_fields(), FieldTag::PathKey),
        "path-a"
    );
    assert_eq!(
        paths[0].edges(),
        [edges["entry-a"], edges["internal-a"], edges["exit"]]
    );
    assert_eq!(paths[0].entry_edge(), edges["entry-a"]);
    assert_eq!(paths[0].internal_edges(), [edges["internal-a"]]);
    assert_eq!(paths[0].exit_edge(), edges["exit"]);
    assert_eq!(
        lir.junction_internal_owner(edges["internal-a"]),
        Some(junction.ordinal())
    );
    assert_eq!(lir.junction_internal_owner(edges["entry-a"]), None);
    assert_eq!(
        lir.junction_internal_edges()
            .map(|relation| (relation.edge(), relation.junction()))
            .collect::<Vec<_>>(),
        [(edges["internal-a"], junction.ordinal())]
    );
    assert_eq!(
        movement
            .identity_fields()
            .map(|field| field.tag())
            .collect::<Vec<_>>(),
        [
            FieldTag::AuthoringNamespaceId,
            FieldTag::MovementKey,
            FieldTag::DirectedEntryApproachKey,
            FieldTag::DirectedExitApproachKey,
            FieldTag::JunctionStableId,
        ]
    );
    assert_eq!(
        paths[0]
            .identity_fields()
            .map(|field| field.tag())
            .collect::<Vec<_>>(),
        [
            FieldTag::AuthoringNamespaceId,
            FieldTag::PathKey,
            FieldTag::MovementStableId,
            FieldTag::EntryEdgeStableId,
            FieldTag::ExitEdgeStableId,
        ]
    );

    let source_map = output.source_map_input();
    assert_eq!(source_map.junction_sources().len(), 1);
    assert_eq!(source_map.movement_sources().len(), 1);
    assert_eq!(source_map.maneuver_path_sources().len(), 2);
    assert_eq!(
        source_map
            .junction_relation_sources()
            .map(|source| (
                source.owner().entity_kind(),
                source.role(),
                source.local_index()
            ))
            .collect::<Vec<_>>(),
        [
            (
                laneflow_static_contract::EntityKind::Junction,
                SourceRelationRole::JunctionMovement,
                0,
            ),
            (
                laneflow_static_contract::EntityKind::Movement,
                SourceRelationRole::MovementManeuverPath,
                0,
            ),
            (
                laneflow_static_contract::EntityKind::Movement,
                SourceRelationRole::MovementManeuverPath,
                1,
            ),
            (
                laneflow_static_contract::EntityKind::ManeuverPath,
                SourceRelationRole::ManeuverPathEdge,
                0,
            ),
            (
                laneflow_static_contract::EntityKind::ManeuverPath,
                SourceRelationRole::ManeuverPathEdge,
                1,
            ),
            (
                laneflow_static_contract::EntityKind::ManeuverPath,
                SourceRelationRole::ManeuverPathEdge,
                2,
            ),
            (
                laneflow_static_contract::EntityKind::ManeuverPath,
                SourceRelationRole::ManeuverPathEdge,
                0,
            ),
            (
                laneflow_static_contract::EntityKind::ManeuverPath,
                SourceRelationRole::ManeuverPathEdge,
                1,
            ),
            (
                laneflow_static_contract::EntityKind::ManeuverPath,
                SourceRelationRole::ManeuverPathEdge,
                2,
            ),
            (
                laneflow_static_contract::EntityKind::Junction,
                SourceRelationRole::JunctionInternalEdge,
                0,
            ),
        ]
    );
}

#[test]
fn compiler_accepts_a_direct_maneuver_path_without_internal_edges() {
    let mut builder = junction_builder("direct-path.document");
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[LaneEdgeReference::local("exit")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_junction(JunctionInput {
            junction_key: "junction-main",
        })
        .unwrap()
        .add_movement(MovementInput {
            movement_key: "movement-main",
            junction: JunctionReference::local("junction-main"),
            directed_entry_approach_key: "approach-entry",
            directed_exit_approach_key: "approach-exit",
        })
        .unwrap()
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path-direct",
            movement: MovementReference::local("movement-main"),
            entry_edge: LaneEdgeReference::local("entry"),
            internal_edges: &[],
            exit_edge: LaneEdgeReference::local("exit"),
        })
        .unwrap();

    let output = Compiler::new()
        .compile(unit([builder.finish().unwrap()]))
        .unwrap();
    let path = output.lir().maneuver_paths().next().unwrap();
    assert_eq!(path.edges().len(), 2);
    assert!(path.internal_edges().is_empty());
    assert_eq!(output.lir().junction_internal_edges().len(), 0);
}

#[test]
fn junction_lir_is_deterministic_and_path_identity_excludes_internal_edges() {
    let baseline = Compiler::new()
        .compile(unit([junction_module(false, "internal-a")]))
        .unwrap();
    let permuted = Compiler::new()
        .compile(unit([junction_module(true, "internal-a")]))
        .unwrap();
    let different_internal = Compiler::new()
        .compile(unit([junction_module(false, "internal-b")]))
        .unwrap();

    assert_eq!(
        baseline.lir.inner.semantic_digest,
        permuted.lir.inner.semantic_digest
    );
    assert_eq!(
        baseline
            .lir()
            .maneuver_paths()
            .map(|path| path.stable_id())
            .collect::<Vec<_>>(),
        permuted
            .lir()
            .maneuver_paths()
            .map(|path| path.stable_id())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        baseline
            .lir()
            .maneuver_paths()
            .map(|path| path.stable_id())
            .collect::<Vec<_>>(),
        different_internal
            .lir()
            .maneuver_paths()
            .map(|path| path.stable_id())
            .collect::<Vec<_>>()
    );
    assert_ne!(
        baseline.lir.inner.semantic_digest,
        different_internal.lir.inner.semantic_digest
    );
}

#[test]
fn compiler_rejects_junction_topology_semantic_failures_before_lir() {
    let add_junction = |builder: &mut SyntheticModuleBuilder, key: &'static str| {
        builder
            .add_junction(JunctionInput { junction_key: key })
            .unwrap();
    };
    let add_movement =
        |builder: &mut SyntheticModuleBuilder, key: &'static str, junction: &'static str| {
            builder
                .add_movement(MovementInput {
                    movement_key: key,
                    junction: JunctionReference::local(junction),
                    directed_entry_approach_key: "approach-entry",
                    directed_exit_approach_key: "approach-exit",
                })
                .unwrap();
        };
    let add_edge = |builder: &mut SyntheticModuleBuilder,
                    key: &'static str,
                    successors: &[LaneEdgeReference<'static>]| {
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: key,
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors,
            })
            .unwrap();
    };

    let mut empty_junction = junction_builder("empty-junction.document");
    add_junction(&mut empty_junction, "junction-empty");
    assert!(compile_diagnostic_codes(empty_junction).contains(&DiagnosticCode::EmptyJunction));

    let mut empty_movement = junction_builder("empty-movement.document");
    add_junction(&mut empty_movement, "junction-main");
    add_movement(&mut empty_movement, "movement-empty", "junction-main");
    assert!(compile_diagnostic_codes(empty_movement).contains(&DiagnosticCode::EmptyMovement));

    let mut disconnected = junction_builder("disconnected-path.document");
    add_edge(&mut disconnected, "entry", &[]);
    add_edge(&mut disconnected, "exit", &[]);
    add_junction(&mut disconnected, "junction-main");
    add_movement(&mut disconnected, "movement-main", "junction-main");
    disconnected
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path-main",
            movement: MovementReference::local("movement-main"),
            entry_edge: LaneEdgeReference::local("entry"),
            internal_edges: &[],
            exit_edge: LaneEdgeReference::local("exit"),
        })
        .unwrap();
    assert!(
        compile_diagnostic_codes(disconnected).contains(&DiagnosticCode::DisconnectedManeuverPath)
    );

    let mut duplicate = junction_builder("duplicate-path.document");
    add_edge(&mut duplicate, "entry", &[LaneEdgeReference::local("exit")]);
    add_edge(&mut duplicate, "exit", &[]);
    add_junction(&mut duplicate, "junction-main");
    add_movement(&mut duplicate, "movement-main", "junction-main");
    for path_key in ["path-a", "path-b"] {
        duplicate
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: path_key,
                movement: MovementReference::local("movement-main"),
                entry_edge: LaneEdgeReference::local("entry"),
                internal_edges: &[],
                exit_edge: LaneEdgeReference::local("exit"),
            })
            .unwrap();
    }
    assert!(
        compile_diagnostic_codes(duplicate)
            .contains(&DiagnosticCode::DuplicateManeuverPathSequence)
    );

    let mut cross_junction = junction_builder("cross-junction-internal.document");
    add_edge(
        &mut cross_junction,
        "entry-a",
        &[LaneEdgeReference::local("internal")],
    );
    add_edge(
        &mut cross_junction,
        "entry-b",
        &[LaneEdgeReference::local("internal")],
    );
    add_edge(
        &mut cross_junction,
        "internal",
        &[
            LaneEdgeReference::local("exit-a"),
            LaneEdgeReference::local("exit-b"),
        ],
    );
    add_edge(&mut cross_junction, "exit-a", &[]);
    add_edge(&mut cross_junction, "exit-b", &[]);
    for suffix in ["a", "b"] {
        let junction_key = if suffix == "a" {
            "junction-a"
        } else {
            "junction-b"
        };
        let movement_key = if suffix == "a" {
            "movement-a"
        } else {
            "movement-b"
        };
        add_junction(&mut cross_junction, junction_key);
        add_movement(&mut cross_junction, movement_key, junction_key);
        let internal = [LaneEdgeReference::local("internal")];
        cross_junction
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: if suffix == "a" { "path-a" } else { "path-b" },
                movement: MovementReference::local(movement_key),
                entry_edge: LaneEdgeReference::local(if suffix == "a" {
                    "entry-a"
                } else {
                    "entry-b"
                }),
                internal_edges: &internal,
                exit_edge: LaneEdgeReference::local(if suffix == "a" {
                    "exit-a"
                } else {
                    "exit-b"
                }),
            })
            .unwrap();
    }
    assert!(
        compile_diagnostic_codes(cross_junction)
            .contains(&DiagnosticCode::InternalEdgeJunctionConflict)
    );

    let mut boundary_conflict = junction_builder("internal-boundary-conflict.document");
    add_edge(
        &mut boundary_conflict,
        "entry",
        &[LaneEdgeReference::local("internal")],
    );
    add_edge(
        &mut boundary_conflict,
        "internal",
        &[
            LaneEdgeReference::local("exit-a"),
            LaneEdgeReference::local("exit-b"),
        ],
    );
    add_edge(&mut boundary_conflict, "exit-a", &[]);
    add_edge(&mut boundary_conflict, "exit-b", &[]);
    add_junction(&mut boundary_conflict, "junction-main");
    add_movement(&mut boundary_conflict, "movement-main", "junction-main");
    let internal = [LaneEdgeReference::local("internal")];
    boundary_conflict
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path-with-internal",
            movement: MovementReference::local("movement-main"),
            entry_edge: LaneEdgeReference::local("entry"),
            internal_edges: &internal,
            exit_edge: LaneEdgeReference::local("exit-a"),
        })
        .unwrap()
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path-with-boundary",
            movement: MovementReference::local("movement-main"),
            entry_edge: LaneEdgeReference::local("internal"),
            internal_edges: &[],
            exit_edge: LaneEdgeReference::local("exit-b"),
        })
        .unwrap();
    assert!(
        compile_diagnostic_codes(boundary_conflict)
            .contains(&DiagnosticCode::InternalBoundaryRoleConflict)
    );
}

#[test]
fn movement_approach_identity_fields_reject_non_ascii_input_atomically() {
    let mut builder = junction_builder("invalid-approach.document");
    builder
        .add_junction(JunctionInput {
            junction_key: "junction-main",
        })
        .unwrap();
    let diagnostic = match builder.add_movement(MovementInput {
        movement_key: "movement-main",
        junction: JunctionReference::local("junction-main"),
        directed_entry_approach_key: "入口",
        directed_exit_approach_key: "approach-exit",
    }) {
        Ok(_) => panic!("non-ASCII identity field must reject the declaration"),
        Err(diagnostic) => diagnostic,
    };
    assert_eq!(
        diagnostic.diagnostics()[0].code(),
        DiagnosticCode::InvalidIdentityAsciiField
    );
    // 同一个稳定键仍可被合法声明，证明失败路径没有预占符号或部分提交资源计数。
    builder
        .add_movement(MovementInput {
            movement_key: "movement-main",
            junction: JunctionReference::local("junction-main"),
            directed_entry_approach_key: "approach-entry",
            directed_exit_approach_key: "approach-exit",
        })
        .unwrap();
}

#[test]
fn compiler_rejects_cross_section_semantic_failures_before_lir() {
    let limits = CompileLimits::p100_initial_v1();
    let make_builder = || {
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: "city/failure",
                source_document_key: "failure.document",
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: None,
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .unwrap();
        SyntheticModuleBuilder::new(header, &limits).unwrap()
    };

    let mut missing_owner = make_builder();
    missing_owner
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "section-a",
            kind_id: "motorLane",
            lanes: &[AuthoringLaneInput {
                authoring_lane_key: "lane-a",
                edge_chain: &[LaneEdgeReference::local("edge-a")],
                lane_group: None,
            }],
        })
        .unwrap();
    let diagnostics = match Compiler::new().compile(unit([missing_owner.finish().unwrap()])) {
        Ok(_) => panic!("missing owner must reject compilation"),
        Err(diagnostics) => diagnostics,
    };
    assert_eq!(
        diagnostics.diagnostics()[0].code(),
        DiagnosticCode::MissingCrossSectionOwner
    );

    let mut disconnected = make_builder();
    disconnected
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-b",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "section-a",
            kind_id: "motorLane",
            lanes: &[AuthoringLaneInput {
                authoring_lane_key: "lane-a",
                edge_chain: &[
                    LaneEdgeReference::local("edge-a"),
                    LaneEdgeReference::local("edge-b"),
                ],
                lane_group: None,
            }],
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "corridor-a",
            reference_section: RoadSectionReference::local("section-a"),
            elements: &[CorridorElementReference::road_section(
                RoadSectionReference::local("section-a"),
            )],
        })
        .unwrap();
    let diagnostics = match Compiler::new().compile(unit([disconnected.finish().unwrap()])) {
        Ok(_) => panic!("disconnected lane chain must reject compilation"),
        Err(diagnostics) => diagnostics,
    };
    assert!(diagnostics.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == DiagnosticCode::DisconnectedAuthoringLaneEdgeChain
    }));

    let mut unknown_middle = make_builder();
    unknown_middle
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-c",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "section-a",
            kind_id: "motorLane",
            lanes: &[AuthoringLaneInput {
                authoring_lane_key: "lane-a",
                edge_chain: &[
                    LaneEdgeReference::local("edge-a"),
                    LaneEdgeReference::local("missing"),
                    LaneEdgeReference::local("edge-c"),
                ],
                lane_group: None,
            }],
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "corridor-a",
            reference_section: RoadSectionReference::local("section-a"),
            elements: &[CorridorElementReference::road_section(
                RoadSectionReference::local("section-a"),
            )],
        })
        .unwrap();
    let diagnostics = match Compiler::new().compile(unit([unknown_middle.finish().unwrap()])) {
        Ok(_) => panic!("unknown lane edge must reject compilation"),
        Err(diagnostics) => diagnostics,
    };
    assert!(
        diagnostics
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::UnknownReferenceTarget)
    );
    assert!(diagnostics.diagnostics().iter().all(|diagnostic| {
        diagnostic.code() != DiagnosticCode::DisconnectedAuthoringLaneEdgeChain
    }));

    let mut multiple_owner = make_builder();
    multiple_owner
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "section-a",
            kind_id: "motorLane",
            lanes: &[AuthoringLaneInput {
                authoring_lane_key: "lane-a",
                edge_chain: &[LaneEdgeReference::local("edge-a")],
                lane_group: None,
            }],
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "corridor-a",
            reference_section: RoadSectionReference::local("section-a"),
            elements: &[CorridorElementReference::road_section(
                RoadSectionReference::local("section-a"),
            )],
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "corridor-b",
            reference_section: RoadSectionReference::local("section-a"),
            elements: &[CorridorElementReference::road_section(
                RoadSectionReference::local("section-a"),
            )],
        })
        .unwrap();
    let diagnostics = match Compiler::new().compile(unit([multiple_owner.finish().unwrap()])) {
        Ok(_) => panic!("multiple cross-section owners must reject compilation"),
        Err(diagnostics) => diagnostics,
    };
    assert!(
        diagnostics
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code() == DiagnosticCode::MultipleCrossSectionOwners })
    );

    let mut group_parent_mismatch = make_builder();
    group_parent_mismatch
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-b",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_lane_group(LaneGroupInput {
            lane_group_key: "group-a",
            road_section: RoadSectionReference::local("section-a"),
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "section-a",
            kind_id: "motorLane",
            lanes: &[AuthoringLaneInput {
                authoring_lane_key: "lane-a",
                edge_chain: &[LaneEdgeReference::local("edge-a")],
                lane_group: None,
            }],
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "section-b",
            kind_id: "motorLane",
            lanes: &[AuthoringLaneInput {
                authoring_lane_key: "lane-b",
                edge_chain: &[LaneEdgeReference::local("edge-b")],
                lane_group: Some(LaneGroupReference::local("group-a")),
            }],
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "corridor-a",
            reference_section: RoadSectionReference::local("section-a"),
            elements: &[
                CorridorElementReference::road_section(RoadSectionReference::local("section-a")),
                CorridorElementReference::road_section(RoadSectionReference::local("section-b")),
            ],
        })
        .unwrap();
    let diagnostics = match Compiler::new().compile(unit([group_parent_mismatch.finish().unwrap()]))
    {
        Ok(_) => panic!("lane-group parent mismatch must reject compilation"),
        Err(diagnostics) => diagnostics,
    };
    assert!(
        diagnostics
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::LaneGroupParentMismatch)
    );
}

#[test]
fn source_changes_do_not_change_lir_semantic_digest() {
    let left = unit([module(
        "city/a",
        "left.document",
        &[],
        &[("edge-a", 10.0, &[])],
    )]);
    let right = unit([module(
        "city/a",
        "right.document",
        &[],
        &[("edge-a", 10.0, &[])],
    )]);
    let mut compiler = Compiler::new();
    let left = compiler.compile(left).unwrap();
    let right = compiler.compile(right).unwrap();

    assert_eq!(
        left.lir.inner.semantic_digest,
        right.lir.inner.semantic_digest
    );
    assert_ne!(
        left.source_map_input()
            .lane_edge_sources()
            .next()
            .unwrap()
            .primary_source()
            .source_document_key(),
        right
            .source_map_input()
            .lane_edge_sources()
            .next()
            .unwrap()
            .primary_source()
            .source_document_key()
    );
}

#[test]
fn thirty_two_failures_do_not_pollute_reused_compiler() {
    let missing = [LaneEdgeReference::local("missing")];
    let mut compiler = Compiler::new();
    for index in 0..32 {
        let failed = unit([module(
            &format!("failed/{index}"),
            &format!("failed-{index}.document"),
            &[],
            &[("edge-a", 10.0, &missing)],
        )]);
        let diagnostics = match compiler.compile(failed) {
            Ok(_) => panic!("expected failed compilation"),
            Err(diagnostics) => diagnostics,
        };
        assert!(matches!(
            diagnostics.diagnostics()[0].payload(),
            DiagnosticPayload::UnknownReferenceTarget { .. }
        ));
    }

    let recovered = unit([module(
        "city/a",
        "city-a.document",
        &[],
        &[("edge-a", 10.0, &[])],
    )]);
    let fresh = unit([module(
        "city/a",
        "city-a.document",
        &[],
        &[("edge-a", 10.0, &[])],
    )]);
    assert_eq!(
        compiler
            .compile(recovered)
            .unwrap()
            .lir
            .inner
            .semantic_digest,
        Compiler::new()
            .compile(fresh)
            .unwrap()
            .lir
            .inner
            .semantic_digest
    );
}

#[test]
fn source_map_output_limit_fails_after_lir_without_exposing_partial_output() {
    let probe = unit([module(
        "city/a",
        "city-a.document",
        &[],
        &[("edge-a", 10.0, &[])],
    )]);
    let hir = build_hir(&probe).unwrap();
    let mir = lower_to_mir(&probe, &hir).unwrap();
    let lir_output_bytes = freeze_lir(&probe, &mir).unwrap().lir.output_bytes;

    let mut constrained = unit([module(
        "city/a",
        "city-a.document",
        &[],
        &[("edge-a", 10.0, &[])],
    )]);
    constrained.limits = CompileLimits::p100_initial_v1().with_test_lir_limits(
        u32::MAX,
        u32::MAX,
        u32::try_from(lir_output_bytes).unwrap(),
        u32::MAX,
    );
    let mut compiler = Compiler::new();
    let diagnostics = match compiler.compile(constrained) {
        Ok(_) => panic!("expected source-map output limit failure"),
        Err(diagnostics) => diagnostics,
    };
    assert!(diagnostics.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.payload(),
        DiagnosticPayload::CompileLimitExceeded {
            dimension: CompileLimitDimension::OutputBytes,
            limit,
            observed,
        } if *limit == lir_output_bytes && observed > limit
    )));

    let recovered = unit([module(
        "city/recovered",
        "recovered.document",
        &[],
        &[("edge-a", 10.0, &[])],
    )]);
    assert!(compiler.compile(recovered).is_ok());
}

#[test]
fn compiler_freezes_gate_stop_line_and_waiting_zone_closure() {
    let mut builder = control_builder("control.document");
    add_valid_control(&mut builder, false);
    let output = Compiler::new()
        .compile(unit([builder.finish().unwrap()]))
        .unwrap();
    let lir = output.lir();

    assert_eq!(lir.stop_lines().len(), 2);
    assert_eq!(lir.maneuver_gates().len(), 2);
    assert_eq!(lir.waiting_zones().len(), 1);
    let path = lir.maneuver_paths().next().unwrap();
    let gates = path.maneuver_gates();
    assert_eq!(gates.len(), 2);
    assert_eq!(lir.maneuver_gate(gates[0]).unwrap().transition_index(), 0);
    assert_eq!(lir.maneuver_gate(gates[1]).unwrap().transition_index(), 1);
    let waiting = lir.waiting_zones().next().unwrap();
    assert_eq!(waiting.maneuver_path(), path.ordinal());
    assert_eq!(waiting.entry_gate(), gates[0]);
    assert_eq!(waiting.release_gate(), gates[1]);
    assert_eq!(waiting.max_occupancy(), 3);
    assert_eq!(path.waiting_zones(), &[waiting.ordinal()]);

    for gate in lir.maneuver_gates() {
        let stop_line = lir.stop_line(gate.stop_line()).unwrap();
        assert_eq!(stop_line.maneuver_gates(), &[gate.ordinal()]);
        assert_eq!(
            stop_line.lane_edge(),
            path.edges()[gate.transition_index() as usize]
        );
    }

    let source_map = output.source_map_input();
    assert_eq!(source_map.stop_line_sources().len(), 2);
    assert_eq!(source_map.maneuver_gate_sources().len(), 2);
    assert_eq!(source_map.waiting_zone_sources().len(), 1);
    let roles = source_map
        .junction_relation_sources()
        .map(|source| source.role())
        .collect::<Vec<_>>();
    assert_eq!(
        roles
            .iter()
            .filter(|role| **role == SourceRelationRole::ManeuverPathGate)
            .count(),
        2
    );
    assert_eq!(
        roles
            .iter()
            .filter(|role| **role == SourceRelationRole::ManeuverPathWaitingZone)
            .count(),
        1
    );
    assert_eq!(
        roles
            .iter()
            .filter(|role| **role == SourceRelationRole::StopLineManeuverGate)
            .count(),
        2
    );
}

#[test]
fn compiler_precompiles_static_route_control_occurrences_and_reverse_indexes() {
    let mut builder = control_builder("static-route.document");
    add_valid_control(&mut builder, false);
    builder
        .add_static_route(StaticRouteInput {
            static_route_key: "route-main",
            edge_sequence: &[
                LaneEdgeReference::local("entry"),
                LaneEdgeReference::local("middle"),
                LaneEdgeReference::local("exit"),
            ],
        })
        .unwrap();
    let output = Compiler::new()
        .compile(unit([builder.finish().unwrap()]))
        .unwrap();
    let lir = output.lir();
    let route = lir.static_routes().next().unwrap();
    let path = lir.maneuver_paths().next().unwrap();
    let path_gates = path.maneuver_gates();
    let waiting = lir.waiting_zones().next().unwrap();

    assert_eq!(route.edges(), path.edges());
    assert_eq!(
        route.transition_gates().collect::<Vec<_>>(),
        [Some(path_gates[0]), Some(path_gates[1])]
    );
    let maneuvers = route.maneuver_occurrences().collect::<Vec<_>>();
    assert_eq!(maneuvers.len(), 1);
    assert_eq!(maneuvers[0].maneuver_path(), path.ordinal());
    assert_eq!(maneuvers[0].entry_route_edge_index(), 0);
    assert_eq!(maneuvers[0].exit_route_edge_index(), 2);
    assert_eq!(maneuvers[0].gate_occurrence_range(), 0..2);
    assert_eq!(maneuvers[0].waiting_zone_occurrence_range(), 0..1);

    let gates = route.gate_occurrences().collect::<Vec<_>>();
    assert_eq!(gates.len(), 2);
    assert_eq!(gates[0].maneuver_gate(), path_gates[0]);
    assert_eq!(gates[0].next_gate_occurrence_index(), Some(1));
    assert_eq!(gates[0].next_boundary_route_edge_index(), 1);
    assert_eq!(gates[0].waiting_zone_occurrence_index(), Some(0));
    assert_eq!(gates[1].maneuver_gate(), path_gates[1]);
    assert_eq!(gates[1].next_gate_occurrence_index(), None);
    assert_eq!(gates[1].next_boundary_route_edge_index(), 2);

    let waiting_occurrences = route.waiting_zone_occurrences().collect::<Vec<_>>();
    assert_eq!(waiting_occurrences.len(), 1);
    assert_eq!(waiting_occurrences[0].waiting_zone(), waiting.ordinal());
    assert_eq!(waiting_occurrences[0].entry_gate_occurrence_index(), 0);
    assert_eq!(waiting_occurrences[0].release_gate_occurrence_index(), 1);
    assert_eq!(waiting_occurrences[0].entry_route_edge_index(), 0);
    assert_eq!(waiting_occurrences[0].release_route_edge_index(), 1);

    for (edge_index, edge) in route.edges().iter().copied().enumerate() {
        assert_eq!(
            lir.lane_edge(edge)
                .unwrap()
                .static_route_occurrences()
                .collect::<Vec<_>>(),
            [CanonicalStaticRouteOccurrenceRef {
                static_route: route.ordinal(),
                occurrence_index: edge_index as u32,
            }]
        );
    }
    assert_eq!(path.static_route_occurrences().len(), 1);
    assert_eq!(
        lir.maneuver_gate(path_gates[0])
            .unwrap()
            .static_route_occurrences()
            .len(),
        1
    );
    assert_eq!(waiting.static_route_occurrences().len(), 1);

    let source_map = output.source_map_input();
    assert_eq!(source_map.static_route_sources().len(), 1);
    let route_sources = source_map.route_relation_sources().collect::<Vec<_>>();
    assert_eq!(
        route_sources
            .iter()
            .map(|source| source.role())
            .collect::<Vec<_>>(),
        [
            SourceRelationRole::StaticRouteEdge,
            SourceRelationRole::StaticRouteEdge,
            SourceRelationRole::StaticRouteEdge,
            SourceRelationRole::StaticRouteManeuverOccurrence,
            SourceRelationRole::StaticRouteGateOccurrence,
            SourceRelationRole::StaticRouteGateOccurrence,
            SourceRelationRole::StaticRouteWaitingZoneOccurrence,
        ]
    );
    assert!(
        route_sources[..3]
            .iter()
            .all(|source| source.contributing_sources().len() == 0)
    );
    assert!(
        route_sources[3..]
            .iter()
            .all(|source| source.contributing_sources().len() == 1)
    );
}

#[test]
fn static_route_preserves_repeated_edge_occurrences() {
    let mut builder = junction_builder("static-route-repeated-edge.document");
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "loop",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[
                LaneEdgeReference::local("loop"),
                LaneEdgeReference::local("exit"),
            ],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_static_route(StaticRouteInput {
            static_route_key: "route-loop",
            edge_sequence: &[
                LaneEdgeReference::local("loop"),
                LaneEdgeReference::local("loop"),
                LaneEdgeReference::local("exit"),
            ],
        })
        .unwrap();

    let output = Compiler::new()
        .compile(unit([builder.finish().unwrap()]))
        .unwrap();
    let lir = output.lir();
    let route = lir.static_routes().next().unwrap();
    assert_eq!(route.edges()[0], route.edges()[1]);
    assert_ne!(route.edges()[1], route.edges()[2]);
    assert_eq!(
        lir.lane_edge(route.edges()[0])
            .unwrap()
            .static_route_occurrences()
            .collect::<Vec<_>>(),
        [
            CanonicalStaticRouteOccurrenceRef {
                static_route: route.ordinal(),
                occurrence_index: 0,
            },
            CanonicalStaticRouteOccurrenceRef {
                static_route: route.ordinal(),
                occurrence_index: 1,
            },
        ]
    );
    assert_eq!(
        lir.lane_edge(route.edges()[2])
            .unwrap()
            .static_route_occurrences()
            .collect::<Vec<_>>(),
        [CanonicalStaticRouteOccurrenceRef {
            static_route: route.ordinal(),
            occurrence_index: 2,
        }]
    );
}

#[test]
fn static_route_limit_failure_preserves_the_builder() {
    let mut builder = junction_builder("static-route-limit.document");
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "loop",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[LaneEdgeReference::local("loop")],
        })
        .unwrap();
    let over_limit = vec![
        LaneEdgeReference::local("loop");
        usize::try_from(
            CompileLimits::p100_initial_v1().value(CompileLimitDimension::RouteOccurrenceCount)
        )
        .unwrap()
            + 1
    ];
    let diagnostics = match builder.add_static_route(StaticRouteInput {
        static_route_key: "route-over-limit",
        edge_sequence: &over_limit,
    }) {
        Ok(_) => panic!("route occurrence limit must fail before owning the input"),
        Err(diagnostics) => diagnostics,
    };
    assert!(matches!(
        diagnostics.diagnostics()[0].payload(),
        DiagnosticPayload::CompileLimitExceeded {
            dimension: CompileLimitDimension::RouteOccurrenceCount,
            limit: 1_920,
            observed: 1_921,
        }
    ));

    builder
        .add_static_route(StaticRouteInput {
            static_route_key: "route-valid-after-failure",
            edge_sequence: &[LaneEdgeReference::local("loop")],
        })
        .unwrap();
    assert!(
        Compiler::new()
            .compile(unit([builder.finish().unwrap()]))
            .is_ok()
    );
}

#[test]
fn static_route_semantics_ignore_control_and_route_declaration_order() {
    let mut left = control_builder("static-route-left.document");
    add_valid_control(&mut left, false);
    left.add_static_route(StaticRouteInput {
        static_route_key: "route-main",
        edge_sequence: &[
            LaneEdgeReference::local("entry"),
            LaneEdgeReference::local("middle"),
            LaneEdgeReference::local("exit"),
        ],
    })
    .unwrap();

    let mut right = control_builder("static-route-right.document");
    right
        .add_static_route(StaticRouteInput {
            static_route_key: "route-main",
            edge_sequence: &[
                LaneEdgeReference::local("entry"),
                LaneEdgeReference::local("middle"),
                LaneEdgeReference::local("exit"),
            ],
        })
        .unwrap();
    add_valid_control(&mut right, true);

    let left = Compiler::new()
        .compile(unit([left.finish().unwrap()]))
        .unwrap();
    let right = Compiler::new()
        .compile(unit([right.finish().unwrap()]))
        .unwrap();
    assert_eq!(
        left.lir.inner.semantic_digest,
        right.lir.inner.semantic_digest
    );
    assert_eq!(
        left.lir()
            .static_routes()
            .map(|route| (route.stable_id(), route.edges().to_vec()))
            .collect::<Vec<_>>(),
        right
            .lir()
            .static_routes()
            .map(|route| (route.stable_id(), route.edges().to_vec()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn static_route_frontend_and_hir_reject_empty_disconnected_and_terminal_control() {
    let mut empty = junction_builder("static-route-empty.document");
    let diagnostics = match empty.add_static_route(StaticRouteInput {
        static_route_key: "route-empty",
        edge_sequence: &[],
    }) {
        Ok(_) => panic!("empty route must fail before mutation"),
        Err(diagnostics) => diagnostics,
    };
    assert_eq!(
        diagnostics.diagnostics()[0].code(),
        DiagnosticCode::EmptyStaticRoute
    );

    let mut disconnected = junction_builder("static-route-disconnected.document");
    disconnected
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "left",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "right",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_static_route(StaticRouteInput {
            static_route_key: "route-disconnected",
            edge_sequence: &[
                LaneEdgeReference::local("left"),
                LaneEdgeReference::local("right"),
            ],
        })
        .unwrap();
    assert!(
        compile_diagnostic_codes(disconnected)
            .contains(&DiagnosticCode::DisconnectedStaticRouteEdge)
    );

    let mut terminal = control_builder("static-route-terminal.document");
    add_valid_control(&mut terminal, false);
    terminal
        .add_static_route(StaticRouteInput {
            static_route_key: "route-terminal",
            edge_sequence: &[LaneEdgeReference::local("entry")],
        })
        .unwrap();
    assert!(
        compile_diagnostic_codes(terminal)
            .contains(&DiagnosticCode::StaticRouteTerminatesAtStopLine)
    );

    let mut boundaries = route_validation_builder("static-route-boundaries.document");
    boundaries
        .add_static_route(StaticRouteInput {
            static_route_key: "route-starts-inside",
            edge_sequence: &[
                LaneEdgeReference::local("middle"),
                LaneEdgeReference::local("exit"),
            ],
        })
        .unwrap()
        .add_static_route(StaticRouteInput {
            static_route_key: "route-ends-inside",
            edge_sequence: &[
                LaneEdgeReference::local("entry"),
                LaneEdgeReference::local("middle"),
            ],
        })
        .unwrap()
        .add_static_route(StaticRouteInput {
            static_route_key: "route-no-full-match",
            edge_sequence: &[
                LaneEdgeReference::local("entry"),
                LaneEdgeReference::local("middle"),
                LaneEdgeReference::local("detour"),
            ],
        })
        .unwrap()
        .add_static_route(StaticRouteInput {
            static_route_key: "route-uncovered-internal",
            edge_sequence: &[
                LaneEdgeReference::local("other"),
                LaneEdgeReference::local("middle"),
                LaneEdgeReference::local("exit"),
            ],
        })
        .unwrap();
    let codes = compile_diagnostic_codes(boundaries);
    assert!(codes.contains(&DiagnosticCode::StaticRouteStartsInsideJunction));
    assert!(codes.contains(&DiagnosticCode::StaticRouteEndsInsideJunction));
    assert!(codes.contains(&DiagnosticCode::StaticRouteManeuverNoFullMatch));
    assert!(codes.contains(&DiagnosticCode::StaticRouteInternalEdgeUncovered));
}

#[test]
fn control_semantics_are_invariant_to_declaration_permutation() {
    let mut left = control_builder("control-left.document");
    add_valid_control(&mut left, false);
    let mut right = control_builder("control-right.document");
    add_valid_control(&mut right, true);
    let left = Compiler::new()
        .compile(unit([left.finish().unwrap()]))
        .unwrap();
    let right = Compiler::new()
        .compile(unit([right.finish().unwrap()]))
        .unwrap();

    assert_eq!(
        left.lir.inner.semantic_digest,
        right.lir.inner.semantic_digest
    );
    assert_eq!(
        left.lir()
            .maneuver_gates()
            .map(|gate| (gate.stable_id(), gate.transition_index()))
            .collect::<Vec<_>>(),
        right
            .lir()
            .maneuver_gates()
            .map(|gate| (gate.stable_id(), gate.transition_index()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn control_closure_rejects_invalid_gate_and_stop_line_topology() {
    let mut out_of_range = control_builder("gate-range.document");
    out_of_range
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-entry",
            lane_edge: LaneEdgeReference::local("entry"),
        })
        .unwrap()
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-invalid",
            maneuver_path: ManeuverPathReference::local("path-main"),
            transition_index: 2,
            stop_line: StopLineReference::local("stop-entry"),
            signal_control: SignalControlInput::None,
        })
        .unwrap();
    assert!(
        compile_diagnostic_codes(out_of_range)
            .contains(&DiagnosticCode::ManeuverGateTransitionOutOfRange)
    );

    let mut duplicate = control_builder("gate-duplicate.document");
    duplicate
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-entry",
            lane_edge: LaneEdgeReference::local("entry"),
        })
        .unwrap();
    for key in ["gate-a", "gate-b"] {
        duplicate
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: key,
                maneuver_path: ManeuverPathReference::local("path-main"),
                transition_index: 0,
                stop_line: StopLineReference::local("stop-entry"),
                signal_control: SignalControlInput::None,
            })
            .unwrap();
    }
    assert!(
        compile_diagnostic_codes(duplicate)
            .contains(&DiagnosticCode::DuplicateManeuverGatePathTransition)
    );

    let mut mismatch = control_builder("gate-mismatch.document");
    mismatch
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-middle",
            lane_edge: LaneEdgeReference::local("middle"),
        })
        .unwrap()
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-entry",
            maneuver_path: ManeuverPathReference::local("path-main"),
            transition_index: 0,
            stop_line: StopLineReference::local("stop-middle"),
            signal_control: SignalControlInput::None,
        })
        .unwrap();
    assert!(
        compile_diagnostic_codes(mismatch).contains(&DiagnosticCode::ManeuverGateStopLineMismatch)
    );

    let mut orphan = control_builder("stop-orphan.document");
    orphan
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-exit",
            lane_edge: LaneEdgeReference::local("exit"),
        })
        .unwrap();
    assert!(compile_diagnostic_codes(orphan).contains(&DiagnosticCode::OrphanStopLine));

    let mut unreferenced = control_builder("stop-unreferenced.document");
    unreferenced
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-entry",
            lane_edge: LaneEdgeReference::local("entry"),
        })
        .unwrap();
    assert!(compile_diagnostic_codes(unreferenced).contains(&DiagnosticCode::UnreferencedStopLine));

    let mut duplicate_stop_line = control_builder("stop-duplicate-edge.document");
    for key in ["stop-entry-a", "stop-entry-b"] {
        duplicate_stop_line
            .add_stop_line(StopLineInput {
                stop_line_key: key,
                lane_edge: LaneEdgeReference::local("entry"),
            })
            .unwrap();
    }
    assert!(
        compile_diagnostic_codes(duplicate_stop_line)
            .contains(&DiagnosticCode::DuplicateStopLineEdge)
    );

    let mut missing_gate = branched_control_builder("stop-missing-gate.document", true);
    missing_gate
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-entry",
            lane_edge: LaneEdgeReference::local("entry"),
        })
        .unwrap()
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-left",
            maneuver_path: ManeuverPathReference::local("path-left"),
            transition_index: 0,
            stop_line: StopLineReference::local("stop-entry"),
            signal_control: SignalControlInput::None,
        })
        .unwrap();
    assert!(
        compile_diagnostic_codes(missing_gate)
            .contains(&DiagnosticCode::MissingManeuverGateCoverage)
    );

    let mut missing_path = branched_control_builder("stop-missing-path.document", false);
    missing_path
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-entry",
            lane_edge: LaneEdgeReference::local("entry"),
        })
        .unwrap()
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-left",
            maneuver_path: ManeuverPathReference::local("path-left"),
            transition_index: 0,
            stop_line: StopLineReference::local("stop-entry"),
            signal_control: SignalControlInput::None,
        })
        .unwrap();
    assert!(
        compile_diagnostic_codes(missing_path)
            .contains(&DiagnosticCode::MissingManeuverPathCoverage)
    );
}

#[test]
fn synthetic_maneuver_path_requires_successors_for_internal_sequence() {
    let mut builder = junction_builder("internal-route.document");
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "internal",
            length_meters: 8.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[],
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
            internal_edges: &[LaneEdgeReference::local("internal")],
            exit_edge: LaneEdgeReference::local("exit"),
        })
        .unwrap()
        .add_static_route(StaticRouteInput {
            static_route_key: "route-main",
            edge_sequence: &[
                LaneEdgeReference::local("entry"),
                LaneEdgeReference::local("internal"),
                LaneEdgeReference::local("exit"),
            ],
        })
        .unwrap();

    let diagnostics = match Compiler::new().compile(unit([builder.finish().unwrap()])) {
        Ok(_) => panic!("Synthetic maneuver paths require explicit successor connectivity"),
        Err(diagnostics) => diagnostics,
    };
    assert!(
        diagnostics
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code() == DiagnosticCode::DisconnectedManeuverPath })
    );
}

#[test]
fn path_owned_internal_transition_accepts_release_stop_without_explicit_successor() {
    let mut builder = junction_builder("internal-release-stop.document");
    let entry_chain = [LaneEdgeReference::local("entry")];
    let exit_chain = [LaneEdgeReference::local("exit")];
    let approach_lanes = [
        AuthoringLaneInput {
            authoring_lane_key: "lane-entry",
            edge_chain: &entry_chain,
            lane_group: None,
        },
        AuthoringLaneInput {
            authoring_lane_key: "lane-exit",
            edge_chain: &exit_chain,
            lane_group: None,
        },
    ];
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[LaneEdgeReference::local("exit")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "middle",
            length_meters: 8.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit",
            length_meters: 12.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "section-main",
            kind_id: "motorLane",
            lanes: &approach_lanes,
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "corridor-main",
            reference_section: RoadSectionReference::local("section-main"),
            elements: &[CorridorElementReference::road_section(
                RoadSectionReference::local("section-main"),
            )],
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
            stop_line_key: "stop-middle",
            lane_edge: LaneEdgeReference::local("middle"),
        })
        .unwrap()
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-release",
            maneuver_path: ManeuverPathReference::local("path-main"),
            transition_index: 1,
            stop_line: StopLineReference::local("stop-middle"),
            signal_control: SignalControlInput::None,
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
        .unwrap();

    let mut input = unit([builder.finish().unwrap()]);
    let junction = input.modules[0]
        .declarations
        .iter_mut()
        .find_map(|declaration| match declaration {
            TypedAstDeclaration::Junction(junction) => Some(junction),
            _ => None,
        })
        .unwrap();
    let namespace = Arc::<str>::from("city/junction");
    let document = Arc::<str>::from("internal-release-stop.document");
    let location = |column| SourceSpan::point(Arc::clone(&document), 1, column);
    junction.approach_edges = Box::new([
        OwnedEntityReference::new(Arc::clone(&namespace), Arc::from("entry"), location(1)),
        OwnedEntityReference::new(Arc::clone(&namespace), Arc::from("exit"), location(2)),
    ]);
    junction.internal_edges = Box::new([OwnedEntityReference::new(
        Arc::clone(&namespace),
        Arc::from("middle"),
        location(3),
    )]);

    let output = Compiler::new().compile(input).unwrap();
    assert_eq!(output.lir().maneuver_gates().count(), 1);
    assert_eq!(output.lir().static_routes().count(), 1);
    let route = output.lir().static_routes().next().unwrap();
    let maneuvers = route.maneuver_occurrences().collect::<Vec<_>>();
    let gates = route.gate_occurrences().collect::<Vec<_>>();
    assert_eq!(maneuvers.len(), 1);
    assert_eq!(maneuvers[0].entry_route_edge_index(), 0);
    assert_eq!(maneuvers[0].exit_route_edge_index(), 2);
    assert_eq!(maneuvers[0].gate_occurrence_range(), 0..1);
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].maneuver_occurrence_index(), 0);
    assert_eq!(gates[0].from_route_edge_index(), 1);
    assert_eq!(gates[0].next_boundary_route_edge_index(), 2);
    assert_eq!(
        route.transition_gates().collect::<Vec<_>>(),
        [None, Some(gates[0].maneuver_gate())]
    );
}

#[test]
fn waiting_zone_validation_rejects_zero_reverse_and_overlap() {
    let mut zero = control_builder("waiting-zero.document");
    let diagnostics = match zero.add_waiting_zone(WaitingZoneInput {
        waiting_zone_key: "waiting-zero",
        maneuver_path: ManeuverPathReference::local("path-main"),
        entry_gate: ManeuverGateReference::local("gate-entry"),
        release_gate: ManeuverGateReference::local("gate-release"),
        max_occupancy: 0,
    }) {
        Ok(_) => panic!("zero waiting-zone capacity must fail"),
        Err(diagnostics) => diagnostics,
    };
    assert_eq!(
        diagnostics.diagnostics()[0].code(),
        DiagnosticCode::InvalidWaitingZoneCapacity
    );

    let mut reverse = control_builder("waiting-reverse.document");
    add_valid_control(&mut reverse, false);
    reverse
        .add_waiting_zone(WaitingZoneInput {
            waiting_zone_key: "waiting-reverse",
            maneuver_path: ManeuverPathReference::local("path-main"),
            entry_gate: ManeuverGateReference::local("gate-release"),
            release_gate: ManeuverGateReference::local("gate-entry"),
            max_occupancy: 1,
        })
        .unwrap();
    assert!(
        compile_diagnostic_codes(reverse).contains(&DiagnosticCode::InvalidWaitingZoneGateOrder)
    );

    let mut overlap = control_builder("waiting-overlap.document");
    add_valid_control(&mut overlap, false);
    overlap
        .add_waiting_zone(WaitingZoneInput {
            waiting_zone_key: "waiting-overlap",
            maneuver_path: ManeuverPathReference::local("path-main"),
            entry_gate: ManeuverGateReference::local("gate-entry"),
            release_gate: ManeuverGateReference::local("gate-release"),
            max_occupancy: 1,
        })
        .unwrap();
    assert!(compile_diagnostic_codes(overlap).contains(&DiagnosticCode::OverlappingWaitingZones));
}

#[test]
fn parking_static_contract_freezes_area_standalone_space_and_source_roles() {
    let output = Compiler::new()
        .compile(unit([parking_module(
            "parking.document",
            "area-main",
            false,
        )]))
        .unwrap();
    let area = output.lir().parking_areas().next().unwrap();
    let spaces = output.lir().parking_spaces().collect::<Vec<_>>();
    assert_eq!(spaces.len(), 2);
    let owned = spaces
        .iter()
        .copied()
        .find(|space| {
            stable_key(space.identity_fields(), FieldTag::ParkingSpaceKey) == "space-owned"
        })
        .unwrap();
    let independent = spaces
        .iter()
        .copied()
        .find(|space| {
            stable_key(space.identity_fields(), FieldTag::ParkingSpaceKey) == "space-independent"
        })
        .unwrap();

    assert_eq!(area.parking_spaces(), [owned.ordinal()]);
    assert_eq!(owned.parking_area(), Some(area.ordinal()));
    assert_eq!(independent.parking_area(), None);
    assert_eq!(owned.entry().progress_meters(), 4.0);
    assert_eq!(owned.exit().progress_meters(), 6.0);
    assert_ne!(owned.entry().lane_edge(), owned.exit().lane_edge());
    assert_eq!(owned.geometry().lateral_offset_meters(), -3.0);
    assert_eq!(owned.geometry().heading_offset_radians(), 0.25);
    assert_eq!(owned.geometry().length_meters(), 5.5);
    assert_eq!(owned.geometry().width_meters(), 2.6);

    assert_eq!(output.source_map_input().parking_area_sources().len(), 1);
    assert_eq!(output.source_map_input().parking_space_sources().len(), 2);
    let roles = output
        .source_map_input()
        .parking_relation_sources()
        .map(|source| source.role())
        .collect::<Vec<_>>();
    assert_eq!(
        roles,
        [
            SourceRelationRole::ParkingSpaceArea,
            SourceRelationRole::ParkingSpaceEntry,
            SourceRelationRole::ParkingSpaceExit,
            SourceRelationRole::ParkingSpaceEntry,
            SourceRelationRole::ParkingSpaceExit,
        ]
    );
}

#[test]
fn parking_identity_and_digest_obey_set_and_organizational_semantics() {
    let first = Compiler::new()
        .compile(unit([parking_module(
            "parking-a.document",
            "area-a",
            false,
        )]))
        .unwrap();
    let permuted = Compiler::new()
        .compile(unit([parking_module("parking-b.document", "area-a", true)]))
        .unwrap();
    assert_eq!(
        first.lir.inner.semantic_digest,
        permuted.lir.inner.semantic_digest
    );

    let reassigned = Compiler::new()
        .compile(unit([parking_module(
            "parking-c.document",
            "area-b",
            false,
        )]))
        .unwrap();
    let owned_id = |output: &CompilationOutput| {
        output
            .lir()
            .parking_spaces()
            .find(|space| {
                stable_key(space.identity_fields(), FieldTag::ParkingSpaceKey) == "space-owned"
            })
            .unwrap()
            .stable_id()
    };
    assert_eq!(owned_id(&first), owned_id(&reassigned));
    assert_ne!(
        first.lir().parking_areas().next().unwrap().stable_id(),
        reassigned.lir().parking_areas().next().unwrap().stable_id()
    );
}

#[test]
fn parking_validation_rejects_orphan_anchor_and_geometry_failures() {
    let mut orphan = parking_builder("parking-orphan.document");
    orphan
        .add_parking_area(ParkingAreaInput {
            parking_area_key: "area-orphan",
        })
        .unwrap();
    assert_eq!(
        compile_diagnostic_codes(orphan),
        [DiagnosticCode::OrphanParkingArea]
    );

    let mut invalid = parking_builder("parking-invalid.document");
    add_parking_edges(&mut invalid);
    invalid
        .add_parking_area(ParkingAreaInput {
            parking_area_key: "area-main",
        })
        .unwrap()
        .add_parking_space(ParkingSpaceInput {
            parking_space_key: "space-invalid",
            parking_area: Some(ParkingAreaReference::local("area-main")),
            entry: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("parking-entry"),
                progress_meters: 0.0,
            },
            exit: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("parking-exit"),
                progress_meters: 20.0,
            },
            geometry: ParkingSpaceGeometryInput {
                lateral_offset_meters: 0.0,
                heading_offset_radians: core::f64::consts::PI,
                length_meters: 0.0,
                width_meters: f64::INFINITY,
            },
        })
        .unwrap();
    let codes = compile_diagnostic_codes(invalid);
    assert_eq!(
        codes
            .iter()
            .filter(|code| **code == DiagnosticCode::InvalidParkingAnchorProgress)
            .count(),
        2
    );
    assert_eq!(
        codes
            .iter()
            .filter(|code| **code == DiagnosticCode::InvalidParkingSpaceGeometry)
            .count(),
        3
    );
    assert!(!codes.contains(&DiagnosticCode::OrphanParkingArea));
}

#[test]
fn compiler_freezes_vehicle_profile_values_identity_and_class_source() {
    let mut builder = access_builder("vehicle-profile.document");
    builder
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "passenger-car",
            extends: None,
        })
        .unwrap()
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "standard-car",
            participant_class: ParticipantClassReference::local("passenger-car"),
            iidm: canonical_iidm_profile(),
        })
        .unwrap();

    let output = Compiler::new()
        .compile(unit([builder.finish().unwrap()]))
        .unwrap();
    let profile = output.lir().vehicle_profiles().next().unwrap();
    assert_eq!(
        stable_key(profile.identity_fields(), FieldTag::VehicleProfileKey),
        "standard-car"
    );
    assert_eq!(profile.length_meters(), 4.5);
    assert_eq!(profile.desired_speed_meters_per_second(), 13.75);
    assert_eq!(profile.min_gap_meters(), 2.0);
    assert_eq!(profile.time_headway_seconds(), 1.4);
    assert_eq!(profile.max_acceleration_meters_per_second_squared(), 1.8);
    assert_eq!(
        profile.comfortable_deceleration_meters_per_second_squared(),
        2.0
    );
    assert_eq!(
        profile.emergency_deceleration_meters_per_second_squared(),
        4.5
    );
    assert_eq!(
        output
            .lir()
            .participant_class(profile.participant_class())
            .unwrap()
            .ordinal(),
        profile.participant_class()
    );
    assert_eq!(output.source_map_input().vehicle_profile_sources().len(), 1);
    let relation = output
        .source_map_input()
        .access_relation_sources()
        .find(|source| source.role() == SourceRelationRole::VehicleProfileParticipantClass)
        .unwrap();
    assert!(matches!(
        relation.owner(),
        crate::AccessRelationOwner::VehicleProfile(ordinal, stable_id)
            if ordinal == profile.ordinal() && stable_id == profile.stable_id()
    ));
}

#[test]
fn compiler_freezes_canonical_frames_in_identity_order_with_sources() {
    let mut builder = access_builder("canonical-frame.document");
    builder
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame-z",
            lane_edge_geometries: &[],
        })
        .unwrap()
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame-a",
            lane_edge_geometries: &[],
        })
        .unwrap();

    let output = Compiler::new()
        .compile(unit([builder.finish().unwrap()]))
        .unwrap();
    let frames = output.lir().canonical_frames().collect::<Vec<_>>();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].ordinal().raw(), 0);
    assert_eq!(frames[1].ordinal().raw(), 1);
    assert_eq!(
        stable_key(frames[0].identity_fields(), FieldTag::CanonicalFrameKey),
        "frame-a"
    );
    assert_eq!(
        stable_key(frames[1].identity_fields(), FieldTag::CanonicalFrameKey),
        "frame-z"
    );
    let sources = output
        .source_map_input()
        .canonical_frame_sources()
        .collect::<Vec<_>>();
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0].ordinal(), frames[0].ordinal());
    assert_eq!(sources[0].stable_id(), frames[0].stable_id());
    assert_eq!(sources[1].ordinal(), frames[1].ordinal());
    assert_eq!(sources[1].stable_id(), frames[1].stable_id());
}

#[test]
fn canonical_frame_identity_changes_lir_semantic_digest() {
    let compile = |key| {
        let mut builder = access_builder("canonical-frame-digest.document");
        builder
            .add_canonical_frame(CanonicalFrameInput {
                canonical_frame_key: key,
                lane_edge_geometries: &[],
            })
            .unwrap();
        Compiler::new()
            .compile(unit([builder.finish().unwrap()]))
            .unwrap()
    };

    let left = compile("frame-a");
    let right = compile("frame-b");
    assert_ne!(
        left.lir.inner.semantic_digest,
        right.lir.inner.semantic_digest
    );
}

#[test]
fn compiler_validates_and_freezes_lane_edge_spatial_sampling_tables() {
    let points = [
        CanonicalPoint3F32Input {
            x: -0.0,
            y: 0.0,
            z: 0.0,
        },
        CanonicalPoint3F32Input {
            x: 8.0,
            y: 0.0,
            z: 0.0,
        },
        CanonicalPoint3F32Input {
            x: 20.0,
            y: 0.0,
            z: 0.0,
        },
    ];
    let geometries = [LaneEdgeGeometryInput {
        lane_edge: LaneEdgeReference::local("edge-main"),
        centerline_points: &points,
    }];
    let mut builder = access_builder("canonical-spatial.document");
    builder
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame-main",
            lane_edge_geometries: &geometries,
        })
        .unwrap();

    let output = Compiler::new()
        .compile(unit([builder.finish().unwrap()]))
        .unwrap();
    let edge = output.lir().lane_edges().next().unwrap();
    let geometry = edge.spatial_geometry().unwrap();
    assert_eq!(geometry.lane_edge(), edge.ordinal());
    assert_eq!(geometry.canonical_frame().raw(), 0);
    assert_eq!(geometry.arc_length_meters(), 20.0);
    let frozen_points = geometry.points().collect::<Vec<_>>();
    assert_eq!(frozen_points.len(), 3);
    assert_eq!(frozen_points[0].x.to_bits(), 0.0_f32.to_bits());
    let segments = geometry.segments().collect::<Vec<_>>();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].length_meters, 8.0);
    assert_eq!(segments[1].cumulative_end_meters, 20.0);
    assert_eq!(segments[0].tangent, [1.0, 0.0, 0.0]);
    assert_eq!(segments[0].up, [0.0, 1.0, 0.0]);

    let relation = output
        .source_map_input()
        .spatial_relation_sources()
        .next()
        .unwrap();
    assert_eq!(relation.owner_ordinal(), geometry.canonical_frame());
    assert_eq!(
        relation.role(),
        SourceRelationRole::CanonicalFrameLaneEdgeGeometry
    );
    assert_eq!(relation.local_index(), 0);
}

#[test]
fn compiler_freezes_non_traversable_facility_geometry_in_canonical_band_order() {
    let mut baseline_unit = spatial_cross_section_unit(false, 2.0, true);
    let baseline_module = &mut baseline_unit.modules[0];
    let mut band_a_index = None;
    let mut band_z_index = None;
    for (index, declaration) in baseline_module.declarations.iter_mut().enumerate() {
        let TypedAstDeclaration::FacilityBand(band) = declaration else {
            continue;
        };
        let (line, target) = if band.header.stable_key.as_ref() == "band-a" {
            (10, &mut band_a_index)
        } else {
            (20, &mut band_z_index)
        };
        band.header.span =
            SourceSpan::point(Arc::from("spatial-cross-section.document"), line, 1).into();
        *target = Some(index);
    }
    let band_a_index = band_a_index.expect("fixture contains band-a");
    let band_z_index = band_z_index.expect("fixture contains band-z");
    if band_a_index < band_z_index {
        baseline_module
            .declarations
            .swap(band_a_index, band_z_index);
    }
    let baseline = Compiler::new().compile(baseline_unit).unwrap();
    let permuted = Compiler::new()
        .compile(spatial_cross_section_unit(true, 2.0, true))
        .unwrap();
    let changed = Compiler::new()
        .compile(spatial_cross_section_unit(false, 3.0, true))
        .unwrap();
    let headless_facilities = Compiler::new()
        .compile(spatial_cross_section_unit(false, 2.0, false))
        .unwrap();
    let mut sparse_unit = spatial_cross_section_unit(false, 2.0, true);
    let sparse_module = &mut sparse_unit.modules[0];
    let sparse_band_a = sparse_module
        .declarations
        .iter_mut()
        .find_map(|declaration| match declaration {
            TypedAstDeclaration::FacilityBand(band)
                if band.header.stable_key.as_ref() == "band-a" =>
            {
                Some(band)
            }
            _ => None,
        })
        .expect("fixture contains band-a");
    sparse_band_a.compiled_geometry = None;
    let sparse = Compiler::new().compile(sparse_unit).unwrap();

    let bands = baseline.lir().facility_bands().collect::<Vec<_>>();
    assert_eq!(bands.len(), 2);
    let keys = bands
        .iter()
        .map(|band| {
            band.identity_fields()
                .find(|field| field.tag() == FieldTag::FacilityBandKey)
                .map(|field| String::from_utf8(field.value_bytes().to_vec()).unwrap())
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(keys, ["band-a", "band-z"]);

    let geometry = bands[0].spatial_geometry().unwrap();
    assert_eq!(geometry.facility_band(), bands[0].ordinal());
    assert_eq!(geometry.canonical_frame().raw(), 0);
    assert_eq!(
        geometry.points().collect::<Vec<_>>(),
        [
            CanonicalPoint3F32 {
                x: 0.0,
                y: 0.0,
                z: 2.0,
            },
            CanonicalPoint3F32 {
                x: 10.0,
                y: 0.0,
                z: 2.0,
            },
        ]
    );
    assert_eq!(baseline.lir.inner.lane_edge_geometries.len(), 1);
    assert_eq!(baseline.lir.inner.facility_band_geometries.len(), 2);
    assert_eq!(
        baseline
            .lir
            .inner
            .facility_band_geometries
            .iter()
            .map(|geometry| geometry.facility_band.raw())
            .collect::<Vec<_>>(),
        [0, 1]
    );
    let sparse_bands = sparse.lir().facility_bands().collect::<Vec<_>>();
    assert!(sparse_bands[0].spatial_geometry().is_none());
    assert_eq!(
        sparse_bands[1]
            .spatial_geometry()
            .expect("ordinal one keeps its sparse geometry")
            .facility_band(),
        sparse_bands[1].ordinal()
    );
    assert_eq!(
        sparse
            .lir
            .inner
            .facility_band_geometries
            .iter()
            .map(|geometry| geometry.facility_band.raw())
            .collect::<Vec<_>>(),
        [1]
    );
    assert_eq!(baseline.lir.inner.canonical_points.len(), 6);
    assert_eq!(baseline.lir.inner.spatial_segments.len(), 1);
    assert_eq!(
        baseline.lir.inner.spatial_segments.len(),
        headless_facilities.lir.inner.spatial_segments.len()
    );
    let baseline_edge = baseline
        .lir()
        .lane_edges()
        .next()
        .expect("fixture retains its lane edge");
    let baseline_lane = baseline_edge
        .spatial_geometry()
        .expect("fixture retains lane geometry");
    let changed_edge = changed
        .lir()
        .lane_edges()
        .next()
        .expect("changed fixture retains its lane edge");
    let changed_lane = changed_edge
        .spatial_geometry()
        .expect("changed fixture retains lane geometry");
    assert_eq!(
        baseline_lane.points().collect::<Vec<_>>(),
        changed_lane.points().collect::<Vec<_>>()
    );
    assert_eq!(
        baseline_lane.segments().collect::<Vec<_>>(),
        changed_lane.segments().collect::<Vec<_>>()
    );

    let spatial_relations = baseline
        .source_map_input()
        .spatial_relation_sources()
        .filter(|relation| {
            relation.role() == SourceRelationRole::CanonicalFrameFacilityBandGeometry
        })
        .map(|relation| {
            (
                relation.role(),
                relation.local_index(),
                relation
                    .primary_source()
                    .text_range()
                    .map(|(start, _)| start.line()),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        spatial_relations,
        [
            (
                SourceRelationRole::CanonicalFrameFacilityBandGeometry,
                0,
                Some(10),
            ),
            (
                SourceRelationRole::CanonicalFrameFacilityBandGeometry,
                1,
                Some(20),
            ),
        ]
    );

    assert_eq!(
        baseline.metrics().semantic_fingerprint(),
        permuted.metrics().semantic_fingerprint()
    );
    assert_ne!(
        baseline.metrics().semantic_fingerprint(),
        changed.metrics().semantic_fingerprint()
    );
    assert_eq!(
        baseline.metrics().lir_record_count(),
        headless_facilities
            .metrics()
            .lir_record_count()
            .saturating_add(6)
    );
    // 两条 point-only facility geometry 行和四个点的目标布局逻辑量。
    assert_eq!(
        baseline.metrics().output_logical_bytes(),
        headless_facilities
            .metrics()
            .output_logical_bytes()
            .saturating_add(80)
    );
    assert!(
        headless_facilities
            .lir()
            .facility_bands()
            .all(|band| band.spatial_geometry().is_none())
    );
}

#[test]
fn imported_facility_frame_keeps_the_band_module_as_its_relation_source() {
    let output = Compiler::new()
        .compile(spatial_cross_section_unit_with_frame(
            false, 2.0, true, true,
        ))
        .unwrap();
    let bands = output.lir().facility_bands().collect::<Vec<_>>();
    assert_eq!(
        output
            .lir
            .inner
            .facility_band_geometries
            .iter()
            .map(|geometry| geometry.facility_band.raw())
            .collect::<Vec<_>>(),
        [0, 1]
    );
    let geometry = bands[0]
        .spatial_geometry()
        .expect("fixture contains compiled FacilityBand geometry");
    let sources = output
        .source_map_input()
        .spatial_relation_sources()
        .filter(|source| source.role() == SourceRelationRole::CanonicalFrameFacilityBandGeometry)
        .collect::<Vec<_>>();

    assert_eq!(sources.len(), 2);
    assert!(sources.iter().all(|source| {
        source.owner_ordinal() == geometry.canonical_frame()
            && source.primary_source().source_document_key() == "spatial-cross-section.document"
    }));
}

#[test]
fn invalid_facility_geometry_fails_without_exposing_partial_lir() {
    let diagnostics = Compiler::new()
        .compile(spatial_cross_section_unit(false, f32::NAN, true))
        .err()
        .expect("non-finite FacilityBand geometry must fail");
    assert!(diagnostics.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == DiagnosticCode::InvalidFacilityBandGeometry
            && matches!(
                diagnostic.payload(),
                DiagnosticPayload::InvalidFacilityBandGeometry {
                    violation: crate::SpatialGeometryViolation::NonFiniteCoordinate { .. },
                    ..
                }
            )
    }));
}

#[test]
fn facility_geometry_without_any_canonical_frame_reports_the_reference_failure() {
    let mut input = unit([cross_section_module(false)]);
    let module = &mut input.modules[0];
    let namespace: Arc<str> = module.descriptor().authoring_namespace_id().into();
    let band = module
        .declarations
        .iter_mut()
        .find_map(|declaration| match declaration {
            TypedAstDeclaration::FacilityBand(band) => Some(band),
            _ => None,
        })
        .unwrap();
    band.compiled_geometry = Some(CompiledFacilityBandGeometry {
        length: EdgeLength::try_new(10.0).unwrap(),
        canonical_frame: OwnedEntityReference::<CanonicalFrameKind>::new(
            namespace,
            Arc::from("missing-frame"),
            band.header.span.clone(),
        ),
        centerline_points: [
            CanonicalPoint3F32Input {
                x: 0.0,
                y: 0.0,
                z: 2.0,
            },
            CanonicalPoint3F32Input {
                x: 10.0,
                y: 0.0,
                z: 2.0,
            },
        ]
        .into(),
        source_ranges: Box::new([]),
    });

    let diagnostics = Compiler::new()
        .compile(input)
        .err()
        .expect("compiled FacilityBand without a resolvable frame must fail");
    assert_eq!(
        diagnostics
            .diagnostics()
            .iter()
            .map(Diagnostic::code)
            .collect::<Vec<_>>(),
        [DiagnosticCode::UnknownReferenceTarget]
    );
}

#[test]
fn spatial_geometry_rejects_length_mismatch_without_partial_output() {
    let points = [
        CanonicalPoint3F32Input {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        CanonicalPoint3F32Input {
            x: 19.0,
            y: 0.0,
            z: 0.0,
        },
    ];
    let geometries = [LaneEdgeGeometryInput {
        lane_edge: LaneEdgeReference::local("edge-main"),
        centerline_points: &points,
    }];
    let mut builder = access_builder("canonical-spatial-length.document");
    builder
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame-main",
            lane_edge_geometries: &geometries,
        })
        .unwrap();

    let diagnostics = Compiler::new()
        .compile(unit([builder.finish().unwrap()]))
        .err()
        .expect("mismatched geometry must fail");
    assert_eq!(diagnostics.diagnostics().len(), 1);
    assert_eq!(
        diagnostics.diagnostics()[0].code(),
        DiagnosticCode::InvalidSpatialGeometry
    );
    assert!(matches!(
        diagnostics.diagnostics()[0].payload(),
        DiagnosticPayload::InvalidSpatialGeometry {
            violation: crate::SpatialGeometryViolation::LengthMismatch { .. },
            ..
        }
    ));
}

#[test]
fn spatial_geometry_set_order_does_not_change_lir_semantics() {
    let compile = |reverse: bool| {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: "city/spatial-order",
                source_document_key: if reverse {
                    "spatial-order-reverse.document"
                } else {
                    "spatial-order.document"
                },
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: Some(42),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .unwrap();
        let mut builder = SyntheticModuleBuilder::new(header, &limits).unwrap();
        for key in ["edge-a", "edge-b"] {
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: 10.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[],
                })
                .unwrap();
        }
        let points_a = [
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
        let points_b = [
            CanonicalPoint3F32Input {
                x: 20.0,
                y: 0.0,
                z: 0.0,
            },
            CanonicalPoint3F32Input {
                x: 30.0,
                y: 0.0,
                z: 0.0,
            },
        ];
        let ordered = [
            LaneEdgeGeometryInput {
                lane_edge: LaneEdgeReference::local("edge-a"),
                centerline_points: &points_a,
            },
            LaneEdgeGeometryInput {
                lane_edge: LaneEdgeReference::local("edge-b"),
                centerline_points: &points_b,
            },
        ];
        let reversed = [ordered[1], ordered[0]];
        builder
            .add_canonical_frame(CanonicalFrameInput {
                canonical_frame_key: "frame-main",
                lane_edge_geometries: if reverse { &reversed } else { &ordered },
            })
            .unwrap();
        Compiler::new()
            .compile(unit([builder.finish().unwrap()]))
            .unwrap()
    };

    assert_eq!(
        compile(false).lir.inner.semantic_digest,
        compile(true).lir.inner.semantic_digest
    );
}

#[test]
fn spatial_geometry_requires_complete_coverage_once_enabled() {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "city/spatial-coverage",
            source_document_key: "spatial-coverage.document",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: Some(42),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .unwrap();
    let mut builder = SyntheticModuleBuilder::new(header, &limits).unwrap();
    for key in ["edge-a", "edge-b"] {
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: key,
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap();
    }
    let points = [
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
        lane_edge: LaneEdgeReference::local("edge-a"),
        centerline_points: &points,
    }];
    builder
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame-main",
            lane_edge_geometries: &geometries,
        })
        .unwrap();

    let diagnostics = Compiler::new()
        .compile(unit([builder.finish().unwrap()]))
        .err()
        .expect("partial coverage must fail");
    assert!(matches!(
        diagnostics.diagnostics()[0].payload(),
        DiagnosticPayload::InvalidSpatialGeometry {
            lane_edge_key,
            violation: crate::SpatialGeometryViolation::MissingEdgeBinding,
            ..
        } if lane_edge_key.as_ref() == "edge-b"
    ));
}

#[test]
fn vehicle_profile_frontend_rejects_invalid_scalars_and_deceleration_order() {
    let mut invalid_scalar = access_builder("vehicle-profile-invalid-scalar.document");
    invalid_scalar
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "car",
            extends: None,
        })
        .unwrap();
    let mut iidm = canonical_iidm_profile();
    iidm.min_gap_meters = -0.1;
    let diagnostics = match invalid_scalar.add_vehicle_profile(VehicleProfileInput {
        vehicle_profile_key: "invalid-gap",
        participant_class: ParticipantClassReference::local("car"),
        iidm,
    }) {
        Ok(_) => panic!("negative minGap must fail"),
        Err(diagnostics) => diagnostics,
    };
    assert_eq!(
        diagnostics.diagnostics()[0].code(),
        DiagnosticCode::InvalidVehicleProfileValue
    );

    let mut invalid_order = access_builder("vehicle-profile-invalid-order.document");
    invalid_order
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "car",
            extends: None,
        })
        .unwrap();
    let mut iidm = canonical_iidm_profile();
    iidm.emergency_deceleration_meters_per_second_squared = 1.0;
    let diagnostics = match invalid_order.add_vehicle_profile(VehicleProfileInput {
        vehicle_profile_key: "invalid-order",
        participant_class: ParticipantClassReference::local("car"),
        iidm,
    }) {
        Ok(_) => panic!("invalid deceleration order must fail"),
        Err(diagnostics) => diagnostics,
    };
    assert_eq!(
        diagnostics.diagnostics()[0].code(),
        DiagnosticCode::InvalidVehicleProfileDecelerationOrder
    );

    let mut invalid_accel = access_builder("vehicle-profile-invalid-accel.document");
    invalid_accel
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "car",
            extends: None,
        })
        .unwrap();
    let mut iidm = canonical_iidm_profile();
    iidm.max_acceleration_meters_per_second_squared = 0.499;
    let diagnostics = match invalid_accel.add_vehicle_profile(VehicleProfileInput {
        vehicle_profile_key: "invalid-accel",
        participant_class: ParticipantClassReference::local("car"),
        iidm,
    }) {
        Ok(_) => panic!("maxAccel below 0.5 must fail"),
        Err(diagnostics) => diagnostics,
    };
    assert_eq!(
        diagnostics.diagnostics()[0].code(),
        DiagnosticCode::InvalidVehicleProfileValue
    );
}

#[test]
fn vehicle_profile_unknown_participant_class_fails_during_hir_resolution() {
    let mut builder = access_builder("vehicle-profile-unknown-class.document");
    builder
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "standard-car",
            participant_class: ParticipantClassReference::local("missing"),
            iidm: canonical_iidm_profile(),
        })
        .unwrap();
    assert_eq!(
        compile_diagnostic_codes(builder),
        [DiagnosticCode::UnknownReferenceTarget]
    );
}

#[test]
fn compiler_freezes_participant_hierarchy_access_rules_and_sources() {
    let output = Compiler::new()
        .compile(unit([access_semantics_module(false)]))
        .unwrap();
    assert_eq!(output.lir().participant_classes().len(), 2);
    assert_eq!(output.lir().access_rules().len(), 2);
    let classes = output
        .lir()
        .participant_classes()
        .map(|class| {
            (
                stable_key(class.identity_fields(), FieldTag::ParticipantClassKey),
                class.ordinal(),
                class.parent(),
                class.depth(),
            )
        })
        .collect::<Vec<_>>();
    let road_user = classes.iter().find(|class| class.0 == "road-user").unwrap();
    let car = classes.iter().find(|class| class.0 == "car").unwrap();
    assert_eq!(road_user.2, None);
    assert_eq!(road_user.3, 0);
    assert_eq!(car.2, Some(road_user.1));
    assert_eq!(car.3, 1);
    assert!(
        output
            .lir()
            .participant_class(road_user.1)
            .unwrap()
            .contains(car.1)
    );

    let allow = output
        .lir()
        .access_rules()
        .find(|rule| {
            stable_key(rule.identity_fields(), FieldTag::AccessRuleKey) == "allow-road-users"
        })
        .unwrap();
    assert_eq!(allow.effect(), AccessEffect::Allow);
    assert_eq!(allow.participant_classes(), &[road_user.1]);
    assert_eq!(allow.regulation().unwrap().jurisdiction(), "CN-test");
    assert!(matches!(allow.target(), CanonicalAccessTarget::LaneEdge(_)));

    let source_map = output.source_map_input();
    assert_eq!(source_map.participant_class_sources().len(), 2);
    assert_eq!(source_map.access_rule_sources().len(), 2);
    assert_eq!(source_map.access_relation_sources().len(), 5);
    assert_eq!(
        source_map
            .access_relation_sources()
            .map(|relation| relation.role())
            .collect::<Vec<_>>(),
        [
            SourceRelationRole::ParticipantClassExtends,
            SourceRelationRole::AccessRuleTarget,
            SourceRelationRole::AccessRuleParticipantClass,
            SourceRelationRole::AccessRuleTarget,
            SourceRelationRole::AccessRuleParticipantClass,
        ]
    );

    let permuted = Compiler::new()
        .compile(unit([access_semantics_module(true)]))
        .unwrap();
    assert_eq!(
        output.lir.inner.semantic_digest,
        permuted.lir.inner.semantic_digest
    );
}

#[test]
fn access_validation_rejects_inheritance_cycles_and_exact_rule_ties() {
    let mut cycle = access_builder("access-cycle.document");
    cycle
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "a",
            extends: Some(ParticipantClassReference::local("b")),
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "b",
            extends: Some(ParticipantClassReference::local("a")),
        })
        .unwrap();
    assert_eq!(
        compile_diagnostic_codes(cycle),
        [DiagnosticCode::ParticipantClassInheritanceCycle]
    );

    let mut ambiguity = access_builder("access-ambiguity.document");
    ambiguity
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "all",
            extends: None,
        })
        .unwrap();
    for (key, effect) in [
        ("allow-all", AccessEffect::Allow),
        ("deny-all", AccessEffect::Deny),
    ] {
        ambiguity
            .add_access_rule(AccessRuleInput {
                access_rule_key: key,
                target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-main")),
                effect,
                participant_classes: &[ParticipantClassReference::local("all")],
                regulation: None,
                priority: 0,
            })
            .unwrap();
    }
    assert_eq!(
        compile_diagnostic_codes(ambiguity),
        [DiagnosticCode::AccessRuleAmbiguity]
    );
}

#[test]
fn compiler_preserves_all_supported_access_target_planes() {
    let mut edge_targets = access_builder("access-edge-targets.document");
    edge_targets
        .add_lane_group(LaneGroupInput {
            lane_group_key: "group-main",
            road_section: RoadSectionReference::local("section-main"),
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "section-main",
            kind_id: "motorLane",
            lanes: &[AuthoringLaneInput {
                authoring_lane_key: "lane-main",
                edge_chain: &[LaneEdgeReference::local("edge-main")],
                lane_group: Some(LaneGroupReference::local("group-main")),
            }],
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "corridor-main",
            reference_section: RoadSectionReference::local("section-main"),
            elements: &[CorridorElementReference::road_section(
                RoadSectionReference::local("section-main"),
            )],
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "all",
            extends: None,
        })
        .unwrap();
    for (key, target, effect) in [
        (
            "rule-edge",
            AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-main")),
            AccessEffect::Deny,
        ),
        (
            "rule-group",
            AccessRuleTargetInput::LaneGroup(LaneGroupReference::local("group-main")),
            AccessEffect::Allow,
        ),
        (
            "rule-section",
            AccessRuleTargetInput::RoadSection(RoadSectionReference::local("section-main")),
            AccessEffect::Deny,
        ),
    ] {
        edge_targets
            .add_access_rule(AccessRuleInput {
                access_rule_key: key,
                target,
                effect,
                participant_classes: &[ParticipantClassReference::local("all")],
                regulation: None,
                priority: 0,
            })
            .unwrap();
    }
    let edge_output = Compiler::new()
        .compile(unit([edge_targets.finish().unwrap()]))
        .unwrap();
    assert_eq!(
        edge_output
            .lir()
            .access_rules()
            .map(|rule| match rule.target() {
                CanonicalAccessTarget::LaneEdge(_) => "edge",
                CanonicalAccessTarget::LaneGroup(_) => "group",
                CanonicalAccessTarget::RoadSection(_) => "section",
                CanonicalAccessTarget::ManeuverPath(_) => "path",
            })
            .collect::<Vec<_>>(),
        ["edge", "group", "section"]
    );

    let mut path_target = junction_builder("access-path-target.document");
    path_target
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[LaneEdgeReference::local("exit")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_junction(JunctionInput {
            junction_key: "junction-main",
        })
        .unwrap()
        .add_movement(MovementInput {
            movement_key: "movement-main",
            junction: JunctionReference::local("junction-main"),
            directed_entry_approach_key: "approach-entry",
            directed_exit_approach_key: "approach-exit",
        })
        .unwrap()
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path-main",
            movement: MovementReference::local("movement-main"),
            entry_edge: LaneEdgeReference::local("entry"),
            internal_edges: &[],
            exit_edge: LaneEdgeReference::local("exit"),
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "all",
            extends: None,
        })
        .unwrap()
        .add_access_rule(AccessRuleInput {
            access_rule_key: "rule-path",
            target: AccessRuleTargetInput::ManeuverPath(ManeuverPathReference::local("path-main")),
            effect: AccessEffect::Deny,
            participant_classes: &[ParticipantClassReference::local("all")],
            regulation: None,
            priority: 7,
        })
        .unwrap();
    let path_output = Compiler::new()
        .compile(unit([path_target.finish().unwrap()]))
        .unwrap();
    let path_rule = path_output.lir().access_rules().next().unwrap();
    assert!(matches!(
        path_rule.target(),
        CanonicalAccessTarget::ManeuverPath(_)
    ));
    assert_eq!(path_rule.priority(), 7);
}

#[test]
fn access_validation_closes_shape_capability_reference_and_regulation_failures() {
    let mut empty = access_builder("access-empty-classes.document");
    empty
        .add_access_rule(AccessRuleInput {
            access_rule_key: "empty",
            target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-main")),
            effect: AccessEffect::Allow,
            participant_classes: &[],
            regulation: None,
            priority: 0,
        })
        .unwrap();
    assert_eq!(
        compile_diagnostic_codes(empty),
        [DiagnosticCode::EmptyAccessRuleParticipantClasses]
    );

    let mut unknown = access_builder("access-unknown-class.document");
    unknown
        .add_access_rule(AccessRuleInput {
            access_rule_key: "unknown",
            target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-main")),
            effect: AccessEffect::Allow,
            participant_classes: &[ParticipantClassReference::local("missing")],
            regulation: None,
            priority: 0,
        })
        .unwrap();
    assert_eq!(
        compile_diagnostic_codes(unknown),
        [DiagnosticCode::UnknownReferenceTarget]
    );

    let mut facility = access_builder("access-facility-band.document");
    facility
        .add_facility_band(FacilityBandInput {
            facility_band_key: "band-main",
            kind_id: "sidewalk",
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "section-main",
            kind_id: "motorLane",
            lanes: &[AuthoringLaneInput {
                authoring_lane_key: "lane-main",
                edge_chain: &[LaneEdgeReference::local("edge-main")],
                lane_group: None,
            }],
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "corridor-main",
            reference_section: RoadSectionReference::local("section-main"),
            elements: &[
                CorridorElementReference::road_section(RoadSectionReference::local("section-main")),
                CorridorElementReference::facility_band(FacilityBandReference::local("band-main")),
            ],
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "all",
            extends: None,
        })
        .unwrap()
        .add_access_rule(AccessRuleInput {
            access_rule_key: "band-rule",
            target: AccessRuleTargetInput::FacilityBand(FacilityBandReference::local("band-main")),
            effect: AccessEffect::Allow,
            participant_classes: &[ParticipantClassReference::local("all")],
            regulation: None,
            priority: 0,
        })
        .unwrap();
    let diagnostics = match Compiler::new().compile(unit([facility.finish().unwrap()])) {
        Ok(_) => panic!("FacilityBand target must fail closed"),
        Err(diagnostics) => diagnostics,
    };
    assert!(matches!(
        diagnostics.diagnostics()[0].payload(),
        DiagnosticPayload::AccessCapabilityUnavailable {
            capability: AccessCapability::FacilityBandTarget,
            ..
        }
    ));

    let mut invalid_regulation = access_builder("access-invalid-regulation.document");
    invalid_regulation
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "all",
            extends: None,
        })
        .unwrap()
        .add_access_rule(AccessRuleInput {
            access_rule_key: "invalid-regulation",
            target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-main")),
            effect: AccessEffect::Allow,
            participant_classes: &[ParticipantClassReference::local("all")],
            regulation: Some(AccessRegulationInput {
                jurisdiction: "",
                version: "2026-01",
                source: None,
            }),
            priority: 0,
        })
        .unwrap();
    assert_eq!(
        compile_diagnostic_codes(invalid_regulation),
        [DiagnosticCode::InvalidAccessRegulationString]
    );

    let mut mismatch = access_builder("access-regulation-mismatch.document");
    mismatch
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "all",
            extends: None,
        })
        .unwrap();
    for (key, jurisdiction) in [("rule-a", "CN-a"), ("rule-b", "CN-b")] {
        mismatch
            .add_access_rule(AccessRuleInput {
                access_rule_key: key,
                target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-main")),
                effect: AccessEffect::Allow,
                participant_classes: &[ParticipantClassReference::local("all")],
                regulation: Some(AccessRegulationInput {
                    jurisdiction,
                    version: "2026-01",
                    source: None,
                }),
                priority: 0,
            })
            .unwrap();
    }
    assert_eq!(
        compile_diagnostic_codes(mismatch),
        [DiagnosticCode::AccessRegulationMismatch]
    );
}
