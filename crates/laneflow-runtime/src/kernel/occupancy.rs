use laneflow_static_contract::{LaneEdgeOrdinal, MAX_VEHICLE_LENGTH_MM, MIN_LANE_EDGE_LENGTH_MM};
use laneflow_static_network::SharedNetworkRevision;

use crate::kernel::tables::{CompiledRoute, RouteSlot, VehicleSlot, for_each_admission_interval};
use crate::{
    ObservationStateSequence, RouteHandle, StepError, VehicleHandle, VehicleState, VehicleStatus,
    WorldGeneration,
};

#[cfg(test)]
use crate::TrafficWorld;
#[cfg(test)]
use std::cell::Cell;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
thread_local! {
    static OCCUPANCY_REBUILD_EVENTS: Cell<u64> = const { Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_occupancy_rebuild_events() {
    OCCUPANCY_REBUILD_EVENTS.set(0);
}

#[cfg(test)]
pub(crate) fn occupancy_rebuild_events() -> u64 {
    OCCUPANCY_REBUILD_EVENTS.with(Cell::get)
}

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

/// 一条物理边上的占用片段，含已进入该边的零进度前杠；不是资源声明。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OccupancyRecord {
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
/// 当前上界 `L=128_000`、`E=100`；零进度车身最多 1_280 条，加入入口点仍在此上界内。
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

/// 一条边自己的占用记录。后缀下标只指向这一条边，插入别的边不会挪动它。
#[derive(Debug)]
struct OccupancyBucket {
    records: Vec<OccupancyRecord>,
    /// `suffix_min_lo[i]` 是本桶 `[i, len)` 中 `lo_mm` 最小记录的桶内下标。
    suffix_min_lo: Vec<u32>,
    /// 同后缀中车辆不同于最小值的次小 `lo_mm`，供 O(1) 排除 self。
    suffix_second_lo: Vec<u32>,
}

impl OccupancyBucket {
    fn empty() -> Self {
        Self {
            records: Vec::new(),
            suffix_min_lo: Vec::new(),
            suffix_second_lo: Vec::new(),
        }
    }

    fn fill_suffix(&mut self) {
        let end = self.records.len();
        if end == 0 {
            return;
        }
        let last = end - 1;
        self.suffix_min_lo[last] = record_slot(last);
        self.suffix_second_lo[last] = SUFFIX_NONE;
        for index in (0..last).rev() {
            let later_min = usize::try_from(self.suffix_min_lo[index + 1])
                .expect("suffix min index fits usize");
            let later_second = suffix_slot(self.suffix_second_lo[index + 1]);
            let (best, second) = merge_suffix_pair(&self.records, index, later_min, later_second);
            self.suffix_min_lo[index] = record_slot(best);
            self.suffix_second_lo[index] = second.map_or(SUFFIX_NONE, record_slot);
        }
    }
}

#[derive(Debug)]
pub(crate) struct OccupancyIndex {
    /// 仅在当前世界重建成功后记录来源；事务纯构造的索引尚未绑定活动世代。
    source: Option<(WorldGeneration, ObservationStateSequence)>,
    buckets: Vec<OccupancyBucket>,
    /// 全部车道的记录条数。生成时直接改它，不把每条车道再加一遍。
    record_len: usize,
    #[cfg(test)]
    inspections: AtomicU64,
    #[cfg(test)]
    occurrence_walks: AtomicU64,
}

/// 每次重建的桶计数/写游标；成功查询不借用它。
///
/// 机动 transition 的反向表按静态路网只建一次，供新鲜摆放找上游后车。
#[derive(Debug)]
pub(crate) struct OccupancyScratch {
    positions: Vec<usize>,
    maneuver_upstream_offsets: Vec<u32>,
    maneuver_upstream_sources: Vec<u32>,
    /// 当前路网和步长下的最大跟车窗。路网不变就复用，不在每辆新车上重扫限速。
    follower_bumper_mm: Option<u32>,
    #[cfg(test)]
    exact_pending: Vec<OccupancyRecord>,
}

#[cfg(test)]
impl OccupancyScratch {
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        let Self {
            positions,
            maneuver_upstream_offsets,
            maneuver_upstream_sources,
            follower_bumper_mm: _,
            exact_pending,
        } = self;
        crate::kernel::state::vec_bytes(positions)
            + crate::kernel::state::vec_bytes(maneuver_upstream_offsets)
            + crate::kernel::state::vec_bytes(maneuver_upstream_sources)
            + crate::kernel::state::vec_bytes(exact_pending)
    }
}

impl OccupancyScratch {
    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        self.positions.capacity()
    }

    fn record_total(&self, bucket_count: usize) -> usize {
        self.positions.iter().take(bucket_count).copied().sum()
    }

    pub(crate) fn follower_bumper_mm(&self) -> Option<u32> {
        self.follower_bumper_mm
    }

    pub(crate) fn remember_follower_bumper_mm(&mut self, reach_mm: u32) {
        self.follower_bumper_mm = Some(reach_mm);
    }

    /// 按目标边列出通过机动 transition 进入该边的前驱。静态路网不变就复用。
    pub(crate) fn ensure_maneuver_upstream(
        &mut self,
        traffic: &laneflow_static_network::SharedTrafficNetwork,
    ) -> Result<(), StepError> {
        let edge_count = usize::try_from(traffic.lane_edge_count()).unwrap_or(0);
        if self.maneuver_upstream_offsets.len() == edge_count.saturating_add(1)
            && !self.maneuver_upstream_offsets.is_empty()
        {
            return Ok(());
        }
        let mut counts = Vec::new();
        counts
            .try_reserve(edge_count)
            .map_err(|_| StepError::OccupancyAllocFailed)?;
        counts.resize(edge_count, 0u32);
        for raw in 0..edge_count {
            let from = laneflow_static_contract::LaneEdgeOrdinal::from_raw(
                u32::try_from(raw).map_err(|_| StepError::OccupancyIntervalIncomplete)?,
            );
            let Some(candidates) = traffic.maneuvers().transition_candidates(from) else {
                continue;
            };
            for candidate in candidates {
                let to = candidate.successor().index();
                if let Some(count) = counts.get_mut(to) {
                    *count = count.saturating_add(1);
                }
            }
        }
        let mut offsets = Vec::new();
        offsets
            .try_reserve(edge_count.saturating_add(1))
            .map_err(|_| StepError::OccupancyAllocFailed)?;
        offsets.resize(edge_count.saturating_add(1), 0u32);
        for index in 0..edge_count {
            offsets[index + 1] = offsets[index].saturating_add(counts[index]);
        }
        let total = usize::try_from(*offsets.last().unwrap_or(&0)).unwrap_or(0);
        let mut sources = Vec::new();
        sources
            .try_reserve(total)
            .map_err(|_| StepError::OccupancyAllocFailed)?;
        sources.resize(total, 0u32);
        let mut cursor = Vec::new();
        cursor
            .try_reserve(offsets.len())
            .map_err(|_| StepError::OccupancyAllocFailed)?;
        cursor.extend_from_slice(&offsets);
        for raw in 0..edge_count {
            let from = u32::try_from(raw).map_err(|_| StepError::OccupancyIntervalIncomplete)?;
            let from_edge = laneflow_static_contract::LaneEdgeOrdinal::from_raw(from);
            let Some(candidates) = traffic.maneuvers().transition_candidates(from_edge) else {
                continue;
            };
            for candidate in candidates {
                let to = candidate.successor().index();
                let Some(slot) = cursor.get_mut(to) else {
                    continue;
                };
                let index = usize::try_from(*slot).unwrap_or(0);
                if let Some(source) = sources.get_mut(index) {
                    *source = from;
                    *slot = slot.saturating_add(1);
                }
            }
        }
        self.maneuver_upstream_offsets = offsets;
        self.maneuver_upstream_sources = sources;
        Ok(())
    }

    pub(crate) fn maneuver_upstream(
        &self,
        edge: laneflow_static_contract::LaneEdgeOrdinal,
    ) -> &[u32] {
        let index = edge.index();
        let Some(start) = self.maneuver_upstream_offsets.get(index).copied() else {
            return &[];
        };
        let Some(end) = self.maneuver_upstream_offsets.get(index + 1).copied() else {
            return &[];
        };
        let start = usize::try_from(start).unwrap_or(0);
        let end = usize::try_from(end).unwrap_or(start);
        self.maneuver_upstream_sources
            .get(start..end.min(self.maneuver_upstream_sources.len()))
            .unwrap_or(&[])
    }
}

