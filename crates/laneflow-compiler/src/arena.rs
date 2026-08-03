use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;

/// 单次编译阶段内有效的有类型 `u32` 区块分配键。
#[repr(transparent)]
pub(crate) struct ArenaKey<K> {
    raw: u32,
    marker: PhantomData<fn() -> K>,
}

impl<K> ArenaKey<K> {
    pub(crate) const fn from_raw(raw: u32) -> Self {
        Self {
            raw,
            marker: PhantomData,
        }
    }

    pub(crate) const fn raw(self) -> u32 {
        self.raw
    }

    pub(crate) fn index(self) -> usize {
        usize::try_from(self.raw).expect("u32 arena key must fit usize on supported targets")
    }
}

impl<K> Clone for ArenaKey<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K> Copy for ArenaKey<K> {}

impl<K> PartialEq for ArenaKey<K> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<K> Eq for ArenaKey<K> {}

impl<K> PartialOrd for ArenaKey<K> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<K> Ord for ArenaKey<K> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.raw.cmp(&other.raw)
    }
}

impl<K> Hash for ArenaKey<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

impl<K> fmt::Debug for ArenaKey<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ArenaKey({})", self.raw)
    }
}

/// 仅追加、连续存储的阶段私有有类型区块分配器。
pub(crate) struct TypedArena<K, T> {
    values: Vec<T>,
    marker: PhantomData<fn() -> K>,
}

impl<K, T> TypedArena<K, T> {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            values: Vec::with_capacity(capacity),
            marker: PhantomData,
        }
    }

    pub(crate) fn push(&mut self, value: T) -> Result<ArenaKey<K>, ArenaKeyOverflow> {
        let next_len = self.values.len().checked_add(1).ok_or(ArenaKeyOverflow)?;
        u32::try_from(next_len).map_err(|_| ArenaKeyOverflow)?;
        let raw = u32::try_from(self.values.len()).map_err(|_| ArenaKeyOverflow)?;
        self.values.push(value);
        Ok(ArenaKey::from_raw(raw))
    }

    pub(crate) fn get(&self, key: ArenaKey<K>) -> &T {
        &self.values[key.index()]
    }

    pub(crate) fn get_mut(&mut self, key: ArenaKey<K>) -> &mut T {
        &mut self.values[key.index()]
    }

    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }

    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = (ArenaKey<K>, &T)> {
        self.values.iter().enumerate().map(|(index, value)| {
            let raw = u32::try_from(index).expect("arena rejected every non-u32 insertion index");
            (ArenaKey::from_raw(raw), value)
        })
    }

    pub(crate) fn into_boxed_slice(self) -> Box<[T]> {
        self.values.into_boxed_slice()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ArenaKeyOverflow;

/// 连续阶段表中的有类型 `u32` 区间。
pub(crate) struct TableRange<T> {
    start: u32,
    len: u32,
    marker: PhantomData<fn() -> T>,
}

impl<T> TableRange<T> {
    pub(crate) const fn empty() -> Self {
        Self {
            start: 0,
            len: 0,
            marker: PhantomData,
        }
    }

    pub(crate) fn try_from_usize(start: usize, len: usize) -> Result<Self, ArenaKeyOverflow> {
        let end = start.checked_add(len).ok_or(ArenaKeyOverflow)?;
        u32::try_from(end).map_err(|_| ArenaKeyOverflow)?;
        Ok(Self {
            start: u32::try_from(start).map_err(|_| ArenaKeyOverflow)?,
            len: u32::try_from(len).map_err(|_| ArenaKeyOverflow)?,
            marker: PhantomData,
        })
    }

    pub(crate) const fn start(self) -> u32 {
        self.start
    }

    pub(crate) const fn len(self) -> u32 {
        self.len
    }

    pub(crate) const fn is_empty(self) -> bool {
        self.len == 0
    }

    pub(crate) fn as_usize_range(self) -> core::ops::Range<usize> {
        let start = usize::try_from(self.start)
            .expect("u32 table offset must fit usize on supported targets");
        let len = usize::try_from(self.len)
            .expect("u32 table length must fit usize on supported targets");
        start..start + len
    }
}

impl<T> Clone for TableRange<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for TableRange<T> {}

impl<T> fmt::Debug for TableRange<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TableRange")
            .field("start", &self.start)
            .field("len", &self.len)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    enum LeftTag {}
    enum RightTag {}

    #[test]
    fn keys_are_dense_u32_and_arena_local_types_do_not_leak_into_layout() {
        assert_eq!(size_of::<ArenaKey<LeftTag>>(), size_of::<u32>());
        assert_eq!(size_of::<ArenaKey<RightTag>>(), size_of::<u32>());

        let mut arena = TypedArena::<LeftTag, _>::with_capacity(2);
        let first = arena.push("a").unwrap();
        let second = arena.push("b").unwrap();
        assert_eq!(first.raw(), 0);
        assert_eq!(second.raw(), 1);
        assert_eq!(arena.get(first), &"a");
        assert_eq!(
            arena.iter().map(|(key, _)| key.raw()).collect::<Vec<_>>(),
            [0, 1]
        );
    }

    #[test]
    fn table_range_rejects_an_exclusive_end_above_u32() {
        if usize::BITS > u32::BITS {
            assert!(matches!(
                TableRange::<()>::try_from_usize(u32::MAX as usize, 1),
                Err(ArenaKeyOverflow)
            ));
        }
    }
}
