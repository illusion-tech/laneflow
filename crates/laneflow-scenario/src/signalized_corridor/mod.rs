//! 走廊 catalog 线格式。人口迁到 Runtime 是 follow-up。

mod catalog;

pub use catalog::{
    CATALOG_VERSION, CorridorCatalog, PORTAL_IDS, PortalCatalogEntry, PortalLaneCatalogEntry,
    RouteCatalogEntry, SpawnSlotCatalogEntry, WeightedRouteChoiceCatalogEntry,
};
