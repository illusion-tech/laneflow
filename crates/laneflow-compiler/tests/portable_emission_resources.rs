use std::{
    alloc::System,
    fs,
    hint::black_box,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

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
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const RESOURCE_TIERS: [u32; 3] = [10_000, 100_000, 1_000_000];
const SYNTHETIC_TILE_ENTITIES: u32 = 22;
const SHARED_ACCESS_ENTITIES: u32 = 3;
const SHARED_BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(2 * 1024 * 1024 * 1024, 2 * 1024 * 1024 * 1024);
const MAX_RETURNED_FILE_BACKED_CANDIDATE_HEAP_BYTES: u64 = 64 * 1024;
const MAX_SHARED_BUILD_ACCOUNTING_OVERHEAD_BYTES: u64 = 64 * 1024;
static NEXT_STAGING_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug)]
struct WorkloadShape {
    stable_entities: u32,
    synthetic_tiles: u32,
    conflict_tiles: u32,
    filler_entities: u32,
}

impl WorkloadShape {
    fn for_stable_entities(stable_entities: u32) -> Self {
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

fn build_mixed_network(shape: WorkloadShape, permuted: bool) -> CompilationUnit {
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

#[derive(Clone, Copy, Debug)]
struct Measurement {
    elapsed_ns: u128,
    stats: Stats,
    sampled_heap_peak_delta_bytes: u64,
}

impl Measurement {
    fn live_delta_bytes(self) -> i128 {
        self.stats.bytes_allocated as i128 - self.stats.bytes_deallocated as i128
    }

    fn positive_live_delta_bytes(self) -> u64 {
        u64::try_from(self.live_delta_bytes().max(0)).expect("positive live delta fits u64")
    }

    fn sampled_transient_heap_peak_bytes(self) -> u64 {
        self.sampled_heap_peak_delta_bytes
            .saturating_sub(self.positive_live_delta_bytes())
    }
}

fn measure<T>(operation: impl FnOnce() -> T) -> (T, Measurement) {
    let baseline = allocator_live_bytes();
    let region = Region::new(GLOBAL);
    let started = Instant::now();
    let output = operation();
    let elapsed_ns = started.elapsed().as_nanos();
    black_box(&output);
    let stats = black_box(region.change());
    let sampled_heap_peak_delta_bytes = allocator_live_bytes().saturating_sub(baseline);
    (
        output,
        Measurement {
            elapsed_ns,
            stats,
            sampled_heap_peak_delta_bytes,
        },
    )
}

fn allocator_live_bytes() -> u64 {
    let stats = GLOBAL.stats();
    u64::try_from(
        stats
            .bytes_allocated
            .saturating_sub(stats.bytes_deallocated),
    )
    .unwrap_or(u64::MAX)
}

fn measure_with_heap_peak<T>(operation: impl FnOnce() -> T) -> (T, Measurement) {
    struct StopSampler<'a>(&'a AtomicBool);
    impl Drop for StopSampler<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    let stop = AtomicBool::new(false);
    let peak = AtomicU64::new(0);
    thread::scope(|scope| {
        scope.spawn(|| {
            while !stop.load(Ordering::Acquire) {
                peak.fetch_max(allocator_live_bytes(), Ordering::Relaxed);
                thread::sleep(Duration::from_millis(1));
            }
            peak.fetch_max(allocator_live_bytes(), Ordering::Relaxed);
        });
        let baseline = allocator_live_bytes();
        peak.store(baseline, Ordering::Relaxed);
        let stop_sampler = StopSampler(&stop);
        let region = Region::new(GLOBAL);
        let started = Instant::now();
        let output = operation();
        let elapsed_ns = started.elapsed().as_nanos();
        black_box(&output);
        let stats = black_box(region.change());
        peak.fetch_max(allocator_live_bytes(), Ordering::Relaxed);
        drop(stop_sampler);
        (
            output,
            Measurement {
                elapsed_ns,
                stats,
                sampled_heap_peak_delta_bytes: peak
                    .load(Ordering::Relaxed)
                    .saturating_sub(baseline),
            },
        )
    })
}

fn print_measurement(
    stage: &str,
    shape: WorkloadShape,
    measurement: Measurement,
    portable_bundle_exact_bytes: u64,
    retained_logical_bytes: u64,
    required_scratch_bytes: u64,
) {
    println!(
        "portable-resource stage={stage} profile=LF-COMP-SINGLE-NETWORK-1M-v2 stable_entities={} synthetic_tiles={} conflict_tiles={} filler_entities={} elapsed_ns={} allocations={} reallocations={} allocated_bytes={} deallocated_bytes={} reallocated_delta_bytes={} live_delta_bytes={} sampled_heap_peak_delta_bytes={} sampled_transient_heap_peak_bytes={} portable_bundle_exact_bytes={portable_bundle_exact_bytes} retained_logical_bytes={retained_logical_bytes} required_scratch_bytes={required_scratch_bytes}",
        shape.stable_entities,
        shape.synthetic_tiles,
        shape.conflict_tiles,
        shape.filler_entities,
        measurement.elapsed_ns,
        measurement.stats.allocations,
        measurement.stats.reallocations,
        measurement.stats.bytes_allocated,
        measurement.stats.bytes_deallocated,
        measurement.stats.bytes_reallocated,
        measurement.live_delta_bytes(),
        measurement.sampled_heap_peak_delta_bytes,
        measurement.sampled_transient_heap_peak_bytes(),
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CanonicalStructureCounts {
    canonical_relation_rows: u64,
    lane_geometry_rows: u32,
    lane_geometry_points: u64,
    lane_geometry_segments: u64,
    facility_geometry_rows: u32,
    facility_geometry_points: u64,
    conflict_region_rows: u32,
    conflict_region_ring_points: u64,
}

fn record_vector_len(row: RegistryCheckedRowView<'_>, tag: u16) -> u64 {
    let field = row.field_by_tag(tag).expect("required record vector");
    match field.value().expect("checked record vector") {
        RegistryCheckedFieldValue::RecordVector(records) => u64::from(records.len()),
        _ => panic!("field tag {tag} must be a record vector"),
    }
}

fn canonical_structure_counts(view: ValueCheckedObjectView<'_>) -> CanonicalStructureCounts {
    let registry = view.registry_view();
    let relation_section = registry.section(3).expect("canonical relation section");
    let canonical_relation_rows = relation_section
        .tables()
        .map(|table| u64::from(table.row_count()))
        .sum();

    let spatial_section = registry.section(4).expect("canonical spatial section");
    let lane_geometry = spatial_section.table(1).expect("lane geometry table");
    let facility_geometry = spatial_section.table(2).expect("facility geometry table");
    let conflict_regions = spatial_section.table(3).expect("conflict region table");

    CanonicalStructureCounts {
        canonical_relation_rows,
        lane_geometry_rows: lane_geometry.row_count(),
        lane_geometry_points: lane_geometry
            .rows()
            .map(|row| record_vector_len(row, 4))
            .sum(),
        lane_geometry_segments: lane_geometry
            .rows()
            .map(|row| record_vector_len(row, 5))
            .sum(),
        facility_geometry_rows: facility_geometry.row_count(),
        facility_geometry_points: facility_geometry
            .rows()
            .map(|row| record_vector_len(row, 3))
            .sum(),
        conflict_region_rows: conflict_regions.row_count(),
        conflict_region_ring_points: conflict_regions
            .rows()
            .map(|row| record_vector_len(row, 5))
            .sum(),
    }
}

fn required_shared_network_scratch_bytes(
    input: &CheckedCanonicalNetworkInput<ImmutableObjectSource>,
    spatial: SpatialBuildOption,
) -> u64 {
    let limits = SharedNetworkBuildLimits::new(SHARED_BUILD_LIMITS.max_retained_bytes(), 0);
    match build_shared_network_revision(
        input.clone(),
        SharedNetworkBuildOptions::new(spatial, limits),
    ) {
        Err(BuildError::BudgetExceeded {
            structure: BuildStructure::BuilderScratch,
            required,
            limit: 0,
        }) => required,
        Err(error) => panic!("zero-scratch probe failed before the scratch budget: {error:?}"),
        Ok(_) => panic!("resource workload unexpectedly requires zero builder scratch bytes"),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ChunkSummary {
    count: u64,
    max_rows: u32,
    max_exact_bytes: u64,
}

fn chunk_summary(view: ValueCheckedObjectView<'_>) -> ChunkSummary {
    let mut summary = ChunkSummary {
        count: 0,
        max_rows: 0,
        max_exact_bytes: 0,
    };
    for section in view.registry_view().sections() {
        for table in section.tables() {
            summary.count += u64::from(table.chunk_count());
            for chunk in 0..table.chunk_count() {
                summary.max_rows = summary
                    .max_rows
                    .max(table.chunk_row_count(chunk).expect("checked chunk"));
                summary.max_exact_bytes = summary
                    .max_exact_bytes
                    .max(table.chunk_exact_byte_length(chunk).expect("checked chunk"));
            }
        }
    }
    summary
}

fn print_object(
    shape: WorkloadShape,
    object: PortableObjectKind,
    exact_bytes: u64,
    digest: Sha256Digest,
    chunks: ChunkSummary,
) {
    println!(
        "portable-resource object={object:?} stable_entities={} exact_bytes={exact_bytes} digest={digest:x} chunks={} max_chunk_rows={} max_chunk_exact_bytes={}",
        shape.stable_entities, chunks.count, chunks.max_rows, chunks.max_exact_bytes,
    );
}

fn run_resource_tier(stable_entity_count: u32) {
    let provenance = PortableEmissionProvenance::try_new("laneflow-resource-probe-v1").unwrap();
    assert!(RESOURCE_TIERS.contains(&stable_entity_count));
    let shape = WorkloadShape::for_stable_entities(stable_entity_count);
    let (unit, source_measurement) = measure_with_heap_peak(|| build_mixed_network(shape, false));
    print_measurement("source-build", shape, source_measurement, 0, 0, 0);
    let (output, compile_measurement) = measure_with_heap_peak(|| {
        Compiler::new()
            .compile(unit)
            .unwrap_or_else(|diagnostics| panic!("compiled output: {diagnostics}"))
    });
    print_measurement("compile", shape, compile_measurement, 0, 0, 0);
    println!(
        "portable-resource compiler-metrics stable_entities={} lir_records={} output_logical_bytes={} compiler_controlled_peak_bytes={} semantic_fingerprint={:02x?}",
        shape.stable_entities,
        output.metrics().lir_record_count(),
        output.metrics().output_logical_bytes(),
        output.metrics().compiler_controlled_peak_bytes(),
        output.metrics().semantic_fingerprint(),
    );
    let semantic_fingerprint = output.metrics().semantic_fingerprint();

    let staging_directory = std::env::temp_dir().join(format!(
        "laneflow-portable-resource-{}-{}",
        std::process::id(),
        NEXT_STAGING_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&staging_directory).unwrap();
    let (candidate, emit_measurement) = measure_with_heap_peak(|| {
        emit_portable_candidate_to_staging(
            &output,
            &provenance,
            FormatLimits::HARD,
            PortableDiffBase::Genesis,
            &staging_directory,
        )
        .expect("file-backed candidate")
    });
    assert!(candidate.canonical_artifact().is_file_backed());
    assert!(candidate.source_map().is_file_backed());
    assert!(candidate.semantic_diff().is_file_backed());
    let live_entries = fs::read_dir(&staging_directory).unwrap().count();
    #[cfg(windows)]
    assert_eq!(live_entries, 3);
    #[cfg(not(windows))]
    assert_eq!(live_entries, 0);
    let lfca_exact_bytes = candidate.canonical_artifact().byte_length().get();
    let lfsm_exact_bytes = candidate.source_map().byte_length().get();
    let lfsd_exact_bytes = candidate.semantic_diff().byte_length().get();
    let portable_bundle_exact_bytes = lfca_exact_bytes + lfsm_exact_bytes + lfsd_exact_bytes;
    assert!(emit_measurement.live_delta_bytes() >= 0);
    assert!(
        emit_measurement.positive_live_delta_bytes()
            <= MAX_RETURNED_FILE_BACKED_CANDIDATE_HEAP_BYTES,
        "the returned file-backed candidate retained more than the explicit handle-only heap bound"
    );
    assert!(
        emit_measurement.positive_live_delta_bytes()
            < lfca_exact_bytes.min(lfsm_exact_bytes).min(lfsd_exact_bytes),
        "the returned file-backed candidate retained at least one complete portable object"
    );
    print_measurement(
        "file-backed-emit",
        shape,
        emit_measurement,
        portable_bundle_exact_bytes,
        0,
        0,
    );
    drop(output);

    if shape.stable_entities == RESOURCE_TIERS[0] {
        let (permuted_unit, permutation_source_measurement) =
            measure_with_heap_peak(|| build_mixed_network(shape, true));
        print_measurement(
            "permutation-source-build",
            shape,
            permutation_source_measurement,
            0,
            0,
            0,
        );
        let (permuted_output, permutation_compile_measurement) = measure_with_heap_peak(|| {
            Compiler::new()
                .compile(permuted_unit)
                .unwrap_or_else(|diagnostics| panic!("permuted compiled output: {diagnostics}"))
        });
        assert_eq!(
            permuted_output.metrics().semantic_fingerprint(),
            semantic_fingerprint
        );
        print_measurement(
            "permutation-compile",
            shape,
            permutation_compile_measurement,
            0,
            0,
            0,
        );
        let (permuted_candidate, permutation_emit_measurement) = measure_with_heap_peak(|| {
            emit_portable_candidate_to_staging(
                &permuted_output,
                &provenance,
                FormatLimits::HARD,
                PortableDiffBase::Genesis,
                &staging_directory,
            )
            .expect("permuted file-backed candidate")
        });
        drop(permuted_output);
        assert!(permutation_emit_measurement.live_delta_bytes() >= 0);
        assert!(
            permutation_emit_measurement.positive_live_delta_bytes()
                <= MAX_RETURNED_FILE_BACKED_CANDIDATE_HEAP_BYTES
        );
        print_measurement(
            "permutation-file-backed-emit",
            shape,
            permutation_emit_measurement,
            portable_bundle_exact_bytes,
            0,
            0,
        );
        assert_eq!(
            permuted_candidate.network_revision(),
            candidate.network_revision()
        );
        assert_ne!(
            permuted_candidate.source_collection_digest(),
            candidate.source_collection_digest(),
            "reordered official source bytes must change their provenance binding"
        );
        assert_eq!(
            permuted_candidate.canonical_artifact().byte_length(),
            candidate.canonical_artifact().byte_length()
        );
        assert_eq!(
            permuted_candidate.source_map().byte_length(),
            candidate.source_map().byte_length()
        );
        assert_eq!(
            permuted_candidate.semantic_diff().byte_length(),
            candidate.semantic_diff().byte_length()
        );
        assert_ne!(
            permuted_candidate.canonical_artifact().digest(),
            candidate.canonical_artifact().digest()
        );
        assert_ne!(
            permuted_candidate.source_map().digest(),
            candidate.source_map().digest()
        );
        assert_ne!(
            permuted_candidate.semantic_diff().digest(),
            candidate.semantic_diff().digest()
        );
        let (permuted_checked, permutation_check_measurement) = measure(|| {
            check_portable_candidate(permuted_candidate, FormatLimits::HARD)
                .expect("checked permuted bundle")
        });
        assert_eq!(permutation_check_measurement.stats.allocations, 0);
        assert_eq!(permutation_check_measurement.stats.reallocations, 0);
        assert_eq!(permutation_check_measurement.stats.bytes_allocated, 0);
        assert_eq!(
            permuted_checked.network_revision(),
            candidate.network_revision()
        );
        print_measurement(
            "permutation-post-emission-check",
            shape,
            permutation_check_measurement,
            portable_bundle_exact_bytes,
            0,
            0,
        );
        println!(
            "portable-resource declaration-permutation stable_entities={} network_revision_equal=true source_binding_changed=true primary_lfca_digest={:x} permuted_lfca_digest={:x} primary_lfsm_digest={:x} permuted_lfsm_digest={:x} primary_lfsd_digest={:x} permuted_lfsd_digest={:x}",
            shape.stable_entities,
            candidate.canonical_artifact().digest(),
            permuted_checked.canonical_artifact_digest(),
            candidate.source_map().digest(),
            permuted_checked.source_map_digest(),
            candidate.semantic_diff().digest(),
            permuted_checked.semantic_diff_digest(),
        );
        drop(permuted_checked);
    } else {
        println!(
            "portable-resource declaration-permutation stable_entities={} evidence=covered-by-10k-and-reorder-equivalent-fixed-vector",
            shape.stable_entities,
        );
    }

    let (checked, check_measurement) = measure(|| {
        check_portable_candidate(candidate, FormatLimits::HARD).expect("checked bundle")
    });
    assert_eq!(check_measurement.stats.allocations, 0);
    assert_eq!(check_measurement.stats.reallocations, 0);
    assert_eq!(check_measurement.stats.bytes_allocated, 0);
    print_measurement(
        "post-emission-check",
        shape,
        check_measurement,
        portable_bundle_exact_bytes,
        0,
        0,
    );
    let structure_counts = canonical_structure_counts(checked.canonical_artifact_view());
    print_object(
        shape,
        PortableObjectKind::CanonicalArtifact,
        lfca_exact_bytes,
        checked.canonical_artifact_digest(),
        chunk_summary(checked.canonical_artifact_view()),
    );
    print_object(
        shape,
        PortableObjectKind::SourceMap,
        lfsm_exact_bytes,
        checked.source_map_digest(),
        chunk_summary(checked.source_map_view()),
    );
    print_object(
        shape,
        PortableObjectKind::SemanticDiff,
        lfsd_exact_bytes,
        checked.semantic_diff_digest(),
        chunk_summary(checked.semantic_diff_view()),
    );

    let canonical_input = checked.canonical_network_input();
    let narrowed_entries = fs::read_dir(&staging_directory).unwrap().count();
    #[cfg(windows)]
    assert_eq!(narrowed_entries, 1);
    #[cfg(not(windows))]
    assert_eq!(narrowed_entries, 0);
    let mut full_revision = None;
    for spatial in [
        SpatialBuildOption::Omit,
        SpatialBuildOption::RetainAvailable,
    ] {
        let required_scratch_bytes =
            required_shared_network_scratch_bytes(&canonical_input, spatial);
        assert!(required_scratch_bytes <= SHARED_BUILD_LIMITS.max_scratch_bytes());
        let (revision, build_measurement) = measure_with_heap_peak(|| {
            build_shared_network_revision(
                canonical_input.clone(),
                SharedNetworkBuildOptions::new(spatial, SHARED_BUILD_LIMITS),
            )
            .expect("shared network revision")
        });
        let retained_logical_bytes = revision.retained_logical_bytes();
        assert!(
            retained_logical_bytes <= SHARED_BUILD_LIMITS.max_retained_bytes(),
            "shared network retained logical bytes exceeded the explicit build budget"
        );
        assert!(
            build_measurement.positive_live_delta_bytes()
                <= retained_logical_bytes
                    .checked_add(MAX_SHARED_BUILD_ACCOUNTING_OVERHEAD_BYTES)
                    .expect("retained accounting bound fits u64"),
            "allocator live delta exceeded retained logical bytes plus the explicit accounting allowance"
        );
        print_measurement(
            match spatial {
                SpatialBuildOption::Omit => "shared-network-build-headless",
                SpatialBuildOption::RetainAvailable => "shared-network-build-spatial",
            },
            shape,
            build_measurement,
            portable_bundle_exact_bytes,
            retained_logical_bytes,
            required_scratch_bytes,
        );
        if spatial == SpatialBuildOption::RetainAvailable {
            full_revision = Some(revision);
        }
    }

    let revision = full_revision.expect("full spatial revision");
    let actual_stable_entities = EntityKind::ALL
        .iter()
        .map(|kind| revision.traffic().entity_counts().count(*kind))
        .sum::<u32>();
    assert_eq!(actual_stable_entities, shape.stable_entities);
    assert!(
        revision
            .traffic()
            .entity_counts()
            .count(EntityKind::ConflictZone)
            > 0
    );
    assert!(
        revision
            .traffic()
            .entity_counts()
            .count(EntityKind::ParticipantStream)
            > 0
    );
    let count = |kind| revision.traffic().entity_counts().count(kind);
    println!(
        "portable-resource composition stable_entities={} road_corridors={} road_sections={} authoring_lanes={} lane_edges={} junctions={} movements={} maneuver_paths={} maneuver_gates={} waiting_zones={} stop_lines={} signal_groups={} signal_controllers={} signal_phases={} parking_facilities={} parking_spaces={} lane_groups={} facility_bands={} participant_classes={} access_rules={} vehicle_profiles={} conflict_zones={} canonical_frames={} participant_streams={} canonical_relation_rows={} lane_geometry_rows={} lane_geometry_points={} lane_geometry_segments={} facility_geometry_rows={} facility_geometry_points={} conflict_region_rows={} conflict_region_ring_points={} network_revision={:x}",
        actual_stable_entities,
        count(EntityKind::RoadCorridor),
        count(EntityKind::RoadSection),
        count(EntityKind::AuthoringLane),
        count(EntityKind::LaneEdge),
        count(EntityKind::Junction),
        count(EntityKind::Movement),
        count(EntityKind::ManeuverPath),
        count(EntityKind::ManeuverGate),
        count(EntityKind::WaitingZone),
        count(EntityKind::StopLine),
        count(EntityKind::SignalGroup),
        count(EntityKind::SignalController),
        count(EntityKind::SignalPhase),
        count(EntityKind::ParkingFacility),
        count(EntityKind::ParkingSpace),
        count(EntityKind::LaneGroup),
        count(EntityKind::FacilityBand),
        count(EntityKind::ParticipantClass),
        count(EntityKind::AccessRule),
        count(EntityKind::VehicleProfile),
        count(EntityKind::ConflictZone),
        count(EntityKind::CanonicalFrame),
        count(EntityKind::ParticipantStream),
        structure_counts.canonical_relation_rows,
        structure_counts.lane_geometry_rows,
        structure_counts.lane_geometry_points,
        structure_counts.lane_geometry_segments,
        structure_counts.facility_geometry_rows,
        structure_counts.facility_geometry_points,
        structure_counts.conflict_region_rows,
        structure_counts.conflict_region_ring_points,
        canonical_input.network_revision().into_digest(),
    );

    drop(revision);
    drop(canonical_input);
    fs::remove_dir(staging_directory).unwrap();
}

#[test]
fn file_backed_mixed_10k_daily_resource_regression() {
    run_resource_tier(10_000);
}

#[test]
#[ignore = "manual single-thread release 100k mixed file-backed capacity evidence"]
fn file_backed_mixed_100k_resource_evidence() {
    run_resource_tier(100_000);
}

#[test]
#[ignore = "manual single-thread release 1m mixed file-backed capacity evidence"]
fn file_backed_mixed_1m_resource_evidence() {
    run_resource_tier(1_000_000);
}
