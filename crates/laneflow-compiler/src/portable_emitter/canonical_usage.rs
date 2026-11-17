//! 消费端规范统计：从受检 LFCA 字节独立求规范路网规模。
//!
//! 只借用受检视图逐表扫描，不建立实体索引、不分配，可在建立索引前执行。每个计入
//! `CanonicalRelationCount` 的角色按可移植规范制品第 4.3 节登记的唯一投影计数，与编译端
//! [`crate::canonical_usage::lir_statistics`] 逐角色对应。

use laneflow_format::{RegistryCheckedObjectView, RegistryCheckedRowView};
use laneflow_static_contract::{EntityKind, SourceRelationRole};

use super::PortableEmissionError;
use super::lfsd::base::{
    checked_ordinal_vector_with, checked_record_vector_with, checked_u32_with,
};
use crate::canonical_usage::{CanonicalStatistics, RelationRoleCounts};

/// 身份表所在节。
const IDENTITY_SECTION: u32 = 1;
/// 实体表所在节；节内按 [`EntityKind::ALL`] 顺序每种实体一张表。
const ENTITY_SECTION: u32 = 2;
/// 路口内部边归属表所在节。
const DERIVED_SECTION: u32 = 3;
/// 空间节。
const SPATIAL_SECTION: u32 = 4;

/// 空间节中三类几何表的表序号、角色与点向量字段标签。
const GEOMETRY_TABLES: [(u32, SourceRelationRole, u16); 3] = [
    (1, SourceRelationRole::CanonicalFrameLaneEdgeGeometry, 4),
    (2, SourceRelationRole::CanonicalFrameFacilityBandGeometry, 3),
    (3, SourceRelationRole::CanonicalFrameConflictZoneRegion, 5),
];

fn count(len: u32) -> u64 {
    u64::from(len)
}

