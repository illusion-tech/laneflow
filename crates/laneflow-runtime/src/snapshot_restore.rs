//! `LFRS` v5 的 verifier-first 读取、语义 lowering 与原子新世界恢复。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use laneflow_runtime_snapshot_wire::generated::lane_flow::runtime_snapshot::v5 as wire;
use laneflow_runtime_snapshot_wire::runtime::VerifierOptions;
use laneflow_static_contract::{
    ConflictZoneId, LaneEdgeId, ManeuverGateId, ManeuverPathId, ParkingFacilityId, ParkingSpaceId,
    ParticipantClassId, ParticipantStreamId, StableId128, VehicleProfileId, WaitingZoneId,
    WaitingZoneOrdinal,
};
use laneflow_static_network::SharedNetworkRevision;
use thiserror::Error;

use crate::{
    AdmittedRouteRegisterError, AdmittedRouteRegisterInput, CommittedNetworkSource, InstallError,
    ManeuverTraversalPhase, ManeuverTraversalState, ObservationStateSequence,
    ParkedVehicleSpawnInput, ParkingError, ParkingTarget, ReserveParkingTarget, RouteHandle,
    SpawnError, StepError, TrafficWorld, VehicleHandle, VehicleSpawnInput, VehicleStatus,
    VirtualEntryAnchorSelector, WaitingMembership, WorldConfig,
};
use crate::{RUNTIME_STATE_VERSION, SNAPSHOT_FORMAT_VERSION};

const MIN_SIZE_PREFIXED_LFRS_BYTES: usize = 12;
const MAX_SCHEMA_TABLE_DEPTH: usize = 6;
const APPARENT_SIZE_MULTIPLIER: usize = 16;
const MICROMETRES_PER_MILLIMETRE: u16 = 1_000;
const ROOT_V5_FIELDS: usize = vtable_field_count(wire::RuntimeSnapshot::VT_CONFLICT_LAG_STATES);
const WORLD_CONFIG_V5_FIELDS: usize =
    vtable_field_count(wire::WorldConfigBinding::VT_FIXED_DELTA_TIME_MS);
const PUBLISHED_SOURCE_V5_FIELDS: usize =
    vtable_field_count(wire::PublishedSourceBinding::VT_NETWORK_REVISION);
const ROUTE_V5_FIELDS: usize = vtable_field_count(wire::SnapshotRoute::VT_EDGES);
const VEHICLE_V5_FIELDS: usize = vtable_field_count(wire::SnapshotVehicle::VT_CONFLICT_RESERVATION);
const PARKING_BINDING_V5_FIELDS: usize =
    vtable_field_count(wire::ParkingBinding::VT_VIRTUAL_ENTRY_PROGRESS_MM);
const MANEUVER_TRAVERSAL_V5_FIELDS: usize =
    vtable_field_count(wire::ManeuverTraversalBinding::VT_PHASE_GATE);
const WAITING_MEMBERSHIP_V5_FIELDS: usize =
    vtable_field_count(wire::WaitingMembershipBinding::VT_ADMISSION_SEQUENCE);
const WAITING_ZONE_STATE_V5_FIELDS: usize =
    vtable_field_count(wire::WaitingZoneState::VT_NEXT_ADMISSION_SEQUENCE);
const CONFLICT_LOCATOR_V5_FIELDS: usize =
    vtable_field_count(wire::ConflictPassageLocatorBinding::VT_CONFLICT_ZONE);
const CONFLICT_ELIGIBILITY_V5_FIELDS: usize =
    vtable_field_count(wire::ConflictEligibilityBinding::VT_FIRST_ELIGIBLE_TICK);
const CONFLICT_PASSAGE_V5_FIELDS: usize =
    vtable_field_count(wire::ConflictPassageBinding::VT_CLEARANCE_PROGRESS_MM);
const CONFLICT_DOWNSTREAM_V5_FIELDS: usize =
    vtable_field_count(wire::ConflictDownstreamIntervalBinding::VT_END_MM);
const CONFLICT_RESERVATION_V5_FIELDS: usize =
    vtable_field_count(wire::ConflictReservationBinding::VT_DOWNSTREAM_INTERVALS);
const CONFLICT_LAG_STATE_V5_FIELDS: usize =
    vtable_field_count(wire::ConflictLagState::VT_REFERENCE_TIME_MS);

const fn vtable_field_count(
    last_field: laneflow_runtime_snapshot_wire::runtime::VOffsetT,
) -> usize {
    (last_field as usize - 4) / 2 + 1
}

/// 快照读取的调用方硬上限维度。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotLimitDimension {
    /// 完整 size-prefixed `LFRS` 字节数。
    WireBytes,
    /// Published 来源的不透明 asset key 字节数。
    AssetKeyBytes,
    /// 路线表项数。
    Routes,
    /// 车辆表项数。
    Vehicles,
    /// 全部路线边 occurrence 总数。
    RouteEdgeOccurrences,
    /// 全部路线冲突 passage occurrence 总数。
    RouteConflictOccurrences,
    /// FlatBuffers verifier 表预算或 apparent-size 预算。
    VerifierBudget,
}

/// `LFRS` 读取显式上限。路线、车辆与 occurrence 还会同时受保存配置和目标配置约束。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotRestoreLimits {
    max_wire_bytes: u64,
    max_asset_key_bytes: u64,
}

impl SnapshotRestoreLimits {
    /// 构造调用方上限；本类型不提供猜测性的全局默认值。
    #[must_use]
    pub const fn new(max_wire_bytes: u64, max_asset_key_bytes: u64) -> Self {
        Self {
            max_wire_bytes,
            max_asset_key_bytes,
        }
    }

    /// 最大输入制品字节数。
    #[must_use]
    pub const fn max_wire_bytes(self) -> u64 {
        self.max_wire_bytes
    }

    /// 最大 Published asset key 字节数。
    #[must_use]
    pub const fn max_asset_key_bytes(self) -> u64 {
        self.max_asset_key_bytes
    }
}

/// 快照恢复失败。任一错误都不会返回半个 `TrafficWorld`。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum SnapshotRestoreError {
    /// 输入命中显式读取、配置或 verifier 上限。
    #[error("快照超过 {dimension:?} 上限: limit={limit}, actual={actual}")]
    LimitExceeded {
        /// 超限维度。
        dimension: SnapshotLimitDimension,
        /// 允许上限。
        limit: u64,
        /// 实际值。
        actual: u64,
    },
    /// size-prefixed framing 过短。
    #[error("LFRS framing 截断")]
    TruncatedFraming,
    /// size prefix 与实际剩余字节数不符。
    #[error("LFRS size prefix 不一致: declared={declared}, actual={actual}")]
    SizePrefixMismatch {
        /// prefix 声明值。
        declared: u64,
        /// 实际 payload 字节数。
        actual: u64,
    },
    /// file identifier 不是 `LFRS`。
    #[error("LFRS file identifier 不匹配")]
    FileIdentifierMismatch,
    /// FlatBuffers verifier 拒绝结构。
    #[error("LFRS FlatBuffers 结构无效")]
    InvalidFlatbuffer,
    /// 选择 tag 与 StableId 的闭合形状不合法。
    #[error("LFRS 世界策略绑定无效")]
    InvalidPolicyBinding,
    /// 容器格式版本未知。
    #[error("LFRS format version 不支持: {actual}")]
    UnsupportedFormatVersion {
        /// 实际版本。
        actual: u32,
    },
    /// Runtime 逻辑状态版本未知。
    #[error("Runtime state version 不支持: {actual}")]
    UnsupportedRuntimeStateVersion {
        /// 实际版本。
        actual: u16,
    },
    /// v5 table 出现 schema 未登记的字段槽；这类字段可能携带禁绑状态。
    #[error("LFRS v5 table {table} 含未知字段槽: supported={supported}, actual={actual}")]
    UnknownTableFields {
        /// table 名。
        table: &'static str,
        /// v1 登记字段数。
        supported: usize,
        /// wire vtable 声明字段数。
        actual: usize,
    },
    /// 必需的结构字段缺席。
    #[error("LFRS 必需字段缺席: {field}")]
    MissingField {
        /// schema 字段名。
        field: &'static str,
    },
    /// 来源种类未闭合到 v1 Published。
    #[error("LFRS source kind 不支持: {actual}")]
    UnsupportedSourceKind {
        /// 原始枚举值。
        actual: u8,
    },
    /// Published asset key 为空。
    #[error("LFRS Published asset key 不能为空")]
    EmptyAssetKey,
    /// 快照来源修订与快照根修订不一致。
    #[error("LFRS Published 来源与快照根的路网修订不一致")]
    SnapshotSourceRevisionMismatch,
    /// 快照修订与目标根不一致。
    #[error("LFRS 路网修订与目标根不一致")]
    NetworkRevisionMismatch,
    /// 任一静态契约版本轴与目标根不一致。
    #[error("LFRS 静态契约版本集与目标根不一致")]
    StaticContractVersionsMismatch,
    /// 保存配置的时钟关系损坏或溢出。
    #[error("LFRS 时钟不满足 time_ms == tick * fixed_delta_time_ms")]
    InvalidClock,
    /// 目标固定步长与快照不同。
    #[error("目标 fixed_delta_time_ms 与快照不一致: snapshot={snapshot}, target={target}")]
    FixedDeltaTimeMismatch {
        /// 保存值。
        snapshot: u64,
        /// 目标值。
        target: u64,
    },
    /// 目标行为语义容量小于保存配置。
    #[error("目标 {dimension:?} 容量不得缩小: snapshot={snapshot}, target={target}")]
    TargetCapacitySmaller {
        /// 容量维度。
        dimension: SnapshotLimitDimension,
        /// 保存配置值。
        snapshot: u64,
        /// 目标配置值。
        target: u64,
    },
    /// 快照局部路线 ID 为保留零值。
    #[error("snapshot_route_id 不得为零")]
    ZeroRouteId,
    /// 快照局部路线 ID 重复。
    #[error("snapshot_route_id 重复: {snapshot_route_id}")]
    DuplicateRouteId {
        /// 重复 ID。
        snapshot_route_id: u64,
    },
    /// 快照局部车辆 ID 为保留零值。
    #[error("snapshot_vehicle_id 不得为零")]
    ZeroVehicleId,
    /// 快照局部车辆 ID 重复。
    #[error("snapshot_vehicle_id 重复: {snapshot_vehicle_id}")]
    DuplicateVehicleId {
        /// 重复 ID。
        snapshot_vehicle_id: u64,
    },
    /// 车辆引用了不存在的快照路线。
    #[error("车辆 {snapshot_vehicle_id} 引用未知路线 {snapshot_route_id}")]
    UnknownRouteReference {
        /// 车辆 ID。
        snapshot_vehicle_id: u64,
        /// 路线 ID。
        snapshot_route_id: u64,
    },
    /// live 顺序含未知车辆 ID。
    #[error("live_order 引用未知车辆 {snapshot_vehicle_id}")]
    UnknownLiveOrderVehicle {
        /// 车辆 ID。
        snapshot_vehicle_id: u64,
    },
    /// live 顺序重复车辆 ID。
    #[error("live_order 重复车辆 {snapshot_vehicle_id}")]
    DuplicateLiveOrderVehicle {
        /// 车辆 ID。
        snapshot_vehicle_id: u64,
    },
    /// live 顺序不是车辆表的精确排列。
    #[error("live_order 不是车辆表的精确排列")]
    IncompleteLiveOrder,
    /// 车辆状态枚举未知或 Unspecified。
    #[error("车辆 {snapshot_vehicle_id} 的 status 不支持: {actual}")]
    InvalidVehicleStatus {
        /// 车辆 ID。
        snapshot_vehicle_id: u64,
        /// 原始枚举值。
        actual: u8,
    },
    /// 车辆 profile 稳定标识在目标根中未知或 kind 不匹配。
    #[error("车辆 {snapshot_vehicle_id} 的 profile 稳定标识未知")]
    UnknownVehicleProfile {
        /// 车辆 ID。
        snapshot_vehicle_id: u64,
    },
    /// 车辆 class 稳定标识在目标根中未知或 kind 不匹配。
    #[error("车辆 {snapshot_vehicle_id} 的 class 稳定标识未知")]
    UnknownParticipantClass {
        /// 车辆 ID。
        snapshot_vehicle_id: u64,
    },
    /// 保存 class 与 profile 派生 class 不一致。
    #[error("车辆 {snapshot_vehicle_id} 的 class 与 profile 不一致")]
    ProfileClassMismatch {
        /// 车辆 ID。
        snapshot_vehicle_id: u64,
    },
    /// 停车位稳定标识在目标根中未知或 kind 不匹配。
    #[error("车辆 {snapshot_vehicle_id} 的停车位稳定标识未知")]
    UnknownParkingSpace {
        /// 车辆 ID。
        snapshot_vehicle_id: u64,
    },
    /// 停车设施稳定标识在目标根中未知或 kind 不匹配。
    #[error("车辆 {snapshot_vehicle_id} 的停车设施稳定标识未知")]
    UnknownParkingFacility { snapshot_vehicle_id: u64 },
    /// parking binding state 未知或 Unspecified。
    #[error("车辆 {snapshot_vehicle_id} 的 parking binding state 不支持: {actual}")]
    InvalidParkingBindingState {
        snapshot_vehicle_id: u64,
        actual: u8,
    },
    /// parking target kind 未知或 Unspecified。
    #[error("车辆 {snapshot_vehicle_id} 的 parking target kind 不支持: {actual}")]
    InvalidParkingTargetKind {
        snapshot_vehicle_id: u64,
        actual: u8,
    },
    /// Reserved/Occupied、target kind 与 semantic entry presence 不闭合。
    #[error("车辆 {snapshot_vehicle_id} 的 parking binding shape 非法")]
    InvalidParkingBindingShape { snapshot_vehicle_id: u64 },
    /// virtual Reserved semantic entry 在目标设施中没有 exact 对应。
    #[error("车辆 {snapshot_vehicle_id} 的 virtual parking entry 无 exact 对应")]
    UnknownVirtualParkingEntry { snapshot_vehicle_id: u64 },
    /// parked 状态与停车绑定不一致，或非 parked 状态携带停车绑定。
    #[error("车辆 {snapshot_vehicle_id} 的 parked 状态与停车绑定不一致")]
    ParkingStatusMismatch {
        /// 车辆 ID。
        snapshot_vehicle_id: u64,
    },
    /// 亚毫米余数不在 `0..1000`。
    #[error("车辆 {snapshot_vehicle_id} 的 carry_um 越界: {actual}")]
    CarryOutOfRange {
        /// 车辆 ID。
        snapshot_vehicle_id: u64,
        /// 实际值。
        actual: u16,
    },
    /// Parked / Completed 车辆仍携带非零速度或余数。
    #[error("车辆 {snapshot_vehicle_id} 的非 Active 状态仍携带运动余量")]
    InvalidInactiveMotion {
        /// 车辆 ID。
        snapshot_vehicle_id: u64,
    },
    /// Completed 车辆不是路线末端状态。
    #[error("车辆 {snapshot_vehicle_id} 的 Completed 状态不在路线末端")]
    InvalidCompletedState {
        /// 车辆 ID。
        snapshot_vehicle_id: u64,
    },
    /// Waiting traversal/membership 的 stable identity、route occurrence 或 phase 不闭合。
    #[error("车辆 {snapshot_vehicle_id} 的 Waiting authority 非法")]
    InvalidWaitingAuthority { snapshot_vehicle_id: u64 },
    /// WaitingZone state row 的 stable identity 未知或重复。
    #[error("WaitingZone state row 非法或重复")]
    InvalidWaitingZoneState,
    /// WaitingZone occupancy/counter/member/queue 关系不闭合。
    #[error("WaitingZone snapshot aggregate 不闭合")]
    WaitingInvariantViolation,
    /// 车辆的资格时钟或 Clearing reservation 不能从稳定身份和车身重建。
    #[error("车辆 {snapshot_vehicle_id} 的 Conflict authority 非法")]
    InvalidConflictAuthority { snapshot_vehicle_id: u64 },
    /// Conflict lag 行未排序、重复、悬空、类别非法或时间在快照未来。
    #[error("Conflict lag history 非法")]
    InvalidConflictHistory,
    /// 路线经规范化 admitted 入口恢复失败。
    #[error("路线 {snapshot_route_id} 恢复失败: {error}")]
    Route {
        /// 快照路线 ID。
        snapshot_route_id: u64,
        /// 共同路线入口错误。
        error: AdmittedRouteRegisterError,
    },
    /// 车辆经共同 spawn 不变量恢复失败。
    #[error("车辆 {snapshot_vehicle_id} 恢复失败: {error}")]
    Vehicle {
        /// 快照车辆 ID。
        snapshot_vehicle_id: u64,
        /// spawn 错误。
        error: SpawnError,
    },
    /// 停车占用经共同停车入口恢复失败。
    #[error("车辆 {snapshot_vehicle_id} 停车恢复失败: {error}")]
    Parking {
        /// 快照车辆 ID。
        snapshot_vehicle_id: u64,
        /// 停车错误。
        error: ParkingError,
    },
    /// 目标 world 安装失败。
    #[error("目标 world 安装失败: {0}")]
    Install(InstallError),
    /// 最终占用索引重建失败。
    #[error("恢复后的占用索引重建失败: {0}")]
    Occupancy(StepError),
    /// 从已恢复 membership 重建并验证 Waiting 依赖图失败。
    #[error("恢复后的 Waiting 依赖图重建失败: {0}")]
    WaitingDependencyRebuild(StepError),
}

