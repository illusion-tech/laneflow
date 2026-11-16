use super::base::{ArtifactIndex, checked_u32_with, singleton_row};
use super::changes::{ChangeBounds, FieldChange, Payload, SpatialChange, add, stage};
use super::entity::checked_u32;
use super::*;
use crate::ledger::{BoundedVec, PassBudget, Plan};

/// 空间节三张几何表：表序号、主体实体种类与投影字段。
const GEOMETRY_TABLES: [(u32, EntityKind, &[u16]); 3] = [
    (1, EntityKind::LaneEdge, &[3, 4, 5, 6]),
    (2, EntityKind::FacilityBand, &[3, 4]),
    (3, EntityKind::ConflictZone, &[3, 4, 5]),
];

/// 一侧几何投影值：主体实体种类、主体稳定标识与暂存字节区中的规范值。
type GeometryValue = (EntityKind, [u8; 16], Slots);

/// 规范几何值的精确字节数：坐标系稳定引用、投影字段数与逐字段（标签、长度、值）。
fn canonical_geometry_len(
    row: RegistryCheckedRowView<'_>,
    projected_tags: &[u16],
) -> Result<usize, PortableEmissionError> {
    projected_tags
        .iter()
        .try_fold(2 + 16 + 2, |length: usize, tag| {
            let value = row
                .field_by_tag(*tag)
                .ok_or(PortableEmissionError::InternalBindingMismatch)?
                .value_bytes();
            length
                .checked_add(2 + 4)
                .and_then(|length| length.checked_add(value.len()))
                .ok_or(PortableEmissionError::ArithmeticOverflow)
        })
}

fn push_canonical_geometry_value(
    staged: &mut BoundedVec<u8>,
    row: RegistryCheckedRowView<'_>,
    frame_stable_id: [u8; 16],
    projected_tags: &[u16],
) -> Result<Slots, PortableEmissionError> {
    stage(staged, |staged| {
        staged.extend_from_slice(&EntityKind::CanonicalFrame.code().to_le_bytes())?;
        staged.extend_from_slice(&frame_stable_id)?;
        staged.extend_from_slice(
            &u16::try_from(projected_tags.len())
                .map_err(|_| PortableEmissionError::ArithmeticOverflow)?
                .to_le_bytes(),
        )?;
        for tag in projected_tags {
            let value = row
                .field_by_tag(*tag)
                .ok_or(PortableEmissionError::InternalBindingMismatch)?
                .value_bytes();
            staged.extend_from_slice(&tag.to_le_bytes())?;
            staged.extend_from_slice(
                &u32::try_from(value.len())
                    .map_err(|_| PortableEmissionError::ArithmeticOverflow)?
                    .to_le_bytes(),
            )?;
            staged.extend_from_slice(value)?;
        }
        Ok(())
    })
}

/// 一侧制品的几何行数与规范几何值字节数之和。
fn geometry_side_bound(
    view: RegistryCheckedObjectView<'_>,
    mismatch: PortableEmissionError,
) -> Result<(usize, usize), PortableEmissionError> {
    let spatial = view.section(4).ok_or(mismatch)?;
    let (mut rows, mut bytes) = (0, 0);
    for (table_ordinal, _, projected_tags) in GEOMETRY_TABLES {
        for geometry in spatial.table(table_ordinal).ok_or(mismatch)?.rows() {
            add(&mut rows, 1)?;
            add(
                &mut bytes,
                canonical_geometry_len(geometry, projected_tags)?,
            )?;
        }
    }
    Ok((rows, bytes))
}

/// 几何差异（第 4 节）的容量与暂存字节上界，以及两侧几何值表的计划。
///
/// 变化至多为两侧几何行数之和；两侧规范几何值全部暂存，修改行的前后值引用它们。
pub(super) fn geometry_change_bounds(
    base: Option<RegistryCheckedObjectView<'_>>,
    target: RegistryCheckedObjectView<'_>,
    bounds: &mut ChangeBounds,
) -> Result<Plan, PortableEmissionError> {
    let (target_rows, target_bytes) =
        geometry_side_bound(target, PortableEmissionError::InternalBindingMismatch)?;
    let (base_rows, base_bytes) = match base {
        Some(base) => geometry_side_bound(base, PortableEmissionError::DiffBaseSemanticMismatch)?,
        None => (0, 0),
    };
    add(&mut bounds.geometry, target_rows)?;
    add(&mut bounds.geometry, base_rows)?;
    add(&mut bounds.staged, target_bytes)?;
    add(&mut bounds.staged, base_bytes)?;
    Ok(Plan::ZERO
        .array::<GeometryValue>(target_rows)
        .array::<GeometryValue>(base_rows))
}

