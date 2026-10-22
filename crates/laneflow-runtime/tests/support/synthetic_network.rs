//! #987 测量用三档合成路网（与编译器证据测试共用合成链）。
//!
//! 复制自 `crates/laneflow-compiler/tests/portable_emission_resources.rs` 的
//! 合成段（WorkloadShape / add_synthetic_tile / build_conflict_module /
//! build_mixed_network），来源变更时同步本文件；计数语义为稳定静态实体
//! （#380 账本三档），与 urban-generator 的名义个体数不同。

#![allow(dead_code)]
#![allow(unused_imports)]

use laneflow_compiler::road_editing as editing;
use laneflow_compiler::{
    AccessEffect, AccessRuleInput, AccessRuleTargetInput, AuthoringLaneInput, CanonicalFrameInput,
    CanonicalPoint3F32Input, CompilationUnit, CompilationUnitBuilder, CompileLimits, Compiler,
    CorridorElementReference, GeometryAccuracyProfile, GeometryDirectionProfile,
    IidmVehicleProfileInput, JunctionInput, JunctionReference, LaneEdgeGeometryInput,
    LaneEdgeInput, LaneEdgeReference, ManeuverGateInput, ManeuverPathInput, ManeuverPathReference,
    MovementInput, MovementReference, ParkingFacilityInput, ParkingFacilityReference,
    ParkingLaneAnchorInput, ParkingSpaceGeometryInput, ParkingSpaceInput, ParticipantClassInput,
    ParticipantClassReference, PortableDiffBase, PortableEmissionProvenance, RoadCorridorInput,
    RoadSectionInput, RoadSectionReference, SignalAspect, SignalControlInput,
    SignalControllerInput, SignalGroupInput, SignalGroupReference, SignalGroupStateInput,
    SignalPhaseInput, SourceModuleHeader, SourceModuleHeaderInput, StopLineInput,
    StopLineReference, SyntheticModuleBuilder, VehicleProfileInput, check_portable_candidate,
    emit_portable_candidate_to_staging,
};
use laneflow_format::{
    CheckedCanonicalNetworkInput, FormatLimits, ImmutableObjectSource, RegistryCheckedFieldValue,
    RegistryCheckedRowView, ValueCheckedObjectView,
};
use laneflow_static_contract::{EntityKind, PortableObjectKind, Sha256Digest};
use laneflow_static_network::{
    BuildError, BuildStructure, SharedNetworkBuildLimits, SharedNetworkBuildOptions,
    SpatialBuildOption, build_shared_network_revision,
};

pub(crate) const SYNTHETIC_TILE_ENTITIES: u32 = 22;
pub(crate) const SHARED_ACCESS_ENTITIES: u32 = 3;

pub(crate) struct WorkloadShape {
    stable_entities: u32,
    synthetic_tiles: u32,
    conflict_tiles: u32,
    filler_entities: u32,
}

impl WorkloadShape {
    pub(crate) fn for_stable_entities(stable_entities: u32) -> Self {
        let conflict_tiles = stable_entities / 800;
        let conflict_entities = conflict_tiles * 31;
        let synthetic_entities = stable_entities - conflict_entities - SHARED_ACCESS_ENTITIES;
        Self {
            stable_entities,
            synthetic_tiles: synthetic_entities / SYNTHETIC_TILE_ENTITIES,
            conflict_tiles,
            filler_entities: synthetic_entities % SYNTHETIC_TILE_ENTITIES,
        }
    }
}

fn key(prefix: &str, ordinal: u32, suffix: &str) -> String {
    format!("{prefix}{ordinal:07}-{suffix}")
}

