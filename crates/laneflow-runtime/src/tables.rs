use laneflow_static_contract::{
    AccessEffect, ConflictZoneOrdinal, LaneEdgeOrdinal, ManeuverGateOrdinal, ManeuverPathOrdinal,
    ParticipantClassOrdinal, ParticipantStreamOrdinal, WaitingZoneOrdinal,
};
use laneflow_static_network::{
    AccessCell, BoundedDistance, ConflictPathAnchor, SharedManeuverNetwork, SharedNetworkRevision,
    SharedTrafficNetwork,
};

#[cfg(test)]
use std::cell::Cell;

use crate::{ConflictRuntimeUnavailable, RouteError, RouteHandle, VehicleState};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ManeuverOccurrence {
    pub path: ManeuverPathOrdinal,
    pub entry_route_edge_index: u32,
    pub exit_route_edge_index: u32,
}

/// 下一盏绑定 `SignalGroup` 的 hop 门。距离从本 hop 的 from 边起点算起。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NextControlled {
    pub hop: u32,
    pub gate: ManeuverGateOrdinal,
    pub distance_from_hop_start: BoundedDistance,
}

/// 限速下降转换：与共享根 `speed_limit_transitions` 同形。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SpeedLimitDrop {
    pub from_route_edge_index: u32,
    pub to_edge: LaneEdgeOrdinal,
    pub target_mm_s: u32,
}

/// 注册时物化的等待区出现项。#282 未消费前也不得静默丢弃。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WaitingOccurrence {
    pub zone: WaitingZoneOrdinal,
    pub maneuver_index: u32,
    pub entry_hop: u32,
    pub release_hop: u32,
    pub storage_length_mm: u32,
}

/// 路线 occurrence 坐标；同一 `LaneEdgeOrdinal` 在循环路线中的不同下标不会折叠。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct RoutePosition {
    pub route_edge_index: u32,
    pub progress_mm: u32,
}

/// `ParticipantStream` owner-local passage 在一条 compiled route 中的一次出现。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ConflictPassageOccurrence {
    pub stream: ParticipantStreamOrdinal,
    pub passage_local_index: u32,
    pub zone: ConflictZoneOrdinal,
    pub maneuver_index: u32,
    pub admission_hop: u32,
    pub entry: RoutePosition,
    pub clearance: RoutePosition,
}

/// `conflicts` 中属于一个 admission hop 的连续半开区间。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ConflictGateRange {
    pub start: u32,
    pub len: u32,
}

/// 本世界 compiled 路线：分段 `u32` 前缀、后缀 `BoundedDistance`、hop 门、
/// 受控 hop 链和限速下降转换。
/// 不上 `u64`，不把 world 身份写进 `RouteHandle`，不存「当前红灯」（ADR 0028 / 0029）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompiledRoute {
    pub edges: Vec<LaneEdgeOrdinal>,
    pub maneuvers: Vec<ManeuverOccurrence>,
    pub hop_gate: Vec<Option<ManeuverGateOrdinal>>,
    /// 按 route order 排列的实际 Gate hops；稀疏路线不逐车扫描空 hop 后缀。
    pub gate_hops: Vec<u32>,
    pub remaining_to_end: Vec<BoundedDistance>,
    pub occurrence_segments: Vec<u32>,
    pub occurrence_offsets: Vec<u32>,
    pub segment_totals: Vec<u32>,
    pub next_controlled: Vec<Option<NextControlled>>,
    pub speed_limit_drop: Vec<SpeedLimitDrop>,
    pub waiting: Vec<WaitingOccurrence>,
    pub conflicts: Vec<ConflictPassageOccurrence>,
    pub conflict_gate_ranges: Vec<ConflictGateRange>,
    pub final_conflict_clearance: Option<(RoutePosition, u32)>,
}

#[cfg(test)]
impl CompiledRoute {
    fn retained_logical_bytes(&self) -> u64 {
        logical_vec_bytes::<LaneEdgeOrdinal>(self.edges.len())
            + logical_vec_bytes::<ManeuverOccurrence>(self.maneuvers.len())
            + logical_vec_bytes::<Option<ManeuverGateOrdinal>>(self.hop_gate.len())
            + logical_vec_bytes::<u32>(self.gate_hops.len())
            + logical_vec_bytes::<BoundedDistance>(self.remaining_to_end.len())
            + logical_vec_bytes::<u32>(self.occurrence_segments.len())
            + logical_vec_bytes::<u32>(self.occurrence_offsets.len())
            + logical_vec_bytes::<u32>(self.segment_totals.len())
            + logical_vec_bytes::<Option<NextControlled>>(self.next_controlled.len())
            + logical_vec_bytes::<SpeedLimitDrop>(self.speed_limit_drop.len())
            + logical_vec_bytes::<WaitingOccurrence>(self.waiting.len())
            + logical_vec_bytes::<ConflictPassageOccurrence>(self.conflicts.len())
            + logical_vec_bytes::<ConflictGateRange>(self.conflict_gate_ranges.len())
    }
}

#[cfg(test)]
fn logical_vec_bytes<T>(len: usize) -> u64 {
    u64::try_from(len)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(std::mem::size_of::<T>()).expect("type size fits u64"))
}

