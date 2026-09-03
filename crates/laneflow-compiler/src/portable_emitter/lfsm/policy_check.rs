//! 从同次受检 source view 独立核对最终位置；不调用 emitter 的位置编码/成员配对。

use super::super::lfsd::base::{checked_stable_id_with, checked_u32_with};
use super::super::lfsd::policy_change::{Scratch, reserved};
use super::*;
use crate::{PolicySourceTarget, SourceLocationView, ValidatedSourceMapInput};
use laneflow_format::RegistryCheckedTableView;

#[cfg(test)]
mod tests;

const MISMATCH: PortableEmissionError = PortableEmissionError::PolicySourceMismatch;
type Row<'a> = RegistryCheckedRowView<'a>;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct LocationKey<'a> {
    kind: u8,
    module: u32,
    document: u32,
    text: Option<(u32, u32, u32, u32)>,
    subject: Option<u8>,
    namespace: Option<&'a str>,
    entity: Option<u16>,
    parents: [Option<&'a str>; 3],
    key: Option<&'a str>,
    owner: Option<u8>,
    relation: Option<u8>,
    occurrence: Option<u8>,
    index: Option<u32>,
    steps: [Option<(u8, u16, u16)>; 4],
    canvas: Option<&'a str>,
}

impl LocationKey<'_> {
    fn empty(kind: u8, module: u32, document: u32) -> Self {
        Self {
            kind,
            module,
            document,
            text: None,
            subject: None,
            namespace: None,
            entity: None,
            parents: [None; 3],
            key: None,
            owner: None,
            relation: None,
            occurrence: None,
            index: None,
            steps: [None; 4],
            canvas: None,
        }
    }
}

