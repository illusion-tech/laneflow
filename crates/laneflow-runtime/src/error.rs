use thiserror::Error;

use laneflow_static_contract::{ConflictZoneOrdinal, NetworkRevisionId, ParticipantStreamOrdinal};

use crate::{RouteHandle, VehicleHandle, VehicleReplaceBlock};

/// #284 冲突仲裁能力尚未安装时，候选 Active 车辆仍未用车尾清除的最后 passage。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConflictRuntimeUnavailable {
    route: RouteHandle,
    stream: ParticipantStreamOrdinal,
    passage_local_index: u32,
    zone: ConflictZoneOrdinal,
}

impl ConflictRuntimeUnavailable {
    pub(crate) const fn new(
        route: RouteHandle,
        stream: ParticipantStreamOrdinal,
        passage_local_index: u32,
        zone: ConflictZoneOrdinal,
    ) -> Self {
        Self {
            route,
            stream,
            passage_local_index,
            zone,
        }
    }

    /// 被检查的已注册或 staged 路线句柄。
    #[must_use]
    pub const fn route(self) -> RouteHandle {
        self.route
    }

    /// passage 所属参与者流。
    #[must_use]
    pub const fn stream(self) -> ParticipantStreamOrdinal {
        self.stream
    }

    /// passage 在其参与者流规范 slice 中的 owner-local 下标。
    #[must_use]
    pub const fn passage_local_index(self) -> u32 {
        self.passage_local_index
    }

    /// passage 指向的冲突区。
    #[must_use]
    pub const fn zone(self) -> ConflictZoneOrdinal {
        self.zone
    }
}

/// `TrafficWorld::install` 失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum InstallError {
    /// `fixed_delta_time_ms` 必须落在 `4..=1000`。
    #[error("fixed_delta_time_ms 必须落在 {min}..={max}，实际 {actual}")]
    DeltaOutOfRange {
        /// 调用方提供的步长。
        actual: u64,
        /// 合法下限（含）。
        min: u64,
        /// 合法上限（含）。
        max: u64,
    },
    /// 当前 `TrafficWorld` 只接受 `worker_count == 1`。
    #[error("当前 TrafficWorld 只接受 worker_count == 1")]
    WorkerCountNotOne,
    /// 某个信号 phase 的 `durationMs` 短于固定步长。
    #[error("信号 phase 时长短于 fixed_delta_time_ms")]
    PhaseShorterThanTick,
    /// 某个信号 phase 的 `durationMs` 不能被固定步长整除。
    #[error("信号 phase 时长必须是 fixed_delta_time_ms 的正整数倍")]
    PhaseNotMultipleOfTick,
    /// 信号 controller 的 cycle 非法。
    #[error("信号 controller cycle 必须为正且含 phase")]
    InvalidSignalProgram,
    /// 已提交来源的修订标识与共享根不一致。
    #[error("已提交来源的修订标识与共享根不一致")]
    SourceRevisionMismatch {
        /// 来源指名的修订标识。
        source_revision: NetworkRevisionId,
        /// 共享根 origin 的修订标识。
        installed_revision: NetworkRevisionId,
    },
}

