use laneflow_static_contract::{
    AccessEffect, LaneEdgeOrdinal, ManeuverGateOrdinal, ManeuverPathOrdinal,
    ParticipantClassOrdinal, WaitingZoneOrdinal,
};
use laneflow_static_network::{
    AccessCell, BoundedDistance, SharedManeuverNetwork, SharedTrafficNetwork,
};

use crate::{RouteError, VehicleState};

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
}

/// 本世界 compiled 路线：分段 `u32` 前缀、后缀 `BoundedDistance`、hop 门、
/// 受控 hop 链和限速下降转换。
/// 不上 `u64`，不把 world 身份写进 `RouteHandle`，不存「当前红灯」（ADR 0028 / 0029）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompiledRoute {
    pub edges: Box<[LaneEdgeOrdinal]>,
    pub maneuvers: Box<[ManeuverOccurrence]>,
    pub hop_gate: Box<[Option<ManeuverGateOrdinal>]>,
    pub remaining_to_end: Box<[BoundedDistance]>,
    pub occurrence_segments: Box<[u32]>,
    pub occurrence_offsets: Box<[u32]>,
    pub segment_totals: Box<[u32]>,
    pub next_controlled: Box<[Option<NextControlled>]>,
    pub speed_limit_drop: Box<[SpeedLimitDrop]>,
    pub waiting: Box<[WaitingOccurrence]>,
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
    traffic: &SharedTrafficNetwork,
    edges: &[LaneEdgeOrdinal],
) -> Result<CompiledRoute, RouteError> {
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

    let mut maneuvers: Vec<ManeuverOccurrence> = Vec::new();
    let network = traffic.maneuvers();
    let transition_len = edges.len().saturating_sub(1);
    let mut coverage = vec![None; edges.len()];
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
    let mut remaining_to_end = vec![BoundedDistance::Finite(0); edges.len()];
    let mut suffix = BoundedDistance::Finite(0);
    for index in (0..edges.len()).rev() {
        let edge = edges[index];
        suffix = suffix.add(*lengths.get(edge.index()).unwrap_or(&0));
        remaining_to_end[index] = suffix;
    }
    let route_lengths: Vec<u32> = edges
        .iter()
        .map(|edge| *lengths.get(edge.index()).unwrap_or(&0))
        .collect();
    let (occurrence_segments, occurrence_offsets, segment_totals) =
        segmented_route_coordinates(&route_lengths);

    let hop_count = edges.len().saturating_sub(1);
    let mut hop_gate = vec![None; hop_count];
    for hop in 0..hop_count {
        hop_gate[hop] = hop_gate_at(network, &maneuvers, hop, edges[hop], edges[hop + 1]);
    }

    let mut next_controlled = vec![None; hop_count];
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
                distance_from_hop_start: BoundedDistance::Finite(0).add(length),
            });
        } else if let Some(controlled) = next.as_mut() {
            controlled.distance_from_hop_start = controlled.distance_from_hop_start.add(length);
        }
        next_controlled[hop] = next;
    }

    let mut speed_limit_drop = Vec::new();
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

    let waiting = compile_waiting(traffic, &hop_gate, &maneuvers)?;

    Ok(CompiledRoute {
        edges: edges.to_vec().into_boxed_slice(),
        maneuvers: maneuvers.into_boxed_slice(),
        hop_gate: hop_gate.into_boxed_slice(),
        remaining_to_end: remaining_to_end.into_boxed_slice(),
        occurrence_segments: occurrence_segments.into_boxed_slice(),
        occurrence_offsets: occurrence_offsets.into_boxed_slice(),
        segment_totals: segment_totals.into_boxed_slice(),
        next_controlled: next_controlled.into_boxed_slice(),
        speed_limit_drop: speed_limit_drop.into_boxed_slice(),
        waiting: waiting.into_boxed_slice(),
    })
}

/// 分段 `u32` 前缀。下一条边长会让当前段溢出时封段、开新段。不上 `u64`（ADR 0028）。
fn segmented_route_coordinates(edge_lengths: &[u32]) -> (Vec<u32>, Vec<u32>, Vec<u32>) {
    let mut segments = Vec::with_capacity(edge_lengths.len());
    let mut offsets = Vec::with_capacity(edge_lengths.len());
    let mut totals = Vec::new();
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
    (segments, offsets, totals)
}

fn compile_waiting(
    traffic: &SharedTrafficNetwork,
    hop_gate: &[Option<ManeuverGateOrdinal>],
    maneuvers: &[ManeuverOccurrence],
) -> Result<Vec<WaitingOccurrence>, RouteError> {
    let mut waiting = Vec::new();
    let network = traffic.maneuvers();
    let relations = traffic.relations();
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
            waiting.push(WaitingOccurrence {
                zone: *zone,
                maneuver_index: u32::try_from(maneuver_index).expect("occurrence index fits u32"),
                entry_hop,
                release_hop,
            });
        }
    }
    Ok(waiting)
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
    let mut entry_paths = Vec::new();
    for candidate in candidates {
        if candidate.successor() != to || candidate.transition_index() != 0 {
            continue;
        }
        let path = network
            .maneuver_path(candidate.maneuver_path())
            .ok_or(RouteError::ManeuverMismatch)?;
        entry_paths.push((candidate.maneuver_path(), path.edges()));
    }
    unique_entry_path_match_filtered(remaining, entry_paths)
}

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
        distance = distance.add(*total);
    }
    Some(distance.add(to_off))
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
        let compiled = compile_route(traffic, path.edges()).expect("compile");
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
            expected = expected.add(*lengths.get(path.edges()[index].index()).unwrap());
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
        let (segments, offsets, totals) = segmented_route_coordinates(&lengths);
        assert_eq!(segments.as_slice(), [0, 1]);
        assert_eq!(
            distance_to_occurrence_start(&segments, &offsets, &totals, 0, 100, 1),
            Some(BoundedDistance::Finite(900))
        );
        let remaining = [
            BoundedDistance::Finite(1_000).add(u32::MAX),
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
            compile_route(traffic, &path.edges()[1..]).unwrap_err(),
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
            compile_route(traffic, &[entry]).unwrap_err(),
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
        compile_route(traffic, path.edges()).expect("full path still compiles");
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
            compile_route(traffic, &[internal]).unwrap_err(),
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
            compile_route(traffic, prefix).unwrap_err(),
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