/// 有类型序号向量字段的元素数。
fn ordinal_vector_len(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<u64, PortableEmissionError> {
    Ok(count(
        checked_ordinal_vector_with(row, tag, mismatch)?.len(),
    ))
}

/// 记录向量字段的元素数。
fn record_vector_len(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<u64, PortableEmissionError> {
    Ok(count(checked_record_vector_with(row, tag, mismatch)?.len()))
}

/// 必备标量引用字段：存在且类型正确时计 1。
fn required_scalar(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<u64, PortableEmissionError> {
    checked_u32_with(row, tag, mismatch)?;
    Ok(1)
}

/// 可选标量引用字段：存在时须类型正确并计 1，缺失计 0。
fn optional_scalar(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<u64, PortableEmissionError> {
    if row.field_by_tag(tag).is_none() {
        return Ok(0);
    }
    required_scalar(row, tag, mismatch)
}

/// 一行实体记录对各角色的贡献。
///
/// 实体种类穷尽匹配；不拥有计入角色的种类没有贡献。
fn add_entity_row(
    roles: &mut RelationRoleCounts,
    kind: EntityKind,
    row: RegistryCheckedRowView<'_>,
    mismatch: PortableEmissionError,
) -> Result<(), PortableEmissionError> {
    use SourceRelationRole as Role;
    match kind {
        EntityKind::RoadCorridor => {
            roles.add(
                Role::RoadCorridorElement,
                record_vector_len(row, 4, mismatch)?,
            );
        }
        EntityKind::RoadSection => {
            roles.add(Role::RoadSectionLane, ordinal_vector_len(row, 5, mismatch)?);
        }
        EntityKind::AuthoringLane => {
            roles.add(
                Role::AuthoringLaneEdge,
                ordinal_vector_len(row, 4, mismatch)?,
            );
        }
        EntityKind::LaneEdge => {
            roles.add(
                Role::LaneEdgeSuccessor,
                ordinal_vector_len(row, 5, mismatch)?,
            );
        }
        EntityKind::Junction => {
            roles.add(
                Role::JunctionMovement,
                ordinal_vector_len(row, 3, mismatch)?,
            );
        }
        EntityKind::Movement => {
            roles.add(
                Role::MovementManeuverPath,
                ordinal_vector_len(row, 6, mismatch)?,
            );
        }
        EntityKind::ManeuverPath => {
            roles.add(
                Role::ManeuverPathEdge,
                ordinal_vector_len(row, 4, mismatch)?,
            );
            roles.add(
                Role::ManeuverPathGate,
                ordinal_vector_len(row, 5, mismatch)?,
            );
            roles.add(
                Role::ManeuverPathWaitingZone,
                ordinal_vector_len(row, 6, mismatch)?,
            );
        }
        EntityKind::ManeuverGate => {
            // role 20 只按 ManeuverGate.signalGroup 计数；SignalGroup 一侧的反向表示不计。
            roles.add(
                Role::ManeuverGateSignalGroup,
                optional_scalar(row, 7, mismatch)?,
            );
        }
        EntityKind::StopLine => {
            roles.add(
                Role::StopLineManeuverGate,
                ordinal_vector_len(row, 4, mismatch)?,
            );
        }
        EntityKind::SignalController => {
            roles.add(
                Role::SignalControllerGroup,
                ordinal_vector_len(row, 5, mismatch)?,
            );
            roles.add(
                Role::SignalControllerPhase,
                ordinal_vector_len(row, 6, mismatch)?,
            );
        }
        EntityKind::SignalPhase => {
            roles.add(Role::SignalPhaseState, record_vector_len(row, 5, mismatch)?);
        }
        EntityKind::ParkingFacility => {
            roles.add(
                Role::ParkingFacilityVirtualEntry,
                record_vector_len(row, 5, mismatch)?,
            );
            roles.add(
                Role::ParkingFacilityVirtualExit,
                record_vector_len(row, 6, mismatch)?,
            );
        }
        EntityKind::ParkingSpace => {
            roles.add(
                Role::ParkingSpaceFacility,
                optional_scalar(row, 3, mismatch)?,
            );
            roles.add(Role::ParkingSpaceEntry, required_scalar(row, 4, mismatch)?);
            roles.add(Role::ParkingSpaceExit, required_scalar(row, 6, mismatch)?);
        }
        EntityKind::LaneGroup => {
            roles.add(Role::LaneGroupMember, ordinal_vector_len(row, 4, mismatch)?);
        }
        EntityKind::ParticipantClass => {
            roles.add(
                Role::ParticipantClassExtends,
                optional_scalar(row, 3, mismatch)?,
            );
        }
        EntityKind::AccessRule => {
            roles.add(Role::AccessRuleTarget, required_scalar(row, 4, mismatch)?);
            roles.add(
                Role::AccessRuleParticipantClass,
                ordinal_vector_len(row, 6, mismatch)?,
            );
        }
        EntityKind::VehicleProfile => {
            roles.add(
                Role::VehicleProfileParticipantClass,
                required_scalar(row, 3, mismatch)?,
            );
        }
        EntityKind::ConflictZone => {
            // role 15 按 ConflictZone.junction 过滤实体行，每行计 1。
            roles.add(
                Role::JunctionConflictZone,
                required_scalar(row, 3, mismatch)?,
            );
        }
        EntityKind::ParticipantStream => {
            // role 16 按 ParticipantStream.junction 过滤实体行，每行计 1。
            roles.add(
                Role::JunctionParticipantStream,
                required_scalar(row, 3, mismatch)?,
            );
            roles.add(
                Role::ParticipantStreamManeuverPath,
                required_scalar(row, 4, mismatch)?,
            );
            roles.add(
                Role::ParticipantStreamConflictPassage,
                record_vector_len(row, 5, mismatch)?,
            );
        }
        EntityKind::WaitingZone
        | EntityKind::SignalGroup
        | EntityKind::FacilityBand
        | EntityKind::RightOfWayPolicySet
        | EntityKind::CanonicalFrame => {}
    }
    Ok(())
}

/// 从受检 LFCA 视图求规范统计；结构与登记不符时返回 `mismatch`。
pub(super) fn lfca_statistics(
    view: RegistryCheckedObjectView<'_>,
    mismatch: PortableEmissionError,
) -> Result<CanonicalStatistics, PortableEmissionError> {
    let identity_rows = view
        .section(IDENTITY_SECTION)
        .and_then(|section| section.table(0))
        .ok_or(mismatch)?
        .row_count();
    let mut statistics = CanonicalStatistics {
        stable_entities: count(identity_rows),
        ..CanonicalStatistics::default()
    };

    let mut tables = view.section(ENTITY_SECTION).ok_or(mismatch)?.tables();
    for kind in EntityKind::ALL {
        let table = tables.next().ok_or(mismatch)?;
        match kind {
            EntityKind::ManeuverGate => statistics.maneuver_gates = count(table.row_count()),
            EntityKind::WaitingZone => statistics.waiting_zones = count(table.row_count()),
            _ => {}
        }
        for row in table.rows() {
            add_entity_row(&mut statistics.relation_roles, kind, row, mismatch)?;
        }
    }
    if tables.next().is_some() {
        return Err(mismatch);
    }

    statistics.relation_roles.add(
        SourceRelationRole::JunctionInternalEdge,
        count(
            view.section(DERIVED_SECTION)
                .and_then(|section| section.table(0))
                .ok_or(mismatch)?
                .row_count(),
        ),
    );

    let spatial = view.section(SPATIAL_SECTION).ok_or(mismatch)?;
    for (table_ordinal, role, points_tag) in GEOMETRY_TABLES {
        let table = spatial.table(table_ordinal).ok_or(mismatch)?;
        statistics
            .relation_roles
            .add(role, count(table.row_count()));
        for row in table.rows() {
            statistics.geometry_points = statistics
                .geometry_points
                .saturating_add(record_vector_len(row, points_tag, mismatch)?);
        }
    }
    Ok(statistics)
}

#[cfg(test)]
mod tests;
