//! 与 `LFRS` 容器编码无关的 Runtime 逻辑状态摘要。

use laneflow_static_contract::Sha256Digest;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    CapturedManeuverTraversalPhase, CapturedParkingBinding, CapturedParkingTarget, CapturedRoute,
    CapturedSnapshot, CapturedVehicle, VehicleStatus,
};

/// 确定性状态摘要规范化版本。
pub const RUNTIME_STATE_DIGEST_VERSION: u16 = 7;
/// SHA-256 域分隔前缀；尾随 NUL 属于前缀字节。
pub const RUNTIME_STATE_DIGEST_DOMAIN: &[u8] = b"laneflow:runtime-state-digest:v1\0";

/// 状态摘要规范化失败（#532 摘要轴错误族；直算路径错误边界见 #960）。
///
/// 全部失败模式无副作用：预留 / 限额失败宿主清点后可直接重试；不变量
/// 破坏说明 C(T) 已不可规范化，该比较点按 `Incomplete` 记录，不得跳过
/// 坏记录、补默认值或签发看似有效的摘要（完整性设计 §4.3）。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum SnapshotDigestError {
    /// 规范化缓冲或分组表容量预留失败。
    #[error("状态摘要容量预留失败")]
    ReservationFailed,
    /// 直算摘要的归一化 scratch 超出调用方限额。
    #[error("直算摘要归一化 scratch 超出限额")]
    LimitExceeded,
    /// 已提交状态破坏摘要前置不变量（与资源不足分开报告）。
    #[error("已提交状态破坏摘要前置不变量: {0}")]
    InvariantViolation(StateInvariantViolation),
}

/// 直算摘要前置的已提交状态不变量（完整性设计 §4.3；与资源不足分开）。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum StateInvariantViolation {
    /// 车辆引用的路线槽位不在 live 集内（悬空路线句柄）。
    #[error("车辆引用悬空路线句柄")]
    DanglingRouteHandle,
    /// live 顺序引用的车辆槽位不在 live 集内（悬空 live-order 引用）。
    #[error("live 顺序引用悬空车辆句柄")]
    DanglingLiveOrderEntry,
    /// 已提交 Conflict authority 与 reservation 级证明不一致。
    #[error("已提交 Conflict authority 与证明不一致")]
    ConflictAuthorityMismatch,
    /// 直算过程 unwind，按不变量破坏上报（不套用修订认证停用政策）。
    #[error("直算摘要过程 panic: {0}")]
    Panicked(&'static str),
    /// live 顺序与存活车辆不一一对应（遗漏或重复）。
    #[error("live 顺序与存活车辆不一一对应")]
    IncompleteLiveOrder,
    /// committed 的 live 路线计数与实际存活路线数不一致。
    #[error("live 路线计数与实际存活路线数不一致")]
    LiveRouteCountMismatch,
}

/// 直算摘要的归一化 scratch 调用方限额（#960）。
///
/// 覆盖直算路径的结构性 scratch 估算（排序记录表、句柄映射表、conflict
/// 视图与 canonical 字节缓冲），按「计数 × 元素大小 + 字节数」累计；
/// 超限返回 `SnapshotDigestError::LimitExceeded`，无副作用可重试。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotDigestLimits {
    max_normalization_bytes: u64,
}

impl SnapshotDigestLimits {
    /// 组装限额；`max_normalization_bytes` 为归一化 scratch 累计字节上限。
    #[must_use]
    pub const fn new(max_normalization_bytes: u64) -> Self {
        Self {
            max_normalization_bytes,
        }
    }

    /// 归一化 scratch 累计字节上限。
    #[must_use]
    pub const fn max_normalization_bytes(self) -> u64 {
        self.max_normalization_bytes
    }
}

/// 直算路径的归一化 scratch 预算器（结构性预留与字节累计的单一记账点）。
pub(crate) struct NormalizationBudget {
    remaining: u64,
}

impl NormalizationBudget {
    /// 无限预算（capture 路径专用：预算记账只属直算路径的调用方限额）。
    pub(crate) fn unlimited() -> Self {
        Self {
            remaining: u64::MAX,
        }
    }

    /// 按元素大小与计数预留前入账（capture 助手内部 scratch 用）。
    pub(crate) fn charge_bytes_for_reserve(
        &mut self,
        element_size: usize,
        count: usize,
    ) -> Result<(), SnapshotDigestError> {
        let bytes = (element_size as u64).saturating_mul(u64::try_from(count).unwrap_or(u64::MAX));
        self.charge(bytes)
    }

    fn charge(&mut self, bytes: u64) -> Result<(), SnapshotDigestError> {
        if bytes > self.remaining {
            return Err(SnapshotDigestError::LimitExceeded);
        }
        self.remaining -= bytes;
        Ok(())
    }

    fn charge_elements<T>(&mut self, count: usize) -> Result<(), SnapshotDigestError> {
        let bytes = (core::mem::size_of::<T>() as u64)
            .saturating_mul(u64::try_from(count).unwrap_or(u64::MAX));
        self.charge(bytes)
    }
}

/// 固定前缀的最大字节数（一次预留）。后续计数字段与记录各自走
/// 可失败的摊销增长，不依赖可选前缀留下的剩余容量。
const fn canonical_prefix_len() -> usize {
    RUNTIME_STATE_DIGEST_DOMAIN.len()
        + 2
        + 2 // 摘要版本 / Runtime 状态版本
        + 8 // 世界身份
        + 32 // 语义路网修订 digest
        + 2 * 6 // 六轴静态契约版本
        + 4
        + 4
        + 8
        + 8 // 四个容量字段
        + 8 // fixed dt
        + 8 * 4 // tick / 时间 / 双游标
        + 1 + 16 // selection tag + 可选策略 StableId 的最大长度
}

/// 摘要轴可失败精确预留（注入点与 capture 侧共用同一快照轴计数器）。
fn digest_try_reserve_exact<T>(
    values: &mut Vec<T>,
    additional: usize,
) -> Result<(), SnapshotDigestError> {
    if additional == 0 {
        return Ok(());
    }
    #[cfg(test)]
    if crate::admin::snapshot::snapshot_reservation_injected_failure() {
        return Err(SnapshotDigestError::ReservationFailed);
    }
    values
        .try_reserve_exact(additional)
        .map_err(|_| SnapshotDigestError::ReservationFailed)
}

/// 连续追加字节采用可失败的摊销增长，避免每条记录都精确扩容。
/// 保留与精确预留相同的失败注入边界；实际写入只发生在成功之后。
fn digest_try_reserve(values: &mut Vec<u8>, additional: usize) -> Result<(), SnapshotDigestError> {
    if additional == 0 {
        return Ok(());
    }
    #[cfg(test)]
    if crate::admin::snapshot::snapshot_reservation_injected_failure() {
        return Err(SnapshotDigestError::ReservationFailed);
    }
    values
        .try_reserve(additional)
        .map_err(|_| SnapshotDigestError::ReservationFailed)
}

