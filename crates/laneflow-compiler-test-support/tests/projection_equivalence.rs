//! 编译器原生模块经 Canonical LIR 投影到当前 Core/Spatial 的集成契约。
//!
//! 本测试不读取 current JSON，也不维护旧 wire format 到编译器输入的转换器。

use std::collections::BTreeMap;

use laneflow_compiler::{
    AccessEffect, AccessRegulationInput, AccessRuleInput, AccessRuleTargetInput,
    AuthoringLaneInput, CanonicalFrameInput, CanonicalPoint3F32Input, CompilationOutput,
    CompilationUnitBuilder, CompileLimits, Compiler, CorridorElementReference, FacilityBandInput,
    FacilityBandReference, IidmVehicleProfileInput, JunctionInput, JunctionReference,
    LaneEdgeGeometryInput, LaneEdgeInput, LaneEdgeReference, LaneGroupInput, LaneGroupReference,
    ManeuverGateInput, ManeuverGateReference, ManeuverPathInput, ManeuverPathReference,
    MovementInput, ParkingAreaInput, ParkingAreaReference, ParkingLaneAnchorInput,
    ParkingSpaceGeometryInput, ParkingSpaceInput, ParticipantClassInput, ParticipantClassReference,
    RoadCorridorInput, RoadSectionInput, RoadSectionReference, SignalAspect, SignalControlInput,
    SignalControllerInput, SignalGroupInput, SignalGroupReference, SignalGroupStateInput,
    SignalPhaseInput, SourceModuleHeader, SourceModuleHeaderInput, StaticRouteInput, StopLineInput,
    StopLineReference, SyntheticModuleBuilder, VehicleProfileInput, WaitingZoneInput,
};
use laneflow_compiler_test_support::project;
use laneflow_core::{
    AccessCell, AccessEffect as CoreAccessEffect, AccessTargetId, CorridorElement, EdgeProgress,
    SignalAspect as CoreSignalAspect, SignalControl as CoreSignalControl,
};
use laneflow_static_contract::{EntityKind, FieldTag};

const NAMESPACE: &str = "fixture/projection";

#[test]
fn compiler_native_fixture_projects_complete_representative_contract() {
    let output = compile_fixture();
    let projection = project(output.lir()).expect("代表性 LIR 必须可投影");
    assert_complete_representative_projection(&output, &projection);
}

#[test]
fn repeated_compilation_and_projection_are_deterministic() {
    let first = compile_fixture();
    let second = compile_fixture();
    assert_eq!(
        first.metrics().semantic_fingerprint(),
        second.metrics().semantic_fingerprint()
    );

    let first_projection = project(first.lir()).expect("首次投影");
    let second_projection = project(second.lir()).expect("重复投影");
    assert_eq!(first_projection.mappings(), second_projection.mappings());
    // 两次结果分别核对同一套完整有类型预期，避免恢复会掩盖字段意义的字符串快照。
    assert_complete_representative_projection(&first, &first_projection);
    assert_complete_representative_projection(&second, &second_projection);
}

