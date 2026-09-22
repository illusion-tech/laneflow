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
    increments: Vec<VehicleHandle>,
    /// 本拍工作副本。容量跨拍保留，稳态不再分配。
    scratch_near: Vec<VehicleHandle>,
    scratch_invalid: Vec<VehicleHandle>,
    scratch_increments: Vec<VehicleHandle>,
    scratch_states: Vec<(VehicleHandle, VehicleState)>,
    scratch_demanded: Vec<ConflictPassageAddress>,
    scratch_cells: Vec<CachedCell>,
    seen: Vec<u32>,
    seen_gen: u32,
    #[cfg(test)]
    full_walked: Vec<VehicleHandle>,
    #[cfg(test)]
    insertions: Vec<(VehicleHandle, ApproachEstimate)>,
}

impl FrontierMaintenance {
    /// 记录本拍必须成为接近来源的活动车辆，并拆掉旧槽位缓存。
    pub(crate) fn note_active_source(&mut self, vehicle: VehicleHandle) {
        self.unlink(vehicle.index());
        self.increments
            .retain(|existing| existing.index() != vehicle.index());
        self.increments.push(vehicle);
    }

    /// 车辆离开 Active 或槽位被释放后，不再作为接近来源。
    pub(crate) fn invalidate(&mut self, vehicle: VehicleHandle) {
        self.unlink(vehicle.index());
        self.increments
            .retain(|existing| existing.index() != vehicle.index());
    }

    /// 成功提交后发布本拍分类，并清空已经消费的生命周期增量。
    pub(crate) fn publish(&mut self) {
        self.ready_near.clear();
        self.ready_invalid.clear();
        std::mem::swap(&mut self.ready_near, &mut self.pending_near);
        std::mem::swap(&mut self.ready_invalid, &mut self.pending_invalid);
        self.pending_near.clear();
        self.pending_invalid.clear();
        self.increments.clear();
        self.seeded = true;
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
        self.increments.clear();
        self.scratch_near.clear();
        self.scratch_invalid.clear();
        self.scratch_increments.clear();
        self.scratch_states.clear();
        self.scratch_demanded.clear();
        self.scratch_cells.clear();
        self.seen.clear();
        self.seen_gen = 0;
    }

    fn snapshot_published_lists(&mut self) {
        let near = std::mem::take(&mut self.ready_near);
        let invalid = std::mem::take(&mut self.ready_invalid);
        let increments = std::mem::take(&mut self.increments);
        refill_handles(&mut self.scratch_near, &near);
        refill_handles(&mut self.scratch_invalid, &invalid);
        refill_handles(&mut self.scratch_increments, &increments);
        self.ready_near = near;
        self.ready_invalid = invalid;
        self.increments = increments;
    }

    fn begin_full_records(&mut self) {
        self.by_cell.clear();
        for slot in &mut self.slots {
            slot.valid = false;
            slot.cells.clear();
            slot.first_excluded_mm = NO_DISTANCE_MM;
        }
    }

