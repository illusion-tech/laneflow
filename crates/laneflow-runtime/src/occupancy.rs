use laneflow_static_contract::{LaneEdgeOrdinal, MAX_VEHICLE_LENGTH_MM, MIN_LANE_EDGE_LENGTH_MM};
use laneflow_static_network::SharedNetworkRevision;

use crate::tables::{CompiledRoute, RouteSlot, VehicleSlot, for_each_occupancy_interval};
use crate::{RouteHandle, StepError, TrafficWorld, VehicleHandle, VehicleState, VehicleStatus};

#[cfg(test)]
use std::cell::Cell;

/// 占用桶键：物理边序号。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct OccupancyBucketOrdinal(u32);

impl OccupancyBucketOrdinal {
    const fn from_edge(edge: LaneEdgeOrdinal) -> Self {
        Self(edge.raw())
    }

    fn index(self) -> usize {
        usize::try_from(self.0).expect("occupancy bucket fits usize")
    }
}

/// 一条物理边上的占用片段，不是资源声明。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OccupancyRecord {
    vehicle: VehicleHandle,
    bucket: OccupancyBucketOrdinal,
    lo_mm: u32,
    hi_mm: u32,
    update_sequence: u32,
}

impl OccupancyRecord {
    const PLACEHOLDER: Self = Self {
        vehicle: VehicleHandle::new(0, 0),
        bucket: OccupancyBucketOrdinal(0),
        lo_mm: 0,
        hi_mm: 0,
        update_sequence: 0,
    };
}

/// 一辆车在合法最短边上最多覆盖的占用记录数。
///
/// 车身长度 `L`、边长 `E`、前杠不在格点时两端各有残段，最多触达 `L/E + 1` 条边。
const fn max_records_per_vehicle() -> usize {
    (MAX_VEHICLE_LENGTH_MM / MIN_LANE_EDGE_LENGTH_MM) as usize + 1
}

/// 后缀表用 `u32` 下标，并保留 `u32::MAX` 作空哨兵，因此占用记录数不得超过该值。
const SUFFIX_INDEX_LIMIT: usize = u32::MAX as usize;

pub(crate) fn occupancy_record_limit(vehicle_capacity: u32) -> usize {
    usize::try_from(vehicle_capacity)
        .unwrap_or(0)
        .saturating_mul(max_records_per_vehicle())
        .min(SUFFIX_INDEX_LIMIT)
}

const SUFFIX_NONE: u32 = u32::MAX;

/// 跟车查询的两个毫米窗：行走用 `front_query_mm`，接纳用 `bumper_gap_mm`。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LeaderQueryHorizon {
    /// §10.1 `bumper_gap_horizon`：后杠间隙大于此窗则本拍不接纳。
    pub bumper_gap_mm: u32,
    /// §10.1 `front_query_horizon`：后续出现项入口大于此窗则早停。
    pub front_query_mm: u32,
}

impl LeaderQueryHorizon {
    #[cfg(test)]
    pub(crate) const UNBOUNDED: Self = Self {
        bumper_gap_mm: u32::MAX,
        front_query_mm: u32::MAX,
    };

    pub(crate) const fn new(bumper_gap_mm: u32, front_query_mm: u32) -> Self {
        Self {
            bumper_gap_mm,
            front_query_mm,
        }
    }
}

fn try_reserve_len<T>(vec: &mut Vec<T>, needed: usize) -> Result<(), StepError> {
    vec.try_reserve(needed.saturating_sub(vec.len()))
        .map_err(|_| StepError::OccupancyAllocFailed)?;
    if vec.capacity() < needed {
        return Err(StepError::OccupancyAllocFailed);
    }
    Ok(())
}

fn occupancy_lo_key(record: &OccupancyRecord) -> (u32, u32, u32, u32) {
    (
        record.lo_mm,
        record.hi_mm,
        record.update_sequence,
        record.vehicle.index(),
    )
}

fn record_slot(index: usize) -> u32 {
    debug_assert!(
        index < SUFFIX_INDEX_LIMIT,
        "occupancy record index must stay below SUFFIX_NONE"
    );
    u32::try_from(index).expect("occupancy record index fits u32 below SUFFIX_NONE")
}

fn suffix_slot(slot: u32) -> Option<usize> {
    (slot != SUFFIX_NONE).then(|| usize::try_from(slot).expect("suffix slot fits usize"))
}

fn merge_suffix_pair(
    records: &[OccupancyRecord],
    current: usize,
    later_min: usize,
    later_second: Option<usize>,
) -> (usize, Option<usize>) {
    let current_rec = &records[current];
    let later_rec = &records[later_min];
    if occupancy_lo_key(current_rec) <= occupancy_lo_key(later_rec) {
        let second = if current_rec.vehicle != later_rec.vehicle {
            Some(later_min)
        } else {
            later_second
        };
        (current, second)
    } else {
        let second = match later_second {
            Some(idx)
                if occupancy_lo_key(&records[idx]) <= occupancy_lo_key(current_rec)
                    && records[idx].vehicle != later_rec.vehicle =>
            {
                Some(idx)
            }
            _ if current_rec.vehicle != later_rec.vehicle => Some(current),
            other => other.filter(|idx| records[*idx].vehicle != later_rec.vehicle),
        };
        (later_min, second)
    }
}

#[derive(Debug)]
pub(crate) struct OccupancyIndex {
    offsets: Vec<usize>,
    scratch: Vec<usize>,
    records: Vec<OccupancyRecord>,
    /// 与 `records` 对齐：`suffix_min_lo[i]` 是同桶 `[i, bucket_end)` 中 `lo_mm` 最小记录的下标。
    suffix_min_lo: Vec<u32>,
    /// 同后缀中车辆不同于最小值的次小 `lo_mm`，供 O(1) 排除 self。
    suffix_second_lo: Vec<u32>,
    #[cfg(test)]
    inspections: Cell<u64>,
    #[cfg(test)]
    occurrence_walks: Cell<u64>,
}

impl OccupancyIndex {
    /// 空构造（不预分配边级表）；全部增长走 try 路径，分配失败映射为
    /// `OccupancyAllocFailed` 而非中止进程。供切换事务暂存构造使用。
    pub(crate) fn try_empty() -> Result<Self, StepError> {
        let mut index = Self {
            offsets: Vec::new(),
            scratch: Vec::new(),
            records: Vec::new(),
            suffix_min_lo: Vec::new(),
            suffix_second_lo: Vec::new(),
            #[cfg(test)]
            inspections: Cell::new(0),
            #[cfg(test)]
            occurrence_walks: Cell::new(0),
        };
        index.try_prepare_scratch(0)?;
        Ok(index)
    }

    pub(crate) fn with_capacity(bucket_count: usize, record_capacity: usize) -> Self {
        Self {
            offsets: vec![0; bucket_count.saturating_add(1)],
            scratch: vec![0; bucket_count],
            records: Vec::with_capacity(record_capacity),
            suffix_min_lo: Vec::with_capacity(record_capacity),
            suffix_second_lo: Vec::with_capacity(record_capacity),
            #[cfg(test)]
            inspections: Cell::new(0),
            #[cfg(test)]
            occurrence_walks: Cell::new(0),
        }
    }

    #[cfg(test)]
    pub(crate) fn inspections(&self) -> u64 {
        self.inspections.get()
    }

    #[cfg(test)]
    pub(crate) fn occurrence_walks(&self) -> u64 {
        self.occurrence_walks.get()
    }

    #[cfg(test)]
    pub(crate) fn records_capacity(&self) -> usize {
        self.records.capacity()
    }

    #[cfg(test)]
    pub(crate) fn records_len(&self) -> usize {
        self.records.len()
    }

    #[cfg(test)]
    pub(crate) fn scratch_capacity(&self) -> usize {
        self.scratch.capacity()
    }

    #[cfg(test)]
    pub(crate) fn offsets_capacity(&self) -> usize {
        self.offsets.capacity()
    }

    #[cfg(test)]
    pub(crate) fn suffix_min_lo_capacity(&self) -> usize {
        self.suffix_min_lo.capacity()
    }

    #[cfg(test)]
    pub(crate) fn suffix_second_lo_capacity(&self) -> usize {
        self.suffix_second_lo.capacity()
    }

    fn note_inspection(&self) {
        #[cfg(test)]
        self.inspections
            .set(self.inspections.get().saturating_add(1));
    }

    fn note_occurrence_walk(&self) {
        #[cfg(test)]
        self.occurrence_walks
            .set(self.occurrence_walks.get().saturating_add(1));
    }

    #[cfg(test)]
    fn reset_inspections(&self) {
        self.inspections.set(0);
        self.occurrence_walks.set(0);
    }

    #[cfg(test)]
    fn rebuild_from_pending(&mut self, pending: &[OccupancyRecord], bucket_count: usize) {
        self.reset_inspections();
        self.try_prepare_scratch(bucket_count)
            .expect("test occupancy scratch");
        for record in pending {
            if let Some(count) = self.scratch.get_mut(record.bucket.index()) {
                *count += 1;
            }
        }
        let total = self.record_total(bucket_count);
        self.try_reserve_records(total)
            .expect("test occupancy records");
        self.finish_layout(bucket_count);
        for record in pending {
            self.write_record(*record);
        }
        self.sort_buckets(bucket_count);
    }

    fn record_total(&self, bucket_count: usize) -> usize {
        self.scratch.iter().take(bucket_count).copied().sum()
    }