fn assert_complete_representative_projection(
    output: &CompilationOutput,
    projection: &laneflow_compiler_test_support::CurrentProjection,
) {
    let lir = output.lir();
    let ids = stable_ids_by_source_key(lir);
    assert_eq!(ids.len(), 54);

    assert_entity_counts(
        lir,
        [
            7, 2, 2, 2, 2, 2, 2, 2, 2, 4, 4, 2, 3, 2, 4, 2, 2, 3, 2, 2, 2, 1,
        ],
    );
    assert_eq!(projection.traffic().lane_graph().edges().len(), 7);
    assert_eq!(projection.traffic().routes().len(), 2);
    assert_eq!(projection.traffic().signals().controllers().len(), 2);
    assert_eq!(projection.traffic().signals().maneuver_gates().len(), 4);
    assert_eq!(projection.traffic().waiting().waiting_zones().len(), 2);
    assert_eq!(projection.traffic().parking().areas().len(), 2);
    assert_eq!(projection.traffic().parking().spaces().len(), 2);
    assert_projection_mappings(projection, lir);
    assert_projection_semantics(projection, &ids);

    for (route_key, gate_count, waiting_zone_count) in [("route", 3, 2), ("aux-route", 1, 0)] {
        let route_id = stable_id_for(route_key, &ids);
        let route = lir
            .static_routes()
            .find(|route| route.stable_id().to_string() == route_id)
            .expect("静态路线必须存在");
        assert_eq!(route.gate_occurrences().len(), gate_count);
        assert_eq!(route.waiting_zone_occurrences().len(), waiting_zone_count);
    }

    let spatial = projection
        .spatial()
        .expect("完整几何必须生成 SpatialRegistry");
    assert_eq!(spatial.len(), 7);
    for (edge_key, progress, expected_position, expected_tangent, expected_up) in [
        (
            "entry",
            2.5,
            [1.5, 0.0, 2.0],
            [0.6, 0.0, 0.8],
            [0.0, 1.0, 0.0],
        ),
        (
            "entry",
            7.5,
            [3.0, 2.0, 5.5],
            [0.0, 0.8, 0.6],
            [0.0, 0.6, -0.8],
        ),
        (
            "internal-a",
            3.0,
            [6.0, 4.0, 7.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        ),
        (
            "internal-b",
            3.5,
            [12.5, 4.0, 7.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        ),
        (
            "exit",
            5.0,
            [21.0, 4.0, 7.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        ),
        (
            "aux-entry",
            4.0,
            [4.0, 10.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        ),
        (
            "aux-internal",
            2.5,
            [10.5, 10.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        ),
        (
            "aux-exit",
            4.5,
            [17.5, 10.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        ),
    ] {
        let edge_id = stable_id_for(edge_key, &ids);
        let handle = projection
            .traffic()
            .lane_graph()
            .edge_handle(edge_id)
            .expect("投影边必须存在");
        let pose = spatial
            .sample(handle, EdgeProgress::try_new(progress).unwrap())
            .expect("几何采样必须成功");
        assert_eq!(
            [
                pose.position().x(),
                pose.position().y(),
                pose.position().z(),
            ],
            expected_position
        );
        assert_eq!(
            [pose.tangent().x(), pose.tangent().y(), pose.tangent().z()],
            expected_tangent
        );
        assert_eq!([pose.up().x(), pose.up().y(), pose.up().z()], expected_up);
    }

    assert_vehicle_profile(
        projection,
        &ids,
        "passenger-car",
        "car",
        [4.5, 10.0, 2.0, 1.5, 1.5, 2.0, 6.0],
    );
    assert_vehicle_profile(
        projection,
        &ids,
        "city-bus",
        "bus",
        [12.0, 8.0, 3.0, 2.5, 1.0, 2.5, 7.0],
    );
}

fn assert_vehicle_profile(
    projection: &laneflow_compiler_test_support::CurrentProjection,
    ids: &BTreeMap<String, String>,
    profile_key: &str,
    class_key: &str,
    expected_iidm: [f64; 7],
) {
    let profile_id = stable_id_for(profile_key, ids);
    let profile_handle = projection
        .traffic()
        .vehicle_profiles()
        .profile_handle(profile_id)
        .expect("车辆配置必须存在");
    let profile = projection
        .traffic()
        .vehicle_profiles()
        .profile(profile_handle)
        .expect("车辆配置句柄必须有效");
    assert_eq!(
        profile.participant_class(),
        projection
            .traffic()
            .participant_classes()
            .class_handle(stable_id_for(class_key, ids))
            .expect("车辆类别必须存在")
    );
    assert_eq!(
        [
            profile.iidm().length,
            profile.iidm().desired_speed,
            profile.iidm().min_gap,
            profile.iidm().time_headway,
            profile.iidm().max_acceleration,
            profile.iidm().comfortable_deceleration,
            profile.iidm().emergency_deceleration,
        ],
        expected_iidm
    );
}

fn compile_fixture() -> CompilationOutput {
    let limits = CompileLimits::p100_initial_v1();
    let mut module = SyntheticModuleBuilder::new(header(&limits), &limits).unwrap();

    module
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry",
            length_meters: 10.0,
            speed_limit_meters_per_second: 13.0,
            successors: &[LaneEdgeReference::local("internal-a")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "internal-a",
            length_meters: 6.0,
            speed_limit_meters_per_second: 11.0,
            successors: &[LaneEdgeReference::local("internal-b")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "internal-b",
            length_meters: 7.0,
            speed_limit_meters_per_second: 9.0,
            successors: &[LaneEdgeReference::local("exit")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit",
            length_meters: 10.0,
            speed_limit_meters_per_second: 7.0,
            successors: &[],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "aux-entry",
            length_meters: 8.0,
            speed_limit_meters_per_second: 15.0,
            successors: &[LaneEdgeReference::local("aux-internal")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "aux-internal",
            length_meters: 5.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[LaneEdgeReference::local("aux-exit")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "aux-exit",
            length_meters: 9.0,
            speed_limit_meters_per_second: 6.0,
            successors: &[],
        })
        .unwrap()
        .add_facility_band(FacilityBandInput {
            facility_band_key: "median",
            kind_id: "median",
        })
        .unwrap()
        .add_lane_group(LaneGroupInput {
            lane_group_key: "main-group",
            road_section: RoadSectionReference::local("main-section"),
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "main-section",
            kind_id: "motorLane",
            lanes: &[AuthoringLaneInput {
                authoring_lane_key: "main-lane",
                edge_chain: &[
                    LaneEdgeReference::local("entry"),
                    LaneEdgeReference::local("internal-a"),
                    LaneEdgeReference::local("internal-b"),
                    LaneEdgeReference::local("exit"),
                ],
                lane_group: Some(LaneGroupReference::local("main-group")),
            }],
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "main-corridor",
            reference_section: RoadSectionReference::local("main-section"),
            elements: &[
                CorridorElementReference::road_section(RoadSectionReference::local("main-section")),
                CorridorElementReference::facility_band(FacilityBandReference::local("median")),
            ],
        })
        .unwrap()
        .add_facility_band(FacilityBandInput {
            facility_band_key: "aux-shoulder",
            kind_id: "shoulder",
        })
        .unwrap()
        .add_lane_group(LaneGroupInput {
            lane_group_key: "aux-group",
            road_section: RoadSectionReference::local("aux-section"),
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "aux-section",
            kind_id: "nonMotorLane",
            lanes: &[AuthoringLaneInput {
                authoring_lane_key: "aux-lane",
                edge_chain: &[
                    LaneEdgeReference::local("aux-entry"),
                    LaneEdgeReference::local("aux-internal"),
                    LaneEdgeReference::local("aux-exit"),
                ],
                lane_group: Some(LaneGroupReference::local("aux-group")),
            }],
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "aux-corridor",
            reference_section: RoadSectionReference::local("aux-section"),
            elements: &[
                CorridorElementReference::facility_band(FacilityBandReference::local(
                    "aux-shoulder",
                )),
                CorridorElementReference::road_section(RoadSectionReference::local("aux-section")),
            ],
        })
        .unwrap()
        .add_junction(JunctionInput {
            junction_key: "junction",
        })
        .unwrap()
        .add_movement(MovementInput {
            movement_key: "movement",
            junction: JunctionReference::local("junction"),
            directed_entry_approach_key: "movement/entry",
            directed_exit_approach_key: "movement/exit",
        })
        .unwrap()
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path",
            movement: laneflow_compiler::MovementReference::local("movement"),
            entry_edge: LaneEdgeReference::local("entry"),
            internal_edges: &[
                LaneEdgeReference::local("internal-a"),
                LaneEdgeReference::local("internal-b"),
            ],
            exit_edge: LaneEdgeReference::local("exit"),
        })
        .unwrap()
        .add_junction(JunctionInput {
            junction_key: "aux-junction",
        })
        .unwrap()
        .add_movement(MovementInput {
            movement_key: "aux-movement",
            junction: JunctionReference::local("aux-junction"),
            directed_entry_approach_key: "aux-movement/entry",
            directed_exit_approach_key: "aux-movement/exit",
        })
        .unwrap()
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "aux-path",
            movement: laneflow_compiler::MovementReference::local("aux-movement"),
            entry_edge: LaneEdgeReference::local("aux-entry"),
            internal_edges: &[LaneEdgeReference::local("aux-internal")],
            exit_edge: LaneEdgeReference::local("aux-exit"),
        })
        .unwrap();

    for (stop_line_key, edge_key) in [
        ("stop-entry", "entry"),
        ("stop-middle", "internal-a"),
        ("stop-release", "internal-b"),
        ("aux-stop", "aux-entry"),
    ] {
        module
            .add_stop_line(StopLineInput {
                stop_line_key,
                lane_edge: LaneEdgeReference::local(edge_key),
            })
            .unwrap();
    }

    module
        .add_signal_group(SignalGroupInput {
            signal_group_key: "main-signal",
        })
        .unwrap()
        .add_signal_group(SignalGroupInput {
            signal_group_key: "secondary-signal",
        })
        .unwrap()
        .add_signal_group(SignalGroupInput {
            signal_group_key: "aux-signal",
        })
        .unwrap();
    let green_states = [
        SignalGroupStateInput {
            signal_group: SignalGroupReference::local("main-signal"),
            aspect: SignalAspect::Green,
        },
        SignalGroupStateInput {
            signal_group: SignalGroupReference::local("secondary-signal"),
            aspect: SignalAspect::Red,
        },
    ];
    let yellow_states = [
        SignalGroupStateInput {
            signal_group: SignalGroupReference::local("secondary-signal"),
            aspect: SignalAspect::Green,
        },
        SignalGroupStateInput {
            signal_group: SignalGroupReference::local("main-signal"),
            aspect: SignalAspect::Yellow,
        },
    ];
    let red_states = [
        SignalGroupStateInput {
            signal_group: SignalGroupReference::local("main-signal"),
            aspect: SignalAspect::Red,
        },
        SignalGroupStateInput {
            signal_group: SignalGroupReference::local("secondary-signal"),
            aspect: SignalAspect::Yellow,
        },
    ];
    let phases = [
        SignalPhaseInput {
            signal_phase_key: "green",
            duration_ms: 3_000,
            states: &green_states,
        },
        SignalPhaseInput {
            signal_phase_key: "yellow",
            duration_ms: 1_000,
            states: &yellow_states,
        },
        SignalPhaseInput {
            signal_phase_key: "red",
            duration_ms: 2_000,
            states: &red_states,
        },
    ];
    let aux_states = [SignalGroupStateInput {
        signal_group: SignalGroupReference::local("aux-signal"),
        aspect: SignalAspect::Yellow,
    }];
    let aux_phases = [SignalPhaseInput {
        signal_phase_key: "aux-caution",
        duration_ms: 4_000,
        states: &aux_states,
    }];
    module
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: "controller",
            offset_ms: 1_250,
            signal_groups: &[
                SignalGroupReference::local("main-signal"),
                SignalGroupReference::local("secondary-signal"),
            ],
            phases: &phases,
        })
        .unwrap()
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: "aux-controller",
            offset_ms: 750,
            signal_groups: &[SignalGroupReference::local("aux-signal")],
            phases: &aux_phases,
        })
        .unwrap()
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "aux-gate",
            maneuver_path: ManeuverPathReference::local("aux-path"),
            transition_index: 0,
            stop_line: StopLineReference::local("aux-stop"),
            signal_control: SignalControlInput::Group(SignalGroupReference::local("aux-signal")),
        })
        .unwrap()
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-entry",
            maneuver_path: ManeuverPathReference::local("path"),
            transition_index: 0,
            stop_line: StopLineReference::local("stop-entry"),
            signal_control: SignalControlInput::Group(SignalGroupReference::local("main-signal")),
        })
        .unwrap()
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-middle",
            maneuver_path: ManeuverPathReference::local("path"),
            transition_index: 1,
            stop_line: StopLineReference::local("stop-middle"),
            signal_control: SignalControlInput::Group(SignalGroupReference::local(
                "secondary-signal",
            )),
        })
        .unwrap()
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-release",
            maneuver_path: ManeuverPathReference::local("path"),
            transition_index: 2,
            stop_line: StopLineReference::local("stop-release"),
            signal_control: SignalControlInput::None,
        })
        .unwrap()
        .add_waiting_zone(WaitingZoneInput {
            waiting_zone_key: "zone-a",
            maneuver_path: ManeuverPathReference::local("path"),
            entry_gate: ManeuverGateReference::local("gate-entry"),
            release_gate: ManeuverGateReference::local("gate-middle"),
            max_occupancy: 2,
        })
        .unwrap()
        .add_waiting_zone(WaitingZoneInput {
            waiting_zone_key: "zone-b",
            maneuver_path: ManeuverPathReference::local("path"),
            entry_gate: ManeuverGateReference::local("gate-middle"),
            release_gate: ManeuverGateReference::local("gate-release"),
            max_occupancy: 1,
        })
        .unwrap()
        .add_parking_area(ParkingAreaInput {
            parking_area_key: "parking-area",
        })
        .unwrap()
        .add_parking_space(ParkingSpaceInput {
            parking_space_key: "parking-space",
            parking_area: Some(ParkingAreaReference::local("parking-area")),
            entry: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("entry"),
                progress_meters: 5.0,
            },
            exit: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("exit"),
                progress_meters: 5.0,
            },
            geometry: ParkingSpaceGeometryInput {
                lateral_offset_meters: 2.5,
                heading_offset_radians: 0.0,
                length_meters: 5.0,
                width_meters: 2.4,
            },
        })
        .unwrap()
        .add_parking_area(ParkingAreaInput {
            parking_area_key: "aux-parking-area",
        })
        .unwrap()
        .add_parking_space(ParkingSpaceInput {
            parking_space_key: "aux-parking-space",
            parking_area: Some(ParkingAreaReference::local("aux-parking-area")),
            entry: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("aux-entry"),
                progress_meters: 4.0,
            },
            exit: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("aux-exit"),
                progress_meters: 4.5,
            },
            geometry: ParkingSpaceGeometryInput {
                lateral_offset_meters: -3.0,
                heading_offset_radians: 0.25,
                length_meters: 12.5,
                width_meters: 3.0,
            },
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "car",
            extends: Some(ParticipantClassReference::local("road-user")),
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "bus",
            extends: Some(ParticipantClassReference::local("road-user")),
        })
        .unwrap()
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "passenger-car",
            participant_class: ParticipantClassReference::local("car"),
            iidm: IidmVehicleProfileInput {
                length_meters: 4.5,
                desired_speed_meters_per_second: 10.0,
                min_gap_meters: 2.0,
                time_headway_seconds: 1.5,
                max_acceleration_meters_per_second_squared: 1.5,
                comfortable_deceleration_meters_per_second_squared: 2.0,
                emergency_deceleration_meters_per_second_squared: 6.0,
            },
        })
        .unwrap()
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "city-bus",
            participant_class: ParticipantClassReference::local("bus"),
            iidm: IidmVehicleProfileInput {
                length_meters: 12.0,
                desired_speed_meters_per_second: 8.0,
                min_gap_meters: 3.0,
                time_headway_seconds: 2.5,
                max_acceleration_meters_per_second_squared: 1.0,
                comfortable_deceleration_meters_per_second_squared: 2.5,
                emergency_deceleration_meters_per_second_squared: 7.0,
            },
        })
        .unwrap()
        .add_access_rule(AccessRuleInput {
            access_rule_key: "car-only",
            target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("entry")),
            effect: AccessEffect::Deny,
            participant_classes: &[ParticipantClassReference::local("car")],
            regulation: Some(AccessRegulationInput {
                jurisdiction: "fixture",
                version: "1",
                source: Some("compiler-native-test"),
            }),
            priority: 10,
        })
        .unwrap()
        .add_access_rule(AccessRuleInput {
            access_rule_key: "main-group-allow",
            target: AccessRuleTargetInput::LaneGroup(LaneGroupReference::local("main-group")),
            effect: AccessEffect::Allow,
            participant_classes: &[
                ParticipantClassReference::local("car"),
                ParticipantClassReference::local("bus"),
            ],
            regulation: None,
            priority: 5,
        })
        .unwrap()
        .add_static_route(StaticRouteInput {
            static_route_key: "route",
            edge_sequence: &[
                LaneEdgeReference::local("entry"),
                LaneEdgeReference::local("internal-a"),
                LaneEdgeReference::local("internal-b"),
                LaneEdgeReference::local("exit"),
            ],
        })
        .unwrap()
        .add_static_route(StaticRouteInput {
            static_route_key: "aux-route",
            edge_sequence: &[
                LaneEdgeReference::local("aux-entry"),
                LaneEdgeReference::local("aux-internal"),
                LaneEdgeReference::local("aux-exit"),
            ],
        })
        .unwrap();

    let entry_points = [
        point(0.0, 0.0, 0.0),
        point(3.0, 0.0, 4.0),
        point(3.0, 4.0, 7.0),
    ];
    let internal_a_points = [point(3.0, 4.0, 7.0), point(9.0, 4.0, 7.0)];
    let internal_b_points = [point(9.0, 4.0, 7.0), point(16.0, 4.0, 7.0)];
    let exit_points = [point(16.0, 4.0, 7.0), point(26.0, 4.0, 7.0)];
    let aux_entry_points = [point(0.0, 10.0, 0.0), point(8.0, 10.0, 0.0)];
    let aux_internal_points = [point(8.0, 10.0, 0.0), point(13.0, 10.0, 0.0)];
    let aux_exit_points = [point(13.0, 10.0, 0.0), point(22.0, 10.0, 0.0)];
    let geometries = [
        geometry("entry", &entry_points),
        geometry("internal-a", &internal_a_points),
        geometry("internal-b", &internal_b_points),
        geometry("exit", &exit_points),
        geometry("aux-entry", &aux_entry_points),
        geometry("aux-internal", &aux_internal_points),
        geometry("aux-exit", &aux_exit_points),
    ];
    module
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame",
            lane_edge_geometries: &geometries,
        })
        .unwrap();

    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().unwrap()).unwrap();
    Compiler::new().compile(unit.build().unwrap()).unwrap()
}

