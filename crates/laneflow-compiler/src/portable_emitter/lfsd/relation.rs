use super::base::{
    ArtifactIndex, checked_ordinal_vector_with, checked_record_vector_with, checked_u8_with,
    checked_u32_with,
};
use super::*;

#[allow(
    clippy::too_many_arguments,
    reason = "the closed A.5 tuple is clearer when every semantic coordinate is explicit"
)]
fn push_artifact_relation(
    relations: &mut Vec<RelationTuple>,
    index: &ArtifactIndex<'_>,
    owner_entity_kind: EntityKind,
    owner_stable_id: [u8; 16],
    role: u8,
    local_index: u32,
    subject_entity_kind: EntityKind,
    subject_ordinal: u32,
    mismatch: PortableEmissionError,
) -> Result<(), PortableEmissionError> {
    relations.push(RelationTuple {
        owner_entity_kind,
        owner_stable_id,
        role,
        local_index,
        subject_entity_kind,
        subject_stable_id: index.stable_id(subject_entity_kind, subject_ordinal, mismatch)?,
    });
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "the closed A.5 vector projection keeps owner, field, role, and subject kind explicit"
)]
fn push_vector_relations(
    relations: &mut Vec<RelationTuple>,
    index: &ArtifactIndex<'_>,
    owner_entity_kind: EntityKind,
    owner_stable_id: [u8; 16],
    owner: RegistryCheckedRowView<'_>,
    field_tag: u16,
    role: u8,
    subject_entity_kind: EntityKind,
    mismatch: PortableEmissionError,
) -> Result<(), PortableEmissionError> {
    let values = checked_ordinal_vector_with(owner, field_tag, mismatch)?;
    for local_index in 0..values.len() {
        push_artifact_relation(
            relations,
            index,
            owner_entity_kind,
            owner_stable_id,
            role,
            local_index,
            subject_entity_kind,
            values.get(local_index).ok_or(mismatch)?,
            mismatch,
        )?;
    }
    Ok(())
}

