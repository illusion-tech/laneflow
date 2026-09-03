//! 从真实两根独立重建局部成员全集；不调用 emitter 的配对或规范值编码器。

use super::base::{
    checked_ordinal_vector_with, checked_stable_id_with, checked_u8_with, checked_u16_with,
    checked_u32_with,
};
use super::policy_change::{Scratch, reserved};
use super::*;
use laneflow_static_contract::PortableFieldType;

const MISMATCH: PortableEmissionError = PortableEmissionError::PolicyDiffMismatch;
type Key<'a> = ([u8; 16], u8, &'a str);

#[derive(Clone, Copy)]
struct SourceMember<'a> {
    key: Key<'a>,
    side: u8,
    row: RegistryCheckedRowView<'a>,
}
#[derive(Clone, Copy)]
struct Actual<'a> {
    key: Key<'a>,
    row: RegistryCheckedRowView<'a>,
}

/// 独立检查 LFSD 4 路权增量及其与 Entity/StaticRule 表的排他分工。
///
/// base 必须是实际 LFCA 输入；最终目标、差异及非 Genesis 基线均重做调用方格式限制、
/// exact binding 与 revision 检查。本函数不证明既有其他领域的完整差异或来源真实性。
pub fn check_portable_policy_diff(
    base: PortableDiffBase<'_>,
    target: &[u8],
    diff: &[u8],
    limits: FormatLimits,
    compile_limits: &crate::CompileLimits,
) -> Result<(), PortableEmissionError> {
    let target_view =
        preflight_object_values(target, PortableObjectKind::CanonicalArtifact, limits)?;
    let diff_view =
        preflight_object_values(diff, PortableObjectKind::SemanticDiff, limits)?.registry_view();
    let base_view = match base {
        PortableDiffBase::Genesis => None,
        PortableDiffBase::Artifact(base) => Some(preflight_object_values(
            base.bytes(),
            PortableObjectKind::CanonicalArtifact,
            limits,
        )?),
    };
    let bindings = diff_view
        .section(0)
        .and_then(|s| s.table(0))
        .and_then(|t| t.row(0))
        .ok_or(MISMATCH)?;
    check_binding(target_view, bindings, false, limits)?;
    if let Some(base) = base_view {
        if checked_u8_with(bindings, 1, MISMATCH)? != 1 {
            return Err(MISMATCH);
        }
        check_binding(base, bindings, true, limits)?;
    } else if checked_u8_with(bindings, 1, MISMATCH)? != 0 {
        return Err(MISMATCH);
    }
    let mut scratch = Scratch::new(compile_limits.value(CompileLimitDimension::StageScratchBytes));
    let target_index = ArtifactIndex::build(target_view.registry_view(), MISMATCH, &mut scratch)?;
    let base_index = base_view
        .map(|v| ArtifactIndex::build(v.registry_view(), MISMATCH, &mut scratch))
        .transpose()?;
    if let Some(base) = &base_index {
        verify_artifact_diff_compatibility(base.view, target_index.view, base, &target_index)?;
    }
    for index in base_index.iter().chain(core::iter::once(&target_index)) {
        validate_policy_references(index, &mut scratch, MISMATCH)?;
    }
    let indices = [base_index.as_ref(), Some(&target_index)];
    let mut count = 0_usize;
    for index in indices.iter().flatten() {
        for table in 1..=4 {
            count = count
                .checked_add(
                    index
                        .view
                        .section(3)
                        .and_then(|s| s.table(table))
                        .ok_or(MISMATCH)?
                        .row_count() as usize,
                )
                .ok_or(PortableEmissionError::ArithmeticOverflow)?;
        }
    }
    let mut source = reserved::<SourceMember<'_>>(count, &mut scratch)?;
    // K 来自两根完整局部表，不由实际差异行反推扫描范围。
    for (side, index) in indices.iter().enumerate() {
        let Some(index) = index else {
            continue;
        };
        for kind in 0..4_u8 {
            for row in index
                .view
                .section(3)
                .and_then(|s| s.table(u32::from(kind) + 1))
                .ok_or(MISMATCH)?
                .rows()
            {
                source.push(SourceMember {
                    side: side as u8,
                    row,
                    key: (
                        index.stable_id(
                            EntityKind::RightOfWayPolicySet,
                            checked_u32_with(row, 1, MISMATCH)?,
                            MISMATCH,
                        )?,
                        kind,
                        text(row, 2)?,
                    ),
                });
            }
        }
    }
    source.sort_unstable_by(|a, b| (a.key, a.side).cmp(&(b.key, b.side)));
    let table = diff_view
        .section(6)
        .and_then(|s| s.table(0))
        .ok_or(MISMATCH)?;
    let mut actual = reserved::<Actual<'_>>(table.row_count() as usize, &mut scratch)?;
    for row in table.rows() {
        actual.push(Actual {
            row,
            key: (
                checked_stable_id_with(row, 2, MISMATCH)?,
                checked_u8_with(row, 3, MISMATCH)?,
                text(row, 4)?,
            ),
        });
    }
    actual.sort_unstable_by(|a, b| a.key.cmp(&b.key));
    let (mut position, mut emitted) = (0, 0);
    while position < source.len() {
        let first = source[position];
        let mut sides = [None, None];
        while position < source.len() && source[position].key == first.key {
            let member = source[position];
            if sides[member.side as usize].replace(member).is_some() {
                return Err(MISMATCH);
            }
            position += 1;
        }
        let operation = match sides {
            [None, Some(_)] => Some(0),
            [Some(_), None] => Some(1),
            [Some(b), Some(a)] => {
                if same_member(indices[0].ok_or(MISMATCH)?, b, &target_index, a)? {
                    None
                } else {
                    Some(2)
                }
            }
            _ => return Err(MISMATCH),
        };
        let Some(operation) = operation else {
            continue;
        };
        let row = actual
            .get(emitted)
            .filter(|r| r.key == first.key)
            .ok_or(MISMATCH)?
            .row;
        if checked_u8_with(row, 1, MISMATCH)? != operation {
            return Err(MISMATCH);
        }
        for (side, source) in sides.iter().enumerate() {
            let payload = row.field_by_tag(5 + side as u16);
            match (source, payload) {
                (None, None) => {}
                (Some(member), Some(payload)) => check_payload(
                    indices[side].ok_or(MISMATCH)?,
                    *member,
                    payload.value_bytes(),
                )?,
                _ => return Err(MISMATCH),
            }
        }
        emitted += 1;
    }
    if emitted != actual.len() {
        return Err(MISMATCH);
    }
    check_field_partition(base_index.as_ref(), &target_index, diff_view, &mut scratch)?;
    Ok(())
}

