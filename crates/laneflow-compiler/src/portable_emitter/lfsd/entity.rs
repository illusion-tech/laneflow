use super::base::{ArtifactIndex, checked_record_vector_with, checked_u8_with, checked_u32_with};
use super::changes::{ChangeBounds, FieldChange, Payload, add, stage};
use super::*;
use crate::ledger::BoundedVec;

fn entity_modify_tags(entity_kind: EntityKind) -> &'static [u16] {
    match entity_kind {
        EntityKind::RoadCorridor => &[3],
        EntityKind::RoadSection => &[4],
        EntityKind::AuthoringLane
        | EntityKind::Junction
        | EntityKind::ManeuverPath
        | EntityKind::WaitingZone
        | EntityKind::SignalGroup
        | EntityKind::SignalController
        | EntityKind::SignalPhase
        | EntityKind::LaneGroup
        | EntityKind::AccessRule
        | EntityKind::RightOfWayPolicySet
        | EntityKind::ConflictZone
        | EntityKind::CanonicalFrame => &[],
        EntityKind::ParkingFacility => &[5, 6],
        EntityKind::LaneEdge => &[3, 4],
        EntityKind::ManeuverGate => &[4],
        EntityKind::StopLine => &[3],
        EntityKind::ParkingSpace => &[5, 7, 8, 9, 10, 11],
        EntityKind::FacilityBand | EntityKind::ParticipantClass => &[4],
        EntityKind::VehicleProfile => &[4, 5, 6, 7, 8, 9, 10],
        EntityKind::ParticipantStream => &[5],
        EntityKind::Movement => &[7],
    }
}

fn static_rule_modify_tags(entity_kind: EntityKind) -> &'static [u16] {
    match entity_kind {
        EntityKind::ManeuverGate => &[6],
        EntityKind::WaitingZone => &[4, 5, 6],
        EntityKind::SignalController => &[3, 4],
        EntityKind::SignalPhase => &[4, 5],
        EntityKind::AccessRule => &[5, 7, 8],
        EntityKind::RightOfWayPolicySet => &[3, 4, 5],
        EntityKind::RoadCorridor
        | EntityKind::RoadSection
        | EntityKind::AuthoringLane
        | EntityKind::LaneEdge
        | EntityKind::Junction
        | EntityKind::Movement
        | EntityKind::ManeuverPath
        | EntityKind::StopLine
        | EntityKind::SignalGroup
        | EntityKind::ParkingSpace
        | EntityKind::LaneGroup
        | EntityKind::FacilityBand
        | EntityKind::ParticipantClass
        | EntityKind::VehicleProfile
        | EntityKind::ConflictZone
        | EntityKind::CanonicalFrame
        | EntityKind::ParticipantStream => &[],
        EntityKind::ParkingFacility => &[4],
    }
}

fn push_stable_ref(
    staged: &mut BoundedVec<u8>,
    index: &ArtifactIndex<'_>,
    entity_kind: EntityKind,
    typed_ordinal: u32,
    mismatch: PortableEmissionError,
) -> Result<(), PortableEmissionError> {
    staged.extend_from_slice(&entity_kind.code().to_le_bytes())?;
    staged.extend_from_slice(&index.stable_id(entity_kind, typed_ordinal, mismatch)?)?;
    Ok(())
}

fn push_path_anchor_stable_value(
    staged: &mut BoundedVec<u8>,
    index: &ArtifactIndex<'_>,
    passage: RegistryCheckedRowView<'_>,
    kind_tag: u16,
    reference_tag: u16,
    progress_tag: u16,
    mismatch: PortableEmissionError,
) -> Result<(), PortableEmissionError> {
    let kind = checked_u8_with(passage, kind_tag, mismatch)?;
    staged.push(kind)?;
    match kind {
        0 => push_stable_ref(
            staged,
            index,
            EntityKind::ManeuverGate,
            checked_u32_with(passage, reference_tag, mismatch)?,
            mismatch,
        )?,
        1 => staged.extend_from_slice(
            &checked_u32_with(passage, reference_tag, mismatch)?.to_le_bytes(),
        )?,
        2 => {
            staged.extend_from_slice(
                &checked_u32_with(passage, reference_tag, mismatch)?.to_le_bytes(),
            )?;
            staged.extend_from_slice(
                &checked_u32_with(passage, progress_tag, mismatch)?.to_le_bytes(),
            )?;
        }
        _ => return Err(mismatch),
    }
    Ok(())
}

/// 以稳定引用展开的字段：被引用实体种类。
fn stable_ref_kind(entity_kind: EntityKind, tag: u16) -> Option<EntityKind> {
    match (entity_kind, tag) {
        (EntityKind::RoadCorridor, 3) => Some(EntityKind::RoadSection),
        (EntityKind::StopLine, 3) => Some(EntityKind::LaneEdge),
        (EntityKind::WaitingZone, 4 | 5) => Some(EntityKind::ManeuverGate),
        _ => None,
    }
}

