//! 消费端规范统计：从受检 LFCA 字节独立求规范路网规模。
//!
//! 只借用受检视图逐表扫描，不建立实体索引、不分配，可在建立索引前执行。每个计入
//! `CanonicalRelationCount` 的角色由 [`lfca_projection`] 穷尽登记其在 LFCA 中的唯一计数
//! 投影（可移植规范制品第 4.3 节），与编译端 [`crate::canonical_usage::lir_statistics`]
//! 逐角色对应；两侧只共享角色定义，各自实现投影。

use laneflow_format::{
    RegistryCheckedObjectView, RegistryCheckedRowView, RegistryCheckedTableView,
};
use laneflow_static_contract::{EntityKind, SourceRelationRole};

use super::PortableEmissionError;
use super::lfsd::base::{
    checked_ordinal_vector_with, checked_record_vector_with, checked_u32_with,
};
use crate::canonical_usage::CanonicalStatistics;

/// 身份表所在节。
const IDENTITY_SECTION: u32 = 1;
/// 实体表所在节；节内按 [`EntityKind::ALL`] 顺序每种实体一张表。
const ENTITY_SECTION: u32 = 2;
/// 路口内部边归属表所在节。
const DERIVED_SECTION: u32 = 3;
/// 空间节。
const SPATIAL_SECTION: u32 = 4;

/// 空间节中三类几何表的表序号与点向量字段标签；点数计入 `GeometryPointCount`。
const GEOMETRY_POINT_FIELDS: [(u32, u16); 3] = [(1, 4), (2, 3), (3, 5)];

/// 一个角色在 LFCA 中的唯一计数投影。
#[derive(Clone, Copy)]
enum LfcaProjection {
    /// 所有者实体行的有类型序号向量字段，每个元素计 1。
    OrdinalVector(EntityKind, u16),
    /// 所有者实体行的记录向量字段，每条记录计 1。
    RecordVector(EntityKind, u16),
    /// 实体行的必备标量引用字段，每行计 1。
    RequiredScalar(EntityKind, u16),
    /// 实体行的可选标量引用字段，存在时计 1。
    OptionalScalar(EntityKind, u16),
    /// 路口内部边归属表的行，每行计 1。
    InternalEdgeRows,
    /// 空间节某张几何表的行，每行计 1，与点数无关。
    GeometryRows(u32),
}

/// 实体行上一个字段投影的贡献。
fn entity_row_count(
    projection: LfcaProjection,
    row: RegistryCheckedRowView<'_>,
    mismatch: PortableEmissionError,
) -> Result<u64, PortableEmissionError> {
    Ok(match projection {
        LfcaProjection::OrdinalVector(_, tag) => {
            u64::from(checked_ordinal_vector_with(row, tag, mismatch)?.len())
        }
        LfcaProjection::RecordVector(_, tag) => {
            u64::from(checked_record_vector_with(row, tag, mismatch)?.len())
        }
        LfcaProjection::RequiredScalar(_, tag) => {
            checked_u32_with(row, tag, mismatch)?;
            1
        }
        LfcaProjection::OptionalScalar(_, tag) => {
            if row.field_by_tag(tag).is_none() {
                0
            } else {
                checked_u32_with(row, tag, mismatch)?;
                1
            }
        }
        LfcaProjection::InternalEdgeRows | LfcaProjection::GeometryRows(_) => {
            return Err(mismatch);
        }
    })
}

/// 计入 `CanonicalRelationCount` 的角色在 LFCA 中的唯一计数投影；不计入的角色返回 `None`。
///
/// 角色穷尽匹配：新增角色时必须在此登记投影。同一关联的双向物理表示只登记一侧，例如
/// role 20 只按 `ManeuverGate.signalGroup` 计数。
const fn lfca_projection(role: SourceRelationRole) -> Option<LfcaProjection> {
    use EntityKind as K;
    use LfcaProjection as P;
    use SourceRelationRole as R;
    Some(match role {
        R::LaneEdgeSuccessor => P::OrdinalVector(K::LaneEdge, 5),
        R::RoadCorridorElement => P::RecordVector(K::RoadCorridor, 4),
        R::RoadSectionLane => P::OrdinalVector(K::RoadSection, 5),
        R::AuthoringLaneEdge => P::OrdinalVector(K::AuthoringLane, 4),
        R::LaneGroupMember => P::OrdinalVector(K::LaneGroup, 4),
        R::JunctionMovement => P::OrdinalVector(K::Junction, 3),
        R::MovementManeuverPath => P::OrdinalVector(K::Movement, 6),
        R::ManeuverPathEdge => P::OrdinalVector(K::ManeuverPath, 4),
        R::JunctionInternalEdge => P::InternalEdgeRows,
        R::ManeuverPathGate => P::OrdinalVector(K::ManeuverPath, 5),
        R::ManeuverPathWaitingZone => P::OrdinalVector(K::ManeuverPath, 6),
        R::StopLineManeuverGate => P::OrdinalVector(K::StopLine, 4),
        R::ParkingFacilityVirtualEntry => P::RecordVector(K::ParkingFacility, 5),
        R::ParkingFacilityVirtualExit => P::RecordVector(K::ParkingFacility, 6),
        // role 15、16 按所有者 junction 字段过滤实体行，每行计 1。
        R::JunctionConflictZone => P::RequiredScalar(K::ConflictZone, 3),
        R::JunctionParticipantStream => P::RequiredScalar(K::ParticipantStream, 3),
        R::SignalControllerGroup => P::OrdinalVector(K::SignalController, 5),
        R::SignalControllerPhase => P::OrdinalVector(K::SignalController, 6),
        R::SignalPhaseState => P::RecordVector(K::SignalPhase, 5),
        R::ManeuverGateSignalGroup => P::OptionalScalar(K::ManeuverGate, 7),
        R::ParkingSpaceFacility => P::OptionalScalar(K::ParkingSpace, 3),
        R::ParkingSpaceEntry => P::RequiredScalar(K::ParkingSpace, 4),
        R::ParkingSpaceExit => P::RequiredScalar(K::ParkingSpace, 6),
        R::ParticipantClassExtends => P::OptionalScalar(K::ParticipantClass, 3),
        R::AccessRuleTarget => P::RequiredScalar(K::AccessRule, 4),
        R::AccessRuleParticipantClass => P::OrdinalVector(K::AccessRule, 6),
        R::VehicleProfileParticipantClass => P::RequiredScalar(K::VehicleProfile, 3),
        R::CanonicalFrameLaneEdgeGeometry => P::GeometryRows(1),
        R::CanonicalFrameFacilityBandGeometry => P::GeometryRows(2),
        R::ParticipantStreamManeuverPath => P::RequiredScalar(K::ParticipantStream, 4),
        R::ParticipantStreamConflictPassage => P::RecordVector(K::ParticipantStream, 5),
        R::CanonicalFrameConflictZoneRegion => P::GeometryRows(3),
        R::PolicyEvidence | R::PolicyGapProfile | R::PolicyStreamRule | R::PolicyGateRule => {
            return None;
        }
    })
}

