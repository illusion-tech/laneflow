use laneflow_compiler::{GateInterpretation, GateProhibition, ManeuverDirection, SignalAspect};

use super::*;
use crate::Template;
use crate::config::SIGNAL_QUANTUM_MS;

const EVIDENCE: &str = "template-v1";
const GAP: &str = "urban-gap";
const GROUPS: [&str; 4] = ["ew-through", "ew-left", "ns-through", "ns-left"];

fn movement_key(entry: Direction, exit: Direction) -> String {
    format!("{}-{}", entry.key(), exit.key())
}

fn turn(entry: Direction, exit: Direction) -> ManeuverDirection {
    if entry.opposite() == exit {
        return ManeuverDirection::Straight;
    }
    let (x, z) = entry.delta();
    let (ox, oz) = exit.delta();
    if -x * oz + z * ox < 0 {
        ManeuverDirection::Left
    } else {
        ManeuverDirection::Right
    }
}

fn uncontrolled(cell: &Cell) -> bool {
    matches!(
        cell.template,
        Template::PriorityT | Template::StaggeredNorthT | Template::StaggeredSouthT
    )
}

fn stream_ref(cell: &Cell, key: &str) -> Result<re::ParticipantStreamReference> {
    Ok(re::ParticipantStreamReference::owner_scoped(
        vec![cell.key()],
        key,
    )?)
}

fn gate_ref(cell: &Cell, movement: &str, key: &str) -> Result<re::ManeuverGateReference> {
    Ok(re::ManeuverGateReference::owner_scoped(
        vec![cell.key(), movement.into(), "path".into()],
        key,
    )?)
}

fn group(cell: &Cell, entry: Direction, left: bool) -> String {
    let axis = if matches!(entry, Direction::West | Direction::East) {
        0
    } else {
        2
    };
    format!("{}.{}", cell.key(), GROUPS[axis + usize::from(left)])
}

