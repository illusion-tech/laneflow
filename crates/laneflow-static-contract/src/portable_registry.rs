//! 可移植规范制品的 section/table/field 静态登记。
//!
//! 本模块逐项转录当前 LFCA（`formatVersion = 4`）、LFSM/LFSD（封套版本 3）与 LFCP 的线格式形状。它是可供 emitter 与
//! 结构预检共享的只读数据，不包含序列化器、文件系统发布、
//! 跨表语义验证或摘要信任判断。

use crate::{PortableFieldType, PortableObjectKind};

/// 一个字段在均一行或按判别值选择的行变体中的存在性。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortableFieldPresence {
    /// 每一行都必须存在。
    Required,
    /// 行允许省略该字段。
    Optional,
    /// 是否必需或禁止由 [`PortableRowShape::DiscriminatedU8`] 决定。
    ByRowVariant,
}

/// 一行中登记的字段及其可选内嵌 RecordVector 行 schema。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortableFieldSchema {
    pub tag: u16,
    pub name: &'static str,
    pub field_type: PortableFieldType,
    pub presence: PortableFieldPresence,
    pub nested_row: Option<&'static PortableRowSchema>,
}

/// 按 `u8` 判别字段选择的精确字段存在性变体。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortableRowVariant {
    pub discriminant: u8,
    /// 必须存在的 tag 位图；tag N 对应 bit N。
    pub required_fields: u32,
    /// 唯一允许存在的 tag 位图；未置位 tag 必须缺失。
    pub allowed_fields: u32,
    /// 非零时，位图中的字段至少存在一个。
    pub at_least_one_field: u32,
}

/// 行字段存在性的闭合形状。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortableRowShape {
    /// 只使用各字段的 Required/Optional 标记。
    Uniform,
    /// 读取指定 `u8` 字段并选择精确 required/allowed matrix。
    DiscriminatedU8 {
        tag: u16,
        variants: &'static [PortableRowVariant],
    },
}

/// TableV1 或 RecordVector 中每一行的静态 schema。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortableRowSchema {
    pub fields: &'static [PortableFieldSchema],
    pub shape: PortableRowShape,
}

/// TableV1 顶层行数约束。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortableRowCardinality {
    Any,
    AtMostOne,
    ExactlyOne,
}

/// 一张 TableV1 的静态登记。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortableTableSchema {
    pub kind: u16,
    pub name: &'static str,
    pub row: &'static PortableRowSchema,
    pub cardinality: PortableRowCardinality,
}

/// 一个 section 的精确、有序 table 登记。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortableSectionSchema {
    pub kind: u16,
    pub name: &'static str,
    pub tables: &'static [PortableTableSchema],
}

/// 一类对象的精确、有序 section 登记。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortableObjectSchema {
    pub kind: PortableObjectKind,
    pub sections: &'static [PortableSectionSchema],
}

/// 返回 tag N 对应的 presence 位；v1 当前只登记 tag `1..=21`。
#[must_use]
pub const fn portable_field_mask(tag: u16) -> u32 {
    if tag < 32 { 1_u32 << tag } else { 0 }
}

const fn field(
    tag: u16,
    name: &'static str,
    field_type: PortableFieldType,
    presence: PortableFieldPresence,
) -> PortableFieldSchema {
    PortableFieldSchema {
        tag,
        name,
        field_type,
        presence,
        nested_row: None,
    }
}

const fn record_field(
    tag: u16,
    name: &'static str,
    presence: PortableFieldPresence,
    nested_row: &'static PortableRowSchema,
) -> PortableFieldSchema {
    PortableFieldSchema {
        tag,
        name,
        field_type: PortableFieldType::RecordVector,
        presence,
        nested_row: Some(nested_row),
    }
}

const fn table(
    kind: u16,
    name: &'static str,
    row: &'static PortableRowSchema,
    cardinality: PortableRowCardinality,
) -> PortableTableSchema {
    PortableTableSchema {
        kind,
        name,
        row,
        cardinality,
    }
}

const R: PortableFieldPresence = PortableFieldPresence::Required;
const O: PortableFieldPresence = PortableFieldPresence::Optional;
const V: PortableFieldPresence = PortableFieldPresence::ByRowVariant;
const ANY: PortableRowCardinality = PortableRowCardinality::Any;
const AT_MOST_ONE: PortableRowCardinality = PortableRowCardinality::AtMostOne;
const ONE: PortableRowCardinality = PortableRowCardinality::ExactlyOne;

// LFCA nested rows.

const IDENTITY_FIELD_ROW_FIELDS: &[PortableFieldSchema] = &[
    field(1, "identityFieldTag", PortableFieldType::U16, R),
    field(2, "value", PortableFieldType::Bytes, R),
];
const IDENTITY_FIELD_ROW: PortableRowSchema = PortableRowSchema {
    fields: IDENTITY_FIELD_ROW_FIELDS,
    shape: PortableRowShape::Uniform,
};

const CORRIDOR_ELEMENT_ROW_FIELDS: &[PortableFieldSchema] = &[
    field(1, "elementKind", PortableFieldType::U8, R),
    field(2, "ordinal", PortableFieldType::U32, R),
];
const CORRIDOR_ELEMENT_ROW: PortableRowSchema = PortableRowSchema {
    fields: CORRIDOR_ELEMENT_ROW_FIELDS,
    shape: PortableRowShape::Uniform,
};

const SIGNAL_PHASE_STATE_ROW_FIELDS: &[PortableFieldSchema] = &[
    field(1, "signalGroup", PortableFieldType::U32, R),
    field(2, "aspect", PortableFieldType::U8, R),
];
const SIGNAL_PHASE_STATE_ROW: PortableRowSchema = PortableRowSchema {
    fields: SIGNAL_PHASE_STATE_ROW_FIELDS,
    shape: PortableRowShape::Uniform,
};

const ACCESS_REGULATION_ROW_FIELDS: &[PortableFieldSchema] = &[
    field(1, "jurisdiction", PortableFieldType::Utf8, R),
    field(2, "version", PortableFieldType::Utf8, R),
    field(3, "source", PortableFieldType::Utf8, O),
];
const ACCESS_REGULATION_ROW: PortableRowSchema = PortableRowSchema {
    fields: ACCESS_REGULATION_ROW_FIELDS,
    shape: PortableRowShape::Uniform,
};

const POINT_ROW_FIELDS: &[PortableFieldSchema] = &[
    field(1, "x", PortableFieldType::F32, R),
    field(2, "y", PortableFieldType::F32, R),
    field(3, "z", PortableFieldType::F32, R),
];
const POINT_ROW: PortableRowSchema = PortableRowSchema {
    fields: POINT_ROW_FIELDS,
    shape: PortableRowShape::Uniform,
};

const POINT_XZ_ROW_FIELDS: &[PortableFieldSchema] = &[
    field(1, "x", PortableFieldType::F32, R),
    field(2, "z", PortableFieldType::F32, R),
];
const POINT_XZ_ROW: PortableRowSchema = PortableRowSchema {
    fields: POINT_XZ_ROW_FIELDS,
    shape: PortableRowShape::Uniform,
};

const SEGMENT_ROW_FIELDS: &[PortableFieldSchema] = &[
    field(1, "lengthMeters", PortableFieldType::F32, R),
    field(2, "cumulativeEndMeters", PortableFieldType::F32, R),
    field(3, "tangentX", PortableFieldType::F32, R),
    field(4, "tangentY", PortableFieldType::F32, R),
    field(5, "tangentZ", PortableFieldType::F32, R),
    field(6, "upX", PortableFieldType::F32, R),
    field(7, "upY", PortableFieldType::F32, R),
    field(8, "upZ", PortableFieldType::F32, R),
];
const SEGMENT_ROW: PortableRowSchema = PortableRowSchema {
    fields: SEGMENT_ROW_FIELDS,
    shape: PortableRowShape::Uniform,
};

// LFCA section 0x0001..0x0002.

const CONTRACT_VERSIONS_FIELDS: &[PortableFieldSchema] = &[
    field(1, "canonicalFormatVersion", PortableFieldType::U16, R),
    field(2, "identityEncodingVersion", PortableFieldType::U16, R),
    field(3, "identityRegistryRevision", PortableFieldType::U16, R),
    field(
        4,
        "networkRevisionDerivationVersion",
        PortableFieldType::U16,
        R,
    ),
    field(5, "constraintContractVersion", PortableFieldType::U16, R),
    field(
        6,
        "staticExecutionContractVersion",
        PortableFieldType::U16,
        R,
    ),
];
const CONTRACT_VERSIONS_ROW: PortableRowSchema = PortableRowSchema {
    fields: CONTRACT_VERSIONS_FIELDS,
    shape: PortableRowShape::Uniform,
};
const LFCA_SECTION_1_TABLES: &[PortableTableSchema] =
    &[table(1, "ContractVersions", &CONTRACT_VERSIONS_ROW, ONE)];

