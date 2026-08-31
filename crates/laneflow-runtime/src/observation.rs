//! 已提交交通观测导出（#303 G2）。
//!
//! 导出只在显式调用时从当前已提交车辆状态重算；无导出调用时不维护全网副本、
//! dirty journal 或后台任务。session 是调用方持有、Runtime 签发的进程内能力，
//! 不进入 Runtime Snapshot。

use core::mem::size_of;

use laneflow_static_contract::{LaneEdgeId, LaneEdgeOrdinal, NetworkRevisionId, Sha256Digest};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[cfg(test)]
use std::cell::Cell;

use crate::tables::for_each_occupancy_interval;
use crate::{TrafficWorld, VehicleStatus, WorldGeneration};

/// 观测进程内绑定契约版本。
pub const OBSERVATION_BINDING_VERSION: u16 = 1;

/// selection digest 的域分隔前缀；末尾 NUL 是摘要输入的一部分。
const OBSERVATION_SELECTION_DIGEST_DOMAIN: &[u8] = b"laneflow:runtime-observation-selection:v1\0";

/// 未选中物理边在 ordinal→selection 映射中的哨兵。
const NOT_SELECTED: usize = usize::MAX;

/// 当前观测 stream 内严格单调的已提交状态序号。
///
/// 安装及新世界世代从 [`Self::INITIAL`] 开始；成功 `step` 和真正改变 v1
/// 观测行的生命周期提交 checked 递增。该值不跨世界世代比较，也不进入快照。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ObservationStateSequence(u64);

impl ObservationStateSequence {
    /// 新观测 stream 的初始状态序号。
    pub const INITIAL: Self = Self(0);

    /// 日志、诊断及 Routing 绑定使用的精确 `u64` 值。
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    #[cfg(test)]
    pub(crate) const fn from_raw_for_test(value: u64) -> Self {
        Self(value)
    }
}

/// Runtime 签发的观测 stream 能力绑定。
///
/// 每个活动世界世代只存在一个观测 stream，因此不另维护会与共同世界世代漂移的
/// 第三计数器；`(world_id, world_generation)` 就是 stream 的唯一身份。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ObservationStreamBinding {
    world_id: u64,
    world_generation: WorldGeneration,
}

impl ObservationStreamBinding {
    const fn new(world_id: u64, world_generation: WorldGeneration) -> Self {
        Self {
            world_id,
            world_generation,
        }
    }

    /// 宿主指定的世界身份。
    #[must_use]
    pub const fn world_id(self) -> u64 {
        self.world_id
    }

    /// Runtime 拥有的活动世界世代。
    #[must_use]
    pub const fn world_generation(self) -> WorldGeneration {
        self.world_generation
    }
}

/// 观测分区选择。显式边集合必须非空、按稳定标识严格升序且无重复。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObservationSelection {
    /// 当前共享修订的全部物理 LaneEdge。
    AllLaneEdges,
    /// 调用方声明的显式 LaneEdge 稳定标识集合。
    ExplicitLaneEdges(Box<[LaneEdgeId]>),
}

/// 本次导出模式。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationExportMode {
    /// 完整基线，包含所选边的全零行。
    Full,
    /// 相对本 session 上一次成功批次的变化行。
    Delta,
}

/// delta 批次引用的上一成功交付位置。full 的 base 为 `None`。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationBatchBase {
    sequence: u64,
    tick: u64,
    observation_state_sequence: ObservationStateSequence,
}

impl ObservationBatchBase {
    /// 上一成功批次的 delivery sequence。
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }

    /// 上一成功批次的已提交 tick。
    #[must_use]
    pub const fn tick(self) -> u64 {
        self.tick
    }

    /// 上一成功批次的观测状态序号。
    #[must_use]
    pub const fn observation_state_sequence(self) -> ObservationStateSequence {
        self.observation_state_sequence
    }
}

/// 单条物理 LaneEdge 的已提交整数交通观测。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommittedTrafficObservationRow {
    lane_edge_stable_id: LaneEdgeId,
    front_vehicle_count: u32,
    occupied_length_mm: u64,
    front_speed_sum_mm_per_second: u64,
}

impl CommittedTrafficObservationRow {
    fn zero(lane_edge_stable_id: LaneEdgeId) -> Self {
        Self {
            lane_edge_stable_id,
            front_vehicle_count: 0,
            occupied_length_mm: 0,
            front_speed_sum_mm_per_second: 0,
        }
    }

    /// 物理 LaneEdge 稳定标识。
    #[must_use]
    pub const fn lane_edge_stable_id(self) -> LaneEdgeId {
        self.lane_edge_stable_id
    }

    /// 前保险杠当前归属该边的 Active 车辆数。
    #[must_use]
    pub const fn front_vehicle_count(self) -> u32 {
        self.front_vehicle_count
    }

    /// 该边所有 Active 车身已提交半开区间的并集长度（毫米）。
    #[must_use]
    pub const fn occupied_length_mm(self) -> u64 {
        self.occupied_length_mm
    }

    /// 前保险杠归属车辆的已提交速度和（毫米每秒）。
    #[must_use]
    pub const fn front_speed_sum_mm_per_second(self) -> u64 {
        self.front_speed_sum_mm_per_second
    }
}

/// 一次成功导出的完整、不可变批次。
#[derive(Debug, Eq, PartialEq)]
pub struct CommittedTrafficObservationBatch {
    binding_version: u16,
    stream: ObservationStreamBinding,
    network_revision: NetworkRevisionId,
    network_revision_derivation_version: u16,
    selection_digest: Sha256Digest,
    mode: ObservationExportMode,
    base: Option<ObservationBatchBase>,
    sequence: u64,
    tick: u64,
    observation_state_sequence: ObservationStateSequence,
    entry_count: u64,
    logical_bytes: u64,
    retained_bytes: u64,
    rows: Vec<CommittedTrafficObservationRow>,
}

impl CommittedTrafficObservationBatch {
    /// 进程内绑定契约版本。
    #[must_use]
    pub const fn binding_version(&self) -> u16 {
        self.binding_version
    }

    /// 世界/世代共同组成的观测 stream 绑定。
    #[must_use]
    pub const fn stream_binding(&self) -> ObservationStreamBinding {
        self.stream
    }

    /// 当前共享根的路网修订标识。
    #[must_use]
    pub const fn network_revision(&self) -> NetworkRevisionId {
        self.network_revision
    }

