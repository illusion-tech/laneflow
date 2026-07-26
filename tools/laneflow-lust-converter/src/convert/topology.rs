//! Convert a parsed SUMO network into Traffic + Spatial packages.
//!
//! Emits Junction / Movement / ManeuverPath, optional static Signals,
//! vehicleProfiles, and optional DUE-derived routes + population table.

use std::collections::HashMap;

use crate::{
    Error, Result,
    convert::{
        junction::normalize_junctions,
        population::{
            POPULATION_CANDIDATE_COUNT, POPULATION_DEPART_END_SECONDS,
            POPULATION_DEPART_START_SECONDS, select_population,
        },
        routes::build_routes_and_bind_population,
        signals::convert_signals,
    },
    output::{
        TopologyArtifacts, finish_topology_artifacts, json_bytes,
        model::{
            Centerline, LaneConnection, LaneEdge, LaneGraph, Parking, PopulationSelection,
            PopulationTable, PopulationTableRecord, Route, SpatialEdge, SpatialPackage,
            TrafficPackage, Units, VehicleProfile,
        },
    },
    sumo::{DueVehicle, LUST_FRAME_ID, SUMO_ID_PREFIX, SumoLane, SumoNetwork, SumoTlLogic},
};

const DEFAULT_FIXED_DELTA_MS: u64 = 16;
const DEFAULT_TRAFFIC_REF: &str = "lust-topology.traffic.json";
const DEFAULT_SPATIAL_REF: &str = "lust-topology.spatial.json";

/// Options for topology / static conversion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopologyConvertOptions {
    pub fixed_delta_ms: u64,
    pub traffic_artifact_ref: String,
    pub spatial_artifact_ref: String,
    /// When true, require pinned LuST `<location>` lexical anchors.
    pub require_lust_location_anchors: bool,
    /// When true, require exactly 10,592 filtered DUE candidates before taking 10k.
    pub require_lust_population_count: bool,
}

impl Default for TopologyConvertOptions {
    fn default() -> Self {
        Self {
            fixed_delta_ms: DEFAULT_FIXED_DELTA_MS,
            traffic_artifact_ref: DEFAULT_TRAFFIC_REF.to_owned(),
            spatial_artifact_ref: DEFAULT_SPATIAL_REF.to_owned(),
            require_lust_location_anchors: false,
            require_lust_population_count: false,
        }
    }
}

/// Topology packages plus harness-only population table bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaticConversionArtifacts {
    pub topology: TopologyArtifacts,
    pub population: Vec<u8>,
    pub population_record_count: usize,
    pub route_count: usize,
}

/// Build validated Traffic/Spatial/Manifest bytes from a SUMO network.
pub fn convert_network_topology(
    network: &SumoNetwork,
    options: &TopologyConvertOptions,
) -> Result<TopologyArtifacts> {
    convert_network_topology_with_tll(network, &[], options)
}

/// Build validated packages using static programs from `tll.static.xml`.
pub fn convert_network_topology_with_tll(
    network: &SumoNetwork,
    tll_programs: &[SumoTlLogic],
    options: &TopologyConvertOptions,
) -> Result<TopologyArtifacts> {
    convert_network_topology_with_tll_and_profiles(network, tll_programs, &[], options)
}

/// Build validated packages with static signals and vehicle profiles.
pub(crate) fn convert_network_topology_with_tll_and_profiles(
    network: &SumoNetwork,
    tll_programs: &[SumoTlLogic],
    vehicle_profiles: &[VehicleProfile],
    options: &TopologyConvertOptions,
) -> Result<TopologyArtifacts> {
    convert_network_packages(network, tll_programs, vehicle_profiles, &[], options)
}

/// Convert topology + routes + population from network inputs and ordered DUE vehicles.
pub(crate) fn convert_static_with_due(
    network: &SumoNetwork,
    tll_programs: &[SumoTlLogic],
    vehicle_profiles: &[VehicleProfile],
    due_vehicles: &[DueVehicle],
    options: &TopologyConvertOptions,
) -> Result<StaticConversionArtifacts> {
    let topology_norm = normalize_junctions(network)?;
    let population =
        select_population(due_vehicles, options.require_lust_population_count)?;
    let bundle = build_routes_and_bind_population(network, &topology_norm, &population)?;

    let topology = convert_network_packages(
        network,
        tll_programs,
        vehicle_profiles,
        &bundle.routes,
        options,
    )?;

    let table = PopulationTable {
        format_version: "0.1",
        selection: PopulationSelection {
            depart_start_seconds: POPULATION_DEPART_START_SECONDS,
            depart_end_seconds_exclusive: POPULATION_DEPART_END_SECONDS,
            require_lust_candidate_count: options.require_lust_population_count,
            candidate_count_expected: options
                .require_lust_population_count
                .then_some(POPULATION_CANDIDATE_COUNT as u64),
            selected_count: u64::try_from(bundle.records.len()).expect("count fits u64"),
            route_catalog_count: u64::try_from(bundle.routes.len()).expect("count fits u64"),
        },
        records: bundle
            .records
            .iter()
            .map(|record| PopulationTableRecord {
                population_rank: record.population_rank,
                vehicle_id: record.vehicle_id.clone(),
                vehicle_profile_id: record.vehicle_profile_id.clone(),
                depart_seconds: record.depart.to_string(),
                route_id: record.route_id.clone(),
                road_edge_ids: record.road_edge_ids.clone(),
                source_file_ordinal: record.source_file_ordinal,
                source_vehicle_ordinal: record.source_vehicle_ordinal,
            })
            .collect(),
    };
    let population_bytes = json_bytes("PopulationTable", &table)?;
    Ok(StaticConversionArtifacts {
        population_record_count: bundle.records.len(),
        route_count: bundle.routes.len(),
        topology,
        population: population_bytes,
    })
}