#[derive(Clone, Debug)]
pub(crate) struct RouteSlot {
    pub generation: u32,
    pub compiled: Option<CompiledRoute>,
    pub live_vehicles: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct VehicleSlot {
    pub generation: u32,
    pub state: Option<VehicleState>,
}

#[cfg(test)]
thread_local! {
    static ROUTE_RESERVATIONS_BEFORE_FAILURE: Cell<Option<usize>> = const { Cell::new(None) };
}

#[cfg(test)]
struct RouteAllocationFailpointReset(Option<usize>);

#[cfg(test)]
impl Drop for RouteAllocationFailpointReset {
    fn drop(&mut self) {
        ROUTE_RESERVATIONS_BEFORE_FAILURE.with(|remaining| remaining.set(self.0));
    }
}

/// 只供同线程单元测试确定性覆盖第 N 次路线编译预留失败。
#[cfg(test)]
pub(crate) fn with_route_allocation_failure_after<T>(
    successful_reservations: usize,
    run: impl FnOnce() -> T,
) -> T {
    ROUTE_RESERVATIONS_BEFORE_FAILURE.with(|remaining| {
        let _reset =
            RouteAllocationFailpointReset(remaining.replace(Some(successful_reservations)));
        run()
    })
}

fn try_reserve_route_exact<T>(values: &mut Vec<T>, capacity: usize) -> Result<(), RouteError> {
    if capacity == 0 {
        return Ok(());
    }
    #[cfg(test)]
    {
        let fail = ROUTE_RESERVATIONS_BEFORE_FAILURE.with(|remaining| match remaining.get() {
            Some(0) => true,
            Some(value) => {
                remaining.set(Some(value - 1));
                false
            }
            None => false,
        });
        if fail {
            return Err(RouteError::AllocationFailed);
        }
    }
    values
        .try_reserve_exact(capacity)
        .map_err(|_| RouteError::AllocationFailed)
}

fn try_route_vec<T>(capacity: usize) -> Result<Vec<T>, RouteError> {
    let mut values = Vec::new();
    try_reserve_route_exact(&mut values, capacity)?;
    Ok(values)
}

fn try_route_vec_filled<T: Clone>(len: usize, value: T) -> Result<Vec<T>, RouteError> {
    let mut values = try_route_vec(len)?;
    values.resize(len, value);
    Ok(values)
}

fn route_pair_connected(
    traffic: &SharedTrafficNetwork,
    from: LaneEdgeOrdinal,
    to: LaneEdgeOrdinal,
) -> bool {
    if traffic
        .successors(from)
        .is_some_and(|successors| successors.contains(&to))
    {
        return true;
    }
    traffic
        .maneuvers()
        .transition_candidates(from)
        .is_some_and(|candidates| {
            candidates
                .iter()
                .any(|candidate| candidate.successor() == to)
        })
}

fn claim_internal_coverage(
    coverage: &mut [Option<ManeuverPathOrdinal>],
    entry_index: usize,
    exit_index: usize,
    path: ManeuverPathOrdinal,
) -> Result<(), RouteError> {
    for slot in coverage
        .iter_mut()
        .take(exit_index)
        .skip(entry_index.saturating_add(1))
    {
        if slot.is_some() {
            return Err(RouteError::ManeuverMismatch);
        }
        *slot = Some(path);
    }
    Ok(())
}

fn hop_ranges_overlap(a_entry: u32, a_exit: u32, b_entry: u32, b_exit: u32) -> bool {
    a_entry < b_exit && b_entry < a_exit
}

fn record_occurrence(
    coverage: &mut [Option<ManeuverPathOrdinal>],
    maneuvers: &[ManeuverOccurrence],
    entry_index: usize,
    exit_index: usize,
    path: ManeuverPathOrdinal,
) -> Result<(), RouteError> {
    if coverage.get(entry_index).copied().flatten().is_some() {
        return Err(RouteError::ManeuverMismatch);
    }
    let entry = u32::try_from(entry_index).expect("route edge index fits u32");
    let exit = u32::try_from(exit_index).expect("route edge index fits u32");
    if maneuvers.iter().any(|occ| {
        hop_ranges_overlap(
            occ.entry_route_edge_index,
            occ.exit_route_edge_index,
            entry,
            exit,
        )
    }) {
        return Err(RouteError::ManeuverMismatch);
    }
    claim_internal_coverage(coverage, entry_index, exit_index, path)
}

/// 注册期唯一出现项编译器。物化分段 `u32` 索引、受控 hop 链与限速下降转换；
/// 不上 `u64`，不冻当前红灯。
pub(crate) fn compile_route(
    revision: &SharedNetworkRevision,
    edges: &[LaneEdgeOrdinal],
    live_conflict_occurrence_count: u64,
    route_conflict_occurrence_capacity: u64,
) -> Result<CompiledRoute, RouteError> {
    let traffic = revision.traffic();
    if edges.is_empty() {
        return Err(RouteError::EmptySequence);
    }
    let count = traffic.lane_edge_count();
    for edge in edges {
        if edge.raw() >= count {
            return Err(RouteError::UnknownEdge);
        }
    }
    let first = edges[0];
    let last = *edges.last().expect("non-empty route");
    if traffic.relations().stop_line_for_edge(last).is_some() {
        return Err(RouteError::ManeuverMismatch);
    }
    if traffic.relations().lane_edge_junction(first).is_some()
        || traffic.relations().lane_edge_junction(last).is_some()
    {
        return Err(RouteError::ManeuverMismatch);
    }
    for pair in edges.windows(2) {
        if !route_pair_connected(traffic, pair[0], pair[1]) {
            return Err(RouteError::Disconnected);
        }
    }

    let network = traffic.maneuvers();
    let transition_len = edges.len().saturating_sub(1);
    let mut maneuvers: Vec<ManeuverOccurrence> = try_route_vec(transition_len)?;
    let mut coverage = try_route_vec_filled(edges.len(), None)?;
    for entry_index in 0..transition_len {
        let Some(path_ordinal) = unique_entry_path_match(
            network,
            edges[entry_index],
            edges[entry_index + 1],
            &edges[entry_index..],
        )?
        else {
            continue;
        };
        let path = network
            .maneuver_path(path_ordinal)
            .ok_or(RouteError::ManeuverMismatch)?;
        let exit_index = entry_index
            .checked_add(path.edges().len())
            .and_then(|value| value.checked_sub(1))
            .ok_or(RouteError::ManeuverMismatch)?;
        if exit_index >= edges.len() {
            return Err(RouteError::ManeuverMismatch);
        }
        record_occurrence(
            &mut coverage,
            &maneuvers,
            entry_index,
            exit_index,
            path_ordinal,
        )?;
        maneuvers.push(ManeuverOccurrence {
            path: path_ordinal,
            entry_route_edge_index: u32::try_from(entry_index).expect("route edge index fits u32"),
            exit_route_edge_index: u32::try_from(exit_index).expect("route edge index fits u32"),
        });
    }
    for (index, edge) in edges.iter().enumerate() {
        if traffic.relations().lane_edge_junction(*edge).is_some() && coverage[index].is_none() {
            return Err(RouteError::ManeuverMismatch);
        }
    }

    let lengths = traffic.lane_lengths_millimetres();
    let speeds = traffic.lane_speed_limits_millimetres_per_second();
    let mut remaining_to_end = try_route_vec_filled(edges.len(), BoundedDistance::Finite(0))?;
    let mut suffix = BoundedDistance::Finite(0);
    for index in (0..edges.len()).rev() {
        let edge = edges[index];
        suffix = suffix.add_u32(*lengths.get(edge.index()).unwrap_or(&0));
        remaining_to_end[index] = suffix;
    }
    let mut route_lengths = try_route_vec(edges.len())?;
    route_lengths.extend(
        edges
            .iter()
            .map(|edge| *lengths.get(edge.index()).unwrap_or(&0)),
    );
    let (occurrence_segments, occurrence_offsets, segment_totals) =
        segmented_route_coordinates(&route_lengths)?;

    let hop_count = edges.len().saturating_sub(1);
    let mut hop_gate = try_route_vec_filled(hop_count, None)?;
    for hop in 0..hop_count {
        hop_gate[hop] = hop_gate_at(network, &maneuvers, hop, edges[hop], edges[hop + 1]);
    }
    let mut gate_hops = try_route_vec(hop_gate.iter().filter(|gate| gate.is_some()).count())?;
    for (hop, gate) in hop_gate.iter().enumerate() {
        if gate.is_some() {
            gate_hops.push(u32::try_from(hop).expect("hop fits u32"));
        }
    }

    let conflict_occurrence_count = count_conflict_occurrences(revision, &maneuvers)?;
    checked_conflict_occurrence_total(
        live_conflict_occurrence_count,
        conflict_occurrence_count,
        route_conflict_occurrence_capacity,
    )?;
    let (conflicts, conflict_gate_ranges, final_conflict_clearance) = compile_conflicts(
        revision,
        edges,
        &maneuvers,
        &hop_gate,
        conflict_occurrence_count,
    )?;

    let mut next_controlled = try_route_vec_filled(hop_count, None)?;
    let mut next: Option<NextControlled> = None;
    for hop in (0..hop_count).rev() {
        let length = route_lengths[hop];
        if let Some(gate) = hop_gate[hop].filter(|gate| {
            traffic
                .relations()
                .maneuver_gate(*gate)
                .and_then(|view| view.signal_group())
                .is_some()
        }) {
            next = Some(NextControlled {
                hop: u32::try_from(hop).expect("hop index fits u32"),
                gate,
                distance_from_hop_start: BoundedDistance::Finite(0).add_u32(length),
            });
        } else if let Some(controlled) = next.as_mut() {
            controlled.distance_from_hop_start = controlled.distance_from_hop_start.add_u32(length);
        }
        next_controlled[hop] = next;
    }

    let mut speed_limit_drop = try_route_vec(hop_count)?;
    for (from_index, pair) in edges.windows(2).enumerate() {
        let from_speed = *speeds.get(pair[0].index()).unwrap_or(&0);
        let to_speed = *speeds.get(pair[1].index()).unwrap_or(&0);
        if to_speed < from_speed {
            speed_limit_drop.push(SpeedLimitDrop {
                from_route_edge_index: u32::try_from(from_index).expect("route index fits u32"),
                to_edge: pair[1],
                target_mm_s: to_speed,
            });
        }
    }

    let waiting = compile_waiting(
        traffic,
        &hop_gate,
        &maneuvers,
        &occurrence_segments,
        &occurrence_offsets,
        &segment_totals,
    )?;
    let mut compiled_edges = try_route_vec(edges.len())?;
    compiled_edges.extend_from_slice(edges);

    Ok(CompiledRoute {
        edges: compiled_edges,
        maneuvers,
        hop_gate,
        gate_hops,
        remaining_to_end,
        occurrence_segments,
        occurrence_offsets,
        segment_totals,
        next_controlled,
        speed_limit_drop,
        waiting,
        conflicts,
        conflict_gate_ranges,
        final_conflict_clearance,
    })
}

/// 分段 `u32` 前缀。下一条边长会让当前段溢出时封段、开新段。不上 `u64`（ADR 0028）。
type SegmentedRouteCoordinates = (Vec<u32>, Vec<u32>, Vec<u32>);

fn segmented_route_coordinates(
    edge_lengths: &[u32],
) -> Result<SegmentedRouteCoordinates, RouteError> {
    let mut segments = try_route_vec(edge_lengths.len())?;
    let mut offsets = try_route_vec(edge_lengths.len())?;
    let mut totals = try_route_vec(edge_lengths.len())?;
    let mut current_total = 0_u32;
    let mut current_has_occurrence = false;
    for edge_length in edge_lengths.iter().copied() {
        let must_start_segment =
            current_has_occurrence && current_total.checked_add(edge_length).is_none();
        if must_start_segment {
            totals.push(current_total);
            current_total = 0;
        }
        segments.push(u32::try_from(totals.len()).expect("segment index fits"));
        offsets.push(current_total);
        current_total = current_total
            .checked_add(edge_length)
            .expect("admitted edge length fits a new segment");
        current_has_occurrence = true;
    }
    if current_has_occurrence {
        totals.push(current_total);
    }
    Ok((segments, offsets, totals))
}

fn compile_waiting(
    traffic: &SharedTrafficNetwork,
    hop_gate: &[Option<ManeuverGateOrdinal>],
    maneuvers: &[ManeuverOccurrence],
    occurrence_segments: &[u32],
    occurrence_offsets: &[u32],
    segment_totals: &[u32],
) -> Result<Vec<WaitingOccurrence>, RouteError> {
    let network = traffic.maneuvers();
    let relations = traffic.relations();
    let waiting_capacity = maneuvers.iter().try_fold(0_usize, |total, occurrence| {
        let path = network
            .maneuver_path(occurrence.path)
            .ok_or(RouteError::ManeuverMismatch)?;
        total
            .checked_add(path.waiting_zones().len())
            .ok_or(RouteError::AllocationFailed)
    })?;
    let mut waiting = try_route_vec(waiting_capacity)?;
    for (maneuver_index, occurrence) in maneuvers.iter().enumerate() {
        let path = network
            .maneuver_path(occurrence.path)
            .ok_or(RouteError::ManeuverMismatch)?;
        for zone in path.waiting_zones() {
            let view = relations
                .waiting_zone(*zone)
                .ok_or(RouteError::ManeuverMismatch)?;
            let entry_gate = relations
                .maneuver_gate(view.entry_gate())
                .ok_or(RouteError::ManeuverMismatch)?;
            let release_gate = relations
                .maneuver_gate(view.release_gate())
                .ok_or(RouteError::ManeuverMismatch)?;
            if entry_gate.path() != occurrence.path || release_gate.path() != occurrence.path {
                return Err(RouteError::ManeuverMismatch);
            }
            let entry_hop = occurrence
                .entry_route_edge_index
                .checked_add(entry_gate.transition_index())
                .ok_or(RouteError::ManeuverMismatch)?;
            let release_hop = occurrence
                .entry_route_edge_index
                .checked_add(release_gate.transition_index())
                .ok_or(RouteError::ManeuverMismatch)?;
            let entry_index = usize::try_from(entry_hop).expect("hop fits usize");
            let release_index = usize::try_from(release_hop).expect("hop fits usize");
            if hop_gate.get(entry_index).copied().flatten() != Some(view.entry_gate())
                || hop_gate.get(release_index).copied().flatten() != Some(view.release_gate())
            {
                return Err(RouteError::ManeuverMismatch);
            }
            let storage_start = entry_index
                .checked_add(1)
                .ok_or(RouteError::WaitingStorageSpanUnbounded)?;
            let storage_end = release_index
                .checked_add(1)
                .ok_or(RouteError::WaitingStorageSpanUnbounded)?;
            let storage_length_mm = match distance_to_occurrence_start(
                occurrence_segments,
                occurrence_offsets,
                segment_totals,
                storage_start,
                0,
                storage_end,
            ) {
                Some(BoundedDistance::Finite(value)) => value,
                Some(BoundedDistance::BeyondFinite) | None => {
                    return Err(RouteError::WaitingStorageSpanUnbounded);
                }
            };
            waiting.push(WaitingOccurrence {
                zone: *zone,
                maneuver_index: u32::try_from(maneuver_index).expect("occurrence index fits u32"),
                entry_hop,
                release_hop,
                storage_length_mm,
            });
        }
    }
    Ok(waiting)
}

fn count_conflict_occurrences(
    revision: &SharedNetworkRevision,
    maneuvers: &[ManeuverOccurrence],
) -> Result<u64, RouteError> {
    let conflict = revision.conflict();
    let mut count = 0_u64;
    for occurrence in maneuvers {
        let streams = conflict
            .maneuver_path_participant_streams(occurrence.path)
            .ok_or(RouteError::ManeuverMismatch)?;
        for stream in streams {
            let view = conflict
                .participant_stream(*stream)
                .filter(|view| view.maneuver_path() == occurrence.path)
                .ok_or(RouteError::ManeuverMismatch)?;
            count = count
                .checked_add(
                    u64::try_from(view.passages().len())
                        .map_err(|_| RouteError::AllocationFailed)?,
                )
                .ok_or(RouteError::AllocationFailed)?;
        }
    }
    Ok(count)
}

fn checked_conflict_occurrence_total(
    current: u64,
    added: u64,
    capacity: u64,
) -> Result<u64, RouteError> {
    let total =
        current
            .checked_add(added)
            .ok_or(RouteError::ConflictOccurrenceCapacityExceeded {
                current,
                added,
                capacity,
            })?;
    if total > capacity {
        return Err(RouteError::ConflictOccurrenceCapacityExceeded {
            current,
            added,
            capacity,
        });
    }
    Ok(total)
}

fn map_conflict_anchor(
    revision: &SharedNetworkRevision,
    route_edge_count: usize,
    occurrence: ManeuverOccurrence,
    anchor: ConflictPathAnchor,
) -> Result<RoutePosition, RouteError> {
    let traffic = revision.traffic();
    let path = traffic
        .maneuvers()
        .maneuver_path(occurrence.path)
        .ok_or(RouteError::ManeuverMismatch)?;
    let path_edge_count = path.edges().len();
    let expected_exit = usize::try_from(occurrence.entry_route_edge_index)
        .expect("route edge index fits usize")
        .checked_add(path_edge_count)
        .and_then(|value| value.checked_sub(1))
        .ok_or(RouteError::ManeuverMismatch)?;
    if u32::try_from(expected_exit).ok() != Some(occurrence.exit_route_edge_index) {
        return Err(RouteError::ManeuverMismatch);
    }

    let position = match anchor {
        ConflictPathAnchor::Gate(gate) => {
            let gate = traffic
                .relations()
                .maneuver_gate(gate)
                .filter(|gate| gate.path() == occurrence.path)
                .ok_or(RouteError::ManeuverMismatch)?;
            let route_edge_index = occurrence
                .entry_route_edge_index
                .checked_add(gate.transition_index())
                .and_then(|value| value.checked_add(1))
                .ok_or(RouteError::ManeuverMismatch)?;
            RoutePosition {
                route_edge_index,
                progress_mm: 0,
            }
        }
        ConflictPathAnchor::EdgeBoundary(boundary) => {
            let boundary = usize::try_from(boundary).expect("path boundary fits usize");
            if boundary > path_edge_count {
                return Err(RouteError::ManeuverMismatch);
            }
            if boundary < path_edge_count {
                RoutePosition {
                    route_edge_index: occurrence
                        .entry_route_edge_index
                        .checked_add(u32::try_from(boundary).expect("path boundary fits u32"))
                        .ok_or(RouteError::ManeuverMismatch)?,
                    progress_mm: 0,
                }
            } else if expected_exit + 1 < route_edge_count {
                RoutePosition {
                    route_edge_index: occurrence
                        .exit_route_edge_index
                        .checked_add(1)
                        .ok_or(RouteError::ManeuverMismatch)?,
                    progress_mm: 0,
                }
            } else {
                let last = *path.edges().last().ok_or(RouteError::ManeuverMismatch)?;
                let progress_mm = *traffic
                    .lane_lengths_millimetres()
                    .get(last.index())
                    .ok_or(RouteError::ManeuverMismatch)?;
                RoutePosition {
                    route_edge_index: occurrence.exit_route_edge_index,
                    progress_mm,
                }
            }
        }
        ConflictPathAnchor::Interior {
            path_edge_index,
            progress_millimetres,
        } => {
            let path_edge = *path
                .edges()
                .get(usize::try_from(path_edge_index).expect("path edge index fits usize"))
                .ok_or(RouteError::ManeuverMismatch)?;
            let length = *traffic
                .lane_lengths_millimetres()
                .get(path_edge.index())
                .ok_or(RouteError::ManeuverMismatch)?;
            if progress_millimetres == 0 || progress_millimetres >= length {
                return Err(RouteError::ManeuverMismatch);
            }
            RoutePosition {
                route_edge_index: occurrence
                    .entry_route_edge_index
                    .checked_add(path_edge_index)
                    .ok_or(RouteError::ManeuverMismatch)?,
                progress_mm: progress_millimetres,
            }
        }
    };
    if usize::try_from(position.route_edge_index).expect("route position fits usize")
        >= route_edge_count
    {
        return Err(RouteError::ManeuverMismatch);
    }
    Ok(position)
}

type CompiledConflicts = (
    Vec<ConflictPassageOccurrence>,
    Vec<ConflictGateRange>,
    Option<(RoutePosition, u32)>,
);

fn try_conflict_occurrence_vec(
    capacity: usize,
) -> Result<Vec<ConflictPassageOccurrence>, RouteError> {
    try_route_vec(capacity)
}

fn compile_conflicts(
    revision: &SharedNetworkRevision,
    route_edges: &[LaneEdgeOrdinal],
    maneuvers: &[ManeuverOccurrence],
    hop_gate: &[Option<ManeuverGateOrdinal>],
    expected_count: u64,
) -> Result<CompiledConflicts, RouteError> {
    let capacity = usize::try_from(expected_count).map_err(|_| RouteError::AllocationFailed)?;
    let mut conflicts = try_conflict_occurrence_vec(capacity)?;
    let traffic = revision.traffic();
    let conflict = revision.conflict();
    for (maneuver_index, occurrence) in maneuvers.iter().copied().enumerate() {
        let streams = conflict
            .maneuver_path_participant_streams(occurrence.path)
            .ok_or(RouteError::ManeuverMismatch)?;
        for stream in streams {
            let stream_view = conflict
                .participant_stream(*stream)
                .filter(|view| view.maneuver_path() == occurrence.path)
                .ok_or(RouteError::ManeuverMismatch)?;
            for (passage_local_index, passage) in stream_view.passages().iter().copied().enumerate()
            {
                let admission_gate = traffic
                    .relations()
                    .maneuver_gate(passage.admission_gate())
                    .filter(|gate| gate.path() == occurrence.path)
                    .ok_or(RouteError::ManeuverMismatch)?;
                let admission_hop = occurrence
                    .entry_route_edge_index
                    .checked_add(admission_gate.transition_index())
                    .ok_or(RouteError::ManeuverMismatch)?;
                if hop_gate
                    .get(usize::try_from(admission_hop).expect("admission hop fits usize"))
                    .copied()
                    .flatten()
                    != Some(passage.admission_gate())
                {
                    return Err(RouteError::ManeuverMismatch);
                }
                let entry =
                    map_conflict_anchor(revision, route_edges.len(), occurrence, passage.entry())?;
                let clearance =
                    map_conflict_anchor(revision, route_edges.len(), occurrence, passage.exit())?;
                if entry >= clearance {
                    return Err(RouteError::ManeuverMismatch);
                }
                conflicts.push(ConflictPassageOccurrence {
                    stream: *stream,
                    passage_local_index: u32::try_from(passage_local_index)
                        .map_err(|_| RouteError::AllocationFailed)?,
                    zone: passage.conflict_zone(),
                    maneuver_index: u32::try_from(maneuver_index)
                        .map_err(|_| RouteError::AllocationFailed)?,
                    admission_hop,
                    entry,
                    clearance,
                });
            }
        }
    }
    if conflicts.len() != capacity {
        return Err(RouteError::ManeuverMismatch);
    }

    finalize_conflicts(conflicts, hop_gate.len())
}

fn finalize_conflicts(
    mut conflicts: Vec<ConflictPassageOccurrence>,
    hop_count: usize,
) -> Result<CompiledConflicts, RouteError> {
    conflicts.sort_unstable_by_key(|occurrence| {
        (
            occurrence.admission_hop,
            occurrence.entry,
            occurrence.clearance,
            occurrence.stream.raw(),
            occurrence.passage_local_index,
        )
    });

    let mut conflict_gate_ranges = try_route_vec_filled(hop_count, ConflictGateRange::default())?;
    let mut cursor = 0_usize;
    for (hop, range) in conflict_gate_ranges.iter_mut().enumerate() {
        let start = cursor;
        while conflicts.get(cursor).is_some_and(|occurrence| {
            usize::try_from(occurrence.admission_hop).expect("admission hop fits usize") == hop
        }) {
            cursor += 1;
        }
        range.start = u32::try_from(start).map_err(|_| RouteError::AllocationFailed)?;
        range.len = u32::try_from(cursor - start).map_err(|_| RouteError::AllocationFailed)?;
    }
    if cursor != conflicts.len() {
        return Err(RouteError::ManeuverMismatch);
    }

    let final_conflict_clearance = conflicts
        .iter()
        .enumerate()
        .max_by_key(|(index, occurrence)| (occurrence.clearance, *index))
        .map(|(index, occurrence)| {
            (
                occurrence.clearance,
                u32::try_from(index).expect("format-bounded conflict occurrence index fits u32"),
            )
        });
    Ok((conflicts, conflict_gate_ranges, final_conflict_clearance))
}

/// 在机动路径入口跳上，用剩余边序列唯一匹配完整 `path.edges()` 前缀。
///
/// 只认 `transition_index == 0`；多条不同 path 都匹配则歧义；有入口候选但对不上完整
/// 路径则失败。非入口跳返回 `Ok(None)`，由 occurrence 覆盖表在编译结束时对路口内部边
/// 失败关闭。
pub(crate) fn unique_entry_path_match(
    network: &SharedManeuverNetwork,
    from: LaneEdgeOrdinal,
    to: LaneEdgeOrdinal,
    remaining: &[LaneEdgeOrdinal],
) -> Result<Option<ManeuverPathOrdinal>, RouteError> {
    let Some(candidates) = network.transition_candidates(from) else {
        return Ok(None);
    };
    let mut matched = None;
    let mut saw_entry = false;
    for candidate in candidates {
        if candidate.successor() != to || candidate.transition_index() != 0 {
            continue;
        }
        let path = network
            .maneuver_path(candidate.maneuver_path())
            .ok_or(RouteError::ManeuverMismatch)?;
        saw_entry = true;
        if remaining.starts_with(path.edges()) {
            match matched {
                None => matched = Some(candidate.maneuver_path()),
                Some(first) if first != candidate.maneuver_path() => {
                    return Err(RouteError::AmbiguousManeuver);
                }
                Some(_) => {}
            }
        }
    }
    if !saw_entry {
        return Ok(None);
    }
    matched.ok_or(RouteError::ManeuverMismatch).map(Some)
}

#[cfg(test)]
fn unique_entry_path_match_filtered<'a>(
    remaining: &[LaneEdgeOrdinal],
    entry_paths: impl IntoIterator<Item = (ManeuverPathOrdinal, &'a [LaneEdgeOrdinal])>,
) -> Result<Option<ManeuverPathOrdinal>, RouteError> {
    let mut matched = None;
    let mut saw_entry = false;
    for (path, edges) in entry_paths {
        saw_entry = true;
        if remaining.starts_with(edges) {
            match matched {
                None => matched = Some(path),
                Some(first) if first != path => return Err(RouteError::AmbiguousManeuver),
                Some(_) => {}
            }
        }
    }
    if !saw_entry {
        return Ok(None);
    }
    matched.ok_or(RouteError::ManeuverMismatch).map(Some)
}

/// 注册时按 hop 下标物化闸。tick 读 `hop_gate` 列，不得再线性扫机动出现项。
fn hop_gate_at(
    network: &SharedManeuverNetwork,
    maneuvers: &[ManeuverOccurrence],
    hop_index: usize,
    from: LaneEdgeOrdinal,
    to: LaneEdgeOrdinal,
) -> Option<ManeuverGateOrdinal> {
    let hop = u32::try_from(hop_index).ok()?;
    let occurrence = maneuvers.iter().find(|occurrence| {
        hop >= occurrence.entry_route_edge_index && hop < occurrence.exit_route_edge_index
    })?;
    let transition_index = hop.checked_sub(occurrence.entry_route_edge_index)?;
    network
        .transition_candidates(from)?
        .iter()
        .find(|candidate| {
            candidate.successor() == to
                && candidate.maneuver_path() == occurrence.path
                && candidate.transition_index() == transition_index
        })?
        .maneuver_gate()
}

fn access_cell_denied(cell: Option<AccessCell>) -> bool {
    match cell {
        Some(AccessCell::Unconstrained) => false,
        Some(AccessCell::Decided {
            effect: AccessEffect::Allow,
            ..
        }) => false,
        Some(AccessCell::Decided {
            effect: AccessEffect::Deny,
            ..
        }) => true,
        Some(_) | None => true,
    }
}

pub(crate) fn route_access_denied(
    traffic: &SharedTrafficNetwork,
    class: ParticipantClassOrdinal,
    edges: &[LaneEdgeOrdinal],
    cursor: usize,
    maneuvers: impl Iterator<Item = (ManeuverPathOrdinal, u32)>,
) -> bool {
    for edge in edges.iter().skip(cursor) {
        if access_cell_denied(traffic.relations().edge_access(*edge, class)) {
            return true;
        }
    }
    let cursor = u32::try_from(cursor).expect("route cursor fits u32");
    for (path, exit_index) in maneuvers {
        if exit_index <= cursor {
            continue;
        }
        if access_cell_denied(traffic.relations().path_access(path, class)) {
            return true;
        }
    }
    false
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RouteCursorPosition {
    route_edge_index: u32,
    progress_mm: u32,
    carry_um: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RouteRearPosition {
    BeforeRouteStart,
    Position(RouteCursorPosition),
}

/// 3A 检查内部错误；调用方把非规范 cursor 映射到各自既有 invariant 错误面。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConflictCapabilityError {
    InvalidCursor,
    RuntimeUnavailable(ConflictRuntimeUnavailable),
}

fn route_rear_position(
    lengths: &[u32],
    edges: &[LaneEdgeOrdinal],
    mut route_edge_index: usize,
    mut progress_mm: u32,
    carry_um: u16,
    length_mm: u32,
) -> Option<RouteRearPosition> {
    if carry_um >= 1_000 {
        return None;
    }
    let edge = *edges.get(route_edge_index)?;
    let edge_length = *lengths.get(edge.index())?;
    if progress_mm > edge_length || (progress_mm == edge_length && carry_um != 0) {
        return None;
    }
    if progress_mm == edge_length && carry_um == 0 && route_edge_index + 1 < edges.len() {
        route_edge_index += 1;
        progress_mm = 0;
    }

    let mut remaining_um = u64::from(length_mm) * 1_000;
    let mut offset_um = u64::from(progress_mm) * 1_000 + u64::from(carry_um);
    while remaining_um > offset_um {
        remaining_um -= offset_um;
        if route_edge_index == 0 {
            return Some(RouteRearPosition::BeforeRouteStart);
        }
        route_edge_index -= 1;
        let previous = *edges.get(route_edge_index)?;
        offset_um = u64::from(*lengths.get(previous.index())?) * 1_000;
    }
    let rear_um = offset_um - remaining_um;
    let rear = RouteCursorPosition {
        route_edge_index: u32::try_from(route_edge_index).ok()?,
        progress_mm: u32::try_from(rear_um / 1_000).ok()?,
        carry_um: u16::try_from(rear_um % 1_000).ok()?,
    };
    Some(RouteRearPosition::Position(rear))
}

/// #284 前的能力保护。只看 compiled route 的最后 clearance，不在 tick 扫描冲突表。
pub(crate) fn check_conflict_capability(
    route: RouteHandle,
    compiled: &CompiledRoute,
    lengths: &[u32],
    route_edge_index: usize,
    progress_mm: u32,
    carry_um: u16,
    vehicle_length_mm: u32,
) -> Result<(), ConflictCapabilityError> {
    let Some((final_clearance, conflict_index)) = compiled.final_conflict_clearance else {
        return Ok(());
    };
    let rear = route_rear_position(
        lengths,
        compiled.edges.as_slice(),
        route_edge_index,
        progress_mm,
        carry_um,
        vehicle_length_mm,
    )
    .ok_or(ConflictCapabilityError::InvalidCursor)?;
    let cleared = match rear {
        RouteRearPosition::BeforeRouteStart => false,
        RouteRearPosition::Position(position) => {
            (
                position.route_edge_index,
                position.progress_mm,
                position.carry_um,
            ) >= (
                final_clearance.route_edge_index,
                final_clearance.progress_mm,
                0,
            )
        }
    };
    if cleared {
        return Ok(());
    }
    let occurrence = compiled
        .conflicts
        .get(usize::try_from(conflict_index).expect("conflict index fits usize"))
        .expect("final conflict index references compiled occurrence");
    Err(ConflictCapabilityError::RuntimeUnavailable(
        ConflictRuntimeUnavailable::new(
            route,
            occurrence.stream,
            occurrence.passage_local_index,
            occurrence.zone,
        ),
    ))
}

pub(crate) fn bumpers_overlap(a_front: u32, a_length: u32, b_front: u32, b_length: u32) -> bool {
    let a_rear = i64::from(a_front) - i64::from(a_length);
    let b_rear = i64::from(b_front) - i64::from(b_length);
    a_rear < i64::from(b_front) && b_rear < i64::from(a_front)
}

const OCCUPANCY_INTERVAL_CAP: usize = 16;
type OccupancyInterval = (LaneEdgeOrdinal, u32, u32);
type OccupancyStack = ([OccupancyInterval; OCCUPANCY_INTERVAL_CAP], usize, bool);

pub(crate) fn for_each_occupancy_interval(
    lengths: &[u32],
    edges: &[LaneEdgeOrdinal],
    mut index: usize,
    mut end: u32,
    mut remaining: u32,
    mut visit: impl FnMut(LaneEdgeOrdinal, u32, u32),
) -> Option<()> {
    while remaining > 0 {
        let edge = *edges.get(index)?;
        let edge_length = *lengths.get(edge.index())?;
        let start = end.saturating_sub(remaining);
        let hi = end.min(edge_length);
        if hi > start {
            visit(edge, start, hi);
        }
        remaining = remaining.saturating_sub(hi.saturating_sub(start));
        if remaining == 0 || index == 0 {
            break;
        }
        index -= 1;
        end = *lengths.get(edges.get(index)?.index())?;
    }
    Some(())
}

fn occupancy_intervals_stack(
    lengths: &[u32],
    edges: &[LaneEdgeOrdinal],
    index: usize,
    end: u32,
    remaining: u32,
) -> Option<OccupancyStack> {
    let mut intervals = [(LaneEdgeOrdinal::from_raw(0), 0, 0); OCCUPANCY_INTERVAL_CAP];
    let mut count = 0;
    let mut overflow = false;
    for_each_occupancy_interval(lengths, edges, index, end, remaining, |edge, lo, hi| {
        if count < OCCUPANCY_INTERVAL_CAP {
            intervals[count] = (edge, lo, hi);
            count += 1;
        } else {
            overflow = true;
        }
    })?;
    Some((intervals, count, overflow))
}

fn occupancy_intervals_vec(
    lengths: &[u32],
    edges: &[LaneEdgeOrdinal],
    index: usize,
    end: u32,
    remaining: u32,
) -> Option<Vec<OccupancyInterval>> {
    let mut intervals = Vec::new();
    for_each_occupancy_interval(lengths, edges, index, end, remaining, |edge, lo, hi| {
        intervals.push((edge, lo, hi));
    })?;
    Some(intervals)
}

fn occupancy_slices_overlap(left: &[OccupancyInterval], right: &[OccupancyInterval]) -> bool {
    left.iter().any(|(edge, a_lo, a_hi)| {
        right
            .iter()
            .any(|(other, b_lo, b_hi)| *edge == *other && *a_lo < *b_hi && *b_lo < *a_hi)
    })
}

/// 两个 route cursor 展开的完整车身物理 footprint 是否逐项相等。
///
/// 短车身走固定栈；超过 16 个物理区间时才按 route occurrence 上界可失败预留。
#[allow(clippy::too_many_arguments)]
pub(crate) fn occupancy_footprints_equal(
    lengths: &[u32],
    a_edges: &[LaneEdgeOrdinal],
    a_index: usize,
    a_progress: u32,
    a_length: u32,
    b_edges: &[LaneEdgeOrdinal],
    b_index: usize,
    b_progress: u32,
    b_length: u32,
) -> Result<bool, ()> {
    let Some((left, left_n, left_overflow)) =
        occupancy_intervals_stack(lengths, a_edges, a_index, a_progress, a_length)
    else {
        return Ok(false);
    };
    let Some((right, right_n, right_overflow)) =
        occupancy_intervals_stack(lengths, b_edges, b_index, b_progress, b_length)
    else {
        return Ok(false);
    };
    if !left_overflow && !right_overflow {
        return Ok(left[..left_n] == right[..right_n]);
    }

    let mut left_full = Vec::new();
    left_full.try_reserve_exact(a_edges.len()).map_err(|_| ())?;
    for_each_occupancy_interval(
        lengths,
        a_edges,
        a_index,
        a_progress,
        a_length,
        |edge, lo, hi| left_full.push((edge, lo, hi)),
    )
    .ok_or(())?;
    let mut right_full = Vec::new();
    right_full
        .try_reserve_exact(b_edges.len())
        .map_err(|_| ())?;
    for_each_occupancy_interval(
        lengths,
        b_edges,
        b_index,
        b_progress,
        b_length,
        |edge, lo, hi| right_full.push((edge, lo, hi)),
    )
    .ok_or(())?;
    Ok(left_full == right_full)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn bodies_overlap(
    lengths: &[u32],
    a_edges: &[LaneEdgeOrdinal],
    a_index: usize,
    a_progress: u32,
    a_length: u32,
    b_edges: &[LaneEdgeOrdinal],
    b_index: usize,
    b_progress: u32,
    b_length: u32,
) -> bool {
    if a_edges.get(a_index) == b_edges.get(b_index)
        && bumpers_overlap(a_progress, a_length, b_progress, b_length)
    {
        return true;
    }
    let Some((left, left_n, left_overflow)) =
        occupancy_intervals_stack(lengths, a_edges, a_index, a_progress, a_length)
    else {
        return false;
    };
    let Some((right, right_n, right_overflow)) =
        occupancy_intervals_stack(lengths, b_edges, b_index, b_progress, b_length)
    else {
        return false;
    };
    if left_overflow || right_overflow {
        let Some(left) = occupancy_intervals_vec(lengths, a_edges, a_index, a_progress, a_length)
        else {
            return false;
        };
        let Some(right) = occupancy_intervals_vec(lengths, b_edges, b_index, b_progress, b_length)
        else {
            return false;
        };
        return occupancy_slices_overlap(&left, &right);
    }
    occupancy_slices_overlap(&left[..left_n], &right[..right_n])
}

/// 前车占用相对后车前保险杠的间隙。走完占用才返回 `Some`；中途失败丢弃局部结果。
/// 后车剩余边序列上每一次同名边出现都计入，取最近间隙。
#[allow(clippy::too_many_arguments)]
pub(crate) fn occupancy_front_gap(
    lengths: &[u32],
    follower_edges: &[LaneEdgeOrdinal],
    follower_index: usize,
    follower_progress: u32,
    leader_edges: &[LaneEdgeOrdinal],
    leader_index: usize,
    leader_progress: u32,
    leader_length: u32,
) -> Option<i64> {
    let mut gap: Option<i64> = None;
    for_each_occupancy_interval(
        lengths,
        leader_edges,
        leader_index,
        leader_progress,
        leader_length,
        |edge, lo, hi| {
            for (found, candidate) in follower_edges.iter().enumerate().skip(follower_index) {
                if *candidate != edge {
                    continue;
                }
                let bumper = if found == follower_index {
                    if hi <= follower_progress {
                        continue;
                    }
                    i64::from(lo) - i64::from(follower_progress)
                } else {
                    let Some(front_to_rear) = remaining_along_route_i64(
                        lengths,
                        follower_edges,
                        follower_index,
                        follower_progress,
                        found,
                        lo,
                    ) else {
                        continue;
                    };
                    front_to_rear
                };
                gap = Some(gap.map_or(bumper, |current| current.min(bumper)));
            }
        },
    )?;
    gap
}

/// 占用/投影有符号间隙（ADR 0028）。`i64` 只服务空隙，不是把路线前缀加宽到 `u64` 的先例。
pub(crate) fn remaining_along_route_i64(
    lengths: &[u32],
    edges: &[LaneEdgeOrdinal],
    from_index: usize,
    from_progress: u32,
    to_index: usize,
    to_progress: u32,
) -> Option<i64> {
    if to_index < from_index || (to_index == from_index && to_progress < from_progress) {
        return None;
    }
    if to_index == from_index {
        return Some(i64::from(to_progress) - i64::from(from_progress));
    }
    let from_edge = *edges.get(from_index)?;
    let mut distance = i64::from(*lengths.get(from_edge.index())?) - i64::from(from_progress);
    for edge in edges.get(from_index + 1..to_index)? {
        distance = distance.checked_add(i64::from(*lengths.get(edge.index())?))?;
    }
    distance.checked_add(i64::from(to_progress))
}

/// 当前进度到路终。O(1) 读后缀列再扣边内进度；不上 `u64`。
pub(crate) fn remaining_to_route_end(
    remaining_to_end: BoundedDistance,
    from_progress: u32,
) -> BoundedDistance {
    remaining_to_end.saturating_sub(from_progress)
}

/// 当前进度到目标边起点。两端后缀相减，不扫剩余边。
///
/// 两端都 `BeyondFinite` 时差仍越界，不能恢复近处有限窗口；限速下降用
/// [`distance_to_occurrence_start`]。
#[cfg(test)]
pub(crate) fn remaining_to_occurrence_start(
    remaining_to_end: &[BoundedDistance],
    from_index: usize,
    from_progress: u32,
    to_index: usize,
) -> Option<BoundedDistance> {
    if to_index < from_index {
        return None;
    }
    let from = remaining_to_end.get(from_index).copied()?;
    let to = remaining_to_end.get(to_index).copied()?;
    Some(
        from.saturating_sub(from_progress)
            .saturating_sub_bounded(to),
    )
}

/// 当前进度到目标边起点。用分段 `u32` 坐标算查询窗口，不上 `u64`。
///
/// 窗口本身溢出才是 `BeyondFinite`；路终越界不影响近处有限距离。
pub(crate) fn distance_to_occurrence_start(
    segments: &[u32],
    offsets: &[u32],
    totals: &[u32],
    from_index: usize,
    from_progress: u32,
    to_index: usize,
) -> Option<BoundedDistance> {
    if to_index < from_index {
        return None;
    }
    let from_seg = usize::try_from(*segments.get(from_index)?).ok()?;
    let to_seg = usize::try_from(*segments.get(to_index)?).ok()?;
    if to_seg < from_seg {
        return None;
    }
    let from_off = offsets.get(from_index)?.checked_add(from_progress)?;
    let to_off = *offsets.get(to_index)?;
    if from_seg == to_seg {
        return Some(BoundedDistance::Finite(to_off.saturating_sub(from_off)));
    }
    let from_total = *totals.get(from_seg)?;
    let mut distance = BoundedDistance::Finite(from_total.checked_sub(from_off)?);
    for total in totals.get(from_seg + 1..to_seg)? {
        distance = distance.add_u32(*total);
    }
    Some(distance.add_u32(to_off))
}

/// 当前 route cursor 到目标 occurrence 内 exact progress 的分段有界距离。
pub(crate) fn distance_to_occurrence_progress(
    segments: &[u32],
    offsets: &[u32],
    totals: &[u32],
    from_index: usize,
    from_progress: u32,
    to_index: usize,
    to_progress: u32,
) -> Option<BoundedDistance> {
    if to_index < from_index {
        return None;
    }
    let from_seg = usize::try_from(*segments.get(from_index)?).ok()?;
    let to_seg = usize::try_from(*segments.get(to_index)?).ok()?;
    if to_seg < from_seg {
        return None;
    }
    let from_off = offsets.get(from_index)?.checked_add(from_progress)?;
    let to_off = offsets.get(to_index)?.checked_add(to_progress)?;
    if from_seg == to_seg {
        return Some(BoundedDistance::Finite(to_off.checked_sub(from_off)?));
    }
    let from_total = *totals.get(from_seg)?;
    let mut distance = BoundedDistance::Finite(from_total.checked_sub(from_off)?);
    for total in totals.get(from_seg + 1..to_seg)? {
        distance = distance.add_u32(*total);
    }
    Some(distance.add_u32(to_off))
}

#[cfg(test)]
mod unique_entry_path_match_tests {
    use super::*;
    use laneflow_static_contract::LaneEdgeOrdinal;

    fn edge(raw: u32) -> LaneEdgeOrdinal {
        LaneEdgeOrdinal::from_raw(raw)
    }

    fn path(raw: u32) -> ManeuverPathOrdinal {
        ManeuverPathOrdinal::from_raw(raw)
    }

    #[test]
    fn remaining_abd_selects_path_abd_not_abc() {
        let abc = [edge(0), edge(1), edge(2)];
        let abd = [edge(0), edge(1), edge(3)];
        let remaining = [edge(0), edge(1), edge(3)];
        let matched = unique_entry_path_match_filtered(
            &remaining,
            [(path(0), abc.as_slice()), (path(1), abd.as_slice())],
        )
        .expect("unique");
        assert_eq!(matched, Some(path(1)));
    }

    #[test]
    fn remaining_abc_selects_path_abc() {
        let abc = [edge(0), edge(1), edge(2)];
        let abd = [edge(0), edge(1), edge(3)];
        let remaining = [edge(0), edge(1), edge(2)];
        let matched = unique_entry_path_match_filtered(
            &remaining,
            [(path(0), abc.as_slice()), (path(1), abd.as_slice())],
        )
        .expect("unique");
        assert_eq!(matched, Some(path(0)));
    }

    #[test]
    fn two_prefix_matches_are_ambiguous() {
        let short = [edge(0), edge(1), edge(2)];
        let long = [edge(0), edge(1), edge(2), edge(3)];
        let remaining = [edge(0), edge(1), edge(2), edge(3)];
        assert_eq!(
            unique_entry_path_match_filtered(
                &remaining,
                [(path(0), short.as_slice()), (path(1), long.as_slice())],
            ),
            Err(RouteError::AmbiguousManeuver)
        );
    }

    #[test]
    fn entry_without_complete_path_is_mismatch() {
        let abc = [edge(0), edge(1), edge(2)];
        let abd = [edge(0), edge(1), edge(3)];
        let remaining = [edge(0), edge(1)];
        assert_eq!(
            unique_entry_path_match_filtered(
                &remaining,
                [(path(0), abc.as_slice()), (path(1), abd.as_slice())],
            ),
            Err(RouteError::ManeuverMismatch)
        );
    }

    #[test]
    fn no_entry_candidate_is_not_a_maneuver_start() {
        let remaining = [edge(0), edge(1)];
        assert_eq!(unique_entry_path_match_filtered(&remaining, []), Ok(None));
    }

    #[test]
    fn duplicate_same_path_is_unique() {
        let abc = [edge(0), edge(1), edge(2)];
        let remaining = [edge(0), edge(1), edge(2)];
        let matched = unique_entry_path_match_filtered(
            &remaining,
            [(path(0), abc.as_slice()), (path(0), abc.as_slice())],
        )
        .expect("unique");
        assert_eq!(matched, Some(path(0)));
    }

    #[test]
    fn overlapping_internal_coverage_is_mismatch() {
        let mut coverage = [None, None, None, None];
        claim_internal_coverage(&mut coverage, 0, 3, path(0)).unwrap();
        assert_eq!(
            claim_internal_coverage(&mut coverage, 1, 3, path(1)),
            Err(RouteError::ManeuverMismatch)
        );
    }

    #[test]
    fn adjacent_occurrences_do_not_share_internal_slots() {
        let mut coverage = [None, None, None, None, None];
        claim_internal_coverage(&mut coverage, 0, 2, path(0)).unwrap();
        claim_internal_coverage(&mut coverage, 2, 4, path(1)).unwrap();
        assert_eq!(coverage[1], Some(path(0)));
        assert_eq!(coverage[2], None);
        assert_eq!(coverage[3], Some(path(1)));
    }

    #[test]
    fn hop_ranges_touching_at_exit_do_not_overlap() {
        assert!(!hop_ranges_overlap(0, 2, 2, 4));
        assert!(hop_ranges_overlap(0, 3, 2, 4));
    }

    #[test]
    fn occurrence_starting_on_last_internal_is_mismatch() {
        let mut coverage = [None, None, None, None];
        record_occurrence(&mut coverage, &[], 0, 3, path(0)).unwrap();
        let recorded = [ManeuverOccurrence {
            path: path(0),
            entry_route_edge_index: 0,
            exit_route_edge_index: 3,
        }];
        assert_eq!(
            record_occurrence(&mut coverage, &recorded, 2, 3, path(1)),
            Err(RouteError::ManeuverMismatch)
        );
    }

    #[test]
    fn record_adjacent_occurrences_succeeds() {
        let mut coverage = [None, None, None, None, None];
        record_occurrence(&mut coverage, &[], 0, 2, path(0)).unwrap();
        let recorded = [ManeuverOccurrence {
            path: path(0),
            entry_route_edge_index: 0,
            exit_route_edge_index: 2,
        }];
        record_occurrence(&mut coverage, &recorded, 2, 4, path(1)).unwrap();
    }
}

#[cfg(test)]
mod compile_route_tests {
    use super::*;
    use std::sync::Arc;

    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
    );

    fn revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).unwrap();
        build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .unwrap()
    }

