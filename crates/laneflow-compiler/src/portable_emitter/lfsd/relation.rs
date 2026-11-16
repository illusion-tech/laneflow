use super::base::{
    ArtifactIndex, checked_ordinal_vector_with, checked_record_vector_with, checked_u8_with,
    checked_u32_with,
};
use super::*;
use crate::ledger::{BoundedVec, PassBudget, Plan};

#[allow(
    clippy::too_many_arguments,
    reason = "the closed A.5 tuple is clearer when every semantic coordinate is explicit"
)]
fn push_artifact_relation(
    relations: &mut dyn FnMut(RelationTuple) -> Result<(), PortableEmissionError>,
    index: &ArtifactIndex<'_>,
    owner_entity_kind: EntityKind,
    owner_stable_id: [u8; 16],
    role: u8,
    local_index: u32,
    subject_entity_kind: EntityKind,
    subject_ordinal: u32,
    mismatch: PortableEmissionError,
) -> Result<(), PortableEmissionError> {
    relations(RelationTuple {
        owner_entity_kind,
        owner_stable_id,
        role,
        local_index,
        subject_entity_kind,
        subject_stable_id: index.stable_id(subject_entity_kind, subject_ordinal, mismatch)?,
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "the closed A.5 vector projection keeps owner, field, role, and subject kind explicit"
)]
fn push_vector_relations(
    relations: &mut dyn FnMut(RelationTuple) -> Result<(), PortableEmissionError>,
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

/// 路口成员关系的待排序记录：（路口序号, 成员序号）。
#[derive(Clone, Copy)]
enum Membership {
    ConflictZone(u32, u32),
    ParticipantStream(u32, u32),
}

/// 逐实体按冻结角色展开所有者局部关系字段，主体的有类型序号经索引解析为稳定标识；
/// 路口的冲突区与参与者流成员关系只记录成员序号，由调用方按路口排序后展开。
fn walk_owner_relations(
    index: &ArtifactIndex<'_>,
    mismatch: PortableEmissionError,
    relations: &mut dyn FnMut(RelationTuple) -> Result<(), PortableEmissionError>,
    memberships: &mut dyn FnMut(Membership) -> Result<(), PortableEmissionError>,
) -> Result<(), PortableEmissionError> {
    for ((owner_kind, owner_stable_id), owner) in index.entities() {
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
                        relations,
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
                relations,
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
                relations,
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
                relations,
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
                relations,
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
                relations,
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
                        relations,
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
                        relations,
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
                relations,
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
                        relations,
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
                if let Some(parking_facility) = owner.row.field_by_tag(3) {
                    let ordinal = match parking_facility.value()? {
                        RegistryCheckedFieldValue::U32(value) => value,
                        _ => return Err(mismatch),
                    };
                    push_artifact_relation(
                        relations,
                        index,
                        *owner_kind,
                        *owner_stable_id,
                        21,
                        0,
                        EntityKind::ParkingFacility,
                        ordinal,
                        mismatch,
                    )?;
                }
                for (field_tag, role) in [(4, 22), (6, 23)] {
                    push_artifact_relation(
                        relations,
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
                relations,
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
                        relations,
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
                    relations,
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
                    relations,
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
                relations,
                index,
                *owner_kind,
                *owner_stable_id,
                27,
                0,
                EntityKind::ParticipantClass,
                checked_u32_with(owner.row, 3, mismatch)?,
                mismatch,
            )?,
            EntityKind::ParkingFacility => {
                for (field_tag, role) in [(5, 13), (6, 14)] {
                    let anchors = checked_record_vector_with(owner.row, field_tag, mismatch)?;
                    for (local_index, anchor) in anchors.rows().enumerate() {
                        push_artifact_relation(
                            relations,
                            index,
                            *owner_kind,
                            *owner_stable_id,
                            role,
                            u32::try_from(local_index)
                                .map_err(|_| PortableEmissionError::ArithmeticOverflow)?,
                            EntityKind::LaneEdge,
                            checked_u32_with(anchor, 1, mismatch)?,
                            mismatch,
                        )?;
                    }
                }
            }
            EntityKind::ParticipantStream => {
                let stream_ordinal = checked_u32_with(owner.row, 1, mismatch)?;
                let junction_ordinal = checked_u32_with(owner.row, 3, mismatch)?;
                memberships(Membership::ParticipantStream(
                    junction_ordinal,
                    stream_ordinal,
                ))?;
                push_artifact_relation(
                    relations,
                    index,
                    *owner_kind,
                    *owner_stable_id,
                    30,
                    0,
                    EntityKind::ManeuverPath,
                    checked_u32_with(owner.row, 4, mismatch)?,
                    mismatch,
                )?;
                let passages = checked_record_vector_with(owner.row, 5, mismatch)?;
                for (local_index, passage) in passages.rows().enumerate() {
                    push_artifact_relation(
                        relations,
                        index,
                        *owner_kind,
                        *owner_stable_id,
                        31,
                        u32::try_from(local_index)
                            .map_err(|_| PortableEmissionError::ArithmeticOverflow)?,
                        EntityKind::ConflictZone,
                        checked_u32_with(passage, 1, mismatch)?,
                        mismatch,
                    )?;
                }
            }
            EntityKind::ConflictZone => memberships(Membership::ConflictZone(
                checked_u32_with(owner.row, 3, mismatch)?,
                checked_u32_with(owner.row, 1, mismatch)?,
            ))?,
            EntityKind::WaitingZone
            | EntityKind::SignalGroup
            | EntityKind::SignalPhase
            | EntityKind::FacilityBand
            | EntityKind::RightOfWayPolicySet
            | EntityKind::CanonicalFrame => {}
        }
    }
    Ok(())
}

/// 已索引制品的所有者局部关系元组数量，即 [`artifact_relation_tuples`] 的容量。
pub(super) fn artifact_relation_count(
    index: &ArtifactIndex<'_>,
    mismatch: PortableEmissionError,
) -> Result<usize, PortableEmissionError> {
    let mut count = 0_usize;
    walk_owner_relations(
        index,
        mismatch,
        &mut |_| {
            count += 1;
            Ok(())
        },
        &mut |_| Ok(()),
    )?;
    let internal_edges = index
        .view
        .section(3)
        .and_then(|section| section.table(0))
        .ok_or(mismatch)?
        .row_count() as usize;
    [
        index.kind_count(EntityKind::ConflictZone),
        index.kind_count(EntityKind::ParticipantStream),
        internal_edges,
    ]
    .into_iter()
    .try_fold(count, |total, value| {
        total
            .checked_add(value)
            .ok_or(PortableEmissionError::ArithmeticOverflow)
    })
}

/// [`artifact_relation_tuples`] 的计划峰值：元组表，以及成员关系排序表与路口内部边局部下标
/// 计数器。
pub(super) fn artifact_relation_plan(
    index: &ArtifactIndex<'_>,
    mismatch: PortableEmissionError,
) -> Result<Plan, PortableEmissionError> {
    Ok(Plan::ZERO
        .array::<RelationTuple>(artifact_relation_count(index, mismatch)?)
        .array::<(u32, u32)>(index.kind_count(EntityKind::ConflictZone))
        .array::<(u32, u32)>(index.kind_count(EntityKind::ParticipantStream))
        .array::<u32>(index.kind_count(EntityKind::Junction)))
}

/// 把已索引制品的全部所有者局部关系投影为按规范顺序排序的关系元组，存入编译遍授予的
/// 序列；计划见 [`artifact_relation_plan`]。
pub(super) fn artifact_relation_tuples(
    index: &ArtifactIndex<'_>,
    mismatch: PortableEmissionError,
    pass: &PassBudget<'_>,
) -> Result<BoundedVec<RelationTuple>, PortableEmissionError> {
    let mut relations = pass.vec::<RelationTuple>(artifact_relation_count(index, mismatch)?)?;
    let mut conflict_zone_memberships =
        pass.vec::<(u32, u32)>(index.kind_count(EntityKind::ConflictZone))?;
    let mut participant_stream_memberships =
        pass.vec::<(u32, u32)>(index.kind_count(EntityKind::ParticipantStream))?;
    walk_owner_relations(
        index,
        mismatch,
        &mut |relation| Ok(relations.push(relation)?),
        &mut |membership| {
            match membership {
                Membership::ConflictZone(junction, zone) => {
                    conflict_zone_memberships.push((junction, zone))?;
                }
                Membership::ParticipantStream(junction, stream) => {
                    participant_stream_memberships.push((junction, stream))?;
                }
            }
            Ok(())
        },
    )?;
    let relations_sink = &mut |relation| Ok(relations.push(relation)?);
    for (role, subject_kind, memberships) in [
        (15, EntityKind::ConflictZone, &mut conflict_zone_memberships),
        (
            16,
            EntityKind::ParticipantStream,
            &mut participant_stream_memberships,
        ),
    ] {
        memberships.sort_unstable();
        let mut previous_junction = None;
        let mut local_index = 0_u32;
        for &(junction_ordinal, subject_ordinal) in memberships.iter() {
            if previous_junction != Some(junction_ordinal) {
                previous_junction = Some(junction_ordinal);
                local_index = 0;
            }
            let owner_stable_id =
                index.stable_id(EntityKind::Junction, junction_ordinal, mismatch)?;
            push_artifact_relation(
                relations_sink,
                index,
                EntityKind::Junction,
                owner_stable_id,
                role,
                local_index,
                subject_kind,
                subject_ordinal,
                mismatch,
            )?;
            local_index = local_index
                .checked_add(1)
                .ok_or(PortableEmissionError::ArithmeticOverflow)?;
        }
    }
    drop((conflict_zone_memberships, participant_stream_memberships));

    let relation_section = index.view.section(3).ok_or(mismatch)?;
    let internal_edges = relation_section.table(0).ok_or(mismatch)?;
    let junctions = index.kind_count(EntityKind::Junction);
    let mut next_internal_index = pass.vec::<u32>(junctions)?;
    next_internal_index.resize(junctions, 0)?;
    for relation in internal_edges.rows() {
        let junction_ordinal = checked_u32_with(relation, 2, mismatch)?;
        let owner_stable_id = index.stable_id(EntityKind::Junction, junction_ordinal, mismatch)?;
        let local_index = next_internal_index
            .get_mut(junction_ordinal as usize)
            .ok_or(mismatch)?;
        let current = *local_index;
        *local_index = current
            .checked_add(1)
            .ok_or(PortableEmissionError::ArithmeticOverflow)?;
        push_artifact_relation(
            relations_sink,
            index,
            EntityKind::Junction,
            owner_stable_id,
            9,
            current,
            EntityKind::LaneEdge,
            checked_u32_with(relation, 1, mismatch)?,
            mismatch,
        )?;
    }
    drop(next_internal_index);
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
        1 | 6 | 7 | 9 | 12 | 15 | 16 | 17 | 26 => Some(RelationPairing::Set),
        20..=25 | 27 | 30 => Some(RelationPairing::Scalar),
        2..=5 | 8 | 10 | 11 | 13 | 14 | 18 | 31 => Some(RelationPairing::DomainOccurrence),
        _ => None,
    }
}

/// 一条关系变化（LFSD 第 3 节行）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RelationChange {
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

/// 关系变化的接收端；写入超出授予容量时返回记账缺陷。
type ChangeSink<'s> = &'s mut dyn FnMut(RelationChange) -> Result<(), PortableEmissionError>;

fn relation_add(tuple: RelationTuple) -> RelationChange {
    RelationChange {
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

fn relation_remove(tuple: RelationTuple) -> RelationChange {
    RelationChange {
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

fn relation_move(before: RelationTuple, after: RelationTuple) -> RelationChange {
    RelationChange {
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

fn relation_reconnect(before: RelationTuple, after: RelationTuple) -> RelationChange {
    RelationChange {
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

fn compare_relation_changes(left: &RelationChange, right: &RelationChange) -> std::cmp::Ordering {
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

const fn group_key(relation: &RelationTuple) -> (EntityKind, [u8; 16], u8) {
    (
        relation.owner_entity_kind,
        relation.owner_stable_id,
        relation.role,
    )
}

const fn subject_key(relation: &RelationTuple) -> (EntityKind, [u8; 16]) {
    (relation.subject_entity_kind, relation.subject_stable_id)
}

/// 一组关系（同一所有者与角色）的局部下标必须恰为 `0..n`；元组表已按规范顺序排序。
fn check_group_local_indexes(
    group: &[RelationTuple],
    mismatch: PortableEmissionError,
) -> Result<(), PortableEmissionError> {
    for (expected, relation) in group.iter().enumerate() {
        if relation.local_index
            != u32::try_from(expected).map_err(|_| PortableEmissionError::ArithmeticOverflow)?
        {
            return Err(mismatch);
        }
    }
    Ok(())
}

/// 规范顺序元组表中下一组（同一所有者与角色）的长度。
fn group_len(relations: &[RelationTuple]) -> usize {
    relations.first().map_or(0, |first| {
        relations.partition_point(|relation| group_key(relation) <= group_key(first))
    })
}

fn pair_set_relations(
    base: &mut [RelationTuple],
    target: &mut [RelationTuple],
    changes: ChangeSink<'_>,
) -> Result<(), PortableEmissionError> {
    for (relations, mismatch) in [
        (&mut *base, PortableEmissionError::DiffBaseSemanticMismatch),
        (&mut *target, PortableEmissionError::InternalBindingMismatch),
    ] {
        relations.sort_unstable_by_key(subject_key);
        if relations
            .windows(2)
            .any(|pair| subject_key(&pair[0]) == subject_key(&pair[1]))
        {
            return Err(mismatch);
        }
    }
    let contains = |relations: &[RelationTuple], relation: &RelationTuple| {
        relations
            .binary_search_by_key(&subject_key(relation), subject_key)
            .is_ok()
    };
    for relation in base.iter() {
        if !contains(target, relation) {
            changes(relation_remove(*relation))?;
        }
    }
    for relation in target.iter() {
        if !contains(base, relation) {
            changes(relation_add(*relation))?;
        }
    }
    Ok(())
}

fn pair_scalar_relations(
    base: &[RelationTuple],
    target: &[RelationTuple],
    changes: ChangeSink<'_>,
) -> Result<(), PortableEmissionError> {
    if base.len() > 1 {
        return Err(PortableEmissionError::DiffBaseSemanticMismatch);
    }
    if target.len() > 1 {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    match (base.first().copied(), target.first().copied()) {
        (None, None) => {}
        (None, Some(after)) => changes(relation_add(after))?,
        (Some(before), None) => changes(relation_remove(before))?,
        (Some(before), Some(after)) if subject_key(&before) != subject_key(&after) => {
            changes(relation_reconnect(before, after))?;
        }
        (Some(_), Some(_)) => {}
    }
    Ok(())
}

fn pair_domain_relations(
    base: &mut [RelationTuple],
    target: &mut [RelationTuple],
    changes: ChangeSink<'_>,
) -> Result<(), PortableEmissionError> {
    let occurrence_key = |relation: &RelationTuple| (subject_key(relation), relation.local_index);
    base.sort_unstable_by_key(occurrence_key);
    target.sort_unstable_by_key(occurrence_key);
    let run = |relations: &[RelationTuple], subject| {
        relations.partition_point(|relation| subject_key(relation) <= subject)
    };
    let (mut base, mut target) = (&*base, &*target);
    while let Some(subject) = match (base.first(), target.first()) {
        (Some(b), Some(a)) => Some(subject_key(b).min(subject_key(a))),
        (Some(b), None) => Some(subject_key(b)),
        (None, Some(a)) => Some(subject_key(a)),
        (None, None) => None,
    } {
        let (before, rest) = base.split_at(run(base, subject));
        base = rest;
        let (after, rest) = target.split_at(run(target, subject));
        target = rest;
        let paired_count = before.len().min(after.len());
        for rank in 0..paired_count {
            if before[rank].local_index != after[rank].local_index {
                changes(relation_move(before[rank], after[rank]))?;
            }
        }
        for relation in &before[paired_count..] {
            changes(relation_remove(*relation))?;
        }
        for relation in &after[paired_count..] {
            changes(relation_add(*relation))?;
        }
    }
    Ok(())
}

/// 写出一条关系变化行。
pub(super) fn relation_change_row(
    t: &super::super::tree::TreeBuilder,
    change: RelationChange,
) -> super::super::tree::RowRef {
    use super::super::tree::{Value, field};
    t.row([
        Some(field(1, Value::U8(change.change_kind))),
        Some(field(2, Value::U16(change.owner_entity_kind.code()))),
        Some(field(3, Value::StableId128(change.owner_stable_id))),
        change
            .subject_stable_id
            .map(|subject| field(4, Value::StableId128(subject))),
        Some(field(5, Value::U8(change.role))),
        change
            .before_local_index
            .map(|index| field(7, Value::U32(index))),
        change
            .after_local_index
            .map(|index| field(8, Value::U32(index))),
        change
            .before_target
            .map(|target| field(9, Value::StableId128(target))),
        change
            .after_target
            .map(|target| field(10, Value::StableId128(target))),
    ])
}

/// 写入两版制品之间的关系变化（LFSD 第 3 节）。
///
/// 两侧元组表已按规范顺序排序，同一所有者与角色的元组相邻成组；逐组依各角色冻结的
/// 配对方式配对后输出增删、移动与重连变更，并按规范顺序排序。组内配对会就地重排
/// 元组表。变化至多为两侧元组数之和。
pub(super) fn artifact_relation_changes(
    base: &mut [RelationTuple],
    target: &mut [RelationTuple],
    changes: &mut BoundedVec<RelationChange>,
) -> Result<(), PortableEmissionError> {
    let (mut base, mut target) = (base, target);
    let sink: ChangeSink<'_> = &mut |change| Ok(changes.push(change)?);
    while !base.is_empty() || !target.is_empty() {
        let key = match (base.first(), target.first()) {
            (Some(b), Some(a)) => group_key(b).min(group_key(a)),
            (Some(b), None) => group_key(b),
            (None, Some(a)) => group_key(a),
            (None, None) => break,
        };
        let base_len = if base.first().is_some_and(|b| group_key(b) == key) {
            group_len(base)
        } else {
            0
        };
        let target_len = if target.first().is_some_and(|a| group_key(a) == key) {
            group_len(target)
        } else {
            0
        };
        let (base_group, base_rest) = core::mem::take(&mut base).split_at_mut(base_len);
        let (target_group, target_rest) = core::mem::take(&mut target).split_at_mut(target_len);
        base = base_rest;
        target = target_rest;
        check_group_local_indexes(base_group, PortableEmissionError::DiffBaseSemanticMismatch)?;
        check_group_local_indexes(target_group, PortableEmissionError::InternalBindingMismatch)?;
        match relation_pairing(key.2).ok_or(PortableEmissionError::InternalBindingMismatch)? {
            RelationPairing::Set => pair_set_relations(base_group, target_group, sink)?,
            RelationPairing::Scalar => pair_scalar_relations(base_group, target_group, sink)?,
            RelationPairing::DomainOccurrence => {
                pair_domain_relations(base_group, target_group, sink)?;
            }
        }
    }
    changes.sort_unstable_by(compare_relation_changes);
    Ok(())
}

/// 把 LIR 的规范关系元组全部写为 Genesis 新增变化；容量为规范关系元组数，路口局部序号
/// 暂存由 `pass` 授予。
pub(super) fn genesis_relation_changes(
    lir: &crate::lir::LirUnit,
    pass: &crate::ledger::PassBudget<'_>,
    changes: &mut BoundedVec<RelationChange>,
) -> Result<(), PortableEmissionError> {
    let mut overflow = None;
    super::super::relations::for_each_canonical_relation_tuple(lir, pass, &mut |relation| {
        if let Err(error) = changes.push(relation_add(relation)) {
            overflow.get_or_insert(error);
        }
    })?;
    if let Some(error) = overflow {
        return Err(error.into());
    }
    changes.sort_unstable_by(compare_relation_changes);
    Ok(())
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

    /// 收集一次配对写出的关系变化。
    fn collect(
        pair: impl FnOnce(ChangeSink<'_>) -> Result<(), PortableEmissionError>,
    ) -> Result<Vec<RelationChange>, PortableEmissionError> {
        let mut changes = Vec::new();
        pair(&mut |change| {
            changes.push(change);
            Ok(())
        })?;
        Ok(changes)
    }

    #[test]
    fn set_pairing_ignores_canonical_position_only_changes() {
        let changes = collect(|sink| {
            pair_set_relations(&mut [relation(1, 1, 7)], &mut [relation(1, 0, 7)], sink)
        })
        .unwrap();
        assert!(changes.is_empty());
    }

    #[test]
    fn scalar_pairing_emits_one_reconnect() {
        let changes = collect(|sink| {
            pair_scalar_relations(&[relation(20, 0, 7)], &[relation(20, 0, 8)], sink)
        })
        .unwrap();
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
        let changes = collect(|sink| {
            pair_domain_relations(
                &mut [relation(8, 0, 7), relation(8, 2, 7)],
                &mut [relation(8, 1, 7), relation(8, 2, 7)],
                sink,
            )
        })
        .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_kind, 2);
        assert_eq!(changes[0].before_local_index, Some(0));
        assert_eq!(changes[0].after_local_index, Some(1));
    }

    #[test]
    fn every_lfsd_relation_role_uses_the_frozen_pairing_family() {
        for role in [1, 6, 7, 9, 12, 15, 16, 17, 26] {
            assert_eq!(relation_pairing(role), Some(RelationPairing::Set));
        }
        for role in [20, 21, 22, 23, 24, 25, 27, 30] {
            assert_eq!(relation_pairing(role), Some(RelationPairing::Scalar));
        }
        for role in [2, 3, 4, 5, 8, 10, 11, 13, 14, 18, 31] {
            assert_eq!(
                relation_pairing(role),
                Some(RelationPairing::DomainOccurrence)
            );
        }
        for role in [0, 19, 28, 29, 32, u8::MAX] {
            assert_eq!(relation_pairing(role), None);
        }
    }

    #[test]
    fn set_pairing_rejects_duplicates_and_reports_only_membership_changes() {
        let changes = collect(|sink| {
            pair_set_relations(
                &mut [relation(1, 0, 7), relation(1, 1, 8)],
                &mut [relation(1, 0, 8), relation(1, 1, 9)],
                sink,
            )
        })
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

        for (mut base, mut target, expected) in [
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
                collect(|sink| pair_set_relations(&mut base, &mut target, sink)),
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
            let changes = collect(|sink| pair_scalar_relations(&base, &target, sink)).unwrap();
            assert_eq!(
                changes.first().map(|change| change.change_kind),
                expected_kind
            );
            assert_eq!(changes.len(), usize::from(expected_kind.is_some()));
        }
        assert_eq!(
            collect(|sink| pair_scalar_relations(
                &[relation(20, 0, 7), relation(20, 1, 8)],
                &[],
                sink
            )),
            Err(PortableEmissionError::DiffBaseSemanticMismatch)
        );
        assert_eq!(
            collect(|sink| pair_scalar_relations(
                &[],
                &[relation(20, 0, 7), relation(20, 1, 8)],
                sink
            )),
            Err(PortableEmissionError::InternalBindingMismatch)
        );
    }

    #[test]
    fn domain_occurrence_pairing_closes_moves_and_unpaired_ranks() {
        let changes = collect(|sink| {
            pair_domain_relations(
                &mut [relation(13, 0, 7), relation(13, 1, 8), relation(13, 2, 7)],
                &mut [relation(13, 0, 8), relation(13, 1, 7), relation(13, 2, 8)],
                sink,
            )
        })
        .unwrap();
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
            check_group_local_indexes(
                &[relation(13, 0, 7)],
                PortableEmissionError::DiffBaseSemanticMismatch
            )
            .is_ok()
        );
        assert_eq!(
            check_group_local_indexes(
                &[relation(13, 1, 7)],
                PortableEmissionError::DiffBaseSemanticMismatch,
            )
            .unwrap_err(),
            PortableEmissionError::DiffBaseSemanticMismatch
        );
        // 两侧各一组时，组内局部下标不连续的一侧在配对前被拒绝。
        let mut changes = crate::portable_emitter::budget::test_pass().vec(4).unwrap();
        assert_eq!(
            artifact_relation_changes(
                &mut [relation(13, 0, 7)],
                &mut [relation(13, 1, 7)],
                &mut changes,
            ),
            Err(PortableEmissionError::InternalBindingMismatch)
        );
    }
}