/// `TrafficWorld::step` 失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum StepError {
    /// 调用方 delta 与 world 固定步长不一致。
    #[error("tick delta 与 fixed_delta_time_ms 不一致")]
    DeltaMismatch {
        /// world 配置的固定步长。
        expected_delta_time_ms: u64,
        /// 本次输入的步长。
        actual_delta_time_ms: u64,
    },
    /// `tick_index` 或 `time_ms` 的 checked 加法溢出。
    #[error("tick_index 或 time_ms 溢出")]
    Overflow,
    /// 当前观测 stream 的状态序号无法继续递增。
    #[error("观测状态序号已耗尽")]
    ObservationStateSequenceExhausted,
    /// 本拍运动产生非有限速度或位移。
    #[error("步进运动非有限")]
    NonFiniteMotion,
    /// 占用记录数超过车辆容量、合法车长/最短边或后缀下标编码给出的上限。
    #[error("占用记录数超过规划上限")]
    OccupancyCapacityExceeded,
    /// 占用索引缓冲 `try_reserve` 失败。
    #[error("占用索引分配失败")]
    OccupancyAllocFailed,
    /// Active 车辆占用区间遍历失败（路线下标或边长越界）。
    #[error("占用区间遍历失败")]
    OccupancyIntervalIncomplete,
    /// parking 状态矩阵、资源反向 binding 或 reservation anchor 不闭合。
    #[error("parking runtime aggregate 不变量损坏")]
    ParkingInvariantViolation,
    /// arrival observation 缓冲预留失败。
    #[error("parking arrival observation 分配失败")]
    ParkingObservationAllocFailed,
    /// WaitingZone committed state、membership、queue 或 counter 不闭合。
    #[error("WaitingZone runtime aggregate 不变量损坏")]
    WaitingInvariantViolation,
    /// Waiting claim/decision/transition/event staging scratch 预留失败。
    #[error("WaitingZone tick scratch 分配失败")]
    WaitingScratchAllocFailed,
    /// 某个 zone 的 admission sequence 无法覆盖本拍 successful entries。
    #[error("WaitingZone admission sequence 已耗尽")]
    WaitingAdmissionSequenceExhausted,
}

/// 路线注册或移除失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum RouteError {
    /// 边序列为空。
    #[error("路线边序列不能为空")]
    EmptySequence,
    /// 边序号越出共享根。
    #[error("路线含未知 LaneEdge")]
    UnknownEdge,
    /// 相邻边在共享根车道后继或机动转移中都不连通。
    #[error("路线边序列不连通")]
    Disconnected,
    /// 路线数量达到 world 容量。
    #[error("路线数量达到容量")]
    CapacityExceeded,
    /// 全部存活路线的边出现项总数会超过 world 容量。
    #[error("路线边出现项总数达到容量")]
    EdgeOccurrenceCapacityExceeded,
    /// 全部存活路线的冲突 passage 出现项总数会超过 world 容量。
    #[error("路线冲突出现项总数达到容量: current={current}, added={added}, capacity={capacity}")]
    ConflictOccurrenceCapacityExceeded {
        /// 提交候选路线前的存活冲突出现项数。
        current: u64,
        /// 候选路线将增加的冲突出现项数。
        added: u64,
        /// world 配置的冲突出现项容量。
        capacity: u64,
    },
    /// 路线编译所需缓冲无法预留。
    #[error("路线编译缓冲分配失败")]
    AllocationFailed,
    /// 本次成功路线命令本应推进输入命令游标，但游标已耗尽。
    #[error("输入命令游标已耗尽")]
    CommandCursorExhausted,
    /// 句柄不是本世界有效路线。
    #[error("路线句柄无效或已失效")]
    StaleHandle,
    /// 仍有 live 车辆引用该路线。
    #[error("路线仍被车辆引用")]
    InUse {
        /// 仍引用该路线的车辆。
        vehicle: VehicleHandle,
        /// 被拒绝移除的路线。
        route: RouteHandle,
    },
    /// 剩余边序列同时匹配多条完整机动路径。
    #[error("路线机动路径匹配不唯一")]
    AmbiguousManeuver,
    /// 走到机动入口，但剩余边序列对不上任何一条完整机动路径。
    #[error("路线对不上完整机动路径")]
    ManeuverMismatch,
    /// WaitingZone entry/release 的本地存储跨度不能证明为有限 `u32` 毫米。
    #[error("WaitingZone 本地存储跨度超出有限 u32 毫米范围")]
    WaitingStorageSpanUnbounded,
}