    #[test]
    fn s1_fixture_compiles_the_single_maneuver_path() {
        let revision = revision();
        let traffic = revision.traffic();
        let path = traffic
            .maneuvers()
            .maneuver_path(ManeuverPathOrdinal::from_raw(0))
            .expect("fixture path");
        let compiled =
            compile_route(revision.as_ref(), path.edges(), 0, u64::MAX).expect("compile");
        assert_eq!(compiled.maneuvers.len(), 1);
        assert_eq!(compiled.maneuvers[0].path, ManeuverPathOrdinal::from_raw(0));
        assert_eq!(compiled.maneuvers[0].entry_route_edge_index, 0);
        assert_eq!(
            compiled.maneuvers[0].exit_route_edge_index,
            u32::try_from(path.edges().len() - 1).expect("path length")
        );
        assert_eq!(
            compiled.hop_gate.first().copied().flatten(),
            path.maneuver_gates().first().copied()
        );
        let lengths = traffic.lane_lengths_millimetres();
        let mut expected = BoundedDistance::Finite(0);
        for index in (0..path.edges().len()).rev() {
            expected = expected.add_u32(*lengths.get(path.edges()[index].index()).unwrap());
            assert_eq!(compiled.remaining_to_end[index], expected);
        }
        assert_eq!(
            remaining_to_route_end(compiled.remaining_to_end[0], 0),
            compiled.remaining_to_end[0]
        );
        let hop_count = path.edges().len().saturating_sub(1);
        assert_eq!(compiled.hop_gate.len(), hop_count);
        assert_eq!(compiled.next_controlled.len(), hop_count);
        if let Some(gate) = compiled.hop_gate.first().copied().flatten() {
            let next = compiled
                .next_controlled
                .first()
                .copied()
                .flatten()
                .expect("first hop with a gate is controlled");
            assert_eq!(next.hop, 0);
            assert_eq!(next.gate, gate);
            let suffix = remaining_to_occurrence_start(
                &compiled.remaining_to_end,
                0,
                0,
                usize::try_from(next.hop).expect("hop") + 1,
            );
            if matches!(suffix, Some(BoundedDistance::Finite(_))) {
                assert_eq!(suffix, Some(next.distance_from_hop_start));
            }
        }
    }

