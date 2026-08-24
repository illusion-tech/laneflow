//! 停车区域、停车位、锚点与矩形几何视图。

use super::{CanonicalIdentityFieldView, impl_stable_entity_view};
use crate::lir::{LirParkingArea, LirParkingSpace, LirUnit};
use laneflow_static_contract::{
    LaneEdgeOrdinal, ParkingAreaId, ParkingAreaOrdinal, ParkingSpaceId, ParkingSpaceOrdinal,
};

impl_stable_entity_view!(
    CanonicalParkingAreaView,
    LirParkingArea,
    ParkingAreaOrdinal,
    ParkingAreaId
);
impl_stable_entity_view!(
    CanonicalParkingSpaceView,
    LirParkingSpace,
    ParkingSpaceOrdinal,
    ParkingSpaceId
);

impl CanonicalParkingAreaView<'_> {
    /// 返回按规范停车位序号冻结的非空成员集合。
    #[must_use]
    pub fn parking_spaces(&self) -> &[ParkingSpaceOrdinal] {
        &self.lir.parking_area_spaces[self.record.parking_spaces.as_usize_range()]
    }
}

impl CanonicalParkingSpaceView<'_> {
    /// 返回可选停车区域组织归属；`None` 表示合法的独立停车位。
    #[must_use]
    pub const fn parking_area(&self) -> Option<ParkingAreaOrdinal> {
        self.record.parking_area
    }

    /// 返回驶入并提交停车动作前必须到达的车道图锚点。
    #[must_use]
    pub const fn entry(&self) -> CanonicalParkingLaneAnchor {
        CanonicalParkingLaneAnchor {
            lane_edge: self.record.entry.lane_edge,
            progress_meters: self.record.entry.progress_meters,
        }
    }

    /// 返回离开停车位后重新接入车道图的锚点。
    #[must_use]
    pub const fn exit(&self) -> CanonicalParkingLaneAnchor {
        CanonicalParkingLaneAnchor {
            lane_edge: self.record.exit.lane_edge,
            progress_meters: self.record.exit.progress_meters,
        }
    }

    /// 返回相对入口边正向切线解释的不可变矩形几何。
    #[must_use]
    pub const fn geometry(&self) -> CanonicalParkingSpaceGeometry {
        CanonicalParkingSpaceGeometry {
            lateral_offset_meters: self.record.geometry.lateral_offset_meters,
            heading_offset_radians: self.record.geometry.heading_offset_radians,
            length_meters: self.record.geometry.length_meters,
            width_meters: self.record.geometry.width_meters,
        }
    }
}

/// Canonical LIR 中一个已验证停车锚点的值视图。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalParkingLaneAnchor {
    lane_edge: LaneEdgeOrdinal,
    progress_meters: f64,
}

impl CanonicalParkingLaneAnchor {
    /// 返回锚点所在的车道图边。
    #[must_use]
    pub const fn lane_edge(self) -> LaneEdgeOrdinal {
        self.lane_edge
    }

    /// 返回从边起点量取的纵向进度，单位为米。
    #[must_use]
    pub const fn progress_meters(self) -> f64 {
        self.progress_meters
    }
}

/// Canonical LIR 中已验证停车位矩形几何的值视图。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalParkingSpaceGeometry {
    lateral_offset_meters: f64,
    heading_offset_radians: f64,
    length_meters: f64,
    width_meters: f64,
}

impl CanonicalParkingSpaceGeometry {
    /// 返回相对入口边中心线的横向偏移，单位为米；正值位于行驶方向左侧。
    #[must_use]
    pub const fn lateral_offset_meters(self) -> f64 {
        self.lateral_offset_meters
    }

    /// 返回相对入口边正向切线的逆时针朝向偏移，单位为弧度。
    #[must_use]
    pub const fn heading_offset_radians(self) -> f64 {
        self.heading_offset_radians
    }

    /// 返回沿停车朝向的泊位长度，单位为米。
    #[must_use]
    pub const fn length_meters(self) -> f64 {
        self.length_meters
    }

    /// 返回垂直停车朝向的泊位宽度，单位为米。
    #[must_use]
    pub const fn width_meters(self) -> f64 {
        self.width_meters
    }
}
