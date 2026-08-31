use std::sync::Arc;

use laneflow_compiler::road_editing as lfre;
use laneflow_compiler::{
    AccessRuleInput, AccessRuleTargetInput, CompilationUnitBuilder, CompileLimits, Compiler,
    DiagnosticCode, GeometryAccuracyProfile, GeometryDirectionProfile, IidmVehicleProfileInput,
    JunctionInput, JunctionReference, LaneEdgeInput, LaneEdgeReference, ManeuverGateInput,
    ManeuverPathInput, ManeuverPathReference, MovementInput, MovementReference,
    ParkingFacilityInput, ParkingFacilityReference, ParkingLaneAnchorInput,
    ParkingSpaceGeometryInput, ParkingSpaceInput, ParticipantClassInput, ParticipantClassReference,
    PortableDiffBase, PortableEmissionProvenance, SignalControlInput, SignalControllerInput,
    SignalGroupInput, SignalGroupReference, SignalGroupStateInput, SignalPhaseInput,
    SourceModuleHeader, SourceModuleHeaderInput, StopLineInput, StopLineReference,
    SyntheticModuleBuilder, VehicleProfileInput, derive_canonical_stable_id_v1,
    emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_runtime::{
    InstallError, LeaveParkingTarget, ParkedVehicleSpawnInput, ParkingBinding, ParkingError,
    ParkingTarget, PoseSource, RebindParkingTarget, ReserveParkingTarget, RouteHandle,
    RouteRegisterInput, SnapshotRestoreLimits, SpawnError, TickInput, TrafficWorld,
    VehicleSpawnInput, VehicleStatus, VirtualEntryAnchorSelector, VirtualExitAnchorSelector,
    WorldConfig, deterministic_state_digest, encode_lfrs, restore_lfrs,
};
use laneflow_static_contract::{
    AccessEffect, ConflictZoneOrdinal, EntityKind, LaneEdgeId, ParkingFacilityOrdinal,
    ParkingSpaceOrdinal, ParticipantStreamOrdinal, SignalAspect, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    ConflictPathAnchor, SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision,
    SpatialBuildOption, build_shared_network_revision,
};

fn install_fixture(
    revision: std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
    config: laneflow_runtime::WorldConfig,
) -> Result<laneflow_runtime::TrafficWorld, laneflow_runtime::InstallError> {
    let origin = *revision.canonical_origin();
    laneflow_runtime::TrafficWorld::install(
        revision,
        config,
        laneflow_runtime::CommittedNetworkSource::Published {
            reference: laneflow_runtime::PublishedLfcaReference::new(
                "fixture://in-process",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("non-empty fixture key"),
        },
        0,
    )
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

fn compile_road_editing_revision(
    module: lfre::RoadEditingSourceModule,
) -> Arc<SharedNetworkRevision> {
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
    let output = Compiler::new()
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
        });
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

fn road_editing_line(start: (f64, f64), end: (f64, f64)) -> lfre::RoadEditingCurveProgram {
    lfre::RoadEditingCurveProgram::try_new(
        lfre::RoadEditingPoint3::try_new(start.0, 0.0, start.1).expect("curve start"),
        vec![lfre::RoadEditingCurveSegment::line(
            lfre::RoadEditingPoint3::try_new(end.0, 0.0, end.1).expect("curve end"),
        )],
    )
    .expect("line curve")
}

fn add_road_editing_approach(
    module: &mut lfre::RoadEditingSourceModuleBuilder<'_>,
    edge_key: &str,
    start: (f64, f64),
    end: (f64, f64),
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
                road_editing_line(start, end),
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
            lfre::LaneEdgeInput::try_new(edge_key, 13.0, Vec::new(), None)
                .expect("approach lane edge"),
        ))
        .expect("add approach lane edge");
}

fn conflict_road_editing_module() -> lfre::RoadEditingSourceModule {
    conflict_road_editing_module_with_stream_count(2)
}

fn conflict_road_editing_module_with_stream_count(
    stream_count: usize,
) -> lfre::RoadEditingSourceModule {
    assert!(stream_count <= 2);
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
        add_road_editing_approach(&mut module, edge, start, end);
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
        .expect("add junction")
        .add_declaration(lfre::RoadEditingDeclaration::ConflictZone(
            lfre::ConflictZoneInput::try_new("center-zone", junction.clone())
                .expect("conflict zone"),
        ))
        .expect("add conflict zone");

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
        if stream_index < stream_count {
            module
                .add_declaration(lfre::RoadEditingDeclaration::ParticipantStream(
                    lfre::ParticipantStreamInput::try_new(
                        stream_key,
                        junction.clone(),
                        path,
                        vec![lfre::ConflictPassageInput::new(
                            zone.clone(),
                            lfre::PathAnchorInput::interior(1, entry_progress)
                                .expect("entry anchor"),
                            lfre::PathAnchorInput::interior(1, exit_progress).expect("exit anchor"),
                        )],
                    )
                    .expect("participant stream"),
                ))
                .expect("add participant stream");
        }
    }

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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 100)).expect("install");
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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 100)).expect("install");
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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 100)).expect("install");
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
        install_fixture(revision, WorldConfig::new(16, 8, 1_024, 1, 100)).expect("install");
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
        WorldConfig::new(4, 4, 1_024, 1, 100),
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
        install_fixture(revision, WorldConfig::new(4, 4, 1_024, 1, 100)).expect("install");
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
fn virtual_reserved_and_occupied_bindings_round_trip_in_snapshot_v2() {
    let revision = compile_virtual_parking_revision(2);
    let mut world = install_fixture(Arc::clone(&revision), WorldConfig::new(8, 4, 1_024, 1, 100))
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

    assert_eq!(laneflow_runtime::SNAPSHOT_FORMAT_VERSION, 2);
    assert_eq!(laneflow_runtime::RUNTIME_STATE_VERSION, 2);
    assert_eq!(laneflow_runtime::RUNTIME_STATE_DIGEST_VERSION, 4);
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

    let mut cross_world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(8, 4, 1_024, 1, 100))
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

    let mut repeated_world = install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 100))
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
        install_fixture(revision, WorldConfig::new(8, 8, 1_024, 1, 100)).expect("install");
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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 100)).expect("install");
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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 1_000)).expect("install");
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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 1_000)).expect("install");
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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 100)).expect("install");
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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 1_000)).expect("install");
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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 1_000)).expect("install");
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
}

#[test]
fn install_rejects_phase_shorter_than_tick() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        add_signalized_corridor(module, 8);
    });
    assert_eq!(
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 16))
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
        install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 4)).expect("install");
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