const CANONICAL_IDENTITY_FIELDS: &[PortableFieldSchema] = &[
    field(1, "entityKind", PortableFieldType::U16, R),
    field(2, "typedOrdinal", PortableFieldType::U32, R),
    field(3, "stableId", PortableFieldType::StableId128, R),
    record_field(4, "identityFields", R, &IDENTITY_FIELD_ROW),
];
const CANONICAL_IDENTITY_ROW: PortableRowSchema = PortableRowSchema {
    fields: CANONICAL_IDENTITY_FIELDS,
    shape: PortableRowShape::Uniform,
};
const LFCA_SECTION_2_TABLES: &[PortableTableSchema] =
    &[table(1, "CanonicalIdentity", &CANONICAL_IDENTITY_ROW, ANY)];

// LFCA section 0x0003: 23 constructible entity tables.

const ROAD_CORRIDOR_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "referenceSection", PortableFieldType::U32, R),
    record_field(4, "elements", R, &CORRIDOR_ELEMENT_ROW),
];
const ROAD_CORRIDOR_ROW: PortableRowSchema = PortableRowSchema {
    fields: ROAD_CORRIDOR_FIELDS,
    shape: PortableRowShape::Uniform,
};

const ROAD_SECTION_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "roadCorridor", PortableFieldType::U32, R),
    field(4, "kindId", PortableFieldType::Utf8, R),
    field(5, "lanes", PortableFieldType::OrdinalVectorU32, R),
];
const ROAD_SECTION_ROW: PortableRowSchema = PortableRowSchema {
    fields: ROAD_SECTION_FIELDS,
    shape: PortableRowShape::Uniform,
};

const AUTHORING_LANE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "roadSection", PortableFieldType::U32, R),
    field(4, "edgeChain", PortableFieldType::OrdinalVectorU32, R),
    field(5, "laneGroup", PortableFieldType::U32, O),
];
const AUTHORING_LANE_ROW: PortableRowSchema = PortableRowSchema {
    fields: AUTHORING_LANE_FIELDS,
    shape: PortableRowShape::Uniform,
};

const LANE_EDGE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "lengthMillimetres", PortableFieldType::U32, R),
    field(
        4,
        "speedLimitMillimetresPerSecond",
        PortableFieldType::U32,
        R,
    ),
    field(5, "successors", PortableFieldType::OrdinalVectorU32, R),
];
const LANE_EDGE_ROW: PortableRowSchema = PortableRowSchema {
    fields: LANE_EDGE_FIELDS,
    shape: PortableRowShape::Uniform,
};

const JUNCTION_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "movements", PortableFieldType::OrdinalVectorU32, R),
];
const JUNCTION_ROW: PortableRowSchema = PortableRowSchema {
    fields: JUNCTION_FIELDS,
    shape: PortableRowShape::Uniform,
};

const MOVEMENT_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "junction", PortableFieldType::U32, R),
    field(4, "directedEntryApproachKey", PortableFieldType::Utf8, R),
    field(5, "directedExitApproachKey", PortableFieldType::Utf8, R),
    field(6, "maneuverPaths", PortableFieldType::OrdinalVectorU32, R),
    field(7, "turnDirection", PortableFieldType::U8, O),
];
const MOVEMENT_ROW: PortableRowSchema = PortableRowSchema {
    fields: MOVEMENT_FIELDS,
    shape: PortableRowShape::Uniform,
};

const MANEUVER_PATH_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "movement", PortableFieldType::U32, R),
    field(4, "edges", PortableFieldType::OrdinalVectorU32, R),
    field(5, "maneuverGates", PortableFieldType::OrdinalVectorU32, R),
    field(6, "waitingZones", PortableFieldType::OrdinalVectorU32, R),
];
const MANEUVER_PATH_ROW: PortableRowSchema = PortableRowSchema {
    fields: MANEUVER_PATH_FIELDS,
    shape: PortableRowShape::Uniform,
};

const MANEUVER_GATE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "maneuverPath", PortableFieldType::U32, R),
    field(4, "transitionIndex", PortableFieldType::U32, R),
    field(5, "stopLine", PortableFieldType::U32, R),
    field(6, "signalControlKind", PortableFieldType::U8, R),
    field(7, "signalGroup", PortableFieldType::U32, O),
];
const MANEUVER_GATE_ROW: PortableRowSchema = PortableRowSchema {
    fields: MANEUVER_GATE_FIELDS,
    shape: PortableRowShape::Uniform,
};

const WAITING_ZONE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "maneuverPath", PortableFieldType::U32, R),
    field(4, "entryGate", PortableFieldType::U32, R),
    field(5, "releaseGate", PortableFieldType::U32, R),
    field(6, "maxOccupancy", PortableFieldType::U32, R),
];
const WAITING_ZONE_ROW: PortableRowSchema = PortableRowSchema {
    fields: WAITING_ZONE_FIELDS,
    shape: PortableRowShape::Uniform,
};

const STOP_LINE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "laneEdge", PortableFieldType::U32, R),
    field(4, "maneuverGates", PortableFieldType::OrdinalVectorU32, R),
];
const STOP_LINE_ROW: PortableRowSchema = PortableRowSchema {
    fields: STOP_LINE_FIELDS,
    shape: PortableRowShape::Uniform,
};

const SIGNAL_GROUP_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "controller", PortableFieldType::U32, R),
    field(4, "maneuverGates", PortableFieldType::OrdinalVectorU32, R),
];
const SIGNAL_GROUP_ROW: PortableRowSchema = PortableRowSchema {
    fields: SIGNAL_GROUP_FIELDS,
    shape: PortableRowShape::Uniform,
};

const SIGNAL_CONTROLLER_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "offsetMs", PortableFieldType::U64, R),
    field(4, "cycleDurationMs", PortableFieldType::U64, R),
    field(5, "signalGroups", PortableFieldType::OrdinalVectorU32, R),
    field(6, "phases", PortableFieldType::OrdinalVectorU32, R),
];
const SIGNAL_CONTROLLER_ROW: PortableRowSchema = PortableRowSchema {
    fields: SIGNAL_CONTROLLER_FIELDS,
    shape: PortableRowShape::Uniform,
};

const SIGNAL_PHASE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "controller", PortableFieldType::U32, R),
    field(4, "durationMs", PortableFieldType::U64, R),
    record_field(5, "states", R, &SIGNAL_PHASE_STATE_ROW),
];
const SIGNAL_PHASE_ROW: PortableRowSchema = PortableRowSchema {
    fields: SIGNAL_PHASE_FIELDS,
    shape: PortableRowShape::Uniform,
};

const PARKING_LANE_ANCHOR_FIELDS: &[PortableFieldSchema] = &[
    field(1, "laneEdge", PortableFieldType::U32, R),
    field(2, "progressMillimetres", PortableFieldType::U32, R),
];
const PARKING_LANE_ANCHOR_ROW: PortableRowSchema = PortableRowSchema {
    fields: PARKING_LANE_ANCHOR_FIELDS,
    shape: PortableRowShape::Uniform,
};

const PARKING_FACILITY_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "parkingSpaces", PortableFieldType::OrdinalVectorU32, R),
    field(4, "virtualCapacity", PortableFieldType::U32, R),
    record_field(5, "virtualEntries", R, &PARKING_LANE_ANCHOR_ROW),
    record_field(6, "virtualExits", R, &PARKING_LANE_ANCHOR_ROW),
];
const PARKING_FACILITY_ROW: PortableRowSchema = PortableRowSchema {
    fields: PARKING_FACILITY_FIELDS,
    shape: PortableRowShape::Uniform,
};

const PARKING_SPACE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "parkingFacility", PortableFieldType::U32, O),
    field(4, "entryLaneEdge", PortableFieldType::U32, R),
    field(5, "entryProgressMillimetres", PortableFieldType::U32, R),
    field(6, "exitLaneEdge", PortableFieldType::U32, R),
    field(7, "exitProgressMillimetres", PortableFieldType::U32, R),
    field(8, "lateralOffsetMillimetres", PortableFieldType::I32, R),
    field(9, "headingOffsetRadians", PortableFieldType::F32, R),
    field(10, "lengthMillimetres", PortableFieldType::U32, R),
    field(11, "widthMillimetres", PortableFieldType::U32, R),
];
const PARKING_SPACE_ROW: PortableRowSchema = PortableRowSchema {
    fields: PARKING_SPACE_FIELDS,
    shape: PortableRowShape::Uniform,
};

const LANE_GROUP_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "roadSection", PortableFieldType::U32, R),
    field(4, "members", PortableFieldType::OrdinalVectorU32, R),
];
const LANE_GROUP_ROW: PortableRowSchema = PortableRowSchema {
    fields: LANE_GROUP_FIELDS,
    shape: PortableRowShape::Uniform,
};