    fn classify(
        &mut self,
        delta_s: f32,
        horizon_ms: Option<u64>,
        states: &[(VehicleHandle, VehicleState)],
    ) -> Result<(), StepError> {
        self.pending_near.clear();
        self.pending_invalid.clear();
        self.pending_near
            .try_reserve(states.len())
            .map_err(|_| StepError::ConflictScratchAllocFailed)?;
        self.pending_invalid
            .try_reserve(states.len())
            .map_err(|_| StepError::ConflictScratchAllocFailed)?;
        for (vehicle, state) in states {
            let Some(remembered) = self.remembered(*vehicle, state) else {
                self.pending_near.push(*vehicle);
                self.pending_invalid.push(*vehicle);
                continue;
            };
            let traveled = state.progress_mm.saturating_sub(remembered.progress);
            let reach = two_tick_reach_mm(state.speed_mm_s, remembered.max_accel, delta_s);
            let gate_remaining = remembered.gate_distance_mm.saturating_sub(traveled);
            let near = remembered.gate_distance_mm != NO_DISTANCE_MM
                && (!reach.is_finite() || f64::from(gate_remaining) <= reach);
            let dirty = horizon_ms.is_some_and(|horizon| {
                remembered.first_excluded_mm != NO_DISTANCE_MM
                    && f64::from(remembered.first_excluded_mm.saturating_sub(traveled))
                        <= horizon_reach_mm(state.speed_mm_s, remembered.max_accel, horizon)
            });
            if near {
                self.pending_near.push(*vehicle);
            }
            if dirty {
                self.pending_invalid.push(*vehicle);
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
        let old = if self.slots[index_usize].valid {
            self.slots[index_usize].cells.clone()
        } else {
            Vec::new()
        };
        self.relink(index, &old, &cells)?;
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

    fn relink(
        &mut self,
        index: u32,
        old: &[CachedCell],
        new_cells: &[CachedCell],
    ) -> Result<(), StepError> {
        for cell in old {
            if let Some(list) = self.by_cell.get_mut(&cell.address) {
                if let Some(position) = list.iter().position(|vehicle| *vehicle == index) {
                    list.swap_remove(position);
                }
                if list.is_empty() {
                    self.by_cell.remove(&cell.address);
                }
            }
        }
        for cell in new_cells {
            let list = self.by_cell.entry(cell.address).or_default();
            list.try_reserve(1)
                .map_err(|_| StepError::ConflictScratchAllocFailed)?;
            list.push(index);
        }
        Ok(())
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
        let cells = std::mem::take(&mut slot.cells);
        slot.valid = false;
        for cell in cells {
            let Some(list) = self.by_cell.get_mut(&cell.address) else {
                continue;
            };
            if let Some(position) = list.iter().position(|vehicle| *vehicle == index) {
                list.swap_remove(position);
            }
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
            increments,
            scratch_near,
            scratch_invalid,
            scratch_increments,
            scratch_states,
            scratch_demanded,
            scratch_cells,
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
            + crate::kernel::state::vec_bytes(increments)
            + crate::kernel::state::vec_bytes(scratch_near)
            + crate::kernel::state::vec_bytes(scratch_invalid)
            + crate::kernel::state::vec_bytes(scratch_increments)
            + crate::kernel::state::vec_bytes(scratch_states)
            + crate::kernel::state::vec_bytes(scratch_demanded)
            + crate::kernel::state::vec_bytes(scratch_cells)
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

fn refill_handles(destination: &mut Vec<VehicleHandle>, source: &[VehicleHandle]) {
    destination.clear();
    destination.extend_from_slice(source);
}

fn address_wanted(wanted: &[ConflictPassageAddress], address: ConflictPassageAddress) -> bool {
    wanted.binary_search(&address).is_ok()
}

fn two_tick_reach_mm(speed_mm_s: u32, max_accel_m_s2: f32, delta_s: f32) -> f64 {
    kinematic_reach_mm(
        speed_mm_s,
        max_accel_m_s2,
        f64::from(delta_s) * 2.0,
        REACH_SLACK_MM,
    )
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
    let mut hop = usize::try_from(state.route_edge_index).ok()?;
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
            let BoundedDistance::Finite(mm) = from_cursor_start.saturating_sub(state.progress_mm)
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

/// 运动准备之后，用拍初状态写下一批近门集合和失效名单。
pub(crate) fn classify_pending(
    step: &mut StepWorkspace<'_>,
    delta_s: f32,
) -> Result<(), StepError> {
    let horizon_ms = step.frontier_proof_horizon_ms();
    let count = step.derived.active_order.len();
    step.workspace.frontier_maintenance.scratch_states.clear();
    for index in 0..count {
        let handle = step.derived.active_order[index];
        let Some(state) = step.vehicle_state(handle).copied() else {
            continue;
        };
        if state.status == VehicleStatus::Active {
            step.workspace
                .frontier_maintenance
                .scratch_states
                .push((handle, state));
        }
    }
    let states = std::mem::take(&mut step.workspace.frontier_maintenance.scratch_states);
    let result = step
        .workspace
        .frontier_maintenance
        .classify(delta_s, horizon_ms, &states);
    step.workspace.frontier_maintenance.scratch_states = states;
    result
}

fn full_walk(step: &mut StepWorkspace<'_>, horizon_ms: u64) -> Result<(), StepError> {
    step.workspace.frontier_maintenance.begin_full_records();
    let vehicles = step.committed.live_order.clone();
    for (sequence, vehicle) in vehicles.into_iter().enumerate() {
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
        .snapshot_published_lists();
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
            let reusable = active_state(step, vehicle).is_some_and(|state| {
                step.workspace
                    .frontier_maintenance
                    .replay_hit(vehicle, &state, horizon_ms)
                    .is_some()
            });
            if reusable {
                walk_if_new(step, vehicle, horizon_ms, &demanded, false)?;
            } else if newly_marked(step, vehicle)? {
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
        let Some(state) = active_state(step, vehicle) else {
            continue;
        };
        let Some(sequence) = live_sequence(step, vehicle)? else {
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
    if !newly_marked(step, vehicle)? {
        return Ok(());
    }
    let Some(state) = active_state(step, vehicle) else {
        return Ok(());
    };
    let Some(sequence) = live_sequence(step, vehicle)? else {
        return Ok(());
    };
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

fn newly_marked(step: &mut StepWorkspace<'_>, vehicle: VehicleHandle) -> Result<bool, StepError> {
    step.workspace.frontier_maintenance.mark(vehicle.index())
}

fn active_state(step: &StepWorkspace<'_>, vehicle: VehicleHandle) -> Option<VehicleState> {
    let state = step.vehicle_state(vehicle).copied()?;
    (state.status == VehicleStatus::Active).then_some(state)
}

fn live_sequence(
    step: &StepWorkspace<'_>,
    vehicle: VehicleHandle,
) -> Result<Option<u32>, StepError> {
    let Some(position) = step
        .committed
        .live_order
        .iter()
        .position(|handle| *handle == vehicle)
    else {
        return Ok(None);
    };
    u32::try_from(position)
        .map(Some)
        .map_err(|_| StepError::ConflictInvariantViolation)
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
            let mut recorded = Vec::new();
            recorded
                .try_reserve(compiled.conflicts.len() - first_conflict)
                .map_err(|_| StepError::ConflictScratchAllocFailed)?;
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
}
