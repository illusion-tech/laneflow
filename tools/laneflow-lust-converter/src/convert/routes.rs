//! Expand road-level DUE routes into lane-level Traffic routes (§3.1).

use std::collections::HashMap;

use crate::{
    Error, Result,
    convert::junction::NormalizedTopology,
    convert::population::PopulationRecord,
    output::model::{ManeuverPath, Route},
    sumo::{SUMO_ID_PREFIX, SumoNetwork},
};

#[derive(Clone, Debug)]
struct HopOption {
    from_lane: u32,
    to_lane: u32,
    path: ManeuverPath,
}

/// One population record bound to a Traffic route catalog entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoundPopulationRecord {
    pub population_rank: u32,
    pub vehicle_id: String,
    pub vehicle_profile_id: String,
    pub depart: crate::sumo::ExactDecimal,
    pub route_id: String,
    pub road_edge_ids: Vec<String>,
    pub source_file_ordinal: u8,
    pub source_vehicle_ordinal: u64,
}

/// Route catalog plus bound population rows.
#[derive(Clone, Debug)]
pub(crate) struct RoutePopulationBundle {
    pub routes: Vec<Route>,
    pub records: Vec<BoundPopulationRecord>,
}

/// Expand selected population road routes and dedupe identical lane sequences.
pub(crate) fn build_routes_and_bind_population(
    network: &SumoNetwork,
    topology: &NormalizedTopology,
    population: &[PopulationRecord],
) -> Result<RoutePopulationBundle> {
    let hops = build_hop_index(topology);
    let lanes_by_edge = build_external_lanes_by_edge(network);

    let mut routes = Vec::new();
    let mut route_id_by_lane_seq: HashMap<Vec<String>, String> = HashMap::new();
    let mut records = Vec::with_capacity(population.len());

    for record in population {
        validate_known_edges(network, &record.road_edge_ids, &record.vehicle_id)?;
        let lane_edge_ids =
            expand_road_route(network, &lanes_by_edge, &hops, &record.road_edge_ids)?;
        let route_id = if let Some(existing) = route_id_by_lane_seq.get(&lane_edge_ids) {
            existing.clone()
        } else {
            let id = format!("{SUMO_ID_PREFIX}route-{}", routes.len());
            route_id_by_lane_seq.insert(lane_edge_ids.clone(), id.clone());
            routes.push(Route {
                id: id.clone(),
                edge_ids: lane_edge_ids,
            });
            id
        };
        records.push(BoundPopulationRecord {
            population_rank: record.population_rank,
            vehicle_id: record.vehicle_id.clone(),
            vehicle_profile_id: format!("{SUMO_ID_PREFIX}{}", record.type_id),
            depart: record.depart,
            route_id,
            road_edge_ids: record.road_edge_ids.clone(),
            source_file_ordinal: record.source_file_ordinal,
            source_vehicle_ordinal: record.source_vehicle_ordinal,
        });
    }

    Ok(RoutePopulationBundle { routes, records })
}

fn expand_road_route(
    network: &SumoNetwork,
    lanes_by_edge: &HashMap<&str, Vec<u32>>,
    hops: &HashMap<(String, String), Vec<HopOption>>,
    road_edge_ids: &[String],
) -> Result<Vec<String>> {
    if road_edge_ids.is_empty() {
        return Err(Error::SumoModel(
            "road-level route must contain at least one edge".to_owned(),
        ));
    }

    let start_lanes = lanes_by_edge
        .get(road_edge_ids[0].as_str())
        .ok_or_else(|| {
            Error::SumoModel(format!(
                "road route starts on unknown or non-external edge {:?}",
                road_edge_ids[0]
            ))
        })?;

    let mut memo: HashMap<(usize, u32), bool> = HashMap::new();
    for &start_lane in start_lanes {
        if can_complete(lanes_by_edge, hops, road_edge_ids, 0, start_lane, &mut memo) {
            return materialize_path(
                network,
                lanes_by_edge,
                hops,
                road_edge_ids,
                start_lane,
                &mut memo,
            );
        }
    }

    Err(Error::SumoModel(format!(
        "no complete lane-level path for road route {road_edge_ids:?}"
    )))
}

