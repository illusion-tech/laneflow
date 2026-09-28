//! Normalize SUMO connections into Junction / Movement / ManeuverPath (§3.1).

use std::collections::{HashMap, HashSet};

use crate::{
    Error, Result,
    output::model::{Junction, ManeuverPath, Movement},
    sumo::{ExactDecimal, SUMO_ID_PREFIX, SumoLane, SumoNetwork},
};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct TraversalKey {
    junction_id: String,
    from_road_edge_id: String,
    to_road_edge_id: String,
    from_lane_index: u32,
    to_lane_index: u32,
    internal_lane_ids: Vec<String>,
}

#[derive(Clone, Debug)]
struct NormalizedTraversal {
    key: TraversalKey,
    entry_lane_id: String,
    exit_lane_id: String,
}

/// Emit Junction / Movement / ManeuverPath aggregates for a SUMO network.
pub fn normalize_junctions(network: &SumoNetwork) -> Result<NormalizedTopology> {
    let lane_by_edge_index = build_lane_index(network);
    let adjacency = build_lane_adjacency(network, &lane_by_edge_index)?;
    let owners_by_int_lane = build_int_lane_owners(network)?;

    let mut dropped_stub_lane_ids = HashSet::new();
    let mut stub_welds: HashMap<String, (ExactDecimal, ExactDecimal)> = HashMap::new();
    let mut traversals = Vec::new();
    for connection in &network.connections {
        let from_edge = network.edge(&connection.from_edge_id).ok_or_else(|| {
            Error::SumoModel(format!(
                "connection from unknown edge {:?}",
                connection.from_edge_id
            ))
        })?;
        let to_edge = network.edge(&connection.to_edge_id).ok_or_else(|| {
            Error::SumoModel(format!(
                "connection to unknown edge {:?}",
                connection.to_edge_id
            ))
        })?;
        if from_edge.function_internal || to_edge.function_internal {
            continue;
        }

        let entry = resolve_lane(
            &lane_by_edge_index,
            &connection.from_edge_id,
            connection.from_lane,
        )?;
        let exit = resolve_lane(
            &lane_by_edge_index,
            &connection.to_edge_id,
            connection.to_lane,
        )?;
        if entry.function_internal || exit.function_internal {
            return Err(Error::SumoModel(format!(
                "external connection {:?}->{:?} resolved to internal lane endpoints",
                connection.from_edge_id, connection.to_edge_id
            )));
        }

        let mut internal_lane_ids = Vec::with_capacity(connection.via_lane_ids.len());
        for via_id in &connection.via_lane_ids {
            let via = network.lane(via_id).ok_or_else(|| {
                Error::SumoModel(format!("connection via references unknown lane {via_id:?}"))
            })?;
            if !via.function_internal {
                return Err(Error::SumoModel(format!(
                    "connection via {via_id:?} is not an internal lane"
                )));
            }
            internal_lane_ids.push(via_id.clone());
        }

        // 点状 stub 内边（LuST 有 84 条，全部在 dir="s" 的直行穿越上）：形状端点距
        // 不足 0.5 m，其弦向是点结处的坐标抖动噪声（实测可达 173° 反向）。这类边在
        // "两端点固定 + 5mm join 容差 + 2° 方向档"下无可行几何表示（位移方向与
        // 行进方向矛盾），唯一出路是从路径中移除并把入口边末点焊接到出口边首点。
        if internal_lane_ids.len() == 1 {
            let stub_id = &internal_lane_ids[0];
            let stub_lane = network.lane(stub_id).expect("via lane checked");
            if lane_shape_span_meters(stub_lane)? < POINT_STUB_MAX_METERS {
                let target = exit.shape.first().ok_or_else(|| {
                    Error::SumoModel(format!("exit lane {:?} has empty shape", exit.id))
                })?;
                match stub_welds.entry(entry.id.clone()) {
                    std::collections::hash_map::Entry::Occupied(previous) => {
                        let previous = previous.get();
                        let dx = previous.0.checked_sub(target.0)?.to_f64()?;
                        let dy = previous.1.checked_sub(target.1)?.to_f64()?;
                        if dx * dx + dy * dy > 1e-6 {
                            return Err(Error::SumoModel(format!(
                                "entry lane {:?} welded to conflicting targets by point-stub removal",
                                entry.id
                            )));
                        }
                    }
                    std::collections::hash_map::Entry::Vacant(slot) => {
                        slot.insert(target.clone());
                    }
                }
                dropped_stub_lane_ids.insert(stub_id.clone());
                internal_lane_ids.clear();
            }
        } else {
            for via_id in &internal_lane_ids {
                let lane = network.lane(via_id).expect("via lane checked");
                if lane_shape_span_meters(lane)? < POINT_STUB_MAX_METERS {
                    return Err(Error::SumoModel(format!(
                        "point-stub internal lane {via_id:?} inside a multi-internal chain is unsupported"
                    )));
                }
            }
        }

        let mut sequence = Vec::with_capacity(internal_lane_ids.len() + 2);
        sequence.push(entry.id.clone());
        sequence.extend(internal_lane_ids.iter().cloned());
        sequence.push(exit.id.clone());
        validate_sequence_connected(&sequence, &adjacency)?;
        validate_no_cycle(&sequence)?;

        let junction_id = resolve_owner(
            network,
            &connection.from_edge_id,
            &connection.to_edge_id,
            &internal_lane_ids,
            &owners_by_int_lane,
        )?;

        traversals.push(NormalizedTraversal {
            key: TraversalKey {
                junction_id,
                from_road_edge_id: connection.from_edge_id.clone(),
                to_road_edge_id: connection.to_edge_id.clone(),
                from_lane_index: connection.from_lane,
                to_lane_index: connection.to_lane,
                internal_lane_ids,
            },
            entry_lane_id: entry.id.clone(),
            exit_lane_id: exit.id.clone(),
        });
    }

    traversals.sort_by(|left, right| left.key.cmp(&right.key));

    let mut signatures = HashSet::new();
    for traversal in &traversals {
        let signature = (
            traversal.entry_lane_id.as_str(),
            traversal.key.internal_lane_ids.as_slice(),
            traversal.exit_lane_id.as_str(),
        );
        if !signatures.insert(signature) {
            return Err(Error::SumoModel(format!(
                "duplicate ManeuverPath traversal signature entry={:?} internals={:?} exit={:?}",
                traversal.entry_lane_id, traversal.key.internal_lane_ids, traversal.exit_lane_id
            )));
        }
    }

    let mut junction_ids = traversals
        .iter()
        .map(|traversal| traversal.key.junction_id.clone())
        .collect::<Vec<_>>();
    junction_ids.sort();
    junction_ids.dedup();

    let junctions = junction_ids
        .iter()
        .map(|id| Junction {
            id: format!("{SUMO_ID_PREFIX}{id}"),
        })
        .collect::<Vec<_>>();

    let mut movements = Vec::new();
    let mut movement_ids = HashSet::new();
    for traversal in &traversals {
        let movement_id = movement_id(
            &traversal.key.junction_id,
            &traversal.key.from_road_edge_id,
            &traversal.key.to_road_edge_id,
        );
        if movement_ids.insert(movement_id.clone()) {
            movements.push(Movement {
                id: movement_id,
                junction_id: format!("{SUMO_ID_PREFIX}{}", traversal.key.junction_id),
                from_road_edge_id: traversal.key.from_road_edge_id.clone(),
                to_road_edge_id: traversal.key.to_road_edge_id.clone(),
            });
        }
    }
    movements.sort_by(|left, right| left.id.cmp(&right.id));

    let mut path_by_connection = HashMap::new();
    let maneuver_paths = traversals
        .iter()
        .map(|traversal| {
            let path = ManeuverPath {
                id: maneuver_path_id(traversal),
                movement_id: movement_id(
                    &traversal.key.junction_id,
                    &traversal.key.from_road_edge_id,
                    &traversal.key.to_road_edge_id,
                ),
                entry_edge_id: format!("{SUMO_ID_PREFIX}{}", traversal.entry_lane_id),
                internal_edge_ids: traversal
                    .key
                    .internal_lane_ids
                    .iter()
                    .map(|id| format!("{SUMO_ID_PREFIX}{id}"))
                    .collect(),
                exit_edge_id: format!("{SUMO_ID_PREFIX}{}", traversal.exit_lane_id),
            };
            path_by_connection.insert(
                (
                    traversal.key.from_road_edge_id.clone(),
                    traversal.key.from_lane_index,
                    traversal.key.to_road_edge_id.clone(),
                    traversal.key.to_lane_index,
                ),
                path.id.clone(),
            );
            path
        })
        .collect::<Vec<_>>();

    if junctions.iter().any(|junction| {
        !movements
            .iter()
            .any(|movement| movement.junction_id == junction.id)
    }) {
        return Err(Error::SumoModel(
            "emitted Junction without Movement after normalization".to_owned(),
        ));
    }
    if movements.iter().any(|movement| {
        !maneuver_paths
            .iter()
            .any(|path| path.movement_id == movement.id)
    }) {
        return Err(Error::SumoModel(
            "emitted Movement without ManeuverPath after normalization".to_owned(),
        ));
    }

    Ok(NormalizedTopology {
        junctions,
        movements,
        maneuver_paths,
        path_by_connection,
        dropped_stub_lane_ids,
        stub_welds,
    })
}

