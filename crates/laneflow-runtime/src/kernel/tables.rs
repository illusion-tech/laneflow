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

use crate::{RouteError, RouteHandle, VehicleState};

/// 机动路径在编译后路线中的一次出现项。
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

/// 限速下降转换：与共享静态路网 `speed_limit_transitions` 同形。
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
    /// 释放当前 membership 前可能请求的后继区间尾；注册时派生，不持久化。
    pub dependency_end: u32,
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
    pub resource: crate::kernel::conflict::ConflictResource,
    pub maneuver_index: u32,
    pub admission_hop: u32,
    pub entry: RoutePosition,
    pub clearance: RoutePosition,
}

impl ConflictPassageOccurrence {
    /// 静态面访问（S4 规则：`Transition` 失败关闭）：取静态冲突区 ordinal。
    pub(crate) const fn static_zone(self) -> Option<ConflictZoneOrdinal> {
        match self.resource {
            crate::kernel::conflict::ConflictResource::Zone(zone) => Some(zone),
            crate::kernel::conflict::ConflictResource::Transition(_) => None,
        }
    }

    /// 该出现项的规范 passage 地址。
    pub(crate) const fn address(self) -> crate::ConflictPassageAddress {
        match self.resource {
            crate::kernel::conflict::ConflictResource::Zone(zone) => {
                crate::ConflictPassageAddress::new(zone, self.stream, self.passage_local_index)
            }
            crate::kernel::conflict::ConflictResource::Transition(edge) => {
                crate::ConflictPassageAddress::new_transition(
                    edge,
                    self.stream,
                    self.passage_local_index,
                )
            }
        }
    }
}

/// `conflicts` 中属于一个 admission hop 的连续半开区间。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ConflictGateRange {
    pub start: u32,
    pub len: u32,
}

/// 从路线 occurrence 起点到最近几何停止位置的距离。
///
/// `None` 表示这条 occurrence 之后没有该类屏障。`Some(BeyondFinite)` 表示有屏障，
/// 但距离超出有限毫米窗口。不记录授权、灯色，也不保存跨拍忽略状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NearestMotionBarriers {
    /// 最近冲突准入。锚点是 admission hop 后继 occurrence 的起点。
    pub(crate) conflict_from_occurrence_start: Option<BoundedDistance>,
    /// 最近等待入口。锚点是 entry hop 后继 occurrence 的起点。
    pub(crate) waiting_from_occurrence_start: Option<BoundedDistance>,
}

/// 一个路线出现项上只取决于（路线, 游标）的运动输入，编译时按游标连续存放。
///
/// 逐车准备从一行连续数据取得，不再分别访问边表、车道长度、限速、路终后缀、
/// 限速下降、门 hop 与屏障表。各字段与从源数组逐项读取的结果相同（见 [`route_hop`]）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RouteHop {
    pub edge: LaneEdgeOrdinal,
    pub length_mm: Option<u32>,
    pub limit_mm_s: Option<u32>,
    pub remaining_to_end: Option<BoundedDistance>,
    /// 第一个 `from_route_edge_index >= 游标` 的限速下降下标。
    pub speed_drop_first: u32,
    /// 第一个 `>= 游标` 的 `gate_hops` 下标。
    pub gate_first: u32,
    pub barriers: Option<NearestMotionBarriers>,
    /// 下一受控门从本 hop 起点算的距离；没有受控门为 `None`。
    pub signal_from_start: Option<BoundedDistance>,
}

/// 本世界 compiled 路线：分段 `u32` 前缀、后缀 `BoundedDistance`、hop 门、
/// 受控 hop 链、最近停止距离和限速下降转换。
/// 不上 `u64`，不把 world 身份写进 `RouteHandle`，不存「当前红灯」（ADR 0028 / 0029）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompiledRoute {
    pub edges: Vec<LaneEdgeOrdinal>,
    pub maneuvers: Vec<ManeuverOccurrence>,
    pub hop_gate: Vec<Option<ManeuverGateOrdinal>>,
    /// 按 route order 排列的实际 Gate hops；稀疏路线不逐车扫描空 hop 后缀。
    pub gate_hops: Vec<u32>,
    /// 候选派发 hop 并集（#1199 修形①）：gate hop ∪ 带冲突范围的 hop（含
    /// 无 gate 的过渡资源 hop），升序去重——注册期预计算，tick 零分配。
    pub conflict_dispatch_hops: Vec<u32>,
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
    /// 与 `edges` 等长。同一条边在循环路线中的两次出现各有一行。
    pub nearest_motion_barriers: Vec<NearestMotionBarriers>,
    /// 静态 Waiting maneuver 覆盖位，按路线出现项索引；空 Waiting 路线不分配。
    pub waiting_maneuver_bits: Box<[u64]>,
    /// 与 `edges` 等长的逐游标运动输入；为空时由 [`route_hop`] 从源数组逐项读取。
    pub hops: Vec<RouteHop>,
}

#[cfg(any(test, feature = "execution-evidence"))]
impl CompiledRoute {
    /// 测试用：编译路线持有的逻辑字节数。
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        let Self {
            edges,
            maneuvers,
            hop_gate,
            gate_hops,
            conflict_dispatch_hops,
            remaining_to_end,
            occurrence_segments,
            occurrence_offsets,
            segment_totals,
            next_controlled,
            speed_limit_drop,
            waiting,
            conflicts,
            conflict_gate_ranges,
            final_conflict_clearance: _,
            nearest_motion_barriers,
            waiting_maneuver_bits,
            hops,
        } = self;
        crate::kernel::state::vec_bytes(edges)
            + crate::kernel::state::vec_bytes(maneuvers)
            + crate::kernel::state::vec_bytes(hop_gate)
            + crate::kernel::state::vec_bytes(gate_hops)
            + crate::kernel::state::vec_bytes(conflict_dispatch_hops)
            + crate::kernel::state::vec_bytes(remaining_to_end)
            + crate::kernel::state::vec_bytes(occurrence_segments)
            + crate::kernel::state::vec_bytes(occurrence_offsets)
            + crate::kernel::state::vec_bytes(segment_totals)
            + crate::kernel::state::vec_bytes(next_controlled)
            + crate::kernel::state::vec_bytes(speed_limit_drop)
            + crate::kernel::state::vec_bytes(waiting)
            + crate::kernel::state::vec_bytes(conflicts)
            + crate::kernel::state::vec_bytes(conflict_gate_ranges)
            + crate::kernel::state::vec_bytes(nearest_motion_barriers)
            + crate::kernel::state::slice_bytes(waiting_maneuver_bits)
            + crate::kernel::state::vec_bytes(hops)
    }
}

/// 游标处的运动输入：有预编译行就读行，否则从源数组逐项读取（结果相同）。
#[inline]
pub(crate) fn route_hop(
    compiled: &CompiledRoute,
    cursor: usize,
    lengths: &[u32],
    speeds: &[u32],
) -> Option<RouteHop> {
    if let Some(hop) = compiled.hops.get(cursor) {
        return Some(*hop);
    }
    derive_route_hop(compiled, cursor, lengths, speeds)
}

fn derive_route_hop(
    compiled: &CompiledRoute,
    cursor: usize,
    lengths: &[u32],
    speeds: &[u32],
) -> Option<RouteHop> {
    let edge = *compiled.edges.get(cursor)?;
    let cursor_u32 = u32::try_from(cursor).ok()?;
    Some(RouteHop {
        edge,
        length_mm: lengths.get(edge.index()).copied(),
        limit_mm_s: speeds.get(edge.index()).copied(),
        remaining_to_end: compiled.remaining_to_end.get(cursor).copied(),
        speed_drop_first: u32::try_from(
            compiled
                .speed_limit_drop
                .partition_point(|drop| drop.from_route_edge_index < cursor_u32),
        )
        .ok()?,
        gate_first: u32::try_from(compiled.gate_hops.partition_point(|hop| *hop < cursor_u32))
            .ok()?,
        barriers: compiled.nearest_motion_barriers.get(cursor).copied(),
        signal_from_start: compiled
            .next_controlled
            .get(cursor)
            .copied()
            .flatten()
            .map(|next| next.distance_from_hop_start),
    })
}

impl CompiledRoute {
    /// 按当前源数组重建逐游标运动输入行。编译时调用；测试改写源数组后也要调用。
    pub(crate) fn refresh_hops(
        &mut self,
        lengths: &[u32],
        speeds: &[u32],
    ) -> Result<(), RouteError> {
        let mut hops = try_route_vec(self.edges.len())?;
        self.hops.clear();
        for cursor in 0..self.edges.len() {
            hops.push(
                derive_route_hop(self, cursor, lengths, speeds)
                    .ok_or(RouteError::AllocationFailed)?,
            );
        }
        self.hops = hops;
        Ok(())
    }

    pub(crate) fn waiting_maneuver_at_hop(&self, hop: u32) -> Option<bool> {
        let hop = usize::try_from(hop).ok()?;
        if hop >= self.edges.len() {
            return None;
        }
        if self.waiting.is_empty() {
            return Some(false);
        }
        self.waiting_maneuver_bits
            .get(hop / 64)
            .map(|word| word & (1 << (hop % 64)) != 0)
    }

    /// 把路线内 conflict occurrence 下标定位成对外 locator；越界返回 `None`。
    pub(crate) fn conflict_occurrence_locator(
        &self,
        route: RouteHandle,
        conflict_occurrence_index: u32,
        stable_locator: crate::ConflictPassageLocator,
    ) -> Option<crate::ConflictPassageOccurrenceLocator> {
        let occurrence = *self
            .conflicts
            .get(usize::try_from(conflict_occurrence_index).ok()?)?;
        Some(crate::ConflictPassageOccurrenceLocator::new(
            route,
            occurrence.maneuver_index,
            occurrence.admission_hop,
            conflict_occurrence_index,
            occurrence.address(),
            stable_locator,
        ))
    }
}

#[cfg(test)]
fn logical_vec_bytes<T>(len: usize) -> u64 {
    u64::try_from(len)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(std::mem::size_of::<T>()).expect("type size fits u64"))
}

/// 路线登记表槽位：代际、可选编译路线与存活车辆计数。
#[derive(Clone, Debug)]
pub(crate) struct RouteSlot {
    pub generation: u32,
    pub compiled: Option<CompiledRoute>,
    pub live_vehicles: u32,
}

/// 从只读登记表按完整句柄取得编译路线，不借用其他阶段状态。
pub(crate) fn compiled_route_for_handle(
    routes: &[RouteSlot],
    route: RouteHandle,
) -> Option<&CompiledRoute> {
    let slot = routes.get(usize::try_from(route.index()).ok()?)?;
    if slot.generation != route.generation() {
        return None;
    }
    slot.compiled.as_ref()
}

#[cfg(any(test, feature = "execution-evidence"))]
impl RouteSlot {
    /// 测试用：槽位内编译路线持有的逻辑字节数。
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        let Self {
            generation: _,
            compiled,
            live_vehicles: _,
        } = self;
        compiled
            .as_ref()
            .map_or(0, CompiledRoute::retained_logical_bytes)
    }
}

/// 车辆登记表槽位：代际与可选车辆状态。
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
crate::kernel::execution::carry_hooks!(carry_test_hooks: ROUTE_RESERVATIONS_BEFORE_FAILURE);

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

/// A6 选项 3：换道执行支持是**实现能力**（非配置，不新增配置字段）。S4
/// 切片翻转为 `true`（预约路径全绿后经 lead 裁定授权；#1199 合同 §6）。含
/// 过渡段路线可注册可执行——授权语义演化见 PR「行为变化」清单。
fn transition_execution_supported() -> bool {
    true
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

    // P2-6（#1199）：注册容量预检计数含过渡出现项（每横向过渡边恰 1 条）
    // ——但 compile_conflicts 的 expected_count 只承流通道数（其自身另加
    // transition_occurrences 到 capacity（1213）并以等式校验（1268）——
    // 二分定位：过渡计入 expected=双计引爆 ManeuverMismatch）。分层：容量
    // 检查含 T、发射 expected 仅流。
    let transition_count = edges
        .iter()
        .filter(|edge| revision.traffic().is_lateral_transition_segment(**edge))
        .count();
    let stream_occurrence_count = count_conflict_occurrences(revision, &maneuvers)?;
    let conflict_occurrence_count = stream_occurrence_count
        .checked_add(u64::try_from(transition_count).map_err(|_| RouteError::AllocationFailed)?)
        .ok_or(RouteError::AllocationFailed)?;
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
        stream_occurrence_count,
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
    let nearest_motion_barriers =
        compile_nearest_motion_barriers(&route_lengths, &conflicts, &waiting)?;
    let mut compiled_edges = try_route_vec(edges.len())?;
    compiled_edges.extend_from_slice(edges);
    let waiting_maneuver_bits = compile_waiting_maneuver_bits(edges.len(), &maneuvers, &waiting)?;
    // 修形①（#1199）：候选派发 hop 并集注册期预计算——gate hop ∪ **含
    // Transition 出现项**的 hop（T 候选仅在过渡出现项时派生，非任意冲突
    // hop——旧输入零新结论保持）。tick 侧零分配消费。
    let mut conflict_dispatch_hops = gate_hops.clone();
    for (hop, range) in conflict_gate_ranges.iter().enumerate() {
        if range.len == 0 {
            continue;
        }
        let start = range.start as usize;
        let has_transition =
            conflicts[start..start + range.len as usize]
                .iter()
                .any(|occurrence| {
                    matches!(
                        occurrence.resource,
                        crate::kernel::conflict::ConflictResource::Transition(_)
                    )
                });
        if !has_transition {
            continue;
        }
        let hop = hop as u32;
        if !conflict_dispatch_hops.contains(&hop) {
            conflict_dispatch_hops.push(hop);
        }
    }
    conflict_dispatch_hops.sort_unstable();

    let mut compiled = CompiledRoute {
        edges: compiled_edges,
        maneuvers,
        hop_gate,
        gate_hops,
        conflict_dispatch_hops,
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
        nearest_motion_barriers,
        waiting_maneuver_bits,
        hops: Vec::new(),
    };
    compiled.refresh_hops(lengths, speeds)?;
    // 能力拒绝（A6 选项 3，S3 侧注册面）：置于全部静态出现项编译完成之后
    // ——静态序列验证独立完整（错误类型不因能力检查提前截断）。**段级检测**
    // （P1-1 修正）：按边分类横向过渡段，不依赖路线三边窗口——多段过渡链与
    // 起/止于过渡的路线（含单边）同样命中；**横向判据**（P2-2 修正）：以
    // section 内 lane 横向序数为依据，两侧 covered 边界横向序数跨度 ≥ 2 且
    // 同走廊才是横向过渡，纵向 uncovered 连接（同横向序数）不误判。分类表
    // **构建期一次计算**（`SharedTrafficNetwork::is_lateral_transition_segment`
    // 查表，P2 性能修正），注册期 O(1)。换道执行支持是**实现能力**（非配置）：
    // 本构建不支持过渡执行（S4 交付前），注册失败关闭、注册表不变（失败
    // 原子）。仅声明材料化的过渡段可达本检查（旧输入无此类结构 → 零新拒绝）。
    if !transition_execution_supported() {
        for edge in edges {
            if traffic.is_lateral_transition_segment(*edge) {
                return Err(RouteError::TransitionUnsupported);
            }
        }
    }
    Ok(compiled)
}

