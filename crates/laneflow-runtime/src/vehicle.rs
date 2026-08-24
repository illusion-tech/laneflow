use laneflow_static_contract::{
    ParkingSpaceOrdinal, ParticipantClassOrdinal, VehicleProfileOrdinal,
};

use crate::{RouteHandle, VehicleHandle};

/// 已提交车辆生命周期状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VehicleStatus {
    /// 在路线上参与占用与步进。
    Active,
    /// 占用停车位，不参与车道占用。
    Parked,
    /// 已到路线终点：句柄仍 live，不进 pose、不步进、不占车道；占车辆容量，直到原子替换。
    Completed,
}

/// 已提交车辆快照。`Completed` 对调用方可读。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VehicleState {
    pub(crate) handle: VehicleHandle,
    pub(crate) profile: VehicleProfileOrdinal,
    pub(crate) class: ParticipantClassOrdinal,
    pub(crate) route: RouteHandle,
    pub(crate) route_edge_index: u32,
    pub(crate) progress: f64,
    pub(crate) speed: f64,
    pub(crate) length: f64,
    pub(crate) status: VehicleStatus,
    pub(crate) parking: Option<ParkingSpaceOrdinal>,
}

impl VehicleState {
    /// 该状态对应的句柄。
    #[must_use]
    pub const fn handle(self) -> VehicleHandle {
        self.handle
    }

    /// 车辆 profile。
    #[must_use]
    pub const fn profile(self) -> VehicleProfileOrdinal {
        self.profile
    }

    /// 参与者类别。
    #[must_use]
    pub const fn class(self) -> ParticipantClassOrdinal {
        self.class
    }

    /// 当前路线句柄。
    #[must_use]
    pub const fn route(self) -> RouteHandle {
        self.route
    }

    /// 路线序列下标。
    #[must_use]
    pub const fn route_edge_index(self) -> u32 {
        self.route_edge_index
    }

    /// 当前边进度。
    #[must_use]
    pub const fn progress(self) -> f64 {
        self.progress
    }

    /// 当前速度。
    #[must_use]
    pub const fn speed(self) -> f64 {
        self.speed
    }

    /// 车身长度。
    #[must_use]
    pub const fn length(self) -> f64 {
        self.length
    }

    /// 生命周期状态。
    #[must_use]
    pub const fn status(self) -> VehicleStatus {
        self.status
    }

    /// 停车占用。
    #[must_use]
    pub const fn parking(self) -> Option<ParkingSpaceOrdinal> {
        self.parking
    }
}

/// 原子替换成功记录。旧句柄立即 stale；新句柄不同 generation。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VehicleReplaceRecord {
    /// 立即失效的旧句柄。
    pub old: VehicleHandle,
    /// 新的 live 句柄。
    pub new: VehicleHandle,
}

/// 入口占用导致的可重试阻塞。已提交世界不变。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VehicleReplaceBlock {
    /// 仍为 Completed 的旧句柄。
    pub old: VehicleHandle,
    /// 挡住入口的已提交车辆。
    pub blocker: VehicleHandle,
    /// `true`：blocker 在替换入口前方。
    pub blocker_ahead: bool,
    /// 相对前保险杠的间隙；无法判定前后时为 `0.0`。
    pub bumper_gap: f64,
}
