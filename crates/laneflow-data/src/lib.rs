#![doc = include_str!("../README.md")]

mod error;
mod scenario;
mod scenario_error;
mod scenario_wire;
mod wire;

use laneflow_core::{
    AccessEffect, AccessRegistry, AccessRegulation, AccessRule, AccessTargetId, CoreError,
    CorridorElementId, CrossSectionRegistry, EdgeLength, FacilityBand, IidmProfileSpec,
    InitialTrafficData, Junction, JunctionRegistry, LaneEdge, LaneGraph, LaneGroup, ManeuverGate,
    ManeuverPath, Movement, ParkingAnchorKind, ParkingArea, ParkingRegistry, ParkingSpace,
    ParkingSpaceGeometry, ParticipantClass, ParticipantClassRegistry, RoadCorridor, RoadSection,
    Route, SectionLane, SignalAspect, SignalControlInput, SignalController, SignalGroup,
    SignalGroupState, SignalPhase, SignalRegistry, SpeedLimit, StopLine, StopLineLocation,
    VehicleProfile, VehicleProfileRegistry,
};
use serde::de::DeserializeOwned;
use serde_json::error::Category;

pub use error::DataError;
pub use scenario::{
    CURRENT_SCENARIO_MANIFEST_FORMAT_VERSION, CURRENT_SPATIAL_FORMAT_VERSION, LoadedScenario,
    LoadedSpatialEdge, LoadedSpatialPackage, NamedArtifact, SPATIAL_PACKAGE_MEDIA_TYPE,
    TRAFFIC_PACKAGE_MEDIA_TYPE, from_scenario_json_slice, from_scenario_json_str,
};
pub use scenario_error::{ArtifactRole, ScenarioDocument, ScenarioError};

use wire::{
    WireCorridorElement, WirePackage, WireParking, WireSignalAspect, WireSignalControl,
    WireSignalControllerKind, WireSignals, WireStopLineLocation, WireVersionHeader,
};

/// 当前 production loader 接受的唯一 data format 版本。
pub const CURRENT_FORMAT_VERSION: &str = "0.9";

/// 已解析并完成 Core normalization 的当前 data package。
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedPackage {
    initial_traffic_data: InitialTrafficData,
}

impl LoadedPackage {
    /// 返回已验证的 Core 初始交通输入。
    pub const fn initial_traffic_data(&self) -> &InitialTrafficData {
        &self.initial_traffic_data
    }

    /// 消费 loaded package 并返回 Core 初始交通输入。
    pub fn into_initial_traffic_data(self) -> InitialTrafficData {
        self.initial_traffic_data
    }
}

/// 从 UTF-8 JSON bytes 解析并完成 Core normalization。
///
/// # Errors
///
/// JSON syntax/shape、format version、units 或 Core domain validation 失败时返回
/// 结构化 `DataError`。本函数不读取文件，也不返回部分初始化结果。
pub fn from_json_slice(input: &[u8]) -> Result<LoadedPackage, DataError> {
    let header: WireVersionHeader = deserialize_json(input)?;
    if header.format_version != CURRENT_FORMAT_VERSION {
        return Err(DataError::UnsupportedFormatVersion {
            expected: CURRENT_FORMAT_VERSION,
            actual: header.format_version,
        });
    }

    let wire: WirePackage = deserialize_json(input)?;
    debug_assert_eq!(wire.format_version, header.format_version);
    normalize(&wire)
}

fn deserialize_json<T>(input: &[u8]) -> Result<T, DataError>
where
    T: DeserializeOwned,
{
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    let value =
        serde_path_to_error::deserialize(&mut deserializer).map_err(DataError::from_path_error)?;
    deserializer
        .end()
        .map_err(|source| DataError::from_json_error("$".to_owned(), source, Category::Syntax))?;
    Ok(value)
}

/// 从 UTF-8 JSON string 解析并完成 Core normalization。
///
/// # Errors
///
/// 与 `from_json_slice` 相同。
pub fn from_json_str(input: &str) -> Result<LoadedPackage, DataError> {
    from_json_slice(input.as_bytes())
}

fn normalize(wire: &WirePackage) -> Result<LoadedPackage, DataError> {
    validate_unit("units.distance", "meter", &wire.units.distance)?;
    validate_unit("units.time", "second", &wire.units.time)?;

    // SSOT cross-section-access §10：profile 域段（phase 1-2）在 lane graph 之前；
    // 拓扑依赖段（phase 3-10）在 lane graph / Junction / Signals / Parking 之后。
    let participant_classes = normalize_participant_classes(wire)?;
    let profile_registry = normalize_profiles(wire, &participant_classes)?;
    let lane_graph = normalize_lane_graph(wire)?;
    let junctions = normalize_junctions(&lane_graph, wire)?;
    let signals = normalize_signals(&lane_graph, &junctions, &wire.signals)?;
    let parking = normalize_parking(&lane_graph, &wire.parking)?;
    let cross_section = normalize_cross_section(&lane_graph, wire)?;
    let access = normalize_access(
        &lane_graph,
        &junctions,
        &cross_section,
        &participant_classes,
        wire,
    )?;
    let routes = normalize_routes(wire)?;

    let initial_traffic_data = InitialTrafficData::try_new(
        lane_graph,
        routes,
        profile_registry,
        junctions,
        signals,
        parking,
        participant_classes,
        cross_section,
        access,
    )
    .map_err(|source| DataError::core(initial_traffic_error_path(wire, &source), source))?;
    Ok(LoadedPackage {
        initial_traffic_data,
    })
}

fn normalize_participant_classes(
    wire: &WirePackage,
) -> Result<ParticipantClassRegistry, DataError> {
    let classes = wire
        .participant_classes
        .iter()
        .map(|class| ParticipantClass::new(class.id.clone(), class.extends_id.as_deref()))
        .collect::<Vec<_>>();
    ParticipantClassRegistry::try_new(classes)
        .map_err(|source| DataError::core(participant_class_error_path(wire, &source), source))
}

fn normalize_profiles(
    wire: &WirePackage,
    participant_classes: &ParticipantClassRegistry,
) -> Result<VehicleProfileRegistry, DataError> {
    let mut normalized_profiles = Vec::with_capacity(wire.vehicle_profiles.len());
    for (index, profile) in wire.vehicle_profiles.iter().enumerate() {
        if profile.model != "iidm" {
            return Err(DataError::UnsupportedVehicleProfileModel {
                path: format!("vehicleProfiles[{index}].model"),
                profile_id: profile.id.clone(),
                actual: profile.model.clone(),
            });
        }

        let participant_class = participant_classes
            .class_handle(&profile.participant_class_id)
            .ok_or_else(|| DataError::UnknownVehicleProfileParticipantClass {
                path: format!("vehicleProfiles[{index}].participantClassId"),
                profile_id: profile.id.clone(),
                class_id: profile.participant_class_id.clone(),
            })?;

        let spec = IidmProfileSpec {
            length: profile.length,
            desired_speed: profile.desired_speed,
            min_gap: profile.min_gap,
            time_headway: profile.time_headway,
            max_acceleration: profile.max_acceleration,
            comfortable_deceleration: profile.comfortable_deceleration,
            emergency_deceleration: profile.emergency_deceleration,
        };
        let normalized = VehicleProfile::try_new_iidm(profile.id.clone(), participant_class, spec)
            .map_err(|source| DataError::core(format!("vehicleProfiles[{index}]"), source))?;
        normalized_profiles.push(normalized);
    }
    VehicleProfileRegistry::try_new(participant_classes, normalized_profiles)
        .map_err(|source| DataError::core("vehicleProfiles", source))
}

