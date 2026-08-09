//! 当前 v0.10 Traffic package 的单遍解码。

use serde_json::value::RawValue;

use super::walk::{self, Ctx, LocationPolicy, ShapeCandidate};
use super::{ByteRange, GateReport, ParseFailure, RootGate, missing_root_field};
use crate::CURRENT_TRAFFIC_FORMAT_VERSION;
use crate::wire::{
    WireAccessEffect, WireAccessRule, WireAccessTarget, WireAccessTargetKind, WireCorridorElement,
    WireFacilityBand, WireGroupSignalControl, WireGroupSignalControlKind, WireJunction,
    WireLaneConnection, WireLaneEdge, WireLaneGraph, WireLaneGroup, WireManeuverGate,
    WireManeuverPath, WireMovement, WireNoneSignalControl, WireNoneSignalControlKind, WirePackage,
    WireParking, WireParkingAnchor, WireParkingArea, WireParkingGeometry, WireParkingSpace,
    WireParticipantClass, WireRegulation, WireRoadCorridor, WireRoadSection, WireRoute,
    WireSectionLane, WireSignalAspect, WireSignalControl, WireSignalController,
    WireSignalControllerKind, WireSignalGroup, WireSignalGroupState, WireSignalPhase, WireSignals,
    WireStopLine, WireStopLineLocation, WireUnits, WireVehicleProfile, WireWaitingZone,
};

const PACKAGE_FIELDS: &[&str] = &[
    "formatVersion",
    "units",
    "laneGraph",
    "junctions",
    "movements",
    "maneuverPaths",
    "routes",
    "vehicleProfiles",
    "participantClasses",
    "facilityBands",
    "roadSections",
    "laneGroups",
    "roadCorridors",
    "accessRules",
    "waitingZones",
    "signals",
    "parking",
    "extensions",
];
const UNITS_FIELDS: &[&str] = &["distance", "time"];
const LANE_GRAPH_FIELDS: &[&str] = &["edges"];
const LANE_EDGE_FIELDS: &[&str] = &["id", "length", "speedLimit", "connections"];
const LANE_CONNECTION_FIELDS: &[&str] = &["toEdgeId"];
const JUNCTION_FIELDS: &[&str] = &["id"];
const MOVEMENT_FIELDS: &[&str] = &["id", "junctionId"];
const MANEUVER_PATH_FIELDS: &[&str] = &[
    "id",
    "movementId",
    "entryEdgeId",
    "internalEdgeIds",
    "exitEdgeId",
];
const ROUTE_FIELDS: &[&str] = &["id", "edgeIds"];
const VEHICLE_PROFILE_FIELDS: &[&str] = &[
    "id",
    "length",
    "model",
    "desiredSpeed",
    "minGap",
    "timeHeadway",
    "maxAcceleration",
    "comfortableDeceleration",
    "emergencyDeceleration",
    "participantClassId",
];
const PARTICIPANT_CLASS_FIELDS: &[&str] = &["id", "extendsId"];
const FACILITY_BAND_FIELDS: &[&str] = &["id", "kindId"];
const ROAD_SECTION_FIELDS: &[&str] = &["id", "kindId", "lanes"];
const SECTION_LANE_FIELDS: &[&str] = &["edgeIds", "laneGroupId"];
const LANE_GROUP_FIELDS: &[&str] = &["id", "roadSectionId"];
const ROAD_CORRIDOR_FIELDS: &[&str] = &["id", "referenceSectionId", "elements"];
const ACCESS_RULE_FIELDS: &[&str] = &[
    "id",
    "target",
    "effect",
    "participantClassIds",
    "timeWindows",
    "regulation",
    "priority",
];
const ACCESS_TARGET_FIELDS: &[&str] = &["kind", "id"];
const WAITING_ZONE_FIELDS: &[&str] = &[
    "id",
    "maneuverPathId",
    "entryGateId",
    "releaseGateId",
    "maxOccupancy",
];
const REGULATION_FIELDS: &[&str] = &["jurisdiction", "version", "source"];
const PARKING_FIELDS: &[&str] = &["areas", "spaces"];
const PARKING_AREA_FIELDS: &[&str] = &["id"];
const PARKING_SPACE_FIELDS: &[&str] = &["id", "areaId", "entry", "exit", "geometry"];
const PARKING_ANCHOR_FIELDS: &[&str] = &["edgeId", "progress"];
const PARKING_GEOMETRY_FIELDS: &[&str] =
    &["lateralOffset", "headingOffsetRadians", "length", "width"];
const SIGNALS_FIELDS: &[&str] = &["stopLines", "maneuverGates", "groups", "controllers"];
const STOP_LINE_FIELDS: &[&str] = &["id", "edgeId", "location"];
const MANEUVER_GATE_FIELDS: &[&str] = &[
    "id",
    "maneuverPathId",
    "transitionIndex",
    "stopLineId",
    "signalControl",
];
const SIGNAL_GROUP_FIELDS: &[&str] = &["id"];
const SIGNAL_CONTROLLER_FIELDS: &[&str] = &["id", "kind", "offsetMs", "groupIds", "phases"];
const SIGNAL_PHASE_FIELDS: &[&str] = &["id", "durationMs", "states"];
const SIGNAL_GROUP_STATE_FIELDS: &[&str] = &["groupId", "aspect"];

const ACCESS_TARGET_KIND_VARIANTS: &[&str] = &[
    "laneEdge",
    "laneGroup",
    "roadSection",
    "maneuverPath",
    "facilityBand",
];
const ACCESS_TARGET_KIND_TABLE: &[(&str, WireAccessTargetKind)] = &[
    ("laneEdge", WireAccessTargetKind::LaneEdge),
    ("laneGroup", WireAccessTargetKind::LaneGroup),
    ("roadSection", WireAccessTargetKind::RoadSection),
    ("maneuverPath", WireAccessTargetKind::ManeuverPath),
    ("facilityBand", WireAccessTargetKind::FacilityBand),
];
const ACCESS_EFFECT_VARIANTS: &[&str] = &["allow", "deny"];
const ACCESS_EFFECT_TABLE: &[(&str, WireAccessEffect)] = &[
    ("allow", WireAccessEffect::Allow),
    ("deny", WireAccessEffect::Deny),
];
const STOP_LINE_LOCATION_VARIANTS: &[&str] = &["edgeEnd"];
const STOP_LINE_LOCATION_TABLE: &[(&str, WireStopLineLocation)] =
    &[("edgeEnd", WireStopLineLocation::EdgeEnd)];