fn check_binding(
    view: ValueCheckedObjectView<'_>,
    bindings: RegistryCheckedRowView<'_>,
    base: bool,
    limits: FormatLimits,
) -> Result<(), PortableEmissionError> {
    let checked = laneflow_format::check_canonical_network_input(view.bytes(), limits)
        .map_err(|_| MISMATCH)?;
    let first = if base { 2 } else { 6 };
    let values: [&[u8]; 4] = [
        &NETWORK_REVISION_DERIVATION_VERSION.to_le_bytes(),
        &checked.network_revision().into_digest().into_bytes(),
        &sha256(view.bytes()).into_bytes(),
        &(view.bytes().len() as u64).to_le_bytes(),
    ];
    for (i, expected) in values.iter().enumerate() {
        if bindings
            .field_by_tag(first + i as u16)
            .ok_or(MISMATCH)?
            .value_bytes()
            != *expected
        {
            return Err(MISMATCH);
        }
    }
    Ok(())
}

fn text(row: RegistryCheckedRowView<'_>, tag: u16) -> Result<&str, PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(MISMATCH)?.value()? {
        RegistryCheckedFieldValue::Utf8(v) => Ok(v),
        _ => Err(MISMATCH),
    }
}

fn reference_kind(kind: u8, tag: u16) -> Option<(EntityKind, bool)> {
    if kind < 2 {
        return None;
    }
    match tag {
        3 => Some((
            if kind == 2 {
                EntityKind::ParticipantStream
            } else {
                EntityKind::ManeuverGate
            },
            false,
        )),
        4 => Some((EntityKind::ParticipantClass, true)),
        6 if kind == 2 => Some((EntityKind::ParticipantStream, true)),
        _ => None,
    }
}