fn add_synthetic_tile(builder: &mut SyntheticModuleBuilder, ordinal: u32) {
    let entry = key("t", ordinal, "entry");
    let middle = key("t", ordinal, "middle");
    let exit = key("t", ordinal, "exit");
    let section = key("t", ordinal, "section");
    let lane_entry = key("t", ordinal, "lane-entry");
    let lane_exit = key("t", ordinal, "lane-exit");
    let corridor = key("t", ordinal, "corridor");
    let junction = key("t", ordinal, "junction");
    let movement = key("t", ordinal, "movement");
    let path = key("t", ordinal, "path");
    let stop_entry = key("t", ordinal, "stop-entry");
    let stop_middle = key("t", ordinal, "stop-middle");
    let group_entry = key("t", ordinal, "group-entry");
    let group_release = key("t", ordinal, "group-release");
    let gate_entry = key("t", ordinal, "gate-entry");
    let gate_release = key("t", ordinal, "gate-release");
    let controller = key("t", ordinal, "controller");
    let phase_go = key("t", ordinal, "phase-go");
    let phase_clear = key("t", ordinal, "phase-clear");
    let parking = key("t", ordinal, "parking");
    let space = key("t", ordinal, "space");
    let frame = key("t", ordinal, "frame");

    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: &entry,
            length_meters: 10.0,
            speed_limit_meters_per_second: 13.0,
            successors: &[LaneEdgeReference::local(&middle)],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: &middle,
            length_meters: 8.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[LaneEdgeReference::local(&exit)],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: &exit,
            length_meters: 12.0,
            speed_limit_meters_per_second: 13.0,
            successors: &[],
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: &section,
            kind_id: "motorLane",
            lanes: &[
                AuthoringLaneInput {
                    authoring_lane_key: &lane_entry,
                    edge_chain: &[LaneEdgeReference::local(&entry)],
                    lane_group: None,
                },
                AuthoringLaneInput {
                    authoring_lane_key: &lane_exit,
                    edge_chain: &[LaneEdgeReference::local(&exit)],
                    lane_group: None,
                },
            ],
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: &corridor,
            reference_section: RoadSectionReference::local(&section),
            elements: &[CorridorElementReference::road_section(
                RoadSectionReference::local(&section),
            )],
        })
        .unwrap()
        .add_junction(JunctionInput {
            junction_key: &junction,
        })
        .unwrap()
        .add_movement(MovementInput {
            turn_direction: None,
            movement_key: &movement,
            junction: JunctionReference::local(&junction),
            directed_entry_approach_key: "westbound",
            directed_exit_approach_key: "eastbound",
        })
        .unwrap()
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: &path,
            movement: MovementReference::local(&movement),
            entry_edge: LaneEdgeReference::local(&entry),
            internal_edges: &[LaneEdgeReference::local(&middle)],
            exit_edge: LaneEdgeReference::local(&exit),
        })
        .unwrap()
        .add_stop_line(StopLineInput {
            stop_line_key: &stop_entry,
            lane_edge: LaneEdgeReference::local(&entry),
        })
        .unwrap()
        .add_stop_line(StopLineInput {
            stop_line_key: &stop_middle,
            lane_edge: LaneEdgeReference::local(&middle),
        })
        .unwrap()
        .add_signal_group(SignalGroupInput {
            signal_group_key: &group_entry,
        })
        .unwrap()
        .add_signal_group(SignalGroupInput {
            signal_group_key: &group_release,
        })
        .unwrap()
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: &gate_entry,
            maneuver_path: ManeuverPathReference::local(&path),
            transition_index: 0,
            stop_line: StopLineReference::local(&stop_entry),
            signal_control: SignalControlInput::Group(SignalGroupReference::local(&group_entry)),
        })
        .unwrap()
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: &gate_release,
            maneuver_path: ManeuverPathReference::local(&path),
            transition_index: 1,
            stop_line: StopLineReference::local(&stop_middle),
            signal_control: SignalControlInput::Group(SignalGroupReference::local(&group_release)),
        })
        .unwrap();

    let groups = [
        SignalGroupReference::local(&group_entry),
        SignalGroupReference::local(&group_release),
    ];
    let go_states = [
        SignalGroupStateInput {
            signal_group: SignalGroupReference::local(&group_entry),
            aspect: SignalAspect::Green,
        },
        SignalGroupStateInput {
            signal_group: SignalGroupReference::local(&group_release),
            aspect: SignalAspect::Red,
        },
    ];
    let clear_states = [
        SignalGroupStateInput {
            signal_group: SignalGroupReference::local(&group_entry),
            aspect: SignalAspect::Yellow,
        },
        SignalGroupStateInput {
            signal_group: SignalGroupReference::local(&group_release),
            aspect: SignalAspect::Green,
        },
    ];
    builder
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: &controller,
            offset_ms: u64::from(ordinal % 1_000),
            signal_groups: &groups,
            phases: &[
                SignalPhaseInput {
                    signal_phase_key: &phase_go,
                    duration_ms: 30_000,
                    states: &go_states,
                },
                SignalPhaseInput {
                    signal_phase_key: &phase_clear,
                    duration_ms: 5_000,
                    states: &clear_states,
                },
            ],
        })
        .unwrap()
        .add_parking_facility(ParkingFacilityInput {
            parking_facility_key: &parking,
            virtual_capacity: 0,
            virtual_entries: &[],
            virtual_exits: &[],
        })
        .unwrap()
        .add_parking_space(ParkingSpaceInput {
            parking_space_key: &space,
            parking_facility: Some(ParkingFacilityReference::local(&parking)),
            entry: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local(&entry),
                progress_meters: 4.0,
            },
            exit: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local(&exit),
                progress_meters: 6.0,
            },
            geometry: ParkingSpaceGeometryInput {
                lateral_offset_meters: -3.0,
                heading_offset_radians: 0.0,
                length_meters: 5.5,
                width_meters: 2.6,
            },
        })
        .unwrap();

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
        entry_points[1],
        CanonicalPoint3F32Input {
            x: 18.0,
            y: 0.0,
            z: 0.0,
        },
    ];
    let exit_points = [
        middle_points[1],
        CanonicalPoint3F32Input {
            x: 30.0,
            y: 0.0,
            z: 0.0,
        },
    ];
    builder
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: &frame,
            lane_edge_geometries: &[
                LaneEdgeGeometryInput {
                    lane_edge: LaneEdgeReference::local(&entry),
                    centerline_points: &entry_points,
                },
                LaneEdgeGeometryInput {
                    lane_edge: LaneEdgeReference::local(&middle),
                    centerline_points: &middle_points,
                },
                LaneEdgeGeometryInput {
                    lane_edge: LaneEdgeReference::local(&exit),
                    centerline_points: &exit_points,
                },
            ],
        })
        .unwrap();
}