#[cfg(test)]
impl OccupancyIndex {
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        let Self {
            source: _,
            buckets,
            record_len: _,
            inspections: _,
            occurrence_walks: _,
        } = self;
        buckets
            .iter()
            .fold(crate::kernel::state::vec_bytes(buckets), |bytes, bucket| {
                bytes
                    + crate::kernel::state::vec_bytes(&bucket.records)
                    + crate::kernel::state::vec_bytes(&bucket.suffix_min_lo)
                    + crate::kernel::state::vec_bytes(&bucket.suffix_second_lo)
            })
    }
}

/// 路线窗内最近前车。间隙是前保险杠到该车后保险杠，可负。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LeaderContact {
    pub(crate) vehicle: VehicleHandle,
    pub(crate) gap_mm: i64,
}

impl OccupancyIndex {
    /// 空构造（不预分配边级表）；全部增长走 try 路径，分配失败映射为
    /// `OccupancyAllocFailed` 而非中止进程。供切换事务暂存构造使用。
    pub(crate) fn try_empty() -> Result<(Self, OccupancyScratch), StepError> {
        let mut index = Self {
            source: None,
            buckets: Vec::new(),
            record_len: 0,
            #[cfg(test)]
            inspections: AtomicU64::new(0),
            #[cfg(test)]
            occurrence_walks: AtomicU64::new(0),
        };
        let mut scratch = OccupancyScratch {
            positions: Vec::new(),
            maneuver_upstream_offsets: Vec::new(),
            maneuver_upstream_sources: Vec::new(),
            follower_bumper_mm: None,
            #[cfg(test)]
            exact_pending: Vec::new(),
        };
        index.try_prepare_scratch(&mut scratch, 0)?;
        Ok((index, scratch))
    }

    pub(crate) fn with_capacity(
        bucket_count: usize,
        _record_capacity: usize,
    ) -> (Self, OccupancyScratch) {
        let mut buckets = Vec::with_capacity(bucket_count);
        buckets.resize_with(bucket_count, OccupancyBucket::empty);
        let scratch = OccupancyScratch {
            positions: vec![0; bucket_count],
            maneuver_upstream_offsets: Vec::new(),
            maneuver_upstream_sources: Vec::new(),
            follower_bumper_mm: None,
            #[cfg(test)]
            exact_pending: Vec::new(),
        };
        let index = Self {
            source: None,
            buckets,
            record_len: 0,
            #[cfg(test)]
            inspections: AtomicU64::new(0),
            #[cfg(test)]
            occurrence_walks: AtomicU64::new(0),
        };
        (index, scratch)
    }