/// 原子恢复成功结果：新 world 与快照局部 ID 到新句柄的映射。
pub struct RestoredSnapshot {
    world: TrafficWorld,
    routes: Vec<(u64, RouteHandle)>,
    vehicles: Vec<(u64, VehicleHandle)>,
}

impl std::fmt::Debug for RestoredSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RestoredSnapshot")
            .field("world_id", &self.world.world_id())
            .field("routes", &self.routes)
            .field("vehicles", &self.vehicles)
            .finish()
    }
}

impl RestoredSnapshot {
    /// 借用完整恢复后的 world。
    #[must_use]
    pub const fn world(&self) -> &TrafficWorld {
        &self.world
    }

    /// 消费结果并取得 world。
    #[must_use]
    pub fn into_world(self) -> TrafficWorld {
        self.world
    }

    /// 全部快照局部路线 ID 到新句柄的映射，按局部 ID 升序。
    ///
    /// 宿主用它重建检查点已有路线的耐久身份表；检查点之后新注册路线的宿主 ID
    /// 仍由输入命令载荷拥有。
    #[must_use]
    pub fn route_mappings(&self) -> &[(u64, RouteHandle)] {
        &self.routes
    }

    /// 全部快照局部车辆 ID 到新句柄的映射，按局部 ID 升序。
    ///
    /// 宿主用它重建检查点已有车辆的耐久身份表；检查点之后新生成车辆的宿主 ID
    /// 仍由输入命令载荷拥有。
    #[must_use]
    pub fn vehicle_mappings(&self) -> &[(u64, VehicleHandle)] {
        &self.vehicles
    }

    /// 按快照局部路线 ID 查询新句柄。
    #[must_use]
    pub fn route_handle(&self, snapshot_route_id: u64) -> Option<RouteHandle> {
        self.routes
            .binary_search_by_key(&snapshot_route_id, |(id, _)| *id)
            .ok()
            .map(|index| self.routes[index].1)
    }

    /// 按快照局部车辆 ID 查询新句柄。
    #[must_use]
    pub fn vehicle_handle(&self, snapshot_vehicle_id: u64) -> Option<VehicleHandle> {
        self.vehicles
            .binary_search_by_key(&snapshot_vehicle_id, |(id, _)| *id)
            .ok()
            .map(|index| self.vehicles[index].1)
    }
}

/// verifier-first 读取 `LFRS`，核对目标根/来源/配置并原子构造一个新 world。
///
/// fresh restore 从 [`crate::WorldGeneration::INITIAL`] 建立新观测 stream；调用方不得让
/// 同一 `world_id` 的旧 world 或旧 session 与返回值并存。任一失败只丢弃局部 staging，
/// 不返回半恢复 world。
pub fn restore_lfrs(
    bytes: &[u8],
    revision: Arc<SharedNetworkRevision>,
    source: CommittedNetworkSource,
    target_config: WorldConfig,
    limits: SnapshotRestoreLimits,
) -> Result<RestoredSnapshot, SnapshotRestoreError> {
    let root = verify_lfrs(bytes, limits)?;
    validate_bindings(root, revision.as_ref(), &source, target_config, limits)?;

    // 路线重编译必须得到完整实际冲突出现项总数，才能同时校验
    // 快照与目标容量。这个 staging world 不对外发布，也不按该上限预分配内存。
    let staging_config = WorldConfig::new(
        target_config.vehicle_capacity(),
        target_config.route_capacity(),
        target_config.route_edge_occurrence_capacity(),
        u64::MAX,
        target_config.worker_count(),
        target_config.fixed_delta_time_ms(),
    );
    let mut world = TrafficWorld::install(
        revision,
        staging_config,
        source,
        root.world_id(),
        decode_world_policy(root.world_policy())?,
    )
    .map_err(SnapshotRestoreError::Install)?;
    let root_revision = root
        .network_revision()
        .expect("binding validation requires network_revision");
    let contracts = root
        .static_contract_versions()
        .expect("binding validation requires static_contract_versions");

    let mut route_map = BTreeMap::new();
    for route in root.routes() {
        let snapshot_route_id = route.snapshot_route_id();
        if snapshot_route_id == 0 {
            return Err(SnapshotRestoreError::ZeroRouteId);
        }
        if route_map.contains_key(&snapshot_route_id) {
            return Err(SnapshotRestoreError::DuplicateRouteId { snapshot_route_id });
        }
        let stable_edges = route
            .edges()
            .iter()
            .map(|stable_id| StableId128::from_bytes(stable_id.0))
            .collect::<Vec<_>>();
        let handle = world
            .register_admitted_route(AdmittedRouteRegisterInput::new(
                laneflow_static_contract::NetworkRevisionId::from_digest(
                    laneflow_static_contract::Sha256Digest::from_bytes(root_revision.0),
                ),
                contracts.network_revision_derivation_version(),
                stable_edges,
            ))
            .map_err(|error| SnapshotRestoreError::Route {
                snapshot_route_id,
                error,
            })?;
        route_map.insert(snapshot_route_id, handle);
    }
    let snapshot_config = root.world_config();
    validate_state_count(
        SnapshotLimitDimension::RouteConflictOccurrences,
        world.live_route_conflict_occurrence_count,
        snapshot_config.route_conflict_occurrence_capacity(),
        target_config.route_conflict_occurrence_capacity(),
    )?;
    world.config = target_config;
    // 在私有 staging 恢复已提交时钟；Waiting phase 保留历史归因，不按当前信号重解释。
    world.tick_index = root.tick();
    world.time_ms = root.time_ms();
    world.event_cursor = root.event_cursor();
    world.refresh_signals();

    let vehicle_rows = root.vehicles();
    let mut vehicle_map = BTreeMap::new();
    // 非 Active 先恢复，Active 最后恢复；每辆车都以最终状态一次提交，
    // 因此 Completed 不会被临时当成 Active，Active 的 carry 也参与提交前 3A 校验。
    for active_pass in [false, true] {
        for vehicle in vehicle_rows {
            let status = decode_vehicle_status(vehicle.snapshot_vehicle_id(), vehicle.status())?;
            if (status == VehicleStatus::Active) != active_pass {
                continue;
            }
            restore_vehicle(&mut world, vehicle, status, &route_map, &mut vehicle_map)?;
        }
    }

    let mut seen_live = BTreeSet::new();
    let mut live_order = Vec::with_capacity(root.live_order().len());
    for snapshot_vehicle_id in root.live_order() {
        let Some(handle) = vehicle_map.get(&snapshot_vehicle_id).copied() else {
            return Err(SnapshotRestoreError::UnknownLiveOrderVehicle {
                snapshot_vehicle_id,
            });
        };
        if !seen_live.insert(snapshot_vehicle_id) {
            return Err(SnapshotRestoreError::DuplicateLiveOrderVehicle {
                snapshot_vehicle_id,
            });
        }
        live_order.push(handle);
    }
    if seen_live.len() != vehicle_map.len() {
        return Err(SnapshotRestoreError::IncompleteLiveOrder);
    }

    world.live_order = live_order;
    world.rebuild_active_order();
    restore_waiting_aggregate(&mut world, root)?;
    restore_conflict_aggregate(&mut world, root, &vehicle_map)?;
    world.observation_state_sequence = ObservationStateSequence::INITIAL;
    world.command_cursor = root.command_cursor();
    world.event_cursor = root.event_cursor();
    world.next_states.clear();
    world.refresh_signals();
    world
        .rebuild_occupancy_index()
        .map_err(SnapshotRestoreError::Occupancy)?;

    Ok(RestoredSnapshot {
        world,
        routes: route_map.into_iter().collect(),
        vehicles: vehicle_map.into_iter().collect(),
    })
}

fn decode_conflict_locator(
    world: &TrafficWorld,
    binding: wire::ConflictPassageLocatorBinding<'_>,
) -> Result<(crate::ConflictPassageLocator, crate::ConflictPassageAddress), ()> {
    let stream_stable = binding.participant_stream().ok_or(())?;
    let zone_stable = binding.conflict_zone().ok_or(())?;
    let stream_id = ParticipantStreamId::from_untyped(StableId128::from_bytes(stream_stable.0));
    let zone_id = ConflictZoneId::from_untyped(StableId128::from_bytes(zone_stable.0));
    let identity = world.revision.identity();
    let stream = identity.ordinal(stream_id).ok_or(())?;
    let zone = identity.ordinal(zone_id).ok_or(())?;
    let address = world
        .conflict_arbiter
        .unique_address(zone, stream)
        .ok_or(())?;
    let locator = crate::ConflictPassageLocator::new(stream_id, zone_id);
    (world.conflict_passage_locator(address) == Some(locator))
        .then_some((locator, address))
        .ok_or(())
}

fn route_position_um(
    world: &TrafficWorld,
    route: RouteHandle,
    route_edge_index: u32,
    progress_mm: u32,
    carry_um: u16,
) -> Option<u128> {
    let edges = world.route_edges(route)?;
    let index = usize::try_from(route_edge_index).ok()?;
    let edge = *edges.get(index)?;
    let lengths = world.revision.traffic().lane_lengths_millimetres();
    if progress_mm > *lengths.get(edge.index())? || carry_um >= MICROMETRES_PER_MILLIMETRE {
        return None;
    }
    let prefix_mm = edges[..index].iter().try_fold(0_u128, |sum, edge| {
        sum.checked_add(u128::from(*lengths.get(edge.index())?))
    })?;
    prefix_mm
        .checked_add(u128::from(progress_mm))?
        .checked_mul(u128::from(MICROMETRES_PER_MILLIMETRE))?
        .checked_add(u128::from(carry_um))
}

