#![doc = include_str!("../README.md")]

mod builder;
mod error;
mod identity;
mod numeric;
mod origin;
mod spatial;
mod traffic;

#[cfg(test)]
mod tests;

use std::sync::Arc;

pub use builder::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};
pub use error::{BuildError, BuildErrorClass, BuildStructure};
pub use identity::SharedIdentityIndex;
pub use origin::{
    CanonicalNetworkOrigin, PARTITION_PLANNING_HINTS_DERIVATION_VERSION, StaticContractVersions,
};
pub use spatial::{
    CanonicalPoint, FacilityGeometryView, LaneGeometryView, LanePoseNetwork, SegmentGeometry,
    SharedSpatialNetwork,
};
pub use traffic::{
    EntityCounts, ManeuverPathView, ManeuverTransitionCandidate, PartitionPlanningHints, RangeU32,
    SharedManeuverNetwork, SharedTrafficNetwork,
};

/// 一个不可变、同修订绑定的共享静态路网根。
///
/// component 由根直接拥有且不实现 `Clone`；调用方只应克隆外层 `Arc`。
pub struct SharedNetworkRevision {
    origin: CanonicalNetworkOrigin,
    traffic: SharedTrafficNetwork,
    identity: SharedIdentityIndex,
    planning_hints: PartitionPlanningHints,
    spatial: Option<SharedSpatialNetwork>,
}

impl SharedNetworkRevision {
    #[must_use]
    pub const fn network_revision(&self) -> laneflow_static_contract::NetworkRevisionId {
        self.origin.network_revision()
    }

    #[must_use]
    pub const fn canonical_origin(&self) -> &CanonicalNetworkOrigin {
        &self.origin
    }

    #[must_use]
    pub const fn traffic(&self) -> &SharedTrafficNetwork {
        &self.traffic
    }

    #[must_use]
    pub const fn identity(&self) -> &SharedIdentityIndex {
        &self.identity
    }

    #[must_use]
    pub const fn planning_hints(&self) -> &PartitionPlanningHints {
        &self.planning_hints
    }

    #[must_use]
    pub const fn spatial(&self) -> Option<&SharedSpatialNetwork> {
        self.spatial.as_ref()
    }

    #[must_use]
    pub fn retained_logical_bytes(&self) -> u64 {
        u64::try_from(core::mem::size_of::<Self>()).expect("root type size fits u64")
            + self.traffic.retained_logical_bytes()
            + self.identity.retained_logical_bytes()
            + self.planning_hints.retained_logical_bytes()
            + self
                .spatial
                .as_ref()
                .map_or(0, SharedSpatialNetwork::retained_logical_bytes)
    }
}

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SharedNetworkRevision>();
    assert_send_sync::<Arc<SharedNetworkRevision>>();
};
