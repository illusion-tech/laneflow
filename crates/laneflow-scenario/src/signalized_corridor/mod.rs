//! 走廊 catalog TOML 线格式。

mod catalog;

pub use catalog::{
    CATALOG_VERSION, CorridorCatalog, PORTAL_IDS, PortalCatalogEntry, PortalLaneCatalogEntry,
    RouteCatalogEntry, SpawnSlotCatalogEntry, WeightedRouteChoiceCatalogEntry,
};