    /// 路网修订派生版本。
    #[must_use]
    pub const fn network_revision_derivation_version(&self) -> u16 {
        self.network_revision_derivation_version
    }

    /// 当前 session 不可变选择的摘要。
    #[must_use]
    pub const fn selection_digest(&self) -> Sha256Digest {
        self.selection_digest
    }

    /// full 或 delta。
    #[must_use]
    pub const fn mode(&self) -> ObservationExportMode {
        self.mode
    }

    /// delta 的上一成功基线；full 必为 `None`。
    #[must_use]
    pub const fn base(&self) -> Option<ObservationBatchBase> {
        self.base
    }

    /// 本 session 的唯一 delivery sequence。
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// 导出读取的已提交 tick。
    #[must_use]
    pub const fn tick(&self) -> u64 {
        self.tick
    }

    /// 导出读取的已提交观测状态序号。
    #[must_use]
    pub const fn observation_state_sequence(&self) -> ObservationStateSequence {
        self.observation_state_sequence
    }

    /// 实际返回行数；full 等于 selection 行数，delta 等于变化行数。
    #[must_use]
    pub const fn entry_count(&self) -> u64 {
        self.entry_count
    }

    /// 当前实现中批次结构加已初始化行的精确字节数；不是 wire ABI。
    #[must_use]
    pub const fn logical_bytes(&self) -> u64 {
        self.logical_bytes
    }

    /// 当前实现中批次结构加行缓冲实际容量的精确字节数；不是 wire ABI。
    #[must_use]
    pub const fn retained_bytes(&self) -> u64 {
        self.retained_bytes
    }

