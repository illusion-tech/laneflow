use super::base::{ArtifactIndex, checked_u32_with, singleton_row};
use super::entity::checked_u32;
use super::*;

fn artifact_geometry_values(
    index: &ArtifactIndex<'_>,
    mismatch: PortableEmissionError,
) -> Result<GeometryValues, PortableEmissionError> {
    let spatial = index.view.section(4).ok_or(mismatch)?;
    let mut geometries = BTreeMap::new();
    for (table_ordinal, subject_kind, projected_tags) in [
        (1, EntityKind::LaneEdge, &[3_u16, 4, 5, 6][..]),
        (2, EntityKind::FacilityBand, &[3_u16, 4][..]),
    ] {
        let table = spatial.table(table_ordinal).ok_or(mismatch)?;
        for ordinal in 0..table.row_count() {
            let geometry = table.row(ordinal).ok_or(mismatch)?;
            let subject_stable_id = index.stable_id(
                subject_kind,
                checked_u32_with(geometry, 1, mismatch)?,
                mismatch,
            )?;
            let frame_stable_id = index.stable_id(
                EntityKind::CanonicalFrame,
                checked_u32_with(geometry, 2, mismatch)?,
                mismatch,
            )?;
            if geometries
                .insert(
                    (subject_kind, subject_stable_id),
                    canonical_geometry_value(geometry, frame_stable_id, projected_tags)?,
                )
                .is_some()
            {
                return Err(mismatch);
            }
        }
    }
    Ok(geometries)
}

pub(super) fn artifact_geometry_changes(
    base: &ArtifactIndex<'_>,
    target: &ArtifactIndex<'_>,
) -> Result<Vec<OwnedRow>, PortableEmissionError> {
    let base_values =
        artifact_geometry_values(base, PortableEmissionError::DiffBaseSemanticMismatch)?;
    let target_values =
        artifact_geometry_values(target, PortableEmissionError::InternalBindingMismatch)?;
    let mut changes = Vec::<(u8, EntityKind, [u8; 16], OwnedRow)>::new();
    for ((entity_kind, stable_id), after) in &target_values {
        match base_values.get(&(*entity_kind, *stable_id)) {
            None => changes.push((
                0,
                *entity_kind,
                *stable_id,
                row([
                    field(1, OwnedValue::U8(0)),
                    field(2, OwnedValue::U16(entity_kind.code())),
                    field(4, OwnedValue::StableId128(*stable_id)),
                    field(10, OwnedValue::Bytes(after.clone())),
                ]),
            )),
            Some(before) if before != after => changes.push((
                2,
                *entity_kind,
                *stable_id,
                row([
                    field(1, OwnedValue::U8(2)),
                    field(2, OwnedValue::U16(entity_kind.code())),
                    field(4, OwnedValue::StableId128(*stable_id)),
                    field(9, OwnedValue::Bytes(before.clone())),
                    field(10, OwnedValue::Bytes(after.clone())),
                ]),
            )),
            Some(_) => {}
        }
    }
    for ((entity_kind, stable_id), before) in &base_values {
        if !target_values.contains_key(&(*entity_kind, *stable_id)) {
            changes.push((
                1,
                *entity_kind,
                *stable_id,
                row([
                    field(1, OwnedValue::U8(1)),
                    field(2, OwnedValue::U16(entity_kind.code())),
                    field(4, OwnedValue::StableId128(*stable_id)),
                    field(9, OwnedValue::Bytes(before.clone())),
                ]),
            ));
        }
    }
    changes.sort_unstable_by_key(|(change_kind, entity_kind, stable_id, _)| {
        (*change_kind, *entity_kind, *stable_id)
    });
    Ok(changes.into_iter().map(|(_, _, _, row)| row).collect())
}

pub(super) fn artifact_spatial_configuration_changes(
    base: &ArtifactIndex<'_>,
    target: &ArtifactIndex<'_>,
) -> Result<Vec<OwnedRow>, PortableEmissionError> {
    let before = singleton_row(
        base.view,
        4,
        PortableEmissionError::DiffBaseSemanticMismatch,
    )?;
    let after = singleton_row(
        target.view,
        4,
        PortableEmissionError::InternalBindingMismatch,
    )?;
    if before.bytes() == after.bytes() {
        return Ok(Vec::new());
    }
    Ok(vec![row([
        field(1, OwnedValue::U8(1)),
        field(
            2,
            OwnedValue::Bytes(before.bytes().to_vec().into_boxed_slice()),
        ),
        field(
            3,
            OwnedValue::Bytes(after.bytes().to_vec().into_boxed_slice()),
        ),
    ])])
}

fn canonical_geometry_value(
    row: RegistryCheckedRowView<'_>,
    frame_stable_id: [u8; 16],
    projected_tags: &[u16],
) -> Result<Box<[u8]>, PortableEmissionError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&EntityKind::CanonicalFrame.code().to_le_bytes());
    bytes.extend_from_slice(&frame_stable_id);
    bytes.extend_from_slice(
        &u16::try_from(projected_tags.len())
            .map_err(|_| PortableEmissionError::ArithmeticOverflow)?
            .to_le_bytes(),
    );
    for tag in projected_tags {
        let value = row
            .field_by_tag(*tag)
            .ok_or(PortableEmissionError::InternalBindingMismatch)?
            .value_bytes();
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(
            &u32::try_from(value.len())
                .map_err(|_| PortableEmissionError::ArithmeticOverflow)?
                .to_le_bytes(),
        );
        bytes.extend_from_slice(value);
    }
    Ok(bytes.into_boxed_slice())
}

pub(super) fn genesis_geometry_changes(
    lir: &crate::lir::LirUnit,
    target: RegistryCheckedObjectView<'_>,
) -> Result<Vec<OwnedRow>, PortableEmissionError> {
    let section = target
        .section(4)
        .ok_or(PortableEmissionError::InternalBindingMismatch)?;
    let mut changes = Vec::new();
    for (table_index, (subject_kind, projected_tags)) in [
        (EntityKind::LaneEdge, &[3_u16, 4, 5, 6][..]),
        (EntityKind::FacilityBand, &[3_u16, 4][..]),
    ]
    .into_iter()
    .enumerate()
    {
        let table = section
            .table(
                u32::try_from(table_index + 1)
                    .expect("the canonical spatial registry contains only three tables"),
            )
            .ok_or(PortableEmissionError::InternalBindingMismatch)?;
        for row_index in 0..table.row_count() {
            let geometry = table
                .row(row_index)
                .ok_or(PortableEmissionError::InternalBindingMismatch)?;
            let subject_ordinal = checked_u32(geometry, 1)?;
            let frame_ordinal = checked_u32(geometry, 2)?;
            changes.push((
                subject_kind,
                entity_stable_id(lir, subject_kind, subject_ordinal),
                canonical_geometry_value(
                    geometry,
                    entity_stable_id(lir, EntityKind::CanonicalFrame, frame_ordinal),
                    projected_tags,
                )?,
            ));
        }
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