/// 点状 stub 内边的形状端点距上限（米）：LuST 的 84 条 stub 均 ≤ 0.5 m。
const POINT_STUB_MAX_METERS: f64 = 0.5;

/// 内边形状首末点距离（米）。
fn lane_shape_span_meters(lane: &SumoLane) -> Result<f64> {
    let Some((first, rest)) = lane.shape.split_first() else {
        return Ok(0.0);
    };
    let Some(last) = rest.last() else {
        return Ok(0.0);
    };
    let dx = last.0.checked_sub(first.0)?.to_f64()?;
    let dy = last.1.checked_sub(first.1)?.to_f64()?;
    Ok((dx * dx + dy * dy).sqrt())
}

/// Junction normalization output used by topology and signal conversion.
#[derive(Clone, Debug)]
pub struct NormalizedTopology {
    pub junctions: Vec<Junction>,
    pub movements: Vec<Movement>,
    pub maneuver_paths: Vec<ManeuverPath>,
    /// `(from_road_edge, from_lane, to_road_edge, to_lane) -> ManeuverPath.id`
    pub path_by_connection: HashMap<(String, u32, String, u32), String>,
    /// 被移除的点状 stub 内边（SUMO 原始 lane id）：不再出现在 lane graph / spatial。
    pub dropped_stub_lane_ids: HashSet<String>,
    /// stub 移除后的焊接指令：入口 lane 原始 id → 出口 lane 首点（SUMO 原始坐标）。
    pub stub_welds: HashMap<String, (ExactDecimal, ExactDecimal)>,
}

