//! Core JSON 调试可视化已拆除；走廊/campus gizmos 迁到 Runtime 是 follow-up。

use bevy_app::{App, Plugin};

/// 占位状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LaneFlowDebugGizmosStatus {
    /// 等待。
    #[default]
    WaitingForFrame,
}

/// 占位中心线状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LaneFlowDebugCenterlineStatus {
    /// 空。
    #[default]
    Empty,
}

/// 占位配置。
#[derive(Clone, Copy, Debug, Default)]
pub struct LaneFlowDebugGizmosConfig;

impl LaneFlowDebugGizmosConfig {
    /// 启用占位。
    #[must_use]
    pub const fn enabled(_budget: u32, _segments: u32) -> Self {
        Self
    }
}

/// 占位报告。
#[derive(Clone, Copy, Debug, Default)]
pub struct LaneFlowDebugGizmosReport;

/// 占位中心线。
#[derive(Clone, Debug, Default)]
pub struct LaneFlowDebugCenterlines;

/// 占位车辆过滤。
#[derive(Clone, Copy, Debug, Default)]
pub struct LaneFlowDebugVehicleFilter;

/// 占位 plugin。
#[derive(Clone, Copy, Debug, Default)]
pub struct LaneFlowDebugGizmosPlugin;

impl Plugin for LaneFlowDebugGizmosPlugin {
    fn build(&self, _app: &mut App) {}
}
