//! Convert SUMO static tlLogic programs into Traffic v0.8 Signals (§3.3).

use std::collections::{BTreeMap, HashMap};

use crate::{
    Error, Result,
    output::model::{
        ManeuverGate, SignalControl, SignalController, SignalGroup, SignalGroupState, SignalPhase,
        Signals, StopLine,
    },
    sumo::{SUMO_ID_PREFIX, SumoNetwork, SumoTlLogic},
};

/// 受控 link。字段声明序即 Ord 序（#253 U6：契约要求 group id 按成员
/// connection 词法序派生，linkIndex 仅作兜底——pinned 探针 87/201
/// controller 的 group 编号因此改变；group id 只进入 fail-fast 产物
/// network.lfca，诊断/计数产物不含其字节，锁定数字零漂移）。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct ControlledLink {
    tl_id: String,
    from_road_edge_id: String,
    from_lane_index: u32,
    to_road_edge_id: String,
    to_lane_index: u32,
    link_index: u32,
}

/// G1 六条件之控制语义守卫：被 stub 删焊移除的内边不得出现在任何信号绑定中
/// （stop line 的 edge、maneuver gate 的路径 id）。信号模型按运动（道路边）级
/// 绑定，内边本不该出现；本条 fail-closed 捕捉未来语义漂移。
pub(crate) fn validate_weld_signal_bindings(
    signals: &Signals,
    dropped_stub_lane_ids: &std::collections::HashSet<String>,
) -> Result<()> {
    if dropped_stub_lane_ids.is_empty() {
        return Ok(());
    }
    for stop_line in &signals.stop_lines {
        if dropped_stub_lane_ids.contains(&stop_line.edge_id) {
            return Err(Error::SumoModel(format!(
                "point-stub weld removed internal lane still bound as stop line {:?}",
                stop_line.id
            )));
        }
    }
    for gate in &signals.maneuver_gates {
        // 路径 id 内边段以 ':'/'..' 分隔；内边 id 去掉前导 ':' 做精确 token 比对。
        for stub in dropped_stub_lane_ids {
            let token = stub.strip_prefix(':').unwrap_or(stub);
            if gate
                .maneuver_path_id
                .split([':', '.'])
                .any(|part| part == token)
            {
                return Err(Error::SumoModel(format!(
                    "point-stub weld removed internal lane {stub:?} still bound by maneuver gate {:?}",
                    gate.id
                )));
            }
        }
    }
    Ok(())
}

/// 「声明臂无受控 link」source-health 事实（#253 K4a）：相位状态向量中无任何
/// connection 认领的位置（如 pinned controller `-13968` 缺 index 9）。不
/// fail——按 G1 契约修订记入 conversion report（明列 controller id 与缺失
/// index）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnclaimedSignalArm {
    pub controller_id: String,
    pub missing_link_indices: Vec<u32>,
}

