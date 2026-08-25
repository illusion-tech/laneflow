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
    pub(crate) progress_mm: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct LirParkingSpaceGeometry {
    pub(crate) lateral_offset_mm: i32,
    pub(crate) heading_offset_radians: f32,
    pub(crate) length_mm: u32,
    pub(crate) width_mm: u32,
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

use super::{FreezeEnv, LirParkingCounts, push_lir_identity, relation_range};
use crate::DiagnosticBundle;
use laneflow_static_contract::FieldTag;

pub(super) struct ParkingParts {
    pub parking_areas: Vec<LirParkingArea>,
    pub parking_area_spaces: Vec<ParkingSpaceOrdinal>,
    pub parking_spaces: Vec<LirParkingSpace>,
}

pub(super) fn freeze(
    env: &mut FreezeEnv<'_>,
    counts: &LirParkingCounts,
) -> Result<ParkingParts, DiagnosticBundle> {
    let mut parking_areas = Vec::with_capacity(env.capacity(counts.areas)?);
    let mut parking_area_spaces = Vec::with_capacity(env.capacity(counts.memberships)?);
    for mir_key in env
        .orders
        .parking_areas
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let area = &env.mir.parking_areas[mir_key.index()];
        let identity_range = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::ParkingAreaKey,
            &env.mir.modules[area.module.index()].authoring_namespace_id,
            &area.stable_key,
            None,
            env.limits,
            env.primary_span.clone(),
        )?;
        let member_start = parking_area_spaces.len();
        parking_area_spaces.extend(
            env.mir.parking_area_spaces[area.parking_spaces.as_usize_range()]
                .iter()
                .map(|member| env.orders.parking_spaces.ordinal(member.parking_space)),
        );
        parking_area_spaces[member_start..].sort_unstable();
        parking_areas.push(LirParkingArea {
            ordinal: env.orders.parking_areas.ordinal(mir_key),
            stable_id: area.stable_id,
            identity_fields: identity_range,
            parking_spaces: relation_range(
                member_start,
                parking_area_spaces.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
        });
    }

    let mut parking_spaces = Vec::with_capacity(env.capacity(counts.spaces)?);
    for mir_key in env
        .orders
        .parking_spaces
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let space = &env.mir.parking_spaces[mir_key.index()];
        let identity_range = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::ParkingSpaceKey,
            &env.mir.modules[space.module.index()].authoring_namespace_id,
            &space.stable_key,
            None,
            env.limits,
            env.primary_span.clone(),
        )?;
        parking_spaces.push(LirParkingSpace {
            ordinal: env.orders.parking_spaces.ordinal(mir_key),
            stable_id: space.stable_id,
            identity_fields: identity_range,
            parking_area: space
                .parking_area
                .map(|area| env.orders.parking_areas.ordinal(area)),
            entry: LirParkingLaneAnchor {
                lane_edge: env.orders.lane_edges.ordinal(space.entry.lane_edge),
                progress_mm: space.entry.progress_mm,
            },
            exit: LirParkingLaneAnchor {
                lane_edge: env.orders.lane_edges.ordinal(space.exit.lane_edge),
                progress_mm: space.exit.progress_mm,
            },
            geometry: LirParkingSpaceGeometry {
                lateral_offset_mm: space.geometry.lateral_offset_mm,
                heading_offset_radians: space.geometry.heading_offset_radians,
                length_mm: space.geometry.length_mm,
                width_mm: space.geometry.width_mm,
            },
        });
    }

    Ok(ParkingParts {
        parking_areas,
        parking_area_spaces,
        parking_spaces,
    })
}
