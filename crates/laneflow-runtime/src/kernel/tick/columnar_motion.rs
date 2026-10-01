//! 物理列批量求值；完整 join 后按规范逻辑与检查位置消费错误和真实资源预留。

use super::{
    BoundedDistance, MOTION_DISPATCH_MIN_ACTIVE, MotionBasis, MotionTaskView,
    ParkingArrivalObservation, ParkingBinding, StepError, VehicleState, VehicleStatus,
    distance_to_occurrence_start, finite_meters, hard_room_mm, leader_gap_m,
    motion_dispatch_fuse_forced, push_parking_arrival, remaining_to_route_end, si_meters,
    stop_is_nearer_or_equal,
};
#[cfg(test)]
use super::{
    LAST_MOTION_DISPATCH_STATS, MOTION_CALCULATIONS, MOTION_DIAGNOSTICS, MOTION_SLOT_GAP,
    MotionWorkChunkRecord, aggregate_motion_tls, count_motion_path, motion_dispatch_forced,
    motion_injection, motion_tls_snapshot, note_columnar_work, note_motion_cache_use,
};
use crate::kernel::motion_updates::{MotionCheckpoint, MotionRowReport, MotionUpdates};
use crate::kernel::vehicle_store::BLOCK_ROWS;

pub(super) struct Chunk<'a> {
    cursor: &'a mut [u32],
    progress: &'a mut [u32],
    speed: &'a mut [u32],
    carry: &'a mut [u16],
    reports: &'a mut [MotionRowReport],
}

fn chunks<'a>(
    updates: &'a mut MotionUpdates,
    current: &'a crate::kernel::vehicle_store::VehicleStore,
    extent: usize,
    rows: usize,
) -> impl Iterator<Item = (usize, Chunk<'a>)> + Send {
    updates
        .motion
        .iter_mut()
        .zip(updates.reports.chunks_mut(BLOCK_ROWS))
        .enumerate()
        .take(extent.div_ceil(BLOCK_ROWS))
        .filter(|(block, _)| current.motion[*block].valid.iter().any(|&bits| bits != 0))
        .flat_map(move |(block, (motion, reports))| {
            let len = extent.saturating_sub(block * BLOCK_ROWS).min(BLOCK_ROWS);
            motion.route_cursor[..len]
                .chunks_mut(rows)
                .zip(motion.progress_mm[..len].chunks_mut(rows))
                .zip(motion.speed_mm_s[..len].chunks_mut(rows))
                .zip(motion.carry_um[..len].chunks_mut(rows))
                .zip(reports[..len].chunks_mut(rows))
                .enumerate()
                .take(
                    extent
                        .saturating_sub(block * BLOCK_ROWS)
                        .min(BLOCK_ROWS)
                        .div_ceil(rows),
                )
                .map(
                    move |(chunk, ((((cursor, progress), speed), carry), reports))| {
                        (
                            block * BLOCK_ROWS + chunk * rows,
                            Chunk {
                                cursor,
                                progress,
                                speed,
                                carry,
                                reports,
                            },
                        )
                    },
                )
        })
}

// 只保存本数值批次真正消费的列；没有 VehicleState/PreparedVehicleMotion 输入表。
macro_rules! batch_columns {
    ($($name:ident: $ty:ty = $value:expr),+ $(,)?) => {
        struct Batch { $($name: [$ty; BLOCK_ROWS]),+ }
        impl Batch { fn new() -> Self { Self { $($name: [$value; BLOCK_ROWS]),+ } } }
    };
}
batch_columns! {
    enabled: bool = false, complex: bool = false,
    desired: u32 = 0, min_gap: f32 = 0.0,
    headway: f32 = 0.0, accel: f32 = 1.0, comfort: f32 = 1.0, emergency: f32 = 1.0,
    leader: f32 = f32::INFINITY, has_leader: bool = false,
    stop: f32 = f32::INFINITY,
    route_end: f32 = f32::INFINITY,
    envelope: f32 = 0.0, hard_room: u32 = 0, limit: u32 = 0,
    edge_length: u32 = 0,
    waiting_hop: Option<u32> = None,
    conflict_hop: Option<u32> = None,
    has_proposal: bool = false, proposal_speed: f32 = 0.0, proposal_travel: f32 = 0.0,
    out_proposal_speed: f32 = 0.0, out_proposal_travel: f32 = 0.0,
    travel_mm: u32 = 0, travel_m: f32 = 0.0, exhausted: bool = false, valid: bool = false,
    window: f32 = 0.0, drop_first: usize = 0, drop_index: usize = 0, drop_active: bool = false,
    drop_distance: u32 = 0, drop_limit: u32 = 0,
    walk_active: bool = false, walk_length: u32 = 0, can_hop: bool = false, has_next: bool = false,
}

