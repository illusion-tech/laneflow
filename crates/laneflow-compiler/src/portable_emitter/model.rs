use super::*;

/// LFSM 来源位置值的借用投影：文本来源（模块/文档 ordinal 与起止行列）或道路编辑
/// 来源（主题种类、命名空间、owner-local 键、属性路径与画布选择）。字符串与属性路径
/// 借用受检来源映射的冻结位置上下文，不复制载荷。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum LocationValue<'a> {
    Text {
        source_module_ordinal: u32,
        source_document_ordinal: u32,
        start_line: u32,
        start_column: u32,
        end_line: u32,
        end_column: u32,
    },
    RoadEditing {
        source_module_ordinal: u32,
        source_document_ordinal: u32,
        subject_kind: u8,
        module_namespace: Option<&'a str>,
        entity_kind: Option<u16>,
        owner_local_keys: [Option<&'a str>; 3],
        local_key: Option<&'a str>,
        owner_kind: Option<u8>,
        relation_kind: Option<u8>,
        occurrence_kind: Option<u8>,
        occurrence_ordinal: Option<u32>,
        property_steps: Option<PropertySteps<'a>>,
        canvas_selection: Option<&'a str>,
    },
}

/// 道路编辑属性路径的借用视图；按逐步（步骤种类, 容器码, 成员码）的字典序比较。
#[derive(Clone, Copy, Debug)]
pub(super) struct PropertySteps<'a>(pub(super) &'a [crate::RoadEditingPropertyStep]);

impl PropertySteps<'_> {
    /// 逐步的（步骤种类, 容器码, 成员码）。
    pub(super) fn codes(self) -> impl Iterator<Item = (u8, u16, u16)> {
        self.0.iter().map(|step| step_code(*step))
    }
}

impl PartialEq for PropertySteps<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.codes().eq(other.codes())
    }
}

impl Eq for PropertySteps<'_> {}

impl PartialOrd for PropertySteps<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PropertySteps<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.codes().cmp(other.codes())
    }
}

/// 同一段连续存储内的半开区间。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Slots {
    pub(super) start: usize,
    pub(super) end: usize,
}

impl Slots {
    /// 区间内的元素。
    pub(super) fn of<T>(self, values: &[T]) -> &[T] {
        &values[self.start..self.end]
    }

    /// 区间是否为空。
    pub(super) const fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// 按稳定身份定位实体的来源投影（LFSM StableSource 行）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct StableSourceProjection<'a> {
    pub(super) entity_kind: EntityKind,
    pub(super) stable_id: [u8; 16],
    pub(super) typed_ordinal: u32,
    pub(super) primary: LocationValue<'a>,
    /// 贡献来源在贡献来源存储中的区间。
    pub(super) contributing: Slots,
    /// 策略移动方向来源追加的贡献来源。
    pub(super) direction: Option<LocationValue<'a>>,
    /// 贡献来源序号集合在序号集合存储中的区间。
    pub(super) set: Slots,
}

/// 按所有者局部位置定位的来源投影（LFSM OwnerLocalSource 行）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OwnerLocalProjection<'a> {
    pub(super) owner_entity_kind: EntityKind,
    pub(super) owner_stable_id: [u8; 16],
    pub(super) role: u8,
    pub(super) local_index: u32,
    pub(super) primary: LocationValue<'a>,
    /// 贡献来源在贡献来源存储中的区间。
    pub(super) contributing: Slots,
    /// 贡献来源序号集合的区间。
    pub(super) set: Slots,
    /// 派生约束来源（贡献来源与主来源）序号集合的区间；只有角色 9 非空。
    pub(super) derived_set: Slots,
}

/// 空间几何点区间的来源投影：所有者实体、角色、局部下标、点半开区间与来源位置。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SpatialRangeProjection<'a> {
    pub(super) owner_entity_kind: EntityKind,
    pub(super) owner_stable_id: [u8; 16],
    pub(super) role: u8,
    pub(super) local_index: u32,
    pub(super) point_start: u32,
    pub(super) point_end_exclusive: u32,
    pub(super) source_segment_ordinal: u32,
    pub(super) source: LocationValue<'a>,
}