fn same_member(
    base: &ArtifactIndex<'_>,
    b: SourceMember<'_>,
    target: &ArtifactIndex<'_>,
    a: SourceMember<'_>,
) -> Result<bool, PortableEmissionError> {
    if b.row.field_count() != a.row.field_count() {
        return Ok(false);
    }
    for (bf, af) in b.row.fields().skip(2).zip(a.row.fields().skip(2)) {
        if bf.tag() != af.tag() {
            return Ok(false);
        }
        if let Some((kind, vector)) = reference_kind(b.key.1, bf.tag()) {
            if vector {
                let bv = checked_ordinal_vector_with(b.row, bf.tag(), MISMATCH)?;
                let av = checked_ordinal_vector_with(a.row, af.tag(), MISMATCH)?;
                if bv.len() != av.len() {
                    return Ok(false);
                }
                for i in 0..bv.len() {
                    if base.stable_id(kind, bv.get(i).ok_or(MISMATCH)?, MISMATCH)?
                        != target.stable_id(kind, av.get(i).ok_or(MISMATCH)?, MISMATCH)?
                    {
                        return Ok(false);
                    }
                }
            } else if base.stable_id(
                kind,
                checked_u32_with(b.row, bf.tag(), MISMATCH)?,
                MISMATCH,
            )? != target.stable_id(
                kind,
                checked_u32_with(a.row, af.tag(), MISMATCH)?,
                MISMATCH,
            )? {
                return Ok(false);
            }
        } else if bf.value_bytes() != af.value_bytes() {
            return Ok(false);
        }
    }
    Ok(true)
}

fn check_payload(
    index: &ArtifactIndex<'_>,
    member: SourceMember<'_>,
    payload: &[u8],
) -> Result<(), PortableEmissionError> {
    // Format 已证明完整 RowV1；此处独立比较每个字段和稳定引用，不生成第二份 payload。
    let mut cursor = 16_usize;
    for field in member.row.fields().skip(2) {
        let header = payload.get(cursor..cursor + 12).ok_or(MISMATCH)?;
        let length = usize::try_from(u64::from_le_bytes(
            header[4..12].try_into().map_err(|_| MISMATCH)?,
        ))
        .map_err(|_| MISMATCH)?;
        if u16::from_le_bytes([header[0], header[1]]) != field.tag() {
            return Err(MISMATCH);
        }
        cursor += 12;
        let end = cursor.checked_add(length).ok_or(MISMATCH)?;
        let value = payload.get(cursor..end).ok_or(MISMATCH)?;
        if let Some((kind, vector)) = reference_kind(member.key.1, field.tag()) {
            if header[2] != PortableFieldType::Bytes as u8 {
                return Err(MISMATCH);
            }
            if vector {
                let refs = checked_ordinal_vector_with(member.row, field.tag(), MISMATCH)?;
                if value.len() as u64 != 4 + u64::from(refs.len()) * 18
                    || value[..4] != refs.len().to_le_bytes()
                {
                    return Err(MISMATCH);
                }
                for (i, bytes) in value[4..].as_chunks::<18>().0.iter().enumerate() {
                    check_reference(index, kind, refs.get(i as u32).ok_or(MISMATCH)?, bytes)?;
                }
            } else {
                check_reference(
                    index,
                    kind,
                    checked_u32_with(member.row, field.tag(), MISMATCH)?,
                    value,
                )?;
            }
        } else if header[2] != field.field_type() as u8 || value != field.value_bytes() {
            return Err(MISMATCH);
        }
        cursor = end;
    }
    if cursor != payload.len() {
        return Err(MISMATCH);
    }
    Ok(())
}

fn check_reference(
    index: &ArtifactIndex<'_>,
    kind: EntityKind,
    ordinal: u32,
    value: &[u8],
) -> Result<(), PortableEmissionError> {
    if value.len() != 18
        || value[..2] != kind.code().to_le_bytes()
        || value[2..] != index.stable_id(kind, ordinal, MISMATCH)?
    {
        return Err(MISMATCH);
    }
    Ok(())
}