fn normalize_lane_graph(wire: &WirePackage) -> Result<LaneGraph, DataError> {
    let mut edges = Vec::with_capacity(wire.lane_graph.edges.len());
    for (index, edge) in wire.lane_graph.edges.iter().enumerate() {
        let length = EdgeLength::try_new(edge.length).map_err(|source| {
            DataError::core(format!("laneGraph.edges[{index}].length"), source)
        })?;
        let speed_limit = SpeedLimit::try_new(edge.speed_limit).map_err(|source| {
            DataError::core(format!("laneGraph.edges[{index}].speedLimit"), source)
        })?;
        edges.push(LaneEdge::new(
            edge.id.clone(),
            length,
            speed_limit,
            edge.connections
                .iter()
                .map(|connection| connection.to_edge_id.clone()),
        ));
    }
    LaneGraph::try_new(edges).map_err(|source| DataError::core("laneGraph.edges", source))
}

fn normalize_junctions(
    lane_graph: &LaneGraph,
    wire: &WirePackage,
) -> Result<JunctionRegistry, DataError> {
    let junctions = wire
        .junctions
        .iter()
        .map(|junction| Junction::new(junction.id.clone()))
        .collect::<Vec<_>>();
    let movements = wire
        .movements
        .iter()
        .map(|movement| Movement::new(movement.id.clone(), movement.junction_id.clone()))
        .collect::<Vec<_>>();
    let maneuver_paths = wire
        .maneuver_paths
        .iter()
        .map(|path| {
            ManeuverPath::new(
                path.id.clone(),
                path.movement_id.clone(),
                path.entry_edge_id.clone(),
                path.internal_edge_ids.iter().cloned(),
                path.exit_edge_id.clone(),
            )
        })
        .collect::<Vec<_>>();

    JunctionRegistry::try_new(lane_graph, junctions, movements, maneuver_paths)
        .map_err(|source| DataError::core(junction_error_path(wire, &source), source))
}

fn normalize_signals(
    lane_graph: &LaneGraph,
    junctions: &JunctionRegistry,
    wire: &WireSignals,
) -> Result<SignalRegistry, DataError> {
    let mut stop_lines = Vec::with_capacity(wire.stop_lines.len());
    for stop_line in &wire.stop_lines {
        let location = match stop_line.location {
            WireStopLineLocation::EdgeEnd => StopLineLocation::EdgeEnd,
        };
        stop_lines.push(StopLine::new(
            stop_line.id.clone(),
            stop_line.edge_id.clone(),
            location,
        ));
    }

    let mut groups = Vec::with_capacity(wire.groups.len());
    for group in &wire.groups {
        groups.push(SignalGroup::new(group.id.clone()));
    }

    let mut controllers = Vec::with_capacity(wire.controllers.len());
    for controller in &wire.controllers {
        let mut phases = Vec::with_capacity(controller.phases.len());
        for phase in &controller.phases {
            let mut states = Vec::with_capacity(phase.states.len());
            for state in &phase.states {
                let aspect = match state.aspect {
                    WireSignalAspect::Red => SignalAspect::Red,
                    WireSignalAspect::Yellow => SignalAspect::Yellow,
                    WireSignalAspect::Green => SignalAspect::Green,
                };
                states.push(SignalGroupState::new(state.group_id.clone(), aspect));
            }
            phases.push(SignalPhase::new(
                phase.id.clone(),
                phase.duration_ms,
                states,
            ));
        }

        let normalized = match controller.kind {
            WireSignalControllerKind::FixedTime => SignalController::new_fixed_time(
                controller.id.clone(),
                controller.offset_ms,
                controller.group_ids.iter().cloned(),
                phases,
            ),
        };
        controllers.push(normalized);
    }

    let mut maneuver_gates = Vec::with_capacity(wire.maneuver_gates.len());
    for gate in &wire.maneuver_gates {
        let control = match &gate.signal_control {
            WireSignalControl::Group(control) => {
                let _kind = control.kind;
                SignalControlInput::Group(control.group_id.clone())
            }
            WireSignalControl::None(control) => {
                let _kind = control.kind;
                SignalControlInput::None
            }
        };
        maneuver_gates.push(ManeuverGate::new(
            gate.id.clone(),
            gate.maneuver_path_id.clone(),
            gate.transition_index,
            gate.stop_line_id.clone(),
            control,
        ));
    }

    SignalRegistry::try_new(
        lane_graph,
        junctions,
        stop_lines,
        groups,
        controllers,
        maneuver_gates,
    )
    .map_err(|source| DataError::core(signal_error_path(wire, &source), source))
}

fn normalize_parking(
    lane_graph: &LaneGraph,
    wire: &WireParking,
) -> Result<ParkingRegistry, DataError> {
    let areas = wire
        .areas
        .iter()
        .map(|area| ParkingArea::new(area.id.clone()))
        .collect::<Vec<_>>();
    let spaces = wire
        .spaces
        .iter()
        .map(|space| {
            ParkingSpace::new(
                space.id.clone(),
                space.area_id.as_deref().map(str::to_owned),
                space.entry.edge_id.clone(),
                space.entry.progress,
                space.exit.edge_id.clone(),
                space.exit.progress,
                ParkingSpaceGeometry::new(
                    space.geometry.lateral_offset,
                    space.geometry.heading_offset_radians,
                    space.geometry.length,
                    space.geometry.width,
                ),
            )
        })
        .collect::<Vec<_>>();
    ParkingRegistry::try_new(lane_graph, areas, spaces)
        .map_err(|source| DataError::core(parking_error_path(wire, &source), source))
}

fn normalize_cross_section(
    lane_graph: &LaneGraph,
    wire: &WirePackage,
) -> Result<CrossSectionRegistry, DataError> {
    let bands = wire
        .facility_bands
        .iter()
        .map(|band| FacilityBand::new(band.id.clone(), band.kind_id.clone()))
        .collect::<Vec<_>>();
    let sections = wire
        .road_sections
        .iter()
        .map(|section| {
            RoadSection::new(
                section.id.clone(),
                section.kind_id.clone(),
                section.lanes.iter().map(|lane| {
                    SectionLane::new(lane.edge_ids.iter().cloned(), lane.lane_group_id.as_deref())
                }),
            )
        })
        .collect::<Vec<_>>();
    let groups = wire
        .lane_groups
        .iter()
        .map(|group| LaneGroup::new(group.id.clone(), group.road_section_id.clone()))
        .collect::<Vec<_>>();
    let corridors = wire
        .road_corridors
        .iter()
        .map(|corridor| {
            RoadCorridor::new(
                corridor.id.clone(),
                corridor.reference_section_id.clone(),
                corridor.elements.iter().map(|element| match element {
                    WireCorridorElement::Section(section) => {
                        CorridorElementId::section(section.section_id.clone())
                    }
                    WireCorridorElement::Band(band) => {
                        CorridorElementId::band(band.band_id.clone())
                    }
                }),
            )
        })
        .collect::<Vec<_>>();

    CrossSectionRegistry::try_new(lane_graph, bands, sections, groups, corridors)
        .map_err(|source| DataError::core(cross_section_error_path(wire, &source), source))
}

fn normalize_access(
    lane_graph: &LaneGraph,
    junctions: &JunctionRegistry,
    cross_section: &CrossSectionRegistry,
    participant_classes: &ParticipantClassRegistry,
    wire: &WirePackage,
) -> Result<AccessRegistry, DataError> {
    let rules = wire
        .access_rules
        .iter()
        .enumerate()
        .map(|(index, rule)| {
            let target = match rule.target.kind {
                wire::WireAccessTargetKind::LaneEdge => {
                    AccessTargetId::lane_edge(rule.target.id.clone())
                }
                wire::WireAccessTargetKind::LaneGroup => {
                    AccessTargetId::lane_group(rule.target.id.clone())
                }
                wire::WireAccessTargetKind::RoadSection => {
                    AccessTargetId::road_section(rule.target.id.clone())
                }
                wire::WireAccessTargetKind::ManeuverPath => {
                    AccessTargetId::maneuver_path(rule.target.id.clone())
                }
                wire::WireAccessTargetKind::FacilityBand => {
                    AccessTargetId::facility_band(rule.target.id.clone())
                }
            };
            let effect = match rule.effect {
                wire::WireAccessEffect::Allow => AccessEffect::Allow,
                wire::WireAccessEffect::Deny => AccessEffect::Deny,
            };
            let mut definition = AccessRule::new(
                rule.id.clone(),
                target,
                effect,
                rule.participant_class_ids.iter().cloned(),
            )
            // wire 层 timeWindows 只作 capability guard 归因标记进 Core（v1 一律拒绝）。
            .with_time_windows(rule.time_windows.is_some())
            .with_priority(rule.priority.unwrap_or(0));
            if let Some(regulation) = &rule.regulation {
                let parsed = AccessRegulation::try_new(
                    regulation.jurisdiction.clone(),
                    regulation.version.clone(),
                    regulation.source.as_deref(),
                )
                .map_err(|source| {
                    let field = match &source {
                        CoreError::InvalidAccessRegulationString { field, .. } => *field,
                        _ => unreachable!("AccessRegulation::try_new 只返回 regulation 字段错误"),
                    };
                    DataError::core(format!("accessRules[{index}].regulation.{field}"), source)
                })?;
                definition = definition.with_regulation(parsed);
            }
            Ok(definition)
        })
        .collect::<Result<Vec<_>, DataError>>()?;

    AccessRegistry::try_new(
        lane_graph,
        junctions,
        cross_section,
        participant_classes,
        rules,
    )
    .map_err(|source| DataError::core(access_error_path(wire, &source), source))
}