/// 道路编辑属性路径步骤的（步骤种类, 容器码, 成员码）。
fn step_code(step: crate::RoadEditingPropertyStep) -> (u8, u16, u16) {
    match step {
        crate::RoadEditingPropertyStep::TableField { table, field_id } => {
            (0, road_editing_table_code(table), field_id)
        }
        crate::RoadEditingPropertyStep::StructMember {
            structure,
            member_id,
        } => (1, road_editing_struct_code(structure), u16::from(member_id)),
        crate::RoadEditingPropertyStep::UnionVariant {
            union,
            discriminant,
        } => (2, road_editing_union_code(union), u16::from(discriminant)),
    }
}

pub(super) fn road_editing_table_code(value: crate::RoadEditingTableKind) -> u16 {
    use crate::RoadEditingTableKind as K;
    match value {
        K::RoadEditingSource => 0,
        K::ModuleHeader => 1,
        K::Provenance => 2,
        K::LineSegment => 3,
        K::CubicBezierSegment => 4,
        K::CurveSegment => 5,
        K::CurveProgram => 6,
        K::RoadAlignment => 7,
        K::CorridorElement => 8,
        K::RoadCorridor => 9,
        K::RoadSection => 10,
        K::AuthoringLane => 11,
        K::LaneEdge => 12,
        K::Junction => 13,
        K::Movement => 14,
        K::ManeuverPath => 15,
        K::ManeuverGate => 16,
        K::WaitingZone => 17,
        K::StopLine => 18,
        K::SignalGroup => 19,
        K::SignalController => 20,
        K::SignalPhaseState => 21,
        K::SignalPhase => 22,
        K::ParkingFacility => 23,
        K::ParkingLaneAnchor => 24,
        K::ParkingSpaceGeometry => 25,
        K::ParkingSpace => 26,
        K::LaneGroup => 27,
        K::FacilityBand => 28,
        K::ParticipantClass => 29,
        K::AccessRegulation => 30,
        K::AccessRule => 31,
        K::IidmVehicleProfile => 32,
        K::VehicleProfile => 33,
        K::ConflictZoneRegion => 34,
        K::CanonicalFrame => 35,
        K::ConflictZone => 36,
        K::PathAnchor => 37,
        K::ConflictPassage => 38,
        K::ParticipantStream => 39,
        K::RightOfWayPolicySet => 40,
        K::PolicyEvidence => 41,
        K::PolicyGapProfile => 42,
        K::PolicyStreamRule => 43,
        K::PolicyGateRule => 44,
        K::CircularArcSegment => 45,
        K::ClothoidSegment => 46,
        K::BoundaryExtent => 47,
        K::BoundaryMember => 48,
        K::CrossingRule => 49,
        K::Guardrail => 50,
        K::Curb => 51,
        K::RoadVehicleGeometry => 52,
    }
}

pub(super) fn road_editing_struct_code(value: crate::RoadEditingStructKind) -> u16 {
    match value {
        crate::RoadEditingStructKind::Digest256 => 0,
        crate::RoadEditingStructKind::OptionalU64 => 1,
        crate::RoadEditingStructKind::Vec3F64 => 2,
        crate::RoadEditingStructKind::LinearWidthProfile => 3,
        crate::RoadEditingStructKind::Vec2F64 => 4,
        crate::RoadEditingStructKind::BoundaryStation => 5,
        crate::RoadEditingStructKind::CurbEnd => 6,
    }
}

pub(super) fn road_editing_union_code(value: crate::RoadEditingUnionKind) -> u16 {
    match value {
        crate::RoadEditingUnionKind::CurveSegmentGeometry => 0,
        crate::RoadEditingUnionKind::BoundaryPayload => 1,
    }
}

/// 一条规范关系元组：所有者端点、关系角色、所有者局部下标与主体端点，两端均以
/// 实体种类和稳定标识表达。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct RelationTuple {
    pub(super) owner_entity_kind: EntityKind,
    pub(super) owner_stable_id: [u8; 16],
    pub(super) role: u8,
    pub(super) local_index: u32,
    pub(super) subject_entity_kind: EntityKind,
    pub(super) subject_stable_id: [u8; 16],
}