impl Batch {
    fn store(&mut self, row: usize, basis: MotionBasis) {
        self.enabled[row] = true;
        self.desired[row] = basis.desired_mm_s;
        self.min_gap[row] = si_meters(basis.min_gap_mm);
        self.headway[row] = basis.time_headway;
        self.accel[row] = basis.max_accel;
        self.comfort[row] = basis.comfort_decel;
        self.emergency[row] = basis.emergency_decel;
        self.has_leader[row] = basis.leader_gap.is_some();
        self.leader[row] = leader_gap_m(basis.leader_gap).unwrap_or(f32::INFINITY);
        self.route_end[row] = finite_meters(basis.route_end).unwrap_or(f32::INFINITY);
        self.envelope[row] = basis.envelope_m;
        self.edge_length[row] = basis.edge_length_mm;
        self.drop_first[row] = basis.speed_drop_first;
        self.limit[row] = basis.current_limit_mm_s;
        if let Some((travel, speed)) = basis.proposal {
            self.has_proposal[row] = true;
            self.proposal_travel[row] = travel;
            self.proposal_speed[row] = speed;
        }
    }
}

fn movement_stop(
    mut stop: Option<BoundedDistance>,
    waiting: Option<crate::kernel::waiting::WaitingStopConstraint>,
    conflict: Option<crate::kernel::waiting::WaitingStopConstraint>,
) -> Option<BoundedDistance> {
    for next in [waiting, conflict].into_iter().flatten() {
        if stop.is_none_or(|current| stop_is_nearer_or_equal(next.distance, current)) {
            stop = Some(next.distance);
        }
    }
    stop
}

fn arrival(
    view: MotionTaskView<'_>,
    old: VehicleState,
    next: VehicleState,
) -> Result<bool, StepError> {
    let Some(ParkingBinding::Reserved(reservation)) =
        view.read.committed.parking.binding(old.handle)
    else {
        return Ok(false);
    };
    if next.status != VehicleStatus::Active {
        return Err(StepError::ParkingInvariantViolation);
    }
    Ok(!view.read.parking_arrived_for(old, reservation)
        && view.read.parking_arrived_for(next, reservation))
}

fn write_value(chunk: &mut Chunk<'_>, row: usize, next: VehicleState) {
    chunk.cursor[row] = next.route_edge_index;
    chunk.progress[row] = next.progress_mm;
    chunk.speed[row] = next.speed_mm_s;
    chunk.carry[row] = next.carry_um;
    chunk.reports[row].completed = next.status == VehicleStatus::Completed;
}

#[derive(Clone, Copy)]
enum NumericPhase {
    Fused,
    Proposal,
    Project,
    Quantize,
}

