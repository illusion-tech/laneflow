use std::collections::{BTreeMap, BTreeSet};

use laneflow_compiler::{GateInterpretation, GateProhibition, ManeuverDirection, SignalAspect};

use super::*;
use crate::config::SIGNAL_QUANTUM_MS;
use crate::{Layout, Template};

const EVIDENCE: &str = "template-v2";
const GAP: &str = "urban-gap";
const POCKET_METERS: f64 = 12.0;
const POCKET_SHIFT_METERS: f64 = 4.0;
const RELEASE_STUB_METERS: f64 = 1.0;
const WAITING_OCCUPANCY: u32 = 2;

fn local_key(entry: Direction, entry_lane: u32, exit: Direction, exit_lane: u32) -> String {
    format!("{}{}-{}{}", entry.key(), entry_lane, exit.key(), exit_lane)
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

fn arterial(direction: Direction) -> bool {
    matches!(direction, Direction::West | Direction::East)
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

fn signal_key(cell: &Cell, group: &str) -> String {
    format!("{}.{group}", cell.key())
}

fn group_of(entry: Direction, kind: ManeuverDirection) -> &'static str {
    if kind == ManeuverDirection::Left && arterial(entry) {
        if entry == Direction::East {
            "east-left"
        } else {
            "west-left"
        }
    } else if arterial(entry) {
        "ew-through"
    } else {
        "ns-through"
    }
}

fn direction_rank(direction: Direction) -> i32 {
    match direction {
        Direction::West => 0,
        Direction::East => 1,
        Direction::South => 2,
        Direction::North => 3,
    }
}

fn interpretation(cell: &Cell, entry: Direction, kind: ManeuverDirection) -> GateInterpretation {
    if uncontrolled(cell) {
        return GateInterpretation::Uncontrolled;
    }
    match (arterial(entry), kind) {
        (true, ManeuverDirection::Right) => GateInterpretation::DirectionalRightProtected,
        (false, ManeuverDirection::Right) => GateInterpretation::CnCircularRightTurn,
        (false, ManeuverDirection::Left) => GateInterpretation::PermissiveGroup,
        (_, ManeuverDirection::Straight | ManeuverDirection::Left | ManeuverDirection::UTurn) => {
            GateInterpretation::ProtectedGroup
        }
    }
}

fn is_protected(control: &str) -> bool {
    control == "ProtectedGroup" || control == "DirectionalRightProtected"
}

/// 缺臂后仍为空的车道改走仍存在的直行。支路没有直行时，原组合只剩下左转或右转。
fn approach_turns(cell: &Cell, entry: Direction) -> Result<Vec<Vec<ManeuverDirection>>> {
    let present = |kind: ManeuverDirection| {
        Direction::ALL
            .into_iter()
            .any(|exit| exit != entry && cell.has_arm(exit) && turn(entry, exit) == kind)
    };
    let mut lanes = if arterial(entry) {
        vec![
            vec![ManeuverDirection::Left],
            vec![ManeuverDirection::Straight],
            vec![ManeuverDirection::Straight, ManeuverDirection::Right],
        ]
    } else {
        vec![
            vec![ManeuverDirection::Left, ManeuverDirection::Straight],
            vec![ManeuverDirection::Right, ManeuverDirection::Straight],
        ]
    };
    for lane in &mut lanes {
        lane.retain(|kind| present(*kind));
        if lane.is_empty() && present(ManeuverDirection::Straight) {
            lane.push(ManeuverDirection::Straight);
        }
        if lane.is_empty() {
            return Err(crate::validation(
                "lane use",
                format!("{} {} has no remaining turn", cell.key(), entry.key()),
            ));
        }
    }
    Ok(lanes)
}

fn entry_lanes(cell: &Cell, entry: Direction, kind: ManeuverDirection) -> Result<Vec<u32>> {
    Ok(approach_turns(cell, entry)?
        .into_iter()
        .enumerate()
        .filter(|(_, turns)| turns.contains(&kind))
        .map(|(index, _)| index as u32)
        .collect())
}

fn exit_for(cell: &Cell, entry: Direction, kind: ManeuverDirection) -> Result<Direction> {
    Direction::ALL
        .into_iter()
        .find(|&exit| exit != entry && cell.has_arm(exit) && turn(entry, exit) == kind)
        .ok_or_else(|| crate::validation("lane use", format!("{} missing {:?}", cell.key(), kind)))
}

fn target_lanes(layout: &Layout, cell: &Cell, exit: Direction) -> Result<Vec<u32>> {
    let count = exit.lane_count();
    let Some(next) = layout.neighbour(cell, exit) else {
        return Ok((0..count).collect());
    };
    let turns = approach_turns(next, exit.opposite())?;
    let lanes = (0..count)
        .filter(|&lane| !turns[lane as usize].is_empty())
        .collect::<Vec<_>>();
    if lanes.is_empty() {
        return Err(crate::validation(
            "lane use",
            format!("{} {} has no target lane", cell.key(), exit.key()),
        ));
    }
    Ok(lanes)
}

/// 连续且不交叉。目标更多时，多出的车道分给内侧进口。
fn monotone(entries: &[u32], targets: &[u32]) -> Vec<(u32, Vec<u32>)> {
    let entry_count = entries.len();
    let target_count = targets.len();
    let mut assigned = Vec::with_capacity(entry_count);
    if target_count >= entry_count {
        let base = target_count / entry_count;
        let extra = target_count % entry_count;
        let mut cursor = 0;
        for (index, &lane) in entries.iter().enumerate() {
            let count = if index < extra { base + 1 } else { base };
            assigned.push((lane, targets[cursor..cursor + count].to_vec()));
            cursor += count;
        }
    } else {
        let shared = entry_count - target_count + 1;
        let singles = entry_count - shared;
        for (index, &lane) in entries.iter().enumerate() {
            let target = if index < singles {
                index
            } else {
                target_count - 1
            };
            assigned.push((lane, vec![targets[target]]));
        }
    }
    assigned
}

fn travel_in(entry: Direction) -> Point {
    let (dx, dz) = entry.delta();
    [-f64::from(dx), -f64::from(dz)]
}

fn left_of(travel: Point) -> Point {
    [travel[1], -travel[0]]
}

fn connect(edges: &mut BTreeMap<String, Edge>, from: &str, to: &str) {
    let successors = &mut edges
        .get_mut(from)
        .unwrap_or_else(|| panic!("missing edge {from}"))
        .successors;
    if !successors.iter().any(|key| key == to) {
        successors.push(to.to_string());
    }
}

struct InternalEdge<'a> {
    key: &'a str,
    start: Point,
    end: Point,
    geometry: re::RoadEditingCurveProgram,
}

