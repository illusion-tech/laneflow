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

trait RouteObserver {
    fn start_lanes(&mut self, _lanes: &[u32]) {}
    fn state(&mut self, _occurrence: usize, _lane: u32, _result: bool, _cached: bool) {}
    fn hop(&mut self, _occurrence: usize, _hop: &HopOption) {}
    fn materialize_entry(&mut self, _start_lane: u32) {}
    #[cfg(test)]
    fn inject_materialize_failure(&mut self) -> bool {
        false
    }
}

struct NoopObserver;

impl RouteObserver for NoopObserver {}

fn expand_road_route(
    network: &SumoNetwork,
    lanes_by_edge: &HashMap<&str, Vec<u32>>,
    hops: &HashMap<(String, String), Vec<HopOption>>,
    road_edge_ids: &[String],
) -> Result<Vec<String>> {
    expand_road_route_with_observer(
        network,
        lanes_by_edge,
        hops,
        road_edge_ids,
        &mut NoopObserver,
    )
}

fn expand_road_route_with_observer(
    network: &SumoNetwork,
    lanes_by_edge: &HashMap<&str, Vec<u32>>,
    hops: &HashMap<(String, String), Vec<HopOption>>,
    road_edge_ids: &[String],
    observer: &mut impl RouteObserver,
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
    observer.start_lanes(start_lanes);

    let mut memo: HashMap<(usize, u32), bool> = HashMap::new();
    for &start_lane in start_lanes {
        if can_complete_with_observer(
            lanes_by_edge,
            hops,
            road_edge_ids,
            0,
            start_lane,
            &mut memo,
            observer,
        ) {
            return materialize_path_with_observer(
                network,
                lanes_by_edge,
                hops,
                road_edge_ids,
                start_lane,
                &mut memo,
                observer,
            );
        }
    }

    Err(Error::SumoModel(format!(
        "no complete lane-level path for road route {road_edge_ids:?}"
    )))
}

/// #1053：诊断完整入选集合，复用索引但逐条执行生产校验与展开；不跳过或补位。
#[cfg(test)]
pub(super) fn population_route_failures(
    network: &SumoNetwork,
    topology: &NormalizedTopology,
    population: &[PopulationRecord],
) -> Vec<(String, String)> {
    let hops = build_hop_index(topology);
    let lanes_by_edge = build_external_lanes_by_edge(network);
    population
        .iter()
        .filter_map(|record| {
            validate_known_edges(network, &record.road_edge_ids, &record.vehicle_id)
                .and_then(|()| {
                    expand_road_route(network, &lanes_by_edge, &hops, &record.road_edge_ids)
                })
                .err()
                .map(|error| (record.vehicle_id.clone(), error.to_string()))
        })
        .collect()
}

fn can_complete_with_observer(
    lanes_by_edge: &HashMap<&str, Vec<u32>>,
    hops: &HashMap<(String, String), Vec<HopOption>>,
    road_edge_ids: &[String],
    edge_index: usize,
    lane_index: u32,
    memo: &mut HashMap<(usize, u32), bool>,
    observer: &mut impl RouteObserver,
) -> bool {
    if let Some(&cached) = memo.get(&(edge_index, lane_index)) {
        observer.state(edge_index, lane_index, cached, true);
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
                observer.hop(edge_index, hop);
                can_complete_with_observer(
                    lanes_by_edge,
                    hops,
                    road_edge_ids,
                    edge_index + 1,
                    hop.to_lane,
                    memo,
                    &mut *observer,
                )
            })
    };
    observer.state(edge_index, lane_index, result, false);
    memo.insert((edge_index, lane_index), result);
    result
}

