//! Frontier 缓存复用：规范选择、并行求值、完整 join 后原序插入。

use super::{
    CachedCell, FrontierMaintenance, SignalHold, accepted_source, address_wanted, insert_owner,
    raise_signal_bound, signal_hold, vehicle_from_slot,
};
use crate::kernel::conflict::PreparedApproachEta;
use crate::kernel::execution::ExecutionResources;
use crate::kernel::phase::{StepReadView, StepWorkspace};
use crate::{ApproachEstimate, ConflictPassageAddress, StepError, VehicleState};

#[cfg(not(test))]
const PARALLEL_ROWS: usize = 1_024;
#[cfg(test)]
const PARALLEL_ROWS: usize = 1;
const MAX_PARTS: usize = 128;

#[derive(Clone, Copy)]
struct Input {
    state: VehicleState,
    sequence: u32,
    stored_progress: u32,
}

struct Row {
    input: Input,
    cells: usize,
    error: Option<StepError>,
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

/// 只存本次选中的车辆和缓存出现项，不复制逐车缓存，也不按 worker 复制世界。
#[derive(Default)]
pub(crate) struct ReplayScratch {
    select_parts: Vec<SelectPart>,
    rows: Vec<Row>,
    estimates: Vec<ApproachEstimate>,
    /// 与 `estimates` 对齐的 cell 下标；未发出或下标无效时为 `u32::MAX`。
    cell_indices: Vec<u32>,
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
        self.estimates.try_reserve(count).is_ok() && self.cell_indices.try_reserve(count).is_ok()
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
    // 只读并行收集各地址成员的身份、live 序与缓存命中；不可用时原地串行选择。
    let Some(upper) = gather(
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
    let upper = upper.min(step.committed.live_order.len());
    if upper < PARALLEL_ROWS || !scratch.reserve_rows(upper) {
        return apply_selection(step, &scratch.select_parts, |step, input| {
            serial(step, input, horizon_ms, Some(wanted))
        });
    }
    let rows = &mut scratch.rows;
    let selected = apply_selection(step, &scratch.select_parts, |step, input| {
        let cells = step.workspace.frontier_maintenance.slots[input.state.handle.index() as usize]
            .cells
            .len();
        rows.push(Row {
            input,
            cells,
            error: None,
        });
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
    let emitted = compute(
        step.read_view(),
        &step.workspace.frontier_maintenance,
        horizon_ms,
        wanted,
        resources,
        scratch,
    );
    consume(step, scratch, emitted, resources)?;
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

/// 按需求地址分段并行收集成员，返回去重前的成员总数。序号表无法铺好、
/// 暂存扩容失败时返回 `None`，调用方改走串行选择以保留原首错。
fn gather(
    step: &mut StepWorkspace<'_>,
    horizon_ms: u64,
    wanted: &[ConflictPassageAddress],
    resources: &ExecutionResources,
    parts: &mut Vec<SelectPart>,
) -> Option<usize> {
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
    parts
        .iter()
        .try_fold(0_usize, |sum, part| sum.checked_add(part.members))
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
    let Some(state) = read
        .vehicle_state(vehicle)
        .filter(|state| state.status == crate::VehicleStatus::Active)
    else {
        return Some(Candidate::Rejected);
    };
    let Some(sequence) = read
        .derived
        .live_order_index
        .prepared_rank(&read.committed.live_order, vehicle)?
    else {
        return Some(Candidate::Rejected);
    };
    Some(match maintenance.replay_hit(vehicle, &state, horizon_ms) {
        Some(stored_progress) => Candidate::Replay(Input {
            state,
            sequence,
            stored_progress,
        }),
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
        .replay_hit(vehicle, &state, horizon_ms);
    if !step.workspace.frontier_maintenance.mark(vehicle.index())? {
        return Ok(());
    }
    if let Some(stored_progress) = stored_progress {
        emit(
            step,
            Input {
                state,
                sequence,
                stored_progress,
            },
        )
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
) -> Option<usize> {
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
        rows: &'a mut [Row],
        estimates: &'a mut [ApproachEstimate],
        cell_indices: &'a mut [u32],
        emitted: usize,
        valid: bool,
        #[cfg(test)]
        work: crate::kernel::conflict::ConflictWorkCounts,
    }
    let mut parts: [Option<Part<'_>>; MAX_PARTS] = std::array::from_fn(|_| None);
    let mut rest = scratch.estimates.as_mut_slice();
    let mut rest_indices = scratch.cell_indices.as_mut_slice();
    let count = scratch.rows.len().div_ceil(span);
    for (part, rows) in parts.iter_mut().zip(scratch.rows.chunks_mut(span)) {
        let cells = rows.iter().map(|row| row.cells).sum();
        let (output, tail) = rest.split_at_mut(cells);
        let (indices, tail_indices) = std::mem::take(&mut rest_indices).split_at_mut(cells);
        rest_indices = tail_indices;
        *part = Some(Part {
            rows,
            estimates: output,
            cell_indices: indices,
            emitted: 0,
            valid: true,
            #[cfg(test)]
            work: Default::default(),
        });
        rest = tail;
    }
    #[cfg(test)]
    let baseline = crate::kernel::conflict::conflict_work_counts();
    resources.for_each_part(&mut parts[..count], 1, |_, part| {
        let part = part[0].as_mut().expect("prepared replay part");
        #[cfg(test)]
        let before = crate::kernel::conflict::conflict_work_counts();
        let mut offset = 0;
        for row in part.rows.iter_mut() {
            let output = &mut part.estimates[offset..offset + row.cells];
            let indices = &mut part.cell_indices[offset..offset + row.cells];
            offset += row.cells;
            let cells = &maintenance.slots[row.input.state.handle.index() as usize].cells;
            let emitted = &mut part.emitted;
            let valid = &mut part.valid;
            row.error = PreparedReplay::new(read, row.input.state, horizon_ms)
                .and_then(|prepared| {
                    prepared.walk(
                        row.input,
                        cells,
                        Some(wanted),
                        |index, address, estimate| {
                            output[index] = estimate;
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
                })
                .err();
            *valid &= row.error.is_none();
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
    parts[..count]
        .iter()
        .flatten()
        .try_fold(0_usize, |sum, part| {
            part.valid.then_some(sum + part.emitted)
        })
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
    emitted: Option<usize>,
    resources: &ExecutionResources,
) -> Result<(), StepError> {
    let Some(emitted) = emitted.filter(|emitted| *emitted > 0) else {
        return consume_serial(step, scratch);
    };
    #[cfg(test)]
    {
        let mut offset = 0;
        for row in &scratch.rows {
            let vehicle = row.input.state.handle;
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
                let mut offset = 0;
                for row in &scratch.rows {
                    let vehicle = row.input.state.handle;
                    let end = offset + row.cells;
                    // 未发出项的下标为 u32::MAX，不落在任何段内。
                    for (position, cell) in scratch.cell_indices[offset..end].iter().enumerate() {
                        let cell = *cell as usize;
                        if range.contains(&cell) {
                            insert(
                                cell,
                                vehicle,
                                row.input.sequence,
                                scratch.estimates[offset + position],
                            );
                        }
                    }
                    offset = end;
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

fn consume_serial(step: &mut StepWorkspace<'_>, scratch: &ReplayScratch) -> Result<(), StepError> {
    let mut conflict = step
        .committed
        .prepare_conflict(&mut step.derived, &mut step.workspace.conflict);
    let mut offset = 0;
    for row in &scratch.rows {
        if let Some(error) = row.error {
            return Err(error);
        }
        let vehicle = row.input.state.handle;
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
        Input {
            state,
            sequence,
            stored_progress,
        },
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
    let prepared = PreparedReplay::new(step.read_view(), input.state, horizon_ms)?;
    let vehicle = input.state.handle;
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
    fn new(
        read: StepReadView<'_>,
        state: VehicleState,
        horizon_ms: u64,
    ) -> Result<Self, StepError> {
        let profile = read
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
            .ok_or(StepError::ConflictInvariantViolation)?;
        Ok(Self {
            eta: PreparedApproachEta::new(
                state.carry_um,
                state.speed_mm_s,
                profile.max_accel(),
                horizon_ms,
            ),
            hold: signal_hold(read, state.handle, &state),
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
            .state
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
                input.state.speed_mm_s,
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

    fn cached_world(workers: u32) -> (TrafficWorld, Vec<ConflictPassageAddress>) {
        let revision = crate::admin::cutover_migration::tests::conflict_scale_revision();
        let mut world = crate::admin::cutover_migration::tests::conflict_scale_world(revision, 16);
        let live = world.state.committed.live_order.clone();
        let route = world.vehicle(live[0]).unwrap().route;
        let address = world
            .state
            .read_view()
            .compiled_route(route)
            .unwrap()
            .conflicts[0]
            .address();
        // 反向登记让地址成员顺序与 live 序号不同；一个地址有重复路线出现项。
        for vehicle in live.iter().rev().copied() {
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
        (world, vec![address])
    }

    fn run(world: &mut TrafficWorld, wanted: &[ConflictPassageAddress]) -> Result<(), StepError> {
        crate::kernel::conflict::reset_conflict_work_counts();
        world.execution.run(&mut world.state, |state, resources| {
            let mut step = state.step_workspace();
            step.workspace.frontier_maintenance.begin_seen()?;
            step.workspace.frontier_maintenance.insertions.clear();
            step.committed
                .prepare_conflict(&mut step.derived, &mut step.workspace.conflict)
                .clear_approach_frontier();
            demanded(&mut step, 5_000, wanted, Some(resources))
        })
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
                        + scratch.cell_indices.capacity() * size_of::<u32>())
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
                assert_eq!(
                    scratch.rows[7].error,
                    Some(StepError::ConflictInvariantViolation)
                );
                assert!(scratch.rows[8..].iter().all(|row| row.error.is_none()));
                assert!(
                    scratch.estimates[32..]
                        .iter()
                        .any(|estimate| matches!(estimate, ApproachEstimate::Finite(_))),
                    "later work must be joined even though consumption stopped at the earlier error"
                );
            }
        }
    }
}
