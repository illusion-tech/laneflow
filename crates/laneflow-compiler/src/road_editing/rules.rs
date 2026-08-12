use std::collections::BTreeSet;

use crate::declaration::{FacilityKindCategory, facility_kind_category};
use crate::source::external_token_violation;
use crate::{Diagnostic, DiagnosticBundle, RoadEditingInputViolation, SourceTextViolation};

pub(super) fn input_error(field: &str, violation: RoadEditingInputViolation) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::invalid_road_editing_input(field, violation))
}

pub(super) fn validate_token(value: &str, field: &str) -> Result<(), DiagnosticBundle> {
    if let Some(violation) = external_token_violation(value, u64::MAX) {
        return Err(input_error(
            field,
            RoadEditingInputViolation::InvalidText(violation),
        ));
    }
    if let Some(byte_index) = value.as_bytes().windows(2).position(|pair| pair == b"::") {
        return Err(input_error(
            field,
            RoadEditingInputViolation::InvalidText(SourceTextViolation::ReservedDelimiter {
                byte_index: u64::try_from(byte_index).unwrap_or(u64::MAX),
            }),
        ));
    }
    Ok(())
}

pub(super) fn validate_facility_kind(
    value: &str,
    expected: FacilityKindCategory,
    field: &str,
) -> Result<(), DiagnosticBundle> {
    if facility_kind_category(value) != Some(expected) {
        return Err(input_error(
            field,
            RoadEditingInputViolation::InvalidCombination,
        ));
    }
    Ok(())
}

pub(super) fn validate_visible_ascii(value: &str, field: &str) -> Result<(), DiagnosticBundle> {
    if value.is_empty() {
        return Err(input_error(
            field,
            RoadEditingInputViolation::InvalidText(SourceTextViolation::Empty),
        ));
    }
    if let Some((byte_index, byte)) = value
        .bytes()
        .enumerate()
        .find(|(_, byte)| !byte.is_ascii_graphic() && *byte != b' ')
    {
        let violation = if byte.is_ascii() {
            SourceTextViolation::ControlByte {
                byte_index: u64::try_from(byte_index).unwrap_or(u64::MAX),
                byte,
            }
        } else {
            SourceTextViolation::NonAscii {
                byte_index: u64::try_from(byte_index).unwrap_or(u64::MAX),
            }
        };
        return Err(input_error(
            field,
            RoadEditingInputViolation::InvalidText(violation),
        ));
    }
    Ok(())
}

pub(super) fn validate_non_empty_text(value: &str, field: &str) -> Result<(), DiagnosticBundle> {
    if value.is_empty() {
        return Err(input_error(
            field,
            RoadEditingInputViolation::InvalidText(SourceTextViolation::Empty),
        ));
    }
    Ok(())
}

pub(super) fn validate_finite(value: f64, field: &str) -> Result<f64, DiagnosticBundle> {
    if !value.is_finite() {
        return Err(input_error(
            field,
            RoadEditingInputViolation::NonFinite {
                value_bits: value.to_bits(),
            },
        ));
    }
    Ok(if value == 0.0 { 0.0 } else { value })
}

pub(super) fn validate_positive(value: f64, field: &str) -> Result<f64, DiagnosticBundle> {
    let value = validate_finite(value, field)?;
    if value <= 0.0 {
        return Err(input_error(
            field,
            RoadEditingInputViolation::NotGreaterThanZero {
                value_bits: value.to_bits(),
            },
        ));
    }
    Ok(value)
}

pub(super) fn validate_non_negative(value: f64, field: &str) -> Result<f64, DiagnosticBundle> {
    let value = validate_finite(value, field)?;
    if value < 0.0 {
        return Err(input_error(
            field,
            RoadEditingInputViolation::LessThanZero {
                value_bits: value.to_bits(),
            },
        ));
    }
    Ok(value)
}

pub(super) fn require_non_empty<T>(values: &[T], field: &str) -> Result<(), DiagnosticBundle> {
    if values.is_empty() {
        return Err(input_error(
            field,
            RoadEditingInputViolation::EmptyCollection,
        ));
    }
    Ok(())
}

pub(super) fn require_unique<T: Ord>(values: &[T], field: &str) -> Result<(), DiagnosticBundle> {
    let mut seen = BTreeSet::new();
    if values.iter().any(|value| !seen.insert(value)) {
        return Err(input_error(
            field,
            RoadEditingInputViolation::DuplicateValue,
        ));
    }
    Ok(())
}