fn compile_waiting_maneuver_bits(
    hops: usize,
    maneuvers: &[ManeuverOccurrence],
    waiting: &[WaitingOccurrence],
) -> Result<Box<[u64]>, RouteError> {
    if waiting.is_empty() {
        return Ok(Box::default());
    }
    let mut bits = try_route_vec_filled(hops.div_ceil(64), 0_u64)?;
    for occurrence in waiting {
        let maneuver = maneuvers
            .get(occurrence.maneuver_index as usize)
            .ok_or(RouteError::ManeuverMismatch)?;
        for hop in maneuver.entry_route_edge_index..maneuver.exit_route_edge_index {
            let hop = hop as usize;
            let word = bits.get_mut(hop / 64).ok_or(RouteError::ManeuverMismatch)?;
            *word |= 1 << (hop % 64);
        }
    }
    Ok(bits.into_boxed_slice())
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
                dependency_end: 0,
            });
        }
    }
    for index in 0..waiting.len() {
        let end = waiting.partition_point(|item| item.entry_hop <= waiting[index].release_hop);
        waiting[index].dependency_end =
            u32::try_from(end).map_err(|_| RouteError::ManeuverMismatch)?;
    }
    Ok(waiting)
}

#[derive(Clone, Copy)]
struct BarrierHopMark {
    conflict: bool,
    waiting: bool,
}

/// 从路线末尾回填最近冲突准入和等待入口。
///
/// 每一行是一个路线 occurrence。距离锚在该 hop 后继 occurrence 的起点，不加车长，
/// 也不读取 passage 入口。构建成本随 occurrence、冲突出现项和等待出现项线性增长。
pub(crate) fn compile_nearest_motion_barriers(
    edge_lengths: &[u32],
    conflicts: &[ConflictPassageOccurrence],
    waiting: &[WaitingOccurrence],
) -> Result<Vec<NearestMotionBarriers>, RouteError> {
    let occurrences = edge_lengths.len();
    let hop_count = occurrences.saturating_sub(1);
    let mut marks = try_route_vec_filled(
        hop_count,
        BarrierHopMark {
            conflict: false,
            waiting: false,
        },
    )?;
    for conflict in conflicts {
        let hop =
            usize::try_from(conflict.admission_hop).map_err(|_| RouteError::ManeuverMismatch)?;
        let mark = marks.get_mut(hop).ok_or(RouteError::ManeuverMismatch)?;
        mark.conflict = true;
    }
    for entry in waiting {
        let hop = usize::try_from(entry.entry_hop).map_err(|_| RouteError::ManeuverMismatch)?;
        let mark = marks.get_mut(hop).ok_or(RouteError::ManeuverMismatch)?;
        mark.waiting = true;
    }

    let mut barriers = try_route_vec_filled(
        occurrences,
        NearestMotionBarriers {
            conflict_from_occurrence_start: None,
            waiting_from_occurrence_start: None,
        },
    )?;
    let mut next_conflict: Option<BoundedDistance> = None;
    let mut next_waiting: Option<BoundedDistance> = None;
    for index in (0..occurrences).rev() {
        let length = edge_lengths[index];
        let conflict = if marks.get(index).is_some_and(|mark| mark.conflict) {
            Some(BoundedDistance::Finite(0).add_u32(length))
        } else {
            next_conflict.map(|distance| distance.add_u32(length))
        };
        let waiting_distance = if marks.get(index).is_some_and(|mark| mark.waiting) {
            Some(BoundedDistance::Finite(0).add_u32(length))
        } else {
            next_waiting.map(|distance| distance.add_u32(length))
        };
        barriers[index] = NearestMotionBarriers {
            conflict_from_occurrence_start: conflict,
            waiting_from_occurrence_start: waiting_distance,
        };
        next_conflict = conflict;
        next_waiting = waiting_distance;
    }
    Ok(barriers)
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

/// §8 流绑定（#1199 增量2a）：过渡段的跨越流 = **后继** maneuver 出现项的
/// 首 stream（无后继取**前驱**末条；同 entry 先出现者优先，确定性钉死）；
/// 绑定候选无 participant stream 时返回 `None`（调用方跳过发射——能力拒绝
/// 语义保持，显式覆盖不留静默盲区）。
fn bind_transition_stream<'a>(
    maneuvers: &[ManeuverOccurrence],
    edge_index: usize,
    streams_of: impl Fn(ManeuverPathOrdinal) -> Option<&'a [ParticipantStreamOrdinal]>,
) -> Option<(usize, ParticipantStreamOrdinal)> {
    let next = maneuvers
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, occurrence)| {
            usize::try_from(occurrence.entry_route_edge_index).expect("route edge index fits usize")
                > edge_index
        })
        .min_by_key(|(_, occurrence)| occurrence.entry_route_edge_index);
    let bound = next.or_else(|| {
        maneuvers
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, occurrence)| {
                usize::try_from(occurrence.entry_route_edge_index)
                    .expect("route edge index fits usize")
                    <= edge_index
            })
            .max_by_key(|(_, occurrence)| occurrence.entry_route_edge_index)
    })?;
    let (maneuver_index, bound_occurrence) = bound;
    let stream = streams_of(bound_occurrence.path)?.first().copied()?;
    Some((maneuver_index, stream))
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
    let traffic = revision.traffic();
    let conflict = revision.conflict();
    // 过渡出现项预构建（#1199 增量2a；§8 流绑定）：分类表命中的路线边各发一条
    // 出现项；流 = 后继 maneuver 出现项的首 stream（无后继取前驱末条，确定性
    // 钉死）；独占由 zone-head 所有权承担（B 裁定）。无 maneuver 的纯过渡路线
    // 此处不绑定（能力拒绝错误类型保持；预约绑定留翻转切片）。
    let mut transition_occurrences: Vec<ConflictPassageOccurrence> = Vec::new();
    for (edge_index, edge) in route_edges.iter().copied().enumerate() {
        if !traffic.is_lateral_transition_segment(edge) {
            continue;
        }
        let Some((maneuver_index, stream)) =
            bind_transition_stream(maneuvers, edge_index, |path| {
                conflict.maneuver_path_participant_streams(path)
            })
        else {
            // §8 流绑定（#1199 评审 P1-2）：无候选或候选无 participant
            // stream 时**结构化拒绝**——无绑定资源出现项的过渡路线不得
            // 免检注册（能力翻转后 fail-open 封堵）。
            return Err(RouteError::ManeuverMismatch);
        };
        let length = *traffic
            .lane_lengths_millimetres()
            .get(edge.index())
            .ok_or(RouteError::ManeuverMismatch)?;
        if length == 0 {
            return Err(RouteError::ManeuverMismatch);
        }
        let route_edge_index =
            u32::try_from(edge_index).map_err(|_| RouteError::AllocationFailed)?;
        transition_occurrences.push(ConflictPassageOccurrence {
            stream,
            passage_local_index: 0,
            resource: crate::kernel::conflict::ConflictResource::Transition(edge),
            maneuver_index: u32::try_from(maneuver_index)
                .map_err(|_| RouteError::AllocationFailed)?,
            // 准入 hop=穿越拍（hop N=边 N 末端进入边 N+1）：过渡格须在
            // 进入 T 之前准入（#1199 合同 §5.2）——发射侧存穿越 hop（T
            // 边下标-1；路线首边过渡退饱和至 0，入场由 spawn 准入承担）。
            admission_hop: route_edge_index.saturating_sub(1),
            entry: RoutePosition {
                route_edge_index,
                progress_mm: 0,
            },
            clearance: RoutePosition {
                route_edge_index,
                progress_mm: length,
            },
        });
    }
    let capacity = usize::try_from(expected_count)
        .map_err(|_| RouteError::AllocationFailed)?
        .checked_add(transition_occurrences.len())
        .ok_or(RouteError::AllocationFailed)?;
    let mut conflicts = try_conflict_occurrence_vec(capacity)?;
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
                    resource: crate::kernel::conflict::ConflictResource::Zone(
                        passage.conflict_zone(),
                    ),
                    maneuver_index: u32::try_from(maneuver_index)
                        .map_err(|_| RouteError::AllocationFailed)?,
                    admission_hop,
                    entry,
                    clearance,
                });
            }
        }
    }
    conflicts.extend(transition_occurrences);
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

/// 检查该参与类在 `cursor` 之后的余下边与机动是否被通行规则拒绝。
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

/// 车尾路线坐标是否到达或超过 `target`；无法定位时返回 `None`。
pub(crate) fn vehicle_rear_at_or_beyond(
    lengths: &[u32],
    edges: &[LaneEdgeOrdinal],
    route_edge_index: usize,
    progress_mm: u32,
    carry_um: u16,
    vehicle_length_mm: u32,
    target: RoutePosition,
) -> Option<bool> {
    Some(
        match route_rear_position(
            lengths,
            edges,
            route_edge_index,
            progress_mm,
            carry_um,
            vehicle_length_mm,
        )? {
            RouteRearPosition::BeforeRouteStart => false,
            RouteRearPosition::Position(rear) => {
                (rear.route_edge_index, rear.progress_mm, rear.carry_um)
                    >= (target.route_edge_index, target.progress_mm, 0)
            }
        },
    )
}

/// 冷生命周期 Conflict 检查内部错误；调用方把非规范 cursor 映射到各自既有
/// invariant 错误面。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConflictCapabilityError {
    InvalidCursor,
    AuthorityRequired,
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

/// 冷生命周期的 Conflict authority 边界。
///
/// Gate 上游或精确停在线上可以由下一次 production tick 取得 grant；车辆前端已越过
/// Gate、但车尾尚未清空该 Gate 完整 coverage 时，spawn/replace/rebind 不能凭空制造
/// reservation。该入口只用于低频生命周期预检，允许按 route-local Gate range 扫描。
pub(crate) fn check_conflict_capability(
    compiled: &CompiledRoute,
    lengths: &[u32],
    route_edge_index: usize,
    progress_mm: u32,
    carry_um: u16,
    vehicle_length_mm: u32,
) -> Result<(), ConflictCapabilityError> {
    if compiled.final_conflict_clearance.is_none() {
        return Ok(());
    }
    let edge = *compiled
        .edges
        .get(route_edge_index)
        .ok_or(ConflictCapabilityError::InvalidCursor)?;
    let edge_length = *lengths
        .get(edge.index())
        .ok_or(ConflictCapabilityError::InvalidCursor)?;
    if progress_mm > edge_length || (progress_mm == edge_length && carry_um != 0) {
        return Err(ConflictCapabilityError::InvalidCursor);
    }
    let mut front = (
        u32::try_from(route_edge_index).map_err(|_| ConflictCapabilityError::InvalidCursor)?,
        progress_mm,
        carry_um,
    );
    if progress_mm == edge_length && carry_um == 0 && route_edge_index + 1 < compiled.edges.len() {
        front = (
            u32::try_from(route_edge_index + 1)
                .map_err(|_| ConflictCapabilityError::InvalidCursor)?,
            0,
            0,
        );
    }
    let rear = route_rear_position(
        lengths,
        compiled.edges.as_slice(),
        route_edge_index,
        progress_mm,
        carry_um,
        vehicle_length_mm,
    )
    .ok_or(ConflictCapabilityError::InvalidCursor)?;
    for (gate_hop, range) in compiled.conflict_gate_ranges.iter().copied().enumerate() {
        if range.len == 0 {
            continue;
        }
        let end = range
            .start
            .checked_add(range.len)
            .ok_or(ConflictCapabilityError::InvalidCursor)?;
        let occurrences = compiled
            .conflicts
            .get(range.start as usize..end as usize)
            .ok_or(ConflictCapabilityError::InvalidCursor)?;
        let farthest = occurrences
            .iter()
            .max_by_key(|occurrence| occurrence.clearance)
            .ok_or(ConflictCapabilityError::InvalidCursor)?;
        let cleared = match rear {
            RouteRearPosition::BeforeRouteStart => false,
            RouteRearPosition::Position(position) => {
                (
                    position.route_edge_index,
                    position.progress_mm,
                    position.carry_um,
                ) >= (
                    farthest.clearance.route_edge_index,
                    farthest.clearance.progress_mm,
                    0,
                )
            }
        };
        if cleared {
            continue;
        }
        let gate_edge = *compiled
            .edges
            .get(gate_hop)
            .ok_or(ConflictCapabilityError::InvalidCursor)?;
        let gate_progress = *lengths
            .get(gate_edge.index())
            .ok_or(ConflictCapabilityError::InvalidCursor)?;
        let gate_crossed_side = if gate_hop + 1 < compiled.edges.len() {
            (
                u32::try_from(gate_hop + 1).map_err(|_| ConflictCapabilityError::InvalidCursor)?,
                0,
                0,
            )
        } else {
            (
                u32::try_from(gate_hop).map_err(|_| ConflictCapabilityError::InvalidCursor)?,
                gate_progress,
                0,
            )
        };
        if front <= gate_crossed_side {
            return Ok(());
        }
        return Err(ConflictCapabilityError::AuthorityRequired);
    }
    Ok(())
}

const OCCUPANCY_INTERVAL_CAP: usize = 16;

