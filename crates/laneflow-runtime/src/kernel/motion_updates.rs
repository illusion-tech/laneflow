//! P5 直接写下一运动列，P6 按需组装逻辑值，P7 发布同一份列载荷（ADR 0031）。

use super::vehicle_store::{BLOCK_ROWS, MotionBlock, VehicleStore};
use crate::{StepError, VehicleState, VehicleStatus};

#[derive(Clone, Copy, Debug)]
struct UpdateRow {
    slot: usize,
    physical: usize,
}

#[derive(Clone, Copy, Debug)]
struct ControlUpdate {
    logical_index: usize,
    status: VehicleStatus,
    maneuver: Option<crate::ManeuverTraversalState>,
    waiting: Option<crate::WaitingMembership>,
}

/// P5 回报仅存检查结果和稀疏副作用标志，数值唯一写在 next.motion。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum MotionCheckpoint {
    #[default]
    Prelude,
    Parking,
    Waiting,
    Conflict,
    Calculation,
    Arrival,
    Complete,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MotionRowReport {
    /// 独立于物理行的规范位置，零表示没有处理该行。
    pub(crate) canonical_rank: u32,
    /// 同一车辆内已经到达的检查位置；较早失败永不被后续数值结果覆盖。
    pub(crate) checkpoint: MotionCheckpoint,
    pub(crate) done: bool,
    pub(crate) error: Option<StepError>,
    pub(crate) completed: bool,
    pub(crate) arrival: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MotionUpdates {
    pub(crate) motion: Vec<MotionBlock>,
    pub(crate) reports: Vec<MotionRowReport>,
    order: Vec<UpdateRow>,
    control: Vec<ControlUpdate>,
    control_by_row: Vec<u32>,
    published: bool,
}

impl MotionUpdates {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self::try_with_capacity(capacity).expect("next motion storage allocation")
    }

    pub(crate) fn try_with_capacity(
        capacity: usize,
    ) -> Result<Self, std::collections::TryReserveError> {
        let blocks = capacity.div_ceil(BLOCK_ROWS);
        let mut updates = Self::default();
        updates.motion.try_reserve_exact(blocks)?;
        for block in 0..blocks {
            updates.motion.push(MotionBlock::try_with_rows(
                (capacity - block * BLOCK_ROWS).min(BLOCK_ROWS),
            )?);
        }
        Ok(updates)
    }

    /// 回报与规范消费暂存仅为本拍真实物理跨度/活动数准备；空世界不保留整表。
    /// 分配在任何 P5 行求值前完成，失败不会发布下一列或消费真实资源。
    pub(crate) fn try_prepare_rows(
        &mut self,
        extent: usize,
        active: usize,
    ) -> Result<(), StepError> {
        self.reports
            .try_reserve(extent.saturating_sub(self.reports.len()))
            .map_err(|_| StepError::VehicleStorageAllocFailed)?;
        self.control_by_row
            .try_reserve(extent.saturating_sub(self.control_by_row.len()))
            .map_err(|_| StepError::VehicleStorageAllocFailed)?;
        self.order
            .try_reserve(active.saturating_sub(self.order.len()))
            .map_err(|_| StepError::VehicleStorageAllocFailed)?;
        self.reports.resize(extent, MotionRowReport::default());
        self.control_by_row.resize(extent, 0);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn from_states(states: &[(usize, VehicleState)], current: &VehicleStore) -> Self {
        let mut updates = Self::with_capacity(current.capacity());
        updates
            .try_prepare_rows(current.active_extent(), states.len())
            .unwrap();
        for &(slot, state) in states {
            assert_eq!(slot, state.handle.index() as usize);
            updates.push(state, current);
        }
        updates
    }

    pub(crate) fn clear(&mut self) {
        for changed in &self.control {
            self.control_by_row[self.order[changed.logical_index].physical] = 0;
        }
        self.order.clear();
        self.control.clear();
        for block in &mut self.motion {
            block.valid.fill(0);
        }
        self.published = false;
    }

    pub(crate) fn len(&self) -> usize {
        self.order.len()
    }

    #[cfg(test)]
    pub(crate) fn push(&mut self, state: VehicleState, current: &VehicleStore) {
        let physical = current
            .active_row(state.handle)
            .expect("next state has active predecessor");
        let index = self.order.len();
        self.order.push(UpdateRow {
            slot: state.handle.index() as usize,
            physical,
        });
        self.set(index, state, current)
            .expect("test next control allocation");
    }

    pub(crate) fn adopt(
        &mut self,
        state: VehicleState,
        completed: bool,
        current: &VehicleStore,
    ) -> Result<(), StepError> {
        let physical = current
            .active_row(state.handle)
            .expect("column result has active predecessor");
        let index = self.order.len();
        self.order.push(UpdateRow {
            slot: state.handle.index() as usize,
            physical,
        });
        self.motion[physical / BLOCK_ROWS].valid[physical % BLOCK_ROWS / 64] |=
            1 << (physical % 64);
        if completed {
            let old = current
                .active_control(state.handle.index() as usize)
                .expect("next state has active predecessor");
            self.set_control(
                index,
                VehicleStatus::Completed,
                old.maneuver,
                old.waiting,
                current,
            )?;
        }
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn get(&self, index: usize, current: &VehicleStore) -> (usize, VehicleState) {
        let row = self.order[index];
        let state = if self.published {
            current
                .slot(row.slot)
                .state
                .expect("next state has live predecessor")
        } else {
            let block = &self.motion[row.physical / BLOCK_ROWS];
            let offset = row.physical % BLOCK_ROWS;
            assert_ne!(
                block.valid[offset / 64] & (1 << (offset % 64)),
                0,
                "next row initialized"
            );
            let mut state = current
                .active_with_motion(row.slot, row.physical, block)
                .expect("next state has active predecessor");
            if let Some(index) = self.control_by_row[row.physical].checked_sub(1) {
                let control = self.control[index as usize];
                state.status = control.status;
                state.maneuver_traversal = control.maneuver;
                state.waiting_membership = control.waiting;
            }
            state
        };
        (row.slot, state)
    }

    pub(crate) fn try_get(
        &self,
        index: usize,
        current: &VehicleStore,
    ) -> Option<(usize, VehicleState)> {
        (index < self.len()).then(|| self.get(index, current))
    }

    pub(crate) fn iter<'a>(
        &'a self,
        current: &'a VehicleStore,
    ) -> impl Iterator<Item = (usize, VehicleState)> + 'a {
        (0..self.len()).map(|index| self.get(index, current))
    }

    /// 规范位置映射不读取或物化下一状态列。
    pub(crate) fn slot_index(&self, index: usize) -> usize {
        self.order[index].slot
    }

    pub(crate) fn slot_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.order.iter().map(|row| row.slot)
    }

    pub(crate) fn staged_route_cursor(&self, index: usize) -> u32 {
        assert!(!self.published, "next motion has not been published");
        let row = self.order[index];
        self.motion[row.physical / BLOCK_ROWS].route_cursor[row.physical % BLOCK_ROWS]
    }

    pub(crate) fn changed_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.control.iter().map(|changed| changed.logical_index)
    }

    /// 只排序实际控制变化，保留 P7 稀疏资源变更的规范消费顺序。
    pub(crate) fn freeze_controls(&mut self) {
        self.control
            .sort_unstable_by_key(|changed| changed.logical_index);
        for (index, changed) in self.control.iter().enumerate() {
            self.control_by_row[self.order[changed.logical_index].physical] =
                u32::try_from(index + 1).expect("control capacity fits u32");
        }
    }

    #[cfg(test)]
    pub(crate) fn set(
        &mut self,
        index: usize,
        state: VehicleState,
        current: &VehicleStore,
    ) -> Result<(), StepError> {
        let row = self.order[index];
        let block = &mut self.motion[row.physical / BLOCK_ROWS];
        let offset = row.physical % BLOCK_ROWS;
        block.route_cursor[offset] = state.route_edge_index;
        block.progress_mm[offset] = state.progress_mm;
        block.speed_mm_s[offset] = state.speed_mm_s;
        block.carry_um[offset] = state.carry_um;
        block.valid[offset / 64] |= 1 << (offset % 64);
        self.set_control(
            index,
            state.status,
            state.maneuver_traversal,
            state.waiting_membership,
            current,
        )
    }

    /// 控制收尾只写稀疏变化，不重写 P5 已完成的四列数值结果。
    pub(crate) fn set_control(
        &mut self,
        index: usize,
        status: VehicleStatus,
        maneuver: Option<crate::ManeuverTraversalState>,
        waiting: Option<crate::WaitingMembership>,
        current: &VehicleStore,
    ) -> Result<(), StepError> {
        let row = self.order[index];
        let old = current
            .active_control(row.slot)
            .expect("next state has active predecessor");
        let changed =
            status != VehicleStatus::Active || waiting != old.waiting || maneuver != old.maneuver;
        let replacement = ControlUpdate {
            logical_index: index,
            status,
            maneuver,
            waiting,
        };
        match self.control_by_row[row.physical].checked_sub(1) {
            Some(index) => self.control[index as usize] = replacement,
            None if changed => {
                self.control
                    .try_reserve(1)
                    .map_err(|_| StepError::VehicleStorageAllocFailed)?;
                self.control_by_row[row.physical] =
                    u32::try_from(self.control.len() + 1).expect("control capacity fits u32");
                self.control.push(replacement);
            }
            None => {}
        }
        Ok(())
    }

    pub(crate) fn validate(&self, current: &VehicleStore) -> Result<(), StepError> {
        if self.motion.len() != current.motion.len() {
            return Err(StepError::ConflictInvariantViolation);
        }
        for (source, next) in current.motion.iter().zip(&self.motion) {
            for (source, next) in source.valid.iter().zip(next.valid) {
                if *source != next {
                    return Err(StepError::ConflictInvariantViolation);
                }
            }
        }
        let rows: usize = current
            .motion
            .iter()
            .flat_map(|block| block.valid)
            .map(|word| word.count_ones() as usize)
            .sum();
        if rows != self.order.len()
            || self.order.iter().any(|row| {
                current
                    .slot(row.slot)
                    .state
                    .and_then(|state| current.active_row(state.handle))
                    != Some(row.physical)
            })
        {
            return Err(StepError::ConflictInvariantViolation);
        }
        Ok(())
    }

    pub(crate) fn publish(&mut self, current: &mut VehicleStore) {
        // 目录/活动列在安装时预留，稀疏冷池在 P6 按实际变化预留；这里不增长容量。
        for control_index in 0..self.control.len() {
            let index = self.control[control_index].logical_index;
            let row = self.order[index];
            let (_, state) = self.get(index, current);
            if state.status == VehicleStatus::Active {
                current.apply_control(state);
            } else {
                let generation = current.slot(row.slot).generation;
                current.set(
                    row.slot,
                    super::tables::VehicleSlot {
                        generation,
                        state: Some(state),
                    },
                );
                self.motion[row.physical / BLOCK_ROWS].valid[row.physical % BLOCK_ROWS / 64] &=
                    !(1 << (row.physical % 64));
            }
        }
        std::mem::swap(&mut current.motion, &mut self.motion);
        self.published = true;
    }

    /// P6 最后准备发布所需的实际冷记录，不把稀疏池按最大车辆容量预留。
    pub(crate) fn prepare_storage(&self, current: &mut VehicleStore) -> Result<(), StepError> {
        let mut inactive = 0;
        let mut control = 0;
        for changed in &self.control {
            if changed.status != VehicleStatus::Active {
                inactive += 1;
            } else if changed.maneuver.is_some() || changed.waiting.is_some() {
                let (_, state) = self.get(changed.logical_index, current);
                if !current.has_control(state.handle) {
                    control += 1;
                }
            }
        }
        current
            .try_prepare_pools(inactive, control)
            .map_err(|_| StepError::VehicleStorageAllocFailed)
    }

    #[cfg(test)]
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        use super::state::vec_bytes;
        vec_bytes(&self.motion)
            + vec_bytes(&self.reports)
            + vec_bytes(&self.order)
            + vec_bytes(&self.control)
            + vec_bytes(&self.control_by_row)
            + self
                .motion
                .iter()
                .map(MotionBlock::retained_columns_bytes)
                .sum::<u64>()
    }
}