fn normalize_routes(wire: &WirePackage) -> Result<Vec<Route>, DataError> {
    let mut routes = Vec::with_capacity(wire.routes.len());
    for (index, route) in wire.routes.iter().enumerate() {
        routes.push(
            Route::try_new(route.id.clone(), route.edge_ids.iter().cloned()).map_err(|source| {
                DataError::core(route_input_error_path(index, route, &source), source)
            })?,
        );
    }
    Ok(routes)
}

fn parking_error_path(wire: &WireParking, source: &CoreError) -> String {
    match source {
        CoreError::InvalidExternalId {
            field, external_id, ..
        } => match *field {
            "parking.areas[].id" => wire
                .areas
                .iter()
                .position(|area| area.id == *external_id)
                .map_or_else(
                    || "parking.areas".to_owned(),
                    |index| format!("parking.areas[{index}].id"),
                ),
            "parking.spaces[].id" => wire
                .spaces
                .iter()
                .position(|space| space.id == *external_id)
                .map_or_else(
                    || "parking.spaces".to_owned(),
                    |index| format!("parking.spaces[{index}].id"),
                ),
            "parking.spaces[].areaId" => wire
                .spaces
                .iter()
                .position(|space| space.area_id.as_deref() == Some(external_id))
                .map_or_else(
                    || "parking.spaces".to_owned(),
                    |index| format!("parking.spaces[{index}].areaId"),
                ),
            "parking.spaces[].entry.edgeId" => {
                parking_anchor_external_id_path(wire, ParkingAnchorKind::Entry, external_id)
            }
            "parking.spaces[].exit.edgeId" => {
                parking_anchor_external_id_path(wire, ParkingAnchorKind::Exit, external_id)
            }
            _ => "parking".to_owned(),
        },
        CoreError::DuplicateParkingAreaId { area_id } => {
            second_matching_index(&wire.areas, |area| area.id == *area_id).map_or_else(
                || "parking.areas".to_owned(),
                |index| format!("parking.areas[{index}].id"),
            )
        }
        CoreError::DuplicateParkingSpaceId { space_id } => {
            second_matching_index(&wire.spaces, |space| space.id == *space_id).map_or_else(
                || "parking.spaces".to_owned(),
                |index| format!("parking.spaces[{index}].id"),
            )
        }
        CoreError::UnknownParkingSpaceArea { space_id, .. } => parking_space_index(wire, space_id)
            .map_or_else(
                || "parking.spaces".to_owned(),
                |index| format!("parking.spaces[{index}].areaId"),
            ),
        CoreError::UnknownParkingAnchorEdge {
            space_id, anchor, ..
        } => parking_anchor_path(wire, space_id, *anchor, "edgeId"),
        CoreError::ParkingAnchorProgressOutOfRange {
            space_id, anchor, ..
        } => parking_anchor_path(wire, space_id, *anchor, "progress"),
        CoreError::InvalidParkingGeometryValue {
            space_id, field, ..
        } => parking_space_index(wire, space_id).map_or_else(
            || "parking.spaces".to_owned(),
            |index| format!("parking.spaces[{index}].geometry.{field}"),
        ),
        CoreError::OrphanParkingArea { area_id } => wire
            .areas
            .iter()
            .position(|area| area.id == *area_id)
            .map_or_else(
                || "parking.areas".to_owned(),
                |index| format!("parking.areas[{index}]"),
            ),
        _ => "parking".to_owned(),
    }
}

fn parking_space_index(wire: &WireParking, space_id: &str) -> Option<usize> {
    wire.spaces.iter().position(|space| space.id == space_id)
}

fn parking_anchor_path(
    wire: &WireParking,
    space_id: &str,
    anchor: ParkingAnchorKind,
    field: &str,
) -> String {
    let anchor = match anchor {
        ParkingAnchorKind::Entry => "entry",
        ParkingAnchorKind::Exit => "exit",
        _ => "anchor",
    };
    parking_space_index(wire, space_id).map_or_else(
        || "parking.spaces".to_owned(),
        |index| format!("parking.spaces[{index}].{anchor}.{field}"),
    )
}

fn parking_anchor_external_id_path(
    wire: &WireParking,
    anchor: ParkingAnchorKind,
    external_id: &str,
) -> String {
    let index = wire.spaces.iter().position(|space| match anchor {
        ParkingAnchorKind::Entry => space.entry.edge_id == external_id,
        ParkingAnchorKind::Exit => space.exit.edge_id == external_id,
        _ => false,
    });
    let anchor_name = match anchor {
        ParkingAnchorKind::Entry => "entry",
        ParkingAnchorKind::Exit => "exit",
        _ => "anchor",
    };
    index.map_or_else(
        || "parking.spaces".to_owned(),
        |index| format!("parking.spaces[{index}].{anchor_name}.edgeId"),
    )
}

fn participant_class_error_path(wire: &WirePackage, source: &CoreError) -> String {
    match source {
        CoreError::InvalidExternalId {
            field, external_id, ..
        } => match *field {
            "participantClasses[].id" => item_id_path(
                &wire.participant_classes,
                "participantClasses",
                external_id,
                |item| &item.id,
            ),
            "participantClasses[].extendsId" => wire
                .participant_classes
                .iter()
                .position(|item| item.extends_id.as_deref() == Some(external_id))
                .map_or_else(
                    || "participantClasses".to_owned(),
                    |index| format!("participantClasses[{index}].extendsId"),
                ),
            _ => "participantClasses".to_owned(),
        },
        CoreError::DuplicateParticipantClassId { class_id } => {
            second_matching_index(&wire.participant_classes, |item| item.id == *class_id)
                .map_or_else(
                    || "participantClasses".to_owned(),
                    |index| format!("participantClasses[{index}].id"),
                )
        }
        CoreError::UnknownParticipantClassExtends { class_id, .. } => {
            participant_class_path(wire, class_id, ".extendsId")
        }
        CoreError::ParticipantClassInheritanceCycle { class_id } => {
            participant_class_path(wire, class_id, "")
        }
        _ => "participantClasses".to_owned(),
    }
}

fn participant_class_path(wire: &WirePackage, class_id: &str, suffix: &str) -> String {
    wire.participant_classes
        .iter()
        .position(|item| item.id == class_id)
        .map_or_else(
            || "participantClasses".to_owned(),
            |index| format!("participantClasses[{index}]{suffix}"),
        )
}

