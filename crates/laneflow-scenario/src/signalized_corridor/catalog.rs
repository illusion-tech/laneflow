use std::collections::{HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};

/// 当前 scenario-local corridor catalog 版本。
pub const CATALOG_VERSION: &str = "0.2";

/// 走廊编制 Identity v1 的 `AuthoringNamespaceId`。
pub const AUTHORING_NAMESPACE: &str = "laneflow/signalized-corridor";

/// 现行走廊最小路径默认使用的车型编制键。
pub const PASSENGER_CAR_PROFILE_KEY: &str = "passenger-car";

/// 走廊编制中的第二车型键；bind 会解析但不作为默认 spawn profile。
pub const SHUTTLE_BUS_PROFILE_KEY: &str = "shuttle-bus";

/// 本封闭 catalog 的 Traffic Route 数。
pub const ROUTE_COUNT: usize = 28;

/// 本封闭 catalog 要求的最少 physical spawn slot 数。
pub const MIN_SPAWN_SLOT_COUNT: usize = 200;

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

/// catalog 0.2 线格式或交叉引用不合法。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogError {
    UnsupportedVersion(String),
    PortalSet,
    DuplicatePortal(String),
    LaneCount {
        portal_id: String,
        expected: usize,
        actual: usize,
    },
    LaneIndex {
        portal_id: String,
        expected: usize,
        actual: usize,
    },
    EmptyRouteChoices {
        portal_id: String,
        lane_index: usize,
    },
    ZeroWeight {
        portal_id: String,
        route_id: String,
    },
    WeightOverflow {
        portal_id: String,
    },
    DuplicateChoice {
        portal_id: String,
        route_id: String,
    },
    DuplicateRoute(String),
    UnknownPortal(String),
    RouteCount(usize),
    UnknownRoute(String),
    SameEntryExit {
        route_id: String,
    },
    UnreferencedRoute(String),
    InsufficientSlots(usize),
    DuplicateSlot(String),
    EmptyId {
        field: &'static str,
    },
    InvalidProgress {
        slot_id: String,
    },
    DuplicatePosition {
        slot_id: String,
    },
    SlotLane {
        slot_id: String,
    },
    MissingEntrySlot {
        portal_id: String,
        lane_index: usize,
    },
    EntrySlotMismatch {
        portal_id: String,
        lane_index: usize,
    },
}

impl fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported catalog_version {version:?}")
            }
            Self::PortalSet => write!(
                formatter,
                "catalog portals must be exactly {PORTAL_IDS:?} in that order"
            ),
            Self::DuplicatePortal(id) => write!(formatter, "duplicate portal {id:?}"),
            Self::LaneCount {
                portal_id,
                expected,
                actual,
            } => write!(
                formatter,
                "portal {portal_id:?} must have {expected} lanes, found {actual}"
            ),
            Self::LaneIndex {
                portal_id,
                expected,
                actual,
            } => write!(
                formatter,
                "portal {portal_id:?} lane index {actual} must be {expected}"
            ),
            Self::EmptyRouteChoices {
                portal_id,
                lane_index,
            } => write!(
                formatter,
                "portal {portal_id:?} lane {lane_index} has no route choices"
            ),
            Self::ZeroWeight {
                portal_id,
                route_id,
            } => write!(
                formatter,
                "portal {portal_id:?} route {route_id:?} has zero weight"
            ),
            Self::WeightOverflow { portal_id } => {
                write!(formatter, "portal {portal_id:?} route weights overflow")
            }
            Self::DuplicateChoice {
                portal_id,
                route_id,
            } => write!(
                formatter,
                "portal {portal_id:?} repeats route choice {route_id:?}"
            ),
            Self::DuplicateRoute(id) => write!(formatter, "duplicate catalog route {id:?}"),
            Self::UnknownPortal(id) => write!(formatter, "unknown portal {id:?}"),
            Self::RouteCount(actual) => {
                write!(
                    formatter,
                    "catalog must list {ROUTE_COUNT} routes, found {actual}"
                )
            }
            Self::UnknownRoute(id) => write!(formatter, "unknown catalog route {id:?}"),
            Self::SameEntryExit { route_id } => {
                write!(
                    formatter,
                    "route {route_id:?} has the same entry and exit portal"
                )
            }
            Self::UnreferencedRoute(id) => {
                write!(
                    formatter,
                    "catalog route {id:?} is not used by any portal lane"
                )
            }
            Self::InsufficientSlots(actual) => write!(
                formatter,
                "catalog must provide at least {MIN_SPAWN_SLOT_COUNT} spawn slots, found {actual}"
            ),
            Self::DuplicateSlot(id) => write!(formatter, "duplicate spawn slot {id:?}"),
            Self::EmptyId { field } => write!(formatter, "{field} must not be empty"),
            Self::InvalidProgress { slot_id } => {
                write!(
                    formatter,
                    "spawn slot {slot_id:?} progress is not finite or is negative"
                )
            }
            Self::DuplicatePosition { slot_id } => {
                write!(
                    formatter,
                    "spawn slot {slot_id:?} repeats a physical position"
                )
            }
            Self::SlotLane { slot_id } => {
                write!(
                    formatter,
                    "spawn slot {slot_id:?} has no matching portal lane"
                )
            }
            Self::MissingEntrySlot {
                portal_id,
                lane_index,
            } => write!(
                formatter,
                "portal {portal_id:?} lane {lane_index} entry_spawn_slot_id is missing"
            ),
            Self::EntrySlotMismatch {
                portal_id,
                lane_index,
            } => write!(
                formatter,
                "portal {portal_id:?} lane {lane_index} entry slot is not on that portal lane"
            ),
        }
    }
}