fn text(row: Row<'_>, tag: u16) -> Result<&str, PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(MISMATCH)?.value()? {
        RegistryCheckedFieldValue::Utf8(v) => Ok(v),
        _ => Err(MISMATCH),
    }
}
fn number(row: Row<'_>, tag: u16) -> Result<u64, PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(MISMATCH)?.value()? {
        RegistryCheckedFieldValue::U8(v) => Ok(v.into()),
        RegistryCheckedFieldValue::U16(v) => Ok(v.into()),
        RegistryCheckedFieldValue::U32(v) => Ok(v.into()),
        RegistryCheckedFieldValue::U64(v) => Ok(v),
        _ => Err(MISMATCH),
    }
}
fn raw_key(row: Row<'_>) -> Result<LocationKey<'_>, PortableEmissionError> {
    let mut key = LocationKey::empty(
        number(row, 2)? as u8,
        number(row, 3)? as u32,
        number(row, 4)? as u32,
    );
    if key.kind == 0 {
        key.text = Some((
            number(row, 5)? as u32,
            number(row, 6)? as u32,
            number(row, 7)? as u32,
            number(row, 8)? as u32,
        ));
    } else {
        key.subject = Some(number(row, 9)? as u8);
        key.namespace = row.field_by_tag(10).map(|_| text(row, 10)).transpose()?;
        key.entity = row
            .field_by_tag(11)
            .map(|_| number(row, 11).map(|v| v as u16))
            .transpose()?;
        for (i, slot) in key.parents.iter_mut().enumerate() {
            *slot = row
                .field_by_tag(12 + i as u16)
                .map(|_| text(row, 12 + i as u16))
                .transpose()?;
        }
        key.key = row.field_by_tag(15).map(|_| text(row, 15)).transpose()?;
        key.owner = row
            .field_by_tag(16)
            .map(|_| number(row, 16).map(|v| v as u8))
            .transpose()?;
        key.relation = row
            .field_by_tag(17)
            .map(|_| number(row, 17).map(|v| v as u8))
            .transpose()?;
        key.occurrence = row
            .field_by_tag(18)
            .map(|_| number(row, 18).map(|v| v as u8))
            .transpose()?;
        key.index = row
            .field_by_tag(19)
            .map(|_| number(row, 19).map(|v| v as u32))
            .transpose()?;
        if let Some(field) = row.field_by_tag(20) {
            let RegistryCheckedFieldValue::RecordVector(steps) = field.value()? else {
                return Err(MISMATCH);
            };
            for (i, step) in steps.rows().enumerate() {
                *key.steps.get_mut(i).ok_or(MISMATCH)? = Some((
                    number(step, 1)? as u8,
                    number(step, 2)? as u16,
                    number(step, 3)? as u16,
                ));
            }
        }
        key.canvas = row.field_by_tag(21).map(|_| text(row, 21)).transpose()?;
    }
    Ok(key)
}

fn source_key<'a>(
    view: SourceLocationView<'a>,
    documents: &Documents<'_>,
) -> Result<LocationKey<'a>, PortableEmissionError> {
    let (module, document) = documents.resolve(view.source_document_key())?;
    let mut key = LocationKey::empty(0, module, document);
    match view {
        SourceLocationView::Text { start, end, .. } => {
            key.text = Some((start.line(), start.column(), end.line(), end.column()));
        }
        SourceLocationView::RoadEditing(location) => {
            if location.byte_range().is_some() {
                return Err(MISMATCH);
            }
            key.kind = 1;
            let context = location.context();
            let address = match location.subject() {
                crate::RoadEditingSubject::ModuleHeader => {
                    key.subject = Some(0);
                    None
                }
                crate::RoadEditingSubject::RoadAlignment { address } => {
                    key.subject = Some(1);
                    Some(address)
                }
                crate::RoadEditingSubject::Declaration { address } => {
                    key.subject = Some(2);
                    Some(address)
                }
                crate::RoadEditingSubject::OwnerLocal {
                    owner,
                    relation,
                    occurrence,
                } => {
                    key.subject = Some(3);
                    key.relation = Some(road_editing_relation_code(*relation));
                    let (kind, index) = match *occurrence {
                        crate::RoadEditingRelationOccurrence::OrderedProductOrdinal(i) => (0, i),
                        crate::RoadEditingRelationOccurrence::CanonicalSetOrdinal(i) => (1, i),
                    };
                    key.occurrence = Some(kind);
                    key.index = Some(index);
                    match owner {
                        crate::RoadEditingOwner::ModuleHeader => {
                            key.owner = Some(0);
                            None
                        }
                        crate::RoadEditingOwner::Address(a) => {
                            key.owner = Some(1);
                            Some(a)
                        }
                    }
                }
                crate::RoadEditingSubject::Wire { .. } => return Err(MISMATCH),
            };
            if let Some(address) = address {
                key.namespace = Some(address.module_namespace(context));
                key.entity = address.entity_kind().map(EntityKind::code);
                key.key = Some(address.local_key(context));
                for (i, parent) in address.owner_local_keys(context).enumerate() {
                    *key.parents.get_mut(i).ok_or(MISMATCH)? = Some(parent);
                }
            }
            if let Some(path) = location.property_path() {
                for (i, step) in path.steps().iter().enumerate() {
                    *key.steps.get_mut(i).ok_or(MISMATCH)? = Some(match *step {
                        crate::RoadEditingPropertyStep::TableField { table, field_id } => {
                            (0, road_editing_table_code(table), field_id)
                        }
                        crate::RoadEditingPropertyStep::StructMember {
                            structure,
                            member_id,
                        } => (1, road_editing_struct_code(structure), u16::from(member_id)),
                        crate::RoadEditingPropertyStep::UnionVariant {
                            union,
                            discriminant,
                        } => (2, road_editing_union_code(union), u16::from(discriminant)),
                    });
                }
            }
            key.canvas = location.canvas_selection();
        }
    }
    Ok(key)
}

fn ordinals(
    row: Row<'_>,
    tag: u16,
) -> Result<RegistryCheckedOrdinalVectorView<'_>, PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(MISMATCH)?.value()? {
        RegistryCheckedFieldValue::OrdinalVectorU32(v) => Ok(v),
        _ => Err(MISMATCH),
    }
}

#[derive(Clone, Copy)]
struct Member<'a> {
    owner: [u8; 16],
    kind: u8,
    key: &'a str,
    local_index: u32,
    row: Row<'a>,
}

