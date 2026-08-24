//! 停车领域 Canonical LIR 记录。

use laneflow_static_contract::{
    LaneEdgeOrdinal, ParkingAreaId, ParkingAreaOrdinal, ParkingSpaceId, ParkingSpaceOrdinal,
};

use crate::arena::TableRange;

use super::LirIdentityField;

pub(crate) struct LirParkingArea {
    pub(crate) ordinal: ParkingAreaOrdinal,
    pub(crate) stable_id: ParkingAreaId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) parking_spaces: TableRange<ParkingSpaceOrdinal>,
}

#[derive(Clone, Copy)]
pub(crate) struct LirParkingLaneAnchor {
    pub(crate) lane_edge: LaneEdgeOrdinal,
    pub(crate) progress_meters: f64,
}

#[derive(Clone, Copy)]
pub(crate) struct LirParkingSpaceGeometry {
    pub(crate) lateral_offset_meters: f64,
    pub(crate) heading_offset_radians: f64,
    pub(crate) length_meters: f64,
    pub(crate) width_meters: f64,
}

pub(crate) struct LirParkingSpace {
    pub(crate) ordinal: ParkingSpaceOrdinal,
    pub(crate) stable_id: ParkingSpaceId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) parking_area: Option<ParkingAreaOrdinal>,
    pub(crate) entry: LirParkingLaneAnchor,
    pub(crate) exit: LirParkingLaneAnchor,
    pub(crate) geometry: LirParkingSpaceGeometry,
}
