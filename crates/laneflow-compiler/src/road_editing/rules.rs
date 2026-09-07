use std::collections::BTreeSet;

use laneflow_static_contract::{
    MAX_ACCEL_METERS_PER_SECOND_SQUARED, MAX_TIME_HEADWAY_SECONDS,
    MIN_ACCEL_METERS_PER_SECOND_SQUARED, heading_f32_from_si, heading_f32_in_legal_closure,
    millimetres_from_si, millimetres_i32_from_si,
};

use crate::declaration::{FacilityKindCategory, facility_kind_category};
use crate::source::external_token_violation;
use crate::{Diagnostic, DiagnosticBundle, RoadEditingInputViolation, SourceTextViolation};

/// 单个引用组件（编制命名空间或键路径段）的最大字节数。
pub(super) const MAX_COMPONENT_BYTES: u64 = 53;
/// 完整 wire 引用文本的最大字节数。
pub(super) const MAX_REFERENCE_BYTES: u64 = 270;

/// 已通过校验的借用型 wire 引用：可选编制命名空间前缀加 owner 键路径。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ValidatedReference<'a> {
    namespace: Option<&'a str>,
    key_path: &'a str,
}

impl<'a> ValidatedReference<'a> {
    /// 返回可选的编制命名空间前缀。
    pub(super) const fn namespace(self) -> Option<&'a str> {
        self.namespace
    }

    /// 按 `>` 分隔迭代 owner 键路径的各组件。
    pub(super) fn key_components(self) -> impl Iterator<Item = &'a str> {
        self.key_path.split('>')
    }
}

/// 生成指定字段的道路编辑输入违规诊断。
pub(super) fn input_error(field: &str, violation: RoadEditingInputViolation) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::invalid_road_editing_input(field, violation))
}

/// 校验标识 token 文本（含保留分隔符禁令）；违规时返回输入诊断。
pub(super) fn validate_token(value: &str, field: &str) -> Result<(), DiagnosticBundle> {
    if let Some(violation) = token_violation(value, u64::MAX, true) {
        return Err(input_error(field, violation));
    }
    Ok(())
}

/// 校验设施种类字符串属于期望的设施种类类别。
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

/// 返回 token 文本的违规项，可按需禁止保留分隔符 `::`；无违规时返回 `None`。
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

/// 校验 wire 引用的字节上限、可选命名空间限定与精确键深，返回借用型已校验引用。
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

/// 校验文本为非空可见 ASCII；违规时返回输入诊断。
pub(super) fn validate_visible_ascii(value: &str, field: &str) -> Result<(), DiagnosticBundle> {
    if let Some(violation) = visible_ascii_violation(value, u64::MAX) {
        return Err(input_error(field, violation));
    }
    Ok(())
}

/// 返回可见 ASCII 文本的违规项（空、超长、控制字节或非 ASCII）；无违规时返回 `None`。
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

/// 值非有限时返回相应违规项，有限时返回 `None`。
pub(super) fn finite_violation(value: f64) -> Option<RoadEditingInputViolation> {
    (!value.is_finite()).then_some(RoadEditingInputViolation::NonFinite {
        value_bits: value.to_bits(),
    })
}

/// 值非有限或不大于零时返回相应违规项，正值返回 `None`。
pub(super) fn positive_violation(value: f64) -> Option<RoadEditingInputViolation> {
    finite_violation(value).or_else(|| {
        (value <= 0.0).then_some(RoadEditingInputViolation::NotGreaterThanZero {
            value_bits: value.to_bits(),
        })
    })
}

/// 值非有限或为负时返回相应违规项，非负值返回 `None`。
pub(super) fn non_negative_violation(value: f64) -> Option<RoadEditingInputViolation> {
    finite_violation(value).or_else(|| {
        (value < 0.0).then_some(RoadEditingInputViolation::LessThanZero {
            value_bits: value.to_bits(),
        })
    })
}

/// 校验文本非空；为空时返回输入诊断。
pub(super) fn validate_non_empty_text(value: &str, field: &str) -> Result<(), DiagnosticBundle> {
    if value.is_empty() {
        return Err(input_error(
            field,
            RoadEditingInputViolation::InvalidText(SourceTextViolation::Empty),
        ));
    }
    Ok(())
}