fn numerical(
    batch: &mut Batch,
    chunk: &mut Chunk<'_>,
    source: &crate::kernel::vehicle_store::MotionBlock,
    offset: usize,
    kernel: laneflow_motion_kernel::Kernel,
    delta_s: f32,
    phase: NumericPhase,
) -> laneflow_motion_kernel::Stats {
    let n = chunk.cursor.len();
    let (proposal_speed, proposal_travel, out_speed, out_travel) = match phase {
        NumericPhase::Fused | NumericPhase::Proposal => (
            &batch.proposal_speed[..n],
            &batch.proposal_travel[..n],
            &mut batch.out_proposal_speed[..n],
            &mut batch.out_proposal_travel[..n],
        ),
        NumericPhase::Project | NumericPhase::Quantize => (
            &batch.out_proposal_speed[..n],
            &batch.out_proposal_travel[..n],
            &mut batch.proposal_speed[..n],
            &mut batch.proposal_travel[..n],
        ),
    };
    let input = laneflow_motion_kernel::Input {
        enabled: &batch.enabled[..n],
        speed_mm_s: &source.speed_mm_s[offset..offset + n],
        desired_mm_s: &batch.desired[..n],
        progress_mm: &source.progress_mm[offset..offset + n],
        carry_um: &source.carry_um[offset..offset + n],
        hard_room_mm: &batch.hard_room[..n],
        committed_limit_mm_s: &[u32::MAX; BLOCK_ROWS][..n],
        leader_m: &batch.leader[..n],
        has_leader: &batch.has_leader[..n],
        min_gap_m: &batch.min_gap[..n],
        time_headway: &batch.headway[..n],
        max_accel: &batch.accel[..n],
        comfort_decel: &batch.comfort[..n],
        emergency_decel: &batch.emergency[..n],
        stop_m: &batch.stop[..n],
        route_end_m: &batch.route_end[..n],
        envelope_m: &batch.envelope[..n],
        has_proposal: &batch.has_proposal[..n],
        proposal_speed_m_s: proposal_speed,
        proposal_travel_m: proposal_travel,
    };
    let mut output = laneflow_motion_kernel::Output {
        speed_mm_s: chunk.speed,
        progress_mm: chunk.progress,
        carry_um: chunk.carry,
        travel_mm: &mut batch.travel_mm[..n],
        travel_m: &mut batch.travel_m[..n],
        proposal_speed_m_s: out_speed,
        proposal_travel_m: out_travel,
        exhausted: &mut batch.exhausted[..n],
        valid: &mut batch.valid[..n],
        window_m: matches!(phase, NumericPhase::Proposal).then_some(&mut batch.window[..n]),
    };
    let stats = match phase {
        NumericPhase::Fused => kernel.run(&input, &mut output, delta_s),
        NumericPhase::Proposal => kernel.proposal(&input, &mut output, delta_s),
        NumericPhase::Project => kernel.float_projection(&input, &mut output, delta_s),
        NumericPhase::Quantize => kernel.quantize(&input, &mut output, delta_s),
    }
    .expect("physical motion columns have identical ranges");
    #[cfg(test)]
    {
        if matches!(phase, NumericPhase::Fused | NumericPhase::Proposal) {
            note_columnar_work(4, stats.proposal_lanes_computed);
            note_columnar_work(5, stats.proposal_lanes_reused);
        }
        if matches!(phase, NumericPhase::Fused | NumericPhase::Project) {
            note_columnar_work(6, stats.active_lanes);
        }
        note_columnar_work(
            7,
            input.enabled[..stats.vector_lanes]
                .iter()
                .filter(|&&v| v)
                .count(),
        );
        note_columnar_work(8, stats.vector_lanes);
        note_columnar_work(9, stats.integer_vector_lanes);
        note_columnar_work(10, stats.scalar_tail_lanes);
        note_columnar_work(13, 1);
        note_columnar_work(15, stats.active_lanes);
    }
    stats
}

#[allow(clippy::too_many_arguments)]
fn apply_speed_limits(
    view: MotionTaskView<'_>,
    batch: &mut Batch,
    chunk: &mut Chunk<'_>,
    source: &crate::kernel::vehicle_store::MotionBlock,
    offset: usize,
    start: usize,
    kernel: laneflow_motion_kernel::Kernel,
    delta_s: f32,
    boundary: bool,
) {
    let n = chunk.cursor.len();
    for row in 0..n {
        batch.drop_index[row] = batch.drop_first[row];
        batch.drop_active[row] = batch.enabled[row] && batch.valid[row] && batch.complex[row];
    }
    while batch.drop_active[..n].iter().any(|&active| active) {
        for row in 0..n {
            if !batch.drop_active[row] {
                continue;
            }
            let state = view
                .read
                .committed
                .vehicles
                .active_at(start + row)
                .expect("frozen speed-limit route context");
            let gathered = (|| {
                let compiled = view.read.compiled_route(state.route)?;
                let Some(drop) = compiled.speed_limit_drop.get(batch.drop_index[row]) else {
                    return Some(None);
                };
                batch.drop_index[row] += 1;
                #[cfg(test)]
                note_columnar_work(11, 1);
                let distance = distance_to_occurrence_start(
                    &compiled.occurrence_segments,
                    &compiled.occurrence_offsets,
                    &compiled.segment_totals,
                    state.route_edge_index as usize,
                    state.progress_mm,
                    (drop.from_route_edge_index as usize).checked_add(1)?,
                )?;
                let BoundedDistance::Finite(distance) = distance else {
                    return Some(None);
                };
                Some(Some((distance, drop.target_mm_s)))
            })();
            match gathered {
                Some(Some((distance, limit))) => {
                    batch.drop_distance[row] = distance;
                    batch.drop_limit[row] = limit;
                }
                Some(None) => batch.drop_active[row] = false,
                None => {
                    batch.drop_active[row] = false;
                    batch.enabled[row] = false;
                    chunk.reports[row].error = Some(StepError::NonFiniteMotion);
                }
            }
        }
        if boundary {
            kernel
                .boundary(
                    &laneflow_motion_kernel::BoundaryInput {
                        speed_mm_s: &source.speed_mm_s[offset..offset + n],
                        distance_mm: &batch.drop_distance[..n],
                        limit_mm_s: &batch.drop_limit[..n],
                        next_speed_m_s: &batch.proposal_speed[..n],
                    },
                    &mut laneflow_motion_kernel::BoundaryOutput {
                        travel_m: &mut batch.travel_m[..n],
                        active: &mut batch.drop_active[..n],
                    },
                    delta_s,
                )
                .expect("speed boundary columns have identical ranges");
        } else {
            kernel
                .limit(
                    &laneflow_motion_kernel::LimitInput {
                        speed_mm_s: &source.speed_mm_s[offset..offset + n],
                        distance_mm: &batch.drop_distance[..n],
                        limit_mm_s: &batch.drop_limit[..n],
                        window_m: &batch.window[..n],
                        comfort_decel: &batch.comfort[..n],
                        emergency_decel: &batch.emergency[..n],
                    },
                    &mut laneflow_motion_kernel::LimitOutput {
                        next_speed_m_s: &mut batch.out_proposal_speed[..n],
                        active: &mut batch.drop_active[..n],
                    },
                    delta_s,
                )
                .expect("speed projection columns have identical ranges");
        }
    }
}

