//! 正式研究子进程的安全受控分配容器。
//!
//! 计时角色只维护停止护栏必需的存续请求字节与峰值；归因角色才允许在此基础上累计
//! 分配、重分配和释放明细。所有规模相关容器只能通过本模块的可失败 `try_*` 接口
//! 增长。每次增长先预占完整的新请求容量，成功后提交该逻辑容量并释放旧额度；底层
//! 分配器即使暴露额外 capacity，也不能在下一次逻辑增长时绕过额度请求。因此硬上限
//! 拒绝发生在系统分配前，且不需要违反仓库的 safe-Rust 契约。

use std::cell::Cell;
use std::fmt;
use std::mem::size_of;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlledAllocationSnapshot {
    pub allocation_count: u64,
    pub reallocation_count: u64,
    pub allocated_bytes: u64,
    pub reallocated_bytes: u64,
    pub freed_bytes: u64,
    pub live_requested_bytes: u64,
    pub peak_live_requested_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ControlledAllocationRejection {
    pub(crate) field: &'static str,
    pub(crate) hard_ceiling_bytes: u64,
    pub(crate) live_requested_bytes: u64,
    pub(crate) requested_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ControlledAllocationObservation {
    pub(crate) live_requested_bytes: u64,
    pub(crate) peak_live_requested_bytes: u64,
}

#[derive(Debug)]
struct ControlledAllocatorState {
    hard_ceiling_bytes: u64,
    track_allocations: bool,
    live_requested_bytes: Cell<u64>,
    peak_live_requested_bytes: Cell<u64>,
    allocation_count: Cell<u64>,
    reallocation_count: Cell<u64>,
    allocated_bytes: Cell<u64>,
    reallocated_bytes: Cell<u64>,
    freed_bytes: Cell<u64>,
}

/// 只在当前研究子进程单线程内克隆和使用的受控分配状态。
///
/// `Rc<Cell<_>>` 是有意选择：正式研究管线是单线程确定性执行，避免为每个容量请求引入
/// 跨线程原子同步；该类型因此也不能被意外发送到并行工作线程。
#[derive(Clone, Debug)]
pub(crate) struct ControlledAllocator {
    state: Rc<ControlledAllocatorState>,
}

impl ControlledAllocator {
    pub(crate) fn new(hard_ceiling_bytes: u64) -> Self {
        Self::new_for_mode(hard_ceiling_bytes, false)
    }

    pub(crate) fn new_for_mode(hard_ceiling_bytes: u64, track_allocations: bool) -> Self {
        Self {
            state: Rc::new(ControlledAllocatorState {
                hard_ceiling_bytes,
                track_allocations,
                live_requested_bytes: Cell::new(0),
                peak_live_requested_bytes: Cell::new(0),
                allocation_count: Cell::new(0),
                reallocation_count: Cell::new(0),
                allocated_bytes: Cell::new(0),
                reallocated_bytes: Cell::new(0),
                freed_bytes: Cell::new(0),
            }),
        }
    }

    pub(crate) fn observation(&self) -> ControlledAllocationObservation {
        ControlledAllocationObservation {
            live_requested_bytes: self.state.live_requested_bytes.get(),
            peak_live_requested_bytes: self.state.peak_live_requested_bytes.get(),
        }
    }

    pub(crate) fn snapshot(&self) -> ControlledAllocationSnapshot {
        ControlledAllocationSnapshot {
            allocation_count: self.state.allocation_count.get(),
            reallocation_count: self.state.reallocation_count.get(),
            allocated_bytes: self.state.allocated_bytes.get(),
            reallocated_bytes: self.state.reallocated_bytes.get(),
            freed_bytes: self.state.freed_bytes.get(),
            live_requested_bytes: self.state.live_requested_bytes.get(),
            peak_live_requested_bytes: self.state.peak_live_requested_bytes.get(),
        }
    }

    pub(crate) fn begin_request(&self) -> Result<(), crate::StageGenerationError> {
        self.state
            .peak_live_requested_bytes
            .set(self.state.live_requested_bytes.get());
        Ok(())
    }

    pub(crate) fn preoccupy(
        &self,
        field: &'static str,
        requested_bytes: u64,
    ) -> Result<(), ControlledVecError> {
        let live_requested_bytes = self.state.live_requested_bytes.get();
        let next = live_requested_bytes
            .checked_add(requested_bytes)
            .filter(|next| *next <= self.state.hard_ceiling_bytes);
        let Some(next) = next else {
            let rejection = ControlledAllocationRejection {
                field,
                hard_ceiling_bytes: self.state.hard_ceiling_bytes,
                live_requested_bytes,
                requested_bytes,
            };
            return Err(ControlledVecError::HardCeiling(rejection));
        };
        self.state.live_requested_bytes.set(next);
        self.state
            .peak_live_requested_bytes
            .set(self.state.peak_live_requested_bytes.get().max(next));
        Ok(())
    }

    pub(crate) fn cancel_preoccupation(&self, requested_bytes: u64) {
        let previous = self.state.live_requested_bytes.get();
        debug_assert!(previous >= requested_bytes);
        self.state
            .live_requested_bytes
            .set(previous - requested_bytes);
    }

    pub(crate) fn commit_replacement(&self, previous_requested_bytes: u64, requested_bytes: u64) {
        self.cancel_preoccupation(previous_requested_bytes);
        if !self.state.track_allocations {
            return;
        }
        if previous_requested_bytes == 0 {
            self.state
                .allocation_count
                .set(self.state.allocation_count.get() + 1);
            self.state
                .allocated_bytes
                .set(self.state.allocated_bytes.get() + requested_bytes);
        } else {
            self.state
                .reallocation_count
                .set(self.state.reallocation_count.get() + 1);
            self.state
                .reallocated_bytes
                .set(self.state.reallocated_bytes.get() + requested_bytes);
            self.state
                .freed_bytes
                .set(self.state.freed_bytes.get() + previous_requested_bytes);
        }
    }

    pub(crate) fn release_allocation(&self, requested_bytes: u64) {
        self.cancel_preoccupation(requested_bytes);
        if self.state.track_allocations && requested_bytes != 0 {
            self.state
                .freed_bytes
                .set(self.state.freed_bytes.get() + requested_bytes);
        }
    }

    pub(crate) fn hard_ceiling_bytes(&self) -> u64 {
        self.state.hard_ceiling_bytes
    }
}

/// 只能通过可失败增长接口扩容的受控向量。
pub(crate) struct ControlledVec<T> {
    field: &'static str,
    allocator: ControlledAllocator,
    values: Vec<T>,
    requested_capacity: usize,
    accounted_capacity_bytes: u64,
}

impl<T> ControlledVec<T> {
    pub(crate) fn new(field: &'static str, allocator: ControlledAllocator) -> Self {
        Self {
            field,
            allocator,
            values: Vec::new(),
            requested_capacity: 0,
            accounted_capacity_bytes: 0,
        }
    }

    pub(crate) fn try_with_capacity(
        field: &'static str,
        capacity: usize,
        allocator: ControlledAllocator,
    ) -> Result<Self, ControlledVecError> {
        let mut values = Self::new(field, allocator);
        values.try_reserve_exact_capacity(capacity)?;
        Ok(values)
    }

    #[cfg(test)]
    pub(crate) fn allocator(&self) -> ControlledAllocator {
        self.allocator.clone()
    }

    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }

    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        self.requested_capacity
    }

    pub(crate) fn accounted_capacity_bytes(&self) -> u64 {
        self.accounted_capacity_bytes
    }

    pub(crate) fn as_slice(&self) -> &[T] {
        self.values.as_slice()
    }

    pub(crate) fn as_mut_slice(&mut self) -> &mut [T] {
        self.values.as_mut_slice()
    }

    pub(crate) fn iter(&self) -> std::slice::Iter<'_, T> {
        self.values.iter()
    }

    pub(crate) fn clear(&mut self) {
        self.values.clear();
    }

    pub(crate) fn try_push(&mut self, value: T) -> Result<(), ControlledVecError> {
        self.try_reserve(1)?;
        self.values.push(value);
        Ok(())
    }

    pub(crate) fn try_reserve(&mut self, additional: usize) -> Result<(), ControlledVecError> {
        let required_capacity = self
            .len()
            .checked_add(additional)
            .ok_or(ControlledVecError::CapacityOverflow { field: self.field })?;
        if required_capacity <= self.requested_capacity {
            return Ok(());
        }
        self.try_reserve_exact_capacity(required_capacity)
    }

    pub(crate) fn sort_unstable_by<F>(&mut self, compare: F)
    where
        F: FnMut(&T, &T) -> std::cmp::Ordering,
    {
        self.values.sort_unstable_by(compare);
    }

    pub(crate) fn sort_unstable_by_key<K, F>(&mut self, key: F)
    where
        F: FnMut(&T) -> K,
        K: Ord,
    {
        self.values.sort_unstable_by_key(key);
    }

    fn try_reserve_exact_capacity(
        &mut self,
        required_capacity: usize,
    ) -> Result<(), ControlledVecError> {
        if required_capacity <= self.requested_capacity {
            return Ok(());
        }
        let requested_bytes = capacity_bytes::<T>(required_capacity, self.field)?;
        self.allocator.preoccupy(self.field, requested_bytes)?;
        let additional = required_capacity
            .checked_sub(self.len())
            .ok_or(ControlledVecError::CapacityOverflow { field: self.field })?;
        if self.values.try_reserve_exact(additional).is_err() {
            self.allocator.cancel_preoccupation(requested_bytes);
            return Err(ControlledVecError::SystemAllocation { field: self.field });
        }

        self.allocator
            .commit_replacement(self.accounted_capacity_bytes, requested_bytes);
        self.requested_capacity = required_capacity;
        self.accounted_capacity_bytes = requested_bytes;
        Ok(())
    }
}

