use std::collections::{BTreeMap, HashMap};
use std::fmt;

use laneflow_compiler::{CanonicalIdentityViolation, CompileLimits, derive_canonical_stable_id_v1};
use laneflow_runtime::{RouteError, RouteHandle, RouteRegisterInput, TrafficWorld};
use laneflow_static_contract::{
    EntityKind, LaneEdgeId, LaneEdgeOrdinal, NetworkRevisionId, VehicleProfileId,
    VehicleProfileOrdinal,
};
use laneflow_static_network::SharedNetworkRevision;

use super::{
    AUTHORING_NAMESPACE, CatalogError, CorridorCatalog, PASSENGER_CAR_PROFILE_KEY, PORTAL_IDS,
    SHUTTLE_BUS_PROFILE_KEY, SpawnSlotCatalogEntry, validate,
};

/// prepare 阶段把 catalog 0.3 字符串绑到本共享路网修订的类型化序号。
#[derive(Clone, Debug, PartialEq)]
pub struct BoundCorridorCatalog {
    pub network_revision: NetworkRevisionId,
    pub routes: BTreeMap<String, Box<[LaneEdgeOrdinal]>>,
    pub edges: BTreeMap<String, LaneEdgeOrdinal>,
    pub profiles: BTreeMap<String, VehicleProfileOrdinal>,
    pub spawn_slots: Vec<BoundSpawnSlot>,
    /// `catalog.routes` 顺序的边序号序列与出口 portal 下标。
    pub route_exits: Vec<BoundRouteExit>,
    /// 按 portal、lane index 展开的热路径入口车道。
    pub portal_lanes: Vec<BoundPortalLane>,
    /// 每个 portal 在 `portal_lanes` 中的下标。
    pub portal_lane_indices: [Vec<usize>; 6],
}

/// catalog route 绑到本修订边序号序列与出口 portal。
///
/// 入口进度按 portal lane 的 `entry_slot_index` 取，不在路线上缓存单一入口槽。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundRouteExit {
    pub edges: Box<[LaneEdgeOrdinal]>,
    pub exit_portal_index: u8,
}

/// portal lane 的加权 RouteChoice；`route_index` 指向 `route_exits`。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoundRouteChoice {
    pub route_index: usize,
    pub weight: u64,
}

/// 已绑定的 portal lane：共享 entry slot 与加权路线。
#[derive(Clone, Debug, PartialEq)]
pub struct BoundPortalLane {
    pub portal_index: u8,
    pub lane_index: usize,
    pub entry_slot_index: usize,
    pub choices: Vec<BoundRouteChoice>,
    pub total_positive_weight: u64,
}

/// 已绑定到类型化序号的物理 spawn slot。
#[derive(Clone, Debug, PartialEq)]
pub struct BoundSpawnSlot {
    pub slot_id: String,
    pub portal_id: String,
    pub portal_index: u8,
    pub lane_index: usize,
    pub portal_lane_index: usize,
    pub edge: LaneEdgeOrdinal,
    pub progress_mm: u32,
    pub entry_edges: Box<[LaneEdgeOrdinal]>,
    /// `route_exits` 下标；spawn 用 `install_routes` 返回的对应句柄。
    pub route_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BindError {
    Catalog(CatalogError),
    Identity(CanonicalIdentityViolation),
    UnknownRoute(String),
    UnknownEdge(String),
    UnknownProfile(String),
    SlotEdgeNotEntry { slot_id: String, route_id: String },
    InvalidProgress { slot_id: String },
    RouteRegister(RouteError),
}

impl fmt::Display for BindError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Catalog(error) => write!(formatter, "{error}"),
            Self::Identity(error) => {
                write!(formatter, "catalog identity is not Identity v1: {error:?}")
            }
            Self::UnknownRoute(id) => {
                write!(
                    formatter,
                    "catalog route_id {id:?} is not in this network revision"
                )
            }
            Self::UnknownEdge(id) => {
                write!(
                    formatter,
                    "catalog edge_id {id:?} is not in this network revision"
                )
            }
            Self::UnknownProfile(id) => {
                write!(
                    formatter,
                    "vehicle profile {id:?} is not in this network revision"
                )
            }
            Self::SlotEdgeNotEntry { slot_id, route_id } => write!(
                formatter,
                "slot {slot_id:?} edge is not the entry edge of route {route_id:?}"
            ),
            Self::InvalidProgress { slot_id } => {
                write!(
                    formatter,
                    "slot {slot_id:?} progress is outside the bound edge"
                )
            }
            Self::RouteRegister(error) => write!(formatter, "register_route failed: {error}"),
        }
    }
}

impl std::error::Error for BindError {}

impl From<CatalogError> for BindError {
    fn from(error: CatalogError) -> Self {
        Self::Catalog(error)
    }
}