/// 独立检查 LFSM 4 的完整策略来源和 Movement 方向来源。
///
/// source 必须来自同次 CompilationOutput。以实际 LFCA/文档描述符重建全集，不接受
/// LFSM 自报地址作为来源权威；既有角色的来源语义仍由原有编译流水线拥有。
pub fn check_portable_policy_sources(
    artifact: &[u8],
    source: &ValidatedSourceMapInput,
    map: &[u8],
    limits: FormatLimits,
    compile_limits: &crate::CompileLimits,
) -> Result<(), PortableEmissionError> {
    let canonical =
        preflight_object_values(artifact, PortableObjectKind::CanonicalArtifact, limits)?;
    let checked =
        laneflow_format::check_canonical_network_input(artifact, limits).map_err(|_| MISMATCH)?;
    let view = preflight_object_values(map, PortableObjectKind::SourceMap, limits)?.registry_view();
    let bindings = table(view, 0, 0)?.row(0).ok_or(MISMATCH)?;
    if bindings.field_by_tag(4).ok_or(MISMATCH)?.value_bytes() != sha256(artifact).as_bytes()
        || number(bindings, 5)? != artifact.len() as u64
        || bindings.field_by_tag(2).ok_or(MISMATCH)?.value_bytes()
            != checked.network_revision().into_digest().as_bytes()
        || bindings.field_by_tag(8).ok_or(MISMATCH)?.value_bytes()
            != source_collection_digest_from_map(source)?
    {
        return Err(MISMATCH);
    }
    let provenance = table(canonical.registry_view(), 6, 0)?
        .row(0)
        .ok_or(MISMATCH)?;
    // compiler ID 和来源摘要必须同时属于这份 LFCA，不能仅把 LFSM 重新绑定到别的根。
    if provenance.field_by_tag(1).ok_or(MISMATCH)?.value_bytes()
        != bindings.field_by_tag(6).ok_or(MISMATCH)?.value_bytes()
        || provenance.field_by_tag(3).ok_or(MISMATCH)?.value_bytes()
            != bindings.field_by_tag(8).ok_or(MISMATCH)?.value_bytes()
    {
        return Err(MISMATCH);
    }
    let mut scratch = Scratch::new(compile_limits.value(CompileLimitDimension::StageScratchBytes));
    let documents = check_documents(source, view, &mut scratch)?;
    let location_table = table(view, 1, 2)?;
    let root = canonical.registry_view();
    let member_count = (1..=4).try_fold(0_usize, |n, t| {
        n.checked_add(table(root, 3, t)?.row_count() as usize)
            .ok_or(PortableEmissionError::ArithmeticOverflow)
    })?;
    let needs_projection = source.policy_sources().len() != 0
        || table(root, 2, 23)?.row_count() != 0
        || member_count != 0
        || table(root, 2, 5)?
            .rows()
            .any(|r| r.field_by_tag(7).is_some());
    // 空策略路网只做流式位置扫描和引用位图；不为既有 P100 配置建立完整位置键副本。
    let mut locations = reserved::<LocationKey<'_>>(
        if needs_projection {
            location_table.row_count() as usize
        } else {
            0
        },
        &mut scratch,
    )?;
    let mut languages = reserved::<u8>(source.source_modules().len(), &mut scratch)?;
    languages.extend(
        source
            .source_modules()
            .map(|m| (source_language_code(m.source_language()) - 1) as u8),
    );
    // 只保存来源借用及 LFSM ordinal；按实际 ordinal 合并位置流，不随机重扫位置表。
    let mut primaries = reserved::<(u32, crate::SourceModuleSourceView<'_>)>(
        source.source_modules().len(),
        &mut scratch,
    )?;
    for (row, module) in table(view, 1, 0)?
        .rows()
        .zip(source.source_module_sources())
    {
        primaries.push((checked_u32_with(row, 13, MISMATCH)?, module));
    }
    primaries.sort_unstable_by_key(|p| p.0);
    let mut primary_index = 0;
    let mut previous_location = None;
    for (i, row) in location_table.rows().enumerate() {
        if number(row, 1)? != i as u64 {
            return Err(MISMATCH);
        }
        let key = raw_key(row)?;
        if previous_location.is_some_and(|last| last >= key)
            || documents
                .entries
                .get(key.document as usize)
                .is_none_or(|d| d.1 != key.module)
            || languages.get(key.module as usize) != Some(&key.kind)
        {
            return Err(MISMATCH);
        }
        while let Some((ordinal, module)) = primaries.get(primary_index)
            && *ordinal as usize == i
        {
            if key != source_key(module.primary_source(), &documents)? {
                return Err(MISMATCH);
            }
            primary_index += 1;
        }
        previous_location = Some(key);
        if needs_projection {
            locations.push(key);
        }
    }
    if primary_index != primaries.len() {
        return Err(MISMATCH);
    }
    scratch.release(
        (primaries.capacity() * size_of::<(u32, crate::SourceModuleSourceView<'_>)>()) as u64,
    );
    drop(primaries);
    let projection_rows = check_pool(view, location_table.row_count() as usize, &mut scratch)?;
    if projection_rows.policies != table(root, 2, 23)?.row_count() as usize
        || projection_rows.members != member_count
        || projection_rows.movements != table(root, 2, 5)?.row_count() as usize
    {
        return Err(MISMATCH);
    }
    if !needs_projection {
        if projection_rows.movement_contributions {
            return Err(MISMATCH);
        }
        return Ok(());
    }
    let policies = table(root, 2, 23)?;
    let owners = policy_owner_ids(policies, &mut scratch)?;
    let mut members = reserved::<Member<'_>>(member_count, &mut scratch)?;
    for kind in 0..4_u8 {
        let mut previous_owner = None;
        let mut local_index = 0;
        for row in table(root, 3, u32::from(kind) + 1)?.rows() {
            let ordinal = checked_u32_with(row, 1, MISMATCH)?;
            if previous_owner != Some(ordinal) {
                previous_owner = Some(ordinal);
                local_index = 0;
            }
            members.push(Member {
                owner: *owners.get(ordinal as usize).ok_or(MISMATCH)?,
                kind,
                key: text(row, 2)?,
                local_index,
                row,
            });
            local_index += 1;
        }
    }
    scratch.release((owners.capacity() * size_of::<[u8; 16]>()) as u64);
    drop(owners);
    members.sort_unstable_by_key(|m| (m.owner, m.kind, m.key));
    let mut sources =
        reserved::<crate::PolicySourceView<'_>>(source.policy_sources().len(), &mut scratch)?;
    sources.extend(source.policy_sources());
    sources.sort_unstable_by(|a, b| a.target().cmp(b.target()));
    if sources.windows(2).any(|w| w[0].target() == w[1].target()) {
        return Err(MISMATCH);
    }
    let mut stable_rows =
        reserved::<Row<'_>>(table(view, 2, 0)?.row_count() as usize, &mut scratch)?;
    stable_rows.extend(table(view, 2, 0)?.rows());
    let mut local_rows =
        reserved::<Row<'_>>(table(view, 3, 0)?.row_count() as usize, &mut scratch)?;
    local_rows.extend(table(view, 3, 0)?.rows());
    let mut seen = 0_usize;
    let mut identities = policy_identities(root)?;
    for row in policies.rows() {
        let id = checked_stable_id_with(row, 2, MISMATCH)?;
        let ordinal = checked_u32_with(row, 1, MISMATCH)?;
        let identity = identities.next().ok_or(MISMATCH)?;
        if number(identity, 1)? != 24
            || checked_u32_with(identity, 2, MISMATCH)? != ordinal
            || checked_stable_id_with(identity, 3, MISMATCH)? != id
        {
            return Err(MISMATCH);
        }
        let target = PolicySourceTarget::Declaration {
            id: typed_id(id),
            ordinal: Ordinal::from_raw(ordinal),
        };
        let source_view = find_source(&sources, &target)?;
        let actual = find_stable(&stable_rows, 24, id)?;
        if number(actual, 3)? != u64::from(ordinal) {
            return Err(MISMATCH);
        }
        check_projection(source_view, actual, 4, &locations, &documents, &mut scratch)?;
        let primary = source_key(source_view.primary_source(), &documents)?;
        check_policy_primary(primary, identity, source)?;
        check_road_fields(source_view, primary, row, None, &documents)?;
        seen += 1;
    }
    if identities.next().is_some() {
        return Err(MISMATCH);
    }
    for member in &members {
        // 比较借用 key，不从 LFSM localIndex 反推需要检查的来源成员。
        let source_view = sources
            .binary_search_by(|s| match s.target() {
                PolicySourceTarget::Declaration { .. } => core::cmp::Ordering::Less,
                PolicySourceTarget::Member { owner, kind, key } => (
                    stable_id_bytes(*owner),
                    kind.code(),
                    key.as_ref(),
                )
                    .cmp(&(member.owner, member.kind, member.key)),
                PolicySourceTarget::MovementDirection { .. } => core::cmp::Ordering::Greater,
            })
            .map(|i| sources[i])
            .map_err(|_| MISMATCH)?;
        let actual = find_local(
            &local_rows,
            member.owner,
            member.kind + 33,
            member.local_index,
        )?;
        check_projection(source_view, actual, 5, &locations, &documents, &mut scratch)?;
        let primary = source_key(source_view.primary_source(), &documents)?;
        let owner = find_source(
            &sources,
            &PolicySourceTarget::Declaration {
                id: typed_id(member.owner),
                ordinal: Ordinal::from_raw(checked_u32_with(member.row, 1, MISMATCH)?),
            },
        )?;
        let owner_primary = source_key(owner.primary_source(), &documents)?;
        if primary.module != owner_primary.module {
            return Err(MISMATCH);
        }
        if primary.kind == 1 {
            let mut expected = owner_primary;
            expected.subject = Some(3);
            expected.owner = Some(1);
            expected.relation = Some(member.kind + 16);
            expected.occurrence = Some(1);
            expected.index = Some(member.local_index);
            expected.steps = [Some((0, 40, u16::from(member.kind) + 2)), None, None, None];
            if primary != expected {
                return Err(MISMATCH);
            }
        }
        check_road_fields(
            source_view,
            primary,
            member.row,
            Some(member.kind),
            &documents,
        )?;
        seen += 1;
    }
    let mut movement_sources = reserved::<([u8; 16], crate::MovementSourceView<'_>)>(
        source.movement_sources().len(),
        &mut scratch,
    )?;
    movement_sources.extend(
        source
            .movement_sources()
            .map(|s| (stable_id_bytes(s.stable_id()), s)),
    );
    movement_sources.sort_unstable_by_key(|s| s.0);
    for row in table(root, 2, 5)?.rows() {
        let id = checked_stable_id_with(row, 2, MISMATCH)?;
        let actual = find_stable(&stable_rows, EntityKind::Movement.code(), id)?;
        if number(actual, 3)? != number(row, 1)? {
            return Err(MISMATCH);
        }
        let movement_source = movement_sources
            .binary_search_by_key(&id, |s| s.0)
            .map(|i| movement_sources[i].1)
            .map_err(|_| MISMATCH)?;
        let movement_primary = source_key(movement_source.primary_source(), &documents)?;
        if locations.get(number(actual, 4)? as usize) != Some(&movement_primary) {
            return Err(MISMATCH);
        }
        if row.field_by_tag(7).is_some() {
            let direction = find_source(
                &sources,
                &PolicySourceTarget::MovementDirection { id: typed_id(id) },
            )?;
            let expected = source_key(direction.primary_source(), &documents)?;
            if expected.module != movement_primary.module {
                return Err(MISMATCH);
            }
            let ordinal = locations.binary_search(&expected).map_err(|_| MISMATCH)? as u32;
            let contributing = ordinals(actual, 5)?;
            if contributing.len() != 1
                || contributing.get(0) != Some(ordinal)
                || direction.contributing_sources().len() != 0
            {
                return Err(MISMATCH);
            }
            if expected.kind == 1 {
                let mut primary = expected;
                primary.steps = [None; 4];
                if primary != movement_primary {
                    return Err(MISMATCH);
                }
                if expected.steps != [Some((0, 14, 5)), None, None, None] {
                    return Err(MISMATCH);
                }
                if locations.get(number(actual, 4)? as usize) != Some(&primary) {
                    return Err(MISMATCH);
                }
            }
            seen += 1;
        } else if !ordinals(actual, 5)?.is_empty() {
            return Err(MISMATCH);
        }
    }
    if seen != sources.len() {
        return Err(MISMATCH);
    }
    Ok(())
}

fn table(
    view: RegistryCheckedObjectView<'_>,
    section: u32,
    table: u32,
) -> Result<laneflow_format::RegistryCheckedTableView<'_>, PortableEmissionError> {
    view.section(section)
        .and_then(|s| s.table(table))
        .ok_or(MISMATCH)
}