    /// 按 LaneEdge 稳定标识升序的 full/changed rows。
    #[must_use]
    pub fn rows(&self) -> &[CommittedTrafficObservationRow] {
        &self.rows
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SelectedLaneEdge {
    stable_id: LaneEdgeId,
    ordinal: LaneEdgeOrdinal,
}

/// 调用方持有的观测导出 session。字段私有且无公共构造器，不能伪造或改绑。
#[derive(Debug)]
pub struct ObservationExportSession {
    stream: ObservationStreamBinding,
    network_revision: NetworkRevisionId,
    network_revision_derivation_version: u16,
    selection_digest: Sha256Digest,
    selected: Vec<SelectedLaneEdge>,
    ordinal_to_selected: Vec<usize>,
    previous: Option<ObservationBatchBase>,
    baseline_rows: Vec<CommittedTrafficObservationRow>,
}

impl ObservationExportSession {
    /// Runtime 签发的 stream 绑定。
    #[must_use]
    pub const fn stream_binding(&self) -> ObservationStreamBinding {
        self.stream
    }

    /// 不可变 selection digest。
    #[must_use]
    pub const fn selection_digest(&self) -> Sha256Digest {
        self.selection_digest
    }

    /// 上一次成功批次位置；尚未成功导出时为 `None`。
    #[must_use]
    pub const fn previous(&self) -> Option<ObservationBatchBase> {
        self.previous
    }

    /// 当前实现中 session 结构加已初始化 selection/map/baseline 元素的精确字节数。
    pub fn logical_bytes(&self) -> Result<u64, ObservationError> {
        observation_session_bytes(
            self.selected.len(),
            self.ordinal_to_selected.len(),
            self.baseline_rows.len(),
        )
    }

    /// 当前实现中 session 结构加三组缓冲实际容量的精确字节数。
    pub fn retained_bytes(&self) -> Result<u64, ObservationError> {
        observation_session_bytes(
            self.selected.capacity(),
            self.ordinal_to_selected.capacity(),
            self.baseline_rows.capacity(),
        )
    }
}

/// 观测 session 打开或导出失败；任一失败都不推进 session。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum ObservationError {
    /// 显式选择不能为空。
    #[error("显式观测选择不能为空")]
    EmptySelection,
    /// 显式选择超过当前修订物理 LaneEdge 数。
    #[error("显式观测选择超过当前修订 LaneEdge 数")]
    SelectionTooLarge,
    /// 显式选择必须按稳定标识严格升序且无重复。
    #[error("显式观测选择必须严格升序且无重复")]
    SelectionNotStrictlySorted,
    /// 稳定标识不能解析为当前修订的 LaneEdge。
    #[error("显式观测选择含当前修订未知 LaneEdge: {stable_id}")]
    UnknownLaneEdge {
        /// 无法解析的有类型稳定标识。
        stable_id: LaneEdgeId,
    },
    /// session 属于其它世界或已失效世界世代。
    #[error("观测 session 的世界身份或活动世代已失效")]
    StreamBindingMismatch,
    /// session 绑定的路网修订与当前根不一致。
    #[error("观测 session 的路网修订绑定已失效")]
    NetworkRevisionMismatch,
    /// 新 session 第一次导出必须是 full。
    #[error("新观测 session 第一次导出必须是 full")]
    FirstExportMustBeFull,
    /// delivery sequence 无法继续递增。
    #[error("观测 delivery sequence 已耗尽")]
    DeliverySequenceExhausted,
    /// 当前 tick/state sequence 早于 session 上一次成功批次。
    #[error("当前观测状态早于 session 基线")]
    StatePrecedesBaseline,
    /// 行计数、速度和、占用并集长度或资源字节数 checked 算术溢出。
    #[error("观测整数聚合或资源计数溢出")]
    ArithmeticOverflow,
    /// Active 车辆路线/占用区间无法从已提交状态完整重建。
    #[error("观测占用区间遍历失败")]
    OccupancyIntervalIncomplete,
    /// session、scratch 或输出缓冲无法预留。
    #[error("观测导出缓冲分配失败")]
    AllocationFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ObservationInterval {
    row_index: usize,
    lo_mm: u32,
    hi_mm: u32,
}

impl TrafficWorld {
    /// 当前观测 stream 的已提交状态序号。
    #[must_use]
    pub const fn observation_state_sequence(&self) -> ObservationStateSequence {
        self.observation_state_sequence
    }

    /// 打开调用方持有的观测导出 session；失败不留下 Runtime 隐式状态。
    pub fn open_observation_export(
        &self,
        selection: ObservationSelection,
    ) -> Result<ObservationExportSession, ObservationError> {
        let identity = self.revision.identity();
        let lane_edge_count = self.revision.traffic().lane_edge_count();
        let lane_edge_capacity =
            usize::try_from(lane_edge_count).expect("format-bounded LaneEdge count fits usize");
        let mut selected = Vec::new();

        match selection {
            ObservationSelection::AllLaneEdges => {
                try_reserve_observation_exact(&mut selected, lane_edge_capacity)?;
                for raw in 0..lane_edge_count {
                    let ordinal = LaneEdgeOrdinal::from_raw(raw);
                    let stable_id = identity
                        .stable_id(ordinal)
                        .expect("every LaneEdge ordinal has a stable identity");
                    selected.push(SelectedLaneEdge { stable_id, ordinal });
                }
                selected.sort_unstable_by_key(|edge| edge.stable_id);
            }
            ObservationSelection::ExplicitLaneEdges(stable_ids) => {
                if stable_ids.is_empty() {
                    return Err(ObservationError::EmptySelection);
                }
                if stable_ids.len() > lane_edge_capacity {
                    return Err(ObservationError::SelectionTooLarge);
                }
                if stable_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
                    return Err(ObservationError::SelectionNotStrictlySorted);
                }
                try_reserve_observation_exact(&mut selected, stable_ids.len())?;
                for stable_id in stable_ids.iter().copied() {
                    let ordinal = identity
                        .ordinal(stable_id)
                        .ok_or(ObservationError::UnknownLaneEdge { stable_id })?;
                    selected.push(SelectedLaneEdge { stable_id, ordinal });
                }
            }
        }

        let mut ordinal_to_selected = Vec::new();
        try_reserve_observation_exact(&mut ordinal_to_selected, lane_edge_capacity)?;
        ordinal_to_selected.resize(lane_edge_capacity, NOT_SELECTED);
        for (index, edge) in selected.iter().enumerate() {
            ordinal_to_selected[edge.ordinal.index()] = index;
        }
        let selection_digest = selection_digest(&selected)?;
        let origin = *self.revision.canonical_origin();

        Ok(ObservationExportSession {
            stream: ObservationStreamBinding::new(self.world_id, self.world_generation),
            network_revision: origin.network_revision(),
            network_revision_derivation_version: origin
                .static_contract_versions()
                .network_revision_derivation_version(),
            selection_digest,
            selected,
            ordinal_to_selected,
            previous: None,
            baseline_rows: Vec::new(),
        })
    }

    /// 从一个精确已提交边界导出 full 或 delta；失败不推进 session。
    pub fn export_observation(
        &self,
        session: &mut ObservationExportSession,
        mode: ObservationExportMode,
    ) -> Result<CommittedTrafficObservationBatch, ObservationError> {
        let current_stream = ObservationStreamBinding::new(self.world_id, self.world_generation);
        if session.stream != current_stream {
            return Err(ObservationError::StreamBindingMismatch);
        }
        let origin = *self.revision.canonical_origin();
        if session.network_revision != origin.network_revision()
            || session.network_revision_derivation_version
                != origin
                    .static_contract_versions()
                    .network_revision_derivation_version()
        {
            return Err(ObservationError::NetworkRevisionMismatch);
        }
        if session.previous.is_none() && mode != ObservationExportMode::Full {
            return Err(ObservationError::FirstExportMustBeFull);
        }

        let sequence = match session.previous {
            None => 0,
            Some(previous) => previous
                .sequence
                .checked_add(1)
                .ok_or(ObservationError::DeliverySequenceExhausted)?,
        };
        if let Some(previous) = session.previous {
            let state_precedes = self.tick_index < previous.tick
                || self.observation_state_sequence.get()
                    < previous.observation_state_sequence.get();
            if state_precedes {
                return Err(ObservationError::StatePrecedesBaseline);
            }
        }

        let current_rows = self.collect_observation_rows(session)?;
        let output_count = match mode {
            ObservationExportMode::Full => current_rows.len(),
            ObservationExportMode::Delta => current_rows
                .iter()
                .zip(&session.baseline_rows)
                .filter(|(current, previous)| current != previous)
                .count(),
        };
        let mut rows = Vec::new();
        try_reserve_observation_exact(&mut rows, output_count)?;
        match mode {
            ObservationExportMode::Full => rows.extend_from_slice(&current_rows),
            ObservationExportMode::Delta => {
                rows.extend(
                    current_rows.iter().zip(&session.baseline_rows).filter_map(
                        |(current, previous)| (current != previous).then_some(*current),
                    ),
                );
            }
        }

        let entry_count =
            u64::try_from(rows.len()).map_err(|_| ObservationError::ArithmeticOverflow)?;
        let logical_bytes = observation_batch_bytes(rows.len())?;
        let retained_bytes = observation_batch_bytes(rows.capacity())?;
        let base = match mode {
            ObservationExportMode::Full => None,
            ObservationExportMode::Delta => {
                Some(session.previous.expect("delta requires previous full"))
            }
        };
        let committed = ObservationBatchBase {
            sequence,
            tick: self.tick_index,
            observation_state_sequence: self.observation_state_sequence,
        };

        // 所有可失败工作完成后才推进调用方 session。
        session.previous = Some(committed);
        session.baseline_rows = current_rows;

        Ok(CommittedTrafficObservationBatch {
            binding_version: OBSERVATION_BINDING_VERSION,
            stream: current_stream,
            network_revision: origin.network_revision(),
            network_revision_derivation_version: origin
                .static_contract_versions()
                .network_revision_derivation_version(),
            selection_digest: session.selection_digest,
            mode,
            base,
            sequence,
            tick: self.tick_index,
            observation_state_sequence: self.observation_state_sequence,
            entry_count,
            logical_bytes,
            retained_bytes,
            rows,
        })
    }

    fn collect_observation_rows(
        &self,
        session: &ObservationExportSession,
    ) -> Result<Vec<CommittedTrafficObservationRow>, ObservationError> {
        let mut rows = Vec::new();
        try_reserve_observation_exact(&mut rows, session.selected.len())?;
        rows.extend(
            session
                .selected
                .iter()
                .map(|edge| CommittedTrafficObservationRow::zero(edge.stable_id)),
        );

        let lengths = self.revision.traffic().lane_lengths_millimetres();
        let mut interval_count = 0_usize;
        for handle in self.live_order.iter().copied() {
            let state = self
                .vehicle_state(handle)
                .ok_or(ObservationError::OccupancyIntervalIncomplete)?;
            if state.status != VehicleStatus::Active {
                continue;
            }
            let compiled = self
                .compiled_route(state.route)
                .ok_or(ObservationError::OccupancyIntervalIncomplete)?;
            let route_index = usize::try_from(state.route_edge_index)
                .map_err(|_| ObservationError::OccupancyIntervalIncomplete)?;
            for_each_occupancy_interval(
                lengths,
                compiled.edges.as_slice(),
                route_index,
                state.progress_mm,
                state.length_mm,
                |edge, _, _| {
                    if session.ordinal_to_selected[edge.index()] != NOT_SELECTED {
                        interval_count = interval_count.saturating_add(1);
                    }
                },
            )
            .ok_or(ObservationError::OccupancyIntervalIncomplete)?;
            if interval_count == usize::MAX {
                return Err(ObservationError::ArithmeticOverflow);
            }
        }

        let mut intervals = Vec::new();
        try_reserve_observation_exact(&mut intervals, interval_count)?;
        for handle in self.live_order.iter().copied() {
            let state = self
                .vehicle_state(handle)
                .ok_or(ObservationError::OccupancyIntervalIncomplete)?;
            if state.status != VehicleStatus::Active {
                continue;
            }
            let compiled = self
                .compiled_route(state.route)
                .ok_or(ObservationError::OccupancyIntervalIncomplete)?;
            let route_index = usize::try_from(state.route_edge_index)
                .map_err(|_| ObservationError::OccupancyIntervalIncomplete)?;
            let front_edge = *compiled
                .edges
                .get(route_index)
                .ok_or(ObservationError::OccupancyIntervalIncomplete)?;
            let front_row = session.ordinal_to_selected[front_edge.index()];
            if front_row != NOT_SELECTED {
                rows[front_row].front_vehicle_count = rows[front_row]
                    .front_vehicle_count
                    .checked_add(1)
                    .ok_or(ObservationError::ArithmeticOverflow)?;
                rows[front_row].front_speed_sum_mm_per_second = rows[front_row]
                    .front_speed_sum_mm_per_second
                    .checked_add(u64::from(state.speed_mm_s))
                    .ok_or(ObservationError::ArithmeticOverflow)?;
            }
            for_each_occupancy_interval(
                lengths,
                compiled.edges.as_slice(),
                route_index,
                state.progress_mm,
                state.length_mm,
                |edge, lo_mm, hi_mm| {
                    let row_index = session.ordinal_to_selected[edge.index()];
                    if row_index != NOT_SELECTED {
                        intervals.push(ObservationInterval {
                            row_index,
                            lo_mm,
                            hi_mm,
                        });
                    }
                },
            )
            .ok_or(ObservationError::OccupancyIntervalIncomplete)?;
        }
        debug_assert_eq!(intervals.len(), interval_count);

        intervals
            .sort_unstable_by_key(|interval| (interval.row_index, interval.lo_mm, interval.hi_mm));
        let mut cursor = 0;
        while cursor < intervals.len() {
            let row_index = intervals[cursor].row_index;
            let mut lo_mm = intervals[cursor].lo_mm;
            let mut hi_mm = intervals[cursor].hi_mm;
            cursor += 1;
            while cursor < intervals.len() && intervals[cursor].row_index == row_index {
                let next = intervals[cursor];
                if next.lo_mm <= hi_mm {
                    hi_mm = hi_mm.max(next.hi_mm);
                } else {
                    rows[row_index].occupied_length_mm = rows[row_index]
                        .occupied_length_mm
                        .checked_add(u64::from(hi_mm - lo_mm))
                        .ok_or(ObservationError::ArithmeticOverflow)?;
                    lo_mm = next.lo_mm;
                    hi_mm = next.hi_mm;
                }
                cursor += 1;
            }
            rows[row_index].occupied_length_mm = rows[row_index]
                .occupied_length_mm
                .checked_add(u64::from(hi_mm - lo_mm))
                .ok_or(ObservationError::ArithmeticOverflow)?;
        }

        Ok(rows)
    }
}

fn selection_digest(selected: &[SelectedLaneEdge]) -> Result<Sha256Digest, ObservationError> {
    let count = u64::try_from(selected.len()).map_err(|_| ObservationError::ArithmeticOverflow)?;
    let mut hasher = Sha256::new();
    hasher.update(OBSERVATION_SELECTION_DIGEST_DOMAIN);
    hasher.update(count.to_le_bytes());
    for edge in selected {
        hasher.update(edge.stable_id.as_untyped().as_bytes());
    }
    Ok(Sha256Digest::from_bytes(hasher.finalize().into()))
}

fn observation_batch_bytes(row_count: usize) -> Result<u64, ObservationError> {
    let row_bytes = row_count
        .checked_mul(size_of::<CommittedTrafficObservationRow>())
        .and_then(|bytes| bytes.checked_add(size_of::<CommittedTrafficObservationBatch>()))
        .ok_or(ObservationError::ArithmeticOverflow)?;
    u64::try_from(row_bytes).map_err(|_| ObservationError::ArithmeticOverflow)
}

fn observation_session_bytes(
    selected_count: usize,
    ordinal_map_count: usize,
    baseline_count: usize,
) -> Result<u64, ObservationError> {
    let selected_bytes = selected_count
        .checked_mul(size_of::<SelectedLaneEdge>())
        .ok_or(ObservationError::ArithmeticOverflow)?;
    let ordinal_map_bytes = ordinal_map_count
        .checked_mul(size_of::<usize>())
        .ok_or(ObservationError::ArithmeticOverflow)?;
    let baseline_bytes = baseline_count
        .checked_mul(size_of::<CommittedTrafficObservationRow>())
        .ok_or(ObservationError::ArithmeticOverflow)?;
    let total = size_of::<ObservationExportSession>()
        .checked_add(selected_bytes)
        .and_then(|bytes| bytes.checked_add(ordinal_map_bytes))
        .and_then(|bytes| bytes.checked_add(baseline_bytes))
        .ok_or(ObservationError::ArithmeticOverflow)?;
    u64::try_from(total).map_err(|_| ObservationError::ArithmeticOverflow)
}

#[cfg(test)]
thread_local! {
    static OBSERVATION_RESERVATIONS_BEFORE_FAILURE: Cell<Option<usize>> = const { Cell::new(None) };
}

#[cfg(test)]
struct ObservationAllocationFailpointReset(Option<usize>);

#[cfg(test)]
impl Drop for ObservationAllocationFailpointReset {
    fn drop(&mut self) {
        OBSERVATION_RESERVATIONS_BEFORE_FAILURE.with(|remaining| remaining.set(self.0));
    }
}

/// 只供同线程单元测试确定性覆盖第 N 次观测预留失败。
#[cfg(test)]
fn with_observation_allocation_failure_after<T>(
    successful_reservations: usize,
    run: impl FnOnce() -> T,
) -> T {
    OBSERVATION_RESERVATIONS_BEFORE_FAILURE.with(|remaining| {
        let _reset =
            ObservationAllocationFailpointReset(remaining.replace(Some(successful_reservations)));
        run()
    })
}

fn try_reserve_observation_exact<T>(
    values: &mut Vec<T>,
    capacity: usize,
) -> Result<(), ObservationError> {
    if capacity == 0 {
        return Ok(());
    }
    #[cfg(test)]
    {
        let fail =
            OBSERVATION_RESERVATIONS_BEFORE_FAILURE.with(|remaining| match remaining.get() {
                Some(0) => true,
                Some(value) => {
                    remaining.set(Some(value - 1));
                    false
                }
                None => false,
            });
        if fail {
            return Err(ObservationError::AllocationFailed);
        }
    }
    values
        .try_reserve_exact(capacity)
        .map_err(|_| ObservationError::AllocationFailed)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_static_contract::{
        LaneEdgeId, LaneEdgeOrdinal, ParkingSpaceOrdinal, StableId, StableId128,
        VehicleProfileOrdinal,
    };
    use laneflow_static_network::{
        CanonicalNetworkOrigin, SharedNetworkBuildLimits, SharedNetworkBuildOptions,
        SharedNetworkRevision, SpatialBuildOption, build_shared_network_revision,
    };

    use super::*;
    use crate::tables::with_route_allocation_failure_after;
    use crate::{
        CommittedNetworkSource, CutoverError, CutoverPreflightLimits, LeaveParkingTarget,
        LfcaOriginBinding, MigrationPolicyKind, NetworkRevisionCutoverDescriptor,
        ParkedVehicleSpawnInput, ParkingError, ParkingTarget, PublishedLfcaReference,
        ReserveParkingTarget, RouteRegisterInput, SpawnError, StepError, TickInput,
        VehicleSpawnInput, WorldConfig,
    };

    fn revision(retain_spatial: bool) -> Arc<SharedNetworkRevision> {
        let input = check_canonical_network_input(
            include_bytes!(
                "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
            ),
            FormatLimits::HARD,
        )
        .expect("checked LFCA");
        build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                if retain_spatial {
                    SpatialBuildOption::RetainAvailable
                } else {
                    SpatialBuildOption::Omit
                },
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("shared revision")
    }

    fn source_for(origin: CanonicalNetworkOrigin, key: &str) -> CommittedNetworkSource {
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                key,
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("non-empty source key"),
        }
    }

