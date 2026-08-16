use super::*;

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StableSourceProjection {
    pub(super) entity_kind: EntityKind,
    pub(super) stable_id: [u8; 16],
    pub(super) typed_ordinal: u32,
    pub(super) primary: LocationValue,
    pub(super) contributing: Vec<LocationValue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct OwnerLocalProjection {
    pub(super) owner_entity_kind: EntityKind,
    pub(super) owner_stable_id: [u8; 16],
    pub(super) role: u8,
    pub(super) local_index: u32,
    pub(super) primary: LocationValue,
    pub(super) contributing: Vec<LocationValue>,
}

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

pub(super) type RoadEditingAddressProjection =
    (Box<str>, Option<u16>, [Option<Box<str>>; 3], Box<str>);
pub(super) type GeometryValues = BTreeMap<(EntityKind, [u8; 16]), Box<[u8]>>;
pub(super) type RelationGroups = BTreeMap<(EntityKind, [u8; 16], u8), Vec<RelationTuple>>;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct RelationTuple {
    pub(super) owner_entity_kind: EntityKind,
    pub(super) owner_stable_id: [u8; 16],
    pub(super) role: u8,
    pub(super) local_index: u32,
    pub(super) subject_entity_kind: EntityKind,
    pub(super) subject_stable_id: [u8; 16],
}