fn artifact_geometry_values(
    index: &ArtifactIndex<'_>,
    mismatch: PortableEmissionError,
    pass: &PassBudget<'_>,
    staged: &mut BoundedVec<u8>,
) -> Result<BoundedVec<GeometryValue>, PortableEmissionError> {
    let spatial = index.view.section(4).ok_or(mismatch)?;
    let (rows, _) = geometry_side_bound(index.view, mismatch)?;
    let mut geometries = pass.vec::<GeometryValue>(rows)?;
    for (table_ordinal, subject_kind, projected_tags) in GEOMETRY_TABLES {
        let table = spatial.table(table_ordinal).ok_or(mismatch)?;
        for geometry in table.rows() {
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
            geometries.push((
                subject_kind,
                subject_stable_id,
                push_canonical_geometry_value(staged, geometry, frame_stable_id, projected_tags)?,
            ))?;
        }
    }
    geometries.sort_unstable_by_key(|(kind, stable_id, _)| (*kind, *stable_id));
    if geometries
        .windows(2)
        .any(|pair| (pair[0].0, pair[0].1) == (pair[1].0, pair[1].1))
    {
        return Err(mismatch);
    }
    Ok(geometries)
}

/// 写入两版制品之间的几何变化（LFSD 第 4 节）。
///
/// 以主体实体的稳定标识配对几何投影值，输出新增、删除与修改变更并按规范键排序。
pub(super) fn artifact_geometry_changes<'a>(
    base: &ArtifactIndex<'a>,
    target: &ArtifactIndex<'a>,
    pass: &PassBudget<'_>,
    changes: &mut BoundedVec<FieldChange<'a>>,
    staged: &mut BoundedVec<u8>,
) -> Result<(), PortableEmissionError> {
    let base_values = artifact_geometry_values(
        base,
        PortableEmissionError::DiffBaseSemanticMismatch,
        pass,
        staged,
    )?;
    let target_values = artifact_geometry_values(
        target,
        PortableEmissionError::InternalBindingMismatch,
        pass,
        staged,
    )?;
    let change = |change_kind,
                  (entity_kind, stable_id, _): GeometryValue,
                  before: Option<Slots>,
                  after: Option<Slots>| {
        FieldChange {
            change_kind,
            entity_kind,
            stable_id,
            field_tag: 0,
            before: before.map(Payload::Staged),
            after: after.map(Payload::Staged),
        }
    };
    let (mut i, mut j) = (0, 0);
    while i < base_values.len() || j < target_values.len() {
        let before = base_values.get(i).copied();
        let after = target_values.get(j).copied();
        let ordering = match (before, after) {
            (Some(b), Some(a)) => (b.0, b.1).cmp(&(a.0, a.1)),
            (Some(_), None) => core::cmp::Ordering::Less,
            _ => core::cmp::Ordering::Greater,
        };
        match ordering {
            core::cmp::Ordering::Less => {
                let b = before.ok_or(PortableEmissionError::InternalBindingMismatch)?;
                changes.push(change(1, b, Some(b.2), None))?;
                i += 1;
            }
            core::cmp::Ordering::Greater => {
                let a = after.ok_or(PortableEmissionError::InternalBindingMismatch)?;
                changes.push(change(0, a, None, Some(a.2)))?;
                j += 1;
            }
            core::cmp::Ordering::Equal => {
                let (b, a) = before
                    .zip(after)
                    .ok_or(PortableEmissionError::InternalBindingMismatch)?;
                if b.2.of(staged) != a.2.of(staged) {
                    changes.push(change(2, a, Some(b.2), Some(a.2)))?;
                }
                i += 1;
                j += 1;
            }
        }
    }
    changes.sort_unstable_by_key(FieldChange::key);
    Ok(())
}