/// 字段语义值写入暂存字节区的上界；原样借用制品字节的字段为零。
fn staged_field_bound(
    entity_kind: EntityKind,
    entity: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<usize, PortableEmissionError> {
    if entity.field_by_tag(tag).is_none() {
        return Ok(0);
    }
    if stable_ref_kind(entity_kind, tag).is_some() {
        return Ok(18);
    }
    let per_record = match (entity_kind, tag) {
        (EntityKind::SignalPhase, 5) => 19,
        (EntityKind::ParkingFacility, 5 | 6) => 22,
        // 冲突区稳定引用 18 字节，两端锚点各至多 1 字节种类加 18 字节引用。
        (EntityKind::ParticipantStream, 5) => 56,
        _ => return Ok(0),
    };
    let records = checked_record_vector_with(entity, tag, mismatch)?;
    usize::try_from(records.len())
        .ok()
        .and_then(|count| count.checked_mul(per_record))
        .and_then(|bytes| bytes.checked_add(4))
        .ok_or(PortableEmissionError::ArithmeticOverflow)
}

/// 投影字段的语义值：稳定引用与记录向量展开写入暂存字节区，其余字段借用原字节。
fn semantic_field_value<'a>(
    staged: &mut BoundedVec<u8>,
    index: &ArtifactIndex<'a>,
    entity_kind: EntityKind,
    entity: RegistryCheckedRowView<'a>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<Option<Payload<'a>>, PortableEmissionError> {
    let Some(field) = entity.field_by_tag(tag) else {
        return Ok(None);
    };
    if let Some(referenced_kind) = stable_ref_kind(entity_kind, tag) {
        let ordinal = checked_u32_with(entity, tag, mismatch)?;
        return Ok(Some(Payload::Staged(stage(staged, |staged| {
            push_stable_ref(staged, index, referenced_kind, ordinal, mismatch)
        })?)));
    }
    if (entity_kind, tag) == (EntityKind::SignalPhase, 5) {
        let states = checked_record_vector_with(entity, tag, mismatch)?;
        return Ok(Some(Payload::Staged(stage(staged, |staged| {
            staged.extend_from_slice(&states.len().to_le_bytes())?;
            for state in states.rows() {
                push_stable_ref(
                    staged,
                    index,
                    EntityKind::SignalGroup,
                    checked_u32_with(state, 1, mismatch)?,
                    mismatch,
                )?;
                staged.push(checked_u8_with(state, 2, mismatch)?)?;
            }
            Ok::<_, PortableEmissionError>(())
        })?)));
    }
    if entity_kind == EntityKind::ParkingFacility && matches!(tag, 5 | 6) {
        let anchors = checked_record_vector_with(entity, tag, mismatch)?;
        return Ok(Some(Payload::Staged(stage(staged, |staged| {
            staged.extend_from_slice(&anchors.len().to_le_bytes())?;
            for anchor in anchors.rows() {
                push_stable_ref(
                    staged,
                    index,
                    EntityKind::LaneEdge,
                    checked_u32_with(anchor, 1, mismatch)?,
                    mismatch,
                )?;
                staged.extend_from_slice(&checked_u32_with(anchor, 2, mismatch)?.to_le_bytes())?;
            }
            Ok::<_, PortableEmissionError>(())
        })?)));
    }
    if (entity_kind, tag) == (EntityKind::ParticipantStream, 5) {
        let passages = checked_record_vector_with(entity, tag, mismatch)?;
        return Ok(Some(Payload::Staged(stage(staged, |staged| {
            staged.extend_from_slice(&passages.len().to_le_bytes())?;
            for passage in passages.rows() {
                push_stable_ref(
                    staged,
                    index,
                    EntityKind::ConflictZone,
                    checked_u32_with(passage, 1, mismatch)?,
                    mismatch,
                )?;
                push_path_anchor_stable_value(staged, index, passage, 2, 3, 4, mismatch)?;
                push_path_anchor_stable_value(staged, index, passage, 5, 6, 7, mismatch)?;
            }
            Ok::<_, PortableEmissionError>(())
        })?)));
    }
    Ok(Some(Payload::View(field.value_bytes())))
}

/// 实体差异（第 2 节）与静态规则差异（第 5 节）变化表的容量与暂存字节上界。
///
/// 新增与删除各至多一侧实体数；修改至多为 base 实体可修改字段数之和；暂存字节至多为
/// 两侧全部实体可修改字段语义值上界之和。
pub(super) fn field_change_bounds(
    base: &ArtifactIndex<'_>,
    target: &ArtifactIndex<'_>,
    bounds: &mut ChangeBounds,
) -> Result<(), PortableEmissionError> {
    add(&mut bounds.entity, base.len())?;
    add(&mut bounds.entity, target.len())?;
    for ((entity_kind, _), _) in base.entities() {
        add(&mut bounds.entity, entity_modify_tags(*entity_kind).len())?;
        add(
            &mut bounds.static_rule,
            static_rule_modify_tags(*entity_kind).len(),
        )?;
    }
    for (index, mismatch) in [
        (base, PortableEmissionError::DiffBaseSemanticMismatch),
        (target, PortableEmissionError::InternalBindingMismatch),
    ] {
        for ((entity_kind, _), entity) in index.entities() {
            for tag in entity_modify_tags(*entity_kind)
                .iter()
                .chain(static_rule_modify_tags(*entity_kind))
            {
                add(
                    &mut bounds.staged,
                    staged_field_bound(*entity_kind, entity.row, *tag, mismatch)?,
                )?;
            }
        }
    }
    Ok(())
}

fn retained_field_changes<'a>(
    base: &ArtifactIndex<'a>,
    target: &ArtifactIndex<'a>,
    tags: fn(EntityKind) -> &'static [u16],
    change_kind: u8,
    changes: &mut BoundedVec<FieldChange<'a>>,
    staged: &mut BoundedVec<u8>,
) -> Result<(), PortableEmissionError> {
    for ((entity_kind, stable_id), base_entity) in base.entities() {
        let Some(target_entity) = target.entity(&(*entity_kind, *stable_id)) else {
            continue;
        };
        for tag in tags(*entity_kind) {
            let mark = staged.len();
            let before = semantic_field_value(
                staged,
                base,
                *entity_kind,
                base_entity.row,
                *tag,
                PortableEmissionError::DiffBaseSemanticMismatch,
            )?;
            let after = semantic_field_value(
                staged,
                target,
                *entity_kind,
                target_entity.row,
                *tag,
                PortableEmissionError::InternalBindingMismatch,
            )?;
            if before.map(|value| value.bytes(staged)) == after.map(|value| value.bytes(staged)) {
                staged.truncate(mark);
                continue;
            }
            changes.push(FieldChange {
                change_kind,
                entity_kind: *entity_kind,
                stable_id: *stable_id,
                field_tag: *tag,
                before,
                after,
            })?;
        }
    }
    Ok(())
}

