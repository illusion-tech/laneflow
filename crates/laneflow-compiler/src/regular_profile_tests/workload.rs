use super::compiler_api::road_editing as editing;
use super::compiler_api::*;

// 三个共享准入实体与一个覆盖完整路网的路权策略集。
const FIXED_ENTITIES: u32 = 4;
#[derive(Clone, Copy, Debug)]
pub(super) struct WorkloadShape {
    pub(super) synthetic_tiles: u32,
    pub(super) conflict_tiles: u32,
    pub(super) filler_entities: u32,
    pub(super) signals: u32,
    pub(super) geometry_points: u32,
    pub(super) disjoint: bool,
    pub(super) changed: bool,
    pub(super) reordered: bool,
    pub(super) source_partitions: u32,
    pub(super) long_documents: bool,
    pub(super) curved: bool,
    pub(super) policy_evidence: u32,
    pub(super) policy_selectors: bool,
    pub(super) contributing_sources: bool,
}

impl WorkloadShape {
    pub(super) const fn for_stable_entities(
        stable_entities: u32,
        signals: u32,
        geometry_points: u32,
    ) -> Self {
        let conflict_tiles = stable_entities / 800;
        let conflict_entities = conflict_tiles * 35;
        let synthetic_entities = stable_entities - conflict_entities - FIXED_ENTITIES;
        let tile_entities = 14 + 5 * signals;
        Self {
            synthetic_tiles: synthetic_entities / tile_entities,
            conflict_tiles,
            filler_entities: synthetic_entities % tile_entities,
            signals,
            geometry_points,
            disjoint: false,
            changed: false,
            reordered: false,
            source_partitions: 1,
            long_documents: false,
            curved: false,
            policy_evidence: 0,
            policy_selectors: false,
            contributing_sources: false,
        }
    }
}

const JOINT: WorkloadShape = WorkloadShape::for_stable_entities(10_000, 32, 65);
pub(super) const CANDIDATE_OPERATION_BYTES: u64 = 256 * 1024 * 1024;
pub(super) const SHAPES: [(&str, WorkloadShape); 11] = [
    ("mixed", WorkloadShape::for_stable_entities(10_000, 2, 2)),
    (
        "signal-dense",
        WorkloadShape::for_stable_entities(10_000, 32, 2),
    ),
    (
        "geometry-dense",
        WorkloadShape::for_stable_entities(10_000, 2, 65),
    ),
    ("joint", JOINT),
    (
        "partitioned",
        WorkloadShape {
            source_partitions: 16,
            ..JOINT
        },
    ),
    (
        "long-documents",
        WorkloadShape {
            long_documents: true,
            ..JOINT
        },
    ),
    (
        "curved",
        WorkloadShape {
            curved: true,
            ..JOINT
        },
    ),
    (
        "combined",
        WorkloadShape {
            source_partitions: 16,
            long_documents: true,
            curved: true,
            ..JOINT
        },
    ),
    (
        "policy",
        WorkloadShape {
            policy_evidence: 16,
            policy_selectors: true,
            ..JOINT
        },
    ),
    (
        "sources",
        WorkloadShape {
            contributing_sources: true,
            ..JOINT
        },
    ),
    (
        "all",
        WorkloadShape {
            source_partitions: 16,
            long_documents: true,
            curved: true,
            policy_evidence: 16,
            policy_selectors: true,
            contributing_sources: true,
            ..JOINT
        },
    ),
];

fn synthetic_namespace(shape: WorkloadShape, partition: u32) -> String {
    let namespace = if shape.disjoint {
        "city/regular-profile-other"
    } else {
        "city/regular-profile"
    };
    if partition == 0 {
        namespace.to_owned()
    } else {
        format!("{namespace}-{partition:03}")
    }
}

fn document_key(base: &str, long: bool, bytes: usize) -> String {
    if long {
        format!("{base}{}", "x".repeat(bytes - base.len()))
    } else {
        base.to_owned()
    }
}

fn key(prefix: &str, ordinal: u32, suffix: &str) -> String {
    format!("{prefix}{ordinal:07}-{suffix}")
}

