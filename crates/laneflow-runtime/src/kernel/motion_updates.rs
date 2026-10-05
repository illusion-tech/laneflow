//! P5 直接写下一运动列，P6 按需组装逻辑值，P7 发布同一份列载荷（ADR 0031）。

use super::vehicle_store::{ActiveRow, BLOCK_ROWS, MotionBlock, MotionPosition, VehicleStore};
use crate::{RouteHandle, StepError, VehicleHandle, VehicleState, VehicleStatus};

#[derive(Clone, Copy, Debug)]
struct UpdateRow {
    slot: usize,
    physical: usize,
}

/// 下一拍 frontier 只需身份、游标与可达性字段，不读取控制成员或组装整车。
#[derive(Clone, Copy)]
pub(crate) struct FrontierMotionInput {
    pub(crate) vehicle: VehicleHandle,
    pub(crate) route: RouteHandle,
    pub(crate) cursor: u32,
    pub(crate) progress_mm: u32,
    pub(crate) speed_mm_s: u32,
    pub(crate) status: VehicleStatus,
}

impl From<&VehicleState> for FrontierMotionInput {
    fn from(state: &VehicleState) -> Self {
        Self {
            vehicle: state.handle,
            route: state.route,
            cursor: state.route_edge_index,
            progress_mm: state.progress_mm,
            speed_mm_s: state.speed_mm_s,
            status: state.status,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_rows_are_scratch_owned_and_keep_capacity_after_discard() {
        let mut updates = MotionUpdates::default();
        updates.try_prepare_rows(0, 0).unwrap();
        assert_eq!(updates.retained_logical_bytes(), 0);
        updates.try_prepare_rows(0, 17).unwrap();
        assert_eq!(
            updates.retained_logical_bytes(),
            super::super::state::vec_bytes(&updates.order)
                + super::super::state::vec_bytes(&updates.resource_rows)
        );
        assert!(updates.resource_rows.capacity() >= 17);
        let reserved = updates.retained_logical_bytes();
        updates.resource_rows.extend([0, 2, 4]);
        updates.clear();
        assert!(updates.resource_rows.is_empty());
        assert_eq!(updates.retained_logical_bytes(), reserved);
    }

    #[test]
    fn bound_updates_read_next_columns_and_controls_with_a_physical_hole() {
        let world = crate::kernel::waiting::tests::multi_gate_world(3);
        let mut current = world.state.committed.vehicles.clone();
        let first = world.vehicle(world.live_vehicles()[0]).unwrap();
        let last = world.vehicle(world.live_vehicles()[2]).unwrap();
        current
            .slot_mut(world.live_vehicles()[1].index() as usize)
            .state = None;
        let mut completed = first;
        completed.status = VehicleStatus::Completed;
        completed.maneuver_traversal = None;
        completed.waiting_membership = None;
        let mut moved = last;
        moved.progress_mm += 1;
        moved.speed_mm_s += 1;
        moved.carry_um = 999;
        let mut updates = MotionUpdates::from_states(
            &[
                (first.handle.index() as usize, completed),
                (last.handle.index() as usize, moved),
            ],
            &current,
        );
        assert_eq!(updates.validate(&current), Ok(()));
        let row = updates.row(0, &current);
        assert_eq!(row.source.handle(), first.handle);
        assert_eq!(row.source.position(), (&first).into());
        assert_eq!(row.status(), VehicleStatus::Completed);
        assert_eq!(row.control().waiting, None);
        assert_eq!(row.control().maneuver, None);
        assert_eq!(row.state(), completed);
        let row = updates.row(1, &current);
        assert_eq!(row.source.position(), (&last).into());
        assert_eq!(row.position(), (&moved).into());
        assert_eq!(row.state(), moved);
        assert_eq!(current.state(first.handle), Some(first));
        assert_eq!(current.state(last.handle), Some(last));
        assert_eq!(updates.active_len(), 1);
        let inputs: Vec<_> = (0..updates.len())
            .map(|index| updates.frontier_input(index, &current))
            .collect();
        assert_eq!(inputs[0].vehicle, completed.handle);
        assert_eq!(inputs[0].status, VehicleStatus::Completed);
        assert_eq!(inputs[1].route, moved.route);
        assert_eq!(inputs[1].cursor, moved.route_edge_index);
        assert_eq!(inputs[1].progress_mm, moved.progress_mm);
        assert_eq!(inputs[1].speed_mm_s, moved.speed_mm_s);
        // 同一行多次变化仍仅保留一个覆盖；恢复 Active 不重复扣数。
        for status in [VehicleStatus::Parked, VehicleStatus::Active] {
            updates
                .set_control(0, status, None, None, &current)
                .unwrap();
            assert_eq!(updates.control.len(), 1);
            assert_eq!(
                updates.active_len(),
                updates
                    .iter(&current)
                    .filter(|(_, state)| state.status == VehicleStatus::Active)
                    .count()
            );
        }
        updates.freeze_controls();
        assert_eq!(updates.active_len(), 2);
        updates.order[0].slot = last.handle.index() as usize;
        assert_eq!(
            updates.validate(&current),
            Err(StepError::ConflictInvariantViolation)
        );
    }
}

#[derive(Clone, Copy, Debug)]
struct ControlUpdate {
    logical_index: usize,
    status: VehicleStatus,
    maneuver: Option<crate::ManeuverTraversalState>,
    waiting: Option<crate::WaitingMembership>,
}

/// Current 身份绑定与 Next 数值借用；只在稀疏复杂消费者要求时组装完整逻辑值。
pub(crate) struct UpdateView<'a> {
    pub(crate) source: ActiveRow<'a>,
    next: &'a MotionBlock,
    control: Option<&'a ControlUpdate>,
}

impl UpdateView<'_> {
    pub(crate) fn position(&self) -> MotionPosition {
        self.source.position_in(self.next)
    }