/// `spawn_vehicle` 失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum SpawnError {
    /// 车辆 profile 序号越界。
    #[error("未知车辆 profile")]
    UnknownProfile,
    /// 路线句柄无效。
    #[error("未知或失效路线句柄")]
    UnknownRoute,
    /// 路线序列下标越界。
    #[error("路线序列下标越界")]
    RouteIndexOutOfRange,
    /// 进度超过边长。
    #[error("spawn 进度非法")]
    InvalidProgress,
    /// 初速超过当前边基础限速。
    #[error("spawn 初速超过当前边基础限速")]
    SpeedExceedsLimit,
    /// 车辆数量达到 world 容量。
    #[error("车辆数量达到容量")]
    CapacityExceeded,
    /// `(class, Route)` 后缀准入 deny。
    #[error("路线后缀准入拒绝")]
    AccessDenied,
    /// 与已提交车辆车身重叠。
    #[error("spawn 与已提交车辆重叠")]
    Overlap,
    /// 车型长度超过某个尚未清除 Waiting occurrence 的本地存储跨度。
    #[error("车辆长度超过 WaitingZone 本地存储跨度")]
    WaitingVehicleTooLong,
    /// 调用方试图在无法重建既有 Gate/Waiting authority 的 maneuver interior 生成车辆。
    #[error("不能在 stateful maneuver interior 创建无 Waiting authority 的车辆")]
    WaitingStatefulManeuverInterior,
    /// #284 能力不存在，候选 Active 车辆尚未用车尾清除最后冲突通行段。
    #[error("冲突运行时能力尚不可用: {0:?}")]
    ConflictRuntimeUnavailable(ConflictRuntimeUnavailable),
    /// 本次成功生成本应推进观测状态序号，但序号已耗尽。
    #[error("观测状态序号已耗尽")]
    ObservationStateSequenceExhausted,
    /// 本次成功生成本应推进输入命令游标，但游标已耗尽。
    #[error("输入命令游标已耗尽")]
    CommandCursorExhausted,
}

/// `replace_completed_vehicle` 失败。预检失败时已提交世界不变。
#[derive(Clone, Copy, Debug, PartialEq, Error)]
pub enum ReplaceError {
    /// 句柄无效或已失效。
    #[error("未知或失效车辆句柄")]
    StaleHandle,
    /// 旧车不是 Completed。
    #[error("只能替换 Completed 车辆")]
    NotCompleted,
    /// 旧车仍占用停车位。
    #[error("Completed 车辆仍占用停车位")]
    ParkingOccupied,
    /// Completed 车辆仍携带 maneuver traversal 或 Waiting membership。
    #[error("Completed 车辆携带悬空 Waiting authority")]
    WaitingInvariantViolation,
    /// 车辆 profile 序号越界。
    #[error("未知车辆 profile")]
    UnknownProfile,
    /// 路线句柄无效。
    #[error("未知或失效路线句柄")]
    UnknownRoute,
    /// 路线序列下标越界。
    #[error("路线序列下标越界")]
    RouteIndexOutOfRange,
    /// 进度超过边长。
    #[error("replace 进度非法")]
    InvalidProgress,
    /// 初速超过当前边基础限速。
    #[error("replace 初速超过当前边基础限速")]
    SpeedExceedsLimit,
    /// 车辆数量达到 world 容量。
    #[error("车辆数量达到容量")]
    CapacityExceeded,
    /// `(class, Route)` 后缀准入 deny。
    #[error("路线后缀准入拒绝")]
    AccessDenied,
    /// 入口占用/重叠；可原样重放同一 `VehicleSpawnInput`。
    #[error("入口占用阻塞")]
    Blocked(VehicleReplaceBlock),
    /// 车型长度超过某个尚未清除 Waiting occurrence 的本地存储跨度。
    #[error("车辆长度超过 WaitingZone 本地存储跨度")]
    WaitingVehicleTooLong,
    /// 新 Active 候选落在无法重建既有 Gate/Waiting authority 的 maneuver interior。
    #[error("不能在 stateful maneuver interior 创建无 Waiting authority 的车辆")]
    WaitingStatefulManeuverInterior,
    /// #284 能力不存在，新 Active 车辆尚未用车尾清除最后冲突通行段。
    #[error("冲突运行时能力尚不可用: {0:?}")]
    ConflictRuntimeUnavailable(ConflictRuntimeUnavailable),
    /// 本次成功替换本应推进观测状态序号，但序号已耗尽。
    #[error("观测状态序号已耗尽")]
    ObservationStateSequenceExhausted,
    /// 本次成功替换本应推进输入命令游标，但游标已耗尽。
    #[error("输入命令游标已耗尽")]
    CommandCursorExhausted,
}