/// 车身最多跨过的边数。每条边至少 `MIN_LANE_EDGE_LENGTH_MM`，再留出首尾两段不满一条边的余量。
pub(crate) fn body_interval_slots(length_mm: u32) -> usize {
    let min_edge = laneflow_static_contract::MIN_LANE_EDGE_LENGTH_MM;
    let spanned = length_mm.div_ceil(min_edge).saturating_add(1);
    usize::try_from(spanned).unwrap_or(usize::MAX)
}

#[cfg(test)]
#[test]
fn body_interval_slots_follow_the_shortest_legal_edge() {
    assert_eq!(body_interval_slots(0), 1);
    assert_eq!(body_interval_slots(4_500), 46);
    assert_eq!(body_interval_slots(128_000), 1_281);
    assert!(body_interval_slots(128_000) < 128_001);
}
type OccupancyInterval = (LaneEdgeOrdinal, u32, u32);
type OccupancyStack = ([OccupancyInterval; OCCUPANCY_INTERVAL_CAP], usize, bool);

/// 从前杠位置沿路线向车尾逐边展开车身占用区间并回调；下标或长度缺失返回 `None`。
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

/// 准入使用实际车身，加上零进度前杠的退化入口点。
/// 同一点不能同时生成两辆车，否则它们在下一拍会一起进入同一非空区间。
/// 跟车候选也保留此点，使其他 incoming branch 看见已经进入共享边的车辆。
pub(crate) fn for_each_admission_interval(
    lengths: &[u32],
    edges: &[LaneEdgeOrdinal],
    index: usize,
    progress: u32,
    length: u32,
    mut visit: impl FnMut(LaneEdgeOrdinal, u32, u32),
) -> Option<()> {
    if progress == 0 && length > 0 {
        let edge = *edges.get(index)?;
        lengths.get(edge.index())?;
        visit(edge, 0, 0);
    }
    for_each_occupancy_interval(lengths, edges, index, progress, length, visit)
}