const SIGNAL_CONTROLLER_KIND_VARIANTS: &[&str] = &["fixedTime"];
const SIGNAL_CONTROLLER_KIND_TABLE: &[(&str, WireSignalControllerKind)] =
    &[("fixedTime", WireSignalControllerKind::FixedTime)];
const SIGNAL_ASPECT_VARIANTS: &[&str] = &["red", "yellow", "green"];
const SIGNAL_ASPECT_TABLE: &[(&str, WireSignalAspect)] = &[
    ("red", WireSignalAspect::Red),
    ("yellow", WireSignalAspect::Yellow),
    ("green", WireSignalAspect::Green),
];

/// 解析 Traffic package wire（单遍：闸口 + 完整 shape）。
pub(crate) fn parse_traffic(input: &[u8]) -> Result<WirePackage, ParseFailure> {
    let mut fields = PackageFields::default();
    let GateReport { gate, root_range } = super::drive_root(
        input,
        CURRENT_TRAFFIC_FORMAT_VERSION,
        |ctx, key, value, range, mark, gate| {
            fields.handle(ctx, key, value, range, mark, gate);
        },
    )?;
    if let Some(candidate) = gate.deferred {
        return Err(ParseFailure::Shape(candidate));
    }
    let format_version = gate.format_version.expect("闸口保证版本字段存在");
    PackageFields::finish(fields, format_version, root_range)
}

#[derive(Default)]
struct PackageFields {
    units: Option<WireUnits>,
    lane_graph: Option<WireLaneGraph>,
    junctions: Option<Vec<WireJunction>>,
    movements: Option<Vec<WireMovement>>,
    maneuver_paths: Option<Vec<WireManeuverPath>>,
    routes: Option<Vec<WireRoute>>,
    vehicle_profiles: Option<Vec<WireVehicleProfile>>,
    participant_classes: Option<Vec<WireParticipantClass>>,
    facility_bands: Option<Vec<WireFacilityBand>>,
    road_sections: Option<Vec<WireRoadSection>>,
    lane_groups: Option<Vec<WireLaneGroup>>,
    road_corridors: Option<Vec<WireRoadCorridor>>,
    access_rules: Option<Vec<WireAccessRule>>,
    waiting_zones: Option<Vec<WireWaitingZone>>,
    signals: Option<WireSignals>,
    parking: Option<WireParking>,
}

impl PackageFields {
    fn handle<'de>(
        &mut self,
        ctx: &mut Ctx<'de, impl LocationPolicy>,
        key: &str,
        value: &'de RawValue,
        range: ByteRange,
        mark: usize,
        gate: &mut RootGate,
    ) {
        let result = match key {
            "units" => walk::set_once(
                ctx,
                &mut self.units,
                "units",
                value,
                range,
                mark,
                decode_units,
            ),
            "laneGraph" => walk::set_once(
                ctx,
                &mut self.lane_graph,
                "laneGraph",
                value,
                range,
                mark,
                decode_lane_graph,
            ),
            "junctions" => walk::set_once(
                ctx,
                &mut self.junctions,
                "junctions",
                value,
                range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_junction),
            ),
            "movements" => walk::set_once(
                ctx,
                &mut self.movements,
                "movements",
                value,
                range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_movement),
            ),
            "maneuverPaths" => walk::set_once(
                ctx,
                &mut self.maneuver_paths,
                "maneuverPaths",
                value,
                range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_maneuver_path),
            ),
            "routes" => walk::set_once(
                ctx,
                &mut self.routes,
                "routes",
                value,
                range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_route),
            ),
            "vehicleProfiles" => walk::set_once(
                ctx,
                &mut self.vehicle_profiles,
                "vehicleProfiles",
                value,
                range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_vehicle_profile),
            ),
            "participantClasses" => walk::set_once(
                ctx,
                &mut self.participant_classes,
                "participantClasses",
                value,
                range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_participant_class),
            ),
            "facilityBands" => walk::set_once(
                ctx,
                &mut self.facility_bands,
                "facilityBands",
                value,
                range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_facility_band),
            ),
            "roadSections" => walk::set_once(
                ctx,
                &mut self.road_sections,
                "roadSections",
                value,
                range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_road_section),
            ),
            "laneGroups" => walk::set_once(
                ctx,
                &mut self.lane_groups,
                "laneGroups",
                value,
                range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_lane_group),
            ),
            "roadCorridors" => walk::set_once(
                ctx,
                &mut self.road_corridors,
                "roadCorridors",
                value,
                range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_road_corridor),
            ),
            "accessRules" => walk::set_once(
                ctx,
                &mut self.access_rules,
                "accessRules",
                value,
                range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_access_rule),
            ),
            "waitingZones" => walk::set_once(
                ctx,
                &mut self.waiting_zones,
                "waitingZones",
                value,
                range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_waiting_zone),
            ),
            "signals" => walk::set_once(
                ctx,
                &mut self.signals,
                "signals",
                value,
                range,
                mark,
                decode_signals,
            ),
            "parking" => walk::set_once(
                ctx,
                &mut self.parking,
                "parking",
                value,
                range,
                mark,
                decode_parking,
            ),
            // 根 extensions：任何合法 object 都接受，内容不透明。
            "extensions" => walk::check_extensions_object(ctx, value, range),
            _ => Err(ctx.candidate(walk::unknown_field_message(key, PACKAGE_FIELDS), range)),
        };
        if let Err(candidate) = result {
            gate.defer(candidate);
        }
    }

    fn finish(
        self,
        format_version: String,
        root_range: ByteRange,
    ) -> Result<WirePackage, ParseFailure> {
        Ok(WirePackage {
            format_version,
            units: self
                .units
                .ok_or_else(|| missing_root_field("units", root_range))?,
            lane_graph: self
                .lane_graph
                .ok_or_else(|| missing_root_field("laneGraph", root_range))?,
            junctions: self
                .junctions
                .ok_or_else(|| missing_root_field("junctions", root_range))?,
            movements: self
                .movements
                .ok_or_else(|| missing_root_field("movements", root_range))?,
            maneuver_paths: self
                .maneuver_paths
                .ok_or_else(|| missing_root_field("maneuverPaths", root_range))?,
            routes: self
                .routes
                .ok_or_else(|| missing_root_field("routes", root_range))?,
            vehicle_profiles: self
                .vehicle_profiles
                .ok_or_else(|| missing_root_field("vehicleProfiles", root_range))?,
            participant_classes: self
                .participant_classes
                .ok_or_else(|| missing_root_field("participantClasses", root_range))?,
            facility_bands: self
                .facility_bands
                .ok_or_else(|| missing_root_field("facilityBands", root_range))?,
            road_sections: self
                .road_sections
                .ok_or_else(|| missing_root_field("roadSections", root_range))?,
            lane_groups: self
                .lane_groups
                .ok_or_else(|| missing_root_field("laneGroups", root_range))?,
            road_corridors: self
                .road_corridors
                .ok_or_else(|| missing_root_field("roadCorridors", root_range))?,
            access_rules: self
                .access_rules
                .ok_or_else(|| missing_root_field("accessRules", root_range))?,
            waiting_zones: self
                .waiting_zones
                .ok_or_else(|| missing_root_field("waitingZones", root_range))?,
            signals: self
                .signals
                .ok_or_else(|| missing_root_field("signals", root_range))?,
            parking: self
                .parking
                .ok_or_else(|| missing_root_field("parking", root_range))?,
        })
    }
}