    #[test]
    fn every_conflict_anchor_has_one_exact_route_position() {
        let revision = revision();
        let path = revision
            .traffic()
            .maneuvers()
            .maneuver_path(ManeuverPathOrdinal::from_raw(0))
            .expect("fixture path");
        assert_eq!(path.edges().len(), 3, "fixture shape");
        let occurrence = ManeuverOccurrence {
            path: ManeuverPathOrdinal::from_raw(0),
            entry_route_edge_index: 2,
            exit_route_edge_index: 4,
        };
        let gate = path.maneuver_gates()[0];
        assert_eq!(
            map_conflict_anchor(
                revision.as_ref(),
                6,
                occurrence,
                ConflictPathAnchor::Gate(gate)
            ),
            Ok(RoutePosition {
                route_edge_index: 3,
                progress_mm: 0,
            })
        );
        assert_eq!(
            map_conflict_anchor(
                revision.as_ref(),
                6,
                occurrence,
                ConflictPathAnchor::EdgeBoundary(0),
            ),
            Ok(RoutePosition {
                route_edge_index: 2,
                progress_mm: 0,
            })
        );
        assert_eq!(
            map_conflict_anchor(
                revision.as_ref(),
                6,
                occurrence,
                ConflictPathAnchor::EdgeBoundary(1),
            ),
            Ok(RoutePosition {
                route_edge_index: 3,
                progress_mm: 0,
            })
        );
        assert_eq!(
            map_conflict_anchor(
                revision.as_ref(),
                6,
                occurrence,
                ConflictPathAnchor::EdgeBoundary(3),
            ),
            Ok(RoutePosition {
                route_edge_index: 5,
                progress_mm: 0,
            })
        );
        let terminal_length = revision.traffic().lane_lengths_millimetres()
            [path.edges().last().expect("last edge").index()];
        assert_eq!(
            map_conflict_anchor(
                revision.as_ref(),
                5,
                occurrence,
                ConflictPathAnchor::EdgeBoundary(3),
            ),
            Ok(RoutePosition {
                route_edge_index: 4,
                progress_mm: terminal_length,
            })
        );
        assert_eq!(
            map_conflict_anchor(
                revision.as_ref(),
                6,
                occurrence,
                ConflictPathAnchor::Interior {
                    path_edge_index: 1,
                    progress_millimetres: 1,
                },
            ),
            Ok(RoutePosition {
                route_edge_index: 3,
                progress_mm: 1,
            })
        );
    }