/// 值非有限或落在闭区间外时返回相应违规项，区间内值返回 `None`。
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

/// 把 SI 米量化为整数毫米并校验闭区间；返回相应违规项或 `None`。
pub(super) fn millimetre_range_violation(
    value: f64,
    min_mm: u32,
    max_mm: u32,
) -> Option<RoadEditingInputViolation> {
    match millimetres_from_si(value) {
        Some(mm) if (min_mm..=max_mm).contains(&mm) => None,
        None if !value.is_finite() => finite_violation(value),
        Some(_) | None => Some(RoadEditingInputViolation::InvalidCombination),
    }
}

/// 把 SI 米量化为 `i32` 毫米并按绝对值校验闭区间；返回相应违规项或 `None`。
pub(super) fn millimetre_i32_abs_range_violation(
    value: f64,
    min_abs_mm: u32,
    max_abs_mm: u32,
) -> Option<RoadEditingInputViolation> {
    match millimetres_i32_from_si(value) {
        Some(mm) => {
            let abs = mm.unsigned_abs();
            (abs < min_abs_mm || abs > max_abs_mm)
                .then_some(RoadEditingInputViolation::InvalidCombination)
        }
        None if !value.is_finite() => finite_violation(value),
        None => Some(RoadEditingInputViolation::InvalidCombination),
    }
}

/// 把 SI 弧度量化为 `f32` 航向并校验其合法闭包；返回相应违规项或 `None`。
pub(super) fn heading_violation(value: f64) -> Option<RoadEditingInputViolation> {
    match heading_f32_from_si(value) {
        Some(heading) if heading_f32_in_legal_closure(heading) => None,
        None if !value.is_finite() => finite_violation(value),
        Some(_) | None => Some(RoadEditingInputViolation::InvalidCombination),
    }
}

/// 校验时间车头时距量化为 `f32` 后为正且不超过契约上限；返回相应违规项或 `None`。
pub(super) fn time_headway_violation(value: f64) -> Option<RoadEditingInputViolation> {
    finite_violation(value).or_else(|| {
        let quantized = value as f32;
        if !quantized.is_finite() || quantized <= 0.0 {
            Some(RoadEditingInputViolation::NotGreaterThanZero {
                value_bits: value.to_bits(),
            })
        } else if quantized > MAX_TIME_HEADWAY_SECONDS {
            Some(RoadEditingInputViolation::OutsideInclusiveRange {
                value_bits: value.to_bits(),
                minimum_bits: 0.0_f64.to_bits(),
                maximum_bits: f64::from(MAX_TIME_HEADWAY_SECONDS).to_bits(),
            })
        } else {
            None
        }
    })
}

/// 校验加速度值量化为 `f32` 后落在契约闭区间内；返回相应违规项或 `None`。
pub(super) fn accel_violation(value: f64) -> Option<RoadEditingInputViolation> {
    finite_violation(value).or_else(|| {
        let quantized = value as f32;
        if !quantized.is_finite()
            || quantized < MIN_ACCEL_METERS_PER_SECOND_SQUARED
            || quantized > MAX_ACCEL_METERS_PER_SECOND_SQUARED
        {
            Some(RoadEditingInputViolation::OutsideInclusiveRange {
                value_bits: value.to_bits(),
                minimum_bits: f64::from(MIN_ACCEL_METERS_PER_SECOND_SQUARED).to_bits(),
                maximum_bits: f64::from(MAX_ACCEL_METERS_PER_SECOND_SQUARED).to_bits(),
            })
        } else {
            None
        }
    })
}

/// 校验 `f64` 有限并把负零归一化为正零；违规时返回输入诊断。
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

/// 校验 `f64` 有限且大于零；违规时返回输入诊断。
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

/// 校验 `f64` 有限且非负；违规时返回输入诊断。
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

/// 校验 `f64` 有限且落在闭区间内；违规时返回输入诊断。
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

/// 要求集合非空；为空时返回输入诊断。
pub(super) fn require_non_empty<T>(values: &[T], field: &str) -> Result<(), DiagnosticBundle> {
    if values.is_empty() {
        return Err(input_error(
            field,
            RoadEditingInputViolation::EmptyCollection,
        ));
    }
    Ok(())
}

/// 要求集合元素唯一；存在重复时返回输入诊断。
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