const FACILITY_BAND_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "roadCorridor", PortableFieldType::U32, R),
    field(4, "kindId", PortableFieldType::Utf8, R),
];
const FACILITY_BAND_ROW: PortableRowSchema = PortableRowSchema {
    fields: FACILITY_BAND_FIELDS,
    shape: PortableRowShape::Uniform,
};

const PARTICIPANT_CLASS_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "parent", PortableFieldType::U32, O),
    field(4, "depth", PortableFieldType::U32, R),
    field(5, "subtreeEnter", PortableFieldType::U32, R),
    field(6, "subtreeExit", PortableFieldType::U32, R),
];
const PARTICIPANT_CLASS_ROW: PortableRowSchema = PortableRowSchema {
    fields: PARTICIPANT_CLASS_FIELDS,
    shape: PortableRowShape::Uniform,
};

const ACCESS_RULE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "targetKind", PortableFieldType::U8, R),
    field(4, "targetOrdinal", PortableFieldType::U32, R),
    field(5, "effect", PortableFieldType::U8, R),
    field(
        6,
        "participantClasses",
        PortableFieldType::OrdinalVectorU32,
        R,
    ),
    record_field(7, "regulation", O, &ACCESS_REGULATION_ROW),
    field(8, "priority", PortableFieldType::I32, R),
];
const ACCESS_RULE_ROW: PortableRowSchema = PortableRowSchema {
    fields: ACCESS_RULE_FIELDS,
    shape: PortableRowShape::Uniform,
};

const VEHICLE_PROFILE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "participantClass", PortableFieldType::U32, R),
    field(4, "lengthMillimetres", PortableFieldType::U32, R),
    field(
        5,
        "desiredSpeedMillimetresPerSecond",
        PortableFieldType::U32,
        R,
    ),
    field(6, "minGapMillimetres", PortableFieldType::U32, R),
    field(7, "timeHeadwaySeconds", PortableFieldType::F32, R),
    field(
        8,
        "maxAccelerationMetersPerSecondSquared",
        PortableFieldType::F32,
        R,
    ),
    field(
        9,
        "comfortableDecelerationMetersPerSecondSquared",
        PortableFieldType::F32,
        R,
    ),
    field(
        10,
        "emergencyDecelerationMetersPerSecondSquared",
        PortableFieldType::F32,
        R,
    ),
];
const VEHICLE_PROFILE_ROW: PortableRowSchema = PortableRowSchema {
    fields: VEHICLE_PROFILE_FIELDS,
    shape: PortableRowShape::Uniform,
};

const CONFLICT_ZONE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "junction", PortableFieldType::U32, R),
];
const CONFLICT_ZONE_ROW: PortableRowSchema = PortableRowSchema {
    fields: CONFLICT_ZONE_FIELDS,
    shape: PortableRowShape::Uniform,
};

const CANONICAL_FRAME_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
];
const CANONICAL_FRAME_ROW: PortableRowSchema = PortableRowSchema {
    fields: CANONICAL_FRAME_FIELDS,
    shape: PortableRowShape::Uniform,
};

const CONFLICT_PASSAGE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "conflictZone", PortableFieldType::U32, R),
    field(2, "entryKind", PortableFieldType::U8, R),
    field(3, "entryReference", PortableFieldType::U32, R),
    field(4, "entryProgressMillimetres", PortableFieldType::U32, O),
    field(5, "exitKind", PortableFieldType::U8, R),
    field(6, "exitReference", PortableFieldType::U32, R),
    field(7, "exitProgressMillimetres", PortableFieldType::U32, O),
];
const CONFLICT_PASSAGE_ROW: PortableRowSchema = PortableRowSchema {
    fields: CONFLICT_PASSAGE_FIELDS,
    shape: PortableRowShape::Uniform,
};

const PARTICIPANT_STREAM_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "junction", PortableFieldType::U32, R),
    field(4, "maneuverPath", PortableFieldType::U32, R),
    record_field(5, "passages", R, &CONFLICT_PASSAGE_ROW),
];
const PARTICIPANT_STREAM_ROW: PortableRowSchema = PortableRowSchema {
    fields: PARTICIPANT_STREAM_FIELDS,
    shape: PortableRowShape::Uniform,
};

const RIGHT_OF_WAY_POLICY_SET_FIELDS: &[PortableFieldSchema] = &[
    field(1, "typedOrdinal", PortableFieldType::U32, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "jurisdiction", PortableFieldType::Utf8, R),
    field(4, "regulationVersion", PortableFieldType::Utf8, R),
    field(5, "regulationSource", PortableFieldType::Utf8, O),
];
const RIGHT_OF_WAY_POLICY_SET_ROW: PortableRowSchema = PortableRowSchema {
    fields: RIGHT_OF_WAY_POLICY_SET_FIELDS,
    shape: PortableRowShape::Uniform,
};

const LFCA_SECTION_3_TABLES: &[PortableTableSchema] = &[
    table(1, "RoadCorridor", &ROAD_CORRIDOR_ROW, ANY),
    table(2, "RoadSection", &ROAD_SECTION_ROW, ANY),
    table(3, "AuthoringLane", &AUTHORING_LANE_ROW, ANY),
    table(4, "LaneEdge", &LANE_EDGE_ROW, ANY),
    table(5, "Junction", &JUNCTION_ROW, ANY),
    table(6, "Movement", &MOVEMENT_ROW, ANY),
    table(7, "ManeuverPath", &MANEUVER_PATH_ROW, ANY),
    table(8, "ManeuverGate", &MANEUVER_GATE_ROW, ANY),
    table(9, "WaitingZone", &WAITING_ZONE_ROW, ANY),
    table(10, "StopLine", &STOP_LINE_ROW, ANY),
    table(11, "SignalGroup", &SIGNAL_GROUP_ROW, ANY),
    table(12, "SignalController", &SIGNAL_CONTROLLER_ROW, ANY),
    table(13, "SignalPhase", &SIGNAL_PHASE_ROW, ANY),
    table(14, "ParkingFacility", &PARKING_FACILITY_ROW, ANY),
    table(15, "ParkingSpace", &PARKING_SPACE_ROW, ANY),
    table(16, "LaneGroup", &LANE_GROUP_ROW, ANY),
    table(17, "FacilityBand", &FACILITY_BAND_ROW, ANY),
    table(18, "ParticipantClass", &PARTICIPANT_CLASS_ROW, ANY),
    table(19, "AccessRule", &ACCESS_RULE_ROW, ANY),
    table(20, "VehicleProfile", &VEHICLE_PROFILE_ROW, ANY),
    table(21, "ConflictZone", &CONFLICT_ZONE_ROW, ANY),
    table(22, "CanonicalFrame", &CANONICAL_FRAME_ROW, ANY),
    table(23, "ParticipantStream", &PARTICIPANT_STREAM_ROW, ANY),
    table(24, "RightOfWayPolicySet", &RIGHT_OF_WAY_POLICY_SET_ROW, ANY),
];

// LFCA section 0x0004: canonical relation tables.

const JUNCTION_INTERNAL_EDGE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "laneEdge", PortableFieldType::U32, R),
    field(2, "junction", PortableFieldType::U32, R),
];
const JUNCTION_INTERNAL_EDGE_ROW: PortableRowSchema = PortableRowSchema {
    fields: JUNCTION_INTERNAL_EDGE_FIELDS,
    shape: PortableRowShape::Uniform,
};
const POLICY_EVIDENCE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "policy", PortableFieldType::U32, R),
    field(2, "key", PortableFieldType::Utf8, R),
    field(3, "locator", PortableFieldType::Utf8, R),
    field(4, "description", PortableFieldType::Utf8, O),
];
const POLICY_EVIDENCE_ROW: PortableRowSchema = PortableRowSchema {
    fields: POLICY_EVIDENCE_FIELDS,
    shape: PortableRowShape::Uniform,
};
const POLICY_GAP_PROFILE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "policy", PortableFieldType::U32, R),
    field(2, "key", PortableFieldType::Utf8, R),
    field(3, "parameterVersion", PortableFieldType::Utf8, R),
    field(4, "minimumLeadGapMs", PortableFieldType::U64, R),
    field(5, "minimumLagGapMs", PortableFieldType::U64, R),
    field(6, "clearanceBufferMs", PortableFieldType::U64, R),
];
const POLICY_GAP_PROFILE_ROW: PortableRowSchema = PortableRowSchema {
    fields: POLICY_GAP_PROFILE_FIELDS,
    shape: PortableRowShape::Uniform,
};
const POLICY_EVIDENCE_KEY_FIELDS: &[PortableFieldSchema] =
    &[field(1, "key", PortableFieldType::Utf8, R)];
