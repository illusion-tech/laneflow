#![doc = include_str!("../README.md")]

mod config;
mod error;
mod handle;
mod pose;
mod world;

pub use config::{StepOutcome, TickInput, WorldConfig};
pub use error::{InstallError, LookupError, StepError};
pub use handle::{RouteHandle, VehicleHandle};
pub use pose::{CommittedPoseSourceBatch, CommittedSignalGroupBatch, PoseSource};
pub use world::TrafficWorld;

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_runtime_crate_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-runtime");
    }
}