fn cross_section_error_path(wire: &WirePackage, source: &CoreError) -> String {
    match source {
        CoreError::InvalidExternalId {
            field, external_id, ..
        } => match *field {
            "facilityBands[].id" => {
                item_id_path(&wire.facility_bands, "facilityBands", external_id, |item| {
                    &item.id
                })
            }
            "roadSections[].id" => {
                item_id_path(&wire.road_sections, "roadSections", external_id, |item| {
                    &item.id
                })
            }
            "laneGroups[].id" => {
                item_id_path(&wire.lane_groups, "laneGroups", external_id, |item| {
                    &item.id
                })
            }
            "laneGroups[].roadSectionId" => wire
                .lane_groups
                .iter()
                .position(|item| item.road_section_id == *external_id)
                .map_or_else(
                    || "laneGroups".to_owned(),
                    |index| format!("laneGroups[{index}].roadSectionId"),
                ),
            "roadSections[].lanes[].edgeIds[]" => section_lane_edge_value_path(wire, external_id),
            "roadSections[].lanes[].laneGroupId" => {
                section_lane_group_value_path(wire, external_id)
            }
            "roadCorridors[].id" => {
                item_id_path(&wire.road_corridors, "roadCorridors", external_id, |item| {
                    &item.id
                })
            }
            "roadCorridors[].elements[].sectionId" | "roadCorridors[].elements[].bandId" => {
                corridor_element_value_path(wire, external_id)
            }
            _ => "roadSections".to_owned(),
        },
        CoreError::UnknownFacilityKind { kind }
        | CoreError::FacilityKindTokenTooLong { kind, .. } => {
            if let Some(index) = wire
                .facility_bands
                .iter()
                .position(|item| item.kind_id == *kind)
            {
                return format!("facilityBands[{index}].kindId");
            }
            wire.road_sections
                .iter()
                .position(|item| item.kind_id == *kind)
                .map_or_else(
                    || "roadSections".to_owned(),
                    |index| format!("roadSections[{index}].kindId"),
                )
        }
        CoreError::DuplicateFacilityBandId { band_id } => {
            second_matching_index(&wire.facility_bands, |item| item.id == *band_id).map_or_else(
                || "facilityBands".to_owned(),
                |index| format!("facilityBands[{index}].id"),
            )
        }
        CoreError::FacilityBandKindNotNonTraversable { band_id, .. } => wire
            .facility_bands
            .iter()
            .position(|item| item.id == *band_id)
            .map_or_else(
                || "facilityBands".to_owned(),
                |index| format!("facilityBands[{index}].kindId"),
            ),
        CoreError::DuplicateRoadSectionId { section_id } => {
            second_matching_index(&wire.road_sections, |item| item.id == *section_id).map_or_else(
                || "roadSections".to_owned(),
                |index| format!("roadSections[{index}].id"),
            )
        }
        CoreError::RoadSectionKindNotLaneBearing { section_id, .. } => {
            section_path(wire, section_id, ".kindId")
        }
        CoreError::DuplicateLaneGroupId { group_id } => {
            second_matching_index(&wire.lane_groups, |item| item.id == *group_id).map_or_else(
                || "laneGroups".to_owned(),
                |index| format!("laneGroups[{index}].id"),
            )
        }
        CoreError::UnknownLaneGroupRoadSection { group_id, .. } => {
            lane_group_path(wire, group_id, ".roadSectionId")
        }
        CoreError::EmptyRoadSectionLanes { section_id } => section_path(wire, section_id, ".lanes"),
        CoreError::EmptySectionLaneChain {
            section_id,
            lane_index,
        } => section_lane_path(wire, section_id, *lane_index, ".edgeIds"),
        CoreError::UnknownSectionLaneEdge {
            section_id,
            lane_index,
            edge_id,
        }
        | CoreError::DuplicateSectionLaneEdge {
            section_id,
            lane_index,
            edge_id,
        } => {
            let Some(section_index) = section_index(wire, section_id) else {
                return "roadSections".to_owned();
            };
            let lane = &wire.road_sections[section_index].lanes[*lane_index];
            lane.edge_ids
                .iter()
                .position(|item| item == edge_id)
                .map_or_else(
                    || format!("roadSections[{section_index}].lanes[{lane_index}].edgeIds"),
                    |edge_index| {
                        format!(
                            "roadSections[{section_index}].lanes[{lane_index}].edgeIds[{edge_index}]"
                        )
                    },
                )
        }
        CoreError::DisconnectedSectionLane {
            section_id,
            lane_index,
            transition_index,
            ..
        } => {
            let Some(section_index) = section_index(wire, section_id) else {
                return "roadSections".to_owned();
            };
            format!(
                "roadSections[{section_index}].lanes[{lane_index}].edgeIds[{transition}]",
                transition = transition_index + 1
            )
        }
        CoreError::SectionLaneEdgeClaimConflict {
            edge_id,
            duplicate_section_id,
            duplicate_lane_index,
            ..
        } => {
            let Some(section_index) = section_index(wire, duplicate_section_id) else {
                return "roadSections".to_owned();
            };
            let lane = &wire.road_sections[section_index].lanes[*duplicate_lane_index];
            lane.edge_ids
                .iter()
                .position(|item| item == edge_id)
                .map_or_else(
                    || {
                        format!(
                            "roadSections[{section_index}].lanes[{duplicate_lane_index}].edgeIds"
                        )
                    },
                    |edge_index| {
                        format!(
                            "roadSections[{section_index}].lanes[{duplicate_lane_index}].edgeIds[{edge_index}]"
                        )
                    },
                )
        }
        CoreError::UnknownSectionLaneGroup {
            section_id,
            lane_index,
            ..
        }
        | CoreError::SectionLaneGroupSectionMismatch {
            section_id,
            lane_index,
            ..
        } => section_lane_path(wire, section_id, *lane_index, ".laneGroupId"),
        CoreError::EmptyLaneGroup { group_id } => lane_group_path(wire, group_id, ""),
        CoreError::DuplicateRoadCorridorId { corridor_id } => {
            second_matching_index(&wire.road_corridors, |item| item.id == *corridor_id).map_or_else(
                || "roadCorridors".to_owned(),
                |index| format!("roadCorridors[{index}].id"),
            )
        }
        CoreError::EmptyRoadCorridorElements { corridor_id } => {
            corridor_path(wire, corridor_id, ".elements")
        }
        CoreError::UnknownCorridorElement {
            corridor_id,
            element_id,
            ..
        } => corridor_element_path(wire, corridor_id, element_id, false),
        CoreError::DuplicateCorridorElement {
            corridor_id,
            element_id,
            ..
        } => corridor_element_path(wire, corridor_id, element_id, true),
        CoreError::CorridorElementMultipleOwners {
            element_id,
            duplicate_corridor_id,
            ..
        } => corridor_element_path(wire, duplicate_corridor_id, element_id, false),
        CoreError::UnownedCorridorElement {
            element_kind,
            element_id,
        } => match *element_kind {
            "band" => wire
                .facility_bands
                .iter()
                .position(|item| item.id == *element_id)
                .map_or_else(
                    || "facilityBands".to_owned(),
                    |index| format!("facilityBands[{index}]"),
                ),
            _ => section_path(wire, element_id, ""),
        },
        CoreError::CorridorReferenceSectionNotMember { corridor_id, .. } => {
            corridor_path(wire, corridor_id, ".referenceSectionId")
        }
        CoreError::StaticDomainCapacityExceeded { domain, .. } => match *domain {
            "facilityBands" => "facilityBands".to_owned(),
            "roadSections" | "sectionLanes" | "sectionLaneEdgeRefs" => "roadSections".to_owned(),
            "laneGroups" => "laneGroups".to_owned(),
            "roadCorridors" | "corridorElements" => "roadCorridors".to_owned(),
            _ => "roadSections".to_owned(),
        },
        _ => "roadSections".to_owned(),
    }
}

fn section_index(wire: &WirePackage, section_id: &str) -> Option<usize> {
    wire.road_sections
        .iter()
        .position(|item| item.id == section_id)
}

fn section_path(wire: &WirePackage, section_id: &str, suffix: &str) -> String {
    section_index(wire, section_id).map_or_else(
        || "roadSections".to_owned(),
        |index| format!("roadSections[{index}]{suffix}"),
    )
}

fn section_lane_path(
    wire: &WirePackage,
    section_id: &str,
    lane_index: usize,
    suffix: &str,
) -> String {
    section_index(wire, section_id).map_or_else(
        || "roadSections".to_owned(),
        |index| format!("roadSections[{index}].lanes[{lane_index}]{suffix}"),
    )
}