/// 停车生命周期命令失败。所有变体都保证已提交世界零副作用。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum ParkingError {
    #[error("未知或失效车辆句柄")]
    StaleVehicle,
    #[error("未知停车位")]
    UnknownSpace,
    #[error("未知停车设施")]
    UnknownFacility,
    #[error("未知车辆 profile")]
    UnknownProfile,
    #[error("未知或失效路线句柄")]
    UnknownRoute,
    #[error("停车 target kind 与命令不匹配")]
    TargetKindMismatch,
    #[error("车辆生命周期状态不允许该停车命令")]
    InvalidVehicleStatus,
    #[error("车辆已绑定其他停车 payload")]
    VehicleAlreadyBound,
    #[error("停车目标已被其他车辆绑定")]
    TargetBoundByOther,
    #[error("虚拟停车容量已耗尽")]
    VirtualCapacityExhausted,
    #[error("虚拟入口 selector 不属于目标设施")]
    EntrySelectorNotOwned,
    #[error("虚拟出口 selector 不属于目标设施")]
    ExitSelectorNotOwned,
    #[error("路线 occurrence 越界")]
    RouteOccurrenceOutOfRange,
    #[error("停车 retained cursor 进度越界")]
    InvalidProgress,
    #[error("路线 occurrence 与停车 anchor 的 LaneEdge 不匹配")]
    RouteOccurrenceAnchorMismatch,
    #[error("停车入口不再前向可达")]
    EntryNotForwardReachable,
    #[error("路线后缀准入拒绝")]
    AccessDenied,
    #[error("车辆长度超过 WaitingZone 本地存储跨度")]
    WaitingVehicleTooLong,
    #[error("不能在 stateful maneuver interior 创建无 Waiting authority 的车辆")]
    WaitingStatefulManeuverInterior,
    #[error("停车 entry 与 Waiting traversal 区间冲突")]
    WaitingTraversalConflict,
    #[error("车辆没有 exact Reserved binding")]
    NotReserved,
    #[error("车辆尚未精确到达停车入口")]
    NotArrived,
    #[error("车辆没有 exact Occupied binding")]
    NotOccupied,
    #[error("rebind current occurrence 与车辆物理 LaneEdge 不匹配")]
    RebindCurrentOccurrenceMismatch,
    #[error("rebind 会改变车辆完整车身占用 footprint")]
    RebindBodyFootprintMismatch,
    #[error("leave 插入与已提交车辆发生物理重叠")]
    LeavePhysicalOverlap { blocker: VehicleHandle },
    #[error("leave 会让移动 direct follower 无法安全制动")]
    LeaveUnsafeFollower { follower: VehicleHandle },
    #[error("冲突运行时能力尚不可用: {0:?}")]
    ConflictRuntimeUnavailable(ConflictRuntimeUnavailable),
    #[error("车辆数量达到容量")]
    VehicleCapacityExceeded,
    #[error("停车稀疏状态分配失败")]
    AllocationFailed,
    #[error("路线引用计数已耗尽")]
    RouteReferenceCapacityExceeded,
    #[error("停车运行时 aggregate 不变量损坏")]
    InvariantViolation,
    #[error("观测状态序号已耗尽")]
    ObservationStateSequenceExhausted,
    #[error("输入命令游标已耗尽")]
    CommandCursorExhausted,
}