/// Vec<record> 字段的公共解码：逐元素 replay record token。
fn decode_record_vec<'de, T, L, F>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
    mut decode: F,
) -> Result<Vec<T>, ShapeCandidate>
where
    L: LocationPolicy,
    F: FnMut(&mut Ctx<'de, L>, &'de RawValue, ByteRange) -> Result<T, ShapeCandidate>,
{
    let mut items = Vec::new();
    walk::decode_array(
        ctx,
        token,
        range,
        "a sequence",
        |ctx, _index, element, element_range| {
            items.push(decode(ctx, element, element_range)?);
            Ok(())
        },
    )?;
    Ok(items)
}

/// Vec<String> 字段的公共解码（逐元素解码以保留元素级 path）。
fn decode_string_vec<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<Vec<String>, ShapeCandidate> {
    let mut items = Vec::new();
    walk::decode_array(
        ctx,
        token,
        range,
        "a sequence",
        |ctx, _index, element, element_range| {
            items.push(walk::decode_scalar::<String, L>(
                ctx,
                element,
                element_range,
            )?);
            Ok(())
        },
    )?;
    Ok(items)
}

fn decode_units<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireUnits, ShapeCandidate> {
    let mut distance = None;
    let mut time = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireUnits",
        |ctx, key, value, value_range, mark| match key {
            "distance" => walk::set_once(
                ctx,
                &mut distance,
                "distance",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "time" => walk::set_once(
                ctx,
                &mut time,
                "time",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            _ => Err(ctx.candidate(walk::unknown_field_message(key, UNITS_FIELDS), value_range)),
        },
    )?;
    Ok(WireUnits {
        distance: distance
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("distance"), range))?,
        time: time.ok_or_else(|| ctx.candidate(walk::missing_field_message("time"), range))?,
    })
}

fn decode_lane_graph<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireLaneGraph, ShapeCandidate> {
    let mut edges = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireLaneGraph",
        |ctx, key, value, value_range, mark| match key {
            "edges" => walk::set_once(
                ctx,
                &mut edges,
                "edges",
                value,
                value_range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_lane_edge),
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, LANE_GRAPH_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireLaneGraph {
        edges: edges.ok_or_else(|| ctx.candidate(walk::missing_field_message("edges"), range))?,
    })
}

fn decode_lane_edge<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireLaneEdge, ShapeCandidate> {
    let mut id = None;
    let mut length = None;
    let mut speed_limit = None;
    let mut connections = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireLaneEdge",
        |ctx, key, value, value_range, mark| match key {
            "id" => walk::set_once(
                ctx,
                &mut id,
                "id",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "length" => walk::set_once(
                ctx,
                &mut length,
                "length",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "speedLimit" => walk::set_once(
                ctx,
                &mut speed_limit,
                "speedLimit",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "connections" => walk::set_once(
                ctx,
                &mut connections,
                "connections",
                value,
                value_range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_lane_connection),
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, LANE_EDGE_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireLaneEdge {
        id: id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))?,
        length: length
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("length"), range))?,
        speed_limit: speed_limit
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("speedLimit"), range))?,
        connections: connections
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("connections"), range))?,
    })
}

fn decode_lane_connection<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireLaneConnection, ShapeCandidate> {
    let mut to_edge_id = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireLaneConnection",
        |ctx, key, value, value_range, mark| match key {
            "toEdgeId" => walk::set_once(
                ctx,
                &mut to_edge_id,
                "toEdgeId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, LANE_CONNECTION_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireLaneConnection {
        to_edge_id: to_edge_id
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("toEdgeId"), range))?,
    })
}

fn decode_junction<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireJunction, ShapeCandidate> {
    decode_id_only(ctx, token, range, "struct WireJunction", JUNCTION_FIELDS)
        .map(|id| WireJunction { id })
}