fn restore_conflict_aggregate(
    world: &mut TrafficWorld,
    root: wire::RuntimeSnapshot<'_>,
    vehicle_map: &BTreeMap<u64, VehicleHandle>,
) -> Result<(), SnapshotRestoreError> {
    let capacity = usize::try_from(world.config.vehicle_capacity())
        .map_err(|_| SnapshotRestoreError::InvalidConflictHistory)?;
    let mut eligibility = Vec::new();
    eligibility
        .try_reserve_exact(capacity)
        .map_err(|_| SnapshotRestoreError::InvalidConflictHistory)?;
    eligibility.resize(capacity, None);
    world.conflict_eligibility = eligibility;

    for vehicle in root.vehicles() {
        let snapshot_vehicle_id = vehicle.snapshot_vehicle_id();
        let handle = *vehicle_map.get(&snapshot_vehicle_id).ok_or(
            SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            },
        )?;
        if vehicle.conflict_eligibility().is_some() && vehicle.conflict_reservation().is_some() {
            return Err(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            });
        }
        if let Some(binding) = vehicle.conflict_eligibility() {
            let state = *world.vehicle_state(handle).ok_or(
                SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                },
            )?;
            if state.status != VehicleStatus::Active
                || world.conflict_reservation(handle).is_some()
                || binding.first_eligible_tick() > root.tick()
            {
                return Err(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                });
            }
            let (stable_locator, _) =
                decode_conflict_locator(world, binding.passage()).map_err(|_| {
                    SnapshotRestoreError::InvalidConflictAuthority {
                        snapshot_vehicle_id,
                    }
                })?;
            let locator = world
                .conflict_passage_occurrence_locator(
                    state.route,
                    binding.conflict_occurrence_index(),
                )
                .filter(|locator| {
                    locator.stable_locator() == stable_locator
                        && locator.maneuver_occurrence_index()
                            == binding.maneuver_occurrence_index()
                })
                .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                })?;
            let compiled = world.compiled_route(state.route).ok_or(
                SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                },
            )?;
            let maneuver = compiled
                .maneuvers
                .get(locator.maneuver_occurrence_index() as usize)
                .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                })?;
            let gate = compiled
                .hop_gate
                .get(locator.admission_gate_hop() as usize)
                .copied()
                .flatten()
                .and_then(|gate| world.revision.identity().stable_id(gate))
                .map(|gate| *gate.as_untyped());
            if maneuver.entry_route_edge_index != binding.maneuver_entry_route_edge_index()
                || gate
                    != binding
                        .admission_gate()
                        .map(|gate| StableId128::from_bytes(gate.0))
            {
                return Err(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                });
            }
            let eligibility = crate::ConflictEligibilityState::update(
                None,
                locator,
                true,
                binding.first_eligible_tick(),
            )
            .expect("true predicate creates eligibility");
            if !world.conflict_eligibility_authority_valid(&state, eligibility) {
                return Err(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                });
            }
            world.conflict_eligibility[handle.index() as usize] = Some(eligibility);
        }

        let Some(binding) = vehicle.conflict_reservation() else {
            continue;
        };
        let state =
            *world
                .vehicle_state(handle)
                .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                })?;
        let traversal =
            vehicle
                .maneuver_traversal()
                .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                })?;
        if state.status != VehicleStatus::Active
            || state.waiting_membership.is_some()
            || binding.acquired_tick() > root.tick()
            || traversal.phase().0 != wire::ManeuverTraversalPhaseKind::Clearing.0
            || traversal.maneuver_occurrence_index() != binding.maneuver_occurrence_index()
            || binding.passages().is_empty()
            || binding.downstream_intervals().is_empty()
        {
            return Err(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            });
        }
        let compiled = world.compiled_route(state.route).ok_or(
            SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            },
        )?;
        let maneuver = compiled
            .maneuvers
            .get(binding.maneuver_occurrence_index() as usize)
            .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        let path_stable = world
            .revision
            .identity()
            .stable_id(maneuver.path)
            .map(|path| *path.as_untyped());
        let admission_gate =
            binding
                .admission_gate()
                .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                })?;
        let admission_gate_stable = StableId128::from_bytes(admission_gate.0);
        let admission_hop = (maneuver.entry_route_edge_index..maneuver.exit_route_edge_index)
            .find(|hop| {
                compiled
                    .hop_gate
                    .get(*hop as usize)
                    .copied()
                    .flatten()
                    .and_then(|gate| world.revision.identity().stable_id(gate))
                    .is_some_and(|gate| *gate.as_untyped() == admission_gate_stable)
            })
            .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        if maneuver.entry_route_edge_index != binding.maneuver_entry_route_edge_index()
            || traversal
                .maneuver_path()
                .map(|path| StableId128::from_bytes(path.0))
                != path_stable
            || traversal
                .phase_gate()
                .map(|gate| StableId128::from_bytes(gate.0))
                != Some(admission_gate_stable)
        {
            return Err(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            });
        }

        let first_occurrence = binding.passages().get(0).conflict_occurrence_index();
        let passage_count = u32::try_from(binding.passages().len()).map_err(|_| {
            SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            }
        })?;
        let gate_range = compiled
            .conflict_gate_ranges
            .get(admission_hop as usize)
            .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        if gate_range.start != first_occurrence || gate_range.len != passage_count {
            return Err(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            });
        }
        let passage_range = crate::ConflictPassageRange::new(
            state.route,
            binding.maneuver_occurrence_index(),
            admission_hop,
            first_occurrence,
            passage_count,
        )
        .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
            snapshot_vehicle_id,
        })?;
        let gate_edge = compiled.edges.get(admission_hop as usize).copied().ok_or(
            SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            },
        )?;
        let gate_progress_mm = world
            .revision
            .traffic()
            .lane_lengths_millimetres()
            .get(gate_edge.index())
            .copied()
            .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        let gate_um = route_position_um(world, state.route, admission_hop, gate_progress_mm, 0)
            .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        let front_um = route_position_um(
            world,
            state.route,
            state.route_edge_index,
            state.progress_mm,
            state.carry_um,
        )
        .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
            snapshot_vehicle_id,
        })?;
        if front_um < gate_um {
            return Err(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            });
        }
        let tail_um = i128::try_from(front_um).map_err(|_| {
            SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            }
        })? - i128::from(state.length_mm) * i128::from(MICROMETRES_PER_MILLIMETRE);
        let mut restored_cells = Vec::new();
        restored_cells
            .try_reserve_exact(binding.passages().len())
            .map_err(|_| SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        for (offset, row) in binding.passages().iter().enumerate() {
            if row.conflict_occurrence_index()
                != first_occurrence
                    .checked_add(u32::try_from(offset).map_err(|_| {
                        SnapshotRestoreError::InvalidConflictAuthority {
                            snapshot_vehicle_id,
                        }
                    })?)
                    .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                        snapshot_vehicle_id,
                    })?
            {
                return Err(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                });
            }
            let (stable_locator, address) =
                decode_conflict_locator(world, row.passage()).map_err(|_| {
                    SnapshotRestoreError::InvalidConflictAuthority {
                        snapshot_vehicle_id,
                    }
                })?;
            world
                .conflict_passage_occurrence_locator(state.route, row.conflict_occurrence_index())
                .filter(|locator| {
                    locator.stable_locator() == stable_locator
                        && locator.maneuver_occurrence_index()
                            == binding.maneuver_occurrence_index()
                        && locator.admission_gate_hop() == admission_hop
                })
                .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                })?;
            let occurrence = compiled
                .conflicts
                .get(row.conflict_occurrence_index() as usize)
                .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                })?;
            if occurrence.entry.route_edge_index != row.entry_route_edge_index()
                || occurrence.entry.progress_mm != row.entry_progress_mm()
                || occurrence.clearance.route_edge_index != row.clearance_route_edge_index()
                || occurrence.clearance.progress_mm != row.clearance_progress_mm()
            {
                return Err(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                });
            }
            let entry_um = route_position_um(
                world,
                state.route,
                occurrence.entry.route_edge_index,
                occurrence.entry.progress_mm,
                0,
            )
            .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
            let clearance_um = route_position_um(
                world,
                state.route,
                occurrence.clearance.route_edge_index,
                occurrence.clearance.progress_mm,
                0,
            )
            .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
            let entered = front_um >= entry_um;
            let cleared = tail_um
                >= i128::try_from(clearance_um).map_err(|_| {
                    SnapshotRestoreError::InvalidConflictAuthority {
                        snapshot_vehicle_id,
                    }
                })?;
            restored_cells.push(crate::conflict::RestoredConflictCell {
                address,
                occupant: entered && !cleared,
                cleared,
            });
        }
        restored_cells.sort_unstable_by_key(|cell| cell.address);
        if restored_cells
            .windows(2)
            .any(|pair| pair[0].address == pair[1].address)
        {
            return Err(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            });
        }

        let mut downstream = Vec::new();
        downstream
            .try_reserve_exact(binding.downstream_intervals().len())
            .map_err(|_| SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        let mut previous_wire_key = None;
        for row in binding.downstream_intervals() {
            let lane_edge =
                row.lane_edge()
                    .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                        snapshot_vehicle_id,
                    })?;
            let stable = StableId128::from_bytes(lane_edge.0);
            let wire_key = (stable, row.start_mm(), row.end_mm());
            if previous_wire_key.is_some_and(|previous| previous >= wire_key) {
                return Err(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                });
            }
            previous_wire_key = Some(wire_key);
            let edge = world
                .revision
                .identity()
                .ordinal(LaneEdgeId::from_untyped(stable))
                .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                })?;
            let interval = crate::DownstreamInterval::new(edge, row.start_mm(), row.end_mm())
                .filter(|interval| {
                    interval.end_mm()
                        <= world.revision.traffic().lane_lengths_millimetres()[edge.index()]
                })
                .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                })?;
            downstream.push(interval);
        }
        downstream.sort_unstable();
        if downstream.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            });
        }
        let mut expected_downstream = Vec::new();
        let downstream_plan = world
            .reservation_downstream_claim_plan(passage_range, state.length_mm)
            .map_err(|_| SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        expected_downstream
            .try_reserve_exact(downstream_plan.raw_interval_capacity())
            .map_err(|_| SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        world
            .derive_reservation_downstream_claims_from_plan(
                downstream_plan,
                &mut expected_downstream,
            )
            .map_err(|_| SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        if downstream != expected_downstream {
            return Err(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            });
        }
        let follower_min_gap_mm = world
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
            .map(|profile| profile.min_gap_mm())
            .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        world
            .conflict_arbiter
            .restore_reservation(
                handle,
                crate::conflict::RestoredConflictReservation {
                    follower_min_gap_mm,
                    acquired_tick: binding.acquired_tick(),
                    passage_range,
                    cells: &restored_cells,
                    downstream: &downstream,
                },
            )
            .map_err(|_| SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        world.vehicles[handle.index() as usize]
            .state
            .as_mut()
            .expect("restored vehicle exists")
            .maneuver_traversal = Some(ManeuverTraversalState {
            route: state.route,
            maneuver_occurrence_index: binding.maneuver_occurrence_index(),
            phase: ManeuverTraversalPhase::Clearing {
                admission_gate_hop: admission_hop,
            },
        });
    }

    let mut previous_locator = None;
    for row in root.conflict_lag_states() {
        let (locator, address) = decode_conflict_locator(world, row.passage())
            .map_err(|_| SnapshotRestoreError::InvalidConflictHistory)?;
        let key = (
            *locator.participant_stream_stable_id().as_untyped(),
            *locator.conflict_zone_stable_id().as_untyped(),
        );
        if previous_locator.is_some_and(|previous| previous >= key) {
            return Err(SnapshotRestoreError::InvalidConflictHistory);
        }
        previous_locator = Some(key);
        let time = row.reference_time_ms();
        if time > root.time_ms() {
            return Err(SnapshotRestoreError::InvalidConflictHistory);
        }
        let reference = if row.reference_kind().0 == wire::ConflictLagReferenceKind::ActualClear.0 {
            crate::ConflictLagReference::ActualClear(time)
        } else if row.reference_kind().0 == wire::ConflictLagReferenceKind::CutoverFloor.0 {
            crate::ConflictLagReference::CutoverFloor(time)
        } else {
            return Err(SnapshotRestoreError::InvalidConflictHistory);
        };
        world
            .conflict_arbiter
            .restore_lag_reference(address, reference)
            .map_err(|_| SnapshotRestoreError::InvalidConflictHistory)?;
    }
    world.normalize_conflict_eligibility();
    if !world.conflict_state_valid() {
        return Err(SnapshotRestoreError::InvalidConflictHistory);
    }
    Ok(())
}

fn restore_waiting_aggregate(
    world: &mut TrafficWorld,
    root: wire::RuntimeSnapshot<'_>,
) -> Result<(), SnapshotRestoreError> {
    let mut rows = vec![None; world.waiting_zones.len()];
    for row in root.waiting_zones() {
        let zone = row
            .waiting_zone()
            .and_then(|stable| {
                world
                    .revision
                    .identity()
                    .ordinal(WaitingZoneId::from_untyped(StableId128::from_bytes(
                        stable.0,
                    )))
            })
            .ok_or(SnapshotRestoreError::InvalidWaitingZoneState)?;
        if (row.occupancy() == 0 && row.next_admission_sequence() == 0)
            || rows[zone.index()]
                .replace((row.occupancy(), row.next_admission_sequence()))
                .is_some()
        {
            return Err(SnapshotRestoreError::InvalidWaitingZoneState);
        }
    }

    let mut members = Vec::new();
    members
        .try_reserve_exact(world.live_order.len())
        .map_err(|_| SnapshotRestoreError::WaitingInvariantViolation)?;
    for vehicle in world.live_order.iter().copied() {
        if let Some(membership) = world
            .vehicle_state(vehicle)
            .and_then(|state| state.waiting_membership)
        {
            members.push((
                membership.waiting_zone.index(),
                membership.admission_sequence,
                vehicle,
                membership,
            ));
        }
    }
    members.sort_by_key(|(zone, sequence, vehicle, _)| {
        (*zone, *sequence, vehicle.index(), vehicle.generation())
    });
    if members
        .windows(2)
        .any(|pair| pair[0].0 == pair[1].0 && pair[0].1 == pair[1].1)
    {
        return Err(SnapshotRestoreError::WaitingInvariantViolation);
    }

    // 成员已经按 zone 排序；单调游标只消费当前组，计数总成本为 O(zones + members)。
    let mut member_cursor = 0;
    for (zone_index, state) in world.waiting_zones.iter_mut().enumerate() {
        let group_start = member_cursor;
        while members
            .get(member_cursor)
            .is_some_and(|member| member.0 == zone_index)
        {
            member_cursor += 1;
        }
        let member_count = member_cursor - group_start;
        let Some((occupancy, next_admission_sequence)) = rows[zone_index] else {
            if member_count != 0 {
                return Err(SnapshotRestoreError::WaitingInvariantViolation);
            }
            continue;
        };
        let zone = WaitingZoneOrdinal::from_raw(
            u32::try_from(zone_index)
                .map_err(|_| SnapshotRestoreError::WaitingInvariantViolation)?,
        );
        let max_occupancy = world
            .revision
            .traffic()
            .relations()
            .waiting_zone(zone)
            .ok_or(SnapshotRestoreError::InvalidWaitingZoneState)?
            .max_occupancy();
        if usize::try_from(occupancy).ok() != Some(member_count)
            || occupancy > max_occupancy
            || (member_count != 0 && next_admission_sequence == 0)
        {
            return Err(SnapshotRestoreError::WaitingInvariantViolation);
        }
        state.next_admission_sequence = next_admission_sequence;
    }

    for (_, sequence, vehicle, membership) in members {
        let next = world.waiting_zones[membership.waiting_zone.index()].next_admission_sequence;
        if sequence >= next {
            return Err(SnapshotRestoreError::WaitingInvariantViolation);
        }
        world.append_waiting_member(vehicle, membership);
    }
    if !world.waiting_state_valid() || !world.waiting_snapshot_storage_valid() {
        return Err(SnapshotRestoreError::WaitingInvariantViolation);
    }
    world.rebuild_waiting_member_rows();
    world
        .prepare_waiting_dependencies(false)
        .map_err(SnapshotRestoreError::WaitingDependencyRebuild)?;
    Ok(())
}

fn verify_lfrs<'a>(
    bytes: &'a [u8],
    limits: SnapshotRestoreLimits,
) -> Result<wire::RuntimeSnapshot<'a>, SnapshotRestoreError> {
    let byte_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if byte_len > limits.max_wire_bytes {
        return Err(limit_error(
            SnapshotLimitDimension::WireBytes,
            limits.max_wire_bytes,
            byte_len,
        ));
    }
    if bytes.len() < MIN_SIZE_PREFIXED_LFRS_BYTES {
        return Err(SnapshotRestoreError::TruncatedFraming);
    }
    let declared = u32::from_le_bytes(
        bytes[..4]
            .try_into()
            .expect("minimum framing includes size prefix"),
    );
    let actual = u64::try_from(bytes.len() - 4).unwrap_or(u64::MAX);
    if u64::from(declared) != actual {
        return Err(SnapshotRestoreError::SizePrefixMismatch {
            declared: u64::from(declared),
            actual,
        });
    }
    if !wire::runtime_snapshot_size_prefixed_buffer_has_identifier(bytes) {
        return Err(SnapshotRestoreError::FileIdentifierMismatch);
    }

    // canonical FlatBuffers 中每个 table object 至少占一个 4-byte soffset；以实际
    // caller-bounded wire 长度给 verifier 线性表预算，既覆盖 Conflict locator/
    // reservation 的可变嵌套表，也会拒绝利用共享子树造成超线性重复访问的输入。
    let max_tables = bytes.len() / std::mem::size_of::<u32>();
    let max_apparent_size = bytes
        .len()
        .checked_mul(APPARENT_SIZE_MULTIPLIER)
        .ok_or_else(|| {
            limit_error(
                SnapshotLimitDimension::VerifierBudget,
                usize::MAX as u64,
                u64::MAX,
            )
        })?;
    let options = VerifierOptions {
        max_depth: MAX_SCHEMA_TABLE_DEPTH,
        max_tables,
        max_apparent_size,
        ignore_missing_null_terminator: false,
    };
    wire::size_prefixed_root_as_runtime_snapshot_with_opts(&options, bytes)
        .map_err(|_| SnapshotRestoreError::InvalidFlatbuffer)
}

fn validate_bindings(
    root: wire::RuntimeSnapshot<'_>,
    revision: &SharedNetworkRevision,
    source: &CommittedNetworkSource,
    target_config: WorldConfig,
    limits: SnapshotRestoreLimits,
) -> Result<(), SnapshotRestoreError> {
    if root.format_version() != SNAPSHOT_FORMAT_VERSION {
        return Err(SnapshotRestoreError::UnsupportedFormatVersion {
            actual: root.format_version(),
        });
    }
    if root.runtime_state_version() != RUNTIME_STATE_VERSION {
        return Err(SnapshotRestoreError::UnsupportedRuntimeStateVersion {
            actual: root.runtime_state_version(),
        });
    }
    validate_closed_v5_tables(root)?;
    let network_revision = root
        .network_revision()
        .ok_or(SnapshotRestoreError::MissingField {
            field: "network_revision",
        })?;
    if network_revision.0 != *revision.network_revision().as_digest().as_bytes() {
        return Err(SnapshotRestoreError::NetworkRevisionMismatch);
    }
    root.lfca_artifact_digest()
        .ok_or(SnapshotRestoreError::MissingField {
            field: "lfca_artifact_digest",
        })?;
    let contracts = root
        .static_contract_versions()
        .ok_or(SnapshotRestoreError::MissingField {
            field: "static_contract_versions",
        })?;
    let target_contracts = revision.canonical_origin().static_contract_versions();
    if contracts.canonical_format_version() != target_contracts.canonical_format_version()
        || contracts.identity_encoding_version() != target_contracts.identity_encoding_version()
        || contracts.identity_registry_revision() != target_contracts.identity_registry_revision()
        || contracts.network_revision_derivation_version()
            != target_contracts.network_revision_derivation_version()
        || contracts.constraint_contract_version() != target_contracts.constraint_contract_version()
        || contracts.static_execution_contract_version()
            != target_contracts.static_execution_contract_version()
    {
        return Err(SnapshotRestoreError::StaticContractVersionsMismatch);
    }
    if source.network_revision() != revision.network_revision() {
        return Err(SnapshotRestoreError::Install(
            InstallError::SourceRevisionMismatch {
                source_revision: source.network_revision(),
                installed_revision: revision.network_revision(),
            },
        ));
    }

    if root.source_kind() != wire::SourceKind::Published {
        return Err(SnapshotRestoreError::UnsupportedSourceKind {
            actual: root.source_kind().0,
        });
    }
    let published = root
        .source_published()
        .ok_or(SnapshotRestoreError::MissingField {
            field: "source_published",
        })?;
    if published.asset_key().is_empty() {
        return Err(SnapshotRestoreError::EmptyAssetKey);
    }
    let asset_key_len = u64::try_from(published.asset_key().len()).unwrap_or(u64::MAX);
    if asset_key_len > limits.max_asset_key_bytes {
        return Err(limit_error(
            SnapshotLimitDimension::AssetKeyBytes,
            limits.max_asset_key_bytes,
            asset_key_len,
        ));
    }
    published
        .artifact_digest()
        .ok_or(SnapshotRestoreError::MissingField {
            field: "source_published.artifact_digest",
        })?;
    let source_revision =
        published
            .network_revision()
            .ok_or(SnapshotRestoreError::MissingField {
                field: "source_published.network_revision",
            })?;
    if source_revision.0 != network_revision.0 {
        return Err(SnapshotRestoreError::SnapshotSourceRevisionMismatch);
    }

    let snapshot_config = root.world_config();
    if snapshot_config.fixed_delta_time_ms() != target_config.fixed_delta_time_ms() {
        return Err(SnapshotRestoreError::FixedDeltaTimeMismatch {
            snapshot: snapshot_config.fixed_delta_time_ms(),
            target: target_config.fixed_delta_time_ms(),
        });
    }
    validate_capacity_not_smaller(
        SnapshotLimitDimension::Vehicles,
        u64::from(snapshot_config.vehicle_capacity()),
        u64::from(target_config.vehicle_capacity()),
    )?;
    validate_capacity_not_smaller(
        SnapshotLimitDimension::Routes,
        u64::from(snapshot_config.route_capacity()),
        u64::from(target_config.route_capacity()),
    )?;
    validate_capacity_not_smaller(
        SnapshotLimitDimension::RouteEdgeOccurrences,
        snapshot_config.route_edge_occurrence_capacity(),
        target_config.route_edge_occurrence_capacity(),
    )?;
    validate_capacity_not_smaller(
        SnapshotLimitDimension::RouteConflictOccurrences,
        snapshot_config.route_conflict_occurrence_capacity(),
        target_config.route_conflict_occurrence_capacity(),
    )?;
    let expected_time = root
        .tick()
        .checked_mul(snapshot_config.fixed_delta_time_ms())
        .ok_or(SnapshotRestoreError::InvalidClock)?;
    if expected_time != root.time_ms() {
        return Err(SnapshotRestoreError::InvalidClock);
    }

    let route_count = u64::try_from(root.routes().len()).unwrap_or(u64::MAX);
    validate_state_count(
        SnapshotLimitDimension::Routes,
        route_count,
        u64::from(snapshot_config.route_capacity()),
        u64::from(target_config.route_capacity()),
    )?;
    let vehicle_count = u64::try_from(root.vehicles().len()).unwrap_or(u64::MAX);
    validate_state_count(
        SnapshotLimitDimension::Vehicles,
        vehicle_count,
        u64::from(snapshot_config.vehicle_capacity()),
        u64::from(target_config.vehicle_capacity()),
    )?;
    if root.live_order().len() != root.vehicles().len() {
        return Err(SnapshotRestoreError::IncompleteLiveOrder);
    }
    let mut occurrence_count = 0_u64;
    for route in root.routes() {
        occurrence_count = occurrence_count
            .checked_add(u64::try_from(route.edges().len()).unwrap_or(u64::MAX))
            .ok_or_else(|| {
                limit_error(
                    SnapshotLimitDimension::RouteEdgeOccurrences,
                    snapshot_config.route_edge_occurrence_capacity(),
                    u64::MAX,
                )
            })?;
    }
    validate_state_count(
        SnapshotLimitDimension::RouteEdgeOccurrences,
        occurrence_count,
        snapshot_config.route_edge_occurrence_capacity(),
        target_config.route_edge_occurrence_capacity(),
    )?;
    Ok(())
}