fn typed_id<K: EntityKindMarker>(bytes: [u8; 16]) -> StableId<K> {
    StableId::from_untyped(laneflow_static_contract::StableId128::from_bytes(bytes))
}

fn find_source<'a>(
    sources: &[crate::PolicySourceView<'a>],
    target: &PolicySourceTarget,
) -> Result<crate::PolicySourceView<'a>, PortableEmissionError> {
    sources
        .binary_search_by(|s| s.target().cmp(target))
        .map(|i| sources[i])
        .map_err(|_| MISMATCH)
}
fn find_stable<'a>(
    rows: &[Row<'a>],
    kind: u16,
    id: [u8; 16],
) -> Result<Row<'a>, PortableEmissionError> {
    rows.binary_search_by_key(&(kind, id), |r| {
        (
            number(*r, 1).unwrap_or(0) as u16,
            checked_stable_id_with(*r, 2, MISMATCH).unwrap_or_default(),
        )
    })
    .map(|i| rows[i])
    .map_err(|_| MISMATCH)
}
fn find_local<'a>(
    rows: &[Row<'a>],
    id: [u8; 16],
    role: u8,
    index: u32,
) -> Result<Row<'a>, PortableEmissionError> {
    rows.binary_search_by_key(&(24_u16, id, role, index), |r| {
        (
            number(*r, 1).unwrap_or(0) as u16,
            checked_stable_id_with(*r, 2, MISMATCH).unwrap_or_default(),
            number(*r, 3).unwrap_or(0) as u8,
            number(*r, 4).unwrap_or(0) as u32,
        )
    })
    .map(|i| rows[i])
    .map_err(|_| MISMATCH)
}

