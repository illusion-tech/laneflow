#![no_std]
#![doc = include_str!("../README.md")]

#[cfg(test)]
extern crate std;

mod portable;
mod portable_registry;
mod registry;
mod typed;
mod values;

pub use portable::{
    CANONICAL_ARTIFACT_FORMAT_VERSION, CANONICAL_PUBLICATION_DESCRIPTOR_VERSION, ExactByteLength,
    FORMAT_HARD_MAX_CANDIDATE_STAGING_BYTES, FORMAT_HARD_MAX_FIELDS_PER_ROW,
    FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES, FORMAT_HARD_MAX_OBJECT_BYTES,
    FORMAT_HARD_MAX_RECORD_VECTOR_DEPTH, FORMAT_HARD_MAX_ROWS_PER_TABLE,
    FORMAT_HARD_MAX_SECTION_OR_TABLE_BYTES, FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS,
    FORMAT_HARD_MAX_TOTAL_UTF8_BYTES, FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES,
    FORMAT_HARD_MAX_UTF8_FIELD_BYTES, FORMAT_HARD_MAX_VECTOR_ITEMS,
    NETWORK_REVISION_DERIVATION_VERSION, NETWORK_REVISION_DOMAIN_PREFIX, NetworkRevisionId,
    OBJECT_PREAMBLE_V1_BYTE_LENGTH, PortableFieldType, PortableObjectKind,
    SECTION_DIRECTORY_ENTRY_V1_BYTE_LENGTH, SECTION_FORMAT_VERSION_V1,
    SEMANTIC_DIFF_FORMAT_VERSION, SOURCE_MAP_FORMAT_VERSION, Sha256Digest,
};
pub use portable_registry::{
    PortableFieldPresence, PortableFieldSchema, PortableObjectSchema, PortableRowCardinality,
    PortableRowSchema, PortableRowShape, PortableRowVariant, PortableSectionSchema,
    PortableTableSchema, portable_field_mask, portable_object_schema,
};

pub use registry::{
    EntityCategory, EntityKind, FieldEncoding, FieldTag, IDENTITY_ENCODING_VERSION, IDENTITY_MAGIC,
    IDENTITY_REGISTRY_REVISION, STABLE_ID_DOMAIN_PREFIX,
};
pub use typed::{
    AccessRuleId, AccessRuleKind, AccessRuleOrdinal, AuthoringLaneId, AuthoringLaneKind,
    AuthoringLaneOrdinal, CanonicalFrameId, CanonicalFrameKind, CanonicalFrameOrdinal,
    EntityKindMarker, FacilityBandId, FacilityBandKind, FacilityBandOrdinal, JunctionId,
    JunctionKind, JunctionOrdinal, LaneEdgeId, LaneEdgeKind, LaneEdgeOrdinal, LaneGroupId,
    LaneGroupKind, LaneGroupOrdinal, ManeuverGateId, ManeuverGateKind, ManeuverGateOrdinal,
    ManeuverPathId, ManeuverPathKind, ManeuverPathOrdinal, MovementId, MovementKind,
    MovementOrdinal, Ordinal, OrdinalKind, ParkingAreaId, ParkingAreaKind, ParkingAreaOrdinal,
    ParkingSpaceId, ParkingSpaceKind, ParkingSpaceOrdinal, ParticipantClassId,
    ParticipantClassKind, ParticipantClassOrdinal, RoadCorridorId, RoadCorridorKind,
    RoadCorridorOrdinal, RoadSectionId, RoadSectionKind, RoadSectionOrdinal, SignalControllerId,
    SignalControllerKind, SignalControllerOrdinal, SignalGroupId, SignalGroupKind,
    SignalGroupOrdinal, SignalPhaseId, SignalPhaseKind, SignalPhaseOrdinal, StableId, StableId128,
    StableIdTextError, StaticRouteId, StaticRouteKind, StaticRouteOrdinal, StopLineId,
    StopLineKind, StopLineOrdinal, VehicleProfileId, VehicleProfileKind, VehicleProfileOrdinal,
    WaitingZoneId, WaitingZoneKind, WaitingZoneOrdinal,
};
pub use values::{
    AccessEffect, CANONICAL_POINT_COMPONENT_MAX_METERS, CANONICAL_POINT_COMPONENT_MIN_METERS,
    MIN_PARKING_EXTENT_EXCLUSIVE_METERS, MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS,
    MIN_VEHICLE_LENGTH_EXCLUSIVE_METERS, PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS,
    PARKING_HEADING_OFFSET_MAXIMUM_RADIANS, PARKING_HEADING_OFFSET_MINIMUM_RADIANS,
    SPATIAL_CORE_LENGTH_QUANTIZATION_ALLOWANCE_METERS, SPATIAL_JOIN_POSITION_TOLERANCE_METERS,
    SPATIAL_LENGTH_ABS_TOLERANCE_METERS, SPATIAL_LENGTH_REL_TOLERANCE,
    SPATIAL_MIN_PROJECTED_UP_LENGTH, SPATIAL_MIN_SEGMENT_LENGTH_METERS, SignalAspect,
};
