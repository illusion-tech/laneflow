use serde::{Deserialize, Serialize};

/// 当前 scenario-local corridor catalog 版本。
pub const CATALOG_VERSION: &str = "0.2";

/// 走廊 portal 的规范顺序。
pub const PORTAL_IDS: [&str; 6] = [
    "portal-main-west",
    "portal-main-east",
    "portal-side-1-north",
    "portal-side-1-south",
    "portal-side-2-north",
    "portal-side-2-south",
];

/// signalized-corridor 使用的 closed TOML catalog。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorridorCatalog {
    /// 内部 catalog 版本。
    pub catalog_version: String,
    /// portal entries。
    pub portals: Vec<PortalCatalogEntry>,
    /// Traffic route 到 exit portal 的 cross-reference。
    pub routes: Vec<RouteCatalogEntry>,
    /// route-independent physical spawn slots。
    pub spawn_slots: Vec<SpawnSlotCatalogEntry>,
}

/// corridor portal wire entry。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortalCatalogEntry {
    /// portal 外部 ID。
    pub id: String,
    /// 按 lane index 排序的 entry lanes。
    pub lanes: Vec<PortalLaneCatalogEntry>,
}

/// corridor portal lane wire entry。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortalLaneCatalogEntry {
    /// portal-local lane index。
    pub lane_index: usize,
    /// replacement 使用的共享 entry spawn slot。
    pub entry_spawn_slot_id: String,
    /// 按确定性 cumulative-selection 顺序排列的 route choices。
    pub route_choices: Vec<WeightedRouteChoiceCatalogEntry>,
}

/// corridor weighted route-choice wire entry。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WeightedRouteChoiceCatalogEntry {
    /// production Traffic route ID。
    pub route_id: String,
    /// lane-local 正整数 raw weight。
    pub weight: u64,
}

/// corridor route cross-reference wire entry。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteCatalogEntry {
    /// production Traffic route ID。
    pub route_id: String,
    /// exit portal ID。
    pub exit_portal_id: String,
}

/// corridor spawn slot wire entry。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnSlotCatalogEntry {
    /// stable slot ID。
    pub slot_id: String,
    /// slot 所属 entry portal。
    pub portal_id: String,
    /// slot 所属 portal-local lane。
    pub lane_index: usize,
    /// production Traffic edge ID。
    pub edge_id: String,
    /// vehicle 前保险杠 edge-local progress。
    pub progress: f64,
}