    fn try_prepare_scratch(&mut self, bucket_count: usize) -> Result<(), StepError> {
        try_reserve_len(&mut self.offsets, bucket_count.saturating_add(1))?;
        try_reserve_len(&mut self.scratch, bucket_count)?;
        self.scratch.clear();
        self.scratch.resize(bucket_count, 0);
        Ok(())
    }

    fn try_reserve_records(&mut self, needed: usize) -> Result<(), StepError> {
        try_reserve_len(&mut self.records, needed)?;
        try_reserve_len(&mut self.suffix_min_lo, needed)?;
        try_reserve_len(&mut self.suffix_second_lo, needed)?;
        Ok(())
    }

    fn finish_layout(&mut self, bucket_count: usize) {
        self.offsets.clear();
        self.offsets.resize(bucket_count.saturating_add(1), 0);
        for index in 0..bucket_count {
            self.offsets[index + 1] = self.offsets[index].saturating_add(self.scratch[index]);
        }
        let total = self.offsets.get(bucket_count).copied().unwrap_or(0);
        debug_assert!(total <= self.records.capacity());
        self.records.clear();
        self.records.resize(total, OccupancyRecord::PLACEHOLDER);
        self.suffix_min_lo.clear();
        self.suffix_min_lo.resize(total, 0);
        self.suffix_second_lo.clear();
        self.suffix_second_lo.resize(total, SUFFIX_NONE);
        self.scratch.clear();
        if bucket_count == 0 {
            return;
        }
        self.scratch
            .extend_from_slice(&self.offsets[..bucket_count]);
    }

    fn write_record(&mut self, record: OccupancyRecord) {
        let bucket = record.bucket.index();
        let Some(head) = self.scratch.get_mut(bucket) else {
            return;
        };
        let slot = *head;
        if let Some(target) = self.records.get_mut(slot) {
            *target = record;
            *head = slot.saturating_add(1);
        }
    }

    fn sort_buckets(&mut self, bucket_count: usize) {
        for bucket in 0..bucket_count {
            let start = self.offsets[bucket];
            let end = self.offsets[bucket + 1];
            self.records[start..end].sort_unstable_by_key(|record| {
                (
                    record.hi_mm,
                    record.lo_mm,
                    record.update_sequence,
                    record.vehicle.index(),
                )
            });
            self.fill_suffix_min_lo(start, end);
        }
    }

    fn fill_suffix_min_lo(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        let last = end - 1;
        self.suffix_min_lo[last] = record_slot(last);
        self.suffix_second_lo[last] = SUFFIX_NONE;
        for index in (start..last).rev() {
            let later_min = usize::try_from(self.suffix_min_lo[index + 1])
                .expect("suffix min index fits usize");
            let later_second = suffix_slot(self.suffix_second_lo[index + 1]);
            let (best, second) = merge_suffix_pair(&self.records, index, later_min, later_second);
            self.suffix_min_lo[index] = record_slot(best);
            self.suffix_second_lo[index] = second.map_or(SUFFIX_NONE, record_slot);
        }
    }

    fn bucket_span(&self, edge: LaneEdgeOrdinal) -> (usize, usize) {
        let index = OccupancyBucketOrdinal::from_edge(edge).index();
        let Some(start) = self.offsets.get(index).copied() else {
            return (0, 0);
        };
        let Some(end) = self.offsets.get(index + 1).copied() else {
            return (0, 0);
        };
        let end = end.min(self.records.len());
        let start = start.min(end);
        (start, end)
    }

    fn min_lo_from(
        &self,
        start: usize,
        end: usize,
        skip: VehicleHandle,
    ) -> Option<OccupancyRecord> {
        if start >= end {
            return None;
        }
        self.note_inspection();
        let pick = usize::try_from(*self.suffix_min_lo.get(start)?)
            .ok()
            .filter(|index| *index < self.records.len())?;
        let record = *self.records.get(pick)?;
        if record.vehicle != skip {
            return Some(record);
        }
        self.note_inspection();
        let second = suffix_slot(*self.suffix_second_lo.get(start)?)?;
        self.records.get(second).copied()
    }

    fn nearest_ahead(
        &self,
        edge: LaneEdgeOrdinal,
        self_vehicle: VehicleHandle,
        front_mm: u32,
    ) -> Option<OccupancyRecord> {
        let (start, end) = self.bucket_span(edge);
        let skip = self.records[start..end].partition_point(|record| record.hi_mm <= front_mm);
        self.min_lo_from(start.saturating_add(skip), end, self_vehicle)
    }

    fn front_most(
        &self,
        edge: LaneEdgeOrdinal,
        self_vehicle: VehicleHandle,
    ) -> Option<OccupancyRecord> {
        let (start, end) = self.bucket_span(edge);
        self.min_lo_from(start, end, self_vehicle)
    }

    /// 前保险杠到后杠间隙窗内最近前车后保险杠的 `i64` 毫米间隙；可负。
    ///
    /// 当前边取后缀最小 `lo_mm`。后续出现项按入口距离走到 `front_query_mm`（含端点）；
    /// 接纳只看 `bumper_gap_mm`。后杠间隙窗外本拍无 leader。
    pub(crate) fn leader_gap(
        &self,
        self_vehicle: VehicleHandle,
        follower_edges: &[LaneEdgeOrdinal],
        follower_index: usize,
        follower_progress: u32,
        lengths: &[u32],
        horizon: LeaderQueryHorizon,
    ) -> Option<i64> {
        let walk = i64::from(horizon.front_query_mm);
        let accept = i64::from(horizon.bumper_gap_mm);
        let current = *follower_edges.get(follower_index)?;
        let mut best = self
            .nearest_ahead(current, self_vehicle, follower_progress)
            .map(|record| i64::from(record.lo_mm) - i64::from(follower_progress))
            .filter(|gap| *gap <= accept);
        let Some(current_length) = lengths.get(current.index()).copied() else {
            return best;
        };
        let mut base_mm = i64::from(current_length) - i64::from(follower_progress);
        for edge in follower_edges
            .iter()
            .copied()
            .skip(follower_index.saturating_add(1))
        {
            if base_mm > walk {
                break;
            }
            if best.is_some_and(|current_gap| base_mm > current_gap) {
                break;
            }
            self.note_occurrence_walk();
            if let Some(record) = self.front_most(edge, self_vehicle)
                && let Some(gap) = base_mm
                    .checked_add(i64::from(record.lo_mm))
                    .filter(|gap| *gap <= accept)
            {
                best = Some(best.map_or(gap, |current_gap| current_gap.min(gap)));
            }
            let Some(edge_length) = lengths.get(edge.index()).copied() else {
                return best;
            };
            let Some(next_base) = base_mm.checked_add(i64::from(edge_length)) else {
                return best;
            };
            base_mm = next_base;
        }
        best
    }
}

fn vehicle_state_in(vehicles: &[VehicleSlot], handle: VehicleHandle) -> Option<&VehicleState> {
    let slot = vehicles.get(usize::try_from(handle.index()).ok()?)?;
    if slot.generation != handle.generation() {
        return None;
    }
    slot.state.as_ref()
}

fn route_edges_in(routes: &[RouteSlot], route: RouteHandle) -> Option<&[LaneEdgeOrdinal]> {
    let slot = routes.get(usize::try_from(route.index()).ok()?)?;
    if slot.generation != route.generation() {
        return None;
    }
    Some(slot.compiled.as_ref()?.edges.as_ref())
}

fn visit_occupancy_records_with<'a>(
    live_order: &[VehicleHandle],
    vehicles: &[VehicleSlot],
    revision: &SharedNetworkRevision,
    routes: &[RouteSlot],
    staged_by_slot: &[Option<&'a CompiledRoute>],
    mut visit: impl FnMut(OccupancyRecord),
) -> Result<(), StepError> {
    let lengths = revision.traffic().lane_lengths_millimetres();
    for (sequence, handle) in live_order.iter().copied().enumerate() {
        let Some(state) = vehicle_state_in(vehicles, handle) else {
            continue;
        };
        if state.status != VehicleStatus::Active {
            continue;
        }
        let staged_edges = usize::try_from(state.route.index())
            .ok()
            .and_then(|slot| staged_by_slot.get(slot).copied().flatten())
            .map(|compiled| compiled.edges.as_ref());
        let Some(edges) = staged_edges.or_else(|| route_edges_in(routes, state.route)) else {
            continue;
        };
        let Ok(index) = usize::try_from(state.route_edge_index) else {
            return Err(StepError::OccupancyIntervalIncomplete);
        };
        let Ok(update_sequence) = u32::try_from(sequence) else {
            return Err(StepError::OccupancyIntervalIncomplete);
        };
        for_each_occupancy_interval(
            lengths,
            edges,
            index,
            state.progress_mm,
            state.length_mm,
            |edge, lo_mm, hi_mm| {
                visit(OccupancyRecord {
                    vehicle: handle,
                    bucket: OccupancyBucketOrdinal::from_edge(edge),
                    lo_mm,
                    hi_mm,
                    update_sequence,
                });
            },
        )
        .ok_or(StepError::OccupancyIntervalIncomplete)?;
    }
    Ok(())
}

