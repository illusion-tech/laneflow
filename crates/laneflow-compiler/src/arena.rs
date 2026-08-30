//! 编译阶段私有的有类型连续存储基础设施。
//!
//! 本模块把阶段内对象追加到 `Vec<T>`，以致密 `u32` 键和半开区间引用对象或关系表。
//! 键的类型参数只阻止不同表、不同阶段的句柄被意外混用；它不携带分配器身份，也不做
//! 代际校验。因此键只能与创建它的同一次编译、同一张表一起使用，不能进入 LIR、制品
//! 或跨编译缓存。表只追加且不删除，使插入顺序与致密下标保持一致，并允许阶段结束时
//! 一次性冻结为连续只读切片。

use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;

/// 单次编译阶段内有效的有类型 `u32` 区块分配键。
///
/// `K` 区分逻辑表，但不会证明键来自某个特定 [`TypedArena`]。调用者必须保证键没有
/// 跨 arena、跨阶段或跨编译复用。
#[repr(transparent)]
pub(crate) struct ArenaKey<K> {
    raw: u32,
    marker: PhantomData<fn() -> K>,
}

impl<K> ArenaKey<K> {
    /// 从已经过表边界或构造流程验证的原始序号恢复阶段键。
    pub(crate) const fn from_raw(raw: u32) -> Self {
        Self {
            raw,
            marker: PhantomData,
        }
    }

    /// 返回阶段内的原始致密序号；该值不是持久标识。
    pub(crate) const fn raw(self) -> u32 {
        self.raw
    }

    /// 转为当前进程可索引切片的下标。
    ///
    /// # Panics
    ///
    /// 仅当编译目标的 `usize` 无法容纳 `u32` 时 panic；LaneFlow 支持的目标均满足该
    /// 条件。该方法不检查键是否属于随后访问的 arena。
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
///
/// 成功 `push` 返回的键等于元素的插入下标；本类型没有删除或重排操作，因此该关系在
/// arena 的整个生命周期内保持成立。`into_boxed_slice` 会保留同一顺序。
pub(crate) struct TypedArena<K, T> {
    values: Vec<T>,
    marker: PhantomData<fn() -> K>,
}

impl<K, T> TypedArena<K, T> {
    /// 以调用者预检过的容量建立空 arena；容量本身不改变可用键范围。
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            values: Vec::with_capacity(capacity),
            marker: PhantomData,
        }
    }

    /// 追加一个值并返回其阶段内致密键。
    ///
    /// # Errors
    ///
    /// 若追加后的长度无法用 `u32` 表示，则返回 [`ArenaKeyOverflow`]，且不会插入值。
    pub(crate) fn push(&mut self, value: T) -> Result<ArenaKey<K>, ArenaKeyOverflow> {
        let next_len = self.values.len().checked_add(1).ok_or(ArenaKeyOverflow)?;
        u32::try_from(next_len).map_err(|_| ArenaKeyOverflow)?;
        let raw = u32::try_from(self.values.len()).map_err(|_| ArenaKeyOverflow)?;
        self.values.push(value);
        Ok(ArenaKey::from_raw(raw))
    }

    /// 读取由本 arena 产生的键所指向的值。
    ///
    /// # Panics
    ///
    /// 若调用者传入其他 arena 的键或越界原始键，则索引切片时 panic。
    pub(crate) fn get(&self, key: ArenaKey<K>) -> &T {
        &self.values[key.index()]
    }

    /// 可变读取由本 arena 产生的键所指向的值。
    ///
    /// # Panics
    ///
    /// 若调用者传入其他 arena 的键或越界原始键，则索引切片时 panic。
    pub(crate) fn get_mut(&mut self, key: ArenaKey<K>) -> &mut T {
        &mut self.values[key.index()]
    }

    /// 返回已成功插入的元素数。
    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }

    /// 按致密键顺序借用全部值；不暴露底层 `Vec` 的增长或可变能力。
    pub(crate) fn as_slice(&self) -> &[T] {
        &self.values
    }

    /// 按插入顺序遍历全部值，并重建与当前 arena 对应的致密键。
    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = (ArenaKey<K>, &T)> {
        self.values.iter().enumerate().map(|(index, value)| {
            let raw = u32::try_from(index).expect("arena rejected every non-u32 insertion index");
            (ArenaKey::from_raw(raw), value)
        })
    }

    /// 冻结连续值表；返回切片中的下标仍与已签发键的原始序号一致。
    pub(crate) fn into_boxed_slice(self) -> Box<[T]> {
        self.values.into_boxed_slice()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// 阶段表的下一个键或半开区间末端无法表示为 `u32`。
pub(crate) struct ArenaKeyOverflow;

/// 连续阶段表中的有类型 `u32` 半开区间。
///
/// `start` 与 `len` 都以元素数计；构造时同时验证排他末端不超过 `u32::MAX`。与
/// [`ArenaKey`] 相同，类型参数只能避免表类型混用，调用者仍须把区间用于创建它的那张
/// 连续表。
pub(crate) struct TableRange<T> {
    start: u32,
    len: u32,
    marker: PhantomData<fn() -> T>,
}

impl<T> TableRange<T> {
    /// 建立不引用任何记录的规范空区间。
    pub(crate) const fn empty() -> Self {
        Self {
            start: 0,
            len: 0,
            marker: PhantomData,
        }
    }

    /// 从切片下标与元素数建立半开区间。
    ///
    /// # Errors
    ///
    /// 当 `start + len` 发生算术溢出或排他末端不能表示为 `u32` 时返回
    /// [`ArenaKeyOverflow`]。
    pub(crate) fn try_from_usize(start: usize, len: usize) -> Result<Self, ArenaKeyOverflow> {
        let end = start.checked_add(len).ok_or(ArenaKeyOverflow)?;
        u32::try_from(end).map_err(|_| ArenaKeyOverflow)?;
        Ok(Self {
            start: u32::try_from(start).map_err(|_| ArenaKeyOverflow)?,
            len: u32::try_from(len).map_err(|_| ArenaKeyOverflow)?,
            marker: PhantomData,
        })
    }

    /// 返回半开区间的起始元素序号。
    pub(crate) const fn start(self) -> u32 {
        self.start
    }

    /// 返回区间包含的元素数。
    pub(crate) const fn len(self) -> u32 {
        self.len
    }

    /// 判断区间是否不包含任何元素。
    pub(crate) const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// 转换为可直接索引对应连续表的 Rust 半开区间。
    ///
    /// # Panics
    ///
    /// 仅当编译目标的 `usize` 无法容纳已验证的 `u32` 边界时 panic。该方法不检查
    /// 区间是否落在随后访问的具体切片长度内。
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

impl<T> PartialEq for TableRange<T> {
    fn eq(&self, other: &Self) -> bool {
        self.start == other.start && self.len == other.len
    }
}

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
