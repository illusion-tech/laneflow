use thiserror::Error;

use crate::{RouteHandle, VehicleHandle};

/// `TrafficWorld::install` 失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum InstallError {
    /// `fixed_delta_time_ms` 必须为正。
    #[error("fixed_delta_time_ms 必须为正")]
    NonPositiveDelta,
    /// 本切片只允许 1-worker 执行计划。
    #[error("本切片只允许 1-worker 执行计划")]
    WorkerCountNotOne,
    /// 某个信号 phase 的 `durationMs` 短于固定步长。
    #[error("信号 phase 时长短于 fixed_delta_time_ms")]
    PhaseShorterThanTick,
    /// 信号 controller 的 cycle 非法。
    #[error("信号 controller cycle 必须为正且含 phase")]
    InvalidSignalProgram,
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
}

/// 静态路线或停车位等共享根序号查找失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum LookupError {
    /// 静态路线序号越界。
    #[error("静态路线序号越界")]
    UnknownStaticRoute,
}

/// 动态路线注册或移除失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum RouteError {
    /// 边序列为空。
    #[error("动态路线边序列不能为空")]
    EmptySequence,
    /// 边序号越出共享根。
    #[error("动态路线含未知 LaneEdge")]
    UnknownEdge,
    /// 相邻边在共享根后继中不连通。
    #[error("动态路线边序列不连通")]
    Disconnected,
    /// 动态路线数量达到 world 容量。
    #[error("动态路线数量达到容量")]
    CapacityExceeded,
    /// 句柄不是本世界有效动态路线。
    #[error("路线句柄无效或已失效")]
    StaleHandle,
    /// 不能移除静态路线。
    #[error("不能移除静态路线")]
    StaticHandle,
    /// 仍有 live 车辆引用该动态路线。
    #[error("动态路线仍被车辆引用")]
    InUse {
        /// 仍引用该路线的车辆。
        vehicle: VehicleHandle,
        /// 被拒绝移除的路线。
        route: RouteHandle,
    },
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
    /// 进度非有限、为负或超过边长。
    #[error("spawn 进度非法")]
    InvalidProgress,
    /// 初速非有限或为负。
    #[error("spawn 初速非法")]
    InvalidSpeed,
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
}

/// `occupy_parking` 失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum ParkingError {
    /// 车辆句柄无效。
    #[error("未知或失效车辆句柄")]
    UnknownVehicle,
    /// 停车位序号越界。
    #[error("未知停车位")]
    UnknownSpace,
    /// 该车已占用别的车位。
    #[error("车辆已占用其他车位")]
    VehicleBoundToOtherSpace,
    /// 目标车位已被其他车占用。
    #[error("停车位已被其他车辆占用")]
    SpaceOccupiedByOther,
}