/// 对捕获的 Runtime 逻辑状态计算与 `LFRS` wire 编码、进程句柄和快照局部 ID 无关的
/// SHA-256 摘要。
///
/// 规范输入包括：摘要版本、世界身份、语义路网修订与六轴静态契约版本、行为语义
/// 配置（容量 + fixed dt）、tick/时间/双游标、路线实例分组多重集（路线内容 +
/// 车辆绑定）与 live 更新顺序（条目绑定所属路线记录）。LFCA exact-byte
/// digest/length、Published asset 审计来源、worker 计划、`WorldGeneration` 与观测
/// session 不进入逻辑摘要。全部缓冲按计数可失败预留（#532）。
///
/// # Errors
///
/// 规范化缓冲或分组表容量预留失败时返回
/// [`SnapshotDigestError::ReservationFailed`]；输入快照不被修改。
pub fn deterministic_state_digest(
    snapshot: &CapturedSnapshot,
) -> Result<Sha256Digest, SnapshotDigestError> {
    // 车辆记录表：`BTreeMap` 无可失败预留 API，等价换「按局部 ID 排序的
    // 扁平表 + 二分查找」（查找语义与输出字节不变）。
    let mut vehicle_records: Vec<(u64, Vec<u8>, u64)> = Vec::new();
    digest_try_reserve_exact(&mut vehicle_records, snapshot.vehicles.len())?;
    for vehicle in &snapshot.vehicles {
        vehicle_records.push((
            vehicle.snapshot_vehicle_id,
            canonical_vehicle_record(vehicle)?,
            vehicle.snapshot_route_id,
        ));
    }
    vehicle_records.sort_unstable_by_key(|entry| entry.0);
    let vehicle_index = |snapshot_vehicle_id: u64| -> usize {
        vehicle_records
            .binary_search_by_key(&snapshot_vehicle_id, |entry| entry.0)
            .expect("captured live_order closes over captured vehicles")
    };

    let mut route_records: Vec<(u64, Vec<u8>)> = Vec::new();
    digest_try_reserve_exact(&mut route_records, snapshot.routes.len())?;
    for route in &snapshot.routes {
        route_records.push((route.snapshot_route_id, canonical_route_record(route)?));
    }
    route_records.sort_unstable_by_key(|entry| entry.0);
    let route_index = |snapshot_route_id: u64| -> usize {
        route_records
            .binary_search_by_key(&snapshot_route_id, |entry| entry.0)
            .expect("captured vehicle route closes over captured routes")
    };

    let mut legacy_budget = NormalizationBudget {
        remaining: u64::MAX,
    };
    let route_groups =
        canonical_route_groups(&vehicle_records, &route_records, &mut legacy_budget)?;

    let mut canonical = Vec::new();
    push_canonical_header(
        &mut canonical,
        snapshot.world_id,
        &snapshot.origin,
        &snapshot.config,
        snapshot.tick,
        snapshot.time_ms,
        snapshot.command_cursor,
        snapshot.event_cursor,
        &snapshot.policy_selection,
    )?;

    let mut waiting_zone_records = Vec::new();
    digest_try_reserve_exact(&mut waiting_zone_records, snapshot.waiting_zones.len())?;
    for state in &snapshot.waiting_zones {
        waiting_zone_records.push(canonical_waiting_zone_record(state)?);
    }
    waiting_zone_records.sort_unstable();
    try_push_u64(
        &mut canonical,
        u64::try_from(waiting_zone_records.len()).expect("WaitingZone count fits u64"),
    )?;
    for record in &waiting_zone_records {
        try_push_record(&mut canonical, record)?;
    }

    let mut conflict_lag_records = Vec::new();
    digest_try_reserve_exact(
        &mut conflict_lag_records,
        snapshot.conflict_lag_states.len(),
    )?;
    for state in &snapshot.conflict_lag_states {
        conflict_lag_records.push(canonical_conflict_lag_record(state)?);
    }
    conflict_lag_records.sort_unstable();
    try_push_u64(
        &mut canonical,
        u64::try_from(conflict_lag_records.len()).expect("Conflict lag count fits u64"),
    )?;
    for record in &conflict_lag_records {
        try_push_record(&mut canonical, record)?;
    }

    try_push_u64(
        &mut canonical,
        u64::try_from(route_groups.len()).expect("route group count fits u64"),
    )?;
    for group in &route_groups {
        try_push_record(&mut canonical, group)?;
    }
    try_push_u64(
        &mut canonical,
        u64::try_from(snapshot.live_order.len()).expect("live order count fits u64"),
    )?;
    // live 序条目 = 车辆记录 + 所属路线记录（内容）。跨路线换序因此可区分
    // （pose 批次按 live 序产出，顺序可观测）；所属路线内容相同的同值车辆
    // 换序保持等价（有序对内容相同）。
    for snapshot_vehicle_id in &snapshot.live_order {
        let index = vehicle_index(*snapshot_vehicle_id);
        let snapshot_route_id = vehicle_records[index].2;
        try_push_record(&mut canonical, &vehicle_records[index].1)?;
        let route_bytes = &route_records[route_index(snapshot_route_id)].1;
        try_push_record(&mut canonical, route_bytes)?;
    }

    let mut hasher = Sha256::new();
    hasher.update(&canonical);
    Ok(Sha256Digest::from_bytes(hasher.finalize().into()))
}

fn canonical_route_groups(
    vehicle_records: &[(u64, Vec<u8>, u64)],
    route_records: &[(u64, Vec<u8>)],
    budget: &mut NormalizationBudget,
) -> Result<Vec<Vec<u8>>, SnapshotDigestError> {
    // 路线实例分组：路线内容 + 绑定其上的车辆记录多重集。内容相同但绑定
    // 不同的实例因此可区分（remove 语义不同）；内容与绑定均相同的实例
    // 可交换（全部后续命令行为一致），保持摘要相等。
    // 一次按路线实例与车辆记录字节排序；不对每条路线重扫整个车辆表。
    let mut members: Vec<(u64, usize)> = Vec::new();
    budget.charge_elements::<(u64, usize)>(vehicle_records.len())?;
    digest_try_reserve_exact(&mut members, vehicle_records.len())?;
    for (index, (_, _, route_id)) in vehicle_records.iter().enumerate() {
        members.push((*route_id, index));
    }
    members.sort_unstable_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| vehicle_records[left.1].1.cmp(&vehicle_records[right.1].1))
    });
    let mut route_groups: Vec<Vec<u8>> = Vec::new();
    budget.charge_elements::<Vec<u8>>(route_records.len())?;
    digest_try_reserve_exact(&mut route_groups, route_records.len())?;
    let mut start = 0;
    for (route_id, route_bytes) in route_records {
        while members
            .get(start)
            .is_some_and(|member| member.0 < *route_id)
        {
            start += 1;
        }
        let mut end = start;
        while members.get(end).is_some_and(|member| member.0 == *route_id) {
            end += 1;
        }
        let bound = &members[start..end];
        // 分组字节在构造前精确入账：路线记录 + 车辆计数 + 逐成员长度前缀记录。
        let mut group_bytes = u64::try_from(route_bytes.len()).unwrap_or(u64::MAX) + 8;
        for (_, index) in bound {
            group_bytes = group_bytes.saturating_add(8u64.saturating_add(
                u64::try_from(vehicle_records[*index].1.len()).unwrap_or(u64::MAX),
            ));
        }
        budget.charge(group_bytes)?;
        let mut group = Vec::new();
        let group_len =
            usize::try_from(group_bytes).map_err(|_| SnapshotDigestError::ReservationFailed)?;
        digest_try_reserve_exact(&mut group, group_len)?;
        group.extend_from_slice(route_bytes);
        push_u64(
            &mut group,
            u64::try_from(bound.len()).expect("vehicle count fits u64"),
        );
        for (_, index) in bound {
            push_record(&mut group, &vehicle_records[*index].1);
        }
        route_groups.push(group);
        start = end;
    }
    route_groups.sort_unstable();

    Ok(route_groups)
}

fn canonical_route_record(route: &CapturedRoute) -> Result<Vec<u8>, SnapshotDigestError> {
    let mut record = Vec::new();
    digest_try_reserve_exact(&mut record, 8 + route.edges.len() * 16)?;
    push_u64(
        &mut record,
        u64::try_from(route.edges.len()).expect("route edge count fits u64"),
    );
    for edge in &route.edges {
        record.extend_from_slice(edge.as_bytes());
    }
    Ok(record)
}

