#![doc = include_str!("../README.md")]

mod error;
mod wire;

use laneflow_core::{
    CoreError, EdgeLength, IidmProfileSpec, InitialTrafficData, LaneEdge, LaneGraph, MovementGate,
    ParkingAnchorKind, ParkingArea, ParkingRegistry, ParkingSpace, ParkingSpaceGeometry, Route,
    SignalAspect, SignalControlInput, SignalController, SignalGroup, SignalGroupState, SignalPhase,
    SignalRegistry, StopLine, StopLineLocation, VehicleProfile, VehicleProfileRegistry,
};
use serde::de::DeserializeOwned;
use serde_json::error::Category;

pub use error::DataError;

use wire::{
    WirePackage, WireParking, WireSignalAspect, WireSignalControl, WireSignalControllerKind,
    WireSignals, WireStopLineLocation, WireVersionHeader,
};

/// 当前 production loader 接受的唯一 data format 版本。
pub const CURRENT_FORMAT_VERSION: &str = "0.5";

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

    let profile_registry = normalize_profiles(wire)?;
    let lane_graph = normalize_lane_graph(wire)?;
    let signals = normalize_signals(&lane_graph, &wire.signals)?;
    let parking = normalize_parking(&lane_graph, &wire.parking)?;
    let routes = normalize_routes(wire)?;

    let initial_traffic_data = InitialTrafficData::try_new_with_signals_and_parking(
        lane_graph,
        routes,
        profile_registry,
        signals,
        parking,
    )
    .map_err(|source| DataError::core(initial_traffic_error_path(wire, &source), source))?;
    Ok(LoadedPackage {
        initial_traffic_data,
    })
}

fn normalize_profiles(wire: &WirePackage) -> Result<VehicleProfileRegistry, DataError> {
    let mut normalized_profiles = Vec::with_capacity(wire.vehicle_profiles.len());
    for (index, profile) in wire.vehicle_profiles.iter().enumerate() {
        if profile.model != "iidm" {
            return Err(DataError::UnsupportedVehicleProfileModel {
                path: format!("vehicleProfiles[{index}].model"),
                profile_id: profile.id.clone(),
                actual: profile.model.clone(),
            });
        }

        let spec = IidmProfileSpec {
            length: profile.length,
            desired_speed: profile.desired_speed,
            min_gap: profile.min_gap,
            time_headway: profile.time_headway,
            max_acceleration: profile.max_acceleration,
            comfortable_deceleration: profile.comfortable_deceleration,
            emergency_deceleration: profile.emergency_deceleration,
        };
        let normalized = VehicleProfile::try_new_iidm(profile.id.clone(), spec)
            .map_err(|source| DataError::core(format!("vehicleProfiles[{index}]"), source))?;
        normalized_profiles.push(normalized);
    }
    VehicleProfileRegistry::try_new(normalized_profiles)
        .map_err(|source| DataError::core("vehicleProfiles", source))
}

fn normalize_lane_graph(wire: &WirePackage) -> Result<LaneGraph, DataError> {
    let mut edges = Vec::with_capacity(wire.lane_graph.edges.len());
    for (index, edge) in wire.lane_graph.edges.iter().enumerate() {
        let length = EdgeLength::try_new(edge.length).map_err(|source| {
            DataError::core(format!("laneGraph.edges[{index}].length"), source)
        })?;
        edges.push(LaneEdge::new(
            edge.id.clone(),
            length,
            edge.connections
                .iter()
                .map(|connection| connection.to_edge_id.clone()),
        ));
    }
    LaneGraph::try_new(edges).map_err(|source| DataError::core("laneGraph.edges", source))
}

fn normalize_signals(
    lane_graph: &LaneGraph,
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

    let mut movement_gates = Vec::with_capacity(wire.movement_gates.len());
    for gate in &wire.movement_gates {
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
        movement_gates.push(MovementGate::new(
            gate.from_edge_id.clone(),
            gate.to_edge_id.clone(),
            gate.stop_line_id.clone(),
            control,
        ));
    }

    SignalRegistry::try_new(lane_graph, stop_lines, groups, controllers, movement_gates)
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
        | CoreError::MissingMovementGateCoverage { stop_line_id, .. } => wire
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
        CoreError::DuplicateMovementGate {
            from_edge_id,
            to_edge_id,
        } => second_matching_index(&wire.movement_gates, |item| {
            item.from_edge_id == *from_edge_id && item.to_edge_id == *to_edge_id
        })
        .map_or_else(
            || "signals.movementGates".to_owned(),
            |index| format!("signals.movementGates[{index}]"),
        ),
        CoreError::UnknownMovementGateFromEdge { edge_id } => {
            gate_path(wire, |gate| gate.from_edge_id == *edge_id, ".fromEdgeId")
        }
        CoreError::UnknownMovementGateToEdge { edge_id } => {
            gate_path(wire, |gate| gate.to_edge_id == *edge_id, ".toEdgeId")
        }
        CoreError::DisconnectedMovementGate {
            from_edge_id,
            to_edge_id,
        } => gate_path(
            wire,
            |gate| gate.from_edge_id == *from_edge_id && gate.to_edge_id == *to_edge_id,
            "",
        ),
        CoreError::UnknownMovementGateStopLine { stop_line_id } => gate_path(
            wire,
            |gate| gate.stop_line_id == *stop_line_id,
            ".stopLineId",
        ),
        CoreError::MovementGateStopLineMismatch {
            stop_line_id,
            from_edge_id,
            ..
        } => gate_path(
            wire,
            |gate| gate.stop_line_id == *stop_line_id && gate.from_edge_id == *from_edge_id,
            ".stopLineId",
        ),
        CoreError::UnknownMovementGateSignalGroup { group_id } => gate_path(
            wire,
            |gate| matches!(&gate.signal_control, WireSignalControl::Group(control) if control.group_id == *group_id),
            ".signalControl.groupId",
        ),
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
        "signals.movementGates[].fromEdgeId" => {
            gate_path(wire, |gate| gate.from_edge_id == external_id, ".fromEdgeId")
        }
        "signals.movementGates[].toEdgeId" => {
            gate_path(wire, |gate| gate.to_edge_id == external_id, ".toEdgeId")
        }
        "signals.movementGates[].stopLineId" => {
            gate_path(wire, |gate| gate.stop_line_id == external_id, ".stopLineId")
        }
        "signals.movementGates[].signalControl.groupId" => gate_path(
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
    predicate: impl Fn(&wire::WireMovementGate) -> bool,
    suffix: &str,
) -> String {
    wire.movement_gates.iter().position(predicate).map_or_else(
        || "signals.movementGates".to_owned(),
        |index| format!("signals.movementGates[{index}]{suffix}"),
    )
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
