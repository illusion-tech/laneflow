//! `LFRS` v1 的 verifier-first 读取、语义 lowering 与原子新世界恢复。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use laneflow_runtime_snapshot_wire::generated::lane_flow::runtime_snapshot::v1 as wire;
use laneflow_runtime_snapshot_wire::runtime::VerifierOptions;
use laneflow_static_contract::{ParkingSpaceId, ParticipantClassId, StableId128, VehicleProfileId};
use laneflow_static_network::SharedNetworkRevision;
use thiserror::Error;

use crate::{
    AdmittedRouteRegisterError, AdmittedRouteRegisterInput, CommittedNetworkSource, InstallError,
    ObservationStateSequence, ParkingError, RouteHandle, SpawnError, StepError, TrafficWorld,
    VehicleHandle, VehicleSpawnInput, VehicleStatus, WorldConfig,
};
use crate::{RUNTIME_STATE_VERSION, SNAPSHOT_FORMAT_VERSION};

const MIN_SIZE_PREFIXED_LFRS_BYTES: usize = 12;
const MAX_SCHEMA_TABLE_DEPTH: usize = 4;
const APPARENT_SIZE_MULTIPLIER: usize = 16;
const MICROMETRES_PER_MILLIMETRE: u16 = 1_000;

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
    /// v1 event cursor 必须为零。
    #[error("v1 event cursor 必须为零: {actual}")]
    EventCursorNonZero {
        /// 实际值。
        actual: u64,
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
    let root = verify_lfrs(bytes, target_config, limits)?;
    validate_bindings(root, revision.as_ref(), &source, target_config, limits)?;

    let mut world = TrafficWorld::install(revision, target_config, source, root.world_id())
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

    let vehicle_rows = root.vehicles();
    let mut vehicle_map = BTreeMap::new();
    // 非 Active 先恢复并立即转入 Parked / Completed；这样 transient spawn 不会把
    // 合法的非占用重叠误判为 Active 重叠。Active 最后恢复。
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
    world.tick_index = root.tick();
    world.time_ms = root.time_ms();
    world.command_cursor = root.command_cursor();
    world.observation_state_sequence = ObservationStateSequence::INITIAL;
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

fn verify_lfrs<'a>(
    bytes: &'a [u8],
    target_config: WorldConfig,
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

    let max_tables_u64 = u64::from(target_config.route_capacity())
        .checked_add(u64::from(target_config.vehicle_capacity()))
        .and_then(|value| value.checked_add(3))
        .ok_or_else(|| limit_error(SnapshotLimitDimension::VerifierBudget, u64::MAX, u64::MAX))?;
    let max_tables = usize::try_from(max_tables_u64).map_err(|_| {
        limit_error(
            SnapshotLimitDimension::VerifierBudget,
            usize::MAX as u64,
            max_tables_u64,
        )
    })?;
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
    if root.event_cursor() != 0 {
        return Err(SnapshotRestoreError::EventCursorNonZero {
            actual: root.event_cursor(),
        });
    }
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
    let parking_binding_matches = match status {
        VehicleStatus::Parked => vehicle.parking_space().is_some(),
        VehicleStatus::Active | VehicleStatus::Completed => vehicle.parking_space().is_none(),
    };
    if !parking_binding_matches {
        return Err(SnapshotRestoreError::ParkingStatusMismatch {
            snapshot_vehicle_id,
        });
    }
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

    let handle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            profile,
            route,
            vehicle.route_edge_index(),
            vehicle.progress_mm(),
            vehicle.speed_mm_s(),
        ))
        .map_err(|error| SnapshotRestoreError::Vehicle {
            snapshot_vehicle_id,
            error,
        })?;
    match status {
        VehicleStatus::Active => {
            world.vehicles[usize::try_from(handle.index()).expect("vehicle index")]
                .state
                .as_mut()
                .expect("spawned vehicle")
                .carry_um = vehicle.carry_um();
        }
        VehicleStatus::Parked => {
            let parking = vehicle
                .parking_space()
                .expect("parking binding validated for Parked");
            let parking = world
                .revision
                .identity()
                .ordinal(ParkingSpaceId::from_untyped(StableId128::from_bytes(
                    parking.0,
                )))
                .ok_or(SnapshotRestoreError::UnknownParkingSpace {
                    snapshot_vehicle_id,
                })?;
            world.occupy_parking(handle, parking).map_err(|error| {
                SnapshotRestoreError::Parking {
                    snapshot_vehicle_id,
                    error,
                }
            })?;
        }
        VehicleStatus::Completed => {
            world.vehicles[usize::try_from(handle.index()).expect("vehicle index")]
                .state
                .as_mut()
                .expect("spawned vehicle")
                .status = VehicleStatus::Completed;
        }
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
mod tests {
    use super::*;
    use crate::cutover::tests::transaction_tests::world_with_vehicle;
    use crate::{RouteRegisterInput, TickInput, encode_lfrs};

    fn generous_limits() -> SnapshotRestoreLimits {
        SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024)
    }

    #[test]
    fn save_load_restores_exact_logical_state_and_local_id_maps() {
        let (mut original, route, _) = world_with_vehicle(true);
        original.step(TickInput::new(100)).expect("step");
        let profile = laneflow_static_contract::VehicleProfileOrdinal::from_raw(0);
        let second = original
            .spawn_vehicle(VehicleSpawnInput::new(profile, route, 0, 10_000, 0))
            .expect("second far from first");
        original
            .occupy_parking(
                second,
                laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0),
            )
            .expect("park second");
        let snapshot = original.capture_snapshot();
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
            assert_eq!(
                state
                    .parking()
                    .map(|space| { *identity.stable_id(space).expect("parking").as_untyped() }),
                captured.parking_space
            );
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
    fn framing_and_wire_limits_fail_before_flatbuffers_lowering() {
        let (world, _, _) = world_with_vehicle(true);
        let bytes = encode_lfrs(&world.capture_snapshot());
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

        let mut structurally_invalid = encode_lfrs(&world.capture_snapshot());
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

        let mut invalid_clock = world.capture_snapshot();
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
            config.worker_count(),
            config.fixed_delta_time_ms(),
        );
        assert!(matches!(
            restore_lfrs(
                &encode_lfrs(&world.capture_snapshot()),
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

        let mut duplicate_route = world.capture_snapshot();
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

        let mut duplicate_vehicle = world.capture_snapshot();
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
    fn parking_and_live_order_invariants_fail_closed() {
        let (mut world, route, first) = world_with_vehicle(true);
        world
            .occupy_parking(
                first,
                laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0),
            )
            .expect("park first");
        let _second = world
            .spawn_vehicle(VehicleSpawnInput::new(
                laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("second");
        let revision = world.revision();
        let source = world.committed_source().clone();
        let config = world.config();

        let mut parking_mismatch = world.capture_snapshot();
        parking_mismatch.vehicles[0].parking_space = None;
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

        let mut duplicate_live = world.capture_snapshot();
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
    fn vehicle_identity_value_and_overlap_invariants_fail_closed() {
        let (world, _, _) = world_with_vehicle(true);
        let revision = world.revision();
        let source = world.committed_source().clone();
        let config = world.config();

        let mut unknown_profile = world.capture_snapshot();
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

        let mut invalid_carry = world.capture_snapshot();
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

        let mut invalid_completed = world.capture_snapshot();
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

        let mut overlap = world.capture_snapshot();
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
        let valid = encode_lfrs(&world.capture_snapshot());

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

        let mut event_cursor = world.capture_snapshot();
        event_cursor.event_cursor = 7;
        assert_eq!(
            restore_lfrs(
                &encode_lfrs(&event_cursor),
                world.revision(),
                world.committed_source().clone(),
                config,
                generous_limits(),
            )
            .unwrap_err(),
            SnapshotRestoreError::EventCursorNonZero { actual: 7 }
        );
    }

    fn table_field_offset(
        table: laneflow_runtime_snapshot_wire::runtime::Table<'_>,
        field: laneflow_runtime_snapshot_wire::runtime::VOffsetT,
    ) -> usize {
        let relative = usize::from(table.vtable().get(field));
        assert_ne!(relative, 0, "fixture field must be present");
        table.loc() + relative
    }

    #[test]
    fn restored_routes_still_use_common_admitted_compiler() {
        let (mut world, route, _) = world_with_vehicle(true);
        let second = world
            .register_route(RouteRegisterInput::new(
                world.route_edges(route).expect("route").to_vec(),
            ))
            .expect("second route");
        let snapshot = world.capture_snapshot();
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
