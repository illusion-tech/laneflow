//! 已通过 Core domain validation 的初始静态交通输入。

use indexmap::IndexSet;

use crate::{
    error::CoreError, graph::LaneGraph, handle::EdgeHandle, parking::ParkingRegistry,
    profile::VehicleProfileRegistry, route::Route, signal::SignalRegistry,
};

/// 可用于初始化 Core world 的已验证静态交通输入。
#[derive(Clone, Debug, PartialEq)]
pub struct InitialTrafficData {
    lane_graph: LaneGraph,
    routes: Vec<Route>,
    vehicle_profiles: VehicleProfileRegistry,
    signals: SignalRegistry,
    parking: ParkingRegistry,
}

impl InitialTrafficData {
    /// 创建不含 lane graph、route 和 Vehicle Profile 的初始交通输入。
    pub fn empty() -> Self {
        Self {
            lane_graph: LaneGraph::empty(),
            routes: Vec::new(),
            vehicle_profiles: VehicleProfileRegistry::empty(),
            signals: SignalRegistry::empty(),
            parking: ParkingRegistry::empty(),
        }
    }

    /// 创建不含 Signals 的初始 traffic data。
    ///
    /// # Errors
    ///
    /// 初始 route ID 重复、引用未知 edge 或相邻 edge 不连通时返回对应 `CoreError`。
    pub fn try_new<I>(
        lane_graph: LaneGraph,
        routes: I,
        vehicle_profiles: VehicleProfileRegistry,
    ) -> Result<Self, CoreError>
    where
        I: IntoIterator<Item = Route>,
    {
        Self::try_new_with_signals(
            lane_graph,
            routes,
            vehicle_profiles,
            SignalRegistry::empty(),
        )
    }

    /// 创建并校验 lane graph、初始 routes、profile registry 与 static Signals。
    ///
    /// # Errors
    ///
    /// 初始 route ID 重复、引用未知 edge、相邻 edge 不连通或终止在 StopLine edge 时，
    /// 返回对应 `CoreError`。
    pub fn try_new_with_signals<I>(
        lane_graph: LaneGraph,
        routes: I,
        vehicle_profiles: VehicleProfileRegistry,
        signals: SignalRegistry,
    ) -> Result<Self, CoreError>
    where
        I: IntoIterator<Item = Route>,
    {
        Self::try_new_with_signals_and_parking(
            lane_graph,
            routes,
            vehicle_profiles,
            signals,
            ParkingRegistry::empty(),
        )
    }

    /// 创建并校验全部 current static traffic data。
    ///
    /// # Errors
    ///
    /// Signals、Parking 或 route normalization/rebind 失败时返回对应 `CoreError`。
    pub fn try_new_with_signals_and_parking<I>(
        lane_graph: LaneGraph,
        routes: I,
        vehicle_profiles: VehicleProfileRegistry,
        signals: SignalRegistry,
        parking: ParkingRegistry,
    ) -> Result<Self, CoreError>
    where
        I: IntoIterator<Item = Route>,
    {
        let signals = signals.rebind_to_lane_graph(&lane_graph)?;
        let parking = parking.rebind_to_lane_graph(&lane_graph)?;
        let mut route_ids = IndexSet::new();
        let mut validated_routes = Vec::new();

        for route in routes {
            if !route_ids.insert(route.id().to_owned()) {
                return Err(CoreError::DuplicateRouteId {
                    route_id: route.id().to_owned(),
                });
            }
            resolve_route_edges(&lane_graph, &signals, &route)?;
            validated_routes.push(route);
        }

        Ok(Self {
            lane_graph,
            routes: validated_routes,
            vehicle_profiles,
            signals,
            parking,
        })
    }

    /// 返回已验证的 lane graph。
    pub const fn lane_graph(&self) -> &LaneGraph {
        &self.lane_graph
    }

    /// 返回初始 route definitions，保持输入顺序。
    pub fn routes(&self) -> &[Route] {
        &self.routes
    }

    /// 返回 immutable Vehicle Profile registry。
    pub const fn vehicle_profiles(&self) -> &VehicleProfileRegistry {
        &self.vehicle_profiles
    }

    /// 返回 immutable Signals registry。
    pub const fn signals(&self) -> &SignalRegistry {
        &self.signals
    }

    /// 返回 immutable Parking registry。
    pub const fn parking(&self) -> &ParkingRegistry {
        &self.parking
    }

    /// 拆分为 Core-owned parts。
    pub fn into_parts(
        self,
    ) -> (
        LaneGraph,
        Vec<Route>,
        VehicleProfileRegistry,
        SignalRegistry,
        ParkingRegistry,
    ) {
        (
            self.lane_graph,
            self.routes,
            self.vehicle_profiles,
            self.signals,
            self.parking,
        )
    }
}

pub(crate) fn resolve_route_edges(
    lane_graph: &LaneGraph,
    signals: &SignalRegistry,
    route: &Route,
) -> Result<Vec<EdgeHandle>, CoreError> {
    let mut edge_handles = Vec::with_capacity(route.edge_ids().len());
    for edge_id in route.edge_ids() {
        let edge = lane_graph
            .edge_handle(edge_id)
            .ok_or_else(|| CoreError::UnknownRouteEdge {
                route_id: route.id().to_owned(),
                edge_id: edge_id.clone(),
            })?;
        edge_handles.push(edge);
    }

    for [from_edge, to_edge] in edge_handles.array_windows::<2>() {
        if !lane_graph.can_traverse(*from_edge, *to_edge) {
            return Err(CoreError::DisconnectedRouteEdge {
                route_id: route.id().to_owned(),
                from_edge_id: lane_graph
                    .edge_external_id(*from_edge)
                    .expect("resolved route edge must exist")
                    .to_owned(),
                to_edge_id: lane_graph
                    .edge_external_id(*to_edge)
                    .expect("resolved route edge must exist")
                    .to_owned(),
            });
        }
    }

    let final_edge = *edge_handles
        .last()
        .expect("Route constructor guarantees at least one edge");
    if let Some(stop_line) = signals.stop_line_for_edge(final_edge) {
        return Err(CoreError::RouteTerminatesAtStopLine {
            route_id: route.id().to_owned(),
            edge_id: lane_graph
                .edge_external_id(final_edge)
                .expect("resolved route edge must exist")
                .to_owned(),
            stop_line_id: signals
                .stop_line_external_id(stop_line)
                .expect("resolved StopLine handle must exist")
                .to_owned(),
        });
    }

    Ok(edge_handles)
}
