#![no_std]
#![doc = include_str!("../README.md")]

#[cfg(test)]
extern crate std;

mod registry;
mod typed;
mod values;

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
