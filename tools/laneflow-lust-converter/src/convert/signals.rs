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

/// Build Signals from network controlled connections + static tll programs.
pub fn convert_signals(
    network: &SumoNetwork,
    tll_programs: &[SumoTlLogic],
    path_by_connection: &HashMap<(String, u32, String, u32), String>,
) -> Result<Signals> {
    let controlled = collect_controlled_links(network)?;
    if controlled.is_empty() {
        if !tll_programs.is_empty() {
            return Err(Error::SumoModel(
                "tll.static.xml declares controllers but network has no tl/linkIndex connections"
                    .to_owned(),
            ));
        }
        return Ok(empty_signals());
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
        links_by_tl.entry(link.tl_id.as_str()).or_default().push(link);
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

    let mut stop_lines = Vec::new();
    let mut stop_line_by_from_edge = HashMap::new();
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

        let mut from_edge_list = links
            .iter()
            .map(|link| link.from_road_edge_id.clone())
            .collect::<Vec<_>>();
        from_edge_list.sort();
        from_edge_list.dedup();
        for from_edge in from_edge_list {
            if stop_line_by_from_edge.contains_key(&from_edge) {
                continue;
            }
            let stop_lane_index = links
                .iter()
                .filter(|link| link.from_road_edge_id == from_edge)
                .map(|link| link.from_lane_index)
                .min()
                .expect("from edge has at least one controlled link");
            let stop_line_id = format!("{SUMO_ID_PREFIX}stop:{from_edge}");
            let edge_id = format!("{SUMO_ID_PREFIX}{from_edge}_{stop_lane_index}");
            stop_lines.push(StopLine {
                id: stop_line_id.clone(),
                edge_id,
                location: "edgeEnd",
            });
            stop_line_by_from_edge.insert(from_edge, stop_line_id);
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
            let stop_line_id = stop_line_by_from_edge
                .get(&link.from_road_edge_id)
                .expect("stop line created for from edge")
                .clone();
            let group_id = group_id_by_link
                .get(link)
                .expect("every controlled link belongs to a group")
                .clone();
            gates.push(ManeuverGate {
                id: format!(
                    "{SUMO_ID_PREFIX}gate:{}:{}-to-{}:{}",
                    link.tl_id,
                    link.from_road_edge_id,
                    link.to_road_edge_id,
                    link.link_index
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
            for (group_id, members) in group_ids.iter().zip(group_entries.iter().map(|entry| &entry.1))
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

    Ok(Signals {
        stop_lines,
        maneuver_gates: gates,
        groups,
        controllers,
    })
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
            .map(|phase| phase.state.chars().nth(link.link_index as usize).expect("validated"))
            .collect::<Vec<_>>();
        groups.entry(vector).or_default().push(link);
    }
    for members in groups.values_mut() {
        members.sort();
    }
    groups
}

fn phase_state_char(
    phase: &crate::sumo::net::SumoTlPhase,
    link_index: u32,
) -> Result<char> {
    phase
        .state
        .chars()
        .nth(link_index as usize)
        .ok_or_else(|| {
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