    pub(crate) fn state(&self) -> VehicleState {
        let mut state = self.source.state_in(self.next);
        if let Some(control) = self.control {
            state.status = control.status;
            state.maneuver_traversal = control.maneuver;
            state.waiting_membership = control.waiting;
        }
        state
    }

    pub(crate) fn status(&self) -> VehicleStatus {
        self.control
            .map_or(VehicleStatus::Active, |control| control.status)
    }

    pub(crate) fn control(&self) -> super::vehicle_store::ControlState {
        self.control.map_or_else(
            || self.source.control(),
            |control| super::vehicle_store::ControlState {
                maneuver: control.maneuver,
                waiting: control.waiting,
            },
        )
    }
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
    pub(crate) finalize_hints: super::resource_rows::FinalizeHints,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MotionUpdates {
    pub(crate) motion: Vec<MotionBlock>,
    pub(crate) reports: Vec<MotionRowReport>,
    order: Vec<UpdateRow>,
    control: Vec<ControlUpdate>,
    control_by_row: Vec<u32>,
    /// P6 三个消费者共用的规范有序行号；每拍重建，不持有资源权威。
    resource_rows: Vec<usize>,
    published: bool,
}

/// 并行规范消费的段数上限；每段的统计与输出切片都放在栈上。
const ADOPT_MAX_PARTS: usize = 128;
/// 提交前绑定核对的并行段数上限。
const VALIDATE_MAX_PARTS: usize = 128;
/// 更新行少于此数时串行核对。
#[cfg(not(test))]
const VALIDATE_PARALLEL_ROWS: usize = 4_096;
#[cfg(test)]
const VALIDATE_PARALLEL_ROWS: usize = 1;

/// 并行规范消费的结果。
pub(crate) enum ParallelAdopt {
    /// 规范顺序中有已不在存储里的句柄，或稀疏缓冲预留失败，交回串行消费。
    Fallback,
    /// `first_bad` 是首个不合格行的 rank 与错误；`specials` 已按 rank 升序写入
    /// 早于它的到达或完成行，`order`、有效位与资源行也只覆盖早于它的前缀。
    Adopted {
        first_bad: Option<(usize, StepError)>,
    },
}

#[derive(Clone, Copy, Default)]
struct AdoptStats {
    first_bad: Option<(usize, StepError)>,
    skipped: bool,
    retain: usize,
    special: usize,
}

impl MotionUpdates {
    /// 按规范 rank 分段并行消费 P5 回报：worker 写本段的 `order` 行、校验回报并数出
    /// 资源保留行和稀疏的到达/完成行；协调器只求最小出错 rank，并按前缀和切开
    /// 互斥输出让第二遍并行写入。结果与串行逐行 `adopt` 完全相同；稀疏行留给
    /// 调用方按 rank 串行处理（到达观测和完成控制），再返回首错。
    pub(crate) fn adopt_parallel(
        &mut self,
        resources: &crate::kernel::execution::ExecutionResources,
        active_order: &[VehicleHandle],
        current: &VehicleStore,
        specials: &mut Vec<u32>,
    ) -> Result<ParallelAdopt, StepError> {
        let count = active_order.len();
        self.order.clear();
        self.order
            .try_reserve(count)
            .map_err(|_| StepError::VehicleStorageAllocFailed)?;
        self.order.resize(
            count,
            UpdateRow {
                slot: 0,
                physical: 0,
            },
        );
        let parts = resources
            .dispatch_threads()
            .saturating_mul(4)
            .min(ADOPT_MAX_PARTS)
            .min(count)
            .max(1);
        let span = count.div_ceil(parts).max(1);
        let reports = &self.reports;
        let mut stats = [AdoptStats::default(); ADOPT_MAX_PARTS];
        {
            let mut rest: &mut [UpdateRow] = &mut self.order;
            let mut work: [Option<(&mut [UpdateRow], &mut AdoptStats)>; ADOPT_MAX_PARTS] =
                std::array::from_fn(|_| None);
            for (slot, stat) in work.iter_mut().zip(stats.iter_mut()).take(parts) {
                let take = span.min(rest.len());
                let (rows, tail) = std::mem::take(&mut rest).split_at_mut(take);
                *slot = Some((rows, stat));
                rest = tail;
            }
            resources.for_each_part(&mut work[..parts], 1, |part, work| {
                let Some((rows, stat)) = work[0].as_mut() else {
                    return;
                };
                let start = part * span;
                for (offset, row) in rows.iter_mut().enumerate() {
                    let rank = start + offset;
                    let handle = active_order[rank];
                    if current.status(handle).is_none() {
                        stat.skipped = true;
                        return;
                    }
                    let Some(physical) = current.active_row(handle) else {
                        stat.first_bad = Some((rank, StepError::ConflictInvariantViolation));
                        return;
                    };
                    let report = reports[physical];
                    if !report.done || report.canonical_rank as usize != rank + 1 {
                        stat.first_bad = Some((rank, StepError::ConflictInvariantViolation));
                        return;
                    }
                    if let Some(error) = report.error {
                        stat.first_bad = Some((rank, error));
                        return;
                    }
                    if report.checkpoint != MotionCheckpoint::Complete {
                        stat.first_bad = Some((rank, StepError::ConflictInvariantViolation));
                        return;
                    }
                    *row = UpdateRow {
                        slot: handle.index() as usize,
                        physical,
                    };
                    stat.retain += usize::from(report.finalize_hints.retain());
                    stat.special += usize::from(report.arrival || report.completed);
                }
            });
        }
        if stats[..parts].iter().any(|stat| stat.skipped) {
            self.order.clear();
            return Ok(ParallelAdopt::Fallback);
        }
        // 段内遇错即停，段号即 rank 先后：第一个有错的段给出全局最小出错 rank。
        let bad_part = stats[..parts]
            .iter()
            .position(|stat| stat.first_bad.is_some());
        let first_bad = bad_part.and_then(|part| stats[part].first_bad);
        let used = bad_part.map_or(parts, |part| part + 1);
        let limit = first_bad.map_or(count, |(rank, _)| rank);
        let retain_total: usize = stats[..used].iter().map(|stat| stat.retain).sum();
        let special_total: usize = stats[..used].iter().map(|stat| stat.special).sum();
        // 稀疏缓冲是可选暂存：预留失败交回串行消费，不新增领域错误。
        specials.clear();
        if specials.try_reserve(special_total).is_err() {
            self.order.clear();
            return Ok(ParallelAdopt::Fallback);
        }
        specials.resize(special_total, 0);
        // 资源行容量已按 Active 数在 P5 前预留，这里不会再分配。
        self.resource_rows.clear();
        self.resource_rows
            .try_reserve(retain_total)
            .map_err(|_| StepError::VehicleStorageAllocFailed)?;
        self.resource_rows.resize(retain_total, 0);
        {
            let order = &self.order[..limit];
            let mut retain_rest: &mut [usize] = &mut self.resource_rows;
            let mut special_rest: &mut [u32] = specials;
            let mut work: [Option<(&mut [usize], &mut [u32])>; ADOPT_MAX_PARTS] =
                std::array::from_fn(|_| None);
            for (slot, stat) in work.iter_mut().zip(&stats[..used]) {
                let (retain, retain_tail) =
                    std::mem::take(&mut retain_rest).split_at_mut(stat.retain);
                let (special, special_tail) =
                    std::mem::take(&mut special_rest).split_at_mut(stat.special);
                *slot = Some((retain, special));
                retain_rest = retain_tail;
                special_rest = special_tail;
            }
            resources.for_each_part(&mut work[..used], 1, |part, work| {
                let Some((retain, special)) = work[0].as_mut() else {
                    return;
                };
                let start = (part * span).min(limit);
                let end = (start + span).min(limit);
                let (mut retain_at, mut special_at) = (0, 0);
                for (rank, row) in order[start..end].iter().enumerate() {
                    let rank = start + rank;
                    let report = reports[row.physical];
                    if report.finalize_hints.retain() {
                        retain[retain_at] = rank;
                        retain_at += 1;
                    }
                    if report.arrival || report.completed {
                        special[special_at] = u32::try_from(rank).expect("rank fits u32");
                        special_at += 1;
                    }
                }
                debug_assert_eq!((retain_at, special_at), (retain.len(), special.len()));
            });
        }
        // 有效位按物理块并行置位：已消费行正是 rank 落在前缀内且已完成的行。
        let reports = &self.reports;
        let blocks = self.motion.len();
        let block_chunk = blocks.div_ceil(parts).max(1);
        resources.for_each_part(&mut self.motion[..blocks], block_chunk, |chunk, blocks| {
            let first = chunk * block_chunk;
            for (offset, block) in blocks.iter_mut().enumerate() {
                let base = (first + offset) * BLOCK_ROWS;
                for row in 0..BLOCK_ROWS {
                    let Some(report) = reports.get(base + row) else {
                        break;
                    };
                    if report.done
                        && report.canonical_rank != 0
                        && report.canonical_rank as usize <= limit
                    {
                        block.valid[row / 64] |= 1 << (row % 64);
                    }
                }
            }
        });
        Ok(ParallelAdopt::Adopted { first_bad })
    }