/// 单 `id` 字段 record（junction/parking area/signal group）的公共解码。
fn decode_id_only<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
    expecting: &'static str,
    fields: &'static [&'static str],
) -> Result<String, ShapeCandidate> {
    let mut id = None;
    walk::decode_record(
        ctx,
        token,
        range,
        expecting,
        |ctx, key, value, value_range, mark| match key {
            "id" => walk::set_once(
                ctx,
                &mut id,
                "id",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            _ => Err(ctx.candidate(walk::unknown_field_message(key, fields), value_range)),
        },
    )?;
    id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))
}

fn decode_movement<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireMovement, ShapeCandidate> {
    let mut id = None;
    let mut junction_id = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireMovement",
        |ctx, key, value, value_range, mark| match key {
            "id" => walk::set_once(
                ctx,
                &mut id,
                "id",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "junctionId" => walk::set_once(
                ctx,
                &mut junction_id,
                "junctionId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, MOVEMENT_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireMovement {
        id: id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))?,
        junction_id: junction_id
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("junctionId"), range))?,
    })
}

fn decode_maneuver_path<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireManeuverPath, ShapeCandidate> {
    let mut id = None;
    let mut movement_id = None;
    let mut entry_edge_id = None;
    let mut internal_edge_ids = None;
    let mut exit_edge_id = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireManeuverPath",
        |ctx, key, value, value_range, mark| match key {
            "id" => walk::set_once(
                ctx,
                &mut id,
                "id",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "movementId" => walk::set_once(
                ctx,
                &mut movement_id,
                "movementId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "entryEdgeId" => walk::set_once(
                ctx,
                &mut entry_edge_id,
                "entryEdgeId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "internalEdgeIds" => walk::set_once(
                ctx,
                &mut internal_edge_ids,
                "internalEdgeIds",
                value,
                value_range,
                mark,
                decode_string_vec,
            ),
            "exitEdgeId" => walk::set_once(
                ctx,
                &mut exit_edge_id,
                "exitEdgeId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, MANEUVER_PATH_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireManeuverPath {
        id: id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))?,
        movement_id: movement_id
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("movementId"), range))?,
        entry_edge_id: entry_edge_id
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("entryEdgeId"), range))?,
        internal_edge_ids: internal_edge_ids
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("internalEdgeIds"), range))?,
        exit_edge_id: exit_edge_id
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("exitEdgeId"), range))?,
    })
}

fn decode_route<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireRoute, ShapeCandidate> {
    let mut id = None;
    let mut edge_ids = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireRoute",
        |ctx, key, value, value_range, mark| match key {
            "id" => walk::set_once(
                ctx,
                &mut id,
                "id",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "edgeIds" => walk::set_once(
                ctx,
                &mut edge_ids,
                "edgeIds",
                value,
                value_range,
                mark,
                decode_string_vec,
            ),
            _ => Err(ctx.candidate(walk::unknown_field_message(key, ROUTE_FIELDS), value_range)),
        },
    )?;
    Ok(WireRoute {
        id: id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))?,
        edge_ids: edge_ids
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("edgeIds"), range))?,
    })
}

fn decode_vehicle_profile<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireVehicleProfile, ShapeCandidate> {
    let mut id = None;
    let mut length = None;
    let mut model = None;
    let mut desired_speed = None;
    let mut min_gap = None;
    let mut time_headway = None;
    let mut max_acceleration = None;
    let mut comfortable_deceleration = None;
    let mut emergency_deceleration = None;
    let mut participant_class_id = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireVehicleProfile",
        |ctx, key, value, value_range, mark| match key {
            "id" => walk::set_once(
                ctx,
                &mut id,
                "id",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "length" => walk::set_once(
                ctx,
                &mut length,
                "length",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "model" => walk::set_once(
                ctx,
                &mut model,
                "model",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "desiredSpeed" => walk::set_once(
                ctx,
                &mut desired_speed,
                "desiredSpeed",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "minGap" => walk::set_once(
                ctx,
                &mut min_gap,
                "minGap",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "timeHeadway" => walk::set_once(
                ctx,
                &mut time_headway,
                "timeHeadway",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "maxAcceleration" => walk::set_once(
                ctx,
                &mut max_acceleration,
                "maxAcceleration",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "comfortableDeceleration" => walk::set_once(
                ctx,
                &mut comfortable_deceleration,
                "comfortableDeceleration",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "emergencyDeceleration" => walk::set_once(
                ctx,
                &mut emergency_deceleration,
                "emergencyDeceleration",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "participantClassId" => walk::set_once(
                ctx,
                &mut participant_class_id,
                "participantClassId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, VEHICLE_PROFILE_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireVehicleProfile {
        id: id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))?,
        length: length
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("length"), range))?,
        model: model.ok_or_else(|| ctx.candidate(walk::missing_field_message("model"), range))?,
        desired_speed: desired_speed
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("desiredSpeed"), range))?,
        min_gap: min_gap
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("minGap"), range))?,
        time_headway: time_headway
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("timeHeadway"), range))?,
        max_acceleration: max_acceleration
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("maxAcceleration"), range))?,
        comfortable_deceleration: comfortable_deceleration.ok_or_else(|| {
            ctx.candidate(
                walk::missing_field_message("comfortableDeceleration"),
                range,
            )
        })?,
        emergency_deceleration: emergency_deceleration.ok_or_else(|| {
            ctx.candidate(walk::missing_field_message("emergencyDeceleration"), range)
        })?,
        participant_class_id: participant_class_id.ok_or_else(|| {
            ctx.candidate(walk::missing_field_message("participantClassId"), range)
        })?,
    })
}

fn decode_participant_class<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireParticipantClass, ShapeCandidate> {
    let mut id = None;
    let mut extends_id = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireParticipantClass",
        |ctx, key, value, value_range, mark| match key {
            "id" => walk::set_once(
                ctx,
                &mut id,
                "id",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "extendsId" => walk::set_once(
                ctx,
                &mut extends_id,
                "extendsId",
                value,
                value_range,
                mark,
                walk::decode_non_null_string,
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, PARTICIPANT_CLASS_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireParticipantClass {
        id: id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))?,
        extends_id,
    })
}

