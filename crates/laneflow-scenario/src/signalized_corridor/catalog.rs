use std::collections::{HashMap, HashSet};

use laneflow_core::{EdgeProgress, InitialTrafficData};
use serde::{Deserialize, Serialize};

use super::CorridorPopulationError;

/// 当前 scenario-local corridor catalog 版本。
pub const CATALOG_VERSION: &str = "0.2";

/// v0.8 portal 的规范顺序。
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
    /// portal external ID。
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

/// 已完成 catalog 0.2 semantic validation 和稳定排序的 runtime catalog。
#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedCorridorCatalog {
    pub(super) portals: Vec<NormalizedPortal>,
    pub(super) portal_lanes: Vec<NormalizedPortalLane>,
    pub(super) routes: Vec<NormalizedRoute>,
    pub(super) spawn_slots: Vec<NormalizedSpawnSlot>,
}

/// 规范化 portal。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedPortal {
    pub(super) id: String,
    pub(super) portal_lane_indices: Vec<usize>,
}

/// 规范化 portal lane。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedPortalLane {
    pub(super) portal_index: usize,
    pub(super) lane_index: usize,
    pub(super) entry_spawn_slot_index: usize,
    pub(super) route_choices: Vec<NormalizedWeightedRouteChoice>,
    pub(super) total_positive_weight: u64,
}

/// 规范化 weighted route choice。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NormalizedWeightedRouteChoice {
    pub(super) route_index: usize,
    pub(super) weight: u64,
}

/// 规范化 lane route。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedRoute {
    pub(super) id: String,
    pub(super) entry_portal_index: usize,
    pub(super) exit_portal_index: usize,
    pub(super) lane_index: usize,
    pub(super) portal_lane_index: usize,
    pub(super) entry_spawn_slot_index: usize,
    pub(super) entry_route_edge_index: usize,
}

