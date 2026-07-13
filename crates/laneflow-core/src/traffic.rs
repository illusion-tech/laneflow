//! 已通过 Core domain validation 的初始静态交通输入。

use indexmap::IndexSet;

use crate::{
    error::CoreError, graph::LaneGraph, handle::EdgeHandle, profile::VehicleProfileRegistry,
    route::Route,
};

/// 可用于初始化 Core world 的已验证静态交通输入。
#[derive(Clone, Debug, PartialEq)]
pub struct InitialTrafficData {
    lane_graph: LaneGraph,
    routes: Vec<Route>,
    vehicle_profiles: VehicleProfileRegistry,
}

impl InitialTrafficData {
    /// 创建不含 lane graph、route 和 Vehicle Profile 的初始交通输入。
    pub fn empty() -> Self {
        Self {
            lane_graph: LaneGraph::empty(),
            routes: Vec::new(),
            vehicle_profiles: VehicleProfileRegistry::empty(),
        }
    }

    /// 创建并校验 lane graph、初始 routes 与 immutable profile registry。
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
        let mut route_ids = IndexSet::new();
        let mut validated_routes = Vec::new();

        for route in routes {
            if !route_ids.insert(route.id().to_owned()) {
                return Err(CoreError::DuplicateRouteId {
                    route_id: route.id().to_owned(),
                });
            }
            resolve_route_edges(&lane_graph, &route)?;
            validated_routes.push(route);
        }

        Ok(Self {
            lane_graph,
            routes: validated_routes,
            vehicle_profiles,
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

    /// 拆分为 Core-owned parts。
    pub fn into_parts(self) -> (LaneGraph, Vec<Route>, VehicleProfileRegistry) {
        (self.lane_graph, self.routes, self.vehicle_profiles)
    }
}

pub(crate) fn resolve_route_edges(
    lane_graph: &LaneGraph,
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

    Ok(edge_handles)
}
