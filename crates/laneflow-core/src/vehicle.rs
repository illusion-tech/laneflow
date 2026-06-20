//! v0.1 最小 vehicle state 原语。

use crate::error::CoreError;

/// v0.1 车辆运行状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VehicleStatus {
    /// 随 fixed tick 沿 route 推进。
    Active,
    /// 手工或初始保持停止，v0.1 不因前车或信号自动进入该状态。
    Stopped,
    /// route 结束后的终止状态。
    Completed,
}

/// v0.1 最小车辆状态。
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleState {
    /// vehicle id，在 v0.1 内由调用方保证语义。
    pub id: String,
    /// route id；route 存在性由后续 lane graph / route issue 校验。
    pub route_id: String,
    /// 当前 route edge index。
    pub route_edge_index: usize,
    /// 当前 edge 内 progress。
    pub edge_progress: f64,
    /// 车辆配置或当前期望速度。
    pub speed: f64,
    /// 车辆运行状态。
    pub status: VehicleStatus,
}

impl VehicleState {
    /// 创建指定状态的最小车辆状态。
    pub fn new(
        id: impl Into<String>,
        route_id: impl Into<String>,
        route_edge_index: usize,
        edge_progress: f64,
        speed: f64,
        status: VehicleStatus,
    ) -> Self {
        Self {
            id: id.into(),
            route_id: route_id.into(),
            route_edge_index,
            edge_progress,
            speed,
            status,
        }
    }

    /// 创建 active 车辆。
    pub fn active(
        id: impl Into<String>,
        route_id: impl Into<String>,
        route_edge_index: usize,
        edge_progress: f64,
        speed: f64,
    ) -> Self {
        Self::new(
            id,
            route_id,
            route_edge_index,
            edge_progress,
            speed,
            VehicleStatus::Active,
        )
    }

    /// 创建 stopped 车辆。
    pub fn stopped(
        id: impl Into<String>,
        route_id: impl Into<String>,
        route_edge_index: usize,
        edge_progress: f64,
        speed: f64,
    ) -> Self {
        Self::new(
            id,
            route_id,
            route_edge_index,
            edge_progress,
            speed,
            VehicleStatus::Stopped,
        )
    }

    /// 创建 completed 车辆。
    pub fn completed(
        id: impl Into<String>,
        route_id: impl Into<String>,
        route_edge_index: usize,
        edge_progress: f64,
        speed: f64,
    ) -> Self {
        Self::new(
            id,
            route_id,
            route_edge_index,
            edge_progress,
            speed,
            VehicleStatus::Completed,
        )
    }

    /// 返回当前 step 使用的有效速度。
    pub fn effective_speed(&self) -> f64 {
        match self.status {
            VehicleStatus::Active => self.speed,
            VehicleStatus::Stopped | VehicleStatus::Completed => 0.0,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), CoreError> {
        if !self.speed.is_finite() || self.speed < 0.0 {
            return Err(CoreError::InvalidVehicleSpeed {
                vehicle_id: self.id.clone(),
                speed: self.speed,
            });
        }

        if !self.edge_progress.is_finite() || self.edge_progress < 0.0 {
            return Err(CoreError::InvalidVehicleEdgeProgress {
                vehicle_id: self.id.clone(),
                edge_progress: self.edge_progress,
            });
        }

        Ok(())
    }
}