impl std::error::Error for CatalogError {}

/// 校验封闭 catalog 0.2 的版本、重复 ID、portal/lane/weight 与 slot 交叉引用。
///
/// 边是否属于所选 route、progress 是否落在已安装修订的边长内，由 `bind` 对照共享路网修订检查。
pub fn validate(catalog: &CorridorCatalog) -> Result<(), CatalogError> {
    if catalog.catalog_version != CATALOG_VERSION {
        return Err(CatalogError::UnsupportedVersion(
            catalog.catalog_version.clone(),
        ));
    }
    if catalog.portals.len() != PORTAL_IDS.len() {
        return Err(CatalogError::PortalSet);
    }

    let mut seen_portals = HashSet::new();
    for (index, portal) in catalog.portals.iter().enumerate() {
        if portal.id != PORTAL_IDS[index] {
            return Err(CatalogError::PortalSet);
        }
        require_id("portal.id", &portal.id)?;
        if !seen_portals.insert(portal.id.as_str()) {
            return Err(CatalogError::DuplicatePortal(portal.id.clone()));
        }
        let expected_lanes = if PORTAL_IDS[index].contains("-main-") {
            3
        } else {
            2
        };
        if portal.lanes.len() != expected_lanes {
            return Err(CatalogError::LaneCount {
                portal_id: portal.id.clone(),
                expected: expected_lanes,
                actual: portal.lanes.len(),
            });
        }
        for (lane_index, lane) in portal.lanes.iter().enumerate() {
            if lane.lane_index != lane_index {
                return Err(CatalogError::LaneIndex {
                    portal_id: portal.id.clone(),
                    expected: lane_index,
                    actual: lane.lane_index,
                });
            }
            if lane.route_choices.is_empty() {
                return Err(CatalogError::EmptyRouteChoices {
                    portal_id: portal.id.clone(),
                    lane_index,
                });
            }
            require_id("entry_spawn_slot_id", &lane.entry_spawn_slot_id)?;
            let mut choice_routes = HashSet::new();
            let mut weight_sum = 0_u64;
            for choice in &lane.route_choices {
                require_id("route_id", &choice.route_id)?;
                if choice.weight == 0 {
                    return Err(CatalogError::ZeroWeight {
                        portal_id: portal.id.clone(),
                        route_id: choice.route_id.clone(),
                    });
                }
                weight_sum = weight_sum.checked_add(choice.weight).ok_or_else(|| {
                    CatalogError::WeightOverflow {
                        portal_id: portal.id.clone(),
                    }
                })?;
                if !choice_routes.insert(choice.route_id.as_str()) {
                    return Err(CatalogError::DuplicateChoice {
                        portal_id: portal.id.clone(),
                        route_id: choice.route_id.clone(),
                    });
                }
            }
            let _ = weight_sum;
        }
    }

    if catalog.routes.len() != ROUTE_COUNT {
        return Err(CatalogError::RouteCount(catalog.routes.len()));
    }
    let mut route_ids = HashSet::new();
    let mut route_exit = HashMap::new();
    for route in &catalog.routes {
        require_id("route_id", &route.route_id)?;
        require_id("exit_portal_id", &route.exit_portal_id)?;
        if !route_ids.insert(route.route_id.as_str()) {
            return Err(CatalogError::DuplicateRoute(route.route_id.clone()));
        }
        if !PORTAL_IDS.contains(&route.exit_portal_id.as_str()) {
            return Err(CatalogError::UnknownPortal(route.exit_portal_id.clone()));
        }
        route_exit.insert(route.route_id.as_str(), route.exit_portal_id.as_str());
    }

    let mut referenced_routes = HashSet::new();
    for portal in &catalog.portals {
        for lane in &portal.lanes {
            for choice in &lane.route_choices {
                let exit = route_exit
                    .get(choice.route_id.as_str())
                    .ok_or_else(|| CatalogError::UnknownRoute(choice.route_id.clone()))?;
                if *exit == portal.id.as_str() {
                    return Err(CatalogError::SameEntryExit {
                        route_id: choice.route_id.clone(),
                    });
                }
                referenced_routes.insert(choice.route_id.as_str());
            }
        }
    }
    for route in &catalog.routes {
        if !referenced_routes.contains(route.route_id.as_str()) {
            return Err(CatalogError::UnreferencedRoute(route.route_id.clone()));
        }
    }

    if catalog.spawn_slots.len() < MIN_SPAWN_SLOT_COUNT {
        return Err(CatalogError::InsufficientSlots(catalog.spawn_slots.len()));
    }

    let lane_keys = catalog
        .portals
        .iter()
        .flat_map(|portal| {
            portal
                .lanes
                .iter()
                .map(move |lane| (portal.id.as_str(), lane.lane_index))
        })
        .collect::<HashSet<_>>();
    let mut slot_ids = HashSet::new();
    let mut positions = HashSet::new();
    let mut slot_by_id = HashMap::new();
    for slot in &catalog.spawn_slots {
        require_id("slot_id", &slot.slot_id)?;
        require_id("portal_id", &slot.portal_id)?;
        require_id("edge_id", &slot.edge_id)?;
        if !slot_ids.insert(slot.slot_id.as_str()) {
            return Err(CatalogError::DuplicateSlot(slot.slot_id.clone()));
        }
        if !slot.progress.is_finite() || slot.progress < 0.0 {
            return Err(CatalogError::InvalidProgress {
                slot_id: slot.slot_id.clone(),
            });
        }
        let progress_bits = if slot.progress == 0.0 {
            0.0_f64.to_bits()
        } else {
            slot.progress.to_bits()
        };
        if !positions.insert((slot.edge_id.as_str(), progress_bits)) {
            return Err(CatalogError::DuplicatePosition {
                slot_id: slot.slot_id.clone(),
            });
        }
        if !PORTAL_IDS.contains(&slot.portal_id.as_str()) {
            return Err(CatalogError::UnknownPortal(slot.portal_id.clone()));
        }
        if !lane_keys.contains(&(slot.portal_id.as_str(), slot.lane_index)) {
            return Err(CatalogError::SlotLane {
                slot_id: slot.slot_id.clone(),
            });
        }
        slot_by_id.insert(slot.slot_id.as_str(), slot);
    }

    for portal in &catalog.portals {
        for lane in &portal.lanes {
            let Some(entry) = slot_by_id.get(lane.entry_spawn_slot_id.as_str()) else {
                return Err(CatalogError::MissingEntrySlot {
                    portal_id: portal.id.clone(),
                    lane_index: lane.lane_index,
                });
            };
            if entry.portal_id != portal.id || entry.lane_index != lane.lane_index {
                return Err(CatalogError::EntrySlotMismatch {
                    portal_id: portal.id.clone(),
                    lane_index: lane.lane_index,
                });
            }
        }
    }
    Ok(())
}

