use thiserror::Error;

use laneflow_static_contract::NetworkRevisionId;

use crate::{RouteHandle, VehicleHandle, VehicleReplaceBlock};

/// `TrafficWorld::install` 失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum InstallError {
    /// 共享根含门、冲突区或参与者流，必须显式固定路权策略（`install`；跨修订
    /// cutover 以 `PolicyInstall` 内嵌同样可达）。
    #[error("共享根含门、冲突区或参与者流，必须显式固定路权策略")]
    PolicyRequired,
    /// 共享根中不存在指定的路权策略（`install` 的策略绑定解析；跨修订 cutover 以
    /// `PolicyInstall` 内嵌同样可达）。
    #[error("共享根中不存在指定路权策略 {policy:?}")]
    UnknownPolicy {
        policy: laneflow_static_contract::RightOfWayPolicySetId,
    },
    /// 策略间隙参数的步长派生超出可移植毫秒值域（`install`；跨修订 cutover 以
    /// `PolicyInstall` 内嵌同样可达）。
    #[error("策略间隙参数 {gap_profile_index} 的步长派生超出可移植毫秒值域")]
    PolicyGapOverflow { gap_profile_index: u32 },
    /// 世界策略派生表容量算术溢出（`install`；跨修订 cutover 以 `PolicyInstall` 内嵌
    /// 同样可达）。
    #[error("世界策略派生表容量算术溢出")]
    PolicyCapacityOverflow,
    /// 世界策略派生表分配失败（`install`；恢复路径与跨修订 cutover 的 `PolicyInstall`
    /// 内嵌同样可达）。
    #[error("世界策略派生表分配失败")]
    PolicyAllocationFailed,
    /// 冲突仲裁状态容量超出当前平台（`install`）。
    #[error("冲突仲裁状态容量超出当前平台")]
    ConflictArbiterCapacityOverflow,
    /// 冲突仲裁状态分配失败（`install`；恢复路径同样可达）。
    #[error("冲突仲裁状态分配失败")]
    ConflictArbiterAllocationFailed,
    /// 共享路网的冲突通行关系不满足仲裁器安装不变量（`install`）。
    #[error("共享路网中的冲突通行关系不满足仲裁器安装不变量")]
    ConflictArbiterInvalidNetwork,
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
    /// Conflict 候选、claim、decision 或 transition staging scratch 预留失败。
    #[error("Conflict tick scratch 分配失败")]
    ConflictScratchAllocFailed,
    /// 某个 zone 的 admission sequence 无法覆盖本拍 successful entries。
    #[error("WaitingZone admission sequence 已耗尽")]
    WaitingAdmissionSequenceExhausted,
    /// completion、traversal 或 ledger 的 Conflict authority 不闭合。
    #[error("Conflict runtime aggregate 不变量损坏")]
    ConflictInvariantViolation,
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
    /// 调用方试图在已越过 Conflict Gate、但车尾尚未清空 coverage 的位置
    /// 创建没有既有 Conflict authority 的车辆。
    #[error("不能在冲突通行段内部创建无 Conflict authority 的车辆")]
    ConflictAuthorityRequired,
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
    /// Completed 车辆仍携带冲突 reservation/downstream authority。
    #[error("Completed 车辆携带悬空 Conflict authority")]
    ConflictInvariantViolation,
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
    /// 新 Active 候选已越过 Conflict Gate、但车尾尚未清空 coverage，且没有
    /// 可继承的 Conflict authority。
    #[error("不能在冲突通行段内部创建无 Conflict authority 的车辆")]
    ConflictAuthorityRequired,
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
    /// 车辆句柄不属于当前世界或已失效；全部带车辆句柄的停车入口共用。
    #[error("未知或失效车辆句柄")]
    StaleVehicle,
    /// 泊位目标在当前修订中不存在（目标解析阶段）。
    #[error("未知停车位")]
    UnknownSpace,
    /// 停车设施目标在当前修订中不存在（目标解析阶段）。
    #[error("未知停车设施")]
    UnknownFacility,
    /// 车辆 profile 序号越出共享根（`spawn_parked_vehicle`）。
    #[error("未知车辆 profile")]
    UnknownProfile,
    /// 路线句柄无效或不属于当前世界（reserve/leave/rebind/spawn_parked 的路线解析）。
    #[error("未知或失效路线句柄")]
    UnknownRoute,
    /// 停车 target 种类与命令种类不匹配；当前无构造点，防御性保留。
    #[error("停车 target kind 与命令不匹配")]
    TargetKindMismatch,
    /// 车辆生命周期状态不允许该命令（reserve/cancel/leave/rebind 的状态前置校验）。
    #[error("车辆生命周期状态不允许该停车命令")]
    InvalidVehicleStatus,
    /// 车辆已绑定其它停车 payload（`reserve_parking`）。
    #[error("车辆已绑定其他停车 payload")]
    VehicleAlreadyBound,
    /// 显式泊位已被其它车辆绑定（reserve/spawn_parked 的目标可用性检查）。
    #[error("停车目标已被其他车辆绑定")]
    TargetBoundByOther,
    /// 虚拟池容量已耗尽（reserve/spawn_parked 的目标可用性检查）。
    #[error("虚拟停车容量已耗尽")]
    VirtualCapacityExhausted,
    /// 虚拟池入口 selector 不属于目标设施（reserve/rebind 的入口锚解析）。
    #[error("虚拟入口 selector 不属于目标设施")]
    EntrySelectorNotOwned,
    /// 虚拟池出口 selector 不属于目标设施（`leave_parking` 的出口锚解析）。
    #[error("虚拟出口 selector 不属于目标设施")]
    ExitSelectorNotOwned,
    /// 路线出现项下标越界（reserve/leave/rebind/spawn_parked 的出现项解析）。
    #[error("路线 occurrence 越界")]
    RouteOccurrenceOutOfRange,
    /// 停车锚点进度越界（`spawn_parked_vehicle`）。
    #[error("停车 retained cursor 进度越界")]
    InvalidProgress,
    /// 停车锚与路线 occurrence 的 LaneEdge 不一致（reserve/leave/rebind 的锚点-occurrence 校验）。
    #[error("路线 occurrence 与停车 anchor 的 LaneEdge 不匹配")]
    RouteOccurrenceAnchorMismatch,
    /// 停车入口已被越过——位于车辆当前出现项/进度之前，或同进度但 carry 非零、
    /// 不可前向到达（reserve/rebind 的入口可达性校验）。
    #[error("停车入口不再前向可达")]
    EntryNotForwardReachable,
    /// 路线后缀准入策略拒绝（reserve/leave/rebind/spawn_parked）。
    #[error("路线后缀准入拒绝")]
    AccessDenied,
    /// 车型长度超过 WaitingZone 本地存储跨度（leave/rebind 的等待区校验）。
    #[error("车辆长度超过 WaitingZone 本地存储跨度")]
    WaitingVehicleTooLong,
    /// 需在 stateful maneuver interior 建立无既有 Waiting authority 的状态；当前无构造点
    /// （内部映射为 `WaitingTraversalConflict`），防御性保留。
    #[error("不能在 stateful maneuver interior 创建无 Waiting authority 的车辆")]
    WaitingStatefulManeuverInterior,
    /// 停车 entry 与既有 Waiting/maneuver traversal 状态冲突：含区间重叠、stateful
    /// maneuver interior、权威不匹配及到站车辆仍有遍历/等待成员等映射路径
    /// （reserve/park/leave/rebind）。
    #[error("停车 entry 与 Waiting traversal 区间冲突")]
    WaitingTraversalConflict,
    /// 车辆没有 exact Reserved binding（cancel/park/rebind）。
    #[error("车辆没有 exact Reserved binding")]
    NotReserved,
    /// 车辆尚未精确到达停车入口（`park_vehicle`）。
    #[error("车辆尚未精确到达停车入口")]
    NotArrived,
    /// 车辆没有 exact Occupied binding（`leave_parking`）。
    #[error("车辆没有 exact Occupied binding")]
    NotOccupied,
    /// 存在在途 Conflict authority，不能完成该生命周期转换（park/rebind）。
    #[error("active Conflict authority 期间不能完成该停车生命周期转换")]
    ConflictTraversalActive,
    /// rebind 的当前出现项与车辆已提交物理 LaneEdge 不一致（`rebind_parking_route`）。
    #[error("rebind current occurrence 与车辆物理 LaneEdge 不匹配")]
    RebindCurrentOccurrenceMismatch,
    /// rebind 会改变车辆完整车身占用 footprint（`rebind_parking_route`）。
    #[error("rebind 会改变车辆完整车身占用 footprint")]
    RebindBodyFootprintMismatch,
    /// leave 出口 anchor 与已提交车辆物理重叠（`leave_parking`）。
    #[error("leave 插入与已提交车辆发生物理重叠")]
    LeavePhysicalOverlap { blocker: VehicleHandle },
    /// leave 会让移动 direct follower 无法安全制动（`leave_parking`）。
    #[error("leave 会让移动 direct follower 无法安全制动")]
    LeaveUnsafeFollower { follower: VehicleHandle },
    /// 需要既有 Conflict authority 才能建立该状态（leave/rebind 的权威校验）。
    #[error("不能在冲突通行段内部恢复无 Conflict authority 的 Active 车辆")]
    ConflictAuthorityRequired,
    /// 车辆数量达到 world 容量（`spawn_parked_vehicle`）。
    #[error("车辆数量达到容量")]
    VehicleCapacityExceeded,
    /// 停车稀疏状态或派生缓冲分配失败（reserve/leave/rebind/spawn_parked）。
    #[error("停车稀疏状态分配失败")]
    AllocationFailed,
    /// 路线引用计数耗尽（提交路径的路线引用递增）。
    #[error("路线引用计数已耗尽")]
    RouteReferenceCapacityExceeded,
    /// 停车运行时 aggregate 不变量损坏；属内部防御，正常输入不应触达。
    #[error("停车运行时 aggregate 不变量损坏")]
    InvariantViolation,
    /// 提交路径需要推进观测状态序号但序号已耗尽（park/leave/despawn 的 Active 转换
    /// 提交）。
    #[error("观测状态序号已耗尽")]
    ObservationStateSequenceExhausted,
    /// 提交路径需要推进输入命令游标但游标已耗尽（全部停车入口）。
    #[error("输入命令游标已耗尽")]
    CommandCursorExhausted,
}