fn policy_owner_ids(
    policies: RegistryCheckedTableView<'_>,
    scratch: &mut Scratch,
) -> Result<Vec<[u8; 16]>, PortableEmissionError> {
    // 只索引成员需要的 StableId；共用累计预算，不复制策略或 Identity 载荷。
    let mut owners = reserved::<[u8; 16]>(policies.row_count() as usize, scratch)?;
    for (ordinal, row) in policies.rows().enumerate() {
        if checked_u32_with(row, 1, MISMATCH)? as usize != ordinal {
            return Err(MISMATCH);
        }
        owners.push(checked_stable_id_with(row, 2, MISMATCH)?);
    }
    Ok(owners)
}

fn policy_identities(
    view: RegistryCheckedObjectView<'_>,
) -> Result<impl Iterator<Item = Row<'_>>, PortableEmissionError> {
    let offset = (0..23).try_fold(0_u32, |n, i| {
        n.checked_add(table(view, 2, i)?.row_count())
            .ok_or(PortableEmissionError::ArithmeticOverflow)
    })?;
    let identities = table(view, 1, 0)?;
    let expected = offset
        .checked_add(table(view, 2, 23)?.row_count())
        .ok_or(PortableEmissionError::ArithmeticOverflow)?;
    if identities.row_count() != expected {
        return Err(MISMATCH);
    }
    // 仅跳过一次前序种类；之后逐行与策略配对，跨 chunk 保持同一个游标。
    Ok(identities.rows().skip(offset as usize))
}

