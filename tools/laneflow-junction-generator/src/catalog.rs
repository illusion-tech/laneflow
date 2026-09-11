//! 由拓扑编制复杂路口 catalog 并做交叉校验。

use laneflow_scenario::complex_junction::{
    AUTHORING_NAMESPACE, CATALOG_VERSION, JunctionCatalog, POLICY_KEY, PortalCatalogEntry,
    PortalLaneCatalogEntry, RouteCatalogEntry, SpawnSlotCatalogEntry,
    WeightedRouteChoiceCatalogEntry, validate,
};

use crate::Error;
use crate::config::JunctionConfig;
use crate::topology::TopologyBuild;

pub(crate) fn build_catalog(
    config: &JunctionConfig,
    topology: &TopologyBuild,
) -> Result<JunctionCatalog, Error> {
    let clearance = config.endpoint_clearance_meters();
    let mut slots = Vec::new();
    let portals = topology
        .portals
        .iter()
        .map(|portal| {
            let lanes = portal
                .lanes
                .iter()
                .enumerate()
                .map(|(lane_index, lane)| {
                    let length = topology.edge_length(&lane.edge);
                    let minimum_length = clearance * 2.0;
                    if length < minimum_length {
                        return Err(Error::Config(format!(
                            "portal {:?} lane {lane_index} loop edge {:?} is {length} m long; \
                             at least {minimum_length} m is required for one spawn slot",
                            portal.id, lane.edge
                        )));
                    }
                    let mut local_index = 0_u32;
                    let mut progress = clearance;
                    while progress <= length - clearance {
                        slots.push(SpawnSlotCatalogEntry {
                            slot_id: format!("slot-{}-{local_index:03}", lane.edge),
                            portal_id: portal.id.to_owned(),
                            lane_index,
                            edge_id: lane.edge.clone(),
                            progress,
                        });
                        local_index += 1;
                        progress += config.geometry.spawn_slot_pitch_meters;
                    }
                    Ok(PortalLaneCatalogEntry {
                        lane_index,
                        entry_spawn_slot_id: format!("slot-{}-000", lane.edge),
                        route_choices: lane
                            .choices
                            .iter()
                            .map(|&(route_id, weight)| WeightedRouteChoiceCatalogEntry {
                                route_id: route_id.to_owned(),
                                weight,
                            })
                            .collect(),
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?;
            Ok(PortalCatalogEntry {
                id: portal.id.to_owned(),
                lanes,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let routes = topology
        .routes
        .iter()
        .map(|route| RouteCatalogEntry {
            route_id: route.id.to_owned(),
            exit_portal_id: route.exit_portal.to_owned(),
            edge_ids: route.edges.clone(),
            focus: route.focus,
        })
        .collect();

    Ok(JunctionCatalog {
        catalog_version: CATALOG_VERSION.to_owned(),
        policy_selection: laneflow_scenario::complex_junction::CatalogPolicySelection::Pinned {
            policy: laneflow_static_contract::RightOfWayPolicySetId::from_untyped(
                laneflow_compiler::derive_canonical_stable_id_v1(
                    laneflow_static_contract::EntityKind::RightOfWayPolicySet,
                    AUTHORING_NAMESPACE,
                    POLICY_KEY,
                    &laneflow_compiler::CompileLimits::p100_initial_v1(),
                )
                .map_err(|error| Error::Catalog(format!("policy identity: {error:?}")))?,
            )
            .to_string(),
        },
        portals,
        routes,
        spawn_slots: slots,
    })
}

pub(crate) fn validate_catalog(
    catalog: &JunctionCatalog,
    topology: &TopologyBuild,
    config: &JunctionConfig,
) -> Result<(), Error> {
    let encoded = toml::to_string(catalog)?;
    let decoded: JunctionCatalog =
        toml::from_str(&encoded).map_err(|error| Error::Catalog(error.to_string()))?;
    if decoded != *catalog {
        return Err(Error::Catalog(
            "TOML round trip changed catalog semantics".to_owned(),
        ));
    }
    validate(catalog).map_err(|error| Error::Catalog(error.to_string()))?;

    let route_by_id = topology
        .routes
        .iter()
        .map(|route| (route.id, route))
        .collect::<std::collections::HashMap<_, _>>();
    let lane_by_key = catalog
        .portals
        .iter()
        .flat_map(|portal| {
            portal
                .lanes
                .iter()
                .map(move |lane| ((portal.id.as_str(), lane.lane_index), lane))
        })
        .collect::<std::collections::HashMap<_, _>>();
    let clearance = config.endpoint_clearance_meters();
    for slot in &catalog.spawn_slots {
        let lane = lane_by_key
            .get(&(slot.portal_id.as_str(), slot.lane_index))
            .expect("scenario validate checked portal lane");
        for choice in &lane.route_choices {
            let route = route_by_id
                .get(choice.route_id.as_str())
                .ok_or_else(|| Error::Catalog(format!("unknown route {:?}", choice.route_id)))?;
            if route.edges.first().map(String::as_str) != Some(slot.edge_id.as_str()) {
                return Err(Error::Catalog(format!(
                    "slot {:?} edge_id is not the entry edge of route {:?}",
                    slot.slot_id, choice.route_id
                )));
            }
        }
        // slot progress 闭区间对照 corridor-generator：端点净距内不许有停车位。
        let length = topology.edge_length(&slot.edge_id);
        if !slot.progress.is_finite()
            || slot.progress < clearance
            || slot.progress > length - clearance
        {
            return Err(Error::Catalog(format!(
                "slot {:?} progress {} outside [{clearance}, {}]",
                slot.slot_id,
                slot.progress,
                length - clearance
            )));
        }
    }
    Ok(())
}