fn table(
    view: RegistryCheckedObjectView<'_>,
    section: u32,
    table: u32,
    mismatch: PortableEmissionError,
) -> Result<RegistryCheckedTableView<'_>, PortableEmissionError> {
    view.section(section)
        .and_then(|section| section.table(table))
        .ok_or(mismatch)
}

/// 某实体种类的实体表。
fn entity_table(
    view: RegistryCheckedObjectView<'_>,
    kind: EntityKind,
    mismatch: PortableEmissionError,
) -> Result<RegistryCheckedTableView<'_>, PortableEmissionError> {
    let ordinal = EntityKind::ALL
        .iter()
        .position(|candidate| *candidate == kind)
        .ok_or(mismatch)?;
    table(
        view,
        ENTITY_SECTION,
        u32::try_from(ordinal).map_err(|_| mismatch)?,
        mismatch,
    )
}

/// 一个角色投影在 LFCA 中的出现记录数。
fn projection_count(
    view: RegistryCheckedObjectView<'_>,
    projection: LfcaProjection,
    mismatch: PortableEmissionError,
) -> Result<u64, PortableEmissionError> {
    let kind = match projection {
        LfcaProjection::InternalEdgeRows => {
            return Ok(u64::from(
                table(view, DERIVED_SECTION, 0, mismatch)?.row_count(),
            ));
        }
        LfcaProjection::GeometryRows(ordinal) => {
            return Ok(u64::from(
                table(view, SPATIAL_SECTION, ordinal, mismatch)?.row_count(),
            ));
        }
        LfcaProjection::OrdinalVector(kind, _)
        | LfcaProjection::RecordVector(kind, _)
        | LfcaProjection::RequiredScalar(kind, _)
        | LfcaProjection::OptionalScalar(kind, _) => kind,
    };
    let mut total = 0_u64;
    for row in entity_table(view, kind, mismatch)?.rows() {
        total = total.saturating_add(entity_row_count(projection, row, mismatch)?);
    }
    Ok(total)
}

/// 从受检 LFCA 视图求规范统计；结构与登记不符时返回 `mismatch`。
pub(super) fn lfca_statistics(
    view: RegistryCheckedObjectView<'_>,
    mismatch: PortableEmissionError,
) -> Result<CanonicalStatistics, PortableEmissionError> {
    if view
        .section(ENTITY_SECTION)
        .ok_or(mismatch)?
        .tables()
        .count()
        != EntityKind::ALL.len()
    {
        return Err(mismatch);
    }
    let mut statistics = CanonicalStatistics {
        stable_entities: u64::from(table(view, IDENTITY_SECTION, 0, mismatch)?.row_count()),
        maneuver_gates: u64::from(
            entity_table(view, EntityKind::ManeuverGate, mismatch)?.row_count(),
        ),
        waiting_zones: u64::from(
            entity_table(view, EntityKind::WaitingZone, mismatch)?.row_count(),
        ),
        ..CanonicalStatistics::default()
    };
    for role in SourceRelationRole::ALL {
        if let Some(projection) = lfca_projection(role) {
            statistics
                .relation_roles
                .add(role, projection_count(view, projection, mismatch)?);
        }
    }
    for (ordinal, points_tag) in GEOMETRY_POINT_FIELDS {
        for row in table(view, SPATIAL_SECTION, ordinal, mismatch)?.rows() {
            statistics.geometry_points = statistics.geometry_points.saturating_add(u64::from(
                checked_record_vector_with(row, points_tag, mismatch)?.len(),
            ));
        }
    }
    Ok(statistics)
}

#[cfg(test)]
mod tests;