fn require_id(field: &'static str, value: &str) -> Result<(), CatalogError> {
    if value.is_empty() {
        return Err(CatalogError::EmptyId { field });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN: &str =
        include_str!("../../../../examples/data/v0.2-signalized-corridor.catalog.toml");

    fn golden_catalog() -> CorridorCatalog {
        toml::from_str(GOLDEN).expect("checked-in catalog must parse")
    }

    #[test]
    fn checked_in_catalog_is_closed_0_2() {
        validate(&golden_catalog()).expect("checked-in catalog 0.2");
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut catalog = golden_catalog();
        catalog.catalog_version = "0.1".to_owned();
        assert_eq!(
            validate(&catalog),
            Err(CatalogError::UnsupportedVersion("0.1".to_owned()))
        );
    }

    #[test]
    fn rejects_duplicate_slot_and_zero_weight() {
        let mut catalog = golden_catalog();
        catalog.spawn_slots[1].slot_id = catalog.spawn_slots[0].slot_id.clone();
        assert!(matches!(
            validate(&catalog),
            Err(CatalogError::DuplicateSlot(_))
        ));

        let mut catalog = golden_catalog();
        catalog.portals[0].lanes[0].route_choices[0].weight = 0;
        assert!(matches!(
            validate(&catalog),
            Err(CatalogError::ZeroWeight { .. })
        ));
    }

    #[test]
    fn rejects_non_finite_or_negative_progress_and_empty_ids() {
        let mut catalog = golden_catalog();
        catalog.spawn_slots[0].progress = f64::NAN;
        assert!(matches!(
            validate(&catalog),
            Err(CatalogError::InvalidProgress { .. })
        ));

        let mut catalog = golden_catalog();
        catalog.spawn_slots[0].progress = -1.0;
        assert!(matches!(
            validate(&catalog),
            Err(CatalogError::InvalidProgress { .. })
        ));

        let mut catalog = golden_catalog();
        catalog.spawn_slots[0].slot_id.clear();
        assert_eq!(
            validate(&catalog),
            Err(CatalogError::EmptyId { field: "slot_id" })
        );
    }
}