fn check_field_partition(
    base: Option<&ArtifactIndex<'_>>,
    target: &ArtifactIndex<'_>,
    diff: RegistryCheckedObjectView<'_>,
    scratch: &mut Scratch,
) -> Result<(), PortableEmissionError> {
    // policy 的实体增删、保留属性与 Movement 方向属于原表，不能用局部成员行顶替。
    type PartitionKey = (u8, u16, [u8; 16], u16);
    let relevant =
        |section: u8, row: RegistryCheckedRowView<'_>| -> Result<bool, PortableEmissionError> {
            let kind = checked_u16_with(row, 2, MISMATCH)?;
            Ok(kind == EntityKind::RightOfWayPolicySet.code()
                || (section == 1
                    && kind == EntityKind::Movement.code()
                    && checked_u8_with(row, 1, MISMATCH)? == 2))
        };
    let mut count = 0_usize;
    for section in [1, 4] {
        for row in diff
            .section(u32::from(section))
            .and_then(|s| s.table(0))
            .ok_or(MISMATCH)?
            .rows()
        {
            if relevant(section, row)? {
                count = count
                    .checked_add(1)
                    .ok_or(PortableEmissionError::ArithmeticOverflow)?;
            }
        }
    }
    let mut actual = reserved::<(PartitionKey, RegistryCheckedRowView<'_>)>(count, scratch)?;
    for section in [1, 4] {
        for row in diff
            .section(u32::from(section))
            .and_then(|s| s.table(0))
            .ok_or(MISMATCH)?
            .rows()
        {
            if relevant(section, row)? {
                actual.push((
                    (
                        section,
                        checked_u16_with(row, 2, MISMATCH)?,
                        checked_stable_id_with(row, 4, MISMATCH)?,
                        if row.field_by_tag(6).is_some() {
                            checked_u16_with(row, 6, MISMATCH)?
                        } else {
                            0
                        },
                    ),
                    row,
                ));
            }
        }
    }
    actual.sort_unstable_by_key(|r| r.0);
    if actual.windows(2).any(|w| w[0].0 == w[1].0) {
        return Err(MISMATCH);
    }
    let find = |key: PartitionKey| {
        actual
            .binary_search_by_key(&key, |r| r.0)
            .map(|i| actual[i].1)
            .map_err(|_| MISMATCH)
    };
    let mut expected = 0;
    for (side, index) in [base, Some(target)].into_iter().enumerate() {
        let Some(index) = index else {
            continue;
        };
        let other = if side == 0 { Some(target) } else { base };
        for ((kind, id), entity) in index.entities() {
            if *kind != EntityKind::RightOfWayPolicySet {
                continue;
            }
            if other.is_some_and(|o| o.entity(&(*kind, *id)).is_some()) {
                continue;
            }
            let op = if side == 0 { 1 } else { 0 };
            let row = find((1, kind.code(), *id, 0))?;
            if checked_u8_with(row, 1, MISMATCH)? != op
                || row
                    .field_by_tag(if side == 0 { 9 } else { 10 })
                    .ok_or(MISMATCH)?
                    .value_bytes()
                    != entity.row.bytes()
            {
                return Err(MISMATCH);
            }
            expected += 1;
        }
    }
    if let Some(base) = base {
        for ((kind, id), before) in base.entities() {
            let tags: &[u16] = match kind {
                EntityKind::RightOfWayPolicySet => &[3, 4, 5],
                EntityKind::Movement => &[7],
                _ => continue,
            };
            let Some(after) = target.entity(&(*kind, *id)) else {
                continue;
            };
            for tag in tags {
                let bv = before.row.field_by_tag(*tag).map(|f| f.value_bytes());
                let av = after.row.field_by_tag(*tag).map(|f| f.value_bytes());
                if bv == av {
                    continue;
                }
                let section = if *kind == EntityKind::Movement { 1 } else { 4 };
                let op = if section == 1 { 2 } else { 0 };
                let row = find((section, kind.code(), *id, *tag))?;
                if checked_u8_with(row, 1, MISMATCH)? != op
                    || row.field_by_tag(9).map(|f| f.value_bytes()) != bv
                    || row.field_by_tag(10).map(|f| f.value_bytes()) != av
                {
                    return Err(MISMATCH);
                }
                expected += 1;
            }
        }
    }
    if actual.len() != expected {
        return Err(MISMATCH);
    }
    Ok(())
}
