use std::collections::BTreeSet;

use crate::{GeometryAccuracyProfile, GeometryDirectionProfile};

use super::geometry::{
    CurveSegment, NumericFreezeError, OffsetInterval, Point3, StationInterval, direction_accepts,
    half_angle_cosine_squared, position_distance_squared_accepts, position_target,
    position_target_squared, regularity_visit_budget_allows, subdivision_depth_can_split,
};

const FIXTURE: &str = include_str!("../../tests/road-editing-geometry-known-vectors.txt");

#[derive(Debug)]
struct FixtureRecord<'a> {
    columns: Box<[&'a str]>,
}

impl<'a> FixtureRecord<'a> {
    fn kind(&self) -> &'a str {
        self.columns[0]
    }

    fn name(&self) -> &'a str {
        self.columns[1]
    }

    fn field(&self, index: usize) -> &'a str {
        self.columns[index]
    }
}

fn parse_fixture(input: &str) -> Result<Box<[FixtureRecord<'_>]>, String> {
    let mut records = Vec::new();
    let mut identities = BTreeSet::new();
    for (line_index, raw_line) in input.lines().enumerate() {
        let line_number = line_index + 1;
        if raw_line.is_empty() || raw_line.starts_with('#') {
            continue;
        }
        if raw_line.trim() != raw_line {
            return Err(format!("line {line_number} has surrounding whitespace"));
        }
        let record = FixtureRecord {
            columns: raw_line.split('|').collect::<Vec<_>>().into_boxed_slice(),
        };
        if record.columns.iter().any(|column| column.is_empty()) {
            return Err(format!("line {line_number} has an empty field"));
        }
        validate_record(&record).map_err(|error| format!("line {line_number}: {error}"))?;
        if !identities.insert((record.kind(), record.name())) {
            return Err(format!(
                "line {line_number} duplicates {}|{}",
                record.kind(),
                record.name()
            ));
        }
        records.push(record);
    }
    Ok(records.into_boxed_slice())
}

fn validate_record(record: &FixtureRecord<'_>) -> Result<(), String> {
    if !record
        .name()
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(format!("invalid record name {}", record.name()));
    }
    match record.kind() {
        "curve" => {
            let expected_len = match record.field(2) {
                "line" => 16,
                "cubic" => 22,
                other => return Err(format!("unknown curve kind {other}")),
            };
            require_len(record, expected_len)?;
            validate_f64_fields(record, 3..expected_len)
        }
        "offset" => {
            let expected_len = match record.field(2) {
                "line" => 24,
                "cubic" => 30,
                other => return Err(format!("unknown offset curve kind {other}")),
            };
            require_len(record, expected_len)?;
            validate_f64_fields(record, 3..expected_len)
        }
        "profile-threshold" => {
            require_len(record, 12)?;
            parse_accuracy(record.field(2))?;
            parse_direction(record.field(3))?;
            validate_f64_fields(record, 4..12)
        }
        "regularity" => {
            require_len(record, 17)?;
            if record.field(2) != "cubic" {
                return Err("regularity vectors only admit cubic segments".to_owned());
            }
            validate_f64_fields(record, 3..15)?;
            match record.field(15) {
                "ok" => {
                    parse_u32(record.field(16))?;
                    Ok(())
                }
                "error" => {
                    parse_numeric_error(record.field(16))?;
                    Ok(())
                }
                other => Err(format!("unknown regularity outcome {other}")),
            }
        }
        "regularity-budget" => {
            require_len(record, 4)?;
            parse_u32(record.field(2))?;
            parse_bool(record.field(3))?;
            Ok(())
        }
        "subdivision-depth" => {
            require_len(record, 4)?;
            parse_u8(record.field(2))?;
            parse_bool(record.field(3))?;
            Ok(())
        }
        "weld" => {
            require_len(record, 9)?;
            validate_f64_fields(record, 2..8)?;
            if parse_numeric_error(record.field(8))? != NumericFreezeError::SourceJoinGapExceeded {
                return Err("weld rejection must be SourceJoinGapExceeded".to_owned());
            }
            Ok(())
        }
        other => Err(format!("unknown record kind {other}")),
    }
}

