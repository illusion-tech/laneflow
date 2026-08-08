//! Synthetic 孪生构造：消费与 geometry 文档发射相同的走廊语义模型，并以 geometry 编译
//! 输出收获的派生值（长度、successor、规范 f32 点）作为逐位相同的输入，复刻切片 6 的
//! 等价验证模式。孪生无法表达 facility band 几何行，该已知差异在 selfcheck 中单独核对。

use std::collections::BTreeMap;

use laneflow_compiler::{
    AccessEffect, AccessRuleInput, AccessRuleTargetInput, AuthoringLaneInput, CanonicalFrameInput,
    CanonicalPoint3F32Input, CompilationOutput, CorridorElementReference, FacilityBandInput,
    FacilityBandReference, IidmVehicleProfileInput, JunctionInput, JunctionReference,
    LaneEdgeGeometryInput, LaneEdgeInput, LaneEdgeReference, LaneGroupInput, LaneGroupReference,
    ManeuverGateInput, ManeuverGateReference, ManeuverPathInput, ManeuverPathReference,
    MovementInput, MovementReference, ParkingAreaInput, ParkingAreaReference,
    ParkingLaneAnchorInput, ParkingSpaceGeometryInput, ParkingSpaceInput, ParticipantClassInput,
    ParticipantClassReference, RoadCorridorInput, RoadSectionInput, RoadSectionReference,
    SignalAspect, SignalControlInput, SignalControllerInput, SignalGroupInput,
    SignalGroupReference, SignalGroupStateInput, SignalPhaseInput, SourceModuleHeader,
    SourceModuleHeaderInput, StaticRouteInput, StopLineInput, StopLineReference, SyntheticModule,
    SyntheticModuleBuilder, VehicleProfileInput, WaitingZoneInput,
};
use laneflow_static_contract::FieldTag;

use crate::corridor::{
    AccessRuleModel, ClassModel, ControllerModel, CorridorElementModel, CorridorModel, GateModel,
    ProfileModel, SectionModel,
};

/// 一条 lane edge 的孪生输入：全部取自 geometry 编译输出的派生值。
#[derive(Clone, Debug)]
pub struct TwinEdgeHarvest {
    pub length_meters: f64,
    pub speed_mps: f64,
    pub successors: Vec<String>,
    pub points: Vec<[f32; 3]>,
}

/// geometry 编译输出的孪生收获：全部 lane edge 派生值与 facility band 点总数。
#[derive(Clone, Debug, Default)]
pub struct Harvest {
    pub edges: BTreeMap<String, TwinEdgeHarvest>,
    pub band_point_count: u64,
}

fn lane_edge_key_of(edge: &laneflow_compiler::CanonicalLaneEdgeView<'_>) -> String {
    edge.identity_fields()
        .find(|field| field.tag() == FieldTag::LaneEdgeKey)
        .map(|field| String::from_utf8(field.value_bytes().to_vec()).unwrap())
        .expect("lane edge 必须携带 LaneEdgeKey 身份字段")
}

/// 从 geometry 编译输出收获孪生输入：派生长度、限速、successor 与规范 f32 点逐位复制。
pub fn harvest_geometry_output(output: &CompilationOutput) -> Harvest {
    let lir = output.lir();
    let mut harvest = Harvest::default();
    for edge in lir.lane_edges() {
        let geometry = edge
            .spatial_geometry()
            .expect("走廊全部 edge 必须携带空间几何");
        let key = lane_edge_key_of(&edge);
        let successors = edge
            .successors()
            .iter()
            .map(|ordinal| lane_edge_key_of(&lir.lane_edge(*ordinal).unwrap()))
            .collect();
        harvest.edges.insert(
            key,
            TwinEdgeHarvest {
                length_meters: edge.length_meters(),
                speed_mps: edge.speed_limit_meters_per_second(),
                successors,
                points: geometry
                    .points()
                    .map(|point| [point.x, point.y, point.z])
                    .collect(),
            },
        );
    }
    for band in lir.facility_bands() {
        if let Some(geometry) = band.geometry() {
            harvest.band_point_count = harvest
                .band_point_count
                .saturating_add(u64::try_from(geometry.points().len()).unwrap_or(u64::MAX));
        }
    }
    harvest
}