fn decode_facility_band<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireFacilityBand, ShapeCandidate> {
    let mut id = None;
    let mut kind_id = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireFacilityBand",
        |ctx, key, value, value_range, mark| match key {
            "id" => walk::set_once(
                ctx,
                &mut id,
                "id",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "kindId" => walk::set_once(
                ctx,
                &mut kind_id,
                "kindId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, FACILITY_BAND_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireFacilityBand {
        id: id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))?,
        kind_id: kind_id
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("kindId"), range))?,
    })
}

fn decode_road_section<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireRoadSection, ShapeCandidate> {
    let mut id = None;
    let mut kind_id = None;
    let mut lanes = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireRoadSection",
        |ctx, key, value, value_range, mark| match key {
            "id" => walk::set_once(
                ctx,
                &mut id,
                "id",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "kindId" => walk::set_once(
                ctx,
                &mut kind_id,
                "kindId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "lanes" => walk::set_once(
                ctx,
                &mut lanes,
                "lanes",
                value,
                value_range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_section_lane),
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, ROAD_SECTION_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireRoadSection {
        id: id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))?,
        kind_id: kind_id
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("kindId"), range))?,
        lanes: lanes.ok_or_else(|| ctx.candidate(walk::missing_field_message("lanes"), range))?,
    })
}

fn decode_section_lane<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireSectionLane, ShapeCandidate> {
    let mut edge_ids = None;
    let mut lane_group_id = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireSectionLane",
        |ctx, key, value, value_range, mark| match key {
            "edgeIds" => walk::set_once(
                ctx,
                &mut edge_ids,
                "edgeIds",
                value,
                value_range,
                mark,
                decode_string_vec,
            ),
            "laneGroupId" => walk::set_once(
                ctx,
                &mut lane_group_id,
                "laneGroupId",
                value,
                value_range,
                mark,
                walk::decode_non_null_string,
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, SECTION_LANE_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireSectionLane {
        edge_ids: edge_ids
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("edgeIds"), range))?,
        lane_group_id,
    })
}

fn decode_lane_group<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireLaneGroup, ShapeCandidate> {
    let mut id = None;
    let mut road_section_id = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireLaneGroup",
        |ctx, key, value, value_range, mark| match key {
            "id" => walk::set_once(
                ctx,
                &mut id,
                "id",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "roadSectionId" => walk::set_once(
                ctx,
                &mut road_section_id,
                "roadSectionId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, LANE_GROUP_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireLaneGroup {
        id: id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))?,
        road_section_id: road_section_id
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("roadSectionId"), range))?,
    })
}

fn decode_road_corridor<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireRoadCorridor, ShapeCandidate> {
    let mut id = None;
    let mut reference_section_id = None;
    let mut elements = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireRoadCorridor",
        |ctx, key, value, value_range, mark| match key {
            "id" => walk::set_once(
                ctx,
                &mut id,
                "id",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "referenceSectionId" => walk::set_once(
                ctx,
                &mut reference_section_id,
                "referenceSectionId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "elements" => walk::set_once(
                ctx,
                &mut elements,
                "elements",
                value,
                value_range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_corridor_element),
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, ROAD_CORRIDOR_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireRoadCorridor {
        id: id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))?,
        reference_section_id: reference_section_id.ok_or_else(|| {
            ctx.candidate(walk::missing_field_message("referenceSectionId"), range)
        })?,
        elements: elements
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("elements"), range))?,
    })
}

/// untagged corridor 元素：缓冲 key 集合后按 Section{sectionId}/Band{bandId}
/// 分派；任何偏差（多余/重复/缺失 key、值解码失败、结构错误）都归一为
/// untagged mismatch 候选，与 derive 的变体尝试语义一致。
fn decode_corridor_element<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireCorridorElement, ShapeCandidate> {
    let mut section_id = None;
    let mut band_id = None;
    let mut clean = true;
    let structural = walk::scan_record(
        ctx,
        token,
        range,
        "struct WireCorridorElement",
        |ctx, key, value, value_range, _mark| {
            match key {
                "sectionId" => {
                    if section_id.is_some() {
                        clean = false;
                    }
                    match walk::decode_scalar::<String, L>(ctx, value, value_range) {
                        Ok(id) => section_id = Some(id),
                        Err(_) => clean = false,
                    }
                }
                "bandId" => {
                    if band_id.is_some() {
                        clean = false;
                    }
                    match walk::decode_scalar::<String, L>(ctx, value, value_range) {
                        Ok(id) => band_id = Some(id),
                        Err(_) => clean = false,
                    }
                }
                _ => clean = false,
            }
            Ok(())
        },
    )?;
    clean &= structural;
    match (section_id, band_id, clean) {
        (Some(section_id), None, true) => Ok(WireCorridorElement::Section(
            crate::wire::WireCorridorSectionElement { section_id },
        )),
        (None, Some(band_id), true) => Ok(WireCorridorElement::Band(
            crate::wire::WireCorridorBandElement { band_id },
        )),
        _ => Err(ctx.candidate(
            "data did not match any variant of untagged enum WireCorridorElement".to_owned(),
            range,
        )),
    }
}

