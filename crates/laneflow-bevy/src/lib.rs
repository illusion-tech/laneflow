#![doc = include_str!("../README.md")]

#[cfg(feature = "debug-gizmos")]
mod debug;
mod error;
mod plugin;
mod presentation;
mod session;

#[cfg(feature = "debug-gizmos")]
pub use debug::{
    LaneFlowDebugCenterlineStatus, LaneFlowDebugCenterlines, LaneFlowDebugGizmosConfig,
    LaneFlowDebugGizmosPlugin, LaneFlowDebugGizmosReport, LaneFlowDebugGizmosStatus,
    LaneFlowDebugVehicleFilter,
};
pub use error::LaneFlowAdapterError;
pub use plugin::{LaneFlowFixed, LaneFlowOuterFrame, LaneFlowPlugin};
pub use presentation::{
    LaneFlowFramePlacement, LaneFlowPresentationReport, LaneFlowVehicleEntityMap,
};
pub use session::{LaneFlowFrameReport, LaneFlowSession, LaneFlowSessionConfig};

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_bevy_adapter_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-bevy");
    }
}
