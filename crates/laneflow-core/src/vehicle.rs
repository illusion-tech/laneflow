//! Vehicle 输入与 runtime state 原语。

use crate::{
    error::{CoreError, NumericConversionStage},
    graph::EdgeLength,
    handle::{RouteHandle, VehicleHandle, VehicleProfileHandle},
    numeric_policy::MAX_SPEED_INCLUSIVE_METERS_PER_SECOND,
    parking::ParkingReleaseRecord,
};

/// 车辆速度，单位为 meter/second。
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Speed(f32);

impl Speed {
    /// 零速度。
    pub const ZERO: Self = Self(0.0);

    /// 创建经过校验的速度。
    pub fn try_new(value: f32) -> Result<Self, CoreError> {
        if !value.is_finite() || !(0.0..=MAX_SPEED_INCLUSIVE_METERS_PER_SECOND).contains(&value) {
            return Err(CoreError::InvalidSpeed { speed: value });
        }

        Ok(Self(canonicalize_zero_f32(value)))
    }

    /// 返回底层数值。
    pub const fn value(self) -> f32 {
        self.0
    }
}

impl TryFrom<f64> for Speed {
    type Error = CoreError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        let max_inclusive = f64::from(MAX_SPEED_INCLUSIVE_METERS_PER_SECOND);
        if !value.is_finite() || !(0.0..=max_inclusive).contains(&value) {
            return Err(CoreError::InvalidSpeedInput {
                speed: value,
                stage: NumericConversionStage::RawInput,
            });
        }
        Self::try_new(value as f32).map_err(|_| CoreError::InvalidSpeedInput {
            speed: value,
            stage: NumericConversionStage::TargetValue,
        })
    }
}

/// 车辆在当前 tick 实际应用的纵向加速度，单位为 meter/second^2。
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Acceleration(f32);

impl Acceleration {
    /// 零加速度。
    pub const ZERO: Self = Self(0.0);

    /// 创建经过校验的有符号加速度。
    pub fn try_new(value: f32) -> Result<Self, CoreError> {
        if !value.is_finite() {
            return Err(CoreError::InvalidAcceleration {
                acceleration: value,
            });
        }

        Ok(Self(canonicalize_zero_f32(value)))
    }

    /// 返回底层数值。
    pub const fn value(self) -> f32 {
        self.0
    }
}

impl TryFrom<f64> for Acceleration {
    type Error = CoreError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        if !value.is_finite() {
            return Err(CoreError::InvalidAccelerationInput {
                acceleration: value,
                stage: NumericConversionStage::RawInput,
            });
        }
        Self::try_new(value as f32).map_err(|_| CoreError::InvalidAccelerationInput {
            acceleration: value,
            stage: NumericConversionStage::TargetValue,
        })
    }
}

/// 车辆前保险杠在当前 route edge 内的 progress，单位为 meter。
#[derive(Clone, Copy, Debug)]
pub struct EdgeProgress {
    value: f64,
}

impl EdgeProgress {
    /// 零 progress。
    pub const ZERO: Self = Self { value: 0.0 };

    /// 创建经过校验的 edge progress。
    pub fn try_new(value: f64) -> Result<Self, CoreError> {
        if !value.is_finite() || value < 0.0 {
            return Err(CoreError::InvalidEdgeProgress {
                edge_progress: value,
            });
        }

        Ok(Self {
            value: canonicalize_zero_f64(value),
        })
    }

    /// 返回底层数值。
    pub fn value(self) -> f64 {
        self.value
    }

    /// 使用目标 `f32` 行程推进，并在进度权威中精确提升该增量。
    pub(crate) fn advance(self, travel: f32) -> Result<Self, CoreError> {
        if !travel.is_finite() || travel < 0.0 {
            return Err(CoreError::InvalidEdgeProgress {
                edge_progress: self.value() + f64::from(travel),
            });
        }
        Self::try_new(self.value + f64::from(travel))
    }

    /// 跨 edge 后减去规范化 edge length，保留局部进度权威。
    pub(crate) fn rebase_after_edge(self, edge_length: EdgeLength) -> Result<Self, CoreError> {
        Self::try_new(self.value - f64::from(edge_length.value()))
    }
}

impl PartialEq for EdgeProgress {
    fn eq(&self, other: &Self) -> bool {
        self.value() == other.value()
    }
}

impl PartialOrd for EdgeProgress {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.value().partial_cmp(&other.value())
    }
}

fn canonicalize_zero_f32(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}