fn decode_access_rule<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireAccessRule, ShapeCandidate> {
    let mut id = None;
    let mut target = None;
    let mut effect = None;
    let mut participant_class_ids = None;
    let mut time_windows = None;
    let mut regulation = None;
    let mut priority = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireAccessRule",
        |ctx, key, value, value_range, mark| match key {
            "id" => walk::set_once(
                ctx,
                &mut id,
                "id",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "target" => walk::set_once(
                ctx,
                &mut target,
                "target",
                value,
                value_range,
                mark,
                decode_access_target,
            ),
            "effect" => walk::set_once(
                ctx,
                &mut effect,
                "effect",
                value,
                value_range,
                mark,
                |ctx, token, range| {
                    walk::decode_enum(
                        ctx,
                        token,
                        range,
                        "WireAccessEffect",
                        ACCESS_EFFECT_VARIANTS,
                        ACCESS_EFFECT_TABLE,
                    )
                },
            ),
            "participantClassIds" => walk::set_once(
                ctx,
                &mut participant_class_ids,
                "participantClassIds",
                value,
                value_range,
                mark,
                decode_string_vec,
            ),
            "timeWindows" => walk::set_once(
                ctx,
                &mut time_windows,
                "timeWindows",
                value,
                value_range,
                mark,
                walk::decode_time_windows,
            ),
            "regulation" => walk::set_once(
                ctx,
                &mut regulation,
                "regulation",
                value,
                value_range,
                mark,
                decode_non_null_regulation,
            ),
            "priority" => walk::set_once(
                ctx,
                &mut priority,
                "priority",
                value,
                value_range,
                mark,
                |ctx, token, range| walk::decode_priority(ctx, token, range),
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, ACCESS_RULE_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireAccessRule {
        id: id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))?,
        target: target
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("target"), range))?,
        effect: effect
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("effect"), range))?,
        participant_class_ids: participant_class_ids.ok_or_else(|| {
            ctx.candidate(walk::missing_field_message("participantClassIds"), range)
        })?,
        time_windows: time_windows.unwrap_or(false),
        regulation,
        priority,
    })
}

fn decode_access_target<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireAccessTarget, ShapeCandidate> {
    let mut kind = None;
    let mut id = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireAccessTarget",
        |ctx, key, value, value_range, mark| match key {
            "kind" => walk::set_once(
                ctx,
                &mut kind,
                "kind",
                value,
                value_range,
                mark,
                |ctx, token, range| {
                    walk::decode_enum(
                        ctx,
                        token,
                        range,
                        "WireAccessTargetKind",
                        ACCESS_TARGET_KIND_VARIANTS,
                        ACCESS_TARGET_KIND_TABLE,
                    )
                },
            ),
            "id" => walk::set_once(
                ctx,
                &mut id,
                "id",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, ACCESS_TARGET_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireAccessTarget {
        kind: kind.ok_or_else(|| ctx.candidate(walk::missing_field_message("kind"), range))?,
        id: id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))?,
    })
}

fn decode_waiting_zone<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireWaitingZone, ShapeCandidate> {
    let mut id = None;
    let mut maneuver_path_id = None;
    let mut entry_gate_id = None;
    let mut release_gate_id = None;
    let mut max_occupancy = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireWaitingZone",
        |ctx, key, value, value_range, mark| match key {
            "id" => walk::set_once(
                ctx,
                &mut id,
                "id",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "maneuverPathId" => walk::set_once(
                ctx,
                &mut maneuver_path_id,
                "maneuverPathId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "entryGateId" => walk::set_once(
                ctx,
                &mut entry_gate_id,
                "entryGateId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "releaseGateId" => walk::set_once(
                ctx,
                &mut release_gate_id,
                "releaseGateId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "maxOccupancy" => walk::set_once(
                ctx,
                &mut max_occupancy,
                "maxOccupancy",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, WAITING_ZONE_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireWaitingZone {
        id: id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))?,
        maneuver_path_id: maneuver_path_id
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("maneuverPathId"), range))?,
        entry_gate_id: entry_gate_id
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("entryGateId"), range))?,
        release_gate_id: release_gate_id
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("releaseGateId"), range))?,
        max_occupancy: max_occupancy
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("maxOccupancy"), range))?,
    })
}

fn decode_non_null_regulation<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireRegulation, ShapeCandidate> {
    walk::reject_explicit_null(ctx, token, range)?;
    decode_regulation(ctx, token, range)
}

fn decode_regulation<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireRegulation, ShapeCandidate> {
    let mut jurisdiction = None;
    let mut version = None;
    let mut source = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireRegulation",
        |ctx, key, value, value_range, mark| match key {
            "jurisdiction" => walk::set_once(
                ctx,
                &mut jurisdiction,
                "jurisdiction",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "version" => walk::set_once(
                ctx,
                &mut version,
                "version",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "source" => walk::set_once(
                ctx,
                &mut source,
                "source",
                value,
                value_range,
                mark,
                walk::decode_non_null_string,
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, REGULATION_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireRegulation {
        jurisdiction: jurisdiction
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("jurisdiction"), range))?,
        version: version
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("version"), range))?,
        source,
    })
}

fn decode_parking<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireParking, ShapeCandidate> {
    let mut areas = None;
    let mut spaces = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireParking",
        |ctx, key, value, value_range, mark| match key {
            "areas" => walk::set_once(
                ctx,
                &mut areas,
                "areas",
                value,
                value_range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_parking_area),
            ),
            "spaces" => walk::set_once(
                ctx,
                &mut spaces,
                "spaces",
                value,
                value_range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_parking_space),
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, PARKING_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireParking {
        areas: areas.ok_or_else(|| ctx.candidate(walk::missing_field_message("areas"), range))?,
        spaces: spaces
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("spaces"), range))?,
    })
}

fn decode_parking_area<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireParkingArea, ShapeCandidate> {
    decode_id_only(
        ctx,
        token,
        range,
        "struct WireParkingArea",
        PARKING_AREA_FIELDS,
    )
    .map(|id| WireParkingArea { id })
}