fn check_policy_primary(
    key: LocationKey<'_>,
    identity: Row<'_>,
    source: &ValidatedSourceMapInput,
) -> Result<(), PortableEmissionError> {
    let RegistryCheckedFieldValue::RecordVector(fields) =
        identity.field_by_tag(4).ok_or(MISMATCH)?.value()?
    else {
        return Err(MISMATCH);
    };
    let kind = number(identity, 1)? as u16;
    let mut namespace = None;
    let mut local = None;
    let mut parents = [None; 3];
    // Identity 的字段顺序就是 namespace、祖先 key、local key；此处读取真实前像。
    let field_count = fields.len() as usize;
    for (i, field) in fields.rows().enumerate() {
        let value = core::str::from_utf8(field.field_by_tag(2).ok_or(MISMATCH)?.value_bytes())
            .map_err(|_| MISMATCH)?;
        if i == 0 {
            namespace = Some(value);
        } else if i + 1 == field_count {
            local = Some(value);
        } else {
            *parents.get_mut(i - 1).ok_or(MISMATCH)? = Some(value);
        }
    }
    let module = source
        .source_modules()
        .nth(key.module as usize)
        .ok_or(MISMATCH)?;
    if Some(module.authoring_namespace_id()) != namespace || kind != 24 {
        return Err(MISMATCH);
    }
    if key.kind == 0 {
        return Ok(());
    }
    if key.namespace != namespace
        || key.entity != Some(kind)
        || key.key != local
        || key.parents != parents
        || key.subject != Some(2)
        || key.owner.is_some()
        || key.relation.is_some()
        || key.occurrence.is_some()
        || key.index.is_some()
        || key.steps != [None; 4]
    {
        return Err(MISMATCH);
    }
    Ok(())
}