fn visit_occupancy_records(
    live_order: &[VehicleHandle],
    vehicles: &[VehicleSlot],
    revision: &SharedNetworkRevision,
    routes: &[RouteSlot],
    mut visit: impl FnMut(OccupancyRecord),
) -> Result<(), StepError> {
    let lengths = revision.traffic().lane_lengths_millimetres();
    for (sequence, handle) in live_order.iter().copied().enumerate() {
        let Some(state) = vehicle_state_in(vehicles, handle) else {
            continue;
        };
        if state.status != VehicleStatus::Active {
            continue;
        }
        let Some(edges) = route_edges_in(routes, state.route) else {
            continue;
        };
        let Ok(index) = usize::try_from(state.route_edge_index) else {
            return Err(StepError::OccupancyIntervalIncomplete);
        };
        let Ok(update_sequence) = u32::try_from(sequence) else {
            return Err(StepError::OccupancyIntervalIncomplete);
        };
        for_each_occupancy_interval(
            lengths,
            edges,
            index,
            state.progress_mm,
            state.length_mm,
            |edge, lo_mm, hi_mm| {
                visit(OccupancyRecord {
                    vehicle: handle,
                    bucket: OccupancyBucketOrdinal::from_edge(edge),
                    lo_mm,
                    hi_mm,
                    update_sequence,
                });
            },
        )
        .ok_or(StepError::OccupancyIntervalIncomplete)?;
    }
    Ok(())
}

impl TrafficWorld {
    /// 针对给定根与 staged 路线纯构造一份占用索引（不触及活动状态）。
    ///
    /// 供切换事务在 Prepare 段完成可失败的重建（#302 切换合同 §4：
    /// 全部可失败步骤先于换绑），commit 段只做不可失败的替换。
    pub(crate) fn build_occupancy_index_for(
        &self,
        revision: &SharedNetworkRevision,
        routes_staged: &[(usize, CompiledRoute)],
    ) -> Result<OccupancyIndex, StepError> {
        let bucket_count = usize::try_from(revision.traffic().lane_edge_count())
            .expect("lane edge count fits usize");
        let ceiling = occupancy_record_limit(self.config.vehicle_capacity());
        let mut staged = OccupancyIndex::try_empty()?;
        staged.try_prepare_scratch(bucket_count)?;
        let mut staged_by_slot: Vec<Option<&CompiledRoute>> = Vec::new();
        try_reserve_len(&mut staged_by_slot, self.routes.len())?;
        staged_by_slot.resize(self.routes.len(), None);
        for (index, compiled) in routes_staged {
            if let Some(slot) = staged_by_slot.get_mut(*index) {
                *slot = Some(compiled);
            }
        }
        visit_occupancy_records_with(
            &self.live_order,
            &self.vehicles,
            revision,
            &self.routes,
            &staged_by_slot,
            |record| {
                if let Some(count) = staged.scratch.get_mut(record.bucket.index()) {
                    *count += 1;
                }
            },
        )?;
        let total = staged.record_total(bucket_count);
        if total > ceiling {
            return Err(StepError::OccupancyCapacityExceeded);
        }
        staged.try_reserve_records(total)?;
        staged.finish_layout(bucket_count);
        visit_occupancy_records_with(
            &self.live_order,
            &self.vehicles,
            revision,
            &self.routes,
            &staged_by_slot,
            |record| staged.write_record(record),
        )?;
        staged.sort_buckets(bucket_count);
        Ok(staged)
    }