#[allow(clippy::too_many_arguments)]
pub(super) fn add(
    builder: &mut Builder<'_>,
    config: &UrbanConfig,
    cell: &Cell,
    edges: &mut BTreeMap<String, Edge>,
    movements: &mut Vec<Movement>,
    streams: &mut Vec<re::PolicyStreamRuleInput>,
    gates: &mut Vec<re::PolicyGateRuleInput>,
    signals: &mut Vec<SignalProgram>,
) -> Result<u64> {
    if !uncontrolled(cell) {
        signals.push(add_signals(builder, config, cell)?);
    }
    let arms: Vec<_> = Direction::ALL
        .into_iter()
        .filter(|&arm| cell.has_arm(arm))
        .collect();
    let mut internals = Vec::new();
    let mut pairs = Vec::new();
    let mut stop_edges = std::collections::BTreeSet::new();
    for &entry in &arms {
        for &exit in &arms {
            if entry != exit {
                pairs.push((entry, exit));
            }
        }
    }
    let first_movement = movements.len();
    for &(entry, exit) in &pairs {
        let key = movement_key(entry, exit);
        let direction = turn(entry, exit);
        let waiting =
            cell.template == Template::ProtectedWaiting && direction == ManeuverDirection::Left;
        let permissive =
            cell.template == Template::Permissive && direction == ManeuverDirection::Left;
        let interpretation = if uncontrolled(cell) {
            GateInterpretation::Uncontrolled
        } else if permissive {
            GateInterpretation::PermissiveGroup
        } else {
            GateInterpretation::ProtectedGroup
        };
        let path = re::ManeuverPathReference::owner_scoped(vec![cell.key(), key.clone()], "path")?;
        let start = port(cell, entry, true, 30.0);
        let end = port(cell, exit, false, 30.0);
        let (dx, dz) = entry.delta();
        let (ox, oz) = exit.delta();
        let d = [-f64::from(dx), -f64::from(dz)];
        let out = [f64::from(ox), f64::from(oz)];
        let curve_to = |a: Point, b: Point| {
            bezier(
                a,
                [a[0] + d[0] * 12.0, a[1] + d[1] * 12.0],
                [b[0] - out[0] * 12.0, b[1] - out[1] * 12.0],
                b,
            )
        };
        let segments = if waiting {
            // A 12 m pocket between the traffic directions; it holds one declared vehicle.
            let p1 = [
                start[0] + d[0] * 12.0 + d[1] * 4.0,
                start[1] + d[1] * 12.0 - d[0] * 4.0,
            ];
            let p2 = [p1[0] + d[0] * 12.0, p1[1] + d[1] * 12.0];
            vec![
                (
                    start,
                    p1,
                    bezier(
                        start,
                        [start[0] + d[0] * 6.0, start[1] + d[1] * 6.0],
                        [p1[0] - d[0] * 6.0, p1[1] - d[1] * 6.0],
                        p1,
                    )?,
                ),
                (p1, p2, line(p1, p2)?),
                (p2, end, curve_to(p2, end)?),
            ]
        } else if direction == ManeuverDirection::Straight {
            vec![(start, end, line(start, end)?)]
        } else {
            vec![(start, end, curve_to(start, end)?)]
        };
        let mut path_edges = vec![cell.edge_key(entry, true)];
        let mut conflict_geometry = Vec::new();
        for (index, (a, b, geometry)) in segments.into_iter().enumerate() {
            if !waiting || index == 2 {
                sample_curve(&geometry, &mut conflict_geometry);
            }
            let edge = format!("{}.{}.i{index}", cell.key(), key);
            builder.add_declaration(re::RoadEditingDeclaration::LaneEdge(
                re::LaneEdgeInput::try_new(
                    &edge,
                    config.turn_speed_mps,
                    Vec::new(),
                    Some(geometry),
                )?,
            ))?;
            edges.insert(
                edge.clone(),
                Edge {
                    key: edge.clone(),
                    tile: cell.tile,
                    cell: cell.index,
                    start: a,
                    end: b,
                    successors: Vec::new(),
                },
            );
            internals.push(edge_ref(&edge)?);
            path_edges.push(edge);
        }
        path_edges.push(cell.edge_key(exit, false));
        for pair in path_edges.windows(2) {
            edges
                .get_mut(&pair[0])
                .expect("declared path edge")
                .successors
                .push(pair[1].clone());
        }
        builder.add_declaration(re::RoadEditingDeclaration::Movement(
            re::MovementInput::try_new(
                &key,
                re::JunctionReference::local(cell.key())?,
                entry.key(),
                exit.key(),
            )?
            .with_turn_direction(direction),
        ))?;
        builder.add_declaration(re::RoadEditingDeclaration::ManeuverPath(
            re::ManeuverPathInput::try_new(
                "path",
                re::MovementReference::owner_scoped(vec![cell.key()], &key)?,
                edge_ref(&path_edges[0])?,
                path_edges[1..path_edges.len() - 1]
                    .iter()
                    .map(|key| edge_ref(key))
                    .collect::<Result<_>>()?,
                edge_ref(path_edges.last().expect("exit edge"))?,
            )?,
        ))?;
        for (gate, transition, left) in if waiting {
            vec![
                ("admission", 0, false),
                ("waiting-entry", 1, false),
                ("release", 2, true),
            ]
        } else {
            vec![(
                "admission",
                0,
                direction == ManeuverDirection::Left && !permissive,
            )]
        } {
            let rule_key = format!("{}.{}.{}", cell.key(), key, gate);
            let stop = format!("{}.stop", path_edges[transition]);
            if stop_edges.insert(path_edges[transition].clone()) {
                builder.add_declaration(re::RoadEditingDeclaration::StopLine(
                    re::StopLineInput::try_new(&stop, edge_ref(&path_edges[transition])?)?,
                ))?;
            }
            builder.add_declaration(re::RoadEditingDeclaration::ManeuverGate(
                re::ManeuverGateInput::try_new(
                    gate,
                    path.clone(),
                    transition as u32,
                    re::StopLineReference::local(&stop)?,
                    if uncontrolled(cell) {
                        re::RoadEditingSignalControl::None
                    } else {
                        re::RoadEditingSignalControl::SignalGroup(re::SignalGroupReference::local(
                            group(cell, entry, left),
                        )?)
                    },
                )?,
            ))?;
            gates.push(re::PolicyGateRuleInput::try_new(
                &rule_key,
                gate_ref(cell, &key, gate)?,
                None,
                interpretation,
                GateProhibition::None,
                vec![EVIDENCE.into()],
            )?);
        }
        if waiting {
            builder.add_declaration(re::RoadEditingDeclaration::WaitingZone(
                re::WaitingZoneInput::try_new(
                    "waiting",
                    path.clone(),
                    gate_ref(cell, &key, "waiting-entry")?,
                    gate_ref(cell, &key, "release")?,
                    1,
                )?,
            ))?;
        }
        movements.push(Movement {
            key: format!("{}.{}", cell.key(), key),
            cell: cell.index,
            entry,
            exit,
            turn: format!("{direction:?}"),
            waiting,
            control: format!("{interpretation:?}"),
            edges: path_edges,
            conflict_geometry,
        });
    }
    builder.add_declaration(re::RoadEditingDeclaration::Junction(
        re::JunctionInput::try_new(
            cell.key(),
            arms.iter()
                .flat_map(|&a| [cell.edge_key(a, true), cell.edge_key(a, false)])
                .map(|key| edge_ref(&key))
                .collect::<Result<_>>()?,
            internals,
        )?,
    ))?;
    add_conflicts(builder, cell, &movements[first_movement..], streams)
}