fn build_conflict_module(
    limits: &CompileLimits,
    tile_count: u32,
    permuted: bool,
) -> Option<editing::OwnedRoadEditingSourceBuffer> {
    if tile_count == 0 {
        return None;
    }
    let header = editing::RoadEditingModuleHeader::try_new(
        "city/portable-resource-conflicts",
        "portable-resource-conflicts.document",
        Vec::new(),
        editing::RoadEditingProvenance::direct("deterministic mixed capacity fixture").unwrap(),
    )
    .unwrap();
    let mut builder = editing::RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        limits,
    )
    .unwrap();
    let mut ordinals = (0..tile_count).collect::<Vec<_>>();
    if permuted {
        ordinals.reverse();
    }
    for ordinal in ordinals {
        let frame_key = key("c", ordinal, "frame");
        let junction_key = key("c", ordinal, "junction");
        let lane_entry_a = key("c", ordinal, "lane-entry-a");
        let lane_exit_a = key("c", ordinal, "lane-exit-a");
        let lane_entry_b = key("c", ordinal, "lane-entry-b");
        let lane_exit_b = key("c", ordinal, "lane-exit-b");
        let entry_a = key("c", ordinal, "entry-a");
        let internal_a = key("c", ordinal, "internal-a");
        let exit_a = key("c", ordinal, "exit-a");
        let entry_b = key("c", ordinal, "entry-b");
        let internal_b = key("c", ordinal, "internal-b");
        let exit_b = key("c", ordinal, "exit-b");
        let stop_a = key("c", ordinal, "stop-a");
        let stop_b = key("c", ordinal, "stop-b");
        let edge = |key: &str| editing::LaneEdgeReference::local(key).unwrap();
        let junction = editing::JunctionReference::local(&junction_key).unwrap();
        let zone = editing::ConflictZoneReference::owner_scoped(vec![junction_key.clone()], "zone")
            .unwrap();
        let curve = |start_x: f64, end_x: f64| {
            editing::RoadEditingCurveProgram::try_new(
                editing::RoadEditingPoint3::try_new(start_x, 0.0, 0.0).unwrap(),
                vec![editing::RoadEditingCurveSegment::line(
                    editing::RoadEditingPoint3::try_new(end_x, 0.0, 0.0).unwrap(),
                )],
            )
            .unwrap()
        };

        builder
            .add_declaration(editing::RoadEditingDeclaration::CanonicalFrame(
                editing::CanonicalFrameInput::try_new(&frame_key).unwrap(),
            ))
            .unwrap();

        for (suffix, lane_key, edge_key) in [
            ("entry-a", &lane_entry_a, &entry_a),
            ("exit-a", &lane_exit_a, &exit_a),
            ("entry-b", &lane_entry_b, &entry_b),
            ("exit-b", &lane_exit_b, &exit_b),
        ] {
            let alignment_key = key("c", ordinal, &format!("alignment-{suffix}"));
            let corridor_key = key("c", ordinal, &format!("corridor-{suffix}"));
            let section_key = key("c", ordinal, &format!("section-{suffix}"));
            let corridor = editing::RoadCorridorReference::local(&corridor_key).unwrap();
            let section = editing::RoadSectionReference::owner_scoped(
                vec![corridor_key.clone()],
                &section_key,
            )
            .unwrap();
            let lane = editing::AuthoringLaneReference::owner_scoped(
                vec![corridor_key.clone(), section_key.clone()],
                lane_key,
            )
            .unwrap();
            let alignment_curve = if suffix.starts_with("entry") {
                curve(0.0, 30.0)
            } else {
                curve(60.0, 90.0)
            };
            builder
                .add_alignment(
                    editing::RoadAlignmentInput::try_new(
                        &alignment_key,
                        editing::CanonicalFrameReference::local(&frame_key).unwrap(),
                        alignment_curve,
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
                        vec![lane],
                        corridor,
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::AuthoringLane(
                    editing::AuthoringLaneInput::try_new(
                        lane_key,
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

        for (edge_key, successors, explicit_geometry) in [
            (&entry_a, Vec::new(), None),
            (&internal_a, Vec::new(), Some(curve(30.0, 60.0))),
            (&exit_a, Vec::new(), None),
            (&entry_b, Vec::new(), None),
            (&internal_b, Vec::new(), Some(curve(30.0, 60.0))),
            (&exit_b, Vec::new(), None),
        ] {
            builder
                .add_declaration(editing::RoadEditingDeclaration::LaneEdge(
                    editing::LaneEdgeInput::try_new(edge_key, 13.0, successors, explicit_geometry)
                        .unwrap(),
                ))
                .unwrap();
        }

        builder
            .add_declaration(editing::RoadEditingDeclaration::Junction(
                editing::JunctionInput::try_new(
                    &junction_key,
                    [&entry_a, &exit_a, &entry_b, &exit_b]
                        .into_iter()
                        .map(|key| edge(key))
                        .collect(),
                    [&internal_a, &internal_b]
                        .into_iter()
                        .map(|key| edge(key))
                        .collect(),
                )
                .unwrap(),
            ))
            .unwrap()
            .add_declaration(editing::RoadEditingDeclaration::ConflictZone(
                editing::ConflictZoneInput::try_new("zone", junction.clone()).unwrap(),
            ))
            .unwrap();

        for (suffix, entry, internal, exit, stop) in [
            ("a", &entry_a, &internal_a, &exit_a, &stop_a),
            ("b", &entry_b, &internal_b, &exit_b, &stop_b),
        ] {
            let movement_key = format!("movement-{suffix}");
            let stream_key = format!("stream-{suffix}");
            let movement = editing::MovementReference::owner_scoped(
                vec![junction_key.clone()],
                movement_key.clone(),
            )
            .unwrap();
            let path = editing::ManeuverPathReference::owner_scoped(
                vec![junction_key.clone(), movement_key.clone()],
                "path",
            )
            .unwrap();
            let admission_gate = editing::ManeuverGateReference::owner_scoped(
                vec![
                    junction_key.clone(),
                    movement_key.clone(),
                    "path".to_owned(),
                ],
                "admission",
            )
            .unwrap();
            let stop_line = editing::StopLineReference::local(stop).unwrap();
            builder
                .add_declaration(editing::RoadEditingDeclaration::Movement(
                    editing::MovementInput::try_new(
                        movement_key,
                        junction.clone(),
                        format!("entry-{suffix}"),
                        format!("exit-{suffix}"),
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
                    editing::StopLineInput::try_new(stop, edge(entry)).unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::ManeuverGate(
                    editing::ManeuverGateInput::try_new(
                        "admission",
                        path.clone(),
                        0,
                        stop_line,
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
                    editing::CanonicalFrameReference::local(&frame_key).unwrap(),
                    -1.0,
                    1.0,
                    vec![
                        editing::RoadEditingPoint2::try_new(0.0, 0.0).unwrap(),
                        editing::RoadEditingPoint2::try_new(1.0, 0.0).unwrap(),
                        editing::RoadEditingPoint2::try_new(1.0, 1.0).unwrap(),
                        editing::RoadEditingPoint2::try_new(0.0, 1.0).unwrap(),
                    ],
                )
                .unwrap(),
            )
            .unwrap();
    }
    Some(
        editing::RoadEditingSourceWriter::new(limits)
            .write(builder.finish().unwrap())
            .unwrap(),
    )
}

pub(crate) fn build_mixed_network(shape: WorkloadShape, permuted: bool) -> CompilationUnit {
    let limits = CompileLimits::single_network_1m_v2();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "city/portable-resource-probe",
            source_document_key: "portable-resource-probe.document",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: Some(42),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .unwrap();
    let mut module = SyntheticModuleBuilder::new(header, &limits).unwrap();
    let mut synthetic_ordinals = (0..shape.synthetic_tiles).collect::<Vec<_>>();
    let mut filler_ordinals = (0..shape.filler_entities).collect::<Vec<_>>();
    if permuted {
        synthetic_ordinals.reverse();
        filler_ordinals.reverse();
    }
    for ordinal in synthetic_ordinals {
        add_synthetic_tile(&mut module, ordinal);
    }
    for ordinal in filler_ordinals {
        let filler = key("f", ordinal, "participant");
        module
            .add_participant_class(ParticipantClassInput {
                participant_class_key: &filler,
                extends: None,
            })
            .unwrap();
    }
    let shared_class = "shared-car";
    let shared_profile = "shared-vehicle";
    let shared_access = "shared-access";
    let first_entry = key("t", 0, "entry");
    module
        .add_participant_class(ParticipantClassInput {
            participant_class_key: shared_class,
            extends: None,
        })
        .unwrap()
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: shared_profile,
            participant_class: ParticipantClassReference::local(shared_class),
            iidm: IidmVehicleProfileInput {
                length_meters: 4.5,
                desired_speed_meters_per_second: 13.0,
                min_gap_meters: 2.0,
                time_headway_seconds: 1.5,
                max_acceleration_meters_per_second_squared: 1.5,
                comfortable_deceleration_meters_per_second_squared: 2.0,
                emergency_deceleration_meters_per_second_squared: 4.0,
            },
        })
        .unwrap()
        .add_access_rule(AccessRuleInput {
            access_rule_key: shared_access,
            target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local(&first_entry)),
            effect: AccessEffect::Allow,
            participant_classes: &[ParticipantClassReference::local(shared_class)],
            regulation: None,
            priority: 0,
        })
        .unwrap();

    let conflicts = build_conflict_module(&limits, shape.conflict_tiles, permuted);
    let mut unit = CompilationUnitBuilder::new(limits);
    let synthetic = module.finish().unwrap();
    let add_conflicts =
        |unit: &mut CompilationUnitBuilder, conflicts: &editing::OwnedRoadEditingSourceBuffer| {
            unit.add_road_editing_module(
                editing::RoadEditingModuleInput::try_new(
                    "portable-resource-conflicts.document",
                    conflicts.as_bytes(),
                    None,
                )
                .unwrap(),
            )
            .unwrap();
        };
    if permuted {
        if let Some(conflicts) = conflicts.as_ref() {
            add_conflicts(&mut unit, conflicts);
        }
        unit.add_synthetic_module(synthetic).unwrap();
    } else {
        unit.add_synthetic_module(synthetic).unwrap();
        if let Some(conflicts) = conflicts.as_ref() {
            add_conflicts(&mut unit, conflicts);
        }
    }
    unit.build().unwrap()
}