fn signal_aspect(aspect: &str) -> SignalAspect {
    match aspect {
        "red" => SignalAspect::Red,
        "yellow" => SignalAspect::Yellow,
        "green" => SignalAspect::Green,
        other => panic!("不支持的 signal aspect {other}"),
    }
}

fn header(
    namespace: &str,
    document_key: &str,
    limits: &laneflow_compiler::CompileLimits,
) -> SourceModuleHeader {
    SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: namespace,
            source_document_key: document_key,
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: Some(42),
            provenance: "repository:laneflow",
        },
        limits,
    )
    .unwrap()
}

fn add_sections(builder: &mut SyntheticModuleBuilder, sections: &[SectionModel]) {
    for section in sections {
        let lane_keys: Vec<String> = section.lanes.iter().map(|lane| lane.key.clone()).collect();
        let edge_chains: Vec<Vec<LaneEdgeReference<'_>>> = section
            .lanes
            .iter()
            .map(|lane| vec![LaneEdgeReference::local(lane.edge_key.as_str())])
            .collect();
        let lane_inputs: Vec<AuthoringLaneInput<'_>> = section
            .lanes
            .iter()
            .enumerate()
            .map(|(index, lane)| AuthoringLaneInput {
                authoring_lane_key: &lane_keys[index],
                edge_chain: &edge_chains[index],
                lane_group: lane.lane_group.as_deref().map(LaneGroupReference::local),
            })
            .collect();
        builder
            .add_road_section(RoadSectionInput {
                road_section_key: &section.key,
                kind_id: &section.kind_id,
                lanes: &lane_inputs,
            })
            .unwrap();
    }
}

fn add_controllers(builder: &mut SyntheticModuleBuilder, controllers: &[ControllerModel]) {
    for controller in controllers {
        let groups: Vec<_> = controller
            .groups
            .iter()
            .map(|group| SignalGroupReference::local(group.as_str()))
            .collect();
        let states: Vec<Vec<SignalGroupStateInput<'_>>> = controller
            .phases
            .iter()
            .map(|phase| {
                phase
                    .states
                    .iter()
                    .map(|state| SignalGroupStateInput {
                        signal_group: SignalGroupReference::local(state.group.as_str()),
                        aspect: signal_aspect(&state.aspect),
                    })
                    .collect()
            })
            .collect();
        let phase_inputs: Vec<SignalPhaseInput<'_>> = controller
            .phases
            .iter()
            .enumerate()
            .map(|(index, phase)| SignalPhaseInput {
                signal_phase_key: &phase.key,
                duration_ms: phase.duration_ms,
                states: &states[index],
            })
            .collect();
        builder
            .add_signal_controller(SignalControllerInput {
                signal_controller_key: &controller.key,
                offset_ms: controller.offset_ms,
                signal_groups: &groups,
                phases: &phase_inputs,
            })
            .unwrap();
    }
}

fn add_gates(builder: &mut SyntheticModuleBuilder, gates: &[GateModel]) {
    for gate in gates {
        let control = match &gate.signal_group {
            Some(group) => SignalControlInput::Group(SignalGroupReference::local(group.as_str())),
            None => SignalControlInput::None,
        };
        builder
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: &gate.key,
                maneuver_path: ManeuverPathReference::local(gate.path.as_str()),
                transition_index: gate.transition_index,
                stop_line: StopLineReference::local(gate.stop_line.as_str()),
                signal_control: control,
            })
            .unwrap();
    }
}

fn add_classes(builder: &mut SyntheticModuleBuilder, classes: &[ClassModel]) {
    for class in classes {
        builder
            .add_participant_class(ParticipantClassInput {
                participant_class_key: &class.key,
                extends: class
                    .extends
                    .as_deref()
                    .map(ParticipantClassReference::local),
            })
            .unwrap();
    }
}

