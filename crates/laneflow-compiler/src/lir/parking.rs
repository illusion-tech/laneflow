//! 停车领域 Canonical LIR 记录。

use laneflow_static_contract::{
    LaneEdgeOrdinal, ParkingFacilityId, ParkingFacilityOrdinal, ParkingSpaceId, ParkingSpaceOrdinal,
};

use crate::arena::TableRange;

use super::LirIdentityField;

/// 停车设施的 Canonical LIR 记录：同时组织显式停车位与虚拟停车容量，并可拥有多个虚拟入口/出口。
pub(crate) struct LirParkingFacility {
    pub(crate) ordinal: ParkingFacilityOrdinal,
    pub(crate) stable_id: ParkingFacilityId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) parking_spaces: TableRange<ParkingSpaceOrdinal>,
    pub(crate) virtual_capacity: u32,
    pub(crate) virtual_entries: TableRange<LirParkingLaneAnchor>,
    pub(crate) virtual_exits: TableRange<LirParkingLaneAnchor>,
}

/// 车道边上的停车锚点：以车道边序号与边内整数毫米进度定位。
#[derive(Clone, Copy)]
pub(crate) struct LirParkingLaneAnchor {
    pub(crate) lane_edge: LaneEdgeOrdinal,
    pub(crate) progress_mm: u32,
}

/// 停车位的静态几何：横向偏移、朝向偏移与长宽。
#[derive(Clone, Copy)]
pub(crate) struct LirParkingSpaceGeometry {
    pub(crate) lateral_offset_mm: i32,
    pub(crate) heading_offset_radians: f32,
    pub(crate) length_mm: u32,
    pub(crate) width_mm: u32,
}

/// 停车位的 Canonical LIR 记录：有排他占用、静态几何和 parked pose 的具体位置；可选归属于停车设施。
pub(crate) struct LirParkingSpace {
    pub(crate) ordinal: ParkingSpaceOrdinal,
    pub(crate) stable_id: ParkingSpaceId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) parking_facility: Option<ParkingFacilityOrdinal>,
    pub(crate) entry: LirParkingLaneAnchor,
    pub(crate) exit: LirParkingLaneAnchor,
    pub(crate) geometry: LirParkingSpaceGeometry,
}

use super::{FreezeEnv, LirParkingCounts, push_lir_identity, relation_range};
use crate::DiagnosticBundle;
use laneflow_static_contract::FieldTag;

/// 停车领域各表及设施成员、虚拟入口/出口锚点辅表的领域冻结产物。
pub(super) struct ParkingParts {
    pub parking_facilities: Vec<LirParkingFacility>,
    pub parking_facility_spaces: Vec<ParkingSpaceOrdinal>,
    pub parking_spaces: Vec<LirParkingSpace>,
    pub parking_facility_virtual_entries: Vec<LirParkingLaneAnchor>,
    pub parking_facility_virtual_exits: Vec<LirParkingLaneAnchor>,
}

/// 按规范排列冻结停车设施与停车位表；设施成员按停车位规范序号排序冻结。
pub(super) fn freeze(
    env: &mut FreezeEnv<'_>,
    counts: &LirParkingCounts,
) -> Result<ParkingParts, DiagnosticBundle> {
    let mut parking_facilities = Vec::with_capacity(env.capacity(counts.areas)?);
    let mut parking_facility_spaces = Vec::with_capacity(env.capacity(counts.memberships)?);
    let mut parking_facility_virtual_entries =
        Vec::with_capacity(env.capacity(counts.virtual_entries)?);
    let mut parking_facility_virtual_exits =
        Vec::with_capacity(env.capacity(counts.virtual_exits)?);
    for mir_key in env
        .orders
        .parking_facilities
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let area = &env.mir.parking_facilities[mir_key.index()];
        let identity_range = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::ParkingFacilityKey,
            &env.mir.modules[area.module.index()].authoring_namespace_id,
            &area.stable_key,
            None,
            env.limits,
            env.primary_span.clone(),
        )?;
        let member_start = parking_facility_spaces.len();
        parking_facility_spaces.extend(
            env.mir.parking_facility_spaces[area.parking_spaces.as_usize_range()]
                .iter()
                .map(|member| env.orders.parking_spaces.ordinal(member.parking_space)),
        );
        parking_facility_spaces[member_start..].sort_unstable();
        let entry_start = parking_facility_virtual_entries.len();
        parking_facility_virtual_entries.extend(
            env.mir.parking_facility_virtual_entries[area.virtual_entries.as_usize_range()]
                .iter()
                .map(|anchor| LirParkingLaneAnchor {
                    lane_edge: env.orders.lane_edges.ordinal(anchor.lane_edge),
                    progress_mm: anchor.progress_mm,
                }),
        );
        let exit_start = parking_facility_virtual_exits.len();
        parking_facility_virtual_exits.extend(
            env.mir.parking_facility_virtual_exits[area.virtual_exits.as_usize_range()]
                .iter()
                .map(|anchor| LirParkingLaneAnchor {
                    lane_edge: env.orders.lane_edges.ordinal(anchor.lane_edge),
                    progress_mm: anchor.progress_mm,
                }),
        );
        parking_facilities.push(LirParkingFacility {
            ordinal: env.orders.parking_facilities.ordinal(mir_key),
            stable_id: area.stable_id,
            identity_fields: identity_range,
            parking_spaces: relation_range(
                member_start,
                parking_facility_spaces.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
            virtual_capacity: area.virtual_capacity,
            virtual_entries: relation_range(
                entry_start,
                parking_facility_virtual_entries.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
            virtual_exits: relation_range(
                exit_start,
                parking_facility_virtual_exits.len(),
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
            parking_facility: space
                .parking_facility
                .map(|area| env.orders.parking_facilities.ordinal(area)),
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
        parking_facilities,
        parking_facility_spaces,
        parking_spaces,
        parking_facility_virtual_entries,
        parking_facility_virtual_exits,
    })
}