fn section_lane_edge_value_path(wire: &WirePackage, edge_id: &str) -> String {
    for (section_index, section) in wire.road_sections.iter().enumerate() {
        for (lane_index, lane) in section.lanes.iter().enumerate() {
            if let Some(edge_index) = lane.edge_ids.iter().position(|item| item == edge_id) {
                return format!(
                    "roadSections[{section_index}].lanes[{lane_index}].edgeIds[{edge_index}]"
                );
            }
        }
    }
    "roadSections".to_owned()
}

fn section_lane_group_value_path(wire: &WirePackage, group_id: &str) -> String {
    for (section_index, section) in wire.road_sections.iter().enumerate() {
        for (lane_index, lane) in section.lanes.iter().enumerate() {
            if lane.lane_group_id.as_deref() == Some(group_id) {
                return format!("roadSections[{section_index}].lanes[{lane_index}].laneGroupId");
            }
        }
    }
    "roadSections".to_owned()
}

fn lane_group_path(wire: &WirePackage, group_id: &str, suffix: &str) -> String {
    wire.lane_groups
        .iter()
        .position(|item| item.id == group_id)
        .map_or_else(
            || "laneGroups".to_owned(),
            |index| format!("laneGroups[{index}]{suffix}"),
        )
}

fn corridor_path(wire: &WirePackage, corridor_id: &str, suffix: &str) -> String {
    wire.road_corridors
        .iter()
        .position(|item| item.id == corridor_id)
        .map_or_else(
            || "roadCorridors".to_owned(),
            |index| format!("roadCorridors[{index}]{suffix}"),
        )
}

fn corridor_element_index(
    corridor: &wire::WireRoadCorridor,
    element_id: &str,
    duplicate: bool,
) -> Option<usize> {
    let matches = |element: &WireCorridorElement| {
        let id = match element {
            WireCorridorElement::Section(section) => &section.section_id,
            WireCorridorElement::Band(band) => &band.band_id,
        };
        id == element_id
    };
    if duplicate {
        second_matching_index(&corridor.elements, matches)
    } else {
        corridor.elements.iter().position(matches)
    }
}

fn corridor_element_path(
    wire: &WirePackage,
    corridor_id: &str,
    element_id: &str,
    duplicate: bool,
) -> String {
    let Some(corridor_index) = wire
        .road_corridors
        .iter()
        .position(|item| item.id == corridor_id)
    else {
        return "roadCorridors".to_owned();
    };
    corridor_element_index(&wire.road_corridors[corridor_index], element_id, duplicate).map_or_else(
        || format!("roadCorridors[{corridor_index}].elements"),
        |element_index| format!("roadCorridors[{corridor_index}].elements[{element_index}]"),
    )
}

fn corridor_element_value_path(wire: &WirePackage, element_id: &str) -> String {
    for (corridor_index, corridor) in wire.road_corridors.iter().enumerate() {
        if let Some(element_index) = corridor_element_index(corridor, element_id, false) {
            let field = match &corridor.elements[element_index] {
                WireCorridorElement::Section(_) => "sectionId",
                WireCorridorElement::Band(_) => "bandId",
            };
            return format!("roadCorridors[{corridor_index}].elements[{element_index}].{field}");
        }
    }
    "roadCorridors".to_owned()
}

fn access_error_path(wire: &WirePackage, source: &CoreError) -> String {
    match source {
        CoreError::InvalidExternalId {
            field, external_id, ..
        } => match *field {
            "accessRules[].id" => {
                item_id_path(&wire.access_rules, "accessRules", external_id, |item| {
                    &item.id
                })
            }
            "accessRules[].target.id" => wire
                .access_rules
                .iter()
                .position(|item| item.target.id == *external_id)
                .map_or_else(
                    || "accessRules".to_owned(),
                    |index| format!("accessRules[{index}].target.id"),
                ),
            "accessRules[].participantClassIds[]" => {
                for (rule_index, rule) in wire.access_rules.iter().enumerate() {
                    if let Some(class_index) = rule
                        .participant_class_ids
                        .iter()
                        .position(|item| item == external_id)
                    {
                        return format!(
                            "accessRules[{rule_index}].participantClassIds[{class_index}]"
                        );
                    }
                }
                "accessRules".to_owned()
            }
            _ => "accessRules".to_owned(),
        },
        CoreError::DuplicateAccessRuleId { rule_id } => {
            second_matching_index(&wire.access_rules, |item| item.id == *rule_id).map_or_else(
                || "accessRules".to_owned(),
                |index| format!("accessRules[{index}].id"),
            )
        }
        CoreError::UnknownAccessRuleTarget { rule_id, .. } => {
            access_rule_path(wire, rule_id, ".target.id")
        }
        CoreError::EmptyAccessRuleParticipantClasses { rule_id } => {
            access_rule_path(wire, rule_id, ".participantClassIds")
        }
        CoreError::UnknownAccessRuleParticipantClass { rule_id, class_id } => {
            let Some(rule_index) = access_rule_index(wire, rule_id) else {
                return "accessRules".to_owned();
            };
            wire.access_rules[rule_index]
                .participant_class_ids
                .iter()
                .position(|item| item == class_id)
                .map_or_else(
                    || format!("accessRules[{rule_index}].participantClassIds"),
                    |class_index| {
                        format!("accessRules[{rule_index}].participantClassIds[{class_index}]")
                    },
                )
        }
        CoreError::AccessCapabilityUnavailable {
            rule_id,
            capability,
        } => match *capability {
            "timeWindows" => access_rule_path(wire, rule_id, ".timeWindows"),
            _ => access_rule_path(wire, rule_id, ".target.id"),
        },
        CoreError::AccessRegulationMismatch {
            duplicate_rule_id, ..
        } => access_rule_path(wire, duplicate_rule_id, ".regulation"),
        CoreError::AccessRuleAmbiguity { second_rule_id, .. } => {
            access_rule_path(wire, second_rule_id, "")
        }
        _ => "accessRules".to_owned(),
    }
}

fn access_rule_index(wire: &WirePackage, rule_id: &str) -> Option<usize> {
    wire.access_rules.iter().position(|item| item.id == rule_id)
}

fn access_rule_path(wire: &WirePackage, rule_id: &str, suffix: &str) -> String {
    access_rule_index(wire, rule_id).map_or_else(
        || "accessRules".to_owned(),
        |index| format!("accessRules[{index}]{suffix}"),
    )
}

