//! 复杂路口 catalog TOML 线格式与绑定。
//!
//! 覆盖 #285 复杂路口参考场景：单四岔信号路口，主路东西 2+2 车道、次路南北 1+1
//! 车道，含保护左转待转区、许可左转与冲突区，外围绕行环路让路线可以成环重复过门。

mod bind;
mod catalog;

pub use bind::{
    BindError, BoundJunctionCatalog, BoundPortalLane, BoundRouteChoice, BoundRouteExit,
    BoundSpawnSlot, bind,
};
pub use catalog::{
    AUTHORING_NAMESPACE, CATALOG_VERSION, CatalogError, CatalogPolicySelection, FOCUS_ROUTE_IDS,
    JunctionCatalog, MIN_SPAWN_SLOT_COUNT, POLICY_KEY, PORTAL_IDS, PORTAL_LANE_COUNTS,
    PortalCatalogEntry, PortalLaneCatalogEntry, ROUTE_COUNT, RouteCatalogEntry,
    SpawnSlotCatalogEntry, VEHICLE_PROFILE_KEY, WeightedRouteChoiceCatalogEntry, validate,
};
