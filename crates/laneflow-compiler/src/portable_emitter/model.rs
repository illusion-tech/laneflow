use super::*;

/// LFSM 来源位置值的拥有型投影：文本来源（模块/文档 ordinal 与起止行列）或道路编辑
/// 来源（主题种类、命名空间、owner-local 键、属性路径与画布选择）。
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum LocationValue {
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
        module_namespace: Option<Box<str>>,
        entity_kind: Option<u16>,
        owner_local_keys: [Option<Box<str>>; 3],
        local_key: Option<Box<str>>,
        owner_kind: Option<u8>,
        relation_kind: Option<u8>,
        occurrence_kind: Option<u8>,
        occurrence_ordinal: Option<u32>,
        property_steps: Option<Box<[(u8, u16, u16)]>>,
        canvas_selection: Option<Box<str>>,
    },
}

/// 按稳定身份定位实体的来源投影（LFSM StableSource 行）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StableSourceProjection {
    pub(super) entity_kind: EntityKind,
    pub(super) stable_id: [u8; 16],
    pub(super) typed_ordinal: u32,
    pub(super) primary: LocationValue,
    pub(super) contributing: Vec<LocationValue>,
}

/// 按所有者局部位置定位的来源投影（LFSM OwnerLocalSource 行）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct OwnerLocalProjection {
    pub(super) owner_entity_kind: EntityKind,
    pub(super) owner_stable_id: [u8; 16],
    pub(super) role: u8,
    pub(super) local_index: u32,
    pub(super) primary: LocationValue,
    pub(super) contributing: Vec<LocationValue>,
}

/// 空间几何点区间的来源投影：所有者实体、角色、局部下标、点半开区间与来源位置。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SpatialRangeProjection {
    pub(super) owner_entity_kind: EntityKind,
    pub(super) owner_stable_id: [u8; 16],
    pub(super) role: u8,
    pub(super) local_index: u32,
    pub(super) point_start: u32,
    pub(super) point_end_exclusive: u32,
    pub(super) source_segment_ordinal: u32,
    pub(super) source: LocationValue,
}

/// 道路编辑声明地址投影：（模块命名空间, 可选实体种类, 三级 owner-local 键, local key）。
pub(super) type RoadEditingAddressProjection =
    (Box<str>, Option<u16>, [Option<Box<str>>; 3], Box<str>);
/// 以（实体种类, 稳定标识）为键的几何编码值集合。
pub(super) type GeometryValues = BTreeMap<(EntityKind, [u8; 16]), Box<[u8]>>;
/// 以（所有者实体种类, 所有者稳定标识, 关系角色）分组的关系元组集合。
pub(super) type RelationGroups = BTreeMap<(EntityKind, [u8; 16], u8), Vec<RelationTuple>>;

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
