//! Geometry v1 的 schema-specific closed-shape 解析原语。

#![allow(
    dead_code,
    reason = "consumed by the following document-record parser slice"
)]

use std::collections::HashMap;

use super::json::{ByteSpan, JsonCursor, JsonError, JsonErrorKind, StageScratchMeter};

mod junction;
mod numeric;
mod overlay;
mod road;

pub(in crate::module::geometry) use junction::ParsedInternalEdge;
pub(crate) use numeric::{
    FrozenCanonicalPoint, FrozenGeometryPayload, FrozenInternalEdgeCurve, FrozenLateralCurve,
    LateralIntentKind,
};
pub(in crate::module::geometry) use numeric::{
    FrozenCrossSectionLayout, FrozenRoadReference, FrozenRoadStationing, NumericFreezeError,
    NumericFreezeViolation, frozen_polyline_length_meters, parse_finite,
};
pub(in crate::module::geometry) use overlay::{
    ParsedAccessEffect, ParsedAccessTarget, ParsedParkingAnchor, ParsedSignalAspect,
};
pub(in crate::module::geometry) use road::{
    ParsedCorridorElement, ParsedCrossSectionSpan, ParsedCurveSegment, ParsedFacilityBand,
    ParsedLaneDirection, ParsedRoadSection, RawNumber,
};