fn materialize_path_with_observer(
    network: &SumoNetwork,
    lanes_by_edge: &HashMap<&str, Vec<u32>>,
    hops: &HashMap<(String, String), Vec<HopOption>>,
    road_edge_ids: &[String],
    start_lane: u32,
    memo: &mut HashMap<(usize, u32), bool>,
    observer: &mut impl RouteObserver,
) -> Result<Vec<String>> {
    observer.materialize_entry(start_lane);
    #[cfg(test)]
    if observer.inject_materialize_failure() {
        return Err(Error::SumoModel("injected materialize failure".to_owned()));
    }
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
            if can_complete_with_observer(
                lanes_by_edge,
                hops,
                road_edge_ids,
                edge_index + 1,
                hop.to_lane,
                memo,
                &mut *observer,
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

/// 按需内部诊断；默认转换不调用，非测试构建不因入口暂未接入而删除这些实现。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod diagnostics;

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

/// 诊断模式的入选集 route 边引用校验（#253 W2）：只校验存在性与非 internal，
/// 不展开 lane 级路径（#806 实现语义缺口的既定立场不变）——K3(a) 失败域
/// 不因转换模式而豁免，诊断清单不得静默收录引用不存在/internal 边的入选
/// 车辆。
pub(crate) fn validate_population_route_edges(
    network: &SumoNetwork,
    population: &[PopulationRecord],
) -> Result<()> {
    for record in population {
        validate_known_edges(network, &record.road_edge_ids, &record.vehicle_id)?;
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
pub(crate) mod test_support;

#[cfg(test)]
mod tests {
    use super::{
        test_support::{
            ErrorEvidence, NO_COMPLETE_ROUTE_ERROR, PathEvidence, RouteFixture, RouteFixtureKind,
            diagnostic_options, error_evidence, expected_final_chain, path_evidence,
            population_record,
        },
        *,
    };
    use crate::{
        convert::junction::normalize_junctions,
        convert::population::{PopulationRecord, select_population},
        sumo::{due::DueVehicle, parse_sumo_network_xml},
    };

    #[test]
    fn fixture_west_east_expands_through_internal() {
        let xml = include_str!("../../tests/fixtures/minimal/t-junction.net.xml");
        let network = parse_sumo_network_xml(xml).expect("parse");
        let topology = normalize_junctions(
            &network,
            &crate::convert::junction::StubWeldGate::Unrestricted,
        )
        .expect("normalize");
        let vehicles = vec![DueVehicle {
            id: "v0".to_owned(),
            type_id: "passenger1".to_owned(),
            depart: "28800".parse().unwrap(),
            road_edge_ids: vec!["west".to_owned(), "east".to_owned()],
            depart_pos: Some("random".to_owned()),
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
        let topology = normalize_junctions(
            &network,
            &crate::convert::junction::StubWeldGate::Unrestricted,
        )
        .expect("normalize");
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

    #[test]
    fn validate_population_route_edges_rejects_unknown_and_internal() {
        // #253 W2：诊断模式复用的边引用校验——存在性与非 internal 分别
        // fail-closed，合法引用原样通过。
        let xml = include_str!("../../tests/fixtures/minimal/t-junction.net.xml");
        let network = parse_sumo_network_xml(xml).expect("parse");
        let record = |vehicle_id: &str, edges: &[&str]| PopulationRecord {
            population_rank: 0,
            vehicle_id: vehicle_id.to_owned(),
            type_id: "passenger1".to_owned(),
            depart: "28800".parse().unwrap(),
            road_edge_ids: edges.iter().map(|edge| (*edge).to_owned()).collect(),
            source_file_ordinal: 0,
            source_vehicle_ordinal: 0,
        };
        super::validate_population_route_edges(&network, &[record("ok", &["west", "east"])])
            .expect("valid edges accepted");
        let error =
            super::validate_population_route_edges(&network, &[record("v", &["west", "ghost"])])
                .expect_err("unknown edge rejected");
        assert!(error.to_string().contains("unknown road edge"), "{error}");
        let error =
            super::validate_population_route_edges(&network, &[record("v", &["west", ":J_0"])])
                .expect_err("internal edge rejected");
        assert!(error.to_string().contains("internal edge"), "{error}");
    }

    #[test]
    fn a0_fixture_proves_real_lane_gap_and_continuous_control() {
        let p1 = || PathEvidence {
            connection: "A/0->B/0".to_owned(),
            path_id: "sumo:J1:A_0-to-B_0::J1_0_0".to_owned(),
            entry: "sumo:A_0".to_owned(),
            internals: vec!["sumo::J1_0_0".to_owned()],
            exit: "sumo:B_0".to_owned(),
        };
        let p2 = || PathEvidence {
            connection: "B/1->C/0".to_owned(),
            path_id: "sumo:J2:B_1-to-C_0::J2_0_0".to_owned(),
            entry: "sumo:B_1".to_owned(),
            internals: vec!["sumo::J2_0_0".to_owned()],
            exit: "sumo:C_0".to_owned(),
        };

        let gap = RouteFixture::new(RouteFixtureKind::Gap);
        let network = gap.network();
        let mut b_lanes = network
            .lanes
            .iter()
            .filter(|lane| lane.edge_id == "B" && !lane.function_internal)
            .map(|lane| lane.index)
            .collect::<Vec<_>>();
        b_lanes.sort_unstable();
        assert_eq!(b_lanes, [0, 1]);
        assert_eq!(network.lanes.len(), 6);
        assert_eq!(network.connections.len(), 4);
        assert!(
            !network.connections.iter().any(|connection| {
                connection.from_edge_id == "B"
                    && connection.to_edge_id == "B"
                    && connection.from_lane == 0
                    && connection.to_lane == 1
            }),
            "B0 must not acquire an implicit lateral connection"
        );
        let topology = gap.normalized_topology(&network);
        assert_eq!(topology.junctions.len(), 2);
        assert_eq!(topology.maneuver_paths.len(), 2);
        assert_eq!(topology.path_by_connection.len(), 2);
        assert_eq!(path_evidence(&topology, "A", 0, "B", 0), p1());
        assert_eq!(path_evidence(&topology, "B", 1, "C", 0), p2());
        assert!(
            !topology
                .path_by_connection
                .keys()
                .any(|(from, from_lane, to, _)| from == "B" && *from_lane == 0 && to == "C"),
            "B0 must not have a path to C in the gap fixture"
        );
        assert!(topology.dropped_stub_lane_ids.is_empty());
        assert!(topology.stub_welds.is_empty());
        assert!(topology.stub_weld_records.is_empty());

        let continuous = RouteFixture::new(RouteFixtureKind::Continuous);
        let network = continuous.network();
        assert_eq!(network.lanes.len(), 7);
        assert_eq!(network.connections.len(), 6);
        let topology = continuous.normalized_topology(&network);
        assert_eq!(topology.junctions.len(), 2);
        assert_eq!(topology.maneuver_paths.len(), 3);
        assert_eq!(topology.path_by_connection.len(), 3);
        assert_eq!(path_evidence(&topology, "A", 0, "B", 0), p1());
        assert_eq!(path_evidence(&topology, "B", 1, "C", 0), p2());
        assert_eq!(
            path_evidence(&topology, "B", 0, "C", 0),
            PathEvidence {
                connection: "B/0->C/0".to_owned(),
                path_id: "sumo:J2:B_0-to-C_0::J2_1_0".to_owned(),
                entry: "sumo:B_0".to_owned(),
                internals: vec!["sumo::J2_1_0".to_owned()],
                exit: "sumo:C_0".to_owned(),
            }
        );
        assert!(topology.dropped_stub_lane_ids.is_empty());
        assert!(topology.stub_welds.is_empty());
        assert!(topology.stub_weld_records.is_empty());
    }

    #[test]
    fn a0_lane_gap_fails_at_route_expansion_with_exact_error() {
        let fixture = RouteFixture::new(RouteFixtureKind::Gap);
        let network = fixture.network();
        let topology = fixture.normalized_topology(&network);
        let population = fixture.selected_population();
        let error = build_routes_and_bind_population(&network, &topology, &population)
            .expect_err("the real B0/B1 gap must fail route expansion");
        assert_eq!(
            error_evidence(error),
            ErrorEvidence {
                variant: "SumoModel",
                message: NO_COMPLETE_ROUTE_ERROR.to_owned(),
            }
        );
    }

    #[test]
    fn a0_continuous_path_materializes_and_binds_population_deterministically() {
        let fixture = RouteFixture::new(RouteFixtureKind::Continuous);
        let network = fixture.network();
        let topology = fixture.normalized_topology(&network);
        let population = fixture.selected_population();
        assert_eq!(population.len(), 2);
        let build = || build_routes_and_bind_population(&network, &topology, &population);
        let first = build().expect("continuous control must materialize");
        let second = build().expect("repeated continuous control must materialize");

        assert_eq!(first.routes.len(), 1);
        assert_eq!(first.routes[0].id, "sumo:route-0");
        assert_eq!(first.routes[0].edge_ids, expected_final_chain());
        assert!(
            !first.routes[0]
                .edge_ids
                .iter()
                .any(|edge| edge == "sumo:B_1" || edge == "sumo::J2_0_0"),
            "the continuous route must not jump to B1"
        );
        assert_eq!(first.records.len(), population.len());
        for (bound, selected) in first.records.iter().zip(&population) {
            assert_eq!(bound.population_rank, selected.population_rank);
            assert_eq!(bound.vehicle_id, selected.vehicle_id);
            assert_eq!(bound.vehicle_profile_id, "sumo:passenger1");
            assert_eq!(bound.depart, selected.depart);
            assert_eq!(bound.route_id, "sumo:route-0");
            assert_eq!(bound.road_edge_ids, selected.road_edge_ids);
            assert_eq!(bound.source_file_ordinal, selected.source_file_ordinal);
            assert_eq!(
                bound.source_vehicle_ordinal,
                selected.source_vehicle_ordinal
            );
        }

        let catalog = |bundle: &RoutePopulationBundle| {
            bundle
                .routes
                .iter()
                .map(|route| (route.id.clone(), route.edge_ids.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(catalog(&first), catalog(&second));
        assert_eq!(first.records, second.records);
    }

    #[test]
    fn a0_source_reference_failures_are_stage_specific() {
        let unknown_lane = RouteFixture::new(RouteFixtureKind::UnknownLane);
        let network = unknown_lane.network();
        let error = normalize_junctions(
            &network,
            &crate::convert::junction::StubWeldGate::Unrestricted,
        )
        .expect_err("unknown connection lane must fail normalization");
        assert_eq!(
            error_evidence(error),
            ErrorEvidence {
                variant: "SumoModel",
                message: "connection references unknown lane edge=\"B\" index=9".to_owned(),
            }
        );

        let fixture = RouteFixture::new(RouteFixtureKind::Gap);
        let network = fixture.network();
        let topology = fixture.normalized_topology(&network);
        let unknown_road = population_record("unknown-road", &["A", "B", "GHOST"]);
        let expected_unknown_road = ErrorEvidence {
            variant: "SumoModel",
            message: "DUE vehicle \"unknown-road\" references unknown road edge \"GHOST\""
                .to_owned(),
        };
        let error = build_routes_and_bind_population(&network, &topology, &[unknown_road])
            .expect_err("unknown road must fail route-edge validation");
        assert_eq!(error_evidence(error), expected_unknown_road);

        let internal_road = population_record("internal-road", &["A", "B", ":J2_0"]);
        let expected_internal_road = ErrorEvidence {
            variant: "SumoModel",
            message: "DUE vehicle \"internal-road\" route references internal edge \":J2_0\""
                .to_owned(),
        };
        let error = build_routes_and_bind_population(&network, &topology, &[internal_road])
            .expect_err("internal DUE road must fail route-edge validation");
        assert_eq!(error_evidence(error), expected_internal_road);

        let reject_in_diagnostic = |roads: &[&str], expected: fn(&str) -> String| {
            let fixture = RouteFixture::new(RouteFixtureKind::Gap);
            let first_id = fixture.selected_population()[0].vehicle_id.clone();
            let fixture = fixture.with_route_override(&first_id, roads);
            let error = fixture
                .combined(&diagnostic_options())
                .expect_err("diagnostic mode must retain route-edge validation");
            assert_eq!(
                error_evidence(error),
                ErrorEvidence {
                    variant: "SumoModel",
                    message: expected(&first_id),
                }
            );
        };
        reject_in_diagnostic(&["A", "B", "GHOST"], |id| {
            format!("DUE vehicle {id:?} references unknown road edge \"GHOST\"")
        });
        reject_in_diagnostic(&["A", "B", ":J2_0"], |id| {
            format!("DUE vehicle {id:?} route references internal edge \":J2_0\"")
        });
    }

    #[test]
    fn a0_missing_road_hop_shares_current_lane_gap_error() {
        let missing_hop = RouteFixture::new(RouteFixtureKind::MissingHop);
        let network = missing_hop.network();
        let topology = missing_hop.normalized_topology(&network);
        assert_eq!(topology.maneuver_paths.len(), 1);
        assert!(
            !topology
                .path_by_connection
                .keys()
                .any(|(from, _, to, _)| from == "B" && to == "C"),
            "the missing-hop fixture must not have a B to C source path"
        );
        let population = missing_hop.selected_population();
        let error = build_routes_and_bind_population(&network, &topology, &population)
            .expect_err("missing road hop must fail route expansion");

        let gap = RouteFixture::new(RouteFixtureKind::Gap);
        let gap_network = gap.network();
        let gap_topology = gap.normalized_topology(&gap_network);
        let gap_population = gap.selected_population();
        let gap_error =
            build_routes_and_bind_population(&gap_network, &gap_topology, &gap_population)
                .expect_err("lane gap must fail route expansion");
        assert_eq!(error_evidence(error), error_evidence(gap_error));
        assert_eq!(
            error_evidence(
                build_routes_and_bind_population(&network, &topology, &population)
                    .expect_err("repeated missing road hop must fail")
            ),
            ErrorEvidence {
                variant: "SumoModel",
                message: NO_COMPLETE_ROUTE_ERROR.to_owned(),
            }
        );
    }
}