/// 用 Identity v1 把 catalog 字符串绑到已安装共享路网修订的类型化序号。
///
/// 热路径不得再查这些字符串。调用方随后 `install_routes` 再 `spawn_vehicle`。
pub fn bind(
    catalog: &CorridorCatalog,
    revision: &SharedNetworkRevision,
) -> Result<BoundCorridorCatalog, BindError> {
    validate(catalog)?;
    let limits = CompileLimits::p100_initial_v1();
    let mut routes = BTreeMap::new();
    for route in &catalog.routes {
        let mut edges = Vec::with_capacity(route.edge_ids.len());
        for edge_id in &route.edge_ids {
            edges.push(resolve_edge(revision, edge_id, &limits)?);
        }
        routes.insert(route.route_id.clone(), edges.into_boxed_slice());
    }
    for portal in &catalog.portals {
        for lane in &portal.lanes {
            for choice in &lane.route_choices {
                if !routes.contains_key(&choice.route_id) {
                    return Err(BindError::UnknownRoute(choice.route_id.clone()));
                }
            }
        }
    }

    let mut edges = BTreeMap::new();
    for slot in &catalog.spawn_slots {
        if edges.contains_key(&slot.edge_id) {
            continue;
        }
        let ordinal = resolve_edge(revision, &slot.edge_id, &limits)?;
        edges.insert(slot.edge_id.clone(), ordinal);
    }

    let mut profiles = BTreeMap::new();
    for key in [PASSENGER_CAR_PROFILE_KEY, SHUTTLE_BUS_PROFILE_KEY] {
        let ordinal = resolve_profile(revision, key, &limits)?;
        profiles.insert(key.to_owned(), ordinal);
    }

    let mut spawn_slots = catalog
        .spawn_slots
        .iter()
        .map(|slot| bind_slot(catalog, slot, revision, &routes, &edges))
        .collect::<Result<Vec<_>, _>>()?;
    spawn_slots.sort_by(|left, right| {
        portal_rank(&left.portal_id)
            .cmp(&portal_rank(&right.portal_id))
            .then(left.lane_index.cmp(&right.lane_index))
            .then(left.progress_mm.cmp(&right.progress_mm))
            .then(left.slot_id.cmp(&right.slot_id))
    });

    let mut slot_index_by_id = HashMap::new();
    for (index, slot) in spawn_slots.iter().enumerate() {
        slot_index_by_id.insert(slot.slot_id.as_str(), index);
    }
    let mut route_index_by_id = HashMap::new();
    let mut route_exits = Vec::with_capacity(catalog.routes.len());
    for route in &catalog.routes {
        let edges = routes
            .get(&route.route_id)
            .ok_or_else(|| BindError::UnknownRoute(route.route_id.clone()))?
            .clone();
        let exit_portal_index =
            u8::try_from(portal_rank(&route.exit_portal_id)).expect("portal count fits u8");
        route_index_by_id.insert(route.route_id.as_str(), route_exits.len());
        route_exits.push(BoundRouteExit {
            edges,
            exit_portal_index,
        });
    }
    let mut portal_lanes = Vec::new();
    let mut portal_lane_indices = [(); PORTAL_IDS.len()].map(|_| Vec::new());
    for (portal_index, portal) in catalog.portals.iter().enumerate() {
        for lane in &portal.lanes {
            let entry_slot_index = *slot_index_by_id
                .get(lane.entry_spawn_slot_id.as_str())
                .expect("validate checked entry slot");
            let mut choices = Vec::with_capacity(lane.route_choices.len());
            let mut total_positive_weight = 0_u64;
            for choice in &lane.route_choices {
                let route_index = *route_index_by_id
                    .get(choice.route_id.as_str())
                    .ok_or_else(|| BindError::UnknownRoute(choice.route_id.clone()))?;
                choices.push(BoundRouteChoice {
                    route_index,
                    weight: choice.weight,
                });
                total_positive_weight += choice.weight;
            }
            let lane_slot = portal_lanes.len();
            portal_lane_indices[portal_index].push(lane_slot);
            portal_lanes.push(BoundPortalLane {
                portal_index: u8::try_from(portal_index).expect("portal count fits u8"),
                lane_index: lane.lane_index,
                entry_slot_index,
                choices,
                total_positive_weight,
            });
        }
    }
    drop(slot_index_by_id);
    drop(route_index_by_id);
    for slot in &mut spawn_slots {
        slot.portal_lane_index = portal_lanes
            .iter()
            .position(|lane| {
                lane.portal_index == slot.portal_index && lane.lane_index == slot.lane_index
            })
            .expect("validate checked portal lane");
        let lane = &portal_lanes[slot.portal_lane_index];
        slot.route_index = lane
            .choices
            .iter()
            .map(|choice| choice.route_index)
            .min_by_key(|&index| route_exits[index].edges.as_ref())
            .expect("portal lane has route choices");
        slot.entry_edges = route_exits[slot.route_index].edges.clone();
    }

    Ok(BoundCorridorCatalog {
        network_revision: revision.network_revision(),
        routes,
        edges,
        profiles,
        spawn_slots,
        route_exits,
        portal_lanes,
        portal_lane_indices,
    })
}

