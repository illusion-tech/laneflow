//! 路口领域 Canonical LIR 记录。

use laneflow_static_contract::{
    JunctionId, JunctionOrdinal, LaneEdgeOrdinal, ManeuverGateOrdinal, ManeuverPathId,
    ManeuverPathOrdinal, MovementId, MovementOrdinal, WaitingZoneOrdinal,
};

use crate::arena::TableRange;

use super::{LirIdentityField, LirRouteOccurrenceRef};

pub(crate) struct LirJunction {
    pub(crate) ordinal: JunctionOrdinal,
    pub(crate) stable_id: JunctionId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) movements: TableRange<MovementOrdinal>,
}

pub(crate) struct LirMovement {
    pub(crate) ordinal: MovementOrdinal,
    pub(crate) stable_id: MovementId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) junction: JunctionOrdinal,
    pub(crate) directed_entry_approach_key: Box<str>,
    pub(crate) directed_exit_approach_key: Box<str>,
    pub(crate) maneuver_paths: TableRange<ManeuverPathOrdinal>,
}

pub(crate) struct LirManeuverPath {
    pub(crate) ordinal: ManeuverPathOrdinal,
    pub(crate) stable_id: ManeuverPathId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) movement: MovementOrdinal,
    /// 完整 `entry + internal + exit` 序列。
    pub(crate) edges: TableRange<LaneEdgeOrdinal>,
    pub(crate) maneuver_gates: TableRange<ManeuverGateOrdinal>,
    pub(crate) waiting_zones: TableRange<WaitingZoneOrdinal>,
    pub(crate) static_route_occurrences: TableRange<LirRouteOccurrenceRef>,
}

pub(crate) struct LirJunctionInternalEdge {
    pub(crate) edge: LaneEdgeOrdinal,
    pub(crate) junction: JunctionOrdinal,
}

use super::{FreezeEnv, LirJunctionCounts, push_identity_field, push_lir_identity, relation_range};
use crate::DiagnosticBundle;
use laneflow_static_contract::FieldTag;

pub(super) struct JunctionParts {
    pub junctions: Vec<LirJunction>,
    pub junction_movements: Vec<MovementOrdinal>,
    pub movements: Vec<LirMovement>,
    pub movement_maneuver_paths: Vec<ManeuverPathOrdinal>,
    pub maneuver_paths: Vec<LirManeuverPath>,
    pub maneuver_path_edges: Vec<LaneEdgeOrdinal>,
    pub maneuver_path_gates: Vec<ManeuverGateOrdinal>,
    pub maneuver_path_waiting_zones: Vec<WaitingZoneOrdinal>,
    pub junction_internal_edges: Vec<LirJunctionInternalEdge>,
    pub canonical_mir_internal_edge_order: Vec<u32>,
}