const POLICY_EVIDENCE_KEY_ROW: PortableRowSchema = PortableRowSchema {
    fields: POLICY_EVIDENCE_KEY_FIELDS,
    shape: PortableRowShape::Uniform,
};
const POLICY_STREAM_RULE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "policy", PortableFieldType::U32, R),
    field(2, "key", PortableFieldType::Utf8, R),
    field(3, "stream", PortableFieldType::U32, R),
    field(4, "classes", PortableFieldType::OrdinalVectorU32, O),
    field(5, "priority", PortableFieldType::I32, R),
    field(6, "yieldToStreams", PortableFieldType::OrdinalVectorU32, R),
    field(7, "gapProfileKey", PortableFieldType::Utf8, O),
    record_field(8, "evidenceKeys", R, &POLICY_EVIDENCE_KEY_ROW),
];
const POLICY_STREAM_RULE_ROW: PortableRowSchema = PortableRowSchema {
    fields: POLICY_STREAM_RULE_FIELDS,
    shape: PortableRowShape::Uniform,
};
const POLICY_GATE_RULE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "policy", PortableFieldType::U32, R),
    field(2, "key", PortableFieldType::Utf8, R),
    field(3, "gate", PortableFieldType::U32, R),
    field(4, "classes", PortableFieldType::OrdinalVectorU32, O),
    field(5, "interpretation", PortableFieldType::U8, R),
    field(6, "prohibition", PortableFieldType::U8, R),
    record_field(7, "evidenceKeys", R, &POLICY_EVIDENCE_KEY_ROW),
];
const POLICY_GATE_RULE_ROW: PortableRowSchema = PortableRowSchema {
    fields: POLICY_GATE_RULE_FIELDS,
    shape: PortableRowShape::Uniform,
};
const LFCA_SECTION_4_TABLES: &[PortableTableSchema] = &[
    table(1, "JunctionInternalEdge", &JUNCTION_INTERNAL_EDGE_ROW, ANY),
    table(2, "PolicyEvidence", &POLICY_EVIDENCE_ROW, ANY),
    table(3, "PolicyGapProfile", &POLICY_GAP_PROFILE_ROW, ANY),
    table(4, "PolicyStreamRule", &POLICY_STREAM_RULE_ROW, ANY),
    table(5, "PolicyGateRule", &POLICY_GATE_RULE_ROW, ANY),
];

// LFCA section 0x0005..0x0008.

const SPATIAL_PRESENCE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "spatialPresent", PortableFieldType::U8, R),
    field(2, "geometryDirectionProfile", PortableFieldType::U8, R),
];
const SPATIAL_PRESENCE_ROW: PortableRowSchema = PortableRowSchema {
    fields: SPATIAL_PRESENCE_FIELDS,
    shape: PortableRowShape::Uniform,
};
const LANE_EDGE_GEOMETRY_FIELDS: &[PortableFieldSchema] = &[
    field(1, "laneEdge", PortableFieldType::U32, R),
    field(2, "canonicalFrame", PortableFieldType::U32, R),
    field(3, "arcLengthMeters", PortableFieldType::F32, R),
    record_field(4, "points", R, &POINT_ROW),
    record_field(5, "segments", R, &SEGMENT_ROW),
    field(6, "directionProfileApplies", PortableFieldType::U8, R),
];
const LANE_EDGE_GEOMETRY_ROW: PortableRowSchema = PortableRowSchema {
    fields: LANE_EDGE_GEOMETRY_FIELDS,
    shape: PortableRowShape::Uniform,
};
const FACILITY_BAND_GEOMETRY_FIELDS: &[PortableFieldSchema] = &[
    field(1, "facilityBand", PortableFieldType::U32, R),
    field(2, "canonicalFrame", PortableFieldType::U32, R),
    record_field(3, "points", R, &POINT_ROW),
    field(4, "directionProfileApplies", PortableFieldType::U8, R),
];
const FACILITY_BAND_GEOMETRY_ROW: PortableRowSchema = PortableRowSchema {
    fields: FACILITY_BAND_GEOMETRY_FIELDS,
    shape: PortableRowShape::Uniform,
};
const CONFLICT_ZONE_REGION_FIELDS: &[PortableFieldSchema] = &[
    field(1, "conflictZone", PortableFieldType::U32, R),
    field(2, "canonicalFrame", PortableFieldType::U32, R),
    field(3, "minY", PortableFieldType::F32, R),
    field(4, "maxY", PortableFieldType::F32, R),
    record_field(5, "ringXZ", R, &POINT_XZ_ROW),
];
const CONFLICT_ZONE_REGION_ROW: PortableRowSchema = PortableRowSchema {
    fields: CONFLICT_ZONE_REGION_FIELDS,
    shape: PortableRowShape::Uniform,
};
const LFCA_SECTION_5_TABLES: &[PortableTableSchema] = &[
    table(1, "SpatialPresence", &SPATIAL_PRESENCE_ROW, ONE),
    table(2, "LaneEdgeGeometry", &LANE_EDGE_GEOMETRY_ROW, ANY),
    table(3, "FacilityBandGeometry", &FACILITY_BAND_GEOMETRY_ROW, ANY),
    table(4, "ConflictZoneRegion", &CONFLICT_ZONE_REGION_ROW, ANY),
];

const EXECUTION_CONTRACT_FIELDS: &[PortableFieldSchema] = &[
    field(
        1,
        "staticExecutionContractVersion",
        PortableFieldType::U16,
        R,
    ),
    field(2, "constraintContractVersion", PortableFieldType::U16, R),
];
const EXECUTION_CONTRACT_ROW: PortableRowSchema = PortableRowSchema {
    fields: EXECUTION_CONTRACT_FIELDS,
    shape: PortableRowShape::Uniform,
};
const LFCA_SECTION_6_TABLES: &[PortableTableSchema] =
    &[table(1, "ExecutionContract", &EXECUTION_CONTRACT_ROW, ONE)];

const COMPILER_PROVENANCE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "compilerBuildId", PortableFieldType::Utf8, R),
    field(
        2,
        "sourceCollectionDigestVersion",
        PortableFieldType::U16,
        R,
    ),
    field(3, "sourceCollectionDigest", PortableFieldType::Sha256, R),
    field(4, "compileOptionsDigest", PortableFieldType::Sha256, R),
    field(5, "emitterVersion", PortableFieldType::U16, R),
    field(6, "geometryAccuracyProfile", PortableFieldType::U8, R),
];
const COMPILER_PROVENANCE_ROW: PortableRowSchema = PortableRowSchema {
    fields: COMPILER_PROVENANCE_FIELDS,
    shape: PortableRowShape::Uniform,
};
const LFCA_SECTION_7_TABLES: &[PortableTableSchema] = &[table(
    1,
    "CompilerProvenance",
    &COMPILER_PROVENANCE_ROW,
    ONE,
)];

const ARTIFACT_CLAIMS_FIELDS: &[PortableFieldSchema] = &[field(
    1,
    "declaredNetworkRevisionId",
    PortableFieldType::Sha256,
    R,
)];
const ARTIFACT_CLAIMS_ROW: PortableRowSchema = PortableRowSchema {
    fields: ARTIFACT_CLAIMS_FIELDS,
    shape: PortableRowShape::Uniform,
};
const LFCA_SECTION_8_TABLES: &[PortableTableSchema] =
    &[table(1, "ArtifactClaims", &ARTIFACT_CLAIMS_ROW, ONE)];

const LFCA_SECTIONS: &[PortableSectionSchema] = &[
    PortableSectionSchema {
        kind: 1,
        name: "ContractVersions",
        tables: LFCA_SECTION_1_TABLES,
    },
    PortableSectionSchema {
        kind: 2,
        name: "CanonicalIdentityTable",
        tables: LFCA_SECTION_2_TABLES,
    },
    PortableSectionSchema {
        kind: 3,
        name: "CanonicalEntityTables",
        tables: LFCA_SECTION_3_TABLES,
    },
    PortableSectionSchema {
        kind: 4,
        name: "CanonicalRelationTables",
        tables: LFCA_SECTION_4_TABLES,
    },
    PortableSectionSchema {
        kind: 5,
        name: "CanonicalSpatialTables",
        tables: LFCA_SECTION_5_TABLES,
    },
    PortableSectionSchema {
        kind: 6,
        name: "StaticExecutionConstraints",
        tables: LFCA_SECTION_6_TABLES,
    },
    PortableSectionSchema {
        kind: 7,
        name: "CompilerProvenance",
        tables: LFCA_SECTION_7_TABLES,
    },
    PortableSectionSchema {
        kind: 8,
        name: "ArtifactClaims",
        tables: LFCA_SECTION_8_TABLES,
    },
];

// LFSM.

const IMPORT_ROW_FIELDS: &[PortableFieldSchema] =
    &[field(1, "authoringNamespaceId", PortableFieldType::Utf8, R)];
const IMPORT_ROW: PortableRowSchema = PortableRowSchema {
    fields: IMPORT_ROW_FIELDS,
    shape: PortableRowShape::Uniform,
};

