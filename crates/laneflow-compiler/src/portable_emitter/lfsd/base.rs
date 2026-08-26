use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
struct IdentityRecord {
    entity_kind: EntityKind,
    typed_ordinal: u32,
    canonical_fields: Box<[u8]>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct EntityRecord<'a> {
    pub(super) row: RegistryCheckedRowView<'a>,
}

pub(super) struct ArtifactIndex<'a> {
    pub(super) view: RegistryCheckedObjectView<'a>,
    identities: BTreeMap<[u8; 16], IdentityRecord>,
    pub(super) entities: BTreeMap<(EntityKind, [u8; 16]), EntityRecord<'a>>,
    ordinal_stable_ids: BTreeMap<(EntityKind, u32), [u8; 16]>,
}

impl<'a> ArtifactIndex<'a> {
    pub(super) fn build(
        view: RegistryCheckedObjectView<'a>,
        mismatch: PortableEmissionError,
    ) -> Result<Self, PortableEmissionError> {
        let identity_table = view
            .section(1)
            .and_then(|section| section.table(0))
            .ok_or(mismatch)?;
        let mut identities = BTreeMap::new();
        let mut identity_ordinals = BTreeMap::new();
        for identity in identity_table.rows() {
            let entity_kind =
                EntityKind::from_code(checked_u16_with(identity, 1, mismatch)?).ok_or(mismatch)?;
            let typed_ordinal = checked_u32_with(identity, 2, mismatch)?;
            let stable_id = checked_stable_id_with(identity, 3, mismatch)?;
            let canonical_fields = identity
                .field_by_tag(4)
                .ok_or(mismatch)?
                .value_bytes()
                .to_vec()
                .into_boxed_slice();
            if identities
                .insert(
                    stable_id,
                    IdentityRecord {
                        entity_kind,
                        typed_ordinal,
                        canonical_fields,
                    },
                )
                .is_some()
                || identity_ordinals
                    .insert((entity_kind, typed_ordinal), stable_id)
                    .is_some()
            {
                return Err(mismatch);
            }
        }

        let entity_section = view.section(2).ok_or(mismatch)?;
        let mut entities = BTreeMap::new();
        let mut ordinal_stable_ids = BTreeMap::new();
        let mut entity_tables = entity_section.tables();
        for entity_kind in EntityKind::ALL {
            if !entity_kind.is_constructible() {
                continue;
            }
            let entity_table = entity_tables.next().ok_or(mismatch)?;
            for entity in entity_table.rows() {
                let typed_ordinal = checked_u32_with(entity, 1, mismatch)?;
                let stable_id = checked_stable_id_with(entity, 2, mismatch)?;
                if entities
                    .insert((entity_kind, stable_id), EntityRecord { row: entity })
                    .is_some()
                    || ordinal_stable_ids
                        .insert((entity_kind, typed_ordinal), stable_id)
                        .is_some()
                {
                    return Err(mismatch);
                }
            }
        }
        if entity_tables.next().is_some() {
            return Err(mismatch);
        }
        if identities.len() != entities.len() {
            return Err(mismatch);
        }
        for (stable_id, identity) in &identities {
            if identity_ordinals.get(&(identity.entity_kind, identity.typed_ordinal))
                != Some(stable_id)
                || ordinal_stable_ids.get(&(identity.entity_kind, identity.typed_ordinal))
                    != Some(stable_id)
                || !entities.contains_key(&(identity.entity_kind, *stable_id))
            {
                return Err(mismatch);
            }
        }

        Ok(Self {
            view,
            identities,
            entities,
            ordinal_stable_ids,
        })
    }

    pub(super) fn stable_id(
        &self,
        entity_kind: EntityKind,
        typed_ordinal: u32,
        mismatch: PortableEmissionError,
    ) -> Result<[u8; 16], PortableEmissionError> {
        self.ordinal_stable_ids
            .get(&(entity_kind, typed_ordinal))
            .copied()
            .ok_or(mismatch)
    }
}

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

pub(super) fn checked_u32_with(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<u32, PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(mismatch)?.value()? {
        RegistryCheckedFieldValue::U32(value) => Ok(value),
        _ => Err(mismatch),
    }
}

pub(super) fn checked_stable_id_with(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<[u8; 16], PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(mismatch)?.value()? {
        RegistryCheckedFieldValue::StableId128(value) => Ok(value.into_bytes()),
        _ => Err(mismatch),
    }
}

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
    for (stable_id, base_identity) in &base_index.identities {
        if let Some(target_identity) = target_index.identities.get(stable_id)
            && (base_identity.entity_kind != target_identity.entity_kind
                || base_identity.canonical_fields != target_identity.canonical_fields)
        {
            return Err(PortableEmissionError::CrossRevisionStableIdCollision);
        }
    }
    Ok(())
}
