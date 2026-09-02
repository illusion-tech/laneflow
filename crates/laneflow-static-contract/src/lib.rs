#![no_std]
#![doc = include_str!("../README.md")]

#[cfg(test)]
extern crate std;

mod policy;
mod portable;
mod portable_registry;
mod registry;
mod typed;
mod values;

pub use portable::{
    CANONICAL_ARTIFACT_FORMAT_VERSION, CANONICAL_PUBLICATION_DESCRIPTOR_VERSION,
    CHUNKED_SECTION_FORMAT_VERSION, CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH,
    CONSTRAINT_CONTRACT_VERSION, ExactByteLength, FORMAT_HARD_MAX_FIELDS_PER_ROW,
    FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES, FORMAT_HARD_MAX_RECORD_VECTOR_DEPTH,
    FORMAT_HARD_MAX_ROWS_PER_CHUNK, FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS_PER_CHUNK,
    FORMAT_HARD_MAX_STAGED_CHUNK_BYTES, FORMAT_HARD_MAX_TABLE_CHUNK_BYTES,
    FORMAT_HARD_MAX_TOTAL_UTF8_BYTES, FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES,
    FORMAT_HARD_MAX_UTF8_FIELD_BYTES, FORMAT_HARD_MAX_VECTOR_ITEMS,
    NETWORK_REVISION_DERIVATION_VERSION, NETWORK_REVISION_DOMAIN_PREFIX, NetworkRevisionId,
    OBJECT_PREAMBLE_BYTE_LENGTH, PortableFieldType, PortableObjectKind,
    SECTION_DIRECTORY_ENTRY_BYTE_LENGTH, SEMANTIC_DIFF_FORMAT_VERSION,
    SINGLETON_SECTION_FORMAT_VERSION, SOURCE_MAP_FORMAT_VERSION, STATIC_EXECUTION_CONTRACT_VERSION,
    Sha256Digest, TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH,
};
pub use portable_registry::{
    PortableFieldPresence, PortableFieldSchema, PortableObjectSchema, PortableRowCardinality,
    PortableRowSchema, PortableRowShape, PortableRowVariant, PortableSectionSchema,
    PortableTableSchema, policy_local_value_schema, portable_field_mask, portable_object_schema,
};

pub use policy::{
    GateInterpretation, GateProhibition, ManeuverDirection, PolicyLocalChangeKind,
    PolicyLocalMemberKind,
};
pub use registry::{
    EntityCategory, EntityKind, FieldEncoding, FieldTag, IDENTITY_ENCODING_VERSION, IDENTITY_MAGIC,
    IDENTITY_REGISTRY_REVISION, STABLE_ID_DOMAIN_PREFIX,
};
pub use typed::{
    AccessRuleId, AccessRuleKind, AccessRuleOrdinal, AuthoringLaneId, AuthoringLaneKind,
    AuthoringLaneOrdinal, CanonicalFrameId, CanonicalFrameKind, CanonicalFrameOrdinal,
    ConflictZoneId, ConflictZoneKind, ConflictZoneOrdinal, EntityKindMarker, FacilityBandId,
    FacilityBandKind, FacilityBandOrdinal, JunctionId, JunctionKind, JunctionOrdinal, LaneEdgeId,
    LaneEdgeKind, LaneEdgeOrdinal, LaneGroupId, LaneGroupKind, LaneGroupOrdinal, ManeuverGateId,
    ManeuverGateKind, ManeuverGateOrdinal, ManeuverPathId, ManeuverPathKind, ManeuverPathOrdinal,
    MovementId, MovementKind, MovementOrdinal, Ordinal, OrdinalKind, ParkingFacilityId,
    ParkingFacilityKind, ParkingFacilityOrdinal, ParkingSpaceId, ParkingSpaceKind,
    ParkingSpaceOrdinal, ParticipantClassId, ParticipantClassKind, ParticipantClassOrdinal,
    ParticipantStreamId, ParticipantStreamKind, ParticipantStreamOrdinal, RightOfWayPolicySetId,
    RightOfWayPolicySetKind, RightOfWayPolicySetOrdinal, RoadCorridorId, RoadCorridorKind,
    RoadCorridorOrdinal, RoadSectionId, RoadSectionKind, RoadSectionOrdinal, SignalControllerId,
    SignalControllerKind, SignalControllerOrdinal, SignalGroupId, SignalGroupKind,
    SignalGroupOrdinal, SignalPhaseId, SignalPhaseKind, SignalPhaseOrdinal, StableId, StableId128,
    StableIdTextError, StopLineId, StopLineKind, StopLineOrdinal, VehicleProfileId,
    VehicleProfileKind, VehicleProfileOrdinal, WaitingZoneId, WaitingZoneKind, WaitingZoneOrdinal,
};
pub use values::{
    AccessEffect, CANONICAL_POINT_COMPONENT_MAX_METERS, CANONICAL_POINT_COMPONENT_MIN_METERS,
    HEADING_MINUS_PI_F32_BITS, HEADING_PLUS_PI_F32_BITS, MAX_ACCEL_METERS_PER_SECOND_SQUARED,
    MAX_CONFLICT_ZONE_REGION_RING_POINTS, MAX_LANE_EDGE_LENGTH_MM, MAX_MIN_GAP_MM,
    MAX_PARKING_LATERAL_OFFSET_ABS_MM, MAX_SPEED_MM_S, MAX_TIME_HEADWAY_SECONDS,
    MAX_VEHICLE_LENGTH_MM, MIN_ACCEL_METERS_PER_SECOND_SQUARED, MIN_LANE_EDGE_LENGTH_MM,
    MIN_PARKING_LATERAL_OFFSET_ABS_MM, MIN_SPEED_MM_S, MIN_VEHICLE_LENGTH_MM,
    PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM, PARKING_HEADING_OFFSET_MAXIMUM_RADIANS,
    PARKING_HEADING_OFFSET_MINIMUM_RADIANS, SPATIAL_CORE_LENGTH_QUANTIZATION_ALLOWANCE_METERS,
    SPATIAL_JOIN_POSITION_TOLERANCE_METERS, SPATIAL_LENGTH_ABS_TOLERANCE_METERS,
    SPATIAL_LENGTH_REL_TOLERANCE, SPATIAL_MIN_PROJECTED_UP_LENGTH,
    SPATIAL_MIN_SEGMENT_LENGTH_METERS, SignalAspect, heading_f32_from_si,
    heading_f32_in_legal_closure, millimetres_from_si, millimetres_i32_from_si,
};
