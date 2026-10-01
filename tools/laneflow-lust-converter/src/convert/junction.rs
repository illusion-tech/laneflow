//! Normalize SUMO connections into Junction / Movement / ManeuverPath (§3.1).

use std::collections::{HashMap, HashSet};

use crate::{
    Error, Result,
    output::model::{Junction, ManeuverPath, Movement},
    sumo::{ExactDecimal, SUMO_ID_PREFIX, SumoConnection, SumoLane, SumoNetwork},
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
/// `stub_weld_gate` 为授权域门控（#253 R8）：0.5 m 位移例外仅限 pinned
/// 批准域；fixture/单测传 `StubWeldGate::Unrestricted`。
pub fn normalize_junctions(
    network: &SumoNetwork,
    stub_weld_gate: &StubWeldGate,
) -> Result<NormalizedTopology> {
    let lane_by_edge_index = build_lane_index(network)?;
    let adjacency = build_lane_adjacency(network, &lane_by_edge_index)?;
    let owners_by_int_lane = build_int_lane_owners(network)?;

    let mut dropped_stub_lane_ids = HashSet::new();
    let mut stub_welds: HashMap<String, (ExactDecimal, ExactDecimal)> = HashMap::new();
    let mut stub_weld_records = Vec::new();
    let mut traversals = Vec::new();
    let continuations = internal_continuations(network);
    let terminals = internal_terminals(network);
    // G1 六条件之拓扑形态：stub 的多重引用计数（via 引用 + internal from 引用）。
    let mut via_ref_count: HashMap<&str, usize> = HashMap::new();
    let mut internal_from_count: HashMap<String, usize> = HashMap::new();
    for connection in &network.connections {
        for via in &connection.via_lane_ids {
            *via_ref_count.entry(via.as_str()).or_default() += 1;
        }
        if is_internal_edge(network, &connection.from_edge_id) {
            *internal_from_count
                .entry(format!(
                    "{}_{}",
                    connection.from_edge_id, connection.from_lane
                ))
                .or_default() += 1;
        }
    }
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
            network,
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
                // G1 六条件门控（位移 → 局部性 → 拓扑形态 → 共享入口）统一由
                // evaluate_stub_candidate 评估；welded 时应用焊接并落记录。
                let junction_id = resolve_owner(
                    network,
                    &connection.from_edge_id,
                    &connection.to_edge_id,
                    &[],
                    &owners_by_int_lane,
                )?;
                let context = StubEvaluationContext {
                    reference_counts: StubReferenceCounts {
                        via: &via_ref_count,
                        internal_from: &internal_from_count,
                    },
                    gate: stub_weld_gate,
                };
                let evaluation = evaluate_stub_candidate(
                    network,
                    connection,
                    entry,
                    exit,
                    stub_id,
                    &junction_id,
                    &context,
                )?;
                if evaluation.disposition != StubWeldDisposition::Welded {
                    // G1 §5：拒绝处置不中止——不删 stub、不写焊接，原始连接保留
                    // 走普通 via 穿越（下方 sequence 校验与发射照旧；发射不可行
                    // 则入诊断清单）。记录（含诊断摘要）同样入附录。
                    stub_weld_records.push(evaluation.record);
                    // 继续普通路径：internal_lane_ids 保持 [stub] 不清空。
                } else {
                    let target = exit.shape.first().expect("evaluated weld target");
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
                    stub_weld_records.push(evaluation.record);
                }
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
    // #253 K5：movement id 是 from + "-to-" + to 的拼接，含分隔符的合法 SUMO
    // id 可碰撞（(a-to-b, c) 与 (a, b-to-c) 拼出同一串）。发射 id 格式不改，
    // 但记录每个 id 的 (from, to) 来源：同 id 不同来源即歧义合并，fail-closed。
    let mut movement_sources: HashMap<String, (String, String)> = HashMap::new();
    for traversal in &traversals {
        let movement_id = movement_id(
            &traversal.key.junction_id,
            &traversal.key.from_road_edge_id,
            &traversal.key.to_road_edge_id,
        );
        let source = (
            traversal.key.from_road_edge_id.clone(),
            traversal.key.to_road_edge_id.clone(),
        );
        match movement_sources.insert(movement_id.clone(), source.clone()) {
            None => movements.push(Movement {
                id: movement_id,
                junction_id: format!("{SUMO_ID_PREFIX}{}", traversal.key.junction_id),
                from_road_edge_id: traversal.key.from_road_edge_id.clone(),
                to_road_edge_id: traversal.key.to_road_edge_id.clone(),
            }),
            Some(previous) if previous != source => {
                return Err(Error::SumoModel(format!(
                    "movement id {movement_id:?} collides between distinct movements \
                     {previous:?} and {source:?}; delimiter-ambiguous SUMO edge ids"
                )));
            }
            Some(_) => {}
        }
    }
    movements.sort_by(|left, right| left.id.cmp(&right.id));

    // #253 V7：path id 字符串撞名（`-to-` / `.` 分隔符歧义）fail-closed，
    // 记录首见路径的身份用于报错。
    let mut path_sources: HashMap<String, (String, String, String)> = HashMap::new();
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
            let source = (
                path.entry_edge_id.clone(),
                path.exit_edge_id.clone(),
                path.internal_edge_ids.join("."),
            );
            if let Some(previous) = path_sources.insert(path.id.clone(), source.clone()) {
                return Err(Error::SumoModel(format!(
                    "maneuver path id {:?} collides between {:?} and {:?}; delimiter-ambiguous SUMO ids",
                    path.id, previous, source
                )));
            }
            // #253 C7：同 (from,fromLane,to,toLane) 不同内链的重复穿越此前被
            // 静默替换（路由/信号绑定只能发现后者）；重复键 fail-closed。
            if path_by_connection
                .insert(
                    (
                        traversal.key.from_road_edge_id.clone(),
                        traversal.key.from_lane_index,
                        traversal.key.to_road_edge_id.clone(),
                        traversal.key.to_lane_index,
                    ),
                    path.id.clone(),
                )
                .is_some()
            {
                return Err(Error::SumoModel(format!(
                    "duplicate connection mapping {:?}/{} -> {:?}/{} resolves to \
                     multiple maneuver paths",
                    traversal.key.from_road_edge_id,
                    traversal.key.from_lane_index,
                    traversal.key.to_road_edge_id,
                    traversal.key.to_lane_index
                )));
            }
            Ok(path)
        })
        .collect::<Result<Vec<_>>>()?;

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
        stub_weld_records,
    })
}

/// internal junction 续接候选列表：每条候选为 (to_edge, to_lane, via)。
type ContinuationCandidates = Vec<(String, u32, Vec<String>)>;

