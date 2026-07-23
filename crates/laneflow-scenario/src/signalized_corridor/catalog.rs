use std::collections::{HashMap, HashSet};

use laneflow_core::{EdgeProgress, InitialTrafficData};
use serde::{Deserialize, Serialize};

use super::CorridorPopulationError;

/// 当前 scenario-local corridor catalog 版本。
pub const CATALOG_VERSION: &str = "0.1";

/// v0.8 portal 的规范顺序。
pub const PORTAL_IDS: [&str; 6] = [
    "portal-main-west",
    "portal-main-east",
    "portal-side-1-north",
    "portal-side-1-south",
    "portal-side-2-north",
    "portal-side-2-south",
];

/// #188 生成、#203 消费的 closed TOML catalog。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorridorCatalog {
    /// 内部 catalog 版本。
    pub catalog_version: String,
    /// portal entries。
    pub portals: Vec<PortalCatalogEntry>,
    /// lane route entries。
    pub routes: Vec<RouteCatalogEntry>,
    /// stable spawn slots。
    pub spawn_slots: Vec<SpawnSlotCatalogEntry>,
}

/// corridor portal wire entry。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortalCatalogEntry {
    /// portal external ID。
    pub id: String,
    /// 从该 portal 进入的 lane route IDs。
    pub entry_route_ids: Vec<String>,
}

/// corridor lane route wire entry。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteCatalogEntry {
    /// production Traffic route ID。
    pub route_id: String,
    /// entry portal ID。
    pub entry_portal_id: String,
    /// exit portal ID。
    pub exit_portal_id: String,
    /// portal-local lane index。
    pub lane_index: usize,
    /// replacement 使用的 entry spawn slot。
    pub entry_spawn_slot_id: String,
}

/// corridor spawn slot wire entry。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnSlotCatalogEntry {
    /// stable slot ID。
    pub slot_id: String,
    /// slot 所属 entry portal。
    pub portal_id: String,
    /// slot 所属 route。
    pub route_id: String,
    /// slot 在 route 中的 edge occurrence。
    pub route_edge_index: usize,
    /// production Traffic edge ID。
    pub edge_id: String,
    /// vehicle 前保险杠 edge-local progress。
    pub progress: f64,
}

/// 已完成 v0.8 semantic validation 和稳定排序的 runtime catalog。
#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedCorridorCatalog {
    pub(super) portals: Vec<NormalizedPortal>,
    pub(super) routes: Vec<NormalizedRoute>,
    pub(super) spawn_slots: Vec<NormalizedSpawnSlot>,
}

/// 规范化 portal。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedPortal {
    pub(super) id: String,
    pub(super) route_indices: Vec<usize>,
}

/// 规范化 lane route。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedRoute {
    pub(super) id: String,
    pub(super) entry_portal_index: usize,
    pub(super) exit_portal_index: usize,
    pub(super) lane_index: usize,
    pub(super) entry_spawn_slot_index: usize,
}

/// 规范化 spawn slot。
#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedSpawnSlot {
    pub(super) id: String,
    pub(super) portal_index: usize,
    pub(super) route_index: usize,
    pub(super) route_edge_index: usize,
    pub(super) edge_id: String,
    pub(super) edge_progress: EdgeProgress,
}

impl CorridorCatalog {
    /// 从 caller 提供的内存 TOML 解析 closed wire shape。
    pub fn parse(input: &str) -> Result<Self, CorridorPopulationError> {
        Ok(toml::from_str(input)?)
    }