fn header(limits: &CompileLimits) -> SourceModuleHeader {
    SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: NAMESPACE,
            source_document_key: "projection-native.lfsynthetic",
            generator_build_id: "git:projection-native-v1",
            parameters_and_inputs_digest: [0x31; 32],
            frontend_options_digest: [0x32; 32],
            random_seed: None,
            provenance: "repository:laneflow/projection-equivalence",
        },
        limits,
    )
    .unwrap()
}

const fn point(x: f32, y: f32, z: f32) -> CanonicalPoint3F32Input {
    CanonicalPoint3F32Input { x, y, z }
}

const fn geometry<'a>(
    edge_key: &'a str,
    points: &'a [CanonicalPoint3F32Input],
) -> LaneEdgeGeometryInput<'a> {
    LaneEdgeGeometryInput {
        lane_edge: LaneEdgeReference::local(edge_key),
        centerline_points: points,
    }
}

fn stable_ids_by_source_key(
    lir: &laneflow_compiler::ValidatedCanonicalLir,
) -> BTreeMap<String, String> {
    let mut ids = BTreeMap::new();
    macro_rules! add {
        ($iter:expr, $tag:expr) => {
            for view in $iter {
                let key = view
                    .identity_fields()
                    .find(|field| field.tag() == $tag)
                    .map(|field| String::from_utf8(field.value_bytes().to_vec()).unwrap())
                    .unwrap();
                ids.insert(key, view.stable_id().to_string());
            }
        };
    }
    add!(lir.lane_edges(), FieldTag::LaneEdgeKey);
    add!(lir.road_corridors(), FieldTag::CorridorKey);
    add!(lir.road_sections(), FieldTag::SectionKey);
    add!(lir.lane_groups(), FieldTag::LaneGroupKey);
    add!(lir.facility_bands(), FieldTag::FacilityBandKey);
    add!(lir.junctions(), FieldTag::JunctionKey);
    add!(lir.movements(), FieldTag::MovementKey);
    add!(lir.maneuver_paths(), FieldTag::PathKey);
    add!(lir.stop_lines(), FieldTag::StopLineKey);
    add!(lir.maneuver_gates(), FieldTag::GateKey);
    add!(lir.waiting_zones(), FieldTag::WaitingZoneKey);
    add!(lir.signal_groups(), FieldTag::SignalGroupKey);
    add!(lir.signal_controllers(), FieldTag::SignalControllerKey);
    add!(lir.signal_phases(), FieldTag::PhaseKey);
    add!(lir.parking_areas(), FieldTag::ParkingAreaKey);
    add!(lir.parking_spaces(), FieldTag::ParkingSpaceKey);
    add!(lir.participant_classes(), FieldTag::ParticipantClassKey);
    add!(lir.vehicle_profiles(), FieldTag::VehicleProfileKey);
    add!(lir.access_rules(), FieldTag::AccessRuleKey);
    add!(lir.static_routes(), FieldTag::RouteKey);
    add!(lir.canonical_frames(), FieldTag::CanonicalFrameKey);
    ids
}

