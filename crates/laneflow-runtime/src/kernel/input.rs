use laneflow_static_contract::{LaneEdgeOrdinal, VehicleProfileOrdinal};

use crate::RouteHandle;

/// 路线注册输入：共享根边序号的有序非空序列。
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

    /// 路线的共享根边序号序列。
    #[must_use]
    pub fn edges(&self) -> &[LaneEdgeOrdinal] {
        &self.edges
    }
}

/// 这次命令上的可选出发状态。位置是该路线出现项上的前保险杠。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VehicleDepartureState {
    route_edge_index: u32,
    progress_mm: u32,
    speed_mm_s: u32,
}

impl VehicleDepartureState {
    /// 构造出发状态。下标和进度在生成或替换时才对照候选路线检查。
    #[must_use]
    pub const fn new(route_edge_index: u32, progress_mm: u32, speed_mm_s: u32) -> Self {
        Self {
            route_edge_index,
            progress_mm,
            speed_mm_s,
        }
    }

    /// 出发位置的路线出现项下标。
    #[must_use]
    pub const fn route_edge_index(self) -> u32 {
        self.route_edge_index
    }

    /// 出发出现项上的前保险杠进度（毫米）。
    #[must_use]
    pub const fn progress_mm(self) -> u32 {
        self.progress_mm
    }

    /// 出发速度（毫米/秒）。
    #[must_use]
    pub const fn speed_mm_s(self) -> u32 {
        self.speed_mm_s
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
    departure: Option<VehicleDepartureState>,
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
            departure: None,
        }
    }

    /// 附上这次命令的出发状态。不改变车型、路线、当前位置或初速。
    #[must_use]
    pub const fn with_departure(mut self, departure: VehicleDepartureState) -> Self {
        self.departure = Some(departure);
        self
    }

    /// 这次命令附上的出发状态。没有声明时是 `None`。
    #[must_use]
    pub const fn departure(self) -> Option<VehicleDepartureState> {
        self.departure
    }

    /// 车辆档案（Vehicle Profile）序号。
    #[must_use]
    pub const fn profile(self) -> VehicleProfileOrdinal {
        self.profile
    }

    /// 目标路线句柄。
    #[must_use]
    pub const fn route(self) -> RouteHandle {
        self.route
    }

    /// 起始边在路线边序列中的出现项下标。
    #[must_use]
    pub const fn route_edge_index(self) -> u32 {
        self.route_edge_index
    }

    /// 起始边上的毫米进度。
    #[must_use]
    pub const fn progress_mm(self) -> u32 {
        self.progress_mm
    }

    /// 初始速度（毫米/秒）。
    #[must_use]
    pub const fn initial_speed_mm_s(self) -> u32 {
        self.initial_speed_mm_s
    }
}
