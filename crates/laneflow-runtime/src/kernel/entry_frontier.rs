//! 入口 frontier：只为近门车辆维护冲突接近记录。
//!
//! 合同见 `docs/design/traffic-runtime-near-gate-frontier.md`（#740）。
//! 没有研究开关，也没有第二条可退回的全量产品路径。

use std::collections::BTreeMap;

use laneflow_static_contract::{ManeuverGateOrdinal, SignalAspect, SignalGroupOrdinal};
use laneflow_static_network::BoundedDistance;

use super::conflict::{PreparedApproachEta, interpret_gate_policy};
use super::phase::{StepReadView, StepWorkspace};
use super::tables::{ConflictPassageOccurrence, distance_to_occurrence_progress};
use crate::{
    ApproachEstimate, ConflictPassageAddress, GateCandidateKind, GatePolicyDecision, StepError,
    VehicleHandle, VehicleState, VehicleStatus, WorldGeneration,
};

const NO_DISTANCE_MM: u32 = u32::MAX;
/// 边界上的毫米余量，避免浮点比较把刚好可达的车排除在近门之外。
const REACH_SLACK_MM: f64 = 2.0;
/// 证明时窗边界的毫米余量，使更远冲突在进入时窗前触发整段重走。
const HORIZON_SLACK_MM: f64 = 1.0;