/// 规范化 spawn slot。
#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedSpawnSlot {
    pub(super) id: String,
    pub(super) portal_index: usize,
    pub(super) portal_lane_index: usize,
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

        let traffic_route_rank = traffic
            .routes()
            .enumerate()
            .map(|(index, route)| (route.id().to_owned(), index))
            .collect::<HashMap<_, _>>();
        let mut route_entries = HashMap::with_capacity(self.routes.len());
        let mut route_entry_order = Vec::with_capacity(self.routes.len());
        for route in self.routes {
            if route_entries.contains_key(&route.route_id) {
                return Err(CorridorPopulationError::DuplicateRoute {
                    route_id: route.route_id,
                });
            }
            if portal_rank(&route.exit_portal_id).is_none() {
                return Err(CorridorPopulationError::UnknownPortal {
                    portal_id: route.exit_portal_id,
                });
            }
            if !traffic_route_rank.contains_key(&route.route_id) {
                return Err(CorridorPopulationError::UnknownTrafficRoute {
                    route_id: route.route_id,
                });
            }
            route_entry_order.push(route.route_id.clone());
            route_entries.insert(route.route_id.clone(), route);
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
            portal_entries[portal_index] = Some(portal);
        }
        for (portal_index, portal) in portal_entries.iter().enumerate() {
            if portal.is_none() {
                return Err(CorridorPopulationError::MissingPortal {
                    portal_id: PORTAL_IDS[portal_index],
                });
            }
        }

        let mut referenced_routes = HashSet::with_capacity(route_entries.len());
        let mut temporary_lanes = Vec::new();
        let mut temporary_routes = Vec::with_capacity(route_entries.len());
        for (portal_index, portal_entry) in portal_entries.iter_mut().enumerate() {
            let mut portal = portal_entry
                .take()
                .expect("all frozen portals were validated as present");
            let expected = expected_lane_count(portal_index);
            if portal.lanes.len() != expected {
                return Err(CorridorPopulationError::InvalidPortalLaneCount {
                    portal_id: portal.id,
                    expected,
                    actual: portal.lanes.len(),
                });
            }
            portal.lanes.sort_by_key(|lane| lane.lane_index);
            let mut lane_indices = HashSet::with_capacity(portal.lanes.len());
            for mut lane in portal.lanes {
                if lane.lane_index >= expected {
                    return Err(CorridorPopulationError::InvalidLaneIndex {
                        portal_id: portal.id,
                        lane_index: lane.lane_index,
                        lane_count: expected,
                    });
                }
                if !lane_indices.insert(lane.lane_index) {
                    return Err(CorridorPopulationError::DuplicatePortalLane {
                        portal_id: portal.id,
                        lane_index: lane.lane_index,
                    });
                }
                if lane.route_choices.is_empty() {
                    return Err(CorridorPopulationError::EmptyRouteChoices {
                        portal_id: portal.id,
                        lane_index: lane.lane_index,
                    });
                }
                lane.route_choices.sort_by_key(|choice| {
                    traffic_route_rank
                        .get(&choice.route_id)
                        .copied()
                        .unwrap_or(usize::MAX)
                });

                let portal_lane_index = temporary_lanes.len();
                let mut route_choices = Vec::with_capacity(lane.route_choices.len());
                let mut total_positive_weight = 0_u64;
                for choice in lane.route_choices {
                    if choice.weight == 0 {
                        return Err(CorridorPopulationError::InvalidRouteChoiceWeight {
                            portal_id: portal.id,
                            lane_index: lane.lane_index,
                            route_id: choice.route_id,
                            weight: choice.weight,
                        });
                    }
                    total_positive_weight = total_positive_weight
                        .checked_add(choice.weight)
                        .ok_or_else(|| CorridorPopulationError::RouteChoiceWeightOverflow {
                            portal_id: portal.id.clone(),
                            lane_index: lane.lane_index,
                        })?;
                    if !referenced_routes.insert(choice.route_id.clone()) {
                        return Err(CorridorPopulationError::DuplicatePortalRoute {
                            portal_id: portal.id,
                            route_id: choice.route_id,
                        });
                    }
                    let route_entry = route_entries.get(&choice.route_id).ok_or_else(|| {
                        CorridorPopulationError::UnknownRouteChoice {
                            portal_id: portal.id.clone(),
                            lane_index: lane.lane_index,
                            route_id: choice.route_id.clone(),
                        }
                    })?;
                    let exit_portal_index = portal_rank(&route_entry.exit_portal_id)
                        .expect("route exit portal was validated above");
                    if portal_index == exit_portal_index {
                        return Err(CorridorPopulationError::InvalidRoutePortals {
                            route_id: choice.route_id,
                            entry_portal_id: portal.id,
                            exit_portal_id: route_entry.exit_portal_id.clone(),
                        });
                    }
                    let route_index = temporary_routes.len();
                    temporary_routes.push(TemporaryRoute {
                        id: choice.route_id,
                        entry_portal_index: portal_index,
                        exit_portal_index,
                        lane_index: lane.lane_index,
                        portal_lane_index,
                    });
                    route_choices.push(NormalizedWeightedRouteChoice {
                        route_index,
                        weight: choice.weight,
                    });
                }
                temporary_lanes.push(TemporaryPortalLane {
                    portal_index,
                    lane_index: lane.lane_index,
                    entry_spawn_slot_id: lane.entry_spawn_slot_id,
                    route_choices,
                    total_positive_weight,
                });
            }
        }
        if let Some(route_id) = route_entry_order
            .into_iter()
            .find(|route_id| !referenced_routes.contains(route_id))
        {
            return Err(CorridorPopulationError::UnreferencedRoute { route_id });
        }

        let portal_lane_index_by_key = temporary_lanes
            .iter()
            .enumerate()
            .map(|(index, lane)| ((lane.portal_index, lane.lane_index), index))
            .collect::<HashMap<_, _>>();

        let mut slot_ids = HashSet::with_capacity(self.spawn_slots.len());
        let mut physical_locations = HashMap::with_capacity(self.spawn_slots.len());
        let mut spawn_slots = Vec::with_capacity(self.spawn_slots.len());
        for slot in self.spawn_slots {
            if !slot_ids.insert(slot.slot_id.clone()) {
                return Err(CorridorPopulationError::DuplicateSpawnSlot {
                    slot_id: slot.slot_id,
                });
            }
            let Some(portal_index) = portal_rank(&slot.portal_id) else {
                return Err(CorridorPopulationError::UnknownPortal {
                    portal_id: slot.portal_id,
                });
            };
            let Some(portal_lane_index) = portal_lane_index_by_key
                .get(&(portal_index, slot.lane_index))
                .copied()
            else {
                return Err(CorridorPopulationError::SlotLaneMismatch {
                    slot_id: slot.slot_id,
                    portal_id: slot.portal_id,
                    lane_index: slot.lane_index,
                });
            };
            let lane = &temporary_lanes[portal_lane_index];
            let mut shared_route_edge_index = None;
            for choice in &lane.route_choices {
                let route = &temporary_routes[choice.route_index];
                let traffic_route = traffic
                    .routes()
                    .find(|candidate| candidate.id() == route.id)
                    .expect("route existence was validated");
                let mut occurrences = traffic_route
                    .edge_ids()
                    .iter()
                    .enumerate()
                    .filter_map(|(index, edge_id)| (edge_id == &slot.edge_id).then_some(index));
                let route_edge_index = occurrences.next().ok_or_else(|| {
                    CorridorPopulationError::SlotEdgeMissingFromRoute {
                        slot_id: slot.slot_id.clone(),
                        route_id: route.id.clone(),
                        edge_id: slot.edge_id.clone(),
                    }
                })?;
                if occurrences.next().is_some() {
                    return Err(CorridorPopulationError::SlotEdgeOccurrenceAmbiguous {
                        slot_id: slot.slot_id,
                        route_id: route.id.clone(),
                        edge_id: slot.edge_id,
                    });
                }
                if let Some(expected) = shared_route_edge_index {
                    if route_edge_index != expected {
                        return Err(CorridorPopulationError::SlotRouteEdgeIndexMismatch {
                            slot_id: slot.slot_id,
                            route_id: route.id.clone(),
                            expected,
                            actual: route_edge_index,
                        });
                    }
                } else {
                    shared_route_edge_index = Some(route_edge_index);
                }
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
                portal_lane_index,
                route_edge_index: shared_route_edge_index
                    .expect("portal lanes always have at least one route choice"),
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
            let left_lane = &temporary_lanes[left.portal_lane_index];
            let right_lane = &temporary_lanes[right.portal_lane_index];
            (
                left.portal_index,
                left_lane.lane_index,
                left.route_edge_index,
            )
                .cmp(&(
                    right.portal_index,
                    right_lane.lane_index,
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

        let portal_lanes = temporary_lanes
            .into_iter()
            .map(|lane| {
                let entry_spawn_slot_index = slot_index_by_id
                    .get(lane.entry_spawn_slot_id.as_str())
                    .copied()
                    .filter(|slot_index| {
                        let slot = &spawn_slots[*slot_index];
                        slot.portal_lane_index
                            == *portal_lane_index_by_key
                                .get(&(lane.portal_index, lane.lane_index))
                                .expect("portal lane index must remain stable")
                            && slot.route_edge_index == 0
                    })
                    .ok_or_else(|| CorridorPopulationError::InvalidEntrySpawnSlot {
                        portal_id: PORTAL_IDS[lane.portal_index],
                        lane_index: lane.lane_index,
                        slot_id: lane.entry_spawn_slot_id.clone(),
                    })?;
                Ok(NormalizedPortalLane {
                    portal_index: lane.portal_index,
                    lane_index: lane.lane_index,
                    entry_spawn_slot_index,
                    route_choices: lane.route_choices,
                    total_positive_weight: lane.total_positive_weight,
                })
            })
            .collect::<Result<Vec<_>, CorridorPopulationError>>()?;
        let normalized_routes = temporary_routes
            .into_iter()
            .map(|route| {
                let lane = &portal_lanes[route.portal_lane_index];
                let entry = &spawn_slots[lane.entry_spawn_slot_index];
                NormalizedRoute {
                    id: route.id,
                    entry_portal_index: route.entry_portal_index,
                    exit_portal_index: route.exit_portal_index,
                    lane_index: route.lane_index,
                    portal_lane_index: route.portal_lane_index,
                    entry_spawn_slot_index: lane.entry_spawn_slot_index,
                    entry_route_edge_index: entry.route_edge_index,
                }
            })
            .collect::<Vec<_>>();

        let portals = PORTAL_IDS
            .iter()
            .enumerate()
            .map(|(portal_index, portal_id)| NormalizedPortal {
                id: (*portal_id).to_owned(),
                portal_lane_indices: portal_lanes
                    .iter()
                    .enumerate()
                    .filter_map(|(portal_lane_index, lane)| {
                        (lane.portal_index == portal_index).then_some(portal_lane_index)
                    })
                    .collect(),
            })
            .collect();

        Ok(NormalizedCorridorCatalog {
            portals,
            portal_lanes,
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

    /// 返回规范 portal lane 顺序。
    pub fn portal_lanes(&self) -> &[NormalizedPortalLane] {
        &self.portal_lanes
    }

    /// 返回规范 route 顺序。
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

    /// 返回 portal-local lanes 的 normalized indices。
    pub fn portal_lane_indices(&self) -> &[usize] {
        &self.portal_lane_indices
    }
}

impl NormalizedPortalLane {
    /// 返回 owner portal index。
    pub const fn portal_index(&self) -> usize {
        self.portal_index
    }

    /// 返回 portal-local lane index。
    pub const fn lane_index(&self) -> usize {
        self.lane_index
    }

    /// 返回 replacement 使用的共享 entry spawn slot index。
    pub const fn entry_spawn_slot_index(&self) -> usize {
        self.entry_spawn_slot_index
    }

    /// 返回 cumulative-selection 顺序中的 weighted route choices。
    pub fn route_choices(&self) -> &[NormalizedWeightedRouteChoice] {
        &self.route_choices
    }

    /// 返回全部正整数 raw weight 的和。
    pub const fn total_positive_weight(&self) -> u64 {
        self.total_positive_weight
    }
}

impl NormalizedWeightedRouteChoice {
    /// 返回 normalized route index。
    pub const fn route_index(self) -> usize {
        self.route_index
    }

    /// 返回 lane-local raw weight。
    pub const fn weight(self) -> u64 {
        self.weight
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

    /// 返回 owner portal lane index。
    pub const fn portal_lane_index(&self) -> usize {
        self.portal_lane_index
    }

    /// 返回 replacement entry spawn slot index。
    pub const fn entry_spawn_slot_index(&self) -> usize {
        self.entry_spawn_slot_index
    }

    /// 返回 replacement entry slot 在 route 中的 edge occurrence。
    pub const fn entry_route_edge_index(&self) -> usize {
        self.entry_route_edge_index
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

    /// 返回 owner portal lane index。
    pub const fn portal_lane_index(&self) -> usize {
        self.portal_lane_index
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
    portal_lane_index: usize,
}

#[derive(Clone, Debug)]
struct TemporaryPortalLane {
    portal_index: usize,
    lane_index: usize,
    entry_spawn_slot_id: String,
    route_choices: Vec<NormalizedWeightedRouteChoice>,
    total_positive_weight: u64,
}

const fn expected_lane_count(portal_index: usize) -> usize {
    if portal_index < 2 { 3 } else { 2 }
}

fn portal_rank(portal_id: &str) -> Option<usize> {
    PORTAL_IDS
        .iter()
        .position(|candidate| *candidate == portal_id)
}
