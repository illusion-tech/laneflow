//! Overlay wire records。信号与停车子域先按共同 Typed AST 所需字段紧凑保留。

use super::road::{RawNumber, parse_array, parse_raw_number, parse_unique_tokens};
use super::{
    ByteSpan, ClosedFields, JsonCursor, SchemaError, SchemaErrorKind, SpannedString,
    parse_object_members, parse_string, parse_token,
};

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedSignalGroup {
    pub(in crate::module::geometry) signal_group_key: SpannedString,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedSignalController {
    pub(in crate::module::geometry) signal_controller_key: SpannedString,
    pub(in crate::module::geometry) offset_seconds: RawNumber,
    pub(in crate::module::geometry) signal_groups: Box<[SpannedString]>,
    pub(in crate::module::geometry) phases: Box<[ParsedSignalPhase]>,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedSignalPhase {
    pub(in crate::module::geometry) signal_phase_key: SpannedString,
    pub(in crate::module::geometry) duration_seconds: RawNumber,
    pub(in crate::module::geometry) states: Box<[ParsedSignalState]>,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::module::geometry) enum ParsedSignalAspect {
    Red,
    Yellow,
    Green,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedSignalState {
    pub(in crate::module::geometry) signal_group: SpannedString,
    pub(in crate::module::geometry) aspect: ParsedSignalAspect,
    pub(in crate::module::geometry) aspect_span: ByteSpan,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedParkingArea {
    pub(in crate::module::geometry) parking_area_key: SpannedString,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedParkingAnchor {
    pub(in crate::module::geometry) lane_edge: SpannedString,
    pub(in crate::module::geometry) progress_meters: RawNumber,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedParkingGeometry {
    pub(in crate::module::geometry) lateral_offset_meters: RawNumber,
    pub(in crate::module::geometry) heading_offset_radians: RawNumber,
    pub(in crate::module::geometry) length_meters: RawNumber,
    pub(in crate::module::geometry) width_meters: RawNumber,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedParkingSpace {
    pub(in crate::module::geometry) parking_space_key: SpannedString,
    pub(in crate::module::geometry) parking_area: Option<SpannedString>,
    pub(in crate::module::geometry) entry: ParsedParkingAnchor,
    pub(in crate::module::geometry) exit: ParsedParkingAnchor,
    pub(in crate::module::geometry) geometry: ParsedParkingGeometry,
    pub(in crate::module::geometry) span: ByteSpan,
}

pub(in crate::module::geometry) fn parse_signal_groups(
    cursor: &mut JsonCursor<'_>,
) -> Result<Box<[ParsedSignalGroup]>, SchemaError> {
    parse_array(cursor, "signalGroups", false, parse_signal_group)
}

pub(in crate::module::geometry) fn parse_signal_controllers(
    cursor: &mut JsonCursor<'_>,
) -> Result<Box<[ParsedSignalController]>, SchemaError> {
    parse_array(cursor, "signalControllers", false, parse_signal_controller)
}

pub(in crate::module::geometry) fn parse_parking_areas(
    cursor: &mut JsonCursor<'_>,
) -> Result<Box<[ParsedParkingArea]>, SchemaError> {
    parse_array(cursor, "parkingAreas", false, parse_parking_area)
}

pub(in crate::module::geometry) fn parse_parking_spaces(
    cursor: &mut JsonCursor<'_>,
) -> Result<Box<[ParsedParkingSpace]>, SchemaError> {
    parse_array(cursor, "parkingSpaces", false, parse_parking_space)
}

fn parse_signal_group(cursor: &mut JsonCursor<'_>) -> Result<ParsedSignalGroup, SchemaError> {
    let (key, span) = parse_key_record(cursor, "signalGroupKey")?;
    Ok(ParsedSignalGroup {
        signal_group_key: key,
        span,
    })
}

fn parse_signal_controller(
    cursor: &mut JsonCursor<'_>,
) -> Result<ParsedSignalController, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new([
        "signalControllerKey",
        "offsetSeconds",
        "signalGroups",
        "phases",
    ]);
    let mut key = None;
    let mut offset = None;
    let mut groups = None;
    let mut phases = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => key = Some(parse_token(cursor, "signalControllerKey")?),
            1 => offset = Some(parse_raw_number(cursor)?),
            2 => groups = Some(parse_unique_tokens(cursor, "signalGroups")?),
            3 => phases = Some(parse_array(cursor, "phases", true, parse_signal_phase)?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    let groups = groups.expect("required field checked");
    if groups.is_empty() {
        return Err(SchemaError {
            kind: SchemaErrorKind::EmptyArray("signalGroups"),
            span,
        });
    }
    Ok(ParsedSignalController {
        signal_controller_key: key.expect("required field checked"),
        offset_seconds: offset.expect("required field checked"),
        signal_groups: groups,
        phases: phases.expect("required field checked"),
        span,
    })
}

fn parse_signal_phase(cursor: &mut JsonCursor<'_>) -> Result<ParsedSignalPhase, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new(["signalPhaseKey", "durationSeconds", "states"]);
    let mut key = None;
    let mut duration = None;
    let mut states = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => key = Some(parse_token(cursor, "signalPhaseKey")?),
            1 => duration = Some(parse_raw_number(cursor)?),
            2 => states = Some(parse_array(cursor, "states", true, parse_signal_state)?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok(ParsedSignalPhase {
        signal_phase_key: key.expect("required field checked"),
        duration_seconds: duration.expect("required field checked"),
        states: states.expect("required field checked"),
        span,
    })
}

fn parse_signal_state(cursor: &mut JsonCursor<'_>) -> Result<ParsedSignalState, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new(["signalGroup", "aspect"]);
    let mut group = None;
    let mut aspect = None;
    let mut aspect_span = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => group = Some(parse_token(cursor, "signalGroup")?),
            1 => {
                let value = parse_string(cursor)?;
                aspect_span = Some(value.span);
                aspect = Some(match value.value.as_ref() {
                    "red" => ParsedSignalAspect::Red,
                    "yellow" => ParsedSignalAspect::Yellow,
                    "green" => ParsedSignalAspect::Green,
                    _ => {
                        return Err(SchemaError {
                            kind: SchemaErrorKind::InvalidEnum {
                                field: "aspect",
                                value: value.value,
                            },
                            span: value.span,
                        });
                    }
                });
            }
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok(ParsedSignalState {
        signal_group: group.expect("required field checked"),
        aspect: aspect.expect("required field checked"),
        aspect_span: aspect_span.expect("required field checked"),
        span,
    })
}

fn parse_parking_area(cursor: &mut JsonCursor<'_>) -> Result<ParsedParkingArea, SchemaError> {
    let (key, span) = parse_key_record(cursor, "parkingAreaKey")?;
    Ok(ParsedParkingArea {
        parking_area_key: key,
        span,
    })
}

fn parse_parking_space(cursor: &mut JsonCursor<'_>) -> Result<ParsedParkingSpace, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new([
        "parkingSpaceKey",
        "parkingArea",
        "entry",
        "exit",
        "geometry",
    ]);
    let mut key = None;
    let mut area = None;
    let mut entry = None;
    let mut exit = None;
    let mut geometry = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => key = Some(parse_token(cursor, "parkingSpaceKey")?),
            1 => area = Some(parse_token(cursor, "parkingArea")?),
            2 => entry = Some(parse_parking_anchor(cursor)?),
            3 => exit = Some(parse_parking_anchor(cursor)?),
            4 => geometry = Some(parse_parking_geometry(cursor)?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_indices(&[0, 2, 3, 4], span)?;
    Ok(ParsedParkingSpace {
        parking_space_key: key.expect("required field checked"),
        parking_area: area,
        entry: entry.expect("required field checked"),
        exit: exit.expect("required field checked"),
        geometry: geometry.expect("required field checked"),
        span,
    })
}

fn parse_parking_anchor(cursor: &mut JsonCursor<'_>) -> Result<ParsedParkingAnchor, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new(["laneEdge", "progressMeters"]);
    let mut edge = None;
    let mut progress = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => edge = Some(parse_token(cursor, "laneEdge")?),
            1 => progress = Some(parse_raw_number(cursor)?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok(ParsedParkingAnchor {
        lane_edge: edge.expect("required field checked"),
        progress_meters: progress.expect("required field checked"),
        span,
    })
}

fn parse_parking_geometry(
    cursor: &mut JsonCursor<'_>,
) -> Result<ParsedParkingGeometry, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new([
        "lateralOffsetMeters",
        "headingOffsetRadians",
        "lengthMeters",
        "widthMeters",
    ]);
    let mut values = [None, None, None, None];
    parse_object_members(cursor, &mut fields, |index, cursor| {
        values[index] = Some(parse_raw_number(cursor)?);
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    let [lateral, heading, length, width] = values;
    Ok(ParsedParkingGeometry {
        lateral_offset_meters: lateral.expect("required field checked"),
        heading_offset_radians: heading.expect("required field checked"),
        length_meters: length.expect("required field checked"),
        width_meters: width.expect("required field checked"),
        span,
    })
}

fn parse_key_record(
    cursor: &mut JsonCursor<'_>,
    field: &'static str,
) -> Result<(SpannedString, ByteSpan), SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new([field]);
    let mut key = None;
    parse_object_members(cursor, &mut fields, |_, cursor| {
        key = Some(parse_token(cursor, field)?);
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok((key.expect("required field checked"), span))
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedParticipantClass {
    pub(in crate::module::geometry) participant_class_key: SpannedString,
    pub(in crate::module::geometry) extends: Option<SpannedString>,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedVehicleProfile {
    pub(in crate::module::geometry) vehicle_profile_key: SpannedString,
    pub(in crate::module::geometry) participant_class: SpannedString,
    pub(in crate::module::geometry) iidm: Box<[RawNumber; 7]>,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) enum ParsedAccessTarget {
    LaneEdge(SpannedString),
    LaneGroup(SpannedString),
    RoadSection(SpannedString),
    ManeuverPath(SpannedString),
    FacilityBand(SpannedString),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::module::geometry) enum ParsedAccessEffect {
    Allow,
    Deny,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedRegulation {
    pub(in crate::module::geometry) jurisdiction: SpannedString,
    pub(in crate::module::geometry) version: SpannedString,
    pub(in crate::module::geometry) source: Option<SpannedString>,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedAccessRule {
    pub(in crate::module::geometry) access_rule_key: SpannedString,
    pub(in crate::module::geometry) target: ParsedAccessTarget,
    pub(in crate::module::geometry) effect: ParsedAccessEffect,
    pub(in crate::module::geometry) participant_classes: Box<[SpannedString]>,
    pub(in crate::module::geometry) regulation: Option<ParsedRegulation>,
    pub(in crate::module::geometry) priority: RawNumber,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedStaticRoute {
    pub(in crate::module::geometry) static_route_key: SpannedString,
    pub(in crate::module::geometry) edge_sequence: Box<[SpannedString]>,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedStopLine {
    pub(in crate::module::geometry) stop_line_key: SpannedString,
    pub(in crate::module::geometry) lane_edge: SpannedString,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedManeuverGate {
    pub(in crate::module::geometry) maneuver_gate_key: SpannedString,
    pub(in crate::module::geometry) maneuver_path: SpannedString,
    pub(in crate::module::geometry) transition_index: RawNumber,
    pub(in crate::module::geometry) stop_line: SpannedString,
    pub(in crate::module::geometry) signal_control: Option<SpannedString>,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedWaitingZone {
    pub(in crate::module::geometry) waiting_zone_key: SpannedString,
    pub(in crate::module::geometry) maneuver_path: SpannedString,
    pub(in crate::module::geometry) entry_gate: SpannedString,
    pub(in crate::module::geometry) release_gate: SpannedString,
    pub(in crate::module::geometry) max_occupancy: RawNumber,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedOverlays {
    pub(in crate::module::geometry) signal_groups: Box<[ParsedSignalGroup]>,
    pub(in crate::module::geometry) signal_controllers: Box<[ParsedSignalController]>,
    pub(in crate::module::geometry) parking_areas: Box<[ParsedParkingArea]>,
    pub(in crate::module::geometry) parking_spaces: Box<[ParsedParkingSpace]>,
    pub(in crate::module::geometry) participant_classes: Box<[ParsedParticipantClass]>,
    pub(in crate::module::geometry) vehicle_profiles: Box<[ParsedVehicleProfile]>,
    pub(in crate::module::geometry) access_rules: Box<[ParsedAccessRule]>,
    pub(in crate::module::geometry) static_routes: Box<[ParsedStaticRoute]>,
    pub(in crate::module::geometry) stop_lines: Box<[ParsedStopLine]>,
    pub(in crate::module::geometry) maneuver_gates: Box<[ParsedManeuverGate]>,
    pub(in crate::module::geometry) waiting_zones: Box<[ParsedWaitingZone]>,
    pub(in crate::module::geometry) span: ByteSpan,
}

pub(in crate::module::geometry) fn parse_overlays(
    cursor: &mut JsonCursor<'_>,
) -> Result<ParsedOverlays, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new([
        "signalGroups",
        "signalControllers",
        "parkingAreas",
        "parkingSpaces",
        "participantClasses",
        "vehicleProfiles",
        "accessRules",
        "staticRoutes",
        "stopLines",
        "maneuverGates",
        "waitingZones",
    ]);
    let mut signal_groups = None;
    let mut signal_controllers = None;
    let mut parking_areas = None;
    let mut parking_spaces = None;
    let mut participant_classes = None;
    let mut vehicle_profiles = None;
    let mut access_rules = None;
    let mut static_routes = None;
    let mut stop_lines = None;
    let mut maneuver_gates = None;
    let mut waiting_zones = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => signal_groups = Some(parse_signal_groups(cursor)?),
            1 => signal_controllers = Some(parse_signal_controllers(cursor)?),
            2 => parking_areas = Some(parse_parking_areas(cursor)?),
            3 => parking_spaces = Some(parse_parking_spaces(cursor)?),
            4 => participant_classes = Some(parse_participant_classes(cursor)?),
            5 => vehicle_profiles = Some(parse_vehicle_profiles(cursor)?),
            6 => access_rules = Some(parse_access_rules(cursor)?),
            7 => static_routes = Some(parse_static_routes(cursor)?),
            8 => stop_lines = Some(parse_stop_lines(cursor)?),
            9 => maneuver_gates = Some(parse_maneuver_gates(cursor)?),
            10 => waiting_zones = Some(parse_waiting_zones(cursor)?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok(ParsedOverlays {
        signal_groups: signal_groups.unwrap(),
        signal_controllers: signal_controllers.unwrap(),
        parking_areas: parking_areas.unwrap(),
        parking_spaces: parking_spaces.unwrap(),
        participant_classes: participant_classes.unwrap(),
        vehicle_profiles: vehicle_profiles.unwrap(),
        access_rules: access_rules.unwrap(),
        static_routes: static_routes.unwrap(),
        stop_lines: stop_lines.unwrap(),
        maneuver_gates: maneuver_gates.unwrap(),
        waiting_zones: waiting_zones.unwrap(),
        span,
    })
}

pub(in crate::module::geometry) fn parse_participant_classes(
    cursor: &mut JsonCursor<'_>,
) -> Result<Box<[ParsedParticipantClass]>, SchemaError> {
    parse_array(cursor, "participantClasses", false, parse_participant_class)
}

pub(in crate::module::geometry) fn parse_vehicle_profiles(
    cursor: &mut JsonCursor<'_>,
) -> Result<Box<[ParsedVehicleProfile]>, SchemaError> {
    parse_array(cursor, "vehicleProfiles", false, parse_vehicle_profile)
}

pub(in crate::module::geometry) fn parse_access_rules(
    cursor: &mut JsonCursor<'_>,
) -> Result<Box<[ParsedAccessRule]>, SchemaError> {
    parse_array(cursor, "accessRules", false, parse_access_rule)
}

pub(in crate::module::geometry) fn parse_static_routes(
    cursor: &mut JsonCursor<'_>,
) -> Result<Box<[ParsedStaticRoute]>, SchemaError> {
    parse_array(cursor, "staticRoutes", false, parse_static_route)
}

pub(in crate::module::geometry) fn parse_stop_lines(
    cursor: &mut JsonCursor<'_>,
) -> Result<Box<[ParsedStopLine]>, SchemaError> {
    parse_array(cursor, "stopLines", false, parse_stop_line)
}

pub(in crate::module::geometry) fn parse_maneuver_gates(
    cursor: &mut JsonCursor<'_>,
) -> Result<Box<[ParsedManeuverGate]>, SchemaError> {
    parse_array(cursor, "maneuverGates", false, parse_maneuver_gate)
}

pub(in crate::module::geometry) fn parse_waiting_zones(
    cursor: &mut JsonCursor<'_>,
) -> Result<Box<[ParsedWaitingZone]>, SchemaError> {
    parse_array(cursor, "waitingZones", false, parse_waiting_zone)
}

fn parse_participant_class(
    cursor: &mut JsonCursor<'_>,
) -> Result<ParsedParticipantClass, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new(["participantClassKey", "extends"]);
    let mut key = None;
    let mut extends = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => key = Some(parse_token(cursor, "participantClassKey")?),
            1 => extends = Some(parse_token(cursor, "extends")?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_indices(&[0], span)?;
    Ok(ParsedParticipantClass {
        participant_class_key: key.expect("required field checked"),
        extends,
        span,
    })
}

fn parse_vehicle_profile(cursor: &mut JsonCursor<'_>) -> Result<ParsedVehicleProfile, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new(["vehicleProfileKey", "participantClass", "iidm"]);
    let mut key = None;
    let mut class = None;
    let mut iidm = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => key = Some(parse_token(cursor, "vehicleProfileKey")?),
            1 => class = Some(parse_token(cursor, "participantClass")?),
            2 => iidm = Some(parse_iidm(cursor)?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok(ParsedVehicleProfile {
        vehicle_profile_key: key.expect("required field checked"),
        participant_class: class.expect("required field checked"),
        iidm: Box::new(iidm.expect("required field checked")),
        span,
    })
}

fn parse_iidm(cursor: &mut JsonCursor<'_>) -> Result<[RawNumber; 7], SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new([
        "lengthMeters",
        "desiredSpeedMetersPerSecond",
        "minGapMeters",
        "timeHeadwaySeconds",
        "maxAccelerationMetersPerSecondSquared",
        "comfortableDecelerationMetersPerSecondSquared",
        "emergencyDecelerationMetersPerSecondSquared",
    ]);
    let mut values = [None, None, None, None, None, None, None];
    parse_object_members(cursor, &mut fields, |index, cursor| {
        values[index] = Some(parse_raw_number(cursor)?);
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    fields.require_all(ByteSpan { start, end })?;
    let [a, b, c, d, e, f, g] = values;
    Ok([
        a.unwrap(),
        b.unwrap(),
        c.unwrap(),
        d.unwrap(),
        e.unwrap(),
        f.unwrap(),
        g.unwrap(),
    ])
}

fn parse_access_rule(cursor: &mut JsonCursor<'_>) -> Result<ParsedAccessRule, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new([
        "accessRuleKey",
        "target",
        "effect",
        "participantClasses",
        "regulation",
        "priority",
    ]);
    let mut key = None;
    let mut target = None;
    let mut effect = None;
    let mut classes = None;
    let mut regulation = None;
    let mut priority = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => key = Some(parse_token(cursor, "accessRuleKey")?),
            1 => target = Some(parse_access_target(cursor)?),
            2 => {
                let value = parse_string(cursor)?;
                effect = Some(match value.value.as_ref() {
                    "allow" => ParsedAccessEffect::Allow,
                    "deny" => ParsedAccessEffect::Deny,
                    _ => return Err(invalid_enum_value("effect", value)),
                });
            }
            3 => classes = Some(parse_unique_tokens(cursor, "participantClasses")?),
            4 => regulation = Some(parse_regulation(cursor)?),
            5 => priority = Some(parse_integer(cursor, "priority")?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_indices(&[0, 1, 2, 3, 5], span)?;
    let classes = classes.unwrap();
    if classes.is_empty() {
        return Err(SchemaError {
            kind: SchemaErrorKind::EmptyArray("participantClasses"),
            span,
        });
    }
    Ok(ParsedAccessRule {
        access_rule_key: key.unwrap(),
        target: target.unwrap(),
        effect: effect.unwrap(),
        participant_classes: classes,
        regulation,
        priority: priority.unwrap(),
        span,
    })
}

fn parse_access_target(cursor: &mut JsonCursor<'_>) -> Result<ParsedAccessTarget, SchemaError> {
    let start = cursor.begin_object()?.start;
    let names = [
        "kind",
        "laneEdge",
        "laneGroup",
        "roadSection",
        "maneuverPath",
        "facilityBand",
    ];
    let mut fields = ClosedFields::new(names);
    let mut kind = None;
    let mut values: [Option<SpannedString>; 5] = [None, None, None, None, None];
    parse_object_members(cursor, &mut fields, |index, cursor| {
        if index == 0 {
            kind = Some(parse_string(cursor)?)
        } else {
            values[index - 1] = Some(parse_token(cursor, names[index])?)
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_indices(&[0], span)?;
    let kind = kind.unwrap();
    let selected = match kind.value.as_ref() {
        "laneEdge" => 0,
        "laneGroup" => 1,
        "roadSection" => 2,
        "maneuverPath" => 3,
        "facilityBand" => 4,
        _ => return Err(invalid_enum_value("kind", kind)),
    };
    fields.require_indices(&[selected + 1], span)?;
    for index in 0..5 {
        if index != selected && fields.seen[index + 1].is_some() {
            return Err(SchemaError {
                kind: SchemaErrorKind::FieldNotAllowedForVariant {
                    field: names[index + 1],
                    variant: names[selected + 1],
                },
                span: fields.seen[index + 1].unwrap(),
            });
        }
    }
    let value = values[selected].take().unwrap();
    Ok(match selected {
        0 => ParsedAccessTarget::LaneEdge(value),
        1 => ParsedAccessTarget::LaneGroup(value),
        2 => ParsedAccessTarget::RoadSection(value),
        3 => ParsedAccessTarget::ManeuverPath(value),
        4 => ParsedAccessTarget::FacilityBand(value),
        _ => unreachable!(),
    })
}

fn parse_regulation(cursor: &mut JsonCursor<'_>) -> Result<ParsedRegulation, SchemaError> {
    let start = cursor.begin_object()?.start;
    let names = ["jurisdiction", "version", "source"];
    let mut fields = ClosedFields::new(names);
    let mut values: [Option<SpannedString>; 3] = [None, None, None];
    parse_object_members(cursor, &mut fields, |index, cursor| {
        let value = parse_string(cursor)?;
        if value.value.is_empty() || value.value.chars().count() > 128 {
            return Err(SchemaError {
                kind: SchemaErrorKind::EmptyString(names[index]),
                span: value.span,
            });
        }
        values[index] = Some(value);
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_indices(&[0, 1], span)?;
    let [jurisdiction, version, source] = values;
    Ok(ParsedRegulation {
        jurisdiction: jurisdiction.unwrap(),
        version: version.unwrap(),
        source,
        span,
    })
}

fn parse_static_route(cursor: &mut JsonCursor<'_>) -> Result<ParsedStaticRoute, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new(["staticRouteKey", "edgeSequence"]);
    let mut key = None;
    let mut edges = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => key = Some(parse_token(cursor, "staticRouteKey")?),
            1 => {
                edges = Some(parse_array(cursor, "edgeSequence", true, |c| {
                    parse_token(c, "edgeSequence")
                })?)
            }
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok(ParsedStaticRoute {
        static_route_key: key.unwrap(),
        edge_sequence: edges.unwrap(),
        span,
    })
}

fn parse_stop_line(cursor: &mut JsonCursor<'_>) -> Result<ParsedStopLine, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new(["stopLineKey", "laneEdge"]);
    let mut key = None;
    let mut edge = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => key = Some(parse_token(cursor, "stopLineKey")?),
            1 => edge = Some(parse_token(cursor, "laneEdge")?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok(ParsedStopLine {
        stop_line_key: key.unwrap(),
        lane_edge: edge.unwrap(),
        span,
    })
}

fn parse_maneuver_gate(cursor: &mut JsonCursor<'_>) -> Result<ParsedManeuverGate, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new([
        "maneuverGateKey",
        "maneuverPath",
        "transitionIndex",
        "stopLine",
        "signalControl",
    ]);
    let mut key = None;
    let mut path = None;
    let mut index = None;
    let mut stop = None;
    let mut control = None;
    parse_object_members(cursor, &mut fields, |field, cursor| {
        match field {
            0 => key = Some(parse_token(cursor, "maneuverGateKey")?),
            1 => path = Some(parse_token(cursor, "maneuverPath")?),
            2 => index = Some(parse_integer(cursor, "transitionIndex")?),
            3 => stop = Some(parse_token(cursor, "stopLine")?),
            4 => control = Some(parse_nullable_token(cursor, "signalControl")?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok(ParsedManeuverGate {
        maneuver_gate_key: key.unwrap(),
        maneuver_path: path.unwrap(),
        transition_index: index.unwrap(),
        stop_line: stop.unwrap(),
        signal_control: control.unwrap(),
        span,
    })
}

fn parse_waiting_zone(cursor: &mut JsonCursor<'_>) -> Result<ParsedWaitingZone, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new([
        "waitingZoneKey",
        "maneuverPath",
        "entryGate",
        "releaseGate",
        "maxOccupancy",
    ]);
    let mut key = None;
    let mut path = None;
    let mut entry = None;
    let mut release = None;
    let mut max = None;
    parse_object_members(cursor, &mut fields, |field, cursor| {
        match field {
            0 => key = Some(parse_token(cursor, "waitingZoneKey")?),
            1 => path = Some(parse_token(cursor, "maneuverPath")?),
            2 => entry = Some(parse_token(cursor, "entryGate")?),
            3 => release = Some(parse_token(cursor, "releaseGate")?),
            4 => max = Some(parse_integer(cursor, "maxOccupancy")?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok(ParsedWaitingZone {
        waiting_zone_key: key.unwrap(),
        maneuver_path: path.unwrap(),
        entry_gate: entry.unwrap(),
        release_gate: release.unwrap(),
        max_occupancy: max.unwrap(),
        span,
    })
}

fn parse_integer(
    cursor: &mut JsonCursor<'_>,
    field: &'static str,
) -> Result<RawNumber, SchemaError> {
    let value = parse_raw_number(cursor)?;
    if value.token.bytes().any(|b| matches!(b, b'.' | b'e' | b'E')) {
        return Err(SchemaError {
            kind: SchemaErrorKind::InvalidEnum {
                field,
                value: value.token.clone(),
            },
            span: value.span,
        });
    }
    Ok(value)
}
fn parse_nullable_token(
    cursor: &mut JsonCursor<'_>,
    field: &'static str,
) -> Result<Option<SpannedString>, SchemaError> {
    if cursor.next_is(b'n') {
        cursor.parse_literal(b"null")?;
        Ok(None)
    } else {
        parse_token(cursor, field).map(Some)
    }
}
fn invalid_enum_value(field: &'static str, value: SpannedString) -> SchemaError {
    SchemaError {
        kind: SchemaErrorKind::InvalidEnum {
            field,
            value: value.value,
        },
        span: value.span,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ParsedAccessEffect, ParsedSignalAspect, parse_access_rules, parse_maneuver_gates,
        parse_parking_spaces, parse_participant_classes, parse_signal_controllers,
        parse_signal_groups, parse_static_routes, parse_stop_lines, parse_vehicle_profiles,
        parse_waiting_zones,
    };
    use crate::module::geometry::json::JsonCursor;

    #[test]
    fn parses_signal_program_and_parking_space_records() {
        let mut groups = JsonCursor::new(br#"[{"signalGroupKey":"signal.main"}]"#).unwrap();
        assert_eq!(parse_signal_groups(&mut groups).unwrap().len(), 1);

        let mut controllers = JsonCursor::new(br#"[{"signalControllerKey":"controller.main","offsetSeconds":0,"signalGroups":["signal.main"],"phases":[{"signalPhaseKey":"phase.go","durationSeconds":30,"states":[{"signalGroup":"signal.main","aspect":"green"}]}]}]"#).unwrap();
        let controllers = parse_signal_controllers(&mut controllers).unwrap();
        assert_eq!(controllers.len(), 1);
        assert_eq!(
            controllers[0].phases[0].states[0].aspect,
            ParsedSignalAspect::Green
        );

        let mut spaces = JsonCursor::new(br#"[{"parkingSpaceKey":"parking.1","entry":{"laneEdge":"edge.a","progressMeters":1},"exit":{"laneEdge":"edge.a","progressMeters":2},"geometry":{"lateralOffsetMeters":-1,"headingOffsetRadians":0,"lengthMeters":5,"widthMeters":2.5}}]"#).unwrap();
        let spaces = parse_parking_spaces(&mut spaces).unwrap();
        assert!(spaces[0].parking_area.is_none());
        assert_eq!(spaces[0].geometry.width_meters.token.as_ref(), "2.5");
    }

    #[test]
    fn parses_remaining_overlay_record_families() {
        let mut classes = JsonCursor::new(
            br#"[{"participantClassKey":"class.car"},{"participantClassKey":"class.taxi","extends":"class.car"}]"#,
        )
        .unwrap();
        assert_eq!(parse_participant_classes(&mut classes).unwrap().len(), 2);

        let mut profiles = JsonCursor::new(br#"[{"vehicleProfileKey":"vehicle.car","participantClass":"class.car","iidm":{"lengthMeters":4.5,"desiredSpeedMetersPerSecond":15,"minGapMeters":2,"timeHeadwaySeconds":1.5,"maxAccelerationMetersPerSecondSquared":2,"comfortableDecelerationMetersPerSecondSquared":3,"emergencyDecelerationMetersPerSecondSquared":6}}]"#).unwrap();
        assert_eq!(parse_vehicle_profiles(&mut profiles).unwrap().len(), 1);

        let mut rules = JsonCursor::new(br#"[{"accessRuleKey":"access.main","target":{"kind":"laneEdge","laneEdge":"edge.main"},"effect":"allow","participantClasses":["class.car"],"regulation":{"jurisdiction":"CN","version":"1"},"priority":0}]"#).unwrap();
        let rules = parse_access_rules(&mut rules).unwrap();
        assert_eq!(rules[0].effect, ParsedAccessEffect::Allow);

        let mut routes =
            JsonCursor::new(br#"[{"staticRouteKey":"route.main","edgeSequence":["edge.main"]}]"#)
                .unwrap();
        assert_eq!(parse_static_routes(&mut routes).unwrap().len(), 1);

        let mut stops =
            JsonCursor::new(br#"[{"stopLineKey":"stop.main","laneEdge":"edge.main"}]"#).unwrap();
        assert_eq!(parse_stop_lines(&mut stops).unwrap().len(), 1);

        let mut gates = JsonCursor::new(br#"[{"maneuverGateKey":"gate.main","maneuverPath":"path.main","transitionIndex":0,"stopLine":"stop.main","signalControl":null}]"#).unwrap();
        assert!(
            parse_maneuver_gates(&mut gates).unwrap()[0]
                .signal_control
                .is_none()
        );

        let mut zones = JsonCursor::new(br#"[{"waitingZoneKey":"zone.main","maneuverPath":"path.main","entryGate":"gate.entry","releaseGate":"gate.release","maxOccupancy":2}]"#).unwrap();
        assert_eq!(parse_waiting_zones(&mut zones).unwrap().len(), 1);
    }
}