fn compute(
    view: MotionTaskView<'_>,
    rank: &[u32],
    kernel: laneflow_motion_kernel::Kernel,
    start: usize,
    mut chunk: Chunk<'_>,
    delta_s: f32,
) {
    let mut batch = Batch::new();
    let n = chunk.cursor.len();
    #[cfg(test)]
    note_columnar_work(14, n);
    for row in 0..n {
        chunk.reports[row] = MotionRowReport::default();
        let Some(state) = view.read.committed.vehicles.active_at(start + row) else {
            continue;
        };
        let Some(active_index) = rank[state.handle.index() as usize]
            .checked_sub(1)
            .map(|index| index as usize)
        else {
            continue;
        };
        chunk.reports[row].done = true;
        chunk.reports[row].canonical_rank = rank[state.handle.index() as usize];
        let prepared = (|| {
            #[cfg(test)]
            if motion_injection::nonfinite_injected(view.read.binding.world_id, active_index) {
                return Err(StepError::NonFiniteMotion);
            }
            chunk.reports[row].checkpoint = MotionCheckpoint::Parking;
            let parking_binding = view.read.committed.parking.binding(state.handle);
            if !view
                .read
                .parking_state_valid_with_binding(state.handle, state, parking_binding)
                || matches!(parking_binding, Some(ParkingBinding::Occupied(_)))
            {
                return Err(StepError::ParkingInvariantViolation);
            }
            chunk.reports[row].checkpoint = MotionCheckpoint::Waiting;
            let waiting = view.waiting_stop_for(&state)?;
            chunk.reports[row].checkpoint = MotionCheckpoint::Conflict;
            let conflict = view.conflict_stop_for(&state, delta_s)?;
            chunk.reports[row].checkpoint = MotionCheckpoint::Calculation;
            let cached = view
                .motion_cache
                .get(active_index)
                .filter(|entry| entry.vehicle == state.handle);
            if let Some(next) = cached
                .and_then(|entry| entry.preview)
                .and_then(|preview| preview.reuse(waiting, conflict))
            {
                let next = next.apply(state);
                #[cfg(test)]
                note_motion_cache_use(true);
                #[cfg(test)]
                note_columnar_work(3, 1);
                chunk.reports[row].checkpoint = MotionCheckpoint::Arrival;
                chunk.reports[row].arrival = arrival(view, state, next)?;
                write_value(&mut chunk, row, next);
                chunk.reports[row].checkpoint = MotionCheckpoint::Complete;
                return Ok(());
            }
            let reused_basis = cached
                .and_then(|entry| entry.preview)
                .and_then(|preview| preview.basis)
                .filter(|basis| basis.matches(state, delta_s, parking_binding));
            #[cfg(test)]
            note_columnar_work(2, usize::from(reused_basis.is_some()));
            let basis = reused_basis
                .or_else(|| {
                    view.read.prepare_motion_basis(
                        state,
                        delta_s,
                        parking_binding,
                        cached.and_then(|entry| entry.horizon),
                        None,
                    )
                })
                .ok_or(StepError::NonFiniteMotion)?;
            let compiled = view
                .read
                .compiled_route(state.route)
                .ok_or(StepError::NonFiniteMotion)?;
            batch.complex[row] = compiled
                .speed_limit_drop
                .last()
                .is_some_and(|drop| drop.from_route_edge_index >= state.route_edge_index);
            #[cfg(test)]
            MOTION_CALCULATIONS.set(MOTION_CALCULATIONS.get() + 1);
            batch.store(row, basis);
            batch.waiting_hop[row] = waiting.map(|stop| stop.hop);
            batch.conflict_hop[row] = conflict.map(|stop| stop.hop);
            let stop = movement_stop(basis.movement_stop, waiting, conflict);
            batch.stop[row] = stop.and_then(finite_meters).unwrap_or(f32::INFINITY);
            batch.hard_room[row] = hard_room_mm(
                basis.leader_gap,
                basis.min_gap_mm,
                stop,
                basis.route_end,
                basis.edge_length_mm,
                state.progress_mm,
                basis.permitted_for_hard_room,
            );
            chunk.cursor[row] = state.route_edge_index;
            Ok(())
        })();
        if let Err(error) = prepared {
            batch.enabled[row] = false;
            chunk.reports[row].error = Some(error);
        }
    }
    if !batch.enabled[..n].iter().any(|&enabled| enabled) {
        return;
    }
    let source = &view.read.committed.vehicles.motion[start / BLOCK_ROWS];
    let offset = start % BLOCK_ROWS;
    if batch.complex[..n].iter().any(|&complex| complex) {
        numerical(
            &mut batch,
            &mut chunk,
            source,
            offset,
            kernel,
            delta_s,
            NumericPhase::Proposal,
        );
        for row in 0..n {
            if batch.enabled[row] && !batch.valid[row] {
                chunk.reports[row].error = Some(StepError::NonFiniteMotion);
                batch.enabled[row] = false;
            }
        }
        apply_speed_limits(
            view, &mut batch, &mut chunk, source, offset, start, kernel, delta_s, false,
        );
        numerical(
            &mut batch,
            &mut chunk,
            source,
            offset,
            kernel,
            delta_s,
            NumericPhase::Project,
        );
        apply_speed_limits(
            view, &mut batch, &mut chunk, source, offset, start, kernel, delta_s, true,
        );
        numerical(
            &mut batch,
            &mut chunk,
            source,
            offset,
            kernel,
            delta_s,
            NumericPhase::Quantize,
        );
    } else {
        numerical(
            &mut batch,
            &mut chunk,
            source,
            offset,
            kernel,
            delta_s,
            NumericPhase::Fused,
        );
    }
    // 工作掩码驱动真实多跳行走；静态 occurrence/过门读取仅发生在仍行走的行。
    // 非跨边的 next.progress 已由数值内核写好，不再重放或回拷整个世界。
    for row in 0..n {
        let left = batch.edge_length[row].saturating_sub(source.progress_mm[offset + row]);
        batch.walk_active[row] = batch.enabled[row]
            && batch.valid[row]
            && batch.travel_mm[row] >= left
            && (batch.travel_mm[row] > 0 || chunk.carry[row] > 0);
        if batch.walk_active[row] {
            chunk.progress[row] = source.progress_mm[offset + row];
        }
    }
    while batch.walk_active[..n].iter().any(|&active| active) {
        for row in 0..n {
            if !batch.walk_active[row] {
                continue;
            }
            let state = view
                .read
                .committed
                .vehicles
                .active_at(start + row)
                .expect("frozen physical route context");
            let gathered = (|| {
                let compiled = view.read.compiled_route(state.route)?;
                let index = chunk.cursor[row] as usize;
                let edge = *compiled.edges.get(index)?;
                let length = *view
                    .read
                    .binding
                    .revision
                    .traffic()
                    .lane_lengths_millimetres()
                    .get(edge.index())?;
                let boundary = batch.travel_mm[row] >= length.saturating_sub(chunk.progress[row]);
                batch.can_hop[row] = !boundary
                    || (view.read.hop_permitted(
                        state.route,
                        &compiled.edges,
                        index,
                        state.profile,
                    ) && batch.waiting_hop[row].is_none_or(|hop| hop as usize != index)
                        && batch.conflict_hop[row].is_none_or(|hop| hop as usize != index));
                batch.has_next[row] = index + 1 < compiled.edges.len();
                batch.walk_length[row] = length;
                Some(())
            })();
            if gathered.is_none() {
                batch.walk_active[row] = false;
                chunk.reports[row].error = Some(StepError::NonFiniteMotion);
            }
        }
        let edge = laneflow_motion_kernel::EdgeInput {
            edge_length_mm: &batch.walk_length[..n],
            can_hop: &batch.can_hop[..n],
            has_next: &batch.has_next[..n],
            carry_um: chunk.carry,
        };
        let mut next = laneflow_motion_kernel::EdgeOutput {
            route_cursor: chunk.cursor,
            progress_mm: chunk.progress,
            remaining_mm: &mut batch.travel_mm[..n],
            active: &mut batch.walk_active[..n],
            valid: &mut batch.valid[..n],
        };
        let stats = kernel
            .advance(&edge, &mut next)
            .expect("physical route-walk columns have identical ranges");
        #[cfg(test)]
        note_columnar_work(12, stats.active_lanes);
        #[cfg(not(test))]
        let _ = stats;
    }
    for row in 0..n {
        if !batch.enabled[row] || chunk.reports[row].error.is_some() {
            continue;
        }
        let state = view
            .read
            .committed
            .vehicles
            .active_at(start + row)
            .expect("frozen active physical row");
        let completed = (|| {
            let parking_binding = view.read.committed.parking.binding(state.handle);
            if !batch.valid[row] {
                return Err(StepError::NonFiniteMotion);
            }
            let mut next = state;
            next.speed_mm_s = chunk.speed[row];
            next.carry_um = chunk.carry[row];
            let compiled = view
                .read
                .compiled_route(state.route)
                .ok_or(StepError::NonFiniteMotion)?;
            next.route_edge_index = chunk.cursor[row];
            next.progress_mm = chunk.progress[row];
            let limit = if next.route_edge_index == state.route_edge_index {
                batch.limit[row]
            } else {
                let edge = compiled
                    .edges
                    .get(next.route_edge_index as usize)
                    .ok_or(StepError::NonFiniteMotion)?;
                *view
                    .read
                    .binding
                    .revision
                    .traffic()
                    .lane_speed_limits_millimetres_per_second()
                    .get(edge.index())
                    .ok_or(StepError::NonFiniteMotion)?
            };
            next.speed_mm_s = next.speed_mm_s.min(limit);
            let remaining = remaining_to_route_end(
                *compiled
                    .remaining_to_end
                    .get(next.route_edge_index as usize)
                    .ok_or(StepError::NonFiniteMotion)?,
                next.progress_mm,
            );
            if batch.exhausted[row] || matches!(remaining, BoundedDistance::Finite(0)) {
                next.speed_mm_s = 0;
                next.carry_um = 0;
                let parked_arrival = match parking_binding {
                    Some(ParkingBinding::Reserved(value)) => {
                        view.read.parking_arrived_for(next, value)
                    }
                    _ => false,
                };
                if matches!(remaining, BoundedDistance::Finite(0)) && !parked_arrival {
                    next.status = VehicleStatus::Completed;
                }
            }
            #[cfg(test)]
            note_motion_cache_use(false);
            chunk.reports[row].checkpoint = MotionCheckpoint::Arrival;
            chunk.reports[row].arrival = arrival(view, state, next)?;
            write_value(&mut chunk, row, next);
            chunk.reports[row].checkpoint = MotionCheckpoint::Complete;
            Ok(())
        })();
        if let Err(error) = completed {
            chunk.reports[row].error = Some(error);
        }
    }
}