fn decode_world_policy(
    binding: wire::WorldPolicyBinding<'_>,
) -> Result<crate::WorldPolicySelection, SnapshotRestoreError> {
    match (binding.selection(), binding.policy()) {
        (wire::WorldPolicySelectionKind::NotRequired, None) => {
            Ok(crate::WorldPolicySelection::NotRequired)
        }
        (wire::WorldPolicySelectionKind::Pinned, Some(id)) => {
            Ok(crate::WorldPolicySelection::Pinned(crate::PolicyPin {
                policy: laneflow_static_contract::RightOfWayPolicySetId::from_untyped(
                    StableId128::from_bytes(id.0),
                ),
            }))
        }
        _ => Err(SnapshotRestoreError::InvalidPolicyBinding),
    }
}

fn validate_closed_v5_tables(root: wire::RuntimeSnapshot<'_>) -> Result<(), SnapshotRestoreError> {
    validate_table_field_count("RuntimeSnapshot", root._tab, ROOT_V5_FIELDS)?;
    validate_table_field_count(
        "WorldPolicyBinding",
        root.world_policy()._tab,
        vtable_field_count(wire::WorldPolicyBinding::VT_POLICY),
    )?;
    validate_table_field_count(
        "WorldConfigBinding",
        root.world_config()._tab,
        WORLD_CONFIG_V5_FIELDS,
    )?;
    if let Some(published) = root.source_published() {
        validate_table_field_count(
            "PublishedSourceBinding",
            published._tab,
            PUBLISHED_SOURCE_V5_FIELDS,
        )?;
    }
    for route in root.routes() {
        validate_table_field_count("SnapshotRoute", route._tab, ROUTE_V5_FIELDS)?;
    }
    for vehicle in root.vehicles() {
        validate_table_field_count("SnapshotVehicle", vehicle._tab, VEHICLE_V5_FIELDS)?;
        if let Some(parking) = vehicle.parking() {
            validate_table_field_count("ParkingBinding", parking._tab, PARKING_BINDING_V5_FIELDS)?;
        }
        if let Some(traversal) = vehicle.maneuver_traversal() {
            validate_table_field_count(
                "ManeuverTraversalBinding",
                traversal._tab,
                MANEUVER_TRAVERSAL_V5_FIELDS,
            )?;
        }
        if let Some(membership) = vehicle.waiting_membership() {
            validate_table_field_count(
                "WaitingMembershipBinding",
                membership._tab,
                WAITING_MEMBERSHIP_V5_FIELDS,
            )?;
        }
        if let Some(eligibility) = vehicle.conflict_eligibility() {
            validate_table_field_count(
                "ConflictEligibilityBinding",
                eligibility._tab,
                CONFLICT_ELIGIBILITY_V5_FIELDS,
            )?;
            validate_conflict_locator_table(eligibility.passage())?;
        }
        if let Some(reservation) = vehicle.conflict_reservation() {
            validate_table_field_count(
                "ConflictReservationBinding",
                reservation._tab,
                CONFLICT_RESERVATION_V5_FIELDS,
            )?;
            for passage in reservation.passages() {
                validate_table_field_count(
                    "ConflictPassageBinding",
                    passage._tab,
                    CONFLICT_PASSAGE_V5_FIELDS,
                )?;
                validate_conflict_locator_table(passage.passage())?;
            }
            for downstream in reservation.downstream_intervals() {
                validate_table_field_count(
                    "ConflictDownstreamIntervalBinding",
                    downstream._tab,
                    CONFLICT_DOWNSTREAM_V5_FIELDS,
                )?;
            }
        }
    }
    for state in root.waiting_zones() {
        validate_table_field_count("WaitingZoneState", state._tab, WAITING_ZONE_STATE_V5_FIELDS)?;
    }
    for state in root.conflict_lag_states() {
        validate_table_field_count("ConflictLagState", state._tab, CONFLICT_LAG_STATE_V5_FIELDS)?;
        validate_conflict_locator_table(state.passage())?;
    }
    Ok(())
}

fn validate_conflict_locator_table(
    locator: wire::ConflictPassageLocatorBinding<'_>,
) -> Result<(), SnapshotRestoreError> {
    validate_table_field_count(
        "ConflictPassageLocatorBinding",
        locator._tab,
        CONFLICT_LOCATOR_V5_FIELDS,
    )
}

