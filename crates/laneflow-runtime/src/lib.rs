#![doc = include_str!("../README.md")]

mod config;
mod cutover;
mod error;
mod handle;
mod input;
mod occupancy;
mod pose;
mod source;
mod tables;
mod tick;
mod units;
mod vehicle;
mod world;

pub use config::{StepOutcome, TickInput, WorldConfig};
pub use cutover::{
    CUTOVER_DESCRIPTOR_FORMAT_VERSION, CutoverDescriptorError, CutoverPreflightLimits,
    LfcaOriginBinding, MigrationPolicyKind, NetworkRevisionCutoverDescriptor,
    SemanticDiffOriginBinding, WorldBinding,
};
pub use error::{InstallError, ParkingError, ReplaceError, RouteError, SpawnError, StepError};
pub use handle::{RouteHandle, VehicleHandle};
pub use input::{RouteRegisterInput, VehicleSpawnInput};
pub use laneflow_static_contract::ParkingSpaceOrdinal;
pub use pose::{CommittedPoseSourceBatch, CommittedSignalGroupBatch, PoseSource};
pub use source::{CommittedNetworkSource, InvalidPublishedLfcaReference, PublishedLfcaReference};
pub use vehicle::{VehicleReplaceBlock, VehicleReplaceRecord, VehicleState, VehicleStatus};
pub use world::TrafficWorld;

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_runtime_crate_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-runtime");
    }
}