    #[test]
    fn conflict_multiplicity_sorts_ranges_and_uses_max_clearance() {
        let occurrence = |stream: u32,
                          passage_local_index: u32,
                          maneuver_index: u32,
                          admission_hop: u32,
                          route_edge_index: u32,
                          entry_mm: u32,
                          clearance_mm: u32| ConflictPassageOccurrence {
            stream: ParticipantStreamOrdinal::from_raw(stream),
            passage_local_index,
            zone: ConflictZoneOrdinal::from_raw(passage_local_index),
            maneuver_index,
            admission_hop,
            entry: RoutePosition {
                route_edge_index,
                progress_mm: entry_mm,
            },
            clearance: RoutePosition {
                route_edge_index,
                progress_mm: clearance_mm,
            },
        };
        let conflicts = vec![
            occurrence(1, 1, 1, 3, 4, 6_500, 10_000),
            occurrence(0, 0, 0, 0, 1, 2_000, 6_000),
            occurrence(1, 0, 1, 3, 4, 3_000, 7_000),
            occurrence(0, 1, 1, 3, 4, 5_000, 11_000),
            occurrence(1, 0, 0, 0, 1, 3_000, 7_000),
            occurrence(0, 1, 0, 0, 1, 5_000, 11_000),
            occurrence(0, 0, 1, 3, 4, 2_000, 6_000),
            occurrence(1, 1, 0, 0, 1, 6_500, 10_000),
        ];

        let (conflicts, ranges, final_clearance) =
            finalize_conflicts(conflicts, 6).expect("finalize repeated conflicts");
        assert_eq!(
            ranges,
            [
                ConflictGateRange { start: 0, len: 4 },
                ConflictGateRange { start: 4, len: 0 },
                ConflictGateRange { start: 4, len: 0 },
                ConflictGateRange { start: 4, len: 4 },
                ConflictGateRange { start: 8, len: 0 },
                ConflictGateRange { start: 8, len: 0 },
            ]
        );
        assert_eq!(
            conflicts
                .iter()
                .map(|value| (
                    value.maneuver_index,
                    value.stream.raw(),
                    value.passage_local_index,
                ))
                .collect::<Vec<_>>(),
            [
                (0, 0, 0),
                (0, 1, 0),
                (0, 0, 1),
                (0, 1, 1),
                (1, 0, 0),
                (1, 1, 0),
                (1, 0, 1),
                (1, 1, 1),
            ]
        );
        assert_eq!(
            final_clearance,
            Some((
                RoutePosition {
                    route_edge_index: 4,
                    progress_mm: 11_000,
                },
                6,
            )),
            "final clearance is the maximum exit, not the last sorted entry",
        );
    }

