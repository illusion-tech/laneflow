//! Frame、Road、reference curve 与 cross-section 的紧凑 wire records。

use std::collections::HashMap;

use super::{
    ByteSpan, ClosedFields, JsonCursor, JsonError, JsonErrorKind, SchemaError, SchemaErrorKind,
    SpannedString, parse_object_members, parse_string, parse_token,
};

#[derive(Debug)]
pub(in crate::module::geometry) struct RawNumber {
    pub(in crate::module::geometry) token: Box<str>,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedVec3 {
    pub(in crate::module::geometry) components: [RawNumber; 3],
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) enum ParsedCurveSegment {
    Line {
        end: ParsedVec3,
        span: ByteSpan,
    },
    CubicBezier {
        controls: Box<[ParsedVec3; 2]>,
        end: ParsedVec3,
        span: ByteSpan,
    },
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedCurve {
    pub(in crate::module::geometry) start: ParsedVec3,
    pub(in crate::module::geometry) segments: Box<[ParsedCurveSegment]>,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedFrameRecord {
    pub(in crate::module::geometry) frame_key: SpannedString,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedRoadRecord {
    pub(in crate::module::geometry) road_key: SpannedString,
    pub(in crate::module::geometry) frame: SpannedString,
    pub(in crate::module::geometry) reference_line: ParsedCurve,
    pub(in crate::module::geometry) cross_section_spans: Box<[ParsedCrossSectionSpan]>,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) enum ParsedEndStation {
    Number(RawNumber),
    End(ByteSpan),
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedCrossSectionSpan {
    pub(in crate::module::geometry) span_key: SpannedString,
    pub(in crate::module::geometry) corridor_key: SpannedString,
    pub(in crate::module::geometry) start_station_meters: RawNumber,
    pub(in crate::module::geometry) end_station_meters: ParsedEndStation,
    pub(in crate::module::geometry) reference_section_key: SpannedString,
    pub(in crate::module::geometry) reference_lane_key: SpannedString,
    pub(in crate::module::geometry) elements: Box<[ParsedCorridorElement]>,
    pub(in crate::module::geometry) road_sections: Box<[ParsedRoadSection]>,
    pub(in crate::module::geometry) facility_bands: Box<[ParsedFacilityBand]>,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) enum ParsedCorridorElement {
    RoadSection {
        section_key: SpannedString,
        span: ByteSpan,
    },
    FacilityBand {
        facility_band_key: SpannedString,
        span: ByteSpan,
    },
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedRoadSection {
    pub(in crate::module::geometry) section_key: SpannedString,
    pub(in crate::module::geometry) kind_id: SpannedString,
    pub(in crate::module::geometry) lanes: Box<[ParsedLane]>,
    pub(in crate::module::geometry) lane_groups: Box<[ParsedLaneGroup]>,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::module::geometry) enum ParsedLaneDirection {
    Forward,
    Backward,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedLane {
    pub(in crate::module::geometry) lane_key: SpannedString,
    pub(in crate::module::geometry) lane_edge_key: SpannedString,
    pub(in crate::module::geometry) direction: ParsedLaneDirection,
    pub(in crate::module::geometry) width_meters: RawNumber,
    pub(in crate::module::geometry) speed_limit_meters_per_second: RawNumber,
    pub(in crate::module::geometry) lane_group_key: Option<SpannedString>,
    pub(in crate::module::geometry) successors: Box<[SpannedString]>,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedLaneGroup {
    pub(in crate::module::geometry) lane_group_key: SpannedString,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedFacilityBand {
    pub(in crate::module::geometry) facility_band_key: SpannedString,
    pub(in crate::module::geometry) kind_id: SpannedString,
    pub(in crate::module::geometry) width_meters: RawNumber,
    pub(in crate::module::geometry) span: ByteSpan,
}

pub(in crate::module::geometry) fn parse_frame_records(
    cursor: &mut JsonCursor<'_>,
) -> Result<Box<[ParsedFrameRecord]>, SchemaError> {
    parse_array(cursor, "frames", true, parse_frame_record)
}

pub(in crate::module::geometry) fn parse_road_records(
    cursor: &mut JsonCursor<'_>,
) -> Result<Box<[ParsedRoadRecord]>, SchemaError> {
    parse_array(cursor, "roads", true, parse_road_record)
}

fn parse_frame_record(cursor: &mut JsonCursor<'_>) -> Result<ParsedFrameRecord, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new(["frameKey"]);
    let mut frame_key = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        debug_assert_eq!(index, 0);
        frame_key = Some(parse_token(cursor, "frameKey")?);
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok(ParsedFrameRecord {
        frame_key: frame_key.expect("required field checked"),
        span,
    })
}

fn parse_road_record(cursor: &mut JsonCursor<'_>) -> Result<ParsedRoadRecord, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new(["roadKey", "frame", "referenceLine", "crossSectionSpans"]);
    let mut road_key = None;
    let mut frame = None;
    let mut reference_line = None;
    let mut cross_section_spans = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => road_key = Some(parse_token(cursor, "roadKey")?),
            1 => frame = Some(parse_token(cursor, "frame")?),
            2 => reference_line = Some(parse_curve(cursor)?),
            3 => {
                cross_section_spans = Some(parse_array(
                    cursor,
                    "crossSectionSpans",
                    true,
                    parse_cross_section_span,
                )?)
            }
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok(ParsedRoadRecord {
        road_key: road_key.expect("required field checked"),
        frame: frame.expect("required field checked"),
        reference_line: reference_line.expect("required field checked"),
        cross_section_spans: cross_section_spans.expect("required field checked"),
        span,
    })
}

pub(in crate::module::geometry) fn parse_curve(
    cursor: &mut JsonCursor<'_>,
) -> Result<ParsedCurve, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new(["start", "segments"]);
    let mut curve_start = None;
    let mut segments = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => curve_start = Some(parse_vec3(cursor, "start")?),
            1 => segments = Some(parse_array(cursor, "segments", true, parse_curve_segment)?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok(ParsedCurve {
        start: curve_start.expect("required field checked"),
        segments: segments.expect("required field checked"),
        span,
    })
}

fn parse_curve_segment(cursor: &mut JsonCursor<'_>) -> Result<ParsedCurveSegment, SchemaError> {
    let start = cursor.begin_object()?.start;
    let names = ["kind", "control1", "control2", "end"];
    let mut fields = ClosedFields::new(names);
    let mut kind = None;
    let mut control1 = None;
    let mut control2 = None;
    let mut segment_end = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => kind = Some(parse_string(cursor)?),
            1 => control1 = Some(parse_vec3(cursor, "control1")?),
            2 => control2 = Some(parse_vec3(cursor, "control2")?),
            3 => segment_end = Some(parse_vec3(cursor, "end")?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_indices(&[0, 3], span)?;
    let kind = kind.expect("required field checked");
    match kind.value.as_ref() {
        "line" => {
            reject_variant_field(&fields, 1, "line")?;
            reject_variant_field(&fields, 2, "line")?;
            Ok(ParsedCurveSegment::Line {
                end: segment_end.expect("required field checked"),
                span,
            })
        }
        "cubicBezier" => {
            fields.require_indices(&[1, 2], span)?;
            Ok(ParsedCurveSegment::CubicBezier {
                controls: Box::new([
                    control1.expect("required field checked"),
                    control2.expect("required field checked"),
                ]),
                end: segment_end.expect("required field checked"),
                span,
            })
        }
        _ => Err(invalid_enum("kind", kind)),
    }
}

fn parse_vec3(cursor: &mut JsonCursor<'_>, field: &'static str) -> Result<ParsedVec3, SchemaError> {
    let start = cursor.begin_array()?.start;
    let mut components = Vec::with_capacity(3);
    if !cursor.next_is(b']') {
        loop {
            components.push(parse_raw_number(cursor)?);
            if cursor.next_is(b']') {
                break;
            }
            cursor.consume_comma()?;
        }
    }
    let end = cursor.end_array()?.end;
    if components.len() != 3 {
        return Err(SchemaError {
            kind: SchemaErrorKind::WrongArrayLength {
                field,
                expected: 3,
                actual: components.len(),
            },
            span: ByteSpan { start, end },
        });
    }
    Ok(ParsedVec3 {
        components: components.try_into().expect("length checked"),
        span: ByteSpan { start, end },
    })
}

fn parse_cross_section_span(
    cursor: &mut JsonCursor<'_>,
) -> Result<ParsedCrossSectionSpan, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new([
        "spanKey",
        "corridorKey",
        "startStationMeters",
        "endStationMeters",
        "referenceSectionKey",
        "referenceLaneKey",
        "elements",
        "roadSections",
        "facilityBands",
    ]);
    let mut span_key = None;
    let mut corridor_key = None;
    let mut start_station = None;
    let mut end_station = None;
    let mut reference_section_key = None;
    let mut reference_lane_key = None;
    let mut elements = None;
    let mut road_sections = None;
    let mut facility_bands = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => span_key = Some(parse_token(cursor, "spanKey")?),
            1 => corridor_key = Some(parse_token(cursor, "corridorKey")?),
            2 => start_station = Some(parse_raw_number(cursor)?),
            3 => end_station = Some(parse_end_station(cursor)?),
            4 => reference_section_key = Some(parse_token(cursor, "referenceSectionKey")?),
            5 => reference_lane_key = Some(parse_token(cursor, "referenceLaneKey")?),
            6 => {
                elements = Some(parse_array(
                    cursor,
                    "elements",
                    true,
                    parse_corridor_element,
                )?)
            }
            7 => {
                road_sections = Some(parse_array(
                    cursor,
                    "roadSections",
                    true,
                    parse_road_section,
                )?)
            }
            8 => {
                facility_bands = Some(parse_array(
                    cursor,
                    "facilityBands",
                    false,
                    parse_facility_band,
                )?)
            }
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok(ParsedCrossSectionSpan {
        span_key: span_key.expect("required field checked"),
        corridor_key: corridor_key.expect("required field checked"),
        start_station_meters: start_station.expect("required field checked"),
        end_station_meters: end_station.expect("required field checked"),
        reference_section_key: reference_section_key.expect("required field checked"),
        reference_lane_key: reference_lane_key.expect("required field checked"),
        elements: elements.expect("required field checked"),
        road_sections: road_sections.expect("required field checked"),
        facility_bands: facility_bands.expect("required field checked"),
        span,
    })
}

fn parse_end_station(cursor: &mut JsonCursor<'_>) -> Result<ParsedEndStation, SchemaError> {
    if cursor.next_is(b'"') {
        let value = parse_string(cursor)?;
        if value.value.as_ref() == "end" {
            Ok(ParsedEndStation::End(value.span))
        } else {
            Err(invalid_enum("endStationMeters", value))
        }
    } else {
        Ok(ParsedEndStation::Number(parse_raw_number(cursor)?))
    }
}

fn parse_corridor_element(
    cursor: &mut JsonCursor<'_>,
) -> Result<ParsedCorridorElement, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new(["kind", "sectionKey", "facilityBandKey"]);
    let mut kind = None;
    let mut section_key = None;
    let mut facility_band_key = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => kind = Some(parse_string(cursor)?),
            1 => section_key = Some(parse_token(cursor, "sectionKey")?),
            2 => facility_band_key = Some(parse_token(cursor, "facilityBandKey")?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_indices(&[0], span)?;
    let kind = kind.expect("required field checked");
    match kind.value.as_ref() {
        "roadSection" => {
            fields.require_indices(&[1], span)?;
            reject_variant_field(&fields, 2, "roadSection")?;
            Ok(ParsedCorridorElement::RoadSection {
                section_key: section_key.expect("required field checked"),
                span,
            })
        }
        "facilityBand" => {
            fields.require_indices(&[2], span)?;
            reject_variant_field(&fields, 1, "facilityBand")?;
            Ok(ParsedCorridorElement::FacilityBand {
                facility_band_key: facility_band_key.expect("required field checked"),
                span,
            })
        }
        _ => Err(invalid_enum("kind", kind)),
    }
}

fn parse_road_section(cursor: &mut JsonCursor<'_>) -> Result<ParsedRoadSection, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new(["sectionKey", "kindId", "lanes", "laneGroups"]);
    let mut section_key = None;
    let mut kind_id = None;
    let mut lanes = None;
    let mut lane_groups = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => section_key = Some(parse_token(cursor, "sectionKey")?),
            1 => kind_id = Some(parse_token(cursor, "kindId")?),
            2 => lanes = Some(parse_array(cursor, "lanes", true, parse_lane)?),
            3 => lane_groups = Some(parse_array(cursor, "laneGroups", false, parse_lane_group)?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok(ParsedRoadSection {
        section_key: section_key.expect("required field checked"),
        kind_id: kind_id.expect("required field checked"),
        lanes: lanes.expect("required field checked"),
        lane_groups: lane_groups.expect("required field checked"),
        span,
    })
}

fn parse_lane(cursor: &mut JsonCursor<'_>) -> Result<ParsedLane, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new([
        "laneKey",
        "laneEdgeKey",
        "direction",
        "widthMeters",
        "speedLimitMetersPerSecond",
        "laneGroupKey",
        "successors",
    ]);
    let mut lane_key = None;
    let mut lane_edge_key = None;
    let mut direction = None;
    let mut width = None;
    let mut speed = None;
    let mut lane_group_key = None;
    let mut successors = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => lane_key = Some(parse_token(cursor, "laneKey")?),
            1 => lane_edge_key = Some(parse_token(cursor, "laneEdgeKey")?),
            2 => {
                let value = parse_string(cursor)?;
                direction = Some(match value.value.as_ref() {
                    "forward" => ParsedLaneDirection::Forward,
                    "backward" => ParsedLaneDirection::Backward,
                    _ => return Err(invalid_enum("direction", value)),
                });
            }
            3 => width = Some(parse_raw_number(cursor)?),
            4 => speed = Some(parse_raw_number(cursor)?),
            5 => lane_group_key = Some(parse_token(cursor, "laneGroupKey")?),
            6 => successors = Some(parse_unique_tokens(cursor, "successors")?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_indices(&[0, 1, 2, 3, 4, 6], span)?;
    Ok(ParsedLane {
        lane_key: lane_key.expect("required field checked"),
        lane_edge_key: lane_edge_key.expect("required field checked"),
        direction: direction.expect("required field checked"),
        width_meters: width.expect("required field checked"),
        speed_limit_meters_per_second: speed.expect("required field checked"),
        lane_group_key,
        successors: successors.expect("required field checked"),
        span,
    })
}

fn parse_lane_group(cursor: &mut JsonCursor<'_>) -> Result<ParsedLaneGroup, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new(["laneGroupKey"]);
    let mut lane_group_key = None;
    parse_object_members(cursor, &mut fields, |_, cursor| {
        lane_group_key = Some(parse_token(cursor, "laneGroupKey")?);
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok(ParsedLaneGroup {
        lane_group_key: lane_group_key.expect("required field checked"),
        span,
    })
}

fn parse_facility_band(cursor: &mut JsonCursor<'_>) -> Result<ParsedFacilityBand, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new(["facilityBandKey", "kindId", "widthMeters"]);
    let mut facility_band_key = None;
    let mut kind_id = None;
    let mut width = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => facility_band_key = Some(parse_token(cursor, "facilityBandKey")?),
            1 => kind_id = Some(parse_token(cursor, "kindId")?),
            2 => width = Some(parse_raw_number(cursor)?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok(ParsedFacilityBand {
        facility_band_key: facility_band_key.expect("required field checked"),
        kind_id: kind_id.expect("required field checked"),
        width_meters: width.expect("required field checked"),
        span,
    })
}

pub(in crate::module::geometry) fn parse_raw_number(
    cursor: &mut JsonCursor<'_>,
) -> Result<RawNumber, SchemaError> {
    let (token, span) = cursor.parse_number_token()?;
    Ok(RawNumber {
        token: token.into(),
        span,
    })
}

pub(in crate::module::geometry) fn parse_unique_tokens(
    cursor: &mut JsonCursor<'_>,
    field: &'static str,
) -> Result<Box<[SpannedString]>, SchemaError> {
    cursor.begin_array()?;
    let mut values = Vec::new();
    let mut seen = HashMap::<Box<str>, ByteSpan>::new();
    // 与 parse_imports 同款的瞬时 duplicate-key 表入账：insert 前 grow，
    // 成功返回前归还；错误路径不归还，保持失败安全方向。
    let mut seen_scratch_bytes = 0_u64;
    if !cursor.next_is(b']') {
        loop {
            let value = parse_token(cursor, field)?;
            let entry_bytes = value.value.len() as u64 + size_of::<(Box<str>, ByteSpan)>() as u64;
            cursor
                .scratch()
                .grow(entry_bytes)
                .map_err(|exceeded| SchemaError {
                    kind: SchemaErrorKind::Json(JsonError {
                        kind: JsonErrorKind::StageScratchExceeded(exceeded),
                        span: value.span,
                    }),
                    span: value.span,
                })?;
            seen_scratch_bytes += entry_bytes;
            if seen.insert(value.value.clone(), value.span).is_some() {
                return Err(SchemaError {
                    kind: SchemaErrorKind::DuplicateArrayItem {
                        field,
                        value: value.value,
                    },
                    span: value.span,
                });
            }
            let span = value.span;
            cursor
                .push_vec(&mut values, value)
                .map_err(|exceeded| super::stage_scratch_schema_error(exceeded, span))?;
            if cursor.next_is(b']') {
                break;
            }
            cursor.consume_comma()?;
        }
    }
    cursor.end_array()?;
    cursor.scratch().shrink(seen_scratch_bytes);
    cursor.finish_vec(&values);
    Ok(values.into_boxed_slice())
}

pub(in crate::module::geometry) fn parse_array<T>(
    cursor: &mut JsonCursor<'_>,
    field: &'static str,
    non_empty: bool,
    mut parse_item: impl FnMut(&mut JsonCursor<'_>) -> Result<T, SchemaError>,
) -> Result<Box<[T]>, SchemaError> {
    let start = cursor.begin_array()?.start;
    let mut values = Vec::new();
    if !cursor.next_is(b']') {
        loop {
            let value = parse_item(cursor)?;
            cursor.push_vec(&mut values, value).map_err(|exceeded| {
                super::stage_scratch_schema_error(
                    exceeded,
                    ByteSpan {
                        start,
                        end: cursor.offset(),
                    },
                )
            })?;
            if cursor.next_is(b']') {
                break;
            }
            cursor.consume_comma()?;
        }
    }
    let end = cursor.end_array()?.end;
    if non_empty && values.is_empty() {
        return Err(SchemaError {
            kind: SchemaErrorKind::EmptyArray(field),
            span: ByteSpan { start, end },
        });
    }
    cursor.finish_vec(&values);
    Ok(values.into_boxed_slice())
}

fn reject_variant_field<const N: usize>(
    fields: &ClosedFields<N>,
    index: usize,
    variant: &'static str,
) -> Result<(), SchemaError> {
    if let Some(span) = fields.seen[index] {
        Err(SchemaError {
            kind: SchemaErrorKind::FieldNotAllowedForVariant {
                field: fields.names[index],
                variant,
            },
            span,
        })
    } else {
        Ok(())
    }
}

fn invalid_enum(field: &'static str, value: SpannedString) -> SchemaError {
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
        ParsedCurveSegment, ParsedEndStation, ParsedLaneDirection, parse_frame_records,
        parse_road_records,
    };
    use crate::module::geometry::json::JsonCursor;
    use crate::module::geometry::schema::SchemaErrorKind;

    const MINIMAL_ROADS: &[u8] = br#"[{"roadKey":"road.main","frame":"frame.main","referenceLine":{"start":[0,0.0,-0],"segments":[{"kind":"line","end":[10,0,0]},{"kind":"cubicBezier","control1":[11,0,0],"control2":[19,0,5],"end":[20,0,5]}]},"crossSectionSpans":[{"spanKey":"span.main","corridorKey":"corridor.main","startStationMeters":0,"endStationMeters":"end","referenceSectionKey":"section.main","referenceLaneKey":"lane.forward","elements":[{"kind":"roadSection","sectionKey":"section.main"},{"kind":"facilityBand","facilityBandKey":"facility.sidewalk"}],"roadSections":[{"sectionKey":"section.main","kindId":"road.vehicle","lanes":[{"laneKey":"lane.forward","laneEdgeKey":"edge.forward","direction":"forward","widthMeters":3.5,"speedLimitMetersPerSecond":13.9,"successors":[]}],"laneGroups":[]}],"facilityBands":[{"facilityBandKey":"facility.sidewalk","kindId":"facility.sidewalk","widthMeters":2}]}]}]"#;

    #[test]
    fn parses_frame_road_curve_and_cross_section_records_without_losing_number_tokens() {
        let mut frames = JsonCursor::new(br#"[{"frameKey":"frame.main"}]"#).unwrap();
        let frames = parse_frame_records(&mut frames).unwrap();
        assert_eq!(frames[0].frame_key.value.as_ref(), "frame.main");

        let mut roads = JsonCursor::new(MINIMAL_ROADS).unwrap();
        let roads = parse_road_records(&mut roads).unwrap();
        let road = &roads[0];
        assert_eq!(
            road.reference_line.start.components[1].token.as_ref(),
            "0.0"
        );
        assert_eq!(road.reference_line.start.components[2].token.as_ref(), "-0");
        assert!(matches!(
            road.reference_line.segments[0],
            ParsedCurveSegment::Line { .. }
        ));
        assert!(matches!(
            road.reference_line.segments[1],
            ParsedCurveSegment::CubicBezier { .. }
        ));
        let span = &road.cross_section_spans[0];
        assert!(matches!(span.end_station_meters, ParsedEndStation::End(_)));
        assert_eq!(
            span.road_sections[0].lanes[0].direction,
            ParsedLaneDirection::Forward
        );
        assert_eq!(span.facility_bands.len(), 1);
    }

    #[test]
    fn rejects_wrong_vec3_empty_required_arrays_and_variant_field_leakage() {
        let mut wrong_vec = JsonCursor::new(
            br#"[{"roadKey":"r","frame":"f","referenceLine":{"start":[0,0],"segments":[{"kind":"line","end":[1,0,0]}]},"crossSectionSpans":[]}]"#,
        )
        .unwrap();
        assert!(matches!(
            parse_road_records(&mut wrong_vec).unwrap_err().kind,
            SchemaErrorKind::WrongArrayLength {
                field: "start",
                expected: 3,
                actual: 2
            }
        ));

        let mut empty = JsonCursor::new(b"[]").unwrap();
        assert_eq!(
            parse_frame_records(&mut empty).unwrap_err().kind,
            SchemaErrorKind::EmptyArray("frames")
        );

        let mut leaked = JsonCursor::new(
            br#"[{"roadKey":"r","frame":"f","referenceLine":{"start":[0,0,0],"segments":[{"kind":"line","control1":[0,0,0],"end":[1,0,0]}]},"crossSectionSpans":[]}]"#,
        )
        .unwrap();
        assert!(matches!(
            parse_road_records(&mut leaked).unwrap_err().kind,
            SchemaErrorKind::FieldNotAllowedForVariant {
                field: "control1",
                variant: "line"
            }
        ));

        let mut duplicate = JsonCursor::new(br#"["edge.next","edge.next"]"#).unwrap();
        assert!(matches!(
            super::parse_unique_tokens(&mut duplicate, "successors")
                .unwrap_err()
                .kind,
            SchemaErrorKind::DuplicateArrayItem {
                field: "successors",
                ..
            }
        ));
    }
}