fn add_synthetic_tile(
    builder: &mut SyntheticModuleBuilder,
    ordinal: u32,
    shape: WorkloadShape,
) -> Result<(), DiagnosticBundle> {
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
    let waiting = key("t", ordinal, "waiting");
    let lane_group = key("t", ordinal, "lane-group");
    let extra_edges: Vec<_> = (2..shape.signals)
        .map(|index| key("t", ordinal, &format!("middle-{index:03}")))
        .collect();
    let mut internal_edges = vec![LaneEdgeReference::local(&middle)];
    internal_edges.extend(
        extra_edges
            .iter()
            .map(|name| LaneEdgeReference::local(name)),
    );
    for (index, edge) in extra_edges.iter().enumerate() {
        let successor = extra_edges.get(index + 1).unwrap_or(&exit);
        builder.add_lane_edge(LaneEdgeInput {
            lane_edge_key: edge,
            length_meters: 8.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[LaneEdgeReference::local(successor)],
        })?;
    }

    builder.add_lane_group(LaneGroupInput {
        lane_group_key: &lane_group,
        road_section: RoadSectionReference::local(&section),
    })?;

    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: &entry,
            length_meters: 10.0,
            speed_limit_meters_per_second: if shape.changed { 14.0 } else { 13.0 },
            successors: &[LaneEdgeReference::local(&middle)],
        })?
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: &middle,
            length_meters: 8.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[LaneEdgeReference::local(
                extra_edges.first().unwrap_or(&exit),
            )],
        })?
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: &exit,
            length_meters: 12.0,
            speed_limit_meters_per_second: 13.0,
            successors: &[],
        })?
        .add_road_section(RoadSectionInput {
            road_section_key: &section,
            kind_id: "motorLane",
            lanes: &[
                AuthoringLaneInput {
                    authoring_lane_key: &lane_entry,
                    edge_chain: &[LaneEdgeReference::local(&entry)],
                    lane_group: Some(LaneGroupReference::local(&lane_group)),
                },
                AuthoringLaneInput {
                    authoring_lane_key: &lane_exit,
                    edge_chain: &[LaneEdgeReference::local(&exit)],
                    lane_group: Some(LaneGroupReference::local(&lane_group)),
                },
            ],
        })?
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: &corridor,
            reference_section: RoadSectionReference::local(&section),
            elements: &[CorridorElementReference::road_section(
                RoadSectionReference::local(&section),
            )],
        })?
        .add_junction(JunctionInput {
            junction_key: &junction,
        })?
        .add_movement(MovementInput {
            turn_direction: None,
            movement_key: &movement,
            junction: JunctionReference::local(&junction),
            directed_entry_approach_key: "westbound",
            directed_exit_approach_key: "eastbound",
        })?
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: &path,
            movement: MovementReference::local(&movement),
            entry_edge: LaneEdgeReference::local(&entry),
            internal_edges: &internal_edges,
            exit_edge: LaneEdgeReference::local(&exit),
        })?
        .add_stop_line(StopLineInput {
            stop_line_key: &stop_entry,
            lane_edge: LaneEdgeReference::local(&entry),
        })?
        .add_stop_line(StopLineInput {
            stop_line_key: &stop_middle,
            lane_edge: LaneEdgeReference::local(&middle),
        })?
        .add_signal_group(SignalGroupInput {
            signal_group_key: &group_entry,
        })?
        .add_signal_group(SignalGroupInput {
            signal_group_key: &group_release,
        })?
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: &gate_entry,
            maneuver_path: ManeuverPathReference::local(&path),
            transition_index: 0,
            stop_line: StopLineReference::local(&stop_entry),
            signal_control: SignalControlInput::Group(SignalGroupReference::local(&group_entry)),
        })?
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: &gate_release,
            maneuver_path: ManeuverPathReference::local(&path),
            transition_index: 1,
            stop_line: StopLineReference::local(&stop_middle),
            signal_control: SignalControlInput::Group(SignalGroupReference::local(&group_release)),
        })?;

    builder.add_waiting_zone(WaitingZoneInput {
        waiting_zone_key: &waiting,
        maneuver_path: ManeuverPathReference::local(&path),
        entry_gate: ManeuverGateReference::local(&gate_entry),
        release_gate: ManeuverGateReference::local(&gate_release),
        max_occupancy: 3,
    })?;
    let mut group_keys = vec![group_entry, group_release];
    for index in 2..shape.signals {
        let group = key("t", ordinal, &format!("group-{index:03}"));
        builder.add_signal_group(SignalGroupInput {
            signal_group_key: &group,
        })?;
        let stop = key("t", ordinal, &format!("stop-{index:03}"));
        let gate = key("t", ordinal, &format!("gate-{index:03}"));
        builder.add_stop_line(StopLineInput {
            stop_line_key: &stop,
            lane_edge: LaneEdgeReference::local(&extra_edges[index as usize - 2]),
        })?;
        builder.add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: &gate,
            maneuver_path: ManeuverPathReference::local(&path),
            transition_index: index,
            stop_line: StopLineReference::local(&stop),
            signal_control: SignalControlInput::Group(SignalGroupReference::local(&group)),
        })?;
        group_keys.push(group);
    }
    let groups: Vec<_> = group_keys
        .iter()
        .map(|name| SignalGroupReference::local(name))
        .collect();
    let mut phase_keys = vec![phase_go, phase_clear];
    for index in 2..shape.signals {
        phase_keys.push(key("t", ordinal, &format!("phase-{index:03}")));
    }
    let states: Vec<Vec<_>> = (0..shape.signals)
        .map(|phase| {
            groups
                .iter()
                .enumerate()
                .map(|(index, group)| SignalGroupStateInput {
                    signal_group: *group,
                    aspect: if index == (phase as usize + usize::from(shape.changed)) % groups.len()
                    {
                        SignalAspect::Green
                    } else {
                        SignalAspect::Red
                    },
                })
                .collect()
        })
        .collect();
    let mut phases: Vec<_> = phase_keys
        .iter()
        .zip(&states)
        .map(|(name, states)| SignalPhaseInput {
            signal_phase_key: name,
            duration_ms: 5_000,
            states,
        })
        .collect();
    if shape.reordered {
        phases.reverse();
    }
    builder
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: &controller,
            offset_ms: u64::from(ordinal % 1_000),
            signal_groups: &groups,
            phases: &phases,
        })?
        .add_parking_facility(ParkingFacilityInput {
            parking_facility_key: &parking,
            virtual_capacity: 0,
            virtual_entries: &[],
            virtual_exits: &[],
        })?
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
        })?;

    let points = |start: f32, end: f32| {
        (0..shape.geometry_points)
            .map(|index| CanonicalPoint3F32Input {
                x: start + (end - start) * index as f32 / (shape.geometry_points - 1) as f32,
                y: if shape.changed { 0.25 } else { 0.0 },
                z: 0.0,
            })
            .collect::<Vec<_>>()
    };
    let entry_points = points(0.0, 10.0);
    let middle_points = points(10.0, 18.0);
    let exit_start = 18.0 + 8.0 * (shape.signals - 2) as f32;
    let exit_points = points(exit_start, exit_start + 12.0);
    let extra_points: Vec<_> = extra_edges
        .iter()
        .enumerate()
        .map(|(index, _)| points(18.0 + 8.0 * index as f32, 26.0 + 8.0 * index as f32))
        .collect();
    let mut geometries: Vec<_> = extra_edges
        .iter()
        .zip(&extra_points)
        .map(|(edge, points)| LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local(edge),
            centerline_points: points,
        })
        .collect();
    geometries.extend([
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
    ]);
    builder.add_canonical_frame(CanonicalFrameInput {
        canonical_frame_key: &frame,
        lane_edge_geometries: &geometries,
    })?;
    Ok(())
}