fn junction_error_path(wire: &WirePackage, source: &CoreError) -> String {
    match source {
        CoreError::InvalidExternalId {
            field, external_id, ..
        } => match *field {
            "junctions[].id" => {
                item_id_path(&wire.junctions, "junctions", external_id, |item| &item.id)
            }
            "movements[].id" => {
                item_id_path(&wire.movements, "movements", external_id, |item| &item.id)
            }
            "movements[].junctionId" => wire
                .movements
                .iter()
                .position(|item| item.junction_id == *external_id)
                .map_or_else(
                    || "movements".to_owned(),
                    |index| format!("movements[{index}].junctionId"),
                ),
            "maneuverPaths[].id" => {
                item_id_path(&wire.maneuver_paths, "maneuverPaths", external_id, |item| {
                    &item.id
                })
            }
            "maneuverPaths[].movementId" => wire
                .maneuver_paths
                .iter()
                .position(|item| item.movement_id == *external_id)
                .map_or_else(
                    || "maneuverPaths".to_owned(),
                    |index| format!("maneuverPaths[{index}].movementId"),
                ),
            "maneuverPaths[].entryEdgeId" => {
                maneuver_path_edge_value_path(wire, "entry", external_id)
            }
            "maneuverPaths[].internalEdgeIds[]" => {
                maneuver_path_edge_value_path(wire, "internal", external_id)
            }
            "maneuverPaths[].exitEdgeId" => {
                maneuver_path_edge_value_path(wire, "exit", external_id)
            }
            _ => "junctions".to_owned(),
        },
        CoreError::DuplicateJunctionId { junction_id } => {
            second_matching_index(&wire.junctions, |item| item.id == *junction_id).map_or_else(
                || "junctions".to_owned(),
                |index| format!("junctions[{index}].id"),
            )
        }
        CoreError::DuplicateMovementId { movement_id } => {
            second_matching_index(&wire.movements, |item| item.id == *movement_id).map_or_else(
                || "movements".to_owned(),
                |index| format!("movements[{index}].id"),
            )
        }
        CoreError::UnknownMovementJunction { movement_id, .. } => {
            movement_path(wire, movement_id, ".junctionId")
        }
        CoreError::DuplicateManeuverPathId { maneuver_path_id } => {
            second_matching_index(&wire.maneuver_paths, |item| item.id == *maneuver_path_id)
                .map_or_else(
                    || "maneuverPaths".to_owned(),
                    |index| format!("maneuverPaths[{index}].id"),
                )
        }
        CoreError::UnknownManeuverPathMovement {
            maneuver_path_id, ..
        } => maneuver_path_path(wire, maneuver_path_id, ".movementId"),
        CoreError::UnknownManeuverPathEdge {
            maneuver_path_id,
            role,
            edge_id,
        } => maneuver_path_edge_path(wire, maneuver_path_id, role, edge_id),
        CoreError::DisconnectedManeuverPath {
            maneuver_path_id,
            transition_index,
            ..
        } => maneuver_path_transition_target_path(wire, maneuver_path_id, *transition_index),
        CoreError::DuplicateManeuverPathSequence {
            duplicate_maneuver_path_id,
            ..
        } => maneuver_path_path(wire, duplicate_maneuver_path_id, ""),
        CoreError::ManeuverInternalEdgeJunctionConflict {
            edge_id,
            duplicate_junction_id,
            ..
        } => wire
            .maneuver_paths
            .iter()
            .enumerate()
            .find_map(|(path_index, path)| {
                let movement = wire
                    .movements
                    .iter()
                    .find(|movement| movement.id == path.movement_id)?;
                (movement.junction_id == *duplicate_junction_id)
                    .then(|| {
                        path.internal_edge_ids
                            .iter()
                            .position(|item| item == edge_id)
                            .map(|edge_index| {
                                format!("maneuverPaths[{path_index}].internalEdgeIds[{edge_index}]")
                            })
                    })
                    .flatten()
            })
            .unwrap_or_else(|| "maneuverPaths".to_owned()),
        CoreError::ManeuverPathEdgeRoleConflict {
            internal_maneuver_path_id,
            boundary_maneuver_path_id,
            edge_id,
            ..
        } => maneuver_path_edge_role_conflict_path(
            wire,
            internal_maneuver_path_id,
            boundary_maneuver_path_id,
            edge_id,
        ),
        CoreError::EmptyJunction { junction_id } => wire
            .junctions
            .iter()
            .position(|item| item.id == *junction_id)
            .map_or_else(
                || "junctions".to_owned(),
                |index| format!("junctions[{index}]"),
            ),
        CoreError::EmptyMovement { movement_id } => movement_path(wire, movement_id, ""),
        CoreError::StaticDomainCapacityExceeded { domain, .. } => match *domain {
            "junctions" => "junctions".to_owned(),
            "movements" => "movements".to_owned(),
            "maneuverPaths" | "maneuverPathEdgeRefs" => "maneuverPaths".to_owned(),
            _ => "junctions".to_owned(),
        },
        _ => "junctions".to_owned(),
    }
}

fn item_id_path<T>(
    items: &[T],
    root: &str,
    external_id: &str,
    id: impl Fn(&T) -> &String,
) -> String {
    items
        .iter()
        .position(|item| id(item) == external_id)
        .map_or_else(|| root.to_owned(), |index| format!("{root}[{index}].id"))
}

fn movement_path(wire: &WirePackage, movement_id: &str, suffix: &str) -> String {
    wire.movements
        .iter()
        .position(|item| item.id == movement_id)
        .map_or_else(
            || "movements".to_owned(),
            |index| format!("movements[{index}]{suffix}"),
        )
}

fn maneuver_path_path(wire: &WirePackage, maneuver_path_id: &str, suffix: &str) -> String {
    wire.maneuver_paths
        .iter()
        .position(|item| item.id == maneuver_path_id)
        .map_or_else(
            || "maneuverPaths".to_owned(),
            |index| format!("maneuverPaths[{index}]{suffix}"),
        )
}

fn maneuver_path_edge_path(
    wire: &WirePackage,
    maneuver_path_id: &str,
    role: &str,
    edge_id: &str,
) -> String {
    let Some(path_index) = wire
        .maneuver_paths
        .iter()
        .position(|item| item.id == maneuver_path_id)
    else {
        return "maneuverPaths".to_owned();
    };
    let path = &wire.maneuver_paths[path_index];
    match role {
        "entry" => format!("maneuverPaths[{path_index}].entryEdgeId"),
        "exit" => format!("maneuverPaths[{path_index}].exitEdgeId"),
        "internal" => path
            .internal_edge_ids
            .iter()
            .position(|item| item == edge_id)
            .map_or_else(
                || format!("maneuverPaths[{path_index}].internalEdgeIds"),
                |index| format!("maneuverPaths[{path_index}].internalEdgeIds[{index}]"),
            ),
        _ => format!("maneuverPaths[{path_index}]"),
    }
}

fn maneuver_path_edge_role_conflict_path(
    wire: &WirePackage,
    internal_maneuver_path_id: &str,
    boundary_maneuver_path_id: &str,
    edge_id: &str,
) -> String {
    let internal_index = wire
        .maneuver_paths
        .iter()
        .position(|item| item.id == internal_maneuver_path_id);
    let boundary_index = wire
        .maneuver_paths
        .iter()
        .position(|item| item.id == boundary_maneuver_path_id);

    if let (Some(internal), Some(boundary)) = (internal_index, boundary_index)
        && boundary > internal
    {
        let path = &wire.maneuver_paths[boundary];
        if path.entry_edge_id == edge_id {
            return format!("maneuverPaths[{boundary}].entryEdgeId");
        }
        if path.exit_edge_id == edge_id {
            return format!("maneuverPaths[{boundary}].exitEdgeId");
        }
        return format!("maneuverPaths[{boundary}]");
    }

    maneuver_path_edge_path(wire, internal_maneuver_path_id, "internal", edge_id)
}

fn maneuver_path_edge_value_path(wire: &WirePackage, role: &str, edge_id: &str) -> String {
    for (path_index, path) in wire.maneuver_paths.iter().enumerate() {
        match role {
            "entry" if path.entry_edge_id == edge_id => {
                return format!("maneuverPaths[{path_index}].entryEdgeId");
            }
            "internal" => {
                if let Some(edge_index) = path
                    .internal_edge_ids
                    .iter()
                    .position(|item| item == edge_id)
                {
                    return format!("maneuverPaths[{path_index}].internalEdgeIds[{edge_index}]");
                }
            }
            "exit" if path.exit_edge_id == edge_id => {
                return format!("maneuverPaths[{path_index}].exitEdgeId");
            }
            _ => {}
        }
    }
    "maneuverPaths".to_owned()
}

fn maneuver_path_transition_target_path(
    wire: &WirePackage,
    maneuver_path_id: &str,
    transition_index: usize,
) -> String {
    let Some(path_index) = wire
        .maneuver_paths
        .iter()
        .position(|item| item.id == maneuver_path_id)
    else {
        return "maneuverPaths".to_owned();
    };
    let path = &wire.maneuver_paths[path_index];
    if transition_index < path.internal_edge_ids.len() {
        format!("maneuverPaths[{path_index}].internalEdgeIds[{transition_index}]")
    } else {
        format!("maneuverPaths[{path_index}].exitEdgeId")
    }
}

