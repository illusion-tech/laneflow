//! 列式活动运动权威；逻辑值仅在查询/管理边界组装（ADR 0031）。

use crate::kernel::tables::VehicleSlot;
use crate::{RouteHandle, VehicleHandle, VehicleState, VehicleStatus};
use laneflow_static_contract::{ParticipantClassOrdinal, VehicleProfileOrdinal};
use std::collections::TryReserveError;
use std::ops::{Deref, DerefMut};

pub(crate) const BLOCK_ROWS: usize = 128;

#[cfg(test)]
thread_local! {
    pub(crate) static FORCE_COLD_ALLOCATION_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 预览和错误回报只携带会改变的运动值；稳定上下文仍来自本拍 Current。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MotionValue {
    pub(crate) route_edge_index: u32,
    pub(crate) progress_mm: u32,
    pub(crate) speed_mm_s: u32,
    pub(crate) carry_um: u16,
    pub(crate) status: VehicleStatus,
}

impl From<VehicleState> for MotionValue {
    fn from(state: VehicleState) -> Self {
        Self {
            route_edge_index: state.route_edge_index,
            progress_mm: state.progress_mm,
            speed_mm_s: state.speed_mm_s,
            carry_um: state.carry_um,
            status: state.status,
        }
    }
}

impl MotionValue {
    pub(crate) fn apply(self, mut state: VehicleState) -> VehicleState {
        state.route_edge_index = self.route_edge_index;
        state.progress_mm = self.progress_mm;
        state.speed_mm_s = self.speed_mm_s;
        state.carry_um = self.carry_um;
        state.status = self.status;
        state
    }
}

#[derive(Clone, Copy, Debug, Default)]
enum Location {
    Active(usize),
    Inactive(usize),
    #[default]
    Vacant,
}

#[derive(Clone, Copy, Debug, Default)]
struct DirectoryEntry {
    generation: u32,
    location: Location,
    /// 稀疏控制池的 index + 1，零表示常规车辆没有扩展控制记录。
    control: u32,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MotionBlock {
    pub(crate) route_cursor: Vec<u32>,
    pub(crate) progress_mm: Vec<u32>,
    pub(crate) speed_mm_s: Vec<u32>,
    pub(crate) carry_um: Vec<u16>,
    pub(crate) valid: [u64; BLOCK_ROWS / 64],
}

impl MotionBlock {
    pub(crate) fn try_with_rows(rows: usize) -> Result<Self, TryReserveError> {
        let mut block = Self::default();
        block.try_grow_rows(rows)?;
        Ok(block)
    }

    fn try_grow_rows(&mut self, rows: usize) -> Result<(), TryReserveError> {
        debug_assert!(rows <= BLOCK_ROWS);
        grow_column(&mut self.route_cursor, rows, 0)?;
        grow_column(&mut self.progress_mm, rows, 0)?;
        grow_column(&mut self.speed_mm_s, rows, 0)?;
        grow_column(&mut self.carry_um, rows, 0)
    }

    #[cfg(test)]
    pub(crate) fn retained_columns_bytes(&self) -> u64 {
        use crate::kernel::state::vec_bytes;
        vec_bytes(&self.route_cursor)
            + vec_bytes(&self.progress_mm)
            + vec_bytes(&self.speed_mm_s)
            + vec_bytes(&self.carry_um)
    }
}

fn grow_column<T: Clone>(column: &mut Vec<T>, rows: usize, zero: T) -> Result<(), TryReserveError> {
    column.try_reserve_exact(rows.saturating_sub(column.len()))?;
    column.resize(rows, zero);
    Ok(())
}

#[derive(Clone, Debug, Default)]
struct ContextBlock {
    owner: Vec<Option<VehicleHandle>>,
    route: Vec<Option<RouteHandle>>,
    profile: Vec<Option<VehicleProfileOrdinal>>,
    class: Vec<Option<ParticipantClassOrdinal>>,
    length_mm: Vec<u32>,
}

impl ContextBlock {
    fn try_with_rows(rows: usize) -> Result<Self, TryReserveError> {
        let mut block = Self::default();
        block.try_grow_rows(rows)?;
        Ok(block)
    }

    fn try_grow_rows(&mut self, rows: usize) -> Result<(), TryReserveError> {
        grow_column(&mut self.owner, rows, None)?;
        grow_column(&mut self.route, rows, None)?;
        grow_column(&mut self.profile, rows, None)?;
        grow_column(&mut self.class, rows, None)?;
        grow_column(&mut self.length_mm, rows, 0)
    }

    #[cfg(test)]
    fn retained_columns_bytes(&self) -> u64 {
        use crate::kernel::state::vec_bytes;
        vec_bytes(&self.owner)
            + vec_bytes(&self.route)
            + vec_bytes(&self.profile)
            + vec_bytes(&self.class)
            + vec_bytes(&self.length_mm)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ControlState {
    maneuver: Option<crate::ManeuverTraversalState>,
    waiting: Option<crate::WaitingMembership>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct VehicleStore {
    directory: Vec<DirectoryEntry>,
    pub(crate) motion: Vec<MotionBlock>,
    context: Vec<ContextBlock>,
    control: Vec<ControlState>,
    inactive: Vec<Option<VehicleState>>,
    free_active: Vec<u32>,
    free_inactive: Vec<usize>,
    free_control: Vec<usize>,
    capacity: usize,
}

impl VehicleStore {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        let mut store = Self::default();
        store
            .try_reserve_exact(capacity)
            .expect("vehicle storage allocation");
        store
    }

    pub(crate) fn try_reserve_exact(&mut self, additional: usize) -> Result<(), TryReserveError> {
        let capacity = self.len().saturating_add(additional);
        if capacity <= self.capacity {
            return Ok(());
        }
        self.directory.try_reserve_exact(capacity - self.len())?;
        self.free_active
            .try_reserve_exact(capacity - self.free_active.len())?;
        let blocks = capacity.div_ceil(BLOCK_ROWS);
        self.motion.try_reserve_exact(blocks - self.motion.len())?;
        self.context
            .try_reserve_exact(blocks - self.context.len())?;
        for block in 0..blocks {
            let rows = (capacity - block * BLOCK_ROWS).min(BLOCK_ROWS);
            if block < self.motion.len() {
                self.motion[block].try_grow_rows(rows)?;
                self.context[block].try_grow_rows(rows)?;
            } else {
                let motion = MotionBlock::try_with_rows(rows)?;
                let context = ContextBlock::try_with_rows(rows)?;
                self.motion.push(motion);
                self.context.push(context);
            }
        }
        self.free_active
            .extend((self.capacity..capacity).rev().map(|row| {
                u32::try_from(row).expect("physical row fits validated u32 vehicle capacity")
            }));
        self.capacity = capacity;
        Ok(())
    }

    /// 只增长真实生命周期/控制变化需要的池；失败不改变任何车辆或资源权威。
    pub(crate) fn try_prepare_pools(
        &mut self,
        inactive: usize,
        control: usize,
    ) -> Result<(), TryReserveError> {
        let inactive_growth = inactive.saturating_sub(self.free_inactive.len());
        let control_growth = control.saturating_sub(self.free_control.len());
        #[cfg(test)]
        if FORCE_COLD_ALLOCATION_FAILURE.get()
            && (inactive_growth > self.inactive.capacity() - self.inactive.len()
                || control_growth > self.control.capacity() - self.control.len())
        {
            // 容量溢出产生真实 TryReserveError，不让测试依赖机器是否恰好耗尽内存。
            Vec::<u8>::new().try_reserve(usize::MAX)?;
        }
        self.inactive.try_reserve(inactive_growth)?;
        self.free_inactive
            .try_reserve(self.inactive.len() + inactive_growth - self.free_inactive.len())?;
        self.control.try_reserve(control_growth)?;
        self.free_control
            .try_reserve(self.control.len() + control_growth - self.free_control.len())?;
        Ok(())
    }

    pub(crate) fn try_prepare_state(
        &mut self,
        index: usize,
        state: Option<VehicleState>,
    ) -> Result<(), TryReserveError> {
        let old = self.directory.get(index).copied().unwrap_or_default();
        let inactive = usize::from(
            state.is_some_and(|state| state.status != VehicleStatus::Active)
                && !matches!(old.location, Location::Inactive(_)),
        );
        let control = usize::from(
            state.is_some_and(|state| {
                state.status == VehicleStatus::Active
                    && (state.maneuver_traversal.is_some() || state.waiting_membership.is_some())
            }) && old.control == 0,
        );
        self.try_prepare_pools(inactive, control)
    }

    pub(crate) fn has_control(&self, handle: VehicleHandle) -> bool {
        self.directory
            .get(handle.index() as usize)
            .is_some_and(|entry| entry.generation == handle.generation() && entry.control != 0)
    }

    fn set_control(&mut self, index: usize, control: ControlState) {
        let old = self.directory[index].control.checked_sub(1);
        let present = control.maneuver.is_some() || control.waiting.is_some();
        match (old, present) {
            (Some(row), true) => self.control[row as usize] = control,
            (Some(row), false) => {
                self.control[row as usize] = ControlState::default();
                self.free_control.push(row as usize);
                self.directory[index].control = 0;
            }
            (None, true) => {
                let row = self.free_control.pop().unwrap_or_else(|| {
                    let row = self.control.len();
                    self.control.push(ControlState::default());
                    row
                });
                self.control[row] = control;
                self.directory[index].control =
                    u32::try_from(row + 1).expect("control row fits vehicle capacity");
            }
            (None, false) => {}
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.directory.len()
    }
    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }

    pub(crate) fn get(&self, index: usize) -> Option<VehicleSlot> {
        let entry = self.directory.get(index)?;
        let state = match entry.location {
            Location::Vacant => None,
            Location::Inactive(row) => self.inactive[row],
            Location::Active(position) => {
                let block = position / BLOCK_ROWS;
                let row = position % BLOCK_ROWS;
                let context = &self.context[block];
                let motion = &self.motion[block];
                let control = entry
                    .control
                    .checked_sub(1)
                    .map_or_else(ControlState::default, |row| self.control[row as usize]);
                Some(VehicleState {
                    handle: context.owner[row]?,
                    route: context.route[row]?,
                    profile: context.profile[row]?,
                    class: context.class[row]?,
                    length_mm: context.length_mm[row],
                    route_edge_index: motion.route_cursor[row],
                    progress_mm: motion.progress_mm[row],
                    speed_mm_s: motion.speed_mm_s[row],
                    carry_um: motion.carry_um[row],
                    status: VehicleStatus::Active,
                    maneuver_traversal: control.maneuver,
                    waiting_membership: control.waiting,
                })
            }
        };
        Some(VehicleSlot {
            generation: entry.generation,
            state,
        })
    }

    pub(crate) fn slot(&self, index: usize) -> VehicleSlot {
        self.get(index).expect("vehicle slot in range")
    }

    pub(crate) fn state(&self, handle: VehicleHandle) -> Option<VehicleState> {
        let entry = self.get(handle.index() as usize)?;
        (entry.generation == handle.generation())
            .then_some(entry.state)
            .flatten()
    }

    pub(crate) fn active_row(&self, handle: VehicleHandle) -> Option<usize> {
        let entry = self.directory.get(handle.index() as usize)?;
        if entry.generation != handle.generation() {
            return None;
        }
        match entry.location {
            Location::Active(row) => Some(row),
            _ => None,
        }
    }

    pub(crate) fn active_at(&self, physical: usize) -> Option<VehicleState> {
        let handle = *self
            .context
            .get(physical / BLOCK_ROWS)?
            .owner
            .get(physical % BLOCK_ROWS)?;
        let handle = handle?;
        (self.active_row(handle) == Some(physical))
            .then(|| self.state(handle))
            .flatten()
    }

    pub(crate) fn active_extent(&self) -> usize {
        self.motion
            .iter()
            .enumerate()
            .rev()
            .find_map(|(block, motion)| {
                motion
                    .valid
                    .iter()
                    .enumerate()
                    .rev()
                    .find_map(|(word, &bits)| {
                        (bits != 0).then(|| {
                            block * BLOCK_ROWS + word * 64 + 64 - bits.leading_zeros() as usize
                        })
                    })
            })
            .unwrap_or(0)
    }

    pub(crate) fn apply_control(&mut self, state: VehicleState) {
        let index = state.handle.index() as usize;
        self.set_control(
            index,
            ControlState {
                maneuver: state.maneuver_traversal,
                waiting: state.waiting_membership,
            },
        );
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = VehicleSlot> + '_ {
        (0..self.len()).map(|index| self.slot(index))
    }

    pub(crate) fn push(&mut self, slot: VehicleSlot) {
        if self.len() == self.capacity {
            self.try_reserve_exact(1)
                .expect("vehicle storage allocation");
        }
        let index = self.len();
        self.directory.push(DirectoryEntry::default());
        self.set(index, slot);
    }

    pub(crate) fn set(&mut self, index: usize, slot: VehicleSlot) {
        // P7/管理事务已提前准备；测试编辑或局部 staging 在这里先增长、再写状态。
        self.try_prepare_state(index, slot.state)
            .expect("vehicle cold pools preflighted");
        let old_control = self.directory[index].control;
        let previous = self.directory[index].location;
        let active = slot
            .state
            .is_some_and(|state| state.status == VehicleStatus::Active);
        let location = match (previous, active, slot.state.is_some()) {
            (Location::Active(row), true, _) => Location::Active(row),
            (Location::Inactive(row), false, true) => Location::Inactive(row),
            (old, _, _) => {
                match old {
                    Location::Active(position) => {
                        self.motion[position / BLOCK_ROWS].valid[position % BLOCK_ROWS / 64] &=
                            !(1 << (position % 64));
                        self.context[position / BLOCK_ROWS].owner[position % BLOCK_ROWS] = None;
                        self.free_active.push(
                            u32::try_from(position).expect("physical row fits vehicle capacity"),
                        );
                    }
                    Location::Inactive(row) => {
                        self.inactive[row] = None;
                        self.free_inactive.push(row);
                    }
                    Location::Vacant => {}
                }
                if active {
                    Location::Active(
                        self.free_active.pop().expect("active row preallocated") as usize
                    )
                } else if slot.state.is_some() {
                    let row = self.free_inactive.pop().unwrap_or_else(|| {
                        let row = self.inactive.len();
                        self.inactive.push(None);
                        row
                    });
                    Location::Inactive(row)
                } else {
                    Location::Vacant
                }
            }
        };
        self.directory[index] = DirectoryEntry {
            generation: slot.generation,
            location,
            control: old_control,
        };
        if !active {
            self.set_control(index, ControlState::default());
        }
        match (location, slot.state) {
            (Location::Active(position), Some(state)) => {
                let block = position / BLOCK_ROWS;
                let row = position % BLOCK_ROWS;
                let context = &mut self.context[block];
                // 稳定上下文只在发生变化时写入，不随普通运动发布重抄。
                if context.owner[row] != Some(state.handle)
                    || context.route[row] != Some(state.route)
                    || context.profile[row] != Some(state.profile)
                    || context.class[row] != Some(state.class)
                    || context.length_mm[row] != state.length_mm
                {
                    context.owner[row] = Some(state.handle);
                    context.route[row] = Some(state.route);
                    context.profile[row] = Some(state.profile);
                    context.class[row] = Some(state.class);
                    context.length_mm[row] = state.length_mm;
                }
                let motion = &mut self.motion[block];
                motion.route_cursor[row] = state.route_edge_index;
                motion.progress_mm[row] = state.progress_mm;
                motion.speed_mm_s[row] = state.speed_mm_s;
                motion.carry_um[row] = state.carry_um;
                motion.valid[row / 64] |= 1 << (row % 64);
                self.set_control(
                    index,
                    ControlState {
                        maneuver: state.maneuver_traversal,
                        waiting: state.waiting_membership,
                    },
                );
            }
            (Location::Inactive(row), state) => self.inactive[row] = state,
            (Location::Vacant, _) => {}
            _ => unreachable!("vehicle location matches status"),
        }
    }

    pub(crate) fn get_mut(&mut self, index: usize) -> Option<SlotEdit<'_>> {
        let value = self.get(index)?;
        Some(SlotEdit {
            store: self,
            index,
            value,
        })
    }

    pub(crate) fn slot_mut(&mut self, index: usize) -> SlotEdit<'_> {
        self.get_mut(index).expect("vehicle slot in range")
    }

    #[cfg(test)]
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        use crate::kernel::state::vec_bytes;
        vec_bytes(&self.directory)
            + vec_bytes(&self.motion)
            + vec_bytes(&self.context)
            + vec_bytes(&self.control)
            + vec_bytes(&self.inactive)
            + vec_bytes(&self.free_active)
            + vec_bytes(&self.free_inactive)
            + vec_bytes(&self.free_control)
            + self
                .motion
                .iter()
                .map(MotionBlock::retained_columns_bytes)
                .sum::<u64>()
            + self
                .context
                .iter()
                .map(ContextBlock::retained_columns_bytes)
                .sum::<u64>()
    }
}

/// 管理/测试边界的短期编辑值；无并发借用，离开作用域立即写回唯一列权威。
pub(crate) struct SlotEdit<'a> {
    store: &'a mut VehicleStore,
    index: usize,
    value: VehicleSlot,
}