const PROPERTY_STEP_FIELDS: &[PortableFieldSchema] = &[
    field(1, "stepKind", PortableFieldType::U8, R),
    field(2, "containerCode", PortableFieldType::U16, R),
    field(3, "memberCode", PortableFieldType::U16, R),
];
const PROPERTY_STEP_ROW: PortableRowSchema = PortableRowSchema {
    fields: PROPERTY_STEP_FIELDS,
    shape: PortableRowShape::Uniform,
};

const SOURCE_MAP_BINDINGS_FIELDS: &[PortableFieldSchema] = &[
    field(
        1,
        "networkRevisionDerivationVersion",
        PortableFieldType::U16,
        R,
    ),
    field(2, "networkRevision", PortableFieldType::Sha256, R),
    field(
        3,
        "canonicalArtifactFormatVersion",
        PortableFieldType::U16,
        R,
    ),
    field(4, "canonicalArtifactDigest", PortableFieldType::Sha256, R),
    field(5, "canonicalArtifactByteLength", PortableFieldType::U64, R),
    field(6, "compilerBuildId", PortableFieldType::Utf8, R),
    field(
        7,
        "sourceCollectionDigestVersion",
        PortableFieldType::U16,
        R,
    ),
    field(8, "sourceCollectionDigest", PortableFieldType::Sha256, R),
];
const SOURCE_MAP_BINDINGS_ROW: PortableRowSchema = PortableRowSchema {
    fields: SOURCE_MAP_BINDINGS_FIELDS,
    shape: PortableRowShape::Uniform,
};
const LFSM_SECTION_1_TABLES: &[PortableTableSchema] =
    &[table(1, "SourceMapBindings", &SOURCE_MAP_BINDINGS_ROW, ONE)];

const SOURCE_MODULE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "sourceModuleOrdinal", PortableFieldType::U32, R),
    field(2, "authoringNamespaceId", PortableFieldType::Utf8, R),
    field(3, "sourceLanguage", PortableFieldType::U16, R),
    field(4, "sourceDocumentSetDigest", PortableFieldType::Sha256, R),
    field(
        5,
        "sourceDocumentSetDigestVersion",
        PortableFieldType::U32,
        R,
    ),
    field(6, "frontendVersion", PortableFieldType::U32, R),
    field(7, "frontendOptionsDigest", PortableFieldType::Sha256, R),
    field(8, "generatorBuildId", PortableFieldType::Utf8, R),
    field(9, "parametersAndInputsDigest", PortableFieldType::Sha256, R),
    field(10, "randomSeed", PortableFieldType::U64, O),
    field(11, "provenance", PortableFieldType::Utf8, R),
    record_field(12, "imports", R, &IMPORT_ROW),
    field(13, "primaryLocation", PortableFieldType::U32, R),
];
const SOURCE_MODULE_ROW: PortableRowSchema = PortableRowSchema {
    fields: SOURCE_MODULE_FIELDS,
    shape: PortableRowShape::Uniform,
};

const SOURCE_DOCUMENT_FIELDS: &[PortableFieldSchema] = &[
    field(1, "sourceDocumentOrdinal", PortableFieldType::U32, R),
    field(2, "sourceModuleOrdinal", PortableFieldType::U32, R),
    field(3, "sourceDocumentKey", PortableFieldType::Utf8, R),
    field(4, "sourceContentDigest", PortableFieldType::Sha256, R),
    field(5, "sourceRecordByteLength", PortableFieldType::U32, R),
    field(6, "displaySource", PortableFieldType::Utf8, O),
];
const SOURCE_DOCUMENT_ROW: PortableRowSchema = PortableRowSchema {
    fields: SOURCE_DOCUMENT_FIELDS,
    shape: PortableRowShape::Uniform,
};

const SOURCE_LOCATION_FIELDS: &[PortableFieldSchema] = &[
    field(1, "sourceLocationOrdinal", PortableFieldType::U32, V),
    field(2, "sourceLocationKind", PortableFieldType::U8, V),
    field(3, "sourceModuleOrdinal", PortableFieldType::U32, V),
    field(4, "sourceDocumentOrdinal", PortableFieldType::U32, V),
    field(5, "startLine", PortableFieldType::U32, V),
    field(6, "startColumn", PortableFieldType::U32, V),
    field(7, "endLine", PortableFieldType::U32, V),
    field(8, "endColumn", PortableFieldType::U32, V),
    field(9, "roadEditingSubjectKind", PortableFieldType::U8, V),
    field(10, "moduleNamespace", PortableFieldType::Utf8, V),
    field(11, "entityKind", PortableFieldType::U16, V),
    field(12, "ownerLocalKey0", PortableFieldType::Utf8, V),
    field(13, "ownerLocalKey1", PortableFieldType::Utf8, V),
    field(14, "ownerLocalKey2", PortableFieldType::Utf8, V),
    field(15, "localKey", PortableFieldType::Utf8, V),
    field(16, "ownerKind", PortableFieldType::U8, V),
    field(17, "roadEditingRelationKind", PortableFieldType::U8, V),
    field(18, "occurrenceKind", PortableFieldType::U8, V),
    field(19, "occurrenceOrdinal", PortableFieldType::U32, V),
    record_field(20, "propertySteps", V, &PROPERTY_STEP_ROW),
    field(21, "canvasSelection", PortableFieldType::Utf8, V),
];
const SOURCE_LOCATION_VARIANTS: &[PortableRowVariant] = &[
    PortableRowVariant {
        discriminant: 0,
        required_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(3)
            | portable_field_mask(4)
            | portable_field_mask(5)
            | portable_field_mask(6)
            | portable_field_mask(7)
            | portable_field_mask(8),
        allowed_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(3)
            | portable_field_mask(4)
            | portable_field_mask(5)
            | portable_field_mask(6)
            | portable_field_mask(7)
            | portable_field_mask(8),
        at_least_one_field: 0,
    },
    PortableRowVariant {
        discriminant: 1,
        required_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(3)
            | portable_field_mask(4)
            | portable_field_mask(9),
        allowed_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(3)
            | portable_field_mask(4)
            | portable_field_mask(9)
            | portable_field_mask(10)
            | portable_field_mask(11)
            | portable_field_mask(12)
            | portable_field_mask(13)
            | portable_field_mask(14)
            | portable_field_mask(15)
            | portable_field_mask(16)
            | portable_field_mask(17)
            | portable_field_mask(18)
            | portable_field_mask(19)
            | portable_field_mask(20)
            | portable_field_mask(21),
        at_least_one_field: 0,
    },
];
const SOURCE_LOCATION_ROW: PortableRowSchema = PortableRowSchema {
    fields: SOURCE_LOCATION_FIELDS,
    shape: PortableRowShape::DiscriminatedU8 {
        tag: 2,
        variants: SOURCE_LOCATION_VARIANTS,
    },
};
const LFSM_SECTION_2_TABLES: &[PortableTableSchema] = &[
    table(1, "SourceModule", &SOURCE_MODULE_ROW, ANY),
    table(2, "SourceDocument", &SOURCE_DOCUMENT_ROW, ANY),
    table(3, "SourceLocation", &SOURCE_LOCATION_ROW, ANY),
];

const STABLE_ENTITY_SOURCE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "entityKind", PortableFieldType::U16, R),
    field(2, "stableId", PortableFieldType::StableId128, R),
    field(3, "typedOrdinal", PortableFieldType::U32, R),
    field(4, "primaryLocation", PortableFieldType::U32, R),
    field(
        5,
        "contributingLocations",
        PortableFieldType::OrdinalVectorU32,
        R,
    ),
];
const STABLE_ENTITY_SOURCE_ROW: PortableRowSchema = PortableRowSchema {
    fields: STABLE_ENTITY_SOURCE_FIELDS,
    shape: PortableRowShape::Uniform,
};
const LFSM_SECTION_3_TABLES: &[PortableTableSchema] = &[table(
    1,
    "StableEntitySource",
    &STABLE_ENTITY_SOURCE_ROW,
    ANY,
)];

const OWNER_LOCAL_SOURCE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "ownerEntityKind", PortableFieldType::U16, R),
    field(2, "ownerStableId", PortableFieldType::StableId128, R),
    field(3, "sourceRelationRole", PortableFieldType::U8, R),
    field(4, "localIndex", PortableFieldType::U32, R),
    field(5, "primaryLocation", PortableFieldType::U32, R),
    field(
        6,
        "contributingLocations",
        PortableFieldType::OrdinalVectorU32,
        R,
    ),
];
const OWNER_LOCAL_SOURCE_ROW: PortableRowSchema = PortableRowSchema {
    fields: OWNER_LOCAL_SOURCE_FIELDS,
    shape: PortableRowShape::Uniform,
};
const SPATIAL_SOURCE_RANGE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "ownerEntityKind", PortableFieldType::U16, R),
    field(2, "ownerStableId", PortableFieldType::StableId128, R),
    field(3, "sourceRelationRole", PortableFieldType::U8, R),
    field(4, "localIndex", PortableFieldType::U32, R),
    field(5, "pointStart", PortableFieldType::U32, R),
    field(6, "pointEndExclusive", PortableFieldType::U32, R),
    field(7, "sourceSegmentOrdinal", PortableFieldType::U32, R),
    field(8, "sourceLocation", PortableFieldType::U32, R),
];
const SPATIAL_SOURCE_RANGE_ROW: PortableRowSchema = PortableRowSchema {
    fields: SPATIAL_SOURCE_RANGE_FIELDS,
    shape: PortableRowShape::Uniform,
};
const LFSM_SECTION_4_TABLES: &[PortableTableSchema] = &[
    table(1, "OwnerLocalSource", &OWNER_LOCAL_SOURCE_ROW, ANY),
    table(
        2,
        "SpatialGeometrySourceRange",
        &SPATIAL_SOURCE_RANGE_ROW,
        ANY,
    ),
];