fn signal_error_path(wire: &WireSignals, source: &CoreError) -> String {
    match source {
        CoreError::InvalidExternalId {
            field, external_id, ..
        } => signal_external_id_path(wire, field, external_id),
        CoreError::DuplicateStopLineId { stop_line_id } => {
            second_matching_index(&wire.stop_lines, |item| item.id == *stop_line_id).map_or_else(
                || "signals.stopLines".to_owned(),
                |index| format!("signals.stopLines[{index}].id"),
            )
        }
        CoreError::UnknownStopLineEdge { stop_line_id, .. } => wire
            .stop_lines
            .iter()
            .position(|item| item.id == *stop_line_id)
            .map_or_else(
                || "signals.stopLines".to_owned(),
                |index| format!("signals.stopLines[{index}].edgeId"),
            ),
        CoreError::OrphanStopLine { stop_line_id, .. }
        | CoreError::MissingManeuverPathCoverage { stop_line_id, .. }
        | CoreError::MissingManeuverGateCoverage { stop_line_id, .. } => wire
            .stop_lines
            .iter()
            .position(|item| item.id == *stop_line_id)
            .map_or_else(
                || "signals.stopLines".to_owned(),
                |index| format!("signals.stopLines[{index}]"),
            ),
        CoreError::DuplicateStopLineEdge {
            duplicate_stop_line_id,
            ..
        } => wire
            .stop_lines
            .iter()
            .position(|item| item.id == *duplicate_stop_line_id)
            .map_or_else(
                || "signals.stopLines".to_owned(),
                |index| format!("signals.stopLines[{index}].edgeId"),
            ),
        CoreError::DuplicateSignalGroupId { group_id } => {
            second_matching_index(&wire.groups, |item| item.id == *group_id).map_or_else(
                || "signals.groups".to_owned(),
                |index| format!("signals.groups[{index}].id"),
            )
        }
        CoreError::UnownedSignalGroup { group_id } | CoreError::UnusedSignalGroup { group_id } => {
            wire.groups
                .iter()
                .position(|item| item.id == *group_id)
                .map_or_else(
                    || "signals.groups".to_owned(),
                    |index| format!("signals.groups[{index}]"),
                )
        }
        CoreError::DuplicateSignalControllerId { controller_id } => {
            controller_path(wire, controller_id, true, ".id")
        }
        CoreError::EmptySignalControllerGroups { controller_id } => {
            controller_path(wire, controller_id, false, ".groupIds")
        }
        CoreError::EmptySignalControllerPhases { controller_id } => {
            controller_path(wire, controller_id, false, ".phases")
        }
        CoreError::SignalCycleDurationOverflow { controller_id, .. } => {
            controller_path(wire, controller_id, false, ".phases")
        }
        CoreError::InvalidSignalControllerOffset { controller_id, .. }
        | CoreError::SignalControllerOffsetOutOfRange { controller_id, .. } => {
            controller_path(wire, controller_id, false, ".offsetMs")
        }
        CoreError::DuplicateSignalControllerGroup {
            controller_id,
            group_id,
        } => controller_group_path(wire, controller_id, group_id, true),
        CoreError::UnknownSignalControllerGroup {
            controller_id,
            group_id,
        } => controller_group_path(wire, controller_id, group_id, false),
        CoreError::SignalGroupMultipleControllers {
            duplicate_controller_id,
            group_id,
            ..
        } => controller_group_path(wire, duplicate_controller_id, group_id, false),
        CoreError::DuplicateSignalPhaseId {
            controller_id,
            phase_id,
        } => phase_path(wire, controller_id, phase_id, true, ".id"),
        CoreError::InvalidSignalPhaseDuration {
            controller_id,
            phase_id,
            ..
        } => phase_path(wire, controller_id, phase_id, false, ".durationMs"),
        CoreError::MissingSignalPhaseGroup {
            controller_id,
            phase_id,
            ..
        } => phase_path(wire, controller_id, phase_id, false, ".states"),
        CoreError::UnknownSignalPhaseGroup {
            controller_id,
            phase_id,
            group_id,
        } => state_path(wire, controller_id, phase_id, group_id, false),
        CoreError::DuplicateSignalPhaseGroup {
            controller_id,
            phase_id,
            group_id,
        } => state_path(wire, controller_id, phase_id, group_id, true),
        CoreError::DuplicateManeuverGateId { maneuver_gate_id } => {
            second_matching_index(&wire.maneuver_gates, |item| item.id == *maneuver_gate_id)
                .map_or_else(
                    || "signals.maneuverGates".to_owned(),
                    |index| format!("signals.maneuverGates[{index}].id"),
                )
        }
        CoreError::UnknownManeuverGatePath {
            maneuver_gate_id, ..
        } => gate_id_path(wire, maneuver_gate_id, ".maneuverPathId"),
        CoreError::ManeuverGateTransitionOutOfRange {
            maneuver_gate_id, ..
        }
        | CoreError::UnsupportedManeuverGateTransition {
            maneuver_gate_id, ..
        } => gate_id_path(wire, maneuver_gate_id, ".transitionIndex"),
        CoreError::DuplicateManeuverGatePathTransition {
            duplicate_maneuver_gate_id,
            ..
        } => gate_id_path(wire, duplicate_maneuver_gate_id, ".maneuverPathId"),
        CoreError::UnknownManeuverGateStopLine {
            maneuver_gate_id, ..
        }
        | CoreError::ManeuverGateStopLineMismatch {
            maneuver_gate_id, ..
        } => gate_id_path(wire, maneuver_gate_id, ".stopLineId"),
        CoreError::UnknownManeuverGateSignalGroup {
            maneuver_gate_id, ..
        } => gate_id_path(wire, maneuver_gate_id, ".signalControl.groupId"),
        _ => "signals".to_owned(),
    }
}

fn signal_external_id_path(wire: &WireSignals, field: &str, external_id: &str) -> String {
    match field {
        "signals.stopLines[].id" => wire
            .stop_lines
            .iter()
            .position(|item| item.id == external_id)
            .map_or_else(
                || "signals.stopLines".to_owned(),
                |index| format!("signals.stopLines[{index}].id"),
            ),
        "signals.stopLines[].edgeId" => wire
            .stop_lines
            .iter()
            .position(|item| item.edge_id == external_id)
            .map_or_else(
                || "signals.stopLines".to_owned(),
                |index| format!("signals.stopLines[{index}].edgeId"),
            ),
        "signals.groups[].id" => wire
            .groups
            .iter()
            .position(|item| item.id == external_id)
            .map_or_else(
                || "signals.groups".to_owned(),
                |index| format!("signals.groups[{index}].id"),
            ),
        "signals.controllers[].id" => wire
            .controllers
            .iter()
            .position(|item| item.id == external_id)
            .map_or_else(
                || "signals.controllers".to_owned(),
                |index| format!("signals.controllers[{index}].id"),
            ),
        "signals.controllers[].groupIds[]" => {
            for (controller_index, controller) in wire.controllers.iter().enumerate() {
                if let Some(group_index) = controller
                    .group_ids
                    .iter()
                    .position(|item| item == external_id)
                {
                    return format!(
                        "signals.controllers[{controller_index}].groupIds[{group_index}]"
                    );
                }
            }
            "signals.controllers".to_owned()
        }
        "signals.controllers[].phases[].id" => {
            for (controller_index, controller) in wire.controllers.iter().enumerate() {
                if let Some(phase_index) = controller
                    .phases
                    .iter()
                    .position(|item| item.id == external_id)
                {
                    return format!(
                        "signals.controllers[{controller_index}].phases[{phase_index}].id"
                    );
                }
            }
            "signals.controllers".to_owned()
        }
        "signals.controllers[].phases[].states[].groupId" => {
            for (controller_index, controller) in wire.controllers.iter().enumerate() {
                for (phase_index, phase) in controller.phases.iter().enumerate() {
                    if let Some(state_index) = phase
                        .states
                        .iter()
                        .position(|item| item.group_id == external_id)
                    {
                        return format!(
                            "signals.controllers[{controller_index}].phases[{phase_index}].states[{state_index}].groupId"
                        );
                    }
                }
            }
            "signals.controllers".to_owned()
        }
        "signals.maneuverGates[].id" => gate_path(wire, |gate| gate.id == external_id, ".id"),
        "signals.maneuverGates[].maneuverPathId" => gate_path(
            wire,
            |gate| gate.maneuver_path_id == external_id,
            ".maneuverPathId",
        ),
        "signals.maneuverGates[].stopLineId" => {
            gate_path(wire, |gate| gate.stop_line_id == external_id, ".stopLineId")
        }
        "signals.maneuverGates[].signalControl.groupId" => gate_path(
            wire,
            |gate| {
                matches!(
                    &gate.signal_control,
                    WireSignalControl::Group(control) if control.group_id == external_id
                )
            },
            ".signalControl.groupId",
        ),
        _ => "signals".to_owned(),
    }
}

