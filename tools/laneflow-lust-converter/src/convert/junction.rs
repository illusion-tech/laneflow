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
    let continuations = internal_continuations(network);
    let terminals = internal_terminals(network);
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

        // internal junction 续链：netconvert 对 internal junction（等待位置）拆分的
        // movement 编码为外部 connection 的 via 仅含首段 + internal <connection> 续接
        // （SUMO 文档：该 movement 被拆成等待位置前后两段内边，续接 connection 的
        // to 与外部 to 相同、via 为下一段）。外部 via 列表因此不是完整链；先续全
        // 再进 stub 处置。点状 stub 若为首段且续链后带真续接段，删焊会吞掉有真实
        // 几何的续接段，fail-closed（R1 焊接门控只授权恰一条 stub 的删焊）。
        internal_lane_ids = extend_internal_chain(
            &continuations,
            &terminals,
            internal_lane_ids,
            &connection.to_edge_id,
            connection.to_lane,
        )?;

        // 点状 stub 内边（LuST 有 84 条，全部在 dir="s" 的直行穿越上）：形状端点距
        // 不足 0.5 m，其弦向是点结处的坐标抖动噪声（实测可达 173° 反向）。这类边在
        // "两端点固定 + 5mm join 容差 + 2° 方向档"下无可行几何表示（位移方向与
        // 行进方向矛盾），唯一出路是从路径中移除并把入口边末点焊接到出口边首点。
        // 判定看续链后的首段：stub 链带真续接段时删焊会吞掉真实几何，fail-closed；
        // 恰为单段 stub 时按焊接位移门控（STUB_WELD_MAX_DISPLACEMENT_M）决定删焊
        // 或 fail-closed；其余链含 stub 即 fail-closed。
        let first_is_stub = internal_lane_ids.first().is_some_and(|id| {
            lane_shape_span_meters(network.lane(id).expect("via lane checked"))
                .is_ok_and(|span| span < POINT_STUB_MAX_METERS)
        });
        if connection.via_lane_ids.len() == 1 && first_is_stub {
            let stub_id = &internal_lane_ids[0];
            let stub_lane = network.lane(stub_id).expect("via lane checked");
            if lane_shape_span_meters(stub_lane)? < POINT_STUB_MAX_METERS {
                if internal_lane_ids.len() > 1 {
                    return Err(Error::SumoModel(format!(
                        "point-stub internal lane {stub_id:?} with continuation segments \
                         is unsupported: dropping it would swallow real continuation geometry"
                    )));
                }
                let target = exit.shape.first().ok_or_else(|| {
                    Error::SumoModel(format!("exit lane {:?} has empty shape", exit.id))
                })?;
                let entry_end = entry.shape.last().ok_or_else(|| {
                    Error::SumoModel(format!("entry lane {:?} has empty shape", entry.id))
                })?;
                let dx = target.0.checked_sub(entry_end.0)?.to_f64()?;
                let dy = target.1.checked_sub(entry_end.1)?.to_f64()?;
                let displacement = (dx * dx + dy * dy).sqrt();
                if displacement > STUB_WELD_MAX_DISPLACEMENT_M {
                    return Err(Error::SumoModel(format!(
                        "point-stub removal would displace entry lane {:?} by \
                         {displacement:.4} m (> {STUB_WELD_MAX_DISPLACEMENT_M} m) \
                         to reach exit lane {:?} start",
                        entry.id, exit.id
                    )));
                }
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
                        slot.insert(*target);
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

/// internal junction 续接候选列表：每条候选为 (to_edge, to_lane, via)。
type ContinuationCandidates = Vec<(String, u32, Vec<String>)>;

/// internal junction 续接（#253 R1/R5）：from-lane → 候选 (to_edge, to_lane, via)。
/// internal <connection> 的 from 为 internal 边。
fn internal_continuations(network: &SumoNetwork) -> HashMap<String, ContinuationCandidates> {
    let mut map: HashMap<String, ContinuationCandidates> = HashMap::new();
    for connection in &network.connections {
        if !connection.from_edge_id.starts_with(':') {
            continue;
        }
        let from_lane = format!("{}_{}", connection.from_edge_id, connection.from_lane);
        map.entry(from_lane).or_default().push((
            connection.to_edge_id.clone(),
            connection.to_lane,
            connection.via_lane_ids.clone(),
        ));
    }
    map
}

/// 无 via 的终端 exit-link（SUMO 要求的 `[from=:v to=:t]` 伴随 connection），
/// 以完整目的 lane 身份 (from_lane, to_edge, to_lane) 索引。
fn internal_terminals(network: &SumoNetwork) -> HashSet<(String, String, u32)> {
    network
        .connections
        .iter()
        .filter(|connection| {
            connection.from_edge_id.starts_with(':') && connection.via_lane_ids.is_empty()
        })
        .map(|connection| {
            (
                format!("{}_{}", connection.from_edge_id, connection.from_lane),
                connection.to_edge_id.clone(),
                connection.to_lane,
            )
        })
        .collect()
}

/// 沿 internal <connection> 续全 via 链并验证终止语义（#253 R5）：
/// - 仅接受 (to_edge, to_lane) 与外部目的完全一致、贡献新 lane 的续接；
///   不相关目的 lane 的 connection 不参与本穿越（第三轮：中间跳 toLane
///   不匹配不得被吞、也不得拐跑本可终止的链）；多条歧义即 fail-closed；
/// - 匹配目的的带 via 候选全部因已访问被滤 → 回环 error；
/// - 无匹配续接时必须存在匹配 (末段 lane, to_edge, to_lane) 的无 via 终端
///   connection（SUMO 官方要求），缺失或 toLane 不符分别报错；
/// - 深度越界 fail-closed。
fn extend_internal_chain(
    continuations: &HashMap<String, ContinuationCandidates>,
    terminals: &HashSet<(String, String, u32)>,
    mut chain: Vec<String>,
    to_edge_id: &str,
    to_lane: u32,
) -> Result<Vec<String>> {
    let mut visited: HashSet<String> = chain.iter().cloned().collect();
    while let Some(last) = chain.last().cloned() {
        let Some(candidates) = continuations.get(&last) else {
            require_terminal(terminals, &last, to_edge_id, to_lane)?;
            break;
        };
        let with_via: Vec<&(String, u32, Vec<String>)> = candidates
            .iter()
            .filter(|(_, _, via)| !via.is_empty())
            .collect();
        // 统一先匹配完整目的 (to_edge, to_lane) 再检查 visited（R5 第三轮）。
        let matching: Vec<&(String, u32, Vec<String>)> = with_via
            .iter()
            .copied()
            .filter(|(to, lane, _)| to == to_edge_id && *lane == to_lane)
            .collect();
        let usable: Vec<&(String, u32, Vec<String>)> = matching
            .iter()
            .copied()
            .filter(|(_, _, via)| via.iter().any(|v| !visited.contains(v)))
            .collect();
        match usable.len() {
            1 => {
                for lane in &usable[0].2 {
                    if visited.contains(lane) {
                        return Err(Error::SumoModel(format!(
                            "internal connection chain cycles at {lane:?} (to {to_edge_id:?})"
                        )));
                    }
                    visited.insert(lane.clone());
                    chain.push(lane.clone());
                }
            }
            0 => {
                if !matching.is_empty() {
                    // 匹配目的的带 via 候选全部已访问：回环。
                    return Err(Error::SumoModel(format!(
                        "internal connection chain cycles back to visited lanes \
                         from {last:?} to {to_edge_id:?} lane {to_lane}"
                    )));
                }
                require_terminal(terminals, &last, to_edge_id, to_lane)?;
                break;
            }
            _ => {
                return Err(Error::SumoModel(format!(
                    "ambiguous internal connection continuations from {last:?} \
                     to {to_edge_id:?} lane {to_lane}"
                )));
            }
        }
        if chain.len() > 8 {
            return Err(Error::SumoModel(format!(
                "internal connection chain from {:?} exceeds depth 8",
                chain.first()
            )));
        }
    }
    Ok(chain)
}

/// 终止检查：末段 lane 必须有无 via 的 exit-link 直通 (to_edge, to_lane)。
fn require_terminal(
    terminals: &HashSet<(String, String, u32)>,
    last_lane: &str,
    to_edge_id: &str,
    to_lane: u32,
) -> Result<()> {
    let any_target = terminals
        .iter()
        .any(|(from, to, _)| from == last_lane && to == to_edge_id);
    if terminals.contains(&(last_lane.to_owned(), to_edge_id.to_owned(), to_lane)) {
        Ok(())
    } else if any_target {
        Err(Error::SumoModel(format!(
            "internal terminal connection from {last_lane:?} reaches {to_edge_id:?} \
             but with a different toLane than the maneuver exit"
        )))
    } else {
        Err(Error::SumoModel(format!(
            "internal connection chain lacks the terminal exit-link connection \
             from {last_lane:?} to {to_edge_id:?}"
        )))
    }
}

/// 点状 stub 内边的形状端点距上限（米）：LuST 的 84 条 stub 均 ≤ 0.5 m。
const POINT_STUB_MAX_METERS: f64 = 0.5;

/// 点状 stub 删焊的入口末点→出口首点位移上限（米）。#253 G1 补充记录按
/// 「入口边界端点位移 1–6 cm」授权焊接，故取 6 cm；LuST pinned 基线实测
/// 84 条 stub 中 48 条 6–46 cm（最大 0.4617 m）越界（证据见
/// target/issue253-stub-weld-evidence.md），越界者 fail-closed，阈值待
/// G1 修订获批后上调并重锁全网验收数字。
const STUB_WELD_MAX_DISPLACEMENT_M: f64 = 0.06;

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
    fn internal_connection_chain_extends_maneuver_path() {
        // SUMO 对 internal junction（等待位置）拆分的 movement：外部 connection 的
        // via 仅首段，续段由 internal <connection>（to 相同、via 为下一段）承载。
        // 期望：maneuver path 含两段内边，而非首段直达 exit 的假捷径。
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/><lane id="east_1" index="1" speed="13.89" length="20.00" shape="6816.88,5730.52 6836.88,5730.52"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="5.00" shape="6806.88,5727.52 6811.88,5727.52"/></edge>
  <edge id=":J_2" function="internal"><lane id=":J_2_0" index="0" speed="13.89" length="5.00" shape="6811.88,5727.52 6816.88,5727.52"/></edge>
  <junction id="J" type="priority" intLanes=":J_0_0 :J_2_0"/>
  <junction id=":J_2_0" type="internal" incLanes=":J_0_0" intLanes=""/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0" via=":J_2_0"/>
  <connection from=":J_2" to="east" fromLane="0" toLane="0"/>
</net>"#;
        let network = parse_sumo_network_xml(xml).expect("parse");
        let topology = normalize_junctions(&network).expect("normalize");
        assert_eq!(topology.maneuver_paths.len(), 1);
        assert_eq!(
            topology.maneuver_paths[0].internal_edge_ids,
            ["sumo::J_0_0".to_owned(), "sumo::J_2_0".to_owned()]
        );
    }

    #[test]
    fn stub_with_continuation_fails_closed() {
        // 点状 stub 为首段且续链后带真续接段（:J_2_0，5 m）：删焊会吞掉有真实
        // 几何的续接段，fail-closed——R1 焊接门控只授权「恰一条 stub」的删焊。
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="0.40" shape="6806.88,5727.52 6807.18,5727.60"/></edge>
  <edge id=":J_2" function="internal"><lane id=":J_2_0" index="0" speed="13.89" length="5.00" shape="6811.88,5727.52 6816.88,5727.52"/></edge>
  <junction id="J" type="priority" intLanes=":J_0_0 :J_2_0"/>
  <junction id=":J_2_0" type="internal" incLanes=":J_0_0" intLanes=""/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0" via=":J_2_0"/>
  <connection from=":J_2" to="east" fromLane="0" toLane="0"/>
</net>"#;
        let network = parse_sumo_network_xml(xml).expect("parse");
        let error = normalize_junctions(&network).expect_err("stub with continuation must fail");
        assert!(
            error.to_string().contains("continuation"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn sole_stub_weld_beyond_displacement_limit_fails_closed() {
        // 恰一条 stub（无续接，含 SUMO 伴随的终端 exit-link）但入口末点→出口首点
        // 位移 10 m，越出 STUB_WELD_MAX_DISPLACEMENT_M（0.06 m，G1 授权 ≤ 6 cm）：
        // 删焊 fail-closed。
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="0.40" shape="6806.88,5727.52 6807.18,5727.60"/></edge>
  <junction id="J" type="priority" intLanes=":J_0_0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0"/>
</net>"#;
        let network = parse_sumo_network_xml(xml).expect("parse");
        let error = normalize_junctions(&network).expect_err("over-limit weld must fail");
        let message = error.to_string();
        assert!(
            message.contains("displace") && message.contains("0.06"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn sole_stub_within_displacement_limit_still_welds() {
        // 门控正向路径：恰一条 stub 且焊接位移 ≤ 0.06 m（本例 ≈ 0.022 m），
        // 删焊照旧——stub 移出路径，入口焊到出口首点。
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6806.90,5727.53 6826.90,5727.53"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="0.40" shape="6806.88,5727.52 6807.18,5727.60"/></edge>
  <junction id="J" type="priority" intLanes=":J_0_0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0"/>
</net>"#;
        let network = parse_sumo_network_xml(xml).expect("parse");
        let topology = normalize_junctions(&network).expect("normalize");
        assert_eq!(topology.maneuver_paths.len(), 1);
        assert!(
            topology.maneuver_paths[0].internal_edge_ids.is_empty(),
            "stub 应移出路径"
        );
        assert_eq!(topology.dropped_stub_lane_ids.len(), 1);
        let weld = topology
            .stub_welds
            .get("west_0")
            .expect("entry lane welded to exit start");
        assert_eq!(weld.0.to_f64().expect("x"), 6806.90);
        assert_eq!(weld.1.to_f64().expect("y"), 5727.53);
    }

    const CHAIN_BASE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/><lane id="east_1" index="1" speed="13.89" length="20.00" shape="6816.88,5730.52 6836.88,5730.52"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="5.00" shape="6806.88,5727.52 6811.88,5727.52"/></edge>
  <edge id=":J_2" function="internal"><lane id=":J_2_0" index="0" speed="13.89" length="5.00" shape="6811.88,5727.52 6816.88,5727.52"/></edge>
  <junction id="J" type="priority" intLanes=":J_0_0 :J_2_0"/>
  CONNECTIONS
</net>"#;

    fn chain_xml(connections: &str) -> String {
        CHAIN_BASE.replace("CONNECTIONS", connections)
    }

    #[test]
    fn internal_chain_middle_hop_to_lane_mismatch_fails_closed() {
        // R5 第三轮反例 (i)：中间续接 A→east toLane=1 via B 指向 out_1，而穿越
        // 出口是 out_0（终端 B→east toLane=0 正确）。只匹配 to_edge 的旧逻辑会
        // 错误接受 [A, B]；严格化后 A 处无匹配 (east,0) 的续接 → 终端检查
        // （A 无 out_0 终端）→ fail-closed。
        let xml = chain_xml(
            r#"<connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="1" via=":J_2_0"/>
  <connection from=":J_2" to="east" fromLane="0" toLane="0"/>"#,
        );
        let network = parse_sumo_network_xml(&xml).expect("parse");
        let error =
            normalize_junctions(&network).expect_err("middle-hop toLane mismatch must fail");
        assert!(
            error.to_string().contains("terminal"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn internal_chain_ignores_unrelated_destination_lane_continuation() {
        // R5 第三轮反例 (ii)：A 有正确的 out_0 终端，同时存在不相关的
        // A→east toLane=1 via B 续接。旧逻辑优先跟随不相关续接，B 缺 out_0
        // 终端 → 误拒。严格化后不相关目的 lane 的 connection 不参与本穿越，
        // 链在 A 处经终端正确结束。
        let xml = chain_xml(
            r#"<connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="1" via=":J_2_0"/>
  <connection from=":J_2" to="east" fromLane="0" toLane="1"/>"#,
        );
        let network = parse_sumo_network_xml(&xml).expect("parse");
        let topology = normalize_junctions(&network)
            .expect("unrelated toLane continuation must not derail the chain");
        assert_eq!(topology.maneuver_paths.len(), 1);
        assert_eq!(
            topology.maneuver_paths[0].internal_edge_ids,
            ["sumo::J_0_0".to_owned()]
        );
    }

    #[test]
    fn internal_chain_cycle_fails_closed() {
        // in→out via A；A→out via B；B→out via A：回环不得静默结束于 [A, B]。
        let xml = chain_xml(
            r#"<connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0" via=":J_2_0"/>
  <connection from=":J_2" to="east" fromLane="0" toLane="0" via=":J_0_0"/>"#,
        );
        let network = parse_sumo_network_xml(&xml).expect("parse");
        let error = normalize_junctions(&network).expect_err("cycle must fail closed");
        assert!(
            error.to_string().contains("cycles"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn internal_chain_missing_terminal_fails_closed() {
        // 有续接但缺无 via 的终端 exit-link（SUMO 官方要求的伴随 connection）。
        let xml = chain_xml(
            r#"<connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0" via=":J_2_0"/>"#,
        );
        let network = parse_sumo_network_xml(&xml).expect("parse");
        let error = normalize_junctions(&network).expect_err("missing terminal must fail closed");
        assert!(
            error.to_string().contains("terminal"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn internal_chain_terminal_to_lane_mismatch_fails_closed() {
        // 终端 exit-link 的 toLane 与 maneuver 出口 toLane 不一致。
        let xml = chain_xml(
            r#"<connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0" via=":J_2_0"/>
  <connection from=":J_2" to="east" fromLane="0" toLane="1" dir="s" state="M"/>"#,
        );
        let network = parse_sumo_network_xml(&xml).expect("parse");
        let error = normalize_junctions(&network).expect_err("toLane mismatch must fail closed");
        assert!(
            error.to_string().contains("toLane"),
            "unexpected error: {error}"
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