fn add_profiles(builder: &mut SyntheticModuleBuilder, profiles: &[ProfileModel]) {
    for profile in profiles {
        builder
            .add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: &profile.key,
                participant_class: ParticipantClassReference::local(
                    profile.participant_class.as_str(),
                ),
                iidm: IidmVehicleProfileInput {
                    length_meters: profile.iidm[0],
                    desired_speed_meters_per_second: profile.iidm[1],
                    min_gap_meters: profile.iidm[2],
                    time_headway_seconds: profile.iidm[3],
                    max_acceleration_meters_per_second_squared: profile.iidm[4],
                    comfortable_deceleration_meters_per_second_squared: profile.iidm[5],
                    emergency_deceleration_meters_per_second_squared: profile.iidm[6],
                },
            })
            .unwrap();
    }
}

fn add_access_rules(builder: &mut SyntheticModuleBuilder, rules: &[AccessRuleModel]) {
    for rule in rules {
        let classes: Vec<_> = rule
            .participant_classes
            .iter()
            .map(|class| ParticipantClassReference::local(class.as_str()))
            .collect();
        let effect = match rule.effect.as_str() {
            "allow" => AccessEffect::Allow,
            "deny" => AccessEffect::Deny,
            other => panic!("不支持的 access effect {other}"),
        };
        builder
            .add_access_rule(AccessRuleInput {
                access_rule_key: &rule.key,
                target: AccessRuleTargetInput::LaneGroup(LaneGroupReference::local(
                    rule.target_lane_group.as_str(),
                )),
                effect,
                participant_classes: &classes,
                regulation: None,
                priority: 0,
            })
            .unwrap();
    }
}

