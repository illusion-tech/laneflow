use std::collections::BTreeMap;
use std::fmt;

use laneflow_compiler::{CanonicalIdentityFieldView, ValidatedCanonicalLir};
use laneflow_static_contract::{
    FieldTag, LaneEdgeOrdinal, StaticRouteOrdinal, VehicleProfileOrdinal,
};

use super::{CorridorCatalog, SpawnSlotCatalogEntry};

/// prepare 阶段把 catalog 0.2 字符串绑到本共享路网修订的类型化序号。
#[derive(Clone, Debug, PartialEq)]
pub struct BoundCorridorCatalog {
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BindError {
    UnknownRoute(String),
    UnknownEdge(String),
}

impl fmt::Display for BindError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownRoute(id) => {
                write!(formatter, "catalog route_id {id:?} is not in this network")
            }
            Self::UnknownEdge(id) => {
                write!(formatter, "catalog edge_id {id:?} is not in this network")
            }
        }
    }
}

impl std::error::Error for BindError {}

/// 用 Canonical LIR 身份字段把 catalog 字符串绑到本编制的类型化序号。
///
/// 热路径不得再查这些字符串。调用方随后用 `TrafficWorld::static_route` /
/// `spawn_vehicle` 消费序号。
pub fn bind(
    catalog: &CorridorCatalog,
    lir: &ValidatedCanonicalLir,
) -> Result<BoundCorridorCatalog, BindError> {
    let mut routes = BTreeMap::new();
    for route in lir.static_routes() {
        let key = ascii_field(route.identity_fields(), FieldTag::RouteKey);
        routes.insert(key, route.ordinal());
    }
    let mut edges = BTreeMap::new();
    for edge in lir.lane_edges() {
        let key = ascii_field(edge.identity_fields(), FieldTag::LaneEdgeKey);
        edges.insert(key, edge.ordinal());
    }
    let mut profiles = BTreeMap::new();
    for profile in lir.vehicle_profiles() {
        let key = ascii_field(profile.identity_fields(), FieldTag::VehicleProfileKey);
        profiles.insert(key, profile.ordinal());
    }
    for route in &catalog.routes {
        if !routes.contains_key(&route.route_id) {
            return Err(BindError::UnknownRoute(route.route_id.clone()));
        }
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
    let spawn_slots = catalog
        .spawn_slots
        .iter()
        .map(|slot| bind_slot(slot, &edges))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BoundCorridorCatalog {
        routes,
        edges,
        profiles,
        spawn_slots,
    })
}

fn bind_slot(
    slot: &SpawnSlotCatalogEntry,
    edges: &BTreeMap<String, LaneEdgeOrdinal>,
) -> Result<BoundSpawnSlot, BindError> {
    let edge = *edges
        .get(&slot.edge_id)
        .ok_or_else(|| BindError::UnknownEdge(slot.edge_id.clone()))?;
    Ok(BoundSpawnSlot {
        slot_id: slot.slot_id.clone(),
        portal_id: slot.portal_id.clone(),
        lane_index: slot.lane_index,
        edge,
        progress: slot.progress,
    })
}

fn ascii_field<'a>(
    fields: impl IntoIterator<Item = CanonicalIdentityFieldView<'a>>,
    tag: FieldTag,
) -> String {
    let field = fields
        .into_iter()
        .find(|field| field.tag() == tag)
        .expect("required identity field");
    std::str::from_utf8(field.value_bytes())
        .expect("identity field is ASCII")
        .to_owned()
}
