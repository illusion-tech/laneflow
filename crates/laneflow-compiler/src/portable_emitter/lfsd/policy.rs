//! 从最终 LFCA 行核对策略引用；不消费 LIR，也不解析运行时的有效规则。

use super::base::{
    ArtifactIndex, checked_ordinal_vector_with, checked_record_vector_with, checked_u32_with,
};
use super::policy_change::{Scratch, reserved};
use super::*;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct LocalKey<'a> {
    table: u16,
    owner: u32,
    key: &'a str,
}

/// 核对所有 policy，包括未被世界选择的策略及未被规则引用的局部成员。
///
/// 实体 lookup 复用已闭合的 Identity/实体索引。新增 scratch 只保存 evidence/gap
/// 的借用键，按两个表的完整逻辑行数计量；不会复制字符串或按引用数重新扫描表。
pub(super) fn validate_policy_references(
    index: &ArtifactIndex<'_>,
    scratch: &mut Scratch,
    mismatch: PortableEmissionError,
) -> Result<(), PortableEmissionError> {
    let section = index.view.section(3).ok_or(mismatch)?;
    let evidence = section.table(1).ok_or(mismatch)?;
    let gaps = section.table(2).ok_or(mismatch)?;
    let count = u64::from(evidence.row_count())
        .checked_add(u64::from(gaps.row_count()))
        .ok_or(PortableEmissionError::ArithmeticOverflow)?;
    let mut keys = reserved::<LocalKey<'_>>(
        usize::try_from(count).map_err(|_| PortableEmissionError::ArithmeticOverflow)?,
        scratch,
    )?;
    for table in [evidence, gaps] {
        for row in table.rows() {
            let key = member_key(table.kind(), row, mismatch)?;
            index.stable_id(EntityKind::RightOfWayPolicySet, key.owner, mismatch)?;
            if keys.last().is_some_and(|previous| *previous >= key) {
                return Err(mismatch);
            }
            keys.push(key);
        }
    }

    for table_ordinal in [3, 4] {
        let table = section.table(table_ordinal).ok_or(mismatch)?;
        for row in table.rows() {
            let owner = checked_u32_with(row, 1, mismatch)?;
            let policy = index.entity_row(EntityKind::RightOfWayPolicySet, owner, mismatch)?;
            let is_stream_rule = table.kind() == 4;
            let target_kind = if is_stream_rule {
                EntityKind::ParticipantStream
            } else {
                EntityKind::ManeuverGate
            };
            index.stable_id(target_kind, checked_u32_with(row, 3, mismatch)?, mismatch)?;
            if row.field_by_tag(4).is_some() {
                validate_reference_set(index, row, 4, EntityKind::ParticipantClass, mismatch)?;
            }
            if is_stream_rule {
                validate_reference_set(index, row, 6, EntityKind::ParticipantStream, mismatch)?;
                if row.field_by_tag(7).is_some() {
                    require_local_key(&keys, 3, owner, checked_utf8(row, 7, mismatch)?, mismatch)?;
                }
            }
            let evidence_tag = if is_stream_rule { 8 } else { 7 };
            let evidence_keys = checked_record_vector_with(row, evidence_tag, mismatch)?;
            if evidence_keys.is_empty() && policy.field_by_tag(5).is_none() {
                return Err(mismatch);
            }
            // 即使已有策略级来源，也必须检查每个显式 evidence 引用。
            for evidence in evidence_keys.rows() {
                require_local_key(
                    &keys,
                    2,
                    owner,
                    checked_utf8(evidence, 1, mismatch)?,
                    mismatch,
                )?;
            }
        }
    }
    scratch.release((keys.capacity() * size_of::<LocalKey<'_>>()) as u64);
    Ok(())
}

fn member_key(
    table: u16,
    row: RegistryCheckedRowView<'_>,
    mismatch: PortableEmissionError,
) -> Result<LocalKey<'_>, PortableEmissionError> {
    Ok(LocalKey {
        table,
        owner: checked_u32_with(row, 1, mismatch)?,
        key: checked_utf8(row, 2, mismatch)?,
    })
}

fn require_local_key(
    keys: &[LocalKey<'_>],
    table: u16,
    owner: u32,
    key: &str,
    mismatch: PortableEmissionError,
) -> Result<(), PortableEmissionError> {
    keys.binary_search(&LocalKey { table, owner, key })
        .map(|_| ())
        .map_err(|_| mismatch)
}

fn validate_reference_set(
    index: &ArtifactIndex<'_>,
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    entity_kind: EntityKind,
    mismatch: PortableEmissionError,
) -> Result<(), PortableEmissionError> {
    let references = checked_ordinal_vector_with(row, tag, mismatch)?;
    let mut previous = None;
    for position in 0..references.len() {
        let ordinal = references.get(position).ok_or(mismatch)?;
        let stable_id = index.stable_id(entity_kind, ordinal, mismatch)?;
        if previous.is_some_and(|value| value >= stable_id) {
            return Err(mismatch);
        }
        previous = Some(stable_id);
    }
    Ok(())
}

fn checked_utf8<'a>(
    row: RegistryCheckedRowView<'a>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<&'a str, PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(mismatch)?.value()? {
        RegistryCheckedFieldValue::Utf8(value) => Ok(value),
        _ => Err(mismatch),
    }
}

#[cfg(test)]
mod tests;