fn sample_curve(curve: &re::RoadEditingCurveProgram, points: &mut Vec<Point>) {
    assert_eq!(
        curve.segments().len(),
        1,
        "v1 templates use single-segment curves"
    );
    let start = curve.start();
    let a = [start.x(), start.z()];
    points.push(a);
    for segment in curve.segments() {
        match segment.geometry() {
            re::RoadEditingCurveSegmentGeometry::Line { end } => points.push([end.x(), end.z()]),
            re::RoadEditingCurveSegmentGeometry::CubicBezier {
                control_1,
                control_2,
                end,
            } => {
                for i in 1..=32 {
                    let t = f64::from(i) / 32.0;
                    let u = 1.0 - t;
                    points.push([
                        u * u * u * a[0]
                            + 3.0 * u * u * t * control_1.x()
                            + 3.0 * u * t * t * control_2.x()
                            + t * t * t * end.x(),
                        u * u * u * a[1]
                            + 3.0 * u * u * t * control_1.z()
                            + 3.0 * u * t * t * control_2.z()
                            + t * t * t * end.z(),
                    ]);
                }
            }
        }
    }
}

fn crossing(a: &Movement, b: &Movement) -> Option<Point> {
    if a.entry == b.entry {
        return None;
    } // upstream lane occupancy owns a shared approach
    if a.exit == b.exit {
        return a.conflict_geometry.last().copied();
    } // a real merge
    for x in a.conflict_geometry.windows(2) {
        for y in b.conflict_geometry.windows(2) {
            let r = [x[1][0] - x[0][0], x[1][1] - x[0][1]];
            let s = [y[1][0] - y[0][0], y[1][1] - y[0][1]];
            let q = [y[0][0] - x[0][0], y[0][1] - x[0][1]];
            let cross = |a: Point, b: Point| a[0] * b[1] - a[1] * b[0];
            let denominator = cross(r, s);
            if denominator.abs() < 1e-9 {
                continue;
            }
            let t = cross(q, s) / denominator;
            let u = cross(q, r) / denominator;
            if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
                return Some([x[0][0] + t * r[0], x[0][1] + t * r[1]]);
            }
        }
    }
    None
}