fn require_len(record: &FixtureRecord<'_>, expected: usize) -> Result<(), String> {
    if record.columns.len() == expected {
        Ok(())
    } else {
        Err(format!(
            "{}|{} has {} fields; expected {expected}",
            record.kind(),
            record.name(),
            record.columns.len()
        ))
    }
}

fn validate_f64_fields(
    record: &FixtureRecord<'_>,
    range: std::ops::Range<usize>,
) -> Result<(), String> {
    for index in range {
        parse_f64_bits(record.field(index))?;
    }
    Ok(())
}

fn parse_u64_bits(value: &str) -> Result<u64, String> {
    if value.len() != 16
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("invalid binary64 bits {value}"));
    }
    u64::from_str_radix(value, 16)
        .map_err(|error| format!("invalid binary64 bits {value}: {error}"))
}

fn parse_f64_bits(value: &str) -> Result<f64, String> {
    let value = f64::from_bits(parse_u64_bits(value)?);
    if value.is_finite() {
        Ok(value)
    } else {
        Err("known-vector binary64 value must be finite".to_owned())
    }
}

fn parse_u32(value: &str) -> Result<u32, String> {
    let parsed = value
        .parse::<u32>()
        .map_err(|error| format!("invalid u32 {value}: {error}"))?;
    if parsed.to_string() == value {
        Ok(parsed)
    } else {
        Err(format!("u32 is not canonical decimal: {value}"))
    }
}

fn parse_u8(value: &str) -> Result<u8, String> {
    let parsed = value
        .parse::<u8>()
        .map_err(|error| format!("invalid u8 {value}: {error}"))?;
    if parsed.to_string() == value {
        Ok(parsed)
    } else {
        Err(format!("u8 is not canonical decimal: {value}"))
    }
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!("invalid bool {other}")),
    }
}

fn parse_accuracy(value: &str) -> Result<GeometryAccuracyProfile, String> {
    match value {
        "Fine2Cm" => Ok(GeometryAccuracyProfile::Fine2Cm),
        "Balanced5Cm" => Ok(GeometryAccuracyProfile::Balanced5Cm),
        "Compact10Cm" => Ok(GeometryAccuracyProfile::Compact10Cm),
        other => Err(format!("unknown accuracy profile {other}")),
    }
}

fn parse_direction(value: &str) -> Result<GeometryDirectionProfile, String> {
    match value {
        "Smooth1Deg" => Ok(GeometryDirectionProfile::Smooth1Deg),
        "Balanced2Deg" => Ok(GeometryDirectionProfile::Balanced2Deg),
        "Compact5Deg" => Ok(GeometryDirectionProfile::Compact5Deg),
        other => Err(format!("unknown direction profile {other}")),
    }
}

fn parse_numeric_error(value: &str) -> Result<NumericFreezeError, String> {
    match value {
        "HorizontalDerivativeNotProvenNonZero" => {
            Ok(NumericFreezeError::HorizontalDerivativeNotProvenNonZero)
        }
        "SourceJoinGapExceeded" => Ok(NumericFreezeError::SourceJoinGapExceeded),
        other => Err(format!("unknown numeric error {other}")),
    }
}

fn fixture_records() -> Box<[FixtureRecord<'static>]> {
    parse_fixture(FIXTURE).expect("checked-in geometry known-vector fixture must be valid")
}

fn point_at(record: &FixtureRecord<'_>, index: usize) -> Point3 {
    Point3::try_new(
        parse_f64_bits(record.field(index)).unwrap(),
        parse_f64_bits(record.field(index + 1)).unwrap(),
        parse_f64_bits(record.field(index + 2)).unwrap(),
    )
    .unwrap()
}

fn assert_point_bits(actual: Point3, record: &FixtureRecord<'_>, index: usize, label: &str) {
    for (component, expected) in [actual.x, actual.y, actual.z]
        .into_iter()
        .zip(index..index + 3)
    {
        assert_eq!(
            component.to_bits(),
            parse_u64_bits(record.field(expected)).unwrap(),
            "{}|{} {label}",
            record.kind(),
            record.name()
        );
    }
}