/// internal junction 续接（#253 R1/R5）：from-lane → 候选 (to_edge, to_lane, via)。
/// internal <connection> 的 from 为 internal 边。
fn internal_continuations(network: &SumoNetwork) -> HashMap<String, ContinuationCandidates> {
    let mut map: HashMap<String, ContinuationCandidates> = HashMap::new();
    for connection in &network.connections {
        if !is_internal_edge(network, &connection.from_edge_id) {
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
            is_internal_edge(network, &connection.from_edge_id)
                && connection.via_lane_ids.is_empty()
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
///   不相关目的 lane 的 connection 不参与本穿越（#253 R5：中间跳 toLane
///   不匹配不得被吞、也不得拐跑本可终止的链）；多条歧义即 fail-closed；
/// - 匹配目的的带 via 候选全部因已访问被滤 → 回环 error；
/// - 无匹配续接时必须存在匹配 (末段 lane, to_edge, to_lane) 的无 via 终端
///   connection（SUMO 官方要求），缺失或 toLane 不符分别报错；
/// - 深度越界 fail-closed。
fn extend_internal_chain(
    network: &SumoNetwork,
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
        // 统一先匹配完整目的 (to_edge, to_lane) 再检查 visited（#253 R5）。
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
                    // #253 V8：续接 via lane 与原始 via 同标准——非 internal
                    // 即 fail-closed（外部对象混入 internal 链是歧义拓扑）。
                    match network.lane(lane) {
                        Some(via_lane) if via_lane.function_internal => {}
                        _ => {
                            return Err(Error::SumoModel(format!(
                                "internal connection chain references non-internal lane {lane:?} (to {to_edge_id:?})"
                            )));
                        }
                    }
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

/// stub 引用计数（via 引用 + internal from 引用；G1 拓扑形态检查输入）。
struct StubReferenceCounts<'a> {
    via: &'a HashMap<&'a str, usize>,
    internal_from: &'a HashMap<String, usize>,
}

/// 点状 stub 候选评估（G1 六条件机制；normalize 门控与候选扫描器共用同一
/// 推导，manifest disposition 不可能与运行时处置漂移）。门控顺序固定：
/// 位移 → 局部性 → 拓扑形态（via 多重引用 / internal from 引用数）→ 共享
/// 入口影响重验；首个触发项决定处置，其余度量仍全部计入记录。
fn evaluate_stub_candidate(
    network: &SumoNetwork,
    connection: &SumoConnection,
    entry: &SumoLane,
    exit: &SumoLane,
    stub_id: &str,
    junction_id: &str,
    context: &StubEvaluationContext<'_>,
) -> Result<StubEvaluation> {
    let stub_lane = network.lane(stub_id).expect("via lane checked");
    let span = lane_shape_span_meters(stub_lane)?;
    let shape_len = lane_shape_length_meters(stub_lane)?;
    let locality = lane_shape_locality_meters(stub_lane)?;
    let target = exit
        .shape
        .first()
        .ok_or_else(|| Error::SumoModel(format!("exit lane {:?} has empty shape", exit.id)))?;
    let entry_end = entry
        .shape
        .last()
        .ok_or_else(|| Error::SumoModel(format!("entry lane {:?} has empty shape", entry.id)))?;
    let dx = target.0.checked_sub(entry_end.0)?.to_f64()?;
    let dy = target.1.checked_sub(entry_end.1)?.to_f64()?;
    let displacement = (dx * dx + dy * dy).sqrt();
    let controlled = connection.tl_id.is_some() || connection.link_index.is_some();
    let base = StubWeldRecord {
        rule_version: STUB_WELD_RULE_VERSION,
        disposition: StubWeldDisposition::Welded,
        stub_lane_id: stub_id.to_owned(),
        entry_lane_id: entry.id.clone(),
        exit_lane_id: exit.id.clone(),
        junction_id: junction_id.to_owned(),
        entry_end_m: [entry_end.0.to_f64()?, entry_end.1.to_f64()?],
        weld_target_m: [target.0.to_f64()?, target.1.to_f64()?],
        displacement_m: displacement,
        span_m: span,
        shape_len_m: shape_len,
        locality_m: locality,
        controlled,
        shared_traversal_count: 0,
        shared_traversals: Vec::new(),
        detail: String::new(),
    };
    // R8 授权域门控先于几何门控：0.5 m 例外只对 pinned 批准域生效；不满足
    // 一律 BlockedOutOfDomain（保留原始连接走正常穿越），不评估几何豁免。
    match context.gate {
        StubWeldGate::Unrestricted => {}
        StubWeldGate::BlockedDomain => {
            let detail = "stub weld blocked: source is not verify-source verified; \
                          the 0.5 m displacement exception is limited to the pinned \
                          approved domain"
                .to_owned();
            let mut record = base.clone();
            record.disposition = StubWeldDisposition::BlockedOutOfDomain;
            record.detail = detail.clone();
            return Ok(StubEvaluation {
                disposition: StubWeldDisposition::BlockedOutOfDomain,
                record,
            });
        }
        StubWeldGate::VerifiedDomain { net_digest } => {
            let approved = approved_stub_set()?;
            let digest_ok = net_digest.as_deref() == Some(approved.net_digest.as_str());
            let identity_ok =
                approved
                    .identities
                    .contains(&(stub_id, entry.id.as_str(), exit.id.as_str()));
            if !digest_ok || !identity_ok {
                let detail = format!(
                    "stub weld blocked: candidate outside the approved pinned domain \
                     (net digest matches manifest: {digest_ok}, identity in approved set: \
                     {identity_ok})"
                );
                let mut record = base.clone();
                record.disposition = StubWeldDisposition::BlockedOutOfDomain;
                record.detail = detail.clone();
                return Ok(StubEvaluation {
                    disposition: StubWeldDisposition::BlockedOutOfDomain,
                    record,
                });
            }
        }
    }
    if displacement > STUB_WELD_MAX_DISPLACEMENT_M {
        let detail = format!(
            "point-stub removal would displace entry lane {:?} by \
             {displacement:.4} m (> {STUB_WELD_MAX_DISPLACEMENT_M} m) \
             to reach exit lane {:?} start",
            entry.id, exit.id
        );
        let mut record = base.clone();
        record.disposition = StubWeldDisposition::RejectedDisplacement;
        record.detail = detail;
        return Ok(StubEvaluation {
            disposition: StubWeldDisposition::RejectedDisplacement,
            record,
        });
    }
    if locality > STUB_WELD_MAX_LOCALITY_M {
        let detail = format!(
            "point-stub internal lane {stub_id:?} shape is not local: max deviation \
             from its endpoint chord is {locality:.4} m (> {STUB_WELD_MAX_LOCALITY_M} m)"
        );
        let mut record = base.clone();
        record.disposition = StubWeldDisposition::RejectedLocality;
        record.detail = detail;
        return Ok(StubEvaluation {
            disposition: StubWeldDisposition::RejectedLocality,
            record,
        });
    }
    let via_refs = context
        .reference_counts
        .via
        .get(stub_id)
        .copied()
        .unwrap_or(0);
    let from_refs = context
        .reference_counts
        .internal_from
        .get(stub_id)
        .copied()
        .unwrap_or(0);
    if via_refs != 1 || from_refs != 1 {
        let detail = format!(
            "point-stub internal lane {stub_id:?} carries internal semantics: \
             referenced by {via_refs} connection via entries and {from_refs} internal \
             from-lane entries (expected exactly 1 each)"
        );
        let mut record = base.clone();
        record.disposition = StubWeldDisposition::RejectedTopology;
        record.detail = detail;
        return Ok(StubEvaluation {
            disposition: StubWeldDisposition::RejectedTopology,
            record,
        });
    }
    // 共享入口影响重验：所有以 entry 为入口的其他穿越，焊接后首段 join 间隙
    // = |目标 T − 后继首点|，超 compiler join 容差即破坏该穿越。
    let lane_by_edge_index = build_lane_index(network)?;
    let mut record = base;
    let mut broken: Option<String> = None;
    for other in &network.connections {
        if std::ptr::eq(other, connection) || is_internal_edge(network, &other.from_edge_id) {
            continue;
        }
        if other.from_edge_id != connection.from_edge_id || other.from_lane != connection.from_lane
        {
            continue;
        }
        let (successor_desc, successor_start) = if let Some(via) = other.via_lane_ids.first() {
            let lane = network.lane(via).ok_or_else(|| {
                Error::SumoModel(format!(
                    "shared-entry connection via references unknown lane {via:?}"
                ))
            })?;
            (via.clone(), lane.shape.first().copied())
        } else {
            let lane = resolve_lane(&lane_by_edge_index, &other.to_edge_id, other.to_lane)?;
            (lane.id.clone(), lane.shape.first().copied())
        };
        let Some(start) = successor_start else {
            return Err(Error::SumoModel(format!(
                "shared-entry successor lane {successor_desc:?} has empty shape"
            )));
        };
        let gx = target.0.checked_sub(start.0)?.to_f64()?;
        let gy = target.1.checked_sub(start.1)?.to_f64()?;
        let gap = (gx * gx + gy * gy).sqrt();
        record.shared_traversals.push((
            format!("{}_{}", other.to_edge_id, other.to_lane),
            successor_desc,
            gap,
        ));
        if gap > STUB_WELD_JOIN_GAP_M && broken.is_none() {
            broken = Some(format!(
                "point-stub removal would move entry lane {:?} end onto exit lane {:?} \
                 start, breaking shared traversal {:?}: first-internal join gap \
                 {gap:.4} m (> {STUB_WELD_JOIN_GAP_M} m)",
                entry.id,
                exit.id,
                format!(
                    "{}_{}->{}_{}",
                    other.from_edge_id, other.from_lane, other.to_edge_id, other.to_lane
                )
            ));
        }
    }
    record.shared_traversal_count = record.shared_traversals.len();
    if let Some(detail) = broken {
        record.disposition = StubWeldDisposition::RejectedSharedEntry;
        record.detail = detail;
        return Ok(StubEvaluation {
            disposition: StubWeldDisposition::RejectedSharedEntry,
            record,
        });
    }
    Ok(StubEvaluation {
        disposition: StubWeldDisposition::Welded,
        record,
    })
}

/// 全网点状 stub 候选扫描（G1 六条件之限定适用域：候选身份由可复现清单
/// 固定；`evidence/lust-stub-weld-candidates.json` 的生成与防漂移比对的
/// 共同数据源）。身份口径与 normalize 一致：外部 connection、原 via 恰
/// 一条且首段 shape 端点距 < 0.5 m。扫描以 Unrestricted 门控评估几何
/// 处置——R8 的授权域检查（BlockedOutOfDomain）是 runtime-only，manifest
/// 逐字节稳定。
pub fn scan_stub_weld_candidates(network: &SumoNetwork) -> Result<Vec<StubWeldRecord>> {
    let lane_by_edge_index = build_lane_index(network)?;
    let mut via_ref_count: HashMap<&str, usize> = HashMap::new();
    let mut internal_from_count: HashMap<String, usize> = HashMap::new();
    for connection in &network.connections {
        for via in &connection.via_lane_ids {
            *via_ref_count.entry(via.as_str()).or_default() += 1;
        }
        if is_internal_edge(network, &connection.from_edge_id) {
            *internal_from_count
                .entry(format!(
                    "{}_{}",
                    connection.from_edge_id, connection.from_lane
                ))
                .or_default() += 1;
        }
    }
    let owners = build_int_lane_owners(network)?;
    let continuations = internal_continuations(network);
    let terminals = internal_terminals(network);
    let mut candidates = Vec::new();
    for connection in &network.connections {
        if is_internal_edge(network, &connection.from_edge_id) {
            continue;
        }
        let (Some(from_edge), Some(to_edge)) = (
            network.edge(&connection.from_edge_id),
            network.edge(&connection.to_edge_id),
        ) else {
            continue;
        };
        if from_edge.function_internal || to_edge.function_internal {
            continue;
        }
        if connection.via_lane_ids.len() != 1 {
            continue;
        }
        let stub_id = &connection.via_lane_ids[0];
        let Some(stub_lane) = network.lane(stub_id) else {
            continue;
        };
        if !stub_lane.function_internal
            || lane_shape_span_meters(stub_lane)? >= POINT_STUB_MAX_METERS
        {
            continue;
        }
        // 带真续接段的 stub 链在 normalize 走 fail-closed unsupported，不是删焊
        // 候选；扫描只认「完整展开后恰一条 stub」的形态（检查其无贡献新 lane
        // 的续接伴随项）。
        let chain = extend_internal_chain(
            network,
            &continuations,
            &terminals,
            vec![stub_id.clone()],
            &connection.to_edge_id,
            connection.to_lane,
        )?;
        if chain.len() != 1 {
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
        let junction_id = resolve_owner(
            network,
            &connection.from_edge_id,
            &connection.to_edge_id,
            &[],
            &owners,
        )?;
        let context = StubEvaluationContext {
            reference_counts: StubReferenceCounts {
                via: &via_ref_count,
                internal_from: &internal_from_count,
            },
            gate: &StubWeldGate::Unrestricted,
        };
        let evaluation = evaluate_stub_candidate(
            network,
            connection,
            entry,
            exit,
            stub_id,
            &junction_id,
            &context,
        )?;
        candidates.push(evaluation.record);
    }
    candidates.sort_by(|left, right| left.stub_lane_id.cmp(&right.stub_lane_id));
    Ok(candidates)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StubWeldManifestThresholds {
    max_displacement_m: f64,
    max_locality_m: f64,
    join_gap_m: f64,
    point_stub_span_m: f64,
    /// join 间隙阈值的出处（compiler 同名常数，保持一致）。
    join_gap_source: &'static str,
}

/// 候选 manifest（evidence/lust-stub-weld-candidates.json 的内容）：头部为
/// 规则版本、pinned commit、net digest 与生成器；条目为全部候选（含 rejected
/// 的处置与度量），身份由 `scan_stub_weld_candidates` 可复现固定。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StubWeldManifest {
    manifest_version: u32,
    rule_version: &'static str,
    pinned_commit: &'static str,
    net_digest: String,
    generator: &'static str,
    thresholds: StubWeldManifestThresholds,
    candidates: Vec<StubWeldRecord>,
}

/// 生成候选 manifest JSON（生成测试落盘；防漂移测试对 pinned 源重扫比对）。
pub fn stub_weld_manifest_json(network: &SumoNetwork, net_digest: &str) -> Result<String> {
    let candidates = scan_stub_weld_candidates(network)?;
    let manifest = StubWeldManifest {
        manifest_version: 1,
        rule_version: STUB_WELD_RULE_VERSION,
        pinned_commit: crate::LUST_COMMIT,
        net_digest: net_digest.to_owned(),
        generator: "laneflow-lust-converter",
        thresholds: StubWeldManifestThresholds {
            max_displacement_m: STUB_WELD_MAX_DISPLACEMENT_M,
            max_locality_m: STUB_WELD_MAX_LOCALITY_M,
            join_gap_m: STUB_WELD_JOIN_GAP_M,
            point_stub_span_m: POINT_STUB_MAX_METERS,
            join_gap_source: "compiler MAX_SOURCE_JOIN_GAP_METERS",
        },
        candidates,
    };
    serde_json::to_string_pretty(&manifest).map_err(|source| Error::Json {
        document: "lust-stub-weld-candidates",
        source,
    })
}

/// 点状 stub 内边的形状端点距上限（米）：LuST 的 84 条 stub 均 ≤ 0.5 m。
const POINT_STUB_MAX_METERS: f64 = 0.5;

/// 点状 stub 删焊的入口末点→出口首点位移上限（米）。G1 修订已获项目所有者
/// 最终确认（2026-09-30，issuecomment-5902944418；评估背景
/// issuecomment-5901846033 第 5 节）：
/// 仅位移预算上调至 0.5 m（LuST pinned 基线 84 条候选实测最大 0.4617 m，
/// 全部纳入）；局部性 0.05 m / join 间隙 0.005 m / 端点距身份 0.5 m 均不动。
/// G1 §5：超出位移预算或其他合法性条件的对象继续「拒绝处置并记录诊断」
/// ——不中止转换，保留原始连接走正常穿越（发射不可行则入清单）。
pub(crate) const STUB_WELD_MAX_DISPLACEMENT_M: f64 = 0.5;

/// 删焊规则版本（G1 issuecomment-5901846033 六条件机制；@2 = 位移阈值
/// 0.06 m → 0.5 m 启用 + 拒绝处置改为保留记录不中止）。随焊接记录与候选
/// manifest 落出，追溯用。
pub(crate) const STUB_WELD_RULE_VERSION: &str = "stub-weld/g1-six-cond@2";

/// stub 形状局部性上限（米）：全部 shape 点对端点弦（线段）的最大偏离。
/// 实测分布（pinned c4bd5bd3 全部 84 条）：max = 0.0424 m（仅 1 条 > 0.02，
/// 95 分位 0.0118，中位 0）。取实测包络 0.0424 m + 约 18% 余量 → 0.05 m：
/// 覆盖全部实测候选，同时把「形状在中段明显游荡、并非点状噪声」的边挡在
/// 删焊之外（点状 stub 的语义是端点抖动，局部偏离应同量级于位移预算）。
/// G1 提阈值至 0.5 m 时本界不动（局部性与位移是独立量纲）。
pub(crate) const STUB_WELD_MAX_LOCALITY_M: f64 = 0.05;

/// 焊接共享影响重验的 join 间隙上限（米），镜像 compiler 的
/// `MAX_SOURCE_JOIN_GAP_METERS`（0.005 m）：入口 E 被焊接后末点移到目标 T，
/// 所有以 E 为入口的其他穿越的首段 join 间隙 = |T − 后继首点|，超 5 mm 即
/// 破坏该穿越的边界连接，fail-closed（G1 六条件之共享影响复核）。
pub(crate) const STUB_WELD_JOIN_GAP_M: f64 = 0.005;

/// 焊接处置逐条记录（G1 六条件之可追溯记录）：字段与
/// `evidence/lust-stub-weld-candidates.json` 候选条目一致，随诊断清单渲染
/// 落出（已归一化类）。
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StubWeldRecord {
    pub rule_version: &'static str,
    /// 处置结果（manifest disposition 与 normalize 门控共用）。
    pub disposition: StubWeldDisposition,
    pub stub_lane_id: String,
    pub entry_lane_id: String,
    pub exit_lane_id: String,
    pub junction_id: String,
    /// 原入口末点（SUMO 原始坐标）。
    pub entry_end_m: [f64; 2],
    /// 焊接目标 = 出口首点（SUMO 原始坐标）。
    pub weld_target_m: [f64; 2],
    pub displacement_m: f64,
    pub span_m: f64,
    pub shape_len_m: f64,
    pub locality_m: f64,
    /// 申请连接的 tl/linkIndex 承载（运动级信号控制随焊接保留，记录备查）。
    pub controlled: bool,
    /// 共享入口重验通过的关联穿越数。
    pub shared_traversal_count: usize,
    /// 关联穿越明细：(出口 lane, 后继 lane, 重验间隙 m)。
    pub shared_traversals: Vec<(String, String, f64)>,
    /// 拒绝处置的诊断摘要（welded 为空串；G1 §5 记录诊断）。
    pub detail: String,
}

/// 候选处置结果（manifest disposition 与 normalize 报错共用同一推导）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StubWeldDisposition {
    Welded,
    RejectedDisplacement,
    RejectedLocality,
    /// 拓扑形态：via 多重引用 / internal from 引用数异常。
    RejectedTopology,
    RejectedSharedEntry,
    /// 授权域门控拒绝（#253 R8，runtime-only）：0.5 m 例外仅限 pinned 基线 +
    /// manifest 批准集；扫描器/manifest 永不产生本变体（manifest 逐字节稳定）。
    BlockedOutOfDomain,
}

impl serde::Serialize for StubWeldDisposition {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.label())
    }
}

impl StubWeldDisposition {
    pub fn label(self) -> &'static str {
        match self {
            StubWeldDisposition::Welded => "welded",
            StubWeldDisposition::RejectedDisplacement => "rejected-displacement",
            StubWeldDisposition::RejectedLocality => "rejected-locality",
            StubWeldDisposition::RejectedTopology => "rejected-topology",
            StubWeldDisposition::RejectedSharedEntry => "rejected-shared-entry",
            StubWeldDisposition::BlockedOutOfDomain => "blocked-out-of-domain",
        }
    }
}