    /// 并行消费后处理一个稀疏完成行：与 `adopt` 中完成分支相同的控制覆盖。
    pub(crate) fn adopt_completed(
        &mut self,
        rank: usize,
        current: &VehicleStore,
    ) -> Result<(), StepError> {
        let slot = self.order[rank].slot;
        let old = current
            .active_control(slot)
            .expect("next state has active predecessor");
        self.set_control(
            rank,
            VehicleStatus::Completed,
            old.maneuver,
            old.waiting,
            current,
        )
    }
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
        self.resource_rows
            .try_reserve(active.saturating_sub(self.resource_rows.len()))
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
        self.resource_rows.clear();
        for block in &mut self.motion {
            block.valid.fill(0);
        }
        self.published = false;
    }

    pub(crate) fn len(&self) -> usize {
        self.order.len()
    }

    /// 每个更新行至多一个控制覆盖；重复写回替换原项，恢复 Active 也保留该项。
    pub(crate) fn active_len(&self) -> usize {
        self.order.len()
            - self
                .control
                .iter()
                .filter(|change| change.status != VehicleStatus::Active)
                .count()
    }

    /// 按更新序号读取一行 Frontier 分类输入；供分段并行分类使用。
    pub(crate) fn frontier_input(
        &self,
        index: usize,
        current: &VehicleStore,
    ) -> FrontierMotionInput {
        let row = self.row(index, current);
        let offset = self.order[index].physical % BLOCK_ROWS;
        FrontierMotionInput {
            vehicle: row.source.handle(),
            route: row.source.route(),
            cursor: row.next.route_cursor[offset],
            progress_mm: row.next.progress_mm[offset],
            speed_mm_s: row.next.speed_mm_s[offset],
            status: row.status(),
        }
    }