    /// 以 production loader 已规范化的 Traffic 输入完成 cross-reference validation 和排序。
    pub fn normalize(
        self,
        traffic: &InitialTrafficData,
    ) -> Result<NormalizedCorridorCatalog, CorridorPopulationError> {
        if self.catalog_version != CATALOG_VERSION {
            return Err(CorridorPopulationError::UnsupportedCatalogVersion {
                expected: CATALOG_VERSION,
                actual: self.catalog_version,
            });
        }

        let mut portal_entries: [Option<PortalCatalogEntry>; 6] = std::array::from_fn(|_| None);
        for portal in self.portals {
            let Some(portal_index) = portal_rank(&portal.id) else {
                return Err(CorridorPopulationError::UnknownPortal {
                    portal_id: portal.id,
                });
            };
            if portal_entries[portal_index].is_some() {
                return Err(CorridorPopulationError::DuplicatePortal {
                    portal_id: portal.id,
                });
            }
            let expected = expected_lane_count(portal_index);
            if portal.entry_route_ids.len() != expected {
                return Err(CorridorPopulationError::InvalidPortalRouteCount {
                    portal_id: portal.id,
                    expected,
                    actual: portal.entry_route_ids.len(),
                });
            }
            portal_entries[portal_index] = Some(portal);
        }
        for (portal_index, portal) in portal_entries.iter().enumerate() {
            if portal.is_none() {
                return Err(CorridorPopulationError::MissingPortal {
                    portal_id: PORTAL_IDS[portal_index],
                });
            }
        }

        let mut route_ids = HashSet::with_capacity(self.routes.len());
        let mut portal_lanes = HashSet::with_capacity(self.routes.len());
        let mut routes = Vec::with_capacity(self.routes.len());
        for route in self.routes {
            if !route_ids.insert(route.route_id.clone()) {
                return Err(CorridorPopulationError::DuplicateRoute {
                    route_id: route.route_id,
                });
            }
            let Some(entry_portal_index) = portal_rank(&route.entry_portal_id) else {
                return Err(CorridorPopulationError::UnknownPortal {
                    portal_id: route.entry_portal_id,
                });
            };
            let Some(exit_portal_index) = portal_rank(&route.exit_portal_id) else {
                return Err(CorridorPopulationError::UnknownPortal {
                    portal_id: route.exit_portal_id,
                });
            };
            if entry_portal_index == exit_portal_index {
                return Err(CorridorPopulationError::InvalidRoutePortals {
                    route_id: route.route_id,
                    entry_portal_id: route.entry_portal_id,
                    exit_portal_id: route.exit_portal_id,
                });
            }
            let lane_count = expected_lane_count(entry_portal_index);
            if route.lane_index >= lane_count {
                return Err(CorridorPopulationError::InvalidLaneIndex {
                    route_id: route.route_id,
                    portal_id: route.entry_portal_id,
                    lane_index: route.lane_index,
                    lane_count,
                });
            }
            if !portal_lanes.insert((entry_portal_index, route.lane_index)) {
                return Err(CorridorPopulationError::DuplicatePortalLane {
                    portal_id: route.entry_portal_id,
                    lane_index: route.lane_index,
                });
            }
            if !traffic
                .routes()
                .iter()
                .any(|candidate| candidate.id() == route.route_id)
            {
                return Err(CorridorPopulationError::UnknownTrafficRoute {
                    route_id: route.route_id,
                });
            }
            routes.push(TemporaryRoute {
                id: route.route_id,
                entry_portal_index,
                exit_portal_index,
                lane_index: route.lane_index,
                entry_spawn_slot_id: route.entry_spawn_slot_id,
            });
        }

        routes.sort_by_key(|route| (route.entry_portal_index, route.lane_index));
        let route_index_by_id = routes
            .iter()
            .enumerate()
            .map(|(index, route)| (route.id.clone(), index))
            .collect::<HashMap<_, _>>();

        for (portal_index, portal) in portal_entries.iter().enumerate() {
            let portal = portal
                .as_ref()
                .expect("all frozen portals were validated as present");
            let mut referenced = HashSet::with_capacity(portal.entry_route_ids.len());
            for route_id in &portal.entry_route_ids {
                if !referenced.insert(route_id.as_str()) {
                    return Err(CorridorPopulationError::DuplicatePortalRoute {
                        portal_id: portal.id.clone(),
                        route_id: route_id.clone(),
                    });
                }
                let Some(route_index) = route_index_by_id.get(route_id).copied() else {
                    return Err(CorridorPopulationError::PortalRouteSetMismatch {
                        portal_id: portal.id.clone(),
                    });
                };
                if routes[route_index].entry_portal_index != portal_index {
                    return Err(CorridorPopulationError::PortalRouteSetMismatch {
                        portal_id: portal.id.clone(),
                    });
                }
            }
            if routes
                .iter()
                .filter(|route| route.entry_portal_index == portal_index)
                .any(|route| !referenced.contains(route.id.as_str()))
            {
                return Err(CorridorPopulationError::PortalRouteSetMismatch {
                    portal_id: portal.id.clone(),
                });
            }
        }

        let mut slot_ids = HashSet::with_capacity(self.spawn_slots.len());
        let mut physical_locations = HashMap::with_capacity(self.spawn_slots.len());
        let mut spawn_slots = Vec::with_capacity(self.spawn_slots.len());
        for slot in self.spawn_slots {
            if !slot_ids.insert(slot.slot_id.clone()) {
                return Err(CorridorPopulationError::DuplicateSpawnSlot {
                    slot_id: slot.slot_id,
                });
            }
            let Some(route_index) = route_index_by_id.get(&slot.route_id).copied() else {
                return Err(CorridorPopulationError::UnknownSlotRoute {
                    slot_id: slot.slot_id,
                    route_id: slot.route_id,
                });
            };
            let route = &routes[route_index];
            let Some(portal_index) = portal_rank(&slot.portal_id) else {
                return Err(CorridorPopulationError::UnknownPortal {
                    portal_id: slot.portal_id,
                });
            };
            if portal_index != route.entry_portal_index {
                return Err(CorridorPopulationError::SlotPortalMismatch {
                    slot_id: slot.slot_id,
                    portal_id: slot.portal_id,
                    route_id: slot.route_id,
                });
            }
            let traffic_route = traffic
                .routes()
                .iter()
                .find(|candidate| candidate.id() == route.id)
                .expect("route existence was validated");
            let Some(expected_edge_id) = traffic_route.edge_ids().get(slot.route_edge_index) else {
                return Err(CorridorPopulationError::SlotRouteEdgeIndexOutOfRange {
                    slot_id: slot.slot_id,
                    route_edge_index: slot.route_edge_index,
                });
            };
            if expected_edge_id != &slot.edge_id {
                return Err(CorridorPopulationError::SlotEdgeMismatch {
                    slot_id: slot.slot_id,
                    expected_edge_id: expected_edge_id.clone(),
                    actual_edge_id: slot.edge_id,
                });
            }
            let edge_handle = traffic
                .lane_graph()
                .edge_handle(&slot.edge_id)
                .expect("validated route edge must exist");
            let edge_length = traffic
                .lane_graph()
                .edge_length(edge_handle)
                .expect("validated route edge length must exist")
                .value();
            if !slot.progress.is_finite() || slot.progress < 0.0 || slot.progress > edge_length {
                return Err(CorridorPopulationError::InvalidSlotProgress {
                    slot_id: slot.slot_id,
                    progress: slot.progress,
                    edge_length,
                });
            }
            let canonical_progress = if slot.progress == 0.0 {
                0.0
            } else {
                slot.progress
            };
            let physical_key = (slot.edge_id.clone(), canonical_progress.to_bits());
            if let Some(existing_slot_id) =
                physical_locations.insert(physical_key, slot.slot_id.clone())
            {
                return Err(CorridorPopulationError::DuplicateSpawnLocation {
                    slot_id: slot.slot_id,
                    existing_slot_id,
                });
            }
            let edge_progress = EdgeProgress::try_new(canonical_progress).map_err(|_| {
                CorridorPopulationError::InvalidSlotProgress {
                    slot_id: slot.slot_id.clone(),
                    progress: slot.progress,
                    edge_length,
                }
            })?;
            spawn_slots.push(NormalizedSpawnSlot {
                id: slot.slot_id,
                portal_index,
                route_index,
                route_edge_index: slot.route_edge_index,
                edge_id: slot.edge_id,
                edge_progress,
            });
        }

        if spawn_slots.len() < 200 {
            return Err(CorridorPopulationError::InsufficientSpawnSlots {
                required: 200,
                actual: spawn_slots.len(),
            });
        }
        spawn_slots.sort_by(|left, right| {
            let left_route = &routes[left.route_index];
            let right_route = &routes[right.route_index];
            (
                left.portal_index,
                left_route.lane_index,
                left.route_edge_index,
            )
                .cmp(&(
                    right.portal_index,
                    right_route.lane_index,
                    right.route_edge_index,
                ))
                .then_with(|| {
                    left.edge_progress
                        .value()
                        .total_cmp(&right.edge_progress.value())
                })
                .then_with(|| left.id.cmp(&right.id))
        });
        let slot_index_by_id = spawn_slots
            .iter()
            .enumerate()
            .map(|(index, slot)| (slot.id.as_str(), index))
            .collect::<HashMap<_, _>>();

        let normalized_routes = routes
            .into_iter()
            .map(|route| {
                let entry_spawn_slot_index = slot_index_by_id
                    .get(route.entry_spawn_slot_id.as_str())
                    .copied()
                    .filter(|slot_index| {
                        let slot = &spawn_slots[*slot_index];
                        slot.route_index
                            == *route_index_by_id
                                .get(&route.id)
                                .expect("route index must remain stable after sorting")
                            && slot.portal_index == route.entry_portal_index
                            && slot.route_edge_index == 0
                    })
                    .ok_or_else(|| CorridorPopulationError::InvalidEntrySpawnSlot {
                        route_id: route.id.clone(),
                        slot_id: route.entry_spawn_slot_id.clone(),
                    })?;
                Ok(NormalizedRoute {
                    id: route.id,
                    entry_portal_index: route.entry_portal_index,
                    exit_portal_index: route.exit_portal_index,
                    lane_index: route.lane_index,
                    entry_spawn_slot_index,
                })
            })
            .collect::<Result<Vec<_>, CorridorPopulationError>>()?;

        let portals = PORTAL_IDS
            .iter()
            .enumerate()
            .map(|(portal_index, portal_id)| NormalizedPortal {
                id: (*portal_id).to_owned(),
                route_indices: normalized_routes
                    .iter()
                    .enumerate()
                    .filter_map(|(route_index, route)| {
                        (route.entry_portal_index == portal_index).then_some(route_index)
                    })
                    .collect(),
            })
            .collect();

        Ok(NormalizedCorridorCatalog {
            portals,
            routes: normalized_routes,
            spawn_slots,
        })
    }
}