fn resolve_owner(
    network: &SumoNetwork,
    from_road_edge_id: &str,
    to_road_edge_id: &str,
    internal_lane_ids: &[String],
    owners_by_int_lane: &HashMap<&str, &str>,
) -> Result<String> {
    let from_edge = network
        .edge(from_road_edge_id)
        .expect("from road edge checked");
    let to_edge = network.edge(to_road_edge_id).expect("to road edge checked");
    let from_to = from_edge.to_junction_id.as_deref().ok_or_else(|| {
        Error::SumoModel(format!(
            "external from-edge {from_road_edge_id:?} missing @to junction"
        ))
    })?;
    let to_from = to_edge.from_junction_id.as_deref().ok_or_else(|| {
        Error::SumoModel(format!(
            "external to-edge {to_road_edge_id:?} missing @from junction"
        ))
    })?;
    if from_to != to_from {
        return Err(Error::SumoModel(format!(
            "junction owner mismatch: from-edge {from_road_edge_id:?}@to={from_to:?} \
             vs to-edge {to_road_edge_id:?}@from={to_from:?}"
        )));
    }

    let mut int_owners = HashSet::new();
    for lane_id in internal_lane_ids {
        // 未被任何 @intLanes 认领的内边不参与交叉校验：SUMO 里存在只出现在
        // 内部节点 @incLanes 的内边（如 :-24112_2_0），归属以穿越端点为准。
        if let Some(owner) = owners_by_int_lane.get(lane_id.as_str()) {
            int_owners.insert(*owner);
        }
    }
    if !int_owners.is_empty() {
        if int_owners.len() != 1 {
            return Err(Error::SumoModel(format!(
                "internal lanes {:?} span multiple junction owners {:?}",
                internal_lane_ids, int_owners
            )));
        }
        let int_owner = int_owners.into_iter().next().expect("one owner");
        if int_owner != from_to {
            return Err(Error::SumoModel(format!(
                "intLanes owner {int_owner:?} disagrees with edge endpoints {from_to:?}"
            )));
        }
    }

    let junction = network
        .junction(from_to)
        .ok_or_else(|| Error::SumoModel(format!("unknown junction owner {from_to:?}")))?;
    if !junction.can_own_road_junction() {
        return Err(Error::SumoModel(format!(
            "junction {from_to:?} type {:?} cannot own road ManeuverPath traversals",
            junction.junction_type
        )));
    }
    Ok(from_to.to_owned())
}

