//! 走廊 catalog TOML 线格式。

mod bind;
mod catalog;

pub use bind::{BindError, BoundCorridorCatalog, BoundSpawnSlot, bind};
pub use catalog::{
    AUTHORING_NAMESPACE, CATALOG_VERSION, CatalogError, CorridorCatalog, MIN_SPAWN_SLOT_COUNT,
    PASSENGER_CAR_PROFILE_KEY, PORTAL_IDS, PortalCatalogEntry, PortalLaneCatalogEntry, ROUTE_COUNT,
    RouteCatalogEntry, SHUTTLE_BUS_PROFILE_KEY, SpawnSlotCatalogEntry,
    WeightedRouteChoiceCatalogEntry, validate,
};