/// Build Signals from network controlled connections + static tll programs.
/// 返回值附带「声明臂无受控 link」health 事实（可能为空）。
pub fn convert_signals(
    network: &SumoNetwork,
    tll_programs: &[SumoTlLogic],
    path_by_connection: &HashMap<(String, u32, String, u32), String>,
) -> Result<(Signals, Vec<UnclaimedSignalArm>)> {
    let controlled = collect_controlled_links(network)?;
    if controlled.is_empty() {
        if !tll_programs.is_empty() {
            return Err(Error::SumoModel(
                "tll.static.xml declares controllers but network has no tl/linkIndex connections"
                    .to_owned(),
            ));
        }
        // #253 M4：network 内嵌 tlLogic 声明了 controller 却无任何受控
        // link——不一致的信号源不得静默变成无信号网络，fail-closed 报
        // controller id。
        let declared = network.net_tl_logic_ids();
        if !declared.is_empty() {
            return Err(Error::SumoModel(format!(
                "network declares tlLogic controllers {declared:?} but has no controlled tl/linkIndex connections"
            )));
        }
        return Ok((empty_signals(), Vec::new()));
    }

    let net_ids = network.net_tl_logic_ids();
    let mut tll_ids = tll_programs
        .iter()
        .map(|logic| logic.id.clone())
        .collect::<Vec<_>>();
    tll_ids.sort();
    if net_ids != tll_ids {
        return Err(Error::SumoModel(format!(
            "network tlLogic IDs and tll.static.xml IDs do not close exactly: net={net_ids:?} tll={tll_ids:?}"
        )));
    }

    let program_by_id = tll_programs
        .iter()
        .map(|logic| (logic.id.as_str(), logic))
        .collect::<HashMap<_, _>>();

    let mut links_by_tl: BTreeMap<&str, Vec<&ControlledLink>> = BTreeMap::new();
    for link in &controlled {
        links_by_tl
            .entry(link.tl_id.as_str())
            .or_default()
            .push(link);
    }
    // #253 W4：逐 controller 声明-绑定闭环——links_by_tl 只迭代有受控 link 的
    // 键，「net+tll 都声明了、connection 却没绑定」的 controller 此前被静默
    // 漏掉（M4 的空 controlled 全局检查覆盖不到）；不一致的信号源 fail-closed
    // 报未绑定 controller id。
    let bound: std::collections::BTreeSet<&str> = links_by_tl.keys().copied().collect();
    let unbound = net_ids
        .iter()
        .map(String::as_str)
        .filter(|id| !bound.contains(id))
        .collect::<Vec<_>>();
    if !unbound.is_empty() {
        return Err(Error::SumoModel(format!(
            "tlLogic controllers {unbound:?} declared but with no controlled tl/linkIndex connections"
        )));
    }
    for (tl_id, links) in &links_by_tl {
        if !program_by_id.contains_key(tl_id) {
            return Err(Error::SumoModel(format!(
                "controlled connections reference unknown controller {tl_id:?}"
            )));
        }
        let mut indices = links.iter().map(|link| link.link_index).collect::<Vec<_>>();
        indices.sort_unstable();
        if indices.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(Error::SumoModel(format!(
                "controller {tl_id:?} has duplicate linkIndex values"
            )));
        }
    }

    // StopLine 必须绑**声明**车道 id（LaneEdge 用 lane.id，见 SumoLane::
    // laneflow_id）——按 `{edge}_{index}` 约定拼造会在声明 id 偏离约定时
    // 制造悬空引用；诊断模式不过 compiler finish，无人能事后察觉。
    let lane_id_by_edge_index: HashMap<(&str, u32), &str> = network
        .lanes
        .iter()
        .map(|lane| ((lane.edge_id.as_str(), lane.index), lane.id.as_str()))
        .collect();

    let mut unclaimed_arms = Vec::new();
    let mut stop_lines = Vec::new();
    let mut gates = Vec::new();
    let mut groups = Vec::new();
    let mut controllers = Vec::new();

    for (tl_id, mut links) in links_by_tl {
        links.sort();
        let program = program_by_id
            .get(tl_id)
            .copied()
            .expect("controller presence checked");
        if program.logic_type != "static" {
            return Err(Error::SumoModel(format!(
                "tll program {tl_id:?} type must be static, got {:?}",
                program.logic_type
            )));
        }
        validate_program_states(program, &links)?;
        // K4a：相位向量中无受控 link 认领的位置（声明臂）不 fail，收集为
        // health 事实（link 越界仍由 validate_program_states 严格拒绝）。
        // Q5：按 char 计数（与相位向量位置语义一致）。
        let state_len = program
            .phases
            .first()
            .map(|phase| phase.state.chars().count())
            .unwrap_or(0);
        let claimed: std::collections::HashSet<u32> =
            links.iter().map(|link| link.link_index).collect();
        let missing: Vec<u32> = (0..state_len as u32)
            .filter(|index| !claimed.contains(index))
            .collect();
        if !missing.is_empty() {
            unclaimed_arms.push(UnclaimedSignalArm {
                controller_id: tl_id.to_owned(),
                missing_link_indices: missing,
            });
        }

        let equivalence = build_groups(program, &links);
        let mut group_entries = equivalence.into_iter().collect::<Vec<_>>();
        group_entries.sort_by(|left, right| left.1.cmp(&right.1));

        let mut group_ids = Vec::with_capacity(group_entries.len());
        let mut group_id_by_link = HashMap::new();
        for (index, (_vector, members)) in group_entries.iter().enumerate() {
            let group_id = format!("{SUMO_ID_PREFIX}{tl_id}:group-{index}");
            for member in members {
                group_id_by_link.insert(*member, group_id.clone());
            }
            group_ids.push(group_id.clone());
            groups.push(SignalGroup { id: group_id });
        }

        // #253 C4：StopLine 按 (from_edge, from_lane) 逐受控车道建立——compiler
        // 的 ManeuverGateStopLineMismatch 是 LaneEdge 粒度（gate 的停止线必须
        // 位于 pathEdges[transitionIndex] 同一边）；按 from-edge 共享 min-lane
        // 线会让多车道进口道的车道 1/2 gate 绑到车道 0 的线上。
        let mut from_lane_list = links
            .iter()
            .map(|link| (link.from_road_edge_id.clone(), link.from_lane_index))
            .collect::<Vec<_>>();
        from_lane_list.sort();
        from_lane_list.dedup();
        let mut stop_line_by_from_lane = HashMap::new();
        for (from_edge, from_lane) in from_lane_list {
            let declared_lane_id = lane_id_by_edge_index
                .get(&(from_edge.as_str(), from_lane))
                .ok_or_else(|| {
                    Error::SumoModel(format!(
                        "controlled link from lane {from_edge}:{from_lane} not found among parsed lanes"
                    ))
                })?;
            let stop_line_id = format!("{SUMO_ID_PREFIX}stop:{from_edge}_{from_lane}");
            let edge_id = format!("{SUMO_ID_PREFIX}{declared_lane_id}");
            stop_lines.push(StopLine {
                id: stop_line_id.clone(),
                edge_id,
                location: "edgeEnd",
            });
            stop_line_by_from_lane.insert((from_edge, from_lane), stop_line_id);
        }

        for link in &links {
            let path_id = path_by_connection
                .get(&(
                    link.from_road_edge_id.clone(),
                    link.from_lane_index,
                    link.to_road_edge_id.clone(),
                    link.to_lane_index,
                ))
                .ok_or_else(|| {
                    Error::SumoModel(format!(
                        "controlled connection {:?}/{}->{:?}/{} has no unique ManeuverPath",
                        link.from_road_edge_id,
                        link.from_lane_index,
                        link.to_road_edge_id,
                        link.to_lane_index
                    ))
                })?;
            let stop_line_id = stop_line_by_from_lane
                .get(&(link.from_road_edge_id.clone(), link.from_lane_index))
                .expect("stop line created for from lane")
                .clone();
            let group_id = group_id_by_link
                .get(link)
                .expect("every controlled link belongs to a group")
                .clone();
            gates.push(ManeuverGate {
                id: format!(
                    "{SUMO_ID_PREFIX}gate:{}:{}-to-{}:{}",
                    link.tl_id, link.from_road_edge_id, link.to_road_edge_id, link.link_index
                ),
                maneuver_path_id: path_id.clone(),
                transition_index: 0,
                stop_line_id,
                signal_control: SignalControl {
                    kind: "group",
                    group_id,
                },
            });
        }

        let mut phases = Vec::with_capacity(program.phases.len());
        for (phase_index, phase) in program.phases.iter().enumerate() {
            let duration_ms = phase.duration.to_strict_positive_millis()?;
            let mut states = Vec::with_capacity(group_ids.len());
            for (group_id, members) in group_ids
                .iter()
                .zip(group_entries.iter().map(|entry| &entry.1))
            {
                let representative = members[0];
                let ch = phase_state_char(phase, representative.link_index)?;
                states.push(SignalGroupState {
                    group_id: group_id.clone(),
                    aspect: map_aspect(ch)?,
                });
            }
            phases.push(SignalPhase {
                id: format!("{SUMO_ID_PREFIX}{tl_id}:phase-{phase_index}"),
                duration_ms,
                states,
            });
        }

        controllers.push(SignalController {
            id: format!("{SUMO_ID_PREFIX}{tl_id}"),
            kind: "fixedTime",
            offset_ms: program.offset.to_non_negative_millis()?,
            group_ids,
            phases,
        });
    }

    stop_lines.sort_by(|left, right| left.id.cmp(&right.id));
    gates.sort_by(|left, right| left.id.cmp(&right.id));
    groups.sort_by(|left, right| left.id.cmp(&right.id));
    controllers.sort_by(|left, right| left.id.cmp(&right.id));

    Ok((
        Signals {
            stop_lines,
            maneuver_gates: gates,
            groups,
            controllers,
        },
        unclaimed_arms,
    ))
}

