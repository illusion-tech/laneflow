use laneflow_static_contract::{LaneEdgeOrdinal, VehicleProfileOrdinal};

use crate::RouteHandle;

/// 动态路线注册输入：共享根边序号的有序非空序列。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteRegisterInput {
    edges: Box<[LaneEdgeOrdinal]>,
}

impl RouteRegisterInput {
    /// 从边序号序列构造。空序列在 `register_route` 失败。
    #[must_use]
    pub fn new(edges: impl Into<Vec<LaneEdgeOrdinal>>) -> Self {
        Self {
            edges: edges.into().into_boxed_slice(),
        }
    }

    #[must_use]
    pub fn edges(&self) -> &[LaneEdgeOrdinal] {
        &self.edges
    }
}

/// 调用方所有的车辆生成输入。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VehicleSpawnInput {
    profile: VehicleProfileOrdinal,
    route: RouteHandle,
    route_edge_index: u32,
    progress_mm: u32,
    initial_speed_mm_s: u32,
}

impl VehicleSpawnInput {
    /// 构造 spawn 输入。下标是该 `RouteHandle` 序列上的 occurrence 位置。
    #[must_use]
    pub const fn new(
        profile: VehicleProfileOrdinal,
        route: RouteHandle,
        route_edge_index: u32,
        progress_mm: u32,
        initial_speed_mm_s: u32,
    ) -> Self {
        Self {
            profile,
            route,
            route_edge_index,
            progress_mm,
            initial_speed_mm_s,
        }
    }

    #[must_use]
    pub const fn profile(self) -> VehicleProfileOrdinal {
        self.profile
    }

    #[must_use]
    pub const fn route(self) -> RouteHandle {
        self.route
    }

    #[must_use]
    pub const fn route_edge_index(self) -> u32 {
        self.route_edge_index
    }

    #[must_use]
    pub const fn progress_mm(self) -> u32 {
        self.progress_mm
    }

    #[must_use]
    pub const fn initial_speed_mm_s(self) -> u32 {
        self.initial_speed_mm_s
    }
}