    #[test]
    fn conflict_vector_and_gate_range_allocation_failpoints_are_closed() {
        assert_eq!(
            with_route_allocation_failure_after(0, || try_conflict_occurrence_vec(1)),
            Err(RouteError::AllocationFailed),
        );
        assert_eq!(
            with_route_allocation_failure_after(0, || finalize_conflicts(Vec::new(), 1)),
            Err(RouteError::AllocationFailed),
        );
    }

    #[test]
    fn three_a_rear_boundary_is_exact_to_one_micrometre() {
        let revision = revision();
        let path = revision
            .traffic()
            .maneuvers()
            .maneuver_path(ManeuverPathOrdinal::from_raw(0))
            .expect("fixture path");
        let final_clearance = RoutePosition {
            route_edge_index: 2,
            progress_mm: 0,
        };
        let compiled = CompiledRoute {
            edges: path.edges().to_vec(),
            maneuvers: Vec::new(),
            hop_gate: Vec::new(),
            gate_hops: Vec::new(),
            remaining_to_end: Vec::new(),
            occurrence_segments: Vec::new(),
            occurrence_offsets: Vec::new(),
            segment_totals: Vec::new(),
            next_controlled: Vec::new(),
            speed_limit_drop: Vec::new(),
            waiting: Vec::new(),
            conflicts: vec![ConflictPassageOccurrence {
                stream: ParticipantStreamOrdinal::from_raw(7),
                passage_local_index: 3,
                zone: ConflictZoneOrdinal::from_raw(5),
                maneuver_index: 0,
                admission_hop: 0,
                entry: RoutePosition {
                    route_edge_index: 1,
                    progress_mm: 1,
                },
                clearance: final_clearance,
            }],
            conflict_gate_ranges: vec![ConflictGateRange { start: 0, len: 1 }],
            final_conflict_clearance: Some((final_clearance, 0)),
        };
        assert_eq!(
            compiled.retained_logical_bytes(),
            logical_vec_bytes::<LaneEdgeOrdinal>(3)
                + logical_vec_bytes::<ConflictPassageOccurrence>(1)
                + logical_vec_bytes::<ConflictGateRange>(1)
        );
        let route = RouteHandle::new(9, 4);
        let lengths = revision.traffic().lane_lengths_millimetres();

        let before =
            check_conflict_capability(route, &compiled, lengths, 2, 4_499, 999, 4_500).unwrap_err();
        let ConflictCapabilityError::RuntimeUnavailable(unavailable) = before else {
            panic!("one micrometre before clearance must be a 3A rejection");
        };
        assert_eq!(unavailable.route(), route);
        assert_eq!(
            check_conflict_capability(route, &compiled, lengths, 2, 4_500, 0, 4_500),
            Ok(())
        );
        assert_eq!(
            check_conflict_capability(route, &compiled, lengths, 2, 4_500, 1, 4_500),
            Ok(())
        );
        assert_eq!(
            check_conflict_capability(route, &compiled, lengths, 2, 4_500, 0, 4_499),
            Ok(()),
            "a shorter vehicle clears the same passage earlier",
        );
        assert!(matches!(
            check_conflict_capability(route, &compiled, lengths, 2, 4_500, 0, 4_501),
            Err(ConflictCapabilityError::RuntimeUnavailable(_))
        ));

        let previous_length = lengths[path.edges()[1].index()];
        assert_eq!(
            check_conflict_capability(route, &compiled, lengths, 1, previous_length, 0, 0),
            check_conflict_capability(route, &compiled, lengths, 2, 0, 0, 0),
            "previous edge end and next edge zero are one canonical position",
        );
        assert_eq!(
            check_conflict_capability(route, &compiled, lengths, 1, previous_length, 1, 0),
            Err(ConflictCapabilityError::InvalidCursor),
        );
    }

