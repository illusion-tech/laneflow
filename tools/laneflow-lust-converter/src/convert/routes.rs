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

/// #237 最小换道（S1）：过渡材料来源保真。经授权规则派生的编制材料必须携带
/// 来源映射与构造依据，不得冒充原 SUMO connection。
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug)]
pub(crate) enum TransitionProvenance {
    /// 合成来源原生声明。
    Native,
    /// 派生材料（来源映射 + 构造依据，两项均非空）。
    Derived {
        source_mapping: String,
        construction_basis: String,
    },
}

/// 声明的固定预制过渡：**仅声明边界**允许来源 lane 转移（§3.1 契约）。
/// 空声明集下展开行为与既有严格规则逐字节一致（旧输入零新拒绝）。
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug)]
pub(crate) struct TransitionDeclaration {
    pub id: String,
    pub from_road: String,
    pub to_road: String,
    pub from_lane: u32,
    pub to_lane: u32,
    /// 最终拓扑中的真实过渡边链（如 `["sumo::T01"]`）；物化时插在来源 lane
    /// 段与目标 lane 段之间。
    pub final_edge_chain: Vec<String>,
    pub provenance: TransitionProvenance,
}

/// 已校验的过渡声明索引（校验失败关闭）。
#[derive(Default)]
pub(crate) struct TransitionIndex {
    by_boundary: HashMap<(String, String, u32, u32), TransitionDeclaration>,
}

impl TransitionIndex {
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn build(
        declarations: &[TransitionDeclaration],
        network: &SumoNetwork,
    ) -> Result<Self> {
        let mut index = Self::empty();
        // 材质化拓扑基础索引（P2-3）：已知 lane id 全集 + 连接相邻对。
        // wire id 公式对 external 与 internal lane 统一（internal 边 id 自带
        // ':' 前缀，形成 "sumo::…" 形态，与物化输出一致）。
        let wire_id =
            |edge_id: &str, lane_index: u32| format!("{SUMO_ID_PREFIX}{edge_id}_{lane_index}");
        let mut known = std::collections::HashSet::new();
        for lane in &network.lanes {
            known.insert(wire_id(&lane.edge_id, lane.index));
        }
        let mut connected = std::collections::HashSet::new();
        for connection in &network.connections {
            connected.insert((
                wire_id(&connection.from_edge_id, connection.from_lane),
                wire_id(&connection.to_edge_id, connection.to_lane),
            ));
        }
        for declaration in declarations {
            if declaration.from_lane == declaration.to_lane {
                return Err(Error::SumoModel(format!(
                    "transition {} must transfer between distinct lanes",
                    declaration.id
                )));
            }
            if declaration.final_edge_chain.is_empty() {
                return Err(Error::SumoModel(format!(
                    "transition {} must declare a non-empty final edge chain",
                    declaration.id
                )));
            }
            let derived_incomplete = match &declaration.provenance {
                TransitionProvenance::Derived {
                    source_mapping,
                    construction_basis,
                } => source_mapping.trim().is_empty() || construction_basis.trim().is_empty(),
                TransitionProvenance::Native => false,
            };
            if derived_incomplete {
                return Err(Error::SumoModel(format!(
                    "derived transition {} must record source mapping and construction basis",
                    declaration.id
                )));
            }
            // 材质化拓扑校验（存在性 + 连通性含两侧桥接）：声明链必须真实落在
            // 最终拓扑上；未知或不连通的命名不得被视为可用（P2-3 失败关闭）。
            let from_id = lane_laneflow_id(network, &declaration.from_road, declaration.from_lane)
                .map_err(|_| {
                    Error::SumoModel(format!(
                        "transition {} source lane {}/{} is unknown in the materialized topology",
                        declaration.id, declaration.from_road, declaration.from_lane
                    ))
                })?;
            let to_id = lane_laneflow_id(network, &declaration.from_road, declaration.to_lane)
                .map_err(|_| {
                    Error::SumoModel(format!(
                        "transition {} target lane {}/{} is unknown in the materialized topology",
                        declaration.id, declaration.from_road, declaration.to_lane
                    ))
                })?;
            let mut previous = from_id.clone();
            for edge in &declaration.final_edge_chain {
                if !known.contains(edge.as_str()) {
                    return Err(Error::SumoModel(format!(
                        "transition {} final edge chain references unknown lane {:?}",
                        declaration.id, edge
                    )));
                }
                if !connected.contains(&(previous.clone(), edge.clone())) {
                    return Err(Error::SumoModel(format!(
                        "transition {} final edge chain is not connected: {:?} -> {:?}",
                        declaration.id, previous, edge
                    )));
                }
                previous = edge.clone();
            }
            if !connected.contains(&(previous.clone(), to_id.clone())) {
                return Err(Error::SumoModel(format!(
                    "transition {} final edge chain is not connected to target lane: {:?} -> {:?}",
                    declaration.id, previous, to_id
                )));
            }
            let key = (
                declaration.from_road.clone(),
                declaration.to_road.clone(),
                declaration.from_lane,
                declaration.to_lane,
            );
            if index.by_boundary.insert(key, declaration.clone()).is_some() {
                return Err(Error::SumoModel(format!(
                    "duplicate transition declaration {:?} at road pair {:?} -> {:?} lane {} -> {}",
                    declaration.id,
                    declaration.from_road,
                    declaration.to_road,
                    declaration.from_lane,
                    declaration.to_lane
                )));
            }
        }
        Ok(index)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.by_boundary.is_empty()
    }

