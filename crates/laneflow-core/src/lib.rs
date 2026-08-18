#![doc = include_str!("../README.md")]

pub mod access;
mod command_spatial;
pub mod cross_section;
pub mod error;
pub mod event;
pub mod graph;
pub mod handle;
mod id;
pub mod junction;
mod longitudinal;
mod numeric_policy;
mod occupancy;
pub mod parking;
pub mod participant_class;
pub mod profile;
pub mod route;
mod route_distance;
pub mod signal;
mod step_probe;
#[cfg(test)]
mod test_support;
pub mod time;
pub mod traffic;
pub mod vehicle;
pub mod waiting;
pub mod world;

pub use access::{
    AccessCell, AccessEffect, AccessRegistry, AccessRegulation, AccessRule, AccessTargetId,
};
pub use cross_section::{
    CorridorElement, CorridorElementId, CrossSectionRegistry, FacilityBand, FacilityKind,
    FacilityKindCategory, LaneGroup, RoadCorridor, RoadSection, SeamNeighbor, SectionLane,
};
pub use error::{AccessRegulationMismatchDetails, CoreError, WaitingZoneError};
pub use event::{
    CoreEvent, ParkingReservationReleasedEvent, SignalGroupAspectChangedEvent,
    SignalPhaseChangedEvent, VehicleChangedEdgeEvent, VehicleCompletedRouteEvent,
    VehicleFollowingSafetyProjectionAppliedEvent, VehicleParkingArrivalReachedEvent,
    VehicleParkingStopProjectionAppliedEvent, VehicleSignalStopProjectionAppliedEvent,
    VehicleSpeedLimitProjectionAppliedEvent,
};
pub use graph::{EdgeLength, LaneEdge, LaneGraph, SpeedLimit};
pub use handle::{
    AccessRuleHandle, EdgeHandle, FacilityBandHandle, JunctionHandle, LaneGroupHandle,
    ManeuverGateHandle, ManeuverPathHandle, MovementHandle, ParkingAreaHandle, ParkingSpaceHandle,
    ParticipantClassHandle, RoadCorridorHandle, RoadSectionHandle, RouteHandle,
    SignalControllerHandle, SignalGroupHandle, SignalPhaseRef, StopLineHandle, VehicleHandle,
    VehicleProfileHandle, WaitingZoneHandle,
};
pub use junction::{Junction, JunctionRegistry, ManeuverPath, Movement};
pub use parking::{
    LeaveParkingInput, ParkedVehicleSpawnInput, ParkedVehicleSpawnRecord, ParkingAnchorKind,
    ParkingApproachState, ParkingArea, ParkingBindingKind, ParkingCommandEffect,
    ParkingCommandKind, ParkingCommitRecord, ParkingCounts, ParkingLaneAnchor, ParkingLeaveRecord,
    ParkingRegistry, ParkingReleaseReason, ParkingReleaseRecord,
    ParkingReservationCancellationRecord, ParkingReservationRecord, ParkingSnapshot, ParkingSpace,
    ParkingSpaceGeometry, ParkingSpaceState, RebindReservedVehicleRouteInput,
    ReservedVehicleRouteRebindRecord, VehicleParkingState,
};
pub use participant_class::{ParticipantClass, ParticipantClassRegistry};
pub use profile::{IidmProfileSpec, VehicleProfile, VehicleProfileRegistry};
pub use route::{Route, RouteRemoveRecord};
pub use signal::{
    MAX_PORTABLE_SIGNAL_TIME_MS, ManeuverGate, ManeuverGateSignalState, ManeuverGateState,
    SignalAspect, SignalControl, SignalControlInput, SignalController, SignalControllerKind,
    SignalControllerState, SignalGroup, SignalGroupSnapshot, SignalGroupState,
    SignalLayerPermission, SignalPhase, SignalRegistry, StopLine, StopLineLocation,
};
#[doc(hidden)]
pub use step_probe::{NoOpProbe, StepProbe};
#[cfg(any(test, feature = "instrumentation"))]
#[doc(hidden)]
pub use step_probe::{StageTimingProbe, StepStageTimings};
pub use time::{StepResult, TickInput};
pub use traffic::{GateOccurrence, InitialTrafficData, ManeuverOccurrence, WaitingZoneOccurrence};
pub use vehicle::{
    Acceleration, EdgeProgress, Speed, VehicleDespawnRecord, VehicleReplaceBlock,
    VehicleReplaceBlockerPosition, VehicleReplaceExternalId, VehicleReplaceInput,
    VehicleReplaceOutcome, VehicleReplaceRecord, VehicleSpawnInput, VehicleState, VehicleStatus,
};
pub use waiting::{WaitingRegistry, WaitingZone};
pub use world::CoreWorld;

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_core_crate_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-core");
    }
}