fn canonical_vehicle_record(vehicle: &CapturedVehicle) -> Result<Vec<u8>, SnapshotDigestError> {
    // 路线内容由所属实例分组承载，车辆记录不内嵌（内容相同的路线实例
    // 以其车辆绑定区分，见分组构造）。
    let parking_len = match vehicle.parking {
        None => 1,
        Some(CapturedParkingBinding::Occupied { .. }) => 1 + 1 + 1 + 16 + 1,
        Some(CapturedParkingBinding::Reserved { virtual_entry, .. }) => {
            1 + 1 + 1 + 16 + 4 + 1 + if virtual_entry.is_some() { 16 + 4 } else { 0 }
        }
    };
    let traversal_len = if vehicle.maneuver_traversal.is_some() {
        1 + 4 + 16 + 1 + 16
    } else {
        1
    };
    let waiting_len = if vehicle.waiting_membership.is_some() {
        1 + 16 + 4 + 16 + 16 + 8
    } else {
        1
    };
    let eligibility_len = if vehicle.conflict_eligibility.is_some() {
        1 + 4 + 4 + 16 + 4 + 32 + 8
    } else {
        1
    };
    let reservation_len = vehicle
        .conflict_reservation
        .as_ref()
        .map_or(1, |reservation| {
            1 + 8
                + 4
                + 4
                + 16
                + 8
                + reservation.passages.len() * (4 + 32 + 4 + 4 + 4 + 4)
                + 8
                + reservation.downstream_intervals.len() * (16 + 4 + 4)
        });
    let record_len = 4
        + 4
        + 2
        + 4
        + 1
        + 16
        + 16
        + parking_len
        + traversal_len
        + waiting_len
        + eligibility_len
        + reservation_len;
    let mut record = Vec::new();
    digest_try_reserve_exact(&mut record, record_len)?;
    push_u32(&mut record, vehicle.route_edge_index);
    push_u32(&mut record, vehicle.progress_mm);
    push_u16(&mut record, vehicle.carry_um);
    push_u32(&mut record, vehicle.speed_mm_s);
    record.push(match vehicle.status {
        VehicleStatus::Active => 1,
        VehicleStatus::Parked => 2,
        VehicleStatus::Completed => 3,
    });
    record.extend_from_slice(vehicle.profile.as_bytes());
    record.extend_from_slice(vehicle.class.as_bytes());
    match vehicle.parking {
        None => record.push(0),
        Some(binding) => {
            record.push(1);
            let (state, target, occurrence, virtual_entry) = match binding {
                CapturedParkingBinding::Reserved {
                    target,
                    entry_route_occurrence,
                    virtual_entry,
                } => (1, target, Some(entry_route_occurrence), virtual_entry),
                CapturedParkingBinding::Occupied { target } => (2, target, None, None),
            };
            record.push(state);
            match target {
                CapturedParkingTarget::ExplicitSpace(stable) => {
                    record.push(1);
                    record.extend_from_slice(stable.as_bytes());
                }
                CapturedParkingTarget::VirtualPool(stable) => {
                    record.push(2);
                    record.extend_from_slice(stable.as_bytes());
                }
            }
            if let Some(occurrence) = occurrence {
                push_u32(&mut record, occurrence);
            }
            match virtual_entry {
                None => record.push(0),
                Some(entry) => {
                    record.push(1);
                    record.extend_from_slice(entry.lane_edge.as_bytes());
                    push_u32(&mut record, entry.progress_mm);
                }
            }
        }
    }
    match vehicle.maneuver_traversal {
        None => record.push(0),
        Some(traversal) => {
            record.push(1);
            push_u32(&mut record, traversal.maneuver_occurrence_index);
            record.extend_from_slice(traversal.maneuver_path.as_bytes());
            record.push(match traversal.phase {
                CapturedManeuverTraversalPhase::PreGate => 1,
                CapturedManeuverTraversalPhase::Committed => 2,
                CapturedManeuverTraversalPhase::Waiting => 3,
                CapturedManeuverTraversalPhase::Clearing => 4,
            });
            record.extend_from_slice(traversal.phase_gate.as_bytes());
        }
    }
    match vehicle.waiting_membership {
        None => record.push(0),
        Some(membership) => {
            record.push(1);
            record.extend_from_slice(membership.waiting_zone.as_bytes());
            push_u32(&mut record, membership.maneuver_occurrence_index);
            record.extend_from_slice(membership.entry_gate.as_bytes());
            record.extend_from_slice(membership.release_gate.as_bytes());
            push_u64(&mut record, membership.admission_sequence);
        }
    }
    match vehicle.conflict_eligibility {
        None => record.push(0),
        Some(eligibility) => {
            record.push(1);
            push_u32(&mut record, eligibility.maneuver_occurrence_index);
            push_u32(&mut record, eligibility.maneuver_entry_route_edge_index);
            record.extend_from_slice(eligibility.admission_gate.as_bytes());
            push_u32(&mut record, eligibility.conflict_occurrence_index);
            push_conflict_locator(&mut record, eligibility.passage);
            push_u64(&mut record, eligibility.first_eligible_tick);
        }
    }
    match vehicle.conflict_reservation.as_ref() {
        None => record.push(0),
        Some(reservation) => {
            record.push(1);
            push_u64(&mut record, reservation.acquired_tick);
            push_u32(&mut record, reservation.maneuver_occurrence_index);
            push_u32(&mut record, reservation.maneuver_entry_route_edge_index);
            record.extend_from_slice(reservation.admission_gate.as_bytes());
            push_u64(
                &mut record,
                u64::try_from(reservation.passages.len()).expect("passage count fits u64"),
            );
            for passage in &reservation.passages {
                push_u32(&mut record, passage.conflict_occurrence_index);
                push_conflict_locator(&mut record, passage.passage);
                push_u32(&mut record, passage.entry_route_edge_index);
                push_u32(&mut record, passage.entry_progress_mm);
                push_u32(&mut record, passage.clearance_route_edge_index);
                push_u32(&mut record, passage.clearance_progress_mm);
            }
            push_u64(
                &mut record,
                u64::try_from(reservation.downstream_intervals.len())
                    .expect("downstream count fits u64"),
            );
            for interval in &reservation.downstream_intervals {
                record.extend_from_slice(interval.lane_edge.as_bytes());
                push_u32(&mut record, interval.start_mm);
                push_u32(&mut record, interval.end_mm);
            }
        }
    }
    Ok(record)
}

fn push_conflict_locator(
    target: &mut Vec<u8>,
    locator: crate::admin::snapshot::CapturedConflictPassageLocator,
) {
    target.extend_from_slice(locator.participant_stream.as_bytes());
    target.extend_from_slice(locator.conflict_zone.as_bytes());
}

/// 预留后写入长度前缀记录（预留成功则本次写入不再分配）。
fn try_push_record(target: &mut Vec<u8>, record: &[u8]) -> Result<(), SnapshotDigestError> {
    digest_try_reserve(target, 8 + record.len())?;
    push_record(target, record);
    Ok(())
}

/// 可失败预留后写入 `u64` 计数字段。
fn try_push_u64(target: &mut Vec<u8>, value: u64) -> Result<(), SnapshotDigestError> {
    digest_try_reserve(target, 8)?;
    push_u64(target, value);
    Ok(())
}

fn push_record(target: &mut Vec<u8>, record: &[u8]) {
    push_u64(
        target,
        u64::try_from(record.len()).expect("canonical record length fits u64"),
    );
    target.extend_from_slice(record);
}