fn decode_parking_space<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireParkingSpace, ShapeCandidate> {
    let mut id = None;
    let mut area_id = None;
    let mut entry = None;
    let mut exit = None;
    let mut geometry = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireParkingSpace",
        |ctx, key, value, value_range, mark| {
            match key {
                "id" => walk::set_once(
                    ctx,
                    &mut id,
                    "id",
                    value,
                    value_range,
                    mark,
                    walk::decode_scalar,
                ),
                // OmittedAreaId：缺省为 None；显式 null/非字符串按 String 解码失败。
                "areaId" => walk::set_once(
                    ctx,
                    &mut area_id,
                    "areaId",
                    value,
                    value_range,
                    mark,
                    walk::decode_scalar,
                ),
                "entry" => walk::set_once(
                    ctx,
                    &mut entry,
                    "entry",
                    value,
                    value_range,
                    mark,
                    decode_parking_anchor,
                ),
                "exit" => walk::set_once(
                    ctx,
                    &mut exit,
                    "exit",
                    value,
                    value_range,
                    mark,
                    decode_parking_anchor,
                ),
                "geometry" => walk::set_once(
                    ctx,
                    &mut geometry,
                    "geometry",
                    value,
                    value_range,
                    mark,
                    decode_parking_geometry,
                ),
                _ => Err(ctx.candidate(
                    walk::unknown_field_message(key, PARKING_SPACE_FIELDS),
                    value_range,
                )),
            }
        },
    )?;
    Ok(WireParkingSpace {
        id: id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))?,
        area_id,
        entry: entry.ok_or_else(|| ctx.candidate(walk::missing_field_message("entry"), range))?,
        exit: exit.ok_or_else(|| ctx.candidate(walk::missing_field_message("exit"), range))?,
        geometry: geometry
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("geometry"), range))?,
    })
}

fn decode_parking_anchor<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireParkingAnchor, ShapeCandidate> {
    let mut edge_id = None;
    let mut progress = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireParkingAnchor",
        |ctx, key, value, value_range, mark| match key {
            "edgeId" => walk::set_once(
                ctx,
                &mut edge_id,
                "edgeId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "progress" => walk::set_once(
                ctx,
                &mut progress,
                "progress",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, PARKING_ANCHOR_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireParkingAnchor {
        edge_id: edge_id
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("edgeId"), range))?,
        progress: progress
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("progress"), range))?,
    })
}

fn decode_parking_geometry<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireParkingGeometry, ShapeCandidate> {
    let mut lateral_offset = None;
    let mut heading_offset_radians = None;
    let mut length = None;
    let mut width = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireParkingGeometry",
        |ctx, key, value, value_range, mark| match key {
            "lateralOffset" => walk::set_once(
                ctx,
                &mut lateral_offset,
                "lateralOffset",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "headingOffsetRadians" => walk::set_once(
                ctx,
                &mut heading_offset_radians,
                "headingOffsetRadians",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "length" => walk::set_once(
                ctx,
                &mut length,
                "length",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "width" => walk::set_once(
                ctx,
                &mut width,
                "width",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, PARKING_GEOMETRY_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireParkingGeometry {
        lateral_offset: lateral_offset
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("lateralOffset"), range))?,
        heading_offset_radians: heading_offset_radians.ok_or_else(|| {
            ctx.candidate(walk::missing_field_message("headingOffsetRadians"), range)
        })?,
        length: length
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("length"), range))?,
        width: width.ok_or_else(|| ctx.candidate(walk::missing_field_message("width"), range))?,
    })
}

fn decode_signals<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireSignals, ShapeCandidate> {
    let mut stop_lines = None;
    let mut maneuver_gates = None;
    let mut groups = None;
    let mut controllers = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireSignals",
        |ctx, key, value, value_range, mark| match key {
            "stopLines" => walk::set_once(
                ctx,
                &mut stop_lines,
                "stopLines",
                value,
                value_range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_stop_line),
            ),
            "maneuverGates" => walk::set_once(
                ctx,
                &mut maneuver_gates,
                "maneuverGates",
                value,
                value_range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_maneuver_gate),
            ),
            "groups" => walk::set_once(
                ctx,
                &mut groups,
                "groups",
                value,
                value_range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_signal_group),
            ),
            "controllers" => walk::set_once(
                ctx,
                &mut controllers,
                "controllers",
                value,
                value_range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_signal_controller),
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, SIGNALS_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireSignals {
        stop_lines: stop_lines
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("stopLines"), range))?,
        maneuver_gates: maneuver_gates
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("maneuverGates"), range))?,
        groups: groups
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("groups"), range))?,
        controllers: controllers
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("controllers"), range))?,
    })
}

fn decode_stop_line<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireStopLine, ShapeCandidate> {
    let mut id = None;
    let mut edge_id = None;
    let mut location = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireStopLine",
        |ctx, key, value, value_range, mark| match key {
            "id" => walk::set_once(
                ctx,
                &mut id,
                "id",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "edgeId" => walk::set_once(
                ctx,
                &mut edge_id,
                "edgeId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "location" => walk::set_once(
                ctx,
                &mut location,
                "location",
                value,
                value_range,
                mark,
                |ctx, token, range| {
                    walk::decode_enum(
                        ctx,
                        token,
                        range,
                        "WireStopLineLocation",
                        STOP_LINE_LOCATION_VARIANTS,
                        STOP_LINE_LOCATION_TABLE,
                    )
                },
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, STOP_LINE_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireStopLine {
        id: id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))?,
        edge_id: edge_id
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("edgeId"), range))?,
        location: location
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("location"), range))?,
    })
}

fn decode_maneuver_gate<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireManeuverGate, ShapeCandidate> {
    let mut id = None;
    let mut maneuver_path_id = None;
    let mut transition_index = None;
    let mut stop_line_id = None;
    let mut signal_control = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireManeuverGate",
        |ctx, key, value, value_range, mark| match key {
            "id" => walk::set_once(
                ctx,
                &mut id,
                "id",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "maneuverPathId" => walk::set_once(
                ctx,
                &mut maneuver_path_id,
                "maneuverPathId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "transitionIndex" => walk::set_once(
                ctx,
                &mut transition_index,
                "transitionIndex",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "stopLineId" => walk::set_once(
                ctx,
                &mut stop_line_id,
                "stopLineId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "signalControl" => walk::set_once(
                ctx,
                &mut signal_control,
                "signalControl",
                value,
                value_range,
                mark,
                decode_signal_control,
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, MANEUVER_GATE_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireManeuverGate {
        id: id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))?,
        maneuver_path_id: maneuver_path_id
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("maneuverPathId"), range))?,
        transition_index: transition_index
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("transitionIndex"), range))?,
        stop_line_id: stop_line_id
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("stopLineId"), range))?,
        signal_control: signal_control
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("signalControl"), range))?,
    })
}