    fn world_and_route() -> (TrafficWorld, crate::RouteHandle) {
        let revision = revision(true);
        let origin = *revision.canonical_origin();
        let mut world = TrafficWorld::install(
            Arc::clone(&revision),
            WorldConfig::new(16, 8, 1_024, 1_024, 1, 100),
            source_for(origin, "fixture://observation"),
            41,
        )
        .expect("install");
        let edge_for_length = |length: u32| {
            let index = world
                .traffic()
                .lane_lengths_millimetres()
                .iter()
                .position(|actual| *actual == length)
                .expect("fixture LaneEdge length");
            LaneEdgeOrdinal::try_from_usize(index).expect("fixture LaneEdge ordinal")
        };
        let route = world
            .register_route(RouteRegisterInput::new(vec![
                edge_for_length(10_000),
                edge_for_length(8_000),
                edge_for_length(12_000),
            ]))
            .expect("register route");
        (world, route)
    }

    fn spawn(
        world: &mut TrafficWorld,
        route: crate::RouteHandle,
        route_edge_index: u32,
        progress_mm: u32,
        speed_mm_s: u32,
    ) -> crate::VehicleHandle {
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                route_edge_index,
                progress_mm,
                speed_mm_s,
            ))
            .expect("spawn")
    }

    fn lane_id(world: &TrafficWorld, ordinal: LaneEdgeOrdinal) -> LaneEdgeId {
        world
            .revision()
            .identity()
            .stable_id(ordinal)
            .expect("LaneEdge stable id")
    }

    fn all_lane_ids(world: &TrafficWorld) -> Vec<LaneEdgeId> {
        let mut ids: Vec<_> = (0..world.traffic().lane_edge_count())
            .map(|raw| lane_id(world, LaneEdgeOrdinal::from_raw(raw)))
            .collect();
        ids.sort_unstable();
        ids
    }

    fn row_for(
        batch: &CommittedTrafficObservationBatch,
        stable_id: LaneEdgeId,
    ) -> CommittedTrafficObservationRow {
        *batch
            .rows()
            .iter()
            .find(|row| row.lane_edge_stable_id() == stable_id)
            .expect("selected row")
    }

    #[test]
    fn full_exports_all_edges_in_stable_order_and_is_read_only() {
        let (mut world, route) = world_and_route();
        let vehicle = spawn(&mut world, route, 0, 1_000, 0);
        assert_eq!(world.observation_state_sequence().get(), 1);
        let before_state = *world.vehicle_state(vehicle).expect("vehicle");
        let before_tick = world.tick_index();

        let mut session = world
            .open_observation_export(ObservationSelection::AllLaneEdges)
            .expect("open");
        assert!(
            session.retained_bytes().expect("retained")
                >= session.logical_bytes().expect("logical")
        );
        let batch = world
            .export_observation(&mut session, ObservationExportMode::Full)
            .expect("full");

        assert_eq!(batch.binding_version(), OBSERVATION_BINDING_VERSION);
        assert_eq!(batch.stream_binding().world_id(), 41);
        assert_eq!(
            batch.stream_binding().world_generation(),
            WorldGeneration::INITIAL
        );
        assert_eq!(batch.sequence(), 0);
        assert_eq!(batch.base(), None);
        assert_eq!(batch.tick(), 0);
        assert_eq!(batch.observation_state_sequence().get(), 1);
        assert_eq!(
            batch.entry_count(),
            u64::from(world.traffic().lane_edge_count())
        );
        assert_eq!(
            batch.logical_bytes(),
            u64::try_from(
                size_of::<CommittedTrafficObservationBatch>()
                    + core::mem::size_of_val(batch.rows())
            )
            .expect("batch bytes fit u64")
        );
        assert!(batch.retained_bytes() >= batch.logical_bytes());
        assert!(
            batch
                .rows()
                .windows(2)
                .all(|pair| pair[0].lane_edge_stable_id() < pair[1].lane_edge_stable_id())
        );

        let first = world.route_edges(route).expect("route")[0];
        let first_row = row_for(&batch, lane_id(&world, first));
        assert_eq!(first_row.front_vehicle_count(), 1);
        assert_eq!(first_row.occupied_length_mm(), 1_000);
        assert_eq!(first_row.front_speed_sum_mm_per_second(), 0);
        assert_eq!(world.tick_index(), before_tick);
        assert_eq!(world.observation_state_sequence().get(), 1);
        assert_eq!(world.vehicle_state(vehicle), Some(&before_state));
        assert!(
            session.retained_bytes().expect("retained")
                >= session.logical_bytes().expect("logical")
        );
    }

    #[test]
    fn delta_chain_handles_zero_rows_and_same_tick_lifecycle_change() {
        let (mut world, route) = world_and_route();
        let space = ParkingSpaceOrdinal::from_raw(0);
        let (entry_edge, entry_progress_mm) = world
            .traffic()
            .relations()
            .parking_space(space)
            .expect("space")
            .entry();
        let entry_occurrence = world
            .route_edges(route)
            .expect("route")
            .iter()
            .position(|edge| *edge == entry_edge)
            .and_then(|index| u32::try_from(index).ok())
            .expect("parking entry on route");
        let vehicle = spawn(&mut world, route, entry_occurrence, entry_progress_mm, 0);
        assert_eq!(world.active_order, [vehicle]);
        let target = ParkingTarget::ExplicitSpace(space);
        world
            .reserve_parking(
                vehicle,
                ReserveParkingTarget::ExplicitSpace {
                    space,
                    entry_route_occurrence: entry_occurrence,
                },
            )
            .expect("reserve");
        let first_id = lane_id(&world, entry_edge);
        let mut session = world
            .open_observation_export(ObservationSelection::ExplicitLaneEdges(
                vec![first_id].into_boxed_slice(),
            ))
            .expect("open");
        let full = world
            .export_observation(&mut session, ObservationExportMode::Full)
            .expect("full");
        let empty_delta = world
            .export_observation(&mut session, ObservationExportMode::Delta)
            .expect("empty delta");
        assert_eq!(empty_delta.sequence(), 1);
        assert_eq!(empty_delta.entry_count(), 0);
        assert_eq!(empty_delta.rows(), &[]);
        assert_eq!(
            empty_delta.base(),
            Some(ObservationBatchBase {
                sequence: full.sequence(),
                tick: full.tick(),
                observation_state_sequence: full.observation_state_sequence(),
            })
        );

        let sequence_before_park = world.observation_state_sequence();
        world.park_vehicle(vehicle, target).expect("park");
        assert!(world.active_order.is_empty());
        assert_eq!(
            world.observation_state_sequence().get(),
            sequence_before_park.get() + 1
        );
        let cleared = world
            .export_observation(&mut session, ObservationExportMode::Delta)
            .expect("clear delta");
        assert_eq!(cleared.sequence(), 2);
        assert_eq!(cleared.tick(), 0);
        assert_eq!(cleared.entry_count(), 1);
        assert_eq!(cleared.rows()[0].lane_edge_stable_id(), first_id);
        assert_eq!(cleared.rows()[0].front_vehicle_count(), 0);
        assert_eq!(cleared.rows()[0].occupied_length_mm(), 0);
        assert_eq!(cleared.rows()[0].front_speed_sum_mm_per_second(), 0);

        // 同车位幂等成功不改变 v1 观测行，因此不推进状态序号。
        let before_idempotent = world.observation_state_sequence();
        world
            .park_vehicle(vehicle, target)
            .expect("idempotent park");
        assert_eq!(world.observation_state_sequence(), before_idempotent);
        world.step(TickInput::new(100)).expect("parked-only step");
        assert!(world.active_order.is_empty());
        assert_eq!(world.occupancy_inspections(), 0);
    }

    #[test]
    fn cross_edge_body_is_split_but_front_is_owned_by_current_occurrence() {
        let (mut world, route) = world_and_route();
        let edges = world.route_edges(route).expect("route").to_vec();
        let vehicle = spawn(&mut world, route, 1, 1_000, 700);
        let state = *world.vehicle_state(vehicle).expect("vehicle");
        let mut session = world
            .open_observation_export(ObservationSelection::AllLaneEdges)
            .expect("open");
        let full = world
            .export_observation(&mut session, ObservationExportMode::Full)
            .expect("full");
        let previous = row_for(&full, lane_id(&world, edges[0]));
        let current = row_for(&full, lane_id(&world, edges[1]));
        let previous_length = world.traffic().lane_lengths_millimetres()[edges[0].index()];

        assert_eq!(previous.front_vehicle_count(), 0);
        assert_eq!(previous.front_speed_sum_mm_per_second(), 0);
        assert_eq!(
            previous.occupied_length_mm(),
            u64::from(state.length_mm().saturating_sub(1_000).min(previous_length))
        );
        assert_eq!(current.front_vehicle_count(), 1);
        assert_eq!(current.front_speed_sum_mm_per_second(), 700);
        assert_eq!(current.occupied_length_mm(), 1_000);
    }

    #[test]
    fn explicit_selection_is_closed_and_digest_depends_on_resolved_set() {
        let (world, _) = world_and_route();
        let all_ids = all_lane_ids(&world);
        assert_eq!(
            world
                .open_observation_export(ObservationSelection::ExplicitLaneEdges(
                    Vec::new().into_boxed_slice()
                ))
                .unwrap_err(),
            ObservationError::EmptySelection
        );
        assert_eq!(
            world
                .open_observation_export(ObservationSelection::ExplicitLaneEdges(
                    vec![all_ids[0], all_ids[0]].into_boxed_slice()
                ))
                .unwrap_err(),
            ObservationError::SelectionNotStrictlySorted
        );
        if all_ids.len() > 1 {
            assert_eq!(
                world
                    .open_observation_export(ObservationSelection::ExplicitLaneEdges(
                        vec![all_ids[1], all_ids[0]].into_boxed_slice()
                    ))
                    .unwrap_err(),
                ObservationError::SelectionNotStrictlySorted
            );
        }
        let too_many = vec![all_ids[0]; all_ids.len() + 1];
        assert_eq!(
            world
                .open_observation_export(ObservationSelection::ExplicitLaneEdges(
                    too_many.into_boxed_slice()
                ))
                .unwrap_err(),
            ObservationError::SelectionTooLarge
        );
        let unknown = StableId::<laneflow_static_contract::LaneEdgeKind>::from_untyped(
            StableId128::from_bytes([0xff; 16]),
        );
        assert_eq!(
            world
                .open_observation_export(ObservationSelection::ExplicitLaneEdges(
                    vec![unknown].into_boxed_slice()
                ))
                .unwrap_err(),
            ObservationError::UnknownLaneEdge { stable_id: unknown }
        );

        let all = world
            .open_observation_export(ObservationSelection::AllLaneEdges)
            .expect("all");
        let explicit = world
            .open_observation_export(ObservationSelection::ExplicitLaneEdges(
                all_ids.into_boxed_slice(),
            ))
            .expect("explicit all");
        assert_eq!(all.selection_digest(), explicit.selection_digest());
    }

    #[test]
    fn first_delta_and_allocation_failure_do_not_advance_session() {
        let (mut world, route) = world_and_route();
        spawn(&mut world, route, 0, 1_000, 0);
        let mut session = world
            .open_observation_export(ObservationSelection::AllLaneEdges)
            .expect("open");
        assert_eq!(
            world
                .export_observation(&mut session, ObservationExportMode::Delta)
                .unwrap_err(),
            ObservationError::FirstExportMustBeFull
        );
        assert_eq!(session.previous(), None);

        let failed = with_observation_allocation_failure_after(0, || {
            world.export_observation(&mut session, ObservationExportMode::Full)
        });
        assert_eq!(failed.unwrap_err(), ObservationError::AllocationFailed);
        assert_eq!(session.previous(), None);
        let full = world
            .export_observation(&mut session, ObservationExportMode::Full)
            .expect("retry full");
        assert_eq!(full.sequence(), 0);
    }

    #[test]
    fn route_table_changes_do_not_advance_state_but_step_does() {
        let (mut world, route) = world_and_route();
        let before = world.observation_state_sequence();
        let extra = world
            .register_route(RouteRegisterInput::new(
                world.route_edges(route).expect("route").to_vec(),
            ))
            .expect("extra route");
        assert_eq!(world.observation_state_sequence(), before);
        world.remove_route(extra).expect("remove route");
        assert_eq!(world.observation_state_sequence(), before);
        world.step(TickInput::new(100)).expect("step");
        assert_eq!(world.observation_state_sequence().get(), before.get() + 1);
    }

    #[test]
    fn successful_cutover_invalidates_session_and_resets_state_sequence() {
        let (mut world, route) = world_and_route();
        spawn(&mut world, route, 0, 1_000, 0);
        world.step(TickInput::new(100)).expect("step");
        let mut session = world
            .open_observation_export(ObservationSelection::AllLaneEdges)
            .expect("open");
        world
            .export_observation(&mut session, ObservationExportMode::Full)
            .expect("full");

        let target = revision(false);
        let target_origin = *target.canonical_origin();
        let descriptor = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(*world.revision().canonical_origin()),
            LfcaOriginBinding::from_canonical_origin(target_origin),
            None,
            MigrationPolicyKind::SameRevisionRestore,
            world.world_binding(),
        );
        let _ = world
            .cutover_same_revision(
                target,
                source_for(target_origin, "fixture://observation-cutover"),
                &descriptor,
                &CutoverPreflightLimits::new(1_048_576),
            )
            .expect("cutover");
        assert_eq!(
            world.observation_state_sequence(),
            ObservationStateSequence::INITIAL
        );
        assert_eq!(
            world
                .export_observation(&mut session, ObservationExportMode::Delta)
                .unwrap_err(),
            ObservationError::StreamBindingMismatch
        );
        let mut replacement = world
            .open_observation_export(ObservationSelection::AllLaneEdges)
            .expect("replacement session");
        let full = world
            .export_observation(&mut replacement, ObservationExportMode::Full)
            .expect("replacement full");
        assert_eq!(full.sequence(), 0);
        assert_eq!(
            full.observation_state_sequence(),
            ObservationStateSequence::INITIAL
        );
    }

    #[test]
    fn abandoned_cutover_keeps_session_and_state_sequence_live() {
        let (mut world, route) = world_and_route();
        spawn(&mut world, route, 0, 1_000, 0);
        let mut session = world
            .open_observation_export(ObservationSelection::AllLaneEdges)
            .expect("open");
        world
            .export_observation(&mut session, ObservationExportMode::Full)
            .expect("full");
        let generation_before = world.world_generation();
        let state_sequence_before = world.observation_state_sequence();

        let target = revision(false);
        let target_origin = *target.canonical_origin();
        let descriptor = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(*world.revision().canonical_origin()),
            LfcaOriginBinding::from_canonical_origin(target_origin),
            None,
            MigrationPolicyKind::SameRevisionRestore,
            world.world_binding(),
        );
        let result = with_route_allocation_failure_after(0, || {
            world.cutover_same_revision(
                target,
                source_for(target_origin, "fixture://observation-abort"),
                &descriptor,
                &CutoverPreflightLimits::new(1_048_576),
            )
        });
        assert_eq!(result.unwrap_err(), CutoverError::StagingAllocFailed);
        assert_eq!(world.world_generation(), generation_before);
        assert_eq!(world.observation_state_sequence(), state_sequence_before);
        let delta = world
            .export_observation(&mut session, ObservationExportMode::Delta)
            .expect("old session remains live");
        assert_eq!(delta.sequence(), 1);
        assert_eq!(delta.entry_count(), 0);
    }

    #[test]
    fn state_and_delivery_sequence_exhaustion_fail_closed() {
        let (mut world, route) = world_and_route();
        world.observation_state_sequence = ObservationStateSequence::from_raw_for_test(u64::MAX);
        let before_live = world.live_order.len();
        assert_eq!(
            world
                .spawn_vehicle(VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    1_000,
                    0,
                ))
                .unwrap_err(),
            SpawnError::ObservationStateSequenceExhausted
        );
        assert_eq!(world.live_order.len(), before_live);
        let before_tick = world.tick_index();
        assert_eq!(
            world.step(TickInput::new(100)).unwrap_err(),
            StepError::ObservationStateSequenceExhausted
        );
        assert_eq!(world.tick_index(), before_tick);

        world.observation_state_sequence = ObservationStateSequence::INITIAL;
        let space = ParkingSpaceOrdinal::from_raw(0);
        let (entry_edge, entry_progress_mm) = world
            .traffic()
            .relations()
            .parking_space(space)
            .expect("space")
            .entry();
        let entry_occurrence = world
            .route_edges(route)
            .expect("route")
            .iter()
            .position(|edge| *edge == entry_edge)
            .and_then(|index| u32::try_from(index).ok())
            .expect("parking entry on route");
        let vehicle = spawn(&mut world, route, entry_occurrence, entry_progress_mm, 0);
        let mut session = world
            .open_observation_export(ObservationSelection::AllLaneEdges)
            .expect("open");
        world
            .export_observation(&mut session, ObservationExportMode::Full)
            .expect("full");
        session.previous = Some(ObservationBatchBase {
            sequence: u64::MAX,
            tick: world.tick_index(),
            observation_state_sequence: world.observation_state_sequence(),
        });
        let baseline_before = session.baseline_rows.clone();
        assert_eq!(
            world
                .export_observation(&mut session, ObservationExportMode::Delta)
                .unwrap_err(),
            ObservationError::DeliverySequenceExhausted
        );
        assert_eq!(session.baseline_rows, baseline_before);

        let (exit_edge, _) = world
            .traffic()
            .relations()
            .parking_space(space)
            .expect("space")
            .exit();
        let exit_occurrence = world
            .route_edges(route)
            .expect("route")
            .iter()
            .position(|edge| *edge == exit_edge)
            .and_then(|index| u32::try_from(index).ok())
            .expect("parking exit on route");
        let target = ParkingTarget::ExplicitSpace(space);
        let leave = LeaveParkingTarget::ExplicitSpace {
            space,
            route,
            exit_route_occurrence: exit_occurrence,
        };

        world.observation_state_sequence = ObservationStateSequence::from_raw_for_test(u64::MAX);
        let cursor_before_reserve = world.command_cursor();
        let reserve = world
            .reserve_parking(
                vehicle,
                ReserveParkingTarget::ExplicitSpace {
                    space,
                    entry_route_occurrence: entry_occurrence,
                },
            )
            .expect("reservation does not change v1 observation rows");
        assert!(!reserve.is_no_change());
        assert_eq!(world.command_cursor(), cursor_before_reserve + 1);
        assert_eq!(world.observation_state_sequence().get(), u64::MAX);

        let before_state = world.vehicle(vehicle);
        let before_binding = world.parking_binding(vehicle);
        let before_live = world.live_vehicles().to_vec();
        let before_cursor = world.command_cursor();
        assert_eq!(
            world.park_vehicle(vehicle, target).unwrap_err(),
            ParkingError::ObservationStateSequenceExhausted
        );
        assert_eq!(world.vehicle(vehicle), before_state);
        assert_eq!(world.parking_binding(vehicle), before_binding);
        assert_eq!(world.live_vehicles(), before_live);
        assert_eq!(world.command_cursor(), before_cursor);

        world.observation_state_sequence = ObservationStateSequence::INITIAL;
        world
            .park_vehicle(vehicle, target)
            .expect("park changes the active observation row");
        world.observation_state_sequence = ObservationStateSequence::from_raw_for_test(u64::MAX);
        let before_state = world.vehicle(vehicle);
        let before_binding = world.parking_binding(vehicle);
        let before_cursor = world.command_cursor();
        assert_eq!(
            world.leave_parking(vehicle, leave).unwrap_err(),
            ParkingError::ObservationStateSequenceExhausted
        );
        assert_eq!(world.vehicle(vehicle), before_state);
        assert_eq!(world.parking_binding(vehicle), before_binding);
        assert_eq!(world.command_cursor(), before_cursor);

        world.observation_state_sequence = ObservationStateSequence::INITIAL;
        world
            .leave_parking(vehicle, leave)
            .expect("leave restores an active observation row");
        world.observation_state_sequence = ObservationStateSequence::from_raw_for_test(u64::MAX);
        let before_state = world.vehicle(vehicle);
        let before_cursor = world.command_cursor();
        assert_eq!(
            world.despawn_vehicle(vehicle).unwrap_err(),
            ParkingError::ObservationStateSequenceExhausted
        );
        assert_eq!(world.vehicle(vehicle), before_state);
        assert_eq!(world.command_cursor(), before_cursor);

        let vehicle_index = usize::try_from(vehicle.index()).expect("vehicle index");
        world.vehicles[vehicle_index]
            .state
            .as_mut()
            .expect("vehicle remains live")
            .status = VehicleStatus::Completed;
        world.rebuild_active_order();
        let cursor_before_completed_despawn = world.command_cursor();
        let completed = world
            .despawn_vehicle(vehicle)
            .expect("Completed is absent from v1 observation rows");
        assert_eq!(completed.status, VehicleStatus::Completed);
        assert_eq!(world.command_cursor(), cursor_before_completed_despawn + 1);
        assert_eq!(world.observation_state_sequence().get(), u64::MAX);

        let cursor_before_parked_spawn = world.command_cursor();
        let parked = world
            .spawn_parked_vehicle(
                ParkedVehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    entry_occurrence,
                    entry_progress_mm,
                ),
                target,
            )
            .expect("parked spawn is absent from v1 observation rows")
            .vehicle;
        assert_eq!(world.command_cursor(), cursor_before_parked_spawn + 1);
        assert_eq!(world.observation_state_sequence().get(), u64::MAX);
        let cursor_before_parked_despawn = world.command_cursor();
        let parked_record = world
            .despawn_vehicle(parked)
            .expect("Parked is absent from v1 observation rows");
        assert_eq!(parked_record.status, VehicleStatus::Parked);
        assert_eq!(world.command_cursor(), cursor_before_parked_despawn + 1);
        assert_eq!(world.observation_state_sequence().get(), u64::MAX);
    }
}