pub(super) fn prepare(
    workspace: &mut crate::kernel::state::TickWorkspace,
    read: crate::kernel::phase::StepReadView<'_>,
    execution: Option<&crate::kernel::execution::ExecutionResources>,
    delta_s: f32,
    arrivals: &mut Vec<ParkingArrivalObservation>,
    updates: &mut MotionUpdates,
) -> Result<(), StepError> {
    let workload = read.derived.active_order.len();
    let extent = read.committed.vehicles.active_extent();
    workspace.next_state_by_vehicle.fill(0);
    for (rank, handle) in read.derived.active_order.iter().enumerate() {
        workspace.next_state_by_vehicle[handle.index() as usize] =
            u32::try_from(rank + 1).map_err(|_| StepError::ConflictInvariantViolation)?;
    }
    #[cfg(test)]
    let forced = motion_dispatch_forced();
    #[cfg(not(test))]
    let forced = false;
    let dispatched = execution.is_some_and(|resources| {
        matches!(
            resources,
            crate::kernel::execution::ExecutionResources::Pool(_)
        )
    }) && (workload >= MOTION_DISPATCH_MIN_ACTIVE || forced && workload > 0)
        && !motion_dispatch_fuse_forced();
    #[cfg(test)]
    let fallback = dispatched && motion_injection::slot_reserve_injected(read.binding.world_id);
    #[cfg(not(test))]
    let fallback = false;
    #[cfg(test)]
    count_motion_path(|counts| {
        if fallback {
            counts.slot_fallback += 1;
        } else if dispatched {
            counts.dispatched += 1;
        } else {
            counts.fused += 1;
        }
    });
    let execution = (dispatched && !fallback).then_some(execution).flatten();
    let desired_chunk = extent
        .div_ceil(execution.map_or(1, |resources| {
            resources.dispatch_threads().saturating_mul(2)
        }))
        .max(1);
    // 列块内保持连续，块大小只影响工作粒度；尾部和孔洞由显式 work mask 排除。
    let rows = desired_chunk.next_power_of_two().min(BLOCK_ROWS);
    #[cfg(test)]
    let chunk_count = extent.div_ceil(rows);
    #[cfg(test)]
    let diagnostics = MOTION_DIAGNOSTICS.with(std::cell::Cell::get);
    #[cfg(test)]
    let records = diagnostics.then(|| {
        (0..chunk_count)
            .map(|_| MotionWorkChunkRecord::default())
            .collect::<Vec<_>>()
    });
    #[cfg(test)]
    let baseline = diagnostics.then(motion_tls_snapshot);
    #[cfg(test)]
    let participation = crate::kernel::motion_participation::current();
    let view = MotionTaskView {
        read,
        waiting_plans: &workspace.waiting_plans,
        waiting_plan_by_vehicle: &workspace.waiting_plan_by_vehicle,
        conflict_motion_by_vehicle: &workspace.conflict_motion_by_vehicle,
        conflict_staged: &workspace.conflict,
        motion_cache: &workspace.motion_cache,
    };
    let kernel = workspace.motion_kernel;
    let rank = &workspace.next_state_by_vehicle;
    let work = chunks(updates, &read.committed.vehicles, extent, rows);
    let calculate = |_: crate::kernel::phase::StepReadView<'_>, start, chunk| {
        #[cfg(test)]
        if let Some(probe) = &participation
            && execution.is_some()
        {
            probe.enter(start);
        }
        #[cfg(test)]
        let before = diagnostics.then(motion_tls_snapshot);
        compute(view, rank, kernel, start, chunk, delta_s);
        #[cfg(test)]
        if let (Some(records), Some(before)) = (&records, before) {
            records[start / rows].store_deltas(before);
        }
    };
    if let Some(execution) = execution {
        let stats = execution.for_each_work(read, work, calculate);
        #[cfg(test)]
        {
            crate::kernel::execution::note_last_dispatch_stats(stats);
            LAST_MOTION_DISPATCH_STATS.with(|cell| cell.set(Some(stats)));
        }
        #[cfg(not(test))]
        let _ = stats;
    } else {
        for (start, chunk) in work {
            calculate(read, start, chunk);
        }
    }
    #[cfg(test)]
    {
        if let (Some(baseline), Some(records)) = (baseline, &records) {
            aggregate_motion_tls(baseline, records);
        }
        if let Some(rank) = MOTION_SLOT_GAP.with(std::cell::Cell::get)
            && let Some(handle) = read.derived.active_order.get(rank)
            && let Some(physical) = read.committed.vehicles.active_row(*handle)
        {
            updates.reports[physical].done = false;
        }
    }
    // 规范首错及每辆车的真实到达 reserve 交错保留，物理块和 ISA 都不改变消费顺序。
    for (canonical_rank, handle) in read.derived.active_order.iter().copied().enumerate() {
        let Some(state) = read.vehicle_state(handle) else {
            continue;
        };
        let physical = read
            .committed
            .vehicles
            .active_row(handle)
            .ok_or(StepError::ConflictInvariantViolation)?;
        let report = updates.reports[physical];
        if !report.done || report.canonical_rank as usize != canonical_rank + 1 {
            return Err(StepError::ConflictInvariantViolation);
        }
        if let Some(error) = report.error {
            return Err(error);
        }
        if report.checkpoint != MotionCheckpoint::Complete {
            return Err(StepError::ConflictInvariantViolation);
        }
        if report.arrival {
            let Some(ParkingBinding::Reserved(reservation)) =
                read.committed.parking.binding(handle)
            else {
                return Err(StepError::ParkingInvariantViolation);
            };
            push_parking_arrival(
                arrivals,
                ParkingArrivalObservation {
                    vehicle: handle,
                    target: reservation.target(),
                },
                read.binding.world_id,
            )?;
        }
        updates.adopt(state, report.completed, &read.committed.vehicles)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use laneflow_motion_kernel::{Backend, Kernel};

    #[test]
    #[ignore = "manual single-process work counters; global diagnostic session excludes parallel tests"]
    fn columnar_work_diagnostic_smoke() {
        let mut world = crate::kernel::waiting::tests::multi_gate_world(129);
        super::super::take_columnar_work();
        world.step(crate::TickInput::new(100)).unwrap();
        let work = super::super::take_columnar_work();
        super::super::COLUMNAR_WORK_ENABLED.store(false, std::sync::atomic::Ordering::Relaxed);
        assert!(work[0] > 0);
        assert!(work[14] >= 129);
        assert!(work[6] > 0 || work[3] > 0);
        eprintln!(
            "LF814_WORK {work:?}; batch_bytes={}; basis_bytes={}",
            std::mem::size_of::<Batch>(),
            std::mem::size_of::<MotionBasis>()
        );
    }

    #[test]
    fn physical_columns_match_exact_vehicle_primitive_with_holes_permuted_rows_and_cache_prefixes()
    {
        for count in [1, 7, 8, 9, 15, 16, 17, 127, 128, 129] {
            for cache_prefix in [0, 5, usize::MAX] {
                for backend in [Backend::Scalar, Backend::Avx2, Backend::Avx512] {
                    let Some(kernel) = Kernel::for_backend(backend) else {
                        continue;
                    };
                    let mut world = crate::kernel::waiting::tests::multi_gate_world(count);
                    let handles = world.live_vehicles().to_vec();
                    if count > 2 {
                        world.despawn_vehicle(handles[1]).unwrap();
                        // 通过 storage-class 迁移交换两行，逻辑身份/快照值不变。
                        let a = world.vehicle(handles[0]).unwrap();
                        let b = world.vehicle(*handles.last().unwrap()).unwrap();
                        for state in [a, b] {
                            let mut inactive = state;
                            inactive.status = VehicleStatus::Parked;
                            world
                                .state
                                .committed
                                .vehicles
                                .slot_mut(state.handle.index() as usize)
                                .state = Some(inactive);
                        }
                        for state in [a, b] {
                            world
                                .state
                                .committed
                                .vehicles
                                .slot_mut(state.handle.index() as usize)
                                .state = Some(state);
                        }
                        assert_ne!(
                            world.state.committed.vehicles.active_row(a.handle),
                            Some(a.handle.index() as usize)
                        );
                    }
                    world.state.rebuild_occupancy_index().unwrap();
                    world.state.prepare_waiting_step(0.1).unwrap();
                    world.state.prepare_conflict_step(0.1, 1, None).unwrap();
                    world.state.workspace.motion_cache.truncate(cache_prefix);
                    world.state.workspace.motion_kernel = kernel;
                    let before: Vec<_> = world
                        .live_vehicles()
                        .iter()
                        .map(|handle| world.vehicle(*handle))
                        .collect();
                    let read = crate::kernel::phase::StepReadView {
                        binding: &world.state.binding,
                        committed: &world.state.committed,
                        derived: &world.state.derived,
                    };
                    let view = MotionTaskView {
                        read,
                        waiting_plans: &world.state.workspace.waiting_plans,
                        waiting_plan_by_vehicle: &world.state.workspace.waiting_plan_by_vehicle,
                        conflict_motion_by_vehicle: &world
                            .state
                            .workspace
                            .conflict_motion_by_vehicle,
                        conflict_staged: &world.state.workspace.conflict,
                        motion_cache: &world.state.workspace.motion_cache,
                    };
                    let expected: Vec<_> = read
                        .derived
                        .active_order
                        .iter()
                        .copied()
                        .enumerate()
                        .map(|(rank, handle)| {
                            let state = read.vehicle_state(handle).unwrap();
                            let outcome = view.vehicle_motion_outcome(&state, rank, 0.1).unwrap();
                            (handle.index() as usize, outcome.next, outcome.arrival)
                        })
                        .collect();
                    let mut next =
                        MotionUpdates::with_capacity(world.state.committed.vehicles.capacity());
                    let mut arrivals = Vec::new();
                    prepare(
                        &mut world.state.workspace,
                        read,
                        None,
                        0.1,
                        &mut arrivals,
                        &mut next,
                    )
                    .unwrap();
                    assert_eq!(
                        next.iter(&world.state.committed.vehicles)
                            .collect::<Vec<_>>(),
                        expected
                            .iter()
                            .map(|&(slot, state, _)| (slot, state))
                            .collect::<Vec<_>>(),
                        "backend={backend:?}, count={count}, cache_prefix={cache_prefix}"
                    );
                    assert_eq!(
                        world
                            .live_vehicles()
                            .iter()
                            .map(|handle| world.vehicle(*handle))
                            .collect::<Vec<_>>(),
                        before,
                        "P5 must not publish partially computed columns"
                    );
                    assert_eq!(
                        arrivals,
                        expected
                            .iter()
                            .filter_map(|&(_, _, arrival)| arrival)
                            .collect::<Vec<_>>()
                    );
                    next.validate(&world.state.committed.vehicles).unwrap();
                }
            }
        }
    }
}
