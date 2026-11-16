//! LFSD Bytes 内的完整策略成员仍按 RowV1 登记检查并计入同一 chunk 预算。

use crate::{
    FormatError, FormatLimits, FormatStructure, LimitDimension,
    object::RegistryCheckedRowView,
    table::{PreflightBudget, charge_stable_vector, preflight_embedded_row},
    wire::{checked_slice, read_u16, read_u32, read_u64},
};
use laneflow_static_contract::{EntityKind, policy_local_value_schema};

/// 按 RowV1 线格式逐字段遍历一行，向回调提供字段标签与值字节。
pub(crate) fn visit_fields<'a>(
    row: &'a [u8],
    mut visit: impl FnMut(u16, &'a [u8]) -> Result<(), FormatError>,
) -> Result<(), FormatError> {
    let count = read_u32(row, 8, FormatStructure::Row)?;
    let mut cursor = 16_u64;
    for _ in 0..count {
        let tag = read_u16(row, cursor, FormatStructure::Field)?;
        let length = read_u64(row, cursor + 4, FormatStructure::Field)?;
        let value = checked_slice(row, cursor + 12, length, FormatStructure::FieldValue)?;
        visit(tag, value)?;
        cursor = cursor
            .checked_add(12)
            .and_then(|v| v.checked_add(length))
            .ok_or(FormatError::ArithmeticOverflow {
                structure: FormatStructure::Field,
            })?;
    }
    Ok(())
}

/// 预检 LFSD 策略变更行的 Bytes 成员值，并把完整策略成员计入同一 chunk 预算。
pub(crate) fn preflight_change_values(
    row: &[u8],
    limits: FormatLimits,
    budget: &mut PreflightBudget,
) -> Result<(), FormatError> {
    let mut kind = None;
    visit_fields(row, |tag, value| {
        if tag == 3 {
            kind = value.first().copied();
        }
        if matches!(tag, 5 | 6) {
            preflight_member_value(kind.ok_or(mismatch())?, value, limits, budget)?;
        }
        Ok(())
    })
}

/// 按成员 kind 对应的策略局部 RowV1 schema 预检单个成员值，并核对稳定引用向量。
pub(crate) fn preflight_member_value(
    kind: u8,
    bytes: &[u8],
    limits: FormatLimits,
    budget: &mut PreflightBudget,
) -> Result<(), FormatError> {
    let schema = policy_local_value_schema(kind).ok_or(FormatError::UnknownKind {
        structure: FormatStructure::FieldValue,
        code: u64::from(kind),
    })?;
    preflight_embedded_row(bytes, schema, limits, budget)?;
    if kind < 2 {
        return Ok(());
    }
    visit_fields(bytes, |tag, value| {
        match tag {
            3 => stable_ref(
                value,
                if kind == 2 {
                    EntityKind::ParticipantStream
                } else {
                    EntityKind::ManeuverGate
                },
            )?,
            4 => stable_vector(value, EntityKind::ParticipantClass, limits, budget)?,
            6 if kind == 2 => stable_vector(value, EntityKind::ParticipantStream, limits, budget)?,
            _ => {}
        }
        Ok(())
    })
}

/// 对已受检 LFSD 策略成员 Bytes 值给出最小受检遍历入口。
///
/// 输入 `kind` 是 LFSD 变更行 tag 3 登记的成员种类，`bytes` 是 tag 5/6 字段的完整
/// Bytes 载荷。本函数复用完整对象预检的同一检查路径（`policy_local_value_schema`
/// 的 RowV1 登记加稳定引用向量核对）；未通过预检的任意字节不会得到受检视图，
/// 成功返回的 [`RegistryCheckedRowView`] 与该成员的登记 schema 保持对应。
///
/// 本入口不判断该 Bytes 是否确实来自 LFSD 变更行的成员字段；调用方自行保证
/// 只对登记为 RowV1 且已受检的 Bytes 调用。其他 Bytes（稳定引用、引用向量等）
/// 保持不透明，不凭内容猜测类型。
///
/// # Errors
///
/// 成员 kind 未登记或 bytes 未通过策略局部 RowV1 预检时返回相应 [`FormatError`]。
pub fn checked_policy_member_row<'a>(
    kind: u8,
    bytes: &'a [u8],
    limits: FormatLimits,
) -> Result<RegistryCheckedRowView<'a>, FormatError> {
    let mut budget = PreflightBudget::default();
    preflight_member_value(kind, bytes, limits, &mut budget)?;
    let schema = policy_local_value_schema(kind).ok_or(FormatError::UnknownKind {
        structure: FormatStructure::FieldValue,
        code: u64::from(kind),
    })?;
    RegistryCheckedRowView::from_preflighted(bytes, schema).ok_or(FormatError::BindingMismatch {
        structure: FormatStructure::FieldValue,
    })
}

fn stable_ref(value: &[u8], kind: EntityKind) -> Result<(), FormatError> {
    if value.len() != 18 || value[..2] != kind.code().to_le_bytes() {
        return Err(mismatch());
    }
    Ok(())
}

fn stable_vector(
    value: &[u8],
    kind: EntityKind,
    limits: FormatLimits,
    budget: &mut PreflightBudget,
) -> Result<(), FormatError> {
    let count = read_u32(value, 0, FormatStructure::FieldValue)?;
    if count > limits.config().max_vector_items {
        return Err(FormatError::LimitExceeded {
            dimension: LimitDimension::VectorItems,
            actual: u64::from(count),
            limit: u64::from(limits.config().max_vector_items),
        });
    }
    let length = u64::from(count)
        .checked_mul(18)
        .and_then(|v| v.checked_add(4))
        .ok_or(FormatError::ArithmeticOverflow {
            structure: FormatStructure::FieldValue,
        })?;
    if length != value.len() as u64 {
        return Err(mismatch());
    }
    charge_stable_vector(length, limits, budget)?;
    let mut previous: Option<&[u8]> = None;
    for item in value[4..].as_chunks::<18>().0 {
        stable_ref(item, kind)?;
        if previous.is_some_and(|p| p >= &item[2..]) {
            return Err(mismatch());
        }
        previous = Some(&item[2..]);
    }
    Ok(())
}

fn mismatch() -> FormatError {
    FormatError::BindingMismatch {
        structure: FormatStructure::FieldValue,
    }
}

#[cfg(test)]
mod tests;