fn curve_segment(record: &FixtureRecord<'_>, point_start: usize) -> (CurveSegment, usize) {
    match record.field(2) {
        "line" => (
            CurveSegment::Line {
                start: point_at(record, point_start),
                end: point_at(record, point_start + 3),
            },
            point_start + 6,
        ),
        "cubic" => (
            CurveSegment::CubicBezier {
                start: point_at(record, point_start),
                control_1: point_at(record, point_start + 3),
                control_2: point_at(record, point_start + 6),
                end: point_at(record, point_start + 9),
            },
            point_start + 12,
        ),
        other => panic!("validated fixture contains unknown curve kind {other}"),
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CanonicalWeldVector {
    pub(super) accepted_second_end_z: f64,
    pub(super) rejected_second_end_z: f64,
    pub(super) offset_meters: f64,
    pub(super) expected_promoted_point_bits: [u64; 3],
    pub(super) rejected_error: NumericFreezeError,
}

pub(super) fn canonical_weld_vector() -> CanonicalWeldVector {
    let records = fixture_records();
    let record = records
        .iter()
        .find(|record| record.kind() == "weld" && record.name() == "source-offset-canonical")
        .expect("canonical weld vector must be present");
    CanonicalWeldVector {
        accepted_second_end_z: parse_f64_bits(record.field(2)).unwrap(),
        rejected_second_end_z: parse_f64_bits(record.field(3)).unwrap(),
        offset_meters: parse_f64_bits(record.field(4)).unwrap(),
        expected_promoted_point_bits: [
            parse_u64_bits(record.field(5)).unwrap(),
            parse_u64_bits(record.field(6)).unwrap(),
            parse_u64_bits(record.field(7)).unwrap(),
        ],
        rejected_error: parse_numeric_error(record.field(8)).unwrap(),
    }
}

#[test]
fn known_vector_fixture_grammar_is_closed_and_strict() {
    assert_eq!(fixture_records().len(), 21);
    assert!(parse_fixture("unknown|record").is_err());
    assert!(parse_fixture("curve|too-short|line").is_err());
    assert!(parse_fixture("regularity-budget|bad-bool|4094|yes").is_err());
    assert!(parse_fixture(
        "weld|uppercase-bits|3F50624dd2f1a9fc|3fb999999999999a|3ff0000000000000|4024000000000000|0000000000000000|bff0000000000000|SourceJoinGapExceeded"
    )
    .is_err());
    assert!(
        parse_fixture(
            "regularity-budget|duplicate|4094|true\nregularity-budget|duplicate|4095|false"
        )
        .is_err()
    );
}

#[test]
fn scalar_dual_curve_vectors_match_independent_binary64_bits() {
    let records = fixture_records();
    let mut count = 0;
    for record in records.iter().filter(|record| record.kind() == "curve") {
        let parameter = parse_f64_bits(record.field(3)).unwrap();
        let (segment, expected_start) = curve_segment(record, 4);
        let sample = segment.evaluate(parameter).unwrap();
        assert_point_bits(sample.point, record, expected_start, "point");
        assert_point_bits(sample.first, record, expected_start + 3, "first");
        count += 1;
    }
    assert_eq!(count, 2);
}

#[test]
fn scalar_dual_offset_vectors_match_independent_binary64_bits() {
    let records = fixture_records();
    let mut count = 0;
    for record in records.iter().filter(|record| record.kind() == "offset") {
        let parameter = parse_f64_bits(record.field(3)).unwrap();
        let (segment, station_start) = curve_segment(record, 4);
        let station = StationInterval {
            parameter_start: parse_f64_bits(record.field(station_start)).unwrap(),
            parameter_end: parse_f64_bits(record.field(station_start + 1)).unwrap(),
            cumulative_start_meters: parse_f64_bits(record.field(station_start + 2)).unwrap(),
            cumulative_end_meters: parse_f64_bits(record.field(station_start + 3)).unwrap(),
        };
        let offset_start = station_start + 4;
        let offset = OffsetInterval {
            station_start_meters: parse_f64_bits(record.field(offset_start)).unwrap(),
            station_end_meters: parse_f64_bits(record.field(offset_start + 1)).unwrap(),
            offset_start_meters: parse_f64_bits(record.field(offset_start + 2)).unwrap(),
            offset_end_meters: parse_f64_bits(record.field(offset_start + 3)).unwrap(),
        };
        let expected_start = offset_start + 4;
        let sample = segment.evaluate_offset(parameter, station, offset).unwrap();
        assert_point_bits(sample.point, record, expected_start, "offset point");
        assert_point_bits(sample.first, record, expected_start + 3, "offset first");
        count += 1;
    }
    assert_eq!(count, 2);
}

#[test]
fn all_nine_profile_thresholds_freeze_minus_equal_plus_one_ulp() {
    let records = fixture_records();
    let mut combinations = BTreeSet::new();
    for record in records
        .iter()
        .filter(|record| record.kind() == "profile-threshold")
    {
        let accuracy = parse_accuracy(record.field(2)).unwrap();
        let direction = parse_direction(record.field(3)).unwrap();
        assert_eq!(
            position_target(accuracy).to_bits(),
            parse_u64_bits(record.field(4)).unwrap(),
            "{} position target",
            record.name()
        );
        let target_squared = position_target_squared(accuracy).unwrap();
        assert_eq!(
            target_squared.to_bits(),
            parse_u64_bits(record.field(6)).unwrap(),
            "{} squared position target",
            record.name()
        );
        for (index, expected) in [(5, true), (6, true), (7, false)] {
            assert_eq!(
                position_distance_squared_accepts(
                    parse_f64_bits(record.field(index)).unwrap(),
                    target_squared,
                ),
                Ok(expected),
                "{} position threshold field {index}",
                record.name()
            );
        }
        let cosine_squared = half_angle_cosine_squared(direction);
        assert_eq!(
            cosine_squared.to_bits(),
            parse_u64_bits(record.field(8)).unwrap(),
            "{} direction target",
            record.name()
        );
        let left = Point3::try_new(1.0, 0.0, 0.0).unwrap();
        for (index, expected) in [(9, true), (10, true), (11, false)] {
            let right =
                Point3::try_new(1.0, 0.0, parse_f64_bits(record.field(index)).unwrap()).unwrap();
            assert_eq!(
                direction_accepts(left, right, cosine_squared),
                Ok(expected),
                "{} direction threshold field {index}",
                record.name()
            );
        }
        combinations.insert((record.field(2), record.field(3)));
    }
    assert_eq!(combinations.len(), 9);
}

#[test]
fn regularity_vectors_cover_split_near_cusp_cusp_and_visit_gate() {
    let records = fixture_records();
    let mut regularity_count = 0;
    for record in records
        .iter()
        .filter(|record| record.kind() == "regularity")
    {
        let (segment, outcome_start) = curve_segment(record, 3);
        let expected = match record.field(outcome_start) {
            "ok" => Ok(parse_u32(record.field(outcome_start + 1)).unwrap()),
            "error" => Err(parse_numeric_error(record.field(outcome_start + 1)).unwrap()),
            other => panic!("validated fixture contains unknown regularity outcome {other}"),
        };
        assert_eq!(
            segment.prove_horizontal_regularity(),
            expected,
            "{}",
            record.name()
        );
        regularity_count += 1;
    }
    assert_eq!(regularity_count, 3);

    let mut budget_count = 0;
    for record in records
        .iter()
        .filter(|record| record.kind() == "regularity-budget")
    {
        assert_eq!(
            regularity_visit_budget_allows(parse_u32(record.field(2)).unwrap()),
            parse_bool(record.field(3)).unwrap(),
            "{}",
            record.name()
        );
        budget_count += 1;
    }
    assert_eq!(budget_count, 2);

    let mut depth_count = 0;
    for record in records
        .iter()
        .filter(|record| record.kind() == "subdivision-depth")
    {
        assert_eq!(
            subdivision_depth_can_split(parse_u8(record.field(2)).unwrap()),
            parse_bool(record.field(3)).unwrap(),
            "{}",
            record.name()
        );
        depth_count += 1;
    }
    assert_eq!(depth_count, 2);
}