fn check_projection(
    source: crate::PolicySourceView<'_>,
    row: Row<'_>,
    primary_tag: u16,
    locations: &[LocationKey<'_>],
    documents: &Documents<'_>,
    scratch: &mut Scratch,
) -> Result<(), PortableEmissionError> {
    let primary = source_key(source.primary_source(), documents)?;
    if locations.get(number(row, primary_tag)? as usize) != Some(&primary) {
        return Err(MISMATCH);
    }
    let mut expected = reserved::<u32>(source.contributing_sources().len(), scratch)?;
    for view in source.contributing_sources() {
        let key = source_key(view, documents)?;
        expected.push(locations.binary_search(&key).map_err(|_| MISMATCH)? as u32);
    }
    expected.sort_unstable();
    expected.dedup();
    let actual = ordinals(row, primary_tag + 1)?;
    if actual.len() as usize != expected.len()
        || expected
            .iter()
            .enumerate()
            .any(|(i, v)| actual.get(i as u32) != Some(*v))
    {
        return Err(MISMATCH);
    }
    scratch.release((expected.capacity() * size_of::<u32>()) as u64);
    Ok(())
}

fn check_road_fields(
    source: crate::PolicySourceView<'_>,
    primary: LocationKey<'_>,
    row: Row<'_>,
    kind: Option<u8>,
    documents: &Documents<'_>,
) -> Result<(), PortableEmissionError> {
    if primary.kind == 0 {
        return Ok(());
    }
    let mut expected = [None; 7];
    let count = if let Some(kind) = kind {
        let mut count = 0;
        // key 是来源 field 0；LFCA owner 不在成员贡献集合，tag 2.. 恰映射 field 0..。
        for field in row.fields().filter(|f| f.tag() >= 2) {
            expected[count] = Some([
                Some((0, 40, u16::from(kind) + 2)),
                Some((0, u16::from(kind) + 41, field.tag() - 2)),
                None,
                None,
            ]);
            count += 1;
        }
        count
    } else {
        expected[0] = Some([Some((0, 40, 0)), None, None, None]);
        expected[1] = Some([Some((0, 40, 1)), Some((0, 30, 0)), None, None]);
        expected[2] = Some([Some((0, 40, 1)), Some((0, 30, 1)), None, None]);
        if row.field_by_tag(5).is_some() {
            expected[3] = Some([Some((0, 40, 1)), Some((0, 30, 2)), None, None]);
            4
        } else {
            3
        }
    };
    let mut seen = [false; 7];
    for view in source.contributing_sources() {
        let field = source_key(view, documents)?;
        let index = expected[..count]
            .iter()
            .position(|steps| *steps == Some(field.steps))
            .ok_or(MISMATCH)?;
        let mut address = field;
        address.steps = primary.steps;
        if address != primary {
            return Err(MISMATCH);
        }
        seen[index] = true;
    }
    if seen[..count].iter().any(|v| !v) {
        return Err(MISMATCH);
    }
    Ok(())
}

struct Documents<'a> {
    // entries 保持真实全局 ordinal，by_key 只保存排序索引，不重排或复制文档键。
    entries: Vec<(&'a str, u32)>,
    by_key: Vec<u32>,
}

impl Documents<'_> {
    fn resolve(&self, key: &str) -> Result<(u32, u32), PortableEmissionError> {
        let index = self
            .by_key
            .binary_search_by_key(&key, |i| self.entries[*i as usize].0)
            .map_err(|_| MISMATCH)?;
        let document = self.by_key[index];
        Ok((self.entries[document as usize].1, document))
    }
}

