//! 快照恢复的公开上限、错误与完整新世界结果；原始字节只交给格式准入模块。

use crate::{
    AdmittedRouteRegisterError, CommittedNetworkSource, InstallError, ParkingError, RouteHandle,
    SpawnError, StepError, TrafficWorld, VehicleHandle, WorldConfig,
};
use laneflow_static_network::SharedNetworkRevision;
use std::sync::Arc;
use thiserror::Error;

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
    pub(super) world: TrafficWorld,
    pub(super) routes: Vec<(u64, RouteHandle)>,
    pub(super) vehicles: Vec<(u64, VehicleHandle)>,
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
    super::format_admission::restore_lfrs(bytes, revision, source, target_config, limits)
}