fn canonicalize_zero_f64(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

/// 车辆运行状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VehicleStatus {
    /// 随 fixed tick 沿 route 推进。
    Active,
    /// 手工或初始保持停止，当前最小实现不因前车或信号自动进入该状态。
    Stopped,
    /// route 结束后的终止状态。
    Completed,
    /// 由 Parking lifecycle command 持有 off-lane position authority。
    Parked,
}

/// 创建或初始化 vehicle 时使用的外部输入。
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleSpawnInput {
    /// vehicle external ID。
    pub id: String,
    /// immutable Vehicle Profile handle。
    pub profile: VehicleProfileHandle,
    /// route external ID。
    pub route_id: String,
    /// 当前 route edge index。
    pub route_edge_index: usize,
    /// 车辆前保险杠在当前 edge 内的 progress。
    pub edge_progress: EdgeProgress,
    /// 初始当前速度。
    pub initial_speed: Speed,
    /// 车辆运行状态。
    pub status: VehicleStatus,
}

impl VehicleSpawnInput {
    /// 创建指定状态的 vehicle 输入。
    pub fn new(
        id: impl Into<String>,
        profile: VehicleProfileHandle,
        route_id: impl Into<String>,
        route_edge_index: usize,
        edge_progress: EdgeProgress,
        initial_speed: Speed,
        status: VehicleStatus,
    ) -> Self {
        Self {
            id: id.into(),
            profile,
            route_id: route_id.into(),
            route_edge_index,
            edge_progress,
            initial_speed,
            status,
        }
    }

    /// 创建 active vehicle 输入。
    pub fn active(
        id: impl Into<String>,
        profile: VehicleProfileHandle,
        route_id: impl Into<String>,
        route_edge_index: usize,
        edge_progress: EdgeProgress,
        initial_speed: Speed,
    ) -> Self {
        Self::new(
            id,
            profile,
            route_id,
            route_edge_index,
            edge_progress,
            initial_speed,
            VehicleStatus::Active,
        )
    }

    /// 创建 stopped vehicle 输入，其初始运动状态固定为零。
    pub fn stopped(
        id: impl Into<String>,
        profile: VehicleProfileHandle,
        route_id: impl Into<String>,
        route_edge_index: usize,
        edge_progress: EdgeProgress,
    ) -> Self {
        Self::new(
            id,
            profile,
            route_id,
            route_edge_index,
            edge_progress,
            Speed::ZERO,
            VehicleStatus::Stopped,
        )
    }

    /// 创建 completed vehicle 输入，其初始运动状态固定为零。
    pub fn completed(
        id: impl Into<String>,
        profile: VehicleProfileHandle,
        route_id: impl Into<String>,
        route_edge_index: usize,
        edge_progress: EdgeProgress,
    ) -> Self {
        Self::new(
            id,
            profile,
            route_id,
            route_edge_index,
            edge_progress,
            Speed::ZERO,
            VehicleStatus::Completed,
        )
    }
}

/// Core runtime 中的 vehicle 状态。
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleState {
    /// vehicle runtime handle。
    pub handle: VehicleHandle,
    /// immutable Vehicle Profile handle。
    pub profile: VehicleProfileHandle,
    /// 当前 route handle。
    pub route: RouteHandle,
    /// 当前 route edge index。
    pub route_edge_index: usize,
    /// 车辆前保险杠在当前 edge 内的 progress。
    pub edge_progress: EdgeProgress,
    /// 当前纵向速度。
    pub current_speed: Speed,
    /// 当前 tick 实际应用的有符号纵向加速度。
    pub applied_acceleration: Acceleration,
    /// 车辆运行状态。
    pub status: VehicleStatus,
}

impl VehicleState {
    pub(crate) fn new(
        handle: VehicleHandle,
        profile: VehicleProfileHandle,
        route: RouteHandle,
        route_edge_index: usize,
        edge_progress: EdgeProgress,
        current_speed: Speed,
        status: VehicleStatus,
    ) -> Self {
        Self {
            handle,
            profile,
            route,
            route_edge_index,
            edge_progress,
            current_speed,
            applied_acceleration: Acceleration::ZERO,
            status,
        }
    }
}

/// vehicle 被移除时返回的生命周期记录。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VehicleDespawnRecord {
    /// 被移除的 vehicle handle。
    pub handle: VehicleHandle,
    /// 被移除的 vehicle external ID。
    pub external_id: String,
    /// 移除时绑定的 immutable Vehicle Profile handle。
    pub profile: VehicleProfileHandle,
    /// 移除时绑定的 route handle。
    pub route: RouteHandle,
    /// 移除时的 vehicle 状态。
    pub status: VehicleStatus,
    /// Despawn 同步释放的 Parking binding。
    pub parking_release: Option<ParkingReleaseRecord>,
}