fn build_int_lane_owners(network: &SumoNetwork) -> Result<HashMap<&str, &str>> {
    // SUMO 路口簇模型（LuST 有 1855 个 type="internal" 内部节点）：
    // - road junction 的 @intLanes 直接列出部分内边，同时把内部节点 id
    //   以"lane 同名"的形式一并列出；
    // - 其余内边只被内部节点列出（如 :-11042_13_0 仅见于 :-11042_14_0 /
    //   :-11042_16_0 的 @intLanes，road junction -11042 并不列它）。
    // 归属权威只在 road junction：内部节点先归并到簇，再间接解析内边归属。
    let is_internal = |id: &str| {
        network
            .junction(id)
            .is_some_and(|junction| junction.junction_type == "internal")
    };

    // 第一遍：road junction 直接列出的内边 + 内部节点 → 簇的父映射种子。
    let mut owners: HashMap<&str, &str> = HashMap::new();
    let mut cluster_parent: HashMap<&str, &str> = HashMap::new();
    for junction in &network.junctions {
        if !junction.can_own_road_junction() {
            continue;
        }
        for lane_id in &junction.int_lane_ids {
            if is_internal(lane_id) {
                cluster_parent.insert(lane_id.as_str(), junction.id.as_str());
            }
            if let Some(previous) = owners.insert(lane_id.as_str(), junction.id.as_str()) {
                return Err(Error::SumoModel(format!(
                    "internal lane {lane_id:?} listed in both junction {previous:?} and {:?}",
                    junction.id
                )));
            }
        }
    }

    // 第二遍：嵌套内部节点的父映射不动点（内部节点再列出内部节点）。
    loop {
        let mut changed = false;
        for junction in &network.junctions {
            if junction.junction_type != "internal" {
                continue;
            }
            let Some(&parent) = cluster_parent.get(junction.id.as_str()) else {
                continue;
            };
            for lane_id in &junction.int_lane_ids {
                if is_internal(lane_id) && !cluster_parent.contains_key(lane_id.as_str()) {
                    cluster_parent.insert(lane_id.as_str(), parent);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    // 第三遍：只被内部节点列出的内边，经簇父映射间接归并到 road junction。
    for junction in &network.junctions {
        if junction.junction_type != "internal" {
            continue;
        }
        let Some(&parent) = cluster_parent.get(junction.id.as_str()) else {
            continue;
        };
        for lane_id in &junction.int_lane_ids {
            if is_internal(lane_id) {
                continue;
            }
            match owners.insert(lane_id.as_str(), parent) {
                Some(previous) if previous != parent => {
                    return Err(Error::SumoModel(format!(
                        "internal lane {lane_id:?} resolves to conflicting junction owners \
                         {previous:?} and {parent:?} (via internal junction {:?})",
                        junction.id
                    )));
                }
                _ => {}
            }
        }
    }
    Ok(owners)
}

fn build_lane_adjacency(
    network: &SumoNetwork,
    lane_by_edge_index: &HashMap<(String, u32), &SumoLane>,
) -> Result<HashMap<String, HashSet<String>>> {
    let mut adjacency: HashMap<String, HashSet<String>> = HashMap::new();
    for connection in &network.connections {
        let from_lane = resolve_lane(
            lane_by_edge_index,
            &connection.from_edge_id,
            connection.from_lane,
        )?;
        let to_lane = resolve_lane(
            lane_by_edge_index,
            &connection.to_edge_id,
            connection.to_lane,
        )?;
        let mut chain = Vec::with_capacity(connection.via_lane_ids.len() + 2);
        chain.push(from_lane.id.clone());
        for via_id in &connection.via_lane_ids {
            if network.lane(via_id).is_none() {
                return Err(Error::SumoModel(format!(
                    "connection via references unknown lane {via_id:?}"
                )));
            }
            chain.push(via_id.clone());
        }
        chain.push(to_lane.id.clone());
        for window in chain.windows(2) {
            adjacency
                .entry(window[0].clone())
                .or_default()
                .insert(window[1].clone());
        }
        // 点状 stub 穿越在 normalize_junctions 中被移除并焊接为 from→to 直达，
        // 邻接表需同步提供该直达边供 validate_sequence_connected 使用。
        if connection.via_lane_ids.len() == 1 {
            let via = network.lane(&connection.via_lane_ids[0]).ok_or_else(|| {
                Error::SumoModel(format!(
                    "connection via references unknown lane {:?}",
                    connection.via_lane_ids[0]
                ))
            })?;
            if lane_shape_span_meters(via)? < POINT_STUB_MAX_METERS {
                adjacency
                    .entry(from_lane.id.clone())
                    .or_default()
                    .insert(to_lane.id.clone());
            }
        }
    }
    Ok(adjacency)
}

fn validate_sequence_connected(
    sequence: &[String],
    adjacency: &HashMap<String, HashSet<String>>,
) -> Result<()> {
    for window in sequence.windows(2) {
        let connected = adjacency
            .get(&window[0])
            .is_some_and(|next| next.contains(&window[1]));
        if !connected {
            return Err(Error::SumoModel(format!(
                "ManeuverPath sequence is not connected between {:?} and {:?}",
                window[0], window[1]
            )));
        }
    }
    Ok(())
}

fn validate_no_cycle(sequence: &[String]) -> Result<()> {
    let mut seen = HashSet::new();
    for lane_id in sequence {
        if !seen.insert(lane_id.as_str()) {
            return Err(Error::SumoModel(format!(
                "ManeuverPath sequence contains a cycle at {lane_id:?}"
            )));
        }
    }
    Ok(())
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

fn movement_id(junction_id: &str, from_road_edge_id: &str, to_road_edge_id: &str) -> String {
    format!("{SUMO_ID_PREFIX}{junction_id}:{from_road_edge_id}-to-{to_road_edge_id}")
}

fn maneuver_path_id(traversal: &NormalizedTraversal) -> String {
    let internals = if traversal.key.internal_lane_ids.is_empty() {
        "direct".to_owned()
    } else {
        traversal.key.internal_lane_ids.join(".")
    };
    format!(
        "{SUMO_ID_PREFIX}{}:{}-to-{}:{internals}",
        traversal.key.junction_id, traversal.entry_lane_id, traversal.exit_lane_id
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sumo::parse_sumo_network_xml;

    #[test]
    fn fixture_emits_one_junction_two_movements_two_paths() {
        let xml = include_str!("../../tests/fixtures/minimal/t-junction.net.xml");
        let network = parse_sumo_network_xml(xml).expect("parse");
        let topology = normalize_junctions(&network).expect("normalize");
        assert_eq!(topology.junctions.len(), 1);
        assert_eq!(topology.junctions[0].id, "sumo:J");
        assert_eq!(topology.movements.len(), 2);
        assert_eq!(topology.maneuver_paths.len(), 2);
        assert!(
            topology
                .maneuver_paths
                .iter()
                .any(|path| path.internal_edge_ids == ["sumo::J_0_0".to_owned()])
        );
        assert!(
            topology
                .maneuver_paths
                .iter()
                .any(|path| path.internal_edge_ids == ["sumo::J_1_0".to_owned()])
        );
    }

    #[test]
    fn dangling_via_fails_closed() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/></edge>
  <junction id="J" type="priority" intLanes=":J_0_0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":missing_0"/>
</net>"#;
        let network = parse_sumo_network_xml(xml).expect("parse");
        let error = normalize_junctions(&network).expect_err("dangling via");
        assert!(error.to_string().contains("unknown lane"));
    }
}