const DERIVED_RELATION_SOURCE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "ownerEntityKind", PortableFieldType::U16, R),
    field(2, "ownerStableId", PortableFieldType::StableId128, R),
    field(3, "sourceRelationRole", PortableFieldType::U8, R),
    field(4, "localIndex", PortableFieldType::U32, R),
    field(5, "derivationPassVersion", PortableFieldType::U16, R),
    field(6, "constraintVersion", PortableFieldType::U16, R),
    field(7, "sourceLocations", PortableFieldType::OrdinalVectorU32, R),
];
const DERIVED_RELATION_SOURCE_ROW: PortableRowSchema = PortableRowSchema {
    fields: DERIVED_RELATION_SOURCE_FIELDS,
    shape: PortableRowShape::Uniform,
};
const LFSM_SECTION_5_TABLES: &[PortableTableSchema] = &[table(
    1,
    "DerivedRelationSource",
    &DERIVED_RELATION_SOURCE_ROW,
    ANY,
)];

const LFSM_SECTIONS: &[PortableSectionSchema] = &[
    PortableSectionSchema {
        kind: 1,
        name: "SourceMapBindings",
        tables: LFSM_SECTION_1_TABLES,
    },
    PortableSectionSchema {
        kind: 2,
        name: "SourceModules",
        tables: LFSM_SECTION_2_TABLES,
    },
    PortableSectionSchema {
        kind: 3,
        name: "StableEntitySources",
        tables: LFSM_SECTION_3_TABLES,
    },
    PortableSectionSchema {
        kind: 4,
        name: "OwnerLocalSources",
        tables: LFSM_SECTION_4_TABLES,
    },
    PortableSectionSchema {
        kind: 5,
        name: "DerivedRelationSources",
        tables: LFSM_SECTION_5_TABLES,
    },
];

// LFSD.

const SEMANTIC_DIFF_BINDINGS_FIELDS: &[PortableFieldSchema] = &[
    field(1, "baseBindingKind", PortableFieldType::U8, R),
    field(
        2,
        "baseNetworkRevisionDerivationVersion",
        PortableFieldType::U16,
        R,
    ),
    field(3, "baseNetworkRevision", PortableFieldType::Sha256, R),
    field(
        4,
        "baseCanonicalArtifactDigest",
        PortableFieldType::Sha256,
        R,
    ),
    field(
        5,
        "baseCanonicalArtifactByteLength",
        PortableFieldType::U64,
        R,
    ),
    field(
        6,
        "targetNetworkRevisionDerivationVersion",
        PortableFieldType::U16,
        R,
    ),
    field(7, "targetNetworkRevision", PortableFieldType::Sha256, R),
    field(
        8,
        "targetCanonicalArtifactDigest",
        PortableFieldType::Sha256,
        R,
    ),
    field(
        9,
        "targetCanonicalArtifactByteLength",
        PortableFieldType::U64,
        R,
    ),
];
const SEMANTIC_DIFF_BINDINGS_ROW: PortableRowSchema = PortableRowSchema {
    fields: SEMANTIC_DIFF_BINDINGS_FIELDS,
    shape: PortableRowShape::Uniform,
};
const LFSD_SECTION_1_TABLES: &[PortableTableSchema] = &[table(
    1,
    "SemanticDiffBindings",
    &SEMANTIC_DIFF_BINDINGS_ROW,
    ONE,
)];

const ENTITY_CHANGE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "changeKind", PortableFieldType::U8, V),
    field(2, "entityKind", PortableFieldType::U16, V),
    field(3, "ownerStableId", PortableFieldType::StableId128, V),
    field(4, "subjectStableId", PortableFieldType::StableId128, V),
    field(5, "sourceRelationRole", PortableFieldType::U8, V),
    field(6, "fieldTag", PortableFieldType::U16, V),
    field(7, "beforeLocalIndex", PortableFieldType::U32, V),
    field(8, "afterLocalIndex", PortableFieldType::U32, V),
    field(9, "beforeValue", PortableFieldType::Bytes, V),
    field(10, "afterValue", PortableFieldType::Bytes, V),
];
const ENTITY_CHANGE_VARIANTS: &[PortableRowVariant] = &[
    PortableRowVariant {
        discriminant: 0,
        required_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(4)
            | portable_field_mask(10),
        allowed_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(4)
            | portable_field_mask(10),
        at_least_one_field: 0,
    },
    PortableRowVariant {
        discriminant: 1,
        required_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(4)
            | portable_field_mask(9),
        allowed_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(4)
            | portable_field_mask(9),
        at_least_one_field: 0,
    },
    PortableRowVariant {
        discriminant: 2,
        required_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(4)
            | portable_field_mask(6),
        allowed_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(4)
            | portable_field_mask(6)
            | portable_field_mask(9)
            | portable_field_mask(10),
        at_least_one_field: portable_field_mask(9) | portable_field_mask(10),
    },
];
const ENTITY_CHANGE_ROW: PortableRowSchema = PortableRowSchema {
    fields: ENTITY_CHANGE_FIELDS,
    shape: PortableRowShape::DiscriminatedU8 {
        tag: 1,
        variants: ENTITY_CHANGE_VARIANTS,
    },
};

const RELATION_CHANGE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "changeKind", PortableFieldType::U8, V),
    field(2, "entityKind", PortableFieldType::U16, V),
    field(3, "ownerStableId", PortableFieldType::StableId128, V),
    field(4, "subjectStableId", PortableFieldType::StableId128, V),
    field(5, "sourceRelationRole", PortableFieldType::U8, V),
    field(6, "fieldTag", PortableFieldType::U16, V),
    field(7, "beforeLocalIndex", PortableFieldType::U32, V),
    field(8, "afterLocalIndex", PortableFieldType::U32, V),
    field(9, "beforeTarget", PortableFieldType::StableId128, V),
    field(10, "afterTarget", PortableFieldType::StableId128, V),
];
const RELATION_CHANGE_VARIANTS: &[PortableRowVariant] = &[
    PortableRowVariant {
        discriminant: 0,
        required_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(3)
            | portable_field_mask(4)
            | portable_field_mask(5)
            | portable_field_mask(8),
        allowed_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(3)
            | portable_field_mask(4)
            | portable_field_mask(5)
            | portable_field_mask(8),
        at_least_one_field: 0,
    },
    PortableRowVariant {
        discriminant: 1,
        required_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(3)
            | portable_field_mask(4)
            | portable_field_mask(5)
            | portable_field_mask(7),
        allowed_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(3)
            | portable_field_mask(4)
            | portable_field_mask(5)
            | portable_field_mask(7),
        at_least_one_field: 0,
    },
    PortableRowVariant {
        discriminant: 2,
        required_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(3)
            | portable_field_mask(4)
            | portable_field_mask(5)
            | portable_field_mask(7)
            | portable_field_mask(8),
        allowed_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(3)
            | portable_field_mask(4)
            | portable_field_mask(5)
            | portable_field_mask(7)
            | portable_field_mask(8),
        at_least_one_field: 0,
    },
    PortableRowVariant {
        discriminant: 3,
        required_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(3)
            | portable_field_mask(5)
            | portable_field_mask(7)
            | portable_field_mask(8)
            | portable_field_mask(9)
            | portable_field_mask(10),
        allowed_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(3)
            | portable_field_mask(5)
            | portable_field_mask(7)
            | portable_field_mask(8)
            | portable_field_mask(9)
            | portable_field_mask(10),
        at_least_one_field: 0,
    },
];
const RELATION_CHANGE_ROW: PortableRowSchema = PortableRowSchema {
    fields: RELATION_CHANGE_FIELDS,
    shape: PortableRowShape::DiscriminatedU8 {
        tag: 1,
        variants: RELATION_CHANGE_VARIANTS,
    },
};

