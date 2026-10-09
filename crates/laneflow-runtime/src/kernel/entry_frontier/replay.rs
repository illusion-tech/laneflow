//! Frontier 缓存复用：规范选择、并行求值、完整 join 后原序插入。

use super::{
    CachedCell, FrontierMaintenance, SignalHold, accepted_source, address_wanted, insert_owner,
    raise_signal_bound, signal_hold, vehicle_from_slot,
};
use crate::kernel::conflict::PreparedApproachEta;
use crate::kernel::execution::ExecutionResources;
use crate::kernel::phase::{StepReadView, StepWorkspace};
use crate::kernel::vehicle_store::{MotionPosition, MotionRead};
use crate::{ApproachEstimate, ConflictPassageAddress, StepError, VehicleHandle, VehicleState};
use laneflow_static_contract::VehicleProfileOrdinal;

#[cfg(not(test))]
const PARALLEL_ROWS: usize = 1_024;
#[cfg(test)]
const PARALLEL_ROWS: usize = 1;
const MAX_PARTS: usize = 128;

/// 本次冻结借用的计算输入；控制成员与生命周期不进入重放暂存。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Input {
    vehicle: VehicleHandle,
    route: crate::RouteHandle,
    profile: VehicleProfileOrdinal,
    position: MotionPosition,
    speed_mm_s: u32,
    sequence: u32,
    stored_progress: u32,
}

impl Input {
    fn new(state: impl MotionRead, sequence: u32, stored_progress: u32) -> Self {
        Self {
            vehicle: state.handle(),
            route: state.route(),
            profile: state.profile(),
            position: state.position(),
            speed_mm_s: state.speed_mm_s(),
            sequence,
            stored_progress,
        }
    }
}

struct Row {
    input: Input,
    cells: usize,
}

struct ComputedReplay {
    emitted: Option<usize>,
    first_error: Option<(usize, StepError)>,
}

/// 并行收集的一个地址成员：拍初已标记的成员不收集；未通过身份或 live 序检查
/// 的成员记为 `Rejected`，与串行选择一样不标记、不发出。
#[derive(Clone, Copy)]
enum Candidate {
    Rejected,
    Replay(Input),
    Deferred(crate::VehicleHandle),
}

/// 一段需求地址的收集结果；缓冲随池保留。
#[derive(Default)]
struct SelectPart {
    candidates: Vec<(u32, Candidate)>,
    members: usize,
    failed: bool,
}

struct GatheredSelection {
    used_parts: usize,
    member_upper: usize,
}

/// 只存本次选中的车辆和缓存出现项，不复制逐车缓存，也不按 worker 复制世界。
#[derive(Default)]
pub(crate) struct ReplayScratch {
    select_parts: Vec<SelectPart>,
    rows: Vec<Row>,
    estimates: Vec<ApproachEstimate>,
    /// 与 `estimates` 对齐的 cell 下标；未发出或下标无效时为 `u32::MAX`。
    cell_indices: Vec<u32>,
    /// 与 `estimates` 对齐的发出者（车辆, live 序）；只在发出项上有意义。
    /// 按 cell 分段插入时直接按项取，不再逐段回扫全部行。
    owners: Vec<(VehicleHandle, u32)>,
    #[cfg(test)]
    fail_reserve: u8,
}

impl ReplayScratch {
    #[cfg(test)]
    fn retained_bytes(&self) -> u64 {
        let Self {
            select_parts,
            rows,
            estimates,
            cell_indices,
            owners,
            fail_reserve: _,
        } = self;
        crate::kernel::state::vec_bytes(select_parts)
            + select_parts
                .iter()
                .map(|part| crate::kernel::state::vec_bytes(&part.candidates))
                .sum::<u64>()
            + crate::kernel::state::vec_bytes(rows)
            + crate::kernel::state::vec_bytes(estimates)
            + crate::kernel::state::vec_bytes(cell_indices)
            + crate::kernel::state::vec_bytes(owners)
    }

    fn reserve_rows(&mut self, count: usize) -> bool {
        #[cfg(test)]
        if self.fail_reserve == 1 {
            return false;
        }
        self.rows.try_reserve(count).is_ok()
    }

    fn reserve_estimates(&mut self, count: usize) -> bool {
        #[cfg(test)]
        if self.fail_reserve == 2 {
            return false;
        }
        self.estimates.try_reserve(count).is_ok()
            && self.cell_indices.try_reserve(count).is_ok()
            && self.owners.try_reserve(count).is_ok()
    }
}

pub(super) fn demanded(
    step: &mut StepWorkspace<'_>,
    horizon_ms: u64,
    wanted: &[ConflictPassageAddress],
    execution: Option<&ExecutionResources>,
) -> Result<(), StepError> {
    let Some(resources) = execution.filter(|resources| resources.coordinator_parallel()) else {
        return select(step, horizon_ms, wanted, |step, input| {
            serial(step, input, horizon_ms, Some(wanted))
        });
    };
    let mut scratch = resources.frontier_replay().expect("private pool scratch");
    let scratch = &mut *scratch;
    scratch.rows.clear();
    scratch.estimates.clear();
    scratch.cell_indices.clear();
    scratch.owners.clear();
    // 只读并行收集各地址成员的身份、live 序与缓存命中；不可用时原地串行选择。
    let Some(gathered) = gather(
        step,
        horizon_ms,
        wanted,
        resources,
        &mut scratch.select_parts,
    ) else {
        return select(step, horizon_ms, wanted, |step, input| {
            serial(step, input, horizon_ms, Some(wanted))
        });
    };
    // 地址成员数是去重前的上界。容量准备在改动 seen 之前，失败直接融合求值。
    let upper = gathered.member_upper.min(step.committed.live_order.len());
    let materialize_rows = upper >= PARALLEL_ROWS && scratch.reserve_rows(upper);
    let current_parts = &scratch.select_parts[..gathered.used_parts];
    if !materialize_rows {
        return apply_selection(step, current_parts, |step, input| {
            serial(step, input, horizon_ms, Some(wanted))
        });
    }
    let rows = &mut scratch.rows;
    let selected = apply_selection(step, current_parts, |step, input| {
        let cells = step.workspace.frontier_maintenance.slots[input.vehicle.index() as usize]
            .cells
            .len();
        rows.push(Row { input, cells });
        Ok(())
    });
    // 较晚的选择失败不能抢在较早车辆的计算/插入错误之前。
    let total = scratch
        .rows
        .iter()
        .try_fold(0_usize, |sum, row| sum.checked_add(row.cells));
    let prepared = scratch.rows.len() >= PARALLEL_ROWS
        && total.is_some_and(|total| scratch.reserve_estimates(total));
    if !prepared {
        for row in &scratch.rows {
            serial(step, row.input, horizon_ms, Some(wanted))?;
        }
        return selected;
    }
    scratch.estimates.resize(
        total.expect("checked replay size"),
        ApproachEstimate::OutsideHorizon,
    );
    // 未写到的项（越过当前位置、提前越出时窗）不得留下上一拍的结果。
    scratch.estimates.fill(ApproachEstimate::OutsideHorizon);
    scratch
        .cell_indices
        .resize(total.expect("checked replay size"), u32::MAX);
    scratch.owners.resize(
        total.expect("checked replay size"),
        (VehicleHandle::new(0, 0), 0),
    );
    let computed = compute(
        step.read_view(),
        &step.workspace.frontier_maintenance,
        horizon_ms,
        wanted,
        resources,
        scratch,
    );
    consume(step, scratch, computed, resources)?;
    selected
}