#[derive(Debug, Eq, PartialEq)]
pub(super) enum SchemaErrorKind {
    Json(JsonError),
    UnknownField(Box<str>),
    DuplicateField(&'static str),
    MissingField(&'static str),
    UnexpectedConstant {
        field: &'static str,
        expected: &'static str,
    },
    InvalidToken(&'static str),
    EmptyString(&'static str),
    DuplicateImport(Box<str>),
    DuplicateArrayItem {
        field: &'static str,
        value: Box<str>,
    },
    InvalidDigest(&'static str),
    InvalidRandomSeed,
    InvalidProvenanceKind(Box<str>),
    FieldNotAllowedForProvenance {
        field: &'static str,
        kind: &'static str,
    },
    FieldNotAllowedForVariant {
        field: &'static str,
        variant: &'static str,
    },
    InvalidEnum {
        field: &'static str,
        value: Box<str>,
    },
    EmptyArray(&'static str),
    WrongArrayLength {
        field: &'static str,
        expected: usize,
        actual: usize,
    },
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct SchemaError {
    pub(super) kind: SchemaErrorKind,
    pub(super) span: ByteSpan,
}

impl From<JsonError> for SchemaError {
    fn from(error: JsonError) -> Self {
        Self {
            span: error.span,
            kind: SchemaErrorKind::Json(error),
        }
    }
}

#[derive(Debug)]
pub(in crate::module::geometry) struct SpannedString {
    pub(in crate::module::geometry) value: Box<str>,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) enum ParsedProvenance {
    Direct {
        description: SpannedString,
    },
    Generated {
        generator_build_id: SpannedString,
        parameters_and_inputs_digest: [u8; 32],
        frontend_options_digest: [u8; 32],
        random_seed: Option<u64>,
        description: SpannedString,
    },
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedModuleRecord {
    pub(in crate::module::geometry) namespace: SpannedString,
    pub(in crate::module::geometry) document_key: SpannedString,
    pub(in crate::module::geometry) imports: Box<[SpannedString]>,
    pub(in crate::module::geometry) provenance: ParsedProvenance,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedUnitsRecord {
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedGeometryDocument {
    pub(in crate::module::geometry) geometry_version_span: ByteSpan,
    pub(in crate::module::geometry) module: ParsedModuleRecord,
    pub(in crate::module::geometry) units: ParsedUnitsRecord,
    pub(in crate::module::geometry) frames: Box<[road::ParsedFrameRecord]>,
    pub(in crate::module::geometry) roads: Box<[road::ParsedRoadRecord]>,
    pub(in crate::module::geometry) junctions: Box<[junction::ParsedJunctionRecord]>,
    pub(in crate::module::geometry) overlays: overlay::ParsedOverlays,
    pub(in crate::module::geometry) span: ByteSpan,
}

struct ClosedFields<const N: usize> {
    names: [&'static str; N],
    seen: [Option<ByteSpan>; N],
}

impl<const N: usize> ClosedFields<N> {
    const fn new(names: [&'static str; N]) -> Self {
        Self {
            names,
            seen: [None; N],
        }
    }

    fn observe(&mut self, name: &str, span: ByteSpan) -> Result<usize, SchemaError> {
        let Some(index) = self.names.iter().position(|candidate| *candidate == name) else {
            return Err(SchemaError {
                kind: SchemaErrorKind::UnknownField(name.into()),
                span,
            });
        };
        if self.seen[index].replace(span).is_some() {
            return Err(SchemaError {
                kind: SchemaErrorKind::DuplicateField(self.names[index]),
                span,
            });
        }
        Ok(index)
    }

    fn require_all(&self, fallback_span: ByteSpan) -> Result<(), SchemaError> {
        for (index, span) in self.seen.iter().enumerate() {
            if span.is_none() {
                return Err(SchemaError {
                    kind: SchemaErrorKind::MissingField(self.names[index]),
                    span: fallback_span,
                });
            }
        }
        Ok(())
    }

    fn require_indices(
        &self,
        indices: &[usize],
        fallback_span: ByteSpan,
    ) -> Result<(), SchemaError> {
        for index in indices {
            if self.seen[*index].is_none() {
                return Err(SchemaError {
                    kind: SchemaErrorKind::MissingField(self.names[*index]),
                    span: fallback_span,
                });
            }
        }
        Ok(())
    }
}

pub(super) fn parse_module_record(
    cursor: &mut JsonCursor<'_>,
) -> Result<ParsedModuleRecord, SchemaError> {
    let object_start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new(["namespace", "documentKey", "imports", "provenance"]);
    let mut namespace = None;
    let mut document_key = None;
    let mut imports = None;
    let mut provenance = None;

    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => namespace = Some(parse_token(cursor, "namespace")?),
            1 => document_key = Some(parse_token(cursor, "documentKey")?),
            2 => imports = Some(parse_imports(cursor)?),
            3 => provenance = Some(parse_provenance(cursor)?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let object_end = cursor.end_object()?.end;
    let object_span = ByteSpan {
        start: object_start,
        end: object_end,
    };
    fields.require_all(object_span)?;
    Ok(ParsedModuleRecord {
        namespace: namespace.expect("required field checked"),
        document_key: document_key.expect("required field checked"),
        imports: imports.expect("required field checked"),
        provenance: provenance.expect("required field checked"),
        span: object_span,
    })
}

pub(super) fn parse_units_record(
    cursor: &mut JsonCursor<'_>,
) -> Result<ParsedUnitsRecord, SchemaError> {
    let object_start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new(["distance", "angle", "speed", "time"]);
    parse_object_members(cursor, &mut fields, |index, cursor| {
        let (field, expected) = match index {
            0 => ("distance", "meter"),
            1 => ("angle", "radian"),
            2 => ("speed", "meter-per-second"),
            3 => ("time", "second"),
            _ => unreachable!(),
        };
        let (actual, span) = cursor.parse_string()?;
        if actual != expected {
            return Err(SchemaError {
                kind: SchemaErrorKind::UnexpectedConstant { field, expected },
                span,
            });
        }
        Ok(())
    })?;
    let object_end = cursor.end_object()?.end;
    let span = ByteSpan {
        start: object_start,
        end: object_end,
    };
    fields.require_all(span)?;
    Ok(ParsedUnitsRecord { span })
}

pub(super) fn parse_geometry_document(
    source: &[u8],
) -> Result<ParsedGeometryDocument, SchemaError> {
    parse_geometry_document_with_scratch(source, u64::MAX)
}

/// 与 [`parse_geometry_document`] 相同，但把 parser 栈帧、duplicate-key 表等
/// §7.1 阶段 1 暂存计入 `scratch_limit`（`StageScratchBytes` 维度），超限失败关闭。
pub(in crate::module::geometry) fn parse_geometry_document_with_scratch(
    source: &[u8],
    scratch_limit: u64,
) -> Result<ParsedGeometryDocument, SchemaError> {
    let mut cursor = JsonCursor::new_with_scratch(source, scratch_limit)?;
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new([
        "geometryVersion",
        "module",
        "units",
        "frames",
        "roads",
        "junctions",
        "overlays",
    ]);
    let mut version_span = None;
    let mut module = None;
    let mut units = None;
    let mut frames = None;
    let mut roads = None;
    let mut junctions = None;
    let mut overlays = None;
    parse_object_members(&mut cursor, &mut fields, |index, cursor| {
        match index {
            0 => {
                let (version, span) = cursor.parse_string()?;
                if version != "1" {
                    return Err(SchemaError {
                        kind: SchemaErrorKind::UnexpectedConstant {
                            field: "geometryVersion",
                            expected: "1",
                        },
                        span,
                    });
                }
                version_span = Some(span);
            }
            1 => module = Some(parse_module_record(cursor)?),
            2 => units = Some(parse_units_record(cursor)?),
            3 => frames = Some(road::parse_frame_records(cursor)?),
            4 => roads = Some(road::parse_road_records(cursor)?),
            5 => junctions = Some(junction::parse_junction_records(cursor)?),
            6 => overlays = Some(overlay::parse_overlays(cursor)?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    cursor.finish()?;
    Ok(ParsedGeometryDocument {
        geometry_version_span: version_span.unwrap(),
        module: module.unwrap(),
        units: units.unwrap(),
        frames: frames.unwrap(),
        roads: roads.unwrap(),
        junctions: junctions.unwrap(),
        overlays: overlays.unwrap(),
        span,
    })
}

pub(in crate::module::geometry) fn freeze_reference_lines(
    document: &ParsedGeometryDocument,
    accuracy_profile: super::GeometryAccuracyProfile,
    direction_profile: super::GeometryDirectionProfile,
    meter: &mut StageScratchMeter,
) -> Result<Box<[FrozenRoadReference]>, NumericFreezeError> {
    numeric::freeze_reference_lines(document, accuracy_profile, direction_profile, meter)
}

pub(in crate::module::geometry) fn freeze_stationing(
    document: &ParsedGeometryDocument,
    direction_profile: super::GeometryDirectionProfile,
    meter: &mut StageScratchMeter,
) -> Result<Box<[FrozenRoadStationing]>, NumericFreezeError> {
    numeric::freeze_stationing(document, direction_profile, meter)
}

pub(in crate::module::geometry) fn freeze_cross_section_layouts(
    document: &ParsedGeometryDocument,
    meter: &mut StageScratchMeter,
) -> Result<Box<[FrozenCrossSectionLayout]>, NumericFreezeError> {
    numeric::freeze_cross_section_layouts(document, meter)
}

pub(in crate::module::geometry) fn freeze_lateral_curves(
    document: &ParsedGeometryDocument,
    stationing: &[FrozenRoadStationing],
    layouts: &[FrozenCrossSectionLayout],
    accuracy_profile: super::GeometryAccuracyProfile,
    direction_profile: super::GeometryDirectionProfile,
    meter: &mut StageScratchMeter,
) -> Result<Box<[FrozenLateralCurve]>, NumericFreezeError> {
    numeric::freeze_lateral_curves(
        document,
        stationing,
        layouts,
        accuracy_profile,
        direction_profile,
        meter,
    )
}

pub(in crate::module::geometry) fn freeze_geometry_payload(
    document: &ParsedGeometryDocument,
    accuracy_profile: super::GeometryAccuracyProfile,
    direction_profile: super::GeometryDirectionProfile,
    geometry_point_limit: u64,
    meter: &mut StageScratchMeter,
) -> Result<FrozenGeometryPayload, NumericFreezeError> {
    numeric::freeze_geometry_payload(
        document,
        accuracy_profile,
        direction_profile,
        geometry_point_limit,
        meter,
    )
}

fn parse_object_members<const N: usize>(
    cursor: &mut JsonCursor<'_>,
    fields: &mut ClosedFields<N>,
    mut parse_value: impl FnMut(usize, &mut JsonCursor<'_>) -> Result<(), SchemaError>,
) -> Result<(), SchemaError> {
    if cursor.next_is(b'}') {
        return Ok(());
    }
    loop {
        let (name, key_span) = cursor.parse_string()?;
        let index = fields.observe(&name, key_span)?;
        cursor.consume_colon()?;
        parse_value(index, cursor)?;
        if cursor.next_is(b'}') {
            return Ok(());
        }
        cursor.consume_comma()?;
    }
}

fn parse_imports(cursor: &mut JsonCursor<'_>) -> Result<Box<[SpannedString]>, SchemaError> {
    cursor.begin_array()?;
    let mut imports = Vec::new();
    let mut seen = HashMap::<Box<str>, ByteSpan>::new();
    // duplicate-key 表是瞬时暂存：每条 import 的键字节与表项槽位在 insert 前入账，
    // 成功返回前归还全部已记字节；错误路径不归还，保持失败安全方向。
    let mut seen_scratch_bytes = 0_u64;
    if !cursor.next_is(b']') {
        loop {
            let import = parse_token(cursor, "imports")?;
            let entry_bytes = import.value.len() as u64 + size_of::<(Box<str>, ByteSpan)>() as u64;
            cursor
                .scratch()
                .grow(entry_bytes)
                .map_err(|exceeded| SchemaError {
                    kind: SchemaErrorKind::Json(JsonError {
                        kind: JsonErrorKind::StageScratchExceeded(exceeded),
                        span: import.span,
                    }),
                    span: import.span,
                })?;
            seen_scratch_bytes += entry_bytes;
            if seen.insert(import.value.clone(), import.span).is_some() {
                return Err(SchemaError {
                    kind: SchemaErrorKind::DuplicateImport(import.value),
                    span: import.span,
                });
            }
            imports.push(import);
            if cursor.next_is(b']') {
                break;
            }
            cursor.consume_comma()?;
        }
    }
    cursor.end_array()?;
    cursor.scratch().shrink(seen_scratch_bytes);
    Ok(imports.into_boxed_slice())
}

fn parse_provenance(cursor: &mut JsonCursor<'_>) -> Result<ParsedProvenance, SchemaError> {
    let object_start = cursor.begin_object()?.start;
    let names = [
        "kind",
        "generatorBuildId",
        "parametersAndInputsDigest",
        "frontendOptionsDigest",
        "randomSeed",
        "description",
    ];
    let mut fields = ClosedFields::new(names);
    let mut kind = None;
    let mut generator_build_id = None;
    let mut parameters_digest = None;
    let mut frontend_digest = None;
    let mut random_seed = None;
    let mut description = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => kind = Some(parse_string(cursor)?),
            1 => generator_build_id = Some(parse_token(cursor, "generatorBuildId")?),
            2 => parameters_digest = Some(parse_digest(cursor, "parametersAndInputsDigest")?),
            3 => frontend_digest = Some(parse_digest(cursor, "frontendOptionsDigest")?),
            4 => random_seed = Some(parse_random_seed(cursor)?),
            5 => {
                let value = parse_string(cursor)?;
                if value.value.is_empty() {
                    return Err(SchemaError {
                        kind: SchemaErrorKind::EmptyString("description"),
                        span: value.span,
                    });
                }
                description = Some(value);
            }
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let object_end = cursor.end_object()?.end;
    let object_span = ByteSpan {
        start: object_start,
        end: object_end,
    };
    let Some(kind) = kind else {
        return Err(SchemaError {
            kind: SchemaErrorKind::MissingField("kind"),
            span: object_span,
        });
    };
    let Some(description) = description else {
        return Err(SchemaError {
            kind: SchemaErrorKind::MissingField("description"),
            span: object_span,
        });
    };

    match kind.value.as_ref() {
        "direct" => {
            for (index, name) in names.iter().enumerate().take(5).skip(1) {
                if let Some(span) = fields.seen[index] {
                    return Err(SchemaError {
                        kind: SchemaErrorKind::FieldNotAllowedForProvenance {
                            field: name,
                            kind: "direct",
                        },
                        span,
                    });
                }
            }
            Ok(ParsedProvenance::Direct { description })
        }
        "generated" => {
            for (index, span) in fields.seen.iter().enumerate() {
                if span.is_none() {
                    return Err(SchemaError {
                        kind: SchemaErrorKind::MissingField(names[index]),
                        span: object_span,
                    });
                }
            }
            Ok(ParsedProvenance::Generated {
                generator_build_id: generator_build_id.expect("required field checked"),
                parameters_and_inputs_digest: parameters_digest.expect("required field checked"),
                frontend_options_digest: frontend_digest.expect("required field checked"),
                random_seed: random_seed.expect("required field checked"),
                description,
            })
        }
        _ => Err(SchemaError {
            kind: SchemaErrorKind::InvalidProvenanceKind(kind.value),
            span: kind.span,
        }),
    }
}

fn parse_token(
    cursor: &mut JsonCursor<'_>,
    field: &'static str,
) -> Result<SpannedString, SchemaError> {
    let value = parse_string(cursor)?;
    let mut bytes = value.value.bytes();
    let valid_first = bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric());
    if !valid_first
        || !bytes.all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
    {
        return Err(SchemaError {
            kind: SchemaErrorKind::InvalidToken(field),
            span: value.span,
        });
    }
    Ok(value)
}

fn parse_string(cursor: &mut JsonCursor<'_>) -> Result<SpannedString, SchemaError> {
    let (value, span) = cursor.parse_string()?;
    Ok(SpannedString {
        value: value.into_boxed_str(),
        span,
    })
}

fn parse_digest(cursor: &mut JsonCursor<'_>, field: &'static str) -> Result<[u8; 32], SchemaError> {
    let value = parse_string(cursor)?;
    if value.value.len() != 64
        || !value
            .value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(SchemaError {
            kind: SchemaErrorKind::InvalidDigest(field),
            span: value.span,
        });
    }
    let mut digest = [0_u8; 32];
    for (index, chunk) in value.value.as_bytes().chunks_exact(2).enumerate() {
        digest[index] = (hex_nibble(chunk[0]) << 4) | hex_nibble(chunk[1]);
    }
    Ok(digest)
}

fn parse_random_seed(cursor: &mut JsonCursor<'_>) -> Result<Option<u64>, SchemaError> {
    if cursor.next_is(b'n') {
        cursor.parse_literal(b"null")?;
        return Ok(None);
    }
    let value = parse_string(cursor)?;
    let bytes = value.value.as_bytes();
    let valid_shape = bytes == b"0"
        || (matches!(bytes.first(), Some(b'1'..=b'9')) && bytes.iter().all(u8::is_ascii_digit));
    if !valid_shape {
        return Err(SchemaError {
            kind: SchemaErrorKind::InvalidRandomSeed,
            span: value.span,
        });
    }
    value.value.parse().map(Some).map_err(|_| SchemaError {
        kind: SchemaErrorKind::InvalidRandomSeed,
        span: value.span,
    })
}

const fn hex_nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => unreachable!(),
    }
}

#[cfg(test)]
pub(super) const MINIMAL_DOCUMENT: &[u8] = br#"{"overlays":{"signalGroups":[],"signalControllers":[],"parkingAreas":[],"parkingSpaces":[],"participantClasses":[],"vehicleProfiles":[],"accessRules":[],"staticRoutes":[],"stopLines":[],"maneuverGates":[],"waitingZones":[]},"junctions":[],"roads":[{"roadKey":"road.main","frame":"frame.main","referenceLine":{"start":[0,0,0],"segments":[{"kind":"line","end":[10,0,0]}]},"crossSectionSpans":[{"spanKey":"span.main","corridorKey":"corridor.main","startStationMeters":0,"endStationMeters":"end","referenceSectionKey":"section.main","referenceLaneKey":"lane.main","elements":[{"kind":"roadSection","sectionKey":"section.main"}],"roadSections":[{"sectionKey":"section.main","kindId":"road.vehicle","lanes":[{"laneKey":"lane.main","laneEdgeKey":"edge.main","direction":"forward","widthMeters":3.5,"speedLimitMetersPerSecond":10,"successors":[]}],"laneGroups":[]}],"facilityBands":[]}] }],"frames":[{"frameKey":"frame.main"}],"units":{"distance":"meter","angle":"radian","speed":"meter-per-second","time":"second"},"module":{"namespace":"city/main","documentKey":"source/main","imports":[],"provenance":{"kind":"direct","description":"minimal"}},"geometryVersion":"1"}"#;

#[cfg(test)]
mod tests {
    use super::{
        ParsedProvenance, SchemaErrorKind, parse_geometry_document, parse_module_record,
        parse_units_record,
    };
    use crate::module::geometry::json::JsonCursor;

    #[test]
    fn module_record_accepts_reordered_direct_and_generated_closed_shapes() {
        let mut direct = JsonCursor::new(
            br#"{"provenance":{"description":"direct source","kind":"direct"},"imports":["city/base"],"documentKey":"source/main","namespace":"city/main"}"#,
        )
        .unwrap();
        let direct = parse_module_record(&mut direct).unwrap();
        assert_eq!(direct.namespace.value.as_ref(), "city/main");
        assert_eq!(direct.document_key.value.as_ref(), "source/main");
        assert_eq!(direct.imports.len(), 1);
        assert!(matches!(direct.provenance, ParsedProvenance::Direct { .. }));

        let mut generated = JsonCursor::new(
            br#"{"namespace":"city/main","documentKey":"source/main","imports":[],"provenance":{"kind":"generated","generatorBuildId":"generator.v1","parametersAndInputsDigest":"0000000000000000000000000000000000000000000000000000000000000000","frontendOptionsDigest":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","randomSeed":"18446744073709551615","description":"generated source"}}"#,
        )
        .unwrap();
        let generated = parse_module_record(&mut generated).unwrap();
        assert!(matches!(
            generated.provenance,
            ParsedProvenance::Generated {
                random_seed: Some(u64::MAX),
                ..
            }
        ));
    }

    #[test]
    fn closed_shapes_reject_duplicate_unknown_missing_and_union_leakage() {
        let cases = [
            (
                br#"{"namespace":"a","namespace":"b","documentKey":"d","imports":[],"provenance":{"kind":"direct","description":"x"}}"#.as_slice(),
                SchemaErrorKind::DuplicateField("namespace"),
            ),
            (
                br#"{"namespace":"a","documentKey":"d","imports":[],"extra":0,"provenance":{"kind":"direct","description":"x"}}"#.as_slice(),
                SchemaErrorKind::UnknownField("extra".into()),
            ),
            (
                br#"{"namespace":"a","imports":[],"provenance":{"kind":"direct","description":"x"}}"#.as_slice(),
                SchemaErrorKind::MissingField("documentKey"),
            ),
            (
                br#"{"namespace":"a","documentKey":"d","imports":[],"provenance":{"kind":"direct","generatorBuildId":"g","description":"x"}}"#.as_slice(),
                SchemaErrorKind::FieldNotAllowedForProvenance {
                    field: "generatorBuildId",
                    kind: "direct",
                },
            ),
        ];
        for (source, expected) in cases {
            let mut cursor = JsonCursor::new(source).unwrap();
            assert_eq!(parse_module_record(&mut cursor).unwrap_err().kind, expected);
        }
    }

    #[test]
    fn units_require_exact_constants_and_all_fields() {
        let mut valid = JsonCursor::new(
            br#"{"time":"second","speed":"meter-per-second","angle":"radian","distance":"meter"}"#,
        )
        .unwrap();
        parse_units_record(&mut valid).unwrap();

        let mut wrong = JsonCursor::new(
            br#"{"distance":"kilometer","angle":"radian","speed":"meter-per-second","time":"second"}"#,
        )
        .unwrap();
        assert_eq!(
            parse_units_record(&mut wrong).unwrap_err().kind,
            SchemaErrorKind::UnexpectedConstant {
                field: "distance",
                expected: "meter",
            }
        );
    }

    #[test]
    fn complete_document_closes_top_level_and_rejects_trailing_bytes() {
        let source = super::MINIMAL_DOCUMENT;
        let parsed = parse_geometry_document(source).unwrap();
        assert_eq!(parsed.frames.len(), 1);
        assert_eq!(parsed.roads.len(), 1);
        assert!(parsed.junctions.is_empty());
        assert!(parsed.overlays.access_rules.is_empty());

        let mut trailing = source.to_vec();
        trailing.extend_from_slice(b" false");
        assert!(matches!(
            parse_geometry_document(&trailing).unwrap_err().kind,
            SchemaErrorKind::Json(_)
        ));
    }
}
