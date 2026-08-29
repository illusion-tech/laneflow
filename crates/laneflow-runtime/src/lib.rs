#![doc = include_str!("../README.md")]

mod config;
mod cutover;
mod cutover_migration;
mod cutover_transaction;
mod error;
mod handle;
mod input;
mod migration_journal;
mod observation;
mod occupancy;
mod pose;
mod routing;
mod snapshot;
mod snapshot_digest;
mod snapshot_restore;
mod source;
mod tables;
mod tick;
mod units;
mod vehicle;
mod world;

pub use config::{StepOutcome, TickInput, WorldConfig};
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
pub use laneflow_static_contract::ParkingSpaceOrdinal;
pub use migration_journal::{DEFAULT_MIGRATION_DELTA_JOURNAL_BYTES, MigrationJournalStats};
pub use observation::{
    CommittedTrafficObservationBatch, CommittedTrafficObservationRow, OBSERVATION_BINDING_VERSION,
    ObservationBatchBase, ObservationError, ObservationExportMode, ObservationExportSession,
    ObservationSelection, ObservationStateSequence, ObservationStreamBinding,
};
pub use pose::{CommittedPoseSourceBatch, CommittedSignalGroupBatch, PoseSource};
pub use routing::{
    AdmittedRouteRegisterError, AdmittedRouteRegisterInput, CandidateRouteError,
    CandidateRouteInput, CostModelKey, DYNAMIC_COST_BINDING_VERSION, DynamicCostBindingError,
    DynamicCostSnapshotBinding, ObservationSetBinding, ObservationSetError,
    RoutingAdmissionSession, bind_observation_set,
};
pub use snapshot::{
    CapturedRoute, CapturedSnapshot, CapturedVehicle, RUNTIME_STATE_VERSION,
    SNAPSHOT_FORMAT_VERSION, encode_lfrs,
};
pub use snapshot_digest::{
    RUNTIME_STATE_DIGEST_DOMAIN, RUNTIME_STATE_DIGEST_VERSION, deterministic_state_digest,
};
pub use snapshot_restore::{
    RestoredSnapshot, SnapshotLimitDimension, SnapshotRestoreError, SnapshotRestoreLimits,
    restore_lfrs,
};
pub use source::{CommittedNetworkSource, InvalidPublishedLfcaReference, PublishedLfcaReference};
pub use vehicle::{VehicleReplaceBlock, VehicleReplaceRecord, VehicleState, VehicleStatus};
pub use world::{TrafficWorld, WorldGeneration};

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_runtime_crate_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-runtime");
    }
}