fn add_internal(
    builder: &mut Builder<'_>,
    config: &UrbanConfig,
    cell: &Cell,
    edges: &mut BTreeMap<String, Edge>,
    internals: &mut Vec<re::LaneEdgeReference>,
    edge: InternalEdge<'_>,
) -> Result<()> {
    builder.add_declaration(re::RoadEditingDeclaration::LaneEdge(
        re::LaneEdgeInput::try_new(
            edge.key,
            config.turn_speed_mps,
            Vec::new(),
            Some(edge.geometry),
        )?,
    ))?;
    edges.insert(
        edge.key.to_string(),
        Edge {
            key: edge.key.to_string(),
            tile: cell.tile,
            cell: cell.index,
            start: edge.start,
            end: edge.end,
            successors: Vec::new(),
        },
    );
    internals.push(edge_ref(edge.key)?);
    Ok(())
}

fn ensure_stop(
    builder: &mut Builder<'_>,
    stops: &mut BTreeSet<String>,
    edge: &str,
) -> Result<re::StopLineReference> {
    let stop = format!("{edge}.stop");
    if stops.insert(edge.to_string()) {
        builder.add_declaration(re::RoadEditingDeclaration::StopLine(
            re::StopLineInput::try_new(&stop, edge_ref(edge)?)?,
        ))?;
    }
    Ok(re::StopLineReference::local(&stop)?)
}

