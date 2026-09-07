use super::policy_change::{Scratch, reserved};
use super::*;

/// 一份已索引实体的行视图及其规范身份字段前像。
#[derive(Clone, Copy, Debug)]
pub(super) struct EntityRecord<'a> {
    pub(super) row: RegistryCheckedRowView<'a>,
    canonical_fields: &'a [u8],
}

type IndexedEntity<'a> = ((EntityKind, [u8; 16]), EntityRecord<'a>);

/// 一份受检规范制品的实体索引。
///
/// 按实体种类与有类型序号顺序保存全部实体记录，另存稳定标识排序索引以支持二分查找。
pub(super) struct ArtifactIndex<'a> {
    pub(super) view: RegistryCheckedObjectView<'a>,
    // 原始 kind/ordinal 顺序支持 O(1) 引用；StableId 查询仅另存 u32 排序索引。
    entities: Vec<IndexedEntity<'a>>,
    by_stable_id: Vec<u32>,
    kind_ranges: [(usize, usize); EntityKind::ALL.len()],
}

impl<'a> ArtifactIndex<'a> {
    /// 从受检制品视图建立实体索引。
    ///
    /// 逐实体核对身份表与实体表的种类、有类型序号和稳定标识一致性，并拒绝重复稳定标识。
    pub(super) fn build(
        view: RegistryCheckedObjectView<'a>,
        mismatch: PortableEmissionError,
        scratch: &mut Scratch,
    ) -> Result<Self, PortableEmissionError> {
        let identity_table = view
            .section(1)
            .and_then(|section| section.table(0))
            .ok_or(mismatch)?;
        let entity_section = view.section(2).ok_or(mismatch)?;
        let count = identity_table.row_count() as usize;
        let entity_count = entity_section.tables().try_fold(0_usize, |n, t| {
            n.checked_add(t.row_count() as usize)
                .ok_or(PortableEmissionError::ArithmeticOverflow)
        })?;
        if entity_count != count {
            return Err(mismatch);
        }
        let mut entities = reserved::<IndexedEntity<'a>>(count, scratch)?;
        let mut by_stable_id = reserved::<u32>(count, scratch)?;
        let mut kind_ranges = [(0, 0); EntityKind::ALL.len()];
        let mut identities = identity_table.rows();
        let mut entity_tables = entity_section.tables();
        for entity_kind in EntityKind::ALL {
            if !entity_kind.is_constructible() {
                continue;
            }
            let entity_table = entity_tables.next().ok_or(mismatch)?;
            let start = entities.len();
            for (ordinal, entity) in entity_table.rows().enumerate() {
                let identity = identities.next().ok_or(mismatch)?;
                let stable_id = checked_stable_id_with(entity, 2, mismatch)?;
                if checked_u32_with(entity, 1, mismatch)? as usize != ordinal
                    || checked_u16_with(identity, 1, mismatch)? != entity_kind.code()
                    || checked_u32_with(identity, 2, mismatch)? as usize != ordinal
                    || checked_stable_id_with(identity, 3, mismatch)? != stable_id
                {
                    return Err(mismatch);
                }
                entities.push((
                    (entity_kind, stable_id),
                    EntityRecord {
                        row: entity,
                        canonical_fields: identity.field_by_tag(4).ok_or(mismatch)?.value_bytes(),
                    },
                ));
            }
            kind_ranges[usize::from(entity_kind.code() - 1)] = (start, entities.len());
        }
        if entity_tables.next().is_some() || identities.next().is_some() {
            return Err(mismatch);
        }
        by_stable_id.extend(0..identity_table.row_count());
        by_stable_id.sort_unstable_by_key(|i| entities[*i as usize].0.1);
        if by_stable_id
            .windows(2)
            .any(|w| entities[w[0] as usize].0.1 == entities[w[1] as usize].0.1)
        {
            return Err(mismatch);
        }

        Ok(Self {
            view,
            entities,
            by_stable_id,
            kind_ranges,
        })
    }

    /// 按实体种类与有类型序号顺序遍历全部已索引实体。
    pub(super) fn entities(
        &self,
    ) -> impl Iterator<Item = (&(EntityKind, [u8; 16]), &EntityRecord<'a>)> {
        self.entities.iter().map(|(key, value)| (key, value))
    }