#[cfg(test)]
thread_local! {
    /// `usize::MAX` 表示关闭。其余值是还可以成功的链接预留次数，归零时失败。
    static LINK_RESERVE_SUCCESSES: std::cell::Cell<usize> =
        const { std::cell::Cell::new(usize::MAX) };
    /// `usize::MAX` 表示关闭。其余值是提交阶段还可以拆掉的旧链接数，归零时失败。
    static LINK_COMMIT_SUCCESSES: std::cell::Cell<usize> =
        const { std::cell::Cell::new(usize::MAX) };
    /// 唯一地址投影里的排序与相邻去重比较次数。
    static ADDRESS_COMPARISONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// 拆掉旧关联时实际比较过的成员次数。追加新关联不扫描成员。
    static MEMBER_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn link_reserve_should_fail() -> bool {
    LINK_RESERVE_SUCCESSES.with(|remaining| match remaining.get() {
        usize::MAX => false,
        0 => {
            remaining.set(usize::MAX);
            true
        }
        value => {
            remaining.set(value - 1);
            false
        }
    })
}

#[cfg(test)]
fn link_commit_should_fail() -> bool {
    LINK_COMMIT_SUCCESSES.with(|remaining| match remaining.get() {
        usize::MAX => false,
        0 => {
            remaining.set(usize::MAX);
            true
        }
        value => {
            remaining.set(value - 1);
            false
        }
    })
}

#[cfg(test)]
fn note_address_comparison() {
    ADDRESS_COMPARISONS.with(|count| count.set(count.get().saturating_add(1)));
}

#[cfg(test)]
fn note_member_probe() {
    MEMBER_PROBES.with(|count| count.set(count.get().saturating_add(1)));
}

fn compare_passage_address(
    left: ConflictPassageAddress,
    right: ConflictPassageAddress,
) -> std::cmp::Ordering {
    #[cfg(test)]
    note_address_comparison();
    left.cmp(&right)
}

fn detach_vehicle(list: &mut Vec<u32>, index: u32) {
    let Some(position) = list.iter().position(|vehicle| {
        #[cfg(test)]
        note_member_probe();
        *vehicle == index
    }) else {
        return;
    };
    list.swap_remove(position);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FrontierIdentity {
    world_id: u64,
    generation: WorldGeneration,
}

#[derive(Clone, Copy, Debug)]
struct CachedCell {
    address: ConflictPassageAddress,
    distance_mm: u32,
}

struct FrontierSlot {
    generation: u32,
    route_index: u32,
    route_generation: u32,
    edge: u32,
    progress: u32,
    first_excluded_mm: u32,
    gate_distance_mm: u32,
    max_accel: f32,
    cells: Vec<CachedCell>,
    valid: bool,
}

impl Default for FrontierSlot {
    fn default() -> Self {
        Self {
            generation: 0,
            route_index: 0,
            route_generation: 0,
            edge: 0,
            progress: 0,
            first_excluded_mm: NO_DISTANCE_MM,
            gate_distance_mm: NO_DISTANCE_MM,
            max_accel: 0.0,
            cells: Vec::new(),
            valid: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct SignalHold {
    gate_mm: u32,
    delay_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
struct IncrementSlot {
    generation: u32,
    present: bool,
}

impl IncrementSlot {
    const EMPTY: Self = Self {
        generation: 0,
        present: false,
    };
}

/// 跨拍保留的近门名单、冲突距离缓存和生命周期增量。
#[derive(Default)]
pub(crate) struct FrontierMaintenance {
    identity: Option<FrontierIdentity>,
    seeded: bool,
    slots: Vec<FrontierSlot>,
    by_cell: BTreeMap<ConflictPassageAddress, Vec<u32>>,
    ready_near: Vec<VehicleHandle>,
    ready_invalid: Vec<VehicleHandle>,
    pending_near: Vec<VehicleHandle>,
    pending_invalid: Vec<VehicleHandle>,
    /// 槽位上的最新生命周期 generation。`present` 为假表示该槽位不再是增量。
    increment_slots: Vec<IncrementSlot>,
    increment_indexes: Vec<u32>,
    /// 本拍工作副本。容量跨拍保留，稳态不再分配。
    scratch_near: Vec<VehicleHandle>,
    scratch_invalid: Vec<VehicleHandle>,
    scratch_increments: Vec<VehicleHandle>,
    scratch_demanded: Vec<ConflictPassageAddress>,
    scratch_cells: Vec<CachedCell>,
    /// 本车出现项的唯一地址投影。容量跨拍保留，与按路线距离排列的缓存分开。
    scratch_addresses: Vec<ConflictPassageAddress>,
    seen: Vec<u32>,
    seen_gen: u32,
    #[cfg(test)]
    full_walked: Vec<VehicleHandle>,
    #[cfg(test)]
    insertions: Vec<(ConflictPassageAddress, VehicleHandle, ApproachEstimate)>,
}

impl FrontierMaintenance {
    /// 记录本拍必须成为接近来源的活动车辆，并拆掉旧槽位缓存。
    ///
    /// 同一槽位只保留最新 generation，消费时一次整理。
    pub(crate) fn note_active_source(&mut self, vehicle: VehicleHandle) {
        self.unlink(vehicle.index());
        self.write_increment(vehicle.index(), Some(vehicle.generation()));
    }

    /// 车辆离开 Active 或槽位被释放后，不再作为接近来源。
    pub(crate) fn invalidate(&mut self, vehicle: VehicleHandle) {
        self.unlink(vehicle.index());
        self.write_increment(vehicle.index(), None);
    }

    /// 成功提交后发布本拍分类，并清空已经消费的生命周期增量。
    pub(crate) fn publish(&mut self) {
        self.ready_near.clear();
        self.ready_invalid.clear();
        std::mem::swap(&mut self.ready_near, &mut self.pending_near);
        std::mem::swap(&mut self.ready_invalid, &mut self.pending_invalid);
        self.pending_near.clear();
        self.pending_invalid.clear();
        for index in self.increment_indexes.drain(..) {
            if let Some(slot) = usize::try_from(index)
                .ok()
                .and_then(|index| self.increment_slots.get_mut(index))
            {
                slot.present = false;
            }
        }
        self.seeded = true;
    }

    fn write_increment(&mut self, index: u32, generation: Option<u32>) {
        let Some(index_usize) = usize::try_from(index).ok() else {
            return;
        };
        if self.increment_slots.len() <= index_usize {
            self.increment_slots
                .resize(index_usize + 1, IncrementSlot::EMPTY);
        }
        let slot = &mut self.increment_slots[index_usize];
        let was_present = slot.present;
        match generation {
            Some(generation) => {
                slot.generation = generation;
                slot.present = true;
                if !was_present {
                    self.increment_indexes.push(index);
                }
            }
            None => slot.present = false,
        }
    }

    fn ensure_identity(&mut self, world_id: u64, generation: WorldGeneration) -> bool {
        let same = self.identity
            == Some(FrontierIdentity {
                world_id,
                generation,
            });
        if !same {
            self.reset_storage();
            self.identity = Some(FrontierIdentity {
                world_id,
                generation,
            });
            self.seeded = false;
            return false;
        }
        self.seeded
    }

    fn reset_storage(&mut self) {
        self.slots.clear();
        self.by_cell.clear();
        self.ready_near.clear();
        self.ready_invalid.clear();
        self.pending_near.clear();
        self.pending_invalid.clear();
        self.increment_slots.clear();
        self.increment_indexes.clear();
        self.scratch_near.clear();
        self.scratch_invalid.clear();
        self.scratch_increments.clear();
        self.scratch_demanded.clear();
        self.scratch_cells.clear();
        self.scratch_addresses.clear();
        self.seen.clear();
        self.seen_gen = 0;
    }

    fn snapshot_published_lists(&mut self) -> Result<(), StepError> {
        let near = std::mem::take(&mut self.ready_near);
        let invalid = std::mem::take(&mut self.ready_invalid);
        let near_result = refill_handles(&mut self.scratch_near, &near);
        let invalid_result = refill_handles(&mut self.scratch_invalid, &invalid);
        self.ready_near = near;
        self.ready_invalid = invalid;
        near_result?;
        invalid_result?;
        self.collect_increments()
    }

    /// 同一槽位只发出最新 generation。名单保留到成功提交，失败重试仍能看见。
    fn collect_increments(&mut self) -> Result<(), StepError> {
        self.increment_indexes.sort_unstable();
        self.increment_indexes.dedup();
        self.increment_indexes.retain(|index| {
            usize::try_from(*index)
                .ok()
                .and_then(|index| self.increment_slots.get(index))
                .is_some_and(|slot| slot.present)
        });
        self.scratch_increments.clear();
        self.scratch_increments
            .try_reserve(self.increment_indexes.len())
            .map_err(|_| StepError::ConflictScratchAllocFailed)?;
        for index in &self.increment_indexes {
            let slot = self.increment_slots[usize::try_from(*index).expect("index fits usize")];
            self.scratch_increments
                .push(VehicleHandle::new(*index, slot.generation));
        }
        Ok(())
    }

    fn begin_full_records(&mut self) {
        self.by_cell.clear();
        for slot in &mut self.slots {
            slot.valid = false;
            slot.cells.clear();
            slot.first_excluded_mm = NO_DISTANCE_MM;
        }
    }

    /// `updates` 是本拍运动结果。下一拍 frontier 读的就是这份状态。
    fn classify(
        &mut self,
        delta_s: f32,
        horizon_ms: Option<u64>,
        updates: &[(usize, VehicleState)],
    ) -> Result<(), StepError> {
        self.pending_near.clear();
        self.pending_invalid.clear();
        let active = updates
            .iter()
            .filter(|(_, state)| state.status == VehicleStatus::Active)
            .count();
        // 两侧一起备好。只扩 pending 时，publish 交换后另一侧会在下一拍首次增长。
        reserve_capacity(&mut self.pending_near, active)?;
        reserve_capacity(&mut self.pending_invalid, active)?;
        reserve_capacity(&mut self.ready_near, active)?;
        reserve_capacity(&mut self.ready_invalid, active)?;
        for (_, state) in updates {
            if state.status != VehicleStatus::Active {
                continue;
            }
            let vehicle = state.handle;
            let Some(remembered) = self.remembered(vehicle, state) else {
                self.pending_near.push(vehicle);
                self.pending_invalid.push(vehicle);
                continue;
            };
            let traveled = state.progress_mm.saturating_sub(remembered.progress);
            let reach = one_tick_reach_mm(state.speed_mm_s, remembered.max_accel, delta_s);
            let gate_remaining = remembered.gate_distance_mm.saturating_sub(traveled);
            let near = remembered.gate_distance_mm != NO_DISTANCE_MM
                && (!reach.is_finite() || f64::from(gate_remaining) <= reach);
            let dirty = horizon_ms.is_some_and(|horizon| {
                remembered.first_excluded_mm != NO_DISTANCE_MM
                    && f64::from(remembered.first_excluded_mm.saturating_sub(traveled))
                        <= horizon_reach_mm(state.speed_mm_s, remembered.max_accel, horizon)
            });
            if near {
                self.pending_near.push(vehicle);
            }
            if dirty {
                self.pending_invalid.push(vehicle);
            }
        }
        Ok(())
    }

    fn remembered(&self, vehicle: VehicleHandle, state: &VehicleState) -> Option<RememberedSlot> {
        let slot = self.slots.get(usize::try_from(vehicle.index()).ok()?)?;
        if !slot.valid
            || slot.generation != vehicle.generation()
            || slot.route_index != state.route.index()
            || slot.route_generation != state.route.generation()
            || slot.edge != state.route_edge_index
            || state.progress_mm < slot.progress
        {
            return None;
        }
        Some(RememberedSlot {
            progress: slot.progress,
            first_excluded_mm: slot.first_excluded_mm,
            gate_distance_mm: slot.gate_distance_mm,
            max_accel: slot.max_accel,
        })
    }

    fn replay_hit(
        &self,
        vehicle: VehicleHandle,
        state: &VehicleState,
        horizon_ms: u64,
    ) -> Option<u32> {
        let remembered = self.remembered(vehicle, state)?;
        let traveled = state.progress_mm.saturating_sub(remembered.progress);
        if remembered.first_excluded_mm != NO_DISTANCE_MM {
            let remaining = remembered.first_excluded_mm.saturating_sub(traveled);
            if f64::from(remaining)
                <= horizon_reach_mm(state.speed_mm_s, remembered.max_accel, horizon_ms)
            {
                return None;
            }
        }
        Some(remembered.progress)
    }

    fn store(
        &mut self,
        vehicle: VehicleHandle,
        state: &VehicleState,
        first_excluded_mm: u32,
        cells: Vec<CachedCell>,
        gate_distance_mm: u32,
        max_accel: f32,
    ) -> Result<(), StepError> {
        let index = vehicle.index();
        let index_usize =
            usize::try_from(index).map_err(|_| StepError::ConflictInvariantViolation)?;
        if self.slots.len() <= index_usize {
            self.slots
                .try_reserve(index_usize + 1 - self.slots.len())
                .map_err(|_| StepError::ConflictScratchAllocFailed)?;
            self.slots
                .resize_with(index_usize + 1, FrontierSlot::default);
        }
        self.load_unique_addresses(&cells)?;
        self.reserve_links()?;
        let mut old = if self.slots[index_usize].valid {
            std::mem::take(&mut self.slots[index_usize].cells)
        } else {
            Vec::new()
        };
        if let Err(error) = self.commit_links(index, &mut old) {
            self.seeded = false;
            return Err(error);
        }
        let slot = &mut self.slots[index_usize];
        slot.generation = vehicle.generation();
        slot.route_index = state.route.index();
        slot.route_generation = state.route.generation();
        slot.edge = state.route_edge_index;
        slot.progress = state.progress_mm;
        slot.first_excluded_mm = first_excluded_mm;
        slot.gate_distance_mm = gate_distance_mm;
        slot.max_accel = max_accel;
        slot.cells = cells;
        slot.valid = true;
        Ok(())
    }

    /// 把本车出现项投影成唯一地址，排序并去重一次。不改写按路线距离保存的 `cells`。
    fn load_unique_addresses(&mut self, cells: &[CachedCell]) -> Result<(), StepError> {
        self.scratch_addresses.clear();
        self.scratch_addresses
            .try_reserve(cells.len())
            .map_err(|_| StepError::ConflictScratchAllocFailed)?;
        self.scratch_addresses
            .extend(cells.iter().map(|cell| cell.address));
        self.scratch_addresses
            .sort_unstable_by(|left, right| compare_passage_address(*left, *right));
        self.scratch_addresses.dedup_by(|left, right| {
            #[cfg(test)]
            note_address_comparison();
            *left == *right
        });
        Ok(())
    }

    /// 按已经去重的地址各预留一个新槽位。失败时还没有拆旧链接。
    fn reserve_links(&mut self) -> Result<(), StepError> {
        for offset in 0..self.scratch_addresses.len() {
            let address = self.scratch_addresses[offset];
            #[cfg(test)]
            if link_reserve_should_fail() {
                return Err(StepError::ConflictScratchAllocFailed);
            }
            if let Some(list) = self.by_cell.get_mut(&address) {
                list.try_reserve(1)
                    .map_err(|_| StepError::ConflictScratchAllocFailed)?;
            } else {
                let mut list = Vec::new();
                list.try_reserve(1)
                    .map_err(|_| StepError::ConflictScratchAllocFailed)?;
                self.by_cell.insert(address, list);
            }
        }
        Ok(())
    }

    /// 先按唯一旧地址拆掉本槽位，再为每个新地址追加一次。
    ///
    /// 调用前这些新地址已经在 `by_cell` 中，并且各有一个空位。
    fn commit_links(&mut self, index: u32, old: &mut [CachedCell]) -> Result<(), StepError> {
        old.sort_unstable_by_key(|cell| cell.address);
        let mut position = 0;
        while position < old.len() {
            let address = old[position].address;
            position += 1;
            while position < old.len() && old[position].address == address {
                position += 1;
            }
            #[cfg(test)]
            if link_commit_should_fail() {
                return Err(StepError::ConflictScratchAllocFailed);
            }
            self.release_old_address(index, address);
        }
        for offset in 0..self.scratch_addresses.len() {
            let address = self.scratch_addresses[offset];
            let Some(list) = self.by_cell.get_mut(&address) else {
                return Err(StepError::ConflictInvariantViolation);
            };
            list.push(index);
        }
        Ok(())
    }

    fn release_old_address(&mut self, index: u32, address: ConflictPassageAddress) {
        let empty = {
            let Some(list) = self.by_cell.get_mut(&address) else {
                return;
            };
            detach_vehicle(list, index);
            list.is_empty()
        };
        if empty && self.scratch_addresses.binary_search(&address).is_err() {
            self.by_cell.remove(&address);
        }
    }

    fn unlink(&mut self, index: u32) {
        let Some(slot) = usize::try_from(index)
            .ok()
            .and_then(|index| self.slots.get_mut(index))
        else {
            return;
        };
        if !slot.valid {
            return;
        }
        let mut cells = std::mem::take(&mut slot.cells);
        slot.valid = false;
        cells.sort_unstable_by_key(|cell| cell.address);
        let mut previous: Option<ConflictPassageAddress> = None;
        for cell in &cells {
            if previous == Some(cell.address) {
                continue;
            }
            previous = Some(cell.address);
            let Some(list) = self.by_cell.get_mut(&cell.address) else {
                continue;
            };
            detach_vehicle(list, index);
            if list.is_empty() {
                self.by_cell.remove(&cell.address);
            }
        }
    }

    fn begin_seen(&mut self) -> Result<(), StepError> {
        self.seen_gen = self.seen_gen.wrapping_add(1);
        if self.seen_gen == 0 {
            self.seen.fill(0);
            self.seen_gen = 1;
        }
        Ok(())
    }

    fn mark(&mut self, index: u32) -> Result<bool, StepError> {
        let index_usize =
            usize::try_from(index).map_err(|_| StepError::ConflictInvariantViolation)?;
        if self.seen.len() <= index_usize {
            self.seen
                .try_reserve(index_usize + 1 - self.seen.len())
                .map_err(|_| StepError::ConflictScratchAllocFailed)?;
            self.seen.resize(index_usize + 1, 0);
        }
        let newly = self.seen[index_usize] != self.seen_gen;
        self.seen[index_usize] = self.seen_gen;
        Ok(newly)
    }

    fn is_marked(&self, index: u32) -> bool {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.seen.get(index))
            .is_some_and(|seen| *seen == self.seen_gen)
    }

    #[cfg(test)]
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        let Self {
            identity: _,
            seeded: _,
            slots,
            by_cell,
            ready_near,
            ready_invalid,
            pending_near,
            pending_invalid,
            increment_slots,
            increment_indexes,
            scratch_near,
            scratch_invalid,
            scratch_increments,
            scratch_demanded,
            scratch_cells,
            scratch_addresses,
            seen,
            seen_gen: _,
            full_walked,
            insertions,
        } = self;
        crate::kernel::state::vec_bytes(slots)
            + slots
                .iter()
                .map(|slot| crate::kernel::state::vec_bytes(&slot.cells))
                .sum::<u64>()
            + by_cell
                .iter()
                .map(|(address, vehicles)| {
                    u64::try_from(core::mem::size_of_val(address)).unwrap_or(u64::MAX)
                        + crate::kernel::state::vec_bytes(vehicles)
                })
                .sum::<u64>()
            + crate::kernel::state::vec_bytes(ready_near)
            + crate::kernel::state::vec_bytes(ready_invalid)
            + crate::kernel::state::vec_bytes(pending_near)
            + crate::kernel::state::vec_bytes(pending_invalid)
            + crate::kernel::state::vec_bytes(increment_slots)
            + crate::kernel::state::vec_bytes(increment_indexes)
            + crate::kernel::state::vec_bytes(scratch_near)
            + crate::kernel::state::vec_bytes(scratch_invalid)
            + crate::kernel::state::vec_bytes(scratch_increments)
            + crate::kernel::state::vec_bytes(scratch_demanded)
            + crate::kernel::state::vec_bytes(scratch_cells)
            + crate::kernel::state::vec_bytes(scratch_addresses)
            + crate::kernel::state::vec_bytes(seen)
            + crate::kernel::state::vec_bytes(full_walked)
            + crate::kernel::state::vec_bytes(insertions)
    }
}

#[derive(Clone, Copy)]
struct RememberedSlot {
    progress: u32,
    first_excluded_mm: u32,
    gate_distance_mm: u32,
    max_accel: f32,
}

fn reserve_capacity(list: &mut Vec<VehicleHandle>, needed: usize) -> Result<(), StepError> {
    if list.capacity() >= needed {
        return Ok(());
    }
    list.try_reserve(needed - list.len())
        .map_err(|_| StepError::ConflictScratchAllocFailed)
}

fn refill_handles(
    destination: &mut Vec<VehicleHandle>,
    source: &[VehicleHandle],
) -> Result<(), StepError> {
    destination.clear();
    destination
        .try_reserve(source.len())
        .map_err(|_| StepError::ConflictScratchAllocFailed)?;
    destination.extend_from_slice(source);
    Ok(())
}

fn address_wanted(wanted: &[ConflictPassageAddress], address: ConflictPassageAddress) -> bool {
    wanted.binary_search(&address).is_ok()
}

fn horizon_reach_mm(speed_mm_s: u32, max_accel_m_s2: f32, horizon_ms: u64) -> f64 {
    kinematic_reach_mm(
        speed_mm_s,
        max_accel_m_s2,
        horizon_ms as f64 / 1_000.0,
        HORIZON_SLACK_MM,
    )
}

fn one_tick_reach_mm(speed_mm_s: u32, max_accel_m_s2: f32, delta_s: f32) -> f64 {
    kinematic_reach_mm(
        speed_mm_s,
        max_accel_m_s2,
        f64::from(delta_s),
        REACH_SLACK_MM,
    )
}

fn kinematic_reach_mm(speed_mm_s: u32, max_accel_m_s2: f32, seconds: f64, slack_mm: f64) -> f64 {
    if !seconds.is_finite() || seconds < 0.0 || !max_accel_m_s2.is_finite() || max_accel_m_s2 < 0.0
    {
        return f64::INFINITY;
    }
    let accel_mm_s2 = f64::from(max_accel_m_s2) * 1_000.0;
    f64::from(speed_mm_s) * seconds + 0.5 * accel_mm_s2 * seconds * seconds + slack_mm
}

fn raise_signal_bound(
    kinematic: ApproachEstimate,
    hold: Option<SignalHold>,
    entry_mm: u32,
    horizon_ms: u64,
) -> ApproachEstimate {
    let Some(hold) = hold else {
        return kinematic;
    };
    if entry_mm < hold.gate_mm {
        return kinematic;
    }
    let Some(delay) = hold.delay_ms else {
        return kinematic;
    };
    if delay == 0 {
        return kinematic;
    }
    let ApproachEstimate::Finite(ms) = kinematic else {
        return kinematic;
    };
    let eta = ms.max(delay);
    if eta >= horizon_ms {
        ApproachEstimate::OutsideHorizon
    } else {
        ApproachEstimate::Finite(eta)
    }
}

fn signal_hold(
    read: StepReadView<'_>,
    vehicle: VehicleHandle,
    state: &VehicleState,
) -> Option<SignalHold> {
    if read.conflict_reservation(vehicle).is_some() {
        return None;
    }
    let compiled = read.compiled_route(state.route)?;
    let (gate, gate_mm) = first_red_gate(read, compiled, state)?;
    Some(SignalHold {
        gate_mm,
        delay_ms: signal_release_delay_ms(read, gate, state),
    })
}

fn first_red_gate(
    read: StepReadView<'_>,
    compiled: &super::tables::CompiledRoute,
    state: &VehicleState,
) -> Option<(ManeuverGateOrdinal, u32)> {
    // 零进度、零余量表示上一边终点已经规范成下一条边的起点，车辆仍停在上一 hop 的准入门上。
    let rolled = state.progress_mm == 0 && state.carry_um == 0 && state.route_edge_index > 0;
    let mut hop = usize::try_from(if rolled {
        state.route_edge_index - 1
    } else {
        state.route_edge_index
    })
    .ok()?;
    let progress_base = if rolled {
        let edge = *compiled.edges.get(hop)?;
        *read
            .binding
            .revision
            .traffic()
            .lane_lengths_millimetres()
            .get(edge.index())?
    } else {
        state.progress_mm
    };
    let mut from_cursor_start = BoundedDistance::Finite(0);
    let mut accumulated = false;
    while hop < compiled.next_controlled.len() {
        let next = compiled.next_controlled[hop]?;
        from_cursor_start = if accumulated {
            from_cursor_start.add_bounded(next.distance_from_hop_start)
        } else {
            next.distance_from_hop_start
        };
        accumulated = true;
        if read.gate_is_restrictive(next.gate, state.profile) {
            let BoundedDistance::Finite(mm) = from_cursor_start.saturating_sub(progress_base)
            else {
                return None;
            };
            return Some((next.gate, mm));
        }
        let next_hop = usize::try_from(next.hop).ok()?.checked_add(1)?;
        if next_hop <= hop {
            return None;
        }
        hop = next_hop;
    }
    None
}

fn signal_release_delay_ms(
    read: StepReadView<'_>,
    gate: ManeuverGateOrdinal,
    state: &VehicleState,
) -> Option<u64> {
    let relations = read.binding.revision.traffic().relations();
    let group = relations.maneuver_gate(gate)?.signal_group()?;
    let controller = relations.signal_controller(relations.signal_group(group)?.controller())?;
    let cycle = controller.cycle_ms();
    if cycle == 0 {
        return None;
    }
    let position = u64::try_from(
        (u128::from(read.committed.time_ms) + u128::from(controller.offset_ms()))
            % u128::from(cycle),
    )
    .ok()?;
    let phases = controller.phases();
    if phases.is_empty() {
        return None;
    }
    let index = phases
        .partition_point(|phase| relations.phase_end_offset_ms(*phase).unwrap_or(0) <= position);
    let current = *phases.get(index)?;
    let mut wait = relations
        .phase_end_offset_ms(current)?
        .saturating_sub(position);
    let mut cursor = index;
    loop {
        cursor += 1;
        if cursor == phases.len() {
            cursor = 0;
        }
        if cursor == index || wait > cycle {
            return None;
        }
        let phase = phases[cursor];
        if gate_opens(read, gate, state, phase, group) {
            return Some(wait);
        }
        wait = wait.saturating_add(relations.phase_duration_ms(phase).unwrap_or(0));
    }
}

fn gate_opens(
    read: StepReadView<'_>,
    gate: ManeuverGateOrdinal,
    state: &VehicleState,
    phase: laneflow_static_contract::SignalPhaseOrdinal,
    group: SignalGroupOrdinal,
) -> bool {
    let relations = read.binding.revision.traffic().relations();
    let aspect = relations
        .phase_states(phase)
        .and_then(|(groups, aspects)| {
            groups
                .iter()
                .zip(aspects.iter().copied())
                .find(|(candidate, _)| **candidate == group)
                .map(|(_, aspect)| aspect)
        })
        .unwrap_or(SignalAspect::Red);
    let Some(class) = relations
        .vehicle_profile(state.profile)
        .map(|profile| profile.class())
    else {
        return false;
    };
    let Some(rule) = read.policy().and_then(|policy| policy.gate(gate, class)) else {
        return false;
    };
    !matches!(
        interpret_gate_policy(*rule, true, Some(aspect)).unwrap_or(GatePolicyDecision::DenyAndStop),
        GatePolicyDecision::DenyAndStop
    )
}

fn gate_distance_mm(compiled: &super::tables::CompiledRoute, state: &VehicleState) -> u32 {
    let cursor = if state.progress_mm == 0 && state.carry_um == 0 {
        state.route_edge_index.saturating_sub(1)
    } else {
        state.route_edge_index
    };
    let first = compiled.gate_hops.partition_point(|hop| *hop < cursor);
    let Some(gate_hop) = compiled.gate_hops.get(first).copied() else {
        return NO_DISTANCE_MM;
    };
    if gate_hop < state.route_edge_index {
        return 0;
    }
    let (Ok(from_index), Ok(gate_index)) = (
        usize::try_from(state.route_edge_index),
        usize::try_from(gate_hop),
    ) else {
        return NO_DISTANCE_MM;
    };
    let Some(to_index) = gate_index.checked_add(1) else {
        return NO_DISTANCE_MM;
    };
    match super::tables::distance_to_occurrence_start(
        &compiled.occurrence_segments,
        &compiled.occurrence_offsets,
        &compiled.segment_totals,
        from_index,
        state.progress_mm,
        to_index,
    ) {
        Some(BoundedDistance::Finite(mm)) => mm,
        Some(BoundedDistance::BeyondFinite) | None => NO_DISTANCE_MM,
    }
}

/// 重建本拍入口 frontier。名单未发布或世界身份变化时全量重走。
pub(crate) fn rebuild(step: &mut StepWorkspace<'_>) -> Result<(), StepError> {
    let Some(horizon_ms) = step.frontier_proof_horizon_ms() else {
        return Ok(());
    };
    #[cfg(test)]
    {
        step.workspace.frontier_maintenance.full_walked.clear();
        step.workspace.frontier_maintenance.insertions.clear();
    }
    let seeded = step
        .workspace
        .frontier_maintenance
        .ensure_identity(step.binding.world_id, step.binding.world_generation);
    if !seeded || step.read_view().policy().is_none() {
        return full_walk(step, horizon_ms);
    }
    held_walk(step, horizon_ms)
}

/// 用本拍运动结果写下一批近门集合和失效名单。
/// 没有证明时窗时不分类，也不为没有读者的名单预留容量。
pub(crate) fn classify_pending(
    step: &mut StepWorkspace<'_>,
    delta_s: f32,
    updates: &[(usize, VehicleState)],
) -> Result<(), StepError> {
    let Some(horizon_ms) = step.frontier_proof_horizon_ms() else {
        let maintenance = &mut step.workspace.frontier_maintenance;
        maintenance.pending_near.clear();
        maintenance.pending_invalid.clear();
        return Ok(());
    };
    step.workspace
        .frontier_maintenance
        .classify(delta_s, Some(horizon_ms), updates)
}

fn full_walk(step: &mut StepWorkspace<'_>, horizon_ms: u64) -> Result<(), StepError> {
    step.workspace.frontier_maintenance.begin_full_records();
    let count = step.committed.live_order.len();
    for sequence in 0..count {
        let vehicle = step.committed.live_order[sequence];
        let sequence =
            u32::try_from(sequence).map_err(|_| StepError::ConflictInvariantViolation)?;
        let Some(state) = step.vehicle_state(vehicle).copied() else {
            continue;
        };
        if state.status != VehicleStatus::Active {
            continue;
        }
        walk_vehicle(step, vehicle, state, sequence, horizon_ms, None, true)?;
    }
    Ok(())
}

fn held_walk(step: &mut StepWorkspace<'_>, horizon_ms: u64) -> Result<(), StepError> {
    step.workspace
        .frontier_maintenance
        .snapshot_published_lists()?;
    let delta_s = step.binding.config.fixed_delta_time_ms() as f32 / 1_000.0;
    step.workspace.frontier_maintenance.begin_seen()?;
    collect_targets(step, delta_s)?;
    let demanded = std::mem::take(&mut step.workspace.frontier_maintenance.scratch_demanded);

    step.workspace.frontier_maintenance.begin_seen()?;
    let invalid_len = step.workspace.frontier_maintenance.scratch_invalid.len();
    let increment_len = step.workspace.frontier_maintenance.scratch_increments.len();
    for index in 0..invalid_len {
        let vehicle = step.workspace.frontier_maintenance.scratch_invalid[index];
        walk_if_new(step, vehicle, horizon_ms, &demanded, true)?;
    }
    for index in 0..increment_len {
        let vehicle = step.workspace.frontier_maintenance.scratch_increments[index];
        walk_if_new(step, vehicle, horizon_ms, &demanded, true)?;
    }
    let near_len = step.workspace.frontier_maintenance.scratch_near.len();
    for index in 0..near_len {
        let vehicle = step.workspace.frontier_maintenance.scratch_near[index];
        let Some(state) = active_state(step, vehicle) else {
            continue;
        };
        if step
            .workspace
            .frontier_maintenance
            .replay_hit(vehicle, &state, horizon_ms)
            .is_none()
        {
            walk_if_new(step, vehicle, horizon_ms, &demanded, true)?;
        }
    }

    let demanded_len = demanded.len();
    step.workspace
        .frontier_maintenance
        .scratch_increments
        .clear();
    for index in 0..demanded_len {
        let address = demanded[index];
        let count = step
            .workspace
            .frontier_maintenance
            .by_cell
            .get(&address)
            .map(|indexes| indexes.len())
            .unwrap_or(0);
        for position in 0..count {
            let Some(vehicle_index) = step
                .workspace
                .frontier_maintenance
                .by_cell
                .get(&address)
                .and_then(|indexes| indexes.get(position))
                .copied()
            else {
                continue;
            };
            if step.workspace.frontier_maintenance.is_marked(vehicle_index) {
                continue;
            }
            let Some(vehicle) = vehicle_from_slot(step, vehicle_index)? else {
                continue;
            };
            let Some((state, sequence)) = accepted_source(step, vehicle)? else {
                continue;
            };
            let reusable = step
                .workspace
                .frontier_maintenance
                .replay_hit(vehicle, &state, horizon_ms)
                .is_some();
            if !step.workspace.frontier_maintenance.mark(vehicle.index())? {
                continue;
            }
            if reusable {
                replay_walk(step, vehicle, state, sequence, horizon_ms, Some(&demanded))?;
            } else {
                step.workspace
                    .frontier_maintenance
                    .scratch_increments
                    .try_reserve(1)
                    .map_err(|_| StepError::ConflictScratchAllocFailed)?;
                step.workspace
                    .frontier_maintenance
                    .scratch_increments
                    .push(vehicle);
            }
        }
    }
    let deferred = step.workspace.frontier_maintenance.scratch_increments.len();
    for index in 0..deferred {
        let vehicle = step.workspace.frontier_maintenance.scratch_increments[index];
        let Some((state, sequence)) = accepted_source(step, vehicle)? else {
            continue;
        };
        walk_vehicle(
            step,
            vehicle,
            state,
            sequence,
            horizon_ms,
            Some(&demanded),
            true,
        )?;
    }
    step.workspace.frontier_maintenance.scratch_demanded = demanded;
    Ok(())
}

fn walk_if_new(
    step: &mut StepWorkspace<'_>,
    vehicle: VehicleHandle,
    horizon_ms: u64,
    demanded: &[ConflictPassageAddress],
    record: bool,
) -> Result<(), StepError> {
    let Some((state, sequence)) = accepted_source(step, vehicle)? else {
        return Ok(());
    };
    if !step.workspace.frontier_maintenance.mark(vehicle.index())? {
        return Ok(());
    }
    walk_vehicle(
        step,
        vehicle,
        state,
        sequence,
        horizon_ms,
        Some(demanded),
        record,
    )
}

/// 先确认句柄仍是活动车辆并取得 live 序号，再允许调用方占用槽位标记。
fn accepted_source(
    step: &mut StepWorkspace<'_>,
    vehicle: VehicleHandle,
) -> Result<Option<(VehicleState, u32)>, StepError> {
    let Some(state) = active_state(step, vehicle) else {
        return Ok(None);
    };
    let Some(sequence) = live_sequence(step, vehicle)? else {
        return Ok(None);
    };
    Ok(Some((state, sequence)))
}

fn active_state(step: &StepWorkspace<'_>, vehicle: VehicleHandle) -> Option<VehicleState> {
    let state = step.vehicle_state(vehicle).copied()?;
    (state.status == VehicleStatus::Active).then_some(state)
}

fn live_sequence(
    step: &mut StepWorkspace<'_>,
    vehicle: VehicleHandle,
) -> Result<Option<u32>, StepError> {
    let slots = step.committed.vehicles.len();
    step.derived
        .live_rank(&step.committed.live_order, slots, vehicle)
        .map_err(|_| StepError::ConflictScratchAllocFailed)
}

fn vehicle_from_slot(
    step: &StepWorkspace<'_>,
    index: u32,
) -> Result<Option<VehicleHandle>, StepError> {
    let Some(slot) = usize::try_from(index)
        .ok()
        .and_then(|index| step.workspace.frontier_maintenance.slots.get(index))
    else {
        return Ok(None);
    };
    if !slot.valid {
        return Ok(None);
    }
    Ok(Some(VehicleHandle::new(index, slot.generation)))
}

fn collect_targets(step: &mut StepWorkspace<'_>, delta_s: f32) -> Result<(), StepError> {
    step.workspace.frontier_maintenance.scratch_demanded.clear();
    if step.read_view().policy().is_none() {
        return Ok(());
    }
    let near_len = step.workspace.frontier_maintenance.scratch_near.len();
    let increment_len = step.workspace.frontier_maintenance.scratch_increments.len();
    for index in 0..near_len + increment_len {
        let vehicle = if index < near_len {
            step.workspace.frontier_maintenance.scratch_near[index]
        } else {
            step.workspace.frontier_maintenance.scratch_increments[index - near_len]
        };
        let Some(state) = active_state(step, vehicle) else {
            continue;
        };
        if step.conflict_reservation(vehicle).is_some() {
            continue;
        }
        collect_vehicle_targets(step, state, delta_s)?;
    }
    step.workspace
        .frontier_maintenance
        .scratch_demanded
        .sort_unstable();
    step.workspace.frontier_maintenance.scratch_demanded.dedup();
    Ok(())
}

fn collect_vehicle_targets(
    step: &mut StepWorkspace<'_>,
    state: VehicleState,
    delta_s: f32,
) -> Result<(), StepError> {
    let profile = step
        .binding
        .revision
        .traffic()
        .relations()
        .vehicle_profile(state.profile)
        .ok_or(StepError::ConflictInvariantViolation)?;
    let class = profile.class();
    let reach = one_tick_reach_mm(state.speed_mm_s, profile.max_accel(), delta_s);
    let mut gate_list_index = 0usize;
    loop {
        let mut hops = [0u32; 8];
        let mut hop_count = 0usize;
        let mut finished = true;
        {
            let Some(compiled) = step.compiled_route(state.route) else {
                return Err(StepError::ConflictInvariantViolation);
            };
            let cursor = if state.progress_mm == 0 && state.carry_um == 0 {
                state.route_edge_index.saturating_sub(1)
            } else {
                state.route_edge_index
            };
            let first = compiled
                .gate_hops
                .partition_point(|hop| *hop < cursor)
                .max(gate_list_index);
            for (offset, gate_hop) in compiled.gate_hops[first..].iter().copied().enumerate() {
                let in_reach = if !reach.is_finite() || gate_hop < state.route_edge_index {
                    true
                } else {
                    let (Ok(from_index), Ok(gate_index)) = (
                        usize::try_from(state.route_edge_index),
                        usize::try_from(gate_hop),
                    ) else {
                        return Err(StepError::ConflictInvariantViolation);
                    };
                    let Some(to_index) = gate_index.checked_add(1) else {
                        return Err(StepError::ConflictInvariantViolation);
                    };
                    match super::tables::distance_to_occurrence_start(
                        &compiled.occurrence_segments,
                        &compiled.occurrence_offsets,
                        &compiled.segment_totals,
                        from_index,
                        state.progress_mm,
                        to_index,
                    ) {
                        Some(BoundedDistance::Finite(mm)) => f64::from(mm) <= reach,
                        Some(BoundedDistance::BeyondFinite) => false,
                        None => true,
                    }
                };
                if !in_reach {
                    break;
                }
                if hop_count == hops.len() {
                    finished = false;
                    gate_list_index = first + offset;
                    break;
                }
                let Some(gate) = compiled
                    .hop_gate
                    .get(
                        usize::try_from(gate_hop)
                            .map_err(|_| StepError::ConflictInvariantViolation)?,
                    )
                    .copied()
                    .flatten()
                else {
                    return Err(StepError::ConflictInvariantViolation);
                };
                let GatePolicyDecision::Candidate(kind) =
                    step.read_view().gate_policy_decision(gate, state.profile)
                else {
                    continue;
                };
                if kind == GateCandidateKind::Protected {
                    continue;
                }
                hops[hop_count] = gate_hop;
                hop_count += 1;
            }
        }
        for gate_hop in hops[..hop_count].iter().copied() {
            append_gate_targets(step, state, gate_hop, class)?;
        }
        if finished {
            break;
        }
    }
    Ok(())
}

fn append_gate_targets(
    step: &mut StepWorkspace<'_>,
    state: VehicleState,
    gate_hop: u32,
    class: laneflow_static_contract::ParticipantClassOrdinal,
) -> Result<(), StepError> {
    let mut occurrences = [ConflictPassageAddress::new(
        laneflow_static_contract::ConflictZoneOrdinal::from_raw(0),
        laneflow_static_contract::ParticipantStreamOrdinal::from_raw(0),
        0,
    ); 32];
    let mut occurrence_count = 0usize;
    let mut occurrence_cursor = 0usize;
    loop {
        let mut batch = 0usize;
        {
            let Some(compiled) = step.compiled_route(state.route) else {
                return Err(StepError::ConflictInvariantViolation);
            };
            let range = *compiled
                .conflict_gate_ranges
                .get(usize::try_from(gate_hop).map_err(|_| StepError::ConflictInvariantViolation)?)
                .ok_or(StepError::ConflictInvariantViolation)?;
            let end = usize::try_from(
                range
                    .start
                    .checked_add(range.len)
                    .ok_or(StepError::ConflictInvariantViolation)?,
            )
            .map_err(|_| StepError::ConflictInvariantViolation)?;
            let start = usize::try_from(range.start)
                .map_err(|_| StepError::ConflictInvariantViolation)?
                + occurrence_cursor;
            let Some(slice) = compiled.conflicts.get(start..end) else {
                return Err(StepError::ConflictInvariantViolation);
            };
            for occurrence in slice.iter().take(occurrences.len()) {
                occurrences[batch] = occurrence.address();
                batch += 1;
            }
        }
        if batch == 0 {
            break;
        }
        for occurrence in occurrences[..batch].iter().copied() {
            let target_len = {
                let Some(policy) = step.read_view().policy() else {
                    return Err(StepError::ConflictInvariantViolation);
                };
                let Some((zone, targets)) = policy.yield_targets(
                    occurrence.stream(),
                    class,
                    occurrence.passage_local_index(),
                ) else {
                    return Err(StepError::ConflictInvariantViolation);
                };
                if zone != occurrence.zone() {
                    return Err(StepError::ConflictInvariantViolation);
                }
                targets.len()
            };
            for target_index in 0..target_len {
                let address = {
                    let Some(policy) = step.read_view().policy() else {
                        return Err(StepError::ConflictInvariantViolation);
                    };
                    let Some((_, targets)) = policy.yield_targets(
                        occurrence.stream(),
                        class,
                        occurrence.passage_local_index(),
                    ) else {
                        return Err(StepError::ConflictInvariantViolation);
                    };
                    let Some(target) = targets.get(target_index) else {
                        return Err(StepError::ConflictInvariantViolation);
                    };
                    ConflictPassageAddress::new(
                        occurrence.zone(),
                        target.stream(),
                        target.passage_local_index(),
                    )
                };
                step.workspace
                    .frontier_maintenance
                    .scratch_demanded
                    .try_reserve(1)
                    .map_err(|_| StepError::ConflictScratchAllocFailed)?;
                step.workspace
                    .frontier_maintenance
                    .scratch_demanded
                    .push(address);
            }
        }
        occurrence_cursor += batch;
        occurrence_count += batch;
        if batch < occurrences.len() {
            break;
        }
    }
    let _ = occurrence_count;
    Ok(())
}

fn walk_vehicle(
    step: &mut StepWorkspace<'_>,
    vehicle: VehicleHandle,
    state: VehicleState,
    sequence: u32,
    horizon_ms: u64,
    wanted: Option<&[ConflictPassageAddress]>,
    record: bool,
) -> Result<(), StepError> {
    if record {
        record_walk(step, vehicle, state, sequence, horizon_ms, wanted)
    } else {
        replay_walk(step, vehicle, state, sequence, horizon_ms, wanted)
    }
}

fn record_walk(
    step: &mut StepWorkspace<'_>,
    vehicle: VehicleHandle,
    state: VehicleState,
    sequence: u32,
    horizon_ms: u64,
    wanted: Option<&[ConflictPassageAddress]>,
) -> Result<(), StepError> {
    #[cfg(test)]
    step.workspace
        .frontier_maintenance
        .full_walked
        .push(vehicle);
    let profile = step
        .binding
        .revision
        .traffic()
        .relations()
        .vehicle_profile(state.profile)
        .ok_or(StepError::ConflictInvariantViolation)?;
    let max_accel = profile.max_accel();
    let (hold, gate_distance) = {
        let read = step.read_view();
        let hold = signal_hold(read, vehicle, &state);
        let gate_distance = read
            .compiled_route(state.route)
            .map(|compiled| gate_distance_mm(compiled, &state))
            .unwrap_or(NO_DISTANCE_MM);
        (hold, gate_distance)
    };
    let (recorded, first_excluded) = {
        let (compiled, mut conflict) = step
            .committed
            .prepare_conflict_for_route(
                &mut step.derived,
                &mut step.workspace.conflict,
                state.route,
            )
            .ok_or(StepError::ConflictInvariantViolation)?;
        let first_conflict = compiled.conflicts.partition_point(|occurrence| {
            (
                occurrence.entry.route_edge_index,
                occurrence.entry.progress_mm,
            ) < (state.route_edge_index, state.progress_mm)
        });
        if first_conflict == compiled.conflicts.len() {
            (Vec::new(), NO_DISTANCE_MM)
        } else {
            let prepared =
                PreparedApproachEta::new(state.carry_um, state.speed_mm_s, max_accel, horizon_ms);
            // 只为实际留在时窗内的出现项增长。第一项就在时窗外时，空缓存不占后缀容量。
            let mut recorded = Vec::new();
            let mut first_excluded = NO_DISTANCE_MM;
            for occurrence in &compiled.conflicts[first_conflict..] {
                #[cfg(test)]
                crate::kernel::conflict::count_conflict_work(|counts| counts.visited_passages += 1);
                let Some(distance_mm) = finite_entry_distance(compiled, &state, occurrence) else {
                    continue;
                };
                let kinematic = prepared.map_or(ApproachEstimate::Unprovable, |prepared| {
                    prepared.lower_bound(u64::from(distance_mm))
                });
                if kinematic == ApproachEstimate::OutsideHorizon {
                    first_excluded = distance_mm;
                    break;
                }
                recorded
                    .try_reserve(1)
                    .map_err(|_| StepError::ConflictScratchAllocFailed)?;
                recorded.push(CachedCell {
                    address: occurrence.address(),
                    distance_mm,
                });
                let estimate = raise_signal_bound(kinematic, hold, distance_mm, horizon_ms);
                if estimate == ApproachEstimate::OutsideHorizon {
                    continue;
                }
                if wanted.is_some_and(|wanted| !address_wanted(wanted, occurrence.address())) {
                    continue;
                }
                #[cfg(test)]
                step.workspace.frontier_maintenance.insertions.push((
                    occurrence.address(),
                    vehicle,
                    estimate,
                ));
                insert_owner(
                    &mut conflict,
                    occurrence.address(),
                    vehicle,
                    sequence,
                    estimate,
                )?;
            }
            (recorded, first_excluded)
        }
    };
    step.workspace.frontier_maintenance.store(
        vehicle,
        &state,
        first_excluded,
        recorded,
        gate_distance,
        max_accel,
    )
}

fn replay_walk(
    step: &mut StepWorkspace<'_>,
    vehicle: VehicleHandle,
    state: VehicleState,
    sequence: u32,
    horizon_ms: u64,
    wanted: Option<&[ConflictPassageAddress]>,
) -> Result<(), StepError> {
    let Some(stored_progress) = step
        .workspace
        .frontier_maintenance
        .replay_hit(vehicle, &state, horizon_ms)
    else {
        return record_walk(step, vehicle, state, sequence, horizon_ms, wanted);
    };
    let cell_count = usize::try_from(vehicle.index())
        .ok()
        .and_then(|index| {
            step.workspace
                .frontier_maintenance
                .slots
                .get(index)
                .map(|slot| slot.cells.len())
        })
        .unwrap_or(0);
    step.workspace.frontier_maintenance.scratch_cells.clear();
    step.workspace
        .frontier_maintenance
        .scratch_cells
        .try_reserve(cell_count)
        .map_err(|_| StepError::ConflictScratchAllocFailed)?;
    for index in 0..cell_count {
        let Some(cell) = usize::try_from(vehicle.index())
            .ok()
            .and_then(|slot_index| {
                step.workspace
                    .frontier_maintenance
                    .slots
                    .get(slot_index)
                    .and_then(|slot| slot.cells.get(index))
                    .copied()
            })
        else {
            break;
        };
        step.workspace.frontier_maintenance.scratch_cells.push(cell);
    }
    let cells = std::mem::take(&mut step.workspace.frontier_maintenance.scratch_cells);
    let profile = step
        .binding
        .revision
        .traffic()
        .relations()
        .vehicle_profile(state.profile)
        .ok_or(StepError::ConflictInvariantViolation)?;
    let hold = signal_hold(step.read_view(), vehicle, &state);
    let prepared = PreparedApproachEta::new(
        state.carry_um,
        state.speed_mm_s,
        profile.max_accel(),
        horizon_ms,
    );
    let traveled = state.progress_mm.saturating_sub(stored_progress);
    {
        let mut conflict = step
            .committed
            .prepare_conflict(&mut step.derived, &mut step.workspace.conflict);
        for cell in &cells {
            if traveled > cell.distance_mm {
                continue;
            }
            let remaining = cell.distance_mm - traveled;
            let kinematic = prepared.map_or(ApproachEstimate::Unprovable, |prepared| {
                prepared.lower_bound(u64::from(remaining))
            });
            if kinematic == ApproachEstimate::OutsideHorizon {
                break;
            }
            let estimate = raise_signal_bound(kinematic, hold, remaining, horizon_ms);
            if estimate == ApproachEstimate::OutsideHorizon {
                continue;
            }
            if wanted.is_some_and(|wanted| !address_wanted(wanted, cell.address)) {
                continue;
            }
            #[cfg(test)]
            step.workspace
                .frontier_maintenance
                .insertions
                .push((cell.address, vehicle, estimate));
            insert_owner(&mut conflict, cell.address, vehicle, sequence, estimate)?;
        }
    }
    step.workspace.frontier_maintenance.scratch_cells = cells;
    Ok(())
}

fn finite_entry_distance(
    compiled: &super::tables::CompiledRoute,
    state: &VehicleState,
    occurrence: &ConflictPassageOccurrence,
) -> Option<u32> {
    let BoundedDistance::Finite(distance_mm) = distance_to_occurrence_progress(
        &compiled.occurrence_segments,
        &compiled.occurrence_offsets,
        &compiled.segment_totals,
        usize::try_from(state.route_edge_index).ok()?,
        state.progress_mm,
        usize::try_from(occurrence.entry.route_edge_index).ok()?,
        occurrence.entry.progress_mm,
    )?
    else {
        return None;
    };
    Some(distance_mm)
}

fn insert_owner(
    conflict: &mut super::conflict::ConflictResolution<'_>,
    address: ConflictPassageAddress,
    vehicle: VehicleHandle,
    sequence: u32,
    estimate: ApproachEstimate,
) -> Result<(), StepError> {
    conflict
        .insert_approach_owner_reduced(address, vehicle, sequence, estimate)
        .map_err(|error| match error {
            super::conflict::ConflictAcquireError::ScratchAllocFailed => {
                StepError::ConflictScratchAllocFailed
            }
            _ => StepError::ConflictInvariantViolation,
        })?;
    #[cfg(test)]
    crate::kernel::conflict::count_conflict_work(|counts| counts.frontier_updates += 1);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::SignalHold;
    use super::raise_signal_bound;
    use crate::kernel::conflict::conflict_work_counts;
    use crate::kernel::conflict::reset_conflict_work_counts;
    use crate::{ApproachEstimate, TickInput};

    #[test]
    fn red_light_bound_is_not_earlier_than_release_and_skips_nearer_entries() {
        let hold = SignalHold {
            gate_mm: 0,
            delay_ms: Some(4_000),
        };
        assert_eq!(
            raise_signal_bound(ApproachEstimate::Finite(0), Some(hold), 0, 5_000),
            ApproachEstimate::Finite(4_000)
        );
        assert_eq!(
            raise_signal_bound(
                ApproachEstimate::Finite(0),
                Some(SignalHold {
                    gate_mm: 10,
                    delay_ms: Some(4_000),
                }),
                0,
                5_000
            ),
            ApproachEstimate::Finite(0)
        );
        assert_eq!(
            raise_signal_bound(
                ApproachEstimate::Finite(100),
                Some(SignalHold {
                    gate_mm: 0,
                    delay_ms: None,
                }),
                0,
                5_000
            ),
            ApproachEstimate::Finite(100)
        );
        assert_eq!(
            raise_signal_bound(
                ApproachEstimate::Finite(100),
                Some(SignalHold {
                    gate_mm: 0,
                    delay_ms: Some(8_000),
                }),
                0,
                5_000
            ),
            ApproachEstimate::OutsideHorizon
        );
        assert_eq!(
            raise_signal_bound(ApproachEstimate::Finite(0), None, 0, 5_000),
            ApproachEstimate::Finite(0)
        );
        assert_eq!(
            raise_signal_bound(ApproachEstimate::Unprovable, Some(hold), 0, 5_000),
            ApproachEstimate::Unprovable
        );
    }

    fn stationary_scale_world(vehicles: u32) -> crate::TrafficWorld {
        let revision = crate::admin::cutover_migration::tests::conflict_scale_revision();
        let mut world =
            crate::admin::cutover_migration::tests::conflict_scale_world(revision, vehicles);
        for slot in &mut world.state.committed.vehicles {
            if let Some(state) = slot.state.as_mut() {
                state.speed_mm_s = 0;
                state.carry_um = 0;
            }
        }
        world
    }

    fn step(world: &mut crate::TrafficWorld) {
        world
            .step(TickInput::new(
                world.state.binding.config.fixed_delta_time_ms(),
            ))
            .expect("frontier step");
    }

    #[test]
    fn warm_cached_vehicle_does_not_rescan_its_route_suffix() {
        let mut world = stationary_scale_world(1);
        step(&mut world);
        reset_conflict_work_counts();
        step(&mut world);
        assert_eq!(conflict_work_counts().visited_passages, 0);
        assert!(world.state.workspace.frontier_maintenance.seeded);
    }

    #[test]
    fn lifecycle_increment_forces_a_full_walk_this_step() {
        let mut world = stationary_scale_world(1);
        step(&mut world);
        step(&mut world);
        let vehicle = world.state.committed.live_order[0];
        world
            .state
            .workspace
            .frontier_maintenance
            .note_active_source(vehicle);
        reset_conflict_work_counts();
        step(&mut world);
        assert!(
            world
                .state
                .workspace
                .frontier_maintenance
                .full_walked
                .contains(&vehicle)
        );
        assert!(conflict_work_counts().visited_passages > 0);
    }

    #[test]
    fn recycled_slot_is_an_approach_source_on_the_next_step() {
        let mut world = stationary_scale_world(2);
        step(&mut world);
        let old = world.state.committed.live_order[1];
        let state = world
            .state
            .vehicle_state(old)
            .copied()
            .expect("rear vehicle");
        world.despawn_vehicle(old).expect("despawn rear vehicle");
        let spawned = world
            .spawn_vehicle(crate::VehicleSpawnInput::new(
                state.profile,
                state.route,
                state.route_edge_index,
                state.progress_mm,
                0,
            ))
            .expect("spawn recycled vehicle");
        assert_ne!(spawned.generation(), old.generation());
        reset_conflict_work_counts();
        step(&mut world);
        assert!(
            world
                .state
                .workspace
                .frontier_maintenance
                .full_walked
                .contains(&spawned)
        );
        assert!(
            !world
                .state
                .workspace
                .frontier_maintenance
                .full_walked
                .contains(&old)
        );
    }

    fn scale_edge(
        revision: &laneflow_static_network::SharedNetworkRevision,
        key: &str,
    ) -> laneflow_static_contract::LaneEdgeOrdinal {
        let limits = laneflow_compiler::CompileLimits::single_network_1m_v2();
        let stable = laneflow_compiler::derive_canonical_stable_id_v1(
            laneflow_static_contract::EntityKind::LaneEdge,
            "city/runtime-live-conflict-cutover",
            key,
            &limits,
        )
        .expect("scale edge identity");
        revision
            .identity()
            .ordinal(laneflow_static_contract::LaneEdgeId::from_untyped(stable))
            .expect("scale edge ordinal")
    }

    fn set_pose(
        world: &mut crate::TrafficWorld,
        vehicle: crate::VehicleHandle,
        route_edge_index: u32,
        progress_mm: u32,
        speed_mm_s: u32,
    ) {
        let state = world.state.committed.vehicles[vehicle.index() as usize]
            .state
            .as_mut()
            .expect("pose vehicle");
        state.route_edge_index = route_edge_index;
        state.progress_mm = progress_mm;
        state.carry_um = 0;
        state.speed_mm_s = speed_mm_s;
    }

    fn route_edge_index(
        world: &crate::TrafficWorld,
        route: crate::RouteHandle,
        edge: laneflow_static_contract::LaneEdgeOrdinal,
    ) -> u32 {
        let compiled = world.state.compiled_route(route).expect("compiled route");
        u32::try_from(
            compiled
                .edges
                .iter()
                .position(|candidate| *candidate == edge)
                .expect("edge on route"),
        )
        .expect("route index")
    }

    struct FailpointReset;

    impl Drop for FailpointReset {
        fn drop(&mut self) {
            super::LINK_RESERVE_SUCCESSES.with(|remaining| remaining.set(usize::MAX));
            super::LINK_COMMIT_SUCCESSES.with(|remaining| remaining.set(usize::MAX));
        }
    }

    fn sorted_insertions(
        world: &crate::TrafficWorld,
    ) -> Vec<(
        crate::ConflictPassageAddress,
        crate::VehicleHandle,
        crate::ApproachEstimate,
    )> {
        let mut rows = world
            .state
            .workspace
            .frontier_maintenance
            .insertions
            .clone();
        rows.sort_by_key(|(address, vehicle, estimate)| {
            (
                *address,
                vehicle.index(),
                vehicle.generation(),
                match estimate {
                    crate::ApproachEstimate::Unprovable => (0_u8, 0_u64),
                    crate::ApproachEstimate::Finite(ms) => (1, *ms),
                    crate::ApproachEstimate::OutsideHorizon => (2, 0),
                },
            )
        });
        rows
    }

    #[test]
    fn conflict_that_enters_the_horizon_after_motion_is_visible_next_step() {
        let revision = crate::admin::cutover_migration::tests::conflict_scale_revision();
        let mut world =
            crate::admin::cutover_migration::tests::conflict_scale_world_with_route_capacity(
                revision.clone(),
                2,
                2,
            );
        let querier = world.state.committed.live_order[0];
        let spare = world.state.committed.live_order[1];
        world.despawn_vehicle(spare).expect("free a slot");
        let querier_route = world.state.vehicle_state(querier).expect("querier").route;
        // 先停在远离路口的位置，避免这一拍先拿到 reservation，下一拍就不再收集让行目标。
        set_pose(&mut world, querier, 0, 0, 0);
        let other_route = world
            .register_route(crate::RouteRegisterInput::new(
                ["other-entry", "other-internal", "other-exit"]
                    .into_iter()
                    .map(|key| scale_edge(&revision, key))
                    .collect::<Vec<_>>(),
            ))
            .expect("other route");
        let profile = world.state.vehicle_state(querier).expect("querier").profile;
        let source = world
            .spawn_vehicle(crate::VehicleSpawnInput::new(profile, other_route, 0, 0, 0))
            .expect("source");
        step(&mut world);
        let remembered = world.state.workspace.frontier_maintenance.slots[source.index() as usize]
            .first_excluded_mm;
        assert_ne!(
            remembered,
            super::NO_DISTANCE_MM,
            "source must start outside the horizon"
        );
        let horizon = world
            .frontier_proof_horizon_ms()
            .expect("scale world has a proof horizon");
        let accel = world
            .traffic()
            .relations()
            .vehicle_profile(profile)
            .expect("profile")
            .max_accel();
        let speed = 10_000_u32;
        let reach = super::horizon_reach_mm(speed, accel, horizon);
        let progress = remembered as f64 - reach - 20.0;
        assert!(
            progress > 0.0,
            "excluded {remembered} reach {reach} must leave room on the edge"
        );
        let progress_mm = progress.round() as u32;
        set_pose(&mut world, source, 0, progress_mm, speed);
        step(&mut world);
        assert!(
            !world
                .state
                .workspace
                .frontier_maintenance
                .insertions
                .iter()
                .any(|(_, vehicle, _)| *vehicle == source),
            "frontier still runs before this tick's motion"
        );
        assert!(
            world
                .state
                .workspace
                .frontier_maintenance
                .ready_invalid
                .contains(&source),
            "motion that brings the excluded conflict inside the horizon must invalidate"
        );
        place_scale_querier_at_gate(&mut world, &revision, querier, querier_route);
        world
            .state
            .workspace
            .frontier_maintenance
            .ready_near
            .push(querier);
        step(&mut world);
        assert!(
            world
                .state
                .workspace
                .frontier_maintenance
                .insertions
                .iter()
                .any(|(_, vehicle, estimate)| {
                    *vehicle == source
                        && !matches!(estimate, crate::ApproachEstimate::OutsideHorizon)
                }),
            "next yield query must see the source, insertions={:?}",
            world.state.workspace.frontier_maintenance.insertions
        );
    }

    #[test]
    fn stale_invalid_handle_does_not_hide_the_recycled_generation() {
        let revision = crate::admin::cutover_migration::tests::conflict_scale_revision();
        let mut world =
            crate::admin::cutover_migration::tests::conflict_scale_world_with_route_capacity(
                revision.clone(),
                2,
                2,
            );
        let querier = world.state.committed.live_order[0];
        let spare = world.state.committed.live_order[1];
        world.despawn_vehicle(spare).expect("free a slot");
        let querier_route = world.state.vehicle_state(querier).expect("querier").route;
        set_pose(&mut world, querier, 0, 0, 0);
        let other_route = world
            .register_route(crate::RouteRegisterInput::new(
                ["other-entry", "other-internal", "other-exit"]
                    .into_iter()
                    .map(|key| scale_edge(&revision, key))
                    .collect::<Vec<_>>(),
            ))
            .expect("other route");
        let profile = world.state.vehicle_state(querier).expect("querier").profile;
        let internal =
            route_edge_index(&world, other_route, scale_edge(&revision, "other-internal"));
        let old = world
            .spawn_vehicle(crate::VehicleSpawnInput::new(
                profile,
                other_route,
                internal,
                0,
                0,
            ))
            .expect("source");
        step(&mut world);
        let excluded = world.state.workspace.frontier_maintenance.slots[old.index() as usize]
            .first_excluded_mm;
        if excluded != super::NO_DISTANCE_MM {
            set_pose(&mut world, old, internal, excluded.saturating_sub(200), 0);
        }
        step(&mut world);
        world
            .state
            .workspace
            .frontier_maintenance
            .ready_invalid
            .push(old);
        let state = world
            .state
            .vehicle_state(old)
            .copied()
            .expect("source state");
        world.despawn_vehicle(old).expect("despawn source");
        let spawned = world
            .spawn_vehicle(crate::VehicleSpawnInput::new(
                state.profile,
                state.route,
                state.route_edge_index,
                0,
                0,
            ))
            .expect("recycled source");
        set_pose(
            &mut world,
            spawned,
            state.route_edge_index,
            state.progress_mm,
            0,
        );
        assert_eq!(spawned.index(), old.index());
        assert_ne!(spawned.generation(), old.generation());
        place_scale_querier_at_gate(&mut world, &revision, querier, querier_route);
        world
            .state
            .workspace
            .frontier_maintenance
            .ready_near
            .push(querier);
        step(&mut world);
        assert!(
            world
                .state
                .workspace
                .frontier_maintenance
                .insertions
                .iter()
                .any(|(_, vehicle, _)| *vehicle == spawned),
            "new generation must be inserted, insertions={:?}",
            world.state.workspace.frontier_maintenance.insertions
        );
        assert!(
            !world
                .state
                .workspace
                .frontier_maintenance
                .insertions
                .iter()
                .any(|(_, vehicle, _)| *vehicle == old)
        );
    }

    #[test]
    fn link_reserve_failure_retries_on_the_same_committed_state() {
        let _reset = FailpointReset;
        let revision = crate::admin::cutover_migration::tests::conflict_scale_revision();
        let mut failed =
            crate::admin::cutover_migration::tests::conflict_scale_world(revision.clone(), 4);
        let mut reference =
            crate::admin::cutover_migration::tests::conflict_scale_world(revision, 4);
        step(&mut failed);
        step(&mut failed);
        step(&mut reference);
        step(&mut reference);
        let vehicle = failed
            .state
            .committed
            .live_order
            .iter()
            .copied()
            .find(|handle| {
                !failed.state.workspace.frontier_maintenance.slots[handle.index() as usize]
                    .cells
                    .is_empty()
            })
            .expect("cached cells");
        failed
            .state
            .workspace
            .frontier_maintenance
            .ready_invalid
            .push(vehicle);
        reference
            .state
            .workspace
            .frontier_maintenance
            .ready_invalid
            .push(vehicle);
        let time_ms = failed.state.committed.time_ms;
        super::LINK_RESERVE_SUCCESSES.with(|remaining| remaining.set(0));
        assert_eq!(
            failed.step(TickInput::new(4)),
            Err(crate::StepError::ConflictScratchAllocFailed)
        );
        assert_eq!(failed.state.committed.time_ms, time_ms);
        assert!(failed.state.workspace.frontier_maintenance.seeded);
        super::LINK_RESERVE_SUCCESSES.with(|remaining| remaining.set(usize::MAX));
        step(&mut failed);
        step(&mut reference);
        assert_eq!(sorted_insertions(&failed), sorted_insertions(&reference));
    }

    #[test]
    fn link_commit_failure_retries_as_a_full_rebuild() {
        let _reset = FailpointReset;
        let revision = crate::admin::cutover_migration::tests::conflict_scale_revision();
        let mut failed =
            crate::admin::cutover_migration::tests::conflict_scale_world(revision.clone(), 4);
        let mut reference =
            crate::admin::cutover_migration::tests::conflict_scale_world(revision, 4);
        step(&mut failed);
        step(&mut failed);
        step(&mut reference);
        step(&mut reference);
        let vehicle = failed
            .state
            .committed
            .live_order
            .iter()
            .copied()
            .find(|handle| {
                !failed.state.workspace.frontier_maintenance.slots[handle.index() as usize]
                    .cells
                    .is_empty()
            })
            .expect("cached cells");
        failed
            .state
            .workspace
            .frontier_maintenance
            .ready_invalid
            .push(vehicle);
        let time_ms = failed.state.committed.time_ms;
        super::LINK_COMMIT_SUCCESSES.with(|remaining| remaining.set(0));
        assert_eq!(
            failed.step(TickInput::new(4)),
            Err(crate::StepError::ConflictScratchAllocFailed)
        );
        assert_eq!(failed.state.committed.time_ms, time_ms);
        assert!(!failed.state.workspace.frontier_maintenance.seeded);
        super::LINK_COMMIT_SUCCESSES.with(|remaining| remaining.set(usize::MAX));
        reference.state.workspace.frontier_maintenance.seeded = false;
        step(&mut failed);
        step(&mut reference);
        assert_eq!(sorted_insertions(&failed), sorted_insertions(&reference));
    }

    fn place_scale_querier_at_gate(
        world: &mut crate::TrafficWorld,
        revision: &laneflow_static_network::SharedNetworkRevision,
        querier: crate::VehicleHandle,
        route: crate::RouteHandle,
    ) {
        let entry = scale_edge(revision, "entry");
        let index = route_edge_index(world, route, entry);
        let length = world.traffic().lane_lengths_millimetres()[entry.index()];
        set_pose(world, querier, index, length.saturating_sub(1), 0);
    }

    fn signal_vehicle(
        world: &mut crate::TrafficWorld,
        route: crate::RouteHandle,
        route_edge_index: u32,
        progress_mm: u32,
    ) -> crate::VehicleHandle {
        let vehicle = world
            .spawn_vehicle(crate::VehicleSpawnInput::new(
                laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                route,
                route_edge_index,
                progress_mm,
                0,
            ))
            .expect("signal vehicle");
        set_pose(world, vehicle, route_edge_index, progress_mm, 0);
        vehicle
    }

    fn finite_estimates(world: &crate::TrafficWorld, vehicle: crate::VehicleHandle) -> Vec<u64> {
        world
            .state
            .workspace
            .frontier_maintenance
            .insertions
            .iter()
            .filter_map(|(_, owner, estimate)| {
                (*owner == vehicle)
                    .then_some(estimate)
                    .and_then(|estimate| {
                        if let crate::ApproachEstimate::Finite(ms) = estimate {
                            Some(*ms)
                        } else {
                            None
                        }
                    })
            })
            .collect()
    }

    #[test]
    fn red_stop_line_on_a_rolled_cursor_is_not_earlier_than_release() {
        let (mut world, route) = crate::admin::cutover_migration::tests::signal_frontier_world(
            4_000,
            Some(4_000),
            false,
        );
        let rolled = signal_vehicle(&mut world, route, 1, 0);
        step_ms(&mut world, 100);
        let estimates = finite_estimates(&world, rolled);
        assert!(
            estimates.contains(&4_000),
            "stop-line arrival must wait for release, estimates={estimates:?}"
        );
        assert!(estimates.iter().all(|ms| *ms != 0));

        crate::admin::format_admission::tests::install_conflict_reservation(
            &mut world, route, rolled,
        );
        set_pose(&mut world, rolled, 1, 0, 0);
        world.state.workspace.frontier_maintenance.seeded = false;
        step_ms(&mut world, 100);
        let estimates = finite_estimates(&world, rolled);
        assert!(
            estimates.iter().all(|ms| *ms != 4_000),
            "an existing reservation is not delayed to the release, estimates={estimates:?}"
        );
    }

    #[test]
    fn red_phase_without_a_release_keeps_the_kinematic_bound() {
        let (mut world, route) =
            crate::admin::cutover_migration::tests::signal_frontier_world(4_000, None, false);
        let rolled = signal_vehicle(&mut world, route, 1, 0);
        step_ms(&mut world, 100);
        let estimates = finite_estimates(&world, rolled);
        assert!(!estimates.is_empty(), "the gate conflict must be inserted");
        assert!(
            estimates.iter().all(|ms| *ms < 1_000),
            "no release in the cycle keeps the kinematic bound, estimates={estimates:?}"
        );
    }

    #[test]
    fn release_in_the_next_cycle_still_raises_the_bound() {
        let (mut world, route) =
            crate::admin::cutover_migration::tests::signal_frontier_world(4_000, Some(200), true);
        // 绿灯阶段不放车，避免先拿到 reservation。时钟进入红灯后再停到停止线上。
        step_ms(&mut world, 100);
        step_ms(&mut world, 100);
        assert_eq!(world.state.committed.time_ms, 200);
        let rolled = signal_vehicle(&mut world, route, 1, 0);
        world.state.workspace.frontier_maintenance.seeded = false;
        step_ms(&mut world, 100);
        let estimates = finite_estimates(&world, rolled);
        assert!(
            estimates.contains(&4_000),
            "red phase must wait for the green phase in the next cycle, estimates={estimates:?}"
        );
    }

    #[test]
    fn outside_horizon_cache_does_not_reserve_the_route_suffix() {
        let mut world = stationary_scale_world(4);
        step(&mut world);
        let mut outside = 0_u32;
        for handle in world.state.committed.live_order.clone() {
            let slot = &world.state.workspace.frontier_maintenance.slots[handle.index() as usize];
            if !slot.valid || slot.first_excluded_mm == super::NO_DISTANCE_MM {
                continue;
            }
            assert!(
                slot.cells.is_empty(),
                "an excluded first conflict keeps no cells"
            );
            assert_eq!(
                slot.cells.capacity(),
                0,
                "an empty cache must not keep capacity for the route suffix"
            );
            outside += 1;
        }
        assert!(
            outside > 0,
            "the rear vehicles must sit outside the proof horizon"
        );
        let cached_bytes = world
            .state
            .workspace
            .frontier_maintenance
            .slots
            .iter()
            .map(|slot| crate::kernel::state::vec_bytes(&slot.cells))
            .sum::<u64>();
        assert_eq!(cached_bytes, 0);
    }

    #[test]
    fn duplicate_passage_address_does_not_allocate_while_committing_links() {
        let _reset = FailpointReset;
        let mut maintenance = super::FrontierMaintenance {
            seeded: true,
            ..super::FrontierMaintenance::default()
        };
        let address = crate::ConflictPassageAddress::new(
            laneflow_static_contract::ConflictZoneOrdinal::from_raw(1),
            laneflow_static_contract::ParticipantStreamOrdinal::from_raw(2),
            3,
        );
        let mut existing = vec![7_u32];
        while existing.len() < existing.capacity() {
            existing.push(7);
        }
        let full = existing.len();
        assert_eq!(existing.capacity(), full);
        maintenance.by_cell.insert(address, existing);
        let duplicates = full.saturating_mul(2).max(2);
        let cells = (0..duplicates)
            .map(|distance| super::CachedCell {
                address,
                distance_mm: u32::try_from(distance).expect("duplicate distance"),
            })
            .collect::<Vec<_>>();
        maintenance
            .load_unique_addresses(&cells)
            .expect("project duplicate addresses");
        assert_eq!(maintenance.scratch_addresses.as_slice(), &[address]);
        super::LINK_RESERVE_SUCCESSES.with(|remaining| remaining.set(0));
        assert_eq!(
            maintenance.reserve_links(),
            Err(crate::StepError::ConflictScratchAllocFailed)
        );
        assert!(maintenance.seeded);
        assert_eq!(
            maintenance.by_cell.get(&address).map(Vec::as_slice),
            Some([7_u32].as_slice())
        );
        super::LINK_RESERVE_SUCCESSES.with(|remaining| remaining.set(usize::MAX));
        maintenance.reserve_links().expect("reserve unique link");
        let reserved = maintenance
            .by_cell
            .get(&address)
            .expect("address")
            .capacity();
        let mut old = Vec::new();
        maintenance
            .commit_links(3, &mut old)
            .expect("commit unique link");
        let list = maintenance.by_cell.get(&address).expect("address");
        assert_eq!(
            list.capacity(),
            reserved,
            "commit must use the reserved slot"
        );
        assert_eq!(list.iter().filter(|slot| **slot == 3).count(), 1);
        assert!(list.contains(&7));
    }

    #[test]
    fn distinct_address_projection_does_not_compare_quadratically() {
        let small = distinct_address_comparisons(256);
        let large = distinct_address_comparisons(512);
        assert!(
            small > 0,
            "sorting distinct addresses must compare them, comparisons={small}"
        );
        assert!(
            large.saturating_mul(2) < small.saturating_mul(7),
            "doubling distinct addresses must stay below quadratic growth: {small} -> {large}"
        );
    }

    #[test]
    fn shared_address_links_do_not_scan_members_quadratically() {
        let (small_comparisons, small_probes) = shared_address_work(128);
        let (large_comparisons, large_probes) = shared_address_work(256);
        assert_eq!(
            small_probes, 0,
            "first insert must not compare existing members"
        );
        assert_eq!(
            large_probes, 0,
            "doubling vehicles on one address must not compare existing members"
        );
        assert!(
            small_comparisons > 0,
            "duplicate passages still need one address projection, comparisons={small_comparisons}"
        );
        assert!(
            large_comparisons.saturating_mul(2) < small_comparisons.saturating_mul(7),
            "doubling vehicles on one address must stay below quadratic growth: {small_comparisons} -> {large_comparisons}"
        );
    }

    fn distinct_address_comparisons(count: u32) -> usize {
        super::ADDRESS_COMPARISONS.with(|comparisons| comparisons.set(0));
        super::MEMBER_PROBES.with(|probes| probes.set(0));
        let mut maintenance = super::FrontierMaintenance {
            seeded: true,
            ..super::FrontierMaintenance::default()
        };
        let cells = (0..count)
            .rev()
            .map(|local| super::CachedCell {
                address: passage_address(local),
                distance_mm: local,
            })
            .collect::<Vec<_>>();
        maintenance
            .load_unique_addresses(&cells)
            .expect("project distinct addresses");
        assert_eq!(maintenance.scratch_addresses.len(), cells.len());
        assert!(maintenance.scratch_addresses.is_sorted());
        assert_eq!(
            cells.first().map(|cell| cell.distance_mm),
            Some(count.saturating_sub(1)),
            "route order stays on the cached cells"
        );
        assert_eq!(cells.last().map(|cell| cell.distance_mm), Some(0));
        maintenance
            .reserve_links()
            .expect("reserve distinct addresses");
        let mut old = Vec::new();
        maintenance
            .commit_links(0, &mut old)
            .expect("commit distinct addresses");
        assert_eq!(super::MEMBER_PROBES.with(std::cell::Cell::get), 0);
        for address in &maintenance.scratch_addresses {
            assert_eq!(
                maintenance.by_cell.get(address).map(Vec::as_slice),
                Some([0_u32].as_slice())
            );
        }
        super::ADDRESS_COMPARISONS.with(std::cell::Cell::get)
    }

    fn shared_address_work(vehicles: u32) -> (usize, usize) {
        super::ADDRESS_COMPARISONS.with(|comparisons| comparisons.set(0));
        super::MEMBER_PROBES.with(|probes| probes.set(0));
        let mut maintenance = super::FrontierMaintenance {
            seeded: true,
            ..super::FrontierMaintenance::default()
        };
        let address = passage_address(4);
        for index in 0..vehicles {
            let cells = [
                super::CachedCell {
                    address,
                    distance_mm: index,
                },
                super::CachedCell {
                    address,
                    distance_mm: index.saturating_add(1),
                },
            ];
            maintenance
                .load_unique_addresses(&cells)
                .expect("project shared address");
            assert_eq!(maintenance.scratch_addresses.as_slice(), &[address]);
            maintenance.reserve_links().expect("reserve shared address");
            let mut old = Vec::new();
            maintenance
                .commit_links(index, &mut old)
                .expect("commit shared address");
        }
        let list = maintenance.by_cell.get(&address).expect("shared address");
        assert_eq!(
            list.len(),
            usize::try_from(vehicles).expect("vehicle count")
        );
        for index in 0..vehicles {
            assert_eq!(list.iter().filter(|slot| **slot == index).count(), 1);
        }
        (
            super::ADDRESS_COMPARISONS.with(std::cell::Cell::get),
            super::MEMBER_PROBES.with(std::cell::Cell::get),
        )
    }

    fn passage_address(local: u32) -> crate::ConflictPassageAddress {
        crate::ConflictPassageAddress::new(
            laneflow_static_contract::ConflictZoneOrdinal::from_raw(1),
            laneflow_static_contract::ParticipantStreamOrdinal::from_raw(2),
            local,
        )
    }

    fn step_ms(world: &mut crate::TrafficWorld, delta_ms: u64) {
        world
            .step(TickInput::new(delta_ms))
            .expect("signal frontier step");
    }
}
