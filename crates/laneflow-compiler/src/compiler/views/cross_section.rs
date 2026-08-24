//! 道路走廊、区段、编制车道、车道组与设施带视图。

use super::{
    CanonicalFacilityBandGeometryView, CanonicalIdentityFieldView, impl_stable_entity_view,
};
use crate::lir::{
    LirAuthoringLane, LirCorridorElement, LirFacilityBand, LirLaneGroup, LirRoadCorridor,
    LirRoadSection, LirUnit,
};
use laneflow_static_contract::{
    AuthoringLaneId, AuthoringLaneOrdinal, FacilityBandId, FacilityBandOrdinal, LaneEdgeOrdinal,
    LaneGroupId, LaneGroupOrdinal, RoadCorridorId, RoadCorridorOrdinal, RoadSectionId,
    RoadSectionOrdinal,
};

impl_stable_entity_view!(
    CanonicalRoadCorridorView,
    LirRoadCorridor,
    RoadCorridorOrdinal,
    RoadCorridorId
);
impl_stable_entity_view!(
    CanonicalRoadSectionView,
    LirRoadSection,
    RoadSectionOrdinal,
    RoadSectionId
);
impl_stable_entity_view!(
    CanonicalAuthoringLaneView,
    LirAuthoringLane,
    AuthoringLaneOrdinal,
    AuthoringLaneId
);
impl_stable_entity_view!(
    CanonicalLaneGroupView,
    LirLaneGroup,
    LaneGroupOrdinal,
    LaneGroupId
);
impl_stable_entity_view!(
    CanonicalFacilityBandView,
    LirFacilityBand,
    FacilityBandOrdinal,
    FacilityBandId
);

/// 道路走廊有序横断面中的一项有类型成员。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CanonicalCorridorElement {
    /// 一个承载编制车道的有方向道路区段。
    RoadSection(RoadSectionOrdinal),
    /// 一个不进入遍历图的非方向设施带。
    FacilityBand(FacilityBandOrdinal),
}

impl CanonicalRoadCorridorView<'_> {
    /// 返回定义横断面参考方向、且已证明属于本走廊的道路区段。
    #[must_use]
    pub const fn reference_section(&self) -> RoadSectionOrdinal {
        self.record.reference_section
    }

    /// 按走廊参考方向从左到右遍历横断面成员；该顺序具有领域语义。
    pub fn elements(&self) -> impl ExactSizeIterator<Item = CanonicalCorridorElement> + '_ {
        self.lir.corridor_elements[self.record.elements.as_usize_range()]
            .iter()
            .map(|element| match element {
                LirCorridorElement::RoadSection(ordinal) => {
                    CanonicalCorridorElement::RoadSection(*ordinal)
                }
                LirCorridorElement::FacilityBand(ordinal) => {
                    CanonicalCorridorElement::FacilityBand(*ordinal)
                }
            })
    }
}

impl CanonicalRoadSectionView<'_> {
    /// 返回唯一拥有本区段的道路走廊。
    #[must_use]
    pub const fn road_corridor(&self) -> RoadCorridorOrdinal {
        self.record.road_corridor
    }

    /// 返回已验证为 lane-bearing 类别的物理设施 token。
    #[must_use]
    pub fn kind_id(&self) -> &str {
        &self.record.kind_id
    }

    /// 返回按走廊参考方向从左到右排列的编制车道序号。
    #[must_use]
    pub fn lanes(&self) -> &[AuthoringLaneOrdinal] {
        &self.lir.road_section_lanes[self.record.lanes.as_usize_range()]
    }
}

impl CanonicalAuthoringLaneView<'_> {
    /// 返回唯一拥有本编制车道的道路区段。
    #[must_use]
    pub const fn road_section(&self) -> RoadSectionOrdinal {
        self.record.road_section
    }

    /// 返回沿行驶方向排列、已证明直接连通的车道图边覆盖链。
    #[must_use]
    pub fn edge_chain(&self) -> &[LaneEdgeOrdinal] {
        &self.lir.authoring_lane_edges[self.record.edge_chain.as_usize_range()]
    }

    /// 返回可选车道组；存在时已证明与本车道属于同一道路区段。
    #[must_use]
    pub const fn lane_group(&self) -> Option<LaneGroupOrdinal> {
        self.record.lane_group
    }
}

impl CanonicalLaneGroupView<'_> {
    /// 返回唯一拥有本组的道路区段。
    #[must_use]
    pub const fn road_section(&self) -> RoadSectionOrdinal {
        self.record.road_section
    }

    /// 返回非空且全部属于同一父区段的编制车道成员。
    #[must_use]
    pub fn members(&self) -> &[AuthoringLaneOrdinal] {
        &self.lir.lane_group_members[self.record.members.as_usize_range()]
    }
}

impl CanonicalFacilityBandView<'_> {
    /// 返回唯一拥有本设施带的道路走廊。
    #[must_use]
    pub const fn road_corridor(&self) -> RoadCorridorOrdinal {
        self.record.road_corridor
    }

    /// 返回已验证为 non-traversable 类别的物理设施 token。
    #[must_use]
    pub fn kind_id(&self) -> &str {
        &self.record.kind_id
    }

    /// 返回 non-traversable 设施带的规范空间几何；headless LIR 返回 `None`。
    #[must_use]
    pub fn spatial_geometry(&self) -> Option<CanonicalFacilityBandGeometryView<'_>> {
        self.lir
            .facility_band_geometries
            .binary_search_by_key(&self.record.ordinal.raw(), |geometry| {
                geometry.facility_band.raw()
            })
            .ok()
            .map(|index| {
                CanonicalFacilityBandGeometryView::from_lir(
                    self.lir,
                    &self.lir.facility_band_geometries[index],
                )
            })
    }
}