/// 写入两版制品之间的实体增删改变化（LFSD 第 2 节）。
///
/// 以稳定标识配对实体并投影可修改字段的前后值；结果按变更种类、实体种类、
/// 稳定标识与字段标签规范排序。
pub(super) fn artifact_entity_changes<'a>(
    base: &ArtifactIndex<'a>,
    target: &ArtifactIndex<'a>,
    changes: &mut BoundedVec<FieldChange<'a>>,
    staged: &mut BoundedVec<u8>,
) -> Result<(), PortableEmissionError> {
    for ((entity_kind, stable_id), entity) in target.entities() {
        if base.entity(&(*entity_kind, *stable_id)).is_none() {
            changes.push(FieldChange {
                change_kind: 0,
                entity_kind: *entity_kind,
                stable_id: *stable_id,
                field_tag: 0,
                before: None,
                after: Some(Payload::View(entity.row.bytes())),
            })?;
        }
    }
    for ((entity_kind, stable_id), entity) in base.entities() {
        if target.entity(&(*entity_kind, *stable_id)).is_none() {
            changes.push(FieldChange {
                change_kind: 1,
                entity_kind: *entity_kind,
                stable_id: *stable_id,
                field_tag: 0,
                before: Some(Payload::View(entity.row.bytes())),
                after: None,
            })?;
        }
    }
    retained_field_changes(base, target, entity_modify_tags, 2, changes, staged)?;
    changes.sort_unstable_by_key(FieldChange::key);
    Ok(())
}

/// 写入两版制品之间的静态规则字段修改变化（LFSD 第 5 节）。
///
/// 只覆盖划分给静态规则的字段，与第 2 节实体字段的分工互不重叠。
pub(super) fn artifact_static_rule_changes<'a>(
    base: &ArtifactIndex<'a>,
    target: &ArtifactIndex<'a>,
    changes: &mut BoundedVec<FieldChange<'a>>,
    staged: &mut BoundedVec<u8>,
) -> Result<(), PortableEmissionError> {
    retained_field_changes(base, target, static_rule_modify_tags, 0, changes, staged)?;
    changes.sort_unstable_by_key(FieldChange::key);
    Ok(())
}

fn checked_stable_id(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
) -> Result<[u8; 16], PortableEmissionError> {
    match row
        .field_by_tag(tag)
        .ok_or(PortableEmissionError::InternalBindingMismatch)?
        .value()?
    {
        RegistryCheckedFieldValue::StableId128(value) => Ok(value.into_bytes()),
        _ => Err(PortableEmissionError::InternalBindingMismatch),
    }
}