fn controller_path(
    wire: &WireSignals,
    controller_id: &str,
    duplicate: bool,
    suffix: &str,
) -> String {
    let index = if duplicate {
        second_matching_index(&wire.controllers, |controller| {
            controller.id == controller_id
        })
    } else {
        wire.controllers
            .iter()
            .position(|controller| controller.id == controller_id)
    };
    index.map_or_else(
        || "signals.controllers".to_owned(),
        |index| format!("signals.controllers[{index}]{suffix}"),
    )
}

fn controller_group_path(
    wire: &WireSignals,
    controller_id: &str,
    group_id: &str,
    duplicate: bool,
) -> String {
    let Some(controller_index) = wire
        .controllers
        .iter()
        .position(|controller| controller.id == controller_id)
    else {
        return "signals.controllers".to_owned();
    };
    let group_ids = &wire.controllers[controller_index].group_ids;
    let group_index = if duplicate {
        second_matching_index(group_ids, |candidate| candidate == group_id)
    } else {
        group_ids.iter().position(|candidate| candidate == group_id)
    };
    group_index.map_or_else(
        || format!("signals.controllers[{controller_index}].groupIds"),
        |index| format!("signals.controllers[{controller_index}].groupIds[{index}]"),
    )
}

fn phase_path(
    wire: &WireSignals,
    controller_id: &str,
    phase_id: &str,
    duplicate: bool,
    suffix: &str,
) -> String {
    let Some(controller_index) = wire
        .controllers
        .iter()
        .position(|controller| controller.id == controller_id)
    else {
        return "signals.controllers".to_owned();
    };
    let phases = &wire.controllers[controller_index].phases;
    let phase_index = if duplicate {
        second_matching_index(phases, |phase| phase.id == phase_id)
    } else {
        phases.iter().position(|phase| phase.id == phase_id)
    };
    let Some(phase_index) = phase_index else {
        return format!("signals.controllers[{controller_index}].phases");
    };
    format!("signals.controllers[{controller_index}].phases[{phase_index}]{suffix}")
}

fn state_path(
    wire: &WireSignals,
    controller_id: &str,
    phase_id: &str,
    group_id: &str,
    duplicate: bool,
) -> String {
    let Some(controller_index) = wire
        .controllers
        .iter()
        .position(|controller| controller.id == controller_id)
    else {
        return "signals.controllers".to_owned();
    };
    let Some(phase_index) = wire.controllers[controller_index]
        .phases
        .iter()
        .position(|phase| phase.id == phase_id)
    else {
        return format!("signals.controllers[{controller_index}].phases");
    };
    let states = &wire.controllers[controller_index].phases[phase_index].states;
    let state_index = if duplicate {
        second_matching_index(states, |state| state.group_id == group_id)
    } else {
        states.iter().position(|state| state.group_id == group_id)
    };
    state_index.map_or_else(
        || format!("signals.controllers[{controller_index}].phases[{phase_index}].states"),
        |index| {
            format!(
                "signals.controllers[{controller_index}].phases[{phase_index}].states[{index}].groupId"
            )
        },
    )
}

fn gate_path(
    wire: &WireSignals,
    predicate: impl Fn(&wire::WireManeuverGate) -> bool,
    suffix: &str,
) -> String {
    wire.maneuver_gates.iter().position(predicate).map_or_else(
        || "signals.maneuverGates".to_owned(),
        |index| format!("signals.maneuverGates[{index}]{suffix}"),
    )
}

fn gate_id_path(wire: &WireSignals, maneuver_gate_id: &str, suffix: &str) -> String {
    gate_path(wire, |gate| gate.id == maneuver_gate_id, suffix)
}

fn route_input_error_path(index: usize, route: &wire::WireRoute, source: &CoreError) -> String {
    match source {
        CoreError::InvalidExternalId { field, .. } if *field == "routes[].id" => {
            format!("routes[{index}].id")
        }
        CoreError::InvalidExternalId {
            field, external_id, ..
        } if *field == "routes[].edgeIds[]" => route
            .edge_ids
            .iter()
            .position(|item| item == external_id)
            .map_or_else(
                || format!("routes[{index}].edgeIds"),
                |edge_index| format!("routes[{index}].edgeIds[{edge_index}]"),
            ),
        CoreError::EmptyRoute { .. } => format!("routes[{index}].edgeIds"),
        _ => format!("routes[{index}]"),
    }
}

fn initial_traffic_error_path(wire: &WirePackage, source: &CoreError) -> String {
    match source {
        CoreError::DuplicateRouteId { route_id } => {
            second_matching_index(&wire.routes, |route| route.id == *route_id).map_or_else(
                || "routes".to_owned(),
                |index| format!("routes[{index}].id"),
            )
        }
        CoreError::UnknownRouteEdge { route_id, edge_id } => {
            route_edge_path(wire, route_id, |route| {
                route.edge_ids.iter().position(|item| item == edge_id)
            })
        }
        CoreError::DisconnectedRouteEdge {
            route_id,
            from_edge_id,
            to_edge_id,
        } => route_edge_path(wire, route_id, |route| {
            route
                .edge_ids
                .windows(2)
                .position(|pair| pair[0] == *from_edge_id && pair[1] == *to_edge_id)
                .map(|index| index + 1)
        }),
        CoreError::RouteTerminatesAtStopLine { route_id, .. } => {
            route_edge_path(wire, route_id, |route| route.edge_ids.len().checked_sub(1))
        }
        CoreError::RouteStartsInsideJunction { route_id, .. } => {
            route_edge_path(wire, route_id, |_| Some(0))
        }
        CoreError::RouteEndsInsideJunction { route_id, .. } => {
            route_edge_path(wire, route_id, |route| route.edge_ids.len().checked_sub(1))
        }
        CoreError::RouteManeuverNoFullMatch {
            route_id,
            entry_route_edge_index,
            ..
        }
        | CoreError::RouteManeuverMultipleFullMatches {
            route_id,
            entry_route_edge_index,
            ..
        } => route_edge_path(wire, route_id, |_| entry_route_edge_index.checked_add(1)),
        CoreError::RouteManeuverInternalOverlap {
            route_id,
            route_edge_index,
            ..
        }
        | CoreError::RouteInternalEdgeUncovered {
            route_id,
            route_edge_index,
            ..
        } => route_edge_path(wire, route_id, |_| Some(*route_edge_index)),
        _ => "initialTrafficData".to_owned(),
    }
}

fn route_edge_path(
    wire: &WirePackage,
    route_id: &str,
    find_edge: impl Fn(&wire::WireRoute) -> Option<usize>,
) -> String {
    let Some(route_index) = wire.routes.iter().position(|route| route.id == route_id) else {
        return "routes".to_owned();
    };
    find_edge(&wire.routes[route_index]).map_or_else(
        || format!("routes[{route_index}].edgeIds"),
        |edge_index| format!("routes[{route_index}].edgeIds[{edge_index}]"),
    )
}

fn second_matching_index<T>(items: &[T], predicate: impl Fn(&T) -> bool) -> Option<usize> {
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| predicate(item).then_some(index))
        .nth(1)
}

fn validate_unit(
    path: &'static str,
    expected: &'static str,
    actual: &str,
) -> Result<(), DataError> {
    if actual == expected {
        Ok(())
    } else {
        Err(DataError::InvalidUnit {
            path,
            expected,
            actual: actual.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_data_crate_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-data");
    }
}
