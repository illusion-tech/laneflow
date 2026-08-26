use super::base::{ArtifactIndex, checked_record_vector_with, checked_u8_with, checked_u32_with};
use super::*;

fn entity_modify_tags(entity_kind: EntityKind) -> &'static [u16] {
    match entity_kind {
        EntityKind::RoadCorridor => &[3],
        EntityKind::RoadSection => &[4],
        EntityKind::AuthoringLane
        | EntityKind::Junction
        | EntityKind::Movement
        | EntityKind::ManeuverPath
        | EntityKind::WaitingZone
        | EntityKind::SignalGroup
        | EntityKind::SignalController
        | EntityKind::SignalPhase
        | EntityKind::ParkingArea
        | EntityKind::LaneGroup
        | EntityKind::AccessRule
        | EntityKind::StaticRoute
        | EntityKind::CanonicalFrame => &[],
        EntityKind::LaneEdge => &[3, 4],
        EntityKind::ManeuverGate => &[4],
        EntityKind::StopLine => &[3],
        EntityKind::ParkingSpace => &[5, 7, 8, 9, 10, 11],
        EntityKind::FacilityBand | EntityKind::ParticipantClass => &[4],
        EntityKind::VehicleProfile => &[4, 5, 6, 7, 8, 9, 10],
    }
}

fn static_rule_modify_tags(entity_kind: EntityKind) -> &'static [u16] {
    match entity_kind {
        EntityKind::ManeuverGate => &[6],
        EntityKind::WaitingZone => &[4, 5, 6],
        EntityKind::SignalController => &[3, 4],
        EntityKind::SignalPhase => &[4, 5],
        EntityKind::AccessRule => &[5, 7, 8],
        EntityKind::RoadCorridor
        | EntityKind::RoadSection
        | EntityKind::AuthoringLane
        | EntityKind::LaneEdge
        | EntityKind::Junction
        | EntityKind::Movement
        | EntityKind::ManeuverPath
        | EntityKind::StopLine
        | EntityKind::SignalGroup
        | EntityKind::ParkingArea
        | EntityKind::ParkingSpace
        | EntityKind::LaneGroup
        | EntityKind::FacilityBand
        | EntityKind::ParticipantClass
        | EntityKind::VehicleProfile
        | EntityKind::StaticRoute
        | EntityKind::CanonicalFrame => &[],
    }
}

fn stable_ref_value(
    index: &ArtifactIndex<'_>,
    entity_kind: EntityKind,
    typed_ordinal: u32,
    mismatch: PortableEmissionError,
) -> Result<Box<[u8]>, PortableEmissionError> {
    let mut value = Vec::with_capacity(18);
    value.extend_from_slice(&entity_kind.code().to_le_bytes());
    value.extend_from_slice(&index.stable_id(entity_kind, typed_ordinal, mismatch)?);
    Ok(value.into_boxed_slice())
}