fn validate_table_field_count(
    table_name: &'static str,
    table: laneflow_runtime_snapshot_wire::runtime::Table<'_>,
    supported: usize,
) -> Result<(), SnapshotRestoreError> {
    let actual = table.vtable().num_fields();
    if actual > supported {
        return Err(SnapshotRestoreError::UnknownTableFields {
            table: table_name,
            supported,
            actual,
        });
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum DecodedParkingBinding {
    Reserved(ReserveParkingTarget),
    Occupied(ParkingTarget),
}

#[derive(Clone, Copy, Default)]
struct DecodedWaitingAuthority {
    traversal: Option<ManeuverTraversalState>,
    membership: Option<WaitingMembership>,
}

fn decode_waiting_authority(
    world: &TrafficWorld,
    vehicle: wire::SnapshotVehicle<'_>,
    status: VehicleStatus,
    route: RouteHandle,
) -> Result<DecodedWaitingAuthority, SnapshotRestoreError> {
    let snapshot_vehicle_id = vehicle.snapshot_vehicle_id();
    if status != VehicleStatus::Active {
        return if vehicle.maneuver_traversal().is_none()
            && vehicle.waiting_membership().is_none()
            && vehicle.conflict_eligibility().is_none()
            && vehicle.conflict_reservation().is_none()
        {
            Ok(DecodedWaitingAuthority::default())
        } else {
            Err(SnapshotRestoreError::InvalidWaitingAuthority {
                snapshot_vehicle_id,
            })
        };
    }

    let Some(binding) = vehicle.maneuver_traversal() else {
        return if vehicle.waiting_membership().is_none() && vehicle.conflict_reservation().is_none()
        {
            Ok(DecodedWaitingAuthority::default())
        } else {
            Err(SnapshotRestoreError::InvalidWaitingAuthority {
                snapshot_vehicle_id,
            })
        };
    };
    let clearing = binding.phase().0 == wire::ManeuverTraversalPhaseKind::Clearing.0;
    if clearing
        && (vehicle.waiting_membership().is_some() || vehicle.conflict_reservation().is_none())
    {
        return Err(SnapshotRestoreError::InvalidConflictAuthority {
            snapshot_vehicle_id,
        });
    }
    if !clearing && vehicle.conflict_reservation().is_some() {
        return Err(SnapshotRestoreError::InvalidConflictAuthority {
            snapshot_vehicle_id,
        });
    }
    let compiled =
        world
            .compiled_route(route)
            .ok_or(SnapshotRestoreError::InvalidWaitingAuthority {
                snapshot_vehicle_id,
            })?;
    let path = binding
        .maneuver_path()
        .and_then(|stable| {
            world
                .revision
                .identity()
                .ordinal(ManeuverPathId::from_untyped(StableId128::from_bytes(
                    stable.0,
                )))
        })
        .ok_or(SnapshotRestoreError::InvalidWaitingAuthority {
            snapshot_vehicle_id,
        })?;
    let phase_gate = binding
        .phase_gate()
        .and_then(|stable| {
            world
                .revision
                .identity()
                .ordinal(ManeuverGateId::from_untyped(StableId128::from_bytes(
                    stable.0,
                )))
        })
        .ok_or(SnapshotRestoreError::InvalidWaitingAuthority {
            snapshot_vehicle_id,
        })?;
    let anchor = world
        .resolve_maneuver_anchor(
            route,
            crate::world::ManeuverOccurrenceAnchor::OccurrenceIndex(
                binding.maneuver_occurrence_index(),
            ),
            path,
            phase_gate,
        )
        .ok_or(SnapshotRestoreError::InvalidWaitingAuthority {
            snapshot_vehicle_id,
        })?;
    let phase_hop = anchor.gate_hop;
    let phase = if binding.phase().0 == wire::ManeuverTraversalPhaseKind::PreGate.0 {
        ManeuverTraversalPhase::PreGate {
            next_gate_hop: phase_hop,
        }
    } else if binding.phase().0 == wire::ManeuverTraversalPhaseKind::Committed.0 {
        ManeuverTraversalPhase::Committed {
            last_crossed_gate_hop: phase_hop,
        }
    } else if binding.phase().0 == wire::ManeuverTraversalPhaseKind::Waiting.0 {
        ManeuverTraversalPhase::Waiting {
            release_gate_hop: phase_hop,
        }
    } else if clearing {
        ManeuverTraversalPhase::Clearing {
            admission_gate_hop: phase_hop,
        }
    } else {
        return Err(SnapshotRestoreError::InvalidWaitingAuthority {
            snapshot_vehicle_id,
        });
    };
    let traversal = ManeuverTraversalState {
        route,
        maneuver_occurrence_index: anchor.occurrence_index,
        phase,
    };

    let membership = match vehicle.waiting_membership() {
        None => None,
        Some(binding) => {
            if binding.maneuver_occurrence_index() != traversal.maneuver_occurrence_index {
                return Err(SnapshotRestoreError::InvalidWaitingAuthority {
                    snapshot_vehicle_id,
                });
            }
            let zone = binding
                .waiting_zone()
                .and_then(|stable| {
                    world
                        .revision
                        .identity()
                        .ordinal(WaitingZoneId::from_untyped(StableId128::from_bytes(
                            stable.0,
                        )))
                })
                .ok_or(SnapshotRestoreError::InvalidWaitingAuthority {
                    snapshot_vehicle_id,
                })?;
            let entry_gate = binding
                .entry_gate()
                .and_then(|stable| {
                    world
                        .revision
                        .identity()
                        .ordinal(ManeuverGateId::from_untyped(StableId128::from_bytes(
                            stable.0,
                        )))
                })
                .ok_or(SnapshotRestoreError::InvalidWaitingAuthority {
                    snapshot_vehicle_id,
                })?;
            let release_gate = binding
                .release_gate()
                .and_then(|stable| {
                    world
                        .revision
                        .identity()
                        .ordinal(ManeuverGateId::from_untyped(StableId128::from_bytes(
                            stable.0,
                        )))
                })
                .ok_or(SnapshotRestoreError::InvalidWaitingAuthority {
                    snapshot_vehicle_id,
                })?;
            let occurrence = compiled
                .waiting
                .iter()
                .find(|occurrence| {
                    occurrence.maneuver_index == traversal.maneuver_occurrence_index
                        && occurrence.zone == zone
                        && compiled
                            .hop_gate
                            .get(occurrence.entry_hop as usize)
                            .copied()
                            .flatten()
                            == Some(entry_gate)
                        && compiled
                            .hop_gate
                            .get(occurrence.release_hop as usize)
                            .copied()
                            .flatten()
                            == Some(release_gate)
                })
                .ok_or(SnapshotRestoreError::InvalidWaitingAuthority {
                    snapshot_vehicle_id,
                })?;
            Some(WaitingMembership {
                waiting_zone: zone,
                admission_sequence: binding.admission_sequence(),
                release_hop: occurrence.release_hop,
            })
        }
    };
    Ok(DecodedWaitingAuthority {
        traversal: Some(traversal),
        membership,
    })
}

fn decode_parking_binding(
    world: &TrafficWorld,
    vehicle: wire::SnapshotVehicle<'_>,
    status: VehicleStatus,
) -> Result<Option<DecodedParkingBinding>, SnapshotRestoreError> {
    let snapshot_vehicle_id = vehicle.snapshot_vehicle_id();
    let Some(binding) = vehicle.parking() else {
        return if status == VehicleStatus::Parked {
            Err(SnapshotRestoreError::ParkingStatusMismatch {
                snapshot_vehicle_id,
            })
        } else {
            Ok(None)
        };
    };
    let state = if binding.state() == wire::ParkingBindingStateKind::Reserved {
        wire::ParkingBindingStateKind::Reserved
    } else if binding.state() == wire::ParkingBindingStateKind::Occupied {
        wire::ParkingBindingStateKind::Occupied
    } else {
        return Err(SnapshotRestoreError::InvalidParkingBindingState {
            snapshot_vehicle_id,
            actual: binding.state().0,
        });
    };
    let target_wire = binding.target().ok_or(SnapshotRestoreError::MissingField {
        field: "vehicles.parking.target",
    })?;
    let identity = world.revision.identity();
    let target = if binding.target_kind() == wire::ParkingTargetKind::ExplicitSpace {
        ParkingTarget::ExplicitSpace(
            identity
                .ordinal(ParkingSpaceId::from_untyped(StableId128::from_bytes(
                    target_wire.0,
                )))
                .ok_or(SnapshotRestoreError::UnknownParkingSpace {
                    snapshot_vehicle_id,
                })?,
        )
    } else if binding.target_kind() == wire::ParkingTargetKind::VirtualPool {
        ParkingTarget::VirtualPool(
            identity
                .ordinal(ParkingFacilityId::from_untyped(StableId128::from_bytes(
                    target_wire.0,
                )))
                .ok_or(SnapshotRestoreError::UnknownParkingFacility {
                    snapshot_vehicle_id,
                })?,
        )
    } else {
        return Err(SnapshotRestoreError::InvalidParkingTargetKind {
            snapshot_vehicle_id,
            actual: binding.target_kind().0,
        });
    };

    match (state, target, status) {
        (
            wire::ParkingBindingStateKind::Reserved,
            ParkingTarget::ExplicitSpace(space),
            VehicleStatus::Active,
        ) => {
            if binding.virtual_entry_edge().is_some() || binding.virtual_entry_progress_mm() != 0 {
                return Err(SnapshotRestoreError::InvalidParkingBindingShape {
                    snapshot_vehicle_id,
                });
            }
            Ok(Some(DecodedParkingBinding::Reserved(
                ReserveParkingTarget::ExplicitSpace {
                    space,
                    entry_route_occurrence: binding.entry_route_occurrence(),
                },
            )))
        }
        (
            wire::ParkingBindingStateKind::Reserved,
            ParkingTarget::VirtualPool(facility),
            VehicleStatus::Active,
        ) => {
            let entry_wire = binding.virtual_entry_edge().ok_or(
                SnapshotRestoreError::InvalidParkingBindingShape {
                    snapshot_vehicle_id,
                },
            )?;
            let entry_edge = identity
                .ordinal(LaneEdgeId::from_untyped(StableId128::from_bytes(
                    entry_wire.0,
                )))
                .ok_or(SnapshotRestoreError::UnknownVirtualParkingEntry {
                    snapshot_vehicle_id,
                })?;
            let view = world
                .revision
                .traffic()
                .relations()
                .parking_facility(facility)
                .ok_or(SnapshotRestoreError::UnknownParkingFacility {
                    snapshot_vehicle_id,
                })?;
            let selector = view
                .virtual_entries()
                .iter()
                .position(|anchor| {
                    anchor.lane_edge() == entry_edge
                        && anchor.progress_mm() == binding.virtual_entry_progress_mm()
                })
                .ok_or(SnapshotRestoreError::UnknownVirtualParkingEntry {
                    snapshot_vehicle_id,
                })?;
            Ok(Some(DecodedParkingBinding::Reserved(
                ReserveParkingTarget::VirtualPool {
                    facility,
                    entry_anchor: VirtualEntryAnchorSelector::from_raw(
                        u32::try_from(selector).expect("virtual entry selector fits u32"),
                    ),
                    entry_route_occurrence: binding.entry_route_occurrence(),
                },
            )))
        }
        (wire::ParkingBindingStateKind::Occupied, target, VehicleStatus::Parked) => {
            if binding.entry_route_occurrence() != 0
                || binding.virtual_entry_edge().is_some()
                || binding.virtual_entry_progress_mm() != 0
            {
                return Err(SnapshotRestoreError::InvalidParkingBindingShape {
                    snapshot_vehicle_id,
                });
            }
            Ok(Some(DecodedParkingBinding::Occupied(target)))
        }
        _ => Err(SnapshotRestoreError::ParkingStatusMismatch {
            snapshot_vehicle_id,
        }),
    }
}

fn restore_vehicle(
    world: &mut TrafficWorld,
    vehicle: wire::SnapshotVehicle<'_>,
    status: VehicleStatus,
    route_map: &BTreeMap<u64, RouteHandle>,
    vehicle_map: &mut BTreeMap<u64, VehicleHandle>,
) -> Result<(), SnapshotRestoreError> {
    let snapshot_vehicle_id = vehicle.snapshot_vehicle_id();
    if snapshot_vehicle_id == 0 {
        return Err(SnapshotRestoreError::ZeroVehicleId);
    }
    if vehicle_map.contains_key(&snapshot_vehicle_id) {
        return Err(SnapshotRestoreError::DuplicateVehicleId {
            snapshot_vehicle_id,
        });
    }
    let snapshot_route_id = vehicle.snapshot_route_id();
    let Some(route) = route_map.get(&snapshot_route_id).copied() else {
        return Err(SnapshotRestoreError::UnknownRouteReference {
            snapshot_vehicle_id,
            snapshot_route_id,
        });
    };
    if vehicle.carry_um() >= MICROMETRES_PER_MILLIMETRE {
        return Err(SnapshotRestoreError::CarryOutOfRange {
            snapshot_vehicle_id,
            actual: vehicle.carry_um(),
        });
    }
    let parking = decode_parking_binding(world, vehicle, status)?;
    if status != VehicleStatus::Active && (vehicle.speed_mm_s() != 0 || vehicle.carry_um() != 0) {
        return Err(SnapshotRestoreError::InvalidInactiveMotion {
            snapshot_vehicle_id,
        });
    }

    let profile = vehicle
        .profile()
        .ok_or(SnapshotRestoreError::MissingField {
            field: "vehicles.profile",
        })?;
    let class = vehicle.class().ok_or(SnapshotRestoreError::MissingField {
        field: "vehicles.class",
    })?;
    let identity = world.revision.identity();
    let profile = identity
        .ordinal(VehicleProfileId::from_untyped(StableId128::from_bytes(
            profile.0,
        )))
        .ok_or(SnapshotRestoreError::UnknownVehicleProfile {
            snapshot_vehicle_id,
        })?;
    let class = identity
        .ordinal(ParticipantClassId::from_untyped(StableId128::from_bytes(
            class.0,
        )))
        .ok_or(SnapshotRestoreError::UnknownParticipantClass {
            snapshot_vehicle_id,
        })?;
    let profile_view = world
        .revision
        .traffic()
        .relations()
        .vehicle_profile(profile)
        .expect("identity ordinal resolves profile row");
    if profile_view.class() != class {
        return Err(SnapshotRestoreError::ProfileClassMismatch {
            snapshot_vehicle_id,
        });
    }
    let waiting = decode_waiting_authority(world, vehicle, status, route)?;

    if status == VehicleStatus::Completed {
        let edges = world
            .route_edges(route)
            .expect("restored route handle remains live");
        let index = usize::try_from(vehicle.route_edge_index()).unwrap_or(usize::MAX);
        let at_route_end = edges.get(index).is_some_and(|edge| {
            index + 1 == edges.len()
                && vehicle.progress_mm()
                    == world.revision.traffic().lane_lengths_millimetres()[edge.index()]
        });
        if !at_route_end {
            return Err(SnapshotRestoreError::InvalidCompletedState {
                snapshot_vehicle_id,
            });
        }
    }

    let handle = if let Some(DecodedParkingBinding::Occupied(target)) = parking {
        world
            .spawn_parked_vehicle(
                ParkedVehicleSpawnInput::new(
                    profile,
                    route,
                    vehicle.route_edge_index(),
                    vehicle.progress_mm(),
                ),
                target,
            )
            .map_err(|error| SnapshotRestoreError::Parking {
                snapshot_vehicle_id,
                error,
            })?
            .vehicle
    } else {
        world
            .restore_unparked_vehicle(
                VehicleSpawnInput::new(
                    profile,
                    route,
                    vehicle.route_edge_index(),
                    vehicle.progress_mm(),
                    vehicle.speed_mm_s(),
                ),
                vehicle.carry_um(),
                status,
                waiting.traversal,
                waiting.membership,
                vehicle.conflict_eligibility().is_some()
                    || vehicle.conflict_reservation().is_some(),
            )
            .map_err(|error| SnapshotRestoreError::Vehicle {
                snapshot_vehicle_id,
                error,
            })?
    };
    match status {
        VehicleStatus::Active => {
            if let Some(DecodedParkingBinding::Reserved(target)) = parking {
                world.reserve_parking(handle, target).map_err(|error| {
                    SnapshotRestoreError::Parking {
                        snapshot_vehicle_id,
                        error,
                    }
                })?;
            }
        }
        VehicleStatus::Parked => {}
        VehicleStatus::Completed => {}
    }
    if !world
        .vehicle_state(handle)
        .copied()
        .is_some_and(|state| world.restored_waiting_authority_valid(state))
    {
        return Err(SnapshotRestoreError::InvalidWaitingAuthority {
            snapshot_vehicle_id,
        });
    }
    vehicle_map.insert(snapshot_vehicle_id, handle);
    Ok(())
}

const fn decode_vehicle_status(
    snapshot_vehicle_id: u64,
    status: wire::VehicleStatusKind,
) -> Result<VehicleStatus, SnapshotRestoreError> {
    if status.0 == wire::VehicleStatusKind::Active.0 {
        Ok(VehicleStatus::Active)
    } else if status.0 == wire::VehicleStatusKind::Parked.0 {
        Ok(VehicleStatus::Parked)
    } else if status.0 == wire::VehicleStatusKind::Completed.0 {
        Ok(VehicleStatus::Completed)
    } else {
        Err(SnapshotRestoreError::InvalidVehicleStatus {
            snapshot_vehicle_id,
            actual: status.0,
        })
    }
}

const fn validate_capacity_not_smaller(
    dimension: SnapshotLimitDimension,
    snapshot: u64,
    target: u64,
) -> Result<(), SnapshotRestoreError> {
    if target < snapshot {
        return Err(SnapshotRestoreError::TargetCapacitySmaller {
            dimension,
            snapshot,
            target,
        });
    }
    Ok(())
}

const fn validate_state_count(
    dimension: SnapshotLimitDimension,
    actual: u64,
    snapshot_limit: u64,
    target_limit: u64,
) -> Result<(), SnapshotRestoreError> {
    if actual > snapshot_limit {
        return Err(limit_error(dimension, snapshot_limit, actual));
    }
    if actual > target_limit {
        return Err(limit_error(dimension, target_limit, actual));
    }
    Ok(())
}

const fn limit_error(
    dimension: SnapshotLimitDimension,
    limit: u64,
    actual: u64,
) -> SnapshotRestoreError {
    SnapshotRestoreError::LimitExceeded {
        dimension,
        limit,
        actual,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::cutover::tests::transaction_tests::{revision, source_for, world_with_vehicle};
    use crate::cutover_migration::tests::virtual_parking_cutover_world;
    use crate::{
        CapturedParkingBinding, CapturedParkingTarget, CapturedVirtualParkingEntry,
        ParkedVehicleSpawnInput, ParkingBinding, ParkingTarget, RouteRegisterInput, TickInput,
        VehicleState, encode_lfrs,
    };

    fn generous_limits() -> SnapshotRestoreLimits {
        SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024)
    }

    fn conflict_world_with_route() -> (TrafficWorld, RouteHandle) {
        conflict_world_with_route_config(WorldConfig::new(8, 4, 1_024, 1_024, 1, 100))
    }

    fn conflict_world_with_route_config(config: WorldConfig) -> (TrafficWorld, RouteHandle) {
        let revision = revision(true);
        let origin = *revision.canonical_origin();
        let mut world = TrafficWorld::install(
            Arc::clone(&revision),
            config,
            source_for(origin, "fixture://conflict-snapshot"),
            77,
            crate::test_policy::selection(&revision),
        )
        .expect("install conflict world");
        let stream = laneflow_static_contract::ParticipantStreamOrdinal::from_raw(0);
        let path = revision
            .conflict()
            .participant_stream(stream)
            .expect("fixture stream")
            .maneuver_path();
        let route_edges = revision
            .traffic()
            .maneuvers()
            .maneuver_path(path)
            .expect("fixture path")
            .edges()
            .to_vec();
        let route = world
            .register_route(RouteRegisterInput::new(route_edges))
            .expect("conflict route");
        (world, route)
    }

    pub(crate) fn world_with_conflict_reservation() -> (TrafficWorld, VehicleHandle) {
        world_with_conflict_reservation_config(WorldConfig::new(8, 4, 1_024, 1_024, 1, 100))
    }

    fn world_with_conflict_reservation_config(
        config: WorldConfig,
    ) -> (TrafficWorld, VehicleHandle) {
        let (mut world, route) = conflict_world_with_route_config(config);
        let vehicle = world
            .restore_unparked_vehicle(
                VehicleSpawnInput::new(
                    laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    0,
                    0,
                ),
                0,
                VehicleStatus::Active,
                None,
                None,
                true,
            )
            .expect("upstream vehicle");
        install_conflict_reservation(&mut world, route, vehicle);
        (world, vehicle)
    }

    pub(crate) fn install_conflict_reservation(
        world: &mut TrafficWorld,
        route: RouteHandle,
        vehicle: VehicleHandle,
    ) {
        let (gate_range, first_occurrence) = {
            let compiled = world.compiled_route(route).expect("compiled route");
            let first_occurrence = *compiled.conflicts.first().expect("conflict occurrence");
            let gate_range = compiled.conflict_gate_ranges[first_occurrence.admission_hop as usize];
            (gate_range, first_occurrence)
        };
        {
            let state = world.vehicles[vehicle.index() as usize]
                .state
                .as_mut()
                .expect("vehicle state");
            state.route_edge_index = first_occurrence.entry.route_edge_index;
            state.progress_mm = first_occurrence.entry.progress_mm;
            state.carry_um = 0;
            state.speed_mm_s = 0;
            state.waiting_membership = None;
        }
        let front_um = route_position_um(
            world,
            route,
            first_occurrence.entry.route_edge_index,
            first_occurrence.entry.progress_mm,
            0,
        )
        .expect("front position");
        let length_mm = world.vehicle_state(vehicle).expect("vehicle").length_mm;
        let tail_um = i128::try_from(front_um).expect("front fits i128")
            - i128::from(length_mm) * i128::from(MICROMETRES_PER_MILLIMETRE);
        let mut cells = Vec::new();
        let range_end = gate_range.start + gate_range.len;
        for index in gate_range.start..range_end {
            let occurrence = world
                .compiled_route(route)
                .expect("compiled route")
                .conflicts[index as usize];
            let entry_um = route_position_um(
                world,
                route,
                occurrence.entry.route_edge_index,
                occurrence.entry.progress_mm,
                0,
            )
            .expect("entry");
            let clearance_um = route_position_um(
                world,
                route,
                occurrence.clearance.route_edge_index,
                occurrence.clearance.progress_mm,
                0,
            )
            .expect("clearance");
            let cleared = tail_um >= i128::try_from(clearance_um).expect("clearance fits i128");
            cells.push(crate::conflict::RestoredConflictCell {
                address: occurrence.address(),
                occupant: front_um >= entry_um && !cleared,
                cleared,
            });
        }
        cells.sort_unstable_by_key(|cell| cell.address);
        let passage_range = crate::ConflictPassageRange::new(
            route,
            first_occurrence.maneuver_index,
            first_occurrence.admission_hop,
            gate_range.start,
            gate_range.len,
        )
        .expect("passage range");
        let mut downstream = Vec::new();
        world
            .derive_reservation_downstream_claims(passage_range, length_mm, &mut downstream)
            .expect("derive downstream physical union");
        assert!(!downstream.is_empty());
        let follower_min_gap_mm = world
            .revision
            .traffic()
            .relations()
            .vehicle_profile(world.vehicle_state(vehicle).expect("vehicle").profile)
            .expect("vehicle profile")
            .min_gap_mm();
        let reservation = world
            .conflict_arbiter
            .restore_reservation(
                vehicle,
                crate::conflict::RestoredConflictReservation {
                    follower_min_gap_mm,
                    acquired_tick: 0,
                    passage_range,
                    cells: &cells,
                    downstream: &downstream,
                },
            )
            .expect("restore test reservation");
        world.vehicles[vehicle.index() as usize]
            .state
            .as_mut()
            .expect("vehicle state")
            .maneuver_traversal = Some(ManeuverTraversalState {
            route,
            maneuver_occurrence_index: first_occurrence.maneuver_index,
            phase: ManeuverTraversalPhase::Clearing {
                admission_gate_hop: reservation.admission_gate_hop(),
            },
        });
        world
            .conflict_arbiter
            .restore_lag_reference(
                cells[0].address,
                crate::ConflictLagReference::ActualClear(0),
            )
            .expect("tick-zero history");
        assert!(world.conflict_state_valid());
    }

    pub(crate) fn world_with_conflict_eligibility() -> (TrafficWorld, VehicleHandle) {
        let (mut world, route) = conflict_world_with_route();
        let locator = world
            .conflict_passage_occurrence_locator(route, 0)
            .expect("first conflict occurrence");
        let (gate_hop, gate_progress) = {
            let compiled = world.compiled_route(route).expect("compiled route");
            let hop = locator.admission_gate_hop();
            let edge = compiled.edges[hop as usize];
            (
                hop,
                world.revision.traffic().lane_lengths_millimetres()[edge.index()],
            )
        };
        let vehicle = world
            .restore_unparked_vehicle(
                VehicleSpawnInput::new(
                    laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                    route,
                    gate_hop,
                    gate_progress,
                    0,
                ),
                0,
                VehicleStatus::Active,
                None,
                None,
                true,
            )
            .expect("restore vehicle before conflict Gate");
        world.conflict_eligibility.resize(
            usize::try_from(world.config.vehicle_capacity()).expect("vehicle capacity"),
            None,
        );
        world.conflict_eligibility[vehicle.index() as usize] =
            crate::ConflictEligibilityState::update(None, locator, true, 0);
        assert!(world.conflict_state_valid());
        (world, vehicle)
    }

    #[test]
    fn restored_conflict_authority_continues_through_the_production_tick() {
        for (mut world, label) in [
            (world_with_conflict_reservation().0, "reservation"),
            (world_with_conflict_eligibility().0, "eligibility"),
        ] {
            let tick_before = world.tick_index();
            world
                .step(crate::TickInput::new(100))
                .unwrap_or_else(|error| panic!("{label} production continuation: {error:?}"));
            assert_eq!(world.tick_index(), tick_before + 1, "{label}");
            assert!(world.conflict_state_valid(), "{label}");
        }
    }

    #[test]
    fn conflict_eligibility_blocks_route_rebind_without_partial_commit() {
        let (mut world, vehicle) = world_with_conflict_eligibility();
        let state = *world.vehicle_state(vehicle).expect("eligible vehicle");
        let before = world.capture_snapshot().expect("capture before rebind");
        assert!(matches!(
            world.rebind_parking_route(
                vehicle,
                crate::RebindParkingTarget::ExplicitSpace {
                    space: laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0),
                    new_route: state.route,
                    new_current_route_occurrence: state.route_edge_index,
                    new_entry_route_occurrence: state.route_edge_index,
                },
            ),
            Err(crate::ParkingError::ConflictTraversalActive)
        ));
        assert_eq!(
            world.capture_snapshot().expect("capture after rebind"),
            before,
            "eligibility-protected rebind must remain atomic"
        );
    }

    #[test]
    fn conflict_reservation_and_tick_zero_history_round_trip() {
        let (world, _) = world_with_conflict_reservation();
        let captured = world.capture_snapshot().expect("capture Conflict state");
        assert!(
            captured
                .vehicles
                .iter()
                .any(|row| row.conflict_reservation.is_some())
        );
        assert_eq!(
            captured.conflict_lag_states[0].reference,
            crate::ConflictLagReference::ActualClear(0)
        );
        let bytes = encode_lfrs(&captured);
        let restored = restore_lfrs(
            &bytes,
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            generous_limits(),
        )
        .expect("restore Conflict state");
        let reservation_vehicle_id = captured
            .vehicles
            .iter()
            .find(|row| row.conflict_reservation.is_some())
            .expect("captured reservation owner")
            .snapshot_vehicle_id;
        let restored_handle = restored
            .vehicle_handle(reservation_vehicle_id)
            .expect("restored vehicle map");
        assert!(
            restored
                .world()
                .conflict_reservation(restored_handle)
                .is_some()
        );
        let recaptured = restored.world().capture_snapshot().expect("recapture");
        assert_eq!(captured, recaptured);
        assert_eq!(
            crate::deterministic_state_digest(&captured).expect("source digest"),
            crate::deterministic_state_digest(&recaptured).expect("restored digest")
        );
    }

    #[test]
    fn clearing_marker_is_decoded_before_conflict_aggregate_installation() {
        let (world, vehicle) = world_with_conflict_reservation();
        let state = *world.vehicle_state(vehicle).expect("Clearing vehicle");
        let expected = state.maneuver_traversal.expect("Clearing marker");
        let captured = world.capture_snapshot().expect("capture Conflict state");
        let bytes = encode_lfrs(&captured);
        let root = wire::size_prefixed_root_as_runtime_snapshot(&bytes).expect("verified LFRS");
        let row = root
            .vehicles()
            .iter()
            .find(|row| row.snapshot_vehicle_id() == u64::from(vehicle.index()) + 1)
            .expect("reservation owner row");

        let decoded = decode_waiting_authority(&world, row, VehicleStatus::Active, state.route)
            .expect("Clearing anchor decodes before reservation installation");
        assert_eq!(decoded.traversal, Some(expected));
        assert!(decoded.membership.is_none());
        assert!(world.restored_waiting_authority_valid(VehicleState {
            maneuver_traversal: decoded.traversal,
            waiting_membership: decoded.membership,
            ..state
        }));
    }

    #[test]
    fn conflict_reservation_requires_exact_gate_range_and_crossed_side() {
        let (world, vehicle) = world_with_conflict_reservation();
        let captured = world.capture_snapshot().expect("capture Conflict state");
        let snapshot_vehicle_id = captured.vehicles[vehicle.index() as usize].snapshot_vehicle_id;

        let mut wrong_range = captured.clone();
        let passages = &mut wrong_range.vehicles[vehicle.index() as usize]
            .conflict_reservation
            .as_mut()
            .expect("reservation")
            .passages;
        if passages.len() > 1 {
            passages.pop();
        } else {
            let mut extra = passages[0];
            extra.conflict_occurrence_index += 1;
            passages.push(extra);
        }
        assert_eq!(
            restore_lfrs(
                &encode_lfrs(&wrong_range),
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            }
        );

        let mut upstream = captured;
        let owner = &mut upstream.vehicles[vehicle.index() as usize];
        owner.route_edge_index = 0;
        owner.progress_mm = 0;
        owner.carry_um = 0;
        assert_eq!(
            restore_lfrs(
                &encode_lfrs(&upstream),
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            }
        );
    }

    #[test]
    fn conflict_downstream_union_is_rederived_from_reservation_proof() {
        let (world, _) = world_with_conflict_reservation();
        let captured = world.capture_snapshot().expect("capture Conflict state");
        let owner = captured
            .vehicles
            .iter()
            .find(|vehicle| vehicle.conflict_reservation.is_some())
            .expect("reservation owner");
        let snapshot_vehicle_id = owner.snapshot_vehicle_id;
        let mut changed = captured.clone();
        let interval = changed
            .vehicles
            .iter_mut()
            .find(|vehicle| vehicle.snapshot_vehicle_id == snapshot_vehicle_id)
            .and_then(|vehicle| vehicle.conflict_reservation.as_mut())
            .and_then(|reservation| reservation.downstream_intervals.first_mut())
            .expect("downstream interval");
        assert!(interval.end_mm - interval.start_mm > 1);
        interval.start_mm += 1;
        assert_eq!(
            restore_lfrs(
                &encode_lfrs(&changed),
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            }
        );

        let mut missing = captured;
        missing
            .vehicles
            .iter_mut()
            .find(|vehicle| vehicle.snapshot_vehicle_id == snapshot_vehicle_id)
            .and_then(|vehicle| vehicle.conflict_reservation.as_mut())
            .expect("reservation")
            .downstream_intervals
            .pop();
        assert_eq!(
            restore_lfrs(
                &encode_lfrs(&missing),
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            }
        );
    }

    #[test]
    fn pending_conflict_authority_does_not_hide_an_invalid_endpoint_cursor() {
        let (world, vehicle) = world_with_conflict_reservation();
        let mut captured = world.capture_snapshot().expect("capture Conflict state");
        let state = world.vehicle_state(vehicle).expect("vehicle");
        let edge = world.route_edges(state.route).expect("route")
            [usize::try_from(state.route_edge_index).expect("route index")];
        let owner = &mut captured.vehicles[vehicle.index() as usize];
        owner.progress_mm = world.revision.traffic().lane_lengths_millimetres()[edge.index()];
        owner.carry_um = 1;
        let snapshot_vehicle_id = owner.snapshot_vehicle_id;
        assert_eq!(
            restore_lfrs(
                &encode_lfrs(&captured),
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::Vehicle {
                snapshot_vehicle_id,
                error: crate::SpawnError::InvalidProgress,
            }
        );
    }

    #[test]
    fn conflict_nested_tables_fit_exact_small_world_verifier_budget() {
        let config = WorldConfig::new(1, 1, 64, 64, 1, 100);
        let (world, _) = world_with_conflict_reservation_config(config);
        let captured = world.capture_snapshot().expect("capture Conflict state");
        let restored = restore_lfrs(
            &encode_lfrs(&captured),
            world.revision(),
            world.committed_source().clone(),
            config,
            generous_limits(),
        )
        .expect("nested v5 tables fit the caller-bounded verifier budget");
        assert_eq!(
            restored.world().capture_snapshot().expect("recapture"),
            captured
        );
    }

    #[test]
    fn conflict_eligibility_preserves_tick_zero_distinct_from_none() {
        let (world, vehicle) = world_with_conflict_eligibility();
        let captured = world.capture_snapshot().expect("capture eligibility");
        let binding = captured.vehicles[vehicle.index() as usize]
            .conflict_eligibility
            .expect("saved eligibility");
        assert_eq!(binding.first_eligible_tick, 0);

        let restored = restore_lfrs(
            &encode_lfrs(&captured),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            generous_limits(),
        )
        .expect("restore eligibility");
        let recaptured = restored.world().capture_snapshot().expect("recapture");
        assert_eq!(recaptured, captured);
        assert_eq!(
            recaptured.vehicles[vehicle.index() as usize]
                .conflict_eligibility
                .expect("restored eligibility")
                .first_eligible_tick,
            0
        );

        let mut absent = captured;
        absent.vehicles[vehicle.index() as usize].conflict_eligibility = None;
        assert_ne!(
            crate::deterministic_state_digest(&absent).expect("None digest"),
            crate::deterministic_state_digest(&recaptured).expect("tick-zero digest")
        );
    }

    #[test]
    fn conflict_eligibility_rejects_gate_policy_deny_at_restored_time() {
        const POLICIES: &[u8] = include_bytes!(
            "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/expected.lfca"
        );
        let revision = laneflow_static_network::build_shared_network_revision(
            laneflow_format::check_canonical_network_input(
                POLICIES,
                laneflow_format::FormatLimits::HARD,
            )
            .expect("checked policy fixture"),
            laneflow_static_network::SharedNetworkBuildOptions::new(
                laneflow_static_network::SpatialBuildOption::Omit,
                laneflow_static_network::SharedNetworkBuildLimits::new(
                    64 * 1_024 * 1_024,
                    16 * 1_024 * 1_024,
                ),
            ),
        )
        .expect("shared policy fixture");
        let profile = laneflow_static_contract::VehicleProfileOrdinal::from_raw(0);
        let policy_count = revision
            .identity()
            .entity_count(laneflow_static_contract::EntityKind::RightOfWayPolicySet);
        let stream_count = revision
            .identity()
            .entity_count(laneflow_static_contract::EntityKind::ParticipantStream);
        let pin = |ordinal| {
            crate::WorldPolicySelection::Pinned(crate::PolicyPin {
                policy: revision
                    .identity()
                    .stable_id(
                        laneflow_static_contract::RightOfWayPolicySetOrdinal::from_raw(ordinal),
                    )
                    .expect("policy identity"),
            })
        };
        let origin = *revision.canonical_origin();
        let mut selection = None;
        for stream_raw in 0..stream_count {
            let stream = laneflow_static_contract::ParticipantStreamOrdinal::from_raw(stream_raw);
            let gate = revision
                .conflict()
                .participant_stream(stream)
                .and_then(|view| view.passages().first())
                .map(|passage| passage.admission_gate())
                .expect("stream passage Gate");
            let mut candidate = None;
            let mut deny = None;
            for policy_raw in 0..policy_count {
                let world = TrafficWorld::install(
                    Arc::clone(&revision),
                    WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
                    source_for(origin, "fixture://eligibility-policy-selection"),
                    77,
                    pin(policy_raw),
                )
                .expect("install selected policy");
                match world.gate_policy_decision(gate, profile) {
                    crate::GatePolicyDecision::Candidate(_) => candidate = Some(policy_raw),
                    crate::GatePolicyDecision::DenyAndStop => deny = Some(policy_raw),
                }
            }
            if let (Some(candidate), Some(deny)) = (candidate, deny) {
                selection = Some((stream, candidate, deny));
                break;
            }
        }
        let (stream, candidate_policy, deny_policy) =
            selection.expect("fixture has Candidate/Deny policy pair for one stream");
        let mut world = TrafficWorld::install(
            Arc::clone(&revision),
            WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
            source_for(origin, "fixture://eligibility-policy-selection"),
            77,
            pin(candidate_policy),
        )
        .expect("install Candidate policy");
        let path = revision
            .conflict()
            .participant_stream(stream)
            .expect("selected stream")
            .maneuver_path();
        let route = world
            .register_route(RouteRegisterInput::new(
                revision
                    .traffic()
                    .maneuvers()
                    .maneuver_path(path)
                    .expect("selected path")
                    .edges()
                    .to_vec(),
            ))
            .expect("selected route");
        let locator = world
            .conflict_passage_occurrence_locator(route, 0)
            .expect("selected conflict occurrence");
        let gate_hop = locator.admission_gate_hop();
        let gate_progress = {
            let edge =
                world.compiled_route(route).expect("compiled route").edges[gate_hop as usize];
            revision.traffic().lane_lengths_millimetres()[edge.index()]
        };
        let vehicle = world
            .restore_unparked_vehicle(
                VehicleSpawnInput::new(profile, route, gate_hop, gate_progress, 0),
                0,
                VehicleStatus::Active,
                None,
                None,
                true,
            )
            .expect("restore Candidate vehicle");
        world.conflict_eligibility.resize(
            usize::try_from(world.config.vehicle_capacity()).expect("vehicle capacity"),
            None,
        );
        world.conflict_eligibility[vehicle.index() as usize] =
            crate::ConflictEligibilityState::update(None, locator, true, 0);
        assert!(world.conflict_state_valid());

        let mut captured = world
            .capture_snapshot()
            .expect("capture Candidate eligibility");
        let snapshot_vehicle_id = captured.vehicles[vehicle.index() as usize].snapshot_vehicle_id;
        captured.policy_selection = pin(deny_policy);

        assert_eq!(
            restore_lfrs(
                &encode_lfrs(&captured),
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            }
        );
    }

    #[test]
    fn dangling_and_wrong_occurrence_conflict_locators_fail_closed() {
        let (world, _) = world_with_conflict_reservation();
        let captured = world.capture_snapshot().expect("capture Conflict state");
        let snapshot_vehicle_id = captured.vehicles[0].snapshot_vehicle_id;

        let mut dangling = captured.clone();
        dangling.vehicles[0]
            .conflict_reservation
            .as_mut()
            .expect("reservation")
            .passages[0]
            .passage
            .participant_stream = StableId128::from_bytes([0xff; 16]);
        assert_eq!(
            restore_lfrs(
                &encode_lfrs(&dangling),
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            }
        );

        let mut wrong_occurrence = captured;
        wrong_occurrence.vehicles[0]
            .conflict_reservation
            .as_mut()
            .expect("reservation")
            .passages[0]
            .entry_route_edge_index += 1;
        assert_eq!(
            restore_lfrs(
                &encode_lfrs(&wrong_occurrence),
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            }
        );
    }

    #[test]
    fn same_revision_cutover_preserves_conflict_authority_and_history() {
        let (mut world, _) = world_with_conflict_reservation();
        let before = world.capture_snapshot().expect("capture before cutover");
        let origin = *world.revision().canonical_origin();
        let descriptor = crate::NetworkRevisionCutoverDescriptor::new(
            crate::LfcaOriginBinding::from_canonical_origin(origin),
            crate::LfcaOriginBinding::from_canonical_origin(origin),
            None,
            crate::MigrationPolicyKind::SameRevisionRestore,
            world.world_binding(),
        );
        let target = world.revision();
        let _events = world
            .cutover_same_revision(
                Arc::clone(&target),
                source_for(origin, "fixture://same-revision-conflict"),
                &descriptor,
                &crate::CutoverPreflightLimits::new(1_048_576),
            )
            .expect("same-revision Conflict cutover");
        let after = world.capture_snapshot().expect("capture after cutover");
        assert_eq!(
            after.vehicles[0].conflict_reservation,
            before.vehicles[0].conflict_reservation
        );
        assert_eq!(
            after.vehicles[0].maneuver_traversal,
            before.vehicles[0].maneuver_traversal
        );
        assert_eq!(after.conflict_lag_states, before.conflict_lag_states);
        assert!(world.conflict_state_valid());
    }

    #[test]
    fn same_revision_cutover_preserves_conflict_eligibility() {
        let (mut world, _) = world_with_conflict_eligibility();
        let before = world.capture_snapshot().expect("capture before cutover");
        let origin = *world.revision().canonical_origin();
        let descriptor = crate::NetworkRevisionCutoverDescriptor::new(
            crate::LfcaOriginBinding::from_canonical_origin(origin),
            crate::LfcaOriginBinding::from_canonical_origin(origin),
            None,
            crate::MigrationPolicyKind::SameRevisionRestore,
            world.world_binding(),
        );
        let target = world.revision();
        let _events = world
            .cutover_same_revision(
                Arc::clone(&target),
                source_for(origin, "fixture://same-revision-eligibility"),
                &descriptor,
                &crate::CutoverPreflightLimits::new(1_048_576),
            )
            .expect("same-revision eligibility cutover");
        let after = world.capture_snapshot().expect("capture after cutover");
        assert_eq!(
            after.vehicles[0].conflict_eligibility,
            before.vehicles[0].conflict_eligibility
        );
        assert!(world.conflict_state_valid());
    }

    #[test]
    fn duplicate_and_future_conflict_history_fail_closed() {
        let (world, _) = world_with_conflict_reservation();
        let captured = world.capture_snapshot().expect("capture Conflict state");
        let mut duplicate = captured.clone();
        duplicate
            .conflict_lag_states
            .push(duplicate.conflict_lag_states[0]);
        assert_eq!(
            restore_lfrs(
                &encode_lfrs(&duplicate),
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::InvalidConflictHistory
        );
        let mut future = captured;
        future.conflict_lag_states[0].reference = crate::ConflictLagReference::CutoverFloor(1);
        assert_eq!(
            restore_lfrs(
                &encode_lfrs(&future),
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::InvalidConflictHistory
        );
    }

    fn captured_parking_for(
        world: &TrafficWorld,
        vehicle: VehicleHandle,
    ) -> Option<CapturedParkingBinding> {
        let revision = world.revision();
        let identity = revision.identity();
        let stable_target = |target: ParkingTarget| match target {
            ParkingTarget::ExplicitSpace(space) => CapturedParkingTarget::ExplicitSpace(
                *identity.stable_id(space).expect("space").as_untyped(),
            ),
            ParkingTarget::VirtualPool(facility) => CapturedParkingTarget::VirtualPool(
                *identity.stable_id(facility).expect("facility").as_untyped(),
            ),
        };
        world.parking_binding(vehicle).map(|binding| match binding {
            ParkingBinding::Occupied(target) => CapturedParkingBinding::Occupied {
                target: stable_target(target),
            },
            ParkingBinding::Reserved(reservation) => {
                let virtual_entry = match reservation.target() {
                    ParkingTarget::ExplicitSpace(_) => None,
                    ParkingTarget::VirtualPool(_) => {
                        let (edge, progress_mm) = world
                            .reservation_anchor(reservation)
                            .expect("reserved virtual anchor");
                        Some(CapturedVirtualParkingEntry {
                            lane_edge: *identity.stable_id(edge).expect("edge").as_untyped(),
                            progress_mm,
                        })
                    }
                };
                CapturedParkingBinding::Reserved {
                    target: stable_target(reservation.target()),
                    entry_route_occurrence: reservation.entry_route_occurrence(),
                    virtual_entry,
                }
            }
        })
    }

    #[test]
    fn save_load_restores_exact_logical_state_and_local_id_maps() {
        let (mut original, route, _) = world_with_vehicle(true);
        original.step(TickInput::new(100)).expect("step");
        let profile = laneflow_static_contract::VehicleProfileOrdinal::from_raw(0);
        let _second = original
            .spawn_parked_vehicle(
                ParkedVehicleSpawnInput::new(profile, route, 0, 10_000),
                ParkingTarget::ExplicitSpace(
                    laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0),
                ),
            )
            .expect("park second")
            .vehicle;
        let snapshot = original.capture_snapshot().expect("capture");
        assert_eq!(snapshot.vehicles[0].status, VehicleStatus::Active);
        assert_eq!(snapshot.vehicles[1].status, VehicleStatus::Parked);
        let bytes = encode_lfrs(&snapshot);

        let restored = restore_lfrs(
            &bytes,
            original.revision(),
            original.committed_source().clone(),
            original.config(),
            generous_limits(),
        )
        .expect("restore");
        let world = restored.world();
        assert_eq!(world.world_id(), snapshot.world_id);
        assert_eq!(world.world_generation(), crate::WorldGeneration::INITIAL);
        assert_eq!(world.tick_index(), snapshot.tick);
        assert_eq!(world.time_ms(), snapshot.time_ms);
        assert_eq!(world.command_cursor(), snapshot.command_cursor);
        assert_eq!(world.config(), snapshot.config);
        assert_eq!(
            world.observation_state_sequence(),
            ObservationStateSequence::INITIAL
        );
        assert_eq!(world.committed_source(), snapshot.source());

        let identity = world.revision.identity();
        for captured in &snapshot.routes {
            let handle = restored
                .route_handle(captured.snapshot_route_id)
                .expect("route map");
            let stable_edges = world
                .route_edges(handle)
                .expect("restored route")
                .iter()
                .map(|edge| *identity.stable_id(*edge).expect("edge").as_untyped())
                .collect::<Vec<_>>();
            assert_eq!(stable_edges, captured.edges);
        }
        for captured in &snapshot.vehicles {
            let handle = restored
                .vehicle_handle(captured.snapshot_vehicle_id)
                .expect("vehicle map");
            let state = world.vehicle(handle).expect("restored vehicle");
            assert_eq!(
                state.route(),
                restored
                    .route_handle(captured.snapshot_route_id)
                    .expect("route map")
            );
            assert_eq!(state.route_edge_index(), captured.route_edge_index);
            assert_eq!(state.progress_mm(), captured.progress_mm);
            assert_eq!(state.carry_um(), captured.carry_um);
            assert_eq!(state.speed_mm_s(), captured.speed_mm_s);
            assert_eq!(state.status(), captured.status);
            assert_eq!(
                *identity
                    .stable_id(state.profile())
                    .expect("profile")
                    .as_untyped(),
                captured.profile
            );
            assert_eq!(
                *identity
                    .stable_id(state.class())
                    .expect("class")
                    .as_untyped(),
                captured.class
            );
            assert_eq!(captured_parking_for(world, handle), captured.parking);
        }
        let mapped_live_order = snapshot
            .live_order
            .iter()
            .map(|id| restored.vehicle_handle(*id).expect("live map"))
            .collect::<Vec<_>>();
        assert_eq!(world.live_vehicles(), mapped_live_order);
        let mut world = restored.into_world();
        world
            .step(TickInput::new(100))
            .expect("restored world steps");
    }

    #[test]
    fn exhausted_command_cursor_restores_parked_and_reserved_without_new_commands() {
        let (mut world, _, _) = virtual_parking_cutover_world();
        world.command_cursor = u64::MAX;
        let captured = world.capture_snapshot().expect("capture");
        assert!(captured.vehicles.iter().any(|vehicle| matches!(
            vehicle.parking,
            Some(CapturedParkingBinding::Occupied { .. })
        )));
        assert!(captured.vehicles.iter().any(|vehicle| matches!(
            vehicle.parking,
            Some(CapturedParkingBinding::Reserved { .. })
        )));
        let restored = restore_lfrs(
            &encode_lfrs(&captured),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            generous_limits(),
        )
        .expect("restore does not consume commands");
        assert_eq!(restored.world().command_cursor(), u64::MAX);
        assert_eq!(
            crate::deterministic_state_digest(
                &restored
                    .world()
                    .capture_snapshot()
                    .expect("capture restored")
            )
            .expect("restored digest"),
            crate::deterministic_state_digest(&captured).expect("source digest")
        );
    }

    #[test]
    fn framing_and_wire_limits_fail_before_flatbuffers_lowering() {
        let (world, _, _) = world_with_vehicle(true);
        let bytes = encode_lfrs(&world.capture_snapshot().expect("capture"));
        assert_eq!(
            restore_lfrs(
                &bytes,
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                SnapshotRestoreLimits::new(1, 4 * 1_024),
            )
            .unwrap_err(),
            limit_error(
                SnapshotLimitDimension::WireBytes,
                1,
                u64::try_from(bytes.len()).expect("length")
            )
        );
        assert_eq!(
            restore_lfrs(
                &bytes[..8],
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::TruncatedFraming
        );
        let mut wrong_prefix = bytes.clone();
        wrong_prefix[0] ^= 1;
        assert!(matches!(
            restore_lfrs(
                &wrong_prefix,
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                generous_limits(),
            ),
            Err(SnapshotRestoreError::SizePrefixMismatch { .. })
        ));
        let mut wrong_identifier = bytes;
        wrong_identifier[8..12].copy_from_slice(b"NOPE");
        assert_eq!(
            restore_lfrs(
                &wrong_identifier,
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::FileIdentifierMismatch
        );

        let valid = encode_lfrs(&world.capture_snapshot().expect("capture"));
        let asset_key_len = {
            let root = wire::size_prefixed_root_as_runtime_snapshot(&valid).expect("valid LFRS");
            u64::try_from(
                root.source_published()
                    .expect("published")
                    .asset_key()
                    .len(),
            )
            .expect("asset key length")
        };
        assert_eq!(
            restore_lfrs(
                &valid,
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                SnapshotRestoreLimits::new(u64::try_from(valid.len()).expect("wire length"), 0),
            )
            .unwrap_err(),
            limit_error(SnapshotLimitDimension::AssetKeyBytes, 0, asset_key_len)
        );

        let mut structurally_invalid = encode_lfrs(&world.capture_snapshot().expect("capture"));
        structurally_invalid[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            restore_lfrs(
                &structurally_invalid,
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::InvalidFlatbuffer
        );
    }

    #[test]
    fn clock_capacity_and_duplicate_ids_fail_closed() {
        let (world, _, _) = world_with_vehicle(true);
        let revision = world.revision();
        let source = world.committed_source().clone();
        let config = world.config();

        let mut invalid_clock = world.capture_snapshot().expect("capture");
        invalid_clock.time_ms = 1;
        assert_eq!(
            restore_lfrs(
                &encode_lfrs(&invalid_clock),
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::InvalidClock
        );

        let smaller = WorldConfig::new(
            config.vehicle_capacity() - 1,
            config.route_capacity(),
            config.route_edge_occurrence_capacity(),
            config.route_conflict_occurrence_capacity(),
            config.worker_count(),
            config.fixed_delta_time_ms(),
        );
        assert!(matches!(
            restore_lfrs(
                &encode_lfrs(&world.capture_snapshot().expect("capture")),
                Arc::clone(&revision),
                source.clone(),
                smaller,
                generous_limits(),
            ),
            Err(SnapshotRestoreError::TargetCapacitySmaller {
                dimension: SnapshotLimitDimension::Vehicles,
                ..
            })
        ));

        let mut duplicate_route = world.capture_snapshot().expect("capture");
        duplicate_route
            .routes
            .push(duplicate_route.routes[0].clone());
        assert!(matches!(
            restore_lfrs(
                &encode_lfrs(&duplicate_route),
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            ),
            Err(SnapshotRestoreError::DuplicateRouteId { .. })
        ));

        let mut duplicate_vehicle = world.capture_snapshot().expect("capture");
        duplicate_vehicle
            .vehicles
            .push(duplicate_vehicle.vehicles[0].clone());
        duplicate_vehicle
            .live_order
            .push(duplicate_vehicle.live_order[0]);
        assert!(matches!(
            restore_lfrs(
                &encode_lfrs(&duplicate_vehicle),
                revision,
                source,
                config,
                generous_limits(),
            ),
            Err(SnapshotRestoreError::DuplicateVehicleId { .. })
        ));
    }

    #[test]
    fn config_axes_and_republished_source_follow_restore_contract() {
        let (world, _, _) = world_with_vehicle(true);
        let revision = world.revision();
        let source = world.committed_source().clone();
        let config = world.config();
        let bytes = encode_lfrs(&world.capture_snapshot().expect("capture"));

        let different_dt = WorldConfig::new(
            config.vehicle_capacity(),
            config.route_capacity(),
            config.route_edge_occurrence_capacity(),
            config.route_conflict_occurrence_capacity(),
            config.worker_count(),
            config.fixed_delta_time_ms() + 1,
        );
        assert_eq!(
            restore_lfrs(
                &bytes,
                Arc::clone(&revision),
                source.clone(),
                different_dt,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::FixedDeltaTimeMismatch {
                snapshot: config.fixed_delta_time_ms(),
                target: config.fixed_delta_time_ms() + 1,
            }
        );

        let smaller_routes = WorldConfig::new(
            config.vehicle_capacity(),
            config.route_capacity() - 1,
            config.route_edge_occurrence_capacity(),
            config.route_conflict_occurrence_capacity(),
            config.worker_count(),
            config.fixed_delta_time_ms(),
        );
        assert_eq!(
            restore_lfrs(
                &bytes,
                Arc::clone(&revision),
                source.clone(),
                smaller_routes,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::TargetCapacitySmaller {
                dimension: SnapshotLimitDimension::Routes,
                snapshot: u64::from(config.route_capacity()),
                target: u64::from(config.route_capacity() - 1),
            }
        );

        let smaller_occurrences = WorldConfig::new(
            config.vehicle_capacity(),
            config.route_capacity(),
            config.route_edge_occurrence_capacity() - 1,
            config.route_conflict_occurrence_capacity(),
            config.worker_count(),
            config.fixed_delta_time_ms(),
        );
        assert_eq!(
            restore_lfrs(
                &bytes,
                Arc::clone(&revision),
                source.clone(),
                smaller_occurrences,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::TargetCapacitySmaller {
                dimension: SnapshotLimitDimension::RouteEdgeOccurrences,
                snapshot: config.route_edge_occurrence_capacity(),
                target: config.route_edge_occurrence_capacity() - 1,
            }
        );

        let larger = WorldConfig::new(
            config.vehicle_capacity() + 1,
            config.route_capacity() + 1,
            config.route_edge_occurrence_capacity() + 1,
            config.route_conflict_occurrence_capacity() + 1,
            config.worker_count(),
            config.fixed_delta_time_ms(),
        );
        let restored = restore_lfrs(
            &bytes,
            Arc::clone(&revision),
            source.clone(),
            larger,
            generous_limits(),
        )
        .expect("semantic capacities may grow");
        assert_eq!(restored.world().config(), larger);
        drop(restored);

        let mut saved_with_other_worker = world.capture_snapshot().expect("capture");
        saved_with_other_worker.config = WorldConfig::new(
            config.vehicle_capacity(),
            config.route_capacity(),
            config.route_edge_occurrence_capacity(),
            config.route_conflict_occurrence_capacity(),
            99,
            config.fixed_delta_time_ms(),
        );
        let restored = restore_lfrs(
            &encode_lfrs(&saved_with_other_worker),
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        )
        .expect("saved worker plan is ignored and rebuilt from target config");
        assert_eq!(
            restored.world().config().worker_count(),
            config.worker_count()
        );
        drop(restored);

        let republished = CommittedNetworkSource::Published {
            reference: crate::PublishedLfcaReference::new(
                "asset://same-revision-republished",
                laneflow_static_contract::Sha256Digest::from_bytes([0xa5; 32]),
                laneflow_static_contract::ExactByteLength::new(777),
                revision.network_revision(),
            )
            .expect("republished source"),
        };
        assert_ne!(republished, source);
        let restored = restore_lfrs(
            &bytes,
            revision,
            republished.clone(),
            config,
            generous_limits(),
        )
        .expect("same semantic revision permits republished exact bytes");
        assert_eq!(restored.world().committed_source(), &republished);
    }

    #[test]
    fn occurrence_capacity_max_and_max_plus_one_fail_atomically() {
        let (world, _, _) = world_with_vehicle(true);
        let revision = world.revision();
        let source = world.committed_source().clone();
        let mut at_max = world.capture_snapshot().expect("capture");
        let occurrence_count = at_max
            .routes
            .iter()
            .map(|route| u64::try_from(route.edges.len()).expect("edge count"))
            .sum::<u64>();
        at_max.config = WorldConfig::new(
            at_max.config.vehicle_capacity(),
            at_max.config.route_capacity(),
            occurrence_count,
            at_max.config.route_conflict_occurrence_capacity(),
            at_max.config.worker_count(),
            at_max.config.fixed_delta_time_ms(),
        );
        let exact_config = at_max.config;
        let restored = restore_lfrs(
            &encode_lfrs(&at_max),
            Arc::clone(&revision),
            source.clone(),
            exact_config,
            generous_limits(),
        )
        .expect("occurrence total exactly at max");
        assert_eq!(
            restored
                .world()
                .capture_snapshot()
                .expect("capture")
                .routes()
                .iter()
                .map(|route| u64::try_from(route.edges().len()).expect("edge count"))
                .sum::<u64>(),
            occurrence_count
        );

        let mut max_plus_one = at_max;
        let extra_edge = max_plus_one.routes[0].edges[0];
        max_plus_one.routes[0].edges.push(extra_edge);
        assert_eq!(
            restore_lfrs(
                &encode_lfrs(&max_plus_one),
                revision,
                source,
                exact_config,
                generous_limits(),
            )
            .unwrap_err(),
            limit_error(
                SnapshotLimitDimension::RouteEdgeOccurrences,
                occurrence_count,
                occurrence_count + 1,
            )
        );
    }

    #[test]
    fn parking_and_live_order_invariants_fail_closed() {
        let (mut world, route, _) = world_with_vehicle(true);
        let _second = world
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
        let revision = world.revision();
        let source = world.committed_source().clone();
        let config = world.config();

        let mut parking_mismatch = world.capture_snapshot().expect("capture");
        parking_mismatch.vehicles[1].parking = None;
        assert!(matches!(
            restore_lfrs(
                &encode_lfrs(&parking_mismatch),
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            ),
            Err(SnapshotRestoreError::ParkingStatusMismatch { .. })
        ));

        let mut duplicate_live = world.capture_snapshot().expect("capture");
        duplicate_live.live_order[1] = duplicate_live.live_order[0];
        assert!(matches!(
            restore_lfrs(
                &encode_lfrs(&duplicate_live),
                revision,
                source,
                config,
                generous_limits(),
            ),
            Err(SnapshotRestoreError::DuplicateLiveOrderVehicle { .. })
        ));
    }

    #[test]
    fn virtual_parking_corruption_capacity_and_duplicate_resources_fail_closed() {
        let (world, reserved, _occupied) = virtual_parking_cutover_world();
        let revision = world.revision();
        let source = world.committed_source().clone();
        let config = world.config();
        let before = world.capture_snapshot().expect("base virtual snapshot");
        let reserved_id = before
            .vehicles
            .iter()
            .find(|vehicle| vehicle.status == VehicleStatus::Active)
            .expect("reserved row")
            .snapshot_vehicle_id;
        assert_eq!(
            world.vehicle(reserved).expect("reserved").status(),
            VehicleStatus::Active
        );

        let mut missing_target = before.clone();
        let row = missing_target
            .vehicles
            .iter_mut()
            .find(|vehicle| vehicle.snapshot_vehicle_id == reserved_id)
            .expect("reserved row");
        let Some(CapturedParkingBinding::Reserved { target, .. }) = row.parking.as_mut() else {
            panic!("reserved binding")
        };
        *target = CapturedParkingTarget::VirtualPool(StableId128::from_bytes([0xab; 16]));
        assert_eq!(
            restore_lfrs(
                &encode_lfrs(&missing_target),
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::UnknownParkingFacility {
                snapshot_vehicle_id: reserved_id
            }
        );

        let facility = laneflow_static_contract::ParkingFacilityOrdinal::from_raw(0);
        let facility_stable = *world
            .revision
            .identity()
            .stable_id(facility)
            .expect("facility stable id")
            .as_untyped();
        let mut wrong_kind = before.clone();
        let row = wrong_kind
            .vehicles
            .iter_mut()
            .find(|vehicle| vehicle.snapshot_vehicle_id == reserved_id)
            .expect("reserved row");
        let Some(CapturedParkingBinding::Reserved { target, .. }) = row.parking.as_mut() else {
            panic!("reserved binding")
        };
        *target = CapturedParkingTarget::ExplicitSpace(facility_stable);
        assert_eq!(
            restore_lfrs(
                &encode_lfrs(&wrong_kind),
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::UnknownParkingSpace {
                snapshot_vehicle_id: reserved_id
            }
        );

        let mut moved_anchor = before.clone();
        let row = moved_anchor
            .vehicles
            .iter_mut()
            .find(|vehicle| vehicle.snapshot_vehicle_id == reserved_id)
            .expect("reserved row");
        let Some(CapturedParkingBinding::Reserved {
            virtual_entry: Some(entry),
            ..
        }) = row.parking.as_mut()
        else {
            panic!("virtual entry")
        };
        entry.progress_mm += 1;
        assert_eq!(
            restore_lfrs(
                &encode_lfrs(&moved_anchor),
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::UnknownVirtualParkingEntry {
                snapshot_vehicle_id: reserved_id
            }
        );

        let mut over_capacity = before.clone();
        let template = over_capacity
            .vehicles
            .iter()
            .find(|vehicle| vehicle.status == VehicleStatus::Parked)
            .expect("occupied row")
            .clone();
        for snapshot_vehicle_id in [3, 4] {
            let mut duplicate = template.clone();
            duplicate.snapshot_vehicle_id = snapshot_vehicle_id;
            over_capacity.vehicles.push(duplicate);
            over_capacity.live_order.push(snapshot_vehicle_id);
        }
        assert!(matches!(
            restore_lfrs(
                &encode_lfrs(&over_capacity),
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            ),
            Err(SnapshotRestoreError::Parking {
                error: ParkingError::VirtualCapacityExhausted,
                ..
            })
        ));
        assert_eq!(world.capture_snapshot().expect("zero publish"), before);

        let (mut explicit_world, route, _) = world_with_vehicle(true);
        explicit_world
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
            .expect("explicit parked");
        let explicit_revision = explicit_world.revision();
        let explicit_source = explicit_world.committed_source().clone();
        let explicit_config = explicit_world.config();
        let mut duplicate_resource = explicit_world.capture_snapshot().expect("explicit capture");
        let mut duplicate = duplicate_resource
            .vehicles
            .iter()
            .find(|vehicle| vehicle.status == VehicleStatus::Parked)
            .expect("parked row")
            .clone();
        duplicate.snapshot_vehicle_id = 3;
        duplicate_resource.vehicles.push(duplicate);
        duplicate_resource.live_order.push(3);
        assert!(matches!(
            restore_lfrs(
                &encode_lfrs(&duplicate_resource),
                explicit_revision,
                explicit_source,
                explicit_config,
                generous_limits(),
            ),
            Err(SnapshotRestoreError::Parking {
                error: ParkingError::TargetBoundByOther,
                ..
            })
        ));
    }

    #[test]
    fn dangling_references_and_live_order_gaps_fail_closed() {
        let (world, _route, _first) = world_with_vehicle(true);
        let revision = world.revision();
        let source = world.committed_source().clone();
        let config = world.config();

        // 悬空路线引用：车辆指向不存在的局部路线 ID（合同 §5「悬空引用」）。
        let mut dangling_route = world.capture_snapshot().expect("capture");
        dangling_route.vehicles[0].snapshot_route_id =
            dangling_route.routes[0].snapshot_route_id + 99;
        assert!(matches!(
            restore_lfrs(
                &encode_lfrs(&dangling_route),
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            ),
            Err(SnapshotRestoreError::UnknownRouteReference { .. })
        ));

        // live 序含未知车辆：非零但不指向任何快照车辆。
        let mut unknown_live = world.capture_snapshot().expect("capture");
        unknown_live.live_order[0] = 99;
        assert!(matches!(
            restore_lfrs(
                &encode_lfrs(&unknown_live),
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            ),
            Err(SnapshotRestoreError::UnknownLiveOrderVehicle { .. })
        ));

        // live 序缺项：长度小于活跃车辆数（合同 §5「精确排列」）。
        let mut incomplete_live = world.capture_snapshot().expect("capture");
        incomplete_live.live_order.pop();
        assert!(matches!(
            restore_lfrs(
                &encode_lfrs(&incomplete_live),
                Arc::clone(&revision),
                source,
                config,
                generous_limits(),
            ),
            Err(SnapshotRestoreError::IncompleteLiveOrder)
        ));
    }

    #[test]
    fn unknown_parking_space_and_participant_class_fail_closed() {
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
        let revision = world.revision();
        let source = world.committed_source().clone();
        let config = world.config();

        // 未知停车位稳定标识：绑定一致（Parked + Some）但 ID 不解析。
        let mut unknown_space = world.capture_snapshot().expect("capture");
        unknown_space.vehicles[1].parking = Some(CapturedParkingBinding::Occupied {
            target: CapturedParkingTarget::ExplicitSpace(
                laneflow_static_contract::StableId128::from_bytes([0xAB; 16]),
            ),
        });
        assert!(matches!(
            restore_lfrs(
                &encode_lfrs(&unknown_space),
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            ),
            Err(SnapshotRestoreError::UnknownParkingSpace { .. })
        ));

        // 未知参与者类别稳定标识：profile 可解析、class 不解析。
        let mut unknown_class = world.capture_snapshot().expect("capture");
        unknown_class.vehicles[0].class =
            laneflow_static_contract::StableId128::from_bytes([0xCD; 16]);
        assert!(matches!(
            restore_lfrs(
                &encode_lfrs(&unknown_class),
                revision,
                source,
                config,
                generous_limits(),
            ),
            Err(SnapshotRestoreError::UnknownParticipantClass { .. })
        ));
    }

    #[test]
    fn vehicle_identity_value_and_overlap_invariants_fail_closed() {
        let (world, _, _) = world_with_vehicle(true);
        let revision = world.revision();
        let source = world.committed_source().clone();
        let config = world.config();

        let mut unknown_profile = world.capture_snapshot().expect("capture");
        unknown_profile.vehicles[0].profile = StableId128::from_bytes([0xff; 16]);
        assert!(matches!(
            restore_lfrs(
                &encode_lfrs(&unknown_profile),
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            ),
            Err(SnapshotRestoreError::UnknownVehicleProfile { .. })
        ));

        let mut invalid_carry = world.capture_snapshot().expect("capture");
        invalid_carry.vehicles[0].carry_um = 1_000;
        assert!(matches!(
            restore_lfrs(
                &encode_lfrs(&invalid_carry),
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            ),
            Err(SnapshotRestoreError::CarryOutOfRange { .. })
        ));

        let mut invalid_completed = world.capture_snapshot().expect("capture");
        invalid_completed.vehicles[0].status = VehicleStatus::Completed;
        assert!(matches!(
            restore_lfrs(
                &encode_lfrs(&invalid_completed),
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            ),
            Err(SnapshotRestoreError::InvalidCompletedState { .. })
        ));

        let mut overlap = world.capture_snapshot().expect("capture");
        let mut duplicate = overlap.vehicles[0].clone();
        duplicate.snapshot_vehicle_id = 2;
        overlap.vehicles.push(duplicate);
        overlap.live_order.push(2);
        assert!(matches!(
            restore_lfrs(
                &encode_lfrs(&overlap),
                revision,
                source,
                config,
                generous_limits(),
            ),
            Err(SnapshotRestoreError::Vehicle {
                error: SpawnError::Overlap,
                ..
            })
        ));
    }

    #[test]
    fn closed_versions_bindings_and_enums_reject_unknown_values() {
        let (world, _, _) = world_with_vehicle(true);
        let revision = world.revision();
        let source = world.committed_source().clone();
        let config = world.config();
        let valid = encode_lfrs(&world.capture_snapshot().expect("capture"));

        let mut prior_format = valid.clone();
        let format_offset = {
            let root =
                wire::size_prefixed_root_as_runtime_snapshot(&prior_format).expect("verified LFRS");
            table_field_offset(root._tab, wire::RuntimeSnapshot::VT_FORMAT_VERSION)
        };
        prior_format[format_offset..format_offset + 4].copy_from_slice(&4_u32.to_le_bytes());
        assert_eq!(
            restore_lfrs(
                &prior_format,
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::UnsupportedFormatVersion { actual: 4 }
        );

        let mut unknown_format = valid.clone();
        unknown_format[format_offset..format_offset + 4].copy_from_slice(&6_u32.to_le_bytes());
        assert_eq!(
            restore_lfrs(
                &unknown_format,
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::UnsupportedFormatVersion { actual: 6 }
        );

        let mut prior_runtime = valid.clone();
        let runtime_offset = {
            let root = wire::size_prefixed_root_as_runtime_snapshot(&prior_runtime)
                .expect("verified LFRS");
            table_field_offset(root._tab, wire::RuntimeSnapshot::VT_RUNTIME_STATE_VERSION)
        };
        prior_runtime[runtime_offset..runtime_offset + 2].copy_from_slice(&4_u16.to_le_bytes());
        assert_eq!(
            restore_lfrs(
                &prior_runtime,
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::UnsupportedRuntimeStateVersion { actual: 4 }
        );

        let mut unknown_runtime = valid.clone();
        unknown_runtime[runtime_offset..runtime_offset + 2].copy_from_slice(&6_u16.to_le_bytes());
        assert_eq!(
            restore_lfrs(
                &unknown_runtime,
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::UnsupportedRuntimeStateVersion { actual: 6 }
        );

        let mut unknown_fields = valid.clone();
        append_zero_root_vtable_field(&mut unknown_fields);
        assert_eq!(
            restore_lfrs(
                &unknown_fields,
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::UnknownTableFields {
                table: "RuntimeSnapshot",
                supported: ROOT_V5_FIELDS,
                actual: ROOT_V5_FIELDS + 4,
            }
        );

        let mut missing_published = valid.clone();
        clear_root_table_field(
            &mut missing_published,
            wire::RuntimeSnapshot::VT_SOURCE_PUBLISHED,
        );
        assert_eq!(
            restore_lfrs(
                &missing_published,
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::MissingField {
                field: "source_published",
            }
        );

        let mut missing_required_routes = valid.clone();
        clear_root_table_field(
            &mut missing_required_routes,
            wire::RuntimeSnapshot::VT_ROUTES,
        );
        assert_eq!(
            restore_lfrs(
                &missing_required_routes,
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::InvalidFlatbuffer
        );

        let mut unknown_source = valid.clone();
        let source_kind_offset = {
            let root = wire::size_prefixed_root_as_runtime_snapshot(&unknown_source)
                .expect("verified LFRS");
            table_field_offset(root._tab, wire::RuntimeSnapshot::VT_SOURCE_KIND)
        };
        unknown_source[source_kind_offset] = 0xff;
        assert_eq!(
            restore_lfrs(
                &unknown_source,
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::UnsupportedSourceKind { actual: 0xff }
        );

        let mut unknown_status = valid.clone();
        let status_offset = {
            let root = wire::size_prefixed_root_as_runtime_snapshot(&unknown_status)
                .expect("verified LFRS");
            table_field_offset(
                root.vehicles().get(0)._tab,
                wire::SnapshotVehicle::VT_STATUS,
            )
        };
        unknown_status[status_offset] = 0xff;
        assert!(matches!(
            restore_lfrs(
                &unknown_status,
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            ),
            Err(SnapshotRestoreError::InvalidVehicleStatus { actual: 0xff, .. })
        ));

        let (parking_world, _, _) = virtual_parking_cutover_world();
        let parking_revision = parking_world.revision();
        let parking_source = parking_world.committed_source().clone();
        let parking_config = parking_world.config();
        let parking_valid =
            encode_lfrs(&parking_world.capture_snapshot().expect("parking capture"));
        let mut unknown_parking_state = parking_valid.clone();
        let (parking_state_offset, parking_vehicle_id) = {
            let root = wire::size_prefixed_root_as_runtime_snapshot(&unknown_parking_state)
                .expect("verified parking LFRS");
            let vehicle = root.vehicles().get(0);
            let binding = vehicle.parking().expect("parking binding");
            (
                table_field_offset(binding._tab, wire::ParkingBinding::VT_STATE),
                vehicle.snapshot_vehicle_id(),
            )
        };
        unknown_parking_state[parking_state_offset] = 0xff;
        assert_eq!(
            restore_lfrs(
                &unknown_parking_state,
                Arc::clone(&parking_revision),
                parking_source.clone(),
                parking_config,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::InvalidParkingBindingState {
                snapshot_vehicle_id: parking_vehicle_id,
                actual: 0xff,
            }
        );

        let mut unknown_parking_kind = parking_valid;
        let parking_kind_offset = {
            let root = wire::size_prefixed_root_as_runtime_snapshot(&unknown_parking_kind)
                .expect("verified parking LFRS");
            let binding = root.vehicles().get(0).parking().expect("parking binding");
            table_field_offset(binding._tab, wire::ParkingBinding::VT_TARGET_KIND)
        };
        unknown_parking_kind[parking_kind_offset] = 0xff;
        assert_eq!(
            restore_lfrs(
                &unknown_parking_kind,
                parking_revision,
                parking_source,
                parking_config,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::InvalidParkingTargetKind {
                snapshot_vehicle_id: parking_vehicle_id,
                actual: 0xff,
            }
        );

        let mut wrong_revision = valid.clone();
        let revision_offset = {
            let root = wire::size_prefixed_root_as_runtime_snapshot(&wrong_revision)
                .expect("verified LFRS");
            table_field_offset(root._tab, wire::RuntimeSnapshot::VT_NETWORK_REVISION)
        };
        wrong_revision[revision_offset] ^= 1;
        assert_eq!(
            restore_lfrs(
                &wrong_revision,
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::NetworkRevisionMismatch
        );

        let mut wrong_contract = valid.clone();
        let contract_offset = {
            let root = wire::size_prefixed_root_as_runtime_snapshot(&wrong_contract)
                .expect("verified LFRS");
            table_field_offset(
                root._tab,
                wire::RuntimeSnapshot::VT_STATIC_CONTRACT_VERSIONS,
            )
        };
        wrong_contract[contract_offset] ^= 1;
        assert_eq!(
            restore_lfrs(
                &wrong_contract,
                Arc::clone(&revision),
                source.clone(),
                config,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::StaticContractVersionsMismatch
        );

        let mut wrong_source_revision = valid.clone();
        let source_revision_offset = {
            let root = wire::size_prefixed_root_as_runtime_snapshot(&wrong_source_revision)
                .expect("verified LFRS");
            table_field_offset(
                root.source_published().expect("published")._tab,
                wire::PublishedSourceBinding::VT_NETWORK_REVISION,
            )
        };
        wrong_source_revision[source_revision_offset] ^= 1;
        assert_eq!(
            restore_lfrs(
                &wrong_source_revision,
                revision,
                source,
                config,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::SnapshotSourceRevisionMismatch
        );

        // 事件游标随切片 C 事件批次通道成为真实轴：非零值恢复为世界状态。
        let mut event_cursor = world.capture_snapshot().expect("capture");
        event_cursor.event_cursor = 7;
        let restored = restore_lfrs(
            &encode_lfrs(&event_cursor),
            world.revision(),
            world.committed_source().clone(),
            config,
            generous_limits(),
        )
        .unwrap()
        .into_world();
        assert_eq!(restored.event_cursor(), 7);
    }

    fn table_field_offset(
        table: laneflow_runtime_snapshot_wire::runtime::Table<'_>,
        field: laneflow_runtime_snapshot_wire::runtime::VOffsetT,
    ) -> usize {
        let relative = usize::from(table.vtable().get(field));
        assert_ne!(relative, 0, "fixture field must be present");
        table.loc() + relative
    }

    fn root_vtable_start(bytes: &[u8]) -> usize {
        let root = wire::size_prefixed_root_as_runtime_snapshot(bytes).expect("verified LFRS");
        let table = root._tab.loc();
        let backwards = i32::from_le_bytes(
            bytes[table..table + 4]
                .try_into()
                .expect("root table vtable offset"),
        );
        assert!(backwards > 0);
        table - usize::try_from(backwards).expect("positive vtable offset")
    }

    fn clear_root_table_field(
        bytes: &mut [u8],
        field: laneflow_runtime_snapshot_wire::runtime::VOffsetT,
    ) {
        let vtable = root_vtable_start(bytes);
        let entry = vtable + usize::from(field);
        bytes[entry..entry + 2].copy_from_slice(&0_u16.to_le_bytes());
    }

    fn append_zero_root_vtable_field(bytes: &mut Vec<u8>) {
        let (vtable, table, backwards) = {
            let root = wire::size_prefixed_root_as_runtime_snapshot(bytes).expect("verified LFRS");
            let table = root._tab.loc();
            let backwards = i32::from_le_bytes(
                bytes[table..table + 4]
                    .try_into()
                    .expect("root table vtable offset"),
            );
            (root_vtable_start(bytes), table, backwards)
        };
        let current_bytes = u16::from_le_bytes(
            bytes[vtable..vtable + 2]
                .try_into()
                .expect("vtable byte length"),
        );
        let extended_bytes = current_bytes
            .checked_add(8)
            .expect("four extra fields preserve root table alignment");
        let extra = vtable + usize::from(current_bytes);
        bytes.splice(extra..extra, [0_u8; 8]);
        let declared = u32::from_le_bytes(bytes[..4].try_into().expect("size prefix"));
        bytes[..4].copy_from_slice(
            &declared
                .checked_add(8)
                .expect("extended size prefix")
                .to_le_bytes(),
        );
        let root_offset = u32::from_le_bytes(bytes[4..8].try_into().expect("root offset"));
        bytes[4..8].copy_from_slice(
            &root_offset
                .checked_add(8)
                .expect("shifted root offset")
                .to_le_bytes(),
        );
        bytes[table + 8..table + 12].copy_from_slice(
            &backwards
                .checked_add(8)
                .expect("shifted vtable offset")
                .to_le_bytes(),
        );
        bytes[vtable..vtable + 2].copy_from_slice(&extended_bytes.to_le_bytes());
    }

    #[test]
    fn restored_routes_still_use_common_admitted_compiler() {
        let (mut world, route, _) = world_with_vehicle(true);
        let second = world
            .register_route(RouteRegisterInput::new(
                world.route_edges(route).expect("route").to_vec(),
            ))
            .expect("second route");
        let snapshot = world.capture_snapshot().expect("capture");
        let restored = restore_lfrs(
            &encode_lfrs(&snapshot),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            generous_limits(),
        )
        .expect("restore");
        assert!(restored.route_handle(1).is_some());
        assert!(restored.route_handle(2).is_some());
        assert!(restored.route_handle(3).is_none());
        assert!(world.route_edges(second).is_some());
    }
}
