//! 规范空间几何与坐标框架视图。

use super::{CanonicalIdentityFieldView, impl_stable_entity_view};
use crate::lir::{
    LirCanonicalFrame, LirCanonicalPoint3F32, LirFacilityBandGeometry, LirLaneEdgeGeometry,
    LirSpatialSegment, LirUnit,
};
use laneflow_static_contract::{
    CanonicalFrameId, CanonicalFrameOrdinal, FacilityBandOrdinal, LaneEdgeOrdinal,
};

/// 一条 `LaneEdge` 的只读规范中心线及预计算采样表。
#[derive(Clone, Copy)]
pub struct CanonicalLaneEdgeGeometryView<'a> {
    lir: &'a LirUnit,
    lane_edge: LaneEdgeOrdinal,
    geometry: &'a LirLaneEdgeGeometry,
}

impl<'a> CanonicalLaneEdgeGeometryView<'a> {
    pub(in crate::compiler) const fn from_lir(
        lir: &'a LirUnit,
        lane_edge: LaneEdgeOrdinal,
        geometry: &'a LirLaneEdgeGeometry,
    ) -> Self {
        Self {
            lir,
            lane_edge,
            geometry,
        }
    }
}

impl CanonicalLaneEdgeGeometryView<'_> {
    #[must_use]
    pub const fn lane_edge(&self) -> LaneEdgeOrdinal {
        self.lane_edge
    }

    #[must_use]
    pub const fn canonical_frame(&self) -> CanonicalFrameOrdinal {
        self.geometry.canonical_frame
    }

    #[must_use]
    pub const fn arc_length_meters(&self) -> f32 {
        self.geometry.arc_length_meters
    }

    pub fn points(&self) -> impl ExactSizeIterator<Item = CanonicalPoint3F32> + '_ {
        self.lir.canonical_points[self.geometry.points.as_usize_range()]
            .iter()
            .copied()
            .map(CanonicalPoint3F32::from)
    }

    pub fn segments(&self) -> impl ExactSizeIterator<Item = CanonicalSpatialSegment> + '_ {
        self.lir.spatial_segments[self.geometry.segments.as_usize_range()]
            .iter()
            .copied()
            .map(CanonicalSpatialSegment::from)
    }
}

/// 已量化到 canonical frame 的只读 `f32` 点，单位为米。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalPoint3F32 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl From<LirCanonicalPoint3F32> for CanonicalPoint3F32 {
    fn from(point: LirCanonicalPoint3F32) -> Self {
        Self {
            x: point.x,
            y: point.y,
            z: point.z,
        }
    }
}

/// 中心线采样使用的单段累计弧长和正交局部基。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalSpatialSegment {
    pub length_meters: f32,
    pub cumulative_end_meters: f32,
    pub tangent: [f32; 3],
    pub up: [f32; 3],
}

impl From<LirSpatialSegment> for CanonicalSpatialSegment {
    fn from(segment: LirSpatialSegment) -> Self {
        Self {
            length_meters: segment.length_meters,
            cumulative_end_meters: segment.cumulative_end_meters,
            tangent: segment.tangent,
            up: segment.up,
        }
    }
}

/// 一条 non-traversable `FacilityBand` 的只读规范中心线。
#[derive(Clone, Copy)]
pub struct CanonicalFacilityBandGeometryView<'a> {
    lir: &'a LirUnit,
    geometry: &'a LirFacilityBandGeometry,
}

impl<'a> CanonicalFacilityBandGeometryView<'a> {
    pub(in crate::compiler) const fn from_lir(
        lir: &'a LirUnit,
        geometry: &'a LirFacilityBandGeometry,
    ) -> Self {
        Self { lir, geometry }
    }
}

impl CanonicalFacilityBandGeometryView<'_> {
    #[must_use]
    pub const fn facility_band(&self) -> FacilityBandOrdinal {
        self.geometry.facility_band
    }

    #[must_use]
    pub const fn canonical_frame(&self) -> CanonicalFrameOrdinal {
        self.geometry.canonical_frame
    }

    pub fn points(&self) -> impl ExactSizeIterator<Item = CanonicalPoint3F32> + '_ {
        self.lir.canonical_points[self.geometry.points.as_usize_range()]
            .iter()
            .copied()
            .map(CanonicalPoint3F32::from)
    }
}

impl_stable_entity_view!(
    CanonicalFrameView,
    LirCanonicalFrame,
    CanonicalFrameOrdinal,
    CanonicalFrameId
);
