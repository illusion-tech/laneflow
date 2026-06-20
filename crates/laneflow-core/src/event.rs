//! Core step 输出事件。

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
    /// 车辆 id。
    pub vehicle_id: String,
    /// route id。
    pub route_id: String,
    /// 切换前的 lane edge id。
    pub from_edge_id: String,
    /// 切换后的 lane edge id。
    pub to_edge_id: String,
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
    /// 车辆 id。
    pub vehicle_id: String,
    /// route id。
    pub route_id: String,
    /// 完成时所在的 lane edge id。
    pub edge_id: String,
    /// 完成时所在的 route edge index。
    pub route_edge_index: usize,
}