fn semantic_field_value(
    index: &ArtifactIndex<'_>,
    entity_kind: EntityKind,
    entity: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<Option<Box<[u8]>>, PortableEmissionError> {
    let Some(field) = entity.field_by_tag(tag) else {
        return Ok(None);
    };
    let stable_ref_kind = match (entity_kind, tag) {
        (EntityKind::RoadCorridor, 3) => Some(EntityKind::RoadSection),
        (EntityKind::StopLine, 3) => Some(EntityKind::LaneEdge),
        (EntityKind::WaitingZone, 4 | 5) => Some(EntityKind::ManeuverGate),
        _ => None,
    };
    if let Some(referenced_kind) = stable_ref_kind {
        return Ok(Some(stable_ref_value(
            index,
            referenced_kind,
            checked_u32_with(entity, tag, mismatch)?,
            mismatch,
        )?));
    }
    if (entity_kind, tag) == (EntityKind::SignalPhase, 5) {
        let states = checked_record_vector_with(entity, tag, mismatch)?;
        let capacity = usize::try_from(states.len())
            .map_err(|_| PortableEmissionError::ArithmeticOverflow)?
            .checked_mul(19)
            .and_then(|value| value.checked_add(4))
            .ok_or(PortableEmissionError::ArithmeticOverflow)?;
        let mut value = Vec::with_capacity(capacity);
        value.extend_from_slice(&states.len().to_le_bytes());
        for state in states.rows() {
            value.extend_from_slice(&stable_ref_value(
                index,
                EntityKind::SignalGroup,
                checked_u32_with(state, 1, mismatch)?,
                mismatch,
            )?);
            value.push(checked_u8_with(state, 2, mismatch)?);
        }
        return Ok(Some(value.into_boxed_slice()));
    }
    Ok(Some(field.value_bytes().to_vec().into_boxed_slice()))
}

#[derive(Debug, Eq, PartialEq)]
struct FieldChangeProjection {
    entity_kind: EntityKind,
    stable_id: [u8; 16],
    field_tag: u16,
    before: Option<Box<[u8]>>,
    after: Option<Box<[u8]>>,
}

fn retained_field_changes(
    base: &ArtifactIndex<'_>,
    target: &ArtifactIndex<'_>,
    tags: fn(EntityKind) -> &'static [u16],
) -> Result<Vec<FieldChangeProjection>, PortableEmissionError> {
    let mut changes = Vec::new();
    for ((entity_kind, stable_id), base_entity) in &base.entities {
        let Some(target_entity) = target.entities.get(&(*entity_kind, *stable_id)) else {
            continue;
        };
        for tag in tags(*entity_kind) {
            let before = semantic_field_value(
                base,
                *entity_kind,
                base_entity.row,
                *tag,
                PortableEmissionError::DiffBaseSemanticMismatch,
            )?;
            let after = semantic_field_value(
                target,
                *entity_kind,
                target_entity.row,
                *tag,
                PortableEmissionError::InternalBindingMismatch,
            )?;
            if before != after {
                changes.push(FieldChangeProjection {
                    entity_kind: *entity_kind,
                    stable_id: *stable_id,
                    field_tag: *tag,
                    before,
                    after,
                });
            }
        }
    }
    changes.sort_unstable_by_key(|change| (change.entity_kind, change.stable_id, change.field_tag));
    Ok(changes)
}

pub(super) fn artifact_entity_changes(
    base: &ArtifactIndex<'_>,
    target: &ArtifactIndex<'_>,
) -> Result<Vec<OwnedRow>, PortableEmissionError> {
    let mut changes = Vec::<(u8, EntityKind, [u8; 16], u16, OwnedRow)>::new();
    for ((entity_kind, stable_id), entity) in &target.entities {
        if !base.entities.contains_key(&(*entity_kind, *stable_id)) {
            changes.push((
                0,
                *entity_kind,
                *stable_id,
                0,
                row([
                    field(1, OwnedValue::U8(0)),
                    field(2, OwnedValue::U16(entity_kind.code())),
                    field(4, OwnedValue::StableId128(*stable_id)),
                    field(
                        10,
                        OwnedValue::Bytes(entity.row.bytes().to_vec().into_boxed_slice()),
                    ),
                ]),
            ));
        }
    }
    for ((entity_kind, stable_id), entity) in &base.entities {
        if !target.entities.contains_key(&(*entity_kind, *stable_id)) {
            changes.push((
                1,
                *entity_kind,
                *stable_id,
                0,
                row([
                    field(1, OwnedValue::U8(1)),
                    field(2, OwnedValue::U16(entity_kind.code())),
                    field(4, OwnedValue::StableId128(*stable_id)),
                    field(
                        9,
                        OwnedValue::Bytes(entity.row.bytes().to_vec().into_boxed_slice()),
                    ),
                ]),
            ));
        }
    }
    for change in retained_field_changes(base, target, entity_modify_tags)? {
        let mut fields = vec![
            field(1, OwnedValue::U8(2)),
            field(2, OwnedValue::U16(change.entity_kind.code())),
            field(4, OwnedValue::StableId128(change.stable_id)),
            field(6, OwnedValue::U16(change.field_tag)),
        ];
        if let Some(before) = change.before {
            fields.push(field(9, OwnedValue::Bytes(before)));
        }
        if let Some(after) = change.after {
            fields.push(field(10, OwnedValue::Bytes(after)));
        }
        changes.push((
            2,
            change.entity_kind,
            change.stable_id,
            change.field_tag,
            row(fields),
        ));
    }
    changes.sort_unstable_by_key(|(change_kind, entity_kind, stable_id, field_tag, _)| {
        (*change_kind, *entity_kind, *stable_id, *field_tag)
    });
    Ok(changes.into_iter().map(|(_, _, _, _, row)| row).collect())
}

pub(super) fn artifact_static_rule_changes(
    base: &ArtifactIndex<'_>,
    target: &ArtifactIndex<'_>,
) -> Result<Vec<OwnedRow>, PortableEmissionError> {
    retained_field_changes(base, target, static_rule_modify_tags)?
        .into_iter()
        .map(|change| {
            let mut fields = vec![
                field(1, OwnedValue::U8(0)),
                field(2, OwnedValue::U16(change.entity_kind.code())),
                field(4, OwnedValue::StableId128(change.stable_id)),
                field(6, OwnedValue::U16(change.field_tag)),
            ];
            if let Some(before) = change.before {
                fields.push(field(9, OwnedValue::Bytes(before)));
            }
            if let Some(after) = change.after {
                fields.push(field(10, OwnedValue::Bytes(after)));
            }
            Ok(row(fields))
        })
        .collect()
}

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

pub(super) fn genesis_entity_changes(
    target: RegistryCheckedObjectView<'_>,
) -> Result<Vec<OwnedRow>, PortableEmissionError> {
    let section = target
        .section(2)
        .ok_or(PortableEmissionError::InternalBindingMismatch)?;
    let mut changes = Vec::new();
    let mut tables = section.tables();
    for entity_kind in EntityKind::ALL {
        if !entity_kind.is_constructible() {
            continue;
        }
        let table = tables
            .next()
            .ok_or(PortableEmissionError::InternalBindingMismatch)?;
        for entity in table.rows() {
            changes.push((
                entity_kind,
                checked_stable_id(entity, 2)?,
                entity.bytes().to_vec().into_boxed_slice(),
            ));
        }
    }
    if tables.next().is_some() {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    changes.sort_unstable_by_key(|(kind, stable_id, _)| (*kind, *stable_id));
    Ok(changes
        .into_iter()
        .map(|(kind, stable_id, bytes)| {
            row([
                field(1, OwnedValue::U8(0)),
                field(2, OwnedValue::U16(kind.code())),
                field(4, OwnedValue::StableId128(stable_id)),
                field(10, OwnedValue::Bytes(bytes)),
            ])
        })
        .collect())
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
            (EntityKind::Movement, &[][..], &[][..]),
            (EntityKind::ManeuverPath, &[][..], &[][..]),
            (EntityKind::ManeuverGate, &[4][..], &[6][..]),
            (EntityKind::WaitingZone, &[][..], &[4, 5, 6][..]),
            (EntityKind::StopLine, &[3][..], &[][..]),
            (EntityKind::SignalGroup, &[][..], &[][..]),
            (EntityKind::SignalController, &[][..], &[3, 4][..]),
            (EntityKind::SignalPhase, &[][..], &[4, 5][..]),
            (EntityKind::ParkingArea, &[][..], &[][..]),
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
            (EntityKind::StaticRoute, &[][..], &[][..]),
            (EntityKind::CanonicalFrame, &[][..], &[][..]),
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