impl NormalizedCorridorCatalog {
    /// 返回规范 portal 顺序。
    pub fn portals(&self) -> &[NormalizedPortal] {
        &self.portals
    }

    /// 返回规范 lane route 顺序。
    pub fn routes(&self) -> &[NormalizedRoute] {
        &self.routes
    }

    /// 返回规范 spawn-slot 顺序。
    pub fn spawn_slots(&self) -> &[NormalizedSpawnSlot] {
        &self.spawn_slots
    }
}

impl NormalizedPortal {
    /// 返回 portal external ID。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回 portal-local lane routes 的 normalized indices。
    pub fn route_indices(&self) -> &[usize] {
        &self.route_indices
    }
}

impl NormalizedRoute {
    /// 返回 production route ID。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回 entry portal index。
    pub const fn entry_portal_index(&self) -> usize {
        self.entry_portal_index
    }

    /// 返回 exit portal index。
    pub const fn exit_portal_index(&self) -> usize {
        self.exit_portal_index
    }

    /// 返回 portal-local lane index。
    pub const fn lane_index(&self) -> usize {
        self.lane_index
    }

    /// 返回 replacement entry spawn slot index。
    pub const fn entry_spawn_slot_index(&self) -> usize {
        self.entry_spawn_slot_index
    }
}

impl NormalizedSpawnSlot {
    /// 返回 stable slot ID。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回 portal index。
    pub const fn portal_index(&self) -> usize {
        self.portal_index
    }

    /// 返回 route index。
    pub const fn route_index(&self) -> usize {
        self.route_index
    }

    /// 返回 route edge occurrence。
    pub const fn route_edge_index(&self) -> usize {
        self.route_edge_index
    }

    /// 返回 production edge ID。
    pub fn edge_id(&self) -> &str {
        &self.edge_id
    }

    /// 返回 validated edge progress。
    pub const fn edge_progress(&self) -> EdgeProgress {
        self.edge_progress
    }
}

#[derive(Clone, Debug)]
struct TemporaryRoute {
    id: String,
    entry_portal_index: usize,
    exit_portal_index: usize,
    lane_index: usize,
    entry_spawn_slot_id: String,
}

const fn expected_lane_count(portal_index: usize) -> usize {
    if portal_index < 2 { 3 } else { 2 }
}

fn portal_rank(portal_id: &str) -> Option<usize> {
    PORTAL_IDS
        .iter()
        .position(|candidate| *candidate == portal_id)
}
