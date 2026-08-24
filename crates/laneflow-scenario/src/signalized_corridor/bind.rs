use std::collections::BTreeMap;
use std::fmt;

use laneflow_compiler::{CanonicalIdentityViolation, CompileLimits, derive_canonical_stable_id_v1};
use laneflow_static_contract::{
    EntityKind, LaneEdgeId, LaneEdgeOrdinal, NetworkRevisionId, StaticRouteId, StaticRouteOrdinal,
    VehicleProfileId, VehicleProfileOrdinal,
};
use laneflow_static_network::SharedNetworkRevision;

use super::{
    AUTHORING_NAMESPACE, CatalogError, CorridorCatalog, PASSENGER_CAR_PROFILE_KEY,
    SHUTTLE_BUS_PROFILE_KEY, SpawnSlotCatalogEntry, validate,
};

/// prepare 阶段把 catalog 0.2 字符串绑到本共享路网修订的类型化序号。
#[derive(Clone, Debug, PartialEq)]
pub struct BoundCorridorCatalog {
    pub network_revision: NetworkRevisionId,
    pub routes: BTreeMap<String, StaticRouteOrdinal>,
    pub edges: BTreeMap<String, LaneEdgeOrdinal>,
    pub profiles: BTreeMap<String, VehicleProfileOrdinal>,
    pub spawn_slots: Vec<BoundSpawnSlot>,
}

/// 已绑定到类型化序号的物理 spawn slot。
#[derive(Clone, Debug, PartialEq)]
pub struct BoundSpawnSlot {
    pub slot_id: String,
    pub portal_id: String,
    pub lane_index: usize,
    pub edge: LaneEdgeOrdinal,
    pub progress: f64,
    pub entry_route: StaticRouteOrdinal,
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
/// 热路径不得再查这些字符串。调用方随后用 `TrafficWorld::static_route` /
/// `spawn_vehicle` 消费序号。
pub fn bind(
    catalog: &CorridorCatalog,
    revision: &SharedNetworkRevision,
) -> Result<BoundCorridorCatalog, BindError> {
    validate(catalog)?;
    let limits = CompileLimits::p100_initial_v1();
    let mut routes = BTreeMap::new();
    for route in &catalog.routes {
        let ordinal = resolve_route(revision, &route.route_id, &limits)?;
        routes.insert(route.route_id.clone(), ordinal);
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

    let spawn_slots = catalog
        .spawn_slots
        .iter()
        .map(|slot| bind_slot(catalog, slot, revision, &routes, &edges))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BoundCorridorCatalog {
        network_revision: revision.network_revision(),
        routes,
        edges,
        profiles,
        spawn_slots,
    })
}

fn bind_slot(
    catalog: &CorridorCatalog,
    slot: &SpawnSlotCatalogEntry,
    revision: &SharedNetworkRevision,
    routes: &BTreeMap<String, StaticRouteOrdinal>,
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
        let route = *routes
            .get(&choice.route_id)
            .ok_or_else(|| BindError::UnknownRoute(choice.route_id.clone()))?;
        let route_edges = revision
            .traffic()
            .relations()
            .static_route_edges(route)
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
        .lane_lengths_meters()
        .get(edge.index())
        .ok_or_else(|| BindError::UnknownEdge(slot.edge_id.clone()))?;
    if !slot.progress.is_finite() || slot.progress <= 0.0 || slot.progress >= length {
        return Err(BindError::InvalidProgress {
            slot_id: slot.slot_id.clone(),
        });
    }
    let entry_route = *routes
        .get(&lane.route_choices[0].route_id)
        .ok_or_else(|| BindError::UnknownRoute(lane.route_choices[0].route_id.clone()))?;
    Ok(BoundSpawnSlot {
        slot_id: slot.slot_id.clone(),
        portal_id: slot.portal_id.clone(),
        lane_index: slot.lane_index,
        edge,
        progress: slot.progress,
        entry_route,
    })
}

fn resolve_route(
    revision: &SharedNetworkRevision,
    key: &str,
    limits: &CompileLimits,
) -> Result<StaticRouteOrdinal, BindError> {
    let stable =
        derive_canonical_stable_id_v1(EntityKind::StaticRoute, AUTHORING_NAMESPACE, key, limits)
            .map_err(BindError::Identity)?;
    revision
        .identity()
        .ordinal(StaticRouteId::from_untyped(stable))
        .ok_or_else(|| BindError::UnknownRoute(key.to_owned()))
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