fn push_u16(target: &mut Vec<u8>, value: u16) {
    target.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(target: &mut Vec<u8>, value: u32) {
    target.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(target: &mut Vec<u8>, value: u64) {
    target.extend_from_slice(&value.to_le_bytes());
}

/// 单条等待区状态的 canonical 记录（现行捕获路径与直算路径共用）。
fn canonical_waiting_zone_record(
    state: &crate::CapturedWaitingZoneState,
) -> Result<Vec<u8>, SnapshotDigestError> {
    let mut record = Vec::new();
    digest_try_reserve_exact(&mut record, 16 + 4 + 8)?;
    record.extend_from_slice(state.waiting_zone.as_bytes());
    push_u32(&mut record, state.occupancy);
    push_u64(&mut record, state.next_admission_sequence);
    Ok(record)
}

/// 单条冲突滞后状态的 canonical 记录（现行捕获路径与直算路径共用）。
fn canonical_conflict_lag_record(
    state: &crate::admin::snapshot::CapturedConflictLagState,
) -> Result<Vec<u8>, SnapshotDigestError> {
    let mut record = Vec::new();
    digest_try_reserve_exact(&mut record, 32 + 1 + 8)?;
    push_conflict_locator(&mut record, state.passage);
    let (tag, time) = match state.reference {
        crate::ConflictLagReference::NoHistory => (0, 0),
        crate::ConflictLagReference::ActualClear(time) => (1, time),
        crate::ConflictLagReference::CutoverFloor(time) => (2, time),
    };
    record.push(tag);
    push_u64(&mut record, time);
    Ok(record)
}

/// 写入 canonical 固定前缀（现行捕获路径与直算路径共用；逐字节一致）。
#[allow(clippy::too_many_arguments)]
fn push_canonical_header(
    canonical: &mut Vec<u8>,
    world_id: u64,
    origin: &laneflow_static_network::CanonicalNetworkOrigin,
    config: &crate::WorldConfig,
    tick: u64,
    time_ms: u64,
    command_cursor: u64,
    event_cursor: u64,
    policy_selection: &crate::WorldPolicySelection,
) -> Result<(), SnapshotDigestError> {
    digest_try_reserve_exact(canonical, canonical_prefix_len())?;
    digest_try_reserve_exact(canonical, canonical_prefix_len())?;
    canonical.extend_from_slice(RUNTIME_STATE_DIGEST_DOMAIN);
    push_u16(canonical, RUNTIME_STATE_DIGEST_VERSION);
    push_u16(canonical, crate::RUNTIME_STATE_VERSION);
    push_u64(canonical, world_id);
    canonical.extend_from_slice(origin.network_revision().as_digest().as_bytes());
    let contracts = origin.static_contract_versions();
    push_u16(canonical, contracts.canonical_format_version());
    push_u16(canonical, contracts.identity_encoding_version());
    push_u16(canonical, contracts.identity_registry_revision());
    push_u16(canonical, contracts.network_revision_derivation_version());
    push_u16(canonical, contracts.constraint_contract_version());
    push_u16(canonical, contracts.static_execution_contract_version());
    push_u32(canonical, config.vehicle_capacity());
    push_u32(canonical, config.route_capacity());
    push_u64(canonical, config.route_edge_occurrence_capacity());
    push_u64(canonical, config.route_conflict_occurrence_capacity());
    push_u64(canonical, config.fixed_delta_time_ms());
    push_u64(canonical, tick);
    push_u64(canonical, time_ms);
    push_u64(canonical, command_cursor);
    push_u64(canonical, event_cursor);
    match policy_selection {
        crate::WorldPolicySelection::NotRequired => canonical.push(0),
        crate::WorldPolicySelection::Pinned(pin) => {
            canonical.push(1);
            canonical.extend_from_slice(pin.policy.as_untyped().as_bytes());
        }
    }
    Ok(())
}

impl crate::kernel::state::WorldState {
    /// 直算确定性状态摘要：不经 `capture_snapshot` 的整份快照拷贝，从已提交
    /// 状态逐记录抽取并立即归一化（完整性设计 §4.3；#960）。
    ///
    /// 字节与现行 `deterministic_state_digest(&CapturedSnapshot)` 逐字节一致
    ///（digest 7 / runtime state 5 不变）；局部 ID 按 capture 语义由 live 槽位序
    /// 分配。归一化 scratch 由 `limits` 约束；预留 / 限额失败无副作用可重试。
    /// 读取真实触达已提交字节，不给结论保留缓存。
    ///
    /// # Panics
    ///
    /// 不向调用方传播：抽取过程中引用无法解析等 unwind 被入口捕获并按
    /// [`StateInvariantViolation::Panicked`] 上报。
    pub(crate) fn direct_state_digest(
        &self,
        limits: SnapshotDigestLimits,
    ) -> Result<Sha256Digest, SnapshotDigestError> {
        let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.direct_state_digest_inner(limits)
        }));
        match attempt {
            Ok(result) => result,
            Err(payload) => Err(SnapshotDigestError::InvariantViolation(
                StateInvariantViolation::Panicked(
                    payload
                        .downcast_ref::<&str>()
                        .copied()
                        .unwrap_or("opaque direct-digest panic"),
                ),
            )),
        }
    }

    fn direct_state_digest_inner(
        &self,
        limits: SnapshotDigestLimits,
    ) -> Result<Sha256Digest, SnapshotDigestError> {
        use crate::admin::snapshot::FIRST_SNAPSHOT_ID;
        let mut budget = NormalizationBudget {
            remaining: limits.max_normalization_bytes,
        };
        let identity = self.binding.revision.identity();

        // Conflict 视图（与 capture 同一构造路径；重建失败即不变量破坏）。
        let conflict_index_len = self.binding.config.vehicle_capacity() as usize;
        budget.charge_elements::<Option<crate::kernel::conflict::CommittedConflictIndexEntry>>(
            conflict_index_len,
        )?;
        let mut conflict_index = Vec::new();
        digest_try_reserve_exact(&mut conflict_index, conflict_index_len)?;
        conflict_index.resize(conflict_index_len, None);
        let conflict_view = self
            .conflict_read()
            .persistence_view(&mut conflict_index)
            .ok_or(SnapshotDigestError::InvariantViolation(
                StateInvariantViolation::ConflictAuthorityMismatch,
            ))?;

        // 路线：live 槽位序分配局部 ID，单条物化立即归一化后丢弃。容量取
        // 实测存活路线数并核对 committed 计数器——失真计数不得经隐式扩容
        // 绕过预算与预留（完整性设计 §4.3：直算不假设输入结构闭合）。
        let route_capacity = self
            .committed
            .routes
            .iter()
            .filter(|slot| slot.compiled.is_some())
            .count();
        if route_capacity != usize::try_from(self.committed.live_route_count).unwrap_or(usize::MAX)
        {
            return Err(SnapshotDigestError::InvariantViolation(
                StateInvariantViolation::LiveRouteCountMismatch,
            ));
        }
        budget.charge_elements::<(u64, Vec<u8>)>(route_capacity)?;
        budget.charge_elements::<(u32, u32, u64)>(route_capacity)?;
        let mut route_records: Vec<(u64, Vec<u8>)> = Vec::new();
        let mut route_ids: Vec<(u32, u32, u64)> = Vec::new();
        digest_try_reserve_exact(&mut route_records, route_capacity)?;
        digest_try_reserve_exact(&mut route_ids, route_capacity)?;
        for (slot_index, slot) in self.committed.routes.iter().enumerate() {
            if slot.compiled.is_none() {
                continue;
            }
            let handle = crate::RouteHandle::new(
                u32::try_from(slot_index).expect("route index fits u32"),
                slot.generation,
            );
            let snapshot_route_id = FIRST_SNAPSHOT_ID + route_records.len() as u64;
            route_ids.push((handle.generation(), handle.index(), snapshot_route_id));
            let route = self
                .captured_route_at(identity, &mut budget, slot_index, snapshot_route_id)
                .transpose()
                .expect("live route slot yields captured route")
                .map_err(direct_capture_error)?;
            let record = canonical_route_record(&route)?;
            budget.charge(u64::try_from(record.len()).unwrap_or(u64::MAX))?;
            route_records.push((snapshot_route_id, record));
        }
        route_ids.sort_unstable();
        let route_id_for = |generation: u32, index: u32| -> Option<u64> {
            route_ids
                .binary_search_by_key(&(generation, index), |entry| (entry.0, entry.1))
                .ok()
                .map(|position| route_ids[position].2)
        };

        // 车辆：live 槽位序分配局部 ID；悬空路线句柄显式报告不变量破坏。
        // 容量取实测存活车辆数（live 顺序完整性随后单独核对）。
        let vehicle_capacity = self
            .committed
            .vehicles
            .iter()
            .filter(|slot| slot.state.is_some())
            .count();
        budget.charge_elements::<(u64, Vec<u8>, u64)>(vehicle_capacity)?;
        budget.charge_elements::<(u32, u32, u64)>(vehicle_capacity)?;
        let mut vehicle_records: Vec<(u64, Vec<u8>, u64)> = Vec::new();
        let mut vehicle_ids: Vec<(u32, u32, u64)> = Vec::new();
        digest_try_reserve_exact(&mut vehicle_records, vehicle_capacity)?;
        digest_try_reserve_exact(&mut vehicle_ids, vehicle_capacity)?;
        for (slot_index, slot) in self.committed.vehicles.iter().enumerate() {
            if slot.state.is_none() {
                continue;
            }
            let generation = slot.generation;
            let snapshot_vehicle_id = FIRST_SNAPSHOT_ID + vehicle_records.len() as u64;
            vehicle_ids.push((
                generation,
                u32::try_from(slot_index).expect("vehicle index fits u32"),
                snapshot_vehicle_id,
            ));
            let state = slot
                .state
                .as_ref()
                .expect("checked vehicle slot holds state");
            let snapshot_route_id = route_id_for(state.route.generation(), state.route.index())
                .ok_or(SnapshotDigestError::InvariantViolation(
                    StateInvariantViolation::DanglingRouteHandle,
                ))?;
            // 此槽位的路线句柄已解析；捕获助手只查询同一只读车辆的所属路线。
            let route_id_for_capture = |_, _| snapshot_route_id;
            let vehicle = self
                .captured_vehicle_at(
                    identity,
                    &conflict_view,
                    &route_id_for_capture,
                    &mut budget,
                    slot_index,
                    snapshot_vehicle_id,
                )
                .transpose()
                .expect("live vehicle slot yields captured vehicle")
                .map_err(direct_capture_error)?;
            let record = canonical_vehicle_record(&vehicle)?;
            budget.charge(u64::try_from(record.len()).unwrap_or(u64::MAX))?;
            vehicle_records.push((snapshot_vehicle_id, record, snapshot_route_id));
        }
        vehicle_ids.sort_unstable();

        let route_groups = canonical_route_groups(&vehicle_records, &route_records, &mut budget)?;

        // canonical 组装：前缀 + 等待区 + 冲突滞后 + 路线分组 + live 序。
        budget.charge(u64::try_from(canonical_prefix_len()).unwrap_or(u64::MAX))?;
        let mut canonical = Vec::new();
        push_canonical_header(
            &mut canonical,
            self.binding.world_id,
            self.binding.revision.canonical_origin(),
            &self.binding.config,
            self.committed.tick_index,
            self.committed.time_ms,
            self.committed.command_cursor,
            self.committed.event_cursor,
            &self.policy_selection(),
        )?;

        let mut waiting_zone_records = Vec::new();
        let waiting_state_count = self
            .committed
            .waiting_zones
            .iter()
            .filter(|state| state.occupancy != 0 || state.next_admission_sequence != 0)
            .count();
        budget.charge_elements::<Vec<u8>>(waiting_state_count)?;
        digest_try_reserve_exact(&mut waiting_zone_records, waiting_state_count)?;
        for (index, state) in self.committed.waiting_zones.iter().copied().enumerate() {
            if state.occupancy == 0 && state.next_admission_sequence == 0 {
                continue;
            }
            let zone = laneflow_static_contract::WaitingZoneOrdinal::from_raw(
                u32::try_from(index).expect("WaitingZone index fits u32"),
            );
            let captured = crate::CapturedWaitingZoneState {
                waiting_zone: *identity
                    .stable_id(zone)
                    .expect("WaitingZone resolves stable id")
                    .as_untyped(),
                occupancy: state.occupancy,
                next_admission_sequence: state.next_admission_sequence,
            };
            let record = canonical_waiting_zone_record(&captured)?;
            budget.charge(u64::try_from(record.len()).unwrap_or(u64::MAX))?;
            waiting_zone_records.push(record);
        }
        waiting_zone_records.sort_unstable();
        charge_canonical_segment(&mut budget, &waiting_zone_records)?;
        try_push_u64(
            &mut canonical,
            u64::try_from(waiting_zone_records.len()).expect("WaitingZone count fits u64"),
        )?;
        for record in &waiting_zone_records {
            try_push_record(&mut canonical, record)?;
        }

        let conflict_lag_count = self.conflict_read().persisted_lag_rows().count();
        budget.charge_elements::<Vec<u8>>(conflict_lag_count)?;
        let mut conflict_lag_records = Vec::new();
        digest_try_reserve_exact(&mut conflict_lag_records, conflict_lag_count)?;
        for (address, reference) in self.conflict_read().persisted_lag_rows() {
            let locator = self
                .conflict_passage_locator(address)
                .expect("installed conflict address resolves stable locator");
            let captured = crate::admin::snapshot::CapturedConflictLagState {
                passage: crate::admin::snapshot::CapturedConflictPassageLocator {
                    participant_stream: *locator.participant_stream_stable_id().as_untyped(),
                    conflict_zone: *locator.conflict_zone_stable_id().as_untyped(),
                },
                reference,
            };
            let record = canonical_conflict_lag_record(&captured)?;
            budget.charge(u64::try_from(record.len()).unwrap_or(u64::MAX))?;
            conflict_lag_records.push(record);
        }
        conflict_lag_records.sort_unstable();
        charge_canonical_segment(&mut budget, &conflict_lag_records)?;
        try_push_u64(
            &mut canonical,
            u64::try_from(conflict_lag_records.len()).expect("Conflict lag count fits u64"),
        )?;
        for record in &conflict_lag_records {
            try_push_record(&mut canonical, record)?;
        }

        charge_canonical_segment(&mut budget, &route_groups)?;
        try_push_u64(
            &mut canonical,
            u64::try_from(route_groups.len()).expect("route group count fits u64"),
        )?;
        for group in &route_groups {
            try_push_record(&mut canonical, group)?;
        }
        let vehicle_id_for = |generation: u32, index: u32| -> Option<u64> {
            vehicle_ids
                .binary_search_by_key(&(generation, index), |entry| (entry.0, entry.1))
                .ok()
                .map(|position| vehicle_ids[position].2)
        };
        // 局部 ID 由上方逐条 push 生成，记录表不重排；它直接编码记录索引，
        // 与可能稀疏、带世代且顺序不同的 Runtime 句柄无关。
        let record_index = |snapshot_id: u64| -> usize {
            usize::try_from(snapshot_id - FIRST_SNAPSHOT_ID).expect("local record index fits usize")
        };
        // live 顺序必须是存活车辆的完整无重复排列：长度先核（遗漏直接暴露），
        // 写入时以命中标记核对唯一性——只验证句柄可解析不足以拒绝失真状态。
        let live_order = &self.committed.live_order;
        if live_order.len() != vehicle_capacity {
            return Err(SnapshotDigestError::InvariantViolation(
                StateInvariantViolation::IncompleteLiveOrder,
            ));
        }
        budget.charge_elements::<bool>(live_order.len())?;
        let mut seen = Vec::new();
        digest_try_reserve_exact(&mut seen, vehicle_capacity)?;
        seen.resize(vehicle_capacity, false);
        budget.charge(8)?;
        try_push_u64(
            &mut canonical,
            u64::try_from(live_order.len()).expect("live order count fits u64"),
        )?;
        // 校验和编码共用一次句柄查找；每条先入账再写本地缓冲。
        // 中途失配只丢弃本次缓冲，不修改世界，也不签发部分摘要。
        for handle in live_order {
            let snapshot_vehicle_id = vehicle_id_for(handle.generation(), handle.index()).ok_or(
                SnapshotDigestError::InvariantViolation(
                    StateInvariantViolation::DanglingLiveOrderEntry,
                ),
            )?;
            let index = record_index(snapshot_vehicle_id);
            if seen[index] {
                return Err(SnapshotDigestError::InvariantViolation(
                    StateInvariantViolation::IncompleteLiveOrder,
                ));
            }
            seen[index] = true;
            let snapshot_route_id = vehicle_records[index].2;
            let route_bytes = &route_records[record_index(snapshot_route_id)].1;
            let record_bytes = 16_u64
                .saturating_add(u64::try_from(vehicle_records[index].1.len()).unwrap_or(u64::MAX))
                .saturating_add(u64::try_from(route_bytes.len()).unwrap_or(u64::MAX));
            budget.charge(record_bytes)?;
            try_push_record(&mut canonical, &vehicle_records[index].1)?;
            try_push_record(&mut canonical, route_bytes)?;
        }

        let mut hasher = Sha256::new();
        hasher.update(&canonical);
        Ok(Sha256Digest::from_bytes(hasher.finalize().into()))
    }
}

