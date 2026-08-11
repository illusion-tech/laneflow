use std::collections::BTreeSet;

use crate::declaration::{FacilityKindCategory, facility_kind_category};
use crate::source::external_token_violation;
use crate::{Diagnostic, DiagnosticBundle, RoadEditingInputViolation, SourceTextViolation};

pub(super) const MAX_COMPONENT_BYTES: u64 = 53;
pub(super) const MAX_REFERENCE_BYTES: u64 = 270;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ValidatedReference<'a> {
    namespace: Option<&'a str>,
    key_path: &'a str,
}

impl<'a> ValidatedReference<'a> {
    pub(super) const fn namespace(self) -> Option<&'a str> {
        self.namespace
    }

    pub(super) fn key_components(self) -> impl Iterator<Item = &'a str> {
        self.key_path.split('>')
    }
}

pub(super) fn input_error(field: &str, violation: RoadEditingInputViolation) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::invalid_road_editing_input(field, violation))
}

pub(super) fn validate_token(value: &str, field: &str) -> Result<(), DiagnosticBundle> {
    if let Some(violation) = token_violation(value, u64::MAX, true) {
        return Err(input_error(field, violation));
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

pub(super) fn token_violation(
    value: &str,
    limit: u64,
    forbid_qualification_delimiter: bool,
) -> Option<RoadEditingInputViolation> {
    if let Some(violation) = external_token_violation(value, limit) {
        return Some(RoadEditingInputViolation::InvalidText(violation));
    }
    if forbid_qualification_delimiter
        && let Some(byte_index) = value.as_bytes().windows(2).position(|pair| pair == b"::")
    {
        return Some(RoadEditingInputViolation::InvalidText(
            SourceTextViolation::ReservedDelimiter {
                byte_index: u64::try_from(byte_index).unwrap_or(u64::MAX),
            },
        ));
    }
    None
}

pub(super) fn validate_wire_reference(
    value: &str,
    expected_key_component_count: u8,
    allow_qualified: bool,
) -> Result<ValidatedReference<'_>, RoadEditingInputViolation> {
    let observed_wire_bytes = u64::try_from(value.len()).unwrap_or(u64::MAX);
    if observed_wire_bytes > MAX_REFERENCE_BYTES {
        return Err(RoadEditingInputViolation::InvalidText(
            SourceTextViolation::TooLong {
                limit: MAX_REFERENCE_BYTES,
                observed: observed_wire_bytes,
            },
        ));
    }

    let (namespace, key_path) = match value.split_once("::") {
        Some((namespace, key_path)) => {
            if !allow_qualified {
                return Err(RoadEditingInputViolation::InvalidCombination);
            }
            if key_path.contains("::") {
                let byte_index = value
                    .match_indices("::")
                    .nth(1)
                    .map_or(u64::MAX, |(index, _)| {
                        u64::try_from(index).unwrap_or(u64::MAX)
                    });
                return Err(RoadEditingInputViolation::InvalidText(
                    SourceTextViolation::ReservedDelimiter { byte_index },
                ));
            }
            if let Some(violation) = token_violation(namespace, MAX_COMPONENT_BYTES, true) {
                return Err(violation);
            }
            (Some(namespace), key_path)
        }
        None => (None, value),
    };

    let mut component_count = 0_u8;
    for component in key_path.split('>') {
        if let Some(violation) = token_violation(component, MAX_COMPONENT_BYTES, true) {
            return Err(violation);
        }
        component_count = component_count.saturating_add(1);
    }
    if component_count != expected_key_component_count {
        return Err(RoadEditingInputViolation::InvalidReferenceDepth {
            expected: expected_key_component_count,
            actual: component_count,
        });
    }
    Ok(ValidatedReference {
        namespace,
        key_path,
    })
}

pub(super) fn validate_visible_ascii(value: &str, field: &str) -> Result<(), DiagnosticBundle> {
    if let Some(violation) = visible_ascii_violation(value, u64::MAX) {
        return Err(input_error(field, violation));
    }
    Ok(())
}

pub(super) fn visible_ascii_violation(
    value: &str,
    limit: u64,
) -> Option<RoadEditingInputViolation> {
    if value.is_empty() {
        return Some(RoadEditingInputViolation::InvalidText(
            SourceTextViolation::Empty,
        ));
    }
    let observed = u64::try_from(value.len()).unwrap_or(u64::MAX);
    if observed > limit {
        return Some(RoadEditingInputViolation::InvalidText(
            SourceTextViolation::TooLong { limit, observed },
        ));
    }
    value
        .bytes()
        .enumerate()
        .find(|(_, byte)| !byte.is_ascii_graphic() && *byte != b' ')
        .map(|(byte_index, byte)| {
            RoadEditingInputViolation::InvalidText(if byte.is_ascii() {
                SourceTextViolation::ControlByte {
                    byte_index: u64::try_from(byte_index).unwrap_or(u64::MAX),
                    byte,
                }
            } else {
                SourceTextViolation::NonAscii {
                    byte_index: u64::try_from(byte_index).unwrap_or(u64::MAX),
                }
            })
        })
}

pub(super) fn finite_violation(value: f64) -> Option<RoadEditingInputViolation> {
    (!value.is_finite()).then_some(RoadEditingInputViolation::NonFinite {
        value_bits: value.to_bits(),
    })
}

pub(super) fn positive_violation(value: f64) -> Option<RoadEditingInputViolation> {
    finite_violation(value).or_else(|| {
        (value <= 0.0).then_some(RoadEditingInputViolation::NotGreaterThanZero {
            value_bits: value.to_bits(),
        })
    })
}

pub(super) fn non_negative_violation(value: f64) -> Option<RoadEditingInputViolation> {
    finite_violation(value).or_else(|| {
        (value < 0.0).then_some(RoadEditingInputViolation::LessThanZero {
            value_bits: value.to_bits(),
        })
    })
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

pub(super) fn inclusive_range_violation(
    value: f64,
    minimum: f64,
    maximum: f64,
) -> Option<RoadEditingInputViolation> {
    finite_violation(value).or_else(|| {
        (!(minimum..=maximum).contains(&value)).then_some(
            RoadEditingInputViolation::OutsideInclusiveRange {
                value_bits: value.to_bits(),
                minimum_bits: minimum.to_bits(),
                maximum_bits: maximum.to_bits(),
            },
        )
    })
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

pub(super) fn validate_inclusive_range(
    value: f64,
    minimum: f64,
    maximum: f64,
    field: &str,
) -> Result<f64, DiagnosticBundle> {
    let value = validate_finite(value, field)?;
    if let Some(violation) = inclusive_range_violation(value, minimum, maximum) {
        return Err(input_error(field, violation));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_reference_preserves_borrowed_components_and_excludes_delimiters_from_bytes() {
        let parsed = validate_wire_reference("other::junction>movement>path", 3, true)
            .expect("qualified owner reference");

        assert_eq!(parsed.namespace(), Some("other"));
        assert_eq!(
            parsed.key_components().collect::<Vec<_>>(),
            ["junction", "movement", "path"]
        );
        assert_eq!(parsed.key_components().count(), 3);
        assert_eq!(
            parsed.namespace().map_or(0, str::len)
                + parsed.key_components().map(str::len).sum::<usize>(),
            5 + 8 + 8 + 4
        );
    }

    #[test]
    fn wire_reference_rejects_depth_qualification_and_component_boundaries() {
        assert!(matches!(
            validate_wire_reference("junction>movement", 3, true),
            Err(RoadEditingInputViolation::InvalidReferenceDepth {
                expected: 3,
                actual: 2
            })
        ));
        assert!(matches!(
            validate_wire_reference("other::edge", 1, false),
            Err(RoadEditingInputViolation::InvalidCombination)
        ));
        let oversized = "x".repeat(54);
        assert!(matches!(
            validate_wire_reference(&oversized, 1, true),
            Err(RoadEditingInputViolation::InvalidText(
                SourceTextViolation::TooLong {
                    limit: 53,
                    observed: 54
                }
            ))
        ));
    }
}