    pub(crate) fn resource_rows(&self) -> &[usize] {
        &self.resource_rows
    }

    /// 容量已在 P5 求值前准备；筛选不分配，也不改变首错或资源消费顺序。
    #[cfg(test)]
    pub(crate) fn select_resource_rows(
        &mut self,
        current: &VehicleStore,
        mut select: impl FnMut(UpdateView<'_>) -> bool,
    ) {
        self.resource_rows.clear();
        for index in 0..self.order.len() {
            if select(self.row(index, current)) {
                self.resource_rows.push(index);
            }
        }
    }

    /// 只补充 Waiting 暂存实际新增的控制义务；旧成员及 P5 完成行已由 hints 保留。
    /// 原候选已规范有序，控制覆盖唯一；追加前查原前缀避免超过已预留的 Active 容量。
    pub(crate) fn supplement_resource_rows(&mut self) {
        let original = self.resource_rows.len();
        for changed in &self.control {
            if (changed.status != VehicleStatus::Active
                || changed.waiting.is_some()
                || changed.maneuver.is_some())
                && self.resource_rows[..original]
                    .binary_search(&changed.logical_index)
                    .is_err()
            {
                self.resource_rows.push(changed.logical_index);
            }
        }
        if self.resource_rows.len() != original {
            self.resource_rows.sort_unstable();
        }
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
        // 手工逻辑值夹具默认保留完整遍历，独立于生产筛选谓词。
        self.resource_rows.push(index);
        self.set(index, state, current)
            .expect("test next control allocation");
    }

    pub(crate) fn adopt(
        &mut self,
        handle: crate::VehicleHandle,
        completed: bool,
        hints: super::resource_rows::FinalizeHints,
        current: &VehicleStore,
    ) -> Result<(), StepError> {
        let physical = current
            .active_row(handle)
            .expect("column result has active predecessor");
        let index = self.order.len();
        self.order.push(UpdateRow {
            slot: handle.index() as usize,
            physical,
        });
        self.motion[physical / BLOCK_ROWS].valid[physical % BLOCK_ROWS / 64] |=
            1 << (physical % 64);
        if completed {
            let old = current
                .active_control(handle.index() as usize)
                .expect("next state has active predecessor");
            self.set_control(
                index,
                VehicleStatus::Completed,
                old.maneuver,
                old.waiting,
                current,
            )?;
        }
        if hints.retain() {
            self.resource_rows.push(index);
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

    pub(crate) fn row<'a>(&'a self, index: usize, current: &'a VehicleStore) -> UpdateView<'a> {
        assert!(!self.published, "next motion has not been published");
        let row = self.order[index];
        let next = &self.motion[row.physical / BLOCK_ROWS];
        assert_ne!(
            next.valid[row.physical % BLOCK_ROWS / 64] & (1 << (row.physical % 64)),
            0,
            "next row initialized"
        );
        UpdateView {
            source: current
                .active_binding_at(row.slot, row.physical)
                .expect("next state has active predecessor"),
            next,
            control: self.control_by_row[row.physical]
                .checked_sub(1)
                .map(|index| &self.control[index as usize]),
        }
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
        let changed = self
            .control_changed(index, status, maneuver, waiting, current)
            .expect("next state has active predecessor");
        self.set_control_changed(index, status, maneuver, waiting, changed)
    }

    /// 控制记录相对拍初是否变化；行没有 Active 前驱时返回 `None`。只读，可在
    /// 并行规划里先算好再交给 [`Self::set_control_changed`]。
    pub(crate) fn control_changed(
        &self,
        index: usize,
        status: VehicleStatus,
        maneuver: Option<crate::ManeuverTraversalState>,
        waiting: Option<crate::WaitingMembership>,
        current: &VehicleStore,
    ) -> Option<bool> {
        let old = current.active_control(self.order.get(index)?.slot)?;
        Some(status != VehicleStatus::Active || waiting != old.waiting || maneuver != old.maneuver)
    }

    /// 同 [`Self::set_control`]，`changed` 由调用方按 [`Self::control_changed`] 算好。
    pub(crate) fn set_control_changed(
        &mut self,
        index: usize,
        status: VehicleStatus,
        maneuver: Option<crate::ManeuverTraversalState>,
        waiting: Option<crate::WaitingMembership>,
        changed: bool,
    ) -> Result<(), StepError> {
        let row = self.order[index];
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

    #[cfg(test)]
    pub(crate) fn validate(&self, current: &VehicleStore) -> Result<(), StepError> {
        self.validate_with(current, None)
    }

    /// 提交前核对更新与当前车辆存储同形；多线程时绑定核对分段并行。
    pub(crate) fn validate_with(
        &self,
        current: &VehicleStore,
        execution: Option<&crate::kernel::execution::ExecutionResources>,
    ) -> Result<(), StepError> {
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
        if rows != self.order.len() || !self.rows_bound(current, execution) {
            return Err(StepError::ConflictInvariantViolation);
        }
        Ok(())
    }

    /// 每个更新行仍绑定到同一活动车位与物理行；多线程时分段并行核对。
    fn rows_bound(
        &self,
        current: &VehicleStore,
        execution: Option<&crate::kernel::execution::ExecutionResources>,
    ) -> bool {
        let bound = |rows: &[UpdateRow]| {
            rows.iter()
                .all(|row| current.active_binding_at(row.slot, row.physical).is_some())
        };
        let Some(resources) = execution.filter(|resources| {
            resources.coordinator_parallel() && self.order.len() >= VALIDATE_PARALLEL_ROWS
        }) else {
            return bound(&self.order);
        };
        let parts = resources
            .dispatch_threads()
            .saturating_mul(4)
            .min(VALIDATE_MAX_PARTS)
            .min(self.order.len())
            .max(1);
        let span = self.order.len().div_ceil(parts).max(1);
        let mut results = [true; VALIDATE_MAX_PARTS];
        resources.for_each_part(&mut results[..parts], 1, |part, out| {
            let start = (part * span).min(self.order.len());
            let end = (start + span).min(self.order.len());
            out[0] = bound(&self.order[start..end]);
        });
        results[..parts].iter().all(|bound| *bound)
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
                let handle = self.row(changed.logical_index, current).source.handle();
                if !current.has_control(handle) {
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
        let Self {
            motion,
            reports,
            order,
            control,
            control_by_row,
            resource_rows,
            published: _,
        } = self;
        vec_bytes(motion)
            + vec_bytes(reports)
            + vec_bytes(order)
            + vec_bytes(control)
            + vec_bytes(control_by_row)
            + vec_bytes(resource_rows)
            + motion
                .iter()
                .map(MotionBlock::retained_columns_bytes)
                .sum::<u64>()
    }
}