    #[test]
    fn conflict_capacity_checked_add_overflow_is_closed() {
        assert_eq!(
            checked_conflict_occurrence_total(u64::MAX, 1, u64::MAX).unwrap_err(),
            RouteError::ConflictOccurrenceCapacityExceeded {
                current: u64::MAX,
                added: 1,
                capacity: u64::MAX,
            },
        );
    }

    #[test]
    fn conflict_occurrence_10k_100k_retained_logical_bytes_are_linear() {
        let per_occurrence = logical_vec_bytes::<ConflictPassageOccurrence>(1);
        assert_eq!(
            per_occurrence,
            u64::try_from(std::mem::size_of::<ConflictPassageOccurrence>())
                .expect("type size fits u64"),
        );
        let product = per_occurrence.checked_mul(10_000).expect("10k ledger");
        let scaling = per_occurrence.checked_mul(100_000).expect("100k ledger");
        assert_eq!(scaling, product * 10);
        println!(
            "conflict-route-scale-evidence retained_logical_bytes_per_occurrence={per_occurrence} occurrences=10000/100000 retained_logical_bytes={product}/{scaling}"
        );
    }

    #[test]
    fn suffix_window_cannot_recover_nearby_finite_when_both_ends_overflow() {
        let remaining = [BoundedDistance::BeyondFinite, BoundedDistance::BeyondFinite];
        assert_eq!(
            remaining_to_occurrence_start(&remaining, 0, 999, 1),
            Some(BoundedDistance::BeyondFinite)
        );
        assert_eq!(
            BoundedDistance::Finite(6_000).saturating_sub(999),
            BoundedDistance::Finite(5_001)
        );
    }