fn empty_signals() -> Signals {
    Signals {
        stop_lines: Vec::new(),
        maneuver_gates: Vec::new(),
        groups: Vec::new(),
        controllers: Vec::new(),
    }
}

fn collect_controlled_links(network: &SumoNetwork) -> Result<Vec<ControlledLink>> {
    let mut links = Vec::new();
    for connection in &network.connections {
        let Some(tl_id) = connection.tl_id.as_ref() else {
            continue;
        };
        let from_edge = network.edge(&connection.from_edge_id).ok_or_else(|| {
            Error::SumoModel(format!(
                "signalized connection from unknown edge {:?}",
                connection.from_edge_id
            ))
        })?;
        let to_edge = network.edge(&connection.to_edge_id).ok_or_else(|| {
            Error::SumoModel(format!(
                "signalized connection to unknown edge {:?}",
                connection.to_edge_id
            ))
        })?;
        // #253 U4：带 tl/linkIndex 的 connection 端点为 internal edge 此前
        // 被静默跳过——其相位位置会被误报为 unclaimed arm（pinned 探针 0
        // 实例），fail-closed 报 controller 与 connection。
        if from_edge.function_internal || to_edge.function_internal {
            return Err(Error::SumoModel(format!(
                "signalized connection {:?}/{} -> {:?}/{} under controller {tl_id:?} resolves to internal edges",
                connection.from_edge_id,
                connection.from_lane,
                connection.to_edge_id,
                connection.to_lane
            )));
        }
        let link_index = connection.link_index.ok_or_else(|| {
            Error::SumoModel(format!(
                "signalized connection {:?}->{:?} missing linkIndex",
                connection.from_edge_id, connection.to_edge_id
            ))
        })?;
        links.push(ControlledLink {
            tl_id: tl_id.clone(),
            link_index,
            from_road_edge_id: connection.from_edge_id.clone(),
            from_lane_index: connection.from_lane,
            to_road_edge_id: connection.to_edge_id.clone(),
            to_lane_index: connection.to_lane,
        });
    }
    links.sort();
    Ok(links)
}

