//! Core step 输出事件。

use crate::{EdgeHandle, RouteHandle, VehicleHandle};

/// Core step 产生的可观察事件。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CoreEvent {
    /// 车辆从 route 中的一个 edge 切换到下一个 edge。
    VehicleChangedEdge(VehicleChangedEdgeEvent),
    /// 车辆到达 route 末端。
    VehicleCompletedRoute(VehicleCompletedRouteEvent),
}

/// 车辆跨 edge 事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VehicleChangedEdgeEvent {
    /// 事件所属的 post-step tick index。
    pub tick_index: u64,
    /// 车辆 handle。
    pub vehicle: VehicleHandle,
    /// route handle。
    pub route: RouteHandle,
    /// 切换前的 lane edge handle。
    pub from_edge: EdgeHandle,
    /// 切换后的 lane edge handle。
    pub to_edge: EdgeHandle,
    /// 切换前的 route edge index。
    pub from_route_edge_index: usize,
    /// 切换后的 route edge index。
    pub to_route_edge_index: usize,
}

/// 车辆完成 route 事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VehicleCompletedRouteEvent {
    /// 事件所属的 post-step tick index。
    pub tick_index: u64,
    /// 车辆 handle。
    pub vehicle: VehicleHandle,
    /// route handle。
    pub route: RouteHandle,
    /// 完成时所在的 lane edge handle。
    pub edge: EdgeHandle,
    /// 完成时所在的 route edge index。
    pub route_edge_index: usize,
}