impl BoundCorridorCatalog {
    /// 对本世界每条 catalog 路线恰好 `register_route` 一次。失败撤回本次注册的句柄。
    pub fn install_routes(&self, world: &mut TrafficWorld) -> Result<Vec<RouteHandle>, BindError> {
        if world.revision().network_revision() != self.network_revision {
            return Err(BindError::UnknownRoute(
                "TrafficWorld 修订与 catalog bind 不一致".to_owned(),
            ));
        }
        let needed = u32::try_from(self.route_exits.len()).expect("route count fits u32");
        if world.config().route_capacity() < needed {
            return Err(BindError::RouteRegister(RouteError::CapacityExceeded));
        }
        let mut handles = Vec::with_capacity(self.route_exits.len());
        for exit in &self.route_exits {
            match world.register_route(RouteRegisterInput::new(exit.edges.to_vec())) {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    for handle in handles.iter().rev().copied() {
                        let _ = world.remove_route(handle);
                    }
                    return Err(BindError::RouteRegister(error));
                }
            }
        }
        Ok(handles)
    }
}

fn bind_slot(
    catalog: &CorridorCatalog,
    slot: &SpawnSlotCatalogEntry,
    revision: &SharedNetworkRevision,
    routes: &BTreeMap<String, Box<[LaneEdgeOrdinal]>>,
    edges: &BTreeMap<String, LaneEdgeOrdinal>,
) -> Result<BoundSpawnSlot, BindError> {
    let edge = *edges
        .get(&slot.edge_id)
        .ok_or_else(|| BindError::UnknownEdge(slot.edge_id.clone()))?;
    let portal = catalog
        .portals
        .iter()
        .find(|portal| portal.id == slot.portal_id)
        .expect("validate checked portal");
    let lane = portal
        .lanes
        .iter()
        .find(|lane| lane.lane_index == slot.lane_index)
        .expect("validate checked portal lane");
    for choice in &lane.route_choices {
        let route_edges = routes
            .get(&choice.route_id)
            .ok_or_else(|| BindError::UnknownRoute(choice.route_id.clone()))?;
        if route_edges.first().copied() != Some(edge) {
            return Err(BindError::SlotEdgeNotEntry {
                slot_id: slot.slot_id.clone(),
                route_id: choice.route_id.clone(),
            });
        }
    }
    let length = *revision
        .traffic()
        .lane_lengths_millimetres()
        .get(edge.index())
        .ok_or_else(|| BindError::UnknownEdge(slot.edge_id.clone()))?;
    if !slot.progress.is_finite() || slot.progress < 0.0 {
        return Err(BindError::InvalidProgress {
            slot_id: slot.slot_id.clone(),
        });
    }
    let progress_mm = (slot.progress * 1_000.0).round_ties_even();
    if progress_mm < 0.0 || progress_mm > f64::from(length) {
        return Err(BindError::InvalidProgress {
            slot_id: slot.slot_id.clone(),
        });
    }
    let progress_mm = progress_mm as u32;
    Ok(BoundSpawnSlot {
        slot_id: slot.slot_id.clone(),
        portal_id: slot.portal_id.clone(),
        portal_index: u8::try_from(portal_rank(&slot.portal_id)).expect("portal count fits u8"),
        lane_index: slot.lane_index,
        portal_lane_index: 0,
        edge,
        progress_mm,
        entry_edges: Box::from([]),
        route_index: 0,
    })
}

fn portal_rank(portal_id: &str) -> usize {
    PORTAL_IDS
        .iter()
        .position(|id| *id == portal_id)
        .expect("validate checked portal")
}

fn resolve_edge(
    revision: &SharedNetworkRevision,
    key: &str,
    limits: &CompileLimits,
) -> Result<LaneEdgeOrdinal, BindError> {
    let stable =
        derive_canonical_stable_id_v1(EntityKind::LaneEdge, AUTHORING_NAMESPACE, key, limits)
            .map_err(BindError::Identity)?;
    revision
        .identity()
        .ordinal(LaneEdgeId::from_untyped(stable))
        .ok_or_else(|| BindError::UnknownEdge(key.to_owned()))
}

fn resolve_profile(
    revision: &SharedNetworkRevision,
    key: &str,
    limits: &CompileLimits,
) -> Result<VehicleProfileOrdinal, BindError> {
    let stable =
        derive_canonical_stable_id_v1(EntityKind::VehicleProfile, AUTHORING_NAMESPACE, key, limits)
            .map_err(BindError::Identity)?;
    revision
        .identity()
        .ordinal(VehicleProfileId::from_untyped(stable))
        .ok_or_else(|| BindError::UnknownProfile(key.to_owned()))
}