const GEOMETRY_CHANGE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "changeKind", PortableFieldType::U8, V),
    field(2, "entityKind", PortableFieldType::U16, V),
    field(3, "ownerStableId", PortableFieldType::StableId128, V),
    field(4, "subjectStableId", PortableFieldType::StableId128, V),
    field(5, "sourceRelationRole", PortableFieldType::U8, V),
    field(6, "fieldTag", PortableFieldType::U16, V),
    field(7, "beforeLocalIndex", PortableFieldType::U32, V),
    field(8, "afterLocalIndex", PortableFieldType::U32, V),
    field(9, "beforeCanonicalValue", PortableFieldType::Bytes, V),
    field(10, "afterCanonicalValue", PortableFieldType::Bytes, V),
];
const GEOMETRY_CHANGE_VARIANTS: &[PortableRowVariant] = &[
    PortableRowVariant {
        discriminant: 0,
        required_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(4)
            | portable_field_mask(10),
        allowed_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(4)
            | portable_field_mask(10),
        at_least_one_field: 0,
    },
    PortableRowVariant {
        discriminant: 1,
        required_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(4)
            | portable_field_mask(9),
        allowed_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(4)
            | portable_field_mask(9),
        at_least_one_field: 0,
    },
    PortableRowVariant {
        discriminant: 2,
        required_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(4)
            | portable_field_mask(9)
            | portable_field_mask(10),
        allowed_fields: portable_field_mask(1)
            | portable_field_mask(2)
            | portable_field_mask(4)
            | portable_field_mask(9)
            | portable_field_mask(10),
        at_least_one_field: 0,
    },
];
const GEOMETRY_CHANGE_ROW: PortableRowSchema = PortableRowSchema {
    fields: GEOMETRY_CHANGE_FIELDS,
    shape: PortableRowShape::DiscriminatedU8 {
        tag: 1,
        variants: GEOMETRY_CHANGE_VARIANTS,
    },
};

const STATIC_RULE_CHANGE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "changeKind", PortableFieldType::U8, V),
    field(2, "entityKind", PortableFieldType::U16, V),
    field(3, "ownerStableId", PortableFieldType::StableId128, V),
    field(4, "subjectStableId", PortableFieldType::StableId128, V),
    field(5, "sourceRelationRole", PortableFieldType::U8, V),
    field(6, "fieldTag", PortableFieldType::U16, V),
    field(7, "beforeLocalIndex", PortableFieldType::U32, V),
    field(8, "afterLocalIndex", PortableFieldType::U32, V),
    field(9, "beforeCanonicalValue", PortableFieldType::Bytes, V),
    field(10, "afterCanonicalValue", PortableFieldType::Bytes, V),
];
const STATIC_RULE_CHANGE_VARIANTS: &[PortableRowVariant] = &[PortableRowVariant {
    discriminant: 0,
    required_fields: portable_field_mask(1)
        | portable_field_mask(2)
        | portable_field_mask(4)
        | portable_field_mask(6),
    allowed_fields: portable_field_mask(1)
        | portable_field_mask(2)
        | portable_field_mask(4)
        | portable_field_mask(6)
        | portable_field_mask(9)
        | portable_field_mask(10),
    at_least_one_field: portable_field_mask(9) | portable_field_mask(10),
}];
const STATIC_RULE_CHANGE_ROW: PortableRowSchema = PortableRowSchema {
    fields: STATIC_RULE_CHANGE_FIELDS,
    shape: PortableRowShape::DiscriminatedU8 {
        tag: 1,
        variants: STATIC_RULE_CHANGE_VARIANTS,
    },
};

const SPATIAL_CONFIGURATION_CHANGE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "changeKind", PortableFieldType::U8, V),
    field(2, "beforeSpatialPresence", PortableFieldType::Bytes, V),
    field(3, "afterSpatialPresence", PortableFieldType::Bytes, V),
];
const SPATIAL_CONFIGURATION_CHANGE_VARIANTS: &[PortableRowVariant] = &[
    PortableRowVariant {
        discriminant: 0,
        required_fields: portable_field_mask(1) | portable_field_mask(3),
        allowed_fields: portable_field_mask(1) | portable_field_mask(3),
        at_least_one_field: 0,
    },
    PortableRowVariant {
        discriminant: 1,
        required_fields: portable_field_mask(1) | portable_field_mask(2) | portable_field_mask(3),
        allowed_fields: portable_field_mask(1) | portable_field_mask(2) | portable_field_mask(3),
        at_least_one_field: 0,
    },
];
const SPATIAL_CONFIGURATION_CHANGE_ROW: PortableRowSchema = PortableRowSchema {
    fields: SPATIAL_CONFIGURATION_CHANGE_FIELDS,
    shape: PortableRowShape::DiscriminatedU8 {
        tag: 1,
        variants: SPATIAL_CONFIGURATION_CHANGE_VARIANTS,
    },
};

const LFSD_SECTION_2_TABLES: &[PortableTableSchema] =
    &[table(1, "EntityChange", &ENTITY_CHANGE_ROW, ANY)];
const LFSD_SECTION_3_TABLES: &[PortableTableSchema] =
    &[table(1, "RelationChange", &RELATION_CHANGE_ROW, ANY)];
const LFSD_SECTION_4_TABLES: &[PortableTableSchema] =
    &[table(1, "GeometryChange", &GEOMETRY_CHANGE_ROW, ANY)];
const LFSD_SECTION_5_TABLES: &[PortableTableSchema] =
    &[table(1, "StaticRuleChange", &STATIC_RULE_CHANGE_ROW, ANY)];
const LFSD_SECTION_6_TABLES: &[PortableTableSchema] = &[table(
    1,
    "SpatialConfigurationChange",
    &SPATIAL_CONFIGURATION_CHANGE_ROW,
    AT_MOST_ONE,
)];
const LFSD_SECTIONS: &[PortableSectionSchema] = &[
    PortableSectionSchema {
        kind: 1,
        name: "SemanticDiffBindings",
        tables: LFSD_SECTION_1_TABLES,
    },
    PortableSectionSchema {
        kind: 2,
        name: "EntityChanges",
        tables: LFSD_SECTION_2_TABLES,
    },
    PortableSectionSchema {
        kind: 3,
        name: "RelationChanges",
        tables: LFSD_SECTION_3_TABLES,
    },
    PortableSectionSchema {
        kind: 4,
        name: "GeometryChanges",
        tables: LFSD_SECTION_4_TABLES,
    },
    PortableSectionSchema {
        kind: 5,
        name: "StaticRuleChanges",
        tables: LFSD_SECTION_5_TABLES,
    },
    PortableSectionSchema {
        kind: 6,
        name: "SpatialConfigurationChanges",
        tables: LFSD_SECTION_6_TABLES,
    },
];

// LFCP.

const CANONICAL_ARTIFACT_BINDING_FIELDS: &[PortableFieldSchema] = &[
    field(
        1,
        "canonicalArtifactFormatVersion",
        PortableFieldType::U16,
        R,
    ),
    field(
        2,
        "networkRevisionDerivationVersion",
        PortableFieldType::U16,
        R,
    ),
    field(3, "networkRevision", PortableFieldType::Sha256, R),
    field(4, "canonicalArtifactDigest", PortableFieldType::Sha256, R),
    field(5, "canonicalArtifactByteLength", PortableFieldType::U64, R),
];
const CANONICAL_ARTIFACT_BINDING_ROW: PortableRowSchema = PortableRowSchema {
    fields: CANONICAL_ARTIFACT_BINDING_FIELDS,
    shape: PortableRowShape::Uniform,
};
const SOURCE_MAP_BINDING_FIELDS: &[PortableFieldSchema] = &[
    field(1, "sourceMapFormatVersion", PortableFieldType::U16, R),
    field(2, "sourceMapDigest", PortableFieldType::Sha256, R),
    field(3, "sourceMapByteLength", PortableFieldType::U64, R),
    field(4, "compilerBuildId", PortableFieldType::Utf8, R),
    field(
        5,
        "sourceCollectionDigestVersion",
        PortableFieldType::U16,
        R,
    ),
    field(6, "sourceCollectionDigest", PortableFieldType::Sha256, R),
];
const SOURCE_MAP_BINDING_ROW: PortableRowSchema = PortableRowSchema {
    fields: SOURCE_MAP_BINDING_FIELDS,
    shape: PortableRowShape::Uniform,
};
const PUBLICATION_PROVENANCE_FIELDS: &[PortableFieldSchema] = &[
    field(1, "publisherKind", PortableFieldType::U8, R),
    field(2, "publisherBuildId", PortableFieldType::Utf8, R),
    field(3, "artifactObjectKey", PortableFieldType::Utf8, R),
    field(4, "sourceMapObjectKey", PortableFieldType::Utf8, R),
    field(5, "controlledBuildProvenance", PortableFieldType::Utf8, O),
    field(6, "controlledTimestamp", PortableFieldType::Utf8, O),
];
const PUBLICATION_PROVENANCE_ROW: PortableRowSchema = PortableRowSchema {
    fields: PUBLICATION_PROVENANCE_FIELDS,
    shape: PortableRowShape::Uniform,
};
const LFCP_SECTION_1_TABLES: &[PortableTableSchema] = &[table(
    1,
    "CanonicalArtifactBinding",
    &CANONICAL_ARTIFACT_BINDING_ROW,
    ONE,
)];
const LFCP_SECTION_2_TABLES: &[PortableTableSchema] =
    &[table(1, "SourceMapBinding", &SOURCE_MAP_BINDING_ROW, ONE)];
