#![doc = include_str!("../README.md")]

#[cfg(test)]
extern crate self as laneflow_runtime;
#[cfg(test)]
mod test_policy;

mod admin;
mod facade;
mod kernel;

pub use admin::cutover::{
    CUTOVER_DESCRIPTOR_FORMAT_VERSION, CutoverDescriptorError, CutoverError, CutoverEvent,
    CutoverEventBatch, CutoverPreflightLimits, LfcaOriginBinding, MigrationPolicyKind,
    NetworkRevisionCutoverDescriptor, SemanticDiffOriginBinding, WorldBinding,
};
pub use admin::cutover_transaction::{
    CutoverCommit, CutoverTransaction, CutoverTransactionLimits, DEFAULT_MAX_CATCH_UP_LAG_TICKS,
    DEFAULT_MAX_RECORDS_PER_PUMP, PumpOutcome,
};
pub use admin::migration_journal::{DEFAULT_MIGRATION_DELTA_JOURNAL_BYTES, MigrationJournalStats};
pub use admin::snapshot::{
    CapturedManeuverTraversal, CapturedManeuverTraversalPhase, CapturedParkingBinding,
    CapturedParkingTarget, CapturedRoute, CapturedSnapshot, CapturedVehicle,
    CapturedVirtualParkingEntry, CapturedWaitingMembership, CapturedWaitingZoneState,
    RUNTIME_STATE_VERSION, SNAPSHOT_FORMAT_VERSION, SnapshotCaptureError, encode_lfrs,
};
pub use admin::snapshot_digest::{
    RUNTIME_STATE_DIGEST_DOMAIN, RUNTIME_STATE_DIGEST_VERSION, SnapshotDigestError,
    deterministic_state_digest,
};
pub use admin::snapshot_restore::{
    RestoredSnapshot, SnapshotLimitDimension, SnapshotRestoreError, SnapshotRestoreLimits,
    restore_lfrs,
};
pub use facade::TrafficWorld;
pub use facade::observation::{
    CommittedTrafficObservationBatch, CommittedTrafficObservationRow, OBSERVATION_BINDING_VERSION,
    ObservationBatchBase, ObservationError, ObservationExportMode, ObservationExportSession,
    ObservationSelection, ObservationStateSequence, ObservationStreamBinding,
};
pub use facade::routing::{
    AdmittedRouteRegisterError, AdmittedRouteRegisterInput, CandidateRouteError,
    CandidateRouteInput, CostModelKey, DYNAMIC_COST_BINDING_VERSION, DynamicCostBindingError,
    DynamicCostSnapshotBinding, ObservationSetBinding, ObservationSetError,
    RoutingAdmissionSession, bind_observation_set,
};
pub use facade::source::{
    CommittedNetworkSource, InvalidPublishedLfcaReference, PublishedLfcaReference,
};
pub use kernel::config::{StepOutcome, TickInput, WorldConfig};
pub use kernel::conflict::{
    ApproachEstimate, ConflictEligibilityState, ConflictGapOutcome, ConflictLagReference,
    ConflictPassageAddress, ConflictPassageLocator, ConflictPassageOccurrenceLocator,
    ConflictPassageRange, ConflictReservation, ConflictResourceNoGrant, ConflictYieldOutcome,
    DownstreamInterval, DownstreamRoutePoint, GateCandidateKind, GatePolicyDecision,
};
pub use kernel::conflict_tick::{
    ConflictDecision, ConflictDecisionOutcome, ConflictNoGrantReason, ConflictRouteAnchor,
};
pub use kernel::error::{
    InstallError, ParkingError, ReplaceError, RouteError, SpawnError, StepError,
};
pub use kernel::handle::{RouteHandle, VehicleHandle};
pub use kernel::input::{RouteRegisterInput, VehicleSpawnInput};
pub use kernel::parking::{
    LeaveParkingTarget, ParkedVehicleSpawnInput, ParkedVehicleSpawnRecord,
    ParkingArrivalObservation, ParkingBinding, ParkingCancelRecord, ParkingCommandOutcome,
    ParkingFacilityCounts, ParkingLeaveRecord, ParkingParkRecord, ParkingPoolCounts,
    ParkingRebindRecord, ParkingReservation, ParkingReserveRecord, ParkingSpaceState,
    ParkingTarget, RebindParkingTarget, ReserveParkingTarget, VehicleDespawnRecord,
    VirtualEntryAnchorSelector, VirtualExitAnchorSelector,
};
pub use kernel::policy::{DerivedPolicyGap, PolicyPin, WorldPolicySelection};
pub use kernel::pose::{CommittedPoseSourceBatch, CommittedSignalGroupBatch, PoseSource};
pub use kernel::transitions::{
    TrafficTransitionAnchor, TrafficTransitionEvent, TrafficTransitionKind,
};
pub use kernel::vehicle::{VehicleReplaceBlock, VehicleReplaceRecord, VehicleState, VehicleStatus};
pub use kernel::waiting::{
    ManeuverTraversalPhase, ManeuverTraversalState, WaitingDecision, WaitingDecisionOutcome,
    WaitingMembership, WaitingMembershipReleaseRecord, WaitingNoGrantReason,
    WaitingProjectionReason, WaitingRouteAnchor, WaitingZoneMember, WaitingZoneSnapshot,
};
pub use kernel::world::WorldGeneration;
pub use laneflow_static_contract::{ParkingFacilityOrdinal, ParkingSpaceOrdinal};

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_runtime_crate_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-runtime");
    }
}
