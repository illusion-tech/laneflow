#![doc = include_str!("../README.md")]

mod error;
mod plugin;
mod session;

pub use error::LaneFlowAdapterError;
pub use plugin::{LaneFlowFixed, LaneFlowOuterFrame, LaneFlowPlugin};
pub use session::{LaneFlowFrameReport, LaneFlowSession, LaneFlowSessionConfig};

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_bevy_adapter_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-bevy");
    }
}
