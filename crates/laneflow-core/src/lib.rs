#![doc = include_str!("../README.md")]

pub mod error;
pub mod event;
pub mod graph;
pub mod route;
pub mod time;
pub mod vehicle;
pub mod world;

pub use error::CoreError;
pub use event::CoreEvent;
pub use graph::{EDGE_BOUNDARY_EPSILON, EdgeLength, LaneEdge, LaneGraph};
pub use route::Route;
pub use time::{StepResult, TickInput};
pub use vehicle::{EdgeProgress, Speed, VehicleState, VehicleStatus};
pub use world::CoreWorld;

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_core_crate_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-core");
    }
}