/// 候选评估上下文：引用计数 + 授权域门控（#253 R8，束参数）。
struct StubEvaluationContext<'a> {
    reference_counts: StubReferenceCounts<'a>,
    gate: &'a StubWeldGate,
}

/// 候选评估（度量 + 处置 + 记录），normalize 门控与候选扫描器共用；
/// 拒绝原因在 record.detail。
struct StubEvaluation {
    disposition: StubWeldDisposition,
    record: StubWeldRecord,
}

/// stub 删焊策略（`TopologyConvertOptions.stub_weld_policy`，#253 R8）：0.5 m
/// 位移例外是 G1 对**固定 pinned 基线 + manifest 批准集**的授权，通用输入
/// 不得无条件继承。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum StubWeldPolicy {
    /// G1 授权域三条件（默认）：来源 verify-source 已验证 + net digest 命中
    /// manifest + 候选身份（stub/entry/exit 三元组）∈ 批准集。缺一即拒
    /// （BlockedOutOfDomain，保留原始连接走正常穿越）。
    #[default]
    Auto,
    /// 显式测试策略：跳过授权域检查。**仅限 crate 内单测**——`cfg(test)`
    /// 门控使生产公开 API 不可达（#253 C6；integration fixture 经审计无
    /// stub 依赖，pipeline/通用 XML 必须保持 Auto）。
    #[cfg(test)]
    AllowUnrestricted,
}