fn can_complete(
    lanes_by_edge: &HashMap<&str, Vec<u32>>,
    hops: &HashMap<(String, String), Vec<HopOption>>,
    road_edge_ids: &[String],
    edge_index: usize,
    lane_index: u32,
    memo: &mut HashMap<(usize, u32), bool>,
) -> bool {
    if let Some(&cached) = memo.get(&(edge_index, lane_index)) {
        return cached;
    }
    let result = if edge_index + 1 == road_edge_ids.len() {
        lanes_by_edge
            .get(road_edge_ids[edge_index].as_str())
            .is_some_and(|lanes| lanes.contains(&lane_index))
    } else {
        let from = &road_edge_ids[edge_index];
        let to = &road_edge_ids[edge_index + 1];
        hops.get(&(from.clone(), to.clone()))
            .into_iter()
            .flatten()
            .filter(|hop| hop.from_lane == lane_index)
            .any(|hop| {
                can_complete(
                    lanes_by_edge,
                    hops,
                    road_edge_ids,
                    edge_index + 1,
                    hop.to_lane,
                    memo,
                )
            })
    };
    memo.insert((edge_index, lane_index), result);
    result
}

fn materialize_path(
    network: &SumoNetwork,
    lanes_by_edge: &HashMap<&str, Vec<u32>>,
    hops: &HashMap<(String, String), Vec<HopOption>>,
    road_edge_ids: &[String],
    start_lane: u32,
    memo: &mut HashMap<(usize, u32), bool>,
) -> Result<Vec<String>> {
    let mut lane_edge_ids = vec![lane_laneflow_id(network, &road_edge_ids[0], start_lane)?];
    let mut current_lane = start_lane;

    for edge_index in 0..road_edge_ids.len().saturating_sub(1) {
        let from = &road_edge_ids[edge_index];
        let to = &road_edge_ids[edge_index + 1];
        let options = hops.get(&(from.clone(), to.clone())).ok_or_else(|| {
            Error::SumoModel(format!(
                "no ManeuverPath hop from road edge {from:?} to {to:?}"
            ))
        })?;

        let mut chosen: Option<&HopOption> = None;
        for hop in options.iter().filter(|hop| hop.from_lane == current_lane) {
            if can_complete(
                lanes_by_edge,
                hops,
                road_edge_ids,
                edge_index + 1,
                hop.to_lane,
                memo,
            ) {
                chosen = Some(hop);
                break;
            }
        }
        let hop = chosen.ok_or_else(|| {
            Error::SumoModel(format!(
                "ambiguous or incomplete ManeuverPath expansion at {from:?} lane {current_lane} -> {to:?}"
            ))
        })?;

        for internal in &hop.path.internal_edge_ids {
            lane_edge_ids.push(internal.clone());
        }
        lane_edge_ids.push(hop.path.exit_edge_id.clone());
        current_lane = hop.to_lane;
    }

    Ok(lane_edge_ids)
}

fn build_hop_index(topology: &NormalizedTopology) -> HashMap<(String, String), Vec<HopOption>> {
    let path_by_id: HashMap<&str, &ManeuverPath> = topology
        .maneuver_paths
        .iter()
        .map(|path| (path.id.as_str(), path))
        .collect();
    let mut hops: HashMap<(String, String), Vec<HopOption>> = HashMap::new();
    for ((from_road, from_lane, to_road, to_lane), path_id) in &topology.path_by_connection {
        let path = (*path_by_id
            .get(path_id.as_str())
            .expect("path_by_connection references emitted ManeuverPath"))
        .clone();
        hops.entry((from_road.clone(), to_road.clone()))
            .or_default()
            .push(HopOption {
                from_lane: *from_lane,
                to_lane: *to_lane,
                path,
            });
    }
    for options in hops.values_mut() {
        options.sort_by(|left, right| {
            left.from_lane
                .cmp(&right.from_lane)
                .then(left.to_lane.cmp(&right.to_lane))
                .then(left.path.id.cmp(&right.path.id))
        });
    }
    hops
}