fn add_conflicts(
    builder: &mut Builder<'_>,
    cell: &Cell,
    movements: &[Movement],
    streams: &mut Vec<re::PolicyStreamRuleInput>,
) -> Result<u64> {
    let mut references = 0;
    let mut zones = vec![Vec::new(); movements.len()];
    let mut opponents = vec![Vec::new(); movements.len()];
    for (i, a) in movements.iter().enumerate() {
        for (j, b) in movements.iter().enumerate().skip(i + 1) {
            let Some(center) = crossing(a, b) else {
                continue;
            };
            let key = format!(
                "{}-{}",
                movement_key(a.entry, a.exit),
                movement_key(b.entry, b.exit)
            );
            let reference = re::ConflictZoneReference::owner_scoped(vec![cell.key()], &key)?;
            builder.add_declaration(re::RoadEditingDeclaration::ConflictZone(
                re::ConflictZoneInput::try_new(&key, re::JunctionReference::local(cell.key())?)?,
            ))?;
            builder.add_conflict_zone_region(re::ConflictZoneRegionInput::try_new(
                reference.clone(),
                re::CanonicalFrameReference::local(FRAME_KEY)?,
                -1.0,
                1.0,
                [[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]]
                    .into_iter()
                    .map(|p| {
                        Ok(re::RoadEditingPoint2::try_new(
                            center[0] + p[0],
                            center[1] + p[1],
                        )?)
                    })
                    .collect::<Result<_>>()?,
            )?)?;
            zones[i].push(reference.clone());
            zones[j].push(reference);
            opponents[i].push(j);
            opponents[j].push(i);
        }
    }
    let rank = |m: &Movement| {
        (
            !matches!(m.entry, Direction::West | Direction::East),
            m.turn != "Straight",
            m.entry,
            m.exit,
        )
    };
    for (i, m) in movements.iter().enumerate() {
        let key = movement_key(m.entry, m.exit);
        builder.add_declaration(re::RoadEditingDeclaration::ParticipantStream(
            re::ParticipantStreamInput::try_new(
                &key,
                re::JunctionReference::local(cell.key())?,
                re::ManeuverPathReference::owner_scoped(vec![cell.key(), key.clone()], "path")?,
                zones[i]
                    .iter()
                    .map(|zone| {
                        Ok(re::ConflictPassageInput::new(
                            zone.clone(),
                            re::PathAnchorInput::gate(gate_ref(
                                cell,
                                &key,
                                if m.waiting { "release" } else { "admission" },
                            )?),
                            re::PathAnchorInput::edge_boundary((m.edges.len() - 1) as u32),
                        ))
                    })
                    .collect::<Result<_>>()?,
            )?,
        ))?;
        let yield_to = opponents[i]
            .iter()
            .copied()
            .filter(|&j| {
                if uncontrolled(cell) {
                    rank(&movements[j]) < rank(m)
                } else {
                    m.control == "PermissiveGroup" && movements[j].turn == "Straight"
                }
            })
            .map(|j| stream_ref(cell, &movement_key(movements[j].entry, movements[j].exit)))
            .collect::<Result<Vec<_>>>()?;
        if m.control == "PermissiveGroup" && yield_to.is_empty() {
            return Err(crate::validation("permissive template", &m.key));
        }
        let priority = if uncontrolled(cell) {
            100 - movements
                .iter()
                .filter(|other| rank(other) < rank(m))
                .count() as i32
        } else if m.turn == "Straight" {
            100
        } else {
            0
        };
        let gap = (!yield_to.is_empty()).then(|| GAP.into());
        references += 1 + yield_to.len() as u64;
        streams.push(re::PolicyStreamRuleInput::try_new(
            &m.key,
            stream_ref(cell, &key)?,
            None,
            priority,
            yield_to,
            gap,
            vec![EVIDENCE.into()],
        )?);
    }
    Ok(references)
}

