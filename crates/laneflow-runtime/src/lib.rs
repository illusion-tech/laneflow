#![doc = include_str!("../README.md")]

#[cfg(test)]
extern crate self as laneflow_runtime;
#[cfg(test)]
#[path = "../tests/support/policy.rs"]
mod test_policy;

mod config;
mod conflict;
mod conflict_tick;
mod cutover;
mod cutover_migration;
mod cutover_transaction;
mod downstream_index;
mod error;
mod handle;
mod input;
mod migration_journal;
mod observation;
mod occupancy;
mod parking;
mod policy;
mod pose;
mod routing;
mod snapshot;
mod snapshot_digest;
mod snapshot_restore;
mod source;
mod tables;
mod tick;
mod transitions;
mod units;
mod vehicle;
mod waiting;
mod waiting_dependencies;
mod waiting_graph;
mod world;

pub use config::{StepOutcome, TickInput, WorldConfig};
pub use conflict::{
    ApproachEstimate, ConflictEligibilityState, ConflictGapOutcome, ConflictLagReference,
    ConflictPassageAddress, ConflictPassageLocator, ConflictPassageOccurrenceLocator,
    ConflictPassageRange, ConflictReservation, ConflictResourceNoGrant, ConflictYieldOutcome,
    DownstreamInterval, DownstreamRoutePoint, GateCandidateKind, GatePolicyDecision,
};
pub use conflict_tick::{
    ConflictDecision, ConflictDecisionOutcome, ConflictNoGrantReason, ConflictRouteAnchor,
};
pub use cutover::{
    CUTOVER_DESCRIPTOR_FORMAT_VERSION, CutoverDescriptorError, CutoverError, CutoverEvent,
    CutoverEventBatch, CutoverPreflightLimits, LfcaOriginBinding, MigrationPolicyKind,
    NetworkRevisionCutoverDescriptor, SemanticDiffOriginBinding, WorldBinding,
};
pub use cutover_transaction::{
    CutoverCommit, CutoverTransaction, CutoverTransactionLimits, DEFAULT_MAX_CATCH_UP_LAG_TICKS,
    DEFAULT_MAX_RECORDS_PER_PUMP, PumpOutcome,
};
pub use error::{InstallError, ParkingError, ReplaceError, RouteError, SpawnError, StepError};
pub use handle::{RouteHandle, VehicleHandle};
pub use input::{RouteRegisterInput, VehicleSpawnInput};
pub use laneflow_static_contract::{ParkingFacilityOrdinal, ParkingSpaceOrdinal};
pub use migration_journal::{DEFAULT_MIGRATION_DELTA_JOURNAL_BYTES, MigrationJournalStats};
pub use observation::{
    CommittedTrafficObservationBatch, CommittedTrafficObservationRow, OBSERVATION_BINDING_VERSION,
    ObservationBatchBase, ObservationError, ObservationExportMode, ObservationExportSession,
    ObservationSelection, ObservationStateSequence, ObservationStreamBinding,
};
pub use parking::{
    LeaveParkingTarget, ParkedVehicleSpawnInput, ParkedVehicleSpawnRecord,
    ParkingArrivalObservation, ParkingBinding, ParkingCancelRecord, ParkingCommandOutcome,
    ParkingFacilityCounts, ParkingLeaveRecord, ParkingParkRecord, ParkingPoolCounts,
    ParkingRebindRecord, ParkingReservation, ParkingReserveRecord, ParkingSpaceState,
    ParkingTarget, RebindParkingTarget, ReserveParkingTarget, VehicleDespawnRecord,
    VirtualEntryAnchorSelector, VirtualExitAnchorSelector,
};
pub use policy::{DerivedPolicyGap, PolicyPin, WorldPolicySelection};
pub use pose::{CommittedPoseSourceBatch, CommittedSignalGroupBatch, PoseSource};
pub use routing::{
    AdmittedRouteRegisterError, AdmittedRouteRegisterInput, CandidateRouteError,
    CandidateRouteInput, CostModelKey, DYNAMIC_COST_BINDING_VERSION, DynamicCostBindingError,
    DynamicCostSnapshotBinding, ObservationSetBinding, ObservationSetError,
    RoutingAdmissionSession, bind_observation_set,
};
pub use snapshot::{
    CapturedManeuverTraversal, CapturedManeuverTraversalPhase, CapturedParkingBinding,
    CapturedParkingTarget, CapturedRoute, CapturedSnapshot, CapturedVehicle,
    CapturedVirtualParkingEntry, CapturedWaitingMembership, CapturedWaitingZoneState,
    RUNTIME_STATE_VERSION, SNAPSHOT_FORMAT_VERSION, SnapshotCaptureError, encode_lfrs,
};
pub use snapshot_digest::{
    RUNTIME_STATE_DIGEST_DOMAIN, RUNTIME_STATE_DIGEST_VERSION, SnapshotDigestError,
    deterministic_state_digest,
};
pub use snapshot_restore::{
    RestoredSnapshot, SnapshotLimitDimension, SnapshotRestoreError, SnapshotRestoreLimits,
    restore_lfrs,
};
pub use source::{CommittedNetworkSource, InvalidPublishedLfcaReference, PublishedLfcaReference};
pub use transitions::{TrafficTransitionAnchor, TrafficTransitionEvent, TrafficTransitionKind};
pub use vehicle::{VehicleReplaceBlock, VehicleReplaceRecord, VehicleState, VehicleStatus};
pub use waiting::{
    ManeuverTraversalPhase, ManeuverTraversalState, WaitingDecision, WaitingDecisionOutcome,
    WaitingMembership, WaitingMembershipReleaseRecord, WaitingNoGrantReason,
    WaitingProjectionReason, WaitingRouteAnchor, WaitingZoneMember, WaitingZoneSnapshot,
};
pub use world::{TrafficWorld, WorldGeneration};

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_runtime_crate_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-runtime");
    }
}