    #[cfg(test)]
    pub(crate) fn inspections(&self) -> u64 {
        self.inspections.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn occurrence_walks(&self) -> u64 {
        self.occurrence_walks.load(Ordering::Relaxed)
    }

    fn record_count(&self) -> usize {
        self.record_len
    }

    #[cfg(test)]
    pub(crate) fn records_capacity(&self) -> usize {
        self.buckets.iter().fold(0, |sum, bucket| {
            sum.saturating_add(bucket.records.capacity())
        })
    }

    #[cfg(test)]
    pub(crate) fn records_len(&self) -> usize {
        self.record_count()
    }

    #[cfg(test)]
    pub(crate) fn offsets_capacity(&self) -> usize {
        self.buckets.capacity()
    }

    #[cfg(test)]
    pub(crate) fn suffix_min_lo_capacity(&self) -> usize {
        self.buckets.iter().fold(0, |sum, bucket| {
            sum.saturating_add(bucket.suffix_min_lo.capacity())
        })
    }

    #[cfg(test)]
    pub(crate) fn suffix_second_lo_capacity(&self) -> usize {
        self.buckets.iter().fold(0, |sum, bucket| {
            sum.saturating_add(bucket.suffix_second_lo.capacity())
        })
    }

    #[cfg(test)]
    pub(crate) fn records_snapshot(&self) -> Vec<OccupancyRecord> {
        self.buckets
            .iter()
            .flat_map(|bucket| bucket.records.iter().copied())
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn same_layout(&self, other: &Self) -> bool {
        self.buckets.len() == other.buckets.len()
            && self
                .buckets
                .iter()
                .zip(&other.buckets)
                .all(|(left, right)| {
                    left.records == right.records
                        && left.suffix_min_lo == right.suffix_min_lo
                        && left.suffix_second_lo == right.suffix_second_lo
                })
    }

    fn note_inspection(&self) {
        #[cfg(test)]
        self.inspections
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_add(1))
            })
            .expect("counter update");
    }

    fn note_occurrence_walk(&self) {
        #[cfg(test)]
        self.occurrence_walks
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_add(1))
            })
            .expect("counter update");
    }

    #[cfg(test)]
    fn reset_inspections(&self) {
        self.inspections.store(0, Ordering::Relaxed);
        self.occurrence_walks.store(0, Ordering::Relaxed);
    }

    #[cfg(test)]
    fn rebuild_from_pending(
        &mut self,
        scratch: &mut OccupancyScratch,
        pending: &[OccupancyRecord],
        bucket_count: usize,
    ) {
        self.reset_inspections();
        self.try_prepare_scratch(scratch, bucket_count)
            .expect("test occupancy scratch");
        for record in pending {
            if let Some(count) = scratch.positions.get_mut(record.bucket.index()) {
                *count += 1;
            }
        }
        self.try_reserve_records(scratch, bucket_count)
            .expect("test occupancy records");
        self.finish_layout(scratch, bucket_count);
        for record in pending {
            self.write_record(scratch, *record);
        }
        self.sort_buckets(bucket_count);
    }

    fn try_prepare_scratch(
        &mut self,
        scratch: &mut OccupancyScratch,
        bucket_count: usize,
    ) -> Result<(), StepError> {
        if self.buckets.len() < bucket_count {
            let extra = bucket_count - self.buckets.len();
            self.buckets
                .try_reserve(extra)
                .map_err(|_| StepError::OccupancyAllocFailed)?;
            for _ in 0..extra {
                self.buckets.push(OccupancyBucket::empty());
            }
        } else if self.buckets.len() > bucket_count {
            self.buckets.truncate(bucket_count);
        }
        try_reserve_len(&mut scratch.positions, bucket_count)?;
        scratch.positions.clear();
        scratch.positions.resize(bucket_count, 0);
        Ok(())
    }

    /// 按每条边已经数好的条数预留。不缩小已有容量，下一拍同一条边不再重新分配。
    fn try_reserve_records(
        &mut self,
        scratch: &OccupancyScratch,
        bucket_count: usize,
    ) -> Result<(), StepError> {
        for index in 0..bucket_count {
            let needed = scratch.positions.get(index).copied().unwrap_or(0);
            let bucket = self
                .buckets
                .get_mut(index)
                .ok_or(StepError::OccupancyIntervalIncomplete)?;
            try_reserve_len(&mut bucket.records, needed)?;
            try_reserve_len(&mut bucket.suffix_min_lo, needed)?;
            try_reserve_len(&mut bucket.suffix_second_lo, needed)?;
        }
        Ok(())
    }

    fn finish_layout(&mut self, scratch: &mut OccupancyScratch, bucket_count: usize) {
        let mut total = 0usize;
        for index in 0..bucket_count {
            let count = scratch.positions.get(index).copied().unwrap_or(0);
            total = total.saturating_add(count);
            let bucket = &mut self.buckets[index];
            debug_assert!(bucket.records.capacity() >= count);
            debug_assert!(bucket.suffix_min_lo.capacity() >= count);
            debug_assert!(bucket.suffix_second_lo.capacity() >= count);
            bucket.records.clear();
            bucket.records.resize(count, OccupancyRecord::PLACEHOLDER);
            bucket.suffix_min_lo.clear();
            bucket.suffix_min_lo.resize(count, 0);
            bucket.suffix_second_lo.clear();
            bucket.suffix_second_lo.resize(count, SUFFIX_NONE);
        }
        self.record_len = total;
        scratch.positions.clear();
        scratch.positions.resize(bucket_count, 0);
    }

    fn write_record(&mut self, scratch: &mut OccupancyScratch, record: OccupancyRecord) {
        let bucket_index = record.bucket.index();
        let Some(head) = scratch.positions.get_mut(bucket_index) else {
            return;
        };
        let slot = *head;
        if let Some(target) = self
            .buckets
            .get_mut(bucket_index)
            .and_then(|bucket| bucket.records.get_mut(slot))
        {
            *target = record;
            *head = slot.saturating_add(1);
        }
    }

    fn sort_buckets(&mut self, bucket_count: usize) {
        for bucket in self.buckets.iter_mut().take(bucket_count) {
            bucket.records.sort_unstable_by_key(|record| {
                (
                    record.hi_mm,
                    record.lo_mm,
                    record.update_sequence,
                    record.vehicle.index(),
                )
            });
            bucket.fill_suffix();
        }
    }

    fn bucket(&self, edge: LaneEdgeOrdinal) -> Option<&OccupancyBucket> {
        self.buckets.get(edge.index())
    }

    fn min_lo_from(
        &self,
        bucket: &OccupancyBucket,
        start: usize,
        skip: VehicleHandle,
    ) -> Option<OccupancyRecord> {
        if start >= bucket.records.len() {
            return None;
        }
        self.note_inspection();
        let pick = usize::try_from(*bucket.suffix_min_lo.get(start)?)
            .ok()
            .filter(|index| *index < bucket.records.len())?;
        let record = *bucket.records.get(pick)?;
        if record.vehicle != skip {
            return Some(record);
        }
        self.note_inspection();
        let second = suffix_slot(*bucket.suffix_second_lo.get(start)?)?;
        bucket.records.get(second).copied()
    }

    fn nearest_ahead(
        &self,
        edge: LaneEdgeOrdinal,
        self_vehicle: VehicleHandle,
        front_mm: u32,
    ) -> Option<OccupancyRecord> {
        let bucket = self.bucket(edge)?;
        let skip = bucket
            .records
            .partition_point(|record| record.hi_mm <= front_mm);
        self.min_lo_from(bucket, skip, self_vehicle)
    }

    fn front_most(
        &self,
        edge: LaneEdgeOrdinal,
        self_vehicle: VehicleHandle,
    ) -> Option<OccupancyRecord> {
        let bucket = self.bucket(edge)?;
        self.min_lo_from(bucket, 0, self_vehicle)
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
        self.nearest_leader(
            self_vehicle,
            follower_edges,
            follower_index,
            follower_progress,
            lengths,
            horizon,
            false,
        )
        .map(|contact| contact.gap_mm)
    }

    /// 与 [`Self::leader_gap`] 同一行走，并保留最近前车的身份。
    ///
    /// `non_negative` 时跳过后杠已经落在跟随者前杠之后的片段，供新鲜摆放使用。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn nearest_leader(
        &self,
        self_vehicle: VehicleHandle,
        follower_edges: &[LaneEdgeOrdinal],
        follower_index: usize,
        follower_progress: u32,
        lengths: &[u32],
        horizon: LeaderQueryHorizon,
        non_negative: bool,
    ) -> Option<LeaderContact> {
        let walk = i64::from(horizon.front_query_mm);
        let accept = i64::from(horizon.bumper_gap_mm);
        let current = *follower_edges.get(follower_index)?;
        let mut best = self
            .nearest_ahead(current, self_vehicle, follower_progress)
            .map(|record| LeaderContact {
                vehicle: record.vehicle,
                gap_mm: i64::from(record.lo_mm) - i64::from(follower_progress),
            })
            .filter(|contact| contact.gap_mm <= accept && (!non_negative || contact.gap_mm >= 0));
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
            if best.is_some_and(|current| base_mm > current.gap_mm) {
                break;
            }
            self.note_occurrence_walk();
            if let Some(record) = self.front_most(edge, self_vehicle)
                && let Some(gap) = base_mm
                    .checked_add(i64::from(record.lo_mm))
                    .filter(|gap| *gap <= accept && (!non_negative || *gap >= 0))
                && best.is_none_or(|current| gap < current.gap_mm)
            {
                best = Some(LeaderContact {
                    vehicle: record.vehicle,
                    gap_mm: gap,
                });
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

    /// `hi_mm` 落在闭区间内的记录。桶按前杠排序，窗外的车不访问。
    pub(crate) fn for_each_record_in_hi_window(
        &self,
        edge: LaneEdgeOrdinal,
        min_hi: u32,
        max_hi: u32,
        mut visit: impl FnMut(VehicleHandle, u32),
    ) {
        if min_hi > max_hi {
            return;
        }
        let Some(records) = self.bucket(edge).map(|bucket| bucket.records.as_slice()) else {
            return;
        };
        let from = records.partition_point(|record| record.hi_mm < min_hi);
        let to = records.partition_point(|record| record.hi_mm <= max_hi);
        for record in &records[from..to] {
            visit(record.vehicle, record.update_sequence);
        }
    }

    /// 按将要插入的边预留空位。只增长被插入的边，不移动其他边的记录。
    fn reserve_insert_slots(&mut self, extra: &[OccupancyRecord]) -> Result<(), StepError> {
        for (index, record) in extra.iter().enumerate() {
            if extra[..index]
                .iter()
                .any(|earlier| earlier.bucket == record.bucket)
            {
                continue;
            }
            let count = extra
                .iter()
                .filter(|item| item.bucket == record.bucket)
                .count();
            let bucket = self
                .buckets
                .get_mut(record.bucket.index())
                .ok_or(StepError::OccupancyIntervalIncomplete)?;
            let needed = bucket
                .records
                .len()
                .checked_add(count)
                .ok_or(StepError::OccupancyAllocFailed)?;
            try_reserve_len(&mut bucket.records, needed)?;
            try_reserve_len(&mut bucket.suffix_min_lo, needed)?;
            try_reserve_len(&mut bucket.suffix_second_lo, needed)?;
        }
        Ok(())
    }

    /// 把新记录插进各自的桶。调用前容量必须已经够，这里不再分配。
    pub(crate) fn insert_reserved_records(&mut self, extra: &[OccupancyRecord]) {
        for record in extra {
            self.insert_reserved_record(*record);
        }
    }

    fn insert_reserved_record(&mut self, record: OccupancyRecord) {
        let Some(bucket) = self.buckets.get_mut(record.bucket.index()) else {
            debug_assert!(false, "spawn occupancy bucket was reserved");
            return;
        };
        let key = (
            record.hi_mm,
            record.lo_mm,
            record.update_sequence,
            record.vehicle.index(),
        );
        let at = bucket.records.partition_point(|existing| {
            (
                existing.hi_mm,
                existing.lo_mm,
                existing.update_sequence,
                existing.vehicle.index(),
            ) < key
        });
        debug_assert!(bucket.records.len() < bucket.records.capacity());
        debug_assert!(bucket.suffix_min_lo.len() < bucket.suffix_min_lo.capacity());
        debug_assert!(bucket.suffix_second_lo.len() < bucket.suffix_second_lo.capacity());
        bucket.records.insert(at, record);
        bucket.suffix_min_lo.insert(at, 0);
        bucket.suffix_second_lo.insert(at, SUFFIX_NONE);
        bucket.fill_suffix();
        self.record_len = self.record_len.saturating_add(1);
    }

    #[cfg(test)]
    pub(crate) fn record_keys(&self) -> Vec<(u32, u32, u32, u32, u32)> {
        self.buckets
            .iter()
            .flat_map(|bucket| bucket.records.iter())
            .map(|record| {
                (
                    record.bucket.0,
                    record.lo_mm,
                    record.hi_mm,
                    record.vehicle.index(),
                    record.update_sequence,
                )
            })
            .collect()
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
    Some(slot.compiled.as_ref()?.edges.as_slice())
}

fn visit_occupancy_records_with(
    live_order: &[VehicleHandle],
    vehicles: &[VehicleSlot],
    revision: &SharedNetworkRevision,
    routes: &[RouteSlot],
    staged_by_slot: &[Option<&CompiledRoute>],
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
        for_each_admission_interval(
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
        for_each_admission_interval(
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

fn rebuild_occupancy_index(
    binding: &crate::kernel::state::WorldBindingState,
    committed: &crate::kernel::state::CommittedWorldState,
    active_order: &[VehicleHandle],
    occupancy: &mut OccupancyIndex,
    scratch: &mut OccupancyScratch,
) -> Result<(), StepError> {
    #[cfg(test)]
    if super::exact_path_research::candidate_enabled() {
        return exact_candidate::rebuild(binding, committed, active_order, occupancy, scratch);
    }
    #[cfg(test)]
    let count_timer =
        super::exact_path_research::begin(super::exact_path_research::Stage::OccupancyCount);
    let bucket_count = usize::try_from(binding.revision.traffic().lane_edge_count())
        .expect("lane edge count fits usize");
    let ceiling = occupancy_record_limit(binding.config.vehicle_capacity());
    #[cfg(test)]
    occupancy.reset_inspections();
    occupancy.try_prepare_scratch(scratch, bucket_count)?;
    visit_occupancy_records(
        active_order,
        &committed.vehicles,
        &binding.revision,
        &committed.routes,
        |record| {
            if let Some(count) = scratch.positions.get_mut(record.bucket.index()) {
                *count += 1;
            }
        },
    )?;
    let total = scratch.record_total(bucket_count);
    if total > ceiling {
        return Err(StepError::OccupancyCapacityExceeded);
    }
    #[cfg(test)]
    drop(count_timer);
    #[cfg(test)]
    let layout_timer =
        super::exact_path_research::begin(super::exact_path_research::Stage::OccupancyLayout);
    occupancy.try_reserve_records(scratch, bucket_count)?;
    occupancy.finish_layout(scratch, bucket_count);
    #[cfg(test)]
    drop(layout_timer);
    #[cfg(test)]
    let fill_timer =
        super::exact_path_research::begin(super::exact_path_research::Stage::OccupancyFill);
    visit_occupancy_records(
        active_order,
        &committed.vehicles,
        &binding.revision,
        &committed.routes,
        |record| occupancy.write_record(scratch, record),
    )?;
    #[cfg(test)]
    drop(fill_timer);
    #[cfg(test)]
    let _sort_timer =
        super::exact_path_research::begin(super::exact_path_research::Stage::OccupancySortSuffix);
    occupancy.sort_buckets(bucket_count);
    Ok(())
}

#[cfg(test)]
#[path = "tests/occupancy_exact_candidate.rs"]
pub(crate) mod exact_candidate;

impl crate::kernel::state::WorldState {
    /// 针对给定根与 staged 路线纯构造一份占用索引（不触及活动状态）。
    ///
    /// 供切换事务在 Prepare 段完成可失败的重建（#302 切换合同 §4：
    /// 全部可失败步骤先于换绑），commit 段只做不可失败的替换。
    pub(crate) fn build_occupancy_index_for(
        &self,
        revision: &SharedNetworkRevision,
        routes_staged: &[(usize, CompiledRoute)],
    ) -> Result<(OccupancyIndex, OccupancyScratch), StepError> {
        let bucket_count = usize::try_from(revision.traffic().lane_edge_count())
            .expect("lane edge count fits usize");
        let ceiling = occupancy_record_limit(self.binding.config.vehicle_capacity());
        let (mut staged, mut scratch) = OccupancyIndex::try_empty()?;
        staged.try_prepare_scratch(&mut scratch, bucket_count)?;
        let mut staged_by_slot: Vec<Option<&CompiledRoute>> = Vec::new();
        try_reserve_len(&mut staged_by_slot, self.committed.routes.len())?;
        staged_by_slot.resize(self.committed.routes.len(), None);
        for (index, compiled) in routes_staged {
            if let Some(slot) = staged_by_slot.get_mut(*index) {
                *slot = Some(compiled);
            }
        }
        visit_occupancy_records_with(
            &self.derived.active_order,
            &self.committed.vehicles,
            revision,
            &self.committed.routes,
            &staged_by_slot,
            |record| {
                if let Some(count) = scratch.positions.get_mut(record.bucket.index()) {
                    *count += 1;
                }
            },
        )?;
        let total = scratch.record_total(bucket_count);
        if total > ceiling {
            return Err(StepError::OccupancyCapacityExceeded);
        }
        staged.try_reserve_records(&scratch, bucket_count)?;
        staged.finish_layout(&mut scratch, bucket_count);
        visit_occupancy_records_with(
            &self.derived.active_order,
            &self.committed.vehicles,
            revision,
            &self.committed.routes,
            &staged_by_slot,
            |record| staged.write_record(&mut scratch, record),
        )?;
        staged.sort_buckets(bucket_count);
        Ok((staged, scratch))
    }

    pub(crate) fn rebuild_occupancy_index(&mut self) -> Result<(), StepError> {
        #[cfg(test)]
        super::parking_command_research::note(|counts| {
            counts.occupancy_builds += 1;
            counts.occupancy_inputs += self.derived.active_order.len();
        });
        self.derived.occupancy.source = None;
        rebuild_occupancy_index(
            &self.binding,
            &self.committed,
            &self.derived.active_order,
            &mut self.derived.occupancy,
            &mut self.workspace.occupancy_scratch,
        )?;
        self.derived.occupancy.source = Some((
            self.binding.world_generation,
            self.committed.observation_state_sequence,
        ));
        #[cfg(test)]
        OCCUPANCY_REBUILD_EVENTS.with(|events| events.set(events.get().saturating_add(1)));
        #[cfg(test)]
        super::parking_command_research::note(|counts| {
            counts.occupancy_records += self.derived.occupancy.record_count();
        });
        Ok(())
    }

    /// 成功插入后把这一辆车补进派生索引。来源版本对不上时作废索引，留给下一次命令重建。
    ///
    /// 容量和分配在提交车辆之前由 [`Self::reserve_spawn_occupancy`] 预检。
    pub(crate) fn apply_spawn_occupancy(
        &mut self,
        handle: VehicleHandle,
        previous: ObservationStateSequence,
        mut records: Vec<OccupancyRecord>,
    ) {
        for record in &mut records {
            record.vehicle = handle;
        }
        let generation = self.binding.world_generation;
        if self.derived.occupancy.source != Some((generation, previous)) {
            self.derived.occupancy.source = None;
            return;
        }
        self.derived.occupancy.insert_reserved_records(&records);
        self.derived.occupancy.source =
            Some((generation, self.committed.observation_state_sequence));
    }

    /// 提交前确认增量插入放得下。索引来源不是当前序号时返回空，提交后再作废。
    pub(crate) fn reserve_spawn_occupancy(
        &mut self,
        input: crate::VehicleSpawnInput,
        vehicle_length_mm: u32,
        update_sequence: u32,
    ) -> Result<Vec<OccupancyRecord>, StepError> {
        let generation = self.binding.world_generation;
        if self.derived.occupancy.source
            != Some((generation, self.committed.observation_state_sequence))
        {
            return Ok(Vec::new());
        }
        let edges = self
            .route_edges(input.route())
            .ok_or(StepError::OccupancyIntervalIncomplete)?;
        let cursor = usize::try_from(input.route_edge_index())
            .map_err(|_| StepError::OccupancyIntervalIncomplete)?;
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
        let mut interval_count = 0usize;
        for_each_admission_interval(
            lengths,
            edges,
            cursor,
            input.progress_mm(),
            vehicle_length_mm,
            |_, _, _| {
                interval_count = interval_count.saturating_add(1);
            },
        )
        .ok_or(StepError::OccupancyIntervalIncomplete)?;
        let mut records = Vec::new();
        records
            .try_reserve(interval_count)
            .map_err(|_| StepError::OccupancyAllocFailed)?;
        for_each_admission_interval(
            lengths,
            edges,
            cursor,
            input.progress_mm(),
            vehicle_length_mm,
            |edge, lo_mm, hi_mm| {
                records.push(OccupancyRecord {
                    vehicle: crate::VehicleHandle::new(0, 0),
                    bucket: OccupancyBucketOrdinal::from_edge(edge),
                    lo_mm,
                    hi_mm,
                    update_sequence,
                });
            },
        )
        .ok_or(StepError::OccupancyIntervalIncomplete)?;
        let ceiling = occupancy_record_limit(self.binding.config.vehicle_capacity());
        if self
            .derived
            .occupancy
            .record_count()
            .saturating_add(records.len())
            > ceiling
        {
            return Err(StepError::OccupancyCapacityExceeded);
        }
        self.derived.occupancy.reserve_insert_slots(&records)?;
        Ok(records)
    }

    /// 两次 step 之间的命令读取当前提交态；重复拒绝可以复用同一份索引。
    pub(crate) fn ensure_current_occupancy(&mut self) -> Result<(), StepError> {
        let source = (
            self.binding.world_generation,
            self.committed.observation_state_sequence,
        );
        if self.derived.occupancy.source != Some(source) {
            self.rebuild_occupancy_index()?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn occupancy_inspections(&self) -> u64 {
        self.derived.occupancy.inspections()
    }
}

#[cfg(test)]
pub(crate) mod tests {
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

    use crate::kernel::tables::{
        occupancy_front_gap, remaining_along_route_i64, with_route_allocation_failure_after,
    };
    use crate::kernel::tick::leader_query_horizon;
    use crate::kernel::units::ceil_mm;
    use crate::{
        ParkedVehicleSpawnInput, ParkingTarget, RouteError, RouteRegisterInput, StepError,
        TickInput, VehicleSpawnInput, WorldConfig,
    };

    fn install_fixture(
        revision: std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
        config: crate::WorldConfig,
    ) -> Result<crate::TrafficWorld, crate::InstallError> {
        let origin = *revision.canonical_origin();
        crate::TrafficWorld::install(
            std::sync::Arc::clone(&revision),
            config,
            crate::ExecutionConfig::new(std::num::NonZeroU32::MIN),
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
            crate::test_policy::selection(&revision),
        )
    }

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
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

    pub(super) fn compile_revision(
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

    pub(super) fn add_car_profile(module: &mut SyntheticModuleBuilder) {
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
        let lengths = world
            .state
            .binding
            .revision
            .traffic()
            .lane_lengths_millimetres();
        let edges = world.route_edges(state.route).unwrap();
        world.state.leader_bumper_gap(state, edges, lengths)
    }

    fn assert_index_matches_scan(world: &TrafficWorld) {
        let lengths = world
            .state
            .binding
            .revision
            .traffic()
            .lane_lengths_millimetres();
        for handle in world.state.committed.live_order.iter().copied() {
            let Some(state) = world.state.vehicle_state(handle) else {
                continue;
            };
            if state.status != VehicleStatus::Active {
                continue;
            }
            let Some(edges) = world.route_edges(state.route) else {
                continue;
            };
            let cursor = usize::try_from(state.route_edge_index).unwrap();
            let horizon = world.state.leader_query_horizon_for(state);
            let indexed = world.state.derived.occupancy.leader_gap(
                state.handle,
                edges,
                cursor,
                state.progress_mm,
                lengths,
                horizon,
            );
            let scanned = world.state.leader_bumper_gap_scan(state, edges, lengths);
            let wrapped = world.state.leader_bumper_gap(state, edges, lengths);
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
            .state
            .committed
            .live_order
            .iter()
            .copied()
            .filter(|handle| {
                world
                    .state
                    .vehicle_state(*handle)
                    .is_some_and(|state| state.status == VehicleStatus::Active)
            })
            .count() as u64
    }

    pub(crate) fn zero_progress_merge_fixture() -> (TrafficWorld, VehicleHandle, VehicleHandle) {
        let revision = compile_revision(|module| {
            add_car_profile(module);
            for (key, length) in [("left", 10.0), ("right", 11.0)] {
                module
                    .add_lane_edge(LaneEdgeInput {
                        lane_edge_key: key,
                        length_meters: length,
                        speed_limit_meters_per_second: 10.0,
                        successors: &[laneflow_compiler::LaneEdgeReference::local("shared")],
                    })
                    .unwrap();
            }
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "shared",
                    length_meters: 12.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[],
                })
                .unwrap();
        });
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 16)).unwrap();
        let left = edge_for_length(&world, 10_000);
        let right = edge_for_length(&world, 11_000);
        let shared = edge_for_length(&world, 12_000);
        let follower_route = world
            .register_route(RouteRegisterInput::new(vec![left, shared]))
            .unwrap();
        let leader_route = world
            .register_route(RouteRegisterInput::new(vec![right, shared]))
            .unwrap();
        let profile = VehicleProfileOrdinal::from_raw(0);
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                profile,
                follower_route,
                0,
                9_998,
                3_000,
            ))
            .unwrap();
        let leader = world
            .state
            .place_existing_active_vehicle(VehicleSpawnInput::new(
                profile,
                leader_route,
                1,
                0,
                4_000,
            ))
            .unwrap();
        world.state.rebuild_occupancy_index().unwrap();
        (world, follower, leader)
    }

    #[test]
    fn current_occupancy_tracks_step_slot_reuse_generation_and_failed_rebuild() {
        fn matches_fresh(world: &mut TrafficWorld) {
            world.state.ensure_current_occupancy().unwrap();
            let (fresh, _) = world
                .state
                .build_occupancy_index_for(world.state.binding.revision.as_ref(), &[])
                .unwrap();
            assert!(
                world.state.derived.occupancy.same_layout(&fresh),
                "incremental occupancy diverged from a fresh rebuild"
            );
        }
        let (mut world, first, _) = zero_progress_merge_fixture();
        matches_fresh(&mut world);
        world.step(TickInput::new(16)).unwrap();
        matches_fresh(&mut world);
        let old = world.vehicle(first).unwrap();
        world.despawn_vehicle(first).unwrap();
        let new = world
            .spawn_vehicle(VehicleSpawnInput::new(old.profile, old.route, 0, 1_000, 0))
            .unwrap();
        assert_eq!(new.index(), first.index());
        assert_ne!(new.generation(), first.generation());
        matches_fresh(&mut world);
        world.state.binding.world_generation =
            world.state.binding.world_generation.checked_next().unwrap();
        matches_fresh(&mut world);
        let valid = world.vehicle(new).unwrap();
        world.state.committed.vehicles[new.index() as usize]
            .state
            .as_mut()
            .unwrap()
            .route_edge_index = u32::MAX;
        assert!(world.state.rebuild_occupancy_index().is_err());
        assert!(world.state.derived.occupancy.source.is_none());
        world.state.committed.vehicles[new.index() as usize].state = Some(valid);
        matches_fresh(&mut world);
    }

    #[test]
    fn merged_zero_progress_front_is_visible_to_the_other_incoming_branch() {
        let (world, follower, _) = zero_progress_merge_fixture();
        let state = world.state.vehicle_state(follower).unwrap();
        let lengths = world.traffic().lane_lengths_millimetres();
        let edges = world.route_edges(state.route()).unwrap();
        assert_eq!(index_gap(&world, state), Some(2));
        assert_eq!(
            world.state.leader_bumper_gap_scan(state, edges, lengths),
            Some(2)
        );
    }

    #[test]
    fn merged_zero_progress_front_prevents_next_tick_shared_edge_overlap() {
        let (mut world, follower, leader) = zero_progress_merge_fixture();
        world.step(TickInput::new(16)).unwrap();
        let follower = world.state.vehicle_state(follower).unwrap();
        let leader = world.state.vehicle_state(leader).unwrap();
        assert_eq!(
            follower.route_edge_index(),
            0,
            "follower entered the occupied shared edge: {follower:?}; leader={leader:?}"
        );
        assert_index_matches_scan(&world);
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
    fn inserting_one_lane_does_not_move_other_lanes() {
        fn record(
            edge: LaneEdgeOrdinal,
            vehicle: u32,
            lo_mm: u32,
            hi_mm: u32,
            update_sequence: u32,
        ) -> OccupancyRecord {
            OccupancyRecord {
                vehicle: VehicleHandle::new(vehicle, 0),
                bucket: OccupancyBucketOrdinal::from_edge(edge),
                lo_mm,
                hi_mm,
                update_sequence,
            }
        }

        let early = LaneEdgeOrdinal::from_raw(0);
        let late = LaneEdgeOrdinal::from_raw(5);
        let farther = LaneEdgeOrdinal::from_raw(7);
        let (mut index, mut scratch) = OccupancyIndex::with_capacity(8, 0);
        let mut pending = vec![
            record(late, 1, 1_000, 2_000, 0),
            record(early, 2, 0, 500, 1),
        ];
        index.rebuild_from_pending(&mut scratch, &pending, 8);
        let late_records = index.buckets[5].records.as_ptr();
        let late_suffix = index.buckets[5].suffix_min_lo.as_ptr();
        let late_second = index.buckets[5].suffix_second_lo.as_ptr();
        let added_early = record(early, 3, 800, 1_200, 2);
        index
            .reserve_insert_slots(&[added_early])
            .expect("reserve the touched lane");
        assert_eq!(
            index.buckets[5].records.as_ptr(),
            late_records,
            "reserving an earlier lane must not move a later lane"
        );
        index.insert_reserved_record(added_early);
        assert_eq!(index.buckets[5].records.as_ptr(), late_records);
        assert_eq!(index.buckets[5].suffix_min_lo.as_ptr(), late_suffix);
        assert_eq!(index.buckets[5].suffix_second_lo.as_ptr(), late_second);
        assert_eq!(index.buckets[5].records.len(), 1);
        let early_records = index.buckets[0].records.as_ptr();
        let added_far = record(farther, 4, 100, 400, 3);
        index
            .reserve_insert_slots(&[added_far])
            .expect("reserve the farther lane");
        assert_eq!(index.buckets[0].records.as_ptr(), early_records);
        assert_eq!(index.buckets[5].records.as_ptr(), late_records);
        index.insert_reserved_record(added_far);
        assert_eq!(
            index.buckets[0].records.as_ptr(),
            early_records,
            "inserting a later lane must not move an earlier lane"
        );
        assert_eq!(index.buckets[5].records.as_ptr(), late_records);
        let summed = index
            .buckets
            .iter()
            .map(|bucket| bucket.records.len())
            .sum::<usize>();
        assert_eq!(
            index.records_len(),
            summed,
            "cached record total must match the buckets after inserts"
        );
        assert_eq!(index.buckets[5].suffix_min_lo.as_ptr(), late_suffix);
        pending.push(added_early);
        pending.push(added_far);
        let (mut fresh, mut fresh_scratch) = OccupancyIndex::with_capacity(8, 0);
        fresh.rebuild_from_pending(&mut fresh_scratch, &pending, 8);
        assert_eq!(fresh.records_len(), pending.len());
        assert!(
            index.same_layout(&fresh),
            "bucket insert must match a full rebuild"
        );
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
        let (mut occupancy, mut scratch) = OccupancyIndex::with_capacity(2, 40);
        occupancy.rebuild_from_pending(&mut scratch, &pending, 2);
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
        let (mut index, mut scratch) = OccupancyIndex::with_capacity(1, 2);
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
        index.rebuild_from_pending(&mut scratch, &pending, 1);
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
        let (mut index, mut scratch) = OccupancyIndex::with_capacity(1, 4);
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
        index.rebuild_from_pending(&mut scratch, &pending, 1);
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
        let (mut index, mut scratch) = OccupancyIndex::with_capacity(2, 3);
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
        index.rebuild_from_pending(&mut scratch, &pending, 2);
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
        let (mut index, mut scratch) = OccupancyIndex::with_capacity(2, 2);
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
        index.rebuild_from_pending(&mut scratch, &pending, 2);
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
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100)).unwrap();
        let route = register_full_spatial_route(&mut world);
        let profile = world
            .state
            .binding
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
        world
            .state
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
        assert_index_matches_scan(&world);
        world.step(TickInput::new(100)).unwrap();
        world
            .state
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
        assert_index_matches_scan(&world);
    }

    #[test]
    fn empty_and_solo_vehicle_have_no_leader() {
        let revision = two_edge_revision();
        let stem = LaneEdgeOrdinal::from_raw(0);
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100)).expect("install");
        world
            .state
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
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
        world
            .state
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
        let state = world.state.vehicle_state(solo).copied().unwrap();
        assert_eq!(index_gap(&world, &state), None);
        assert_index_matches_scan(&world);
    }

    #[test]
    fn vehicle_behind_on_current_edge_is_not_leader() {
        let revision = two_edge_revision();
        let stem = LaneEdgeOrdinal::from_raw(0);
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![stem]))
            .expect("route");
        let profile = world
            .state
            .binding
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
        world
            .state
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
        let state = world.state.vehicle_state(follower).copied().unwrap();
        assert_eq!(index_gap(&world, &state), None);
        assert_index_matches_scan(&world);
    }

    #[test]
    fn leader_fully_on_next_edge_matches_scan() {
        let revision = two_edge_revision();
        let stem = LaneEdgeOrdinal::from_raw(0);
        let tail = LaneEdgeOrdinal::from_raw(1);
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![stem, tail]))
            .expect("route");
        let profile = world
            .state
            .binding
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
        let stem_len = world
            .state
            .binding
            .revision
            .traffic()
            .lane_lengths_millimetres()[stem.index()];
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                stem_len.saturating_sub(1_000),
                0,
            ))
            .expect("follower on stem");
        world
            .state
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
        assert_index_matches_scan(&world);
        let state = world.state.vehicle_state(follower).copied().unwrap();
        let gap = index_gap(&world, &state).expect("next-edge leader inside bumper window");
        assert!(gap > 0, "next-edge rear bumper must be ahead, gap={gap}");
    }

    #[test]
    fn cycle_wrap_uses_later_occurrence_of_vehicle_behind() {
        let revision = loop_revision();
        let a = LaneEdgeOrdinal::from_raw(0);
        let b = LaneEdgeOrdinal::from_raw(1);
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100)).expect("install");
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
        world
            .state
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
        assert_index_matches_scan(&world);
        let state = world.state.vehicle_state(follower).copied().unwrap();
        assert_eq!(
            index_gap(&world, &state),
            None,
            "wrap gap is tens of metres, beyond rest bumper_gap_horizon"
        );
        let lengths = world
            .state
            .binding
            .revision
            .traffic()
            .lane_lengths_millimetres();
        let edges = world.route_edges(state.route).unwrap();
        let cursor = usize::try_from(state.route_edge_index).unwrap();
        let unbounded = world.state.derived.occupancy.leader_gap(
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
            install_fixture(revision, WorldConfig::new(8, 4, 3, 3, 100)).expect("install");

        let route = world
            .register_route(RouteRegisterInput::new(vec![a, b, a]))
            .expect("three occurrences exactly fill capacity");
        assert_eq!(world.state.committed.live_route_count, 1);
        assert_eq!(world.state.committed.live_route_edge_occurrence_count, 3);
        let route_slots = world.state.committed.routes.len();

        assert_eq!(
            world
                .register_route(RouteRegisterInput::new(vec![a]))
                .unwrap_err(),
            RouteError::EdgeOccurrenceCapacityExceeded
        );
        assert_eq!(world.state.committed.live_route_count, 1);
        assert_eq!(world.state.committed.live_route_edge_occurrence_count, 3);
        assert_eq!(world.state.committed.routes.len(), route_slots);

        world
            .remove_route(route)
            .expect("unused route releases all occurrences");
        assert_eq!(world.state.committed.live_route_count, 0);
        assert_eq!(world.state.committed.live_route_edge_occurrence_count, 0);

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
        assert_eq!(world.state.committed.live_route_edge_occurrence_count, 3);
        assert_eq!(
            world
                .register_route(RouteRegisterInput::new(vec![a]))
                .unwrap_err(),
            RouteError::EdgeOccurrenceCapacityExceeded
        );
    }

    #[test]
    fn route_compilation_allocation_failure_leaves_world_unchanged() {
        let revision = loop_revision();
        let a = LaneEdgeOrdinal::from_raw(0);
        let b = LaneEdgeOrdinal::from_raw(1);
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 3, 3, 100)).expect("install");

        for successful_reservations in [0, 5] {
            let result = with_route_allocation_failure_after(successful_reservations, || {
                world.register_route(RouteRegisterInput::new(vec![a, b, a]))
            });
            assert_eq!(result.unwrap_err(), RouteError::AllocationFailed);
            assert_eq!(world.state.committed.live_route_count, 0);
            assert_eq!(world.state.committed.live_route_edge_occurrence_count, 0);
            assert!(world.state.committed.routes.is_empty());
            assert!(world.state.committed.free_routes.is_empty());
        }

        let route = world
            .register_route(RouteRegisterInput::new(vec![a, b, a]))
            .expect("failpoint reset leaves world reusable");
        assert_eq!(world.route_edges(route), Some([a, b, a].as_slice()));
        assert_eq!(world.state.committed.live_route_count, 1);
        assert_eq!(world.state.committed.live_route_edge_occurrence_count, 3);
    }

    #[test]
    fn route_registration_preflight_has_stable_error_priority() {
        let revision = loop_revision();
        let a = LaneEdgeOrdinal::from_raw(0);
        let mut no_route_slots =
            install_fixture(Arc::clone(&revision), WorldConfig::new(8, 0, 0, 0, 100))
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
            install_fixture(revision, WorldConfig::new(8, 1, 0, 0, 100)).expect("install");
        assert_eq!(
            no_occurrences
                .register_route(RouteRegisterInput::new(vec![a]))
                .unwrap_err(),
            RouteError::EdgeOccurrenceCapacityExceeded
        );
        assert_eq!(no_occurrences.state.committed.live_route_count, 0);
        assert_eq!(
            no_occurrences
                .state
                .committed
                .live_route_edge_occurrence_count,
            0
        );
        assert!(no_occurrences.state.committed.routes.is_empty());

        let mut overflow = install_fixture(
            loop_revision(),
            WorldConfig::new(8, 1, u64::MAX, u64::MAX, 100),
        )
        .expect("install");
        overflow.state.committed.live_route_edge_occurrence_count = u64::MAX;
        assert_eq!(
            overflow
                .register_route(RouteRegisterInput::new(vec![a]))
                .unwrap_err(),
            RouteError::EdgeOccurrenceCapacityExceeded
        );
        assert_eq!(overflow.state.committed.live_route_count, 0);
        assert_eq!(
            overflow.state.committed.live_route_edge_occurrence_count,
            u64::MAX
        );
        assert!(overflow.state.committed.routes.is_empty());
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
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100)).unwrap();
        let route = register_full_spatial_route(&mut world);
        world.state.committed.routes[route.index() as usize]
            .compiled
            .as_mut()
            .expect("route")
            .waiting
            .clear();
        let profile = world
            .state
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .unwrap();
        let _parked = world
            .spawn_parked_vehicle(
                ParkedVehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    1_000 + profile.length_mm() + profile.min_gap_mm() + 2_000,
                ),
                ParkingTarget::ExplicitSpace(ParkingSpaceOrdinal::from_raw(0)),
            )
            .unwrap()
            .vehicle;
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
            .state
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
        let follower_state = world.state.vehicle_state(follower).copied().unwrap();
        assert_eq!(index_gap(&world, &follower_state), None);
        assert_index_matches_scan(&world);

        let revision = two_edge_revision();
        let stem = LaneEdgeOrdinal::from_raw(0);
        let tail = LaneEdgeOrdinal::from_raw(1);
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![stem, tail]))
            .expect("route");
        let tail_len = world
            .state
            .binding
            .revision
            .traffic()
            .lane_lengths_millimetres()[tail.index()];
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
        world
            .state
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
        let follower_state = world.state.vehicle_state(follower).copied().unwrap();
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
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100)).expect("install");
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
            .state
            .restore_unparked_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    follower_route,
                    0,
                    5_000,
                    10_000,
                ),
                0,
                crate::VehicleStatus::Active,
                None,
                None,
                false,
            )
            .expect("follower");
        world
            .state
            .workspace
            .frontier_maintenance
            .note_active_source(follower);
        world
            .state
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
        assert_index_matches_scan(&world);
        let follower_state = world.state.vehicle_state(follower).copied().unwrap();
        let leader_state = world
            .state
            .committed
            .live_order
            .iter()
            .copied()
            .find_map(|handle| {
                let state = world.state.vehicle_state(handle)?;
                (handle != follower).then_some(*state)
            })
            .expect("leader state");
        let lengths = world
            .state
            .binding
            .revision
            .traffic()
            .lane_lengths_millimetres();
        let follower_edges = world.route_edges(follower_state.route).unwrap();
        let leader_edges = world.route_edges(leader_state.route).unwrap();
        let horizon = world.state.leader_query_horizon_for(&follower_state);
        let indexed = world.state.derived.occupancy.leader_gap(
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
            install_fixture(revision, WorldConfig::new(n, 4, 1_024, 1_024, 100)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![edge]))
            .expect("route");
        let profile = world
            .state
            .binding
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
        let inspections = world.state.occupancy_inspections();
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
        world
            .state
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
        assert_index_matches_scan(&world);
    }

    #[test]
    fn repeated_edge_prefers_nearer_occurrence() {
        let revision = loop_revision();
        let a = LaneEdgeOrdinal::from_raw(0);
        let b = LaneEdgeOrdinal::from_raw(1);
        let mut world =
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100)).expect("install");
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
        world
            .state
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
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
        let mut world = install_fixture(revision, WorldConfig::new(1, 4, 1_024, 1_024, 1_000))
            .expect("install");
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
        let cap = world.state.derived.occupancy.records_capacity();
        let len = world.state.derived.occupancy.records_len();
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
            let next = world.state.derived.occupancy.records_capacity();
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
                world.state.derived.occupancy.records_capacity(),
                high_water,
                "after body-span high-water, ticks must not grow occupancy record capacity"
            );
        }
    }

    #[test]
    fn large_vehicle_capacity_does_not_reserve_envelope() {
        let revision = two_edge_revision();
        let stem = LaneEdgeOrdinal::from_raw(0);
        let mut world = install_fixture(revision, WorldConfig::new(10_000, 4, 1_024, 1_024, 100))
            .expect("install");
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
        let cap = world.state.derived.occupancy.records_capacity();
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
            world.state.derived.occupancy.suffix_min_lo_capacity() < 256,
            "suffix min table must follow actual records, cap={}",
            world.state.derived.occupancy.suffix_min_lo_capacity()
        );
        assert!(
            world.state.derived.occupancy.suffix_second_lo_capacity() < 256,
            "suffix second table must follow actual records, cap={}",
            world.state.derived.occupancy.suffix_second_lo_capacity()
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
        let (mut index, mut scratch) = OccupancyIndex::with_capacity(1, 2);
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
        index.rebuild_from_pending(&mut scratch, &pending, 1);
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
        let mut scratch = OccupancyScratch {
            positions: Vec::new(),
            maneuver_upstream_offsets: Vec::new(),
            maneuver_upstream_sources: Vec::new(),
            follower_bumper_mm: None,
            exact_pending: Vec::new(),
        };
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
        index.rebuild_from_pending(&mut scratch, &pending, 1);
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
        let (mut occupancy, mut scratch) = OccupancyIndex::with_capacity(2, pending.len());
        occupancy.rebuild_from_pending(&mut scratch, &pending, 2);
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
        let (mut occupancy, mut scratch) = OccupancyIndex::with_capacity(2, pending.len());
        occupancy.rebuild_from_pending(&mut scratch, &pending, 2);
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
        let (mut occupancy, mut scratch) = OccupancyIndex::with_capacity(2, pending.len());
        occupancy.rebuild_from_pending(&mut scratch, &pending, 2);
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
        let (mut index, mut scratch) = OccupancyIndex::with_capacity(1, 2);
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
        index.rebuild_from_pending(&mut scratch, &pending, 1);
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
        let (mut occupancy, mut scratch) = OccupancyIndex::with_capacity(16, pending.len());
        occupancy.rebuild_from_pending(&mut scratch, &pending, 16);
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
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100)).expect("install");
        let edge = LaneEdgeOrdinal::from_raw(0);
        let route = world
            .register_route(RouteRegisterInput::new(vec![edge]))
            .expect("route");
        let profile = world
            .state
            .binding
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
        world
            .state
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
        let state = world.state.vehicle_state(follower).copied().unwrap();
        assert_eq!(index_gap(&world, &state), None);
        assert_index_matches_scan(&world);

        let phantom_progress = follower_progress
            .saturating_add(horizon.bumper_gap_mm)
            .saturating_add(profile.length_mm())
            .saturating_add(1);
        let mut phantom_world = install_fixture(
            long_corridor_revision(400.0),
            WorldConfig::new(8, 4, 1_024, 1_024, 100),
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
            .state
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
        let phantom_state = phantom_world
            .state
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
            WorldConfig::new(8, 4, 1_024, 1_024, 100),
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
            .state
            .rebuild_occupancy_index()
            .expect("occupancy rebuild");
        let near_state = near_world
            .state
            .vehicle_state(near_follower)
            .copied()
            .unwrap();
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
            let (mut occupancy, mut scratch) =
                OccupancyIndex::with_capacity(edge_count, pending.len());
            occupancy.rebuild_from_pending(&mut scratch, &pending, edge_count);
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
            let (mut occupancy, mut scratch) =
                OccupancyIndex::with_capacity(edge_count, pending.len());
            occupancy.rebuild_from_pending(&mut scratch, &pending, edge_count);
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
            install_fixture(revision, WorldConfig::new(8, 4, 1_024, 1_024, 100)).expect("install");
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
        let before_len = world.state.derived.occupancy.records_len();
        let before_time = world.state.committed.time_ms;
        let slot = usize::try_from(handle.index()).expect("vehicle index fits usize");
        world.state.committed.vehicles[slot]
            .state
            .as_mut()
            .expect("spawned vehicle")
            .route_edge_index = 10_000;
        assert_eq!(
            world.step(TickInput::new(100)),
            Err(StepError::OccupancyIntervalIncomplete)
        );
        assert_eq!(world.state.committed.time_ms, before_time);
        assert_eq!(
            world.state.derived.occupancy.records_len(),
            before_len,
            "failed rebuild must not replace occupancy records"
        );
    }

    #[test]
    fn candidate_preserves_existing_occupancy_oracles_and_boundaries() {
        crate::kernel::exact_path_research::with_candidate(true, || {
            full_spatial_follower_matches_scan_oracle();
            leader_fully_on_next_edge_matches_scan();
            cycle_wrap_uses_later_occurrence_of_vehicle_behind();
            parked_and_completed_are_not_leaders();
            diverge_overhang_matches_scan_and_occupancy_front_gap();
            formula_horizon_hides_leader_beyond_and_matches_filtered_scan();
            corrupt_route_index_fails_closed_occupancy_rebuild();
        });
    }
}