    /// 声明边界处允许「当前 lane `current_lane` 转移到 hop 出发 lane
    /// `hop_from_lane` 后继续该 hop」；无声明返回 `None`（严格规则）。
    fn allows(
        &self,
        from_road: &str,
        to_road: &str,
        current_lane: u32,
        hop_from_lane: u32,
    ) -> Option<&TransitionDeclaration> {
        self.by_boundary.get(&(
            from_road.to_owned(),
            to_road.to_owned(),
            current_lane,
            hop_from_lane,
        ))
    }
}

/// 展开入选人口道路路线并按相同 lane 序列去重（无过渡声明；旧行为不变）。
pub(crate) fn build_routes_and_bind_population(
    network: &SumoNetwork,
    topology: &NormalizedTopology,
    population: &[PopulationRecord],
) -> Result<RoutePopulationBundle> {
    build_routes_and_bind_population_with_transitions(
        network,
        topology,
        population,
        &TransitionIndex::empty(),
    )
}

/// 过渡感知展开入口（S1）：仅声明边界允许来源 lane 转移；声明为转移路径的
/// **次选**——严格语义失败时才启用（不引入新偏好，旧结论不变）。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn build_routes_and_bind_population_with_transitions(
    network: &SumoNetwork,
    topology: &NormalizedTopology,
    population: &[PopulationRecord],
    transitions: &TransitionIndex,
) -> Result<RoutePopulationBundle> {
    let hops = build_hop_index(topology);
    let lanes_by_edge = build_external_lanes_by_edge(network);

    let mut routes = Vec::new();
    let mut route_id_by_lane_seq: HashMap<Vec<String>, String> = HashMap::new();
    let mut records = Vec::with_capacity(population.len());

    for record in population {
        validate_known_edges(network, &record.road_edge_ids, &record.vehicle_id)?;
        let lane_edge_ids = expand_road_route(
            network,
            &lanes_by_edge,
            &hops,
            &record.road_edge_ids,
            transitions,
        )?;
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
    /// S1 新增观察点（只增不改）：经声明边界的转移 hop。
    fn transition_hop(&mut self, _occurrence: usize, _declaration: &TransitionDeclaration) {}
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
    transitions: &TransitionIndex,
) -> Result<Vec<String>> {
    expand_road_route_with_observer(
        network,
        lanes_by_edge,
        hops,
        road_edge_ids,
        transitions,
        &mut NoopObserver,
    )
}

/// 两相展开（S1 选择语义，裁定②）：相 1 严格语义（空过渡索引，与既有行为
/// 逐字节一致、旧结论不变）；仅当相 1 失败且存在声明时启用相 2（声明转移）。
/// 相 2 失败仍返回相 1 的错误文本（错误语义保持）。
fn expand_road_route_with_observer(
    network: &SumoNetwork,
    lanes_by_edge: &HashMap<&str, Vec<u32>>,
    hops: &HashMap<(String, String), Vec<HopOption>>,
    road_edge_ids: &[String],
    transitions: &TransitionIndex,
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

    let strict = TransitionIndex::empty();
    match search_and_materialize(
        network,
        lanes_by_edge,
        hops,
        road_edge_ids,
        &strict,
        observer,
    ) {
        Ok(chain) => Ok(chain),
        Err(strict_error) if !transitions.is_empty() => search_and_materialize(
            network,
            lanes_by_edge,
            hops,
            road_edge_ids,
            transitions,
            observer,
        )
        .map_err(|_| strict_error),
        Err(strict_error) => Err(strict_error),
    }
}

fn search_and_materialize(
    network: &SumoNetwork,
    lanes_by_edge: &HashMap<&str, Vec<u32>>,
    hops: &HashMap<(String, String), Vec<HopOption>>,
    road_edge_ids: &[String],
    transitions: &TransitionIndex,
    observer: &mut impl RouteObserver,
) -> Result<Vec<String>> {
    let mut memo: HashMap<(usize, u32), bool> = HashMap::new();
    let start_lanes = lanes_by_edge
        .get(road_edge_ids[0].as_str())
        .map(Vec::as_slice)
        .unwrap_or_default();
    for &start_lane in start_lanes {
        if can_complete_with_observer(
            lanes_by_edge,
            hops,
            road_edge_ids,
            0,
            start_lane,
            transitions,
            &mut memo,
            observer,
        ) {
            return materialize_path_with_observer(
                network,
                lanes_by_edge,
                hops,
                road_edge_ids,
                start_lane,
                transitions,
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
                    expand_road_route(
                        network,
                        &lanes_by_edge,
                        &hops,
                        &record.road_edge_ids,
                        &TransitionIndex::empty(),
                    )
                })
                .err()
                .map(|error| (record.vehicle_id.clone(), error.to_string()))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn can_complete_with_observer(
    lanes_by_edge: &HashMap<&str, Vec<u32>>,
    hops: &HashMap<(String, String), Vec<HopOption>>,
    road_edge_ids: &[String],
    edge_index: usize,
    lane_index: u32,
    transitions: &TransitionIndex,
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
            .filter(|hop| {
                hop.from_lane == lane_index
                    || transitions
                        .allows(from, to, lane_index, hop.from_lane)
                        .is_some()
            })
            .any(|hop| {
                observer.hop(edge_index, hop);
                can_complete_with_observer(
                    lanes_by_edge,
                    hops,
                    road_edge_ids,
                    edge_index + 1,
                    hop.to_lane,
                    transitions,
                    memo,
                    &mut *observer,
                )
            })
    };
    observer.state(edge_index, lane_index, result, false);
    memo.insert((edge_index, lane_index), result);
    result
}

#[allow(clippy::too_many_arguments)]
fn materialize_path_with_observer(
    network: &SumoNetwork,
    lanes_by_edge: &HashMap<&str, Vec<u32>>,
    hops: &HashMap<(String, String), Vec<HopOption>>,
    road_edge_ids: &[String],
    start_lane: u32,
    transitions: &TransitionIndex,
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

        // 选择语义（裁定②）：严格 hop 优先；仅严格候选全部不可完成时才尝试
        // 声明转移 hop——不引入新偏好，旧结论不变。
        let mut chosen: Option<(&HopOption, Option<&TransitionDeclaration>)> = None;
        for hop in options.iter().filter(|hop| hop.from_lane == current_lane) {
            if can_complete_with_observer(
                lanes_by_edge,
                hops,
                road_edge_ids,
                edge_index + 1,
                hop.to_lane,
                transitions,
                memo,
                &mut *observer,
            ) {
                chosen = Some((hop, None));
                break;
            }
        }
        if chosen.is_none() {
            for hop in options.iter().filter(|hop| hop.from_lane != current_lane) {
                let Some(declaration) = transitions.allows(from, to, current_lane, hop.from_lane)
                else {
                    continue;
                };
                if can_complete_with_observer(
                    lanes_by_edge,
                    hops,
                    road_edge_ids,
                    edge_index + 1,
                    hop.to_lane,
                    transitions,
                    memo,
                    &mut *observer,
                ) {
                    chosen = Some((hop, Some(declaration)));
                    break;
                }
            }
        }
        let (hop, transition) = chosen.ok_or_else(|| {
            Error::SumoModel(format!(
                "ambiguous or incomplete ManeuverPath expansion at {from:?} lane {current_lane} -> {to:?}"
            ))
        })?;

        if let Some(declaration) = transition {
            observer.transition_hop(edge_index, declaration);
            // 声明转移的物化段：真实过渡边链 + 目标 lane 段（来源保真由
            // 声明 provenance 承担；不得冒充原 SUMO connection）。
            for edge in &declaration.final_edge_chain {
                lane_edge_ids.push(edge.clone());
            }
            lane_edge_ids.push(lane_laneflow_id(network, from, hop.from_lane)?);
        }
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

    // ---- S1（#1194）：过渡声明与边界转移（只增不改） ----

    /// Gap 网络 + 固定预制过渡段（`T01` 0→1、`T10` 1→0）与真实连接
    /// （`B→T01→B`、`B→T10→B`）：声明链的材质化拓扑校验（存在性 + 连通性
    /// 含两侧桥接）以此为基（P2-3）。`include_p0` 提供严格成功对照。
    fn transition_net_xml(include_p0: bool) -> String {
        let i0_edge = if include_p0 {
            "  <edge id=\":J2_1\" function=\"internal\"><lane id=\":J2_1_0\" index=\"0\" speed=\"13.89\" length=\"4.00\" shape=\"6836.88,5727.52 6840.88,5727.52\"/></edge>\n"
        } else {
            ""
        };
        let j2_int = if include_p0 {
            ":J2_0_0 :J2_1_0"
        } else {
            ":J2_0_0"
        };
        let p0_connections = if include_p0 {
            "  <connection from=\"B\" to=\"C\" fromLane=\"0\" toLane=\"0\" via=\":J2_1_0\"/>\n  <connection from=\":J2_1\" to=\"C\" fromLane=\"0\" toLane=\"0\"/>\n"
        } else {
            ""
        };
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<net version="1.16">
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="A" from="X" to="J1"><lane id="A_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="B" from="J1" to="J2"><lane id="B_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/><lane id="B_1" index="1" speed="13.89" length="20.00" shape="6816.88,5730.52 6836.88,5730.52"/></edge>
  <edge id="C" from="J2" to="Z"><lane id="C_0" index="0" speed="13.89" length="20.00" shape="6840.88,5727.52 6860.88,5727.52"/></edge>
  <edge id="T01" from="J2" to="J1"><lane id="T01_0" index="0" speed="13.89" length="9.49" shape="6836.88,5727.52 6845.88,5730.52"/></edge>
  <edge id="T10" from="J2" to="J1"><lane id="T10_0" index="0" speed="13.89" length="9.49" shape="6836.88,5730.52 6845.88,5727.52"/></edge>
  <edge id=":J1_0" function="internal"><lane id=":J1_0_0" index="0" speed="13.89" length="10.00" shape="6806.88,5727.52 6816.88,5727.52"/></edge>
  <edge id=":J2_0" function="internal"><lane id=":J2_0_0" index="0" speed="13.89" length="5.00" shape="6836.88,5730.52 6840.88,5727.52"/></edge>
{i0_edge}  <junction id="X" type="dead_end" incLanes="" intLanes=""/>
  <junction id="Z" type="dead_end" incLanes="C_0" intLanes=""/>
  <junction id="J1" type="priority" incLanes="A_0 T01_0 T10_0" intLanes=":J1_0_0"/>
  <junction id="J2" type="priority" incLanes="B_0 B_1" intLanes="{j2_int}"/>
  <connection from="A" to="B" fromLane="0" toLane="0" via=":J1_0_0"/>
  <connection from=":J1_0" to="B" fromLane="0" toLane="0"/>
  <connection from="B" to="C" fromLane="1" toLane="0" via=":J2_0_0"/>
  <connection from=":J2_0" to="C" fromLane="0" toLane="0"/>
{p0_connections}  <connection from="B" to="T01" fromLane="0" toLane="0"/>
  <connection from="T01" to="B" fromLane="0" toLane="1"/>
  <connection from="B" to="T10" fromLane="1" toLane="0"/>
  <connection from="T10" to="B" fromLane="0" toLane="0"/>
</net>
"#
        )
    }

    fn transition_fixture(include_p0: bool) -> (SumoNetwork, NormalizedTopology) {
        let network = parse_sumo_network_xml(&transition_net_xml(include_p0))
            .expect("parse transition fixture");
        let topology = normalize_junctions(
            &network,
            &crate::convert::junction::StubWeldGate::Unrestricted,
        )
        .expect("normalize transition fixture");
        (network, topology)
    }

    fn s1_transition_declaration() -> TransitionDeclaration {
        TransitionDeclaration {
            id: "t01".to_owned(),
            from_road: "B".to_owned(),
            to_road: "C".to_owned(),
            from_lane: 0,
            to_lane: 1,
            final_edge_chain: vec!["sumo:T01_0".to_owned()],
            provenance: TransitionProvenance::Native,
        }
    }

    fn s1_expected_transition_chain() -> Vec<String> {
        [
            "sumo:A_0",
            "sumo::J1_0_0",
            "sumo:B_0",
            "sumo:T01_0",
            "sumo:B_1",
            "sumo::J2_0_0",
            "sumo:C_0",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }

    #[test]
    fn s1_empty_declaration_index_allows_no_transfer() {
        let empty = TransitionIndex::empty();
        assert!(empty.is_empty());
        assert!(empty.allows("B", "C", 0, 1).is_none());
    }

    #[test]
    fn s1_transition_declaration_validation_fails_closed() {
        let (network, _) = transition_fixture(false);
        let same_lane = TransitionDeclaration {
            from_lane: 1,
            to_lane: 1,
            ..s1_transition_declaration()
        };
        assert!(TransitionIndex::build(std::slice::from_ref(&same_lane), &network).is_err());

        let empty_chain = TransitionDeclaration {
            final_edge_chain: Vec::new(),
            ..s1_transition_declaration()
        };
        assert!(TransitionIndex::build(std::slice::from_ref(&empty_chain), &network).is_err());

        let derived_missing = TransitionDeclaration {
            provenance: TransitionProvenance::Derived {
                source_mapping: String::new(),
                construction_basis: "construction basis".to_owned(),
            },
            ..s1_transition_declaration()
        };
        assert!(TransitionIndex::build(std::slice::from_ref(&derived_missing), &network).is_err());

        let derived_full = TransitionDeclaration {
            provenance: TransitionProvenance::Derived {
                source_mapping: "sumo connection (B,0)->(C,1) 映射说明".to_owned(),
                construction_basis: "授权规则派生：合成来源原生声明".to_owned(),
            },
            ..s1_transition_declaration()
        };
        assert!(TransitionIndex::build(std::slice::from_ref(&derived_full), &network).is_ok());

        let duplicate = vec![s1_transition_declaration(), s1_transition_declaration()];
        assert!(TransitionIndex::build(&duplicate, &network).is_err());

        // P2-3 负例：未知链边、断链、桥接不连通均失败关闭。
        let unknown_chain = TransitionDeclaration {
            final_edge_chain: vec!["sumo::T99".to_owned()],
            ..s1_transition_declaration()
        };
        assert!(TransitionIndex::build(std::slice::from_ref(&unknown_chain), &network).is_err());

        let broken_chain = TransitionDeclaration {
            final_edge_chain: vec!["sumo:T01_0".to_owned(), "sumo:T10_0".to_owned()],
            ..s1_transition_declaration()
        };
        assert!(TransitionIndex::build(std::slice::from_ref(&broken_chain), &network).is_err());

        let wrong_bridge = TransitionDeclaration {
            final_edge_chain: vec!["sumo:T10_0".to_owned()],
            ..s1_transition_declaration()
        };
        assert!(TransitionIndex::build(std::slice::from_ref(&wrong_bridge), &network).is_err());
    }

    #[test]
    fn s1_declared_transition_materializes_crossing_chain() {
        let (network, topology) = transition_fixture(false);
        let population = vec![population_record("selected-z", &["A", "B", "C"])];
        let transitions =
            TransitionIndex::build(std::slice::from_ref(&s1_transition_declaration()), &network)
                .expect("valid declaration");
        let bundle = build_routes_and_bind_population_with_transitions(
            &network,
            &topology,
            &population,
            &transitions,
        )
        .expect("declared transition materializes the crossing chain");
        assert_eq!(bundle.routes.len(), 1);
        assert_eq!(bundle.routes[0].edge_ids, s1_expected_transition_chain());
    }

    #[test]
    fn s1_undeclared_boundary_keeps_strict_failure_and_error_text() {
        let (network, topology) = transition_fixture(false);
        let population = vec![population_record("selected-z", &["A", "B", "C"])];
        let strict = build_routes_and_bind_population(&network, &topology, &population)
            .expect_err("gap without declarations fails exactly as before");
        // 有效但无关的声明（B→C 1->0，经 T10）：B→C 缺口仍失败，错误文本不变。
        let elsewhere = TransitionIndex::build(
            std::slice::from_ref(&TransitionDeclaration {
                id: "t10".to_owned(),
                from_road: "B".to_owned(),
                to_road: "C".to_owned(),
                from_lane: 1,
                to_lane: 0,
                final_edge_chain: vec!["sumo:T10_0".to_owned()],
                provenance: TransitionProvenance::Native,
            }),
            &network,
        )
        .expect("valid declaration");
        let with_elsewhere = build_routes_and_bind_population_with_transitions(
            &network,
            &topology,
            &population,
            &elsewhere,
        )
        .expect_err("lane gap at B->C persists with unrelated declarations");
        let strict_evidence = error_evidence(strict);
        let elsewhere_evidence = error_evidence(with_elsewhere);
        assert_eq!(strict_evidence, elsewhere_evidence);
        assert_eq!(
            strict_evidence,
            ErrorEvidence {
                variant: "SumoModel",
                message: NO_COMPLETE_ROUTE_ERROR.to_owned(),
            }
        );
    }

    #[test]
    fn s1_declaration_never_changes_strict_success() {
        let (network, topology) = transition_fixture(true);
        let population = vec![population_record("selected-z", &["A", "B", "C"])];
        let transitions =
            TransitionIndex::build(std::slice::from_ref(&s1_transition_declaration()), &network)
                .expect("valid declaration");
        let bundle = build_routes_and_bind_population_with_transitions(
            &network,
            &topology,
            &population,
            &transitions,
        )
        .expect("strict success must remain reachable with declarations present");
        assert_eq!(bundle.routes[0].edge_ids, expected_final_chain());
    }
}