/// 两版制品之间的空间配置变化（LFSD 第 6 节）。
///
/// 空间配置唯一行逐字节相同则不产生变化；不同则借用前后完整字节。
pub(super) fn artifact_spatial_configuration_change<'a>(
    base: &ArtifactIndex<'a>,
    target: &ArtifactIndex<'a>,
) -> Result<Option<SpatialChange<'a>>, PortableEmissionError> {
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
        return Ok(None);
    }
    Ok(Some(SpatialChange::Changed(before.bytes(), after.bytes())))
}

/// 把目标制品的全部几何记录写为 Genesis 新增变化，按主体种类与稳定标识规范排序。
pub(super) fn genesis_geometry_changes<'a>(
    lir: &crate::lir::LirUnit,
    target: RegistryCheckedObjectView<'a>,
    changes: &mut BoundedVec<FieldChange<'a>>,
    staged: &mut BoundedVec<u8>,
) -> Result<(), PortableEmissionError> {
    let section = target
        .section(4)
        .ok_or(PortableEmissionError::InternalBindingMismatch)?;
    for (table_ordinal, subject_kind, projected_tags) in GEOMETRY_TABLES {
        let table = section
            .table(table_ordinal)
            .ok_or(PortableEmissionError::InternalBindingMismatch)?;
        for geometry in table.rows() {
            let subject_ordinal = checked_u32(geometry, 1)?;
            let frame_ordinal = checked_u32(geometry, 2)?;
            let value = push_canonical_geometry_value(
                staged,
                geometry,
                entity_stable_id(lir, EntityKind::CanonicalFrame, frame_ordinal),
                projected_tags,
            )?;
            changes.push(FieldChange {
                change_kind: 0,
                entity_kind: subject_kind,
                stable_id: entity_stable_id(lir, subject_kind, subject_ordinal),
                field_tag: 0,
                before: None,
                after: Some(Payload::Staged(value)),
            })?;
        }
    }
    changes.sort_unstable_by_key(FieldChange::key);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use super::*;
    use laneflow_format::preflight_object_values;

    const FULL_SPATIAL_LFCA: &[u8] =
        include_bytes!("../../../tests/fixtures/portable/lfca-full-spatial/expected.lfca");

    fn field_value_range(
        bytes: &[u8],
        section: u32,
        table: u32,
        row: u32,
        tag: u16,
    ) -> Range<usize> {
        let view = laneflow_format::preflight_object_registry(
            bytes,
            PortableObjectKind::CanonicalArtifact,
            FormatLimits::HARD,
        )
        .unwrap();
        let value = view
            .section(section)
            .and_then(|section| section.table(table))
            .and_then(|table| table.row(row))
            .and_then(|row| row.field_by_tag(tag))
            .unwrap()
            .value_bytes();
        let range = bytes
            .subslice_range(value)
            .expect("field value belongs to object bytes");
        range.start..range.end
    }

    fn geometry_changes<'a>(
        base: &ArtifactIndex<'a>,
        target: &ArtifactIndex<'a>,
    ) -> Vec<FieldChange<'a>> {
        let pass = crate::portable_emitter::budget::test_pass();
        let mut changes = pass.vec(1024).unwrap();
        let mut staged = pass.vec(1 << 20).unwrap();
        artifact_geometry_changes(base, target, &pass, &mut changes, &mut staged).unwrap();
        changes.to_vec()
    }

    #[test]
    fn direction_profile_applies_only_is_geometry_modify_not_spatial_configuration_modify() {
        for (table, subject_kind, applies_tag) in [
            (1_u32, EntityKind::LaneEdge, 6_u16),
            (2_u32, EntityKind::FacilityBand, 4_u16),
        ] {
            let mut base_bytes = FULL_SPATIAL_LFCA.to_vec();
            let mut target_bytes = FULL_SPATIAL_LFCA.to_vec();
            let direction_profile = field_value_range(&base_bytes, 4, 0, 0, 2);
            let accuracy_profile = field_value_range(&base_bytes, 6, 0, 0, 6);
            let applies = field_value_range(&base_bytes, 4, table, 0, applies_tag);
            for bytes in [&mut base_bytes, &mut target_bytes] {
                bytes[direction_profile.clone()].copy_from_slice(&[1]);
                bytes[accuracy_profile.clone()].copy_from_slice(&[1]);
                crate::compiler::portable_fixture_tests::refresh_portable_chunk_digest_containing(
                    bytes,
                    PortableObjectKind::CanonicalArtifact,
                    direction_profile.start,
                );
                crate::compiler::portable_fixture_tests::refresh_portable_chunk_digest_containing(
                    bytes,
                    PortableObjectKind::CanonicalArtifact,
                    accuracy_profile.start,
                );
            }
            base_bytes[applies.clone()].copy_from_slice(&[0]);
            target_bytes[applies.clone()].copy_from_slice(&[1]);
            crate::compiler::portable_fixture_tests::refresh_portable_chunk_digest_containing(
                &mut base_bytes,
                PortableObjectKind::CanonicalArtifact,
                applies.start,
            );
            crate::compiler::portable_fixture_tests::refresh_portable_chunk_digest_containing(
                &mut target_bytes,
                PortableObjectKind::CanonicalArtifact,
                applies.start,
            );

            let base_view = preflight_object_values(
                &base_bytes,
                PortableObjectKind::CanonicalArtifact,
                FormatLimits::HARD,
            )
            .unwrap()
            .registry_view();
            let target_view = preflight_object_values(
                &target_bytes,
                PortableObjectKind::CanonicalArtifact,
                FormatLimits::HARD,
            )
            .unwrap()
            .registry_view();
            let base = ArtifactIndex::build(
                base_view,
                PortableEmissionError::DiffBaseSemanticMismatch,
                &crate::portable_emitter::budget::test_pass(),
            )
            .unwrap();
            let target = ArtifactIndex::build(
                target_view,
                PortableEmissionError::InternalBindingMismatch,
                &crate::portable_emitter::budget::test_pass(),
            )
            .unwrap();

            let changes = geometry_changes(&base, &target);
            assert_eq!(changes.len(), 1, "{subject_kind:?}");
            let change = &changes[0];
            assert_eq!(change.change_kind, 2);
            assert_eq!(change.entity_kind, subject_kind);
            assert!(change.before.is_some() && change.after.is_some());
            assert!(
                artifact_spatial_configuration_change(&base, &target)
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[test]
    fn conflict_zone_region_field_change_is_a_geometry_modify() {
        let base_bytes = FULL_SPATIAL_LFCA.to_vec();
        let mut target_bytes = FULL_SPATIAL_LFCA.to_vec();
        let min_y = field_value_range(&target_bytes, 4, 3, 0, 3);
        target_bytes[min_y.start] ^= 1;
        crate::compiler::portable_fixture_tests::refresh_portable_chunk_digest_containing(
            &mut target_bytes,
            PortableObjectKind::CanonicalArtifact,
            min_y.start,
        );

        let base_view = preflight_object_values(
            &base_bytes,
            PortableObjectKind::CanonicalArtifact,
            FormatLimits::HARD,
        )
        .unwrap()
        .registry_view();
        let target_view = preflight_object_values(
            &target_bytes,
            PortableObjectKind::CanonicalArtifact,
            FormatLimits::HARD,
        )
        .unwrap()
        .registry_view();
        let base = ArtifactIndex::build(
            base_view,
            PortableEmissionError::DiffBaseSemanticMismatch,
            &crate::portable_emitter::budget::test_pass(),
        )
        .unwrap();
        let target = ArtifactIndex::build(
            target_view,
            PortableEmissionError::InternalBindingMismatch,
            &crate::portable_emitter::budget::test_pass(),
        )
        .unwrap();

        let changes = geometry_changes(&base, &target);
        assert_eq!(changes.len(), 1);
        let change = &changes[0];
        assert_eq!(change.change_kind, 2);
        assert_eq!(change.entity_kind, EntityKind::ConflictZone);
        assert!(change.before.is_some() && change.after.is_some());
        assert!(
            artifact_spatial_configuration_change(&base, &target)
                .unwrap()
                .is_none()
        );
    }
}
