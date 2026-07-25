#![doc = include_str!("../README.md")]

mod command_spatial;
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
pub mod profile;
pub mod route;
mod route_distance;
pub mod signal;
#[cfg(test)]
mod test_support;
pub mod time;
pub mod traffic;
pub mod vehicle;
pub mod world;

pub use error::CoreError;
pub use event::{
    CoreEvent, ParkingReservationReleasedEvent, SignalGroupAspectChangedEvent,
    SignalPhaseChangedEvent, VehicleChangedEdgeEvent, VehicleCompletedRouteEvent,
    VehicleFollowingSafetyProjectionAppliedEvent, VehicleParkingArrivalReachedEvent,
    VehicleParkingStopProjectionAppliedEvent, VehicleSignalStopProjectionAppliedEvent,
    VehicleSpeedLimitProjectionAppliedEvent,
};
pub use graph::{EdgeLength, LaneEdge, LaneGraph, SpeedLimit};
pub use handle::{
    EdgeHandle, JunctionHandle, ManeuverGateHandle, ManeuverPathHandle, MovementHandle,
    ParkingAreaHandle, ParkingSpaceHandle, RouteHandle, SignalControllerHandle, SignalGroupHandle,
    SignalPhaseRef, StopLineHandle, VehicleHandle, VehicleProfileHandle,
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
pub use profile::{IidmProfileSpec, VehicleProfile, VehicleProfileRegistry};
pub use route::{Route, RouteRemoveRecord};
pub use signal::{
    MAX_PORTABLE_SIGNAL_TIME_MS, ManeuverGate, ManeuverGateSignalState, ManeuverGateState,
    SignalAspect, SignalControl, SignalControlInput, SignalController, SignalControllerKind,
    SignalControllerState, SignalGroup, SignalGroupSnapshot, SignalGroupState,
    SignalLayerPermission, SignalPhase, SignalRegistry, StopLine, StopLineLocation,
};
pub use time::{StepResult, TickInput};
pub use traffic::{InitialTrafficData, ManeuverOccurrence};
pub use vehicle::{
    Acceleration, EdgeProgress, Speed, VehicleDespawnRecord, VehicleReplaceBlock,
    VehicleReplaceBlockerPosition, VehicleReplaceExternalId, VehicleReplaceInput,
    VehicleReplaceOutcome, VehicleReplaceRecord, VehicleSpawnInput, VehicleState, VehicleStatus,
};
pub use world::CoreWorld;

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_core_crate_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-core");
    }
}