fn stable_id_for<'a>(source_key: &str, ids: &'a BTreeMap<String, String>) -> &'a str {
    ids.get(source_key).map(String::as_str).unwrap()
}

fn assert_projection_mappings(
    projection: &laneflow_compiler_test_support::CurrentProjection,
    lir: &laneflow_compiler::ValidatedCanonicalLir,
) {
    let mut mappings = projection.mappings().entries().iter();
    macro_rules! assert_table {
        ($kind:expr, $views:expr, $has_external_id:expr) => {
            for view in $views {
                let mapping = mappings.next().expect("映射报告不得遗漏 LIR 实体");
                let stable_id = view.stable_id();
                assert_eq!(mapping.entity_kind(), $kind);
                assert_eq!(mapping.lir_ordinal(), view.ordinal().raw());
                assert_eq!(mapping.stable_id(), stable_id.into_untyped());
                if $has_external_id {
                    let expected_external_id = stable_id.to_string();
                    assert_eq!(
                        mapping.current_external_id(),
                        Some(expected_external_id.as_str())
                    );
                } else {
                    assert_eq!(mapping.current_external_id(), None);
                }
            }
        };
    }

    assert_table!(EntityKind::RoadCorridor, lir.road_corridors(), true);
    assert_table!(EntityKind::RoadSection, lir.road_sections(), true);
    assert_table!(EntityKind::AuthoringLane, lir.authoring_lanes(), false);
    assert_table!(EntityKind::LaneEdge, lir.lane_edges(), true);
    assert_table!(EntityKind::Junction, lir.junctions(), true);
    assert_table!(EntityKind::Movement, lir.movements(), true);
    assert_table!(EntityKind::ManeuverPath, lir.maneuver_paths(), true);
    assert_table!(EntityKind::ManeuverGate, lir.maneuver_gates(), true);
    assert_table!(EntityKind::WaitingZone, lir.waiting_zones(), true);
    assert_table!(EntityKind::StopLine, lir.stop_lines(), true);
    assert_table!(EntityKind::SignalGroup, lir.signal_groups(), true);
    assert_table!(EntityKind::SignalController, lir.signal_controllers(), true);
    assert_table!(EntityKind::SignalPhase, lir.signal_phases(), true);
    assert_table!(EntityKind::ParkingArea, lir.parking_areas(), true);
    assert_table!(EntityKind::ParkingSpace, lir.parking_spaces(), true);
    assert_table!(EntityKind::LaneGroup, lir.lane_groups(), true);
    assert_table!(EntityKind::FacilityBand, lir.facility_bands(), true);
    assert_table!(
        EntityKind::ParticipantClass,
        lir.participant_classes(),
        true
    );
    assert_table!(EntityKind::AccessRule, lir.access_rules(), true);
    assert_table!(EntityKind::VehicleProfile, lir.vehicle_profiles(), true);
    assert_table!(EntityKind::StaticRoute, lir.static_routes(), true);
    assert_table!(EntityKind::CanonicalFrame, lir.canonical_frames(), true);

    assert!(
        mappings.next().is_none(),
        "映射报告不得包含没有对应 LIR 实体的额外记录"
    );
}