impl<T: Clone> ControlledVec<T> {
    pub(crate) fn try_extend_from_slice(&mut self, values: &[T]) -> Result<(), ControlledVecError> {
        self.try_reserve(values.len())?;
        self.values.extend_from_slice(values);
        Ok(())
    }

    pub(crate) fn try_resize(
        &mut self,
        new_len: usize,
        value: T,
    ) -> Result<(), ControlledVecError> {
        if new_len > self.len() {
            self.try_reserve(new_len - self.len())?;
        }
        self.values.resize(new_len, value);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn try_clone(
        &self,
        field: &'static str,
    ) -> Result<ControlledVec<T>, ControlledVecError> {
        let mut cloned = ControlledVec::try_with_capacity(field, self.len(), self.allocator())?;
        cloned.try_extend_from_slice(self.as_slice())?;
        Ok(cloned)
    }
}

impl<T> Drop for ControlledVec<T> {
    fn drop(&mut self) {
        self.allocator
            .release_allocation(self.accounted_capacity_bytes);
    }
}

impl<T: fmt::Debug> fmt::Debug for ControlledVec<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControlledVec")
            .field("field", &self.field)
            .field("values", &self.values)
            .finish()
    }
}

impl<T: PartialEq> PartialEq for ControlledVec<T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<T: Eq> Eq for ControlledVec<T> {}