fn validate_program_states(program: &SumoTlLogic, links: &[&ControlledLink]) -> Result<()> {
    // #253 L2/Q5：相位状态串按 **char** 计数且等宽先行校验（非 ASCII 的
    // 字节宽度会产生幻影 index）；逐字符校验 SUMO 信号字母表——未知字符
    // 不得被 unclaimed-arm 例外静默放过。
    const SIGNAL_ALPHABET: [char; 7] = ['G', 'g', 'y', 'u', 'r', 'o', 'O'];
    let widths: Vec<usize> = program
        .phases
        .iter()
        .map(|phase| phase.state.chars().count())
        .collect();
    if let Some(&first) = widths.first()
        && widths.iter().any(|&width| width != first)
    {
        return Err(Error::SumoModel(format!(
            "tlLogic {:?} phase states have inconsistent widths {widths:?}",
            program.id
        )));
    }
    for (phase_index, phase) in program.phases.iter().enumerate() {
        for (position, ch) in phase.state.chars().enumerate() {
            if !SIGNAL_ALPHABET.contains(&ch) {
                return Err(Error::SumoModel(format!(
                    "tlLogic {:?} phase {phase_index} has unsupported signal state character {ch:?} at index {position}",
                    program.id
                )));
            }
        }
    }
    let max_index = links
        .iter()
        .map(|link| link.link_index)
        .max()
        .expect("controller has links");
    for phase in &program.phases {
        if phase.state.chars().count() <= max_index as usize {
            return Err(Error::SumoModel(format!(
                "tlLogic {:?} phase state {:?} is shorter than linkIndex {max_index}",
                program.id, phase.state
            )));
        }
        for link in links {
            let _ = phase_state_char(phase, link.link_index)?;
        }
    }
    Ok(())
}