#[allow(clippy::too_many_arguments)]
fn add_gate(
    builder: &mut Builder<'_>,
    cell: &Cell,
    movement: &str,
    gate: &str,
    path: &re::ManeuverPathReference,
    transition: u32,
    stop_edge: &str,
    signal: Option<&str>,
    interpretation: GateInterpretation,
    stops: &mut BTreeSet<String>,
    gates: &mut Vec<re::PolicyGateRuleInput>,
) -> Result<()> {
    let stop = ensure_stop(builder, stops, stop_edge)?;
    let signal_control = match signal {
        Some(group) => {
            re::RoadEditingSignalControl::SignalGroup(re::SignalGroupReference::local(group)?)
        }
        None => re::RoadEditingSignalControl::None,
    };
    builder.add_declaration(re::RoadEditingDeclaration::ManeuverGate(
        re::ManeuverGateInput::try_new(gate, path.clone(), transition, stop, signal_control)?,
    ))?;
    gates.push(re::PolicyGateRuleInput::try_new(
        format!("{}.{movement}.{gate}", cell.key()),
        gate_ref(cell, movement, gate)?,
        None,
        interpretation,
        GateProhibition::None,
        vec![EVIDENCE.into()],
    )?);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add_path(
    builder: &mut Builder<'_>,
    config: &UrbanConfig,
    cell: &Cell,
    edges: &mut BTreeMap<String, Edge>,
    internals: &mut Vec<re::LaneEdgeReference>,
    stops: &mut BTreeSet<String>,
    gates: &mut Vec<re::PolicyGateRuleInput>,
    local: &str,
    entry_label: &str,
    exit_label: &str,
    kind: ManeuverDirection,
    entry_edge: &str,
    exit_edge: &str,
    start: Point,
    end: Point,
    curve: re::RoadEditingCurveProgram,
    signal: Option<&str>,
    interpretation: GateInterpretation,
) -> Result<(Vec<String>, Vec<Point>)> {
    let mut conflict_geometry = Vec::new();
    sample_curve(&curve, &mut conflict_geometry);
    let internal = format!("{}.{local}.i0", cell.key());
    add_internal(
        builder,
        config,
        cell,
        edges,
        internals,
        InternalEdge {
            key: &internal,
            start,
            end,
            geometry: curve,
        },
    )?;
    let path_edges = vec![entry_edge.to_string(), internal, exit_edge.to_string()];
    for pair in path_edges.windows(2) {
        connect(edges, &pair[0], &pair[1]);
    }
    let path = re::ManeuverPathReference::owner_scoped(vec![cell.key(), local.into()], "path")?;
    builder.add_declaration(re::RoadEditingDeclaration::Movement(
        re::MovementInput::try_new(
            local,
            re::JunctionReference::local(cell.key())?,
            entry_label,
            exit_label,
        )?
        .with_turn_direction(kind),
    ))?;
    builder.add_declaration(re::RoadEditingDeclaration::ManeuverPath(
        re::ManeuverPathInput::try_new(
            "path",
            re::MovementReference::owner_scoped(vec![cell.key()], local)?,
            edge_ref(entry_edge)?,
            vec![edge_ref(&path_edges[1])?],
            edge_ref(exit_edge)?,
        )?,
    ))?;
    add_gate(
        builder,
        cell,
        local,
        "admission",
        &path,
        0,
        entry_edge,
        signal,
        interpretation,
        stops,
        gates,
    )?;
    Ok((path_edges, conflict_geometry))
}

/// 待转区在分流之前。释放门之后用一条短的路段边作为共同出口，再分到各目标车道。
#[allow(clippy::too_many_arguments)]
fn add_waiting(
    builder: &mut Builder<'_>,
    config: &UrbanConfig,
    cell: &Cell,
    edges: &mut BTreeMap<String, Edge>,
    internals: &mut Vec<re::LaneEdgeReference>,
    approaches: &mut Vec<re::LaneEdgeReference>,
    stops: &mut BTreeSet<String>,
    gates: &mut Vec<re::PolicyGateRuleInput>,
    entry: Direction,
) -> Result<Vec<String>> {
    let lane = 0;
    let local = format!("{}{lane}-wait", entry.key());
    let incoming = cell.edge_key(entry, true, lane);
    let start = lane_port(cell, entry, true, lane, 30.0);
    let travel = travel_in(entry);
    let left = left_of(travel);
    let pocket_start = [
        start[0] + travel[0] * POCKET_METERS + left[0] * POCKET_SHIFT_METERS,
        start[1] + travel[1] * POCKET_METERS + left[1] * POCKET_SHIFT_METERS,
    ];
    let pocket_end = [
        pocket_start[0] + travel[0] * POCKET_METERS,
        pocket_start[1] + travel[1] * POCKET_METERS,
    ];
    let stub_end = [
        pocket_end[0] + travel[0] * RELEASE_STUB_METERS,
        pocket_end[1] + travel[1] * RELEASE_STUB_METERS,
    ];
    let approach = format!("{}.{local}.approach", cell.key());
    let pocket = format!("{}.{local}.pocket", cell.key());
    let stub = format!("{}.{local}.release", cell.key());
    add_internal(
        builder,
        config,
        cell,
        edges,
        internals,
        InternalEdge {
            key: &approach,
            start,
            end: pocket_start,
            geometry: bezier(
                start,
                [start[0] + travel[0] * 6.0, start[1] + travel[1] * 6.0],
                [
                    pocket_start[0] - travel[0] * 6.0,
                    pocket_start[1] - travel[1] * 6.0,
                ],
                pocket_start,
            )?,
        },
    )?;
    add_internal(
        builder,
        config,
        cell,
        edges,
        internals,
        InternalEdge {
            key: &pocket,
            start: pocket_start,
            end: pocket_end,
            geometry: line(pocket_start, pocket_end)?,
        },
    )?;
    add_corridor(
        builder,
        &stub,
        &[CorridorLane {
            key: stub.clone(),
            start: pocket_end,
            end: stub_end,
            successors: Vec::new(),
        }],
        config.turn_speed_mps,
        edges,
        cell.tile,
        cell.index,
    )?;
    approaches.push(edge_ref(&stub)?);
    let path_edges = vec![incoming, approach, pocket.clone(), stub];
    for pair in path_edges.windows(2) {
        connect(edges, &pair[0], &pair[1]);
    }
    let path = re::ManeuverPathReference::owner_scoped(vec![cell.key(), local.clone()], "path")?;
    builder.add_declaration(re::RoadEditingDeclaration::Movement(
        re::MovementInput::try_new(
            &local,
            re::JunctionReference::local(cell.key())?,
            entry.key(),
            format!("{}-release", entry.key()),
        )?
        .with_turn_direction(ManeuverDirection::Left),
    ))?;
    builder.add_declaration(re::RoadEditingDeclaration::ManeuverPath(
        re::ManeuverPathInput::try_new(
            "path",
            re::MovementReference::owner_scoped(vec![cell.key()], &local)?,
            edge_ref(&path_edges[0])?,
            vec![edge_ref(&path_edges[1])?, edge_ref(&path_edges[2])?],
            edge_ref(&path_edges[3])?,
        )?,
    ))?;
    let through = signal_key(cell, "ew-through");
    let left_group = signal_key(cell, group_of(entry, ManeuverDirection::Left));
    let protected = GateInterpretation::ProtectedGroup;
    for (gate, transition, group) in [
        ("admission", 0, through.as_str()),
        ("waiting-entry", 1, through.as_str()),
        ("release", 2, left_group.as_str()),
    ] {
        add_gate(
            builder,
            cell,
            &local,
            gate,
            &path,
            transition,
            &path_edges[transition as usize],
            Some(group),
            protected,
            stops,
            gates,
        )?;
    }
    builder.add_declaration(re::RoadEditingDeclaration::WaitingZone(
        re::WaitingZoneInput::try_new(
            "waiting",
            path,
            gate_ref(cell, &local, "waiting-entry")?,
            gate_ref(cell, &local, "release")?,
            WAITING_OCCUPANCY,
        )?,
    ))?;
    Ok(path_edges)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn add(
    builder: &mut Builder<'_>,
    config: &UrbanConfig,
    layout: &Layout,
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
    let mut approaches = Vec::new();
    for &arm in &arms {
        for entering in [true, false] {
            for lane in 0..arm.lane_count() {
                approaches.push(edge_ref(&cell.edge_key(arm, entering, lane))?);
            }
        }
    }
    let mut stops = BTreeSet::new();
    let first_movement = movements.len();
    for &entry in &arms {
        for kind in [
            ManeuverDirection::Left,
            ManeuverDirection::Straight,
            ManeuverDirection::Right,
        ] {
            let entries = entry_lanes(cell, entry, kind)?;
            if entries.is_empty() {
                continue;
            }
            let exit = exit_for(cell, entry, kind)?;
            let targets = target_lanes(layout, cell, exit)?;
            let waiting = cell.template == Template::ProtectedWaiting
                && kind == ManeuverDirection::Left
                && arterial(entry);
            let prefix = if waiting {
                Some(add_waiting(
                    builder,
                    config,
                    cell,
                    edges,
                    &mut internals,
                    &mut approaches,
                    &mut stops,
                    gates,
                    entry,
                )?)
            } else {
                None
            };
            let signal = (!uncontrolled(cell)).then(|| signal_key(cell, group_of(entry, kind)));
            let control = interpretation(cell, entry, kind);
            for (entry_lane, exit_lanes) in monotone(&entries, &targets) {
                for exit_lane in exit_lanes {
                    let local = local_key(entry, entry_lane, exit, exit_lane);
                    let exit_edge = cell.edge_key(exit, false, exit_lane);
                    let exit_point = lane_port(cell, exit, false, exit_lane, 30.0);
                    let (entry_edge, start, route_prefix) = if let Some(prefix) = &prefix {
                        let stub = prefix.last().expect("release stub");
                        (stub.clone(), edges[stub.as_str()].end, prefix.clone())
                    } else {
                        (
                            cell.edge_key(entry, true, entry_lane),
                            lane_port(cell, entry, true, entry_lane, 30.0),
                            Vec::new(),
                        )
                    };
                    let travel = travel_in(entry);
                    let (ox, oz) = exit.delta();
                    let chord = [exit_point[0] - start[0], exit_point[1] - start[1]];
                    let lateral = (chord[0] * travel[1] - chord[1] * travel[0]).abs();
                    // 同号直行与进出口共线，用直线。错位直行必须沿行驶方向相切，否则 2° 方向检查失败。
                    let curve = if kind == ManeuverDirection::Straight
                        && route_prefix.is_empty()
                        && lateral < 1e-3
                    {
                        line(start, exit_point)?
                    } else {
                        bezier(
                            start,
                            [start[0] + travel[0] * 12.0, start[1] + travel[1] * 12.0],
                            [
                                exit_point[0] - f64::from(ox) * 12.0,
                                exit_point[1] - f64::from(oz) * 12.0,
                            ],
                            exit_point,
                        )?
                    };
                    let (path_edges, conflict_geometry) = add_path(
                        builder,
                        config,
                        cell,
                        edges,
                        &mut internals,
                        &mut stops,
                        gates,
                        &local,
                        entry.key(),
                        exit.key(),
                        kind,
                        &entry_edge,
                        &exit_edge,
                        start,
                        exit_point,
                        curve,
                        signal.as_deref(),
                        control,
                    )?;
                    movements.push(Movement {
                        key: format!("{}.{}", cell.key(), local),
                        cell: cell.index,
                        entry,
                        exit,
                        entry_lane,
                        exit_lane,
                        turn: format!("{kind:?}"),
                        waiting: !route_prefix.is_empty(),
                        control: format!("{control:?}"),
                        edges: path_edges,
                        route_prefix,
                        conflict_geometry,
                    });
                }
            }
        }
    }
    builder.add_declaration(re::RoadEditingDeclaration::Junction(
        re::JunctionInput::try_new(cell.key(), approaches, internals)?,
    ))?;
    add_conflicts(builder, cell, &movements[first_movement..], streams)
}

fn sample_curve(curve: &re::RoadEditingCurveProgram, points: &mut Vec<Point>) {
    assert_eq!(
        curve.segments().len(),
        1,
        "templates use single-segment curves"
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
    }
    if a.exit == b.exit && a.exit_lane == b.exit_lane {
        return a.conflict_geometry.last().copied();
    }
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

fn signal_yields(movement: &Movement, other: &Movement) -> bool {
    let earlier = direction_rank(other.entry) < direction_rank(movement.entry);
    if movement.control == "PermissiveGroup" && other.turn == "Straight" {
        return true;
    }
    if movement.control == "CnCircularRightTurn" && other.turn == "Straight" {
        return true;
    }
    let left = |value: &Movement| value.control == "PermissiveGroup" && value.turn == "Left";
    let right = |value: &Movement| value.control == "CnCircularRightTurn";
    if left(movement) && left(other)
        || right(movement) && left(other)
        || left(movement) && right(other)
    {
        return earlier;
    }
    false
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
            if !uncontrolled(cell)
                && is_protected(&a.control)
                && is_protected(&b.control)
                && group_of(a.entry, turn(a.entry, a.exit))
                    == group_of(b.entry, turn(b.entry, b.exit))
            {
                return Err(crate::validation(
                    "protected crossing",
                    format!("{} {}", a.key, b.key),
                ));
            }
            let key = format!(
                "{}x{}",
                local_key(a.entry, a.entry_lane, a.exit, a.exit_lane),
                local_key(b.entry, b.entry_lane, b.exit, b.exit_lane)
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
    let rank = |movement: &Movement| {
        (
            !arterial(movement.entry),
            movement.turn != "Straight",
            movement.entry,
            movement.exit,
            movement.entry_lane,
            movement.exit_lane,
        )
    };
    for (i, movement) in movements.iter().enumerate() {
        if zones[i].is_empty() {
            if movement.control == "PermissiveGroup" {
                return Err(crate::validation("permissive template", &movement.key));
            }
            continue;
        }
        let key = local_key(
            movement.entry,
            movement.entry_lane,
            movement.exit,
            movement.exit_lane,
        );
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
                            re::PathAnchorInput::gate(gate_ref(cell, &key, "admission")?),
                            re::PathAnchorInput::edge_boundary((movement.edges.len() - 1) as u32),
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
                    rank(&movements[j]) < rank(movement)
                } else {
                    signal_yields(movement, &movements[j])
                }
            })
            .map(|j| {
                stream_ref(
                    cell,
                    &local_key(
                        movements[j].entry,
                        movements[j].entry_lane,
                        movements[j].exit,
                        movements[j].exit_lane,
                    ),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        if movement.control == "PermissiveGroup" && yield_to.is_empty() {
            return Err(crate::validation("permissive template", &movement.key));
        }
        let priority = if uncontrolled(cell) {
            100 - movements
                .iter()
                .filter(|other| rank(other) < rank(movement))
                .count() as i32
        } else if movement.turn == "Straight" {
            100
        } else if movement.control == "PermissiveGroup" || movement.control == "CnCircularRightTurn"
        {
            40 - direction_rank(movement.entry)
        } else {
            80
        };
        let gap = (!yield_to.is_empty()).then(|| GAP.into());
        references += 1 + yield_to.len() as u64;
        streams.push(re::PolicyStreamRuleInput::try_new(
            &movement.key,
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
    let mut plan = vec![("ew-through", false)];
    if !entry_lanes(cell, Direction::East, ManeuverDirection::Left)?.is_empty() {
        plan.push(("east-left", true));
    }
    if !entry_lanes(cell, Direction::West, ManeuverDirection::Left)?.is_empty() {
        plan.push(("west-left", true));
    }
    plan.push(("ns-through", false));
    let groups: Vec<_> = plan
        .iter()
        .map(|(name, _)| signal_key(cell, name))
        .collect();
    for group in &groups {
        builder.add_declaration(re::RoadEditingDeclaration::SignalGroup(
            re::SignalGroupInput::try_new(group)?,
        ))?;
    }
    let controller = format!("{}.controller", cell.key());
    let mut phases = Vec::new();
    for (index, (_, left)) in plan.iter().enumerate() {
        for (suffix, aspect, quanta) in [
            (
                "green",
                SignalAspect::Green,
                if *left {
                    config.signals.left_quanta
                } else {
                    config.signals.through_quanta
                },
            ),
            ("yellow", SignalAspect::Yellow, config.signals.yellow_quanta),
            ("all-red", SignalAspect::Red, config.signals.all_red_quanta),
        ] {
            let key = format!("p{index}.{suffix}");
            let duration_ms = u64::from(quanta)
                .checked_mul(SIGNAL_QUANTUM_MS)
                .ok_or_else(|| crate::Error::Config("signal time overflow".into()))?;
            let states = groups
                .iter()
                .enumerate()
                .map(|(group_index, group)| {
                    re::RoadEditingSignalPhaseState::try_new(
                        re::SignalGroupReference::local(group)?,
                        if group_index == index {
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
                    .map(|(group_index, group)| {
                        (
                            group.clone(),
                            format!(
                                "{:?}",
                                if group_index == index {
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
    let cycle_ms: u64 = phases.iter().map(|phase| phase.duration_ms).sum();
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
                .map(|phase| {
                    re::SignalPhaseReference::owner_scoped(vec![controller.clone()], &phase.key)
                })
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
            re::RegulationIdentity::try_new("engineering-workload", "cn-urban-template-v2")?,
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