impl<T> std::ops::Index<usize> for ControlledVec<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        &self.values[index]
    }
}

impl<T> std::ops::IndexMut<usize> for ControlledVec<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.values[index]
    }
}

impl<'a, T> IntoIterator for &'a ControlledVec<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ControlledVecError {
    HardCeiling(ControlledAllocationRejection),
    CapacityOverflow { field: &'static str },
    SystemAllocation { field: &'static str },
}

impl From<ControlledVecError> for crate::StageGenerationError {
    fn from(error: ControlledVecError) -> Self {
        match error {
            ControlledVecError::HardCeiling(rejection) => Self::ControlledAllocationHardCeiling {
                field: rejection.field,
                hard_ceiling_bytes: rejection.hard_ceiling_bytes,
                live_requested_bytes: rejection.live_requested_bytes,
                requested_bytes: rejection.requested_bytes,
            },
            ControlledVecError::CapacityOverflow { field } => {
                Self::ControlledAllocationCapacityOverflow(field)
            }
            ControlledVecError::SystemAllocation { field } => {
                Self::ControlledAllocationFailed(field)
            }
        }
    }
}

fn capacity_bytes<T>(capacity: usize, field: &'static str) -> Result<u64, ControlledVecError> {
    if size_of::<T>() == 0 {
        return Ok(0);
    }
    capacity
        .checked_mul(size_of::<T>())
        .and_then(|bytes| u64::try_from(bytes).ok())
        .ok_or(ControlledVecError::CapacityOverflow { field })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_ceiling_rejects_before_allocating_and_preserves_live_bytes() {
        let allocator = ControlledAllocator::new(8);
        let mut values =
            ControlledVec::<u32>::try_with_capacity("test values", 2, allocator.clone()).unwrap();
        assert_eq!(
            allocator.observation(),
            ControlledAllocationObservation {
                live_requested_bytes: 8,
                peak_live_requested_bytes: 8,
            }
        );

        values.try_push(1).unwrap();
        values.try_push(2).unwrap();
        let rejection = values.try_push(3).unwrap_err();
        assert_eq!(
            rejection,
            ControlledVecError::HardCeiling(ControlledAllocationRejection {
                field: "test values",
                hard_ceiling_bytes: 8,
                live_requested_bytes: 8,
                requested_bytes: 12,
            })
        );
        assert_eq!(values.len(), 2);
        assert_eq!(allocator.observation().live_requested_bytes, 8);
    }

    #[test]
    fn deallocation_releases_live_bytes_but_preserves_peak() {
        let allocator = ControlledAllocator::new(64);
        {
            let _values =
                ControlledVec::<u64>::try_with_capacity("test values", 4, allocator.clone())
                    .unwrap();
            assert_eq!(allocator.observation().live_requested_bytes, 32);
        }
        assert_eq!(
            allocator.observation(),
            ControlledAllocationObservation {
                live_requested_bytes: 0,
                peak_live_requested_bytes: 32,
            }
        );
    }

    #[test]
    fn clone_uses_same_ceiling_and_is_counted_as_simultaneously_live() {
        let allocator = ControlledAllocator::new(16);
        let mut original =
            ControlledVec::<u32>::try_with_capacity("original", 2, allocator.clone()).unwrap();
        original.try_push(1).unwrap();
        original.try_push(2).unwrap();
        let clone = original.try_clone("clone").unwrap();
        assert_eq!(clone.as_slice(), &[1, 2]);
        assert_eq!(
            allocator.observation(),
            ControlledAllocationObservation {
                live_requested_bytes: 16,
                peak_live_requested_bytes: 16,
            }
        );
    }

    #[test]
    fn failed_growth_keeps_original_capacity_and_accounting() {
        let allocator = ControlledAllocator::new(19);
        let mut values =
            ControlledVec::<u32>::try_with_capacity("values", 2, allocator.clone()).unwrap();
        values.try_push(1).unwrap();
        values.try_push(2).unwrap();
        assert!(matches!(
            values.try_push(3),
            Err(ControlledVecError::HardCeiling(_))
        ));
        assert_eq!(values.capacity(), 2);
        assert_eq!(allocator.observation().live_requested_bytes, 8);
    }
}