/// canonical 一段追加（计数 u64 + 逐记录长度前缀）在写入前整体入账。
fn charge_canonical_segment(
    budget: &mut NormalizationBudget,
    records: &[Vec<u8>],
) -> Result<(), SnapshotDigestError> {
    let mut bytes = 8_u64;
    for record in records {
        bytes = bytes
            .saturating_add(8)
            .saturating_add(u64::try_from(record.len()).unwrap_or(u64::MAX));
    }
    budget.charge(bytes)
}

/// capture 助手错误到直算错误的映射（Conflict authority 不变量 → 显式变体）。
fn direct_capture_error(
    error: crate::admin::snapshot::SnapshotCaptureError,
) -> SnapshotDigestError {
    match error {
        crate::admin::snapshot::SnapshotCaptureError::ReservationFailed => {
            SnapshotDigestError::ReservationFailed
        }
        crate::admin::snapshot::SnapshotCaptureError::LimitExceeded => {
            SnapshotDigestError::LimitExceeded
        }
        crate::admin::snapshot::SnapshotCaptureError::ConflictInvariantViolation => {
            SnapshotDigestError::InvariantViolation(
                StateInvariantViolation::ConflictAuthorityMismatch,
            )
        }
    }
}

impl crate::TrafficWorld {
    /// 直算确定性状态摘要（完整性设计 §4.3；#960）：直接从已提交状态计算，
    /// 不经 `capture_snapshot` 的整份快照拷贝，字节与现行捕获路径逐字节一致。
    /// 只读已提交状态，不改变世界、不推进游标；归一化 scratch 由 `limits` 约束，
    /// 预留 / 限额失败无副作用、宿主清点后可直接重试。
    ///
    /// # Errors
    ///
    /// 预留失败返回 [`SnapshotDigestError::ReservationFailed`]；scratch 超出
    /// `limits` 返回 [`SnapshotDigestError::LimitExceeded`]；已提交状态破坏
    /// 摘要前置不变量（悬空路线句柄、悬空 live-order 引用、Conflict
    /// authority 与证明不一致、抽取过程 unwind）返回
    /// [`SnapshotDigestError::InvariantViolation`]，该比较点按 `Incomplete` 记录，
    /// 不签发摘要。
    ///
    /// # Panics
    ///
    /// 世界因执行 panic 失效后调用会 panic；宿主必须销毁并重新构建世界。
    /// 摘要过程自身的 unwind 被入口捕获，按不变量破坏上报，不向调用方传播。
    pub fn direct_state_digest(
        &self,
        limits: SnapshotDigestLimits,
    ) -> Result<Sha256Digest, SnapshotDigestError> {
        self.execution.assert_usable();
        self.state.direct_state_digest(limits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CapturedRoute;
    use crate::admin::cutover::tests::transaction_tests::world_with_vehicle;
    use crate::{
        ParkedVehicleSpawnInput, ParkingTarget, SnapshotRestoreLimits, TickInput, WorldConfig,
        encode_lfrs, restore_lfrs,
    };

    #[test]
    fn dynamic_record_appends_amortize_growth_and_preserve_bytes() {
        let mut actual = Vec::new();
        let mut expected = Vec::new();
        let mut growths = 0;
        for index in 0..4_096_u64 {
            let capacity = actual.capacity();
            try_push_u64(&mut actual, index).unwrap();
            try_push_record(&mut actual, &index.to_le_bytes()).unwrap();
            growths += usize::from(actual.capacity() != capacity);
            push_u64(&mut expected, index);
            push_record(&mut expected, &index.to_le_bytes());
        }
        assert_eq!(actual, expected);
        // 只约束摊销增长，不绑定分配器的精确容量或某个墙钟阈值。
        assert!(growths <= 32, "unexpected per-record growth: {growths}");
    }

    #[test]
    fn dynamic_append_failure_preserves_bytes_and_is_retryable() {
        let mut bytes = vec![1, 2, 3];
        let before = bytes.clone();
        let failed = crate::admin::snapshot::with_snapshot_allocation_failure_after(0, || {
            try_push_record(&mut bytes, &[4, 5])
        });
        assert_eq!(failed, Err(SnapshotDigestError::ReservationFailed));
        assert_eq!(bytes, before);
        assert_eq!(
            digest_try_reserve(&mut bytes, usize::MAX),
            Err(SnapshotDigestError::ReservationFailed)
        );
        assert_eq!(bytes, before);
        try_push_record(&mut bytes, &[4, 5]).unwrap();
        let mut expected = before;
        push_record(&mut expected, &[4, 5]);
        assert_eq!(bytes, expected);
    }

    #[test]
    #[ignore = "同机 release 摘要字节负载取证；不是城市世界或墙钟 CI 门槛"]
    fn digest_scale_wall_clock_evidence() {
        use std::{hint::black_box, time::Instant};

        let (world, _, _) = world_with_vehicle(true);
        let base = world.capture_snapshot().unwrap();
        // 仅放大规范记录负载，不伪装为已验证的十万辆运行时世界。
        // 一个大路线分组同时覆盖组内追加与顶层 live 序追加。
        for count in [10_000_u32, 100_000] {
            let mut snapshot = base.clone();
            snapshot.vehicles.clear();
            snapshot.live_order.clear();
            for index in 0..count {
                let mut vehicle = base.vehicles[0].clone();
                vehicle.snapshot_vehicle_id = u64::from(index) + 1;
                vehicle.progress_mm = index;
                snapshot.live_order.push(vehicle.snapshot_vehicle_id);
                snapshot.vehicles.push(vehicle);
            }
            let expected = deterministic_state_digest(&snapshot).unwrap();
            let mut samples = Vec::new();
            for _ in 0..3 {
                let started = Instant::now();
                let digest = black_box(deterministic_state_digest(black_box(&snapshot)).unwrap());
                samples.push(started.elapsed().as_nanos());
                assert_eq!(digest, expected);
            }
            println!("digest-records={count} samples_ns={samples:?} digest={expected:?}");
        }
    }

    #[test]
    fn pregrouped_route_bytes_match_linear_reference() {
        let (world, _, _) = world_with_vehicle(true);
        let snapshot = world.capture_snapshot().unwrap();
        for vehicle_count in [0, 1, 4, 24] {
            let mut route_records = Vec::new();
            for id in [17, 3, 29, 11] {
                let mut route = snapshot.routes[0].clone();
                route.snapshot_route_id = id;
                if id == 3 {
                    route.edges.push(route.edges[0]);
                }
                route_records.push((id, canonical_route_record(&route).unwrap()));
            }
            let mut vehicle_records = Vec::new();
            for index in 0..vehicle_count {
                let mut vehicle = snapshot.vehicles[0].clone();
                vehicle.progress_mm = index * 7;
                vehicle_records.push((
                    u64::from(index * 5 + 1),
                    canonical_vehicle_record(&vehicle).unwrap(),
                    [17, 3, 29][index as usize % 3],
                ));
            }
            // 保留基线的逐路线全扫，仅作小夹具字节参考；第四条路线保持空置。
            let mut expected = Vec::new();
            for (route_id, bytes) in &route_records {
                let mut bound: Vec<_> = vehicle_records
                    .iter()
                    .filter(|record| record.2 == *route_id)
                    .map(|record| record.1.as_slice())
                    .collect();
                bound.sort_unstable();
                let mut group = bytes.clone();
                push_u64(&mut group, bound.len() as u64);
                for record in bound {
                    push_record(&mut group, record);
                }
                expected.push(group);
            }
            expected.sort_unstable();
            route_records.sort_unstable_by_key(|record| record.0);
            vehicle_records.reverse();
            assert_eq!(
                canonical_route_groups(
                    &vehicle_records,
                    &route_records,
                    &mut NormalizationBudget {
                        remaining: u64::MAX,
                    },
                )
                .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn digest_is_stable_across_restore_handle_and_local_id_reassignment() {
        let (mut world, route, _) = world_with_vehicle(true);
        world.step(TickInput::new(100)).expect("step");
        // 捕获槽位序为 Active / Parked；restore 为避免 transient 非占用重叠会先
        // staging Parked，再 staging Active，因此新进程槽位与局部 ID 会反转。
        let _parked = world
            .spawn_parked_vehicle(
                ParkedVehicleSpawnInput::new(
                    laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    10_000,
                ),
                ParkingTarget::ExplicitSpace(
                    laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0),
                ),
            )
            .expect("parked second")
            .vehicle;
        let before = world.capture_snapshot().expect("capture");
        let before_digest = deterministic_state_digest(&before).expect("digest");
        let restored = restore_lfrs(
            &encode_lfrs(&before),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            crate::ExecutionConfig::new(std::num::NonZeroU32::MIN),
            SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
        )
        .expect("restore");
        let after = restored.world().capture_snapshot().expect("capture");
        assert_ne!(before.vehicles[0].status, before.vehicles[1].status);
        assert_ne!(before.vehicles[0].status, after.vehicles[0].status);
        assert_eq!(
            before_digest,
            deterministic_state_digest(&after).expect("digest")
        );
    }

    #[test]
    fn digest_reservation_failure_is_retryable_and_byte_stable() {
        // #532：规范化的每个预留注入点都失败关闭（无副作用），清点后
        // 重试得到同一摘要；排序表替换 BTreeMap 不改变输出字节（由上方
        // known-vector 与等价类测试兜底）。
        let (mut world, _, _) = world_with_vehicle(true);
        world.step(TickInput::new(100)).expect("step");
        let snapshot = world.capture_snapshot().expect("capture");
        let expected = deterministic_state_digest(&snapshot).expect("baseline digest");
        for fail_after in 0..14 {
            let failed =
                crate::admin::snapshot::with_snapshot_allocation_failure_after(fail_after, || {
                    deterministic_state_digest(&snapshot)
                });
            assert_eq!(
                failed,
                Err(SnapshotDigestError::ReservationFailed),
                "fail_after={fail_after}"
            );
        }
        assert_eq!(
            deterministic_state_digest(&snapshot).expect("retry after clearing"),
            expected
        );
    }

    /// 直算摘要与现行 capture + digest 路径在既有夹具上逐字节一致（#960 验收）。
    #[test]
    fn direct_digest_matches_capture_path_bytes() {
        let digest_limits = SnapshotDigestLimits::new(64 * 1024 * 1024);
        // ① install 后未 step：空 live 集。
        {
            let (world, _, _) = world_with_vehicle(true);
            let captured = world.capture_snapshot().expect("capture");
            assert_eq!(
                world.direct_state_digest(digest_limits).expect("direct"),
                deterministic_state_digest(&captured).expect("legacy"),
                "fresh world: 直算与捕获路径逐字节一致"
            );
        }
        // ② step 后：车辆携带 eligibility / lag / live 序推进。
        {
            let (mut world, _, _) = world_with_vehicle(true);
            world.step(crate::TickInput::new(100)).expect("step");
            let captured = world.capture_snapshot().expect("capture");
            assert_eq!(
                world.direct_state_digest(digest_limits).expect("direct"),
                deterministic_state_digest(&captured).expect("legacy"),
                "stepped world: 直算与捕获路径逐字节一致"
            );
        }
        // ③ 追加 parked 车辆：路线绑定多重集覆盖 parking 绑定变体。
        {
            let (mut world, route, _) = world_with_vehicle(true);
            world.step(crate::TickInput::new(100)).expect("step");
            let space = laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0);
            world
                .spawn_parked_vehicle(
                    crate::ParkedVehicleSpawnInput::new(
                        laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                        route,
                        0,
                        1_000,
                    ),
                    crate::ParkingTarget::ExplicitSpace(space),
                )
                .expect("spawn parked");
            let captured = world.capture_snapshot().expect("capture");
            assert_eq!(
                world.direct_state_digest(digest_limits).expect("direct"),
                deterministic_state_digest(&captured).expect("legacy"),
                "parked world: 直算与捕获路径逐字节一致"
            );
        }
    }

    /// 限额不足返回 `LimitExceeded` 且无副作用：同一世界放宽限额后重试，
    /// 字节与捕获路径一致。
    #[test]
    fn direct_digest_limit_exceeded_is_retryable_without_side_effects() {
        let (world, _, _) = world_with_vehicle(true);
        let tiny = SnapshotDigestLimits::new(16);
        assert_eq!(
            world.direct_state_digest(tiny),
            Err(SnapshotDigestError::LimitExceeded),
            "冲突视图或记录表的 scratch 即超过 16 字节"
        );
        let captured = world.capture_snapshot().expect("capture after retry");
        assert_eq!(
            world
                .direct_state_digest(SnapshotDigestLimits::new(64 * 1024 * 1024))
                .expect("retry with sufficient limits"),
            deterministic_state_digest(&captured).expect("legacy"),
            "限额失败无副作用，重试结果仍逐字节一致"
        );
    }

    #[test]
    fn direct_digest_matches_sparse_reused_slots_and_rejects_stale_handles() {
        let (mut world, original_route, original_vehicle) = world_with_vehicle(true);
        let edges = world.route_edges(original_route).unwrap().to_vec();
        let empty_route = world
            .register_route(crate::RouteRegisterInput::new(edges.clone()))
            .unwrap();
        let parked_route = world
            .register_route(crate::RouteRegisterInput::new(edges.clone()))
            .unwrap();
        let parked = world
            .spawn_parked_vehicle(
                crate::ParkedVehicleSpawnInput::new(
                    laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                    parked_route,
                    0,
                    1_000,
                ),
                crate::ParkingTarget::ExplicitSpace(
                    laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0),
                ),
            )
            .unwrap();
        world.despawn_vehicle(original_vehicle).unwrap();
        world.remove_route(original_route).unwrap();
        world.remove_route(empty_route).unwrap();
        let limits = SnapshotDigestLimits::new(1_048_576);
        let assert_same = |world: &crate::TrafficWorld| {
            assert_eq!(
                world.direct_state_digest(limits).unwrap(),
                deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap(),
            );
        };
        assert_same(&world);
        let reused_route = world
            .register_route(crate::RouteRegisterInput::new(edges))
            .unwrap();
        let reused_vehicle = world
            .spawn_vehicle(
                crate::VehicleSpawnInput::new(
                    laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                    reused_route,
                    0,
                    4_500,
                    0,
                )
                .with_open_entrance(),
            )
            .unwrap();
        assert_eq!(reused_vehicle.index(), original_vehicle.index());
        assert_ne!(reused_vehicle.generation(), original_vehicle.generation());
        assert_eq!(
            world.state.committed.live_order,
            [parked.vehicle, reused_vehicle]
        );
        assert_same(&world);
        world.state.committed.live_order.reverse();
        assert_same(&world);

        // 槽位相同而世代不同的旧句柄不能借助连续局部 ID 被误认成存活对象。
        world.state.committed.live_order[0] = original_vehicle;
        assert_eq!(
            world.direct_state_digest(limits),
            Err(SnapshotDigestError::InvariantViolation(
                StateInvariantViolation::DanglingLiveOrderEntry,
            )),
        );
        world.state.committed.live_order[0] = reused_vehicle;
        world
            .state
            .committed
            .vehicles
            .slot_mut(reused_vehicle.index() as usize)
            .state
            .as_mut()
            .unwrap()
            .route = empty_route;
        assert_eq!(
            world.direct_state_digest(limits),
            Err(SnapshotDigestError::InvariantViolation(
                StateInvariantViolation::DanglingRouteHandle,
            )),
        );
    }

    /// 直算路径的容量预留失败沿用 `ReservationFailed`（注入点与捕获/摘要轴
    /// 共用同一快照轴计数器），重试成功；fail_after 循环覆盖全部预留点，
    /// 含 live 顺序命中标记数组（先可失败预留、再初始化）。
    #[test]
    fn direct_digest_reservation_failure_fails_closed_and_is_retryable() {
        let (world, _, _) = world_with_vehicle(true);
        let limits = SnapshotDigestLimits::new(64 * 1024 * 1024);
        let expected = world.direct_state_digest(limits).unwrap();
        let mut reached_success = false;
        for fail_after in 0..256 {
            let failed =
                crate::admin::snapshot::with_snapshot_allocation_failure_after(fail_after, || {
                    world.direct_state_digest(limits)
                });
            if failed == Ok(expected) {
                reached_success = true;
                break;
            }
            assert_eq!(
                failed,
                Err(SnapshotDigestError::ReservationFailed),
                "fail_after={fail_after}"
            );
            assert!(
                world.direct_state_digest(limits).is_ok(),
                "注入清零后重试成功（fail_after={fail_after}）"
            );
        }
        assert!(reached_success, "all reservation sites covered");
        let captured = world.capture_snapshot().expect("capture after retry");
        assert_eq!(
            world
                .direct_state_digest(SnapshotDigestLimits::new(64 * 1024 * 1024))
                .expect("retry after clearing injection"),
            deterministic_state_digest(&captured).expect("legacy"),
        );
    }

    /// capture 助手错误映射：Conflict 不变量破坏进显式变体，预留失败保持
    /// 资源不足类。
    #[test]
    fn direct_capture_error_mapping_separates_invariant_from_reservation() {
        use crate::admin::snapshot::SnapshotCaptureError;
        assert_eq!(
            direct_capture_error(SnapshotCaptureError::ReservationFailed),
            SnapshotDigestError::ReservationFailed
        );
        assert_eq!(
            direct_capture_error(SnapshotCaptureError::ConflictInvariantViolation),
            SnapshotDigestError::InvariantViolation(
                StateInvariantViolation::ConflictAuthorityMismatch
            )
        );
    }
    /// 限额边界单调：存在精确临界值——恰好在阈值成功、少一字节即
    /// `LimitExceeded`（预算覆盖全部归一化分配，含成员表与 canonical 追加）。
    #[test]
    fn direct_digest_limit_boundary_is_exact_and_monotonic() {
        let (world, _, _) = world_with_vehicle(true);
        let high = SnapshotDigestLimits::new(1024 * 1024);
        assert!(world.direct_state_digest(high).is_ok(), "宽松限额必须成功");
        let mut low = 0_u64;
        let mut high_bytes = high.max_normalization_bytes();
        while high_bytes - low > 1 {
            let middle = low + (high_bytes - low) / 2;
            match world.direct_state_digest(SnapshotDigestLimits::new(middle)) {
                Ok(_) => high_bytes = middle,
                Err(SnapshotDigestError::LimitExceeded) => low = middle,
                Err(other) => panic!("二分中只允许限额错误，得到 {other:?}"),
            }
        }
        assert_eq!(
            world.direct_state_digest(SnapshotDigestLimits::new(high_bytes)),
            world.direct_state_digest(SnapshotDigestLimits::new(high_bytes)),
            "临界值稳定成功"
        );
        assert_eq!(
            world.direct_state_digest(SnapshotDigestLimits::new(high_bytes - 1)),
            Err(SnapshotDigestError::LimitExceeded),
            "临界值减一字节必须超限（预算覆盖全部实际分配）"
        );
    }

    #[test]
    fn captured_downstream_budget_matches_allocated_types_at_boundary() {
        use crate::admin::snapshot::{
            CapturedConflictDownstreamInterval, CapturedConflictPassage, SnapshotCaptureError,
        };
        let (world, vehicle) =
            crate::admin::format_admission::tests::world_with_conflict_reservation();
        let state = &world.state;
        let mut index = vec![None; state.binding.config.vehicle_capacity() as usize];
        let view = state.conflict_read().persistence_view(&mut index).unwrap();
        let reservation = view.authority(vehicle).unwrap().reservation;
        let vehicle_state = state.vehicle_state(vehicle).unwrap();
        let plan = state
            .reservation_downstream_claim_plan(reservation.passage_range(), vehicle_state.length_mm)
            .unwrap();
        let claims = reservation.downstream_claim_count() as usize;
        assert!(claims > 0, "fixture must contain downstream claims");
        // 按实际四个 Vec 的元素类型独立计算，不从被测预算器反推阈值。
        let expected_bytes = (core::mem::size_of::<CapturedConflictPassage>()
            * reservation.passage_range().passage_count() as usize
            + core::mem::size_of::<CapturedConflictDownstreamInterval>() * claims
            + core::mem::size_of::<crate::DownstreamInterval>() * claims
            + core::mem::size_of::<crate::DownstreamInterval>() * plan.raw_interval_capacity())
            as u64;
        let capture = |budget: &mut NormalizationBudget| {
            state.captured_vehicle_at(
                state.binding.revision.identity(),
                &view,
                &|_, _| 1,
                budget,
                vehicle.index() as usize,
                1,
            )
        };
        let mut exact = NormalizationBudget {
            remaining: expected_bytes,
        };
        let captured = capture(&mut exact).unwrap().unwrap();
        assert_eq!(
            captured
                .conflict_reservation
                .unwrap()
                .downstream_intervals
                .len(),
            claims
        );
        assert_eq!(
            exact.remaining, 0,
            "all allocated element bytes must be charged"
        );
        let mut short = NormalizationBudget {
            remaining: expected_bytes - 1,
        };
        assert!(matches!(
            capture(&mut short),
            Err(SnapshotCaptureError::LimitExceeded)
        ));
        let mut retry = NormalizationBudget {
            remaining: expected_bytes,
        };
        assert!(capture(&mut retry).unwrap().is_some());
        let snapshot = world.capture_snapshot().unwrap();
        assert_eq!(
            world
                .direct_state_digest(SnapshotDigestLimits::new(1_048_576))
                .unwrap(),
            deterministic_state_digest(&snapshot).unwrap(),
        );
    }

    /// live 顺序清空（保留存活车辆）不得签发摘要：完整性检查拒绝。
    #[test]
    fn direct_digest_rejects_cleared_live_order() {
        let (mut world, _, _) = world_with_vehicle(true);
        world.state.committed.live_order.clear();
        assert_eq!(
            world.direct_state_digest(SnapshotDigestLimits::new(1024 * 1024)),
            Err(SnapshotDigestError::InvariantViolation(
                StateInvariantViolation::IncompleteLiveOrder,
            )),
            "遗漏的 live 顺序按不变量破坏拒绝，不签发摘要"
        );
    }

    /// live 顺序长度相符但含重复条目：唯一性标记拒绝。
    #[test]
    fn direct_digest_rejects_duplicated_live_order() {
        let (mut world, _, _) = world_with_vehicle(true);
        let first = laneflow_static_contract::LaneEdgeOrdinal::from_raw(0);
        let successors: &[laneflow_static_contract::LaneEdgeOrdinal] =
            world.traffic().successors(first).unwrap_or(&[]);
        let edges = if let Some(second) = successors.first() {
            vec![first, *second]
        } else {
            vec![first]
        };
        let second = world
            .register_route(crate::RouteRegisterInput::new(edges))
            .expect("second route");
        // parked spawn 不占车道：与现有 Active 车辆位置无冲突。
        let space = laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0);
        world
            .spawn_parked_vehicle(
                crate::ParkedVehicleSpawnInput::new(
                    laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                    second,
                    0,
                    1_000,
                ),
                crate::ParkingTarget::ExplicitSpace(space),
            )
            .expect("second vehicle");
        // 两辆车、live 顺序替换为同一句柄两次：长度相符但重复。
        let duplicated = world.state.committed.live_order[0];
        world.state.committed.live_order[1] = duplicated;
        assert_eq!(
            world.direct_state_digest(SnapshotDigestLimits::new(1024 * 1024)),
            Err(SnapshotDigestError::InvariantViolation(
                StateInvariantViolation::IncompleteLiveOrder,
            )),
        );
    }

    /// committed 的 live 路线计数失真：与实测存活路线数核对后拒绝。
    #[test]
    fn direct_digest_rejects_distorted_route_count() {
        let (mut world, _, _) = world_with_vehicle(true);
        world.state.committed.live_route_count += 1;
        assert_eq!(
            world.direct_state_digest(SnapshotDigestLimits::new(1024 * 1024)),
            Err(SnapshotDigestError::InvariantViolation(
                StateInvariantViolation::LiveRouteCountMismatch,
            )),
        );
    }
    /// 瞬态失真（状态在检查点之间发生失真、下一个检查点前还原）不留摘要差异
    /// （漏检面 §5 第 13 条的真实注入）：直改 committed 的等待区占用后还原，
    /// 检查点时刻的摘要与干净状态一致；未还原的失真改变摘要（对照）。
    /// 集成测试只有公共 API，命令会永久改变游标，无法构造本场景。
    #[test]
    fn transient_distortion_between_checkpoints_leaves_digest_unchanged() {
        let (mut world, _, _) = world_with_vehicle(true);
        let limits = SnapshotDigestLimits::new(64 * 1024 * 1024);
        let clean = world.direct_state_digest(limits).expect("clean digest");
        assert!(
            !world.state.committed.waiting_zones.is_empty(),
            "fixture must carry waiting zones"
        );
        // 注入字段取语义自洽的 next_admission_sequence（单调历史计数器，
        // 与成员计数无一致性约束）：直改 occupancy 会造出成员表脱钩的
        // 无效等待聚合，不属 §5.13 的合法瞬态（审阅意见）。
        let zone = world
            .state
            .committed
            .waiting_zones
            .iter()
            .position(|zone| zone.occupancy == 0)
            .expect("fixture must carry an empty waiting zone");
        let original = world.state.committed.waiting_zones[zone];
        // 失真窗口：历史计数被直改（非命令、不动游标、不破坏聚合不变量）。
        world.state.committed.waiting_zones[zone].next_admission_sequence =
            original.next_admission_sequence.wrapping_add(1);
        let inside = world
            .direct_state_digest(limits)
            .expect("digest inside window");
        assert_ne!(
            inside, clean,
            "窗口内的失真改变摘要（对照：检查点落入窗口可见）"
        );
        // 窗口收敛：还原直改，检查点时刻摘要回到干净值。
        world.state.committed.waiting_zones[zone] = original;
        assert_eq!(
            world
                .direct_state_digest(limits)
                .expect("digest after restore"),
            clean,
            "检查点之间的瞬态收敛不留摘要差异（漏检面 §5 第 13 条）"
        );
    }

    #[test]
    fn digest_distinguishes_vehicle_binding_across_identical_routes() {
        // 两条内容相同的路线：车辆绑定分布不同 => 逻辑状态不同（remove 语义
        // 不同），摘要必须区分；内容与绑定均相同的实例互换 => 等价，摘要相等。
        let (world, _, _) = world_with_vehicle(true);
        let base = world.capture_snapshot().expect("capture");
        let second_vehicle = {
            let mut vehicle = base.vehicles[0].clone();
            vehicle.snapshot_vehicle_id = base.vehicles[0].snapshot_vehicle_id + 1;
            vehicle
        };
        let mut split = base.clone();
        split.routes.push(CapturedRoute {
            snapshot_route_id: base.routes[0].snapshot_route_id + 1,
            edges: base.routes[0].edges.clone(),
        });
        split.vehicles.push(second_vehicle.clone());
        split.vehicles[1].snapshot_route_id = split.routes[1].snapshot_route_id;
        split.live_order.push(second_vehicle.snapshot_vehicle_id);

        // A：两车分挂两条相同路线；B：两车同挂第一条、第二条空置。
        let mut merged = split.clone();
        merged.vehicles[1].snapshot_route_id = split.routes[0].snapshot_route_id;
        assert_ne!(
            deterministic_state_digest(&split).expect("digest"),
            deterministic_state_digest(&merged).expect("digest")
        );

        // 互换可交换实例的车辆绑定（分组不变）=> 摘要相等。
        let mut swapped = split.clone();
        swapped.vehicles[0].snapshot_route_id = split.routes[1].snapshot_route_id;
        swapped.vehicles[1].snapshot_route_id = split.routes[0].snapshot_route_id;
        assert_eq!(
            deterministic_state_digest(&split).expect("digest"),
            deterministic_state_digest(&swapped).expect("digest")
        );
    }

    #[test]
    fn digest_distinguishes_live_order_across_different_routes() {
        // 两条内容不同的路线、两辆同值车：live 序换序 => pose 批次顺序不同，
        // 摘要必须区分；所属路线内容相同时换序保持等价（边界登记）。
        let (world, _, _) = world_with_vehicle(true);
        let base = world.capture_snapshot().expect("capture");
        let second_route_id = base.routes[0].snapshot_route_id + 1;
        let second_vehicle_id = base.vehicles[0].snapshot_vehicle_id + 1;

        let mut forward = base.clone();
        let mut different_edges = base.routes[0].edges.clone();
        // 内容不同：追加重复首边（occurrence 语义允许重复边）。
        different_edges.push(base.routes[0].edges[0]);
        forward.routes.push(CapturedRoute {
            snapshot_route_id: second_route_id,
            edges: different_edges,
        });
        let mut second = base.vehicles[0].clone();
        second.snapshot_vehicle_id = second_vehicle_id;
        second.snapshot_route_id = second_route_id;
        forward.vehicles.push(second.clone());
        forward.live_order.push(second_vehicle_id);

        let mut reversed = forward.clone();
        reversed.live_order = vec![second_vehicle_id, base.vehicles[0].snapshot_vehicle_id];
        assert_ne!(
            deterministic_state_digest(&forward).expect("digest"),
            deterministic_state_digest(&reversed).expect("digest")
        );

        // 边界：两条路线内容改回相同 => 条目字节相同，换序等价。
        let mut equal_forward = forward.clone();
        equal_forward.routes[1].edges = base.routes[0].edges.clone();
        let mut equal_reversed = equal_forward.clone();
        equal_reversed.live_order = reversed.live_order.clone();
        assert_eq!(
            deterministic_state_digest(&equal_forward).expect("digest"),
            deterministic_state_digest(&equal_reversed).expect("digest")
        );
    }

    #[test]
    fn digest_ignores_local_ids_and_source_audit() {
        let (world, _, _) = world_with_vehicle(true);
        let original = world.capture_snapshot().expect("capture");
        let expected = deterministic_state_digest(&original).expect("digest");
        // digest 7：独立保留已审核的车辆/路线记录后缀，替换版本与 LFCA 六节摘要，
        // 追加 tag + 从身份表读取的 policy StableId，以及空 Conflict
        // lag 分类的显式计数；本向量锁定完整 digest 7 前像。
        // 下方等价类同时约束来源审计与局部 ID 不进入摘要。
        assert_eq!(
            expected,
            Sha256Digest::from_bytes([
                0xdf, 0x23, 0xb5, 0x88, 0x8b, 0x33, 0x1e, 0x67, 0xc3, 0xd8, 0xf6, 0x96, 0x64, 0x9c,
                0xe0, 0xce, 0x55, 0x8b, 0xe3, 0xb3, 0x74, 0xf2, 0x67, 0x3c, 0xd8, 0xce, 0xab, 0xc9,
                0x7e, 0xee, 0x87, 0xf3
            ])
        );
        let mut equivalent = original.clone();
        equivalent.routes[0].snapshot_route_id = 91;
        equivalent.vehicles[0].snapshot_vehicle_id = 72;
        equivalent.vehicles[0].snapshot_route_id = 91;
        equivalent.live_order[0] = 72;
        let crate::CommittedNetworkSource::Published { reference } = &original.source;
        equivalent.source = crate::CommittedNetworkSource::Published {
            reference: crate::PublishedLfcaReference::new(
                "asset://republished-audit-name",
                laneflow_static_contract::Sha256Digest::from_bytes([0xa5; 32]),
                laneflow_static_contract::ExactByteLength::new(777),
                reference.network_revision(),
            )
            .expect("source"),
        };
        assert_eq!(
            expected,
            deterministic_state_digest(&equivalent).expect("digest")
        );
    }

    #[test]
    fn semantic_capacity_and_live_order_change_the_digest() {
        let (mut world, route, _) = world_with_vehicle(true);
        world
            .spawn_parked_vehicle(
                ParkedVehicleSpawnInput::new(
                    laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    10_000,
                ),
                ParkingTarget::ExplicitSpace(
                    laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0),
                ),
            )
            .expect("parked second");
        let original = world.capture_snapshot().expect("capture");
        let expected = deterministic_state_digest(&original).expect("digest");

        let mut larger_capacity = original.clone();
        larger_capacity.config = WorldConfig::new(
            larger_capacity.config.vehicle_capacity() + 1,
            larger_capacity.config.route_capacity(),
            larger_capacity.config.route_edge_occurrence_capacity(),
            larger_capacity.config.route_conflict_occurrence_capacity(),
            larger_capacity.config.fixed_delta_time_ms(),
        );
        assert_ne!(
            expected,
            deterministic_state_digest(&larger_capacity).expect("digest")
        );

        let mut swapped_live_order = original;
        swapped_live_order.live_order.swap(0, 1);
        assert_ne!(
            expected,
            deterministic_state_digest(&swapped_live_order).expect("digest")
        );
    }
}
