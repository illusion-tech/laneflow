#![doc = include_str!("../README.md")]

pub mod error;
pub mod event;
pub mod graph;
pub mod handle;
mod id;
pub mod route;
pub mod time;
pub mod vehicle;
pub mod world;

pub use error::CoreError;
pub use event::{CoreEvent, VehicleChangedEdgeEvent, VehicleCompletedRouteEvent};
pub use graph::{EDGE_BOUNDARY_EPSILON, EdgeLength, LaneEdge, LaneGraph};
pub use handle::{EdgeHandle, RouteHandle, VehicleHandle};
pub use route::{Route, RouteRemoveRecord};
pub use time::{StepResult, TickInput};
pub use vehicle::{
    EdgeProgress, Speed, VehicleDespawnRecord, VehicleSpawnInput, VehicleState, VehicleStatus,
};
pub use world::CoreWorld;

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_core_crate_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-core");
    }
}
