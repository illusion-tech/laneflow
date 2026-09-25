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

/// 车辆进入仿真范围的方向。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntranceDirection {
    /// 沿着这条边的行驶方向进入范围。
    AlongLane,
    /// 逆着这条边的行驶方向。不是合法进入方向。
    AgainstLane,
}

/// 这次命令上的可选开放入口。位置是该路线的起点出现项。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VehicleEntrance {
    route_edge_index: u32,
    direction: EntranceDirection,
}

impl VehicleEntrance {
    /// 构造开放入口绑定。下标和方向在生成或替换时才对照候选路线检查。
    #[must_use]
    pub const fn new(route_edge_index: u32, direction: EntranceDirection) -> Self {
        Self {
            route_edge_index,
            direction,
        }
    }

    /// 开放入口的路线出现项下标。
    #[must_use]
    pub const fn route_edge_index(self) -> u32 {
        self.route_edge_index
    }

    /// 进入范围的方向。
    #[must_use]
    pub const fn direction(self) -> EntranceDirection {
        self.direction
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
    entrance: Option<VehicleEntrance>,
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
            entrance: None,
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

    /// 附上这次命令的开放入口。不改变车型、路线、当前位置、初速或出发声明。
    #[must_use]
    pub const fn with_entrance(mut self, entrance: VehicleEntrance) -> Self {
        self.entrance = Some(entrance);
        self
    }

    /// 这次命令附上的开放入口。没有绑定时是 `None`。
    #[must_use]
    pub const fn entrance(self) -> Option<VehicleEntrance> {
        self.entrance
    }

    /// 把路线起点、沿边顺行绑定为开放入口。
    ///
    /// 只在这条边没有已建模前驱时，才允许省略超出起点的车尾。
    #[must_use]
    pub const fn with_open_entrance(self) -> Self {
        self.with_entrance(VehicleEntrance::new(0, EntranceDirection::AlongLane))
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
