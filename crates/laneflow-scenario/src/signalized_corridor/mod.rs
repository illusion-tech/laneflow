//! 走廊 catalog TOML 线格式与 caller-owned 人口政策。

mod bind;
mod catalog;
mod population;
mod prng;

pub use bind::{
    BindError, BoundCorridorCatalog, BoundPortalLane, BoundRouteChoice, BoundRouteExit,
    BoundSpawnSlot, bind,
};
pub use catalog::{
    AUTHORING_NAMESPACE, CATALOG_VERSION, CatalogError, CorridorCatalog, MIN_SPAWN_SLOT_COUNT,
    PASSENGER_CAR_PROFILE_KEY, PORTAL_IDS, PortalCatalogEntry, PortalLaneCatalogEntry, ROUTE_COUNT,
    RouteCatalogEntry, SHUTTLE_BUS_PROFILE_KEY, SpawnSlotCatalogEntry,
    WeightedRouteChoiceCatalogEntry, validate,
};
pub use population::{
    CorridorBoundaryReport, CorridorPopulationCapacities, CorridorPopulationConfig,
    CorridorPopulationController, CorridorPopulationCounts, CorridorPopulationError,
    CorridorPopulationPrepare, CorridorReplaceApplyError, CorridorReplaceAttemptOutcome,
    CorridorVehiclePlan, DEFAULT_SEED, DEFAULT_TARGET_VEHICLE_COUNT, MAX_TARGET_VEHICLE_COUNT,
    MIN_TARGET_VEHICLE_COUNT,
};
pub use prng::SplitMix64;