pub(super) fn artifact_relation_tuples(
    index: &ArtifactIndex<'_>,
    mismatch: PortableEmissionError,
) -> Result<Vec<RelationTuple>, PortableEmissionError> {
    let mut relations = Vec::new();
    for ((owner_kind, owner_stable_id), owner) in &index.entities {
        match owner_kind {
            EntityKind::RoadCorridor => {
                let elements = checked_record_vector_with(owner.row, 4, mismatch)?;
                for (local_index, element) in elements.rows().enumerate() {
                    let local_index = u32::try_from(local_index)
                        .map_err(|_| PortableEmissionError::ArithmeticOverflow)?;
                    let subject_kind = match checked_u8_with(element, 1, mismatch)? {
                        0 => EntityKind::RoadSection,
                        1 => EntityKind::FacilityBand,
                        _ => return Err(mismatch),
                    };
                    push_artifact_relation(
                        &mut relations,
                        index,
                        *owner_kind,
                        *owner_stable_id,
                        2,
                        local_index,
                        subject_kind,
                        checked_u32_with(element, 2, mismatch)?,
                        mismatch,
                    )?;
                }
            }
            EntityKind::RoadSection => push_vector_relations(
                &mut relations,
                index,
                *owner_kind,
                *owner_stable_id,
                owner.row,
                5,
                3,
                EntityKind::AuthoringLane,
                mismatch,
            )?,
            EntityKind::AuthoringLane => push_vector_relations(
                &mut relations,
                index,
                *owner_kind,
                *owner_stable_id,
                owner.row,
                4,
                4,
                EntityKind::LaneEdge,
                mismatch,
            )?,
            EntityKind::LaneEdge => push_vector_relations(
                &mut relations,
                index,
                *owner_kind,
                *owner_stable_id,
                owner.row,
                5,
                1,
                EntityKind::LaneEdge,
                mismatch,
            )?,
            EntityKind::Junction => push_vector_relations(
                &mut relations,
                index,
                *owner_kind,
                *owner_stable_id,
                owner.row,
                3,
                6,
                EntityKind::Movement,
                mismatch,
            )?,
            EntityKind::Movement => push_vector_relations(
                &mut relations,
                index,
                *owner_kind,
                *owner_stable_id,
                owner.row,
                6,
                7,
                EntityKind::ManeuverPath,
                mismatch,
            )?,
            EntityKind::ManeuverPath => {
                for (field_tag, role, subject_kind) in [
                    (4, 8, EntityKind::LaneEdge),
                    (5, 10, EntityKind::ManeuverGate),
                    (6, 11, EntityKind::WaitingZone),
                ] {
                    push_vector_relations(
                        &mut relations,
                        index,
                        *owner_kind,
                        *owner_stable_id,
                        owner.row,
                        field_tag,
                        role,
                        subject_kind,
                        mismatch,
                    )?;
                }
            }
            EntityKind::ManeuverGate => {
                if let Some(signal_group) = owner.row.field_by_tag(7) {
                    push_artifact_relation(
                        &mut relations,
                        index,
                        *owner_kind,
                        *owner_stable_id,
                        20,
                        0,
                        EntityKind::SignalGroup,
                        match signal_group.value()? {
                            RegistryCheckedFieldValue::U32(value) => value,
                            _ => return Err(mismatch),
                        },
                        mismatch,
                    )?;
                }
            }
            EntityKind::StopLine => push_vector_relations(
                &mut relations,
                index,
                *owner_kind,
                *owner_stable_id,
                owner.row,
                4,
                12,
                EntityKind::ManeuverGate,
                mismatch,
            )?,
            EntityKind::SignalController => {
                for (field_tag, role, subject_kind) in [
                    (5, 17, EntityKind::SignalGroup),
                    (6, 18, EntityKind::SignalPhase),
                ] {
                    push_vector_relations(
                        &mut relations,
                        index,
                        *owner_kind,
                        *owner_stable_id,
                        owner.row,
                        field_tag,
                        role,
                        subject_kind,
                        mismatch,
                    )?;
                }
            }
            EntityKind::ParkingSpace => {
                if let Some(parking_area) = owner.row.field_by_tag(3) {
                    let ordinal = match parking_area.value()? {
                        RegistryCheckedFieldValue::U32(value) => value,
                        _ => return Err(mismatch),
                    };
                    push_artifact_relation(
                        &mut relations,
                        index,
                        *owner_kind,
                        *owner_stable_id,
                        21,
                        0,
                        EntityKind::ParkingArea,
                        ordinal,
                        mismatch,
                    )?;
                }
                for (field_tag, role) in [(4, 22), (6, 23)] {
                    push_artifact_relation(
                        &mut relations,
                        index,
                        *owner_kind,
                        *owner_stable_id,
                        role,
                        0,
                        EntityKind::LaneEdge,
                        checked_u32_with(owner.row, field_tag, mismatch)?,
                        mismatch,
                    )?;
                }
            }
            EntityKind::LaneGroup => push_vector_relations(
                &mut relations,
                index,
                *owner_kind,
                *owner_stable_id,
                owner.row,
                4,
                5,
                EntityKind::AuthoringLane,
                mismatch,
            )?,
            EntityKind::ParticipantClass => {
                if let Some(parent) = owner.row.field_by_tag(3) {
                    let ordinal = match parent.value()? {
                        RegistryCheckedFieldValue::U32(value) => value,
                        _ => return Err(mismatch),
                    };
                    push_artifact_relation(
                        &mut relations,
                        index,
                        *owner_kind,
                        *owner_stable_id,
                        24,
                        0,
                        EntityKind::ParticipantClass,
                        ordinal,
                        mismatch,
                    )?;
                }
            }
            EntityKind::AccessRule => {
                let subject_kind = match checked_u8_with(owner.row, 3, mismatch)? {
                    0 => EntityKind::LaneEdge,
                    1 => EntityKind::LaneGroup,
                    2 => EntityKind::RoadSection,
                    3 => EntityKind::ManeuverPath,
                    _ => return Err(mismatch),
                };
                push_artifact_relation(
                    &mut relations,
                    index,
                    *owner_kind,
                    *owner_stable_id,
                    25,
                    0,
                    subject_kind,
                    checked_u32_with(owner.row, 4, mismatch)?,
                    mismatch,
                )?;
                push_vector_relations(
                    &mut relations,
                    index,
                    *owner_kind,
                    *owner_stable_id,
                    owner.row,
                    6,
                    26,
                    EntityKind::ParticipantClass,
                    mismatch,
                )?;
            }
            EntityKind::VehicleProfile => push_artifact_relation(
                &mut relations,
                index,
                *owner_kind,
                *owner_stable_id,
                27,
                0,
                EntityKind::ParticipantClass,
                checked_u32_with(owner.row, 3, mismatch)?,
                mismatch,
            )?,
            EntityKind::StaticRoute
            | EntityKind::WaitingZone
            | EntityKind::SignalGroup
            | EntityKind::SignalPhase
            | EntityKind::ParkingArea
            | EntityKind::FacilityBand
            | EntityKind::CanonicalFrame => {}
        }
    }

    let relation_section = index.view.section(3).ok_or(mismatch)?;
    let internal_edges = relation_section.table(0).ok_or(mismatch)?;
    let mut next_internal_index = BTreeMap::<[u8; 16], u32>::new();
    for relation in internal_edges.rows() {
        let owner_stable_id = index.stable_id(
            EntityKind::Junction,
            checked_u32_with(relation, 2, mismatch)?,
            mismatch,
        )?;
        let local_index = next_internal_index.entry(owner_stable_id).or_default();
        push_artifact_relation(
            &mut relations,
            index,
            EntityKind::Junction,
            owner_stable_id,
            9,
            *local_index,
            EntityKind::LaneEdge,
            checked_u32_with(relation, 1, mismatch)?,
            mismatch,
        )?;
        *local_index = local_index
            .checked_add(1)
            .ok_or(PortableEmissionError::ArithmeticOverflow)?;
    }
    relations.sort_unstable();
    Ok(relations)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RelationPairing {
    Set,
    Scalar,
    DomainOccurrence,
}

fn relation_pairing(role: u8) -> Option<RelationPairing> {
    match role {
        1 | 6 | 7 | 9 | 12 | 17 | 26 => Some(RelationPairing::Set),
        20..=25 | 27 => Some(RelationPairing::Scalar),
        2..=5 | 8 | 10 | 11 | 18 => Some(RelationPairing::DomainOccurrence),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RelationChangeProjection {
    change_kind: u8,
    owner_entity_kind: EntityKind,
    owner_stable_id: [u8; 16],
    role: u8,
    subject_stable_id: Option<[u8; 16]>,
    before_local_index: Option<u32>,
    after_local_index: Option<u32>,
    before_target: Option<[u8; 16]>,
    after_target: Option<[u8; 16]>,
}

fn relation_add(tuple: RelationTuple) -> RelationChangeProjection {
    RelationChangeProjection {
        change_kind: 0,
        owner_entity_kind: tuple.owner_entity_kind,
        owner_stable_id: tuple.owner_stable_id,
        role: tuple.role,
        subject_stable_id: Some(tuple.subject_stable_id),
        before_local_index: None,
        after_local_index: Some(tuple.local_index),
        before_target: None,
        after_target: None,
    }
}

fn relation_remove(tuple: RelationTuple) -> RelationChangeProjection {
    RelationChangeProjection {
        change_kind: 1,
        owner_entity_kind: tuple.owner_entity_kind,
        owner_stable_id: tuple.owner_stable_id,
        role: tuple.role,
        subject_stable_id: Some(tuple.subject_stable_id),
        before_local_index: Some(tuple.local_index),
        after_local_index: None,
        before_target: None,
        after_target: None,
    }
}

fn relation_move(before: RelationTuple, after: RelationTuple) -> RelationChangeProjection {
    RelationChangeProjection {
        change_kind: 2,
        owner_entity_kind: before.owner_entity_kind,
        owner_stable_id: before.owner_stable_id,
        role: before.role,
        subject_stable_id: Some(before.subject_stable_id),
        before_local_index: Some(before.local_index),
        after_local_index: Some(after.local_index),
        before_target: None,
        after_target: None,
    }
}

fn relation_reconnect(before: RelationTuple, after: RelationTuple) -> RelationChangeProjection {
    RelationChangeProjection {
        change_kind: 3,
        owner_entity_kind: before.owner_entity_kind,
        owner_stable_id: before.owner_stable_id,
        role: before.role,
        subject_stable_id: None,
        before_local_index: Some(before.local_index),
        after_local_index: Some(after.local_index),
        before_target: Some(before.subject_stable_id),
        after_target: Some(after.subject_stable_id),
    }
}

fn compare_relation_changes(
    left: &RelationChangeProjection,
    right: &RelationChangeProjection,
) -> std::cmp::Ordering {
    left.change_kind
        .cmp(&right.change_kind)
        .then_with(|| left.owner_entity_kind.cmp(&right.owner_entity_kind))
        .then_with(|| left.owner_stable_id.cmp(&right.owner_stable_id))
        .then_with(|| left.role.cmp(&right.role))
        .then_with(|| match left.change_kind {
            0 => left
                .after_local_index
                .cmp(&right.after_local_index)
                .then_with(|| left.subject_stable_id.cmp(&right.subject_stable_id)),
            1 => left
                .before_local_index
                .cmp(&right.before_local_index)
                .then_with(|| left.subject_stable_id.cmp(&right.subject_stable_id)),
            2 => left
                .before_local_index
                .cmp(&right.before_local_index)
                .then_with(|| left.after_local_index.cmp(&right.after_local_index))
                .then_with(|| left.subject_stable_id.cmp(&right.subject_stable_id)),
            3 => left
                .before_local_index
                .cmp(&right.before_local_index)
                .then_with(|| left.after_local_index.cmp(&right.after_local_index))
                .then_with(|| left.before_target.cmp(&right.before_target))
                .then_with(|| left.after_target.cmp(&right.after_target)),
            _ => std::cmp::Ordering::Equal,
        })
}

fn group_relations(
    relations: Vec<RelationTuple>,
    mismatch: PortableEmissionError,
) -> Result<RelationGroups, PortableEmissionError> {
    let mut groups = BTreeMap::new();
    for relation in relations {
        groups
            .entry((
                relation.owner_entity_kind,
                relation.owner_stable_id,
                relation.role,
            ))
            .or_insert_with(Vec::new)
            .push(relation);
    }
    for relations in groups.values_mut() {
        relations.sort_unstable_by_key(|relation| relation.local_index);
        for (expected, relation) in relations.iter().enumerate() {
            if relation.local_index
                != u32::try_from(expected).map_err(|_| PortableEmissionError::ArithmeticOverflow)?
            {
                return Err(mismatch);
            }
        }
    }
    Ok(groups)
}

fn pair_set_relations(
    base: &[RelationTuple],
    target: &[RelationTuple],
    changes: &mut Vec<RelationChangeProjection>,
) -> Result<(), PortableEmissionError> {
    let mut base_members = BTreeMap::new();
    for relation in base {
        if base_members
            .insert(
                (relation.subject_entity_kind, relation.subject_stable_id),
                *relation,
            )
            .is_some()
        {
            return Err(PortableEmissionError::DiffBaseSemanticMismatch);
        }
    }
    let mut target_members = BTreeMap::new();
    for relation in target {
        if target_members
            .insert(
                (relation.subject_entity_kind, relation.subject_stable_id),
                *relation,
            )
            .is_some()
        {
            return Err(PortableEmissionError::InternalBindingMismatch);
        }
    }
    changes.extend(
        base_members
            .iter()
            .filter(|(subject, _)| !target_members.contains_key(subject))
            .map(|(_, relation)| relation_remove(*relation)),
    );
    changes.extend(
        target_members
            .iter()
            .filter(|(subject, _)| !base_members.contains_key(subject))
            .map(|(_, relation)| relation_add(*relation)),
    );
    Ok(())
}

fn pair_scalar_relations(
    base: &[RelationTuple],
    target: &[RelationTuple],
    changes: &mut Vec<RelationChangeProjection>,
) -> Result<(), PortableEmissionError> {
    if base.len() > 1 {
        return Err(PortableEmissionError::DiffBaseSemanticMismatch);
    }
    if target.len() > 1 {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    match (base.first().copied(), target.first().copied()) {
        (None, None) => {}
        (None, Some(after)) => changes.push(relation_add(after)),
        (Some(before), None) => changes.push(relation_remove(before)),
        (Some(before), Some(after))
            if (before.subject_entity_kind, before.subject_stable_id)
                != (after.subject_entity_kind, after.subject_stable_id) =>
        {
            changes.push(relation_reconnect(before, after));
        }
        (Some(_), Some(_)) => {}
    }
    Ok(())
}

fn pair_domain_relations(
    base: &[RelationTuple],
    target: &[RelationTuple],
    changes: &mut Vec<RelationChangeProjection>,
) {
    let mut base_occurrences = BTreeMap::<(EntityKind, [u8; 16]), Vec<RelationTuple>>::new();
    let mut target_occurrences = BTreeMap::<(EntityKind, [u8; 16]), Vec<RelationTuple>>::new();
    for relation in base {
        base_occurrences
            .entry((relation.subject_entity_kind, relation.subject_stable_id))
            .or_default()
            .push(*relation);
    }
    for relation in target {
        target_occurrences
            .entry((relation.subject_entity_kind, relation.subject_stable_id))
            .or_default()
            .push(*relation);
    }
    for occurrences in base_occurrences.values_mut() {
        occurrences.sort_unstable_by_key(|relation| relation.local_index);
    }
    for occurrences in target_occurrences.values_mut() {
        occurrences.sort_unstable_by_key(|relation| relation.local_index);
    }
    let mut subjects: Vec<_> = base_occurrences
        .keys()
        .chain(target_occurrences.keys())
        .copied()
        .collect();
    subjects.sort_unstable();
    subjects.dedup();
    for subject in &subjects {
        let before = base_occurrences.get(subject).map_or(&[][..], Vec::as_slice);
        let after = target_occurrences
            .get(subject)
            .map_or(&[][..], Vec::as_slice);
        let paired_count = before.len().min(after.len());
        for rank in 0..paired_count {
            if before[rank].local_index != after[rank].local_index {
                changes.push(relation_move(before[rank], after[rank]));
            }
        }
        changes.extend(before[paired_count..].iter().copied().map(relation_remove));
        changes.extend(after[paired_count..].iter().copied().map(relation_add));
    }
}

fn relation_change_row(change: RelationChangeProjection) -> OwnedRow {
    let mut fields = vec![
        field(1, OwnedValue::U8(change.change_kind)),
        field(2, OwnedValue::U16(change.owner_entity_kind.code())),
        field(3, OwnedValue::StableId128(change.owner_stable_id)),
    ];
    if let Some(subject) = change.subject_stable_id {
        fields.push(field(4, OwnedValue::StableId128(subject)));
    }
    fields.push(field(5, OwnedValue::U8(change.role)));
    if let Some(index) = change.before_local_index {
        fields.push(field(7, OwnedValue::U32(index)));
    }
    if let Some(index) = change.after_local_index {
        fields.push(field(8, OwnedValue::U32(index)));
    }
    if let Some(target) = change.before_target {
        fields.push(field(9, OwnedValue::StableId128(target)));
    }
    if let Some(target) = change.after_target {
        fields.push(field(10, OwnedValue::StableId128(target)));
    }
    row(fields)
}

pub(super) fn artifact_relation_changes(
    base: &ArtifactIndex<'_>,
    target: &ArtifactIndex<'_>,
) -> Result<Vec<OwnedRow>, PortableEmissionError> {
    let mut base_groups = group_relations(
        artifact_relation_tuples(base, PortableEmissionError::DiffBaseSemanticMismatch)?,
        PortableEmissionError::DiffBaseSemanticMismatch,
    )?;
    let mut target_groups = group_relations(
        artifact_relation_tuples(target, PortableEmissionError::InternalBindingMismatch)?,
        PortableEmissionError::InternalBindingMismatch,
    )?;
    let mut group_keys: Vec<_> = base_groups
        .keys()
        .chain(target_groups.keys())
        .copied()
        .collect();
    group_keys.sort_unstable();
    group_keys.dedup();
    let mut changes = Vec::new();
    for key in group_keys {
        let base_relations = base_groups.remove(&key).unwrap_or_default();
        let target_relations = target_groups.remove(&key).unwrap_or_default();
        match relation_pairing(key.2).ok_or(PortableEmissionError::InternalBindingMismatch)? {
            RelationPairing::Set => {
                pair_set_relations(&base_relations, &target_relations, &mut changes)?;
            }
            RelationPairing::Scalar => {
                pair_scalar_relations(&base_relations, &target_relations, &mut changes)?;
            }
            RelationPairing::DomainOccurrence => {
                pair_domain_relations(&base_relations, &target_relations, &mut changes);
            }
        }
    }
    changes.sort_unstable_by(compare_relation_changes);
    Ok(changes.into_iter().map(relation_change_row).collect())
}

pub(super) fn genesis_relation_changes(lir: &crate::lir::LirUnit) -> Vec<OwnedRow> {
    canonical_relation_tuples(lir)
        .into_iter()
        .map(|relation| {
            row([
                field(1, OwnedValue::U8(0)),
                field(2, OwnedValue::U16(relation.owner_entity_kind.code())),
                field(3, OwnedValue::StableId128(relation.owner_stable_id)),
                field(4, OwnedValue::StableId128(relation.subject_stable_id)),
                field(5, OwnedValue::U8(relation.role)),
                field(8, OwnedValue::U32(relation.local_index)),
            ])
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;

    fn relation(role: u8, local_index: u32, subject: u8) -> RelationTuple {
        let mut owner_stable_id = [0_u8; 16];
        owner_stable_id[15] = 1;
        let mut subject_stable_id = [0_u8; 16];
        subject_stable_id[15] = subject;
        RelationTuple {
            owner_entity_kind: EntityKind::LaneEdge,
            owner_stable_id,
            role,
            local_index,
            subject_entity_kind: EntityKind::LaneEdge,
            subject_stable_id,
        }
    }

    #[test]
    fn set_pairing_ignores_canonical_position_only_changes() {
        let mut changes = Vec::new();
        pair_set_relations(&[relation(1, 1, 7)], &[relation(1, 0, 7)], &mut changes).unwrap();
        assert!(changes.is_empty());
    }

    #[test]
    fn scalar_pairing_emits_one_reconnect() {
        let mut changes = Vec::new();
        pair_scalar_relations(&[relation(20, 0, 7)], &[relation(20, 0, 8)], &mut changes).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_kind, 3);
        assert_eq!(
            changes[0].before_target,
            Some(relation(20, 0, 7).subject_stable_id)
        );
        assert_eq!(
            changes[0].after_target,
            Some(relation(20, 0, 8).subject_stable_id)
        );
    }

    #[test]
    fn occurrence_pairing_uses_same_subject_rank() {
        let mut changes = Vec::new();
        pair_domain_relations(
            &[relation(8, 0, 7), relation(8, 2, 7)],
            &[relation(8, 1, 7), relation(8, 2, 7)],
            &mut changes,
        );
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_kind, 2);
        assert_eq!(changes[0].before_local_index, Some(0));
        assert_eq!(changes[0].after_local_index, Some(1));
    }

    #[test]
    fn every_lfsd_relation_role_uses_the_frozen_pairing_family() {
        for role in [1, 6, 7, 9, 12, 17, 26] {
            assert_eq!(relation_pairing(role), Some(RelationPairing::Set));
        }
        for role in [20, 21, 22, 23, 24, 25, 27] {
            assert_eq!(relation_pairing(role), Some(RelationPairing::Scalar));
        }
        for role in [2, 3, 4, 5, 8, 10, 11, 18] {
            assert_eq!(
                relation_pairing(role),
                Some(RelationPairing::DomainOccurrence)
            );
        }
        for role in [0, 13, 14, 15, 16, 19, 28, 29, 30, u8::MAX] {
            assert_eq!(relation_pairing(role), None);
        }
    }

    #[test]
    fn set_pairing_rejects_duplicates_and_reports_only_membership_changes() {
        let mut changes = Vec::new();
        pair_set_relations(
            &[relation(1, 0, 7), relation(1, 1, 8)],
            &[relation(1, 0, 8), relation(1, 1, 9)],
            &mut changes,
        )
        .unwrap();
        assert_eq!(
            changes
                .iter()
                .map(|change| (change.change_kind, change.subject_stable_id))
                .collect::<Vec<_>>(),
            [
                (1, Some(relation(1, 0, 7).subject_stable_id)),
                (0, Some(relation(1, 0, 9).subject_stable_id)),
            ]
        );

        for (base, target, expected) in [
            (
                vec![relation(1, 0, 7), relation(1, 1, 7)],
                vec![],
                PortableEmissionError::DiffBaseSemanticMismatch,
            ),
            (
                vec![],
                vec![relation(1, 0, 7), relation(1, 1, 7)],
                PortableEmissionError::InternalBindingMismatch,
            ),
        ] {
            assert_eq!(
                pair_set_relations(&base, &target, &mut Vec::new()),
                Err(expected)
            );
        }
    }

    #[test]
    fn scalar_pairing_closes_absence_cardinality_and_reconnect() {
        for (base, target, expected_kind) in [
            (vec![], vec![relation(20, 0, 7)], Some(0)),
            (vec![relation(20, 0, 7)], vec![], Some(1)),
            (vec![relation(20, 0, 7)], vec![relation(20, 0, 7)], None),
            (vec![relation(20, 0, 7)], vec![relation(20, 0, 8)], Some(3)),
        ] {
            let mut changes = Vec::new();
            pair_scalar_relations(&base, &target, &mut changes).unwrap();
            assert_eq!(
                changes.first().map(|change| change.change_kind),
                expected_kind
            );
            assert_eq!(changes.len(), usize::from(expected_kind.is_some()));
        }
        assert_eq!(
            pair_scalar_relations(
                &[relation(20, 0, 7), relation(20, 1, 8)],
                &[],
                &mut Vec::new()
            ),
            Err(PortableEmissionError::DiffBaseSemanticMismatch)
        );
        assert_eq!(
            pair_scalar_relations(
                &[],
                &[relation(20, 0, 7), relation(20, 1, 8)],
                &mut Vec::new()
            ),
            Err(PortableEmissionError::InternalBindingMismatch)
        );
    }

    #[test]
    fn domain_occurrence_pairing_closes_moves_and_unpaired_ranks() {
        let mut changes = Vec::new();
        pair_domain_relations(
            &[relation(13, 0, 7), relation(13, 1, 8), relation(13, 2, 7)],
            &[relation(13, 0, 8), relation(13, 1, 7), relation(13, 2, 8)],
            &mut changes,
        );
        assert_eq!(changes.len(), 4);
        assert_eq!(
            changes
                .iter()
                .map(|change| (
                    change.change_kind,
                    change.before_local_index,
                    change.after_local_index,
                    change.subject_stable_id,
                ))
                .collect::<Vec<_>>(),
            [
                (
                    2,
                    Some(0),
                    Some(1),
                    Some(relation(13, 0, 7).subject_stable_id),
                ),
                (1, Some(2), None, Some(relation(13, 0, 7).subject_stable_id),),
                (
                    2,
                    Some(1),
                    Some(0),
                    Some(relation(13, 0, 8).subject_stable_id),
                ),
                (0, None, Some(2), Some(relation(13, 0, 8).subject_stable_id),),
            ]
        );
    }

    #[test]
    fn relation_groups_reject_non_contiguous_local_indexes_before_pairing() {
        assert!(
            group_relations(
                vec![relation(13, 0, 7)],
                PortableEmissionError::DiffBaseSemanticMismatch
            )
            .is_ok()
        );
        assert_eq!(
            group_relations(
                vec![relation(13, 1, 7)],
                PortableEmissionError::DiffBaseSemanticMismatch,
            )
            .unwrap_err(),
            PortableEmissionError::DiffBaseSemanticMismatch
        );
    }
}
