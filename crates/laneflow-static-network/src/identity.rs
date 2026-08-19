use core::mem::size_of;

use laneflow_static_contract::{
    EntityKind, EntityKindMarker, Ordinal, OrdinalKind, StableId, StableId128,
};

use crate::{BuildError, BuildStructure, EntityCounts};

const ENTITY_KIND_COUNT: usize = EntityKind::ALL.len();

#[derive(Clone, Copy)]
pub(crate) struct IdentityReverseEntry {
    pub(crate) entity_kind: EntityKind,
    pub(crate) stable_id: StableId128,
    pub(crate) ordinal: u32,
}

impl IdentityReverseEntry {
    const DUMMY: Self = Self {
        entity_kind: EntityKind::RoadCorridor,
        stable_id: StableId128::ZERO,
        ordinal: 0,
    };
}

/// 所有稳定实体的 typed ordinal ↔ StableId128 冷双向索引。
pub struct SharedIdentityIndex {
    forward: [Box<[StableId128]>; ENTITY_KIND_COUNT],
    reverse: Box<[IdentityReverseEntry]>,
}

impl SharedIdentityIndex {
    pub(crate) fn from_parts(
        forward: [Box<[StableId128]>; ENTITY_KIND_COUNT],
        reverse: Box<[IdentityReverseEntry]>,
    ) -> Self {
        Self { forward, reverse }
    }

    #[must_use]
    pub fn stable_id<K>(&self, ordinal: Ordinal<K>) -> Option<StableId<K>>
    where
        K: EntityKindMarker + OrdinalKind,
    {
        let raw = *self.forward[kind_index(K::KIND)].get(ordinal.index())?;
        Some(StableId::from_untyped(raw))
    }

    #[must_use]
    pub fn ordinal<K>(&self, stable_id: StableId<K>) -> Option<Ordinal<K>>
    where
        K: EntityKindMarker + OrdinalKind,
    {
        let raw = stable_id.into_untyped();
        let index = self
            .reverse
            .binary_search_by(|entry| (entry.entity_kind, entry.stable_id).cmp(&(K::KIND, raw)))
            .ok()?;
        Some(Ordinal::from_raw(self.reverse[index].ordinal))
    }

    #[must_use]
    pub fn entity_count(&self, entity_kind: EntityKind) -> u32 {
        u32::try_from(self.forward[kind_index(entity_kind)].len())
            .expect("format-bounded identity count fits u32")
    }

    #[must_use]
    pub fn retained_logical_bytes(&self) -> u64 {
        let forward_count = self.forward.iter().map(|items| items.len()).sum::<usize>();
        logical_bytes::<StableId128>(forward_count)
            + logical_bytes::<IdentityReverseEntry>(self.reverse.len())
    }
}

pub(crate) fn allocate_forward_identity(
    counts: &EntityCounts,
) -> Result<[Vec<StableId128>; ENTITY_KIND_COUNT], BuildError> {
    let mut failed = false;
    let result = core::array::from_fn(|index| {
        let entity_kind = EntityKind::ALL[index];
        let capacity = usize::try_from(counts.count(entity_kind)).expect("u32 count fits usize");
        let mut values = Vec::new();
        if values.try_reserve_exact(capacity).is_err() {
            failed = true;
        }
        values
    });
    if failed {
        return Err(BuildError::AllocationFailure {
            structure: BuildStructure::CanonicalIdentity,
        });
    }
    Ok(result)
}

pub(crate) fn radix_sort_reverse_identity(
    entries: Vec<IdentityReverseEntry>,
    mut check_cancelled: impl FnMut() -> Result<(), BuildError>,
) -> Result<Vec<IdentityReverseEntry>, BuildError> {
    check_cancelled()?;
    let mut source = entries;
    let mut target = Vec::new();
    target
        .try_reserve_exact(source.len())
        .map_err(|_| BuildError::AllocationFailure {
            structure: BuildStructure::BuilderScratch,
        })?;
    target.resize(source.len(), IdentityReverseEntry::DUMMY);

    for pass in 0..18_usize {
        check_cancelled()?;
        let mut counts = [0_usize; 256];
        for entry in &source {
            counts[usize::from(identity_key_byte(*entry, pass))] += 1;
        }
        let mut offsets = [0_usize; 256];
        let mut next = 0_usize;
        for (offset, count) in offsets.iter_mut().zip(counts) {
            *offset = next;
            next += count;
        }
        for entry in &source {
            let bucket = usize::from(identity_key_byte(*entry, pass));
            target[offsets[bucket]] = *entry;
            offsets[bucket] += 1;
        }
        core::mem::swap(&mut source, &mut target);

        if pass == 15 {
            for (index, pair) in source.windows(2).enumerate() {
                if index & 1_023 == 0 {
                    check_cancelled()?;
                }
                if pair[0].stable_id == pair[1].stable_id {
                    return Err(BuildError::DuplicateStableId {
                        stable_id: pair[0].stable_id,
                    });
                }
            }
        }
    }

    Ok(source)
}

pub(crate) fn seal_forward_identity(
    forward: [Vec<StableId128>; ENTITY_KIND_COUNT],
) -> [Box<[StableId128]>; ENTITY_KIND_COUNT] {
    forward.map(Vec::into_boxed_slice)
}

pub(crate) const fn kind_index(entity_kind: EntityKind) -> usize {
    (entity_kind.code() - 1) as usize
}

pub(crate) const fn reverse_entry_bytes() -> usize {
    size_of::<IdentityReverseEntry>()
}

fn identity_key_byte(entry: IdentityReverseEntry, pass: usize) -> u8 {
    match pass {
        0..=15 => entry.stable_id.as_bytes()[15 - pass],
        16 => entry.entity_kind.code().to_le_bytes()[0],
        17 => entry.entity_kind.code().to_le_bytes()[1],
        _ => unreachable!("identity radix pass is closed"),
    }
}

fn logical_bytes<T>(len: usize) -> u64 {
    u64::try_from(
        len.checked_mul(size_of::<T>())
            .expect("retained size fits usize"),
    )
    .expect("retained size fits u64")
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;

    use super::*;

    #[test]
    fn reverse_identity_sort_propagates_cancellation_between_passes() {
        let checks = Cell::new(0_u32);
        let result = radix_sort_reverse_identity(Vec::new(), || {
            let next = checks.get() + 1;
            checks.set(next);
            if next == 2 {
                Err(BuildError::Cancelled)
            } else {
                Ok(())
            }
        });

        assert!(matches!(result, Err(BuildError::Cancelled)));
        assert_eq!(checks.get(), 2);
    }
}