fn add_signals(
    builder: &mut Builder<'_>,
    config: &UrbanConfig,
    cell: &Cell,
) -> Result<SignalProgram> {
    let group_indices = if cell.template == Template::Permissive {
        vec![0, 2]
    } else {
        vec![0, 1, 2, 3]
    };
    let groups: Vec<_> = group_indices
        .iter()
        .map(|&i| format!("{}.{}", cell.key(), GROUPS[i]))
        .collect();
    for g in &groups {
        builder.add_declaration(re::RoadEditingDeclaration::SignalGroup(
            re::SignalGroupInput::try_new(g)?,
        ))?;
    }
    let controller = format!("{}.controller", cell.key());
    let mut phases = Vec::new();
    for (green, &group_index) in group_indices.iter().enumerate() {
        for (suffix, aspect, quanta) in [
            (
                "green",
                SignalAspect::Green,
                if group_index % 2 == 0 {
                    config.signals.through_quanta
                } else {
                    config.signals.left_quanta
                },
            ),
            ("yellow", SignalAspect::Yellow, config.signals.yellow_quanta),
            ("all-red", SignalAspect::Red, config.signals.all_red_quanta),
        ] {
            let key = format!("p{green}.{suffix}");
            let duration_ms = u64::from(quanta)
                .checked_mul(SIGNAL_QUANTUM_MS)
                .ok_or_else(|| crate::Error::Config("signal time overflow".into()))?;
            let states = groups
                .iter()
                .enumerate()
                .map(|(i, g)| {
                    re::RoadEditingSignalPhaseState::try_new(
                        re::SignalGroupReference::local(g)?,
                        if i == green {
                            aspect
                        } else {
                            SignalAspect::Red
                        },
                    )
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            builder.add_declaration(re::RoadEditingDeclaration::SignalPhase(
                re::SignalPhaseInput::try_new(
                    &key,
                    duration_ms,
                    states,
                    re::SignalControllerReference::local(&controller)?,
                )?,
            ))?;
            phases.push(SignalPhase {
                key,
                duration_ms,
                states: groups
                    .iter()
                    .enumerate()
                    .map(|(i, g)| {
                        (
                            g.clone(),
                            format!(
                                "{:?}",
                                if i == green {
                                    aspect
                                } else {
                                    SignalAspect::Red
                                }
                            ),
                        )
                    })
                    .collect(),
            });
        }
    }
    let cycle_ms: u64 = phases.iter().map(|p| p.duration_ms).sum();
    let offset_ms = (u64::from(cell.index) * u64::from(config.signals.offset_step_quanta)
        % (cycle_ms / SIGNAL_QUANTUM_MS))
        * SIGNAL_QUANTUM_MS;
    builder.add_declaration(re::RoadEditingDeclaration::SignalController(
        re::SignalControllerInput::try_new(
            &controller,
            offset_ms,
            groups
                .iter()
                .map(re::SignalGroupReference::local)
                .collect::<std::result::Result<_, _>>()?,
            phases
                .iter()
                .map(|p| re::SignalPhaseReference::owner_scoped(vec![controller.clone()], &p.key))
                .collect::<std::result::Result<_, _>>()?,
        )?,
    ))?;
    Ok(SignalProgram {
        key: controller,
        offset_ms,
        cycle_ms,
        phases,
    })
}

pub(super) fn add_policy(
    builder: &mut Builder<'_>,
    streams: Vec<re::PolicyStreamRuleInput>,
    gates: Vec<re::PolicyGateRuleInput>,
) -> Result<()> {
    builder.add_declaration(re::RoadEditingDeclaration::RightOfWayPolicySet(
        re::RightOfWayPolicySetInput::try_new(
            POLICY_KEY,
            re::RegulationIdentity::try_new("engineering-workload", "cn-urban-template-v1")?,
            vec![re::PolicyEvidenceInput::try_new(
                EVIDENCE,
                "repository:tools/laneflow-urban-generator/README.md#templates",
                Some("Versioned technical validation template; not a legal certification".into()),
            )?],
            vec![re::PolicyGapProfileInput::try_new(
                GAP,
                "urban-conservative-v1",
                5_000,
                2_000,
                500,
            )?],
            streams,
            gates,
        )?,
    ))?;
    Ok(())
}