fn select(
    step: &mut StepWorkspace<'_>,
    horizon_ms: u64,
    wanted: &[ConflictPassageAddress],
    mut emit: impl FnMut(&mut StepWorkspace<'_>, Input) -> Result<(), StepError>,
) -> Result<(), StepError> {
    // 选择与回调都不读地址成员表；暂时移出，使每个地址只查一次有序表。
    let by_cell = std::mem::take(&mut step.workspace.frontier_maintenance.by_cell);
    let result = wanted.iter().try_for_each(|address| {
        let Some(indexes) = by_cell.get(address) else {
            return Ok(());
        };
        indexes
            .iter()
            .try_for_each(|index| select_member(step, horizon_ms, *index, &mut emit))
    });
    step.workspace.frontier_maintenance.by_cell = by_cell;
    result
}

/// 按需求地址分段并行收集成员，返回本轮有效分段数与去重前的成员总数。
/// 尾段保留容量但不属于本轮选择。序号表无法铺好、
/// 暂存扩容失败时返回 `None`，调用方改走串行选择以保留原首错。
fn gather(
    step: &mut StepWorkspace<'_>,
    horizon_ms: u64,
    wanted: &[ConflictPassageAddress],
    resources: &ExecutionResources,
    parts: &mut Vec<SelectPart>,
) -> Option<GatheredSelection> {
    let slots = step.committed.vehicles.len();
    if !step
        .derived
        .prepare_live_rank(&step.committed.live_order, slots)
    {
        return None;
    }
    let count = resources
        .dispatch_threads()
        .saturating_mul(4)
        .min(MAX_PARTS)
        .min(wanted.len())
        .max(1);
    let span = wanted.len().div_ceil(count).max(1);
    if parts.len() < count {
        parts.try_reserve(count - parts.len()).ok()?;
        parts.resize_with(count, SelectPart::default);
    }
    let read = step.read_view();
    let maintenance = &step.workspace.frontier_maintenance;
    resources.for_each_part(&mut parts[..count], 1, |part, out| {
        let out = &mut out[0];
        out.candidates.clear();
        out.members = 0;
        out.failed = false;
        let start = (part * span).min(wanted.len());
        let end = (start + span).min(wanted.len());
        for address in &wanted[start..end] {
            let Some(indexes) = maintenance.by_cell.get(address) else {
                continue;
            };
            out.members += indexes.len();
            for index in indexes.iter().copied() {
                if maintenance.is_marked(index) {
                    continue;
                }
                let Some(candidate) = gather_member(read, maintenance, horizon_ms, index) else {
                    out.failed = true;
                    return;
                };
                if out.candidates.try_reserve(1).is_err() {
                    out.failed = true;
                    return;
                }
                out.candidates.push((index, candidate));
            }
        }
    });
    let parts = &parts[..count];
    if parts.iter().any(|part| part.failed) {
        return None;
    }
    let member_upper = parts
        .iter()
        .try_fold(0_usize, |sum, part| sum.checked_add(part.members))?;
    Some(GatheredSelection {
        used_parts: count,
        member_upper,
    })
}

/// 只读求一个成员的选择结果；序号表未铺好时返回 `None`。
fn gather_member(
    read: StepReadView<'_>,
    maintenance: &FrontierMaintenance,
    horizon_ms: u64,
    index: u32,
) -> Option<Candidate> {
    let Some(vehicle) = usize::try_from(index)
        .ok()
        .and_then(|slot| maintenance.slots.get(slot))
        .filter(|slot| slot.valid)
        .map(|slot| crate::VehicleHandle::new(index, slot.generation))
    else {
        return Some(Candidate::Rejected);
    };
    let Some(state) = read.committed.vehicles.active_binding(vehicle) else {
        return Some(Candidate::Rejected);
    };
    let Some(sequence) = read
        .derived
        .live_order_index
        .prepared_rank(&read.committed.live_order, vehicle)?
    else {
        return Some(Candidate::Rejected);
    };
    Some(match maintenance.replay_hit(state, horizon_ms) {
        Some(stored_progress) => Candidate::Replay(Input::new(state, sequence, stored_progress)),
        None => Candidate::Deferred(vehicle),
    })
}

/// 按收集顺序（即串行选择的地址、成员顺序）去重并发出；与串行选择逐项相同。
fn apply_selection(
    step: &mut StepWorkspace<'_>,
    parts: &[SelectPart],
    mut emit: impl FnMut(&mut StepWorkspace<'_>, Input) -> Result<(), StepError>,
) -> Result<(), StepError> {
    for (index, candidate) in parts.iter().flat_map(|part| &part.candidates) {
        if step.workspace.frontier_maintenance.is_marked(*index) {
            continue;
        }
        match *candidate {
            Candidate::Rejected => {}
            Candidate::Replay(input) => {
                step.workspace.frontier_maintenance.mark(*index)?;
                emit(step, input)?;
            }
            Candidate::Deferred(vehicle) => {
                step.workspace.frontier_maintenance.mark(*index)?;
                let deferred = &mut step.workspace.frontier_maintenance.scratch_increments;
                deferred
                    .try_reserve(1)
                    .map_err(|_| StepError::ConflictScratchAllocFailed)?;
                deferred.push(vehicle);
            }
        }
    }
    Ok(())
}

fn select_member(
    step: &mut StepWorkspace<'_>,
    horizon_ms: u64,
    index: u32,
    emit: &mut impl FnMut(&mut StepWorkspace<'_>, Input) -> Result<(), StepError>,
) -> Result<(), StepError> {
    if step.workspace.frontier_maintenance.is_marked(index) {
        return Ok(());
    }
    let Some(vehicle) = vehicle_from_slot(step, index)? else {
        return Ok(());
    };
    let Some((state, sequence)) = accepted_source(step, vehicle)? else {
        return Ok(());
    };
    let stored_progress = step
        .workspace
        .frontier_maintenance
        .replay_hit(&state, horizon_ms);
    if !step.workspace.frontier_maintenance.mark(vehicle.index())? {
        return Ok(());
    }
    if let Some(stored_progress) = stored_progress {
        emit(step, Input::new(&state, sequence, stored_progress))
    } else {
        let deferred = &mut step.workspace.frontier_maintenance.scratch_increments;
        deferred
            .try_reserve(1)
            .map_err(|_| StepError::ConflictScratchAllocFailed)?;
        deferred.push(vehicle);
        Ok(())
    }
}

fn compute(
    read: StepReadView<'_>,
    maintenance: &FrontierMaintenance,
    horizon_ms: u64,
    wanted: &[ConflictPassageAddress],
    resources: &ExecutionResources,
    scratch: &mut ReplayScratch,
) -> ComputedReplay {
    let span = scratch
        .rows
        .len()
        .div_ceil(
            resources
                .dispatch_threads()
                .saturating_mul(4)
                .min(MAX_PARTS),
        )
        .max(1);
    struct Part<'a> {
        rows: &'a [Row],
        estimates: &'a mut [ApproachEstimate],
        cell_indices: &'a mut [u32],
        owners: &'a mut [(VehicleHandle, u32)],
        emitted: usize,
        valid: bool,
        first_error: Option<(usize, StepError)>,
        #[cfg(test)]
        work: crate::kernel::conflict::ConflictWorkCounts,
    }
    let mut parts: [Option<Part<'_>>; MAX_PARTS] = std::array::from_fn(|_| None);
    let mut rest = scratch.estimates.as_mut_slice();
    let mut rest_indices = scratch.cell_indices.as_mut_slice();
    let mut rest_owners = scratch.owners.as_mut_slice();
    let count = scratch.rows.len().div_ceil(span);
    for (part, rows) in parts.iter_mut().zip(scratch.rows.chunks(span)) {
        let cells = rows.iter().map(|row| row.cells).sum();
        let (output, tail) = rest.split_at_mut(cells);
        let (indices, tail_indices) = std::mem::take(&mut rest_indices).split_at_mut(cells);
        rest_indices = tail_indices;
        let (owners, tail_owners) = std::mem::take(&mut rest_owners).split_at_mut(cells);
        rest_owners = tail_owners;
        *part = Some(Part {
            rows,
            estimates: output,
            cell_indices: indices,
            owners,
            emitted: 0,
            valid: true,
            first_error: None,
            #[cfg(test)]
            work: Default::default(),
        });
        rest = tail;
    }
    #[cfg(test)]
    let baseline = crate::kernel::conflict::conflict_work_counts();
    resources.for_each_part(&mut parts[..count], 1, |part_index, part| {
        let part = part[0].as_mut().expect("prepared replay part");
        #[cfg(test)]
        let before = crate::kernel::conflict::conflict_work_counts();
        let mut offset = 0;
        for (index, row) in part.rows.iter().enumerate() {
            let output = &mut part.estimates[offset..offset + row.cells];
            let indices = &mut part.cell_indices[offset..offset + row.cells];
            let owners = &mut part.owners[offset..offset + row.cells];
            let owner = (row.input.vehicle, row.input.sequence);
            offset += row.cells;
            let cells = &maintenance.slots[row.input.vehicle.index() as usize].cells;
            let emitted = &mut part.emitted;
            let valid = &mut part.valid;
            let result = PreparedReplay::new(read, row.input, horizon_ms).and_then(|prepared| {
                prepared.walk(
                    row.input,
                    cells,
                    Some(wanted),
                    |index, address, estimate| {
                        output[index] = estimate;
                        owners[index] = owner;
                        *emitted += 1;
                        match read
                            .conflict_read()
                            .cell_index(address)
                            .ok()
                            .and_then(|cell| u32::try_from(cell).ok())
                            .filter(|cell| *cell != u32::MAX)
                        {
                            Some(cell) => indices[index] = cell,
                            None => *valid = false,
                        }
                        Ok(())
                    },
                )
            });
            if let Err(error) = result {
                *valid = false;
                part.first_error
                    .get_or_insert((part_index * span + index, error));
            }
        }
        #[cfg(test)]
        {
            part.work = crate::kernel::conflict::conflict_work_counts().wrapping_sub(before);
        }
    });
    #[cfg(test)]
    crate::kernel::conflict::set_conflict_work_counts(
        parts[..count]
            .iter()
            .flatten()
            .fold(baseline, |sum, part| sum.wrapping_add(part.work)),
    );
    ComputedReplay {
        emitted: parts[..count]
            .iter()
            .flatten()
            .try_fold(0_usize, |sum, part| {
                part.valid.then_some(sum + part.emitted)
            }),
        // 分段顺序与选择行序相同；只归约错误位置，不抢先跳过更早的插入检查。
        first_error: parts[..count]
            .iter()
            .flatten()
            .find_map(|part| part.first_error),
    }
}

/// 按序列出发出项的 cell 下标（跳过未发出的 `u32::MAX`）。
struct EmittedCells<'a> {
    indices: &'a [u32],
    remaining: usize,
}

impl Iterator for EmittedCells<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        while let Some((first, rest)) = self.indices.split_first() {
            self.indices = rest;
            if *first != u32::MAX {
                self.remaining -= 1;
                return Some(*first as usize);
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for EmittedCells<'_> {}

/// 规范插入。`emitted` 为发出项总数，仅在各行无错且全部发出项都有 cell 下标
/// 时给出；此时按 cell 分段并行插入（归约与顺序无关），否则逐项串行插入，
/// 保留原首错位置。
fn consume(
    step: &mut StepWorkspace<'_>,
    scratch: &ReplayScratch,
    computed: ComputedReplay,
    resources: &ExecutionResources,
) -> Result<(), StepError> {
    let Some(emitted) = computed.emitted.filter(|emitted| *emitted > 0) else {
        return consume_serial(step, scratch, computed.first_error);
    };
    #[cfg(test)]
    {
        let mut offset = 0;
        for row in &scratch.rows {
            let vehicle = row.input.vehicle;
            let cells = &step.workspace.frontier_maintenance.slots[vehicle.index() as usize].cells;
            for (cell, estimate) in cells
                .iter()
                .zip(&scratch.estimates[offset..offset + row.cells])
            {
                if *estimate != ApproachEstimate::OutsideHorizon {
                    step.workspace.frontier_maintenance.insertions.push((
                        cell.address,
                        vehicle,
                        *estimate,
                    ));
                }
            }
            offset += row.cells;
        }
        crate::kernel::conflict::count_conflict_work(|counts| counts.frontier_updates += emitted);
    }
    #[cfg(not(test))]
    let _ = emitted;
    step.committed
        .prepare_conflict(&mut step.derived, &mut step.workspace.conflict)
        .insert_approach_owners_partitioned(
            resources,
            EmittedCells {
                indices: &scratch.cell_indices,
                remaining: emitted,
            },
            |range, insert| {
                // 未发出项的下标为 u32::MAX，不落在任何段内。
                for (position, cell) in scratch.cell_indices.iter().enumerate() {
                    let cell = *cell as usize;
                    if range.contains(&cell) {
                        let (vehicle, sequence) = scratch.owners[position];
                        insert(cell, vehicle, sequence, scratch.estimates[position]);
                    }
                }
            },
        )
        .map_err(|error| match error {
            crate::kernel::conflict::ConflictAcquireError::ScratchAllocFailed => {
                StepError::ConflictScratchAllocFailed
            }
            _ => StepError::ConflictInvariantViolation,
        })
}

fn consume_serial(
    step: &mut StepWorkspace<'_>,
    scratch: &ReplayScratch,
    first_error: Option<(usize, StepError)>,
) -> Result<(), StepError> {
    let mut conflict = step
        .committed
        .prepare_conflict(&mut step.derived, &mut step.workspace.conflict);
    let mut offset = 0;
    for (index, row) in scratch.rows.iter().enumerate() {
        if let Some((at, error)) = first_error
            && at == index
        {
            return Err(error);
        }
        let vehicle = row.input.vehicle;
        let cells = &step.workspace.frontier_maintenance.slots[vehicle.index() as usize].cells;
        for (cell, estimate) in cells
            .iter()
            .zip(&scratch.estimates[offset..offset + row.cells])
        {
            if *estimate == ApproachEstimate::OutsideHorizon {
                continue;
            }
            #[cfg(test)]
            step.workspace
                .frontier_maintenance
                .insertions
                .push((cell.address, vehicle, *estimate));
            insert_owner(
                &mut conflict,
                cell.address,
                vehicle,
                row.input.sequence,
                *estimate,
            )?;
        }
        offset += row.cells;
    }
    Ok(())
}

pub(super) fn cached(
    step: &mut StepWorkspace<'_>,
    state: VehicleState,
    sequence: u32,
    stored_progress: u32,
    horizon_ms: u64,
    wanted: Option<&[ConflictPassageAddress]>,
) -> Result<(), StepError> {
    serial(
        step,
        Input::new(&state, sequence, stored_progress),
        horizon_ms,
        wanted,
    )
}

fn serial(
    step: &mut StepWorkspace<'_>,
    input: Input,
    horizon_ms: u64,
    wanted: Option<&[ConflictPassageAddress]>,
) -> Result<(), StepError> {
    let prepared = PreparedReplay::new(step.read_view(), input, horizon_ms)?;
    let vehicle = input.vehicle;
    let mut conflict = step
        .committed
        .prepare_conflict(&mut step.derived, &mut step.workspace.conflict);
    let maintenance = &mut step.workspace.frontier_maintenance;
    let cells = &maintenance.slots[vehicle.index() as usize].cells;
    prepared.walk(input, cells, wanted, |_, address, estimate| {
        #[cfg(test)]
        maintenance.insertions.push((address, vehicle, estimate));
        insert_owner(&mut conflict, address, vehicle, input.sequence, estimate)
    })
}

struct PreparedReplay {
    eta: Option<PreparedApproachEta>,
    hold: Option<SignalHold>,
    horizon_ms: u64,
    emergency_decel: f32,
}

impl PreparedReplay {
    fn new(read: StepReadView<'_>, input: Input, horizon_ms: u64) -> Result<Self, StepError> {
        let profile = read
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(input.profile)
            .ok_or(StepError::ConflictInvariantViolation)?;
        Ok(Self {
            eta: PreparedApproachEta::new(
                input.position.carry_um,
                input.speed_mm_s,
                profile.max_accel(),
                horizon_ms,
            ),
            hold: signal_hold(
                read,
                input.vehicle,
                input.route,
                input.profile,
                input.position,
            ),
            horizon_ms,
            emergency_decel: profile.emergency_decel(),
        })
    }

    fn walk(
        self,
        input: Input,
        cells: &[CachedCell],
        wanted: Option<&[ConflictPassageAddress]>,
        mut emit: impl FnMut(usize, ConflictPassageAddress, ApproachEstimate) -> Result<(), StepError>,
    ) -> Result<(), StepError> {
        let traveled = input
            .position
            .progress_mm
            .saturating_sub(input.stored_progress);
        for (index, cell) in cells.iter().enumerate() {
            if traveled > cell.distance_mm {
                continue;
            }
            let remaining = cell.distance_mm - traveled;
            let kinematic = self.eta.map_or(ApproachEstimate::Unprovable, |eta| {
                eta.lower_bound(u64::from(remaining))
            });
            if kinematic == ApproachEstimate::OutsideHorizon {
                break;
            }
            let estimate = raise_signal_bound(
                kinematic,
                self.hold,
                remaining,
                self.horizon_ms,
                input.speed_mm_s,
                self.emergency_decel,
            );
            if estimate == ApproachEstimate::OutsideHorizon {
                continue;
            }
            if wanted.is_some_and(|wanted| !address_wanted(wanted, cell.address)) {
                continue;
            }
            emit(index, cell.address, estimate)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::entry_frontier::NO_DISTANCE_MM;
    use crate::kernel::execution::WorldExecution;
    use crate::{ExecutionConfig, TrafficWorld};
    use std::num::NonZeroU32;

    #[test]
    fn compact_replay_input_matches_full_state_at_position_boundaries() {
        let (mut world, _) = cached_world(4);
        let vehicle = world.state.committed.live_order[0];
        for (cursor, progress, carry, speed) in [
            (0, 0, 0, 0),
            (0, 100, 999, 10_000),
            (1, 0, 0, 0),
            (1, 0, 777, 10_000),
            (1, 1, 777, 10_001),
        ] {
            let mut state = world.vehicle(vehicle).unwrap();
            state.route_edge_index = cursor;
            state.progress_mm = progress;
            state.carry_um = carry;
            state.speed_mm_s = speed;
            world
                .state
                .committed
                .vehicles
                .slot_mut(vehicle.index() as usize)
                .state = Some(state);
            let state = world.vehicle(vehicle).unwrap();
            let row = world
                .state
                .committed
                .vehicles
                .active_binding(vehicle)
                .unwrap();
            let full = Input::new(&state, 9, 17);
            assert_eq!(Input::new(row, 9, 17), full);
            for horizon in [1, 5_000, 50_000] {
                let maintenance = &world.state.workspace.frontier_maintenance;
                assert_eq!(
                    maintenance.replay_hit(row, horizon),
                    maintenance.replay_hit(&state, horizon),
                );
            }
        }
        assert!(size_of::<Input>() < size_of::<(VehicleState, u32, u32)>());
        println!(
            "REPLAY_LAYOUT input={} candidate={} selected_member={} row={} full_state={}",
            size_of::<Input>(),
            size_of::<Candidate>(),
            size_of::<(u32, Candidate)>(),
            size_of::<Row>(),
            size_of::<VehicleState>(),
        );
    }

    #[test]
    fn compact_replay_matches_full_state_signal_and_eta_primitives() {
        use crate::kernel::entry_frontier::PreparedSignalApproach;

        for (green_ms, green_first) in [(Some(4_000), false), (None, false), (Some(200), true)] {
            let (mut world, route) = crate::admin::cutover_migration::tests::signal_frontier_world(
                4_000,
                green_ms,
                green_first,
            );
            let vehicle = world
                .spawn_vehicle(
                    crate::VehicleSpawnInput::new(
                        VehicleProfileOrdinal::from_raw(0),
                        route,
                        1,
                        0,
                        0,
                    )
                    .with_open_entrance(),
                )
                .unwrap();
            let address = world
                .state
                .read_view()
                .compiled_route(route)
                .unwrap()
                .conflicts[0]
                .address();
            let cells: Vec<_> = [0, 1, 2_000, 60_000, 5_000_000]
                .into_iter()
                .map(|distance_mm| CachedCell {
                    address,
                    distance_mm,
                })
                .collect();
            for reserved in [false, true] {
                if reserved {
                    crate::admin::format_admission::tests::install_conflict_reservation(
                        &mut world, route, vehicle,
                    );
                }
                for (cursor, progress, carry, speed) in [
                    (1, 0, 0, 0),
                    (1, 0, 777, 0),
                    (0, 100, 999, 10_000),
                    (1, 100, 0, 10_000),
                ] {
                    let mut state = world.vehicle(vehicle).unwrap();
                    state.route_edge_index = cursor;
                    state.progress_mm = progress;
                    state.carry_um = carry;
                    state.speed_mm_s = speed;
                    world
                        .state
                        .committed
                        .vehicles
                        .slot_mut(vehicle.index() as usize)
                        .state = Some(state);
                    let read = world.state.read_view();
                    let state = world.vehicle(vehicle).unwrap();
                    let row = read.committed.vehicles.active_binding(vehicle).unwrap();
                    let profile = read
                        .binding
                        .revision
                        .traffic()
                        .relations()
                        .vehicle_profile(state.profile)
                        .unwrap();
                    for horizon in [1, 4_000, 5_000, 50_000] {
                        let eta = PreparedApproachEta::new(
                            state.carry_um,
                            state.speed_mm_s,
                            profile.max_accel(),
                            horizon,
                        );
                        let signal =
                            PreparedSignalApproach::new(read, &state, profile.emergency_decel());
                        for wanted in [None, Some([address].as_slice()), Some([].as_slice())] {
                            let mut expected = Vec::new();
                            for (index, cell) in cells.iter().enumerate() {
                                let traveled = state.progress_mm;
                                if traveled > cell.distance_mm {
                                    continue;
                                }
                                let remaining = cell.distance_mm - traveled;
                                let kinematic = eta.map_or(ApproachEstimate::Unprovable, |eta| {
                                    eta.lower_bound(u64::from(remaining))
                                });
                                if kinematic == ApproachEstimate::OutsideHorizon {
                                    break;
                                }
                                let estimate = signal.apply(kinematic, remaining, horizon);
                                if estimate != ApproachEstimate::OutsideHorizon
                                    && wanted.is_none_or(|wanted| wanted.contains(&cell.address))
                                {
                                    expected.push((index, cell.address, estimate));
                                }
                            }
                            let input = Input::new(row, 7, 0);
                            let mut actual = Vec::new();
                            PreparedReplay::new(read, input, horizon)
                                .unwrap()
                                .walk(input, &cells, wanted, |index, address, estimate| {
                                    actual.push((index, address, estimate));
                                    Ok(())
                                })
                                .unwrap();
                            assert_eq!(
                                actual, expected,
                                "signal projection: {green_ms:?}, reserved={reserved}"
                            );
                            if green_ms == Some(4_000)
                                && !reserved
                                && cursor == 1
                                && progress == 0
                                && carry == 0
                                && speed == 0
                                && horizon == 5_000
                                && wanted.is_none()
                            {
                                assert_eq!(actual[0].2, ApproachEstimate::Finite(4_000));
                            }
                        }
                    }
                    let mut invalid = Input::new(row, 7, 0);
                    invalid.profile = VehicleProfileOrdinal::from_raw(u32::MAX);
                    assert!(matches!(
                        PreparedReplay::new(read, invalid, 5_000),
                        Err(StepError::ConflictInvariantViolation)
                    ));
                }
            }
        }
    }

    fn cached_world(workers: u32) -> (TrafficWorld, Vec<ConflictPassageAddress>) {
        cached_world_with_multiple_addresses(workers, false)
    }

    fn cached_world_with_multiple_addresses(
        workers: u32,
        multiple_addresses: bool,
    ) -> (TrafficWorld, Vec<ConflictPassageAddress>) {
        cached_world_with_population(workers, 16, multiple_addresses)
    }

    fn cached_world_with_population(
        workers: u32,
        population: u32,
        multiple_addresses: bool,
    ) -> (TrafficWorld, Vec<ConflictPassageAddress>) {
        let revision = crate::admin::cutover_migration::tests::conflict_scale_revision();
        let mut world =
            crate::admin::cutover_migration::tests::conflict_scale_world_with_route_capacity(
                revision,
                population,
                if multiple_addresses { 2 } else { 1 },
            );
        let live = world.state.committed.live_order.clone();
        let route = world.vehicle(live[0]).unwrap().route;
        let address = world
            .state
            .read_view()
            .compiled_route(route)
            .unwrap()
            .conflicts[0]
            .address();
        let mut wanted = vec![address];
        if multiple_addresses {
            let revision = world.revision();
            let conflict = revision.conflict();
            let stream = conflict
                .conflict_zone(address.zone())
                .unwrap()
                .participant_streams()
                .iter()
                .copied()
                .find(|stream| *stream != address.stream())
                .unwrap();
            let local = conflict
                .participant_stream(stream)
                .unwrap()
                .passages()
                .iter()
                .position(|passage| passage.conflict_zone() == address.zone())
                .unwrap();
            wanted.push(ConflictPassageAddress::new(
                address.zone(),
                stream,
                local as u32,
            ));
            wanted.sort_unstable();
        }
        // 直接铺缓存以隔离选择与消费；两个规范地址登记不同成员。反向登记让
        // 地址成员顺序与 live 序号不同，同一地址还有重复路线出现项。
        world.state.workspace.frontier_maintenance.ensure_identity(
            world.state.binding.world_id,
            world.state.binding.world_generation,
        );
        for vehicle in live.iter().rev().copied() {
            let address = wanted[vehicle.index() as usize % wanted.len()];
            let mut state = world.vehicle(vehicle).unwrap();
            state.speed_mm_s = 10_000;
            let accel = world
                .traffic()
                .relations()
                .vehicle_profile(state.profile)
                .unwrap()
                .max_accel();
            world
                .state
                .workspace
                .frontier_maintenance
                .store(
                    vehicle,
                    &state,
                    NO_DISTANCE_MM,
                    [0, 1_000, 1_000, 5_000_000]
                        .into_iter()
                        .map(|distance_mm| CachedCell {
                            address,
                            distance_mm,
                        })
                        .collect(),
                    NO_DISTANCE_MM,
                    accel,
                )
                .unwrap();
            // 第一项已越过，末项在时窗外；这两项应清空，不可消费上一轮结果。
            state.progress_mm += 100;
            world
                .state
                .committed
                .vehicles
                .slot_mut(vehicle.index() as usize)
                .state = Some(state);
        }
        world.execution = WorldExecution::start_private(
            ExecutionConfig::new(NonZeroU32::new(workers).unwrap()),
            &world.state,
        );
        (world, wanted)
    }

    fn run(world: &mut TrafficWorld, wanted: &[ConflictPassageAddress]) -> Result<(), StepError> {
        crate::kernel::conflict::reset_conflict_work_counts();
        world.execution.run(&mut world.state, |state, resources| {
            let mut step = state.step_workspace();
            step.workspace.frontier_maintenance.begin_seen()?;
            step.workspace.frontier_maintenance.insertions.clear();
            step.workspace
                .frontier_maintenance
                .scratch_increments
                .clear();
            step.committed
                .prepare_conflict(&mut step.derived, &mut step.workspace.conflict)
                .clear_approach_frontier();
            demanded(&mut step, 5_000, wanted, Some(resources))
        })
    }

    fn assert_matches_serial(
        reference: &mut TrafficWorld,
        pooled: &mut TrafficWorld,
        wanted: &[ConflictPassageAddress],
    ) {
        run(reference, wanted).unwrap();
        let expected_work = crate::kernel::conflict::conflict_work_counts();
        run(pooled, wanted).unwrap();
        let expected = &reference.state.workspace.frontier_maintenance;
        let actual = &pooled.state.workspace.frontier_maintenance;
        assert_eq!(actual.insertions, expected.insertions);
        assert_eq!(actual.scratch_increments, expected.scratch_increments);
        for index in 0..pooled.state.committed.vehicles.len() {
            assert_eq!(
                actual.is_marked(index as u32),
                expected.is_marked(index as u32),
                "seen mark for slot {index}"
            );
        }
        assert_eq!(
            pooled.state.conflict_read().approach_frontier_cells(),
            reference.state.conflict_read().approach_frontier_cells()
        );
        assert_eq!(
            crate::kernel::conflict::conflict_work_counts(),
            expected_work
        );
    }

    #[test]
    fn pooled_replay_ignores_retained_parts_when_demand_shrinks() {
        for workers in [4, 16] {
            for fail_reserve in [0, 1, 2] {
                let (mut reference, wanted) = cached_world_with_multiple_addresses(1, true);
                let (mut pooled, _) = cached_world_with_multiple_addresses(workers, true);
                for world in [&mut reference, &mut pooled] {
                    for index in [5, 11] {
                        // 尾段同时保留重放和 deferred 候选，收缩后两者都不得消费。
                        world.state.workspace.frontier_maintenance.slots[index].first_excluded_mm =
                            0;
                    }
                }
                assert_matches_serial(&mut reference, &mut pooled, &wanted);
                let capacities = {
                    let mut scratch = pooled.execution.resources().frontier_replay().unwrap();
                    assert_eq!(scratch.select_parts.len(), 2);
                    let capacities = scratch
                        .select_parts
                        .iter()
                        .map(|part| part.candidates.capacity())
                        .collect::<Vec<_>>();
                    scratch.fail_reserve = fail_reserve;
                    capacities
                };
                for current in [&wanted[..1], &[][..], &wanted[..], &wanted[..1], &[][..]] {
                    assert_matches_serial(&mut reference, &mut pooled, current);
                    let scratch = pooled.execution.resources().frontier_replay().unwrap();
                    assert_eq!(scratch.select_parts.len(), 2);
                    assert_eq!(
                        scratch
                            .select_parts
                            .iter()
                            .map(|part| part.candidates.capacity())
                            .collect::<Vec<_>>(),
                        capacities,
                        "retained candidate capacity"
                    );
                }
            }
        }
    }

    #[test]
    fn pooled_replay_shrink_rechecks_reused_handles_and_changed_routes() {
        for workers in [4, 16] {
            let (mut reference, wanted) = cached_world_with_multiple_addresses(1, true);
            let (mut pooled, _) = cached_world_with_multiple_addresses(workers, true);
            assert_matches_serial(&mut reference, &mut pooled, &wanted);
            for world in [&mut reference, &mut pooled] {
                let old = world.state.committed.live_order[1];
                let state = world.vehicle(old).unwrap();
                world.despawn_vehicle(old).unwrap();
                let replacement = world
                    .spawn_vehicle(crate::VehicleSpawnInput::new(
                        state.profile,
                        state.route,
                        0,
                        20_000,
                        1_000,
                    ))
                    .unwrap();
                assert_eq!(replacement.index(), old.index());
                assert_ne!(replacement.generation(), old.generation());
                let replacement_state = world.vehicle(replacement).unwrap();
                let accel = world.state.workspace.frontier_maintenance.slots[old.index() as usize]
                    .max_accel;
                world
                    .state
                    .workspace
                    .frontier_maintenance
                    .store(
                        replacement,
                        &replacement_state,
                        NO_DISTANCE_MM,
                        vec![CachedCell {
                            address: wanted[1],
                            distance_mm: 1_000,
                        }],
                        NO_DISTANCE_MM,
                        accel,
                    )
                    .unwrap();
                let removed = VehicleHandle::new(3, 0);
                world.despawn_vehicle(removed).unwrap();

                let changed = VehicleHandle::new(5, 0);
                let old_route = world.vehicle(changed).unwrap().route;
                let edges = world.route_edges(old_route).unwrap().to_vec();
                let new_route = world
                    .register_route(crate::RouteRegisterInput::new(edges))
                    .unwrap();
                world.state.committed.routes[old_route.index() as usize].live_vehicles -= 1;
                world.state.committed.routes[new_route.index() as usize].live_vehicles = 1;
                world
                    .state
                    .committed
                    .vehicles
                    .slot_mut(changed.index() as usize)
                    .state
                    .as_mut()
                    .unwrap()
                    .route = new_route;
                world.state.rebuild_occupancy_index().unwrap();
            }
            for current in [&wanted[..1], &[][..], &wanted[..]] {
                assert_matches_serial(&mut reference, &mut pooled, current);
            }
            assert!(
                pooled
                    .state
                    .workspace
                    .frontier_maintenance
                    .scratch_increments
                    .contains(&VehicleHandle::new(5, 0)),
                "changed route must defer using the current state"
            );
        }
    }

    #[test]
    fn pooled_replay_shrink_preserves_first_error_and_retry() {
        for workers in [4, 16] {
            for fail_reserve in [0, 1, 2] {
                let (mut reference, wanted) = cached_world_with_multiple_addresses(1, true);
                let (mut pooled, _) = cached_world_with_multiple_addresses(workers, true);
                assert_matches_serial(&mut reference, &mut pooled, &wanted);
                pooled
                    .execution
                    .resources()
                    .frontier_replay()
                    .unwrap()
                    .fail_reserve = fail_reserve;
                let profile = reference.vehicle(VehicleHandle::new(8, 0)).unwrap().profile;
                for world in [&mut reference, &mut pooled] {
                    world
                        .state
                        .committed
                        .vehicles
                        .slot_mut(8)
                        .state
                        .as_mut()
                        .unwrap()
                        .profile =
                        laneflow_static_contract::VehicleProfileOrdinal::from_raw(u32::MAX);
                    assert_eq!(
                        run(world, &wanted[..1]),
                        Err(StepError::ConflictInvariantViolation)
                    );
                }
                assert_eq!(
                    pooled.state.workspace.frontier_maintenance.insertions,
                    reference.state.workspace.frontier_maintenance.insertions
                );
                assert_eq!(
                    pooled.state.workspace.frontier_maintenance.insertions.len(),
                    6,
                    "three earlier sources, two occurrences each"
                );
                for world in [&mut reference, &mut pooled] {
                    world
                        .state
                        .committed
                        .vehicles
                        .slot_mut(8)
                        .state
                        .as_mut()
                        .unwrap()
                        .profile = profile;
                }
                assert_matches_serial(&mut reference, &mut pooled, &wanted[..1]);
            }
        }
    }

    #[test]
    fn pooled_replay_shrink_after_cutover_does_not_read_previous_world_inputs() {
        for workers in [4, 16] {
            let (mut reference, wanted) = cached_world_with_multiple_addresses(1, true);
            let (mut pooled, _) = cached_world_with_multiple_addresses(workers, true);
            assert_matches_serial(&mut reference, &mut pooled, &wanted);
            let threads = pooled.execution.thread_ids();
            for world in [&mut reference, &mut pooled] {
                // 缓存夹具向前推进 100 mm；切换前把车头放回当前边内并重建占用。
                for vehicle in world.state.committed.live_order.clone() {
                    world
                        .state
                        .committed
                        .vehicles
                        .slot_mut(vehicle.index() as usize)
                        .state
                        .as_mut()
                        .unwrap()
                        .progress_mm -= 200;
                }
                world.state.rebuild_occupancy_index().unwrap();
                let revision = world.revision();
                let binding =
                    crate::LfcaOriginBinding::from_canonical_origin(*revision.canonical_origin());
                let descriptor = crate::NetworkRevisionCutoverDescriptor::new(
                    binding,
                    binding,
                    None,
                    crate::MigrationPolicyKind::SameRevisionRestore,
                    world.world_binding(),
                );
                let _events = world
                    .cutover_same_revision(
                        revision,
                        world.committed_source().clone(),
                        &descriptor,
                        &crate::CutoverPreflightLimits::new(1_048_576),
                    )
                    .unwrap();
                // 与 Frontier 重建入口一致：新世界代次使缓存冷启动，池缓冲仍保留。
                world.state.workspace.frontier_maintenance.ensure_identity(
                    world.state.binding.world_id,
                    world.state.binding.world_generation,
                );
                assert!(world.state.workspace.frontier_maintenance.slots.is_empty());
            }
            assert_eq!(pooled.execution.thread_ids(), threads);
            assert_eq!(
                pooled
                    .execution
                    .resources()
                    .frontier_replay()
                    .unwrap()
                    .select_parts
                    .len(),
                2
            );
            assert_matches_serial(&mut reference, &mut pooled, &wanted[..1]);
            assert_matches_serial(&mut reference, &mut pooled, &[]);

            let captured = pooled.capture_snapshot().unwrap();
            let bytes = crate::encode_lfrs(&captured);
            let restore = |workers| {
                crate::restore_lfrs(
                    &bytes,
                    pooled.revision(),
                    pooled.committed_source().clone(),
                    pooled.config(),
                    ExecutionConfig::new(NonZeroU32::new(workers).unwrap()),
                    crate::SnapshotRestoreLimits::new(1_048_576, 1_024),
                )
                .unwrap()
                .into_world()
            };
            let mut restored_reference = restore(1);
            let mut restored = restore(workers);
            assert!(
                restored
                    .execution
                    .resources()
                    .frontier_replay()
                    .unwrap()
                    .select_parts
                    .is_empty()
            );
            assert_matches_serial(&mut restored_reference, &mut restored, &wanted[..1]);
        }
    }

    #[test]
    fn pooled_replay_keeps_order_and_clears_reused_outputs() {
        let (mut reference, wanted) = cached_world(1);
        run(&mut reference, &wanted).unwrap();
        let expected = reference
            .state
            .workspace
            .frontier_maintenance
            .insertions
            .clone();
        let expected_work = crate::kernel::conflict::conflict_work_counts();
        let expected_frontier = reference.state.conflict_read().approach_frontier_cells();
        assert!(
            expected_frontier
                .iter()
                .any(|cell| *cell != Default::default())
        );
        assert_eq!(expected_work.eta_preparations, 16);
        assert_eq!(expected_work.eta_distance_evaluations, 48);
        assert_eq!(expected.len(), 32, "two surviving occurrences per vehicle");
        for workers in [4, 16] {
            let (mut pooled, wanted) = cached_world(workers);
            run(&mut pooled, &wanted).unwrap();
            assert_eq!(
                crate::kernel::conflict::conflict_work_counts(),
                expected_work
            );
            assert_eq!(
                pooled.state.workspace.frontier_maintenance.insertions,
                expected
            );
            assert_eq!(
                pooled.state.conflict_read().approach_frontier_cells(),
                expected_frontier
            );
            {
                let mut scratch = pooled.execution.resources().frontier_replay().unwrap();
                assert_eq!(
                    scratch.rows.len(),
                    16,
                    "fixture must dispatch cached sources"
                );
                assert_eq!(scratch.estimates.len(), 64);
                let bytes = scratch.retained_bytes();
                assert_eq!(
                    bytes,
                    (scratch.select_parts.capacity() * size_of::<SelectPart>()
                        + scratch
                            .select_parts
                            .iter()
                            .map(|part| part.candidates.capacity() * size_of::<(u32, Candidate)>())
                            .sum::<usize>()
                        + scratch.rows.capacity() * size_of::<Row>()
                        + scratch.estimates.capacity() * size_of::<ApproachEstimate>()
                        + scratch.cell_indices.capacity() * size_of::<u32>()
                        + scratch.owners.capacity() * size_of::<(VehicleHandle, u32)>())
                        as u64
                );
                assert!(bytes > 0);
                scratch.estimates.fill(ApproachEstimate::Finite(0));
            }
            run(&mut pooled, &wanted).unwrap();
            assert_eq!(
                pooled.state.workspace.frontier_maintenance.insertions,
                expected
            );
            assert_eq!(
                pooled.state.conflict_read().approach_frontier_cells(),
                expected_frontier
            );
        }
    }

    #[test]
    fn pooled_selection_matches_serial_with_marks_rejections_and_deferrals() {
        // 拍初已标记、非 Active、缓存需重走三类成员混在地址成员表里；并行收集
        // 后的去重与发出须与串行选择逐项一致。
        let prepare = |world: &mut TrafficWorld| {
            let live = world.state.committed.live_order.clone();
            let completed = live[3];
            world
                .state
                .committed
                .vehicles
                .slot_mut(completed.index() as usize)
                .state
                .as_mut()
                .unwrap()
                .status = crate::VehicleStatus::Completed;
            for vehicle in [live[5], live[11]] {
                let state = world.vehicle(vehicle).unwrap();
                let maintenance = &mut world.state.workspace.frontier_maintenance;
                let cells = maintenance.slots[vehicle.index() as usize].cells.clone();
                let accel = maintenance.slots[vehicle.index() as usize].max_accel;
                // 首个排除项就在眼前：缓存不能复用，转入增量重走。
                maintenance
                    .store(vehicle, &state, 0, cells, NO_DISTANCE_MM, accel)
                    .unwrap();
            }
            live[7]
        };
        let run_marked = |world: &mut TrafficWorld,
                          wanted: &[ConflictPassageAddress],
                          marked: crate::VehicleHandle| {
            world.execution.run(&mut world.state, |state, resources| {
                let mut step = state.step_workspace();
                let maintenance = &mut step.workspace.frontier_maintenance;
                maintenance.begin_seen()?;
                maintenance.insertions.clear();
                maintenance.scratch_increments.clear();
                maintenance.mark(marked.index())?;
                step.committed
                    .prepare_conflict(&mut step.derived, &mut step.workspace.conflict)
                    .clear_approach_frontier();
                demanded(&mut step, 5_000, wanted, Some(resources))
            })
        };
        let mut expected = None;
        for workers in [1, 4, 16] {
            let (mut world, wanted) = cached_world(workers);
            let marked = prepare(&mut world);
            run_marked(&mut world, &wanted, marked).unwrap();
            let maintenance = &world.state.workspace.frontier_maintenance;
            let observed = (
                maintenance.insertions.clone(),
                maintenance.scratch_increments.clone(),
                world.state.conflict_read().approach_frontier_cells(),
            );
            assert_eq!(observed.1.len(), 2, "two deferred sources");
            assert!(observed.0.iter().all(|(_, vehicle, _)| *vehicle != marked));
            match &expected {
                Some(expected) => assert_eq!(&observed, expected, "{workers} workers"),
                None => expected = Some(observed),
            }
        }
    }

    #[test]
    fn optional_replay_allocation_failure_uses_serial_primitive() {
        let (mut reference, wanted) = cached_world(1);
        run(&mut reference, &wanted).unwrap();
        for fail_reserve in [1, 2] {
            let (mut pooled, wanted) = cached_world(4);
            pooled
                .execution
                .resources()
                .frontier_replay()
                .unwrap()
                .fail_reserve = fail_reserve;
            run(&mut pooled, &wanted).unwrap();
            assert_eq!(
                pooled.state.workspace.frontier_maintenance.insertions,
                reference.state.workspace.frontier_maintenance.insertions
            );
        }
    }

    #[test]
    fn replay_error_consumes_only_earlier_rows_after_full_join() {
        let mut expected = None;
        for workers in [1, 4, 16] {
            let (mut world, wanted) = cached_world(workers);
            let bad = world.state.committed.live_order[8];
            world
                .state
                .committed
                .vehicles
                .slot_mut(bad.index() as usize)
                .state
                .as_mut()
                .unwrap()
                .profile = laneflow_static_contract::VehicleProfileOrdinal::from_raw(u32::MAX);
            assert_eq!(
                run(&mut world, &wanted),
                Err(StepError::ConflictInvariantViolation)
            );
            let insertions = &world.state.workspace.frontier_maintenance.insertions;
            assert_eq!(
                insertions.len(),
                14,
                "seven earlier sources, two occurrences each"
            );
            if let Some(expected) = &expected {
                assert_eq!(insertions, expected);
            } else {
                expected = Some(insertions.clone());
            }
            if let Some(scratch) = world.execution.resources().frontier_replay() {
                assert!(
                    scratch.estimates[32..]
                        .iter()
                        .any(|estimate| matches!(estimate, ApproachEstimate::Finite(_))),
                    "later work must be joined even though consumption stopped at the earlier error"
                );
            }
        }
    }

    #[test]
    fn replay_multiple_errors_keep_selection_order_and_retry_clears_reports() {
        let mut expected = None;
        for workers in [1, 4, 16] {
            let (mut world, wanted) = cached_world_with_population(workers, 32, false);
            let originals: Vec<_> = [2, 3, 17]
                .into_iter()
                .map(|row| {
                    let vehicle = world.state.committed.live_order[31 - row];
                    (vehicle, world.vehicle(vehicle).unwrap().profile)
                })
                .collect();
            for (vehicle, _) in &originals {
                world
                    .state
                    .committed
                    .vehicles
                    .slot_mut(vehicle.index() as usize)
                    .state
                    .as_mut()
                    .unwrap()
                    .profile = VehicleProfileOrdinal::from_raw(u32::MAX);
            }
            assert_eq!(
                run(&mut world, &wanted),
                Err(StepError::ConflictInvariantViolation)
            );
            let inserted = world
                .state
                .workspace
                .frontier_maintenance
                .insertions
                .clone();
            assert_eq!(
                inserted.len(),
                4,
                "two sources before the first bad profile"
            );
            match &expected {
                Some(expected) => assert_eq!(&inserted, expected, "{workers} workers"),
                None => expected = Some(inserted),
            }
            if let Some(scratch) = world.execution.resources().frontier_replay() {
                assert!(
                    scratch.estimates[72..]
                        .iter()
                        .any(|estimate| matches!(estimate, ApproachEstimate::Finite(_))),
                    "sources after all three errors must finish before consumption returns"
                );
            }
            for (vehicle, profile) in originals {
                world
                    .state
                    .committed
                    .vehicles
                    .slot_mut(vehicle.index() as usize)
                    .state
                    .as_mut()
                    .unwrap()
                    .profile = profile;
            }
            let (mut fresh, _) = cached_world_with_population(1, 32, false);
            assert_matches_serial(&mut fresh, &mut world, &wanted);
        }
    }

    #[test]
    fn replay_earlier_insert_failure_precedes_later_computed_error() {
        let mut expected = None;
        for workers in [1, 4, 16] {
            let (mut world, mut wanted) = cached_world(workers);
            let invalid =
                ConflictPassageAddress::new(wanted[0].zone(), wanted[0].stream(), u32::MAX);
            wanted.push(invalid);
            wanted.sort_unstable();
            let earlier = world.state.committed.live_order[14];
            world.state.workspace.frontier_maintenance.slots[earlier.index() as usize].cells[1]
                .address = invalid;
            let later = world.state.committed.live_order[8];
            world
                .state
                .committed
                .vehicles
                .slot_mut(later.index() as usize)
                .state
                .as_mut()
                .unwrap()
                .profile = VehicleProfileOrdinal::from_raw(u32::MAX);
            assert_eq!(
                run(&mut world, &wanted),
                Err(StepError::ConflictInvariantViolation)
            );
            let inserted = &world.state.workspace.frontier_maintenance.insertions;
            assert_eq!(
                inserted.len(),
                3,
                "one complete source and the failed insertion"
            );
            assert_eq!(inserted.last().unwrap().0, invalid);
            match &expected {
                Some(expected) => assert_eq!(inserted, expected, "{workers} workers"),
                None => expected = Some(inserted.clone()),
            }
        }
    }
}