    #[test]
    fn segmented_window_recovers_nearby_finite_across_suffix_overflow() {
        let lengths = [1_000_u32, u32::MAX];
        let (segments, offsets, totals) =
            segmented_route_coordinates(&lengths).expect("segmented coordinates");
        assert_eq!(segments.as_slice(), [0, 1]);
        assert_eq!(
            distance_to_occurrence_start(&segments, &offsets, &totals, 0, 100, 1),
            Some(BoundedDistance::Finite(900))
        );
        let remaining = [
            BoundedDistance::Finite(1_000).add_u32(u32::MAX),
            BoundedDistance::Finite(u32::MAX),
        ];
        assert_eq!(remaining[0], BoundedDistance::BeyondFinite);
        assert_eq!(
            remaining_to_occurrence_start(&remaining, 0, 100, 1),
            Some(BoundedDistance::BeyondFinite)
        );
    }

    #[test]
    fn starting_on_internal_maneuver_edge_is_mismatch() {
        let revision = revision();
        let traffic = revision.traffic();
        let path = traffic
            .maneuvers()
            .maneuver_path(ManeuverPathOrdinal::from_raw(0))
            .expect("fixture path");
        assert!(
            path.edges().len() >= 3,
            "fixture path must have an internal hop"
        );
        assert_eq!(
            compile_route(revision.as_ref(), &path.edges()[1..], 0, u64::MAX).unwrap_err(),
            RouteError::ManeuverMismatch
        );
    }

    #[test]
    fn ending_on_stop_line_edge_is_mismatch() {
        let revision = revision();
        let traffic = revision.traffic();
        let path = traffic
            .maneuvers()
            .maneuver_path(ManeuverPathOrdinal::from_raw(0))
            .expect("fixture path");
        let entry = path.edges()[0];
        assert!(
            traffic.relations().stop_line_for_edge(entry).is_some(),
            "fixture path entry must carry a StopLine"
        );
        assert_eq!(
            compile_route(revision.as_ref(), &[entry], 0, u64::MAX).unwrap_err(),
            RouteError::ManeuverMismatch
        );
    }

    #[test]
    fn s1_path_hops_are_connected_by_successor_or_maneuver() {
        let revision = revision();
        let traffic = revision.traffic();
        let path = traffic
            .maneuvers()
            .maneuver_path(ManeuverPathOrdinal::from_raw(0))
            .expect("fixture path");
        for pair in path.edges().windows(2) {
            assert!(
                route_pair_connected(traffic, pair[0], pair[1]),
                "path hop {:?} -> {:?} must be connected",
                pair[0],
                pair[1]
            );
        }
        compile_route(revision.as_ref(), path.edges(), 0, u64::MAX)
            .expect("full path still compiles");
    }

    #[test]
    fn internal_maneuver_hop_is_not_an_entry_match() {
        let revision = revision();
        let traffic = revision.traffic();
        let path = traffic
            .maneuvers()
            .maneuver_path(ManeuverPathOrdinal::from_raw(0))
            .expect("fixture path");
        assert!(
            path.edges().len() >= 3,
            "fixture path must have an internal hop"
        );
        assert_eq!(
            unique_entry_path_match(
                traffic.maneuvers(),
                path.edges()[1],
                path.edges()[2],
                &path.edges()[1..],
            ),
            Ok(None)
        );
    }

    #[test]
    fn single_internal_edge_is_mismatch() {
        let revision = revision();
        let traffic = revision.traffic();
        let path = traffic
            .maneuvers()
            .maneuver_path(ManeuverPathOrdinal::from_raw(0))
            .expect("fixture path");
        let internal = path.edges()[1];
        assert!(
            traffic.relations().lane_edge_junction(internal).is_some(),
            "fixture internal edge must belong to a junction"
        );
        assert_eq!(
            compile_route(revision.as_ref(), &[internal], 0, u64::MAX).unwrap_err(),
            RouteError::ManeuverMismatch
        );
    }

    #[test]
    fn ending_on_internal_maneuver_edge_is_mismatch() {
        let revision = revision();
        let traffic = revision.traffic();
        let path = traffic
            .maneuvers()
            .maneuver_path(ManeuverPathOrdinal::from_raw(0))
            .expect("fixture path");
        let prefix = &path.edges()[..2];
        assert!(
            traffic
                .relations()
                .lane_edge_junction(*prefix.last().expect("prefix"))
                .is_some(),
            "truncated path must end on a junction-owned edge"
        );
        assert_eq!(
            compile_route(revision.as_ref(), prefix, 0, u64::MAX).unwrap_err(),
            RouteError::ManeuverMismatch
        );
    }
}

#[cfg(test)]
mod occupancy_interval_tests {
    use super::*;
    use laneflow_static_contract::LaneEdgeOrdinal;

    fn edge(raw: u32) -> LaneEdgeOrdinal {
        LaneEdgeOrdinal::from_raw(raw)
    }

    fn chain(start: u32, count: usize) -> Vec<LaneEdgeOrdinal> {
        (0..count)
            .map(|offset| edge(start + u32::try_from(offset).expect("edge ordinal")))
            .collect()
    }

    fn overflow_count() -> usize {
        OCCUPANCY_INTERVAL_CAP + 1
    }

    fn overflow_body_length() -> u32 {
        u32::try_from(overflow_count() * 2 - 1).expect("body fits")
    }

    #[test]
    fn overflow_without_shared_edges_is_not_overlap() {
        let span = overflow_count();
        let lengths = vec![2; span * 2];
        let left = chain(0, span);
        let right = chain(u32::try_from(span).expect("start"), span);
        let last = span - 1;
        let body = overflow_body_length();
        assert!(!bodies_overlap(
            &lengths, &left, last, 2, body, &right, last, 2, body,
        ));
    }

    #[test]
    fn overflow_is_not_overlap_when_only_dropped_interval_is_disjoint() {
        let span = overflow_count();
        let lengths = vec![2; span];
        let edges = chain(0, span);
        let last = span - 1;
        assert!(!bodies_overlap(
            &lengths,
            &edges,
            last,
            2,
            overflow_body_length(),
            &edges,
            0,
            1,
            1,
        ));
    }

    #[test]
    fn overflow_still_overlaps_when_only_dropped_interval_collides() {
        let span = overflow_count();
        let lengths = vec![2; span];
        let edges = chain(0, span);
        let last = span - 1;
        assert!(bodies_overlap(
            &lengths,
            &edges,
            last,
            2,
            overflow_body_length(),
            &edges,
            0,
            2,
            1,
        ));
    }

    #[test]
    fn occupancy_front_gap_discards_partial_walk() {
        let lengths = [1];
        let follower = [edge(0)];
        let leader = [edge(99), edge(0)];
        assert_eq!(
            occupancy_front_gap(&lengths, &follower, 0, 0, &leader, 1, 1, 2),
            None
        );
    }

    #[test]
    fn occupancy_front_gap_returns_rear_bumper_on_same_edge() {
        let lengths = [10_000];
        let edges = [edge(0)];
        assert_eq!(
            occupancy_front_gap(&lengths, &edges, 0, 1_000, &edges, 0, 6_000, 2_000),
            Some(3_000)
        );
    }

    #[test]
    fn occupancy_front_gap_uses_later_repeated_edge() {
        let lengths = [10_000, 5_000];
        let follower = [edge(0), edge(1), edge(0)];
        let leader = [edge(0)];
        assert_eq!(
            occupancy_front_gap(&lengths, &follower, 0, 9_000, &leader, 0, 1_000, 1_000),
            Some(6_000)
        );
    }

    #[test]
    fn occupancy_front_gap_prefers_nearest_repeated_edge() {
        let lengths = [10_000, 5_000];
        let follower = [edge(0), edge(1), edge(0)];
        let leader = [edge(0)];
        assert_eq!(
            occupancy_front_gap(&lengths, &follower, 0, 1_000, &leader, 0, 6_000, 2_000),
            Some(3_000)
        );
    }
}