    fn find(&self, id: [u8; 16]) -> Option<&IndexedEntity<'a>> {
        self.by_stable_id
            .binary_search_by_key(&id, |i| self.entities[*i as usize].0.1)
            .ok()
            .map(|i| &self.entities[self.by_stable_id[i] as usize])
    }

    /// 按实体种类与稳定标识精确查找实体记录。
    pub(super) fn entity(&self, key: &(EntityKind, [u8; 16])) -> Option<&EntityRecord<'a>> {
        self.find(key.1)
            .filter(|(actual, _)| actual == key)
            .map(|(_, record)| record)
    }

    fn ordinal_entity(&self, kind: EntityKind, ordinal: u32) -> Option<&IndexedEntity<'a>> {
        let (start, end) = self.kind_ranges[usize::from(kind.code() - 1)];
        self.entities[start..end].get(ordinal as usize)
    }

    /// 把实体种类与有类型序号解析为稳定标识；序号越界时返回给定的错误。
    pub(super) fn stable_id(
        &self,
        entity_kind: EntityKind,
        typed_ordinal: u32,
        mismatch: PortableEmissionError,
    ) -> Result<[u8; 16], PortableEmissionError> {
        self.ordinal_entity(entity_kind, typed_ordinal)
            .map(|(key, _)| key.1)
            .ok_or(mismatch)
    }

    /// 把实体种类与有类型序号解析为实体行视图；序号越界时返回给定的错误。
    pub(super) fn entity_row(
        &self,
        entity_kind: EntityKind,
        typed_ordinal: u32,
        mismatch: PortableEmissionError,
    ) -> Result<RegistryCheckedRowView<'a>, PortableEmissionError> {
        self.ordinal_entity(entity_kind, typed_ordinal)
            .map(|(_, entity)| entity.row)
            .ok_or(mismatch)
    }
}

/// 读取指定标签的 `U8` 字段；字段缺失或类型不符时返回给定的错误。
pub(super) fn checked_u8_with(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<u8, PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(mismatch)?.value()? {
        RegistryCheckedFieldValue::U8(value) => Ok(value),
        _ => Err(mismatch),
    }
}

/// 读取指定标签的 `U16` 字段；字段缺失或类型不符时返回给定的错误。
pub(super) fn checked_u16_with(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<u16, PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(mismatch)?.value()? {
        RegistryCheckedFieldValue::U16(value) => Ok(value),
        _ => Err(mismatch),
    }
}

/// 读取指定标签的 `U32` 字段；字段缺失或类型不符时返回给定的错误。
pub(in crate::portable_emitter) fn checked_u32_with(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<u32, PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(mismatch)?.value()? {
        RegistryCheckedFieldValue::U32(value) => Ok(value),
        _ => Err(mismatch),
    }
}

/// 读取指定标签的 `StableId128` 字段；字段缺失或类型不符时返回给定的错误。
pub(in crate::portable_emitter) fn checked_stable_id_with(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<[u8; 16], PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(mismatch)?.value()? {
        RegistryCheckedFieldValue::StableId128(value) => Ok(value.into_bytes()),
        _ => Err(mismatch),
    }
}

/// 读取指定标签的有类型序号向量字段；字段缺失或类型不符时返回给定的错误。
pub(super) fn checked_ordinal_vector_with(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<RegistryCheckedOrdinalVectorView<'_>, PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(mismatch)?.value()? {
        RegistryCheckedFieldValue::OrdinalVectorU32(value) => Ok(value),
        _ => Err(mismatch),
    }
}

/// 读取指定标签的记录向量字段；字段缺失或类型不符时返回给定的错误。
pub(super) fn checked_record_vector_with(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<RegistryCheckedRecordVectorView<'_>, PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(mismatch)?.value()? {
        RegistryCheckedFieldValue::RecordVector(value) => Ok(value),
        _ => Err(mismatch),
    }
}

/// 读取指定节唯一表的首行；节、表或行缺失时返回给定的错误。
pub(super) fn singleton_row(
    view: RegistryCheckedObjectView<'_>,
    section_ordinal: u32,
    mismatch: PortableEmissionError,
) -> Result<RegistryCheckedRowView<'_>, PortableEmissionError> {
    view.section(section_ordinal)
        .and_then(|section| section.table(0))
        .and_then(|table| table.row(0))
        .ok_or(mismatch)
}

/// 核对基线与目标制品之间可以生成语义差异。
///
/// 两侧的静态契约版本与执行契约行必须逐字节相同；同一稳定标识跨修订不得改变
/// 实体种类或规范身份字段。
pub(super) fn verify_artifact_diff_compatibility(
    base: RegistryCheckedObjectView<'_>,
    target: RegistryCheckedObjectView<'_>,
    base_index: &ArtifactIndex<'_>,
    target_index: &ArtifactIndex<'_>,
) -> Result<(), PortableEmissionError> {
    let base_contract_versions =
        singleton_row(base, 0, PortableEmissionError::DiffBaseSemanticMismatch)?;
    let target_contract_versions =
        singleton_row(target, 0, PortableEmissionError::InternalBindingMismatch)?;
    let base_execution_contract =
        singleton_row(base, 5, PortableEmissionError::DiffBaseSemanticMismatch)?;
    let target_execution_contract =
        singleton_row(target, 5, PortableEmissionError::InternalBindingMismatch)?;
    if base_contract_versions.bytes() != target_contract_versions.bytes()
        || base_execution_contract.bytes() != target_execution_contract.bytes()
    {
        return Err(PortableEmissionError::UnsupportedSemanticContractTransition);
    }
    for ((base_kind, stable_id), base_identity) in base_index.entities() {
        if let Some(((target_kind, _), target_identity)) = target_index.find(*stable_id)
            && (base_kind != target_kind
                || base_identity.canonical_fields != target_identity.canonical_fields)
        {
            return Err(PortableEmissionError::CrossRevisionStableIdCollision);
        }
    }
    Ok(())
}