fn convert_network_packages(
    network: &SumoNetwork,
    tll_programs: &[SumoTlLogic],
    vehicle_profiles: &[VehicleProfile],
    routes: &[Route],
    options: &TopologyConvertOptions,
) -> Result<TopologyArtifacts> {
    if options.require_lust_location_anchors && !network.location.matches_lust_anchors() {
        return Err(Error::SumoModel(format!(
            "SUMO <location> does not match pinned LuST anchors (netOffset={:?}, convBoundary={:?})",
            network.location.net_offset_raw, network.location.conv_boundary_raw
        )));
    }

    let origin = network.location.canonical_origin()?;
    let lane_by_edge_index = build_lane_index(network);
    let mut connections_by_from: HashMap<String, Vec<String>> = HashMap::new();

    for connection in &network.connections {
        let from_lane = resolve_lane(
            &lane_by_edge_index,
            &connection.from_edge_id,
            connection.from_lane,
        )?;
        let to_lane = resolve_lane(
            &lane_by_edge_index,
            &connection.to_edge_id,
            connection.to_lane,
        )?;
        let mut chain = Vec::with_capacity(connection.via_lane_ids.len() + 2);
        chain.push(from_lane.laneflow_id());
        for via_id in &connection.via_lane_ids {
            ensure_lane_exists(network, via_id)?;
            chain.push(format!("{SUMO_ID_PREFIX}{via_id}"));
        }
        chain.push(to_lane.laneflow_id());
        for window in chain.windows(2) {
            connections_by_from
                .entry(window[0].clone())
                .or_default()
                .push(window[1].clone());
        }
    }

    for targets in connections_by_from.values_mut() {
        targets.sort();
        targets.dedup();
    }

    let topology = normalize_junctions(network)?;
    let signals = convert_signals(network, tll_programs, &topology.path_by_connection)?;

    let mut lane_edges = Vec::with_capacity(network.lanes.len());
    let mut spatial_edges = Vec::with_capacity(network.lanes.len());

    for lane in &network.lanes {
        let id = lane.laneflow_id();
        let length = lane.length.to_f64()?;
        let speed_limit = lane.speed.to_f64()?;
        if length <= 0.0 {
            return Err(Error::SumoModel(format!(
                "lane {:?} length must be positive, got {length}",
                lane.id
            )));
        }
        if speed_limit <= 0.0 {
            return Err(Error::SumoModel(format!(
                "lane {:?} speed must be positive, got {speed_limit}",
                lane.id
            )));
        }
        let connections = connections_by_from
            .get(&id)
            .into_iter()
            .flatten()
            .map(|to_edge_id| LaneConnection {
                to_edge_id: to_edge_id.clone(),
            })
            .collect();
        lane_edges.push(LaneEdge {
            id: id.clone(),
            length,
            speed_limit,
            connections,
        });

        let mut points = Vec::with_capacity(lane.shape.len());
        for (sx, sy) in &lane.shape {
            let projected_x = sx.checked_sub(network.location.net_offset.0)?;
            let projected_y = sy.checked_sub(network.location.net_offset.1)?;
            let x = projected_x.checked_sub(origin.0)?.to_f64()?;
            let z = projected_y.checked_sub(origin.1)?.to_f64()?;
            points.push([x, 0.0, z]);
        }
        spatial_edges.push(SpatialEdge {
            traffic_edge_id: id,
            centerline: Centerline { points },
        });
    }

    let traffic = TrafficPackage {
        format_version: "0.8",
        units: Units {
            distance: "meter",
            time: "second",
        },
        lane_graph: LaneGraph { edges: lane_edges },
        junctions: topology.junctions,
        movements: topology.movements,
        maneuver_paths: topology.maneuver_paths,
        routes: routes.to_vec(),
        vehicle_profiles: vehicle_profiles.to_vec(),
        signals,
        parking: Parking {
            areas: Vec::new(),
            spaces: Vec::new(),
        },
    };
    let spatial = SpatialPackage {
        format_version: "0.1",
        frame_id: LUST_FRAME_ID.to_owned(),
        edges: spatial_edges,
    };

    let edge_count = traffic.lane_graph.edges.len();
    let traffic_bytes = json_bytes("TrafficPackage", &traffic)?;
    let spatial_bytes = json_bytes("SpatialPackage", &spatial)?;
    finish_topology_artifacts(
        options.fixed_delta_ms,
        options.traffic_artifact_ref.clone(),
        options.spatial_artifact_ref.clone(),
        traffic_bytes,
        spatial_bytes,
        edge_count,
    )
}

fn build_lane_index(network: &SumoNetwork) -> HashMap<(String, u32), &SumoLane> {
    network
        .lanes
        .iter()
        .map(|lane| ((lane.edge_id.clone(), lane.index), lane))
        .collect()
}

fn resolve_lane<'a>(
    index: &HashMap<(String, u32), &'a SumoLane>,
    edge_id: &str,
    lane_index: u32,
) -> Result<&'a SumoLane> {
    index
        .get(&(edge_id.to_owned(), lane_index))
        .copied()
        .ok_or_else(|| {
            Error::SumoModel(format!(
                "connection references unknown lane edge={edge_id:?} index={lane_index}"
            ))
        })
}

fn ensure_lane_exists(network: &SumoNetwork, lane_id: &str) -> Result<()> {
    if network.lane(lane_id).is_some() {
        Ok(())
    } else {
        Err(Error::SumoModel(format!(
            "connection via references unknown lane {lane_id:?}"
        )))
    }
}