pub(super) fn freeze(
    env: &mut FreezeEnv<'_>,
    counts: &LirJunctionCounts,
) -> Result<JunctionParts, DiagnosticBundle> {
    let mut junctions = Vec::with_capacity(env.capacity(counts.junctions)?);
    let mut junction_movements = Vec::with_capacity(env.capacity(counts.junction_movements)?);
    for mir_key in env
        .orders
        .junctions
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let junction = &env.mir.junctions[mir_key.index()];
        let identity_range = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::JunctionKey,
            &env.mir.modules[junction.module.index()].authoring_namespace_id,
            &junction.stable_key,
            None,
            env.limits,
            env.primary_span.clone(),
        )?;
        let relation_start = junction_movements.len();
        junction_movements.extend(
            env.mir.junction_movements[junction.movements.as_usize_range()]
                .iter()
                .map(|member| env.orders.movements.ordinal(member.movement)),
        );
        // 所有者成员关系是集合语义；按子实体规范序号冻结，避免声明先后进入语义摘要。
        junction_movements[relation_start..].sort_unstable();
        junctions.push(LirJunction {
            ordinal: env.orders.junctions.ordinal(mir_key),
            stable_id: junction.stable_id,
            identity_fields: identity_range,
            movements: relation_range(
                relation_start,
                junction_movements.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
        });
    }

    let mut movements = Vec::with_capacity(env.capacity(counts.movements)?);
    let mut movement_maneuver_paths = Vec::with_capacity(env.capacity(counts.movement_paths)?);
    for mir_key in env
        .orders
        .movements
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let movement = &env.mir.movements[mir_key.index()];
        let identity_start = env.identity_fields.len();
        for (tag, value) in [
            (
                FieldTag::AuthoringNamespaceId,
                env.mir.modules[movement.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
            ),
            (FieldTag::MovementKey, movement.stable_key.as_bytes()),
            (
                FieldTag::DirectedEntryApproachKey,
                movement.directed_entry_approach_key.as_bytes(),
            ),
            (
                FieldTag::DirectedExitApproachKey,
                movement.directed_exit_approach_key.as_bytes(),
            ),
            (
                FieldTag::JunctionStableId,
                env.mir.junctions[movement.junction.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes(),
            ),
        ] {
            push_identity_field(
                env.identity_fields,
                env.identity_field_bytes,
                tag,
                value,
                env.limits,
                env.primary_span.clone(),
            )?;
        }
        let relation_start = movement_maneuver_paths.len();
        movement_maneuver_paths.extend(
            env.mir.movement_maneuver_paths[movement.maneuver_paths.as_usize_range()]
                .iter()
                .map(|member| env.orders.maneuver_paths.ordinal(member.maneuver_path)),
        );
        movement_maneuver_paths[relation_start..].sort_unstable();
        movements.push(LirMovement {
            ordinal: env.orders.movements.ordinal(mir_key),
            stable_id: movement.stable_id,
            identity_fields: relation_range(
                identity_start,
                env.identity_fields.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
            junction: env.orders.junctions.ordinal(movement.junction),
            directed_entry_approach_key: movement.directed_entry_approach_key.as_ref().into(),
            directed_exit_approach_key: movement.directed_exit_approach_key.as_ref().into(),
            maneuver_paths: relation_range(
                relation_start,
                movement_maneuver_paths.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
        });
    }

    let mut maneuver_paths = Vec::with_capacity(env.capacity(counts.maneuver_paths)?);
    let mut maneuver_path_edges = Vec::with_capacity(env.capacity(counts.maneuver_path_edges)?);
    let mut maneuver_path_gates = Vec::with_capacity(env.capacity(counts.maneuver_path_gates)?);
    let mut maneuver_path_waiting_zones =
        Vec::with_capacity(env.capacity(counts.maneuver_path_waiting_zones)?);
    for mir_key in env
        .orders
        .maneuver_paths
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let path = &env.mir.maneuver_paths[mir_key.index()];
        let edges = &env.mir.maneuver_path_edges[path.edges.as_usize_range()];
        let identity_start = env.identity_fields.len();
        for (tag, value) in [
            (
                FieldTag::AuthoringNamespaceId,
                env.mir.modules[path.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
            ),
            (FieldTag::PathKey, path.stable_key.as_bytes()),
            (
                FieldTag::MovementStableId,
                env.mir.movements[path.movement.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes(),
            ),
            (
                FieldTag::EntryEdgeStableId,
                env.mir.lane_edges[edges[0].target.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes(),
            ),
            (
                FieldTag::ExitEdgeStableId,
                env.mir.lane_edges[edges[edges.len() - 1].target.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes(),
            ),
        ] {
            push_identity_field(
                env.identity_fields,
                env.identity_field_bytes,
                tag,
                value,
                env.limits,
                env.primary_span.clone(),
            )?;
        }
        let edge_start = maneuver_path_edges.len();
        maneuver_path_edges.extend(
            edges
                .iter()
                .map(|edge| env.orders.lane_edges.ordinal(edge.target)),
        );
        let gate_start = maneuver_path_gates.len();
        maneuver_path_gates.extend(
            env.mir.maneuver_path_gates[path.maneuver_gates.as_usize_range()]
                .iter()
                .map(|member| env.orders.maneuver_gates.ordinal(member.maneuver_gate)),
        );
        let waiting_start = maneuver_path_waiting_zones.len();
        maneuver_path_waiting_zones.extend(
            env.mir.maneuver_path_waiting_zones[path.waiting_zones.as_usize_range()]
                .iter()
                .map(|member| env.orders.waiting_zones.ordinal(member.waiting_zone)),
        );
        maneuver_paths.push(LirManeuverPath {
            ordinal: env.orders.maneuver_paths.ordinal(mir_key),
            stable_id: path.stable_id,
            identity_fields: relation_range(
                identity_start,
                env.identity_fields.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
            movement: env.orders.movements.ordinal(path.movement),
            edges: relation_range(
                edge_start,
                maneuver_path_edges.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
            maneuver_gates: relation_range(
                gate_start,
                maneuver_path_gates.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
            waiting_zones: relation_range(
                waiting_start,
                maneuver_path_waiting_zones.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
            static_route_occurrences: TableRange::empty(),
        });
    }

    let mut canonical_mir_internal_edge_order: Vec<u32> = (0..env
        .capacity(counts.junction_internal_edges)?)
        .map(|index| u32::try_from(index).expect("LIR precheck proved relation count fits u32"))
        .collect();
    canonical_mir_internal_edge_order.sort_unstable_by_key(|index| {
        env.orders
            .lane_edges
            .ordinal(env.mir.junction_internal_edges[*index as usize].edge)
    });
    let junction_internal_edges = canonical_mir_internal_edge_order
        .iter()
        .map(|index| {
            let relation = &env.mir.junction_internal_edges[*index as usize];
            LirJunctionInternalEdge {
                edge: env.orders.lane_edges.ordinal(relation.edge),
                junction: env.orders.junctions.ordinal(relation.junction),
            }
        })
        .collect::<Vec<_>>();

    Ok(JunctionParts {
        junctions,
        junction_movements,
        movements,
        movement_maneuver_paths,
        maneuver_paths,
        maneuver_path_edges,
        maneuver_path_gates,
        maneuver_path_waiting_zones,
        junction_internal_edges,
        canonical_mir_internal_edge_order,
    })
}