fn assert_projection_semantics(
    projection: &laneflow_compiler_test_support::CurrentProjection,
    ids: &BTreeMap<String, String>,
) {
    let traffic = projection.traffic();
    let graph = traffic.lane_graph();
    let edge_handles = ["entry", "internal-a", "internal-b", "exit"].map(|key| {
        graph
            .edge_handle(stable_id_for(key, ids))
            .expect("投影边必须存在")
    });
    let aux_edge_handles = ["aux-entry", "aux-internal", "aux-exit"].map(|key| {
        graph
            .edge_handle(stable_id_for(key, ids))
            .expect("辅助投影边必须存在")
    });
    assert_eq!(
        graph.next_edges(edge_handles[0]),
        Some([edge_handles[1]].as_slice())
    );
    assert_eq!(
        graph.next_edges(edge_handles[1]),
        Some([edge_handles[2]].as_slice())
    );
    assert_eq!(
        graph.next_edges(edge_handles[2]),
        Some([edge_handles[3]].as_slice())
    );
    assert_eq!(graph.next_edges(edge_handles[3]), Some([].as_slice()));
    assert_eq!(
        graph.next_edges(aux_edge_handles[0]),
        Some([aux_edge_handles[1]].as_slice())
    );
    assert_eq!(
        graph.next_edges(aux_edge_handles[1]),
        Some([aux_edge_handles[2]].as_slice())
    );
    assert_eq!(graph.next_edges(aux_edge_handles[2]), Some([].as_slice()));

    for (edge_key, expected_length, expected_speed_limit) in [
        ("entry", 10.0, 13.0),
        ("internal-a", 6.0, 11.0),
        ("internal-b", 7.0, 9.0),
        ("exit", 10.0, 7.0),
        ("aux-entry", 8.0, 15.0),
        ("aux-internal", 5.0, 8.0),
        ("aux-exit", 9.0, 6.0),
    ] {
        let edge_id = stable_id_for(edge_key, ids);
        assert_eq!(
            graph.edge_length_by_id(edge_id).expect("边长度").value(),
            expected_length
        );
        assert_eq!(
            graph
                .edge_speed_limit_by_id(edge_id)
                .expect("边限速")
                .value(),
            expected_speed_limit
        );
    }

    let cross_section = traffic.cross_section();
    let section = cross_section
        .section_handle(stable_id_for("main-section", ids))
        .expect("道路横断面必须存在");
    let group = cross_section
        .group_handle(stable_id_for("main-group", ids))
        .expect("车道组必须存在");
    let band = cross_section
        .band_handle(stable_id_for("median", ids))
        .expect("设施带必须存在");
    let corridor = cross_section
        .corridor_handle(stable_id_for("main-corridor", ids))
        .expect("道路走廊必须存在");
    assert_eq!(
        cross_section.section(section).expect("道路区段").kind_id(),
        "motorLane"
    );
    assert_eq!(
        cross_section.band(band).expect("设施带").kind_id(),
        "median"
    );
    assert_eq!(cross_section.lane_group_section(group), Some(section));
    assert_eq!(
        cross_section
            .group_lanes(group)
            .expect("车道组成员")
            .collect::<Vec<_>>(),
        vec![0]
    );
    let section_lanes = cross_section
        .section_lanes(section)
        .expect("横断面车道")
        .collect::<Vec<_>>();
    assert_eq!(section_lanes.len(), 1);
    assert_eq!(section_lanes[0].0, 0);
    assert_eq!(section_lanes[0].1, edge_handles);
    assert_eq!(
        cross_section.corridor_reference_section(corridor),
        Some(section)
    );
    assert_eq!(
        cross_section.corridor_elements(corridor),
        Some(
            [
                CorridorElement::Section(section),
                CorridorElement::Band(band),
            ]
            .as_slice()
        )
    );
    let aux_section = cross_section
        .section_handle(stable_id_for("aux-section", ids))
        .expect("辅助道路横断面必须存在");
    let aux_group = cross_section
        .group_handle(stable_id_for("aux-group", ids))
        .expect("辅助车道组必须存在");
    let aux_band = cross_section
        .band_handle(stable_id_for("aux-shoulder", ids))
        .expect("辅助设施带必须存在");
    let aux_corridor = cross_section
        .corridor_handle(stable_id_for("aux-corridor", ids))
        .expect("辅助道路走廊必须存在");
    assert_eq!(
        cross_section
            .section(aux_section)
            .expect("辅助道路区段")
            .kind_id(),
        "nonMotorLane"
    );
    assert_eq!(
        cross_section.band(aux_band).expect("辅助设施带").kind_id(),
        "shoulder"
    );
    assert_eq!(
        cross_section.lane_group_section(aux_group),
        Some(aux_section)
    );
    assert_eq!(
        cross_section
            .group_lanes(aux_group)
            .expect("辅助车道组成员")
            .collect::<Vec<_>>(),
        vec![0]
    );
    let aux_section_lanes = cross_section
        .section_lanes(aux_section)
        .expect("辅助横断面车道")
        .collect::<Vec<_>>();
    assert_eq!(aux_section_lanes.len(), 1);
    assert_eq!(aux_section_lanes[0].0, 0);
    assert_eq!(aux_section_lanes[0].1, aux_edge_handles);
    assert_eq!(
        cross_section.corridor_reference_section(aux_corridor),
        Some(aux_section)
    );
    assert_eq!(
        cross_section.corridor_elements(aux_corridor),
        Some(
            [
                CorridorElement::Band(aux_band),
                CorridorElement::Section(aux_section),
            ]
            .as_slice()
        )
    );

    let junctions = traffic.junctions();
    let junction = junctions
        .junction_handle(stable_id_for("junction", ids))
        .expect("路口必须存在");
    let movement = junctions
        .movement_handle(stable_id_for("movement", ids))
        .expect("交通流向必须存在");
    let path = junctions
        .maneuver_path_handle(stable_id_for("path", ids))
        .expect("机动路径必须存在");
    assert_eq!(junctions.movement_junction(movement), Some(junction));
    assert_eq!(junctions.maneuver_path_movement(path), Some(movement));
    assert_eq!(
        junctions.maneuver_path_edges(path),
        Some(edge_handles.as_slice())
    );
    assert_eq!(junctions.internal_edge_owner(edge_handles[0]), None);
    assert_eq!(
        junctions.internal_edge_owner(edge_handles[1]),
        Some(junction)
    );
    assert_eq!(
        junctions.internal_edge_owner(edge_handles[2]),
        Some(junction)
    );
    assert_eq!(junctions.internal_edge_owner(edge_handles[3]), None);
    let aux_junction = junctions
        .junction_handle(stable_id_for("aux-junction", ids))
        .expect("辅助路口必须存在");
    let aux_movement = junctions
        .movement_handle(stable_id_for("aux-movement", ids))
        .expect("辅助交通流向必须存在");
    let aux_path = junctions
        .maneuver_path_handle(stable_id_for("aux-path", ids))
        .expect("辅助机动路径必须存在");
    assert_eq!(
        junctions.movement_junction(aux_movement),
        Some(aux_junction)
    );
    assert_eq!(
        junctions.maneuver_path_movement(aux_path),
        Some(aux_movement)
    );
    assert_eq!(
        junctions.maneuver_path_edges(aux_path),
        Some(aux_edge_handles.as_slice())
    );
    assert_eq!(junctions.internal_edge_owner(aux_edge_handles[0]), None);
    assert_eq!(
        junctions.internal_edge_owner(aux_edge_handles[1]),
        Some(aux_junction)
    );
    assert_eq!(junctions.internal_edge_owner(aux_edge_handles[2]), None);

    let signals = traffic.signals();
    let signal_groups = ["main-signal", "secondary-signal"].map(|key| {
        signals
            .group_handle(stable_id_for(key, ids))
            .expect("信号组必须存在")
    });
    let signal_group = signal_groups[0];
    let secondary_signal_group = signal_groups[1];
    let controller = signals
        .controller_handle(stable_id_for("controller", ids))
        .expect("信号控制器必须存在");
    for group in signal_groups {
        assert_eq!(signals.group_controller(group), Some(controller));
    }
    assert_eq!(
        signals.controller_groups(controller),
        Some(signal_groups.as_slice())
    );
    assert_eq!(
        signals.controller_cycle_duration_ms(controller),
        Some(6_000)
    );
    assert_eq!(
        signals
            .controller(controller)
            .expect("信号控制器")
            .offset_ms(),
        1_250
    );
    for (phase_key, duration_ms, aspects, end_offset_ms) in [
        (
            "green",
            3_000,
            [CoreSignalAspect::Green, CoreSignalAspect::Red],
            3_000,
        ),
        (
            "yellow",
            1_000,
            [CoreSignalAspect::Yellow, CoreSignalAspect::Green],
            4_000,
        ),
        (
            "red",
            2_000,
            [CoreSignalAspect::Red, CoreSignalAspect::Yellow],
            6_000,
        ),
    ] {
        let phase = signals
            .phase_ref(controller, stable_id_for(phase_key, ids))
            .expect("信号相位必须存在");
        assert_eq!(
            signals.phase(phase).expect("信号相位").duration_ms(),
            duration_ms
        );
        assert_eq!(signals.phase_aspects(phase), Some(aspects.as_slice()));
        assert_eq!(signals.phase_end_offset_ms(phase), Some(end_offset_ms));
    }
    let aux_signal_group = signals
        .group_handle(stable_id_for("aux-signal", ids))
        .expect("辅助信号组必须存在");
    let aux_controller = signals
        .controller_handle(stable_id_for("aux-controller", ids))
        .expect("辅助信号控制器必须存在");
    assert_eq!(
        signals.group_controller(aux_signal_group),
        Some(aux_controller)
    );
    assert_eq!(
        signals.controller_groups(aux_controller),
        Some([aux_signal_group].as_slice())
    );
    assert_eq!(
        signals.controller_cycle_duration_ms(aux_controller),
        Some(4_000)
    );
    assert_eq!(
        signals
            .controller(aux_controller)
            .expect("辅助信号控制器")
            .offset_ms(),
        750
    );
    let aux_phase = signals
        .phase_ref(aux_controller, stable_id_for("aux-caution", ids))
        .expect("辅助信号相位必须存在");
    assert_eq!(
        signals
            .phase(aux_phase)
            .expect("辅助信号相位")
            .duration_ms(),
        4_000
    );
    assert_eq!(
        signals.phase_aspects(aux_phase),
        Some([CoreSignalAspect::Yellow].as_slice())
    );
    assert_eq!(signals.phase_end_offset_ms(aux_phase), Some(4_000));

    let stop_lines = ["stop-entry", "stop-middle", "stop-release"].map(|key| {
        signals
            .stop_line_handle(stable_id_for(key, ids))
            .expect("停止线必须存在")
    });
    let gates = ["gate-entry", "gate-middle", "gate-release"].map(|key| {
        signals
            .maneuver_gate_handle(stable_id_for(key, ids))
            .expect("机动门必须存在")
    });
    for index in 0..3 {
        assert_eq!(
            signals.stop_line_edge(stop_lines[index]),
            Some(edge_handles[index])
        );
        assert_eq!(signals.maneuver_gate_path(gates[index]), Some(path));
        assert_eq!(
            signals.maneuver_gate_stop_line(gates[index]),
            Some(stop_lines[index])
        );
        assert_eq!(
            signals
                .maneuver_gate(gates[index])
                .expect("机动门")
                .transition_index(),
            index as u32
        );
    }
    assert_eq!(
        signals.maneuver_gate_control(gates[0]),
        Some(CoreSignalControl::Group(signal_group))
    );
    assert_eq!(
        signals.maneuver_gate_control(gates[1]),
        Some(CoreSignalControl::Group(secondary_signal_group))
    );
    assert_eq!(
        signals.maneuver_gate_control(gates[2]),
        Some(CoreSignalControl::None)
    );
    let aux_stop_line = signals
        .stop_line_handle(stable_id_for("aux-stop", ids))
        .expect("辅助停止线必须存在");
    let aux_gate = signals
        .maneuver_gate_handle(stable_id_for("aux-gate", ids))
        .expect("辅助机动门必须存在");
    assert_eq!(
        signals.stop_line_edge(aux_stop_line),
        Some(aux_edge_handles[0])
    );
    assert_eq!(signals.maneuver_gate_path(aux_gate), Some(aux_path));
    assert_eq!(
        signals.maneuver_gate_stop_line(aux_gate),
        Some(aux_stop_line)
    );
    assert_eq!(
        signals
            .maneuver_gate(aux_gate)
            .expect("辅助机动门")
            .transition_index(),
        0
    );
    assert_eq!(
        signals.maneuver_gate_control(aux_gate),
        Some(CoreSignalControl::Group(aux_signal_group))
    );

    let waiting = traffic.waiting();
    for (zone_key, entry_gate, release_gate, max_occupancy) in [
        ("zone-a", gates[0], gates[1], 2),
        ("zone-b", gates[1], gates[2], 1),
    ] {
        let zone = waiting
            .waiting_zone_handle(stable_id_for(zone_key, ids))
            .expect("等待区必须存在");
        assert_eq!(waiting.waiting_zone_path(zone), Some(path));
        assert_eq!(waiting.waiting_zone_entry_gate(zone), Some(entry_gate));
        assert_eq!(waiting.waiting_zone_release_gate(zone), Some(release_gate));
        assert_eq!(
            waiting.waiting_zone(zone).expect("等待区").max_occupancy(),
            max_occupancy
        );
    }

    let parking = traffic.parking();
    let area = parking
        .area_handle(stable_id_for("parking-area", ids))
        .expect("停车区域必须存在");
    let space = parking
        .space_handle(stable_id_for("parking-space", ids))
        .expect("停车位必须存在");
    assert_eq!(parking.space_area(space), Some(Some(area)));
    assert_eq!(parking.area_spaces(area), Some([space].as_slice()));
    let entry_anchor = parking.space_entry(space).expect("停车位入口");
    let exit_anchor = parking.space_exit(space).expect("停车位出口");
    assert_eq!(entry_anchor.edge(), edge_handles[0]);
    assert_eq!(entry_anchor.progress(), 5.0);
    assert_eq!(exit_anchor.edge(), edge_handles[3]);
    assert_eq!(exit_anchor.progress(), 5.0);
    let geometry = parking.space_geometry(space).expect("停车位几何");
    assert_eq!(geometry.lateral_offset(), 2.5);
    assert_eq!(geometry.heading_offset_radians(), 0.0);
    assert_eq!(geometry.length(), 5.0);
    assert_eq!(geometry.width(), 2.4);
    let aux_area = parking
        .area_handle(stable_id_for("aux-parking-area", ids))
        .expect("辅助停车区域必须存在");
    let aux_space = parking
        .space_handle(stable_id_for("aux-parking-space", ids))
        .expect("辅助停车位必须存在");
    assert_eq!(parking.space_area(aux_space), Some(Some(aux_area)));
    assert_eq!(parking.area_spaces(aux_area), Some([aux_space].as_slice()));
    let aux_entry_anchor = parking.space_entry(aux_space).expect("辅助停车位入口");
    let aux_exit_anchor = parking.space_exit(aux_space).expect("辅助停车位出口");
    assert_eq!(aux_entry_anchor.edge(), aux_edge_handles[0]);
    assert_eq!(aux_entry_anchor.progress(), 4.0);
    assert_eq!(aux_exit_anchor.edge(), aux_edge_handles[2]);
    assert_eq!(aux_exit_anchor.progress(), 4.5);
    let aux_geometry = parking.space_geometry(aux_space).expect("辅助停车位几何");
    assert_eq!(aux_geometry.lateral_offset(), -3.0);
    assert_eq!(aux_geometry.heading_offset_radians(), 0.25);
    assert_eq!(aux_geometry.length(), 12.5);
    assert_eq!(aux_geometry.width(), 3.0);

    let classes = traffic.participant_classes();
    let road_user = classes
        .class_handle(stable_id_for("road-user", ids))
        .expect("道路使用者类别必须存在");
    let car = classes
        .class_handle(stable_id_for("car", ids))
        .expect("车辆类别必须存在");
    let bus = classes
        .class_handle(stable_id_for("bus", ids))
        .expect("公交车类别必须存在");
    assert_eq!(classes.depth(road_user), Some(0));
    assert_eq!(classes.depth(car), Some(1));
    assert_eq!(classes.depth(bus), Some(1));
    assert!(classes.is_descendant_or_self(car, road_user));
    assert!(classes.is_descendant_or_self(bus, road_user));

    let access = traffic.access();
    let rule = access
        .rule_handle(stable_id_for("car-only", ids))
        .expect("准入规则必须存在");
    let definition = access.rule(rule).expect("准入规则");
    assert_eq!(definition.target().id(), stable_id_for("entry", ids));
    assert_eq!(definition.effect(), CoreAccessEffect::Deny);
    assert_eq!(
        definition.participant_class_ids(),
        [stable_id_for("car", ids)]
    );
    assert_eq!(definition.priority(), "10");
    let regulation = definition.regulation().expect("法规来源必须保留");
    assert_eq!(regulation.jurisdiction(), "fixture");
    assert_eq!(regulation.version(), "1");
    assert_eq!(regulation.source(), Some("compiler-native-test"));
    let group_rule = access
        .rule_handle(stable_id_for("main-group-allow", ids))
        .expect("车道组准入规则必须存在");
    let group_definition = access.rule(group_rule).expect("车道组准入规则");
    assert_eq!(
        group_definition.target(),
        &AccessTargetId::lane_group(stable_id_for("main-group", ids))
    );
    assert_eq!(group_definition.effect(), CoreAccessEffect::Allow);
    assert_eq!(group_definition.participant_class_ids().len(), 2);
    assert!(
        group_definition
            .participant_class_ids()
            .iter()
            .any(|class_id| class_id == stable_id_for("car", ids))
    );
    assert!(
        group_definition
            .participant_class_ids()
            .iter()
            .any(|class_id| class_id == stable_id_for("bus", ids))
    );
    assert_eq!(group_definition.priority(), "5");
    assert!(group_definition.regulation().is_none());
    assert_eq!(
        access.edge_access(edge_handles[0], car),
        AccessCell::Decided {
            rule,
            effect: CoreAccessEffect::Deny,
        }
    );
    for edge in &edge_handles[1..] {
        assert_eq!(
            access.edge_access(*edge, car),
            AccessCell::Decided {
                rule: group_rule,
                effect: CoreAccessEffect::Allow,
            }
        );
    }
    for edge in edge_handles {
        assert_eq!(
            access.edge_access(edge, bus),
            AccessCell::Decided {
                rule: group_rule,
                effect: CoreAccessEffect::Allow,
            }
        );
    }

    let route = traffic
        .routes()
        .find(|route| route.id() == stable_id_for("route", ids))
        .expect("静态路线必须存在");
    assert_eq!(route.id(), stable_id_for("route", ids));
    assert!(
        route.edge_ids().iter().map(String::as_str).eq([
            "entry",
            "internal-a",
            "internal-b",
            "exit"
        ]
        .map(|key| stable_id_for(key, ids)))
    );
    let aux_route = traffic
        .routes()
        .find(|route| route.id() == stable_id_for("aux-route", ids))
        .expect("辅助静态路线必须存在");
    assert!(
        aux_route.edge_ids().iter().map(String::as_str).eq([
            "aux-entry",
            "aux-internal",
            "aux-exit"
        ]
        .map(|key| stable_id_for(key, ids)))
    );
}

fn assert_entity_counts(lir: &laneflow_compiler::ValidatedCanonicalLir, expected: [usize; 22]) {
    let actual = [
        lir.lane_edges().len(),
        lir.road_corridors().len(),
        lir.road_sections().len(),
        lir.authoring_lanes().len(),
        lir.lane_groups().len(),
        lir.facility_bands().len(),
        lir.junctions().len(),
        lir.movements().len(),
        lir.maneuver_paths().len(),
        lir.stop_lines().len(),
        lir.maneuver_gates().len(),
        lir.waiting_zones().len(),
        lir.signal_groups().len(),
        lir.signal_controllers().len(),
        lir.signal_phases().len(),
        lir.parking_areas().len(),
        lir.parking_spaces().len(),
        lir.participant_classes().len(),
        lir.vehicle_profiles().len(),
        lir.access_rules().len(),
        lir.static_routes().len(),
        lir.canonical_frames().len(),
    ];
    assert_eq!(actual, expected);
}