    pub(crate) fn rebuild_occupancy_index(&mut self) -> Result<(), StepError> {
        let bucket_count = usize::try_from(self.revision.traffic().lane_edge_count())
            .expect("lane edge count fits usize");
        let ceiling = occupancy_record_limit(self.config.vehicle_capacity());
        let occupancy = &mut self.occupancy;
        #[cfg(test)]
        occupancy.reset_inspections();
        occupancy.try_prepare_scratch(bucket_count)?;
        visit_occupancy_records(
            &self.live_order,
            &self.vehicles,
            &self.revision,
            &self.routes,
            |record| {
                if let Some(count) = occupancy.scratch.get_mut(record.bucket.index()) {
                    *count += 1;
                }
            },
        )?;
        let total = occupancy.record_total(bucket_count);
        if total > ceiling {
            return Err(StepError::OccupancyCapacityExceeded);
        }
        occupancy.try_reserve_records(total)?;
        occupancy.finish_layout(bucket_count);
        visit_occupancy_records(
            &self.live_order,
            &self.vehicles,
            &self.revision,
            &self.routes,
            |record| occupancy.write_record(record),
        )?;
        occupancy.sort_buckets(bucket_count);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn occupancy_inspections(&self) -> u64 {
        self.occupancy.inspections()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use laneflow_compiler::{
        CompilationUnitBuilder, CompileLimits, Compiler, IidmVehicleProfileInput, LaneEdgeInput,
        ParticipantClassInput, ParticipantClassReference, PortableDiffBase,
        PortableEmissionProvenance, SourceModuleHeader, SourceModuleHeaderInput,
        SyntheticModuleBuilder, VehicleProfileInput, emit_portable_candidate,
    };
    use laneflow_format::{
        FormatLimits, check_canonical_network_input, check_post_emission_bundle,
    };
    use laneflow_static_contract::{ParkingSpaceOrdinal, VehicleProfileOrdinal};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision,
        SpatialBuildOption, build_shared_network_revision,
    };

    use crate::tables::{occupancy_front_gap, remaining_along_route_i64};
    use crate::tick::leader_query_horizon;
    use crate::units::ceil_mm;
    use crate::{
        RouteError, RouteRegisterInput, StepError, TickInput, TrafficWorld, VehicleSpawnInput,
        WorldConfig,
    };

    fn install_fixture(
        revision: std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
        config: crate::WorldConfig,
    ) -> Result<crate::TrafficWorld, crate::InstallError> {
        let origin = *revision.canonical_origin();
        crate::TrafficWorld::install(
            revision,
            config,
            crate::CommittedNetworkSource::Published {
                reference: crate::PublishedLfcaReference::new(
                    "fixture://in-process",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .expect("non-empty fixture key"),
            },
            0,
        )
    }

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
    );

    fn edge_for_length(world: &TrafficWorld, length: u32) -> LaneEdgeOrdinal {
        let index = world
            .traffic()
            .lane_lengths_millimetres()
            .iter()
            .position(|actual| *actual == length)
            .expect("fixture lane length");
        LaneEdgeOrdinal::from_raw(u32::try_from(index).expect("ordinal"))
    }

    fn register_full_spatial_route(world: &mut TrafficWorld) -> crate::RouteHandle {
        world
            .register_route(RouteRegisterInput::new(vec![
                edge_for_length(world, 10_000),
                edge_for_length(world, 8_000),
                edge_for_length(world, 12_000),
            ]))
            .expect("register full-spatial route")
    }

    fn iidm() -> IidmVehicleProfileInput {
        IidmVehicleProfileInput {
            length_meters: 4.5,
            desired_speed_meters_per_second: 13.75,
            min_gap_meters: 2.0,
            time_headway_seconds: 1.4,
            max_acceleration_meters_per_second_squared: 1.8,
            comfortable_deceleration_meters_per_second_squared: 2.0,
            emergency_deceleration_meters_per_second_squared: 4.5,
        }
    }

    fn compile_revision(
        configure: impl FnOnce(&mut SyntheticModuleBuilder),
    ) -> Arc<SharedNetworkRevision> {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: "city/occupancy-index",
                source_document_key: "occupancy-index.document",
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: Some(42),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .expect("source header");
        let mut module = SyntheticModuleBuilder::new(header, &limits).expect("synthetic module");
        configure(&mut module);
        let mut unit = CompilationUnitBuilder::new(limits);
        unit.add_synthetic_module(module.finish().expect("finished module"))
            .expect("compilation module");
        let output = Compiler::new()
            .compile(unit.build().expect("compilation unit"))
            .expect("compiled output");
        let provenance = PortableEmissionProvenance::try_new("laneflow-occupancy-index-v1")
            .expect("portable provenance");
        let candidate = emit_portable_candidate(
            &output,
            &provenance,
            FormatLimits::HARD,
            PortableDiffBase::Genesis,
        )
        .expect("portable candidate");
        let checked = check_post_emission_bundle(
            candidate.canonical_artifact().bytes(),
            candidate.source_map().bytes(),
            candidate.semantic_diff().bytes(),
            candidate.expected_semantic_diff_base(),
            FormatLimits::HARD,
        )
        .expect("post-emission checked bundle");
        build_shared_network_revision(
            checked.canonical_network_input(),
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::Omit,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("shared network revision")
    }

    fn add_car_profile(module: &mut SyntheticModuleBuilder) {
        module
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "road-user",
                extends: None,
            })
            .expect("class")
            .add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: "car",
                participant_class: ParticipantClassReference::local("road-user"),
                iidm: iidm(),
            })
            .expect("profile");
    }

    fn two_edge_revision() -> Arc<SharedNetworkRevision> {
        compile_revision(|module| {
            add_car_profile(module);
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "stem",
                    length_meters: 20.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[laneflow_compiler::LaneEdgeReference::local("tail")],
                })
                .expect("stem")
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "tail",
                    length_meters: 20.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[],
                })
                .expect("tail");
        })
    }

    fn loop_revision() -> Arc<SharedNetworkRevision> {
        compile_revision(|module| {
            add_car_profile(module);
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "loop-a",
                    length_meters: 20.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[laneflow_compiler::LaneEdgeReference::local("loop-b")],
                })
                .expect("a")
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "loop-b",
                    length_meters: 20.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[laneflow_compiler::LaneEdgeReference::local("loop-a")],
                })
                .expect("b");
        })
    }

    fn index_gap(world: &TrafficWorld, state: &VehicleState) -> Option<i64> {
        let lengths = world.revision.traffic().lane_lengths_millimetres();
        let edges = world.route_edges(state.route).unwrap();
        world.leader_bumper_gap(state, edges, lengths)
    }

    fn assert_index_matches_scan(world: &TrafficWorld) {
        let lengths = world.revision.traffic().lane_lengths_millimetres();
        for handle in world.live_order.iter().copied() {
            let Some(state) = world.vehicle_state(handle) else {
                continue;
            };
            if state.status != VehicleStatus::Active {
                continue;
            }
            let Some(edges) = world.route_edges(state.route) else {
                continue;
            };
            let cursor = usize::try_from(state.route_edge_index).unwrap();
            let horizon = world.leader_query_horizon_for(state);
            let indexed = world.occupancy.leader_gap(
                state.handle,
                edges,
                cursor,
                state.progress_mm,
                lengths,
                horizon,
            );
            let scanned = world.leader_bumper_gap_scan(state, edges, lengths);
            let wrapped = world.leader_bumper_gap(state, edges, lengths);
            assert_eq!(
                indexed, scanned,
                "occupancy index gap must match scan-within-horizon for {handle:?}"
            );
            assert_eq!(
                wrapped, indexed,
                "leader_bumper_gap must use the occupancy index for {handle:?}"
            );
        }
    }

    fn active_count(world: &TrafficWorld) -> u64 {
        world
            .live_order
            .iter()
            .copied()
            .filter(|handle| {
                world
                    .vehicle_state(*handle)
                    .is_some_and(|state| state.status == VehicleStatus::Active)
            })
            .count() as u64
    }

    #[test]
    fn unaligned_body_span_fits_planned_record_limit() {
        assert_eq!(max_records_per_vehicle(), 1_281);
        assert_eq!(occupancy_record_limit(1), 1_281);
    }

    #[test]
    fn occupancy_record_limit_fits_suffix_u32_slots() {
        let per_vehicle = max_records_per_vehicle();
        let max_uncapped_vehicles = SUFFIX_INDEX_LIMIT / per_vehicle;
        let max_uncapped = u32::try_from(max_uncapped_vehicles).expect("fits u32");
        assert_eq!(
            occupancy_record_limit(max_uncapped),
            max_uncapped_vehicles.saturating_mul(per_vehicle)
        );
        assert_eq!(
            occupancy_record_limit(max_uncapped.saturating_add(1)),
            SUFFIX_INDEX_LIMIT
        );
        assert_eq!(occupancy_record_limit(u32::MAX), SUFFIX_INDEX_LIMIT);
    }

    #[test]
    fn try_reserve_len_grows_from_spare_capacity() {
        let mut values = Vec::<u32>::with_capacity(4);
        values.push(1);
        assert!(values.len() < values.capacity());
        let needed = values.capacity() + 1;
        try_reserve_len(&mut values, needed).expect("reserve relative to len");
        assert!(
            values.capacity() >= needed,
            "capacity={} needed={needed}",
            values.capacity()
        );
        assert_eq!(values.len(), 1);
    }

    #[test]
    fn skipping_min_lo_self_stays_constant_time() {
        let current = LaneEdgeOrdinal::from_raw(0);
        let later = LaneEdgeOrdinal::from_raw(1);
        let follower = VehicleHandle::new(0, 0);
        let mut pending = vec![OccupancyRecord {
            vehicle: follower,
            bucket: OccupancyBucketOrdinal::from_edge(current),
            lo_mm: 0,
            hi_mm: 1_000,
            update_sequence: 0,
        }];
        for index in 1..=32_u32 {
            pending.push(OccupancyRecord {
                vehicle: VehicleHandle::new(index, 0),
                bucket: OccupancyBucketOrdinal::from_edge(current),
                lo_mm: 1_000 * index,
                hi_mm: 1_000 * index + 500,
                update_sequence: index,
            });
        }
        let mut occupancy = OccupancyIndex::with_capacity(2, 40);
        occupancy.rebuild_from_pending(&pending, 2);
        occupancy.reset_inspections();
        let lengths = [40_000, 10_000];
        let edges = [current, later, current];
        let gap = occupancy.leader_gap(
            follower,
            &edges,
            0,
            1_000,
            &lengths,
            LeaderQueryHorizon::UNBOUNDED,
        );
        assert_eq!(gap, Some(0));
        let inspections = occupancy.inspections();
        assert!(
            inspections <= 8,
            "skipping the min-lo self record must stay O(1), inspections={inspections}"
        );
    }

    #[test]
    fn nearest_ahead_skips_self_and_uses_rear_bumper() {
        let edge = LaneEdgeOrdinal::from_raw(0);
        let follower = VehicleHandle::new(0, 0);
        let leader = VehicleHandle::new(1, 0);
        let mut index = OccupancyIndex::with_capacity(1, 2);
        let pending = vec![
            OccupancyRecord {
                vehicle: follower,
                bucket: OccupancyBucketOrdinal::from_edge(edge),
                lo_mm: 0,
                hi_mm: 1_000,
                update_sequence: 0,
            },
            OccupancyRecord {
                vehicle: leader,
                bucket: OccupancyBucketOrdinal::from_edge(edge),
                lo_mm: 6_000,
                hi_mm: 8_000,
                update_sequence: 1,
            },
        ];
        index.rebuild_from_pending(&pending, 1);
        let gap = index.leader_gap(
            follower,
            &[edge],
            0,
            1_000,
            &[10_000],
            LeaderQueryHorizon::UNBOUNDED,
        );
        assert_eq!(gap, Some(5_000));
    }

    #[test]
    fn overlapping_records_use_smallest_rear_bumper() {
        let edge = LaneEdgeOrdinal::from_raw(0);
        let follower = VehicleHandle::new(0, 0);
        let short = VehicleHandle::new(1, 0);
        let mid = VehicleHandle::new(2, 0);
        let long = VehicleHandle::new(3, 0);
        let mut index = OccupancyIndex::with_capacity(1, 4);
        let pending = vec![
            OccupancyRecord {
                vehicle: follower,
                bucket: OccupancyBucketOrdinal::from_edge(edge),
                lo_mm: 0,
                hi_mm: 1_000,
                update_sequence: 0,
            },
            OccupancyRecord {
                vehicle: short,
                bucket: OccupancyBucketOrdinal::from_edge(edge),
                lo_mm: 5_000,
                hi_mm: 7_000,
                update_sequence: 1,
            },
            OccupancyRecord {
                vehicle: mid,
                bucket: OccupancyBucketOrdinal::from_edge(edge),
                lo_mm: 6_000,
                hi_mm: 7_500,
                update_sequence: 2,
            },
            OccupancyRecord {
                vehicle: long,
                bucket: OccupancyBucketOrdinal::from_edge(edge),
                lo_mm: 2_000,
                hi_mm: 8_000,
                update_sequence: 3,
            },
        ];
        index.rebuild_from_pending(&pending, 1);
        let gap = index.leader_gap(
            follower,
            &[edge],
            0,
            1_000,
            &[10_000],
            LeaderQueryHorizon::UNBOUNDED,
        );
        assert_eq!(gap, Some(1_000));
    }

    #[test]
    fn overlapping_downstream_records_use_smallest_rear_bumper() {
        let first = LaneEdgeOrdinal::from_raw(0);
        let second = LaneEdgeOrdinal::from_raw(1);
        let follower = VehicleHandle::new(0, 0);
        let short = VehicleHandle::new(1, 0);
        let long = VehicleHandle::new(2, 0);
        let mut index = OccupancyIndex::with_capacity(2, 3);
        let pending = vec![
            OccupancyRecord {
                vehicle: follower,
                bucket: OccupancyBucketOrdinal::from_edge(first),
                lo_mm: 8_000,
                hi_mm: 9_000,
                update_sequence: 0,
            },
            OccupancyRecord {
                vehicle: short,
                bucket: OccupancyBucketOrdinal::from_edge(second),
                lo_mm: 5_000,
                hi_mm: 7_000,
                update_sequence: 1,
            },
            OccupancyRecord {
                vehicle: long,
                bucket: OccupancyBucketOrdinal::from_edge(second),
                lo_mm: 2_000,
                hi_mm: 8_000,
                update_sequence: 2,
            },
        ];
        index.rebuild_from_pending(&pending, 2);
        let lengths = [10_000, 10_000];
        let edges = [first, second];
        let gap = index.leader_gap(
            follower,
            &edges,
            0,
            9_000,
            &lengths,
            LeaderQueryHorizon::UNBOUNDED,
        );
        assert_eq!(
            gap,
            remaining_along_route_i64(&lengths, &edges, 0, 9_000, 1, 2_000)
        );
    }

    #[test]
    fn later_occurrence_uses_front_most_record() {
        let first = LaneEdgeOrdinal::from_raw(0);
        let second = LaneEdgeOrdinal::from_raw(1);
        let follower = VehicleHandle::new(0, 0);
        let leader = VehicleHandle::new(1, 0);
        let mut index = OccupancyIndex::with_capacity(2, 2);
        let pending = vec![
            OccupancyRecord {
                vehicle: follower,
                bucket: OccupancyBucketOrdinal::from_edge(first),
                lo_mm: 8_000,
                hi_mm: 9_000,
                update_sequence: 0,
            },
            OccupancyRecord {
                vehicle: leader,
                bucket: OccupancyBucketOrdinal::from_edge(second),
                lo_mm: 500,
                hi_mm: 1_500,
                update_sequence: 1,
            },
        ];
        index.rebuild_from_pending(&pending, 2);
        let lengths = [10_000, 5_000];
        let edges = [first, second];
        let gap = index.leader_gap(
            follower,
            &edges,
            0,
            9_000,
            &lengths,
            LeaderQueryHorizon::UNBOUNDED,
        );
        assert_eq!(
            gap,
            remaining_along_route_i64(&lengths, &edges, 0, 9_000, 1, 500)
        );
    }

    #[test]
    fn full_spatial_follower_matches_scan_oracle() {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).unwrap();
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .unwrap();
        let mut world = install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 100)).unwrap();
        let route = register_full_spatial_route(&mut world);
        let profile = world
            .revision
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .unwrap();
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000 + profile.length_mm() + profile.min_gap_mm() + 2_000,
                0,
            ))
            .unwrap();
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .unwrap();
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        assert_index_matches_scan(&world);
        world.step(TickInput::new(100)).unwrap();
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        assert_index_matches_scan(&world);
    }

    #[test]
    fn empty_and_solo_vehicle_have_no_leader() {
        let revision = two_edge_revision();
        let stem = LaneEdgeOrdinal::from_raw(0);
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 100)).expect("install");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        world.step(TickInput::new(100)).unwrap();
        let route = world
            .register_route(RouteRegisterInput::new(vec![stem]))
            .expect("route");
        let solo = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("solo");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        let state = world.vehicle_state(solo).copied().unwrap();
        assert_eq!(index_gap(&world, &state), None);
        assert_index_matches_scan(&world);
    }

    #[test]
    fn vehicle_behind_on_current_edge_is_not_leader() {
        let revision = two_edge_revision();
        let stem = LaneEdgeOrdinal::from_raw(0);
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 100)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![stem]))
            .expect("route");
        let profile = world
            .revision
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .unwrap();
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("behind");
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000 + profile.length_mm() + profile.min_gap_mm() + 2_000,
                0,
            ))
            .expect("ahead");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        let state = world.vehicle_state(follower).copied().unwrap();
        assert_eq!(index_gap(&world, &state), None);
        assert_index_matches_scan(&world);
    }

    #[test]
    fn leader_fully_on_next_edge_matches_scan() {
        let revision = two_edge_revision();
        let stem = LaneEdgeOrdinal::from_raw(0);
        let tail = LaneEdgeOrdinal::from_raw(1);
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 100)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![stem, tail]))
            .expect("route");
        let profile = world
            .revision
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .unwrap();
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                1,
                profile.length_mm() + 1_000,
                0,
            ))
            .expect("leader on tail");
        let stem_len = world.revision.traffic().lane_lengths_millimetres()[stem.index()];
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                stem_len.saturating_sub(1_000),
                0,
            ))
            .expect("follower on stem");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        assert_index_matches_scan(&world);
        let state = world.vehicle_state(follower).copied().unwrap();
        let gap = index_gap(&world, &state).expect("next-edge leader inside bumper window");
        assert!(gap > 0, "next-edge rear bumper must be ahead, gap={gap}");
    }

    #[test]
    fn cycle_wrap_uses_later_occurrence_of_vehicle_behind() {
        let revision = loop_revision();
        let a = LaneEdgeOrdinal::from_raw(0);
        let b = LaneEdgeOrdinal::from_raw(1);
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 100)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![a, b, a]))
            .expect("route");
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("physically behind");
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                9_000,
                0,
            ))
            .expect("near end of first a");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        assert_index_matches_scan(&world);
        let state = world.vehicle_state(follower).copied().unwrap();
        assert_eq!(
            index_gap(&world, &state),
            None,
            "wrap gap is tens of metres, beyond rest bumper_gap_horizon"
        );
        let lengths = world.revision.traffic().lane_lengths_millimetres();
        let edges = world.route_edges(state.route).unwrap();
        let cursor = usize::try_from(state.route_edge_index).unwrap();
        let unbounded = world.occupancy.leader_gap(
            state.handle,
            edges,
            cursor,
            state.progress_mm,
            lengths,
            LeaderQueryHorizon::UNBOUNDED,
        );
        let gap = unbounded.expect("wrap-around leader without bumper filter");
        assert!(
            gap > 20_000,
            "leader behind on the current occurrence must be found via the next a, gap={gap}"
        );
    }

    #[test]
    fn route_edge_occurrence_capacity_counts_repeats_and_releases_only_on_success() {
        let revision = loop_revision();
        let a = LaneEdgeOrdinal::from_raw(0);
        let b = LaneEdgeOrdinal::from_raw(1);
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 3, 1, 100)).expect("install");

        let route = world
            .register_route(RouteRegisterInput::new(vec![a, b, a]))
            .expect("three occurrences exactly fill capacity");
        assert_eq!(world.live_route_count, 1);
        assert_eq!(world.live_route_edge_occurrence_count, 3);
        let route_slots = world.routes.len();

        assert_eq!(
            world
                .register_route(RouteRegisterInput::new(vec![a]))
                .unwrap_err(),
            RouteError::EdgeOccurrenceCapacityExceeded
        );
        assert_eq!(world.live_route_count, 1);
        assert_eq!(world.live_route_edge_occurrence_count, 3);
        assert_eq!(world.routes.len(), route_slots);

        world
            .remove_route(route)
            .expect("unused route releases all occurrences");
        assert_eq!(world.live_route_count, 0);
        assert_eq!(world.live_route_edge_occurrence_count, 0);

        let route = world
            .register_route(RouteRegisterInput::new(vec![a, b, a]))
            .expect("released capacity can be reused");
        let vehicle = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                0,
                0,
            ))
            .expect("spawn");
        assert_eq!(
            world.remove_route(route).unwrap_err(),
            RouteError::InUse { vehicle, route }
        );
        assert_eq!(world.live_route_edge_occurrence_count, 3);
        assert_eq!(
            world
                .register_route(RouteRegisterInput::new(vec![a]))
                .unwrap_err(),
            RouteError::EdgeOccurrenceCapacityExceeded
        );
    }

    #[test]
    fn route_registration_preflight_has_stable_error_priority() {
        let revision = loop_revision();
        let a = LaneEdgeOrdinal::from_raw(0);
        let mut no_route_slots =
            install_fixture(Arc::clone(&revision), WorldConfig::new(8, 0, 0, 1, 100))
                .expect("install");

        assert_eq!(
            no_route_slots
                .register_route(RouteRegisterInput::new(Vec::new()))
                .unwrap_err(),
            RouteError::EmptySequence
        );
        assert_eq!(
            no_route_slots
                .register_route(RouteRegisterInput::new(vec![a]))
                .unwrap_err(),
            RouteError::CapacityExceeded
        );

        let mut no_occurrences =
            install_fixture(revision, WorldConfig::new(8, 1, 0, 1, 100)).expect("install");
        assert_eq!(
            no_occurrences
                .register_route(RouteRegisterInput::new(vec![a]))
                .unwrap_err(),
            RouteError::EdgeOccurrenceCapacityExceeded
        );
        assert_eq!(no_occurrences.live_route_count, 0);
        assert_eq!(no_occurrences.live_route_edge_occurrence_count, 0);
        assert!(no_occurrences.routes.is_empty());

        let mut overflow =
            install_fixture(loop_revision(), WorldConfig::new(8, 1, u64::MAX, 1, 100))
                .expect("install");
        overflow.live_route_edge_occurrence_count = u64::MAX;
        assert_eq!(
            overflow
                .register_route(RouteRegisterInput::new(vec![a]))
                .unwrap_err(),
            RouteError::EdgeOccurrenceCapacityExceeded
        );
        assert_eq!(overflow.live_route_count, 0);
        assert_eq!(overflow.live_route_edge_occurrence_count, u64::MAX);
        assert!(overflow.routes.is_empty());
    }

    #[test]
    fn parked_and_completed_are_not_leaders() {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).unwrap();
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .unwrap();
        let mut world = install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 100)).unwrap();
        let route = register_full_spatial_route(&mut world);
        let profile = world
            .revision
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .unwrap();
        let parked = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000 + profile.length_mm() + profile.min_gap_mm() + 2_000,
                0,
            ))
            .unwrap();
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .unwrap();
        world
            .occupy_parking(parked, ParkingSpaceOrdinal::from_raw(0))
            .expect("park");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        let follower_state = world.vehicle_state(follower).copied().unwrap();
        assert_eq!(index_gap(&world, &follower_state), None);
        assert_index_matches_scan(&world);

        let revision = two_edge_revision();
        let stem = LaneEdgeOrdinal::from_raw(0);
        let tail = LaneEdgeOrdinal::from_raw(1);
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 100)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![stem, tail]))
            .expect("route");
        let tail_len = world.revision.traffic().lane_lengths_millimetres()[tail.index()];
        let finishing = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                1,
                tail_len,
                0,
            ))
            .expect("at route end");
        world.step(TickInput::new(100)).unwrap();
        assert_eq!(
            world.vehicle(finishing).unwrap().status(),
            VehicleStatus::Completed
        );
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("follower");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        let follower_state = world.vehicle_state(follower).copied().unwrap();
        assert_eq!(index_gap(&world, &follower_state), None);
        assert_index_matches_scan(&world);
    }

    #[test]
    fn diverge_overhang_matches_scan_and_occupancy_front_gap() {
        let revision = compile_revision(|module| {
            add_car_profile(module);
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "stem",
                    length_meters: 10.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[
                        laneflow_compiler::LaneEdgeReference::local("left"),
                        laneflow_compiler::LaneEdgeReference::local("right"),
                    ],
                })
                .expect("stem")
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "left",
                    length_meters: 20.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[],
                })
                .expect("left")
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "right",
                    length_meters: 20.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[],
                })
                .expect("right");
        });
        let traffic = revision.traffic();
        let count = traffic.lane_edge_count();
        let stem = (0..count)
            .map(LaneEdgeOrdinal::from_raw)
            .find(|edge| {
                traffic
                    .successors(*edge)
                    .is_some_and(|successors| successors.len() == 2)
            })
            .expect("stem");
        let branches = traffic.successors(stem).expect("branches");
        let left = branches[0];
        let right = branches[1];
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 100)).expect("install");
        let leader_route = world
            .register_route(RouteRegisterInput::new(vec![stem, left]))
            .expect("left route");
        let follower_route = world
            .register_route(RouteRegisterInput::new(vec![stem, right]))
            .expect("right route");
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                leader_route,
                1,
                500,
                0,
            ))
            .expect("leader");
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                follower_route,
                0,
                5_000,
                10_000,
            ))
            .expect("follower");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        assert_index_matches_scan(&world);
        let follower_state = world.vehicle_state(follower).copied().unwrap();
        let leader_state = world
            .live_order
            .iter()
            .copied()
            .find_map(|handle| {
                let state = world.vehicle_state(handle)?;
                (handle != follower).then_some(*state)
            })
            .expect("leader state");
        let lengths = world.revision.traffic().lane_lengths_millimetres();
        let follower_edges = world.route_edges(follower_state.route).unwrap();
        let leader_edges = world.route_edges(leader_state.route).unwrap();
        let horizon = world.leader_query_horizon_for(&follower_state);
        let indexed = world.occupancy.leader_gap(
            follower_state.handle,
            follower_edges,
            usize::try_from(follower_state.route_edge_index).unwrap(),
            follower_state.progress_mm,
            lengths,
            horizon,
        );
        let pair = occupancy_front_gap(
            lengths,
            follower_edges,
            usize::try_from(follower_state.route_edge_index).unwrap(),
            follower_state.progress_mm,
            leader_edges,
            usize::try_from(leader_state.route_edge_index).unwrap(),
            leader_state.progress_mm,
            leader_state.length_mm,
        );
        assert_eq!(indexed, pair);
        world.step(TickInput::new(100)).unwrap();
        let follower_after = world.vehicle(follower).unwrap();
        assert!(
            follower_after.progress_mm() < 6_000,
            "follower must not enter leader overhang, progress={}",
            follower_after.progress_mm()
        );
    }

    #[test]
    fn dense_same_edge_inspections_are_not_quadratic() {
        let revision = compile_revision(|module| {
            add_car_profile(module);
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "corridor",
                    length_meters: 400.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[],
                })
                .expect("corridor");
        });
        let edge = LaneEdgeOrdinal::from_raw(0);
        let n = 32_u32;
        let mut world =
            install_fixture(revision, WorldConfig::new(n, 4, 1_024, 1, 100)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![edge]))
            .expect("route");
        let profile = world
            .revision
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .unwrap();
        let spacing = profile.length_mm() + profile.min_gap_mm() + 1_000;
        for slot in 0..n {
            let progress = 5_000 + slot * spacing;
            world
                .spawn_vehicle(VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    progress,
                    0,
                ))
                .expect("spawn");
        }
        world.step(TickInput::new(100)).unwrap();
        let n_active = active_count(&world);
        let inspections = world.occupancy_inspections();
        let all_pairs = n_active.saturating_mul(n_active.saturating_sub(1));
        assert_eq!(n_active, u64::from(n));
        assert!(
            inspections >= n_active.saturating_sub(1),
            "index query must inspect at least one claim per follower with a leader, inspections={inspections} n={n_active}"
        );
        assert!(
            inspections < all_pairs,
            "inspections={inspections} must be below all-pairs={all_pairs}"
        );
        assert!(
            inspections <= n_active.saturating_mul(4),
            "single-edge dense query should be near-linear, inspections={inspections} n={n_active}"
        );
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        assert_index_matches_scan(&world);
    }

    #[test]
    fn repeated_edge_prefers_nearer_occurrence() {
        let revision = loop_revision();
        let a = LaneEdgeOrdinal::from_raw(0);
        let b = LaneEdgeOrdinal::from_raw(1);
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 100)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![a, b, a]))
            .expect("route");
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("rear");
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                2,
                6_000,
                0,
            ))
            .expect("ahead on repeated");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        assert_index_matches_scan(&world);
    }

    #[test]
    fn short_edge_chain_does_not_grow_record_capacity_after_warmup() {
        let revision = compile_revision(|module| {
            add_car_profile(module);
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "stem",
                    length_meters: 20.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[laneflow_compiler::LaneEdgeReference::local("s0")],
                })
                .expect("stem");
            let keys = ["s0", "s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9"];
            for (index, key) in keys.iter().enumerate() {
                if index + 1 < keys.len() {
                    let next = laneflow_compiler::LaneEdgeReference::local(keys[index + 1]);
                    module
                        .add_lane_edge(LaneEdgeInput {
                            lane_edge_key: key,
                            length_meters: 1.0,
                            speed_limit_meters_per_second: 10.0,
                            successors: std::slice::from_ref(&next),
                        })
                        .expect("short");
                } else {
                    module
                        .add_lane_edge(LaneEdgeInput {
                            lane_edge_key: key,
                            length_meters: 1.0,
                            speed_limit_meters_per_second: 10.0,
                            successors: &[],
                        })
                        .expect("short tail");
                }
            }
        });
        let traffic = revision.traffic();
        let mut edges = Vec::new();
        let mut current = (0..traffic.lane_edge_count())
            .map(LaneEdgeOrdinal::from_raw)
            .find(|edge| {
                traffic
                    .successors(*edge)
                    .is_some_and(|successors| !successors.is_empty())
                    && traffic.lane_lengths_millimetres()[edge.index()] >= 20_000
            })
            .expect("stem");
        loop {
            edges.push(current);
            let Some(successors) = traffic.successors(current) else {
                break;
            };
            let Some(next) = successors.first().copied() else {
                break;
            };
            current = next;
        }
        let mut world =
            install_fixture(revision, WorldConfig::new(1, 4, 1_024, 1, 1_000)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route");
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                5,
                500,
                0,
            ))
            .expect("spawn spanning five 1 m edges");
        world.step(TickInput::new(1_000)).unwrap();
        let ceiling = occupancy_record_limit(1);
        let cap = world.occupancy.records_capacity();
        let len = world.occupancy.records_len();
        assert!(
            len > 4,
            "body on the 1 m chain must emit more than four occupancy records, got {len}"
        );
        assert!(
            cap >= len,
            "retained occupancy capacity must cover actual records, cap={cap} len={len}"
        );
        assert!(
            cap < ceiling,
            "first rebuild must not reserve the global envelope, cap={cap} ceiling={ceiling}"
        );
        let mut high_water = cap;
        for _ in 0..8 {
            world.step(TickInput::new(1_000)).unwrap();
            let next = world.occupancy.records_capacity();
            assert!(
                next < ceiling,
                "span growth must stay below the fail-closed ceiling, cap={next} ceiling={ceiling}"
            );
            assert!(
                next >= high_water,
                "occupancy record capacity must be high-water, cap={next} high_water={high_water}"
            );
            high_water = next;
        }
        for _ in 0..8 {
            world.step(TickInput::new(1_000)).unwrap();
            assert_eq!(
                world.occupancy.records_capacity(),
                high_water,
                "after body-span high-water, ticks must not grow occupancy record capacity"
            );
        }
    }

    #[test]
    fn large_vehicle_capacity_does_not_reserve_envelope() {
        let revision = two_edge_revision();
        let stem = LaneEdgeOrdinal::from_raw(0);
        let mut world =
            install_fixture(revision, WorldConfig::new(10_000, 4, 1_024, 1, 100)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![stem]))
            .expect("route");
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("solo");
        world.step(TickInput::new(100)).unwrap();
        let cap = world.occupancy.records_capacity();
        let ceiling = occupancy_record_limit(10_000);
        assert!(
            cap < 256,
            "one vehicle must not reserve the capacity envelope, cap={cap}"
        );
        assert!(
            cap < ceiling,
            "retained occupancy capacity must stay below the fail-closed ceiling, cap={cap} ceiling={ceiling}"
        );
        assert!(
            world.occupancy.suffix_min_lo_capacity() < 256,
            "suffix min table must follow actual records, cap={}",
            world.occupancy.suffix_min_lo_capacity()
        );
        assert!(
            world.occupancy.suffix_second_lo_capacity() < 256,
            "suffix second table must follow actual records, cap={}",
            world.occupancy.suffix_second_lo_capacity()
        );
    }

    fn long_corridor_revision(length_meters: f64) -> Arc<SharedNetworkRevision> {
        compile_revision(|module| {
            add_car_profile(module);
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "corridor",
                    length_meters,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[],
                })
                .expect("corridor");
        })
    }

    fn expected_bumper_si(
        speed: f32,
        profile: laneflow_static_network::VehicleProfileView,
        delta_s: f32,
    ) -> f32 {
        let v_upper = speed + profile.max_accel() * delta_s;
        let travel_upper = 0.5 * (speed + v_upper) * delta_s;
        let hard = travel_upper + v_upper * v_upper / (2.0 * profile.emergency_decel());
        let min_gap = profile.min_gap_mm() as f32 / 1_000.0;
        let comfort = min_gap + speed * profile.time_headway();
        let minimum_gap = min_gap + travel_upper + 0.001;
        hard.max(comfort).max(minimum_gap)
    }

    #[test]
    fn leader_query_horizon_ceils_bumper_then_adds_max_vehicle_length() {
        let revision = two_edge_revision();
        let profile = revision
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .unwrap();
        let delta_s = 0.1_f32;
        let horizon = leader_query_horizon(0, profile, delta_s).expect("finite horizon");
        let expected_bumper =
            ceil_mm(f64::from(expected_bumper_si(0.0, profile, delta_s))).expect("ceil bumper");
        assert_eq!(horizon.bumper_gap_mm, expected_bumper);
        assert_eq!(
            horizon.front_query_mm,
            expected_bumper.saturating_add(MAX_VEHICLE_LENGTH_MM)
        );
        assert!(horizon.front_query_mm > horizon.bumper_gap_mm);
        assert_eq!(leader_query_horizon(0, profile, f32::NAN), None);
        assert_eq!(leader_query_horizon(0, profile, 0.0), None);
        assert_eq!(leader_query_horizon(0, profile, -1.0), None);
        assert_eq!(ceil_mm(1.0), Some(1_000));
        assert_eq!(ceil_mm(1.000_1), Some(1_001));
        assert_eq!(ceil_mm(-0.1), None);

        let moving = leader_query_horizon(5_000, profile, delta_s).expect("moving horizon");
        let speed = 5.0_f32;
        let min_gap = profile.min_gap_mm() as f32 / 1_000.0;
        let comfort = min_gap + speed * profile.time_headway();
        let bumper_si = expected_bumper_si(speed, profile, delta_s);
        assert!(
            (comfort - bumper_si).abs() < f32::EPSILON,
            "nonzero-speed case must lock the comfort term, comfort={comfort} bumper={bumper_si}"
        );
        let expected_moving = ceil_mm(f64::from(bumper_si)).expect("ceil bumper");
        assert_eq!(moving.bumper_gap_mm, expected_moving);
        assert_eq!(
            moving.front_query_mm,
            expected_moving.saturating_add(MAX_VEHICLE_LENGTH_MM)
        );
    }

    #[test]
    fn current_edge_accepts_bumper_window_not_front_padding() {
        let edge = LaneEdgeOrdinal::from_raw(0);
        let follower = VehicleHandle::new(0, 0);
        let leader = VehicleHandle::new(1, 0);
        let mut index = OccupancyIndex::with_capacity(1, 2);
        let bumper = 20_000_u32;
        let front = 150_000_u32;
        let pending = vec![
            OccupancyRecord {
                vehicle: follower,
                bucket: OccupancyBucketOrdinal::from_edge(edge),
                lo_mm: 0,
                hi_mm: 1_000,
                update_sequence: 0,
            },
            OccupancyRecord {
                vehicle: leader,
                bucket: OccupancyBucketOrdinal::from_edge(edge),
                lo_mm: 1_000 + bumper,
                hi_mm: 1_000 + bumper + 2_000,
                update_sequence: 1,
            },
        ];
        index.rebuild_from_pending(&pending, 1);
        let at_bumper = index.leader_gap(
            follower,
            &[edge],
            0,
            1_000,
            &[400_000],
            LeaderQueryHorizon::new(bumper, front),
        );
        assert_eq!(at_bumper, Some(i64::from(bumper)));

        pending_replace_lo(&mut index, &pending, follower, leader, edge, bumper + 1);
        let phantom = index.leader_gap(
            follower,
            &[edge],
            0,
            1_000,
            &[400_000],
            LeaderQueryHorizon::new(bumper, front),
        );
        assert_eq!(
            phantom, None,
            "gap inside front padding must not be a leader"
        );
    }

    fn pending_replace_lo(
        index: &mut OccupancyIndex,
        pending: &[OccupancyRecord],
        follower: VehicleHandle,
        leader: VehicleHandle,
        edge: LaneEdgeOrdinal,
        leader_lo: u32,
    ) {
        let pending = vec![
            OccupancyRecord {
                vehicle: follower,
                bucket: OccupancyBucketOrdinal::from_edge(edge),
                lo_mm: pending[0].lo_mm,
                hi_mm: pending[0].hi_mm,
                update_sequence: 0,
            },
            OccupancyRecord {
                vehicle: leader,
                bucket: OccupancyBucketOrdinal::from_edge(edge),
                lo_mm: 1_000 + leader_lo,
                hi_mm: 1_000 + leader_lo + 2_000,
                update_sequence: 1,
            },
        ];
        index.rebuild_from_pending(&pending, 1);
    }

    #[test]
    fn subsequent_entrance_walks_front_and_accepts_bumper() {
        let first = LaneEdgeOrdinal::from_raw(0);
        let second = LaneEdgeOrdinal::from_raw(1);
        let follower = VehicleHandle::new(0, 0);
        let leader = VehicleHandle::new(1, 0);
        let lengths = [10_000_u32, 10_000];
        let edges = [first, second];
        let remaining_on_current = 9_000_u32;
        let pending = vec![
            OccupancyRecord {
                vehicle: follower,
                bucket: OccupancyBucketOrdinal::from_edge(first),
                lo_mm: 0,
                hi_mm: 1_000,
                update_sequence: 0,
            },
            OccupancyRecord {
                vehicle: leader,
                bucket: OccupancyBucketOrdinal::from_edge(second),
                lo_mm: 0,
                hi_mm: 1_000,
                update_sequence: 1,
            },
        ];
        let mut occupancy = OccupancyIndex::with_capacity(2, pending.len());
        occupancy.rebuild_from_pending(&pending, 2);
        occupancy.reset_inspections();
        let accepted = occupancy.leader_gap(
            follower,
            &edges,
            0,
            1_000,
            &lengths,
            LeaderQueryHorizon::new(remaining_on_current, remaining_on_current),
        );
        assert_eq!(accepted, Some(i64::from(remaining_on_current)));
        assert_eq!(occupancy.occurrence_walks(), 1);

        occupancy.reset_inspections();
        let skipped = occupancy.leader_gap(
            follower,
            &edges,
            0,
            1_000,
            &lengths,
            LeaderQueryHorizon::new(
                remaining_on_current.saturating_sub(1),
                remaining_on_current.saturating_sub(1),
            ),
        );
        assert_eq!(skipped, None);
        assert_eq!(occupancy.occurrence_walks(), 0);

        occupancy.reset_inspections();
        let phantom = occupancy.leader_gap(
            follower,
            &edges,
            0,
            1_000,
            &lengths,
            LeaderQueryHorizon::new(remaining_on_current.saturating_sub(1), remaining_on_current),
        );
        assert_eq!(phantom, None);
        assert_eq!(
            occupancy.occurrence_walks(),
            1,
            "front walk must still visit; bumper window rejects"
        );
    }

    #[test]
    fn subsequent_gap_beyond_bumper_is_dropped_after_visit() {
        let first = LaneEdgeOrdinal::from_raw(0);
        let second = LaneEdgeOrdinal::from_raw(1);
        let follower = VehicleHandle::new(0, 0);
        let leader = VehicleHandle::new(1, 0);
        let lengths = [10_000_u32, 10_000];
        let edges = [first, second];
        let bumper = 9_000_u32;
        let pending = vec![
            OccupancyRecord {
                vehicle: follower,
                bucket: OccupancyBucketOrdinal::from_edge(first),
                lo_mm: 0,
                hi_mm: 1_000,
                update_sequence: 0,
            },
            OccupancyRecord {
                vehicle: leader,
                bucket: OccupancyBucketOrdinal::from_edge(second),
                lo_mm: 1,
                hi_mm: 1_001,
                update_sequence: 1,
            },
        ];
        let mut occupancy = OccupancyIndex::with_capacity(2, pending.len());
        occupancy.rebuild_from_pending(&pending, 2);
        occupancy.reset_inspections();
        let gap = occupancy.leader_gap(
            follower,
            &edges,
            0,
            1_000,
            &lengths,
            LeaderQueryHorizon::new(bumper, bumper),
        );
        assert_eq!(gap, None);
        assert_eq!(
            occupancy.occurrence_walks(),
            1,
            "entrance == walk window must still visit the later occurrence"
        );
    }

    #[test]
    fn wrap_occurrence_beyond_walk_window_is_not_leader() {
        let a = LaneEdgeOrdinal::from_raw(0);
        let b = LaneEdgeOrdinal::from_raw(1);
        let follower = VehicleHandle::new(0, 0);
        let behind = VehicleHandle::new(1, 0);
        let lengths = [10_000_u32, 10_000];
        let edges = [a, b, a];
        let window = 5_000_u32;
        let pending = vec![
            OccupancyRecord {
                vehicle: follower,
                bucket: OccupancyBucketOrdinal::from_edge(a),
                lo_mm: 8_000,
                hi_mm: 9_000,
                update_sequence: 0,
            },
            OccupancyRecord {
                vehicle: behind,
                bucket: OccupancyBucketOrdinal::from_edge(a),
                lo_mm: 1_000,
                hi_mm: 2_000,
                update_sequence: 1,
            },
        ];
        let mut occupancy = OccupancyIndex::with_capacity(2, pending.len());
        occupancy.rebuild_from_pending(&pending, 2);
        occupancy.reset_inspections();
        let bounded = occupancy.leader_gap(
            follower,
            &edges,
            0,
            9_000,
            &lengths,
            LeaderQueryHorizon::new(window, window),
        );
        let bounded_walks = occupancy.occurrence_walks();
        occupancy.reset_inspections();
        let unbounded = occupancy.leader_gap(
            follower,
            &edges,
            0,
            9_000,
            &lengths,
            LeaderQueryHorizon::UNBOUNDED,
        );
        let unbounded_walks = occupancy.occurrence_walks();
        assert_eq!(bounded, None);
        assert_eq!(
            unbounded,
            remaining_along_route_i64(&lengths, &edges, 0, 9_000, 2, 1_000)
        );
        assert_eq!(bounded_walks, 1, "must visit empty later b, not wrap to a");
        assert!(
            unbounded_walks > bounded_walks,
            "unbounded wrap must walk the repeated a, bounded={bounded_walks} unbounded={unbounded_walks}"
        );
    }

    #[test]
    fn overlapping_negative_gap_stays_visible_at_zero_bumper_window() {
        let edge = LaneEdgeOrdinal::from_raw(0);
        let follower = VehicleHandle::new(0, 0);
        let leader = VehicleHandle::new(1, 0);
        let mut index = OccupancyIndex::with_capacity(1, 2);
        let pending = vec![
            OccupancyRecord {
                vehicle: follower,
                bucket: OccupancyBucketOrdinal::from_edge(edge),
                lo_mm: 0,
                hi_mm: 5_000,
                update_sequence: 0,
            },
            OccupancyRecord {
                vehicle: leader,
                bucket: OccupancyBucketOrdinal::from_edge(edge),
                lo_mm: 4_000,
                hi_mm: 8_000,
                update_sequence: 1,
            },
        ];
        index.rebuild_from_pending(&pending, 1);
        let gap = index.leader_gap(
            follower,
            &[edge],
            0,
            5_000,
            &[20_000],
            LeaderQueryHorizon::new(0, 150_000),
        );
        assert_eq!(gap, Some(-1_000));
    }

    #[test]
    fn close_current_leader_stops_later_occurrence_walks() {
        let edges: Vec<_> = (0..16_u32).map(LaneEdgeOrdinal::from_raw).collect();
        let lengths = vec![10_000_u32; 16];
        let follower = VehicleHandle::new(0, 0);
        let mut pending = vec![OccupancyRecord {
            vehicle: follower,
            bucket: OccupancyBucketOrdinal::from_edge(edges[0]),
            lo_mm: 0,
            hi_mm: 1_000,
            update_sequence: 0,
        }];
        pending.push(OccupancyRecord {
            vehicle: VehicleHandle::new(1, 0),
            bucket: OccupancyBucketOrdinal::from_edge(edges[0]),
            lo_mm: 1_000,
            hi_mm: 2_000,
            update_sequence: 1,
        });
        for index in 2..16_u32 {
            pending.push(OccupancyRecord {
                vehicle: VehicleHandle::new(index, 0),
                bucket: OccupancyBucketOrdinal::from_edge(edges[index as usize]),
                lo_mm: 100,
                hi_mm: 1_000,
                update_sequence: index,
            });
        }
        let mut occupancy = OccupancyIndex::with_capacity(16, pending.len());
        occupancy.rebuild_from_pending(&pending, 16);
        occupancy.reset_inspections();
        let gap = occupancy.leader_gap(
            follower,
            &edges,
            0,
            1_000,
            &lengths,
            LeaderQueryHorizon::UNBOUNDED,
        );
        assert_eq!(gap, Some(0));
        assert_eq!(occupancy.occurrence_walks(), 0);
    }

    #[test]
    fn formula_horizon_hides_leader_beyond_and_matches_filtered_scan() {
        let revision = long_corridor_revision(400.0);
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 100)).expect("install");
        let edge = LaneEdgeOrdinal::from_raw(0);
        let route = world
            .register_route(RouteRegisterInput::new(vec![edge]))
            .expect("route");
        let profile = world
            .revision
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .unwrap();
        let horizon = leader_query_horizon(0, profile, 0.1).expect("finite horizon");
        let follower_progress = 1_000_u32;
        let far_progress = follower_progress
            .saturating_add(horizon.front_query_mm)
            .saturating_add(profile.length_mm())
            .saturating_add(1);
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                far_progress,
                0,
            ))
            .expect("far leader");
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                follower_progress,
                0,
            ))
            .expect("follower");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        let state = world.vehicle_state(follower).copied().unwrap();
        assert_eq!(index_gap(&world, &state), None);
        assert_index_matches_scan(&world);

        let phantom_progress = follower_progress
            .saturating_add(horizon.bumper_gap_mm)
            .saturating_add(profile.length_mm())
            .saturating_add(1);
        let mut phantom_world = install_fixture(
            long_corridor_revision(400.0),
            WorldConfig::new(8, 4, 1_024, 1, 100),
        )
        .expect("install");
        let phantom_route = phantom_world
            .register_route(RouteRegisterInput::new(vec![edge]))
            .expect("route");
        phantom_world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                phantom_route,
                0,
                phantom_progress,
                0,
            ))
            .expect("phantom leader");
        let phantom_follower = phantom_world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                phantom_route,
                0,
                follower_progress,
                0,
            ))
            .expect("follower");
        phantom_world
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
        let phantom_state = phantom_world
            .vehicle_state(phantom_follower)
            .copied()
            .unwrap();
        assert_eq!(index_gap(&phantom_world, &phantom_state), None);
        assert_index_matches_scan(&phantom_world);

        let near_progress = follower_progress
            .saturating_add(horizon.bumper_gap_mm)
            .saturating_add(profile.length_mm());
        let mut near_world = install_fixture(
            long_corridor_revision(400.0),
            WorldConfig::new(8, 4, 1_024, 1, 100),
        )
        .expect("install");
        let near_route = near_world
            .register_route(RouteRegisterInput::new(vec![edge]))
            .expect("route");
        near_world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                near_route,
                0,
                near_progress,
                0,
            ))
            .expect("horizon leader");
        let near_follower = near_world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                near_route,
                0,
                follower_progress,
                0,
            ))
            .expect("follower");
        near_world
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
        let near_state = near_world.vehicle_state(near_follower).copied().unwrap();
        assert_eq!(
            index_gap(&near_world, &near_state),
            Some(i64::from(horizon.bumper_gap_mm))
        );
        assert_index_matches_scan(&near_world);
    }

    #[test]
    fn subsequent_walks_follow_front_query_not_remaining_edge_count() {
        let follower = VehicleHandle::new(0, 0);
        let front = 20_000_u32;
        for edge_count in [8_usize, 16, 32] {
            let edges: Vec<_> = (0..edge_count as u32)
                .map(LaneEdgeOrdinal::from_raw)
                .collect();
            let lengths = vec![10_000_u32; edge_count];
            let last = edge_count as u32 - 1;
            let pending = vec![
                OccupancyRecord {
                    vehicle: follower,
                    bucket: OccupancyBucketOrdinal::from_edge(edges[0]),
                    lo_mm: 0,
                    hi_mm: 1_000,
                    update_sequence: 0,
                },
                OccupancyRecord {
                    vehicle: VehicleHandle::new(last, 0),
                    bucket: OccupancyBucketOrdinal::from_edge(edges[last as usize]),
                    lo_mm: 100,
                    hi_mm: 1_000,
                    update_sequence: last,
                },
            ];
            let mut occupancy = OccupancyIndex::with_capacity(edge_count, pending.len());
            occupancy.rebuild_from_pending(&pending, edge_count);
            occupancy.reset_inspections();
            let gap = occupancy.leader_gap(
                follower,
                &edges,
                0,
                1_000,
                &lengths,
                LeaderQueryHorizon::new(2_000, front),
            );
            let walks = occupancy.occurrence_walks();
            assert_eq!(gap, None, "edge_count={edge_count}");
            assert_eq!(
                walks, 2,
                "subsequent walks must follow front_query, edge_count={edge_count} walks={walks}"
            );
        }

        let formula_front = 130_010_u32;
        for edge_count in [32_usize, 64] {
            let edges: Vec<_> = (0..edge_count as u32)
                .map(LaneEdgeOrdinal::from_raw)
                .collect();
            let lengths = vec![10_000_u32; edge_count];
            let last = edge_count as u32 - 1;
            let pending = vec![
                OccupancyRecord {
                    vehicle: follower,
                    bucket: OccupancyBucketOrdinal::from_edge(edges[0]),
                    lo_mm: 0,
                    hi_mm: 1_000,
                    update_sequence: 0,
                },
                OccupancyRecord {
                    vehicle: VehicleHandle::new(last, 0),
                    bucket: OccupancyBucketOrdinal::from_edge(edges[last as usize]),
                    lo_mm: 100,
                    hi_mm: 1_000,
                    update_sequence: last,
                },
            ];
            let mut occupancy = OccupancyIndex::with_capacity(edge_count, pending.len());
            occupancy.rebuild_from_pending(&pending, edge_count);
            occupancy.reset_inspections();
            let gap = occupancy.leader_gap(
                follower,
                &edges,
                0,
                1_000,
                &lengths,
                LeaderQueryHorizon::new(2_010, formula_front),
            );
            let walks = occupancy.occurrence_walks();
            assert_eq!(gap, None, "formula-scale edge_count={edge_count}");
            assert_eq!(
                walks, 13,
                "walks must follow ~130 m front_query, edge_count={edge_count} walks={walks}"
            );
        }
    }

    #[test]
    fn corrupt_route_index_fails_closed_occupancy_rebuild() {
        let revision = two_edge_revision();
        let stem = LaneEdgeOrdinal::from_raw(0);
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1, 100)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![stem]))
            .expect("route");
        let handle = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("solo");
        world.step(TickInput::new(100)).unwrap();
        let before_len = world.occupancy.records_len();
        let before_time = world.time_ms;
        let slot = usize::try_from(handle.index()).expect("vehicle index fits usize");
        world.vehicles[slot]
            .state
            .as_mut()
            .expect("spawned vehicle")
            .route_edge_index = 10_000;
        assert_eq!(
            world.step(TickInput::new(100)),
            Err(StepError::OccupancyIntervalIncomplete)
        );
        assert_eq!(world.time_ms, before_time);
        assert_eq!(
            world.occupancy.records_len(),
            before_len,
            "failed rebuild must not replace occupancy records"
        );
    }
}