fn check_documents<'a>(
    source: &'a ValidatedSourceMapInput,
    view: RegistryCheckedObjectView<'_>,
    scratch: &mut Scratch,
) -> Result<Documents<'a>, PortableEmissionError> {
    let module_table = table(view, 1, 0)?;
    if module_table.row_count() as usize != source.source_modules().len() {
        return Err(MISMATCH);
    }
    for (i, (module, row)) in source.source_modules().zip(module_table.rows()).enumerate() {
        if number(row, 1)? != i as u64
            || text(row, 2)? != module.authoring_namespace_id()
            || number(row, 3)? != u64::from(source_language_code(module.source_language()))
            || row.field_by_tag(4).ok_or(MISMATCH)?.value_bytes()
                != module.source_document_set_digest()
            || number(row, 5)? != u64::from(module.source_document_set_digest_version())
            || number(row, 6)? != u64::from(module.frontend_version())
            || row.field_by_tag(7).ok_or(MISMATCH)?.value_bytes()
                != module.frontend_options_digest()
            || text(row, 8)? != module.generator_build_id()
            || row.field_by_tag(9).ok_or(MISMATCH)?.value_bytes()
                != module.parameters_and_inputs_digest()
            || row.field_by_tag(10).map(|_| number(row, 10)).transpose()? != module.random_seed()
            || text(row, 11)? != module.provenance()
        {
            return Err(MISMATCH);
        }
        let RegistryCheckedFieldValue::RecordVector(imports) =
            row.field_by_tag(12).ok_or(MISMATCH)?.value()?
        else {
            return Err(MISMATCH);
        };
        if imports.len() as usize != module.imports().len() {
            return Err(MISMATCH);
        }
        for (actual, expected) in imports.rows().zip(module.imports()) {
            if text(actual, 1)? != expected {
                return Err(MISMATCH);
            }
        }
    }
    let mut documents = reserved::<(&str, u32)>(source.source_documents().len(), scratch)?;
    let table = table(view, 1, 1)?;
    if table.row_count() as usize != source.source_documents().len() {
        return Err(MISMATCH);
    }
    // 受检输入的文档按模块拓扑顺序连续登记；每个模块最多消费一次。
    let mut modules = source.source_modules().enumerate().peekable();
    for (i, (doc, row)) in source.source_documents().zip(table.rows()).enumerate() {
        while modules
            .peek()
            .is_some_and(|(_, m)| m.authoring_namespace_id() != doc.authoring_namespace_id())
        {
            modules.next();
        }
        let module = modules.peek().ok_or(MISMATCH)?.0 as u32;
        if number(row, 1)? != i as u64
            || number(row, 2)? != u64::from(module)
            || text(row, 3)? != doc.source_document_key()
            || row.field_by_tag(4).ok_or(MISMATCH)?.value_bytes() != doc.source_document_digest()
            || number(row, 5)? != u64::from(doc.source_record_byte_len())
            || row.field_by_tag(6).map(|_| text(row, 6)).transpose()?
                != doc.origin().display_source()
        {
            return Err(MISMATCH);
        }
        documents.push((doc.source_document_key(), module));
    }
    let mut by_key = reserved::<u32>(documents.len(), scratch)?;
    by_key.extend(0..u32::try_from(documents.len()).map_err(|_| MISMATCH)?);
    by_key.sort_unstable_by_key(|i| documents[*i as usize].0);
    if by_key
        .windows(2)
        .any(|w| documents[w[0] as usize].0 == documents[w[1] as usize].0)
    {
        return Err(MISMATCH);
    }
    Ok(Documents {
        entries: documents,
        by_key,
    })
}

#[derive(Default)]
struct ProjectionRowCounts {
    policies: usize,
    members: usize,
    movements: usize,
    movement_contributions: bool,
}

fn check_pool(
    view: RegistryCheckedObjectView<'_>,
    location_count: usize,
    scratch: &mut Scratch,
) -> Result<ProjectionRowCounts, PortableEmissionError> {
    let mut used = reserved::<bool>(location_count, scratch)?;
    used.resize(location_count, false);
    let mut counts = ProjectionRowCounts::default();
    let mut previous_stable = None;
    for (section, table_id, scalar, vector) in [
        (1, 0, Some(13), None),
        (2, 0, Some(4), Some(5)),
        (3, 0, Some(5), Some(6)),
        (3, 1, Some(8), None),
        (4, 0, None, Some(7)),
    ] {
        for row in table(view, section, table_id)?.rows() {
            // 与位置引用复用一次扫描；全局键跨 chunk 连续，保证后续二分的有序与唯一性。
            let movement = match (section, table_id) {
                (2, 0) => {
                    let kind = number(row, 1)?;
                    let key = (kind, checked_stable_id_with(row, 2, MISMATCH)?);
                    if previous_stable.is_some_and(|previous| previous >= key) {
                        return Err(MISMATCH);
                    }
                    previous_stable = Some(key);
                    counts.policies += usize::from(kind == 24);
                    counts.movements += usize::from(kind == 6);
                    kind == 6
                }
                (3, 0) => {
                    counts.members += usize::from(number(row, 1)? == 24);
                    false
                }
                _ => false,
            };
            if let Some(tag) = scalar {
                *used.get_mut(number(row, tag)? as usize).ok_or(MISMATCH)? = true;
            }
            if let Some(tag) = vector {
                let values = ordinals(row, tag)?;
                counts.movement_contributions |= movement && !values.is_empty();
                for i in 0..values.len() {
                    *used
                        .get_mut(values.get(i).ok_or(MISMATCH)? as usize)
                        .ok_or(MISMATCH)? = true;
                }
            }
        }
    }
    if used.iter().any(|v| !v) {
        return Err(MISMATCH);
    }
    scratch.release(used.capacity() as u64);
    Ok(counts)
}