fn build_groups<'a>(
    program: &SumoTlLogic,
    links: &[&'a ControlledLink],
) -> HashMap<Vec<char>, Vec<&'a ControlledLink>> {
    let mut groups: HashMap<Vec<char>, Vec<&ControlledLink>> = HashMap::new();
    for link in links {
        let vector = program
            .phases
            .iter()
            .map(|phase| {
                phase
                    .state
                    .chars()
                    .nth(link.link_index as usize)
                    .expect("validated")
            })
            .collect::<Vec<_>>();
        groups.entry(vector).or_default().push(link);
    }
    for members in groups.values_mut() {
        members.sort();
    }
    groups
}

fn phase_state_char(phase: &crate::sumo::net::SumoTlPhase, link_index: u32) -> Result<char> {
    phase.state.chars().nth(link_index as usize).ok_or_else(|| {
        Error::SumoModel(format!(
            "phase state {:?} missing linkIndex {link_index}",
            phase.state
        ))
    })
}

fn map_aspect(ch: char) -> Result<&'static str> {
    match ch {
        'G' | 'g' => Ok("green"),
        'y' | 'u' => Ok("yellow"),
        'r' | 'o' | 'O' => Ok("red"),
        other => Err(Error::SumoModel(format!(
            "unsupported SUMO signal state character {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        convert::junction::{StubWeldGate, normalize_junctions},
        sumo::{parse_sumo_network_xml, parse_tll_static_xml},
    };

    #[test]
    fn declared_controller_without_links_fails_closed() {
        // #253 M4：network 内嵌 tlLogic 声明了 controller 却无任何受控
        // connection——不一致的信号源不得静默变成无信号网络。
        let net = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/></edge>
  <junction id="J" type="priority" intLanes=""/>
  <connection from="west" to="east" fromLane="0" toLane="0"/>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="G"/>
  </tlLogic>
</net>"#;
        let network = parse_sumo_network_xml(net).expect("parse net");
        let error = super::convert_signals(&network, &[], &Default::default())
            .expect_err("declared controller without links must fail closed");
        assert!(
            error.to_string().contains("\"J\""),
            "error must name the controller: {error}"
        );
    }

    #[test]
    fn unsupported_phase_alphabet_char_fails_closed() {
        // #253 Q5：未知相位字符不得被 unclaimed-arm 例外静默放过。
        let net = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="10.00" shape="6806.88,5727.52 6816.88,5727.52"/></edge>
  <junction id="J" type="traffic_light" intLanes=":J_0_0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0" tl="J" linkIndex="0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0"/>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="GX"/>
  </tlLogic>
</net>"#;
        let network = parse_sumo_network_xml(net).expect("parse net");
        let topology =
            normalize_junctions(&network, &StubWeldGate::Unrestricted).expect("normalize");
        let tll = parse_tll_static_xml(
            r#"<additional>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="GX"/>
  </tlLogic>
</additional>"#,
        )
        .expect("parse tll");
        let error = super::convert_signals(&network, &tll, &topology.path_by_connection)
            .expect_err("unsupported phase alphabet char must fail");
        assert!(
            error.to_string().contains("unsupported signal state"),
            "{error}"
        );
    }

    #[test]
    fn group_ids_follow_connection_lexicographic_order() {
        // #253 U6：group id 按成员 connection 词法序派生（linkIndex 兜底）。
        // aaa(linkIndex 5, G) 与 bbb(linkIndex 2, R) 两个 group：旧
        // linkIndex 优先序会让 bbb 拿 group-0，新词法序让 aaa 拿 group-0。
        let net = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="aaa" from="W1" to="J"><lane id="aaa_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="bbb" from="W2" to="J"><lane id="bbb_0" index="0" speed="13.89" length="20.00" shape="6786.88,5737.52 6806.88,5737.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="10.00" shape="6806.88,5727.52 6816.88,5727.52"/></edge>
  <edge id=":J_1" function="internal"><lane id=":J_1_0" index="0" speed="13.89" length="10.00" shape="6806.88,5737.52 6816.88,5737.52"/></edge>
  <junction id="J" type="traffic_light" intLanes=":J_0_0 :J_1_0"/>
  <connection from="aaa" to="east" fromLane="0" toLane="0" via=":J_0_0" tl="J" linkIndex="5"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0"/>
  <connection from="bbb" to="east" fromLane="0" toLane="0" via=":J_1_0" tl="J" linkIndex="2"/>
  <connection from=":J_1" to="east" fromLane="0" toLane="0"/>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="rrrrrG"/>
  </tlLogic>
</net>"#;
        let network = parse_sumo_network_xml(net).expect("parse net");
        let topology =
            normalize_junctions(&network, &StubWeldGate::Unrestricted).expect("normalize");
        let tll = parse_tll_static_xml(
            r#"<additional>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="rrrrrG"/>
  </tlLogic>
</additional>"#,
        )
        .expect("parse tll");
        let (signals, unclaimed) =
            super::convert_signals(&network, &tll, &topology.path_by_connection)
                .expect("convert signals");
        assert_eq!(
            unclaimed,
            vec![super::UnclaimedSignalArm {
                controller_id: "J".to_owned(),
                missing_link_indices: vec![0, 1, 3, 4],
            }],
            "state len 6 只认领 index 2/5"
        );
        let gate_aaa = signals
            .maneuver_gates
            .iter()
            .find(|gate| gate.id.ends_with(":5"))
            .expect("gate for aaa");
        let gate_bbb = signals
            .maneuver_gates
            .iter()
            .find(|gate| gate.id.ends_with(":2"))
            .expect("gate for bbb");
        assert_eq!(gate_aaa.signal_control.group_id, "sumo:J:group-0");
        assert_eq!(gate_bbb.signal_control.group_id, "sumo:J:group-1");
    }

    #[test]
    fn inconsistent_phase_widths_fail_closed() {
        // #253 L2：相位状态串不等宽即 fail-closed（unclaimed arm 收集与
        // linkIndex 校验以等宽为前提）。
        let net = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="10.00" shape="6806.88,5727.52 6816.88,5727.52"/></edge>
  <junction id="J" type="traffic_light" intLanes=":J_0_0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0" tl="J" linkIndex="0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0"/>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="G"/>
    <phase duration="4" state="Gr"/>
  </tlLogic>
</net>"#;
        let network = parse_sumo_network_xml(net).expect("parse net");
        let topology =
            normalize_junctions(&network, &StubWeldGate::Unrestricted).expect("normalize");
        let tll = parse_tll_static_xml(
            r#"<additional>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="G"/>
    <phase duration="4" state="Gr"/>
  </tlLogic>
</additional>"#,
        )
        .expect("parse tll");
        let error = super::convert_signals(&network, &tll, &topology.path_by_connection)
            .expect_err("inconsistent phase widths must fail closed");
        assert!(error.to_string().contains("inconsistent widths"), "{error}");
    }

    #[test]
    fn unclaimed_signal_arm_becomes_health_fact_not_failure() {
        // #253 K4a：相位向量中的声明臂（无受控 link 的位置）不 fail——转换
        // 成功并返回 health 事实（controller id + 缺失 index）。
        let net = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/><lane id="east_1" index="1" speed="13.89" length="20.00" shape="6816.88,5730.52 6836.88,5730.52"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="5.00" shape="6806.88,5727.52 6811.88,5727.52"/></edge>
  <edge id=":J_1" function="internal"><lane id=":J_1_0" index="0" speed="13.89" length="5.00" shape="6806.88,5727.52 6811.88,5729.52"/></edge>
  <junction id="J" type="traffic_light" intLanes=":J_0_0 :J_1_0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0" tl="J" linkIndex="0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0"/>
  <connection from="west" to="east" fromLane="0" toLane="1" via=":J_1_0" tl="J" linkIndex="2"/>
  <connection from=":J_1" to="east" fromLane="0" toLane="1"/>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="GrG"/>
  </tlLogic>
</net>"#;
        let network = parse_sumo_network_xml(net).expect("parse net");
        let topology =
            normalize_junctions(&network, &StubWeldGate::Unrestricted).expect("normalize");
        let tll = parse_tll_static_xml(
            r#"<additional>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="GrG"/>
  </tlLogic>
</additional>"#,
        )
        .expect("parse tll");
        let (signals, unclaimed) =
            super::convert_signals(&network, &tll, &topology.path_by_connection)
                .expect("unclaimed arm is a health fact, not a failure");
        assert_eq!(signals.controllers.len(), 1);
        assert_eq!(
            unclaimed,
            vec![super::UnclaimedSignalArm {
                controller_id: "J".to_owned(),
                missing_link_indices: vec![1],
            }]
        );
    }

    #[test]
    fn link_index_beyond_phase_vector_still_fails_closed() {
        // K4a 严格性保留：受控 link 的 index 超出相位向量仍 fail。
        let net = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="5.00" shape="6806.88,5727.52 6811.88,5727.52"/></edge>
  <junction id="J" type="traffic_light" intLanes=":J_0_0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0" tl="J" linkIndex="3"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0"/>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="GrG"/>
  </tlLogic>
</net>"#;
        let network = parse_sumo_network_xml(net).expect("parse net");
        let topology =
            normalize_junctions(&network, &StubWeldGate::Unrestricted).expect("normalize");
        let tll = parse_tll_static_xml(
            r#"<additional>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="GrG"/>
  </tlLogic>
</additional>"#,
        )
        .expect("parse tll");
        let error = super::convert_signals(&network, &tll, &topology.path_by_connection)
            .expect_err("link index beyond phase vector must fail");
        assert!(error.to_string().contains("linkIndex 3"), "{error}");
    }

    #[test]
    fn multi_lane_controlled_approach_gets_stop_line_per_lane() {
        // #253 C4：compiler 的 ManeuverGateStopLineMismatch 是 LaneEdge 粒度
        // （hir/control.rs）——多车道受控进口道必须每车道一条 StopLine，gate
        // 绑本车道线，否则车道 1/2 的 gate 落在车道 0 的线上。
        let net = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/><lane id="west_1" index="1" speed="13.89" length="20.00" shape="6786.88,5730.52 6806.88,5730.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/><lane id="east_1" index="1" speed="13.89" length="20.00" shape="6816.88,5730.52 6836.88,5730.52"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="5.00" shape="6806.88,5727.52 6811.88,5727.52"/></edge>
  <edge id=":J_1" function="internal"><lane id=":J_1_0" index="0" speed="13.89" length="5.00" shape="6806.88,5730.52 6811.88,5730.52"/></edge>
  <junction id="J" type="traffic_light" intLanes=":J_0_0 :J_1_0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0" tl="J" linkIndex="0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0"/>
  <connection from="west" to="east" fromLane="1" toLane="1" via=":J_1_0" tl="J" linkIndex="1"/>
  <connection from=":J_1" to="east" fromLane="0" toLane="1"/>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="GG"/>
  </tlLogic>
</net>"#;
        let network = parse_sumo_network_xml(net).expect("parse net");
        let topology =
            normalize_junctions(&network, &StubWeldGate::Unrestricted).expect("normalize");
        let tll = parse_tll_static_xml(
            r#"<additional>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="GG"/>
  </tlLogic>
</additional>"#,
        )
        .expect("parse tll");
        let (signals, unclaimed) =
            super::convert_signals(&network, &tll, &topology.path_by_connection)
                .expect("convert signals");
        assert!(unclaimed.is_empty(), "fixture claims every arm");
        // 每受控车道一条 StopLine，绑本车道边。
        assert_eq!(signals.stop_lines.len(), 2, "per-lane stop lines");
        assert!(
            signals
                .stop_lines
                .iter()
                .any(|line| line.edge_id == "sumo:west_0")
        );
        assert!(
            signals
                .stop_lines
                .iter()
                .any(|line| line.edge_id == "sumo:west_1")
        );
        // 每个 gate 绑本车道的 StopLine（linkIndex 后缀区分 gate）。
        let gate0 = signals
            .maneuver_gates
            .iter()
            .find(|gate| gate.id.ends_with(":0"))
            .expect("gate for lane 0");
        let gate1 = signals
            .maneuver_gates
            .iter()
            .find(|gate| gate.id.ends_with(":1"))
            .expect("gate for lane 1");
        assert_eq!(gate0.stop_line_id, "sumo:stop:west_0");
        assert_eq!(gate1.stop_line_id, "sumo:stop:west_1");
    }

    #[test]
    fn stop_line_binds_declared_lane_id_not_convention() {
        // 声明车道 id 偏离 `{edge}_{index}` 约定（SUMO 合法）时，StopLine.
        // edge_id 必须引用声明 id（LaneEdge 的 id 来源）——拼造约定 id 会
        // 制造悬空引用，而诊断模式不过 compiler finish，无人能察觉。
        let net = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="0,0" convBoundary="0.00,0.00,100.00,100.00"/>
  <edge id="west" from="W" to="J"><lane id="westDeclared" index="0" speed="13.89" length="20.00" shape="0.00,0.00 20.00,0.00"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="30.00,0.00 50.00,0.00"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="5.00" shape="20.00,0.00 25.00,0.00"/></edge>
  <junction id="J" type="traffic_light" intLanes=":J_0_0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0" tl="J" linkIndex="0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0"/>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="G"/>
  </tlLogic>
</net>"#;
        let network = parse_sumo_network_xml(net).expect("parse net");
        let topology =
            normalize_junctions(&network, &StubWeldGate::Unrestricted).expect("normalize");
        let tll = parse_tll_static_xml(
            r#"<additional>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="G"/>
  </tlLogic>
</additional>"#,
        )
        .expect("parse tll");
        let (signals, _) = super::convert_signals(&network, &tll, &topology.path_by_connection)
            .expect("convert signals");
        assert_eq!(signals.stop_lines.len(), 1);
        assert_eq!(
            signals.stop_lines[0].edge_id, "sumo:westDeclared",
            "StopLine 必须引用声明车道 id"
        );
    }
}

#[cfg(test)]
mod k7_count_tests {
    #[test]
    fn signal_phase_count_reaches_topology_counts() {
        // K7：相位计数穿 TopologyCounts（fixture：1 controller × 2 phases）。
        use crate::sumo::{parse_sumo_network_xml, parse_tll_static_xml};
        let net = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="10.00" shape="6806.88,5727.52 6816.88,5727.52"/></edge>
  <junction id="J" type="traffic_light" intLanes=":J_0_0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0" tl="J" linkIndex="0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0"/>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="G"/>
    <phase duration="4" state="y"/>
  </tlLogic>
</net>"#;
        let network = parse_sumo_network_xml(net).expect("parse net");
        let tll = parse_tll_static_xml(
            r#"<additional>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="G"/>
    <phase duration="4" state="y"/>
  </tlLogic>
</additional>"#,
        )
        .expect("parse tll");
        let artifacts = crate::convert::topology::convert_network_topology_with_tll_and_profiles(
            &network,
            &tll,
            &[],
            &crate::convert::topology::TopologyConvertOptions::default(),
            crate::output::geom::ReportSource::unverified_unknown(),
        )
        .expect("fixture compiles");
        assert_eq!(artifacts.counts.signal_phases, 2);
    }
}

#[cfg(test)]
mod w4_binding_tests {
    use crate::sumo::{parse_sumo_network_xml, parse_tll_static_xml};

    // net+tll 均声明 controller A/B，但只有 A 绑定了受控 connection。
    const NET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="0,0" convBoundary="0,0,100,100"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="0,0 20,0"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="30,0 50,0"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="10.00" shape="20,0 30,0"/></edge>
  <junction id="J" type="traffic_light" intLanes=":J_0_0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0" tl="A" linkIndex="0"/>
  <tlLogic id="A" type="static" programID="1" offset="0">
    <phase duration="31" state="G"/>
  </tlLogic>
  <tlLogic id="B" type="static" programID="1" offset="0">
    <phase duration="31" state="G"/>
  </tlLogic>
</net>"#;

    const TLL: &str = r#"<additional>
  <tlLogic id="A" type="static" programID="1" offset="0">
    <phase duration="31" state="G"/>
  </tlLogic>
  <tlLogic id="B" type="static" programID="1" offset="0">
    <phase duration="31" state="G"/>
  </tlLogic>
</additional>"#;

    #[test]
    fn declared_controller_without_links_fails_closed() {
        // #253 W4：links_by_tl 只迭代有受控 link 的键——声明了 B 却无人绑定
        // 此前被静默漏掉；逐 controller 闭环 fail-closed 报未绑定者，已绑定
        // 的 A 不得被误报。
        let network = parse_sumo_network_xml(NET).expect("parse net");
        let tll = parse_tll_static_xml(TLL).expect("parse tll");
        let error = super::convert_signals(&network, &tll, &std::collections::HashMap::new())
            .expect_err("unbound declared controller must fail closed");
        let message = error.to_string();
        assert!(message.contains("no controlled tl/linkIndex"), "{message}");
        assert!(message.contains("\"B\""), "{message}");
        assert!(!message.contains("\"A\""), "{message}");
    }
}
