//! 停车设施、停车位、锚点与矩形几何视图。

use super::{CanonicalIdentityFieldView, impl_stable_entity_view};
use crate::lir::{LirParkingFacility, LirParkingSpace, LirUnit};
use laneflow_static_contract::{
    LaneEdgeOrdinal, ParkingFacilityId, ParkingFacilityOrdinal, ParkingSpaceId, ParkingSpaceOrdinal,
};

impl_stable_entity_view!(
    CanonicalParkingFacilityView,
    LirParkingFacility,
    ParkingFacilityOrdinal,
    ParkingFacilityId
);
impl_stable_entity_view!(
    CanonicalParkingSpaceView,
    LirParkingSpace,
    ParkingSpaceOrdinal,
    ParkingSpaceId
);

impl CanonicalParkingFacilityView<'_> {
    /// 返回按规范停车位序号冻结的显式成员集合；虚拟专用设施可为空。
    #[must_use]
    pub fn parking_spaces(&self) -> &[ParkingSpaceOrdinal] {
        &self.lir.parking_facility_spaces[self.record.parking_spaces.as_usize_range()]
    }

    #[must_use]
    pub const fn virtual_capacity(&self) -> u32 {
        self.record.virtual_capacity
    }

    #[must_use]
    pub fn total_capacity(&self) -> u64 {
        u64::try_from(self.parking_spaces().len())
            .unwrap_or(u64::MAX)
            .saturating_add(u64::from(self.record.virtual_capacity))
    }

    pub fn virtual_entries(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalParkingLaneAnchor> + '_ {
        self.lir.parking_facility_virtual_entries[self.record.virtual_entries.as_usize_range()]
            .iter()
            .map(|anchor| CanonicalParkingLaneAnchor {
                lane_edge: anchor.lane_edge,
                progress_mm: anchor.progress_mm,
            })
    }

    pub fn virtual_exits(&self) -> impl ExactSizeIterator<Item = CanonicalParkingLaneAnchor> + '_ {
        self.lir.parking_facility_virtual_exits[self.record.virtual_exits.as_usize_range()]
            .iter()
            .map(|anchor| CanonicalParkingLaneAnchor {
                lane_edge: anchor.lane_edge,
                progress_mm: anchor.progress_mm,
            })
    }
}

impl CanonicalParkingSpaceView<'_> {
    /// 返回可选停车区域组织归属；`None` 表示合法的独立停车位。
    #[must_use]
    pub const fn parking_facility(&self) -> Option<ParkingFacilityOrdinal> {
        self.record.parking_facility
    }

    /// 返回驶入并提交停车动作前必须到达的车道图锚点。
    #[must_use]
    pub const fn entry(&self) -> CanonicalParkingLaneAnchor {
        CanonicalParkingLaneAnchor {
            lane_edge: self.record.entry.lane_edge,
            progress_mm: self.record.entry.progress_mm,
        }
    }

    /// 返回离开停车位后重新接入车道图的锚点。
    #[must_use]
    pub const fn exit(&self) -> CanonicalParkingLaneAnchor {
        CanonicalParkingLaneAnchor {
            lane_edge: self.record.exit.lane_edge,
            progress_mm: self.record.exit.progress_mm,
        }
    }

    /// 返回相对入口边正向切线解释的不可变矩形几何。
    #[must_use]
    pub const fn geometry(&self) -> CanonicalParkingSpaceGeometry {
        CanonicalParkingSpaceGeometry {
            lateral_offset_mm: self.record.geometry.lateral_offset_mm,
            heading_offset_radians: self.record.geometry.heading_offset_radians,
            length_mm: self.record.geometry.length_mm,
            width_mm: self.record.geometry.width_mm,
        }
    }
}

/// Canonical LIR 中一个已验证停车锚点的值视图。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalParkingLaneAnchor {
    lane_edge: LaneEdgeOrdinal,
    progress_mm: u32,
}

impl CanonicalParkingLaneAnchor {
    /// 返回锚点所在的车道图边。
    #[must_use]
    pub const fn lane_edge(self) -> LaneEdgeOrdinal {
        self.lane_edge
    }

    /// 返回从边起点量取的纵向进度，单位为毫米。
    #[must_use]
    pub const fn progress_mm(self) -> u32 {
        self.progress_mm
    }
}

/// Canonical LIR 中已验证停车位矩形几何的值视图。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalParkingSpaceGeometry {
    lateral_offset_mm: i32,
    heading_offset_radians: f32,
    length_mm: u32,
    width_mm: u32,
}

impl CanonicalParkingSpaceGeometry {
    /// 返回相对入口边中心线的横向偏移，单位为毫米；正值位于行驶方向左侧。
    #[must_use]
    pub const fn lateral_offset_mm(self) -> i32 {
        self.lateral_offset_mm
    }

    /// 返回相对入口边正向切线的逆时针朝向偏移，单位为弧度。
    #[must_use]
    pub const fn heading_offset_radians(self) -> f32 {
        self.heading_offset_radians
    }

    /// 返回沿停车朝向的泊位长度，单位为毫米。
    #[must_use]
    pub const fn length_mm(self) -> u32 {
        self.length_mm
    }

    /// 返回垂直停车朝向的泊位宽度，单位为毫米。
    #[must_use]
    pub const fn width_mm(self) -> u32 {
        self.width_mm
    }
}
