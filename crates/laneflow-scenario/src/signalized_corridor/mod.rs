//! 走廊 catalog TOML 线格式。

mod bind;
mod catalog;

pub use bind::{BindError, BoundCorridorCatalog, BoundSpawnSlot, bind};
pub use catalog::{
    CATALOG_VERSION, CorridorCatalog, PORTAL_IDS, PortalCatalogEntry, PortalLaneCatalogEntry,
    RouteCatalogEntry, SpawnSlotCatalogEntry, WeightedRouteChoiceCatalogEntry,
};
