//! 横断面领域 Canonical LIR 记录。

use laneflow_static_contract::{
    AuthoringLaneId, AuthoringLaneOrdinal, FacilityBandId, FacilityBandOrdinal, LaneEdgeOrdinal,
    LaneGroupId, LaneGroupOrdinal, RoadCorridorId, RoadCorridorOrdinal, RoadSectionId,
    RoadSectionOrdinal,
};

use crate::arena::TableRange;

use super::LirIdentityField;

pub(crate) enum LirCorridorElement {
    RoadSection(RoadSectionOrdinal),
    FacilityBand(FacilityBandOrdinal),
}

pub(crate) struct LirRoadCorridor {
    pub(crate) ordinal: RoadCorridorOrdinal,
    pub(crate) stable_id: RoadCorridorId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) reference_section: RoadSectionOrdinal,
    pub(crate) elements: TableRange<LirCorridorElement>,
}

pub(crate) struct LirRoadSection {
    pub(crate) ordinal: RoadSectionOrdinal,
    pub(crate) stable_id: RoadSectionId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) road_corridor: RoadCorridorOrdinal,
    pub(crate) kind_id: Box<str>,
    pub(crate) lanes: TableRange<AuthoringLaneOrdinal>,
}

pub(crate) struct LirAuthoringLane {
    pub(crate) ordinal: AuthoringLaneOrdinal,
    pub(crate) stable_id: AuthoringLaneId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) road_section: RoadSectionOrdinal,
    pub(crate) edge_chain: TableRange<LaneEdgeOrdinal>,
    pub(crate) lane_group: Option<LaneGroupOrdinal>,
}

pub(crate) struct LirLaneGroup {
    pub(crate) ordinal: LaneGroupOrdinal,
    pub(crate) stable_id: LaneGroupId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) road_section: RoadSectionOrdinal,
    pub(crate) members: TableRange<AuthoringLaneOrdinal>,
}

pub(crate) struct LirFacilityBand {
    pub(crate) ordinal: FacilityBandOrdinal,
    pub(crate) stable_id: FacilityBandId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) road_corridor: RoadCorridorOrdinal,
    pub(crate) kind_id: Box<str>,
}