/// 运行时的授权域门控（由 options 策略 + ReportSource 推导，见
/// convert_network_packages）。`ReportSource.verified` 的类型级保证（R2：
/// 外部无法构造 verified=true）使「来源已验证」在 Auto 下可依赖。
#[derive(Clone, Debug)]
pub(crate) enum StubWeldGate {
    /// Auto 且来源未 verified：候选一律 BlockedOutOfDomain。
    BlockedDomain,
    /// Auto 且来源 verified：net digest 须命中 manifest netDigest，候选身份
    /// 须 ∈ 批准集。
    VerifiedDomain { net_digest: Option<String> },
    /// 显式测试策略：不做授权域检查（仅 fixture）。
    Unrestricted,
}

/// 批准的 stub 候选集合（`evidence/lust-stub-weld-candidates.json` 内嵌，
/// OnceLock 懒解析一次）。身份口径：SUMO 原始 id 三元组。
struct ApprovedStubSet {
    net_digest: String,
    identities: HashSet<(&'static str, &'static str, &'static str)>,
}

#[derive(serde::Deserialize)]
struct ApprovedManifestView {
    #[serde(rename = "netDigest")]
    net_digest: String,
    candidates: Vec<ApprovedCandidateView>,
}

#[derive(serde::Deserialize)]
struct ApprovedCandidateView {
    #[serde(rename = "stubLaneId")]
    stub_lane_id: String,
    #[serde(rename = "entryLaneId")]
    entry_lane_id: String,
    #[serde(rename = "exitLaneId")]
    exit_lane_id: String,
}

/// 内嵌的候选 manifest（junction.rs 位于 src/convert/，evidence 在其上两级）。
const APPROVED_MANIFEST_JSON: &str = include_str!("../../evidence/lust-stub-weld-candidates.json");

fn approved_stub_set() -> Result<&'static ApprovedStubSet> {
    static SET: std::sync::OnceLock<std::result::Result<ApprovedStubSet, String>> =
        std::sync::OnceLock::new();
    SET.get_or_init(|| {
        let view: ApprovedManifestView =
            serde_json::from_str(APPROVED_MANIFEST_JSON).map_err(|error| error.to_string())?;
        // 解析一次、泄漏一次：身份集合以 'static 引用服务整个进程生命周期。
        let leaked: &'static ApprovedManifestView = Box::leak(Box::new(view));
        Ok(ApprovedStubSet {
            net_digest: leaked.net_digest.clone(),
            identities: leaked
                .candidates
                .iter()
                .map(|candidate| {
                    (
                        candidate.stub_lane_id.as_str(),
                        candidate.entry_lane_id.as_str(),
                        candidate.exit_lane_id.as_str(),
                    )
                })
                .collect(),
        })
    })
    .as_ref()
    .map_err(|message| Error::SumoModel(format!("approved stub manifest parse failed: {message}")))
}

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