fn build_conflict_module(
    limits: &CompileProfile,
    shape: WorkloadShape,
    permuted: bool,
) -> Option<editing::OwnedRoadEditingSourceBuffer> {
    let tile_count = shape.conflict_tiles;
    if tile_count == 0 {
        return None;
    }
    let header = editing::RoadEditingModuleHeader::try_new(
        if shape.disjoint {
            "city/regular-profile-conflicts-other"
        } else {
            "city/regular-profile-conflicts"
        },
        document_key(
            "portable-resource-conflicts.document",
            shape.long_documents,
            53,
        ),
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
            let point = |x, y| editing::RoadEditingPoint3::try_new(x, y, 0.0).unwrap();
            let delta = end_x - start_x;
            let segments = if shape.curved {
                vec![
                    editing::RoadEditingCurveSegment::cubic_bezier(
                        point(start_x + delta / 6.0, 0.0),
                        point(start_x + delta / 3.0, 3.0),
                        point(start_x + delta / 2.0, 3.0),
                    ),
                    editing::RoadEditingCurveSegment::cubic_bezier(
                        point(start_x + 2.0 * delta / 3.0, 3.0),
                        point(start_x + 5.0 * delta / 6.0, 0.0),
                        point(end_x, 0.0),
                    ),
                ]
            } else {
                vec![editing::RoadEditingCurveSegment::line(point(end_x, 0.0))]
            };
            editing::RoadEditingCurveProgram::try_new(point(start_x, 0.0), segments).unwrap()
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
            let band_key = key("c", ordinal, &format!("band-{suffix}"));
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
                        vec![
                            editing::RoadEditingCorridorElement::RoadSection(section.clone()),
                            editing::RoadEditingCorridorElement::FacilityBand(
                                editing::FacilityBandReference::owner_scoped(
                                    vec![corridor_key.clone()],
                                    &band_key,
                                )
                                .unwrap(),
                            ),
                        ],
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::RoadSection(
                    editing::RoadSectionInput::try_new(
                        &section_key,
                        "motorLane",
                        vec![lane],
                        corridor.clone(),
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
            builder
                .add_declaration(editing::RoadEditingDeclaration::FacilityBand(
                    editing::FacilityBandInput::try_new(
                        &band_key,
                        "sidewalk",
                        editing::LinearWidthProfile::try_new(2.0, 2.0).unwrap(),
                        corridor,
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

pub(super) fn build_synthetic_module(
    limits: &CompileProfile,
    shape: WorkloadShape,
    partition: u32,
    permuted: bool,
) -> Result<SyntheticModule, DiagnosticBundle> {
    let namespace = synthetic_namespace(shape, partition);
    let base_document = if partition == 0 {
        "portable-resource-probe.document".to_owned()
    } else {
        format!("portable-resource-probe-{partition:03}.document")
    };
    let document = document_key(&base_document, shape.long_documents, 1_024);
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: &namespace,
            source_document_key: &document,
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: Some(42),
            provenance: "repository:laneflow",
        },
        limits,
    )?;
    let mut module = SyntheticModuleBuilder::new(header, limits)?;
    let mut synthetic_ordinals = (0..shape.synthetic_tiles)
        .filter(|ordinal| ordinal % shape.source_partitions == partition)
        .collect::<Vec<_>>();
    let mut filler_ordinals = (0..shape.filler_entities).collect::<Vec<_>>();
    if permuted {
        synthetic_ordinals.reverse();
        filler_ordinals.reverse();
    }
    for ordinal in synthetic_ordinals {
        add_synthetic_tile(&mut module, ordinal, shape)?;
    }
    for ordinal in filler_ordinals.into_iter().filter(|_| partition == 0) {
        let filler = key("f", ordinal, "participant");
        module.add_participant_class(ParticipantClassInput {
            participant_class_key: &filler,
            extends: None,
        })?;
    }
    if partition == 0 {
        let shared_class = "shared-car";
        let shared_profile = "shared-vehicle";
        let shared_access = "shared-access";
        let first_entry = key("t", 0, "entry");
        module
            .add_participant_class(ParticipantClassInput {
                participant_class_key: shared_class,
                extends: None,
            })?
            .add_vehicle_profile(VehicleProfileInput {
                geometry: RoadVehicleGeometryInput::RigidTwoAxle(RigidTwoAxleGeometryInput {
                    wheelbase_m: 2.6,
                    track_width_m: 1.6,
                    reference_height_m: 0.35,
                    rear_overhang_m: 0.75,
                    width_m: 1.9,
                    bottom_height_m: -0.15,
                    top_height_m: 1.15,
                    max_steering_angle_rad: 0.6,
                    max_steering_rate_rad_s: 0.7,
                }),
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
            })?
            .add_access_rule(AccessRuleInput {
                access_rule_key: shared_access,
                target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local(&first_entry)),
                effect: AccessEffect::Allow,
                participant_classes: &[ParticipantClassReference::local(shared_class)],
                regulation: None,
                priority: 0,
            })?;
    }
    module.finish()
}

pub(super) fn build_mixed_network(shape: WorkloadShape, permuted: bool) -> CompilationUnit {
    assemble_with_frontend_profile(
        shape,
        permuted,
        CompileProfile::single_network_10k_stable_entities(),
        CompileProfile::single_network_10k_stable_entities(),
    )
    .unwrap()
    .build()
    .unwrap()
}

// 前端使用正式常规档；返回构建器供包内测试读取计数或独立收紧编译单元的限额。
#[allow(dead_code)] // 独立集成测试只使用全程常规档构造入口。
pub(super) fn assemble_mixed_network(
    shape: WorkloadShape,
    permuted: bool,
    admission_limits: CompileProfile,
) -> Result<CompilationUnitBuilder, DiagnosticBundle> {
    assemble_with_frontend_profile(
        shape,
        permuted,
        admission_limits,
        CompileProfile::single_network_10k_stable_entities(),
    )
}

fn assemble_with_frontend_profile(
    shape: WorkloadShape,
    permuted: bool,
    admission_limits: CompileProfile,
    limits: CompileProfile,
) -> Result<CompilationUnitBuilder, DiagnosticBundle> {
    assert!(shape.source_partitions > 0 && shape.source_partitions <= shape.synthetic_tiles);
    let conflicts = build_conflict_module(&limits, shape, permuted);
    let policy = build_policy_module(&limits, shape);
    let synthetic: Vec<_> = (0..shape.source_partitions)
        .map(|partition| build_synthetic_module(&limits, shape, partition, permuted).unwrap())
        .collect();
    let mut unit = CompilationUnitBuilder::new(admission_limits);
    unit.add_synthetic_module(policy)?;
    let conflict_document = document_key(
        "portable-resource-conflicts.document",
        shape.long_documents,
        53,
    );
    let add_conflicts =
        |unit: &mut CompilationUnitBuilder, conflicts: &editing::OwnedRoadEditingSourceBuffer| {
            unit.add_road_editing_module(
                editing::RoadEditingModuleInput::try_new(
                    &conflict_document,
                    conflicts.as_bytes(),
                    None,
                )
                .unwrap(),
            )?;
            Ok::<_, DiagnosticBundle>(())
        };
    if permuted {
        if let Some(conflicts) = conflicts.as_ref() {
            add_conflicts(&mut unit, conflicts)?;
        }
        for module in synthetic.into_iter().rev() {
            unit.add_synthetic_module(module)?;
        }
    } else {
        for module in synthetic {
            unit.add_synthetic_module(module)?;
        }
        if let Some(conflicts) = conflicts.as_ref() {
            add_conflicts(&mut unit, conflicts)?;
        }
    }
    Ok(unit)
}

fn build_policy_module(limits: &CompileProfile, shape: WorkloadShape) -> SyntheticModule {
    let conflict_namespace = if shape.disjoint {
        "city/regular-profile-conflicts-other"
    } else {
        "city/regular-profile-conflicts"
    };
    let document = document_key("profile-policy.document", shape.long_documents, 1_024);
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: if shape.disjoint {
                "city/regular-profile-policy-other"
            } else {
                "city/regular-profile-policy"
            },
            source_document_key: &document,
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: Some(42),
            provenance: "repository:laneflow",
        },
        limits,
    )
    .unwrap();
    let mut builder = SyntheticModuleBuilder::new(header, limits).unwrap();
    for partition in 0..shape.source_partitions {
        builder
            .add_import(&synthetic_namespace(shape, partition))
            .unwrap();
    }
    builder.add_import(conflict_namespace).unwrap();
    let span = builder.policy_source_span();
    let contributions = [
        builder.policy_source_span(),
        builder.policy_source_span(),
        builder.policy_source_span(),
        builder.policy_source_span(),
    ];
    let source = PolicyInputSource {
        primary: &span,
        contributing: if shape.contributing_sources {
            &contributions
        } else {
            &[]
        },
    };
    let evidence_keys: Vec<_> = (0..shape.policy_evidence)
        .map(|index| format!("basis-{index:03}"))
        .collect();
    let evidence_refs: Vec<_> = evidence_keys.iter().map(String::as_str).collect();
    let evidence: Vec<_> = evidence_keys
        .iter()
        .map(|key| PolicyEvidenceInput {
            evidence_key: key,
            locator: "repository:regular-profile-policy",
            description: Some("regular-profile policy coverage"),
            source,
        })
        .collect();
    let gaps: Vec<_> = (shape.policy_evidence > 0)
        .then_some(PolicyGapProfileInput {
            profile_key: "gap",
            parameter_version: "gap-1",
            minimum_lead_gap_ms: if shape.changed { 1_200 } else { 1_000 },
            minimum_lag_gap_ms: 500,
            clearance_buffer_ms: 100,
            source,
        })
        .into_iter()
        .collect();
    let mut gate_storage = Vec::new();
    let mut stream_storage = Vec::new();
    for ordinal in 0..shape.synthetic_tiles {
        let namespace = synthetic_namespace(shape, ordinal % shape.source_partitions);
        for suffix in ["entry", "release"] {
            gate_storage.push((
                key("t", ordinal, suffix),
                namespace.clone(),
                key("t", ordinal, &format!("gate-{suffix}")),
                vec![],
                GateInterpretation::ProtectedGroup,
            ));
        }
        for index in 2..shape.signals {
            gate_storage.push((
                key("t", ordinal, &format!("extra-{index:03}")),
                namespace.clone(),
                key("t", ordinal, &format!("gate-{index:03}")),
                vec![],
                GateInterpretation::ProtectedGroup,
            ));
        }
    }
    for ordinal in 0..shape.conflict_tiles {
        for suffix in ["a", "b"] {
            let junction = key("c", ordinal, "junction");
            gate_storage.push((
                key("c", ordinal, suffix),
                conflict_namespace.to_owned(),
                "admission".into(),
                vec![
                    junction.clone(),
                    format!("movement-{suffix}"),
                    "path".into(),
                ],
                GateInterpretation::Uncontrolled,
            ));
            stream_storage.push((
                key("c", ordinal, suffix),
                format!("stream-{suffix}"),
                vec![junction],
            ));
        }
    }
    let gate_owners: Vec<Vec<&str>> = gate_storage
        .iter()
        .map(|row| row.3.iter().map(String::as_str).collect())
        .collect();
    let stream_owners: Vec<Vec<&str>> = stream_storage
        .iter()
        .map(|row| row.2.iter().map(String::as_str).collect())
        .collect();
    let mut gates: Vec<_> = gate_storage
        .iter()
        .zip(&gate_owners)
        .map(
            |((key, namespace, gate, _, interpretation), owners)| PolicyGateRuleInput {
                rule_key: key,
                gate: OwnerQualifiedReference {
                    target: EntityReference::imported(namespace, gate),
                    owner_keys: owners,
                },
                participant_classes: None,
                interpretation: *interpretation,
                prohibition: GateProhibition::None,
                evidence_keys: &evidence_refs,
                source,
            },
        )
        .collect();
    let yields: Vec<Vec<_>> = stream_storage
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if shape.policy_evidence > 0 && index % 2 == 0 {
                vec![OwnerQualifiedReference {
                    target: EntityReference::imported(
                        conflict_namespace,
                        &stream_storage[index + 1].1,
                    ),
                    owner_keys: &stream_owners[index + 1],
                }]
            } else {
                vec![]
            }
        })
        .collect();
    let streams: Vec<_> = stream_storage
        .iter()
        .zip(&stream_owners)
        .enumerate()
        .map(
            |(index, ((key, stream, _), owners))| PolicyStreamRuleInput {
                rule_key: key,
                stream: OwnerQualifiedReference {
                    target: EntityReference::imported(conflict_namespace, stream),
                    owner_keys: owners,
                },
                participant_classes: None,
                priority: if shape.policy_evidence > 0 {
                    (index % 2 + 1) as i32
                } else {
                    0
                },
                yield_to_streams: &yields[index],
                gap_profile_key: (!yields[index].is_empty()).then_some("gap"),
                evidence_keys: &evidence_refs,
                source,
            },
        )
        .collect();
    let root_namespace = synthetic_namespace(shape, 0);
    let classes = [EntityReference::imported(&root_namespace, "shared-car")];
    let selector_keys: Vec<_> = gate_storage
        .iter()
        .map(|row| format!("{}-car", row.0))
        .collect();
    if shape.policy_selectors {
        let selected: Vec<_> = gates
            .iter()
            .zip(&selector_keys)
            .map(|(gate, key)| PolicyGateRuleInput {
                rule_key: key,
                participant_classes: Some(&classes),
                ..*gate
            })
            .collect();
        gates.extend(selected);
    }
    builder
        .add_right_of_way_policy_set(RightOfWayPolicySetInput {
            policy_set_key: "rules",
            regulation: RegulationIdentity {
                jurisdiction: "engineering",
                version: "candidate-1",
                source: Some("repository:profile-candidate"),
            },
            evidence: &evidence,
            gap_profiles: &gaps,
            stream_rules: &streams,
            gate_rules: &gates,
            source,
        })
        .unwrap();
    builder.finish().unwrap()
}