const LFCP_SECTION_3_TABLES: &[PortableTableSchema] = &[table(
    1,
    "PublicationProvenance",
    &PUBLICATION_PROVENANCE_ROW,
    ONE,
)];
const LFCP_SECTIONS: &[PortableSectionSchema] = &[
    PortableSectionSchema {
        kind: 1,
        name: "CanonicalArtifactBinding",
        tables: LFCP_SECTION_1_TABLES,
    },
    PortableSectionSchema {
        kind: 2,
        name: "SourceMapBinding",
        tables: LFCP_SECTION_2_TABLES,
    },
    PortableSectionSchema {
        kind: 3,
        name: "PublicationProvenance",
        tables: LFCP_SECTION_3_TABLES,
    },
];

const LFCA_SCHEMA: PortableObjectSchema = PortableObjectSchema {
    kind: PortableObjectKind::CanonicalArtifact,
    sections: LFCA_SECTIONS,
};
const LFSM_SCHEMA: PortableObjectSchema = PortableObjectSchema {
    kind: PortableObjectKind::SourceMap,
    sections: LFSM_SECTIONS,
};
const LFSD_SCHEMA: PortableObjectSchema = PortableObjectSchema {
    kind: PortableObjectKind::SemanticDiff,
    sections: LFSD_SECTIONS,
};
const LFCP_SCHEMA: PortableObjectSchema = PortableObjectSchema {
    kind: PortableObjectKind::CanonicalPublicationDescriptor,
    sections: LFCP_SECTIONS,
};

/// 返回指定对象种类的当前静态登记。
#[must_use]
pub const fn portable_object_schema(kind: PortableObjectKind) -> &'static PortableObjectSchema {
    match kind {
        PortableObjectKind::CanonicalArtifact => &LFCA_SCHEMA,
        PortableObjectKind::SourceMap => &LFSM_SCHEMA,
        PortableObjectKind::SemanticDiff => &LFSD_SCHEMA,
        PortableObjectKind::CanonicalPublicationDescriptor => &LFCP_SCHEMA,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    #[test]
    fn contract_versions_match_appendix_a1_literal_registry() {
        let fields = portable_object_schema(PortableObjectKind::CanonicalArtifact).sections[0]
            .tables[0]
            .row
            .fields;
        let expected = [
            (1, "canonicalFormatVersion"),
            (2, "identityEncodingVersion"),
            (3, "identityRegistryRevision"),
            (4, "networkRevisionDerivationVersion"),
            (5, "constraintContractVersion"),
            (6, "staticExecutionContractVersion"),
        ];

        assert_eq!(fields.len(), expected.len());
        for (field, (tag, name)) in fields.iter().zip(expected) {
            assert_eq!(field.tag, tag);
            assert_eq!(field.name, name);
            assert_eq!(field.field_type, PortableFieldType::U16);
            assert_eq!(field.presence, PortableFieldPresence::Required);
            assert!(field.nested_row.is_none());
        }
    }

    #[test]
    fn appendix_registry_matches_reviewed_literal_fingerprint() {
        // 这只是对已依据附录 A 人工复核过的 Rust 登记做防漂移固定，不是独立格式 oracle。
        // 更新该值必须先逐项审阅附录；不得从测试失败输出自动追认新的 production registry。
        // LFCA 5 的新增行另按路权策略实施合同 §4.1 逐 tag/type/presence 复核。
        assert_eq!(appendix_registry_fingerprint(), 0xe39a_cf2d_8d23_4a02);
    }

    #[test]
    fn appendix_registry_has_exact_section_and_table_counts() {
        for kind in PortableObjectKind::ALL {
            for object in [portable_object_schema(kind)] {
                assert_eq!(object.kind, kind);
                assert_eq!(object.sections.len(), kind.section_count() as usize);
                assert_eq!(
                    object
                        .sections
                        .iter()
                        .map(|section| section.tables.len())
                        .sum::<usize>(),
                    kind.table_count() as usize
                );
                for (section_index, section) in object.sections.iter().enumerate() {
                    assert_eq!(section.kind as usize, section_index + 1);
                    let mut previous_kind = 0_u16;
                    for table in section.tables {
                        assert!(table.kind > previous_kind);
                        previous_kind = table.kind;
                    }
                }
            }
        }
    }

    #[test]
    fn lfca_traffic_hot_columns_are_integer_millimetres() {
        let schema = portable_object_schema(PortableObjectKind::CanonicalArtifact);

        let lane = schema.sections[2].tables[3].row.fields;
        assert_eq!(lane[2].name, "lengthMillimetres");
        assert_eq!(lane[2].field_type, PortableFieldType::U32);
        assert_eq!(lane[3].name, "speedLimitMillimetresPerSecond");
        assert_eq!(lane[3].field_type, PortableFieldType::U32);

        let parking = schema.sections[2].tables[14].row.fields;
        assert_eq!(parking[4].name, "entryProgressMillimetres");
        assert_eq!(parking[4].field_type, PortableFieldType::U32);
        assert_eq!(parking[6].name, "exitProgressMillimetres");
        assert_eq!(parking[6].field_type, PortableFieldType::U32);
        assert_eq!(parking[7].name, "lateralOffsetMillimetres");
        assert_eq!(parking[7].field_type, PortableFieldType::I32);
        assert_eq!(parking[8].name, "headingOffsetRadians");
        assert_eq!(parking[8].field_type, PortableFieldType::F32);
        assert_eq!(parking[9].name, "lengthMillimetres");
        assert_eq!(parking[9].field_type, PortableFieldType::U32);
        assert_eq!(parking[10].name, "widthMillimetres");
        assert_eq!(parking[10].field_type, PortableFieldType::U32);

        let profile = schema.sections[2].tables[19].row.fields;
        assert_eq!(profile[3].name, "lengthMillimetres");
        assert_eq!(profile[3].field_type, PortableFieldType::U32);
        assert_eq!(profile[4].name, "desiredSpeedMillimetresPerSecond");
        assert_eq!(profile[4].field_type, PortableFieldType::U32);
        assert_eq!(profile[5].name, "minGapMillimetres");
        assert_eq!(profile[5].field_type, PortableFieldType::U32);
        assert_eq!(profile[6].field_type, PortableFieldType::F32);
        assert_eq!(profile[7].field_type, PortableFieldType::F32);
        assert_eq!(profile[8].field_type, PortableFieldType::F32);
        assert_eq!(profile[9].field_type, PortableFieldType::F32);
    }

    #[test]
    fn field_and_variant_registries_are_closed_and_ordered() {
        for kind in PortableObjectKind::ALL {
            for schema in [portable_object_schema(kind)] {
                for section in schema.sections {
                    for table in section.tables {
                        check_row(table.row);
                    }
                }
            }
        }
    }

    fn check_row(row: &PortableRowSchema) {
        let mut previous = 0;
        let mut universe = 0_u32;
        for field in row.fields {
            assert!(field.tag > previous);
            assert!(field.tag <= 31);
            previous = field.tag;
            universe |= portable_field_mask(field.tag);
            assert_eq!(
                field.field_type == PortableFieldType::RecordVector,
                field.nested_row.is_some()
            );
            if let Some(nested) = field.nested_row {
                check_row(nested);
            }
        }
        match row.shape {
            PortableRowShape::Uniform => {
                assert!(
                    row.fields
                        .iter()
                        .all(|field| field.presence != PortableFieldPresence::ByRowVariant)
                );
            }
            PortableRowShape::DiscriminatedU8 { tag, variants } => {
                let field = row.fields.iter().find(|field| field.tag == tag).unwrap();
                assert_eq!(field.field_type, PortableFieldType::U8);
                assert!(
                    row.fields
                        .iter()
                        .all(|field| field.presence == PortableFieldPresence::ByRowVariant)
                );
                let mut previous_discriminant = None;
                for variant in variants {
                    assert_eq!(variant.allowed_fields & !universe, 0);
                    assert_eq!(variant.required_fields & !variant.allowed_fields, 0);
                    assert_eq!(variant.at_least_one_field & !variant.allowed_fields, 0);
                    assert_ne!(variant.required_fields & portable_field_mask(tag), 0);
                    if let Some(previous) = previous_discriminant {
                        assert!(previous < variant.discriminant);
                    }
                    previous_discriminant = Some(variant.discriminant);
                }
            }
        }
    }

    fn appendix_registry_fingerprint() -> u64 {
        let mut hash = FNV_OFFSET_BASIS;
        for kind in PortableObjectKind::ALL {
            for byte in std::format!("{:?}", portable_object_schema(kind)).bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME);
            }
        }
        hash
    }
}