impl Deref for SlotEdit<'_> {
    type Target = VehicleSlot;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}
impl DerefMut for SlotEdit<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}
impl Drop for SlotEdit<'_> {
    fn drop(&mut self) {
        self.store.set(self.index, self.value.clone());
    }
}

impl<'a> IntoIterator for &'a VehicleStore {
    type Item = VehicleSlot;
    type IntoIter = VehicleStoreIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        VehicleStoreIter {
            store: self,
            index: 0,
        }
    }
}

pub(crate) struct VehicleStoreIter<'a> {
    store: &'a VehicleStore,
    index: usize,
}
impl Iterator for VehicleStoreIter<'_> {
    type Item = VehicleSlot;
    fn next(&mut self) -> Option<VehicleSlot> {
        let slot = self.store.get(self.index)?;
        self.index += 1;
        Some(slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_pools_grow_for_actual_records_and_reuse_without_growth() {
        let world = crate::kernel::waiting::tests::multi_gate_world(1);
        let mut state = world.vehicle(world.live_vehicles()[0]).unwrap();
        state.maneuver_traversal = None;
        state.waiting_membership = None;
        let mut store = VehicleStore::with_capacity(1024);
        store.push(VehicleSlot {
            generation: state.handle.generation(),
            state: Some(state),
        });
        assert_eq!(store.control.capacity(), 0);
        assert_eq!(store.inactive.capacity(), 0);
        let physical = store.active_row(state.handle).unwrap();
        let control = crate::ManeuverTraversalState {
            route: state.route,
            maneuver_occurrence_index: 0,
            phase: crate::ManeuverTraversalPhase::PreGate { next_gate_hop: 0 },
        };
        state.maneuver_traversal = Some(control);
        store.try_prepare_state(0, Some(state)).unwrap();
        let reserved = store.retained_logical_bytes();
        store.set(
            0,
            VehicleSlot {
                generation: state.handle.generation(),
                state: Some(state),
            },
        );
        assert_eq!(store.retained_logical_bytes(), reserved);
        assert_eq!(store.control.len(), 1);
        assert_eq!(store.state(state.handle), Some(state));
        state.status = VehicleStatus::Parked;
        store.try_prepare_state(0, Some(state)).unwrap();
        let reserved = store.retained_logical_bytes();
        store.set(
            0,
            VehicleSlot {
                generation: state.handle.generation(),
                state: Some(state),
            },
        );
        assert_eq!(store.retained_logical_bytes(), reserved);
        assert!(store.active_row(state.handle).is_none());
        assert_eq!(store.free_control.len(), 1);
        assert_eq!(store.state(state.handle), Some(state));
        state.status = VehicleStatus::Active;
        store.try_prepare_state(0, Some(state)).unwrap();
        let reserved = store.retained_logical_bytes();
        store.set(
            0,
            VehicleSlot {
                generation: state.handle.generation(),
                state: Some(state),
            },
        );
        assert_eq!(store.retained_logical_bytes(), reserved);
        assert_eq!(store.active_row(state.handle), Some(physical));
        assert_eq!(store.control.len(), 1);
        assert_eq!(store.inactive.len(), 1);
        assert_eq!(store.state(state.handle), Some(state));
    }

    #[test]
    fn failed_cold_preflight_keeps_directory_columns_and_logical_state() {
        let world = crate::kernel::waiting::tests::multi_gate_world(1);
        let state = world.vehicle(world.live_vehicles()[0]).unwrap();
        let mut store = VehicleStore::with_capacity(128);
        store.push(VehicleSlot {
            generation: state.handle.generation(),
            state: Some(state),
        });
        let physical = store.active_row(state.handle);
        let mut parked = state;
        parked.status = VehicleStatus::Parked;
        FORCE_COLD_ALLOCATION_FAILURE.set(true);
        let result = store.try_prepare_state(0, Some(parked));
        FORCE_COLD_ALLOCATION_FAILURE.set(false);
        assert!(result.is_err());
        assert_eq!(store.state(state.handle), Some(state));
        assert_eq!(store.active_row(state.handle), physical);
        assert!(store.inactive.is_empty());
    }
}
