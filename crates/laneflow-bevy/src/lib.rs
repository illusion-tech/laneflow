#![doc = include_str!("../README.md")]

mod error;
mod lifecycle;
mod observation;
mod plugin;
mod session;

pub use error::LaneFlowAdapterError;
pub use lifecycle::{
    LaneFlowVehicleDespawnRecord, LaneFlowVehicleReplaceOutcome, LaneFlowVehicleReplaceRecord,
    despawn_vehicle, replace_completed_vehicle,
};
pub use observation::{
    LaneFlowJunctionObservation, LaneFlowJunctionObservationContext, LaneFlowJunctionVehicleRow,
};
pub use plugin::{
    LaneFlowFixed, LaneFlowFixedSet, LaneFlowOuterFrame, LaneFlowOuterFrameSet, LaneFlowPlugin,
};
pub use session::{
    LaneFlowCommittedPoseBatch, LaneFlowConsumptionContext, LaneFlowCutoverRecord,
    LaneFlowFrameReport, LaneFlowSession, LaneFlowSessionConfig, LaneFlowTargetSpatial,
    LaneFlowWorldMut,
};

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_bevy_adapter_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-bevy");
    }
}