fn build_external_lanes_by_edge(network: &SumoNetwork) -> HashMap<&str, Vec<u32>> {
    let mut lanes_by_edge: HashMap<&str, Vec<u32>> = HashMap::new();
    for lane in &network.lanes {
        if lane.function_internal {
            continue;
        }
        lanes_by_edge
            .entry(lane.edge_id.as_str())
            .or_default()
            .push(lane.index);
    }
    for lanes in lanes_by_edge.values_mut() {
        lanes.sort_unstable();
        lanes.dedup();
    }
    lanes_by_edge
}

fn validate_known_edges(
    network: &SumoNetwork,
    road_edge_ids: &[String],
    vehicle_id: &str,
) -> Result<()> {
    for edge_id in road_edge_ids {
        let edge = network.edge(edge_id).ok_or_else(|| {
            Error::SumoModel(format!(
                "DUE vehicle {vehicle_id:?} references unknown road edge {edge_id:?}"
            ))
        })?;
        if edge.function_internal {
            return Err(Error::SumoModel(format!(
                "DUE vehicle {vehicle_id:?} route references internal edge {edge_id:?}"
            )));
        }
    }
    Ok(())
}

fn lane_laneflow_id(network: &SumoNetwork, edge_id: &str, lane_index: u32) -> Result<String> {
    network
        .lanes
        .iter()
        .find(|lane| lane.edge_id == edge_id && lane.index == lane_index && !lane.function_internal)
        .map(crate::sumo::SumoLane::laneflow_id)
        .ok_or_else(|| {
            Error::SumoModel(format!(
                "missing external lane edge={edge_id:?} index={lane_index}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        convert::junction::normalize_junctions,
        convert::population::{PopulationRecord, select_population},
        sumo::{due::DueVehicle, parse_sumo_network_xml},
    };

    #[test]
    fn fixture_west_east_expands_through_internal() {
        let xml = include_str!("../../tests/fixtures/minimal/t-junction.net.xml");
        let network = parse_sumo_network_xml(xml).expect("parse");
        let topology = normalize_junctions(&network).expect("normalize");
        let vehicles = vec![DueVehicle {
            id: "v0".to_owned(),
            type_id: "passenger1".to_owned(),
            depart: "28800".parse().unwrap(),
            road_edge_ids: vec!["west".to_owned(), "east".to_owned()],
            source_file_ordinal: 0,
            source_vehicle_ordinal: 0,
        }];
        let population = select_population(&vehicles, false).expect("population");
        let bundle =
            build_routes_and_bind_population(&network, &topology, &population).expect("routes");
        assert_eq!(bundle.routes.len(), 1);
        assert_eq!(
            bundle.routes[0].edge_ids,
            [
                "sumo:west_0".to_owned(),
                "sumo::J_0_0".to_owned(),
                "sumo:east_0".to_owned(),
            ]
        );
        assert_eq!(bundle.records[0].route_id, "sumo:route-0");
        assert_eq!(bundle.records[0].vehicle_profile_id, "sumo:passenger1");
    }

    #[test]
    fn identical_lane_sequences_share_catalog_entry() {
        let xml = include_str!("../../tests/fixtures/minimal/t-junction.net.xml");
        let network = parse_sumo_network_xml(xml).expect("parse");
        let topology = normalize_junctions(&network).expect("normalize");
        let population = vec![
            PopulationRecord {
                population_rank: 0,
                vehicle_id: "a".to_owned(),
                type_id: "passenger1".to_owned(),
                depart: "28800".parse().unwrap(),
                road_edge_ids: vec!["west".to_owned(), "south".to_owned()],
                source_file_ordinal: 0,
                source_vehicle_ordinal: 0,
            },
            PopulationRecord {
                population_rank: 1,
                vehicle_id: "b".to_owned(),
                type_id: "passenger2a".to_owned(),
                depart: "28801".parse().unwrap(),
                road_edge_ids: vec!["west".to_owned(), "south".to_owned()],
                source_file_ordinal: 0,
                source_vehicle_ordinal: 1,
            },
        ];
        let bundle =
            build_routes_and_bind_population(&network, &topology, &population).expect("routes");
        assert_eq!(bundle.routes.len(), 1);
        assert_eq!(bundle.records[0].route_id, bundle.records[1].route_id);
    }
}