/// Genesis 实体变化表的容量：目标全部实体行数。
pub(super) fn genesis_entity_bound(
    target: RegistryCheckedObjectView<'_>,
) -> Result<usize, PortableEmissionError> {
    let section = target
        .section(2)
        .ok_or(PortableEmissionError::InternalBindingMismatch)?;
    section.tables().try_fold(0_usize, |count, table| {
        count
            .checked_add(table.row_count() as usize)
            .ok_or(PortableEmissionError::ArithmeticOverflow)
    })
}

/// 把目标制品的全部实体写为 Genesis 新增变化，按实体种类与稳定标识规范排序。
pub(super) fn genesis_entity_changes<'a>(
    target: RegistryCheckedObjectView<'a>,
    changes: &mut BoundedVec<FieldChange<'a>>,
) -> Result<(), PortableEmissionError> {
    let section = target
        .section(2)
        .ok_or(PortableEmissionError::InternalBindingMismatch)?;
    let mut tables = section.tables();
    for entity_kind in EntityKind::ALL {
        if !entity_kind.is_constructible() {
            continue;
        }
        let table = tables
            .next()
            .ok_or(PortableEmissionError::InternalBindingMismatch)?;
        for entity in table.rows() {
            changes.push(FieldChange {
                change_kind: 0,
                entity_kind,
                stable_id: checked_stable_id(entity, 2)?,
                field_tag: 0,
                before: None,
                after: Some(Payload::View(entity.bytes())),
            })?;
        }
    }
    if tables.next().is_some() {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    changes.sort_unstable_by_key(FieldChange::key);
    Ok(())
}

/// 读取目标制品行指定标签的 `U32` 字段；字段缺失或类型不符即报内部绑定错误。
pub(super) fn checked_u32(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
) -> Result<u32, PortableEmissionError> {
    match row
        .field_by_tag(tag)
        .ok_or(PortableEmissionError::InternalBindingMismatch)?
        .value()?
    {
        RegistryCheckedFieldValue::U32(value) => Ok(value),
        _ => Err(PortableEmissionError::InternalBindingMismatch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entity_kind_has_one_frozen_entity_and_static_rule_field_partition() {
        let expected = [
            (EntityKind::RoadCorridor, &[3_u16][..], &[][..]),
            (EntityKind::RoadSection, &[4][..], &[][..]),
            (EntityKind::AuthoringLane, &[][..], &[][..]),
            (EntityKind::LaneEdge, &[3, 4][..], &[][..]),
            (EntityKind::Junction, &[][..], &[][..]),
            (EntityKind::Movement, &[7][..], &[][..]),
            (EntityKind::ManeuverPath, &[][..], &[][..]),
            (EntityKind::ManeuverGate, &[4][..], &[6][..]),
            (EntityKind::WaitingZone, &[][..], &[4, 5, 6][..]),
            (EntityKind::StopLine, &[3][..], &[][..]),
            (EntityKind::SignalGroup, &[][..], &[][..]),
            (EntityKind::SignalController, &[][..], &[3, 4][..]),
            (EntityKind::SignalPhase, &[][..], &[4, 5][..]),
            (EntityKind::ParkingFacility, &[5, 6][..], &[4][..]),
            (EntityKind::ParkingSpace, &[5, 7, 8, 9, 10, 11][..], &[][..]),
            (EntityKind::LaneGroup, &[][..], &[][..]),
            (EntityKind::FacilityBand, &[4][..], &[][..]),
            (EntityKind::ParticipantClass, &[4][..], &[][..]),
            (EntityKind::AccessRule, &[][..], &[5, 7, 8][..]),
            (
                EntityKind::VehicleProfile,
                &[4, 5, 6, 7, 8, 9, 10][..],
                &[][..],
            ),
            (EntityKind::ConflictZone, &[][..], &[][..]),
            (EntityKind::CanonicalFrame, &[][..], &[][..]),
            (EntityKind::ParticipantStream, &[5][..], &[][..]),
            (EntityKind::RightOfWayPolicySet, &[][..], &[3, 4, 5][..]),
        ];
        assert_eq!(expected.len(), EntityKind::ALL.len());
        for (actual, (kind, entity_tags, static_rule_tags)) in
            EntityKind::ALL.into_iter().zip(expected)
        {
            assert_eq!(actual, kind);
            assert_eq!(
                entity_modify_tags(kind),
                entity_tags,
                "{kind:?} entity fields"
            );
            assert_eq!(
                static_rule_modify_tags(kind),
                static_rule_tags,
                "{kind:?} static-rule fields"
            );
        }
    }
}