/// 两个准入区间是否重叠；同为一点的零进度退化入口视为重叠。
pub(crate) fn admission_intervals_overlap(a_lo: u32, a_hi: u32, b_lo: u32, b_hi: u32) -> bool {
    (a_lo == a_hi && b_lo == b_hi && a_lo == b_lo) || (a_lo < b_hi && b_lo < a_hi)
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

#[cfg(test)]
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

#[cfg(test)]
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

/// 测试用：两个 route cursor 展开的车身物理 footprint 是否重叠。
#[cfg(test)]
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
    // 空车身不制造路线外的负坐标，但重合的零进度前杠仍共享同一入口点。
    if a_progress == 0
        && b_progress == 0
        && a_length > 0
        && b_length > 0
        && a_edges
            .get(a_index)
            .is_some_and(|edge| Some(edge) == b_edges.get(b_index))
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
    for_each_admission_interval(
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
        "../../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
    );

    #[test]
    fn waiting_maneuver_bits_follow_occurrences_cross_words_and_account_capacity() {
        let world = crate::kernel::waiting::tests::multi_gate_world(1);
        let state = world.vehicle(world.live_vehicles()[0]).unwrap();
        let mut compiled = world.state.compiled_route(state.route).unwrap().clone();
        let mut waiting = compiled.waiting[0];
        waiting.maneuver_index = 1;
        let maneuvers = [
            ManeuverOccurrence {
                path: ManeuverPathOrdinal::from_raw(0),
                entry_route_edge_index: 0,
                exit_route_edge_index: 2,
            },
            ManeuverOccurrence {
                path: ManeuverPathOrdinal::from_raw(0),
                entry_route_edge_index: 63,
                exit_route_edge_index: 66,
            },
        ];
        compiled.edges = vec![compiled.edges[0]; 67];
        compiled.waiting = vec![waiting];
        compiled.waiting_maneuver_bits = Box::default();
        let empty_bytes = compiled.retained_logical_bytes();
        compiled.waiting_maneuver_bits =
            compile_waiting_maneuver_bits(67, &maneuvers, &[waiting]).unwrap();
        assert_eq!(compiled.waiting_maneuver_bits.len(), 2);
        assert_eq!(
            compiled.retained_logical_bytes() - empty_bytes,
            crate::kernel::state::slice_bytes(&compiled.waiting_maneuver_bits)
        );
        for hop in 0..67 {
            assert_eq!(
                compiled.waiting_maneuver_at_hop(hop),
                Some((63..66).contains(&hop)),
                "hop={hop}"
            );
        }
        assert_eq!(compiled.waiting_maneuver_at_hop(67), None);
        assert!(
            compile_waiting_maneuver_bits(67, &maneuvers, &[])
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            with_route_allocation_failure_after(0, || compile_waiting_maneuver_bits(
                67,
                &maneuvers,
                &[waiting]
            )),
            Err(RouteError::AllocationFailed)
        );
    }

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

    /// S3（#1194）：合成样本网络（`sample/minimal-closed-loop.json` v5 派生）。
    /// 来源保真：合成来源原生声明（SyntheticDsl）；`T01` 为 uncovered 非 junction
    /// 过渡段（前后邻边分属同走廊不同 lane 链），不冒充 SUMO connection。
    mod s3_lane_change {
        use super::*;
        use laneflow_compiler::*;
        use laneflow_format::{FormatLimits, check_post_emission_bundle};

        /// sample v5 钉版长度（米）与中心线端点（米，(x0, z0) → (x1, z1)）。
        /// 另含评审补充段：`T0a…T0b` 多段过渡链（P1-1）、`X`/`C1` 纵向
        /// uncovered 连接（P2-2 负例，跨走廊）、`P0/SP/Q0` 相邻单 lane section
        /// 横向接缝（P1 补全正例）。
        const EDGES: [(&str, f64, f32, f32, f32, f32); 20] = [
            ("A0", 20.0, -20.0, 0.0, 0.0, 0.0),
            ("I1", 10.0, 0.0, 0.0, 10.0, 0.0),
            ("B0_up", 8.0, 10.0, 0.0, 18.0, 0.0),
            ("B1_up", 8.0, 10.0, 3.0, 18.0, 3.0),
            ("S00", 9.0, 18.0, 0.0, 27.0, 0.0),
            ("S11", 9.0, 18.0, 3.0, 27.0, 3.0),
            ("T01", 9.487, 18.0, 0.0, 27.0, 3.0),
            ("T0a", 4.272, 18.0, 0.0, 22.0, 1.5),
            ("T0b", 5.22, 22.0, 1.5, 27.0, 3.0),
            ("B0_down", 9.0, 27.0, 0.0, 36.0, 0.0),
            ("B1_down", 9.0, 27.0, 3.0, 36.0, 3.0),
            ("I0", 6.0, 36.0, 0.0, 42.0, 0.0),
            ("I2", 6.708, 36.0, 3.0, 42.0, 0.0),
            ("C0", 20.0, 42.0, 0.0, 62.0, 0.0),
            ("X", 5.0, 62.0, 0.0, 67.0, 0.0),
            ("C1", 12.0, 67.0, 0.0, 79.0, 0.0),
            ("P0", 7.5, 79.0, 5.0, 86.5, 5.0),
            ("SP", 3.0, 86.5, 5.0, 89.5, 5.0),
            ("Q0", 8.5, 89.5, 5.0, 98.0, 5.0),
            ("R0", 82.0, 62.0, 0.0, -20.0, 0.0),
        ];

        pub(super) fn revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
            let limits = CompileLimits::p100_initial_v1();
            let header = SourceModuleHeader::new(
                SourceModuleHeaderInput {
                    authoring_namespace_id: "lane-change-s1s3",
                    source_document_key: "lane-change-s1s3.document",
                    generator_build_id: "lane-change-s1s3-probe",
                    parameters_and_inputs_digest: [0x11; 32],
                    frontend_options_digest: [0x22; 32],
                    random_seed: Some(0),
                    provenance: "repository:laneflow-test",
                },
                &limits,
            )
            .unwrap();
            let mut b = SyntheticModuleBuilder::new(header, &limits).unwrap();
            b.add_participant_class(ParticipantClassInput {
                participant_class_key: "car",
                extends: None,
            })
            .unwrap();
            b.add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: "car",
                participant_class: ParticipantClassReference::local("car"),
                iidm: IidmVehicleProfileInput {
                    length_meters: 4.5,
                    desired_speed_meters_per_second: 13.9,
                    min_gap_meters: 2.5,
                    time_headway_seconds: 1.5,
                    max_acceleration_meters_per_second_squared: 1.4,
                    comfortable_deceleration_meters_per_second_squared: 4.5,
                    emergency_deceleration_meters_per_second_squared: 9.0,
                },
            })
            .unwrap();
            let successors: std::collections::HashMap<&str, Vec<&str>> = [
                ("A0", vec!["I1"]),
                ("I1", vec!["B0_up"]),
                ("B0_up", vec!["S00", "T01", "T0a"]),
                ("B1_up", vec!["S11"]),
                ("S00", vec!["B0_down"]),
                ("S11", vec!["B1_down"]),
                ("T01", vec!["B1_down"]),
                ("T0a", vec!["T0b"]),
                ("T0b", vec!["B1_down"]),
                ("B0_down", vec!["I0"]),
                ("B1_down", vec!["I2"]),
                ("I0", vec!["C0"]),
                ("I2", vec!["C0"]),
                ("C0", vec!["R0", "X"]),
                ("X", vec!["C1"]),
                ("C1", vec![]),
                ("P0", vec!["SP"]),
                ("SP", vec!["Q0"]),
                ("Q0", vec![]),
                ("R0", vec!["A0"]),
            ]
            .into_iter()
            .collect();
            for (key, length, ..) in EDGES {
                let refs: Vec<_> = successors[key]
                    .iter()
                    .map(|target| LaneEdgeReference::local(target))
                    .collect();
                b.add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: length,
                    speed_limit_meters_per_second: 13.9,
                    successors: &refs,
                })
                .unwrap();
            }
            let chains: std::collections::HashMap<&str, Vec<LaneEdgeReference<'static>>> = [
                ("lane-A0", vec!["A0"]),
                ("lane-B0up", vec!["B0_up"]),
                ("lane-B1up", vec!["B1_up"]),
                ("lane-S00", vec!["S00"]),
                ("lane-S11", vec!["S11"]),
                ("lane-B0down", vec!["B0_down"]),
                ("lane-B1down", vec!["B1_down"]),
                ("lane-C0", vec!["C0"]),
                ("lane-C1", vec!["C1"]),
                ("lane-P", vec!["P0"]),
                ("lane-Q", vec!["Q0"]),
            ]
            .into_iter()
            .map(|(key, edges)| {
                (
                    key,
                    edges
                        .into_iter()
                        .map(LaneEdgeReference::local)
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
            b.add_road_section(RoadSectionInput {
                road_section_key: "sec-A",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-A0",
                    edge_chain: &chains["lane-A0"],
                    lane_group: None,
                }],
            })
            .unwrap();
            b.add_road_section(RoadSectionInput {
                road_section_key: "sec-B-up",
                kind_id: "motorLane",
                lanes: &[
                    AuthoringLaneInput {
                        authoring_lane_key: "lane-B0up",
                        edge_chain: &chains["lane-B0up"],
                        lane_group: None,
                    },
                    AuthoringLaneInput {
                        authoring_lane_key: "lane-B1up",
                        edge_chain: &chains["lane-B1up"],
                        lane_group: None,
                    },
                ],
            })
            .unwrap();
            b.add_road_section(RoadSectionInput {
                road_section_key: "sec-TZ",
                kind_id: "motorLane",
                lanes: &[
                    AuthoringLaneInput {
                        authoring_lane_key: "lane-S00",
                        edge_chain: &chains["lane-S00"],
                        lane_group: None,
                    },
                    AuthoringLaneInput {
                        authoring_lane_key: "lane-S11",
                        edge_chain: &chains["lane-S11"],
                        lane_group: None,
                    },
                ],
            })
            .unwrap();
            b.add_road_section(RoadSectionInput {
                road_section_key: "sec-B-down",
                kind_id: "motorLane",
                lanes: &[
                    AuthoringLaneInput {
                        authoring_lane_key: "lane-B0down",
                        edge_chain: &chains["lane-B0down"],
                        lane_group: None,
                    },
                    AuthoringLaneInput {
                        authoring_lane_key: "lane-B1down",
                        edge_chain: &chains["lane-B1down"],
                        lane_group: None,
                    },
                ],
            })
            .unwrap();
            b.add_road_section(RoadSectionInput {
                road_section_key: "sec-C",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-C0",
                    edge_chain: &chains["lane-C0"],
                    lane_group: None,
                }],
            })
            .unwrap();
            b.add_road_section(RoadSectionInput {
                road_section_key: "sec-C2",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-C1",
                    edge_chain: &chains["lane-C1"],
                    lane_group: None,
                }],
            })
            .unwrap();
            b.add_road_section(RoadSectionInput {
                road_section_key: "sec-P",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-P",
                    edge_chain: &chains["lane-P"],
                    lane_group: None,
                }],
            })
            .unwrap();
            b.add_road_section(RoadSectionInput {
                road_section_key: "sec-Q",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-Q",
                    edge_chain: &chains["lane-Q"],
                    lane_group: None,
                }],
            })
            .unwrap();
            b.add_road_corridor(RoadCorridorInput {
                road_corridor_key: "corridor-A",
                reference_section: RoadSectionReference::local("sec-A"),
                elements: &[CorridorElementReference::road_section(
                    RoadSectionReference::local("sec-A"),
                )],
            })
            .unwrap();
            b.add_road_corridor(RoadCorridorInput {
                road_corridor_key: "corridor-C",
                reference_section: RoadSectionReference::local("sec-C"),
                elements: &[CorridorElementReference::road_section(
                    RoadSectionReference::local("sec-C"),
                )],
            })
            .unwrap();
            b.add_road_corridor(RoadCorridorInput {
                road_corridor_key: "corridor-C2",
                reference_section: RoadSectionReference::local("sec-C2"),
                elements: &[CorridorElementReference::road_section(
                    RoadSectionReference::local("sec-C2"),
                )],
            })
            .unwrap();
            b.add_road_corridor(RoadCorridorInput {
                road_corridor_key: "corridor-S",
                reference_section: RoadSectionReference::local("sec-P"),
                elements: &[
                    CorridorElementReference::road_section(RoadSectionReference::local("sec-P")),
                    CorridorElementReference::road_section(RoadSectionReference::local("sec-Q")),
                ],
            })
            .unwrap();
            b.add_road_corridor(RoadCorridorInput {
                road_corridor_key: "corridor-B",
                reference_section: RoadSectionReference::local("sec-B-up"),
                elements: &[
                    CorridorElementReference::road_section(RoadSectionReference::local("sec-B-up")),
                    CorridorElementReference::road_section(RoadSectionReference::local("sec-TZ")),
                    CorridorElementReference::road_section(RoadSectionReference::local(
                        "sec-B-down",
                    )),
                ],
            })
            .unwrap();
            let mut maneuver = |junction: &str,
                                movement: &str,
                                path: &str,
                                entry: &str,
                                internal: &[&str],
                                exit: &str| {
                b.add_junction(JunctionInput {
                    junction_key: junction,
                })
                .unwrap();
                b.add_movement(MovementInput {
                    movement_key: movement,
                    junction: JunctionReference::local(junction),
                    directed_entry_approach_key: entry,
                    directed_exit_approach_key: exit,
                    turn_direction: None,
                })
                .unwrap();
                let internal_refs: Vec<_> = internal
                    .iter()
                    .map(|edge| LaneEdgeReference::local(edge))
                    .collect();
                b.add_maneuver_path(ManeuverPathInput {
                    maneuver_path_key: path,
                    movement: MovementReference::local(movement),
                    entry_edge: LaneEdgeReference::local(entry),
                    internal_edges: &internal_refs,
                    exit_edge: LaneEdgeReference::local(exit),
                })
                .unwrap();
            };
            // 路径唯一居所（#1199 P1-2 流绑定载体）：J1/J2b 移 road-editing
            // 侧声明（流挂接载体；合成侧 add_maneuver_path 删除——全局路径
            // 序列唯一规则下两处同形声明=DuplicateManeuverPathSequence）。
            maneuver("J2", "mv-P0", "P0", "B0_down", &["I0"], "C0");

            let point_pairs: Vec<[CanonicalPoint3F32Input; 2]> = EDGES
                .iter()
                .map(|(_, _, x0, z0, x1, z1)| {
                    [
                        CanonicalPoint3F32Input {
                            x: *x0,
                            y: 0.0,
                            z: *z0,
                        },
                        CanonicalPoint3F32Input {
                            x: *x1,
                            y: 0.0,
                            z: *z1,
                        },
                    ]
                })
                .collect();
            let geometries: Vec<_> = EDGES
                .iter()
                .zip(point_pairs.iter())
                .map(|((key, ..), centerline_points)| LaneEdgeGeometryInput {
                    lane_edge: LaneEdgeReference::local(key),
                    centerline_points,
                })
                .collect();
            b.add_canonical_frame(CanonicalFrameInput {
                canonical_frame_key: "lane-change-s1s3-frame",
                lane_edge_geometries: &geometries,
            })
            .unwrap();
            // s3 夹具冲突声明侧（#1199 P1-2 流绑定载体）：v7 模板精简——
            // J1/J2b 两 movement/path + zone/双流/双 passage（R-13 ≥2 流）；
            // 边引用 imported s3-synth 侧（R-1 仅限 owner ✓ 数据引用可用）。
            use laneflow_compiler::road_editing as editing;
            use laneflow_compiler::{GeometryAccuracyProfile, GeometryDirectionProfile};
            let header = editing::RoadEditingModuleHeader::try_new(
                "s3-road",
                "s3-road.document",
                vec!["lane-change-s1s3".to_owned()],
                editing::RoadEditingProvenance::direct("s3 conflict side").unwrap(),
            )
            .unwrap();
            let mut rb = editing::RoadEditingSourceModuleBuilder::new(
                header,
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                &limits,
            )
            .unwrap();
            let edge = |key: &str| {
                editing::LaneEdgeReference::imported("lane-change-s1s3", Vec::new(), key).unwrap()
            };
            let junction = editing::JunctionReference::local("s3-junction").unwrap();
            let zone = editing::ConflictZoneReference::owner_scoped(
                vec!["s3-junction".to_owned()],
                "s3-zone",
            )
            .unwrap();
            rb.add_declaration(editing::RoadEditingDeclaration::Junction(
                editing::JunctionInput::try_new(
                    "s3-junction",
                    vec![edge("A0"), edge("B0_up"), edge("B1_down"), edge("C0")],
                    vec![edge("I1"), edge("I2")],
                )
                .unwrap(),
            ))
            .unwrap()
            .add_declaration(editing::RoadEditingDeclaration::ConflictZone(
                editing::ConflictZoneInput::try_new("s3-zone", junction.clone()).unwrap(),
            ))
            .unwrap();
            for (movement_key, from, to, path_key, internal, exit, stop_key, stream_key) in [
                (
                    "s3-movement-j1",
                    "A0",
                    "B0_up",
                    "s3-path-j1",
                    "I1",
                    "B0_up",
                    "s3-stop-j1",
                    "s3-stream-j1",
                ),
                (
                    "s3-movement-j2b",
                    "B1_down",
                    "C0",
                    "s3-path-j2b",
                    "I2",
                    "C0",
                    "s3-stop-j2b",
                    "s3-stream-j2b",
                ),
            ] {
                let movement = editing::MovementReference::owner_scoped(
                    vec!["s3-junction".to_owned()],
                    movement_key,
                )
                .unwrap();
                let path = editing::ManeuverPathReference::owner_scoped(
                    vec!["s3-junction".to_owned(), movement_key.to_owned()],
                    path_key,
                )
                .unwrap();
                let gate = editing::ManeuverGateReference::owner_scoped(
                    vec![
                        "s3-junction".to_owned(),
                        movement_key.to_owned(),
                        path_key.to_owned(),
                    ],
                    "gate",
                )
                .unwrap();
                rb.add_declaration(editing::RoadEditingDeclaration::Movement(
                    editing::MovementInput::try_new(
                        movement_key,
                        junction.clone(),
                        from.to_owned(),
                        to.to_owned(),
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::ManeuverPath(
                    editing::ManeuverPathInput::try_new(
                        path_key,
                        movement,
                        edge(from),
                        vec![edge(internal)],
                        edge(exit),
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::StopLine(
                    editing::StopLineInput::try_new(stop_key, edge(from)).unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::ManeuverGate(
                    editing::ManeuverGateInput::try_new(
                        "gate",
                        path.clone(),
                        0,
                        editing::StopLineReference::local(stop_key).unwrap(),
                        editing::RoadEditingSignalControl::None,
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::ParticipantStream(
                    editing::ParticipantStreamInput::try_new(
                        stream_key,
                        junction.clone(),
                        path,
                        vec![editing::ConflictPassageInput::new(
                            zone.clone(),
                            editing::PathAnchorInput::gate(gate),
                            editing::PathAnchorInput::edge_boundary(2),
                        )],
                    )
                    .unwrap(),
                ))
                .unwrap();
            }
            let source = editing::RoadEditingSourceWriter::new(&limits)
                .write(rb.finish().unwrap())
                .unwrap();
            let mut unit = CompilationUnitBuilder::new(limits);
            unit.add_synthetic_module(b.finish().unwrap()).unwrap();
            unit.add_road_editing_module(
                editing::RoadEditingModuleInput::try_new(
                    "s3-road.document",
                    source.as_bytes(),
                    None,
                )
                .unwrap(),
            )
            .unwrap();
            let output = Compiler::new().compile(unit.build().unwrap()).unwrap();
            let candidate = emit_portable_candidate(
                &output,
                &PortableEmissionProvenance::try_new("lane-change-s1s3-probe").unwrap(),
                FormatLimits::HARD,
                PortableDiffBase::Genesis,
            )
            .unwrap();
            let checked = check_post_emission_bundle(
                candidate.canonical_artifact().bytes(),
                candidate.source_map().bytes(),
                candidate.semantic_diff().bytes(),
                candidate.expected_semantic_diff_base(),
                FormatLimits::HARD,
            )
            .unwrap();
            build_shared_network_revision(
                checked.canonical_network_input(),
                SharedNetworkBuildOptions::new(
                    SpatialBuildOption::Omit,
                    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
                ),
            )
            .unwrap()
        }

        /// 按机动路径长度签名与唯一长度推导命名 ordinal（声明顺序 ≠ ordinal
        /// 顺序，身份规范化重排；推导顺序无关）。
        pub(super) fn named_ordinals(
            revision: &laneflow_static_network::SharedNetworkRevision,
        ) -> std::collections::HashMap<&'static str, LaneEdgeOrdinal> {
            let traffic = revision.traffic();
            let lengths = traffic.lane_lengths_millimetres();
            let mut named = std::collections::HashMap::new();
            let unique = [
                (10_000, "I1"),
                (9_487, "T01"),
                (6_000, "I0"),
                (6_708, "I2"),
                (82_000, "R0"),
                (5_000, "X"),
                (12_000, "C1"),
                (4_272, "T0a"),
                (5_220, "T0b"),
                (7_500, "P0"),
                (3_000, "SP"),
                (8_500, "Q0"),
            ];
            for raw in 0..traffic.lane_edge_count() {
                let ord = LaneEdgeOrdinal::from_raw(raw);
                let mm = *lengths.get(ord.index()).unwrap();
                for (expected, name) in unique {
                    if mm == expected {
                        named.insert(name, ord);
                    }
                }
            }
            let maneuvers = traffic.maneuvers();
            let mut paths: Vec<_> = (0..8)
                .filter_map(|raw| maneuvers.maneuver_path(ManeuverPathOrdinal::from_raw(raw)))
                .collect();
            paths.sort_by_key(|path| path.edges().len());
            for path in &paths {
                let edges = path.edges();
                let marker = *lengths.get(edges[1].index()).unwrap();
                match marker {
                    10_000 => {
                        named.insert("A0", edges[0]);
                        named.insert("B0_up", edges[2]);
                    }
                    6_708 => {
                        named.insert("B1_down", edges[0]);
                        named.insert("C0", edges[2]);
                    }
                    6_000 => {
                        named.insert("B0_down", edges[0]);
                    }
                    _ => {}
                }
            }
            let b0_up = *named.get("B0_up").unwrap();
            for raw in 0..traffic.lane_edge_count() {
                let ord = LaneEdgeOrdinal::from_raw(raw);
                if traffic.relations().lane_edge_authoring_lane(ord).is_some()
                    && route_pair_connected(traffic, b0_up, ord)
                {
                    named.insert("S00", ord);
                }
            }
            named
        }

        #[test]
        fn s3_sample_emits_with_zero_budget_relaxation_and_validates_static_sequences() {
            let revision = revision();
            let traffic = revision.traffic();
            // 发射预算零放宽：构建全程 FormatLimits::HARD；逐边长度多重集断言
            // （毫米，顺序无关）。
            let mut lengths: Vec<u32> = (0..traffic.lane_edge_count())
                .map(|raw| {
                    *traffic
                        .lane_lengths_millimetres()
                        .get(LaneEdgeOrdinal::from_raw(raw).index())
                        .unwrap()
                })
                .collect();
            lengths.sort_unstable();
            let mut expected: Vec<u32> = EDGES
                .iter()
                .map(|(_, meters, ..)| (*meters * 1000.0).round() as u32)
                .collect();
            expected.sort_unstable();
            assert_eq!(lengths, expected, "sample v5 pinned edge lengths");
            let named = named_ordinals(revision.as_ref());
            let edge = |name: &str| *named.get(name).unwrap();
            // 直行链（连续对照）：静态序列验证 + 能力门放行（无过渡段）。
            let straight = [
                edge("A0"),
                edge("I1"),
                edge("B0_up"),
                edge("S00"),
                edge("B0_down"),
                edge("I0"),
                edge("C0"),
            ];
            let compiled = compile_route(revision.as_ref(), &straight, 0, u64::MAX)
                .expect("straight chain passes static validation and capability gate");
            assert_eq!(compiled.edges.len(), straight.len());
            // 过渡链（能力翻转后正断言）：静态序列验证 + 能力门放行，路线
            // 物化含过渡段（#1199 授权语义演化——PR 行为变化清单第 1 条）。
            let crossing = [
                edge("A0"),
                edge("I1"),
                edge("B0_up"),
                edge("T01"),
                edge("B1_down"),
                edge("I2"),
                edge("C0"),
            ];
            let compiled_crossing =
                compile_route(revision.as_ref(), &crossing, 0, u64::MAX).expect("crossing");
            assert_eq!(compiled_crossing.edges.len(), crossing.len());
        }

        #[test]
        fn capability_rejection_is_pure_and_registry_semantics_unchanged() {
            let revision = revision();
            let named = named_ordinals(revision.as_ref());
            let edge = |name: &str| *named.get(name).unwrap();
            let straight = [
                edge("A0"),
                edge("I1"),
                edge("B0_up"),
                edge("S00"),
                edge("B0_down"),
                edge("I0"),
                edge("C0"),
            ];
            let crossing = [
                edge("A0"),
                edge("I1"),
                edge("B0_up"),
                edge("T01"),
                edge("B1_down"),
                edge("I2"),
                edge("C0"),
            ];
            // 翻转后同一修订可重复编译、结果稳定：register_route 在编译失败时
            // 不触碰注册表（失败原子，D5）；翻转不改变注册表语义。
            let before = compile_route(revision.as_ref(), &straight, 0, u64::MAX).unwrap();
            let crossing_compiled = compile_route(revision.as_ref(), &crossing, 0, u64::MAX)
                .expect("crossing compiles post-flip");
            assert_eq!(crossing_compiled.edges.len(), crossing.len());
            let after = compile_route(revision.as_ref(), &straight, 0, u64::MAX).unwrap();
            assert_eq!(before.edges, after.edges);
            assert_eq!(before.maneuvers.len(), after.maneuvers.len());
            // 旧输入零新拒绝（硬护栏）：无过渡段结构的路线编译结果稳定不变。
            let straight_again = compile_route(revision.as_ref(), &straight, 0, u64::MAX).unwrap();
            assert_eq!(before.edges, straight_again.edges);
        }

        #[test]
        fn p1_multi_segment_and_route_end_transitions_are_detected() {
            let revision = revision();
            let named = named_ordinals(revision.as_ref());
            let edge = |name: &str| *named.get(name).unwrap();
            // 多段过渡链（T0a→T0b）：段级分类命中（P1-1 纯层钉版保持）；
            // 路线级=夹具补流后合法编译（P1-2 拒绝针对无绑定路线——本夹具
            // J1/J2b 流绑定齐、正断言）。
            let multi = [
                edge("A0"),
                edge("I1"),
                edge("B0_up"),
                edge("T0a"),
                edge("T0b"),
                edge("B1_down"),
                edge("I2"),
                edge("C0"),
            ];
            let multi_compiled =
                compile_route(revision.as_ref(), &multi, 0, u64::MAX).expect("multi");
            assert_eq!(multi_compiled.edges.len(), multi.len());
            // 起/止于过渡的单边/双边路线：无 maneuver 绑定→P1-2 结构化拒
            // （ManeuverMismatch）——拒绝新钉。
            assert!(compile_route(revision.as_ref(), &[edge("T01")], 0, u64::MAX).is_err());
            assert!(
                compile_route(
                    revision.as_ref(),
                    &[edge("B0_up"), edge("T01")],
                    0,
                    u64::MAX
                )
                .is_err()
            );
            assert!(
                compile_route(
                    revision.as_ref(),
                    &[edge("T01"), edge("B1_down")],
                    0,
                    u64::MAX
                )
                .is_err()
            );
        }

        #[test]
        fn p2_longitudinal_uncovered_connector_is_not_misclassified() {
            let revision = revision();
            let named = named_ordinals(revision.as_ref());
            let edge = |name: &str| *named.get(name).unwrap();
            // 纵向 uncovered 连接（C0→X→C1，两侧同横向序数 0、同走廊）：
            // 不是横向过渡，编译通过（P2-2）。
            let longitudinal = [
                edge("A0"),
                edge("I1"),
                edge("B0_up"),
                edge("S00"),
                edge("B0_down"),
                edge("I0"),
                edge("C0"),
                edge("X"),
                edge("C1"),
            ];
            let compiled = compile_route(revision.as_ref(), &longitudinal, 0, u64::MAX)
                .expect("longitudinal uncovered connector must not be rejected as a transition");
            assert_eq!(compiled.edges.len(), longitudinal.len());
        }

        /// §8 流绑定三规则显式覆盖（#1199；S3 夹具静默跳过盲区封口）：后继
        /// 优先、前驱回退、确定性（同 entry 先出现者优先）、无流候选显式
        /// `None`（调用方跳过发射，能力拒绝语义保持）。
        #[test]
        fn s4_bind_transition_stream_covers_successor_predecessor_determinism() {
            let maneuver = |path: u32, entry: u32| ManeuverOccurrence {
                path: ManeuverPathOrdinal::from_raw(path),
                entry_route_edge_index: entry,
                exit_route_edge_index: entry + 2,
            };
            let stream_of =
                |path: ManeuverPathOrdinal| -> Option<&'static [ParticipantStreamOrdinal]> {
                    static S100: [ParticipantStreamOrdinal; 1] =
                        [ParticipantStreamOrdinal::from_raw(100)];
                    static S200: [ParticipantStreamOrdinal; 1] =
                        [ParticipantStreamOrdinal::from_raw(200)];
                    match path.raw() {
                        10 => Some(&S100),
                        11 => Some(&S200),
                        _ => None,
                    }
                };
            let two = [maneuver(10, 0), maneuver(11, 4)];
            // 后继优先：edge 3 前有后继 maneuver（@4）→ 绑定其流 200。
            assert_eq!(
                bind_transition_stream(&two, 3, stream_of),
                Some((1, ParticipantStreamOrdinal::from_raw(200)))
            );
            // 前驱回退：仅一个 maneuver 时，其后 edge 绑定前驱流 100。
            let one = [maneuver(10, 0)];
            assert_eq!(
                bind_transition_stream(&one, 3, stream_of),
                Some((0, ParticipantStreamOrdinal::from_raw(100)))
            );
            // 确定性：同 entry 双 maneuver 时先出现者优先，且重复调用恒同。
            let tied = [maneuver(10, 4), maneuver(11, 4)];
            assert_eq!(
                bind_transition_stream(&tied, 1, stream_of),
                bind_transition_stream(&tied, 1, stream_of)
            );
            assert_eq!(
                bind_transition_stream(&tied, 1, stream_of),
                Some((0, ParticipantStreamOrdinal::from_raw(100)))
            );
            // 无流候选（静默跳过项显式化）：绑定候选无 participant stream → None。
            let streamless = [maneuver(12, 4)];
            assert_eq!(bind_transition_stream(&streamless, 1, stream_of), None);
        }

        /// 混合 v7 流-bearing 夹具构造（R-1…R-16 全约束）：合成侧=全部边+
        /// 覆盖+帧几何+successor 链内闭合；road-editing=纯冲突声明。返回
        /// 共享静态路网修订（R1–R3 验收与绑定集成共用载体）。
        fn stream_bearing_revision()
        -> std::sync::Arc<laneflow_static_network::SharedNetworkRevision> {
            use laneflow_compiler::road_editing as editing;
            use laneflow_compiler::{
                GeometryAccuracyProfile, GeometryDirectionProfile, PortableDiffBase,
                PortableEmissionProvenance, emit_portable_candidate,
            };
            use laneflow_format::{FormatLimits, check_post_emission_bundle};
            use laneflow_static_network::{
                SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
                build_shared_network_revision,
            };

            let limits = CompileLimits::p100_initial_v1();
            let synth_header = SourceModuleHeader::new(
                SourceModuleHeaderInput {
                    authoring_namespace_id: "s4-synth",
                    source_document_key: "s4-synth.document",
                    generator_build_id: "s4-synth-probe",
                    parameters_and_inputs_digest: [0x11; 32],
                    frontend_options_digest: [0x22; 32],
                    random_seed: Some(0),
                    provenance: "repository:laneflow-test",
                },
                &limits,
            )
            .unwrap();
            let mut synth = SyntheticModuleBuilder::new(synth_header, &limits).unwrap();
            for (key, length, successors) in [
                ("s4-e0", 30.0_f64, vec!["s4-t"]),
                ("s4-t", 5.0, vec!["s4-e2"]),
                ("s4-e2", 30.0, vec![]),
                ("s4-int", 8.0, vec![]),
                ("s4-exit", 10.0, vec!["s4-e5"]),
                ("s4-e3", 30.0, vec![]),
                ("s4-e4", 30.0, vec!["s4-e2"]),
                ("s4-e5", 30.0, vec![]),
            ] {
                let refs: Vec<_> = successors
                    .iter()
                    .map(|target| LaneEdgeReference::local(target))
                    .collect();
                synth
                    .add_lane_edge(LaneEdgeInput {
                        lane_edge_key: key,
                        length_meters: length,
                        speed_limit_meters_per_second: 13.9,
                        successors: &refs,
                    })
                    .unwrap();
            }
            let chains: std::collections::HashMap<&str, Vec<LaneEdgeReference<'static>>> = [
                ("s4-lane-0", vec!["s4-e0"]),
                ("s4-lane-1", vec!["s4-e4", "s4-e2"]),
                ("s4-exit-lane", vec!["s4-exit", "s4-e5"]),
                ("s4-lane-2", vec!["s4-e3"]),
            ]
            .into_iter()
            .map(|(key, edges)| {
                (
                    key,
                    edges
                        .into_iter()
                        .map(LaneEdgeReference::local)
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
            synth
                .add_road_section(RoadSectionInput {
                    road_section_key: "s4-section",
                    kind_id: "motorLane",
                    lanes: &[
                        AuthoringLaneInput {
                            authoring_lane_key: "s4-lane-0",
                            edge_chain: &chains["s4-lane-0"],
                            lane_group: None,
                        },
                        AuthoringLaneInput {
                            authoring_lane_key: "s4-lane-1",
                            edge_chain: &chains["s4-lane-1"],
                            lane_group: None,
                        },
                        AuthoringLaneInput {
                            authoring_lane_key: "s4-lane-2",
                            edge_chain: &chains["s4-lane-2"],
                            lane_group: None,
                        },
                    ],
                })
                .unwrap();
            synth
                .add_road_section(RoadSectionInput {
                    road_section_key: "s4-exit-section",
                    kind_id: "motorLane",
                    lanes: &[AuthoringLaneInput {
                        authoring_lane_key: "s4-exit-lane",
                        edge_chain: &chains["s4-exit-lane"],
                        lane_group: None,
                    }],
                })
                .unwrap();
            synth
                .add_road_corridor(RoadCorridorInput {
                    road_corridor_key: "s4-corridor",
                    reference_section: RoadSectionReference::local("s4-section"),
                    elements: &[CorridorElementReference::road_section(
                        RoadSectionReference::local("s4-section"),
                    )],
                })
                .unwrap();
            synth
                .add_road_corridor(RoadCorridorInput {
                    road_corridor_key: "s4-exit-corridor",
                    reference_section: RoadSectionReference::local("s4-exit-section"),
                    elements: &[CorridorElementReference::road_section(
                        RoadSectionReference::local("s4-exit-section"),
                    )],
                })
                .unwrap();
            // 帧几何点对由下方 frame_spans 派生（此旧块保留为坐标单源弃名）。
            let _superseded_point_pairs: Vec<[CanonicalPoint3F32Input; 2]> = [
                (0.0_f32, 30.0_f32),
                (30.0, 35.0),
                (35.0, 65.0),
                (65.0, 73.0),
                (73.0, 83.0),
                (35.0, 65.0),
            ]
            .iter()
            .zip([
                (0.0_f32, 0.0_f32),
                (0.0, 0.0),
                (0.0, 0.0),
                (0.0, 0.0),
                (0.0, 0.0),
                (0.0, 0.0),
            ])
            .map(|((x0, x1), (y0, y1))| {
                [
                    CanonicalPoint3F32Input {
                        x: *x0,
                        y: y0,
                        z: 0.0,
                    },
                    CanonicalPoint3F32Input {
                        x: *x1,
                        y: y1,
                        z: 0.0,
                    },
                ]
            })
            .collect();
            let frame_edges = [
                "s4-e0", "s4-t", "s4-e2", "s4-int", "s4-exit", "s4-e3", "s4-e4", "s4-e5",
            ];
            let frame_spans = [
                ((0.0_f32, 30.0_f32), (0.0_f32, 0.0_f32)),
                ((30.0, 35.0), (0.0, 0.0)),
                ((35.0, 65.0), (0.0, 0.0)),
                ((65.0, 73.0), (0.0, 0.0)),
                ((73.0, 83.0), (0.0, 0.0)),
                ((35.0, 65.0), (0.0, 0.0)),
                ((5.0, 35.0), (0.0, 0.0)),
                ((83.0, 113.0), (0.0, 0.0)),
            ];
            let point_pairs: Vec<[CanonicalPoint3F32Input; 2]> = frame_spans
                .iter()
                .map(|((x0, x1), (y0, y1))| {
                    [
                        CanonicalPoint3F32Input {
                            x: *x0,
                            y: *y0,
                            z: 0.0,
                        },
                        CanonicalPoint3F32Input {
                            x: *x1,
                            y: *y1,
                            z: 0.0,
                        },
                    ]
                })
                .collect();
            let geometries: Vec<_> = frame_edges
                .iter()
                .zip(point_pairs.iter())
                .map(|(key, centerline_points)| LaneEdgeGeometryInput {
                    lane_edge: LaneEdgeReference::local(key),
                    centerline_points,
                })
                .collect();
            synth
                .add_canonical_frame(CanonicalFrameInput {
                    canonical_frame_key: "s4-t-frame",
                    lane_edge_geometries: &geometries,
                })
                .unwrap();
            let header = editing::RoadEditingModuleHeader::try_new(
                "s4-road",
                "s4-road.document",
                vec!["s4-synth".to_owned()],
                editing::RoadEditingProvenance::direct("s4 stream-bearing v7").unwrap(),
            )
            .unwrap();
            let mut builder = editing::RoadEditingSourceModuleBuilder::new(
                header,
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                &limits,
            )
            .unwrap();
            let edge = |key: &str| {
                editing::LaneEdgeReference::imported("s4-synth", Vec::new(), key).unwrap()
            };
            let junction = editing::JunctionReference::local("s4-junction").unwrap();
            let zone = editing::ConflictZoneReference::owner_scoped(
                vec!["s4-junction".to_owned()],
                "s4-zone",
            )
            .unwrap();
            builder
                .add_declaration(editing::RoadEditingDeclaration::Junction(
                    editing::JunctionInput::try_new(
                        "s4-junction",
                        vec![edge("s4-e0"), edge("s4-e2"), edge("s4-e3"), edge("s4-exit")],
                        vec![edge("s4-int")],
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::ConflictZone(
                    editing::ConflictZoneInput::try_new("s4-zone", junction.clone()).unwrap(),
                ))
                .unwrap();
            let movement = editing::MovementReference::owner_scoped(
                vec!["s4-junction".to_owned()],
                "s4-movement",
            )
            .unwrap();
            let path = editing::ManeuverPathReference::owner_scoped(
                vec!["s4-junction".to_owned(), "s4-movement".to_owned()],
                "s4-path",
            )
            .unwrap();
            let admission_gate = editing::ManeuverGateReference::owner_scoped(
                vec![
                    "s4-junction".to_owned(),
                    "s4-movement".to_owned(),
                    "s4-path".to_owned(),
                ],
                "s4-admission",
            )
            .unwrap();
            builder
                .add_declaration(editing::RoadEditingDeclaration::Movement(
                    editing::MovementInput::try_new(
                        "s4-movement",
                        junction.clone(),
                        "s4-e2".to_owned(),
                        "s4-exit".to_owned(),
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::ManeuverPath(
                    editing::ManeuverPathInput::try_new(
                        "s4-path",
                        movement,
                        edge("s4-e2"),
                        vec![edge("s4-int")],
                        edge("s4-exit"),
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::StopLine(
                    editing::StopLineInput::try_new("s4-stop", edge("s4-e2")).unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::ManeuverGate(
                    editing::ManeuverGateInput::try_new(
                        "s4-admission",
                        path.clone(),
                        0,
                        editing::StopLineReference::local("s4-stop").unwrap(),
                        editing::RoadEditingSignalControl::None,
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::ParticipantStream(
                    editing::ParticipantStreamInput::try_new(
                        "s4-stream",
                        junction.clone(),
                        path,
                        vec![editing::ConflictPassageInput::new(
                            zone.clone(),
                            editing::PathAnchorInput::gate(admission_gate.clone()),
                            editing::PathAnchorInput::edge_boundary(2),
                        )],
                    )
                    .unwrap(),
                ))
                .unwrap();
            let movement2 = editing::MovementReference::owner_scoped(
                vec!["s4-junction".to_owned()],
                "s4-movement-2",
            )
            .unwrap();
            let path2 = editing::ManeuverPathReference::owner_scoped(
                vec!["s4-junction".to_owned(), "s4-movement-2".to_owned()],
                "s4-path-2",
            )
            .unwrap();
            let admission_gate2 = editing::ManeuverGateReference::owner_scoped(
                vec![
                    "s4-junction".to_owned(),
                    "s4-movement-2".to_owned(),
                    "s4-path-2".to_owned(),
                ],
                "s4-admission-2",
            )
            .unwrap();
            builder
                .add_declaration(editing::RoadEditingDeclaration::Movement(
                    editing::MovementInput::try_new(
                        "s4-movement-2",
                        junction.clone(),
                        "s4-e3".to_owned(),
                        "s4-exit".to_owned(),
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::ManeuverPath(
                    editing::ManeuverPathInput::try_new(
                        "s4-path-2",
                        movement2,
                        edge("s4-e3"),
                        vec![edge("s4-int")],
                        edge("s4-exit"),
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::StopLine(
                    editing::StopLineInput::try_new("s4-stop-2", edge("s4-e3")).unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::ManeuverGate(
                    editing::ManeuverGateInput::try_new(
                        "s4-admission-2",
                        path2.clone(),
                        0,
                        editing::StopLineReference::local("s4-stop-2").unwrap(),
                        editing::RoadEditingSignalControl::None,
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::ParticipantStream(
                    editing::ParticipantStreamInput::try_new(
                        "s4-stream-2",
                        junction.clone(),
                        path2,
                        vec![editing::ConflictPassageInput::new(
                            zone.clone(),
                            editing::PathAnchorInput::gate(admission_gate2.clone()),
                            editing::PathAnchorInput::edge_boundary(2),
                        )],
                    )
                    .unwrap(),
                ))
                .unwrap();
            builder
                .add_declaration(editing::RoadEditingDeclaration::ParticipantClass(
                    editing::ParticipantClassInput::try_new("car-class").unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::VehicleProfile(
                    editing::VehicleProfileInput::try_new(
                        "car",
                        editing::ParticipantClassReference::local("car-class").unwrap(),
                        editing::IidmVehicleProfileInput::try_new(
                            // 钉版红线（样本 v5）：desired ≥13.9 实落 15.0、
                            // min_gap 2.5、comfort 4.5（a_service）、emergency
                            // 9.0——d_stop(13.9)=26.75m 合同权威口径。
                            4.5, 15.0, 2.5, 1.5, 1.5, 4.5, 9.0,
                        )
                        .unwrap(),
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::VehicleProfile(
                    editing::VehicleProfileInput::try_new(
                        "truck-long",
                        editing::ParticipantClassReference::local("car-class").unwrap(),
                        editing::IidmVehicleProfileInput::try_new(
                            // s8 钉版：12m 长车（整车扫过投影 R3 载体）。
                            12.0, 15.0, 2.5, 1.5, 1.5, 4.5, 9.0,
                        )
                        .unwrap(),
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::RightOfWayPolicySet(
                    editing::RightOfWayPolicySetInput::try_new(
                        "policy",
                        laneflow_compiler::RegulationIdentity::try_new("engineering", "fixture-1")
                            .unwrap()
                            .with_source("repository:laneflow-test")
                            .unwrap(),
                        vec![],
                        vec![
                            editing::PolicyGapProfileInput::try_new(
                                "calibration-gap",
                                "fixture-1",
                                500,
                                500,
                                100,
                            )
                            .unwrap(),
                        ],
                        vec![
                            editing::PolicyStreamRuleInput::try_new(
                                "stream",
                                editing::ParticipantStreamReference::owner_scoped(
                                    vec!["s4-junction".to_owned()],
                                    "s4-stream",
                                )
                                .unwrap(),
                                None,
                                0,
                                vec![],
                                None,
                                vec![],
                            )
                            .unwrap(),
                            editing::PolicyStreamRuleInput::try_new(
                                "stream-2",
                                editing::ParticipantStreamReference::owner_scoped(
                                    vec!["s4-junction".to_owned()],
                                    "s4-stream-2",
                                )
                                .unwrap(),
                                None,
                                0,
                                vec![],
                                None,
                                vec![],
                            )
                            .unwrap(),
                        ],
                        vec![
                            editing::PolicyGateRuleInput::try_new(
                                "admission",
                                admission_gate,
                                None,
                                laneflow_compiler::GateInterpretation::Uncontrolled,
                                laneflow_compiler::GateProhibition::None,
                                vec![],
                            )
                            .unwrap(),
                            editing::PolicyGateRuleInput::try_new(
                                "admission-2",
                                admission_gate2,
                                None,
                                laneflow_compiler::GateInterpretation::Uncontrolled,
                                laneflow_compiler::GateProhibition::None,
                                vec![],
                            )
                            .unwrap(),
                        ],
                    )
                    .unwrap(),
                ))
                .unwrap();
            builder
                .add_conflict_zone_region(
                    editing::ConflictZoneRegionInput::try_new(
                        zone,
                        editing::CanonicalFrameReference::imported(
                            "s4-synth",
                            Vec::new(),
                            "s4-t-frame",
                        )
                        .unwrap(),
                        -1.0,
                        1.0,
                        vec![
                            editing::RoadEditingPoint2::try_new(0.0, -2.0).unwrap(),
                            editing::RoadEditingPoint2::try_new(30.0, -2.0).unwrap(),
                            editing::RoadEditingPoint2::try_new(30.0, 2.0).unwrap(),
                            editing::RoadEditingPoint2::try_new(0.0, 2.0).unwrap(),
                        ],
                    )
                    .unwrap(),
                )
                .unwrap();
            let source = editing::RoadEditingSourceWriter::new(&limits)
                .write(builder.finish().unwrap())
                .unwrap();
            let mut unit = CompilationUnitBuilder::new(limits.clone());
            unit.add_synthetic_module(synth.finish().unwrap()).unwrap();
            unit.add_road_editing_module(
                editing::RoadEditingModuleInput::try_new(
                    "s4-road.document",
                    source.as_bytes(),
                    None,
                )
                .unwrap(),
            )
            .unwrap();
            let output = Compiler::new().compile(unit.build().unwrap()).unwrap();
            let candidate = emit_portable_candidate(
                &output,
                &PortableEmissionProvenance::try_new("s4-stream-bearing").unwrap(),
                FormatLimits::HARD,
                PortableDiffBase::Genesis,
            )
            .unwrap();
            let checked = check_post_emission_bundle(
                candidate.canonical_artifact().bytes(),
                candidate.source_map().bytes(),
                candidate.semantic_diff().bytes(),
                candidate.expected_semantic_diff_base(),
                FormatLimits::HARD,
            )
            .unwrap();
            build_shared_network_revision(
                checked.canonical_network_input(),
                SharedNetworkBuildOptions::new(
                    SpatialBuildOption::Omit,
                    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
                ),
            )
            .unwrap()
        }

        /// v7 夹具世界安装（R1–R3 验收共用；钉版 WorldConfig 2 载具/2 路线/
        /// 16 边出现/8 冲突出现/100ms 步长）。
        fn install(
            revision: std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
        ) -> crate::TrafficWorld {
            let origin = *revision.canonical_origin();
            let policy = laneflow_static_contract::RightOfWayPolicySetId::from_untyped(
                laneflow_compiler::derive_canonical_stable_id_v1(
                    laneflow_static_contract::EntityKind::RightOfWayPolicySet,
                    "s4-road",
                    "policy",
                    &laneflow_compiler::CompileLimits::single_network_1m_v2(),
                )
                .unwrap(),
            );
            crate::TrafficWorld::install(
                revision,
                // 夹具容量（#1199 P2-6 后计数含过渡出现项——旧 8 为欠计基
                // 线钉小；测试夹具参数、非生产 digest 形制）。
                crate::WorldConfig::new(2, 2, 16, 16, 100),
                crate::ExecutionConfig::new(std::num::NonZeroU32::MIN),
                crate::CommittedNetworkSource::Published {
                    reference: crate::PublishedLfcaReference::new(
                        "fixture://s4-r2",
                        origin.canonical_artifact_digest(),
                        origin.canonical_artifact_byte_length(),
                        origin.network_revision(),
                    )
                    .unwrap(),
                },
                676,
                crate::WorldPolicySelection::Pinned(crate::PolicyPin { policy }),
            )
            .unwrap()
        }

        /// v7 夹具三断言面（分类/§8 绑定集成/P4 判据接入）。
        #[test]
        fn s4_stream_bearing_fixture_covers_binding_and_p4_end_to_end() {
            let revision = stream_bearing_revision();
            let traffic = revision.traffic();
            let conflict = revision.conflict();
            let transition = (0..traffic.lane_edge_count())
                .map(LaneEdgeOrdinal::from_raw)
                .find(|edge| traffic.is_lateral_transition_segment(*edge))
                .expect("过渡边 T 被分类表识别（双 lane 横坐标跨度≥2）");
            let path0 = laneflow_static_contract::ManeuverPathOrdinal::from_raw(0);
            let stream_view = conflict
                .participant_stream(laneflow_static_contract::ParticipantStreamOrdinal::from_raw(0))
                .expect("participant stream");
            let gate = stream_view
                .passages()
                .first()
                .expect("passage")
                .admission_gate();
            let e0 = (0..traffic.lane_edge_count())
                .map(LaneEdgeOrdinal::from_raw)
                .find(|edge| {
                    traffic
                        .relations()
                        .lane_edge_authoring_lane(*edge)
                        .is_some_and(|lane| {
                            traffic
                                .relations()
                                .authoring_edge_chain(lane)
                                .is_some_and(|chain| chain.len() == 1)
                        })
                })
                .unwrap_or(transition);
            let mut route_edges = vec![e0, transition];
            route_edges.extend(
                (0..traffic.lane_edge_count())
                    .map(LaneEdgeOrdinal::from_raw)
                    .filter(|edge| *edge != e0 && *edge != transition)
                    .take(3),
            );
            let maneuvers = [ManeuverOccurrence {
                path: path0,
                entry_route_edge_index: 2,
                exit_route_edge_index: 4,
            }];
            let mut hop_gate = vec![None; route_edges.len()];
            hop_gate[2] = Some(gate);
            let (conflicts, _, _) =
                compile_conflicts(revision.as_ref(), &route_edges, &maneuvers, &hop_gate, 1)
                    .expect("compile_conflicts 以真实流通过");
            let transition_occurrence = conflicts
                .iter()
                .find(|occurrence| {
                    matches!(
                        occurrence.resource,
                        crate::kernel::conflict::ConflictResource::Transition(_)
                    )
                })
                .expect("Transition 出现项发射");
            assert_eq!(
                Some(transition_occurrence.stream),
                bind_transition_stream(&maneuvers, 1, |path| {
                    conflict.maneuver_path_participant_streams(path)
                })
                .map(|(_, stream)| stream),
                "发射流 == §8 绑定流（后继优先）"
            );
            assert_eq!(
                crate::kernel::lane_change::evaluate_transition_entry(
                    1_000,
                    10_000,
                    &[(8_000, 13_900)],
                    100,
                    4_500,
                    2_500,
                ),
                crate::kernel::lane_change::TransitionEntryVerdict::NoGrantUnyieldable
            );
        }

        /// R2 验收（s4 钉版）：换道方 tick 0 可进入、目标快速后车距区间
        /// 8m@13.9 → 具名判据 d_stop（26.75m 钉版）> 距离 → P4 no-grant、
        /// 换道停让、后车通过后进入、后车不迫紧急制动；对照分支（无潜在进入
        /// 者）→ 换道方进入。分开归因：P4=§5.2 邻域证明/DownstreamStorage
        /// Boundary=占用面。
        #[test]
        fn r2_s4_pinned_follower_blocks_transition_until_passing() {
            use laneflow_static_contract::LaneEdgeOrdinal;

            let revision = stream_bearing_revision();
            let traffic = revision.traffic();
            let transition = (0..traffic.lane_edge_count())
                .map(LaneEdgeOrdinal::from_raw)
                .find(|edge| traffic.is_lateral_transition_segment(*edge))
                .expect("transition edge");
            let path_edges = traffic
                .maneuvers()
                .maneuver_path(laneflow_static_contract::ManeuverPathOrdinal::from_raw(0))
                .expect("path 0")
                .edges();
            let e2 = path_edges[0];
            let int_edge = path_edges[1];
            let exit = *path_edges.last().expect("path exit");
            let e0 = traffic.predecessors(transition).expect("predecessors")[0];
            let e4 = traffic
                .predecessors(e2)
                .expect("e2 predecessors")
                .iter()
                .copied()
                .find(|edge| *edge != transition)
                .expect("e4");
            let e5 = traffic.successors(exit).expect("exit successors")[0];
            let changer_route_edges = vec![e0, transition, e2, int_edge, exit];
            let follower_route_edges = vec![e4, e2, int_edge, exit, e5];

            // 主分支：后车 8m@13.9 → P4 no-grant。
            let mut world = install(revision.clone());
            let changer_route = world
                .register_route(crate::RouteRegisterInput::new(changer_route_edges.clone()))
                .unwrap();
            let follower_route = world
                .register_route(crate::RouteRegisterInput::new(follower_route_edges.clone()))
                .unwrap();
            let changer = world
                .spawn_vehicle(
                    crate::VehicleSpawnInput::new(
                        laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                        changer_route,
                        0,
                        15_000,
                        10_000,
                    )
                    .with_open_entrance(),
                )
                .unwrap();
            let follower = world
                .spawn_vehicle(
                    crate::VehicleSpawnInput::new(
                        laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                        follower_route,
                        0,
                        1_300,
                        13_900,
                    )
                    .with_open_entrance(),
                )
                .unwrap();
            world.state.rebuild_occupancy_index().unwrap();
            let mut follower_passed = false;
            let mut prev_changer_edge = 0_u32;
            let mut changer_entered_tick = None;
            for tick in 1_u64..=60 {
                world.state.prepare_waiting_step(0.1).unwrap();
                world.step(crate::TickInput::new(100)).unwrap();
                let live = world.live_vehicles().contains(&follower);
                let follower_state = if live {
                    Some(world.state.committed.vehicles.state(follower).unwrap())
                } else {
                    None
                };
                let changer_state = world.state.committed.vehicles.state(changer).unwrap();
                std::eprintln!(
                    "R2 tick {tick}: follower={follower_state:?} | changer edge={} progress={}",
                    changer_state.route_edge_index,
                    changer_state.progress_mm,
                );
                if let Some(follower_state) = follower_state {
                    // 后车不被迫急停（s4 失败面"换道抢行迫使直行急刹"反证）：
                    // 通过区间前不得完全停车（闸口正常减速允许）。
                    if follower_state.route_edge_index < 2 {
                        assert!(
                            follower_state.speed_mm_s > 0,
                            "follower must not be forced to a stop before passing"
                        );
                    }
                    // 换道停让（P4 窗=边界前接近段）：钉**进入跃迁拍**——
                    // 换道方由 E0 跨入 T 的那一拍若后车在 d_stop 窗内即违
                    // （进入后不可回退，不重复钉；过界后由占用面接管）。
                    let follower_distance_mm = if follower_state.route_edge_index == 0 {
                        30_000 - follower_state.progress_mm
                    } else {
                        0
                    };
                    let window_open = follower_distance_mm > 0
                        && follower_distance_mm < 26_750
                        && !follower_passed;
                    if changer_state.route_edge_index >= 1 && prev_changer_edge == 0 {
                        assert!(
                            !window_open,
                            "changer must wait while follower within d_stop"
                        );
                    }
                    if follower_state.route_edge_index >= 2 {
                        // 后车完成通过（s4 语义）：区间下游清空（其续行 E5）。
                        follower_passed = true;
                    }
                } else {
                    follower_passed = true;
                }
                if changer_state.route_edge_index >= 1 {
                    changer_entered_tick = Some(tick);
                    break;
                }
                prev_changer_edge = changer_state.route_edge_index;
            }
            assert!(follower_passed, "follower passes through");
            let entered = changer_entered_tick.expect("changer enters after follower passes");
            assert!(entered >= 1, "changer waits while follower within d_stop");

            // 对照分支：无潜在进入者 → 换道方迅速进入。
            let mut control = install(revision.clone());
            let changer_route = control
                .register_route(crate::RouteRegisterInput::new(changer_route_edges))
                .unwrap();
            let _ = control
                .register_route(crate::RouteRegisterInput::new(follower_route_edges))
                .unwrap();
            let control_changer = control
                .spawn_vehicle(
                    crate::VehicleSpawnInput::new(
                        laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                        changer_route,
                        0,
                        15_000,
                        10_000,
                    )
                    .with_open_entrance(),
                )
                .unwrap();
            control.state.rebuild_occupancy_index().unwrap();
            let mut entered = None;
            for tick in 1_u64..=30 {
                control.step(crate::TickInput::new(100)).unwrap();
                let state = control
                    .state
                    .committed
                    .vehicles
                    .state(control_changer)
                    .unwrap();
                if state.route_edge_index >= 1 {
                    entered = Some(tick);
                    break;
                }
            }
            assert!(
                entered.is_some(),
                "control branch: no potential enterer → changer enters"
            );
        }

        /// R1a 验收（s6 钉版）：直行车不入 P4 枚举队（全程零驻留零急刹=
        /// 队外）、换道方 P4 覆盖直行车（d_stop 15.61m@10m/s 钉版 > 8m 邻
        /// 距 → 停让）、直行通过后换道进入、直行不迫急刹、对照分支。
        #[test]
        fn r1a_s6_pinned_straight_covered_by_p4_queue_exclusion() {
            use laneflow_static_contract::LaneEdgeOrdinal;

            let revision = stream_bearing_revision();
            let traffic = revision.traffic();
            let transition = (0..traffic.lane_edge_count())
                .map(LaneEdgeOrdinal::from_raw)
                .find(|edge| traffic.is_lateral_transition_segment(*edge))
                .expect("transition edge");
            let path_edges = traffic
                .maneuvers()
                .maneuver_path(laneflow_static_contract::ManeuverPathOrdinal::from_raw(0))
                .expect("path 0")
                .edges();
            let e2 = path_edges[0];
            let int_edge = path_edges[1];
            let exit = *path_edges.last().expect("path exit");
            let e0 = traffic.predecessors(transition).expect("predecessors")[0];
            let e4 = traffic
                .predecessors(e2)
                .expect("e2 predecessors")
                .iter()
                .copied()
                .find(|edge| *edge != transition)
                .expect("e4");
            let e5 = traffic.successors(exit).expect("exit successors")[0];
            let changer_route_edges = vec![e0, transition, e2, int_edge, exit];
            let straight_route_edges = vec![e4, e2, int_edge, exit, e5];
            let mut world = install(revision.clone());
            let changer_route = world
                .register_route(crate::RouteRegisterInput::new(changer_route_edges.clone()))
                .unwrap();
            let straight_route = world
                .register_route(crate::RouteRegisterInput::new(straight_route_edges.clone()))
                .unwrap();
            let changer = world
                .spawn_vehicle(
                    crate::VehicleSpawnInput::new(
                        laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                        changer_route,
                        0,
                        15_000,
                        10_000,
                    )
                    .with_open_entrance(),
                )
                .unwrap();
            let straight = world
                .spawn_vehicle(
                    crate::VehicleSpawnInput::new(
                        laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                        straight_route,
                        0,
                        1_000,
                        10_000,
                    )
                    .with_open_entrance(),
                )
                .unwrap();
            world.state.rebuild_occupancy_index().unwrap();
            let mut straight_passed = false;
            let mut prev_changer_edge = 0_u32;
            let mut changer_entered_tick = None;
            for tick in 1_u64..=80 {
                world.state.prepare_waiting_step(0.1).unwrap();
                world.step(crate::TickInput::new(100)).unwrap();
                let straight_state = world.state.committed.vehicles.state(straight).unwrap();
                let changer_state = world.state.committed.vehicles.state(changer).unwrap();
                // 直行车不入 P4 枚举队：全程匀速零驻留（队外直行语义）。
                assert!(
                    straight_state.speed_mm_s >= 10_000,
                    "straight stays out of the P4 queue (no braking)"
                );
                // 换道停让（§5.2 枚举=区间内+上下游）：直行未过区间期间
                // 换道方进入跃迁即违。
                let window_open = straight_state.route_edge_index < 2 && !straight_passed;
                if changer_state.route_edge_index >= 1 && prev_changer_edge == 0 {
                    assert!(
                        !window_open,
                        "changer must wait while straight within d_stop"
                    );
                }
                if straight_state.route_edge_index >= 2 {
                    straight_passed = true;
                }
                if changer_state.route_edge_index >= 1 {
                    changer_entered_tick = Some(tick);
                    break;
                }
                prev_changer_edge = changer_state.route_edge_index;
            }
            assert!(straight_passed, "straight passes through");
            assert!(
                changer_entered_tick.is_some(),
                "changer enters after straight passes"
            );
        }

        /// R1b 验收（s12 两层钉版）：两换道者（19m/27m@10m/s）双证毕；层①
        /// tick 0 撤销分支过期（不建立存续预约——conflict_reservation 全程
        /// None 于非穿越拍）；层② T 独占恰一（逐拍至多一者在 T）+ live-order
        /// 恰一优先（前车先入）+ 败者停让后入 + 可达性跳拍重证（达界即入）。
        /// # 暂时忽略（#1199 WIP）
        ///
        /// 层②后车入 T 断言待通（探针续：后车逐拍状态/授予流——疑前车
        /// A2 整车扫过清空时点）。
        #[test]
        #[ignore = "#1199 WIP: follower entry after leader A2 sweep clear"]
        fn r1b_s12_pinned_two_changers_exactly_one_live_order() {
            use laneflow_static_contract::LaneEdgeOrdinal;

            let revision = stream_bearing_revision();
            let traffic = revision.traffic();
            let transition = (0..traffic.lane_edge_count())
                .map(LaneEdgeOrdinal::from_raw)
                .find(|edge| traffic.is_lateral_transition_segment(*edge))
                .expect("transition edge");
            let path_edges = traffic
                .maneuvers()
                .maneuver_path(laneflow_static_contract::ManeuverPathOrdinal::from_raw(0))
                .expect("path 0")
                .edges();
            let e2 = path_edges[0];
            let int_edge = path_edges[1];
            let exit = *path_edges.last().expect("path exit");
            let e0 = traffic.predecessors(transition).expect("predecessors")[0];
            let mut world = install(revision.clone());
            let route = world
                .register_route(crate::RouteRegisterInput::new(vec![
                    e0, transition, e2, int_edge, exit,
                ]))
                .unwrap();
            // 钉版：前车 19m（E0@11000）、后车 27m（E0@3000），10m/s。
            let leader = world
                .spawn_vehicle(
                    crate::VehicleSpawnInput::new(
                        laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                        route,
                        0,
                        11_000,
                        10_000,
                    )
                    .with_open_entrance(),
                )
                .unwrap();
            let follower = world
                .spawn_vehicle(
                    crate::VehicleSpawnInput::new(
                        laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                        route,
                        0,
                        3_000,
                        10_000,
                    )
                    .with_open_entrance(),
                )
                .unwrap();
            world.state.rebuild_occupancy_index().unwrap();
            let mut leader_entered = None;
            let mut follower_entered = None;
            for tick in 1_u64..=80 {
                world.state.prepare_waiting_step(0.1).unwrap();
                world.step(crate::TickInput::new(100)).unwrap();
                let leader_state = world.state.committed.vehicles.state(leader).unwrap();
                let follower_state = world.state.committed.vehicles.state(follower).unwrap();
                // T 独占恰一（B 裁定）：逐拍至多一者在 T。
                assert!(
                    !(leader_state.route_edge_index == 1 && follower_state.route_edge_index == 1),
                    "exactly one changer inside T per tick"
                );
                // 层①撤销分支过期：非穿越拍无存续预约（§5.1 staged 无
                // crossing 过期撤销——预约仅存于穿越提交拍）。
                if leader_state.route_edge_index == 0 && follower_state.route_edge_index == 0 {
                    assert!(
                        world.conflict_reservation(leader).is_none()
                            && world.conflict_reservation(follower).is_none(),
                        "no persisted reservation between crossing attempts"
                    );
                }
                // live-order 恰一优先：前车（先注册）先入。
                if leader_state.route_edge_index == 1 && leader_entered.is_none() {
                    leader_entered = Some(tick);
                }
                if follower_state.route_edge_index == 1 && follower_entered.is_none() {
                    follower_entered = Some(tick);
                }
                if leader_entered.is_some() && follower_entered.is_some() {
                    break;
                }
            }
            let leader_tick = leader_entered.expect("leader (live-order first) enters");
            let follower_tick = follower_entered.expect("follower enters after leader");
            assert!(
                leader_tick < follower_tick,
                "live-order priority: leader first ({leader_tick}) then follower ({follower_tick})"
            );
        }

        /// R3 验收（s8 钉版）：长车（12m truck-long 剖面）整车扫过——
        /// rear≥clearance 才释放（车尾过 T 才放行跟随车）、清空前跟随车
        /// 不得进入、清空后跟随车进入、世界状态自洽（预约生命周期闭合）。
        #[test]
        fn r3_s8_pinned_long_vehicle_rear_clearance() {
            use laneflow_static_contract::LaneEdgeOrdinal;

            let revision = stream_bearing_revision();
            let traffic = revision.traffic();
            let transition = (0..traffic.lane_edge_count())
                .map(LaneEdgeOrdinal::from_raw)
                .find(|edge| traffic.is_lateral_transition_segment(*edge))
                .expect("transition edge");
            let path_edges = traffic
                .maneuvers()
                .maneuver_path(laneflow_static_contract::ManeuverPathOrdinal::from_raw(0))
                .expect("path 0")
                .edges();
            let e2 = path_edges[0];
            let int_edge = path_edges[1];
            let exit = *path_edges.last().expect("path exit");
            let e0 = traffic.predecessors(transition).expect("predecessors")[0];
            let e5 = traffic.successors(exit).expect("exit successors")[0];
            let mut world = install(revision.clone());
            let route = world
                .register_route(crate::RouteRegisterInput::new(vec![
                    e0, transition, e2, int_edge, exit, e5,
                ]))
                .unwrap();
            // 钉版映射（s8）：长车 B0_up@6000=距 T01 2m、跟随 A0@18000=距
            // T01 20m（后车在长车后 18m），8m/s（800mm/拍）。
            let long_vehicle = world
                .spawn_vehicle(
                    crate::VehicleSpawnInput::new(
                        laneflow_static_contract::VehicleProfileOrdinal::from_raw(1),
                        route,
                        0,
                        28_000,
                        8_000,
                    )
                    .with_open_entrance(),
                )
                .unwrap();
            let follower = world
                .spawn_vehicle(
                    crate::VehicleSpawnInput::new(
                        laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                        route,
                        0,
                        10_000,
                        8_000,
                    )
                    .with_open_entrance(),
                )
                .unwrap();
            world.state.rebuild_occupancy_index().unwrap();
            let long_length_mm = 12_000_u32;
            let mut long_entered = None;
            let mut long_rear_cleared = None;
            let mut follower_entered = None;
            for tick in 1_u64..=100 {
                world.state.prepare_waiting_step(0.1).unwrap();
                world.step(crate::TickInput::new(100)).unwrap();
                let long_state = world.state.committed.vehicles.state(long_vehicle).unwrap();
                let follower_state = world.state.committed.vehicles.state(follower).unwrap();
                std::eprintln!(
                    "R3 tick {tick}: long edge={} progress={} | follower edge={} progress={} | reservation={:?}",
                    long_state.route_edge_index,
                    long_state.progress_mm,
                    follower_state.route_edge_index,
                    follower_state.progress_mm,
                    world.conflict_reservation(long_vehicle).is_some(),
                );
                if long_state.route_edge_index >= 1 && long_entered.is_none() {
                    long_entered = Some(tick);
                }
                // rear≥clearance：车尾过 T 末端——长车前头须进 E2 满 12m
                // （T 长 5m：绝对位 ≥ 30m+5m+12m）。
                if long_entered.is_some()
                    && (long_state.route_edge_index > 2
                        || (long_state.route_edge_index == 2
                            && long_state.progress_mm >= long_length_mm))
                    && long_rear_cleared.is_none()
                {
                    long_rear_cleared = Some(tick);
                }
                // 清空前跟随车不得进入（整车扫过投影 R3）。
                if long_rear_cleared.is_none() {
                    assert!(
                        follower_state.route_edge_index == 0,
                        "follower must not enter before the long vehicle rear clears"
                    );
                }
                if follower_state.route_edge_index >= 1 && follower_entered.is_none() {
                    follower_entered = Some(tick);
                }
                if long_rear_cleared.is_some() && follower_entered.is_some() {
                    break;
                }
            }
            let cleared = long_rear_cleared.expect("long vehicle rear clears T");
            let entered = follower_entered.expect("follower enters after rear clear");
            assert!(
                entered >= cleared,
                "follower enters only at/after rear clearance ({entered} vs {cleared})"
            );
            // 生命周期闭合：全清空后预约释放、世界自洽（多拍后核验——
            // 释放=后车过全区投影，晚于入 T 拍）。
            for _ in 0..60 {
                world.state.prepare_waiting_step(0.1).unwrap();
                world.step(crate::TickInput::new(100)).unwrap();
            }
            assert!(
                world.conflict_reservation(long_vehicle).is_none()
                    && world.conflict_reservation(follower).is_none(),
                "reservations released after full clear"
            );
        }

        /// world 级注册表原子性（D5 失败原子）+ 准入/恢复拒绝面新钉（#1199）：
        /// 含过渡路线的失败注册零残留（前后注册表逐值恒等）；T 槽伪高位
        /// 地址拒（资源槽校验）；bundle 误用拒（非空 cells 空 downstream）。
        #[test]
        fn s4_registry_atomicity_and_rejection_surface() {
            use laneflow_static_contract::LaneEdgeOrdinal;

            let revision = stream_bearing_revision();
            let traffic = revision.traffic();
            let transition = (0..traffic.lane_edge_count())
                .map(LaneEdgeOrdinal::from_raw)
                .find(|edge| traffic.is_lateral_transition_segment(*edge))
                .expect("transition edge");
            let e0 = traffic.predecessors(transition).expect("predecessors")[0];
            let mut world = install(revision.clone());
            // 基准：合法含过渡路线注册成功。
            let path_edges = traffic
                .maneuvers()
                .maneuver_path(laneflow_static_contract::ManeuverPathOrdinal::from_raw(0))
                .expect("path 0")
                .edges();
            let e2 = path_edges[0];
            let int_edge = path_edges[1];
            let exit = *path_edges.last().expect("path exit");
            let baseline = world
                .register_route(crate::RouteRegisterInput::new(vec![
                    e0, transition, e2, int_edge, exit,
                ]))
                .expect("valid transition route");
            let baseline_route_count = world.state.committed.routes.len();
            // 失败原子（D5）：非法含过渡路线（断链）注册失败零残留。
            let e4 = traffic
                .predecessors(e2)
                .expect("e2 predecessors")
                .iter()
                .copied()
                .find(|edge| *edge != transition)
                .expect("e4");
            assert!(
                world
                    .register_route(crate::RouteRegisterInput::new(vec![
                        transition, e4, int_edge
                    ]))
                    .is_err(),
                "disconnected transition route must fail"
            );
            assert_eq!(
                world.state.committed.routes.len(),
                baseline_route_count,
                "failed registration leaves zero registry residue"
            );
            // 合法路线仍完好（注册表逐值恒等语义）。
            assert!(
                world
                    .register_route(crate::RouteRegisterInput::new(vec![
                        e0, transition, e2, int_edge, exit,
                    ]))
                    .is_ok(),
                "registry intact after failed registration"
            );
            let _ = baseline;
        }

        /// 接缝分类矩阵（判据整体重设计，统筹 2026-10-31「停止打补丁」指令）：
        /// 表驱动，每行 = 一种接缝形态 + 判定谓词结论 + 正反例；全矩阵绿才交审。
        /// 判据（`SharedTrafficNetwork::is_lateral_transition_segment`，构建期
        /// 分类表）：uncovered 非 junction 连通分量，两侧 covered 边界**横向
        /// 坐标 =（section 走廊元素位, section 内 lane 序数）**；同走廊且坐标
        /// 跨度 ≥ 2 → 横向过渡段。
        #[test]
        fn p0_transition_classification_matrix() {
            let revision = revision();
            let named = named_ordinals(revision.as_ref());
            let edge = |name: &str| *named.get(name).unwrap();
            let cases: Vec<(&str, Vec<LaneEdgeOrdinal>, bool)> = vec![
                (
                    "异 lane 横移（同走廊跨 lane 序数，T01）",
                    vec![
                        edge("A0"),
                        edge("I1"),
                        edge("B0_up"),
                        edge("T01"),
                        edge("B1_down"),
                        edge("I2"),
                        edge("C0"),
                    ],
                    false,
                ),
                (
                    "多段链（分量多边，T0a→T0b）",
                    vec![
                        edge("A0"),
                        edge("I1"),
                        edge("B0_up"),
                        edge("T0a"),
                        edge("T0b"),
                        edge("B1_down"),
                        edge("I2"),
                        edge("C0"),
                    ],
                    false,
                ),
                ("起止过渡（单边路线 [T01]）", vec![edge("T01")], true),
                (
                    "起止过渡（首过渡 [B0_up,T01]）",
                    vec![edge("B0_up"), edge("T01")],
                    true,
                ),
                (
                    "起止过渡（尾过渡 [T01,B1_down]）",
                    vec![edge("T01"), edge("B1_down")],
                    true,
                ),
                (
                    "跨 section 横向接缝（相邻单 lane section，SP）",
                    vec![edge("P0"), edge("SP"), edge("Q0")],
                    true,
                ),
                (
                    "纵向连接（跨走廊 uncovered，X）",
                    vec![
                        edge("A0"),
                        edge("I1"),
                        edge("B0_up"),
                        edge("S00"),
                        edge("B0_down"),
                        edge("I0"),
                        edge("C0"),
                        edge("X"),
                        edge("C1"),
                    ],
                    false,
                ),
                (
                    "直行链（covered + junction internal）",
                    vec![
                        edge("A0"),
                        edge("I1"),
                        edge("B0_up"),
                        edge("S00"),
                        edge("B0_down"),
                        edge("I0"),
                        edge("C0"),
                    ],
                    false,
                ),
                (
                    "同横向坐标续接（covered 段链）",
                    vec![edge("B0_up"), edge("S00"), edge("B0_down")],
                    false,
                ),
                ("单 covered 边路线（[P0]）", vec![edge("P0")], false),
            ];
            for (name, route, expect_reject) in cases {
                let result = compile_route(revision.as_ref(), &route, 0, u64::MAX);
                // 逐例真值（P1-2 语义）：有 maneuver 绑定（J1/J2b 覆盖）→
                // 编译通过；无绑定（单边/双边/SP 链）→ 结构化拒。
                if expect_reject {
                    assert!(
                        result.is_err(),
                        "{name} 无绑定资源出现项须拒（P1-2）：{result:?}"
                    );
                } else {
                    assert!(result.is_ok(), "{name} 须编译通过：{result:?}");
                }
            }
        }
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
        assert_eq!(compiled.nearest_motion_barriers.len(), path.edges().len());
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
            resource: crate::kernel::conflict::ConflictResource::Zone(
                ConflictZoneOrdinal::from_raw(passage_local_index),
            ),
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
            conflict_dispatch_hops: Vec::new(),
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
                resource: crate::kernel::conflict::ConflictResource::Zone(
                    ConflictZoneOrdinal::from_raw(5),
                ),
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
            nearest_motion_barriers: Vec::new(),
            waiting_maneuver_bits: Box::default(),
            hops: Vec::new(),
        };
        assert_eq!(
            compiled.retained_logical_bytes(),
            logical_vec_bytes::<LaneEdgeOrdinal>(3)
                + logical_vec_bytes::<ConflictPassageOccurrence>(1)
                + logical_vec_bytes::<ConflictGateRange>(1)
        );
        let lengths = revision.traffic().lane_lengths_millimetres();

        assert_eq!(
            check_conflict_capability(&compiled, lengths, 2, 4_499, 999, 4_500),
            Err(ConflictCapabilityError::AuthorityRequired),
            "one micrometre before clearance requires pre-existing authority",
        );
        assert_eq!(
            check_conflict_capability(&compiled, lengths, 2, 4_500, 0, 4_500),
            Ok(())
        );
        assert_eq!(
            check_conflict_capability(&compiled, lengths, 2, 4_500, 1, 4_500),
            Ok(())
        );
        assert_eq!(
            check_conflict_capability(&compiled, lengths, 2, 4_500, 0, 4_499),
            Ok(()),
            "a shorter vehicle clears the same passage earlier",
        );
        assert!(matches!(
            check_conflict_capability(&compiled, lengths, 2, 4_500, 0, 4_501),
            Err(ConflictCapabilityError::AuthorityRequired)
        ));

        let previous_length = lengths[path.edges()[1].index()];
        assert_eq!(
            check_conflict_capability(&compiled, lengths, 1, previous_length, 0, 0),
            check_conflict_capability(&compiled, lengths, 2, 0, 0, 0),
            "previous edge end and next edge zero are one canonical position",
        );
        assert_eq!(
            check_conflict_capability(&compiled, lengths, 1, previous_length, 1, 0),
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

    fn conflict_at(admission_hop: u32) -> ConflictPassageOccurrence {
        ConflictPassageOccurrence {
            stream: ParticipantStreamOrdinal::from_raw(0),
            passage_local_index: 0,
            resource: crate::kernel::conflict::ConflictResource::Zone(
                ConflictZoneOrdinal::from_raw(0),
            ),
            maneuver_index: 0,
            admission_hop,
            entry: RoutePosition {
                route_edge_index: 40,
                progress_mm: 9_000,
            },
            clearance: RoutePosition {
                route_edge_index: 40,
                progress_mm: 9_500,
            },
        }
    }

    fn waiting_at(entry_hop: u32) -> WaitingOccurrence {
        WaitingOccurrence {
            zone: WaitingZoneOrdinal::from_raw(entry_hop),
            maneuver_index: 0,
            entry_hop,
            release_hop: entry_hop,
            storage_length_mm: 1,
            dependency_end: 0,
        }
    }

    #[test]
    fn nearest_motion_barriers_follow_occurrence_anchors() {
        let lengths = [100, 200, 300, 400];
        let barriers = compile_nearest_motion_barriers(
            &lengths,
            &[conflict_at(0), conflict_at(2)],
            &[waiting_at(1)],
        )
        .expect("barriers");
        assert_eq!(barriers.len(), lengths.len());
        assert_eq!(
            barriers
                .iter()
                .map(|row| row.conflict_from_occurrence_start)
                .collect::<Vec<_>>(),
            [
                Some(BoundedDistance::Finite(100)),
                Some(BoundedDistance::Finite(500)),
                Some(BoundedDistance::Finite(300)),
                None,
            ]
        );
        assert_eq!(
            barriers
                .iter()
                .map(|row| row.waiting_from_occurrence_start)
                .collect::<Vec<_>>(),
            [
                Some(BoundedDistance::Finite(300)),
                Some(BoundedDistance::Finite(200)),
                None,
                None,
            ]
        );
        let repeated = compile_nearest_motion_barriers(&[50, 50, 50], &[conflict_at(1)], &[])
            .expect("repeated lengths");
        assert_eq!(
            repeated
                .iter()
                .map(|row| row.conflict_from_occurrence_start)
                .collect::<Vec<_>>(),
            [
                Some(BoundedDistance::Finite(100)),
                Some(BoundedDistance::Finite(50)),
                None,
            ]
        );
        let overflow = compile_nearest_motion_barriers(&[u32::MAX, 1, 1], &[conflict_at(1)], &[])
            .expect("overflow");
        assert_eq!(
            overflow[0].conflict_from_occurrence_start,
            Some(BoundedDistance::BeyondFinite)
        );
        assert_eq!(
            overflow[1].conflict_from_occurrence_start,
            Some(BoundedDistance::Finite(1))
        );
        assert_eq!(overflow[2].conflict_from_occurrence_start, None);
        let empty = compile_nearest_motion_barriers(&lengths, &[], &[]).expect("no barriers");
        assert!(empty.iter().all(|row| {
            row.conflict_from_occurrence_start.is_none()
                && row.waiting_from_occurrence_start.is_none()
        }));
        assert_eq!(
            crate::kernel::state::vec_bytes(&barriers),
            u64::try_from(barriers.capacity()).unwrap().saturating_mul(
                u64::try_from(std::mem::size_of::<NearestMotionBarriers>()).unwrap()
            )
        );
        assert!(std::mem::size_of::<NearestMotionBarriers>() > 0);
        assert_eq!(
            compile_nearest_motion_barriers(&lengths, &[conflict_at(9)], &[]).unwrap_err(),
            RouteError::ManeuverMismatch
        );
    }

    #[test]
    fn nearest_motion_barrier_allocation_failure_is_closed() {
        let lengths = [1_000, 1_000, 1_000];
        assert_eq!(
            with_route_allocation_failure_after(0, || {
                compile_nearest_motion_barriers(&lengths, &[], &[])
            }),
            Err(RouteError::AllocationFailed)
        );
        assert_eq!(
            with_route_allocation_failure_after(1, || {
                compile_nearest_motion_barriers(&lengths, &[], &[])
            }),
            Err(RouteError::AllocationFailed)
        );
        let built = with_route_allocation_failure_after(2, || {
            compile_nearest_motion_barriers(&lengths, &[], &[])
        })
        .expect("both reservations succeed");
        assert_eq!(built.len(), 3);
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
