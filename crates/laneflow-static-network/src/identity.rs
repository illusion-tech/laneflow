use core::mem::size_of;

use laneflow_static_contract::{
    EntityKind, EntityKindMarker, Ordinal, OrdinalKind, StableId, StableId128,
};

use crate::{BuildError, BuildStructure, EntityCounts};

const ENTITY_KIND_COUNT: usize = EntityKind::ALL.len();

/// StableId128 → typed ordinal 反向索引条目。
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
    /// 由已就序的正向表与反向条目组装索引；仅供 builder 调用。
    pub(crate) fn from_parts(
        forward: [Box<[StableId128]>; ENTITY_KIND_COUNT],
        reverse: Box<[IdentityReverseEntry]>,
    ) -> Self {
        Self { forward, reverse }
    }

    /// 按 typed ordinal 查询稳定标识；越界时返回 None。
    #[must_use]
    pub fn stable_id<K>(&self, ordinal: Ordinal<K>) -> Option<StableId<K>>
    where
        K: EntityKindMarker + OrdinalKind,
    {
        let raw = *self.forward[kind_index(K::KIND)].get(ordinal.index())?;
        Some(StableId::from_untyped(raw))
    }

    /// 按稳定标识查询 typed ordinal；未收录时返回 None。
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

    /// 返回指定实体种类的稳定实体数量。
    #[must_use]
    pub fn entity_count(&self, entity_kind: EntityKind) -> u32 {
        u32::try_from(self.forward[kind_index(entity_kind)].len())
            .expect("format-bounded identity count fits u32")
    }

    /// 返回本索引全部保留内存的逻辑字节数。
    #[must_use]
    pub fn retained_logical_bytes(&self) -> u64 {
        let forward_count = self.forward.iter().map(|items| items.len()).sum::<usize>();
        logical_bytes::<StableId128>(forward_count)
            + logical_bytes::<IdentityReverseEntry>(self.reverse.len())
    }
}

/// 按实体计数为各类正向身份表精确预分配容量。
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

/// 对反向身份条目做 LSD 基数排序：第 0-15 趟按稳定标识字节、第 16-17 趟按实体类别字节，
/// 最终按 (EntityKind, StableId128) 有序；第 15 趟后检测重复稳定标识。
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

/// 把各类正向身份 Vec 封存为 Box 切片。
pub(crate) fn seal_forward_identity(
    forward: [Vec<StableId128>; ENTITY_KIND_COUNT],
) -> [Box<[StableId128]>; ENTITY_KIND_COUNT] {
    forward.map(Vec::into_boxed_slice)
}

/// 返回实体种类在 ALL 数组中的下标。
pub(crate) const fn kind_index(entity_kind: EntityKind) -> usize {
    (entity_kind.code() - 1) as usize
}

/// 返回单条反向身份条目的字节大小。
#[allow(dead_code)]
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