/// untagged signal control：按 `kind` 字符串分派（"group" 需恰好一个合法
/// `groupId` 且无多余 key；"none" 不得携带 `groupId`）；任何偏差归一为
/// untagged mismatch 候选。
fn decode_signal_control<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireSignalControl, ShapeCandidate> {
    let mut kind = None;
    let mut group_id = None;
    let mut clean = true;
    let structural = walk::scan_record(
        ctx,
        token,
        range,
        "struct WireSignalControl",
        |ctx, key, value, value_range, _mark| {
            match key {
                "kind" => {
                    if kind.is_some() {
                        clean = false;
                    }
                    match walk::decode_scalar::<String, L>(ctx, value, value_range) {
                        Ok(value) => kind = Some(value),
                        Err(_) => clean = false,
                    }
                }
                "groupId" => {
                    if group_id.is_some() {
                        clean = false;
                    }
                    match walk::decode_scalar::<String, L>(ctx, value, value_range) {
                        Ok(value) => group_id = Some(value),
                        Err(_) => clean = false,
                    }
                }
                _ => clean = false,
            }
            Ok(())
        },
    )?;
    clean &= structural;
    match (kind.as_deref(), group_id, clean) {
        (Some("group"), Some(group_id), true) => {
            Ok(WireSignalControl::Group(WireGroupSignalControl {
                kind: WireGroupSignalControlKind::Group,
                group_id,
            }))
        }
        (Some("none"), None, true) => Ok(WireSignalControl::None(WireNoneSignalControl {
            kind: WireNoneSignalControlKind::None,
        })),
        _ => Err(ctx.candidate(
            "data did not match any variant of untagged enum WireSignalControl".to_owned(),
            range,
        )),
    }
}

fn decode_signal_group<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireSignalGroup, ShapeCandidate> {
    decode_id_only(
        ctx,
        token,
        range,
        "struct WireSignalGroup",
        SIGNAL_GROUP_FIELDS,
    )
    .map(|id| WireSignalGroup { id })
}

fn decode_signal_controller<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireSignalController, ShapeCandidate> {
    let mut id = None;
    let mut kind = None;
    let mut offset_ms = None;
    let mut group_ids = None;
    let mut phases = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireSignalController",
        |ctx, key, value, value_range, mark| match key {
            "id" => walk::set_once(
                ctx,
                &mut id,
                "id",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "kind" => walk::set_once(
                ctx,
                &mut kind,
                "kind",
                value,
                value_range,
                mark,
                |ctx, token, range| {
                    walk::decode_enum(
                        ctx,
                        token,
                        range,
                        "WireSignalControllerKind",
                        SIGNAL_CONTROLLER_KIND_VARIANTS,
                        SIGNAL_CONTROLLER_KIND_TABLE,
                    )
                },
            ),
            "offsetMs" => walk::set_once(
                ctx,
                &mut offset_ms,
                "offsetMs",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "groupIds" => walk::set_once(
                ctx,
                &mut group_ids,
                "groupIds",
                value,
                value_range,
                mark,
                decode_string_vec,
            ),
            "phases" => walk::set_once(
                ctx,
                &mut phases,
                "phases",
                value,
                value_range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_signal_phase),
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, SIGNAL_CONTROLLER_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireSignalController {
        id: id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))?,
        kind: kind.ok_or_else(|| ctx.candidate(walk::missing_field_message("kind"), range))?,
        offset_ms: offset_ms
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("offsetMs"), range))?,
        group_ids: group_ids
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("groupIds"), range))?,
        phases: phases
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("phases"), range))?,
    })
}

fn decode_signal_phase<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireSignalPhase, ShapeCandidate> {
    let mut id = None;
    let mut duration_ms = None;
    let mut states = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireSignalPhase",
        |ctx, key, value, value_range, mark| match key {
            "id" => walk::set_once(
                ctx,
                &mut id,
                "id",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "durationMs" => walk::set_once(
                ctx,
                &mut duration_ms,
                "durationMs",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "states" => walk::set_once(
                ctx,
                &mut states,
                "states",
                value,
                value_range,
                mark,
                |ctx, token, range| decode_record_vec(ctx, token, range, decode_signal_group_state),
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, SIGNAL_PHASE_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireSignalPhase {
        id: id.ok_or_else(|| ctx.candidate(walk::missing_field_message("id"), range))?,
        duration_ms: duration_ms
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("durationMs"), range))?,
        states: states
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("states"), range))?,
    })
}

fn decode_signal_group_state<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireSignalGroupState, ShapeCandidate> {
    let mut group_id = None;
    let mut aspect = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireSignalGroupState",
        |ctx, key, value, value_range, mark| match key {
            "groupId" => walk::set_once(
                ctx,
                &mut group_id,
                "groupId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "aspect" => walk::set_once(
                ctx,
                &mut aspect,
                "aspect",
                value,
                value_range,
                mark,
                |ctx, token, range| {
                    walk::decode_enum(
                        ctx,
                        token,
                        range,
                        "WireSignalAspect",
                        SIGNAL_ASPECT_VARIANTS,
                        SIGNAL_ASPECT_TABLE,
                    )
                },
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, SIGNAL_GROUP_STATE_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireSignalGroupState {
        group_id: group_id
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("groupId"), range))?,
        aspect: aspect
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("aspect"), range))?,
    })
}
