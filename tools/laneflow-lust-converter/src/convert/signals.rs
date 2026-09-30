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

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct ControlledLink {
    tl_id: String,
    link_index: u32,
    from_road_edge_id: String,
    from_lane_index: u32,
    to_road_edge_id: String,
    to_lane_index: u32,
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
        let state_len = program
            .phases
            .first()
            .map(|phase| phase.state.len())
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
            let stop_line_id = format!("{SUMO_ID_PREFIX}stop:{from_edge}_{from_lane}");
            let edge_id = format!("{SUMO_ID_PREFIX}{from_edge}_{from_lane}");
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
        if from_edge.function_internal || to_edge.function_internal {
            continue;
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
    // #253 L2：相位状态串等宽先行校验——不等宽的位置语义不可判（unclaimed
    // arm 收集与 linkIndex 越界检查都以等宽为前提），fail-closed 报各宽度。
    let widths: Vec<usize> = program
        .phases
        .iter()
        .map(|phase| phase.state.len())
        .collect();
    if let Some(&first) = widths.first()
        && widths.iter().any(|&width| width != first)
    {
        return Err(Error::SumoModel(format!(
            "tlLogic {:?} phase states have inconsistent widths {widths:?}",
            program.id
        )));
    }
    let max_index = links
        .iter()
        .map(|link| link.link_index)
        .max()
        .expect("controller has links");
    for phase in &program.phases {
        if phase.state.len() <= max_index as usize {
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
        let artifacts = crate::convert_network_topology_with_tll(
            &network,
            &tll,
            &crate::TopologyConvertOptions::default(),
        )
        .expect("fixture compiles");
        assert_eq!(artifacts.counts.signal_phases, 2);
    }
}