/// 构造单份走廊的 Synthetic 孪生模块；lane edge 长度、successor 与几何点逐位取自
/// `harvest`，其余语义内容来自同一 `CorridorModel`。
pub fn build_corridor_twin(
    model: &CorridorModel,
    namespace: &str,
    document_key: &str,
    limits: &laneflow_compiler::CompileLimits,
    harvest: &Harvest,
) -> SyntheticModule {
    let mut builder =
        SyntheticModuleBuilder::new(header(namespace, document_key, limits), limits).unwrap();

    for (edge_key, _) in model.all_edges() {
        let edge = harvest
            .edges
            .get(edge_key)
            .unwrap_or_else(|| panic!("收获缺少 edge {edge_key}"));
        let successors: Vec<_> = edge
            .successors
            .iter()
            .map(|successor| LaneEdgeReference::local(successor.as_str()))
            .collect();
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: edge_key,
                length_meters: edge.length_meters,
                speed_limit_meters_per_second: edge.speed_mps,
                successors: &successors,
            })
            .unwrap();
    }
    let point_buffers: Vec<Vec<CanonicalPoint3F32Input>> = model
        .all_edges()
        .map(|(edge_key, _)| {
            harvest.edges[edge_key]
                .points
                .iter()
                .map(|point| CanonicalPoint3F32Input {
                    x: point[0],
                    y: point[1],
                    z: point[2],
                })
                .collect()
        })
        .collect();
    let geometries: Vec<LaneEdgeGeometryInput<'_>> = point_buffers
        .iter()
        .zip(model.all_edges())
        .map(|(points, (edge_key, _))| LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local(edge_key),
            centerline_points: points.as_slice(),
        })
        .collect();
    builder
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame.main",
            lane_edge_geometries: geometries.as_slice(),
        })
        .unwrap();

    for road in &model.roads {
        for band in &road.bands {
            builder
                .add_facility_band(FacilityBandInput {
                    facility_band_key: &band.key,
                    kind_id: &band.kind_id,
                })
                .unwrap();
        }
        for section in &road.sections {
            for group in &section.lane_groups {
                builder
                    .add_lane_group(LaneGroupInput {
                        lane_group_key: group,
                        road_section: RoadSectionReference::local(section.key.as_str()),
                    })
                    .unwrap();
            }
        }
        add_sections(&mut builder, &road.sections);
        let elements: Vec<_> = road
            .elements
            .iter()
            .map(|element| match element {
                CorridorElementModel::RoadSection(section) => {
                    CorridorElementReference::road_section(RoadSectionReference::local(
                        section.as_str(),
                    ))
                }
                CorridorElementModel::FacilityBand(band) => {
                    CorridorElementReference::facility_band(FacilityBandReference::local(
                        band.as_str(),
                    ))
                }
            })
            .collect();
        builder
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: &road.corridor_key,
                reference_section: RoadSectionReference::local(road.reference_section.as_str()),
                elements: &elements,
            })
            .unwrap();
    }

    for junction in &model.junctions {
        builder
            .add_junction(JunctionInput {
                junction_key: &junction.key,
            })
            .unwrap();
        // geometry 前端每 connection 一条 movement；孪生按同构镜像（键=路径键）。
        for connection in &junction.connections {
            builder
                .add_movement(MovementInput {
                    movement_key: &connection.path_key,
                    junction: JunctionReference::local(junction.key.as_str()),
                    directed_entry_approach_key: &format!("{}/entry", connection.path_key),
                    directed_exit_approach_key: &format!("{}/exit", connection.path_key),
                })
                .unwrap();
        }
        for connection in &junction.connections {
            builder
                .add_maneuver_path(ManeuverPathInput {
                    maneuver_path_key: &connection.path_key,
                    movement: MovementReference::local(connection.path_key.as_str()),
                    entry_edge: LaneEdgeReference::local(connection.entry_edge.as_str()),
                    internal_edges: &[LaneEdgeReference::local(connection.internal_edge.as_str())],
                    exit_edge: LaneEdgeReference::local(connection.exit_edge.as_str()),
                })
                .unwrap();
        }
    }

    for (key, edge) in &model.stop_lines {
        builder
            .add_stop_line(StopLineInput {
                stop_line_key: key,
                lane_edge: LaneEdgeReference::local(edge.as_str()),
            })
            .unwrap();
    }
    for group in &model.signal_groups {
        builder
            .add_signal_group(SignalGroupInput {
                signal_group_key: group,
            })
            .unwrap();
    }
    add_controllers(&mut builder, &model.controllers);
    add_gates(&mut builder, &model.gates);

    for zone in &model.waiting_zones {
        builder
            .add_waiting_zone(WaitingZoneInput {
                waiting_zone_key: &zone.key,
                maneuver_path: ManeuverPathReference::local(zone.path.as_str()),
                entry_gate: ManeuverGateReference::local(zone.entry_gate.as_str()),
                release_gate: ManeuverGateReference::local(zone.release_gate.as_str()),
                max_occupancy: zone.max_occupancy,
            })
            .unwrap();
    }
    for area in &model.parking_areas {
        builder
            .add_parking_area(ParkingAreaInput {
                parking_area_key: area,
            })
            .unwrap();
    }
    for (key, area, entry, exit) in &model.parking_spaces {
        builder
            .add_parking_space(ParkingSpaceInput {
                parking_space_key: key,
                parking_area: Some(ParkingAreaReference::local(area.as_str())),
                entry: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local(crate::corridor::PARKING_EDGE),
                    progress_meters: *entry,
                },
                exit: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local(crate::corridor::PARKING_EDGE),
                    progress_meters: *exit,
                },
                geometry: ParkingSpaceGeometryInput {
                    lateral_offset_meters: 1.5,
                    heading_offset_radians: 0.0,
                    length_meters: 6.0,
                    width_meters: 2.5,
                },
            })
            .unwrap();
    }

    add_classes(&mut builder, &model.classes);
    add_profiles(&mut builder, &model.profiles);
    add_access_rules(&mut builder, &model.access_rules);
    for (key, edges) in &model.routes {
        let edge_refs: Vec<_> = edges
            .iter()
            .map(|edge| LaneEdgeReference::local(edge.as_str()))
            .collect();
        builder
            .add_static_route(StaticRouteInput {
                static_route_key: key,
                edge_sequence: &edge_refs,
            })
            .unwrap();
    }

    builder.finish().unwrap()
}