/// 内边 shape 折线长度（米）。
fn lane_shape_length_meters(lane: &SumoLane) -> Result<f64> {
    let mut len = 0.0;
    for window in lane.shape.windows(2) {
        let dx = window[1].0.checked_sub(window[0].0)?.to_f64()?;
        let dy = window[1].1.checked_sub(window[0].1)?.to_f64()?;
        len += (dx * dx + dy * dy).sqrt();
    }
    Ok(len)
}

/// 形状局部性（米）：全部 shape 点对端点弦（线段，垂足参数 clamp 到
/// [0,1]）的最大偏离。点状 stub 的语义是端点抖动噪声，局部偏离应与位移
/// 预算同量级；明显游荡的非局部形状不进入删焊（G1 六条件之独立几何门控）。
fn lane_shape_locality_meters(lane: &SumoLane) -> Result<f64> {
    let Some((first, rest)) = lane.shape.split_first() else {
        return Ok(0.0);
    };
    let Some(last) = rest.last() else {
        return Ok(0.0);
    };
    let dx = last.0.checked_sub(first.0)?.to_f64()?;
    let dy = last.1.checked_sub(first.1)?.to_f64()?;
    let chord = (dx * dx + dy * dy).sqrt();
    let mut worst = 0.0_f64;
    for (px, py) in &lane.shape[1..lane.shape.len() - 1] {
        let rel_x = px.checked_sub(first.0)?.to_f64()?;
        let rel_y = py.checked_sub(first.1)?.to_f64()?;
        let deviation = if chord < 1e-12 {
            (rel_x * rel_x + rel_y * rel_y).sqrt()
        } else {
            let t = ((rel_x * dx + rel_y * dy) / (chord * chord)).clamp(0.0, 1.0);
            let cx = rel_x - t * dx;
            let cy = rel_y - t * dy;
            (cx * cx + cy * cy).sqrt()
        };
        worst = worst.max(deviation);
    }
    Ok(worst)
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

    /// 焊接处置逐条记录（G1 可追溯），随诊断清单渲染落出。
    pub stub_weld_records: Vec<StubWeldRecord>,
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

    // #253 V2（T4 升级）：intLanes 成员限 internal——拼错的 id、外部 lane、
    // 非 internal junction 混入都会让归属静默错位；fail-closed 报 junction
    // id 与成员。
    for junction in &network.junctions {
        for member in &junction.int_lane_ids {
            match (network.lane(member), network.junction(member)) {
                (Some(lane), _) if lane.function_internal => {}
                (_, Some(nested)) if nested.junction_type == "internal" => {}
                _ => {
                    return Err(Error::SumoModel(format!(
                        "junction {:?} intLanes member {member:?} is not internal (no function=internal / non-internal junction id)",
                        junction.id
                    )));
                }
            }
        }
    }

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
                if !is_internal(lane_id) {
                    continue;
                }
                match cluster_parent.get(lane_id.as_str()) {
                    Some(&existing) if existing != parent => {
                        // #253 P5：两个已归属 cluster 节点列出同一嵌套节点且父
                        // 不同——此前按 XML 序静默跳过（归属取决于文件顺序），
                        // fail-closed 报节点与两个父。
                        return Err(Error::SumoModel(format!(
                            "nested internal junction {lane_id:?} has conflicting cluster                              parents {existing:?} and {parent:?} (via internal junction {:?})",
                            junction.id
                        )));
                    }
                    Some(_) => {}
                    None => {
                        cluster_parent.insert(lane_id.as_str(), parent);
                        changed = true;
                    }
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

/// #253 P4：internal 判定以 parser 记录的 `SumoEdge::function_internal` 为准——
/// id 的 ':' 前缀只是 LuST 的习惯命名，不是规范保证；未知 edge 按非 internal
/// （与此前前缀法对无前缀 id 的语义一致）。
fn is_internal_edge(network: &SumoNetwork, edge_id: &str) -> bool {
    network
        .edge(edge_id)
        .is_some_and(|edge| edge.function_internal)
}

/// 按 (edge_id, index) 收集 lane 地址索引（#253 M3：同地址声明不同 lane id
/// 的歧义拓扑 fail-closed，不得静默保留 XML 序最后一条）。
fn build_lane_index(network: &SumoNetwork) -> Result<HashMap<(String, u32), &SumoLane>> {
    let mut index = HashMap::with_capacity(network.lanes.len());
    for lane in &network.lanes {
        if let Some(previous) = index.insert((lane.edge_id.clone(), lane.index), lane)
            && previous.id != lane.id
        {
            return Err(Error::SumoModel(format!(
                "duplicate lane address: edge {:?} lane index {} declared as both {:?} and {:?}",
                lane.edge_id, lane.index, previous.id, lane.id
            )));
        }
    }
    Ok(index)
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
        let topology =
            normalize_junctions(&network, &StubWeldGate::Unrestricted).expect("normalize");
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
        let topology =
            normalize_junctions(&network, &StubWeldGate::Unrestricted).expect("normalize");
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
        let error = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect_err("stub with continuation must fail");
        assert!(
            error.to_string().contains("continuation"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn sole_stub_over_displacement_retained_with_record() {
        // G1 §5：位移 10 m 越出 STUB_WELD_MAX_DISPLACEMENT_M（0.5 m）→ 拒绝
        // 处置不中止：不删 stub、不写焊接，原始连接保留走普通 via 穿越，
        // 诊断记录（rejected-displacement + 摘要）入附录。
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
        let topology = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect("refused weld retains the lane");
        assert!(
            topology.dropped_stub_lane_ids.is_empty(),
            "refused stub must not be dropped"
        );
        assert!(
            topology.stub_welds.is_empty(),
            "refused weld must not rewrite the entry endpoint"
        );
        assert_eq!(
            topology.maneuver_paths[0].internal_edge_ids,
            ["sumo::J_0_0".to_owned()],
            "原始连接保留：stub 留在路径里"
        );
        assert_eq!(topology.stub_weld_records.len(), 1);
        let record = &topology.stub_weld_records[0];
        assert_eq!(
            record.disposition,
            StubWeldDisposition::RejectedDisplacement
        );
        assert!(record.detail.contains("displace"));
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
        let topology =
            normalize_junctions(&network, &StubWeldGate::Unrestricted).expect("normalize");
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
        // #253 R5 反例 (i)：中间续接 A→east toLane=1 via B 指向 out_1，而穿越
        // 出口是 out_0（终端 B→east toLane=0 正确）。只匹配 to_edge 的旧逻辑会
        // 错误接受 [A, B]；严格化后 A 处无匹配 (east,0) 的续接 → 终端检查
        // （A 无 out_0 终端）→ fail-closed。
        let xml = chain_xml(
            r#"<connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="1" via=":J_2_0"/>
  <connection from=":J_2" to="east" fromLane="0" toLane="0"/>"#,
        );
        let network = parse_sumo_network_xml(&xml).expect("parse");
        let error = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect_err("middle-hop toLane mismatch must fail");
        assert!(
            error.to_string().contains("terminal"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn internal_chain_ignores_unrelated_destination_lane_continuation() {
        // #253 R5 反例 (ii)：A 有正确的 out_0 终端，同时存在不相关的
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
        let topology = normalize_junctions(&network, &StubWeldGate::Unrestricted)
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
        let error = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect_err("cycle must fail closed");
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
        let error = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect_err("missing terminal must fail closed");
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
        let error = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect_err("toLane mismatch must fail closed");
        assert!(
            error.to_string().contains("toLane"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn stub_weld_nonlocal_shape_retained_with_record() {
        // G1 六条件之独立几何门控：stub 形状对端点弦的最大偏离超 0.05 m
        // （本例中部点偏离约 0.27 m，非点状噪声）→ fail-closed，不进删焊。
        // 位移 0.022 m 过门控，确保触发的是局部性检查。
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6806.90,5727.53 6826.90,5727.53"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="0.40" shape="6806.88,5727.52 6806.90,5727.80 6806.95,5727.60 6807.00,5727.62 6807.18,5727.60"/></edge>
  <junction id="J" type="priority" intLanes=":J_0_0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0"/>
</net>"#;
        let network = parse_sumo_network_xml(xml).expect("parse");
        let topology = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect("non-local stub retained");
        assert_eq!(topology.stub_weld_records.len(), 1);
        let record = &topology.stub_weld_records[0];
        assert_eq!(record.disposition, StubWeldDisposition::RejectedLocality);
        assert!(record.detail.contains("not local"));
        assert!(
            topology.maneuver_paths[0]
                .internal_edge_ids
                .contains(&"sumo::J_0_0".to_owned())
        );
    }

    #[test]
    fn stub_weld_multiply_referenced_retained_with_records() {
        // G1 六条件之拓扑形态：stub 被两条 connection 的 via 引用（承载多处
        // 内部语义）→ fail-closed。
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="south" from="S" to="J"><lane id="south_0" index="0" speed="13.89" length="20.00" shape="6786.88,5737.52 6806.88,5727.72"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6806.90,5727.53 6826.90,5727.53"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="0.40" shape="6806.88,5727.52 6807.18,5727.60"/></edge>
  <junction id="J" type="priority" intLanes=":J_0_0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0"/>
  <connection from="south" to="east" fromLane="0" toLane="0" via=":J_0_0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0"/>
</net>"#;
        let network = parse_sumo_network_xml(xml).expect("parse");
        // 两条 connection 各自评估同一 stub：均被拒（via 引用数 2），均保留。
        let topology = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect("multiply referenced stub retained");
        assert_eq!(topology.stub_weld_records.len(), 2);
        for record in &topology.stub_weld_records {
            assert_eq!(record.disposition, StubWeldDisposition::RejectedTopology);
            assert!(record.detail.contains("carries internal semantics"));
        }
        assert!(topology.dropped_stub_lane_ids.is_empty());
    }

    #[test]
    fn stub_weld_shared_entry_breaking_retained_with_record() {
        // G1 六条件之共享影响复核：入口 west_0 还有一条正常穿越
        // west→east_1（后继 :J_2_0 首点距焊接目标 0.20 m > 5 mm join 容差），
        // 焊接会移动 west_0 全局端点、破坏该穿越 → fail-closed。
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6806.90,5727.53 6826.90,5727.53"/><lane id="east_1" index="1" speed="13.89" length="20.00" shape="6806.90,5730.53 6826.90,5730.53"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="0.40" shape="6806.88,5727.52 6807.18,5727.60"/></edge>
  <edge id=":J_2" function="internal"><lane id=":J_2_0" index="0" speed="13.89" length="5.00" shape="6806.90,5727.73 6811.88,5729.53"/></edge>
  <junction id="J" type="priority" intLanes=":J_0_0 :J_2_0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0"/>
  <connection from="west" to="east" fromLane="0" toLane="1" via=":J_2_0"/>
  <connection from=":J_2" to="east" fromLane="0" toLane="1"/>
</net>"#;
        let network = parse_sumo_network_xml(xml).expect("parse");
        let topology = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect("shared-breaking weld retained");
        assert_eq!(topology.stub_weld_records.len(), 1);
        let record = &topology.stub_weld_records[0];
        assert_eq!(record.disposition, StubWeldDisposition::RejectedSharedEntry);
        assert!(record.detail.contains("breaking shared traversal"));
        // 关联穿越明细：正常穿越 west→east_1 的后继间隙 0.20 m 记录在案。
        assert_eq!(record.shared_traversals.len(), 1);
        assert!((record.shared_traversals[0].2 - 0.20).abs() < 1e-6);
    }

    #[test]
    fn stub_weld_records_controlled_connection_fields() {
        // G1 可追溯记录：受控（tl/linkIndex）候选焊接成功时，逐条记录含
        // 规则版本、处置、受控标记与全部度量，字段同候选 manifest。
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6806.90,5727.53 6826.90,5727.53"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="0.40" shape="6806.88,5727.52 6807.18,5727.60"/></edge>
  <junction id="J" type="priority" intLanes=":J_0_0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0" tl="J" linkIndex="0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0"/>
</net>"#;
        let network = parse_sumo_network_xml(xml).expect("parse");
        let topology = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect("controlled stub welds");
        assert_eq!(topology.stub_weld_records.len(), 1);
        let record = &topology.stub_weld_records[0];
        assert_eq!(record.rule_version, STUB_WELD_RULE_VERSION);
        assert_eq!(record.disposition, StubWeldDisposition::Welded);
        assert!(record.controlled, "tl/linkIndex connection must be flagged");
        assert_eq!(record.stub_lane_id, ":J_0_0");
        assert_eq!(record.entry_lane_id, "west_0");
        assert_eq!(record.exit_lane_id, "east_0");
        assert!((record.displacement_m - 0.0224).abs() < 1e-3);
        assert!(record.span_m < 0.5);
        assert!(record.locality_m < 0.05);
        assert_eq!(record.shared_traversal_count, 0);
    }

    #[test]
    fn stub_weld_blocked_domain_refuses_and_retains() {
        // R8(a)：未验证来源（BlockedDomain 门控）+ 几何可焊形态 → 不焊、
        // 保留原始连接走普通穿越，记录 BlockedOutOfDomain。
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
        let topology =
            normalize_junctions(&network, &StubWeldGate::BlockedDomain).expect("normalize");
        assert!(
            topology.stub_welds.is_empty(),
            "blocked domain must not weld"
        );
        assert!(topology.dropped_stub_lane_ids.is_empty());
        assert_eq!(
            topology.maneuver_paths[0].internal_edge_ids,
            ["sumo::J_0_0".to_owned()],
            "原始连接保留"
        );
        assert_eq!(topology.stub_weld_records.len(), 1);
        let record = &topology.stub_weld_records[0];
        assert_eq!(record.disposition, StubWeldDisposition::BlockedOutOfDomain);
        assert!(record.detail.contains("not verify-source verified"));
    }

    #[test]
    fn stub_weld_verified_domain_rejects_unknown_identity() {
        // R8(b)：verified 域内 net digest 命中 manifest，但候选身份（重命名/
        // 同坐标新 id）不在批准集 → BlockedOutOfDomain，保留原始连接。
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
        let gate = StubWeldGate::VerifiedDomain {
            net_digest: Some(
                "sha256:6f5d76223cf14b797ae6267f13b23eb6c872d76adec1fb22a8569a806dc09341"
                    .to_owned(),
            ),
        };
        let topology = normalize_junctions(&network, &gate).expect("normalize");
        assert!(topology.stub_welds.is_empty());
        assert_eq!(topology.stub_weld_records.len(), 1);
        let record = &topology.stub_weld_records[0];
        assert_eq!(record.disposition, StubWeldDisposition::BlockedOutOfDomain);
        assert!(
            record.detail.contains("outside the approved pinned domain"),
            "unexpected detail: {}",
            record.detail
        );
        assert!(
            record.detail.contains("identity in approved set: false"),
            "unexpected detail: {}",
            record.detail
        );
    }

    #[test]
    fn duplicate_connection_quadruple_fails_closed() {
        // #253 C7：同 (from,fromLane,to,toLane) 但内链不同的两条穿越——
        // path_by_connection 此前静默替换前者（路由/信号绑定只能发现后者），
        // 现 fail-closed 报重复映射。
        let xml = chain_xml(
            r#"<connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_2_0"/>
  <connection from=":J_2" to="east" fromLane="0" toLane="0"/>"#,
        );
        let network = parse_sumo_network_xml(&xml).expect("parse");
        let error = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect_err("duplicate connection mapping must fail closed");
        assert!(
            error.to_string().contains("duplicate connection mapping"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn duplicate_lane_address_fails_closed() {
        // #253 M3：同 (edge, index) 声明两条不同 lane id——connection 按
        // edge+index 寻址会产生歧义绑定，fail-closed 报错（带两个 lane id）。
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/><lane id="west_alt" index="0" speed="13.89" length="20.00" shape="6786.88,5727.62 6806.88,5727.62"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/></edge>
  <junction id="J" type="priority" intLanes=""/>
  <connection from="west" to="east" fromLane="0" toLane="0"/>
</net>"#;
        let network = parse_sumo_network_xml(xml).expect("parse");
        let error = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect_err("duplicate lane address must fail closed");
        let message = error.to_string();
        assert!(message.contains("duplicate lane address"), "{message}");
        assert!(
            message.contains("west_0") && message.contains("west_alt"),
            "{message}"
        );
    }

    #[test]
    fn movement_id_delimiter_collision_fails_closed() {
        // #253 K5：movement id 是 from + "-to-" + to 拼接——(from="a-to-b", to="c")
        // 与 (from="a", to="b-to-c") 拼出同一串，歧义合并必须 fail-closed
        // （发射 id 格式不变，只检测碰撞）。
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="a-to-b" from="W1" to="J"><lane id="a-to-b_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="c" from="J" to="E1"><lane id="c_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/></edge>
  <edge id="a" from="W2" to="J"><lane id="a_0" index="0" speed="13.89" length="20.00" shape="6786.88,5737.52 6806.88,5737.52"/></edge>
  <edge id="b-to-c" from="J" to="E2"><lane id="b-to-c_0" index="0" speed="13.89" length="20.00" shape="6816.88,5737.52 6836.88,5737.52"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="5.00" shape="6806.88,5727.52 6811.88,5727.52"/></edge>
  <edge id=":J_1" function="internal"><lane id=":J_1_0" index="0" speed="13.89" length="5.00" shape="6806.88,5737.52 6811.88,5737.52"/></edge>
  <junction id="J" type="priority" intLanes=":J_0_0 :J_1_0"/>
  <connection from="a-to-b" to="c" fromLane="0" toLane="0" via=":J_0_0"/>
  <connection from=":J_0" to="c" fromLane="0" toLane="0"/>
  <connection from="a" to="b-to-c" fromLane="0" toLane="0" via=":J_1_0"/>
  <connection from=":J_1" to="b-to-c" fromLane="0" toLane="0"/>
</net>"#;
        let network = parse_sumo_network_xml(xml).expect("parse");
        let error = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect_err("delimiter collision must fail closed");
        let message = error.to_string();
        assert!(
            message.contains("collides between distinct movements"),
            "{message}"
        );
        assert!(
            message.contains("a-to-b") && message.contains("b-to-c"),
            "{message}"
        );
    }

    #[test]
    fn internal_classification_uses_function_metadata() {
        // #253 P4：internal 判定用 function 元数据而非 ':' 前缀。
        // (a) 无前缀但 function="internal" 的边：其 connection 按 internal
        // 跳过（旧前缀法误当外部 connection，报 internal lane endpoint）。
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/></edge>
  <edge id="weird" function="internal"><lane id="weird_0" index="0" speed="13.89" length="10.00" shape="6806.88,5727.52 6816.88,5727.52"/></edge>
  <junction id="J" type="priority" intLanes=""/>
  <connection from="west" to="east" fromLane="0" toLane="0" via="weird_0"/>
  <connection from="weird" to="east" fromLane="0" toLane="0"/>
</net>"#;
        let network = parse_sumo_network_xml(xml).expect("parse");
        let topology = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect("unprefixed internal edge is skipped as internal");
        assert_eq!(topology.maneuver_paths.len(), 1);
        assert_eq!(
            topology.maneuver_paths[0].internal_edge_ids,
            ["sumo:weird_0".to_owned()]
        );
    }

    #[test]
    fn colon_prefixed_non_internal_edge_is_external() {
        // (b) ':' 前缀但无 function="internal" 的边按外部处理——其 connection
        // 生成穿越（旧前缀法会把它当 internal 跳过）。
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id=":fake" from="W" to="J"><lane id=":fake_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="10.00" shape="6806.88,5727.52 6816.88,5727.52"/></edge>
  <junction id="J" type="priority" intLanes=":J_0_0"/>
  <connection from=":fake" to="east" fromLane="0" toLane="0" via=":J_0_0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0"/>
</net>"#;
        let network = parse_sumo_network_xml(xml).expect("parse");
        let topology = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect("colon-prefixed non-internal edge treated as external");
        assert_eq!(topology.maneuver_paths.len(), 1);
        assert_eq!(topology.maneuver_paths[0].entry_edge_id, "sumo::fake_0");
    }

    #[test]
    fn nested_internal_junction_parent_conflict_fails_closed() {
        // #253 P5：两个已归属 cluster 节点列出同一嵌套内部节点且父不同——
        // 旧实现按 XML 序静默跳过，fail-closed 报节点与两个父。
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="R1"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="R2" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/></edge>
  <junction id="R1" type="priority" intLanes="A"/>
  <junction id="R2" type="priority" intLanes="B"/>
  <junction id="A" type="internal" intLanes="N"/>
  <junction id="B" type="internal" intLanes="N"/>
  <junction id="N" type="internal" intLanes=""/>
</net>"#;
        let network = parse_sumo_network_xml(xml).expect("parse");
        let error = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect_err("conflicting nested parents must fail closed");
        let message = error.to_string();
        assert!(message.contains("conflicting cluster"), "{message}");
        assert!(
            message.contains("\"N\"") && message.contains("R1") && message.contains("R2"),
            "{message}"
        );
    }

    #[test]
    fn int_lanes_rejects_non_internal_lane_member() {
        // #253 V2：成员存在但非 internal（lane 无 function=internal）同样
        // fail-closed——存在性升级后为 internal 限定。
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <junction id="J" type="priority" intLanes="west_0"/>
</net>"#;
        let network = parse_sumo_network_xml(xml).expect("parse");
        let error = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect_err("non-internal lane member must fail");
        assert!(error.to_string().contains("is not internal"), "{error}");
    }

    #[test]
    fn maneuver_path_id_delimiter_collision_fails_closed() {
        // #253 V7：内边 join(".") 分隔符歧义——单 lane "a_0.b_0"（edge "a_0.b"）
        // 与链 ["a_0","b_0"] 拼出同一 path id，fail-closed 报撞名双方。
        // lane id 全部遵循 {edge}_{index} 约定（terminal 键依赖该约定）。
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/></edge>
  <edge id="a_0.b" function="internal"><lane id="a_0.b_0" index="0" speed="13.89" length="10.00" shape="6806.88,5727.52 6816.88,5727.52"/></edge>
  <edge id="a" function="internal"><lane id="a_0" index="0" speed="13.89" length="5.00" shape="6806.88,5727.52 6811.88,5727.52"/></edge>
  <edge id="b" function="internal"><lane id="b_0" index="0" speed="13.89" length="5.00" shape="6811.88,5727.52 6816.88,5727.52"/></edge>
  <junction id="J" type="priority" intLanes="a_0.b_0 a_0 b_0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via="a_0.b_0"/>
  <connection from="a_0.b" to="east" fromLane="0" toLane="0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via="a_0"/>
  <connection from="a" to="east" fromLane="0" toLane="0" via="b_0"/>
  <connection from="b" to="east" fromLane="0" toLane="0"/>
</net>"#;
        let network = parse_sumo_network_xml(xml).expect("parse");
        let error = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect_err("path id collision must fail");
        let message = error.to_string();
        assert!(message.contains("collides between"), "{message}");
    }

    #[test]
    fn chain_continuation_rejects_non_internal_lane() {
        // #253 V8：续接段 via lane 非 internal 即 fail-closed。
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="5.00" shape="6806.88,5727.52 6811.88,5727.52"/></edge>
  <edge id="fake" from="X" to="Y"><lane id="fake_0" index="0" speed="13.89" length="5.00" shape="6811.88,5727.52 6816.88,5727.52"/></edge>
  <junction id="J" type="priority" intLanes=":J_0_0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0" via="fake_0"/>
  <connection from="fake" to="east" fromLane="0" toLane="0"/>
</net>"#;
        let network = parse_sumo_network_xml(xml).expect("parse");
        let error = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect_err("non-internal continuation lane must fail");
        assert!(error.to_string().contains("non-internal"), "{error}");
    }

    #[test]
    fn int_lanes_dangling_member_fails_closed() {
        // #253 V2（T4 升级）：intLanes 拼错成员（既非 lane 也非 internal
        // junction）被无检查记为 owner 并可能静默忽略——fail-closed 报
        // junction 与成员。
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/></edge>
  <junction id="J" type="priority" intLanes="typo_lane_0"/>
</net>"#;
        let network = parse_sumo_network_xml(xml).expect("parse");
        let error = normalize_junctions(&network, &StubWeldGate::Unrestricted)
            .expect_err("dangling intLanes member must fail");
        let message = error.to_string();
        assert!(message.contains("is not internal"), "{message}");
        assert!(message.contains("typo_lane_0"), "{message}");
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
        let error =
            normalize_junctions(&network, &StubWeldGate::Unrestricted).expect_err("dangling via");
        assert!(error.to_string().contains("unknown lane"));
    }
}
