#![doc = include_str!("../README.md")]

mod command_spatial;
pub mod error;
pub mod event;
pub mod graph;
pub mod handle;
mod id;
mod longitudinal;
mod occupancy;
pub mod parking;
pub mod profile;
pub mod route;
mod route_distance;
pub mod signal;
pub mod time;
pub mod traffic;
pub mod vehicle;
pub mod world;

pub use error::CoreError;
pub use event::{
    CoreEvent, SignalGroupAspectChangedEvent, SignalPhaseChangedEvent, VehicleChangedEdgeEvent,
    VehicleCompletedRouteEvent, VehicleFollowingSafetyProjectionAppliedEvent,
    VehicleSignalStopProjectionAppliedEvent,
};
pub use graph::{EDGE_BOUNDARY_EPSILON, EdgeLength, LaneEdge, LaneGraph};
pub use handle::{
    EdgeHandle, ParkingAreaHandle, ParkingSpaceHandle, RouteHandle, SignalControllerHandle,
    SignalGroupHandle, SignalPhaseRef, StopLineHandle, VehicleHandle, VehicleProfileHandle,
};
pub use parking::{
    LeaveParkingInput, ParkedVehicleSpawnInput, ParkedVehicleSpawnRecord, ParkingAnchorKind,
    ParkingApproachState, ParkingArea, ParkingBindingKind, ParkingCommandEffect,
    ParkingCommandKind, ParkingCommitRecord, ParkingCounts, ParkingLaneAnchor, ParkingLeaveRecord,
    ParkingRegistry, ParkingReleaseReason, ParkingReleaseRecord,
    ParkingReservationCancellationRecord, ParkingReservationRecord, ParkingSnapshot, ParkingSpace,
    ParkingSpaceGeometry, ParkingSpaceState, RebindReservedVehicleRouteInput,
    ReservedVehicleRouteRebindRecord, VehicleParkingState,
};
pub use profile::{GEOMETRY_GAP_EPSILON, IidmProfileSpec, VehicleProfile, VehicleProfileRegistry};
pub use route::{Route, RouteRemoveRecord};
pub use signal::{
    MAX_PORTABLE_SIGNAL_TIME_MS, MovementGate, MovementGateKey, MovementGateSignalState,
    MovementGateState, SignalAspect, SignalControl, SignalControlInput, SignalController,
    SignalControllerKind, SignalControllerState, SignalGroup, SignalGroupSnapshot,
    SignalGroupState, SignalLayerPermission, SignalPhase, SignalRegistry, StopLine,
    StopLineLocation,
};
pub use time::{StepResult, TickInput};
pub use traffic::InitialTrafficData;
pub use vehicle::{
    Acceleration, EdgeProgress, Speed, VehicleDespawnRecord, VehicleSpawnInput, VehicleState,
    VehicleStatus,
};
pub use world::CoreWorld;

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_core_crate_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-core");
    }
}
