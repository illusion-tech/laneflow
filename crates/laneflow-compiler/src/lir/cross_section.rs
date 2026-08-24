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

use super::{FreezeEnv, LirCrossSectionCounts, push_lir_identity, relation_range};
use crate::DiagnosticBundle;
use crate::mir::{MirAuthoringLaneKey, MirCorridorElement};
use laneflow_static_contract::FieldTag;

pub(super) struct CrossSectionParts {
    pub road_corridors: Vec<LirRoadCorridor>,
    pub corridor_elements: Vec<LirCorridorElement>,
    pub road_sections: Vec<LirRoadSection>,
    pub road_section_lanes: Vec<AuthoringLaneOrdinal>,
    pub authoring_lanes: Vec<LirAuthoringLane>,
    pub authoring_lane_edges: Vec<LaneEdgeOrdinal>,
    pub lane_groups: Vec<LirLaneGroup>,
    pub lane_group_members: Vec<AuthoringLaneOrdinal>,
    pub facility_bands: Vec<LirFacilityBand>,
}

pub(super) fn freeze(
    env: &mut FreezeEnv<'_>,
    counts: &LirCrossSectionCounts,
) -> Result<CrossSectionParts, DiagnosticBundle> {
    let mut road_corridors = Vec::with_capacity(env.capacity(counts.road_corridors)?);
    let mut corridor_elements = Vec::with_capacity(env.capacity(counts.corridor_elements)?);
    for mir_key in env
        .orders
        .road_corridors
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let corridor = &env.mir.road_corridors[mir_key.index()];
        let identity_range = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::CorridorKey,
            &env.mir.modules[corridor.module.index()].authoring_namespace_id,
            &corridor.stable_key,
            None,
            env.limits,
            env.primary_span.clone(),
        )?;
        let relation_start = corridor_elements.len();
        corridor_elements.extend(
            env.mir.corridor_elements[corridor.elements.as_usize_range()]
                .iter()
                .map(|element| match element {
                    MirCorridorElement::RoadSection { road_section, .. } => {
                        LirCorridorElement::RoadSection(
                            env.orders.road_sections.ordinal(*road_section),
                        )
                    }
                    MirCorridorElement::FacilityBand { facility_band, .. } => {
                        LirCorridorElement::FacilityBand(
                            env.orders.facility_bands.ordinal(*facility_band),
                        )
                    }
                }),
        );
        road_corridors.push(LirRoadCorridor {
            ordinal: env.orders.road_corridors.ordinal(mir_key),
            stable_id: corridor.stable_id,
            identity_fields: identity_range,
            reference_section: env.orders.road_sections.ordinal(corridor.reference_section),
            elements: relation_range(
                relation_start,
                corridor_elements.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
        });
    }

    let mut road_sections = Vec::with_capacity(env.capacity(counts.road_sections)?);
    let mut road_section_lanes = Vec::with_capacity(env.capacity(counts.section_lanes)?);
    for mir_key in env
        .orders
        .road_sections
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let section = &env.mir.road_sections[mir_key.index()];
        let parent_id = env.mir.road_corridors[section.road_corridor.index()]
            .stable_id
            .into_untyped();
        let identity_range = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::SectionKey,
            &env.mir.modules[section.module.index()].authoring_namespace_id,
            &section.stable_key,
            Some((FieldTag::RoadCorridorStableId, parent_id.as_bytes())),
            env.limits,
            env.primary_span.clone(),
        )?;
        let relation_start = road_section_lanes.len();
        road_section_lanes.extend(section.lanes.as_usize_range().map(|index| {
            env.orders
                .authoring_lanes
                .ordinal(MirAuthoringLaneKey::from_raw(
                    u32::try_from(index).expect("MIR range prevalidated as u32"),
                ))
        }));
        road_sections.push(LirRoadSection {
            ordinal: env.orders.road_sections.ordinal(mir_key),
            stable_id: section.stable_id,
            identity_fields: identity_range,
            road_corridor: env.orders.road_corridors.ordinal(section.road_corridor),
            kind_id: section.kind_id.as_ref().into(),
            lanes: relation_range(
                relation_start,
                road_section_lanes.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
        });
    }

    let mut authoring_lanes = Vec::with_capacity(env.capacity(counts.authoring_lanes)?);
    let mut authoring_lane_edges = Vec::with_capacity(env.capacity(counts.authoring_lane_edges)?);
    for mir_key in env
        .orders
        .authoring_lanes
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let lane = &env.mir.authoring_lanes[mir_key.index()];
        let parent_id = env.mir.road_sections[lane.road_section.index()]
            .stable_id
            .into_untyped();
        let identity_range = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::LaneKey,
            &env.mir.modules[lane.module.index()].authoring_namespace_id,
            &lane.stable_key,
            Some((FieldTag::RoadSectionStableId, parent_id.as_bytes())),
            env.limits,
            env.primary_span.clone(),
        )?;
        let relation_start = authoring_lane_edges.len();
        authoring_lane_edges.extend(
            env.mir.authoring_lane_edges[lane.edge_chain.as_usize_range()]
                .iter()
                .map(|edge| env.orders.lane_edges.ordinal(edge.target)),
        );
        authoring_lanes.push(LirAuthoringLane {
            ordinal: env.orders.authoring_lanes.ordinal(mir_key),
            stable_id: lane.stable_id,
            identity_fields: identity_range,
            road_section: env.orders.road_sections.ordinal(lane.road_section),
            edge_chain: relation_range(
                relation_start,
                authoring_lane_edges.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
            lane_group: lane
                .lane_group
                .map(|key| env.orders.lane_groups.ordinal(key)),
        });
    }

    let mut lane_groups = Vec::with_capacity(env.capacity(counts.lane_groups)?);
    let mut lane_group_members = Vec::with_capacity(env.capacity(counts.lane_group_members)?);
    for mir_key in env
        .orders
        .lane_groups
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let group = &env.mir.lane_groups[mir_key.index()];
        let parent_id = env.mir.road_sections[group.road_section.index()]
            .stable_id
            .into_untyped();
        let identity_range = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::LaneGroupKey,
            &env.mir.modules[group.module.index()].authoring_namespace_id,
            &group.stable_key,
            Some((FieldTag::RoadSectionStableId, parent_id.as_bytes())),
            env.limits,
            env.primary_span.clone(),
        )?;
        let relation_start = lane_group_members.len();
        lane_group_members.extend(
            env.mir.lane_group_members[group.members.as_usize_range()]
                .iter()
                .map(|member| env.orders.authoring_lanes.ordinal(member.lane)),
        );
        lane_groups.push(LirLaneGroup {
            ordinal: env.orders.lane_groups.ordinal(mir_key),
            stable_id: group.stable_id,
            identity_fields: identity_range,
            road_section: env.orders.road_sections.ordinal(group.road_section),
            members: relation_range(
                relation_start,
                lane_group_members.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
        });
    }

    let mut facility_bands = Vec::with_capacity(env.capacity(counts.facility_bands)?);
    for mir_key in env
        .orders
        .facility_bands
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let band = &env.mir.facility_bands[mir_key.index()];
        let parent_id = env.mir.road_corridors[band.road_corridor.index()]
            .stable_id
            .into_untyped();
        let identity_range = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::FacilityBandKey,
            &env.mir.modules[band.module.index()].authoring_namespace_id,
            &band.stable_key,
            Some((FieldTag::RoadCorridorStableId, parent_id.as_bytes())),
            env.limits,
            env.primary_span.clone(),
        )?;
        facility_bands.push(LirFacilityBand {
            ordinal: env.orders.facility_bands.ordinal(mir_key),
            stable_id: band.stable_id,
            identity_fields: identity_range,
            road_corridor: env.orders.road_corridors.ordinal(band.road_corridor),
            kind_id: band.kind_id.as_ref().into(),
        });
    }

    Ok(CrossSectionParts {
        road_corridors,
        corridor_elements,
        road_sections,
        road_section_lanes,
        authoring_lanes,
        authoring_lane_edges,
        lane_groups,
        lane_group_members,
        facility_bands,
    })
}
