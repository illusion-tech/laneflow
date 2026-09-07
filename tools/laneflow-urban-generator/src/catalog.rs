use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use laneflow_compiler::{CompileLimits, derive_canonical_stable_id_v1};
use laneflow_runtime::{
    CommittedNetworkSource, PolicyPin, PublishedLfcaReference, RouteRegisterInput, TrafficWorld,
    WorldConfig, WorldPolicySelection,
};
use laneflow_static_contract::{
    AccessEffect, EntityKind, LaneEdgeId, LaneEdgeOrdinal, ParticipantClassOrdinal,
    RightOfWayPolicySetId,
};
use laneflow_static_network::{AccessCell, SharedNetworkRevision};
use serde::{Deserialize, Serialize};

use crate::source::{self, Movement, ParkingAnchor, SignalProgram};
use crate::{GeneratedSource, Result, UrbanConfig, validation};

#[derive(Debug, Serialize, Deserialize)]
pub struct Route {
    pub key: String,
    pub category: String,
    pub edge_keys: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AnchorRoute {
    pub edge: String,
    pub progress_mm: u32,
    pub route: String,
    pub route_edge_index: u32,
    pub virtual_anchor_index: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ParkingTarget {
    pub key: String,
    pub stable_id: String,
    pub facility: String,
    pub facility_stable_id: String,
    pub tile: u32,
    pub kind: String,
    pub capacity: u32,
    pub entries: Vec<AnchorRoute>,
    pub exits: Vec<AnchorRoute>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Catalog {
    pub catalog_version: u32,
    pub scale: String,
    pub network_revision: String,
    pub namespace: String,
    pub common_namespace: String,
    pub frame_id: String,
    pub policy_id: String,
    pub edge_ids: BTreeMap<String, String>,
    pub profile_ids: BTreeMap<String, String>,
    pub parking_capacity_by_tile: BTreeMap<String, u64>,
    pub routes: Vec<Route>,
    pub parking: Vec<ParkingTarget>,
    pub movements: Vec<Movement>,
    pub signals: Vec<SignalProgram>,
}

pub(crate) fn build(
    source: &GeneratedSource,
    config: &UrbanConfig,
    revision: &SharedNetworkRevision,
) -> Result<Catalog> {
    let mut routes = Vec::new();
    for movement in &source.movements {
        routes.push(Route {
            key: movement.key.clone(),
            category: "junction".into(),
            edge_keys: movement.edges.clone(),
        });
    }
    for cell in &source.layout.cells {
        for arm in crate::Direction::ALL {
            let Some(next) = source.layout.neighbour(cell, arm) else {
                continue;
            };
            let from = source
                .movements
                .iter()
                .find(|m| m.cell == cell.index && m.exit == arm)
                .expect("exit movement");
            let to = source
                .movements
                .iter()
                .find(|m| m.cell == next.index && m.entry == arm.opposite())
                .expect("entry movement");
            routes.push(Route {
                key: format!("{}.cross.{}", cell.key(), arm.key()),
                category: if cell.tile == next.tile {
                    "cross-cell"
                } else {
                    "cross-tile"
                }
                .into(),
                edge_keys: from.edges.iter().chain(&to.edges).cloned().collect(),
            });
        }
    }
    let mut parking = Vec::new();
    let mut capacity = BTreeMap::new();
    for pool in &source.parking {
        let mut targets = [Vec::new(), Vec::new()];
        for (side, anchors) in [&pool.entries, &pool.exits].into_iter().enumerate() {
            for (index, anchor) in anchors.iter().enumerate() {
                let edge_keys = parking_route(source, anchor, side == 0)?;
                let route_edge_index = edge_keys
                    .iter()
                    .position(|key| key == &anchor.edge)
                    .expect("parking anchor on route")
                    as u32;
                let key = format!(
                    "{}.{}.{}",
                    pool.key,
                    if side == 0 { "arrival" } else { "departure" },
                    index
                );
                targets[side].push(AnchorRoute {
                    edge: anchor.edge.clone(),
                    progress_mm: anchor.progress_mm,
                    route: key.clone(),
                    route_edge_index,
                    virtual_anchor_index: if pool.virtual_pool {
                        Some(virtual_anchor_index(revision, &pool.key, side, anchor)?)
                    } else {
                        None
                    },
                });
                routes.push(Route {
                    key,
                    category: if side == 0 {
                        "parking-arrival"
                    } else {
                        "parking-departure"
                    }
                    .into(),
                    edge_keys,
                });
            }
        }
        *capacity.entry(format!("t{:03}", pool.tile)).or_default() += u64::from(pool.capacity);
        let kind = if pool.virtual_pool {
            EntityKind::ParkingFacility
        } else {
            EntityKind::ParkingSpace
        };
        let id = local_id(kind, source::NAMESPACE, &pool.key)?;
        let [entries, exits] = targets;
        parking.push(ParkingTarget {
            key: pool.key.clone(),
            stable_id: format!("{id:x}"),
            facility: pool.facility.clone(),
            facility_stable_id: format!(
                "{:x}",
                local_id(
                    EntityKind::ParkingFacility,
                    source::NAMESPACE,
                    &pool.facility
                )?
            ),
            tile: pool.tile,
            kind: if pool.virtual_pool {
                "virtual"
            } else {
                "explicit"
            }
            .into(),
            capacity: pool.capacity,
            entries,
            exits,
        });
    }
    if capacity
        .values()
        .any(|&n| n < crate::config::MIN_PARKING_PER_TILE)
    {
        return Err(validation(
            "parking capacity",
            "tile has fewer than 750 route-connected positions",
        ));
    }
    let mut edge_ids = BTreeMap::new();
    for key in source.edges.keys() {
        edge_ids.insert(
            key.clone(),
            format!(
                "{:x}",
                local_id(EntityKind::LaneEdge, source::NAMESPACE, key)?
            ),
        );
    }
    let mut profile_ids = BTreeMap::new();
    for profile in &config.profiles {
        profile_ids.insert(
            profile.key.clone(),
            format!(
                "{:x}",
                local_id(
                    EntityKind::VehicleProfile,
                    source::COMMON_NAMESPACE,
                    &profile.key
                )?
            ),
        );
    }
    Ok(Catalog {
        catalog_version: 1,
        scale: source.layout.scale.name().into(),
        network_revision: format!("{:x}", revision.network_revision().as_digest()),
        namespace: source::NAMESPACE.into(),
        common_namespace: source::COMMON_NAMESPACE.into(),
        frame_id: format!(
            "{:x}",
            local_id(
                EntityKind::CanonicalFrame,
                source::NAMESPACE,
                source::FRAME_KEY
            )?
        ),
        policy_id: format!(
            "{:x}",
            local_id(
                EntityKind::RightOfWayPolicySet,
                source::NAMESPACE,
                source::POLICY_KEY
            )?
        ),
        edge_ids,
        profile_ids,
        parking_capacity_by_tile: capacity,
        routes,
        parking,
        movements: source.movements.clone(),
        signals: source.signals.clone(),
    })
}

fn parking_route(
    source: &GeneratedSource,
    anchor: &ParkingAnchor,
    arrival: bool,
) -> Result<Vec<String>> {
    let movement = if arrival {
        source
            .movements
            .iter()
            .find(|m| m.edges.last() == Some(&anchor.edge))
    } else {
        source
            .movements
            .iter()
            .find(|m| m.edges.first() == Some(&anchor.edge))
    };
    if let Some(movement) = movement {
        return Ok(movement.edges.clone());
    }
    // An explicit bay exits onto an outgoing road. Continue over the public cell link
    // and through the neighbouring junction, instead of registering a one-edge stub.
    for next in &source.edges[&anchor.edge].successors {
        if let Some(movement) = source
            .movements
            .iter()
            .find(|m| m.edges.first() == Some(next))
        {
            return Ok(std::iter::once(anchor.edge.clone())
                .chain(movement.edges.iter().cloned())
                .collect());
        }
    }
    Err(validation("parking route", &anchor.edge))
}

fn virtual_anchor_index(
    revision: &SharedNetworkRevision,
    facility: &str,
    side: usize,
    anchor: &ParkingAnchor,
) -> Result<u32> {
    let facility_id = laneflow_static_contract::ParkingFacilityId::from_untyped(local_id(
        EntityKind::ParkingFacility,
        source::NAMESPACE,
        facility,
    )?);
    let ordinal = revision
        .identity()
        .ordinal(facility_id)
        .ok_or_else(|| validation("parking identity", facility))?;
    let view = revision
        .traffic()
        .relations()
        .parking_facility(ordinal)
        .expect("compiled facility");
    let anchors = if side == 0 {
        view.virtual_entries()
    } else {
        view.virtual_exits()
    };
    let edge = revision
        .identity()
        .ordinal(LaneEdgeId::from_untyped(local_id(
            EntityKind::LaneEdge,
            source::NAMESPACE,
            &anchor.edge,
        )?))
        .expect("anchor edge");
    anchors
        .iter()
        .position(|value| value.lane_edge() == edge && value.progress_mm() == anchor.progress_mm)
        .map(|i| i as u32)
        .ok_or_else(|| validation("parking anchor identity", facility))
}

pub(crate) fn local_id(
    kind: EntityKind,
    namespace: &str,
    key: &str,
) -> Result<laneflow_static_contract::StableId128> {
    derive_canonical_stable_id_v1(kind, namespace, key, &CompileLimits::single_network_1m_v2())
        .map_err(|e| validation("identity", e))
}

pub(crate) fn install(
    revision: Arc<SharedNetworkRevision>,
    catalog: &Catalog,
    scale: crate::Scale,
) -> Result<TrafficWorld> {
    let origin = *revision.canonical_origin();
    let edge_occurrences: u64 = catalog
        .routes
        .iter()
        .map(|r| r.edge_keys.len() as u64)
        .sum();
    let mut passages_by_internal_edge = BTreeMap::new();
    for raw in 0..revision
        .identity()
        .entity_count(EntityKind::ParticipantStream)
    {
        let stream = revision
            .conflict()
            .participant_stream(laneflow_static_contract::ParticipantStreamOrdinal::from_raw(raw))
            .expect("compiled stream");
        let path = revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .expect("compiled path");
        let id = revision
            .identity()
            .stable_id(path.edges()[1])
            .expect("path identity");
        passages_by_internal_edge.insert(
            format!("{:x}", id.into_untyped()),
            stream.passages().len() as u64,
        );
    }
    let conflict_occurrences = catalog
        .routes
        .iter()
        .flat_map(|route| &route.edge_keys)
        .map(|key| {
            passages_by_internal_edge
                .get(&catalog.edge_ids[key])
                .copied()
                .unwrap_or(0)
        })
        .sum();
    TrafficWorld::install(
        revision,
        WorldConfig::new(
            scale.nominal_individual_count(),
            catalog.routes.len() as u32,
            edge_occurrences,
            conflict_occurrences,
            1,
            scale.fixed_step_ms(),
        ),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://lf-cn-urban-v1",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .map_err(|e| validation("source reference", e))?,
        },
        542,
        WorldPolicySelection::Pinned(PolicyPin {
            policy: RightOfWayPolicySetId::from_untyped(local_id(
                EntityKind::RightOfWayPolicySet,
                source::NAMESPACE,
                source::POLICY_KEY,
            )?),
        }),
    )
    .map_err(|e| validation("empty world install", e))
}

pub(crate) fn validate(
    source: &GeneratedSource,
    catalog: &Catalog,
    world: &mut TrafficWorld,
) -> Result<()> {
    let revision = world.revision();
    let traffic = revision.traffic();
    if traffic.lane_edge_count() as usize != source.edges.len() {
        return Err(validation(
            "edge coverage",
            "source/installed edge counts differ",
        ));
    }
    let mut edge_map = BTreeMap::new();
    for key in source.edges.keys() {
        let id = LaneEdgeId::from_untyped(local_id(EntityKind::LaneEdge, source::NAMESPACE, key)?);
        let ordinal = revision
            .identity()
            .ordinal(id)
            .ok_or_else(|| validation("edge identity", key))?;
        edge_map.insert(key.clone(), ordinal);
    }
    // Check the compiled graph, not just the source's intended links.
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([LaneEdgeOrdinal::from_raw(0)]);
    while let Some(edge) = queue.pop_front() {
        if !seen.insert(edge) {
            continue;
        }
        queue.extend(traffic.successors(edge).into_iter().flatten().copied());
        queue.extend(traffic.predecessors(edge).into_iter().flatten().copied());
    }
    if seen.len() != source.edges.len() {
        return Err(validation(
            "weak connectivity",
            (seen.len(), source.edges.len()),
        ));
    }
    let pose = revision
        .spatial()
        .and_then(|s| s.lane_pose())
        .ok_or_else(|| validation("spatial", "missing lane geometry"))?;
    for (key, edge) in &source.edges {
        let from = edge_map[key];
        let actual: BTreeSet<_> = traffic
            .successors(from)
            .unwrap_or_default()
            .iter()
            .copied()
            .collect();
        let expected: BTreeSet<_> = edge.successors.iter().map(|k| edge_map[k]).collect();
        if actual != expected {
            return Err(validation("successor closure", key));
        }
        for successor in actual {
            let a = pose.lane_geometry(from).expect("compiled geometry");
            let b = pose.lane_geometry(successor).expect("compiled geometry");
            let end = a.points().last().expect("geometry endpoint");
            let start = b.points().first().expect("geometry endpoint");
            if a.canonical_frame() != b.canonical_frame()
                || (end.x - start.x).abs() > 0.05
                || (end.y - start.y).abs() > 0.05
                || (end.z - start.z).abs() > 0.05
            {
                return Err(validation("successor geometry", key));
            }
        }
        match traffic
            .relations()
            .edge_access(from, ParticipantClassOrdinal::from_raw(0))
        {
            Some(
                AccessCell::Unconstrained
                | AccessCell::Decided {
                    effect: AccessEffect::Allow,
                    ..
                },
            ) => {}
            _ => return Err(validation("profile access", key)),
        }
    }
    for route in &catalog.routes {
        world
            .register_route(RouteRegisterInput::new(
                route
                    .edge_keys
                    .iter()
                    .map(|key| edge_map[key])
                    .collect::<Vec<_>>(),
            ))
            .map_err(|e| validation("register route", (&route.key, e)))?;
    }
    for target in &catalog.parking {
        let facility = revision
            .identity()
            .ordinal(laneflow_static_contract::ParkingFacilityId::from_untyped(
                local_id(
                    EntityKind::ParkingFacility,
                    source::NAMESPACE,
                    &target.facility,
                )?,
            ))
            .expect("catalog facility");
        if target.kind == "virtual" {
            let view = traffic
                .relations()
                .parking_facility(facility)
                .expect("compiled facility");
            if view.virtual_capacity() != target.capacity
                || view.virtual_entries().len() != target.entries.len()
                || view.virtual_exits().len() != target.exits.len()
            {
                return Err(validation("parking pool correspondence", &target.key));
            }
        } else {
            let ordinal = revision
                .identity()
                .ordinal(laneflow_static_contract::ParkingSpaceId::from_untyped(
                    local_id(EntityKind::ParkingSpace, source::NAMESPACE, &target.key)?,
                ))
                .expect("catalog space");
            let view = traffic
                .relations()
                .parking_space(ordinal)
                .expect("compiled space");
            if target.capacity != 1
                || view.area() != Some(facility)
                || view.entry()
                    != (
                        edge_map[&target.entries[0].edge],
                        target.entries[0].progress_mm,
                    )
                || view.exit() != (edge_map[&target.exits[0].edge], target.exits[0].progress_mm)
            {
                return Err(validation("explicit parking correspondence", &target.key));
            }
            for raw in 0..revision.identity().entity_count(EntityKind::VehicleProfile) {
                let profile = traffic
                    .relations()
                    .vehicle_profile(laneflow_static_contract::VehicleProfileOrdinal::from_raw(
                        raw,
                    ))
                    .expect("compiled profile");
                if profile.length_mm() > view.geometry().2 {
                    return Err(validation("parking profile eligibility", &target.key));
                }
            }
        }
        for anchor in target.entries.iter().chain(&target.exits) {
            if anchor.progress_mm
                >= traffic.lane_lengths_millimetres()[edge_map[&anchor.edge].index()]
            {
                return Err(validation("parking anchor bounds", &target.key));
            }
        }
    }
    Ok(())
}
