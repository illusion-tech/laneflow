//! v0.9 protected-turning signalized-corridor 的 caller-owned population policy。

mod catalog;
mod error;
mod population;
mod prng;

pub use catalog::{
    CATALOG_VERSION, CorridorCatalog, NormalizedCorridorCatalog, NormalizedPortal,
    NormalizedPortalLane, NormalizedRoute, NormalizedSpawnSlot, NormalizedWeightedRouteChoice,
    PORTAL_IDS, PortalCatalogEntry, PortalLaneCatalogEntry, RouteCatalogEntry,
    SpawnSlotCatalogEntry, WeightedRouteChoiceCatalogEntry,
};
pub use error::CorridorPopulationError;
pub use population::{
    CorridorBoundaryReport, CorridorPopulationCapacities, CorridorPopulationConfig,
    CorridorPopulationController, CorridorPopulationCounts, CorridorPopulationPrepare,
    CorridorReplaceApplyError, CorridorReplaceAttemptOutcome, DEFAULT_SEED,
    DEFAULT_TARGET_VEHICLE_COUNT, MAX_TARGET_VEHICLE_COUNT, MIN_TARGET_VEHICLE_COUNT,
};
pub use prng::SplitMix64;
